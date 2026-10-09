// SPDX-License-Identifier: AGPL-3.0-only
//! 套餐（Packages）管理 ：
//! Admin / Reseller Resource / Capability Management，
//! 客户即继承该套餐的配额与能力开关。
//!
//! 限制项：
//! - `disk_quota_mb`     磁盘配额（MB，0 = 不限），创建/变更用户时下发到系统 quota
//! - `max_sites`         最大站点数（0 = 不限），创建站点时硬拦截
//! - `max_domains`       单站点最大域名数（0 = 不限），创建/编辑站点时硬拦截
//! - `max_bandwidth_mb`  月流量上限（MB，0 = 不限；面板暂无流量统计，仅记录与展示）
//! - `max_mysql_dbs`     MySQL / MariaDB 数据库数量（0 = 不限），建库时硬拦截
//! - `max_pgsql_dbs`     PostgreSQL 数据库数量（0 = 不限；面板暂无 PG 模块，仅记录与展示）
//! - `max_ftp_users`     FTP 账号数量（0 = 不限；面板暂无 FTP 模块，仅记录与展示）
//! - `fpm_spec_ref`      PHP-FPM 规格模板名（'' = 面板默认）
//! - `allow_ssh`         是否允许使用 SSH 终端
//! - `allow_proxy`       是否允许普通用户创建/编辑「反向代理」站点（upstream / location）
//! - `allow_php`         是否允许该套餐的用户建 PHP 站点（默认开放；admin/reseller 恒可）
//! - `allow_waf`         是否允许该套餐的用户为站点开启 WAF / 限速 / 限并发
//!                       （默认关闭，且仍要求全局 ModSecurity 已安装并启用）
//! - `allow_docker`      是否允许该套餐的用户使用容器功能（默认关闭），
//!                       且**仅当容器运行时为 Podman 时**才对非管理员生效
//! - `allow_apps`        是否允许部署应用（默认关闭）
//! - `app_types`         允许部署的应用类型（逗号分隔，如 `python,nodejs`）；空 = 不限
//! - `max_apps`          每个站点可部署的应用数量上限（0 = 不限）
//! - `app_port_span`     每个用户分到的端口个数（0 = 不限）：端口段由「基准 + 用户ID × N」自动算出
//! - `app_max_total`     该用户全部站点合计可部署的应用数量上限（0 = 不限）
//!
//! 归属：`owner_id = 0` 为全局套餐（admin 维护，所有人可用）；
//! `owner_id` 为某 admin / reseller 自身 id 时，为仅该账号可见的私有套餐
//! （reseller 自建的私有套餐，其资源 / 能力不得超过 reseller 自身套餐的上限）。
//!
//! 端点：
//! - GET  /system/package/list    套餐列表（admin 全量；reseller 全局 + 自己名下）
//! - POST /system/package/add     新增
//! - POST /system/package/update  修改
//! - POST /system/package/delete  删除（被客户引用时拒绝）

use std::net::SocketAddr;

use axum::Json;
use axum::extract::Extension;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::{ZapError, ZapJsonResult, audit};

