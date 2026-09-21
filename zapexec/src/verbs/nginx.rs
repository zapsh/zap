//! Nginx 服务配置与运行状态管理（root 执行）。
//!
//! 提供「服务器配置 → Nginx 配置」（可视化 + 文件编辑）与
//! 「服务器状态 → Nginx Server」两个页面的后端能力：
//!
//! - 状态探测：安装位置 / 版本 / 主配置路径 / 运行态（systemd 或 pid）
//! - 配置读写：`<nginx prefix>/conf` 目录内的文件均可编辑（含 `mime.types`、
//!   `fastcgi_params` 等无 `.conf` 后缀的 include 数据文件）；主配置
//!   include 指令（`conf.d/*.conf`、`sites-enabled/*.conf` 等）指向 conf 目录树外的
//!   目标目录时，仅放行其中的 `*.conf`
//!   （面板托管的站点 vhost 以 `zap-` 开头被排除，避免误改托管文件）
//! - 保存链路：备份 → 写入 → `nginx -t` 校验 → 失败自动回滚 → 运行中则重载
//! - 服务控制：start / stop / restart / reload（优先 systemd unit `nginx`）
//! - 状态页监控：`stub_status` 挂在面板默认站点（`00-default.conf` 的 80 server）上，
//!   不再单独占端口，探测 / 启停并采集请求统计（仅本机 127.0.0.1 可访问）

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use zap_proto::Response;

use super::root_cmd;
use super::site;

/// 单个配置文件体积上限（读取与写入共用，防止意外读取超大文件拖垮连接）。
const MAX_CONF_BYTES: u64 = 2 * 1024 * 1024;

/// 从 conf 目录树内收集可编辑配置文件的防御性最大深度。
/// conf/ 及其任意子目录（conf.d/、sites-enabled/、用户自建 vhosts/ 等）都应可编辑，
/// 上限仅用于防御 symlink 循环等异常目录结构导致的无限递归。
const SCAN_DEPTH: usize = 16;

// ── 探测与工具 ─────────────────────────────────────────────

/// (主配置, nginx 二进制)。未探测到安装时返回 None。
fn probe() -> Option<(PathBuf, PathBuf)> {
    let conf = site::find_nginx_conf_file()?;
    let bin = site::nginx_bin(&conf);
    Some((conf, bin))
}

/// 输出 stderr（截断），供校验失败等场景使用。
fn output_err(o: &std::process::Output, fallback: &str) -> String {
    let text = String::from_utf8_lossy(&o.stderr).trim().to_string();
    let text = if text.is_empty() {
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    } else {
        text
    };
    if text.is_empty() {
        fallback.to_string()
    } else {
        text.chars().take(2000).collect()
    }
}

fn quote_bin(bin: &Path) -> String {
    bin.to_string_lossy().replace('\'', "'\\''")
}

/// 读取 nginx 版本（`nginx -v` 的输出首行）。
fn nginx_version(bin: &Path) -> String {
    let o = root_cmd(&bin.to_string_lossy()).arg("-v").output();
    match o {
        Ok(o) => {
            // `nginx -v` 将版本输出到 stderr，个别包装脚本可能走 stdout，两者都读。
            let text = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let text = if text.is_empty() {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            } else {
                text
            };
            text.lines().next().unwrap_or("").trim().to_string()
        }
        Err(_) => String::new(),
    }
}

/// 读取 master pid（/var/run/nginx.pid）。
fn read_pid() -> Option<i64> {
    let text = std::fs::read_to_string("/var/run/nginx.pid").ok()?;
    text.trim().parse::<i64>().ok().filter(|p| *p > 0)
}

