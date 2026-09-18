//! 数据迁移（服务器配置 → 数据迁移，仅 admin）。
//!
//! /home 磁盘不足时，把用户家目录数据整体迁移到新挂载点（如 /home2）：
//! 1. 物理搬移由 zapexec 以 root 执行（`user.home_migrate`，支持跨文件系统）；
//! 2. 更新 `user.home_dir` 与站点 `web_root/log_root`（数据库路径前缀跟随）；
//! 3. 对涉及站点重新同步 Nginx vhost / PHP-FPM pool。
//!
//! 团队成员（子账号）共享父账号的家目录与 Linux 账号，因此**同一家目录只搬移一次**，
//! 其余共享账号直接复用搬移结果，仅更新库中路径并重新同步其名下站点。
//!
//! 存量用户迁移不改变其登录凭据与站点配置；新用户仍按「运行环境默认设置」
//! 里的默认挂载点（user_home_root）创建。

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::{
    Json,
    extract::{Extension, Query},
};
use serde::Deserialize;
use serde_json::json;

use crate::db;
use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;
use zap_proto::Request;

/// 挂载点/家目录路径合法（绝对路径、无 `..`、无空白、非根）。
fn mount_ok(m: &str) -> bool {
    !m.is_empty() && m.starts_with('/') && !m.contains("..") && !m.contains(' ') && m.len() > 1
}

fn norm(m: &str) -> String {
    m.trim().trim_end_matches('/').to_string()
}

#[derive(Debug, Deserialize)]
pub struct MigratePreviewQuery {
    /// 源挂载点（默认 /home）
    src: Option<String>,
}

/// GET /system/migrate/users?src=/home：列出位于源挂载点下、可迁移的面板用户。
pub async fn migrate_users_preview(
    claims: ValidatedClaims,
    Query(q): Query<MigratePreviewQuery>,
) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可查看数据迁移".to_string()));
    }
    let src = norm(q.src.as_deref().unwrap_or("/home"));
    if !mount_ok(&src) {
        return Err(ZapError::New(-1, "源挂载点非法".to_string()));
    }

    let pool = db::get_db_pool().await;
    // 站点数按「家目录」统计而非单个账号：团队成员共享父账号家目录，其站点也在同一目录下，
    // 搬移时是一次整体搬走；share_count = 共享该家目录的面板账号数（含自己）。
    let rows: Vec<(i64, String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT u.id, u.username, u.linux_user, u.home_dir,
                (SELECT COUNT(*) FROM site s
                  WHERE s.user_id IN (SELECT x.id FROM user x WHERE x.home_dir = u.home_dir)),
                (SELECT COUNT(*) FROM user x WHERE x.home_dir = u.home_dir)
         FROM user u WHERE u.home_dir LIKE ? AND u.home_dir != '' ORDER BY u.id",
    )
    .bind(format!("{src}/%"))
    .fetch_all(pool)
    .await?;

    let candidates = rows
        .into_iter()
        .map(
            |(id, username, linux_user, home_dir, site_count, share_count)| {
                json!({
                    "id": id,
                    "username": username,
                    "linux_user": linux_user,
                    "home_dir": home_dir,
                    "site_count": site_count,
                    "share_count": share_count,
                })
            },
        )
        .collect::<Vec<_>>();

    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": { "src": src, "count": candidates.len(), "candidates": candidates }
    })))
}

#[derive(Debug, Deserialize)]
pub struct MigratePayload {
    /// 目标挂载点（如 /home2，须已挂载好且目录存在权限可写）
    pub dest: String,
    /// 源挂载点（默认 /home）
    #[serde(default)]
    pub src: Option<String>,
    /// 指定迁移的用户；空 = 迁移源挂载点下的全部用户
    #[serde(default)]
    pub user_ids: Option<Vec<i64>>,
}

