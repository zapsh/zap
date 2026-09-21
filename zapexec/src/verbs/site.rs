//! 站点 Nginx vhost 同步（root 执行）。
//!
//! 契约（与 appstore 安装的 Nginx 应用配合）：
//! - vhost 文件写入 `<nginx prefix>/conf/sites-enabled/zap-site-{id}.conf`
//!   （目录由 nginx.conf 中的 `include sites-enabled/*.conf` / `conf.d/*.conf` 自动探测）
//! - 站点文档根：`{home_dir}/www/{sanitize(name)}-{id}/`（归属用户家目录下），
//!   首次同步自动创建并写占位 index.html
//! - 配置写入后先执行 `nginx -t` 校验，失败即回滚删除文件，绝不带着坏配置 reload
//! - PHP 联动：`php_socket` 形如 `unix:/path` 或 `host:port`；为 None 时不生成 PHP location
//!
//! 安全边界：文件名由 site_id 决定；名称仅用于注释与目录名（sanitize 后）；
//! 不执行任何来自站点输入的命令，只做文件渲染 + 白名单 nginx 校验/reload。

use std::path::{Path, PathBuf};

use serde_json::json;
use zap_proto::{LocationSpec, Response, UpstreamSpec};

use super::root_cmd;

pub(super) fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

/// ACME HTTP-01 验证根目录（面板自管，与维护页同级的 `_zap/` 下）：
/// `{ZAP_PATH}/data/www/_zap/acme`，真正的挑战文件位于其下的
/// `.well-known/acme-challenge/{token}`。
///
/// 由 zapd（`acme.http_write` / `acme.http_clear` 动词）写入，本模块负责在 vhost
/// 里渲染对应的 `location`，两者必须始终使用同一路径。
pub(super) fn acme_webroot() -> PathBuf {
    zap_path().join("data/www/_zap/acme")
}

/// 渲染进每个 vhost 的 ACME HTTP-01 location 片段。
///
/// `^~` 前缀匹配优先于用户自定义 location 与反代正则路径，因此反代站点同样能被验证；
/// 面板（zapd）只负责往验证根里增删 token 文件，无需重载 nginx。
fn render_acme_location() -> String {
    let root = acme_webroot();
    format!(
        "\n    # ACME HTTP-01 域名验证（面板统一托管，请勿手工修改）\n\
         \x20   location ^~ /.well-known/acme-challenge/ {{\n\
         \x20       alias {root}/.well-known/acme-challenge/;\n\
         \x20       default_type \"text/plain\";\n\
         \x20   }}\n",
        root = root.display()
    )
}

// ── Nginx 探测 ───────────────────────────────────────────────

/// 查找已部署 Nginx 的主配置 conf/nginx.conf：
/// 1) 环境变量 `ZAP_NGINX_PREFIX/conf/nginx.conf`（若手工部署可指定）
/// 2) 软件安装根（默认 /usr/local/apps，`ZAP_APPS_DIR` 可覆盖）下递归寻找
///    含 `include`（sites-enabled/conf.d）的运行时配置
pub(super) fn find_nginx_conf_file() -> Option<PathBuf> {
    if let Ok(prefix) = std::env::var("ZAP_NGINX_PREFIX") {
        let p = PathBuf::from(prefix).join("conf/nginx.conf");
        if p.is_file() {
            return Some(p);
        }
    }
    let mut cands = Vec::new();
    collect_nginx_confs(&super::install_root(), 0, &mut cands);
    // 优先选择带 sites-enabled include 的运行时配置
    cands.sort_by_key(|p| !runtime_nginx_hint(p));
    cands.into_iter().find(|p| runtime_nginx_hint(p))
}

fn collect_nginx_confs(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let conf = p.join("conf").join("nginx.conf");
        if conf.is_file() {
            out.push(conf);
        } else {
            collect_nginx_confs(&p, depth + 1, out);
        }
    }
}

fn runtime_nginx_hint(conf: &Path) -> bool {
    std::fs::read_to_string(conf)
        .map(|s| {
            s.contains("include")
                && (s.contains("sites-enabled") || s.contains("conf.d") || s.contains("vhost"))
        })
        .unwrap_or(false)
}

/// 解析 nginx.conf 里 include 的站点配置目录（相对 conf 目录）。
/// 优先 sites-enabled，其次 conf.d / vhost。
pub(super) fn vhosts_dir(nginx_conf: &Path) -> Result<PathBuf, String> {
    let conf_dir = nginx_conf
        .parent()
        .ok_or_else(|| "nginx.conf 父目录无效".to_string())?;
    let text =
        std::fs::read_to_string(nginx_conf).map_err(|e| format!("读取 nginx.conf 失败: {e}"))?;
    let mut targets: Vec<&str> = text
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            let rest = t.strip_prefix("include")?.trim();
            let rest = rest.trim_end_matches(';').trim();
            if rest.is_empty()
                || rest.contains("mime.types")
                || rest.contains("fastcgi")
                || rest.starts_with("http_")
            {
                return None;
            }
            Some(rest)
        })
        .collect();
    targets.dedup();
    let chosen = targets
        .iter()
        .find(|t| t.contains("sites-enabled"))
        .or_else(|| {
            targets
                .iter()
                .find(|t| t.contains("conf.d") || t.contains("vhost"))
        })
        .or_else(|| targets.first());
    match chosen {
        Some(raw) => {
            // raw 形如 sites-enabled/*.conf 或 conf.d/*.conf；目录取第一段
            let name = raw
                .split(['/', ' '])
                .next()
                .unwrap_or(raw)
                .trim()
                .trim_end_matches('*');
            if name.is_empty() {
                return Err("无法从 nginx.conf include 中识别站点配置目录".to_string());
            }
            Ok(conf_dir.join(name))
        }
        None => Err(
            "nginx.conf 未 include 站点配置目录（sites-enabled / conf.d），请先在 Nginx 主配置中添加 include".to_string(),
        ),
    }
}

pub(super) fn nginx_bin(nginx_conf: &Path) -> PathBuf {
    let sbin = nginx_conf
        .parent()
        .and_then(|c| c.parent())
        .map(|p| p.join("sbin").join("nginx"))
        .filter(|p| p.is_file());
    sbin.unwrap_or_else(|| PathBuf::from("nginx"))
}

pub(super) fn nginx_running() -> bool {
    for pid_file in [PathBuf::from("/var/run/nginx.pid")] {
        if let Ok(content) = std::fs::read_to_string(&pid_file)
            && let Ok(pid) = content.trim().parse::<i32>()
            && pid > 0
            && root_cmd("kill")
                .args(["-0", &pid.to_string()])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        {
            return true;
        }
    }
    root_cmd("pgrep")
        .args(["-x", "nginx"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 提取 nginx 命令的 stderr（截断）
fn output_err(o: &std::process::Output, fallback: &str) -> String {
    let text = String::from_utf8_lossy(&o.stderr).trim().to_string();
    let text = if text.is_empty() {
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    } else {
        text
    };
    let text = if text.is_empty() {
        fallback.to_string()
    } else {
        text
    };
    let mut out = text.lines().take(12).collect::<Vec<_>>().join("\n");
    if out.len() > 1500 {
        out = out.chars().take(1500).collect();
    }
    out
}

// ── 文档根 ───────────────────────────────────────────────────

/// 站点骨架模板：`{ZAP_PATH}/data/www/skel/index.html`，为新站点的默认首页模板。
/// 与 IP 默认页 / 维护页（同为 {ZAP_PATH}/data/www/_zap/）放在一起，便于统一维护。
/// 运维可直接修改该模板（支持 __SITE_NAME__ / __SITE_ID__ / __SITE_DOMAINS__ /
/// __SITE_ROOT__ / __CREATED_AT__ 占位符），下次建站即生效。
fn skel_file() -> PathBuf {
    zap_path().join("data/www/skel/index.html")
}

/// skel 模板缺失时的兜底页（保证离线/精简部署也能建站成功）
fn fallback_index_html() -> String {
    "<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n\
     <title>站点已创建</title>\n</head>\n<body>\n<h1>站点已创建</h1>\n\
     <p>此页面由 Zap 面板自动生成，将站点文件放入本目录即可。</p>\n</body>\n</html>\n"
        .to_string()
}

/// 用 skel 模板渲染站点默认首页（模板不存在时回退内置页面）
fn render_index_html(site_id: i64, name: &str, domains: &[String], root: &Path) -> String {
    let tpl = match std::fs::read_to_string(skel_file()) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return fallback_index_html(),
    };
    let created_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    apply_placeholders(&tpl, site_id, name, domains, root, &created_at)
}

/// 纯函数：替换 skel 模板中的占位符（单测覆盖）
fn apply_placeholders(
    tpl: &str,
    site_id: i64,
    name: &str,
    domains: &[String],
    root: &Path,
    created_at: &str,
) -> String {
    let domains_text = if domains.is_empty() {
        "未绑定域名".to_string()
    } else {
        domains.join("、")
    };
    tpl.replace("__SITE_NAME__", name)
        .replace("__SITE_ID__", &site_id.to_string())
        .replace("__SITE_DOMAINS__", &domains_text)
        .replace("__SITE_ROOT__", &root.to_string_lossy())
        .replace("__CREATED_AT__", created_at)
}

fn ensure_web_root(
    root: &Path,
    site_id: i64,
    name: &str,
    domains: &[String],
) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| format!("{e}"))?;
    let index = root.join("index.html");
    if !index.exists() {
        std::fs::write(&index, render_index_html(site_id, name, domains, root))
            .map_err(|e| format!("{e}"))?;
    }
    Ok(())
}

// ── 渲染（纯函数，单测覆盖）──────────────────────────────────

/// 站点类型白名单
const SITE_TYPES: [&str; 3] = ["php", "static", "proxy"];
/// 伪静态预设 key（custom = 使用自定义规则文本）
const PSEUDO_PRESETS: [&str; 6] = [
    "none",
    "thinkphp",
    "laravel",
    "wordpress",
    "codeigniter",
    "custom",
];

fn norm_site_type(t: &str) -> &'static str {
    match t.trim().to_lowercase().as_str() {
        "static" => "static",
        "proxy" => "proxy",
        _ => "php",
    }
}

/// 伪静态规则 → `location /` 内的指令（None = 走默认 try_files）。
/// kind 只匹配白名单预设；custom 使用面板提交的多行指令原文。
fn pseudo_location_body(kind: &str, custom: &str) -> Option<String> {
    match kind.trim().to_lowercase().as_str() {
        "thinkphp" => Some(
            "if (!-e $request_filename) {\n    rewrite ^(.*)$ /index.php?s=$1 last;\n}".to_string(),
        ),
        "codeigniter" => Some(
            "if (!-e $request_filename) {\n    rewrite ^(.*)$ /index.php/$1 last;\n}".to_string(),
        ),
        "laravel" | "wordpress" | "drupal" | "typecho" => {
            Some("try_files $uri $uri/ /index.php?$query_string;".to_string())
        }
        "custom" => {
            let c = custom.trim();
            (!c.is_empty()).then(|| c.to_string())
        }
        _ => None,
    }
}

