use std::collections::HashMap;

use once_cell::sync::Lazy;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};
use tokio::sync::RwLock;
use tokio_cron_scheduler::{Job, JobScheduler, job::job_data::Uuid};
use tracing::{debug, info, warn};

use crate::db::get_db_pool;

static GLOBAL_SCHEDULED_MAP: Lazy<RwLock<HashMap<String, JobScheduler>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static GLOBAL_JOB_MAP: Lazy<RwLock<HashMap<String, Uuid>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

static GLOBAL_SYSTEM_INFO: Lazy<RwLock<HashMap<String, String>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

async fn system_scheduled_task() {
    // debug!("Scheduled task executed at: {:?}", chrono::Utc::now());
    let pool = get_db_pool().await;
    let per_10s = 10; // seconds
    // load avg
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );
    let load_avg = sysinfo::System::load_average();
    sys.refresh_cpu_usage();
    let cpu_usage = sys.global_cpu_usage();
    let memory_usage =
        (sys.total_memory() - sys.available_memory()) as f32 / sys.total_memory() as f32 * 100.0;
    let swap_usage = if sys.total_swap() == 0 {
        0.00_f32
    } else {
        (sys.total_swap() - sys.free_swap()) as f32 / sys.total_swap() as f32 * 100.0
    };

    // network traffic
    let mut networks = Networks::new_with_refreshed_list();
    networks.refresh(true);
    for (interface_name, data) in networks.iter() {
        let last_received = GLOBAL_SYSTEM_INFO
            .read()
            .await
            .get(&format!("net_{}_total_received", interface_name))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let last_transmitted = GLOBAL_SYSTEM_INFO
            .read()
            .await
            .get(&format!("net_{}_total_transmitted", interface_name))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let last_packets_received = GLOBAL_SYSTEM_INFO
            .read()
            .await
            .get(&format!("net_{}_packets_received", interface_name))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let last_packets_transmitted = GLOBAL_SYSTEM_INFO
            .read()
            .await
            .get(&format!("net_{}_packets_transmitted", interface_name))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        let received = if data.total_received() > last_received {
            (data.total_received() - last_received) / per_10s
        } else {
            0
        };
        let transmitted = if data.total_transmitted() > last_transmitted {
            (data.total_transmitted() - last_transmitted) / per_10s
        } else {
            0
        };
        let packets_received = if data.packets_received() > last_packets_received {
            (data.packets_received() - last_packets_received) / per_10s
        } else {
            0
        };
        let packets_transmitted = if data.packets_transmitted() > last_packets_transmitted {
            (data.packets_transmitted() - last_packets_transmitted) / per_10s
        } else {
            0
        };
        let ip: Vec<String> = data.ip_networks().iter().map(|v| v.to_string()).collect();

        let _ = sqlx::query(
            "insert into networks_stats (name,
        received,transmitted,
        errors_on_received,errors_on_transmitted,
        packets_received,packets_transmitted,
        total_received,total_transmitted,
        total_packets_received,total_packets_transmitted,
        total_errors_on_received,total_errors_on_transmitted,
        ipaddrs,created_at) 
        values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
        )
        .bind(interface_name)
        .bind(received as i64)
        .bind(transmitted as i64)
        .bind(data.errors_on_received() as i64)
        .bind(data.errors_on_transmitted() as i64)
        .bind(packets_received as i64)
        .bind(packets_transmitted as i64)
        .bind(data.total_received() as i64)
        .bind(data.total_transmitted() as i64)
        .bind(data.total_packets_received() as i64)
        .bind(data.total_packets_transmitted() as i64)
        .bind(data.total_errors_on_received() as i64)
        .bind(data.total_errors_on_transmitted() as i64)
        .bind(ip.join(","))
        .bind(chrono::Local::now().timestamp())
        .execute(pool)
        .await;

        GLOBAL_SYSTEM_INFO
            .write()
            .await
            .entry(format!("net_{}_packets_received", interface_name))
            .and_modify(|v| *v = data.packets_received().to_string())
            .or_insert(data.packets_received().to_string());
        GLOBAL_SYSTEM_INFO
            .write()
            .await
            .entry(format!("net_{}_packets_transmitted", interface_name))
            .and_modify(|v| *v = data.packets_transmitted().to_string())
            .or_insert(data.packets_transmitted().to_string());
        GLOBAL_SYSTEM_INFO
            .write()
            .await
            .entry(format!("net_{}_total_received", interface_name))
            .and_modify(|v| *v = data.total_received().to_string())
            .or_insert(data.total_received().to_string());
        GLOBAL_SYSTEM_INFO
            .write()
            .await
            .entry(format!("net_{}_total_transmitted", interface_name))
            .and_modify(|v| *v = data.total_transmitted().to_string())
            .or_insert(data.total_transmitted().to_string());
    }

    let _ = sqlx::query(
        "insert into system_stats (loadavg_one,loadavg_five,loadavg_fifteen,cpu_usage,memory_usage,swap_usage,created_at) values ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(load_avg.one)
    .bind(load_avg.five)
    .bind(load_avg.fifteen)
    .bind(cpu_usage)
    .bind(memory_usage)
    .bind(swap_usage)
    .bind(chrono::Local::now().timestamp())
    .execute(pool)
    .await;
}

/// 监控原始数据保留天数（超过即被每日任务清理）。
const MONITOR_RETENTION_DAYS: i64 = 30;

