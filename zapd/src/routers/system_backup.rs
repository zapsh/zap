//! 备份中心（"备份"）：目录 / 数据库的一次性备份与还原，以及可定时执行的备份任务。
//!
//! 所有真正的文件/数据库操作都经 `zapexec`（root）完成——备份目录属主为 root，
//! zapd 以非 root 运行、无权直接读写，故列清单 / 删除 / 还原也走动词。
//!
//! 云存储自动上传（文件管理里已配置的云存储）作为后续能力，任务表已预留 `cloud_id`
//! 字段；本阶段 `cloud_id` 仅记录、暂不触发上传。

use std::net::SocketAddr;

use axum::extract::{Extension, Query};
use axum::Json;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::Row;

use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;
use zap_proto::Request;

fn now_ts() -> i64 {
    chrono::Local::now().timestamp()
}

fn require_admin(claims: &ValidatedClaims) -> Result<(), ZapError> {
    if is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "仅管理员可使用备份功能".to_string()))
    }
}

fn default_backup_dir() -> String {
    let base = std::env::var("ZAP_PATH").unwrap_or_else(|_| "/usr/local/zap".to_string());
    format!("{base}/data/backup")
}

/// 当前生效的备份根目录：面板设置的 `backup.backup_dir` 优先，空则回退默认。
fn backup_root_path() -> String {
    let dir = crate::config::get_config()
        .read()
        .ok()
        .and_then(|c| {
            let d = c.backup.backup_dir.trim().to_string();
            if d.is_empty() { None } else { Some(d) }
        });
    dir.unwrap_or_else(default_backup_dir)
}

