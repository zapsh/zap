//! 插件运行时（mlua 嵌入 zapexec）。
//!
//! 插件是放在 `$ZAP_PATH/plugins/<name>/`（系统级）或 `$HOME/plugins/<name>/`（用户级）
//! 的目录，含 `manifest.yaml` + `main.lua`。`main.lua` 定义 `on_run(ctx)`，通过全局表 `zap`
//! 调用受限能力：
//!   - `zap.log(msg)`                   打印日志（回传前端）
//!   - `zap.option(name)`               读取运行选项
//!   - `zap.exec(prog, {args})`         以 root 执行（仅 scope=system）
//!   - `zap.exec_as_user(prog, {args})` 以站点 Linux 账号执行（仅 scope=site）
//!   - `zap.site_root()` / `zap.site_linux_user()`  当前站点上下文（scope=site）
//!   - `zap.home_dir()`               当前执行身份的家目录（scope=system 为调用方 home，scope=site 为站点 Linux 账号 home）
//!
//! 安全边界：
//!   - 插件只能声明结构化 UI（manifest），不能注入前端代码；
//!   - 执行身份由 scope 决定，scope=site 时通过 user_cmd + drop_privileges 降到站点账号
//!     （清附加组 → setgid → setuid），scope=system 才以 root 执行；
//!   - 插件名只允许 `[A-Za-z0-9_-]`，目录越界即拒绝。

use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tracing::{info, warn};
use zap_proto::Response;

fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

fn is_plugin_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !s.contains("..")
}

fn read_manifest(dir: &Path) -> Result<serde_yaml::Value, String> {
    for name in ["manifest.yaml", "manifest.yml"] {
        let p = dir.join(name);
        if p.is_file() {
            let txt = std::fs::read_to_string(&p).map_err(|e| format!("{e}"))?;
            return serde_yaml::from_str(&txt).map_err(|e| format!("manifest 解析失败: {e}"));
        }
    }
    Err("未找到 manifest.yaml".into())
}