/// 渲染自定义 location 的指令体（含 8 空格缩进、结尾换行）；
/// 非法/空项返回空串（入参在同步前已整体校验，这里只是渲染兜底）。
fn render_location_body(l: &LocationSpec) -> String {
    let mut b = String::new();
    match l.kind.trim().to_lowercase().as_str() {
        "proxy" => {
            let target = l.target.trim();
            b.push_str("        proxy_pass ");
            b.push_str(target);
            b.push_str(";\n");
            b.push_str(
                "        proxy_set_header Host $host;\n\
                   proxy_set_header X-Real-IP $remote_addr;\n\
                   proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n\
                   proxy_set_header X-Forwarded-Proto $scheme;\n",
            );
            if l.ws {
                b.push_str(
                    "        proxy_http_version 1.1;\n\
                       proxy_set_header Upgrade $http_upgrade;\n\
                       proxy_set_header Connection \"upgrade\";\n",
                );
            }
            // 追加自定义请求头（同名可覆盖默认头；nginx 后者覆盖前者）
            for h in &l.headers {
                let k = h.key.trim();
                if k.is_empty() {
                    continue;
                }
                b.push_str(&format!(
                    "        proxy_set_header {k} {};\n",
                    h.value.trim()
                ));
            }
            // 超时（秒）
            if l.conn_timeout > 0 {
                b.push_str(&format!(
                    "        proxy_connect_timeout {}s;\n",
                    l.conn_timeout
                ));
            }
            if l.read_timeout > 0 {
                b.push_str(&format!(
                    "        proxy_read_timeout {}s;\n",
                    l.read_timeout
                ));
            }
            if l.send_timeout > 0 {
                b.push_str(&format!(
                    "        proxy_send_timeout {}s;\n",
                    l.send_timeout
                ));
            }
            // 跳转策略
            if !l.proxy_redirect.trim().is_empty() {
                b.push_str(&format!(
                    "        proxy_redirect {};\n",
                    l.proxy_redirect.trim()
                ));
            }
            // 关闭缓冲（SSE / 流式输出）
            if l.no_buffering {
                b.push_str("        proxy_buffering off;\n");
            }
            // 反代缓存（共享缓存区由执行端在站点发布前幂等创建）
            if !l.cache.trim().is_empty() {
                b.push_str(&format!("        proxy_cache {};\n", l.cache.trim()));
                if l.cache_valid.trim().is_empty() {
                    b.push_str("        proxy_cache_valid 200 1m;\n");
                } else {
                    b.push_str(&format!(
                        "        proxy_cache_valid {};\n",
                        l.cache_valid.trim()
                    ));
                }
            }
        }
        "redirect" => {
            let code = if l.code == 0 { 301 } else { l.code };
            b.push_str(&format!("        return {code} {};\n", l.target.trim()));
        }
        "deny" => {
            let code = if l.code == 0 { 403 } else { l.code };
            b.push_str(&format!("        return {code};\n"));
        }
        "alias" => {
            b.push_str(&format!("        alias {};\n", l.target.trim()));
        }
        // raw：高级自由指令体（每行原样输出，同步前已校验，禁止 include / 块嵌套）
        "raw" => {
            for line in l.raw.lines() {
                b.push_str("        ");
                b.push_str(line);
                b.push('\n');
            }
        }
        _ => {}
    }
    b
}

/// TLS 高级设置（仅绑定证书时生效；值已在入参层做过字符白名单过滤）
struct SslTlsCfg {
    /// ssl_protocols 值；空 = 回退 TLSv1.2 TLSv1.3
    protocols: String,
    /// ssl_ciphers 值；空 = 不输出指令（跟随 nginx 内置默认）
    ciphers: String,
    /// 服务端优先选择密码套件（ssl_prefer_server_ciphers on）
    prefer_server_ciphers: bool,
    /// 启用 HTTP/2
    http2: bool,
    /// true = nginx ≥ 1.25.1（server 内写 `http2 on;`）；
    /// false = 老版本（listen 443 ssl http2 内嵌）
    http2_on_syntax: bool,
}

/// ssl_protocols 值白名单过滤：仅保留 TLSv1.1 / TLSv1.2 / TLSv1.3（按出现顺序、去重）。
/// 返回空串表示没有合法项（调用方应回退默认 TLSv1.2 TLSv1.3）。
fn sanitize_protocols(raw: &str) -> String {
    const ALLOWED: [&str; 3] = ["TLSv1.1", "TLSv1.2", "TLSv1.3"];
    let mut seen: Vec<&str> = Vec::new();
    for tok in raw.split_whitespace() {
        if ALLOWED.contains(&tok) && !seen.contains(&tok) {
            seen.push(tok);
        }
    }
    seen.join(" ")
}

