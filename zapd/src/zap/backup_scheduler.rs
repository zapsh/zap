//! 备份任务调度器：周期性扫描启用的备份任务，命中 cron 即触发一次备份。
//!
//! 复用脚本/自动化同款的 5 段 cron 解析（`script_cron::Cron`），避免重复实现。
//! 每个任务在同一分钟内最多触发一次（`last_run_at` 去重），天然防重。

use std::time::Duration;

use tracing::{info, warn};

use crate::db::get_db_pool;
use crate::routers::system_backup::run_backup;
use crate::zap::script_cron::Cron;

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
    let rows: Vec<(i64, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT id, name, target_type, target, schedule, last_run_at \
         FROM backup_jobs WHERE enabled = 1",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let now = chrono::Local::now();
    let ts = now.timestamp();

    for (id, name, target_type, target, schedule, last_run_at) in rows {
        let cron = match Cron::parse(&schedule) {
            Ok(c) => c,
            Err(e) => {
                warn!("备份任务 {id} 的 cron 表达式非法: {e}");
                continue;
            }
        };
        // 命中本分钟，且与上次运行间隔超过 60s（防同一分钟内重复触发）
        if cron.matches(&now) && (ts - last_run_at) > 60 {
            tokio::spawn(run_job(id, name, target_type, target, ts));
        }
    }
}

async fn run_job(id: i64, name: String, target_type: String, target: String, ts: i64) {
    let result = run_backup(&target_type, &target, &name, "", Some(id)).await;
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
