//! 面板基础配置（仅 admin）。
//!
//! 「系统设置 → 基础设置」这个页面已下线：Mail 迁到「Zap 设置 → 通知设置」，
//! 建站默认网络与联系信息不再提供界面入口 —— 但存储格式与端点保持不变，
//! 已存配置照旧生效（站点 vhost 同步仍会读这里的默认 IPv4 / IPv6）。
//!
//! 三个 Tab 的配置统一存储于 `{data}/server_env.yaml` 的 conf 区，键名带 `basic_` 前缀，
//! 与运行环境的默认配置（webserver / php_default 等）互不干扰。
//!
//! 端点：
//! - GET  /system/config/basic   读取基础 / Mail / 联系信息（当前面板只消费 mail 一段）
//! - POST /system/config/basic   保存（支持按 Tab 部分提交，未传字段保持不变；
//!   Mail 密码留空表示不改动原密码）
//!
//! **Mail 密码**为敏感项：库中只存 [`crate::zap::crypto`] 的 `v1:` 密文，
//! 读取时仅返回「是否已设置 + 掩码提示」（历史明文值在首次读取时就地加密回写）。
//! 后续发信逻辑取用时应走 `crypto::decrypt_password()`。

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::{Json, extract::Extension};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::crypto;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;
use crate::zap::server_env;

// ── 键定义 ──────────────────────────────────────────────────

/// 基础设置（建站默认网络）：默认 IPv4 / 默认 IPv6 / 网络设备。
///
/// 站点 vhost 同步时读取：非空 → `listen {ip}:80` / `listen {ip}:443 ssl`；
/// 留空（默认）→ 通配 `listen 80`。见 [`crate::routers::site::sync_site_vhost`]。
pub(crate) const K_IPV4: &str = "basic_default_ipv4";
pub(crate) const K_IPV6: &str = "basic_default_ipv6";
pub(crate) const K_IFACE: &str = "basic_network_iface";

/// Mail（发信参数，供后续系统通知 / 工单邮件发送使用）。
const K_MAIL_HOST: &str = "basic_mail_host";
const K_MAIL_PORT: &str = "basic_mail_port";
const K_MAIL_ENCRYPTION: &str = "basic_mail_encryption";
const K_MAIL_FROM: &str = "basic_mail_from";
const K_MAIL_USERNAME: &str = "basic_mail_username";
const K_MAIL_PASSWORD: &str = "basic_mail_password";

/// 联系信息（面板对外展示的客服 / 联系方式）。
const K_CONTACT_NAME: &str = "basic_contact_name";
const K_CONTACT_EMAIL: &str = "basic_contact_email";
const K_CONTACT_QQ: &str = "basic_contact_qq";
const K_CONTACT_WECHAT: &str = "basic_contact_wechat";
const K_CONTACT_PHONE: &str = "basic_contact_phone";
const K_CONTACT_REMARK: &str = "basic_contact_remark";

/// 读取 conf 区全部键值（`{data}/server_env.yaml`）。
fn load_conf() -> HashMap<String, String> {
    server_env::conf_all().into_iter().collect()
}

fn get(conf: &HashMap<String, String>, key: &str) -> String {
    conf.get(key).cloned().unwrap_or_default()
}

/// Mail 密码：读取密文并给出「是否已设置 + 掩码提示」。
///
/// 密码本身永不回显；历史上明文保存的值在这里顺手加密回写一次（一次性迁移）。
fn mail_password_view(conf: &HashMap<String, String>) -> (bool, String) {
    let stored = get(conf, K_MAIL_PASSWORD);
    if stored.is_empty() {
        return (false, String::new());
    }
    if !crypto::is_encrypted(&stored) {
        // 历史明文：就地升级为密文
        server_env::conf_set(
            K_MAIL_PASSWORD,
            &crypto::encrypt_password(&stored),
            "面板基础设置",
        );
    }
    let plain = crypto::decrypt_password(&stored);
    if plain.is_empty() {
        return (false, String::new());
    }
    (true, crypto::mask_secret(&plain))
}

/// 网络候选：来自 zapexec 环境探测快照（`server_env.yaml` 的 auto 区）。
///
/// 返回 `(interfaces, ipv4_all, ipv6_all, default_ipv4, default_ipv6)`，
/// 供面板下拉选择；探测快照缺失时全部为空（此时仍可手工填写）。
fn network_options() -> (Vec<Value>, Vec<String>, Vec<String>, String, String) {
    let Some(payload) = server_env::snapshot().0 else {
        return (
            Vec::new(),
            Vec::new(),
            Vec::new(),
            String::new(),
            String::new(),
        );
    };
    let net = payload.get("network").cloned().unwrap_or(Value::Null);
    let strs = |key: &str| -> Vec<String> {
        net.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let interfaces = net
        .get("interfaces")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let text = |key: &str| -> String {
        net.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    (
        interfaces,
        strs("ipv4_all"),
        strs("ipv6_all"),
        text("default_ipv4"),
        text("default_ipv6"),
    )
}

// ── handlers ────────────────────────────────────────────────

/// GET /system/config/basic
pub async fn basic_get(claims: ValidatedClaims) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可查看通知设置".to_string()));
    }
    let conf = load_conf();
    let (mail_password_set, mail_password_hint) = mail_password_view(&conf);
    let (net_ifaces, net_ipv4, net_ipv6, net_def_v4, net_def_v6) = network_options();
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "basic": {
                "ipv4": get(&conf, K_IPV4),
                "ipv6": get(&conf, K_IPV6),
                "iface": get(&conf, K_IFACE),
                // 下拉候选（zapexec 环境探测）；空数组表示尚未探测到
                "network": {
                    "interfaces": net_ifaces,
                    "ipv4_all": net_ipv4,
                    "ipv6_all": net_ipv6,
                    "default_ipv4": net_def_v4,
                    "default_ipv6": net_def_v6,
                },
            },
            "mail": {
                "host": get(&conf, K_MAIL_HOST),
                "port": get(&conf, K_MAIL_PORT),
                "encryption": get(&conf, K_MAIL_ENCRYPTION),
                "from": get(&conf, K_MAIL_FROM),
                "username": get(&conf, K_MAIL_USERNAME),
                // 密码仅写入不回显（库中为密文），只给「是否已设置」与掩码提示
                "password": "",
                "password_set": mail_password_set,
                "password_hint": mail_password_hint,
            },
            "contact": {
                "name": get(&conf, K_CONTACT_NAME),
                "email": get(&conf, K_CONTACT_EMAIL),
                "qq": get(&conf, K_CONTACT_QQ),
                "wechat": get(&conf, K_CONTACT_WECHAT),
                "phone": get(&conf, K_CONTACT_PHONE),
                "remark": get(&conf, K_CONTACT_REMARK),
            },
        }
    })))
}

