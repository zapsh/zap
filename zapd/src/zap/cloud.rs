//! 云存储（对象存储）管理：多存储配置 + 凭据加密落盘 + opendal 统一读写。
//!
//! ## 目录布局（按用户隔离，与 `crontab.yaml` 同级）
//!
//! ```text
//! {ZAP_PATH}/data/users/<user>/
//! ├── crontab.yaml
//! └── cloud/
//!     └── stores/
//!         └── <id>.json        # 一个云存储一个文件（0600）
//! ```
//!
//! 为什么「一个存储一个文件」而不是单文件存数组：并发写不同存储互不干扰、
//! 单条损坏不影响其它、删除即删文件；将来按存储挂同步任务/日志时也只需在
//! `cloud/` 下平级加目录（如 `cloud/logs/<store-id>/`）。
//!
//! ## 凭据加密
//!
//! `access_key_id` / `secret_access_key` / `security_token` 序列化后用
//! `zap-crypto`（AES-256-GCM，机器主密钥 `/etc/zap/secret.key`）加密，以
//! `v1:<nonce>:<ct>` 形式存进 `secret` 字段；endpoint/bucket 等元数据保持明文，
//! 便于排障与迁移。另外只存一份脱敏提示（`access_key_hint`）供列表展示，
//! 因此**列表接口无需解密**。
//!
//! ## 统一访问层
//!
//! 四种服务（AWS S3 / 阿里云 OSS / 腾讯云 COS / S3 兼容）统一走 opendal
//! [`Operator`]，`root` 作为桶内逻辑根：所有对外路径都是**相对逻辑根**的路径
//! （如 `docs/a.txt`），前端无需感知 bucket 与 root 的拼接。

use std::error::Error as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::TryStreamExt;
use opendal::{Operator, services};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::zap::ZapError;
use crate::zap::crypto::{decrypt, encrypt};
use crate::zap::user_cron;

/// 配置文件格式版本（将来结构变更时据此迁移）
const STORE_VERSION: u32 = 1;
/// 单次列目录最多返回的条目数（超大目录保护面板与浏览器）。
///
/// 只在前端侧生效：靠提前停止翻页实现，**不会**作为 `max-keys` 发给对象存储
/// （COS / OSS 的 `max-keys` 上限是 1000，见 [`list`] 里的说明）。
const LIST_LIMIT: usize = 2000;
/// 连通性测试超时
const TEST_TIMEOUT: Duration = Duration::from_secs(15);
/// 连通性测试最多读取的条目数（能读到就说明通，不必翻完整个桶）
const TEST_ENTRIES: usize = 5;

fn fail(msg: impl Into<String>) -> ZapError {
    ZapError::New(-1, msg.into())
}

/// opendal 错误 → 面板错误（带操作上下文，前端直接展示）。
///
/// 会补齐错误链：opendal 的 `Display` 只渲染一级 `source`，而 reqwest 的 `Display`
/// 又只有一句 `error sending request for url (...)`，真正的原因（DNS 解析失败 /
/// 证书不受信 / 连接被拒 / 超时）还埋在更内层，不挖出来就只能靠猜。
fn oe(ctx: &str, err: opendal::Error) -> ZapError {
    ZapError::New(-1, format!("{ctx}失败：{err}{}", cause_chain(&err)))
}

/// 把错误链补齐成 `；原因：a → b → c`，必要时再附一句代理提醒。
fn cause_chain(err: &opendal::Error) -> String {
    // opendal 自己已经打印了一级 source，用它去重，避免同一句出现两遍
    let mut seen = err.to_string();
    let mut causes: Vec<String> = Vec::new();

    let mut cur = err.source();
    while let Some(e) = cur {
        let text = e.to_string();
        if !text.is_empty() && !seen.contains(&text) && !causes.contains(&text) {
            seen.push_str(&text);
            causes.push(text);
        }
        cur = e.source();
    }

    let mut out = String::new();
    if !causes.is_empty() {
        out.push_str("；原因：");
        out.push_str(&causes.join(" → "));
    }
    // 只在「请求根本没发出去」这类失败后面提代理，正常的业务报错（403/404…）不提
    if is_network_failure(&seen) {
        out.push_str(&proxy_hint());
    }
    out
}

/// 是否属于「请求没能发出去」这一类失败。
fn is_network_failure(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    // 不要用裸词 `connect`：正常响应的 headers 里就有 `connection: keep-alive`,
    // 会把「服务端明明答了话」的业务错误也误判成网络故障。
    [
        "reqwest::send",
        "reqwest::fetch",
        "connect error",
        "unreachable",
        "dns error",
        "timed out",
        "handshake",
        "certificate",
    ]
    .iter()
    .any(|key| text.contains(key))
}

/// 进程里存在 HTTP(S)_PROXY 时的提醒。
///
/// reqwest 默认会读这些环境变量，代理不通就表现成 `error sending request`。
/// WSL 里从 Windows 继承来的 `127.0.0.1:1080` 代理是典型死因：WSL2 里那个地址
/// 指向 Linux 自己，代理软件其实跑在 Windows 侧，连接必然被拒。
fn proxy_hint() -> String {
    for name in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(value) = std::env::var(name) {
            let value = value.trim();
            if !value.is_empty() {
                return format!(
                    "；提示：进程环境里设置了 {name}={value}，请求会经该代理发出；\
                     若代理不可用请 unset 该变量，或把目标域名加入 NO_PROXY"
                );
            }
        }
    }
    String::new()
}

