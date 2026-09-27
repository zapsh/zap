//! 站点应用（Application Manager）：把 python / nodejs 之类的用户进程托管起来。
//!
//! 设计要点：
//! - **应用永远以站点归属的 unix 用户运行**：一个应用 = 一个 systemd unit，
//!   `[Service] User=<站点用户>`，绝不会以 root 身份启动用户代码
//! - 依赖安装（pip / npm）同样以站点用户身份跑，产物（`.venv` / `node_modules`）归站点用户
//! - 工作目录强制收敛在 `/home/<站点用户>/` 之内，越界直接拒绝
//! - 日志落到站点日志目录下的 `app-<name>.log`（面板可 tail）
//! - 新增应用类型 = `zap_proto::APP_TYPES` 加一项 + 本文件补一个「依赖准备 + 默认命令」分支

use std::path::{Path, PathBuf};

use serde_json::json;
use zap_proto::{Response, app_type_supported};

use super::root_cmd;

// ── 校验 ────────────────────────────────────────────────

/// 应用名：只允许 systemd unit 名安全字符集（也决定 unit 文件名）
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// unix 用户名：小写开头，且**不允许 root**
fn valid_user(u: &str) -> bool {
    !u.is_empty()
        && u != "root"
        && u.len() <= 32
        && u.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && u.starts_with(|c: char| c.is_ascii_lowercase())
}

/// unit 是逐行解析的：任何写进去的字段都不能含换行，否则等于给用户一个
/// 「追加任意 systemd 指令」的口子（`ExecStart=` 后面再塞一行 `ExecStart=` 之类）
fn no_newline(s: &str) -> bool {
    !s.contains('\n') && !s.contains('\r')
}

fn systemd_quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// 工作目录必须真实存在，且落在站点用户的家目录之内
fn check_workdir(workdir: &str, owner: &str) -> Result<PathBuf, String> {
    if !workdir.starts_with('/') {
        return Err("工作目录必须是绝对路径".to_string());
    }
    let home = format!("/home/{owner}");
    let p = PathBuf::from(workdir);
    let real = p
        .canonicalize()
        .map_err(|_| format!("工作目录不存在或不可访问：{workdir}"))?;
    let home_real = PathBuf::from(&home)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&home));
    if !real.starts_with(&home_real) {
        return Err(format!("工作目录必须在 {home} 之内"));
    }
    Ok(real)
}

/// 应用日志超过这个体积就在重新部署时归档一份（100 MB）
const APP_LOG_MAX_BYTES: u64 = 100 * 1024 * 1024;

fn unit_name(site_id: i64, name: &str) -> String {
    format!("zap-app-{site_id}-{name}.service")
}

fn unit_path(site_id: i64, name: &str) -> PathBuf {
    PathBuf::from("/etc/systemd/system").join(unit_name(site_id, name))
}

// ── 以站点用户身份执行命令 ───────────────────────────────

/// 以 `user` 身份在 `workdir` 下执行一条 shell。root 端只负责切身份，不留 shell 注入面
/// （命令通过环境变量传递，不做字符串拼接）。
fn run_as(user: &str, workdir: &Path, cmd: &str) -> Result<(bool, String), String> {
    let bash = if Path::new("/bin/bash").exists() {
        "/bin/bash"
    } else {
        "/bin/sh"
    };
    let mut c = root_cmd(bash);
    c.arg("-c")
        .arg(format!(
            "if command -v runuser >/dev/null 2>&1; then \
               exec runuser -u \"$ZAP_RUN_USER\" -- {bash} -c \"$ZAP_RUN_CMD\"; \
             fi; \
             exec su -s {bash} -c \"$ZAP_RUN_CMD\" \"$ZAP_RUN_USER\""
        ))
        .env("ZAP_RUN_USER", user)
        .env("ZAP_RUN_CMD", cmd)
        .env("HOME", format!("/home/{user}"))
        .current_dir(workdir);
    let out = c.output().map_err(|e| format!("执行命令失败: {e}"))?;
    let merged = if out.status.success() {
        String::from_utf8_lossy(&out.stdout).to_string()
    } else {
        let mut s = String::from_utf8_lossy(&out.stdout).to_string();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        s
    };
    Ok((out.status.success(), merged))
}