const MAX_NAME_LEN: usize = 64;

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct PackageRow {
    pub id: i64,
    pub name: String,
    pub remark: String,
    pub disk_quota_mb: i64,
    pub max_sites: i64,
    /// 单站点最大域名数（0 = 不限）
    pub max_domains: i64,
    pub max_bandwidth_mb: i64,
    /// MySQL / MariaDB 数据库数量（0 = 不限）
    pub max_mysql_dbs: i64,
    /// PostgreSQL 数据库数量（0 = 不限，仅记录）
    pub max_pgsql_dbs: i64,
    /// FTP 账号数量（0 = 不限，仅记录）
    pub max_ftp_users: i64,
    pub fpm_spec_ref: String,
    pub allow_ssh: i32,
    /// 允许普通用户使用反向代理（upstream / location）
    pub allow_proxy: i32,
    /// 允许该套餐的用户建 PHP 站点（默认开放；关闭后普通用户不能新建/编辑 PHP 站点）
    pub allow_php: i32,
    /// 允许该套餐的用户使用容器功能；**仅在容器运行时为 Podman 时生效**
    /// （Docker 下容器由 root 跑、没有隔离，一律不对非管理员开放）
    pub allow_docker: i32,
    /// 允许该套餐的用户为站点开启 WAF / 限速 / 限并发（仍要求全局 WAF 已安装并启用）
    pub allow_waf: i32,
    /// 允许使用应用管理（Application Manager）：部署 python / nodejs 等长驻进程
    pub allow_apps: i32,
    /// 允许部署的应用类型（逗号分隔，如 `python,nodejs`）；空 = 不限
    pub app_types: String,
    /// 每个站点可部署的应用数量上限（0 = 不限）
    pub max_apps: i64,
    /// 每个用户分到的端口个数（0 = 不限）：端口段由「基准 + 用户ID × N」自动算出
    pub app_port_span: i64,
    /// 该用户全部站点合计可部署的应用数量上限（0 = 不限）
    pub app_max_total: i64,
    pub owner_id: i64,
    pub status: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLS: &str = "id, name, remark, disk_quota_mb, max_sites, max_domains, max_bandwidth_mb, \
                    max_mysql_dbs, max_pgsql_dbs, max_ftp_users, \
                    fpm_spec_ref, allow_ssh, allow_proxy, allow_php, allow_docker, allow_waf, \
                    allow_apps, app_types, max_apps, app_port_span, app_max_total, \
                    owner_id, status, \
                    created_at, updated_at";

fn validate_name(raw: &str) -> Result<String, ZapError> {
    let n = raw.trim();
    if n.is_empty() {
        return Err(ZapError::New(-1, "套餐名不能为空".to_string()));
    }
    if n.chars().count() > MAX_NAME_LEN {
        return Err(ZapError::New(
            -1,
            format!("套餐名最长 {MAX_NAME_LEN} 个字符"),
        ));
    }
    Ok(n.to_string())
}

/// 数值限制项：不小于 0，0 表示「不限」
fn validate_limit(v: i64, label: &str) -> Result<i64, ZapError> {
    if v < 0 {
        return Err(ZapError::New(
            -1,
            format!("{label} 不能为负数（0 表示不限）"),
        ));
    }
    Ok(v)
}

/// reseller 自建（子）套餐的待校验数值 / 能力集合。
struct ResellerSubVals {
    disk_quota_mb: i64,
    max_sites: i64,
    max_domains: i64,
    max_bandwidth_mb: i64,
    max_mysql_dbs: i64,
    max_pgsql_dbs: i64,
    max_ftp_users: i64,
    app_max_total: i64,
    max_apps: i64,
    app_port_span: i64,
    allow_ssh: i32,
    allow_proxy: i32,
    allow_php: i32,
    allow_docker: i32,
    allow_waf: i32,
    allow_apps: i32,
    app_types: String,
}

/// reseller 创建的子套餐，其各项资源 / 能力不得超过 reseller 自身套餐（父套餐）的允许范围：
/// - 数值上限：父套餐为 0（不限）时子套餐可任意；否则子套餐不得超过父套餐对应值。
/// - 能力开关：父套餐未开启的能力，子套餐不得开启。
/// - 应用类型：子套餐类型必须是父套餐类型的子集。
fn enforce_reseller_subpackage(parent: &PackageRow, v: &ResellerSubVals) -> Result<(), ZapError> {
    let cap = |label: &str, child: i64, pmax: i64| -> Result<(), ZapError> {
        if pmax > 0 && child > pmax {
            return Err(ZapError::New(
                -1,
                format!("{label}不能超过你自身套餐的 {pmax}（父套餐上限）"),
            ));
        }
        Ok(())
    };
    cap("磁盘配额(MB)", v.disk_quota_mb, parent.disk_quota_mb)?;
    cap("最大站点数", v.max_sites, parent.max_sites)?;
    cap("单站点最大域名数", v.max_domains, parent.max_domains)?;
    cap(
        "月流量上限(MB)",
        v.max_bandwidth_mb,
        parent.max_bandwidth_mb,
    )?;
    cap("MySQL 数据库数量", v.max_mysql_dbs, parent.max_mysql_dbs)?;
    cap(
        "PostgreSQL 数据库数量",
        v.max_pgsql_dbs,
        parent.max_pgsql_dbs,
    )?;
    cap("FTP 账号数量", v.max_ftp_users, parent.max_ftp_users)?;
    cap("每用户应用总数", v.app_max_total, parent.app_max_total)?;
    cap("每站点应用数", v.max_apps, parent.max_apps)?;
    cap("每用户端口数", v.app_port_span, parent.app_port_span)?;

    let flag = |label: &str, child: i32, p: i32| -> Result<(), ZapError> {
        if p == 0 && child == 1 {
            return Err(ZapError::New(
                -1,
                format!("{label}未在你自身套餐中开启，不能授予客户"),
            ));
        }
        Ok(())
    };
    flag("SSH 终端", v.allow_ssh, parent.allow_ssh)?;
    flag("反向代理", v.allow_proxy, parent.allow_proxy)?;
    flag("PHP 站点", v.allow_php, parent.allow_php)?;
    flag("Docker 容器", v.allow_docker, parent.allow_docker)?;
    flag("WAF", v.allow_waf, parent.allow_waf)?;
    flag("应用托管", v.allow_apps, parent.allow_apps)?;

    if !parent.app_types.is_empty() {
        let pset: std::collections::HashSet<&str> =
            parent.app_types.split(',').map(|s| s.trim()).collect();
        for t in v.app_types.split(',') {
            let t = t.trim();
            if !t.is_empty() && !pset.contains(t) {
                return Err(ZapError::New(
                    -1,
                    format!("应用类型「{t}」未在你自身套餐中开启，不能授予客户"),
                ));
            }
        }
    }
    Ok(())
}

/// 套餐对操作者是否可见：admin 全量；reseller 仅全局套餐（owner_id=0）与自己名下
fn visible(r: &PackageRow, is_admin: bool, actor_id: i64) -> bool {
    // 管理员可见全部；reseller 仅可见自己创建的套餐（owner_id=自己），
    // 看不到 admin 创建的全局/私有套餐。
    is_admin || r.owner_id == actor_id
}

/// 统计各套餐被引用的客户数（key = package_id）
async fn usage_counts() -> std::collections::HashMap<i64, i64> {
    let pool = db::get_db_pool().await;
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT package_id, COUNT(*) FROM user WHERE package_id > 0 GROUP BY package_id",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter().collect()
}

fn row_json(r: &PackageRow, users_count: i64) -> Value {
    json!({
        "id": r.id,
        "name": r.name,
        "remark": r.remark,
        "disk_quota_mb": r.disk_quota_mb,
        "max_sites": r.max_sites,
        "max_domains": r.max_domains,
        "max_bandwidth_mb": r.max_bandwidth_mb,
        "max_mysql_dbs": r.max_mysql_dbs,
        "max_pgsql_dbs": r.max_pgsql_dbs,
        "max_ftp_users": r.max_ftp_users,
        "fpm_spec_ref": r.fpm_spec_ref,
        "allow_ssh": r.allow_ssh == 1,
        "allow_proxy": r.allow_proxy == 1,
        "allow_php": r.allow_php == 1,
        "allow_docker": r.allow_docker == 1,
        "allow_waf": r.allow_waf == 1,
        "allow_apps": r.allow_apps == 1,
        "app_types": r.app_types,
        "max_apps": r.max_apps,
        "app_port_span": r.app_port_span,
        "app_max_total": r.app_max_total,
        "owner_id": r.owner_id,
        "status": r.status,
        "users_count": users_count,
        "created_at": r.created_at,
        "updated_at": r.updated_at,
    })
}

// ── 供其它模块调用的查询辅助 ────────────────────────────────

/// 取用户绑定的套餐（未绑定 / 套餐缺失或已停用 → None）。
/// 用于站点数限制、SSH 终端开关等运行时校验。
pub async fn package_of_user(user_id: i64) -> Option<PackageRow> {
    let pool = db::get_db_pool().await;
    let pid: Option<i64> = sqlx::query_scalar("SELECT package_id FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let pid = pid.filter(|v| *v > 0)?;
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM packages WHERE id = ? AND status = 1"
    )))
    .bind(pid)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 全局默认套餐（owner_id = 0 中启用且 id 最小者；不存在时返回 None）。
