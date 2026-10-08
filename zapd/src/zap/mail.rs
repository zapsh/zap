// SPDX-License-Identifier: AGPL-3.0-only
//! 可插拔邮件发送层（email-sdk）。
//!
//! 设计目标：把「通知发信」与具体服务商解耦。当前支持四类渠道：
//! - `smtp`     ：通用 SMTP（lettre，rustls 加密，避免 OpenSSL 依赖）
//! - `sendgrid` ：SendGrid Web API v3（API Key）
//! - `aliyun`   ：阿里云邮件推送 DirectMail（RPC + HMAC-SHA1 签名）
//! - `tencent`  ：腾讯云 SES（TC3-HMAC-SHA256 签名）
//!
//! 配置统一存于 `server_env` 的 conf 区（键名 `basic_mail_*`，密码/密钥经
//! [`crate::zap::crypto`] 加密）。[`send`] 在「未配置对应渠道」时直接 Ok(()) 静默跳过，
//! 调用方（notify）无需关心是否配置。

use std::collections::BTreeMap;

use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Digest};
use tracing::error;

use crate::zap::crypto;
use crate::zap::server_env;

/// 发信渠道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailProvider {
    Smtp,
    SendGrid,
    Aliyun,
    Tencent,
}

impl MailProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            MailProvider::Smtp => "smtp",
            MailProvider::SendGrid => "sendgrid",
            MailProvider::Aliyun => "aliyun",
            MailProvider::Tencent => "tencent",
        }
    }

    pub fn from_str(s: &str) -> MailProvider {
        match s {
            "sendgrid" => MailProvider::SendGrid,
            "aliyun" => MailProvider::Aliyun,
            "tencent" => MailProvider::Tencent,
            _ => MailProvider::Smtp,
        }
    }
}

/// SMTP 加密方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpEncryption {
    Ssl,
    Tls,
    None,
}

impl SmtpEncryption {
    pub fn from_str(s: &str) -> SmtpEncryption {
        match s {
            "ssl" => SmtpEncryption::Ssl,
            "none" => SmtpEncryption::None,
            _ => SmtpEncryption::Tls,
        }
    }
}

/// 完整发信配置（密钥为已解密明文，仅进程内存中使用）。
pub struct MailConfig {
    pub provider: MailProvider,
    pub from: String,
    // SMTP
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_encryption: SmtpEncryption,
    pub smtp_username: String,
    pub smtp_password: String,
    // SendGrid
    pub sg_api_key: String,
    // 阿里云 DirectMail
    pub aliyun_access_key: String,
    pub aliyun_access_secret: String,
    pub aliyun_region: String,
    // 腾讯云 SES
    pub tencent_secret_id: String,
    pub tencent_secret_key: String,
    pub tencent_region: String,
}

// ── 键名（与 system_basic.rs 保持一致）──────────────────────────
const K_PROVIDER: &str = "basic_mail_provider";
const K_FROM: &str = "basic_mail_from";
const K_SMTP_HOST: &str = "basic_mail_host";
const K_SMTP_PORT: &str = "basic_mail_port";
const K_SMTP_ENC: &str = "basic_mail_encryption";
const K_SMTP_USER: &str = "basic_mail_username";
const K_SMTP_PASS: &str = "basic_mail_password";
const K_SG_KEY: &str = "basic_mail_sg_apikey";
const K_ALI_KEY: &str = "basic_mail_aliyun_key";
const K_ALI_SECRET: &str = "basic_mail_aliyun_secret";
const K_ALI_REGION: &str = "basic_mail_aliyun_region";
const K_TC_ID: &str = "basic_mail_tencent_id";
const K_TC_KEY: &str = "basic_mail_tencent_key";
const K_TC_REGION: &str = "basic_mail_tencent_region";

