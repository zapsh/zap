//! 四层转发（Nginx stream）——**仅管理员**。
//!
//! TCP / UDP 端口转发：把宿主机某个端口接到任意后端（数据库、游戏服、内网服务…）。
//! 规则存在面板库里，保存即渲染 `zap-stream.conf` 并由 zapexec 走
//! 「写盘 → `nginx -t` → 失败回滚 → 重载」，主配置缺 `include` 时自动补一行。
//!
//! 为什么限管理员：一条规则就是「把机器上任意端口接到任意地址」，
//! 既能绕过防火墙暴露内网服务，也能抢占其它服务的端口 —— 不当作普通能力开放。
//!
//! 端点：
//! - GET  /system/stream/status  能力探测（是否支持 stream / 是否已 include）
//! - GET  /system/stream/list    规则列表
//! - POST /system/stream/add     新增
//! - POST /system/stream/update  修改
//! - POST /system/stream/delete  删除
//! - POST /system/stream/apply   按当前库里规则重新渲染并生效

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Json;
use axum::extract::Extension;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db;
use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;
use zap_proto::Request;

#[derive(sqlx::FromRow, Debug, Clone)]
struct StreamRow {
    id: i64,
    name: String,
    listen_ip: String,
    listen_port: i64,
    protocol: String,
    target_host: String,
    target_port: i64,
    mode: String,
    raw: String,
    backend_mode: String,
    targets: String,
    listen_opts: String,
    proxy_connect_timeout: String,
    proxy_timeout: String,
    proxy_responses: i64,
    ssl_enable: i32,
    ssl_certificate: String,
    ssl_certificate_key: String,
    ssl_protocols: String,
    ssl_ciphers: String,
    ssl_certificate_id: i64,
    ssl_preread: i32,
    proxy_pass: String,
    extra: String,
    remark: String,
    status: i32,
    created_at: i64,
    updated_at: i64,
}

const COLS: &str = "id, name, listen_ip, listen_port, protocol, target_host, target_port, \
                    mode, raw, backend_mode, targets, listen_opts, proxy_connect_timeout, \
                    proxy_timeout, proxy_responses, ssl_enable, ssl_certificate, \
                    ssl_certificate_key, ssl_protocols, ssl_ciphers, ssl_certificate_id, \
                    ssl_preread, proxy_pass, extra, \
                    remark, status, created_at, updated_at";

impl StreamRow {
    fn is_advanced(&self) -> bool {
        self.mode == "advanced"
    }

    /// 负载组模式：生成 `upstream { }` 并 proxy_pass 到组名。
    /// 单后端模式直接 `proxy_pass host:port;`，不再多一层 upstream。
    fn is_group(&self) -> bool {
        self.backend_mode != "single"
    }
}

fn require_admin(claims: &ValidatedClaims) -> Result<(), ZapError> {
    if is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "仅管理员可管理四层转发".to_string()))
    }
}

fn row_json(r: &StreamRow) -> Value {
    json!({
        "id": r.id,
        "name": r.name,
        "listen_ip": r.listen_ip,
        "listen_port": r.listen_port,
        "protocol": r.protocol,
        "target_host": r.target_host,
        "target_port": r.target_port,
        // 合并展示/回填用：`10.0.1.10:3306` 或 upstream 名
        "target": backend_target(r),
        // 负载组自动生成的 upstream 名（填了 proxy_pass 变量 / 高级模式时没有）
        "upstream": upstream_name(r),
        "mode": r.mode,
        "raw": r.raw,
        "backend_mode": r.backend_mode,
        "targets": r.targets,
        "listen_opts": r.listen_opts,
        "proxy_connect_timeout": r.proxy_connect_timeout,
        "proxy_timeout": r.proxy_timeout,
        "proxy_responses": r.proxy_responses,
        "ssl_enable": r.ssl_enable == 1,
        "ssl_certificate": r.ssl_certificate,
        "ssl_certificate_key": r.ssl_certificate_key,
        "ssl_protocols": r.ssl_protocols,
        "ssl_ciphers": r.ssl_ciphers,
        "ssl_certificate_id": r.ssl_certificate_id,
        "ssl_preread": r.ssl_preread == 1,
        "proxy_pass": r.proxy_pass,
        "extra": r.extra,
        "remark": r.remark,
        "status": r.status,
        "created_at": r.created_at,
        "updated_at": r.updated_at,
    })
}

// ── 校验 ────────────────────────────────────────────────────

/// 规则名：只是标识，但会进配置文件注释，不能带换行/引号。
fn validate_name(raw: &str) -> Result<String, ZapError> {
    let n = raw.trim();
    if n.is_empty() || n.chars().count() > 64 {
        return Err(ZapError::New(-1, "规则名不能为空且最长 64 个字符".to_string()));
    }
    if n.chars().any(|c| c.is_control() || c == '"' || c == '\'') {
        return Err(ZapError::New(-1, "规则名不能包含引号或控制字符".to_string()));
    }
    Ok(n.to_string())
}

/// 监听地址：留空当 `0.0.0.0`；否则必须是 IP 字面量（不接受域名，避免解析歧义）。
fn validate_listen_ip(raw: &str) -> Result<String, ZapError> {
    let v = raw.trim();
    if v.is_empty() {
        return Ok("0.0.0.0".to_string());
    }
    v.parse::<std::net::IpAddr>()
        .map_err(|_| ZapError::New(-1, format!("监听地址不是合法 IP：{v}")))?;
    Ok(v.to_string())
}

fn validate_port(v: i64, label: &str) -> Result<i64, ZapError> {
    if !(1..=65535).contains(&v) {
        return Err(ZapError::New(
            -1,
            format!("{label}必须在 1–65535 之间（收到 {v}）"),
        ));
    }
    Ok(v)
}

/// 后端地址：域名 / IP / `[IPv6]` 都收，但绝不收引号、空白与 `;` `{` 这类
/// 能改写 nginx 配置结构的字符。
fn validate_host(raw: &str) -> Result<String, ZapError> {
    let v = raw.trim();
    if v.is_empty() || v.chars().count() > 253 {
        return Err(ZapError::New(-1, "后端地址不能为空且最长 253 个字符".to_string()));
    }
    let ok = v.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']')
    });
    if !ok {
        return Err(ZapError::New(
            -1,
            "后端地址只能包含字母、数字、. - _ : [ ]（不支持带空格或其它符号）".to_string(),
        ));
    }
    Ok(v.to_string())
}

fn validate_protocol(raw: &str) -> Result<String, ZapError> {
    match raw.trim().to_lowercase().as_str() {
        "" | "tcp" => Ok("tcp".to_string()),
        "udp" => Ok("udp".to_string()),
        other => Err(ZapError::New(
            -1,
            format!("协议只支持 tcp / udp（收到：{other}）"),
        )),
    }
}

fn validate_mode(raw: &str) -> Result<String, ZapError> {
    match raw.trim().to_lowercase().as_str() {
        "" | "basic" => Ok("basic".to_string()),
        "advanced" => Ok("advanced".to_string()),
        other => Err(ZapError::New(
            -1,
            format!("模式只支持 basic / advanced（收到：{other}）"),
        )),
    }
}

/// 后端模式：`single` 直接转发到 `target_host:target_port`；`group` 生成 upstream 负载组。
fn validate_backend_mode(raw: &str) -> Result<String, ZapError> {
    match raw.trim().to_lowercase().as_str() {
        // 默认 group：与「upstream 是一组服务器」的语义一致
        "" | "group" | "upstream" => Ok("group".to_string()),
        "single" => Ok("single".to_string()),
        other => Err(ZapError::New(
            -1,
            format!("后端模式只支持 single / group（收到：{other}）"),
        )),
    }
}

