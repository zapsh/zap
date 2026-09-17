//! 「SSL/TLS」菜单后端：SSL 证书管理。
//!
//! 支持三种来源并统一按 PEM 保存到 `ssl_cert` 表（四段材料）：
//!   - cert_content：证书（crt；Let's Encrypt 时为叶子证书）
//!   - key_content ：私钥（key）
//!   - ca_bundle   ：中间链（ca-bundle；自签名 / 单段证书为空）
//!   - csr         ：证书签名请求（手动导入或自签名生成时产生）
//!
//! 功能：手动添加 / 修改 / 删除 / 查看；rcgen 生成自签名证书；
//! instant-acme 以「异步订单」方式向 Let's Encrypt 申请
//! （HTTP-01 走面板自管的 well-known 验证根，DNS-01 支持手动解析与服务商 API 自动处理）。

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{Extension, Query};
use serde::Deserialize;
use serde_json::json;
use tracing::warn;

use crate::{
    db,
    zap::jwt::ValidatedClaims,
    zap::{ZapError, ZapJsonResult, acme, audit, crypto, jwt},
};

// ── 行结构 ───────────────────────────────────────────────────

#[derive(sqlx::FromRow, Debug, serde::Serialize)]
struct CertListRow {
    id: i64,
    /// 证书归属用户 id（0 = 历史系统证书，仅管理员可见 / 可转归属）
    user_id: i64,
    /// 归属用户登录名（系统证书或用户已删除时为空）
    owner_name: String,
    name: String,
    domains: String,
    cert_type: String,
    not_before: i64,
    not_after: i64,
    status: i64,
    remark: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow, Debug, serde::Serialize)]
struct CertDetailRow {
    id: i64,
    user_id: i64,
    name: String,
    domains: String,
    cert_type: String,
    cert_content: String,
    key_content: String,
    ca_bundle: String,
    csr: String,
    not_before: i64,
    not_after: i64,
    status: i64,
    remark: String,
    created_at: i64,
    updated_at: i64,
}

// ── 归属 / 可见性 ────────────────────────────────────────────

/// 证书按归属用户隔离，可见性与站点一致：
/// admin → 全部（含 user_id=0 的系统证书）；reseller → 自己 + 名下客户；
/// 普通用户 → 仅自己的证书。返回该证书的归属 user_id。
async fn cert_in_scope(claims: &jwt::Claims, cert_id: i64) -> Result<i64, ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM ssl_cert WHERE id = ?")
        .bind(cert_id)
        .fetch_optional(pool)
        .await?;
    let Some((cuid,)) = row else {
        return Err(ZapError::New(-1, "证书不存在".to_string()));
    };
    if jwt::is_admin(claims) || cuid == claims.id as i64 {
        return Ok(cuid);
    }
    if jwt::is_reseller(claims) {
        let (cnt,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM user WHERE id = ? AND owner_id = ?")
                .bind(cuid)
                .bind(claims.id as i64)
                .fetch_one(pool)
                .await?;
        if cnt > 0 {
            return Ok(cuid);
        }
    }
    Err(ZapError::New(
        -1,
        "无权访问该证书：证书归属其他用户（SSL 证书按归属用户隔离）".to_string(),
    ))
}

/// 归属目标校验：admin → 任意；reseller → 自己或名下客户；普通用户 → 仅自己。
/// 与站点归属共用同一套规则。
async fn resolve_cert_owner(claims: &jwt::Claims, target: i64) -> Result<(), ZapError> {
    if target <= 0 {
        return Err(ZapError::New(-1, "证书归属用户不合法".to_string()));
    }
    crate::routers::site::resolve_target_user(claims, target).await
}

// ── 列表 / 详情 ──────────────────────────────────────────────

pub async fn cert_list(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let cols = "c.id, c.user_id, u.username AS owner_name, c.name, c.domains, c.cert_type, \
                c.not_before, c.not_after, c.status, c.remark, c.created_at, c.updated_at";
    let rows: Vec<CertListRow> = if jwt::is_admin(&claims) {
        sqlx::query_as(&format!(
            "SELECT {cols} FROM ssl_cert c LEFT JOIN user u ON u.id = c.user_id \
             ORDER BY c.id DESC"
        ))
        .fetch_all(pool)
        .await?
    } else if jwt::is_reseller(&claims) {
        sqlx::query_as(&format!(
            "SELECT {cols} FROM ssl_cert c LEFT JOIN user u ON u.id = c.user_id \
             WHERE c.user_id = ? OR c.user_id IN (SELECT id FROM user WHERE owner_id = ?) \
             ORDER BY c.id DESC"
        ))
        .bind(claims.id as i64)
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(&format!(
            "SELECT {cols} FROM ssl_cert c LEFT JOIN user u ON u.id = c.user_id \
             WHERE c.user_id = ? ORDER BY c.id DESC"
        ))
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await?
    };
    Ok(Json(json!({ "code": 0, "message": "OK", "data": rows })))
}

#[derive(Debug, Deserialize)]
pub struct CertDetailQuery {
    pub id: i64,
}

pub async fn cert_detail(
    Query(q): Query<CertDetailQuery>,
    claims: ValidatedClaims,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    cert_in_scope(&claims, q.id).await?;
    let row: Option<CertDetailRow> = sqlx::query_as("SELECT * FROM ssl_cert WHERE id = ?")
        .bind(q.id)
        .fetch_optional(pool)
        .await?;
    match row {
        Some(r) => Ok(Json(json!({ "code": 0, "message": "OK", "data": r }))),
        None => Err(ZapError::New(-1, "证书不存在".to_string())),
    }
}

// ── 手动添加 / 修改 / 删除 ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CertAddPayload {
    pub name: String,
    /// 证书归属用户 id：admin / reseller 可选（默认自己；reseller 仅能选自己或名下客户），
    /// 普通用户强制归属自己
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub domains: String,
    #[serde(default)]
    pub cert_content: String,
    #[serde(default)]
    pub key_content: String,
    #[serde(default)]
    pub ca_bundle: String,
    #[serde(default)]
    pub csr: String,
    #[serde(default)]
    pub remark: String,
}

