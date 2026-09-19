// 站点管理（admin 管理全部 / reseller 管理所属客户的站点 / 普通用户管理自己的站点）
// 一个站点可绑定多个域名与多个 IP（site_domain / site_ip 子表）
use axum::{Json, extract::Extension, extract::Query};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};
use tracing::{info, warn};

use crate::{
    db,
    zap::{
        ZapError, ZapJsonResult, audit,
        jwt::{self, ValidatedClaims},
        server_env,
    },
};
use zap_proto::{LocationSpec, Request, UpstreamSpec};

use super::system_basic::{K_IPV4 as K_DEFAULT_IPV4, K_IPV6 as K_DEFAULT_IPV6};
use super::user::USER_KIND_MEMBER;

// ── SQL 行结构 ──────────────────────────────────────────────

// sqlx 行映射元组别名（避免 clippy::type_complexity）
type SiteRow = (
    i64,
    i64,
    String,
    i32,
    String,
    i64,
    i64,
    Option<String>,
    String,
);
type SiteRowExt = (
    i64,
    i64,
    String,
    i32,
    String,
    i64,
    i64,
    Option<String>,
    String,
    Vec<String>,
    Vec<String>,
);
type SyncOneRow = (
    String,
    i32,
    String,
    String,
    String,
    String,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

#[derive(sqlx::FromRow, Debug)]
struct OwnerCandidate {
    id: i64,
    username: String,
    nickname: String,
}

// ── 入参 ────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct SiteListQuery {
    pub search: Option<String>,
    pub status: Option<i32>,
    pub user_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SiteAddPayload {
    #[serde(default)]
    pub user_id: Option<i64>,
    /// 站点名称（非必填，留空默认取第一个域名）
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub ips: Vec<String>,
    #[serde(default)]
    pub status: Option<i32>,
    #[serde(default)]
    pub remark: Option<String>,
    /// PHP 实例标识（appstore 已安装 PHP 应用的 instance，如 php74）；空表示未绑定
    #[serde(default)]
    pub php_instance: Option<String>,
    /// 站点类型：php（默认，PHP/PHP+静态）/ static（纯静态）/ proxy（反向代理）
    #[serde(default)]
    pub site_type: String,
    /// 伪静态预设：none / thinkphp / laravel / wordpress / codeigniter / custom
    #[serde(default)]
    pub pseudo_static: String,
    /// 伪静态自定义规则（多行指令，仅 preset=custom 时使用；仅运营者可提交）
    #[serde(default)]
    pub pseudo_custom: String,
    /// true = 站点目录使用归属用户家目录下已存在的自定义目录（需同时给 web_root）
    #[serde(default)]
    pub web_root_custom: bool,
    /// 自定义站点目录绝对路径（web_root_custom=true 时必填）
    #[serde(default)]
    pub web_root: Option<String>,
    /// 自动目录自定义子路径（相对归属用户家目录，如 www/blog；空/未填 = 面板默认规划
    /// {home}/www/{name}-{id}）。目录不存在时 vhost 同步阶段会自动创建。
    #[serde(default)]
    pub web_root_sub: Option<String>,
    /// 自定义 upstream 组
    #[serde(default)]
    pub upstreams: Vec<UpstreamSpec>,
    /// 自定义 location（反代 / 跳转 / 拒绝 / 站内目录）
    #[serde(default)]
    pub locations: Vec<LocationSpec>,
    /// 绑定的证书库证书 id（0 = 不启用 HTTPS）
    #[serde(default)]
    pub ssl_cert_id: Option<i64>,
    /// 允许 HTTP 跳转到 HTTPS（仅绑定证书后生效）
    #[serde(default)]
    pub force_https: bool,
    /// TLS 协议版本（空格分隔的 nginx ssl_protocols 值；空 = 面板默认 TLSv1.2 TLSv1.3）
    #[serde(default)]
    pub ssl_protocols: String,
    /// SSL 密码套件（nginx ssl_ciphers 值；空 = 不输出指令，跟随执行端默认）
    #[serde(default)]
    pub ssl_ciphers: String,
    /// 服务端密码套件优先（ssl_prefer_server_ciphers，仅影响 TLSv1.2）
    #[serde(default = "default_true")]
    pub ssl_prefer_server_ciphers: bool,
    /// 启用 HTTP/2
    #[serde(default = "default_true")]
    pub ssl_http2: bool,
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct SiteUpdatePayload {
    pub id: i64,
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub name: Option<String>,
    /// None 表示域名保持不变；Some(任意数组，可为空) 表示整体覆盖
    #[serde(default)]
    pub domains: Option<Vec<String>>,
    #[serde(default)]
    pub ips: Option<Vec<String>>,
    #[serde(default)]
    pub status: Option<i32>,
    /// 运行状态：running / stopped / maintenance；与 status 同时传时以此为准
    #[serde(default)]
    pub run_state: Option<String>,
    #[serde(default)]
    pub remark: Option<String>,
    /// None 表示 PHP 实例保持不变；Some(空串) 表示清除 PHP 实例
    #[serde(default)]
    pub php_instance: Option<String>,
    /// 以下为站点扩展档案：None 表示保持不变（整体覆盖式提交时全量给出）
    #[serde(default)]
    pub site_type: Option<String>,
    #[serde(default)]
    pub pseudo_static: Option<String>,
    #[serde(default)]
    pub pseudo_custom: Option<String>,
    /// Some(true)=切换为「选择已有目录」模式（需同时给 web_root）；
    /// Some(false)=切回面板自动目录
    #[serde(default)]
    pub web_root_custom: Option<bool>,
    /// 自定义站点目录绝对路径（web_root_custom=true 且首次指定时必填）
    #[serde(default)]
    pub web_root: Option<String>,
    /// 自动目录自定义子路径（相对归属用户家目录）；Some(非空) 时刷新为 {home}/{sub}，
    /// None / 空串 = 不迁移目录（维持现有文档根或按面板默认规划）。
    #[serde(default)]
    pub web_root_sub: Option<String>,
    #[serde(default)]
    pub upstreams: Option<Vec<UpstreamSpec>>,
    #[serde(default)]
    pub locations: Option<Vec<LocationSpec>>,
    /// SSL 证书绑定：None = 保持不变；Some(0) = 解除绑定；Some(id) = 绑定证书库证书
    #[serde(default)]
    pub ssl_cert_id: Option<i64>,
    /// 允许 HTTP 跳转 HTTPS：None = 保持不变
    #[serde(default)]
    pub force_https: Option<bool>,
    /// TLS 协议版本：None = 保持不变（空串 = 面板默认 TLSv1.2 TLSv1.3）
    #[serde(default)]
    pub ssl_protocols: Option<String>,
    /// SSL 密码套件：None = 保持不变（空串 = 不输出指令，跟随执行端默认）
    #[serde(default)]
    pub ssl_ciphers: Option<String>,
    /// 服务端密码套件优先：None = 保持不变
    #[serde(default)]
    pub ssl_prefer_server_ciphers: Option<bool>,
    /// 启用 HTTP/2：None = 保持不变
    #[serde(default)]
    pub ssl_http2: Option<bool>,
}

/// 站点已有目录浏览入参
#[derive(Debug, Deserialize)]
pub struct SiteDirsPayload {
    /// 归属用户 id（admin/reseller 可指定他人；普通用户忽略，恒为自己）
    #[serde(default)]
    pub user_id: Option<i64>,
    /// 当前浏览的目录（不传 = 家目录根）；返回其下子目录
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SiteDeletePayload {
    pub ids: Vec<i64>,
    /// 是否同时删除站点数据：网站文件（web_root）+ 日志（log_root）。
    /// 前端删除确认框勾选后才为 true，未勾选只删配置，站点目录与日志保留。
    #[serde(default)]
    pub remove_data: bool,
}

// ── 工具函数 ────────────────────────────────────────────────

fn has_role(roles: &str, role: &str) -> bool {
    roles.split(',').any(|r| r.trim() == role)
}

/// 归属组 ID：团队成员（`user_kind = 1`）→ 其父账号 id；其余账号 → 自己。
///
/// 团队（站长 + 其成员）共享站点与证书：可见性 / 管理权判定先折算到归属组，
/// 再按「组长本人 + 组内成员」的账号集合过滤（条件见 `group_scope_cond`）。
/// 无团队的普通用户组 ID 即自身，行为与「只能看自己的」等价。
pub(crate) async fn group_id_of(uid: i64) -> i64 {
    let pool = db::get_db_pool().await;
    let row: Option<(i32, i64)> =
        sqlx::query_as("SELECT user_kind, owner_id FROM user WHERE id = ?")
            .bind(uid)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match row {
        Some((kind, owner)) if kind == USER_KIND_MEMBER && owner > 0 => owner,
        _ => uid,
    }
}

/// 归属组范围条件：`{col}` 属于「组长本人 + 其团队成员」。
/// 三个占位符依次绑定：组 ID、`USER_KIND_MEMBER`、组 ID。
pub(crate) fn group_scope_cond(col: &str) -> String {
    format!("({col} = ? OR {col} IN (SELECT id FROM user WHERE user_kind = ? AND owner_id = ?))")
}

/// 站点管理角色门禁：admin / reseller / 普通用户（demo 等不可访问）
fn require_manageable(claims: &jwt::Claims) -> Result<(), ZapError> {
    if jwt::is_admin(claims) || jwt::is_reseller(claims) || has_role(&claims.roles, "user") {
        Ok(())
    } else {
        Err(ZapError::New(
            -1,
            "权限不足，仅支持 admin / reseller / 普通用户访问站点管理".to_string(),
        ))
    }
}

/// 校验归属用户是否可被当前操作者指定：
/// 归属对象可以是 admin / reseller / user 任一角色（即可以归属自己或其它运营账号），
/// 但必须落在当前操作者的管理范围：
/// admin → 任意上述账号；reseller → 自己 + 自己的客户；普通用户 → 只能是自己
pub(crate) async fn resolve_target_user(claims: &jwt::Claims, target: i64) -> Result<(), ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT roles, owner_id FROM user WHERE id = ?")
            .bind(target)
            .fetch_optional(pool)
            .await?;
    let Some((roles, owner_id)) = row else {
        return Err(ZapError::New(-1, "指定的归属用户不存在".to_string()));
    };
    let manageable =
        has_role(&roles, "admin") || has_role(&roles, "reseller") || has_role(&roles, "user");
    if !manageable {
        return Err(ZapError::New(
            -1,
            "该账号（如只读演示账号）不能作为站点归属".to_string(),
        ));
    }
    if jwt::is_admin(claims) {
        return Ok(());
    }
    if jwt::is_reseller(claims) {
        if target == claims.id as i64 || owner_id == claims.id as i64 {
            Ok(())
        } else {
            Err(ZapError::New(
                -1,
                "只能将站点归属自己或所属客户".to_string(),
            ))
        }
    } else if target == claims.id as i64 {
        Ok(())
    } else if group_id_of(target).await == group_id_of(claims.id as i64).await {
        // 团队共享：可把站点归属给同团队的其他成员（站长 ↔ 成员、成员 ↔ 成员）
        Ok(())
    } else {
        Err(ZapError::New(
            -1,
            "只能将站点归属自己或同团队的成员".to_string(),
        ))
    }
}

/// 校验站点是否处于当前操作者的管理范围
async fn site_in_scope(claims: &jwt::Claims, site_id: i64) -> Result<(), ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM site WHERE id = ?")
        .bind(site_id)
        .fetch_optional(pool)
        .await?;
    let Some((uid,)) = row else {
        return Err(ZapError::New(-1, "站点不存在".to_string()));
    };
    if jwt::is_admin(claims) {
        return Ok(());
    }
    if jwt::is_reseller(claims) {
        // reseller：自己的站点也可直接管理
        if uid == claims.id as i64 {
            return Ok(());
        }
        let owner: Option<(i64,)> = sqlx::query_as("SELECT owner_id FROM user WHERE id = ?")
            .bind(uid)
            .fetch_optional(pool)
            .await?;
        match owner {
            Some((o,)) if o == claims.id as i64 => Ok(()),
            _ => Err(ZapError::New(
                -1,
                "只能管理自己或所属客户的站点".to_string(),
            )),
        }
    } else if uid == claims.id as i64 {
        Ok(())
    } else if group_id_of(uid).await == group_id_of(claims.id as i64).await {
        // 团队共享：站长与其成员互相可管理组内站点
        Ok(())
    } else {
        Err(ZapError::New(
            -1,
            "只能管理自己或同团队成员的站点".to_string(),
        ))
    }
}

/// 单个域名归一化（判重与入库都以归一化结果为准，避免 `A.com` / `a.com.` / `http://a.com/x`
/// 这类写法绕过唯一性检查）：
/// - 去首尾空白、转小写
/// - 允许粘贴整条 URL：去掉 scheme、端口与路径/查询串
/// - 去掉结尾点（`a.com.` → `a.com`）
/// - 国际化域名（中文等）转 punycode（`xn--`）
/// - 保留泛域名写法 `*.example.com`
fn normalize_domain(raw: &str) -> Result<String, String> {
    let mut s = raw.trim().to_lowercase();
    for p in ["https://", "http://", "//"] {
        if let Some(rest) = s.strip_prefix(p) {
            s = rest.to_string();
            break;
        }
    }
    if let Some(idx) = s.find(['/', '?', '#']) {
        s = s[..idx].to_string();
    }
    // 域名里出现 ':' 只可能是 host:port（IPv6 形式的域名不支持）
    if let Some(idx) = s.find(':') {
        s = s[..idx].to_string();
    }
    s = s.trim_end_matches('.').to_string();
    if s.is_empty() {
        return Err("域名不能为空".to_string());
    }
    if !s.is_ascii() {
        let (star, host) = match s.strip_prefix("*.") {
            Some(h) => (true, h),
            None => (false, s.as_str()),
        };
        let ascii = idna::domain_to_ascii(host).map_err(|_| {
            format!(
                "域名 {} 无法转换为 punycode（请检查是否含非法字符）",
                raw.trim()
            )
        })?;
        s = if star { format!("*.{ascii}") } else { ascii };
    }
    Ok(s)
}

/// 规范化域名数组：归一化 + 去空 + 去重（保持顺序）
fn norm_domains(raw: &[String]) -> Result<Vec<String>, ZapError> {
    let mut out: Vec<String> = Vec::new();
    for d in raw {
        if d.trim().is_empty() {
            continue;
        }
        let t = normalize_domain(d).map_err(|e| ZapError::New(-1, e))?;
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    Ok(out)
}

/// 规范化 IP 数组：trim + 去空 + 去重（保持顺序）
fn norm_ips(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in raw {
        let t = s.trim().to_string();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// 域名合法性：支持泛域名 `*.a.com`；逐标签校验（长度、字符集、不能以 - 开头/结尾）
/// 并禁止空标签（`a..com`、`.a.com`、`a.com.`）。
fn valid_domain(d: &str) -> bool {
    let host = d.strip_prefix("*.").unwrap_or(d);
    if host.is_empty() || host.chars().count() > 253 || d.starts_with('.') {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.chars().count() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    })
}

/// 名称非必填：留空默认取第一个域名
fn fallback_name(name: &str, domains: &[String]) -> String {
    if name.trim().is_empty() {
        domains.first().cloned().unwrap_or_default()
    } else {
        name.trim().to_string()
    }
}

/// PHP 实例标识校验：允许字母/数字/./_/-/@，最长 120；空串表示未绑定
fn valid_php_instance(s: &str) -> bool {
    s.chars().count() <= 120
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'@' | b'/'))
}

/// 未绑定套餐（或套餐未设置域名上限）时的兜底上限，避免无套餐用户无限制绑定域名。
const DEFAULT_MAX_DOMAINS: usize = 50;

/// 生效的域名上限：套餐 max_domains > 0 时用套餐值，否则用兜底值。
fn domain_limit_of(max_domains: i64) -> usize {
    if max_domains > 0 {
        max_domains as usize
    } else {
        DEFAULT_MAX_DOMAINS
    }
}

fn validate_site_fields(
    name: &str,
    domains: &[String],
    ips: &[String],
    remark: &str,
    max_domains: i64,
) -> Result<(), ZapError> {
    if name.is_empty() {
        return Err(ZapError::New(
            -1,
            "站点名称或域名不能为空（名称留空时默认取第一个域名）".to_string(),
        ));
    }
    if name.chars().count() > 120 {
        return Err(ZapError::New(
            -1,
            "站点名称过长（最多 120 个字符）".to_string(),
        ));
    }
    // 单站点域名数：优先取归属用户套餐的 max_domains，未配置时用兜底值
    let limit = domain_limit_of(max_domains);
    if domains.len() > limit {
        return Err(ZapError::New(
            -1,
            if max_domains > 0 {
                format!(
                    "当前套餐限制单个站点最多绑定 {limit} 个域名（已填 {} 个）",
                    domains.len()
                )
            } else {
                format!(
                    "单个站点最多绑定 {limit} 个域名（已填 {} 个）",
                    domains.len()
                )
            },
        ));
    }
    for d in domains {
        if !valid_domain(d) {
            return Err(ZapError::New(
                -1,
                format!("域名 {} 格式不正确（仅支持字母/数字/./-/ _）", d),
            ));
        }
    }
    if ips.len() > 50 {
        return Err(ZapError::New(-1, "单个站点最多绑定 50 个 IP".to_string()));
    }
    for ip in ips {
        if ip.parse::<IpAddr>().is_err() {
            return Err(ZapError::New(
                -1,
                format!("绑定的 IP {} 地址格式不正确", ip),
            ));
        }
    }
    if remark.chars().count() > 500 {
        return Err(ZapError::New(-1, "备注过长（最多 500 个字符）".to_string()));
    }
    Ok(())
}

/// 计算站点的文档根与日志目录：统一规划在归属用户家目录下
/// - web_root = {home}/www/{sanitize(name)}-{site_id}
/// - log_root = {home}/logs/{site_id}-{sanitize(name)}
///
/// 日志目录用 ID 打头：站点改名不会造成日志目录漂移/旧目录残留，且按创建顺序排列，
/// 后缀保留站点名便于人工辨认（统计按 `*/access.log` 扫描，不依赖命名）。
///
/// 归属用户无 home_dir 时返回空串（执行端回退 {ZAP_PATH}/data/www/...，兼容老站点）
async fn site_dirs_for(
    user_id: i64,
    name: &str,
    site_id: i64,
) -> Result<(String, String), ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(String,)> = sqlx::query_as("SELECT home_dir FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    let Some((home,)) = row else {
        return Ok((String::new(), String::new()));
    };
    let home = home.trim();
    if home.is_empty() {
        return Ok((String::new(), String::new()));
    }
    let seg = zap_proto::sanitize_site_name(name);
    Ok((
        format!("{home}/www/{seg}-{site_id}"),
        format!("{home}/logs/{site_id}-{seg}"),
    ))
}

