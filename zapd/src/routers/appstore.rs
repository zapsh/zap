//! AppStore 路由：仓库管理 / 包安装卸载升级 / 脚本管理 / 运行记录与实时日志。

use std::{collections::BTreeMap, net::SocketAddr};

use axum::{
    Json,
    extract::{
        Extension, Path, Query,
        ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
};
use futures_util::SinkExt;
use jsonwebtoken::{DecodingKey, Validation, decode};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{error, info};

use crate::{
    config,
    routers::system_env,
    zap::{
        ZapError, ZapJsonResult, appstore as ast, audit,
        jwt::{self, Claims, ValidatedClaims},
        user_cron,
    },
    zapexec,
};
use zap_proto::Request;

fn require_admin(claims: &Claims) -> Result<(), ZapError> {
    if jwt::is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "权限不足，需要管理员权限".to_string()))
    }
}

/// 包角色门禁（install / upgrade）：
/// - admin 恒可安装/升级，不受 roles 限制；
/// - roles 为空/未声明 = 仅 admin 可见可操作；
/// - 声明了 roles = 当前登录角色必须命中其一才放行（否则拒绝并提示开放角色）。
async fn check_pkg_roles(claims: &Claims, pkg_path: &str) -> Result<(), ZapError> {
    if jwt::is_admin(claims) {
        return Ok(());
    }
    let denied = || {
        ZapError::New(
            -1,
            "该软件包默认仅管理员可操作；如需开放请在该包 app.yaml 声明 roles 并授予当前角色"
                .to_string(),
        )
    };
    let Some(roles) = ast::package_roles_of(pkg_path).await else {
        return Err(denied());
    };
    if roles.is_empty() {
        return Err(denied());
    }
    let mine: Vec<&str> = claims
        .roles
        .split(',')
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .collect();
    if roles.iter().any(|r| mine.contains(&r.as_str())) {
        Ok(())
    } else {
        Err(ZapError::New(
            -1,
            format!(
                "当前角色无权安装该应用（该应用仅对以下角色开放: {}）",
                roles.join(" / ")
            ),
        ))
    }
}

/// 校验脚本路径：必须位于 custom/scripts/ 下（脚本管理为 admin 专属）。
fn validate_script_path(path: &str) -> Result<(), ZapError> {
    if !path.starts_with("scripts/") {
        return Err(ZapError::New(
            -1,
            "只允许操作 scripts/ 下的脚本".to_string(),
        ));
    }
    if path.contains("..") {
        return Err(ZapError::New(-1, "脚本路径不合法".to_string()));
    }
    Ok(())
}

// ── Git 源管理（多源）───────────────────────────────────────

/// 列出全部 Git 源（含内置源）。
pub async fn list_repos(_claims: ValidatedClaims) -> ZapJsonResult {
    let repos = ast::list_repos().await;
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": { "repos": repos } }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct RepoAddPayload {
    pub name: String,
    pub url: String,
}

/// 添加 Git 源：clone 到 repos/<id>/ 并写入 repos.yaml。
pub async fn repo_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<RepoAddPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let name = payload.name.trim().to_string();
    let url = payload.url.trim().to_string();
    if name.is_empty() || url.is_empty() {
        return Err(ZapError::New(-1, "名称和 Git 地址均不能为空".to_string()));
    }

    let run_id = ast::generate_run_id();
    let log_path = ast::log_path_for(&run_id);
    ast::register_run(&run_id, "repo_add", &url, &claims.sub, &log_path).await?;

    let resp = zapexec::call(Request::AppstoreRepoAdd {
        name: name.clone(),
        url: url.clone(),
        run_id: run_id.clone(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(&run_id, "failed", resp.code as i64).await;
        return Err(ZapError::New(resp.code, resp.message));
    }
    ast::watch_log(run_id.clone(), log_path.clone());
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_repo_add",
        &name,
        &url,
    )
    .await;
    info!("AppStore repo add started: {name} ({url})");
    Ok(Json(json!({
        "code": 0,
        "message": "添加源已启动",
        "data": { "run_id": run_id, "log": log_path }
    })))
}

