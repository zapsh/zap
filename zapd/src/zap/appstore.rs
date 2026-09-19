//! AppStore：本地目录扫描（包 / 已安装实例）+ 安装任务在通用任务队列上的登记。
//!
//! 任务本身（登记、状态机、日志、可见性）已抽到 [`crate::zap::task`]，全站共用；
//! 本模块只保留 AppStore 的专属约定：日志落在 `appstore/logs/`、运行快照落在
//! `appstore/runs/<task_id>/`，以及历史记录的裁剪规则。

use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use tracing::{info, warn};

use crate::{
    config, db,
    zap::{ZapError, jwt},
};

/// 日志结束标记（`__ZAP_DONE__ <exit_code>`，zapexec 写入）：与通用任务队列同源。
pub const DONE_MARKER: &str = crate::zap::task::DONE_MARKER;

/// AppStore 任务在 `task_queue.kind` 里的大类标记。
pub const KIND: &str = crate::zap::task::KIND_APPSTORE;

/// 并发互斥组：源码编译型任务（安装 / 升级）**全局同一时刻只允许一个**。
///
/// 编译吃满 CPU 与内存，并行只会互相拖慢、还让日志交叉难以排查，
/// 所以后到的请求直接拒绝并提示"已有编译任务在进行"，
/// 而不是悄悄排队（排队需要"结束后自动启动下一个"的调度器，暂不引入）。
pub const COMPILE_GROUP: &str = "appstore:compile";

/// 日志监控超时：编译安装可能跑很久，给足 24 小时。
const WATCH_TIMEOUT_SECS: u64 = 24 * 3600;

/// 单个计划任务保留的运行记录条数上限（超出后自动清理最旧的）。
///
/// 一分钟一次的任务一天就是 1440 条；不设上限的话 `task_queue` 表与
/// `appstore/logs/` 目录会一起无限增长。
pub const MAX_RUNS_PER_JOB: i64 = 50;

/// 全表运行记录兜底上限（涵盖手动运行脚本、安装/更新等所有来源）。
pub const MAX_RUNS_TOTAL: i64 = 1000;

/// 运行记录就是通用任务队列里的一条任务。
///
/// 保留别名是因为升级、计划任务、Docker 构建等调用点历史上都以 `AppstoreRun`
/// 引用它；字段与查询能力见 [`crate::zap::task::Task`]。
pub type AppstoreRun = crate::zap::task::Task;

/// AppStore 根目录（与 zap.db 同级的 appstore/）
pub fn appstore_dir() -> PathBuf {
    let cfg = config::get_config().read().unwrap();
    let db_path = Path::new(&cfg.db.path);
    db_path
        .parent()
        .map(|p| p.join("appstore"))
        .unwrap_or_else(|| PathBuf::from("data/appstore"))
}

/// 已安装软件目录（apps/）
pub fn apps_dir() -> PathBuf {
    let cfg = config::get_config().read().unwrap();
    let db_path = Path::new(&cfg.db.path);
    db_path
        .parent()
        .map(|p| p.join("apps"))
        .unwrap_or_else(|| PathBuf::from("data/apps"))
}

pub fn logs_dir() -> PathBuf {
    appstore_dir().join("logs")
}

pub fn log_path_for(run_id: &str) -> String {
    crate::zap::task::log_path_in(&logs_dir(), run_id)
}

/// 生成任务号（与通用队列同源：同一个号既是 `task_id`，也是传给 zapexec 的 `run_id`）。
pub fn generate_run_id() -> String {
    crate::zap::task::new_id()
}