/// 自定义片段长度上限（高级模式 / 全局片段）：防手滑贴进整本配置。
const MAX_FRAGMENT: usize = 16_000;

/// 自定义片段的通用检查：长度、控制字符（换行/制表符除外）、以及**不许再套
/// 一个 `stream { }`** —— 面板渲染时已经包了一层，再嵌套 `nginx -t` 必然失败。
fn validate_fragment(raw: &str, label: &str) -> Result<String, ZapError> {
    let v = raw.trim();
    if v.chars().count() > MAX_FRAGMENT {
        return Err(ZapError::New(
            -1,
            format!("{label}过长（上限 {MAX_FRAGMENT} 字符）"),
        ));
    }
    if v.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err(ZapError::New(
            -1,
            format!("{label}不能包含控制字符"),
        ));
    }
    let low = v.to_lowercase();
    if low.contains("stream") && low.contains("{") {
        // 只挡「stream {」这种嵌套块写法；`proxy_ssl_server_name` 之类不受影响
        let nested = low
            .split("stream")
            .skip(1)
            .any(|tail| tail.trim_start().starts_with('{'));
        if nested {
            return Err(ZapError::New(
                -1,
                format!("{label}里不要写 `stream {{ }}`：面板已包好 stream 块，直接写 upstream / map / server 即可"),
            ));
        }
    }
    Ok(raw.to_string())
}

/// server 块内的附加指令：只能是若干条指令行，不许带 `{ }`（那会破坏块结构）。
fn validate_extra(raw: &str) -> Result<String, ZapError> {
    let v = validate_fragment(raw, "自定义指令")?;
    if v.contains('{') || v.contains('}') {
        return Err(ZapError::New(
            -1,
            "自定义指令里不能出现 { 或 }（它会被插进 server { } 内部）".to_string(),
        ));
    }
    Ok(v)
}

/// 多后端：一行一个 `host:port[ 参数]`（参数如 `weight=2 max_fails=3 backup`）。
/// 空则回退到 target_host / target_port 单后端。
fn validate_targets(raw: &str) -> Result<String, ZapError> {
    let v = validate_fragment(raw, "后端列表")?;
    if v.trim().is_empty() {
        return Ok(String::new());
    }
    for line in v.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let addr = parts.next().unwrap_or_default();
        let (host, port) = split_host_port(addr)?;
        validate_host(host)?;
        validate_port(port, "后端端口")?;
        for p in parts {
            if !p
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '=' | '.' | '-'))
            {
                return Err(ZapError::New(
                    -1,
                    format!("后端参数不合法：{p}（只支持 weight= / max_fails= / backup 等）"),
                ));
            }
        }
    }
    Ok(v)
}

/// 拆 `host:port`：`[::1]:3306` 要按最后一个冒号拆，IPv6 里的冒号不能算。
fn split_host_port(addr: &str) -> Result<(&str, i64), ZapError> {
    let (host, port) = match addr.rsplit_once(':') {
        Some((h, p)) => (h, p),
        None => {
            return Err(ZapError::New(
                -1,
                format!("后端地址要写成 host:port（收到：{addr}）"),
            ));
        }
    };
    let port: i64 = port.parse().map_err(|_| {
        ZapError::New(-1, format!("后端端口不是数字：{port}（{addr}）"))
    })?;
    Ok((host, port))
}

/// listen 的附加参数：`reuseport` / `ssl` / `backlog=1024` / `so_keepalive=on` …
/// 只收这些字符；`udp` / `tcp` 由协议字段决定，重复写会被忽略。
fn validate_listen_opts(raw: &str) -> Result<String, ZapError> {
    let v = raw.trim();
    if v.is_empty() {
        return Ok(String::new());
    }
    for tok in v.split_whitespace() {
        if !tok
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '=' | '.' | '-'))
        {
            return Err(ZapError::New(
                -1,
                format!("listen 参数不合法：{tok}（如 reuseport / ssl / backlog=1024）"),
            ));
        }
    }
    Ok(v.to_string())
}

/// nginx 时间写法：`10s` / `5m` / `1h` / `500ms`；留空表示用默认。
fn validate_time_opt(raw: &str, label: &str) -> Result<String, ZapError> {
    let v = raw.trim();
    if v.is_empty() {
        return Ok(String::new());
    }
    let ok = (v.ends_with("ms") && v[..v.len() - 2].chars().all(|c| c.is_ascii_digit()))
        || (v.ends_with(|c: char| matches!(c, 's' | 'm' | 'h' | 'd'))
            && v[..v.len() - 1].chars().all(|c| c.is_ascii_digit()))
        || v.chars().all(|c| c.is_ascii_digit());
    if !ok {
        return Err(ZapError::New(
            -1,
            format!("{label}不合法：{v}（写成 5s / 1m / 1h / 500ms）"),
        ));
    }
    Ok(v.to_string())
}

/// 证书/私钥路径：必须绝对路径且不含引号与 `;` `{` `}`。
fn validate_cert_path(raw: &str, label: &str) -> Result<String, ZapError> {
    let v = raw.trim();
    if v.is_empty() {
        return Ok(String::new());
    }
    if !v.starts_with('/') || v.contains('"') || v.contains('\'') || v.contains(';')
        || v.contains('{') || v.contains('}')
    {
        return Err(ZapError::New(
            -1,
            format!("{label}必须是绝对路径，且不含引号与 ; {{ }}（收到：{v}）"),
        ));
    }
    Ok(v.to_string())
}

/// `ssl_protocols` / `ssl_ciphers`：前者是协议列表，后者是 OpenSSL 串，
/// 统一按「不含引号与 ; { }」来卡（ciphers 里的 `:` `!` `-` `+` 都要放行）。
fn validate_ssl_text(raw: &str, label: &str) -> Result<String, ZapError> {
    let v = raw.trim();
    if v.is_empty() {
        return Ok(String::new());
    }
    if v.contains('"') || v.contains('\'') || v.contains(';') || v.contains('{')
        || v.contains('}')
    {
        return Err(ZapError::New(
            -1,
            format!("{label}不能包含引号或 ; {{ }}（收到：{v}）"),
        ));
    }
    Ok(v.to_string())
}

/// `proxy_pass` 自定义目标：只允许 **变量**（如 `$backend`），配合全局片段里的
/// `map $ssl_preread_server_name $backend` 做 SNI 分流；留空则用本规则的 upstream。
fn validate_proxy_pass(raw: &str) -> Result<String, ZapError> {
    let v = raw.trim();
    if v.is_empty() {
        return Ok(String::new());
    }
    let Some(name) = v.strip_prefix('$') else {
        return Err(ZapError::New(
            -1,
            "自定义 proxy_pass 只能填变量（以 $ 开头，如 $backend）".to_string(),
        ));
    };
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(ZapError::New(
            -1,
            format!("变量名不合法：{v}（只能字母数字下划线）"),
        ));
    }
    Ok(v.to_string())
}

// ── 渲染 ────────────────────────────────────────────────────

