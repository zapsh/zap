// SPDX-License-Identifier: AGPL-3.0-only
//! 系统软件源管理（系统管理 → 软件源，仅管理员）。
//!
//! 端点（均需管理员）：
//! - GET  /system/repo/status    发行版识别 + 源文件清单
//! - GET  /system/repo/catalog   镜像站目录 + 第三方源预设（按发行版生成内容）
//! - POST /system/repo/preview   换源预览（写入前先看 diff）
//! - POST /system/repo/apply     执行换源（逐文件备份 + 写入，可顺带更新缓存）
//! - GET  /system/repo/file      读源文件内容
//! - POST /system/repo/file      保存源文件（手动编辑 / 添加第三方源）
//! - POST /system/repo/delete    删除第三方源文件（删除前备份）
//! - GET  /system/repo/backups   备份列表（「最初的源」从这里还原）
//! - POST /system/repo/restore   从备份还原
//! - POST /system/repo/cache     更新包管理器缓存
//!
//! 所有 root 动作（读写 /etc、跑包管理器）都走 `zapexec` 白名单动词，
//! 本文件只做参数校验、内容生成与审计，不直接碰文件系统。

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{Extension, Query};
use serde::Deserialize;
use serde_json::{Value, json};

use zap_proto::Request;

use crate::zap::api_ok;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;
use crate::zap::osrepo;
use crate::zap::{ZapError, ZapJsonResult};

fn require_admin(claims: &ValidatedClaims) -> Result<(), ZapError> {
    if is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "仅管理员可管理系统软件源".to_string()))
    }
}

/// 执行一次 zapexec 请求，透传其 data 载荷。
async fn exec(req: Request) -> Result<Value, ZapError> {
    let resp = crate::zapexec::call(req).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(resp.data.unwrap_or(Value::Null))
}

/// rpm 系换源前需要先读到候选文件内容（读 /etc 需要 root，走 exec）。
async fn read_rpm_candidates(
    distro: &osrepo::DistroInfo,
) -> Result<Vec<(String, String)>, ZapError> {
    if distro.family != "rhel" {
        return Ok(vec![]);
    }
    let list = exec(Request::OsRepoList).await?;
    let files = list
        .get("files")
        .and_then(|f| f.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for f in files {
        let Some(path) = f.get("path").and_then(|p| p.as_str()) else {
            continue;
        };
        if !(osrepo::is_distro_repo_file(path, &distro.id) || osrepo::is_epel_repo_file(path)) {
            continue;
        }
        let data = exec(Request::OsRepoRead {
            path: path.to_string(),
        })
        .await?;
        let content = data
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();
        out.push((path.to_string(), content));
    }
    Ok(out)
}

/// 组装换源改动列表（可选带上 EPEL）。
async fn build_switch_changes(
    distro: &osrepo::DistroInfo,
    mirror_id: &str,
    include_epel: bool,
) -> Result<Vec<(String, String)>, ZapError> {
    let contents = read_rpm_candidates(distro).await?;
    let mut changes = osrepo::switch_preview(distro, mirror_id, &contents)
        .ok_or_else(|| ZapError::New(-1, "当前发行版不支持在线换源".to_string()))?;
    if include_epel
        && let Some(mirror) = osrepo::mirrors_for(&distro.family, &distro.id)
            .into_iter()
            .find(|m| m.id == mirror_id)
    {
        changes.extend(osrepo::epel_switch_preview(mirror.url, &contents));
    }
    if changes.is_empty() {
        return Err(ZapError::New(-1, "没有找到可换源的源文件".to_string()));
    }
    Ok(changes)
}

/// GET /system/repo/status
pub async fn status(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let data = exec(Request::OsRepoList).await?;
    api_ok(data)
}

/// GET /system/repo/catalog
pub async fn catalog(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let distro = osrepo::detect();
    api_ok(json!({
        "distro": distro,
        "switchable": osrepo::switchable(&distro),
        "mirrors": osrepo::mirrors_for(&distro.family, &distro.id),
        "presets": osrepo::presets_for(&distro),
    }))
}

#[derive(Debug, Deserialize)]
pub struct MirrorBody {
    pub mirror_id: String,
    #[serde(default)]
    pub include_epel: bool,
}

/// POST /system/repo/preview
pub async fn preview(claims: ValidatedClaims, Json(body): Json<MirrorBody>) -> ZapJsonResult {
    require_admin(&claims)?;
    let distro = osrepo::detect();
    let changes = build_switch_changes(&distro, &body.mirror_id, body.include_epel).await?;
    let items: Vec<Value> = changes
        .into_iter()
        .map(|(path, content)| json!({ "path": path, "content": content }))
        .collect();
    api_ok(json!({ "changes": items }))
}

#[derive(Debug, Deserialize)]
pub struct ApplyBody {
    pub mirror_id: String,
    #[serde(default)]
    pub include_epel: bool,
    #[serde(default)]
    pub update_cache: bool,
}

/// POST /system/repo/apply
pub async fn apply(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<ApplyBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let distro = osrepo::detect();
    let changes = build_switch_changes(&distro, &body.mirror_id, body.include_epel).await?;
    let mut written: Vec<String> = Vec::new();
    for (path, content) in &changes {
        exec(Request::OsRepoSave {
            path: path.clone(),
            content: content.clone(),
            dry_run: false,
        })
        .await?;
        written.push(path.clone());
    }
    if body.update_cache {
        exec(Request::OsRepoCacheUpdate {
            manager: String::new(),
        })
        .await?;
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "os_repo_switch",
        &distro.id,
        &format!(
            "换源为 {}（{} 个文件{}）",
            body.mirror_id,
            written.len(),
            if body.update_cache {
                "，已更新缓存"
            } else {
                ""
            }
        ),
    )
    .await;
    api_ok(json!({
        "written": written,
        "cacheUpdated": body.update_cache,
    }))
}

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub path: String,
}

