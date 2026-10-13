// SPDX-License-Identifier: AGPL-3.0-only
//! 系统日志查看器：探测可查看的日志源 + 按游标读取内容。
//!
//! 面板的「服务器状态 → 日志查看器」要能看内核日志、系统日志、nginx、php、mysql
//! 以及应用商店装的其它服务日志。这些日志有两个特点决定了必须走 zapexec（root）：
//!
//! 1. **journald 权限**：面板进程通常不在 `systemd-journal` 组里，直接 spawn
//!    `journalctl` 会**静默返回空**（退出码 0），既不报错也没内容 —— 与
//!    [`crate::verbs::ban_log`] 遇到的情况完全一致。
//! 2. **/var/log 权限**：nginx / mysql / fail2ban 等日志是 0640 root:adm，
//!    面板账号读不到。
//!
//! 日志源统一用 id 表达，前端只传 id：
//! - `journal:<unit>` → `journalctl -u <unit>`
//! - `journal:`       → `journalctl`（全部系统日志）
//! - `kernel:`        → `journalctl -k`；无 journald 时退化 `dmesg`
//! - `file:<path>`    → 文本日志（按字节偏移增量 / 末尾 N 行）
//!
//! 读取一律**只读**，不提供清空 / 轮转之类的写操作。

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::json;
use zap_proto::Response;

/// 单次读取的行数上限：防止误填巨大值把整份日志灌进响应。
const MAX_LINES: u32 = 5000;
/// 末尾读取的默认行数。
const DEFAULT_LINES: u32 = 200;

fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

/// 第三方软件安装根目录（应用商店装的 nginx / php / mysql 都在这下面）。
fn apps_root() -> PathBuf {
    super::install_root()
}

// ── 日志源描述 ────────────────────────────────────────────────

struct Source {
    /// 前端回传的 id（`journal:<unit>` / `journal:` / `kernel:` / `file:<path>`）
    id: String,
    /// 显示名
    name: String,
    /// 分组（内核 / 系统 / Web / PHP / 数据库 / 面板 / 安全 / 服务）
    group: String,
    /// journal | file | kernel
    kind: &'static str,
    /// 副标题：路径或 unit 名
    desc: String,
    /// 文件大小（仅 file；其余为 0）
    size: u64,
    /// 是否管理员手动添加的自定义路径（前端据此显示"删除"按钮）
    custom: bool,
}

impl Source {
    fn file(path: &str, name: &str, group: &str) -> Option<Self> {
        let p = Path::new(path);
        if !p.is_file() {
            return None;
        }
        Some(Self {
            id: format!("file:{path}"),
            name: name.to_string(),
            group: group.to_string(),
            kind: "file",
            desc: path.to_string(),
            size: p.metadata().map(|m| m.len()).unwrap_or(0),
            custom: false,
        })
    }

    fn journal(unit: &str, name: &str, group: &str) -> Self {
        Self {
            id: format!("journal:{unit}"),
            name: name.to_string(),
            group: group.to_string(),
            kind: "journal",
            desc: unit.to_string(),
            size: 0,
            custom: false,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        json!({
            "id": self.id,
            "name": self.name,
            "group": self.group,
            "kind": self.kind,
            "desc": self.desc,
            "size": self.size,
            "custom": self.custom,
        })
    }
}

// ── 通配符展开（只支持单个路径分量内的 `*`）────────────────────

fn glob_match(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    if !name.starts_with(parts[0]) {
        return false;
    }
    let mut pos = parts[0].len();
    for p in &parts[1..parts.len() - 1] {
        match name[pos..].find(p) {
            Some(i) => pos += i + p.len(),
            None => return false,
        }
    }
    let last = parts[parts.len() - 1];
    last.is_empty() || name[pos..].ends_with(last)
}

/// 展开形如 `{apps}/nginx-*/logs/*.log` 的模式，只返回存在的普通文件。
fn glob_files(pattern: &str) -> Vec<PathBuf> {
    let mut cur: Vec<PathBuf> = vec![if pattern.starts_with('/') {
        PathBuf::from("/")
    } else {
        PathBuf::from(".")
    }];
    for comp in pattern.split('/').filter(|c| !c.is_empty()) {
        let mut next = Vec::new();
        for base in &cur {
            if comp.contains('*') {
                if let Ok(rd) = std::fs::read_dir(base) {
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if glob_match(comp, &name) {
                            next.push(e.path());
                        }
                    }
                }
            } else {
                next.push(base.join(comp));
            }
        }
        cur = next;
        if cur.len() > 200 {
            cur.truncate(200);
        }
    }
    let mut out: Vec<PathBuf> = cur.into_iter().filter(|p| p.is_file()).collect();
    out.sort();
    out
}

