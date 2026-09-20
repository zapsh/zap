//! 路由 → 角色 权限矩阵中间件（fail-closed）。
//!
//!
//! 这里把「路径前缀 → 所需角色」收敛成一张表，由中间件统一执行：
//!
//! - **默认拒绝（fail-closed）**：未显式登记的路径一律要求 admin。
//!   新增接口忘记登记时默认不可被普通用户访问，而不是默认放行。
//! - 表中只登记「放宽」的条目（Public / User / Reseller）。
//! - 资源归属（owner）维度仍由 handler 自行收敛：中间件只回答"你有没有资格敲这扇门"，
//!   "你能看到哪几条数据"依旧是 handler 的职责（如站点、证书、文件、SSH 连接）。
//!
//! ## 两层校验
//!
//! 1. **角色下限**（本表 `Required`）：接口的硬门槛，代码内固定，不可配置。
//! 2. **动作级权限点**（本表第三列 ns + `role_permissions` 表）：可运营配置。
//!    请求按方法展开成 `{ns}:view`（GET/HEAD）或 `{ns}:edit`（其余），
//!    用户所属任一角色在 `role_permissions` 中持有该 key 才放行；admin 恒直通。
//!
//! 前端 `v-permission` / 菜单树只是体验层，**不是安全边界**，安全边界在这里。
//!
//! 匹配方式：先剥掉 URL 前缀与 `/api`，再取**最长前缀**命中项。

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock, RwLock},
};

use axum::{
    Json,
    extract::Request,
    http::{Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;
use tracing::warn;

use crate::zap::jwt::{self, Claims};

/// 访问某接口所需的最低角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Required {
    /// 免鉴权：健康检查、登录
    Public,
    /// 任意已登录用户（含 demo）；数据范围由 handler 按 owner 收敛
    User,
    /// admin 或 reseller
    Reseller,
    /// 仅 admin
    Admin,
}

impl Required {
    fn label(self) -> &'static str {
        match self {
            Required::Public => "public",
            Required::User => "user",
            Required::Reseller => "reseller",
            Required::Admin => "admin",
        }
    }
}

/// 权限点声明：命名空间 + 动作。
///
/// - `Perm::module(ns)`：动作由 HTTP 方法派生（GET/HEAD → `view`，其余 → `edit`）；
/// - `Perm::action(ns, act)`：固定动作，忽略方法。用于
///   · 语义明确的写操作（`site:delete`、`ssl:create`）
///   · 用 GET 做状态变更的历史端点（`/system/job/start` → `system.job:edit`）
///   · POST 但只读的端点（`/ssl/cert/parse`、`/site/dirs` → `view`）
#[derive(Debug, Clone, Copy)]
pub struct Perm {
    pub ns: &'static str,
    pub action: Option<&'static str>,
}

impl Perm {
    const fn module(ns: &'static str) -> Self {
        Self { ns, action: None }
    }
    const fn action(ns: &'static str, action: &'static str) -> Self {
        Self {
            ns,
            action: Some(action),
        }
    }
}

