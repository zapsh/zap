//! 站点 nginx 日志轮转（按天）：切割 → gzip → 清理超期归档 → nginx reopen。
//!
//! 替代外部 logrotate，与流量统计配合：
//! - 轮转前先跑一次带宽采集，避免「上次采集后到切割前」的增量随归档丢失；
//! - 切割后当前 `access.log` 变成新文件（inode 变化），下一轮采集自动从 0 重读。

use tracing::{info, warn};

use crate::db::get_db_pool;
use zap_proto::Request;

/// 归档保留天数（超过即删除）
pub const KEEP_DAYS: u32 = 30;

/// 每日任务：轮转全部站点的 access.log / error.log
pub async fn rotate_all() {
    crate::zap::usage::collect_bandwidth().await;

    let pool = get_db_pool().await;
    let roots: Vec<String> = sqlx::query_scalar("SELECT log_root FROM site WHERE log_root <> ''")
        .fetch_all(pool)
        .await
        .unwrap_or_default();

    // 即便没有带独立日志目录的站点，全局 WAF 审计日志（/var/log/modsec_audit.log）仍要轮转，
    // 因此即使 roots 为空也照常调用轮转（动词内部会轮转审计日志）。
    match crate::zapexec::call(Request::SiteLogRotate {
        log_roots: roots,
        keep_days: KEEP_DAYS,
    })
    .await
    {
        Ok(resp) => {
            if resp.code != 0 {
                warn!("站点日志轮转失败: {}", resp.message);
                return;
            }
            let d = resp.data.unwrap_or_default();
            let rotated = d
                .get("rotated")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let removed = d.get("removed").and_then(|v| v.as_u64()).unwrap_or(0);
            let reopened = d.get("reopened").and_then(|v| v.as_bool()).unwrap_or(false);
            info!(
                "站点日志轮转完成: 切割 {} 个文件，清理 {} 个归档，nginx reopen={}",
                rotated, removed, reopened
            );
        }
        Err(e) => warn!("站点日志轮转请求失败: {}", e),
    }
}
