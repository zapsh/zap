//! 数据库管理（MySQL / MariaDB）。
//!
//! 通过本机 `mysql` 客户端 + `zapadm` 凭据（由 zap-crypto 从
//! /etc/zap/credentials 解密读取）执行管理操作，**不使用 root 账号**：
//!
//! - GET  /api/database/status          服务状态与版本
//! - GET  /api/database/list            库列表（含大小 / 表数 / 字符集）
//! - POST /api/database/create          创建库（可一并创建同名用户并授予该库权限）
//! - POST /api/database/drop            删除库
//! - GET  /api/database/users           用户列表（及其授权）
//! - POST /api/database/user/create     创建用户并授权到指定库
//! - POST /api/database/user/drop       删除用户
//! - GET  /api/database/remote          远程访问授权列表
//! - POST /api/database/remote/grant    授权某主机远程访问某库
//! - POST /api/database/remote/revoke   撤销远程授权
//!
//! 多租户：非管理员只能看到并操作以「用户名_」为前缀的库，
//! 创建库时会自动补上该前缀；管理员不受限制。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use axum::Json;
use axum::extract::Query;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::jwt::{self, ValidatedClaims, is_admin};

/// 凭据坐标（与 `zapctl cred show mysql zapadm` 一致）
const CRED_SERVICE: &str = "mysql";
const CRED_USER: &str = "zapadm";

/// 面板展示用的连接地址主机位（本机管理走 socket，对外连接统一提示回环地址）
pub(crate) const DB_HOST: &str = "127.0.0.1";

/// 服务端口取不到时的兜底值
const DEFAULT_PORT: u16 = 3306;

/// mysql 客户端候选路径（按优先级）
const MYSQL_BINS: &[&str] = &[
    "/usr/local/mysql/bin/mysql",
    "/usr/local/apps/mysql-8.4/bin/mysql",
    "/usr/bin/mysql",
];

/// 本机 socket 候选（本地连接优先走 socket，无需开 TCP）
const SOCKETS: &[&str] = &[
    "/tmp/mysql.sock",
    "/var/run/mysqld/mysqld.sock",
    "/run/mysqld/mysqld.sock",
];

/// 系统库：列表与统计中一律排除
const SYSTEM_SCHEMAS: &[&str] = &["information_schema", "mysql", "performance_schema", "sys"];

// ── 连接与执行 ──────────────────────────────────────────────

fn mysql_bin() -> Result<PathBuf, ZapError> {
    MYSQL_BINS
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
        .ok_or_else(|| {
            ZapError::New(
                -1,
                "未找到 mysql 客户端，请先安装 MySQL / MariaDB".to_string(),
            )
        })
}

fn socket_path() -> Option<&'static str> {
    SOCKETS
        .iter()
        .copied()
        .find(|s| std::path::Path::new(s).exists())
}

/// 从 /etc/zap/credentials 解密读取 zapadm 密码
///
/// 凭据文件由 root 侧（zapctl / zapexec）写入、属组为面板组 zapadm、权限 0440，
/// 凭据目录 0750 —— 本进程以 zapadm 运行正是靠这个属组读到的。
/// 若目录/文件退回 root-only（0700 / 0400），本函数会报「凭据不存在」，
/// 但文件其实在：用 `ls -ld /etc/zap/credentials /etc/zap/credentials/*.cred` 核对。
fn zapadm_password() -> Result<String, ZapError> {
    zap_crypto::read_cred(CRED_SERVICE, CRED_USER)
        .map_err(|e| ZapError::New(-1, format!("读取数据库凭据失败：{e}")))
}