/// 权限矩阵：`(路径前缀, 所需角色下限, 权限点声明)`。
///
/// - 第三列 `None` 表示只受角色下限约束（个人接口，无运营配置项）；
/// - admin-only 的条目即使与默认值相同也显式列出：它们同时是权限点的登记处，
///   漏登记等于该接口绕过权限点校验（仅剩角色下限）。
///
/// 拆到动作级后**不再给模块留兜底条目**（如没有裸 `/site` 规则）：
/// 新增接口必须显式登记，否则落到默认 `Admin` 下限（fail-closed）。
const RULES: &[(&str, Required, Option<Perm>)] = &[
    // ── 免鉴权 ───────────────────────────────────────────────
    ("/health", Required::Public, None),
    ("/auth/login", Required::Public, None),
    // ── 登录态即可访问的个人接口 ─────────────────────────────
    ("/auth", Required::User, None),
    ("/user", Required::User, None),
    // ── 仪表盘统计卡片：任意登录用户，数据范围由 handler 按 owner 收敛 ──
    ("/dashboard/counts", Required::User, None),
    // ── 文档（CHANGELOG / 用户手册 / FAQ / 升级指南）：所有已登录用户 ──
    ("/docs", Required::User, None),
    // ── 站点：view / create / update / delete / state / sync ──
    (
        "/site/list",
        Required::User,
        Some(Perm::action("site", "view")),
    ),
    (
        "/site/users",
        Required::Reseller,
        Some(Perm::action("site", "view")),
    ),
    (
        "/site/feature",
        Required::User,
        Some(Perm::action("site", "view")),
    ),
    // POST 但只做目录浏览，按读取授权
    (
        "/site/dirs",
        Required::User,
        Some(Perm::action("site", "view")),
    ),
    (
        "/site/add",
        Required::User,
        Some(Perm::action("site", "create")),
    ),
    (
        "/site/update",
        Required::User,
        Some(Perm::action("site", "update")),
    ),
    (
        "/site/delete",
        Required::User,
        Some(Perm::action("site", "delete")),
    ),
    (
        "/site/state",
        Required::User,
        Some(Perm::action("site", "state")),
    ),
    (
        "/site/sync",
        Required::User,
        Some(Perm::action("site", "sync")),
    ),
    (
        "/site/sync_all",
        Required::Reseller,
        Some(Perm::action("site", "sync")),
    ),
    // ── 站点日志与流量分析 ────────────────────────────────
    (
        "/site/logs",
        Required::User,
        Some(Perm::action("site", "view")),
    ),
    (
        "/site/logs/archives",
        Required::User,
        Some(Perm::action("site", "view")),
    ),
    (
        "/site/logs/clear",
        Required::User,
        Some(Perm::action("site", "update")),
    ),
    (
        "/site/logs/rotate",
        Required::User,
        Some(Perm::action("site", "update")),
    ),
    (
        "/site/traffic",
        Required::User,
        Some(Perm::action("site", "view")),
    ),
    // ── SSL 证书：view / create / update / delete ────────────
    (
        "/ssl/cert/list",
        Required::User,
        Some(Perm::action("ssl", "view")),
    ),
    (
        "/ssl/cert/detail",
        Required::User,
        Some(Perm::action("ssl", "view")),
    ),
    // POST 但只解析证书内容，按读取授权
    (
        "/ssl/cert/parse",
        Required::User,
        Some(Perm::action("ssl", "view")),
    ),
    (
        "/ssl/cert/add",
        Required::User,
        Some(Perm::action("ssl", "create")),
    ),
    (
        "/ssl/cert/self-sign",
        Required::User,
        Some(Perm::action("ssl", "create")),
    ),
    (
        "/ssl/cert/letsencrypt",
        Required::User,
        Some(Perm::action("ssl", "create")),
    ),
    (
        "/ssl/cert/update",
        Required::User,
        Some(Perm::action("ssl", "update")),
    ),
    (
        "/ssl/cert/delete",
        Required::User,
        Some(Perm::action("ssl", "delete")),
    ),
    // ── Let's Encrypt 订单（异步流程：提交 → 轮询 → 验证 / 取消）──
    // 子路径必须逐条登记：前缀规则是「最长命中」，漏一条就会继承父路径的动作。
    (
        "/ssl/letsencrypt",
        Required::User,
        Some(Perm::action("ssl", "create")),
    ),
    (
        "/ssl/letsencrypt/orders",
        Required::User,
        Some(Perm::action("ssl", "view")),
    ),
    (
        "/ssl/letsencrypt/status",
        Required::User,
        Some(Perm::action("ssl", "view")),
    ),
    (
        "/ssl/letsencrypt/verify",
        Required::User,
        Some(Perm::action("ssl", "create")),
    ),
    (
        "/ssl/letsencrypt/cancel",
        Required::User,
        Some(Perm::action("ssl", "create")),
    ),
    // ── ACME DNS-01 服务商凭据（证书页加载即拉清单，必须对用户可见）──
    (
        "/ssl/acme/dns/providers",
        Required::User,
        Some(Perm::action("ssl", "view")),
    ),
    (
        "/ssl/acme/dns/list",
        Required::User,
        Some(Perm::action("ssl", "view")),
    ),
    // save 为 upsert（新增与编辑同一入口），按修改授权
    (
        "/ssl/acme/dns/save",
        Required::User,
        Some(Perm::action("ssl", "update")),
    ),
    (
        "/ssl/acme/dns/delete",
        Required::User,
        Some(Perm::action("ssl", "delete")),
    ),
    // POST 但只做连通性验证，按读取授权
    (
        "/ssl/acme/dns/test",
        Required::User,
        Some(Perm::action("ssl", "view")),
    ),
    // ── 文件管理：view / write / delete（handler 内按 home 收敛）──
    (
        "/system/files/list",
        Required::User,
        Some(Perm::action("system.file", "view")),
    ),
    (
        "/system/files/read",
        Required::User,
        Some(Perm::action("system.file", "view")),
    ),
    (
        "/system/files/download",
        Required::User,
        Some(Perm::action("system.file", "view")),
    ),
    (
        "/system/files/info",
        Required::User,
        Some(Perm::action("system.file", "view")),
    ),
    (
        "/system/files/write",
        Required::User,
        Some(Perm::action("system.file", "write")),
    ),
    // 打包下载是读取的另一种形式；复制会落地新文件，按写入授权
    (
        "/system/files/archive",
        Required::User,
        Some(Perm::action("system.file", "view")),
    ),
    (
        "/system/files/copy",
        Required::User,
        Some(Perm::action("system.file", "write")),
    ),
    (
        "/system/files/mkdir",
        Required::User,
        Some(Perm::action("system.file", "write")),
    ),
    (
        "/system/files/upload",
        Required::User,
        Some(Perm::action("system.file", "write")),
    ),
    (
        "/system/files/rename",
        Required::User,
        Some(Perm::action("system.file", "write")),
    ),
    (
        "/system/files/chmod",
        Required::User,
        Some(Perm::action("system.file", "write")),
    ),
    (
        "/system/files/chown",
        Required::Admin,
        Some(Perm::action("system.file", "write")),
    ),
    (
        "/system/files/delete",
        Required::User,
        Some(Perm::action("system.file", "delete")),
    ),
    // ── 云存储：view / write / delete（配置与对象都按当前用户隔离）──
    (
        "/system/cloud/stores",
        Required::User,
        Some(Perm::action("system.cloud", "view")),
    ),
    (
        "/system/cloud/test",
        Required::User,
        Some(Perm::action("system.cloud", "view")),
    ),
    (
        "/system/cloud/list",
        Required::User,
        Some(Perm::action("system.cloud", "view")),
    ),
    (
        "/system/cloud/download",
        Required::User,
        Some(Perm::action("system.cloud", "view")),
    ),
    (
        "/system/cloud/store/save",
        Required::User,
        Some(Perm::action("system.cloud", "write")),
    ),
    (
        "/system/cloud/mkdir",
        Required::User,
        Some(Perm::action("system.cloud", "write")),
    ),
    (
        "/system/cloud/upload",
        Required::User,
        Some(Perm::action("system.cloud", "write")),
    ),
    (
        "/system/cloud/rename",
        Required::User,
        Some(Perm::action("system.cloud", "write")),
    ),
    (
        "/system/cloud/local/list",
        Required::User,
        Some(Perm::action("system.cloud", "view")),
    ),
    (
        "/system/cloud/upload-local",
        Required::User,
        Some(Perm::action("system.cloud", "write")),
    ),
    (
        "/system/cloud/delete",
        Required::User,
        Some(Perm::action("system.cloud", "delete")),
    ),
    (
        "/system/cloud/store/delete",
        Required::User,
        Some(Perm::action("system.cloud", "delete")),
    ),
    // ── 应用商店：view / install / uninstall / upgrade / manage / log / retry ──
    (
        "/appstore/install",
        Required::User,
        Some(Perm::action("appstore", "install")),
    ),
    (
        "/appstore/uninstall",
        Required::User,
        Some(Perm::action("appstore", "uninstall")),
    ),
    (
        "/appstore/upgrade",
        Required::User,
        Some(Perm::action("appstore", "upgrade")),
    ),
    (
        "/appstore/instance",
        Required::User,
        Some(Perm::action("appstore", "manage")),
    ),
    (
        "/appstore/runs",
        Required::User,
        Some(Perm::action("appstore", "log")),
    ),
    (
        "/appstore/log",
        Required::User,
        Some(Perm::action("appstore", "log")),
    ),
    (
        "/appstore/ws",
        Required::User,
        Some(Perm::action("appstore", "log")),
    ),
    (
        "/appstore/run/files",
        Required::User,
        Some(Perm::action("appstore", "log")),
    ),
    (
        "/appstore/run/file/read",
        Required::User,
        Some(Perm::action("appstore", "log")),
    ),
    (
        "/appstore/run/file/write",
        Required::Admin,
        Some(Perm::action("appstore", "retry")),
    ),
    (
        "/appstore/run/retry",
        Required::Admin,
        Some(Perm::action("appstore", "retry")),
    ),
    (
        "/appstore/repos/add",
        Required::Admin,
        Some(Perm::action("appstore.repo", "create")),
    ),
    (
        "/appstore/repos/remove",
        Required::Admin,
        Some(Perm::action("appstore.repo", "delete")),
    ),
    (
        "/appstore/repos/update",
        Required::Admin,
        Some(Perm::action("appstore.repo", "update")),
    ),
    (
        "/appstore/scripts/tree",
        Required::Admin,
        Some(Perm::action("appstore.script", "view")),
    ),
    (
        "/appstore/script/read",
        Required::Admin,
        Some(Perm::action("appstore.script", "view")),
    ),
    (
        "/appstore/script/write",
        Required::Admin,
        Some(Perm::action("appstore.script", "write")),
    ),
    (
        "/appstore/script/run",
        Required::Admin,
        Some(Perm::action("appstore.script", "run")),
    ),
    (
        "/appstore/script/stop",
        Required::Admin,
        Some(Perm::action("appstore.script", "run")),
    ),
    (
        "/appstore/script/delete",
        Required::Admin,
        Some(Perm::action("appstore.script", "write")),
    ),
    (
        "/appstore/packages",
        Required::User,
        Some(Perm::action("appstore", "view")),
    ),
    (
        "/appstore/installed",
        Required::User,
        Some(Perm::action("appstore", "view")),
    ),
    (
        "/appstore/repos",
        Required::User,
        Some(Perm::action("appstore", "view")),
    ),
    // ── 用户 / 角色 / 菜单 / IP：增删改查拆开 ────────────────
    (
        "/system/user/list",
        Required::Reseller,
        Some(Perm::action("system.user", "view")),
    ),
    (
        "/system/user/resellers",
        Required::Admin,
        Some(Perm::action("system.user", "view")),
    ),
    (
        "/system/user/add",
        Required::Reseller,
        Some(Perm::action("system.user", "create")),
    ),
    (
        "/system/user/update",
        Required::Reseller,
        Some(Perm::action("system.user", "update")),
    ),
    (
        "/system/user/delete",
        Required::Reseller,
        Some(Perm::action("system.user", "delete")),
    ),
    (
        "/system/user/home_sync",
        Required::Admin,
        Some(Perm::action("system.user", "sync")),
    ),
    // 权限点目录：只是静态的文案/分组元数据，不含任何用户数据，
    // 开放给普通用户是为了让"团队成员"页面能给成员做权限收紧选择。
    ("/system/role/permission-catalog", Required::User, None),
    // 家目录备份：任意登录用户可备份自己的家目录，目标用户名在 handler 内收敛
    (
        "/system/user/backup-home",
        Required::User,
        Some(Perm::action("system.user", "backup")),
    ),
    (
        "/system/role/list",
        Required::Admin,
        Some(Perm::action("system.role", "view")),
    ),
    (
        "/system/role/permissions",
        Required::Admin,
        Some(Perm::action("system.role", "view")),
    ),
    (
        "/system/role/add",
        Required::Admin,
        Some(Perm::action("system.role", "create")),
    ),
    (
        "/system/role/update",
        Required::Admin,
        Some(Perm::action("system.role", "update")),
    ),
    (
        "/system/role/delete",
        Required::Admin,
        Some(Perm::action("system.role", "delete")),
    ),
    (
        "/system/menus/tree",
        Required::User,
        Some(Perm::action("system.menu", "view")),
    ),
    (
        "/system/menus/revision",
        Required::User,
        Some(Perm::action("system.menu", "view")),
    ),
    (
        "/system/menus/list",
        Required::Admin,
        Some(Perm::action("system.menu", "view")),
    ),
    (
        "/system/menus/add",
        Required::Admin,
        Some(Perm::action("system.menu", "create")),
    ),
    (
        "/system/menus/update",
        Required::Admin,
        Some(Perm::action("system.menu", "update")),
    ),
    (
        "/system/menus/status",
        Required::Admin,
        Some(Perm::action("system.menu", "update")),
    ),
    (
        "/system/menus/delete",
        Required::Admin,
        Some(Perm::action("system.menu", "delete")),
    ),
    (
        "/system/ip/list",
        Required::Admin,
        Some(Perm::action("system.ip", "view")),
    ),
    (
        "/system/ip/add",
        Required::Admin,
        Some(Perm::action("system.ip", "create")),
    ),
    (
        "/system/ip/update",
        Required::Admin,
        Some(Perm::action("system.ip", "update")),
    ),
    (
        "/system/ip/batch-reserved",
        Required::Admin,
        Some(Perm::action("system.ip", "update")),
    ),
    (
        "/system/ip/delete",
        Required::Admin,
        Some(Perm::action("system.ip", "delete")),
    ),
    // ── 开发者接口（API Token）───────────────────────────────
    (
        "/dev/api-token/list",
        Required::Admin,
        Some(Perm::action("dev", "view")),
    ),
    (
        "/dev/api-docs",
        Required::Admin,
        Some(Perm::action("dev", "view")),
    ),
    (
        "/dev/api-token/create",
        Required::Admin,
        Some(Perm::action("dev", "create")),
    ),
    (
        "/dev/api-token/update",
        Required::Admin,
        Some(Perm::action("dev", "update")),
    ),
    (
        "/dev/api-token/delete",
        Required::Admin,
        Some(Perm::action("dev", "delete")),
    ),
    // ── 服务器配置：view / edit + 高危动作单独拆出 ───────────
    (
        "/system/config/services/action",
        Required::Admin,
        Some(Perm::action("system.config", "service")),
    ),
    (
        "/system/config/processes/kill",
        Required::Admin,
        Some(Perm::action("system.config", "process")),
    ),
    (
        "/system/config/ssh/restart",
        Required::Admin,
        Some(Perm::action("system.config", "ssh")),
    ),
    (
        "/system/config/ssh/install",
        Required::Admin,
        Some(Perm::action("system.config", "ssh")),
    ),
    (
        "/system/config/firewall",
        Required::Admin,
        Some(Perm::action("system.config", "firewall")),
    ),
    (
        "/system/config",
        Required::Admin,
        Some(Perm::module("system.config")),
    ),
    // ── 只读监控 / 审计 ──────────────────────────────────────
    (
        "/system/info",
        Required::User,
        Some(Perm::action("system.monitor", "view")),
    ),
    // 「关于」页只有面板自身的版本信息，不存在泄露面（admin 专属的是 /system/status）
    (
        "/system/about",
        Required::User,
        Some(Perm::action("system.monitor", "view")),
    ),
    (
        "/system/status",
        Required::Admin,
        Some(Perm::action("system.monitor", "view")),
    ),
    (
        "/system/overview",
        Required::Admin,
        Some(Perm::action("system.monitor", "view")),
    ),
    (
        "/system/audit",
        Required::Admin,
        Some(Perm::action("system.audit", "view")),
    ),
    // ── 全局任务：GET 做状态变更，显式标记 edit ──────────────
    (
        "/system/job",
        Required::Admin,
        Some(Perm::action("system.job", "edit")),
    ),
    // ── 终端与密钥：路径含动态 id，按方法派生 view / edit ────
    ("/terminal", Required::User, Some(Perm::module("terminal"))),
    // ── 计划任务（crontab）：所有角色管理自己的任务；执行身份由 handler 收敛 ──
    // 更具体的条目排在前面亦无妨：匹配取最长前缀。
    (
        "/terminal/crontab/exec-users",
        Required::Admin,
        Some(Perm::action("crontab", "view")),
    ),
    (
        "/terminal/crontab",
        Required::User,
        Some(Perm::module("crontab")),
    ),
    // ── 其余 admin-only 模块：按方法派生 view / edit ─────────
    (
        "/system/nginx",
        Required::Admin,
        Some(Perm::module("service.nginx")),
    ),
    (
        "/system/service-conf",
        Required::Admin,
        Some(Perm::module("service.conf")),
    ),
    (
        "/system/package",
        Required::Reseller,
        Some(Perm::module("system.package")),
    ),
    // 规格模板：reseller 可读（自己名下 + 全局模板），写入仅 admin
    (
        "/system/fpm-specs/list",
        Required::Reseller,
        Some(Perm::module("system.fpm_spec")),
    ),
    (
        "/system/fpm-specs",
        Required::Admin,
        Some(Perm::module("system.fpm_spec")),
    ),
    (
        "/system/update",
        Required::Admin,
        Some(Perm::module("system.update")),
    ),
    (
        "/system/migrate",
        Required::Admin,
        Some(Perm::module("system.migrate")),
    ),
    (
        "/system/env",
        Required::Admin,
        Some(Perm::module("system.env")),
    ),
    (
        "/system/cron",
        Required::Admin,
        Some(Perm::module("system.cron")),
    ),
    // Web 应用（/webapps/*，页面级路由）：登录用户即可，是否可访问由
    // `webapp.phpmyadmin:view` 权限点控制，可在「角色权限」中按角色分配。
    (
        "/webapps",
        Required::User,
        Some(Perm::action("webapp.phpmyadmin", "view")),
    ),
    // 数据库管理：管理员可管全部库，普通用户只能管自己前缀下的库
    ("/database", Required::User, Some(Perm::module("database"))),
    // 容器管理：整机资源（启停容器、拉镜像、删卷），仅管理员
    ("/docker", Required::Admin, Some(Perm::module("docker"))),
    // 镜像构建 / 详情 / 列表：可以按角色放开（多用户场景）。
    //
    // 这几条写在 `/docker` 之后，靠「更长前缀优先」覆盖模块级门禁：
    // 普通用户拿到 `docker:build` 后能构建自己的镜像、看镜像详情，
    // 但容器 / 卷 / 网络 / Compose 仍然只有 admin 能动。
    (
        "/docker/image/build",
        Required::User,
        Some(Perm::action("docker", "build")),
    ),
    (
        "/docker/image/inspect",
        Required::User,
        Some(Perm::action("docker", "build")),
    ),
    // 环境探测只回有没有装 Docker / 版本号，对构建者可见更合理：
    // 否则镜像页顶部的探测条会因 403 显示成「未检测到 Docker」。
    // 容器只读视图：给了 `docker:view` 就能看容器列表 / 资源占用 / 详情 / 日志。
    // 只是「看」——改状态要 `docker:manage`，删容器 / 建容器 / 卷 / 网络 / Compose /
    // exec 终端仍然只有 admin（那些等于直接动宿主机）。
    (
        "/docker/containers",
        Required::User,
        Some(Perm::action("docker", "view")),
    ),
    (
        "/docker/stats",
        Required::User,
        Some(Perm::action("docker", "view")),
    ),
    (
        "/docker/container/inspect",
        Required::User,
        Some(Perm::action("docker", "view")),
    ),
    (
        "/docker/container/logs",
        Required::User,
        Some(Perm::action("docker", "view")),
    ),
    // 容器启停：start / stop / restart / pause / unpause。
    // kill / remove 由 handler 再挡一层，避免「给个启停」变成「能删容器」。
    (
        "/docker/container/action",
        Required::User,
        Some(Perm::action("docker", "manage")),
    ),
    // 环境探测只回有没有装 Docker / 版本号，构建者与容器查看者都要能看到：
    // 否则页面顶部的探测条会因 403 显示成「未检测到 Docker」。
    // 构建者不受影响：`docker:build` 蕴含 `docker:view`（见 IMPLIED_PERMS）。
    (
        "/docker/status",
        Required::User,
        Some(Perm::action("docker", "view")),
    ),
    (
        "/docker/images",
        Required::User,
        Some(Perm::action("docker", "build")),
    ),
    // 通用任务队列：应用商店安装 / Docker 构建 / 备份 / 升级 / 计划任务的运行记录。
    //
    // 读取类（列表 / 统计 / 详情 / 日志 / 实时日志）对普通用户开放 `task:view`：
    // 列表与详情在 handler 里按归属收敛 —— 非管理员只看得到自己的任务，
    // 所以"能看到任务"不等于"能看到别人的日志"。
    (
        "/task/list",
        Required::User,
        Some(Perm::action("task", "view")),
    ),
    (
        "/task/stats",
        Required::User,
        Some(Perm::action("task", "view")),
    ),
    (
        "/task/detail",
        Required::User,
        Some(Perm::action("task", "view")),
    ),
    (
        "/task/log",
        Required::User,
        Some(Perm::action("task", "view")),
    ),
    (
        "/task/ws",
        Required::User,
        Some(Perm::action("task", "view")),
    ),
    // 取消自己的任务：`task:control`，handler 再挡一层「非管理员只能操作自己的」。
    (
        "/task/cancel",
        Required::User,
        Some(Perm::action("task", "control")),
    ),
    // 暂停 / 继续是下发进程信号，先只对管理员开放：让普通用户随意挂起他人任务
    // 相当于一种拒绝服务手段。
    (
        "/task/pause",
        Required::Admin,
        Some(Perm::action("task", "control")),
    ),
    (
        "/task/resume",
        Required::Admin,
        Some(Perm::action("task", "control")),
    ),
];

