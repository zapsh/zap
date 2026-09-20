//! Let's Encrypt（ACME v2）证书申请：异步订单编排。
//!
//! 因此这里把流程拆成**可跨请求存活的订单**：订单句柄（ACME order URL）、待签私钥、
//! 每个域名的挑战明细全部落库；HTTP-01 改为往面板自管的验证根写验证文件
//! （站点 vhost 里已固定渲染 `/.well-known/acme-challenge/` location，
//! 由 `zapexec` 以 root 身份落盘，全程不需要动 nginx 配置）。
//!
//! 对外语义：
//!   创建订单 →（DNS-01 手动时）用户去解析 → 触发验证 → 前端轮询订单状态 → 签发入库

pub mod dns;

use std::time::Duration;

use instant_acme::{
    Account, AccountCredentials, ChallengeType as AcmeChallengeType, Identifier, NewAccount,
    NewOrder, Order, OrderStatus, RetryPolicy,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use zap_proto::{AcmeChallengeEntry, Request};

use crate::{
    db,
    zap::{ZapError, crypto, jwt},
    zapexec,
};

/// Let's Encrypt 生产环境目录 URL。
pub const DIRECTORY_LE: &str = "https://acme-v02.api.letsencrypt.org/directory";
/// Let's Encrypt 测试环境（签发假证书，用于联调，不受正式配额限制）。
pub const DIRECTORY_LE_STAGING: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";

/// 订单在库里的最长保留时间（到期后前端不再展示，避免堆积）。
const ORDER_TTL: i64 = 7 * 86400;
/// 通知服务端校验后，等待校验 / 签发结果的上限。
const POLL_TIMEOUT: Duration = Duration::from_secs(120);
/// DNS-01 等待 TXT 记录传播的上限。
const DNS_WAIT_TIMEOUT: Duration = Duration::from_secs(120);

// ── 类型 ───────────────────────────────────────────────────

/// 域名验证方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeKind {
    Http01,
    Dns01,
}

impl ChallengeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChallengeKind::Http01 => "http-01",
            ChallengeKind::Dns01 => "dns-01",
        }
    }

    fn acme_type(&self) -> AcmeChallengeType {
        match self {
            ChallengeKind::Http01 => AcmeChallengeType::Http01,
            ChallengeKind::Dns01 => AcmeChallengeType::Dns01,
        }
    }

    /// 从前端字符串解析；`dns` 为默认（面板主推，且是唯一支持泛域名的方式）。
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "http" | "http-01" | "http01" => ChallengeKind::Http01,
            _ => ChallengeKind::Dns01,
        }
    }
}

/// DNS-01 的子模式：手动（用户自行解析）/ 自动（面板调 DNS 服务商 API）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsMode {
    Manual,
    Auto,
}

impl DnsMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            DnsMode::Manual => "manual",
            DnsMode::Auto => "auto",
        }
    }

    pub fn parse(s: &str, allow_auto: bool) -> Self {
        if allow_auto && matches!(s.trim().to_lowercase().as_str(), "auto" | "automatic") {
            DnsMode::Auto
        } else {
            DnsMode::Manual
        }
    }
}

/// 单个域名的挑战明细（JSON 存在 `ssl_acme_order.challenges`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChallengeItem {
    /// ACME 侧展示的域名（泛域名带 `*.` 前缀）
    pub domain: String,
    /// pending | validating | valid | invalid
    pub status: String,
    #[serde(default)]
    pub challenge_url: String,
    /// http-01：验证文件名
    #[serde(default)]
    pub token: String,
    /// http-01：验证文件内容
    #[serde(default)]
    pub key_auth: String,
    /// dns-01：待添加的主机记录 `_acme-challenge.<域>`
    #[serde(default)]
    pub dns_host: String,
    /// dns-01：TXT 记录值
    #[serde(default)]
    pub dns_value: String,
    /// dns-01 自动模式：服务商返回的记录 id（回收时用）
    #[serde(default)]
    pub dns_record_id: String,
    /// dns-01：TXT 是否已在公共递归 DNS 上可见
    #[serde(default)]
    pub propagated: bool,
}