/// ssl_ciphers 值过滤：仅保留 nginx 套件字符（字母/数字/:!+_- 与空格分隔），
/// 剔除任何可用于截断注入指令的字符（; # \n 引号 花括号 斜杠等）。
fn sanitize_ciphers(raw: &str) -> String {
    raw.chars()
        .filter(|c| {
            c.is_ascii_alphanumeric() || matches!(c, ':' | '!' | '+' | '-' | '_' | '.' | ' ')
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// nginx ≥ 1.25.1 起 `listen ... http2` 参数被移除，改为 server 内 `http2 on;`。
/// 探测版本以决定指令写法；版本未知时按新语法（面板分发的 Nginx 均为新版本）。
fn nginx_http2_on_syntax(bin: &std::path::Path) -> bool {
    let Ok(out) = std::process::Command::new(bin).arg("-v").output() else {
        return true;
    };
    let raw = if out.stderr.is_empty() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let Some(idx) = raw.rfind('/') else {
        return true;
    };
    let ver: String = raw[idx + 1..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let nums: Vec<u32> = ver.split('.').filter_map(|p| p.parse().ok()).collect();
    match nums.as_slice() {
        [maj, min, ..] => (*maj, *min) >= (1, 25),
        _ => true,
    }
}

/// 完整 vhost 渲染（纯函数）：
/// - site_type：php（默认，PHP/PHP+静态）/ static / proxy（反向代理，忽略 root/PHP）
/// - 伪静态预设只影响默认 `location /`（php/static 类型）
/// - upstreams 渲染到 server 块之前；locations 按序渲染进 server（nginx 最长前缀匹配覆盖默认规则）
///
/// 入参全部为借用/复制字段，聚合为 [`VhostRenderSpec`] 传递（避免 15 个平铺参数）。
#[derive(Clone, Copy)]
struct VhostRenderSpec<'a> {
    site_id: i64,
    name: &'a str,
    domains: &'a [String],
    /// 文档根（proxy 类型为空串，不渲染 root 指令）
    root: &'a str,
    php_socket: Option<&'a str>,
    access_log: Option<&'a str>,
    error_log: Option<&'a str>,
    site_type: &'a str,
    pseudo_static: &'a str,
    pseudo_custom: &'a str,
    upstreams: &'a [UpstreamSpec],
    locations: &'a [LocationSpec],
    /// (证书 fullchain 路径, 私钥路径)；None = 不启用 HTTPS
    ssl_files: Option<(&'a str, &'a str)>,
    force_https: bool,
    ssl_tls: Option<&'a SslTlsCfg>,
    /// 共享主机 IPv4（面板基础设置「默认 IPv4」）；空 = 通配 `listen 80`
    listen_ipv4: &'a str,
    /// 共享主机 IPv6（面板基础设置「默认 IPv6」）；空 = 通配 `listen [::]:80`
    listen_ipv6: &'a str,
}

/// 生成 listen 指令：指定了共享 IP 就绑定 `IP:端口`，否则通配。
///
/// `suffix` 为附加参数（如 ` ssl` / ` ssl http2`）。
fn listen_directive(ipv4: &str, ipv6: &str, port: u16, suffix: &str) -> String {
    let v4 = if ipv4.trim().is_empty() {
        format!("    listen {port}{suffix};\n")
    } else {
        format!("    listen {}:{port}{suffix};\n", ipv4.trim())
    };
    let v6 = if ipv6.trim().is_empty() {
        format!("    listen [::]:{port}{suffix};\n")
    } else {
        format!("    listen [{}]:{port}{suffix};\n", ipv6.trim())
    };
    format!("{v4}{v6}")
}

fn render_vhost_full(a: VhostRenderSpec<'_>) -> String {
    let VhostRenderSpec {
        site_id,
        name,
        domains,
        root,
        php_socket,
        access_log,
        error_log,
        site_type,
        pseudo_static,
        pseudo_custom,
        upstreams,
        locations,
        ssl_files,
        force_https,
        ssl_tls,
        listen_ipv4,
        listen_ipv6,
    } = a;
    // 监听地址：指定共享 IP 时绑定 `IP:端口`，否则沿用通配监听
    let listen_80 = listen_directive(listen_ipv4, listen_ipv6, 80, "");
    let listen_443 = listen_directive(listen_ipv4, listen_ipv6, 443, " ssl");
    let s_type = norm_site_type(site_type);
    let comment = name.chars().filter(|c| !c.is_control()).collect::<String>();
    let server_name = {
        let parts: Vec<&str> = domains
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() {
            "_".to_string()
        } else {
            parts.join(" ")
        }
    };
    let mut out = String::new();
    out.push_str(&format!(
        "# Generated by Zap Panel — site \"{comment}\" (id={site_id}, type={s_type}) — DO NOT EDIT\n"
    ));
    // upstream 块（server 之前）
    for u in upstreams {
        let uname = u.name.trim();
        if uname.is_empty() {
            continue;
        }
        out.push_str(&format!("upstream {uname} {{\n"));
        // 负载均衡策略（默认轮询无需输出）
        match u.balance.trim() {
            "least_conn" => out.push_str("    least_conn;\n"),
            "ip_hash" => out.push_str("    ip_hash;\n"),
            _ => {}
        }
        for s in &u.servers_ext {
            let addr = s.addr.trim();
            if addr.is_empty() {
                continue;
            }
            let mut line = format!("    server {addr}");
            if s.weight > 0 {
                line.push_str(&format!(" weight={}", s.weight));
            }
            if s.max_fails > 0 {
                line.push_str(&format!(" max_fails={}", s.max_fails));
            }
            if s.fail_timeout > 0 {
                line.push_str(&format!(" fail_timeout={}s", s.fail_timeout));
            }
            if s.backup {
                line.push_str(" backup");
            }
            if s.down {
                line.push_str(" down");
            }
            out.push_str(&line);
            out.push_str(";\n");
        }
        out.push_str("}\n\n");
    }
    // 端口内容体（root / 日志 / 伪静态 / PHP / location），80 与 443 共用一份
    let mut core = String::new();
    // ACME HTTP-01：所有站点一律带上验证路径（含反代站点），
    // 面板随时可签发证书，不必先建 CA 无关的临时站点
    core.push_str(&render_acme_location());
    if s_type != "proxy" {
        core.push_str(&format!("    root {root};\n"));
        if let Some(p) = access_log {
            core.push_str(&format!("    access_log {p};\n"));
        }
        if let Some(p) = error_log {
            core.push_str(&format!("    error_log {p};\n"));
        }
        if php_socket.is_some() {
            core.push_str("    index index.php index.html;\n");
        } else {
            core.push_str("    index index.html;\n");
        }
        // 默认 location /：伪静态预设覆盖默认 try_files
        core.push_str("\n    location / {\n");
        match pseudo_location_body(pseudo_static, pseudo_custom) {
            Some(body) => {
                for line in body.lines() {
                    core.push_str("        ");
                    core.push_str(line);
                    core.push('\n');
                }
            }
            None => core.push_str("        try_files $uri $uri/ =404;\n"),
        }
        core.push_str("    }\n");
        if let Some(sock) = php_socket {
            core.push_str("\n    # PHP 实例联动\n");
            core.push_str("    location ~ \\.php$ {\n");
            core.push_str(&format!("        fastcgi_pass {sock};\n"));
            core.push_str("        fastcgi_index index.php;\n        include fastcgi_params;\n");
            core.push_str(
                "        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n    }\n",
            );
        }
    } else if let Some(p) = access_log {
        core.push_str(&format!("    access_log {p};\n"));
        if let Some(pl) = error_log {
            core.push_str(&format!("    error_log {pl};\n"));
        }
    }
    // 自定义 locations（proxy 必须提供，php/static 用于扩展覆盖）
    for l in locations {
        let path = l.path.trim();
        if !path.starts_with('/') {
            continue;
        }
        let body = render_location_body(l);
        if body.is_empty() {
            continue;
        }
        core.push_str(&format!("\n    location {path} {{\n{body}    }}\n"));
    }

    // SSL/TLS：绑定证书后才监听 443；允许 HTTP 跳转时 80 只保留 301
    let http2_enabled = ssl_files.is_some() && ssl_tls.map(|c| c.http2).unwrap_or(false);
    // nginx < 1.25.1 只能把 http2 内嵌到 listen 参数；新版本用独立 `http2 on;` 指令
    let listen_443 = if http2_enabled && ssl_tls.is_some_and(|c| !c.http2_on_syntax) {
        listen_directive(listen_ipv4, listen_ipv6, 443, " ssl http2")
    } else {
        listen_443
    };
    let ssl_directives = ssl_files.map(|(cert, key)| {
        let mut s = format!("    ssl_certificate {cert};\n    ssl_certificate_key {key};\n");
        // 协议：显式设置取白名单交集；空/非法则回退面板默认 TLSv1.2 TLSv1.3
        let protocols = ssl_tls
            .map(|c| sanitize_protocols(&c.protocols))
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "TLSv1.2 TLSv1.3".to_string());
        s.push_str(&format!("    ssl_protocols {protocols};\n"));
        if let Some(c) = ssl_tls {
            let ciphers = sanitize_ciphers(&c.ciphers);
            if !ciphers.is_empty() {
                s.push_str(&format!("    ssl_ciphers {ciphers};\n"));
            }
            if c.prefer_server_ciphers {
                s.push_str("    ssl_prefer_server_ciphers on;\n");
            }
            if c.http2 && c.http2_on_syntax {
                s.push_str("    http2 on;\n");
            }
        }
        s
    });
    match (&ssl_directives, force_https) {
        (Some(sd), true) => {
            out.push_str("server {\n");
            out.push_str(&listen_80);
            out.push_str(&format!("    server_name {server_name};\n"));
            out.push_str("    return 301 https://$host$request_uri;\n");
            out.push_str("}\n\n");
            out.push_str("server {\n");
            out.push_str(&listen_443);
            out.push_str(&format!("    server_name {server_name};\n"));
            out.push_str(sd);
            out.push_str(&core);
            out.push_str("}\n");
        }
        _ => {
            out.push_str("server {\n");
            out.push_str(&listen_80);
            out.push_str(&format!("    server_name {server_name};\n"));
            out.push_str(&core);
            out.push_str("}\n");
            if let Some(sd) = &ssl_directives {
                out.push_str("server {\n");
                out.push_str(&listen_443);
                out.push_str(&format!("    server_name {server_name};\n"));
                out.push_str(sd);
                out.push_str(&core);
                out.push_str("}\n");
            }
        }
    }
    out
}

/// 面板托管默认站点文件名（常驻 sites-available / sites-enabled）。
pub(super) const DEFAULT_VHOST_FILE: &str = "00-default.conf";

/// 状态页 URL 路径（stub_status，挂在默认站点 server 上）。
pub(super) const STUB_PATH: &str = "/nginx_status";

/// 状态页 location：`stub=true` 时注入默认站点的 80 server，仅本机可访问（外部 403）。
///
/// 关键点：**不能**在 server 级写 `return 444;` —— 它在 location 匹配之前就生效，
/// 会把状态页一起掐断（实测返回空响应）。所以 444 下沉到 `location /`，
/// 状态页用精确匹配 `location = /nginx_status` 抢在前面。
pub(super) fn render_stub_location(stub: bool) -> String {
    if !stub {
        return String::new();
    }
    format!(
        "    # zap: 面板状态页 stub_status（本机采集）\n\
         \x20   location = {STUB_PATH} {{\n\
         \x20       stub_status on;\n\
         \x20       allow 127.0.0.1;\n\
         \x20       deny all;\n\
         \x20       access_log off;\n\
         \x20   }}\n"
    )
}

/// 默认站点「关闭 IP 访问」形态：未匹配 / 已停止的域名与 IP 直连一律断开连接（444），
/// 443 拒绝握手——避免被同机的其它站点按 default_server 规则"接走"造成串站。
pub(super) fn render_default_vhost(stub: bool) -> String {
    format!(
        concat!(
            "# Generated by Zap Panel — 默认站点（未匹配 / 已停止的域名）— DO NOT EDIT\n",
            "server {{\n",
            "    listen 80 default_server;\n",
            "    listen [::]:80 default_server;\n",
            "    server_name _;\n",
            "{stub}",
            "    location / {{\n",
            "        return 444;\n",
            "    }}\n",
            "}}\n",
            // 443 默认 server：拒绝握手，未匹配域名不落到任意站点的 443 上（防串站）
            "server {{\n",
            "    listen 443 ssl default_server;\n",
            "    listen [::]:443 ssl default_server;\n",
            "    server_name _;\n",
            "    ssl_reject_handshake on;\n",
            "}}\n"
        ),
        stub = render_stub_location(stub),
    )
}

// ── 站点 SSL 证书文件（{ZAP_PATH}/data/ssl/site_{id}/）────────────────

fn site_ssl_dir(site_id: i64) -> PathBuf {
    zap_path().join("data/ssl").join(format!("site_{site_id}"))
}

/// 落盘站点 SSL 证书（fullchain / key）。目录 0755、文件 0644，
/// 保证任意运行身份的 nginx worker 均可读取。返回 (fullchain, key) 绝对路径。
fn write_site_ssl_files(
    site_id: i64,
    fullchain: &str,
    key: &str,
) -> Result<(String, String), String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = site_ssl_dir(site_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建站点 SSL 目录失败: {e}"))?;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
    let full = dir.join("fullchain.pem");
    let key_f = dir.join("key.pem");
    std::fs::write(&full, fullchain).map_err(|e| format!("写入证书文件失败: {e}"))?;
    std::fs::write(&key_f, key).map_err(|e| format!("写入证书私钥文件失败: {e}"))?;
    let _ = std::fs::set_permissions(&full, std::fs::Permissions::from_mode(0o644));
    let _ = std::fs::set_permissions(&key_f, std::fs::Permissions::from_mode(0o644));
    Ok((
        full.to_string_lossy().to_string(),
        key_f.to_string_lossy().to_string(),
    ))
}

/// 移除站点 SSL 证书文件（解绑 / 站点删除时调用）
fn remove_site_ssl_files(site_id: i64) {
    let dir = site_ssl_dir(site_id);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 维护页目录：`{ZAP_PATH}/data/www/_zap`（面板自管，不占用站点目录）
fn maintenance_dir() -> PathBuf {
    zap_path().join("data/www/_zap")
}

/// 维护页：不存在时生成一份默认页面（管理员可直接改这个文件定制内容）
fn ensure_maintenance_page() -> Result<PathBuf, String> {
    let dir = maintenance_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建维护页目录失败: {e}"))?;
    let file = dir.join("maintenance.html");
    if !file.exists() {
        let html = concat!(
            "<!DOCTYPE html>\n",
            "<html lang=\"zh-CN\">\n",
            "<head><meta charset=\"utf-8\">",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
            "<title>站点维护中</title>\n",
            "<style>body{font-family:system-ui,-apple-system,\"PingFang SC\",\"Microsoft YaHei\",sans-serif;",
            "display:flex;align-items:center;justify-content:center;height:100vh;margin:0;",
            "background:#f5f7fa;color:#303133}",
            ".box{text-align:center;padding:32px}",
            "h1{font-size:20px;margin:0 0 8px}p{color:#909399;font-size:14px;margin:0}</style>\n",
            "</head>\n<body><div class=\"box\">",
            "<h1>站点维护中</h1><p>我们正在维护该站点，请稍后再访问。</p>",
            "</div></body></html>\n"
        );
        std::fs::write(&file, html).map_err(|e| format!("写入维护页失败: {e}"))?;
    }
    // nginx worker 以 www 组读取，页面需可读
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644));
    Ok(dir)
}

/// 维护态 vhost：503 + 维护页（站点域名仍匹配，但不再走业务目录/PHP）
fn render_maintenance_vhost(site_id: i64, name: &str, domains: &[String], maint: &str) -> String {
    let comment = name.chars().filter(|c| !c.is_control()).collect::<String>();
    let server_name = {
        let parts: Vec<&str> = domains
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() {
            "_".to_string()
        } else {
            parts.join(" ")
        }
    };
    format!(
        "# Generated by Zap Panel — site \"{comment}\" (id={site_id}) 维护中 — DO NOT EDIT\n\
         server {{\n\
         \x20   listen 80;\n\
         \x20   listen [::]:80;\n\
         \x20   server_name {server_name};\n\
         \x20   root {maint};\n\
         \x20   error_page 503 /maintenance.html;\n\
         \n\
         \x20   location = /maintenance.html {{\n\
         \x20       root {maint};\n\
         \x20       default_type text/html;\n\
         \x20   }}\n\
         \n\
         \x20   location / {{\n\
         \x20       return 503;\n\
         \x20   }}\n\
         }}\n"
    )
}

/// 停用官方安装模板自带的默认站点（conf/sites-enabled/default.conf 等），
/// 避免与面板默认站点争夺 default_server 导致 nginx -t 报 duplicate default server。
/// 改名保留为 `.zap-disabled`（不匹配 *.conf，不再被 include 加载），幂等。
pub(super) fn deactivate_official_default(conf_file: &Path) {
    if let Ok(dir) = vhosts_dir(conf_file) {
        let official = dir.join("default.conf");
        let disabled = dir.join("default.conf.zap-disabled");
        if official.exists() && !disabled.exists() {
            let _ = std::fs::rename(&official, &disabled);
        }
    }
}

/// 确保面板默认站点已发布（best-effort）：仅在 00-default.conf 不存在时创建
/// （默认关闭 IP 访问，直接 444）。文件已存在说明面板发布过 / 管理员通过
/// 「默认站点（IP 访问）」开关设置过内容，一律不覆盖，避免站点发布冲掉在线配置。
/// 与主配置已有的 default_server 冲突时自动回滚，只告警不阻断站点本身的发布。
pub(super) fn ensure_default_vhost(conf_file: &Path, bin: &Path) {
    deactivate_official_default(conf_file);
    let avail = super::webconf::available_dir("nginx").join(DEFAULT_VHOST_FILE);
    if avail.exists() {
        return;
    }
    let edir = super::webconf::enabled_dir("nginx");
    let injected = match super::webconf::ensure_include(conf_file, &edir, "nginx") {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("默认站点未发布（无法注入 include）: {e}");
            return;
        }
    };
    if let Err(e) =
        super::webconf::publish_named("nginx", DEFAULT_VHOST_FILE, &render_default_vhost(false))
    {
        if injected {
            super::webconf::restore_include(conf_file);
        }
        tracing::warn!("默认站点发布失败: {e}");
        return;
    }
    if let Err(e) = nginx_test(bin) {
        let _ = super::webconf::purge_named("nginx", DEFAULT_VHOST_FILE);
        if injected {
            super::webconf::restore_include(conf_file);
        }
        tracing::warn!("默认站点导致 nginx -t 失败，已回滚（不影响站点本身）: {e}");
    }
}

