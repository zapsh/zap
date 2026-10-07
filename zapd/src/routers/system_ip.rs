// SPDX-License-Identifier: AGPL-3.0-only
use axum::{Json, extract::Extension};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};
use tracing::info;

use crate::{
    db,
    zap::{
        ZapError, ZapJsonResult, audit,
        jwt::{self, ValidatedClaims},
    },
};

#[derive(sqlx::FromRow, Debug)]
struct IpPoolRow {
    id: i64,
    address: String,
    version: i32,
    ip_type: String,
    reserved: i32,
    remark: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct IpPoolAddPayload {
    pub addresses: Vec<String>,
    #[serde(default)]
    pub ip_type: Option<String>,
    #[serde(default)]
    pub reserved: Option<i32>,
    #[serde(default)]
    pub remark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IpPoolUpdatePayload {
    pub id: i64,
    #[serde(default)]
    pub ip_type: Option<String>,
    #[serde(default)]
    pub reserved: Option<i32>,
    #[serde(default)]
    pub remark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IpPoolDeletePayload {
    pub ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
pub struct IpPoolBatchReservedPayload {
    pub ids: Vec<i64>,
    pub reserved: i32,
}

/// admin 专属权限校验
fn require_admin(claims: &jwt::Claims) -> Result<(), ZapError> {
    if jwt::is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "权限不足，需要管理员权限".to_string()))
    }
}

fn validate_ip_type(ip_type: &str) -> Result<String, ZapError> {
    match ip_type {
        "shared" | "dedicated" => Ok(ip_type.to_string()),
        _ => Err(ZapError::New(
            -1,
            "ip_type 仅支持 shared（公共 IP）或 dedicated（独享 IP）".to_string(),
        )),
    }
}

/// IP 池列表 + 汇总统计
pub async fn ip_list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let pool = db::get_db_pool().await;
    let rows: Vec<IpPoolRow> =
        sqlx::query_as("SELECT * FROM ip_pool ORDER BY version ASC, address ASC")
            .fetch_all(pool)
            .await?;

    let (mut v4, mut v6, mut shared, mut dedicated, mut reserved) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    for r in &rows {
        if r.version == 6 {
            v6 += 1;
        } else {
            v4 += 1;
        }
        if r.ip_type == "shared" {
            shared += 1;
        } else {
            dedicated += 1;
        }
        if r.reserved == 1 {
            reserved += 1;
        }
    }
    let stats = json!({
        "total": rows.len(),
        "v4": v4,
        "v6": v6,
        "shared": shared,
        "dedicated": dedicated,
        "reserved": reserved,
    });

    let list: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "address": r.address,
                "version": r.version,
                "ip_type": r.ip_type,
                "reserved": r.reserved,
                "remark": r.remark,
                "created_at": r.created_at,
                "updated_at": r.updated_at,
            })
        })
        .collect();

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": list,
        "stats": stats,
    })))
}

/// 添加 IP（支持批量；自动识别 IPv4/IPv6，跳过已存在项）
pub async fn ip_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<IpPoolAddPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let ip_type = payload
        .ip_type
        .unwrap_or_else(|| "shared".to_string())
        .trim()
        .to_lowercase();
    let ip_type = validate_ip_type(&ip_type)?;
    let reserved = payload.reserved.unwrap_or(0).clamp(0, 1);
    let remark = payload.remark.unwrap_or_default().trim().to_string();

    let mut added = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    let mut invalid: Vec<String> = Vec::new();
    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();

    for raw in &payload.addresses {
        let address = raw.trim();
        if address.is_empty() {
            continue;
        }
        let parsed: IpAddr = match address.parse() {
            Ok(ip) => ip,
            Err(_) => {
                invalid.push(address.to_string());
                continue;
            }
        };
        // 已存在则跳过
        let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM ip_pool WHERE address = ?")
            .bind(address)
            .fetch_optional(pool)
            .await?;
        if exists.is_some() {
            skipped.push(address.to_string());
            continue;
        }
        let version = if parsed.is_ipv6() { 6 } else { 4 };
        sqlx::query(
            "INSERT INTO ip_pool (address, version, ip_type, reserved, remark, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(address)
        .bind(version)
        .bind(&ip_type)
        .bind(reserved)
        .bind(&remark)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        added += 1;
    }

    if added > 0 {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "ip_pool_add",
            &format!("added={}", added),
            &format!("type={}", ip_type),
        )
        .await;
        info!(
            "IP pool add: added={} skipped={} invalid={}",
            added,
            skipped.len(),
            invalid.len()
        );
    }

    Ok(Json(json!({
        "code": 0,
        "message": format!("成功添加 {} 个 IP", added),
        "data": { "added": added, "skipped": skipped, "invalid": invalid }
    })))
}

