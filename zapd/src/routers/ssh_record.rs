// SPDX-License-Identifier: AGPL-3.0-only
//! SSH 终端会话录制与回放（asciinema v2 cast 格式）。
//!
//! 只录**远端输出**，不录键盘输入：输入里可能有密码、token，
//! 录进文件等于把凭据明文落盘（回放时也看不出「用户敲了什么」这种错觉）。
//! 输出本身同样可能含敏感内容，所以录制文件按用户隔离存放、
//! 只允许本人读取，并随时可删。
//!
//! 落盘用同步 `std::fs`：单次写入只是几十字节到几 KB，
//! 走 `tokio::fs` 反而每次都要额外投递一次阻塞任务。

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use axum::extract::{Path, Query};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Executor;
use sqlx::Row;
use tracing::{info, warn};

use crate::db;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::{ZapError, ZapJsonResult};

/// 单个录制文件的体积上限：到了就停止追加（会话照常继续，只是不再录）。
///
/// 一条 8 小时的会话如果一直刷日志，不加限制能把磁盘写满。
const MAX_RECORD_BYTES: u64 = 32 * 1024 * 1024;

/// 每个用户保留的录制条数上限，超出丢最旧的。
const MAX_KEEP_PER_USER: usize = 200;

/// 录制保留天数：超过就清掉（录制是审计用的，不是归档系统）。
const MAX_KEEP_DAYS: i64 = 30;

/// 回放时单条录制的最大读取体积（> 这个大小的 cast 前端也吃不消）
const MAX_READ_BYTES: u64 = 32 * 1024 * 1024;

// ── 建表 ────────────────────────────────────────────────────

pub async fn init_table() {
    let pool = db::get_db_pool().await;
    let sql = r#"
    CREATE TABLE IF NOT EXISTS ssh_recordings (
        id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
        -- 归属用户：录制内容可能含敏感输出，严格按用户隔离
        user_id INTEGER NOT NULL DEFAULT 0,
        conn_id INTEGER NOT NULL DEFAULT 0,
        -- 文件名（不含目录）：目录由 user_id 决定，见 recordings_dir
        file_name VARCHAR(255) NOT NULL DEFAULT '',
        started_at INTEGER NOT NULL DEFAULT 0,
        ended_at INTEGER NOT NULL DEFAULT 0,
        duration_ms INTEGER NOT NULL DEFAULT 0,
        size_bytes INTEGER NOT NULL DEFAULT 0,
        cols INTEGER NOT NULL DEFAULT 80,
        rows INTEGER NOT NULL DEFAULT 24,
        status INTEGER NOT NULL DEFAULT 1
    );
    CREATE INDEX IF NOT EXISTS idx_ssh_rec_user ON ssh_recordings(user_id);
    CREATE INDEX IF NOT EXISTS idx_ssh_rec_conn ON ssh_recordings(conn_id);
    "#;
    pool.execute(sql).await.unwrap();
}

/// 录制文件根目录：`{data}/ssh_recordings/<user_id>/`
fn recordings_dir(user_id: i64) -> PathBuf {
    crate::zap::user_cron::data_dir()
        .join("ssh_recordings")
        .join(user_id.to_string())
}

// ── 录制器 ──────────────────────────────────────────────────

/// 一次终端会话的录制器；未开启录制时用 `None`（见 `Recorder::start`）。
pub(crate) struct Recorder {
    started: Instant,
    started_at: i64,
    path: PathBuf,
    file: Option<File>,
    cols: u32,
    rows: u32,
    bytes: u64,
    /// 已达体积上限：后续不再写盘（会话继续跑，只是不录了）
    capped: bool,
    /// 是否已正常入库：未入库的（会话半途失败）由 `Drop` 清掉文件
    kept: bool,
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        // 会话在开 PTY / 起 shell 时就失败了：文件没入库，留着就是永远清不掉的孤儿。
        // 正常收尾走 `finish`，会先把 kept 置上再落到这里。
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
        let _ = fs::remove_file(&self.path);
    }
}