#[derive(sqlx::FromRow, Debug, Clone, Default)]
pub struct AcmeOrderRow {
    pub id: i64,
    pub user_id: i64,
    pub account_id: i64,
    pub cert_id: i64,
    pub name: String,
    pub remark: String,
    pub domains: String,
    pub directory: String,
    pub status: String,
    pub challenge_type: String,
    pub dns_mode: String,
    pub dns_provider_id: i64,
    pub order_url: String,
    pub key_pem: String,
    pub challenges: String,
    pub error: String,
    pub expires_at: i64,
}

impl AcmeOrderRow {
    fn items(&self) -> Vec<ChallengeItem> {
        serde_json::from_str(&self.challenges).unwrap_or_default()
    }
}

/// 下发给前端的订单视图。
#[derive(Debug, Clone, Serialize)]
pub struct OrderView {
    pub id: i64,
    pub status: String,
    /// 人类可读的当前阶段，前端可直接展示
    pub stage: String,
    pub cert_id: i64,
    pub domains: Vec<String>,
    pub challenge_type: String,
    pub dns_mode: String,
    /// DNS-01 手动模式下需要用户去添加的解析记录
    pub todos: Vec<ChallengeItem>,
    pub error: String,
    pub expires_at: i64,
}

/// 创建订单的参数。
pub struct CreateParams {
    pub user_id: i64,
    pub email: String,
    pub domains: Vec<String>,
    pub name: String,
    pub remark: String,
    pub staging: bool,
    pub challenge: ChallengeKind,
    pub dns_mode: DnsMode,
    pub dns_provider_id: i64,
}

fn le_err(e: instant_acme::Error) -> String {
    format!("ACME 协议错误：{e}")
}

/// 把认证 / 签发过程中的技术错误翻译成人话。
fn friendly_err(e: String) -> String {
    if e.contains("urn:ietf:params:acme:error:rateLimited") || e.contains("429") {
        return "超出 Let's Encrypt 申请频率限制（同一主域名每周有配额），请稍后再试；\
                联调用先切「测试环境」不会占用正式配额"
            .to_string();
    }
    if e.contains("urn:ietf:params:acme:error:unauthorized") {
        return format!("域名验证未通过：{e}");
    }
    if e.contains("urn:ietf:params:acme:error:incorrect") {
        return format!("DNS 记录已生效但内容不匹配，请删除旧的同名 TXT 后重试：{e}");
    }
    e
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn directory_url(directory: &str) -> &'static str {
    if directory == "letsencrypt-staging" {
        DIRECTORY_LE_STAGING
    } else {
        DIRECTORY_LE
    }
}

/// 去掉泛域名前缀：`*.example.com` → `example.com`（DNS-01 的记录名基于裸域）。
pub fn strip_wildcard(domain: &str) -> String {
    domain.trim().trim_start_matches("*.").to_string()
}

// ── 订单读写 ───────────────────────────────────────────────

async fn load_row(order_id: i64) -> Result<AcmeOrderRow, String> {
    let row: Option<AcmeOrderRow> = sqlx::query_as(
        "SELECT id, user_id, account_id, cert_id, name, remark, domains, directory, status,
                challenge_type, dns_mode, dns_provider_id, order_url, key_pem, challenges,
                error, expires_at
         FROM ssl_acme_order WHERE id = ?",
    )
    .bind(order_id)
    .fetch_optional(db::get_db_pool().await)
    .await
    .map_err(|e| format!("读取订单失败: {e}"))?;
    row.ok_or_else(|| "订单不存在".to_string())
}

async fn set_status(order_id: i64, status: &str) -> Result<(), String> {
    sqlx::query("UPDATE ssl_acme_order SET status = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(now_secs())
        .bind(order_id)
        .execute(db::get_db_pool().await)
        .await
        .map_err(|e| format!("更新订单状态失败: {e}"))?;
    Ok(())
}

async fn set_error(order_id: i64, err: &str) -> Result<(), String> {
    sqlx::query(
        "UPDATE ssl_acme_order SET status = 'failed', error = ?, updated_at = ? WHERE id = ?",
    )
    .bind(err)
    .bind(now_secs())
    .bind(order_id)
    .execute(db::get_db_pool().await)
    .await
    .map_err(|e| format!("写入订单错误失败: {e}"))?;
    Ok(())
}

async fn save_items(order_id: i64, items: &[ChallengeItem]) -> Result<(), String> {
    sqlx::query("UPDATE ssl_acme_order SET challenges = ?, updated_at = ? WHERE id = ?")
        .bind(serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string()))
        .bind(now_secs())
        .bind(order_id)
        .execute(db::get_db_pool().await)
        .await
        .map_err(|e| format!("更新挑战明细失败: {e}"))?;
    Ok(())
}

