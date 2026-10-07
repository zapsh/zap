// SPDX-License-Identifier: AGPL-3.0-only
//! 缓存 / 产物清理：安装日志、运行现场（编译产物）、下载缓存，以及 `data/tmp` 下的临时上传暂存。
//!
//! 设计目标：
//! - 清理**已结束**任务的遗留（日志 / `runs/<id>/` 目录 / 下载缓存目录），释放磁盘；
//! - **绝不**清理正在运行的任务：`task_queue` 中 `status ∈ {pending, running}` 的任务
//!   其 `task_id` 既对应 `runs/<id>/` 目录、也对应 `logs/run-<id>.log`，删了会破坏在跑的任务。
//!
//! 覆盖的日志都遵循 `run-<task_id>.log` 命名，且 `task_id == task_queue.task_id`，
//! 因此同一份"运行中任务号集合"即可保护 appstore / 计划任务(cron) / Docker 构建 三类日志。
//!
//! **删除必须走 root 特权的 zapexec**：面板进程(zapd)以 `zapadm` 运行，删不掉 zapexec(root)
//! 创建的编译产物 / 日志目录；若就地用 `std::fs::remove_*` 会静默失败，导致"提示成功却删不掉"。
//! 所以所有删除统一经 [`remove_via_exec`] 转发到执行端（root）完成。
//!
//! 后续新增清理类型，只需在 [`TARGET_*`] 常量登记，并在 [`analyze_all`] / [`clean`] 中加分支。

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use tracing::warn;
use zap_proto::Request;

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
/// 应用商店上传暂存（`data/tmp/upload`）：纯临时文件（按 `用户id-时间戳` 命名），
/// 不对应任何运行任务，可直接整体清空。
pub const TARGET_APPSTORE_UPLOAD_TMP: &str = "appstore_upload_tmp";
/// 插件上传暂存（`data/tmp/plugin_uploads`）：同上，纯临时文件，可直接清理。
pub const TARGET_PLUGIN_UPLOAD_TMP: &str = "plugin_upload_tmp";

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
    /// 实际成功删除的条目数（仅在删除真正成功时计数，避免"假成功"）。
    pub removed: u64,
    /// 实际释放的字节数。
    pub freed: u64,
    /// 因正在运行 / 排队而被跳过的条目数。
    pub skipped_running: u64,
    /// 因权限 / 执行端不可用等原因删除失败的条目数。
    pub failed: u64,
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

/// 通过 root 特权的 zapexec 删除一个文件 / 目录。
///
/// 面板进程以 `zapadm` 运行，删不掉 zapexec(root) 创建的编译产物 / 日志目录；
/// 因此一切删除都转发到执行端。`skip_owner_check + as_user=None` 让执行端以 root 身份、
/// 不受属主校验地删除（路径经 `resolve_path` 剔除 `..`，且 `file.delete` 禁止删除
/// 系统关键目录，安全性由执行端兜底）。成功返回 `true`，失败仅记日志并返 `false`，
/// 调用方据此把条目计入 `failed` —— 绝不谎报成功。
async fn remove_via_exec(path: &Path) -> bool {
    let p = path.to_string_lossy().to_string();
    match crate::zapexec::call(Request::FileDelete {
        path: p,
        as_user: None,
        skip_owner_check: true,
    })
    .await
    {
        Ok(r) if r.code == 0 => true,
        Ok(r) => {
            warn!("缓存清理删除失败 {}: {}", path.display(), r.message);
            false
        }
        Err(e) => {
            warn!("缓存清理删除失败 {}: {}", path.display(), e);
            false
        }
    }
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

async fn clean_logs(dir: &Path, protected: &HashSet<String>) -> CleanResult {
    let mut removed = 0u64;
    let mut freed = 0u64;
    let mut skipped = 0u64;
    let mut failed = 0u64;
    for id in log_run_ids(dir) {
        if protected.contains(&id) {
            skipped += 1;
            continue;
        }
        // 预估该条目的总体积（主日志 + .ret/.pid 边车）
        let mut size = 0u64;
        for ext in ["", ".ret", ".pid"] {
            let p = dir.join(format!("run-{id}{ext}"));
            if p.exists() {
                size += dir_size(&p);
            }
        }
        // 主日志删除成功才算"清理掉该条目"；边车尽力删除即可
        let main = dir.join(format!("run-{id}.log"));
        if remove_via_exec(&main).await {
            removed += 1;
            freed += size;
            for ext in [".ret", ".pid"] {
                let _ = remove_via_exec(&dir.join(format!("run-{id}{ext}"))).await;
            }
        } else {
            failed += 1;
        }
    }
    CleanResult {
        id: TARGET_APPSTORE_LOGS,
        removed,
        freed,
        skipped_running: skipped,
        failed,
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
async fn clean_user_logs(
    id: &'static str,
    base: &Path,
    sub: &str,
    protected: &HashSet<String>,
) -> CleanResult {
    let mut removed = 0u64;
    let mut freed = 0u64;
    let mut skipped = 0u64;
    let mut failed = 0u64;
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
                let mut size = 0u64;
                for ext in ["", ".ret", ".pid"] {
                    let p = dir.join(format!("run-{run_id}{ext}"));
                    if p.exists() {
                        size += dir_size(&p);
                    }
                }
                let main = dir.join(format!("run-{run_id}.log"));
                if remove_via_exec(&main).await {
                    removed += 1;
                    freed += size;
                    for ext in [".ret", ".pid"] {
                        let _ = remove_via_exec(&dir.join(format!("run-{run_id}{ext}"))).await;
                    }
                } else {
                    failed += 1;
                }
            }
        }
    }
    CleanResult {
        id,
        removed,
        freed,
        skipped_running: skipped,
        failed,
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

async fn clean_runs(dir: &Path, protected: &HashSet<String>) -> CleanResult {
    let mut removed = 0u64;
    let mut freed = 0u64;
    let mut skipped = 0u64;
    let mut failed = 0u64;
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
            let size = dir_size(&p);
            if remove_via_exec(&p).await {
                removed += 1;
                freed += size;
            } else {
                failed += 1;
            }
        }
    }
    CleanResult {
        id: TARGET_APPSTORE_RUNS,
        removed,
        freed,
        skipped_running: skipped,
        failed,
    }
}