/// POST /system/migrate/home：把用户从源挂载点迁移到目标挂载点。
pub async fn migrate_home_mv(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<MigratePayload>,
) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可执行数据迁移".to_string()));
    }
    let src = norm(payload.src.as_deref().unwrap_or("/home"));
    let dest = norm(&payload.dest);
    if !mount_ok(&src) || !mount_ok(&dest) {
        return Err(ZapError::New(-1, "源/目标挂载点非法".to_string()));
    }
    if src == dest {
        return Err(ZapError::New(
            -1,
            "源与目标挂载点相同，无需迁移".to_string(),
        ));
    }
    if dest == "/home" {
        return Err(ZapError::New(-1, "目标挂载点不能是默认 /home".to_string()));
    }
    let pool = db::get_db_pool().await;
    let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT id, username, linux_user, home_dir FROM user
         WHERE home_dir LIKE ? AND home_dir != '' ORDER BY id",
    )
    .bind(format!("{src}/%"))
    .fetch_all(pool)
    .await?;

    // 按 user_ids 收敛（空 = 全部）
    let ids: Option<std::collections::HashSet<i64>> =
        payload.user_ids.map(|v| v.into_iter().collect());
    let mut ok_items: Vec<serde_json::Value> = Vec::new();
    let mut fail_items: Vec<serde_json::Value> = Vec::new();
    // 已搬移的家目录：旧路径 → 新路径。团队成员共享父账号家目录时，
    // 目录只搬一次，后续账号直接复用结果（避免「源目录已不存在」的失败）。
    let mut moved: HashMap<String, String> = HashMap::new();
    // 已处理过的账号（同一家目录下的账号会被成组处理，避免重复更新/重复同步）
    let mut done: std::collections::HashSet<i64> = std::collections::HashSet::new();

    for (id, username, linux_user, old_home) in rows {
        if let Some(set) = &ids
            && !set.contains(&id)
        {
            continue;
        }
        if done.contains(&id) {
            continue;
        }
        let name = old_home.rsplit('/').next().unwrap_or("");
        if name.is_empty() {
            fail_items.push(json!({ "id": id, "username": username, "error": "家目录路径非法" }));
            continue;
        }
        let new_home = format!("{dest}/{name}");

        // 同一家目录（站长 + 其团队成员）只搬一次：已搬过则直接复用，
        // 不再调用执行端（源目录此时已被搬走，重复调用必然失败）。
        let reused = moved.get(&old_home).is_some_and(|d| d == &new_home);
        if !reused {
            // 迁移时同步更新 Linux 账号家目录指针（usermod -d）；
            // 账号名缺失（历史数据）时按用户名派生，随后由用户同步落库。
            let owner = if linux_user.is_empty() {
                zap_proto::linux_username(&username)
            } else {
                linux_user.clone()
            };
            let resp = match crate::zapexec::call(Request::UserHomeMigrate {
                src_home: old_home.clone(),
                dest_home: new_home.clone(),
                owner,
            })
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    fail_items.push(json!({
                        "id": id, "username": username,
                        "home_dir": old_home,
                        "error": format!("执行端通信失败：{e}"),
                    }));
                    continue;
                }
            };
            if resp.code != 0 {
                fail_items.push(json!({
                    "id": id, "username": username,
                    "home_dir": old_home,
                    "error": format!("搬移失败：{}", resp.message),
                }));
                continue;
            }
            moved.insert(old_home.clone(), new_home.clone());
        }

        // 目录已被整体搬走：共享该家目录的所有面板账号（站长 + 团队成员）都要跟着更新，
        // 否则残留的旧路径会让这些账号的站点指向已失效的目录。
        let group: Vec<(i64, String, String)> = sqlx::query_as(
            "SELECT id, username, linux_user FROM user WHERE home_dir = ? ORDER BY id",
        )
        .bind(&old_home)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        let group = if group.is_empty() {
            vec![(id, username.clone(), linux_user.clone())]
        } else {
            group
        };

        for (idx, (uid, uname, ulu)) in group.iter().enumerate() {
            // 数据迁移中站点 vhost/FPM 会重建，先记录涉及站点
            let site_ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM site WHERE user_id = ?")
                .bind(uid)
                .fetch_all(pool)
                .await
                .unwrap_or_default();

            // 1) 更新用户家目录
            let _ = sqlx::query("UPDATE user SET home_dir = ? WHERE id = ?")
                .bind(&new_home)
                .bind(uid)
                .execute(pool)
                .await;
            // 2) 站点路径前缀跟随（web_root / log_root）
            let _ = sqlx::query(
                "UPDATE site SET web_root = REPLACE(web_root, ?, ?), log_root = REPLACE(log_root, ?, ?)
                 WHERE user_id = ?",
            )
            .bind(&old_home)
            .bind(&new_home)
            .bind(&old_home)
            .bind(&new_home)
            .bind(uid)
            .execute(pool)
            .await;

            // 3) 涉及站点重新同步（nginx vhost / FPM pool 使用新路径）
            let mut site_errors: Vec<String> = Vec::new();
            let mut synced = 0;
            for sid in &site_ids {
                match crate::routers::site::sync_one_site(*sid).await {
                    Ok(_) => synced += 1,
                    Err(e) => site_errors.push(format!("site#{sid}: {e}")),
                }
            }

            ok_items.push(json!({
                "id": uid,
                "username": uname,
                "linux_user": ulu,
                "old_home": old_home,
                "new_home": new_home,
                "sites": site_ids.len(),
                "sites_synced": synced,
                "site_errors": site_errors,
                // 共享家目录：仅第一个账号真正执行搬移，其余复用结果
                "reused": reused || idx > 0,
            }));
            done.insert(*uid);
        }
    }

    let ok_n = ok_items.len();
    let fail_n = fail_items.len();
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "user_home_migrate",
        &format!("{src} → {dest}"),
        &format!("ok={ok_n} fail={fail_n}"),
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": format!("迁移完成：成功 {ok_n}，失败 {fail_n}"),
        "data": { "src": src, "dest": dest, "mode": "system", "ok": ok_items, "fail": fail_items }
    })))
}
