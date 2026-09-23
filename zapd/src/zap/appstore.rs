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

/// 自定义脚本任务的大类标记：脚本只是复用 AppStore 的日志目录与执行通道，
/// 本身不是「应用商店」，在任务队列里要能一眼看出是脚本。
pub const KIND_SCRIPT: &str = crate::zap::task::KIND_SCRIPT;

/// 并发互斥组：源码编译型任务（安装 / 升级）**全局同一时刻只允许一个**。
///
/// 编译吃满 CPU 与内存，并行只会互相拖慢、还让日志交叉难以排查；
/// 但直接拒绝第二个请求体验太差（用户只能反复重试），所以登记为 `pending`
/// 排队，由 `task` 的调度器在前者结束后自动放行。
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

/// 面板数据根目录（apps/ 与 users/ 的父目录；对应 zapexec 的 `{ZAP_PATH}/data`）。
pub fn data_dir() -> PathBuf {
    let cfg = config::get_config().read().unwrap();
    let db_path = Path::new(&cfg.db.path);
    db_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("data"))
}

/// 已安装软件目录（apps/）：规则统一在 `zap_proto::appstore`，两边不再各算一套
pub fn apps_dir() -> PathBuf {
    zap_proto::appstore::apps_dir(&data_dir())
}

/// 一个已安装实例的定位信息。
///
/// 同一个包可以装多份（多版本 PHP、多站点 WordPress），因此定位实例不能只靠
/// `pkg_path`，还要有 `instance`（站点类为 `site:<id>`，并带上归属用户）。
#[derive(Debug, Clone)]
pub struct InstalledSlot {
    pub category: String,
    pub name: String,
    pub instance: String,
    /// 归属面板用户（站点类有）
    pub owner: Option<String>,
    /// 站点 id（站点类有）
    pub site_id: Option<String>,
    /// 槽位目录（meta.yaml / info.yaml / provision.json 都在这里）
    pub dir: PathBuf,
    /// 脚本在 info.yaml 里登记的实例名（可能与槽位目录名不同，如 php74 / default）
    pub registered: Option<String>,
}

