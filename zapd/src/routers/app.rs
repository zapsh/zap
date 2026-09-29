//! 站点应用管理（Application Manager）：部署 / 启停 / 删除 / 日志。
//!
//! 权限与配额都由套餐（packages）控制：
//! - `allow_apps`：总开关（默认关闭）；admin / reseller 恒可
//! - `app_types`：允许部署的类型白名单（`python,nodejs`），空 = 不限
//! - `max_apps`：每个站点的应用数量上限（0 = 不限）
//!
//! 进程本身由 zapexec 侧托管成一个 systemd unit，**以站点归属的 unix 用户运行**。

use axum::{
    Json,
    extract::{Extension, Query},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use tracing::info;

use crate::{
    db,
    zap::{
        ZapError, ZapJsonResult, audit,
        jwt::{self, ValidatedClaims},
    },
};
use zap_proto::{APP_TYPES, LocationSpec, Request};

use super::{package, site};

// ── 载荷 ────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SiteAppQuery {
    pub site_id: i64,
}

#[derive(Deserialize)]
pub struct AppLogQuery {
    pub site_id: i64,
    pub name: String,
    #[serde(default = "default_lines")]
    pub lines: usize,
}

fn default_lines() -> usize {
    200
}

#[derive(Deserialize)]
pub struct AppDeployPayload {
    pub site_id: i64,
    /// 应用名（站点内唯一）
    pub name: String,
    /// python | nodejs
    pub app_type: String,
    /// 运行时版本（如 `3.11` / `20`）；空 = 系统默认版本
    pub runtime_version: Option<String>,
    /// 构建 / 编译命令（部署时先跑它）；空 = 不构建
    pub build_cmd: Option<String>,
    /// python：是否生成 .venv 虚拟环境（默认开启）
    pub create_venv: Option<bool>,
    /// 工作目录（空 = 站点 web_root）
    pub workdir: Option<String>,
    pub entry: Option<String>,
    /// 自定义启动命令（非空则覆盖类型默认模板）
    pub command: Option<String>,
    pub port: Option<i64>,
    /// 环境变量，每行一条 KEY=VALUE
    pub env: Option<String>,
    pub autostart: Option<bool>,
    /// 是否安装依赖（pip / npm）
    pub install_deps: Option<bool>,
    /// 自动分配端口（true 时忽略 port，在套餐端口段里挑一个没被占用的）
    pub auto_port: Option<bool>,
    /// 域名：填了就自动创建一个反向代理站点（`site_type=proxy`，把应用反代出去）。
    /// 不填 site_id 时用它建站；填了 site_id 时忽略。
    pub domain: Option<String>,
    /// 挂载点：站点上用哪个 location 前缀把应用反代出去，默认 `/`。
    /// 同一站点挂多个应用时各填各的（`/`、`/api`、`/admin` …）。
    pub mount_path: Option<String>,
    /// 挂载点匹配方式：空（默认）= 前缀匹配；`exact` = 精确匹配 `location = /api`；
    /// `prefer` = 优先前缀 `location ^~ /api`
    pub match_mode: Option<String>,
    /// 是否把挂载前缀剥掉再转发（proxy_pass 带 URI）：挂 `/njs` 时
    /// `/njs/a` 转发给后端变成 `/a`。不传时子路径挂载默认剥离。
    pub strip_prefix: Option<bool>,
}

#[derive(Deserialize)]
pub struct AppActionPayload {
    pub site_id: i64,
    pub name: String,
    /// start | stop | restart | enable | disable
    pub action: String,
}

#[derive(Deserialize)]
pub struct AppRemovePayload {
    pub site_id: i64,
    pub name: String,
}

// ── 上下文与能力 ────────────────────────────────────────

/// 站点上下文：工作目录 / 日志目录 / 运行身份
struct SiteCtx {
    web_root: String,
    log_root: String,
    /// 应用进程以它运行（unix 用户，永不为 root）
    owner: String,
}

