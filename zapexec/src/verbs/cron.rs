//! 面板用户的计划任务（crontab）：以指定 Linux 账号后台运行一条命令 / 脚本。
//!
//! 由 zapd 的 `zap::user_cron` 调度器 / 手动触发调用，输出追加写入调用方指定的
//! 日志文件（须位于 `{ZAP_PATH}/data/users/` 之下），结束后追加
//! `__ZAP_DONE__ <exit_code>` 供 zapd 判定成败。
//!
//! 权限边界：`linux_user` 必须是真实存在的系统账号；非 admin 面板用户由 zapd
//! 在调用前收敛为「其自身账号」，zapexec 只做合法性与存在性校验。

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::json;
use zap_proto::Response;

use super::root_cmd;

fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

/// 合法 Linux 账号名：小写字母 / 数字 / 下划线 / 连字符，1..=32 位且以字母数字开头。
fn valid_linux_user(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
}

fn user_exists(name: &str) -> bool {
    let Ok(cname) = std::ffi::CString::new(name) else {
        return false;
    };
    unsafe {
        let pw = libc::getpwnam(cname.as_ptr());
        if pw.is_null() {
            return false;
        }
        // 禁止以 root 运行普通用户任务
        (*pw).pw_uid != 0
    }
}

/// 日志路径校验：绝对路径、无 `..`、位于 `{ZAP_PATH}/data/users/` 之下且以 `.log` 结尾。
///
/// `pub(super)`：镜像构建（`docker::image_build`）等长任务共用同一套约定 ——
/// 运行日志只允许落在面板自管的 `data/users/` 之下，避免被诱导写任意文件。
pub(super) fn safe_log_path(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err("日志路径必须是绝对路径".into());
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("日志路径不合法".into());
    }
    let base = zap_path().join("data").join("users");
    if !p.starts_with(&base) {
        return Err("日志路径必须位于 data/users 之下".into());
    }
    if p.extension().and_then(|e| e.to_str()) != Some("log") {
        return Err("日志文件必须以 .log 结尾".into());
    }
    Ok(p)
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 以 `linux_user` 身份后台运行 `command`，stdout/stderr 追加写入 `log_path`。
pub async fn run(
    run_id: String,
    linux_user: String,
    home_dir: String,
    command: String,
    kind: String,
    log_path: String,
) -> Response {
    let joined = tokio::task::spawn_blocking(move || -> Result<Response, String> {
        if !valid_linux_user(&linux_user) {
            return Err("运行账号不合法".into());
        }
        if !user_exists(&linux_user) {
            return Err(format!("运行账号不存在或不可用: {linux_user}"));
        }
        let cmd = command.trim();
        if cmd.is_empty() {
            return Err("执行内容不能为空".into());
        }
        if cmd.len() > 4096 {
            return Err("执行内容过长（上限 4096 字节）".into());
        }
        let log_path = safe_log_path(&log_path)?;
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        // kind=script：以 `bash <脚本>` 方式执行，不依赖可执行位 / shebang
        let run_cmd = if kind == "script" {
            let script = PathBuf::from(cmd);
            if !script.is_absolute() {
                return Err("脚本路径必须是绝对路径".into());
            }
            if !script.is_file() {
                return Err(format!("脚本不存在: {cmd}"));
            }
            format!("/bin/bash -- {}", sh_quote(cmd))
        } else {
            cmd.to_string()
        };

        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| format!("打开日志失败: {e}"))?;
        let _ = writeln!(log, "=== [{}] crontab run {run_id} ===", now_ts());
        let _ = writeln!(log, "user: {linux_user}");
        let _ = writeln!(
            log,
            "cwd : {}",
            if home_dir.is_empty() {
                "-"
            } else {
                home_dir.as_str()
            }
        );
        let _ = writeln!(log, "cmd : {cmd}");
        let _ = writeln!(log, "---");
        let _ = log.flush();
        let log_stderr = log.try_clone().map_err(|e| e.to_string())?;

        let cwd = if home_dir.is_empty() {
            None
        } else {
            let dir = Path::new(&home_dir);
            dir.is_dir().then(|| dir.to_path_buf())
        };

        let mut c = root_cmd("/bin/bash");
        c.arg("-c")
            .arg(
                // runuser 优先（不依赖 PAM 会话），回退 su；两者都不登录，
                // 仅切换执行身份，保留调用方设置的工作目录与环境。
                "if command -v runuser >/dev/null 2>&1; then \
                   exec runuser -u \"$ZAP_RUN_USER\" -- /bin/bash -c \"$ZAP_RUN_CMD\"; \
                 fi; \
                 exec su -s /bin/bash -c \"$ZAP_RUN_CMD\" \"$ZAP_RUN_USER\"",
            )
            .env("ZAP_RUN_USER", &linux_user)
            .env("ZAP_RUN_CMD", &run_cmd)
            .env("ZAP_USER", &linux_user)
            .env("LOGNAME", &linux_user)
            .env("USER", &linux_user)
            .env(
                "HOME",
                if home_dir.is_empty() {
                    "/tmp"
                } else {
                    home_dir.as_str()
                },
            )
            .stdout(std::process::Stdio::from(log))
            .stderr(std::process::Stdio::from(log_stderr));
        if let Some(dir) = cwd {
            c.current_dir(dir);
        }

        let mut child = c.spawn().map_err(|e| format!("启动任务失败: {e}"))?;
        let pid = child.id();
        let log_for_done = log_path.clone();
        std::thread::spawn(move || {
            let code = child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&log_for_done) {
                let _ = writeln!(f, "\n__ZAP_DONE__ {code}");
                let _ = f.flush();
            }
        });

        Ok(Response::ok(
            "计划任务已启动",
            Some(json!({
                "run_id": run_id,
                "pid": pid,
                "log": log_path.to_string_lossy(),
            })),
        ))
    })
    .await;

    match joined {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => Response::err(-1, e),
        Err(e) => Response::err(-1, format!("任务执行失败: {e}")),
    }
}