/// 启动自检：把历史站点的日志目录从 `{name}-{id}` 迁移为 `{id}-{name}`。
///
/// - 库中 `log_root` 与当前规划值不一致时才更新（幂等）；
/// - 旧目录存在且新目录不存在时整体 `rename`，历史 access.log / error.log 不丢
///   （inode 不变，流量增量统计不会出现漏计/重计）；
/// - 变更过的站点自动重同步 vhost，避免 nginx 仍指向旧路径。
pub async fn migrate_log_roots() {
    let pool = db::get_db_pool().await;
    let rows: Vec<(i64, i64, String, String)> =
        sqlx::query_as("SELECT id, user_id, name, log_root FROM site")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let mut changed: Vec<i64> = Vec::new();

    for (id, user_id, name, log_root) in rows {
        let Ok((_, want)) = site_dirs_for(user_id, &name, id).await else {
            continue;
        };
        let old_s = log_root.trim().to_string();
        if want.is_empty() || old_s.is_empty() || old_s == want {
            continue;
        }
        let old = std::path::Path::new(&old_s);
        let new = std::path::Path::new(&want);
        if old.is_dir() && !new.exists() {
            match std::fs::rename(old, new) {
                Ok(()) => info!("站点 {} 日志目录迁移: {} -> {}", id, old_s, want),
                Err(e) => {
                    warn!(
                        "站点 {} 日志目录迁移失败（保留原路径）: {} -> {} err={}",
                        id, old_s, want, e
                    );
                    continue;
                }
            }
        }
        if sqlx::query("UPDATE site SET log_root = ? WHERE id = ?")
            .bind(&want)
            .bind(id)
            .execute(pool)
            .await
            .is_ok()
        {
            changed.push(id);
        }
    }

    if !changed.is_empty() {
        info!(
            "日志目录命名迁移完成，重同步 {} 个站点 vhost",
            changed.len()
        );
        for id in changed {
            let _ = sync_one_site(id).await;
        }
    }
}

/// 判重用键：除自身外，`a.com` 与 `www.a.com` 互为冲突键（同站点内允许共存，
/// 跨站点一律判冲突，避免 nginx 最长匹配带来的"看起来绑上了、实际不生效"）。
fn domain_match_keys(d: &str) -> Vec<String> {
    let mut keys = vec![d.to_string()];
    if let Some(bare) = d.strip_prefix("www.") {
        keys.push(bare.to_string());
    } else if !d.starts_with("*.") {
        keys.push(format!("www.{d}"));
    }
    keys
}

/// 泛域名 `*.suffix` 是否覆盖 `domain`（两个泛域名覆盖范围相交也算冲突）。
fn wildcard_covers(suffix: &str, domain: &str) -> bool {
    if let Some(other) = domain.strip_prefix("*.") {
        other == suffix
            || other.ends_with(&format!(".{suffix}"))
            || suffix.ends_with(&format!(".{other}"))
    } else {
        domain == suffix || domain.ends_with(&format!(".{suffix}"))
    }
}

/// LIKE 通配符转义（域名里可能出现 `_`，不转义会误判冲突）。
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// 站点运行状态（三态）：running / stopped / maintenance
pub const RUN_RUNNING: &str = "running";
pub const RUN_STOPPED: &str = "stopped";
pub const RUN_MAINTENANCE: &str = "maintenance";

/// 归一化运行状态：非法值返回 None（调用方据此报错）
fn normalize_run_state(s: &str) -> Option<&'static str> {
    match s.trim().to_lowercase().as_str() {
        "running" | "start" => Some(RUN_RUNNING),
        "stopped" | "stop" => Some(RUN_STOPPED),
        "maintenance" | "maintain" => Some(RUN_MAINTENANCE),
        _ => None,
    }
}

/// run_state → 兼容用的 status（running/maintenance 视为启用，stopped 为停用）
fn run_state_to_status(state: &str) -> i32 {
    if state == RUN_STOPPED { 0 } else { 1 }
}

/// 冲突提示：管理员看得到占用方站点（便于排查），普通用户只提示被占用，避免信息泄露。
fn conflict_err(input: &str, hit: &str, site_id: i64, is_admin: bool) -> ZapError {
    if is_admin {
        ZapError::New(
            -1,
            format!("域名 {input} 与站点 id={site_id} 已绑定的 {hit} 冲突"),
        )
    } else {
        ZapError::New(-1, format!("域名 {input} 已被占用，请更换或联系管理员"))
    }
}

/// 严格模式域名冲突检查（多租户：跨用户同样判冲突）：
/// 1. 完全相同
/// 2. `a.com` ↔ `www.a.com`
/// 3. 已存在的泛域名覆盖本次域名（如 `*.a.com` 覆盖 `b.a.com`）
/// 4. 本次提交的是泛域名，且覆盖了别人的精确域名
///
/// `exclude_site` 用于更新时排除自身；`is_admin` 决定错误信息是否暴露占用方。
async fn ensure_domains_unique(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    domains: &[String],
    exclude_site: i64,
    is_admin: bool,
) -> Result<(), ZapError> {
    // 泛域名数量通常很少，一次性取出后在内存里比对
    let wildcards: Vec<(i64, String)> = sqlx::query_as(
        "SELECT site_id, domain FROM site_domain WHERE site_id != ? AND domain LIKE '*.%'",
    )
    .bind(exclude_site)
    .fetch_all(&mut **tx)
    .await?;

    for d in domains {
        // 1) + 2) 精确匹配与 www 变体（走唯一索引）
        for key in domain_match_keys(d) {
            let exists: Option<(i64,)> =
                sqlx::query_as("SELECT site_id FROM site_domain WHERE domain = ? AND site_id != ?")
                    .bind(&key)
                    .bind(exclude_site)
                    .fetch_optional(&mut **tx)
                    .await?;
            if let Some((sid,)) = exists {
                return Err(conflict_err(d, &key, sid, is_admin));
            }
        }
        // 3) 被别人的泛域名覆盖
        for (sid, w) in &wildcards {
            if let Some(suffix) = w.strip_prefix("*.")
                && wildcard_covers(suffix, d)
            {
                return Err(conflict_err(d, w, *sid, is_admin));
            }
        }
        // 4) 本次是泛域名，覆盖了别人的精确域名
        if let Some(suffix) = d.strip_prefix("*.") {
            let like = format!("%.{}", like_escape(suffix));
            let rows: Vec<(i64, String)> = sqlx::query_as(
                "SELECT site_id, domain FROM site_domain WHERE site_id != ? \
                 AND (domain = ? OR domain LIKE ? ESCAPE '\\')",
            )
            .bind(exclude_site)
            .bind(suffix)
            .bind(&like)
            .fetch_all(&mut **tx)
            .await?;
            if let Some((sid, hit)) = rows.first() {
                return Err(conflict_err(d, hit, *sid, is_admin));
            }
        }
    }
    Ok(())
}

// ── 站点扩展档案（site_profile）与功能开关 ───────────────────────

/// 站点类型白名单（与 zapexec 保持一致）
const SITE_TYPES: [&str; 3] = ["php", "static", "proxy"];
/// 伪静态预设 key 白名单
const PSEUDO_PRESETS: [&str; 6] = [
    "none",
    "thinkphp",
    "laravel",
    "wordpress",
    "codeigniter",
    "custom",
];

fn is_operator(claims: &jwt::Claims) -> bool {
    jwt::is_admin(claims) || jwt::is_reseller(claims)
}

/// 反向代理能力门禁（「自定义目录」已全量开放，不再受套餐限制）：
/// - admin / reseller（operator）恒开放；
/// - 普通用户以「绑定套餐」的 allow_proxy 为准；未绑定 / 套餐停用时回退全局「默认套餐」。
async fn gates_for(claims: &jwt::Claims) -> bool {
    if is_operator(claims) {
        return true;
    }
    match crate::routers::package::effective_package_of(claims.id as i64).await {
        Some(pkg) => pkg.allow_proxy == 1,
        None => false,
    }
}

fn norm_site_type(t: &str) -> Result<&'static str, ZapError> {
    let t = t.trim().to_lowercase();
    if SITE_TYPES.contains(&t.as_str()) {
        Ok(match t.as_str() {
            "static" => "static",
            "proxy" => "proxy",
            _ => "php",
        })
    } else {
        Err(ZapError::New(
            -1,
            format!("站点类型仅支持 php / static / proxy（收到：{t}）"),
        ))
    }
}

/// 伪静态预设配套校验：白名单 + custom 时的规则文本约束。
/// 空 preset 视为「未改动」，此时不允许附带自定义规则文本。
fn norm_pseudo(preset: &str, custom: &str, allow_custom: bool) -> Result<(), ZapError> {
    let p = preset.trim().to_lowercase();
    if p.is_empty() {
        if !custom.trim().is_empty() {
            return Err(ZapError::New(
                -1,
                "未选择伪静态预设，不能附带自定义规则文本".to_string(),
            ));
        }
        return Ok(());
    }
    if !PSEUDO_PRESETS.contains(&p.as_str()) {
        return Err(ZapError::New(-1, format!("伪静态预设不支持：{preset}")));
    }
    if p == "custom" {
        if !allow_custom {
            return Err(ZapError::New(
                -1,
                "自定义伪静态规则仅向管理员 / 代理商开放".to_string(),
            ));
        }
        if custom.trim().is_empty() {
            return Err(ZapError::New(
                -1,
                "请填写自定义伪静态规则（预设选 custom 时必填）".to_string(),
            ));
        }
        if custom.contains('#') {
            return Err(ZapError::New(
                -1,
                "自定义伪静态规则中不允许使用 # 注释".to_string(),
            ));
        }
    } else if !custom.trim().is_empty() {
        return Err(ZapError::New(
            -1,
            "预设不是 custom，不能附带自定义规则文本".to_string(),
        ));
    }
    Ok(())
}

/// 站点扩展档案（与 site_profile 列一一对应；ssl_cert_id>0 = 绑定证书库证书启用 HTTPS）
/// 8..11：TLS 高级设置（ssl_protocols / ssl_ciphers / ssl_prefer_server_ciphers / ssl_http2）
type ProfileRow = (
    String,
    String,
    String,
    bool,
    String,
    String,
    i64,
    bool,
    String,
    String,
    bool,
    bool,
);

