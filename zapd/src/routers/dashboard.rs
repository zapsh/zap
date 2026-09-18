//! 仪表盘统计卡片数据源。
//!
//! GET /api/dashboard/counts → 按当前角色可见范围统计：
//! - 用户数量：admin → 全部客户账号；reseller → 名下客户；普通用户 → 0（不展示该卡片）
//! - 站点数量：admin → 全部；reseller → 自己 + 名下客户；普通用户 → 自己的站点
//! - 数据库数量：MySQL / MariaDB 中「用户名_」前缀的库（admin 统计全部非系统库）

use axum::Json;
use serde_json::json;

use crate::db::get_db_pool;
use crate::zap::ZapJsonResult;
use crate::zap::jwt::{ValidatedClaims, is_admin, is_reseller};

pub async fn counts(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = get_db_pool().await;
    let me = claims.id as i64;
    let admin = is_admin(&claims);
    let reseller = is_reseller(&claims);

    // ── 用户数量 ────────────────────────────────────────────
    let users: i64 = if admin {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM user WHERE (',' || COALESCE(roles,'') || ',') LIKE '%,user,%'",
        )
        .fetch_one(pool)
        .await
        .unwrap_or(0)
    } else if reseller {
        sqlx::query_scalar("SELECT COUNT(*) FROM user WHERE owner_id = ?")
            .bind(me)
            .fetch_one(pool)
            .await
            .unwrap_or(0)
    } else {
        0
    };

    // ── 站点数量 ────────────────────────────────────────────
    let sites: i64 = if admin {
        sqlx::query_scalar("SELECT COUNT(*) FROM site")
            .fetch_one(pool)
            .await
            .unwrap_or(0)
    } else if reseller {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM site \
             WHERE user_id = ? OR user_id IN (SELECT id FROM user WHERE owner_id = ?)",
        )
        .bind(me)
        .bind(me)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
    } else {
        // 团队共享：统计与站点列表一致（归属组内共享可见）
        let gid = crate::routers::site::group_id_of(me).await;
        sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM site WHERE {}",
            crate::routers::site::group_scope_cond("user_id")
        ))
        .bind(gid)
        .bind(crate::routers::user::USER_KIND_MEMBER)
        .bind(gid)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
    };

    // ── 数据库数量（按「用户名_」前缀归属）────────────────────
    let databases: i64 = if admin {
        crate::routers::database::count_schemas(&[])
    } else {
        // reseller：自己 + 名下客户；普通用户：自己
        let names: Vec<String> = if reseller {
            let mut v: Vec<String> =
                sqlx::query_scalar("SELECT username FROM user WHERE id = ? OR owner_id = ?")
                    .bind(me)
                    .bind(me)
                    .fetch_all(pool)
                    .await
                    .unwrap_or_default();
            v.sort_unstable();
            v.dedup();
            v
        } else {
            sqlx::query_scalar("SELECT username FROM user WHERE id = ?")
                .bind(me)
                .fetch_all(pool)
                .await
                .unwrap_or_default()
        };
        let prefixes: Vec<String> = names
            .iter()
            .map(|n| crate::routers::database::schema_prefix_of(n))
            .collect();
        crate::routers::database::count_schemas(&prefixes)
    };

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "users": users,
            "sites": sites,
            "databases": databases,
        }
    })))
}