// ── 服务类型 ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudService {
    /// 亚马逊 S3
    AwsS3,
    /// 阿里云对象存储 OSS
    Oss,
    /// 腾讯云对象存储 COS
    Cos,
    /// 兼容 S3 协议的第三方存储（MinIO / Ceph / 自建网关…）
    S3Compat,
}

impl CloudService {
    pub fn parse(raw: &str) -> Result<Self, ZapError> {
        Ok(match raw.trim() {
            "aws_s3" => Self::AwsS3,
            "oss" => Self::Oss,
            "cos" => Self::Cos,
            "s3_compat" => Self::S3Compat,
            other => return Err(fail(format!("不支持的云存储类型：{other}"))),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AwsS3 => "aws_s3",
            Self::Oss => "oss",
            Self::Cos => "cos",
            Self::S3Compat => "s3_compat",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AwsS3 => "AWS S3",
            Self::Oss => "阿里云 OSS",
            Self::Cos => "腾讯云 COS",
            Self::S3Compat => "S3 兼容存储",
        }
    }

    /// endpoint 是否必填（AWS S3 可省略，由 region 推导官方地址）
    fn endpoint_required(self) -> bool {
        !matches!(self, Self::AwsS3)
    }
}

// ── 数据模型 ────────────────────────────────────────────────

/// 云存储凭据（仅存在于内存，落盘前加密）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    /// 临时凭据（STS 的 security token），没有则为空
    #[serde(default)]
    pub security_token: String,
}

/// 落盘结构（`stores/<id>.json`）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudStore {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub service: CloudService,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub region: String,
    pub bucket: String,
    /// 桶内逻辑根目录（空 = 桶根）
    #[serde(default)]
    pub root: String,
    /// 虚拟主机样式（`bucket.endpoint`）；false = path 样式（MinIO 等自建常用）
    #[serde(default)]
    pub virtual_host_style: bool,
    /// 加密后的凭据（`v1:<nonce>:<ct>`）
    pub secret: String,
    /// access key 脱敏提示（如 `LTAI****cdef`），仅用于展示
    #[serde(default)]
    pub access_key_hint: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl CloudStore {
    /// 解密凭据（明文只在内存中短暂存在）。
    pub fn credentials(&self) -> Result<Credentials, ZapError> {
        let plain = decrypt(&self.secret).map_err(|e| fail(format!("凭据解密失败：{e}")))?;
        if plain.is_empty() {
            return Err(fail("凭据为空，请重新保存该云存储的密钥"));
        }
        serde_json::from_str(&plain).map_err(|e| fail(format!("凭据解析失败：{e}")))
    }

    /// 对外视图：不含密文，凭据只给脱敏提示。
    pub fn view(&self) -> StoreView {
        StoreView {
            id: self.id.clone(),
            name: self.name.clone(),
            service: self.service.as_str(),
            service_label: self.service.label(),
            endpoint: self.endpoint.clone(),
            region: self.region.clone(),
            bucket: self.bucket.clone(),
            root: self.root.clone(),
            virtual_host_style: self.virtual_host_style,
            access_key_hint: self.access_key_hint.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreView {
    pub id: String,
    pub name: String,
    pub service: &'static str,
    pub service_label: &'static str,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub root: String,
    pub virtual_host_style: bool,
    pub access_key_hint: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 新增 / 编辑请求体
#[derive(Debug, Deserialize)]
pub struct StoreInput {
    /// 空 = 新建，非空 = 编辑
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub service: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub region: String,
    pub bucket: String,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub virtual_host_style: bool,
    /// 编辑时留空 = 沿用原凭据
    #[serde(default)]
    pub access_key_id: String,
    #[serde(default)]
    pub secret_access_key: String,
    #[serde(default)]
    pub security_token: String,
}

// ── 路径与落盘 ──────────────────────────────────────────────

/// `{ZAP_PATH}/data/users/<user>/cloud`
///
/// 与 `crontab.yaml` 同级：用户的「个人数据」都收在 `users/<user>/` 下，
/// 备份/迁移时整目录打包即可。
pub fn user_cloud_dir(username: &str) -> PathBuf {
    user_cron::users_dir().join(username).join("cloud")
}

/// `.../cloud/stores`：云存储配置目录
pub fn stores_dir(username: &str) -> PathBuf {
    user_cloud_dir(username).join("stores")
}

fn store_file(username: &str, id: &str) -> PathBuf {
    stores_dir(username).join(format!("{id}.json"))
}

/// 校验用户名 + 云存储 ID，返回规范化 ID。
fn safe_store_id(id: &str) -> Result<String, ZapError> {
    let id = id.trim();
    // ID 由服务端生成：16 位十六进制
    if id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(id.to_ascii_lowercase())
    } else {
        Err(fail("云存储 ID 不合法"))
    }
}

fn new_store_id() -> Result<String, ZapError> {
    let mut buf = [0u8; 8];
    getrandom::getrandom(&mut buf).map_err(|e| fail(format!("生成云存储 ID 失败：{e}")))?;
    Ok(hex::encode(buf))
}

/// 目录权限收紧到 0700（配置里含密文与桶信息，同机其他用户不应可见）。
fn tighten_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(err) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)) {
            warn!("设置目录权限失败 {}: {err}", path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// 文件权限收紧到 0600。
fn tighten_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(err) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            warn!("设置文件权限失败 {}: {err}", path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn read_store_file(path: &Path) -> Result<CloudStore, ZapError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| fail(format!("读取云存储配置失败 {}：{e}", path.display())))?;
    serde_json::from_str(&content).map_err(|e| fail(format!("云存储配置解析失败：{e}")))
}

/// 原子写：先写临时文件再 rename，避免中断留下半截 JSON。
fn write_store_file(username: &str, store: &CloudStore) -> Result<(), ZapError> {
    let dir = stores_dir(username);
    std::fs::create_dir_all(&dir)
        .map_err(|e| fail(format!("创建配置目录失败 {}：{e}", dir.display())))?;
    tighten_dir(&dir);
    if let Some(parent) = dir.parent() {
        tighten_dir(parent);
    }

    let path = dir.join(format!("{}.json", store.id));
    let tmp = dir.join(format!("{}.json.tmp", store.id));
    let content =
        serde_json::to_vec_pretty(store).map_err(|e| fail(format!("云存储配置序列化失败：{e}")))?;
    std::fs::write(&tmp, &content).map_err(|e| fail(format!("保存云存储配置失败：{e}")))?;
    tighten_file(&tmp);
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(format!("保存云存储配置失败：{e}"))
    })?;
    tighten_file(&path);
    Ok(())
}

