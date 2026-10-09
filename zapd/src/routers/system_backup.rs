// SPDX-License-Identifier: AGPL-3.0-only
//! 备份中心（"备份"）：目录 / 数据库的一次性备份与还原，以及可定时执行的备份任务。
//!
//! 所有真正的文件/数据库操作都经 `zapexec`（root）完成——备份目录属主为 root，
//! zapd 以非 root 运行、无权直接读写，故列清单 / 删除 / 还原也走动词。
//!
//! 云存储自动上传（文件管理里已配置的云存储）作为后续能力，任务表已预留 `cloud_id`
//! 字段；本阶段 `cloud_id` 仅记录、暂不触发上传。

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{Extension, Query};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
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
    let dir = crate::config::get_config().read().ok().and_then(|c| {
        let d = c.backup.backup_dir.trim().to_string();
        if d.is_empty() { None } else { Some(d) }
    });
    dir.unwrap_or_else(default_backup_dir)
}

// ── 请求体 ────────────────────────────────────────────────────────

/// 备份策略全局 KV 读取（缺省返回空串）。
async fn gs_get(key: &str) -> String {
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM global_settings WHERE key=?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    row.map(|r| r.0).unwrap_or_default()
}

/// 备份策略全局 KV 写入（upsert）。
async fn gs_set(key: &str, value: &str) {
    let pool = crate::db::get_db_pool().await;
    let _ = sqlx::query(
        "INSERT INTO global_settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await;
}

/// 是否允许用户自助备份（默认开；管理员可在「备份策略」里关闭）。
async fn policy_allow_user_backup() -> bool {
    gs_get("backup_allow_user").await != "0"
}

/// 是否允许用户创建定时备份任务（默认开；管理员可在「备份策略」里关闭）。
async fn policy_allow_user_job() -> bool {
    gs_get("backup_allow_job").await != "0"
}

/// 全局保留份数（管理员全量备份 / 未设个人保留时采用）。
async fn policy_global_retain() -> i64 {
    gs_get("backup_global_retain")
        .await
        .parse::<i64>()
        .unwrap_or(7)
        .max(0)
}

/// 某用户的个人备份保留份数：个人设置优先，否则全局，兜底 7。
async fn user_retention(username: &str) -> i64 {
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String,)> = sqlx::query_as("SELECT prefs FROM user WHERE username=?")
        .bind(username)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    if let Some((prefs,)) = row
        && let Ok(v) = serde_json::from_str::<Value>(&prefs)
        && let Some(n) = v.get("backup_retain").and_then(|x| x.as_i64())
        && n > 0
    {
        return n;
    }
    policy_global_retain().await
}

/// 管理员通用排除清单（换行/逗号分隔）。空 KV 时回落到内置默认，覆盖最常见的缓存与
/// 递归陷阱（home 备份必须排除 `backups`/`.zap`，否则会打包自己的备份目录）。
fn default_exclude_list() -> Vec<String> {
    vec![
        ".cache".into(),
        ".npm".into(),
        ".composer".into(),
        "tmp".into(),
        "logs".into(),
        "*.log".into(),
        "backups".into(),
        ".zap".into(),
        ".git".into(),
        "node_modules".into(),
        "vendor".into(),
    ]
}

/// 管理员通用排除清单（KV `backup_exclude_default`，留空用内置默认）。
async fn policy_exclude_default() -> Vec<String> {
    let raw = gs_get("backup_exclude_default").await;
    if raw.trim().is_empty() {
        return default_exclude_list();
    }
    raw.split(['\n', ',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 全量备份模式：`home` = 按家目录；`site` = 按站点+应用+库；`both` = 两者都跑。
async fn policy_backup_mode() -> String {
    let m = gs_get("backup_mode").await;
    if matches!(m.as_str(), "home" | "site" | "both") {
        m
    } else {
        "home".to_string()
    }
}

/// 全量备份落盘位置：`home` = 各用户 `<home>/backups`（自己管理/还原）；
/// `system` = 系统备份根（集中、管理员可见）。
async fn policy_backup_dest() -> String {
    let d = gs_get("backup_dest").await;
    if matches!(d.as_str(), "home" | "system") {
        d
    } else {
        "system".to_string()
    }
}

/// 用户的自定义排除文件（root 读取）：`<home>/.zap/backup_exclude.txt`。
fn user_exclude_file(home: &str) -> Option<String> {
    let f = format!("{}/.zap/backup_exclude.txt", home.trim_end_matches('/'));
    Some(f)
}

/// 额外备份目录（策略一站点附加目录 / 策略二用户家目录外路径），统一从此表读。
async fn extra_paths(owner_type: &str, owner_id: i64) -> Vec<String> {
    let pool = crate::db::get_db_pool().await;
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT path FROM backup_paths WHERE owner_type=? AND owner_id=? AND path<>''",
    )
    .bind(owner_type)
    .bind(owner_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter().map(|r| r.0).collect()
}

/// 计算某次全量备份的落盘根与归档属主：dest=home 且给了家目录 → 写 `<home>/backups`、
/// 归档 chown 给该用户；否则写系统备份根（root 属主）。
fn resolve_full_root(dest: &str, home: &str, owner: &str) -> (String, Option<String>) {
    if dest == "home" && !home.trim().is_empty() {
        (
            format!("{}/backups", home.trim_end_matches('/')),
            Some(owner.to_string()),
        )
    } else {
        (backup_root_path(), None)
    }
}
async fn user_backup_root(user_id: i64) -> Result<(String, String), ZapError> {
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT username, home_dir FROM user WHERE id=?")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    let Some((username, home)) = row else {
        return Err(ZapError::New(-1, "用户不存在".to_string()));
    };
    let home = home.trim();
    if home.is_empty() {
        return Err(ZapError::New(
            -1,
            format!("用户 {username} 的家目录尚未初始化"),
        ));
    }
    Ok((username, format!("{}/backups", home.trim_end_matches('/'))))
}

/// 取用户名对应的家目录（用于把用户任务归档落到其家目录 backups）。
async fn home_of_username(username: &str) -> Result<String, ZapError> {
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String,)> = sqlx::query_as("SELECT home_dir FROM user WHERE username=?")
        .bind(username)
        .fetch_optional(pool)
        .await
        .map_err(|e| ZapError::New(-1, format!("读用户家目录失败: {e}")))?;
    row.map(|r| r.0)
        .filter(|h| !h.trim().is_empty())
        .ok_or_else(|| ZapError::New(-1, format!("用户 {username} 家目录尚未初始化")))
}

/// 按任务 owner 解析归档落盘根：owner 为空（管理员任务）= 系统备份根；
/// 否则取该用户家目录 backups（遵从「落盘位置」策略）。调度器与手动执行共用。
pub async fn resolve_job_root(owner: &str) -> Result<(String, Option<String>), ZapError> {
    if owner.trim().is_empty() {
        return Ok((backup_root_path(), None));
    }
    let home = home_of_username(owner).await?;
    Ok(resolve_full_root(&policy_backup_dest().await, &home, owner))
}

/// 判断 path 是否落在 root 目录内（含 root 自身），用于限制普通用户只能备份自己家目录。
fn path_within(path: &str, root: &str) -> bool {
    let Ok(cp) = std::fs::canonicalize(path) else {
        return false;
    };
    let Ok(cr) = std::fs::canonicalize(root) else {
        return false;
    };
    cp.starts_with(cr)
}

/// 库名是否带有「已存在用户」的前缀（用于区分有主库 / 无前缀库）。
async fn db_has_user_prefix(db_name: &str) -> bool {
    let Some(prefix) = db_name.split('_').next() else {
        return false;
    };
    if prefix.is_empty() {
        return false;
    }
    let pool = crate::db::get_db_pool().await;
    sqlx::query_as::<_, (String,)>("SELECT username FROM user WHERE username=?")
        .bind(prefix)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .is_some()
}

/// 按库名前缀（登录名 + '_'）解析归属用户名，并返回其家目录备份根。
///
/// 库名前缀本身就是一个已存在用户 → 归该用户（`<home>/backups`）；
/// 无前缀 / 前缀查不到对应用户（典型是管理员自建的无前缀库）→ 归 `operator`
/// （执行备份的管理员）家目录 `backups/`。这样管理员备份无前缀库不会再因
/// 「找不到用户」而失败；仅当 `operator` 自身家目录也未初始化才报错。
async fn db_owner_root(db_name: &str, operator: &str) -> Result<(String, String), ZapError> {
    let pool = crate::db::get_db_pool().await;
    // 前缀本身即可作为候选用户，再补一个 operator 兜底
    let prefix = db_name.split('_').next().unwrap_or("").to_string();
    let candidates: Vec<&str> = if prefix.is_empty() {
        vec![operator]
    } else {
        vec![prefix.as_str(), operator]
    };
    for user in candidates.into_iter().filter(|u| !u.is_empty()) {
        if let Some((home,)) =
            sqlx::query_as::<_, (String,)>("SELECT home_dir FROM user WHERE username=?")
                .bind(user)
                .fetch_optional(pool)
                .await?
        {
            let home = home.trim();
            if !home.is_empty() {
                return Ok((
                    user.to_string(),
                    format!("{}/backups", home.trim_end_matches('/')),
                ));
            }
        }
    }
    Err(ZapError::New(
        -1,
        format!("无前缀库 {db_name} 无法归属：操作者 {operator} 的家目录尚未初始化"),
    ))
}

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
    // 还原到原路径（to_original=true）时前端不传此字段，给默认值避免反序列化失败 422
    #[serde(default)]
    pub target_dir: String,
    #[serde(default)]
    pub to_original: bool,
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
    owner: String,
    last_run_at: i64,
    last_status: i64,
    last_message: String,
}

