//! 菜单种子数据（结构化）：侧栏入口的唯一真源。
//!
//! 以前是一整段 `INSERT INTO menus (...) VALUES (...)` 的 SQL，三个痛点：
//!
//! - id 要手写：加一个菜单得先想「用哪个号」，父子靠数字 id 关联，看不出结构；
//! - 状态分散：真正的最终形态（hidden / status / feature / 父级）散落在
//!   `sync_added_menus()` 的 UPDATE 里，种子里写的是历史形态；
//! - 授权另写一套：`role_menus` 又抄了一遍 id 列表，两者极易对不上。
//!
//! 现在改成一张声明式清单：
//!
//! - `name` 是稳定逻辑键（同时是前端路由名，表上有 UNIQUE），父子用
//!   `parent("父 name")` 关联，不再出现数字；
//! - **id 由 SQLite 自增**：声明顺序即插入顺序（父必须先于子），所以加菜单
//!   不用再挑号，也不用管历史库里哪些号被占了；
//! - `roles` 逗号分隔，既写进 `menus.roles`（标注），也用来生成 `role_menus`
//!   （真正的可见性），一份数据两处用，不会对不上；
//! - 加菜单 = 往 [`MENU_SEEDS`] 里加一行，`sync_added_menus()` 不用再动。
//!
//! 只作用于**新建库**：已存在的库保留原样（菜单可由管理员在「菜单管理」里改），
//! 这正是我们想要的 —— 种子是初始状态，不是每启动一次就覆盖用户改过的菜单。

use sqlx::SqlitePool;
use std::collections::HashMap;

// 角色组合常量：给菜单标注可见范围用（`menus.roles` + `role_menus` 同源）
const R_ALL: &str = "admin,user,reseller,demo";
pub(crate) const R_ADMIN: &str = "admin";
const R_ADMIN_USER: &str = "admin,user";
const R_ADMIN_RESELLER: &str = "admin,reseller";
const R_ADMIN_USER_RESELLER: &str = "admin,user,reseller";
const R_RESELLER: &str = "reseller";

/// 一条菜单种子。
///
/// 只写「有信息量」的字段，其余取默认值（`redirect` 空、`hidden=false`、
/// `affix=false`、`feature=""`、`status=1`、`parent` 顶层）。
pub struct MenuSeed {
    /// 稳定逻辑键 = 前端路由名，全表唯一
    pub name: &'static str,
    pub title: &'static str,
    pub parent: Option<&'static str>,
    /// 路由路径：顶层写 `/xxx`，子菜单写相对父级的段
    pub path: &'static str,
    pub component: &'static str,
    pub redirect: &'static str,
    /// `dir` = 目录（有子菜单）；`menu` = 具体页面
    pub kind: &'static str,
    pub icon: &'static str,
    /// 不进侧栏但行保留（老入口 / 已合并页签的旧项）
    pub hidden: bool,
    /// 标签页固定（`affix`）
    pub affix: bool,
    /// 环境能力门禁：空串 = 常显；`docker` = 后台装了 Docker 才下发
    pub feature: &'static str,
    /// 可见角色（逗号分隔）；同时决定 `role_menus`
    pub roles: &'static str,
    pub sort_order: i32,
    /// 0 = 停用：不进侧栏，行保留以兼容既有授权记录
    pub status: i32,
}

impl MenuSeed {
    pub(crate) const fn new(
        name: &'static str,
        title: &'static str,
        kind: &'static str,
        path: &'static str,
        component: &'static str,
        roles: &'static str,
        sort_order: i32,
    ) -> Self {
        Self {
            name,
            title,
            parent: None,
            path,
            component,
            redirect: "",
            kind,
            icon: "",
            hidden: false,
            affix: false,
            feature: "",
            roles,
            sort_order,
            status: 1,
        }
    }