/// 按规则渲染 `stream { }` 块。
///
/// 全局片段（global）先输出，规则随后 —— `map` / `resolver` / 公共 `upstream`
/// 要能被下面的 server 引用到，顺序不能反。
/// 没有任何启用规则且全局片段为空时返回空串（撤掉配置文件）。
fn render_conf(rows: &[StreamRow], global: &str) -> String {
    let active: Vec<&StreamRow> = rows.iter().filter(|r| r.status == 1).collect();
    let global = global.trim();
    if active.is_empty() && global.is_empty() {
        return String::new();
    }
    let mut out = String::from("# 由面板生成（四层转发），勿手工修改\nstream {\n");
    out.push_str(
        "    log_format zap_stream '$remote_addr [$time_local] $protocol $status '\n    \
         '\"$bytes_sent\" \"$bytes_received\" $session_time $upstream_addr';\n",
    );
    // 绝对路径：相对路径会按 nginx prefix 解析，很多机器没有 logs 目录
    // → open() 失败、配置起不来。目录由 zapexec 写配置时建好。
    // 跟 nginx 自己的日志放一起（/var/log/nginx），轮转和查看都在一起。
    out.push_str("    access_log /var/log/nginx/zap-stream.log zap_stream;\n");

    if !global.is_empty() {
        out.push_str("\n    # ── 全局自定义片段 ──\n");
        for line in global.lines() {
            if line.trim().is_empty() {
                out.push('\n');
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
    }

    for r in &active {
        out.push('\n');
        out.push_str(&render_rule(r));
    }
    out.push_str("}\n");
    out
}

/// 单条规则：advanced 直接吐 `raw`（stream 块内的任意配置），basic 按字段拼。
fn render_rule(r: &StreamRow) -> String {
    let mut out = format!("    # rule: {} (id={})\n", r.name, r.id);
    if r.is_advanced() {
        let raw = r.raw.trim();
        if raw.is_empty() {
            return out;
        }
        out.push_str(raw);
        out.push('\n');
        return out;
    }

    let udp = r.protocol == "udp";
    // 自定义 proxy_pass（SNI 分流用的变量）时不生成 upstream
    let custom_pass = !r.proxy_pass.trim().is_empty();
    let group = r.is_group();
    if group && !custom_pass {
        out.push_str(&format!("    upstream {} {{\n", upstream_name(r)));
        let servers = backend_servers(r);
        for s in servers {
            out.push_str(&format!("        server {s};\n"));
        }
        out.push_str("    }\n");
    }

    out.push_str("    server {\n");
    // listen：协议参数由 protocol 决定，opts 里再写一遍就跳过，避免重复
    let opts = r.listen_opts.trim();
    let mut listen = format!("{}:{}", r.listen_ip, r.listen_port);
    if udp && !opts.split_whitespace().any(|t| t == "udp") {
        listen.push_str(" udp");
    }
    if !opts.is_empty() {
        listen.push(' ');
        listen.push_str(opts);
    }
    out.push_str(&format!("        listen {listen};\n"));

    if r.ssl_enable == 1 {
        // 证书库优先：应用时已把 PEM 落盘成这两个文件
        let (cert, key) = if r.ssl_certificate_id > 0 {
            (
                Some(cert_file(r.id, "crt")),
                Some(cert_file(r.id, "key")),
            )
        } else {
            (
                (!r.ssl_certificate.is_empty()).then(|| r.ssl_certificate.clone()),
                (!r.ssl_certificate_key.is_empty()).then(|| r.ssl_certificate_key.clone()),
            )
        };
        if let Some(c) = cert {
            out.push_str(&format!("        ssl_certificate {c};\n"));
        }
        if let Some(k) = key {
            out.push_str(&format!("        ssl_certificate_key {k};\n"));
        }
        if !r.ssl_protocols.is_empty() {
            out.push_str(&format!("        ssl_protocols {};\n", r.ssl_protocols));
        }
        if !r.ssl_ciphers.is_empty() {
            out.push_str(&format!("        ssl_ciphers {};\n", r.ssl_ciphers));
        }
        out.push_str("        ssl_handshake_timeout 60s;\n");
    }
    if r.ssl_preread == 1 {
        out.push_str("        ssl_preread on;\n");
    }

    let target = if custom_pass {
        r.proxy_pass.trim().to_string()
    } else if group {
        format!("zap_stream_{}", r.id)
    } else {
        backend_target(r)
    };
    out.push_str(&format!("        proxy_pass {target};\n"));
    let conn = if r.proxy_connect_timeout.is_empty() {
        "5s"
    } else {
        r.proxy_connect_timeout.as_str()
    };
    out.push_str(&format!("        proxy_connect_timeout {conn};\n"));
    let timeout = if !r.proxy_timeout.is_empty() {
        r.proxy_timeout.clone()
    } else if udp {
        // UDP 没有连接概念，会话超时给短一些
        "10s".to_string()
    } else {
        "1h".to_string()
    };
    out.push_str(&format!("        proxy_timeout {timeout};\n"));
    // UDP 要声明等几个响应包；TCP 一般不写（写了反而要求响应数）
    let responses = if r.proxy_responses > 0 {
        r.proxy_responses
    } else if udp {
        1
    } else {
        0
    };
    if responses > 0 {
        out.push_str(&format!("        proxy_responses {responses};\n"));
    }

    let extra = r.extra.trim();
    if !extra.is_empty() {
        out.push_str(&format!("\n        # 自定义指令\n"));
        for line in extra.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            out.push_str(&format!("        {line}\n"));
        }
    }
    out.push_str("    }\n");
    out
}

/// 拆分合并写法的后端地址 → (host, port)：
/// - `10.0.1.10:3306` / `db.example.com:3306` / `[::1]:3306` → 带端口
/// - `backend_api` → upstream 名，端口记 0（渲染时直接引用，不拼端口）
fn split_target(raw: &str) -> Result<(String, i64), ZapError> {
    let v = raw.trim();
    if v.is_empty() {
        return Err(ZapError::New(-1, "后端地址不能为空".to_string()));
    }
    if let Some((h, p)) = v.rsplit_once(':') {
        if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) {
            let port: i64 = p.parse().map_err(|_| {
                ZapError::New(-1, format!("后端端口不是数字：{p}（{v}）"))
            })?;
            let port = validate_port(port, "后端端口")?;
            return Ok((validate_host(h)?, port));
        }
    }
    Ok((validate_host(v)?, 0))
}

/// 负载组自动生成的 upstream 名：`zap_stream_{规则ID}`。
/// 高级模式（自己写配置）或填了 `proxy_pass` 变量时不生成，返回空串。
fn upstream_name(r: &StreamRow) -> String {
    if r.is_advanced() || !r.is_group() || !r.proxy_pass.trim().is_empty() {
        return String::new();
    }
    format!("zap_stream_{}", r.id)
}

/// 单后端的 `proxy_pass` 目标。支持三种写法：
/// - `10.0.1.10:443` / `db.example.com:3306` —— 地址自带端口，原样用；
/// - `10.0.1.10` + 端口 3306 —— 拼成 `10.0.1.10:3306`；
/// - `backend_api` + 端口 0 —— **upstream 名**（在「全局配置」里定义的），不带端口直接引用。
fn backend_target(r: &StreamRow) -> String {
    let host = r.target_host.trim();
    if host_has_port(host) {
        return host.to_string();
    }
    if r.target_port > 0 {
        return format!("{host}:{}", r.target_port);
    }
    host.to_string()
}

/// 地址里是否已带端口：冒号后全是数字才算（`[::1]` 这种 IPv6 不算）。
fn host_has_port(host: &str) -> bool {
    matches!(host.rsplit_once(':'), Some((_, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// basic 模式的后端列表：没填 `targets` 就用单后端（带上默认的健康检查参数）。
fn backend_servers(r: &StreamRow) -> Vec<String> {
    let multi: Vec<String> = r
        .targets
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                return None;
            }
            // 用户没写参数时补一份默认值，写了就完全按用户写的来
            let mut it = l.split_whitespace();
            let addr = it.next().unwrap_or_default().to_string();
            let rest: Vec<&str> = it.collect();
            if rest.is_empty() {
                Some(format!("{addr} max_fails=3 fail_timeout=10s"))
            } else {
                Some(format!("{addr} {}", rest.join(" ")))
            }
        })
        .collect();
    if !multi.is_empty() {
        return multi;
    }
    vec![format!(
        "{}:{} max_fails=3 fail_timeout=10s",
        r.target_host, r.target_port
    )]
}

// ── 端点 ────────────────────────────────────────────────────

/// GET /system/stream/status
pub async fn status(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(Request::NginxStreamStatus).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": resp.data }),
    ))
}