// ── 核心：执行一次备份（手动 / 定时共用）───────────────────────────

/// 执行一次备份：写记录 → 调 zapexec → 更新记录；定时任务（job_id 有值）会按
/// `retain_count` 清理旧归档。返回 `{ record_id, path, size, data }`。
pub(crate) struct RunBackupArgs<'a> {
    pub(crate) target_type: &'a str,
    pub(crate) target_json: &'a str,
    pub(crate) name: &'a str,
    pub(crate) dest_dir: &'a str,
    pub(crate) job_id: Option<i64>,
    pub(crate) owner: &'a str,
    pub(crate) dest_root: Option<&'a str>,
    pub(crate) excludes: &'a [String],
    pub(crate) exclude_file: Option<&'a str>,
    pub(crate) manifest: &'a [String],
}

pub async fn run_backup<'a>(a: RunBackupArgs<'a>) -> Result<Value, ZapError> {
    let RunBackupArgs {
        target_type,
        target_json,
        name,
        dest_dir,
        job_id,
        owner,
        dest_root,
        excludes,
        exclude_file,
        manifest,
    } = a;
    let ts = now_ts();
    // 定时任务给归档名加时间戳，避免覆盖上一次；手动用原名。
    let eff_name = match job_id {
        Some(_) => format!("{name}-{ts}"),
        None => name.to_string(),
    };

    let pool = crate::db::get_db_pool().await;
    let system_root = backup_root_path();
    // dest_root 有值（用户家目录 backups，绝对路径）= 用户自助备份；否则落系统备份根。
    let eff_root = dest_root.unwrap_or(&system_root).to_string();
    let manifest_str = serde_json::to_string(manifest).unwrap_or_else(|_| "[]".to_string());
    let rec = sqlx::query(
        "INSERT INTO backup_records (job_id, kind, name, status, created_at, owner, dest_root, manifest) \
         VALUES (?,?,?,0,?,?,?,?)",
    )
    .bind(job_id.unwrap_or(0))
    .bind(target_type)
    .bind(&eff_name)
    .bind(ts)
    .bind(owner)
    .bind(dest_root.unwrap_or(""))
    .bind(&manifest_str)
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
                backup_root: eff_root.clone(),
                exclude: excludes.to_vec(),
                exclude_file: exclude_file.map(|s| s.to_string()),
            })
            .await?
        }
        "db" => {
            let t: DbTarget = serde_json::from_str(target_json)
                .map_err(|e| ZapError::New(-1, format!("target 解析失败: {e}")))?;
            // 留空的连接信息默认用面板 zapadm 本机直连；填写则作为远程数据库备份。
            let user = if t.user.trim().is_empty() {
                "zapadm".to_string()
            } else {
                t.user.clone()
            };
            let host = if t.host.trim().is_empty() {
                "127.0.0.1".to_string()
            } else {
                t.host.clone()
            };
            let port = if t.port <= 0 { 3306 } else { t.port };
            let password = if t.engine == "mysql" && t.password.trim().is_empty() {
                zap_crypto::read_cred("mysql", "zapadm").unwrap_or_default()
            } else {
                t.password.clone()
            };
            let socket = match t.socket.clone() {
                Some(s) if !s.trim().is_empty() => Some(s),
                _ => crate::routers::database::socket_path().await,
            };
            crate::zapexec::call(Request::BackupDb {
                name: eff_name.clone(),
                engine: t.engine,
                db_name: t.db_name,
                user,
                password,
                host,
                port,
                socket,
                dest_dir: dest_dir.to_string(),
                db_path: t.db_path,
                backup_root: eff_root.clone(),
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

    if let Some(jid) = job_id
        && let Ok(retain) = job_retain(jid).await
        && retain > 0
        && !bpath.is_empty()
    {
        prune_old(jid, retain, &bpath, &eff_root).await;
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

/// 按归属用户清理旧归档，保留最新 `keep` 份（用户自助备份的保留策略）。
async fn prune_by_owner(owner: &str, keep: i64, root: &str) {
    if keep <= 0 {
        return;
    }
    let pool = crate::db::get_db_pool().await;
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, path FROM backup_records WHERE owner=? AND status=1 AND path<>'' \
         ORDER BY id DESC",
    )
    .bind(owner)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (rid, path) in rows.into_iter().skip(keep as usize) {
        let _ = crate::zapexec::call(Request::BackupDelete {
            path: path.clone(),
            backup_root: root.to_string(),
        })
        .await;
        let _ = sqlx::query("DELETE FROM backup_records WHERE id=?")
            .bind(rid)
            .execute(pool)
            .await;
    }
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
    let target = serde_json::to_string(&DirTarget {
        paths: payload.paths,
        as_user: payload.as_user,
    })
    .map_err(|e| ZapError::New(-1, format!("参数序列化失败: {e}")))?;
    let data = run_backup(RunBackupArgs {
        target_type: "dir",
        target_json: &target,
        name: &name,
        dest_dir: &payload.dest_dir,
        job_id: None,
        owner: "",
        dest_root: None,
        excludes: &[],
        exclude_file: None,
        manifest: &[],
    })
    .await?;
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "backup_dir",
        &name,
        "",
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "备份完成", "data": data }),
    ))
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
    let data = run_backup(RunBackupArgs {
        target_type: "db",
        target_json: &target,
        name: &name,
        dest_dir: &payload.dest_dir,
        job_id: None,
        owner: "",
        dest_root: None,
        excludes: &[],
        exclude_file: None,
        manifest: &[],
    })
    .await?;
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "backup_db",
        &name,
        "",
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "数据库导出完成", "data": data }),
    ))
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
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ZapError::New(-1, "数据库名不能为空".to_string()));
    }
    // 权限：管理员可备份任意库；普通会员 / reseller 仅能备份自己名下的库
    // （库名前缀 = 登录名，与数据库列表的可见范围 prefix 一致）；
    // 管理员可统一关闭「用户自助备份」。
    if !is_admin(&claims) {
        if !policy_allow_user_backup().await {
            return Err(ZapError::New(-1, "管理员已关闭用户自助备份".to_string()));
        }
        if !name.starts_with(&format!("{}_", claims.sub.trim())) {
            return Err(ZapError::New(-1, "只能备份自己的数据库".to_string()));
        }
    }
    // 归属：库名前缀对应用户 → 该用户；无前缀（管理员自建库）→ 操作者。
    let (owner, root) = db_owner_root(&name, &claims.sub).await?;
    let home = root
        .trim_end_matches("/backups")
        .trim_end_matches('/')
        .to_string();
    // 无前缀库（管理员自建）统一写系统备份根；有前缀的用户库仍服从全局
    // 「落盘位置」策略（home=家目录 / system=系统目录），与全量备份一致。
    let eff_root = if db_has_user_prefix(&name).await {
        resolve_full_root(&policy_backup_dest().await, &home, &owner).0
    } else {
        backup_root_path()
    };
    let retain = user_retention(&owner).await;
    let pwd = zap_crypto::read_cred("mysql", "zapadm").map_err(|e| {
        ZapError::New(
            -1,
            format!("读取数据库凭据失败：{e}（请确认面板能连上 MySQL）"),
        )
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
    let home = root.trim_end_matches("/backups").trim_end_matches('/');
    let excludes = policy_exclude_default().await;
    let exf = user_exclude_file(home);
    let data = run_backup(RunBackupArgs {
        target_type: "db",
        target_json: &target,
        name: &name,
        dest_dir: "",
        job_id: None,
        owner: &owner,
        dest_root: Some(&eff_root),
        excludes: &excludes,
        exclude_file: exf.as_deref(),
        manifest: std::slice::from_ref(&name),
    })
    .await?;
    // 按用户个人保留份数清理旧归档
    prune_by_owner(&owner, retain, &root).await;
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "backup_db_quick",
        &name,
        "",
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "数据库备份完成", "data": data }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct SiteQuickPayload {
    /// 站点 id（后端据此校验归属并解析文档根，不信任前端传入的路径）
    pub id: i64,
}

/// POST /system/backup/site_quick —— 按站点 id 一键备份文档根目录。
///
/// 站点管理列表的「备份」按钮调用：权限与站点管理一致（管理员 / reseller / 本人 / 同团队，
/// 复用 `site_in_scope`），避免普通会员越权备份他人站点；文档根从数据库解析，防止前端
/// 伪造路径打包任意目录。
pub async fn site_quick(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<SiteQuickPayload>,
) -> ZapJsonResult {
    // 归属校验：仅站点主人 / 其 reseller / 管理员 / 同团队成员可备份
    crate::routers::site::site_in_scope(&claims, payload.id).await?;
    // 管理员可统一关闭「用户自助备份」
    if !is_admin(&claims) && !policy_allow_user_backup().await {
        return Err(ZapError::New(-1, "管理员已关闭用户自助备份".to_string()));
    }
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String, String, i64)> =
        sqlx::query_as("SELECT name, web_root, user_id FROM site WHERE id = ?")
            .bind(payload.id)
            .fetch_optional(pool)
            .await?;
    let Some((name, web_root, uid)) = row else {
        return Err(ZapError::New(-1, "站点不存在".to_string()));
    };
    let web_root = web_root.trim();
    if web_root.is_empty() {
        return Err(ZapError::New(
            -1,
            "该站点无文档根目录（反向代理类站点），无可备份内容".to_string(),
        ));
    }
    // 站点归属到其主人的家目录 backups，并取该用户的保留份数
    let (owner, root) = user_backup_root(uid).await?;
    let retain = user_retention(&owner).await;
    let home = root.trim_end_matches("/backups").trim_end_matches('/');
    let arc_name = if name.trim().is_empty() {
        format!("site-{}", payload.id)
    } else {
        name.trim().to_string()
    };
    // 策略一（手动版）：站点文档根 + 应用工作目录 + 客户追加目录，一并打包
    let pool = crate::db::get_db_pool().await;
    let mut paths: Vec<String> = vec![web_root.to_string()];
    let apps: Vec<(String,)> =
        sqlx::query_as("SELECT workdir FROM site_apps WHERE site_id=? AND workdir<>''")
            .bind(payload.id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    for (w,) in apps {
        paths.push(w);
    }
    let extra = extra_paths("site", payload.id).await;
    paths.extend(extra);
    let excludes = policy_exclude_default().await;
    let exf = user_exclude_file(home);
    let target = serde_json::to_string(&DirTarget {
        paths: paths.clone(),
        as_user: None,
    })
    .map_err(|e| ZapError::New(-1, format!("参数序列化失败: {e}")))?;
    let data = run_backup(RunBackupArgs {
        target_type: "dir",
        target_json: &target,
        name: &arc_name,
        dest_dir: "",
        job_id: None,
        owner: &owner,
        dest_root: Some(&root),
        excludes: &excludes,
        exclude_file: exf.as_deref(),
        manifest: &paths,
    })
    .await?;
    // 按用户个人保留份数清理旧归档
    prune_by_owner(&owner, retain, &root).await;
    let _ = audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "backup_site_quick",
        &arc_name,
        "",
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "站点目录备份完成", "data": data }),
    ))
}