    pub(crate) const fn parent(mut self, name: &'static str) -> Self {
        self.parent = Some(name);
        self
    }
    pub(crate) const fn redirect(mut self, redirect: &'static str) -> Self {
        self.redirect = redirect;
        self
    }
    pub(crate) const fn icon(mut self, icon: &'static str) -> Self {
        self.icon = icon;
        self
    }
    pub(crate) const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }
    pub(crate) const fn affix(mut self) -> Self {
        self.affix = true;
        self
    }
    pub(crate) const fn feature(mut self, feature: &'static str) -> Self {
        self.feature = feature;
        self
    }
    pub(crate) const fn disabled(mut self) -> Self {
        self.status = 0;
        self
    }
}

/// 商业模块（Zap Pro）追加的菜单种子。
///
/// 未启用 `commercial` 时返回空表 —— 播种循环只写一处，不用到处 `#[cfg]`。
pub(crate) fn pro_seeds() -> &'static [MenuSeed] {
    #[cfg(feature = "commercial")]
    {
        crate::pro::menu::SEEDS
    }
    #[cfg(not(feature = "commercial"))]
    {
        &[]
    }
}

/// 全部种子：内置清单 + 商业模块追加的部分。
fn all_seeds() -> impl Iterator<Item = &'static MenuSeed> {
    MENU_SEEDS.iter().chain(pro_seeds())
}