fn systemctl(args: &[&str]) -> Result<(bool, String), String> {
    let mut c = root_cmd("/usr/bin/systemctl");
    let out = c
        .args(args)
        .output()
        .map_err(|e| format!("systemctl 执行失败: {e}"))?;
    let s = if out.status.success() {
        String::from_utf8_lossy(&out.stdout).to_string()
    } else {
        let mut s = String::from_utf8_lossy(&out.stdout).to_string();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        s
    };
    Ok((out.status.success(), s.trim().to_string()))
}

// ── 类型相关：依赖准备 + 默认启动命令 ────────────────────

/// 安装依赖（以站点用户身份）。返回人类可读的进度说明。
fn prepare_deps(app_type: &str, workdir: &Path, owner: &str, install: bool) -> Result<String, String> {
    if !install {
        return Ok(String::new());
    }
    let mut log = String::new();
    match app_type {
        "python" => {
            let req = workdir.join("requirements.txt");
            if !req.exists() {
                return Ok(log);
            }
            let venv = workdir.join(".venv");
            if !venv.join("bin/python").exists() {
                let (ok, out) = run_as(owner, workdir, "python3 -m venv .venv")?;
                if !ok {
                    return Err(format!("创建虚拟环境失败：{out}"));
                }
                log.push_str("已创建 .venv\n");
            }
            let (ok, out) = run_as(owner, workdir, ".venv/bin/pip install -r requirements.txt")?;
            if !ok {
                return Err(format!("pip install 失败：{out}"));
            }
            log.push_str("依赖安装完成（pip）\n");
        }
        "nodejs" => {
            let pkg = workdir.join("package.json");
            if !pkg.exists() {
                return Ok(log);
            }
            let cmd = if workdir.join("package-lock.json").exists() {
                "npm ci"
            } else {
                "npm install"
            };
            let (ok, out) = run_as(owner, workdir, cmd)?;
            if !ok {
                return Err(format!("{cmd} 失败：{out}"));
            }
            log.push_str(&format!("依赖安装完成（{cmd}）\n"));
        }
        _ => {}
    }
    Ok(log)
}

/// 推导默认启动命令（用户填了 `command` 就不走这里）
fn default_command(app_type: &str, workdir: &Path, entry: &str, port: i64) -> Result<String, String> {
    match app_type {
        "python" => {
            let venv_py = workdir.join(".venv/bin/python");
            let py = if venv_py.exists() {
                workdir.join(".venv/bin/python").to_string_lossy().to_string()
            } else {
                "python3".to_string()
            };
            if entry.contains(':') {
                // module:app 形态：优先 gunicorn，其次 uvicorn
                let gunicorn = workdir.join(".venv/bin/gunicorn");
                let uvicorn = workdir.join(".venv/bin/uvicorn");
                if gunicorn.exists() {
                    return Ok(format!(
                        "{} -b 127.0.0.1:{} {}",
                        gunicorn.to_string_lossy(),
                        port,
                        entry
                    ));
                }
                if uvicorn.exists() {
                    return Ok(format!(
                        "{} --host 127.0.0.1 --port {} {}",
                        uvicorn.to_string_lossy(),
                        port,
                        entry
                    ));
                }
                return Err(
                    "入口是 `模块:应用` 形态，需要 gunicorn 或 uvicorn：\
                     请把它们写进 requirements.txt，或在「启动命令」里自定义"
                        .to_string(),
                );
            }
            Ok(format!("{py} {entry}"))
        }
        "nodejs" => {
            if !entry.is_empty() {
                return Ok(format!("node {entry}"));
            }
            // 未填入口：优先 package.json 的 start 脚本，其次常见入口文件
            if let Ok(txt) = std::fs::read_to_string(workdir.join("package.json")) {
                if txt.contains("\"start\"") {
                    return Ok("npm start".to_string());
                }
            }
            for f in ["server.js", "app.js", "index.js", "main.js"] {
                if workdir.join(f).exists() {
                    return Ok(format!("node {f}"));
                }
            }
            Err("未找到入口：请填写入口文件（如 server.js）或在「启动命令」里自定义".to_string())
        }
        _ => Err(format!("不支持的应用类型：{app_type}")),
    }
}

