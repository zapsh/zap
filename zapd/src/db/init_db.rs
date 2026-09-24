use std::collections::HashMap;

use sqlx::Executor;
use tracing::info;

use super::get_db_pool;
use super::menu_seed;

/// 建表与种子数据入口。
///
/// **约定（开发阶段）**：表结构与初始数据都以 `CREATE TABLE` + 种子清单为准，新建库
/// 一次建齐（列、菜单、授权都在里面），所以加列 / 加菜单后**不必** `--reset-db`；
/// 对**已存在**的库，`migrate_add_columns()` 补列、`sync_added_menus()` 补菜单
/// （按种子清单补齐库里缺失的项，含 Zap Pro 菜单：老库换 Pro 二进制后靠它补入口）。
/// 删列、改类型、改约束、改菜单种子（已存在的库不会被覆盖）仍需重建数据库。
pub async fn init_schema() {
    init_system_user_table_schema().await;
    init_system_monitor_table_schema().await;
    init_system_monitor_networks_table_schema().await;
    init_monitor_indexes().await;
    init_roles_table().await;
    // 菜单：建表 + 结构化种子（id 自增），随后按种子里的 roles 生成 role_menus
    let menu_ids = init_menus_table().await;
    init_role_menus_table(&menu_ids).await;
    // 用户级菜单例外：给单个用户开小灶 / 收窄入口（仅渲染层）
    init_user_menus_table().await;
    // 老库补入口：开发阶段为空，将来新增菜单时在这里补（见函数注释）
    sync_added_menus().await;
    // 动作级权限点（请求级鉴权依据；role_menus 仅用于菜单渲染）
    init_role_permissions_table().await;
    init_audit_table().await;
    init_login_attempts_table().await;
    // 登录记录（个人中心「最近登录」用）
    init_login_history_table().await;
    init_hourly_stats_tables().await;
    crate::routers::ssh_terminal::init_table().await;
    // 通用任务队列（应用商店安装 / Docker 构建 / 备份 / 升级 / 计划任务都登记在这里）
    init_task_queue_table().await;
    // 老数据订正：脚本运行以前登记成 appstore（见函数注释）
    sync_script_task_kind().await;
    // IP 池管理表
    init_ip_pool_table().await;
    // 用户站点管理表
    init_site_table().await;
    // 站点扩展档案（类型/伪静态/upstream/location/自定义目录 + TLS 高级设置）
    init_site_profile_table().await;
    // PHP-FPM 规格模板表（user.fpm_spec_ref 已在 user 表中定义）
    init_fpm_spec_table().await;
    // 套餐（Packages）表：创建客户时可选择的资源套餐
    init_packages_table().await;
    // 站内信（通知中心）表
    init_notice_message_table().await;
    // API Token 管理表
    init_api_token_table().await;
    // SSL/TLS 证书管理表
    init_ssl_cert_table().await;
    // SSL/TLS：ACME 账户 / 订单 / DNS 服务商凭据（Let's Encrypt 申请）
    init_ssl_acme_account_table().await;
    init_ssl_acme_order_table().await;
    init_ssl_acme_dns_provider_table().await;
    // 老库补列：新增列自动 ALTER 到已有表，避免每次加列都必须重建数据库
    migrate_add_columns().await;
    // 依赖上面的补列结果，必须排在其后
    sync_menu_features().await;
}

/// 幂等补列：列已存在则跳过，否则 `ALTER TABLE ... ADD COLUMN`。
///
/// 开发阶段没有存量库要补，暂无人调用（见 [`migrate_add_columns`]），
/// 保留待将来系统升级时使用。
#[allow(dead_code)]
async fn ensure_column(table: &str, column: &str, decl: &str) {
    if !table_exists(table).await {
        return;
    }
    let pool = get_db_pool().await;
    let exists: bool =
        sqlx::query_scalar("SELECT COUNT(*) > 0 FROM pragma_table_info(?) WHERE name = ?")
            .bind(table)
            .bind(column)
            .fetch_one(pool)
            .await
            .unwrap_or(false);
    if exists {
        return;
    }
    let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {decl}");
    if let Err(e) = sqlx::query(&sql).execute(pool).await {
        eprintln!("补列失败 {table}.{column}: {e}");
    }
}

/// 历史库新增列清单：**开发阶段为空**——所有列都已直接写进各自的 CREATE TABLE
/// （见 `init_system_user_table_schema` / `init_site_table` / `init_menus_table` /
/// `init_task_queue_table`），新库建表即完整，不必靠 ALTER 补。
///
/// 等有存量库要升级时再用：在 CREATE TABLE 里加列的同时，把同一行登记到这里，
/// 老库启动时由 [`ensure_column`] 幂等补上。示例（取消注释即可用）：
///
/// ```ignore
/// ensure_column("user", "new_col", "TEXT NOT NULL DEFAULT ''").await;
/// ```
async fn migrate_add_columns() {
    // 会话版本号：老库补列后，存量用户一律从 0 起算（不影响已有 token）
    ensure_column("user", "token_version", "INTEGER NOT NULL DEFAULT 0").await;
    // 静态 API Token 的会话版本号：同样从 0 起算，与用户当前版本号对齐
    ensure_column("api_token", "token_version", "INTEGER NOT NULL DEFAULT 0").await;
}

/// 菜单能力门禁赋值（**老库升级**用）。
///
/// **开发阶段为空**：新库的 `feature` 直接写在种子里（如 docker / docker-index，
/// 见 [`menu_seed::MENU_SEEDS`]）。存量库需要时在这里按 name 补，同样不要写 id：
///
/// ```ignore
/// sqlx::query("UPDATE menus SET feature = 'docker', updated_at = strftime('%s','now') \
///              WHERE name IN ('docker', 'docker-index') AND feature <> 'docker'")
///     .execute(pool).await;
/// ```
async fn sync_menu_features() {}

// ── user ───────────────────────────────────────────────────

