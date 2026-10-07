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

/// 读取某用户的 prefs JSON（空则 Null）。
async fn read_user_prefs(user_id: i64) -> Value {
    let pool = db::get_db_pool().await;
    let raw: Option<String> = sqlx::query_scalar("SELECT prefs FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

/// 写回某用户的 prefs JSON。
async fn write_user_prefs(user_id: i64, prefs: Value) -> Result<(), ZapError> {
    let pool = db::get_db_pool().await;
    let s = serde_json::to_string(&prefs)
        .map_err(|e| ZapError::New(-1, format!("prefs 序列化失败: {e}")))?;
    sqlx::query("UPDATE user SET prefs = ? WHERE id = ?")
        .bind(s)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| ZapError::New(-1, format!("保存失败: {e}")))?;
    Ok(())
}

/// 读取当前生效的「全局覆盖」模板（存于 server_env `mail_templates`）。
fn global_template_override() -> Value {
    server_env::conf_get("mail_templates")
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null)
}

/// GET /system/mail/templates
///
/// 返回当前登录者「可编辑范围」下各事件的**当前生效**模板：
/// - 管理员 → 全局覆盖（作用域 `global`），影响所有 owner_id=0 的用户通知；
/// - reseller → 自身覆盖（作用域 `self`），叠加在全局之上，仅影响其名下用户。
///
/// 展示文本 = 内置默认 → 全局覆盖 → 自身覆盖（依次叠加），让用户看到真实生效内容。
pub async fn mail_templates_get(claims: ValidatedClaims) -> ZapJsonResult {
    let is_adm = is_admin(&claims);
    let scope = if is_adm { "global" } else { "self" };
    let global_ov = global_template_override();
    let self_ov = if is_adm {
        Value::Null
    } else {
        read_user_prefs(claims.id as i64).await
    };

    let mut map = serde_json::Map::new();
    for &ev in MAIL_EVENTS {
        let (mut subj, mut body) = crate::zap::notify::builtin_template(ev);
        if let Some(t) = global_ov.get(ev) {
            if let Some(s) = t.get("subject").and_then(|x| x.as_str()) {
                subj = s.to_string();
            }
            if let Some(b) = t.get("body").and_then(|x| x.as_str()) {
                body = b.to_string();
            }
        }
        if !is_adm {
            if let Some(t) = self_ov.get("mail_templates").and_then(|x| x.get(ev)) {
                if let Some(s) = t.get("subject").and_then(|x| x.as_str()) {
                    subj = s.to_string();
                }
                if let Some(b) = t.get("body").and_then(|x| x.as_str()) {
                    body = b.to_string();
                }
            }
        }
        map.insert(ev.to_string(), json!({ "subject": subj, "body": body }));
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
    body: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MailTemplatePayload {
    /// 事件 -> { subject, body }；仅提交需要改的事件，未提交事件保留原值。
    pub events: HashMap<String, MailTemplateEvent>,
}

/// POST /system/mail/templates
///
/// 保存当前登录者作用域下的模板覆盖：
/// - 管理员 → 写入 server_env `mail_templates`（全局）；
/// - reseller → 写入自身 `user.prefs.mail_templates`（私有）。
///
/// 仅提交的事件被覆盖，其余事件保留；非法事件名 / 超长会被拒绝。
pub async fn mail_templates_save(
    claims: ValidatedClaims,
    Json(payload): Json<MailTemplatePayload>,
) -> ZapJsonResult {
    let is_adm = is_admin(&claims);

    let mut override_map: serde_json::Map<String, Value> = if is_adm {
        global_template_override()
            .as_object()
            .cloned()
            .unwrap_or_default()
    } else {
        read_user_prefs(claims.id as i64)
            .await
            .get("mail_templates")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    };

    for (ev, t) in &payload.events {
        if !MAIL_EVENTS.contains(&ev.as_str()) {
            return Err(ZapError::New(-1, format!("未知事件类型: {ev}")));
        }
        let subject = t.subject.clone().unwrap_or_default();
        let body = t.body.clone().unwrap_or_default();
        if subject.len() > 256 || body.len() > 4096 {
            return Err(ZapError::New(-1, "模板内容过长（主题≤256，正文≤4096）".to_string()));
        }
        override_map.insert(ev.clone(), json!({ "subject": subject, "body": body }));
    }

    if is_adm {
        let s = serde_json::to_string(&Value::Object(override_map))
            .map_err(|e| ZapError::New(-1, format!("序列化失败: {e}")))?;
        server_env::conf_set_many(&[("mail_templates".to_string(), s)], "邮件通知模板");
    } else {
        let mut prefs = read_user_prefs(claims.id as i64).await;
        if prefs.is_null() {
            prefs = Value::Object(serde_json::Map::new());
        }
        prefs
            .as_object_mut()
            .unwrap()
            .insert("mail_templates".to_string(), Value::Object(override_map));
        write_user_prefs(claims.id as i64, prefs).await?;
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
    )
    .await
    {
        Ok(()) => Ok(Json(json!({ "code": 0, "message": "测试邮件已发送，请检查收件箱" }))),
        Err(e) => Err(ZapError::New(-1, format!("发送失败: {e}"))),
    }
}