/// 登记一条运行记录（status=running）。
///
/// `job_key` 非空表示由定时任务触发，既是历史列表的查询条件，也是按任务
/// 保留数量的依据：`cron:<username>:<id>`（管理员计划任务）或
/// `crontab:<username>:<id>`（用户计划任务）；非任务触发传空串。
/// 登记后会异步裁剪历史，避免无限增长。
pub async fn register_run_with_key(
    run_id: &str,
    action: &str,
    pkg: &str,
    username: &str,
    log_path: &str,
    job_key: &str,
) -> Result<(), ZapError> {
    crate::zap::task::enqueue(crate::zap::task::NewTask {
        task_id: run_id.to_string(),
        kind: KIND.to_string(),
        action: action.to_string(),
        pkg: pkg.to_string(),
        username: username.to_string(),
        title: String::new(),
        log_path: log_path.to_string(),
        job_key: job_key.to_string(),
        // AppStore 的安装 / 升级不做全局互斥（同一时刻只允许一个编译的任务
        // 由调用方显式传 group_key 控制，见 docker_build 等后续接入点）
        group_key: String::new(),
        group_limit: 0,
    })
    .await?;
    // 裁剪在后台跑，不拖慢本次触发；无归属的运行只受全表上限约束
    if !job_key.is_empty() {
        let key = job_key.to_string();
        tokio::spawn(async move { prune_runs(&key).await });
    }
    Ok(())
}

/// 登记一条与定时任务无关的运行记录（`job_key` 为空）。
pub async fn register_run(
    run_id: &str,
    action: &str,
    pkg: &str,
    username: &str,
    log_path: &str,
) -> Result<(), ZapError> {
    register_run_with_key(run_id, action, pkg, username, log_path, "").await
}

/// 登记一条**编译型**运行记录（安装 / 升级）：同一时刻全局只允许一个。
///
/// 两步判定：先用组内活跃数快速失败（含排队中的，避免并发提交一起挤进来），
/// 登记后若发现自己是排队的（并发窗口里被抢先），撤掉这条并给出同样的提示。
pub async fn register_compile_run(
    run_id: &str,
    action: &str,
    pkg: &str,
    username: &str,
    log_path: &str,
) -> Result<(), ZapError> {
    if crate::zap::task::active_in_group(COMPILE_GROUP).await >= 1 {
        return Err(ZapError::New(
            -1,
            "已有编译任务在进行中，请等它结束后再试".to_string(),
        ));
    }
    let t = crate::zap::task::enqueue(crate::zap::task::NewTask {
        task_id: run_id.to_string(),
        kind: KIND.to_string(),
        action: action.to_string(),
        pkg: pkg.to_string(),
        username: username.to_string(),
        title: format!("{action} {pkg}"),
        log_path: log_path.to_string(),
        job_key: String::new(),
        group_key: COMPILE_GROUP.to_string(),
        group_limit: 1,
    })
    .await?;
    if t.status != crate::zap::task::STATUS_RUNNING {
        let _ = crate::zap::task::delete(&t.task_id).await;
        return Err(ZapError::New(
            -1,
            "已有编译任务在进行中，请等它结束后再试".to_string(),
        ));
    }
    Ok(())
}

/// 列出某个定时任务最近的运行记录（新的在前）。
pub async fn list_runs_by_key(job_key: &str, limit: i64) -> Result<Vec<AppstoreRun>, sqlx::Error> {
    let filter = crate::zap::task::Filter {
        job_key: Some(job_key.to_string()),
        ..Default::default()
    };
    Ok(crate::zap::task::list(filter, 1, limit.clamp(1, 200))
        .await?
        .0)
}

/// 清空某个定时任务的全部运行历史（记录 + 日志 + 快照），返回删除条数。
pub async fn delete_runs_by_key(job_key: &str) -> Result<i64, sqlx::Error> {
    let pool = db::get_db_pool().await;
    let rows = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT id, task_id, log_path FROM task_queue WHERE job_key = ?",
    )
    .bind(job_key)
    .fetch_all(pool)
    .await?;
    let n = rows.len() as i64;
    for (_, run_id, log_path) in &rows {
        remove_run_artifacts(run_id, log_path);
    }
    sqlx::query("DELETE FROM task_queue WHERE job_key = ?")
        .bind(job_key)
        .execute(pool)
        .await?;
    Ok(n)
}