// ── 配置 CRUD ───────────────────────────────────────────────

pub fn list_stores(username: &str) -> Result<Vec<StoreView>, ZapError> {
    user_cron::safe_username(username)?;
    let dir = stores_dir(username);
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        // 目录不存在 = 该用户还没配置过云存储
        Err(_) => return Ok(Vec::new()),
    };

    let mut out: Vec<StoreView> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match read_store_file(&path) {
            Ok(store) => out.push(store.view()),
            // 单条损坏不影响其余（比如手工改坏了文件）
            Err(err) => warn!("跳过无法解析的云存储配置 {}: {err}", path.display()),
        }
    }
    out.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

/// 读取某个云存储（含密文，仅内部使用）。
pub fn get_store(username: &str, id: &str) -> Result<CloudStore, ZapError> {
    user_cron::safe_username(username)?;
    let id = safe_store_id(id)?;
    let path = store_file(username, &id);
    if !path.exists() {
        return Err(fail("云存储不存在"));
    }
    read_store_file(&path)
}

pub fn save_store(username: &str, input: StoreInput) -> Result<StoreView, ZapError> {
    user_cron::safe_username(username)?;

    let name = input.name.trim();
    if name.is_empty() {
        return Err(fail("请填写云存储名称"));
    }
    if name.chars().count() > 40 {
        return Err(fail("云存储名称最多 40 个字符"));
    }

    let service = CloudService::parse(&input.service)?;

    let bucket = input.bucket.trim();
    if bucket.is_empty() {
        return Err(fail("请填写 Bucket 名称"));
    }

    let endpoint = normalize_endpoint(&input.endpoint);
    if endpoint.is_empty() && service.endpoint_required() {
        return Err(fail(format!(
            "{} 需要填写 Endpoint（如 oss-cn-hangzhou.aliyuncs.com）",
            service.label()
        )));
    }

    let region = input.region.trim().to_string();
    if matches!(service, CloudService::AwsS3) && region.is_empty() {
        return Err(fail("AWS S3 需要填写 Region（如 us-east-1）"));
    }

    let root = normalize_root(&input.root);

    // 编辑时保留原创建时间与（未修改的）凭据
    let existing = if input.id.trim().is_empty() {
        None
    } else {
        Some(get_store(username, input.id.trim())?)
    };

    let (secret, hint) = resolve_secret(&input, existing.as_ref())?;

    let now = chrono::Local::now().timestamp();
    let store = CloudStore {
        version: STORE_VERSION,
        id: existing
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or(new_store_id()?),
        name: name.to_string(),
        service,
        endpoint,
        region,
        bucket: bucket.to_string(),
        root,
        virtual_host_style: input.virtual_host_style,
        secret,
        access_key_hint: hint,
        created_at: existing.as_ref().map(|s| s.created_at).unwrap_or(now),
        updated_at: now,
    };
    write_store_file(username, &store)?;
    Ok(store.view())
}

/// 计算要落盘的密文与脱敏提示：请求里给了密钥就重新加密，否则沿用原密文。
fn resolve_secret(
    input: &StoreInput,
    existing: Option<&CloudStore>,
) -> Result<(String, String), ZapError> {
    let ak = input.access_key_id.trim();
    let sk = input.secret_access_key.trim();

    if ak.is_empty() && sk.is_empty() {
        return match existing {
            Some(store) => Ok((store.secret.clone(), store.access_key_hint.clone())),
            None => Err(fail("请填写 AccessKey ID 与 AccessKey Secret")),
        };
    }
    if ak.is_empty() || sk.is_empty() {
        return Err(fail("AccessKey ID 与 AccessKey Secret 需要同时填写"));
    }

    let cred = Credentials {
        access_key_id: ak.to_string(),
        secret_access_key: sk.to_string(),
        security_token: input.security_token.trim().to_string(),
    };
    let plain = serde_json::to_string(&cred).map_err(|e| fail(format!("凭据序列化失败：{e}")))?;
    let secret = encrypt(&plain).map_err(|e| fail(format!("凭据加密失败：{e}")))?;
    Ok((secret, mask_key(ak)))
}