/// 删除 IP（可批量）
pub async fn ip_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<IpPoolDeletePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if payload.ids.is_empty() {
        return Err(ZapError::New(-1, "请选择要删除的 IP".to_string()));
    }
    let pool = db::get_db_pool().await;
    let mut deleted = 0i64;
    for id in &payload.ids {
        let r = sqlx::query("DELETE FROM ip_pool WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        deleted += r.rows_affected() as i64;
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ip_pool_delete",
        &format!("ids={:?}", payload.ids),
        &format!("deleted={}", deleted),
    )
    .await;
    info!("IP pool delete: ids={:?} deleted={}", payload.ids, deleted);
    Ok(Json(json!({
        "code": 0,
        "message": format!("已删除 {} 个 IP", deleted)
    })))
}

/// 更新单个 IP（类型 / Reserved / 备注）
pub async fn ip_update(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<IpPoolUpdatePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if payload.ip_type.is_none() && payload.reserved.is_none() && payload.remark.is_none() {
        return Err(ZapError::New(-1, "没有需要更新的字段".to_string()));
    }
    let ip_type = match payload.ip_type {
        Some(t) => Some(validate_ip_type(t.trim())?),
        None => None,
    };

    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();

    let result = if let Some(ip_type) = &ip_type {
        sqlx::query("UPDATE ip_pool SET ip_type = ?, updated_at = ? WHERE id = ?")
            .bind(ip_type)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?
    } else if let Some(reserved) = payload.reserved {
        let reserved = reserved.clamp(0, 1);
        sqlx::query("UPDATE ip_pool SET reserved = ?, updated_at = ? WHERE id = ?")
            .bind(reserved)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?
    } else if let Some(remark) = &payload.remark {
        sqlx::query("UPDATE ip_pool SET remark = ?, updated_at = ? WHERE id = ?")
            .bind(remark.trim())
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?
    } else {
        return Err(ZapError::New(-1, "没有需要更新的字段".to_string()));
    };

    if result.rows_affected() == 0 {
        return Err(ZapError::New(-1, "IP 不存在".to_string()));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ip_pool_update",
        &format!("id={}", payload.id),
        "",
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": "更新成功"
    })))
}

/// 批量设置 Reserved（保留 / 取消保留）
pub async fn ip_batch_reserved(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<IpPoolBatchReservedPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if payload.ids.is_empty() {
        return Err(ZapError::New(-1, "请选择要操作的 IP".to_string()));
    }
    let reserved = payload.reserved.clamp(0, 1);
    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();
    let mut updated = 0i64;
    for id in &payload.ids {
        let r = sqlx::query("UPDATE ip_pool SET reserved = ?, updated_at = ? WHERE id = ?")
            .bind(reserved)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
        updated += r.rows_affected() as i64;
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ip_pool_batch_reserved",
        &format!("ids={:?}", payload.ids),
        &format!("reserved={}", reserved),
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": format!("已{} {} 个 IP", if reserved == 1 { "保留" } else { "取消保留" }, updated)
    })))
}
