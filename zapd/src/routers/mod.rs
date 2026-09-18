use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Request},
    http::{Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rust_embed::RustEmbed;
use serde_json::json;

use crate::zap::jwt::{claims_from_token, is_demo};

/// 演示账号只读守卫：demo 角色仅允许 GET 请求（浏览），其余写操作一律拒绝。
/// /auth/* 为个人账户操作（登录/登出/改密/2FA），放行以免演示账号被锁死。
#[allow(clippy::result_large_err)] // axum 中间件约定 Result<Response, Response>
async fn demo_readonly_guard(req: Request, next: Next) -> Result<Response, Response> {
    if req.method() == Method::GET || req.uri().path().starts_with("/auth/") {
        return Ok(next.run(req).await);
    }
    // 先拷贝 token 再异步解析（避免借用 req 跨 await）
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(String::from);
    if let Some(token) = bearer
        && let Some(claims) = claims_from_token(&token).await
        && is_demo(&claims)
    {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "code": -1, "message": "演示账号仅支持浏览，不能执行操作" })),
        )
            .into_response());
    }
    Ok(next.run(req).await)
}

pub mod access;
pub mod appstore;
pub mod auth;
pub mod cloud;
pub mod dashboard;
pub mod database;
pub mod dev;
pub mod docker;
pub mod docs;
pub mod fpm_spec;
pub mod notice;
pub mod package;
pub mod site;
pub mod ssh_terminal;
pub mod ssh_user_keys;
pub mod ssl;
pub mod system_audit;
pub mod system_basic;
pub mod system_config;
pub mod system_cron;
pub mod system_env;
pub mod system_file;
pub mod system_firewall;
pub mod system_info;
pub mod system_ip;
pub mod system_job;
pub mod system_menu;
pub mod system_migrate;
pub mod system_nginx;
pub mod system_role;
pub mod system_service_conf;
pub mod system_update;
pub mod system_zap;
pub mod user;
pub mod user_cron;
pub mod webapps;

#[derive(RustEmbed)]
#[folder = "../web/dist/"]
struct Assets;

static INDEX_HTML: &str = "index.html";

/// 云存储上传的请求体上限（4 GiB）。
///
/// 上传走「边收边写」的流式写入，内存占用与文件大小无关，因此这里只用于
/// 挡住明显异常的请求（axum 默认上限是 2 MB，对云存储来说太小）。
const CLOUD_UPLOAD_LIMIT: usize = 4 * 1024 * 1024 * 1024;

async fn index_html() -> Response {
    match Assets::get(INDEX_HTML) {
        Some(content) => {
            let html = String::from_utf8_lossy(&content.data).into_owned();
            let html = inject_base_tag(&html, &crate::config::url_prefix_path());
            Response::builder()
                .header(header::CONTENT_TYPE, "text/html")
                .body(Body::from(html))
                .unwrap()
        }
        None => not_found().await,
    }
}

/// 向 SPA 首页注入 `<base>` 与 URL 前缀。
///
/// 前端构建产物用相对路径引用资源（vite `base: './'`），因此**必须**注入 `<base>`：
/// 否则在多级路由（如 `/system/users`、`/zap/system/users`）直接刷新时，
/// `./assets/xxx.js` 会被解析成 `/system/assets/xxx.js` 而 404，页面一片空白。
///
/// - 未启用前缀：注入 `<base href="/">`
/// - 启用前缀：注入 `<base href="/zap/">`
///
/// 同时注入 `window.__ZAP_BASE__`（无前缀时为空串），供前端 axios baseURL、
/// vue-router base 与 WebSocket 使用。
fn inject_base_tag(html: &str, prefix_path: &str) -> String {
    let base = if prefix_path.is_empty() {
        "/".to_string()
    } else {
        format!("{prefix_path}/")
    };
    let inject = format!(
        "\n    <base href=\"{base}\">\n    <script>window.__ZAP_BASE__=\"{prefix_path}\"</script>"
    );
    match html.find("<head>") {
        Some(pos) => {
            let at = pos + "<head>".len();
            let (head, tail) = html.split_at(at);
            format!("{head}{inject}{tail}")
        }
        None => format!("{inject}{html}"),
    }
}

async fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("404"))
        .unwrap()
}

async fn static_handler(uri: Uri) -> Response {
    let mut full = uri.path().to_string();
    // `Router::nest` 会剥离前缀，这里再幂等剥一次：
    // 保证无论拿到的是完整路径还是剥离后的路径都能命中文件。
    let prefix = crate::config::url_prefix_path();
    if !prefix.is_empty() {
        if full == prefix {
            full = "/".to_string();
        } else if let Some(rest) = full.strip_prefix(&format!("{prefix}/")) {
            full = format!("/{rest}");
        }
    }
    let path = full.trim_start_matches('/');

    if path.is_empty() || path == INDEX_HTML {
        return index_html().await;
    }

    match Assets::get(path) {
        Some(content) => {
            let body = Body::from(content.data);
            let mime = mime_guess::from_path(path).first_or_octet_stream();

            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(body)
                .unwrap()
        }
        None => {
            if path.contains('.') {
                return not_found().await;
            }

            index_html().await
        }
    }
}

/// Health check endpoint — no auth required
async fn health_check() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "timestamp": chrono::Local::now().timestamp(),
    }))
}

/// 组装完整路由：前缀取自 `zap.yaml` 的 `server.url_prefix`。
pub fn routers() -> Router {
    build_routers(&crate::config::url_prefix())
}

/// 按给定前缀组装路由（`prefix` 为空表示不启用前缀）。
///
/// 配置 `server.url_prefix` 后，页面与接口全部挂到 `/{prefix}` 下：
/// - 页面：`/zap/dashboard`
/// - 接口：`/zap/api/auth/login`
///
/// 前缀之外的路径（`/`、`/api/*` 等）一律返回 **404**，不做重定向：
/// 这样外部探测根路径时无法发现真实入口，起到隐藏后台入口的作用。
///
/// 未配置前缀时行为与之前完全一致（`/api/*` + 根路径 SPA）。
fn build_routers(prefix: &str) -> Router {
    let inner = Router::new()
        // Web 应用（/webapps/*，页面级路由，自带鉴权）
        .nest("/webapps", webapps::routers())
        .fallback(static_handler)
        .nest("/api", api_routers());

    let prefix = prefix.trim().trim_matches('/');
    if prefix.is_empty() {
        return inner;
    }

    let nested = format!("/{prefix}");
    // nest 的 catch-all 能匹配 /zap 与 /zap/xxx，但匹配不到 /zap/（尾斜杠），
    // 补一条显式路由，保证 /zap 与 /zap/ 两种写法都能打开首页。
    let nested_slash = format!("{nested}/");
    Router::new()
        .nest(&nested, inner)
        .route(&nested_slash, get(index_html))
        // 前缀之外的任何路径都 404（不回跳，避免暴露前缀）
        .fallback(not_found)
}

