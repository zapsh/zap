use axum::{Json, extract::Extension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{QueryBuilder, Sqlite};
use std::net::SocketAddr;
use tracing::{info, warn};

use crate::{
    db,
    zap::{
        ZapError, ZapJsonResult, appstore as ast, audit,
        jwt::{self, Claims, ValidatedClaims},
        user_cron,
    },
};

#[derive(sqlx::FromRow, Debug)]
struct UserInfo {
    id: i64,
    username: String,
    email: String,
    phone: Option<String>,
    nickname: String,
    home_dir: String,
    linux_user: String,
    fpm_pool: String,
    /// PHP-FPM 规格引用：''=面板默认 / 'inherit'=继承 owner(reseller) 名下默认 / 模板名
    fpm_spec_ref: String,
    last_login_ip: String,
    last_login_time: i64,
    status: i32,
    roles: String,
    permissions: String,
    owner_id: i64,
    /// 用户类型：0=普通用户/客户 / 1=成员（子账号，共享父账号家目录与系统账号）
    user_kind: i32,
    /// 父账号对该成员收紧（取消）的权限点，逗号分隔；仅成员生效
    perm_deny: String,
    /// 只读账号：1=共享可见但不可改（生效权限只保留 `{ns}:view`）
    read_only: i32,
    package_id: i64,
    /// 家目录磁盘用量（字节，定时任务 du 采集；0 = 未采集）
    disk_used_bytes: i64,
    disk_stat_at: i64,
    /// 本月出站流量（字节，解析站点 access.log 汇总）
    bandwidth_used_bytes: i64,
    /// 流量统计周期（YYYYMM）
    bandwidth_period: String,
    bandwidth_stat_at: i64,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserPayload {
    pub username: String,
    pub password: String,
    pub email: String,
    pub phone: Option<String>,
    pub nickname: Option<String>,
    pub roles: Option<String>,
    pub owner_id: Option<i64>,
    /// 该用户 PHP-FPM pool 规格（JSON 字符串；空 = 使用面板默认规格）
    pub fpm_pool: Option<String>,
    /// PHP-FPM 规格引用：''=面板默认 / 'inherit'=继承 owner(reseller) 名下默认 / 模板名
    pub fpm_spec_ref: Option<String>,
    /// 套餐 id（0 / None = 不绑定套餐）
    pub package_id: Option<i64>,
    /// 个人附加权限点（逗号分隔或数组；**只做加法**，在角色权限之外临时开小灶）
    #[serde(default)]
    pub permissions: Option<PermissionInput>,
    /// 用户类型：0=普通用户/客户（默认，独立家目录与 Linux 账号）；
    /// 1=成员（子账号，共享父账号的家目录与 Linux 账号，权限默认继承父账号）
    #[serde(default)]
    pub user_kind: Option<i32>,
    /// 父账号对该成员收紧（取消）的权限点（逗号分隔或数组）；仅成员生效
    #[serde(default)]
    pub perm_deny: Option<PermissionInput>,
    /// 只读账号：`true` 时该用户只能查看（生效权限收敛为 `{ns}:view`），
    /// 用于「共享可见但不能改」的成员 / 客户
    #[serde(default)]
    pub read_only: Option<bool>,
}

/// 附加权限入参：兼容数组与逗号分隔字符串两种写法。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum PermissionInput {
    List(Vec<String>),
    Csv(String),
}

impl PermissionInput {
    /// 归一化并过滤非法权限点（只保留权限目录里存在的 key）。
    fn normalize(&self) -> String {
        let valid = crate::routers::access::all_perm_keys();
        let raw: Vec<String> = match self {
            PermissionInput::List(v) => v.clone(),
            PermissionInput::Csv(s) => s.split(',').map(|x| x.to_string()).collect(),
        };
        let mut keys: Vec<String> = raw
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| valid.contains(s))
            .collect();
        keys.sort();
        keys.dedup();
        keys.join(",")
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserPayload {
    pub id: i64,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub nickname: Option<String>,
    pub roles: Option<String>,
    pub status: Option<i32>,
    pub password: Option<String>,
    /// 该用户 PHP-FPM pool 规格（JSON 字符串；空 = 恢复面板默认）
    pub fpm_pool: Option<String>,
    /// PHP-FPM 规格引用：''=面板默认 / 'inherit'=继承 owner(reseller) 名下默认 / 模板名；
    /// 提交引用（含面板默认）时后端会同步清空旧的自定义 fpm_pool
    pub fpm_spec_ref: Option<String>,
    /// 套餐 id（0 = 解除套餐绑定）
    pub package_id: Option<i64>,
    /// 个人附加权限点（不传 = 不改动；传空数组/空串 = 清空附加权限）
    #[serde(default)]
    pub permissions: Option<PermissionInput>,
    /// 父账号对该成员收紧（取消）的权限点（不传 = 不改动；传空 = 取消全部收紧）
    #[serde(default)]
    pub perm_deny: Option<PermissionInput>,
    /// 只读账号开关（不传 = 不改动）
    #[serde(default)]
    pub read_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteUserPayload {
    pub id: i64,
}

/// 权限变更单独记一条审计：权限是敏感变更，需要能追溯「谁在什么时候给谁开了什么」。
///
/// `user_update` 只记一条笼统的 `user_update`，无法回答"附加权限被谁改过"，
/// 因此附加权限变更额外落一条 `user_permissions_set`。
async fn audit_permissions_set(claims: &jwt::Claims, ip: &str, uid: i64, keys: &str) {
    audit::log(
        Some(claims),
        Some(ip),
        "user_permissions_set",
        &format!("id={uid}"),
        keys,
    )
    .await;
}

/// 内置初始管理员（安装时创建的 admin）的用户 ID：
/// 不可删除、不可禁用、角色不可变更，且除本人外任何人都不能修改其信息/密码。
const ROOT_ADMIN_ID: i64 = 1;

/// 用户类型：成员（子账号）。与 `access::USER_KIND_MEMBER` 同一取值。
pub const USER_KIND_MEMBER: i32 = crate::routers::access::USER_KIND_MEMBER;

/// 用户的归属与运行实体信息（成员创建 / 删除时读取父账号用）。
#[derive(sqlx::FromRow)]
struct UserAccountRow {
    home_dir: String,
    linux_user: String,
    #[allow(dead_code)]
    roles: String,
    package_id: i64,
    owner_id: i64,
    user_kind: i32,
}

/// 读取用户的归属与运行实体信息；不存在返回 None。
async fn load_account(id: i64) -> Result<Option<UserAccountRow>, ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<UserAccountRow> =
        sqlx::query_as("SELECT home_dir, linux_user, roles, package_id, owner_id, user_kind FROM user WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(row)
}

/// 该用户是否为成员（子账号）。
async fn user_is_member(uid: i64) -> bool {
    matches!(
        load_account(uid).await,
        Ok(Some(a)) if a.user_kind == USER_KIND_MEMBER
    )
}

/// 目标用户是否可被当前操作者当作「自己的成员」管理。
async fn is_own_member(claims: &jwt::Claims, target: i64) -> bool {
    match load_account(target).await {
        Ok(Some(a)) => a.user_kind == USER_KIND_MEMBER && a.owner_id == claims.id as i64,
        _ => false,
    }
}

/// Require admin role; return error if not admin
fn require_admin(claims: &jwt::Claims) -> Result<(), ZapError> {
    if jwt::is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "权限不足，需要管理员权限".to_string()))
    }
}