/// 执行 SQL，返回「无表头 + TAB 分隔」的输出。
///
/// 密码通过 `MYSQL_PWD` 环境变量传递，避免出现在进程命令行里。
fn run_sql(sql: &str) -> Result<String, ZapError> {
    let bin = mysql_bin()?;
    let pwd = zapadm_password()?;

    let mut cmd = Command::new(bin);
    cmd.env("MYSQL_PWD", &pwd);
    if let Some(sock) = socket_path() {
        cmd.arg("--socket").arg(sock);
    }
    cmd.arg("-u")
        .arg(CRED_USER)
        .arg("-N")
        .arg("-B")
        .arg("-e")
        .arg(sql);

    let out = cmd
        .output()
        .map_err(|e| ZapError::New(-1, format!("执行数据库命令失败：{e}")))?;

    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(ZapError::New(-1, format!("数据库操作失败：{err}")))
    }
}

/// 执行若干条语句（用分号分隔，内部自行转义标识符）。
fn run_sqls(sqls: &[String]) -> Result<(), ZapError> {
    let joined = sqls.join("; ");
    run_sql(&joined).map(|_| ())
}

// ── 校验与转义 ──────────────────────────────────────────────

/// 校验标识符（库名 / 用户名）：字母、数字、下划线、连字符，长度 ≤ 64。
fn check_ident(name: &str, label: &str) -> Result<String, ZapError> {
    if name.is_empty() || name.len() > 64 {
        return Err(ZapError::New(-1, format!("{label}长度必须在 1-64 之间")));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ZapError::New(
            -1,
            format!("{label}只能包含字母、数字、下划线和连字符"),
        ));
    }
    Ok(name.to_string())
}

/// 转义 SQL 字符串字面量（用于密码等）。
fn escape_literal(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// 主机部分（用于 user@host）：允许 %、IP、localhost 与域名。
fn check_host(host: &str) -> Result<String, ZapError> {
    let h = host.trim();
    if h.is_empty() || h.len() > 255 {
        return Err(ZapError::New(-1, "主机地址不合法".to_string()));
    }
    if !h
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '%' | '_' | ':'))
    {
        return Err(ZapError::New(-1, "主机地址含有非法字符".to_string()));
    }
    Ok(h.to_string())
}

/// 本机来源的等价写法。
///
/// MySQL 的 `user@host` 是**按来源字符串精确匹配**的：`'u'@'localhost'` 只覆盖
/// unix socket 连接，以及开启了反解（127.0.0.1 → localhost）时的回环 TCP 连接；
/// 一旦实例开了 `skip-name-resolve`，应用用 `127.0.0.1:3306` 连就会匹配不上而被拒。
/// 因此建库时统一把「本机来源」展开成这三种写法，socket / IPv4 / IPv6 入口都能连。
const LOCAL_HOST_ALIASES: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// 把请求里的 host 展开成需要授权的来源列表。
///
/// 本机来源（localhost / 127.0.0.1 / ::1 任一）→ 全部等价写法；
/// 其它（远程 IP / 域名 / `%`）→ 原样返回，不做猜测。
fn host_targets(host: &str) -> Vec<String> {
    let h = host.trim();
    if LOCAL_HOST_ALIASES.iter().any(|a| a.eq_ignore_ascii_case(h)) {
        return LOCAL_HOST_ALIASES.iter().map(|a| a.to_string()).collect();
    }
    vec![h.to_string()]
}

/// 非管理员可见的库名前缀；管理员返回 None 表示不限制。
fn schema_prefix(claims: &jwt::Claims) -> Option<String> {
    if is_admin(claims) {
        return None;
    }
    Some(schema_prefix_of(&claims.sub))
}

/// 用户名 → 库名前缀（非字母数字统一替换为 `_`，与建库补前缀规则一致）
pub(crate) fn schema_prefix_of(username: &str) -> String {
    let user: String = username
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{user}_")
}

/// 统计库数量：`prefixes` 为空表示统计全部非系统库（admin）。
///
/// MySQL 不可用 / 未配置时返回 0（不阻断仪表盘渲染）。
pub(crate) fn count_schemas(prefixes: &[String]) -> i64 {
    let Ok(out) = run_sql("SELECT s.SCHEMA_NAME FROM information_schema.SCHEMATA s") else {
        return 0;
    };
    let mut n = 0i64;
    for line in out.lines() {
        let name = line.trim();
        if name.is_empty() || SYSTEM_SCHEMAS.contains(&name) {
            continue;
        }
        if !prefixes.is_empty() && !prefixes.iter().any(|p| name.starts_with(p.as_str())) {
            continue;
        }
        n += 1;
    }
    n
}

