// SPDX-License-Identifier: AGPL-3.0-only
use axum::Json;
use serde_json::json;

use crate::zap::{
    self, ZapError, ZapJsonResult,
    jwt::{self, ValidatedClaims},
};

/// 全局任务调度器的启停属于系统级操作，仅管理员可操作。
fn require_admin(claims: &jwt::Claims) -> Result<(), ZapError> {
    if jwt::is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "权限不足，需要管理员权限".to_string()))
    }
}

pub async fn stop_job(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    zap::job::stop_system_job().await;
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
    })))
}

pub async fn start_job(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    zap::job::start_system_job().await;
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
    })))
}
