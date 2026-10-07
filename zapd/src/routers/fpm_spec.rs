// SPDX-License-Identifier: AGPL-3.0-only
//! PHP-FPM pool 规格模板库（admin 维护，全局一张表）。
//!
//! 模板名约定：
//! - 以 `{用户名}_` 开头 → 视为「归该用户名下」（其名下客户 / reseller 场景可用、可被继承）
//! - 其它名字 → 全局通用模板（所有人添加用户时都可见可选）
//!
//! 用户侧引用（`user.fpm_spec_ref`）：
//! - `''`            面板全局默认（`fpm_pool_defaults` → 内置兜底）
//! - `inherit`       继承归属者（owner，通常为 reseller）名下的默认模板
//! - 其它            精确模板名（须存在于模板表）
//!
//! 解析顺序（建站同步时下发 PhpPoolSync 前调用 [`resolve_user_spec`]）：
//! 存量自定义 `user.fpm_pool`(JSON) → fpm_spec_ref 引用的模板 / 继承 → 全局默认 → 内置兜底。
//!
//! 端点：
//! - GET  /system/fpm-specs/list    模板列表（admin 全量；reseller 仅自己名下 + 全局）
//! - POST /system/fpm-specs/add     新增（admin）
//! - POST /system/fpm-specs/update  修改（admin）
//! - POST /system/fpm-specs/delete  删除（admin）

use std::net::SocketAddr;

use axum::Json;
use axum::extract::Extension;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::{ZapError, ZapJsonResult, audit};

/// `user.fpm_spec_ref` 中表示「继承 owner(reseller) 名下默认」的保留值。
pub const INHERIT: &str = "inherit";

#[derive(sqlx::FromRow, Debug)]
struct SpecRow {
    id: i64,
    name: String,
    spec: String,
    remark: String,
    created_at: i64,
    updated_at: i64,
}

/// 模板名归属：取「以 `{用户名}_` 开头」且匹配到的用户名（多个时取最长前缀，避免
/// `ab` 与 `ab_c` 这类嵌套用户名误判）。无匹配 = 全局通用模板。
fn template_owner(name: &str, usernames: &[String]) -> Option<String> {
    let mut best: Option<&String> = None;
    for u in usernames {
        if name.starts_with(&format!("{u}_")) {
            let better = match best {
                Some(b) => u.len() > b.len(),
                None => true,
            };
            if better {
                best = Some(u);
            }
        }
    }
    best.cloned()
}

fn is_global_template(name: &str, usernames: &[String]) -> bool {
    template_owner(name, usernames).is_none()
}

/// 校验模板名：仅字母数字与 `_.-`，不以特殊字符开头，1..=64，且避开保留字。
fn validate_name(name: &str) -> Result<String, ZapError> {
    let n = name.trim();
    if n.is_empty() {
        return Err(ZapError::New(-1, "模板名不能为空".to_string()));
    }
    if n.len() > 64 {
        return Err(ZapError::New(-1, "模板名最长 64 字符".to_string()));
    }
    let ok = n
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
        && n.chars()
            .next()
            .map(|c| c.is_ascii_alphanumeric())
            .unwrap_or(false);
    if !ok {
        return Err(ZapError::New(
            -1,
            "模板名只能包含字母、数字、_ . -，且须以字母或数字开头（建议形如 resellerA_high）"
                .to_string(),
        ));
    }
    if n == INHERIT {
        return Err(ZapError::New(
            -1,
            format!("{INHERIT} 为保留字，不能用作模板名"),
        ));
    }
    Ok(n.to_string())
}

fn validate_spec_json(spec: &str) -> Result<String, ZapError> {
    let s = spec.trim().to_string();
    if s.is_empty() {
        return Err(ZapError::New(
            -1,
            "spec 不能为空，至少为 {}（JSON 对象）".to_string(),
        ));
    }
    match serde_json::from_str::<Value>(&s) {
        Ok(Value::Object(_)) => Ok(s),
        _ => Err(ZapError::New(
            -1,
            "spec 必须是 JSON 对象（如 {\"max_children\": 16, \"memory_limit\": \"512M\"}）"
                .to_string(),
        )),
    }
}