/// 归一化用户 fpm pool 规格：
/// - None / 空 → Some("")（不指定，使用面板默认）
/// - 其它 → 必须是 JSON 对象字符串
fn normalize_fpm_spec(raw: Option<String>) -> Result<Option<String>, ZapError> {
    match raw {
        None => Ok(None),
        Some(v) => {
            let v = v.trim().to_string();
            if v.is_empty() {
                return Ok(Some(String::new()));
            }
            match serde_json::from_str::<Value>(&v) {
                Ok(Value::Object(_)) => Ok(Some(v)),
                _ => Err(ZapError::New(
                    -1,
                    "fpm_pool 必须是 JSON 对象（如 {\"max_children\": 12}）".to_string(),
                )),
            }
        }
    }
}

/// 补齐「面板用户 → 运行实体」（幂等）：
/// 1. 确保 user.linux_user 有值（空则按用户名派生并落库）；
/// 2. 创建 Linux 系统账号（useradd -M -s nologin -d {home_dir}）；
/// 3. 初始化家目录骨架（www/logs/tmp）并归该账号所有。
///
/// 每个面板用户对应一个独立 Linux 账号，站点与 PHP-FPM pool 均以该账号运行。
/// 站点同步 / 用户同步 / 新增用户均调用；失败返回 Err 描述。
pub async fn ensure_user_runtime(uid: i64) -> Result<(), String> {
    let pool = db::get_db_pool().await;
    let row: Option<(String, String, String, i64, i32)> = sqlx::query_as(
        "SELECT username, home_dir, linux_user, owner_id, user_kind FROM user WHERE id = ?",
    )
    .bind(uid)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some((username, home_dir, mut linux_user, owner_id, user_kind)) = row else {
        return Err(format!("用户 {uid} 不存在"));
    };
    // 成员（子账号）共享父账号的 Linux 账号与家目录：本身不建系统账号，
    // 只需保证父账号的运行实体就绪（层级只有一层，递归一次即收敛）。
    if user_kind == USER_KIND_MEMBER {
        if owner_id > 0 && owner_id != uid {
            return Box::pin(ensure_user_runtime(owner_id)).await;
        }
        return Ok(());
    }
    if home_dir.is_empty() {
        return Err(format!("用户 {username} 未配置家目录（home_dir 为空）"));
    }
    if linux_user.is_empty() {
        linux_user = zap_proto::linux_username(&username);
        sqlx::query("UPDATE user SET linux_user = ? WHERE id = ?")
            .bind(&linux_user)
            .bind(uid)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    // 1) Linux 系统账号（nologin）
    let resp = crate::zapexec::call(zap_proto::types::Request::UserSystemInit {
        linux_user: linux_user.clone(),
        home_dir: home_dir.clone(),
    })
    .await
    .map_err(|e| e.to_string())?;
    if resp.code != 0 {
        return Err(format!("创建 Linux 账号失败: {}", resp.message));
    }
    // 2) 家目录骨架归该账号所有
    let resp = crate::zapexec::call(zap_proto::types::Request::UserHomeInit {
        home_dir: home_dir.clone(),
        owner: linux_user,
    })
    .await
    .map_err(|e| e.to_string())?;
    if resp.code != 0 {
        return Err(format!("初始化家目录失败: {}", resp.message));
    }
    Ok(())
}

/// 按用户当前套餐下发磁盘配额（best-effort：失败仅写日志，不阻断用户创建/编辑）。
/// 未绑定套餐、套餐未提供配额字段或用户无 Linux 系统账号时跳过。
pub async fn sync_package_quota(user_id: i64) {
    // 成员共享父账号的系统账号：配额属于父账号，不能按成员自己的套餐改写
    if user_is_member(user_id).await {
        return;
    }
    let Some(pkg) = crate::routers::package::package_of_user(user_id).await else {
        return;
    };
    let pool = db::get_db_pool().await;
    let lu: Option<String> = sqlx::query_scalar("SELECT linux_user FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let Some(linux_user) = lu.filter(|s| !s.trim().is_empty()) else {
        return;
    };
    let req = zap_proto::types::Request::UserQuotaSet {
        linux_user: linux_user.clone(),
        quota_mb: pkg.disk_quota_mb,
    };
    match crate::zapexec::call(req).await {
        Ok(resp) if resp.code == 0 => info!(
            "套餐「{}」磁盘配额已下发: {} = {} MB",
            pkg.name, linux_user, pkg.disk_quota_mb
        ),
        Ok(resp) => warn!(
            "下发磁盘配额失败({}): {}（配额未生效时可检查文件系统是否启用 quota）",
            linux_user, resp.message
        ),
        Err(e) => warn!("下发磁盘配额失败({}): {}", linux_user, e),
    }
}

/// Fetch the owner_id of a user. Returns -1 when the user does not exist.
async fn get_user_owner_id(id: i64) -> Result<i64, ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(i64,)> = sqlx::query_as("SELECT owner_id FROM user WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(o,)| o).unwrap_or(-1))
}

pub async fn user_info(claims: Claims) -> Json<Value> {
    let uid = claims.id;
    let pool = db::get_db_pool().await;
    let result: Result<UserInfo, sqlx::Error> = sqlx::query_as("select * from user where id = ?")
        .bind(uid as i64)
        .fetch_one(pool)
        .await;
    if let Ok(user) = result {
        // 当前生效套餐：绑定套餐（启用中）优先，未绑定回退全局默认套餐（用于首页展示限额）
        let bound_pkg = crate::routers::package::package_of_user(user.id).await;
        let bound = bound_pkg.is_some();
        let pkg = match bound_pkg {
            Some(p) => Some(p),
            None => crate::routers::package::default_package().await,
        };
        // 生效权限点：角色权限（role_permissions）∪ 个人附加权限（user.permissions）
        // ∪ 父账号继承（成员）− 父账号收紧（perm_deny）。
        // 仅用于前端 v-permission 做按钮级体验控制，请求级拦截在 access::guard。
        let perms = crate::routers::access::effective_permissions_of(user.id as u64).await;

        return Json(json!({
            "code": 0,
            "message": "OK",
            "data": {
                "id": user.id,
                "username": user.username,
                "email": user.email,
                "phone": user.phone.clone().unwrap_or_default(),
                "nickname": user.nickname,
                "home_dir": user.home_dir,
                "linux_user": user.linux_user,
                "fpm_pool": user.fpm_pool,
                "fpm_spec_ref": user.fpm_spec_ref,
                "last_login_ip": user.last_login_ip,
                "last_login_time": user.last_login_time,
                // 资源用量：磁盘（du 采集）/ 本月流量（access.log 汇总）
                "disk_used_bytes": user.disk_used_bytes,
                "disk_stat_at": user.disk_stat_at,
                "bandwidth_used_bytes": user.bandwidth_used_bytes,
                "bandwidth_period": user.bandwidth_period,
                "bandwidth_stat_at": user.bandwidth_stat_at,
                "roles": user.roles.split(',').collect::<Vec<&str>>(),
                "permissions": perms,
                // 用户类型与归属：0=普通用户/客户 / 1=成员（子账号）
                "user_kind": user.user_kind,
                "owner_id": user.owner_id,
                // 父账号对该成员收紧的权限点（仅成员有值）
                "perm_deny": user.perm_deny.split(',').filter(|s| !s.is_empty()).collect::<Vec<&str>>(),
                // 只读账号：共享可见但不可改
                "read_only": user.read_only != 0,
                // 套餐信息：package_bound 标记是否绑定自己的套餐（false = 回退全局默认）
                "package_bound": bound,
                "package": pkg.map(|p| json!({
                    "id": p.id,
                    "name": p.name,
                    "remark": p.remark,
                    "disk_quota_mb": p.disk_quota_mb,
                    "max_sites": p.max_sites,
                    "max_domains": p.max_domains,
                    "max_bandwidth_mb": p.max_bandwidth_mb,
                    "max_mysql_dbs": p.max_mysql_dbs,
                    "max_pgsql_dbs": p.max_pgsql_dbs,
                    "max_ftp_users": p.max_ftp_users,
                    "fpm_spec_ref": p.fpm_spec_ref,
                    "allow_ssh": p.allow_ssh == 1,
                    "allow_proxy": p.allow_proxy == 1,
                })),
            }
        }));
    }
    Json(json!({
        "code": -1,
        "message": "User not found",
    }))
}

// ── 个人中心 → 偏好设置 ──────────────────────────────────────

fn default_true() -> bool {
    true
}

fn default_autossl_mode() -> String {
    "deferrals".to_string()
}

/// 当前用户通知/其它偏好（个人中心 → 偏好设置），存 user.prefs（JSON）。
/// 子项 `*_disable`：对应父通知类别下的子通知被禁用（cPanel 风格偏好覆盖）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NoticePrefs {
    /// 账户接近磁盘配额
    #[serde(default = "default_true")]
    pub notify_disk_quota: bool,
    /// 账户接近带宽限制
    #[serde(default = "default_true")]
    pub notify_bandwidth: bool,
    /// SSL 证书即将过期
    #[serde(default = "default_true")]
    pub notify_ssl_expiry: bool,
    /// 账户密码变化
    #[serde(default = "default_true")]
    pub notify_password_change: bool,
    #[serde(default)]
    pub password_change_disable: bool,
    /// 有人登录我的账户（成功登录通知）
    #[serde(default)]
    pub notify_login: bool,
    #[serde(default)]
    pub login_disable: bool,
    /// AutoSSL 通知模式：deferrals=失败及延后 / failures=仅失败 / disabled=禁用
    #[serde(default = "default_autossl_mode")]
    pub autossl_notify_mode: String,
}

