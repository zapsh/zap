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
use tokio::sync::OnceCell;

pub static DB_POOL: OnceCell<Pool<Sqlite>> = OnceCell::const_new();

pub async fn get_db_pool() -> &'static SqlitePool {
    get_db_pool_opt().await.expect("数据库连接初始化失败")
}

/// 数据库连接（不 panic，初始化失败返回 None，供迁移等非关键路径使用）。
pub async fn get_db_pool_opt() -> Option<&'static SqlitePool> {
    DB_POOL
        .get_or_try_init(|| async {
            let filename = crate::config::get_config().read().unwrap().db.path.clone();
            let options = SqliteConnectOptions::new()
                .filename(filename)
                .create_if_missing(true)
                // 并发场景（部署任务频繁读写 + 面板高频读库）下，未设此项时 UPDATE
                // 会直接报 "database is locked" 而失败；设超时让 sqlite 自旋等待而非报错，
                // 否则 `set_app_deploy_status` 等更新可能被静默丢弃（状态卡在 deploying）。
                .busy_timeout(std::time::Duration::from_secs(5));
            SqlitePoolOptions::new()
                .max_connections(50)
                .connect_with(options)
                .await
        })
        .await
        .ok()
}
