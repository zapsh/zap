//! Web 应用挂载：`/webapps/*`。
//!
//! 当前提供 phpMyAdmin：程序本体由应用商店安装到 `/usr/local/apps/phpmyadmin`，
//! 这里不再单独占用端口，而是由 zapd 直接充当 FastCGI 客户端，
//! 把请求转交给系统 PHP-FPM 执行 —— 复用面板自身的 HTTPS、登录态与权限体系。
//!
//! 与 `/api/*` 不同，这些是**页面级**路由（不挂 `access::guard`），
//! 因此 handler 主动调用 `access::authorize_page` 走同一张规则表与权限点。

use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{OriginalUri, Request};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;

use crate::routers::auth::SESSION_COOKIE;
use crate::zap::fastcgi;
use crate::zap::jwt;

/// phpMyAdmin 的挂载路径（URL 空间）。
const PMA_MOUNT: &str = "/webapps/phpmyadmin";
/// phpMyAdmin 程序目录（应用商店安装后的软链）。
const PMA_ROOT: &str = "/usr/local/apps/phpmyadmin";
/// 请求体上限（导入 SQL 用；与 php.ini 的 upload_max_filesize 取小者生效）。
const MAX_BODY: usize = 64 * 1024 * 1024;

pub fn routers() -> Router {
    Router::new()
        .route("/phpmyadmin", any(phpmyadmin))
        .route("/phpmyadmin/", any(phpmyadmin))
        .route("/phpmyadmin/{*rest}", any(phpmyadmin))
}

async fn phpmyadmin(OriginalUri(original): OriginalUri, req: Request) -> Response {
    match handle(&original, req).await {
        Ok(resp) => resp,
        Err((code, msg)) => (code, msg).into_response(),
    }
}