// ── 环境探测 ─────────────────────────────────────────────────

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

/// 值得单独列出来的服务 unit 关键字（其余服务不逐个罗列，避免列表爆炸）。
const UNIT_KEYWORDS: &[&str] = &[
    "nginx", "openresty", "apache", "httpd", "php", "mysql", "mariadb", "postgres", "redis",
    "memcached", "mongodb", "ssh", "docker", "zap", "cron", "fail2ban", "supervisor", "tomcat",
    "rabbitmq", "elasticsearch", "minio",
];

/// 从 `systemctl list-unit-files` 里挑出已安装的相关服务 unit。
fn systemd_units() -> Vec<String> {
    let Ok(out) = std::process::Command::new("systemctl")
        .args(["list-unit-files", "--type=service", "--no-legend", "--plain"])
        .env("LC_ALL", "C")
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut v: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(|n| n.strip_suffix(".service").unwrap_or(n).to_string())
        .filter(|u| {
            !u.is_empty() && UNIT_KEYWORDS.iter().any(|k| u.to_lowercase().contains(k))
        })
        .collect();
    v.sort();
    v.dedup();
    v.truncate(40);
    v
}

/// mysql / mariadb 的错误日志路径：优先读 my.cnf 的 `log_error`，再看常见候选。
fn mysql_error_log() -> Option<String> {
    let cnf = [
        "/usr/local/mysql/etc/my.cnf",
        "/etc/my.cnf",
        "/etc/mysql/my.cnf",
    ]
    .iter()
    .find(|p| Path::new(p).is_file())
    .map(|p| p.to_string());
    if let Some(c) = cnf
        && let Ok(text) = std::fs::read_to_string(&c)
    {
        for line in text.lines() {
            let t = line.trim();
            if let Some(v) = t.strip_prefix("log_error") {
                let v = v.trim_start().trim_start_matches(['=', ' ']);
                let v = v.trim().trim_matches('"').trim_matches('\'');
                if !v.is_empty() && Path::new(v).is_file() {
                    return Some(v.to_string());
                }
            }
        }
    }
    [
        "/var/log/mysqld.log",
        "/var/log/mysql/error.log",
        "/var/lib/mysql/error.log",
    ]
    .iter()
    .map(|p| p.to_string())
    .find(|p| Path::new(p).is_file())
}

// ── 自定义日志源（管理员手动添加的路径）──────────────────────

/// 持久化位置：`{ZAP_PATH}/data/logs/custom_sources.json`。
///
/// 走文件而不是加表：这份清单只是"查看器里多几个入口"，没有查询 / 关联需求，
/// 单独建表要写迁移，收益不抵成本。文件由 zapexec（root）读写，面板账号
/// 是否可写 `{ZAP_PATH}/data` 都不影响。
fn custom_file() -> PathBuf {
    zap_path().join("data/logs/custom_sources.json")
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct CustomSource {
    name: String,
    path: String,
}

fn load_custom() -> Vec<CustomSource> {
    std::fs::read_to_string(custom_file())
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<CustomSource>>(&s).ok())
        .unwrap_or_default()
}

fn save_custom(list: &[CustomSource]) -> Result<(), String> {
    let p = custom_file();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let s = serde_json::to_string_pretty(list).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(&p, s).map_err(|e| format!("写入 {} 失败: {e}", p.display()))?;
    Ok(())
}

/// 路径合法性：绝对路径，且不含 `..` 段（防目录穿越）。
fn valid_path(p: &str) -> bool {
    p.starts_with('/') && !p.split('/').any(|seg| seg == "..")
}

// ── 日志源清单 ───────────────────────────────────────────────

