use sqlx::Executor;

use super::get_db_pool;

/// 建表与种子数据入口。
///
/// **约定（开发阶段）**：表结构改动以 `CREATE TABLE` 为准，全新数据库直接按此建表；
/// 对**已存在**的库，新增列通过 `migrate_add_columns()` 幂等 `ALTER TABLE ... ADD COLUMN`
/// 补齐（只加列，不做数据搬运 / 改类型 / 版本水位表），所以加列后**不必** `--reset-db`；
/// 删列、改类型、改约束仍需重建数据库。
pub async fn init_schema() {
    init_system_user_table_schema().await;
    init_system_monitor_table_schema().await;
    init_system_monitor_networks_table_schema().await;
    init_roles_table().await;
    init_menus_table().await;
    init_role_menus_table().await;
    // 用户级菜单例外：给单个用户开小灶 / 收窄入口（仅渲染层）
    init_user_menus_table().await;
    // 老库补入口：新增菜单对已存在的库同样生效（见函数注释）
    sync_added_menus().await;
    // 动作级权限点（请求级鉴权依据；role_menus 仅用于菜单渲染）
    init_role_permissions_table().await;
    init_audit_table().await;
    init_login_attempts_table().await;
    init_hourly_stats_tables().await;
    crate::routers::ssh_terminal::init_table().await;
    // 通用任务队列（应用商店安装 / Docker 构建 / 备份 / 升级 / 计划任务都登记在这里）
    init_task_queue_table().await;
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

/// 历史库新增列清单：CREATE TABLE 里加列后，同步登记到这里即可自动迁移。
async fn migrate_add_columns() {
    // user：磁盘用量（du 家目录）+ 本月带宽（access.log 汇总）
    ensure_column("user", "disk_used_bytes", "INTEGER NOT NULL DEFAULT 0").await;
    ensure_column("user", "disk_stat_at", "INTEGER NOT NULL DEFAULT 0").await;
    ensure_column("user", "bandwidth_used_bytes", "INTEGER NOT NULL DEFAULT 0").await;
    ensure_column("user", "bandwidth_period", "TEXT NOT NULL DEFAULT ''").await;
    ensure_column("user", "bandwidth_stat_at", "INTEGER NOT NULL DEFAULT 0").await;
    // site：access.log 增量解析游标 + 流量计数
    ensure_column("site", "traffic_offset", "INTEGER NOT NULL DEFAULT 0").await;
    ensure_column("site", "traffic_inode", "TEXT NOT NULL DEFAULT ''").await;
    ensure_column("site", "traffic_total_bytes", "INTEGER NOT NULL DEFAULT 0").await;
    ensure_column("site", "traffic_month_bytes", "INTEGER NOT NULL DEFAULT 0").await;
    ensure_column("site", "traffic_month", "TEXT NOT NULL DEFAULT ''").await;
    ensure_column("site", "traffic_stat_at", "INTEGER NOT NULL DEFAULT 0").await;
    // site：磁盘占用（web_root + log_root）
    ensure_column("site", "disk_used_bytes", "INTEGER NOT NULL DEFAULT 0").await;
    ensure_column("site", "disk_stat_at", "INTEGER NOT NULL DEFAULT 0").await;
    // user：子账号（成员）支持
    ensure_column("user", "user_kind", "INTEGER NOT NULL DEFAULT 0").await;
    ensure_column("user", "perm_deny", "TEXT NOT NULL DEFAULT ''").await;
    // user：只读账号（共享可见但不可改）
    ensure_column("user", "read_only", "INTEGER NOT NULL DEFAULT 0").await;
    // menus：能力门禁列（依赖后台组件的菜单靠它决定是否下发）
    ensure_column("menus", "feature", "TEXT NOT NULL DEFAULT ''").await;
    // task_queue：由 appstore_runs 改名而来的旧库缺这些通用队列列
    ensure_column("task_queue", "kind", "TEXT NOT NULL DEFAULT 'appstore'").await;
    ensure_column("task_queue", "group_key", "TEXT NOT NULL DEFAULT ''").await;
    ensure_column("task_queue", "group_limit", "INTEGER NOT NULL DEFAULT 0").await;
    ensure_column("task_queue", "title", "TEXT NOT NULL DEFAULT ''").await;
    ensure_column("task_queue", "progress", "INTEGER NOT NULL DEFAULT -1").await;
    ensure_column("task_queue", "control", "TEXT NOT NULL DEFAULT ''").await;
    ensure_column("task_queue", "payload", "TEXT NOT NULL DEFAULT ''").await;
    ensure_column("task_queue", "updated_at", "INTEGER NOT NULL DEFAULT 0").await;
}

/// 菜单能力门禁赋值：给「依赖后台组件」的菜单打上 `feature` 标记。
///
/// 必须在 `migrate_add_columns()` 之后跑（老库要先补出 `feature` 列），
/// 故独立成一个函数而不是并进 `sync_added_menus()`。幂等，只改未标记的行。
async fn sync_menu_features() {
    let pool = get_db_pool().await;
    // 容器管理（17 父 / 171 子）：没装 Docker 的机器不该出现入口，
    // 装包后 `zap::feature` 重新探测到即自动出现在侧栏。
    let _ = sqlx::query(
        "UPDATE menus SET feature = 'docker', updated_at = strftime('%s','now') \
         WHERE id IN (17, 171) AND feature <> 'docker'",
    )
    .execute(pool)
    .await;
}

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

    // Insert admin with runtime-generated bcrypt hash (avoids $2y$ prefix issues).
    // Password: use $ZAP_ADMIN_PASSWORD if set (fresh DBs only), otherwise default "123456".
    let default_password = std::env::var("ZAP_ADMIN_PASSWORD")
        .map(|p| p.trim().to_string())
        .unwrap_or_default();
    let default_password = if default_password.is_empty() {
        "123456".to_string()
    } else {
        default_password
    };
    let hashed = bcrypt::hash(&default_password, bcrypt::DEFAULT_COST)
        .expect("failed to hash default password");
    let now = chrono::Utc::now().timestamp();

    sqlx::query(
        "INSERT INTO user (username, home_dir, linux_user, password, email, nickname, phone, last_login_time, last_login_ip, status, roles, permissions, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 'admin', '', ?, ?)",
    )
    .bind("admin")
    .bind("/home/admin")
    .bind("admin")
    .bind(&hashed)
    .bind("admin@demo.zap.cn")
    .bind("admin")
    .bind("18826002600")
    .bind(now)
    .bind("127.0.0.1")
    .bind(now)
    .bind(now)
    .execute(get_db_pool().await)
    .await
    .unwrap();
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

/// 幂等补齐「后续版本新增」的菜单入口。
///
/// `init_menus_table` 只在建表时跑一次，老库升级后拿不到新功能的入口，
/// 于是新页面做了也看不见（侧边栏由 menus 表驱动）。这里只 INSERT 缺失项：
///
/// - 菜单：父菜单存在才补；
/// - 授权：已拥有同级菜单（SSL 证书）的角色一并获得新入口，
///   免得老安装升级后管理员还得手工去「角色权限」里勾。
///
/// 重复执行无副作用（主键 / UNIQUE(role_id, menu_id) 冲突即忽略）。
async fn sync_added_menus() {
    let pool = get_db_pool().await;
    // 应用商店收敛为侧栏单一入口：原来「应用商店 / 已安装应用」两个子菜单改成
    // 页面内的 nav pill（全部应用 / 已安装 / 我的站点应用），任务队列与日志走抽屉。
    // 只隐藏不删除：菜单管理里仍可查到，/appstore/installed 由前端重定向兜底。
    let _ = sqlx::query(
        "UPDATE menus SET hidden = 1, updated_at = strftime('%s','now') \
         WHERE id = 62 AND hidden <> 1",
    )
    .execute(pool)
    .await;
    // 子菜单只剩 index 一个时，侧栏会把父项渲染成单一链接（SidebarItem.hasOneShowingChild）
    let _ = sqlx::query(
        "UPDATE menus SET hidden = 0, updated_at = strftime('%s','now') \
         WHERE id = 61 AND hidden <> 0",
    )
    .execute(pool)
    .await;
    // SSL/TLS → DNS 服务商（ACME DNS-01 自动模式的服务商凭据）
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO menus
            (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
         SELECT 112, 11, 'ssl-dns-providers', 'dns-providers', 'ssl-tls/dns-providers/index',
                'menu', 'DNS服务商', 'material-symbols:dns', 0, 'admin,user', 2, 1,
                strftime('%s','now'), strftime('%s','now')
         WHERE EXISTS (SELECT 1 FROM menus WHERE id = 11)",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO role_menus (role_id, menu_id)
         SELECT role_id, 112 FROM role_menus WHERE menu_id = 111",
    )
    .execute(pool)
    .await;
    // SSL/TLS 收敛为侧栏单一入口：「DNS服务商」子菜单（112）收起，
    // 改为证书列表页头部按钮 + 抽屉（DnsProvidersPane）。
    // 只隐藏不删除：页面路由 /ssl-tls/dns-providers 仍可达，菜单管理里也还能勾回来。
    let _ = sqlx::query(
        "UPDATE menus SET hidden = 1, updated_at = strftime('%s','now') \
         WHERE id = 112 AND hidden <> 1",
    )
    .execute(pool)
    .await;

    // 「脚本/自动化」收进「系统设置」：一级目录只为两个页面而存在，太占地方。
    // 子菜单改挂到 system（id=2）下排在末尾 —— 路径是相对父级的，所以 URL 跟着
    // 变成 /system/scripts 与 /system/cron，无需改 component。
    // 目录 10 保留但 hidden=1：id 段已分配、role_menus 里还有指向它的行，
    // 删掉会让这些授权记录变成孤儿，隐藏即可让侧栏干净。
    let _ = sqlx::query(
        "UPDATE menus SET parent_id = 2, sort_order = 10, updated_at = strftime('%s','now') \
         WHERE id = 101 AND parent_id <> 2",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "UPDATE menus SET parent_id = 2, sort_order = 11, updated_at = strftime('%s','now') \
         WHERE id = 102 AND parent_id <> 2",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "UPDATE menus SET hidden = 1, updated_at = strftime('%s','now') \
         WHERE id = 10 AND hidden <> 1",
    )
    .execute(pool)
    .await;

    // 容器管理（Docker）：位于「计划任务」之下，管理员专属单页（nav pill 内切换
    // 容器 / 镜像 / 卷 / 网络 / Compose，故只需要一个子菜单）。
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO menus
            (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
         SELECT 17, 0, 'docker', '/docker', 'Layout', '/docker/index', 'dir', '容器管理',
                'material-symbols:deployed-code', 0, 'admin', 5, 1,
                strftime('%s','now'), strftime('%s','now')
         WHERE NOT EXISTS (SELECT 1 FROM menus WHERE id = 17)",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO menus
            (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
         SELECT 171, 17, 'docker-index', 'index', 'docker/index', '', 'menu', '容器',
                'material-symbols:deployed-code', 0, 'admin', 1, 1,
                strftime('%s','now'), strftime('%s','now')
         WHERE EXISTS (SELECT 1 FROM menus WHERE id = 17)",
    )
    .execute(pool)
    .await;
    // 菜单授权：menus.roles 只用于前端排序参考，侧栏可见性由 role_menus 决定，
    // 不补这条的话管理员在新菜单上线后依然看不到入口。
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO role_menus (role_id, menu_id)
         SELECT r.id, 17 FROM roles r WHERE r.role_key = 'admin'",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO role_menus (role_id, menu_id)
         SELECT r.id, 171 FROM roles r WHERE r.role_key = 'admin'",
    )
    .execute(pool)
    .await;

    // 任务队列：管理员的全局任务视角（应用商店安装 / Docker 构建 / 备份 / 升级 /
    // 计划任务的运行记录都汇总在这里），挂在「系统设置」下。
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO menus
            (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
         SELECT 29, 2, 'tasks', 'tasks', 'system/tasks/index', 'menu', '任务队列',
                'material-symbols:view-list', 0, 'admin', 6, 1,
                strftime('%s','now'), strftime('%s','now')
         WHERE EXISTS (SELECT 1 FROM menus WHERE id = 2)",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO role_menus (role_id, menu_id)
         SELECT r.id, 29 FROM roles r WHERE r.role_key = 'admin'",
    )
    .execute(pool)
    .await;

    // 团队成员（子账号）：任意用户管理自己名下的成员，故对 admin/user/reseller 全部开放。
    // 入口已并入「个人中心」（顶栏头像 → 个人中心 → 团队成员 pill），侧边栏不再单列，
    // 故两条都 hidden=1；路由 /team 仍保留，旧链接不会 404。
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO menus
            (id, parent_id, name, path, component, redirect, type, title, icon, hidden, affix, roles, sort_order, status, created_at, updated_at)
         SELECT 18, 0, 'team', '/team', 'Layout', '/team/index', 'dir', '团队成员',
                'material-symbols:group', 1, 0, 'admin,user,reseller', 9, 1,
                strftime('%s','now'), strftime('%s','now')
         WHERE NOT EXISTS (SELECT 1 FROM menus WHERE id = 18)",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO menus
            (id, parent_id, name, path, component, type, title, icon, hidden, affix, roles, sort_order, status, created_at, updated_at)
         SELECT 181, 18, 'team-index', 'index', 'team/index', 'menu', '团队成员',
                'material-symbols:group', 1, 1, 'admin,user,reseller', 1, 1,
                strftime('%s','now'), strftime('%s','now')
         WHERE EXISTS (SELECT 1 FROM menus WHERE id = 18)",
    )
    .execute(pool)
    .await;
    // 老库在入口下线前已存在这两行，INSERT OR IGNORE 改不到，这里补一道幂等收敛。
    // 只改 hidden：菜单保留（界面「菜单管理」里仍可查到），不删是为了避免下次启动被重新补齐。
    let _ = sqlx::query(
        "UPDATE menus SET hidden = 1, updated_at = strftime('%s','now')
         WHERE id IN (18, 181) AND hidden <> 1",
    )
    .execute(pool)
    .await;
    // 侧栏可见性由 role_menus 决定：沿用「站点」菜单（91）的授权集合
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO role_menus (role_id, menu_id)
         SELECT role_id, 18 FROM role_menus WHERE menu_id = 91",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO role_menus (role_id, menu_id)
         SELECT role_id, 181 FROM role_menus WHERE menu_id = 91",
    )
    .execute(pool)
    .await;

    // About ZAP（版本信息 + 文档入口）：原侧栏「文档」一级菜单（16 / 161-164）
    // 已整合进来，挂在「系统设置」下只留一个入口。
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO menus
            (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
         SELECT 30, 2, 'about', 'about', 'system/about/index', 'menu', 'About ZAP',
                'material-symbols:info', 0, 'admin,user,reseller,demo', 9, 1,
                strftime('%s','now'), strftime('%s','now')
         WHERE EXISTS (SELECT 1 FROM menus WHERE id = 2)",
    )
    .execute(pool)
    .await;
    // 文档 md 无敏感信息，沿用原「文档」菜单的授权集合（admin/user/reseller/demo），
    // 避免普通用户升级后找不到文档。
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO role_menus (role_id, menu_id)
         SELECT role_id, 30 FROM role_menus WHERE menu_id = 16",
    )
    .execute(pool)
    .await;
    // 父目录也要一并授权：子菜单授权父不授权时，整棵子树在 build_menu_tree 里消失。
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO role_menus (role_id, menu_id)
         SELECT role_id, 2 FROM role_menus WHERE menu_id = 16",
    )
    .execute(pool)
    .await;
    // 旧「文档」菜单整组隐藏：菜单保留（菜单管理里仍可查到），路由 /docs/<id> 由前端
    // 以隐藏路由常驻，旧链接不会 404。
    let _ = sqlx::query(
        "UPDATE menus SET hidden = 1, updated_at = strftime('%s','now')
         WHERE id IN (16, 161, 162, 163, 164) AND hidden <> 1",
    )
    .execute(pool)
    .await;
}

