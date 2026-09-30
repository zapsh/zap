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

use serde_json::{Value, json};
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
    let mut items = Vec::new();
    for base in &bases {
        let Ok(rd) = std::fs::read_dir(base) else { continue };
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
                continue;
            }
            let Ok(m) = read_manifest(&p) else { continue };
            if let Some(sc) = &scope {
                if m.get("scope").and_then(|v| v.as_str()) != Some(sc.as_str()) {
                    continue;
                }
            }
            let placement = m
                .get("ui")
                .and_then(|u| u.get("placement"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(sl) = &slot {
                if &placement != sl {
                    continue;
                }
            }
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

    let out = tokio::task::spawn_blocking(move || -> Result<String, String> {
        run_lua(&code, &scope, run_user.as_deref(), run_root.as_deref(), &options, &action)
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
    options: &HashMap<String, String>,
    action: &str,
) -> Result<String, String> {
    let lua = mlua::Lua::new();
    let log = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let zap_tbl = lua
        .create_table()
        .map_err(|e| format!("创建 zap 表失败: {e}"))?;

    // zap.log
    {
        let log = log.clone();
        let f = lua.create_function(move |_, msg: String| {
            log.borrow_mut().push_str(&msg);
            log.borrow_mut().push('\n');
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
        let f_exec = lua.create_function(move |_, (prog, args): (String, mlua::Table)| {
            if scope == "site" {
                return Err(mlua::Error::RuntimeError(
                    "site 作用域禁止 zap.exec，请改用 zap.exec_as_user".into(),
                ));
            }
            run_capture(None, &prog, &table_to_vec(&args)).map_err(mlua::Error::RuntimeError)
        });
        zap_tbl
            .set("exec", f_exec.map_err(|e| format!("exec 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
        let run_user2 = run_user.clone();
        let f_user = lua.create_function(move |_, (prog, args): (String, mlua::Table)| match &run_user2
        {
            Some(u) => run_capture(Some(u), &prog, &table_to_vec(&args))
                .map_err(mlua::Error::RuntimeError),
            None => Err(mlua::Error::RuntimeError("site 作用域插件未提供运行账号".into())),
        });
        zap_tbl
            .set("exec_as_user", f_user.map_err(|e| format!("exec_as_user 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.site_root / zap.site_linux_user
    {
        let run_root = run_root.unwrap_or_default().to_string();
        let run_user3 = run_user.unwrap_or_default().to_string();
        let f_root = lua.create_function(move |_, ()| Ok(run_root.clone()));
        zap_tbl
            .set("site_root", f_root.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
        let f_user = lua.create_function(move |_, ()| Ok(run_user3.clone()));
        zap_tbl
            .set("site_linux_user", f_user.map_err(|e| format!("{e}"))?)
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
fn run_capture(user: Option<&str>, program: &str, args: &[String]) -> Result<String, String> {
    let mut cmd = match user {
        Some(u) => {
            let (mut c, acc) = super::user_cmd(program, u).map_err(|e| format!("降权失败: {e}"))?;
            let (uid, gid) = (acc.uid, acc.gid);
            let _ = unsafe {
                c.pre_exec(move || {
                    unsafe {
                        libc::setsid();
                        super::cloexec_inherited_fds();
                        super::drop_privileges(uid, gid)?;
                    }
                    Ok(())
                })
            };
            c
        }
        None => super::root_cmd(program),
    };
    let out = cmd
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("执行 {program} 失败: {e}"))?;
    let mut s = String::new();
    s.push_str(&String::from_utf8_lossy(&out.stdout));
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        return Err(format!(
            "命令退出码 {}:\n{}",
            out.status.code().unwrap_or(-1),
            s
        ));
    }
    Ok(s)
}