/// 从 server_env 读取并还原发信配置。
///
/// 返回 `None` 表示「未配置对应渠道所需字段」，此时调用方应静默跳过发信。
pub fn load_config() -> Option<MailConfig> {
    let conf = server_env::conf_all();
    let get = |k: &str| conf.get(k).cloned().unwrap_or_default();
    let decrypt = |k: &str| -> String {
        let raw = get(k);
        if raw.is_empty() {
            return String::new();
        }
        crypto::decrypt_password(&raw)
    };

    let provider = MailProvider::from_str(&get(K_PROVIDER));
    let from = get(K_FROM).trim().to_string();
    if from.is_empty() {
        return None;
    }

    let cfg = MailConfig {
        provider,
        from,
        smtp_host: get(K_SMTP_HOST),
        smtp_port: get(K_SMTP_PORT).parse::<u16>().unwrap_or(587),
        smtp_encryption: SmtpEncryption::from_str(&get(K_SMTP_ENC)),
        smtp_username: get(K_SMTP_USER),
        smtp_password: decrypt(K_SMTP_PASS),
        sg_api_key: decrypt(K_SG_KEY),
        aliyun_access_key: get(K_ALI_KEY),
        aliyun_access_secret: decrypt(K_ALI_SECRET),
        aliyun_region: get(K_ALI_REGION),
        tencent_secret_id: get(K_TC_ID),
        tencent_secret_key: decrypt(K_TC_KEY),
        tencent_region: get(K_TC_REGION),
    };

    // 各渠道必备字段校验；不齐则视为未配置
    let ready = match cfg.provider {
        MailProvider::Smtp => {
            !cfg.smtp_host.is_empty() && !cfg.smtp_password.is_empty()
        }
        MailProvider::SendGrid => !cfg.sg_api_key.is_empty(),
        MailProvider::Aliyun => {
            !cfg.aliyun_access_key.is_empty()
                && !cfg.aliyun_access_secret.is_empty()
        }
        MailProvider::Tencent => {
            !cfg.tencent_secret_id.is_empty() && !cfg.tencent_secret_key.is_empty()
        }
    };
    if !ready {
        return None;
    }
    Some(cfg)
}

/// 发送一封邮件。
///
/// `body` 为**主内容**，`is_html` 标记其是否为 HTML；`alt_text` 为可选的纯文本兜底
/// （HTML 模式下用于构造 `multipart/alternative`，提升客户端兼容性与垃圾邮件评分）。
/// 仅纯文本模式下则只发 `text/plain`。
///
/// 未配置对应渠道时静默跳过（返回 Ok）；发送失败返回 Err(原因) 供调用方记录。
pub async fn send(
    to: &str,
    subject: &str,
    body: &str,
    is_html: bool,
    alt_text: Option<&str>,
) -> Result<(), String> {
    let cfg = match load_config() {
        Some(c) => c,
        None => return Ok(()), // 未配置，不阻塞业务
    };
    let to = to.trim();
    if to.is_empty() {
        return Ok(());
    }
    let (html, text) = if is_html {
        (Some(body), alt_text)
    } else {
        (None, Some(body))
    };
    match cfg.provider {
        MailProvider::Smtp => smtp_send(&cfg, to, subject, html, text).await,
        MailProvider::SendGrid => sendgrid_send(&cfg, to, subject, html, text).await,
        MailProvider::Aliyun => aliyun_send(&cfg, to, subject, html, text).await,
        MailProvider::Tencent => tencent_send(&cfg, to, subject, html, text).await,
    }
}

// ── SMTP（lettre）──────────────────────────────────────────────
async fn smtp_send(
    cfg: &MailConfig,
    to: &str,
    subject: &str,
    html: Option<&str>,
    text: Option<&str>,
) -> Result<(), String> {
    use lettre::{
        AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
        message::{Mailbox, MultiPart, SinglePart, header::ContentType},
        transport::smtp::authentication::Credentials,
    };

    let from: Mailbox = cfg
        .from
        .parse()
        .map_err(|e| format!("发件人地址非法: {e}"))?;
    let to_mbox: Mailbox = to.parse().map_err(|e| format!("收件人地址非法: {e}"))?;

    let msg = match (html.filter(|h| !h.is_empty()), text.filter(|t| !t.is_empty())) {
        (Some(h), Some(t)) => Message::builder()
            .from(from)
            .to(to_mbox)
            .subject(subject)
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(t.to_string()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(h.to_string()),
                    ),
            )
            .map_err(|e| format!("构建邮件失败: {e}"))?,
        (Some(h), None) => Message::builder()
            .from(from)
            .to(to_mbox)
            .subject(subject)
            .header(ContentType::TEXT_HTML)
            .body(h.to_string())
            .map_err(|e| format!("构建邮件失败: {e}"))?,
        (None, Some(t)) => Message::builder()
            .from(from)
            .to(to_mbox)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(t.to_string())
            .map_err(|e| format!("构建邮件失败: {e}"))?,
        (None, None) => return Ok(()),
    };

    let mut builder = match cfg.smtp_encryption {
        SmtpEncryption::Ssl => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.smtp_host)
            .map_err(|e| format!("SMTP 连接配置错误: {e}"))?,
        SmtpEncryption::Tls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.smtp_host)
            .map_err(|e| format!("SMTP 连接配置错误: {e}"))?,
        SmtpEncryption::None => {
            // 无加密：直连指定主机（仅在内网 / 可信网络使用）
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.smtp_host)
        }
    };
    builder = builder.port(cfg.smtp_port);
    if !cfg.smtp_username.is_empty() {
        builder = builder.credentials(Credentials::new(
            cfg.smtp_username.clone(),
            cfg.smtp_password.clone(),
        ));
    }
    let transport = builder.build();

    transport
        .send(msg)
        .await
        .map_err(|e| format!("SMTP 发送失败: {e}"))?;
    Ok(())
}