/// load_profile 的原始查询行（列序见 SQL；i64 为 SQLite 原生整数，映射时转 bool）
type ProfileRowRaw = (
    String,
    String,
    String,
    i64,
    String,
    String,
    i64,
    i64,
    String,
    String,
    i64,
    i64,
);

/// 读取站点扩展档案；老站点（无档案行）返回默认值
async fn load_profile(site_id: i64) -> ProfileRow {
    let pool = db::get_db_pool().await;
    let row: Option<ProfileRowRaw> = sqlx::query_as(
        "SELECT site_type, pseudo_static, pseudo_custom, web_root_custom, upstreams, locations, \
                ssl_cert_id, force_https, ssl_protocols, ssl_ciphers, \
                ssl_prefer_server_ciphers, ssl_http2 \
         FROM site_profile WHERE site_id = ?",
    )
    .bind(site_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    row.map(|(t, p, pc, wc, u, l, ssl, fh, pr, ci, pp, h2)| {
        (
            t,
            p,
            pc,
            wc != 0,
            u,
            l,
            ssl,
            fh != 0,
            pr,
            ci,
            pp != 0,
            h2 != 0,
        )
    })
    .unwrap_or_else(|| {
        (
            "php".into(),
            "none".into(),
            String::new(),
            false,
            "[]".into(),
            "[]".into(),
            0,
            false,
            String::new(),
            String::new(),
            true,
            true,
        )
    })
}

/// 写回站点扩展档案（存在则整体覆盖）
#[allow(clippy::too_many_arguments)]
async fn save_profile(
    site_id: i64,
    site_type: &str,
    pseudo_static: &str,
    pseudo_custom: &str,
    web_root_custom: bool,
    upstreams: &[UpstreamSpec],
    locations: &[LocationSpec],
    ssl_cert_id: i64,
    force_https: bool,
    ssl_protocols: &str,
    ssl_ciphers: &str,
    ssl_prefer_server_ciphers: bool,
    ssl_http2: bool,
) -> Result<(), ZapError> {
    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();
    let u = serde_json::to_string(upstreams).unwrap_or_else(|_| "[]".to_string());
    let l = serde_json::to_string(locations).unwrap_or_else(|_| "[]".to_string());
    sqlx::query(
        "INSERT INTO site_profile (site_id, site_type, pseudo_static, pseudo_custom, \
         web_root_custom, upstreams, locations, ssl_cert_id, force_https, \
         ssl_protocols, ssl_ciphers, ssl_prefer_server_ciphers, ssl_http2, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(site_id) DO UPDATE SET \
           site_type = excluded.site_type, pseudo_static = excluded.pseudo_static, \
           pseudo_custom = excluded.pseudo_custom, web_root_custom = excluded.web_root_custom, \
           upstreams = excluded.upstreams, locations = excluded.locations, \
           ssl_cert_id = excluded.ssl_cert_id, force_https = excluded.force_https, \
           ssl_protocols = excluded.ssl_protocols, ssl_ciphers = excluded.ssl_ciphers, \
           ssl_prefer_server_ciphers = excluded.ssl_prefer_server_ciphers, \
           ssl_http2 = excluded.ssl_http2, \
           updated_at = excluded.updated_at",
    )
    .bind(site_id)
    .bind(site_type)
    .bind(pseudo_static)
    .bind(pseudo_custom)
    .bind(if web_root_custom { 1i64 } else { 0i64 })
    .bind(&u)
    .bind(&l)
    .bind(ssl_cert_id)
    .bind(if force_https { 1i64 } else { 0i64 })
    .bind(ssl_protocols)
    .bind(ssl_ciphers)
    .bind(if ssl_prefer_server_ciphers {
        1i64
    } else {
        0i64
    })
    .bind(if ssl_http2 { 1i64 } else { 0i64 })
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// 校验证书库绑定：证书必须存在、启用，且归属与站点归属用户一致（或同属一个归属组）。
/// 证书按用户隔离：跨归属绑定会让站点归属用户在「SSL/TLS」中看不到该证书；
/// 团队成员共享站点，因此证书与站点归属同属一个归属组时也允许绑定。
async fn ensure_cert_bindable(cert_id: i64, owner_user_id: i64) -> Result<(), ZapError> {
    let pool = db::get_db_pool().await;
    let st: Option<(i64, i64)> =
        sqlx::query_as("SELECT status, user_id FROM ssl_cert WHERE id = ?")
            .bind(cert_id)
            .fetch_optional(pool)
            .await?;
    match st {
        Some((1, cuid)) if cuid == owner_user_id => Ok(()),
        // 团队共享：证书与站点归属同组（站长 + 成员）即可绑定
        Some((1, cuid)) if group_id_of(cuid).await == group_id_of(owner_user_id).await => Ok(()),
        Some((1, _)) => Err(ZapError::New(
            -1,
            "所选 SSL 证书不属于本站点的归属用户：请先在「SSL/TLS → 证书管理」为该用户添加证书，\
             或将现有证书「归属」改为该用户（系统证书 0 需先转归属）后再绑定"
                .to_string(),
        )),
        Some(_) => Err(ZapError::New(
            -1,
            "所选 SSL 证书已停用，请先在「SSL/TLS → 证书管理」中启用或换用其他证书".to_string(),
        )),
        None => Err(ZapError::New(
            -1,
            "所选 SSL 证书不存在，请刷新后重新选择（证书可能已被删除）".to_string(),
        )),
    }
}

/// JSON 文本 → 结构体列表（脏数据回退为空列表）
fn parse_specs<T: serde::de::DeserializeOwned>(text: &str) -> Vec<T> {
    serde_json::from_str(text).unwrap_or_default()
}

/// 归属用户家目录（空 = 尚未初始化）
async fn home_dir_of(user_id: i64) -> Result<String, ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(String,)> = sqlx::query_as("SELECT home_dir FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    match row {
        Some((h,)) => Ok(h.trim().to_string()),
        None => Err(ZapError::New(-1, "归属用户不存在".to_string())),
    }
}

/// 校验并确认「选择已有目录」：绝对路径、在归属用户家目录内、目录确实存在。
/// 存在性交给 root 侧目录浏览 verb 验证（zapd 进程非 root，读不了别人的家目录）。
async fn resolve_custom_web_root(owner: i64, raw: &str) -> Result<String, ZapError> {
    let p = raw.trim().to_string();
    if p.is_empty() {
        return Err(ZapError::New(
            -1,
            "请指定自定义站点目录（已在服务器上创建）".to_string(),
        ));
    }
    if !p.starts_with('/') {
        return Err(ZapError::New(
            -1,
            "自定义站点目录必须是绝对路径".to_string(),
        ));
    }
    if p.split('/').any(|s| s == "..") {
        return Err(ZapError::New(-1, "自定义站点目录不允许包含 ..".to_string()));
    }
    let home = home_dir_of(owner).await?;
    if home.is_empty() {
        return Err(ZapError::New(
            -1,
            "该用户家目录尚未初始化：请先用「自动目录」创建并同步一次站点，再改用自定义目录"
                .to_string(),
        ));
    }
    if !(p == home || p.starts_with(&format!("{home}/"))) {
        return Err(ZapError::New(
            -1,
            format!("自定义站点目录必须在归属用户家目录（{home}）下"),
        ));
    }
    let resp = crate::zapexec::call(Request::FsBrowseDirs { base: p.clone() }).await?;
    if resp.code != 0 {
        return Err(ZapError::New(
            -1,
            format!("目录不可用（不存在 / 无权限 / 非目录）：{}", resp.message),
        ));
    }
    Ok(p)
}

/// 归一化「自动目录自定义子路径」为 home 下的相对段（允许粘贴完整绝对路径：
/// 已在 home 前缀下则截掉；其余情况剥离首尾 `/`）。
/// 返回格式：`a/b/c`。拒绝 `..`、控制字符与非常规路径字符。
fn clean_auto_sub(raw: &str, home: &str) -> Result<String, ZapError> {
    let mut s = raw.trim();
    // 粘贴了完整绝对路径（带 home 前缀）时直接截掉前缀
    if let Some(rest) = s.strip_prefix(home) {
        s = rest;
    }
    let s = s.trim_matches('/').trim();
    let mut segs: Vec<&str> = Vec::new();
    for part in s.split('/') {
        let seg = part.trim();
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return Err(ZapError::New(-1, "自定义站点目录不允许包含 ..".to_string()));
        }
        if seg.chars().any(|c| {
            c.is_control() || !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '~'))
        }) {
            return Err(ZapError::New(
                -1,
                format!("自定义站点目录段含非法字符：{seg}（仅允许字母/数字/_/-/./~）"),
            ));
        }
        segs.push(seg);
    }
    if segs.is_empty() {
        return Err(ZapError::New(
            -1,
            "请填写自定义站点目录（例如 www/blog）".to_string(),
        ));
    }
    Ok(segs.join("/"))
}

/// 自动目录文档根规划：
/// - `sub` 为空 → 面板默认规划 {home}/www/{sanitize(name)}-{site_id}（site_dirs_for）；
/// - `sub` 非空 → {home}/{clean sub}（目录不存在时由 vhost 同步阶段递归创建，不会写占位覆盖已有文件）。
///
/// 日志目录始终为 {home}/logs/{site_id}-{sanitize(name)}。
async fn auto_dirs_for(
    owner: i64,
    name: &str,
    site_id: i64,
    sub: Option<&str>,
) -> Result<(String, String), ZapError> {
    let raw = sub.map(str::trim).unwrap_or("");
    if raw.is_empty() {
        return site_dirs_for(owner, name, site_id).await;
    }
    let home = home_dir_of(owner).await?;
    if home.is_empty() {
        return Err(ZapError::New(
            -1,
            "该用户家目录尚未初始化：请先创建并同步一次站点，再指定自定义站点目录".to_string(),
        ));
    }
    let rel = clean_auto_sub(raw, &home)?;
    let (_, log_root) = site_dirs_for(owner, name, site_id).await?;
    Ok((format!("{home}/{rel}"), log_root))
}