async fn handle(original: &Uri, req: Request) -> Result<Response, (StatusCode, String)> {
    let method = req.method().clone();

    // ── 1) 路径切分：base（URL 前缀）+ rest（应用内路径）──────
    // 注意：`nest("/webapps")` 会把子路由看到的 URI 重写为应用内路径，
    // 因此这里必须用 OriginalUri（浏览器原始路径）来定位挂载点。
    let full = original.path().to_string();
    let Some(idx) = full.find(PMA_MOUNT) else {
        return Err((StatusCode::NOT_FOUND, "路径错误".to_string()));
    };
    let base = full[..idx].to_string(); // 如 ""（无前缀）或 "/zap"
    let rest = full[idx + PMA_MOUNT.len()..]
        .trim_start_matches('/')
        .to_string();

    // ── 1.1) 规范地址：`/webapps/phpmyadmin` → `/webapps/phpmyadmin/` ──
    // phpMyAdmin 输出的静态资源、表单 action 都是**相对路径**（其 Scripts::getDisplay()
    // 里 `base_dir` 只对子目录脚本非空），只有当页面 URL 以 `/` 结尾时，
    // `js/dist/console.js` 才会解析到 `/webapps/phpmyadmin/js/...`。
    // 若以不带斜杠的地址进入（如应用商店 info.yaml 里的 web_url、安装日志里的访问地址），
    // 浏览器会请求成 `/webapps/js/dist/console.js` → 一片 404 且页面无样式。
    // 这里统一 308 跳到带斜杠的规范地址，并保留查询串（便于携带 ?token=）。
    if rest.is_empty() && !full.ends_with('/') {
        let mut location = format!("{base}{PMA_MOUNT}/");
        if let Some(q) = original.query() {
            location.push('?');
            location.push_str(q);
        }
        return Response::builder()
            .status(StatusCode::PERMANENT_REDIRECT)
            .header(header::LOCATION, location)
            .body(Body::empty())
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "重定向失败".to_string()));
    }

    // ── 2) 登录态：Bearer / 会话 Cookie / ?token= ─────────────
    // 注意：HTTP/2 下浏览器不发 `Host` 头（用 :authority 伪头），
    // 因此优先取 URI 的 authority，Host 头仅作回退。
    let host = original
        .authority()
        .map(|a| a.as_str().to_string())
        .or_else(|| {
            req.headers()
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "(未知)".to_string());
    // 同名 Cookie 可能因 Path 不同而重复（历史遗留 + 新会话），浏览器会一并发出。
    // 逐个验证并取第一个有效的，避免被先出现的失效值误判为未登录。
    let candidates = crate::routers::access::token_candidates_from_page_request(&req);
    let token = candidates.first().cloned();
    let mut claims = None;
    for t in &candidates {
        if let Some(c) = jwt::claims_from_token(t).await {
            claims = Some(c);
            break;
        }
    }
    let Some(claims) = claims else {
        // 区分「没带 Cookie」「带了但没有 zap_token」「带了但失效」三种情况
        let reason = match token {
            Some(_) => {
                "请求已携带 zap_token，但令牌无效或已过期 —— 请回到面板重新登录一次".to_string()
            }
            None => match cookie_names(&req) {
                None => "请求完全没有 Cookie 头 —— 请确认登录地址与当前访问地址在同一个域名/IP 下"
                    .to_string(),
                Some(names) if names.is_empty() => {
                    "请求带了一个空的 Cookie 头 —— 请在面板重新登录一次".to_string()
                }
                Some(names) => format!(
                    "请求带了 Cookie，但其中没有 zap_token（实际收到：{}）。\
                     多为浏览器把旧会话清掉了或 host 不一致，请在面板重新登录一次",
                    names.join(", ")
                ),
            },
        };
        return Ok(login_required_page(
            &base,
            &full,
            &reason,
            &host,
            &raw_cookie(&req),
        ));
    };

    // ── 3) 角色 + 权限点（webapp.phpmyadmin:view）────────────
    crate::routers::access::authorize_page(&claims, PMA_MOUNT, &method).await?;

    // 会话续期：每次成功访问都重新签发一个长有效期 JWT 并写回 Cookie（滑动窗口）。
    // 否则 Cookie 只在登录那一刻下发，有效期一过（浏览器不再发送）就会被提示需要登录。
    let renewed = jwt::generate_jwt_token_with_expire(
        claims.sub.clone(),
        claims.id,
        &claims.roles,
        false,
        crate::routers::auth::WEBAPP_SESSION_SECS,
    )
    .ok();

    // ── 4) 起：程序目录 / 文件解析 / 静态直出或 FastCGI 转发 ──
    let outcome = serve_pma(original, req, &base, &rest, &method).await;

    // 成功与失败都要写回续期 Cookie：
    // 否则「首次用 ?token= 进入，但程序未安装 / FPM 未启动」这类场景
    // 不会建立会话，用户去掉 token 直连时又会回到「未携带 Cookie」。
    let resp = match outcome {
        Ok(r) => r,
        Err((code, msg)) => (code, msg).into_response(),
    };
    Ok(with_session(resp, renewed.as_deref()))
}