/// 聚合上一整点小时的数据到小时级汇总表（报表/长周期图表使用）。
async fn aggregate_hourly_stats() {
    let pool = get_db_pool().await;
    let now = chrono::Local::now().timestamp();
    let hour_start = (now / 3600 - 1) * 3600;
    let hour_end = hour_start + 3600;

    let _ = sqlx::query(
        "INSERT INTO system_stats_hourly
            (hour_start, avg_loadavg_one, avg_cpu_usage, max_cpu_usage, avg_memory_usage, max_memory_usage, avg_swap_usage)
         SELECT ?, AVG(loadavg_one), AVG(cpu_usage), MAX(cpu_usage), AVG(memory_usage), MAX(memory_usage), AVG(swap_usage)
         FROM system_stats WHERE created_at >= ? AND created_at < ?
         ON CONFLICT(hour_start) DO NOTHING",
    )
    .bind(hour_start)
    .bind(hour_start)
    .bind(hour_end)
    .execute(pool)
    .await;

    let _ = sqlx::query(
        "INSERT INTO networks_stats_hourly
            (name, hour_start, avg_received, avg_transmitted, max_received, max_transmitted)
         SELECT name, ?, AVG(received), AVG(transmitted), MAX(received), MAX(transmitted)
         FROM networks_stats WHERE created_at >= ? AND created_at < ?
         GROUP BY name
         ON CONFLICT(name, hour_start) DO NOTHING",
    )
    .bind(hour_start)
    .bind(hour_start)
    .bind(hour_end)
    .execute(pool)
    .await;
}

/// 清理超过保留期的监控原始数据，防止库无限膨胀。
async fn cleanup_monitor_data() {
    let pool = get_db_pool().await;
    let cutoff = chrono::Local::now().timestamp() - MONITOR_RETENTION_DAYS * 24 * 3600;
    let r1 = sqlx::query("DELETE FROM system_stats WHERE created_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await;
    let r2 = sqlx::query("DELETE FROM networks_stats WHERE created_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await;
    match (r1, r2) {
        (Ok(a), Ok(b)) => info!(
            "监控数据清理完成: system_stats {} 行, networks_stats {} 行",
            a.rows_affected(),
            b.rows_affected()
        ),
        _ => warn!("监控数据清理任务执行异常"),
    }
}

pub async fn init_system_jobs() {
    let sched: JobScheduler = JobScheduler::new()
        .await
        .expect("can't start job scheduler");
    add_jobs(&sched).await;

    if sched.start().await.is_err() {
        info!("scheduled start failed")
    }
    // 启动时立即执行一次存量数据清理
    tokio::spawn(async move {
        cleanup_monitor_data().await;
    });
    // 启动 60s 后补采一次资源用量（避开启动期 IO 高峰）
    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        usage_scheduled_task().await;
    });
    let mut sched_map = GLOBAL_SCHEDULED_MAP.write().await;
    sched_map.insert("zap".to_string(), sched);
}

/// 资源用量采集（用户磁盘 du + 站点磁盘 du + 站点流量日志解析）
async fn usage_scheduled_task() {
    crate::zap::usage::collect_all().await;
}

pub async fn add_jobs(sched: &JobScheduler) {
    let job = Job::new_async("1/10 * * * * *", |_uuid, _lock| {
        Box::pin(system_scheduled_task())
    })
    .unwrap();
    let system_job_uuid = sched.add(job).await.unwrap();
    let mut job_map = GLOBAL_JOB_MAP.write().await;
    job_map.insert("system".to_string(), system_job_uuid.into());

    // 每小时过 5 分：聚合上一整点小时数据
    let agg = Job::new_async("5 * * * * *", |_uuid, _lock| {
        Box::pin(aggregate_hourly_stats())
    })
    .expect("invalid cron: hourly aggregate");
    let _ = sched.add(agg).await;

    // 每日 01:00:00：清理超期监控原始数据
    let clean = Job::new_async("0 0 1 * * *", |_uuid, _lock| {
        Box::pin(cleanup_monitor_data())
    })
    .expect("invalid cron: daily cleanup");
    let _ = sched.add(clean).await;

    // 每 30 分钟（整点 / 半点）：采集用户磁盘用量与站点流量
    let usage = Job::new_async("0 0,30 * * * *", |_uuid, _lock| {
        Box::pin(usage_scheduled_task())
    })
    .expect("invalid cron: usage collect");
    let _ = sched.add(usage).await;

    // 每日 00:05：站点日志轮转（按天切割 + gzip 归档 + 清理超期 + nginx reopen）
    let rotate = Job::new_async("0 5 0 * * *", |_uuid, _lock| {
        Box::pin(crate::zap::logrotate::rotate_all())
    })
    .expect("invalid cron: log rotate");
    let _ = sched.add(rotate).await;
}

pub async fn stop_system_job() {
    let mut zap_sched_map = GLOBAL_SCHEDULED_MAP.write().await;
    if let Some(sched) = zap_sched_map.get_mut("zap") {
        let _ = sched.shutdown().await;
    }
    zap_sched_map.clear();
    GLOBAL_JOB_MAP.write().await.clear();
}

pub async fn start_system_job() {
    let mut zap_sched_map = GLOBAL_SCHEDULED_MAP.write().await;
    let sched: JobScheduler = JobScheduler::new()
        .await
        .expect("can't start job scheduler");
    add_jobs(&sched).await;
    let _ = sched.start().await;
    zap_sched_map.insert("zap".to_string(), sched);
}
