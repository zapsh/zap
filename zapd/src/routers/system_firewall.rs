// SPDX-License-Identifier: AGPL-3.0-only
//! 服务器配置 → 防火墙设置（仅 admin）。
//!
//! 面板侧只做「入参校验 + 转发 zapexec 白名单动词」，具体命令由 zapexec 按后端
//! （firewalld / ufw / nftables / iptables）翻译，这里不感知具体后端，
//! 便于后续扩展新的防火墙实现。
//!
//! 端点：
//! - GET  /system/config/firewall                状态 + 端口规则 + 转发 + 黑名单
//! - POST /system/config/firewall/rule/add       新增放行 / 拒绝规则
//! - POST /system/config/firewall/rule/delete    删除规则
//! - POST /system/config/firewall/toggle         启停 / 开机自启
//! - POST /system/config/firewall/forward/add    新增端口转发（DNAT）
//! - POST /system/config/firewall/forward/delete 删除端口转发
//! - POST /system/config/firewall/blacklist/add  IP 黑名单：封禁来源
//! - POST /system/config/firewall/blacklist/delete
//! - GET  /system/config/firewall/export         导出规则 JSON
//! - POST /system/config/firewall/import         导入规则 JSON
//!
//! 安全：面板自身监听端口（server.port）受保护 —— 不允许在该端口上添加 drop
//! 规则、不允许删除它的放行规则、不允许把它转发走，避免把面板自己锁在外面；
//! 黑名单不允许封禁当前登录 IP（防误操作自锁）。

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

/// POST /system/config/firewall/rule/update —— 修改规则（先删后加）
pub async fn firewall_rule_update(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallRuleUpdatePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(zap_proto::Request::FirewallRuleUpdate {
        id: payload.id.clone(),
        port: payload.port,
        proto: payload.proto.clone(),
        action: payload.action.clone(),
        source: payload.source.clone(),
        comment: payload.comment.clone(),
        panel_port: panel_port(),
    })
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_rule_update",
        "firewall",
        &format!(
            "id={} {} {}/{} {} -> {}",
            payload.id, payload.action, payload.proto, payload.port, payload.source, resp.message
        ),
    )
    .await;

    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

#[derive(Debug, Deserialize)]
pub struct FirewallRuleUpdatePayload {
    pub id: String,
    pub port: u16,
    pub proto: String,
    pub action: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub comment: String,
}

#[derive(Debug, Deserialize)]
pub struct FirewallForwardAddPayload {
    #[serde(default = "default_proto")]
    pub proto: String,
    pub listen_port: u16,
    /// 目标地址（IP，不支持域名 / CIDR）
    pub target_addr: String,
    pub target_port: u16,
    #[serde(default)]
    pub comment: String,
}

/// POST /system/config/firewall/forward/add
pub async fn firewall_forward_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallForwardAddPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let detail = format!(
        "{}/{} -> {}:{}",
        payload.proto, payload.listen_port, payload.target_addr, payload.target_port
    );
    let resp = crate::zapexec::call(zap_proto::Request::FirewallForwardAdd {
        proto: payload.proto,
        listen_port: payload.listen_port,
        target_addr: payload.target_addr,
        target_port: payload.target_port,
        comment: payload.comment,
        panel_port: panel_port(),
    })
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_forward_add",
        "firewall",
        &format!("{} -> {}", detail, resp.message),
    )
    .await;

    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

/// POST /system/config/firewall/forward/delete
pub async fn firewall_forward_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallRuleDeletePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(zap_proto::Request::FirewallForwardDelete {
        id: payload.id.clone(),
    })
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_forward_delete",
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
pub struct FirewallBlacklistAddPayload {
    /// IP / CIDR
    pub ip: String,
    #[serde(default)]
    pub comment: String,
}