/// GET /system/stream/list
pub async fn list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let pool = db::get_db_pool().await;
    let rows: Vec<StreamRow> =
        sqlx::query_as(&format!("SELECT {COLS} FROM nginx_stream ORDER BY id"))
            .fetch_all(pool)
            .await?;
    let items: Vec<Value> = rows.iter().map(row_json).collect();
    Ok(Json(json!({ "code": 0, "message": "ok", "data": { "items": items } })))
}

/// 高级字段（add / update 共用，`None` = 保持原值 / 用默认）。
#[derive(Debug, Default, Deserialize)]
pub struct AdvInput {
    /// basic / advanced
    #[serde(default)]
    pub mode: Option<String>,
    /// advanced：stream 块内的整段自定义配置
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub targets: Option<String>,
    #[serde(default)]
    pub listen_opts: Option<String>,
    #[serde(default)]
    pub proxy_connect_timeout: Option<String>,
    #[serde(default)]
    pub proxy_timeout: Option<String>,
    #[serde(default)]
    pub proxy_responses: Option<i64>,
    #[serde(default)]
    pub ssl_enable: Option<bool>,
    #[serde(default)]
    pub ssl_certificate: Option<String>,
    #[serde(default)]
    pub ssl_certificate_key: Option<String>,
    #[serde(default)]
    pub ssl_protocols: Option<String>,
    /// single（单后端，直接 proxy_pass） / group（upstream 负载组）
    #[serde(default)]
    pub backend_mode: Option<String>,
    #[serde(default)]
    pub ssl_ciphers: Option<String>,
    /// 证书库（ssl_cert）里的证书 id；0 = 用下面的手工路径
    #[serde(default)]
    pub ssl_certificate_id: Option<i64>,
    #[serde(default)]
    pub ssl_preread: Option<bool>,
    #[serde(default)]
    pub proxy_pass: Option<String>,
    #[serde(default)]
    pub extra: Option<String>,
}

/// 归一化后的高级字段：校验过、可直接落库与渲染。
struct Adv {
    mode: String,
    raw: String,
    backend_mode: String,
    targets: String,
    listen_opts: String,
    proxy_connect_timeout: String,
    proxy_timeout: String,
    proxy_responses: i64,
    ssl_enable: i32,
    ssl_certificate: String,
    ssl_certificate_key: String,
    ssl_protocols: String,
    ssl_ciphers: String,
    ssl_certificate_id: i64,
    ssl_preread: i32,
    proxy_pass: String,
    extra: String,
}

/// 有传就校验后采用，没传就用 `cur`（update 传当前值，add 传默认值）。
macro_rules! adv_field {
    ($val:expr, $cur:expr, $check:expr) => {
        match $val {
            Some(v) => $check(&v)?,
            None => $cur,
        }
    };
}

async fn build_adv(input: AdvInput, cur: Option<&StreamRow>) -> Result<Adv, ZapError> {
    let cur_str = |f: fn(&StreamRow) -> &str| -> String {
        cur.map(|c| f(c).to_string()).unwrap_or_default()
    };
    let cur_i64 = |f: fn(&StreamRow) -> i64| -> i64 { cur.map(f).unwrap_or(0) };
    let cur_i32 = |f: fn(&StreamRow) -> i32| -> i32 { cur.map(f).unwrap_or(0) };

    let mode = adv_field!(input.mode, cur_str(|c| &c.mode), validate_mode);
    let raw = adv_field!(input.raw, cur_str(|c| &c.raw), |v| {
        validate_fragment(v, "自定义配置")
    });
    let backend_mode = adv_field!(
        input.backend_mode,
        cur_str(|c| &c.backend_mode),
        validate_backend_mode
    );
    let targets = adv_field!(input.targets, cur_str(|c| &c.targets), validate_targets);
    let listen_opts = adv_field!(input.listen_opts, cur_str(|c| &c.listen_opts), validate_listen_opts);
    let proxy_connect_timeout = adv_field!(
        input.proxy_connect_timeout,
        cur_str(|c| &c.proxy_connect_timeout),
        |v| validate_time_opt(v, "proxy_connect_timeout")
    );
    let proxy_timeout = adv_field!(input.proxy_timeout, cur_str(|c| &c.proxy_timeout), |v| {
        validate_time_opt(v, "proxy_timeout")
    });
    let proxy_responses = input
        .proxy_responses
        .unwrap_or_else(|| cur_i64(|c| c.proxy_responses))
        .clamp(0, 1000);
    let ssl_enable = match input.ssl_enable {
        Some(v) => i32::from(v),
        None => cur_i32(|c| c.ssl_enable),
    };
    let ssl_certificate = adv_field!(
        input.ssl_certificate,
        cur_str(|c| &c.ssl_certificate),
        |v| validate_cert_path(v, "证书路径")
    );
    let ssl_certificate_key = adv_field!(
        input.ssl_certificate_key,
        cur_str(|c| &c.ssl_certificate_key),
        |v| validate_cert_path(v, "私钥路径")
    );
    let ssl_protocols = adv_field!(input.ssl_protocols, cur_str(|c| &c.ssl_protocols), |v| {
        validate_ssl_text(v, "ssl_protocols")
    });
    let ssl_ciphers = adv_field!(input.ssl_ciphers, cur_str(|c| &c.ssl_ciphers), |v| {
        validate_ssl_text(v, "ssl_ciphers")
    });
    let ssl_certificate_id = match input.ssl_certificate_id {
        Some(v) => {
            if v < 0 {
                0
            } else if v == 0 {
                0
            } else {
                // 保存时就确认证书还在，免得应用时才发现引用了不存在的证书
                cert_exists(v).await?;
                v
            }
        }
        None => cur_i64(|c| c.ssl_certificate_id),
    };
    let ssl_preread = match input.ssl_preread {
        Some(v) => i32::from(v),
        None => cur_i32(|c| c.ssl_preread),
    };
    let proxy_pass = adv_field!(input.proxy_pass, cur_str(|c| &c.proxy_pass), |v| {
        validate_proxy_pass(v)
    });
    let extra = adv_field!(input.extra, cur_str(|c| &c.extra), validate_extra);

    let adv = Adv {
        mode,
        raw,
        backend_mode,
        targets,
        listen_opts,
        proxy_connect_timeout,
        proxy_timeout,
        proxy_responses,
        ssl_enable,
        ssl_certificate,
        ssl_certificate_key,
        ssl_protocols,
        ssl_ciphers,
        ssl_certificate_id,
        ssl_preread,
        proxy_pass,
        extra,
    };
    check_adv_combo(&adv)?;
    Ok(adv)
}