impl Default for NoticePrefs {
    fn default() -> Self {
        Self {
            notify_disk_quota: true,
            notify_bandwidth: true,
            notify_ssl_expiry: true,
            notify_password_change: true,
            password_change_disable: false,
            notify_login: false,
            login_disable: false,
            autossl_notify_mode: "deferrals".to_string(),
        }
    }
}

/// GET /user/prefs：读取当前用户的偏好设置（无记录时返回默认值）。
pub async fn user_prefs_get(claims: Claims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let prefs: Option<String> = sqlx::query_scalar("SELECT prefs FROM user WHERE id = ?")
        .bind(claims.id as i64)
        .fetch_optional(pool)
        .await?;
    let prefs = prefs
        .and_then(|s| serde_json::from_str::<NoticePrefs>(&s).ok())
        .unwrap_or_default();
    Ok(Json(json!({ "code": 0, "message": "ok", "data": prefs })))
}

/// POST /user/prefs：保存当前用户的偏好设置（仅能改自己）。
pub async fn user_prefs_save(claims: Claims, Json(payload): Json<NoticePrefs>) -> ZapJsonResult {
    let mut data = payload;
    // 约束 AutoSSL 通知模式取值，避免非法字符进入
    match data.autossl_notify_mode.as_str() {
        "failures" | "disabled" => {}
        _ => data.autossl_notify_mode = "deferrals".to_string(),
    }
    let store = serde_json::to_string(&data).unwrap_or_default();
    let pool = db::get_db_pool().await;
    sqlx::query("UPDATE user SET prefs = ? WHERE id = ?")
        .bind(&store)
        .bind(claims.id as i64)
        .execute(pool)
        .await?;
    Ok(Json(
        json!({ "code": 0, "message": "偏好设置已保存", "data": data }),
    ))
}

/// 用户列表里随行带回的站点摘要：展开行的「站点详细」直接取用，
/// 前端不必再拉一次站点列表（也避免范围不一致 —— 这里已按 `users` 的可见集合收敛）。
#[derive(sqlx::FromRow)]
struct SiteBriefRow {
    id: i64,
    user_id: i64,
    name: String,
    status: i32,
    remark: String,
    php_instance: String,
    disk_used_bytes: i64,
    disk_stat_at: i64,
    created_at: Option<i64>,
}

/// List users — admin sees all, reseller sees only own customers
pub async fn user_list(claims: ValidatedClaims) -> ZapJsonResult {
    let is_admin = jwt::is_admin(&claims);
    let is_reseller = jwt::is_reseller(&claims);
    if !is_admin && !is_reseller {
        return Err(ZapError::New(-1, "权限不足".to_string()));
    }

    let pool = db::get_db_pool().await;

    // 用 `SELECT *` 与 `UserInfo` 保持同步：sqlx::FromRow 按「列名」取值，
    // 显式列清单一旦漏掉 user 表的新增列（如 disk_used_bytes），整行解析就会失败、
    // 接口 500，表现为页面「加载不到任何用户」。
    let mut querybuilder: QueryBuilder<'_, Sqlite> = QueryBuilder::new("SELECT * FROM user");
    if is_reseller && !is_admin {
        querybuilder
            .push(" WHERE owner_id = ")
            .push_bind(claims.id as i64);
    }
    querybuilder.push(" order by id desc");
    let users: Vec<UserInfo> = querybuilder.build_query_as().fetch_all(pool).await?;
    // 套餐名映射（列表展示用；未绑定套餐时 id 为 0，查不到即空串）
    let pkg_rows: Vec<(i64, String)> = sqlx::query_as("SELECT id, name FROM packages")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    let pkg_names: std::collections::HashMap<i64, String> = pkg_rows.into_iter().collect();
    // 归属用户名映射：列表里「成员 of X」直接取用，前端不必再查一次
    let name_of: std::collections::HashMap<i64, String> =
        users.iter().map(|u| (u.id, u.username.clone())).collect();

    // 站点摘要：一次取出可见用户的全部站点并按 user_id 分组，避免逐户查询（N+1）
    let ids: Vec<i64> = users.iter().map(|u| u.id).collect();
    let mut sites_by_user: std::collections::HashMap<i64, Vec<Value>> =
        std::collections::HashMap::new();
    if !ids.is_empty() {
        let mut site_qb = QueryBuilder::<Sqlite>::new(
            "SELECT id, user_id, name, status, remark, php_instance, disk_used_bytes, \
             disk_stat_at, created_at FROM site WHERE user_id IN (",
        );
        let mut sep = site_qb.separated(", ");
        for id in &ids {
            sep.push_bind(*id);
        }
        site_qb.push(") ORDER BY name");
        if let Ok(rows) = site_qb
            .build_query_as::<SiteBriefRow>()
            .fetch_all(pool)
            .await
        {
            for s in rows {
                sites_by_user.entry(s.user_id).or_default().push(json!({
                    "id": s.id,
                    "name": s.name,
                    "status": s.status,
                    "remark": s.remark,
                    "php_instance": s.php_instance,
                    "disk_used_bytes": s.disk_used_bytes,
                    "disk_stat_at": s.disk_stat_at,
                    "created_at": s.created_at.unwrap_or(0),
                }));
            }
        }
    }

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": users.iter().map(|user| {
            json!({
                "id": user.id,
                "username": user.username,
                "email": user.email,
                "phone": user.phone.clone().unwrap_or_default(),
                "nickname": user.nickname,
                "home_dir": user.home_dir,
                "linux_user": user.linux_user,
                "fpm_pool": user.fpm_pool,
                "fpm_spec_ref": user.fpm_spec_ref,
                "last_login_ip": user.last_login_ip,
                "last_login_time": user.last_login_time,
                "status": user.status,
                "roles": user.roles.split(',').collect::<Vec<&str>>(),
                "permissions": user.permissions.split(',').collect::<Vec<&str>>(),
                "owner_id": user.owner_id,
                "owner_username": name_of.get(&user.owner_id).cloned().unwrap_or_default(),
                // 成员（子账号）标记与父账号收紧清单
                "user_kind": user.user_kind,
                "perm_deny": user.perm_deny.split(',').filter(|s| !s.is_empty()).collect::<Vec<&str>>(),
                // 只读账号：共享可见但不可改
                "read_only": user.read_only != 0,
                "package_id": user.package_id,
                "package_name": pkg_names
                    .get(&user.package_id)
                    .cloned()
                    .unwrap_or_default(),
                // 资源用量：磁盘（定时 du 家目录）/ 本月流量（汇总站点 access.log）
                // disk_used_bytes 为 0 表示未采集或家目录不存在
                "disk_used_bytes": user.disk_used_bytes,
                "disk_stat_at": user.disk_stat_at,
                "bandwidth_used_bytes": user.bandwidth_used_bytes,
                "bandwidth_period": user.bandwidth_period,
                "bandwidth_stat_at": user.bandwidth_stat_at,
                // 该用户名下的站点摘要（列表行展开时的「站点详细」）
                "sites": sites_by_user.remove(&user.id).unwrap_or_default(),
                "created_at": user.created_at,
                "updated_at": user.updated_at,
            })
        }).collect::<Vec<Value>>(),
        "total": users.len(),
    })))
}

