use axum::{
    Json,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod acme;
pub mod appstore;
pub mod audit;
pub mod auto_update;
pub mod certmgr;
pub mod cloud;
pub mod crypto;
pub mod docker_build;
pub mod fastcgi;
pub mod global;
pub mod job;
pub mod jwt;
pub mod logrotate;
pub mod notify;
pub mod script_cron;
pub mod server_env;
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
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ZapErrorResponse {
    pub code: i64,
    pub message: String,
}

pub type ZapJsonResult = Result<axum::Json<serde_json::Value>, ZapError>;
pub type ZapResult<T> = Result<T, ZapError>;

impl IntoResponse for ZapError {
    fn into_response(self) -> Response {
        let (code, message) = match self {
            ZapError::Error(msg) => (1, msg.to_string()),
            ZapError::JsonParseError(e) => (2, e.to_string()),
            ZapError::JsonWebTokenError(e) => (3, e.to_string()),
            ZapError::DataBaseError(e) => (4, e.to_string()),
            ZapError::IOError(e) => (5, e.to_string()),
            ZapError::NotFound => (404, self.to_string()),
            ZapError::Message(msg) => (0, msg.to_string()),
            ZapError::New(code, msg) => (code, msg.to_string()),
        };
        (
            hyper::StatusCode::OK,
            Json(ZapErrorResponse { code, message }),
        )
            .into_response()
    }
}

impl ZapError {
    pub fn new(code: i64, message: String) -> ZapJsonResult {
        Err(ZapError::New(code, message))
    }
}
