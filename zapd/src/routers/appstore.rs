//! AppStore 路由：仓库管理 / 包安装卸载升级 / 脚本管理 / 运行记录与实时日志。

use std::{collections::BTreeMap, net::SocketAddr};

use axum::{
    Json,
    extract::{Extension, Path, Query},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use tracing::info;

use crate::{
    routers::system_env,
    zap::{
        ZapError, ZapJsonResult, appstore as ast, audit,
        jwt::{self, Claims, ValidatedClaims},
        task, user_cron,
    },
    zapexec,
};
use zap_proto::Request;

/// 运行身份门禁：声明 `run_as: user`（或 `scope: site`）的包必须是 `webapps` 分类。
///
/// 降权通道是给「装进用户站点目录」的建站包（WordPress 之类）用的：脚本以面板用户
/// 对应的 Linux 账号运行，拿不到 root。其它分类的脚本要装系统目录、管 systemd，
/// 不允许走这条通道（zapexec 侧会二次校验，这里先把错误提示前置到面板层）。
async fn check_pkg_run_as(pkg_path: &str) -> Result<(), ZapError> {
    let Some(mode) = ast::package_run_as_of(pkg_path).await else {
        return Ok(());
    };
    if mode != "user" {
        return Ok(());
    }
    let cat = pkg_path.split('/').next().unwrap_or("");
    if cat != "webapps" {
        return Err(ZapError::New(
            -1,
            "只有 webapps 分类的包允许以 Linux 用户身份运行（app.yaml: run_as: user）".to_string(),
        ));
    }
    Ok(())
}

// ── 面板侧编排（建站 / 建库）────────────────────────────────
//
// 建站类包（webapps）的脚本以 Linux 账号运行，拿不到「建站点」「建数据库」的权限。
// 因此由面板在任务入队前把资源准备好，再把连接信息作为环境变量注入脚本；
// 脚本只负责下载、落地、写配置文件。好处：
// - 数据库凭据（zapadm）不出面板进程，也不落 options.env（那是给用户看的）；
// - 套餐配额、多租户前缀（{用户名}_）、随机密码策略自动生效；
// - 重跑 / 卸载复用同一份资源（存于 apps/<pkg>/provision.json，0600 仅 root 可读）。

/// 编排结果存放位置：实例槽位下的 `provision.json`（root only，0600）。
///
/// 站点类槽位在用户私有目录里，全局类在 `apps/<category>/<name>/<instance>/`；
/// 查不到实例时回退旧布局，保证升级面板前装的应用依然能卸载 / 升级。
fn provision_file(pkg_path: &str, instance: Option<&str>) -> PathBuf {
    let legacy = ast::apps_dir().join(pkg_path).join("provision.json");
    if let Some(slot) = ast::find_slot(pkg_path, instance) {
        let p = slot.dir.join("provision.json");
        if p.is_file() {
            return p;
        }
    }
    legacy
}

/// 实例槽位目录；查不到就回退旧布局 `apps/<pkg_path>/`。
fn slot_dir_of(pkg_path: &str, instance: Option<&str>) -> PathBuf {
    ast::find_slot(pkg_path, instance)
        .map(|s| s.dir)
        .unwrap_or_else(|| ast::apps_dir().join(pkg_path))
}

/// 读取上次安装的编排结果（卸载 / 升级复用同一站点与库）。
///
/// provision.json 由 zapexec（root）以 0600 落盘：面板以 root 运行时能读，
/// 降权运行时读不到，所以这里只把它当「可选」来源，读不到就退到脚本登记的
/// info.yaml（0644）里取站点 / 库字段。info.yaml 没有密码，卸载脚本会自动
/// 跳过备份与删库，但仍能正确清理站点文件 —— 比直接报「缺少 SITE_ROOT」
/// 把卸载卡死要好。
pub(crate) fn load_provision(
    pkg_path: &str,
    instance: Option<&str>,
) -> Option<BTreeMap<String, String>> {
    if let Ok(content) = std::fs::read_to_string(provision_file(pkg_path, instance))
        && let Ok(env) = serde_json::from_str::<BTreeMap<String, String>>(&content)
        && !env.is_empty()
    {
        return Some(env);
    }
    provision_from_info(pkg_path, instance)
}

/// 从脚本登记的 info.yaml 还原站点 / 数据库字段（不含密码）。
fn provision_from_info(pkg_path: &str, instance: Option<&str>) -> Option<BTreeMap<String, String>> {
    let content =
        std::fs::read_to_string(slot_dir_of(pkg_path, instance).join("info.yaml")).ok()?;
    let v: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    for (key, dst) in [
        ("site_id", "SITE_ID"),
        ("site_root", "SITE_ROOT"),
        ("domain", "SITE_DOMAIN"),
        ("db_name", "DB_NAME"),
        ("db_user", "DB_USER"),
    ] {
        if let Some(s) = v
            .get(key)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
        {
            env.insert(dst.to_string(), (*s).to_string());
        }
    }
    if env.is_empty() { None } else { Some(env) }
}

/// 按包声明准备站点与数据库，返回待注入脚本的环境变量。
///
/// 失败即中止安装（任务不会入队），避免脚本跑到一半才发现没有数据库。
async fn provision_for(
    claims: &Claims,
    pkg_path: &str,
    options: Option<&BTreeMap<String, String>>,
) -> Result<Option<BTreeMap<String, String>>, ZapError> {
    let Some(spec) = ast::package_provision_of(pkg_path).await else {
        return Ok(None);
    };
    let mut env: BTreeMap<String, String> = BTreeMap::new();

    if let Some(site) = &spec.site {
        let opt_name = site
            .domain_option
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("SITE_DOMAIN");
        let domain = options
            .and_then(|o| o.get(opt_name))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ZapError::New(
                    -1,
                    format!("缺少安装选项 {opt_name}：建站包需要目标站点域名"),
                )
            })?;
        let s = crate::routers::site::provision_site(
            claims,
            domain,
            site.mode.as_deref(),
            site.php.as_deref(),
            site.rewrite.as_deref(),
        )
        .await?;
        env.insert("SITE_ID".into(), s.id.to_string());
        env.insert("SITE_DOMAIN".into(), s.domain);
        env.insert("SITE_ROOT".into(), s.root);
        env.insert("SITE_OWNER".into(), s.owner);
        env.insert("SITE_LINUX_USER".into(), s.linux_user);
        if !s.php_instance.is_empty() {
            env.insert("PHP_INSTANCE".into(), s.php_instance);
        }
        if let Some(sock) = s.php_socket {
            env.insert("PHP_FPM_SOCK".into(), sock);
        }
    }

    if let Some(db) = &spec.database {
        // 每个实例一个专用库 + 专用用户：只授权自己那个库，互不影响
        let created = crate::routers::database::provision_db(
            claims,
            db.name.as_deref(),
            db.charset.as_deref(),
            db.host.as_deref(),
        )
        .await?;
        env.insert("DB_NAME".into(), created.name);
        env.insert("DB_CHARSET".into(), created.charset);
        if let Some(u) = created.user {
            env.insert("DB_USER".into(), u);
        }
        if let Some(h) = created.host {
            // 该账号被授权的连接来源（默认 localhost），与 DB_HOST（连接地址）区分
            env.insert("DB_USER_HOST".into(), h);
        }
        // 明文密码只进脚本进程环境与 provision.json（0600），前端与 options.env 都没有
        if let Some(p) = created.password {
            env.insert("DB_PASS".into(), p);
        }
        env.insert(
            "DB_HOST".into(),
            crate::routers::database::DB_HOST.to_string(),
        );
        env.insert(
            "DB_PORT".into(),
            crate::routers::database::db_port().to_string(),
        );
    }

    if env.is_empty() {
        return Ok(None);
    }
    // 落盘交给 zapexec（root）在任务里做：webapps 槽位会被 chown 给站点账号，
    // 面板进程之后写不进去 —— 早期 save_provision 静默失败正是 provision.json
    // 缺失、卸载报「缺少 SITE_ROOT」的根因。
    Ok(Some(env))
}

