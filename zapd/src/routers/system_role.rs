use axum::{Json, extract::Extension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Sqlite;
use std::net::SocketAddr;
use tracing::info;

use crate::{
    db,
    zap::{ZapError, ZapJsonResult, audit, jwt::ValidatedClaims},
};

// ── types ──────────────────────────────────────────────────

#[derive(sqlx::FromRow, Debug, Serialize)]
struct RoleRow {
    id: i64,
    name: String,
    role_key: String,
    description: String,
    status: i64,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateRolePayload {
    pub name: String,
    pub role_key: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRolePayload {
    pub id: i64,
    pub name: Option<String>,
    pub role_key: Option<String>,
    pub description: Option<String>,
    pub status: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteRolePayload {
    pub id: i64,
}

/// 设置角色权限：
/// - `menu_ids`：菜单可见性（仅前端渲染，不是安全边界）
/// - `permissions`：动作级权限点（`{ns}:view` / `{ns}:edit`），请求级鉴权依据
#[derive(Debug, Deserialize)]
pub struct SetRolePermissionsPayload {
    pub role_id: i64,
    #[serde(default)]
    pub menu_ids: Vec<i64>,
    /// 缺省 `None` 表示不改动已配置的权限点（前端只改菜单时保持原样）
    #[serde(default)]
    pub permissions: Option<Vec<String>>,
}

fn role_to_value(r: &RoleRow) -> Value {
    json!({
        "id": r.id,
        "name": r.name,
        "role_key": r.role_key,
        "description": r.description,
        "status": r.status,
        "created_at": r.created_at,
        "updated_at": r.updated_at,
    })
}

// ── helpers ────────────────────────────────────────────────

/// System built-in role keys. These roles cannot be deleted, disabled,
/// or have their key changed.
const BUILTIN_ROLE_KEYS: [&str; 4] = ["admin", "reseller", "user", "demo"];

fn is_builtin_role(role_key: &str) -> bool {
    BUILTIN_ROLE_KEYS.contains(&role_key)
}

// ── handlers ───────────────────────────────────────────────

pub async fn role_list(_claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let rows: Vec<RoleRow> = sqlx::query_as("SELECT * FROM roles ORDER BY id")
        .fetch_all(pool)
        .await?;

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": rows.iter().map(role_to_value).collect::<Vec<_>>(),
        "total": rows.len(),
    })))
}

pub async fn role_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CreateRolePayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();
    let desc = payload.description.unwrap_or_default();

    if is_builtin_role(&payload.role_key) {
        return Err(ZapError::New(-1, "系统内置角色标识不可使用".to_string()));
    }

    let result = sqlx::query(
        "INSERT INTO roles (name, role_key, description, status, created_at, updated_at) VALUES (?, ?, ?, 1, ?, ?)",
    )
    .bind(&payload.name)
    .bind(&payload.role_key)
    .bind(&desc)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await;

    match result {
        Ok(r) => {
            let role_id = r.last_insert_rowid();
            // 新角色默认只给「仪表盘」：登录后至少能进主页，其余保持 fail-closed。
            // 个人中心（/profile）是前端 constant route，无需菜单授权。
            // 动作级权限点仍为空 —— 看得见 ≠ 能改，需在「角色权限」里显式勾选。
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO role_menus (role_id, menu_id) \
                 SELECT ?, id FROM menus WHERE name = 'dashboard' AND parent_id = 0 AND status = 1",
            )
            .bind(role_id)
            .execute(pool)
            .await;
            audit::log(
                Some(&claims),
                Some(client_addr.ip().to_string().as_str()),
                "role_create",
                &format!("id={role_id}"),
                &format!("name={}, key={}", payload.name, payload.role_key),
            )
            .await;
            info!(
                "Role created: {} (id: {})",
                payload.name,
                r.last_insert_rowid()
            );
            Ok(Json(
                json!({ "code": 0, "message": "创建成功", "data": { "id": r.last_insert_rowid() } }),
            ))
        }
        Err(e) if e.to_string().contains("UNIQUE") => {
            Err(ZapError::New(-1, "角色名称或标识已存在".to_string()))
        }
        Err(e) => Err(ZapError::from(e)),
    }
}

