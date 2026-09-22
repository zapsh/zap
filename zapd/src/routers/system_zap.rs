//! 「系统设置 → Zap 设置」后端：面板自身运行参数（zap.yaml 的 `server.*`）。
//!
//! 四个 Tab 一一对应配置字段：
//! - 服务设置：`server.address`（绑定 IP） / `server.port`（监听端口）
//! - SSL 证书：`server.cert_file` / `server.key_file`（自签 / 证书库 / 手动粘贴）
//! - 访问前缀：`server.url_prefix`
//! - 配置文件：只读展示 zap.yaml 实际路径与内容（排查"改了没生效"）
//!
//! 保存流程：校验 → 写入证书/私钥文件 → 更新内存配置 → 落盘 zap.yaml。
//! 端口、绑定 IP、证书、URL 前缀都在进程启动时生效，保存后需重启 zapd。
//!
//! 端点（均仅 admin）：
//! - GET  /system/config/zap               读取配置 / 当前证书信息 / 可选证书
//! - POST /system/config/zap               保存（按 Tab 部分提交）
//! - POST /system/config/zap/ssl/self-sign 重新生成自签证书

use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use axum::{Json, extract::Extension};
use serde::Deserialize;
use serde_json::json;
use tracing::info;

use crate::db;
use crate::routers::ssl;
use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;

// ── 证书来源持久化（{data}/server_env.yaml 的 conf 区，避免塞进 zap.yaml）──

const K_SSL_SOURCE: &str = "zap_ssl_source";
const K_SSL_CERT_ID: &str = "zap_ssl_cert_id";

/// 证书来源：self-signed（自动生成）/ library（证书库）/ manual（手动粘贴）
const SRC_SELF: &str = "self-signed";
const SRC_LIBRARY: &str = "library";
const SRC_MANUAL: &str = "manual";

fn conf_get(key: &str) -> String {
    crate::zap::server_env::conf_get(key).unwrap_or_default()
}

fn conf_set(key: &str, value: &str) {
    crate::zap::server_env::conf_set(key, value, "Zap 设置");
}

/// 当前生效的服务配置快照。
fn server_snapshot() -> Result<(String, u16, String, String, String), String> {
    let guard = crate::config::get_config()
        .read()
        .map_err(|_| "配置读取失败".to_string())?;
    Ok((
        guard.server.address.clone(),
        guard.server.port,
        guard.server.url_prefix.clone(),
        guard.server.cert_file.clone(),
        guard.server.key_file.clone(),
    ))
}

// ── GET /system/config/zap ──────────────────────────────────

#[derive(sqlx::FromRow, Debug, serde::Serialize)]
struct CertOption {
    id: i64,
    name: String,
    domains: String,
    cert_type: String,
    not_after: i64,
}

