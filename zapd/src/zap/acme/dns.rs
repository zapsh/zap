// SPDX-License-Identifier: AGPL-3.0-only
//! DNS-01 验证支持：DNS 服务商 API + TXT 记录传播检测。
//!
//! 分层目的很单纯：订单编排（`super`）只关心「给某个 `_acme-challenge` 记录加/删一条 TXT」，
//! 至于这条记录是 Cloudflare 还是 DNSPod 来落地，编排层不感知。
//!
//! 两种使用方式：
//! - **手动模式**：完全不走本模块对外发 requests，只由 `txt_propagated()` 做传播检测，
//!   前端把 `_acme-challenge` 主机记录与 TXT 值展示给用户自行到服务商后台添加；
//! - **自动模式**：由 [`DnsProvider`] 实现调用服务商开放 API 自动增删记录。
//!
//! 新增服务商的唯一改动点：写一个 `impl DnsProvider` 并在 [`Registry`] 里登记。

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

/// 对外 HTTP 请求超时（DNS API 一般秒回，留足 retry 空间）。
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// 全部 DNS 请求共用一个 rustls 客户端：连接池复用，且完全不依赖 OpenSSL。
fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

// ── 服务商元信息（前端凭据表单据此动态渲染）──────────────────

/// 一个凭据字段的描述。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldSpec {
    pub key: &'static str,
    pub label: &'static str,
    /// 敏感字段：读接口一律脱敏，前端渲染成密码框
    pub secret: bool,
    pub hint: &'static str,
}

/// 服务商在「新建 DNS 服务商」下拉里的展示信息。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderMeta {
    /// 存库标识
    pub kind: &'static str,
    pub label: &'static str,
    pub fields: Vec<FieldSpec>,
    /// 是否支持解析记录管理（列出域名 / 增删改记录）；不支持的服务商前端不展示入口
    pub records: bool,
}

fn field(key: &'static str, label: &'static str, secret: bool, hint: &'static str) -> FieldSpec {
    FieldSpec {
        key,
        label,
        secret,
        hint,
    }
}

/// 所有已支持的服务商（顺序即前端下拉顺序）。
pub fn providers() -> Vec<ProviderMeta> {
    vec![
        ProviderMeta {
            kind: "cloudflare",
            label: "Cloudflare",
            fields: vec![field(
                "api_token",
                "API Token",
                true,
                "控制台 → My Profile → API Tokens，需「编辑区域 DNS」权限",
            )],
            records: true,
        },
        ProviderMeta {
            kind: "dnspod",
            label: "腾讯云 DNSPod",
            fields: vec![
                field(
                    "id",
                    "SecretId（API 密钥 ID）",
                    false,
                    "DNSPod 令牌中的 ID 部分",
                ),
                field(
                    "token",
                    "SecretKey（API 令牌）",
                    true,
                    "DNSPod 令牌中的 Token 部分",
                ),
            ],
            records: true,
        },
        ProviderMeta {
            kind: "namesilo",
            label: "NameSilo",
            fields: vec![field(
                "api_key",
                "API Key",
                true,
                "NameSilo 控制台 → API Manager 生成的 API Key（需勾选 DNS 相关权限）",
            )],
            records: true,
        },
    ]
}

fn meta_of(kind: &str) -> Option<ProviderMeta> {
    providers().into_iter().find(|p| p.kind == kind)
}

// ── Provider trait ─────────────────────────────────────────

/// 账号下的一个域名（zone）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DnsDomain {
    /// 服务商侧唯一标识（Cloudflare 为 zone id，DNSPod 为域名本身）
    pub id: String,
    /// 域名（用于后续记录操作）
    pub name: String,
    /// 状态，如 active
    pub status: String,
}

/// 一条解析记录。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DnsRecord {
    pub id: String,
    /// 主机记录：Cloudflare 为完整 FQDN，DNSPod 为子域名（不含主域）
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub content: String,
    #[serde(default = "default_ttl")]
    pub ttl: u32,
    /// MX / SRV 优先级
    #[serde(default)]
    pub priority: Option<u32>,
    /// 解析线路（DNSPod）
    #[serde(default)]
    pub line: Option<String>,
}

/// 记录列表分页结果。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DnsRecordPage {
    pub records: Vec<DnsRecord>,
    pub total: u64,
    pub page: u32,
    pub pages: u32,
}

/// 新建 / 修改记录的入参。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DnsRecordInput {
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub content: String,
    #[serde(default = "default_ttl")]
    pub ttl: u32,
    #[serde(default)]
    pub priority: Option<u32>,
    #[serde(default)]
    pub line: Option<String>,
}

fn default_ttl() -> u32 {
    600
}

