// SPDX-License-Identifier: AGPL-3.0-only
//! Nginx 服务配置与运行状态（服务器配置 → Nginx 配置 / 服务器状态 → Nginx Server）。
//!
//! 端点（均需管理员）：
//! - GET  /system/nginx/status            状态探测（未安装时 installed=false）
//! - GET  /system/nginx/config            列出可编辑配置（主配置 + conf 白名单）
//! - GET  /system/nginx/config/content    读取指定配置内容（Query: path）
//! - POST /system/nginx/config/save       保存配置（备份 → 写入 → nginx -t → 回滚/重载）
//! - POST /system/nginx/control           服务控制 start/stop/restart/reload
//! - POST /system/nginx/default-vhost     设置默认站点（IP 访问开关）{ enable }
//! - GET  /system/nginx/stub-status       查询状态页 stub_status 并采集指标
//! - POST /system/nginx/stub-status       开启 / 关闭状态页 { enable }

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{Extension, Query};
use serde::Deserialize;
use serde_json::json;

use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;
use zap_proto::Request;

fn require_admin(claims: &ValidatedClaims) -> Result<(), ZapError> {
    if is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "仅管理员可访问".to_string()))
    }
}

/// 执行一次 zapexec 请求，透传其 code/message/data。
async fn exec(req: Request) -> Result<Json<serde_json::Value>, ZapError> {
    let resp = crate::zapexec::call(req).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": resp.data }),
    ))
}

/// GET /system/nginx/status
pub async fn nginx_status(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::NginxStatus).await
}

/// GET /system/nginx/config
pub async fn nginx_conf_list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::NginxConfList).await
}

#[derive(Debug, Deserialize)]
pub struct NginxReadQuery {
    pub path: String,
}

/// GET /system/nginx/config/content?path=...
pub async fn nginx_conf_read(
    claims: ValidatedClaims,
    Query(q): Query<NginxReadQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::NginxConfRead { path: q.path }).await
}

#[derive(Debug, Deserialize)]
pub struct NginxSaveBody {
    pub path: String,
    pub content: String,
}

/// POST /system/nginx/config/save
pub async fn nginx_conf_save(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<NginxSaveBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let result = exec(Request::NginxConfSave {
        path: body.path.clone(),
        content: body.content.clone(),
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "nginx_conf_save",
            "nginx",
            &format!("保存 Nginx 配置 {}", body.path),
        )
        .await;
    }
    result
}

#[derive(Debug, Deserialize)]
pub struct NginxControlBody {
    pub action: String,
}

/// POST /system/nginx/control
pub async fn nginx_control(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<NginxControlBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if !matches!(
        body.action.as_str(),
        "start" | "stop" | "restart" | "reload"
    ) {
        return Err(ZapError::New(
            -1,
            "仅支持 start / stop / restart / reload".to_string(),
        ));
    }
    let result = exec(Request::NginxControl {
        action: body.action.clone(),
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "nginx_control",
            "nginx",
            &format!("Nginx 服务操作 {}", body.action),
        )
        .await;
    }
    result
}

/// GET /system/nginx/stub-status（查询状态页并采集指标）
pub async fn nginx_stub_status_get(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::NginxStubStatus { enable: None }).await
}

#[derive(Debug, Deserialize)]
pub struct NginxStubStatusBody {
    pub enable: bool,
}

/// POST /system/nginx/stub-status（开启 / 关闭状态页）
pub async fn nginx_stub_status_set(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<NginxStubStatusBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let result = exec(Request::NginxStubStatus {
        enable: Some(body.enable),
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "nginx_stub_status",
            "nginx",
            if body.enable {
                "启用 Nginx 状态页 stub_status"
            } else {
                "关闭 Nginx 状态页 stub_status"
            },
        )
        .await;
    }
    result
}

#[derive(Debug, Deserialize)]
pub struct NginxDefaultVhostBody {
    pub enable: bool,
}

/// POST /system/nginx/default-vhost
pub async fn nginx_default_vhost(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<NginxDefaultVhostBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let result = exec(Request::NginxDefaultVhost {
        enable: body.enable,
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "nginx_default_vhost",
            "nginx",
            if body.enable {
                "开启默认站点（IP / 未匹配域名显示欢迎页）"
            } else {
                "关闭默认站点（IP / 未匹配域名直接断开）"
            },
        )
        .await;
    }
    result
}