pub async fn cert_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CertAddPayload>,
) -> ZapJsonResult {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ZapError::New(-1, "请填写证书名称".to_string()));
    }
    let has_material = !payload.cert_content.trim().is_empty()
        || !payload.key_content.trim().is_empty()
        || !payload.csr.trim().is_empty();
    if !has_material {
        return Err(ZapError::New(
            -1,
            "证书内容为空：至少提供 cert / key / csr 中的一种".to_string(),
        ));
    }
    // 证书（或 CSR）与私钥必须配对，否则保存后无法部署
    if let Err(e) = check_cert_key_pair(&payload.cert_content, &payload.csr, &payload.key_content) {
        return Err(ZapError::New(-1, e));
    }
    // 域名 / 有效期：未提供域名时自动从证书（或 CSR）解析
    let parsed = parse_pem_info(&payload.cert_content).or_else(|| parse_pem_info(&payload.csr));
    let domains = if payload.domains.trim().is_empty() {
        parsed
            .as_ref()
            .map(|p| p.domains_str.clone())
            .unwrap_or_default()
    } else {
        payload.domains.trim().to_string()
    };
    let (not_before, not_after) = parsed
        .as_ref()
        .map(|p| (p.not_before, p.not_after))
        .unwrap_or((0, 0));

    // 归属用户：admin / reseller 可指定（resolve_cert_owner 校验范围）；普通用户强制归属自己
    let owner_user = match payload.user_id {
        Some(t) => {
            resolve_cert_owner(&claims, t).await?;
            t
        }
        None => claims.id as i64,
    };

    let pool = db::get_db_pool().await;
    let now = chrono::Utc::now().timestamp();
    let r = sqlx::query(
        "INSERT INTO ssl_cert
            (user_id, name, domains, cert_type, cert_content, key_content, ca_bundle, csr,
             not_before, not_after, status, remark, created_at, updated_at)
         VALUES (?, ?, ?, 'upload', ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)",
    )
    .bind(owner_user)
    .bind(&name)
    .bind(&domains)
    .bind(payload.cert_content.trim())
    .bind(payload.key_content.trim())
    .bind(payload.ca_bundle.trim())
    .bind(payload.csr.trim())
    .bind(not_before)
    .bind(not_after)
    .bind(payload.remark.trim())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    let id = r.last_insert_rowid();
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssl_cert_add",
        &name,
        &format!("id={id}"),
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": { "id": id } }),
    ))
}

/// 修改证书。四段 PEM 为 `Option`：未传（如只切换启用状态）时保持原值不变，
/// 显式传空串才会清空对应内容。
#[derive(Debug, Deserialize)]
pub struct CertUpdatePayload {
    pub id: i64,
    /// 归属转移（可选）：仅 admin / reseller 可用，目标须在其管理范围内；
    /// 不传或与现归属相同则保持
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub domains: String,
    #[serde(default)]
    pub cert_content: Option<String>,
    #[serde(default)]
    pub key_content: Option<String>,
    #[serde(default)]
    pub ca_bundle: Option<String>,
    #[serde(default)]
    pub csr: Option<String>,
    #[serde(default)]
    pub remark: String,
    #[serde(default)]
    pub status: Option<i32>,
}

pub async fn cert_update(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CertUpdatePayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    // 归属可见性校验：admin 全部 / reseller 名下客户 / 普通用户仅自己的证书
    let cur_owner = cert_in_scope(&claims, payload.id).await?;
    // 归属转移（可选）：仅当显式传入且与现归属不同时生效，目标范围同站点归属规则
    let new_owner = match payload.user_id {
        Some(t) if t != cur_owner => {
            resolve_cert_owner(&claims, t).await?;
            Some(t)
        }
        _ => None,
    };
    let now = chrono::Utc::now().timestamp();
    let status = payload.status.unwrap_or(1);

    // 证书（或 CSR）与私钥必须配对；只改私钥时用库里现有的证书 / CSR 比对
    if let Some(key) = payload.key_content.as_deref()
        && !key.trim().is_empty()
    {
        let mut material = payload
            .cert_content
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| payload.csr.clone().filter(|s| !s.trim().is_empty()))
            .unwrap_or_default();
        if material.trim().is_empty() {
            let row: Option<(String, String)> =
                sqlx::query_as("SELECT cert_content, csr FROM ssl_cert WHERE id = ?")
                    .bind(payload.id)
                    .fetch_optional(pool)
                    .await?;
            material = row
                .map(|(c, s)| if !c.trim().is_empty() { c } else { s })
                .unwrap_or_default();
        }
        if let Err(e) = check_cert_key_pair(&material, "", key) {
            return Err(ZapError::New(-1, e));
        }
    }

    // 未提供域名时，若本次上传了证书（或 CSR）则自动解析填充
    let mut domains = payload.domains.trim().to_string();
    let mut validity: Option<(i64, i64)> = None;
    if domains.is_empty() {
        if let Some(p) = parse_pem_info(payload.cert_content.as_deref().unwrap_or("")) {
            domains = p.domains_str.clone();
            if p.not_after > 0 {
                validity = Some((p.not_before, p.not_after));
            }
        } else if let Some(p) = parse_pem_info(payload.csr.as_deref().unwrap_or("")) {
            domains = p.domains_str.clone();
        }
    } else if let Some(p) = parse_pem_info(payload.cert_content.as_deref().unwrap_or(""))
        && p.not_after > 0
    {
        validity = Some((p.not_before, p.not_after));
    }

    let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new("UPDATE ssl_cert SET ");
    {
        let mut sep = qb.separated(", ");
        sep.push("name = ")
            .push_bind_unseparated(payload.name.trim());
        sep.push("domains = ").push_bind_unseparated(domains);
        sep.push("remark = ")
            .push_bind_unseparated(payload.remark.trim());
        sep.push("status = ").push_bind_unseparated(status);
        if let Some(nu) = new_owner {
            sep.push("user_id = ").push_bind_unseparated(nu);
        }
        sep.push("updated_at = ").push_bind_unseparated(now);
    }
    if let Some(v) = payload.cert_content.as_deref() {
        qb.push(", cert_content = ").push_bind(v.trim());
    }
    if let Some(v) = payload.key_content.as_deref() {
        qb.push(", key_content = ").push_bind(v.trim());
    }
    if let Some(v) = payload.ca_bundle.as_deref() {
        qb.push(", ca_bundle = ").push_bind(v.trim());
    }
    if let Some(v) = payload.csr.as_deref() {
        qb.push(", csr = ").push_bind(v.trim());
    }
    if let Some((nb, na)) = validity {
        qb.push(", not_before = ")
            .push_bind(nb)
            .push(", not_after = ")
            .push_bind(na);
    }
    qb.push(" WHERE id = ").push_bind(payload.id);
    let r = qb.build().execute(pool).await?;
    if r.rows_affected() == 0 {
        return Err(ZapError::New(-1, "证书不存在".to_string()));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssl_cert_update",
        &payload.name,
        &format!("id={}", payload.id),
    )
    .await;

    // 归属转移联动：证书不再属于部分站点时，自动解除这些站点的 HTTPS 绑定并重新同步
    if let Some(nu) = new_owner {
        let bound: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT s.id, s.user_id FROM site s JOIN site_profile p ON p.site_id = s.id \
             WHERE p.ssl_cert_id = ?",
        )
        .bind(payload.id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for (sid, suid) in bound {
            if suid != nu {
                let _ = sqlx::query(
                    "UPDATE site_profile SET ssl_cert_id = 0, force_https = 0 WHERE site_id = ?",
                )
                .bind(sid)
                .execute(pool)
                .await;
                if let Err(e) = crate::routers::site::sync_one_site(sid).await {
                    warn!(
                        "证书 {} 归属变更后自动解绑站点 {} 失败: {}",
                        payload.id, sid, e
                    );
                }
            }
        }
    }

    // 站点绑定联动：证书内容/状态可能已变更，重同步引用它的站点以刷新落盘证书与 vhost
    let sites: Vec<i64> =
        sqlx::query_scalar("SELECT site_id FROM site_profile WHERE ssl_cert_id = ?")
            .bind(payload.id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    for sid in sites {
        if let Err(e) = crate::routers::site::sync_one_site(sid).await {
            warn!("证书 {} 更新后重同步站点 {} 失败: {}", payload.id, sid, e);
        }
    }
    Ok(Json(json!({ "code": 0, "message": "OK" })))
}