async fn mark_issued(order_id: i64, cert_id: i64) -> Result<(), String> {
    sqlx::query(
        "UPDATE ssl_acme_order SET status = 'issued', cert_id = ?, error = '', updated_at = ? \
         WHERE id = ?",
    )
    .bind(cert_id)
    .bind(now_secs())
    .bind(order_id)
    .execute(db::get_db_pool().await)
    .await
    .map_err(|e| format!("更新订单结果失败: {e}"))?;
    Ok(())
}

fn view_of(row: &AcmeOrderRow) -> OrderView {
    let stage = match row.status.as_str() {
        "pending" => match (row.challenge_type.as_str(), row.dns_mode.as_str()) {
            ("dns-01", "manual") => "等待到域名服务商添加 TXT 解析记录",
            ("dns-01", _) => "正在通过 DNS 服务商 API 添加解析记录",
            _ => "正在准备 HTTP 验证文件",
        },
        "processing" => "正在与 Let's Encrypt 校验并签发证书",
        "issued" => "签发完成，已保存到证书库",
        "failed" => "签发失败",
        "cancelled" => "已取消",
        "expired" => "订单已过期，请重新申请",
        _ => "处理中",
    }
    .to_string();

    OrderView {
        id: row.id,
        status: row.status.clone(),
        stage,
        cert_id: row.cert_id,
        domains: row
            .domains
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        challenge_type: row.challenge_type.clone(),
        dns_mode: row.dns_mode.clone(),
        todos: row.items(),
        error: row.error.clone(),
        expires_at: row.expires_at,
    }
}