/// 校验库归属：非管理员只能操作自己前缀下的库。
fn ensure_owned(claims: &jwt::Claims, schema: &str) -> Result<String, ZapError> {
    let name = check_ident(schema, "数据库名")?;
    if let Some(p) = schema_prefix(claims)
        && !name.starts_with(&p)
    {
        return Err(ZapError::New(
            -1,
            format!("无权操作数据库 `{name}`：只能管理以 `{p}` 开头的库"),
        ));
    }
    Ok(name)
}

// ── 请求体 ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SchemaReq {
    pub name: String,
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// 轻量模式（light=1）：跳过容量 / 表数量统计（大库统计较慢）
    #[serde(default)]
    pub light: Option<u8>,
}

#[derive(Deserialize)]
pub struct CreateDbReq {
    pub name: String,
    #[serde(default = "default_charset")]
    pub charset: String,
    /// 是否同时创建「与库同名」的数据库用户并授予该库全部权限（默认 false = 仅建库）
    #[serde(default)]
    pub create_user: bool,
    /// 自定义数据库用户名（不含 `{用户名}_` 前缀）；留空 = 与库名同名
    #[serde(default)]
    pub user: Option<String>,
    /// 自定义密码；留空 = 自动生成 16 位随机密码
    #[serde(default)]
    pub password: Option<String>,
    /// 数据库用户允许连接的主机（默认 localhost）
    #[serde(default = "default_host")]
    pub host: String,
}

/// 生成 16 位随机密码（大小写字母 + 数字，剔除 0/O/1/l/I 等易混淆字符）。
///
/// 密码只含字母数字，避免在 SQL 字面量、配置文件、终端里产生转义歧义。
fn gen_db_password() -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    (0..16)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
}

fn default_charset() -> String {
    "utf8mb4".to_string()
}

#[derive(Deserialize)]
pub struct UserCreateReq {
    pub user: String,
    pub password: String,
    /// 允许连接的主机：localhost / % / 具体 IP
    #[serde(default = "default_host")]
    pub host: String,
    /// 授权到的库；为空表示不授权库（仅创建用户）
    #[serde(default)]
    pub schema: Option<String>,
}

fn default_host() -> String {
    "localhost".to_string()
}

#[derive(Deserialize)]
pub struct UserDropReq {
    pub user: String,
    #[serde(default = "default_host")]
    pub host: String,
}

#[derive(Deserialize)]
pub struct RemoteGrantReq {
    pub user: String,
    pub schema: String,
    /// 允许远程连接的主机（IP 或 %）
    pub host: String,
    /// 是否同时设置/更新密码（可选）
    #[serde(default)]
    pub password: Option<String>,
}

/// 成功返回：与其它模块保持一致的 `{ code: 0, data: ... }` 包装。
fn ok(data: Value) -> ZapJsonResult {
    Ok(Json(json!({ "code": 0, "data": data })))
}

// ── handlers ────────────────────────────────────────────────

/// GET /api/database/status：服务状态与版本。
pub async fn status(_claims: ValidatedClaims) -> ZapJsonResult {
    let version = run_sql("SELECT VERSION()")?
        .trim()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    // SQL 模式便于排障（严格模式会拦截隐式截断、非法日期等写法）
    let sql_mode = run_sql("SELECT @@global.sql_mode")
        .map(|s| s.trim().lines().next().unwrap_or_default().to_string())
        .unwrap_or_default();
    // 监听端口：管理操作走 socket，但客户端连接需要 `host:port`
    let port: u16 = run_sql("SELECT @@port")
        .ok()
        .and_then(|s| s.trim().lines().next()?.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT);
    ok(json!({
        "ok": true,
        "version": version,
        "user": CRED_USER,
        "host": DB_HOST,
        "port": port,
        "addr": format!("{DB_HOST}:{port}"),
        "socket": socket_path(),
        "sql_mode": sql_mode,
    }))
}