/// 决定这次安装落在哪个实例槽位。
///
/// - 站点类（webapps）：`site:<站点id>` —— 同一站点重复安装是同一个实例，
///   不同站点各占一个槽位，第二个站点装 WordPress 不会再覆盖第一个的登记；
/// - 声明 `allow_multiple_instances` 的包：用版本短名（`7.4.33` → `74`），
///   多版本 PHP 各占一个槽位；
/// - 其余：None（zapexec 按 `default` 处理，与旧布局一致）。
async fn install_instance(
    pkg_path: &str,
    version: &str,
    provision: Option<&BTreeMap<String, String>>,
) -> Option<String> {
    use zap_proto::appstore::{Slot, WEBAPPS_CATEGORY};

    let cat = pkg_path.split('/').next().unwrap_or_default();
    if cat == WEBAPPS_CATEGORY
        && let Some(site_id) = provision
            .and_then(|p| p.get("SITE_ID"))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
    {
        return Some(Slot::site_instance(site_id));
    }
    if !ast::package_multi_instance_of(pkg_path)
        .await
        .unwrap_or(false)
    {
        return None;
    }
    let short: String = version.split('.').take(2).collect();
    if short.is_empty() { None } else { Some(short) }
}

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
    // 一个包可以有多个实例（多版本 PHP / 多站点 WordPress），所以是列表而不是单值
    let mut installed_map: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();
    for inst in &installed {
        if let Some(p) = inst.get("pkg_path").and_then(|x| x.as_str()) {
            installed_map
                .entry(p.to_string())
                .or_default()
                .push(inst.clone());
        }
    }
    let mut items: Vec<Value> = Vec::new();
    for mut pkg in pkgs {
        let pkg_path = pkg
            .get("pkg_path")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        if let Some(insts) = installed_map.get(&pkg_path) {
            if let Some(inst) = insts.first() {
                // 兼容旧字段：取第一个实例（多实例时看 installed_instances）
                pkg["installed"] = json!(true);
                pkg["installed_version"] = inst.get("version").cloned().unwrap_or(Value::Null);
                pkg["installed_source"] = inst.get("source").cloned().unwrap_or(Value::Null);
                pkg["installed_at"] = inst.get("installed_at").cloned().unwrap_or(Value::Null);
                pkg["upgraded_from"] = inst.get("upgraded_from").cloned().unwrap_or(Value::Null);
            }
            // 全部实例：前端据此显示「已装 N 个实例」并逐个操作
            pkg["installed_instances"] = json!(
                insts
                    .iter()
                    .map(|i| {
                        json!({
                            "instance": i.get("instance"),
                            "instance_key": i.get("instance_key"),
                            "version": i.get("version"),
                            "owner": i.get("owner"),
                            "site_id": i.get("site_id"),
                        })
                    })
                    .collect::<Vec<_>>()
            );
        } else {
            pkg["installed"] = json!(false);
            pkg["installed_instances"] = json!([]);
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
    // 运行身份门禁：run_as: user 仅 webapps 分类可用
    check_pkg_run_as(&payload.pkg_path).await?;
    // 自定义包包含任意脚本，仅管理员可安装
    if payload.source == "custom" {
        require_admin(&claims)?;
    }
    let options = match sanitize_options(payload.options.clone()) {
        Ok(o) => o,
        Err(e) => return Err(ZapError::New(-1, e)),
    };
    // 面板侧编排：建站类包（webapps）先由面板建好站点 / 数据库；
    // 失败直接返回，任务不会入队（避免脚本跑一半发现没库可用）
    let provision = provision_for(&claims, &payload.pkg_path, options.as_ref()).await?;
    // 决定这次安装落在哪个实例槽位（多版本 / 多站点各占一个槽位，互不覆盖）
    let instance = install_instance(&payload.pkg_path, &payload.version, provision.as_ref()).await;
    let run_id = ast::generate_run_id();
    let log_path = ast::log_path_for(&run_id);

    // 注入操作者上下文：面板登录用户与虚拟主机运行模式（固定为独立系统用户）
    let req = Request::AppstoreInstall {
        pkg_path: payload.pkg_path.clone(),
        source: payload.source.clone(),
        repo_id: payload.repo_id.clone(),
        version: payload.version.clone(),
        action: payload.action.clone(),
        options,
        instance: instance.clone(),
        provision,
        user: Some(claims.sub.clone()),
        run_mode: Some(system_env::VHOST_MODE.to_string()),
        run_id: run_id.clone(),
    };
    // 编译型任务走并发组：同一时刻只允许一个编译在跑，后到的排队等自动放行
    let run = ast::enqueue_compile(
        &run_id,
        "install",
        &payload.pkg_path,
        &claims.sub,
        &log_path,
        &serde_json::to_string(&req).unwrap_or_default(),
    )
    .await?;
    // 拿到槽位就立即启动；排队中的留给调度器（等前一个结束后自动跑）
    if run.status == task::STATUS_RUNNING {
        task::launch(&run).await?;
    }
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
        "data": {
            "run_id": run_id,
            "log": log_path,
            "status": run.status,
            "queued": run.status == task::STATUS_PENDING,
            "position": task::queue_position(&run).await,
        }
    })))
}