/// GET /system/backup/list —— 列出备份目录下的归档。
pub async fn list(claims: ValidatedClaims, Query(query): Query<BackupListQuery>) -> ZapJsonResult {
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
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": resp.data }),
    ))
}

/// POST /system/backup/delete —— 删除归档。
pub async fn delete(
    claims: ValidatedClaims,
    Json(payload): Json<BackupDeletePayload>,
) -> ZapJsonResult {
    if payload.path.trim().is_empty() {
        return Err(ZapError::New(-1, "路径不能为空".to_string()));
    }
    let pool = crate::db::get_db_pool().await;
    // 归属校验：管理员可删任意；用户仅能删自己（owner = 登录名）的备份
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT owner, dest_root FROM backup_records WHERE path=?")
            .bind(&payload.path)
            .fetch_optional(pool)
            .await?;
    let root = match row {
        Some((owner, dest_root)) => {
            if !is_admin(&claims) && owner != claims.sub {
                return Err(ZapError::New(-1, "无权操作该备份".to_string()));
            }
            if dest_root.trim().is_empty() {
                backup_root_path()
            } else {
                dest_root
            }
        }
        None => {
            // 老数据（无 owner 记录）仅管理员可删
            if !is_admin(&claims) {
                return Err(ZapError::New(-1, "无权操作该备份".to_string()));
            }
            backup_root_path()
        }
    };
    let resp = crate::zapexec::call(Request::BackupDelete {
        path: payload.path.trim().to_string(),
        backup_root: root,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(-1, resp.message));
    }
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
    if payload.path.trim().is_empty() {
        return Err(ZapError::New(-1, "路径不能为空".to_string()));
    }
    if !payload.to_original && payload.target_dir.trim().is_empty() {
        return Err(ZapError::New(-1, "解包目标目录不能为空".to_string()));
    }
    let pool = crate::db::get_db_pool().await;
    // 归属校验：管理员可还原任意；用户仅能还原自己（owner = 登录名）的备份，
    // 无论该归档位于自家 home/backups 还是系统备份根（管理员全量备份的副本）。
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT owner, dest_root FROM backup_records WHERE path=?")
            .bind(&payload.path)
            .fetch_optional(pool)
            .await?;
    let root = match row {
        Some((owner, dest_root)) => {
            if !is_admin(&claims) && owner != claims.sub {
                return Err(ZapError::New(-1, "无权操作该备份".to_string()));
            }
            if dest_root.trim().is_empty() {
                backup_root_path()
            } else {
                dest_root
            }
        }
        None => {
            if !is_admin(&claims) {
                return Err(ZapError::New(-1, "无权操作该备份".to_string()));
            }
            backup_root_path()
        }
    };
    let resp = crate::zapexec::call(Request::BackupRestoreDir {
        path: payload.path.trim().to_string(),
        target_dir: payload.target_dir.trim().to_string(),
        to_original: payload.to_original,
        as_user: None,
        skip_owner_check: false,
        backup_root: root,
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
        &format!("to_original={}", payload.to_original),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "目录还原完成" })))
}

