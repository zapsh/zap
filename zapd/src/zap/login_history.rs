//! 登录记录：个人中心「最近登录」的数据来源。
//!
//! 与审计日志（`audit_logs`）分开建表，是为了让「我的登录记录」这类高频个人查询
//! 不必在整站操作日志里捞；同时它额外保存了 `user_agent`，便于判断登录来源。
//!
//! 记录同时覆盖**失败**尝试：用户看到「有人试过我的密码」比事后查审计更直观。

use serde::Serialize;
use tracing::error;

use crate::db;

/// 登录结果。
pub const STATUS_SUCCESS: &str = "success";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_2FA_FAILED: &str = "2fa_failed";

/// 每个（用户, 用户名）保留的记录条数上限，超出部分写入后即时修剪。
///
/// 用户名一并参与是因为失败尝试的 `user_id` 恒为 0（账号不存在/未匹配），
/// 只按 user_id 修剪会把所有未知账号的记录挤成一团。
const KEEP_PER_USER: i64 = 50;

#[derive(Serialize, sqlx::FromRow, Debug)]
pub struct LoginRecord {
    pub id: i64,
    pub user_id: i64,
    pub username: String,
    pub ip: String,
    pub user_agent: String,
    pub status: String,
    pub created_at: i64,
}

/// 写入一条登录记录（失败仅记日志，不阻断登录流程）。
///
/// `user_id` 传 0 表示账号未能匹配到用户（用户名不存在 / 密码错误）。
pub async fn record(user_id: i64, username: &str, ip: &str, user_agent: &str, status: &str) {
    let pool = db::get_db_pool().await;
    let now = chrono::Utc::now().timestamp();
    let r = sqlx::query(
        "INSERT INTO login_history (user_id, username, ip, user_agent, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(username)
    .bind(ip)
    .bind(user_agent)
    .bind(status)
    .bind(now)
    .execute(pool)
    .await;
    if let Err(e) = r {
        error!("写入登录记录失败 (username={username}): {e}");
        return;
    }
    trim(user_id, username).await;
}

/// 只保留最近 [`KEEP_PER_USER`] 条，其余删除。
async fn trim(user_id: i64, username: &str) {
    let pool = db::get_db_pool().await;
    let r = sqlx::query(
        "DELETE FROM login_history
         WHERE user_id = ? AND username = ?
           AND id NOT IN (
             SELECT id FROM login_history
             WHERE user_id = ? AND username = ?
             ORDER BY id DESC LIMIT ?
           )",
    )
    .bind(user_id)
    .bind(username)
    .bind(user_id)
    .bind(username)
    .bind(KEEP_PER_USER)
    .execute(pool)
    .await;
    if let Err(e) = r {
        error!("修剪登录记录失败 (username={username}): {e}");
    }
}

/// 查询某用户的登录记录（按时间倒序，分页），返回 (行, 总数)。
pub async fn list(
    user_id: i64,
    page: i64,
    page_size: i64,
) -> Result<(Vec<LoginRecord>, i64), sqlx::Error> {
    let pool = db::get_db_pool().await;
    let page = page.max(1);
    let page_size = page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM login_history WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(pool)
        .await?;

    let rows = sqlx::query_as::<_, LoginRecord>(
        "SELECT id, user_id, username, ip, user_agent, status, created_at
         FROM login_history WHERE user_id = ?
         ORDER BY id DESC LIMIT ? OFFSET ?",
    )
    .bind(user_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok((rows, total))
}