/// **只允许显式授予**的权限点：内置角色初始化时不会自动带上（admin 除外）。
///
/// 规则里写了 `Required::User` 只代表「这条接口的角色下限是普通用户」，
/// 不代表「普通用户默认就该有这个能力」。凡是「拿到之后等于拿到宿主机 root」的
/// 能力都登记在这里，由管理员在「角色权限」里逐个勾选，默认关闭：
///
/// - `docker:build`：构建镜像 = 让 Containerfile 里的 `RUN` 以 root 跑在宿主机上。
///
/// - `docker:build`：构建镜像 = 让 Containerfile 里的 `RUN` 以 root 跑在宿主机上。
/// - `docker:view`：看得到别人的容器（列表 / 日志 / inspect）—— 日志可能含敏感信息。
/// - `docker:manage`：启停别人的容器，等于能中断宿主机上的服务。
const EXPLICIT_ONLY_PERMS: &[&str] = &["docker:build", "docker:view", "docker:manage"];

/// 权限点蕴含：持有左侧即视为持有右侧。
///
/// `docker:build` 自带 `docker:view`：构建完总得看得到镜像与容器状态，
/// 否则镜像页顶部的环境探测条会因为缺 `docker:view` 而 403（显示成「未检测到 Docker」）。
const IMPLIED_PERMS: &[(&str, &str)] = &[("docker:build", "docker:view")];

/// 权限集合是否覆盖某权限点（含蕴含推导）。
fn perm_satisfied(set: &HashSet<String>, key: &str) -> bool {
    if set.contains(key) {
        return true;
    }
    IMPLIED_PERMS
        .iter()
        .any(|(from, to)| *to == key && set.contains(*from))
}

/// 权限点命名空间的中文名（用于角色权限配置页与权限目录接口）。
const NS_LABELS: &[(&str, &str)] = &[
    ("task", "任务队列"),
    ("system.menu", "菜单管理"),
    ("system.file", "文件管理"),
    ("system.cloud", "云存储"),
    ("system.monitor", "服务器状态"),
    ("system.user", "用户管理"),
    ("system.package", "套餐管理"),
    ("system.fpm_spec", "PHP-FPM 规格"),
    ("system.role", "角色权限"),
    ("system.audit", "审计日志"),
    ("system.update", "系统更新"),
    ("system.migrate", "数据迁移"),
    ("system.env", "运行环境"),
    ("system.cron", "计划任务"),
    ("system.job", "全局任务"),
    ("service.nginx", "Nginx 服务"),
    ("service.conf", "服务配置"),
    ("system.ip", "IP 池"),
    ("database", "数据库管理"),
    ("docker", "容器管理"),
    ("system.config", "服务器配置"),
    ("site", "站点管理"),
    ("ssl", "SSL 证书"),
    ("terminal", "终端与密钥"),
    ("crontab", "计划任务"),
    ("appstore", "应用商店"),
    ("appstore.repo", "应用源管理"),
    ("appstore.script", "自定义脚本"),
    ("dev", "开发者接口"),
    ("webapp.phpmyadmin", "phpMyAdmin"),
];

