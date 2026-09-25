//! 服务配置 → 总览（管理员）：应用商店已安装应用中「登记了 systemd 服务」的实例。
//!
//! 端点（均需管理员，均透传 zapexec 返回）：
//! - GET  /system/services/overview   服务卡片（状态 / 开机自启 / 版本）
//! - POST /system/services/control    启停（body: svc, action）
//! - POST /system/services/boot       开机自启（body: svc, enable）
//!
//! 可操作的 unit 名单由 zapexec 扫 `info.yaml` 的 `svc_name` 得出，不在 zapd 侧
//! 硬编码服务名 —— 发行版 unit 叫法不同（php-fpm-8.3 / mysql / nginx），
//! 交给安装脚本登记、exec 侧校验，避免这里再维护一份名单。

use std::net::SocketAddr;

use axum::Json;
use axum::extract::Extension;
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

/// GET /system/services/overview
pub async fn overview(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::ServicesOverview).await
}

#[derive(Debug, Deserialize)]
pub struct ControlBody {
    /// systemd unit 名（必须已由某个已装应用登记）
    pub svc: String,
    /// start / stop / restart / reload
    pub action: String,
}

/// POST /system/services/control
pub async fn control(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<ControlBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let result = exec(Request::ServicesControl {
        svc: body.svc.clone(),
        action: body.action.clone(),
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "services_control",
            "services",
            &format!("对服务 {} 执行 {}", body.svc, body.action),
        )
        .await;
    }
    result
}

#[derive(Debug, Deserialize)]
pub struct BootBody {
    pub svc: String,
    pub enable: bool,
}

/// POST /system/services/boot
pub async fn boot(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<BootBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let result = exec(Request::ServicesBoot {
        svc: body.svc.clone(),
        enable: body.enable,
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "services_boot",
            "services",
            &format!(
                "{}服务 {} 的开机自启",
                if body.enable { "开启" } else { "关闭" },
                body.svc
            ),
        )
        .await;
    }
    result
}