/// 轻量业务校验（站点类型 / 伪静态 / 功能开关门禁 / upstream/location 字段形态）。
/// 更细的 nginx 语法与注入校验由 zapexec 同步时兜底执行。
async fn validate_advanced_inputs(
    claims: &jwt::Claims,
    site_type: &str,
    pseudo_static: &str,
    pseudo_custom: &str,
    upstreams: &[UpstreamSpec],
    locations: &[LocationSpec],
) -> Result<(), ZapError> {
    let op = is_operator(claims);
    let g_proxy = gates_for(claims).await;
    let t = site_type.trim().to_lowercase();
    // 未指定类型（增量编辑）跳过类型相关门禁
    if !t.is_empty() {
        norm_site_type(site_type)?;
        if t == "proxy" && !g_proxy {
            return Err(ZapError::New(
                -1,
                "反向代理功能未对当前账号开放，请联系管理员在「系统 → 套餐」中为你的套餐开启"
                    .to_string(),
            ));
        }
    }
    norm_pseudo(pseudo_static, pseudo_custom, op)?;
    if (!upstreams.is_empty() || !locations.is_empty()) && !g_proxy {
        return Err(ZapError::New(
            -1,
            "自定义 upstream / location 未对当前账号开放，请联系管理员在「系统 → 套餐」中开启「反向代理」".to_string(),
        ));
    }
    if t == "proxy" && locations.is_empty() {
        return Err(ZapError::New(
            -1,
            "反向代理站点至少需要配置一个 location（例如 location / 转发到后端）".to_string(),
        ));
    }
    if locations.len() > 16 {
        return Err(ZapError::New(-1, "自定义 location 最多 16 个".to_string()));
    }
    if upstreams.len() > 8 {
        return Err(ZapError::New(-1, "upstream 组最多 8 个".to_string()));
    }
    let kinds = ["proxy", "redirect", "deny", "alias", "raw"];
    let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for u in upstreams {
        let n = u.name.trim();
        if n.is_empty()
            || !n
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ZapError::New(
                -1,
                format!("upstream 名称非法：{}（仅字母/数字/_/-）", u.name),
            ));
        }
        if !names.insert(n.to_string()) {
            return Err(ZapError::New(-1, format!("upstream 名称重复：{n}")));
        }
        match u.balance.trim() {
            "" | "least_conn" | "ip_hash" => {}
            other => {
                return Err(ZapError::New(
                    -1,
                    format!("upstream 负载策略不支持：{other}"),
                ));
            }
        }
        for s in &u.servers_ext {
            let a = s.addr.trim();
            if a.len() > 200
                || (!a.is_empty()
                    && !a.chars().all(|c| {
                        c.is_ascii_alphanumeric()
                            || matches!(c, '.' | ':' | '/' | '_' | '-' | '[' | ']' | '%')
                    }))
            {
                return Err(ZapError::New(
                    -1,
                    format!("upstream {n} 的 server 地址含非法字符：{a}"),
                ));
            }
            if s.weight > 1000 || s.max_fails > 100 || s.fail_timeout > 3600 {
                return Err(ZapError::New(
                    -1,
                    format!(
                        "upstream {n} 的 server 参数超限（weight ≤ 1000 / max_fails ≤ 100 / fail_timeout ≤ 3600s）"
                    ),
                ));
            }
        }
    }
    for l in locations {
        let p = l.path.trim();
        if !p.starts_with('/') {
            return Err(ZapError::New(
                -1,
                format!("location 路径必须以 / 开头：{}", l.path),
            ));
        }
        if !kinds.contains(&l.kind.trim().to_lowercase().as_str()) {
            return Err(ZapError::New(
                -1,
                format!("location 类型不支持：{}", l.kind),
            ));
        }
        let k = l.kind.trim().to_lowercase();
        if l.target.len() > 400 || l.target.contains('{') || l.target.contains('}') {
            return Err(ZapError::New(
                -1,
                format!("location「{}」的目标参数过长或含非法字符", l.path),
            ));
        }
        // 高级参数形态（nginx 语法细节由执行端同步时兜底）
        if l.conn_timeout > 86400 || l.read_timeout > 86400 || l.send_timeout > 86400 {
            return Err(ZapError::New(-1, "代理超时最大 86400 秒".to_string()));
        }
        if l.headers.len() > 20 {
            return Err(ZapError::New(
                -1,
                "每个 location 自定义请求头最多 20 个".to_string(),
            ));
        }
        for h in &l.headers {
            if h.key.trim().len() > 64
                || (!h.key.trim().is_empty()
                    && !h
                        .key
                        .trim()
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')))
            {
                return Err(ZapError::New(
                    -1,
                    "自定义请求头名称仅允许字母/数字/_/-".to_string(),
                ));
            }
            if h.value.trim().len() > 500
                || h.value
                    .trim()
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '{' | '}' | ';' | '#'))
            {
                return Err(ZapError::New(
                    -1,
                    format!("请求头「{}」的值非法或超长", h.key),
                ));
            }
        }
        if !l.cache.trim().is_empty() && l.cache.trim() != "zap_cache" {
            return Err(ZapError::New(
                -1,
                "缓存区仅支持内置的 zap_cache".to_string(),
            ));
        }
        match k.as_str() {
            "deny" => {
                if !matches!(l.code, 0 | 403 | 404 | 410 | 444) {
                    return Err(ZapError::New(
                        -1,
                        format!("拒绝状态码仅支持 403/404/410/444（收到 {}）", l.code),
                    ));
                }
            }
            "redirect" => {
                if !matches!(l.code, 0 | 301 | 302 | 303 | 307 | 308) {
                    return Err(ZapError::New(
                        -1,
                        format!("跳转状态码仅支持 301/302/303/307/308（收到 {}）", l.code),
                    ));
                }
            }
            "raw" => {
                if l.raw.trim().is_empty() {
                    return Err(ZapError::New(
                        -1,
                        format!("location「{}」类型为 raw 时指令体不能为空", l.path),
                    ));
                }
                if l.raw.len() > 8000 {
                    return Err(ZapError::New(
                        -1,
                        "raw 自由指令体过长（上限 8000 字符）".to_string(),
                    ));
                }
                if l.raw.contains('{') || l.raw.contains('}') {
                    return Err(ZapError::New(
                        -1,
                        "raw 自由指令体不允许花括号（仅支持单层 location 内指令）".to_string(),
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// ── 处理器 ──────────────────────────────────────────────────

/// 站点列表（按角色裁剪范围）+ 汇总统计
pub async fn site_list(claims: ValidatedClaims, Query(q): Query<SiteListQuery>) -> ZapJsonResult {
    require_manageable(&claims)?;
    let pool = db::get_db_pool().await;
    let base_sql = "SELECT s.id, s.user_id, s.name, s.status, s.remark, s.created_at, s.updated_at, \
                    u.username AS owner_username, s.php_instance \
                    FROM site s LEFT JOIN user u ON u.id = s.user_id";
    let rows: Vec<SiteRow> = if jwt::is_admin(&claims) {
        sqlx::query_as(&format!("{} ORDER BY s.id DESC", base_sql))
            .fetch_all(pool)
            .await?
    } else if jwt::is_reseller(&claims) {
        // reseller：自己的站点 + 名下客户的站点
        sqlx::query_as(&format!(
            "{} WHERE s.user_id = ? OR s.user_id IN (SELECT id FROM user WHERE owner_id = ?) \
                 ORDER BY s.id DESC",
            base_sql
        ))
        .bind(claims.id as i64)
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await?
    } else {
        // 团队共享：站长 ↔ 成员、成员 ↔ 成员互相可见（无团队时等价于只看自己的）
        let gid = group_id_of(claims.id as i64).await;
        sqlx::query_as(&format!(
            "{} WHERE {} ORDER BY s.id DESC",
            base_sql,
            group_scope_cond("s.user_id")
        ))
        .bind(gid)
        .bind(USER_KIND_MEMBER)
        .bind(gid)
        .fetch_all(pool)
        .await?
    };

    // 批量加载子表域名 / IP
    let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
    let mut domain_map: HashMap<i64, Vec<String>> = HashMap::new();
    let mut ip_map: HashMap<i64, Vec<String>> = HashMap::new();
    let mut vh_map: HashMap<i64, String> = HashMap::new();
    let mut rs_map: HashMap<i64, String> = HashMap::new();
    let mut ve_map: HashMap<i64, String> = HashMap::new();
    let mut vt_map: HashMap<i64, i64> = HashMap::new();
    let mut dir_map: HashMap<i64, (String, String)> = HashMap::new();
    // 站点磁盘占用（字节，定时任务 du web_root + log_root；0 = 尚未采集）
    let mut disk_map: HashMap<i64, i64> = HashMap::new();
    let mut disk_stat_map: HashMap<i64, i64> = HashMap::new();
    // 站点本月流量（字节，定时任务解析 access.log；列表列展示用）
    let mut traffic_map: HashMap<i64, i64> = HashMap::new();
    // 站点扩展档案（类型 / 伪静态 / 自定义目录 / upstream / location）
    let mut pf_map: HashMap<i64, ProfileRow> = HashMap::new();
    // 归属用户的 Linux 系统账号（system 模式下 PHP pool 按此账号隔离）
    let mut lu_map: HashMap<i64, String> = HashMap::new();
    if !ids.is_empty() {
        let ph = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let dsql = format!(
            "SELECT site_id, domain FROM site_domain WHERE site_id IN ({}) ORDER BY id",
            ph
        );
        let mut dq = sqlx::query_as::<_, (i64, String)>(&dsql);
        for id in &ids {
            dq = dq.bind(id);
        }
        for (sid, d) in dq.fetch_all(pool).await? {
            domain_map.entry(sid).or_default().push(d);
        }
        let isql = format!(
            "SELECT site_id, ip FROM site_ip WHERE site_id IN ({}) ORDER BY id",
            ph
        );
        let mut iq = sqlx::query_as::<_, (i64, String)>(&isql);
        for id in &ids {
            iq = iq.bind(id);
        }
        for (sid, ip) in iq.fetch_all(pool).await? {
            ip_map.entry(sid).or_default().push(ip);
        }
        // vhost 同步状态 / 失败原因 / 最近同步时间（独立 map，不进入主行 tuple）
        let vsql = format!(
            "SELECT id, vhost_state, run_state, vhost_error, vhost_synced_at \
             FROM site WHERE id IN ({}) ORDER BY id",
            ph
        );
        let mut vq = sqlx::query_as::<_, (i64, String, String, String, i64)>(&vsql);
        for id in &ids {
            vq = vq.bind(id);
        }
        for (sid, state, run, err, ts) in vq.fetch_all(pool).await? {
            vh_map.insert(sid, state);
            rs_map.insert(sid, run);
            ve_map.insert(sid, err);
            vt_map.insert(sid, ts);
        }
        // 站点文档根 / 日志目录 / 磁盘占用（独立 map，不进入主行 tuple）
        let dirsql = format!(
            "SELECT id, web_root, log_root, disk_used_bytes, disk_stat_at, traffic_month_bytes \
             FROM site WHERE id IN ({}) ORDER BY id",
            ph
        );
        let mut dirq = sqlx::query_as::<_, (i64, String, String, i64, i64, i64)>(&dirsql);
        for id in &ids {
            dirq = dirq.bind(id);
        }
        for (sid, w, l, disk, ds_at, tmon) in dirq.fetch_all(pool).await? {
            dir_map.insert(sid, (w, l));
            disk_map.insert(sid, disk);
            disk_stat_map.insert(sid, ds_at);
            traffic_map.insert(sid, tmon);
        }
        // 站点扩展档案（类型 / 伪静态 / 自定义目录 / upstream / location / SSL 绑定 / TLS 高级）
        let psql2 = format!(
            "SELECT site_id, site_type, pseudo_static, pseudo_custom, web_root_custom, \
             upstreams, locations, ssl_cert_id, force_https, ssl_protocols, ssl_ciphers, \
             ssl_prefer_server_ciphers, ssl_http2 \
             FROM site_profile WHERE site_id IN ({})",
            ph
        );
        let mut pq2 = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                String,
                i64,
                String,
                String,
                i64,
                i64,
                String,
                String,
                i64,
                i64,
            ),
        >(&psql2);
        for id in &ids {
            pq2 = pq2.bind(id);
        }
        for (sid, t, p, pc, wc, u, l, ssl, fh, pr, ci, pp, h2) in pq2.fetch_all(pool).await? {
            pf_map.insert(
                sid,
                (
                    t,
                    p,
                    pc,
                    wc != 0,
                    u,
                    l,
                    ssl,
                    fh != 0,
                    pr,
                    ci,
                    pp != 0,
                    h2 != 0,
                ),
            );
        }
        // 归属用户的 Linux 系统账号（system 模式下 PHP pool 按此账号隔离）
        let mut owner_ids: Vec<i64> = rows.iter().map(|r| r.1).collect();
        owner_ids.sort_unstable();
        owner_ids.dedup();
        if !owner_ids.is_empty() {
            let ph2 = owner_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let lusql = format!("SELECT id, linux_user FROM user WHERE id IN ({})", ph2);
            let mut lq = sqlx::query_as::<_, (i64, String)>(&lusql);
            for id in &owner_ids {
                lq = lq.bind(id);
            }
            for (uid, lu) in lq.fetch_all(pool).await? {
                lu_map.insert(uid, lu);
            }
        }
    }

    // 站点绑定证书的显示名（仅 SSL 启用的站点；证书被删后保持空名，前端可提示已失效）
    let mut ssl_name_map: HashMap<i64, String> = HashMap::new();
    let cert_ids: Vec<i64> = pf_map.values().map(|p| p.6).filter(|c| *c > 0).collect();
    if !cert_ids.is_empty() {
        let phc = cert_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let csql = format!("SELECT id, name FROM ssl_cert WHERE id IN ({})", phc);
        let mut cq = sqlx::query_as::<_, (i64, String)>(&csql);
        for cid in &cert_ids {
            cq = cq.bind(cid);
        }
        for (cid, cname) in cq.fetch_all(pool).await? {
            ssl_name_map.insert(cid, cname);
        }
    }

    let mut recs: Vec<SiteRowExt> = rows
        .into_iter()
        .map(|r| {
            let domains = domain_map.remove(&r.0).unwrap_or_default();
            let ips = ip_map.remove(&r.0).unwrap_or_default();
            (r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, domains, ips)
        })
        .collect();

    // 前端筛选（内存过滤，数据量小）
    if let Some(uid) = q.user_id {
        recs.retain(|r| r.1 == uid);
    }
    if let Some(status) = q.status
        && (status == 0 || status == 1)
    {
        recs.retain(|r| r.3 == status);
    }
    if let Some(search) = q.search {
        let s = search.trim().to_lowercase();
        if !s.is_empty() {
            recs.retain(|r| {
                r.2.to_lowercase().contains(&s)
                    || r.9.iter().any(|d| d.contains(&s))
                    || r.10.iter().any(|ip| ip.to_lowercase().contains(&s))
            });
        }
    }

    let (mut running, mut stopped) = (0usize, 0usize);
    for r in &recs {
        if r.3 == 1 {
            running += 1;
        } else {
            stopped += 1;
        }
    }
    // 同步失败数（失败原因见每行的 vhost_error）
    let failed = recs
        .iter()
        .filter(|r| {
            matches!(
                vh_map.get(&r.0).map(|s| s.as_str()).unwrap_or("pending"),
                "failed" | "error"
            )
        })
        .count();
    let list: Vec<Value> = recs
        .iter()
        .map(|r| {
            json!({
                "id": r.0,
                "user_id": r.1,
                "owner_username": r.7.as_deref().unwrap_or(""),
                "linux_user": lu_map.get(&r.1).cloned().unwrap_or_default(),
                "name": r.2,
                "php_instance": r.8,
                "domains": r.9,
                "ips": r.10,
                "status": r.3,
                "run_state": rs_map.get(&r.0).cloned().unwrap_or_else(|| if r.3 == 1 { RUN_RUNNING.to_string() } else { RUN_STOPPED.to_string() }),
                "vhost_state": vh_map.get(&r.0).cloned().unwrap_or_else(|| "pending".into()),
                "vhost_error": ve_map.get(&r.0).cloned().unwrap_or_default(),
                "vhost_synced_at": vt_map.get(&r.0).copied().unwrap_or(0),
                "web_root": dir_map.get(&r.0).map(|d| d.0.clone()).unwrap_or_default(),
                "log_root": dir_map.get(&r.0).map(|d| d.1.clone()).unwrap_or_default(),
                "disk_used_bytes": disk_map.get(&r.0).copied().unwrap_or(0),
                "disk_stat_at": disk_stat_map.get(&r.0).copied().unwrap_or(0),
                "traffic_month_bytes": traffic_map.get(&r.0).copied().unwrap_or(0),
                "site_type": pf_map.get(&r.0).map(|p| p.0.clone()).unwrap_or_else(|| "php".into()),
                "pseudo_static": pf_map.get(&r.0).map(|p| p.1.clone()).unwrap_or_else(|| "none".into()),
                "pseudo_custom": pf_map.get(&r.0).map(|p| p.2.clone()).unwrap_or_default(),
                "web_root_custom": pf_map.get(&r.0).map(|p| p.3).unwrap_or(false),
                "upstreams": pf_map.get(&r.0).map(|p| serde_json::from_str::<Value>(&p.4).unwrap_or_else(|_| json!([]))).unwrap_or_else(|| json!([])),
                "locations": pf_map.get(&r.0).map(|p| serde_json::from_str::<Value>(&p.5).unwrap_or_else(|_| json!([]))).unwrap_or_else(|| json!([])),
                "ssl_cert_id": pf_map.get(&r.0).map(|p| p.6).unwrap_or(0),
                "force_https": pf_map.get(&r.0).map(|p| p.7).unwrap_or(false),
                "ssl_protocols": pf_map.get(&r.0).map(|p| p.8.clone()).unwrap_or_default(),
                "ssl_ciphers": pf_map.get(&r.0).map(|p| p.9.clone()).unwrap_or_default(),
                "ssl_prefer_server_ciphers": pf_map.get(&r.0).map(|p| p.10).unwrap_or(true),
                "ssl_http2": pf_map.get(&r.0).map(|p| p.11).unwrap_or(true),
                "ssl_cert_name": pf_map.get(&r.0).and_then(|p| ssl_name_map.get(&p.6)).cloned().unwrap_or_default(),
                "remark": r.4,
                "created_at": r.5,
                "updated_at": r.6,
            })
        })
        .collect();

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "total": recs.len(),
            "running": running,
            "stopped": stopped,
            "failed": failed,
            "rows": list,
        }
    })))
}

