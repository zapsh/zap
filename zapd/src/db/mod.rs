// SPDX-License-Identifier: AGPL-3.0-only
pub mod init_db;
/// 菜单种子数据（结构化清单，见模块注释）
pub mod menu_seed;
pub mod models;

use sqlx::{
    Sqlite, SqlitePool,
    pool::Pool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::fs;
use tokio::sync::OnceCell;
use tracing::warn;

pub static DB_POOL: OnceCell<Pool<Sqlite>> = OnceCell::const_new();

pub async fn get_db_pool() -> &'static SqlitePool {
    get_db_pool_opt().await.expect("数据库连接初始化失败")
}

/// 数据库连接（不 panic，初始化失败返回 None，供迁移等非关键路径使用）。
pub async fn get_db_pool_opt() -> Option<&'static SqlitePool> {
    DB_POOL
        .get_or_try_init(|| async {
            let filename = crate::config::get_config().read().unwrap().db.path.clone();

            // 预先以 0o600 创建库文件（若不存在）：确保即便下面的显式 chmod 因故未执行，
            // 文件也不会以默认 0644 落地。仅属主可读写，与 secret.key 一致。
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                if !std::path::Path::new(&filename).exists()
                    && let Err(e) = std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .mode(0o600)
                        .open(&filename)
                {
                    warn!("预创建数据库文件失败 {}: {e}", filename);
                }
            }

            let options = SqliteConnectOptions::new()
                .filename(&filename)
                .create_if_missing(true)
                // 并发场景（部署任务频繁读写 + 面板高频读库）下，未设此项时 UPDATE
                // 会直接报 "database is locked" 而失败；设超时让 sqlite 自旋等待而非报错，
                // 否则 `set_app_deploy_status` 等更新可能被静默丢弃（状态卡在 deploying）。
                .busy_timeout(std::time::Duration::from_secs(5));
            let pool = SqlitePoolOptions::new()
                .max_connections(50)
                .connect_with(options)
                .await?;

            // 数据库含用户凭据 / 令牌等敏感数据，收紧为仅属主读写（0o600），与 secret.key 一致；
            // 同时覆盖 WAL/SHM 伴随文件（sqlite 写入时按进程默认 umask 以 0644 创建），
            // 避免本地其他用户直接读取库文件或 WAL 中的在途明文。
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // 先触发一次被动 checkpoint，使 -wal/-shm 落地，便于一并收紧权限。
                let _ = sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
                    .execute(&pool)
                    .await;
                let perms = fs::Permissions::from_mode(0o600);
                for cand in [
                    filename.clone(),
                    format!("{filename}-wal"),
                    format!("{filename}-shm"),
                ] {
                    if !std::path::Path::new(&cand).exists() {
                        continue;
                    }
                    if let Err(e) = tokio::fs::set_permissions(&cand, perms.clone()).await {
                        warn!("设置数据库文件权限失败 {}: {e}", cand);
                    }
                }
            }
            Ok::<_, sqlx::Error>(pool)
        })
        .await
        .ok()
}