/// 删除云存储配置，返回被删除的配置（供审计记录名称）。
pub fn delete_store(username: &str, id: &str) -> Result<StoreView, ZapError> {
    let store = get_store(username, id)?;
    let path = store_file(username, &store.id);
    std::fs::remove_file(&path).map_err(|e| fail(format!("删除云存储配置失败：{e}")))?;
    Ok(store.view())
}

/// access key 脱敏：保留前 4 后 4，中间打星（短 key 全打星）。
fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len().max(4));
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}****{tail}")
}

/// endpoint 归一化：补全协议头、去掉尾部 `/`。
fn normalize_endpoint(raw: &str) -> String {
    let mut value = raw.trim().trim_end_matches('/').to_string();
    if value.is_empty() {
        return value;
    }
    if !value.contains("://") {
        // 云厂商 endpoint 一律走 HTTPS
        value = format!("https://{value}");
    }
    value
}

/// 桶内根目录归一化：去首尾 `/`，丢掉 `.` 与 `..`。
fn normalize_root(raw: &str) -> String {
    raw.split('/')
        .filter(|seg| !seg.is_empty() && *seg != "." && *seg != "..")
        .collect::<Vec<_>>()
        .join("/")
}

// ── opendal 接入 ────────────────────────────────────────────

/// 安装 opendal 的进程级默认 HTTP 传输层（幂等，可重复调用）。
///
/// opendal 0.59 起，S3/OSS/COS 这类 HTTP 服务不再自带传输层，而是统一取一个全局默认
/// 传输层。facade 只在启用 `auto-register-services` 时用 `ctor` 在 `main` 之前自动安装，
/// 而我们为了瘦身关掉了 `default-features`（也就不带这个 feature），结果所有请求都会以
///
/// ```text
/// ConfigInvalid: default HTTP transport is not installed
/// advice: call HttpTransporter::install_default before using HTTP services
/// ```
///
/// 失败——配好了 COS 却一个文件都读不到，正是这个原因。
///
/// 这里不打开 `auto-register-services`（那是给 URI 方式构造 Operator 用的全局服务注册表，
/// 我们用不上），只显式安装默认传输层：`install_default` 内部幂等（注册表用 `Once`、
/// 传输层是 first-installed-wins），再用一个 `Once` 兜底，保证任何入口都不会漏装。
fn ensure_http_transport() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(opendal::install_default);
}

/// 构造 opendal `Operator`（每次操作瞬时构造，配置改动立即生效）。
pub fn build_operator(store: &CloudStore) -> Result<Operator, ZapError> {
    ensure_http_transport();

    let cred = store.credentials()?;
    let root = if store.root.is_empty() {
        "/".to_string()
    } else {
        format!("/{}/", store.root)
    };

    let op = match store.service {
        CloudService::AwsS3 | CloudService::S3Compat => {
            let mut builder = services::S3::default()
                .bucket(&store.bucket)
                .root(&root)
                .access_key_id(&cred.access_key_id)
                .secret_access_key(&cred.secret_access_key)
                // 不读 ~/.aws 配置与实例元数据：面板的存储配置必须自洽，
                // 否则会"看起来配好了但实际在用机器上的另一套凭据"
                .disable_config_load()
                .disable_ec2_metadata();
            if !store.endpoint.is_empty() {
                builder = builder.endpoint(&store.endpoint);
            }
            if !store.region.is_empty() {
                builder = builder.region(&store.region);
            }
            if store.virtual_host_style {
                builder = builder.enable_virtual_host_style();
            }
            if !cred.security_token.is_empty() {
                builder = builder.session_token(&cred.security_token);
            }
            Operator::new(builder)
                .map_err(|e| fail(format!("初始化 {} 客户端失败：{e}", store.service.label())))?
        }
        CloudService::Oss => {
            let mut builder = services::Oss::default()
                .bucket(&store.bucket)
                .root(&root)
                .endpoint(&store.endpoint)
                .access_key_id(&cred.access_key_id)
                .access_key_secret(&cred.secret_access_key)
                .addressing_style(if store.virtual_host_style {
                    "virtual"
                } else {
                    "path"
                });
            if !cred.security_token.is_empty() {
                builder = builder.security_token(&cred.security_token);
            }
            Operator::new(builder)
                .map_err(|e| fail(format!("初始化 {} 客户端失败：{e}", store.service.label())))?
        }
        CloudService::Cos => {
            let mut builder = services::Cos::default()
                .bucket(&store.bucket)
                .root(&root)
                .endpoint(&store.endpoint)
                .secret_id(&cred.access_key_id)
                .secret_key(&cred.secret_access_key)
                .disable_config_load();
            if !cred.security_token.is_empty() {
                builder = builder.security_token(&cred.security_token);
            }
            Operator::new(builder)
                .map_err(|e| fail(format!("初始化 {} 客户端失败：{e}", store.service.label())))?
        }
    };
    Ok(op)
}

