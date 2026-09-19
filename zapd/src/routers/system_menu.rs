use axum::{Json, extract::Extension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Sqlite;
use std::collections::HashSet;
use std::net::SocketAddr;
use tracing::info;

use crate::{
    db,
    zap::{ZapError, ZapJsonResult, audit, jwt, jwt::ValidatedClaims},
};

// ── types ──────────────────────────────────────────────────

#[derive(sqlx::FromRow, Debug, Serialize)]
pub struct MenuRow {
    pub id: i64,
    pub parent_id: i64,
    pub name: String,
    pub path: String,
    pub component: String,
    pub redirect: String,
    #[sqlx(rename = "type")]
    pub menu_type: String,
    pub title: String,
    pub icon: String,
    pub hidden: i64,
    pub keep_alive: i64,
    pub affix: i64,
    pub roles: String,
    pub sort_order: i64,
    pub status: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateMenuPayload {
    pub parent_id: Option<i64>,
    pub name: String,
    pub path: String,
    pub component: Option<String>,
    pub redirect: Option<String>,
    #[serde(rename = "type")]
    pub menu_type: Option<String>,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub hidden: Option<i64>,
    pub keep_alive: Option<i64>,
    pub affix: Option<i64>,
    pub roles: Option<String>,
    pub sort_order: Option<i64>,
    pub status: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMenuPayload {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: Option<String>,
    pub path: Option<String>,
    pub component: Option<String>,
    pub redirect: Option<String>,
    #[serde(rename = "type")]
    pub menu_type: Option<String>,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub hidden: Option<i64>,
    pub keep_alive: Option<i64>,
    pub affix: Option<i64>,
    pub roles: Option<String>,
    pub sort_order: Option<i64>,
    pub status: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteMenuPayload {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct MenuStatusPayload {
    pub id: i64,
    pub status: i64,
}

// ── tree builder ───────────────────────────────────────────

fn menu_to_tree_value(m: &MenuRow, children: Vec<Value>, explicit: bool) -> Value {
    let mut meta = json!({
        "title": m.title,
        "icon": m.icon,
        "affix": m.affix == 1,
    });
    // 用户级显式授权的菜单绕开两道前端过滤：
    // - hidden：既然管理员单独勾给了这个人，即便入口本身已隐藏（如迁走的「团队成员」）也对他显示；
    // - roles：menus.roles 只是内置角色的静态标注，前端 filterAsyncRoutes 会据此二次过滤，
    //   不下发才不会被自定义角色 / 经销商误杀（否则后端放行、前端照样不显示）。
    if m.hidden == 1 && !explicit {
        meta["hidden"] = json!(true);
    }
    if m.keep_alive == 1 {
        meta["keepAlive"] = json!(true);
    }
    if !explicit && !m.roles.is_empty() {
        meta["roles"] = json!(m.roles.split(',').map(|s| s.trim()).collect::<Vec<_>>());
    }

    let mut obj = json!({
        "id": m.id,
        "name": m.name,
        "path": m.path,
        "component": m.component,
        "type": m.menu_type,
        "meta": meta,
        "order": m.sort_order,
        "status": m.status,
    });
    if !m.redirect.is_empty() {
        obj["redirect"] = json!(m.redirect);
    }
    if !children.is_empty() {
        obj["children"] = json!(children);
    }
    obj
}

fn build_menu_tree(rows: &[MenuRow], parent_id: i64, extra: &HashSet<i64>) -> Vec<Value> {
    rows.iter()
        .filter(|r| r.parent_id == parent_id)
        .map(|r| {
            let children = build_menu_tree(rows, r.id, extra);
            menu_to_tree_value(r, children, extra.contains(&r.id))
        })
        .collect()
}

/// 某人被「用户级例外」显式放行的菜单 id（见 `user_menus` 表）。
pub async fn extra_menu_ids(uid: i64) -> HashSet<i64> {
    let pool = db::get_db_pool().await;
    sqlx::query_scalar::<_, i64>("SELECT menu_id FROM user_menus WHERE user_id = ?")
        .bind(uid)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// 当前用户可见的菜单行：`status = 1` 且 （角色授权 ∪ 用户级例外），admin 恒为全量。
///
/// `claims.roles` 是逗号分隔的 role_key，先映射到 roles.id 再查 `role_menus`；
/// 子菜单授权父不授权时，整棵子树在 `build_menu_tree` 里自然消失。
pub async fn visible_menu_rows(claims: &jwt::Claims) -> Result<Vec<MenuRow>, sqlx::Error> {
    let pool = db::get_db_pool().await;
    let my_keys: Vec<&str> = claims
        .roles
        .split(',')
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .collect();
    let is_admin = my_keys.contains(&"admin");

    if is_admin {
        return sqlx::query_as("SELECT * FROM menus WHERE status = 1 ORDER BY sort_order, id")
            .fetch_all(pool)
            .await;
    }
    if my_keys.is_empty() {
        return Ok(Vec::new());
    }

    // 角色 key → 角色 id
    let mut qb = sqlx::QueryBuilder::<Sqlite>::new("SELECT id FROM roles WHERE role_key IN (");
    let mut sep = qb.separated(", ");
    for k in &my_keys {
        sep.push_bind(*k);
    }
    qb.push(")");
    let role_ids: Vec<(i64,)> = qb.build_query_as().fetch_all(pool).await?;
    if role_ids.is_empty() {
        return Ok(Vec::new());
    }

    // 角色授予 ∪ 用户级例外
    let extra = extra_menu_ids(claims.id as i64).await;
    let mut qb2 = sqlx::QueryBuilder::<Sqlite>::new(
        "SELECT * FROM menus WHERE status = 1 AND (id IN \
         (SELECT menu_id FROM role_menus WHERE role_id IN (",
    );
    let mut sep2 = qb2.separated(", ");
    for (rid,) in &role_ids {
        sep2.push_bind(*rid);
    }
    qb2.push("))");
    if !extra.is_empty() {
        qb2.push(" OR id IN (");
        let mut sep3 = qb2.separated(", ");
        for mid in &extra {
            sep3.push_bind(*mid);
        }
        qb2.push(")");
    }
    qb2.push(") ORDER BY sort_order, id");
    qb2.build_query_as::<MenuRow>().fetch_all(pool).await
}

// ── handlers ───────────────────────────────────────────────

/// Get menu tree visible to the current user (for rendering sidebar)。
///
/// - admin 恒返回全部启用的菜单；
/// - 其他角色按「用户角色 key → roles.id → role_menus」动态授权过滤，
///   角色管理里给角色勾选的菜单即在此生效（menus.roles 仅为内置角色的静态标注）；
/// - 最后并上 **用户级例外**（`user_menus`），见 `visible_menu_rows`。
pub async fn get_menus_tree(claims: ValidatedClaims) -> ZapJsonResult {
    let rows = visible_menu_rows(&claims).await?;
    let extra = extra_menu_ids(claims.id as i64).await;
    let tree = build_menu_tree(&rows, 0, &extra);
    Ok(Json(json!({ "code": 0, "message": "ok", "data": tree })))
}

/// Get flat menu list (for admin management)
pub async fn menu_list(_claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let rows: Vec<MenuRow> = sqlx::query_as("SELECT * FROM menus ORDER BY sort_order, id")
        .fetch_all(pool)
        .await?;

    let tree = build_menu_tree(&rows, 0, &HashSet::new());
    Ok(Json(json!({ "code": 0, "message": "ok", "data": tree })))
}

/// Create menu
pub async fn menu_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CreateMenuPayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();

    let result = sqlx::query(
        "INSERT INTO menus (parent_id, name, path, component, redirect, type, title, icon, hidden, keep_alive, affix, roles, sort_order, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(payload.parent_id.unwrap_or(0))
    .bind(&payload.name)
    .bind(&payload.path)
    .bind(payload.component.unwrap_or_default())
    .bind(payload.redirect.unwrap_or_default())
    .bind(payload.menu_type.unwrap_or_else(|| "menu".into()))
    .bind(payload.title.unwrap_or_default())
    .bind(payload.icon.unwrap_or_default())
    .bind(payload.hidden.unwrap_or(0))
    .bind(payload.keep_alive.unwrap_or(0))
    .bind(payload.affix.unwrap_or(0))
    .bind(payload.roles.unwrap_or_default())
    .bind(payload.sort_order.unwrap_or(0))
    .bind(payload.status.unwrap_or(1))
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "menu_create",
        &format!("id={}", result.last_insert_rowid()),
        &payload.name,
    )
    .await;
    info!(
        "Menu created: {} (id: {})",
        payload.name,
        result.last_insert_rowid()
    );
    Ok(Json(
        json!({ "code": 0, "message": "创建成功", "data": { "id": result.last_insert_rowid() } }),
    ))
}

/// Update menu
pub async fn menu_update(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<UpdateMenuPayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();

    let mut qb: sqlx::QueryBuilder<'_, Sqlite> = sqlx::QueryBuilder::new("UPDATE menus SET ");
    let mut sep = qb.separated(", ");

    if let Some(v) = payload.parent_id {
        sep.push("parent_id = ").push_bind_unseparated(v);
    }
    if let Some(ref v) = payload.name {
        sep.push("name = ").push_bind_unseparated(v);
    }
    if let Some(ref v) = payload.path {
        sep.push("path = ").push_bind_unseparated(v);
    }
    if let Some(ref v) = payload.component {
        sep.push("component = ").push_bind_unseparated(v);
    }
    if let Some(ref v) = payload.redirect {
        sep.push("redirect = ").push_bind_unseparated(v);
    }
    if let Some(ref v) = payload.menu_type {
        sep.push("type = ").push_bind_unseparated(v);
    }
    if let Some(ref v) = payload.title {
        sep.push("title = ").push_bind_unseparated(v);
    }
    if let Some(ref v) = payload.icon {
        sep.push("icon = ").push_bind_unseparated(v);
    }
    if let Some(v) = payload.hidden {
        sep.push("hidden = ").push_bind_unseparated(v);
    }
    if let Some(v) = payload.keep_alive {
        sep.push("keep_alive = ").push_bind_unseparated(v);
    }
    if let Some(v) = payload.affix {
        sep.push("affix = ").push_bind_unseparated(v);
    }
    if let Some(ref v) = payload.roles {
        sep.push("roles = ").push_bind_unseparated(v);
    }
    if let Some(v) = payload.sort_order {
        sep.push("sort_order = ").push_bind_unseparated(v);
    }
    if let Some(v) = payload.status {
        sep.push("status = ").push_bind_unseparated(v);
    }
    sep.push("updated_at = ").push_bind_unseparated(now);

    qb.push(" WHERE id = ").push_bind(payload.id);
    let result = qb.build().execute(pool).await?;

    if result.rows_affected() == 0 {
        return Err(ZapError::New(-1, "菜单不存在".to_string()));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "menu_update",
        &format!("id={}", payload.id),
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "更新成功" })))
}

/// Delete menu (and children)
pub async fn menu_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<DeleteMenuPayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    // 先记录子菜单 id：连同源菜单一起回收两条授权表的关联行，避免留下孤儿
    let child_ids: Vec<i64> =
        sqlx::query_scalar::<_, i64>("SELECT id FROM menus WHERE parent_id = ?")
            .bind(payload.id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    // Delete children first
    let _ = sqlx::query("DELETE FROM menus WHERE parent_id = ?")
        .bind(payload.id)
        .execute(pool)
        .await;
    let result = sqlx::query("DELETE FROM menus WHERE id = ?")
        .bind(payload.id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ZapError::New(-1, "菜单不存在".to_string()));
    }

    // Clean orphaned role_menus / user_menus（连同被一起删掉的子菜单）
    for mid in child_ids.iter().chain(std::iter::once(&payload.id)) {
        let _ = sqlx::query("DELETE FROM role_menus WHERE menu_id = ?")
            .bind(*mid)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM user_menus WHERE menu_id = ?")
            .bind(*mid)
            .execute(pool)
            .await;
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "menu_delete",
        &format!("id={}", payload.id),
        "",
    )
    .await;

    Ok(Json(json!({ "code": 0, "message": "删除成功" })))
}

/// Toggle menu status
pub async fn menu_status(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<MenuStatusPayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let result = sqlx::query("UPDATE menus SET status = ? WHERE id = ?")
        .bind(payload.status)
        .bind(payload.id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ZapError::New(-1, "菜单不存在".to_string()));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "menu_status",
        &format!("id={}, status={}", payload.id, payload.status),
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "OK" })))
}
