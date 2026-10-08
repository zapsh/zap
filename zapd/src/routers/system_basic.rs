// SPDX-License-Identifier: AGPL-3.0-only
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
use crate::db;
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

/// Mail 发信渠道：`smtp` / `sendgrid` / `aliyun` / `tencent`（默认 `smtp`，向后兼容）。
const K_MAIL_PROVIDER: &str = "basic_mail_provider";
/// SendGrid API Key（敏感，密文）。
const K_MAIL_SG_KEY: &str = "basic_mail_sg_apikey";
/// 阿里云邮件推送 AccessKey / AccessSecret（Secret 敏感，密文）。
const K_MAIL_ALI_KEY: &str = "basic_mail_aliyun_key";
const K_MAIL_ALI_SECRET: &str = "basic_mail_aliyun_secret";
const K_MAIL_ALI_REGION: &str = "basic_mail_aliyun_region";
/// 腾讯云 SES SecretId / SecretKey（Key 敏感，密文）。
const K_MAIL_TC_ID: &str = "basic_mail_tencent_id";
const K_MAIL_TC_KEY: &str = "basic_mail_tencent_key";
const K_MAIL_TC_REGION: &str = "basic_mail_tencent_region";

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

/// 敏感项（密码 / API Key / Secret）：读取密文并给出「是否已设置 + 掩码提示」。
///
/// 明文永不回显；密文解密失败（含历史明文）按「未设置」处理。
fn secret_view(conf: &HashMap<String, String>, key: &str) -> (bool, String) {
    let stored = get(conf, key);
    if stored.is_empty() {
        return (false, String::new());
    }
    let plain = crypto::decrypt_password(&stored);
    if plain.is_empty() {
        return (false, String::new());
    }
    (true, crypto::mask_secret(&plain))
}