/// 未绑定套餐的普通用户回退到该套餐，用于能力判定。
pub async fn default_package() -> Option<PackageRow> {
    let pool = db::get_db_pool().await;
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM packages WHERE owner_id = 0 AND status = 1 ORDER BY id ASC LIMIT 1"
    )))
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 用户实际生效的套餐：绑定套餐（启用中）优先；
/// 未绑定 / 套餐停用 / 套餐缺失则回退全局「默认套餐」。
/// 用于站点反代、自定义目录等能力门禁的运行时校验（与 SSH 终端的
/// 「未绑定不限制」不同：站点能力默认关闭，由默认套餐统一定义）。
pub async fn effective_package_of(user_id: i64) -> Option<PackageRow> {
    match package_of_user(user_id).await {
        Some(pkg) => Some(pkg),
        None => default_package().await,
    }
}

/// 校验操作者是否有权使用该套餐，并返回套餐行（user add/update 时调用）。
pub async fn load_for_actor(
    id: i64,
    is_admin: bool,
    actor_id: i64,
) -> Result<PackageRow, ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<PackageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM packages WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) if visible(&r, is_admin, actor_id) => Ok(r),
        _ => Err(ZapError::New(-1, "套餐不存在或无权使用".to_string())),
    }
}

// ── handlers ────────────────────────────────────────────────