async fn load_all_usernames() -> Vec<String> {
    let pool = db::get_db_pool().await;
    sqlx::query_scalar("SELECT username FROM user WHERE username != ''")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// 校验某个 fpm_spec_ref 对操作者是否可用（user add/update 时调用）。
/// - `''` / `inherit` 恒允许
/// - 模板名：必须存在；非 admin（reseller）只能选自己名下或全局通用模板
pub async fn validate_spec_ref(
    spec_ref: &str,
    is_admin: bool,
    actor_username: &str,
) -> Result<(), ZapError> {
    let r = spec_ref.trim();
    if r.is_empty() || r == INHERIT {
        return Ok(());
    }
    let pool = db::get_db_pool().await;
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM fpm_spec")
        .fetch_all(pool)
        .await?;
    if !names.iter().any(|n| n == r) {
        return Err(ZapError::New(
            -1,
            format!("FPM 规格模板「{r}」不存在，请刷新后重试"),
        ));
    }
    if !is_admin {
        let usernames = load_all_usernames().await;
        let mine = r.starts_with(&format!("{actor_username}_"));
        if !mine && !is_global_template(r, &usernames) {
            return Err(ZapError::New(
                -1,
                format!("不能选择他人名下的规格模板「{r}」"),
            ));
        }
    }
    Ok(())
}

/// 全局兜底 + 面板默认（scope=conf, fpm_pool_defaults）合并后的 base。
async fn global_base() -> serde_json::Map<String, Value> {
    let mut base = crate::routers::system_env::default_fpm_spec();
    if let Some(v) = crate::zap::server_env::conf_get("fpm_pool_defaults")
        && let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&v)
    {
        for (k, val) in obj {
            base.insert(k, val);
        }
    }
    base
}

fn merge_obj(base: &mut serde_json::Map<String, Value>, spec_json: &str) {
    if spec_json.trim().is_empty() {
        return;
    }
    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(spec_json) {
        for (k, val) in obj {
            base.insert(k, val);
        }
    }
}

/// 归属者（reseller）名下默认模板 spec：优先 `{username}_default`，
/// 其次 `{username}_` 前缀下最新（updated_at 最大）的模板。无则 None。
async fn owner_default_template_spec(owner_username: &str) -> Option<String> {
    let pool = db::get_db_pool().await;
    let rows: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT name, spec, updated_at FROM fpm_spec")
            .fetch_all(pool)
            .await
            .ok()?;
    let prefix = format!("{owner_username}_");
    let owned: Vec<&(String, String, i64)> = rows
        .iter()
        .filter(|(n, _, _)| n.starts_with(&prefix))
        .collect();
    if owned.is_empty() {
        return None;
    }
    // 显式默认模板（{username}_default）优先
    let explicit = owned
        .iter()
        .find(|(n, _, _)| *n == format!("{prefix}default"));
    if let Some((_, spec, _)) = explicit {
        return Some(spec.clone());
    }
    // 兜底：名下最新模板
    owned
        .iter()
        .max_by_key(|(_, _, ts)| *ts)
        .map(|(_, spec, _)| spec.clone())
}

/// 解析用户最终 pool 规格 JSON 字符串（供建站同步下发 PhpPoolSync）。
///
/// 优先级：存量自定义 fpm_pool(JSON) → fpm_spec_ref（模板名 / inherit）→ 全局默认 → 内置兜底。
pub async fn resolve_user_spec(
    user_fpm_pool: Option<&str>,
    spec_ref: &str,
    owner_id: Option<i64>,
) -> String {
    let mut base = global_base().await;

    let ufp = user_fpm_pool.map(str::trim).unwrap_or("");
    if !ufp.is_empty() {
        // 旧版自定义 JSON 覆盖（兼容存量数据，最高优先）
        merge_obj(&mut base, ufp);
        return serde_json::Value::Object(base).to_string();
    }

    let r = spec_ref.trim();
    if !r.is_empty() {
        let pool = db::get_db_pool().await;
        let chosen: Option<String> = if r == INHERIT {
            match owner_id {
                Some(oid) if oid > 0 => {
                    // owner 的 username → 名下默认模板
                    let uname: Option<String> =
                        sqlx::query_scalar("SELECT username FROM user WHERE id = ?")
                            .bind(oid)
                            .fetch_optional(pool)
                            .await
                            .ok()
                            .flatten();
                    match uname {
                        Some(u) => owner_default_template_spec(&u).await,
                        None => None, // 归属者已被删除 → 回退全局默认
                    }
                }
                _ => None, // 系统直属（无 reseller）→ 回退全局默认
            }
        } else {
            sqlx::query_scalar("SELECT spec FROM fpm_spec WHERE name = ?")
                .bind(r)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten()
        };
        if let Some(spec) = chosen {
            merge_obj(&mut base, &spec);
        }
    }

    serde_json::Value::Object(base).to_string()
}

// ── handlers ────────────────────────────────────────────────