pub async fn role_update(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<UpdateRolePayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();

    // Built-in roles: name/description may be edited, but key/status cannot.
    let role: Option<RoleRow> = sqlx::query_as("SELECT * FROM roles WHERE id = ?")
        .bind(payload.id)
        .fetch_optional(pool)
        .await?;
    let Some(role) = role else {
        return Err(ZapError::New(-1, "角色不存在".to_string()));
    };
    if is_builtin_role(&role.role_key) {
        if payload.role_key.is_some() && payload.role_key.as_deref() != Some(role.role_key.as_str())
        {
            return Err(ZapError::New(-1, "系统内置角色的标识不可修改".to_string()));
        }
        if payload.status == Some(0) {
            return Err(ZapError::New(-1, "系统内置角色不可禁用".to_string()));
        }
    }

    let mut qb: sqlx::QueryBuilder<'_, Sqlite> = sqlx::QueryBuilder::new("UPDATE roles SET ");
    let mut sep = qb.separated(", ");

    if let Some(ref name) = payload.name {
        sep.push("name = ").push_bind_unseparated(name);
    }
    if let Some(ref key) = payload.role_key {
        sep.push("role_key = ").push_bind_unseparated(key);
    }
    if let Some(ref desc) = payload.description {
        sep.push("description = ").push_bind_unseparated(desc);
    }
    if let Some(status) = payload.status {
        sep.push("status = ").push_bind_unseparated(status);
    }
    sep.push("updated_at = ").push_bind_unseparated(now);

    qb.push(" WHERE id = ").push_bind(payload.id);
    let result = qb.build().execute(pool).await?;

    if result.rows_affected() == 0 {
        return Err(ZapError::New(-1, "角色不存在".to_string()));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "role_update",
        &format!("id={}", payload.id),
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "更新成功" })))
}

pub async fn role_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<DeleteRolePayload>,
) -> ZapJsonResult {
    // Prevent deleting built-in roles
    let pool = db::get_db_pool().await;
    let role: RoleRow = sqlx::query_as("SELECT * FROM roles WHERE id = ?")
        .bind(payload.id)
        .fetch_one(pool)
        .await?;

    if is_builtin_role(&role.role_key) {
        return Err(ZapError::New(-1, "系统内置角色不可删除".to_string()));
    }

    let result = sqlx::query("DELETE FROM roles WHERE id = ?")
        .bind(payload.id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ZapError::New(-1, "角色不存在".to_string()));
    }

    // Also clean up role_menus / role_permissions
    let _ = sqlx::query("DELETE FROM role_menus WHERE role_id = ?")
        .bind(payload.id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM role_permissions WHERE role_id = ?")
        .bind(payload.id)
        .execute(pool)
        .await;
    crate::routers::access::invalidate_perm_cache();

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "role_delete",
        &format!("id={}", payload.id),
        &format!("name={}", role.name),
    )
    .await;

    info!("Role deleted: id={}", payload.id);
    Ok(Json(json!({ "code": 0, "message": "删除成功" })))
}

/// Get role permissions：菜单可见性 + 动作级权限点
pub async fn role_permissions_get(
    _claims: ValidatedClaims,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ZapJsonResult {
    let role_id: i64 = params
        .get("role_id")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let pool = db::get_db_pool().await;
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT menu_id FROM role_menus WHERE role_id = ?")
        .bind(role_id)
        .fetch_all(pool)
        .await?;
    let menu_ids: Vec<i64> = rows.into_iter().map(|r| r.0).collect();

    let perms: Vec<(String,)> =
        sqlx::query_as("SELECT perm_key FROM role_permissions WHERE role_id = ?")
            .bind(role_id)
            .fetch_all(pool)
            .await?;
    let permissions: Vec<String> = perms.into_iter().map(|r| r.0).collect();

    Ok(Json(json!({
        "code": 0,
        "data": { "menu_ids": menu_ids, "permissions": permissions },
    })))
}

/// 权限点目录（角色权限配置页的数据源）：来自 `routers::access` 的权限矩阵，
/// 保证「可勾选的权限点」与「实际生效的校验规则」是同一份定义。
pub async fn permission_catalog(_claims: ValidatedClaims) -> ZapJsonResult {
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": { "groups": crate::routers::access::permission_catalog() },
    })))
}

/// Set role permissions
pub async fn role_permissions_set(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SetRolePermissionsPayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;

    // Remove existing
    let _ = sqlx::query("DELETE FROM role_menus WHERE role_id = ?")
        .bind(payload.role_id)
        .execute(pool)
        .await;

    // Insert new
    for menu_id in &payload.menu_ids {
        let _ = sqlx::query("INSERT OR IGNORE INTO role_menus (role_id, menu_id) VALUES (?, ?)")
            .bind(payload.role_id)
            .bind(menu_id)
            .execute(pool)
            .await;
    }

    // 动作级权限点：只接受目录内合法 key，避免脏数据与拼写错误
    let mut granted: Vec<String> = Vec::new();
    if let Some(perms) = &payload.permissions {
        let _ = sqlx::query("DELETE FROM role_permissions WHERE role_id = ?")
            .bind(payload.role_id)
            .execute(pool)
            .await;
        let valid = crate::routers::access::all_perm_keys();
        for key in perms {
            let key = key.trim();
            if !valid.contains(key) {
                continue;
            }
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO role_permissions (role_id, perm_key) VALUES (?, ?)",
            )
            .bind(payload.role_id)
            .bind(key)
            .execute(pool)
            .await;
            granted.push(key.to_string());
        }
        // 权限点变更立即生效（撤销不必等缓存过期）
        crate::routers::access::invalidate_perm_cache();
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "role_permissions_set",
        &format!("role_id={}", payload.role_id),
        &format!("menu_ids={:?} permissions={:?}", payload.menu_ids, granted),
    )
    .await;

    Ok(Json(json!({ "code": 0, "message": "权限设置成功" })))
}