/// DNS 服务商的最小能力集：给某条记录加一个 TXT、删掉它、并自检连通性。
///
/// `host` 一律是完整的待验证主机名（`_acme-challenge.<域>`，已去掉通配前缀）。
///
/// 解析记录管理（列出域名 / 增删改记录）是可选能力，默认返回「不支持」，
/// 由各服务商按需实现；前端依据 [`ProviderMeta::records`] 决定是否展示入口。
#[async_trait]
pub trait DnsProvider: Send + Sync {
    /// 添加一条 TXT，返回服务商侧的记录 id（删除时用）。
    async fn add_txt(&self, host: &str, value: &str) -> Result<String, String>;
    /// 删除一条 TXT（幂等，找不到记录也返回 Ok）。
    async fn remove_txt(&self, host: &str, record_id: &str) -> Result<(), String>;
    /// 连通性 / 凭据自检，成功返回一句可读描述（如 zone 数量）。
    async fn ping(&self) -> Result<String, String>;

    /// 列出账号下的全部域名（zone）。
    async fn list_domains(&self) -> Result<Vec<DnsDomain>, String> {
        Err("该 DNS 服务商暂不支持解析记录管理".to_string())
    }
    /// 列出某域名下的解析记录（分页，per_page = 50）。
    async fn list_records(&self, _domain: &str, _page: u32) -> Result<DnsRecordPage, String> {
        Err("该 DNS 服务商暂不支持解析记录管理".to_string())
    }
    /// 新增一条记录，返回服务商侧记录 id。
    async fn create_record(&self, _domain: &str, _r: &DnsRecordInput) -> Result<String, String> {
        Err("该 DNS 服务商暂不支持解析记录管理".to_string())
    }
    /// 修改一条记录。
    async fn update_record(
        &self,
        _domain: &str,
        _id: &str,
        _r: &DnsRecordInput,
    ) -> Result<(), String> {
        Err("该 DNS 服务商暂不支持解析记录管理".to_string())
    }
    /// 删除一条记录。
    async fn delete_record(&self, _domain: &str, _id: &str) -> Result<(), String> {
        Err("该 DNS 服务商暂不支持解析记录管理".to_string())
    }
}

/// 从（服务商类型, 解密后的凭据 JSON）构造实例。
pub fn build(kind: &str, creds: &Value) -> Result<Box<dyn DnsProvider>, String> {
    if meta_of(kind).is_none() {
        return Err(format!("不支持的 DNS 服务商：{kind}"));
    }
    let get = |key: &str| -> Result<String, String> {
        let v = creds
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        v.ok_or_else(|| format!("缺少凭据字段：{key}"))
    };
    match kind {
        "cloudflare" => Ok(Box::new(Cloudflare {
            api_token: get("api_token")?,
        })),
        "dnspod" => Ok(Box::new(Dnspod {
            id: get("id")?,
            token: get("token")?,
        })),
        "namesilo" => Ok(Box::new(Namesilo {
            api_key: get("api_key")?,
        })),
        other => Err(format!("不支持的 DNS 服务商：{other}")),
    }
}

// ── TXT 传播检测（DNS-over-HTTPS，无需 UDP socket）──────────
//
// 走公共解析器而不是自己发 UDP：既避免新增 DNS 库，也天然复用了 rustls 链路。