// ── SendGrid（Web API v3）──────────────────────────────────────
async fn sendgrid_send(
    cfg: &MailConfig,
    to: &str,
    subject: &str,
    html: Option<&str>,
    text: Option<&str>,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let mut content = Vec::new();
    if let Some(t) = text.filter(|t| !t.is_empty()) {
        content.push(serde_json::json!({ "type": "text/plain", "value": t }));
    }
    if let Some(h) = html.filter(|h| !h.is_empty()) {
        content.push(serde_json::json!({ "type": "text/html", "value": h }));
    }
    if content.is_empty() {
        return Ok(());
    }
    let body = serde_json::json!({
        "personalizations": [{ "to": [{ "email": to }] }],
        "from": { "email": cfg.from },
        "subject": subject,
        "content": content,
    });
    let resp = client
        .post("https://api.sendgrid.com/v3/mail/send")
        .bearer_auth(&cfg.sg_api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("SendGrid 请求失败: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(format!("SendGrid 返回 {status}: {txt}"));
    }
    Ok(())
}

// ── 阿里云 DirectMail（SingleSendMail，RPC + HMAC-SHA1）──────────
async fn aliyun_send(
    cfg: &MailConfig,
    to: &str,
    subject: &str,
    html: Option<&str>,
    text: Option<&str>,
) -> Result<(), String> {
    let ak = &cfg.aliyun_access_key;
    let sk = &cfg.aliyun_access_secret;
    let region = if cfg.aliyun_region.is_empty() {
        "cn-hangzhou".to_string()
    } else {
        cfg.aliyun_region.clone()
    };

    let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let nonce = format!("{:x}", rand::random::<u64>());

    // 公共参数 + 业务参数
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    params.insert("AccessKeyId".into(), ak.clone());
    params.insert("Action".into(), "SingleSendMail".into());
    params.insert("AccountName".into(), cfg.from.clone());
    params.insert("ToAddress".into(), to.to_string());
    params.insert("Subject".into(), subject.to_string());
    if let Some(h) = html.filter(|h| !h.is_empty()) {
        params.insert("HtmlBody".into(), h.to_string());
    }
    if let Some(t) = text.filter(|t| !t.is_empty()) {
        params.insert("TextBody".into(), t.to_string());
    }
    params.insert("ReplyToAddress".into(), "false".into());
    params.insert("AddressType".into(), "0".into());
    params.insert("RegionId".into(), region);
    params.insert("Format".into(), "JSON".into());
    params.insert("Version".into(), "2015-11-23".into());
    params.insert("SignatureMethod".into(), "HMAC-SHA1".into());
    params.insert("SignatureNonce".into(), nonce);
    params.insert("SignatureVersion".into(), "1.0".into());
    params.insert("Timestamp".into(), timestamp);

    // 规范查询串
    let canonical = params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let string_to_sign = format!(
        "GET&{}&{}",
        percent_encode("/"),
        percent_encode(&canonical)
    );

    // HMAC-SHA1，密钥 = AccessKeySecret + "&"
    type HmacSha1 = Hmac<Sha1>;
    let mut mac = HmacSha1::new_from_slice(format!("{sk}&").as_bytes())
        .map_err(|e| format!("签名初始化失败: {e}"))?;
    mac.update(string_to_sign.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

    let mut query = canonical;
    query.push_str(&format!("&Signature={}", percent_encode(&sig)));

    let endpoint = "https://dm.aliyuncs.com/";
    // 用原始 query 串直接请求（避免 reqwest 二次编码）
    let resp = reqwest::Client::new()
        .get(format!("{endpoint}?{query}"))
        .send()
        .await
        .map_err(|e| format!("阿里云请求失败: {e}"))?;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() && !txt.contains("\"Code\"") {
        return Err(format!("阿里云返回 {status}: {txt}"));
    }
    // 阿里云成功时返回 {"RequestId":...}；业务错误返回 {"Code":"...","Message":"..."}
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
        if let Some(code) = v.get("Code").and_then(|c| c.as_str()) {
            if code != "OK" && !code.is_empty() {
                let msg = v.get("Message").and_then(|m| m.as_str()).unwrap_or("");
                return Err(format!("阿里云发送失败 [{code}]: {msg}"));
            }
        }
    }
    Ok(())
}

// ── 腾讯云 SES（SendEmail，TC3-HMAC-SHA256）─────────────────────
async fn tencent_send(
    cfg: &MailConfig,
    to: &str,
    subject: &str,
    html: Option<&str>,
    text: Option<&str>,
) -> Result<(), String> {
    let secret_id = &cfg.tencent_secret_id;
    let secret_key = &cfg.tencent_secret_key;
    let region = if cfg.tencent_region.is_empty() {
        "ap-guangzhou".to_string()
    } else {
        cfg.tencent_region.clone()
    };
    let service = "ses";
    let host = "ses.tencentcloudapi.com";
    let action = "SendEmail";
    let version = "2021-01-11";

    let mut body_map = serde_json::Map::new();
    if let Some(h) = html.filter(|h| !h.is_empty()) {
        body_map.insert("Html".into(), serde_json::json!({ "Data": h }));
    }
    if let Some(t) = text.filter(|t| !t.is_empty()) {
        body_map.insert("Text".into(), serde_json::json!({ "Data": t }));
    }

    let payload = serde_json::json!({
        "FromEmailAddress": cfg.from,
        "Destination": { "To": [to] },
        "Subject": { "Data": subject },
        "Body": serde_json::Value::Object(body_map),
    });
    let payload_str = payload.to_string();

    let now = Utc::now();
    let timestamp = now.timestamp();
    let date = now.format("%Y-%m-%d").to_string();

    let hashed_payload = hex_sha256(payload_str.as_bytes());
    let canonical_headers = format!("content-type:application/json; charset=utf-8\nhost:{host}\n");
    let signed_headers = "content-type;host";
    let canonical_request = format!(
        "POST\n/\n\n{canonical_headers}\n{signed_headers}\n{hashed_payload}"
    );
    let credential_scope = format!("{date}/{service}/tc3_request");
    let canonical_hash = hex_sha256(canonical_request.as_bytes());
    let string_to_sign = format!(
        "TC3-HMAC-SHA256\n{timestamp}\n{credential_scope}\n{canonical_hash}"
    );

    // 派生签名密钥
    let secret_date = hmac_sha256(format!("TC3{secret_key}").as_bytes(), date.as_bytes());
    let secret_service = hmac_sha256(&secret_date, service.as_bytes());
    let secret_signing = hmac_sha256(&secret_service, b"tc3_request");
    let signature = hex_encode(&hmac_sha256(&secret_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "TC3-HMAC-SHA256 Credential={secret_id}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );

    let resp = reqwest::Client::new()
        .post(format!("https://{host}"))
        .header("Authorization", authorization)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Host", host)
        .header("X-TC-Action", action)
        .header("X-TC-Region", region)
        .header("X-TC-Timestamp", timestamp.to_string())
        .header("X-TC-Version", version)
        .body(payload_str)
        .send()
        .await
        .map_err(|e| format!("腾讯云请求失败: {e}"))?;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("腾讯云返回 {status}: {txt}"));
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
        if let Some(err) = v.get("Response").and_then(|r| r.get("Error")) {
            let code = err.get("Code").and_then(|c| c.as_str()).unwrap_or("");
            let msg = err.get("Message").and_then(|m| m.as_str()).unwrap_or("");
            return Err(format!("腾讯云发送失败 [{code}]: {msg}"));
        }
    }
    Ok(())
}

// ── 工具 ────────────────────────────────────────────────────────
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn hex_sha256(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac key size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn hex_encode(data: &[u8]) -> String {
    hex::encode(data)
}

/// 供 notify 等调用方在发信失败时统一记录（避免散落 error!）。
pub fn log_send_error(context: &str, err: &str) {
    error!("邮件发送失败（{context}）: {err}");
}
