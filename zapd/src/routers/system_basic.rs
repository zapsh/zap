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
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{Json, extract::Extension, extract::Path};
use sqlx::{QueryBuilder, Sqlite};
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
/// Mailgun API Key（敏感，密文）。
const K_MG_KEY: &str = "basic_mail_mailgun_key";
const K_MG_DOMAIN: &str = "basic_mail_mailgun_domain";
const K_MG_REGION: &str = "basic_mail_mailgun_region";

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
    let (mg_key_set, mg_key_hint) = secret_view(&conf, K_MG_KEY);
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
                // ── Mailgun ──
                "mailgun_api_key": "",
                "mailgun_api_key_set": mg_key_set,
                "mailgun_api_key_hint": mg_key_hint,
                "mailgun_domain": get(&conf, K_MG_DOMAIN),
                "mailgun_region": get(&conf, K_MG_REGION),
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
    // ── Mailgun ──
    pub mailgun_api_key: Option<String>,
    /// 留空 = 不修改
    pub mailgun_domain: Option<String>,
    /// us（默认）/ eu
    pub mailgun_region: Option<String>,
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
            if !["smtp", "sendgrid", "aliyun", "tencent", "mailgun"].contains(&p.as_str()) {
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
        // ── Mailgun ──
        push_secret(&mut upserts, &m.mailgun_api_key, K_MG_KEY, 256)?;
        push_opt(&mut upserts, &m.mailgun_domain, K_MG_DOMAIN, 128)?;
        push_opt(&mut upserts, &m.mailgun_region, K_MG_REGION, 8)?;
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
                "template_id": rt.template_id,
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
    template_id: Option<String>,
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
        let template_id = t.template_id.clone().unwrap_or_default();
        if subject.len() > 256 || body_text.len() > 16384 || body_html.len() > 16384 {
            return Err(ZapError::New(-1, "模板内容过长（主题≤256，正文≤16384）".to_string()));
        }
        if template_id.len() > 128 {
            return Err(ZapError::New(-1, "模板 ID 过长（最大 128 字符）".to_string()));
        }
        sqlx::query(
            "INSERT INTO mail_templates (owner_id, event, subject, body_text, body_html, is_html, template_id, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(owner_id, event) DO UPDATE SET \
               subject=excluded.subject, body_text=excluded.body_text, body_html=excluded.body_html, \
               is_html=excluded.is_html, template_id=excluded.template_id, updated_at=excluded.updated_at",
        )
        .bind(owner_id)
        .bind(ev)
        .bind(&subject)
        .bind(&body_text)
        .bind(&body_html)
        .bind(if is_html { 1i64 } else { 0i64 })
        .bind(&template_id)
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
        None,
        None,
    )
    .await
    {
        Ok(()) => Ok(Json(json!({ "code": 0, "message": "测试邮件已发送，请检查收件箱" }))),
        Err(e) => Err(ZapError::New(-1, format!("发送失败: {e}"))),
    }
}

/// POST /system/mail/broadcast —— admin 全局 / reseller 名下客户：主动向客户群发通知。
///
/// 收件人按角色自动收敛，无需（也不能）在前端指定越权范围：
/// - admin：全平台所有「启用」账号；
/// - reseller：仅自己名下（owner_id = 自身）的客户。
///
/// 渠道 `channel`：
/// - `inbox`：仅站内信（落 `notice_message`，无需邮箱）；
/// - `email`：仅邮件（需邮箱，无邮箱的账号自动跳过）；
/// - `both`：两者都发（默认）。
///
/// 若传 `user_ids` 则在这些账号内再筛选（reseller 传入的越权 id 会被 owner 条件自动剔除）。
/// 主题 / 正文由调用方自由撰写（可先在前端复用通知模板预填后再改），后端只负责投递。
#[derive(Debug, Deserialize)]
pub struct MailBroadcastPayload {
    /// 通知主题（必填）
    pub subject: String,
    /// 纯文本正文（站内信恒用此字段；邮件非 HTML 模式必填，亦作 HTML 兜底）
    #[serde(default)]
    pub body_text: String,
    /// HTML 正文（is_html=true 且含邮件渠道时必填）
    #[serde(default)]
    pub body_html: Option<String>,
    /// 邮件是否为 HTML 格式（默认纯文本）
    #[serde(default)]
    pub is_html: bool,
    /// 发送渠道：inbox=仅站内信，email=仅邮件，both=两者都发（默认 both）
    #[serde(default)]
    pub channel: Option<String>,
    /// 指定收件人 id 列表；留空 = 当前角色作用域内的全部客户
    #[serde(default)]
    pub user_ids: Option<Vec<i64>>,
}