#[derive(Debug, Deserialize)]
struct DohAnswer {
    #[serde(rename = "data")]
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DohResponse {
    #[serde(rename = "Answer")]
    answer: Option<Vec<DohAnswer>>,
}

/// 查询公共解析器里该 TXT 是否已可见。
///
/// 返回 false 只代表「还没传播到这个解析器」，不代表记录错了 —— 因此只用于提示与重试，
/// 不能作为中断流程的依据。
pub async fn txt_propagated(host: &str, value: &str) -> bool {
    for endpoint in [
        "https://dns.google/resolve",
        "https://cloudflare-dns.com/dns-query",
    ] {
        let Ok(rsp) = http()
            .get(endpoint)
            .query(&[("name", host), ("type", "TXT")])
            .send()
            .await
        else {
            continue;
        };
        let Ok(parsed) = rsp.json::<DohResponse>().await else {
            continue;
        };
        let hit = parsed
            .answer
            .unwrap_or_default()
            .iter()
            .filter_map(|a| a.data.as_deref())
            .any(|d| d.trim_matches('"') == value);
        if hit {
            return true;
        }
    }
    false
}

/// 轮询直到 TXT 可见或超时。`timeout` 为 0 时只做一次即时探测。
pub async fn wait_txt(host: &str, value: &str, timeout: Duration) -> bool {
    if txt_propagated(host, value).await {
        return true;
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if txt_propagated(host, value).await {
            return true;
        }
    }
    false
}

// ── 凭据脱敏 ───────────────────────────────────────────────

/// 按服务商字段定义把凭据脱敏成可下发的 JSON（secret 字段只保留首尾 4 位）。
pub fn mask(kind: &str, creds: &Value) -> Value {
    let Some(meta) = meta_of(kind) else {
        return json!({});
    };
    let mut out = serde_json::Map::new();
    for f in meta.fields {
        let raw = creds.get(f.key).and_then(|v| v.as_str()).unwrap_or("");
        let shown = if raw.is_empty() {
            String::new()
        } else if !f.secret || raw.len() <= 8 {
            raw.to_string()
        } else {
            let head = &raw[..4];
            let tail = &raw[raw.len() - 4..];
            format!("{head}****{tail}")
        };
        out.insert(f.key.to_string(), json!(shown));
    }
    json!(out)
}

// ── Cloudflare ─────────────────────────────────────────────

struct Cloudflare {
    api_token: String,
}

const CF_BASE: &str = "https://api.cloudflare.com/client/v4";

#[derive(Debug, Deserialize)]
struct CfZone {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct CfRecord {
    id: String,
}

#[derive(Debug, Deserialize)]
struct CfRecordDetail {
    id: String,
    name: String,
    #[serde(rename = "type")]
    r#type: String,
    content: String,
    #[serde(default)]
    ttl: u32,
    #[serde(default)]
    priority: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CfRecordList {
    result: Vec<CfRecordDetail>,
    #[serde(default)]
    result_info: Option<CfResultInfo>,
}

#[derive(Debug, Deserialize)]
struct CfResultInfo {
    total_count: i64,
}

impl Cloudflare {
    fn auth(&self) -> String {
        format!("Bearer {}", self.api_token)
    }

    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: String,
        body: Option<Value>,
    ) -> Result<T, String> {
        let mut req = http()
            .request(method, &url)
            .header(reqwest::header::AUTHORIZATION, self.auth());
        if let Some(b) = body {
            req = req.json(&b);
        }
        let rsp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        if !rsp.status().is_success() {
            let status = rsp.status();
            let text = rsp.text().await.unwrap_or_default();
            return Err(format!("Cloudflare 返回 {status}：{text}"));
        }
        rsp.json::<T>()
            .await
            .map_err(|e| format!("响应解析失败: {e}"))
    }

    /// 找到能托管该记录名的 zone（取名字最长的匹配项，支持多级后缀域名）。
    async fn zone_for(&self, host: &str) -> Result<CfZone, String> {
        let mut best: Option<CfZone> = None;
        for page in 1..=5u32 {
            let url = format!("{CF_BASE}/zones?page={page}&per_page=50&status=active");
            let body: Value = self.call(reqwest::Method::GET, url, None).await?;
            let zones: Vec<CfZone> =
                serde_json::from_value(body.get("result").cloned().unwrap_or_else(|| json!([])))
                    .map_err(|e| format!("Cloudflare zone 列表解析失败: {e}"))?;
            if zones.is_empty() {
                break;
            }
            for z in zones {
                let owned = format!(".{}", z.name.trim_end_matches('.'));
                let matched = host == z.name || host.ends_with(&owned);
                let longer = best
                    .as_ref()
                    .map(|b: &CfZone| z.name.len() > b.name.len())
                    .unwrap_or(true);
                if matched && longer {
                    best = Some(z);
                }
            }
        }
        best.ok_or_else(|| format!("Cloudflare 账号下没有能管理 {host} 的域名"))
    }
}

#[async_trait]
impl DnsProvider for Cloudflare {
    async fn add_txt(&self, host: &str, value: &str) -> Result<String, String> {
        let zone = self.zone_for(host).await?;
        let url = format!("{CF_BASE}/zones/{}/dns_records", zone.id);
        let body: Value = self
            .call(
                reqwest::Method::POST,
                url,
                Some(json!({
                    "type": "TXT",
                    "name": host,
                    "content": value,
                    "ttl": 60,
                })),
            )
            .await?;
        let rec: CfRecord =
            serde_json::from_value(body.get("result").cloned().unwrap_or_else(|| json!({})))
                .map_err(|e| format!("Cloudflare 返回体解析失败: {e}"))?;
        Ok(rec.id)
    }

    async fn remove_txt(&self, host: &str, record_id: &str) -> Result<(), String> {
        let zone = self.zone_for(host).await?;
        let url = format!("{CF_BASE}/zones/{}/dns_records/{record_id}", zone.id);
        if let Err(e) = self.call::<Value>(reqwest::Method::DELETE, url, None).await {
            // 404 视为已删除，其余错误才上报（清理失败不应掩盖签发结果）
            if !e.contains("404") {
                return Err(e);
            }
        }
        Ok(())
    }