/// 可选归属用户列表：
/// admin → 全部 admin/reseller/user 账号（含自己）；reseller → 自己 + 自己的客户；
/// 普通用户 / 成员 → 自己所在归属组（站长 + 团队成员），站点在团队内共享
pub async fn site_users(claims: ValidatedClaims) -> ZapJsonResult {
    require_manageable(&claims)?;
    let pool = db::get_db_pool().await;
    let users: Vec<OwnerCandidate> = if !jwt::is_admin(&claims) && !jwt::is_reseller(&claims) {
        // 归属组内可选：本站长 + 其团队成员
        let gid = group_id_of(claims.id as i64).await;
        sqlx::query_as(
            "SELECT id, username, nickname FROM user
             WHERE status = 1 AND (id = ? OR (user_kind = ? AND owner_id = ?)) ORDER BY id",
        )
        .bind(gid)
        .bind(USER_KIND_MEMBER)
        .bind(gid)
        .fetch_all(pool)
        .await?
    } else if jwt::is_reseller(&claims) {
        // 自己优先展示，再补充名下客户
        let mut v: Vec<OwnerCandidate> =
            sqlx::query_as("SELECT id, username, nickname FROM user WHERE id = ? AND status = 1")
                .bind(claims.id as i64)
                .fetch_all(pool)
                .await?;
        let customers: Vec<OwnerCandidate> = sqlx::query_as(
            "SELECT id, username, nickname FROM user \
             WHERE status = 1 AND owner_id = ? AND (',' || roles || ',') LIKE '%,user,%' \
             ORDER BY id DESC",
        )
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await?;
        for c in customers {
            if !v.iter().any(|u| u.id == c.id) {
                v.push(c);
            }
        }
        v
    } else {
        sqlx::query_as(
            "SELECT id, username, nickname FROM user \
             WHERE status = 1 AND ((',' || roles || ',') LIKE '%,admin,%' \
                OR (',' || roles || ',') LIKE '%,reseller,%' \
                OR (',' || roles || ',') LIKE '%,user,%') \
             ORDER BY id DESC",
        )
        .fetch_all(pool)
        .await?
    };
    let list: Vec<Value> = users
        .iter()
        .map(|u| {
            json!({
                "id": u.id,
                "username": u.username,
                "nickname": u.nickname,
            })
        })
        .collect();
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": list,
    })))
}

/// 站点能力读取：返回当前操作者的实际能力（gates）与是否管理员。
/// admin / reseller 恒为全能力；普通用户反向代理取决于其套餐的 allow_proxy
/// （未绑定套餐时回退全局「默认套餐」）；自定义目录已全量开放。
pub async fn site_feature(claims: ValidatedClaims) -> ZapJsonResult {
    require_manageable(&claims)?;
    let g_proxy = gates_for(&claims).await;
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "is_admin": jwt::is_admin(&claims),
            "gates": {
                "proxy": g_proxy,
                // 自定义目录已全量开放（保留字段兼容旧前端）
                "custom_dir": true,
            },
        }
    })))
}

/// 站点已有目录浏览（供「选择已有目录」使用）：
/// admin/reseller 可指定归属用户浏览；普通用户只能浏览自己的家目录
pub async fn site_dirs_browse(
    claims: ValidatedClaims,
    Json(payload): Json<SiteDirsPayload>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    let op = is_operator(&claims);
    let owner = match payload.user_id {
        Some(uid) if op => {
            resolve_target_user(&claims, uid).await?;
            uid
        }
        Some(_) => {
            return Err(ZapError::New(
                -1,
                "普通用户只能浏览自己家目录下的目录".to_string(),
            ));
        }
        None => claims.id as i64,
    };
    let home = home_dir_of(owner).await?;
    if home.is_empty() {
        return Err(ZapError::New(
            -1,
            "该用户家目录尚未初始化：请先用「自动目录」创建并同步一次站点".to_string(),
        ));
    }
    let base = match payload.path.as_deref() {
        Some(p) => {
            let p = p.trim();
            if !p.starts_with('/') {
                return Err(ZapError::New(-1, "目录路径必须是绝对路径".to_string()));
            }
            if p.split('/').any(|s| s == "..") {
                return Err(ZapError::New(-1, "目录路径不允许包含 ..".to_string()));
            }
            if !(p == home || p.starts_with(&format!("{home}/"))) {
                return Err(ZapError::New(
                    -1,
                    format!("只能浏览归属用户家目录（{home}）下的目录"),
                ));
            }
            p.to_string()
        }
        None => home.clone(),
    };
    let resp = crate::zapexec::call(Request::FsBrowseDirs { base: base.clone() }).await?;
    if resp.code != 0 {
        return Err(ZapError::New(
            resp.code,
            format!("读取目录失败：{}", resp.message),
        ));
    }
    let dirs = resp
        .data
        .and_then(|d| d.get("dirs").cloned())
        .unwrap_or_else(|| json!([]));
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": { "home": home, "path": base, "dirs": dirs }
    })))
}