async fn load_site_ctx(site_id: i64) -> Result<SiteCtx, ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT s.web_root, s.log_root, u.linux_user, u.username \
         FROM site s JOIN user u ON u.id = s.user_id WHERE s.id = ?",
    )
    .bind(site_id)
    .fetch_optional(pool)
    .await?;
    let Some((web_root, log_root, linux_user, username)) = row else {
        return Err(ZapError::New(-1, "站点不存在".to_string()));
    };
    // linux_user 是面板为该客户创建的系统账号；老数据可能为空，回退登录名
    let owner = if linux_user.trim().is_empty() {
        username
    } else {
        linux_user
    };
    if owner.trim().is_empty() || owner == "root" {
        return Err(ZapError::New(
            -1,
            "站点没有可用的运行用户，无法部署应用".to_string(),
        ));
    }
    Ok(SiteCtx {
        web_root,
        log_root,
        owner,
    })
}

/// 操作者是否被允许使用应用管理（admin / reseller 恒可）
fn apps_allowed(claims: &jwt::Claims) -> bool {
    jwt::is_admin(claims) || jwt::is_reseller(claims)
}

/// 套餐能力：总开关 + 允许的类型 + 每站点数量上限
struct AppCaps {
    allowed: bool,
    types: Vec<String>,
    /// 每站点应用数上限（0 = 不限）
    max_apps: i64,
    /// 应用可监听端口范围（0/0 = 不限）
    port_min: i64,
    port_max: i64,
    /// 该用户全部站点合计的应用数上限（0 = 不限）
    max_total: i64,
    /// 套餐配了每用户端口数，但端口池已排到 65535 之外 → 无法再分配端口
    exhausted: bool,
}

async fn caps_of(claims: &jwt::Claims) -> AppCaps {
    if apps_allowed(claims) {
        return AppCaps {
            allowed: true,
            types: APP_TYPES.iter().map(|s| s.to_string()).collect(),
            max_apps: 0,
            port_min: 0,
            port_max: 0,
            max_total: 0,
            exhausted: false,
        };
    }
    match package::effective_package_of(claims.id as i64).await {
        Some(pkg) => {
            // 端口段自动计算：基准 10000 + 用户ID × 每用户端口数
            let range = package::user_port_range(claims.id as i64, pkg.app_port_span);
            AppCaps {
                allowed: pkg.allow_apps == 1,
                types: if pkg.app_types.is_empty() {
                    APP_TYPES.iter().map(|s| s.to_string()).collect()
                } else {
                    pkg.app_types
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                },
                max_apps: pkg.max_apps,
                port_min: range.map(|(lo, _)| lo).unwrap_or(0),
                port_max: range.map(|(_, hi)| hi).unwrap_or(0),
                max_total: pkg.app_max_total,
                exhausted: pkg.app_port_span > 0 && range.is_none(),
            }
        }
        None => AppCaps {
            allowed: false,
            types: Vec::new(),
            max_apps: 0,
            port_min: 0,
            port_max: 0,
            max_total: 0,
            exhausted: false,
        },
    }
}

/// 在允许范围内挑一个没被占用的端口：优先套餐端口段，未配端口段时从 10000 起找
async fn pick_free_port(caps: &AppCaps) -> Result<i64, ZapError> {
    let pool = db::get_db_pool().await;
    let used: Vec<i64> = sqlx::query_scalar("SELECT port FROM site_apps WHERE port > 0")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    let used: std::collections::HashSet<i64> = used.into_iter().collect();
    let (lo, hi) = if caps.port_min > 0 && caps.port_max > 0 {
        (caps.port_min, caps.port_max)
    } else {
        (10000, 65535)
    };
    (lo..=hi)
        .find(|p| !used.contains(p))
        .ok_or_else(|| ZapError::New(-1, "套餐端口段内已无可用端口".to_string()))
}