impl Recorder {
    /// 开启录制并写入 asciinema v2 的头行。建目录 / 建文件失败时静默降级为「不录」——
    /// 录制失败不该让终端连不上。
    pub(crate) fn start(user_id: i64, conn_id: i64, cols: u32, rows: u32) -> Option<Self> {
        let dir = recordings_dir(user_id);
        if let Err(e) = fs::create_dir_all(&dir) {
            warn!("创建录制目录失败（本次会话不录制）: {e}");
            return None;
        }
        let started_at = chrono::Utc::now().timestamp();
        let name = format!(
            "{}-c{}-{}.cast",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            conn_id,
            started_at % 100000
        );
        let path = dir.join(&name);
        let mut file = match File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                warn!("创建录制文件失败（本次会话不录制）: {e}");
                return None;
            }
        };
        let header = json!({
            "version": 2,
            "width": cols,
            "height": rows,
            "timestamp": started_at,
            "title": format!("SSH #{conn_id}"),
            "env": { "TERM": "xterm-256color", "SHELL": "/bin/bash" },
        });
        let line = format!("{header}\n");
        if let Err(e) = file.write_all(line.as_bytes()) {
            warn!("写录制头失败（本次会话不录制）: {e}");
            return None;
        }
        Some(Self {
            started: Instant::now(),
            started_at,
            path,
            file: Some(file),
            cols,
            rows,
            bytes: line.len() as u64,
            capped: false,
            kept: false,
        })
    }

    /// 记一段远端输出。
    pub(crate) fn output(&mut self, data: &[u8]) {
        if self.capped {
            return;
        }
        let text = String::from_utf8_lossy(data);
        let payload = serde_json::to_string(text.as_ref()).unwrap_or_else(|_| "\"\"".to_string());
        let ts = self.started.elapsed().as_secs_f64();
        self.write_event(&format!("[{:.3}, \"o\", {payload}]\n", ts));
    }

    /// 记一次窗口尺寸变化（回放时据此调整终端大小，否则宽屏内容会串行）。
    pub(crate) fn resize(&mut self, cols: u32, rows: u32) {
        if self.capped {
            return;
        }
        let ts = self.started.elapsed().as_secs_f64();
        self.write_event(&format!("[{:.3}, \"r\", \"{cols}x{rows}\"]\n", ts));
    }

    fn write_event(&mut self, line: &str) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if self.bytes + line.len() as u64 > MAX_RECORD_BYTES {
            // 到上限：写一条说明收尾，之后不再落盘
            self.capped = true;
            let notice = "[录制体积已达上限，后续内容未录制]";
            let payload = serde_json::to_string(&format!("\r\n{notice}\r\n")).unwrap_or_default();
            let ts = self.started.elapsed().as_secs_f64();
            let _ = file.write_all(format!("[{ts:.3}, \"o\", {payload}]\n").as_bytes());
            let _ = file.flush();
            return;
        }
        if file.write_all(line.as_bytes()).is_err() {
            // 磁盘满 / 只读：停止录制，别让写错误一直刷
            self.capped = true;
            self.file = None;
            return;
        }
        self.bytes += line.len() as u64;
    }

    /// 会话结束：落库 + 顺手清理超量的历史录制。
    pub(crate) async fn finish(mut self, user_id: i64, conn_id: i64) {
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
        self.file = None;
        let size = fs::metadata(&self.path)
            .map(|m| m.len())
            .unwrap_or(self.bytes);
        let duration_ms = self.started.elapsed().as_millis() as i64;
        // 空录制（连上就断开、没任何输出）不入库，免得列表里堆一堆 0 字节
        if self.bytes <= 200 && duration_ms < 1000 {
            let _ = fs::remove_file(&self.path);
            return;
        }
        let file_name = self
            .path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let now = chrono::Utc::now().timestamp();
        let _ = sqlx::query(
            "INSERT INTO ssh_recordings \
             (user_id, conn_id, file_name, started_at, ended_at, duration_ms, size_bytes, cols, rows, status) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
        )
        .bind(user_id)
        .bind(conn_id)
        .bind(&file_name)
        .bind(self.started_at)
        .bind(now)
        .bind(duration_ms)
        .bind(size as i64)
        .bind(self.cols as i64)
        .bind(self.rows as i64)
        .execute(db::get_db_pool().await)
        .await;
        self.kept = true;
        info!(
            "SSH session recorded: {} ({} bytes, {} ms)",
            file_name, size, duration_ms
        );
        prune(user_id).await;
    }
}

/// 清理超量 / 过期的录制：条数上限优先，其次按天数。
async fn prune(user_id: i64) {
    let pool = db::get_db_pool().await;
    let cutoff = chrono::Utc::now().timestamp() - MAX_KEEP_DAYS * 86400;
    let expired: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, file_name FROM ssh_recordings \
         WHERE user_id = ? AND started_at < ? ORDER BY id",
    )
    .bind(user_id)
    .bind(cutoff)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (id, name) in expired {
        remove_one(user_id, id, &name).await;
    }
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, file_name FROM ssh_recordings WHERE user_id = ? ORDER BY id DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    if rows.len() > MAX_KEEP_PER_USER {
        for (id, name) in rows.into_iter().skip(MAX_KEEP_PER_USER) {
            remove_one(user_id, id, &name).await;
        }
    }
}