async fn init_system_user_table_schema() {
    if table_exists("user").await {
        return;
    }
    // Create table
    let create_sql = r#"
    CREATE TABLE user (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        username VARCHAR(128) UNIQUE NOT NULL,
        password VARCHAR(256) NOT NULL,
        email VARCHAR(256) UNIQUE NOT NULL,
        phone VARCHAR(32) UNIQUE,
        nickname TEXT,
        home_dir TEXT NOT NULL DEFAULT '',
        linux_user TEXT NOT NULL DEFAULT '',
        fpm_pool TEXT NOT NULL DEFAULT '',
        -- fpm_spec_ref：''=面板全局默认 / 'inherit'=继承 owner(reseller) 名下默认 / 模板名
        fpm_spec_ref TEXT NOT NULL DEFAULT '',
        -- prefs：个人中心 → 偏好设置（JSON 字符串）
        prefs TEXT NOT NULL DEFAULT '',
        last_login_time INTEGER,
        last_login_ip TEXT,
        status INTEGER DEFAULT 1,
        roles TEXT,
        permissions TEXT,
        owner_id INTEGER DEFAULT 0,
        -- user_kind：0=普通用户/客户（独立家目录与 Linux 账号）/ 1=成员（子账号）
        --   成员共享父账号（owner_id）的家目录与 Linux 系统账号，不另建系统账号，
        --   权限默认继承父账号的生效权限，由父账号通过 perm_deny 再收紧
        user_kind INTEGER NOT NULL DEFAULT 0,
        -- perm_deny：父账号对该成员取消（收紧）的权限点，逗号分隔；仅 user_kind=1 生效
        perm_deny TEXT NOT NULL DEFAULT '',
        -- read_only：只读账号（1=是）。生效权限一律只保留查看类（{ns}:view），
        --   用于「共享可见但不能改」的成员 / 客户（新建用户时勾选即可）
        read_only INTEGER NOT NULL DEFAULT 0,
        package_id INTEGER NOT NULL DEFAULT 0,
        totp_secret TEXT NOT NULL DEFAULT '',
        totp_enabled INTEGER NOT NULL DEFAULT 0,
        -- token_version：会话版本号。「下线所有设备」时 +1，JWT Claims 里带 tv，
        --   小于库中当前值的 token 一律判为已下线（见 zap::session）。
        token_version INTEGER NOT NULL DEFAULT 0,
        -- 磁盘用量（字节）与采集时间：定时任务 du 家目录写入（0 = 尚未采集）
        disk_used_bytes INTEGER NOT NULL DEFAULT 0,
        disk_stat_at INTEGER NOT NULL DEFAULT 0,
        -- 本月出站流量（字节）：解析名下站点 nginx access.log 汇总（0 = 尚未采集）
        bandwidth_used_bytes INTEGER NOT NULL DEFAULT 0,
        -- 流量统计周期（YYYYMM）：跨月自动重置本月计数
        bandwidth_period TEXT NOT NULL DEFAULT '',
        bandwidth_stat_at INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER,
        updated_at INTEGER
    )
    "#;
    get_db_pool().await.execute(create_sql).await.unwrap();

    // 注意：初始管理员不由这里插入，交给调用方在建表后调 ensure_initial_admin()
    // —— 新建表的分支只在库还不存在时才会走到，而「已有库但管理员被删光」同样
    // 需要补一条，两边统一由 ensure_initial_admin 的幂等判断覆盖。
}

/// 面板用户名合法性：Linux 账号名与家目录末段都派生自它，因此必须同时满足
/// `useradd` 的约束——小写字母或 `_` 开头，只含 `[a-z0-9_-]`，长度 ≤ 32。
pub fn valid_admin_username(u: &str) -> bool {
    !u.is_empty()
        && u.len() <= 32
        && u.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && u.chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
}

/// 初始管理员凭据：默认 `admin` / `123456`。
///
/// install.sh 会在 zapd 首次启动前执行 `zapd --init-admin <用户> --admin-password <密码>`
/// 覆盖它——凭据只经由命令行传递，不落任何文件。
static INITIAL_ADMIN: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();

/// 设定初始管理员凭据（`--init-admin` 用）。用户名同时派生 Linux 账号与家目录
/// `/home/{linux_user}`，规则与面板新建用户一致（`zap_proto::linux_username`）。
pub fn set_initial_admin(username: &str, password: &str) {
    let _ = INITIAL_ADMIN.set((username.to_string(), password.to_string()));
}

/// 库里还没有任何 admin 时插入初始管理员，返回是否真的插入了。
///
/// 幂等：已有 admin 一律不动（既不插入第二条，也不改现有账号的密码——重跑安装
/// 脚本不会把线上密码改回去）。**建表后调用**（`init_schema` 之后）。
pub async fn ensure_initial_admin() -> bool {
    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user WHERE roles LIKE '%admin%'")
        .fetch_one(get_db_pool().await)
        .await
        .unwrap_or(0);
    if existing > 0 {
        return false;
    }

    let (username, password) = INITIAL_ADMIN
        .get()
        .cloned()
        .unwrap_or_else(|| ("admin".to_string(), "123456".to_string()));
    let linux_user = zap_proto::linux_username(&username);
    let home_dir = format!("/home/{linux_user}");
    // bcrypt 在运行时生成（避免 $2y$ 前缀兼容问题）
    let hashed =
        bcrypt::hash(&password, bcrypt::DEFAULT_COST).expect("failed to hash admin password");
    let now = chrono::Utc::now().timestamp();

    sqlx::query(
        "INSERT INTO user (username, home_dir, linux_user, password, email, nickname, phone, last_login_time, last_login_ip, status, roles, permissions, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 'admin', '', ?, ?)",
    )
    .bind(&username)
    .bind(&home_dir)
    .bind(&linux_user)
    .bind(&hashed)
    .bind(format!("{username}@demo.zap.cn"))
    .bind(&username)
    .bind("")
    .bind(now)
    .bind("127.0.0.1")
    .bind(now)
    .bind(now)
    .execute(get_db_pool().await)
    .await
    .unwrap();
    info!("初始管理员已创建：{username}（Linux 账号 {linux_user}，家目录 {home_dir}）");
    true
}

// ── packages（套餐）────────────────────────────────────────