#[derive(Debug, Deserialize)]
pub struct RepoRemovePayload {
    pub id: String,
}

/// 删除 Git 源（内置源禁止删除）。
pub async fn repo_remove(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<RepoRemovePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let id = payload.id.trim().to_string();
    if id.is_empty() {
        return Err(ZapError::New(-1, "缺少源 id".to_string()));
    }
    let resp = zapexec::call(Request::AppstoreRepoRemove { id: id.clone() }).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_repo_remove",
        &id,
        "",
    )
    .await;
    info!("AppStore repo removed: {id}");
    Ok(Json(
        json!({ "code": 0, "message": "源已删除", "data": { "id": id } }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct RepoUpdatePayload {
    pub id: String,
}

/// 更新单个 Git 源（fetch + reset，首次则 clone）。
pub async fn repo_update(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<RepoUpdatePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let id = payload.id.trim().to_string();
    if id.is_empty() {
        return Err(ZapError::New(-1, "缺少源 id".to_string()));
    }
    let run_id = ast::generate_run_id();
    let log_path = ast::log_path_for(&run_id);
    ast::register_run(&run_id, "repo_update", &id, &claims.sub, &log_path).await?;

    let resp = zapexec::call(Request::AppstoreRepoUpdate {
        id: id.clone(),
        run_id: run_id.clone(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(&run_id, "failed", resp.code as i64).await;
        return Err(ZapError::New(resp.code, resp.message));
    }
    ast::watch_log(run_id.clone(), log_path.clone());
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_repo_update",
        &id,
        "",
    )
    .await;
    info!("AppStore repo update started: {id}");
    Ok(Json(json!({
        "code": 0,
        "message": "更新源已启动",
        "data": { "run_id": run_id, "log": log_path }
    })))
}

// ── 包列表 / 安装 / 卸载 / 升级 ─────────────────────────────

pub async fn packages(_claims: ValidatedClaims) -> ZapJsonResult {
    let pkgs = ast::scan_packages().await;
    let installed = ast::scan_installed().await;
    let mut installed_map = std::collections::HashMap::new();
    for inst in &installed {
        if let Some(p) = inst.get("pkg_path").and_then(|x| x.as_str()) {
            installed_map.insert(p.to_string(), inst.clone());
        }
    }
    let mut items: Vec<Value> = Vec::new();
    for mut pkg in pkgs {
        let pkg_path = pkg
            .get("pkg_path")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        if let Some(inst) = installed_map.get(&pkg_path) {
            pkg["installed"] = json!(true);
            pkg["installed_version"] = inst.get("version").cloned().unwrap_or(Value::Null);
            pkg["installed_source"] = inst.get("source").cloned().unwrap_or(Value::Null);
            pkg["installed_at"] = inst.get("installed_at").cloned().unwrap_or(Value::Null);
            pkg["upgraded_from"] = inst.get("upgraded_from").cloned().unwrap_or(Value::Null);
        } else {
            pkg["installed"] = json!(false);
        }
        items.push(pkg);
    }
    items.sort_by(|a, b| {
        a.get("category")
            .and_then(|x| x.as_str())
            .cmp(&b.get("category").and_then(|x| x.as_str()))
            .then_with(|| {
                a.get("name")
                    .and_then(|x| x.as_str())
                    .cmp(&b.get("name").and_then(|x| x.as_str()))
            })
    });
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": { "packages": items, "installed": installed }
    })))
}

#[derive(Debug, Deserialize)]
pub struct InstallPayload {
    pub pkg_path: String,
    pub source: String,
    pub repo_id: Option<String>,
    pub version: String,
    /// 用户点击的操作（app.yaml actions 键，如 bin/build）
    pub action: Option<String>,
    /// 安装表单选项：选项名 -> 字符串化值
    pub options: Option<BTreeMap<String, String>>,
}