/// 删库记录 + 删文件（文件没了也照样清记录，避免留下打不开的幽灵条目）
async fn remove_one(user_id: i64, id: i64, file_name: &str) {
    if !file_name.is_empty() {
        let p = recordings_dir(user_id).join(file_name);
        let _ = fs::remove_file(p);
    }
    let _ = sqlx::query("DELETE FROM ssh_recordings WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(db::get_db_pool().await)
        .await;
}

// ── 接口 ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListQuery {
    /// 只看某条连接的录制；缺省 = 全部
    pub conn_id: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

/// GET /terminal/recordings —— 我的会话录制列表（按时间倒序）
pub async fn list_recordings(claims: ValidatedClaims, Query(q): Query<ListQuery>) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let limit = q.limit.clamp(1, 500);
    let rows: Vec<sqlx::sqlite::SqliteRow> = match q.conn_id {
        Some(cid) => {
            sqlx::query(
                "SELECT r.*, c.name AS conn_name, c.host AS conn_host \
                 FROM ssh_recordings r LEFT JOIN ssh_connections c ON c.id = r.conn_id \
                 WHERE r.user_id = ? AND r.conn_id = ? ORDER BY r.id DESC LIMIT ?",
            )
            .bind(claims.id as i64)
            .bind(cid)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query(
                "SELECT r.*, c.name AS conn_name, c.host AS conn_host \
                 FROM ssh_recordings r LEFT JOIN ssh_connections c ON c.id = r.conn_id \
                 WHERE r.user_id = ? ORDER BY r.id DESC LIMIT ?",
            )
            .bind(claims.id as i64)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };
    let items: Vec<Value> = rows.iter().map(|r| row_to_json(r)).collect();
    crate::zap::api_ok(items)
}

fn row_to_json(r: &sqlx::sqlite::SqliteRow) -> Value {
    json!({
        "id": r.try_get::<i64, _>("id").unwrap_or(0),
        "conn_id": r.try_get::<i64, _>("conn_id").unwrap_or(0),
        "conn_name": r.try_get::<String, _>("conn_name").unwrap_or_default(),
        "conn_host": r.try_get::<String, _>("conn_host").unwrap_or_default(),
        "started_at": r.try_get::<i64, _>("started_at").unwrap_or(0),
        "duration_ms": r.try_get::<i64, _>("duration_ms").unwrap_or(0),
        "size_bytes": r.try_get::<i64, _>("size_bytes").unwrap_or(0),
    })
}

/// GET /terminal/recordings/{id} —— 取回一条录制的 asciinema cast 文本供回放
pub async fn get_recording(claims: ValidatedClaims, Path(id): Path<i64>) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT user_id, file_name FROM ssh_recordings WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    let Some((owner, file_name)) = row else {
        return Err(ZapError::New(-1, "录制不存在".to_string()));
    };
    // 录制内容等同终端输出，归属隔离必须和终端一样严
    if owner != claims.id as i64 {
        return Err(ZapError::New(-1, "无权访问该录制".to_string()));
    }
    let path = recordings_dir(owner).join(&file_name);
    let meta = fs::metadata(&path).map_err(|_| ZapError::New(-1, "录制文件已丢失".to_string()))?;
    if meta.len() > MAX_READ_BYTES {
        return Err(ZapError::New(-1, "录制文件过大，无法在线回放".to_string()));
    }
    let content =
        fs::read_to_string(&path).map_err(|_| ZapError::New(-1, "读取录制文件失败".to_string()))?;
    audit::log(
        Some(&claims),
        None,
        "ssh_recording_play",
        &format!("id={}", id),
        &format!("回放终端会话录制 #{id}"),
    )
    .await;
    crate::zap::api_ok(json!({ "id": id, "content": content }))
}

/// DELETE /terminal/recordings/{id} —— 删除一条录制（库记录 + 文件）
pub async fn delete_recording(claims: ValidatedClaims, Path(id): Path<i64>) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT user_id, file_name FROM ssh_recordings WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    let Some((owner, file_name)) = row else {
        return Err(ZapError::New(-1, "录制不存在".to_string()));
    };
    if owner != claims.id as i64 {
        return Err(ZapError::New(-1, "无权删除该录制".to_string()));
    }
    remove_one(owner, id, &file_name).await;
    audit::log(
        Some(&claims),
        None,
        "ssh_recording_delete",
        &format!("id={}", id),
        &format!("删除终端会话录制 #{id}"),
    )
    .await;
    crate::zap::api_ok(json!({ "deleted": id }))
}