// ── 请求体 ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct DirTarget {
    pub paths: Vec<String>,
    #[serde(default)]
    pub as_user: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DbTarget {
    pub engine: String,
    pub db_name: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: i32,
    /// 本机 socket 路径（优先于 host/port；空则走 TCP）
    #[serde(default)]
    pub socket: Option<String>,
    #[serde(default)]
    pub db_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BackupDirPayload {
    pub name: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub dest_dir: String,
    #[serde(default)]
    pub as_user: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BackupDbPayload {
    pub name: String,
    pub engine: String,
    pub db_name: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: i32,
    #[serde(default)]
    pub db_path: Option<String>,
    #[serde(default)]
    pub dest_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct BackupListQuery {
    #[serde(default)]
    pub dir: String,
}

#[derive(Debug, Deserialize)]
pub struct BackupDeletePayload {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct BackupRestoreDirPayload {
    pub path: String,
    pub target_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct DbQuickPayload {
    /// 要导出的数据库名（面板自己的 zapadm 凭据 + 本机 socket 直连，无需前端传密码）
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct BackupRestoreDbPayload {
    pub path: String,
    pub engine: String,
    pub db_name: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: i32,
    #[serde(default)]
    pub db_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BackupJobPayload {
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    /// dir | db
    pub target_type: String,
    /// JSON 字符串（DirTarget / DbTarget）
    pub target: String,
    #[serde(default)]
    pub schedule: String,
    #[serde(default = "default_one")]
    pub enabled: i64,
    #[serde(default = "default_retain")]
    pub retain_count: i64,
    #[serde(default)]
    pub cloud_id: String,
}

fn default_one() -> i64 {
    1
}
fn default_retain() -> i64 {
    7
}

#[derive(Debug, Deserialize)]
pub struct BackupJobRunPayload {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct BackupSettingPayload {
    /// 新的备份根目录（绝对路径）。空字符串 = 恢复默认 `{ZAP_PATH}/data/backup`。
    pub path: String,
}

// ── 任务行 ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
struct BackupJobRow {
    id: i64,
    name: String,
    target_type: String,
    target: String,
    schedule: String,
    enabled: i64,
    retain_count: i64,
    cloud_id: String,
    last_run_at: i64,
    last_status: i64,
    last_message: String,
}

// ── 核心：执行一次备份（手动 / 定时共用）───────────────────────────

/// 执行一次备份：写记录 → 调 zapexec → 更新记录；定时任务（job_id 有值）会按
/// `retain_count` 清理旧归档。返回 `{ record_id, path, size, data }`。
pub async fn run_backup(
    target_type: &str,
    target_json: &str,
    name: &str,
    dest_dir: &str,
    job_id: Option<i64>,
) -> Result<Value, ZapError> {
    let ts = now_ts();
    // 定时任务给归档名加时间戳，避免覆盖上一次；手动用原名。
    let eff_name = match job_id {
        Some(_) => format!("{name}-{ts}"),
        None => name.to_string(),
    };

    let pool = crate::db::get_db_pool().await;
    let backup_root = backup_root_path();
    let rec = sqlx::query(
        "INSERT INTO backup_records (job_id, kind, name, status, created_at) VALUES (?,?,?,0,?)",
    )
    .bind(job_id.unwrap_or(0))
    .bind(target_type)
    .bind(&eff_name)
    .bind(ts)
    .execute(pool)
    .await
    .map_err(|e| ZapError::New(-1, format!("写备份记录失败: {e}")))?;
    let record_id = rec.last_insert_rowid();

    let resp = match target_type {
        "dir" => {
            let t: DirTarget = serde_json::from_str(target_json)
                .map_err(|e| ZapError::New(-1, format!("target 解析失败: {e}")))?;
            crate::zapexec::call(Request::BackupDir {
                name: eff_name.clone(),
                paths: t.paths,
                dest_dir: dest_dir.to_string(),
                as_user: t.as_user,
                skip_owner_check: false,
                backup_root: backup_root.clone(),
            })
            .await?
        }
        "db" => {
            let t: DbTarget = serde_json::from_str(target_json)
                .map_err(|e| ZapError::New(-1, format!("target 解析失败: {e}")))?;
            crate::zapexec::call(Request::BackupDb {
                name: eff_name.clone(),
                engine: t.engine,
                db_name: t.db_name,
                user: t.user,
                password: t.password,
                host: t.host,
                port: t.port,
                socket: t.socket,
                dest_dir: dest_dir.to_string(),
                db_path: t.db_path,
                backup_root: backup_root.clone(),
            })
            .await?
        }
        _ => return Err(ZapError::New(-1, "未知备份类型".to_string())),
    };

    if resp.code != 0 {
        let _ = sqlx::query("UPDATE backup_records SET status=-1, message=? WHERE id=?")
            .bind(&resp.message)
            .bind(record_id)
            .execute(pool)
            .await;
        return Err(ZapError::New(-1, resp.message));
    }

    let data = resp.data.clone().unwrap_or(Value::Null);
    let bpath = data["path"].as_str().unwrap_or("").to_string();
    let bsize = data["size"].as_i64().unwrap_or(0);
    let _ = sqlx::query("UPDATE backup_records SET status=1, path=?, size=? WHERE id=?")
        .bind(&bpath)
        .bind(bsize)
        .bind(record_id)
        .execute(pool)
        .await;

    if let Some(jid) = job_id {
        if let Ok(retain) = job_retain(jid).await {
            if retain > 0 && !bpath.is_empty() {
                prune_old(jid, retain, &bpath, &backup_root).await;
            }
        }
    }

    Ok(json!({
        "record_id": record_id,
        "path": bpath,
        "size": bsize,
        "data": data,
    }))
}

async fn job_retain(id: i64) -> Result<i64, ZapError> {
    let pool = crate::db::get_db_pool().await;
    let row: Option<(i64,)> = sqlx::query_as("SELECT retain_count FROM backup_jobs WHERE id=?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ZapError::New(-1, format!("读任务失败: {e}")))?;
    Ok(row.map(|r| r.0).unwrap_or(0))
}

/// 保留最新 `keep` 份，其余经 zapexec 删除（zapd 无备份根写权限）。
async fn prune_old(job_id: i64, keep: i64, keep_path: &str, backup_root: &str) {
    let pool = crate::db::get_db_pool().await;
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, path FROM backup_records WHERE job_id=? AND status=1 AND path<>'' ORDER BY id DESC",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (rid, path) in rows.into_iter().skip(keep as usize) {
        let _ = crate::zapexec::call(Request::BackupDelete {
            path: path.clone(),
            backup_root: backup_root.to_string(),
        })
        .await;
        let _ = sqlx::query("DELETE FROM backup_records WHERE id=?")
            .bind(rid)
            .execute(pool)
            .await;
    }
    let _ = keep_path;
}

// ── Handlers ──────────────────────────────────────────────────────

/// POST /system/backup/create_dir —— 打包目录/文件。
pub async fn create_dir(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<BackupDirPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ZapError::New(-1, "归档名不能为空".to_string()));
    }
    if payload.paths.is_empty() {
        return Err(ZapError::New(-1, "待备份路径不能为空".to_string()));
    }
    let target =
        serde_json::to_string(&DirTarget { paths: payload.paths, as_user: payload.as_user })
            .map_err(|e| ZapError::New(-1, format!("参数序列化失败: {e}")))?;
    let data = run_backup("dir", &target, &name, &payload.dest_dir, None).await?;
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "backup_dir",
        &name,
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "备份完成", "data": data })))
}

/// POST /system/backup/create_db —— 导出数据库。
pub async fn create_db(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<BackupDbPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ZapError::New(-1, "归档名不能为空".to_string()));
    }
    let target = serde_json::to_string(&DbTarget {
        engine: payload.engine,
        db_name: payload.db_name,
        user: payload.user,
        password: payload.password,
        host: payload.host,
        port: payload.port,
        socket: None,
        db_path: payload.db_path,
    })
    .map_err(|e| ZapError::New(-1, format!("参数序列化失败: {e}")))?;
    let data = run_backup("db", &target, &name, &payload.dest_dir, None).await?;
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "backup_db",
        &name,
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "数据库导出完成", "data": data })))
}