/// GET /system/fpm-specs/list —— admin 全量；reseller 仅自己名下 + 全局模板。
pub async fn spec_list(claims: ValidatedClaims) -> ZapJsonResult {
    let is_admin = crate::zap::jwt::is_admin(&claims);
    let is_reseller = crate::zap::jwt::is_reseller(&claims);
    if !is_admin && !is_reseller {
        return Err(ZapError::New(-1, "权限不足".to_string()));
    }
    let actor = claims.sub.clone();
    let pool = db::get_db_pool().await;
    let rows: Vec<SpecRow> = sqlx::query_as(
        "SELECT id, name, spec, remark, created_at, updated_at FROM fpm_spec ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await?;
    let usernames = load_all_usernames().await;

    let mut items = Vec::new();
    for r in rows {
        let owner = template_owner(&r.name, &usernames);
        if !is_admin {
            // reseller 视角：只给「自己名下」与「全局通用」
            let mine = owner.as_deref() == Some(actor.as_str());
            if !mine && !is_global_template(&r.name, &usernames) {
                continue;
            }
        }
        items.push(json!({
            "id": r.id,
            "name": r.name,
            "spec": r.spec,
            "remark": r.remark,
            "owner": owner, // null = 全局通用
            "created_at": r.created_at,
            "updated_at": r.updated_at,
        }));
    }
    Ok(Json(json!({ "code": 0, "message": "OK", "data": items })))
}

#[derive(Debug, Deserialize)]
pub struct SpecAddPayload {
    pub name: String,
    pub spec: String,
    pub remark: Option<String>,
}

/// POST /system/fpm-specs/add
pub async fn spec_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SpecAddPayload>,
) -> ZapJsonResult {
    if !crate::zap::jwt::is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可维护 FPM 规格模板".to_string()));
    }
    let name = validate_name(&payload.name)?;
    let spec = validate_spec_json(&payload.spec)?;
    let remark = payload.remark.unwrap_or_default().trim().to_string();
    let now = chrono::Local::now().timestamp();

    let pool = db::get_db_pool().await;
    let result = sqlx::query(
        "INSERT INTO fpm_spec (name, spec, remark, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&name)
    .bind(&spec)
    .bind(&remark)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await;
    let new_id = match result {
        Ok(r) => r.last_insert_rowid(),
        Err(sqlx::Error::Database(e)) if e.message().contains("idx_fpm_spec_name") => {
            return Err(ZapError::New(-1, format!("模板名「{name}」已存在")));
        }
        Err(e) => return Err(ZapError::from(e)),
    };

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "fpm_spec_add",
        &format!("id={new_id}"),
        &format!("name={name} spec={spec}"),
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "规格模板已创建", "data": { "id": new_id } }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct SpecUpdatePayload {
    pub id: i64,
    pub name: Option<String>,
    pub spec: Option<String>,
    pub remark: Option<String>,
}

/// POST /system/fpm-specs/update
pub async fn spec_update(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SpecUpdatePayload>,
) -> ZapJsonResult {
    if !crate::zap::jwt::is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可维护 FPM 规格模板".to_string()));
    }
    let pool = db::get_db_pool().await;
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM fpm_spec WHERE id = ?")
        .bind(payload.id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(ZapError::New(-1, "模板不存在".to_string()));
    }

    let mut ups: Vec<(String, String)> = Vec::new();
    if let Some(n) = payload.name {
        ups.push(("name".to_string(), validate_name(&n)?));
    }
    if let Some(s) = payload.spec {
        ups.push(("spec".to_string(), validate_spec_json(&s)?));
    }
    if let Some(rm) = payload.remark {
        ups.push(("remark".to_string(), rm.trim().to_string()));
    }
    if ups.is_empty() {
        return Err(ZapError::New(-1, "没有需要更新的字段".to_string()));
    }

    let now = chrono::Local::now().timestamp();
    for (k, v) in &ups {
        let sql = format!("UPDATE fpm_spec SET {k} = ?, updated_at = ? WHERE id = ?");
        let result = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(v)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await;
        if let Err(sqlx::Error::Database(e)) = &result
            && e.message().contains("idx_fpm_spec_name")
        {
            return Err(ZapError::New(-1, format!("模板名「{v}」已存在")));
        }
        result?;
    }

    let detail = ups
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "fpm_spec_update",
        &format!("id={}", payload.id),
        &detail,
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "规格模板已更新" })))
}

#[derive(Debug, Deserialize)]
pub struct SpecDeletePayload {
    pub id: i64,
}

/// POST /system/fpm-specs/delete
pub async fn spec_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SpecDeletePayload>,
) -> ZapJsonResult {
    if !crate::zap::jwt::is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可维护 FPM 规格模板".to_string()));
    }
    let pool = db::get_db_pool().await;
    let result = sqlx::query("DELETE FROM fpm_spec WHERE id = ?")
        .bind(payload.id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ZapError::New(-1, "模板不存在".to_string()));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "fpm_spec_delete",
        &format!("id={}", payload.id),
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "规格模板已删除" })))
}