    async fn ping(&self) -> Result<String, String> {
        let url = format!("{CF_BASE}/zones?per_page=1");
        let body: Value = self.call(reqwest::Method::GET, url, None).await?;
        let total = body
            .get("result_info")
            .and_then(|r| r.get("total_count"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        Ok(format!("连接正常，账号下共 {total} 个域名"))
    }

    async fn list_domains(&self) -> Result<Vec<DnsDomain>, String> {
        let mut out = Vec::new();
        for page in 1..=10u32 {
            let url = format!("{CF_BASE}/zones?page={page}&per_page=50&status=active");
            let body: Value = self.call(reqwest::Method::GET, url, None).await?;
            let zones: Vec<CfZone> =
                serde_json::from_value(body.get("result").cloned().unwrap_or_else(|| json!([])))
                    .map_err(|e| format!("Cloudflare zone 列表解析失败: {e}"))?;
            if zones.is_empty() {
                break;
            }
            for z in zones {
                out.push(DnsDomain {
                    id: z.id,
                    name: z.name,
                    status: "active".to_string(),
                });
            }
        }
        Ok(out)
    }

    async fn list_records(&self, domain: &str, page: u32) -> Result<DnsRecordPage, String> {
        let zone = self.zone_for(domain).await?;
        let url = format!(
            "{CF_BASE}/zones/{}/dns_records?page={}&per_page=50",
            zone.id,
            page.max(1)
        );
        let body: CfRecordList = self.call(reqwest::Method::GET, url, None).await?;
        let records: Vec<DnsRecord> = body
            .result
            .into_iter()
            .map(|r| DnsRecord {
                id: r.id,
                name: r.name,
                r#type: r.r#type,
                content: r.content,
                ttl: r.ttl,
                priority: r.priority,
                line: None,
            })
            .collect();
        let total = body
            .result_info
            .map(|i| i.total_count as u64)
            .unwrap_or(records.len() as u64);
        let pages = if total == 0 {
            1
        } else {
            total.div_ceil(50) as u32
        };
        Ok(DnsRecordPage {
            records,
            total,
            page: page.max(1),
            pages,
        })
    }

    async fn create_record(&self, domain: &str, r: &DnsRecordInput) -> Result<String, String> {
        let zone = self.zone_for(domain).await?;
        let mut body = json!({
            "type": r.r#type,
            "name": r.name,
            "content": r.content,
            "ttl": r.ttl,
        });
        if matches!(r.r#type.to_uppercase().as_str(), "MX" | "SRV")
            && let Some(p) = r.priority
        {
            body["priority"] = json!(p);
        }
        body["proxied"] = json!(false);
        let url = format!("{CF_BASE}/zones/{}/dns_records", zone.id);
        let rsp: Value = self.call(reqwest::Method::POST, url, Some(body)).await?;
        rsp.get("result")
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Cloudflare 未返回记录 id".to_string())
    }

    async fn update_record(
        &self,
        domain: &str,
        id: &str,
        r: &DnsRecordInput,
    ) -> Result<(), String> {
        let zone = self.zone_for(domain).await?;
        let mut body = json!({
            "type": r.r#type,
            "name": r.name,
            "content": r.content,
            "ttl": r.ttl,
        });
        if matches!(r.r#type.to_uppercase().as_str(), "MX" | "SRV")
            && let Some(p) = r.priority
        {
            body["priority"] = json!(p);
        }
        let url = format!("{CF_BASE}/zones/{}/dns_records/{}", zone.id, id);
        let _: Value = self.call(reqwest::Method::PATCH, url, Some(body)).await?;
        Ok(())
    }

    async fn delete_record(&self, domain: &str, id: &str) -> Result<(), String> {
        let zone = self.zone_for(domain).await?;
        let url = format!("{CF_BASE}/zones/{}/dns_records/{}", zone.id, id);
        let _: Value = self.call(reqwest::Method::DELETE, url, None).await?;
        Ok(())
    }
}

// ── 腾讯云 DNSPod ──────────────────────────────────────────

struct Dnspod {
    id: String,
    token: String,
}

const DNSPOD_BASE: &str = "https://dnsapi.cn";

#[derive(Debug, Deserialize)]
struct DnspodDomain {
    name: String,
}

#[derive(Debug, Deserialize)]
struct DnspodDomains {
    domains: Option<Vec<DnspodDomain>>,
}

/// DNSPod 旧版接口常把数值字段以字符串返回（如 `record_total: "14"`、`ttl: "600"`）。
/// 以下几个反序列化器兼容 null / number / string 三种形态，统一成数字。
fn de_flex_u32<'de, D>(d: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => Ok(n.as_u64().unwrap_or(0) as u32),
        serde_json::Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                Ok(0)
            } else {
                s.parse::<u32>().map_err(D::Error::custom)
            }
        }
        _ => Err(D::Error::custom("期望数值或字符串")),
    }
}

fn de_opt_flex_u32<'de, D>(d: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(n) => Ok(Some(n.as_u64().unwrap_or(0) as u32)),
        serde_json::Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                Ok(None)
            } else {
                s.parse::<u32>().map(Some).map_err(D::Error::custom)
            }
        }
        _ => Err(D::Error::custom("期望数值或字符串")),
    }
}