#[derive(Debug, Default, Deserialize)]
pub struct BasicPane {
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub iface: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MailPane {
    pub host: Option<String>,
    pub port: Option<String>,
    /// ssl / tls(starttls) / none
    pub encryption: Option<String>,
    pub from: Option<String>,
    pub username: Option<String>,
    /// 留空 = 不修改原密码
    pub password: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ContactPane {
    pub name: Option<String>,
    pub email: Option<String>,
    pub qq: Option<String>,
    pub wechat: Option<String>,
    pub phone: Option<String>,
    pub remark: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct BasicSavePayload {
    pub basic: Option<BasicPane>,
    pub mail: Option<MailPane>,
    pub contact: Option<ContactPane>,
}

/// POST /system/config/basic
pub async fn basic_save(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(payload): Json<BasicSavePayload>,
) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可修改通知设置".to_string()));
    }

    let mut upserts: Vec<(String, String)> = Vec::new();

    // 收集某可选项：Some(v) 即写入（v 可为空串 = 清空该键）
    fn push_opt(
        upserts: &mut Vec<(String, String)>,
        field: &Option<String>,
        key: &'static str,
        limit: usize,
    ) -> Result<(), ZapError> {
        if let Some(v) = field {
            let v = v.trim().to_string();
            if v.len() > limit {
                return Err(ZapError::New(
                    -1,
                    format!("「{key}」长度超限（最大 {limit} 字符）"),
                ));
            }
            upserts.push((key.to_string(), v));
        }
        Ok(())
    }

    if let Some(b) = &payload.basic {
        push_opt(&mut upserts, &b.ipv4, K_IPV4, 64)?;
        push_opt(&mut upserts, &b.ipv6, K_IPV6, 64)?;
        push_opt(&mut upserts, &b.iface, K_IFACE, 32)?;
    }
    if let Some(m) = &payload.mail {
        push_opt(&mut upserts, &m.host, K_MAIL_HOST, 128)?;
        push_opt(&mut upserts, &m.port, K_MAIL_PORT, 8)?;
        if let Some(e) = &m.encryption {
            let e = e.trim().to_string();
            if !["ssl", "tls", "none"].contains(&e.as_str()) {
                return Err(ZapError::New(
                    -1,
                    "加密方式仅支持 ssl / tls / none".to_string(),
                ));
            }
            upserts.push((K_MAIL_ENCRYPTION.to_string(), e));
        }
        push_opt(&mut upserts, &m.from, K_MAIL_FROM, 128)?;
        push_opt(&mut upserts, &m.username, K_MAIL_USERNAME, 128)?;
        if let Some(p) = &m.password {
            let p = p.trim().to_string();
            if !p.is_empty() {
                if p.len() > 256 {
                    return Err(ZapError::New(
                        -1,
                        "「mail.password」长度超限（最大 256 字符）".to_string(),
                    ));
                }
                // 敏感项：密文入库（{data}/server_env.yaml 只存 v1: 密文）
                upserts.push((K_MAIL_PASSWORD.to_string(), crypto::encrypt_password(&p)));
            }
        }
    }
    if let Some(c) = &payload.contact {
        push_opt(&mut upserts, &c.name, K_CONTACT_NAME, 64)?;
        push_opt(&mut upserts, &c.email, K_CONTACT_EMAIL, 128)?;
        push_opt(&mut upserts, &c.qq, K_CONTACT_QQ, 64)?;
        push_opt(&mut upserts, &c.wechat, K_CONTACT_WECHAT, 64)?;
        push_opt(&mut upserts, &c.phone, K_CONTACT_PHONE, 32)?;
        push_opt(&mut upserts, &c.remark, K_CONTACT_REMARK, 512)?;
    }

    // 一次落盘到 {data}/server_env.yaml
    server_env::conf_set_many(&upserts, "面板基础设置");

    let detail = upserts
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let detail_str: &str = if detail.is_empty() {
        "未提交任何配置"
    } else {
        detail.as_str()
    };
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "basic_config_save",
        "system",
        detail_str,
    )
    .await;

    Ok(Json(json!({ "code": 0, "message": "基础设置已保存" })))
}