/// 订单归属可见性：admin 全部 / reseller 自己与名下客户 / 普通用户仅自己。
pub async fn order_in_scope(claims: &jwt::Claims, order_id: i64) -> Result<AcmeOrderRow, ZapError> {
    let row = load_row(order_id).await.map_err(ZapError::Message)?;
    if jwt::is_admin(claims) || row.user_id == claims.id as i64 {
        return Ok(row);
    }
    if jwt::is_reseller(claims) {
        let (cnt,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM user WHERE id = ? AND owner_id = ?")
                .bind(row.user_id)
                .bind(claims.id as i64)
                .fetch_one(db::get_db_pool().await)
                .await?;
        if cnt > 0 {
            return Ok(row);
        }
    }
    Err(ZapError::New(
        -1,
        "无权访问该证书订单：订单归属其他用户（SSL 证书按归属用户隔离）".to_string(),
    ))
}

/// 列出可见的未完成订单（前端刷新后可接续）。
pub async fn list_orders(claims: &jwt::Claims) -> Result<Vec<OrderView>, String> {
    let pool = db::get_db_pool().await;
    // 公共条件先写死（status / 有效期），各角色只追加归属条件，避免拼接出非法 SQL
    let cols = "id, user_id, account_id, cert_id, name, remark, domains, directory, status, \
                challenge_type, dns_mode, dns_provider_id, order_url, key_pem, challenges, \
                error, expires_at";
    let base = "FROM ssl_acme_order WHERE status IN ('pending','processing') AND expires_at > ?";
    let rows: Vec<AcmeOrderRow> = if jwt::is_admin(claims) {
        sqlx::query_as(&format!("SELECT {cols} {base} ORDER BY id DESC"))
            .bind(now_secs())
            .fetch_all(pool)
            .await
    } else if jwt::is_reseller(claims) {
        sqlx::query_as(&format!(
            "SELECT {cols} {base} AND (user_id = ? OR user_id IN (SELECT id FROM user WHERE owner_id = ?)) \
             ORDER BY id DESC"
        ))
        .bind(now_secs())
        .bind(claims.id as i64)
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as(&format!(
            "SELECT {cols} {base} AND user_id = ? ORDER BY id DESC"
        ))
        .bind(now_secs())
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await
    }
    .map_err(|e| format!("查询订单失败: {e}"))?;
    Ok(rows.iter().map(view_of).collect())
}

// ── 账户 ───────────────────────────────────────────────────

/// 复用（必要时创建）该用户在该环境下的 ACME 账户。
///
/// 账户凭据是复用账户的唯一凭据，明文泄漏等于账户被人代签，因此一律加密入库。
async fn ensure_account(
    user_id: i64,
    directory: &str,
    email: &str,
) -> Result<(Account, i64), String> {
    let pool = db::get_db_pool().await;
    let exist: Option<(i64, String)> =
        sqlx::query_as("SELECT id, credentials FROM ssl_acme_account WHERE user_id = ? AND directory = ? AND email = ?")
            .bind(user_id)
            .bind(directory)
            .bind(email)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("查询 ACME 账户失败: {e}"))?;

    if let Some((id, enc)) = exist {
        let json = crypto::decrypt(&enc).map_err(|e| format!("ACME 账户凭据解密失败: {e}"))?;
        let account = account_from_json(&json).await?;
        return Ok((account, id));
    }

    let contact = format!("mailto:{email}");
    let contacts: Vec<&str> = vec![contact.as_str()];
    let new = NewAccount {
        contact: &contacts,
        terms_of_service_agreed: true,
        only_return_existing: false,
    };
    let (account, creds) = Account::builder()
        .map_err(|e| format!("初始化 ACME 客户端失败: {e}"))?
        .create(&new, directory_url(directory).to_string(), None)
        .await
        .map_err(|e| format!("创建 ACME 账户失败: {e}"))?;

    let json = serde_json::to_string(&creds).map_err(|e| format!("序列化账户凭据失败: {e}"))?;
    let enc = crypto::encrypt(&json).map_err(|e| format!("加密账户凭据失败: {e}"))?;
    let now = now_secs();
    let r = sqlx::query(
        "INSERT INTO ssl_acme_account (user_id, directory, email, credentials, account_url,
                                       created_at, updated_at)
         VALUES (?, ?, ?, ?, '', ?, ?)",
    )
    .bind(user_id)
    .bind(directory)
    .bind(email)
    .bind(&enc)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| format!("ACME 账户落库失败: {e}"))?;
    info!(email = %email, directory = %directory, "ACME 账户创建成功");
    Ok((account, r.last_insert_rowid()))
}

async fn account_from_json(json: &str) -> Result<Account, String> {
    let creds: AccountCredentials =
        serde_json::from_str(json).map_err(|e| format!("解析 ACME 账户凭据失败: {e}"))?;
    Account::builder()
        .map_err(|e| format!("初始化 ACME 客户端失败: {e}"))?
        .from_credentials(creds)
        .await
        .map_err(|e| format!("恢复 ACME 账户失败: {e}"))
}

/// 按订单所属账户恢复 `Account`（每个请求 / 后台任务各自恢复一次，避免共享连接状态）。
async fn account_for(row: &AcmeOrderRow) -> Result<Account, String> {
    let (enc,): (String,) = sqlx::query_as("SELECT credentials FROM ssl_acme_account WHERE id = ?")
        .bind(row.account_id)
        .fetch_one(db::get_db_pool().await)
        .await
        .map_err(|e| format!("读取 ACME 账户失败: {e}"))?;
    let json = crypto::decrypt(&enc).map_err(|e| format!("ACME 账户凭据解密失败: {e}"))?;
    account_from_json(&json).await
}

// ── DNS 服务商凭据 ─────────────────────────────────────────

#[derive(sqlx::FromRow, Debug, Clone, Default)]
pub struct DnsProviderRow {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub provider: String,
    pub credentials: String,
    pub remark: String,
    pub status: i64,
}

/// 载入 DNS 服务商凭据并构造实例（自动模式用）。
pub async fn load_dns_provider(
    user_id: i64,
    provider_id: i64,
) -> Result<Box<dyn dns::DnsProvider>, String> {
    let row: DnsProviderRow =
        sqlx::query_as("SELECT id, user_id, name, provider, credentials, remark, status FROM ssl_acme_dns_provider \
                        WHERE id = ? AND user_id = ? AND status = 1")
            .bind(provider_id)
            .bind(user_id)
            .fetch_optional(db::get_db_pool().await)
            .await
            .map_err(|e| format!("读取 DNS 服务商失败: {e}"))?
            .ok_or_else(|| "DNS 服务商不存在或已被删除".to_string())?;
    let json =
        crypto::decrypt(&row.credentials).map_err(|e| format!("DNS 服务商凭据解密失败: {e}"))?;
    let creds: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| format!("DNS 服务商凭据解析失败: {e}"))?;
    dns::build(&row.provider, &creds)
}

