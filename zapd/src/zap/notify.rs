// SPDX-License-Identifier: AGPL-3.0-only
//! 站内信通知中心。
//!
//! 事件发生时向 `notice_message` 写入一条站内信；是否真正送达取决于
//! 该用户在「个人中心 → 偏好设置」中的通知偏好（存 `user.prefs`，JSON）。

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use tracing::error;

use crate::db;
use crate::zap::mail;
use crate::zap::server_env;

/// 依据用户通知偏好判定某类通知是否放行。
///
/// 规则：主开关（notify_key，如 `notify_login`）为 false 则不放行；
/// 子开关（disable_key，如 `login_disable`）为 true 表示“禁用”该类通知。
/// 老库 / 未配置过偏好时，使用调用方给定的默认值。
async fn prefs_allows(
    user_id: i64,
    notify_key: &str,
    disable_key: &str,
    notify_default: bool,
) -> bool {
    let pool = db::get_db_pool().await;
    let raw: Option<String> = sqlx::query_scalar("SELECT prefs FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let prefs: Value = raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);

    let notify = prefs
        .get(notify_key)
        .and_then(|v| v.as_bool())
        .unwrap_or(notify_default);
    if !notify {
        return false;
    }
    let disable = prefs
        .get(disable_key)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    !disable
}

/// 写入一条站内信（失败仅记录错误日志，不阻塞业务）。
pub async fn push(user_id: i64, msg_type: &str, title: &str, body: &str) {
    let pool = db::get_db_pool().await;
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = sqlx::query(
        "INSERT INTO notice_message (user_id, type, title, body, is_read, created_at)
         VALUES (?, ?, ?, ?, 0, ?)",
    )
    .bind(user_id)
    .bind(msg_type)
    .bind(title)
    .bind(body)
    .bind(now)
    .execute(pool)
    .await
    {
        error!("写入站内信失败 (user_id={user_id}, type={msg_type}): {e}");
    }
}

/// 读取用户「默认发送渠道」偏好（存 `user.prefs.notify_channels`，数组）。
///
/// 取值：`site`（站内信）/ `email`（邮件）。未配置或为空时回退为仅 `site`。
async fn prefs_channels(user_id: i64) -> HashSet<String> {
    let pool = db::get_db_pool().await;
    let raw: Option<String> = sqlx::query_scalar("SELECT prefs FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let prefs: Value = raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let mut set = HashSet::new();
    if let Some(arr) = prefs.get("notify_channels").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                set.insert(s.to_string());
            }
        }
    }
    if set.is_empty() {
        set.insert("site".to_string());
    }
    set
}

/// 登录成功通知（偏好：登录成功通知，默认不通知）。
pub async fn login_success(user_id: i64, username: &str, ip: &str) {
    if !prefs_allows(user_id, "notify_login", "login_disable", false).await {
        return;
    }
    let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let body = format!(
        "您的账户 {username} 于 {time} 通过 IP {ip} 成功登录。若非本人操作，请立即修改密码或联系管理员。"
    );
    let channels = prefs_channels(user_id).await;
    if channels.contains("site") {
        push(user_id, "login_success", "登录成功通知", &body).await;
    }
    if channels.contains("email") {
        let mut p = HashMap::new();
        p.insert("username", username.to_string());
        p.insert("time", time);
        p.insert("ip", ip.to_string());
        email_user(user_id, "login_success", &p).await;
    }
}

/// 账户密码变更通知（偏好：账户密码变更通知，默认通知）。
/// `operator` 为执行改密操作的用户名（本人或管理员/经销商）。
pub async fn password_changed(user_id: i64, operator: &str) {
    if !prefs_allows(
        user_id,
        "notify_password_change",
        "password_change_disable",
        true,
    )
    .await
    {
        return;
    }
    let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let body = format!(
        "您的账户密码已于 {time} 被修改（操作者：{operator}）。若非本人操作，请立即联系管理员。"
    );
    let channels = prefs_channels(user_id).await;
    if channels.contains("site") {
        push(user_id, "password_change", "账户密码变更通知", &body).await;
    }
    if channels.contains("email") {
        let mut p = HashMap::new();
        p.insert("time", time);
        p.insert("operator", operator.to_string());
        email_user(user_id, "password_change", &p).await;
    }
}

// ── 邮件渠道 + 模板 ───────────────────────────────────────────

/// 渲染模板：将 `{{key}}` 占位符替换为参数值。
fn render(template: &str, params: &HashMap<&str, String>) -> String {
    let mut s = template.to_string();
    for (k, v) in params {
        s = s.replace(&format!("{{{{{}}}}}", k), v);
    }
    s
}