fn de_opt_flex_i64<'de, D>(d: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(n) => Ok(Some(n.as_i64().unwrap_or(0))),
        serde_json::Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                Ok(None)
            } else {
                s.parse::<i64>().map(Some).map_err(D::Error::custom)
            }
        }
        _ => Err(D::Error::custom("期望数值或字符串")),
    }
}

#[derive(Debug, Deserialize)]
struct DnspodRecordDetail {
    id: String,
    name: String,
    #[serde(rename = "type")]
    r#type: String,
    value: String,
    #[serde(default, deserialize_with = "de_flex_u32")]
    ttl: u32,
    #[serde(default)]
    line: Option<String>,
    #[serde(default, deserialize_with = "de_opt_flex_u32")]
    mx: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct DnspodRecordList {
    records: Option<Vec<DnspodRecordDetail>>,
    info: Option<DnspodRecordInfo>,
}

#[derive(Debug, Deserialize)]
struct DnspodRecordInfo {
    #[serde(default, deserialize_with = "de_opt_flex_i64")]
    record_total: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DnspodUnit {}

impl Dnspod {
    fn login_token(&self) -> String {
        format!("{},{}", self.id, self.token)
    }

    /// DNSPod API 统一走 form POST，业务错误藏在 `status.code` 里（HTTP 恒 200）。
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        action: &str,
        extra: Vec<(&str, String)>,
    ) -> Result<T, String> {
        let mut form: Vec<(&str, String)> = vec![
            ("login_token", self.login_token()),
            ("format", "json".to_string()),
            ("lang", "cn".to_string()),
        ];
        form.extend(extra);
        let rsp = http()
            .post(format!("{DNSPOD_BASE}/{action}"))
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("请求失败: {e}"))?;
        let body: Value = rsp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        let code = body
            .get("status")
            .and_then(|s| s.get("code"))
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();
        if code != "1" {
            let msg = body
                .get("status")
                .and_then(|s| s.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("未知错误");
            return Err(format!("DNSPod {action} 失败（{code}）：{msg}"));
        }
        serde_json::from_value(body).map_err(|e| format!("响应解析失败: {e}"))
    }

    /// 找出能承载该记录名的主域名，并拆出子域名部分。
    async fn split_host(&self, host: &str) -> Result<(String, String), String> {
        #[derive(Debug, Deserialize)]
        struct Domains {
            domains: Option<Vec<DnspodDomain>>,
        }
        let body: Domains = self.post("Domain.List", vec![]).await?;
        let mut best: Option<String> = None;
        for d in body.domains.unwrap_or_default() {
            let owned = format!(".{}", d.name.trim_end_matches('.'));
            if (host == d.name || host.ends_with(&owned))
                && best
                    .as_ref()
                    .map(|b| d.name.len() > b.len())
                    .unwrap_or(true)
            {
                best = Some(d.name);
            }
        }
        let domain = best.ok_or_else(|| format!("DNSPod 账号下没有能管理 {host} 的域名"))?;
        let sub = host
            .strip_suffix(&domain)
            .unwrap_or(host)
            .trim_end_matches('.')
            .to_string();
        Ok((domain, sub))
    }
}

#[async_trait]
impl DnsProvider for Dnspod {
    async fn add_txt(&self, host: &str, value: &str) -> Result<String, String> {
        let (domain, sub) = self.split_host(host).await?;
        #[derive(Debug, Deserialize)]
        struct Created {
            record: Option<CfRecord>,
        }
        let body: Created = self
            .post(
                "Record.Create",
                vec![
                    ("domain", domain),
                    ("sub_domain", sub),
                    ("record_type", "TXT".to_string()),
                    ("record_line", "默认".to_string()),
                    ("value", value.to_string()),
                    ("ttl", "60".to_string()),
                ],
            )
            .await?;
        body.record
            .map(|r| r.id)
            .ok_or_else(|| "DNSPod 未返回记录 id".to_string())
    }

    async fn remove_txt(&self, host: &str, record_id: &str) -> Result<(), String> {
        let (domain, _) = self.split_host(host).await?;
        #[derive(Debug, Deserialize)]
        struct Unit {}
        let _: Unit = self
            .post(
                "Record.Remove",
                vec![("domain", domain), ("record", record_id.to_string())],
            )
            .await?;
        Ok(())
    }

    async fn ping(&self) -> Result<String, String> {
        #[derive(Debug, Deserialize)]
        struct Domains {
            domains: Option<Vec<DnspodDomain>>,
        }
        let body: Domains = self.post("Domain.List", vec![]).await?;
        let n = body.domains.map(|d| d.len()).unwrap_or(0);
        Ok(format!("连接正常，账号下共 {n} 个域名"))
    }