/// 反复探测直至稳定（最多约 3 秒），返回最终 running 状态。
fn settle_running(want: bool, tries: u32) -> bool {
    for _ in 0..tries {
        let running = site::nginx_running();
        if running == want {
            return running;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    site::nginx_running()
}

/// 校验路径属于可编辑白名单：主配置本身、其 conf 目录树内的任意文件，
/// 或主配置 include 指向目录内的 *.conf（排除 zap 托管前缀与备份/临时文件）。
fn validate_conf_path(raw: &str) -> Result<(PathBuf, PathBuf, bool), String> {
    let Some((conf, bin)) = probe() else {
        return Err("Nginx 未安装或未探测到主配置，请先在应用商店安装 Nginx".to_string());
    };
    let conf_dir = conf
        .parent()
        .ok_or_else(|| "主配置路径异常".to_string())?
        .to_path_buf();
    let main_canon = conf
        .canonicalize()
        .map_err(|e| format!("主配置不可访问: {e}"))?;

    let path = PathBuf::from(raw);
    let canon = path
        .canonicalize()
        .map_err(|e| format!("文件不存在或不可访问: {e}"))?;

    let is_main = canon == main_canon;
    if is_main {
        return Ok((main_canon, bin, true));
    }
    // 非主配置：conf 目录树内的文件均可编辑（mime.types / fastcgi_params 等）；
    // 主配置 include（conf.d / sites-enabled 等）指向 conf 目录树外的目录，仅放行 *.conf
    let conf_dir_canon = conf_dir
        .canonicalize()
        .map_err(|e| format!("conf 目录不可访问: {e}"))?;
    let in_conf_tree = canon.starts_with(&conf_dir_canon);
    let include_roots: Vec<PathBuf> = parse_include_targets(&conf, &conf_dir)
        .into_iter()
        .filter_map(|t| include_dir_root(&t))
        .filter_map(|r| r.canonicalize().ok())
        .collect();
    if !in_conf_tree && !include_roots.iter().any(|r| canon.starts_with(r)) {
        return Err("仅允许编辑 Nginx conf 目录内的文件，或主配置 include（conf.d / sites-enabled）目录内的 .conf 文件".to_string());
    }
    let name = canon
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    if name.starts_with("zap-")
        || name.ends_with(".bak")
        || name.ends_with(".tmp")
        || name.contains(".zap")
    {
        return Err("该文件为面板托管的站点/缓存配置，请勿直接编辑".to_string());
    }
    if !in_conf_tree && !name.ends_with(".conf") {
        return Err("include 目录内仅支持编辑 .conf 配置文件".to_string());
    }
    Ok((canon, bin, false))
}

/// 数据区备份目录：`{ZAP_PATH}/data/backups/nginx`。
/// 统一约定：所有服务的配置文件备份都放在 `{ZAP_PATH}/data/backups/<svc>`
/// 下，每服务一个子目录（nginx、php74、php81…），便于统一浏览与容量管理。
fn backup_dir() -> PathBuf {
    site::zap_path().join("data/backups/nginx")
}

/// 备份当前文件（保留最近 20 份）。
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

    // 修剪旧备份（只保留文件名时间戳最新的 20 份）
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

// ── verbs ──────────────────────────────────────────────────

/// nginx.status：安装 / 版本 / 配置路径 / 运行态。
pub async fn status() -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let Some((conf, bin)) = probe() else {
            return Ok(Response::ok(
                "nginx 未安装",
                Some(json!({ "installed": false })),
            ));
        };
        let running = site::nginx_running();
        Ok(Response::ok(
            "ok",
            Some(json!({
                "installed": true,
                "conf_file": conf.display().to_string(),
                "conf_dir": conf.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                "bin": bin.display().to_string(),
                "version": nginx_version(&bin),
                "running": running,
                "pid": read_pid(),
                // 这两个字段名是历史命名（面板依赖），实际调用已平台无关
                "systemd": super::svc::exists("nginx"),
                "systemd_active": super::svc::is_active("nginx"),
                "default_conf": default_vhost_path().display().to_string(),
                "default_ip_access": default_ip_access(),
                "default_page": default_page_path().display().to_string(),
            })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// nginx.conf_list：列出主配置 + conf 目录白名单文件。
pub async fn conf_list() -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let Some((conf, _bin)) = probe() else {
            return Ok(Response::ok(
                "nginx 未安装",
                Some(json!({
                    "installed": false,
                    "files": [],
                })),
            ));
        };
        let conf_dir = conf.parent().unwrap_or(Path::new("/")).to_path_buf();
        let mut files: Vec<serde_json::Value> = Vec::new();
        let meta = std::fs::metadata(&conf).map_err(|e| format!("读取主配置失败: {e}"))?;
        files.push(file_entry(&conf, &conf_dir, true, meta.len()));

        // 目录扫描会再次收录主配置自身,跳过它避免列表出现两个 nginx.conf
        collect_conf_files(&conf_dir, &conf_dir, 0, Some(&conf), &mut files);
        // include 感知：主配置 include（conf.d/*.conf、sites-enabled/*.conf 等）指到的文件
        // 即使位于 conf 目录树外也一并收录（排除面板托管 zap-* 与模块加载目录）。
        let mut seen: std::collections::HashSet<PathBuf> = files
            .iter()
            .filter_map(|f| f.get("path").and_then(|p| p.as_str()).map(PathBuf::from))
            .collect();
        for target in parse_include_targets(&conf, &conf_dir) {
            let Some(root) = include_dir_root(&target) else {
                continue;
            };
            let Ok(root_c) = root.canonicalize() else {
                continue;
            };
            let mut hits: Vec<PathBuf> = Vec::new();
            expand_include(&target, &mut hits);
            for hit in hits {
                let canon = hit.canonicalize().unwrap_or(hit);
                // canonicalize 后飞出 include 根目录的文件是软链指向的面板托管文件
                //（如 00-default.conf 位于 sites-available），不属于可手工编辑范围
                if !canon.starts_with(&root_c) {
                    continue;
                }
                let name = canon
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                if !seen.insert(canon.clone())
                    || !name.ends_with(".conf")
                    || name.starts_with("zap-")
                    || name.ends_with(".bak")
                    || name.ends_with(".tmp")
                    || name.contains(".zap")
                {
                    continue;
                }
                let Ok(meta) = std::fs::metadata(&canon) else {
                    continue;
                };
                if meta.len() > MAX_CONF_BYTES {
                    continue;
                }
                files.push(file_entry(&canon, &conf_dir, false, meta.len()));
            }
        }

        Ok(Response::ok(
            "ok",
            Some(json!({
                "installed": true,
                "conf_file": conf.display().to_string(),
                "conf_dir": conf_dir.display().to_string(),
                "files": files,
            })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

fn file_entry(path: &Path, base: &Path, is_main: bool, size: u64) -> serde_json::Value {
    let rel = path
        .strip_prefix(base)
        .map(|p| p.display().to_string())
        .or_else(|_| path.strip_prefix("/").map(|p| p.display().to_string()))
        .unwrap_or_else(|_| path.display().to_string());
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    json!({
        "path": path.display().to_string(),
        "rel": rel,
        "name": path.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
        "is_main": is_main,
        "size": size,
        "mtime": mtime,
    })
}

/// 解析主配置中的 include 指令，返回 include 目标路径（相对路径以 conf 目录为基准）。
/// 跳过注释行与模块加载目录（如 /etc/nginx/modules-enabled），避免把 load_module 片段混入可编辑列表。
fn parse_include_targets(conf: &Path, conf_dir: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(conf) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some(rest) = t.strip_prefix("include") else {
            continue;
        };
        let rest = rest.trim().trim_end_matches(';').trim();
        if rest.is_empty() {
            continue;
        }
        // 去掉行内注释（# 后的内容）
        let rest = rest.split('#').next().unwrap_or(rest).trim();
        if rest.starts_with("http_")
            || Path::new(rest)
                .components()
                .any(|c| c.as_os_str().to_string_lossy().contains("modules"))
        {
            continue;
        }
        let p = Path::new(rest);
        let target = if p.is_absolute() {
            p.to_path_buf()
        } else {
            conf_dir.join(p)
        };
        out.push(target);
    }
    out
}

/// 通配匹配（仅 * 与 ?，Linux 大小写敏感）。
fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = name.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark): (Option<usize>, usize) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// 展开 include 目标（支持多级通配目录，如 sites-enabled/*/*.conf）为实际文件路径。
fn expand_include(target: &Path, out: &mut Vec<PathBuf>) {
    let s = target.to_string_lossy();
    if !s.contains('*') && !s.contains('?') {
        if target.is_file() {
            out.push(target.to_path_buf());
        }
        return;
    }
    let comps: Vec<&str> = s.split('/').filter(|c| !c.is_empty()).collect();
    if comps.is_empty() {
        return;
    }
    let mut root = PathBuf::new();
    if s.starts_with('/') {
        root.push("/");
    }
    expand_glob_dir(&root, &comps, out);
}

fn expand_glob_dir(dir: &Path, comps: &[&str], out: &mut Vec<PathBuf>) {
    let Some((head, rest)) = comps.split_first() else {
        return;
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let nm = e.file_name();
        let nm = nm.to_string_lossy();
        if !glob_match(head, &nm) {
            continue;
        }
        let p = e.path();
        if rest.is_empty() {
            if p.is_file() {
                out.push(p);
            }
        } else if p.is_dir() {
            expand_glob_dir(&p, rest, out);
        }
    }
}

/// include 目标对应的可编辑根目录：通配目标取通配符前的目录段；单文件 include 取父目录。
fn include_dir_root(target: &Path) -> Option<PathBuf> {
    let s = target.to_string_lossy();
    let mut root = PathBuf::new();
    if s.starts_with('/') {
        root.push("/");
    }
    let mut wildcard_seen = false;
    for seg in s.split('/') {
        if seg.is_empty() {
            continue;
        }
        if seg.contains('*') || seg.contains('?') {
            wildcard_seen = true;
            break;
        }
        root.push(seg);
    }
    if !wildcard_seen && target.is_file() {
        root.pop(); // 单文件 include（如 conf 子目录里的独立文件）：允许其所在目录
    }
    (!root.as_os_str().is_empty()).then_some(root)
}

fn collect_conf_files(
    dir: &Path,
    base: &Path,
    depth: usize,
    skip: Option<&Path>,
    out: &mut Vec<serde_json::Value>,
) {
    if depth > SCAN_DEPTH {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            // 不跟随符号链接目录,避免 conf 树内出现 symlink 环时无限递归;
            // 常规 nginx 子目录均为真实目录,不受影响。
            if p.is_symlink() {
                continue;
            }
            collect_conf_files(&p, base, depth + 1, skip, out);
            continue;
        }
        if let Some(skip_path) = skip
            && p == skip_path
        {
            continue; // 主配置已单独收录,跳过目录扫描里的同名条目
        }
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if name.starts_with("zap-")
            || name.ends_with(".bak")
            || name.ends_with(".tmp")
            || name.contains(".zap")
        {
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&p) {
            if meta.len() > MAX_CONF_BYTES {
                continue; // 超出读取上限的文件不提供在线编辑
            }
            out.push(file_entry(&p, base, false, meta.len()));
        }
    }
}

/// nginx.conf_read：读取白名单文件内容。
pub async fn conf_read(path: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (canon, _bin, is_main) = validate_conf_path(&path)?;
        let meta = std::fs::metadata(&canon).map_err(|e| format!("读取文件失败: {e}"))?;
        if meta.len() > MAX_CONF_BYTES {
            return Err("文件过大，不支持在线编辑（请到终端处理）".to_string());
        }
        let content =
            std::fs::read_to_string(&canon).map_err(|e| format!("读取文件失败: {e}"))?;
        Ok(Response::ok(
            "ok",
            Some(json!({
                "path": canon.display().to_string(),
                "is_main": is_main,
                "content": content,
                "size": meta.len(),
                "mtime": meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0),
            })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// nginx.conf_save：备份 → 写入 → `nginx -t` 校验 → 失败回滚 → 运行中重载。
pub async fn conf_save(path: String, content: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        if content.len() as u64 > MAX_CONF_BYTES {
            return Err("内容超过 2MB，请精简后重试".to_string());
        }
        let (canon, bin, is_main) = validate_conf_path(&path)?;

        let old = std::fs::read_to_string(&canon).map_err(|e| format!("读取原文件失败: {e}"))?;

        // 主配置保护：不得删除面板托管的站点 include 行（否则站点将全部失效）
        if is_main && old.contains("sites-enabled") && !content.contains("sites-enabled") {
            return Err(
                "配置中缺少面板托管的 include（sites-enabled 目录），已取消保存以避免站点全部失效。\n\
                 请保留 # zap: 面板托管站点配置 那一行 include"
                    .to_string(),
            );
        }

        let backup = backup_file(&canon)?;

        // 写入（同目录 tmp + rename，原子替换）
        let tmp = canon.with_extension("conf.tmp");
        std::fs::write(&tmp, &content).map_err(|e| format!("写入配置失败: {e}"))?;
        std::fs::rename(&tmp, &canon).map_err(|e| format!("替换配置失败: {e}"))?;

        // nginx -t 校验
        if let Err(e) = site::nginx_test(&bin) {
            // 回滚
            let _ = std::fs::copy(&backup, &canon);
            return Err(format!("nginx -t 校验未通过，已自动回滚原配置：\n{e}"));
        }

        // 重载（未运行则跳过，不视为错误）
        let running = site::nginx_running();
        let (reloaded, reason) = if running {
            match site::reload_nginx(&bin) {
                Ok(()) => (true, String::new()),
                Err(e) => (false, e),
            }
        } else {
            (
                false,
                "nginx 未运行，配置已保存，将在启动时生效".to_string(),
            )
        };

        Ok(Response::ok(
            "ok",
            Some(json!({
                "path": canon.display().to_string(),
                "backup": backup.display().to_string(),
                "tested": true,
                "reloaded": reloaded,
                "reason": reason,
            })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

// ── 默认站点（IP / 未匹配域名兜底）─────────────────────────

/// 默认站点欢迎页目录：`{ZAP_PATH}/data/www/_zap`（与维护页同目录，nginx worker 可读）。
fn default_page_dir() -> PathBuf {
    site::zap_path().join("data/www/_zap")
}

/// 默认站点欢迎页文件（可直接编辑定制 IP 直连展示内容）。
fn default_page_path() -> PathBuf {
    default_page_dir().join("ip-index.html")
}

/// 面板托管默认站点文件（webservers 数据根下的 sites-available）。
fn default_vhost_path() -> PathBuf {
    super::webconf::available_dir("nginx").join(site::DEFAULT_VHOST_FILE)
}

/// 当前默认站点形态：true = 欢迎页（开启 IP 访问）；false = 444 断开。
fn default_ip_access() -> bool {
    std::fs::read_to_string(default_vhost_path())
        .map(|c| c.contains("ip-index.html"))
        .unwrap_or(false)
}

/// 确保默认站点欢迎页存在（不存在时生成默认页面，目录 0755 / 文件 0644）。
fn ensure_default_page() -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = default_page_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建默认页目录失败: {e}"))?;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
    let file = default_page_path();
    if !file.exists() {
        let html = concat!(
            "<!DOCTYPE html>\n",
            "<html lang=\"zh-CN\">\n",
            "<head><meta charset=\"utf-8\">",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
            "<title>服务器默认页</title>\n",
            "<style>body{font-family:system-ui,-apple-system,\"PingFang SC\",\"Microsoft YaHei\",sans-serif;",
            "display:flex;align-items:center;justify-content:center;height:100vh;margin:0;",
            "background:#f5f7fa;color:#303133}",
            ".box{text-align:center;padding:32px}",
            "h1{font-size:22px;margin:0 0 12px}p{color:#909399;font-size:14px;line-height:1.8;margin:0 0 6px}</style>\n",
            "</head>\n",
            "<body><div class=\"box\">",
            "<h1>服务器已就绪</h1>",
            "<p>该页面由 Zap 面板的默认站点提供，用于通过服务器 IP 或未绑定域名访问的场景。</p>",
            "<p>如需修改展示内容，请直接编辑默认站点欢迎页文件。</p>",
            "</div></body></html>\n"
        );
        std::fs::write(&file, html).map_err(|e| format!("写入默认页失败: {e}"))?;
    }
    let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644));
    Ok(file)
}

/// 默认站点「开启 IP 访问」形态：IP / 未匹配域名展示欢迎页；
/// 443 无证书仍拒绝握手，避免被任意站点的 443 接走造成串站。
/// `stub=true` 时带上状态页 location（精确匹配优先于 `/`，不影响欢迎页）。
fn render_ip_vhost(page_dir: &Path, stub: bool) -> String {
    format!(
        "# Generated by Zap Panel — 默认站点（IP / 未匹配域名欢迎页）— DO NOT EDIT\n\
         server {{\n\
         \x20   listen 80 default_server;\n\
         \x20   listen [::]:80 default_server;\n\
         \x20   server_name _;\n\
         {stub}\
         \x20   root {root};\n\
         \x20   index ip-index.html;\n\
         }}\n\
         server {{\n\
         \x20   listen 443 ssl default_server;\n\
         \x20   listen [::]:443 ssl default_server;\n\
         \x20   server_name _;\n\
         \x20   ssl_reject_handshake on;\n\
         }}\n",
        stub = site::render_stub_location(stub),
        root = page_dir.display(),
    )
}

/// nginx.default_vhost：设置默认站点（IP / 未匹配域名兜底）形态。
/// enable=false → 直接断开（80 返回 444、443 拒握手，防串站）；
/// enable=true  → IP / 未匹配域名展示欢迎页（页面文件 `ip-index.html` 可自行编辑定制）。
pub async fn default_vhost(enable: bool) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let Some((conf, bin)) = probe() else {
            return Err("Nginx 未安装或未探测到主配置".to_string());
        };
        // 停用官方模板自带默认站点，避免 duplicate default server
        site::deactivate_official_default(&conf);
        let edir = super::webconf::enabled_dir("nginx");
        let injected = super::webconf::ensure_include(&conf, &edir, "nginx")
            .map_err(|e| format!("主配置注入 include 失败: {e}"))?;
        // 状态页开关与 IP 访问形态互相保留：改形态不丢状态页，改状态页不丢形态
        let stub = stub_enabled();
        // 旧版独立端口的状态页文件（若还在）随本次重建一起清掉
        let purged_legacy = purge_legacy_stub();
        let new = if enable {
            ensure_default_page()?;
            render_ip_vhost(&default_page_dir(), stub)
        } else {
            site::render_default_vhost(stub)
        };

        let path = default_vhost_path();
        let old = std::fs::read_to_string(&path).ok();
        // 失败回滚：恢复旧内容（或移除新建文件），并回滚本次的 include 注入
        let rollback = || {
            match &old {
                Some(o) => {
                    let _ = super::webconf::publish_named("nginx", site::DEFAULT_VHOST_FILE, o);
                }
                None => {
                    let _ = super::webconf::purge_named("nginx", site::DEFAULT_VHOST_FILE);
                }
            }
            if injected {
                super::webconf::restore_include(&conf);
            }
        };
        if let Err(e) = super::webconf::publish_named("nginx", site::DEFAULT_VHOST_FILE, &new) {
            rollback();
            return Err(format!("写入默认站点配置失败: {e}"));
        }
        if let Err(e) = site::nginx_test(&bin) {
            rollback();
            return Err(format!("nginx -t 校验未通过，已回滚默认站点配置：\n{e}"));
        }

        let running = site::nginx_running();
        let (reloaded, reason) = if running {
            match site::reload_nginx(&bin) {
                Ok(()) => (true, String::new()),
                Err(e) => (false, e),
            }
        } else {
            (
                false,
                "nginx 未运行，配置已生效，将在启动时加载".to_string(),
            )
        };
        Ok(Response::ok(
            "ok",
            Some(json!({
                "enable": enable,
                "stub": stub,
                "purged_legacy": purged_legacy,
                "reloaded": reloaded,
                "reason": reason,
                "default_conf": path.display().to_string(),
                "default_page": default_page_path().display().to_string(),
            })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

// ── 状态页 stub_status 采集 ─────────────────────────────

/// 旧版状态页 server 文件名（独立监听 127.0.0.1 的一个端口）。
///
/// 已改为挂在面板默认站点（`00-default.conf`）上，不再单独占端口；这里保留名字
/// 只为升级时清理存量文件（见 [`purge_legacy_stub`]）。
const LEGACY_STUB_SERVER_FILE: &str = "zap-stub.conf";
/// 状态页 URL 路径（注入默认站点，见 `site::STUB_PATH`）。
const STUB_PATH: &str = "/nginx_status";

/// 状态页采集指标（stub_status 输出）。
#[derive(Default)]
struct StubMetrics {
    active: u64,
    accepts: u64,
    handled: u64,
    requests: u64,
    reading: u64,
    writing: u64,
    waiting: u64,
}

/// 旧版状态页配置文件路径（独立端口方案）。
fn legacy_stub_path() -> PathBuf {
    super::webconf::available_dir("nginx").join(LEGACY_STUB_SERVER_FILE)
}

/// 清理旧版状态页配置（available + enabled 软链）。
/// 只做 best-effort：残留顶多多监听一个本机端口，不影响面板采集。
fn purge_legacy_stub() -> bool {
    super::webconf::purge_named("nginx", LEGACY_STUB_SERVER_FILE).unwrap_or(false)
}

/// 状态页是否已启用：默认站点里带 `stub_status`，或仍是旧版独立文件。
fn stub_enabled() -> bool {
    if std::fs::read_to_string(default_vhost_path())
        .map(|c| c.contains("stub_status"))
        .unwrap_or(false)
    {
        return true;
    }
    legacy_stub_path().exists()
}

/// 默认站点对外提供 HTTP 的端口（状态页就挂在这个 server 上）。
/// 解析首个非 ssl 的 `listen`；认不出来就按 80 走。
fn stub_port() -> u16 {
    let content = std::fs::read_to_string(default_vhost_path()).unwrap_or_default();
    for line in content.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix("listen ") else {
            continue;
        };
        if rest.contains("ssl") {
            continue;
        }
        let tok = rest.split_whitespace().next().unwrap_or("");
        let addr = tok.trim_end_matches(';');
        // 形如：80 / 127.0.0.1:80 / [::]:80 / *:80
        let port = addr.rsplit(':').next().unwrap_or(addr);
        let port = port.trim_end_matches(']').trim_start_matches('[');
        if let Ok(p) = port.parse::<u16>() {
            return p;
        }
    }
    80
}

/// 采集状态页文本并解析为指标。
fn fetch_stub(port: u16) -> Result<StubMetrics, String> {
    use std::io::{Read, Write};
    let addr = format!("127.0.0.1:{port}");
    let mut stream =
        std::net::TcpStream::connect(&addr).map_err(|e| format!("连接状态页 {addr} 失败: {e}"))?;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(3)));
    let req = format!(
        "GET {STUB_PATH} HTTP/1.0\r\nHost: 127.0.0.1\r\nUser-Agent: zap-panel\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("请求状态页失败: {e}"))?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取状态页失败: {e}"))?;
    let text = String::from_utf8_lossy(&buf).to_string();
    let body = text.split("\r\n\r\n").nth(1).unwrap_or(&text);
    parse_stub(body)
}

/// 解析 stub_status 文本：
/// ```text
/// Active connections: 132
/// server accepts handled requests
///  4189490 4189490 105460000
/// Reading: 5 Writing: 32 Waiting: 95
/// ```
fn parse_stub(body: &str) -> Result<StubMetrics, String> {
    let mut m = StubMetrics::default();
    for line in body.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("Active connections:") {
            m.active = v.trim().parse().unwrap_or(0);
        } else if l.starts_with("Reading:") {
            let toks: Vec<&str> = l.split_whitespace().collect();
            let mut i = 0;
            while i + 1 < toks.len() {
                let key = toks[i].trim_end_matches(':');
                let val = toks[i + 1].parse().unwrap_or(0);
                match key {
                    "Reading" => m.reading = val,
                    "Writing" => m.writing = val,
                    "Waiting" => m.waiting = val,
                    _ => {}
                }
                i += 2;
            }
        } else if !l.starts_with("server accepts handled requests") {
            let toks: Vec<&str> = l.split_whitespace().collect();
            if toks.len() == 3 && toks.iter().all(|t| t.chars().all(|c| c.is_ascii_digit())) {
                m.accepts = toks[0].parse().unwrap_or(0);
                m.handled = toks[1].parse().unwrap_or(0);
                m.requests = toks[2].parse().unwrap_or(0);
            }
        }
    }
    Ok(m)
}

/// 读取主配置里的指令值（`worker_processes` / `worker_connections`）。
fn config_directive(conf: &Path, name: &str) -> Option<String> {
    let content = std::fs::read_to_string(conf).ok()?;
    for line in content.lines() {
        let (code, _) = line.split_once('#').unwrap_or((line, ""));
        let l = code.trim();
        if (l == name || l.starts_with(&format!("{name} ")) || l.starts_with(&format!("{name}\t")))
            && let Some(rest) = l[name.len()..].split_whitespace().next()
        {
            let v = rest.trim_end_matches(';');
            if v.eq_ignore_ascii_case("auto") || v.chars().all(|c| c.is_ascii_digit()) {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// 遍历 /proc 统计 nginx 进程分布（master / worker / cache）。
fn nginx_processes() -> serde_json::Value {
    let mut master = 0u32;
    let mut workers = 0u32;
    let mut cache = 0u32;
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let Ok(cmd) = std::fs::read_to_string(format!("/proc/{name}/cmdline")) else {
                continue;
            };
            let c = cmd.replace('\0', " ");
            if !c.contains("nginx") {
                continue;
            }
            if c.contains("worker process") {
                workers += 1;
            } else if c.contains("cache manager process") || c.contains("cache loader process") {
                cache += 1;
            } else {
                master += 1;
            }
        }
    }
    json!({
        "master": master,
        "workers": workers,
        "cache": cache,
        "total": master + workers + cache,
    })
}

/// nginx.stub_status：查询 / 设置状态页。
/// enable=None → 查询是否启用并采集指标；
/// enable=Some(true/false) → 开启 / 关闭状态页。
///
/// 状态页不再单开端口：`location = /nginx_status` 直接挂在面板托管的默认站点
/// （`00-default.conf` 的 80 server）上，采集端走 127.0.0.1:<默认 HTTP 端口>，
/// 靠 `allow/deny` 拦住外部访问。开关状态与默认站点的「IP 访问」形态互相保留
/// （两处渲染都带 stub 参数，改一个不会丢另一个）。
pub async fn stub_status(enable: Option<bool>) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let Some((conf, bin)) = probe() else {
            return Err("Nginx 未安装或未探测到主配置".to_string());
        };
        let worker_processes = config_directive(&conf, "worker_processes").unwrap_or_default();
        let worker_connections = config_directive(&conf, "worker_connections").unwrap_or_default();
        let running = site::nginx_running();

        match enable {
            None => {
                // 旧版（独立端口）存量配置：首次查询时顺手迁到默认站点
                migrate_legacy_stub(&conf, &bin);
                let enabled = stub_enabled();
                let port = stub_port();
                let metrics = if enabled && running {
                    match fetch_stub(port).ok() {
                        Some(m) => json!({
                            "active": m.active,
                            "accepts": m.accepts,
                            "handled": m.handled,
                            "requests": m.requests,
                            "reading": m.reading,
                            "writing": m.writing,
                            "waiting": m.waiting,
                        }),
                        None => serde_json::Value::Null,
                    }
                } else {
                    serde_json::Value::Null
                };
                Ok(Response::ok(
                    "ok",
                    Some(json!({
                        "enabled": enabled,
                        "port": port,
                        "path": STUB_PATH,
                        "metrics": metrics,
                        "worker_processes": worker_processes,
                        "worker_connections": worker_connections,
                        "processes": nginx_processes(),
                        "running": running,
                    })),
                ))
            }
            Some(on) => {
                let (purged_legacy, reloaded, reason) = apply_stub(&conf, &bin, on)?;
                Ok(Response::ok(
                    "ok",
                    Some(json!({
                        "enable": on,
                        "port": stub_port(),
                        "path": STUB_PATH,
                        "purged_legacy": purged_legacy,
                        "reloaded": reloaded,
                        "reason": reason,
                    })),
                ))
            }
        }
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 把状态页开关写进默认站点（`00-default.conf`）并重载：
/// `on=true` 注入 `location = /nginx_status`，`false` 移除。
///
/// 默认站点当前的「IP 访问」形态原样保留，只动状态页这一段。
/// 返回 `(是否清掉了旧版文件, 是否已 reload, 未 reload 的原因)`。
fn apply_stub(conf: &Path, bin: &Path, on: bool) -> Result<(bool, bool, String), String> {
    // 旧版（独立端口）残留：顺手清掉，避免白占一个本机端口
    let purged_legacy = purge_legacy_stub();
    // 状态页挂在默认站点上，先保证它存在（不存在时按当前形态创建一份）
    site::deactivate_official_default(conf);
    site::ensure_default_vhost(conf, bin);
    let edir = super::webconf::enabled_dir("nginx");
    let injected = super::webconf::ensure_include(conf, &edir, "nginx")
        .map_err(|e| format!("主配置注入 include 失败: {e}"))?;
    let path = default_vhost_path();
    let old = std::fs::read_to_string(&path).ok();
    // 保留默认站点当前的「IP 访问」形态，只改状态页这一段
    let new = if default_ip_access() {
        ensure_default_page()?;
        render_ip_vhost(&default_page_dir(), on)
    } else {
        site::render_default_vhost(on)
    };
    let rollback = || {
        match &old {
            Some(o) => {
                let _ = super::webconf::publish_named("nginx", site::DEFAULT_VHOST_FILE, o);
            }
            None => {
                let _ = super::webconf::purge_named("nginx", site::DEFAULT_VHOST_FILE);
            }
        }
        if injected {
            super::webconf::restore_include(conf);
        }
    };
    if let Err(e) = super::webconf::publish_named("nginx", site::DEFAULT_VHOST_FILE, &new) {
        rollback();
        return Err(format!("写入状态页配置失败: {e}"));
    }
    if let Err(e) = site::nginx_test(bin) {
        rollback();
        return Err(format!("nginx -t 校验未通过，已回滚状态页配置：\n{e}"));
    }
    let (reloaded, reason) = if site::nginx_running() {
        match site::reload_nginx(bin) {
            Ok(()) => (true, String::new()),
            Err(e) => (false, e),
        }
    } else {
        (
            false,
            "nginx 未运行，配置已生效，将在启动时加载".to_string(),
        )
    };
    Ok((purged_legacy, reloaded, reason))
}

/// 旧版「独立端口」状态页（`zap-stub.conf`）迁到默认站点：best-effort，
/// 同一进程只尝试一次（面板每 5 秒查一次状态页，没必要反复试）。
fn migrate_legacy_stub(conf: &Path, bin: &Path) {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    if !legacy_stub_path().exists() {
        return;
    }
    if let Err(e) = apply_stub(conf, bin, true) {
        tracing::warn!("旧版状态页配置迁移失败（保留原配置继续采集）: {e}");
    }
}

/// nginx.control：start / stop / restart / reload。
pub async fn control(action: &str) -> Response {
    let action = action.to_string();
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        if !matches!(action.as_str(), "start" | "stop" | "restart" | "reload") {
            return Err(format!("不支持的 Nginx 操作: {action}"));
        }
        let Some((_conf, bin)) = probe() else {
            return Err("Nginx 未安装".to_string());
        };

        // 优先走服务管理器
        let sysd = super::svc::exists("nginx");
        let method = if sysd {
            super::svc::act(&action, "nginx").map_err(|e| format!("nginx {action} 失败：{e}"))?;
            // restart 后稍等稳定
            if action == "restart" {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            // method 值沿用历史命名（面板依赖）
            "systemd".to_string()
        } else {
            // 二进制信号方式
            match action.as_str() {
                "reload" => {
                    if !site::nginx_running() {
                        return Err("Nginx 未运行，无需重载".to_string());
                    }
                    site::reload_nginx(&bin)?;
                }
                "stop" => {
                    if site::nginx_running() {
                        let o = root_cmd(super::platform::SHELL)
                            .args(["-c"])
                            .arg(format!("'{}' -s quit 2>&1", quote_bin(&bin)))
                            .output()
                            .map_err(|e| format!("执行 nginx -s quit 失败: {e}"))?;
                        if !o.status.success() {
                            return Err(format!(
                                "nginx -s quit 失败：{}",
                                output_err(&o, "未知错误")
                            ));
                        }
                        settle_running(false, 10);
                    }
                }
                "start" | "restart" => {
                    if action == "restart" && site::nginx_running() {
                        let _ = root_cmd(super::platform::SHELL)
                            .args(["-c"])
                            .arg(format!("'{}' -s quit 2>&1", quote_bin(&bin)))
                            .output();
                        settle_running(false, 10);
                    }
                    if !site::nginx_running() {
                        let o = root_cmd(super::platform::SHELL)
                            .args(["-c"])
                            .arg(format!("'{}' 2>&1", quote_bin(&bin)))
                            .output()
                            .map_err(|e| format!("启动 nginx 失败: {e}"))?;
                        if !o.status.success() {
                            return Err(format!("启动 nginx 失败：{}", output_err(&o, "未知错误")));
                        }
                        settle_running(true, 10);
                    }
                }
                _ => unreachable!(),
            }
            "binary".to_string()
        };

        let state = if site::nginx_running() {
            "running"
        } else {
            "stopped"
        };
        Ok(Response::ok(
            "ok",
            Some(json!({
                "action": action,
                "method": method,
                "state": state,
            })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_line_parse_is_robust() {
        // nginx_version 依赖系统命令，这里只验证文本取首行逻辑不 panic
        let s = "nginx version: nginx/1.27.3\nbuilt by gcc 12.2.0\n";
        let first = s.lines().next().unwrap_or("").trim().to_string();
        assert!(first.contains("nginx/1.27.3"));
    }
}