pub async fn mail_broadcast_send(
    claims: ValidatedClaims,
    Json(payload): Json<MailBroadcastPayload>,
) -> ZapJsonResult {
    let is_adm = is_admin(&claims);
    let subject = payload.subject.trim();
    if subject.is_empty() {
        return Err(ZapError::New(-1, "通知主题不能为空".to_string()));
    }
    let channel = payload.channel.clone().unwrap_or_else(|| "both".to_string());
    let do_email = channel == "email" || channel == "both";
    let do_inbox = channel == "inbox" || channel == "both";
    if !do_email && !do_inbox {
        return Err(ZapError::New(-1, "请选择有效的发送渠道".to_string()));
    }

    let body_text = payload.body_text.trim();
    let body_html = payload.body_html.as_deref().unwrap_or("").trim();
    if do_email && !payload.is_html && body_text.is_empty() {
        return Err(ZapError::New(-1, "邮件纯文本正文不能为空".to_string()));
    }
    if do_email && payload.is_html && body_html.is_empty() {
        return Err(ZapError::New(-1, "邮件 HTML 正文不能为空".to_string()));
    }
    if do_inbox && body_text.is_empty() {
        return Err(ZapError::New(-1, "站内信正文不能为空".to_string()));
    }

    // 收件人：启用状态；reseller 仅限名下（owner_id=自身），admin 不限。
    // 站内信不依赖邮箱，所以这里不再强制 email 非空（邮件渠道再按 email 过滤）。
    let pool = db::get_db_pool().await;
    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new("SELECT id, username, email FROM user WHERE status = 1");
    if !is_adm {
        qb.push(" AND owner_id = ").push_bind(claims.id as i64);
    }
    if let Some(ids) = &payload.user_ids {
        if !ids.is_empty() {
            qb.push(" AND id IN (");
            let mut first = true;
            for id in ids {
                if !first {
                    qb.push(", ");
                }
                qb.push_bind(*id);
                first = false;
            }
            qb.push(")");
        }
    }
    let recipients: Vec<(i64, String, String)> = qb.build_query_as().fetch_all(pool).await?;
    if recipients.is_empty() {
        return Ok(Json(json!({ "code": 0, "message": "没有符合条件的收件人" })));
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut sent_email = 0i64;
    let mut sent_inbox = 0i64;
    let mut failed: Vec<Value> = Vec::new();

    for (id, username, email) in recipients {
        // 站内信：无邮箱也能收到，直接落 notice_message（type=broadcast）
        if do_inbox {
            match sqlx::query(
                "INSERT INTO notice_message (user_id, type, title, body, is_read, created_at) \
                 VALUES (?, 'broadcast', ?, ?, 0, ?)",
            )
            .bind(id)
            .bind(subject)
            .bind(body_text)
            .bind(now)
            .execute(pool)
            .await
            {
                Ok(_) => sent_inbox += 1,
                Err(e) => failed.push(json!({
                    "username": username,
                    "error": format!("站内信写入失败: {e}")
                })),
            }
        }

        // 邮件：需要邮箱，无邮箱的账号跳过（站内信已单独计入）
        if do_email {
            if email.is_empty() {
                if !do_inbox {
                    failed.push(json!({
                        "username": username,
                        "error": "该账号未填写邮箱，已跳过"
                    }));
                }
                continue;
            }
            let (body, alt) = if payload.is_html {
                (body_html, Some(body_text))
            } else {
                (body_text, None)
            };
            match crate::zap::mail::send(&email, subject, body, payload.is_html, alt, None, None).await {
                Ok(()) => sent_email += 1,
                Err(e) => failed.push(json!({ "email": email, "username": username, "error": e })),
            }
        }
    }

    let mut parts: Vec<String> = Vec::new();
    if do_email {
        parts.push(format!("邮件 {sent_email} 封"));
    }
    if do_inbox {
        parts.push(format!("站内信 {sent_inbox} 条"));
    }
    let message = if failed.is_empty() {
        format!("已发送：{}", parts.join("，"))
    } else {
        format!("{}，失败 {} 项", parts.join("，"), failed.len())
    };
    Ok(Json(json!({
        "code": 0,
        "message": message,
        "data": { "sent_email": sent_email, "sent_inbox": sent_inbox, "failed": failed }
    })))
}

// ── 群发通知模板（用户自建命名模板，如「维护通知」）─────────────────
//
// 作用域：
// - `global`：owner_id = 0，仅管理员可见 / 可建可改可删；
// - `self`：owner_id = 创建者自身，私有（reseller 只能建这种）。
// 列表返回「全局 + 自己私有」，供群发时一键预填。

#[derive(Debug, Deserialize)]
pub struct BroadcastTemplatePayload {
    /// 模板名称（必填）
    pub name: String,
    /// 作用域：global / self；留空按 self 处理（创建时非管理员强制 self）
    #[serde(default)]
    pub scope: Option<String>,
    /// 主题（必填）
    pub subject: String,
    /// 纯文本正文
    #[serde(default)]
    pub body_text: String,
    /// HTML 正文（is_html=true 时使用）
    #[serde(default)]
    pub body_html: Option<String>,
    /// 是否 HTML 格式
    #[serde(default)]
    pub is_html: bool,
}

#[derive(sqlx::FromRow, serde::Serialize)]
struct BroadcastTemplateRow {
    id: i64,
    owner_id: i64,
    scope: String,
    name: String,
    subject: String,
    body_text: String,
    body_html: String,
    is_html: i64,
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 权限：global 仅管理员可改删；self 管理员可改删任意，非管理员只能改删 owner=自身 的。
fn can_edit_template(claims: &ValidatedClaims, owner_id: i64, scope: &str) -> bool {
    if is_admin(claims) {
        return true;
    }
    scope == "self" && owner_id == claims.id as i64
}

/// GET /system/broadcast/templates —— 列表：全局 + 自己私有。
pub async fn broadcast_templates_list(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let rows = sqlx::query_as::<_, BroadcastTemplateRow>(
        "SELECT id, owner_id, scope, name, subject, body_text, body_html, is_html \
         FROM broadcast_templates WHERE scope = 'global' OR owner_id = ? ORDER BY id DESC",
    )
    .bind(claims.id as i64)
    .fetch_all(pool)
    .await?;
    let list: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "owner_id": r.owner_id,
                "scope": r.scope,
                "name": r.name,
                "subject": r.subject,
                "body_text": r.body_text,
                "body_html": r.body_html,
                "is_html": r.is_html != 0,
            })
        })
        .collect();
    Ok(Json(json!({ "code": 0, "message": "ok", "data": { "list": list } })))
}