    async fn list_domains(&self) -> Result<Vec<DnsDomain>, String> {
        let body: DnspodDomains = self.post("Domain.List", vec![]).await?;
        Ok(body
            .domains
            .unwrap_or_default()
            .into_iter()
            .map(|d| DnsDomain {
                id: d.name.clone(),
                name: d.name,
                status: "active".to_string(),
            })
            .collect())
    }

    async fn list_records(&self, domain: &str, page: u32) -> Result<DnsRecordPage, String> {
        let (domain_name, _sub) = self.split_host(domain).await?;
        let offset = (page.max(1) - 1) * 50;
        let body: DnspodRecordList = self
            .post(
                "Record.List",
                vec![
                    ("domain", domain_name),
                    ("offset", offset.to_string()),
                    ("length", "50".to_string()),
                ],
            )
            .await?;
        let records: Vec<DnsRecord> = body
            .records
            .unwrap_or_default()
            .into_iter()
            .map(|r| DnsRecord {
                id: r.id,
                name: r.name,
                r#type: r.r#type,
                content: r.value,
                ttl: r.ttl,
                priority: r.mx,
                line: r.line,
            })
            .collect();
        let total = body
            .info
            .and_then(|i| i.record_total)
            .unwrap_or(records.len() as i64) as u64;
        let pages = if total == 0 {
            1
        } else {
            total.div_ceil(50) as u32
        };
        Ok(DnsRecordPage {
            records,
            total,
            page: page.max(1),
            pages,
        })
    }