#[derive(Debug, Deserialize)]
pub struct UninstallPayload {
    pub pkg_path: String,
    /// 实例名（缺省 default；站点类为 `site:<id>`）：多实例时指明卸掉哪一个
    pub instance: Option<String>,
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

    // 回传安装时的编排结果（站点 / 数据库），供 uninstall.sh 先备份数据再删文件
    let provision = load_provision(&payload.pkg_path, payload.instance.as_deref());
    let resp = zapexec::call(Request::AppstoreUninstall {
        pkg_path: payload.pkg_path.clone(),
        options,
        instance: payload.instance.clone(),
        provision,
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
    /// 实例名（缺省 default；站点类为 `site:<id>`）：多实例时指明升级哪一个
    pub instance: Option<String>,
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
    // 运行身份门禁：run_as: user 仅 webapps 分类可用
    check_pkg_run_as(&payload.pkg_path).await?;
    // 版本要按实例读：多版本 PHP 各自有 meta.yaml，只认 pkg_path 会读到错的那个
    let old_version = ast::installed_version_for(&payload.pkg_path, payload.instance.as_deref())
        .await
        .unwrap_or_default();
    let options = match sanitize_options(payload.options.clone()) {
        Ok(o) => o,
        Err(e) => return Err(ZapError::New(-1, e)),
    };
    let run_id = ast::generate_run_id();
    let log_path = ast::log_path_for(&run_id);

    let req = Request::AppstoreUpgrade {
        pkg_path: payload.pkg_path.clone(),
        source: payload.source.clone(),
        repo_id: payload.repo_id.clone(),
        version: payload.version.clone(),
        old_version,
        action: payload.action.clone(),
        options,
        instance: payload.instance.clone(),
        provision: load_provision(&payload.pkg_path, payload.instance.as_deref()),
        user: Some(claims.sub.clone()),
        run_mode: Some(system_env::VHOST_MODE.to_string()),
        run_id: run_id.clone(),
    };
    // 升级同样是编译型任务，受同一把"全局只允许一个"的约束（后到的排队）
    let run = ast::enqueue_compile(
        &run_id,
        "upgrade",
        &payload.pkg_path,
        &claims.sub,
        &log_path,
        &serde_json::to_string(&req).unwrap_or_default(),
    )
    .await?;
    if run.status == task::STATUS_RUNNING {
        task::launch(&run).await?;
    }
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
        "data": {
            "run_id": run_id,
            "log": log_path,
            "status": run.status,
            "queued": run.status == task::STATUS_PENDING,
            "position": task::queue_position(&run).await,
        }
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
    let req = Request::AppstoreRunRetry {
        run_id: payload.run_id.clone(),
        new_run_id: new_run_id.clone(),
    };
    let req_json = serde_json::to_string(&req).unwrap_or_default();

    // 重跑安装 / 升级本质上还是一次编译：走同一个并发组，前面有编译在跑就排队，
    // 由调度器在前一个结束后放行（启动参数随记录落库）。
    let compile_task = if matches!(run.action.as_str(), "install" | "upgrade") {
        Some(
            ast::enqueue_compile(
                &new_run_id,
                &run.action,
                &run.pkg,
                &claims.sub,
                &log_path,
                &req_json,
            )
            .await?,
        )
    } else {
        // 脚本 / 卸载等不做互斥，登记后直接下发
        ast::register_run(&new_run_id, &run.action, &run.pkg, &claims.sub, &log_path).await?;
        None
    };

    let (queued, position) = match &compile_task {
        // 拿到编译槽位：立即启动
        Some(t) if t.status == task::STATUS_RUNNING => {
            task::launch(t).await?;
            (false, 0)
        }
        // 前面还有编译：留给调度器，返回排队位次给前端提示
        Some(t) => (true, task::queue_position(t).await),
        None => {
            let resp = zapexec::call(req).await?;
            if resp.code != 0 {
                ast::finish_run(&new_run_id, "failed", resp.code as i64).await;
                return Err(ZapError::New(resp.code, resp.message));
            }
            ast::watch_log(new_run_id.clone(), log_path.clone());
            (false, 0)
        }
    };
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
            "original_run_id": payload.run_id,
            "queued": queued,
            "position": position
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
    // to_json() 同时给出 task_id 与 run_id：老前端读 run_id，新任务页读 task_id
    let items: Vec<Value> = rows.iter().map(|r| r.to_json()).collect();
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

// WebSocket 实时日志已收进通用任务队列路由 `/task/ws/{task_id}`
// （`/appstore/ws/{run_id}` 作为兼容路径指向同一实现，见 routers/mod.rs）。

// ── 已安装应用（实例管理）───────────────────────────────────

/// 已安装应用列表：root 侧扫描 apps/*/meta.yaml + info.yaml 并探测运行状态。
pub async fn installed_apps(claims: ValidatedClaims) -> ZapJsonResult {
    let resp = zapexec::call(Request::AppstoreInstalled).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    let mut data = resp.data.unwrap_or_else(|| json!({}));
    // 执行端可能还是旧版本（不返回 owner / instance_key）：用面板本地扫描补齐，
    // 保证「只看到自己的站点应用」这条隔离规则不依赖两端同时升级。
    for it in data
        .get_mut("items")
        .and_then(|v| v.as_array_mut())
        .unwrap_or(&mut Vec::new())
    {
        let pkg = it
            .get("pkg_path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let inst = it
            .get("instance")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if let Some(slot) = ast::find_slot(pkg, inst.as_deref()) {
            it["instance_key"] = json!(slot.key());
            if it.get("owner").map(|v| v.is_null()).unwrap_or(true) {
                it["owner"] = json!(slot.owner);
            }
            if it.get("site_id").map(|v| v.is_null()).unwrap_or(true) {
                it["site_id"] = json!(slot.site_id);
            }
        }
    }
    // 按用户隔离：站点类应用（webapps）跟着站点账号走，只给归属者看；
    // 全局类（nginx / php …）是系统级服务，所有人可见，但启停仍需管理员权限。
    if !jwt::is_admin(&claims) {
        let me = claims.sub.as_str();
        if let Some(items) = data.get_mut("items").and_then(|v| v.as_array_mut()) {
            items.retain(|it| match it.get("owner").and_then(|o| o.as_str()) {
                Some(owner) => owner == me,
                None => true,
            });
        }
    }
    Ok(Json(json!({ "code": 0, "message": "OK", "data": data })))
}

#[derive(Debug, Deserialize)]
pub struct InstanceActionPayload {
    /// 形如 application/php 的包路径
    pub pkg_path: String,
    /// 实例名（缺省 default）：多实例时指明操作哪一个
    pub instance: Option<String>,
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
        instance: payload.instance.clone(),
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