/// 鉴权通过后的实际处理：定位程序目录与目标文件，静态直出或转交 PHP-FPM。
async fn serve_pma(
    original: &Uri,
    req: Request,
    base: &str,
    rest: &str,
    method: &Method,
) -> Result<Response, (StatusCode, String)> {
    let root = PathBuf::from(PMA_ROOT);
    if !root.is_dir() {
        return Err((
            StatusCode::NOT_FOUND,
            "未找到 phpMyAdmin 程序目录 /usr/local/apps/phpmyadmin；\
             请先在「应用商店 → Web 应用程序」安装 phpMyAdmin。"
                .to_string(),
        ));
    }
    let root_real = root.canonicalize().unwrap_or_else(|_| root.clone());

    // 目标文件（防目录穿越）
    let rel = if rest.is_empty() {
        "index.php".to_string()
    } else {
        rest.to_string()
    };
    let target = root.join(&rel);
    let Ok(target_real) = target.canonicalize() else {
        return Err((StatusCode::NOT_FOUND, format!("文件不存在: {rel}")));
    };
    if !target_real.starts_with(&root_real) {
        return Err((StatusCode::FORBIDDEN, "非法的文件路径".to_string()));
    }
    // 目录 → 其下 index.php
    let script_real = if target_real.is_dir() {
        let p = target_real.join("index.php");
        if !p.is_file() {
            return Err((StatusCode::FORBIDDEN, "禁止列目录".to_string()));
        }
        p
    } else {
        target_real.clone()
    };

    // 静态资源直出（不经 FPM，更快也更省资源）
    if !script_real
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e == "php")
        .unwrap_or(false)
        && let Ok(resp) = serve_static(&script_real).await
    {
        return Ok(resp);
    }

    // 读取请求体
    let (parts, body) = req.into_parts();
    let body: Bytes = axum::body::to_bytes(body, MAX_BODY)
        .await
        .map_err(|_| (StatusCode::BAD_REQUEST, "读取请求体失败".to_string()))?;

    // 组装 FastCGI 参数并执行
    let script_url = format!(
        "{base}{PMA_MOUNT}/{}",
        script_real
            .strip_prefix(&root_real)
            .unwrap_or(&script_real)
            .to_string_lossy()
    );
    let socket = resolve_fpm_socket().await?;
    let params = build_params(
        &parts.headers,
        original,
        method,
        &script_url,
        &script_real,
        &root_real,
        &body,
    );

    let fcgi = fastcgi::request(&socket, &params, &body)
        .await
        .map_err(|e| {
            tracing::warn!("phpMyAdmin FastCGI 请求失败: {e}");
            (StatusCode::BAD_GATEWAY, format!("PHP-FPM 执行失败: {e}"))
        })?;

    build_response(fcgi)
}

/// 把续期后的会话 Cookie 写回响应（滑动窗口）。
fn with_session(resp: Response, token: Option<&str>) -> Response {
    let Some(token) = token else { return resp };
    let (mut parts, body) = resp.into_parts();
    // 必须用 append，不能用 extend：HeaderMap::extend 对同名键是「替换」，
    // 写入 zap_token 时会把 Web 应用（phpMyAdmin 等）自己的 Set-Cookie 全部清掉，
    // 表现为会话永远建立不起来（Failed to set session cookie）。
    if let Some(value) = crate::routers::auth::session_cookie(token).get(header::SET_COOKIE) {
        parts.headers.append(header::SET_COOKIE, value.clone());
    }
    Response::from_parts(parts, body)
}