/// 读取用户元信息（邮箱 / 归属 reseller / 偏好 JSON）。
async fn load_user_meta(user_id: i64) -> Option<(String, i64, Value)> {
    let pool = db::get_db_pool().await;
    let row: Option<(String, i64, Option<String>)> = sqlx::query_as(
        "SELECT email, owner_id, prefs FROM user WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let (email, owner_id, prefs_raw) = row?;
    let prefs: Value = prefs_raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    Some((email, owner_id, prefs))
}

/// 模板解析：内置默认 → 全局覆盖（server_env `mail_templates`）→ reseller 自定义覆盖。
///
/// `owner_id` 为收件用户的归属 reseller；reseller 可在自己的偏好 `mail_templates` 里
/// 自定义其名下用户通知的模版（主题/正文），实现「经销商自定义模板」需求。
async fn resolve_template(event: &str, owner_id: i64) -> (String, String) {
    let (mut subj, mut body) = builtin_template(event);
    if let Some(raw) = server_env::conf_get("mail_templates") {
        if let Ok(map) = serde_json::from_str::<Value>(&raw) {
            if let Some(t) = map.get(event) {
                if let Some(s) = t.get("subject").and_then(|x| x.as_str()) {
                    subj = s.to_string();
                }
                if let Some(b) = t.get("body").and_then(|x| x.as_str()) {
                    body = b.to_string();
                }
            }
        }
    }
    if owner_id != 0 {
        if let Some((_, _, owner_prefs)) = load_user_meta(owner_id).await {
            if let Some(t) = owner_prefs.get("mail_templates").and_then(|x| x.get(event)) {
                if let Some(s) = t.get("subject").and_then(|x| x.as_str()) {
                    subj = s.to_string();
                }
                if let Some(b) = t.get("body").and_then(|x| x.as_str()) {
                    body = b.to_string();
                }
            }
        }
    }
    (subj, body)
}

/// 给单个用户发邮件（是否真正发送由调用方按全局渠道 `notify_channels` 控制；
/// 此处仅校验邮箱非空与发信配置，未配置则静默跳过；失败仅记录）。
async fn email_user(user_id: i64, event: &str, params: &HashMap<&str, String>) {
    let Some((email, owner_id, _)) = load_user_meta(user_id).await else {
        return;
    };
    if email.trim().is_empty() {
        return;
    }
    let (subj_t, body_t) = resolve_template(event, owner_id).await;
    let subject = render(&subj_t, params);
    let body = render(&body_t, params);
    if let Err(e) = mail::send(&email, &subject, &body).await {
        mail::log_send_error(event, &e);
    }
}

/// 内置默认模板（管理员 / reseller 未自定义时使用）。
pub fn builtin_template(event: &str) -> (String, String) {
    match event {
        "login_success" => (
            "【ZAP】登录成功通知".into(),
            "您的账户于 {{time}} 通过 IP {{ip}} 成功登录。若非本人操作，请立即修改密码或联系管理员。".into(),
        ),
        "password_change" => (
            "【ZAP】账户密码变更通知".into(),
            "您的账户密码已于 {{time}} 被修改（操作者：{{operator}}）。若非本人操作，请立即联系管理员。".into(),
        ),
        "site_created" => (
            "【ZAP】站点创建成功".into(),
            "您创建的站点「{{site}}」已成功部署。可在面板中管理该站点的域名、SSL 与运行设置。".into(),
        ),
        "disk_low" => (
            "【ZAP】磁盘空间不足预警".into(),
            "您的磁盘用量已达 {{pct}}%，即将超过套餐配额。请及时清理文件或联系管理员升级套餐。".into(),
        ),
        _ => ("【ZAP】系统通知".into(), "您有一条新的系统通知。".into()),
    }
}

/// 站点创建成功：按全局渠道下发（受发信配置控制；未配置邮件则静默跳过）。
pub async fn site_created(user_id: i64, site_name: &str) {
    let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let channels = prefs_channels(user_id).await;
    if channels.contains("site") {
        push(
            user_id,
            "site_created",
            "站点创建成功",
            &format!("您创建的站点「{site_name}」已成功部署。"),
        )
        .await;
    }
    if channels.contains("email") {
        let mut p = HashMap::new();
        p.insert("site", site_name.to_string());
        p.insert("time", time);
        email_user(user_id, "site_created", &p).await;
    }
}

/// 磁盘空间不足预警（带 24h 冷却，避免周期采集重复打扰）。
pub async fn disk_low(user_id: i64, _owner_id: i64, pct: i32) {
    let cooldown_key = format!("disk_alert_at_{user_id}");
    if let Some(last) = server_env::conf_get(&cooldown_key) {
        if let Ok(ts) = last.parse::<i64>() {
            if chrono::Local::now().timestamp() - ts < 86400 {
                return; // 24h 内已提醒
            }
        }
    }
    let channels = prefs_channels(user_id).await;
    if channels.contains("site") {
        push(
            user_id,
            "disk_low",
            "磁盘空间不足预警",
            &format!("您的磁盘用量已达 {pct}%，即将超过套餐配额。"),
        )
        .await;
    }
    if channels.contains("email") {
        let mut p = HashMap::new();
        p.insert("pct", pct.to_string());
        email_user(user_id, "disk_low", &p).await;
    }
    let _ = server_env::conf_set_many(
        &[(cooldown_key, chrono::Local::now().timestamp().to_string())],
        "磁盘预警冷却",
    );
}