/// 套餐限制：MySQL / MariaDB 数据库数量（0 = 不限）。
///
/// 与站点数限制一致：只对普通用户（有 `{用户名}_` 前缀）生效，管理员不受限；
/// 未绑定套餐 / 套餐停用时不做拦截。
async fn ensure_db_quota(claims: &jwt::Claims) -> Result<(), ZapError> {
    let Some(prefix) = schema_prefix(claims) else {
        return Ok(());
    };
    let Some(pkg) = crate::routers::package::package_of_user(claims.id as i64).await else {
        return Ok(());
    };
    if pkg.max_mysql_dbs <= 0 {
        return Ok(());
    }
    let out = run_sql("SELECT SCHEMA_NAME FROM information_schema.SCHEMATA")?;
    let used = out
        .lines()
        .filter(|l| l.starts_with(prefix.as_str()))
        .count() as i64;
    if used >= pkg.max_mysql_dbs {
        return Err(ZapError::New(
            -1,
            format!(
                "已达套餐「{}」的数据库上限 {} 个（当前 {} 个），无法继续创建",
                pkg.name, pkg.max_mysql_dbs, used
            ),
        ));
    }
    Ok(())
}

/// GET /api/database/list：库列表（含大小、表数、字符集、可访问账号数）。
///
/// `light=1` 跳过容量与表数量统计（库多/表大时统计较慢），只返回基础信息。
pub async fn list(claims: ValidatedClaims, Query(q): Query<ListQuery>) -> ZapJsonResult {
    let light = q.light.unwrap_or(0) != 0;
    let sql = if light {
        "SELECT s.SCHEMA_NAME, \
         COALESCE(s.DEFAULT_CHARACTER_SET_NAME,''), \
         COALESCE(s.DEFAULT_COLLATION_NAME,'') \
         FROM information_schema.SCHEMATA s \
         ORDER BY s.SCHEMA_NAME"
    } else {
        "SELECT s.SCHEMA_NAME, \
         COALESCE(s.DEFAULT_CHARACTER_SET_NAME,''), \
         COALESCE(s.DEFAULT_COLLATION_NAME,''), \
         COALESCE(SUM(t.DATA_LENGTH + t.INDEX_LENGTH), 0), \
         COUNT(t.TABLE_NAME) \
         FROM information_schema.SCHEMATA s \
         LEFT JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = s.SCHEMA_NAME \
         GROUP BY s.SCHEMA_NAME, s.DEFAULT_CHARACTER_SET_NAME, s.DEFAULT_COLLATION_NAME \
         ORDER BY s.SCHEMA_NAME"
    };

    let out = run_sql(sql)?;
    let prefix = schema_prefix(&claims);

    // 每库可访问账号数：mysql.db 记录库级授权（读不到就显示 0，不影响主流程）
    let mut user_counts: HashMap<String, u64> = HashMap::new();
    if !light
        && let Ok(privs) = run_sql(
            "SELECT Db, COUNT(DISTINCT User) FROM mysql.db \
             WHERE Db NOT IN ('mysql','sys','performance_schema','information_schema') \
             GROUP BY Db",
        )
    {
        for line in privs.lines() {
            let mut cols = line.split('\t');
            if let (Some(db), Some(count)) = (cols.next(), cols.next()) {
                user_counts.insert(db.to_string(), count.parse().unwrap_or(0));
            }
        }
    }

    let mut items: Vec<Value> = Vec::new();
    for line in out.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 3 {
            continue;
        }
        let name = cols[0].to_string();
        if SYSTEM_SCHEMAS.contains(&name.as_str()) {
            continue;
        }
        if let Some(p) = &prefix
            && !name.starts_with(p.as_str())
        {
            continue;
        }
        let size: u64 = cols.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let tables: u64 = cols.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
        let users = user_counts.get(&name).copied().unwrap_or(0);
        items.push(json!({
            "name": name,
            "charset": cols[1],
            "collation": cols[2],
            "size": size,
            "tables": tables,
            "users": users,
        }));
    }

    ok(json!({ "ok": true, "list": items, "prefix": prefix, "light": light }))
}

