//! AppStore 缓存 / 产物清理：安装日志、运行现场（编译产物）、下载缓存。
//!
//! 设计目标：
//! - 清理**已结束**任务的遗留（日志 / `runs/<id>/` 目录 / 下载缓存目录），释放磁盘；
//! - **绝不**清理正在运行的任务：`task_queue` 中 `status ∈ {pending, running}` 的任务
//!   其 `task_id` 既对应 `runs/<id>/` 目录、也对应 `logs/run-<id>.log`，删了会破坏在跑的任务。
//!
//! 覆盖的日志都遵循 `run-<task_id>.log` 命名，且 `task_id == task_queue.task_id`，
//! 因此同一份"运行中任务号集合"即可保护 appstore / 计划任务(cron) / Docker 构建 三类日志。
//!
//! 后续新增清理类型，只需在 [`TARGET_*`] 常量登记，并在 [`analyze_all`] / [`clean`] 中加分支。

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::zap::appstore;
use crate::zap::user_cron;
use crate::zap::ZapError;

/// 可清理目标标识（前端按 id 勾选；新增类型在此登记）。
pub const TARGET_APPSTORE_LOGS: &str = "appstore_logs";
pub const TARGET_APPSTORE_RUNS: &str = "appstore_runs";
pub const TARGET_APPSTORE_CACHE: &str = "appstore_cache";
/// 计划任务(cron)日志：`{data}/users/<用户名>/cron-logs/run-<task_id>.log`
pub const TARGET_USER_CRON_LOGS: &str = "user_cron_logs";
/// Docker 构建日志：`{data}/users/<用户名>/docker-build-logs/run-<task_id>.log`
pub const TARGET_USER_DOCKER_LOGS: &str = "user_docker_logs";

/// 单个目标的扫描结果（清理前预览用）。
#[derive(Debug, Serialize)]
pub struct TargetStat {
    pub id: &'static str,
    /// 可清理条目数（已排除正在运行的）。
    pub count: u64,
    /// 可清理字节数。
    pub size: u64,
    /// 因正在运行 / 排队而被跳过的条目数。
    pub protected: u64,
    /// 目标目录（仅管理员可见，便于排查）。
    pub dir: String,
}

/// 单次清理结果。
#[derive(Debug, Serialize)]
pub struct CleanResult {
    pub id: &'static str,
    pub removed: u64,
    pub freed: u64,
    pub skipped_running: u64,
}

/// 取当前"正在运行 / 排队"的任务号集合：这些 run_id 对应的日志与运行现场必须保留。
async fn protected_run_ids() -> HashSet<String> {
    let pool = crate::db::get_db_pool().await;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT task_id FROM task_queue WHERE status IN ('pending','running')",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    ids.into_iter().collect()
}

/// 递归计算路径占用字节数（不跟随符号链接，避免越界）。
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.is_file() {
            return meta.len();
        }
        if let Ok(entries) = std::fs::read_dir(path) {
            for e in entries.flatten() {
                total += dir_size(&e.path());
            }
        }
    }
    total
}

/// 列出目录下所有 `run-<id>.log` 对应的 id。
fn log_run_ids(dir: &Path) -> Vec<String> {
    let mut ids = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(id) = name.strip_prefix("run-")
                && let Some(id) = id.strip_suffix(".log")
            {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

// ── 日志 ────────────────────────────────────────────────

fn analyze_logs(dir: &Path, protected: &HashSet<String>) -> TargetStat {
    let mut count = 0u64;
    let mut size = 0u64;
    let mut protected_count = 0u64;
    for id in log_run_ids(dir) {
        if protected.contains(&id) {
            protected_count += 1;
            continue;
        }
        // 日志本体 + 同 id 的 .ret（退出码）/ .pid（进程号）
        for ext in ["", ".ret", ".pid"] {
            let p = dir.join(format!("run-{id}{ext}"));
            if p.exists() {
                size += dir_size(&p);
            }
        }
        count += 1;
    }
    TargetStat {
        id: TARGET_APPSTORE_LOGS,
        count,
        size,
        protected: protected_count,
        dir: dir.display().to_string(),
    }
}

fn clean_logs(dir: &Path, protected: &HashSet<String>) -> CleanResult {
    let mut removed = 0u64;
    let mut freed = 0u64;
    let mut skipped = 0u64;
    for id in log_run_ids(dir) {
        if protected.contains(&id) {
            skipped += 1;
            continue;
        }
        for ext in ["", ".ret", ".pid"] {
            let p = dir.join(format!("run-{id}{ext}"));
            if let Ok(meta) = std::fs::symlink_metadata(&p) {
                freed += meta.len();
                let _ = std::fs::remove_file(&p);
                removed += 1;
            }
        }
    }
    CleanResult {
        id: TARGET_APPSTORE_LOGS,
        removed,
        freed,
        skipped_running: skipped,
    }
}

// ── 多用户日志（cron / Docker 构建）────────────────────

/// 扫描某用户的 `<users_dir>/<用户名>/<sub>` 下所有 `run-<id>.log` 的聚合统计。
///
/// `sub` 为日志子目录名（`cron-logs` / `docker-build-logs`）。直接从 `users_dir()`
/// 遍历每个用户名子目录，无需逐个用户拼路径，便于一次汇总成单个清理目标。
fn analyze_user_logs(
    id: &'static str,
    base: &Path,
    sub: &str,
    protected: &HashSet<String>,
) -> TargetStat {
    let mut count = 0u64;
    let mut size = 0u64;
    let mut protected_count = 0u64;
    if let Ok(users) = std::fs::read_dir(base) {
        for u in users.flatten() {
            let dir = u.path().join(sub);
            if !dir.is_dir() {
                continue;
            }
            for run_id in log_run_ids(&dir) {
                if protected.contains(&run_id) {
                    protected_count += 1;
                    continue;
                }
                for ext in ["", ".ret", ".pid"] {
                    let p = dir.join(format!("run-{run_id}{ext}"));
                    if p.exists() {
                        size += dir_size(&p);
                    }
                }
                count += 1;
            }
        }
    }
    TargetStat {
        id,
        count,
        size,
        protected: protected_count,
        dir: base.display().to_string(),
    }
}

/// 清理某用户的 `<users_dir>/<用户名>/<sub>` 下的日志（跳过运行中任务）。
fn clean_user_logs(
    id: &'static str,
    base: &Path,
    sub: &str,
    protected: &HashSet<String>,
) -> CleanResult {
    let mut removed = 0u64;
    let mut freed = 0u64;
    let mut skipped = 0u64;
    if let Ok(users) = std::fs::read_dir(base) {
        for u in users.flatten() {
            let dir = u.path().join(sub);
            if !dir.is_dir() {
                continue;
            }
            for run_id in log_run_ids(&dir) {
                if protected.contains(&run_id) {
                    skipped += 1;
                    continue;
                }
                for ext in ["", ".ret", ".pid"] {
                    let p = dir.join(format!("run-{run_id}{ext}"));
                    if let Ok(meta) = std::fs::symlink_metadata(&p) {
                        freed += meta.len();
                        let _ = std::fs::remove_file(&p);
                        removed += 1;
                    }
                }
            }
        }
    }
    CleanResult {
        id,
        removed,
        freed,
        skipped_running: skipped,
    }
}

// ── 运行现场（编译产物） ────────────────────────────────

fn analyze_runs(dir: &Path, protected: &HashSet<String>) -> TargetStat {
    let mut count = 0u64;
    let mut size = 0u64;
    let mut protected_count = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let id = p.file_name().expect("dir entry has a name").to_string_lossy().to_string();
            if protected.contains(&id) {
                protected_count += 1;
                continue;
            }
            size += dir_size(&p);
            count += 1;
        }
    }
    TargetStat {
        id: TARGET_APPSTORE_RUNS,
        count,
        size,
        protected: protected_count,
        dir: dir.display().to_string(),
    }
}