/// Create a new user — admin or reseller (own customer only)；
/// 任意登录用户都可创建挂在自己名下的「成员」（子账号，见 `USER_KIND_MEMBER`）。
pub async fn user_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CreateUserPayload>,
) -> ZapJsonResult {
    create_user_inner(&claims, &client_addr.ip().to_string(), payload).await
}

/// 创建用户的实际实现：`/system/user/add` 与 `/user/team/add` 共用。
async fn create_user_inner(
    claims: &jwt::Claims,
    ip: &str,
    payload: CreateUserPayload,
) -> ZapJsonResult {
    let is_admin = jwt::is_admin(claims);
    let is_reseller = jwt::is_reseller(claims);
    // 成员（子账号）：共享父账号的家目录与 Linux 系统账号，权限默认继承父账号
    let is_member_create = payload.user_kind.unwrap_or(0) == USER_KIND_MEMBER;
    // 层级固定一层：成员不能再建成员
    if user_is_member(claims.id as i64).await {
        return Err(ZapError::New(-1, "成员账号不能再创建成员".to_string()));
    }
    if !is_member_create && !is_admin && !is_reseller {
        return Err(ZapError::New(-1, "权限不足".to_string()));
    }

    let hashed = bcrypt::hash(&payload.password, bcrypt::DEFAULT_COST)
        .map_err(|e| ZapError::Error(format!("密码加密失败: {}", e)))?;

    let now = chrono::Local::now().timestamp();
    // reseller 只能创建普通用户客户；admin 可指定角色
    let roles = if is_admin {
        payload.roles.unwrap_or_else(|| "user".to_string())
    } else {
        "user".to_string()
    };
    // 昵称可留空（空串/空白同样按未填写处理），默认与用户名同名
    let nickname = payload
        .nickname
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| payload.username.clone());
    // 空手机号存 NULL，避免 UNIQUE 约束下多个空串互相冲突
    let phone = payload
        .phone
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());

    let mut package = None;
    let package_id: i64;
    let mut fpm_pool: String;
    let mut fpm_spec_ref: String;
    let home_dir: String;
    let lu: String;
    let mut owner_id: i64 = claims.id as i64;

    if is_member_create {
        // ── 成员分支：共享父账号的家目录 / 系统账号 / 套餐，不建自己的运行实体 ──
        // 归属默认建在自己名下；admin 可挂到指定用户下
        if is_admin && let Some(oid) = payload.owner_id.filter(|v| *v > 0) {
            owner_id = oid;
        }
        let parent = load_account(owner_id)
            .await?
            .ok_or_else(|| ZapError::New(-1, "父账号不存在".to_string()))?;
        if parent.user_kind == USER_KIND_MEMBER {
            return Err(ZapError::New(
                -1,
                "成员账号下不能再创建成员（只支持一层）".to_string(),
            ));
        }
        if parent.home_dir.is_empty() || parent.linux_user.is_empty() {
            return Err(ZapError::New(
                -1,
                "父账号尚未初始化家目录与系统账号，无法创建成员".to_string(),
            ));
        }
        home_dir = parent.home_dir;
        lu = parent.linux_user;
        package_id = parent.package_id;
        // PHP-FPM：成员复用父账号的 pool，不单独配置规格
        fpm_pool = String::new();
        fpm_spec_ref = String::new();
    } else {
        // reseller 创建的客户归属自己；admin 可指定归属（默认系统直属）
        owner_id = if is_admin {
            payload.owner_id.unwrap_or(0)
        } else {
            claims.id as i64
        };
        // 套餐：校验可见性（admin 全部；reseller 仅全局与自己名下）
        package = match payload.package_id.filter(|v| *v > 0) {
            Some(pid) => Some(
                crate::routers::package::load_for_actor(pid, is_admin, claims.id as i64).await?,
            ),
            None => None,
        };
        package_id = package.as_ref().map(|p| p.id).unwrap_or(0);
        // fpm 规格：fpm_pool（旧版自定义 JSON）与 fpm_spec_ref（模板/继承/默认）互斥。
        // 前端新流程只传 fpm_spec_ref；一旦显式指定引用，不再保留自定义 JSON（避免遮蔽模板）。
        fpm_pool = normalize_fpm_spec(payload.fpm_pool)?.unwrap_or_default();
        fpm_spec_ref = payload.fpm_spec_ref.unwrap_or_default().trim().to_string();
        // 未显式选择模板时继承套餐绑定的 FPM 规格模板
        if fpm_spec_ref.is_empty()
            && let Some(p) = &package
            && !p.fpm_spec_ref.trim().is_empty()
        {
            fpm_spec_ref = p.fpm_spec_ref.trim().to_string();
            // 套餐模板可能由 admin 创建，reseller 使用时同样校验可见性
            crate::routers::fpm_spec::validate_spec_ref(&fpm_spec_ref, is_admin, &claims.sub)
                .await?;
        }
        if !fpm_spec_ref.is_empty() {
            fpm_pool.clear();
        }
        // 引用校验：reseller 只能选自己名下或全局通用模板；admin 校验模板存在性
        if fpm_spec_ref.is_empty() || fpm_spec_ref == crate::routers::fpm_spec::INHERIT {
            // '' 与 inherit 恒允许
        } else if is_admin {
            crate::routers::fpm_spec::validate_spec_ref(&fpm_spec_ref, true, "").await?;
        } else {
            crate::routers::fpm_spec::validate_spec_ref(&fpm_spec_ref, false, &claims.sub).await?;
        }

        // 家目录 / Linux 账号：{默认挂载点}/{linux_username(username)} 派生，
        // 站点文档根与站点日志均规划于其下；派生名与已有账号冲突时追加 -n 后缀。
        // 默认挂载点取自运行环境默认设置（conf: user_home_root，默认 /home）；
        // /home 磁盘不足时管理员可切换到新挂载点（如 /home2），此后新建用户即落到新挂载点，
        // 存量用户不受影响（数据迁移请使用「服务器配置 → 数据迁移」）。
        let lu_base = zap_proto::linux_username(&payload.username);
        let pool = db::get_db_pool().await;
        let mut cand = lu_base.clone();
        let mut n: i64 = 2;
        loop {
            let cnt: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM user WHERE linux_user = ? AND linux_user != ''",
            )
            .bind(&cand)
            .fetch_one(pool)
            .await
            .unwrap_or((0,));
            if cnt.0 == 0 {
                break;
            }
            cand = format!("{lu_base}-{n}");
            n += 1;
        }
        let home_root = crate::zap::server_env::conf_get("user_home_root")
            .filter(|s| s.starts_with('/') && !s.contains("..") && s.len() > 1)
            .unwrap_or_else(|| "/home".to_string());
        lu = cand;
        home_dir = format!("{home_root}/{lu}");
    }

    // 附加权限：预先归一化（只保留权限目录内合法 key），供写入与审计共用
    let granted_perms = payload
        .permissions
        .as_ref()
        .map(|p| p.normalize())
        .unwrap_or_default();
    // 父账号对成员的收紧清单（仅成员生效），同样只保留合法 key
    let denied_perms = payload
        .perm_deny
        .as_ref()
        .map(|p| p.normalize())
        .unwrap_or_default();

    let pool = db::get_db_pool().await;
    let result = sqlx::query(
        "INSERT INTO user (username, home_dir, linux_user, fpm_pool, fpm_spec_ref, password, email, phone, nickname, roles, permissions, owner_id, user_kind, perm_deny, read_only, package_id, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
    )
    .bind(&payload.username)
    .bind(&home_dir)
    .bind(&lu)
    .bind(&fpm_pool)
    .bind(&fpm_spec_ref)
    .bind(&hashed)
    .bind(&payload.email)
    .bind(phone)
    .bind(&nickname)
    .bind(&roles)
    .bind(&granted_perms)
    .bind(owner_id)
    .bind(if is_member_create { USER_KIND_MEMBER } else { 0 })
    .bind(&denied_perms)
    // 只读：共享可见但不可改（生效权限只保留 {ns}:view）
    .bind(if payload.read_only.unwrap_or(false) { 1 } else { 0 })
    .bind(package_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await;

    match result {
        Ok(r) => {
            audit::log(
                Some(claims),
                Some(ip),
                "user_create",
                &format!("id={}", r.last_insert_rowid()),
                &format!("username={}", payload.username),
            )
            .await;
            // 生效权限缓存是全量加载的：新建的用户不在缓存里，必须立即失效，
            // 否则 `/user/info` 回给它空权限，前端 v-permission 会把它的按钮
            // （SSL 等模块）全判成无权限，直到下次用户变更或重启才恢复
            crate::routers::access::invalidate_user_perm_cache();
            // 建号即带附加权限 / 收紧清单 / 只读标记 → 单独审计
            if payload.permissions.is_some()
                || payload.perm_deny.is_some()
                || payload.read_only.is_some()
            {
                crate::routers::access::invalidate_user_perm_cache();
                audit_permissions_set(
                    claims,
                    ip,
                    r.last_insert_rowid(),
                    &format!(
                        "grant=[{}] deny=[{}] read_only={}",
                        granted_perms,
                        denied_perms,
                        payload.read_only.unwrap_or(false)
                    ),
                )
                .await;
            }
            info!(
                "User created: {} (id: {})",
                payload.username,
                r.last_insert_rowid()
            );
            // 按全局运行模式补齐运行实体（system=Linux 账号 / www=家目录骨架）。
            // 尽力而为：失败仅告警，站点同步时仍会递归补齐。
            // 成员共享父账号的运行实体，不需要也不能建自己的系统账号。
            let new_id = r.last_insert_rowid();
            if !is_member_create {
                if let Err(e) = ensure_user_runtime(new_id).await {
                    warn!("初始化用户运行实体失败(id={}): {}", new_id, e);
                }
                // 套餐：下发磁盘配额（系统账号就绪后执行，best-effort）
                if package.is_some() {
                    sync_package_quota(new_id).await;
                }
            }
            Ok(Json(json!({
                "code": 0,
                "message": "用户创建成功",
                "data": { "id": new_id, "home_dir": home_dir, "linux_user": lu }
            })))
        }
        Err(e) => {
            if let sqlx::Error::Database(db_err) = &e {
                let msg = db_err.message();
                if msg.contains("user.phone") {
                    return Err(ZapError::New(-1, "手机号已被其他用户使用".to_string()));
                }
                if msg.contains("user.email") {
                    return Err(ZapError::New(-1, "邮箱已存在".to_string()));
                }
                if msg.contains("user.username") {
                    return Err(ZapError::New(-1, "用户名已存在".to_string()));
                }
            }
            if e.to_string().contains("UNIQUE") {
                Err(ZapError::New(-1, "用户名、邮箱或手机号已存在".to_string()))
            } else {
                Err(ZapError::from(e))
            }
        }
    }
}