/// POST /api/database/create：创建数据库。
///
/// 非管理员的库名 / 用户名一律自动补 `{用户名}_` 前缀；`create_user=true` 时
/// 一并创建「与库同名」的用户并授予该库全部权限（密码留空则随机生成，
/// 明文密码仅在本次响应中返回一次）。建用户失败会回滚刚建的库，避免半成品。
/// 建库结果（HTTP 接口与 AppStore provision 共用）。
#[derive(Debug, Clone)]
pub(crate) struct CreatedDb {
    pub name: String,
    pub charset: String,
    /// 专用用户名（create_user=false 时为 None）
    pub user: Option<String>,
    /// 允许连接的主机（create_user=false 时为 None）
    pub host: Option<String>,
    /// 明文密码（仅随机生成时在此出现一次，之后不再落盘可读取的位置）
    pub password: Option<String>,
}

/// 建库执行体：套餐配额 → 前缀补全 → 建库 →（可选）建同名专用用户并授权。
///
/// HTTP 接口与 AppStore 建站包的 provision 共用同一条路径，确保配额、
/// 多租户前缀、回滚逻辑只有一份实现。
pub(crate) async fn create_schema(
    claims: &jwt::Claims,
    req: CreateDbReq,
) -> Result<CreatedDb, ZapError> {
    // 套餐配额：数据库数量上限（0 = 不限）
    ensure_db_quota(claims).await?;
    let raw = req.name.trim();
    let prefix = schema_prefix(claims);
    let name = match &prefix {
        Some(p) if !raw.starts_with(p.as_str()) => check_ident(&format!("{p}{raw}"), "数据库名")?,
        _ => ensure_owned(claims, raw)?,
    };
    let charset = if req.charset.trim().is_empty() {
        "utf8mb4".to_string()
    } else {
        check_ident(req.charset.trim(), "字符集")?
    };

    run_sqls(&[format!("CREATE DATABASE `{name}` CHARACTER SET {charset}")])?;

    if !req.create_user {
        return Ok(CreatedDb {
            name,
            charset,
            user: None,
            host: None,
            password: None,
        });
    }

    // 库 + 用户一条龙：用户名默认与库名同名（前缀一致），也允许高级模式自定义
    let base = req.user.as_deref().map(str::trim).unwrap_or("");
    let raw_user = if base.is_empty() {
        name.clone()
    } else {
        match &prefix {
            Some(p) if !base.starts_with(p.as_str()) => format!("{p}{base}"),
            _ => base.to_string(),
        }
    };
    let user = check_ident(&raw_user, "用户名")?;
    let host = check_host(&req.host)?;
    // 本机来源展开成 localhost / 127.0.0.1 / ::1：应用用什么写法连都能进
    let hosts = host_targets(&host);
    let password = match req.password.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() => {
            if p.len() < 8 {
                let _ = run_sqls(&[format!("DROP DATABASE `{name}`")]);
                return Err(ZapError::New(-1, "密码长度不能少于 8 位".to_string()));
            }
            p.to_string()
        }
        _ => gen_db_password(),
    };

    // 逐个来源建号并授权：任何一个失败，只回滚**本次新建**的账号，
    // 不动同名的既有账号（那可能是用户自己建的，删掉会误伤）。
    let mut created: Vec<String> = Vec::new();
    for h in &hosts {
        let sqls = [
            format!(
                "CREATE USER '{user}'@'{h}' IDENTIFIED BY '{}'",
                escape_literal(&password)
            ),
            format!("GRANT ALL PRIVILEGES ON `{name}`.* TO '{user}'@'{h}'"),
        ];
        if let Err(e) = run_sqls(&sqls) {
            let mut undo: Vec<String> = created
                .iter()
                .map(|c| format!("DROP USER '{user}'@'{c}'"))
                .collect();
            undo.push(format!("DROP DATABASE `{name}`"));
            undo.push("FLUSH PRIVILEGES".to_string());
            let _ = run_sqls(&undo);
            return Err(e);
        }
        created.push(h.clone());
    }
    let _ = run_sqls(&["FLUSH PRIVILEGES".to_string()]);

    Ok(CreatedDb {
        name,
        charset,
        user: Some(user),
        host: Some(host),
        password: Some(password),
    })
}