/// 域名 -> 建一个反代站点：站点类型 proxy，挂载点（默认 `/`）反代到 127.0.0.1:<port>。
/// 走 `site::site_add` 是为了复用建站那一整套校验（套餐站点数、反代开关、目录规划）。
async fn create_proxy_site(
    claims: &jwt::Claims,
    client_addr: SocketAddr,
    domain: &str,
    name: &str,
    port: i64,
    mount: &str,
    match_mode: &str,
    strip_prefix: bool,
) -> Result<i64, ZapError> {
    // 建站归属：admin / reseller 没有默认归属，落到操作者本人
    let owner = if jwt::is_admin(claims) || jwt::is_reseller(claims) {
        Some(claims.id as i64)
    } else {
        None
    };
    let add = site::SiteAddPayload {
        user_id: owner,
        name: Some(name.to_string()),
        domains: vec![domain.to_string()],
        ips: Vec::new(),
        status: Some(1),
        remark: Some(format!("应用 {name} 的反代站点（由应用部署自动创建）")),
        php_instance: None,
        site_type: "proxy".to_string(),
        pseudo_static: "none".to_string(),
        pseudo_custom: String::new(),
        web_root_custom: false,
        web_root: None,
        web_root_sub: None,
        upstreams: Vec::new(),
        locations: vec![LocationSpec {
            path: site::normalize_mount_path(mount).map_err(|e| ZapError::New(-1, e))?,
            match_mode: site::normalize_match_mode(match_mode),
            kind: "proxy".to_string(),
            target: format!("http://127.0.0.1:{port}"),
            code: 0,
            ws: true,
            strip_prefix,
            app_name: name.to_string(),
            ..Default::default()
        }],
        ssl_cert_id: None,
        force_https: false,
        ssl_protocols: String::new(),
        ssl_ciphers: String::new(),
        ssl_prefer_server_ciphers: true,
        ssl_http2: true,
        sec: None,
    };
    let Json(v) = site::site_add(
        jwt::ValidatedClaims(claims.clone()),
        Extension(client_addr),
        Json(add),
    )
    .await?;
    v.get("data")
        .and_then(|d| d.get("id"))
        .and_then(|x| x.as_i64())
        .filter(|id| *id > 0)
        .ok_or_else(|| ZapError::New(-1, "反代站点创建失败：未拿到站点 ID".to_string()))
}

/// 应用名合法性：会被拼进 systemd unit 名 `zap-app-{site_id}-{name}.service`，
/// 所以只允许字母数字和 `-_.`；输入上限 48，留出前缀空间。
fn valid_app_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 48
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// 最终应用名：admin 直接用输入名；其余角色自动补上站点归属用户名前缀，
/// 免得不同用户都叫 `api` 时 unit 名看着分不清是谁的。
fn final_app_name(name: &str, is_admin: bool, owner: &str) -> Result<String, String> {
    if is_admin || owner.is_empty() {
        return Ok(name.to_string());
    }
    let prefix = format!("{owner}-");
    if name.starts_with(&prefix) {
        Ok(name.to_string())
    } else {
        let out = format!("{prefix}{name}");
        if out.len() > 64 {
            return Err("应用名太长（加上用户名前缀后超过 64 字符）".to_string());
        }
        Ok(out)
    }
}

/// 端口校验：不给特权端口；套餐配了范围就必须落在范围内；且不能和别的应用撞端口
async fn require_port_ok(
    caps: &AppCaps,
    site_id: i64,
    name: &str,
    port: i64,
) -> Result<(), ZapError> {
    if port <= 0 {
        return Ok(());
    }
    if port < 1024 {
        return Err(ZapError::New(-1, "端口必须在 1024-65535 之间".to_string()));
    }
    if caps.exhausted {
        return Err(ZapError::New(
            -1,
            "端口池已排满：请联系管理员调整端口基准或每用户端口数".to_string(),
        ));
    }
    if caps.port_min > 0 && caps.port_max > 0 && (port < caps.port_min || port > caps.port_max) {
        return Err(ZapError::New(
            -1,
            format!(
                "端口需在套餐允许的 {}-{} 范围内",
                caps.port_min, caps.port_max
            ),
        ));
    }
    // 端口全局唯一：避免两个应用抢同一个端口，谁都起不来
    let pool = db::get_db_pool().await;
    let used: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM site_apps WHERE port = ? AND NOT (site_id = ? AND name = ?)",
    )
    .bind(port)
    .bind(site_id)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    if used > 0 {
        return Err(ZapError::New(-1, format!("端口 {port} 已被其他应用占用")));
    }
    Ok(())
}