    async fn create_record(&self, domain: &str, r: &DnsRecordInput) -> Result<String, String> {
        let (domain_name, _sub) = self.split_host(domain).await?;
        let mut extra: Vec<(&str, String)> = vec![
            ("domain", domain_name),
            ("sub_domain", r.name.clone()),
            ("record_type", r.r#type.clone()),
            ("value", r.content.clone()),
            ("ttl", r.ttl.to_string()),
        ];
        // 线路：未填时按 DNSPod 默认线路（"默认"），空字符串会让 Record.Create /
        // Record.Modify 报「记录线路不正确」（错误码 26）。与 add_txt 保持一致。
        let line = r
            .line
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "默认".to_string());
        extra.push(("record_line", line));
        if matches!(r.r#type.to_uppercase().as_str(), "MX")
            && let Some(p) = r.priority
        {
            extra.push(("mx", p.to_string()));
        }
        #[derive(Debug, Deserialize)]
        struct Created {
            record: Option<DnspodRecordId>,
        }
        #[derive(Debug, Deserialize)]
        struct DnspodRecordId {
            id: String,
        }
        let body: Created = self.post("Record.Create", extra).await?;
        body.record
            .map(|x| x.id)
            .ok_or_else(|| "DNSPod 未返回记录 id".to_string())
    }

    async fn update_record(
        &self,
        domain: &str,
        id: &str,
        r: &DnsRecordInput,
    ) -> Result<(), String> {
        let (domain_name, _sub) = self.split_host(domain).await?;
        let mut extra: Vec<(&str, String)> = vec![
            ("domain", domain_name),
            ("record_id", id.to_string()),
            ("sub_domain", r.name.clone()),
            ("record_type", r.r#type.clone()),
            ("value", r.content.clone()),
            ("ttl", r.ttl.to_string()),
        ];
        // 线路：未填时按 DNSPod 默认线路（"默认"），空字符串会让 Record.Create /
        // Record.Modify 报「记录线路不正确」（错误码 26）。与 add_txt 保持一致。
        let line = r
            .line
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "默认".to_string());
        extra.push(("record_line", line));
        if matches!(r.r#type.to_uppercase().as_str(), "MX")
            && let Some(p) = r.priority
        {
            extra.push(("mx", p.to_string()));
        }
        let _: DnspodUnit = self.post("Record.Modify", extra).await?;
        Ok(())
    }

    async fn delete_record(&self, domain: &str, id: &str) -> Result<(), String> {
        let (domain_name, _sub) = self.split_host(domain).await?;
        let _: DnspodUnit = self
            .post(
                "Record.Remove",
                vec![("domain", domain_name), ("record_id", id.to_string())],
            )
            .await?;
        Ok(())
    }
}

// ── NameSilo ────────────────────────────────────────────────
//
// API 特点（与 Cloudflare / DNSPod 不同之处）：
// - 成功码是字符串 "300"（不是 1），`code != "300"` 即失败；
// - 需带 `version=1&type=json&key=<APIKEY>` 走 query，返回 JSON；
// - 记录操作的 `host` 是「子域名标签」（`@` 表示主域），不是完整 FQDN；
//   `domain` 必须是 NameSilo 注册的基域名；
// - TTL 取 NameSilo 允许集合内的值（含 60/300/600/3600 等）。
const NS_BASE: &str = "https://www.namesilo.com/api";

struct Namesilo {
    api_key: String,
}

#[derive(Debug, Deserialize)]
struct NamesiloDomains {
    reply: NamesiloDomainReply,
}

#[derive(Debug, Deserialize)]
struct NamesiloDomainReply {
    domains: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct NamesiloAddReply {
    reply: NamesiloAddReplyInner,
}

#[derive(Debug, Deserialize)]
struct NamesiloAddReplyInner {
    record_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NamesiloUnit {}

#[derive(Debug, Deserialize)]
struct NamesiloRecords {
    reply: NamesiloRecordsReply,
}

#[derive(Debug, Deserialize)]
struct NamesiloRecordsReply {
    resource_record: Option<Vec<NamesiloRecord>>,
}

#[derive(Debug, Deserialize)]
struct NamesiloRecord {
    record_id: String,
    #[serde(rename = "type")]
    r#type: String,
    /// 子域名标签：`@` 表示主域，其余为 www / _acme-challenge 等
    host: String,
    value: String,
    #[serde(deserialize_with = "de_flex_u32")]
    ttl: u32,
    /// MX / SRV 优先级（NameSilo 叫 distance）
    #[serde(default, deserialize_with = "de_opt_flex_u32")]
    distance: Option<u32>,
}

impl Namesilo {
    /// 把前端传入的完整主机名收敛成 NameSilo 的「子域名标签」。
    /// `www.example.com` → `www`；`example.com` / `@` → `@`。
    fn host_label(full: &str, base: &str) -> String {
        let owned = format!(".{}", base.trim_end_matches('.'));
        let label = full
            .strip_suffix(&owned)
            .unwrap_or(full)
            .trim_end_matches('.');
        if label.is_empty() {
            "@".to_string()
        } else {
            label.to_string()
        }
    }

    /// 找出能承载该主机名的注册基域名（取名字最长的匹配项）。
    async fn split_host(&self, host: &str) -> Result<(String, String), String> {
        let body: NamesiloDomains = self.call("domainList", vec![]).await?;
        let mut best: Option<String> = None;
        for d in body.reply.domains.unwrap_or_default() {
            let owned = format!(".{}", d.trim_end_matches('.'));
            if (host == d || host.ends_with(&owned))
                && best
                    .as_ref()
                    .map(|b| d.len() > b.len())
                    .unwrap_or(true)
            {
                best = Some(d);
            }
        }
        let domain = best.ok_or_else(|| format!("NameSilo 账号下没有能管理 {host} 的域名"))?;
        let sub = host
            .strip_suffix(&domain)
            .unwrap_or(host)
            .trim_end_matches('.')
            .to_string();
        Ok((domain, sub))
    }

    /// 统一发 NameSilo 请求：GET + query，校验成功码 "300"，解析 reply。
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        action: &str,
        params: Vec<(&str, String)>,
    ) -> Result<T, String> {
        let mut q: Vec<(String, String)> = vec![
            ("version".to_string(), "1".to_string()),
            ("type".to_string(), "json".to_string()),
            ("key".to_string(), self.api_key.clone()),
        ];
        for (k, v) in params {
            q.push((k.to_string(), v));
        }
        let rsp = http()
            .get(NS_BASE)
            .query(&q)
            .send()
            .await
            .map_err(|e| format!("请求失败: {e}"))?;
        let body: Value = rsp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        let code = body
            .get("code")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();
        if code != "300" {
            let msg = body
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("未知错误")
                .to_string();
            return Err(format!("NameSilo {action} 失败（{code}）：{msg}"));
        }
        serde_json::from_value(body).map_err(|e| format!("响应解析失败: {e}"))
    }
}

#[async_trait]
impl DnsProvider for Namesilo {
    async fn add_txt(&self, host: &str, value: &str) -> Result<String, String> {
        let (domain, sub) = self.split_host(host).await?;
        let body: NamesiloAddReply = self
            .call(
                "dnsAddRecord",
                vec![
                    ("domain", domain),
                    ("rrtype", "TXT".to_string()),
                    ("rrhost", sub),
                    ("rrvalue", value.to_string()),
                    // NameSilo 最小 TTL 集合含 60/300/600/3600，3600 最稳
                    ("rrttl", "3600".to_string()),
                ],
            )
            .await?;
        body.reply
            .record_id
            .ok_or_else(|| "NameSilo 未返回记录 id".to_string())
    }

    async fn remove_txt(&self, host: &str, record_id: &str) -> Result<(), String> {
        let (domain, _sub) = self.split_host(host).await?;
        let _: NamesiloUnit = self
            .call(
                "dnsDeleteRecord",
                vec![("domain", domain), ("rrid", record_id.to_string())],
            )
            .await?;
        Ok(())
    }

    async fn ping(&self) -> Result<String, String> {
        let body: NamesiloDomains = self.call("domainList", vec![]).await?;
        let n = body.reply.domains.map(|d| d.len()).unwrap_or(0);
        Ok(format!("连接正常，账号下共 {n} 个域名"))
    }

    async fn list_domains(&self) -> Result<Vec<DnsDomain>, String> {
        let body: NamesiloDomains = self.call("domainList", vec![]).await?;
        Ok(body
            .reply
            .domains
            .unwrap_or_default()
            .into_iter()
            .map(|d| DnsDomain {
                id: d.clone(),
                name: d,
                status: "active".to_string(),
            })
            .collect())
    }

    async fn list_records(&self, domain: &str, page: u32) -> Result<DnsRecordPage, String> {
        let (domain_name, _sub) = self.split_host(domain).await?;
        let body: NamesiloRecords = self
            .call("dnsListRecords", vec![("domain", domain_name.clone())])
            .await?;
        let all: Vec<DnsRecord> = body
            .reply
            .resource_record
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                // 还原成完整 FQDN 便于展示；主域用 `@` 时即基域名本身
                let name = if r.host == "@" {
                    domain_name.clone()
                } else {
                    format!("{}.{}", r.host, domain_name)
                };
                DnsRecord {
                    id: r.record_id,
                    name,
                    r#type: r.r#type,
                    content: r.value,
                    ttl: r.ttl,
                    priority: r.distance,
                    line: None,
                }
            })
            .collect();
        let total = all.len() as u64;
        let per: usize = 50;
        let pages = if total == 0 {
            1
        } else {
            total.div_ceil(per as u64) as u32
        };
        let p = page.max(1) as usize;
        let start = ((p - 1) * per).min(all.len());
        let end = (start + per).min(all.len());
        Ok(DnsRecordPage {
            records: if start < all.len() {
                all[start..end].to_vec()
            } else {
                vec![]
            },
            total,
            page: p as u32,
            pages,
        })
    }

    async fn create_record(&self, domain: &str, r: &DnsRecordInput) -> Result<String, String> {
        let (domain_name, _sub) = self.split_host(domain).await?;
        let host = Self::host_label(&r.name, &domain_name);
        let body: NamesiloAddReply = self
            .call(
                "dnsAddRecord",
                vec![
                    ("domain", domain_name),
                    ("rrtype", r.r#type.clone()),
                    ("rrhost", host),
                    ("rrvalue", r.content.clone()),
                    ("rrttl", r.ttl.to_string()),
                ],
            )
            .await?;
        body.reply
            .record_id
            .ok_or_else(|| "NameSilo 未返回记录 id".to_string())
    }

    async fn update_record(
        &self,
        domain: &str,
        id: &str,
        r: &DnsRecordInput,
    ) -> Result<(), String> {
        let (domain_name, _sub) = self.split_host(domain).await?;
        let host = Self::host_label(&r.name, &domain_name);
        let _: NamesiloUnit = self
            .call(
                "dnsUpdateRecord",
                vec![
                    ("domain", domain_name),
                    ("rrid", id.to_string()),
                    ("rrtype", r.r#type.clone()),
                    ("rrhost", host),
                    ("rrvalue", r.content.clone()),
                    ("rrttl", r.ttl.to_string()),
                ],
            )
            .await?;
        Ok(())
    }

    async fn delete_record(&self, domain: &str, id: &str) -> Result<(), String> {
        let (domain_name, _sub) = self.split_host(domain).await?;
        let _: NamesiloUnit = self
            .call(
                "dnsDeleteRecord",
                vec![("domain", domain_name), ("rrid", id.to_string())],
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 脱敏：secret 字段只留首尾，非 secret 字段原样返回。
    #[test]
    fn mask_keeps_head_tail() {
        let creds = json!({ "api_token": "abcdefghijklmnop" });
        let m = mask("cloudflare", &creds);
        assert_eq!(m["api_token"], json!("abcd****mnop"));

        let dnspod = json!({ "id": "123456", "token": "abcdefghijklmnop" });
        let m = mask("dnspod", &dnspod);
        assert_eq!(m["id"], json!("123456"));
        assert_eq!(m["token"], json!("abcd****mnop"));

        let ns = json!({ "api_key": "abcdefghijklmnop" });
        let m = mask("namesilo", &ns);
        assert_eq!(m["api_key"], json!("abcd****mnop"));
    }

    /// 未知服务商不该 panic，返回空对象即可（页面降级显示）。
    #[test]
    fn mask_unknown_provider() {
        assert_eq!(mask("nope", &json!({"k": "v"})), json!({}));
        assert!(build("nope", &json!({})).is_err());
    }
}