/// POST /api/database/create：创建数据库。
///
/// 非管理员的库名 / 用户名一律自动补 `{用户名}_` 前缀；`create_user=true` 时
/// 一并创建「与库同名」的用户并授予该库全部权限（密码留空则随机生成，
/// 明文密码仅在本次响应中返回一次）。建用户失败会回滚刚建的库，避免半成品。
pub async fn create(claims: ValidatedClaims, Json(req): Json<CreateDbReq>) -> ZapJsonResult {
    let d = create_schema(&claims, req).await?;
    let mut data = json!({ "ok": true, "name": d.name, "charset": d.charset });
    if let (Some(u), Some(h), Some(p)) = (&d.user, &d.host, &d.password) {
        data["user"] = json!(u);
        data["host"] = json!(h);
        // 明文密码仅在创建响应中返回一次，请提示用户立即保存
        data["password"] = json!(p);
    }
    ok(data)
}

// ── AppStore provision：为建站包分配数据库 ──────────────────

/// 库是否已存在（用于 provision 自动避让重名）。
fn schema_exists(name: &str) -> bool {
    let sql = format!(
        "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = '{}'",
        escape_literal(name)
    );
    run_sql(&sql)
        .map(|s| s.trim().lines().next().unwrap_or_default().trim() == name)
        .unwrap_or(false)
}

/// 数据库服务端口（脚本连接用；取不到时用 3306 兜底）。
pub(crate) fn db_port() -> u16 {
    run_sql("SELECT @@port")
        .ok()
        .and_then(|s| {
            s.trim()
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .parse()
                .ok()
        })
        .unwrap_or(DEFAULT_PORT)
}

/// AppStore provision：为建站类包建一个「专用库 + 专用用户」。
///
/// - 库名 = 用户前缀 + `base`（`base` 缺省 `app`），重名自动加序号
///   （同一包装多个实例不会互相覆盖）；
/// - 密码随机 16 位，**只出现在返回值与本次脚本 env 中**（不落 options.env，
///   也不发给前端），脚本负责写进自己的配置文件；
/// - 配额 / 前缀 / 回滚规则与普通建库完全一致。
pub(crate) async fn provision_db(
    claims: &jwt::Claims,
    base: Option<&str>,
    charset: Option<&str>,
    host: Option<&str>,
) -> Result<CreatedDb, ZapError> {
    let base = match base.map(str::trim).filter(|s| !s.is_empty()) {
        Some(b) => b.to_string(),
        None => "app".to_string(),
    };
    let prefix = schema_prefix(claims);
    let charset = charset
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("utf8mb4")
        .to_string();
    let host = host
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("localhost")
        .to_string();

    for i in 0..32 {
        let raw = if i == 0 {
            base.clone()
        } else {
            format!("{base}_{}", i + 1)
        };
        // 前缀由 create_schema 内部补全；这里按同样规则预判重名
        let full = match &prefix {
            Some(p) if !raw.starts_with(p.as_str()) => format!("{p}{raw}"),
            _ => raw.clone(),
        };
        if schema_exists(&full) {
            continue;
        }
        let req = CreateDbReq {
            name: raw,
            charset: charset.clone(),
            create_user: true,
            user: None,
            password: None,
            host: host.clone(),
        };
        return create_schema(claims, req).await;
    }
    Err(ZapError::New(
        -1,
        format!("数据库名 `{base}` 已被占用，无法自动分配库名"),
    ))
}

