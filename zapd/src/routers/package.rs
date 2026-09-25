//! 套餐（Packages）管理 —— 对齐 cPanel/WHM 的 Packages：
//! 由管理员 / 经销商预先定义一组资源限制，创建客户（用户）时选择，
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
//! - `allow_docker`      是否允许该套餐的用户使用容器功能（默认关闭），
//!                       且**仅当容器运行时为 Podman 时**才对非管理员生效
//!   「自定义目录」不再作为套餐能力：home 目录内任意目录已全量开放。
//!
//! 归属：`owner_id = 0` 为全局套餐（admin 维护，所有人可用）；
//! reseller 自建套餐 `owner_id` 为自己，仅本人可用。
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
    pub owner_id: i64,
    pub status: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLS: &str = "id, name, remark, disk_quota_mb, max_sites, max_domains, max_bandwidth_mb, \
                    max_mysql_dbs, max_pgsql_dbs, max_ftp_users, \
                    fpm_spec_ref, allow_ssh, allow_proxy, allow_php, allow_docker, \
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

/// 套餐对操作者是否可见：admin 全量；reseller 仅全局套餐（owner_id=0）与自己名下
fn visible(r: &PackageRow, is_admin: bool, actor_id: i64) -> bool {
    is_admin || r.owner_id == 0 || r.owner_id == actor_id
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
    sqlx::query_as(&format!(
        "SELECT {COLS} FROM packages WHERE id = ? AND status = 1"
    ))
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
    sqlx::query_as(&format!(
        "SELECT {COLS} FROM packages WHERE owner_id = 0 AND status = 1 ORDER BY id ASC LIMIT 1"
    ))
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
    let row: Option<PackageRow> =
        sqlx::query_as(&format!("SELECT {COLS} FROM packages WHERE id = ?"))
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
    let rows: Vec<PackageRow> =
        sqlx::query_as(&format!("SELECT {COLS} FROM packages ORDER BY id DESC"))
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
    pub status: Option<i32>,
}

/// POST /system/package/add —— admin 建全局套餐；reseller 建自己名下套餐
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
    let allow_ssh = i32::from(payload.allow_ssh.unwrap_or(false));
    let allow_proxy = i32::from(payload.allow_proxy.unwrap_or(false));
    // PHP 默认开放；容器默认关闭（容器还要求运行时是 Podman，见 routers::docker 门禁）
    let allow_php = i32::from(payload.allow_php.unwrap_or(true));
    let allow_docker = i32::from(payload.allow_docker.unwrap_or(false));
    let status = payload.status.unwrap_or(1).clamp(0, 1);
    // admin 建全局套餐；reseller 建自己名下套餐
    let owner_id: i64 = if is_admin { 0 } else { claims.id as i64 };
    let now = chrono::Local::now().timestamp();

    let pool = db::get_db_pool().await;
    let result = sqlx::query(
        "INSERT INTO packages (name, remark, disk_quota_mb, max_sites, max_domains, max_bandwidth_mb, \
         max_mysql_dbs, max_pgsql_dbs, max_ftp_users, \
         fpm_spec_ref, allow_ssh, allow_proxy, allow_php, allow_docker, \
         owner_id, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    pub status: Option<i32>,
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

    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();

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
