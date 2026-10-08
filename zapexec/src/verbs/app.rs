// SPDX-License-Identifier: AGPL-3.0-only
//! 站点应用（Application Manager）：把 python / nodejs 之类的用户进程托管起来。
//!
//! 设计要点：
//! - **应用永远以站点归属的 unix 用户运行**：一个应用 = 一个 systemd unit，
//!   `[Service] User=<站点用户>`，绝不会以 root 身份启动用户代码
//! - 依赖安装（pip / npm）同样以站点用户身份跑，产物（`.venv` / `node_modules`）归站点用户
//! - 工作目录强制收敛在 `/home/<站点用户>/` 之内，越界直接拒绝
//! - 日志落到站点日志目录下的 `app-<name>.log`（面板可 tail）
//! - 新增应用类型 = `zap_proto::APP_TYPES` 加一项 + 本文件补一个「依赖准备 + 默认命令」分支

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

use serde_json::json;
use zap_proto::{Response, app_type_supported};

use super::env::uv_bin;
use super::env::UV_PYTHON_INSTALL_DIR;
use super::log_line;
use super::root_cmd;

// ── 运行时版本探测 ──────────────────────────────────────

/// 版本号只允许 `3.11` / `3` / `20` 这种点分数字：它会被拼进命令里
/// （`python3.11 -m venv`），绝不能带空格或 shell 元字符。
fn valid_version(v: &str) -> bool {
    !v.is_empty() && v.len() <= 16 && v.chars().all(|c| c.is_ascii_digit() || c == '.')
}

// ── Go / Rust 工具链（编译型，单版本，管理员手动安装）─────────────

/// Go / Rust 工具链可执行文件所在目录：管理员装到这些位置之一即可，无需多版本管理。
/// 编译型语言只在「构建」阶段需要工具链；运行时是 workdir 下的原生二进制，不需要工具链。
fn toolchain_bin_dir(app_type: &str) -> Option<String> {
    let bin = match app_type {
        "go" => "go",
        "rust" => "cargo",
        _ => return None,
    };
    for base in [
        "/usr/local/bin",
        "/usr/bin",
        "/usr/local/go/bin",
        "/root/.cargo/bin",
    ] {
        if Path::new(base).join(bin).exists() {
            return Some(base.to_string());
        }
    }
    None
}

/// 构建命令前把工具链目录塞进 PATH（找不到就交给 shell 自己找），
/// 免得站点用户 `su` 环境里找不到 go / cargo。
fn with_toolchain_path(app_type: &str, cmd: &str) -> String {
    match toolchain_bin_dir(app_type) {
        Some(dir) => format!("PATH={dir}:$PATH {cmd}"),
        None => cmd.to_string(),
    }
}

/// 探测工具链是否已安装（供部署向导提示）：已知目录 OR 当前 PATH 里能找到。
fn toolchain_installed(app_type: &str) -> bool {
    if toolchain_bin_dir(app_type).is_some() {
        return true;
    }
    let bin = match app_type {
        "go" => "go",
        "rust" => "cargo",
        _ => return false,
    };
    std::process::Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 扫目录里形如 `python3.11` 的可执行文件，收集次要版本号（降序去重）
fn scan_python_versions(dir: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut vs: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            let rest = n.strip_prefix("python")?;
            // 只收 `python3.x`（含 `python3`），跳过 python2 / python3-config 之类
            if !rest.starts_with('3') {
                return None;
            }
            if rest != "3" && !rest.starts_with("3.") {
                return None;
            }
            Some(rest.to_string())
        })
        .collect();
    vs.sort();
    vs.dedup();
    vs.reverse();
    vs
}

/// node 版本：系统 node + 常见的多版本安装目录（nvm / n / nodejs 官方包）
fn scan_node_versions() -> Vec<String> {
    let mut vs: Vec<String> = Vec::new();
    // 系统默认 node
    if let Some(v) = node_version_of("node") {
        vs.push(v);
    }
    // 多版本管理器 / 官方分发：每个子目录里的 bin/node
    for base in [
        "/usr/local/n/versions/node",
        "/usr/local/lib/nodejs",
        "/opt/nodejs",
    ] {
        if let Ok(rd) = std::fs::read_dir(base) {
            let mut subs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            subs.sort();
            for p in subs {
                let bin = p.join("bin/node");
                if bin.exists() {
                    if let Some(v) = node_version_of(&bin.to_string_lossy()) {
                        vs.push(v);
                    }
                }
            }
        }
    }
    // 各用户 home 下的 nvm（以 root 身份也能读到）
    if let Ok(rd) = std::fs::read_dir("/home") {
        for e in rd.flatten() {
            let nvm = e.path().join(".nvm/versions/node");
            if let Ok(inner) = std::fs::read_dir(&nvm) {
                let mut subs: Vec<PathBuf> = inner.flatten().map(|x| x.path()).collect();
                subs.sort();
                for p in subs {
                    let bin = p.join("bin/node");
                    if bin.exists() {
                        if let Some(v) = node_version_of(&bin.to_string_lossy()) {
                            vs.push(v);
                        }
                    }
                }
            }
        }
    }
    // node 只认大版本（18 / 20 / 22）
    let mut major: Vec<String> = vs
        .iter()
        .filter_map(|v| v.split('.').next())
        .map(|s| s.to_string())
        .collect();
    major.sort();
    major.dedup();
    major.reverse();
    major
}

fn node_version_of(bin: &str) -> Option<String> {
    let o = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    let t = text.trim().trim_start_matches('v');
    if t.is_empty() || !t.chars().next()?.is_ascii_digit() {
        return None;
    }
    Some(t.to_string())
}