// ── 同步 / 移除入口 ──────────────────────────────────────────

/// 目录参数校验：必须是绝对路径且不含 `..`（zapd 传入的家目录/站点路径来自用户表，双保险）
fn dir_arg_ok(p: &str) -> bool {
    p.starts_with('/') && !p.split('/').any(|s| s == "..")
}

/// `p` 是否位于 `base` 目录内（逐段组件比较，不触碰文件系统；拒绝绝对路径中的 `..`/前缀混入）
fn path_under(base: &Path, p: &Path) -> bool {
    fn comps(path: &Path) -> Option<Vec<String>> {
        if !path.is_absolute() {
            return None;
        }
        let mut v = Vec::new();
        for c in path.components() {
            match c {
                std::path::Component::Normal(s) => v.push(s.to_string_lossy().to_string()),
                // 绝对路径必然以根目录开头，忽略
                std::path::Component::RootDir => {}
                std::path::Component::CurDir => {}
                // 出现 .. 或跨平台前缀（Windows）一律拒绝
                _ => return None,
            }
        }
        Some(v)
    }
    let (Some(b), Some(p)) = (comps(base), comps(p)) else {
        return false;
    };
    p.len() >= b.len() && b.iter().zip(p.iter()).all(|(x, y)| x == y)
}

/// 校验 vhost 高级配置：站点类型 / 伪静态 / 自定义目录 / upstream / location。
/// 失败返回带原因的 Err，同步方据此中止发布（nginx -t 仅是最后一道保险）。
fn validate_vhost_cfg(
    site_type: &str,
    pseudo_static: &str,
    pseudo_custom: &str,
    web_root_custom: bool,
    root: Option<&Path>,
    upstreams: &[UpstreamSpec],
    locations: &[LocationSpec],
) -> Result<(), String> {
    let raw = site_type.trim().to_lowercase();
    if !SITE_TYPES.contains(&raw.as_str()) {
        return Err(format!(
            "站点类型仅支持 php / static / proxy（收到：{site_type}）"
        ));
    }
    let s_type = norm_site_type(site_type);
    let pseudo = pseudo_static.trim().to_lowercase();
    if !PSEUDO_PRESETS.contains(&pseudo.as_str()) {
        return Err(format!("伪静态预设不支持：{pseudo_static}"));
    }
    if !pseudo_custom.trim().is_empty() && pseudo != "custom" {
        return Err("已填写自定义伪静态规则，但伪静态预设不是 custom".to_string());
    }
    if pseudo == "custom" {
        let c = pseudo_custom;
        if c.contains("server") || c.contains("location ") {
            return Err("自定义伪静态规则中不允许出现 server / location 指令".to_string());
        }
        if c.matches('{').count() != c.matches('}').count() {
            return Err("自定义伪静态规则花括号不配对".to_string());
        }
        if c.lines().any(|l| !l.trim().is_empty() && l.contains('#')) {
            return Err("自定义伪静态规则中不允许使用 # 注释".to_string());
        }
    }
    if web_root_custom {
        let r = root.ok_or_else(|| "自定义站点目录缺少目录参数".to_string())?;
        if !r.is_dir() {
            return Err(format!(
                "自定义站点目录不存在或不是目录：{}（请先在服务器上创建该目录）",
                r.display()
            ));
        }
    }
    // upstream 组：名称唯一合法、负载策略白名单、server 行表单/文本校验
    let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for u in upstreams {
        let n = u.name.trim();
        if n.is_empty()
            || !n
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        {
            return Err(format!(
                "upstream 名称非法：{}（仅允许字母/数字/_/-）",
                u.name
            ));
        }
        if !names.insert(n.to_string()) {
            return Err(format!("upstream 名称重复：{n}"));
        }
        match u.balance.trim() {
            "" | "least_conn" | "ip_hash" => {}
            other => return Err(format!("upstream 负载策略不支持：{other}")),
        }
        let mut cnt = 0;
        if u.servers_ext.len() > 16 {
            return Err(format!("upstream {n} 的 server 数量超过上限（16）"));
        }
        for s in &u.servers_ext {
            let a = s.addr.trim();
            if a.is_empty() {
                continue;
            }
            cnt += 1;
            if a.len() > 200
                || !a.chars().all(|c| {
                    c.is_ascii_alphanumeric()
                        || matches!(c, '.' | ':' | '/' | '_' | '-' | '[' | ']' | '%')
                })
            {
                return Err(format!("upstream {n} 的 server 地址含非法字符：{a}"));
            }
            if s.weight > 1000 || s.max_fails > 100 || s.fail_timeout > 3600 {
                return Err(format!(
                    "upstream {n} 的 server 参数超限（weight ≤ 1000 / max_fails ≤ 100 / fail_timeout ≤ 3600s）"
                ));
            }
        }
        if cnt == 0 {
            return Err(format!("upstream {n} 至少需要一个 server 地址"));
        }
    }
    // 数量上限
    if s_type != "proxy" && locations.len() > 16 {
        return Err("自定义 location 最多 16 个".to_string());
    }
    if upstreams.len() > 8 {
        return Err("upstream 组最多 8 个".to_string());
    }
    // locations
    let mut has_root_loc = false;
    for l in locations {
        let p = l.path.trim();
        if !p.starts_with('/')
            || !p.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '/' | '_' | '.' | '-' | '~' | '%' | '@' | ':' | '=' | '&')
            })
        {
            return Err(format!(
                "location 路径非法：{}（必须以 / 开头，仅字母/数字/常用路径字符）",
                l.path
            ));
        }
        if p == "/" {
            has_root_loc = true;
        }
        let kind = l.kind.trim().to_lowercase();
        match kind.as_str() {
            "proxy" => {
                let t = l.target.trim();
                if t.is_empty()
                    || t.chars()
                        .any(|c| c.is_control() || matches!(c, '{' | '}' | ';' | '#' | '$'))
                {
                    return Err(format!("proxy_pass 目标非法：{}", l.target));
                }
                if let Some(url) = t
                    .strip_prefix("http://")
                    .or_else(|| t.strip_prefix("https://"))
                {
                    if url.trim().is_empty() {
                        return Err("proxy_pass 目标 URL 缺少主机".to_string());
                    }
                } else if t.starts_with("unix:") {
                    return Err(
                        "proxy_pass 暂不支持 unix socket，请填 http(s):// 地址或 upstream 组名"
                            .to_string(),
                    );
                } else if !names.contains(t) {
                    return Err(format!(
                        "proxy_pass 目标 {t} 不是已配置的 upstream 组，也不是 http(s):// 地址"
                    ));
                }
                // 超时上限
                if l.conn_timeout > 86400 || l.read_timeout > 86400 || l.send_timeout > 86400 {
                    return Err("代理超时最大 86400 秒".to_string());
                }
                // 自定义请求头
                for (idx, h) in l.headers.iter().enumerate() {
                    let k = h.key.trim();
                    if k.is_empty()
                        || k.len() > 64
                        || !k
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
                    {
                        return Err(format!(
                            "第 {} 个自定义请求头名称非法（仅字母/数字/_/-）：{}",
                            idx + 1,
                            h.key
                        ));
                    }
                    let v = h.value.trim();
                    if v.is_empty()
                        || v.len() > 500
                        || v.chars()
                            .any(|c| c.is_control() || matches!(c, '{' | '}' | ';' | '#'))
                    {
                        return Err(format!(
                            "第 {} 个自定义请求头（{}）的值非法或超长",
                            idx + 1,
                            k
                        ));
                    }
                }
                // proxy_redirect
                let pr = l.proxy_redirect.trim();
                if !pr.is_empty()
                    && (pr.len() > 400
                        || pr
                            .chars()
                            .any(|c| c.is_control() || matches!(c, '{' | '}' | ';' | '#')))
                {
                    return Err("proxy_redirect 规则非法或超长".to_string());
                }
                // 反代缓存（zone 固定白名单，由执行端幂等创建共享缓存区）
                let cz = l.cache.trim();
                if !cz.is_empty() && cz != "zap_cache" {
                    return Err("缓存区仅支持内置的 zap_cache".to_string());
                }
                if !l.cache_valid.trim().is_empty()
                    && (l.cache_valid.len() > 200
                        || l.cache_valid
                            .chars()
                            .any(|c| c.is_control() || matches!(c, '{' | '}' | ';' | '#')))
                {
                    return Err("proxy_cache_valid 规则非法或超长".to_string());
                }
            }
            "redirect" => {
                if !matches!(l.code, 0 | 301 | 302 | 303 | 307 | 308) {
                    return Err(format!(
                        "redirect 状态码仅支持 301/302/303/307/308（收到 {}）",
                        l.code
                    ));
                }
                let t = l.target.trim();
                if t.is_empty()
                    || t.chars()
                        .any(|c| c.is_control() || matches!(c, '{' | '}' | ';' | '#' | '$'))
                    || !(t.starts_with("http://")
                        || t.starts_with("https://")
                        || t.starts_with('/'))
                {
                    return Err(format!(
                        "redirect 目标需为 http(s):// 地址或站内路径：{}",
                        l.target
                    ));
                }
            }
            "deny" => {
                if !matches!(l.code, 0 | 403 | 404 | 410 | 444) {
                    return Err(format!(
                        "deny 状态码仅支持 403/404/410/444（收到 {}）",
                        l.code
                    ));
                }
            }
            "alias" => {
                let t = l.target.trim();
                let r =
                    root.ok_or_else(|| "alias location 仅用于 php / static 站点".to_string())?;
                let tp = Path::new(t);
                if !t.starts_with('/') || !path_under(r, tp) {
                    return Err(format!("alias 目标必须位于站点目录内：{t}"));
                }
            }
            // raw：高级自由指令体（逐行渲染在 location 块内，防越界）
            "raw" => {
                let r = &l.raw;
                if r.trim().is_empty() {
                    return Err("raw 自由指令体不能为空".to_string());
                }
                if r.len() > 8000 {
                    return Err("raw 自由指令体过长（上限 8000 字符）".to_string());
                }
                for line in r.lines() {
                    let s = line.trim();
                    if s.is_empty() {
                        continue;
                    }
                    if s.starts_with('#') {
                        return Err("raw 自由指令体中不允许使用 # 注释".to_string());
                    }
                    if s == "include" || s.starts_with("include ") || s.starts_with("include\t") {
                        return Err("raw 自由指令体中不允许 include".to_string());
                    }
                    if s.starts_with("server")
                        || s.starts_with("location ")
                        || s.starts_with("upstream")
                    {
                        return Err(
                            "raw 自由指令体不允许嵌套 server / location / upstream 块".to_string()
                        );
                    }
                    if s.contains('{') || s.contains('}') {
                        return Err(
                            "raw 自由指令体不允许出现花括号（仅支持单层 location 内指令）"
                                .to_string(),
                        );
                    }
                }
            }
            _ => return Err(format!("location 类型不支持：{}", l.kind)),
        }
    }
    if s_type == "proxy" && !has_root_loc {
        return Err(
            "反向代理站点至少需要一个 location /（兜底转发），请检查 locations 配置".to_string(),
        );
    }
    Ok(())
}