/// POST /system/config/firewall/blacklist/add
pub async fn firewall_blacklist_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallBlacklistAddPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    // 防自锁：不允许封禁当前登录 IP（精确匹配；网段交由操作者自行斟酌）
    let ip = payload.ip.trim().to_string();
    if ip == client_addr.ip().to_string() {
        return Err(ZapError::New(
            -1,
            "不能封禁当前登录 IP，否则会把自己锁在外面".to_string(),
        ));
    }
    // 白名单优先：命中白名单的 IP 一律拒绝加入黑名单
    let protected = crate::zap::ip_ban::protected_ips().await;
    if protected.iter().any(|w| w == &ip) {
        return Err(ZapError::New(
            -1,
            format!("{ip} 在 IP 白名单中，已拒绝封禁（如需封禁请先移出白名单）"),
        ));
    }
    let comment = payload.comment.trim().to_string();
    let resp = crate::zapexec::call(zap_proto::Request::FirewallBlacklistAdd {
        ip: payload.ip,
        comment: payload.comment,
    })
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_blacklist_add",
        "firewall",
        &format!("ip={} -> {}", ip, resp.message),
    )
    .await;

    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    // 备注记入面板侧黑名单清单（防火墙规则本身多数后端不携带备注）
    if !comment.is_empty() {
        let mut list = bl_load().await;
        if let Some(e) = list
            .iter_mut()
            .find(|e| e.get("ip").and_then(|v| v.as_str()) == Some(ip.as_str()))
        {
            e["comment"] = json!(comment);
        } else {
            list.push(json!({
                "ip": ip,
                "comment": comment,
                "created_at": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            }));
        }
        bl_save(&list).await;
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

/// GET /system/config/firewall/blacklist —— 面板侧黑名单清单（含备注）
pub async fn firewall_blacklist_list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": { "blacklist": bl_load().await },
    })))
}

#[derive(Debug, Deserialize)]
pub struct FirewallBlacklistDeletePayload {
    pub id: String,
    /// 面板清单按 IP 记录备注，删除时一并清理
    #[serde(default)]
    pub ip: String,
}