// ── unit 渲染 ───────────────────────────────────────────

fn render_unit(
    site_id: i64,
    name: &str,
    owner: &str,
    workdir: &Path,
    exec: &str,
    port: i64,
    env: &str,
    log_file: Option<&Path>,
) -> String {
    let mut u = String::new();
    u.push_str("[Unit]\n");
    u.push_str(&format!(
        "Description=Zap App {name} (site {site_id})\n"
    ));
    u.push_str("After=network.target\n\n");
    u.push_str("[Service]\n");
    u.push_str("Type=simple\n");
    // 关键：用户代码以站点用户身份运行
    u.push_str(&format!("User={owner}\n"));
    u.push_str(&format!("WorkingDirectory={}\n", workdir.to_string_lossy()));
    for line in env.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        u.push_str(&format!("Environment=\"{}\"\n", systemd_quote(t)));
    }
    if port > 0 {
        u.push_str(&format!("Environment=\"PORT={port}\"\n"));
    }
    u.push_str(&format!("ExecStart={exec}\n"));
    u.push_str("Restart=always\n");
    u.push_str("RestartSec=3\n");
    // 加固：不允许提权、独立 /tmp
    u.push_str("NoNewPrivileges=yes\n");
    u.push_str("PrivateTmp=yes\n");
    if let Some(f) = log_file {
        let p = f.to_string_lossy();
        u.push_str(&format!("StandardOutput=append:{p}\n"));
        u.push_str(&format!("StandardError=append:{p}\n"));
    }
    u.push_str("\n[Install]\n");
    u.push_str("WantedBy=multi-user.target\n");
    u
}

fn app_log_file(log_dir: &str, name: &str) -> Option<PathBuf> {
    if log_dir.is_empty() {
        return None;
    }
    Some(PathBuf::from(log_dir).join(format!("app-{name}.log")))
}

async fn blocking<F>(f: F) -> Response
where
    F: FnOnce() -> Result<Response, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => Response::err(-1, e),
        Err(e) => Response::err(-1, format!("任务执行失败: {e}")),
    }
}

// ── 对外动词 ────────────────────────────────────────────

/// 部署（幂等）：准备依赖 -> 渲染 unit -> daemon-reload -> 重启
#[allow(clippy::too_many_arguments)]
pub async fn deploy(
    site_id: i64,
    name: &str,
    app_type: &str,
    workdir: &str,
    entry: &str,
    command: &str,
    port: i64,
    env: &str,
    autostart: bool,
    install_deps: bool,
    owner_user: &str,
    log_dir: &str,
) -> Response {
    let (name, app_type, workdir, entry, command, env, owner_user, log_dir) = (
        name.to_string(),
        app_type.to_string(),
        workdir.to_string(),
        entry.to_string(),
        command.to_string(),
        env.to_string(),
        owner_user.to_string(),
        log_dir.to_string(),
    );
    blocking(move || {
        if !valid_name(&name) {
            return Err("应用名只能包含字母、数字、- 和 _".to_string());
        }
        if !valid_user(&owner_user) {
            return Err("站点用户不合法（且不能是 root）".to_string());
        }
        if !app_type_supported(&app_type) {
            return Err(format!("不支持的应用类型：{app_type}"));
        }
        for (f, v) in [
            ("工作目录", workdir.as_str()),
            ("入口", entry.as_str()),
            ("启动命令", command.as_str()),
            ("环境变量", env.as_str()),
        ] {
            if !no_newline(v) {
                return Err(format!("{f} 不能包含换行"));
            }
        }
        let wd = check_workdir(&workdir, &owner_user)?;

        let mut steps = prepare_deps(&app_type, &wd, &owner_user, install_deps)?;

        let exec = if !command.trim().is_empty() {
            command.trim().to_string()
        } else {
            default_command(&app_type, &wd, &entry, port)?
        };

        // 应用日志落站点日志目录，属主给站点用户（否则 systemd 以该用户写不进去）
        let log_file = app_log_file(&log_dir, &name);
        if let Some(f) = &log_file {
            if let Some(parent) = f.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // 重部署时若日志已过大就先归档一份，避免长期运行的站点被日志撑爆磁盘
            // （应用日志不参与站点日志轮转，这里是最省事的兜底）
            if let Ok(meta) = f.metadata() {
                if meta.len() > APP_LOG_MAX_BYTES {
                    let old = f.with_extension("log.1");
                    let _ = std::fs::rename(f, old);
                }
            }
            if !f.exists() {
                let _ = std::fs::write(f, "");
            }
            let _ = std::process::Command::new("chown")
                .args([format!("{owner_user}:{owner_user}")])
                .arg(f)
                .output();
        }

        let unit = render_unit(
            site_id,
            &name,
            &owner_user,
            &wd,
            &exec,
            port,
            &env,
            log_file.as_deref(),
        );
        let path = unit_path(site_id, &name);
        std::fs::write(&path, unit).map_err(|e| format!("写入 unit 失败: {e}"))?;

        let unit = unit_name(site_id, &name);
        systemctl(&["daemon-reload"])?;
        if autostart {
            let _ = systemctl(&["enable", &unit]);
        } else {
            let _ = systemctl(&["disable", &unit]);
        }
        let (ok, out) = systemctl(&["restart", &unit])?;
        if !ok {
            return Err(format!("启动失败：{out}"));
        }
        steps.push_str("已部署并启动\n");
        Ok(Response::ok(
            "部署完成",
            Some(json!({ "steps": steps, "unit": unit, "exec": exec })),
        ))
    })
    .await
}