/// 逻辑路径归一化：丢弃空段与 `.`，`..` 只回退一层（不允许越出逻辑根）。
pub fn normalize_path(raw: &str) -> Result<String, ZapError> {
    if raw.contains('\0') {
        return Err(fail("路径包含非法字符"));
    }
    let mut segs: Vec<&str> = Vec::new();
    for seg in raw.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    let out = segs.join("/");
    if out.len() > 1024 {
        return Err(fail("路径过长"));
    }
    Ok(out)
}

fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn parent_of(path: &str) -> String {
    match path.rfind('/') {
        Some(idx) => path[..idx].to_string(),
        None => String::new(),
    }
}

// ── 浏览与操作 ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct CloudEntry {
    pub name: String,
    /// 相对逻辑根的路径；目录以 `/` 结尾（前端据此区分并可原样回传）
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// RFC3339 时间串（前端本地化展示），可能为空
    pub modified: String,
    pub etag: String,
}

#[derive(Debug, Serialize)]
pub struct CloudListData {
    pub current_path: String,
    /// 上一级路径（根目录时为空串）
    pub parent_path: String,
    pub entries: Vec<CloudEntry>,
    /// 是否因为条目过多被截断
    pub truncated: bool,
}

/// 列目录（单层，目录优先）。
pub async fn list(
    store: &CloudStore,
    path: &str,
    show_hidden: bool,
) -> Result<CloudListData, ZapError> {
    let op = build_operator(store)?;
    let current = normalize_path(path)?;
    let prefix = if current.is_empty() {
        String::new()
    } else {
        format!("{current}/")
    };

    // 用流式 lister 而不是 `list_with().limit()`：
    //
    // `limit` 在 opendal 里是**每页条数**，会原样变成对象存储的 `max-keys`。
    // COS / OSS 的 `max-keys` 只允许 0–1000，传 2001 会被判为 `InvalidArgument`，
    // 整个列目录请求直接失败（S3 / MinIO 是静默截断到 1000，所以只有 COS/OSS 会炸）。
    // 这里不设 limit（用服务端默认页大小，opendal 自动翻页），读到面板上限就停，
    // 因此既不会超协议上限，也不会把超大目录整个拉进内存。
    //
    // recursive(false) 表示「只列当前层」（等价于旧的 delimiter("/")）。
    let mut lister = op
        .lister_with(&prefix)
        .recursive(false)
        .await
        .map_err(|e| oe("读取目录", e))?;

    let mut out: Vec<CloudEntry> = Vec::new();
    let mut truncated = false;
    while let Some(entry) = lister.try_next().await.map_err(|e| oe("读取目录", e))? {
        if out.len() >= LIST_LIMIT {
            truncated = true;
            break;
        }
        let full = entry.path();
        // delimiter 模式下条目只会是「直接子目录（带尾 /）」或「直接子文件」
        let rel = full.strip_prefix(prefix.as_str()).unwrap_or(full);
        let name = rel.trim_end_matches('/');
        // 跳过目录自身占位对象（create_dir 产生的 `a/`）
        if name.is_empty() || name.contains('/') {
            continue;
        }
        if !show_hidden && name.starts_with('.') {
            continue;
        }

        let meta = entry.metadata();
        let is_dir = meta.is_dir() || full.ends_with('/');
        let path = if is_dir {
            format!("{}/", join_path(&current, name))
        } else {
            join_path(&current, name)
        };
        out.push(CloudEntry {
            name: name.to_string(),
            path,
            is_dir,
            size: if is_dir { 0 } else { meta.content_length() },
            modified: meta
                .last_modified()
                .map(|t| t.to_string())
                .unwrap_or_default(),
            etag: meta.etag().unwrap_or_default().to_string(),
        });
    }

    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(CloudListData {
        parent_path: parent_of(&current),
        current_path: current,
        entries: out,
        truncated,
    })
}

pub struct CloudDownload {
    pub stream: opendal::FuturesBytesStream,
    pub name: String,
    pub size: u64,
    pub content_type: String,
}

/// 打开对象用于下载（流式，不在内存里攒整个文件）。
pub async fn download(store: &CloudStore, path: &str) -> Result<CloudDownload, ZapError> {
    let op = build_operator(store)?;
    let target = normalize_path(path)?;
    if target.is_empty() {
        return Err(fail("缺少文件路径"));
    }

    let meta = op.stat(&target).await.map_err(|e| oe("读取文件信息", e))?;
    if meta.is_dir() {
        return Err(fail("目标是目录，无法下载"));
    }

    let content_type = meta
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();
    let size = meta.content_length();
    let name = target.rsplit('/').next().unwrap_or(&target).to_string();

    let reader = op.reader(&target).await.map_err(|e| oe("打开文件", e))?;
    let stream = reader
        .into_bytes_stream(..)
        .await
        .map_err(|e| oe("读取文件", e))?;

    Ok(CloudDownload {
        stream,
        name,
        size,
        content_type,
    })
}