fn discover() -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push = |s: Source, out: &mut Vec<Source>| {
        if seen.insert(s.id.clone()) {
            out.push(s);
        }
    };
    let journal = journal_available();

    // ── 内核 ──
    push(
        Source {
            id: "kernel:".to_string(),
            name: "内核日志".to_string(),
            group: "内核".to_string(),
            kind: "kernel",
            desc: if journal {
                "journalctl -k".to_string()
            } else {
                "dmesg".to_string()
            },
            size: 0,
            custom: false,
        },
        &mut out,
    );
    for p in ["/var/log/kern.log", "/var/log/dmesg"] {
        if let Some(s) = Source::file(p, "内核日志（文件）", "内核") {
            push(s, &mut out);
        }
    }

    // ── 系统 ──
    if journal {
        push(
            Source {
                id: "journal:".to_string(),
                name: "系统日志".to_string(),
                group: "系统".to_string(),
                kind: "journal",
                desc: "journalctl（全部）".to_string(),
                size: 0,
                custom: false,
            },
            &mut out,
        );
    }
    for (p, n) in [
        ("/var/log/syslog", "系统日志"),
        ("/var/log/messages", "系统日志"),
        ("/var/log/auth.log", "认证日志"),
        ("/var/log/secure", "认证日志"),
        ("/var/log/boot.log", "启动日志"),
    ] {
        if let Some(s) = Source::file(p, n, "系统") {
            push(s, &mut out);
        }
    }

    // ── Web（nginx）──
    for (p, n) in [
        ("/var/log/nginx/error.log", "Nginx 错误日志"),
        ("/var/log/nginx/access.log", "Nginx 访问日志"),
        ("/var/log/nginx/zap-stream.log", "Nginx 四层转发日志"),
    ] {
        if let Some(s) = Source::file(p, n, "Web") {
            push(s, &mut out);
        }
    }
    let apps = apps_root().to_string_lossy().to_string();
    for pat in [
        format!("{apps}/*/logs/error.log"),
        format!("{apps}/*/logs/access.log"),
        format!("{apps}/*/log/error.log"),
    ] {
        for f in glob_files(&pat) {
            let path = f.to_string_lossy().to_string();
            // 用目录名当显示名：`.../nginx-1.28.0/logs/error.log` → `nginx-1.28.0 error.log`
            let name = f
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .map(|d| format!("{} {}", d.to_string_lossy(), f.file_name().unwrap_or_default().to_string_lossy()))
                .unwrap_or_else(|| path.clone());
            if let Some(s) = Source::file(&path, &name, "服务") {
                push(s, &mut out);
            }
        }
    }

    // ── PHP ──
    for f in glob_files("/var/log/zap/php/*.log") {
        let path = f.to_string_lossy().to_string();
        let name = format!("PHP 错误日志 · {}", f.file_name().unwrap_or_default().to_string_lossy());
        if let Some(s) = Source::file(&path, &name, "PHP") {
            push(s, &mut out);
        }
    }

    // ── 数据库 ──
    if let Some(p) = mysql_error_log()
        && let Some(s) = Source::file(&p, "MySQL 错误日志", "数据库")
    {
        push(s, &mut out);
    }

    // ── 面板自身 ──
    let zap = zap_path().to_string_lossy().to_string();
    for f in glob_files(&format!("{zap}/data/appstore/logs/*.log")) {
        let path = f.to_string_lossy().to_string();
        let name = format!(
            "应用商店 · {}",
            f.file_name().unwrap_or_default().to_string_lossy()
        );
        if let Some(s) = Source::file(&path, &name, "面板") {
            push(s, &mut out);
        }
    }
    for f in glob_files("/var/log/zap/*.log") {
        let path = f.to_string_lossy().to_string();
        let name = format!("面板日志 · {}", f.file_name().unwrap_or_default().to_string_lossy());
        if let Some(s) = Source::file(&path, &name, "面板") {
            push(s, &mut out);
        }
    }

    // ── 安全 ──
    for (p, n) in [
        ("/var/log/modsec_audit.log", "WAF 审计日志"),
        ("/var/log/fail2ban.log", "fail2ban 日志"),
    ] {
        if let Some(s) = Source::file(p, n, "安全") {
            push(s, &mut out);
        }
    }

    // ── 自定义（手动添加；文件此刻不存在也照样列出，读取时才报"打不开"）──
    for c in load_custom() {
        push(
            Source {
                id: format!("file:{}", c.path),
                name: c.name.clone(),
                group: "自定义".to_string(),
                kind: "file",
                desc: c.path.clone(),
                size: Path::new(&c.path).metadata().map(|m| m.len()).unwrap_or(0),
                custom: true,
            },
            &mut out,
        );
    }

    // ── 已安装服务（systemd journal）──
    if journal {
        for u in systemd_units() {
            push(Source::journal(&u, &format!("服务 · {u}"), "服务"), &mut out);
        }
    }

    out
}

