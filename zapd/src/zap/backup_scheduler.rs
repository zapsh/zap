// SPDX-License-Identifier: AGPL-3.0-only
//! 备份任务调度器：周期性扫描启用的备份任务，命中 cron 即触发一次备份。
//!
//! 复用脚本/自动化同款的 5 段 cron 解析（`script_cron::Cron`），避免重复实现。
//! 每个任务在同一分钟内最多触发一次（`last_run_at` 去重），天然防重。

use std::time::Duration;

use tracing::{info, warn};

use crate::db::get_db_pool;
use crate::routers::system_backup::{RunBackupArgs, run_backup};
use crate::zap::script_cron::Cron;

/// 读全局策略 KV（与 `system_backup::gs_get` 同源，调度器独立实现避免跨模块依赖）。
async fn gs_get(key: &str) -> String {
    let pool = get_db_pool().await;
    let r: Option<(String,)> = sqlx::query_as("SELECT value FROM global_settings WHERE key=?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    r.map(|x| x.0).unwrap_or_default()
}

async fn gs_set(key: &str, v: &str) {
    let pool = get_db_pool().await;
    let _ = sqlx::query(
        "INSERT INTO global_settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(key)
    .bind(v)
    .execute(pool)
    .await;
}

/// 启动调度循环（后台任务，不阻塞主流程）。
pub fn start() {
    tokio::spawn(async {
        // 等服务起来后再开始扫，避免与建库初始化抢跑
        tokio::time::sleep(Duration::from_secs(5)).await;
        loop {
            tick().await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

async fn tick() {
    let pool = get_db_pool().await;
    let rows: Vec<(i64, String, String, String, String, i64, String)> = sqlx::query_as(
        "SELECT id, name, target_type, target, schedule, last_run_at, owner \
         FROM backup_jobs WHERE enabled = 1",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let now = chrono::Local::now();
    let ts = now.timestamp();

    for (id, name, target_type, target, schedule, last_run_at, owner) in rows {
        let cron = match Cron::parse(&schedule) {
            Ok(c) => c,
            Err(e) => {
                warn!("备份任务 {id} 的 cron 表达式非法: {e}");
                continue;
            }
        };
        // 命中本分钟，且与上次运行间隔超过 60s（防同一分钟内重复触发）
        if cron.matches(&now) && (ts - last_run_at) > 60 {
            tokio::spawn(run_job(id, name, target_type, target, owner, ts));
        }
    }

    // 全量备份：管理员在「备份策略」里开启并配置 cron 后，按计划自动跑一次。
    // 与任务共用 30s 扫描节拍 + 同分钟去重（last_run 落 global_settings）。
    let all_enabled = gs_get("backup_all_enabled").await != "0";
    let all_schedule = gs_get("backup_all_schedule").await;
    if all_enabled
        && !all_schedule.trim().is_empty()
        && let Ok(cron) = Cron::parse(&all_schedule)
        && cron.matches(&now)
    {
        let last = gs_get("backup_all_last_run")
            .await
            .parse::<i64>()
            .unwrap_or(0);
        if (ts - last) > 60 {
            gs_set("backup_all_last_run", &ts.to_string()).await;
            info!("触发全量备份（计划 {all_schedule}）");
            tokio::spawn(crate::routers::system_backup::run_backup_all());
        }
    }
}

async fn run_job(
    id: i64,
    name: String,
    target_type: String,
    target: String,
    owner: String,
    ts: i64,
) {
    // 按任务归属解析归档落盘根：管理员任务（owner 空）= 系统备份根；用户任务 = 其家目录 backups。
    let root = match crate::routers::system_backup::resolve_job_root(&owner).await {
        Ok(r) => r.0,
        Err(e) => {
            warn!("备份任务 {id}({name}) 解析归属根失败: {e}");
            let pool = get_db_pool().await;
            let _ = sqlx::query(
                "UPDATE backup_jobs SET last_run_at=?, last_status=-1, last_message=? WHERE id=?",
            )
            .bind(ts)
            .bind(e.to_string())
            .bind(id)
            .execute(pool)
            .await;
            return;
        }
    };
    let result = run_backup(RunBackupArgs {
        target_type: &target_type,
        target_json: &target,
        name: &name,
        dest_dir: "",
        job_id: Some(id),
        owner: &owner,
        dest_root: Some(&root),
        excludes: &[],
        exclude_file: None,
        manifest: &[],
    })
    .await;
    let pool = get_db_pool().await;
    match result {
        Ok(_) => {
            let _ = sqlx::query(
                "UPDATE backup_jobs SET last_run_at=?, last_status=1, last_message='ok' WHERE id=?",
            )
            .bind(ts)
            .bind(id)
            .execute(pool)
            .await;
            info!("备份任务 {id}({name}) 执行成功");
        }
        Err(e) => {
            let msg = e.to_string();
            warn!("备份任务 {id}({name}) 执行失败: {msg}");
            let _ = sqlx::query(
                "UPDATE backup_jobs SET last_run_at=?, last_status=-1, last_message=? WHERE id=?",
            )
            .bind(ts)
            .bind(&msg)
            .bind(id)
            .execute(pool)
            .await;
        }
    }
}