/// 动作的中文名（角色权限配置页展示）。
const ACTION_LABELS: &[(&str, &str)] = &[
    ("view", "查看"),
    ("edit", "编辑"),
    ("create", "创建"),
    ("update", "修改"),
    ("delete", "删除"),
    ("write", "写入"),
    ("sync", "同步"),
    ("state", "启停/维护"),
    ("install", "安装"),
    ("uninstall", "卸载"),
    ("upgrade", "升级"),
    ("manage", "实例管理"),
    ("control", "任务控制"),
    ("log", "运行日志"),
    ("retry", "重跑"),
    ("run", "执行"),
    ("service", "服务启停"),
    ("process", "进程管理"),
    ("ssh", "SSH 服务"),
    ("firewall", "防火墙"),
    ("build", "构建"),
    ("manage", "启停"),
];

/// 未知动作用原样兜底（新增动作忘了登记中文名时，界面至少能看清是什么）。
fn action_label(action: &str) -> String {
    ACTION_LABELS
        .iter()
        .find(|(k, _)| *k == action)
        .map(|(_, l)| (*l).to_string())
        .unwrap_or_else(|| action.to_string())
}

/// 剥离 URL 前缀（`server.url_prefix`）与 `/api`，得到与 `RULES` 对齐的路径。
///
/// 中间件挂在 `api_routers()` 上，`Router::nest` 通常已剥掉外层前缀；
/// 这里幂等再剥一次，保证前缀启用/未启用、以及中间件挂载层级变化时行为一致。
fn normalize(path: &str) -> String {
    let mut p = path;
    if let Some(rest) = strip_prefix(p, &crate::config::url_prefix_path()) {
        p = rest;
    }
    if let Some(rest) = strip_prefix(p, "/api") {
        p = rest;
    }
    if p.is_empty() {
        "/".to_string()
    } else {
        p.to_string()
    }
}

/// 按路径段剥离前缀：`/api/health` - `/api` = `/health`；`/api` - `/api` = `/`。
fn strip_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return None;
    }
    match path.strip_prefix(prefix) {
        Some("") => Some("/"),
        Some(rest) if rest.starts_with('/') => Some(rest),
        _ => None,
    }
}

/// 前缀是否命中（按路径段边界，避免 `/appstore/script` 命中 `/appstore/scripts/tree`）。
fn prefix_hit(path: &str, prefix: &str) -> bool {
    match path.strip_prefix(prefix) {
        Some("") => true,
        Some(rest) => rest.starts_with('/'),
        None => false,
    }
}

/// 查询路径的命中项：最长前缀命中；未命中 → `(Admin, None)`（默认拒绝）。
fn lookup(path: &str) -> (Required, Option<Perm>) {
    let mut best: Option<(usize, Required, Option<Perm>)> = None;
    for (prefix, req, perm) in RULES {
        if !prefix_hit(path, prefix) {
            continue;
        }
        if best.is_none_or(|(len, _, _)| prefix.len() > len) {
            best = Some((prefix.len(), *req, *perm));
        }
    }
    best.map(|(_, r, perm)| (r, perm))
        .unwrap_or((Required::Admin, None))
}

/// 查询路径所需角色（兼容旧调用与测试）。
fn required_for(path: &str) -> Required {
    lookup(path).0
}

/// 请求实际需要的权限点：`{ns}:{action}`。
///
/// 动作来源：规则显式指定，或按 HTTP 方法派生（GET/HEAD → `view`，其余 → `edit`）。
/// 未登记权限点的接口返回 `None`，此时只有角色下限生效（如 `/user/*`、`/auth/*`）。
pub fn perm_key_for(path: &str, method: &Method) -> Option<String> {
    let perm = lookup(path).1?;
    let action = match perm.action {
        Some(a) => a,
        None if method == Method::GET || method == Method::HEAD => "view",
        None => "edit",
    };
    Some(format!("{}:{action}", perm.ns))
}

/// 权限目录项：一个命名空间 = 一个可勾选的模块，含其全部动作。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PermGroup {
    pub ns: &'static str,
    pub label: &'static str,
    /// 该模块的动作列表（如 `view` / `create` / `update` / `delete`）
    pub actions: Vec<PermAction>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PermAction {
    pub key: String,
    pub label: String,
}

/// 权限目录（角色权限配置页的数据源）：去重自 `RULES`，顺序与矩阵一致。
///
/// 动作集合 = 规则里显式声明的动作 ∪ （若存在按方法派生的规则，则加 view / edit）。
pub fn permission_catalog() -> Vec<PermGroup> {
    let mut out: Vec<PermGroup> = Vec::new();
    for (_, _, perm_opt) in RULES {
        let Some(perm) = perm_opt else { continue };
        let ns: &'static str = perm.ns;
        let group = match out.iter_mut().find(|g| g.ns == ns) {
            Some(g) => g,
            None => {
                let label = NS_LABELS
                    .iter()
                    .find(|(k, _)| *k == ns)
                    .map(|(_, l)| *l)
                    .unwrap_or(ns);
                out.push(PermGroup {
                    ns,
                    label,
                    actions: Vec::new(),
                });
                out.last_mut().unwrap()
            }
        };
        match perm.action {
            Some(a) => group.actions.push(PermAction {
                key: format!("{ns}:{a}"),
                label: action_label(a),
            }),
            None => {
                group.actions.push(PermAction {
                    key: format!("{ns}:view"),
                    label: action_label("view"),
                });
                group.actions.push(PermAction {
                    key: format!("{ns}:edit"),
                    label: action_label("edit"),
                });
            }
        }
    }

    // 去重（同一动作可能由多条规则声明）
    for g in &mut out {
        let mut seen: HashSet<String> = HashSet::new();
        g.actions.retain(|a| seen.insert(a.key.clone()));
    }
    out
}

/// 全部合法权限点（用于校验写入，拒绝脏数据）。
pub fn all_perm_keys() -> HashSet<String> {
    permission_catalog()
        .into_iter()
        .flat_map(|g| g.actions.into_iter().map(|a| a.key))
        .collect()
}

/// 内置角色的默认权限点：**逐条规则**推导，保证「升级前后行为一致」。
///
/// 规则：角色能满足该条规则的角色下限，就获得这条规则对应的权限点。
/// - admin：全部（且运行时恒直通，避免配置失误把自己锁死）
/// - reseller：下限为 User / Reseller 的规则
/// - 其它内置角色（user / demo）：仅下限为 User 的规则
///
/// 逐条推导（而非按模块整包）可以避免给普通用户塞入用不上的动作：
/// 例如 `appstore:retry` 对应的是 admin-only 规则，普通用户不该出现在他的清单里。
///
/// [`EXPLICIT_ONLY_PERMS`] 里的权限点不参与推导（admin 除外）：它们的规则下限是
/// `Required::User`，但能力本身太重，只能由管理员逐角色勾选，默认关闭。
pub fn default_permissions_for(role_key: &str) -> Vec<String> {
    let is_admin = role_key == "admin";
    let is_reseller = role_key == "reseller";

    let mut set: HashSet<String> = HashSet::new();
    for (_, req, perm_opt) in RULES {
        let Some(perm) = perm_opt else { continue };
        let reachable =
            is_admin || *req == Required::User || (is_reseller && *req == Required::Reseller);
        if !reachable {
            continue;
        }
        let mut keys: Vec<String> = match perm.action {
            Some(a) => vec![format!("{}:{a}", perm.ns)],
            None => vec![format!("{}:view", perm.ns), format!("{}:edit", perm.ns)],
        };
        if !is_admin {
            keys.retain(|k| !EXPLICIT_ONLY_PERMS.contains(&k.as_str()));
        }
        set.extend(keys);
    }

    let mut out: Vec<String> = set.into_iter().collect();
    out.sort();
    out
}

// ── 角色权限缓存 ───────────────────────────────────────────

type PermMap = HashMap<String, HashSet<String>>;
static PERM_CACHE: OnceLock<RwLock<Option<Arc<PermMap>>>> = OnceLock::new();

fn cache_slot() -> &'static RwLock<Option<Arc<PermMap>>> {
    PERM_CACHE.get_or_init(|| RwLock::new(None))
}

/// 角色权限变更后调用，立即失效缓存（撤销权限不必等过期）。
///
/// 用户生效权限由角色权限推导而来，角色一改成员的继承结果也跟着变，
/// 因此这里必须连带清掉用户生效权限缓存（否则成员要等到下次用户变更才生效）。
pub fn invalidate_perm_cache() {
    if let Ok(mut guard) = cache_slot().write() {
        *guard = None;
    }
    invalidate_user_perm_cache();
}

async fn load_perm_map() -> PermMap {
    // 数据库不可用时返回空表（非 admin 一律拒绝），而不是 panic 掉整个请求。
    let Some(pool) = crate::db::get_db_pool_opt().await else {
        return PermMap::new();
    };
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT r.role_key, p.perm_key FROM role_permissions p JOIN roles r ON r.id = p.role_id",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut map: PermMap = HashMap::new();
    for (role, key) in rows {
        map.entry(role).or_default().insert(key);
    }
    map
}

/// `role_key → 权限点集合`（进程内缓存，写操作通过 `invalidate_perm_cache` 失效）。
pub async fn perm_map() -> Arc<PermMap> {
    if let Ok(guard) = cache_slot().read()
        && let Some(cached) = guard.as_ref()
    {
        return cached.clone();
    }
    let map = Arc::new(load_perm_map().await);
    if let Ok(mut guard) = cache_slot().write() {
        *guard = Some(map.clone());
    }
    map
}

// ── 用户生效权限缓存 ───────────────────────────────────────

/// 用户类型：成员（子账号）。见 `user.user_kind`。
///
/// 成员共享父账号（`owner_id`）的家目录与 Linux 系统账号，权限默认继承父账号。
pub const USER_KIND_MEMBER: i32 = 1;

