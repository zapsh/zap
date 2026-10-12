// SPDX-License-Identifier: AGPL-3.0-only
//! IP Ban 日志读取：以 root 身份读 journal / 文件，返回增量行与游标。
//!
//! 面板进程（zapadm / 开发用户）通常没有 systemd-journal 读权限，
//! 直接 spawn journalctl 会**静默返回空**（退出码 0），导致计数永远为 0 且
//! 不报错。因此这一特权读取统一走 zapexec（root），与封禁动作一致。

use std::io::{Read, Seek, SeekFrom};

use serde_json::json;
use zap_proto::Response;

/// 解析来源串：`auto` / `journal:<units>` / `file:<path>`。
enum Src {
    Auto,
    Journal(Vec<String>),
    File(String),
}

fn resolve(source: &str) -> Src {
    let t = source.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("auto") {
        return Src::Auto;
    }
    if let Some(units) = t.strip_prefix("journal:") {
        let units: Vec<String> = units
            .split(',')
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .collect();
        return if units.is_empty() {
            Src::Auto
        } else {
            Src::Journal(units)
        };
    }
    if t.eq_ignore_ascii_case("journal")
        || t.eq_ignore_ascii_case("journalctl")
        || t.eq_ignore_ascii_case("systemd-journal")
    {
        return Src::Journal(vec!["ssh".into(), "sshd".into()]);
    }
    if let Some(p) = t.strip_prefix("file:") {
        let p = p.trim();
        return if p.is_empty() {
            Src::Auto
        } else {
            Src::File(p.into())
        };
    }
    Src::File(t.to_string())
}

/// journalctl 是否可用（--version 退出 0）。
fn journal_available() -> bool {
    std::process::Command::new("journalctl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 探测 sshd 文本日志（RHEL / 装了 rsyslog 的 Debian）。
fn detect_log_file() -> Option<String> {
    for p in [
        "/var/log/auth.log",
        "/var/log/secure",
        "/var/log/auth.log.1",
        "/var/log/secure.1",
    ] {
        if std::path::Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    None
}

/// 读 journal 指定 units 的增量行，返回 (行, 最新游标)。
/// 首次（cursor 空）只定位游标、不重放历史。
fn read_journal(units: &[String], cursor: &str) -> Result<(Vec<String>, String), String> {
    let mut cmd = std::process::Command::new("journalctl");
    for u in units {
        cmd.arg("-u").arg(u);
    }
    cmd.args(["-o", "json", "--no-pager"]);
    cmd.env("LC_ALL", "C");
    if !cursor.is_empty() {
        cmd.arg("--after-cursor").arg(cursor);
    } else {
        // 首次：定位到最新一条的游标作为起点，不返回任何历史行
        cmd.args(["-n", "1"]);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("执行 journalctl 失败: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if !cursor.is_empty() {
            // 游标失效（日志被 vacuum / 轮转）时退回定位模式
            return read_journal(units, "");
        }
        return Err(if err.is_empty() {
            "journalctl 返回非零".to_string()
        } else {
            err
        });
    }
    let mut lines = Vec::new();
    let mut new_cursor = cursor.to_string();
    for raw in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
            continue;
        };
        if let Some(c) = v.get("__CURSOR").and_then(|x| x.as_str()) {
            new_cursor = c.to_string();
        }
        let msg = match v.get("MESSAGE") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => continue,
        };
        if !msg.is_empty() {
            lines.push(msg);
        }
    }
    Ok((lines, new_cursor))
}

/// 读文件增量行（按字节偏移）；首次 cursor 空 → 从文件末尾开始，只计新行。
fn read_file(path: &str, cursor: &str) -> Result<(Vec<String>, String), String> {
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let offset: u64 = if cursor.is_empty() {
        len
    } else {
        cursor.parse::<u64>().unwrap_or(len)
    };
    let offset = if offset > len { 0 } else { offset };
    let content = std::fs::File::open(path)
        .and_then(|mut f| {
            f.seek(SeekFrom::Start(offset))?;
            let mut buf = String::new();
            f.read_to_string(&mut buf)?;
            Ok(buf)
        })
        .map_err(|e| format!("读取 {path} 失败: {e}"))?;
    let consumed = content.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let new_offset = offset + consumed as u64;
    let lines: Vec<String> = content[..consumed].lines().map(|l| l.to_string()).collect();
    Ok((lines, new_offset.to_string()))
}

pub async fn read(source: String, cursor: String) -> Response {
    let (lines, new_cursor, label) = match resolve(&source) {
        Src::Auto => {
            if let Some(path) = detect_log_file() {
                match read_file(&path, &cursor) {
                    Ok((ls, c)) => (ls, c, path),
                    Err(e) => return Response::err(-1, e),
                }
            } else if journal_available() {
                match read_journal(&["ssh".into(), "sshd".into()], &cursor) {
                    Ok((ls, c)) => (ls, c, "journalctl -u ssh".to_string()),
                    Err(e) => return Response::err(-1, e),
                }
            } else {
                return Response::err(
                    -1,
                    "未找到 sshd 日志（auth.log / secure），系统也没有 journalctl".to_string(),
                );
            }
        }
        Src::Journal(units) => match read_journal(&units, &cursor) {
            Ok((ls, c)) => (ls, c, format!("journalctl -u {}", units.join(" -u "))),
            Err(e) => return Response::err(-1, e),
        },
        Src::File(path) => match read_file(&path, &cursor) {
            Ok((ls, c)) => (ls, c, path),
            Err(e) => return Response::err(-1, e),
        },
    };
    Response::ok(
        "ok",
        Some(json!({ "lines": lines, "cursor": new_cursor, "source": label })),
    )
}