/// Mail 密码视图（沿用通用 secret_view）。
fn mail_password_view(conf: &HashMap<String, String>) -> (bool, String) {
    secret_view(conf, K_MAIL_PASSWORD)
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
    let (sg_key_set, sg_key_hint) = secret_view(&conf, K_MAIL_SG_KEY);
    let (ali_secret_set, ali_secret_hint) = secret_view(&conf, K_MAIL_ALI_SECRET);
    let (tc_key_set, tc_key_hint) = secret_view(&conf, K_MAIL_TC_KEY);
    let (net_ifaces, net_ipv4, net_ipv6, net_def_v4, net_def_v6) = network_options();
    // provider 默认值（向后兼容：未配置视为 smtp）
    let mail_provider = {
        let p = get(&conf, K_MAIL_PROVIDER);
        if p.is_empty() {
            "smtp".to_string()
        } else {
            p
        }
    };
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
                "provider": mail_provider,
                "from": get(&conf, K_MAIL_FROM),
                // ── SMTP ──
                "host": get(&conf, K_MAIL_HOST),
                "port": get(&conf, K_MAIL_PORT),
                "encryption": get(&conf, K_MAIL_ENCRYPTION),
                "username": get(&conf, K_MAIL_USERNAME),
                "password": "",
                "password_set": mail_password_set,
                "password_hint": mail_password_hint,
                // ── SendGrid ──
                "sg_api_key": "",
                "sg_api_key_set": sg_key_set,
                "sg_api_key_hint": sg_key_hint,
                // ── 阿里云 ──
                "aliyun_access_key": get(&conf, K_MAIL_ALI_KEY),
                "aliyun_access_secret": "",
                "aliyun_access_secret_set": ali_secret_set,
                "aliyun_access_secret_hint": ali_secret_hint,
                "aliyun_region": get(&conf, K_MAIL_ALI_REGION),
                // ── 腾讯云 ──
                "tencent_secret_id": get(&conf, K_MAIL_TC_ID),
                "tencent_secret_key": "",
                "tencent_secret_key_set": tc_key_set,
                "tencent_secret_key_hint": tc_key_hint,
                "tencent_region": get(&conf, K_MAIL_TC_REGION),
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
    /// smtp / sendgrid / aliyun / tencent
    pub provider: Option<String>,
    pub host: Option<String>,
    pub port: Option<String>,
    /// ssl / tls(starttls) / none
    pub encryption: Option<String>,
    pub from: Option<String>,
    pub username: Option<String>,
    /// 留空 = 不修改原密码
    pub password: Option<String>,
    // ── SendGrid ──
    /// 留空 = 不修改
    pub sg_api_key: Option<String>,
    // ── 阿里云 DirectMail ──
    pub aliyun_access_key: Option<String>,
    /// 留空 = 不修改
    pub aliyun_access_secret: Option<String>,
    pub aliyun_region: Option<String>,
    // ── 腾讯云 SES ──
    pub tencent_secret_id: Option<String>,
    /// 留空 = 不修改
    pub tencent_secret_key: Option<String>,
    pub tencent_region: Option<String>,
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

    // 敏感项（密码 / API Key / Secret）：非空才写入（密文入库），空串表示不改动（保留原值）。
    fn push_secret(
        upserts: &mut Vec<(String, String)>,
        field: &Option<String>,
        key: &'static str,
        limit: usize,
    ) -> Result<(), ZapError> {
        if let Some(p) = field {
            let p = p.trim().to_string();
            if p.is_empty() {
                return Ok(());
            }
            if p.len() > limit {
                return Err(ZapError::New(
                    -1,
                    format!("「{key}」长度超限（最大 {limit} 字符）"),
                ));
            }
            let enc = crypto::encrypt_password(&p).map_err(|e| ZapError::New(-1, e))?;
            upserts.push((key.to_string(), enc));
        }
        Ok(())
    }

    if let Some(b) = &payload.basic {
        push_opt(&mut upserts, &b.ipv4, K_IPV4, 64)?;
        push_opt(&mut upserts, &b.ipv6, K_IPV6, 64)?;
        push_opt(&mut upserts, &b.iface, K_IFACE, 32)?;
    }
    if let Some(m) = &payload.mail {
        // 渠道（默认 smtp，向后兼容旧客户端）
        if let Some(p) = &m.provider {
            let p = p.trim().to_string();
            if !["smtp", "sendgrid", "aliyun", "tencent"].contains(&p.as_str()) {
                return Err(ZapError::New(
                    -1,
                    "发信渠道仅支持 smtp / sendgrid / aliyun / tencent".to_string(),
                ));
            }
            upserts.push((K_MAIL_PROVIDER.to_string(), p));
        }
        push_opt(&mut upserts, &m.from, K_MAIL_FROM, 128)?;
        // ── SMTP ──
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
        push_opt(&mut upserts, &m.username, K_MAIL_USERNAME, 128)?;
        push_secret(&mut upserts, &m.password, K_MAIL_PASSWORD, 256)?;
        // ── SendGrid ──
        push_secret(&mut upserts, &m.sg_api_key, K_MAIL_SG_KEY, 256)?;
        // ── 阿里云 ──
        push_opt(&mut upserts, &m.aliyun_access_key, K_MAIL_ALI_KEY, 128)?;
        push_secret(&mut upserts, &m.aliyun_access_secret, K_MAIL_ALI_SECRET, 256)?;
        push_opt(&mut upserts, &m.aliyun_region, K_MAIL_ALI_REGION, 32)?;
        // ── 腾讯云 ──
        push_opt(&mut upserts, &m.tencent_secret_id, K_MAIL_TC_ID, 128)?;
        push_secret(&mut upserts, &m.tencent_secret_key, K_MAIL_TC_KEY, 256)?;
        push_opt(&mut upserts, &m.tencent_region, K_MAIL_TC_REGION, 32)?;
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

// ── 通知邮件模板（admin 全局 / reseller 私有）────────────────
/// 支持自定义模板的事件（与 [`crate::zap::notify::builtin_template`] 保持一致）。
const MAIL_EVENTS: &[&str] = &[
    "login_success",
    "password_change",
    "site_created",
    "disk_low",
];

/// GET /system/mail/templates
///
/// 返回当前登录者「可编辑范围」下各事件的**当前生效**模板（含纯文本与 HTML 两种正文、
/// 以及是否 HTML 标记）：
/// - 管理员 → 全局覆盖（作用域 `global`，owner_id=0），影响所有用户；
/// - reseller → 自身覆盖（作用域 `self`，owner_id=其自身），叠加在全局之上，仅影响其名下用户。
///
/// 展示文本 = 内置默认 → 全局覆盖 → 自身覆盖（依次叠加），让用户看到真实生效内容。
pub async fn mail_templates_get(claims: ValidatedClaims) -> ZapJsonResult {
    let is_adm = is_admin(&claims);
    let scope = if is_adm { "global" } else { "self" };
    // 管理员编辑全局（owner_id=0）；reseller 编辑自身覆盖（owner_id=其 id）
    let owner_id = if is_adm { 0 } else { claims.id as i64 };

    let mut map = serde_json::Map::new();
    for &ev in MAIL_EVENTS {
        let rt = crate::zap::notify::resolve_template(ev, owner_id).await;
        map.insert(
            ev.to_string(),
            json!({
                "subject": rt.subject,
                "body_text": rt.body_text,
                "body_html": rt.body_html,
                "is_html": rt.is_html,
            }),
        );
    }

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "scope": scope,
            "events": Value::Object(map),
        }
    })))
}