/// POST /system/backup/restore_db —— 还原数据库。
pub async fn restore_db(
    claims: ValidatedClaims,
    Json(payload): Json<BackupRestoreDbPayload>,
) -> ZapJsonResult {
    if payload.path.trim().is_empty() {
        return Err(ZapError::New(-1, "归档路径不能为空".to_string()));
    }
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT owner, dest_root FROM backup_records WHERE path=?")
            .bind(&payload.path)
            .fetch_optional(pool)
            .await?;
    let root = match row {
        Some((owner, dest_root)) => {
            if !is_admin(&claims) && owner != claims.sub {
                return Err(ZapError::New(-1, "无权操作该备份".to_string()));
            }
            if dest_root.trim().is_empty() {
                backup_root_path()
            } else {
                dest_root
            }
        }
        None => {
            if !is_admin(&claims) {
                return Err(ZapError::New(-1, "无权操作该备份".to_string()));
            }
            backup_root_path()
        }
    };
    // 还原走面板 zapadm 凭据，用户无需输入密码（与备份一致）
    let (user, password) = if payload.password.trim().is_empty() {
        let pwd = zap_crypto::read_cred("mysql", "zapadm")
            .map_err(|e| ZapError::New(-1, format!("读取数据库凭据失败：{e}")))?;
        ("zapadm".to_string(), pwd)
    } else {
        (payload.user.clone(), payload.password.clone())
    };
    let audit_path = payload.path.clone();
    let audit_db = payload.db_name.clone();
    let resp = crate::zapexec::call(Request::BackupRestoreDb {
        path: payload.path.trim().to_string(),
        engine: payload.engine,
        db_name: payload.db_name,
        user,
        password,
        host: payload.host,
        port: payload.port,
        db_path: payload.db_path,
        backup_root: root,
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
///
/// 管理员可见全部；普通用户仅见自己（owner = 登录名）的任务，且受「允许用户定时备份」开关约束。
pub async fn jobs_list(claims: ValidatedClaims) -> ZapJsonResult {
    let admin = is_admin(&claims);
    if !admin && !policy_allow_user_job().await {
        return Err(ZapError::New(-1, "管理员已关闭用户定时备份".to_string()));
    }
    let pool = crate::db::get_db_pool().await;
    let rows: Vec<BackupJobRow> = if admin {
        sqlx::query_as(
            "SELECT id, name, target_type, target, schedule, enabled, retain_count, cloud_id, \
             owner, last_run_at, last_status, last_message FROM backup_jobs ORDER BY id DESC",
        )
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as(
            "SELECT id, name, target_type, target, schedule, enabled, retain_count, cloud_id, \
             owner, last_run_at, last_status, last_message FROM backup_jobs WHERE owner=? \
             ORDER BY id DESC",
        )
        .bind(&claims.sub)
        .fetch_all(pool)
        .await
    }
    .map_err(|e| ZapError::New(-1, format!("读任务失败: {e}")))?;
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": { "jobs": rows } }),
    ))
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
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": { "records": rows } }),
    ))
}

