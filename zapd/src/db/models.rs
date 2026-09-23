use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct UserModel {
    pub id: u64,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Deserialize, Serialize)]
pub struct SystemStatsModel {
    pub id: u64,
    pub loadavg_one: f64,
    pub loadavg_five: f64,
    pub loadavg_fifteen: f64,

    pub cpu_usage: f64,
    pub memory_usage: f64,
    pub swap_usage: f64,

    pub created_at: u64,
}

/// 降采样后的监控点（时间桶 AVG 聚合，见 `get_system_status` 的 range 模式）。
#[derive(Debug, Clone, sqlx::FromRow, Deserialize, Serialize)]
pub struct SystemStatsPoint {
    pub created_at: i64,
    pub loadavg_one: f64,
    pub loadavg_five: f64,
    pub loadavg_fifteen: f64,
    pub cpu_usage: f64,
    pub memory_usage: f64,
    pub swap_usage: f64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, sqlx::FromRow, Deserialize, Serialize)]
pub struct NetworksStatsModel {
    pub id: u64,
    pub name: String,
    pub received: i64,
    pub transmitted: i64,
    pub errors_on_received: i64,
    pub errors_on_transmitted: i64,
    pub packets_received: i64,
    pub packets_transmitted: i64,
    pub total_received: i64,
    pub total_transmitted: i64,
    pub total_packets_received: i64,
    pub total_packets_transmitted: i64,
    pub total_errors_on_received: i64,
    pub total_errors_on_transmitted: i64,
    pub ipaddrs: String,
    pub created_at: u64,
}
#[derive(Debug, Clone, Deserialize, Serialize, sqlx::FromRow)]
pub struct NetworksStatsForDashboard {
    pub name: String,
    pub received: i64,
    pub transmitted: i64,
    pub packets_received: i64,
    pub packets_transmitted: i64,
    pub total_received: i64,
    pub total_transmitted: i64,
    pub ipaddrs: String,
    pub created_at: u64,
}