/// 菜单清单：声明顺序 = 插入顺序 = id 自增顺序；父菜单必须先声明。
///
/// 分组与侧栏顺序一致（组间空行 = 一级入口，缩进的子项挂在其上）。
pub static MENU_SEEDS: &[MenuSeed] = &[
    // ── 仪表盘 ──────────────────────────────────────────────
    MenuSeed::new(
        "dashboard",
        "仪表盘",
        "menu",
        "/dashboard",
        "dashboard/index",
        R_ALL,
        1,
    )
    .icon("material-symbols:home")
    .affix(),
    // ── 站点 ────────────────────────────────────────────────
    MenuSeed::new(
        "site",
        "站点",
        "menu",
        "/site",
        "Layout",
        R_ADMIN_USER_RESELLER,
        2,
    )
    .icon("material-symbols:public")
    .redirect("/site/index")
    .affix(),
    MenuSeed::new(
        "site-index",
        "站点",
        "menu",
        "index",
        "site/index",
        R_ADMIN_USER_RESELLER,
        1,
    )
    .parent("site")
    .icon("material-symbols:public")
    .affix(),
    // ── 数据库 ──────────────────────────────────────────────
    MenuSeed::new(
        "database",
        "数据库",
        "menu",
        "/database",
        "Layout",
        R_ADMIN_USER,
        3,
    )
    .icon("material-symbols:database")
    .redirect("/database/index")
    .affix(),
    MenuSeed::new(
        "database-index",
        "数据库",
        "menu",
        "index",
        "database/index",
        R_ADMIN_USER,
        1,
    )
    .parent("database")
    .icon("material-symbols:database")
    .affix(),
    // ── 系统设置（目录）──────────────────────────────────────
    MenuSeed::new("system", "系统设置", "dir", "/system", "Layout", R_ALL, 12)
        .icon("material-symbols:settings")
        .redirect("/system/access")
        .affix(),
    // 基础设置（旧 `basic-config`）已下线：Mail 并入 Zap 设置的「通知设置」页签，
    // 建站默认网络与联系信息不再提供界面入口（键值仍留在 server_env.yaml）。
    // 存量库里的这条菜单由 `init_db::sync_added_menus()` 停用。
    MenuSeed::new(
        "zap-config",
        "Zap 设置",
        "menu",
        "zap-config",
        "system/config/zap",
        R_ADMIN,
        1,
    )
    .parent("system")
    .icon("material-symbols:settings-applications")
    .affix(),
    // 用户管理 + 角色管理合到一页（页面内 nav pill 切换）
    MenuSeed::new(
        "access",
        "用户与角色",
        "menu",
        "access",
        "system/access/index",
        R_ADMIN,
        3,
    )
    .parent("system")
    .icon("material-symbols:badge")
    .affix(),
    // 角色管理已并入 access；保留本行只为兼容既有 role_menus 授权
    MenuSeed::new(
        "roles",
        "角色管理",
        "menu",
        "roles",
        "system/access/index",
        R_ADMIN,
        4,
    )
    .parent("system")
    .icon("material-symbols:visibility")
    .affix()
    .disabled(),
    MenuSeed::new(
        "menus",
        "菜单管理",
        "menu",
        "menus",
        "system/menus/index",
        R_ADMIN,
        5,
    )
    .parent("system")
    .icon("material-symbols:menu")
    .affix(),
    MenuSeed::new(
        "tasks",
        "任务队列",
        "menu",
        "tasks",
        "system/tasks/index",
        R_ADMIN,
        6,
    )
    .parent("system")
    .icon("material-symbols:view-list"),
    MenuSeed::new(
        "audit",
        "审计日志",
        "menu",
        "audit",
        "system/audit/index",
        R_ADMIN,
        7,
    )
    .parent("system")
    .icon("material-symbols:confirmation-number")
    .affix(),
    // 系统更新已并入 about 的第二个 nav pill；保留本行只为兼容既有授权
    MenuSeed::new(
        "system-update",
        "系统更新",
        "menu",
        "update",
        "system/about/index",
        R_ADMIN,
        8,
    )
    .parent("system")
    .icon("material-symbols:refresh")
    .affix()
    .disabled(),
    MenuSeed::new(
        "about",
        "About ZAP",
        "menu",
        "about",
        "system/about/index",
        R_ALL,
        9,
    )
    .parent("system")
    .icon("material-symbols:info"),
    // 自动化脚本：自定义脚本 + 计划任务 合到一页（页面内 nav pill 切换）
    MenuSeed::new(
        "automation-scripts",
        "自动化脚本",
        "menu",
        "automation",
        "automation/index",
        R_ADMIN,
        10,
    )
    .parent("system")
    .icon("material-symbols:timer")
    .affix(),
    // 计划任务已并入 automation-scripts；保留本行只为兼容既有授权
    MenuSeed::new(
        "script-cron",
        "计划任务",
        "menu",
        "cron",
        "automation/index",
        R_ADMIN,
        11,
    )
    .parent("system")
    .icon("material-symbols:alarm")
    .affix()
    .disabled(),
    // ── 服务器配置（目录）────────────────────────────────────
    MenuSeed::new(
        "server",
        "服务器配置",
        "dir",
        "/server",
        "Layout",
        R_ADMIN,
        9,
    )
    .icon("material-symbols:tune")
    .redirect("/server/system")
    .affix(),
    // 系统管理：服务器时间 / 系统服务 / SSH 服务 / 进程管理 合到一页
    MenuSeed::new(
        "server-system",
        "系统管理",
        "menu",
        "system",
        "server/system/index",
        R_ADMIN,
        1,
    )
    .parent("server")
    .icon("material-symbols:settings")
    .affix(),
    // 系统服务已并入 server-system
    MenuSeed::new(
        "server-services",
        "系统服务",
        "menu",
        "services",
        "server/system/index",
        R_ADMIN,
        2,
    )
    .parent("server")
    .icon("material-symbols:build")
    .affix()
    .disabled(),
    // 服务配置：Nginx / PHP / MySQL 配置合到一页
    MenuSeed::new(
        "server-service-conf",
        "服务配置",
        "menu",
        "service-conf",
        "server/service-conf/index",
        R_ADMIN,
        3,
    )
    .parent("server")
    .icon("material-symbols:dns")
    .affix(),
    // SSH 服务已并入 server-system
    MenuSeed::new(
        "server-ssh",
        "SSH 服务",
        "menu",
        "ssh",
        "server/system/index",
        R_ADMIN,
        4,
    )
    .parent("server")
    .icon("material-symbols:cable")
    .affix()
    .disabled(),
    // 进程管理已并入 server-system
    MenuSeed::new(
        "server-process",
        "进程管理",
        "menu",
        "process",
        "server/system/index",
        R_ADMIN,
        5,
    )
    .parent("server")
    .icon("material-symbols:memory")
    .affix()
    .disabled(),
    // 网络配置：网络设置 + IP 设置 合到一页
    MenuSeed::new(
        "server-network",
        "网络配置",
        "menu",
        "network",
        "server/network/index",
        R_ADMIN,
        6,
    )
    .parent("server")
    .icon("material-symbols:link")
    .affix(),
    // IP 设置已并入 server-network
    MenuSeed::new(
        "server-ip",
        "IP 设置",
        "menu",
        "ip",
        "server/network/index",
        R_ADMIN,
        7,
    )
    .parent("server")
    .icon("material-symbols:badge")
    .affix()
    .disabled(),
    MenuSeed::new(
        "server-firewall",
        "防火墙",
        "menu",
        "firewall",
        "server/firewall/index",
        R_ADMIN,
        8,
    )
    .parent("server")
    .icon("material-symbols:lock")
    .affix(),
    MenuSeed::new(
        "server-env",
        "运行环境",
        "menu",
        "env",
        "server/env/index",
        R_ADMIN,
        9,
    )
    .parent("server")
    .icon("material-symbols:auto-fix-high")
    .affix(),
    // 同步运行环境已并入 server-env（第二个 nav pill）
    MenuSeed::new(
        "server-entities",
        "同步运行环境",
        "menu",
        "entities",
        "server/env/index",
        R_ADMIN,
        10,
    )
    .parent("server")
    .icon("material-symbols:account-circle")
    .affix()
    .disabled(),
    MenuSeed::new(
        "server-migrate",
        "数据迁移",
        "menu",
        "migrate",
        "server/migrate/index",
        R_ADMIN,
        11,
    )
    .parent("server")
    .icon("material-symbols:sort")
    .affix(),
    // ── 终端 ────────────────────────────────────────────────
    MenuSeed::new("terminal", "终端", "menu", "/terminal", "Layout", R_ALL, 4)
        .icon("material-symbols:monitor")
        .redirect("/terminal/index")
        .affix(),
    MenuSeed::new(
        "terminal-index",
        "终端",
        "menu",
        "index",
        "terminal/index",
        R_ALL,
        1,
    )
    .parent("terminal")
    .icon("material-symbols:monitor")
    .affix(),
    // ── 文件管理 ────────────────────────────────────────────
    MenuSeed::new("files", "文件管理", "menu", "/files", "Layout", R_ALL, 3)
        .icon("material-symbols:folder")
        .redirect("/files/index")
        .affix(),
    MenuSeed::new(
        "files-index",
        "文件管理",
        "menu",
        "index",
        "files/index",
        R_ALL,
        1,
    )
    .parent("files")
    .icon("material-symbols:folder")
    .affix(),
    // ── 客户管理（reseller 专属）─────────────────────────────
    MenuSeed::new(
        "reseller-users",
        "客户管理",
        "menu",
        "/reseller/users",
        "Layout",
        R_RESELLER,
        5,
    )
    .icon("material-symbols:account-circle")
    .redirect("/reseller/users/index")
    .affix(),
    MenuSeed::new(
        "reseller-users-index",
        "客户管理",
        "menu",
        "index",
        "system/access/index",
        R_RESELLER,
        1,
    )
    .parent("reseller-users")
    .icon("material-symbols:account-circle")
    .affix(),
    MenuSeed::new(
        "reseller-packages",
        "套餐",
        "menu",
        "packages",
        "system/packages/index",
        R_ADMIN_RESELLER,
        2,
    )
    .parent("reseller-users")
    .icon("material-symbols:storefront"),
    // ── SSL/TLS（目录）──────────────────────────────────────
    MenuSeed::new(
        "ssl-tls",
        "SSL/TLS",
        "dir",
        "/ssl-tls",
        "Layout",
        R_ADMIN_USER,
        6,
    )
    .icon("material-symbols:lock")
    .redirect("/ssl-tls/certs")
    .affix(),
    MenuSeed::new(
        "ssl-certs",
        "SSL证书",
        "menu",
        "certs",
        "ssl-tls/certs/index",
        R_ADMIN_USER,
        1,
    )
    .parent("ssl-tls")
    .icon("material-symbols:lock")
    .affix(),
    // DNS 服务商收进证书页头部按钮 + 抽屉，入口隐藏（页面路由仍可达）
    MenuSeed::new(
        "ssl-dns-providers",
        "DNS服务商",
        "menu",
        "dns-providers",
        "ssl-tls/dns-providers/index",
        R_ADMIN_USER,
        2,
    )
    .parent("ssl-tls")
    .icon("material-symbols:dns")
    .hidden(),
    // ── 应用商店 ────────────────────────────────────────────
    MenuSeed::new(
        "appstore",
        "应用商店",
        "menu",
        "/appstore",
        "Layout",
        R_ALL,
        7,
    )
    .icon("material-symbols:storefront")
    .redirect("/appstore/index")
    .affix(),
    MenuSeed::new(
        "appstore-index",
        "应用商店",
        "menu",
        "index",
        "appstore/index",
        R_ALL,
        1,
    )
    .parent("appstore")
    .icon("material-symbols:storefront")
    .affix(),
    // 已安装应用改成应用商店页内的 nav pill，入口隐藏（路由仍可达）
    MenuSeed::new(
        "installed",
        "已安装应用",
        "menu",
        "installed",
        "appstore/installed",
        R_ADMIN_USER_RESELLER,
        2,
    )
    .parent("appstore")
    .icon("material-symbols:deployed-code")
    .affix()
    .hidden(),
    // ── 服务器状态（目录）────────────────────────────────────
    MenuSeed::new(
        "server-status",
        "服务器状态",
        "dir",
        "/server-status",
        "Layout",
        R_ADMIN,
        8,
    )
    .icon("material-symbols:monitor-heart")
    .redirect("/server-status/index")
    .affix(),
    MenuSeed::new(
        "server-status-index",
        "Server Monitor",
        "menu",
        "index",
        "server-status/index",
        R_ADMIN,
        1,
    )
    .parent("server-status")
    .icon("material-symbols:monitoring")
    .affix(),
    MenuSeed::new(
        "server-status-nginx",
        "Nginx Server",
        "menu",
        "nginx-server",
        "server-status/nginx-server/index",
        R_ADMIN,
        2,
    )
    .parent("server-status")
    .icon("material-symbols:monitor")
    .affix(),
    // ── 开发（目录）─────────────────────────────────────────
    MenuSeed::new(
        "dev",
        "开发",
        "dir",
        "/dev",
        "Layout",
        R_ADMIN_USER_RESELLER,
        13,
    )
    .icon("material-symbols:build")
    .redirect("/dev/api-tokens")
    .affix(),
    MenuSeed::new(
        "api-tokens",
        "API Tokens",
        "menu",
        "api-tokens",
        "dev/api-tokens/index",
        R_ADMIN_USER_RESELLER,
        1,
    )
    .parent("dev")
    .icon("material-symbols:key")
    .affix(),
    MenuSeed::new(
        "api-docs",
        "API 文档",
        "menu",
        "api-docs",
        "dev/api-docs/index",
        R_ADMIN_USER_RESELLER,
        2,
    )
    .parent("dev")
    .icon("material-symbols:description")
    .affix(),
    MenuSeed::new(
        "app-script-guide",
        "应用脚本编写",
        "menu",
        "app-script-guide",
        "dev/app-script-guide/index",
        R_ADMIN_USER_RESELLER,
        3,
    )
    .parent("dev")
    .icon("material-symbols:menu-book")
    .affix(),
    // ── 计划任务（目录）──────────────────────────────────────
    MenuSeed::new("crontab", "计划任务", "dir", "/crontab", "Layout", R_ALL, 4)
        .icon("material-symbols:schedule")
        .redirect("/crontab/index")
        .affix(),
    MenuSeed::new(
        "crontab-index",
        "定时任务",
        "menu",
        "index",
        "crontab/index",
        R_ALL,
        1,
    )
    .parent("crontab")
    .icon("material-symbols:alarm")
    .affix(),
    // ── 容器管理（Docker）：没装 Docker 时不下发 ──────────────
    MenuSeed::new("docker", "容器管理", "dir", "/docker", "Layout", R_ADMIN, 5)
        .icon("material-symbols:deployed-code")
        .redirect("/docker/index")
        .feature("docker"),
    MenuSeed::new(
        "docker-index",
        "容器",
        "menu",
        "index",
        "docker/index",
        R_ADMIN,
        1,
    )
    .parent("docker")
    .icon("material-symbols:deployed-code")
    .feature("docker"),
    // ── 团队成员：入口已并入「个人中心」，整组隐藏 ──────────────
    MenuSeed::new(
        "team",
        "团队成员",
        "dir",
        "/team",
        "Layout",
        R_ADMIN_USER_RESELLER,
        9,
    )
    .icon("material-symbols:group")
    .redirect("/team/index")
    .hidden(),
    MenuSeed::new(
        "team-index",
        "团队成员",
        "menu",
        "index",
        "team/index",
        R_ADMIN_USER_RESELLER,
        1,
    )
    .parent("team")
    .icon("material-symbols:group")
    .affix()
    .hidden(),
    // ── 文档：已整合进 About ZAP，整组隐藏（路由仍可达）────────
    MenuSeed::new("docs", "文档", "dir", "/docs", "Layout", R_ALL, 14)
        .icon("material-symbols:menu-book")
        .redirect("/docs/index")
        .affix()
        .hidden(),
    MenuSeed::new(
        "docs-changelog",
        "更新日志",
        "menu",
        "changelog",
        "docs/doc",
        R_ALL,
        1,
    )
    .parent("docs")
    .icon("material-symbols:history")
    .hidden(),
    MenuSeed::new(
        "docs-manual",
        "用户手册",
        "menu",
        "manual",
        "docs/doc",
        R_ALL,
        2,
    )
    .parent("docs")
    .icon("material-symbols:book")
    .hidden(),
    MenuSeed::new("docs-faq", "FAQ", "menu", "faq", "docs/doc", R_ALL, 3)
        .parent("docs")
        .icon("material-symbols:help")
        .hidden(),
    MenuSeed::new(
        "docs-upgrade",
        "升级指南",
        "menu",
        "upgrade",
        "docs/doc",
        R_ALL,
        4,
    )
    .parent("docs")
    .icon("material-symbols:upgrade")
    .hidden(),
    // ── 旧「脚本/自动化」目录：两个页面已挂到「系统设置」下 ──────
    // 目录本身隐藏，保留是为了不占掉已分配的 id 段（历史库里的 role_menus 行
    // 还指向它）。新库里它只是个空壳入口，可随时删。
    MenuSeed::new(
        "automation",
        "脚本/自动化",
        "dir",
        "/automation",
        "Layout",
        R_ADMIN,
        11,
    )
    .icon("material-symbols:timer")
    .redirect("/system/automation")
    .affix()
    .hidden(),
];