/// POST /api/database/drop：删除数据库。
pub async fn drop_db(claims: ValidatedClaims, Json(req): Json<SchemaReq>) -> ZapJsonResult {
    let name = ensure_owned(&claims, req.name.trim())?;
    run_sqls(&[format!("DROP DATABASE `{name}`")])?;
    ok(json!({ "ok": true, "name": name }))
}

/// GET /api/database/users：数据库用户列表（含授权）。
pub async fn users(claims: ValidatedClaims) -> ZapJsonResult {
    let out = run_sql("SELECT user, host FROM mysql.user ORDER BY user, host")?;
    let skip = [
        "root",
        "mysql.session",
        "mysql.sys",
        "mysql.infoschema",
        "debian-sys-maint",
        "zapadm",
    ];
    let prefix = schema_prefix(&claims);

    let mut items: Vec<Value> = Vec::new();
    for line in out.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 2 {
            continue;
        }
        let user = cols[0].to_string();
        let host = cols[1].to_string();
        if skip.contains(&user.as_str()) {
            continue;
        }
        if let Some(p) = &prefix
            && !user.starts_with(p.as_str())
        {
            continue;
        }
        let grants = run_sql(&format!("SHOW GRANTS FOR '{user}'@'{host}'")).unwrap_or_default();
        items.push(json!({ "user": user, "host": host, "grants": grants }));
    }

    ok(json!({ "ok": true, "list": items }))
}

/// POST /api/database/user/create：创建用户（可选授权到某个库）。
pub async fn user_create(claims: ValidatedClaims, Json(req): Json<UserCreateReq>) -> ZapJsonResult {
    let prefix = schema_prefix(&claims);
    let user = match &prefix {
        Some(p) if !req.user.starts_with(p.as_str()) => {
            check_ident(&format!("{p}{}", req.user.trim()), "用户名")?
        }
        _ => check_ident(req.user.trim(), "用户名")?,
    };
    let host = check_host(&req.host)?;
    if req.password.len() < 8 {
        return Err(ZapError::New(-1, "密码长度不能少于 8 位".to_string()));
    }
    let pwd = escape_literal(&req.password);

    let mut sqls = vec![format!(
        "CREATE USER '{user}'@'{host}' IDENTIFIED BY '{pwd}'"
    )];
    if let Some(schema) = req.schema.as_deref() {
        let db = ensure_owned(&claims, schema.trim())?;
        sqls.push(format!(
            "GRANT ALL PRIVILEGES ON `{db}`.* TO '{user}'@'{host}'"
        ));
    }
    sqls.push("FLUSH PRIVILEGES".to_string());
    run_sqls(&sqls)?;

    ok(json!({ "ok": true, "user": user, "host": host }))
}

/// POST /api/database/user/drop：删除用户。
pub async fn user_drop(claims: ValidatedClaims, Json(req): Json<UserDropReq>) -> ZapJsonResult {
    let user = check_ident(req.user.trim(), "用户名")?;
    let host = check_host(&req.host)?;
    if let Some(p) = schema_prefix(&claims)
        && !user.starts_with(&p)
    {
        return Err(ZapError::New(-1, format!("无权删除用户 `{user}`")));
    }
    run_sqls(&[
        format!("DROP USER '{user}'@'{host}'"),
        "FLUSH PRIVILEGES".to_string(),
    ])?;
    ok(json!({ "ok": true, "user": user }))
}

