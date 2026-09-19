//! 用户级菜单例外（`user_menus`）：在「角色 → role_menus」之外，
//! 给**单个用户**单独放行的侧边栏入口。
//!
//! 只决定**菜单渲染**（体验层），请求级鉴权仍由 `routers::access` 按角色 /
//! 个人权限点判断：勾了菜单但缺 `*:view` / `*:edit`，进页面照样 403。
//!
//! 授予范围收敛（防越权）：
//! - admin：任意账号，候选为全部启用菜单；
//! - 其他人（reseller / 普通用户）：只能把自己**已经可见**的菜单再分出去，
//!   且目标必须是自己名下成员 / 客户（`user.owner_id = 自己`）。

use axum::{
    Json,
    extract::{Extension, Query},
};
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;

use crate::{
    db,
    zap::{ZapError, ZapJsonResult, audit, jwt, jwt::ValidatedClaims},
};

use super::system_menu;

// ── helpers ────────────────────────────────────────────────

/// 目标用户是否可被当前操作者调整菜单。
///
/// - 本人：允许（例如管理员给自己例外显示某个隐藏入口）；
/// - admin：任意账号；
/// - 其他人：只有自己名下成员 / 客户（`owner_id = 自己`）。
async fn manageable(claims: &jwt::Claims, target: i64) -> bool {
    if target == claims.id as i64 || jwt::is_admin(claims) {
        return true;
    }
    let pool = db::get_db_pool().await;
    let owner: Option<(i64,)> = sqlx::query_as("SELECT owner_id FROM user WHERE id = ?")
        .bind(target)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);
    matches!(owner, Some((o,)) if o == claims.id as i64)
}

/// 操作者可授予的菜单 id 集合。
///
/// 写接口同样按它过滤（后端兜底，前端置灰只是提示）：不能把别人没有、
/// 自己也看不到的菜单分出去。
async fn grantable_ids(claims: &jwt::Claims) -> HashSet<i64> {
    let pool = db::get_db_pool().await;
    if jwt::is_admin(claims) {
        return sqlx::query_scalar::<_, i64>("SELECT id FROM menus WHERE status = 1")
            .fetch_all(pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
    }
    // 非 admin：自己看得见的才谈得上「分给别人」（含角色授权与自己的例外）
    system_menu::visible_menu_rows(claims)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.id)
        .collect()
}

fn sorted(ids: HashSet<i64>) -> Vec<i64> {
    let mut v: Vec<i64> = ids.into_iter().collect();
    v.sort();
    v
}

// ── payloads ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UserMenusPayload {
    pub user_id: i64,
    /// 全量覆盖：一次请求即该用户最终的额外菜单清单
    pub menu_ids: Vec<i64>,
}

// ── handlers ───────────────────────────────────────────────

/// GET /user/menus?user_id=
///
/// `menu_ids`：该用户已获得的额外菜单；
/// `available_ids`：当前操作者能授予的范围（前端据此置灰不可选项）。
///
/// 权限：admin 任意账号；其他人对**自己名下成员 / 客户**可用（同 `/user/team/*`）。
pub async fn user_menus_get(
    claims: ValidatedClaims,
    Query(params): Query<HashMap<String, String>>,
) -> ZapJsonResult {
    let user_id: i64 = params
        .get("user_id")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if user_id <= 0 {
        return Err(ZapError::New(-1, "参数无效：user_id".to_string()));
    }
    if !manageable(&claims, user_id).await {
        return Err(ZapError::New(
            -1,
            "权限不足：只能管理自己名下的账号".to_string(),
        ));
    }

    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": {
            "user_id": user_id,
            "menu_ids": sorted(system_menu::extra_menu_ids(user_id).await),
            "available_ids": sorted(grantable_ids(&claims).await),
        },
    })))
}

/// POST /user/menus/set — 全量覆盖某用户的额外菜单。
pub async fn user_menus_set(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<UserMenusPayload>,
) -> ZapJsonResult {
    let uid = payload.user_id;
    if uid <= 0 {
        return Err(ZapError::New(-1, "参数无效：user_id".to_string()));
    }
    if !manageable(&claims, uid).await {
        return Err(ZapError::New(
            -1,
            "权限不足：只能管理自己名下的账号".to_string(),
        ));
    }

    let pool = db::get_db_pool().await;
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user WHERE id = ?")
        .bind(uid)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    if exists == 0 {
        return Err(ZapError::New(-1, "用户不存在".to_string()));
    }

    // 越权收敛：只保留操作者自己有权授予的菜单，其余静默丢弃
    let allowed = grantable_ids(&claims).await;
    let kept: Vec<i64> = payload
        .menu_ids
        .into_iter()
        .filter(|m| allowed.contains(m))
        .collect();

    let now = chrono::Local::now().timestamp();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_menus WHERE user_id = ?")
        .bind(uid)
        .execute(&mut *tx)
        .await?;
    for mid in &kept {
        sqlx::query(
            "INSERT OR IGNORE INTO user_menus (user_id, menu_id, created_at) VALUES (?, ?, ?)",
        )
        .bind(uid)
        .bind(*mid)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "user_menus_set",
        &format!("user_id={uid}"),
        &format!(
            "menu_ids=[{}]",
            kept.iter()
                .map(|m| m.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
    )
    .await;

    Ok(Json(json!({ "code": 0, "message": "保存成功" })))
}