/// 组合约束：单个字段都合法，但放一起会生成跑不起来的配置（同步，便于单测）。
fn check_adv_combo(a: &Adv) -> Result<(), ZapError> {
    if a.mode == "advanced" && a.raw.trim().is_empty() {
        return Err(ZapError::New(
            -1,
            "高级模式必须填写自定义配置（或切回基础模式）".to_string(),
        ));
    }
    // 证书库优先：选了库里的证书就不再要求手工路径
    let have_cert = a.ssl_certificate_id > 0
        || (!a.ssl_certificate.is_empty() && !a.ssl_certificate_key.is_empty());
    if a.ssl_enable == 1 && !have_cert {
        return Err(ZapError::New(
            -1,
            "开启 TLS 终止时要么从证书库选一张证书，要么填证书路径与私钥路径".to_string(),
        ));
    }
    if a.ssl_preread == 1 && a.proxy_pass.is_empty() {
        return Err(ZapError::New(
            -1,
            "开启 ssl_preread 时要填 proxy_pass 变量（如 $backend），\
             并在 Nginx 服务配置里用 map $ssl_preread_server_name $backend 定义它"
                .to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct AddBody {
    pub name: String,
    pub listen_port: i64,
    /// 合并写法：`10.0.1.10:3306` 或 `backend_api`（upstream 名，不带端口）。
    /// 传它就忽略下面的 target_host / target_port（那两个只为兼容旧调用保留）。
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub target_host: Option<String>,
    #[serde(default)]
    pub target_port: Option<i64>,
    #[serde(default)]
    pub listen_ip: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default = "default_status")]
    pub status: i32,
    #[serde(flatten)]
    pub adv: AdvInput,
}

fn default_status() -> i32 {
    1
}

/// POST /system/stream/add
pub async fn add(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<AddBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let name = validate_name(&body.name)?;
    let listen_ip = validate_listen_ip(body.listen_ip.as_deref().unwrap_or(""))?;
    let listen_port = validate_port(body.listen_port, "监听端口")?;
    let protocol = validate_protocol(body.protocol.as_deref().unwrap_or(""))?;
    // 高级模式自己写整段配置，后端地址/端口只是展示与端口冲突参考，留空也放行
    let advanced = validate_mode(body.adv.mode.as_deref().unwrap_or("basic"))? == "advanced";
    let (target_host, target_port) = match body.target.as_deref() {
        Some(t) if !t.trim().is_empty() => split_target(t)?,
        // 高级模式下允许后端地址留空（自定义配置里自己写）
        Some(_) if advanced => ("-".to_string(), 0),
        Some(_) => return Err(ZapError::New(-1, "后端地址不能为空".to_string())),
        None => {
            let host = match body.target_host.as_deref() {
                Some(v) if !(advanced && v.trim().is_empty()) => validate_host(v)?,
                Some(_) => "-".to_string(),
                None => return Err(ZapError::New(-1, "后端地址不能为空".to_string())),
            };
            // 端口填 0 = 后端是 upstream 名，转发时不再拼端口
            let port = match body.target_port.unwrap_or(0) {
                0 => 0,
                p if advanced && !(1..=65535).contains(&p) => 0,
                p => validate_port(p, "后端端口")?,
            };
            (host, port)
        }
    };
    let remark = body.remark.unwrap_or_default().trim().to_string();
    let status = body.status.clamp(0, 1);
    let adv = build_adv(body.adv, None).await?;

    let pool = db::get_db_pool().await;
    let dup: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM nginx_stream \
         WHERE listen_ip = ? AND listen_port = ? AND protocol = ?",
    )
    .bind(&listen_ip)
    .bind(listen_port)
    .bind(&protocol)
    .fetch_one(pool)
    .await
    .unwrap_or(false);
    if dup {
        return Err(ZapError::New(
            -1,
            format!("{listen_ip}:{listen_port}（{protocol}）已被其它规则占用"),
        ));
    }

    let now = chrono::Local::now().timestamp();
    let result = sqlx::query(
        "INSERT INTO nginx_stream (name, listen_ip, listen_port, protocol, target_host, \
         target_port, mode, raw, backend_mode, targets, listen_opts, proxy_connect_timeout, \
         proxy_timeout, proxy_responses, ssl_enable, ssl_certificate, ssl_certificate_key, \
         ssl_protocols, ssl_ciphers, ssl_certificate_id, ssl_preread, proxy_pass, extra, \
         remark, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&name)
    .bind(&listen_ip)
    .bind(listen_port)
    .bind(&protocol)
    .bind(&target_host)
    .bind(target_port)
    .bind(&adv.mode)
    .bind(&adv.raw)
    .bind(&adv.backend_mode)
    .bind(&adv.targets)
    .bind(&adv.listen_opts)
    .bind(&adv.proxy_connect_timeout)
    .bind(&adv.proxy_timeout)
    .bind(adv.proxy_responses)
    .bind(adv.ssl_enable)
    .bind(&adv.ssl_certificate)
    .bind(&adv.ssl_certificate_key)
    .bind(&adv.ssl_protocols)
    .bind(&adv.ssl_ciphers)
    .bind(adv.ssl_certificate_id)
    .bind(adv.ssl_preread)
    .bind(&adv.proxy_pass)
    .bind(&adv.extra)
    .bind(&remark)
    .bind(status)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await;

    let new_id = match result {
        Ok(r) => r.last_insert_rowid(),
        Err(sqlx::Error::Database(e)) if e.message().contains("nginx_stream.name") => {
            return Err(ZapError::New(-1, format!("规则名「{name}」已存在")));
        }
        Err(e) => return Err(ZapError::from(e)),
    };

    audit::log(
        Some(&claims),
        Some(addr.ip().to_string().as_str()),
        "stream_add",
        &format!("id={new_id}"),
        &format!(
            "{listen_ip}:{listen_port}/{protocol} -> {target_host}:{target_port} name={name}"
        ),
    )
    .await;

    let applied = apply_all().await;
    match applied {
        Ok(()) => Ok(Json(
            json!({ "code": 0, "message": "规则已添加并生效", "data": { "id": new_id } }),
        )),
        Err(e) => Ok(Json(json!({
            "code": 0,
            "message": format!("规则已保存，但Nginx未生效：{e}"),
            "data": { "id": new_id, "applied": false },
        }))),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateBody {
    pub id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub listen_ip: Option<String>,
    #[serde(default)]
    pub listen_port: Option<i64>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub target_host: Option<String>,
    #[serde(default)]
    pub target_port: Option<i64>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub status: Option<i32>,
    #[serde(flatten)]
    pub adv: AdvInput,
}

/// POST /system/stream/update：只改传了的字段，改完重新渲染生效。
pub async fn update(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<UpdateBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let pool = db::get_db_pool().await;
    let cur: Option<StreamRow> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM nginx_stream WHERE id = ?"
    ))
    .bind(body.id)
    .fetch_optional(pool)
    .await?;
    let Some(cur) = cur else {
        return Err(ZapError::New(-1, "规则不存在".to_string()));
    };

    let name = match &body.name {
        Some(v) => validate_name(v)?,
        None => cur.name.clone(),
    };
    let listen_ip = match &body.listen_ip {
        Some(v) => validate_listen_ip(v)?,
        None => cur.listen_ip.clone(),
    };
    let listen_port = match body.listen_port {
        Some(v) => validate_port(v, "监听端口")?,
        None => cur.listen_port,
    };
    let protocol = match &body.protocol {
        Some(v) => validate_protocol(v)?,
        None => cur.protocol.clone(),
    };
    // 同上：高级模式下后端地址/端口不参与渲染，留空也放行
    let advanced = validate_mode(body.adv.mode.as_deref().unwrap_or(&cur.mode))? == "advanced";
    let (target_host, target_port) = match body.target.as_deref() {
        Some(t) if !t.trim().is_empty() => split_target(t)?,
        Some(_) if advanced => ("-".to_string(), 0),
        Some(_) => return Err(ZapError::New(-1, "后端地址不能为空".to_string())),
        None => {
            let host = match &body.target_host {
                Some(v) if !(advanced && v.trim().is_empty()) => validate_host(v)?,
                Some(_) => "-".to_string(),
                None => cur.target_host.clone(),
            };
            let port = match body.target_port {
                Some(0) => 0,
                Some(p) if advanced && !(1..=65535).contains(&p) => 0,
                Some(p) => validate_port(p, "后端端口")?,
                None => cur.target_port,
            };
            (host, port)
        }
    };
    let remark = body.remark.clone().unwrap_or(cur.remark.clone());
    let status = body.status.unwrap_or(cur.status).clamp(0, 1);
    let adv = build_adv(body.adv, Some(&cur)).await?;

    let dup: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM nginx_stream \
         WHERE listen_ip = ? AND listen_port = ? AND protocol = ? AND id <> ?",
    )
    .bind(&listen_ip)
    .bind(listen_port)
    .bind(&protocol)
    .bind(body.id)
    .fetch_one(pool)
    .await
    .unwrap_or(false);
    if dup {
        return Err(ZapError::New(
            -1,
            format!("{listen_ip}:{listen_port}（{protocol}）已被其它规则占用"),
        ));
    }

    let now = chrono::Local::now().timestamp();
    sqlx::query(
        "UPDATE nginx_stream SET name = ?, listen_ip = ?, listen_port = ?, protocol = ?, \
         target_host = ?, target_port = ?, mode = ?, raw = ?, backend_mode = ?, targets = ?, \
         listen_opts = ?, proxy_connect_timeout = ?, proxy_timeout = ?, proxy_responses = ?, \
         ssl_enable = ?, ssl_certificate = ?, ssl_certificate_key = ?, ssl_protocols = ?, \
         ssl_ciphers = ?, ssl_certificate_id = ?, ssl_preread = ?, proxy_pass = ?, extra = ?, \
         remark = ?, status = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&name)
    .bind(&listen_ip)
    .bind(listen_port)
    .bind(&protocol)
    .bind(&target_host)
    .bind(target_port)
    .bind(&adv.mode)
    .bind(&adv.raw)
    .bind(&adv.backend_mode)
    .bind(&adv.targets)
    .bind(&adv.listen_opts)
    .bind(&adv.proxy_connect_timeout)
    .bind(&adv.proxy_timeout)
    .bind(adv.proxy_responses)
    .bind(adv.ssl_enable)
    .bind(&adv.ssl_certificate)
    .bind(&adv.ssl_certificate_key)
    .bind(&adv.ssl_protocols)
    .bind(&adv.ssl_ciphers)
    .bind(adv.ssl_certificate_id)
    .bind(adv.ssl_preread)
    .bind(&adv.proxy_pass)
    .bind(&adv.extra)
    .bind(&remark)
    .bind(status)
    .bind(now)
    .bind(body.id)
    .execute(pool)
    .await?;

    audit::log(
        Some(&claims),
        Some(addr.ip().to_string().as_str()),
        "stream_update",
        &format!("id={}", body.id),
        &format!("{listen_ip}:{listen_port}/{protocol} -> {target_host}:{target_port}"),
    )
    .await;

    finish_apply("规则已更新").await
}

#[derive(Debug, Deserialize)]
pub struct IdBody {
    pub id: i64,
}

/// POST /system/stream/delete
pub async fn delete(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<IdBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let pool = db::get_db_pool().await;
    let done = sqlx::query("DELETE FROM nginx_stream WHERE id = ?")
        .bind(body.id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ZapError::New(-1, "规则不存在".to_string()));
    }
    audit::log(
        Some(&claims),
        Some(addr.ip().to_string().as_str()),
        "stream_delete",
        &format!("id={}", body.id),
        &format!("id={}", body.id),
    )
    .await;
    finish_apply("规则已删除").await
}

/// POST /system/stream/apply：按库里现有规则重新渲染（主配置被改坏时用它重建）。
pub async fn apply(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    finish_apply("四层转发配置已重新应用").await
}

// ── 证书库 ──────────────────────────────────────────────────

/// 证书库里选的证书落盘目录：`{data}/ssl/stream/`。
///
/// nginx 的 `ssl_certificate` 只认文件路径，所以库里的 PEM 要先落成文件。
fn cert_dir() -> PathBuf {
    crate::zap::appstore::data_dir().join("ssl").join("stream")
}

fn cert_file(rule_id: i64, ext: &str) -> String {
    cert_dir()
        .join(format!("zap-stream-{rule_id}.{ext}"))
        .to_string_lossy()
        .to_string()
}

/// 保存时确认引用的证书还在（避免应用时才炸）。
async fn cert_exists(id: i64) -> Result<(), ZapError> {
    let pool = db::get_db_pool().await;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ssl_cert WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    if n == 0 {
        return Err(ZapError::New(
            -1,
            format!("证书库里没有 id={id} 的证书（可能已被删除）"),
        ));
    }
    Ok(())
}

/// 把启用规则引用的证书写成文件（私钥 0600，只有 root / 面板用户能读）。
async fn write_cert_files(rows: &[StreamRow]) -> Result<(), String> {
    let need: Vec<&StreamRow> = rows
        .iter()
        .filter(|r| r.status == 1 && r.ssl_certificate_id > 0)
        .collect();
    if need.is_empty() {
        return Ok(());
    }
    let dir = cert_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建证书目录失败: {e}"))?;
    let pool = db::get_db_pool().await;
    for r in &need {
        let pem: Option<(String, String)> =
            sqlx::query_as("SELECT cert_content, key_content FROM ssl_cert WHERE id = ?")
                .bind(r.ssl_certificate_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("读取证书失败: {e}"))?;
        let Some((cert, key)) = pem else {
            return Err(format!(
                "规则「{}」引用的证书（id={}）已不存在，请重新选择",
                r.name, r.ssl_certificate_id
            ));
        };
        if cert.trim().is_empty() || key.trim().is_empty() {
            return Err(format!(
                "规则「{}」引用的证书（id={}）内容为空",
                r.name, r.ssl_certificate_id
            ));
        }
        let crt = cert_file(r.id, "crt");
        let key_path = cert_file(r.id, "key");
        std::fs::write(&crt, cert).map_err(|e| format!("写入证书文件失败: {e}"))?;
        std::fs::write(&key_path, key).map_err(|e| format!("写入私钥文件失败: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
        }
    }
    // 规则删掉 / 关掉之后别把私钥留在盘上
    let keep: std::collections::HashSet<i64> = need.iter().map(|r| r.id).collect();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(rest) = name.strip_prefix("zap-stream-") else {
                continue;
            };
            let id = rest
                .trim_end_matches(".crt")
                .trim_end_matches(".key")
                .parse::<i64>()
                .unwrap_or(-1);
            if id > 0 && !keep.contains(&id) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(())
}

/// GET /system/stream/certs：证书库里可选择的证书（下拉用）。
pub async fn certs(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let pool = db::get_db_pool().await;
    let rows: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT id, name, domains, not_after FROM ssl_cert ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, domains, not_after)| {
            json!({ "id": id, "name": name, "domains": domains, "not_after": not_after })
        })
        .collect();
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": { "items": items } }),
    ))
}