/// 该用户全部站点已部署的应用数（admin / reseller 不受总量限制，不查）
async fn count_user_apps(user_id: i64) -> i64 {
    let pool = db::get_db_pool().await;
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM site_apps a JOIN site s ON s.id = a.site_id WHERE s.user_id = ?",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

async fn require_caps(claims: &jwt::Claims) -> Result<AppCaps, ZapError> {
    let caps = caps_of(claims).await;
    if !caps.allowed {
        return Err(ZapError::New(
            -1,
            "当前套餐未开放应用管理（Application Manager）".to_string(),
        ));
    }
    Ok(caps)
}

async fn count_apps(site_id: i64) -> i64 {
    let pool = db::get_db_pool().await;
    sqlx::query_scalar("SELECT COUNT(*) FROM site_apps WHERE site_id = ?")
        .bind(site_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

fn exec_err(resp: &zap_proto::Response) -> ZapError {
    ZapError::New(resp.code, resp.message.clone())
}

// ── 路由 ────────────────────────────────────────────────

/// GET /site/app/caps —— 当前用户在某站点上的应用能力（前端据此禁用 UI）
pub async fn app_caps(claims: ValidatedClaims, Query(q): Query<SiteAppQuery>) -> ZapJsonResult {
    site::site_in_scope(&claims, q.site_id).await?;
    let caps = caps_of(&claims).await;
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": {
            "allowed": caps.allowed,
            "types": caps.types,
            "max_apps": caps.max_apps,
            "port_min": caps.port_min,
            "port_max": caps.port_max,
            "max_total": caps.max_total,
            "used_total": count_user_apps(claims.id as i64).await,
        },
    })))
}

/// GET /site/app/list —— 应用列表 + 实时运行状态
pub async fn app_list(claims: ValidatedClaims, Query(q): Query<SiteAppQuery>) -> ZapJsonResult {
    site::site_in_scope(&claims, q.site_id).await?;
    let pool = db::get_db_pool().await;
    let rows: Vec<(
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        String,
        i64,
        i64,
    )> = sqlx::query_as(
        "SELECT id, name, app_type, runtime_version, build_cmd, workdir, entry, command, \
                port, env, autostart, running \
             FROM site_apps WHERE site_id = ? ORDER BY id",
    )
    .bind(q.site_id)
    .fetch_all(pool)
    .await?;

    // 实时状态：zapexec 不可用时静默降级（列表照出，状态为 unknown）
    let mut live: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    let names: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
    if !names.is_empty() {
        if let Ok(resp) = crate::zapexec::call(Request::AppStatus {
            site_id: q.site_id,
            names,
        })
        .await
        {
            if resp.code == 0 {
                if let Some(list) = resp
                    .data
                    .as_ref()
                    .and_then(|d| d.get("apps"))
                    .and_then(|v| v.as_array())
                {
                    for a in list {
                        if let Some(n) = a.get("name").and_then(|v| v.as_str()) {
                            live.insert(n.to_string(), a.clone());
                        }
                    }
                }
            }
        }
    }

    let apps: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                name,
                app_type,
                runtime_version,
                build_cmd,
                workdir,
                entry,
                command,
                port,
                env,
                autostart,
                running,
            )| {
                let st = live.get(&name).cloned().unwrap_or(json!({
                    "state": "unknown", "active": false, "enabled": false, "pid": 0,
                }));
                json!({
                    "id": id,
                    "name": name,
                    "app_type": app_type,
                    "runtime_version": runtime_version,
                    "build_cmd": build_cmd,
                    "workdir": workdir,
                    "entry": entry,
                    "command": command,
                    "port": port,
                    "env": env,
                    "autostart": autostart == 1,
                    "running": running == 1,
                    "state": st.get("state").and_then(|v| v.as_str()).unwrap_or("unknown"),
                    "active": st.get("active").and_then(|v| v.as_bool()).unwrap_or(false),
                    "enabled": st.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
                    "pid": st.get("pid").and_then(|v| v.as_i64()).unwrap_or(0),
                })
            },
        )
        .collect();
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": { "apps": apps } }),
    ))
}