#[derive(Debug, Deserialize)]
pub struct CertDeletePayload {
    pub id: i64,
}

pub async fn cert_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CertDeletePayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    // 归属可见性校验：admin 全部 / reseller 名下客户 / 普通用户仅自己的证书
    cert_in_scope(&claims, payload.id).await?;
    // 被站点绑定的证书不允许删除：先到站点「SSL/TLS」中解绑
    let refs: Vec<i64> =
        sqlx::query_scalar("SELECT site_id FROM site_profile WHERE ssl_cert_id = ?")
            .bind(payload.id)
            .fetch_all(pool)
            .await?;
    if !refs.is_empty() {
        return Err(ZapError::New(
            -1,
            format!(
                "该证书正被 {} 个站点绑定，请先在站点「SSL/TLS」页中解绑后再删除",
                refs.len()
            ),
        ));
    }
    let r = sqlx::query("DELETE FROM ssl_cert WHERE id = ?")
        .bind(payload.id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(ZapError::New(-1, "证书不存在".to_string()));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssl_cert_delete",
        &format!("id={}", payload.id),
        "删除证书",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "OK" })))
}

// ── 证书解析（自动读取域名 / 有效期等，供添加证书时自动填充）──
//
// 安全说明：这里刻意不走 OpenSSL。证书 / CSR 是用户在页面上粘贴的**不可信输入**，
// 而 X.509 / ASN.1 解析历史上是 OpenSSL 的高危区域（Heartbleed、ASN.1 BIO 系列漏洞等）。
// 改用纯 Rust 的 `x509-parser`（基于 nom 的 DER 解析，无 C 代码、无 unsafe），
// 可让这条链路完全处于 Rust 的内存安全保证之内；指纹用 RustCrypto 的 sha2 自行计算。

/// 证书解析结果。`pub(crate)`：「Zap 设置」展示面板当前证书时复用同一套解析逻辑。
#[derive(Debug, serde::Serialize)]
pub(crate) struct ParsedCertInfo {
    /// cert：X.509 证书；csr：证书签名请求
    pub(crate) kind: String,
    pub(crate) domains: Vec<String>,
    pub(crate) domains_str: String,
    pub(crate) common_name: String,
    pub(crate) subject: String,
    pub(crate) issuer: String,
    pub(crate) not_before: i64,
    pub(crate) not_after: i64,
    pub(crate) serial: String,
    pub(crate) fingerprint: String,
    pub(crate) key_type: String,
    pub(crate) key_bits: u32,
    /// SAN 中解析出的域名数量（0 表示证书不含 SAN，域名取自 CN）
    pub(crate) sans_count: usize,
    /// PEM 中包含的证书数量（>1 说明粘贴的是含中间链的 fullchain）
    pub(crate) cert_count: usize,
    /// 与私钥的匹配结果（仅当同时提交了私钥时才有值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) key_match: Option<bool>,
    /// 无法完成匹配校验的原因（如私钥格式错误 / 带密码）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) key_error: Option<String>,
}

fn push_uniq(v: &mut Vec<String>, s: String) {
    let s = s.trim().to_string();
    if s.is_empty() {
        return;
    }
    if !v.iter().any(|x| x.eq_ignore_ascii_case(&s)) {
        v.push(s);
    }
}

/// 从 PEM 中取出第一张证书（fullchain 时即叶子证书），并统计证书总数量。
fn first_cert_der(pem: &str) -> Option<(Vec<u8>, usize)> {
    use x509_parser::pem::Pem;

    let mut count = 0usize;
    let mut first: Option<Vec<u8>> = None;
    for item in Pem::iter_from_buffer(pem.as_bytes()) {
        let Ok(block) = item else { continue };
        let label = block.label.trim().to_ascii_uppercase();
        // 跳过 CSR（CERTIFICATE REQUEST）等非证书块
        if !label.contains("CERTIFICATE") || label.contains("REQUEST") {
            continue;
        }
        count += 1;
        if first.is_none() {
            first = Some(block.contents.clone());
        }
    }
    first.map(|der| (der, count))
}