/// Update an existing user.
/// - admin: any user
/// - reseller: own customers only, and cannot change roles
/// - 任意用户：自己的成员（子账号），可改基本信息 / 状态 / 密码 / 权限收紧
pub async fn user_update(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<UpdateUserPayload>,
) -> ZapJsonResult {
    update_user_inner(&claims, &client_addr.ip().to_string(), payload).await
}

/// 更新用户的实际实现：`/system/user/update` 与 `/user/team/update` 共用。
async fn update_user_inner(
    claims: &jwt::Claims,
    ip: &str,
    payload: UpdateUserPayload,
) -> ZapJsonResult {
    // 内置管理员保护：不可禁用、角色不可变更；其余信息（含密码）仅本人可改
    if payload.id == ROOT_ADMIN_ID {
        if payload.status == Some(0) {
            return Err(ZapError::New(-1, "内置管理员账号不可禁用".to_string()));
        }
        if payload.roles.is_some() {
            return Err(ZapError::New(-1, "内置管理员的角色不可变更".to_string()));
        }
        if claims.id as i64 != ROOT_ADMIN_ID {
            return Err(ZapError::New(
                -1,
                "内置管理员账号只能由本人修改".to_string(),
            ));
        }
    }
    // 非管理员不能修改角色（防止提权，admin 除外）
    if !jwt::is_admin(claims) && payload.roles.is_some() {
        return Err(ZapError::New(-1, "权限不足，不能修改角色".to_string()));
    }
    // PHP-FPM pool 规格（资源配额类）仅管理员可配置
    if !jwt::is_admin(claims) && payload.fpm_pool.is_some() {
        return Err(ZapError::New(
            -1,
            "权限不足，不能修改 PHP-FPM 自定义规格".to_string(),
        ));
    }
    // fpm_spec_ref（模板 / 继承）：reseller 可为自己客户设置，但只能选自己名下或全局通用模板
    if !jwt::is_admin(claims) && payload.fpm_spec_ref.is_some() {
        let rv = payload.fpm_spec_ref.as_deref().unwrap_or("").trim();
        if !rv.is_empty() && rv != crate::routers::fpm_spec::INHERIT {
            crate::routers::fpm_spec::validate_spec_ref(rv, false, &claims.sub).await?;
        }
    }
    // 套餐：admin 全部可用；reseller 仅全局套餐与自己名下套餐
    if let Some(pid) = payload.package_id
        && pid > 0
    {
        crate::routers::package::load_for_actor(pid, jwt::is_admin(claims), claims.id as i64)
            .await?;
    }

    // 更新他人时的归属/权限校验
    if payload.id != claims.id as i64 {
        if jwt::is_admin(claims) {
            // admin: full access
        } else if jwt::is_reseller(claims) {
            // reseller: own customers only
            let owner_id = get_user_owner_id(payload.id).await?;
            if owner_id != claims.id as i64 {
                return Err(ZapError::New(
                    -1,
                    "权限不足，只能管理自己的客户".to_string(),
                ));
            }
        } else if is_own_member(claims, payload.id).await {
            // 父账号管理自己的成员：角色 / FPM / 套餐类字段由下面的通用规则继续收敛
        } else {
            require_admin(claims)?;
        }
    }

    let pool = db::get_db_pool().await;
    let now = chrono::Local::now().timestamp();

    let has_any_field = payload.email.is_some()
        || payload.phone.is_some()
        || payload.nickname.is_some()
        || payload.roles.is_some()
        || payload.status.is_some()
        || payload.password.is_some()
        || payload.fpm_pool.is_some()
        || payload.fpm_spec_ref.is_some()
        || payload.package_id.is_some()
        || payload.permissions.is_some()
        || payload.perm_deny.is_some()
        || payload.read_only.is_some();

    if !has_any_field {
        return Err(ZapError::New(-1, "没有需要更新的字段".to_string()));
    }

    let mut qb: QueryBuilder<'_, Sqlite> = QueryBuilder::new("UPDATE user SET ");
    let mut separated = qb.separated(", ");

    if let Some(ref email) = payload.email {
        separated.push("email = ").push_bind_unseparated(email);
    }
    if let Some(ref phone) = payload.phone {
        let p = phone.trim();
        if p.is_empty() {
            // 空手机号清空为 NULL
            separated
                .push("phone = ")
                .push_bind_unseparated(Option::<String>::None);
        } else {
            separated.push("phone = ").push_bind_unseparated(p);
        }
    }
    if let Some(ref nickname) = payload.nickname {
        let n = nickname.trim();
        if n.is_empty() {
            // 清空昵称时回退为用户名，避免列表/展示出现空白
            separated.push("nickname = username");
        } else {
            separated.push("nickname = ").push_bind_unseparated(n);
        }
    }
    if let Some(ref roles) = payload.roles {
        separated.push("roles = ").push_bind_unseparated(roles);
    }
    if let Some(ref perms) = payload.permissions {
        separated
            .push("permissions = ")
            .push_bind_unseparated(perms.normalize());
    }
    if let Some(ref deny) = payload.perm_deny {
        separated
            .push("perm_deny = ")
            .push_bind_unseparated(deny.normalize());
    }
    if let Some(ro) = payload.read_only {
        separated
            .push("read_only = ")
            .push_bind_unseparated(if ro { 1 } else { 0 });
    }
    if let Some(status) = payload.status {
        separated.push("status = ").push_bind_unseparated(status);
    }
    if let Some(ref password) = payload.password {
        let hashed = bcrypt::hash(password, bcrypt::DEFAULT_COST)
            .map_err(|e| ZapError::Error(format!("密码加密失败: {}", e)))?;
        separated.push("password = ").push_bind_unseparated(hashed);
    }
    if let Some(ref fpm) = payload.fpm_pool {
        let norm = normalize_fpm_spec(Some(fpm.clone()))?.unwrap_or_default();
        separated
            .push("fpm_pool = ")
            .push_bind_unseparated(norm.clone());
        // 自定义 JSON 与模板引用互斥：提交自定义时清空引用
        if payload.fpm_spec_ref.is_none() {
            separated
                .push("fpm_spec_ref = ")
                .push_bind_unseparated(String::new());
        }
    }
    if let Some(ref rv) = payload.fpm_spec_ref {
        let norm = rv.trim().to_string();
        if jwt::is_admin(claims) && !norm.is_empty() && norm != crate::routers::fpm_spec::INHERIT {
            crate::routers::fpm_spec::validate_spec_ref(&norm, true, "").await?;
        }
        separated
            .push("fpm_spec_ref = ")
            .push_bind_unseparated(norm);
        // 切到「模板 / 继承 / 面板默认」后清除旧的自定义 JSON，避免遮蔽新选择
        separated
            .push("fpm_pool = ")
            .push_bind_unseparated(String::new());
    }
    if let Some(pid) = payload.package_id {
        separated
            .push("package_id = ")
            .push_bind_unseparated(pid.max(0));
    }
    separated.push("updated_at = ").push_bind_unseparated(now);

    qb.push(" WHERE id = ").push_bind(payload.id);

    let result = match qb.build().execute(pool).await {
        Ok(r) => r,
        Err(sqlx::Error::Database(db_err)) if db_err.message().contains("user.phone") => {
            return Err(ZapError::New(-1, "手机号已被其他用户使用".to_string()));
        }
        Err(sqlx::Error::Database(db_err)) if db_err.message().contains("user.email") => {
            return Err(ZapError::New(-1, "邮箱已被其他用户使用".to_string()));
        }
        Err(e) => return Err(ZapError::from(e)),
    };

    if result.rows_affected() == 0 {
        return Err(ZapError::New(-1, "用户不存在".to_string()));
    }

    audit::log(
        Some(claims),
        Some(ip),
        "user_update",
        &format!("id={}", payload.id),
        "",
    )
    .await;

    // 附加权限 / 收紧清单 / 只读标记变更：单独审计；这几项与角色都会改变生效权限，
    // 必须立刻失效缓存（收紧清单会改变成员的生效权限，不能等缓存过期）
    let perm_changed =
        payload.permissions.is_some() || payload.perm_deny.is_some() || payload.read_only.is_some();
    if perm_changed || payload.roles.is_some() {
        crate::routers::access::invalidate_user_perm_cache();
    }
    if perm_changed {
        audit_permissions_set(
            claims,
            ip,
            payload.id,
            &format!(
                "grant=[{}] deny=[{}] read_only={}",
                payload
                    .permissions
                    .as_ref()
                    .map(|p| p.normalize())
                    .unwrap_or_default(),
                payload
                    .perm_deny
                    .as_ref()
                    .map(|p| p.normalize())
                    .unwrap_or_default(),
                payload.read_only.unwrap_or(false),
            ),
        )
        .await;
    }

    // 套餐变更 → 重新下发磁盘配额（best-effort）
    if payload.package_id.is_some() {
        sync_package_quota(payload.id).await;
    }

    let resp = json!({ "code": 0, "message": "用户更新成功" });
    // 密码被修改（本人或管理员/经销商改密）→ 向目标用户发站内信（受其通知偏好控制）
    if payload.password.is_some() {
        crate::zap::notify::password_changed(payload.id, &claims.sub).await;
    }
    Ok(Json(resp))
}