/// POST /site/app/deploy —— 新建或重新部署（同一个 (站点, 名称) 幂等覆盖）
pub async fn app_deploy(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<AppDeployPayload>,
) -> ZapJsonResult {
    let caps = require_caps(&claims).await?;

    let app_type = payload.app_type.trim().to_ascii_lowercase();
    if !caps.types.iter().any(|t| *t == app_type) {
        return Err(ZapError::New(
            -1,
            format!(
                "当前套餐不允许部署 {app_type} 应用（允许：{}）",
                caps.types.join(", ")
            ),
        ));
    }

    let name = payload.name.trim().to_string();
    if !valid_app_name(&name) {
        return Err(ZapError::New(
            -1,
            "应用名只能用字母、数字、-、_ 和 .，且不超过 48 个字符".to_string(),
        ));
    }

    let entry = payload.entry.clone().unwrap_or_default();
    let command = payload.command.clone().unwrap_or_default();
    let env = payload.env.clone().unwrap_or_default();
    let port = payload.port.unwrap_or(0);
    let autostart = payload.autostart.unwrap_or(true);
    let install_deps = payload.install_deps.unwrap_or(false);
    let mut runtime_version = payload
        .runtime_version
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    // 没指定版本就跟随全局默认版本（Python 由 uv 管、Node 由 fnm 管）
    if runtime_version.is_empty() && app_type == "python" {
        runtime_version = crate::routers::system_env::python_default();
    }
    if runtime_version.is_empty() && app_type == "nodejs" {
        runtime_version = crate::routers::system_env::node_default();
    }
    let build_cmd = payload
        .build_cmd
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    let create_venv = payload.create_venv.unwrap_or(true);
    if !runtime_version.is_empty()
        && !runtime_version
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
    {
        return Err(ZapError::New(
            -1,
            "运行时版本号只能包含数字和点".to_string(),
        ));
    }

    let domain = payload
        .domain
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();

    // 端口先定下来：自动分配时在套餐端口段里挑空闲的（建站要用它做反代目标）
    let port = if payload.auto_port.unwrap_or(false) || (payload.site_id == 0 && port <= 0) {
        pick_free_port(&caps).await?
    } else {
        port
    };

    // 挂载点：站点上用哪个前缀反代这个应用（默认 /，可自定义；一个站点可挂多个应用）
    let mount = payload
        .mount_path
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    let match_mode = payload.match_mode.clone().unwrap_or_default();
    let strip_prefix = payload.strip_prefix;

    // 站点来源二选一：直接选已有站点，或填域名自动建一个反代站点
    let site_id = if payload.site_id > 0 {
        site::site_in_scope(&claims, payload.site_id).await?;
        payload.site_id
    } else {
        if domain.is_empty() {
            return Err(ZapError::New(
                -1,
                "请选择要部署到的站点，或填写域名自动创建反代站点".to_string(),
            ));
        }
        create_proxy_site(
            &claims,
            client_addr,
            &domain,
            &name,
            port,
            &mount,
            &match_mode,
            strip_prefix.unwrap_or(mount != "/"),
        )
        .await?
    };

    let ctx = load_site_ctx(site_id).await?;

    // 应用名：admin 不限制前缀，其余角色自动带上站点归属用户名前缀
    let name = final_app_name(&name, jwt::is_admin(&claims), &ctx.owner)
        .map_err(|e| ZapError::New(-1, e))?;

    // 工作目录：不填用站点根目录；填了也必须在站点目录内（执行端还会再校验一次）
    let workdir = match payload
        .workdir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(w) => {
            // 与执行端一致：只要落在站点用户的家目录内即可（项目目录可以不在 web_root 下）
            let home = format!("/home/{}", ctx.owner);
            let full = if w.starts_with('/') {
                w.to_string()
            } else {
                format!("{}/{}", home, w.trim_start_matches('/'))
            };
            if !full.starts_with(&home) {
                return Err(ZapError::New(-1, format!("工作目录必须在 {home} 之内")));
            }
            full
        }
        None => ctx.web_root.clone(),
    };

    require_port_ok(&caps, site_id, &name, port).await?;
    if caps.max_total > 0 && count_user_apps(claims.id as i64).await >= caps.max_total {
        return Err(ZapError::New(
            -1,
            format!("当前套餐限制每个用户最多 {} 个应用", caps.max_total),
        ));
    }

    // 数量上限：仅新建时校验（重新部署同名应用不算新增）
    let pool = db::get_db_pool().await;
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM site_apps WHERE site_id = ? AND name = ?")
            .bind(payload.site_id)
            .bind(&name)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    if exists == 0 && caps.max_apps > 0 && count_apps(site_id).await >= caps.max_apps {
        return Err(ZapError::New(
            -1,
            format!("当前套餐限制每个站点最多 {} 个应用", caps.max_apps),
        ));
    }

    let resp = crate::zapexec::call(Request::AppDeploy {
        site_id,
        name: name.clone(),
        app_type: app_type.clone(),
        runtime_version: runtime_version.clone(),
        build_cmd: build_cmd.clone(),
        create_venv,
        workdir: workdir.clone(),
        entry: entry.clone(),
        command: command.clone(),
        port,
        env: env.clone(),
        autostart,
        install_deps,
        owner_user: ctx.owner.clone(),
        log_dir: ctx.log_root.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(exec_err(&resp));
    }

    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO site_apps (site_id, name, app_type, runtime_version, build_cmd, workdir, \
                entry, command, port, env, autostart, running, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?) \
         ON CONFLICT(site_id, name) DO UPDATE SET \
           app_type = excluded.app_type, runtime_version = excluded.runtime_version, \
           build_cmd = excluded.build_cmd, workdir = excluded.workdir, entry = excluded.entry, \
           command = excluded.command, port = excluded.port, env = excluded.env, \
           autostart = excluded.autostart, running = 1, updated_at = excluded.updated_at",
    )
    .bind(site_id)
    .bind(&name)
    .bind(&app_type)
    .bind(&runtime_version)
    .bind(&build_cmd)
    .bind(&workdir)
    .bind(&entry)
    .bind(&command)
    .bind(port)
    .bind(&env)
    .bind(i64::from(autostart))
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| ZapError::New(-1, format!("保存应用配置失败：{e}")))?;

    // 部署完只写库还不够：nginx 上没有这条反代，站点等于没同步
    let mounted =
        site::ensure_app_location(site_id, &name, port, &mount, &match_mode, strip_prefix)
            .await
            .unwrap_or(None);
    let sync_ok = match site::sync_one_site(site_id).await {
        Ok(_) => true,
        Err(e) => {
            info!("app deploy: site sync failed: site={site_id} err={e}");
            false
        }
    };

    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "app_deploy",
        &format!("site={} name={}", site_id, name),
        &format!("type={app_type} workdir={workdir}"),
    )
    .await;
    info!(
        "app deploy: site={} name={} type={}",
        site_id, name, app_type
    );
    Ok(Json(json!({
        "code": 0,
        "message": resp.message,
        "data": {
            "site_id": site_id,
            "name": name,
            "port": port,
            "mounted": mounted,
            "synced": sync_ok,
            "detail": resp.data.unwrap_or(Value::Null),
        }
    })))
}

