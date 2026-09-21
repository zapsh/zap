//! 服务器配置 → 防火墙设置（仅 admin）。
//!
//! 面板侧只做「入参校验 + 转发 zapexec 白名单动词」，具体命令由 zapexec 按后端
//! （firewalld / ufw / nftables / iptables）翻译，这里不感知具体后端，
//! 便于后续扩展新的防火墙实现。
//!
//! 端点：
//! - GET  /system/config/firewall             状态 + 规则列表
//! - POST /system/config/firewall/rule/add    新增放行 / 拒绝规则
//! - POST /system/config/firewall/rule/delete 删除规则
//! - POST /system/config/firewall/toggle      启停 / 开机自启
//!
//! 安全：面板自身监听端口（server.port）受保护 —— 不允许在该端口上添加 drop
//! 规则，也不允许删除它的放行规则，避免把面板自己锁在外面。

use std::net::SocketAddr;

use axum::{Json, extract::Extension};
use serde::Deserialize;
use serde_json::json;

use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;

/// 当前面板监听端口（用于自杀保护）
fn panel_port() -> u16 {
    crate::config::get_config()
        .read()
        .map(|c| c.server.port)
        .unwrap_or(0)
}

fn require_admin(claims: &ValidatedClaims) -> Result<(), ZapError> {
    if is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "仅管理员可管理防火墙".to_string()))
    }
}

/// GET /system/config/firewall
pub async fn firewall_status(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(zap_proto::Request::FirewallStatus {
        panel_port: panel_port(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({
        "code": 0,
        "message": resp.message,
        "data": resp.data,
    })))
}

#[derive(Debug, Deserialize)]
pub struct FirewallRuleAddPayload {
    pub port: u16,
    #[serde(default = "default_proto")]
    pub proto: String,
    #[serde(default = "default_action")]
    pub action: String,
    /// 来源 IP / CIDR，空表示不限来源
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub comment: String,
}

fn default_proto() -> String {
    "tcp".to_string()
}
fn default_action() -> String {
    "accept".to_string()
}

/// POST /system/config/firewall/rule/add
pub async fn firewall_rule_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallRuleAddPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let detail = format!(
        "port={} proto={} action={} source={} comment={}",
        payload.port, payload.proto, payload.action, payload.source, payload.comment
    );
    let resp = crate::zapexec::call(zap_proto::Request::FirewallRuleAdd {
        port: payload.port,
        proto: payload.proto,
        action: payload.action,
        source: payload.source,
        comment: payload.comment,
        panel_port: panel_port(),
    })
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_rule_add",
        "firewall",
        &format!("{} -> {}", detail, resp.message),
    )
    .await;

    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

#[derive(Debug, Deserialize)]
pub struct FirewallRuleDeletePayload {
    pub id: String,
}

/// POST /system/config/firewall/rule/delete
pub async fn firewall_rule_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallRuleDeletePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(zap_proto::Request::FirewallRuleDelete {
        id: payload.id.clone(),
        panel_port: panel_port(),
    })
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_rule_delete",
        "firewall",
        &format!("id={} -> {}", payload.id, resp.message),
    )
    .await;

    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

#[derive(Debug, Deserialize)]
pub struct FirewallTogglePayload {
    /// start | stop | enable | disable
    pub action: String,
}

/// POST /system/config/firewall/toggle
pub async fn firewall_toggle(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallTogglePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if !matches!(
        payload.action.as_str(),
        "start" | "stop" | "enable" | "disable"
    ) {
        return Err(ZapError::New(
            -1,
            "动作仅支持 start / stop / enable / disable".to_string(),
        ));
    }
    let resp = crate::zapexec::call(zap_proto::Request::FirewallToggle {
        action: payload.action.clone(),
    })
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_toggle",
        "firewall",
        &format!("action={} -> {}", payload.action, resp.message),
    )
    .await;

    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}