/// 收集 GeneralName 列表中的 dNSName / iPAddress。
fn collect_general_names(
    names: &[x509_parser::extensions::GeneralName<'_>],
    out: &mut Vec<String>,
) {
    use x509_parser::extensions::GeneralName;

    for gn in names {
        match gn {
            GeneralName::DNSName(d) => push_uniq(out, (*d).to_string()),
            GeneralName::IPAddress(b) => {
                if b.len() == 4 {
                    let mut a = [0u8; 4];
                    a.copy_from_slice(b);
                    push_uniq(out, std::net::Ipv4Addr::from(a).to_string());
                } else if b.len() == 16 {
                    let mut a = [0u8; 16];
                    a.copy_from_slice(b);
                    push_uniq(out, std::net::Ipv6Addr::from(a).to_string());
                }
            }
            _ => {}
        }
    }
}

/// SHA256 指纹（冒号分隔）。
fn sha256_fingerprint(der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode_upper(Sha256::digest(der))
        .as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect::<Vec<_>>()
        .join(":")
}

/// X509Name → 常用名（CN）。
fn common_name(name: &x509_parser::x509::X509Name<'_>) -> String {
    name.iter_common_name()
        .filter_map(|a| a.as_str().ok())
        .next()
        .unwrap_or("")
        .to_string()
}

/// 公钥类型与位数（Ed25519 / Ed448 等 `parsed()` 未覆盖的算法按 OID 兜底）。
fn key_meta(spki: &x509_parser::x509::SubjectPublicKeyInfo<'_>) -> (String, u32) {
    use x509_parser::public_key::PublicKey;

    let bits = spki.parsed().map(|k| k.key_size() as u32).unwrap_or(0);
    let name = match spki.parsed() {
        Ok(PublicKey::RSA(_)) => "RSA",
        Ok(PublicKey::EC(_)) => "EC",
        Ok(PublicKey::DSA(_)) => "DSA",
        Ok(PublicKey::GostR3410(_)) | Ok(PublicKey::GostR3410_2012(_)) => "GOST",
        _ => match spki.algorithm.algorithm.to_id_string().as_str() {
            "1.3.101.112" => "Ed25519",
            "1.3.101.113" => "Ed448",
            _ => "未知",
        },
    };
    (name.to_string(), bits)
}

pub(crate) fn parse_certificate(pem: &str) -> Option<ParsedCertInfo> {
    use x509_parser::prelude::*;

    let (der, cert_count) = first_cert_der(pem)?;
    let (_, cert) = X509Certificate::from_der(&der).ok()?;
    let subject = cert.subject().to_string();
    let issuer = cert.issuer().to_string();
    let cn = common_name(cert.subject());

    let mut domains: Vec<String> = Vec::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        collect_general_names(&san.value.general_names, &mut domains);
    }
    let sans_count = domains.len();
    // 无 SAN 时退回 CN（老式证书 / 自签名证书常见）
    if domains.is_empty() && !cn.is_empty() {
        domains.push(cn.clone());
    }
    let validity = cert.validity();
    let (key_type, key_bits) = key_meta(cert.public_key());

    Some(ParsedCertInfo {
        kind: "cert".to_string(),
        domains_str: domains.join(", "),
        domains,
        common_name: cn,
        subject,
        issuer,
        not_before: validity.not_before.timestamp(),
        not_after: validity.not_after.timestamp(),
        serial: cert.raw_serial_as_string(),
        fingerprint: sha256_fingerprint(&der),
        key_type,
        key_bits,
        sans_count,
        cert_count,
        key_match: None,
        key_error: None,
    })
}

fn parse_csr_pem(pem: &str) -> Option<ParsedCertInfo> {
    use x509_parser::prelude::*;

    let (_, block) = parse_x509_pem(pem.as_bytes()).ok()?;
    let (_, req) = X509CertificationRequest::from_der(&block.contents).ok()?;
    let info = &req.certification_request_info;
    let subject = info.subject.to_string();
    let cn = common_name(&info.subject);

    let mut domains: Vec<String> = Vec::new();
    if let Some(exts) = req.requested_extensions() {
        for ext in exts {
            if let ParsedExtension::SubjectAlternativeName(san) = ext {
                collect_general_names(&san.general_names, &mut domains);
            }
        }
    }
    let sans_count = domains.len();
    if domains.is_empty() && !cn.is_empty() {
        domains.push(cn.clone());
    }
    let (key_type, key_bits) = key_meta(&info.subject_pki);

    Some(ParsedCertInfo {
        kind: "csr".to_string(),
        domains_str: domains.join(", "),
        domains,
        common_name: cn,
        subject,
        issuer: String::new(),
        not_before: 0,
        not_after: 0,
        serial: String::new(),
        fingerprint: String::new(),
        key_type,
        key_bits,
        sans_count,
        cert_count: 0,
        key_match: None,
        key_error: None,
    })
}

/// 自动识别 PEM 类型并解析（证书优先取第一张，即叶子证书）。
fn parse_pem_info(raw: &str) -> Option<ParsedCertInfo> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if t.contains("BEGIN CERTIFICATE REQUEST") || t.contains("BEGIN NEW CERTIFICATE REQUEST") {
        return parse_csr_pem(t);
    }
    if t.contains("BEGIN CERTIFICATE")
        || t.contains("BEGIN X509 CERTIFICATE")
        || t.contains("BEGIN TRUSTED CERTIFICATE")
    {
        return parse_certificate(t);
    }
    None
}

// ── 证书 / CSR 与私钥配对校验 ────────────────────────────────

/// 校验证书（或 CSR）里的公钥与给定私钥是否属于同一对密钥。
///
/// - `Ok(true)`：公钥一致，私钥与该证书匹配
/// - `Ok(false)`：两者都能解析，但公钥不一致
/// - `Err(msg)`：私钥 / 证书无法解析，无法完成校验
pub(crate) fn key_matches(pem: &str, key_pem: &str) -> Result<bool, String> {
    // 与证书解析链路一样刻意避开 OpenSSL：私钥用 rcgen（ring 后端）解析并导出
    // SubjectPublicKeyInfo，证书 / CSR 用 x509-parser，两边都只比较裸公钥位串。
    let cert_key = cert_pubkey_bits(pem)?;

    // 私钥优先；也接受直接粘贴公钥（便于只做比对）
    let kp = key_pem.trim();
    if kp.contains("PUBLIC KEY") {
        let der =
            first_pem_der(kp).ok_or_else(|| "公钥无法解析：请粘贴 PEM 格式公钥".to_string())?;
        return Ok(spki_bits(&der)? == cert_key);
    }
    Ok(private_key_pubkey_bits(kp)? == cert_key)
}