/// GET /site/app/runtimes —— 探测服务器已安装的运行时版本（部署向导下拉用）
pub async fn app_runtimes(claims: ValidatedClaims) -> ZapJsonResult {
    let caps = require_caps(&claims).await?;
    if !caps.allowed {
        return Ok(Json(json!({
            "code": 0,
            "message": "当前套餐未开启应用管理",
            "data": { "python": [], "nodejs": [], "types": [] },
        })));
    }
    match crate::zapexec::call(Request::AppRuntimes).await {
        Ok(resp) if resp.code == 0 => {
            let d = resp.data.unwrap_or(Value::Null);
            Ok(Json(json!({
                "code": 0,
                "message": resp.message,
                "data": {
                    "python": d.get("python").cloned().unwrap_or(json!([])),
                    "nodejs": d.get("nodejs").cloned().unwrap_or(json!([])),
                    "types": caps.types,
                    "allowed": caps.allowed,
                    "port_min": caps.port_min,
                    "port_max": caps.port_max,
                },
            })))
        }
        Ok(resp) => Err(exec_err(&resp)),
        Err(e) => Err(e),
    }
}

/// GET /site/app/list_all —— 跨站点应用列表（应用管理面板用，带站点名与端口）
pub async fn app_list_all(claims: ValidatedClaims) -> ZapJsonResult {
    let caps = require_caps(&claims).await?;
    if !caps.allowed {
        return Ok(Json(json!({ "code": 0, "message": "", "data": [] })));
    }
    let pool = db::get_db_pool().await;
    let base = "SELECT a.id, a.site_id, s.name AS site_name, a.name, a.app_type, \
                       a.runtime_version, a.build_cmd, a.workdir, a.entry, a.command, \
                       a.port, a.env, a.autostart, a.running \
                FROM site_apps a JOIN site s ON s.id = a.site_id";
    type Row = (
        i64,
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        String,
        i64,
        i64,
    );
    let rows: Vec<Row> = if jwt::is_admin(&claims) {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{base} ORDER BY s.name, a.name"
        )))
        .fetch_all(pool)
        .await?
    } else if jwt::is_reseller(&claims) {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{base} WHERE s.user_id = ? OR s.user_id IN (SELECT id FROM user WHERE owner_id = ?) \
                 ORDER BY s.name, a.name"
        )))
        .bind(claims.id as i64)
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{base} WHERE s.user_id = ? ORDER BY s.name, a.name"
        )))
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await?
    };

    // 挂载点：站点 locations 里标了 app_name 的那条就是该应用挂在哪
    let mut mounts: std::collections::HashMap<(i64, String), (String, String)> =
        std::collections::HashMap::new();
    let mut seen_sites: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for r in &rows {
        if !seen_sites.insert(r.1) {
            continue;
        }
        for (app, p, m) in site::app_mounts(r.1).await {
            mounts.insert((r.1, app), (p, m));
        }
    }

    // 实时进程状态：按站点批量问一次 zapexec（不可用则降级为 unknown，列表照出）
    let mut live: std::collections::HashMap<(i64, String), Value> =
        std::collections::HashMap::new();
    let mut by_site: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    for r in &rows {
        by_site.entry(r.1).or_default().push(r.3.clone());
    }
    for (sid, names) in by_site {
        if let Ok(resp) = crate::zapexec::call(Request::AppStatus {
            site_id: sid,
            names,
        })
        .await
        {
            if resp.code != 0 {
                continue;
            }
            if let Some(list) = resp
                .data
                .as_ref()
                .and_then(|d| d.get("apps"))
                .and_then(|v| v.as_array())
            {
                for a in list {
                    if let Some(n) = a.get("name").and_then(|v| v.as_str()) {
                        live.insert((sid, n.to_string()), a.clone());
                    }
                }
            }
        }
    }

    let apps: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                site_id,
                site_name,
                name,
                app_type,
                runtime_version,
                build_cmd,
                workdir,
                entry,
                command,
                port,
                env,
                autostart,
                running,
            )| {
                let st = live
                    .get(&(site_id, name.clone()))
                    .cloned()
                    .unwrap_or(json!({
                        "state": "unknown", "active": false, "enabled": false, "pid": 0,
                    }));
                let (mount_path, mount_mode) = mounts
                    .get(&(site_id, name.clone()))
                    .cloned()
                    .unwrap_or_default();
                json!({
                    "id": id,
                    "site_id": site_id,
                    "site_name": site_name,
                    "name": name,
                    "mount_path": mount_path,
                    "mount_mode": mount_mode,
                    "app_type": app_type,
                    "runtime_version": runtime_version,
                    "build_cmd": build_cmd,
                    "workdir": workdir,
                    "entry": entry,
                    "command": command,
                    "port": port,
                    "env": env,
                    "autostart": autostart == 1,
                    "running": running == 1,
                    "state": st.get("state").and_then(|v| v.as_str()).unwrap_or("unknown"),
                    "active": st.get("active").and_then(|v| v.as_bool()).unwrap_or(false),
                    "enabled": st.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
                    "pid": st.get("pid").and_then(|v| v.as_i64()).unwrap_or(0),
                })
            },
        )
        .collect();
    Ok(Json(json!({ "code": 0, "message": "", "data": apps })))
}