/// 新增站点（普通用户归属自动为当前登录用户；admin/reseller 需显式指定客户）
pub async fn site_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SiteAddPayload>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    let is_admin = jwt::is_admin(&claims);
    let is_reseller = jwt::is_reseller(&claims);
    let owner = if is_admin || is_reseller {
        payload
            .user_id
            .ok_or_else(|| ZapError::New(-1, "请选择站点的归属用户".to_string()))?
    } else {
        // 普通用户默认归自己；团队共享下可显式归属给同团队成员（前端未开放时恒为自己）
        match payload.user_id {
            Some(t) if t > 0 => t,
            _ => claims.id as i64,
        }
    };
    resolve_target_user(&claims, owner).await?;

    let domains = norm_domains(&payload.domains)?;
    let ips = norm_ips(&payload.ips);
    let name = fallback_name(payload.name.as_deref().unwrap_or(""), &domains);
    let status = payload.status.unwrap_or(1).clamp(0, 1);
    // 新建站点默认 running；显式传 status=0 时按 stopped 建（保持老行为）
    let run_state = if status == 0 {
        RUN_STOPPED
    } else {
        RUN_RUNNING
    };
    let remark = payload.remark.unwrap_or_default().trim().to_string();
    let php_instance = payload.php_instance.unwrap_or_default().trim().to_string();
    if !valid_php_instance(&php_instance) {
        return Err(ZapError::New(
            -1,
            "PHP 实例标识不合法（最长 120 字符，仅允许字母/数字/./_/-/@）".to_string(),
        ));
    }
    // 站点类型 / 伪静态 / 自定义目录 / upstream / location：白名单 + 功能开关门禁
    let site_type = {
        let t = payload.site_type.trim().to_lowercase();
        norm_site_type(if t.is_empty() { "php" } else { &t })?
    };
    let pseudo_static = {
        let p = payload.pseudo_static.trim().to_lowercase();
        if p.is_empty() { "none".to_string() } else { p }
    };
    validate_advanced_inputs(
        &claims,
        &payload.site_type,
        &pseudo_static,
        &payload.pseudo_custom,
        &payload.upstreams,
        &payload.locations,
    )
    .await?;
    // 自定义已有目录：在开启事务前向 root 侧验证「存在且位于家目录内」（避免事务内做外部 IO）
    let custom_web_root = if payload.web_root_custom {
        Some(resolve_custom_web_root(owner, payload.web_root.as_deref().unwrap_or("")).await?)
    } else {
        None
    };
    let pool = db::get_db_pool().await;
    // 归属用户的套餐配额：站点数上限 + 单站点域名数上限（均为 0 时表示不限）
    let pkg = crate::routers::package::package_of_user(owner).await;
    let max_domains = pkg.as_ref().map(|p| p.max_domains).unwrap_or(0);
    validate_site_fields(&name, &domains, &ips, &remark, max_domains)?;

    // 套餐限制：最大站点数（0 = 不限），达到上限时硬拦截
    if let Some(pkg) = &pkg
        && pkg.max_sites > 0
    {
        let used: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM site WHERE user_id = ?")
            .bind(owner)
            .fetch_one(pool)
            .await?;
        if used.0 >= pkg.max_sites {
            return Err(ZapError::New(
                -1,
                format!(
                    "已达套餐「{}」的站点上限 {} 个（当前 {} 个），无法继续创建",
                    pkg.name, pkg.max_sites, used.0
                ),
            ));
        }
    }
    let now = chrono::Local::now().timestamp();

    // SSL/TLS：主记录事务开始前先校验证书库绑定（存在 + 启用 + 归属一致），避免半提交
    let ssl_cert_id = payload.ssl_cert_id.unwrap_or(0).max(0);
    let force_https = payload.force_https;
    if ssl_cert_id > 0 {
        ensure_cert_bindable(ssl_cert_id, owner).await?;
    }

    let mut tx = pool.begin().await?;
    ensure_domains_unique(&mut tx, &domains, 0, jwt::is_admin(&claims)).await?;
    let r = sqlx::query(
        "INSERT INTO site (user_id, name, php_instance, status, run_state, remark, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(owner)
    .bind(&name)
    .bind(&php_instance)
    .bind(status)
    .bind(run_state)
    .bind(&remark)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let id = r.last_insert_rowid();
    for d in &domains {
        sqlx::query("INSERT INTO site_domain (site_id, domain) VALUES (?, ?)")
            .bind(id)
            .bind(d)
            .execute(&mut *tx)
            .await?;
    }
    for ip in &ips {
        sqlx::query("INSERT INTO site_ip (site_id, ip) VALUES (?, ?)")
            .bind(id)
            .bind(ip)
            .execute(&mut *tx)
            .await?;
    }
    // 站点文档根 / 日志目录：
    // - 自动目录：规划到归属用户家目录下 {home}/www/{name}-{id}（vhost 同步时由 zapexec 递归创建）
    // - 自定义目录：使用用户选择的归属家目录下已有目录（已在事务前验证过存在）
    let (web_root, log_root) = if let Some(cw) = &custom_web_root {
        let (_, lr) = site_dirs_for(owner, &name, id).await?;
        (cw.clone(), lr)
    } else {
        // 自动目录：支持用户指定 {home}/子路径（缺省走面板默认规划 www/{name}-{id}）；
        // 目录不存在时由 vhost 同步阶段的 ensure_web_root 递归创建
        auto_dirs_for(owner, &name, id, payload.web_root_sub.as_deref()).await?
    };
    if !web_root.is_empty() {
        sqlx::query("UPDATE site SET web_root = ?, log_root = ? WHERE id = ?")
            .bind(&web_root)
            .bind(&log_root)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    // 站点扩展档案（类型 / 伪静态 / upstream / location / 自定义目录标记 / SSL）
    if let Err(e) = save_profile(
        id,
        site_type,
        &pseudo_static,
        &payload.pseudo_custom,
        payload.web_root_custom,
        &payload.upstreams,
        &payload.locations,
        ssl_cert_id,
        force_https,
        payload.ssl_protocols.trim(),
        payload.ssl_ciphers.trim(),
        payload.ssl_prefer_server_ciphers,
        payload.ssl_http2,
    )
    .await
    {
        warn!("save site_profile failed (id={}): {}", id, e);
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "site_create",
        &format!("id={}", id),
        &format!(
            "user_id={} name={} domains={} ips={} php_instance={} site_type={} pseudo={} web_root_custom={}",
            owner,
            name,
            domains.join(","),
            ips.join(","),
            php_instance,
            site_type,
            pseudo_static,
            payload.web_root_custom
        ),
    )
    .await;
    info!(
        "site create: id={} user_id={} domains={:?}",
        id, owner, domains
    );

    Ok(Json(json!({
        "code": 0,
        "message": "站点添加成功",
        "data": { "id": id }
    })))
}

/// 更新站点（名称 / 多域名 / 多 IP / 状态 / 备注 / 归属用户转移）
pub async fn site_update(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SiteUpdatePayload>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    site_in_scope(&claims, payload.id).await?;

    let pool = db::get_db_pool().await;
    let row: Option<(i64, String, i32, String, String, String)> = sqlx::query_as(
        "SELECT user_id, name, status, remark, php_instance, web_root FROM site WHERE id = ?",
    )
    .bind(payload.id)
    .fetch_optional(pool)
    .await?;
    let Some((uid, old_name, mut status, mut remark, mut php_instance, old_web_root)) = row else {
        return Err(ZapError::New(-1, "站点不存在".to_string()));
    };

    // 站点扩展档案：未显式提交的字段沿用现值（老站点无档案则用默认值）
    let prof = load_profile(payload.id).await;
    let eff_type_raw = payload.site_type.clone().unwrap_or_else(|| prof.0.clone());
    let eff_type = norm_site_type(&eff_type_raw)?;
    // 空预设归一为 none（老档案/空提交不再显示空字符串）
    let eff_pseudo = {
        let v = payload
            .pseudo_static
            .clone()
            .unwrap_or_else(|| prof.1.clone());
        if v.trim().is_empty() {
            "none".to_string()
        } else {
            v
        }
    };
    let eff_pseudo_custom = payload
        .pseudo_custom
        .clone()
        .unwrap_or_else(|| prof.2.clone());
    let eff_custom = payload.web_root_custom.unwrap_or(prof.3);
    let eff_upstreams: Vec<UpstreamSpec> = match &payload.upstreams {
        Some(v) => v.clone(),
        None => parse_specs(&prof.4),
    };
    let eff_locations: Vec<LocationSpec> = match &payload.locations {
        Some(v) => v.clone(),
        None => parse_specs(&prof.5),
    };
    // SSL 绑定：None = 保持现值；Some(0) = 解绑；Some(id) = 绑定证书库证书
    let eff_ssl_cert_id = payload.ssl_cert_id.unwrap_or(prof.6).max(0);
    let eff_force_https = payload.force_https.unwrap_or(prof.7);
    // TLS 高级设置：None = 保持现值；空串 = 面板默认（协议 TLSv1.2+TLSv1.3 / 不输出套件）
    let eff_ssl_protocols = payload
        .ssl_protocols
        .clone()
        .unwrap_or_else(|| prof.8.clone());
    let eff_ssl_ciphers = payload
        .ssl_ciphers
        .clone()
        .unwrap_or_else(|| prof.9.clone());
    let eff_ssl_prefer = payload.ssl_prefer_server_ciphers.unwrap_or(prof.10);
    let eff_ssl_http2 = payload.ssl_http2.unwrap_or(prof.11);

    // 归属转移
    let new_owner = if let Some(nid) = payload.user_id {
        if !jwt::is_admin(&claims) && !jwt::is_reseller(&claims) {
            return Err(ZapError::New(-1, "普通用户无权转移站点归属".to_string()));
        }
        nid
    } else {
        uid
    };
    if new_owner != uid {
        resolve_target_user(&claims, new_owner).await?;
    }
    // SSL 证书归属校验需在最终归属确定后进行（存在 + 启用 + 归属与站点一致）
    if eff_ssl_cert_id > 0 {
        ensure_cert_bindable(eff_ssl_cert_id, new_owner).await?;
    }

    // 域名 / IP：不传则保留原值，传入则整体覆盖
    let domains = if let Some(ds) = &payload.domains {
        norm_domains(ds)?
    } else {
        sqlx::query_as::<_, (String,)>(
            "SELECT domain FROM site_domain WHERE site_id = ? ORDER BY id",
        )
        .bind(payload.id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| r.0)
        .collect()
    };
    let ips = if let Some(is) = &payload.ips {
        norm_ips(is)
    } else {
        sqlx::query_as::<_, (String,)>("SELECT ip FROM site_ip WHERE site_id = ? ORDER BY id")
            .bind(payload.id)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|r| r.0)
            .collect()
    };
    let name = match &payload.name {
        Some(n) => {
            let n = fallback_name(n, &domains);
            // 若编辑后仅剩域名而旧名称为空字符串（理论上不会发生），做一次兜底
            if n.is_empty() { old_name.clone() } else { n }
        }
        None => old_name.clone(),
    };
    // 运行状态：优先用 run_state；只传 status 时按兼容规则映射（0→stopped，1→running）
    let mut run_state = match &payload.run_state {
        Some(rs) => normalize_run_state(rs)
            .ok_or_else(|| {
                ZapError::New(
                    -1,
                    "运行状态仅支持 running / stopped / maintenance".to_string(),
                )
            })?
            .to_string(),
        None => sqlx::query_scalar::<_, String>("SELECT run_state FROM site WHERE id = ?")
            .bind(payload.id)
            .fetch_optional(pool)
            .await?
            .filter(|s| normalize_run_state(s).is_some())
            .unwrap_or_else(|| RUN_RUNNING.to_string()),
    };
    if let Some(s) = payload.status {
        status = s.clamp(0, 1);
        if payload.run_state.is_none() {
            run_state = if status == 0 {
                RUN_STOPPED.to_string()
            } else {
                RUN_RUNNING.to_string()
            };
        }
    }
    if let Some(rk) = &payload.remark {
        remark = rk.trim().to_string();
    }
    if let Some(p) = &payload.php_instance {
        let p = p.trim().to_string();
        if !valid_php_instance(&p) {
            return Err(ZapError::New(
                -1,
                "PHP 实例标识不合法（最长 120 字符，仅允许字母/数字/./_/-/@）".to_string(),
            ));
        }
        php_instance = p;
    }
    // 归属可能已转移，域名数配额按新归属用户的套餐计算
    let max_domains = crate::routers::package::package_of_user(new_owner)
        .await
        .map(|p| p.max_domains)
        .unwrap_or(0);
    validate_site_fields(&name, &domains, &ips, &remark, max_domains)?;

    // 门禁只针对“本次显式变更”：存量反代 / 自定义目录站点被普通用户增量编辑时不会被误拦截；
    // 自动目录子路径仅当“与当前文档根不同”才算显式变更（编辑自动目录站点时前端会回填相同子路径）
    let auto_sub_active = !eff_custom
        && payload
            .web_root_sub
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
        && {
            let home = home_dir_of(new_owner).await?;
            if home.is_empty() {
                true
            } else if let Some(rel0) = old_web_root.strip_prefix(&format!("{home}/")) {
                rel0.trim() != payload.web_root_sub.as_deref().map(str::trim).unwrap_or("")
            } else {
                // 当前文档根不在该家目录下（换属主 / 历史目录）：非空子路径视为迁移意图
                true
            }
        };
    validate_advanced_inputs(
        &claims,
        payload.site_type.as_deref().unwrap_or(""),
        payload.pseudo_static.as_deref().unwrap_or(""),
        payload.pseudo_custom.as_deref().unwrap_or(""),
        payload.upstreams.as_deref().unwrap_or_default(),
        payload.locations.as_deref().unwrap_or_default(),
    )
    .await?;
    // 自定义已有目录：归属/路径解析（root 侧验证，事务外执行）
    let new_custom_root: Option<String> = if eff_custom {
        let path = payload
            .web_root
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| (!old_web_root.is_empty()).then_some(old_web_root.as_str()))
            .ok_or_else(|| ZapError::New(-1, "自定义站点目录模式需要提供目录路径".to_string()))?;
        Some(resolve_custom_web_root(new_owner, path).await?)
    } else {
        None
    };
    // 自动目录自定义子路径（auto 模式且显式提交非空子路径 → 刷新文档根为 {home}/{sub}）
    let new_auto_dirs: Option<(String, String)> = if eff_custom || !auto_sub_active {
        None
    } else {
        Some(
            auto_dirs_for(
                new_owner,
                &name,
                payload.id,
                payload.web_root_sub.as_deref(),
            )
            .await?,
        )
    };
    // 目录需要刷新：进入/退出自定义模式、切属主、改名、自定义路径 / 子路径变化
    let dir_changed = eff_custom != prof.3
        || new_owner != uid
        || name != old_name
        || (eff_custom && new_custom_root.as_deref() != Some(old_web_root.as_str()))
        || matches!(&new_auto_dirs, Some((w, _)) if w != &old_web_root);

    let now = chrono::Local::now().timestamp();
    let mut tx = pool.begin().await?;
    let r = sqlx::query(
        "UPDATE site SET user_id = ?, name = ?, php_instance = ?, status = ?, run_state = ?, \
         remark = ?, updated_at = ? WHERE id = ?",
    )
    .bind(new_owner)
    .bind(&name)
    .bind(&php_instance)
    .bind(status)
    .bind(run_state)
    .bind(&remark)
    .bind(now)
    .bind(payload.id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        tx.rollback().await?;
        return Err(ZapError::New(-1, "站点不存在".to_string()));
    }
    // 域名整体覆盖：先校验唯一性，再重建
    if payload.domains.is_some() {
        ensure_domains_unique(&mut tx, &domains, payload.id, jwt::is_admin(&claims)).await?;
        sqlx::query("DELETE FROM site_domain WHERE site_id = ?")
            .bind(payload.id)
            .execute(&mut *tx)
            .await?;
        for d in &domains {
            sqlx::query("INSERT INTO site_domain (site_id, domain) VALUES (?, ?)")
                .bind(payload.id)
                .bind(d)
                .execute(&mut *tx)
                .await?;
        }
    }
    if payload.ips.is_some() {
        sqlx::query("DELETE FROM site_ip WHERE site_id = ?")
            .bind(payload.id)
            .execute(&mut *tx)
            .await?;
        for ip in &ips {
            sqlx::query("INSERT INTO site_ip (site_id, ip) VALUES (?, ?)")
                .bind(payload.id)
                .bind(ip)
                .execute(&mut *tx)
                .await?;
        }
    }
    // 站点目录跟随变更刷新（已有目录 / 自动目录自定义子路径 / 面板自动规划目录）
    if dir_changed {
        let (web_root, log_root) = if let Some(cw) = &new_custom_root {
            let (_, lr) = site_dirs_for(new_owner, &name, payload.id).await?;
            (cw.clone(), lr)
        } else if let Some((w, l)) = &new_auto_dirs {
            (w.clone(), l.clone())
        } else {
            site_dirs_for(new_owner, &name, payload.id).await?
        };
        if !web_root.is_empty() {
            sqlx::query("UPDATE site SET web_root = ?, log_root = ? WHERE id = ?")
                .bind(&web_root)
                .bind(&log_root)
                .bind(payload.id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;

    // 写回站点扩展档案（整体覆盖式提交，保证与 DB 现值一致）
    if let Err(e) = save_profile(
        payload.id,
        eff_type,
        &eff_pseudo,
        &eff_pseudo_custom,
        eff_custom,
        &eff_upstreams,
        &eff_locations,
        eff_ssl_cert_id,
        eff_force_https,
        &eff_ssl_protocols,
        &eff_ssl_ciphers,
        eff_ssl_prefer,
        eff_ssl_http2,
    )
    .await
    {
        warn!("save site_profile failed (id={}): {}", payload.id, e);
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "site_update",
        &format!("id={}", payload.id),
        &format!(
            "user_id={} name={} domains={} status={}",
            new_owner,
            name,
            domains.join(","),
            status
        ),
    )
    .await;
    info!("site update: id={} domains={:?}", payload.id, domains);

    Ok(Json(json!({
        "code": 0,
        "message": "更新成功"
    })))
}

/// 删除站点（可批量，连带删除其域名 / IP 绑定）
pub async fn site_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SiteDeletePayload>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    if payload.ids.is_empty() {
        return Err(ZapError::New(-1, "请选择要删除的站点".to_string()));
    }
    for id in &payload.ids {
        site_in_scope(&claims, *id).await?;
    }

    // 先清理 Nginx vhost（尽力而为，失败不阻塞删除）
    for id in &payload.ids {
        if let Ok(resp) = crate::zapexec::call(Request::SiteVhostRemove {
            site_id: *id,
            name: String::new(),
        })
        .await
            && resp.code != 0
        {
            tracing::warn!("remove vhost for site {} failed: {}", id, resp.message);
        }
    }

    // 勾选「同时删除网站数据」：删除前先取回目录规划（DB 记录删除后就没了），
    // 再让执行端 rm -rf 文档根与日志目录（日志随网站数据一起删，避免残留）。
    // 目录清理失败只记录告警，不阻塞站点本身删除（否则 zapexec 不可用时站点删不掉）。
    let mut data_removed = 0usize;
    if payload.remove_data {
        let dph = payload
            .ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let dsql = format!(
            "SELECT id, user_id, name, web_root, log_root FROM site WHERE id IN ({})",
            dph
        );
        let mut dq = sqlx::query_as::<_, (i64, i64, String, String, String)>(&dsql);
        for id in &payload.ids {
            dq = dq.bind(id);
        }
        let dir_rows = dq
            .fetch_all(db::get_db_pool().await)
            .await
            .unwrap_or_default();

        let mut web_roots: Vec<String> = Vec::new();
        let mut log_roots: Vec<String> = Vec::new();
        for (id, user_id, name, web_root, log_root) in dir_rows {
            // 库中为空时用统一规划值兜底（老站点或未同步过的站点）
            let (def_web, def_log) = site_dirs_for(user_id, &name, id).await.unwrap_or_default();
            let w = if web_root.trim().is_empty() {
                def_web
            } else {
                web_root.trim().to_string()
            };
            let l = if log_root.trim().is_empty() {
                def_log
            } else {
                log_root.trim().to_string()
            };
            if !w.is_empty() {
                web_roots.push(w);
            }
            if !l.is_empty() {
                log_roots.push(l);
            }
        }

        if !web_roots.is_empty() || !log_roots.is_empty() {
            let want = web_roots.len() + log_roots.len();
            match crate::zapexec::call(Request::SiteDataRemove {
                web_roots,
                log_roots,
            })
            .await
            {
                Ok(resp) if resp.code == 0 => {
                    data_removed = resp
                        .data
                        .as_ref()
                        .and_then(|d| d.get("count").and_then(|c| c.as_u64()))
                        .unwrap_or(want as u64) as usize;
                }
                Ok(resp) => {
                    tracing::warn!("remove site data failed: {}", resp.message);
                }
                Err(e) => {
                    tracing::warn!("remove site data error: {}", e);
                }
            }
        }
    }

    let placeholders = payload
        .ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let pool = db::get_db_pool().await;
    let mut tx = pool.begin().await?;

    let ssql = format!("DELETE FROM site WHERE id IN ({})", placeholders);
    let mut q = sqlx::query(&ssql);
    for id in &payload.ids {
        q = q.bind(id);
    }
    let r = q.execute(&mut *tx).await?;
    let deleted = r.rows_affected();

    let dsql = format!(
        "DELETE FROM site_domain WHERE site_id IN ({})",
        placeholders
    );
    let mut dq = sqlx::query(&dsql);
    for id in &payload.ids {
        dq = dq.bind(id);
    }
    dq.execute(&mut *tx).await?;

    let isql = format!("DELETE FROM site_ip WHERE site_id IN ({})", placeholders);
    let mut iq = sqlx::query(&isql);
    for id in &payload.ids {
        iq = iq.bind(id);
    }
    iq.execute(&mut *tx).await?;

    // 站点扩展档案（site_profile）随站点删除
    let psql = format!(
        "DELETE FROM site_profile WHERE site_id IN ({})",
        placeholders
    );
    let mut pq = sqlx::query(&psql);
    for id in &payload.ids {
        pq = pq.bind(id);
    }
    pq.execute(&mut *tx).await?;

    // 兜底：清理历史遗留的孤儿子表行（站点已不存在但域名/IP 仍残留），
    // 否则这些域名会一直"幽灵占用"，多租户下表现为新用户绑定不上。
    let _ = sqlx::query("DELETE FROM site_domain WHERE site_id NOT IN (SELECT id FROM site)")
        .execute(&mut *tx)
        .await;
    let _ = sqlx::query("DELETE FROM site_ip WHERE site_id NOT IN (SELECT id FROM site)")
        .execute(&mut *tx)
        .await;

    tx.commit().await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "site_delete",
        &format!("ids={:?}", payload.ids),
        &format!(
            "deleted={} remove_data={} data_dirs_removed={}",
            deleted, payload.remove_data, data_removed
        ),
    )
    .await;
    info!(
        "site delete: ids={:?} deleted={} remove_data={} data_dirs_removed={}",
        payload.ids, deleted, payload.remove_data, data_removed
    );

    let msg = if payload.remove_data {
        format!(
            "已删除 {} 个站点，并清理了 {} 个数据/日志目录",
            deleted, data_removed
        )
    } else {
        format!("已删除 {} 个站点", deleted)
    };

    Ok(Json(json!({
        "code": 0,
        "message": msg
    })))
}

#[derive(Debug, Deserialize)]
pub struct SiteSyncPayload {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct SiteStatePayload {
    pub id: i64,
    /// running（启动）/ stopped（停止）/ maintenance（维护页）
    pub state: String,
}

/// 站点启停 / 维护三态切换：更新状态 → 同步 vhost（停止撤软链、维护发维护页）→ 审计
pub async fn site_state(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SiteStatePayload>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    site_in_scope(&claims, payload.id).await?;

    let state = normalize_run_state(&payload.state).ok_or_else(|| {
        ZapError::New(
            -1,
            "运行状态仅支持 running / stopped / maintenance".to_string(),
        )
    })?;

    let pool = db::get_db_pool().await;
    let status = run_state_to_status(state);
    let now = chrono::Local::now().timestamp();
    let r = sqlx::query(
        "UPDATE site SET run_state = ?, status = ?, vhost_state = 'pending', updated_at = ? WHERE id = ?",
    )
    .bind(state)
    .bind(status)
    .bind(now)
    .bind(payload.id)
    .execute(pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ZapError::New(-1, "站点不存在".to_string()));
    }