/// GET /system/backup/setting —— 读取备份存储目录与磁盘可用空间。
pub async fn setting_get(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let root = backup_root_path();
    let (disk_free, disk_total) =
        match crate::zapexec::call(Request::BackupDisk { dir: root.clone() }).await {
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
    Ok(Json(
        json!({ "code": 0, "message": "已保存，后续备份将写入新目录" }),
    ))
}

/// POST /system/backup/job/save —— 新建 / 更新备份任务。
///
/// 管理员：可建任意任务（含云同步 cloud_id）。
/// 普通用户：受「允许用户定时备份」开关约束；任务归属自己（owner = 登录名），
/// 且目录类只能备份家目录内路径、数据库类只能备份自己名下的库，不允许设置云同步。
pub async fn job_save(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<BackupJobPayload>,
) -> ZapJsonResult {
    let admin = is_admin(&claims);
    if !admin && !policy_allow_user_job().await {
        return Err(ZapError::New(-1, "管理员已关闭用户定时备份".to_string()));
    }
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

    let mut target = payload.target.clone();

    let owner = if admin {
        String::new()
    } else {
        claims.sub.trim().to_string()
    };

    // 普通用户：校验作用域，防止越权备份他人数据
    if !admin {
        if payload.target_type == "dir" {
            let t: DirTarget = serde_json::from_str(&target)
                .map_err(|e| ZapError::New(-1, format!("目标解析失败: {e}")))?;
            let home = home_of_username(&owner).await?;
            for p in &t.paths {
                if !path_within(p, &home) {
                    return Err(ZapError::New(
                        -1,
                        format!("只能备份自己家目录内的路径（{home}）"),
                    ));
                }
            }
            // 不允许普通用户指定以其他身份执行（as_user 强制为 None）
            target = serde_json::to_string(&DirTarget {
                paths: t.paths,
                as_user: None,
            })
            .map_err(|e| ZapError::New(-1, format!("目标序列化失败: {e}")))?;
        } else {
            let t: DbTarget = serde_json::from_str(&target)
                .map_err(|e| ZapError::New(-1, format!("目标解析失败: {e}")))?;
            if !t.db_name.starts_with(&format!("{}_", owner)) {
                return Err(ZapError::New(-1, "只能备份自己名下的数据库".to_string()));
            }
        }
    }

    let cloud_id = if admin { payload.cloud_id.as_str() } else { "" };
    let ts = now_ts();
    let pool = crate::db::get_db_pool().await;
    let id = match payload.id {
        Some(id) if id > 0 => {
            // 编辑：校验归属（普通用户只能改自己的任务）
            if !admin {
                let existing: Option<(String,)> =
                    sqlx::query_as("SELECT owner FROM backup_jobs WHERE id=?")
                        .bind(id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| ZapError::New(-1, format!("读任务失败: {e}")))?;
                match existing {
                    None => return Err(ZapError::New(-1, "任务不存在".to_string())),
                    Some((o,)) if o.as_str() != owner.as_str() => {
                        return Err(ZapError::New(-1, "只能修改自己的任务".to_string()));
                    }
                    _ => {}
                }
            }
            sqlx::query(
                "UPDATE backup_jobs SET name=?, target_type=?, target=?, schedule=?, \
                 enabled=?, retain_count=?, cloud_id=?, owner=?, updated_at=? WHERE id=?",
            )
            .bind(&name)
            .bind(&payload.target_type)
            .bind(&target)
            .bind(&payload.schedule)
            .bind(payload.enabled)
            .bind(payload.retain_count)
            .bind(cloud_id)
            .bind(&owner)
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
                 retain_count, cloud_id, owner, created_at, updated_at) \
                 VALUES (?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&name)
            .bind(&payload.target_type)
            .bind(&target)
            .bind(&payload.schedule)
            .bind(payload.enabled)
            .bind(payload.retain_count)
            .bind(cloud_id)
            .bind(&owner)
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
    Ok(Json(
        json!({ "code": 0, "message": "已保存", "data": { "id": id } }),
    ))
}

/// POST /system/backup/job/delete —— 删除任务（同时清其历史记录）。
pub async fn job_delete(
    claims: ValidatedClaims,
    Json(payload): Json<BackupJobRunPayload>,
) -> ZapJsonResult {
    let admin = is_admin(&claims);
    let pool = crate::db::get_db_pool().await;
    if !admin {
        if !policy_allow_user_job().await {
            return Err(ZapError::New(-1, "管理员已关闭用户定时备份".to_string()));
        }
        let existing: Option<(String,)> =
            sqlx::query_as("SELECT owner FROM backup_jobs WHERE id=?")
                .bind(payload.id)
                .fetch_optional(pool)
                .await
                .map_err(|e| ZapError::New(-1, format!("读任务失败: {e}")))?;
        match existing {
            None => return Err(ZapError::New(-1, "任务不存在".to_string())),
            Some((o,)) if o.as_str() != claims.sub.trim() => {
                return Err(ZapError::New(-1, "只能删除自己的任务".to_string()));
            }
            _ => {}
        }
    }
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
    let admin = is_admin(&claims);
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String, String, String, String)> =
        sqlx::query_as("SELECT name, target_type, target, owner FROM backup_jobs WHERE id=?")
            .bind(payload.id)
            .fetch_optional(pool)
            .await
            .map_err(|e| ZapError::New(-1, format!("读任务失败: {e}")))?;
    let (name, target_type, target, owner) = match row {
        Some(r) => r,
        None => return Err(ZapError::New(-1, "任务不存在".to_string())),
    };
    if !admin {
        if !policy_allow_user_job().await {
            return Err(ZapError::New(-1, "管理员已关闭用户定时备份".to_string()));
        }
        if owner.as_str() != claims.sub.trim() {
            return Err(ZapError::New(-1, "只能执行自己的任务".to_string()));
        }
        // 重新校验作用域（防止用户篡改 target 越权）
        if target_type == "dir" {
            let t: DirTarget = serde_json::from_str(&target)
                .map_err(|e| ZapError::New(-1, format!("目标解析失败: {e}")))?;
            let home = home_of_username(&owner).await?;
            for p in &t.paths {
                if !path_within(p, &home) {
                    return Err(ZapError::New(
                        -1,
                        format!("只能备份自己家目录内的路径（{home}）"),
                    ));
                }
            }
        } else {
            let t: DbTarget = serde_json::from_str(&target)
                .map_err(|e| ZapError::New(-1, format!("目标解析失败: {e}")))?;
            if !t.db_name.starts_with(&format!("{}_", owner)) {
                return Err(ZapError::New(-1, "只能备份自己名下的数据库".to_string()));
            }
        }
    }
    let (eff_root, _) = resolve_job_root(&owner).await?;
    let data = run_backup(RunBackupArgs {
        target_type: &target_type,
        target_json: &target,
        name: &name,
        dest_dir: "",
        job_id: Some(payload.id),
        owner: &owner,
        dest_root: Some(&eff_root),
        excludes: &[],
        exclude_file: None,
        manifest: &[],
    })
    .await?;
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
    Ok(Json(
        json!({ "code": 0, "message": "执行完成", "data": data }),
    ))
}

// ── 备份策略（管理员）──────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct BackupPolicyPayload {
    #[serde(default)]
    pub allow_user: Option<bool>,
    /// 是否允许普通用户创建定时备份任务（默认开）。
    #[serde(default)]
    pub allow_user_job: Option<bool>,
    #[serde(default)]
    pub global_retain: Option<i64>,
    #[serde(default)]
    pub all_enabled: Option<bool>,
    #[serde(default)]
    pub all_schedule: Option<String>,
    /// 全量备份策略：`home`=按家目录；`site`=按站点+应用+库；`both`=两者都跑。
    #[serde(default)]
    pub mode: Option<String>,
    /// 落盘位置：`home`=各用户 `<home>/backups`；`system`=系统备份根。
    #[serde(default)]
    pub dest: Option<String>,
    /// 管理员通用排除清单（换行/逗号分隔；空 = 内置默认）。
    #[serde(default)]
    pub exclude_default: Option<String>,
}