/// 未登录提示页（401）。
///
/// 这里**不 302 到 SPA 登录页**：新标签页直接打开时，前端会立刻发出 `/api/*`
/// 请求并弹出「登录已过期」的误报提示。独立提示页信息明确，
/// 同时给出「前往登录（带回跳）」与「返回面板首页」两个出口。
const LOGIN_REQUIRED_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>需要登录 - Zap</title>
<style>
  body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;
       font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,"PingFang SC","Microsoft YaHei",Arial,sans-serif;
       background:#f5f7fa;color:#303133}
  .card{background:#fff;border:1px solid #e4e7ed;border-radius:8px;padding:32px 40px;
        max-width:560px;box-shadow:0 2px 12px rgba(0,0,0,.06)}
  h1{margin:0 0 12px;font-size:18px}
  p{margin:0 0 20px;line-height:1.8;color:#606266}
  code{background:#f0f2f5;padding:1px 5px;border-radius:3px}
  .reason{margin:0 0 20px;padding:10px 12px;border-left:3px solid #e6a23c;
          background:#fdf6ec;color:#a3722c;font-size:13px}
  a{display:inline-block;padding:8px 18px;background:#409eff;color:#fff;
    border-radius:4px;text-decoration:none;font-size:14px}
  a.home{margin-left:8px;background:#fff;color:#409eff;border:1px solid #409eff}
  @media (prefers-color-scheme: dark){
    body{background:#1a1a1a;color:#e5eaf3}
    .card{background:#242424;border-color:#3a3a3a}
    p{color:#a8abb2}
    code{background:#333}
  }
</style>
</head>
<body>
<div class="card">
  <h1>需要登录</h1>
  <p>
    请先在 Zap 面板登录后，再访问 <code>__PATH__</code>。<br>
    若面板中已登录仍看到本页，请确认打开的是<strong>同一个地址</strong>
    —— 会话 Cookie 按域名 / IP 隔离（端口不影响）。<br>
    若 Cookie 列表中已存在 <code>zap_token</code>，请检查其 <strong>Expires</strong>
    是否已过期——过期的 Cookie 浏览器不会再发送（重新登录面板即可刷新）。
  </p>
  <p class="reason">诊断：__REASON__</p>
  <p class="reason">服务端当前时间：__NOWTIME__（UTC）—— 请与 Cookie 的 Expires 对比</p>
  <p class="reason">本次请求 Host：__HOST__（HTTP/2 下通常取不到，属正常现象）</p>
  <p class="reason">服务端实际收到的 Cookie 原文：__RAWCOOKIE__</p>
  <a href="__BACK__">前往登录</a>
  <a class="home" href="__HOME__">返回面板首页</a>
</div>
</body>
</html>
"#;

/// 未登录：返回独立提示页（401，含诊断原因、前往登录与返回首页入口）。
fn login_required_page(
    base: &str,
    full: &str,
    reason: &str,
    host: &str,
    raw_cookie: &str,
) -> Response {
    let back = format!("{base}/login?redirect={}", urlencode(full));
    let home = if base.is_empty() {
        "/".to_string()
    } else {
        format!("{base}/")
    };
    let html = LOGIN_REQUIRED_HTML
        .replace("__PATH__", &escape_html(full))
        .replace("__BACK__", &escape_html(&back))
        .replace("__HOME__", &escape_html(&home))
        .replace("__REASON__", &escape_html(reason))
        .replace(
            "__NOWTIME__",
            &chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        )
        .replace("__HOST__", &escape_html(host))
        .replace("__RAWCOOKIE__", &escape_html(raw_cookie));

    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(html))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// 服务端实际收到的 Cookie 头原文（仅用于诊断页展示；没有则给出占位说明）。
fn raw_cookie(req: &Request) -> String {
    crate::routers::access::all_cookies(req.headers())
        .unwrap_or_else(|| "(没有 Cookie 头)".to_string())
}

/// 取请求 Cookie 头中的键名列表（不含值，仅用于诊断展示）。
fn cookie_names(req: &Request) -> Option<Vec<String>> {
    let raw = crate::routers::access::all_cookies(req.headers())?;
    let names = raw
        .split(';')
        .filter_map(|p| p.split_once('=').map(|(k, _)| k.trim().to_string()))
        .filter(|k| !k.is_empty())
        .collect::<Vec<_>>();
    Some(names)
}

/// HTML 文本转义（路径来自请求，必须转义后再嵌入页面）。
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// 极简 URL 编码：仅用于把回跳路径放进查询串。
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 静态文件直出（js / css / 图片 / 字体等）。
async fn serve_static(path: &Path) -> Result<Response, (StatusCode, String)> {
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("读取文件失败: {e}")))?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut resp = Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(Body::from(data))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // 静态资源可缓存（带 etag 性质的最后修改时间即可）
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    Ok(resp)
}

/// 定位系统 PHP-FPM 通道。
///
/// **只认「系统默认 pool」的 socket**（`php-fpm-{ver}.sock`，如 `/run/php-fpm-74.sock`，
/// 属主 `www`、0666），**排除用户专属 pool**（`php-fpm-{linux_user}-{ver}.sock`）。
/// 后者是按面板用户隔离的私有通道 —— `pool_sync` 生成时写死了
/// `listen.owner = {linux_user}` / `listen.group = www` / `listen.mode = 0660`：
/// - 面板进程既不是该 Linux 账号、也不在 `www` 组里，`connect` 必然
///   `Permission denied (os error 13)`；
/// - 就算把面板账号塞进 `www` 组勉强连上，phpMyAdmin 作为系统级入口也不该跑在某个用户的
///   pool 里（那份 pool 的 worker 身份、`open_basedir`、session 目录都是按该用户裁剪的）。
///
/// 挑选顺序：
/// 1) 面板默认 PHP（conf 区 `php_default`）对应的 socket；
/// 2) 其余系统默认 socket（版本号降序，较新的 PHP 优先）；
/// 3) 逐个**实际试探连接**，取第一个真正连得上的候选 —— 权限/属组问题自动跳过；
///    全部失败时把候选连同失败原因一并返回，避免只剩一句 Permission denied 无从下手。
async fn resolve_fpm_socket() -> Result<String, (StatusCode, String)> {
    let prefer = crate::zap::server_env::conf_get("php_default").unwrap_or_default();
    let prefer_ver = normalize_ver(&prefer);

    let mut preferred: Vec<String> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    for dir in ["/run", "/var/run"] {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            // 非「系统默认 pool」：用户 pool 单独记录，便于报错时说明为什么不用它
            let Some(ver) = system_pool_ver(&name) else {
                if name.starts_with("php-fpm")
                    && name.ends_with(".sock")
                    && !excluded.contains(&name)
                {
                    excluded.push(name);
                }
                continue;
            };
            let is_socket = std::fs::metadata(e.path())
                .map(|m| m.file_type().is_socket())
                .unwrap_or(false);
            if !is_socket {
                continue;
            }
            // /run 与 /var/run 通常指向同一处，按真实路径去重
            let real = std::fs::canonicalize(e.path()).unwrap_or_else(|_| e.path());
            if seen.contains(&real) {
                continue;
            }
            seen.push(real);

            let path = e.path().to_string_lossy().into_owned();
            if !prefer_ver.is_empty() && ver == prefer_ver {
                preferred.push(path);
            } else {
                others.push(path);
            }
        }
    }

    // 版本号降序：未显式指定时优先较新的 PHP
    others.sort_by_key(|a| std::cmp::Reverse(socket_ver_key(a)));
    let candidates: Vec<String> = preferred.into_iter().chain(others).collect();

    let mut tried = Vec::new();
    for path in &candidates {
        match tokio::net::UnixStream::connect(path).await {
            Ok(_) => return Ok(path.clone()),
            Err(e) => tried.push(format!("{path}（{e}）")),
        }
    }

    let msg = if candidates.is_empty() {
        let mut m = "未找到可用的系统 PHP-FPM socket（/run/php-fpm-{版本}.sock）".to_string();
        if !excluded.is_empty() {
            m.push_str("；以下为用户专属 pool，phpMyAdmin 不使用：");
            m.push_str(&excluded.join("、"));
        }
        m.push_str("。请先在应用商店安装 PHP 并启动 FPM 服务。");
        m
    } else {
        format!(
            "所有系统 PHP-FPM socket 均无法连接：{}。\
             请确认 FPM 已在运行、且面板运行账号有该 socket 的读写权限。",
            tried.join("；")
        )
    };
    Err((StatusCode::SERVICE_UNAVAILABLE, msg))
}

/// 判断 socket 文件名是否属于「系统默认 pool」，是则返回归一化后的版本号。
///
/// - 系统默认：`php-fpm-{ver}.sock`；
/// - 用户 pool：`php-fpm-{linux_user}-{ver}.sock`（版本号前面多一段 Linux 账号名）。
fn system_pool_ver(name: &str) -> Option<String> {
    let rest = name.strip_prefix("php-fpm-")?.strip_suffix(".sock")?;
    if rest.is_empty() || rest.contains('-') {
        return None;
    }
    Some(normalize_ver(rest))
}

/// 版本号归一化：`8.3` / `83` / `php83` 统一为 `83`，便于互相比较。
fn normalize_ver(v: &str) -> String {
    v.trim_start_matches("php").replace('.', "")
}

/// 从 socket 路径取版本号数值用于排序：`/run/php-fpm-8.3.sock` → 83。
fn socket_ver_key(path: &str) -> u64 {
    let name = path.rsplit('/').next().unwrap_or(path);
    system_pool_ver(name)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
}

/// 构造 CGI/FastCGI 参数。
///
/// 安全：不向 PHP 透传 `Authorization`（面板 JWT），
/// 并从 Cookie 中剔除面板会话 Cookie —— 避免面板凭据泄露给 Web 应用。
fn build_params(
    headers: &HeaderMap,
    uri: &Uri,
    method: &Method,
    script_url: &str,
    script_real: &Path,
    root_real: &Path,
    body: &Bytes,
) -> Vec<(String, String)> {
    let query = uri.query().unwrap_or_default().to_string();
    let request_uri = match uri.query() {
        Some(q) => format!("{}?{q}", uri.path()),
        None => uri.path().to_string(),
    };
    // HTTP/2 下浏览器不发 `Host` 头（改用 :authority 伪头），
    // 因此优先取 URI 的 authority，Host 头仅作回退。
    // 否则 PHP 会拿到 localhost:443，据此生成的链接与 Cookie 域全部对不上。
    let host = uri
        .authority()
        .map(|a| a.as_str().to_string())
        .or_else(|| {
            headers
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "localhost".to_string());
    let server_port = host.rsplit_once(':').map(|(_, p)| p.to_string());
    let server_name = host
        .rsplit_once(':')
        .map(|(h, _)| h.to_string())
        .unwrap_or_else(|| host.clone());
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let mut params: Vec<(String, String)> = vec![
        ("GATEWAY_INTERFACE".into(), "CGI/1.1".into()),
        ("SERVER_SOFTWARE".into(), "Zap".into()),
        ("SERVER_PROTOCOL".into(), "HTTP/1.1".into()),
        ("REQUEST_METHOD".into(), method.to_string()),
        ("REQUEST_URI".into(), request_uri),
        ("QUERY_STRING".into(), query),
        ("SCRIPT_NAME".into(), script_url.to_string()),
        (
            "SCRIPT_FILENAME".into(),
            script_real.to_string_lossy().into_owned(),
        ),
        (
            "DOCUMENT_ROOT".into(),
            root_real.to_string_lossy().into_owned(),
        ),
        ("DOCUMENT_URI".into(), script_url.to_string()),
        ("PATH_INFO".into(), String::new()),
        ("CONTENT_TYPE".into(), content_type),
        ("CONTENT_LENGTH".into(), body.len().to_string()),
        ("SERVER_NAME".into(), server_name),
        (
            "SERVER_PORT".into(),
            server_port.clone().unwrap_or_else(|| "443".into()),
        ),
        // 必须与客户端实际协议保持一致，否则 phpMyAdmin 会报
        // “mismatch between HTTPS indicated on server and client”。
        // 代价：phpMyAdmin 会改用 `__Secure-` 前缀的会话 Cookie，
        // 该前缀要求浏览器把站点视为「安全来源」，自签证书需先导入信任库。
        ("REQUEST_SCHEME".into(), "https".into()),
        ("HTTPS".into(), "on".into()),
    ];

    // HTTP/2 下没有 Host 头，这里给 PHP 补一个 HTTP_HOST
    // （PHP 程序普遍用它判断域名，缺失会导致生成的链接/Cookie 域出错）
    if !headers.contains_key(header::HOST) {
        params.push(("HTTP_HOST".into(), host.clone()));
    }

    // 标准反向代理头：告知后端应用真实的协议与端口（PHP 侧为
    // $_SERVER['HTTP_X_FORWARDED_PROTO'] / ['HTTP_X_FORWARDED_PORT']）。
    // 客户端自带同名头时不覆盖，避免被伪造。
    if !headers.contains_key("x-forwarded-proto") {
        params.push(("HTTP_X_FORWARDED_PROTO".into(), "https".into()));
    }
    if !headers.contains_key("x-forwarded-port")
        && let Some(p) = server_port.as_ref()
    {
        params.push(("HTTP_X_FORWARDED_PORT".into(), p.clone()));
    }

    // HTTP_*：除 Authorization 外逐头透传；Cookie 需剔除面板会话 Cookie
    let cookie = filtered_cookie(headers);
    for (name, value) in headers.iter() {
        let Ok(v) = value.to_str() else { continue };
        // 面板 JWT（Authorization）与会话 Cookie 不下发给 Web 应用
        if name == header::AUTHORIZATION {
            continue;
        }
        if name == header::COOKIE {
            if let Some(c) = &cookie {
                params.push(("HTTP_COOKIE".into(), c.clone()));
            }
            continue;
        }
        let key = name.as_str().to_uppercase().replace('-', "_");
        params.push((format!("HTTP_{key}"), v.to_string()));
    }

    params
}

/// 去掉面板会话 Cookie 后的 Cookie 头（其余原样透传给 PHP）。
fn filtered_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = crate::routers::access::all_cookies(headers)?;
    let prefix = format!("{SESSION_COOKIE}=");
    let kept: Vec<&str> = raw
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with(&prefix))
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join("; "))
    }
}

/// FastCGI 响应 → axum 响应（跳过 hop-by-hop 头与非法的头名/值）。
fn build_response(fcgi: fastcgi::FcgiResponse) -> Result<Response, (StatusCode, String)> {
    let mut builder = Response::builder().status(fcgi.status);
    for (k, v) in fcgi.headers {
        if k.eq_ignore_ascii_case("status")
            || k.eq_ignore_ascii_case("connection")
            || k.eq_ignore_ascii_case("transfer-encoding")
        {
            continue;
        }
        let Ok(name) = HeaderName::from_bytes(k.as_bytes()) else {
            tracing::warn!("丢弃 FastCGI 响应头：头名非法 {k:?}");
            continue;
        };
        let Ok(value) = HeaderValue::from_str(&v) else {
            // Set-Cookie 里是会话令牌，只记长度不记内容，避免写进日志
            if name == header::SET_COOKIE {
                tracing::warn!(
                    "丢弃 FastCGI 响应头：Set-Cookie 的值非法（长度 {}）",
                    v.len()
                );
            } else {
                tracing::warn!("丢弃 FastCGI 响应头：{k} 的值非法（长度 {}）", v.len());
            }
            continue;
        };
        // header() 是追加语义，多个 Set-Cookie 会各自保留，不会互相覆盖
        builder = builder.header(name, value);
    }
    builder
        .body(Body::from(fcgi.body))
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("构造响应失败: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_system_pool_is_picked() {
        // 系统默认 pool：版本号紧跟前缀，中间没有额外 '-'
        assert_eq!(system_pool_ver("php-fpm-74.sock"), Some("74".into()));
        assert_eq!(system_pool_ver("php-fpm-8.3.sock"), Some("83".into()));
        // 用户专属 pool（`{linux_user}-{ver}`）必须排除：连上去必然 Permission denied
        assert_eq!(system_pool_ver("php-fpm-admin-74.sock"), None);
        assert_eq!(system_pool_ver("php-fpm-zap_user-8.3.sock"), None);
        // 无关文件
        assert_eq!(system_pool_ver("php-fpm.conf"), None);
        assert_eq!(system_pool_ver("php-fpm-.sock"), None);
        assert_eq!(system_pool_ver("nginx.sock"), None);
    }

    #[test]
    fn version_key_puts_newer_first() {
        assert_eq!(socket_ver_key("/run/php-fpm-8.3.sock"), 83);
        assert!(socket_ver_key("/run/php-fpm-8.3.sock") > socket_ver_key("/run/php-fpm-74.sock"));
        // 版本号无法解析的排最后
        assert_eq!(socket_ver_key("/run/php-fpm-.sock"), 0);
    }

    #[test]
    fn version_forms_are_normalized() {
        assert_eq!(normalize_ver("8.3"), "83");
        assert_eq!(normalize_ver("php83"), "83");
        assert_eq!(normalize_ver("74"), "74");
    }
}