/// 校验并规整 options：仅接受合法 shell 变量名，拒绝系统保留前缀；
/// 值统一截断超长输入防滥用。
fn sanitize_options(
    options: Option<BTreeMap<String, String>>,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(mut opts) = options else {
        return Ok(None);
    };
    if opts.len() > 64 {
        return Err("安装选项数量过多".into());
    }
    for (k, v) in opts.iter_mut() {
        let valid = !k.is_empty()
            && k.len() <= 64
            && k.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !k.starts_with("ZAP_")
            && !k.starts_with("PKG_")
            && !k.starts_with("APP_")
            && !k.starts_with("ACTION")
            && !k.starts_with("SCRIPT_")
            && !k.starts_with("RUN_")
            && k != "PATH"
            && k != "HOME";
        if !valid {
            return Err(format!(
                "非法选项名: {k}（须为字母/下划线开头且不含保留前缀）"
            ));
        }
        if v.chars().count() > 4096 {
            return Err(format!("选项 {k} 的值过长"));
        }
    }
    Ok(Some(opts))
}

pub async fn install(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<InstallPayload>,
) -> ZapJsonResult {
    // 包角色门禁：admin 恒可安装；roles 缺省 = 仅 admin，声明后按白名单校验
    check_pkg_roles(&claims, &payload.pkg_path).await?;
    // 自定义包包含任意脚本，仅管理员可安装
    if payload.source == "custom" {
        require_admin(&claims)?;
    }
    let options = match sanitize_options(payload.options.clone()) {
        Ok(o) => o,
        Err(e) => return Err(ZapError::New(-1, e)),
    };
    let run_id = ast::generate_run_id();
    let log_path = ast::log_path_for(&run_id);
    ast::register_run(
        &run_id,
        "install",
        &payload.pkg_path,
        &claims.sub,
        &log_path,
    )
    .await?;

    // 注入操作者上下文：面板登录用户与虚拟主机运行模式（固定为独立系统用户）
    let user = claims.sub.clone();
    let run_mode = system_env::VHOST_MODE.to_string();

    let resp = zapexec::call(Request::AppstoreInstall {
        pkg_path: payload.pkg_path.clone(),
        source: payload.source.clone(),
        repo_id: payload.repo_id.clone(),
        version: payload.version.clone(),
        action: payload.action.clone(),
        options,
        user: Some(user),
        run_mode: Some(run_mode),
        run_id: run_id.clone(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(&run_id, "failed", resp.code as i64).await;
        return Err(ZapError::New(resp.code, resp.message));
    }
    ast::watch_log(run_id.clone(), log_path.clone());
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_install",
        &payload.pkg_path,
        &format!(
            "source={} version={} options=[{}]",
            payload.source,
            payload.version,
            payload
                .options
                .as_ref()
                .map(|o| o.keys().cloned().collect::<Vec<_>>().join(","))
                .unwrap_or_default()
        ),
    )
    .await;
    info!(
        "AppStore install started: {} ({})",
        payload.pkg_path, payload.source
    );
    Ok(Json(json!({
        "code": 0,
        "message": "安装已启动",
        "data": { "run_id": run_id, "log": log_path }
    })))
}

#[derive(Debug, Deserialize)]
pub struct UninstallPayload {
    pub pkg_path: String,
    /// 卸载表单选项：选项名 -> 字符串化值（app.yaml options.uninstall）
    pub options: Option<BTreeMap<String, String>>,
}

pub async fn uninstall(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<UninstallPayload>,
) -> ZapJsonResult {
    let options = match sanitize_options(payload.options.clone()) {
        Ok(o) => o,
        Err(e) => return Err(ZapError::New(-1, e)),
    };
    let run_id = ast::generate_run_id();
    let log_path = ast::log_path_for(&run_id);
    ast::register_run(
        &run_id,
        "uninstall",
        &payload.pkg_path,
        &claims.sub,
        &log_path,
    )
    .await?;

    // 注入操作者上下文：面板登录用户与虚拟主机运行模式（固定为独立系统用户）
    let user = claims.sub.clone();
    let run_mode = system_env::VHOST_MODE.to_string();

    let resp = zapexec::call(Request::AppstoreUninstall {
        pkg_path: payload.pkg_path.clone(),
        options,
        user: Some(user),
        run_mode: Some(run_mode),
        run_id: run_id.clone(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(&run_id, "failed", resp.code as i64).await;
        return Err(ZapError::New(resp.code, resp.message));
    }
    ast::watch_log(run_id.clone(), log_path.clone());
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_uninstall",
        &payload.pkg_path,
        &payload
            .options
            .as_ref()
            .map(|o| {
                format!(
                    "options=[{}]",
                    o.keys().cloned().collect::<Vec<_>>().join(",")
                )
            })
            .unwrap_or_default(),
    )
    .await;
    info!("AppStore uninstall started: {}", payload.pkg_path);
    Ok(Json(json!({
        "code": 0,
        "message": "卸载已启动",
        "data": { "run_id": run_id, "log": log_path }
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpgradePayload {
    pub pkg_path: String,
    pub source: String,
    pub repo_id: Option<String>,
    pub version: String,
    /// 用户点击的操作（app.yaml actions 键）
    pub action: Option<String>,
    /// 升级表单选项：选项名 -> 字符串化值
    pub options: Option<BTreeMap<String, String>>,
}

pub async fn upgrade(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<UpgradePayload>,
) -> ZapJsonResult {
    // 包角色门禁：admin 恒可升级；roles 缺省 = 仅 admin，声明后按白名单校验
    check_pkg_roles(&claims, &payload.pkg_path).await?;
    let old_version = ast::installed_version_of(&payload.pkg_path)
        .await
        .unwrap_or_default();
    let options = match sanitize_options(payload.options.clone()) {
        Ok(o) => o,
        Err(e) => return Err(ZapError::New(-1, e)),
    };
    let run_id = ast::generate_run_id();
    let log_path = ast::log_path_for(&run_id);
    ast::register_run(
        &run_id,
        "upgrade",
        &payload.pkg_path,
        &claims.sub,
        &log_path,
    )
    .await?;

    // 注入操作者上下文：面板登录用户与虚拟主机运行模式（固定为独立系统用户）
    let user = claims.sub.clone();
    let run_mode = system_env::VHOST_MODE.to_string();

    let resp = zapexec::call(Request::AppstoreUpgrade {
        pkg_path: payload.pkg_path.clone(),
        source: payload.source.clone(),
        repo_id: payload.repo_id.clone(),
        version: payload.version.clone(),
        old_version,
        action: payload.action.clone(),
        options,
        user: Some(user),
        run_mode: Some(run_mode),
        run_id: run_id.clone(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(&run_id, "failed", resp.code as i64).await;
        return Err(ZapError::New(resp.code, resp.message));
    }
    ast::watch_log(run_id.clone(), log_path.clone());
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_upgrade",
        &payload.pkg_path,
        &format!(
            "source={} version={} options=[{}]",
            payload.source,
            payload.version,
            payload
                .options
                .as_ref()
                .map(|o| o.keys().cloned().collect::<Vec<_>>().join(","))
                .unwrap_or_default()
        ),
    )
    .await;
    info!("AppStore upgrade started: {}", payload.pkg_path);
    Ok(Json(json!({
        "code": 0,
        "message": "升级已启动",
        "data": { "run_id": run_id, "log": log_path }
    })))
}

// ── 脚本管理 ────────────────────────────────────────────────

/// 递归构建自定义脚本树（路径相对 custom/）。
fn build_script_tree(dir: &std::path::Path, rel_base: &std::path::Path) -> Value {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(entry);
            } else if path.is_file() {
                files.push(entry);
            }
        }
    }
    dirs.sort_by_key(|e| e.file_name());
    files.sort_by_key(|e| e.file_name());
    let mut children: Vec<Value> = Vec::new();
    for entry in dirs {
        children.push(build_script_tree(&entry.path(), rel_base));
    }
    for entry in files {
        let path = entry.path();
        let rel = path
            .strip_prefix(rel_base)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        children.push(json!({
            "type": "file",
            "name": entry.file_name().to_string_lossy(),
            "path": rel,
        }));
    }
    json!({
        "type": "dir",
        "name": name,
        "path": dir.strip_prefix(rel_base).map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "children": children,
    })
}

pub async fn scripts_tree(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    user_cron::safe_username(&claims.sub)?;
    // 自定义脚本按用户隔离：{data}/users/<username>/scripts/
    // 路径统一相对**用户目录**（如 scripts/backup.sh），与 script_read/write 契约一致
    let base = user_cron::users_dir().join(&claims.sub);
    let root = base.join("scripts");
    // 首次访问即把该用户的脚本目录建出来（与 crontab.yaml、cloud/ 同级）
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| ZapError::New(-1, format!("创建脚本目录失败: {e}")))?;
    let tree = tokio::task::spawn_blocking(move || {
        if root.is_dir() {
            build_script_tree(&root, &base)
        } else {
            Value::Null
        }
    })
    .await
    .unwrap_or(Value::Null);
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": { "tree": tree } }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct ScriptPathQuery {
    pub path: String,
}

pub async fn script_read(
    claims: ValidatedClaims,
    Query(q): Query<ScriptPathQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    user_cron::safe_username(&claims.sub)?;
    validate_script_path(&q.path)?;
    let resp = zapexec::call(Request::AppstoreScriptRead {
        path: q.path.clone(),
        username: claims.sub.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": resp.data }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct ScriptWritePayload {
    pub path: String,
    pub content: String,
}

pub async fn script_write(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ScriptWritePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    user_cron::safe_username(&claims.sub)?;
    validate_script_path(&payload.path)?;
    let resp = zapexec::call(Request::AppstoreScriptWrite {
        path: payload.path.clone(),
        content: payload.content.clone(),
        username: claims.sub.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_script_write",
        &payload.path,
        "",
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "保存成功", "data": resp.data }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct ScriptDeletePayload {
    pub path: String,
}

/// 删除自定义脚本/目录（仅限 scripts/ 下；脚本根目录本身不可删）。
pub async fn script_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ScriptDeletePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    user_cron::safe_username(&claims.sub)?;
    validate_script_path(&payload.path)?;
    let resp = zapexec::call(Request::AppstoreScriptDelete {
        path: payload.path.clone(),
        username: claims.sub.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_script_delete",
        &payload.path,
        "",
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "删除成功", "data": resp.data }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct ScriptRunPayload {
    pub path: String,
}

pub async fn script_run(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ScriptRunPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    user_cron::safe_username(&claims.sub)?;
    validate_script_path(&payload.path)?;
    let run_id = ast::generate_run_id();
    let log_path = ast::log_path_for(&run_id);
    ast::register_run(&run_id, "script", &payload.path, &claims.sub, &log_path).await?;

    let resp = zapexec::call(Request::AppstoreScriptRun {
        path: payload.path.clone(),
        run_id: run_id.clone(),
        username: claims.sub.clone(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(&run_id, "failed", resp.code as i64).await;
        return Err(ZapError::New(resp.code, resp.message));
    }
    ast::watch_log(run_id.clone(), log_path.clone());
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_script_run",
        &payload.path,
        "",
    )
    .await;
    info!("AppStore script run started: {}", payload.path);
    Ok(Json(json!({
        "code": 0,
        "message": "脚本已启动",
        "data": { "run_id": run_id, "log": log_path }
    })))
}

#[derive(Debug, Deserialize)]
pub struct ScriptStopPayload {
    pub run_id: String,
}

pub async fn script_stop(
    claims: ValidatedClaims,
    Json(payload): Json<ScriptStopPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = zapexec::call(Request::AppstoreScriptStop {
        run_id: payload.run_id.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": "已发送停止信号" })))
}

// ── 运行快照（失败后查看/编辑脚本并重跑）─────────────────────

#[derive(Debug, Deserialize)]
pub struct RunFilesQuery {
    pub run_id: String,
}

/// 列出一次运行的可编辑脚本快照文件树（runs/<run_id>/pkg/）。
pub async fn run_files(claims: ValidatedClaims, Query(q): Query<RunFilesQuery>) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = zapexec::call(Request::AppstoreRunFiles {
        run_id: q.run_id.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": resp.data }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct RunFileReadQuery {
    pub run_id: String,
    pub path: String,
}

/// 读取运行快照内文件内容（编辑前查看）。
pub async fn run_file_read(
    claims: ValidatedClaims,
    Query(q): Query<RunFileReadQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = zapexec::call(Request::AppstoreRunFileRead {
        run_id: q.run_id.clone(),
        path: q.path.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": resp.data }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct RunFileWritePayload {
    pub run_id: String,
    pub path: String,
    pub content: String,
}

/// 写入运行快照内文件（修改脚本后保存；会改变待执行内容，仅管理员）。
pub async fn run_file_write(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<RunFileWritePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = zapexec::call(Request::AppstoreRunFileWrite {
        run_id: payload.run_id.clone(),
        path: payload.path.clone(),
        content: payload.content.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_run_file_write",
        &format!("{}:{}", payload.run_id, payload.path),
        "",
    )
    .await;
    info!(
        "AppStore run snapshot file written: {}:{}",
        payload.run_id, payload.path
    );
    Ok(Json(
        json!({ "code": 0, "message": "已保存", "data": resp.data }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct RunRetryPayload {
    pub run_id: String,
}

/// 重跑某次失败的运行：复用其快照（含已编辑脚本），以新的 run_id 重新执行。
/// 快照内脚本会以 root 执行，仅管理员可操作。
pub async fn run_retry(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<RunRetryPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    // 原运行记录必须存在，以便向 runs 表登记新的重跑记录
    let run = ast::get_run(&payload.run_id)
        .await?
        .ok_or_else(|| ZapError::New(-1, "原任务不存在，无法重跑".to_string()))?;
    let new_run_id = ast::generate_run_id();
    let log_path = ast::log_path_for(&new_run_id);
    ast::register_run(&new_run_id, &run.action, &run.pkg, &claims.sub, &log_path).await?;

    let resp = zapexec::call(Request::AppstoreRunRetry {
        run_id: payload.run_id.clone(),
        new_run_id: new_run_id.clone(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(&new_run_id, "failed", resp.code as i64).await;
        return Err(ZapError::New(resp.code, resp.message));
    }
    ast::watch_log(new_run_id.clone(), log_path.clone());
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_run_retry",
        &payload.run_id,
        &format!("new_run_id={new_run_id}"),
    )
    .await;
    info!(
        "AppStore run retry started: {} -> {}",
        payload.run_id, new_run_id
    );
    Ok(Json(json!({
        "code": 0,
        "message": "重跑已启动",
        "data": {
            "run_id": new_run_id,
            "log": log_path,
            "original_run_id": payload.run_id
        }
    })))
}

// ── 运行记录 / 日志 ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

pub async fn runs(claims: ValidatedClaims, Query(q): Query<RunsQuery>) -> ZapJsonResult {
    // 运行日志按归属用户隔离：非管理员只看得到自己（reseller 含名下客户）的记录
    let (rows, total) =
        ast::list_runs_for(&claims, q.page.unwrap_or(1), q.page_size.unwrap_or(20)).await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "run_id": r.run_id,
                "action": r.action,
                "pkg": r.pkg,
                "username": r.username,
                "status": r.status,
                "exit_code": r.exit_code,
                "log_path": r.log_path,
                "started_at": r.started_at,
                "finished_at": r.finished_at,
            })
        })
        .collect();
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": { "items": items, "total": total } }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub offset: Option<u64>,
}

pub async fn log(
    claims: ValidatedClaims,
    Path(run_id): Path<String>,
    Query(q): Query<LogQuery>,
) -> ZapJsonResult {
    // 归属校验：越权读取他人安装日志一律拒绝
    let run = ast::ensure_run_access(&claims, &run_id).await?;
    let (content, exit_code, done) = ast::read_log(&run.log_path, q.offset.unwrap_or(0)).await?;
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": { "content": content, "exit_code": exit_code, "done": done }
    })))
}

// ── WebSocket 实时日志 ──────────────────────────────────────

pub async fn ws_log(
    ws: WebSocketUpgrade,
    Path(run_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(token) = params.get("token").cloned() else {
        return unauthorized("Missing token");
    };
    // 克隆密钥后立即释放锁：RwLockReadGuard 非 Send，跨 await 会让 handler future 非 Send
    let secure_key = config::get_config().read().unwrap().jwt.jwt_secure.clone();
    let claims = match decode::<Claims>(
        &token,
        &DecodingKey::from_secret(secure_key.as_bytes()),
        &Validation::default(),
    ) {
        Ok(data) => data.claims,
        Err(_) => return unauthorized("Invalid token"),
    };
    // 归属校验：实时日志同样按归属用户隔离，不能凭 run_id 串看他人安装输出。
    // 校验失败也照常升级，再回一条可读的 error 帧：直接回 403 时浏览器只会触发
    // onerror，前端拿不到任何原因，只能显示含糊的「连接错误」。
    let access = ast::ensure_run_access(&claims, &run_id).await;
    ws.on_upgrade(move |mut socket| async move {
        match access {
            Ok(_) => handle_ws_log(socket, run_id).await,
            Err(e) => {
                let _ = socket
                    .send(Message::Text(Utf8Bytes::from(
                        json!({ "type": "error", "message": e.to_string() }).to_string(),
                    )))
                    .await;
                let _ = socket.close().await;
            }
        }
    })
}

/// WebSocket 握手失败的统一响应（升级前返回，前端表现为连接失败）。
fn unauthorized(message: &str) -> axum::response::Response {
    axum::response::Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(axum::body::Body::from(message.to_string()))
        .unwrap()
}

// ── 已安装应用（实例管理）───────────────────────────────────

/// 已安装应用列表：root 侧扫描 apps/*/meta.yaml + info.yaml 并探测运行状态。
pub async fn installed_apps(_claims: ValidatedClaims) -> ZapJsonResult {
    let resp = zapexec::call(Request::AppstoreInstalled).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": resp.data }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct InstanceActionPayload {
    /// 形如 application/php 的包路径
    pub pkg_path: String,
    /// start | stop | restart
    pub action: String,
}

/// 对已安装应用的实例执行启停（仅管理员；要求脚本登记 svc_name）。
pub async fn instance_action(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<InstanceActionPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if !["start", "stop", "restart"].contains(&payload.action.as_str()) {
        return Err(ZapError::New(-1, "不支持的实例操作".to_string()));
    }
    let resp = zapexec::call(Request::AppstoreInstanceAction {
        pkg_path: payload.pkg_path.clone(),
        action: payload.action.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "appstore_instance_action",
        &payload.pkg_path,
        &payload.action,
    )
    .await;
    info!(
        "AppStore instance action {}: {}",
        payload.action, payload.pkg_path
    );
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": resp.data }),
    ))
}

async fn handle_ws_log(mut socket: WebSocket, run_id: String) {
    info!("AppStore log WebSocket connected: {run_id}");
    let Some(run) = ast::get_run(&run_id).await.unwrap_or(None) else {
        let _ = socket
            .send(Message::Text(Utf8Bytes::from(
                json!({ "type": "error", "message": "任务不存在" }).to_string(),
            )))
            .await;
        return;
    };
    let log_path = run.log_path;
    let mut offset: u64 = 0;

    loop {
        match ast::read_log(&log_path, offset).await {
            Ok((text, exit_code, done)) => {
                if !text.is_empty() {
                    // 去掉完成标记行，避免重复展示
                    let clean = if done {
                        ast::strip_done_marker(&text)
                    } else {
                        text.clone()
                    };
                    if !clean.is_empty()
                        && socket
                            .send(Message::Text(Utf8Bytes::from(
                                json!({ "type": "log", "data": clean }).to_string(),
                            )))
                            .await
                            .is_err()
                    {
                        return;
                    }
                    offset += text.len() as u64;
                }
                if done {
                    let status = if exit_code == Some(0) {
                        "success"
                    } else {
                        "failed"
                    };
                    let _ = socket
                        .send(Message::Text(Utf8Bytes::from(
                            json!({ "type": "done", "status": status, "exit_code": exit_code })
                                .to_string(),
                        )))
                        .await;
                    let _ = socket.close().await;
                    return;
                }
            }
            Err(e) => {
                error!("read appstore log {run_id} failed: {e}");
                let _ = socket.close().await;
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}