/// 取 PEM 中第一个块的 DER 内容（忽略标签）。
fn first_pem_der(pem: &str) -> Option<Vec<u8>> {
    x509_parser::pem::Pem::iter_from_buffer(pem.as_bytes())
        .flatten()
        .next()
        .map(|b| b.contents.clone())
}

/// 从 SubjectPublicKeyInfo 的 DER 中取出裸公钥位串。
fn spki_bits(spki_der: &[u8]) -> Result<Vec<u8>, String> {
    use x509_parser::prelude::FromDer;

    let (_, spki) = x509_parser::x509::SubjectPublicKeyInfo::from_der(spki_der)
        .map_err(|e| format!("公钥无法解析：{e}"))?;
    Ok(spki.subject_public_key.as_ref().to_vec())
}

/// 取证书（或 CSR）里的裸公钥位串。
fn cert_pubkey_bits(pem: &str) -> Result<Vec<u8>, String> {
    use x509_parser::prelude::FromDer;

    for item in x509_parser::pem::Pem::iter_from_buffer(pem.as_bytes()) {
        let Ok(block) = item else { continue };
        let label = block.label.trim().to_ascii_uppercase();
        if label.contains("REQUEST") {
            let (_, csr) = x509_parser::certification_request::X509CertificationRequest::from_der(
                &block.contents,
            )
            .map_err(|e| format!("CSR 无法解析：{e}"))?;
            return Ok(csr
                .certification_request_info
                .subject_pki
                .subject_public_key
                .as_ref()
                .to_vec());
        }
        if label.contains("CERTIFICATE") {
            let (_, cert) = x509_parser::certificate::X509Certificate::from_der(&block.contents)
                .map_err(|e| format!("证书无法解析：{e}"))?;
            return Ok(cert
                .tbs_certificate
                .subject_pki
                .subject_public_key
                .as_ref()
                .to_vec());
        }
    }
    Err("证书无法解析：请粘贴 PEM 格式的证书（crt）或 CSR".to_string())
}

/// 解析私钥并导出它的裸公钥位串（与证书侧同格式，便于直接比对）。
fn private_key_pubkey_bits(key_pem: &str) -> Result<Vec<u8>, String> {
    let pair = rcgen::KeyPair::from_pem(key_pem)
        .map_err(|_| "私钥无法解析：请粘贴 PEM 格式私钥（暂不支持带密码的私钥）".to_string())?;
    spki_bits(&pair.public_key_der())
}

/// 保存前的配对校验：证书（或 CSR）与私钥同时提供时必须能配上，
/// 避免把无法部署的材料存进库里。
fn check_cert_key_pair(cert: &str, csr: &str, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Ok(());
    }
    let material = if !cert.trim().is_empty() { cert } else { csr };
    if material.trim().is_empty() {
        // 只填了私钥、没有证书 / CSR，无从比对
        return Ok(());
    }
    match key_matches(material, key) {
        Ok(true) => Ok(()),
        Ok(false) => {
            Err("证书与私钥不匹配：该私钥不属于这张证书（或 CSR），保存后无法部署".to_string())
        }
        Err(e) => Err(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CertParsePayload {
    #[serde(default)]
    pub pem: String,
    /// 可选的私钥 PEM：提供时顺便校验两者是否匹配
    #[serde(default)]
    pub key_pem: String,
}

/// 解析证书 / CSR：返回域名、有效期、签发者、指纹等，用于添加证书时自动填充；
/// 同时传入 `key_pem` 时会一并给出「证书与私钥是否匹配」的结果。
pub async fn cert_parse(
    _claims: ValidatedClaims,
    Json(payload): Json<CertParsePayload>,
) -> ZapJsonResult {
    let mut info = match parse_pem_info(&payload.pem) {
        Some(i) => i,
        None => {
            return Err(ZapError::New(
                -1,
                "无法识别证书内容：请粘贴 PEM 格式的证书（crt）或 CSR".to_string(),
            ));
        }
    };
    let key = payload.key_pem.trim();
    if !key.is_empty() {
        match key_matches(&payload.pem, key) {
            Ok(m) => info.key_match = Some(m),
            Err(e) => info.key_error = Some(e),
        }
    }
    Ok(Json(json!({ "code": 0, "message": "OK", "data": info })))
}
// ── 自签名生成 ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CertSelfSignPayload {
    pub name: String,
    /// 证书归属用户 id：admin / reseller 可选（默认自己），普通用户强制自己
    #[serde(default)]
    pub user_id: Option<i64>,
    pub domains: String,
    #[serde(default = "default_sign_days")]
    pub days: i64,
    #[serde(default)]
    pub remark: String,
}

fn default_sign_days() -> i64 {
    365
}

/// 拆分逗号 / 空格 / 换行分隔的域名列表。
pub(crate) fn split_domains(raw: &str) -> Vec<String> {
    raw.split([',', ' ', '\t', '\n', '\r', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 使用 rcgen 生成自签名证书，返回 (crt, key, csr, not_before, not_after)。
fn gen_self_signed(
    domains: &[String],
    days: i64,
) -> Result<(String, String, String, i64, i64), ZapError> {
    use rcgen::{CertificateParams, DnType, KeyPair};
    use time::{Duration, OffsetDateTime};

    if domains.is_empty() {
        return Err(ZapError::New(
            -1,
            "请填写至少一个域名或 IP（多个用逗号分隔）".to_string(),
        ));
    }
    let mut params = CertificateParams::new(domains.to_vec())
        .map_err(|e| ZapError::New(-1, format!("域名不合法: {e}")))?;
    params
        .distinguished_name
        .push(DnType::CommonName, domains[0].as_str());
    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(days);

    let key_pair =
        KeyPair::generate().map_err(|e| ZapError::New(-1, format!("密钥生成失败: {e}")))?;
    let csr = params
        .serialize_request(&key_pair)
        .map_err(|e| ZapError::New(-1, format!("CSR 生成失败: {e}")))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| ZapError::New(-1, format!("证书签名失败: {e}")))?;
    let csr_pem = csr
        .pem()
        .map_err(|e| ZapError::New(-1, format!("CSR 序列化失败: {e}")))?;

    Ok((
        cert.pem(),
        key_pair.serialize_pem(),
        csr_pem,
        (now - Duration::days(1)).unix_timestamp(),
        (now + Duration::days(days)).unix_timestamp(),
    ))
}

pub async fn cert_self_sign(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CertSelfSignPayload>,
) -> ZapJsonResult {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ZapError::New(-1, "请填写证书名称".to_string()));
    }
    let domains = split_domains(&payload.domains);
    let days = payload.days.clamp(1, 3650);
    let (cert_pem, key_pem, csr_pem, not_before, not_after) = gen_self_signed(&domains, days)?;

    // 归属用户：admin / reseller 可指定（resolve_cert_owner 校验范围）；普通用户强制自己
    let owner_user = match payload.user_id {
        Some(t) => {
            resolve_cert_owner(&claims, t).await?;
            t
        }
        None => claims.id as i64,
    };

    let pool = db::get_db_pool().await;
    let now = chrono::Utc::now().timestamp();
    let domains_str = domains.join(", ");
    let r = sqlx::query(
        "INSERT INTO ssl_cert
            (user_id, name, domains, cert_type, cert_content, key_content, ca_bundle, csr,
             not_before, not_after, status, remark, created_at, updated_at)
         VALUES (?, ?, ?, 'self-signed', ?, ?, '', ?, ?, ?, 1, ?, ?, ?)",
    )
    .bind(owner_user)
    .bind(&name)
    .bind(&domains_str)
    .bind(&cert_pem)
    .bind(&key_pem)
    .bind(&csr_pem)
    .bind(not_before)
    .bind(not_after)
    .bind(payload.remark.trim())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    let id = r.last_insert_rowid();
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssl_cert_self_sign",
        &name,
        &format!("id={id}, domains={domains_str}, days={days}"),
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": { "id": id } }),
    ))
}