/// `app.runtimes`：返回服务器上已安装的版本，供部署向导下拉
pub async fn runtimes() -> Response {
    blocking(move || {
        let mut py = scan_python_versions("/usr/bin");
        py.extend(scan_python_versions("/usr/local/bin"));
        // uv 管理的解释器也要列出来（系统 /usr/bin 下没有对应二进制）
        if let Some(v) = super::env::detect_python().get("versions") {
            if let Some(arr) = v.as_array() {
                for item in arr {
                    if let Some(ver) = item.get("version").and_then(|x| x.as_str()) {
                        py.push(ver.to_string());
                    }
                }
            }
        }
        py.sort();
        py.dedup();
        py.reverse();
        let data = json!({
            "python": py,
            "nodejs": scan_node_versions(),
            "go": toolchain_installed("go"),
            "rust": toolchain_installed("rust"),
            });
        Ok(Response::ok("运行时版本探测完成", Some(data)))
    })
    .await
}

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

/// 越权防护（defense-in-depth）：普通用户部署的应用，其运行账号必须等于请求方本人，
/// 否则可借 `owner_user` 把应用以他人站点用户身份拉起（跨站点篡改 / 提权）。
/// 管理员（`skip_owner_check`）不受此约束（但其站点归属仍由 zapd 校验）。
fn ensure_owner_matches(
    owner_user: &str,
    requester: &Option<String>,
    skip_owner_check: bool,
) -> Result<(), String> {
    if skip_owner_check {
        return Ok(());
    }
    match requester.as_deref() {
        Some(r) if r == owner_user => Ok(()),
        _ => Err(
            "应用必须以请求方本人的站点用户部署（owner_user 与请求用户不一致）".to_string(),
        ),
    }
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
        // uv 默认只看执行者家目录：带上共享解释器目录，站点用户建 .venv
        // 时才能找到 root 统一安装的 Python 版本（目录本身只读）。
        .env("UV_PYTHON_INSTALL_DIR", UV_PYTHON_INSTALL_DIR)
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

/// 把可能很长的命令输出裁剪到末尾 N 个字符，避免一次性把 npm/pip 的万字输出灌进部署日志。
/// 诊断时真正有用的报错通常在尾部，故保留末尾。
fn clip(s: &str) -> String {
    const MAX: usize = 4000;
    let n: usize = s.chars().count();
    if n <= MAX {
        return s.to_string();
    }
    let skip = n - MAX;
    format!("…（前 {skip} 字符已省略）\n{}", s.chars().skip(skip).collect::<String>())
}

/// 部署被取消时 `app_deploy_stop` 写下的哨兵文案：所有步骤失败/返回时据此判断是否已取消。
const DEPLOY_CANCELED: &str = "部署已取消";

/// 把 `log` 路径的 `.log` 后缀换成 `ext`（同目录下生成 `.pid` / `.cancel` 哨兵）。
fn swap_ext(log: &str, ext: &str) -> String {
    if let Some(s) = log.strip_suffix(".log") {
        format!("{s}.{ext}")
    } else {
        format!("{log}.{ext}")
    }
}

/// 部署任务日志会被三方写入：zapexec(root)、zapd(面板用户)、站点用户
/// （`run_as_stream` 里 `( cmd ) >> log` 的重定向以站点用户身份执行）。
/// 这里以 root 预创建并放开为 0o666，同时**逐层给日志路径的祖先目录补 `o+x`（穿越位）**——
/// 否则这些目录若落在受限 home 内（如开发机 `/home/zap` 为 0700），站点用户无 `x` 位便无法
/// 访问日志文件，表现为 `( cmd ) >> log` 报 `Permission denied`，git/npm 输出全丢、失败原因
/// 与 `__ZAP_DONE__` 完成标记都写不进日志。
fn prepare_task_log(log: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = std::path::Path::new(log);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
        // 逐层确保 others 有执行(x)位，让站点用户能穿透到日志文件。
        // 只加 x 不加 w/r，不开放目录的写/列读权限。
        let mut cur = Some(parent.to_path_buf());
        while let Some(mut p) = cur {
            if let Ok(meta) = std::fs::metadata(&p) {
                let mode = meta.permissions().mode();
                if mode & 0o001 == 0 {
                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode | 0o001));
                }
            }
            if !p.pop() {
                break;
            }
            cur = Some(p);
        }
    }
    if !path.exists() {
        let _ = std::fs::write(path, "");
    }
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666));
}

/// 部署任务是否被请求取消：zapd 的 `AppDeployStop` 会写 `<log>.cancel` 哨兵。
fn deploy_canceled(log: &str) -> bool {
    std::path::Path::new(&swap_ext(log, "cancel")).exists()
}

/// 以站点用户身份跑一条命令，输出**实时追加**到 `log` 文件（前端 WebSocket 边跑边看）；
/// 命令以 `setsid` 自立进程组，当前子进程 pid 写入 `pid_path`，便于 `AppDeployStop`
/// 按进程组 `kill(-pid)` 整组终止（git clone / npm install 这类分钟级任务需要可中断）。
/// 返回命令是否成功退出；若进程是被取消信号打死的且已取消，返回 `Err(DEPLOY_CANCELED)`。
fn run_as_stream(
    log: &str,
    user: &str,
    workdir: &Path,
    cmd: &str,
    pid_path: &str,
) -> Result<bool, String> {
    let bash = if Path::new("/bin/bash").exists() {
        "/bin/bash"
    } else {
        "/bin/sh"
    };
    // 把用户命令包进子 shell，确保末尾的重定向对整个复合命令生效；
    // 日志路径做单引号转义，避免路径里出现引号 / 空格 / 特殊字符破坏命令。
    let safe = log.replace('\\', "\\\\").replace('\'', "'\\''");
    let redirected = format!("( {cmd} ) >> '{safe}' 2>&1");
    let mut c = root_cmd(bash);
    c.arg("-c")
        .arg(format!(
            "if command -v runuser >/dev/null 2>&1; then \
               exec runuser -u \"$ZAP_RUN_USER\" -- {bash} -c \"$ZAP_RUN_CMD\" 2>> '{safe}'; \
             fi; \
             exec su -s {bash} -c \"$ZAP_RUN_CMD\" \"$ZAP_RUN_USER\" 2>> '{safe}'"
        ))
        .env("ZAP_RUN_USER", user)
        .env("ZAP_RUN_CMD", redirected)
        .env("HOME", format!("/home/{user}"))
        .env("UV_PYTHON_INSTALL_DIR", UV_PYTHON_INSTALL_DIR)
        .current_dir(workdir);
    // 自立进程组（setsid）：取消时 `kill(-pid)` 才能精准只杀本步的子树，
    // 不会误伤 zapexec 主进程或其它并发任务。
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = c
        .spawn()
        .map_err(|e| format!("启动命令失败: {e}"))?;
    // 记录当前步骤的进程组 leader pid，供取消逻辑整组终止
    let _ = std::fs::write(pid_path, child.id().to_string());
    let out = child
        .wait()
        .map_err(|e| format!("等待命令失败: {e}"))?;
    if !out.success() && !deploy_canceled(log) {
        // 失败时把退出码落进日志：上游只报"git 拉取失败"这类笼统信息，
        // 退出码（如 127=命令不存在、128=git 致命错误）是关键排查线索。
        let code = out
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "被信号终止".to_string());
        log_line(log, &format!("（命令退出码：{code}）"));
    }
    if !out.success() && deploy_canceled(log) {
        return Err(DEPLOY_CANCELED.to_string());
    }
    Ok(out.success())
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
/// python 解释器：指定版本时用 `python3.11`，否则用系统默认 `python3`
fn python_bin(version: &str) -> String {
    if version.is_empty() {
        return "python3".to_string();
    }
    if version == "3" {
        return "python3".to_string();
    }
    format!("python{version}")
}

