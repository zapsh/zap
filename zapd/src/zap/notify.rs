// SPDX-License-Identifier: AGPL-3.0-only
//! 站内信通知中心。
//!
//! 事件发生时向 `notice_message` 写入一条站内信；是否真正送达取决于
//! 该用户在「个人中心 → 偏好设置」中的通知偏好（存 `user.prefs`，JSON）。

use serde_json::Value;
use tracing::error;

use crate::db;

/// 依据用户通知偏好判定某类通知是否放行。
///
/// 规则：主开关（notify_key，如 `notify_login`）为 false 则不放行；
/// 子开关（disable_key，如 `login_disable`）为 true 表示“禁用”该类通知。
/// 老库 / 未配置过偏好时，使用调用方给定的默认值。
async fn prefs_allows(
    user_id: i64,
    notify_key: &str,
    disable_key: &str,
    notify_default: bool,
) -> bool {
    let pool = db::get_db_pool().await;
    let raw: Option<String> = sqlx::query_scalar("SELECT prefs FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let prefs: Value = raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);

    let notify = prefs
        .get(notify_key)
        .and_then(|v| v.as_bool())
        .unwrap_or(notify_default);
    if !notify {
        return false;
    }
    let disable = prefs
        .get(disable_key)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    !disable
}

/// 写入一条站内信（失败仅记录错误日志，不阻塞业务）。
pub async fn push(user_id: i64, msg_type: &str, title: &str, body: &str) {
    let pool = db::get_db_pool().await;
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = sqlx::query(
        "INSERT INTO notice_message (user_id, type, title, body, is_read, created_at)
         VALUES (?, ?, ?, ?, 0, ?)",
    )
    .bind(user_id)
    .bind(msg_type)
    .bind(title)
    .bind(body)
    .bind(now)
    .execute(pool)
    .await
    {
        error!("写入站内信失败 (user_id={user_id}, type={msg_type}): {e}");
    }
}

/// 登录成功通知（偏好：登录成功通知，默认不通知）。
pub async fn login_success(user_id: i64, username: &str, ip: &str) {
    if !prefs_allows(user_id, "notify_login", "login_disable", false).await {
        return;
    }
    let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let body = format!(
        "您的账户 {username} 于 {time} 通过 IP {ip} 成功登录。若非本人操作，请立即修改密码或联系管理员。"
    );
    push(user_id, "login_success", "登录成功通知", &body).await;
}

/// 账户密码变更通知（偏好：账户密码变更通知，默认通知）。
/// `operator` 为执行改密操作的用户名（本人或管理员/经销商）。
pub async fn password_changed(user_id: i64, operator: &str) {
    if !prefs_allows(
        user_id,
        "notify_password_change",
        "password_change_disable",
        true,
    )
    .await
    {
        return;
    }
    let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let body = format!(
        "您的账户密码已于 {time} 被修改（操作者：{operator}）。若非本人操作，请立即联系管理员。"
    );
    push(user_id, "password_change", "账户密码变更通知", &body).await;
}