/// Delete a user — admin: any; reseller: own customers only；任意用户可删除自己的成员
pub async fn user_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<DeleteUserPayload>,
) -> ZapJsonResult {
    delete_user_inner(&claims, &client_addr.ip().to_string(), payload.id).await
}

/// 删除用户的实际实现：`/system/user/delete` 与 `/user/team/delete` 共用。
async fn delete_user_inner(claims: &jwt::Claims, ip: &str, id: i64) -> ZapJsonResult {
    if id == ROOT_ADMIN_ID {
        return Err(ZapError::New(-1, "内置管理员账号不可删除".to_string()));
    }
    if jwt::is_admin(claims) {
        // admin: full access (still cannot delete self)
    } else if jwt::is_reseller(claims) {
        // reseller: own customers only
        let owner_id = get_user_owner_id(id).await?;
        if owner_id != claims.id as i64 {
            return Err(ZapError::New(
                -1,
                "权限不足，只能删除自己的客户".to_string(),
            ));
        }
    } else if is_own_member(claims, id).await {
        // 父账号删除自己的成员
    } else {
        require_admin(claims)?;
    }

    if id == claims.id as i64 {
        return Err(ZapError::New(-1, "不能删除自己".to_string()));
    }

    let pool = db::get_db_pool().await;

    // 目标账号的归属信息：成员共享父账号的系统账号，删除时绝不能连带 userdel
    let is_member = user_is_member(id).await;
    // 名下还有成员时不允许删除：成员会变成无主账号（归属落空且无人可管）
    let member_cnt: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM user WHERE owner_id = ? AND user_kind = 1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    if member_cnt > 0 {
        return Err(ZapError::New(
            -1,
            format!("该用户名下还有 {member_cnt} 个成员账号，请先删除这些成员"),
        ));
    }

    // 独立系统用户模式下，站点的 vhost / FPM pool / 目录都绑定归属用户的 Linux 账号，
    // 直接删除会让这些配置全部悬空且无法回收，因此要求先处理站点
    let site_cnt: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM site WHERE user_id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    if site_cnt > 0 {
        return Err(ZapError::New(
            -1,
            format!("该用户名下还有 {site_cnt} 个站点，请先删除或转移站点后再删除用户"),
        ));
    }

    // 先记录待清理的 Linux 账号（删除用户后按它清 pool + userdel）。
    // 成员共享父账号的系统账号，这里必须留空，否则会把父账号一起 userdel 掉。
    let linux_user: Option<String> = if is_member {
        None
    } else {
        sqlx::query_scalar("SELECT linux_user FROM user WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .filter(|s: &String| !s.is_empty())
    };

    let result = sqlx::query("DELETE FROM user WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ZapError::New(-1, "用户不存在".to_string()));
    }

    // 用户级菜单例外随账号回收，避免留下无主行
    let _ = sqlx::query("DELETE FROM user_menus WHERE user_id = ?")
        .bind(id)
        .execute(pool)
        .await;

    // 清理该用户的 SSL 证书：先解除站点 HTTPS 绑定引用，再删除证书，避免悬空
    let cert_ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM ssl_cert WHERE user_id = ?")
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    for cid in cert_ids {
        let _ = sqlx::query(
            "UPDATE site_profile SET ssl_cert_id = 0, force_https = 0 WHERE ssl_cert_id = ?",
        )
        .bind(cid)
        .execute(pool)
        .await;
        let _ = sqlx::query("DELETE FROM ssl_cert WHERE id = ?")
            .bind(cid)
            .execute(pool)
            .await;
    }

    // 删除用户后清理运行实体：清掉该账号在所有 PHP 实例中的 pool 并 userdel
    if let Some(lu) = linux_user {
        match crate::zapexec::call(zap_proto::types::Request::UserSystemRemove {
            linux_user: lu.clone(),
        })
        .await
        {
            Ok(resp) if resp.code != 0 => {
                warn!("清理 Linux 账号失败(id={}): {}", id, resp.message);
            }
            Err(e) => {
                warn!("清理 Linux 账号失败(id={}): {}", id, e);
            }
            _ => {
                info!("Linux 账号已清理: {} (user id={})", lu, id);
            }
        }
    }

    crate::routers::access::invalidate_user_perm_cache();

    audit::log(
        Some(claims),
        Some(ip),
        "user_delete",
        &format!("id={id}"),
        "",
    )
    .await;

    info!("User deleted: id={}", id);
    Ok(Json(json!({
        "code": 0,
        "message": "用户删除成功"
    })))
}

// ── 团队成员（子账号）────────────────────────────────────────
// 成员共享父账号的家目录与 Linux 系统账号（同一个 uid），权限默认继承父账号的
// 生效权限，父账号可通过 `perm_deny` 再收紧。层级固定一层：成员不能再建成员。
//
// 这几个接口的角色下限是普通用户（`/user` 前缀），让任何用户都能管理自己的团队，
// 不必拥有「用户管理」模块（system.user:*）的权限。

#[derive(Debug, Deserialize)]
pub struct TeamAddPayload {
    pub username: String,
    pub password: String,
    pub email: String,
    pub phone: Option<String>,
    pub nickname: Option<String>,
    /// 父账号对该成员收紧（取消）的权限点（逗号分隔或数组）
    #[serde(default)]
    pub perm_deny: Option<PermissionInput>,
    /// 只读成员：可见但不可改（生效权限只保留 `{ns}:view`）
    #[serde(default)]
    pub read_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TeamUpdatePayload {
    pub id: i64,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub nickname: Option<String>,
    pub status: Option<i32>,
    pub password: Option<String>,
    /// 收紧清单（不传 = 不改动；传空数组/空串 = 取消全部收紧）
    #[serde(default)]
    pub perm_deny: Option<PermissionInput>,
    /// 只读开关（不传 = 不改动）
    #[serde(default)]
    pub read_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TeamDeletePayload {
    pub id: i64,
}

/// 当前用户名下的成员列表（含共享的家目录 / 系统账号，便于前端说明「共用父账号」）。
pub async fn team_list(claims: Claims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let rows: Vec<TeamRow> = sqlx::query_as(
        "SELECT id, username, nickname, email, phone, status, roles, permissions, perm_deny, \
                read_only, home_dir, linux_user, last_login_time, last_login_ip, created_at, updated_at \
         FROM user WHERE owner_id = ? AND user_kind = 1 ORDER BY id DESC",
    )
    .bind(claims.id as i64)
    .fetch_all(pool)
    .await?;

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": rows.iter().map(|r| {
            json!({
                "id": r.id,
                "username": r.username,
                "nickname": r.nickname,
                "email": r.email,
                "phone": r.phone.clone().unwrap_or_default(),
                "status": r.status,
                "roles": r.roles.split(',').collect::<Vec<&str>>(),
                "permissions": r.permissions.split(',').filter(|s| !s.is_empty()).collect::<Vec<&str>>(),
                "perm_deny": r.perm_deny.split(',').filter(|s| !s.is_empty()).collect::<Vec<&str>>(),
                // 只读成员：共享可见但不可改
                "read_only": r.read_only != 0,
                // 共享父账号的家目录与系统账号（展示用：说明成员不单独建账号）
                "home_dir": r.home_dir,
                "linux_user": r.linux_user,
                "last_login_time": r.last_login_time,
                "last_login_ip": r.last_login_ip,
                "created_at": r.created_at,
                "updated_at": r.updated_at,
            })
        }).collect::<Vec<Value>>(),
        "total": rows.len(),
    })))
}