fn clean_runs(dir: &Path, protected: &HashSet<String>) -> CleanResult {
    let mut removed = 0u64;
    let mut freed = 0u64;
    let mut skipped = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let id = p.file_name().expect("dir entry has a name").to_string_lossy().to_string();
            if protected.contains(&id) {
                skipped += 1;
                continue;
            }
            freed += dir_size(&p);
            let _ = std::fs::remove_dir_all(&p);
            removed += 1;
        }
    }
    CleanResult {
        id: TARGET_APPSTORE_RUNS,
        removed,
        freed,
        skipped_running: skipped,
    }
}

// ── 下载缓存 ────────────────────────────────────────────

fn analyze_cache(dir: &Path) -> TargetStat {
    let mut count = 0u64;
    let mut size = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            size += dir_size(&p);
            count += 1;
        }
    }
    TargetStat {
        id: TARGET_APPSTORE_CACHE,
        count,
        size,
        protected: 0,
        dir: dir.display().to_string(),
    }
}

fn clean_cache(dir: &Path) -> CleanResult {
    let mut removed = 0u64;
    let mut freed = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            freed += dir_size(&p);
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            } else {
                let _ = std::fs::remove_file(&p);
            }
            removed += 1;
        }
    }
    CleanResult {
        id: TARGET_APPSTORE_CACHE,
        removed,
        freed,
        skipped_running: 0,
    }
}

/// 扫描所有目标（清理前预览）。
pub async fn analyze_all() -> Vec<TargetStat> {
    let protected = protected_run_ids().await;
    let root = appstore::appstore_dir();
    let users = user_cron::users_dir();
    vec![
        analyze_logs(&appstore::logs_dir(), &protected),
        analyze_runs(&root.join("runs"), &protected),
        analyze_cache(&root.join("cache")),
        analyze_user_logs(TARGET_USER_CRON_LOGS, &users, "cron-logs", &protected),
        analyze_user_logs(
            TARGET_USER_DOCKER_LOGS,
            &users,
            "docker-build-logs",
            &protected,
        ),
    ]
}

/// 清理指定目标（id 列表；未知 id 忽略）。
pub async fn clean(targets: &[String]) -> Result<Vec<CleanResult>, ZapError> {
    let protected = protected_run_ids().await;
    let root = appstore::appstore_dir();
    let users = user_cron::users_dir();
    let mut out = Vec::new();
    for t in targets {
        match t.as_str() {
            TARGET_APPSTORE_LOGS => out.push(clean_logs(&appstore::logs_dir(), &protected)),
            TARGET_APPSTORE_RUNS => out.push(clean_runs(&root.join("runs"), &protected)),
            TARGET_APPSTORE_CACHE => out.push(clean_cache(&root.join("cache"))),
            TARGET_USER_CRON_LOGS => out.push(clean_user_logs(
                TARGET_USER_CRON_LOGS,
                &users,
                "cron-logs",
                &protected,
            )),
            TARGET_USER_DOCKER_LOGS => out.push(clean_user_logs(
                TARGET_USER_DOCKER_LOGS,
                &users,
                "docker-build-logs",
                &protected,
            )),
            _ => {}
        }
    }
    Ok(out)
}