/// 新建目录（对象存储用零字节占位对象表示目录）。
pub async fn create_dir(store: &CloudStore, path: &str) -> Result<(), ZapError> {
    let op = build_operator(store)?;
    let target = normalize_path(path)?;
    if target.is_empty() {
        return Err(fail("缺少目录路径"));
    }
    op.create_dir(&format!("{target}/"))
        .await
        .map_err(|e| oe("创建目录", e))?;
    Ok(())
}

/// 删除：文件删单个对象；目录递归删（`path` 以 `/` 结尾表示目录）。
pub async fn delete(store: &CloudStore, path: &str) -> Result<bool, ZapError> {
    let op = build_operator(store)?;
    let is_dir = path.trim_end().ends_with('/');
    let target = normalize_path(path)?;
    if target.is_empty() {
        return Err(fail("不能删除逻辑根目录"));
    }

    if is_dir {
        let prefix = format!("{target}/");
        op.delete_with(&prefix)
            .recursive(true)
            .await
            .map_err(|e| oe("删除目录", e))?;
        return Ok(true);
    }

    // 无占位对象的「目录」在 stat 下会报 not found，此时按前缀递归处理，
    // 但必须先确认确实有子对象，避免把同前缀的兄弟对象误删（如 `a` 与 `ab/`）。
    if op.stat(&target).await.is_err() {
        // 流式探一个条目即可：`list_with().limit(1)` 的 1 是**每页条数**，
        // 配合 `.await` 会把整个目录按 1 条一页翻完（大目录直接打到超时）。
        let mut probe = op
            .lister_with(&format!("{target}/"))
            .await
            .map_err(|e| oe("读取目录", e))?;
        let empty = probe
            .try_next()
            .await
            .map_err(|e| oe("读取目录", e))?
            .is_none();
        if empty {
            return Err(fail("目标不存在"));
        }
        op.delete_with(&format!("{target}/"))
            .recursive(true)
            .await
            .map_err(|e| oe("删除目录", e))?;
        return Ok(true);
    }

    op.delete(&target).await.map_err(|e| oe("删除文件", e))?;
    Ok(false)
}

/// 重命名 / 移动。
///
/// 对象存储没有真正的 rename：文件走「读-写-删」，目录递归复制后删除。
/// 跨服务、跨实现行为一致（不依赖各家的 copy 语义）。
pub async fn rename(store: &CloudStore, from: &str, to: &str) -> Result<bool, ZapError> {
    let op = build_operator(store)?;
    let is_dir = from.trim_end().ends_with('/');
    let src = normalize_path(from)?;
    let dst = normalize_path(to)?;

    if src.is_empty() || dst.is_empty() {
        return Err(fail("源路径与目标路径不能为空"));
    }
    if src == dst {
        return Ok(is_dir);
    }
    if is_dir && (dst == src || dst.starts_with(&format!("{src}/"))) {
        return Err(fail("不能把目录移动到它自己的子目录下"));
    }

    if !is_dir {
        let data = op.read(&src).await.map_err(|e| oe("读取源文件", e))?;
        op.write(&dst, data)
            .await
            .map_err(|e| oe("写入目标文件", e))?;
        op.delete(&src).await.map_err(|e| oe("删除源文件", e))?;
        return Ok(false);
    }

    let src_prefix = format!("{src}/");
    let dst_prefix = format!("{dst}/");
    let children = op
        .list_with(&src_prefix)
        .recursive(true)
        .await
        .map_err(|e| oe("读取源目录", e))?;
    if children.is_empty() {
        return Err(fail("源目录不存在或为空"));
    }

    for child in children {
        let child_path = child.path();
        let rel = child_path
            .strip_prefix(src_prefix.as_str())
            .unwrap_or(child_path);
        let data = op.read(child_path).await.map_err(|e| oe("读取源文件", e))?;
        op.write(&format!("{dst_prefix}{rel}"), data)
            .await
            .map_err(|e| oe("写入目标文件", e))?;
        op.delete(child_path)
            .await
            .map_err(|e| oe("删除源文件", e))?;
    }
    Ok(true)
}

#[derive(Debug, Serialize)]
pub struct TestOutcome {
    /// 列出的样例条目数（能列出即说明鉴权与网络都通）
    pub entries: usize,
    pub elapsed_ms: u128,
}

/// 流式上传会话：边收边写，内存占用与文件大小无关。
///
/// 浏览器上传的是 multipart 分片，这里把每个分片直接写进 opendal [`opendal::Writer`]，
/// 避免「先攒成 `Vec<u8>` 再上传」在遇到大文件时把内存打满。
pub struct UploadSession {
    writer: opendal::Writer,
    written: u64,
}

impl UploadSession {
    /// 开始写入某个对象（路径为相对逻辑根的文件路径）。
    pub async fn create(store: &CloudStore, path: &str) -> Result<Self, ZapError> {
        let op = build_operator(store)?;
        let target = normalize_path(path)?;
        if target.is_empty() {
            return Err(fail("缺少目标文件路径"));
        }
        let writer = op.writer(&target).await.map_err(|e| oe("开始写入", e))?;
        Ok(Self { writer, written: 0 })
    }

    pub async fn write(&mut self, chunk: bytes::Bytes) -> Result<(), ZapError> {
        self.written += chunk.len() as u64;
        self.writer
            .write(chunk)
            .await
            .map_err(|e| oe("写入文件", e))
    }