/// Python 项目要不要补装 WSGI/ASGI 服务器，装哪个。
///
/// 返回 `None` = 不需要（普通脚本，直接 `python3 app.py` 就行）。
/// ASGI（FastAPI）用 uvicorn，其余（Flask / Django / 裸 WSGI）用 gunicorn。
fn server_pkg(entry: &str, src: &str, req: &str) -> Option<&'static str> {
    let entry = entry.trim();
    let is_asgi = src.contains("FastAPI(")
        || req.to_lowercase().contains("fastapi")
        || entry.contains("uvicorn");
    let needs_server = entry.contains(':')
        || (entry.ends_with(".py") && (src.contains("Flask(") || src.contains("FastAPI(")));
    if !needs_server {
        return None;
    }
    Some(if is_asgi { "uvicorn" } else { "gunicorn" })
}

fn prepare_deps(
    task_log: &str,
    app_type: &str,
    version: &str,
    create_venv: bool,
    workdir: &Path,
    owner: &str,
    install: bool,
    entry: &str,
    pid_path: &str,
) -> Result<String, String> {
    let mut log = String::new();
    match app_type {
        "python" => {
            let venv = workdir.join(".venv");
            let need_venv = create_venv || install;
            let req = workdir.join("requirements.txt");
            let uv = uv_bin();

            // 明确要求建环境、或要装依赖时都要有 .venv。
            // 优先 uv：它能按指定版本现拉一个解释器，不依赖系统装没装 pythonX-venv。
            if need_venv && !venv.join("bin/python").exists() {
                // 指定版本时先确保共享目录里有该解释器（root 补装）：
                // 共享目录站点用户只读，缺版本时他们自己装不进去。
                if !version.is_empty() && uv.is_some() {
                    super::env::ensure_python_shared(version)?;
                }
                let (cmd, how) = match &uv {
                    Some(uv) => {
                        if version.is_empty() {
                            (format!("{uv} venv .venv"), "uv（默认版本）".to_string())
                        } else {
                            (
                                format!("{uv} venv --python {version} .venv"),
                                format!("uv --python {version}"),
                            )
                        }
                    }
                    None => {
                        let py = python_bin(version);
                        (format!("{py} -m venv .venv"), format!("{py} -m venv"))
                    }
                };
                if !run_as_stream(task_log, owner, workdir, &cmd, pid_path)? {
                    if deploy_canceled(task_log) {
                        return Err(DEPLOY_CANCELED.to_string());
                    }
                    return Err(format!("创建虚拟环境失败（{cmd}）"));
                }
                log.push_str(&format!("已创建 .venv（{how}）\n"));
            }
            if install && req.exists() {
                // 装依赖：uv pip > .venv/bin/pip > ensurepip 兜底。
                // 早先固定走 .venv/bin/pip，而很多发行版的 venv 里压根没 pip
                // （缺 pythonX-venv），于是直接 `.venv/bin/pip: No such file`。
                let cmd = match &uv {
                    Some(uv) => format!(
                        "VIRTUAL_ENV={} {uv} pip install -r requirements.txt",
                        venv.to_string_lossy()
                    ),
                    None if venv.join("bin/pip").exists() => {
                        ".venv/bin/pip install -r requirements.txt".to_string()
                    }
                    None => {
                        // venv 里没 pip：先补装，再走 pip
                        let (ok, out) =
                            run_as(owner, workdir, ".venv/bin/python -m ensurepip --upgrade")?;
                        if !ok {
                            return Err(format!(
                                "虚拟环境里没有 pip，且 ensurepip 失败：{out}\\n\
                             建议安装 uv（curl -LsSf https://astral.sh/uv/install.sh | sh），\\
                             或给系统补上 pythonX-venv"
                            ));
                        }
                        ".venv/bin/pip install -r requirements.txt".to_string()
                    }
                };
                log_line(task_log, &format!("开始安装依赖（{cmd}）…\n"));
                if !run_as_stream(task_log, owner, workdir, &cmd, pid_path)? {
                    if deploy_canceled(task_log) {
                        return Err(DEPLOY_CANCELED.to_string());
                    }
                    return Err(format!("依赖安装失败（{cmd}）"));
                }
                let summary = if uv.is_some() {
                    "依赖安装完成（uv pip）"
                } else {
                    "依赖安装完成（pip）"
                };
                log.push_str(&format!("{summary}\n"));
            }
            // 需要生产级服务器但 venv 里没有 → 自动补装，
            // 别让用户为了能部署去改 requirements.txt。
            if venv.join("bin/python").exists() {
                let src = if !entry.is_empty() && !entry.contains(':') {
                    std::fs::read_to_string(workdir.join(entry)).unwrap_or_default()
                } else {
                    String::new()
                };
                let req_txt = std::fs::read_to_string(&req).unwrap_or_default();
                if let Some(pkg) = server_pkg(entry, &src, &req_txt) {
                    if !venv.join(format!("bin/{pkg}")).exists() {
                        let cmd = match &uv {
                            Some(uv) => format!(
                                "VIRTUAL_ENV={} {uv} pip install {pkg}",
                                venv.to_string_lossy()
                            ),
                            _ if venv.join("bin/pip").exists() => {
                                format!(".venv/bin/pip install {pkg}")
                            }
                            _ => String::new(),
                        };
                        if !cmd.is_empty() {
                            match run_as(owner, workdir, &cmd) {
                                Ok((true, _)) => log.push_str(&format!("已自动安装 {pkg}\n")),
                                Ok((false, out)) => log.push_str(&format!(
                                    "自动安装 {pkg} 失败（可写进 requirements.txt 或自定义启动命令）：{out}\n"
                                )),
                                Err(e) => log.push_str(&format!("自动安装 {pkg} 出错：{e}\n")),
                            }
                        }
                    }
                }
            }
        }
        "nodejs" => {
            let pkg = workdir.join("package.json");
            if !install || !pkg.exists() {
                return Ok(log);
            }
            let npm = if workdir.join("package-lock.json").exists() {
                "npm ci"
            } else {
                "npm install"
            };
            // 指定了版本就让 PATH 先命中 fnm 里那一份，其它用户直接用全局软链
            let cmd = match super::env::node_bin_for(version) {
                Some(p) => match p.parent() {
                    Some(dir) => format!("PATH={}:$PATH {}", dir.to_string_lossy(), npm),
                    None => npm.to_string(),
                },
                None => npm.to_string(),
            };
            log_line(task_log, &format!("开始安装依赖（{cmd}）…\n"));
            if !run_as_stream(task_log, owner, workdir, &cmd, pid_path)? {
                if deploy_canceled(task_log) {
                    return Err(DEPLOY_CANCELED.to_string());
                }
                return Err(format!("{cmd} 失败"));
            }
            log.push_str(&format!("依赖安装完成（{cmd}）\n"));
        }
        "static" => {
            // 静态站点多为 node 构建（vite / webpack）：需要依赖时跑 npm install / ci
            if install {
                let pkg = workdir.join("package.json");
                if pkg.exists() {
                    let npm = if workdir.join("package-lock.json").exists()
                        || workdir.join("yarn.lock").exists()
                        || workdir.join("pnpm-lock.yaml").exists()
                    {
                        "npm ci"
                    } else {
                        "npm install"
                    };
                    log_line(task_log, &format!("开始安装依赖（{npm}）…\n"));
                    if !run_as_stream(task_log, owner, workdir, &npm, pid_path)? {
                        if deploy_canceled(task_log) {
                            return Err(DEPLOY_CANCELED.to_string());
                        }
                        return Err(format!("依赖安装失败（{npm}）"));
                    }
                    log.push_str(&format!("依赖安装完成（{npm}）\n"));
                }
            }
        }
        "generic" => {
            // 通用型：不编译、不准备依赖，用户自行提供启动命令（如 java -jar app.jar）。
            // 构建命令（如有）由部署主流程以系统默认 PATH 执行，这里无需额外处理。
            return Ok(String::new());
        }
        "go" => {
            // 编译型：依赖在 `go build` 时由 go.mod 自动拉取；勾选「安装依赖」仅做预取，
            // 失败不致命（构建仍会拉）。
            if install && workdir.join("go.mod").exists() {
                let cmd = with_toolchain_path("go", "go mod download");
                log_line(task_log, "预取 Go 依赖（go mod download）…\n");
                if !run_as_stream(task_log, owner, workdir, &cmd, pid_path)? {
                    if deploy_canceled(task_log) {
                        return Err(DEPLOY_CANCELED.to_string());
                    }
                    log.push_str("go mod download 失败（构建时会自动拉取，可忽略）\n");
                } else {
                    log.push_str("Go 依赖预取完成\n");
                }
            }
        }
        "rust" => {
            if install && workdir.join("Cargo.toml").exists() {
                let cmd = with_toolchain_path("rust", "cargo fetch");
                log_line(task_log, "预取 Rust 依赖（cargo fetch）…\n");
                if !run_as_stream(task_log, owner, workdir, &cmd, pid_path)? {
                    if deploy_canceled(task_log) {
                        return Err(DEPLOY_CANCELED.to_string());
                    }
                    log.push_str("cargo fetch 失败（构建时会自动拉取，可忽略）\n");
                } else {
                    log.push_str("Rust 依赖预取完成\n");
                }
            }
        }
        _ => {}
    }
    // npm 11 起默认跳过未批准的 install 脚本（install-scripts 白名单机制），
    // 这类告警不影响安装成功（@parcel/watcher / esbuild 等通过 optionalDependencies
    // 自带预编译二进制）。检测到时在日志里给出为什么跳过、以及如何消除告警。
    if let Ok(content) = std::fs::read_to_string(task_log) {
        if content.contains("npm warn install-scripts") {
            log.push_str(&build_install_scripts_hint(&content));
        }
    }
    Ok(log)
}

