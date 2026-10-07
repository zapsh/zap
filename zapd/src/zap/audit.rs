// SPDX-License-Identifier: AGPL-3.0-only
//! 操作审计日志。
//!
//! 记录管理操作的关键信息（操作者、动作、目标、详情、来源 IP），
//! 供安全追溯与合规审计使用。

use serde::Serialize;
use tracing::error;

use crate::{db, zap::jwt::Claims};

#[derive(Serialize, sqlx::FromRow, Debug)]
pub struct AuditLogRow {
    pub id: i64,
    pub user_id: i64,
    pub username: String,
    pub action: String,
    pub target: String,
    pub detail: String,
    pub ip: String,
    pub created_at: i64,
}

/// 写入一条审计日志（失败仅记录错误日志，不阻塞业务）。
pub async fn log(
    claims: Option<&Claims>,
    ip: Option<&str>,
    action: &str,
    target: &str,
    detail: &str,
) {
    let (user_id, username) = match claims {
        Some(c) => (c.id as i64, c.sub.clone()),
        None => (0, "system".to_string()),
    };
    let pool = db::get_db_pool().await;
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "INSERT INTO audit_logs (user_id, username, action, target, detail, ip, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(username)
    .bind(action)
    .bind(target)
    .bind(detail)
    .bind(ip.unwrap_or(""))
    .bind(now)
    .execute(pool)
    .await;
    if let Err(e) = result {
        error!("写入审计日志失败 (action={action}): {e}");
    }
}

/// 查询审计日志（按时间倒序，分页），返回 (行, 总数)。
pub async fn list_logs(
    page: i64,
    page_size: i64,
    action: Option<&str>,
    username: Option<&str>,
) -> Result<(Vec<AuditLogRow>, i64), sqlx::Error> {
    let pool = db::get_db_pool().await;
    let page = page.max(1);
    let page_size = page_size.clamp(1, 200);
    let offset = (page - 1) * page_size;

    let action_like = action.filter(|s| !s.is_empty()).map(|s| format!("%{s}%"));
    let username_like = username.filter(|s| !s.is_empty()).map(|s| format!("%{s}%"));

    let where_clause = match (&action_like, &username_like) {
        (Some(_), Some(_)) => "WHERE action LIKE ? AND username LIKE ?",
        (Some(_), None) => "WHERE action LIKE ?",
        (None, Some(_)) => "WHERE username LIKE ?",
        (None, None) => "",
    };

    let total_sql = format!("SELECT COUNT(*) FROM audit_logs {where_clause}");
    let mut count_q = sqlx::query_as::<_, (i64,)>(sqlx::AssertSqlSafe(total_sql.as_str()));
    if let Some(a) = &action_like {
        count_q = count_q.bind(a);
    }
    if let Some(u) = &username_like {
        count_q = count_q.bind(u);
    }
    let (total,) = count_q.fetch_one(pool).await?;

    let sql = format!(
        "SELECT id, user_id, username, action, target, detail, ip, created_at
         FROM audit_logs {where_clause} ORDER BY id DESC LIMIT ? OFFSET ?"
    );
    let mut q = sqlx::query_as::<_, AuditLogRow>(sqlx::AssertSqlSafe(sql.as_str()));
    if let Some(a) = &action_like {
        q = q.bind(a);
    }
    if let Some(u) = &username_like {
        q = q.bind(u);
    }
    let rows = q.bind(page_size).bind(offset).fetch_all(pool).await?;
    Ok((rows, total))
}