/// 按声明顺序写入 `menus`（id 由 SQLite 自增），返回 `name → id` 映射。
///
/// 父菜单必须在子菜单之前声明，否则子项的 `parent_id` 落不成（记 0 并告警）。
pub async fn seed_menus(pool: &SqlitePool) -> HashMap<String, i64> {
    let mut ids: HashMap<String, i64> = HashMap::new();
    for seed in all_seeds() {
        let parent_id = match seed.parent {
            Some(p) => match ids.get(p) {
                Some(id) => *id,
                None => {
                    eprintln!(
                        "菜单种子 {}: 父菜单 {p} 未声明（顺序不对？），按顶层处理",
                        seed.name
                    );
                    0
                }
            },
            None => 0,
        };
        let inserted = sqlx::query(
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
        match inserted {
            Ok(res) => {
                ids.insert(seed.name.to_string(), res.last_insert_rowid());
            }
            Err(e) => eprintln!("菜单种子 {} 写入失败: {e}", seed.name),
        }
    }
    ids
}

/// 按种子的 `roles` 生成 `role_menus`：角色按 role_key 查 id，菜单按 name 查 id。
///
/// `menu_ids` 为空（表已存在、不是本次建的）时回落到库里查一遍 name → id，
/// 这样「只重建 role_menus」的场景也能正确授权。
pub async fn seed_role_menus(pool: &SqlitePool, menu_ids: &HashMap<String, i64>) {
    let fallback;
    let ids: &HashMap<String, i64> = if menu_ids.is_empty() {
        fallback = sqlx::query_as::<_, (String, i64)>("SELECT name, id FROM menus")
            .fetch_all(pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        &fallback
    } else {
        menu_ids
    };

    for seed in all_seeds() {
        let Some(menu_id) = ids.get(seed.name) else {
            continue;
        };
        for role in seed
            .roles
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO role_menus (role_id, menu_id)
                 SELECT r.id, ? FROM roles r WHERE r.role_key = ?",
            )
            .bind(*menu_id)
            .bind(role)
            .execute(pool)
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// 种子清单自身的约束：name 唯一、父菜单先声明、roles 非空。
    #[test]
    fn seeds_are_well_formed() {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (i, seed) in MENU_SEEDS.iter().enumerate() {
            assert!(!seed.name.is_empty(), "第 {i} 条种子缺 name");
            assert!(!seed.title.is_empty(), "{} 缺 title", seed.name);
            assert!(!seed.path.is_empty(), "{} 缺 path", seed.name);
            assert!(
                !seed.roles.is_empty(),
                "{} 缺 roles（决定谁能看到它）",
                seed.name
            );
            assert!(
                seed.kind == "dir" || seed.kind == "menu",
                "{} 的 kind 只能是 dir / menu",
                seed.name
            );
            let dup = seen.insert(seed.name, i);
            assert!(dup.is_none(), "菜单 name 重复：{}", seed.name);
        }
        // 父菜单必须先声明，否则 parent_id 落不成
        for (i, seed) in MENU_SEEDS.iter().enumerate() {
            if let Some(parent) = seed.parent {
                let pos = seen.get(parent).copied().unwrap_or(usize::MAX);
                assert!(pos < i, "{} 的父菜单 {parent} 必须在它之前声明", seed.name);
            }
        }
    }

    /// 真跑一遍建库：id 自增、父子关联、role_menus 按 roles 生成。
    #[tokio::test]
    async fn seeds_insert_with_auto_ids_and_grants() {
        // 内存库：max/min 都锁成 1 条连接，保证整段测试始终在同一个库上
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .idle_timeout(None)
            .connect("sqlite::memory:")
            .await
            .expect("内存库连接失败");
        for ddl in [
            "CREATE TABLE menus (id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
                parent_id INTEGER DEFAULT 0, name VARCHAR(64) NOT NULL UNIQUE,
                path VARCHAR(128) NOT NULL DEFAULT '', component VARCHAR(256) DEFAULT '',
                redirect VARCHAR(128) DEFAULT '', type VARCHAR(16) NOT NULL DEFAULT 'menu',
                title VARCHAR(64) NOT NULL DEFAULT '', icon VARCHAR(64) DEFAULT '',
                hidden INTEGER DEFAULT 0, affix INTEGER DEFAULT 0, feature TEXT NOT NULL DEFAULT '',
                roles TEXT DEFAULT '', sort_order INTEGER DEFAULT 0, status INTEGER DEFAULT 1,
                created_at INTEGER, updated_at INTEGER)",
            "CREATE TABLE roles (id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT, role_key VARCHAR(64) UNIQUE)",
            "CREATE TABLE role_menus (id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT, role_id INTEGER, menu_id INTEGER, UNIQUE(role_id, menu_id))",
            "INSERT INTO roles (role_key) VALUES ('admin'),('user'),('reseller'),('demo')",
        ] {
            sqlx::query(ddl).execute(&pool).await.expect("建表失败");
        }

        let ids = seed_menus(&pool).await;
        assert_eq!(ids.len(), all_seeds().count(), "每条种子都应写入并拿到 id");
        // id 自增：不重复且连续可查（不校验具体值，避免写死）
        let mut id_list: Vec<i64> = ids.values().copied().collect();
        id_list.sort_unstable();
        id_list.dedup();
        assert_eq!(id_list.len(), all_seeds().count(), "id 必须互不相同");

        // 父子关联
        for seed in all_seeds() {
            if let Some(parent) = seed.parent {
                let (parent_id,): (i64,) =
                    sqlx::query_as("SELECT parent_id FROM menus WHERE name = ?")
                        .bind(seed.name)
                        .fetch_one(&pool)
                        .await
                        .unwrap();
                assert_eq!(parent_id, ids[parent], "{} 的父级没挂上", seed.name);
            }
        }

        seed_role_menus(&pool, &ids).await;
        async fn grants(pool: &SqlitePool, role: &str) -> Vec<String> {
            sqlx::query_scalar::<_, String>(
                "SELECT m.name FROM role_menus rm
                 JOIN menus m ON m.id = rm.menu_id
                 JOIN roles r ON r.id = rm.role_id
                 WHERE r.role_key = ? ORDER BY m.name",
            )
            .bind(role)
            .fetch_all(pool)
            .await
            .unwrap()
        }
        let admin = grants(&pool, "admin").await;
        // 管理员拿到除「客户管理」（reseller 专属）以外的全部入口
        let expected: Vec<String> = all_seeds()
            .map(|s| s.name.to_string())
            .filter(|n| n != "reseller-users" && n != "reseller-users-index")
            .collect();
        assert_eq!(
            admin.iter().collect::<std::collections::BTreeSet<_>>(),
            expected.iter().collect::<std::collections::BTreeSet<_>>(),
            "管理员可见菜单与种子不一致"
        );
        // demo 是最小集合：仪表盘 / 终端 / 文件 / 应用商店 / 计划任务 / 文档 / About ZAP
        let demo = grants(&pool, "demo").await;
        for must in [
            "dashboard",
            "terminal-index",
            "files-index",
            "appstore-index",
            "crontab-index",
            "docs-faq",
            "about",
        ] {
            assert!(demo.iter().any(|n| n == must), "demo 应能看到 {must}");
        }
        for must_not in [
            "access",
            "tasks",
            "automation-scripts",
            "site-index",
            "api-tokens",
            "team-index",
        ] {
            assert!(
                !demo.iter().any(|n| n == must_not),
                "demo 不该看到 {must_not}"
            );
        }
        // 与「旧库最终态」对齐的可见条数（防止重构悄悄改了可见范围）
        assert_eq!(demo.len(), 16, "demo 可见菜单数变化");
        let reseller = grants(&pool, "reseller").await;
        assert_eq!(reseller.len(), 28, "reseller 可见菜单数变化");
        // reseller 专属：客户管理只给它自己
        assert!(reseller.iter().any(|n| n == "reseller-users"));
        assert!(!admin.iter().any(|n| n == "reseller-users"));

        // 自动化脚本是 admin 专属
        let user = grants(&pool, "user").await;
        assert_eq!(user.len(), 30, "user 可见菜单数变化");
        assert!(
            !user.iter().any(|n| n == "automation-scripts"),
            "自动化脚本必须仅 admin 可见"
        );
    }
}