/// 继承链最大解析深度（成员只允许一层，迭代只是防御脏数据成环）。
const MAX_INHERIT_DEPTH: usize = 4;

/// `user.id → 生效权限点集合`。
///
/// 生效 = 自身（角色授予 ∪ 个人附加）∪ 父账号继承（成员） − 父账号收紧（perm_deny）；
/// 只读账号（`user.read_only=1`）再收敛为仅查看类权限点（`{ns}:view`）。
type UserPermMap = HashMap<i64, HashSet<String>>;
static USER_PERM_CACHE: OnceLock<RwLock<Option<Arc<UserPermMap>>>> = OnceLock::new();

fn user_cache_slot() -> &'static RwLock<Option<Arc<UserPermMap>>> {
    USER_PERM_CACHE.get_or_init(|| RwLock::new(None))
}

/// 用户新增 / 修改 / 删除后调用（只读标记随之一起失效）。
pub fn invalidate_user_perm_cache() {
    if let Ok(mut guard) = user_cache_slot().write() {
        *guard = None;
    }
    if let Ok(mut guard) = readonly_slot().write() {
        *guard = None;
    }
}

/// 用户自身权限点（不含继承）：角色授予 ∪ 个人附加权限。
fn own_perms(role_perms: &PermMap, roles: &str, extra: &str) -> HashSet<String> {
    let mut set: HashSet<String> = HashSet::new();
    let keys: Vec<&str> = roles
        .split(',')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .collect();
    // admin 与运行时直通保持一致：视为持有全部权限点
    if keys.contains(&"admin") {
        set.extend(all_perm_keys());
    } else {
        for r in &keys {
            if let Some(s) = role_perms.get(*r) {
                set.extend(s.iter().cloned());
            }
        }
    }
    for p in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        set.insert(p.to_string());
    }
    set
}

fn split_keys(csv: &str) -> HashSet<String> {
    csv.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

async fn load_user_perm_map() -> UserPermMap {
    let Some(pool) = crate::db::get_db_pool_opt().await else {
        return UserPermMap::new();
    };
    let rows: Vec<(i64, String, String, i64, i32, String, i32)> = sqlx::query_as(
        "SELECT id, roles, permissions, owner_id, user_kind, perm_deny, read_only FROM user",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let role_perms = perm_map().await;
    let mut own: UserPermMap = UserPermMap::new();
    // 成员 → 父账号；仅 user_kind=1 且 owner_id 有效时登记
    let mut parent_of: HashMap<i64, i64> = HashMap::new();
    let mut deny_of: HashMap<i64, HashSet<String>> = HashMap::new();
    let mut readonly: HashSet<i64> = HashSet::new();
    for (id, roles, permissions, owner_id, user_kind, perm_deny, read_only) in rows {
        own.insert(id, own_perms(&role_perms, &roles, &permissions));
        if user_kind == USER_KIND_MEMBER && owner_id > 0 && owner_id != id {
            parent_of.insert(id, owner_id);
            deny_of.insert(id, split_keys(&perm_deny));
        }
        if read_only != 0 {
            readonly.insert(id);
        }
    }

    // 成员默认全量继承父账号的生效权限，父账号可再收紧：
    // 生效 = （父生效 ∪ 自身）− perm_deny，且高危权限（EXPLICIT_ONLY）只能本人显式持有。
    let mut eff = own.clone();
    for _ in 0..MAX_INHERIT_DEPTH {
        let mut changed = false;
        for (id, parent) in &parent_of {
            let Some(parent_set) = eff.get(parent).cloned() else {
                continue;
            };
            let self_set = own.get(id).cloned().unwrap_or_default();
            let mut merged = parent_set;
            merged.extend(self_set.iter().cloned());
            merged.retain(|k| !EXPLICIT_ONLY_PERMS.contains(&k.as_str()) || self_set.contains(k));
            if let Some(d) = deny_of.get(id) {
                for k in d {
                    merged.remove(k);
                }
            }
            if eff.get(id) != Some(&merged) {
                eff.insert(*id, merged);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // 只读账号：共享可见但不可改 —— 只保留查看类权限点（{ns}:view）
    if !readonly.is_empty() {
        for (id, set) in eff.iter_mut() {
            if readonly.contains(id) {
                set.retain(|k| k.ends_with(":view"));
            }
        }
    }
    eff
}

pub async fn user_perm_map() -> Arc<UserPermMap> {
    if let Ok(guard) = user_cache_slot().read()
        && let Some(cached) = guard.as_ref()
    {
        return cached.clone();
    }
    let map = Arc::new(load_user_perm_map().await);
    if let Ok(mut guard) = user_cache_slot().write() {
        *guard = Some(map.clone());
    }
    map
}

// ── 只读账号 ───────────────────────────────────────────────
//
// `user.read_only=1` 的账号「共享可见但不能改」。生效权限已在
// `load_user_perm_map` 里收敛为 `{ns}:view`，但门禁还有一路**角色授予**：
// 角色默认权限（user 角色自带 `site:update`）不经过收敛，若照旧取
// 「角色 ∪ 用户」的并集，只读会被角色路径整体绕过。只读账号因此单独
// 只走「用户生效权限」这一路。

type ReadOnlySet = HashSet<i64>;
static READONLY_CACHE: OnceLock<RwLock<Option<Arc<ReadOnlySet>>>> = OnceLock::new();

fn readonly_slot() -> &'static RwLock<Option<Arc<ReadOnlySet>>> {
    READONLY_CACHE.get_or_init(|| RwLock::new(None))
}

/// `read_only` 用户集合（进程内缓存，随用户变更一起失效）。
async fn readonly_set() -> Arc<ReadOnlySet> {
    if let Ok(guard) = readonly_slot().read()
        && let Some(cached) = guard.as_ref()
    {
        return cached.clone();
    }
    let Some(pool) = crate::db::get_db_pool_opt().await else {
        return Arc::new(ReadOnlySet::new());
    };
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT id FROM user WHERE read_only <> 0")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    let set = Arc::new(rows.into_iter().map(|(id,)| id).collect::<ReadOnlySet>());
    if let Ok(mut guard) = readonly_slot().write() {
        *guard = Some(set.clone());
    }
    set
}

/// 该用户是否被标记为只读（`user.read_only=1`）。
pub async fn user_is_read_only(uid: u64) -> bool {
    readonly_set().await.contains(&(uid as i64))
}

/// 只读账号可放行的写接口白名单：**个人账户操作**（改自己密码 / 2FA、消息已读）。
///
/// 其余没登记权限点的写接口（如成员管理 `/user/team/*`）一律拒绝。
const READONLY_WRITE_WHITELIST: &[&str] = &["/auth", "/user/notices"];

/// 只读账号是否可发起该请求 —— 用于**未登记权限点**的接口兜底。
///
/// 登记了权限点的接口走 `action_gate`：只读只剩 `{ns}:view`，写操作自然被拒；
/// 没登记权限点的接口（`lookup` 第三列为 `None`）没有 key 可校验，只能按
/// 方法 + 白名单判定，否则 `/user/team/add` 这类写接口会被整个漏掉。
pub fn readonly_allows(path: &str, method: &Method) -> bool {
    // Web 终端虽走 GET，连上就是交互式 shell（等于间接写），只读账号不给
    if prefix_hit(path, "/terminal/ws") {
        return false;
    }
    if method == Method::GET || method == Method::HEAD {
        return true;
    }
    READONLY_WRITE_WHITELIST
        .iter()
        .any(|prefix| prefix_hit(path, prefix))
}

/// 用户是否持有该权限点（**生效**权限：含成员继承与父账号收紧）。
pub fn user_has_perm(map: &UserPermMap, uid: u64, key: &str) -> bool {
    map.get(&(uid as i64))
        .is_some_and(|set| perm_satisfied(set, key))
}

/// 某用户的生效权限点（排序后）：供 `/user/info` 回传前端做按钮级控制。
pub async fn effective_permissions_of(uid: u64) -> Vec<String> {
    let mut map = user_perm_map().await;
    // 缓存里查不到该用户（新建账号后缓存未失效）：重算一次再回。
    // 否则前端拿到空权限数组，所有 v-permission 按钮都被判成无权限。
    if !map.contains_key(&(uid as i64)) {
        invalidate_user_perm_cache();
        map = user_perm_map().await;
    }
    let mut out: Vec<String> = map
        .get(&(uid as i64))
        .map(|set| set.iter().cloned().collect())
        .unwrap_or_default();
    out.sort();
    out
}

/// 用户（其任一角色）是否持有该权限点。
pub fn role_has_perm(map: &PermMap, claims: &Claims, key: &str) -> bool {
    claims
        .roles
        .split(',')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .any(|r| map.get(r).is_some_and(|set| perm_satisfied(set, key)))
}

fn satisfies(claims: &Claims, required: Required) -> bool {
    match required {
        Required::Public | Required::User => true,
        Required::Reseller => jwt::is_admin(claims) || jwt::is_reseller(claims),
        Required::Admin => jwt::is_admin(claims),
    }
}

fn deny(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "code": -1, "message": message }))).into_response()
}

/// 第二层（动作级）校验的结果：通过，或附带原因的拒绝。
enum ActionGate {
    Allowed,
    Denied(String),
}