/// 递归收敛站点树属主与权限（幂等）：
/// - web tree：chown -R {owner}:{owner 主组}；目录 755 / 文件 644
///   （nginx worker 走 others 位读静态文件，站点文件不再归属 / 依赖 www 组）
/// - log tree（is_log=true）：chown -R www:www；目录 770 / 文件 660
///   （日志由 nginx worker 写入，必须保持 www 组可写）
fn fix_tree_owner(root: &Path, owner: &str, is_log: bool) -> Result<(), String> {
    // web tree 归运行账号 + 其主组；log tree 恒归 www:www（nginx worker 写日志）
    let group = if is_log {
        "www".to_string()
    } else {
        super::user::run_group_of(owner)
    };
    let script = tree_fix_script(root, owner, &group, is_log);
    let o = root_cmd("bash")
        .args(["-c", &script])
        .output()
        .map_err(|e| format!("收敛站点树属主/权限失败: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(format!(
            "chown/chmod 站点树失败：{}",
            output_err(&o, "未知错误")
        ))
    }
}

/// 生成站点树收敛脚本（纯函数，单测覆盖）：
/// `{{}}` 是 find 的匹配文件占位符（format! 里必须转义），漏写会让 chmod 因缺文件参数报错。
fn tree_fix_script(root: &Path, owner: &str, group: &str, is_log: bool) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let (dir_mode, file_mode) = if is_log {
        ("770", "660")
    } else {
        ("755", "644")
    };
    let root_s = root.to_string_lossy();
    format!(
        "chown -R {}:{} {} && find {} -type d -exec chmod {} {{}} \\; && find {} -type f -exec chmod {} {{}} \\;",
        q(owner),
        q(group),
        q(&root_s),
        q(&root_s),
        dir_mode,
        q(&root_s),
        file_mode
    )
}

/// 站点 vhost 同步命令的完整入参：协议 `site.vhost_sync` 的字段原样搬入，
/// 由 [`vhost_sync`] 一路传给 [`vhost_sync_inner`]（避免平铺 22 个参数）。
pub(super) struct SiteConfig {
    pub(super) site_id: i64,
    pub(super) name: String,
    pub(super) domains: Vec<String>,
    pub(super) enabled: bool,
    pub(super) mode: Option<String>,
    pub(super) php_socket: Option<String>,
    pub(super) web_root: Option<String>,
    pub(super) log_root: Option<String>,
    pub(super) owner_user: Option<String>,
    pub(super) site_type: String,
    pub(super) pseudo_static: String,
    pub(super) pseudo_custom: String,
    pub(super) web_root_custom: bool,
    pub(super) upstreams: Vec<UpstreamSpec>,
    pub(super) locations: Vec<LocationSpec>,
    pub(super) ssl_fullchain: Option<String>,
    pub(super) ssl_key: Option<String>,
    pub(super) force_https: bool,
    pub(super) ssl_protocols: String,
    pub(super) ssl_ciphers: String,
    pub(super) ssl_prefer_server_ciphers: bool,
    pub(super) ssl_http2: bool,
    pub(super) listen_ipv4: String,
    pub(super) listen_ipv6: String,
}

/// 同步站点完整 vhost 配置（拆箱到阻塞线程执行）。
pub(super) async fn vhost_sync(cfg: SiteConfig) -> Response {
    tokio::task::spawn_blocking(move || vhost_sync_inner(cfg))
        .await
        .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
        .unwrap_or_else(|e| Response::err(-1, e))
}

/// 站点运行状态：running / stopped / maintenance（None 时按 enabled 推导，兼容老面板）
fn run_mode(mode: Option<&str>, enabled: bool) -> &str {
    match mode.unwrap_or("").trim().to_lowercase().as_str() {
        "stopped" | "stop" => "stopped",
        "maintenance" | "maintain" => "maintenance",
        "running" | "start" => "running",
        _ => {
            if enabled {
                "running"
            } else {
                "stopped"
            }
        }
    }
}

/// 站点配置是否需要反代共享缓存区（任意 proxy location 启用了 zap_cache）
fn needs_cache_zone(locations: &[LocationSpec]) -> bool {
    locations
        .iter()
        .any(|l| l.kind.trim().eq_ignore_ascii_case("proxy") && !l.cache.trim().is_empty())
}

/// 幂等准备反代共享缓存区：
/// - 磁盘缓存目录（{ZAP_PATH}/data/cache/nginx-zap_cache）存在且属主 www:www（nginx worker 写）；
/// - 向 nginx include 目录发布固定名 zone 定义文件（include 位于 http 上下文，zone 随之生效）。
///
/// zone 文件独立于站点 vhost、可被多个站点复用；无站点引用时残留亦无害。
fn ensure_cache_zone(_edir: &std::path::Path) -> Result<(), String> {
    let cache_root = zap_path().join("data/cache/nginx-zap_cache");
    if !cache_root.exists() {
        std::fs::create_dir_all(&cache_root).map_err(|e| format!("创建反代缓存目录失败: {e}"))?;
        let _ = std::process::Command::new("chown")
            .args(["-R", "www:www"])
            .arg(&cache_root)
            .status();
    }
    let content = format!(
        "# Generated by Zap Panel — 反代共享缓存区（zap_cache）— DO NOT EDIT\n\
         proxy_cache_path {} levels=1:2 keys_zone=zap_cache:32m max_size=1g inactive=60m use_temp_path=off;\n",
        cache_root.to_string_lossy()
    );
    super::webconf::publish_named("nginx", "00-zap-cache.conf", &content).map(|_| ())
}