/// GET /api/database/remote：远程访问授权列表（host 不是本机来源的账号）。
pub async fn remote_list(claims: ValidatedClaims) -> ZapJsonResult {
    let out = run_sql("SELECT user, host FROM mysql.user ORDER BY user, host")?;
    let prefix = schema_prefix(&claims);

    let mut items: Vec<Value> = Vec::new();
    for line in out.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 2 {
            continue;
        }
        let user = cols[0].to_string();
        let host = cols[1].to_string();
        // 远程授权：host 不是 localhost / 127.0.0.1 / ::1
        if matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1") {
            continue;
        }
        if let Some(p) = &prefix
            && !user.starts_with(p.as_str())
        {
            continue;
        }
        let grants = run_sql(&format!("SHOW GRANTS FOR '{user}'@'{host}'")).unwrap_or_default();
        items.push(json!({ "user": user, "host": host, "grants": grants }));
    }

    ok(json!({ "ok": true, "list": items }))
}

/// POST /api/database/remote/grant：授权某主机远程访问某库。
pub async fn remote_grant(
    claims: ValidatedClaims,
    Json(req): Json<RemoteGrantReq>,
) -> ZapJsonResult {
    let user = check_ident(req.user.trim(), "用户名")?;
    let db = ensure_owned(&claims, req.schema.trim())?;
    let host = check_host(&req.host)?;
    if let Some(p) = schema_prefix(&claims)
        && !user.starts_with(&p)
    {
        return Err(ZapError::New(-1, format!("无权配置用户 `{user}`")));
    }

    let ident = format!("'{user}'@'{host}'");
    let exists = run_sql(&format!(
        "SELECT 1 FROM mysql.user WHERE user = '{user}' AND host = '{host}'"
    ))
    .map(|s| !s.trim().is_empty())
    .unwrap_or(false);

    let mut sqls = Vec::new();
    if !exists {
        match req.password.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => {
                if p.len() < 8 {
                    return Err(ZapError::New(-1, "密码长度不能少于 8 位".to_string()));
                }
                sqls.push(format!(
                    "CREATE USER {ident} IDENTIFIED BY '{}'",
                    escape_literal(p)
                ));
            }
            _ => {
                return Err(ZapError::New(
                    -1,
                    "该用户在该主机下不存在，设置密码后才能创建".to_string(),
                ));
            }
        }
    }
    sqls.push(format!("GRANT ALL PRIVILEGES ON `{db}`.* TO {ident}"));
    sqls.push("FLUSH PRIVILEGES".to_string());
    run_sqls(&sqls)?;

    ok(json!({ "ok": true, "user": user, "host": host, "schema": db }))
}

/// POST /api/database/remote/revoke：撤销远程授权（删除该 host 下的账号）。
pub async fn remote_revoke(claims: ValidatedClaims, Json(req): Json<UserDropReq>) -> ZapJsonResult {
    let user = check_ident(req.user.trim(), "用户名")?;
    let host = check_host(&req.host)?;
    if matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return Err(ZapError::New(
            -1,
            "只能撤销远程主机（非 localhost）的授权".to_string(),
        ));
    }
    if let Some(p) = schema_prefix(&claims)
        && !user.starts_with(&p)
    {
        return Err(ZapError::New(-1, format!("无权撤销用户 `{user}`")));
    }
    run_sqls(&[
        format!("DROP USER '{user}'@'{host}'"),
        "FLUSH PRIVILEGES".to_string(),
    ])?;
    ok(json!({ "ok": true, "user": user, "host": host }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_host_expands_to_all_loopback_forms() {
        // 本机来源 → 三种写法都授权：应用用 localhost / 127.0.0.1 / ::1 连都能进
        let all = vec!["localhost", "127.0.0.1", "::1"];
        assert_eq!(host_targets("localhost"), all);
        assert_eq!(host_targets("127.0.0.1"), all);
        assert_eq!(host_targets("::1"), all);
        // 大小写、两侧空白都归一
        assert_eq!(host_targets(" LocalHost "), all);
    }

    #[test]
    fn remote_host_is_kept_as_is() {
        assert_eq!(host_targets("10.0.0.5"), vec!["10.0.0.5"]);
        assert_eq!(host_targets("db.internal"), vec!["db.internal"]);
        // `%` 已覆盖所有来源，不再展开
        assert_eq!(host_targets("%"), vec!["%"]);
    }
}