/// start | stop | restart | enable | disable
pub async fn action(site_id: i64, name: &str, action: &str) -> Response {
    let (name, action) = (name.to_string(), action.to_string());
    blocking(move || {
        if !valid_name(&name) {
            return Err("应用名不合法".to_string());
        }
        let unit = unit_name(site_id, &name);
        match action.as_str() {
            "start" | "stop" | "restart" | "enable" | "disable" => {
                let (ok, out) = systemctl(&[action.as_str(), &unit])?;
                if !ok {
                    return Err(format!("{action} 失败：{out}"));
                }
            }
            other => return Err(format!("未知动作：{other}")),
        }
        Ok(Response::ok("ok", None))
    })
    .await
}

/// 批量查询实时状态：active / enabled / pid
pub async fn status(site_id: i64, names: &[String]) -> Response {
    let names = names.to_vec();
    blocking(move || {
        let mut list = Vec::new();
        for n in names {
            if !valid_name(&n) {
                continue;
            }
            let unit = unit_name(site_id, &n);
            let (_, active) = systemctl(&["is-active", &unit]).unwrap_or((false, "unknown".into()));
            let (_, enabled) =
                systemctl(&["is-enabled", &unit]).unwrap_or((false, "disabled".into()));
            let (_, pid) = systemctl(&["show", "-p", "MainPID", "--value", &unit])
                .unwrap_or((false, "0".into()));
            list.push(json!({
                "name": n,
                "active": active == "active",
                "state": active,
                "enabled": enabled == "enabled",
                "pid": pid.parse::<i64>().unwrap_or(0),
            }));
        }
        Ok(Response::ok("ok", Some(json!({ "apps": list }))))
    })
    .await
}

pub async fn remove(site_id: i64, name: &str) -> Response {
    let name = name.to_string();
    blocking(move || {
        if !valid_name(&name) {
            return Err("应用名不合法".to_string());
        }
        let unit = unit_name(site_id, &name);
        let _ = systemctl(&["stop", &unit]);
        let _ = systemctl(&["disable", &unit]);
        let p = unit_path(site_id, &name);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("删除 unit 失败: {e}"))?;
        }
        systemctl(&["daemon-reload"])?;
        Ok(Response::ok("已删除", None))
    })
    .await
}