/// DNS 服务商归属可见性：与证书订单同一套隔离规则。
pub async fn dns_in_scope(claims: &jwt::Claims, id: i64) -> Result<DnsProviderRow, ZapError> {
    let row: DnsProviderRow =
        sqlx::query_as("SELECT id, user_id, name, provider, credentials, remark, status FROM ssl_acme_dns_provider WHERE id = ?")
            .bind(id)
            .fetch_optional(db::get_db_pool().await)
            .await?
            .ok_or_else(|| ZapError::New(-1, "DNS 服务商不存在".to_string()))?;
    if jwt::is_admin(claims) || row.user_id == claims.id as i64 {
        return Ok(row);
    }
    if jwt::is_reseller(claims) {
        let (cnt,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM user WHERE id = ? AND owner_id = ?")
                .bind(row.user_id)
                .bind(claims.id as i64)
                .fetch_one(db::get_db_pool().await)
                .await?;
        if cnt > 0 {
            return Ok(row);
        }
    }
    Err(ZapError::New(
        -1,
        "无权访问该 DNS 服务商：归属其他用户（SSL 证书按归属用户隔离）".to_string(),
    ))
}

// ── 订单流程 ───────────────────────────────────────────────

/// 创建订单。
///
/// - dns-01 + manual：返回待解析记录后即停在 `pending`，等用户解析完调 [`trigger`]；
/// - 其余组合：后台任务一路推进到签发，接口立即返回订单 id 供前端轮询。
pub async fn create_order(p: CreateParams) -> Result<OrderView, String> {
    if p.domains.is_empty() {
        return Err("请填写至少一个域名".to_string());
    }
    for d in &p.domains {
        if d.parse::<std::net::IpAddr>().is_ok() {
            return Err(format!("Let's Encrypt 不支持 IP 地址申请：{d}"));
        }
    }
    let wildcard = p.domains.iter().any(|d| d.starts_with("*."));
    if wildcard && p.challenge == ChallengeKind::Http01 {
        return Err(
            "泛域名证书必须用 DNS 验证，请切换为 DNS 验证（HTTP 验证不支持通配符）".to_string(),
        );
    }
    let directory = if p.staging {
        "letsencrypt-staging"
    } else {
        "letsencrypt"
    };
    let now = now_secs();

    let (account, account_id) = ensure_account(p.user_id, directory, &p.email).await?;

    let identifiers: Vec<Identifier> = p
        .domains
        .iter()
        .map(|d| Identifier::Dns(strip_wildcard(d)))
        .collect();

    let mut order = account
        .new_order(&NewOrder::new(&identifiers))
        .await
        .map_err(|e| friendly_err(le_err(e)))?;
    let order_url = order.url().to_string();
    let items = collect_challenges(&mut order, p.challenge).await?;
    if items.is_empty() {
        return Err("ACME 服务端没有返回任何域名验证信息".to_string());
    }

    let remark = if p.staging {
        format!("{}（staging）", p.remark)
    } else {
        p.remark.clone()
    };
    let r = sqlx::query(
        "INSERT INTO ssl_acme_order
            (user_id, account_id, cert_id, name, remark, domains, directory, status,
             challenge_type, dns_mode, dns_provider_id, order_url, key_pem, challenges,
             error, expires_at, created_at, updated_at)
         VALUES (?, ?, 0, ?, ?, ?, ?, 'pending', ?, ?, ?, ?, '', ?, '', ?, ?, ?)",
    )
    .bind(p.user_id)
    .bind(account_id)
    .bind(&p.name)
    .bind(&remark)
    .bind(p.domains.join(", "))
    .bind(directory)
    .bind(p.challenge.as_str())
    .bind(p.dns_mode.as_str())
    .bind(p.dns_provider_id)
    .bind(&order_url)
    .bind(serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string()))
    .bind(now + ORDER_TTL)
    .bind(now)
    .bind(now)
    .execute(db::get_db_pool().await)
    .await
    .map_err(|e| format!("订单落库失败: {e}"))?;
    let order_id = r.last_insert_rowid();
    info!(id = order_id, domains = %p.domains.join(","), kind = %p.challenge.as_str(), "ACME 订单已创建");

    // dns-01 手动：留在 pending 等用户解析；其余后台推进
    let wait_user = p.challenge == ChallengeKind::Dns01 && p.dns_mode == DnsMode::Manual;
    if !wait_user {
        tokio::spawn(process_order(order_id));
    }
    view(order_id).await
}

