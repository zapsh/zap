// SPDX-License-Identifier: AGPL-3.0-only
//! 站内信（通知中心）用户端接口：列表 / 未读数 / 已读 / 全部已读 / 删除。
//! 数据来源：`notice_message` 表，事件侧按用户偏好（user.prefs）决定是否写入。

use axum::{Json, extract::Query};
use serde::Deserialize;
use serde_json::json;

use crate::{
    db,
    zap::{ZapJsonResult, jwt::ValidatedClaims},
};

#[derive(Deserialize)]
pub struct NoticesQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    10
}

#[derive(sqlx::FromRow, serde::Serialize, Debug)]
pub struct NoticeRow {
    pub id: i64,
    /// 通知类型（事件名）：login_success / password_change 等
    pub r#type: String,
    pub title: String,
    pub body: String,
    pub is_read: i64,
    pub created_at: i64,
}

/// GET /user/notices?page=1&page_size=10 —— 分页列表，附未读数。
pub async fn notices_list(
    claims: ValidatedClaims,
    Query(query): Query<NoticesQuery>,
) -> ZapJsonResult {
    let uid = claims.id as i64;
    let pool = db::get_db_pool().await;
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notice_message WHERE user_id = ?")
        .bind(uid)
        .fetch_one(pool)
        .await?;
    let (unread_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM notice_message WHERE user_id = ? AND is_read = 0")
            .bind(uid)
            .fetch_one(pool)
            .await?;
    let list = sqlx::query_as::<_, NoticeRow>(
        "SELECT id, type, title, body, is_read, created_at
         FROM notice_message WHERE user_id = ?
         ORDER BY id DESC LIMIT ? OFFSET ?",
    )
    .bind(uid)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": { "list": list, "total": total, "unread_count": unread_count },
    })))
}

/// GET /user/notices/unread —— 未读消息数（顶栏铃铛轮询）。
pub async fn notices_unread(claims: ValidatedClaims) -> ZapJsonResult {
    let uid = claims.id as i64;
    let pool = db::get_db_pool().await;
    let (unread,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM notice_message WHERE user_id = ? AND is_read = 0")
            .bind(uid)
            .fetch_one(pool)
            .await?;
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": { "unread": unread },
    })))
}

#[derive(Deserialize)]
pub struct ReadPayload {
    pub id: i64,
}

/// POST /user/notices/read —— 标记单条已读（仅限本人的消息）。
pub async fn notices_read(
    claims: ValidatedClaims,
    Json(payload): Json<ReadPayload>,
) -> ZapJsonResult {
    let uid = claims.id as i64;
    let pool = db::get_db_pool().await;
    sqlx::query("UPDATE notice_message SET is_read = 1 WHERE id = ? AND user_id = ?")
        .bind(payload.id)
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(Json(json!({ "code": 0, "message": "已标记为已读" })))
}

/// POST /user/notices/read_all —— 全部标记已读。
pub async fn notices_read_all(claims: ValidatedClaims) -> ZapJsonResult {
    let uid = claims.id as i64;
    let pool = db::get_db_pool().await;
    sqlx::query("UPDATE notice_message SET is_read = 1 WHERE user_id = ? AND is_read = 0")
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(Json(json!({ "code": 0, "message": "全部已读" })))
}

#[derive(Deserialize)]
pub struct DeletePayload {
    #[serde(default)]
    pub ids: Vec<i64>,
}

/// POST /user/notices/delete —— 删除消息（仅限本人的消息）。
pub async fn notices_delete(
    claims: ValidatedClaims,
    Json(payload): Json<DeletePayload>,
) -> ZapJsonResult {
    let uid = claims.id as i64;
    let pool = db::get_db_pool().await;
    for id in &payload.ids {
        sqlx::query("DELETE FROM notice_message WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(uid)
            .execute(pool)
            .await?;
    }
    Ok(Json(json!({ "code": 0, "message": "删除成功" })))
}