pub async fn log(site_id: i64, name: &str, lines: usize) -> Response {
    let name = name.to_string();
    blocking(move || {
        let _ = site_id;
        if !valid_name(&name) {
            return Err("应用名不合法".to_string());
        }
        let unit = unit_name(site_id, &name);
        // 优先文件日志；没有配置日志目录时退回 journalctl
        let out = systemctl(&["show", "-p", "StandardOutput", &unit])
            .map(|(_, v)| v)
            .unwrap_or_default();
        let content = if let Some(p) = out.strip_prefix("append:") {
            std::fs::read_to_string(p).unwrap_or_default()
        } else {
            let mut c = root_cmd("/usr/bin/journalctl");
            let o = c
                .args(["-u", &unit, "-n", &lines.to_string(), "--no-pager"])
                .output();
            o.map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default()
        };
        let tail: Vec<&str> = content.lines().rev().take(lines).collect();
        let text: Vec<String> = tail.into_iter().rev().map(|l| l.to_string()).collect();
        Ok(Response::ok("ok", Some(json!({ "lines": text }))))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("zap-app-test-{name}"));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn name_charset_is_enforced() {
        assert!(valid_name("my-app_1"));
        assert!(!valid_name(""));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("../etc"));
        assert!(!valid_name("app name"));
    }

    #[test]
    fn root_is_never_a_valid_run_user() {
        assert!(!valid_user("root"));
        assert!(valid_user("admin"));
        assert!(!valid_user("Admin"));
    }

    #[test]
    fn workdir_must_stay_inside_site_user_home() {
        assert!(check_workdir("/etc", "admin").is_err());
        assert!(check_workdir("relative/path", "admin").is_err());
    }

    #[test]
    fn python_module_app_uses_gunicorn_when_present() {
        let d = tmp("py");
        std::fs::create_dir_all(d.join(".venv/bin")).unwrap();
        std::fs::write(d.join(".venv/bin/gunicorn"), "").unwrap();
        let cmd = default_command("python", &d, "wsgi:app", 8000).unwrap();
        assert!(cmd.contains("gunicorn"));
        assert!(cmd.contains("-b 127.0.0.1:8000"));
        assert!(cmd.ends_with("wsgi:app"));
    }

    #[test]
    fn python_script_uses_venv_python() {
        let d = tmp("py2");
        std::fs::create_dir_all(d.join(".venv/bin")).unwrap();
        std::fs::write(d.join(".venv/bin/python"), "").unwrap();
        let cmd = default_command("python", &d, "main.py", 0).unwrap();
        assert!(cmd.contains(".venv/bin/python"));
        assert!(cmd.ends_with("main.py"));
    }

    #[test]
    fn nodejs_falls_back_to_npm_start() {
        let d = tmp("node");
        std::fs::write(d.join("package.json"), r#"{"scripts":{"start":"node s.js"}}"#).unwrap();
        assert_eq!(default_command("nodejs", &d, "", 3000).unwrap(), "npm start");
        std::fs::write(d.join("package.json"), r#"{"name":"x"}"#).unwrap();
        assert!(default_command("nodejs", &d, "", 3000).is_err());
    }

    #[test]
    fn unit_renders_with_site_user_and_restart() {
        let u = render_unit(
            1,
            "demo",
            "admin",
            Path::new("/home/admin/w4u.cn/app"),
            "/home/admin/w4u.cn/app/.venv/bin/python main.py",
            8000,
            "DEBUG=1\nSECRET=x",
            Some(Path::new("/home/admin/logs/1-w4u-cn/app-demo.log")),
        );
        assert!(u.contains("User=admin"));
        assert!(u.contains("WorkingDirectory=/home/admin/w4u.cn/app"));
        assert!(u.contains("ExecStart=/home/admin/w4u.cn/app/.venv/bin/python main.py"));
        assert!(u.contains("Restart=always"));
        assert!(u.contains("Environment=\"DEBUG=1\""));
        assert!(u.contains("Environment=\"PORT=8000\""));
        assert!(u.contains("StandardOutput=append:/home/admin/logs/1-w4u-cn/app-demo.log"));
        assert!(u.contains("NoNewPrivileges=yes"));
        assert_eq!(u.matches("ExecStart=").count(), 1);
    }
}