/// 从任务日志里找出被 npm 11 跳过 install 脚本的包（形如 `@parcel/watcher@2.6.0`），
/// 生成一段中文提示 + 可直接复制进 package.json 的 `allowScripts` 片段。
fn build_install_scripts_hint(content: &str) -> String {
    let mut pkgs: Vec<String> = Vec::new();
    for line in content.lines() {
        if let Some(rest) = line.split_once("npm warn install-scripts") {
            // 形如：`  @parcel/watcher@2.6.0 (install: node scripts/build-from-source.js)`
            if let Some(tok) = rest.1.trim().split_whitespace().next() {
                if tok.contains('@') && !tok.starts_with('(') {
                    pkgs.push(tok.to_string());
                }
            }
        }
    }
    if pkgs.is_empty() {
        return String::new();
    }
    let mut msg = String::from(
        "提示：npm 11 默认跳过未批准 install 脚本（install-scripts 白名单），本次跳过了：",
    );
    msg.push_str(&pkgs.join("、"));
    msg.push_str(
        "。这些包通常通过 optionalDependencies 自带预编译二进制，一般不影响运行。\n\
         若想消除该告警，可在 package.json 增加：\n  \"allowScripts\": { ",
    );
    let entries: Vec<String> = pkgs.iter().map(|p| format!("\"{p}\": true")).collect();
    msg.push_str(&entries.join(", "));
    msg.push_str(
        " }\n或运行：npm install-scripts approve <包名>\n",
    );
    msg
}