/// POST /system/broadcast/templates —— 新建。
pub async fn broadcast_templates_create(
    claims: ValidatedClaims,
    Json(payload): Json<BroadcastTemplatePayload>,
) -> ZapJsonResult {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(ZapError::New(-1, "模板名称不能为空".to_string()));
    }
    if payload.subject.trim().is_empty() {
        return Err(ZapError::New(-1, "模板主题不能为空".to_string()));
    }
    let scope = payload.scope.clone().unwrap_or_else(|| "self".to_string());
    if scope != "global" && scope != "self" {
        return Err(ZapError::New(-1, "scope 只能是 global 或 self".to_string()));
    }
    if scope == "global" && !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可创建全局模板".to_string()));
    }
    let owner_id = if scope == "global" { 0 } else { claims.id as i64 };
    let now = now_ts();
    let pool = db::get_db_pool().await;
    let body_html = payload.body_html.as_deref().unwrap_or("").trim();
    let res = sqlx::query(
        "INSERT INTO broadcast_templates \
         (owner_id, scope, name, subject, body_text, body_html, is_html, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(owner_id)
    .bind(&scope)
    .bind(name)
    .bind(payload.subject.trim())
    .bind(payload.body_text.trim())
    .bind(body_html)
    .bind(payload.is_html as i64)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(Json(json!({
        "code": 0,
        "message": "模板已创建",
        "data": { "id": res.last_insert_rowid() }
    })))
}

/// PUT /system/broadcast/templates/:id —— 编辑（不改 scope / owner）。
pub async fn broadcast_templates_update(
    claims: ValidatedClaims,
    Path(id): Path<i64>,
    Json(payload): Json<BroadcastTemplatePayload>,
) -> ZapJsonResult {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(ZapError::New(-1, "模板名称不能为空".to_string()));
    }
    if payload.subject.trim().is_empty() {
        return Err(ZapError::New(-1, "模板主题不能为空".to_string()));
    }
    let pool = db::get_db_pool().await;
    let (owner_id, scope): (i64, String) =
        sqlx::query_as("SELECT owner_id, scope FROM broadcast_templates WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| ZapError::New(-1, "模板不存在".to_string()))?;
    if !can_edit_template(&claims, owner_id, &scope) {
        return Err(ZapError::New(-1, "无权编辑该模板".to_string()));
    }
    let body_html = payload.body_html.as_deref().unwrap_or("").trim();
    sqlx::query(
        "UPDATE broadcast_templates SET name = ?, subject = ?, body_text = ?, \
         body_html = ?, is_html = ?, updated_at = ? WHERE id = ?",
    )
    .bind(name)
    .bind(payload.subject.trim())
    .bind(payload.body_text.trim())
    .bind(body_html)
    .bind(payload.is_html as i64)
    .bind(now_ts())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(Json(json!({ "code": 0, "message": "模板已更新" })))
}

/// DELETE /system/broadcast/templates/:id —— 删除。
pub async fn broadcast_templates_delete(
    claims: ValidatedClaims,
    Path(id): Path<i64>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let (owner_id, scope): (i64, String) =
        sqlx::query_as("SELECT owner_id, scope FROM broadcast_templates WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| ZapError::New(-1, "模板不存在".to_string()))?;
    if !can_edit_template(&claims, owner_id, &scope) {
        return Err(ZapError::New(-1, "无权删除该模板".to_string()));
    }
    sqlx::query("DELETE FROM broadcast_templates WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(Json(json!({ "code": 0, "message": "模板已删除" })))
}