#[derive(sqlx::FromRow)]
struct TeamRow {
    id: i64,
    username: String,
    nickname: String,
    email: String,
    phone: Option<String>,
    status: i32,
    roles: String,
    permissions: String,
    perm_deny: String,
    /// 只读成员标记：1=只能查看
    read_only: i32,
    home_dir: String,
    linux_user: String,
    last_login_time: i64,
    last_login_ip: String,
    created_at: i64,
    updated_at: i64,
}

/// 新增成员（强制挂在自己名下、强制 user_kind=成员）。
pub async fn team_add(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<TeamAddPayload>,
) -> ZapJsonResult {
    let inner = CreateUserPayload {
        username: payload.username,
        password: payload.password,
        email: payload.email,
        phone: payload.phone,
        nickname: payload.nickname,
        roles: None,
        owner_id: Some(claims.id as i64),
        fpm_pool: None,
        fpm_spec_ref: None,
        package_id: None,
        permissions: None,
        user_kind: Some(USER_KIND_MEMBER),
        perm_deny: payload.perm_deny,
        read_only: payload.read_only,
    };
    create_user_inner(&claims, &client_addr.ip().to_string(), inner).await
}

/// 修改自己的成员（基本信息 / 状态 / 密码 / 权限收紧）。
pub async fn team_update(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<TeamUpdatePayload>,
) -> ZapJsonResult {
    let inner = UpdateUserPayload {
        id: payload.id,
        email: payload.email,
        phone: payload.phone,
        nickname: payload.nickname,
        roles: None,
        status: payload.status,
        password: payload.password,
        fpm_pool: None,
        fpm_spec_ref: None,
        package_id: None,
        permissions: None,
        perm_deny: payload.perm_deny,
        read_only: payload.read_only,
    };
    update_user_inner(&claims, &client_addr.ip().to_string(), inner).await
}