pub async fn view(order_id: i64) -> Result<OrderView, String> {
    Ok(view_of(&load_row(order_id).await?))
}

/// 触发验证（DNS-01 手动模式下用户点「我已经解析好了」）。
///
/// 幂等：已在处理中 / 已签发时不做任何推动，直接回当前状态。
pub async fn trigger(order_id: i64) -> Result<OrderView, String> {
    let row = load_row(order_id).await?;
    if matches!(row.status.as_str(), "processing") {
        return Ok(view_of(&row));
    }
    if row.status == "issued" {
        return Ok(view_of(&row));
    }
    if row.challenge_type == "dns-01"
        && row.dns_mode == "manual"
        && let Err(e) = refresh_propagation(&row).await
    {
        warn!(id = order_id, error = %e, "DNS 传播检测失败（不影响继续验证）");
    }
    tokio::spawn(process_order(order_id));
    view(order_id).await
}

/// 取消 / 放弃订单：清理已产生的外部副作用（验证文件、DNS TXT），不影响已签发证书。
pub async fn cancel(order_id: i64) -> Result<OrderView, String> {
    let mut row = load_row(order_id).await?;
    if row.status == "issued" {
        return Err("该订单的证书已签发，请到证书管理里删除".to_string());
    }
    if !matches!(row.status.as_str(), "pending" | "processing" | "failed") {
        return Ok(view_of(&row));
    }
    cleanup(&row).await;
    row.status = "cancelled".to_string();
    let _ = set_status(order_id, "cancelled").await;
    Ok(view_of(&row))
}

/// 后台推进：准备挑战 → 通知服务端开始校验 → finalize → 下载证书 → 入库。
async fn process_order(order_id: i64) {
    match drive_order(order_id).await {
        Ok(()) => info!(id = order_id, "ACME 订单签发完成"),
        Err(e) => {
            let msg = friendly_err(e);
            warn!(id = order_id, error = %msg, "ACME 订单签发失败");
            // 失败也要清理外部副作用，避免留下临时解析记录
            if let Ok(row) = load_row(order_id).await {
                cleanup(&row).await;
            }
            let _ = set_error(order_id, &msg).await;
        }
    }
}

async fn drive_order(order_id: i64) -> Result<(), String> {
    let row = load_row(order_id).await?;
    if !matches!(row.status.as_str(), "pending" | "processing" | "failed") {
        return Ok(());
    }
    set_status(order_id, "processing").await?;

    // 1) 准备验证素材：http-01 落盘 token 文件 / dns-01 自动建 TXT 记录
    let items = prepare_challenges(&row).await?;

    // 2) 逐个告知服务端「可以开始校验了」
    let account = account_for(&row).await?;
    let mut order = account.order(row.order_url.clone()).await.map_err(le_err)?;
    let kind = if row.challenge_type == "dns-01" {
        ChallengeKind::Dns01
    } else {
        ChallengeKind::Http01
    };
    ready_all(&mut order, kind).await?;

    let mut items = items;
    for it in items.iter_mut() {
        if it.status == "pending" {
            it.status = "validating".to_string();
        }
    }
    let _ = save_items(order_id, &items).await;

    // 3) 等服务端判定（DNS 传播慢时这里会重试到超时）
    let policy = RetryPolicy::new().timeout(POLL_TIMEOUT);
    let status = order.poll_ready(&policy).await.map_err(le_err)?;
    if status != OrderStatus::Ready {
        return Err(auth_failure(&row.order_url, order.state().status).await);
    }

    // 4) finalize：由 instant-acme 生成 CSR 与私钥
    let key_pem = order.finalize().await.map_err(le_err)?;

    // 5) 轮询下载证书链
    let chain = order.poll_certificate(&policy).await.map_err(le_err)?;

    // 6) 入库 + 清理
    let cert_id = store_certificate(&row, &chain, &key_pem).await?;
    cleanup(&row).await;
    for it in items.iter_mut() {
        it.status = "valid".to_string();
    }
    let _ = save_items(order_id, &items).await;
    let _ = mark_issued(order_id, cert_id).await;
    Ok(())
}