// ── 下载缓存 ────────────────────────────────────────────

/// 通用「整目录清空」型目标的扫描：统计目录下全部条目。
///
/// 用于**纯临时目录**（如下载缓存、上传暂存）——不对应任何运行任务，
/// 因此无需运行中保护，可直接整体清理。
fn analyze_whole_dir(id: &'static str, dir: &Path) -> TargetStat {
    let mut count = 0u64;
    let mut size = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            size += dir_size(&e.path());
            count += 1;
        }
    }
    TargetStat {
        id,
        count,
        size,
        protected: 0,
        dir: dir.display().to_string(),
    }
}

/// 通用「整目录清空」型目标的清理：删除目录下每个条目（无需运行中保护）。
async fn clean_whole_dir(id: &'static str, dir: &Path) -> CleanResult {
    let mut removed = 0u64;
    let mut freed = 0u64;
    let mut failed = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let size = dir_size(&p);
            if remove_via_exec(&p).await {
                removed += 1;
                freed += size;
            } else {
                failed += 1;
            }
        }
    }
    CleanResult {
        id,
        removed,
        freed,
        skipped_running: 0,
        failed,
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
        analyze_whole_dir(TARGET_APPSTORE_CACHE, &root.join("cache")),
        analyze_user_logs(TARGET_USER_CRON_LOGS, &users, "cron-logs", &protected),
        analyze_user_logs(
            TARGET_USER_DOCKER_LOGS,
            &users,
            "docker-build-logs",
            &protected,
        ),
        analyze_whole_dir(
            TARGET_APPSTORE_UPLOAD_TMP,
            &appstore::data_dir().join("tmp").join("upload"),
        ),
        analyze_whole_dir(
            TARGET_PLUGIN_UPLOAD_TMP,
            &appstore::data_dir().join("tmp").join("plugin_uploads"),
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
            TARGET_APPSTORE_LOGS => {
                out.push(clean_logs(&appstore::logs_dir(), &protected).await)
            }
            TARGET_APPSTORE_RUNS => {
                out.push(clean_runs(&root.join("runs"), &protected).await)
            }
            TARGET_APPSTORE_CACHE => {
                out.push(clean_whole_dir(TARGET_APPSTORE_CACHE, &root.join("cache")).await)
            }
            TARGET_APPSTORE_UPLOAD_TMP => out.push(
                clean_whole_dir(
                    TARGET_APPSTORE_UPLOAD_TMP,
                    &appstore::data_dir().join("tmp").join("upload"),
                )
                .await,
            ),
            TARGET_PLUGIN_UPLOAD_TMP => out.push(
                clean_whole_dir(
                    TARGET_PLUGIN_UPLOAD_TMP,
                    &appstore::data_dir().join("tmp").join("plugin_uploads"),
                )
                .await,
            ),
            TARGET_USER_CRON_LOGS => {
                out.push(
                    clean_user_logs(TARGET_USER_CRON_LOGS, &users, "cron-logs", &protected).await,
                )
            }
            TARGET_USER_DOCKER_LOGS => {
                out.push(
                    clean_user_logs(
                        TARGET_USER_DOCKER_LOGS,
                        &users,
                        "docker-build-logs",
                        &protected,
                    )
                    .await,
                )
            }
            _ => {}
        }
    }
    Ok(out)
}