/// 读全局自定义片段（`stream { }` 顶层指令：resolver / map / 公共 upstream …）。
async fn load_global() -> Result<String, String> {
    let pool = db::get_db_pool().await;
    let v: Option<String> = sqlx::query_scalar("SELECT content FROM nginx_stream_global WHERE id = 1")
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("读取全局配置失败: {e}"))?;
    Ok(v.unwrap_or_default())
}

/// GET /system/stream/global
pub async fn global_get(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let content = load_global().await.map_err(|e| ZapError::New(-1, e))?;
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": { "content": content } }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct GlobalBody {
    #[serde(default)]
    pub content: String,
}

/// POST /system/stream/global/save
pub async fn global_save(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<GlobalBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let content = validate_fragment(&body.content, "全局配置")?;
    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();
    sqlx::query(
        "INSERT INTO nginx_stream_global (id, content, updated_at) VALUES (1, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
    )
    .bind(&content)
    .bind(now)
    .execute(pool)
    .await?;
    audit::log(
        Some(&claims),
        Some(addr.ip().to_string().as_str()),
        "stream_global_save",
        "global",
        &format!("{} 字符", content.chars().count()),
    )
    .await;
    finish_apply("全局配置已保存").await
}

/// 渲染并下发到 zapexec；失败时把原因带回前端（库里已经存好，不影响数据）。
async fn finish_apply(ok_msg: &str) -> ZapJsonResult {
    match apply_all().await {
        Ok(()) => Ok(Json(json!({ "code": 0, "message": ok_msg, "data": { "applied": true } }))),
        Err(e) => Ok(Json(json!({
            "code": 0,
            "message": format!("{ok_msg}，但Nginx未生效：{e}"),
            "data": { "applied": false },
        }))),
    }
}