async fn init_menus_table() {
    if table_exists("menus").await {
        return;
    }
    let sql = r#"
    CREATE TABLE menus (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        parent_id INTEGER DEFAULT 0,
        name VARCHAR(64) NOT NULL,
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

    -- Dashboard
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (1, 0, 'dashboard', '/dashboard', 'dashboard/index', '', 'menu', '仪表盘', 'material-symbols:home', 1, 'admin,user', 1, 1, strftime('%s','now'), strftime('%s','now'));

    -- 站点管理（Layout 包裹 + 一级直链：单个子菜单，位于仪表盘之下）
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (9, 0, 'site', '/site', 'Layout', '/site/index', 'menu', '站点', 'material-symbols:public', 1, 'admin,user,reseller', 2, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (91, 9, 'site-index', 'index', 'site/index', 'menu', '站点', 'material-symbols:public', 1, 'admin,user,reseller', 1, 1, strftime('%s','now'), strftime('%s','now'));

    -- 数据库管理（Layout 包裹 + 一级直链：紧随站点之后；user 仅能管自己前缀的库）
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (14, 0, 'database', '/database', 'Layout', '/database/index', 'menu', '数据库', 'material-symbols:database', 1, 'admin,user', 3, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (141, 14, 'database-index', 'index', 'database/index', 'menu', '数据库', 'material-symbols:database', 1, 'admin,user', 1, 1, strftime('%s','now'), strftime('%s','now'));

    -- System dir
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (2, 0, 'system', '/system', 'Layout', '/system/access', 'dir', '系统设置', 'material-symbols:settings', 1, 'admin', 12, 1, strftime('%s','now'), strftime('%s','now'));

    -- System children
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (26, 2, 'basic-config', 'basic-config', 'system/config/basic', 'menu', '基础设置', 'material-symbols:tune', 1, 'admin', 1, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (28, 2, 'zap-config', 'zap-config', 'system/config/zap', 'menu', 'Zap 设置', 'material-symbols:settings-applications', 1, 'admin', 2, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    -- 用户管理 + 角色管理合到一页（页面内 nav pill 切换）
    VALUES (21, 2, 'access', 'access', 'system/access/index', 'menu', '用户与角色', 'material-symbols:badge', 1, 'admin', 3, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    -- 角色管理已并入 21；保留本行只为兼容既有 role_menus 授权，status=0 不进侧栏
    VALUES (22, 2, 'roles', 'roles', 'system/access/index', 'menu', '角色管理', 'material-symbols:visibility', 1, 'admin', 4, 0, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (23, 2, 'menus', 'menus', 'system/menus/index', 'menu', '菜单管理', 'material-symbols:menu', 1, 'admin', 5, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (24, 2, 'audit', 'audit', 'system/audit/index', 'menu', '审计日志', 'material-symbols:confirmation-number', 1, 'admin', 7, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (27, 2, 'system-update', 'update', 'system/update/index', 'menu', '系统更新', 'material-symbols:refresh', 1, 'admin', 8, 1, strftime('%s','now'), strftime('%s','now'));

    -- Server config dir（服务器配置：运维项；原「服务配置」一级菜单已并入其中）
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (7, 0, 'server', '/server', 'Layout', '/server/system', 'dir', '服务器配置', 'material-symbols:tune', 1, 'admin', 9, 1, strftime('%s','now'), strftime('%s','now'));

    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    -- 系统管理：服务器时间 / 系统服务 / SSH 服务 / 进程管理 合到一页（页面内 nav pill 切换）
    VALUES (71, 7, 'server-system', 'system', 'server/system/index', 'menu', '系统管理', 'material-symbols:settings', 1, 'admin', 1, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    -- 系统服务已并入 71；保留本行只为兼容既有 role_menus 授权，status=0 不进侧栏
    VALUES (72, 7, 'server-services', 'services', 'server/system/index', 'menu', '系统服务', 'material-symbols:build', 1, 'admin', 2, 0, strftime('%s','now'), strftime('%s','now'));
    -- 服务配置：Nginx / PHP / MySQL 的配置合到一页，页面内用 nav pill 切换
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (82, 7, 'server-service-conf', 'service-conf', 'server/service-conf/index', 'menu', '服务配置', 'material-symbols:dns', 1, 'admin', 3, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    -- SSH 服务已并入 71
    VALUES (73, 7, 'server-ssh', 'ssh', 'server/system/index', 'menu', 'SSH 服务', 'material-symbols:cable', 1, 'admin', 4, 0, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    -- 进程管理已并入 71
    VALUES (74, 7, 'server-process', 'process', 'server/system/index', 'menu', '进程管理', 'material-symbols:memory', 1, 'admin', 5, 0, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    -- 网络配置：网络设置 + IP 设置 合到一页
    VALUES (75, 7, 'server-network', 'network', 'server/network/index', 'menu', '网络配置', 'material-symbols:link', 1, 'admin', 6, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    -- IP 设置已并入 75
    VALUES (76, 7, 'server-ip', 'ip', 'server/network/index', 'menu', 'IP 设置', 'material-symbols:badge', 1, 'admin', 7, 0, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (80, 7, 'server-firewall', 'firewall', 'server/firewall/index', 'menu', '防火墙', 'material-symbols:lock', 1, 'admin', 8, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (77, 7, 'server-env', 'env', 'server/env/index', 'menu', '运行环境', 'material-symbols:auto-fix-high', 1, 'admin', 9, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    -- 同步运行环境已并入 77（运行环境页第二个 nav pill）
    VALUES (78, 7, 'server-entities', 'entities', 'server/env/index', 'menu', '同步运行环境', 'material-symbols:account-circle', 1, 'admin', 10, 0, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (79, 7, 'server-migrate', 'migrate', 'server/migrate/index', 'menu', '数据迁移', 'material-symbols:sort', 1, 'admin', 11, 1, strftime('%s','now'), strftime('%s','now'));

    -- Terminal（Layout 包裹 + 一级直链：单个子菜单）
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (4, 0, 'terminal', '/terminal', 'Layout', '/terminal/index', 'menu', '终端', 'material-symbols:monitor', 1, 'admin,user', 4, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (41, 4, 'terminal-index', 'index', 'terminal/index', 'menu', '终端', 'material-symbols:monitor', 1, 'admin,user', 1, 1, strftime('%s','now'), strftime('%s','now'));

    -- File manager（Layout 包裹 + 一级直链：单个子菜单）
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (3, 0, 'files', '/files', 'Layout', '/files/index', 'menu', '文件管理', 'material-symbols:folder', 1, 'admin,user', 3, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (31, 3, 'files-index', 'index', 'files/index', 'menu', '文件管理', 'material-symbols:folder', 1, 'admin,user', 1, 1, strftime('%s','now'), strftime('%s','now'));

    -- Reseller customer management (Layout + child page)
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (5, 0, 'reseller-users', '/reseller/users', 'Layout', '/reseller/users/index', 'menu', '客户管理', 'material-symbols:account-circle', 1, 'reseller', 5, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (51, 5, 'reseller-users-index', 'index', 'system/access/index', 'menu', '客户管理', 'material-symbols:account-circle', 1, 'reseller', 1, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (52, 5, 'reseller-packages', 'packages', 'system/packages/index', 'menu', '套餐', 'material-symbols:storefront', 0, 'admin,reseller', 2, 1, strftime('%s','now'), strftime('%s','now'));

    -- SSL/TLS（Layout + 子菜单，位于应用商店之前，admin/user）
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (11, 0, 'ssl-tls', '/ssl-tls', 'Layout', '/ssl-tls/certs', 'dir', 'SSL/TLS', 'material-symbols:lock', 1, 'admin,user', 6, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (111, 11, 'ssl-certs', 'certs', 'ssl-tls/certs/index', 'menu', 'SSL证书', 'material-symbols:lock', 1, 'admin,user', 1, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (112, 11, 'ssl-dns-providers', 'dns-providers', 'ssl-tls/dns-providers/index', 'menu', 'DNS服务商', 'material-symbols:dns', 0, 'admin,user', 2, 1, strftime('%s','now'), strftime('%s','now'));

    -- AppStore (Layout + children)
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (6, 0, 'appstore', '/appstore', 'Layout', '/appstore/index', 'menu', '应用商店', 'material-symbols:storefront', 1, 'admin,user', 7, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (61, 6, 'appstore-index', 'index', 'appstore/index', 'menu', '应用商店', 'material-symbols:storefront', 1, 'admin,user', 1, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (62, 6, 'installed', 'installed', 'appstore/installed', 'menu', '已安装应用', 'material-symbols:deployed-code', 1, 'admin,user,reseller', 2, 1, strftime('%s','now'), strftime('%s','now'));

    -- Server status（子菜单：服务器信息 tabs + Nginx Server，应用商店之后，admin）
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (8, 0, 'server-status', '/server-status', 'Layout', '/server-status/index', 'dir', '服务器状态', 'material-symbols:monitor-heart', 1, 'admin', 8, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (81, 8, 'server-status-index', 'index', 'server-status/index', 'menu', 'Server Monitor', 'material-symbols:monitoring', 1, 'admin', 1, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (87, 8, 'server-status-nginx', 'nginx-server', 'server-status/nginx-server/index', 'menu', 'Nginx Server', 'material-symbols:monitor', 1, 'admin', 2, 1, strftime('%s','now'), strftime('%s','now'));

    -- 脚本/自动化的两个页面如今挂在「系统设置」下（见 sync_added_menus 的迁移说明）。
    -- id=10 这个顶层目录保留是为了不占掉已分配的 id，也为了让老版本的 role_menus 行不变成孤儿；
    -- hidden=1 让它不出现在侧栏，页面 /system/scripts · /system/cron 才是入口。
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, hidden, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (10, 0, 'automation', '/automation', 'Layout', '/automation/scripts', 'dir', '脚本/自动化', 'material-symbols:timer', 1, 1, 'admin', 11, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (101, 2, 'appstore-scripts', 'scripts', 'automation/scripts/index', 'menu', '自定义脚本', 'material-symbols:description', 1, 'admin', 10, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (102, 2, 'script-cron', 'cron', 'automation/cron/index', 'menu', '计划任务', 'material-symbols:alarm', 1, 'admin', 11, 1, strftime('%s','now'), strftime('%s','now'));

    -- Dev（Layout + 子菜单，位于最下方，admin/user/reseller）
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (12, 0, 'dev', '/dev', 'Layout', '/dev/api-tokens', 'dir', '开发', 'material-symbols:build', 1, 'admin,user,reseller', 13, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (121, 12, 'api-tokens', 'api-tokens', 'dev/api-tokens/index', 'menu', 'API Tokens', 'material-symbols:key', 1, 'admin,user,reseller', 1, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (122, 12, 'api-docs', 'api-docs', 'dev/api-docs/index', 'menu', 'API 文档', 'material-symbols:description', 1, 'admin,user,reseller', 2, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (123, 12, 'app-script-guide', 'app-script-guide', 'dev/app-script-guide/index', 'menu', '应用脚本编写', 'material-symbols:menu-book', 1, 'admin,user,reseller', 3, 1, strftime('%s','now'), strftime('%s','now'));

    -- 计划任务（Layout + 子菜单，所有角色可用；执行身份由后端收敛，admin 可选执行用户）
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (15, 0, 'crontab', '/crontab', 'Layout', '/crontab/index', 'dir', '计划任务', 'material-symbols:schedule', 1, 'admin,user,reseller,demo', 4, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (151, 15, 'crontab-index', 'index', 'crontab/index', 'menu', '定时任务', 'material-symbols:alarm', 1, 'admin,user,reseller,demo', 1, 1, strftime('%s','now'), strftime('%s','now'));

    -- 文档（Layout + 子菜单：更新日志 / 用户手册 / FAQ / 升级指南）
    -- 顺序排在最末：放在 sort_order=14，避免挤掉系统设置/开发等更常用的入口
    INSERT INTO menus (id, parent_id, name, path, component, redirect, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (16, 0, 'docs', '/docs', 'Layout', '/docs/index', 'dir', '文档', 'material-symbols:menu-book', 1, 'admin,user,reseller,demo', 14, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (161, 16, 'docs-changelog', 'changelog', 'docs/doc', 'menu', '更新日志', 'material-symbols:history', 0, 'admin,user,reseller,demo', 1, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (162, 16, 'docs-manual', 'manual', 'docs/doc', 'menu', '用户手册', 'material-symbols:book', 0, 'admin,user,reseller,demo', 2, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (163, 16, 'docs-faq', 'faq', 'docs/doc', 'menu', 'FAQ', 'material-symbols:help', 0, 'admin,user,reseller,demo', 3, 1, strftime('%s','now'), strftime('%s','now'));
    INSERT INTO menus (id, parent_id, name, path, component, type, title, icon, affix, roles, sort_order, status, created_at, updated_at)
    VALUES (164, 16, 'docs-upgrade', 'upgrade', 'docs/doc', 'menu', '升级指南', 'material-symbols:upgrade', 0, 'admin,user,reseller,demo', 4, 1, strftime('%s','now'), strftime('%s','now'));
    "#;
    let _ = get_db_pool().await.execute(sql).await;
}

// ── role_menus ─────────────────────────────────────────────

async fn init_role_menus_table() {
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
    -- Admin gets all menu IDs
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 1);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 2);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 21);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 22);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 23);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 25);
    -- User gets dashboard only
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 1);
    -- File manager: admin gets all, user gets read access
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 3);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 3);
    -- Terminal: both admin and user
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 4);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 4);
    -- Terminal / File manager 子菜单授权（admin/user/reseller/demo）
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 41);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 41);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 41);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 41);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 31);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 31);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 31);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 31);
    -- Reseller: same base permissions as user + customer management
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 1);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 3);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 4);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 5);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 51);
    -- 套餐（Packages）：admin 与 reseller 均可使用
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 52);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 52);
    -- AppStore: admin / user / reseller 均可访问
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 6);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 61);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 6);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 61);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 6);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 61);
    -- Demo: dashboard, files, terminal, appstore（与普通用户一致）
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 1);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 3);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 4);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 6);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 61);
    -- Server status: admin 专属（服务器信息 tabs 81 + Nginx Server 87）
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 8);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 81);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 87);
    -- Server config: admin 专属（服务配置已并入其中，见菜单 82）
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 7);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 71);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 72);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 82);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 73);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 74);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 75);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 76);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 80);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 77);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 78);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 79);
    -- 站点管理：admin 全部 / user 自己的站点 / reseller 所属客户的站点
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 9);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 9);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 9);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 91);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 91);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 91);
    -- 数据库管理：admin 全部 / user 仅自己前缀的库
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 14);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 14);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 141);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 141);
    -- SSL/TLS：admin / user
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 11);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 111);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 112);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 11);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 111);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 112);
    -- 已安装应用：admin / user / reseller
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 62);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 62);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 62);
    -- 基础设置：仅 admin
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 26);
    -- Zap 设置：仅 admin
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 28);
    -- 审计日志：仅 admin
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 24);
    -- 系统更新：仅 admin
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 27);
    -- 脚本/自动化（自定义脚本 + 计划任务）：仅 admin
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 10);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 101);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 102);
    -- 开发：admin / user / reseller
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 12);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 121);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 122);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 123);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 12);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 121);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 122);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 123);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 12);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 121);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 122);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 123);
    -- 计划任务（每用户管理自己的定时任务）：admin / user / reseller / demo 均可见
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 15);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 151);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 15);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 151);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 15);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 151);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 15);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 151);
    -- 文档（CHANGELOG / 用户手册 / FAQ / 升级指南）：admin / user / reseller / demo 均可见
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 16);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 161);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 162);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 163);
    INSERT INTO role_menus (role_id, menu_id) VALUES (1, 164);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 16);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 161);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 162);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 163);
    INSERT INTO role_menus (role_id, menu_id) VALUES (2, 164);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 16);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 161);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 162);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 163);
    INSERT INTO role_menus (role_id, menu_id) VALUES (3, 164);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 16);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 161);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 162);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 163);
    INSERT INTO role_menus (role_id, menu_id) VALUES (4, 164);
    "#;
    let _ = get_db_pool().await.execute(sql).await;
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
        -- kind：任务大类（appstore / docker / backup / system / cron / crontab / site），
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