/// POST /system/backup/db_quick —— 按库名一键导出（复用面板 zapadm 凭据 + 本机 socket）。
///
/// 数据库列表页的「备份」按钮调用：避免把 MySQL 密码暴露到前端，连接身份与面板自身
/// 一致（`zapadm`@localhost，经 socket 直连最稳）。
pub async fn db_quick(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<DbQuickPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ZapError::New(-1, "数据库名不能为空".to_string()));
    }
    let pwd = zap_crypto::read_cred("mysql", "zapadm").map_err(|e| {
        ZapError::New(-1, format!("读取数据库凭据失败：{e}（请确认面板能连上 MySQL）"))
    })?;
    let socket = crate::routers::database::socket_path().await;
    let target = serde_json::to_string(&DbTarget {
        engine: "mysql".to_string(),
        db_name: name.clone(),
        user: "zapadm".to_string(),
        password: pwd,
        host: "127.0.0.1".to_string(),
        port: 3306,
        socket,
        db_path: None,
    })
    .map_err(|e| ZapError::New(-1, format!("参数序列化失败: {e}")))?;
    let data = run_backup("db", &target, &name, "", None).await?;
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "backup_db_quick",
        &name,
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "数据库备份完成", "data": data })))
}

/// GET /system/backup/list —— 列出备份目录下的归档。
pub async fn list(
    claims: ValidatedClaims,
    Query(query): Query<BackupListQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let root = backup_root_path();
    let dir = if query.dir.trim().is_empty() {
        root.clone()
    } else {
        query.dir.trim().to_string()
    };
    let resp = crate::zapexec::call(Request::BackupList {
        dir,
        backup_root: root,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(-1, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": "ok", "data": resp.data })))
}

/// POST /system/backup/delete —— 删除归档。
pub async fn delete(
    claims: ValidatedClaims,
    Json(payload): Json<BackupDeletePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if payload.path.trim().is_empty() {
        return Err(ZapError::New(-1, "路径不能为空".to_string()));
    }
    let resp = crate::zapexec::call(Request::BackupDelete {
        path: payload.path.trim().to_string(),
        backup_root: backup_root_path(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(-1, resp.message));
    }
    let pool = crate::db::get_db_pool().await;
    let _ = sqlx::query("DELETE FROM backup_records WHERE path=?")
        .bind(&payload.path)
        .execute(pool)
        .await;
    Ok(Json(json!({ "code": 0, "message": "已删除" })))
}

/// POST /system/backup/restore_dir —— 还原目录。
pub async fn restore_dir(
    claims: ValidatedClaims,
    Json(payload): Json<BackupRestoreDirPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if payload.path.trim().is_empty() || payload.target_dir.trim().is_empty() {
        return Err(ZapError::New(-1, "路径与目标目录不能为空".to_string()));
    }
    let resp = crate::zapexec::call(Request::BackupRestoreDir {
        path: payload.path.trim().to_string(),
        target_dir: payload.target_dir.trim().to_string(),
        as_user: None,
        skip_owner_check: false,
        backup_root: backup_root_path(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(-1, resp.message));
    }
    let _ = audit::log(
        Some(&claims),
        None,
        "backup_restore_dir",
        &payload.path,
        &payload.target_dir,
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "目录还原完成" })))
}

/// POST /system/backup/restore_db —— 还原数据库。
pub async fn restore_db(
    claims: ValidatedClaims,
    Json(payload): Json<BackupRestoreDbPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if payload.path.trim().is_empty() {
        return Err(ZapError::New(-1, "归档路径不能为空".to_string()));
    }
    let audit_path = payload.path.clone();
    let audit_db = payload.db_name.clone();
    let resp = crate::zapexec::call(Request::BackupRestoreDb {
        path: payload.path.trim().to_string(),
        engine: payload.engine,
        db_name: payload.db_name,
        user: payload.user,
        password: payload.password,
        host: payload.host,
        port: payload.port,
        db_path: payload.db_path,
        backup_root: backup_root_path(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(-1, resp.message));
    }
    let _ = audit::log(
        Some(&claims),
        None,
        "backup_restore_db",
        &audit_path,
        &audit_db,
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "数据库还原完成" })))
}