/// GET /system/backup/policy —— 读取全局备份策略。
///
/// 只读且无非敏感字段，向所有登录角色开放：
/// 普通用户在「备份任务」页需要据此判断「允许用户定时备份」开关是否开启。
pub async fn policy_get(_claims: ValidatedClaims) -> ZapJsonResult {
    let allow = policy_allow_user_backup().await;
    let allow_job = policy_allow_user_job().await;
    let retain = policy_global_retain().await;
    let all_enabled = gs_get("backup_all_enabled").await != "0";
    let all_schedule = gs_get("backup_all_schedule").await;
    let mode = policy_backup_mode().await;
    let dest = policy_backup_dest().await;
    let exclude_default = gs_get("backup_exclude_default").await;
    let report = gs_get("backup_all_report").await;
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": {
            "allow_user_backup": allow,
            "allow_user_job": allow_job,
            "global_retain": retain,
            "all_enabled": all_enabled,
            "all_schedule": all_schedule,
            "mode": mode,
            "dest": dest,
            "exclude_default": exclude_default,
            "last_report": serde_json::from_str::<Value>(&report).unwrap_or(Value::Null),
        },
    })))
}

/// POST /system/backup/policy —— 保存全局备份策略。
pub async fn policy_set(
    claims: ValidatedClaims,
    Json(payload): Json<BackupPolicyPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if let Some(v) = payload.allow_user {
        gs_set("backup_allow_user", if v { "1" } else { "0" }).await;
    }
    if let Some(v) = payload.allow_user_job {
        gs_set("backup_allow_job", if v { "1" } else { "0" }).await;
    }
    if let Some(v) = payload.global_retain
        && v >= 0
    {
        gs_set("backup_global_retain", &v.to_string()).await;
    }
    if let Some(v) = payload.all_enabled {
        gs_set("backup_all_enabled", if v { "1" } else { "0" }).await;
    }
    if let Some(v) = payload.all_schedule {
        gs_set("backup_all_schedule", v.trim()).await;
    }
    if let Some(v) = payload.mode {
        let m = v.trim().to_string();
        if matches!(m.as_str(), "home" | "site" | "both") {
            gs_set("backup_mode", &m).await;
        }
    }
    if let Some(v) = payload.dest {
        let d = v.trim().to_string();
        if matches!(d.as_str(), "home" | "system") {
            gs_set("backup_dest", &d).await;
        }
    }
    if let Some(v) = payload.exclude_default {
        gs_set("backup_exclude_default", v.trim()).await;
    }
    let _ = audit::log(Some(&claims), None, "backup_policy_set", "", "").await;
    Ok(Json(json!({ "code": 0, "message": "策略已更新" })))
}

// ── 额外备份目录（策略一站点附加目录 / 策略二用户家目录外路径）──────

#[derive(Debug, Deserialize, Default)]
pub struct BackupPathPayload {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub owner_type: String,
    #[serde(default)]
    pub owner_id: i64,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct BackupPathListQuery {
    #[serde(default)]
    pub owner_type: String,
    #[serde(default)]
    pub owner_id: i64,
}

/// GET /system/backup/paths —— 额外备份目录列表（仅管理员可见/管理）。
pub async fn paths_list(
    claims: ValidatedClaims,
    Query(q): Query<BackupPathListQuery>,
) -> ZapJsonResult {
    let pool = crate::db::get_db_pool().await;
    let rows: Vec<(i64, String, i64, String, String, i64)> = if is_admin(&claims) {
        if q.owner_type.is_empty() {
            sqlx::query_as(
                "SELECT id, owner_type, owner_id, path, note, COALESCE(created_at,0) \
                 FROM backup_paths ORDER BY id DESC",
            )
            .fetch_all(pool)
            .await
        } else {
            sqlx::query_as(
                "SELECT id, owner_type, owner_id, path, note, COALESCE(created_at,0) \
                 FROM backup_paths WHERE owner_type=? AND owner_id=? ORDER BY id DESC",
            )
            .bind(&q.owner_type)
            .bind(q.owner_id)
            .fetch_all(pool)
            .await
        }
    } else {
        let uid = claims.id as i64;
        let mut items: Vec<(i64, String, i64, String, String, i64)> = sqlx::query_as(
            "SELECT id, owner_type, owner_id, path, note, COALESCE(created_at,0) \
             FROM backup_paths WHERE owner_type='user' AND owner_id=?",
        )
        .bind(uid)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        let owned: Vec<(i64,)> = sqlx::query_as("SELECT id FROM site WHERE user_id=?")
            .bind(uid)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
        for (sid,) in owned {
            let s: Vec<(i64, String, i64, String, String, i64)> = sqlx::query_as(
                "SELECT id, owner_type, owner_id, path, note, COALESCE(created_at,0) \
                 FROM backup_paths WHERE owner_type='site' AND owner_id=?",
            )
            .bind(sid)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
            items.extend(s);
        }
        Ok(items)
    }
    .map_err(|e| ZapError::New(-1, format!("查询失败: {e}")))?;
    let data: Vec<Value> = rows
        .into_iter()
        .map(|(id, ot, oid, path, note, created)| {
            json!({"id":id,"owner_type":ot,"owner_id":oid,"path":path,"note":note,"created_at":created})
        })
        .collect();
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": { "items": data } }),
    ))
}

