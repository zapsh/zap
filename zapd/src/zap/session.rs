//! 会话版本号（tokenVersion）与「下线所有设备」。
//!
//! JWT 是无状态的：签发后服务端不保存任何会话，因此无法按 token 逐个作废
//! （登出只是让浏览器丢弃 Cookie，token 本身在 `exp` 之前依然有效）。
//!
//! 折中做法是给每个用户一个**单调递增的版本号**：
//!
//! - 签发时把版本号写进 Claims 的 `tv`（见 [`crate::zap::jwt::Claims`]）；
//! - 每次校验 token 时比对：小于库中当前版本号的 token 一律判为已下线；
//! - 「下线所有设备」= 版本号 +1，所有已签发的 token（面板、Web 应用 Cookie、
//!   静态 API Token）随之失效。
//!
//! 版本号在内存里缓存一份（启动时全量载入），校验路径因此不必查库——
//! 每个请求只是一次 `RwLock` 读。

use std::collections::HashMap;
use std::sync::RwLock;

use once_cell::sync::Lazy;
use tracing::error;

use crate::db;

/// user_id -> 当前会话版本号。
static VERSIONS: Lazy<RwLock<HashMap<i64, i64>>> = Lazy::new(|| RwLock::new(HashMap::new()));

/// 启动时把全量用户的版本号载入内存。
///
/// 必须在建库之后调用：否则老库里已经「下线过所有设备」的用户（版本号 > 0）
/// 在缓存未命中时会被当成 0，签发出立刻失效的 token。
pub async fn load_all() {
    let pool = db::get_db_pool().await;
    let rows: Vec<(i64, i64)> = sqlx::query_as("SELECT id, token_version FROM user")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    match VERSIONS.write() {
        Ok(mut m) => {
            m.clear();
            for (id, version) in rows {
                m.insert(id, version);
            }
        }
        Err(e) => error!("载入会话版本号失败: {e}"),
    }
}

/// 该用户的当前版本号。
///
/// 缓存未命中一律按 0 处理：新建用户的默认值就是 0，因此这个回退是正确的
/// （历史用户已在 [`load_all`] 载入）。
pub fn version_of(user_id: u64) -> i64 {
    // 锁被污染时降级为 0（放行），而不是让整个请求失败——
    // 版本号只影响「是否已下线」，不影响其它任何校验。
    VERSIONS
        .read()
        .map(|m| m.get(&(user_id as i64)).copied().unwrap_or(0))
        .unwrap_or(0)
}

/// 递增会话版本号（下线该用户名下的所有设备），返回新版本号。
pub async fn bump(user_id: i64) -> Result<i64, sqlx::Error> {
    let pool = db::get_db_pool().await;
    sqlx::query("UPDATE user SET token_version = token_version + 1 WHERE id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    let (version,): (i64,) = sqlx::query_as("SELECT token_version FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    // 静态 API Token 一并对齐到新版本号：否则「下线所有设备」会漏掉这条后门
    sqlx::query("UPDATE api_token SET token_version = ? WHERE user_id = ?")
        .bind(version)
        .bind(user_id)
        .execute(pool)
        .await?;
    set(user_id, version);
    Ok(version)
}

/// 直接写入缓存（新建用户 / 重建库后同步用）。
pub fn set(user_id: i64, version: i64) {
    if let Ok(mut m) = VERSIONS.write() {
        m.insert(user_id, version);
    }
}