/// 拉取服务端返回的失败原因（ACME 把错误挂在 authorization / challenge 上）。
async fn auth_failure(order_url: &str, status: OrderStatus) -> String {
    format!(
        "域名授权未完成（order={order_url}, status={status:?}）：\
         请检查域名解析是否指向本机、HTTP 验证路径是否可被 nginx 正确处理，\
         或改用 DNS 验证"
    )
}

/// 遍历订单里的所有 authorization，取出对应类型的挑战明细。
async fn collect_challenges(
    order: &mut Order,
    kind: ChallengeKind,
) -> Result<Vec<ChallengeItem>, String> {
    let ct = kind.acme_type();
    let mut out = Vec::new();
    let mut authorizations = order.authorizations();
    while let Some(res) = authorizations.next().await {
        let mut auth = res.map_err(le_err)?;
        let domain = auth.identifier().to_string();
        let challenge = auth.challenge(ct.clone()).ok_or_else(|| {
            format!(
                "ACME 服务端未为 {domain} 提供 {} 挑战（该域名可能不支持此验证方式）",
                kind.as_str()
            )
        })?;
        let ka = challenge.key_authorization();
        let mut item = ChallengeItem {
            domain,
            status: "pending".to_string(),
            challenge_url: challenge.url.clone(),
            ..Default::default()
        };
        match kind {
            ChallengeKind::Http01 => {
                item.token = challenge.token.clone();
                item.key_auth = ka.as_str().to_string();
            }
            ChallengeKind::Dns01 => {
                let base = strip_wildcard(&item.domain);
                item.dns_host = format!("_acme-challenge.{base}");
                item.dns_value = ka.dns_value();
            }
        }
        out.push(item);
    }
    Ok(out)
}

/// 对每个 authorization 的指定挑战类型发起 `set_ready`（= 请服务端来校验）。
async fn ready_all(order: &mut Order, kind: ChallengeKind) -> Result<(), String> {
    let ct = kind.acme_type();
    let mut authorizations = order.authorizations();
    while let Some(res) = authorizations.next().await {
        let mut auth = res.map_err(le_err)?;
        // 已通过的域名不必重复校验
        if auth.status == instant_acme::AuthorizationStatus::Valid {
            continue;
        }
        let Some(mut challenge) = auth.challenge(ct.clone()) else {
            return Err(format!("缺少 {} 挑战，无法继续验证", kind.as_str()));
        };
        challenge
            .set_ready()
            .await
            .map_err(|e| friendly_err(le_err(e)))?;
    }
    Ok(())
}

/// 准备外部可见的验证素材。返回更新后的挑战明细（含 DNS 记录 id / 传播状态）。
async fn prepare_challenges(row: &AcmeOrderRow) -> Result<Vec<ChallengeItem>, String> {
    let mut items = row.items();
    match row.challenge_type.as_str() {
        "http-01" => {
            let entries: Vec<AcmeChallengeEntry> = items
                .iter()
                .filter(|i| !i.token.is_empty())
                .map(|i| AcmeChallengeEntry {
                    token: i.token.clone(),
                    key_auth: i.key_auth.clone(),
                })
                .collect();
            if entries.is_empty() {
                return Err("缺少 HTTP-01 验证材料，请重新创建订单".to_string());
            }
            // 由 zapexec 以 root 落盘到面板自管验证根，nginx 无需重载
            let rsp = zapexec::call(Request::AcmeHttpWrite { entries })
                .await
                .map_err(|e| format!("下发 HTTP 验证文件失败: {e}"))?;
            if rsp.code != 0 {
                return Err(format!("下发 HTTP 验证文件失败: {}", rsp.message));
            }
        }
        "dns-01" if row.dns_mode == "auto" => {
            let provider = load_dns_provider(row.user_id, row.dns_provider_id).await?;
            for it in items.iter_mut() {
                if it.dns_record_id.is_empty() {
                    it.dns_record_id = provider
                        .add_txt(&it.dns_host, &it.dns_value)
                        .await
                        .map_err(|e| format!("为 {} 添加 TXT 记录失败: {e}", it.domain))?;
                }
            }
            let _ = save_items(row.id, &items).await;
            // 无论手动 / 自动都要等记录传播到公共解析器，否则 LE 取不到值
            for it in items.iter_mut() {
                it.propagated = dns::wait_txt(&it.dns_host, &it.dns_value, DNS_WAIT_TIMEOUT).await;
                if !it.propagated {
                    warn!(host = %it.dns_host, "DNS TXT 未在 {}s 内传播到公共解析器", DNS_WAIT_TIMEOUT.as_secs());
                }
            }
            let _ = save_items(row.id, &items).await;
        }
        _ => {}
    }
    Ok(items)
}