/// 裁剪运行记录：按任务保留最近 `MAX_RUNS_PER_JOB` 条，全表兜底保留最近
/// `MAX_RUNS_TOTAL` 条；被裁掉的记录连同日志文件、运行快照一起删除。
///
/// 只删"超出保留范围的旧记录"，正在运行的一定是最新的一条，不会被误删。
pub async fn prune_runs(job_key: &str) {
    prune_keep_recent(Some(job_key), MAX_RUNS_PER_JOB).await;
    prune_keep_recent(None, MAX_RUNS_TOTAL).await;
}

/// 删除超出 `keep` 条的旧记录；`scope` 为 None 时按全表裁剪。
async fn prune_keep_recent(scope: Option<&str>, keep: i64) {
    let pool = db::get_db_pool().await;
    let rows = match scope {
        Some(key) => {
            sqlx::query_as::<_, (i64, String, String)>(
                "SELECT id, task_id, log_path FROM task_queue \
                 WHERE job_key = ? ORDER BY started_at DESC, id DESC LIMIT -1 OFFSET ?",
            )
            .bind(key)
            .bind(keep)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, (i64, String, String)>(
                "SELECT id, task_id, log_path FROM task_queue \
                 ORDER BY started_at DESC, id DESC LIMIT -1 OFFSET ?",
            )
            .bind(keep)
            .fetch_all(pool)
            .await
        }
    };
    let stale = match rows {
        Ok(r) => r,
        Err(e) => {
            warn!("查询待清理的运行记录失败: {e}");
            return;
        }
    };
    if stale.is_empty() {
        return;
    }
    // 先按 log_path 删磁盘产物，再一条 SQL 批量删记录：
    // 存量可能很大（长年未清理），逐条 DELETE 会明显拖慢后台任务
    for (_, run_id, log_path) in &stale {
        remove_run_artifacts(run_id, log_path);
    }
    let deleted = match scope {
        Some(key) => {
            sqlx::query(
                "DELETE FROM task_queue WHERE id IN (\
                 SELECT id FROM task_queue WHERE job_key = ? \
                 ORDER BY started_at DESC, id DESC LIMIT -1 OFFSET ?)",
            )
            .bind(key)
            .bind(keep)
            .execute(pool)
            .await
        }
        None => {
            sqlx::query(
                "DELETE FROM task_queue WHERE id IN (\
                 SELECT id FROM task_queue \
                 ORDER BY started_at DESC, id DESC LIMIT -1 OFFSET ?)",
            )
            .bind(keep)
            .execute(pool)
            .await
        }
    };
    if let Err(e) = deleted {
        warn!("清理运行记录失败: {e}");
        return;
    }
    info!(
        "已清理 {} 条历史运行记录（保留最近 {keep} 条）",
        stale.len()
    );
    // 被裁掉的可能是某任务的「上次运行」：断开引用，避免查看日志时指向空记录
    crate::zap::script_cron::clear_dangling_last_run_ids().await;
}

/// 删除一次运行留下的磁盘产物：日志文件 + 运行快照目录。
///
/// 日志路径直接取自 DB 的 `log_path`；快照目录按 run_id 拼出，run_id 是
/// 十六进制串不含路径分隔符，拼接安全。删除失败（已被清理/不在本机）忽略。
fn remove_run_artifacts(run_id: &str, log_path: &str) {
    if !log_path.is_empty() {
        let _ = std::fs::remove_file(log_path);
    }
    if !run_id.is_empty() {
        let _ = std::fs::remove_dir_all(appstore_dir().join("runs").join(run_id));
    }
}

pub async fn finish_run(run_id: &str, status: &str, exit_code: i64) {
    // 返回值是"同组下一个待放行任务"：AppStore 未设并发组，忽略
    let _ = crate::zap::task::finish(run_id, status, exit_code).await;
}

pub async fn get_run(run_id: &str) -> Result<Option<AppstoreRun>, sqlx::Error> {
    crate::zap::task::get(run_id).await
}

pub async fn list_runs(page: i64, page_size: i64) -> Result<(Vec<AppstoreRun>, i64), sqlx::Error> {
    crate::zap::task::list(Default::default(), page, page_size).await
}

// ── 运行记录归属 / 可见性 ─────────────────────────────────