fn vhost_sync_inner(cfg: SiteConfig) -> Result<Response, String> {
    // 解构为局部变量；name/domains 由 owned 转为借用，与函数体历史用法（&str / &[String]）一致
    let SiteConfig {
        site_id,
        name,
        domains,
        enabled,
        mode,
        php_socket,
        web_root,
        log_root,
        owner_user,
        site_type,
        pseudo_static,
        pseudo_custom,
        web_root_custom,
        upstreams,
        locations,
        ssl_fullchain,
        ssl_key,
        force_https,
        ssl_protocols,
        ssl_ciphers,
        ssl_prefer_server_ciphers,
        ssl_http2,
        listen_ipv4,
        listen_ipv6,
    } = cfg;
    let name = &name;
    let domains = &domains;
    let state = run_mode(mode.as_deref(), enabled);
    let conf_file = match find_nginx_conf_file() {
        Some(c) => c,
        None => {
            return if state == "running" {
                Err(
                    "未找到 Nginx 安装（安装根 /usr/local/apps 下无 conf/nginx.conf）。\
                     请先在「应用商店 → Web服务器 → Nginx」安装并部署 Nginx"
                        .to_string(),
                )
            } else {
                // 站点停用且无 Nginx：无事可清理
                Ok(Response::ok("站点已停用（Nginx 未安装，无需清理）", None))
            };
        }
    };
    let bin = nginx_bin(&conf_file);
    // 生效配置统一放 /etc/zap/webservers/nginx/{sites-available,sites-enabled}；
    // 旧位置（<nginx prefix>/conf/sites-enabled）仅用于一次性迁移。
    let legacy_dir = vhosts_dir(&conf_file).ok();
    let edir = super::webconf::enabled_dir("nginx");
    let avail = super::webconf::available_path("nginx", site_id);

    if let Some(dir) = &legacy_dir {
        super::webconf::migrate_legacy(dir, "nginx", site_id)?;
    }

    // 默认站点常驻（停止/未匹配域名不再落到别的站点上）
    ensure_default_vhost(&conf_file, &bin);

    if state == "stopped" {
        if super::webconf::unpublish("nginx", site_id)? && nginx_running() {
            reload_nginx(&bin)?;
        }
        super::webconf::backup_snapshot(site_id, "nginx", "# 站点已停止\n");
        return Ok(Response::ok(
            "站点已停止（配置保留在 sites-available，访问由默认站点接管）",
            None,
        ));
    }

    // 维护态：只发布维护页 vhost，不触碰业务目录与 PHP
    if state == "maintenance" {
        let maint = ensure_maintenance_page()?;
        let content = render_maintenance_vhost(site_id, name, domains, &maint.to_string_lossy());
        let previous = std::fs::read_to_string(&avail).ok();
        super::webconf::publish("nginx", site_id, &content)?;
        if let Err(e) = nginx_test(&bin) {
            match previous {
                Some(prev) => {
                    let _ = std::fs::write(&avail, prev);
                }
                None => {
                    let _ = super::webconf::purge("nginx", site_id);
                }
            }
            return Err(format!("nginx -t 校验失败，已回滚维护配置：\n{e}"));
        }
        super::webconf::backup_snapshot(site_id, "nginx", &content);
        if nginx_running() {
            reload_nginx(&bin)?;
            return Ok(Response::ok("站点已切到维护页，Nginx 已重载", None));
        }
        return Ok(Response::ok("维护页配置已写入并通过校验", None));
    }

    // 文档根：采用面板入库的 web_root（位于归属用户家目录下）。
    // 反向代理（proxy）类型不需要文档根。
    let s_type = norm_site_type(&site_type);
    let is_proxy = s_type == "proxy";
    let root: Option<PathBuf> = if is_proxy {
        None
    } else {
        Some(match web_root.as_deref() {
            Some(w) if !w.trim().is_empty() => {
                if !dir_arg_ok(w) {
                    return Err("web_root 必须是形如 /home/u/www/xxx 的绝对路径".to_string());
                }
                PathBuf::from(w)
            }
            // 独立系统用户模式下站点必须落在归属用户家目录，不再回退共享目录
            _ => {
                return Err(
                    "站点缺少 web_root（必须位于归属用户家目录下，如 /home/{用户}/www/{站点}）：\
                     请重新保存站点后再次同步"
                        .to_string(),
                );
            }
        })
    };
    // 自定义目录：必须是已存在的目录；自动目录：递归创建 + 占位首页
    if let Some(r) = &root {
        if web_root_custom {
            if !r.is_dir() {
                return Err(format!(
                    "自定义站点目录不存在或不是目录：{}（请先在服务器上创建该目录，再重新同步）",
                    r.display()
                ));
            }
        } else {
            // create_dir_all 会递归创建归属用户家目录骨架（/home/{u}/www/...）
            ensure_web_root(r, site_id, name, domains)?;
        }
    }
    // 高级配置校验（类型 / 伪静态 / upstream / location 等）：失败即中止发布
    validate_vhost_cfg(
        &site_type,
        &pseudo_static,
        &pseudo_custom,
        web_root_custom,
        root.as_deref(),
        &upstreams,
        &locations,
    )?;
    // 站点树属主/权限收敛（自动目录）：
    // - web tree：归归属用户的 Linux 账号及其主组，目录 755 / 文件 644
    //   （nginx worker 走 others 位读静态文件，php-fpm 以该账号身份读写）
    // - log tree：恒归 www:www，目录 770 / 文件 660（nginx 写 access/error.log）
    // 自定义目录不递归改动用户已有文件属主/权限（由用户自管，nginx 需可读其文件）。
    if let Some(r) = &root {
        let web_owner = owner_user
            .as_deref()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                "站点未绑定运行账号（归属面板用户缺失），无法收敛站点文件属主".to_string()
            })?;
        if !web_root_custom {
            fix_tree_owner(r, web_owner, false)?;
        }
    }

    // 日志：面板规划了 log_root（{home}/logs/{site}）时生成独立 access/error 日志
    let (mut access_log, mut error_log) = (None, None);
    if let Some(lr) = log_root.as_deref() {
        let lr = lr.trim();
        if !lr.is_empty() {
            if !dir_arg_ok(lr) {
                return Err("log_root 必须是形如 /home/u/logs/xxx 的绝对路径".to_string());
            }
            let ldir = PathBuf::from(lr);
            std::fs::create_dir_all(&ldir).map_err(|e| format!("创建站点日志目录失败: {e}"))?;
            fix_tree_owner(&ldir, "www", true)?;
            access_log = Some(ldir.join("access.log").to_string_lossy().to_string());
            error_log = Some(ldir.join("error.log").to_string_lossy().to_string());
        }
    }

    let root_s = root
        .as_deref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    // SSL/TLS：绑定证书（fullchain + key 齐备）时落盘证书文件供 nginx 引用；
    // 未启用或材料缺失时清理历史文件，防止旧证书残留。
    let ssl_files: Option<(String, String)> = match (ssl_fullchain.as_deref(), ssl_key.as_deref()) {
        (Some(fc), Some(k)) if !fc.trim().is_empty() && !k.trim().is_empty() => {
            Some(write_site_ssl_files(site_id, fc, k)?)
        }
        _ => {
            remove_site_ssl_files(site_id);
            None
        }
    };
    let ssl_refs: Option<(&str, &str)> = ssl_files.as_ref().map(|(a, b)| (a.as_str(), b.as_str()));
    // TLS 高级设置：仅绑定证书时构造渲染配置（http2 指令写法取决于 nginx 版本）
    let ssl_tls_cfg = ssl_files.as_ref().map(|_| SslTlsCfg {
        protocols: ssl_protocols,
        ciphers: ssl_ciphers,
        prefer_server_ciphers: ssl_prefer_server_ciphers,
        http2: ssl_http2,
        http2_on_syntax: nginx_http2_on_syntax(&bin),
    });
    let content = render_vhost_full(VhostRenderSpec {
        site_id,
        name,
        domains,
        root: &root_s,
        php_socket: php_socket.as_deref(),
        access_log: access_log.as_deref(),
        error_log: error_log.as_deref(),
        site_type: &site_type,
        pseudo_static: &pseudo_static,
        pseudo_custom: &pseudo_custom,
        upstreams: &upstreams,
        locations: &locations,
        ssl_files: ssl_refs,
        force_https,
        ssl_tls: ssl_tls_cfg.as_ref(),
        listen_ipv4: &listen_ipv4,
        listen_ipv6: &listen_ipv6,
    });

    // 面板侧快照（渲染源 / 入参 / 历史版本）：失败不影响发布，仅作排障与回滚副本
    let meta = json!({
        "site_id": site_id,
        "name": name,
        "domains": domains,
        "site_type": s_type,
        "pseudo_static": pseudo_static,
        "web_root_custom": web_root_custom,
        "web_root": root.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "log_root": log_root,
        "owner_user": owner_user,
        "php_socket": php_socket,
        "upstreams": upstreams,
        "locations": locations,
        "enabled": enabled,
        "ssl_enabled": ssl_files.is_some(),
        "force_https": force_https,
    });
    if let Err(e) = super::webconf::write_snapshot(site_id, "nginx", &content, &meta) {
        tracing::warn!("写入站点配置快照失败（不影响发布）: {e}");
    }

    // 主配置幂等注入 include（指向新的 sites-enabled）；返回 true 表示本次改了主配置
    let injected = super::webconf::ensure_include(&conf_file, &edir, "nginx")?;

    // 反代缓存：任一 proxy location 启用 zap_cache 时，先幂等发布共享缓存区
    // （nginx -t 需要 zone 已存在；失败时恢复 include 再中止）
    if needs_cache_zone(&locations)
        && let Err(e) = ensure_cache_zone(&edir)
    {
        if injected {
            super::webconf::restore_include(&conf_file);
        }
        return Err(e);
    }

    // 发布前保留上一版内容，便于校验失败时回滚
    let previous = std::fs::read_to_string(&avail).ok();
    if let Err(e) = super::webconf::publish("nginx", site_id, &content) {
        if injected {
            super::webconf::restore_include(&conf_file);
        }
        return Err(e);
    }

    // 校验：失败即回滚（恢复上一版或撤下本次发布），绝不带着坏配置 reload
    if let Err(e) = nginx_test(&bin) {
        match previous {
            Some(prev) => {
                super::webconf::backup_snapshot(site_id, "nginx", &content);
                let _ = std::fs::write(&avail, prev);
            }
            None => {
                let _ = super::webconf::purge("nginx", site_id);
            }
        }
        if injected {
            super::webconf::restore_include(&conf_file);
        }
        return Err(format!("nginx -t 校验失败，已回滚本次配置：\n{e}"));
    }
    // 通过校验：留一份历史版本用于回滚
    super::webconf::backup_snapshot(site_id, "nginx", &content);

    let data = json!({
        "site_id": site_id,
        "available": avail.to_string_lossy().to_string(),
        "enabled": super::webconf::enabled_path("nginx", site_id).to_string_lossy().to_string(),
        "root": root.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "site_type": s_type,
    });
    if nginx_running() {
        reload_nginx(&bin)?;
        Ok(Response::ok("站点配置已同步，Nginx 已重载", Some(data)))
    } else {
        Ok(Response::ok(
            "站点配置已写入并通过校验；Nginx 当前未运行，启动后自动生效",
            Some(data),
        ))
    }
}

