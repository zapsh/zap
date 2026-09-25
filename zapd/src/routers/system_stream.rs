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
    remark: String,
    status: i32,
    created_at: i64,
    updated_at: i64,
}

const COLS: &str = "id, name, listen_ip, listen_port, protocol, target_host, target_port, \
                    remark, status, created_at, updated_at";

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

// ── 渲染 ────────────────────────────────────────────────────

/// 按规则渲染 `stream { }` 块；没有启用的规则就返回空串（撤掉配置）。
fn render_conf(rows: &[StreamRow]) -> String {
    let active: Vec<&StreamRow> = rows.iter().filter(|r| r.status == 1).collect();
    if active.is_empty() {
        return String::new();
    }
    let mut out = String::from("# 由面板生成（四层转发），勿手工修改\nstream {\n");
    out.push_str(
        "    log_format zap_stream '$remote_addr [$time_local] $protocol $status '\n    \
         '\"$bytes_sent\" \"$bytes_received\" $session_time $upstream_addr';\n",
    );
    out.push_str("    access_log logs/zap-stream.log zap_stream;\n");
    for r in &active {
        let udp = r.protocol == "udp";
        out.push_str(&format!("\n    # rule: {} (id={})\n", r.name, r.id));
        out.push_str(&format!("    upstream zap_stream_{} {{\n", r.id));
        out.push_str(&format!(
            "        server {}:{} max_fails=3 fail_timeout=10s;\n",
            r.target_host, r.target_port
        ));
        out.push_str("    }\n    server {\n");
        out.push_str(&format!(
            "        listen {}:{}{};\n",
            r.listen_ip,
            r.listen_port,
            if udp { " udp" } else { "" }
        ));
        out.push_str(&format!("        proxy_pass zap_stream_{};\n", r.id));
        out.push_str("        proxy_connect_timeout 5s;\n");
        // UDP 没有连接概念，超时按会话给短一些
        out.push_str(if udp {
            "        proxy_responses 1;\n        proxy_timeout 10s;\n"
        } else {
            "        proxy_timeout 1h;\n"
        });
        out.push_str("    }\n");
    }
    out.push_str("}\n");
    out
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

#[derive(Debug, Deserialize)]
pub struct AddBody {
    pub name: String,
    pub listen_port: i64,
    pub target_host: String,
    pub target_port: i64,
    #[serde(default)]
    pub listen_ip: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default = "default_status")]
    pub status: i32,
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
    let target_host = validate_host(&body.target_host)?;
    let target_port = validate_port(body.target_port, "后端端口")?;
    let remark = body.remark.unwrap_or_default().trim().to_string();
    let status = body.status.clamp(0, 1);

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
         target_port, remark, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&name)
    .bind(&listen_ip)
    .bind(listen_port)
    .bind(&protocol)
    .bind(&target_host)
    .bind(target_port)
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
    pub target_host: Option<String>,
    #[serde(default)]
    pub target_port: Option<i64>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub status: Option<i32>,
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
    let target_host = match &body.target_host {
        Some(v) => validate_host(v)?,
        None => cur.target_host.clone(),
    };
    let target_port = match body.target_port {
        Some(v) => validate_port(v, "后端端口")?,
        None => cur.target_port,
    };
    let remark = body.remark.clone().unwrap_or(cur.remark.clone());
    let status = body.status.unwrap_or(cur.status).clamp(0, 1);

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
         target_host = ?, target_port = ?, remark = ?, status = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&name)
    .bind(&listen_ip)
    .bind(listen_port)
    .bind(&protocol)
    .bind(&target_host)
    .bind(target_port)
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
    let resp = crate::zapexec::call(Request::NginxStreamApply {
        content: render_conf(&rows),
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
            remark: String::new(),
            status: 1,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn render_skips_disabled_and_empty() {
        let mut r = row(1, "mysql", "0.0.0.0", 13306, "tcp", "10.0.0.5");
        assert!(render_conf(&[r.clone()]).contains("listen 0.0.0.0:13306;"));
        r.status = 0;
        assert_eq!(render_conf(&[r]), "");
        assert_eq!(render_conf(&[]), "");
    }

    #[test]
    fn render_udp_rule_marks_udp() {
        let r = row(2, "dns", "0.0.0.0", 53, "udp", "10.0.0.6");
        let out = render_conf(&[r]);
        assert!(out.contains("listen 0.0.0.0:53 udp;"));
        assert!(out.contains("proxy_responses 1;"));
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