/// 运行日志含脚本输出、绝对路径、配置片段甚至凭据回显，必须按归属用户隔离：
/// admin → 全部；reseller → 自己 + 名下客户；普通用户 → 仅自己。
///
/// 越权一律返回错误，且**不区分"任务不存在"与"无权访问"以外的细节**。
pub async fn ensure_run_access(
    claims: &jwt::Claims,
    run_id: &str,
) -> Result<AppstoreRun, ZapError> {
    crate::zap::task::ensure_access(claims, run_id).await
}

/// 按可见范围分页列出运行记录：非管理员只看得到自己（reseller 含名下客户）的记录。
pub async fn list_runs_for(
    claims: &jwt::Claims,
    page: i64,
    page_size: i64,
) -> Result<(Vec<AppstoreRun>, i64), sqlx::Error> {
    crate::zap::task::list_for(claims, Default::default(), page, page_size).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(sub: &str, roles: &str) -> jwt::Claims {
        jwt::Claims {
            id: 1,
            iat: 0,
            sub: sub.to_string(),
            iss: "Zap".to_string(),
            exp: u64::MAX,
            roles: roles.to_string(),
            pwd_is_default: false,
        }
    }

    /// 不依赖数据库的可见性分支：admin 全部 / 本人 / 其他用户拒绝。
    /// （reseller 分支需查 `user.owner_id`，由集成环境覆盖。）
    #[tokio::test]
    async fn run_visibility_rules() {
        let admin = claims("admin", "admin");
        let alice = claims("alice", "user");

        assert!(
            crate::zap::task::user_visible(&admin, "alice")
                .await
                .unwrap()
        );
        assert!(
            crate::zap::task::user_visible(&alice, "alice")
                .await
                .unwrap()
        );
        assert!(!crate::zap::task::user_visible(&alice, "bob").await.unwrap());
    }
}

/// 后台监控日志直到出现 `__ZAP_DONE__ <code>`，随后更新运行状态。
pub fn watch_log(run_id: String, log_path: String) {
    crate::zap::task::watch_log(run_id, log_path, WATCH_TIMEOUT_SECS);
}

/// 读取日志 offset 之后的内容，同时返回是否已完成。
pub async fn read_log(
    log_path: &str,
    offset: u64,
) -> Result<(String, Option<i64>, bool), ZapError> {
    crate::zap::task::read_log(log_path, offset).await
}

/// 去掉日志尾部完成标记，供最终展示。
pub fn strip_done_marker(content: &str) -> String {
    crate::zap::task::strip_done_marker(content)
}

// ── 本地目录扫描（包列表 / 已安装列表）──────────────────────

#[derive(Debug, Default, serde::Deserialize)]
struct AppYaml {
    name: Option<String>,
    /// app.yaml 的 version 可能是单个字符串或数组（如 `[8.3.3,7.4.33]`），
    /// 数组表示该包支持安装的多个版本，首个为默认版本。
    #[serde(default)]
    version: Versions,
    category: Option<String>,
    title: Option<String>,
    description: Option<String>,
    /// 兼容旧写法：deps 为依赖名列表
    deps: Option<Vec<String>>,
    /// dependencies 映射：依赖库名 -> 版本/要求（如 openssl: 1.1.1w）
    #[serde(default)]
    dependencies: Option<Value>,
    /// 版本 → 附加元数据（如 `"8.0.46": {family: mysql}`），
    /// 合并入口（MySQL / MariaDB 等）据此在版本下拉中分组 / 标注家族。
    #[serde(default)]
    version_meta: Option<Value>,
    /// 自定义操作按钮：动作键 -> 按钮文案（如 build: 编译安装 / bin: 安装）
    #[serde(default)]
    actions: Option<Value>,
    /// 安装/升级可选项：动作键 -> 选项定义列表（形如 {prefix: {...}}）。
    /// 顶层直接为列表时视为作用于 install 动作。
    #[serde(default)]
    options: Option<Value>,
    /// 是否允许多实例安装（为 yes 时即使已安装也可再次安装其他版本）
    #[serde(default, deserialize_with = "de_boolish")]
    allow_multiple_instances: bool,
    default_port: Option<u16>,
    #[serde(default)]
    scripts: Option<Value>,
    /// 可浏览/安装此包的角色白名单（如 [admin, user]）；空或未声明 = 仅 admin 可见可操作。
    /// 声明后 admin 恒可见可操作，命中白名单的角色同样可见可安装
    /// （install/upgrade 后端二次校验）。
    #[serde(default, deserialize_with = "de_str_list")]
    roles: Option<Vec<String>>,
}