/// POST /site/app/action —— start | stop | restart | enable | disable
pub async fn app_action(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<AppActionPayload>,
) -> ZapJsonResult {
    site::site_in_scope(&claims, payload.site_id).await?;
    require_caps(&claims).await?;
    let resp = crate::zapexec::call(Request::AppAction {
        site_id: payload.site_id,
        name: payload.name.clone(),
        action: payload.action.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(exec_err(&resp));
    }
    // 期望状态：start/restart -> running；stop -> 停止
    if let Some(run) = match payload.action.as_str() {
        "start" | "restart" => Some(1),
        "stop" => Some(0),
        _ => None,
    } {
        let pool = db::get_db_pool().await;
        let _ = sqlx::query(
            "UPDATE site_apps SET running = ?, updated_at = ? WHERE site_id = ? AND name = ?",
        )
        .bind(run)
        .bind(chrono::Utc::now().timestamp())
        .bind(payload.site_id)
        .bind(&payload.name)
        .execute(pool)
        .await;
    }
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "app_action",
        &format!("site={} name={}", payload.site_id, payload.name),
        &payload.action,
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "ok" })))
}

/// POST /site/app/remove —— 停服务 + 删 unit + 删记录
pub async fn app_remove(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<AppRemovePayload>,
) -> ZapJsonResult {
    site::site_in_scope(&claims, payload.site_id).await?;
    require_caps(&claims).await?;
    let resp = crate::zapexec::call(Request::AppRemove {
        site_id: payload.site_id,
        name: payload.name.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(exec_err(&resp));
    }
    let pool = db::get_db_pool().await;
    sqlx::query("DELETE FROM site_apps WHERE site_id = ? AND name = ?")
        .bind(payload.site_id)
        .bind(&payload.name)
        .execute(pool)
        .await?;
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "app_remove",
        &format!("site={} name={}", payload.site_id, payload.name),
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "已删除" })))
}