/// 备份文件名查询参数（`?name=xxx.bak`）。
#[derive(Debug, Deserialize)]
pub struct BackupNameQuery {
    pub name: String,
}

/// GET /system/repo/file
pub async fn file_read(claims: ValidatedClaims, Query(q): Query<PathQuery>) -> ZapJsonResult {
    require_admin(&claims)?;
    let data = exec(Request::OsRepoRead { path: q.path }).await?;
    api_ok(data)
}

#[derive(Debug, Deserialize)]
pub struct FileSaveBody {
    pub path: String,
    pub content: String,
}

/// POST /system/repo/file（手动编辑 / 添加第三方源共用）
pub async fn file_save(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<FileSaveBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let data = exec(Request::OsRepoSave {
        path: body.path.clone(),
        content: body.content.clone(),
        dry_run: false,
    })
    .await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "os_repo_save",
        &body.path,
        &format!("写入 {} 字节", body.content.len()),
    )
    .await;
    api_ok(data)
}

/// POST /system/repo/delete
pub async fn file_delete(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<PathQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let data = exec(Request::OsRepoDelete {
        path: body.path.clone(),
    })
    .await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "os_repo_delete",
        &body.path,
        "",
    )
    .await;
    api_ok(data)
}

/// GET /system/repo/backups
pub async fn backups(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let data = exec(Request::OsRepoBackups).await?;
    api_ok(data)
}

/// GET /system/repo/backup?name=xxx.bak —— 预览备份内容
pub async fn backup_read(
    claims: ValidatedClaims,
    Query(q): Query<BackupNameQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let data = exec(Request::OsRepoBackupRead { name: q.name }).await?;
    api_ok(data)
}

#[derive(Debug, Deserialize)]
pub struct RestoreBody {
    pub backup: String,
    pub path: String,
}

/// POST /system/repo/restore
pub async fn restore(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<RestoreBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let data = exec(Request::OsRepoRestore {
        backup: body.backup.clone(),
        path: body.path.clone(),
    })
    .await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "os_repo_restore",
        &body.path,
        &format!("从备份 {} 还原", body.backup),
    )
    .await;
    api_ok(data)
}

/// POST /system/repo/cache
pub async fn cache(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let data = exec(Request::OsRepoCacheUpdate {
        manager: String::new(),
    })
    .await?;
    api_ok(data)
}