/// 套餐（Packages）：创建客户时选用的资源套餐（对齐 cPanel/WHM 的 Packages）。
/// - owner_id = 0：全局套餐（admin 维护，所有人可用）
/// - owner_id != 0：reseller 自建套餐，仅创建者自己可用
/// - 限制项：磁盘配额 / 最大站点数 / 单站点域名数 / 月流量（仅记录）/ MySQL 与 MariaDB 库数 /
///   PostgreSQL 库数 / FTP 用户数 / FPM 规格模板 / SSH 终端开关
/// - 能力项：allow_proxy（普通用户可用反向代理）；「自定义目录」不再作为套餐能力，
///   已对全部用户开放（home 目录内任意目录可选）
/// - 数值 0 表示「不限」
async fn init_packages_table() {
    if table_exists("packages").await {
        return;
    }
    let sql = r#"
    CREATE TABLE packages (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        name VARCHAR(64) UNIQUE NOT NULL,
        remark TEXT NOT NULL DEFAULT '',
        disk_quota_mb INTEGER NOT NULL DEFAULT 0,
        max_sites INTEGER NOT NULL DEFAULT 0,
        max_domains INTEGER NOT NULL DEFAULT 0,
        max_bandwidth_mb INTEGER NOT NULL DEFAULT 0,
        -- 用户可创建的 MySQL / MariaDB 数据库数量（0 = 不限，建库时硬拦截）
        max_mysql_dbs INTEGER NOT NULL DEFAULT 0,
        -- 用户可创建的 PostgreSQL 数据库数量（0 = 不限，仅记录与展示）
        max_pgsql_dbs INTEGER NOT NULL DEFAULT 0,
        -- 用户可创建的 FTP 账号数量（0 = 不限，仅记录与展示）
        max_ftp_users INTEGER NOT NULL DEFAULT 0,
        fpm_spec_ref TEXT NOT NULL DEFAULT '',
        allow_ssh INTEGER NOT NULL DEFAULT 0,
        allow_proxy INTEGER NOT NULL DEFAULT 0,
        owner_id INTEGER NOT NULL DEFAULT 0,
        status INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER,
        updated_at INTEGER
    );
    INSERT INTO packages (name, remark, disk_quota_mb, max_sites, max_domains, max_bandwidth_mb, max_mysql_dbs, max_pgsql_dbs, max_ftp_users, fpm_spec_ref, allow_ssh, allow_proxy, owner_id, status, created_at, updated_at)
    VALUES ('默认套餐', '不限磁盘、不限站点、不限域名、不限数据库与 FTP 账号数，允许 SSH 终端（反向代理默认关闭，可在「编辑套餐」中开启；自定义目录已全量开放）', 0, 0, 0, 0, 0, 0, 0, '', 1, 0, 0, 1, strftime('%s','now'), strftime('%s','now'));
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── monitor ────────────────────────────────────────────────

async fn init_system_monitor_table_schema() {
    if table_exists("system_stats").await {
        return;
    }
    let sql_script = r#"
    CREATE TABLE system_stats (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        loadavg_one REAL,
        loadavg_five REAL,
        loadavg_fifteen REAL,
        cpu_usage REAL,
        memory_usage REAL,
        swap_usage REAL,
        created_at BIGINT
    )
    "#;
    let _ = get_db_pool().await.execute(sql_script).await;
}

/// 监控历史查询按 created_at 范围过滤 + 分桶聚合：这两张表没有索引的话，
/// 一次 30 天范围查询就是 ~26 万行全表扫描（原始数据 10s 一条），
/// 表现就是「切换时间范围没反应，隔一会数据才跳出来」。
async fn init_monitor_indexes() {
    let pool = get_db_pool().await;
    let _ = pool
        .execute("CREATE INDEX IF NOT EXISTS idx_system_stats_created ON system_stats(created_at)")
        .await;
    let _ = pool
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_networks_stats_created ON networks_stats(created_at)",
        )
        .await;
}

async fn init_system_monitor_networks_table_schema() {
    if table_exists("networks_stats").await {
        return;
    }
    let sql_script = r#"
    CREATE TABLE networks_stats (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        name TEXT,
        received BIGINT,
        transmitted BIGINT,
        errors_on_received BIGINT,
        errors_on_transmitted BIGINT,
        packets_received BIGINT,
        packets_transmitted BIGINT,
        total_received BIGINT,
        total_transmitted BIGINT,
        total_packets_received BIGINT,
        total_packets_transmitted BIGINT,
        total_errors_on_received BIGINT,
        total_errors_on_transmitted BIGINT,
        ipaddrs TEXT,
        created_at BIGINT
    )
    "#;
    let _ = get_db_pool().await.execute(sql_script).await;
}

// ── roles ──────────────────────────────────────────────────