/// 读槽位里脚本登记的实例名（info.yaml: instance）。
fn registered_instance_in(dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(dir.join("info.yaml")).ok()?;
    let v: Value = serde_yaml::from_str(&content).ok()?;
    v.get("instance")
        .and_then(|x| x.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

impl InstalledSlot {
    /// 包路径：`<category>/<name>`（定位包源用，多实例时不唯一）
    pub fn pkg_path(&self) -> String {
        format!("{}/{}", self.category, self.name)
    }

    /// 稳定主键：`<category>/<name>@<instance>`
    pub fn key(&self) -> String {
        format!("{}/{}@{}", self.category, self.name, self.instance)
    }
}

/// 扫描全部已安装实例（与 zapexec 的 `scan_slots` 对称）。
///
/// - `apps/<category>/<name>/<instance>/`：全局类；旧布局没有 instance 层
///   （meta.yaml 直接在包目录下），仍按 `default` 实例识别，老安装不会消失；
/// - `users/<owner>/webapps/<name>/<site_id>/`：站点类，登记信息跟随站点账号。
pub fn scan_slots() -> Vec<InstalledSlot> {
    use zap_proto::appstore::{DEFAULT_INSTANCE, Slot, WEBAPPS_CATEGORY};

    let mut out = Vec::new();

    // 1) 全局类
    if let Ok(cats) = std::fs::read_dir(apps_dir()) {
        for cat in cats.flatten() {
            let cat_path = cat.path();
            if !cat_path.is_dir() {
                continue;
            }
            let category = cat.file_name().to_string_lossy().to_string();
            let Ok(pkgs) = std::fs::read_dir(&cat_path) else {
                continue;
            };
            for pkg in pkgs.flatten() {
                let pkg_dir = pkg.path();
                if !pkg_dir.is_dir() {
                    continue;
                }
                let name = pkg.file_name().to_string_lossy().to_string();
                if pkg_dir.join("meta.yaml").is_file() {
                    out.push(InstalledSlot {
                        category: category.clone(),
                        name: name.clone(),
                        instance: DEFAULT_INSTANCE.to_string(),
                        owner: None,
                        site_id: None,
                        dir: pkg_dir.clone(),
                        registered: registered_instance_in(&pkg_dir),
                    });
                }
                let Ok(insts) = std::fs::read_dir(&pkg_dir) else {
                    continue;
                };
                for inst in insts.flatten() {
                    let dir = inst.path();
                    if !dir.is_dir() || !dir.join("meta.yaml").is_file() {
                        continue;
                    }
                    out.push(InstalledSlot {
                        category: category.clone(),
                        name: name.clone(),
                        instance: inst.file_name().to_string_lossy().to_string(),
                        owner: None,
                        site_id: None,
                        registered: registered_instance_in(&dir),
                        dir,
                    });
                }
            }
        }
    }

    // 2) 站点类：每个用户的私有 webapps 目录
    if let Ok(owners) = std::fs::read_dir(data_dir().join("users")) {
        for owner in owners.flatten() {
            let owner_dir = owner.path();
            if !owner_dir.is_dir() {
                continue;
            }
            let owner_name = owner.file_name().to_string_lossy().to_string();
            let Ok(pkgs) = std::fs::read_dir(owner_dir.join(WEBAPPS_CATEGORY)) else {
                continue;
            };
            for pkg in pkgs.flatten() {
                let pkg_dir = pkg.path();
                if !pkg_dir.is_dir() {
                    continue;
                }
                let name = pkg.file_name().to_string_lossy().to_string();
                let Ok(sites) = std::fs::read_dir(&pkg_dir) else {
                    continue;
                };
                for site in sites.flatten() {
                    let dir = site.path();
                    if !dir.is_dir() || !dir.join("meta.yaml").is_file() {
                        continue;
                    }
                    let site_id = site.file_name().to_string_lossy().to_string();
                    out.push(InstalledSlot {
                        category: WEBAPPS_CATEGORY.to_string(),
                        name: name.clone(),
                        instance: Slot::site_instance(&site_id),
                        owner: Some(owner_name.clone()),
                        site_id: Some(site_id),
                        registered: registered_instance_in(&dir),
                        dir,
                    });
                }
            }
        }
    }

    out
}

/// 按 `pkg_path` + 实例名找已安装槽位（instance 为 None 时取该包的第一个）。
pub fn find_slot(pkg_path: &str, instance: Option<&str>) -> Option<InstalledSlot> {
    let mut slots = scan_slots()
        .into_iter()
        .filter(|s| s.pkg_path() == pkg_path)
        .collect::<Vec<_>>();
    if let Some(inst) = instance.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(hit) = slots
            .iter()
            .find(|s| s.instance == inst || s.registered.as_deref() == Some(inst))
        {
            return Some(hit.clone());
        }
        // 该包只有一个槽位：没有歧义（脚本没在 info.yaml 登记实例名也能对上）
        if slots.len() == 1 {
            return slots.into_iter().next();
        }
        // 多实例又对不上：宁可不补齐，也不要把归属信息张冠李戴
        return None;
    }
    slots.sort_by(|a, b| a.instance.cmp(&b.instance));
    slots.into_iter().next()
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
    register_run_kind(run_id, KIND, action, pkg, username, log_path, job_key).await
}

/// 登记一条**自定义脚本**的运行记录（kind = `script`）。
///
/// 与 [`register_run_with_key`] 的唯一区别就是大类：脚本走 AppStore 的执行通道，
/// 但任务队列里要显示成「自定义脚本」而不是「应用商店」。
pub async fn register_script_run_with_key(
    run_id: &str,
    action: &str,
    pkg: &str,
    username: &str,
    log_path: &str,
    job_key: &str,
) -> Result<(), ZapError> {
    register_run_kind(
        run_id,
        KIND_SCRIPT,
        action,
        pkg,
        username,
        log_path,
        job_key,
    )
    .await
}

/// 登记一条与定时任务无关的自定义脚本运行记录（`job_key` 为空）。
pub async fn register_script_run(
    run_id: &str,
    action: &str,
    pkg: &str,
    username: &str,
    log_path: &str,
) -> Result<(), ZapError> {
    register_script_run_with_key(run_id, action, pkg, username, log_path, "").await
}

async fn register_run_kind(
    run_id: &str,
    kind: &str,
    action: &str,
    pkg: &str,
    username: &str,
    log_path: &str,
    job_key: &str,
) -> Result<(), ZapError> {
    crate::zap::task::enqueue(crate::zap::task::NewTask {
        task_id: run_id.to_string(),
        kind: kind.to_string(),
        action: action.to_string(),
        pkg: pkg.to_string(),
        username: username.to_string(),
        title: String::new(),
        log_path: log_path.to_string(),
        job_key: job_key.to_string(),
        // 脚本 / 仓库同步等任务不做全局互斥（编译类走 enqueue_compile 单独入口）
        group_key: String::new(),
        group_limit: 0,
        payload: String::new(),
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

/// 登记一条**编译型**任务（安装 / 升级）。
///
/// 返回里带着准入结论：
/// - `status = running`：拿到槽位了，调用方立刻 [`crate::zap::task::launch`]；
/// - `status = pending`：前面还有编译在跑，这条已入队，由调度器在前一个结束后
///   自动放行（启动参数 `payload` 已随记录落库）。
pub async fn enqueue_compile(
    run_id: &str,
    action: &str,
    pkg: &str,
    username: &str,
    log_path: &str,
    payload: &str,
) -> Result<crate::zap::task::Task, ZapError> {
    crate::zap::task::enqueue(crate::zap::task::NewTask {
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
        payload: payload.to_string(),
    })
    .await
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
            token_version: 0,
            scope: String::new(),
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

    /// 仓库自带的 WordPress 样板包（`webapps/wordpress`）是「建站 + 建库编排」的
    /// 参考实现：字段写错会让 provision 静默失效（脚本拿不到站点与库），
    /// 因此在这里钉住它的关键声明。
    #[test]
    fn sample_wordpress_package_declares_provision() {
        let repo =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/appstore/repos/zap-appstore");
        let app = parse_app_yaml(&repo.join("webapps/wordpress/app.yaml"))
            .expect("样板包 app.yaml 解析失败");
        assert_eq!(app.category.as_deref(), Some("webapps"));
        assert_eq!(app.run_as.as_deref(), Some("user"));
        assert_eq!(app.scope.as_deref(), Some("site"));

        let provision = app.provision.expect("缺少 provision 声明");
        let site = provision.site.expect("缺少 provision.site");
        assert_eq!(site.domain_option.as_deref(), Some("SITE_DOMAIN"));
        assert_eq!(site.mode.as_deref(), Some("create"));
        let db = provision.database.expect("缺少 provision.database");
        assert_eq!(db.name.as_deref(), Some("wp"));
        assert_eq!(db.charset.as_deref(), Some("utf8mb4"));

        // 脚本用 python 编写：解释器按扩展名推导为 python3
        let scripts = app.scripts.expect("缺少 scripts").to_string();
        assert!(scripts.contains("install.py"), "实际: {scripts}");
        assert!(scripts.contains("uninstall.py"), "实际: {scripts}");
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
    /// 脚本运行身份：`user` = 以面板用户对应的 Linux 账号（nologin）执行，
    /// `root` = 沿用特权执行（默认）。仅 `webapps` 分类可声明 `user`。
    run_as: Option<String>,
    /// 作用范围：`site` = 装进用户站点目录（建站类，缺省降权为 user），
    /// `panel` = 面板级工具（如 phpMyAdmin，系统级安装）。
    scope: Option<String>,
    /// 面板侧编排声明（建站 / 建库）。降权脚本没有建站建库的权限，
    /// 由面板先备好资源再把连接信息注入脚本 env。
    provision: Option<ProvisionSpec>,
}

/// 面板侧编排声明：建站类包（`webapps`）安装前由面板准备好的资源。
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ProvisionSpec {
    /// 站点：建站类包需要落地的站点（按域名复用或自动创建）
    pub site: Option<SiteProvision>,
    /// 数据库：为该实例建的专用库 + 专用用户
    pub database: Option<DbProvision>,
}

/// 站点编排：`mode` = `create`（缺省，不存在就建）/ `require`（必须已存在）。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SiteProvision {
    /// 从安装选项里读取域名的选项名（缺省 `SITE_DOMAIN`）
    pub domain_option: Option<String>,
    /// create（缺省）/ require
    pub mode: Option<String>,
    /// PHP 实例标识（缺省取面板默认 PHP）
    pub php: Option<String>,
    /// 伪静态预设（none / wordpress / thinkphp / laravel / codeigniter）
    pub rewrite: Option<String>,
}

/// 数据库编排：库名基名（最终库名 = 用户前缀 + base，重名加序号）。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DbProvision {
    pub name: Option<String>,
    pub charset: Option<String>,
    /// 是否创建专用用户并授权（缺省 true）
    pub user: Option<bool>,
    /// 允许连接的主机（缺省 localhost）
    pub host: Option<String>,
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

/// 读取指定包声明的脚本运行身份（app.yaml `run_as` / `scope`）。
///
/// 返回 `Some("user")`：该包以面板用户对应的 Linux 账号（nologin）运行；
/// `Some("root")` / `None`（包不存在或未声明）= 特权运行。
/// 优先级与 scan_packages 一致：custom 覆盖同名 Git 源包。
pub async fn package_run_as_of(pkg_path: &str) -> Option<String> {
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
                let mode = a.run_as.map(|s| s.trim().to_ascii_lowercase()).or_else(|| {
                    a.scope
                        .as_deref()
                        .map(|s| s.trim().to_ascii_lowercase())
                        .filter(|s| s == "site")
                        .map(|_| "user".to_string())
                });
                return Some(mode.unwrap_or_else(|| "root".to_string()));
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// 读取指定包的面板编排声明（app.yaml `provision`）。
///
/// 命中规则与 `package_run_as_of` 一致（custom 覆盖同名 Git 源包）；
/// 未声明 / 包不存在 → None（脚本自己解决资源，通常是系统级包）。
pub async fn package_provision_of(pkg_path: &str) -> Option<ProvisionSpec> {
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
            if let Some(a) = parse_app_yaml(&yaml_path)
                && let Some(p) = a.provision
            {
                return Some(p);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// 读取某个包是否声明了「允许多实例安装」（app.yaml: allow_multiple_instances）。
///
/// 多实例包（多版本 PHP）在装第二个版本时要落进另一个槽位，否则新登记会覆盖旧的。
pub async fn package_multi_instance_of(pkg_path: &str) -> Option<bool> {
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
                return Some(a.allow_multiple_instances);
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
                    "run_as": app_yaml.run_as.clone().unwrap_or_default(),
                    "scope": app_yaml.scope.clone().unwrap_or_default(),
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
        for slot in scan_slots() {
            let Some(meta) = parse_slot_meta(&slot) else {
                continue;
            };
            items.push(meta);
        }
        items.sort_by(|a, b| {
            a.get("pkg_path")
                .and_then(|x| x.as_str())
                .cmp(&b.get("pkg_path").and_then(|x| x.as_str()))
                .then_with(|| {
                    a.get("instance")
                        .and_then(|x| x.as_str())
                        .cmp(&b.get("instance").and_then(|x| x.as_str()))
                })
        });
        items
    })
    .await
    .unwrap_or_default()
}

/// 读取某个实例已安装的版本（升级时获取 old_version）。
///
/// 必须按槽位定位：多版本 PHP 各自有 meta.yaml，只认 `pkg_path` 会读到错的那个
/// （甚至读到已经被覆盖掉的旧记录）。instance 为 None 时取该包的第一个实例。
pub async fn installed_version_of(pkg_path: &str) -> Option<String> {
    installed_version_for(pkg_path, None).await
}

/// 按 `pkg_path` + 实例名读取已安装版本。
pub async fn installed_version_for(pkg_path: &str, instance: Option<&str>) -> Option<String> {
    let slot = find_slot(pkg_path, instance)?;
    parse_slot_meta(&slot)?
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
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
                "url": "https://github.com/zapsh/zap-appstore.git",
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

/// 解析槽位里的 meta.yaml，并补上实例定位信息（前端据此区分同包的多个安装）。
fn parse_slot_meta(slot: &InstalledSlot) -> Option<Value> {
    let content = std::fs::read_to_string(slot.dir.join("meta.yaml")).ok()?;
    let v: Value = serde_yaml::from_str(&content).ok()?;
    Some(json!({
        "pkg_path": slot.pkg_path(),
        "instance_key": slot.key(),
        "instance": slot.instance,
        "owner": slot.owner,
        "site_id": slot.site_id,
        "name": v.get("name"),
        "version": v.get("version"),
        "category": v.get("category"),
        "source": v.get("source"),
        "installed_at": v.get("installed_at"),
        "upgraded_from": v.get("upgraded_from"),
        "run_id": v.get("run_id"),
    }))
}
