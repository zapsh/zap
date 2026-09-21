//! 「开发」菜单后端：静态 API Token 的生成 / 列表 / 修改 / 吊销，以及 API 文档。
//!
//! API Token 形如 `zap_<48 位 hex>`，数据库仅保存其 SHA-256 哈希与展示前缀；
//! 认证在 `crate::zap::jwt` 中统一处理（`Authorization: Bearer <token>` 与 JWT 并存）。
//! 每个账号管理自己的 Token；demo 账号的写操作会被全局只读守卫拦截。

use axum::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    db,
    zap::{
        ZapError, ZapJsonResult, audit,
        jwt::{self, ValidatedClaims, sha256_hex},
    },
};

/// 生成一个 `zap_` 前缀的随机 API Token（24 字节高熵，hex 编码）。
fn gen_api_token() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut buf);
    format!("{}{}", jwt::API_TOKEN_PREFIX, hex::encode(buf))
}

#[derive(sqlx::FromRow, Debug, serde::Serialize)]
struct ApiTokenRow {
    id: i64,
    name: String,
    prefix: String,
    status: i64,
    expires_at: i64,
    last_used_at: i64,
    created_at: i64,
}

/// GET /api/dev/api-token/list —— 当前登录用户的 API Token 列表
pub async fn api_token_list(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let rows: Vec<ApiTokenRow> = sqlx::query_as(
        "SELECT id, name, prefix, status, expires_at, last_used_at, created_at
         FROM api_token WHERE user_id = ? ORDER BY id DESC",
    )
    .bind(claims.id as i64)
    .fetch_all(pool)
    .await?;
    Ok(Json(json!({ "code": 0, "message": "OK", "data": rows })))
}

#[derive(Debug, Deserialize)]
pub struct ApiTokenCreatePayload {
    /// 备注名称（可为空）
    #[serde(default)]
    pub name: Option<String>,
    /// 有效期（天），缺省或 <= 0 表示永不过期
    #[serde(default)]
    pub expire_days: Option<i64>,
}

/// POST /api/dev/api-token/create —— 新建 API Token（完整值仅本次返回，请立即保存）
pub async fn api_token_create(
    claims: ValidatedClaims,
    Json(payload): Json<ApiTokenCreatePayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let name = payload.name.unwrap_or_default().trim().to_string();
    let now = chrono::Utc::now().timestamp();
    let expires_at = match payload.expire_days {
        Some(d) if d > 0 => now + d * 86400,
        _ => 0,
    };

    let raw = gen_api_token();
    let hash = sha256_hex(&raw);
    // 展示前缀：zap_ + 前 16 位 hex（其余部分不可见，仅用于在列表中辨认）
    let prefix: String = raw.chars().take(jwt::API_TOKEN_PREFIX.len() + 16).collect();

    // token_version 必须与用户当前版本号对齐：否则新 Token 一出生就落后于
    // 「下线所有设备」推高的版本号，第一次使用就被判为已下线
    let r = sqlx::query(
        "INSERT INTO api_token (user_id, name, token_hash, prefix, expires_at, status, token_version, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 1, ?, ?, ?)",
    )
    .bind(claims.id as i64)
    .bind(&name)
    .bind(&hash)
    .bind(&prefix)
    .bind(expires_at)
    .bind(crate::zap::session::version_of(claims.id))
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    audit::log(
        Some(&claims),
        None,
        "api_token_create",
        &name,
        &format!("id={}", r.last_insert_rowid()),
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "id": r.last_insert_rowid(),
            "token": raw,
            "prefix": prefix,
            "expires_at": expires_at,
            "created_at": now,
        }
    })))
}

#[derive(Debug, Deserialize)]
pub struct ApiTokenUpdatePayload {
    pub id: i64,
    /// 备注名称（不传则不改）
    #[serde(default)]
    pub name: Option<String>,
    /// 状态 1 启用 / 0 停用（不传则不改）
    #[serde(default)]
    pub status: Option<i64>,
}

/// POST /api/dev/api-token/update —— 修改备注或启停用
pub async fn api_token_update(
    claims: ValidatedClaims,
    Json(payload): Json<ApiTokenUpdatePayload>,
) -> ZapJsonResult {
    let mut sets: Vec<&str> = Vec::new();
    let mut binds: Vec<serde_json::Value> = Vec::new();
    if let Some(n) = &payload.name {
        let n = n.trim().to_string();
        sets.push("name = ?");
        binds.push(Value::String(n));
    }
    if let Some(s) = payload.status {
        sets.push("status = ?");
        binds.push(Value::Number(s.into()));
    }
    if sets.is_empty() {
        return Err(ZapError::New(-1, "没有需要修改的字段".to_string()));
    }

    let pool = db::get_db_pool().await;
    let sql = format!(
        "UPDATE api_token SET {}, updated_at = ? WHERE id = ? AND user_id = ?",
        sets.join(", ")
    );
    let now = chrono::Utc::now().timestamp();
    let mut q = sqlx::query(&sql);
    for b in &binds {
        match b {
            Value::String(s) => q = q.bind(s),
            Value::Number(n) => q = q.bind(n.as_i64()),
            _ => {}
        }
    }
    q = q.bind(now).bind(payload.id).bind(claims.id as i64);
    let r = q.execute(pool).await?;
    if r.rows_affected() == 0 {
        return Err(ZapError::New(-1, "Token 不存在或无权操作".to_string()));
    }
    audit::log(
        Some(&claims),
        None,
        "api_token_update",
        &format!("id={}", payload.id),
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "OK", "data": null })))
}

#[derive(Debug, Deserialize)]
pub struct ApiTokenDeletePayload {
    pub id: i64,
}

/// POST /api/dev/api-token/delete —— 吊销（删除）API Token
pub async fn api_token_delete(
    claims: ValidatedClaims,
    Json(payload): Json<ApiTokenDeletePayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let r = sqlx::query("DELETE FROM api_token WHERE id = ? AND user_id = ?")
        .bind(payload.id)
        .bind(claims.id as i64)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(ZapError::New(-1, "Token 不存在或无权操作".to_string()));
    }
    audit::log(
        Some(&claims),
        None,
        "api_token_delete",
        &format!("id={}", payload.id),
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "OK", "data": null })))
}

/// GET /api/dev/api-docs —— 返回内置 API 文档（含认证说明与各端点清单）
pub async fn api_docs(_claims: ValidatedClaims) -> ZapJsonResult {
    let docs: Value = serde_json::from_str(include_str!("api_docs.json")).unwrap_or(Value::Null);
    Ok(Json(json!({ "code": 0, "message": "OK", "data": docs })))
}