/// POST /system/backup/paths —— 新增额外备份目录（仅管理员；家目录之外的目录只能由管理员添加）。
pub async fn paths_add(claims: ValidatedClaims, Json(p): Json<BackupPathPayload>) -> ZapJsonResult {
    require_admin(&claims)?;
    let owner_type = p.owner_type.trim();
    if !matches!(owner_type, "site" | "user") {
        return Err(ZapError::New(
            -1,
            "owner_type 仅支持 site / user".to_string(),
        ));
    }
    let path = p.path.trim();
    if path.is_empty() || !path.starts_with('/') {
        return Err(ZapError::New(-1, "目录必须为绝对路径".to_string()));
    }
    let pool = crate::db::get_db_pool().await;
    let ts = now_ts();
    sqlx::query(
        "INSERT INTO backup_paths (owner_type, owner_id, path, note, created_at, updated_at) \
         VALUES (?,?,?,?,?,?)",
    )
    .bind(owner_type)
    .bind(p.owner_id)
    .bind(path)
    .bind(p.note.trim())
    .bind(ts)
    .bind(ts)
    .execute(pool)
    .await
    .map_err(|e| ZapError::New(-1, format!("写入失败: {e}")))?;
    Ok(Json(json!({ "code": 0, "message": "已添加" })))
}

/// DELETE /system/backup/paths —— 删除额外备份目录（仅管理员）。
pub async fn paths_delete(
    claims: ValidatedClaims,
    Json(p): Json<BackupPathPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT owner_type, owner_id FROM backup_paths WHERE id=?")
            .bind(p.id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let (ot, oid) = match row {
        Some(x) => x,
        None => return Err(ZapError::New(-1, "记录不存在".to_string())),
    };
    let _ = (ot, oid);
    sqlx::query("DELETE FROM backup_paths WHERE id=?")
        .bind(p.id)
        .execute(pool)
        .await
        .map_err(|e| ZapError::New(-1, format!("删除失败: {e}")))?;
    Ok(Json(json!({ "code": 0, "message": "已删除" })))
}

// ── 个人备份保留份数（任意登录用户）────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BackupRetainPayload {
    pub retain: i64,
}

/// GET /system/backup/my-retention —— 读取本人的备份保留份数。
pub async fn my_retention_get(claims: ValidatedClaims) -> ZapJsonResult {
    let r = user_retention(&claims.sub).await;
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": { "retain": r } }),
    ))
}

/// POST /system/backup/my-retention —— 设置本人的备份保留份数（写入 user.prefs）。
pub async fn my_retention_set(
    claims: ValidatedClaims,
    Json(payload): Json<BackupRetainPayload>,
) -> ZapJsonResult {
    let n = payload.retain.max(0);
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String,)> = sqlx::query_as("SELECT prefs FROM user WHERE username=?")
        .bind(&claims.sub)
        .fetch_optional(pool)
        .await?;
    let mut prefs: Value = row
        .and_then(|(s,)| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Object(serde_json::Map::new()));
    if let Value::Object(map) = &mut prefs {
        map.insert("backup_retain".to_string(), json!(n));
    }
    let prefs_str = serde_json::to_string(&prefs).unwrap_or_else(|_| "{}".to_string());
    sqlx::query("UPDATE user SET prefs=? WHERE username=?")
        .bind(&prefs_str)
        .bind(&claims.sub)
        .execute(pool)
        .await
        .map_err(|e| ZapError::New(-1, format!("保存失败: {e}")))?;
    Ok(Json(json!({ "code": 0, "message": "已保存" })))
}

// ── 用户自助备份列表（本人可见自己，管理员可见全部）──────────────

#[derive(Debug, sqlx::FromRow)]
struct BackupRecordRow {
    id: i64,
    kind: String,
    name: String,
    path: String,
    size: i64,
    status: i64,
    created_at: i64,
    dest_root: String,
}

/// GET /system/backup/my —— 列出当前用户能还原的备份（含自家 home/backups 与系统目录副本）。
pub async fn my_list(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = crate::db::get_db_pool().await;
    let rows: Vec<BackupRecordRow> = if is_admin(&claims) {
        sqlx::query_as(
            "SELECT id, kind, name, path, size, status, created_at, dest_root \
             FROM backup_records WHERE owner<>'' ORDER BY id DESC LIMIT 500",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| ZapError::New(-1, format!("读记录失败: {e}")))?
    } else {
        sqlx::query_as(
            "SELECT id, kind, name, path, size, status, created_at, dest_root \
             FROM backup_records WHERE owner=? ORDER BY id DESC LIMIT 500",
        )
        .bind(&claims.sub)
        .fetch_all(pool)
        .await
        .map_err(|e| ZapError::New(-1, format!("读记录失败: {e}")))?
    };
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let dest = if r.dest_root.trim().is_empty() {
                "system"
            } else {
                "home"
            };
            json!({
                "id": r.id,
                "kind": r.kind,
                "name": r.name,
                "path": r.path,
                "size": r.size,
                "status": r.status,
                "created_at": r.created_at,
                "dest": dest,
            })
        })
        .collect();
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": { "records": items } }),
    ))
}

// ── 管理员全量备份（遍历所有站点 / 数据库，按主人打标写入系统目录）──

/// POST /system/backup/all —— 立即对全量用户数据做一次备份（后台执行）。
pub async fn backup_all(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    tokio::spawn(async move {
        run_backup_all().await;
    });
    Ok(Json(
        json!({ "code": 0, "message": "已启动全量备份（后台执行）" }),
    ))
}

/// 全量备份执行报告（落 KV `backup_all_report`，供前端展示上次结果）。
#[derive(Debug, Default, Serialize)]
struct BackupAllReport {
    started_at: i64,
    finished_at: i64,
    mode: String,
    dest: String,
    ok: u64,
    fail: u64,
    failures: Vec<Value>,
}

/// 全量备份实现：按 `backup_mode`（home/site/both）遍历用户/站点；数据库单独一遍
/// （库既不在家目录也不在站点目录，故两种策略都补一份库备份；无前缀库归管理员）。
/// 每个资源按主人打标写入目标根（`<home>/backups` 或系统根），并按全局保留份数清理。
pub async fn run_backup_all() {
    let mode = policy_backup_mode().await;
    let dest = policy_backup_dest().await;
    let retain = policy_global_retain().await;
    let excludes = policy_exclude_default().await;
    let mut report = BackupAllReport {
        started_at: now_ts(),
        mode: mode.clone(),
        dest: dest.clone(),
        ..Default::default()
    };

    if matches!(mode.as_str(), "home" | "both") {
        backup_all_home(&mut report, &dest, retain, &excludes).await;
    }
    if matches!(mode.as_str(), "site" | "both") {
        backup_all_sites(&mut report, &dest, retain, &excludes).await;
    }
    // 库不在家目录/站点目录内，两种文件策略都补一份库备份
    backup_all_dbs(&mut report, &dest, retain).await;

    report.finished_at = now_ts();
    let summary = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string());
    gs_set("backup_all_report", &summary).await;
    let _ = audit::log(
        None,
        None,
        "backup_all",
        &mode,
        &format!("dest={dest} ok={} fail={}", report.ok, report.fail),
    )
    .await;
}