/// version 字段解析结果：默认版本 + 全部可安装版本。
#[derive(Debug, Default, Clone)]
struct Versions {
    /// 默认版本（数组首个或单值本身）
    default: String,
    /// 支持安装的全部版本（单值写法时仅一个元素）
    all: Vec<String>,
}

impl<'de> serde::Deserialize<'de> for Versions {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = serde_yaml::Value::deserialize(d)?;
        Ok(versions_from_yaml(&v))
    }
}

/// YAML 标量统一转字符串（版本号、yes/no 等）。
fn yaml_scalar_to_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.trim().to_string()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn versions_from_yaml(v: &serde_yaml::Value) -> Versions {
    match v {
        serde_yaml::Value::Sequence(seq) => {
            let all: Vec<String> = seq.iter().filter_map(yaml_scalar_to_string).collect();
            Versions {
                default: all.first().cloned().unwrap_or_default(),
                all,
            }
        }
        other => {
            let default = yaml_scalar_to_string(other).unwrap_or_default();
            let all = if default.is_empty() {
                Vec::new()
            } else {
                vec![default.clone()]
            };
            Versions { default, all }
        }
    }
}

/// 兼容 yes/no/true/false/1/0 的布尔写法（app.yaml 中常写作 'yes'/'no'）。
fn de_boolish<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_yaml::Value::deserialize(d)?;
    Ok(match &v {
        serde_yaml::Value::Bool(b) => *b,
        serde_yaml::Value::String(s) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "yes" | "true" | "1" | "on" | "y"
        ),
        serde_yaml::Value::Number(n) => n.as_i64().map(|i| i != 0).unwrap_or(false),
        _ => false,
    })
}

/// 字符串或字符串数组统一解析为 Vec<String>（roles 白名单兼容两种写法）。
/// 非字符串/数组时返回 None。
fn de_str_list<'de, D>(d: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_yaml::Value::deserialize(d)?;
    Ok(match v {
        serde_yaml::Value::String(s) => Some(vec![s.trim().to_string()]),
        serde_yaml::Value::Sequence(seq) => {
            Some(seq.iter().filter_map(yaml_scalar_to_string).collect())
        }
        _ => None,
    })
}