    let (msg, data, name, _) = sync_one_site(payload.id).await?;

    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "site_state",
        &format!("id={}", payload.id),
        &format!("name={} state={} {}", name, state, msg),
    )
    .await;

    Ok(Json(json!({ "code": 0, "message": msg, "data": data })))
}

/// 将站点档案同步为 Nginx vhost：按域名/状态/PHP 实例渲染 conf → nginx -t → reload
pub async fn site_sync(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SiteSyncPayload>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    site_in_scope(&claims, payload.id).await?;
    let (msg, data, name, status) = match sync_one_site(payload.id).await {
        Ok(v) => v,
        Err(e) => return Err(e),
    };
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "site_sync",
        &format!("id={}", payload.id),
        &format!("name={} status={} {}", name, status, msg),
    )
    .await;
    info!("site sync: id={} status={}", payload.id, status);
    Ok(Json(json!({ "code": 0, "message": msg, "data": data })))
}

// ── vhost 同步状态机 ────────────────────────────────────────
// pending（同步中）→ synced（成功）/ failed（失败，原因写入 vhost_error）
// 面板可据此展示「已同步 / 同步中 / 失败（原因）」并提供重试入口。

async fn mark_sync_start(id: i64) {
    let pool = db::get_db_pool().await;
    let _ = sqlx::query("UPDATE site SET vhost_state = 'pending', vhost_error = '' WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await;
}

async fn mark_sync_result(id: i64, ok: bool, error: &str) {
    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();
    let state = if ok { "synced" } else { "failed" };
    let _ = sqlx::query(
        "UPDATE site SET vhost_state = ?, vhost_error = ?, vhost_synced_at = ? WHERE id = ?",
    )
    .bind(state)
    .bind(error)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await;
}

/// 单个站点全量同步（幂等，被 /site/sync、/site/sync_all、启停与数据迁移复用）。
/// 无论成功失败都会把结果写回 `vhost_state` / `vhost_error`。
pub(crate) async fn sync_one_site(
    id: i64,
) -> Result<(String, Option<serde_json::Value>, String, i32), ZapError> {
    mark_sync_start(id).await;
    match sync_one_site_inner(id).await {
        Ok(v) => {
            mark_sync_result(id, true, "").await;
            Ok(v)
        }
        Err(e) => {
            mark_sync_result(id, false, &e.to_string()).await;
            Err(e)
        }
    }
}

/// 同步执行体（不负责状态落库，由 sync_one_site 统一处理）
async fn sync_one_site_inner(
    id: i64,
) -> Result<(String, Option<serde_json::Value>, String, i32), ZapError> {
    let pool = db::get_db_pool().await;

    // 站点 + 归属用户（LEFT JOIN：站点可能无主 / 用户已删）
    let row: Option<SyncOneRow> = sqlx::query_as(
        "SELECT s.name, s.status, s.php_instance, s.web_root, s.log_root, s.run_state,
                u.id, u.home_dir, u.linux_user, u.fpm_pool, u.fpm_spec_ref, u.owner_id
         FROM site s LEFT JOIN user u ON u.id = s.user_id
         WHERE s.id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    let Some((
        name,
        status,
        php_instance,
        web_root,
        log_root,
        run_state,
        uid,
        uhome,
        _ulu,
        ufpm,
        uref,
        uowner,
    )) = row
    else {
        return Err(ZapError::New(-1, "站点不存在".to_string()));
    };
    // 域名 → server_name
    let mut domains = Vec::new();
    let dsql = "SELECT domain FROM site_domain WHERE site_id = ? ORDER BY id";
    let dq = sqlx::query_as::<_, (String,)>(dsql);
    for (d,) in dq.bind(id).fetch_all(pool).await? {
        let d = d.trim().to_string();
        if !d.is_empty() {
            domains.push(d);
        }
    }

    // 站点扩展档案（类型 / 伪静态 / 自定义目录 / upstream / location）
    let prof = load_profile(id).await;
    let s_type = norm_site_type(&prof.0)
        .map(str::to_string)
        .unwrap_or_else(|_| "php".to_string());
    let is_proxy = s_type == "proxy";

    // 运行实体准备（幂等）：站点必须绑定面板用户。
    // 每个面板用户对应一个 Linux 系统账号（nologin）：站点文件属主 = 该账号，
    // PHP-FPM pool 也以该账号运行（每用户 × 每 PHP 版本一个 pool）。
    let uid = uid.ok_or_else(|| {
        ZapError::New(
            -1,
            "站点未绑定面板用户，无法同步（请先为该站点指定归属用户）".to_string(),
        )
    })?;
    crate::routers::user::ensure_user_runtime(uid)
        .await
        .map_err(|e| ZapError::New(-1, e))?;
    let lu: Option<String> = sqlx::query_scalar("SELECT linux_user FROM user WHERE id = ?")
        .bind(uid)
        .fetch_one(pool)
        .await?;
    let linux_user = lu.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        ZapError::New(-1, format!("用户 {uid} 缺少 Linux 账号（linux_user 为空）"))
    })?;
    let owner_user = Some(linux_user.clone());

    // PHP 通道：先为归属用户同步「该用户 × 该 PHP 版本」专属 pool，
    // 通道固定为 /var/run/php-fpm-{linux_user}-{ver}.sock
    let php_socket = if is_proxy || php_instance.is_empty() {
        // 反向代理站点不绑定 PHP（有 PHP 实例也忽略，避免为不用的 pool 做联动）
        None
    } else {
        // 解析用户最终 pool 规格：存量自定义 fpm_pool → 模板/inherit(继承 reseller) → 全局默认
        let spec = crate::routers::fpm_spec::resolve_user_spec(
            ufpm.as_deref(),
            uref.as_deref().unwrap_or(""),
            uowner,
        )
        .await;
        let resp = crate::zapexec::call(Request::PhpPoolSync {
            php_instance: php_instance.clone(),
            linux_user: linux_user.clone(),
            home_dir: uhome.unwrap_or_default(),
            spec,
        })
        .await?;
        if resp.code != 0 {
            return Err(ZapError::New(
                resp.code,
                format!("PHP-FPM pool 同步失败：{}", resp.message),
            ));
        }
        // 前缀 unix: 是 nginx upstream / fastcgi_pass 的必需写法
        Some(format!(
            "unix:/var/run/php-fpm-{linux_user}-{}.sock",
            php_version_suffix(&php_instance)
        ))
    };

    let web_root_opt = (!web_root.trim().is_empty()).then_some(web_root);
    let log_root_opt = (!log_root.trim().is_empty()).then_some(log_root);
    let run_state = normalize_run_state(&run_state).unwrap_or(if status == 1 {
        RUN_RUNNING
    } else {
        RUN_STOPPED
    });

    // SSL/TLS：站点绑定证书库证书（prof.6 = cert id / prof.7 = force_https）。
    // 证书缺失或材料不全时降级为不启用 HTTPS（记 warn），不阻塞站点 HTTP 服务。
    let (ssl_fullchain, ssl_key) = if prof.6 > 0 {
        let cert: Option<(String, String, String)> = sqlx::query_as(
            "SELECT cert_content, key_content, ca_bundle FROM ssl_cert WHERE id = ?",
        )
        .bind(prof.6)
        .fetch_optional(pool)
        .await?;
        match cert {
            Some((leaf, key, ca)) if !leaf.trim().is_empty() && !key.trim().is_empty() => {
                let mut chain = leaf.trim().to_string();
                let ca = ca.trim();
                if !ca.is_empty() {
                    chain.push('\n');
                    chain.push_str(ca);
                }
                (Some(chain), Some(key.trim().to_string()))
            }
            Some(_) => {
                warn!(
                    "site {} 绑定的证书 {} 缺少证书/私钥材料，本次同步跳过 HTTPS",
                    id, prof.6
                );
                (None, None)
            }
            None => {
                warn!(
                    "site {} 绑定的证书 {} 不存在，本次同步跳过 HTTPS",
                    id, prof.6
                );
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    // 共享主机地址：面板基础设置「默认 IPv4/IPv6」非空时，站点 vhost 绑定 IP:80 / IP:443；
    // 留空则沿用通配监听（listen 80）。存量站点重新同步（保存/启停）即生效。
    let listen_ipv4 = server_env::conf_get(K_DEFAULT_IPV4).unwrap_or_default();
    let listen_ipv6 = server_env::conf_get(K_DEFAULT_IPV6).unwrap_or_default();

    let resp = crate::zapexec::call(Request::SiteVhostSync {
        site_id: id,
        name: name.clone(),
        domains,
        enabled: status == 1,
        mode: Some(run_state.to_string()),
        php_socket,
        web_root: web_root_opt,
        log_root: log_root_opt,
        owner_user,
        site_type: prof.0,
        pseudo_static: prof.1,
        pseudo_custom: prof.2,
        web_root_custom: prof.3,
        upstreams: parse_specs(&prof.4),
        locations: parse_specs(&prof.5),
        ssl_fullchain,
        ssl_key,
        force_https: prof.7,
        ssl_protocols: prof.8,
        ssl_ciphers: prof.9,
        ssl_prefer_server_ciphers: prof.10,
        ssl_http2: prof.11,
        listen_ipv4,
        listen_ipv6,
    })
    .await?;

    if resp.code != 0 {
        return Err(ZapError::New(
            resp.code,
            format!("vhost 同步失败：{}", resp.message),
        ));
    }

    info!(
        "site sync core: id={} status={} run_state={}",
        id, status, run_state
    );
    Ok((resp.message, resp.data, name, status))
}

/// 全部站点按当前模式重同步：vhost 模式开关切换后的「再同步」入口
pub async fn site_sync_all(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    let pool = db::get_db_pool().await;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM site ORDER BY id")
        .fetch_all(pool)
        .await?;
    if ids.is_empty() {
        return Ok(Json(json!({ "code": 0, "message": "没有需要同步的站点" })));
    }
    let mut ok = 0usize;
    let mut fails: Vec<String> = Vec::new();
    for sid in ids {
        match sync_one_site(sid).await {
            Ok(_) => ok += 1,
            Err(e) => fails.push(format!("站点 #{}：{}", sid, e)),
        }
    }
    let fail = fails.len();
    let summary = if fail == 0 {
        format!("已按当前模式重同步 {} 个站点", ok)
    } else {
        let detail = fails.iter().take(3).cloned().collect::<Vec<_>>().join("; ");
        format!("成功 {} 个，失败 {} 个（{}…）", ok, fail, detail)
    };
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "site_sync_all",
        "all",
        &summary,
    )
    .await;
    info!("site sync all: {}", summary);
    if fail > 0 {
        return Err(ZapError::New(-1, format!("部分站点同步失败：{}", summary)));
    }
    Ok(Json(json!({ "code": 0, "message": summary })))
}

/// PHP 实例 → 版本后缀：php8.3 → 8.3，php74 → 74
fn php_version_suffix(php_instance: &str) -> String {
    php_instance.trim_start_matches("php").to_string()
}

// ── AppStore provision：为建站包准备站点 ────────────────────

/// provision 查询「已存在站点」时的一行：(id, web_root, php_instance, 面板用户名, Linux 账号)
type ProvisionSiteRow = (i64, String, String, Option<String>, Option<String>);

/// provision 交付给安装脚本的站点信息。
#[derive(Debug, Clone)]
pub(crate) struct ProvisionedSite {
    pub id: i64,
    pub domain: String,
    /// 站点文档根（建站包的部署目标，脚本只能写这里）
    pub root: String,
    /// 归属面板用户名
    pub owner: String,
    /// 站点文件属主（Linux 账号，与降权脚本的运行账号一致）
    pub linux_user: String,
    pub php_instance: String,
    /// PHP 通道（unix socket）；未绑定 PHP 实例时为 None
    pub php_socket: Option<String>,
}

/// 组装交付信息：PHP 通道按「用户 × PHP 版本」专属 pool 规则推导。
fn provisioned_site_of(
    id: i64,
    domain: String,
    root: String,
    php_instance: String,
    owner: Option<String>,
    linux_user: Option<String>,
) -> ProvisionedSite {
    let linux_user = linux_user.unwrap_or_default();
    let php_socket = if php_instance.is_empty() || linux_user.is_empty() {
        None
    } else {
        Some(format!(
            "unix:/var/run/php-fpm-{linux_user}-{}.sock",
            php_version_suffix(&php_instance)
        ))
    };
    ProvisionedSite {
        id,
        domain,
        root,
        owner: owner.unwrap_or_default(),
        linux_user,
        php_instance,
        php_socket,
    }
}

/// PHP 实例标识归一：`8.3` / `php83` → `php83`（与 PHP 应用安装时写入的 instance 一致）。
/// 未指定时取面板默认 PHP（`server_env` 的 php_default）。
fn normalize_php_instance(raw: Option<&str>) -> String {
    let s = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| crate::zap::server_env::conf_get("php_default"))
        .unwrap_or_default();
    if s.is_empty() {
        return String::new();
    }
    let ver = s.trim_start_matches("php").replace('.', "");
    if ver.is_empty() {
        String::new()
    } else {
        format!("php{ver}")
    }
}

/// AppStore provision：为建站类包准备站点。
///
/// - 域名已存在（且落在操作者管理范围内）→ 直接复用，不重复建站；
/// - 不存在且 `mode=create`（缺省）→ 按 `/site/add` 的同一套规则新建 PHP 站点
///   （套餐站点数配额 → 域名唯一 → 目录规划 → 档案 → vhost 同步）；
/// - 不存在且 `mode=require` → 报错，提示先建站。
pub(crate) async fn provision_site(
    claims: &jwt::Claims,
    domain: &str,
    mode: Option<&str>,
    php_instance: Option<&str>,
    rewrite: Option<&str>,
) -> Result<ProvisionedSite, ZapError> {
    require_manageable(claims)?;
    let domains = norm_domains(&[domain.to_string()])?;
    let d = match domains.first() {
        Some(d) => d.clone(),
        None => return Err(ZapError::New(-1, "站点域名不能为空".to_string())),
    };

    let owner = claims.id as i64;
    let pool = db::get_db_pool().await;
    let gid = group_id_of(owner).await;

    // 1) 已有站点 → 复用（同一个域名重复安装不会建出第二个站）
    let sql = format!(
        "SELECT s.id, s.web_root, s.php_instance, u.username, u.linux_user \
         FROM site s JOIN site_domain d ON d.site_id = s.id \
         LEFT JOIN user u ON u.id = s.user_id \
         WHERE d.domain = ? AND {} LIMIT 1",
        group_scope_cond("s.user_id")
    );
    let found: Option<ProvisionSiteRow> = sqlx::query_as(&sql)
        .bind(&d)
        .bind(gid)
        .bind(USER_KIND_MEMBER)
        .bind(gid)
        .fetch_optional(pool)
        .await?;
    if let Some((id, root, php, username, linux_user)) = found {
        return Ok(provisioned_site_of(id, d, root, php, username, linux_user));
    }

    if matches!(
        mode.map(|m| m.trim().to_ascii_lowercase()).as_deref(),
        Some("require")
    ) {
        return Err(ZapError::New(
            -1,
            format!(
                "站点 {d} 不存在：请先在「站点」中创建该站点，或把包改为 provision.site.mode: create"
            ),
        ));
    }

    // 2) 新建站点：套餐站点数配额（0 = 不限）
    let pkg = crate::routers::package::package_of_user(owner).await;
    if let Some(pkg) = &pkg
        && pkg.max_sites > 0
    {
        let used: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM site WHERE user_id = ?")
            .bind(owner)
            .fetch_one(pool)
            .await?;
        if used.0 >= pkg.max_sites {
            return Err(ZapError::New(
                -1,
                format!(
                    "已达套餐「{}」的站点上限 {} 个（当前 {} 个），无法为建站包创建站点",
                    pkg.name, pkg.max_sites, used.0
                ),
            ));
        }
    }

    let name = fallback_name("", &domains);
    let php = normalize_php_instance(php_instance);
    let pseudo = rewrite
        .map(|r| r.trim().to_ascii_lowercase())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "none".to_string());
    if !PSEUDO_PRESETS.contains(&pseudo.as_str()) {
        return Err(ZapError::New(-1, format!("伪静态预设不支持：{pseudo}")));
    }
    let now = chrono::Local::now().timestamp();
    let mut tx = pool.begin().await?;
    ensure_domains_unique(&mut tx, &domains, 0, jwt::is_admin(claims)).await?;
    let r = sqlx::query(
        "INSERT INTO site (user_id, name, php_instance, status, run_state, remark, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(owner)
    .bind(&name)
    .bind(&php)
    .bind(1i64)
    .bind(RUN_RUNNING)
    .bind("")
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let id = r.last_insert_rowid();
    for dm in &domains {
        sqlx::query("INSERT INTO site_domain (site_id, domain) VALUES (?, ?)")
            .bind(id)
            .bind(dm)
            .execute(&mut *tx)
            .await?;
    }
    let (web_root, log_root) = auto_dirs_for(owner, &name, id, None).await?;
    if !web_root.is_empty() {
        sqlx::query("UPDATE site SET web_root = ?, log_root = ? WHERE id = ?")
            .bind(&web_root)
            .bind(&log_root)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let _ = save_profile(
        id,
        "php",
        &pseudo,
        "",
        false,
        &[],
        &[],
        0,
        false,
        "",
        "",
        false,
        false,
    )
    .await;
    // vhost 同步失败不阻断：站点已入库，用户可在站点页重新同步
    if let Err(e) = sync_one_site(id).await {
        warn!("provision 建站后 vhost 同步失败 (id={id}): {e}");
    }

    let row: Option<(String, String)> =
        sqlx::query_as("SELECT username, linux_user FROM user WHERE id = ?")
            .bind(owner)
            .fetch_optional(pool)
            .await?;
    let (username, linux_user) = match row {
        Some((u, l)) => (Some(u), Some(l)),
        None => (None, None),
    };
    Ok(provisioned_site_of(
        id, d, web_root, php, username, linux_user,
    ))
}

// ── 站点日志 / 流量分析 ──────────────────────────────────────
// 日志读写一律经 zapexec（日志文件归 www:www，面板进程未必有权限直读）。

/// 取站点日志目录（面板规划 {home}/logs/{site_id}-{name}）
async fn log_root_of(site_id: i64) -> Result<String, ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(String,)> = sqlx::query_as("SELECT log_root FROM site WHERE id = ?")
        .bind(site_id)
        .fetch_optional(pool)
        .await?;
    match row {
        Some((root,)) if !root.trim().is_empty() => Ok(root),
        _ => Err(ZapError::New(-1, "该站点尚未配置日志目录".to_string())),
    }
}

fn default_log_lines() -> usize {
    200
}

fn default_traffic_days() -> u32 {
    30
}

#[derive(Debug, Deserialize)]
pub struct SiteLogsQuery {
    pub id: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub archive: String,
    #[serde(default = "default_log_lines")]
    pub lines: usize,
    #[serde(default)]
    pub keyword: String,
    #[serde(default)]
    pub status: String,
}

/// GET /api/site/logs：站点日志尾部行（当前日志或归档，支持关键词 / 状态码过滤）
pub async fn site_logs(claims: ValidatedClaims, Query(q): Query<SiteLogsQuery>) -> ZapJsonResult {
    require_manageable(&claims)?;
    site_in_scope(&claims, q.id).await?;
    let log_root = log_root_of(q.id).await?;
    let resp = crate::zapexec::call(Request::SiteLogRead {
        log_root,
        kind: q.kind,
        archive: q.archive,
        lines: q.lines,
        keyword: q.keyword,
        status: q.status,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": resp.data.unwrap_or_default(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct SiteLogsArchivesQuery {
    pub id: i64,
}

/// GET /api/site/logs/archives：当前日志与历史归档列表（供查看 / 下载历史）
pub async fn site_logs_archives(
    claims: ValidatedClaims,
    Query(q): Query<SiteLogsArchivesQuery>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    site_in_scope(&claims, q.id).await?;
    let log_root = log_root_of(q.id).await?;
    let resp = crate::zapexec::call(Request::SiteLogList { log_root }).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": resp.data.unwrap_or_default(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct SiteLogsClearPayload {
    pub id: i64,
    /// access | error；空 = 两者都清空
    #[serde(default)]
    pub kind: String,
}

/// POST /api/site/logs/clear：清空站点当前日志
pub async fn site_logs_clear(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SiteLogsClearPayload>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    site_in_scope(&claims, payload.id).await?;
    let log_root = log_root_of(payload.id).await?;
    let resp = crate::zapexec::call(Request::SiteLogClear {
        log_root,
        kind: payload.kind.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    let kind = if payload.kind.trim().is_empty() {
        "all".to_string()
    } else {
        payload.kind
    };
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "site_logs_clear",
        &format!("id={}", payload.id),
        &format!("kind={}", kind),
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": resp.data.unwrap_or_default(),
    })))
}

/// POST /api/site/logs/rotate：手动轮转该站点日志（按天切割 + 归档）
pub async fn site_logs_rotate(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SiteLogsClearPayload>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    site_in_scope(&claims, payload.id).await?;
    let log_root = log_root_of(payload.id).await?;
    // 切割前先把当前日志增量统计完，避免归档部分漏计
    crate::zap::usage::collect_bandwidth().await;
    let resp = crate::zapexec::call(Request::SiteLogRotate {
        log_roots: vec![log_root],
        keep_days: crate::zap::logrotate::KEEP_DAYS,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "site_logs_rotate",
        &format!("id={}", payload.id),
        "manual",
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": resp.data.unwrap_or_default(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct SiteTrafficQuery {
    pub id: i64,
    #[serde(default = "default_traffic_days")]
    pub days: u32,
}

/// GET /api/site/traffic：站点流量分析（按天曲线 + Top URL + 汇总）
pub async fn site_traffic(
    claims: ValidatedClaims,
    Query(q): Query<SiteTrafficQuery>,
) -> ZapJsonResult {
    require_manageable(&claims)?;
    site_in_scope(&claims, q.id).await?;
    let pool = db::get_db_pool().await;
    let days = if q.days == 0 { 30 } else { q.days.min(365) };
    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let since = (chrono::Local::now() - chrono::Duration::days(days as i64 - 1))
        .format("%Y%m%d")
        .to_string();

    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT day, bytes, requests FROM site_traffic_daily \
         WHERE site_id = ? AND day >= ? ORDER BY day",
    )
    .bind(q.id)
    .bind(&since)
    .fetch_all(pool)
    .await?;
    let map: std::collections::HashMap<String, (i64, i64)> =
        rows.into_iter().map(|(d, b, r)| (d, (b, r))).collect();

    // 补齐连续日期（无流量的日子补 0，前端曲线才不会出现断档）
    let mut daily: Vec<serde_json::Value> = Vec::new();
    for i in 0..days {
        let d = (chrono::Local::now() - chrono::Duration::days(days as i64 - 1 - i as i64))
            .format("%Y%m%d")
            .to_string();
        let (b, r) = map.get(&d).copied().unwrap_or((0, 0));
        daily.push(json!({ "day": d, "bytes": b, "requests": r }));
    }

    let top: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT path, SUM(hits), SUM(bytes) FROM site_traffic_path \
         WHERE site_id = ? AND day >= ? GROUP BY path ORDER BY SUM(hits) DESC, SUM(bytes) DESC LIMIT 20",
    )
    .bind(q.id)
    .bind(&since)
    .fetch_all(pool)
    .await?;
    let top: Vec<serde_json::Value> = top
        .into_iter()
        .map(|(p, h, b)| json!({ "path": p, "hits": h, "bytes": b }))
        .collect();

    let summary: Option<(i64, i64, String)> = sqlx::query_as(
        "SELECT traffic_month_bytes, traffic_total_bytes, traffic_month FROM site WHERE id = ?",
    )
    .bind(q.id)
    .fetch_optional(pool)
    .await?;
    let (month_bytes, total_bytes, month) = summary.unwrap_or((0, 0, String::new()));

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "daily": daily,
            "top": top,
            "today_bytes": map.get(&today).map(|v| v.0).unwrap_or(0),
            "month_bytes": month_bytes,
            "month": month,
            "total_bytes": total_bytes,
        },
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_normalization_rules() {
        // 粘贴整条 URL / 大写 / 结尾点 / 端口 都能归一化到同一结果
        for raw in [
            "A.com",
            " a.com ",
            "a.com.",
            "http://a.com",
            "https://A.com/path?x=1",
            "http://a.com:8080/x",
        ] {
            assert_eq!(normalize_domain(raw).unwrap(), "a.com", "输入: {raw}");
        }
        // 泛域名保留写法
        assert_eq!(normalize_domain("*.A.com").unwrap(), "*.a.com");
        // 中文域名转 punycode
        let puny = normalize_domain("中文.com").unwrap();
        assert!(puny.starts_with("xn--"), "应转为 punycode: {puny}");
        assert!(valid_domain(&puny));
        // 空串与纯路径
        assert!(normalize_domain("   ").is_err());
        assert!(normalize_domain("http://").is_err());
    }

    #[test]
    fn domain_validity_rules() {
        assert!(valid_domain("a.com"));
        assert!(valid_domain("www.a.com"));
        assert!(valid_domain("*.a.com"));
        assert!(valid_domain("my-site.example.com"));
        // 非法：空标签 / 点开头结尾 / 连字符开头结尾 / 只有星号 / 非法字符
        assert!(!valid_domain("a..com"));
        assert!(!valid_domain(".a.com"));
        assert!(!valid_domain("a.com."));
        assert!(!valid_domain("-a.com"));
        assert!(!valid_domain("a-.com"));
        assert!(!valid_domain("*"));
        assert!(!valid_domain("a b.com"));
    }

    #[test]
    fn conflict_keys_and_wildcards() {
        // a.com 与 www.a.com 互为冲突键
        assert_eq!(
            domain_match_keys("a.com"),
            vec!["a.com".to_string(), "www.a.com".to_string()]
        );
        assert_eq!(
            domain_match_keys("www.a.com"),
            vec!["www.a.com".to_string(), "a.com".to_string()]
        );

        // 泛域名覆盖：*.a.com 覆盖 a.com / b.a.com / x.b.a.com，不覆盖 b.com
        assert!(wildcard_covers("a.com", "a.com"));
        assert!(wildcard_covers("a.com", "b.a.com"));
        assert!(wildcard_covers("a.com", "x.b.a.com"));
        assert!(!wildcard_covers("a.com", "b.com"));
        assert!(!wildcard_covers("a.com", "ab.com"));
        // 两个泛域名覆盖范围相交即冲突
        assert!(wildcard_covers("a.com", "*.a.com"));
        assert!(wildcard_covers("a.com", "*.b.a.com"));
        assert!(!wildcard_covers("a.com", "*.b.com"));

        // LIKE 转义：域名里的 _ 不应被当作通配符
        assert_eq!(like_escape("a_b.com"), "a\\_b.com");
    }

    #[test]
    fn domain_limit_follows_package() {
        // 套餐未配置（0）→ 用兜底值；配置了 → 用套餐值
        assert_eq!(domain_limit_of(0), DEFAULT_MAX_DOMAINS);
        assert_eq!(domain_limit_of(-5), DEFAULT_MAX_DOMAINS);
        assert_eq!(domain_limit_of(10), 10);
    }

    #[test]
    fn run_state_normalization_and_status_mapping() {
        assert_eq!(normalize_run_state("running").unwrap(), "running");
        assert_eq!(normalize_run_state(" STOP ").unwrap(), "stopped");
        assert_eq!(normalize_run_state("Maintenance").unwrap(), "maintenance");
        assert!(normalize_run_state("paused").is_none(), "非法状态应被拒绝");

        assert_eq!(run_state_to_status("running"), 1);
        assert_eq!(run_state_to_status("maintenance"), 1);
        assert_eq!(run_state_to_status("stopped"), 0);
    }

    #[test]
    fn conflict_message_hides_holder_for_normal_user() {
        let admin = conflict_err("a.com", "www.a.com", 12, true);
        let user = conflict_err("a.com", "www.a.com", 12, false);
        let admin_msg = format!("{admin}");
        let user_msg = format!("{user}");
        assert!(
            admin_msg.contains("id=12"),
            "管理员应看到占用方: {admin_msg}"
        );
        assert!(
            !user_msg.contains("id=12"),
            "普通用户不应看到占用方: {user_msg}"
        );
    }
}