pub async fn vhost_remove(site_id: i64, name: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let Some(conf_file) = find_nginx_conf_file() else {
            return Ok(Response::ok("Nginx 未安装，无需清理", None));
        };
        let bin = nginx_bin(&conf_file);
        // 旧布局遗留文件一并清理
        if let Ok(vdir) = vhosts_dir(&conf_file) {
            let legacy = vdir.join(super::webconf::vhost_name(site_id));
            if legacy.exists() {
                let _ = std::fs::remove_file(&legacy);
            }
        }
        let purged = super::webconf::purge("nginx", site_id)?;
        // 清理站点 SSL 证书文件（绑定已随 vhost 一并删除）
        remove_site_ssl_files(site_id);
        if !purged {
            return Ok(Response::ok("vhost 不存在，无需清理", None));
        }
        if nginx_running() {
            reload_nginx(&bin)?;
        }
        Ok(Response::ok(
            format!("vhost 已移除（site {site_id} {name}）"),
            None,
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 站点数据目录删除（root）：站点删除时勾选「同时删除网站数据」才调用。
///
/// 清理范围 = 文档根（网站文件）+ 日志目录（access.log / error.log 及其轮转归档），
/// 日志随网站数据一起删，不留残留。
///
/// 安全边界（任一不满足即跳过该路径，不整体失败）：
/// - 绝对路径、不含 `..` 段；
/// - 真实路径（canonicalize，穿透符号链接）位于 `/home/*`（至少两级：`/home/{u}/{dir}`）
///   或面板数据目录 `{ZAP_PATH}/data/{www,logs}` 之下（至少一级）；
/// - 不允许是上述容器目录本身（避免误删整个 www / logs / 家目录）。
pub async fn data_remove(web_roots: Vec<String>, log_roots: Vec<String>) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let mut removed: Vec<String> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();

        for raw in web_roots.iter().chain(log_roots.iter()) {
            let path = raw.trim();
            if path.is_empty() {
                continue;
            }
            if !path.starts_with('/') || path.split('/').any(|s| s == "..") {
                skipped.push(path.to_string());
                continue;
            }
            // canonicalize 同时确认存在性：已删/不存在视为完成，不重复统计
            let real = match std::fs::canonicalize(path) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if !data_path_allowed(&real) {
                skipped.push(path.to_string());
                continue;
            }
            match std::fs::remove_dir_all(&real) {
                Ok(()) => removed.push(path.to_string()),
                Err(e) => {
                    // 文档根被替换成文件/软链的极端情况：退化为删文件
                    if std::fs::remove_file(&real).is_ok() {
                        removed.push(path.to_string());
                    } else {
                        skipped.push(format!("{path} ({e})"));
                    }
                }
            }
        }

        let data = json!({
            "removed": removed,
            "skipped": skipped,
            "count": removed.len(),
        });
        if skipped.is_empty() {
            Ok(Response::ok(
                format!("已删除 {} 个站点数据目录", removed.len()),
                Some(data),
            ))
        } else {
            Ok(Response::err(
                -1,
                format!(
                    "已删除 {} 个目录，{} 个被跳过（路径不在允许范围内）：{}",
                    removed.len(),
                    skipped.len(),
                    skipped.join(", ")
                ),
            ))
        }
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 站点数据目录白名单：只允许删除家目录内（`{home}/{user}/{...}`）与
/// 面板数据目录内（`{ZAP_PATH}/data/{www,logs}/{...}`）的路径，且必须是其下子目录。
fn data_path_allowed(real: &Path) -> bool {
    let s = real.to_string_lossy().to_string();
    let zap = zap_path().to_string_lossy().to_string();
    let bases: Vec<(String, usize)> = vec![
        ("/home".to_string(), 2),
        (format!("{zap}/data/www"), 1),
        (format!("{zap}/data/logs"), 1),
    ];
    for (base, min_depth) in bases {
        let Some(rest) = s.strip_prefix(&base) else {
            continue;
        };
        let depth = rest
            .trim_start_matches('/')
            .split('/')
            .filter(|x| !x.is_empty())
            .count();
        if depth >= min_depth {
            return true;
        }
    }
    false
}

pub(super) fn nginx_test(bin: &Path) -> Result<(), String> {
    let o = root_cmd("bash")
        .args(["-c"])
        .arg(format!(
            "'{}' -t 2>&1",
            bin.to_string_lossy().replace('\'', "'\\''")
        ))
        .output()
        .map_err(|e| format!("执行 nginx -t 失败: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(output_err(&o, "nginx -t 返回非零"))
    }
}

pub(super) fn reload_nginx(bin: &Path) -> Result<(), String> {
    let o = root_cmd("bash")
        .args(["-c"])
        .arg(format!(
            "'{}' -s reload 2>&1",
            bin.to_string_lossy().replace('\'', "'\\''")
        ))
        .output()
        .map_err(|e| format!("执行 nginx -s reload 失败: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(format!(
            "nginx -s reload 失败：{}",
            output_err(&o, "未知错误")
        ))
    }
}

// ── 单测 ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use zap_proto::UpstreamServer;

    /// 指定共享 IP 时监听 `IP:80` / `IP:443 ssl`；未指定（默认）沿用通配监听。
    #[test]
    fn shared_ip_binds_listen_address() {
        let domains = vec!["a.com".to_string()];
        let s = render_vhost_full(VhostRenderSpec {
            site_id: 11,
            name: "shared",
            domains: &domains,
            root: "/home/u/www/shared-11",
            php_socket: None,
            access_log: None,
            error_log: None,
            site_type: "php",
            pseudo_static: "none",
            pseudo_custom: "",
            upstreams: &[],
            locations: &[],
            ssl_files: Some(("/a/fullchain.pem", "/a/key.pem")),
            force_https: false,
            ssl_tls: None,
            listen_ipv4: "1.2.3.4",
            listen_ipv6: "2408::1",
        });
        assert!(s.contains("listen 1.2.3.4:80;"), "共享 IPv4 应绑定 IP:80");
        assert!(
            s.contains("listen [2408::1]:80;"),
            "共享 IPv6 应绑定 [IP]:80"
        );
        assert!(s.contains("listen 1.2.3.4:443 ssl;"));
        assert!(s.contains("listen [2408::1]:443 ssl;"));
        assert!(!s.contains("listen 80;"), "不应再出现通配监听");

        let d = render_vhost_full(VhostRenderSpec {
            site_id: 12,
            name: "default",
            domains: &domains,
            root: "/home/u/www/default-12",
            php_socket: None,
            access_log: None,
            error_log: None,
            site_type: "php",
            pseudo_static: "none",
            pseudo_custom: "",
            upstreams: &[],
            locations: &[],
            ssl_files: None,
            force_https: false,
            ssl_tls: None,
            listen_ipv4: "",
            listen_ipv6: "",
        });
        assert!(d.contains("listen 80;"));
        assert!(d.contains("listen [::]:80;"));
    }

    #[test]
    fn listen_directive_forms() {
        assert_eq!(
            listen_directive("", "", 80, ""),
            "    listen 80;\n    listen [::]:80;\n"
        );
        assert_eq!(
            listen_directive("1.2.3.4", "2408::1", 443, " ssl"),
            "    listen 1.2.3.4:443 ssl;\n    listen [2408::1]:443 ssl;\n"
        );
    }

    /// 收敛脚本必须是「chown + find chmod」，且 find 的 `{}` 占位符不能被 format! 吞掉
    /// （历史 bug：生成 `-exec chmod 755 \;` 缺文件参数，权限从未真正收敛）。
    #[test]
    fn tree_fix_script_keeps_find_placeholder() {
        let web = tree_fix_script(Path::new("/home/u/www/a-1"), "u", "u", false);
        assert_eq!(
            web,
            "chown -R 'u':'u' '/home/u/www/a-1' \
             && find '/home/u/www/a-1' -type d -exec chmod 755 {} \\; \
             && find '/home/u/www/a-1' -type f -exec chmod 644 {} \\;"
        );
        // 日志树：恒归 www:www，目录 770 / 文件 660
        let log = tree_fix_script(Path::new("/home/u/logs/a-1"), "www", "www", true);
        assert!(log.contains("chown -R 'www':'www' '/home/u/logs/a-1'"));
        assert!(log.contains("-exec chmod 770 {} \\;"));
        assert!(log.contains("-exec chmod 660 {} \\;"));
        // 含单引号的路径需安全转义
        let weird = tree_fix_script(Path::new("/home/u/it's"), "u", "u", false);
        assert!(weird.contains("'/home/u/it'\\''s'"));
    }

    /// 便捷构造一个表单化 server 行（默认参数）
    fn sv(addr: &str, weight: u32) -> UpstreamServer {
        UpstreamServer {
            addr: addr.into(),
            weight,
            ..Default::default()
        }
    }

    #[test]
    fn skel_placeholders_are_replaced() {
        let tpl =
            "N=__SITE_NAME__;I=__SITE_ID__;D=__SITE_DOMAINS__;R=__SITE_ROOT__;C=__CREATED_AT__";
        let out = apply_placeholders(
            tpl,
            7,
            "blog",
            &["a.com".into(), "b.com".into()],
            Path::new("/home/u/www/blog-7"),
            "2026-09-06 10:00:00",
        );
        assert_eq!(
            out,
            "N=blog;I=7;D=a.com、b.com;R=/home/u/www/blog-7;C=2026-09-06 10:00:00"
        );
    }

    #[test]
    fn skel_placeholders_without_domain() {
        let out = apply_placeholders("D=__SITE_DOMAINS__", 1, "x", &[], Path::new("/r"), "");
        assert_eq!(out, "D=未绑定域名");
    }

    /// 默认 php 站点渲染（等价于旧 render_vhost）
    fn render_default(
        site_id: i64,
        name: &str,
        domains: &[String],
        root: &str,
        php_socket: Option<&str>,
        access_log: Option<&str>,
        error_log: Option<&str>,
    ) -> String {
        render_vhost_full(VhostRenderSpec {
            site_id,
            name,
            domains,
            root,
            php_socket,
            access_log,
            error_log,
            site_type: "php",
            pseudo_static: "none",
            pseudo_custom: "",
            upstreams: &[],
            locations: &[],
            ssl_files: None,
            force_https: false,
            ssl_tls: None,
            listen_ipv4: "",
            listen_ipv6: "",
        })
    }

    #[test]
    fn render_static_vhost() {
        let s = render_default(
            1,
            "blog",
            &["a.com".into(), "b.com".into()],
            "/zap/www/blog-1",
            None,
            None,
            None,
        );
        assert!(s.contains("server_name a.com b.com;"));
        assert!(s.contains("root /zap/www/blog-1;"));
        assert!(s.contains("index index.html;"));
        assert!(!s.contains("fastcgi"));
        assert!(!s.contains("index.php"));
        assert!(!s.contains("access_log"));
    }

    #[test]
    fn render_php_unix_socket() {
        let s = render_default(
            2,
            "app",
            &["app.example.com".into()],
            "/zap/www/app-2",
            Some("unix:/var/run/php-fpm-8.3.sock"),
            None,
            None,
        );
        assert!(s.contains("index index.php index.html;"));
        assert!(s.contains("fastcgi_pass unix:/var/run/php-fpm-8.3.sock;"));
        assert!(s.contains("SCRIPT_FILENAME $document_root$fastcgi_script_name"));
    }

    #[test]
    fn render_php_tcp() {
        let s = render_default(
            3,
            "x",
            &[],
            "/zap/www/x-3",
            Some("127.0.0.1:9000"),
            None,
            None,
        );
        assert!(s.contains("server_name _;"));
        assert!(s.contains("fastcgi_pass 127.0.0.1:9000;"));
    }

    #[test]
    fn render_with_site_logs() {
        let s = render_default(
            7,
            "b",
            &["b.com".into()],
            "/home/u/www/b-7",
            None,
            Some("/home/u/logs/b-7/access.log"),
            Some("/home/u/logs/b-7/error.log"),
        );
        assert!(s.contains("root /home/u/www/b-7;"));
        assert!(s.contains("access_log /home/u/logs/b-7/access.log;"));
        assert!(s.contains("error_log /home/u/logs/b-7/error.log;"));
    }

    #[test]
    fn run_mode_resolution() {
        // 老面板只有 enabled：true→running，false→stopped
        assert_eq!(run_mode(None, true), "running");
        assert_eq!(run_mode(None, false), "stopped");
        assert_eq!(run_mode(Some("STOPPED"), true), "stopped");
        assert_eq!(run_mode(Some("maintenance"), true), "maintenance");
        // 未知值回退 enabled
        assert_eq!(run_mode(Some("bogus"), false), "stopped");
    }

    #[test]
    fn default_and_maintenance_vhost_render() {
        let d = render_default_vhost(false);
        assert!(d.contains("listen 80 default_server"));
        assert!(d.contains("return 444"), "默认站点应直接断开，避免串站");
        assert!(
            !d.contains("stub_status"),
            "未启用状态页时不应注入 location"
        );
        // 444 必须落在 location / 内：server 级 return 会在 location 匹配前生效，
        // 那样状态页永远取不到数据（实测返回空响应）
        assert!(
            d.contains("location / {") && !d.trim_start().starts_with("return 444"),
            "444 必须下沉到 location /"
        );

        let s = render_default_vhost(true);
        assert!(s.contains("stub_status"), "启用状态页应注入 stub_status");
        assert!(s.contains(&format!("location = {STUB_PATH}")));

        let m =
            render_maintenance_vhost(3, "blog", &["a.com".into(), "b.com".into()], "/srv/maint");
        assert!(m.contains("server_name a.com b.com"));
        assert!(m.contains("root /srv/maint"));
        assert!(m.contains("error_page 503 /maintenance.html"));
        assert!(m.contains("return 503"));
    }

    #[test]
    fn render_static_type_has_no_php_and_tryfiles() {
        let s = render_vhost_full(VhostRenderSpec {
            site_id: 1,
            name: "s",
            domains: &["s.com".into()],
            root: "/home/u/www/s-1",
            php_socket: None,
            access_log: None,
            error_log: None,
            site_type: "static",
            pseudo_static: "none",
            pseudo_custom: "",
            upstreams: &[],
            locations: &[],
            ssl_files: None,
            force_https: false,
            ssl_tls: None,
            listen_ipv4: "",
            listen_ipv6: "",
        });
        assert!(s.contains("root /home/u/www/s-1;"));
        assert!(s.contains("try_files $uri $uri/ =404;"));
        assert!(!s.contains("fastcgi"), "静态站点不应有 PHP location");
    }

    #[test]
    fn render_pseudo_thinkphp_and_laravel() {
        let s = render_vhost_full(VhostRenderSpec {
            site_id: 1,
            name: "tp",
            domains: &["tp.com".into()],
            root: "/home/u/www/tp-1",
            php_socket: Some("unix:/run/php.sock"),
            access_log: None,
            error_log: None,
            site_type: "php",
            pseudo_static: "thinkphp",
            pseudo_custom: "",
            upstreams: &[],
            locations: &[],
            ssl_files: None,
            force_https: false,
            ssl_tls: None,
            listen_ipv4: "",
            listen_ipv6: "",
        });
        assert!(
            s.contains("rewrite ^(.*)$ /index.php?s=$1 last;"),
            "thinkphp 伪静态规则应渲染进 location /"
        );

        let s2 = render_vhost_full(VhostRenderSpec {
            site_id: 2,
            name: "lv",
            domains: &["lv.com".into()],
            root: "/home/u/www/lv-2",
            php_socket: Some("unix:/run/php.sock"),
            access_log: None,
            error_log: None,
            site_type: "php",
            pseudo_static: "laravel",
            pseudo_custom: "",
            upstreams: &[],
            locations: &[],
            ssl_files: None,
            force_https: false,
            ssl_tls: None,
            listen_ipv4: "",
            listen_ipv6: "",
        });
        assert!(s2.contains("try_files $uri $uri/ /index.php?$query_string;"));
        assert!(
            !s2.contains("rewrite"),
            "laravel 用 try_files 实现，不应出现 rewrite"
        );
    }

    #[test]
    fn render_proxy_with_upstream_and_locations() {
        let ups = vec![UpstreamSpec {
            name: "backend".into(),
            servers_ext: vec![sv("127.0.0.1:9001", 0), sv("127.0.0.1:9002", 2)],
            ..Default::default()
        }];
        let locs = vec![
            LocationSpec {
                path: "/".into(),
                kind: "proxy".into(),
                target: "backend".into(),
                code: 0,
                ws: true,
                ..Default::default()
            },
            LocationSpec {
                path: "/admin".into(),
                kind: "deny".into(),
                target: String::new(),
                code: 403,
                ws: false,
                ..Default::default()
            },
            LocationSpec {
                path: "/static".into(),
                kind: "alias".into(),
                target: "/home/u/www/p-3/static".into(),
                code: 0,
                ws: false,
                ..Default::default()
            },
        ];
        let s = render_vhost_full(VhostRenderSpec {
            site_id: 3,
            name: "proxy",
            domains: &["p.com".into()],
            root: "",
            php_socket: None,
            access_log: None,
            error_log: None,
            site_type: "proxy",
            pseudo_static: "none",
            pseudo_custom: "",
            upstreams: &ups,
            locations: &locs,
            ssl_files: None,
            force_https: false,
            ssl_tls: None,
            listen_ipv4: "",
            listen_ipv6: "",
        });
        assert!(s.contains("upstream backend {"), "应渲染 upstream 块");
        assert!(s.contains("server 127.0.0.1:9001;"));
        assert!(s.contains("server 127.0.0.1:9002 weight=2;"));
        assert!(s.contains("location / {"));
        assert!(s.contains("proxy_pass backend;"));
        assert!(
            s.contains("proxy_set_header Upgrade $http_upgrade;"),
            "ws 开关应输出升级头"
        );
        assert!(s.contains("location /admin {"));
        assert!(s.contains("return 403;"));
        assert!(s.contains("location /static {"));
        assert!(s.contains("alias /home/u/www/p-3/static;"));
        assert!(!s.contains("root "), "proxy 类型不应渲染 root");
        assert!(!s.contains("fastcgi"));
    }

    /// 反代站点同样要能签发证书：`^~` 的 ACME location 必须优先于用户自定义 location，
    /// 否则 HTTP-01 验证请求会被代理到后端而永远拿不到 token 文件。
    #[test]
    fn acme_location_rendered_for_every_site_type() {
        let expect = |s: &str, kind: &str| {
            let root = acme_webroot();
            assert!(
                s.contains("location ^~ /.well-known/acme-challenge/ {"),
                "{kind} 站点缺少 ACME location"
            );
            assert!(
                s.contains(&format!(
                    "alias {}/.well-known/acme-challenge/;",
                    root.display()
                )),
                "{kind} 站点的 ACME alias 未指向验证根"
            );
        };
        expect(
            &render_default(
                4,
                "blog",
                &["a.com".into()],
                "/zap/www/blog-4",
                None,
                None,
                None,
            ),
            "静态",
        );
        let ups = vec![UpstreamSpec {
            name: "b".into(),
            servers_ext: vec![sv("127.0.0.1:9000", 0)],
            ..Default::default()
        }];
        let locs = vec![LocationSpec {
            path: "/".into(),
            kind: "proxy".into(),
            target: "b".into(),
            ..Default::default()
        }];
        expect(
            &render_vhost_full(VhostRenderSpec {
                site_id: 5,
                name: "p",
                domains: &["p.com".into()],
                root: "",
                php_socket: None,
                access_log: None,
                error_log: None,
                site_type: "proxy",
                pseudo_static: "none",
                pseudo_custom: "",
                upstreams: &ups,
                locations: &locs,
                ssl_files: None,
                force_https: false,
                ssl_tls: None,
                listen_ipv4: "",
                listen_ipv6: "",
            }),
            "反代",
        );
    }

    #[test]
    fn validate_cfg_proxy_requires_root_location() {
        let ups = vec![UpstreamSpec {
            name: "b".into(),
            servers_ext: vec![sv("127.0.0.1:9000", 0)],
            ..Default::default()
        }];
        // proxy 但没有 location / → 拒绝
        let locs = vec![LocationSpec {
            path: "/api".into(),
            kind: "proxy".into(),
            target: "b".into(),
            code: 0,
            ws: false,
            ..Default::default()
        }];
        let e = validate_vhost_cfg("proxy", "none", "", false, None, &ups, &locs);
        assert!(e.is_err());
        assert!(e.unwrap_err().contains("location /"));

        // 合法：带 location /
        let locs2 = vec![
            LocationSpec {
                path: "/".into(),
                kind: "proxy".into(),
                target: "b".into(),
                code: 0,
                ws: false,
                ..Default::default()
            },
            LocationSpec {
                path: "/api".into(),
                kind: "proxy".into(),
                target: "b".into(),
                code: 0,
                ws: false,
                ..Default::default()
            },
        ];
        assert!(validate_vhost_cfg("proxy", "none", "", false, None, &ups, &locs2).is_ok());
    }

    #[test]
    fn validate_cfg_rejects_bad_inputs() {
        let tmp = std::env::temp_dir().join("zap-rs-validate-cfg");
        let _ = std::fs::create_dir_all(&tmp);
        // 未知站点类型
        assert!(validate_vhost_cfg("hack", "none", "", false, Some(&tmp), &[], &[]).is_err());
        // proxy_pass 指向不存在的 upstream 组
        let ups = vec![UpstreamSpec {
            name: "ok".into(),
            servers_ext: vec![sv("127.0.0.1:1", 0)],
            ..Default::default()
        }];
        let locs = vec![LocationSpec {
            path: "/".into(),
            kind: "proxy".into(),
            target: "missing".into(),
            code: 0,
            ws: false,
            ..Default::default()
        }];
        assert!(validate_vhost_cfg("proxy", "none", "", false, None, &ups, &locs).is_err());
        // proxy_pass 注入分号/花括号
        let locs2 = vec![LocationSpec {
            path: "/".into(),
            kind: "proxy".into(),
            target: "ok; #x".into(),
            code: 0,
            ws: false,
            ..Default::default()
        }];
        assert!(validate_vhost_cfg("proxy", "none", "", false, None, &ups, &locs2).is_err());
        // alias 越出站点目录
        let locs3 = vec![LocationSpec {
            path: "/x".into(),
            kind: "alias".into(),
            target: "/etc".into(),
            code: 0,
            ws: false,
            ..Default::default()
        }];
        assert!(validate_vhost_cfg("php", "none", "", false, Some(&tmp), &[], &locs3).is_err());
        // 自定义伪静态花括号不配对
        assert!(
            validate_vhost_cfg(
                "php",
                "custom",
                "if (x) { rewrite ^ y last;",
                false,
                Some(&tmp),
                &[],
                &[]
            )
            .is_err()
        );
        // 自定义伪静态禁止 location
        assert!(
            validate_vhost_cfg(
                "php",
                "custom",
                "location /x {}",
                false,
                Some(&tmp),
                &[],
                &[]
            )
            .is_err()
        );
        // deny 状态码白名单
        let locs4 = vec![LocationSpec {
            path: "/".into(),
            kind: "deny".into(),
            target: String::new(),
            code: 500,
            ws: false,
            ..Default::default()
        }];
        assert!(validate_vhost_cfg("php", "none", "", false, Some(&tmp), &[], &locs4).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn path_under_rejects_escape_and_accepts_subdir() {
        let base = Path::new("/home/u/www");
        assert!(path_under(base, Path::new("/home/u/www/blog")));
        assert!(path_under(base, Path::new("/home/u/www")));
        assert!(
            !path_under(base, Path::new("/home/u/www2/blog")),
            "同前缀不同目录应拒绝"
        );
        assert!(
            !path_under(base, Path::new("/home/u/www/../etc")),
            ".. 应拒绝"
        );
        assert!(!path_under(base, Path::new("/etc")), "站外路径应拒绝");
    }

    #[test]
    fn sanitize_tls_values_filters_injection() {
        // 协议：白名单交集 + 去重，非法 token（含注入片段）被剔除
        assert_eq!(
            sanitize_protocols("TLSv1.2 TLSv1.3 TLSv1.1 TLSv1.2"),
            "TLSv1.2 TLSv1.3 TLSv1.1"
        );
        assert_eq!(sanitize_protocols("TLSv1"), "");
        // 非白名单 token（TLSv1.2;、server_name）整体剔除，不能用于注入指令
        assert_eq!(
            sanitize_protocols("TLSv1.2;\n    server_name evil.com;"),
            ""
        );
        // 套件：非法字符（; # 引号 换行 花括号 斜杠）被剔除，不残留截断字符
        let r = sanitize_ciphers("ECDHE-RSA-AES128-GCM-SHA256:!aNULL;#x\n\"'{} /\\");
        assert!(r.starts_with("ECDHE-RSA-AES128-GCM-SHA256:!aNULL"));
        for bad in [';', '#', '"', '\n', '{', '}', '/', '\\'] {
            assert!(!r.contains(bad), "套件过滤后不应包含 {bad:?}");
        }
        assert_eq!(sanitize_ciphers(""), "");
    }

    #[test]
    fn render_ssl_emits_tls_advanced_directives() {
        let cfg = SslTlsCfg {
            protocols: "TLSv1.3 TLSv1.2".into(),
            ciphers: "ECDHE-RSA-AES128-GCM-SHA256:!aNULL".into(),
            prefer_server_ciphers: true,
            http2: true,
            http2_on_syntax: true, // nginx ≥ 1.25.1
        };
        let s = render_vhost_full(VhostRenderSpec {
            site_id: 9,
            name: "ssl",
            domains: &["ssl.com".into()],
            root: "/home/u/www/ssl-9",
            php_socket: None,
            access_log: None,
            error_log: None,
            site_type: "php",
            pseudo_static: "none",
            pseudo_custom: "",
            upstreams: &[],
            locations: &[],
            ssl_files: Some(("/etc/zap/ssl/fullchain.pem", "/etc/zap/ssl/key.pem")),
            force_https: false,
            ssl_tls: Some(&cfg),
            listen_ipv4: "",
            listen_ipv6: "",
        });
        assert!(
            s.contains("listen 443 ssl;"),
            "新版 nginx 用 http2 on 而非 listen 内嵌"
        );
        assert!(s.contains("http2 on;"));
        assert!(s.contains("ssl_certificate /etc/zap/ssl/fullchain.pem;"));
        assert!(s.contains("ssl_protocols TLSv1.3 TLSv1.2;"));
        assert!(s.contains("ssl_ciphers ECDHE-RSA-AES128-GCM-SHA256:!aNULL;"));
        assert!(s.contains("ssl_prefer_server_ciphers on;"));

        // 老版 nginx（< 1.25.1）：http2 内嵌到 listen；空协议/套件回退或省略
        let cfg2 = SslTlsCfg {
            protocols: String::new(),
            ciphers: String::new(),
            prefer_server_ciphers: false,
            http2: true,
            http2_on_syntax: false,
        };
        let s2 = render_vhost_full(VhostRenderSpec {
            site_id: 10,
            name: "legacy",
            domains: &["old.com".into()],
            root: "/home/u/www/legacy-10",
            php_socket: None,
            access_log: None,
            error_log: None,
            site_type: "static",
            pseudo_static: "none",
            pseudo_custom: "",
            upstreams: &[],
            locations: &[],
            ssl_files: Some(("/a/fullchain.pem", "/a/key.pem")),
            force_https: false,
            ssl_tls: Some(&cfg2),
            listen_ipv4: "",
            listen_ipv6: "",
        });
        assert!(
            s2.contains("listen 443 ssl http2;"),
            "老版 nginx 应内嵌 http2 到 listen"
        );
        assert!(!s2.contains("http2 on;"), "老版 nginx 不支持 http2 on 指令");
        assert!(
            s2.contains("ssl_protocols TLSv1.2 TLSv1.3;"),
            "协议缺省回退 TLSv1.2 TLSv1.3"
        );
        assert!(!s2.contains("ssl_ciphers"), "套件为空不输出 ssl_ciphers");
        assert!(
            !s2.contains("ssl_prefer_server_ciphers"),
            "未开启不输出 prefer 指令"
        );
    }
}