    /// 收尾并返回写入字节数；不调用则对象不完整（丢弃 writer 会自动 abort）。
    pub async fn finish(mut self) -> Result<u64, ZapError> {
        self.writer.close().await.map_err(|e| oe("写入文件", e))?;
        Ok(self.written)
    }

    /// 中途失败时主动丢弃半截对象。
    pub async fn abort(mut self) {
        let _ = self.writer.abort().await;
    }
}

/// 读盘分块大小：单次读入内存的粒度，与文件大小无关。
const UPLOAD_CHUNK: usize = 256 * 1024;

/// 把服务器上的本地文件流式上传成云对象（不经浏览器中转）。
///
/// 供 `/system/cloud/upload-local` 使用：本地路径的可访问性由路由层校验
/// （家目录白名单），这里只负责「读盘 → 写对象」，内存占用与文件大小无关。
pub async fn upload_from_path(
    store: &CloudStore,
    target: &str,
    local: &Path,
) -> Result<u64, ZapError> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(local)
        .await
        .map_err(|e| fail(format!("读取本地文件失败：{e}")))?;
    let mut session = UploadSession::create(store, target).await?;
    let mut buf = vec![0u8; UPLOAD_CHUNK];

    loop {
        let read = match file.read(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                // 读盘失败同样丢弃半截对象，别在桶里留个残缺文件
                session.abort().await;
                return Err(fail(format!("读取本地文件失败：{e}")));
            }
        };
        if read == 0 {
            break;
        }
        if let Err(e) = session
            .write(bytes::Bytes::copy_from_slice(&buf[..read]))
            .await
        {
            session.abort().await;
            return Err(e);
        }
    }

    session.finish().await
}

// ── 前端表单预设 ────────────────────────────────────────────

/// 服务预设：默认端点、哪些字段必填。
///
/// 放在后端是为了让「必填规则」只有一处定义（保存接口用的是同一套校验），
/// 前端不再各自硬编码一份，避免两边漂移。
#[derive(Debug, Serialize)]
pub struct ServicePreset {
    pub id: &'static str,
    pub label: &'static str,
    pub endpoint_required: bool,
    pub endpoint_hint: &'static str,
    pub region_required: bool,
    pub virtual_host_default: bool,
}

pub fn service_catalog() -> Vec<ServicePreset> {
    vec![
        ServicePreset {
            id: "aws_s3",
            label: "AWS S3",
            endpoint_required: false,
            endpoint_hint: "留空按 Region 推导官方地址",
            region_required: true,
            virtual_host_default: true,
        },
        ServicePreset {
            id: "oss",
            label: "阿里云 OSS",
            endpoint_required: true,
            endpoint_hint: "oss-cn-hangzhou.aliyuncs.com",
            region_required: false,
            virtual_host_default: true,
        },
        ServicePreset {
            id: "cos",
            label: "腾讯云 COS",
            endpoint_required: true,
            endpoint_hint: "cos.ap-guangzhou.myqcloud.com",
            region_required: false,
            virtual_host_default: true,
        },
        ServicePreset {
            id: "s3_compat",
            label: "S3 兼容存储",
            endpoint_required: true,
            endpoint_hint: "https://minio.example.com:9000",
            region_required: false,
            virtual_host_default: false,
        },
    ]
}