/// GET /system/backup/jobs —— 备份任务列表。
pub async fn jobs_list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let pool = crate::db::get_db_pool().await;
    let rows: Vec<BackupJobRow> = sqlx::query_as(
        "SELECT id, name, target_type, target, schedule, enabled, retain_count, cloud_id, \
         last_run_at, last_status, last_message FROM backup_jobs ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ZapError::New(-1, format!("读任务失败: {e}")))?;
    Ok(Json(json!({ "code": 0, "message": "ok", "data": { "jobs": rows } })))
}

/// GET /system/backup/records —— 备份历史记录列表。
pub async fn records_list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let pool = crate::db::get_db_pool().await;
    let rows: Vec<Value> = sqlx::query(
        "SELECT id, job_id, kind, name, path, size, status, message, created_at \
         FROM backup_records ORDER BY id DESC LIMIT 500",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ZapError::New(-1, format!("读记录失败: {e}")))?
    .into_iter()
    .map(|r: sqlx::sqlite::SqliteRow| {
        json!({
            "id": r.get::<i64, _>("id"),
            "job_id": r.get::<i64, _>("job_id"),
            "kind": r.get::<String, _>("kind"),
            "name": r.get::<String, _>("name"),
            "path": r.get::<String, _>("path"),
            "size": r.get::<i64, _>("size"),
            "status": r.get::<i64, _>("status"),
            "message": r.get::<String, _>("message"),
            "created_at": r.get::<i64, _>("created_at"),
        })
    })
    .collect();
    Ok(Json(json!({ "code": 0, "message": "ok", "data": { "records": rows } })))
}

/// GET /system/backup/setting —— 读取备份存储目录与磁盘可用空间。
pub async fn setting_get(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let root = backup_root_path();
    let (disk_free, disk_total) = match crate::zapexec::call(Request::BackupDisk { dir: root.clone() }).await {
        Ok(resp) if resp.code == 0 => {
            let d = resp.data.unwrap_or(Value::Null);
            (
                d["disk_free"].as_i64().unwrap_or(0),
                d["disk_total"].as_i64().unwrap_or(0),
            )
        }
        _ => (0, 0),
    };
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": { "path": root, "disk_free": disk_free, "disk_total": disk_total },
    })))
}