/// 动作级校验：权限点 + 只读兜底，接口与页面共用同一套判定。
///
/// - 已登记权限点：只读账号**只看收敛后的生效权限**（仅 `{ns}:view`），
///   角色授予不再兜底 —— 否则角色默认权限（如 user 角色的 `site:update`）
///   会整体绕过 `read_only`；其余账号取「角色 ∪ 用户」并集。
/// - 未登记权限点：正常账号放行（只剩角色下限），只读账号按
///   [`readonly_allows`] 兜底，避免 `/user/team/*` 这类写接口漏判。
async fn action_gate(claims: &Claims, path: &str, method: &Method) -> ActionGate {
    let readonly = user_is_read_only(claims.id).await;
    if let Some(key) = perm_key_for(path, method) {
        let allowed = if readonly {
            user_has_perm(user_perm_map().await.as_ref(), claims.id, &key)
        } else {
            role_has_perm(perm_map().await.as_ref(), claims, &key)
                || user_has_perm(user_perm_map().await.as_ref(), claims.id, &key)
        };
        if allowed {
            ActionGate::Allowed
        } else {
            ActionGate::Denied(format!("权限不足，需要权限点：{key}"))
        }
    } else if readonly && !readonly_allows(path, method) {
        ActionGate::Denied("只读账号只能查看，不能执行该操作".to_string())
    } else {
        ActionGate::Allowed
    }
}

/// 取请求凭据：优先 `Authorization: Bearer`，回退查询串 `?token=`。
///
/// 浏览器 WebSocket 无法自定义请求头，终端与实时日志端点只能把 token 放在
/// query 里（见 `ssh_terminal::ws_terminal`、`appstore::ws_log`），
/// 因此两种取值方式必须等价，否则 WebSocket 会被门禁挡在 401。
fn token_from_request(req: &Request) -> Option<String> {
    if let Some(t) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
    {
        return Some(t.to_string());
    }
    // token 为 JWT（base64url）或 `zap_` 开头的十六进制，均不含需转义字符
    req.uri().query()?.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == "token").then(|| v.to_string())
    })
}

/// 统一角色门禁中间件（挂在 `api_routers()` 上，覆盖所有 `/api/*` 接口）。
#[allow(clippy::result_large_err)] // axum 中间件约定 Result<Response, Response>
pub async fn guard(req: Request, next: Next) -> Result<Response, Response> {
    let path = normalize(req.uri().path());
    let method = req.method().clone();
    let required = required_for(&path);
    if required == Required::Public {
        return Ok(next.run(req).await);
    }

    // 先取 owned token 再异步解析，避免借用 req 跨 await
    let bearer = token_from_request(&req);

    let claims = match bearer.as_deref() {
        Some(token) => jwt::claims_from_token(token).await,
        None => None,
    };

    let Some(claims) = claims else {
        return Err(deny(
            StatusCode::UNAUTHORIZED,
            "未登录或登录已过期，请重新登录",
        ));
    };

    if !satisfies(&claims, required) {
        warn!(
            "access denied: user={} path={} required={}",
            claims.sub,
            path,
            required.label()
        );
        return Err(deny(
            StatusCode::FORBIDDEN,
            "权限不足，该操作需要更高角色权限",
        ));
    }

    // 第二层：动作级权限点 + 只读兜底。admin 恒直通，避免配置失误锁死内置管理员。
    if !jwt::is_admin(&claims)
        && let ActionGate::Denied(reason) = action_gate(&claims, &path, &method).await
    {
        warn!(
            "permission denied: user={} path={} reason={}",
            claims.sub, path, reason
        );
        return Err(deny(StatusCode::FORBIDDEN, &reason));
    }

    Ok(next.run(req).await)
}

/// 页面级路由（`/webapps/*`）鉴权：复用同一张规则表与权限点机制。
///
/// 这类页面不挂在 `api_routers()` 上（没有 `guard` 中间件），由 handler 主动调用，
/// 以取得与接口一致的两层校验（角色下限 + 动作级权限点）与 fail-closed 行为。
pub async fn authorize_page(
    claims: &Claims,
    path: &str,
    method: &Method,
) -> Result<(), (StatusCode, String)> {
    let (required, _) = lookup(path);
    if !satisfies(claims, required) {
        return Err((
            StatusCode::FORBIDDEN,
            format!("权限不足，该页面需要 {} 角色", required.label()),
        ));
    }
    if !jwt::is_admin(claims)
        && let ActionGate::Denied(reason) = action_gate(claims, path, method).await
    {
        return Err((StatusCode::FORBIDDEN, reason));
    }
    Ok(())
}

/// 取页面请求的凭据：`Authorization: Bearer` → 会话 Cookie → 查询串 `?token=`。
///
/// 前端把 token 存在 sessionStorage，直接新开页面不会自动带凭据；
/// 合并请求中所有 `Cookie` 头字段。
///
/// HTTP/2（RFC 7540 §8.1.2.5）允许把 Cookie 拆成多个 `cookie` 头字段，
/// 而 `headers.get(COOKIE)` 只能拿到第一个，其余 Cookie 会被整体忽略。
/// 典型表现：浏览器 Cookie 列表里明明有 `zap_token`，服务端却报告「没有 Cookie」，
/// 且 Web 应用自身的会话 Cookie 也传不进 PHP（表现为会话无法建立）。
pub fn all_cookies(headers: &axum::http::HeaderMap) -> Option<String> {
    let mut out: Option<String> = None;
    for value in headers.get_all(header::COOKIE) {
        if let Ok(s) = value.to_str() {
            match &mut out {
                Some(buf) => {
                    buf.push_str("; ");
                    buf.push_str(s);
                }
                None => out = Some(s.to_string()),
            }
        }
    }
    out
}