/// 读取指定包声明的角色白名单（app.yaml roles 字段）。
/// 优先级与 scan_packages 一致：custom 覆盖同名 Git 源包。
/// 找不到包 / 未声明 roles → None（= 默认仅 admin 可操作，由调用方判定）。
pub async fn package_roles_of(pkg_path: &str) -> Option<Vec<String>> {
    let (cat, name) = pkg_path.split_once('/')?;
    if cat.is_empty() || name.is_empty() {
        return None;
    }
    let pkg_rel = format!("{cat}/{name}");
    let appstore = appstore_dir();
    let repos_root = appstore.join("repos");
    let custom_dir = appstore.join("custom");
    let repo_list = read_repos_value().await.unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        if let Some(repos) = repo_list.get("repos").and_then(|r| r.as_array()) {
            for repo in repos {
                if let Some(id) = repo.get("id").and_then(|v| v.as_str()) {
                    dirs.push(repos_root.join(id));
                }
            }
        }
        dirs.push(custom_dir);
        // 倒序命中：custom 优先级最高
        for dir in dirs.iter().rev() {
            let yaml_path = dir.join(&pkg_rel).join("app.yaml");
            if let Some(a) = parse_app_yaml(&yaml_path) {
                return Some(a.roles.unwrap_or_default());
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// 扫描全部 Git 源 + 自定义包。同名覆盖顺序（优先级从低到高）：
/// 内置源 < 后添加的源 < custom。
pub async fn scan_packages() -> Vec<Value> {
    let repos_root = appstore_dir().join("repos");
    let custom_dir = appstore_dir().join("custom");
    let repo_list = read_repos_value().await.unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let mut by_path: std::collections::BTreeMap<String, Value> =
            std::collections::BTreeMap::new();
        // 先扫各 Git 源（按 repos.yaml 顺序，内置源排最前 → 优先级最低）
        if let Some(repos) = repo_list.get("repos").and_then(|r| r.as_array()) {
            for repo in repos {
                let id = repo.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                let dir = repos_root.join(id);
                scan_source_dir(&dir, "official", Some(id), &mut by_path);
            }
        }
        // custom 最后扫描，覆盖同名官方包
        scan_source_dir(&custom_dir, "custom", None, &mut by_path);
        by_path.into_values().collect()
    })
    .await
    .unwrap_or_default()
}

fn scan_source_dir(
    dir: &Path,
    source: &str,
    repo_id: Option<&str>,
    by_path: &mut std::collections::BTreeMap<String, Value>,
) {
    // AppStore 分类：基础设施 / 应用程序 / Web 应用程序 / 数据层 / 基础库
    for category in ["infra", "application", "webapps", "database", "library"] {
        let cat_dir = dir.join(category);
        let Ok(entries) = std::fs::read_dir(&cat_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let pkg_dir = entry.path();
            if !pkg_dir.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let app_yaml = match parse_app_yaml(&pkg_dir.join("app.yaml")) {
                Some(a) => a,
                None => continue,
            };
            let pkg_path = format!("{category}/{name}");
            // dependencies：优先解析映射（name: version），兼容旧 deps 数组写法
            let dependencies = match app_yaml.dependencies.clone() {
                Some(d) if !d.is_null() => d,
                _ => match app_yaml.deps.clone() {
                    Some(list) => Value::Array(list.into_iter().map(Value::String).collect()),
                    None => json!({}),
                },
            };
            let actions = app_yaml.actions.clone().unwrap_or_else(|| json!({}));
            // 是否提供升级脚本：`scripts.upgrade` 可覆盖缺省文件名（与 zapexec 的 script_file 一致）。
            // 未提供时升级按「uninstall → install」兜底执行，前端据此提示风险。
            let has_upgrade = {
                let file = app_yaml
                    .scripts
                    .as_ref()
                    .and_then(|s| s.get("upgrade"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("upgrade.sh");
                pkg_dir.join(file).is_file()
            };
            by_path.insert(
                pkg_path.clone(),
                json!({
                    "pkg_path": pkg_path,
                    "has_upgrade": has_upgrade,
                    "category": app_yaml.category.clone().unwrap_or_else(|| category.to_string()),
                    "name": app_yaml.name.clone().unwrap_or(name),
                    "title": app_yaml.title.clone().unwrap_or_default(),
                    "description": app_yaml.description.clone().unwrap_or_default(),
                    "version": app_yaml.version.default.clone(),
                    "versions": app_yaml.version.all.clone(),
                    "version_meta": app_yaml.version_meta.clone().unwrap_or(Value::Null),
                    "deps": app_yaml.deps.clone().unwrap_or_default(),
                    "dependencies": dependencies,
                    "actions": actions,
                    "options": app_yaml.options.clone().unwrap_or(Value::Null),
                    "allow_multiple_instances": app_yaml.allow_multiple_instances,
                    "default_port": app_yaml.default_port,
                    "scripts": app_yaml.scripts.clone().unwrap_or(Value::Null),
                    "roles": app_yaml.roles.clone().unwrap_or_default(),
                    "source": source,
                    "repo_id": repo_id,
                }),
            );
        }
    }
}

/// 扫描已安装包（apps/<category>/<name>/meta.yaml）。
/// 目录结构与 zapexec 保持一致：apps_dir 的直接子目录为 category，
/// category 的子目录为具体包名。
pub async fn scan_installed() -> Vec<Value> {
    tokio::task::spawn_blocking(|| {
        let mut items = Vec::new();
        let Ok(cats) = std::fs::read_dir(apps_dir()) else {
            return items;
        };
        for cat in cats.flatten() {
            let cat_path = cat.path();
            if !cat_path.is_dir() {
                continue;
            }
            let Ok(pkgs) = std::fs::read_dir(&cat_path) else {
                continue;
            };
            for pkg in pkgs.flatten() {
                let app_path = pkg.path();
                if !app_path.is_dir() {
                    continue;
                }
                let Some(meta) = parse_meta_yaml(&app_path.join("meta.yaml")) else {
                    continue;
                };
                items.push(meta);
            }
        }
        items.sort_by(|a, b| {
            a.get("pkg_path")
                .and_then(|x| x.as_str())
                .cmp(&b.get("pkg_path").and_then(|x| x.as_str()))
        });
        items
    })
    .await
    .unwrap_or_default()
}

/// 读取某个已安装包的版本（升级时获取 old_version）。
pub async fn installed_version_of(pkg_path: &str) -> Option<String> {
    let apps = apps_dir();
    let pkg_path = pkg_path.to_string();
    tokio::task::spawn_blocking(move || {
        let p = apps.join(&pkg_path).join("meta.yaml");
        parse_meta_yaml(&p).and_then(|m| {
            m.get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
    })
    .await
    .unwrap_or(None)
}

/// 读取 repos.yaml 并返回 Value；不存在时兜底返回内置源（不落盘，写盘由 zapexec 负责）。
pub async fn read_repos_value() -> Option<Value> {
    let path = appstore_dir().join("repos.yaml");
    tokio::task::spawn_blocking(move || {
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(v) = serde_yaml::from_str::<Value>(&content)
        {
            return Some(v);
        }
        Some(json!({
            "repos": [{
                "id": "zap-appstore",
                "name": "Zap 官方应用商店",
                "url": "https://github.com/zapj/zap-appstore.git",
                "builtin": true,
                "enabled": true,
                "version": "",
                "commit": "",
                "updated_at": 0,
            }]
        }))
    })
    .await
    .unwrap_or(None)
}

/// 读取 Git 源列表，附加每个源目录是否存在的信息，供前端展示。
pub async fn list_repos() -> Vec<Value> {
    let repos_root = appstore_dir().join("repos");
    let value = read_repos_value().await;
    tokio::task::spawn_blocking(move || {
        let mut items = Vec::new();
        if let Some(v) = value
            && let Some(repos) = v.get("repos").and_then(|r| r.as_array())
        {
            for repo in repos {
                let id = repo.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                items.push(json!({
                    "id": id,
                    "name": repo.get("name").and_then(|v| v.as_str()).unwrap_or_default(),
                    "url": repo.get("url").and_then(|v| v.as_str()).unwrap_or_default(),
                    "builtin": repo.get("builtin").and_then(|v| v.as_bool()).unwrap_or(false),
                    "enabled": repo.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                    "version": repo.get("version").and_then(|v| v.as_str()).unwrap_or_default(),
                    "commit": repo.get("commit").and_then(|v| v.as_str()).unwrap_or_default(),
                    "updated_at": repo.get("updated_at").and_then(|v| v.as_i64()).unwrap_or(0),
                    "exists": repos_root.join(id).is_dir(),
                }));
            }
        }
        items
    })
    .await
    .unwrap_or_default()
}

fn parse_app_yaml(path: &Path) -> Option<AppYaml> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&content).ok()
}

fn parse_meta_yaml(path: &Path) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_yaml::from_str(&content).ok()?;
    // meta.yaml 位于 apps/{category}/{name}/meta.yaml，pkg_path 取其父目录的相对路径
    let pkg_path = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|cat_dir| cat_dir.file_name())
        .map(|cat| {
            let name = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            format!("{}/{}", cat.to_string_lossy(), name)
        })
        .unwrap_or_default();
    Some(json!({
        "pkg_path": pkg_path,
        "name": v.get("name"),
        "version": v.get("version"),
        "category": v.get("category"),
        "source": v.get("source"),
        "installed_at": v.get("installed_at"),
        "upgraded_from": v.get("upgraded_from"),
        "run_id": v.get("run_id"),
    }))
}