/// POST /system/backup/setting —— 修改备份存储目录（落盘到 zap.yaml）。
pub async fn setting_save(
    claims: ValidatedClaims,
    Json(payload): Json<BackupSettingPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let p = payload.path.trim().to_string();
    if !p.is_empty() && !std::path::Path::new(&p).is_absolute() {
        return Err(ZapError::New(-1, "备份目录必须是绝对路径".to_string()));
    }
    crate::config::mutate_config(|c| {
        c.backup.backup_dir = p.clone();
    })
    .map_err(|e| ZapError::New(-1, format!("保存配置失败: {e}")))?;
    Ok(Json(json!({ "code": 0, "message": "已保存，后续备份将写入新目录" })))
}

/// POST /system/backup/job/save —— 新建 / 更新备份任务。
pub async fn job_save(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<BackupJobPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ZapError::New(-1, "任务名不能为空".to_string()));
    }
    if !matches!(payload.target_type.as_str(), "dir" | "db") {
        return Err(ZapError::New(-1, "target_type 仅支持 dir / db".to_string()));
    }
    if payload.target.trim().is_empty() {
        return Err(ZapError::New(-1, "备份目标不能为空".to_string()));
    }
    let ts = now_ts();
    let pool = crate::db::get_db_pool().await;
    let id = match payload.id {
        Some(id) if id > 0 => {
            sqlx::query(
                "UPDATE backup_jobs SET name=?, target_type=?, target=?, schedule=?, \
                 enabled=?, retain_count=?, cloud_id=?, updated_at=? WHERE id=?",
            )
            .bind(&name)
            .bind(&payload.target_type)
            .bind(&payload.target)
            .bind(&payload.schedule)
            .bind(payload.enabled)
            .bind(payload.retain_count)
            .bind(&payload.cloud_id)
            .bind(ts)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| ZapError::New(-1, format!("更新任务失败: {e}")))?;
            id
        }
        _ => {
            let rec = sqlx::query(
                "INSERT INTO backup_jobs (name, target_type, target, schedule, enabled, \
                 retain_count, cloud_id, created_at, updated_at) \
                 VALUES (?,?,?,?,?,?,?,?,?)",
            )
            .bind(&name)
            .bind(&payload.target_type)
            .bind(&payload.target)
            .bind(&payload.schedule)
            .bind(payload.enabled)
            .bind(payload.retain_count)
            .bind(&payload.cloud_id)
            .bind(ts)
            .bind(ts)
            .execute(pool)
            .await
            .map_err(|e| ZapError::New(-1, format!("创建任务失败: {e}")))?;
            rec.last_insert_rowid()
        }
    };
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "backup_job_save",
        &name,
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "已保存", "data": { "id": id } })))
}

/// POST /system/backup/job/delete —— 删除任务（同时清其历史记录）。
pub async fn job_delete(
    claims: ValidatedClaims,
    Json(payload): Json<BackupJobRunPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let pool = crate::db::get_db_pool().await;
    let _ = sqlx::query("DELETE FROM backup_records WHERE job_id=?")
        .bind(payload.id)
        .execute(pool)
        .await;
    sqlx::query("DELETE FROM backup_jobs WHERE id=?")
        .bind(payload.id)
        .execute(pool)
        .await
        .map_err(|e| ZapError::New(-1, format!("删除任务失败: {e}")))?;
    Ok(Json(json!({ "code": 0, "message": "已删除" })))
}

/// POST /system/backup/job/run —— 立即执行一次任务。
pub async fn job_run(
    claims: ValidatedClaims,
    Json(payload): Json<BackupJobRunPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT name, target_type, target FROM backup_jobs WHERE id=?",
    )
    .bind(payload.id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ZapError::New(-1, format!("读任务失败: {e}")))?;
    let (name, target_type, target) = match row {
        Some(r) => r,
        None => return Err(ZapError::New(-1, "任务不存在".to_string())),
    };
    let data = run_backup(&target_type, &target, &name, "", Some(payload.id)).await?;
    // 同步任务状态
    let (status, msg) = (1i64, "");
    let _ = sqlx::query(
        "UPDATE backup_jobs SET last_run_at=?, last_status=?, last_message=? WHERE id=?",
    )
    .bind(now_ts())
    .bind(status)
    .bind(msg)
    .bind(payload.id)
    .execute(pool)
    .await;
    Ok(Json(json!({ "code": 0, "message": "执行完成", "data": data })))
}