async fn apply_all() -> Result<(), String> {
    let pool = db::get_db_pool().await;
    let rows: Vec<StreamRow> =
        sqlx::query_as(&format!("SELECT {COLS} FROM nginx_stream ORDER BY id"))
            .fetch_all(pool)
            .await
            .map_err(|e| format!("读取规则失败: {e}"))?;
    // 证书库里的证书先落盘（nginx 只认文件路径），再渲染引用这些路径
    write_cert_files(&rows).await?;
    let global = load_global().await?;
    let resp = crate::zapexec::call(Request::NginxStreamApply {
        content: render_conf(&rows, &global),
    })
    .await
    .map_err(|e| format!("下发配置失败: {e}"))?;
    if resp.code != 0 {
        return Err(resp.message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, name: &str, ip: &str, port: i64, proto: &str, host: &str) -> StreamRow {
        StreamRow {
            id,
            name: name.to_string(),
            listen_ip: ip.to_string(),
            listen_port: port,
            protocol: proto.to_string(),
            target_host: host.to_string(),
            target_port: 3306,
            mode: "basic".to_string(),
            raw: String::new(),
            backend_mode: "group".to_string(),
            targets: String::new(),
            listen_opts: String::new(),
            proxy_connect_timeout: String::new(),
            proxy_timeout: String::new(),
            proxy_responses: 0,
            ssl_enable: 0,
            ssl_certificate: String::new(),
            ssl_certificate_key: String::new(),
            ssl_protocols: String::new(),
            ssl_ciphers: String::new(),
            ssl_certificate_id: 0,
            ssl_preread: 0,
            proxy_pass: String::new(),
            extra: String::new(),
            remark: String::new(),
            status: 1,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    /// access_log 必须是绝对路径：相对路径按 nginx prefix 解析，很多机器没有
    /// logs 目录，nginx 加载配置时 open 失败会直接起不来。
    #[test]
    fn access_log_uses_abs_path() {
        let r = row(1, "t", "", 13306, "tcp", "10.0.0.5");
        let out = render_conf(std::slice::from_ref(&r), "");
        assert!(out.contains("access_log /var/log/nginx/zap-stream.log zap_stream;"));
        assert!(!out.contains("access_log logs/"));
    }

    fn render_skips_disabled_and_empty() {
        let mut r = row(1, "mysql", "0.0.0.0", 13306, "tcp", "10.0.0.5");
        assert!(render_conf(&[r.clone()], "").contains("listen 0.0.0.0:13306;"));
        r.status = 0;
        assert_eq!(render_conf(&[r], ""), "");
        assert_eq!(render_conf(&[], ""), "");
    }

    #[test]
    fn render_udp_rule_marks_udp() {
        let r = row(2, "dns", "0.0.0.0", 53, "udp", "10.0.0.6");
        let out = render_conf(&[r], "");
        assert!(out.contains("listen 0.0.0.0:53 udp;"));
        assert!(out.contains("proxy_responses 1;"));
    }

    #[test]
    fn render_advanced_uses_raw_verbatim() {
        let mut r = row(3, "sni", "0.0.0.0", 443, "tcp", "10.0.1.10");
        r.mode = "advanced".to_string();
        r.raw = "upstream backend_api { server 10.0.1.10:443; }\n\
                 server { listen 443; ssl_preread on; proxy_pass backend_api; }"
            .to_string();
        let out = render_conf(&[r], "");
        assert!(out.contains("upstream backend_api"));
        assert!(out.contains("ssl_preread on;"));
        // 高级模式不再自动套 upstream / proxy_timeout
        assert!(!out.contains("zap_stream_3"));
    }

    #[test]
    fn render_global_fragment_comes_first() {
        let r = row(1, "mysql", "0.0.0.0", 13306, "tcp", "10.0.0.5");
        let out = render_conf(&[r], "resolver 10.0.0.2 valid=10s;\nresolver_timeout 10s;");
        assert!(out.contains("resolver 10.0.0.2 valid=10s;"));
        let g = out.find("resolver ").unwrap();
        let s = out.find("server {").unwrap();
        assert!(g < s, "全局片段必须排在 server 之前");
        // 只有全局片段、没有启用规则时也要出配置
        assert!(render_conf(&[], "resolver 8.8.8.8;").contains("stream {"));
    }

    #[test]
    fn render_multi_targets_into_upstream() {
        let mut r = row(4, "db", "0.0.0.0", 3306, "tcp", "10.0.0.5");
        r.targets = "10.0.0.5:3306 weight=2\n10.0.0.6:3306 backup".to_string();
        let out = render_conf(&[r], "");
        assert!(out.contains("server 10.0.0.5:3306 weight=2;"));
        assert!(out.contains("server 10.0.0.6:3306 backup;"));
    }

    #[test]
    fn render_ssl_termination_and_listen_opts() {
        let mut r = row(5, "tls", "0.0.0.0", 3306, "tcp", "10.10.1.100");
        r.ssl_enable = 1;
        r.ssl_certificate = "/etc/nginx/ssl/tls.crt".to_string();
        r.ssl_certificate_key = "/etc/nginx/ssl/tls.key".to_string();
        r.ssl_protocols = "TLSv1.2 TLSv1.3".to_string();
        r.listen_opts = "reuseport".to_string();
        r.proxy_timeout = "300s".to_string();
        let out = render_conf(&[r], "");
        assert!(out.contains("listen 0.0.0.0:3306 reuseport;"));
        assert!(out.contains("ssl_certificate /etc/nginx/ssl/tls.crt;"));
        assert!(out.contains("ssl_protocols TLSv1.2 TLSv1.3;"));
        assert!(out.contains("proxy_timeout 300s;"));
    }

    #[test]
    fn render_preread_uses_custom_proxy_pass() {
        let mut r = row(6, "sni", "0.0.0.0", 443, "tcp", "10.0.1.10");
        r.ssl_preread = 1;
        r.proxy_pass = "$backend".to_string();
        let out = render_conf(
            &[r],
            "map $ssl_preread_server_name $backend { default backend_default; }",
        );
        assert!(out.contains("ssl_preread on;"));
        assert!(out.contains("proxy_pass $backend;"));
        assert!(!out.contains("upstream zap_stream_6"));
    }

    #[test]
    fn advanced_inputs_are_validated() {
        assert_eq!(validate_mode("").unwrap(), "basic");
        assert_eq!(validate_mode("ADVANCED").unwrap(), "advanced");
        assert!(validate_mode("raw").is_err());

        // 再套一层 stream 必须挡掉，否则 nginx -t 必失败
        assert!(validate_fragment("stream { server {} }", "自定义配置").is_err());
        assert!(validate_fragment("upstream a { server 1.1.1.1:53; }", "自定义配置").is_ok());
        assert!(validate_extra("proxy_timeout 5s;").is_ok());
        assert!(validate_extra("upstream x { }").is_err());

        assert!(validate_targets("10.0.0.5:3306\n10.0.0.6:3306 weight=2").is_ok());
        assert!(validate_targets("10.0.0.5").is_err());
        assert!(validate_targets("10.0.0.5:99999").is_err());
        assert!(validate_targets("10.0.0.5:3306 ; rm -rf /").is_err());

        assert_eq!(
            validate_listen_opts("udp reuseport").unwrap(),
            "udp reuseport"
        );
        assert!(validate_listen_opts("backlog=1024").is_ok());
        assert!(validate_listen_opts("reuseport; }").is_err());

        assert_eq!(validate_time_opt("10s", "t").unwrap(), "10s");
        assert!(validate_time_opt("500ms", "t").is_ok());
        assert_eq!(validate_time_opt("", "t").unwrap(), "");
        assert!(validate_time_opt("10 seconds", "t").is_err());

        assert!(validate_cert_path("/etc/nginx/ssl/tls.crt", "证书路径").is_ok());
        assert!(validate_cert_path("tls.crt", "证书路径").is_err());
        assert!(validate_cert_path("/a/b.crt; }", "证书路径").is_err());

        assert_eq!(validate_proxy_pass("$backend").unwrap(), "$backend");
        assert!(validate_proxy_pass("10.0.0.5:3306").is_err());
        assert!(validate_proxy_pass("$a-b").is_err());
    }

    fn adv(mut f: impl FnMut(&mut Adv)) -> Adv {
        let mut a = Adv {
            mode: "basic".to_string(),
            raw: String::new(),
            backend_mode: "group".to_string(),
            targets: String::new(),
            listen_opts: String::new(),
            proxy_connect_timeout: String::new(),
            proxy_timeout: String::new(),
            proxy_responses: 0,
            ssl_enable: 0,
            ssl_certificate: String::new(),
            ssl_certificate_key: String::new(),
            ssl_protocols: String::new(),
            ssl_ciphers: String::new(),
            ssl_certificate_id: 0,
            ssl_preread: 0,
            proxy_pass: String::new(),
            extra: String::new(),
        };
        f(&mut a);
        a
    }

    #[test]
    fn advanced_combinations_are_checked() {
        // 高级模式必须给 raw
        assert!(check_adv_combo(&adv(|a| a.mode = "advanced".to_string())).is_err());

        // 开了 TLS 就得给证书（手工路径或证书库二选一）
        assert!(check_adv_combo(&adv(|a| a.ssl_enable = 1)).is_err());
        assert!(check_adv_combo(&adv(|a| {
            a.ssl_enable = 1;
            a.ssl_certificate_id = 7;
        }))
        .is_ok());
        assert!(check_adv_combo(&adv(|a| {
            a.ssl_enable = 1;
            a.ssl_certificate = "/etc/nginx/a.crt".to_string();
            a.ssl_certificate_key = "/etc/nginx/a.key".to_string();
        }))
        .is_ok());

        // ssl_preread 必须配变量
        assert!(check_adv_combo(&adv(|a| a.ssl_preread = 1)).is_err());
        assert!(check_adv_combo(&adv(|a| {
            a.ssl_preread = 1;
            a.proxy_pass = "$backend".to_string();
        }))
        .is_ok());
    }

    #[test]
    fn target_is_split_into_host_and_port() {
        assert_eq!(
            split_target("10.0.1.10:3306").unwrap(),
            ("10.0.1.10".to_string(), 3306)
        );
        assert_eq!(
            split_target("db.example.com:3306").unwrap(),
            ("db.example.com".to_string(), 3306)
        );
        // IPv6 的冒号不能当端口分隔符
        assert_eq!(split_target("[::1]:3306").unwrap(), ("[::1]".to_string(), 3306));
        // 不带端口 = upstream 名
        assert_eq!(split_target("backend_api").unwrap(), ("backend_api".to_string(), 0));
        assert!(split_target("").is_err());
        assert!(split_target("10.0.1.10:99999").is_err());
        assert!(split_target("10.0.1.10:3306; }").is_err());
    }

    #[test]
    fn render_single_backend_supports_upstream_name() {
        // upstream 名：端口 0，不拼端口
        let mut r = row(7, "api", "0.0.0.0", 443, "tcp", "backend_api");
        r.backend_mode = "single".to_string();
        r.target_port = 0;
        let out = render_conf(&[r], "upstream backend_api { server 10.0.1.10:443; }");
        assert!(out.contains("proxy_pass backend_api;"));
        assert!(!out.contains("backend_api:0"));

        // 地址自带端口：原样用，不再拼一次
        let mut r = row(8, "db", "0.0.0.0", 3306, "tcp", "10.0.1.10:3307");
        r.backend_mode = "single".to_string();
        assert!(render_conf(&[r], "").contains("proxy_pass 10.0.1.10:3307;"));

        // IPv6 不带端口 + 端口字段 → 拼上
        let mut r = row(9, "v6", "0.0.0.0", 3306, "tcp", "[::1]");
        r.backend_mode = "single".to_string();
        r.target_port = 3306;
        assert!(render_conf(&[r], "").contains("proxy_pass [::1]:3306;"));
    }

    #[test]
    fn render_single_backend_skips_upstream() {
        let mut r = row(7, "db", "0.0.0.0", 3306, "tcp", "10.0.0.5");
        r.backend_mode = "single".to_string();
        let out = render_conf(&[r], "");
        assert!(out.contains("proxy_pass 10.0.0.5:3306;"));
        assert!(!out.contains("upstream zap_stream_7"));

        // 负载组模式（默认）仍然生成 upstream
        let g = row(7, "db", "0.0.0.0", 3306, "tcp", "10.0.0.5");
        let out = render_conf(&[g], "");
        assert!(out.contains("upstream zap_stream_7"));
        assert!(out.contains("proxy_pass zap_stream_7;"));
    }

    #[test]
    fn render_uses_cert_from_cert_store() {
        let mut r = row(8, "tls", "0.0.0.0", 3306, "tcp", "10.10.1.100");
        r.ssl_enable = 1;
        r.ssl_certificate_id = 12;
        let out = render_conf(&[r], "");
        // 证书库来的证书走落盘文件（data/ssl/stream/zap-stream-8.crt）
        assert!(out.contains("ssl_certificate") && out.contains("zap-stream-8.crt"));
        assert!(out.contains("zap-stream-8.key"));
    }

    #[test]
    fn backend_mode_is_validated() {
        assert_eq!(validate_backend_mode("").unwrap(), "group");
        assert_eq!(validate_backend_mode("single").unwrap(), "single");
        assert_eq!(validate_backend_mode("upstream").unwrap(), "group");
        assert!(validate_backend_mode("cluster").is_err());
    }

    #[test]
    fn inputs_are_validated() {
        assert!(validate_name("").is_err());
        assert!(validate_name("a\nb").is_err());
        assert_eq!(validate_name(" mysql ").unwrap(), "mysql");
        assert_eq!(validate_listen_ip("").unwrap(), "0.0.0.0");
        assert_eq!(validate_listen_ip("127.0.0.1").unwrap(), "127.0.0.1");
        assert!(validate_listen_ip("example.com").is_err());
        assert!(validate_port(0, "监听端口").is_err());
        assert!(validate_port(70000, "监听端口").is_err());
        assert_eq!(validate_port(8080, "监听端口").unwrap(), 8080);
        // 能改写 nginx 配置结构的字符一律不收
        assert!(validate_host("10.0.0.5; }").is_err());
        assert!(validate_host("10.0.0.5").is_ok());
        assert!(validate_host("[::1]").is_ok());
        assert_eq!(validate_protocol("").unwrap(), "tcp");
        assert_eq!(validate_protocol("UDP").unwrap(), "udp");
        assert!(validate_protocol("http").is_err());
    }
}