/// 登录接口同时下发会话 Cookie（`auth::SESSION_COOKIE`），页面据此鉴权。
pub fn token_candidates_from_page_request(req: &Request) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(t) = token_from_request(req) {
        out.push(t);
    }
    if let Some(cookie) = all_cookies(req.headers()) {
        for part in cookie.split(';') {
            if let Some((k, v)) = part.split_once('=') {
                let v = v.trim();
                if !v.is_empty() && k.trim() == crate::routers::auth::SESSION_COOKIE {
                    out.push(v.to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        middleware,
        routing::{get, post},
    };
    use tower::ServiceExt; // oneshot

    fn app() -> Router {
        Router::new()
            .route("/health", get(|| async { "ok" }))
            .route("/system/config/time", get(|| async { "ok" }))
            .route("/system/user/list", get(|| async { "ok" }))
            .route("/appstore/ws/{run_id}", get(|| async { "ok" }))
            .route("/site/list", get(|| async { "ok" }))
            .route("/site/add", post(|| async { "ok" }))
            .route("/site/update", post(|| async { "ok" }))
            .route("/site/delete", post(|| async { "ok" }))
            .route("/user/team/add", post(|| async { "ok" }))
            .layer(middleware::from_fn(guard))
    }

    async fn status(uri: &str, token: Option<&str>) -> StatusCode {
        send(Method::GET, uri, token).await
    }

    async fn send(method: Method, uri: &str, token: Option<&str>) -> StatusCode {
        let mut req = axum::http::Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        app()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    fn token(roles: &str) -> String {
        token_with(9, roles)
    }

    fn token_with(uid: u64, roles: &str) -> String {
        jwt::generate_jwt_token("tester".to_string(), uid, roles, false).unwrap()
    }

    /// 预置权限缓存（测试进程内共享；内容固定，重复写入等价，不会互相干扰）。
    fn prime_cache() {
        let mut map = PermMap::new();
        map.insert(
            "user".to_string(),
            ["site:view", "site:create", "appstore:log"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        map.insert(
            "reseller".to_string(),
            ["system.user:view", "site:view"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        // uid=100 是"个人附加权限"用例：角色没有任何权限，仅靠附加权限放行
        let mut users = UserPermMap::new();
        users.insert(100, ["site:delete"].iter().map(|s| s.to_string()).collect());
        // uid=200 是"只读"用例：生效权限已被收敛为 `{ns}:view`（见 load_user_perm_map）
        users.insert(200, ["site:view"].iter().map(|s| s.to_string()).collect());

        // 固定内容，直接覆盖：保证并发测试读到的是同一份数据
        *cache_slot().write().unwrap() = Some(Arc::new(map));
        *user_cache_slot().write().unwrap() = Some(Arc::new(users));
        *readonly_slot().write().unwrap() =
            Some(Arc::new([200i64].into_iter().collect::<ReadOnlySet>()));
    }

    /// 只读账号：共享可见（角色下限与权限点照常给 `view`），但一处都改不了。
    ///
    /// 关键是**角色不能兜底**：user 角色默认持有 `site:create` / `site:update`，
    /// 只读账号也是 user 角色，若判定取「角色 ∪ 用户」并集就会被整体绕过。
    #[tokio::test]
    async fn readonly_account_sees_everything_but_changes_nothing() {
        prime_cache();
        let readonly = token_with(200, "user");

        // 看：与资源归属无关，自己 / 父账号共享的都一样放行
        assert_eq!(status("/site/list", Some(&readonly)).await, StatusCode::OK);
        assert_eq!(status("/health", Some(&readonly)).await, StatusCode::OK);

        // 改：角色持有的 create / update / delete 一律不生效
        assert_eq!(
            send(Method::POST, "/site/add", Some(&readonly)).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(Method::POST, "/site/update", Some(&readonly)).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(Method::POST, "/site/delete", Some(&readonly)).await,
            StatusCode::FORBIDDEN
        );

        // 没登记权限点的写接口（成员管理）由只读兜底拦住
        assert_eq!(
            send(Method::POST, "/user/team/add", Some(&readonly)).await,
            StatusCode::FORBIDDEN
        );

        // 对照组：同样角色、非只读的账号照常可写
        assert_eq!(
            send(Method::POST, "/site/add", Some(&token("user"))).await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn middleware_enforces_perm_keys() {
        prime_cache();
        let admin = token("admin");
        let user = token("user");
        let reseller = token("reseller");
        let demo = token("demo");

        // admin 恒直通（即使未配置任何权限点）
        assert_eq!(status("/site/list", Some(&admin)).await, StatusCode::OK);

        // 持有 site:view → 放行；未持有（demo）→ 拒绝，即便角色下限已满足
        assert_eq!(status("/site/list", Some(&user)).await, StatusCode::OK);
        assert_eq!(
            status("/site/list", Some(&demo)).await,
            StatusCode::FORBIDDEN
        );

        // 写操作按具体动作校验：user 有 site:create，reseller 只有 site:view
        assert_eq!(
            send(Method::POST, "/site/add", Some(&user)).await,
            StatusCode::OK
        );
        assert_eq!(
            send(Method::POST, "/site/add", Some(&reseller)).await,
            StatusCode::FORBIDDEN
        );
        // 同一模块的不同动作互不影响：有 create 不等于有 delete
        assert_eq!(
            send(Method::POST, "/site/delete", Some(&user)).await,
            StatusCode::FORBIDDEN
        );

        // 个人附加权限：demo 角色本身无任何权限，靠 user.permissions 单独放行
        let granted = token_with(100, "demo");
        assert_eq!(
            send(Method::POST, "/site/delete", Some(&granted)).await,
            StatusCode::OK
        );
        // 附加权限只覆盖已授予的那一个动作
        assert_eq!(
            send(Method::POST, "/site/add", Some(&granted)).await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn middleware_enforces_matrix() {
        prime_cache();
        let admin = token("admin");
        let reseller = token("reseller");
        let user = token("user");
        let demo = token("demo");

        // 免鉴权
        assert_eq!(status("/health", None).await, StatusCode::OK);

        // 系统级接口：无凭据 401，非 admin 403，admin 放行
        assert_eq!(
            status("/system/config/time", None).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status("/system/config/time", Some(&user)).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status("/system/config/time", Some(&reseller)).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status("/system/config/time", Some(&demo)).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status("/system/config/time", Some(&admin)).await,
            StatusCode::OK
        );

        // reseller 层级：reseller 放行、普通用户拒绝
        assert_eq!(
            status("/system/user/list", Some(&reseller)).await,
            StatusCode::OK
        );
        assert_eq!(
            status("/system/user/list", Some(&user)).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status("/system/user/list", Some(&admin)).await,
            StatusCode::OK
        );

        // 伪造/过期 token 视为未登录
        assert_eq!(
            status("/system/config/time", Some("not-a-token")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    /// 浏览器 WebSocket 不能自定义请求头，token 只能放 query：必须与 Bearer 头等价。
    #[tokio::test]
    async fn websocket_query_token_is_accepted() {
        prime_cache();
        let user = token("user");
        assert_eq!(
            status("/appstore/ws/abc", None).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(&format!("/appstore/ws/abc?token={user}"), None).await,
            StatusCode::OK
        );
        assert_eq!(
            status("/appstore/ws/abc?token=bad", None).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn public_and_personal_paths() {
        assert_eq!(required_for("/health"), Required::Public);
        assert_eq!(required_for("/auth/login"), Required::Public);
        assert_eq!(required_for("/auth/logout"), Required::User);
        assert_eq!(required_for("/auth/totp/setup"), Required::User);
        assert_eq!(required_for("/user/info"), Required::User);
        assert_eq!(required_for("/user/notices/read"), Required::User);
    }

    /// 镜像构建相关接口按「更长前缀覆盖」把角色下限放宽到普通用户，
    /// 但容器 / 卷 / 网络 / Compose 仍须 admin：否则多用户面板里等于送人宿主机 root。
    #[test]
    fn image_build_paths_relax_to_user() {
        assert_eq!(required_for("/docker/image/build"), Required::User);
        assert_eq!(required_for("/docker/image/inspect"), Required::User);
        assert_eq!(required_for("/docker/images"), Required::User);
        assert_eq!(required_for("/docker/status"), Required::User);
        // 容器只读视图 / 启停：登记了权限点，可按需授予给普通用户与成员
        assert_eq!(required_for("/docker/containers"), Required::User);
        assert_eq!(required_for("/docker/stats"), Required::User);
        assert_eq!(required_for("/docker/container/inspect"), Required::User);
        assert_eq!(required_for("/docker/container/logs"), Required::User);
        assert_eq!(required_for("/docker/container/action"), Required::User);
        // 落在 /docker 模块规则上、仍然只有 admin 的：建容器（可挂宿主目录）/
        // 删镜像 / 卷 / 网络 / Compose —— 那些等于直接动宿主机
        assert_eq!(required_for("/docker/container/run"), Required::Admin);
        assert_eq!(required_for("/docker/image/action"), Required::Admin);
        assert_eq!(required_for("/docker/volumes"), Required::Admin);
        assert_eq!(required_for("/docker/network/action"), Required::Admin);
        assert_eq!(required_for("/docker/compose/action"), Required::Admin);
    }

    /// 容器权限点的边界：默认一律不给，靠管理员显式勾选；`build` 自带 `view`。
    #[test]
    fn docker_container_perms_are_explicit() {
        assert_eq!(
            perm_key_for("/docker/containers", &Method::GET),
            Some("docker:view".to_string())
        );
        assert_eq!(
            perm_key_for("/docker/container/logs", &Method::GET),
            Some("docker:view".to_string())
        );
        assert_eq!(
            perm_key_for("/docker/container/action", &Method::POST),
            Some("docker:manage".to_string())
        );
        // 默认角色拿不到：必须在「角色权限」/「附加权限点」里显式勾
        for role in ["user", "reseller", "demo"] {
            let perms = default_permissions_for(role);
            assert!(
                !perms.contains(&"docker:view".to_string()),
                "{role} 不该默认拥有 docker:view"
            );
            assert!(
                !perms.contains(&"docker:manage".to_string()),
                "{role} 不该默认拥有 docker:manage"
            );
        }
        // 构建者自带查看（build ⊃ view），不必再勾一次；但启停不跟着送
        let set = std::collections::HashSet::from(["docker:build".to_string()]);
        assert!(perm_satisfied(&set, "docker:view"));
        assert!(!perm_satisfied(&set, "docker:manage"));
    }

    /// `docker:build` 需要显式勾选：内置角色初始化时不自动带上（admin 除外）。
    #[test]
    fn explicit_only_perms_are_not_default() {
        assert!(default_permissions_for("admin").contains(&"docker:build".to_string()));
        for role in ["user", "reseller", "demo"] {
            assert!(
                !default_permissions_for(role).contains(&"docker:build".to_string()),
                "{role} 不该默认拥有 docker:build"
            );
        }
    }

    #[test]
    fn system_paths_default_to_admin() {
        for p in [
            "/system/config/time",
            "/system/config/time/sync",
            "/system/config/time/timezone",
            "/system/config/network/hostname",
            "/system/config/network/resolver",
            "/system/config/ssh/restart",
            "/system/config/ssh/install",
            "/system/config/services",
            "/system/config/services/action",
            "/system/config/processes",
            "/system/config/processes/kill",
            "/system/config/basic",
            "/system/config/zap",
            "/system/config/firewall/rule/add",
            "/system/job/start",
            "/system/job/stop",
            "/system/status",
            "/system/overview",
            "/system/env",
            "/system/cron/add",
            "/system/nginx/config/save",
            "/system/service-conf/save",
            "/system/ip/add",
            "/system/role/add",
            "/system/audit/list",
            "/system/update/apply",
            "/system/migrate/home",
            "/system/menus/add",
        ] {
            assert_eq!(required_for(p), Required::Admin, "{p} 必须为 admin 专属");
        }
    }

    #[test]
    fn explicitly_relaxed_paths() {
        assert_eq!(required_for("/system/info"), Required::User);
        assert_eq!(required_for("/system/files/list"), Required::User);
        assert_eq!(required_for("/system/menus/tree"), Required::User);
        assert_eq!(required_for("/system/user/list"), Required::Reseller);
        assert_eq!(required_for("/system/package/add"), Required::Reseller);
        assert_eq!(required_for("/system/fpm-specs/list"), Required::Reseller);
        // 更具体的条目覆盖较短前缀
        assert_eq!(required_for("/system/user/resellers"), Required::Admin);
        assert_eq!(required_for("/system/user/home_sync"), Required::Admin);
        assert_eq!(required_for("/system/fpm-specs/add"), Required::Admin);
    }

    /// 用户级菜单例外（user_menus）挂在 `/user` 前缀下：
    /// 父账号要在「团队成员」页给成员配菜单，普通用户必须能调用；
    /// 越权由 handler 内的 owner 收敛拦，因此不挂权限点；
    /// 写操作未登记 → 只读账号被兜底拦住。
    #[test]
    fn user_extra_menu_paths() {
        assert_eq!(required_for("/user/menus"), Required::User);
        assert_eq!(required_for("/user/menus/set"), Required::User);
        assert_eq!(perm_key_for("/user/menus", &Method::GET), None);
        assert_eq!(perm_key_for("/user/menus/set", &Method::POST), None);
        assert!(!readonly_allows("/user/menus/set", &Method::POST));
    }

    #[test]
    fn business_paths() {
        assert_eq!(required_for("/site/list"), Required::User);
        assert_eq!(required_for("/site/users"), Required::Reseller);
        assert_eq!(required_for("/site/sync_all"), Required::Reseller);
        assert_eq!(required_for("/ssl/cert/list"), Required::User);
        assert_eq!(required_for("/terminal/ws/1"), Required::User);
        assert_eq!(required_for("/appstore/packages"), Required::User);
        assert_eq!(required_for("/appstore/repos/add"), Required::Admin);
        assert_eq!(required_for("/appstore/script/run"), Required::Admin);
        assert_eq!(required_for("/appstore/scripts/tree"), Required::Admin);
        assert_eq!(required_for("/dev/api-token/create"), Required::Admin);
        // 云存储：配置与桶内对象同属用户级（各自只看自己的目录）
        assert_eq!(required_for("/system/cloud/stores"), Required::User);
        assert_eq!(required_for("/system/cloud/download"), Required::User);
    }

    /// 路由表必须与权限矩阵对齐：`.route()` 里出现的每一条都得有 RULES 前缀命中。
    ///
    /// 漏登记的后果是**静默**的 —— `lookup` 返回默认 Admin 下限，管理员自测一切正常，
    /// 普通用户 / 成员一进对应页面就吃「权限不足，该操作需要更高角色权限」。
    #[test]
    fn every_route_has_an_access_rule() {
        let src = include_str!("mod.rs");
        let mut rest = src;
        let mut checked = 0;
        while let Some(idx) = rest.find(".route(\"") {
            let tail = &rest[idx + ".route(\"".len()..];
            let Some(end) = tail.find('"') else { break };
            let mut path = &tail[..end];
            // 与 `normalize` 一致：剥掉可选的前缀与 /api
            if let Some(r) = path.strip_prefix(&crate::config::url_prefix_path()) {
                path = if r.is_empty() { "/" } else { r };
            }
            if let Some(r) = path.strip_prefix("/api") {
                path = if r.is_empty() { "/" } else { r };
            }
            assert!(
                RULES.iter().any(|(prefix, _, _)| prefix_hit(path, prefix)),
                "{path} 未在权限矩阵登记，会默认落到 Admin 下限（普通用户 403）"
            );
            checked += 1;
            rest = &tail[end..];
        }
        assert!(checked > 200, "路由解析失败，只扫到 {checked} 条");
    }

    #[test]
    fn cloud_paths_map_to_perms() {
        // `/system/cloud/stores` 与 `/system/cloud/store/*` 只差一个字母，
        // 前缀匹配不能串门，这里一并锁住。
        assert_eq!(
            perm_key_for("/system/cloud/stores", &Method::GET).as_deref(),
            Some("system.cloud:view")
        );
        assert_eq!(
            perm_key_for("/system/cloud/test", &Method::GET).as_deref(),
            Some("system.cloud:view")
        );
        assert_eq!(
            perm_key_for("/system/cloud/list", &Method::GET).as_deref(),
            Some("system.cloud:view")
        );
        assert_eq!(
            perm_key_for("/system/cloud/download", &Method::GET).as_deref(),
            Some("system.cloud:view")
        );
        assert_eq!(
            perm_key_for("/system/cloud/store/save", &Method::POST).as_deref(),
            Some("system.cloud:write")
        );
        assert_eq!(
            perm_key_for("/system/cloud/mkdir", &Method::POST).as_deref(),
            Some("system.cloud:write")
        );
        assert_eq!(
            perm_key_for("/system/cloud/upload", &Method::POST).as_deref(),
            Some("system.cloud:write")
        );
        assert_eq!(
            perm_key_for("/system/cloud/rename", &Method::POST).as_deref(),
            Some("system.cloud:write")
        );
        assert_eq!(
            perm_key_for("/system/cloud/local/list", &Method::GET).as_deref(),
            Some("system.cloud:view")
        );
        assert_eq!(
            perm_key_for("/system/cloud/upload-local", &Method::POST).as_deref(),
            Some("system.cloud:write")
        );
        assert_eq!(
            perm_key_for("/system/cloud/delete", &Method::POST).as_deref(),
            Some("system.cloud:delete")
        );
        assert_eq!(
            perm_key_for("/system/cloud/store/delete", &Method::POST).as_deref(),
            Some("system.cloud:delete")
        );
    }

    #[test]
    fn perm_key_maps_to_action() {
        // 站点：读 / 建 / 改 / 删 / 启停 / 同步 各自独立
        assert_eq!(
            perm_key_for("/site/list", &Method::GET).as_deref(),
            Some("site:view")
        );
        assert_eq!(
            perm_key_for("/site/add", &Method::POST).as_deref(),
            Some("site:create")
        );
        assert_eq!(
            perm_key_for("/site/update", &Method::POST).as_deref(),
            Some("site:update")
        );
        assert_eq!(
            perm_key_for("/site/delete", &Method::POST).as_deref(),
            Some("site:delete")
        );
        assert_eq!(
            perm_key_for("/site/state", &Method::POST).as_deref(),
            Some("site:state")
        );
        assert_eq!(
            perm_key_for("/site/sync_all", &Method::POST).as_deref(),
            Some("site:sync")
        );
        // POST 但只读的端点按 view 授权
        assert_eq!(
            perm_key_for("/ssl/cert/parse", &Method::POST).as_deref(),
            Some("ssl:view")
        );
        assert_eq!(
            perm_key_for("/site/dirs", &Method::POST).as_deref(),
            Some("site:view")
        );
        // GET 但会改系统状态的端点显式声明为 edit
        assert_eq!(
            perm_key_for("/system/job/start", &Method::GET).as_deref(),
            Some("system.job:edit")
        );
        // 高危动作从 edit 中拆出
        assert_eq!(
            perm_key_for("/system/config/processes/kill", &Method::POST).as_deref(),
            Some("system.config:process")
        );
        assert_eq!(
            perm_key_for("/system/config/ssh/restart", &Method::POST).as_deref(),
            Some("system.config:ssh")
        );
        // 未拆动作的模块仍按方法派生
        assert_eq!(
            perm_key_for("/system/cron/list", &Method::GET).as_deref(),
            Some("system.cron:view")
        );
        assert_eq!(
            perm_key_for("/system/cron/add", &Method::POST).as_deref(),
            Some("system.cron:edit")
        );
        // 个人接口不设权限点
        assert_eq!(perm_key_for("/user/info", &Method::GET), None);
        assert_eq!(perm_key_for("/health", &Method::GET), None);
    }

    #[test]
    fn seed_permissions_match_role_floor() {
        let admin = default_permissions_for("admin");
        let reseller = default_permissions_for("reseller");
        let user = default_permissions_for("user");
        let demo = default_permissions_for("demo");

        // admin 拿到全部权限点
        assert!(admin.contains(&"system.config:edit".to_string()));
        assert_eq!(admin.len(), all_perm_keys().len());

        // 普通用户：站点全部动作 / 文件 / 商店可用，服务器配置不可用
        assert!(user.contains(&"site:view".to_string()));
        assert!(user.contains(&"site:create".to_string()));
        assert!(user.contains(&"site:delete".to_string()));
        assert!(user.contains(&"system.file:view".to_string()));
        assert!(!user.contains(&"system.config:view".to_string()));
        assert!(!user.contains(&"system.user:view".to_string()));
        assert_eq!(user, demo);
        // admin-only 动作不会出现在普通用户清单里（逐条规则推导，而非整包授予）
        assert!(!user.contains(&"appstore:retry".to_string()));
        assert!(!user.contains(&"system.menu:create".to_string()));

        // reseller：在普通用户之上追加用户/套餐管理
        assert!(reseller.contains(&"site:view".to_string()));
        assert!(reseller.contains(&"system.user:view".to_string()));
        assert!(reseller.contains(&"system.package:edit".to_string()));
        assert!(!reseller.contains(&"system.config:view".to_string()));
    }

    #[test]
    fn catalog_covers_all_namespaces() {
        let groups = permission_catalog();
        let site = groups
            .iter()
            .find(|g| g.ns == "site")
            .expect("缺少 site 模块");
        assert_eq!(site.label, "站点管理");
        // 站点已拆到动作级
        let keys: Vec<&str> = site.actions.iter().map(|a| a.key.as_str()).collect();
        for k in [
            "site:view",
            "site:create",
            "site:update",
            "site:delete",
            "site:state",
            "site:sync",
        ] {
            assert!(keys.contains(&k), "site 缺少动作 {k}");
        }
        assert!(groups.iter().any(|g| g.ns == "system.config"));
        // 动作 key 唯一且都带中文名
        let mut seen: HashSet<String> = HashSet::new();
        for g in &groups {
            for a in &g.actions {
                assert!(seen.insert(a.key.clone()), "重复动作 {}", a.key);
                assert!(!a.label.is_empty());
            }
        }
    }

    #[test]
    fn unknown_paths_are_admin_only() {
        // 新增接口未登记时的兜底行为：默认拒绝
        assert_eq!(required_for("/some/new/endpoint"), Required::Admin);
    }

    /// 只读账号在**没登记权限点**的接口上的兜底：只放行个人账户操作。
    #[test]
    fn readonly_accounts_cannot_write_unregistered_paths() {
        // 成员管理没有权限点可校验，只读账号不能借这个口子写
        for path in ["/user/team/add", "/user/team/update", "/user/team/delete"] {
            assert!(
                !readonly_allows(path, &Method::POST),
                "{path} 不该对只读账号放行"
            );
        }
        // 个人账户操作放行：否则只读账号改不了自己的密码 / 2FA
        for path in [
            "/auth/change_password",
            "/auth/totp/verify",
            "/user/notices/read",
        ] {
            assert!(readonly_allows(path, &Method::POST), "{path} 应当放行");
        }
        // 读取一律放行；Web 终端例外（GET 但连上就是交互式 shell）
        assert!(readonly_allows("/site/list", &Method::GET));
        assert!(readonly_allows("/system/files/list", &Method::GET));
        assert!(!readonly_allows("/terminal/ws/1", &Method::GET));
    }

    #[test]
    fn normalize_strips_prefix() {
        assert_eq!(normalize("/api/health"), "/health");
        assert_eq!(normalize("/api/system/config/time"), "/system/config/time");
        assert_eq!(normalize("/system/config/time"), "/system/config/time");
        assert_eq!(normalize("/api"), "/");
    }
}
