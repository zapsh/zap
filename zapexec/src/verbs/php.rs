//! PHP-FPM 按用户 pool 管理。
//!
//! 每个面板用户对应一个 Linux 系统账号，站点同步时自动为
//! 「该用户 × 该 PHP 版本」生成一个独立 pool：
//! - `pool_sync`：写入 {php 安装}/etc/php-fpm.d/{linux_user}.conf，
//!   listen unix:/var/run/php-fpm-{linux_user}-{ver}.sock，
//!   pool user = {linux_user}、group = 账号实际主组（worker 以该 Linux 账号运行，进程级隔离），
//!   socket 属主 = {linux_user}:www、mode 0660（nginx worker 属组 www 可连），
//!   open_basedir / session / upload 全部锁进该用户家目录。
//! - `pool_clean`：移除某用户在所有 PHP 实例中的 pool 配置（删除面板用户时调用）。
//!
//! 同一 PHP 版本只有一个 php-fpm master，可以同时加载多个 pool，
//! 每个 pool 各自 user / group / listen socket / 规格，互不影响。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use zap_proto::Response;

use super::root_cmd;

fn linux_user_ok(u: &str) -> bool {
    if u.is_empty() || u.len() > 32 {
        return false;
    }
    let mut chars = u.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn dir_ok(p: &str) -> bool {
    p.starts_with('/') && !p.split('/').any(|s| s == "..")
}

fn php_version(instance: &str) -> String {
    instance.trim_start_matches("php").to_string()
}

fn cmd_err(o: &std::process::Output, fallback: &str) -> String {
    let text = String::from_utf8_lossy(&o.stderr).trim().to_string();
    let text = if text.is_empty() {
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    } else {
        text
    };
    if text.is_empty() {
        fallback.to_string()
    } else {
        text
    }
}

/// 从安装根（默认 /usr/local/apps）下定位 PHP 实例目录（含 etc/php-fpm.conf）。
/// 兼容两种布局：`<root>/php-85` 顶层目录 或 `<root>/<分类>/php-85`。
fn find_php_root(instance: &str) -> Option<PathBuf> {
    let ver = php_version(instance);
    let names = [instance.to_string(), format!("php-{ver}")];
    let base = super::install_root();
    let mut hits = Vec::new();
    if let Ok(top) = std::fs::read_dir(&base) {
        for e in top.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let nm = e.file_name().to_string_lossy().to_string();
            if names.iter().any(|n| n == &nm) && p.join("etc/php-fpm.conf").is_file() {
                hits.push(p.clone());
            }
            // 分类二级目录（application / server / ...）
            if let Ok(sub) = std::fs::read_dir(&p) {
                for e2 in sub.flatten() {
                    let q = e2.path();
                    if !q.is_dir() {
                        continue;
                    }
                    let nm2 = e2.file_name().to_string_lossy().to_string();
                    if names.iter().any(|n| n == &nm2) && q.join("etc/php-fpm.conf").is_file() {
                        hits.push(q);
                    }
                }
            }
        }
    }
    hits.into_iter().next()
}

fn find_fpm_bin(root: &Path) -> Option<PathBuf> {
    for sub in ["sbin/php-fpm", "bin/php-fpm"] {
        let b = root.join(sub);
        if b.is_file() {
            return Some(b);
        }
    }
    None
}

/// 确保 php-fpm.conf include 了 php-fpm.d/*.conf（多 pool 生效前提）。
fn ensure_fpm_include(conf_path: &Path) -> Result<(), String> {
    let content =
        std::fs::read_to_string(conf_path).map_err(|e| format!("读取 php-fpm.conf 失败: {e}"))?;
    let included = content
        .lines()
        .any(|l| l.trim_start().starts_with("include") && !l.trim_start().starts_with(';'));
    if included {
        return Ok(());
    }
    let mut s = content;
    if !s.ends_with('\n') {
        s.push('\n');
    }
    let pool_dir = conf_path
        .parent()
        .map(|p| p.join("php-fpm.d"))
        .ok_or("php-fpm.conf 无父目录")?;
    s.push_str(&format!(
        "\ninclude={}\n",
        pool_dir.join("*.conf").to_string_lossy()
    ));
    std::fs::write(conf_path, s).map_err(|e| format!("更新 php-fpm.conf include 失败: {e}"))
}

