// SPDX-License-Identifier: AGPL-3.0-only
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::verbs::root_cmd;
use zap_proto::Response;

fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

/// 与 AppStore 共享日志目录，便于 zapd 侧 `log_path_for` 读取。
fn logs_dir() -> PathBuf {
    zap_path().join("data/appstore/logs")
}

fn command_exists(name: &str) -> bool {
    root_cmd("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 执行命令并将 stdout/stderr 追加写日志，返回退出码（带 600s 超时防卡死）。
fn run_and_log(program: &str, args: &[&str], log: &mut std::fs::File) -> i32 {
    let out = root_cmd("timeout")
        .arg("600")
        .arg(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    match out {
        Ok(o) => {
            let _ = log.write_all(&o.stdout);
            let _ = log.write_all(&o.stderr);
            let _ = log.flush();
            o.status.code().unwrap_or(-1)
        }
        Err(e) => {
            let _ = writeln!(log, "命令执行失败: {e}");
            -1
        }
    }
}

/// 运行命令并返回首个非空行（stdout 优先，其次 stderr）。
fn first_line(program: &str, args: &[&str]) -> Option<String> {
    let out = root_cmd(program).args(args).output().ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if let Some(l) = stdout.lines().find(|l| !l.trim().is_empty()) {
        return Some(l.trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
}

/// 获取 SSH 版本：多级兜底，避免 OpenSSH ≥9.7 中 `sshd -V` 报 unknown option。
fn ssh_version() -> String {
    // 1. 客户端版本（ssh 与 openssh-server 同包安装，版本一致）
    if let Some(v) = first_line("ssh", &["-V"]) {
        return v;
    }
    // 2. Debian/Ubuntu 包版本
    if let Some(v) = first_line("dpkg-query", &["-W", "-f=${Version}", "openssh-server"]) {
        return format!("OpenSSH {v}");
    }
    // 3. RHEL/CentOS/Fedora 包版本
    if let Some(v) = first_line("rpm", &["-q", "--qf=%{VERSION}", "openssh-server"]) {
        return format!("OpenSSH {v}");
    }
    // 4. 旧版 OpenSSH 仍支持 -V
    if let Some(v) = first_line("sshd", &["-V"]) {
        return v;
    }
    "Unknown".to_string()
}

pub async fn status() -> Response {
    tokio::task::spawn_blocking(|| {
        // 服务名按发行版不同：RHEL 是 sshd，Debian 新版本是 ssh
        let running = super::svc::is_active("sshd") || super::svc::is_active("ssh");

        let port = std::fs::read_to_string("/etc/ssh/sshd_config")
            .ok()
            .and_then(|c| {
                c.lines()
                    .find(|l| l.trim_start().starts_with("Port "))
                    .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
            })
            .unwrap_or(22u16);

        let version = ssh_version();

        let installed = version != "Unknown"
            || ["/usr/sbin/sshd", "/usr/bin/sshd", "/sbin/sshd", "/bin/sshd"]
                .iter()
                .any(|p| std::path::Path::new(p).exists());

        Response::ok(
            "ok",
            Some(
                json!({ "running": running, "installed": installed, "port": port, "version": version }),
            ),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 安装 openssh-server：后台线程执行，日志写 run-{id}.log，结束写 `__ZAP_DONE__ <code>`。
pub async fn install(run_id: String) -> Response {
    // run_id 会被拼进 `run-{id}.log`：必须过白名单，挡掉 `../` 之类的路径穿越
    if !super::valid_token(&run_id) {
        return Response::err(-1, format!("run_id 非法: {run_id}"));
    }
    let log_path = logs_dir().join(format!("run-{run_id}.log"));
    // 同步创建日志文件，确保接口返回时文件已存在
    if let Err(e) = std::fs::create_dir_all(logs_dir()).and_then(|_| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map(|_| ())
    }) {
        return Response::err(-1, format!("创建日志失败: {e}"));
    }

    tokio::task::spawn_blocking(move || {
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap_or_else(|_| {
                std::fs::OpenOptions::new()
                    .append(true)
                    .open("/dev/null")
                    .unwrap()
            });
        let _ = writeln!(log, "=== 安装 openssh-server ===");

        let pms: &[(&str, &[&str])] = &[
            ("apt-get", &["install", "-y", "openssh-server"]),
            ("dnf", &["install", "-y", "openssh-server"]),
            ("yum", &["install", "-y", "openssh-server"]),
            ("apk", &["add", "openssh-server"]),
        ];
        let mut chosen: Option<&(&str, &[&str])> = None;
        for pm in pms {
            if command_exists(pm.0) {
                chosen = Some(pm);
                break;
            }
        }
        let Some((pm, args)) = chosen else {
            let _ = writeln!(log, "错误: 未检测到支持的包管理器 (apt-get/dnf/yum/apk)");
            let _ = writeln!(log, "\n__ZAP_DONE__ 1");
            return Response::err(-1, "未检测到支持的包管理器".to_string());
        };

        let _ = writeln!(log, "使用包管理器: {pm}");
        let code = if *pm == "apt-get" {
            let _ = writeln!(log, "[1/2] 更新软件源索引...");
            let up = run_and_log("apt-get", &["update"], &mut log);
            let _ = writeln!(log, "apt-get update 退出码: {up}");
            let _ = writeln!(log, "[2/2] 安装 openssh-server...");
            run_and_log(pm, args, &mut log)
        } else {
            run_and_log(pm, args, &mut log)
        };

        let _ = writeln!(log, "\n__ZAP_DONE__ {code}");
        if code == 0 {
            Response::ok("openssh-server 安装成功", None)
        } else {
            Response::err(-1, "安装失败，请查看日志确认原因".to_string())
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

// ── sshd_config：可视化（原地改键）+ 文件编辑（全文）两种模式 ──────────
//
// 与 nginx.conf 不同，sshd_config 不做「全量重渲染」：
//   - 发行版配置里有大量 Include、注释、Match 条件块,重渲染必然丢信息;
//   - 后果不对称 —— nginx 配错只是站点挂,sshd 配错会把人锁在门外。
// 所以可视化模式走「原地改键」：只替换命中的那一行,其余原样保留;
// 全文模式则由用户自己把控,但落盘前一律 `sshd -t` 校验 + 自动备份。

const DEFAULT_SSHD_CONF: &str = "/etc/ssh/sshd_config";

/// 只允许编辑 /etc/ssh 下的文件,挡掉路径穿越与误改系统文件。
fn validate_conf_path(raw: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(raw);
    let base = PathBuf::from("/etc/ssh");
    let abs = if p.is_absolute() {
        p.clone()
    } else {
        base.join(&p)
    };
    let canon = match abs.canonicalize() {
        Ok(c) => c,
        // 文件还不存在（新建 Include 片段）时退回父目录判定
        Err(_) => match abs.parent().and_then(|d| d.canonicalize().ok()) {
            Some(d) => d.join(abs.file_name().unwrap_or_default()),
            None => return Err("配置文件路径不存在".to_string()),
        },
    };
    if !canon.starts_with(&base) && canon != std::fs::canonicalize(&base).unwrap_or(base.clone()) {
        return Err("仅允许编辑 /etc/ssh 下的配置文件".to_string());
    }
    Ok(canon)
}

fn sshd_bin() -> PathBuf {
    ["/usr/sbin/sshd", "/usr/bin/sshd", "/sbin/sshd", "/bin/sshd"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("/usr/sbin/sshd"))
}

fn backup_dir() -> PathBuf {
    zap_path().join("data/backups/sshd")
}

/// 备份（保留最近 20 份）。
fn backup_file(path: &Path) -> Result<PathBuf, String> {
    let dir = backup_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建备份目录失败: {e}"))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("conf");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = dir.join(format!("{ts}-{name}.bak"));
    std::fs::copy(path, &dest).map_err(|e| format!("备份失败: {e}"))?;

    let mut kept: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    kept.sort();
    while kept.len() > 20 {
        if let Some(old) = kept.first() {
            let _ = std::fs::remove_file(old);
            kept.remove(0);
        }
    }
    Ok(dest)
}

/// `sshd -t -f <file>` 校验：配错 sshd 会拒绝启动,这一步是把人锁门外的唯一防线。
///
/// 用 **实际配置所在目录** 下的临时文件,避免相对 HostKey 路径解析到别处。
fn validate_content(content: &str, tmpl_dir: &Path) -> Result<(), String> {
    let tmp = tmpl_dir.join(format!(".zap-sshd-{}.conf", std::process::id()));
    std::fs::write(&tmp, content).map_err(|e| format!("写入临时校验文件失败: {e}"))?;
    let out = root_cmd(sshd_bin().to_string_lossy().as_ref())
        .args(["-t", "-f"])
        .arg(&tmp)
        .output();
    let _ = std::fs::remove_file(&tmp);
    let out = out.map_err(|e| format!("执行 sshd -t 失败: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    }
    if text.is_empty() {
        text = "sshd -t 返回非零且无输出".to_string();
    }
    Err(text.chars().take(2000).collect())
}

/// 一行里的指令名（大小写不敏感）：`#Port 22` → Some("Port")。
fn directive_of(line: &str) -> Option<&str> {
    let t = line.trim();
    let t = t.strip_prefix('#').unwrap_or(t).trim_start();
    let mut it = t.split_whitespace();
    let key = it.next()?;
    if key.starts_with('#') || key.len() > 64 {
        return None;
    }
    // 纯数字 / 带等号的多半是噪声或 Include 参数
    if !key.chars().next()?.is_ascii_alphabetic() {
        return None;
    }
    Some(key)
}

/// 是否处于 `Match xxx` 条件块内（可视化模式不碰条件块内的同名指令,避免误改）。
fn is_match_header(line: &str) -> bool {
    let l = line.trim_start();
    l.len() > 5 && l[..5].eq_ignore_ascii_case("match") && l[5..].starts_with(char::is_whitespace)
}

/// 可视化模式：把一批 `Key=Value` 原地写回文本。
///
/// - 命中已存在的行 → 原地替换为 `Key value`（含被 `#` 注释掉的同名行,等同“启用并改值”）;
/// - 未命中 → 追加到全局段末尾（**在第一个 Match 块之前**,否则会掉进条件块）;
/// - value 为空 → 注释该行（等于删除/回退到默认）;
/// - 命中多行同名时只改**第一行**（sshd 对多数关键字取首次出现的值）,其余保持原样。
fn apply_directives(content: &str, set: &[(String, String)]) -> String {
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut remaining: Vec<(String, String)> = Vec::new();

    for (key, value) in set {
        let key_trim = key.trim();
        if key_trim.is_empty() {
            continue;
        }
        let mut hit = false;
        for line in lines.iter_mut() {
            if let Some(k) = directive_of(line)
                && k.eq_ignore_ascii_case(key_trim)
            {
                // 保留原行的指令名写法（sshd 大小写不敏感,但别随手改用户风格）
                *line = if value.trim().is_empty() {
                    let hint = directive_value_hint(key_trim);
                    if hint.is_empty() {
                        format!("#{k}")
                    } else {
                        format!("#{k} {hint}")
                    }
                } else {
                    format!("{k} {value}")
                };
                hit = true;
                break;
            }
        }
        if !hit && !value.trim().is_empty() {
            remaining.push((key_trim.to_string(), value.trim().to_string()));
        }
    }

    if !remaining.is_empty() {
        // 插到第一个 Match 块之前;没有 Match 就整体追加到末尾
        let insert_at = lines
            .iter()
            .position(|l| is_match_header(l))
            .unwrap_or(lines.len());
        let mut block: Vec<String> = vec![String::new(), "# ---- zap panel ----".to_string()];
        for (k, v) in &remaining {
            block.push(format!("{k} {v}"));
        }
        let mut tail = lines.split_off(insert_at);
        lines.extend(block);
        lines.append(&mut tail);
    }

    lines.join("\n")
}

/// 注释掉某行时补一个占位值,避免出现 `#Port` 这种缺参数的注释行被 sshd 误解析。
fn directive_value_hint(key: &str) -> &'static str {
    match key.to_lowercase().as_str() {
        "port" => "22",
        "permitrootlogin" => "yes",
        "passwordauthentication" => "yes",
        "pubkeyauthentication" => "yes",
        "permitemptypasswords" => "no",
        "x11forwarding" => "no",
        "usePAM" | "usepam" => "yes",
        "maxauthtries" => "6",
        "clientaliveinterval" => "0",
        "clientalivecountmax" => "3",
        "logingracetime" => "120",
        "tcpkeepalive" => "yes",
        "printmotd" => "no",
        "gssapiauthentication" => "no",
        "compression" => "delayed",
        "authorizedkeysfile" => ".ssh/authorized_keys",
        "subsystem" => "sftp internal-sftp",
        _ => "",
    }
}

/// 读取配置文件（含全文与状态）。
pub async fn config_get(path: Option<String>) -> Response {
    tokio::task::spawn_blocking(move || -> Response {
        let Ok(conf) = validate_conf_path(path.as_deref().unwrap_or(DEFAULT_SSHD_CONF)) else {
            return Response::err(-1, "配置文件路径非法".to_string());
        };
        let content = match std::fs::read_to_string(&conf) {
            Ok(c) => c,
            Err(e) => return Response::err(-1, format!("读取配置失败: {e}")),
        };
        let md = std::fs::metadata(&conf).ok();
        Response::ok(
            "ok",
            Some(json!({
                "path": conf,
                "content": content,
                "size": md.as_ref().map(|m| m.len()).unwrap_or(0),
                "mtime": md.as_ref().and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()).unwrap_or(0),
                "sshd_bin": sshd_bin(),
                "valid": validate_content(&content, conf.parent().unwrap_or(std::path::Path::new("/etc/ssh"))).is_ok(),
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 可视化模式提交：原地改键 → 校验 → 备份 → 落盘 →（可选）重启。
pub async fn config_apply(
    path: Option<String>,
    set: Vec<(String, String)>,
    restart: bool,
    dry_run: bool,
) -> Response {
    tokio::task::spawn_blocking(move || -> Response {
        let Ok(conf) = validate_conf_path(path.as_deref().unwrap_or(DEFAULT_SSHD_CONF)) else {
            return Response::err(-1, "配置文件路径非法".to_string());
        };
        let original = match std::fs::read_to_string(&conf) {
            Ok(c) => c,
            Err(e) => return Response::err(-1, format!("读取配置失败: {e}")),
        };
        let next = apply_directives(&original, &set);
        let parent = conf.parent().unwrap_or(std::path::Path::new("/etc/ssh")).to_path_buf();
        if let Err(e) = validate_content(&next, &parent) {
            return Response::err(-1, format!("配置校验未通过,已中止保存：\n{e}"));
        }
        // 只校验：返回「如果保存会是这份内容」,不碰磁盘
        if dry_run {
            return Response::ok(
                "校验通过",
                Some(json!({ "content": next, "dry_run": true })),
            );
        }
        let backup = backup_file(&conf).ok();
        if let Err(e) = std::fs::write(&conf, &next) {
            return Response::err(-1, format!("写入失败: {e}"));
        }
        let (reloaded, reason) = if restart {
            match do_restart() {
                Ok(()) => (true, None),
                Err(e) => {
                    // 重启失败：原样还原并再重启一次,避免把人锁在门外
                    let _ = std::fs::write(&conf, &original);
                    let re = do_restart();
                    (
                        false,
                        Some(format!("重启失败已回滚配置：{e}{}", match re {
                            Ok(()) => String::new(),
                            Err(e2) => format!("（回滚后重启仍失败：{e2}）"),
                        })),
                    )
                }
            }
        } else {
            (false, None)
        };
        Response::ok(
            "ok",
            Some(json!({ "content": next, "backup": backup, "restarted": reloaded, "reason": reason })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 全文（文件编辑）模式：校验 → 备份 → 落盘 →（可选）重启,失败回滚。
pub async fn config_save(
    path: Option<String>,
    content: String,
    restart: bool,
    dry_run: bool,
) -> Response {
    tokio::task::spawn_blocking(move || -> Response {
        let Ok(conf) = validate_conf_path(path.as_deref().unwrap_or(DEFAULT_SSHD_CONF)) else {
            return Response::err(-1, "配置文件路径非法".to_string());
        };
        let parent = conf
            .parent()
            .unwrap_or(std::path::Path::new("/etc/ssh"))
            .to_path_buf();
        if let Err(e) = validate_content(&content, &parent) {
            return Response::err(-1, format!("配置校验未通过,已中止保存：\n{e}"));
        }
        if dry_run {
            return Response::ok("校验通过", Some(json!({ "dry_run": true })));
        }
        let original = std::fs::read_to_string(&conf).unwrap_or_default();
        let backup = backup_file(&conf).ok();
        if let Err(e) = std::fs::write(&conf, &content) {
            return Response::err(-1, format!("写入失败: {e}"));
        }
        let (restarted, reason) = if restart {
            match do_restart() {
                Ok(()) => (true, None),
                Err(e) => {
                    let _ = std::fs::write(&conf, &original);
                    let re = do_restart();
                    (
                        false,
                        Some(format!(
                            "重启失败已回滚配置：{e}{}",
                            match re {
                                Ok(()) => String::new(),
                                Err(e2) => format!("（回滚后重启仍失败：{e2}）"),
                            }
                        )),
                    )
                }
            }
        } else {
            (false, None)
        };
        Response::ok(
            "ok",
            Some(json!({ "backup": backup, "restarted": restarted, "reason": reason })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 重启 sshd：服务名按发行版不同（RHEL `sshd` / Debian 新版 `ssh`）。
fn do_restart() -> Result<(), String> {
    match super::svc::act("restart", "sshd") {
        Ok(()) => Ok(()),
        Err(e) => super::svc::act("restart", "ssh").map_err(|e2| {
            if e.contains(e2.as_str()) {
                e
            } else {
                format!("{e2}（sshd: {e}）")
            }
        }),
    }
}

pub async fn restart() -> Response {
    tokio::task::spawn_blocking(|| {
        // 服务名按发行版不同：RHEL 是 sshd，Debian 新版本是 ssh
        let err = match super::svc::act("restart", "sshd") {
            Ok(()) => return Response::ok("SSH 服务已重启", None),
            Err(e) => e,
        };
        match super::svc::act("restart", "ssh") {
            Ok(()) => Response::ok("SSH 服务已重启", None),
            Err(e2) => Response::err(-1, format!("SSH 重启失败: {e2}（sshd: {err}）")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_replaces_existing_line_in_place() {
        let c = "#Port 22\nPermitRootLogin yes\n";
        let out = apply_directives(c, &[("Port".into(), "2222".into())]);
        assert_eq!(out.lines().next(), Some("Port 2222"), "{out}");
        // 其它行不受影响
        assert!(out.contains("PermitRootLogin yes"), "{out}");
    }

    #[test]
    fn apply_appends_missing_key_before_match_block() {
        let c = "Port 22\n\nMatch User alice\n    PasswordAuthentication no\n";
        let out = apply_directives(c, &[("LoginGraceTime".into(), "60".into())]);
        // 新键必须在 Match 之前（全局段）
        assert!(
            out.find("LoginGraceTime 60").unwrap() < out.find("Match User").unwrap(),
            "{out}"
        );
        // Match 块内容原样保留
        assert!(out.contains("Match User alice"), "{out}");
        assert!(out.contains("    PasswordAuthentication no"), "{out}");
    }

    #[test]
    fn apply_empty_value_comments_the_line() {
        let c = "Port 22\n";
        let out = apply_directives(c, &[("Port".into(), String::new())]);
        assert!(out.starts_with("#Port"), "{out}");
        assert!(!out.lines().any(|l| l.trim() == "Port 22"), "{out}");
    }

    #[test]
    fn apply_is_case_insensitive_and_only_first_hit() {
        let c = "PermitRootLogin yes\nPermitRootLogin no\n";
        let out = apply_directives(c, &[("permitrootlogin".into(), "prohibit-password".into())]);
        assert!(
            out.starts_with("PermitRootLogin prohibit-password"),
            "{out}"
        );
        // 第二处同名保持原样（sshd 取首次出现的值,改第二处没有意义反而误导）
        assert!(out.ends_with("PermitRootLogin no"), "{out}");
    }

    #[test]
    fn directive_of_parses_commented_and_plain() {
        assert_eq!(directive_of("#Port 22"), Some("Port"));
        assert_eq!(
            directive_of("   Subsystem sftp internal-sftp"),
            Some("Subsystem")
        );
        assert_eq!(directive_of(""), None);
        assert_eq!(directive_of("# only comment"), Some("only"));
    }

    #[test]
    fn path_outside_etc_ssh_is_rejected() {
        assert!(validate_conf_path("/etc/passwd").is_err());
        assert!(validate_conf_path("/root/.ssh/config").is_err());
    }
}