/// 推导默认启动命令（用户填了 `command` 就不走这里）
fn default_command(
    app_type: &str,
    workdir: &Path,
    entry: &str,
    port: i64,
    version: &str,
) -> Result<String, String> {
    // node 优先用 fnm 装的对应版本（绝对路径），免得 unit 跑成系统默认那一个
    let node = super::env::node_bin_for(version)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "node".to_string());
    match app_type {
        "python" => {
            let venv_py = workdir.join(".venv/bin/python");
            let py = if venv_py.exists() {
                workdir
                    .join(".venv/bin/python")
                    .to_string_lossy()
                    .to_string()
            } else {
                "python3".to_string()
            };
            // 未填入口：按常见文件名探测，别生成一条 `python3 ` 空命令
            let entry = if entry.is_empty() {
                [
                    "app.py",
                    "main.py",
                    "wsgi.py",
                    "manage.py",
                    "run.py",
                    "server.py",
                ]
                .iter()
                .find(|f| workdir.join(f).exists())
                .map(|f| f.to_string())
                .ok_or_else(|| {
                    "未找到入口：请填写入口（如 app.py、wsgi:app），或在「启动命令」里自定义"
                        .to_string()
                })?
            } else {
                entry.to_string()
            };
            // 是 .py 文件时，尽量走生产级服务器：Flask / FastAPI + gunicorn / uvicorn
            if entry.ends_with(".py") {
                let module = entry.trim_end_matches(".py").to_string();
                let src = std::fs::read_to_string(workdir.join(&entry)).unwrap_or_default();
                let gunicorn = workdir.join(".venv/bin/gunicorn");
                let uvicorn = workdir.join(".venv/bin/uvicorn");
                if (src.contains("Flask(") || src.contains("FastAPI(")) && gunicorn.exists() {
                    return Ok(format!(
                        "{} -b 127.0.0.1:{} {}:app",
                        gunicorn.to_string_lossy(),
                        port,
                        module
                    ));
                }
                if src.contains("FastAPI(") && uvicorn.exists() {
                    return Ok(format!(
                        "{} --host 127.0.0.1 --port {} {}:app",
                        uvicorn.to_string_lossy(),
                        port,
                        module
                    ));
                }
            }
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
                return Err("入口是 `模块:应用` 形态，需要 gunicorn 或 uvicorn：\\
                     部署时已尝试自动安装但没装上（见部署日志），\\
                     请把它写进 requirements.txt，或在「启动命令」里自定义"
                    .to_string());
            }
            Ok(format!("{py} {entry}"))
        }
        "nodejs" => {
            if !entry.is_empty() {
                return Ok(format!("{node} {entry}"));
            }
            // 未填入口：优先 package.json 的 start 脚本，其次常见入口文件
            if let Ok(txt) = std::fs::read_to_string(workdir.join("package.json")) {
                if txt.contains("\"start\"") {
                    return Ok("npm start".to_string());
                }
            }
            for f in ["server.js", "app.js", "index.js", "main.js"] {
                if workdir.join(f).exists() {
                    return Ok(format!("{node} {f}"));
                }
            }
            Err("未找到入口：请填写入口文件（如 server.js）或在「启动命令」里自定义".to_string())
        }
        "go" => {
            // 编译产物是 workdir 下的原生二进制：entry 填「相对工作目录的可执行路径」。
            // 运行时不需要 go 工具链（systemd 以 WorkingDirectory 解析相对路径）。
            let bin = entry.trim();
            if bin.is_empty() {
                return Err(
                    "未找到入口：请填写编译产物路径（如 bin/应用名），或在「启动命令」里自定义".to_string(),
                );
            }
            Ok(format!("./{}", bin.trim_start_matches('/').trim_start_matches('.')))
        }
        "rust" => {
            let bin = entry.trim();
            if bin.is_empty() {
                return Err(
                    "未找到入口：请填写编译产物路径（如 target/release/应用名），或在「启动命令」里自定义"
                        .to_string(),
                );
            }
            Ok(format!("./{}", bin.trim_start_matches('/').trim_start_matches('.')))
        }
        "generic" => Err(
            "通用部署必须填写启动命令（如 java -jar app.jar）".to_string(),
        ),
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
    u.push_str(&format!("Description=Zap App {name} (site {site_id})\n"));
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

/// 分支 / 提交 / 子目录：只允许字母数字与 `/ - _ .`，杜绝 git 参数注入
fn git_token_ok(s: &str) -> bool {
    s.is_empty() || s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'))
}

/// 仓库地址：限 http(s)/git 协议，不得含空白或 `--`（防参数注入）
fn valid_repo(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.chars().any(|c| c.is_whitespace()) || t.contains("--") {
        return false;
    }
    t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with("git@")
        || t.starts_with("git://")
}