/// 连通性测试：列逻辑根下的少量对象。
pub async fn test_connection(store: &CloudStore) -> Result<TestOutcome, ZapError> {
    let op = build_operator(store)?;
    let started = std::time::Instant::now();
    let entries = tokio::time::timeout(TEST_TIMEOUT, async {
        // 取到 TEST_ENTRIES 个就停，不把整个桶翻完：能列出条目就说明鉴权与网络都通。
        // （`list_with().limit(n)` 的 n 只是每页条数，`.await` 会把所有页收完，
        // 桶里对象一多就成了成百上千次请求，表现为「测试连接一直转圈到超时」。）
        let mut lister = op.lister_with("").await?;
        let mut count = 0usize;
        while lister.try_next().await?.is_some() {
            count += 1;
            if count >= TEST_ENTRIES {
                break;
            }
        }
        Ok::<usize, opendal::Error>(count)
    })
    .await
    .map_err(|_| fail(format!("连接超时（{} 秒）", TEST_TIMEOUT.as_secs())))?
    .map_err(|e| oe("连接云存储", e))?;

    Ok(TestOutcome {
        entries,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：云存储操作必须能真正发出 HTTP 请求。
    ///
    /// opendal 0.59 起 HTTP 传输层是「进程级安装」的（见 [`ensure_http_transport`]），
    /// 一旦 `build_operator` 漏装，**所有**对象存储请求都会以
    /// `default HTTP transport is not installed` 失败——现象就是「配好 COS 却读不到文件」。
    ///
    /// 这里指向必然拒绝连接的本地端口（127.0.0.1:1），不需要外网：
    /// 只要错误不是「传输层未安装」且带上该地址，就说明请求确实发出去了。
    #[tokio::test]
    async fn build_operator_installs_http_transport() {
        let store = CloudStore {
            version: 1,
            id: "0123456789abcdef".to_string(),
            name: "probe".to_string(),
            service: CloudService::S3Compat,
            // path 样式：主机名保持 127.0.0.1:1，不会被改写成 probe.127.0.0.1
            endpoint: "http://127.0.0.1:1".to_string(),
            region: String::from("zap-test"),
            bucket: "probe".to_string(),
            root: String::new(),
            virtual_host_style: false,
            // `decrypt` 对非 `v1:` 前缀的内容原样返回，测试里免去密钥文件
            secret: r#"{"access_key_id":"ak","secret_access_key":"sk","security_token":""}"#
                .to_string(),
            access_key_hint: String::new(),
            created_at: 0,
            updated_at: 0,
        };

        let op = build_operator(&store).expect("构造 operator 失败");
        let mut lister = op.lister_with("").await.expect("建立列表流失败");
        // 注意：lister_with().await 只是建流，真正发请求是在 try_next()
        let err = lister.try_next().await.expect_err("127.0.0.1:1 不该连通");

        // 走真实的报错格式化路径，顺带确认错误链被补齐
        let msg = oe("读取目录", err).to_string();
        assert!(
            !msg.contains("transport is not installed"),
            "HTTP 传输层未安装：{msg}"
        );
        assert!(msg.contains("127.0.0.1:1"), "请求没有真正发出：{msg}");
    }

    /// 错误链补齐：opendal 只渲染一级 `source`，更内层的原因得自己挖出来，
    /// 否则用户只会看到 "error sending request for url (...)" 这种无从下手的提示。
    #[test]
    fn cause_chain_keeps_root_cause() {
        /// 模拟 hyper-util 的包装层：只说"连接阶段失败"
        #[derive(Debug)]
        struct Middle(std::io::Error);

        impl std::fmt::Display for Middle {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("client error (Connect)")
            }
        }

        impl std::error::Error for Middle {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        let err = opendal::Error::new(opendal::ErrorKind::Unexpected, "send http request")
            .set_source(Middle(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "Connection refused (os error 111)",
            )));

        // opendal 自己只会打印到 "client error (Connect)"
        assert!(!err.to_string().contains("Connection refused"));

        let text = cause_chain(&err);
        assert!(text.contains("；原因："), "缺少原因链：{text}");
        assert!(
            text.contains("Connection refused (os error 111)"),
            "缺少最内层原因：{text}"
        );
    }

    /// 「服务端答了话」的业务错误不该被当成网络故障，否则会附上无关的代理提示。
    #[test]
    fn business_error_is_not_a_network_failure() {
        let answered = "PermissionDenied (permanent) at list, context: { response: Parts { \
            status: 403, headers: {\"connection\": \"keep-alive\", \"content-type\": \"application/xml\"} \
            } } => CosError { code: \"InvalidAccessKeyId\" }";
        assert!(!is_network_failure(answered));

        let not_sent = "Unexpected (temporary) at list, context: { called: reqwest::send } \
            => send http request；原因：client error (Connect) → tcp connect error \
            → Connection refused (os error 111)";
        assert!(is_network_failure(not_sent));
    }

    #[test]
    fn path_traversal_is_contained() {
        assert_eq!(normalize_path("/a/b/c").unwrap(), "a/b/c");
        assert_eq!(normalize_path("a//b/").unwrap(), "a/b");
        // `..` 只能回退，不能越出逻辑根
        assert_eq!(normalize_path("a/../b").unwrap(), "b");
        assert_eq!(normalize_path("../../etc/passwd").unwrap(), "etc/passwd");
        assert_eq!(normalize_path("./a/./b").unwrap(), "a/b");
        assert_eq!(normalize_path("").unwrap(), "");
        assert!(normalize_path("a\0b").is_err());
    }

    #[test]
    fn endpoint_normalized_to_https() {
        assert_eq!(
            normalize_endpoint(" oss-cn-hangzhou.aliyuncs.com/ "),
            "https://oss-cn-hangzhou.aliyuncs.com"
        );
        assert_eq!(
            normalize_endpoint("http://127.0.0.1:9000/"),
            "http://127.0.0.1:9000"
        );
        assert_eq!(normalize_endpoint(""), "");
    }

    #[test]
    fn root_prefix_cleaned() {
        assert_eq!(normalize_root("/website/"), "website");
        assert_eq!(normalize_root("a//b/"), "a/b");
        assert_eq!(normalize_root("../x"), "x");
        assert_eq!(normalize_root("/"), "");
    }

    #[test]
    fn access_key_masked() {
        assert_eq!(mask_key("LTAI5tAbcdEFGH1234"), "LTAI****1234");
        assert_eq!(mask_key("short"), "*****");
    }

    #[test]
    fn store_id_must_be_hex16() {
        assert!(safe_store_id("0123456789abcdef").is_ok());
        assert!(safe_store_id("0123456789ABCDEF").is_ok());
        assert!(safe_store_id("../../etc/passwd").is_err());
        assert!(safe_store_id("01234").is_err());
        let generated = new_store_id().unwrap();
        assert_eq!(generated.len(), 16);
        assert!(safe_store_id(&generated).is_ok());
    }

    #[test]
    fn dir_path_helpers() {
        assert_eq!(join_path("", "a.txt"), "a.txt");
        assert_eq!(join_path("docs/2026", "a.txt"), "docs/2026/a.txt");
        assert_eq!(parent_of("a/b/c.txt"), "a/b");
        assert_eq!(parent_of("a.txt"), "");
    }
}