/// 证书库可选证书（供「证书库」来源下拉选择）。
async fn cert_options() -> Vec<CertOption> {
    let pool = db::get_db_pool().await;
    sqlx::query_as("SELECT id, name, domains, cert_type, not_after FROM ssl_cert ORDER BY id DESC")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// 读取证书 / 私钥文件，解析出页面上展示的证书信息。
fn inspect_ssl(cert_file: &str, key_file: &str) -> serde_json::Value {
    let cert_pem = fs::read_to_string(cert_file).unwrap_or_default();
    let key_pem = fs::read_to_string(key_file).unwrap_or_default();
    let exists = !cert_pem.trim().is_empty() && !key_pem.trim().is_empty();

    let mut info = json!({
        "exists": exists,
        "cert_file": cert_file,
        "key_file": key_file,
        "cert_exists": !cert_pem.trim().is_empty(),
        "key_exists": !key_pem.trim().is_empty(),
        "common_name": "",
        "domains": "",
        "issuer": "",
        "not_before": 0,
        "not_after": 0,
        "days_left": 0,
        "self_signed": false,
        "key_match": serde_json::Value::Null,
        "error": "",
    });

    let parsed = if cert_pem.trim().is_empty() {
        None
    } else {
        ssl::parse_certificate(&cert_pem)
    };
    let Some(p) = parsed else {
        if !cert_pem.trim().is_empty() {
            info["error"] = json!("证书文件无法解析，请确认是 PEM 格式证书");
        }
        return info;
    };

    let now = chrono::Local::now().timestamp();
    info["common_name"] = json!(p.common_name);
    info["domains"] = json!(p.domains_str);
    info["issuer"] = json!(p.issuer);
    info["not_before"] = json!(p.not_before);
    info["not_after"] = json!(p.not_after);
    info["days_left"] = json!((p.not_after - now) / 86400);
    info["self_signed"] = json!(!p.subject.is_empty() && p.subject == p.issuer);

    if !key_pem.trim().is_empty() {
        match ssl::key_matches(&cert_pem, &key_pem) {
            Ok(m) => info["key_match"] = json!(m),
            Err(e) => {
                info["key_match"] = json!(false);
                info["error"] = json!(e);
            }
        }
    }
    info
}

pub async fn zap_get(claims: ValidatedClaims) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可查看 Zap 设置".to_string()));
    }
    let (address, port, url_prefix, cert_file, key_file) =
        server_snapshot().map_err(|e| ZapError::New(-1, e))?;
    let path = crate::config::config_path();
    let content = fs::read_to_string(&path).unwrap_or_default();

    let source = conf_get(K_SSL_SOURCE);
    let source = if source.is_empty() {
        SRC_SELF.to_string()
    } else {
        source
    };
    let cert_id: i64 = conf_get(K_SSL_CERT_ID).parse().unwrap_or(0);

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "config_path": path.display().to_string(),
            "config_exists": path.exists(),
            "config_content": content,
            "server": {
                "address": address,
                "port": port,
                "url_prefix": url_prefix,
                "url_prefix_path": crate::config::url_prefix_path(),
            },
            "ssl": {
                "source": source,
                "cert_id": cert_id,
                // 当前面板实际加载的证书信息（来自 cert_file/key_file）
                "current": inspect_ssl(&cert_file, &key_file),
            },
            "certs": cert_options().await,
        }
    })))
}