/// 默认 pool 规格（spec 为空 / 缺字段时的兜底值）。
///
/// 与面板默认（`fpm_pool_defaults`）保持一致：`ondemand` 按需拉起。站点以独立系统用户
/// 跑独立 pool，大部分 pool 常年无访问，不该为每个 PHP 版本常驻空闲 worker。
fn default_spec() -> BTreeMap<String, String> {
    [
        ("pm", "ondemand"),
        ("max_children", "10"),
        ("max_requests", "1000"),
        ("request_terminate_timeout", "300"),
        ("memory_limit", "256M"),
        ("post_max_size", "128M"),
        ("upload_max_filesize", "128M"),
        ("max_execution_time", "300"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// 渲染单用户 pool 配置文件（纯函数，单测覆盖）。
/// `run_group` 为账号实际主组（同名组被系统占用时可能为 `zap_<user>`）。
fn render_pool_conf(
    linux_user: &str,
    run_group: &str,
    home_dir: &str,
    ver: &str,
    spec: &BTreeMap<String, String>,
) -> String {
    let get = |k: &str, d: &str| -> String {
        spec.get(k)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| d.to_string())
    };
    // 缺省与非法值都回落到按需模式（与面板默认规格一致）
    let mut pm = get("pm", "ondemand");
    if !["dynamic", "static", "ondemand"].contains(&pm.as_str()) {
        pm = "ondemand".to_string();
    }
    let mut out = String::new();
    // 注意：php-fpm 使用 ini 解析器，注释只能用 ';'（'#' 会被当成配置项而报错）
    out.push_str(&format!(
        "; Generated by Zap Panel - php-fpm pool for panel user {linux_user} - DO NOT EDIT\n"
    ));
    out.push_str(&format!("[{linux_user}]\n"));
    out.push_str(&format!("user = {linux_user}\n"));
    out.push_str(&format!("group = {run_group}\n"));
    out.push_str(&format!(
        "listen = /var/run/php-fpm-{linux_user}-{ver}.sock\n"
    ));
    // socket 属主 = 运行账号，属组 = www（nginx worker 以 www 组连接），0660 足够
    out.push_str(&format!("listen.owner = {linux_user}\n"));
    out.push_str("listen.group = www\n");
    out.push_str("listen.mode = 0660\n");
    out.push_str(&format!("pm = {pm}\n"));
    out.push_str(&format!(
        "pm.max_children = {}\n",
        get("max_children", "10")
    ));
    if pm == "dynamic" {
        out.push_str(&format!(
            "pm.start_servers = {}\n",
            get("start_servers", "3")
        ));
        out.push_str(&format!(
            "pm.min_spare_servers = {}\n",
            get("min_spare_servers", "2")
        ));
        out.push_str(&format!(
            "pm.max_spare_servers = {}\n",
            get("max_spare_servers", "5")
        ));
    }
    out.push_str(&format!(
        "pm.max_requests = {}\n",
        get("max_requests", "1000")
    ));
    out.push_str(&format!(
        "request_terminate_timeout = {}\n",
        get("request_terminate_timeout", "300")
    ));
    out.push_str("catch_workers_output = yes\n");
    out.push_str(&format!(
        "php_admin_value[open_basedir] = {home_dir}:/tmp\n"
    ));
    out.push_str(&format!(
        "php_admin_value[session.save_path] = {home_dir}/tmp\n"
    ));
    out.push_str(&format!(
        "php_admin_value[upload_tmp_dir] = {home_dir}/tmp\n"
    ));
    out.push_str(&format!(
        "php_admin_value[memory_limit] = {}\n",
        get("memory_limit", "256M")
    ));
    out.push_str(&format!(
        "php_admin_value[post_max_size] = {}\n",
        get("post_max_size", "128M")
    ));
    out.push_str(&format!(
        "php_admin_value[upload_max_filesize] = {}\n",
        get("upload_max_filesize", "128M")
    ));
    out.push_str(&format!(
        "php_admin_value[max_execution_time] = {}\n",
        get("max_execution_time", "300")
    ));
    out
}

fn parse_spec(spec_json: &str) -> BTreeMap<String, String> {
    let mut m = default_spec();
    if spec_json.trim().is_empty() {
        return m;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(spec_json)
        && let Some(obj) = v.as_object()
    {
        for (k, val) in obj {
            let s = match val {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            m.insert(k.clone(), s);
        }
    }
    m
}

/// 重载目标 php-fpm master：优先服务管理器 reload，退化为向 master 发 USR2。
/// 二者都不可用时不报错（配置已写入并校验通过），由面板层提示手动 reload。
fn reload_master(ver: &str, root: &Path) -> Result<(), String> {
    let pid_candidates = [
        format!("/var/run/php-fpm-{ver}.pid"),
        format!("/run/php-fpm-{ver}.pid"),
        root.join("var/run/php-fpm.pid")
            .to_string_lossy()
            .to_string(),
    ];
    let esc = |s: &str| s.replace('\'', "'\\''");
    let sh = format!(
        // 服务名按发行版差异很大（Alpine 上是 php-fpm83 之类），找不到就自然
        // 退化到下面的 USR2 路径
        "{ctl} reload php-fpm-{v} 2>/dev/null && exit 0; \
         for p in {pids}; do if [ -f \"$p\" ]; then kill -USR2 \"$(cat \"$p\")\" 2>/dev/null && exit 0; fi; done; exit 3",
        ctl = super::svc::CTL,
        v = esc(ver),
        pids = pid_candidates
            .iter()
            .map(|p| format!("'{}'", esc(p)))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let o = root_cmd(super::platform::SHELL)
        .args(["-c", &sh])
        .output()
        .map_err(|e| format!("重载 php-fpm 失败: {e}"))?;
    match o.status.code() {
        Some(3) => Err("未探测到 php-fpm 服务或进程，请手动 reload 使 pool 生效".into()),
        Some(0) => Ok(()),
        _ => Err(cmd_err(&o, "php-fpm reload 失败")),
    }
}

/// 面板用户 → PHP 实例 的专用 pool 同步（幂等）。
pub async fn pool_sync(
    php_instance: String,
    linux_user: String,
    home_dir: String,
    spec: String,
) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let php_instance = &php_instance;
        let linux_user = &linux_user;
        let home_dir = &home_dir;
        let spec = &spec;
        if !linux_user_ok(linux_user) {
            return Err(format!("非法的 Linux 账号名: {linux_user}"));
        }
        if !dir_ok(home_dir) {
            return Err("home_dir 必须是合法绝对路径".into());
        }
        let root = find_php_root(php_instance).ok_or_else(|| {
            format!("未找到 PHP 实例 {php_instance} 的安装（安装根 /usr/local/apps 下无 etc/php-fpm.conf）")
        })?;
        let fpm_bin = find_fpm_bin(&root).ok_or("未找到 php-fpm 可执行文件")?;
        let conf = root.join("etc/php-fpm.conf");
        ensure_fpm_include(&conf)?;
        let pool_dir = root.join("etc/php-fpm.d");
        std::fs::create_dir_all(&pool_dir).map_err(|e| format!("创建 php-fpm.d 失败: {e}"))?;
        // 会话/上传临时目录（open_basedir 白名单指向这里）
        let tmp = format!("{home_dir}/tmp");
        std::fs::create_dir_all(&tmp).map_err(|e| format!("创建用户 tmp 目录失败: {e}"))?;
        let spec_map = parse_spec(spec);
        let ver = php_version(php_instance);
        // pool 的 group 取账号实际主组（同名组被系统占用时落在 zap_<user> 专属组）
        let run_group = super::user::run_group_of(linux_user);
        let content = render_pool_conf(linux_user, &run_group, home_dir, &ver, &spec_map);
        let pool_file = pool_dir.join(format!("{linux_user}.conf"));
        std::fs::write(&pool_file, &content).map_err(|e| format!("写入 pool 配置失败: {e}"))?;
        // 校验：失败即回滚，绝不带着坏配置 reload
        let test = root_cmd(super::platform::SHELL)
            .args(["-c"])
            .arg(format!(
                "'{}' -t -y '{}' 2>&1",
                fpm_bin.to_string_lossy().replace('\'', "'\\''"),
                conf.to_string_lossy().replace('\'', "'\\''")
            ))
            .output()
            .map_err(|e| format!("执行 php-fpm -t 失败: {e}"))?;
        if !test.status.success() {
            let _ = std::fs::remove_file(&pool_file);
            return Err(format!(
                "php-fpm -t 校验失败，已回滚 pool 配置：\n{}",
                cmd_err(&test, "未知错误")
            ));
        }
        match reload_master(&ver, &root) {
            Ok(()) => {
                let data = serde_json::json!({
                    "pool": pool_file.to_string_lossy().to_string(),
                    "socket": format!("/var/run/php-fpm-{linux_user}-{ver}.sock"),
                });
                Ok(Response::ok(
                    format!("PHP-FPM pool 已同步并重载（{linux_user} × {php_instance}）"),
                    Some(data),
                ))
            }
            Err(msg) => Ok(Response::ok(
                format!("pool 配置已写入并通过校验；{msg}"),
                None,
            )),
        }
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 移除某用户在所有已安装 PHP 实例中的 pool 配置并 reload（幂等）。
pub async fn pool_clean(linux_user: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let linux_user = &linux_user;
        if !linux_user_ok(linux_user) {
            return Err(format!("非法的 Linux 账号名: {linux_user}"));
        }
        let mut removed = 0;
        let mut reloaded: Vec<String> = Vec::new();
        // 扫描所有已安装 PHP：目录名 php-{ver}（含分类二级目录）
        let base = super::install_root();
        if let Ok(top) = std::fs::read_dir(&base) {
            for e in top.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let nm = e.file_name().to_string_lossy().to_string();
                if !nm.starts_with("php-") {
                    continue;
                }
                let conf = p.join("etc/php-fpm.conf");
                if !conf.is_file() {
                    continue;
                }
                let pool_file = p.join("etc/php-fpm.d").join(format!("{linux_user}.conf"));
                if pool_file.exists() && std::fs::remove_file(&pool_file).is_ok() {
                    removed += 1;
                    let ver = nm.trim_start_matches("php-").to_string();
                    if reload_master(&ver, &p).is_ok() {
                        reloaded.push(ver);
                    }
                }
            }
        }
        if removed == 0 {
            return Ok(Response::ok("未发现该用户的 PHP-FPM pool 配置", None));
        }
        let data = serde_json::json!({ "removed": removed, "reloaded": reloaded });
        Ok(Response::ok(
            format!("已移除 {linux_user} 的 pool 配置 {removed} 个"),
            Some(data),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_basic_pool() {
        let spec = default_spec();
        let s = render_pool_conf("zap", "zap", "/home/zap", "8.3", &spec);
        assert!(s.contains("[zap]"));
        assert!(s.contains("user = zap"));
        assert!(s.contains("group = zap"));
        assert!(s.contains("listen = /var/run/php-fpm-zap-8.3.sock"));
        assert!(s.contains("listen.owner = zap"));
        assert!(s.contains("listen.group = www"));
        assert!(s.contains("listen.mode = 0660"));
        assert!(s.contains("pm = ondemand"));
        assert!(s.contains("pm.max_children = 10"));
        // 按需模式不写 dynamic 专属的空闲进程项
        assert!(!s.contains("pm.start_servers"));
        assert!(s.contains("open_basedir] = /home/zap:/tmp"));
        assert!(s.contains("session.save_path] = /home/zap/tmp"));
    }

    /// 同名组被系统占用（如 admin）：pool 的 group 必须落到专属运行组
    #[test]
    fn render_pool_uses_resolved_run_group() {
        let spec = default_spec();
        let s = render_pool_conf("admin", "zap_admin", "/home/admin", "8.3", &spec);
        assert!(s.contains("[admin]"));
        assert!(s.contains("user = admin"));
        assert!(s.contains("group = zap_admin"));
        assert!(s.contains("listen.owner = admin"));
    }

    #[test]
    fn render_static_pool_uses_children_only() {
        let mut spec = default_spec();
        spec.insert("pm".into(), "static".into());
        spec.insert("max_children".into(), "4".into());
        let s = render_pool_conf("zap", "zap", "/home/zap", "8.1", &spec);
        assert!(s.contains("pm = static"));
        assert!(s.contains("pm.max_children = 4"));
        assert!(!s.contains("start_servers"));
        assert!(s.contains("php-fpm-zap-8.1.sock"));
    }

    #[test]
    fn invalid_pm_falls_back() {
        let mut spec = default_spec();
        spec.insert("pm".into(), "bogus".into());
        let s = render_pool_conf("zap", "zap", "/home/zap", "8.2", &spec);
        assert!(s.contains("pm = ondemand"));
    }

    #[test]
    fn linux_user_name_rules() {
        assert!(linux_user_ok("zap"));
        assert!(linux_user_ok("zap_01-a"));
        assert!(linux_user_ok("Zap"));
        assert!(!linux_user_ok("1zap"));
        assert!(!linux_user_ok(""));
        assert!(!linux_user_ok("ab cd"));
        assert!(!linux_user_ok("za:p"));
    }
}