/// 列出可查看的日志源。
pub async fn sources() -> Response {
    tokio::task::spawn_blocking(|| {
        let list: Vec<serde_json::Value> = discover().iter().map(|s| s.to_json()).collect();
        Response::ok(
            "ok",
            Some(json!({
                "sources": list,
                "journal": journal_available(),
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("探测日志源失败: {e}")))
}

// ── 读取 ────────────────────────────────────────────────────

/// 取文件末尾 `n` 行（大文件只回读必要的尾部块，不整份载入）。
fn tail_lines(path: &str, n: usize) -> Result<Vec<String>, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("打开 {path} 失败: {e}"))?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut buf: Vec<u8> = Vec::new();
    let mut pos = len;
    while pos > 0 {
        let take = 65536u64.min(pos);
        pos -= take;
        f.seek(SeekFrom::Start(pos))
            .map_err(|e| format!("定位 {path} 失败: {e}"))?;
        let mut tmp = vec![0u8; take as usize];
        f.read_exact(&mut tmp)
            .map_err(|e| format!("读取 {path} 失败: {e}"))?;
        tmp.append(&mut buf);
        buf = tmp;
        // 够 n 行就停（末尾没有换行符时多加 1 行余量）
        if buf.iter().filter(|b| **b == b'\n').count() > n {
            break;
        }
        if buf.len() > 20 * 1024 * 1024 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(n);
    Ok(all[start..].iter().map(|s| s.to_string()).collect())
}

/// 从字节偏移读到文件末尾；只在整行处推进游标，避免半行被切成两条。
fn read_from_offset(path: &str, cursor: &str) -> Result<(Vec<String>, String), String> {
    let len = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("读取 {path} 属性失败: {e}"))?;
    let offset: u64 = cursor.parse::<u64>().unwrap_or(len);
    // 文件被轮转 / 截断后偏移会大于当前长度 → 从头重读
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

/// 读文本日志：cursor 空 = 末尾 N 行；否则增量。
fn read_file_source(path: &str, cursor: &str, lines: u32) -> Result<(Vec<String>, String), String> {
    if cursor.is_empty() {
        let ls = tail_lines(path, lines.max(1) as usize)?;
        // 首次展示历史后，游标直接落到文件末尾，后续只推新增内容
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        Ok((ls, size.to_string()))
    } else {
        read_from_offset(path, cursor)
    }
}

/// 读 journald：`units` 为空表示全部；`kernel` 为真时加 `-k`。
fn read_journal(
    units: &[String],
    kernel: bool,
    cursor: &str,
    lines: u32,
) -> Result<(Vec<String>, String), String> {
    let mut cmd = std::process::Command::new("journalctl");
    for u in units {
        cmd.arg("-u").arg(u);
    }
    if kernel {
        cmd.arg("-k");
    }
    cmd.args(["-o", "json", "--no-pager"]);
    cmd.env("LC_ALL", "C");
    if !cursor.is_empty() {
        cmd.arg("--after-cursor").arg(cursor);
        cmd.args(["-n", &MAX_LINES.to_string()]);
    } else {
        cmd.args(["-n", &lines.clamp(1, MAX_LINES).to_string()]);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("执行 journalctl 失败: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if !cursor.is_empty() {
            // 游标失效（日志被 vacuum / 轮转）→ 退回"取末尾"模式
            return read_journal(units, kernel, "", lines);
        }
        return Err(if err.is_empty() {
            "journalctl 返回非零".to_string()
        } else {
            err
        });
    }
    let mut lines_out = Vec::new();
    let mut new_cursor = cursor.to_string();
    for raw in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
            continue;
        };
        if let Some(c) = v.get("__CURSOR").and_then(|x| x.as_str()) {
            new_cursor = c.to_string();
        }
        // 带上时间与 unit，纯 MESSAGE 没法定位
        let ts = v
            .get("__REALTIME_TIMESTAMP")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .map(fmt_ts)
            .unwrap_or_default();
        let unit = v
            .get("_SYSTEMD_UNIT")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let msg = match v.get("MESSAGE") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => continue,
        };
        if msg.is_empty() {
            continue;
        }
        let prefix = match (ts.is_empty(), unit.is_empty()) {
            (true, true) => String::new(),
            (false, true) => format!("{ts} "),
            (true, false) => format!("[{unit}] "),
            (false, false) => format!("{ts} [{unit}] "),
        };
        lines_out.push(format!("{prefix}{msg}"));
    }
    Ok((lines_out, new_cursor))
}

/// journald 的微秒时间戳 → `MM-DD HH:MM:SS`。
fn fmt_ts(us: u64) -> String {
    let secs = (us / 1_000_000) as i64;
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%m-%d %H:%M:%S")
            .to_string(),
        None => String::new(),
    }
}