// ── POST /system/config/zap ─────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct ZapServerPane {
    pub address: Option<String>,
    pub port: Option<u16>,
    pub url_prefix: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ZapSslPane {
    /// self-signed / library / manual
    pub source: Option<String>,
    /// 来源为 library 时选择的证书库证书 ID
    pub cert_id: Option<i64>,
    pub cert_file: Option<String>,
    pub key_file: Option<String>,
    /// 来源为 manual 时粘贴的 PEM（证书可含中间链）
    pub cert_content: Option<String>,
    pub key_content: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ZapSavePayload {
    pub server: Option<ZapServerPane>,
    pub ssl: Option<ZapSslPane>,
}

/// 规范化 URL 前缀并校验字符集：只允许字母数字与 `._-~/`。
fn validate_prefix(raw: &str) -> Result<String, String> {
    let p = crate::config::normalize_url_prefix(raw);
    if p.is_empty() {
        return Ok(String::new());
    }
    if p.len() > 64 {
        return Err("URL 前缀过长（最多 64 个字符）".to_string());
    }
    for seg in p.split('/') {
        if seg.is_empty()
            || !seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
        {
            return Err("URL 前缀只能包含字母、数字以及 . _ - ~ /".to_string());
        }
    }
    Ok(p)
}

/// 校验并应用「服务设置」Tab。
fn apply_server(pane: &ZapServerPane) -> Result<String, String> {
    let mut address: Option<String> = None;
    if let Some(a) = &pane.address {
        let a = a.trim().to_string();
        if a.is_empty() {
            return Err("绑定 IP 不能为空".to_string());
        }
        if a.parse::<IpAddr>().is_err() {
            return Err("绑定 IP 格式不正确（示例：0.0.0.0、192.168.1.10、::）".to_string());
        }
        address = Some(a);
    }
    let mut port: Option<u16> = None;
    if let Some(p) = pane.port {
        if p == 0 {
            return Err("监听端口需在 1 - 65535 之间".to_string());
        }
        port = Some(p);
    }
    let mut prefix: Option<String> = None;
    if let Some(u) = &pane.url_prefix {
        prefix = Some(validate_prefix(u)?);
    }

    if address.is_none() && port.is_none() && prefix.is_none() {
        return Ok("服务设置未变更".to_string());
    }

    let mut detail = Vec::new();
    if let Some(a) = &address {
        detail.push(format!("address={a}"));
    }
    if let Some(p) = port {
        detail.push(format!("port={p}"));
    }
    if let Some(u) = &prefix {
        detail.push(format!(
            "url_prefix={}",
            if u.is_empty() { "(空)" } else { u }
        ));
    }

    crate::config::mutate_config(|c| {
        if let Some(a) = &address {
            c.server.address = a.clone();
        }
        if let Some(p) = port {
            c.server.port = p;
        }
        if let Some(u) = &prefix {
            c.server.url_prefix = u.clone();
        }
    })?;

    Ok(detail.join(","))
}

/// 路径校验：非空、长度受限，禁止空字节。
fn normalize_path(raw: &str, label: &str) -> Result<String, String> {
    let p = raw.trim().to_string();
    if p.is_empty() {
        return Err(format!("{label}不能为空"));
    }
    if p.len() > 512 {
        return Err(format!("{label}路径过长（最多 512 个字符）"));
    }
    if p.contains('\0') {
        return Err(format!("{label}路径不合法"));
    }
    Ok(p)
}

/// 写 PEM 文件（自动建目录；私钥落 600，证书落 644）。
fn write_pem(path: &str, content: &str, secret: bool) -> Result<(), String> {
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = fs::create_dir_all(parent)
    {
        return Err(format!("创建目录失败 {}: {e}", parent.display()));
    }
    let mut text = content.trim_end().to_string();
    text.push('\n');
    fs::write(path, text).map_err(|e| format!("写入 {path} 失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if secret { 0o600 } else { 0o644 };
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    Ok(())
}

/// 解析「SSL 证书」Tab，返回 `(证书文件路径, 私钥文件路径, 证书PEM, 私钥PEM, 来源, 证书ID)`。
async fn resolve_ssl(
    pane: &ZapSslPane,
) -> Result<(String, String, String, String, String, i64), String> {
    let (cur_cert, _, _, _, cur_key) = server_snapshot()?;
    let cert_file = match &pane.cert_file {
        Some(v) => normalize_path(v, "证书文件路径")?,
        None => cur_cert,
    };
    let key_file = match &pane.key_file {
        Some(v) => normalize_path(v, "私钥文件路径")?,
        None => cur_key,
    };
    if cert_file == key_file {
        return Err("证书文件与私钥文件不能是同一个文件".to_string());
    }

    let source = pane
        .source
        .clone()
        .unwrap_or_else(|| SRC_SELF.to_string())
        .trim()
        .to_lowercase();

    match source.as_str() {
        SRC_SELF => {
            if !crate::zap::certmgr::ensure_certs(&cert_file, &key_file) {
                return Err("自签证书生成失败，请检查目标目录是否可写".to_string());
            }
            let cert = fs::read_to_string(&cert_file).unwrap_or_default();
            let key = fs::read_to_string(&key_file).unwrap_or_default();
            if cert.trim().is_empty() || key.trim().is_empty() {
                return Err("自签证书生成后仍读取不到内容，请检查文件权限".to_string());
            }
            Ok((cert_file, key_file, cert, key, SRC_SELF.to_string(), 0))
        }
        SRC_LIBRARY => {
            let id = pane.cert_id.unwrap_or(0);
            if id <= 0 {
                return Err("请选择要使用的证书".to_string());
            }
            let pool = db::get_db_pool().await;
            let row: Option<(String, String, String)> = sqlx::query_as(
                "SELECT cert_content, key_content, ca_bundle FROM ssl_cert WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("读取证书失败: {e}"))?;
            let (cert, key, ca) = row.ok_or_else(|| "所选证书不存在".to_string())?;
            if cert.trim().is_empty() {
                return Err("所选证书缺少证书内容".to_string());
            }
            if key.trim().is_empty() {
                return Err("所选证书缺少私钥内容".to_string());
            }
            match ssl::key_matches(&cert, &key) {
                Ok(true) => {}
                Ok(false) => return Err("所选证书与它的私钥不匹配，无法用于面板".to_string()),
                Err(e) => return Err(format!("证书校验失败：{e}")),
            }
            // 中间链拼在叶子证书之后，构成 Nginx/OpenSSL 习惯的 fullchain
            let full = if ca.trim().is_empty() {
                cert
            } else {
                format!("{}\n{}", cert.trim_end(), ca.trim_end())
            };
            Ok((cert_file, key_file, full, key, SRC_LIBRARY.to_string(), id))
        }
        SRC_MANUAL => {
            let cert = pane.cert_content.clone().unwrap_or_default();
            let key = pane.key_content.clone().unwrap_or_default();
            if cert.trim().is_empty() {
                return Err("请粘贴证书内容（PEM 格式）".to_string());
            }
            if key.trim().is_empty() {
                return Err("请粘贴私钥内容（PEM 格式）".to_string());
            }
            if ssl::parse_certificate(&cert).is_none() {
                return Err("证书无法解析，请粘贴 PEM 格式的证书".to_string());
            }
            match ssl::key_matches(&cert, &key) {
                Ok(true) => {}
                Ok(false) => return Err("证书与私钥不匹配，请检查后重试".to_string()),
                Err(e) => return Err(e),
            }
            Ok((cert_file, key_file, cert, key, SRC_MANUAL.to_string(), 0))
        }
        other => Err(format!("不支持的证书来源：{other}")),
    }
}

pub async fn zap_save(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ZapSavePayload>,
) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可修改 Zap 设置".to_string()));
    }
    if payload.server.is_none() && payload.ssl.is_none() {
        return Err(ZapError::New(-1, "没有需要保存的内容".to_string()));
    }

    // 前置：改前缀会让已纳管节点失联（Zap Pro §6.4），先记下旧值
    #[cfg(feature = "commercial")]
    let old_prefix = crate::config::url_prefix();

    let mut details: Vec<String> = Vec::new();

    if let Some(s) = &payload.server {
        details.push(apply_server(s).map_err(|e| ZapError::New(-1, e))?);
    }
    if let Some(s) = &payload.ssl {
        let (cert_file, key_file, cert, key, source, cert_id) =
            resolve_ssl(s).await.map_err(|e| ZapError::New(-1, e))?;
        write_pem(&cert_file, &cert, false).map_err(|e| ZapError::New(-1, e))?;
        write_pem(&key_file, &key, true).map_err(|e| ZapError::New(-1, e))?;
        crate::config::mutate_config(|c| {
            c.server.cert_file = cert_file.clone();
            c.server.key_file = key_file.clone();
        })
        .map_err(|e| ZapError::New(-1, e))?;
        conf_set(K_SSL_SOURCE, &source);
        conf_set(K_SSL_CERT_ID, &cert_id.to_string());
        details.push(format!(
            "ssl={source},cert_file={cert_file},key_file={key_file}"
        ));
        info!("panel ssl updated: source={source} cert={cert_file} key={key_file}");
    }

    let detail = details.join(" | ");
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "zap_config_save",
        "system",
        &detail,
    )
    .await;

    // 商业版会往 message 后面追加「N 台节点会失联」的提示（见下方 cfg 块）
    #[allow(unused_mut)]
    let mut message = "已保存到 zap.yaml，重启 Zap 服务后生效".to_string();
    // Zap Pro：前缀真变了且有纳管节点 → 把「N 台会失联 + 该执行什么」带回给管理员
    #[cfg(feature = "commercial")]
    if let Some(s) = &payload.server
        && s.url_prefix.is_some()
    {
        let new_prefix = crate::config::url_prefix();
        if new_prefix != old_prefix {
            message
                .push_str(&crate::pro::cluster::on_prefix_changed(&old_prefix, &new_prefix).await);
        }
    }

    Ok(Json(json!({ "code": 0, "message": message })))
}

// ── POST /system/config/zap/ssl/self-sign ───────────────────

/// 重新生成自签证书：删除现有文件后由 certmgr 重新签发（10 年，CN=zap-local）。
pub async fn ssl_self_sign(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可操作面板证书".to_string()));
    }
    let (_, _, _, cert_file, key_file) = server_snapshot().map_err(|e| ZapError::New(-1, e))?;
    let _ = fs::remove_file(&cert_file);
    let _ = fs::remove_file(&key_file);
    if !crate::zap::certmgr::ensure_certs(&cert_file, &key_file) {
        return Err(ZapError::New(
            -1,
            "自签证书生成失败，请检查目标目录是否可写".to_string(),
        ));
    }
    conf_set(K_SSL_SOURCE, SRC_SELF);
    conf_set(K_SSL_CERT_ID, "0");

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "zap_ssl_self_sign",
        "system",
        &format!("cert_file={cert_file}"),
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": "已重新生成自签证书，重启 Zap 服务后生效"
    })))
}
