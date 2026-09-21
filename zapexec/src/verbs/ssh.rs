use serde_json::json;
use std::io::Write;
use std::path::PathBuf;

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
        // 服务名按平台/发行版不同：OpenBSD 与 RHEL 是 sshd，Debian 新版本是 ssh
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

pub async fn restart() -> Response {
    tokio::task::spawn_blocking(|| {
        // 服务名按平台/发行版不同：OpenBSD 与 RHEL 是 sshd，Debian 新版本是 ssh
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