/// 策略二：遍历用户表，整目录 + 客户追加的家目录外路径，套用管理员通用排除 + 用户自定义排除。
async fn backup_all_home(
    report: &mut BackupAllReport,
    dest: &str,
    retain: i64,
    excludes: &[String],
) {
    let pool = crate::db::get_db_pool().await;
    let users: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, username, home_dir FROM user WHERE home_dir<>''")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    for (uid, username, home) in users {
        let home = home.trim();
        if home.is_empty() {
            continue;
        }
        let (root, as_user) = resolve_full_root(dest, home, &username);
        let mut paths: Vec<String> = vec![home.to_string()];
        let extra = extra_paths("user", uid).await;
        paths.extend(extra);
        let exf = user_exclude_file(home);
        let target = match serde_json::to_string(&DirTarget {
            paths: paths.clone(),
            as_user: as_user.clone(),
        }) {
            Ok(t) => t,
            Err(e) => {
                report.fail += 1;
                report.failures.push(
                    json!({"kind":"home","name":username,"error":format!("序列化失败: {e}")}),
                );
                continue;
            }
        };
        match run_backup(RunBackupArgs {
            target_type: "dir",
            target_json: &target,
            name: &username,
            dest_dir: "",
            job_id: None,
            owner: &username,
            dest_root: Some(&root),
            excludes,
            exclude_file: exf.as_deref(),
            manifest: &paths,
        })
        .await
        {
            Ok(_) => {
                report.ok += 1;
                prune_by_owner(&username, retain, &root).await;
            }
            Err(e) => {
                report.fail += 1;
                report
                    .failures
                    .push(json!({"kind":"home","name":username,"error":e.to_string()}));
            }
        }
    }
}

/// 策略一：遍历站点，文档根 + 应用工作目录 + 客户追加目录一并打包（库在 `backup_all_dbs` 统一处理）。
async fn backup_all_sites(
    report: &mut BackupAllReport,
    dest: &str,
    retain: i64,
    excludes: &[String],
) {
    let pool = crate::db::get_db_pool().await;
    let sites: Vec<(i64, i64, String, String)> =
        sqlx::query_as("SELECT id, user_id, name, web_root FROM site WHERE web_root<>''")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    for (id, uid, name, web_root) in sites {
        let (owner, home_backups) = match user_backup_root(uid).await {
            Ok(x) => x,
            Err(_) => continue,
        };
        let home = home_backups
            .trim_end_matches("/backups")
            .trim_end_matches('/');
        let (root, as_user) = resolve_full_root(dest, home, &owner);
        let mut paths: Vec<String> = vec![web_root.trim().to_string()];
        let apps: Vec<(String,)> =
            sqlx::query_as("SELECT workdir FROM site_apps WHERE site_id=? AND workdir<>''")
                .bind(id)
                .fetch_all(pool)
                .await
                .unwrap_or_default();
        for (w,) in apps {
            paths.push(w);
        }
        let extra = extra_paths("site", id).await;
        paths.extend(extra);
        let exf = user_exclude_file(home);
        let arc = if name.trim().is_empty() {
            format!("site-{}", id)
        } else {
            name.trim().to_string()
        };
        let target = match serde_json::to_string(&DirTarget {
            paths: paths.clone(),
            as_user: as_user.clone(),
        }) {
            Ok(t) => t,
            Err(e) => {
                report.fail += 1;
                report
                    .failures
                    .push(json!({"kind":"site","name":arc,"error":format!("序列化失败: {e}")}));
                continue;
            }
        };
        match run_backup(RunBackupArgs {
            target_type: "dir",
            target_json: &target,
            name: &arc,
            dest_dir: "",
            job_id: None,
            owner: &owner,
            dest_root: Some(&root),
            excludes,
            exclude_file: exf.as_deref(),
            manifest: &paths,
        })
        .await
        {
            Ok(_) => {
                report.ok += 1;
                prune_by_owner(&owner, retain, &root).await;
            }
            Err(e) => {
                report.fail += 1;
                report
                    .failures
                    .push(json!({"kind":"site","name":arc,"error":e.to_string()}));
            }
        }
    }
}

/// 数据库备份：有前缀库归对应用户，无前缀/无法解析前缀的库归管理员（写 admin 家目录或系统根）。
async fn backup_all_dbs(report: &mut BackupAllReport, dest: &str, retain: i64) {
    let pool = crate::db::get_db_pool().await;
    let dbs: Vec<(String,)> = sqlx::query_as(
        "SELECT schema_name FROM information_schema.schemata \
         WHERE schema_name NOT IN ('mysql','information_schema','performance_schema','sys') \
         AND schema_name NOT LIKE 'zap%'",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let socket = crate::routers::database::socket_path().await;
    for (db,) in dbs {
        let (owner, home) = match db_owner_root(&db, "admin").await {
            Ok((o, h)) => (
                o,
                h.trim_end_matches("/backups")
                    .trim_end_matches('/')
                    .to_string(),
            ),
            Err(_) => {
                // 无前缀库归管理员
                let admin_home: Option<(String,)> =
                    sqlx::query_as("SELECT home_dir FROM user WHERE username='admin' LIMIT 1")
                        .fetch_optional(pool)
                        .await
                        .ok()
                        .flatten();
                let h = admin_home
                    .map(|(x,)| x.trim().to_string())
                    .unwrap_or_default();
                ("admin".to_string(), h)
            }
        };
        let (root, _as_user) = resolve_full_root(dest, &home, &owner);
        let pwd = match zap_crypto::read_cred("mysql", "zapadm") {
            Ok(p) => p,
            Err(e) => {
                report.fail += 1;
                report
                    .failures
                    .push(json!({"kind":"db","name":db,"error":format!("读凭据失败: {e}")}));
                continue;
            }
        };
        let target = match serde_json::to_string(&DbTarget {
            engine: "mysql".to_string(),
            db_name: db.clone(),
            user: "zapadm".to_string(),
            password: pwd,
            host: "127.0.0.1".to_string(),
            port: 3306,
            socket: socket.clone(),
            db_path: None,
        }) {
            Ok(t) => t,
            Err(e) => {
                report.fail += 1;
                report
                    .failures
                    .push(json!({"kind":"db","name":db,"error":format!("序列化失败: {e}")}));
                continue;
            }
        };
        match run_backup(RunBackupArgs {
            target_type: "db",
            target_json: &target,
            name: &db,
            dest_dir: "",
            job_id: None,
            owner: &owner,
            dest_root: Some(&root),
            excludes: &[],
            exclude_file: None,
            manifest: std::slice::from_ref(&db),
        })
        .await
        {
            Ok(_) => {
                report.ok += 1;
                prune_by_owner(&owner, retain, &root).await;
            }
            Err(e) => {
                report.fail += 1;
                report
                    .failures
                    .push(json!({"kind":"db","name":db,"error":e.to_string()}));
            }
        }
    }
}