fn api_routers() -> Router {
    Router::new()
        // Health
        .route("/health", get(health_check))
        // Auth
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", get(auth::logout))
        .route("/auth/reflash_token", post(auth::reflash_token))
        .route("/auth/change_password", post(auth::change_password))
        // TOTP 2FA
        .route("/auth/totp/setup", get(auth::totp_setup))
        .route("/auth/totp/verify", post(auth::totp_verify))
        .route("/auth/totp/disable", post(auth::totp_disable))
        .route("/auth/totp/status", get(auth::totp_status))
        .route("/user/info", get(user::user_info))
        .route(
            "/user/prefs",
            get(user::user_prefs_get).post(user::user_prefs_save),
        )
        // 站内信（通知中心，登录用户本人）
        .route("/user/notices", get(notice::notices_list))
        .route("/user/notices/unread", get(notice::notices_unread))
        .route("/user/notices/read", post(notice::notices_read))
        .route("/user/notices/read_all", post(notice::notices_read_all))
        .route("/user/notices/delete", post(notice::notices_delete))
        // User management (admin + reseller)
        .route("/system/user/list", get(user::user_list))
        .route("/system/user/add", post(user::user_add))
        .route("/system/user/update", post(user::user_update))
        .route("/system/user/delete", post(user::user_delete))
        .route("/system/user/resellers", get(user::reseller_list))
        .route("/system/user/home_sync", post(user::user_home_sync))
        // 仪表盘统计卡片（按角色可见范围统计用户 / 站点 / 数据库数量）
        .route("/dashboard/counts", get(dashboard::counts))
        // 套餐（Packages）：创建客户时选择的资源套餐
        .route("/system/package/list", get(package::package_list))
        .route("/system/package/add", post(package::package_add))
        .route("/system/package/update", post(package::package_update))
        // 数据库管理（MySQL / MariaDB，zapadm 凭据 + 本机 mysql 客户端）
        .route("/database/status", get(database::status))
        .route("/database/list", get(database::list))
        .route("/database/create", post(database::create))
        .route("/database/drop", post(database::drop_db))
        .route("/database/users", get(database::users))
        .route("/database/user/create", post(database::user_create))
        .route("/database/user/drop", post(database::user_drop))
        .route("/database/remote", get(database::remote_list))
        .route("/database/remote/grant", post(database::remote_grant))
        .route("/database/remote/revoke", post(database::remote_revoke))
        .route("/system/package/delete", post(package::package_delete))
        // Role management (admin only)
        .route("/system/role/list", get(system_role::role_list))
        .route("/system/role/add", post(system_role::role_add))
        .route("/system/role/update", post(system_role::role_update))
        .route("/system/role/delete", post(system_role::role_delete))
        .route(
            "/system/role/permissions",
            get(system_role::role_permissions_get),
        )
        .route(
            "/system/role/permissions/set",
            post(system_role::role_permissions_set),
        )
        // 动作级权限点目录（角色权限配置页数据源，与 access 权限矩阵同源）
        .route(
            "/system/role/permission-catalog",
            get(system_role::permission_catalog),
        )
        // Menu management (admin only)
        .route("/system/menus/tree", get(system_menu::get_menus_tree))
        .route("/system/menus/list", get(system_menu::menu_list))
        .route("/system/menus/add", post(system_menu::menu_add))
        .route("/system/menus/update", post(system_menu::menu_update))
        .route("/system/menus/delete", post(system_menu::menu_delete))
        .route("/system/menus/status", post(system_menu::menu_status))
        // Server config (admin only)
        .route("/system/config/time", get(system_config::get_time))
        .route("/system/config/time/sync", post(system_config::sync_time))
        .route(
            "/system/config/time/timezone",
            post(system_config::set_timezone),
        )
        .route(
            "/system/config/time/timezones",
            get(system_config::list_timezones),
        )
        // 防火墙设置（服务器配置 → 防火墙，admin only）
        .route(
            "/system/config/firewall",
            get(system_firewall::firewall_status),
        )
        .route(
            "/system/config/firewall/rule/add",
            post(system_firewall::firewall_rule_add),
        )
        .route(
            "/system/config/firewall/rule/delete",
            post(system_firewall::firewall_rule_delete),
        )
        .route(
            "/system/config/firewall/toggle",
            post(system_firewall::firewall_toggle),
        )
        .route("/system/config/network", get(system_config::network_get))
        .route(
            "/system/config/network/hostname",
            post(system_config::network_set_hostname),
        )
        .route(
            "/system/config/network/resolver",
            post(system_config::network_set_resolver),
        )
        // ── IP 池管理 ────────────────────────────────
        .route("/system/ip/list", get(system_ip::ip_list))
        .route("/system/ip/add", post(system_ip::ip_add))
        .route("/system/ip/delete", post(system_ip::ip_delete))
        .route("/system/ip/update", post(system_ip::ip_update))
        .route(
            "/system/ip/batch-reserved",
            post(system_ip::ip_batch_reserved),
        )
        .route("/system/config/ssh/status", get(system_config::ssh_status))
        .route(
            "/system/config/ssh/restart",
            post(system_config::ssh_restart),
        )
        .route(
            "/system/config/ssh/install",
            post(system_config::ssh_install),
        )
        .route(
            "/system/config/ssh/install/log/{run_id}",
            get(system_config::ssh_install_log),
        )
        // Nginx 服务配置（服务器配置 → Nginx 配置 / 服务器状态 → Nginx Server，admin only）
        .route("/system/nginx/status", get(system_nginx::nginx_status))
        .route("/system/nginx/config", get(system_nginx::nginx_conf_list))
        .route(
            "/system/nginx/config/content",
            get(system_nginx::nginx_conf_read),
        )
        .route(
            "/system/nginx/config/save",
            post(system_nginx::nginx_conf_save),
        )
        .route("/system/nginx/control", post(system_nginx::nginx_control))
        .route(
            "/system/nginx/default-vhost",
            post(system_nginx::nginx_default_vhost),
        )
        .route(
            "/system/nginx/stub-status",
            get(system_nginx::nginx_stub_status_get).post(system_nginx::nginx_stub_status_set),
        )
        // 通用服务配置（服务配置：php / mysql / mariadb / docker，admin only）
        .route(
            "/system/service-conf/status",
            get(system_service_conf::status),
        )
        .route(
            "/system/service-conf/list",
            get(system_service_conf::conf_list),
        )
        .route(
            "/system/service-conf/read",
            get(system_service_conf::conf_read),
        )
        .route(
            "/system/service-conf/save",
            post(system_service_conf::conf_save),
        )
        .route(
            "/system/service-conf/keys",
            get(system_service_conf::keys_get),
        )
        .route(
            "/system/service-conf/keys/save",
            post(system_service_conf::keys_save),
        )
        .route(
            "/system/service-conf/control",
            post(system_service_conf::control),
        )
        .route(
            "/system/service-conf/instances",
            get(system_service_conf::instances),
        )
        .route(
            "/system/service-conf/default",
            post(system_service_conf::set_default),
        )
        // 数据迁移（服务器配置 → 数据迁移，admin only）
        .route(
            "/system/migrate/users",
            get(system_migrate::migrate_users_preview),
        )
        .route(
            "/system/migrate/home",
            post(system_migrate::migrate_home_mv),
        )
        // 基础设置（系统设置 → 基础设置，admin only）
        .route(
            "/system/config/basic",
            get(system_basic::basic_get).post(system_basic::basic_save),
        )
        // Zap 设置（系统设置 → Zap 设置，admin only）：动态修改 zap.yaml 的 server.*
        .route(
            "/system/config/zap",
            get(system_zap::zap_get).post(system_zap::zap_save),
        )
        .route(
            "/system/config/zap/ssl/self-sign",
            post(system_zap::ssl_self_sign),
        )
        .route("/system/config/services", get(system_config::list_services))
        .route(
            "/system/config/services/action",
            post(system_config::service_action),
        )
        .route(
            "/system/config/processes",
            get(system_config::list_processes),
        )
        .route(
            "/system/config/processes/kill",
            post(system_config::process_kill),
        )
        // Server runtime env (运行环境状态表，admin only)
        .route("/system/env", get(system_env::env_get))
        .route("/system/env/refresh", post(system_env::env_refresh))
        .route("/system/env/defaults", post(system_env::env_defaults_save))
        // 脚本/自动化：计划任务（admin only）
        .route("/system/cron/list", get(system_cron::cron_list))
        .route("/system/cron/add", post(system_cron::cron_add))
        .route("/system/cron/update", post(system_cron::cron_update))
        .route("/system/cron/delete", post(system_cron::cron_delete))
        .route("/system/cron/toggle", post(system_cron::cron_toggle))
        .route("/system/cron/run_now", post(system_cron::cron_run_now))
        .route("/system/cron/runs", get(system_cron::cron_runs))
        .route(
            "/system/cron/runs_clear",
            post(system_cron::cron_runs_clear),
        )
        // PHP-FPM 规格模板库（admin 维护；reseller 可读自己名下 + 全局模板）
        .route("/system/fpm-specs/list", get(fpm_spec::spec_list))
        .route("/system/fpm-specs/add", post(fpm_spec::spec_add))
        .route("/system/fpm-specs/update", post(fpm_spec::spec_update))
        .route("/system/fpm-specs/delete", post(fpm_spec::spec_delete))
        // 容器管理（Docker Desktop 式单页：容器 / 镜像 / 卷 / 网络 / Compose，admin only）
        .route("/docker/status", get(docker::status))
        .route("/docker/containers", get(docker::containers))
        .route("/docker/stats", get(docker::stats))
        .route("/docker/container/inspect", get(docker::container_inspect))
        .route("/docker/container/logs", get(docker::container_logs))
        .route("/docker/container/action", post(docker::container_action))
        .route("/docker/images", get(docker::images))
        .route("/docker/image/action", post(docker::image_action))
        .route("/docker/volumes", get(docker::volumes))
        .route("/docker/volume/action", post(docker::volume_action))
        .route("/docker/networks", get(docker::networks))
        .route("/docker/network/action", post(docker::network_action))
        // 容器终端 / 守护事件流（WebSocket：浏览器不能带自定义头，token 走 query）
        .route("/docker/exec/ws", get(docker::ws_exec))
        .route("/docker/events/ws", get(docker::ws_events))
        .route("/docker/compose", get(docker::compose_list))
        .route("/docker/compose/action", post(docker::compose_action))
        // SSH terminal
        .route("/terminal/connections", get(ssh_terminal::list_connections))
        .route(
            "/terminal/connections/{id}",
            get(ssh_terminal::get_connection),
        )
        .route(
            "/terminal/connections/create",
            post(ssh_terminal::create_connection),
        )
        .route(
            "/terminal/connections/{id}/update",
            post(ssh_terminal::update_connection),
        )
        .route(
            "/terminal/connections/{id}/delete",
            post(ssh_terminal::delete_connection),
        )
        .route(
            "/terminal/connections/test",
            get(ssh_terminal::test_connection),
        )
        .route(
            "/terminal/connections/{id}/push-key",
            post(ssh_terminal::push_key_to_host),
        )
        .route("/terminal/push-key", post(ssh_terminal::push_key_direct))
        .route("/terminal/ws/{id}", get(ssh_terminal::ws_terminal))
        // 「我的 SSH 密钥」（面板用户自管密钥，存家目录 ~/.ssh；admin 列表额外含系统级密钥）
        .route("/terminal/keys", get(ssh_user_keys::list_keys))
        .route("/terminal/keys/public", get(ssh_user_keys::public_key))
        .route("/terminal/keys/private", get(ssh_user_keys::private_key))
        .route("/terminal/keys/generate", post(ssh_user_keys::generate_key))
        .route("/terminal/keys/import", post(ssh_user_keys::import_key))
        .route("/terminal/keys/delete", post(ssh_user_keys::delete_key))
        // 计划任务（crontab）：所有角色可管理自己的任务；执行身份由 handler 收敛
        .route("/terminal/crontab/list", get(user_cron::cron_list))
        .route(
            "/terminal/crontab/exec-users",
            get(user_cron::cron_exec_users),
        )
        .route("/terminal/crontab/add", post(user_cron::cron_add))
        .route("/terminal/crontab/update", post(user_cron::cron_update))
        .route("/terminal/crontab/delete", post(user_cron::cron_delete))
        .route("/terminal/crontab/toggle", post(user_cron::cron_toggle))
        .route("/terminal/crontab/run_now", post(user_cron::cron_run_now))
        .route("/terminal/crontab/log", get(user_cron::cron_log))
        .route("/terminal/crontab/runs", get(user_cron::cron_runs))
        .route(
            "/terminal/crontab/runs_clear",
            post(user_cron::cron_runs_clear),
        )
        .route(
            "/terminal/crontab/logs_purge",
            post(user_cron::cron_logs_purge),
        )
        // System
        .route("/system/info", get(system_info::system_info))
        .route("/system/status", get(system_info::system_status))
        .route("/system/overview", get(system_info::system_overview))
        .route("/system/about", get(system_info::about))
        .route("/system/job/stop", get(system_job::stop_job))
        .route("/system/job/start", get(system_job::start_job))
        // Audit logs (admin only)
        .route("/system/audit/list", get(system_audit::audit_list))
        // System update (系统设置 → 系统更新, admin only)
        .route("/system/update/status", get(system_update::status_get))
        .route("/system/update/config", post(system_update::config_save))
        .route("/system/update/check", post(system_update::check))
        .route("/system/update/apply", post(system_update::apply))
        .route("/system/update/log/{run_id}", get(system_update::log))
        // File manager
        .route("/system/files/list", get(system_file::file_list))
        .route("/system/files/read", get(system_file::file_read))
        .route("/system/files/write", post(system_file::file_write))
        .route("/system/files/delete", post(system_file::file_delete))
        .route("/system/files/mkdir", post(system_file::file_mkdir))
        .route("/system/files/rename", post(system_file::file_rename))
        .route("/system/files/chmod", post(system_file::file_chmod))
        .route("/system/files/chown", post(system_file::file_chown))
        .route("/system/files/copy", post(system_file::file_copy))
        .route("/system/files/archive", post(system_file::file_archive))
        .route("/system/files/download", get(system_file::file_download))
        .route("/system/files/upload", post(system_file::file_upload))
        .route("/system/files/info", get(system_file::file_info))
        // 云存储（多套配置 + 对象浏览/传输）
        .route("/system/cloud/stores", get(cloud::store_list))
        .route("/system/cloud/store/save", post(cloud::store_save))
        .route("/system/cloud/store/delete", post(cloud::store_delete))
        .route("/system/cloud/test", get(cloud::store_test))
        .route("/system/cloud/list", get(cloud::file_list))
        .route("/system/cloud/download", get(cloud::file_download))
        .route("/system/cloud/mkdir", post(cloud::file_mkdir))
        .route("/system/cloud/delete", post(cloud::file_delete))
        .route("/system/cloud/rename", post(cloud::file_rename))
        // 「从服务器上传」：浏览当前用户可访问的目录 + 直接把服务器文件传到云存储
        .route("/system/cloud/local/list", get(cloud::local_list))
        .route("/system/cloud/upload-local", post(cloud::file_upload_local))
        // 上传单独放开请求体上限（见 CLOUD_UPLOAD_LIMIT 注释）
        .route(
            "/system/cloud/upload",
            post(cloud::file_upload).layer(DefaultBodyLimit::max(CLOUD_UPLOAD_LIMIT)),
        )
        // AppStore（多 Git 源）
        .route("/appstore/repos", get(appstore::list_repos))
        .route("/appstore/repos/add", post(appstore::repo_add))
        .route("/appstore/repos/remove", post(appstore::repo_remove))
        .route("/appstore/repos/update", post(appstore::repo_update))
        .route("/appstore/packages", get(appstore::packages))
        .route("/appstore/install", post(appstore::install))
        .route("/appstore/uninstall", post(appstore::uninstall))
        .route("/appstore/upgrade", post(appstore::upgrade))
        .route("/appstore/installed", get(appstore::installed_apps))
        .route("/appstore/instance/action", post(appstore::instance_action))
        .route("/appstore/scripts/tree", get(appstore::scripts_tree))
        .route("/appstore/script/read", get(appstore::script_read))
        .route("/appstore/script/write", post(appstore::script_write))
        .route("/appstore/script/run", post(appstore::script_run))
        .route("/appstore/script/delete", post(appstore::script_delete))
        .route("/appstore/script/stop", post(appstore::script_stop))
        .route("/appstore/run/files", get(appstore::run_files))
        .route("/appstore/run/file/read", get(appstore::run_file_read))
        .route("/appstore/run/file/write", post(appstore::run_file_write))
        .route("/appstore/run/retry", post(appstore::run_retry))
        .route("/appstore/runs", get(appstore::runs))
        .route("/appstore/log/{run_id}", get(appstore::log))
        .route("/appstore/ws/{run_id}", get(appstore::ws_log))
        // 站点管理（admin 全部 / reseller 所属客户 / user 自己的站点）
        .route("/site/list", get(site::site_list))
        .route("/site/users", get(site::site_users))
        .route("/site/add", post(site::site_add))
        .route("/site/update", post(site::site_update))
        .route("/site/delete", post(site::site_delete))
        .route("/site/sync", post(site::site_sync))
        // 站点能力读取（普通用户能否用反代/自定义目录，依套餐而定）+ 已有目录浏览
        .route("/site/feature", get(site::site_feature))
        .route("/site/dirs", post(site::site_dirs_browse))
        // 站点启停 / 维护三态切换（running / stopped / maintenance）
        .route("/site/state", post(site::site_state))
        .route("/site/sync_all", post(site::site_sync_all))
        // 站点日志（查看 / 归档 / 清空 / 轮转）与流量分析
        .route("/site/logs", get(site::site_logs))
        .route("/site/logs/archives", get(site::site_logs_archives))
        .route("/site/logs/clear", post(site::site_logs_clear))
        .route("/site/logs/rotate", post(site::site_logs_rotate))
        .route("/site/traffic", get(site::site_traffic))
        // SSL/TLS：证书管理（手动导入 / 自签名 / Let's Encrypt）
        .route("/ssl/cert/list", get(ssl::cert_list))
        .route("/ssl/cert/detail", get(ssl::cert_detail))
        .route("/ssl/cert/parse", post(ssl::cert_parse))
        .route("/ssl/cert/add", post(ssl::cert_add))
        .route("/ssl/cert/update", post(ssl::cert_update))
        .route("/ssl/cert/delete", post(ssl::cert_delete))
        .route("/ssl/cert/self-sign", post(ssl::cert_self_sign))
        // SSL/TLS：Let's Encrypt 异步订单（下单 → 验证 → 签发）
        .route("/ssl/letsencrypt", post(ssl::cert_letsencrypt))
        .route("/ssl/letsencrypt/orders", get(ssl::letsencrypt_orders))
        .route("/ssl/letsencrypt/status", get(ssl::letsencrypt_status))
        .route("/ssl/letsencrypt/verify", post(ssl::letsencrypt_verify))
        .route("/ssl/letsencrypt/cancel", post(ssl::letsencrypt_cancel))
        // SSL/TLS：ACME 的 DNS 服务商凭据（DNS-01 自动验证）
        .route("/ssl/acme/dns/providers", get(ssl::acme_dns_providers))
        .route("/ssl/acme/dns/list", get(ssl::acme_dns_list))
        .route("/ssl/acme/dns/save", post(ssl::acme_dns_save))
        .route("/ssl/acme/dns/delete", post(ssl::acme_dns_delete))
        .route("/ssl/acme/dns/test", post(ssl::acme_dns_test))
        // 开发：API Token 管理 + API 文档
        .route("/dev/api-token/list", get(dev::api_token_list))
        .route("/dev/api-token/create", post(dev::api_token_create))
        .route("/dev/api-token/update", post(dev::api_token_update))
        .route("/dev/api-token/delete", post(dev::api_token_delete))
        .route("/dev/api-docs", get(dev::api_docs))
        // 文档（CHANGELOG / 用户手册 / FAQ / 升级指南）—— 登录即可读
        .route("/docs/list", get(docs::docs_list))
        .route("/docs/{name}", get(docs::docs_get))
        // 统一角色门禁（最后添加的 layer 最外层、最先执行）：
        // 路径 → 所需角色见 `access::RULES`，未登记的接口默认要求 admin。
        .layer(middleware::from_fn(demo_readonly_guard))
        .layer(middleware::from_fn(access::guard))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt; // oneshot

    async fn status_of(app: Router, uri: &str) -> StatusCode {
        get(app, uri).await.0
    }

    /// 返回（状态码，响应体文本），用于区分"命中接口"还是"落到 SPA fallback"
    async fn get(app: Router, uri: &str) -> (StatusCode, String) {
        let req = axum::http::Request::builder()
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    async fn no_prefix_keeps_legacy_paths() {
        let app = build_routers("");
        // 健康检查仍在 /api 下
        let (s, body) = get(app.clone(), "/api/health").await;
        assert_eq!(s, StatusCode::OK);
        assert!(body.contains("\"status\":\"ok\""), "应命中 health 接口");
        // 未启用前缀时 /zap/... 走 SPA fallback，不会命中接口
        let (_s, body) = get(app, "/zap/api/health").await;
        assert!(!body.contains("\"status\":\"ok\""), "前缀路径不应命中接口");
    }

    #[tokio::test]
    async fn prefix_moves_api_and_pages() {
        let app = build_routers("zap");
        // 接口搬到前缀下
        let (s, body) = get(app.clone(), "/zap/api/health").await;
        assert_eq!(s, StatusCode::OK);
        assert!(body.contains("\"status\":\"ok\""), "应命中 health 接口");
        // 旧路径不再可访问：一律 404（不重定向，避免暴露前缀）
        assert_eq!(
            status_of(app.clone(), "/api/health").await,
            StatusCode::NOT_FOUND
        );
        // 根路径同样 404
        assert_eq!(status_of(app.clone(), "/").await, StatusCode::NOT_FOUND);
        // 相近但不同的前缀也不应命中
        assert_eq!(
            status_of(app.clone(), "/zap2/api/health").await,
            StatusCode::NOT_FOUND
        );
        // 前缀根路径（含尾斜杠）应能打开首页
        assert_eq!(status_of(app.clone(), "/zap").await, StatusCode::OK);
        assert_eq!(status_of(app, "/zap/").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn prefix_is_normalized() {
        // 首尾斜杠与多级前缀都能正常工作
        assert_eq!(
            status_of(build_routers("/zap/"), "/zap/api/health").await,
            StatusCode::OK
        );
        assert_eq!(
            status_of(build_routers("a/b"), "/a/b/api/health").await,
            StatusCode::OK
        );
    }

    /// 真实路由必须被 `access::guard` 覆盖：
    /// 无凭据 → 401，普通用户 → 403（而非落到 handler 返回 400/200）。
    #[tokio::test]
    async fn api_routes_are_role_guarded() {
        let app = build_routers("");

        async fn call_with(app: &Router, uri: &str, token: Option<&str>) -> StatusCode {
            let mut req = axum::http::Request::builder().uri(uri);
            if let Some(t) = token {
                req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
            }
            let res = app
                .clone()
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap();
            res.status()
        }

        let user = crate::zap::jwt::generate_jwt_token("tester".to_string(), 9, "user", false)
            .expect("生成测试 token 失败");

        // 原先完全无门禁的系统配置端点
        for path in [
            "/api/system/config/time",
            "/api/system/config/ssh/restart",
            "/api/system/config/processes/kill",
            "/api/system/job/start",
        ] {
            assert_eq!(
                call_with(&app, path, None).await,
                StatusCode::UNAUTHORIZED,
                "{path} 未登录应拒绝"
            );
            assert_eq!(
                call_with(&app, path, Some(&user)).await,
                StatusCode::FORBIDDEN,
                "{path} 普通用户应拒绝"
            );
        }

        // 免鉴权接口不受影响
        assert_eq!(call_with(&app, "/api/health", None).await, StatusCode::OK);
    }

    #[test]
    fn base_tag_injection() {
        let html = "<html><head><title>x</title></head><body></body></html>";

        // 有前缀：注入 /zap/ 与全局变量
        let out = inject_base_tag(html, "/zap");
        assert!(out.contains(r#"<base href="/zap/">"#));
        assert!(out.contains(r#"window.__ZAP_BASE__="/zap""#));

        // 无前缀：也要注入 <base href="/">，否则多级路由刷新时资源路径会错
        let out = inject_base_tag(html, "");
        assert!(out.contains(r#"<base href="/">"#));
        assert!(out.contains(r#"window.__ZAP_BASE__="""#));

        // base 必须在页面资源引用之前（紧跟 <head>）
        let out = inject_base_tag(html, "/zap");
        let base_pos = out.find("<base").unwrap();
        let title_pos = out.find("<title>").unwrap();
        assert!(base_pos < title_pos, "base 标签必须在页面资源之前");
    }

    #[test]
    fn normalize_url_prefix_rules() {
        use crate::config::normalize_url_prefix;
        assert_eq!(normalize_url_prefix(""), "");
        assert_eq!(normalize_url_prefix("   "), "");
        assert_eq!(normalize_url_prefix("zap"), "zap");
        assert_eq!(normalize_url_prefix("/zap/"), "zap");
        assert_eq!(normalize_url_prefix("  /zap/  "), "zap");
        assert_eq!(normalize_url_prefix("a/b"), "a/b");
    }
}