// ── Let's Encrypt 申请（ACME 异步订单）──────────────────────
//
// 一个订单代表一次申请：下单后服务端只把订单句柄与各个域名的验证材料落库，
// 后续（用户去解析 DNS / 后台自动校验签发）都围绕这个订单记录展开。
// 前端拿到 order_id 后轮询 status 即可，不必长时间挂起某个 HTTP 请求。

#[derive(Debug, Deserialize)]
pub struct CertLetsEncryptPayload {
    pub email: String,
    pub domains: String,
    /// 证书归属用户 id：admin / reseller 可选（默认自己），普通用户强制自己
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub staging: Option<bool>,
    #[serde(default)]
    pub remark: Option<String>,
    /// 验证方式：`http`（自动写验证文件）| `dns`（默认）
    #[serde(default)]
    pub validation: Option<String>,
    /// dns-01 的子模式：`manual`（默认，用户自行解析）| `auto`（调 DNS 服务商 API）
    #[serde(default)]
    pub dns_mode: Option<String>,
    /// dns-01 自动模式使用的 DNS 服务商 id
    #[serde(default)]
    pub dns_provider_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct LetsEncryptQuery {
    pub order_id: i64,
}

/// 下单创建 ACME 订单。
///
/// - `dns` + `manual`（默认）：返回待添加的 TXT 记录，用户解析完后调 `/letsencrypt/verify`；
/// - `dns` + `auto`、`http`：后台任务自动推进到签发，返回 `order_id` 供轮询。
pub async fn cert_letsencrypt(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CertLetsEncryptPayload>,
) -> ZapJsonResult {
    let email = payload.email.trim().to_string();
    if email.is_empty() {
        return Err(ZapError::New(-1, "请填写 ACME 账户邮箱".to_string()));
    }
    let domains = split_domains(&payload.domains);
    if domains.is_empty() {
        return Err(ZapError::New(-1, "请填写至少一个域名".to_string()));
    }
    // 归属用户：admin / reseller 可指定（resolve_cert_owner 校验范围）；普通用户强制自己
    let owner_user = match payload.user_id {
        Some(t) => {
            resolve_cert_owner(&claims, t).await?;
            t
        }
        None => claims.id as i64,
    };

    let kind = acme::ChallengeKind::parse(payload.validation.as_deref().unwrap_or_default());
    let is_dns = kind == acme::ChallengeKind::Dns01;
    let dns_mode = acme::DnsMode::parse(payload.dns_mode.as_deref().unwrap_or_default(), is_dns);
    let dns_provider_id = payload.dns_provider_id.unwrap_or(0);

    // 自动模式必须先把「哪个服务商、用谁的凭据」讲清楚，避免后台线程里才发现权限不对
    if dns_mode == acme::DnsMode::Auto {
        if dns_provider_id <= 0 {
            return Err(ZapError::New(
                -1,
                "请选择 DNS 服务商（自动 DNS 验证需要调用其 API 添加 TXT 记录）".to_string(),
            ));
        }
        let provider = acme::dns_in_scope(&claims, dns_provider_id).await?;
        if provider.user_id != owner_user && !jwt::is_admin(&claims) {
            return Err(ZapError::New(
                -1,
                "DNS 服务商凭据与证书归属用户不一致，请调整归属或改用手动 DNS 验证".to_string(),
            ));
        }
    }

    let staging = payload.staging.unwrap_or(false);
    let name = payload
        .name
        .clone()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| domains[0].clone());
    let remark = payload
        .remark
        .as_deref()
        .unwrap_or("Let's Encrypt 自动申请")
        .trim()
        .to_string();

    let order = acme::create_order(acme::CreateParams {
        user_id: owner_user,
        email,
        domains: domains.clone(),
        name,
        remark,
        staging,
        challenge: kind,
        dns_mode,
        dns_provider_id,
    })
    .await
    .map_err(|e| ZapError::New(-1, e))?;

    let domains_str = domains.join(", ");
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssl_cert_letsencrypt",
        &domains_str,
        &format!(
            "order_id={}, kind={}, dns_mode={}, staging={}",
            order.id, order.challenge_type, order.dns_mode, staging
        ),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "OK", "data": order })))
}

/// 轮询订单状态：`pending` / `processing` / `issued` / `failed` / `cancelled`。
pub async fn letsencrypt_status(
    Query(q): Query<LetsEncryptQuery>,
    claims: ValidatedClaims,
) -> ZapJsonResult {
    let row = acme::order_in_scope(&claims, q.order_id).await?;
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": acme::view(row.id).await.map_err(|e| ZapError::New(-1, e))? }),
    ))
}