/// POST /system/config/firewall/blacklist/delete
pub async fn firewall_blacklist_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallBlacklistDeletePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(zap_proto::Request::FirewallBlacklistDelete {
        id: payload.id.clone(),
    })
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_blacklist_delete",
        "firewall",
        &format!("id={} -> {}", payload.id, resp.message),
    )
    .await;

    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    // 同步清理面板侧清单（备注）
    if !payload.ip.trim().is_empty() {
        let ip = payload.ip.trim().to_string();
        let mut list = bl_load().await;
        let before = list.len();
        list.retain(|e| e.get("ip").and_then(|v| v.as_str()) != Some(ip.as_str()));
        if list.len() != before {
            bl_save(&list).await;
        }
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

/// GET /system/config/firewall/export —— 导出全部规则为 JSON
pub async fn firewall_export(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(zap_proto::Request::FirewallExport {
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

/// POST /system/config/firewall/import —— 导入规则（导出文件内容原样回传）
pub async fn firewall_import(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<serde_json::Value>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(zap_proto::Request::FirewallImport {
        payload,
        panel_port: panel_port(),
    })
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_import",
        "firewall",
        &resp.message,
    )
    .await;

    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({
        "code": 0,
        "message": resp.message,
        "data": resp.data,
    })))
}

/// POST /system/config/firewall/lockdown —— 一键锁服 / 恢复放行
pub async fn firewall_lockdown(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallLockdownPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(zap_proto::Request::FirewallLockdown {
        panel_port: panel_port(),
        unlock: payload.unlock,
    })
    .await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        if payload.unlock {
            "firewall_unlock"
        } else {
            "firewall_lockdown"
        },
        "firewall",
        &resp.message,
    )
    .await;

    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

#[derive(Debug, Deserialize)]
pub struct FirewallLockdownPayload {
    #[serde(default)]
    pub unlock: bool,
}

// ── IP 白名单（放行优先，命中后不再被封禁）────────────────────

const WL_KEY: &str = "firewall.whitelist";

/// 面板侧黑名单存储（`[{ip, comment, created_at}]`）。
///
/// 防火墙后端的 drop 规则多数不携带备注，备注存在这份清单里，
/// 列表时按 IP 合并展示；IP Ban 的自动封禁（zap:ban:*）不进清单。
const BL_KEY: &str = "firewall.blacklist";

async fn bl_load() -> Vec<serde_json::Value> {
    let pool = crate::db::get_db_pool().await;
    let raw: Option<(String,)> = sqlx::query_as("SELECT value FROM global_settings WHERE key=?")
        .bind(BL_KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    raw.and_then(|r| serde_json::from_str(&r.0).ok())
        .unwrap_or_default()
}

async fn bl_save(list: &[serde_json::Value]) {
    let pool = crate::db::get_db_pool().await;
    let v = serde_json::to_string(list).unwrap_or_else(|_| "[]".to_string());
    let _ = sqlx::query(
        "INSERT INTO global_settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(BL_KEY)
    .bind(v)
    .execute(pool)
    .await;
}

/// 面板侧白名单存储（`[{ip, comment, created_at}]`）。
///
/// 与防火墙后端的 accept 规则是两回事：
/// - 后端规则决定「流量放行」；
/// - 这份清单决定「面板绝不允许把它加进黑名单 / IP Ban 不封它」。
async fn wl_load() -> Vec<serde_json::Value> {
    let pool = crate::db::get_db_pool().await;
    let raw: Option<(String,)> = sqlx::query_as("SELECT value FROM global_settings WHERE key=?")
        .bind(WL_KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    raw.and_then(|r| serde_json::from_str(&r.0).ok())
        .unwrap_or_default()
}

async fn wl_save(list: &[serde_json::Value]) {
    let pool = crate::db::get_db_pool().await;
    let v = serde_json::to_string(list).unwrap_or_else(|_| "[]".to_string());
    let _ = sqlx::query(
        "INSERT INTO global_settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(WL_KEY)
    .bind(v)
    .execute(pool)
    .await;
}

/// GET /system/config/firewall/whitelist
pub async fn firewall_whitelist_list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(zap_proto::Request::FirewallWhitelistList).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    let backend_rules = resp
        .data
        .as_ref()
        .and_then(|d| d.get("whitelist"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": { "whitelist": wl_load().await, "rules": backend_rules },
    })))
}

#[derive(Debug, Deserialize)]
pub struct FirewallWhitelistAddPayload {
    pub ip: String,
    #[serde(default)]
    pub comment: String,
}

/// POST /system/config/firewall/whitelist/add —— 同时写面板清单与防火墙 accept 规则
pub async fn firewall_whitelist_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallWhitelistAddPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let ip = payload.ip.trim().to_string();
    // 合法性校验交给后端白名单动词（IPv4 / IPv6 均支持）
    let resp = crate::zapexec::call(zap_proto::Request::FirewallWhitelistAdd {
        ip: ip.clone(),
        comment: payload.comment.clone(),
    })
    .await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_whitelist_add",
        "firewall",
        &format!("ip={ip} -> {}", resp.message),
    )
    .await;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    let mut list = wl_load().await;
    if !list
        .iter()
        .any(|x| x.get("ip").and_then(|v| v.as_str()) == Some(ip.as_str()))
    {
        list.push(json!({
            "ip": ip,
            "comment": payload.comment,
            "created_at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }));
        wl_save(&list).await;
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

#[derive(Debug, Deserialize)]
pub struct FirewallWhitelistDeletePayload {
    pub ip: String,
}

/// POST /system/config/firewall/whitelist/delete —— 删面板清单 + 防火墙 accept 规则
pub async fn firewall_whitelist_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallWhitelistDeletePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let ip = payload.ip.trim().to_string();
    // 防火墙规则 id：多数后端就是 IP 本身；nft 是 handle、firewalld 带前缀，
    // 因此从 status 里按 IP 反查真实 id 再删。
    let mut rule_id = ip.clone();
    let st = crate::zapexec::call(zap_proto::Request::FirewallStatus {
        panel_port: panel_port(),
    })
    .await;
    if let Ok(r) = st {
        if r.code == 0 {
            if let Some(rules) = r
                .data
                .as_ref()
                .and_then(|d| d.get("whitelist"))
                .and_then(|v| v.as_array())
            {
                if let Some(found) = rules.iter().find(|x| {
                    x.get("ip").and_then(|v| v.as_str()) == Some(ip.as_str())
                }) {
                    rule_id = found
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&ip)
                        .to_string();
                }
            }
        }
    }
    let resp = crate::zapexec::call(zap_proto::Request::FirewallWhitelistDelete { id: rule_id })
        .await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_whitelist_delete",
        "firewall",
        &format!("ip={ip} -> {}", resp.message),
    )
    .await;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    let list: Vec<serde_json::Value> = wl_load()
        .await
        .into_iter()
        .filter(|x| x.get("ip").and_then(|v| v.as_str()) != Some(ip.as_str()))
        .collect();
    wl_save(&list).await;
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

// ── IP Ban（fail2ban）──────────────────────────────────────────

/// GET /system/config/firewall/ban/status
pub async fn firewall_ban_status(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let s = crate::zap::ip_ban::settings().await;
    let st = crate::zap::ip_ban::state().await;
    let bans = crate::zap::ip_ban::records().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let watching: Vec<serde_json::Value> = st
        .hits
        .iter()
        .map(|(k, (n, at))| {
            let (rule, ip) = crate::zap::ip_ban::split_hit_key(k);
            json!({ "rule": rule, "ip": ip, "hits": n, "last": at })
        })
        .collect();
    let ignored: Vec<serde_json::Value> = st
        .ignored
        .iter()
        .map(|(k, (n, reason, _))| {
            let (rule, ip) = crate::zap::ip_ban::split_hit_key(k);
            json!({ "rule": rule, "ip": ip, "hits": n, "reason": reason })
        })
        .collect();
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": {
            "settings": s,
            "bans": bans.iter().map(|b| json!({
                "ip": b.ip,
                "reason": b.reason,
                "hits": b.hits,
                "created_at": b.created_at,
                "expires_at": b.expires_at,
                "remaining": b.expires_at.saturating_sub(now),
            })).collect::<Vec<_>>(),
            "watching": watching,
            "ignored": ignored,
            "sources": st.sources,
            "last_run": st.last_run,
            "total_banned": st.total_banned,
            "last_error": st.last_error,
        },
    })))
}

/// POST /system/config/firewall/ban/settings
pub async fn firewall_ban_settings_save(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<crate::zap::ip_ban::BanSettings>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    crate::zap::ip_ban::save_settings(&payload).await;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "firewall_ban_settings",
        "firewall",
        &format!(
            "enabled={} rules={} ban_time={} interval={}",
            payload.enabled,
            payload.rules.len(),
            payload.ban_time,
            payload.interval
        ),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "已保存" })))
}

#[derive(Debug, Deserialize)]
pub struct FirewallBanUnbanPayload {
    pub ip: String,
}

/// POST /system/config/firewall/ban/unban —— 手动提前解封
pub async fn firewall_ban_unban(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FirewallBanUnbanPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    match crate::zap::ip_ban::unban(&payload.ip).await {
        Ok(()) => {
            audit::log(
                Some(&claims),
                Some(client_addr.ip().to_string().as_str()),
                "firewall_ban_unban",
                "firewall",
                &format!("ip={}", payload.ip),
            )
            .await;
            Ok(Json(json!({ "code": 0, "message": "已解封" })))
        }
        Err(e) => Err(ZapError::New(-1, e)),
    }
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