#[derive(Debug, Default, Deserialize)]
pub struct MailTemplateEvent {
    subject: Option<String>,
    body_text: Option<String>,
    body_html: Option<String>,
    is_html: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MailTemplatePayload {
    /// 事件 -> { subject, body_text, body_html, is_html }；仅提交需要改的事件，未提交事件保留原值。
    pub events: HashMap<String, MailTemplateEvent>,
}

/// POST /system/mail/templates
///
/// 保存当前登录者作用域下的模板覆盖，统一写入 `mail_templates` 表：
/// - 管理员 → owner_id=0（全局）；
/// - reseller → owner_id=自身（私有，仅影响其名下用户）。
///
/// 仅提交的事件被覆盖（按 `(owner_id, event)` upsert），其余事件保留；非法事件名 / 超长会被拒绝。
pub async fn mail_templates_save(
    claims: ValidatedClaims,
    Json(payload): Json<MailTemplatePayload>,
) -> ZapJsonResult {
    let is_adm = is_admin(&claims);
    let owner_id = if is_adm { 0 } else { claims.id as i64 };
    let now = chrono::Local::now().timestamp();
    let pool = db::get_db_pool().await;

    for (ev, t) in &payload.events {
        if !MAIL_EVENTS.contains(&ev.as_str()) {
            return Err(ZapError::New(-1, format!("未知事件类型: {ev}")));
        }
        let subject = t.subject.clone().unwrap_or_default();
        let body_text = t.body_text.clone().unwrap_or_default();
        let body_html = t.body_html.clone().unwrap_or_default();
        let is_html = t.is_html.unwrap_or(false);
        if subject.len() > 256 || body_text.len() > 16384 || body_html.len() > 16384 {
            return Err(ZapError::New(-1, "模板内容过长（主题≤256，正文≤16384）".to_string()));
        }
        sqlx::query(
            "INSERT INTO mail_templates (owner_id, event, subject, body_text, body_html, is_html, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(owner_id, event) DO UPDATE SET \
               subject=excluded.subject, body_text=excluded.body_text, body_html=excluded.body_html, \
               is_html=excluded.is_html, updated_at=excluded.updated_at",
        )
        .bind(owner_id)
        .bind(ev)
        .bind(&subject)
        .bind(&body_text)
        .bind(&body_html)
        .bind(if is_html { 1i64 } else { 0i64 })
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| ZapError::New(-1, format!("保存失败: {e}")))?;
    }

    Ok(Json(json!({ "code": 0, "message": "模板已保存" })))
}

#[derive(Debug, Default, Deserialize)]
pub struct MailTestPayload {
    /// 接收测试邮件的邮箱。
    pub to: String,
}

/// POST /system/mail/test —— 仅管理员：用当前发信渠道向指定邮箱发送一封测试邮件。
pub async fn mail_test_send(
    claims: ValidatedClaims,
    Json(payload): Json<MailTestPayload>,
) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可发送测试邮件".to_string()));
    }
    let to = payload.to.trim().to_string();
    if to.is_empty() || !to.contains('@') {
        return Err(ZapError::New(-1, "请填写有效的接收邮箱".to_string()));
    }
    match crate::zap::mail::send(
        &to,
        "【ZAP】邮件发送测试",
        "<p>这是一封来自 ZAP 的测试邮件，说明您的邮件发信渠道配置正确。</p>",
        true,
        None,
    )
    .await
    {
        Ok(()) => Ok(Json(json!({ "code": 0, "message": "测试邮件已发送，请检查收件箱" }))),
        Err(e) => Err(ZapError::New(-1, format!("发送失败: {e}"))),
    }
}