/// 触发域名验证：DNS 手动模式下用户完成解析后点击；自动 / HTTP 模式幂等无副作用。
pub async fn letsencrypt_verify(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<LetsEncryptQuery>,
) -> ZapJsonResult {
    let row = acme::order_in_scope(&claims, payload.order_id).await?;
    let view = acme::trigger(row.id)
        .await
        .map_err(|e| ZapError::New(-1, e))?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssl_cert_letsencrypt_verify",
        &view.domains.join(", "),
        &format!("order_id={}", view.id),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "OK", "data": view })))
}

/// 取消订单：清理已落的验证文件 / DNS 记录，订单置为 cancelled。
pub async fn letsencrypt_cancel(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<LetsEncryptQuery>,
) -> ZapJsonResult {
    let row = acme::order_in_scope(&claims, payload.order_id).await?;
    let view = acme::cancel(row.id)
        .await
        .map_err(|e| ZapError::New(-1, e))?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssl_cert_letsencrypt_cancel",
        &view.domains.join(", "),
        &format!("order_id={}", view.id),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "OK", "data": view })))
}

/// 未完成的订单列表：前端重进页面后可以直接接着走（不用重新填一遍域名）。
pub async fn letsencrypt_orders(claims: ValidatedClaims) -> ZapJsonResult {
    let orders = acme::list_orders(&claims)
        .await
        .map_err(|e| ZapError::New(-1, e))?;
    Ok(Json(json!({ "code": 0, "message": "OK", "data": orders })))
}

// ── ACME 的 DNS 服务商凭据管理 ─────────────────────────────

/// 支持的服务商及其凭据字段定义（前端据此动态渲染表单，后端不再硬编码）。
pub async fn acme_dns_providers(_claims: ValidatedClaims) -> ZapJsonResult {
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": acme::dns::providers() }),
    ))
}

/// 凭据列表（凭据内容一律脱敏）。
pub async fn acme_dns_list(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let rows: Vec<(i64, i64, String, String, String, String, i64)> = if jwt::is_admin(&claims) {
        sqlx::query_as(
            "SELECT id, user_id, name, provider, credentials, remark, status
             FROM ssl_acme_dns_provider ORDER BY id DESC",
        )
        .fetch_all(pool)
        .await?
    } else if jwt::is_reseller(&claims) {
        sqlx::query_as(
            "SELECT id, user_id, name, provider, credentials, remark, status
             FROM ssl_acme_dns_provider
             WHERE user_id = ? OR user_id IN (SELECT id FROM user WHERE owner_id = ?)
             ORDER BY id DESC",
        )
        .bind(claims.id as i64)
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT id, user_id, name, provider, credentials, remark, status
             FROM ssl_acme_dns_provider WHERE user_id = ? ORDER BY id DESC",
        )
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await?
    };

    let data: Vec<serde_json::Value> = rows
        .into_iter()
        .map(
            |(id, user_id, name, provider, credentials, remark, status)| {
                let plain = crypto::decrypt(&credentials).unwrap_or_default();
                let creds: serde_json::Value = serde_json::from_str(&plain).unwrap_or_default();
                json!({
                    "id": id,
                    "user_id": user_id,
                    "name": name,
                    "provider": provider,
                    "credentials": acme::dns::mask(&provider, &creds),
                    "remark": remark,
                    "status": status,
                })
            },
        )
        .collect();
    Ok(Json(json!({ "code": 0, "message": "OK", "data": data })))
}

#[derive(Debug, Deserialize)]
pub struct AcmeDnsSavePayload {
    /// 传 id 为更新：未提供的凭据字段保持原值（编辑时不强制重填）
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    pub provider: String,
    /// 各服务商所需字段，见 `/ssl/acme/dns/providers`
    #[serde(default)]
    pub credentials: Option<serde_json::Value>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub user_id: Option<i64>,
}

/// 新增 / 更新 DNS 服务商凭据。凭据经 AES-256-GCM 加密入库。
pub async fn acme_dns_save(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<AcmeDnsSavePayload>,
) -> ZapJsonResult {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ZapError::New(-1, "请填写服务商名称".to_string()));
    }
    let provider = payload.provider.trim().to_string();
    if !acme::dns::providers().iter().any(|p| p.kind == provider) {
        return Err(ZapError::New(
            -1,
            format!("不支持的 DNS 服务商：{provider}"),
        ));
    }
    let given = payload.credentials.unwrap_or_default();

    let pool = db::get_db_pool().await;
    let now = chrono::Utc::now().timestamp();
    let saved_id = if let Some(id) = payload.id {
        // 更新：先把库里已有的凭据读出来，只覆盖本次提交的字段
        let row = acme::dns_in_scope(&claims, id).await?;
        let plain = crypto::decrypt(&row.credentials).unwrap_or_default();
        let mut creds: serde_json::Value = serde_json::from_str(&plain)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(map) = given.clone() {
            for (k, v) in map {
                // 空值的输入框视为「不修改」
                let empty = v.as_str().map(|s| s.trim().is_empty()).unwrap_or(false);
                if !empty {
                    creds[k] = v;
                }
            }
        }
        // 落库前自检：字段缺失时在这里就报错，而不是等到签发时才失败
        acme::dns::build(&provider, &creds).map_err(|e| ZapError::New(-1, e))?;
        let enc = crypto::encrypt(&creds.to_string())
            .map_err(|e| ZapError::New(-1, format!("凭据加密失败: {e}")))?;
        sqlx::query(
            "UPDATE ssl_acme_dns_provider SET name = ?, provider = ?, credentials = ?, remark = ?,
                    updated_at = ?
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&provider)
        .bind(&enc)
        .bind(payload.remark.as_deref().unwrap_or_default().trim())
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
        id
    } else {
        acme::dns::build(&provider, &given).map_err(|e| ZapError::New(-1, e))?;
        let owner = match payload.user_id {
            Some(t) => {
                // 归属规则与 SSL 证书一致：reseller 只能选自己或名下客户
                crate::routers::site::resolve_target_user(&claims, t).await?;
                t
            }
            None => claims.id as i64,
        };
        let enc = crypto::encrypt(&given.to_string())
            .map_err(|e| ZapError::New(-1, format!("凭据加密失败: {e}")))?;
        let r = sqlx::query(
            "INSERT INTO ssl_acme_dns_provider
                (user_id, name, provider, credentials, remark, status, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(owner)
        .bind(&name)
        .bind(&provider)
        .bind(&enc)
        .bind(payload.remark.as_deref().unwrap_or_default().trim())
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        r.last_insert_rowid()
    };

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssl_acme_dns_save",
        &name,
        &format!("provider={provider}, id={saved_id}"),
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": { "id": saved_id } }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct AcmeDnsIdPayload {
    pub id: i64,
}

/// 删除 DNS 服务商凭据：被进行中的订单引用时拒绝删除。
pub async fn acme_dns_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<AcmeDnsIdPayload>,
) -> ZapJsonResult {
    let row = acme::dns_in_scope(&claims, payload.id).await?;
    let (busy,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ssl_acme_order
         WHERE dns_provider_id = ? AND status IN ('pending','processing')",
    )
    .bind(payload.id)
    .fetch_one(db::get_db_pool().await)
    .await?;
    if busy > 0 {
        return Err(ZapError::New(
            -1,
            "有进行中的证书申请正在使用该服务商，请先取消对应订单".to_string(),
        ));
    }
    sqlx::query("DELETE FROM ssl_acme_dns_provider WHERE id = ?")
        .bind(payload.id)
        .execute(db::get_db_pool().await)
        .await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssl_acme_dns_delete",
        &row.name,
        &format!("id={}, provider={}", row.id, row.provider),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "OK" })))
}