async fn init_roles_table() {
    if table_exists("roles").await {
        return;
    }
    let sql = r#"
    CREATE TABLE roles (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        name VARCHAR(64) UNIQUE NOT NULL,
        role_key VARCHAR(64) UNIQUE NOT NULL,
        description TEXT DEFAULT '',
        status INTEGER DEFAULT 1,
        created_at INTEGER,
        updated_at INTEGER
    );
    INSERT INTO roles (name, role_key, description, status, created_at, updated_at)
    VALUES ('管理员', 'admin', '系统最高权限角色', 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO roles (name, role_key, description, status, created_at, updated_at)
    VALUES ('普通用户', 'user', '普通用户角色', 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO roles (name, role_key, description, status, created_at, updated_at)
    VALUES ('经销商', 'reseller', '经销商角色', 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO roles (name, role_key, description, status, created_at, updated_at)
    VALUES ('演示', 'demo', '演示角色', 1, strftime('%s','now'), strftime('%s','now'));
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── menus ──────────────────────────────────────────────────

/// 幂等收敛存量库的菜单（**已存在的库**用；新增 / 停用都在这里）。
///
/// 初始菜单已全部写在 [`menu_seed::MENU_SEEDS`] 里，且写的是最终形态（父级 /
/// 隐藏 / 停用 / feature 都在种子里），新库建表即完整，所以这里只处理存量库。
/// 注意**按 name 定位、不要写 id**：新库 id 是自增的，写死的数字会打到完全无关的
/// 菜单上。模板：
///
/// ```ignore
/// // 补菜单（父用 name 查，避免写 id）
/// sqlx::query("INSERT OR IGNORE INTO menus (parent_id, name, path, component, type, title, icon, roles, sort_order, status, created_at, updated_at)
///              SELECT (SELECT id FROM menus WHERE name='system'), 'xxx', 'xxx', 'xxx/index', 'menu', '新页面', 'material-symbols:star', 'admin', 20, 1,
///                     strftime('%s','now'), strftime('%s','now')
///              WHERE NOT EXISTS (SELECT 1 FROM menus WHERE name='xxx')")
///     .execute(pool).await;
/// // 补授权：沿用同级菜单（如 ssl-certs）的授权集合
/// sqlx::query("INSERT OR IGNORE INTO role_menus (role_id, menu_id)
///              SELECT rm.role_id, m.id FROM role_menus rm JOIN menus m ON m.name='xxx'
///              WHERE rm.menu_id = (SELECT id FROM menus WHERE name='ssl-certs')")
///     .execute(pool).await;
/// ```
async fn sync_added_menus() {
    // Zap Pro（商业模块）的菜单同样在**建库**时播入（见 `menu_seed::pro_seeds()`）。
    // 但社区版机器用 `install.sh --pro` 重装后，zapd 换成了 Pro 二进制、库还是老库，
    // 建库播种不会再跑一次 —— Pro 菜单就得在下面按种子补齐，否则面板一个 Pro 入口都没有
    // （现象很像「没换成 Pro 版」，其实二进制已经是 Pro 了）。
    sync_seed_menus().await;
}

/// 把种子清单里**库里还没有**的菜单补进存量库（按 name 去重，父子按 name 关联 id）。
///
/// 只补缺失项：已存在的菜单原样保留（管理员可能手工改过标题 / 排序 / 停用状态，
/// 不能被种子覆盖回去）；新增项按种子的 `roles` 建 `role_menus` 授权，
/// 否则菜单补进去了却没人看得到。全程幂等，每次启动都能安全跑。
async fn sync_seed_menus() {
    let pool = get_db_pool().await;
    let mut ids: HashMap<String, i64> =
        sqlx::query_as::<_, (String, i64)>("SELECT name, id FROM menus")
            .fetch_all(pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
    let mut added: HashMap<String, i64> = HashMap::new();

    for seed in menu_seed::all_seeds() {
        if ids.contains_key(seed.name) {
            continue;
        }
        // 父菜单可能是本次刚补进来的（例如 Pro 的父目录），所以用累积的 ids 解析
        let parent_id = seed.parent.and_then(|p| ids.get(p).copied()).unwrap_or(0);
        if parent_id == 0 && seed.parent.is_some() {
            eprintln!(
                "补种菜单 {}: 父菜单 {:?} 不在库里，按顶层处理",
                seed.name, seed.parent
            );
        }
        let res = sqlx::query(
            "INSERT INTO menus
                (parent_id, name, path, component, redirect, type, title, icon,
                 hidden, affix, feature, roles, sort_order, status, created_at, updated_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,strftime('%s','now'),strftime('%s','now'))",
        )
        .bind(parent_id)
        .bind(seed.name)
        .bind(seed.path)
        .bind(seed.component)
        .bind(seed.redirect)
        .bind(seed.kind)
        .bind(seed.title)
        .bind(seed.icon)
        .bind(seed.hidden as i32)
        .bind(seed.affix as i32)
        .bind(seed.feature)
        .bind(seed.roles)
        .bind(seed.sort_order)
        .bind(seed.status)
        .execute(pool)
        .await;
        match res {
            Ok(r) => {
                let id = r.last_insert_rowid();
                ids.insert(seed.name.to_string(), id);
                added.insert(seed.name.to_string(), id);
            }
            Err(e) => eprintln!("补种菜单 {} 失败: {e}", seed.name),
        }
    }

    // 只给本次新增的菜单建授权：传空表会让 seed_role_menus 回落到全库重刷，
    // 等于把管理员手工撤销的授权又加回来。
    if !added.is_empty() {
        info!("补种菜单 {} 条，同步授权", added.len());
        menu_seed::seed_role_menus(pool, &added).await;
    }
}

/// 建表 + 播种菜单（仅新建库）。
///
/// 种子是 [`menu_seed::MENU_SEEDS`] 那张结构化清单：id 由 SQLite 自增、父子用
/// name 关联、隐藏 / 停用 / feature 都写在种子里（最终形态，不用再靠 UPDATE 收敛）。
/// 返回 `name → id`，交给 [`init_role_menus_table`] 生成授权。
async fn init_menus_table() -> HashMap<String, i64> {
    if table_exists("menus").await {
        return HashMap::new();
    }
    let sql = r#"
    CREATE TABLE menus (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        parent_id INTEGER DEFAULT 0,
        -- name 同时是前端路由名与种子的逻辑键：父子关联靠它，故唯一
        name VARCHAR(64) NOT NULL UNIQUE,
        path VARCHAR(128) NOT NULL DEFAULT '',
        component VARCHAR(256) DEFAULT '',
        redirect VARCHAR(128) DEFAULT '',
        type VARCHAR(16) NOT NULL DEFAULT 'menu',
        title VARCHAR(64) NOT NULL DEFAULT '',
        icon VARCHAR(64) DEFAULT '',
        hidden INTEGER DEFAULT 0,
        keep_alive INTEGER DEFAULT 0,
        affix INTEGER DEFAULT 0,
        -- 环境能力门禁：空串=常显；'docker' 等表示仅在该能力可用时才下发
        -- （见 zap::feature）。控制的是「能不能用」，与人工开关 hidden 无关。
        feature TEXT NOT NULL DEFAULT '',
        roles TEXT DEFAULT '',
        sort_order INTEGER DEFAULT 0,
        status INTEGER DEFAULT 1,
        created_at INTEGER,
        updated_at INTEGER
    );
    "#;
    let pool = get_db_pool().await;
    let _ = pool.execute(sql).await;
    menu_seed::seed_menus(pool).await
}

// ── role_menus ─────────────────────────────────────────────

/// 角色 → 菜单授权（仅新建库）。
///
/// 授权集合来自菜单种子的 `roles` 字段（见 [`menu_seed::seed_role_menus`]）：
/// 角色按 role_key、菜单按 name 解析 id，不再手写数字对，也就不会出现
/// 「菜单加了、授权忘了」的孤儿入口。
async fn init_role_menus_table(menu_ids: &HashMap<String, i64>) {
    if table_exists("role_menus").await {
        return;
    }
    let sql = r#"
    CREATE TABLE role_menus (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        role_id INTEGER NOT NULL,
        menu_id INTEGER NOT NULL,
        UNIQUE(role_id, menu_id)
    );
    "#;
    let pool = get_db_pool().await;
    let _ = pool.execute(sql).await;
    menu_seed::seed_role_menus(pool, menu_ids).await;
}

// ── user_menus（用户级菜单例外）────────────────────────────

/// 单用户的菜单例外：在「角色 → role_menus」之外，单独给某个人放行的侧边栏入口。
///
/// 与 `role_menus` 同构，只是作用域从「一类用户」收窄到「一个人」：给个别成员 /
/// 客户开小灶（例如把隐藏入口只对他显示）时，不必为此新建一个角色。
///
/// **只影响菜单渲染**，不是请求级鉴权依据（安全边界仍在 `routers::access`）——
/// 勾了菜单但缺 `*:view` / `*:edit` 权限点，页面照样会 403。
async fn init_user_menus_table() {
    let pool = get_db_pool().await;
    let sql = r#"
    CREATE TABLE IF NOT EXISTS user_menus (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL,
        menu_id INTEGER NOT NULL,
        created_at BIGINT,
        UNIQUE(user_id, menu_id)
    )
    "#;
    let _ = pool.execute(sql).await;
    let _ = pool
        .execute("CREATE INDEX IF NOT EXISTS idx_user_menus_user ON user_menus(user_id)")
        .await;
    let _ = pool
        .execute("CREATE INDEX IF NOT EXISTS idx_user_menus_menu ON user_menus(menu_id)")
        .await;
}

// ── role_permissions（动作级权限点）────────────────────────

/// 角色 → 权限点（`{ns}:view` / `{ns}:edit`）。
///
/// 与 `role_menus` 的区别：`role_menus` 只决定**前端菜单渲染**，
/// 这里才是**请求级鉴权**的依据（见 `routers::access`）。
///
/// 种子数据直接由 `access` 的权限矩阵推导：内置角色升级前后的可达范围完全一致，
/// 不会出现「加了权限校验后普通用户被锁死」的情况。新增自定义角色默认无权限，
/// 需管理员在「角色权限」中显式勾选（fail-closed）。
async fn init_role_permissions_table() {
    if !table_exists("role_permissions").await {
        let sql = r#"
    CREATE TABLE role_permissions (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        role_id INTEGER NOT NULL,
        perm_key VARCHAR(64) NOT NULL,
        UNIQUE(role_id, perm_key)
    );
    CREATE INDEX idx_role_permissions_role ON role_permissions(role_id);
    "#;
        let _ = get_db_pool().await.execute(sql).await;
    }

    // 建表与补齐分开：权限矩阵新增模块（如 database）后，老安装的内置角色
    // 也要拿到对应权限点，否则新功能对老用户直接 403。
    sync_builtin_role_permissions().await;
}

/// 把内置角色的权限点补齐到「权限矩阵推导出的默认值」。
///
/// 内置角色（admin/reseller/user/demo）属于系统定义，权限随代码升级而扩展；
/// 只做 INSERT OR IGNORE —— 只补缺失的权限点，不会删除管理员额外授予的权限点。
/// 自定义角色不受影响（保持 fail-closed，需管理员在「角色权限」中显式勾选）。
async fn sync_builtin_role_permissions() {
    let pool = get_db_pool().await;
    let roles: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, role_key FROM roles WHERE role_key IN ('admin','reseller','user','demo')",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (role_id, role_key) in roles {
        for perm in crate::routers::access::default_permissions_for(&role_key) {
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO role_permissions (role_id, perm_key) VALUES (?, ?)",
            )
            .bind(role_id)
            .bind(perm)
            .execute(pool)
            .await;
        }
    }
}

// ── audit logs ─────────────────────────────────────────────

async fn init_audit_table() {
    if table_exists("audit_logs").await {
        return;
    }
    let sql = r#"
    CREATE TABLE audit_logs (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL DEFAULT 0,
        username VARCHAR(128) NOT NULL DEFAULT '',
        action VARCHAR(64) NOT NULL DEFAULT '',
        target TEXT NOT NULL DEFAULT '',
        detail TEXT NOT NULL DEFAULT '',
        ip VARCHAR(64) NOT NULL DEFAULT '',
        created_at INTEGER
    );
    CREATE INDEX idx_audit_logs_action ON audit_logs(action);
    CREATE INDEX idx_audit_logs_user ON audit_logs(username);
    CREATE INDEX idx_audit_logs_created ON audit_logs(created_at);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── login attempt lockout ──────────────────────────────────

async fn init_login_attempts_table() {
    if table_exists("login_attempts").await {
        return;
    }
    let sql = r#"
    CREATE TABLE login_attempts (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        username VARCHAR(128) NOT NULL DEFAULT '',
        ip VARCHAR(64) NOT NULL DEFAULT '',
        failed_count INTEGER NOT NULL DEFAULT 0,
        locked_until INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER,
        UNIQUE(username, ip)
    );
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── login history ──────────────────────────────────────────

async fn init_login_history_table() {
    if table_exists("login_history").await {
        return;
    }
    let sql = r#"
    CREATE TABLE login_history (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL DEFAULT 0,
        username VARCHAR(128) NOT NULL DEFAULT '',
        ip VARCHAR(64) NOT NULL DEFAULT '',
        -- user_agent：登录来源（浏览器 / 设备），失败记录同样留存便于追溯
        user_agent TEXT NOT NULL DEFAULT '',
        -- status：success | failed | 2fa_failed
        status VARCHAR(32) NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_login_history_user ON login_history(user_id, created_at);
    CREATE INDEX idx_login_history_created ON login_history(created_at);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── hourly aggregated monitoring stats ─────────────────────

async fn init_hourly_stats_tables() {
    if !table_exists("system_stats_hourly").await {
        let sql = r#"
        CREATE TABLE system_stats_hourly (
            id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
            hour_start INTEGER NOT NULL,
            avg_loadavg_one REAL,
            avg_cpu_usage REAL,
            max_cpu_usage REAL,
            avg_memory_usage REAL,
            max_memory_usage REAL,
            avg_swap_usage REAL,
            UNIQUE(hour_start)
        );
        "#;
        let _ = get_db_pool().await.execute(sql).await;
    }
    if !table_exists("networks_stats_hourly").await {
        let sql = r#"
        CREATE TABLE networks_stats_hourly (
            id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            hour_start INTEGER NOT NULL,
            avg_received REAL,
            avg_transmitted REAL,
            max_received REAL,
            max_transmitted REAL,
            UNIQUE(name, hour_start)
        );
        "#;
        let _ = get_db_pool().await.execute(sql).await;
    }
}

// ── appstore ────────────────────────────────────────────────

// ── cron（脚本/自动化：计划任务）已改为 data/users/<user>/cron-jobs.yaml ──

// ── appstore ────────────────────────────────────────────────

/// 通用任务队列表 `task_queue`。
///
/// 这张表最早只服务应用商店的安装任务（旧名 `appstore_runs`），现在 Docker 构建、
/// 家目录备份、系统升级、计划任务都在往里登记，因此表名与用途对齐为 `task_queue`。
///
/// **旧库迁移**：`appstore_runs` 原地 `RENAME`（数据、主键、索引一并保留），
/// 再靠 `migrate_add_columns()` 把新增列补上，所以存量面板升级后历史记录不会丢。
async fn init_task_queue_table() {
    if table_exists("appstore_runs").await && !table_exists("task_queue").await {
        match get_db_pool()
            .await
            .execute("ALTER TABLE appstore_runs RENAME TO task_queue")
            .await
        {
            Ok(_) => println!("任务队列表已迁移: appstore_runs -> task_queue"),
            Err(e) => eprintln!("任务队列表迁移失败（保留旧表）: {e}"),
        }
    }
    if table_exists("task_queue").await {
        // 改表名不改列名：旧库的 `run_id` 列要一起跟着改成 `task_id`，
        // 否则新旧库的字段对不上，内核只能写两套 SQL。
        if column_exists("task_queue", "run_id").await
            && !column_exists("task_queue", "task_id").await
        {
            let renamed = get_db_pool()
                .await
                .execute("ALTER TABLE task_queue RENAME COLUMN run_id TO task_id")
                .await;
            match renamed {
                Ok(_) => println!("任务队列字段已迁移: run_id -> task_id"),
                Err(e) => eprintln!("任务队列字段迁移失败: {e}"),
            }
        }
        return;
    }
    let sql = r#"
    CREATE TABLE task_queue (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        task_id TEXT NOT NULL UNIQUE,
        -- kind：任务大类（appstore / docker / backup / system / cron / crontab / site / script），
        --   管理页按它分组展示；action 是类内的具体动作（install / image_build / ...）
        kind TEXT NOT NULL DEFAULT 'appstore',
        action TEXT NOT NULL DEFAULT '',
        -- pkg：任务对象（包名 / 镜像名 / 脚本路径 / 备份目标），由各 kind 自行解释
        pkg TEXT NOT NULL DEFAULT '',
        username TEXT NOT NULL DEFAULT '',
        -- status：pending（排队等并发槽）/ running / success / failed / canceled
        status TEXT NOT NULL DEFAULT 'running',
        exit_code INTEGER NOT NULL DEFAULT -1,
        log_path TEXT NOT NULL DEFAULT '',
        -- 归属键：同一次触发（如某个计划任务）的多次运行串在一起，
        --   "cron:<username>:<id>" | "crontab:<username>:<id>" | "docker-build:<username>"；
        --   空串表示手动触发。改表结构直接改这里，新增列靠 ensure_column 补。
        job_key TEXT NOT NULL DEFAULT '',
        -- 并发互斥组 + 组内并行上限：group_key 相同且 group_limit>0 时，
        --   组内同时处于 pending/running 的任务不得超过 group_limit
        --   （如应用商店编译全局只允许 1 个：group_key='appstore:compile', limit=1）。
        --   超出上限的登记为 pending，由调度器在前一个结束后放行。0 = 不限制。
        group_key TEXT NOT NULL DEFAULT '',
        group_limit INTEGER NOT NULL DEFAULT 0,
        -- 管理页展示用：人类可读标题与进度（0-100，-1 = 不适用）
        title TEXT NOT NULL DEFAULT '',
        progress INTEGER NOT NULL DEFAULT -1,
        -- control：控制指令（''=无 / 'cancel'=请求取消 / 'pause'=请求暂停），
        --   由管理页写入，执行侧（zapexec）读取并执行，执行后清空
        control TEXT NOT NULL DEFAULT '',
        -- payload：排队任务的启动参数（序列化后的 zapexec 请求）。
        --   排队任务要等前一个结束才启动，那时 HTTP 请求早已返回，
        --   只有把"该干什么"落库，调度器才知道怎么把它跑起来。
        payload TEXT NOT NULL DEFAULT '',
        started_at INTEGER NOT NULL DEFAULT 0,
        finished_at INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_task_queue_started ON task_queue(started_at);
    CREATE INDEX idx_task_queue_job ON task_queue(job_key, started_at);
    CREATE INDEX idx_task_queue_group ON task_queue(group_key, status);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

/// 老数据订正：自定义脚本的运行记录以前复用 AppStore 的登记入口，`kind` 是
/// `appstore`，任务队列里一律显示「应用商店」，看不出是谁跑的。
///
/// 脚本路径一定在 `scripts/` 下（`validate_script_path` 的约束），据此把存量
/// 记录归位到 `script`。幂等，每次启动都跑一遍也无妨。
async fn sync_script_task_kind() {
    if !table_exists("task_queue").await {
        return;
    }
    let _ = get_db_pool()
        .await
        .execute("UPDATE task_queue SET kind = 'script' WHERE kind = 'appstore' AND pkg LIKE 'scripts/%'")
        .await;
}

// ── ip_pool（IP 池管理）─────────────────────────────────────

async fn init_ip_pool_table() {
    if table_exists("ip_pool").await {
        return;
    }
    let sql = r#"
    CREATE TABLE ip_pool (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        address TEXT NOT NULL UNIQUE,
        version INTEGER NOT NULL DEFAULT 4,
        ip_type TEXT NOT NULL DEFAULT 'shared',
        reserved INTEGER NOT NULL DEFAULT 0,
        remark TEXT NOT NULL DEFAULT '',
        created_at INTEGER,
        updated_at INTEGER
    );
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── site（用户站点管理）─────────────────────────────────────

async fn init_site_table() {
    if table_exists("site").await {
        return;
    }
    let sql = r#"
    -- 站点主表：一个站点可绑定多个域名 / 多个 IP（见 site_domain / site_ip）
    CREATE TABLE site (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL DEFAULT 0,
        name TEXT NOT NULL DEFAULT '',
        php_instance TEXT NOT NULL DEFAULT '',
        vhost_state TEXT NOT NULL DEFAULT 'pending',
        vhost_error TEXT NOT NULL DEFAULT '',
        vhost_synced_at INTEGER NOT NULL DEFAULT 0,
        run_state TEXT NOT NULL DEFAULT 'running',
        web_root TEXT NOT NULL DEFAULT '',
        log_root TEXT NOT NULL DEFAULT '',
        status INTEGER NOT NULL DEFAULT 1,
        remark TEXT NOT NULL DEFAULT '',
        -- 流量统计：access.log 增量解析游标（inode 变化 / 文件变小 = 日志轮转，从 0 重读）
        traffic_offset INTEGER NOT NULL DEFAULT 0,
        traffic_inode TEXT NOT NULL DEFAULT '',
        traffic_total_bytes INTEGER NOT NULL DEFAULT 0,
        -- 本月流量（字节）与所属周期（YYYYMM）
        traffic_month_bytes INTEGER NOT NULL DEFAULT 0,
        traffic_month TEXT NOT NULL DEFAULT '',
        traffic_stat_at INTEGER NOT NULL DEFAULT 0,
        -- 站点磁盘占用（字节）：web_root + log_root 的 du 结果（0 = 尚未采集）
        disk_used_bytes INTEGER NOT NULL DEFAULT 0,
        disk_stat_at INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER,
        updated_at INTEGER
    );
    CREATE INDEX idx_site_user_id ON site(user_id);

    -- 站点按天流量（解析 access.log 增量累加，供站点流量分析曲线使用）
    CREATE TABLE IF NOT EXISTS site_traffic_daily (
        site_id INTEGER NOT NULL,
        day TEXT NOT NULL,              -- YYYYMMDD
        bytes INTEGER NOT NULL DEFAULT 0,
        requests INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (site_id, day)
    );

    -- 站点按天 Top URL（每天每站点最多保留 TOP_PATH_KEEP 条，其余按 hits 修剪）
    CREATE TABLE IF NOT EXISTS site_traffic_path (
        site_id INTEGER NOT NULL,
        day TEXT NOT NULL,              -- YYYYMMDD
        path TEXT NOT NULL,
        hits INTEGER NOT NULL DEFAULT 0,
        bytes INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (site_id, day, path)
    );
    CREATE INDEX idx_traffic_path_day ON site_traffic_path(site_id, day);

    CREATE TABLE site_domain (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        site_id INTEGER NOT NULL DEFAULT 0,
        domain TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX idx_site_domain_site_id ON site_domain(site_id);
    CREATE UNIQUE INDEX idx_site_domain_domain ON site_domain(domain);

    CREATE TABLE site_ip (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        site_id INTEGER NOT NULL DEFAULT 0,
        ip TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX idx_site_ip_site_id ON site_ip(site_id);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── site_profile（站点扩展档案，1:1 site.id）────────────────────

async fn init_site_profile_table() {
    let sql = r#"
    CREATE TABLE IF NOT EXISTS site_profile (
        site_id INTEGER NOT NULL PRIMARY KEY,
        site_type TEXT NOT NULL DEFAULT 'php',
        pseudo_static TEXT NOT NULL DEFAULT 'none',
        pseudo_custom TEXT NOT NULL DEFAULT '',
        web_root_custom INTEGER NOT NULL DEFAULT 0,
        upstreams TEXT NOT NULL DEFAULT '[]',
        locations TEXT NOT NULL DEFAULT '[]',
        ssl_cert_id INTEGER NOT NULL DEFAULT 0,
        force_https INTEGER NOT NULL DEFAULT 0,
        -- TLS 高级设置（空串/缺省 = 面板默认：协议回退 TLSv1.2+TLSv1.3，密码套件不输出）
        ssl_http2 INTEGER NOT NULL DEFAULT 1,
        ssl_prefer_server_ciphers INTEGER NOT NULL DEFAULT 1,
        ssl_protocols TEXT NOT NULL DEFAULT '',
        ssl_ciphers TEXT NOT NULL DEFAULT '',
        updated_at INTEGER NOT NULL DEFAULT 0
    );
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── api_token（API Token 管理）──────────────────────────────

async fn init_api_token_table() {
    let sql = r#"
    CREATE TABLE IF NOT EXISTS api_token (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL DEFAULT 0,
        name TEXT NOT NULL DEFAULT '',
        token_hash TEXT NOT NULL DEFAULT '',
        prefix TEXT NOT NULL DEFAULT '',
        last_used_at INTEGER NOT NULL DEFAULT 0,
        expires_at INTEGER NOT NULL DEFAULT 0,
        status INTEGER NOT NULL DEFAULT 1,
        -- token_version：该 Token 记录的会话版本号。「下线所有设备」时统一推高，
        --   版本号落后的 Token 在下次请求时判为已下线（见 zap::session::bump）
        token_version INTEGER NOT NULL DEFAULT 0,
        -- scope：'' 普通用户 Token / 'cluster' 集群节点机器凭据（Zap Pro）。
        --   'cluster' 凭据由 access::guard 收口：只能访问 /pro/cluster/agent/**
        scope TEXT NOT NULL DEFAULT '',
        created_at INTEGER,
        updated_at INTEGER
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_api_token_hash ON api_token(token_hash);
    CREATE INDEX IF NOT EXISTS idx_api_token_user ON api_token(user_id);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── ssl_cert（SSL/TLS 证书管理）────────────────────────────

async fn init_ssl_cert_table() {
    let sql = r#"
    CREATE TABLE IF NOT EXISTS ssl_cert (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        -- user_id：证书归属用户（0 = 历史系统证书，仅管理员可见，可在编辑中转为归属某用户）
        user_id INTEGER NOT NULL DEFAULT 0,
        name TEXT NOT NULL DEFAULT '',
        domains TEXT NOT NULL DEFAULT '',
        cert_type TEXT NOT NULL DEFAULT 'upload',
        cert_content TEXT NOT NULL DEFAULT '',
        key_content TEXT NOT NULL DEFAULT '',
        ca_bundle TEXT NOT NULL DEFAULT '',
        csr TEXT NOT NULL DEFAULT '',
        not_before INTEGER NOT NULL DEFAULT 0,
        not_after INTEGER NOT NULL DEFAULT 0,
        status INTEGER NOT NULL DEFAULT 1,
        remark TEXT NOT NULL DEFAULT '',
        created_at INTEGER,
        updated_at INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_ssl_cert_user ON ssl_cert(user_id);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── SSL/TLS：ACME（Let's Encrypt）配套表 ──────────────────────
//
// 三张表围绕「异步订单」设计：
//   account     —— 复用 ACME 账户，避免每次申请都注册新账户（LE 有 rate limit）
//   order       —— 订单全流程落库，进程重启 / 前端刷新都能接着走
//   dns_provider—— DNS 服务商 API 凭据（DNS-01 自动模式），凭据一律加密存储

/// ssl_acme_account：ACME 账户（按 用户 + 邮箱 + 环境 唯一）。
async fn init_ssl_acme_account_table() {
    let sql = r#"
    CREATE TABLE IF NOT EXISTS ssl_acme_account (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL DEFAULT 0,
        -- letsencrypt | letsencrypt-staging
        directory TEXT NOT NULL DEFAULT 'letsencrypt',
        email TEXT NOT NULL DEFAULT '',
        -- AccountCredentials JSON（zap-crypto 加密后存储）
        credentials TEXT NOT NULL DEFAULT '',
        account_url TEXT NOT NULL DEFAULT '',
        created_at INTEGER,
        updated_at INTEGER
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_ssl_acme_account_uniq
        ON ssl_acme_account(user_id, directory, email);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

/// ssl_acme_order：ACME 订单与其挑战明细。
async fn init_ssl_acme_order_table() {
    let sql = r#"
    CREATE TABLE IF NOT EXISTS ssl_acme_order (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL DEFAULT 0,
        account_id INTEGER NOT NULL DEFAULT 0,
        -- 签发成功后回填 ssl_cert.id
        cert_id INTEGER NOT NULL DEFAULT 0,
        -- 证书名（签发后写入 ssl_cert.name）
        name TEXT NOT NULL DEFAULT '',
        domains TEXT NOT NULL DEFAULT '',
        -- let's encrypt 环境同 account.directory
        directory TEXT NOT NULL DEFAULT 'letsencrypt',
        -- pending | processing | issued | failed | cancelled | expired
        status TEXT NOT NULL DEFAULT 'pending',
        -- http-01 | dns-01
        challenge_type TEXT NOT NULL DEFAULT 'http-01',
        -- 仅 dns-01 有意义：manual（用户自行解析）| auto（调 DNS API）
        dns_mode TEXT NOT NULL DEFAULT 'manual',
        dns_provider_id INTEGER NOT NULL DEFAULT 0,
        -- 订单 URL：配合账户凭据即可用 Account::order(url) 恢复句柄
        order_url TEXT NOT NULL DEFAULT '',
        -- 私钥 PEM（zap-crypto 加密存）；签发后明文转入 ssl_cert.key_content
        key_pem TEXT NOT NULL DEFAULT '',
        -- JSON 数组：[{domain, status, challenge_url, token, key_auth, dns_host, dns_value,
        --             dns_record_id, propagated}]
        challenges TEXT NOT NULL DEFAULT '[]',
        error TEXT NOT NULL DEFAULT '',
        expires_at INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER,
        updated_at INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_ssl_acme_order_user ON ssl_acme_order(user_id);
    CREATE INDEX IF NOT EXISTS idx_ssl_acme_order_status ON ssl_acme_order(status);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

/// ssl_acme_dns_provider：DNS 服务商 API 凭据（DNS-01 自动验证用）。
async fn init_ssl_acme_dns_provider_table() {
    let sql = r#"
    CREATE TABLE IF NOT EXISTS ssl_acme_dns_provider (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL DEFAULT 0,
        name TEXT NOT NULL DEFAULT '',
        -- cloudflare | dnspod（与 zap::acme::dns::providers() 保持一致）
        provider TEXT NOT NULL DEFAULT '',
        -- 各服务商所需字段的 JSON（zap-crypto 加密后存储），读接口一律脱敏
        credentials TEXT NOT NULL DEFAULT '',
        remark TEXT NOT NULL DEFAULT '',
        status INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER,
        updated_at INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_ssl_acme_dns_provider_user ON ssl_acme_dns_provider(user_id);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── fpm_spec（PHP-FPM 规格模板库，仅 admin 维护）────────────────

async fn init_fpm_spec_table() {
    let sql = r#"
    CREATE TABLE IF NOT EXISTS fpm_spec (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL DEFAULT '',
        spec TEXT NOT NULL DEFAULT '',
        remark TEXT NOT NULL DEFAULT '',
        created_at INTEGER,
        updated_at INTEGER
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_fpm_spec_name ON fpm_spec(name);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── notice_message（站内信 / 通知中心）────────────────────────

async fn init_notice_message_table() {
    let sql = r#"
    CREATE TABLE IF NOT EXISTS notice_message (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL DEFAULT 0,
        type TEXT NOT NULL DEFAULT '',
        title TEXT NOT NULL DEFAULT '',
        body TEXT NOT NULL DEFAULT '',
        is_read INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_notice_user ON notice_message(user_id, id);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── helper ─────────────────────────────────────────────────

async fn table_exists(table_name: &str) -> bool {
    let pool = get_db_pool().await;
    let result: Result<(String,), sqlx::Error> =
        sqlx::query_as("select name from sqlite_master where name = ?")
            .bind(table_name)
            .fetch_one(pool)
            .await;
    result.is_ok()
}

/// 列是否存在（`pragma_table_info` 查表结构，用于幂等改名 / 补列）。
async fn column_exists(table: &str, column: &str) -> bool {
    if !table_exists(table).await {
        return false;
    }
    let pool = get_db_pool().await;
    sqlx::query_scalar::<_, bool>("SELECT COUNT(*) > 0 FROM pragma_table_info(?) WHERE name = ?")
        .bind(table)
        .bind(column)
        .fetch_one(pool)
        .await
        .unwrap_or(false)
}