/// 把仓库拉取到 `dest`：若 `dest/.git` 已存在则 pull，否则 clone。
/// 各 git 步骤的输出**实时追加**到 `log`（部署任务日志），方便前端边跑边看；
/// 返回最终工作目录（在 `dest` 基础上叠加 `git_subdir`）与当前 HEAD 短哈希。
fn git_clone(
    log: &str,
    repo_url: &str,
    branch: &str,
    git_ref: &str,
    git_subdir: &str,
    git_depth: i64,
    dest: &Path,
    owner: &str,
    pid_path: &str,
) -> Result<(PathBuf, String), String> {
    log_line(log, &format!("== 同步仓库 {repo_url} =="));
    // 目标目录由调用方以站点用户身份建好（保证属主正确、clone 可写）
    let ok = if dest.join(".git").exists() {
        // 已克隆：fetch 后用 reset --hard 对齐到远程分支，不依赖本地分支的 tracking 配置
        // （否则 git pull 会因 "no tracking information" 失败，例如首次部署超时打断后残留的仓库）。
        let _ = run_as_stream(log, owner, dest, "git fetch --progress --all --tags", pid_path)?;
        let target = if !branch.is_empty() {
            format!("origin/{branch}")
        } else {
            // 未指定分支：对齐到 FETCH_HEAD（即远程默认分支）
            "FETCH_HEAD".to_string()
        };
        run_as_stream(log, owner, dest, &format!("git reset --hard {target}"), pid_path)?
    } else {
        let mut cmd = String::from("git clone --progress");
        if git_depth > 0 {
            cmd.push_str(&format!(" --depth {git_depth}"));
        }
        if !branch.is_empty() {
            cmd.push_str(&format!(" --branch {branch}"));
        }
        cmd.push_str(&format!(" {repo_url} ."));
        run_as_stream(log, owner, dest, &cmd, pid_path)?
    };
    if !ok {
        if deploy_canceled(log) {
            return Err(DEPLOY_CANCELED.to_string());
        }
        return Err("git 拉取失败（详见部署日志）".to_string());
    }
    if !git_ref.is_empty() {
        if !run_as_stream(log, owner, dest, &format!("git checkout {git_ref}"), pid_path)? {
            if deploy_canceled(log) {
                return Err(DEPLOY_CANCELED.to_string());
            }
            return Err(format!("git checkout 失败（{git_ref}）"));
        }
    }
    let (_, head) = run_as(owner, dest, "git rev-parse --short HEAD")?;
    let commit = head.lines().next().unwrap_or("").trim().to_string();
    let wd = if git_subdir.is_empty() {
        dest.to_path_buf()
    } else {
        dest.join(git_subdir)
    };
    if !wd.exists() {
        return Err(format!("仓库子目录不存在：{git_subdir}"));
    }
    Ok((wd, commit))
}

/// 静态型构建产物目录：显式指定优先，否则按常见约定自动探测
fn resolve_static_output(wd: &Path, build_output: &str) -> PathBuf {
    if !build_output.is_empty() {
        return wd.join(build_output);
    }
    for cand in [
        "dist",
        "build",
        "public",
        "_site",
        "out",
        ".output/public",
        ".next",
        ".vuepress/dist",
        "docs/.vuepress/dist",
    ] {
        let p = wd.join(cand);
        if p.is_dir() {
            return p;
        }
    }
    wd.to_path_buf()
}

