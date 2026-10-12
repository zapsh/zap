// SPDX-License-Identifier: AGPL-3.0-only
use axum::{
    Json,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

pub mod acme;
pub mod admin_bootstrap;
pub mod appstore;
pub mod audit;
pub mod auto_update;
pub mod backup_scheduler;
pub mod cache_clean;
pub mod cert_renew;
pub mod certmgr;
pub mod cloud;
pub mod crypto;
pub mod docker_build;
pub mod fastcgi;
pub mod feature;
pub mod global;
pub mod ip_ban;
pub mod job;
pub mod jwt;
pub mod login_history;
pub mod logrotate;
pub mod mail;
pub mod mirror;
pub mod notify;
pub mod plugin_schedule;
pub mod script_cron;
pub mod security_check;
pub mod server_env;
pub mod session;
pub mod system_info;
pub mod task;
pub mod totp;
pub mod types;
pub mod update_config;
pub mod updater;
pub mod usage;
pub mod user_cron;

#[derive(Error, Debug)]
pub enum ZapError {
    /// 处理io错误
    #[error("IO Error: {0}")]
    IOError(#[from] std::io::Error),

    #[error("Json Parse Error : {0}")]
    JsonParseError(#[from] serde_json::Error),

    #[error("Json Web Token Error : {0}")]
    JsonWebTokenError(#[from] jsonwebtoken::errors::Error),

    #[error("DataBase Error : {0}")]
    DataBaseError(#[from] sqlx::Error),

    #[error("404 Not Found")]
    NotFound,

    #[error("{0}")]
    Message(String),

    #[error("{0}")]
    Error(String),

    #[error("{1}")]
    New(i64, String),

    /// 带 i18n key 的错误：前端用 key 命中 locale 翻译，`message` 作为中文兜底。
    #[error("{2}")]
    Key(i64, String, String),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ZapErrorResponse {
    pub code: i64,
    pub message: String,
    /// 前端翻译用的 i18n key（未接 key 的老错误为 null）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

pub type ZapJsonResult = Result<axum::Json<serde_json::Value>, ZapError>;
pub type ZapResult<T> = Result<T, ZapError>;

impl IntoResponse for ZapError {
    fn into_response(self) -> Response {
        let (code, message, key) = match self {
            ZapError::Error(msg) => (1, msg.to_string(), None),
            ZapError::JsonParseError(e) => (2, e.to_string(), None),
            ZapError::JsonWebTokenError(e) => (3, e.to_string(), None),
            ZapError::DataBaseError(e) => (4, e.to_string(), None),
            ZapError::IOError(e) => (5, e.to_string(), None),
            ZapError::NotFound => (404, self.to_string(), None),
            ZapError::Message(msg) => (0, msg.to_string(), None),
            ZapError::New(code, msg) => (code, msg.to_string(), None),
            ZapError::Key(code, k, msg) => (code, msg.to_string(), Some(k)),
        };
        (
            hyper::StatusCode::OK,
            Json(ZapErrorResponse { code, message, key }),
        )
            .into_response()
    }
}

impl ZapError {
    pub fn new(code: i64, message: String) -> ZapJsonResult {
        Err(ZapError::New(code, message))
    }

    /// 带 i18n key 的错误：前端按 key 命中 locale 翻译，`message` 作为中文兜底（未翻译时显示）。
    pub fn key(code: i64, key: impl Into<String>, message: impl Into<String>) -> ZapError {
        ZapError::Key(code, key.into(), message.into())
    }
}

/// 统一成功响应信封：`{ code: 0, message: "ok", data }`。
///
/// 全站响应应统一走这几个 helper，避免各处手写 `json!({code})` 导致信封字段缺失/不一致。
pub fn api_ok(data: impl Serialize) -> ZapJsonResult {
    Ok(Json(json!({ "code": 0, "message": "ok", "data": data })))
}

/// 统一成功响应（无 data 载荷）：`{ code: 0, message }`。
pub fn api_ok_msg(message: impl Into<String>) -> ZapJsonResult {
    Ok(Json(json!({ "code": 0, "message": message.into() })))
}

/// 统一成功响应（自定义 message + data）：`{ code: 0, message, data }`。
pub fn api_ok_data(message: impl Into<String>, data: impl Serialize) -> ZapJsonResult {
    Ok(Json(
        json!({ "code": 0, "message": message.into(), "data": data }),
    ))
}

/// 统一错误响应：`{ code, message }`（等价于 `ZapError::new`，便于与 `api_ok` 配对）。
pub fn api_err(code: i64, message: impl Into<String>) -> ZapJsonResult {
    ZapError::new(code, message.into())
}

/// 统一错误响应（带 i18n key）：`{ code, message, key }`，前端按 key 翻译，message 兜底。
pub fn api_err_key(code: i64, key: impl Into<String>, message: impl Into<String>) -> ZapJsonResult {
    Err(ZapError::key(code, key, message))
}