/// GET /system/package/list
pub async fn package_list(claims: ValidatedClaims) -> ZapJsonResult {
    let is_admin = crate::zap::jwt::is_admin(&claims);
    let is_reseller = crate::zap::jwt::is_reseller(&claims);
    if !is_admin && !is_reseller {
        return Err(ZapError::New(-1, "权限不足".to_string()));
    }
    let actor_id = claims.id as i64;
    let pool = db::get_db_pool().await;
    let rows: Vec<PackageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM packages ORDER BY id DESC"
    )))
    .fetch_all(pool)
    .await?;
    let usage = usage_counts().await;

    let items: Vec<Value> = rows
        .iter()
        .filter(|r| visible(r, is_admin, actor_id))
        .map(|r| row_json(r, usage.get(&r.id).copied().unwrap_or(0)))
        .collect();
    Ok(Json(json!({ "code": 0, "message": "OK", "data": items })))
}

#[derive(Debug, Deserialize)]
pub struct PackageAddPayload {
    pub name: String,
    pub remark: Option<String>,
    /// 磁盘配额（MB，0 = 不限）
    pub disk_quota_mb: Option<i64>,
    /// 最大站点数（0 = 不限）
    pub max_sites: Option<i64>,
    /// 单站点最大域名数（0 = 不限）
    pub max_domains: Option<i64>,
    /// 月流量上限（MB，0 = 不限，仅记录）
    pub max_bandwidth_mb: Option<i64>,
    /// MySQL / MariaDB 数据库数量（0 = 不限）
    pub max_mysql_dbs: Option<i64>,
    /// PostgreSQL 数据库数量（0 = 不限，仅记录）
    pub max_pgsql_dbs: Option<i64>,
    /// FTP 账号数量（0 = 不限，仅记录）
    pub max_ftp_users: Option<i64>,
    /// PHP-FPM 规格模板名（'' = 面板默认）
    pub fpm_spec_ref: Option<String>,
    /// 是否允许 SSH 终端
    pub allow_ssh: Option<bool>,
    /// 是否允许普通用户使用反向代理（upstream / location）
    pub allow_proxy: Option<bool>,
    /// 是否允许建 PHP 站点（默认 true：PHP 站点是默认能力）
    pub allow_php: Option<bool>,
    /// 是否允许使用容器功能（默认 false；且仅 Podman 运行时对非管理员生效）
    pub allow_docker: Option<bool>,
    pub allow_waf: Option<bool>,
    /// 是否允许使用应用管理（默认 false）
    pub allow_apps: Option<bool>,
    /// 允许部署的应用类型（逗号分隔）；空 = 不限
    pub app_types: Option<String>,
    /// 每个站点可部署的应用数上限（0 = 不限）
    pub max_apps: Option<i64>,
    /// 每个用户分到的端口个数（0 = 不限）
    pub app_port_span: Option<i64>,
    /// 该用户全部站点合计的应用数上限（0 = 不限）
    pub app_max_total: Option<i64>,
    pub status: Option<i32>,
    /// 归属作用域（仅 admin 新建时生效）：`global` = 全局套餐（owner_id=0，默认）；
    /// `self` = 仅自己可见的私有套餐（owner_id = 当前 admin 自身）。
    /// reseller 忽略此字段，一律建自己名下的私有套餐。
    pub scope: Option<String>,
}

/// POST /system/package/add —— admin 建全局套餐；reseller 建自己名下套餐
/// 归一化套餐里配置的「允许的应用类型」：只保留受支持的类型，去重后逗号分隔。
/// 空串 / 全是无效值 = 不限（允许全部已支持类型）。
pub fn package_app_types_normalized(raw: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for t in raw.split(|c: char| c == ',' || c.is_whitespace()) {
        let t = t.trim().to_ascii_lowercase();
        if t.is_empty() || !zap_proto::app_type_supported(&t) || out.contains(&t) {
            continue;
        }
        out.push(t);
    }
    out.sort();
    out.join(",")
}

/// 端口池基准：用户 #N 的端口段从这里往上排
pub const APP_PORT_BASE: i64 = 10000;

/// 校验「每用户端口个数」：0 = 不限；其余必须是正数，
/// 且要留出足够余量 —— 用户 ID 增长后不能排到 65535 之外。
pub fn validate_port_span(span: i64) -> Result<i64, ZapError> {
    if span == 0 {
        return Ok(0);
    }
    if span < 1 || span > 4096 {
        return Err(ZapError::New(
            -1,
            "每用户端口数需在 1-4096 之间（0 = 不限）".to_string(),
        ));
    }
    Ok(span)
}

/// 算出用户 #`uid` 的端口段：`[base + uid*span, base + (uid+1)*span - 1]`。
///
/// 例：基准 10000、每用户 100 个 → 用户 1 拿到 10100-10199，用户 2 拿到 10200-10299。
/// `span = 0`（不限）或端口池已被前面的用户排满时返回 `None`。
pub fn user_port_range(uid: i64, span: i64) -> Option<(i64, i64)> {
    if span <= 0 {
        return None;
    }
    let lo = APP_PORT_BASE + uid.saturating_mul(span);
    let hi = lo + span - 1;
    if hi > 65535 {
        return None;
    }
    Some((lo, hi))
}

