// SPDX-License-Identifier: AGPL-3.0-only
//! 审计日志查询接口（仅管理员）。

use axum::{Json, extract::Query};
use serde::Deserialize;
use serde_json::json;

use crate::zap::{
    ZapError, ZapJsonResult, audit,
    jwt::{ValidatedClaims, is_admin},
};

#[derive(Deserialize)]
pub struct AuditListQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    20
}

/// GET /system/audit/list?page=1&page_size=20&action=&username=
pub async fn audit_list(
    claims: ValidatedClaims,
    Query(query): Query<AuditListQuery>,
) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可查看审计日志".to_string()));
    }
    let (rows, total) = audit::list_logs(
        query.page,
        query.page_size,
        query.action.as_deref(),
        query.username.as_deref(),
    )
    .await?;
    Ok(Json(json!({
        "code": 0,
        "data": rows,
        "total": total,
    })))
}
