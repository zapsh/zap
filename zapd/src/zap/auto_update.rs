// SPDX-License-Identifier: AGPL-3.0-only
//! 自动更新调度：分钟粒度轮询 `{data}/update_config.yaml`，命中 cron 表达式时触发升级。
//!
//! 用轻量轮询（而非 tokio-cron-scheduler）的原因：自动更新开关与 cron
//! 由前端随时修改并即时写入 YAML，轮询方案配置即时生效、无需重建调度任务；
//! 且 zapd 升级重启后新实例会自然接续，不会留下孤儿调度器。

use std::sync::atomic::Ordering;

use chrono::{Datelike, Timelike};

use crate::zap::updater;

/// 解析 cron 字段（`*` / `*/n` / `a-b` / `a,b` / 单值），返回匹配集合。
fn parse_field(field: &str, min: u32, max: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if part == "*" {
            out.extend(min..=max);
        } else if let Some(step) = part.strip_prefix("*/") {
            if let Ok(n) = step.parse::<u32>()
                && n > 0
            {
                out.extend((min..=max).step_by(n as usize));
            }
        } else if let Some((a, b)) = part.split_once('-') {
            if let (Ok(s), Ok(e)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                out.extend(s.min(e)..=s.max(e));
            }
        } else if let Ok(v) = part.parse::<u32>() {
            out.push(v);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// 标准 5 段 cron（分 时 日 月 周）是否命中给定时刻。
/// 周：0/7 均表示周日。日与周同时受限时取并集命中（宽松，符合常见面板直觉）。
pub fn cron_matches(expr: &str, now: &chrono::DateTime<chrono::Local>) -> bool {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return false;
    }
    let mins = parse_field(parts[0], 0, 59);
    let hours = parse_field(parts[1], 0, 23);
    let days = parse_field(parts[2], 1, 31);
    let months = parse_field(parts[3], 1, 12);
    let mut weeks = parse_field(parts[4], 0, 7);
    if weeks.contains(&7) {
        weeks.push(0); // cron 7 等价周日 0
    }
    mins.contains(&now.minute())
        && hours.contains(&now.hour())
        && days.contains(&now.day())
        && months.contains(&now.month())
        && weeks.contains(&now.weekday().num_days_from_sunday())
}

/// cron 表达式基础校验（段数与合法字符）。
pub fn validate_cron(expr: &str) -> Result<(), String> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(format!(
            "cron 表达式需为 5 段（分 时 日 月 周），当前为 {} 段",
            parts.len()
        ));
    }
    let legal = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '*' | '/' | '-' | ',' | ' '))
    };
    for p in &parts {
        if p.is_empty() || !legal(p) {
            return Err(format!("cron 字段非法: {p}"));
        }
    }
    Ok(())
}

/// 启动自动更新轮询（在 main 启动后调用一次即可）。
pub fn start() {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(20));
        timer.tick().await; // 首次立即 tick
        let mut last_min: i64 = -1;
        loop {
            timer.tick().await;
            let now = chrono::Local::now();
            let minute = now.timestamp() / 60;
            if minute == last_min {
                continue;
            }
            last_min = minute;

            let cfg = updater::load_config();
            if cfg.auto == 0 || cfg.cron.trim().is_empty() {
                continue;
            }
            if updater::UPGRADING.load(Ordering::SeqCst) {
                continue;
            }
            if !cron_matches(&cfg.cron, &now) {
                continue;
            }
            tracing::info!(cron = %cfg.cron, "自动更新定时触发");
            tokio::spawn(async move {
                match updater::launch_update("auto").await {
                    Ok(info) => {
                        tracing::info!(
                            run_id = %info.run_id,
                            version = %info.latest,
                            "自动更新已启动"
                        )
                    }
                    Err(e) => tracing::warn!("自动更新触发失败: {e}"),
                }
            });
        }
    });
}
