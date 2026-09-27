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
use zap_proto::{Request, APP_TYPES};

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
        "SELECT s.web_root, s.log_root, u.linux_user, u.username \\
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
    max_apps: i64,
}

async fn caps_of(claims: &jwt::Claims) -> AppCaps {
    if apps_allowed(claims) {
        return AppCaps {
            allowed: true,
            types: APP_TYPES.iter().map(|s| s.to_string()).collect(),
            max_apps: 0,
        };
    }
    match package::effective_package_of(claims.id as i64).await {
        Some(pkg) => AppCaps {
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
        },
        None => AppCaps {
            allowed: false,
            types: Vec::new(),
            max_apps: 0,
        },
    }
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
pub async fn app_caps(
    claims: ValidatedClaims,
    Query(q): Query<SiteAppQuery>,
) -> ZapJsonResult {
    site::site_in_scope(&claims, q.site_id).await?;
    let caps = caps_of(&claims).await;
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": { "allowed": caps.allowed, "types": caps.types, "max_apps": caps.max_apps },
    })))
}

/// GET /site/app/list —— 应用列表 + 实时运行状态
pub async fn app_list(
    claims: ValidatedClaims,
    Query(q): Query<SiteAppQuery>,
) -> ZapJsonResult {
    site::site_in_scope(&claims, q.site_id).await?;
    let pool = db::get_db_pool().await;
    let rows: Vec<(i64, String, String, String, String, String, i64, String, i64, i64)> =
        sqlx::query_as(
            "SELECT id, name, app_type, workdir, entry, command, port, env, autostart, running \\
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
                if let Some(list) = resp.data.as_ref().and_then(|d| d.get("apps")).and_then(|v| v.as_array()) {
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
        .map(|(id, name, app_type, workdir, entry, command, port, env, autostart, running)| {
            let st = live.get(&name).cloned().unwrap_or(json!({
                "state": "unknown", "active": false, "enabled": false, "pid": 0,
            }));
            json!({
                "id": id,
                "name": name,
                "app_type": app_type,
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
        })
        .collect();
    Ok(Json(json!({ "code": 0, "message": "ok", "data": { "apps": apps } })))
}

/// POST /site/app/deploy —— 新建或重新部署（同一个 (站点, 名称) 幂等覆盖）
pub async fn app_deploy(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<AppDeployPayload>,
) -> ZapJsonResult {
    site::site_in_scope(&claims, payload.site_id).await?;
    let caps = require_caps(&claims).await?;

    let app_type = payload.app_type.trim().to_ascii_lowercase();
    if !caps.types.iter().any(|t| *t == app_type) {
        return Err(ZapError::New(
            -1,
            format!("当前套餐不允许部署 {app_type} 应用（允许：{}）", caps.types.join(", ")),
        ));
    }

    let name = payload.name.trim().to_string();
    if name.is_empty() || name.len() > 64 {
        return Err(ZapError::New(-1, "应用名长度需在 1-64 之间".to_string()));
    }

    let entry = payload.entry.clone().unwrap_or_default();
    let command = payload.command.clone().unwrap_or_default();
    let env = payload.env.clone().unwrap_or_default();
    let port = payload.port.unwrap_or(0);
    let autostart = payload.autostart.unwrap_or(true);
    let install_deps = payload.install_deps.unwrap_or(false);

    let ctx = load_site_ctx(payload.site_id).await?;

    // 工作目录：不填用站点根目录；填了也必须在站点目录内（执行端还会再校验一次）
    let workdir = match payload.workdir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(w) => {
            let root = ctx.web_root.trim_end_matches('/');
            let full = if w.starts_with('/') {
                w.to_string()
            } else {
                format!("{root}/{}", w.trim_start_matches('/'))
            };
            if !root.is_empty() && !full.starts_with(root) {
                return Err(ZapError::New(-1, "工作目录必须在站点目录内".to_string()));
            }
            full
        }
        None => ctx.web_root.clone(),
    };

    // 数量上限：仅新建时校验（重新部署同名应用不算新增）
    let pool = db::get_db_pool().await;
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM site_apps WHERE site_id = ? AND name = ?",
    )
    .bind(payload.site_id)
    .bind(&name)
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    if exists == 0 && caps.max_apps > 0 && count_apps(payload.site_id).await >= caps.max_apps {
        return Err(ZapError::New(
            -1,
            format!("当前套餐限制每个站点最多 {} 个应用", caps.max_apps),
        ));
    }

    let resp = crate::zapexec::call(Request::AppDeploy {
        site_id: payload.site_id,
        name: name.clone(),
        app_type: app_type.clone(),
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
        "INSERT INTO site_apps (site_id, name, app_type, workdir, entry, command, port, env, \\
                autostart, running, created_at, updated_at) \\
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?) \\
         ON CONFLICT(site_id, name) DO UPDATE SET \\
           app_type = excluded.app_type, workdir = excluded.workdir, entry = excluded.entry, \\
           command = excluded.command, port = excluded.port, env = excluded.env, \\
           autostart = excluded.autostart, running = 1, updated_at = excluded.updated_at",
    )
    .bind(payload.site_id)
    .bind(&name)
    .bind(&app_type)
    .bind(&workdir)
    .bind(&entry)
    .bind(&command)
    .bind(port)
    .bind(&env)
    .bind(i64::from(autostart))
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "app_deploy",
        &format!("site={} name={}", payload.site_id, name),
        &format!("type={app_type} workdir={workdir}"),
    )
    .await;
    info!("app deploy: site={} name={} type={}", payload.site_id, name, app_type);
    Ok(Json(json!({
        "code": 0,
        "message": resp.message,
        "data": resp.data.unwrap_or(Value::Null),
    })))
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
    Ok(Json(json!({ "code": 0, "message": "ok", "data": { "lines": lines } })))
}