pub async fn package_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<PackageAddPayload>,
) -> ZapJsonResult {
    let is_admin = crate::zap::jwt::is_admin(&claims);
    let is_reseller = crate::zap::jwt::is_reseller(&claims);
    if !is_admin && !is_reseller {
        return Err(ZapError::New(-1, "权限不足".to_string()));
    }
    let name = validate_name(&payload.name)?;
    let remark = payload.remark.unwrap_or_default().trim().to_string();
    let disk_quota_mb = validate_limit(payload.disk_quota_mb.unwrap_or(0), "磁盘配额")?;
    let max_sites = validate_limit(payload.max_sites.unwrap_or(0), "最大站点数")?;
    let max_domains = validate_limit(payload.max_domains.unwrap_or(0), "单站点最大域名数")?;
    let max_bandwidth_mb = validate_limit(payload.max_bandwidth_mb.unwrap_or(0), "月流量上限")?;
    let max_mysql_dbs = validate_limit(payload.max_mysql_dbs.unwrap_or(0), "MySQL 数据库数量")?;
    let max_pgsql_dbs =
        validate_limit(payload.max_pgsql_dbs.unwrap_or(0), "PostgreSQL 数据库数量")?;
    let max_ftp_users = validate_limit(payload.max_ftp_users.unwrap_or(0), "FTP 账号数量")?;
    let fpm_spec_ref = payload.fpm_spec_ref.unwrap_or_default().trim().to_string();
    if !fpm_spec_ref.is_empty() {
        crate::routers::fpm_spec::validate_spec_ref(&fpm_spec_ref, is_admin, claims.sub.as_str())
            .await?;
    }
    let allow_apps = i32::from(payload.allow_apps.unwrap_or(false));
    let app_types = package_app_types_normalized(payload.app_types.as_deref().unwrap_or(""));
    let max_apps = validate_limit(payload.max_apps.unwrap_or(0), "每站点应用数上限")?;
    let app_port_span = validate_port_span(payload.app_port_span.unwrap_or(0))?;
    let app_max_total = validate_limit(payload.app_max_total.unwrap_or(0), "用户应用总数上限")?;
    let allow_ssh = i32::from(payload.allow_ssh.unwrap_or(false));
    let allow_proxy = i32::from(payload.allow_proxy.unwrap_or(false));
    // PHP 默认开放；容器默认关闭（容器还要求运行时是 Podman，见 routers::docker 门禁）
    let allow_php = i32::from(payload.allow_php.unwrap_or(true));
    let allow_docker = i32::from(payload.allow_docker.unwrap_or(false));
    // WAF 默认关闭：全局未装 ModSecurity 时站点不该渲染 modsecurity 指令
    let allow_waf = i32::from(payload.allow_waf.unwrap_or(false));
    let status = payload.status.unwrap_or(1).clamp(0, 1);
    // 归属：reseller 只能建自己名下的私有套餐；admin 可选全局（默认）或仅自己可见的私有套餐
    let owner_id: i64 = if is_admin {
        match payload.scope.as_deref() {
            Some("self") => claims.id as i64,
            _ => 0,
        }
    } else {
        claims.id as i64
    };
    let now = chrono::Local::now().timestamp();

    // reseller 创建的子套餐不得超过其自身套餐（父套餐）的允许范围
    if is_reseller && !is_admin {
        match package_of_user(claims.id as i64).await {
            Some(parent) => {
                let sub = ResellerSubVals {
                    disk_quota_mb,
                    max_sites,
                    max_domains,
                    max_bandwidth_mb,
                    max_mysql_dbs,
                    max_pgsql_dbs,
                    max_ftp_users,
                    app_max_total,
                    max_apps,
                    app_port_span,
                    allow_ssh,
                    allow_proxy,
                    allow_php,
                    allow_docker,
                    allow_waf,
                    allow_apps,
                    app_types: app_types.clone(),
                };
                enforce_reseller_subpackage(&parent, &sub)?;
            }
            None => {
                return Err(ZapError::New(
                    -1,
                    "你自身未绑定套餐，无法创建子套餐（请先由管理员为你分配套餐）".to_string(),
                ));
            }
        }
    }

    let pool = db::get_db_pool().await;
    let result = sqlx::query(
        "INSERT INTO packages (name, remark, disk_quota_mb, max_sites, max_domains, max_bandwidth_mb, \
         max_mysql_dbs, max_pgsql_dbs, max_ftp_users, \
         fpm_spec_ref, allow_ssh, allow_proxy, allow_php, allow_docker, allow_waf, \
         allow_apps, app_types, max_apps, app_port_span, app_max_total, \
         owner_id, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&name)
    .bind(&remark)
    .bind(disk_quota_mb)
    .bind(max_sites)
    .bind(max_domains)
    .bind(max_bandwidth_mb)
    .bind(max_mysql_dbs)
    .bind(max_pgsql_dbs)
    .bind(max_ftp_users)
    .bind(&fpm_spec_ref)
    .bind(allow_ssh)
    .bind(allow_proxy)
    .bind(allow_php)
    .bind(allow_docker)
    .bind(allow_waf)
    .bind(allow_apps)
    .bind(&app_types)
    .bind(max_apps)
    .bind(app_port_span)
    .bind(app_max_total)
    .bind(owner_id)
    .bind(status)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await;

    let new_id = match result {
        Ok(r) => r.last_insert_rowid(),
        Err(sqlx::Error::Database(e)) if e.message().contains("packages.name") => {
            return Err(ZapError::New(-1, format!("套餐名「{name}」已存在")));
        }
        Err(e) => return Err(ZapError::from(e)),
    };

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "package_add",
        &format!("id={new_id}"),
        &format!(
            "name={name} sites={max_sites} domains={max_domains} disk={disk_quota_mb}MB \
             mysql={max_mysql_dbs} pgsql={max_pgsql_dbs} ftp={max_ftp_users}"
        ),
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "套餐已创建", "data": { "id": new_id } }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct PackageUpdatePayload {
    pub id: i64,
    pub name: Option<String>,
    pub remark: Option<String>,
    pub disk_quota_mb: Option<i64>,
    pub max_sites: Option<i64>,
    pub max_domains: Option<i64>,
    pub max_bandwidth_mb: Option<i64>,
    /// MySQL / MariaDB 数据库数量（0 = 不限）
    pub max_mysql_dbs: Option<i64>,
    /// PostgreSQL 数据库数量（0 = 不限，仅记录）
    pub max_pgsql_dbs: Option<i64>,
    /// FTP 账号数量（0 = 不限，仅记录）
    pub max_ftp_users: Option<i64>,
    pub fpm_spec_ref: Option<String>,
    pub allow_ssh: Option<bool>,
    /// 是否允许普通用户使用反向代理（upstream / location）
    pub allow_proxy: Option<bool>,
    /// 是否允许建 PHP 站点；未传则保持不变
    pub allow_php: Option<bool>,
    /// 是否允许使用容器功能；未传则保持不变
    pub allow_docker: Option<bool>,
    pub allow_waf: Option<bool>,
    /// 是否允许使用应用管理；未传则保持不变
    pub allow_apps: Option<bool>,
    /// 允许部署的应用类型（逗号分隔）；未传则保持不变
    pub app_types: Option<String>,
    /// 每个站点可部署的应用数上限（0 = 不限）；未传则保持不变
    pub max_apps: Option<i64>,
    /// 每个用户分到的端口个数；未传则保持不变
    pub app_port_span: Option<i64>,
    /// 该用户全部站点合计的应用数上限；未传则保持不变
    pub app_max_total: Option<i64>,
    pub status: Option<i32>,
    /// 归属作用域（仅 admin 编辑时生效）：`global` = 改为全局（owner_id=0）；
    /// `self` = 改为仅当前管理员可见（owner_id = 当前管理员）。reseller 忽略此字段。
    pub scope: Option<String>,
}

/// POST /system/package/update —— 仅能修改自己可见的套餐
pub async fn package_update(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<PackageUpdatePayload>,
) -> ZapJsonResult {
    let is_admin = crate::zap::jwt::is_admin(&claims);
    let is_reseller = crate::zap::jwt::is_reseller(&claims);
    if !is_admin && !is_reseller {
        return Err(ZapError::New(-1, "权限不足".to_string()));
    }
    let actor_id = claims.id as i64;
    let current = load_for_actor(payload.id, is_admin, actor_id).await?;

    // 全局套餐（owner_id=0）仅管理员可编辑/删除；reseller 只能管理自己名下的套餐
    if !is_admin && current.owner_id == 0 {
        return Err(ZapError::New(
            -1,
            "权限不足：全局套餐仅管理员可管理".to_string(),
        ));
    }

    // reseller 修改自有子套餐时，结果各项不得超过其父套餐（reseller 自身套餐）的允许范围
    if is_reseller && !is_admin {
        if let Some(parent) = package_of_user(claims.id as i64).await {
            let eff = ResellerSubVals {
                disk_quota_mb: payload.disk_quota_mb.unwrap_or(current.disk_quota_mb),
                max_sites: payload.max_sites.unwrap_or(current.max_sites),
                max_domains: payload.max_domains.unwrap_or(current.max_domains),
                max_bandwidth_mb: payload.max_bandwidth_mb.unwrap_or(current.max_bandwidth_mb),
                max_mysql_dbs: payload.max_mysql_dbs.unwrap_or(current.max_mysql_dbs),
                max_pgsql_dbs: payload.max_pgsql_dbs.unwrap_or(current.max_pgsql_dbs),
                max_ftp_users: payload.max_ftp_users.unwrap_or(current.max_ftp_users),
                app_max_total: payload.app_max_total.unwrap_or(current.app_max_total),
                max_apps: payload.max_apps.unwrap_or(current.max_apps),
                app_port_span: payload.app_port_span.unwrap_or(current.app_port_span),
                allow_ssh: i32::from(payload.allow_ssh.unwrap_or(current.allow_ssh == 1)),
                allow_proxy: i32::from(payload.allow_proxy.unwrap_or(current.allow_proxy == 1)),
                allow_php: i32::from(payload.allow_php.unwrap_or(current.allow_php == 1)),
                allow_docker: i32::from(payload.allow_docker.unwrap_or(current.allow_docker == 1)),
                allow_waf: i32::from(payload.allow_waf.unwrap_or(current.allow_waf == 1)),
                allow_apps: i32::from(payload.allow_apps.unwrap_or(current.allow_apps == 1)),
                app_types: payload
                    .app_types
                    .clone()
                    .map(|s| package_app_types_normalized(&s))
                    .unwrap_or_else(|| current.app_types.clone()),
            };
            enforce_reseller_subpackage(&parent, &eff)?;
        }
    }

    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();

    // 归属变更（仅管理员可改）：global -> owner_id=0（全局）；self -> 当前管理员名下私有。
    // reseller 忽略 scope；全局套餐的归属变更也只允许管理员操作（上面已拦截非管理员）。
    if is_admin {
        if let Some(scope) = payload.scope.as_deref() {
            let new_owner: i64 = match scope {
                "self" => claims.id as i64,
                _ => 0,
            };
            if new_owner != current.owner_id {
                sqlx::query("UPDATE packages SET owner_id = ?, updated_at = ? WHERE id = ?")
                    .bind(new_owner)
                    .bind(now)
                    .bind(payload.id)
                    .execute(pool)
                    .await?;
            }
        }
    }

    // 逐字段更新，便于精确审计与错误提示
    if let Some(n) = payload.name {
        let n = validate_name(&n)?;
        let r = sqlx::query("UPDATE packages SET name = ?, updated_at = ? WHERE id = ?")
            .bind(&n)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await;
        if let Err(sqlx::Error::Database(e)) = &r
            && e.message().contains("packages.name")
        {
            return Err(ZapError::New(-1, format!("套餐名「{n}」已存在")));
        }
        r?;
    }
    if let Some(rm) = payload.remark {
        sqlx::query("UPDATE packages SET remark = ?, updated_at = ? WHERE id = ?")
            .bind(rm.trim())
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.disk_quota_mb {
        let v = validate_limit(v, "磁盘配额")?;
        sqlx::query("UPDATE packages SET disk_quota_mb = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.max_sites {
        let v = validate_limit(v, "最大站点数")?;
        sqlx::query("UPDATE packages SET max_sites = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.max_domains {
        let v = validate_limit(v, "单站点最大域名数")?;
        sqlx::query("UPDATE packages SET max_domains = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.max_bandwidth_mb {
        let v = validate_limit(v, "月流量上限")?;
        sqlx::query("UPDATE packages SET max_bandwidth_mb = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.max_mysql_dbs {
        let v = validate_limit(v, "MySQL 数据库数量")?;
        sqlx::query("UPDATE packages SET max_mysql_dbs = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.max_pgsql_dbs {
        let v = validate_limit(v, "PostgreSQL 数据库数量")?;
        sqlx::query("UPDATE packages SET max_pgsql_dbs = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.max_ftp_users {
        let v = validate_limit(v, "FTP 账号数量")?;
        sqlx::query("UPDATE packages SET max_ftp_users = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.fpm_spec_ref {
        let v = v.trim().to_string();
        if !v.is_empty() {
            crate::routers::fpm_spec::validate_spec_ref(&v, is_admin, claims.sub.as_str()).await?;
        }
        sqlx::query("UPDATE packages SET fpm_spec_ref = ?, updated_at = ? WHERE id = ?")
            .bind(&v)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.allow_ssh {
        sqlx::query("UPDATE packages SET allow_ssh = ?, updated_at = ? WHERE id = ?")
            .bind(i32::from(v))
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.allow_proxy {
        sqlx::query("UPDATE packages SET allow_proxy = ?, updated_at = ? WHERE id = ?")
            .bind(i32::from(v))
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.allow_php {
        sqlx::query("UPDATE packages SET allow_php = ?, updated_at = ? WHERE id = ?")
            .bind(i32::from(v))
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.allow_docker {
        sqlx::query("UPDATE packages SET allow_docker = ?, updated_at = ? WHERE id = ?")
            .bind(i32::from(v))
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }

    if let Some(v) = payload.allow_waf {
        sqlx::query("UPDATE packages SET allow_waf = ?, updated_at = ? WHERE id = ?")
            .bind(i32::from(v))
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.allow_apps {
        sqlx::query("UPDATE packages SET allow_apps = ?, updated_at = ? WHERE id = ?")
            .bind(i32::from(v))
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = &payload.app_types {
        sqlx::query("UPDATE packages SET app_types = ?, updated_at = ? WHERE id = ?")
            .bind(package_app_types_normalized(v))
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.max_apps {
        let n = validate_limit(v, "每站点应用数上限")?;
        sqlx::query("UPDATE packages SET max_apps = ?, updated_at = ? WHERE id = ?")
            .bind(n)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.app_port_span {
        let n = validate_port_span(v)?;
        sqlx::query("UPDATE packages SET app_port_span = ?, updated_at = ? WHERE id = ?")
            .bind(n)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.app_max_total {
        let n = validate_limit(v, "用户应用总数上限")?;
        sqlx::query("UPDATE packages SET app_max_total = ?, updated_at = ? WHERE id = ?")
            .bind(n)
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.status {
        sqlx::query("UPDATE packages SET status = ?, updated_at = ? WHERE id = ?")
            .bind(v.clamp(0, 1))
            .bind(now)
            .bind(payload.id)
            .execute(pool)
            .await?;
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "package_update",
        &format!("id={}", payload.id),
        &format!("name={}", current.name),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "套餐已更新" })))
}

#[derive(Debug, Deserialize)]
pub struct PackageDeletePayload {
    pub id: i64,
}

/// POST /system/package/delete —— 仍被客户引用时拒绝删除
pub async fn package_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<PackageDeletePayload>,
) -> ZapJsonResult {
    let is_admin = crate::zap::jwt::is_admin(&claims);
    let is_reseller = crate::zap::jwt::is_reseller(&claims);
    if !is_admin && !is_reseller {
        return Err(ZapError::New(-1, "权限不足".to_string()));
    }
    let actor_id = claims.id as i64;
    let current = load_for_actor(payload.id, is_admin, actor_id).await?;

    // 全局套餐（owner_id=0）仅管理员可编辑/删除；reseller 只能管理自己名下的套餐
    if !is_admin && current.owner_id == 0 {
        return Err(ZapError::New(
            -1,
            "权限不足：全局套餐仅管理员可管理".to_string(),
        ));
    }

    let pool = db::get_db_pool().await;
    let used: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM user WHERE package_id = ?")
        .bind(payload.id)
        .fetch_one(pool)
        .await?;
    if used.0 > 0 {
        return Err(ZapError::New(
            -1,
            format!(
                "套餐「{}」仍被 {} 个客户使用，请先将这些客户变更到其它套餐后再删除",
                current.name, used.0
            ),
        ));
    }

    sqlx::query("DELETE FROM packages WHERE id = ?")
        .bind(payload.id)
        .execute(pool)
        .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "package_delete",
        &format!("id={}", payload.id),
        &format!("name={}", current.name),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "套餐已删除" })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_range_defaults_and_bounds() {
        // span = 0 → 不限
        assert_eq!(user_port_range(1, 0), None);
        // 基准 10000 + uid*span：用户 1 拿 10100-10199，用户 2 拿 10200-10299
        assert_eq!(user_port_range(1, 100), Some((10100, 10199)));
        assert_eq!(user_port_range(2, 100), Some((10200, 10299)));
        assert_eq!(user_port_range(0, 100), Some((10000, 10099)));
        // span = 1：每人一个端口，段首尾相同
        assert_eq!(user_port_range(3, 1), Some((10003, 10003)));
        // 端口池排满 65535 之后给不出段，而不是给出一个越界的
        assert_eq!(user_port_range(60000, 1), None);
        assert_eq!(user_port_range(600, 100), None);
        // 越界的 span 直接拒
        assert!(validate_port_span(0).is_ok());
        assert!(validate_port_span(1).is_ok());
        assert!(validate_port_span(4097).is_err());
        assert!(validate_port_span(-1).is_err());
    }

    #[test]
    fn app_types_keep_only_supported_ones() {
        assert_eq!(
            package_app_types_normalized("python,nodejs"),
            "nodejs,python"
        );
        assert_eq!(package_app_types_normalized("python,python"), "python");
        // 未知类型（如 php 尚未支持）直接丢弃
        assert_eq!(package_app_types_normalized("python,php"), "python");
        assert_eq!(package_app_types_normalized(""), "");
    }
}