/// GET /site/app/log —— 应用日志尾部
pub async fn app_log(claims: ValidatedClaims, Query(q): Query<AppLogQuery>) -> ZapJsonResult {
    site::site_in_scope(&claims, q.site_id).await?;
    let resp = crate::zapexec::call(Request::AppLog {
        site_id: q.site_id,
        name: q.name,
        lines: q.lines,
    })
    .await?;
    if resp.code != 0 {
        return Err(exec_err(&resp));
    }
    let lines = resp
        .data
        .as_ref()
        .and_then(|d| d.get("lines"))
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": { "lines": lines } }),
    ))
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn app_name_charset_is_guarded() {
        assert!(valid_app_name("helloflask"));
        assert!(valid_app_name("my_app.v2"));
        // 会被拼进 systemd unit 名 / 命令行，这些必须挡住
        assert!(!valid_app_name(""));
        assert!(!valid_app_name("my app"));
        assert!(!valid_app_name("app;rm -rf /"));
        assert!(!valid_app_name("应用名"));
        assert!(!valid_app_name(&"a".repeat(49)));
    }

    #[test]
    fn admin_keeps_bare_name_users_get_owner_prefix() {
        // admin：原样使用
        assert_eq!(final_app_name("api", true, "alice").unwrap(), "api");
        // 普通用户：自动带站点归属用户名前缀，且不会重复加
        assert_eq!(final_app_name("api", false, "alice").unwrap(), "alice-api");
        assert_eq!(
            final_app_name("alice-api", false, "alice").unwrap(),
            "alice-api"
        );
        // 没有归属信息时不硬加前缀
        assert_eq!(final_app_name("api", false, "").unwrap(), "api");
    }

    /// 回归防护：多行 SQL 的续行符写错成 `\\` 时编译照过、运行才炸
    /// （SQLite 报 `unrecognized token: "\"`，而这类错误只在部署时才暴露）。
    #[test]
    fn sql_continuations_have_single_backslash() {
        let src = include_str!("app.rs");
        for (i, line) in src.lines().enumerate() {
            if line.contains("SELECT") || line.contains("INSERT") || line.contains("UPDATE") {
                assert!(
                    !line.trim_end().ends_with("\\\\"),
                    "第 {} 行 SQL 续行符写成了双反斜杠（会泄漏进 SQL 文本）：{}",
                    i + 1,
                    line.trim()
                );
            }
        }
    }
}