/// dns-01 手动模式：刷新每条记录的传播状态给前端即时反馈。
async fn refresh_propagation(row: &AcmeOrderRow) -> Result<(), String> {
    let mut items = row.items();
    for it in items.iter_mut() {
        it.propagated = dns::wait_txt(&it.dns_host, &it.dns_value, Duration::from_secs(15)).await;
    }
    save_items(row.id, &items).await
}

/// 清理外部副作用（验证文件 / DNS TXT），幂等。
async fn cleanup(row: &AcmeOrderRow) {
    let items = row.items();
    match row.challenge_type.as_str() {
        "http-01" => {
            let tokens: Vec<String> = items
                .iter()
                .filter(|i| !i.token.is_empty())
                .map(|i| i.token.clone())
                .collect();
            if !tokens.is_empty() {
                let _ = zapexec::call(Request::AcmeHttpClear { tokens }).await;
            }
        }
        "dns-01" if row.dns_mode == "auto" => {
            if let Ok(provider) = load_dns_provider(row.user_id, row.dns_provider_id).await {
                for it in items.iter().filter(|i| !i.dns_record_id.is_empty()) {
                    let _ = provider.remove_txt(&it.dns_host, &it.dns_record_id).await;
                }
            }
        }
        _ => {}
    }
}

/// 证书入库：拆分叶子 / 中间链，并从叶子证书读出准确有效期。
async fn store_certificate(row: &AcmeOrderRow, chain: &str, key_pem: &str) -> Result<i64, String> {
    let (leaf, ca) = crate::routers::ssl::split_leaf_chain(chain);
    let (not_before, not_after) = crate::routers::ssl::parse_certificate(&leaf)
        .map(|i| (i.not_before, i.not_after))
        .unwrap_or((0, 0));
    let now = now_secs();
    let r = sqlx::query(
        "INSERT INTO ssl_cert
            (user_id, name, domains, cert_type, cert_content, key_content, ca_bundle, csr,
             not_before, not_after, status, remark, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, '', ?, ?, 1, ?, ?, ?)",
    )
    .bind(row.user_id)
    .bind(&row.name)
    .bind(&row.domains)
    .bind(&row.directory)
    .bind(&leaf)
    .bind(key_pem)
    .bind(&ca)
    .bind(not_before)
    .bind(not_after)
    .bind(&row.remark)
    .bind(now)
    .bind(now)
    .execute(db::get_db_pool().await)
    .await
    .map_err(|e| format!("证书入库失败: {e}"))?;
    Ok(r.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_stripped() {
        assert_eq!(strip_wildcard("*.example.com"), "example.com");
        assert_eq!(strip_wildcard("example.com"), "example.com");
    }

    #[test]
    fn default_validation_is_manual_dns() {
        assert_eq!(ChallengeKind::parse("dns"), ChallengeKind::Dns01);
        assert_eq!(ChallengeKind::parse("http"), ChallengeKind::Http01);
        // 未显式开启自动模式时，强制回落到手动
        assert_eq!(DnsMode::parse("auto", false), DnsMode::Manual);
        assert_eq!(DnsMode::parse("auto", true), DnsMode::Auto);
        assert_eq!(DnsMode::parse("anything", true), DnsMode::Manual);
    }
}