/// 部署（幂等）：准备依赖 -> 渲染 unit -> daemon-reload -> 重启
#[allow(clippy::too_many_arguments)]
pub async fn deploy(
    site_id: i64,
    name: &str,
    app_type: &str,
    runtime_version: &str,
    build_cmd: &str,
    create_venv: bool,
    workdir: &str,
    entry: &str,
    command: &str,
    port: i64,
    env: &str,
    autostart: bool,
    install_deps: bool,
    owner_user: &str,
    log_dir: &str,
    task_log: &str,
    requester: Option<String>,
    skip_owner_check: bool,
    repo_url: &str,
    branch: &str,
    git_ref: &str,
    git_subdir: &str,
    git_depth: i64,
    build_output: &str,
) -> Response {
    let (
        name,
        app_type,
        runtime_version,
        build_cmd,
        workdir,
        entry,
        command,
        env,
        owner_user,
        log_dir,
        task_log,
        requester,
        skip_owner_check,
        repo_url,
        branch,
        git_ref,
        git_subdir,
        build_output,
    ) = (
        name.to_string(),
        app_type.to_string(),
        runtime_version.to_string(),
        build_cmd.to_string(),
        workdir.to_string(),
        entry.to_string(),
        command.to_string(),
        env.to_string(),
        owner_user.to_string(),
        log_dir.to_string(),
        task_log.to_string(),
        requester.clone(),
        skip_owner_check,
        repo_url.to_string(),
        branch.to_string(),
        git_ref.to_string(),
        git_subdir.to_string(),
        build_output.to_string(),
    );
    blocking(move || {
        if !valid_name(&name) {
            return Err("应用名只能包含字母、数字、- 和 _".to_string());
        }
        if !valid_user(&owner_user) {
            return Err("站点用户不合法（且不能是 root）".to_string());
        }
        // 越权防护（defense-in-depth）：普通用户部署的应用，其运行账号必须等于请求方本人，
        // 否则可借 `owner_user` 把应用以他人站点用户身份拉起（跨站点篡改 / 提权）。
        // 管理员（skip_owner_check）不受此约束，但 zapd 侧仍会校验站点归属。
        ensure_owner_matches(&owner_user, &requester, skip_owner_check)?;
        if !app_type_supported(&app_type) {
            return Err(format!("不支持的应用类型：{app_type}"));
        }
        if !runtime_version.is_empty() && !valid_version(&runtime_version) {
            return Err(format!("运行时版本号不合法：{runtime_version}"));
        }
        for (f, v) in [
            ("工作目录", workdir.as_str()),
            ("入口", entry.as_str()),
            ("启动命令", command.as_str()),
            ("构建命令", build_cmd.as_str()),
            ("环境变量", env.as_str()),
        ] {
            if !no_newline(v) {
                return Err(format!("{f} 不能包含换行"));
            }
        }
        prepare_task_log(&task_log);
        log_line(&task_log, "开始部署…");
        // 当前步骤进程组 leader 的 pid 写到这里，供 AppDeployStop 整组终止；
        // 取消哨兵 <task_log>.cancel 由 zapd 的 stop 请求写入。
        let pid_path = swap_ext(&task_log, "pid");
        // 源代码：有仓库地址时先 clone / pull（以站点用户身份），否则沿用现有 workdir
        let (wd, git_commit) = if !repo_url.is_empty() {
            if !git_token_ok(&branch) || !git_token_ok(&git_ref) || !git_token_ok(&git_subdir) {
                return Err("分支 / 提交 / 子目录含非法字符".to_string());
            }
            if !valid_repo(&repo_url) {
                return Err(format!("非法的仓库地址：{repo_url}"));
            }
            let dest = PathBuf::from(&workdir);
            // 越界防护：clone 目标必须落在站点用户家目录之内
            let home = format!("/home/{owner_user}");
            if !dest.starts_with(&home) {
                return Err(format!("代码目录必须在 {home} 之内"));
            }
            // 目标目录以站点用户身份创建，确保后续 clone 可写（绝不 root 建目录）
            if let Some(parent) = dest.parent() {
                let _ = run_as(&owner_user, parent, &format!("mkdir -p {}", dest.display()))?;
            }
            let (wd, git_commit) = git_clone(
                &task_log,
                &repo_url,
                &branch,
                &git_ref,
                &git_subdir,
                git_depth,
                &dest,
                &owner_user,
                &pid_path,
            )?;
            (wd, git_commit)
        } else {
            (check_workdir(&workdir, &owner_user)?, String::new())
        };

        if deploy_canceled(&task_log) {
            return Ok(Response::err(-130, DEPLOY_CANCELED));
        }
        log_line(&task_log, &prepare_deps(
            &task_log,
            &app_type,
            &runtime_version,
            create_venv,
            &wd,
            &owner_user,
            install_deps,
            &entry,
            &pid_path,
        )?);

        if deploy_canceled(&task_log) {
            return Ok(Response::err(-130, DEPLOY_CANCELED));
        }
        // 构建命令：在装完依赖之后、拉起进程之前跑（npm run build / 迁移脚本之类）
        let bc = build_cmd.trim();
        if !bc.is_empty() {
            // 编译型（go/rust）：构建阶段才需要工具链，预先把工具链目录塞进 PATH，
            // 免得站点用户 `su` 环境里找不到 go / cargo（运行时是原生二进制，无需工具链）。
            let bc = if app_type == "go" || app_type == "rust" {
                with_toolchain_path(&app_type, bc)
            } else {
                bc.to_string()
            };
            log_line(&task_log, &format!("== 构建：{bc} =="));
            if !run_as_stream(&task_log, &owner_user, &wd, &bc, &pid_path)? {
                if deploy_canceled(&task_log) {
                    return Ok(Response::err(-130, DEPLOY_CANCELED));
                }
                return Err("构建命令失败（详见部署日志）".to_string());
            }
            log_line(&task_log, "构建命令执行完成");
        }

        // 静态型：构建出静态文件后直接返回产物目录（由面板改写站点 web_root），
        // 不托管 systemd 进程
        if app_type == "static" {
            let output = resolve_static_output(&wd, &build_output);
            if !output.is_dir() {
                return Err(format!(
                    "未找到静态构建产物目录（已尝试 dist/build/public/_site/out 等，\
                     或请在表单指定 build_output）：{}",
                    output.display()
                ));
            }
            log_line(&task_log, "已构建静态产物");
            return Ok(Response::ok(
                "部署完成（静态站点）",
                Some(json!({
                    "output_dir": output.to_string_lossy(),
                    "git_commit": git_commit,
                })),
            ));
        }

        let exec = if !command.trim().is_empty() {
            command.trim().to_string()
        } else {
            default_command(&app_type, &wd, &entry, port, &runtime_version)?
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
        log_line(&task_log, &format!("== 启动 ==\n{}\n已部署并启动", clip(&out)));
        Ok(Response::ok(
            "部署完成",
            Some(json!({
                "unit": unit,
                "git_commit": git_commit,
            })),
        ))
    })
    .await
}

/// 终止运行中的部署任务：读取 `<log>.pid` 里的进程组 leader pid，向整个进程组发
/// SIGTERM，宽限 5 秒后 SIGKILL；并写 `<log>.cancel` 哨兵供执行侧各步骤间识别“已取消”。
pub async fn app_deploy_stop(log_path: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let pid_path = swap_ext(&log_path, "pid");
        let cancel_path = swap_ext(&log_path, "cancel");
        let pid: i32 = std::fs::read_to_string(&pid_path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| "部署进程不存在或已结束".to_string())?;
        // 先写哨兵，确保执行侧即使本步已结束、下步即将开始时也能感知取消
        let _ = std::fs::write(&cancel_path, "");
        let mut alive = unsafe { libc::kill(-pid, libc::SIGTERM) } == 0;
        for _ in 0..5 {
            if !alive {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            alive = unsafe { libc::kill(-pid, 0) } == 0;
        }
        if alive {
            unsafe { libc::kill(-pid, libc::SIGKILL); }
        }
        Ok::<_, String>(Response::ok("已发送停止信号", None))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
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
            "start" | "stop" | "restart" | "enable" | "disable" | "mask" | "unmask" => {
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
    fn non_admin_cannot_deploy_as_other_user() {
        // 普通用户必须以本人站点账号部署
        assert!(ensure_owner_matches("alice", &Some("alice".into()), false).is_ok());
        // owner_user 与请求用户不一致 → 拒绝（跨站点篡改 / 提权）
        assert!(ensure_owner_matches("alice", &Some("bob".into()), false).is_err());
        // 请求方缺失系统账号也拒绝
        assert!(ensure_owner_matches("alice", &None, false).is_err());
        // 管理员不受此约束
        assert!(ensure_owner_matches("alice", &Some("bob".into()), true).is_ok());
        assert!(ensure_owner_matches("alice", &None, true).is_ok());
    }

    #[test]
    fn python_module_app_uses_gunicorn_when_present() {
        let d = tmp("py");
        std::fs::create_dir_all(d.join(".venv/bin")).unwrap();
        std::fs::write(d.join(".venv/bin/gunicorn"), "").unwrap();
        let cmd = default_command("python", &d, "wsgi:app", 8000, "").unwrap();
        assert!(cmd.contains("gunicorn"));
        assert!(cmd.contains("-b 127.0.0.1:8000"));
        assert!(cmd.ends_with("wsgi:app"));
    }

    #[test]
    fn python_script_uses_venv_python() {
        let d = tmp("py2");
        std::fs::create_dir_all(d.join(".venv/bin")).unwrap();
        std::fs::write(d.join(".venv/bin/python"), "").unwrap();
        let cmd = default_command("python", &d, "main.py", 0, "").unwrap();
        assert!(cmd.contains(".venv/bin/python"));
        assert!(cmd.ends_with("main.py"));
    }

    #[test]
    fn nodejs_falls_back_to_npm_start() {
        let d = tmp("node");
        std::fs::write(
            d.join("package.json"),
            r#"{"scripts":{"start":"node s.js"}}"#,
        )
        .unwrap();
        assert_eq!(
            default_command("nodejs", &d, "", 3000, "").unwrap(),
            "npm start"
        );
        std::fs::write(d.join("package.json"), r#"{"name":"x"}"#).unwrap();
        assert!(default_command("nodejs", &d, "", 3000, "").is_err());
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

#[cfg(test)]
mod runtime_tests {
    use super::*;

    #[test]
    fn python_scan_only_picks_python3() {
        let vs = scan_python_versions("/usr/bin");
        for v in &vs {
            assert!(v == "3" || v.starts_with("3."), "意外的版本号: {v}");
        }
        // 服务器上必然装了 python3，否则 python 应用类型无从跑起
        assert!(
            vs.iter().any(|v| v.starts_with('3')),
            "未探测到 python3: {vs:?}"
        );
    }

    #[test]
    fn version_format_guard() {
        assert!(valid_version("3.11"));
        assert!(valid_version("20"));
        // 会被拼进 `python{version} -m venv`，这些必须挡住
        assert!(!valid_version("3.11; rm -rf /"));
        assert!(!valid_version("$(id)"));
        assert!(!valid_version(""));
    }

    #[test]
    fn python_bin_maps_version() {
        assert_eq!(python_bin(""), "python3");
        assert_eq!(python_bin("3"), "python3");
        assert_eq!(python_bin("3.11"), "python3.11");
    }
}

#[cfg(test)]
mod detect_tests {
    use super::*;

    #[test]
    fn python_scan_only_picks_python3() {
        let vs = scan_python_versions("/usr/bin");
        for v in &vs {
            assert!(v == "3" || v.starts_with("3."), "unexpected version: {v}");
        }
        assert!(
            vs.iter().any(|v| v.starts_with('3')),
            "no python3 found: {vs:?}"
        );
    }

    #[test]
    fn version_format_guard() {
        assert!(valid_version("3.11"));
        assert!(valid_version("20"));
        assert!(!valid_version("3.11; rm -rf /"));
        assert!(!valid_version("$(id)"));
        assert!(!valid_version(""));
    }

    #[test]
    fn python_bin_maps_version() {
        assert_eq!(python_bin(""), "python3");
        assert_eq!(python_bin("3"), "python3");
        assert_eq!(python_bin("3.11"), "python3.11");
    }
}

#[cfg(test)]
mod entry_tests {
    use super::*;

    #[test]
    fn python_entry_is_detected_instead_of_empty_command() {
        let d = std::env::temp_dir().join("zap-app-entry-test");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("app.py"),
            "from flask import Flask\napp = Flask(__name__)\n",
        )
        .unwrap();

        // 没装 gunicorn：退回 `python3 app.py`，绝不能生成空的 `python3 `
        let cmd = default_command("python", &d, "", 8080, "").unwrap();
        assert!(cmd.ends_with("app.py"), "{cmd}");
        assert!(!cmd.trim().ends_with("python3"), "{cmd}");

        // 装了 gunicorn：Flask 项目自动走 gunicorn，端口来自 PORT
        std::fs::create_dir_all(d.join(".venv/bin")).unwrap();
        std::fs::write(d.join(".venv/bin/gunicorn"), "").unwrap();
        let cmd = default_command("python", &d, "", 8080, "").unwrap();
        assert!(cmd.contains("gunicorn"), "{cmd}");
        assert!(cmd.contains("127.0.0.1:8080"), "{cmd}");
        assert!(cmd.ends_with("app:app"), "{cmd}");

        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn python_without_entry_file_errors_clearly() {
        let d = std::env::temp_dir().join("zap-app-entry-test-empty");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let err = default_command("python", &d, "", 8080, "").unwrap_err();
        assert!(err.contains("未找到入口"), "{err}");
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod server_pkg_tests {
    use super::server_pkg;

    #[test]
    fn plain_script_needs_no_server() {
        assert_eq!(server_pkg("app.py", "print('hi')", ""), None);
    }

    #[test]
    fn flask_gets_gunicorn() {
        assert_eq!(
            server_pkg(
                "app.py",
                "from flask import Flask\napp = Flask(__name__)",
                "flask"
            ),
            Some("gunicorn")
        );
    }

    #[test]
    fn fastapi_gets_uvicorn() {
        assert_eq!(
            server_pkg(
                "main.py",
                "from fastapi import FastAPI\napp = FastAPI()",
                ""
            ),
            Some("uvicorn")
        );
        // 源码看不到时，requirements 里的 fastapi 也算数
        assert_eq!(
            server_pkg("wsgi:app", "", "fastapi\nuvicorn"),
            Some("uvicorn")
        );
    }

    /// `模块:应用` 形态裸跑不了，必须给服务器
    #[test]
    fn module_app_form_needs_server() {
        assert_eq!(server_pkg("wsgi:app", "", ""), Some("gunicorn"));
    }
}