#[derive(Debug, Deserialize)]
pub struct AcmeDnsTestPayload {
    provider: String,
    #[serde(default)]
    credentials: serde_json::Value,
    /// 传 id 表示「用已保存的凭据测试」：缺失字段自动取库里的值，密钥本身不出网
    #[serde(default)]
    id: Option<i64>,
}

/// 连通性测试：不修改任何数据，用来在保存前 / 编辑后确认凭据有效。
pub async fn acme_dns_test(
    claims: ValidatedClaims,
    Json(payload): Json<AcmeDnsTestPayload>,
) -> ZapJsonResult {
    let mut creds = payload.credentials.clone();
    if let Some(id) = payload.id {
        let row = acme::dns_in_scope(&claims, id).await?;
        let plain = crypto::decrypt(&row.credentials).unwrap_or_default();
        let stored: serde_json::Value = serde_json::from_str(&plain).unwrap_or_default();
        if let serde_json::Value::Object(map) = stored {
            for (k, v) in map {
                // 本次提交的字段优先（用户可能还没保存就想试新密钥）
                if creds.get(&k).is_none() {
                    creds[k] = v;
                }
            }
        }
    }
    let provider = acme::dns::build(&payload.provider, &creds).map_err(|e| ZapError::New(-1, e))?;
    let detail = provider
        .ping()
        .await
        .map_err(|e| ZapError::New(-1, format!("连接测试失败：{e}")))?;
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": { "message": detail } }),
    ))
}

/// 将 PEM 证书链拆为（叶子证书, 中间链），供 crt / ca-bundle 分列存储。
pub(crate) fn split_leaf_chain(pem: &str) -> (String, String) {
    const END: &str = "-----END CERTIFICATE-----";
    let mut blocks: Vec<String> = Vec::new();
    for part in pem.split(END) {
        if let Some(pos) = part.find("-----BEGIN CERTIFICATE-----") {
            let seg = part[pos..].trim();
            if !seg.is_empty() {
                blocks.push(format!("{seg}{END}"));
            }
        }
    }
    if blocks.is_empty() {
        return (pem.trim().to_string(), String::new());
    }
    let leaf = blocks.remove(0);
    let ca = blocks.join("\n");
    (leaf, ca)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 证书 / CSR 的域名、有效期应能被自动解析出来（添加证书时无需手工填写域名）。
    #[test]
    fn parse_self_signed_materials() {
        let (cert_pem, _key, csr_pem, not_before, not_after) = gen_self_signed(
            &["example.com".to_string(), "www.example.com".to_string()],
            30,
        )
        .expect("生成自签名证书失败");

        let cert = parse_pem_info(&cert_pem).expect("解析证书失败");
        assert_eq!(cert.kind, "cert");
        assert_eq!(cert.domains, vec!["example.com", "www.example.com"]);
        assert_eq!(cert.common_name, "example.com");
        assert_eq!(cert.cert_count, 1);
        assert_eq!((cert.not_before, cert.not_after), (not_before, not_after));
        assert!(!cert.fingerprint.is_empty());

        let csr = parse_pem_info(&csr_pem).expect("解析 CSR 失败");
        assert_eq!(csr.kind, "csr");
        assert_eq!(csr.domains, vec!["example.com", "www.example.com"]);
    }

    #[test]
    fn parse_invalid_input() {
        assert!(parse_pem_info("").is_none());
        assert!(parse_pem_info("   ").is_none());
        assert!(parse_pem_info("not a pem at all").is_none());
    }

    /// 证书 / CSR 与私钥的配对校验：配套的应为 true，换一把私钥应为 false。
    #[test]
    fn cert_key_pair_check() {
        let (cert_pem, key_pem, csr_pem, _, _) =
            gen_self_signed(&["example.com".to_string()], 30).unwrap();

        assert_eq!(key_matches(&cert_pem, &key_pem), Ok(true));
        assert_eq!(key_matches(&csr_pem, &key_pem), Ok(true));
        assert_eq!(check_cert_key_pair(&cert_pem, "", &key_pem), Ok(()));
        assert_eq!(check_cert_key_pair("", &csr_pem, &key_pem), Ok(()));

        // 另一把私钥（自签名生成时用的是随机密钥对）
        let (_c2, other_key, _csr2, _, _) =
            gen_self_signed(&["example.com".to_string()], 30).unwrap();
        assert_eq!(key_matches(&cert_pem, &other_key), Ok(false));
        assert!(check_cert_key_pair(&cert_pem, "", &other_key).is_err());

        // 没填私钥 / 没填证书时不做校验
        assert_eq!(check_cert_key_pair(&cert_pem, "", ""), Ok(()));
        assert_eq!(check_cert_key_pair("", "", &key_pem), Ok(()));

        // 私钥内容非法时应给出可读错误而不是 panic
        assert!(key_matches(&cert_pem, "not a key").is_err());
    }
}