/// 删除自己的成员（不会连带删除共享的父账号系统账号）。
pub async fn team_delete(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<TeamDeletePayload>,
) -> ZapJsonResult {
    delete_user_inner(&claims, &client_addr.ip().to_string(), payload.id).await
}

/// List all reseller users — admin only (used to assign customer ownership)
pub async fn reseller_list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;

    let pool = db::get_db_pool().await;
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, username, nickname FROM user WHERE roles LIKE '%reseller%' ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": rows.iter().map(|(id, username, nickname)| {
            json!({ "id": id, "username": username, "nickname": nickname })
        }).collect::<Vec<Value>>(),
    })))
}

/// 批量补齐所有用户运行实体（admin only）：
/// 为每个用户创建 Linux 系统账号（nologin）并初始化其家目录骨架。
/// 个别失败不影响整体（结果里给出失败清单）。
pub async fn user_home_sync(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
) -> ZapJsonResult {
    require_admin(&claims)?;

    let pool = db::get_db_pool().await;
    // 成员共享父账号的运行实体，跳过（父账号自己会补齐，重复执行没有意义）
    let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT id, username, home_dir, linux_user FROM user WHERE home_dir != '' AND user_kind != 1 ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let mut ok_items: Vec<Value> = Vec::new();
    let mut fail_items: Vec<Value> = Vec::new();
    for (id, username, home_dir, linux_user) in rows {
        match ensure_user_runtime(id).await {
            Ok(()) => {
                ok_items.push(json!({
                    "id": id,
                    "username": username,
                    "home_dir": home_dir,
                    "linux_user": linux_user,
                    "mode": "system",
                }));
            }
            Err(e) => {
                fail_items.push(json!({
                    "id": id,
                    "username": username,
                    "home_dir": home_dir,
                    "linux_user": linux_user,
                    "mode": "system",
                    "error": e,
                }));
            }
        }
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "user_home_sync",
        &format!("ok={} fail={}", ok_items.len(), fail_items.len()),
        "mode=system",
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": format!("运行实体同步完成：成功 {}，失败 {}", ok_items.len(), fail_items.len()),
        "data": { "ok": ok_items, "fail": fail_items, "mode": "system" }
    })))
}

// ── 家目录备份 ────────────────────────────────────────────
// 把某个面板用户的家目录整体打包成 `{home}/backups/home_backup_<时间戳>.tar.gz`
// （排除 `backups` 目录自身，避免把上一份备份再套进去）。
//
// 打包以该用户的 Linux 账号身份执行（复用 crontab 的 CronRun 动词），产出文件
// 自然归该用户所有；过程可能持续很久，故走后台 run + 日志的形式，接口只返回
// `run_id`，前端轮询通用运行记录判断成败。

#[derive(Debug, Deserialize)]
pub struct BackupHomePayload {
    /// 目标面板用户名（留空 = 自己）；非 admin 只能备份自己
    pub username: Option<String>,
}

/// 家目录备份的运行记录归类键（同一用户的历史备份串在一起）。
fn backup_run_key(username: &str) -> String {
    format!("home_backup:{username}")
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// zapexec 在任务结束时追加的完成标记：`__ZAP_DONE__ <exit_code>`。
async fn read_done_marker(log: &str) -> Option<i64> {
    let content = tokio::fs::read_to_string(log).await.ok()?;
    content.lines().rev().find_map(|line| {
        line.trim()
            .strip_prefix("__ZAP_DONE__ ")?
            .trim()
            .parse::<i64>()
            .ok()
    })
}

/// 后台盯住日志文件，出现完成标记（或超时）即落定运行记录状态。
fn watch_home_backup(run_id: String, log: String) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(24 * 3600);
        loop {
            if let Some(code) = read_done_marker(&log).await {
                ast::finish_run(&run_id, if code == 0 { "success" } else { "failed" }, code).await;
                break;
            }
            if tokio::time::Instant::now() > deadline {
                ast::finish_run(&run_id, "failed", -1).await;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

/// POST /system/user/backup-home
pub async fn backup_home(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<BackupHomePayload>,
) -> ZapJsonResult {
    let target = payload
        .username
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .unwrap_or(&claims.sub)
        .to_string();
    // 代客备份限管理员：普通账号只碰得动自己的家目录
    if target != claims.sub && !jwt::is_admin(&claims) {
        return Err(ZapError::New(-1, "只能备份自己的家目录".to_string()));
    }

    let pool = db::get_db_pool().await;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT home_dir, linux_user FROM user WHERE username = ?")
            .bind(&target)
            .fetch_optional(pool)
            .await?;
    let Some((home, linux_user)) = row else {
        return Err(ZapError::New(-1, format!("用户 {target} 不存在")));
    };
    if home.is_empty() {
        return Err(ZapError::New(
            -1,
            format!("用户 {target} 的家目录尚未初始化"),
        ));
    }
    if linux_user.is_empty() {
        return Err(ZapError::New(
            -1,
            format!("用户 {target} 尚未创建 Linux 运行账号"),
        ));
    }

    let home = home.trim_end_matches('/').to_string();
    let backups_dir = format!("{home}/backups");
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = format!("{backups_dir}/home_backup_{stamp}.tar.gz");
    let command = format!(
        "mkdir -p {} && tar -czf {} -C {} --exclude=./backups .",
        sh_quote(&backups_dir),
        sh_quote(&dest),
        sh_quote(&home)
    );

    let run_id = ast::generate_run_id();
    let log = user_cron::log_path(&target, &run_id);
    ast::register_run_with_key(
        &run_id,
        "home-backup",
        "home-backup",
        &target,
        &log,
        &backup_run_key(&target),
    )
    .await?;

    let resp = crate::zapexec::call(zap_proto::types::Request::CronRun {
        run_id: run_id.clone(),
        linux_user,
        home_dir: home.clone(),
        command,
        kind: String::new(),
        log_path: log.clone(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(&run_id, "failed", resp.code as i64).await;
        return Err(ZapError::New(resp.code, resp.message));
    }
    watch_home_backup(run_id.clone(), log.clone());

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "home_backup",
        &format!("{target}: {home} → {dest}"),
        &run_id,
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": "家目录备份已开始",
        "data": { "run_id": run_id, "username": target, "path": dest, "log": log }
    })))
}