/// 无 journald 时的内核日志：`dmesg`（只读，不做 -c 清空）。
fn read_dmesg(lines: u32) -> Result<(Vec<String>, String), String> {
    let res = std::process::Command::new("dmesg")
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("执行 dmesg 失败: {e}"))?;
    if !res.status.success() {
        // 受 kernel.dmesg_restrict 限制时非 root 会被拒绝；zapexec 以 root 运行，这里只兜底
        let err = String::from_utf8_lossy(&res.stderr).trim().to_string();
        return Err(if err.is_empty() {
            "dmesg 返回非零".to_string()
        } else {
            err
        });
    }
    let text = String::from_utf8_lossy(&res.stdout).to_string();
    let all: Vec<&str> = text.lines().collect();
    let n = lines.max(1) as usize;
    let start = all.len().saturating_sub(n);
    Ok((all[start..].iter().map(|s| s.to_string()).collect(), String::new()))
}

/// 列出手动添加的自定义日志路径。
pub async fn custom_list() -> Response {
    let list = load_custom();
    Response::ok("ok", Some(json!({ "sources": list })))
}

/// 添加一个自定义日志路径（只登记，不校验文件此刻存在）。
pub async fn custom_add(name: String, path: String) -> Response {
    let path = path.trim().to_string();
    if !valid_path(&path) {
        return Response::err(-1, "路径必须是绝对路径，且不能包含 ..".to_string());
    }
    let mut list = load_custom();
    if list.iter().any(|c| c.path == path) {
        return Response::err(-1, "该路径已在列表中".to_string());
    }
    let name = if name.trim().is_empty() {
        Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone())
    } else {
        name.trim().to_string()
    };
    list.push(CustomSource { name, path });
    match save_custom(&list) {
        Ok(()) => Response::ok("ok", None),
        Err(e) => Response::err(-1, e),
    }
}

/// 删除一个自定义日志路径（只删登记，不动日志文件本身）。
pub async fn custom_remove(path: String) -> Response {
    let path = path.trim().to_string();
    let mut list = load_custom();
    let before = list.len();
    list.retain(|c| c.path != path);
    if list.len() == before {
        return Response::err(-1, "该路径不在列表中".to_string());
    }
    match save_custom(&list) {
        Ok(()) => Response::ok("ok", None),
        Err(e) => Response::err(-1, e),
    }
}

/// 读取某个日志源。
pub async fn read(source: String, cursor: String, lines: u32, keyword: String) -> Response {
    let n = if lines == 0 { DEFAULT_LINES } else { lines.min(MAX_LINES) };
    let result = tokio::task::spawn_blocking(move || -> Result<(Vec<String>, String, String), String> {
        let s = source.trim();
        if s.starts_with("kernel:") {
            if journal_available() {
                let (ls, c) = read_journal(&[], true, &cursor, n)?;
                Ok((ls, c, "journalctl -k".to_string()))
            } else {
                let (ls, c) = read_dmesg(n)?;
                Ok((ls, c, "dmesg".to_string()))
            }
        } else if let Some(units) = s.strip_prefix("journal:") {
            let units: Vec<String> = units
                .split(',')
                .map(|u| u.trim().to_string())
                .filter(|u| !u.is_empty())
                .collect();
            let label = if units.is_empty() {
                "journalctl".to_string()
            } else {
                format!("journalctl -u {}", units.join(" -u "))
            };
            let (ls, c) = read_journal(&units, false, &cursor, n)?;
            Ok((ls, c, label))
        } else if let Some(p) = s.strip_prefix("file:") {
            let p = p.trim();
            if p.is_empty() {
                return Err("日志路径为空".to_string());
            }
            let (ls, c) = read_file_source(p, &cursor, n)?;
            Ok((ls, c, p.to_string()))
        } else {
            Err(format!("无法识别的日志源: {s}"))
        }
    })
    .await
    .unwrap_or_else(|e| Err(format!("读取日志失败: {e}")));

    match result {
        Ok((mut ls, new_cursor, label)) => {
            // 关键字过滤（大小写不敏感）
            let kw = keyword.trim().to_lowercase();
            if !kw.is_empty() {
                ls.retain(|l| l.to_lowercase().contains(&kw));
            }
            Response::ok(
                "ok",
                Some(json!({
                    "lines": ls,
                    "cursor": new_cursor,
                    "source": label,
                    "count": ls.len(),
                })),
            )
        }
        Err(e) => Response::err(-1, e),
    }
}
