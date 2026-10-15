// SPDX-License-Identifier: AGPL-3.0-only
<<<<<<< HEAD
//! 
=======
//!
>>>>>>> 9d9e2db (fix(security): stop path traversal in plugin log watch via filename allowlist + canonicalize)
//! 初始管理员的运行实体自愈 + `--init-admin` 子命令。
//!
//! 全新数据库只在 `user` 表里写了一条记录，真正的 Linux 账号与家目录要等
//! zapexec(root) 去建。install.sh 在 Linux 上会先行建好，这里负责它没覆盖到的
//! 场景：手动部署（没走 install.sh）、以及家目录被误删之后的重启恢复。

use std::{path::Path, time::Duration};

use tracing::{error, info, warn};

/// `zapd --init-admin <username> [--admin-password <password>]`
///
<<<<<<< HEAD
/// 1. zap.db not exist → create admin user → exit. 
=======
/// 1. zap.db not exist → create admin user → exit.
>>>>>>> 9d9e2db (fix(security): stop path traversal in plugin log watch via filename allowlist + canonicalize)
/// only for first-time setup, or when the admin account is missing.
pub async fn init_admin_cli(username: &str, password: Option<&str>) -> i32 {
    let username = username.trim().to_ascii_lowercase();
    if !crate::db::init_db::valid_admin_username(&username) {
<<<<<<< HEAD
        error!("Username '{username}' is invalid: Start with [a-z_]; only [a-z0-9_-]; max 32 chars.");
=======
        error!(
            "Username '{username}' is invalid: Start with [a-z_]; only [a-z0-9_-]; max 32 chars."
        );
>>>>>>> 9d9e2db (fix(security): stop path traversal in plugin log watch via filename allowlist + canonicalize)
        return 1;
    }
    let password = match password.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => match zap_crypto::generate_password(16, false) {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to generate a random password: {e}");
                return 1;
            }
        },
    };

    crate::db::init_db::set_initial_admin(&username, &password);
    crate::db::init_db::init_schema().await;
    let created = crate::db::init_db::ensure_initial_admin().await;
    // 家目录 / Linux 账号：best-effort，zapexec 还没起来时也不影响账号本身
    ensure_admin_home().await;

    if created {
        println!("administrator has created:{username} / {password}");
    } else {
        let existing = admin_username().await.unwrap_or_else(|| "?".to_string());
        println!("administrator already exists, no changes made:{existing}");
    }
    0
}

/// 补齐初始管理员的 Linux 账号与家目录骨架（幂等，best-effort）。
///
/// 在后台任务里调用：zapd 刚起来时 zapexec 可能还没就绪，因此失败会重试几次。
pub async fn ensure_admin_home() {
    let Some((uid, home_dir)) = admin_home().await else {
        return;
    };
    if home_dir.is_empty() {
        warn!("管理员账号未配置家目录（home_dir 为空），跳过家目录初始化");
        return;
    }
    if Path::new(&home_dir).exists() {
        return;
    }

    info!("管理员家目录 {home_dir} 不存在，正在通过 zapexec 初始化 ...");
    let mut last_err = String::new();
    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        match crate::routers::user::ensure_user_runtime(uid).await {
            Ok(()) => {
                info!("管理员家目录已就绪：{home_dir}");
                return;
            }
            Err(e) => {
                last_err = e;
                warn!(
                    "初始化管理员家目录失败（第 {} 次）：{last_err}",
                    attempt + 1
                );
            }
        }
    }
    warn!(
        "管理员家目录初始化未成功（{home_dir}）：{last_err}。\
         面板其它功能不受影响，可稍后在「用户」页面重试或手动创建"
    );
}

/// 取管理员的用户名。
async fn admin_username() -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT username FROM user WHERE roles LIKE '%admin%' ORDER BY id LIMIT 1",
    )
    .fetch_optional(crate::db::get_db_pool().await)
    .await
    .ok()
    .flatten()
}

/// 取管理员的 uid 与家目录（roles 含 admin 的第一个账号）。
async fn admin_home() -> Option<(i64, String)> {
    sqlx::query_as::<_, (i64, String)>(
        "SELECT id, home_dir FROM user WHERE roles LIKE '%admin%' ORDER BY id LIMIT 1",
    )
    .fetch_optional(crate::db::get_db_pool().await)
    .await
    .ok()
    .flatten()
}