/// 列出插件（系统级 + 当前用户级），按 placement 槽位 / scope 过滤。
pub async fn plugin_list(
    _actor: String,
    home: String,
    slot: Option<String>,
    scope: Option<String>,
) -> Response {
    let zap = zap_path();
    // 用户级插件目录固定为 <home>/.zap/plugins；系统级为 <zap>/plugins
    let bases = vec![
        zap.join("plugins"),
        Path::new(&home).join(".zap").join("plugins"),
    ];
    info!(
        "plugin_list: home={home:?} slot={slot:?} scope={scope:?} zap={}",
        zap.display()
    );
    for base in &bases {
        info!("  base={} exists={}", base.display(), base.exists());
    }
    let mut items = Vec::new();
    for base in &bases {
        let Ok(rd) = std::fs::read_dir(base) else {
            warn!("  read_dir 失败（跳过）: {}", base.display());
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let name = match p.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !is_plugin_name(&name) {
                warn!("  跳过非法插件名: {name}");
                continue;
            }
            let Ok(m) = read_manifest(&p) else {
                warn!("  插件 {name} 读取 manifest 失败（跳过）: {}", p.display());
                if let Err(e) = read_manifest(&p) {
                    warn!("    manifest 错误: {e}");
                }
                continue;
            };
            let pl_scope = m.get("scope").and_then(|v| v.as_str()).unwrap_or("system");
            let placement = m
                .get("ui")
                .and_then(|u| u.get("placement"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            info!(
                "  插件 {name}: scope={pl_scope} placement={placement:?} slot={slot:?} scope_filter={scope:?}"
            );
            if let Some(sc) = &scope {
                if pl_scope != sc.as_str() {
                    warn!("    -> 被 scope 过滤丢弃");
                    continue;
                }
            }
            if let Some(sl) = &slot {
                if &placement != sl {
                    warn!("    -> 被 slot 过滤丢弃 (want {sl:?})");
                    continue;
                }
            }
            info!("    -> 命中，加入列表");
            let options = serde_json::to_value(
                m.get("options").cloned().unwrap_or(serde_yaml::Value::Null),
            )
            .unwrap_or(Value::Null);
            let actions = serde_json::to_value(
                m.get("actions").cloned().unwrap_or(serde_yaml::Value::Null),
            )
            .unwrap_or(Value::Null);
            items.push(json!({
                "name": name,
                "title": m.get("title").and_then(|v| v.as_str()).unwrap_or(&name),
                "async": m.get("async").and_then(|v| v.as_bool()).unwrap_or(false),
                "scope": m.get("scope").and_then(|v| v.as_str()).unwrap_or("system"),
                "placement": placement,
                "label": m.get("ui").and_then(|u| u.get("label")).and_then(|v| v.as_str())
                    .unwrap_or(&name),
                "icon": m.get("ui").and_then(|u| u.get("icon")).and_then(|v| v.as_str()).unwrap_or(""),
                "tab": m.get("ui").and_then(|u| u.get("tab")).and_then(|v| v.as_str()).unwrap_or(""),
                "options": options,
                "actions": actions,
            }));
        }
    }
    info!("plugin_list 完成: 返回 {} 个插件", items.len());
    Response::ok("ok", Some(json!(items)))
}

/// 运行插件。
pub async fn plugin_run(
    name: String,
    _actor: String,
    home: String,
    _site_id: Option<i64>,
    site_root: Option<String>,
    site_linux_user: Option<String>,
    action: String,
    options: HashMap<String, String>,
) -> Response {
    if !is_plugin_name(&name) {
        return Response::err(-1, "非法插件名");
    }
    let zap = zap_path();
    let home_path = Path::new(&home);
    let candidates = [
        home_path.join(".zap").join("plugins").join(&name),
        zap.join("plugins").join(&name),
    ];
    let dir = candidates
        .into_iter()
        .find(|p| p.is_dir())
        .unwrap_or_else(|| home_path.join(".zap").join("plugins").join(&name));
    if !dir.is_dir() {
        return Response::err(-1, format!("插件不存在: {name}"));
    }
    let manifest = match read_manifest(&dir) {
        Ok(m) => m,
        Err(e) => return Response::err(-1, e),
    };
    let scope = manifest
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("system")
        .to_string();
    let (run_user, run_root) = if scope == "site" {
        match (site_root.clone(), site_linux_user.clone()) {
            (Some(r), Some(u)) => (Some(u), Some(r)),
            _ => return Response::err(-1, "site 作用域插件需要 site_root / site_linux_user"),
        }
    } else {
        (None, None)
    };
    let lua_file = dir.join("main.lua");
    let code = match std::fs::read_to_string(&lua_file) {
        Ok(c) => c,
        Err(e) => return Response::err(-1, format!("读取插件脚本失败: {e}")),
    };

    // 异步模式：后台执行，日志实时落盘，立即返回 task_id + log_path，由前端 SSE 订阅
    let is_async = manifest
        .get("async")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if is_async {
        let task_id = format!("{name}-{}", chrono::Utc::now().timestamp_millis());
        let log_dir = zap.join("data/plugins/logs");
        if let Some(p) = log_dir.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = std::fs::create_dir_all(&log_dir);
        let log_path = log_dir.join(format!("{task_id}.log"));
        // 取消哨兵文件：前端经 zapd 写 <log>.cancel，看门狗线程轮询到就杀掉子进程
        let cancel_path = log_path.with_extension("log.cancel");
        let lp = log_path.clone();
        // 取消标志 + 当前子进程 pid（exec_as_user 走 setsid，pid 即进程组号，可整组杀）
        let cancel = Arc::new(AtomicBool::new(false));
        let child_pid: Arc<std::sync::Mutex<Option<(u32, bool)>>> =
            Arc::new(std::sync::Mutex::new(None));
        let cancel_flag = cancel.clone();
        let child_pid_w = child_pid.clone();
        let cancel_path_w = cancel_path.clone();
        std::thread::spawn(move || loop {
            if cancel_path_w.exists() || cancel_flag.load(Ordering::SeqCst) {
                cancel_flag.store(true, Ordering::SeqCst);
                if let Some((pid, session_leader)) = child_pid_w.lock().unwrap().take() {
                    unsafe {
                        // session_leader（setsid 过）= 杀整个进程组；否则只杀单进程，避免误伤 zapexec
                        if session_leader {
                            libc::kill(-(pid as i32), libc::SIGKILL);
                        } else {
                            libc::kill(pid as i32, libc::SIGKILL);
                        }
                    }
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        });
        tokio::task::spawn_blocking(move || {
            let res = run_lua(
                &code,
                &scope,
                run_user.as_deref(),
                run_root.as_deref(),
                &home,
                &options,
                &action,
                Some(lp.clone()),
                cancel.clone(),
                child_pid.clone(),
            );
            let cancelled = cancel.load(Ordering::SeqCst);
            // 清掉取消哨兵文件（若存在）
            let _ = std::fs::remove_file(&cancel_path);
            match res {
                Ok(_) => super::finish_log(lp.to_str().unwrap_or(""), 0),
                Err(e) => {
                    if cancelled {
                        super::log_line(lp.to_str().unwrap_or(""), "任务已取消");
                        super::finish_log(lp.to_str().unwrap_or(""), -2);
                    } else {
                        super::log_line(lp.to_str().unwrap_or(""), &format!("插件执行失败: {e}"));
                        super::finish_log(lp.to_str().unwrap_or(""), -1);
                    }
                }
            }
        });
        return Response::ok(
            "ok",
            Some(json!({
                "task_id": task_id,
                "log_path": log_path.display().to_string(),
                "async": true,
            })),
        );
    }

    let out = tokio::task::spawn_blocking(move || -> Result<String, String> {
        run_lua(
            &code,
            &scope,
            run_user.as_deref(),
            run_root.as_deref(),
            &home,
            &options,
            &action,
            None,
            Arc::new(AtomicBool::new(false)),
            Arc::new(std::sync::Mutex::new(None)),
        )
    })
    .await
    .unwrap_or_else(|e| Err(format!("插件执行线程崩溃: {e}")));

    match out {
        Ok(log) => Response::ok("插件执行完成", Some(json!({ "log": log }))),
        Err(e) => Response::err(-1, e),
    }
}

/// 在 mlua 沙箱里执行插件主体并调用 on_run。
fn run_lua(
    code: &str,
    scope: &str,
    run_user: Option<&str>,
    run_root: Option<&str>,
    home: &str,
    options: &HashMap<String, String>,
    action: &str,
    log_file: Option<std::path::PathBuf>,
    cancel: Arc<AtomicBool>,
    child_pid: Arc<std::sync::Mutex<Option<(u32, bool)>>>,
) -> Result<String, String> {
    let lua = mlua::Lua::new();
    let log = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    // 异步模式：日志同时落盘（zapexec 既有的长任务日志协议）；exec 输出也实时 tee 到同一文件，
    // 这样前端 SSE 能边跑边看 composer 等命令的进度，而不是等到结束才一次性灌出来。
    let logf: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>> = log_file.and_then(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
            .map(std::sync::Mutex::new)
            .map(std::sync::Arc::new)
    });
    let zap_tbl = lua
        .create_table()
        .map_err(|e| format!("创建 zap 表失败: {e}"))?;

    // zap.log
    {
        let log = log.clone();
        let logf = logf.clone();
        let f = lua.create_function(move |_, msg: String| {
            log.borrow_mut().push_str(&msg);
            log.borrow_mut().push('\n');
            if let Some(f) = logf.as_ref() {
                use std::io::Write;
                let _ = f.lock().unwrap().write_all(msg.as_bytes());
                let _ = f.lock().unwrap().write_all(b"\n");
            }
            Ok(())
        });
        zap_tbl
            .set("log", f.map_err(|e| format!("log 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.option
    {
        let options = options.clone();
        let f = lua.create_function(move |_, key: String| {
            Ok(options.get(&key).cloned().unwrap_or_default())
        });
        zap_tbl
            .set("option", f.map_err(|e| format!("option 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.exec（仅 system）/ zap.exec_as_user（仅 site）
    {
        let scope = scope.to_string();
        let run_user = run_user.map(|s| s.to_string());
        let logf_exec = logf.clone();
        // 每个闭包各持一份 cancel / child_pid 的 clone（Arc 不 Copy，不能共享同一个绑定）
        let cancel_exec = cancel.clone();
        let cancel_user = cancel.clone();
        let child_pid_exec = child_pid.clone();
        let child_pid_user = child_pid.clone();
        let f_exec = lua.create_function(move |_, (prog, args): (String, mlua::Table)| {
            if scope == "site" {
                return Err(mlua::Error::RuntimeError(
                    "site 作用域禁止 zap.exec，请改用 zap.exec_as_user".into(),
                ));
            }
            run_capture(
                None,
                &prog,
                &table_to_vec(&args),
                logf_exec.clone(),
                cancel_exec.clone(),
                child_pid_exec.clone(),
            )
            .map_err(mlua::Error::RuntimeError)
        });
        zap_tbl
            .set("exec", f_exec.map_err(|e| format!("exec 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
        let run_user2 = run_user.clone();
        let logf_user = logf.clone();
        let f_user = lua.create_function(move |_, (prog, args): (String, mlua::Table)| match &run_user2
        {
            Some(u) => run_capture(
                Some(u),
                &prog,
                &table_to_vec(&args),
                logf_user.clone(),
                cancel_user.clone(),
                child_pid_user.clone(),
            )
            .map_err(mlua::Error::RuntimeError),
            None => Err(mlua::Error::RuntimeError("site 作用域插件未提供运行账号".into())),
        });
        zap_tbl
            .set("exec_as_user", f_user.map_err(|e| format!("exec_as_user 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.site_root / zap.site_linux_user / zap.home_dir
    {
        let run_root = run_root.unwrap_or_default().to_string();
        let run_user3 = run_user.unwrap_or_default().to_string();
        let home_dir = home.to_string();
        let f_root = lua.create_function(move |_, ()| Ok(run_root.clone()));
        zap_tbl
            .set("site_root", f_root.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
        let f_user = lua.create_function(move |_, ()| Ok(run_user3.clone()));
        zap_tbl
            .set("site_linux_user", f_user.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
        let f_home = lua.create_function(move |_, ()| Ok(home_dir.clone()));
        zap_tbl
            .set("home_dir", f_home.map_err(|e| format!("home_dir 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
    }

    lua.globals()
        .set("zap", zap_tbl)
        .map_err(|e| format!("注入 zap 失败: {e}"))?;

    // 执行插件主体（定义 on_run）
    lua.load(code)
        .exec()
        .map_err(|e| format!("插件脚本错误: {e}"))?;

    let on_run: mlua::Function = lua
        .globals()
        .get("on_run")
        .map_err(|e| format!("插件未定义 on_run: {e}"))?;
    let ctx = lua.create_table().map_err(|e| format!("{e}"))?;
    ctx.set("action", action).map_err(|e| format!("{e}"))?;
    ctx.set("scope", scope).map_err(|e| format!("{e}"))?;
    on_run
        .call::<_, ()>(ctx)
        .map_err(|e| format!("on_run 执行失败: {e}"))?;

    Ok(log.borrow().clone())
}

fn table_to_vec(t: &mlua::Table) -> Vec<String> {
    let mut v = Vec::new();
    for pair in t.clone().pairs::<i64, String>() {
        if let Ok((_, s)) = pair {
            v.push(s);
        }
    }
    v
}

/// 以指定身份执行命令并捕获合并输出（stdout + stderr）。
///
/// `logf` 非空时（异步插件），子进程的标准输出/错误会被实时 tee 到该日志文件，
/// 这样前端 SSE 能边跑边看进度；同步插件传 `None`，行为与原来一致（结束一次性返回）。
/// `cancel` / `child_pid` 用于异步插件的运行中取消：被取消时看门狗会杀掉本进程（组），
/// 这里检测标志后提前结束拷贝循环。
fn run_capture(
    user: Option<&str>,
    program: &str,
    args: &[String],
    logf: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>>,
    cancel: Arc<AtomicBool>,
    child_pid: Arc<std::sync::Mutex<Option<(u32, bool)>>>,
) -> Result<String, String> {
    use std::io::{Read, Write};
    let session_leader = user.is_some(); // exec_as_user 走 setsid，pid 即进程组号
    let mut cmd = match user {
        Some(u) => {
            let (mut c, acc) = super::user_cmd(program, u).map_err(|e| format!("降权失败: {e}"))?;
            let (uid, gid) = (acc.uid, acc.gid);
            let _ = unsafe {
                c.pre_exec(move || {
                    libc::setsid();
                    super::cloexec_inherited_fds();
                    super::drop_privileges(uid, gid)?;
                    Ok(())
                })
            };
            c
        }
        None => super::root_cmd(program),
    };
    let mut child = cmd
        .args(args)
        .stdin(Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("执行 {program} 失败: {e}"))?;
    // 登记 pid，供取消看门狗杀进程（组）
    *child_pid.lock().unwrap() = Some((child.id(), session_leader));

    // 两个线程分别把 stdout / stderr 拷进缓冲区，并（异步时）实时落盘
    let copy =
        |r: Option<Box<dyn std::io::Read + Send>>,
         logf: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>>,
         cancel: Arc<AtomicBool>|
         -> String {
        let mut s = String::new();
        if let Some(mut r) = r {
            let mut buf = [0u8; 4096];
            loop {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                match r.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        s.push_str(&chunk);
                        if let Some(f) = &logf {
                            let _ = f.lock().unwrap().write_all(chunk.as_bytes());
                            let _ = f.lock().unwrap().flush();
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        s
    };
    let logf_out = logf.clone();
    let cancel_out = cancel.clone();
    let stdout: Option<Box<dyn std::io::Read + Send>> =
        child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>);
    let stderr: Option<Box<dyn std::io::Read + Send>> =
        child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>);
    let t_out = std::thread::spawn(move || copy(stdout, logf_out, cancel_out));
    let t_err = std::thread::spawn(move || copy(stderr, logf.clone(), cancel.clone()));

    let status = child
        .wait()
        .map_err(|e| format!("等待 {program} 失败: {e}"))?;
    let mut s = t_out.join().unwrap_or_default();
    s.push_str(&t_err.join().unwrap_or_default());

    if !status.success() {
        return Err(format!(
            "命令退出码 {}:\n{}",
            status.code().unwrap_or(-1),
            s
        ));
    }
    Ok(s)
}
