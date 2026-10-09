// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{
        Extension, Path, Query,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use russh::client;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Executor, Row};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use zap_proto::Request;

use crate::db;
use crate::zap::audit;
use crate::zap::crypto;
use crate::zap::jwt::{Claims, ValidatedClaims};
use crate::zap::{ZapError, ZapJsonResult};

/// SSH 建连 + 认证的**整段**超时。
///
/// 主机不可达时不能让请求干等到 OS 层 TCP 超时（分钟级），
/// 那样会白白占着一个 WebSocket 和一个 tokio 任务。
const SSH_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// 空闲自动断开：输入/输出都没有这么久就结束会话。
///
/// 用户忘关标签页时，一条 SSH 连接会一直占着（服务端也维持着会话），
/// 时间久了既占资源又留着一个已认证的通道。
const SSH_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// 单次会话最长时长：即使一直有活动也不无限期挂着。
const SSH_MAX_SESSION: Duration = Duration::from_secs(8 * 60 * 60);

/// 同一时刻最多允许的终端会话数。
///
/// 没有上限时，多开几个标签页就能堆出一大把 SSH 连接（每条都要占用本地 fd、
/// 一个 tokio 任务，以及对端的一个会话）。满了直接拒绝开新会话。
const MAX_SSH_SESSIONS: usize = 32;

/// 认证失败限流：同一用户 + 同一连接在 `SSH_FAIL_WINDOW` 内失败这么多次就先不许再试。
///
/// 没有它的话，一个面板账号就能拿终端当爆破器，对着目标主机反复试密码
/// （还会触发对端的 MaxStartups / fail2ban）。
const SSH_FAIL_LIMIT: u32 = 5;
const SSH_FAIL_WINDOW: Duration = Duration::from_secs(300);

/// 全局并发会话配额（见 `MAX_SSH_SESSIONS`）
static SSH_SESSION_SLOTS: Semaphore = Semaphore::const_new(MAX_SSH_SESSIONS);

/// 认证失败计数：`(user_id, conn_id) -> (失败次数, 首次失败时刻)`
static SSH_AUTH_FAILS: LazyLock<Mutex<HashMap<(i64, i64), (u32, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 结构化错误帧：和 resize / ask_password 同一套控制消息协议。
///
/// 以前错误是直接把文本混进终端输出流（`"认证失败: xxx\r\n"`），前端分不清
/// 这是 shell 输出还是报错；现在统一成 JSON，前端据此弹提示。
fn error_frame(msg: &str) -> Message {
    Message::Text(axum::extract::ws::Utf8Bytes::from(
        json!({ "type": "error", "message": msg }).to_string(),
    ))
}

// ── Database schema ────────────────────────────────────────

pub async fn init_table() {
    if !table_exists("ssh_connections").await {
        let sql = r#"
        CREATE TABLE ssh_connections (
            id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
            -- user_id：归属用户 id（0 = 历史/无主连接，启动时自动划归 admin）
            user_id INTEGER NOT NULL DEFAULT 0,
            name VARCHAR(128) NOT NULL,
            host VARCHAR(256) NOT NULL,
            port INTEGER DEFAULT 22,
            username VARCHAR(128) NOT NULL DEFAULT 'root',
            auth_type VARCHAR(32) NOT NULL DEFAULT 'password',
            password VARCHAR(512) DEFAULT '',
            ssh_key_name VARCHAR(128) DEFAULT '',
            -- host_key_fingerprint：主机密钥指纹（TOFU，见 ssh_connect）。
            -- 空 = 还没信任过该主机的密钥，首次连成后写入；非空则后续连接必须匹配
            host_key_fingerprint VARCHAR(128) DEFAULT '',
            remark TEXT DEFAULT '',
            status INTEGER DEFAULT 1,
            sort_order INTEGER DEFAULT 0,
            created_at INTEGER,
            updated_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_ssh_conn_user ON ssh_connections(user_id);
        "#;
        let pool = db::get_db_pool().await;
        pool.execute(sql).await.unwrap();
        info!("ssh_connections table created");
    }
    // 老库升级（幂等）：为存量 ssh_connections 补充归属列
    let _ = db::get_db_pool()
        .await
        .execute("ALTER TABLE ssh_connections ADD COLUMN user_id INTEGER NOT NULL DEFAULT 0")
        .await;
    // 主机密钥指纹（TOFU）：存量库补列（幂等，已存在则报错被忽略）
    let _ = db::get_db_pool()
        .await
        .execute(
            "ALTER TABLE ssh_connections ADD COLUMN host_key_fingerprint VARCHAR(128) DEFAULT ''",
        )
        .await;
    let _ = db::get_db_pool()
        .await
        .execute("CREATE INDEX IF NOT EXISTS idx_ssh_conn_user ON ssh_connections(user_id)")
        .await;
    // 严格隔离的归属兜底：所有历史/无主连接（user_id=0，含旧版默认本机连接）
    // 一律划归 admin 账号（admin 在 init_schema 中先于此函数种子化），
    // 保证这些连接仍由面板管理员可见可管，同时不会泄漏给其他任何用户。
    let _ = db::get_db_pool()
        .await
        .execute(
            "UPDATE ssh_connections SET user_id = (SELECT id FROM user WHERE username = 'admin' LIMIT 1) \
             WHERE user_id = 0 AND EXISTS (SELECT 1 FROM user WHERE username = 'admin')",
        )
        .await;
    // 默认本地连接：面板本机 127.0.0.1/root，密码留空，归属 admin。
    // 新库与老库都幂等补插（已存在 root@127.0.0.1 或 root@localhost 则跳过），
    // 用户后续自行编辑填写密码或改为密钥。
    ensure_default_loopback_connection().await;
    // 注：用户 SSH 密钥不建表——密钥只存用户家目录 ~/.ssh/zap_<name>（见
    // zapexec ssh_user_key），列表由磁盘扫描得出，避免库重建后元数据与文件失联。
}

/// 幂等补插默认本地连接（127.0.0.1 / root / 22，密码为空），归属 admin。
/// 空密码连接双击时由前端弹窗输入密码、仅本次会话使用不落库。
async fn ensure_default_loopback_connection() {
    let pool = db::get_db_pool().await;
    let existing = sqlx::query(
        "SELECT id FROM ssh_connections
         WHERE username = 'root' AND (host = '127.0.0.1' OR host = 'localhost')
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    if existing.is_some() {
        return;
    }
    let now = chrono::Utc::now().timestamp();
    let remark = "本机默认连接：未设置密码，双击连接时输入密码（仅本次会话使用，不会保存）";
    // 归属 admin（严格隔离下仅 admin 自己能看见）；查不到 admin 时先落 user_id=0，
    // 下次启动由 init_table 的兜底迁移划归 admin
    let owner: i64 = sqlx::query_scalar("SELECT id FROM user WHERE username = 'admin' LIMIT 1")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(0);
    sqlx::query(
        "INSERT INTO ssh_connections (user_id, name, host, port, username, auth_type, password, ssh_key_name, remark, status, sort_order, created_at, updated_at)
         VALUES (?, 'localhost', '127.0.0.1', 22, 'root', 'password', '', '', ?, 1, 0, ?, ?)",
    )
    .bind(owner)
    .bind(remark)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .ok();
    info!("Default loopback SSH connection seeded (root@127.0.0.1, no password)");
}

async fn table_exists(table_name: &str) -> bool {
    let pool = db::get_db_pool().await;
    let result: Result<(String,), sqlx::Error> =
        sqlx::query_as("select name from sqlite_master where name = ?")
            .bind(table_name)
            .fetch_one(pool)
            .await;
    result.is_ok()
}

// ── Models ─────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct SshConnection {
    pub id: i64,
    /// 归属用户 id（连接严格按归属隔离：各角色仅能查看/管理自己的连接）
    pub owner_id: i64,
    /// 归属用户名（用户已删除时为空）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner_name: String,
    pub name: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub auth_type: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub password: String,
    /// 是否已保存密码（空密码 = 未设置，连接时由前端弹窗临时输入，不落库）
    pub has_password: bool,
    pub ssh_key_name: String,
    pub remark: String,
    pub status: i32,
    pub sort_order: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

fn row_to_conn(row: &sqlx::sqlite::SqliteRow) -> SshConnection {
    let stored_password: String = row.try_get("password").unwrap_or_default();
    SshConnection {
        id: row.get("id"),
        owner_id: row.try_get("owner_id").unwrap_or(0),
        owner_name: row.try_get("owner_name").unwrap_or_default(),
        name: row.get("name"),
        host: row.get("host"),
        port: row.get("port"),
        username: row.get("username"),
        auth_type: row.get("auth_type"),
        password: String::new(), // 一律不回传密文
        has_password: !stored_password.is_empty(),
        ssh_key_name: row.try_get("ssh_key_name").unwrap_or_default(),
        remark: row.try_get("remark").unwrap_or_default(),
        status: row.try_get("status").unwrap_or(1),
        sort_order: row.try_get("sort_order").unwrap_or(0),
        created_at: row.try_get("created_at").unwrap_or(0),
        updated_at: row.try_get("updated_at").unwrap_or(0),
    }
}

// ── 归属 / 可见性 ────────────────────────────────────────────

/// SSH 连接严格按归属隔离（为安全起见，admin / reseller / 普通用户一律只看自己的连接）：
/// 仅 owner = 当前登录用户时可访问；历史/无主连接（user_id=0）已在启动迁移中划归 admin。
async fn connection_in_scope(claims: &Claims, conn_id: i64) -> Result<i64, ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM ssh_connections WHERE id = ?")
        .bind(conn_id)
        .fetch_optional(pool)
        .await?;
    let Some((owner_id,)) = row else {
        return Err(ZapError::New(-1, "连接不存在".to_string()));
    };
    if owner_id != claims.id as i64 {
        return Err(ZapError::New(
            -1,
            "无权访问该连接：连接归属其他用户".to_string(),
        ));
    }
    Ok(owner_id)
}

#[derive(Debug, Deserialize)]
pub struct CreateConnectionPayload {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: i32,
    #[serde(default = "default_username")]
    pub username: String,
    #[serde(default = "default_auth_type")]
    pub auth_type: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub ssh_key_name: String,
    #[serde(default)]
    pub remark: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateConnectionPayload {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<i32>,
    pub username: Option<String>,
    pub auth_type: Option<String>,
    pub password: Option<String>,
    pub ssh_key_name: Option<String>,
    pub remark: Option<String>,
    pub status: Option<i32>,
    pub sort_order: Option<i32>,
    /// `true` = 解除对当前主机密钥的信任（清空 TOFU 指纹），下次连接重新记录。
    /// 主机重装但地址没变、导致指纹校验不过时用。缺省（老客户端）为 None，不动。
    pub reset_host_key: Option<bool>,
}

fn default_port() -> i32 {
    22
}
fn default_username() -> String {
    "root".to_string()
}
fn default_auth_type() -> String {
    "password".to_string()
}

// ── CRUD handlers ──────────────────────────────────────────

const CONN_SEL_COLS: &str = "s.id, s.name, s.host, s.port, s.username, s.auth_type, s.password, \
                             s.ssh_key_name, s.remark, s.status, s.sort_order, s.created_at, s.updated_at, \
                             s.user_id AS owner_id";

/// 列表严格隔离：仅列出当前登录用户（含 admin / reseller）自己创建的连接。
pub async fn list_connections(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let sel = format!(
        "SELECT {CONN_SEL_COLS}, u.username AS owner_name \
         FROM ssh_connections s LEFT JOIN user u ON u.id = s.user_id"
    );
    let rows: Vec<sqlx::sqlite::SqliteRow> = sqlx::query(sqlx::AssertSqlSafe(format!(
        "{sel} WHERE s.user_id = ? ORDER BY s.sort_order, s.id"
    )))
    .bind(claims.id as i64)
    .fetch_all(pool)
    .await?;

    let connections: Vec<SshConnection> = rows.iter().map(row_to_conn).collect();
    crate::zap::api_ok(connections)
}

pub async fn get_connection(claims: ValidatedClaims, Path(id): Path<i64>) -> ZapJsonResult {
    connection_in_scope(&claims, id).await?;
    let pool = db::get_db_pool().await;
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {CONN_SEL_COLS}, u.username AS owner_name \
         FROM ssh_connections s LEFT JOIN user u ON u.id = s.user_id \
         WHERE s.id = ?"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            let mut conn = row_to_conn(&r);
            conn.password = String::new(); // 脱敏
            crate::zap::api_ok(conn)
        }
        None => Err(ZapError::New(-1, "连接不存在".to_string())),
    }
}

pub async fn create_connection(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CreateConnectionPayload>,
) -> ZapJsonResult {
    if payload.name.trim().is_empty() {
        return Err(ZapError::New(-1, "连接名称不能为空".to_string()));
    }
    if payload.host.trim().is_empty() {
        return Err(ZapError::New(-1, "主机地址不能为空".to_string()));
    }
    if payload.auth_type != "password" && payload.auth_type != "key" {
        return Err(ZapError::New(
            -1,
            "认证类型仅支持 password 或 key".to_string(),
        ));
    }
    // 密码认证允许密码为空：表示「未设置密码」，连接时由前端弹窗临时输入、不落库。
    // 密钥认证的 ssh_key_name 同样允许为空：表示不绑定面板密钥，连接时自动探测
    // 家目录 ~/.ssh 下的默认私钥（id_ed25519 → id_ecdsa → id_rsa）。

    let pool = db::get_db_pool().await;
    let now = chrono::Utc::now().timestamp();

    // 密码加密后入库，杜绝明文存储（加密失败直接报错，不落明文）
    let encrypted_password =
        crypto::encrypt_password(&payload.password).map_err(|e| ZapError::New(-1, e))?;

    sqlx::query(
        "INSERT INTO ssh_connections (user_id, name, host, port, username, auth_type, password, ssh_key_name, remark, status, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 0, ?, ?)"
    )
    .bind(claims.id as i64)
    .bind(payload.name.trim())
    .bind(payload.host.trim())
    .bind(payload.port)
    .bind(&payload.username)
    .bind(&payload.auth_type)
    .bind(encrypted_password)
    .bind(&payload.ssh_key_name)
    .bind(&payload.remark)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssh_connection_create",
        &format!("{}@{}:{}", payload.username, payload.host, payload.port),
        &payload.name,
    )
    .await;

    info!("SSH connection created: {}", payload.name);
    Ok(Json(json!({ "code": 0, "message": "创建成功" })))
}

pub async fn update_connection(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateConnectionPayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    // 归属校验：仅本人 / reseller 名下客户 / admin 可编辑
    connection_in_scope(&claims, id).await?;

    let now = chrono::Utc::now().timestamp();

    if let Some(v) = payload.name.as_deref().map(|s| s.trim().to_string()) {
        sqlx::query("UPDATE ssh_connections SET name = ?, updated_at = ? WHERE id = ?")
            .bind(&v)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.host.as_deref().map(|s| s.trim().to_string()) {
        // 换了主机 → 原先信任的主机密钥不再适用，清空指纹，下次连接重新走 TOFU
        sqlx::query(
            "UPDATE ssh_connections SET host = ?, host_key_fingerprint = '', updated_at = ? \
             WHERE id = ?",
        )
        .bind(&v)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    }
    if let Some(v) = payload.port {
        // 端口变了同理：指向的已经不是之前那台服务了
        sqlx::query(
            "UPDATE ssh_connections SET port = ?, host_key_fingerprint = '', updated_at = ? \
             WHERE id = ?",
        )
        .bind(v)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    }
    if let Some(v) = payload.username {
        sqlx::query("UPDATE ssh_connections SET username = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.auth_type {
        sqlx::query("UPDATE ssh_connections SET auth_type = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.password {
        // 密码加密后入库（加密失败直接报错，不落明文）
        let encrypted = crypto::encrypt_password(&v).map_err(|e| ZapError::New(-1, e))?;
        sqlx::query("UPDATE ssh_connections SET password = ?, updated_at = ? WHERE id = ?")
            .bind(encrypted)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.ssh_key_name {
        sqlx::query("UPDATE ssh_connections SET ssh_key_name = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.remark {
        sqlx::query("UPDATE ssh_connections SET remark = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.status {
        sqlx::query("UPDATE ssh_connections SET status = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.sort_order {
        sqlx::query("UPDATE ssh_connections SET sort_order = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
    }

    // 显式解除信任（主机重装但地址没变）：清空指纹，下次连接重新记录
    if payload.reset_host_key == Some(true) {
        sqlx::query("UPDATE ssh_connections SET host_key_fingerprint = '' WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        info!("SSH host key trust reset for connection {}", id);
    }

    info!("SSH connection updated: id={}", id);
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssh_connection_update",
        &format!("id={}", id),
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "更新成功" })))
}

pub async fn delete_connection(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Path(id): Path<i64>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    // 归属校验：仅本人 / reseller 名下客户 / admin 可删除
    connection_in_scope(&claims, id).await?;

    let result = sqlx::query("DELETE FROM ssh_connections WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ZapError::New(-1, "连接不存在".to_string()));
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssh_connection_delete",
        &format!("id={}", id),
        "",
    )
    .await;

    info!("SSH connection deleted: id={}", id);
    Ok(Json(json!({ "code": 0, "message": "删除成功" })))
}

// ── WebSocket SSH terminal ─────────────────────────────────

pub async fn ws_terminal(
    ws: WebSocketUpgrade,
    Path(id): Path<i64>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    // Validate token from query parameter (browser WebSocket doesn't support custom headers)
    let token = match params.get("token") {
        Some(t) => t.clone(),
        None => {
            return axum::response::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("Missing token"))
                .unwrap();
        }
    };

    // 走统一校验入口：签名 + 有效期 + 会话版本号（被「下线所有设备」作废的一并拒绝）
    let Some(claims) = crate::zap::jwt::decode_verified(&token) else {
        return axum::response::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(axum::body::Body::from("Invalid token"))
            .unwrap();
    };
    // 终端票据（一键 SSH）：绑死在这条连接上 —— 截到票据也换不了连接
    if claims.scope == crate::zap::jwt::SSH_SCOPE && claims.sub != format!("ssh:{id}") {
        return axum::response::Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(axum::body::Body::from(
                "该终端票据绑定的不是这条连接".to_string(),
            ))
            .unwrap();
    }
    // 演示账号仅支持浏览，禁止通过终端执行命令
    if crate::zap::jwt::is_demo(&claims) {
        return axum::response::Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(axum::body::Body::from("演示账号仅支持浏览，不能使用终端"))
            .unwrap();
    }
    // 套餐限制：已绑定套餐且未开启 SSH 终端时禁止使用（未绑定套餐不限制）
    if let Some(pkg) = crate::routers::package::package_of_user(claims.id as i64).await
        && pkg.allow_ssh != 1
    {
        return axum::response::Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(axum::body::Body::from(format!(
                "当前套餐「{}」未开启 SSH 终端，请联系服务商变更套餐",
                pkg.name
            )))
            .unwrap();
    }

    // 连接归属校验：严格隔离——任何角色（含 admin/reseller）仅能对自己创建的连接建立终端会话
    if let Err(e) = connection_in_scope(&claims, id).await {
        let msg = match &e {
            ZapError::New(_, m) => m.clone(),
            other => other.to_string(),
        };
        let status = if msg.contains("连接不存在") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::FORBIDDEN
        };
        return axum::response::Response::builder()
            .status(status)
            .body(axum::body::Body::from(msg))
            .unwrap();
    }

    let rows: u32 = params
        .get("rows")
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let cols: u32 = params
        .get("cols")
        .and_then(|v| v.parse().ok())
        .unwrap_or(80);

    ws.on_upgrade(move |socket| handle_terminal(socket, id, rows, cols, claims))
}

async fn handle_terminal(socket: WebSocket, conn_id: i64, rows: u32, cols: u32, claims: Claims) {
    info!("Terminal WebSocket connected for connection {}", conn_id);

    // 并发会话配额：满了就不要再往下建 SSH 连接了。
    // permit 持有到函数结束，会话关掉才归还。
    let _session_permit = match SSH_SESSION_SLOTS.try_acquire() {
        Ok(p) => p,
        Err(_) => {
            warn!("SSH terminal session limit reached (max {MAX_SSH_SESSIONS})");
            let (mut tx, _) = socket.split();
            let _ = tx
                .send(error_frame(&format!(
                    "并发终端会话已达上限（{MAX_SSH_SESSIONS}），请先关闭一些标签页"
                )))
                .await;
            let _ = tx.close().await;
            return;
        }
    };

    let conn_info = match load_connection_info(conn_id).await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load connection {}: {}", conn_id, e);
            // 把可读原因直接回显到终端（如 www 模式下「我的密钥」不可用），而非静默断开
            let msg = match &e {
                ZapError::New(_, m) => m.clone(),
                other => other.to_string(),
            };
            send_error_and_close(socket, &msg).await;
            return;
        }
    };

    // 密码认证但未保存密码（如默认的 localhost 连接）：先等待前端通过 WebSocket
    // 下发本次会话的临时密码 {"type":"auth","password":"..."}，认证后即丢弃、不落库
    let mut temporary_password: Option<String> = None;
    let need_ask_password = conn_info.auth_type == "password" && conn_info.password.is_empty();
    let (mut ws_tx, mut ws_rx) = if need_ask_password {
        let (mut tx, mut rx) = socket.split();
        let _ = tx
            .send(Message::Text(axum::extract::ws::Utf8Bytes::from(
                "\x1b[33m需要 SSH 密码，请在弹窗中输入（仅本次会话使用，不会保存）\x1b[0m\r\n",
            )))
            .await;
        // 结构化控制消息：前端据此弹出密码输入框并回发 {"type":"auth","password":...}。
        // 兜底场景：库里存了密文但解密后为空（如加密密钥轮换、历史数据），
        // 此时前端列表的 has_password 仍为 true、不会提前弹窗，只有这条消息能救场。
        let _ = tx
            .send(Message::Text(axum::extract::ws::Utf8Bytes::from(
                r#"{"type":"ask_password"}"#,
            )))
            .await;
        let pwd = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                match rx.next().await {
                    Some(Ok(Message::Text(t))) => {
                        if let Ok(auth) = serde_json::from_str::<AuthMsg>(t.as_ref())
                            && auth.kind == "auth"
                            && !auth.password.is_empty()
                        {
                            break auth.password;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break String::new(),
                    _ => continue,
                }
            }
        })
        .await
        .unwrap_or_default();
        if pwd.is_empty() {
            let _ = tx
                .send(error_frame("密码输入超时或已取消，连接已关闭"))
                .await;
            let _ = tx.close().await;
            return;
        }
        temporary_password = Some(pwd);
        (tx, rx)
    } else {
        socket.split()
    };

    // 认证失败限流：别把一次连接浪费在被限流的请求上，也不给爆破留窗口
    let user_id = claims.id as i64;
    if let Some(wait) = ssh_auth_blocked(user_id, conn_id) {
        let _ = ws_tx
            .send(error_frame(&format!(
                "认证失败次数过多（{SSH_FAIL_LIMIT} 次），请 {wait} 秒后再试"
            )))
            .await;
        let _ = ws_tx.close().await;
        return;
    }

    // 认证：临时密码优先（空密码连接由前端下发），否则使用库中保存的凭据
    let mut auth_info = conn_info;
    if let Some(pwd) = temporary_password {
        auth_info.password = pwd;
    }
    // 整段超时：不可达主机不能一直挂到 OS 层 TCP 超时（分钟级），
    // 也不让一个 WebSocket + tokio 任务无限期占着
    let handle = match tokio::time::timeout(SSH_CONNECT_TIMEOUT, ssh_connect(&auth_info)).await {
        Ok(Ok(h)) => {
            // 认证成功，清掉这条连接的失败计数
            clear_ssh_auth_failures(user_id, conn_id);
            h
        }
        Ok(Err(e)) => {
            error!("SSH authentication failed: {}", e);
            note_ssh_auth_failure(user_id, conn_id);
            let _ = ws_tx.send(error_frame(&format!("认证失败: {e}"))).await;
            let _ = ws_tx.close().await;
            return;
        }
        Err(_) => {
            // 连不上（主机不可达）不算凭据问题，不记入失败计数
            error!(
                "SSH connect timed out after {}s (connection {})",
                SSH_CONNECT_TIMEOUT.as_secs(),
                conn_id
            );
            let _ = ws_tx
                .send(error_frame(&format!(
                    "建连超时：{} 秒内没能连上 {}:{}",
                    SSH_CONNECT_TIMEOUT.as_secs(),
                    auth_info.host,
                    auth_info.port
                )))
                .await;
            let _ = ws_tx.close().await;
            return;
        }
    };

    let mut channel = match handle.channel_open_session().await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to open SSH channel: {}", e);
            let _ = ws_tx
                .send(error_frame(&format!("打开通道失败: {e}")))
                .await;
            let _ = ws_tx.close().await;
            return;
        }
    };

    if let Err(e) = channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
    {
        error!("Failed to request PTY: {}", e);
        let _ = ws_tx.send(error_frame(&format!("请求 PTY 失败: {e}"))).await;
        let _ = ws_tx.close().await;
        return;
    }

    if let Err(e) = channel.request_shell(true).await {
        error!("Failed to start shell: {}", e);
        let _ = ws_tx
            .send(error_frame(&format!("启动 shell 失败: {e}")))
            .await;
        let _ = ws_tx.close().await;
        return;
    }

    info!("SSH terminal started for connection {}", conn_id);

    // 终端是最高危操作（等于把目标机器的 shell 交给登录者），会话必须留痕
    audit::log(
        Some(&claims),
        None,
        "ssh_session_start",
        &format!(
            "{}@{}:{}",
            auth_info.username, auth_info.host, auth_info.port
        ),
        &format!("SSH 终端会话开启（连接 #{conn_id}）"),
    )
    .await;

    // Channel: WebSocket → PTY resize (cols, rows)
    let (resize_tx, mut resize_rx) = tokio::sync::mpsc::channel::<(u32, u32)>(16);

    // 主循环：远端输出 → WebSocket，WebSocket 输入 → 远端。
    // `ssh_writer` 是 'static 的写端，不占用 channel 借用，便于与 wait() 并发。
    let mut ssh_writer = channel.make_writer();
    // 会话计时：空闲多久收尾 + 整条会话最长能开多久。
    // 用 tokio 的 Instant —— `sleep_until` 只认它，和 `std::time::Instant` 不是同一类型。
    let started = tokio::time::Instant::now();
    let mut last_active = tokio::time::Instant::now();

    loop {
        tokio::select! {
            msg = channel.wait() => {
                let Some(msg) = msg else { break };
                match msg {
                    ChannelMsg::Data { data } => {
                        last_active = tokio::time::Instant::now();
                        if ws_tx.send(Message::Binary(data)).await.is_err() {
                            break;
                        }
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    ChannelMsg::ExitStatus { exit_status } => {
                        info!(
                            "SSH shell exited with status {} (connection {})",
                            exit_status, conn_id
                        );
                        // 明确告诉前端「结束了」，否则界面会停在没有任何输出的假死状态
                        let _ = ws_tx
                            .send(Message::Text(axum::extract::ws::Utf8Bytes::from(
                                format!("\r\n[远端 shell 已退出，状态码 {exit_status}]\r\n"),
                            )))
                            .await;
                        break;
                    }
                    _ => {}
                }
            }
            ws_msg = ws_rx.next() => {
                let Some(Ok(msg)) = ws_msg else { break };
                last_active = tokio::time::Instant::now();
                match msg {
                    Message::Text(t) => {
                        let txt = t.as_ref();
                        // resize 控制消息：前端 fit 后自动同步窗口尺寸。
                        // 这里刻意不用 continue —— 否则会跳过循环末尾的 window_change，
                        // 导致空闲状态下改窗口要一直等到下一个事件才生效。
                        if let Ok(resize) = serde_json::from_str::<ResizeMsg>(txt)
                            && resize.kind == "resize"
                            && resize.cols > 0
                            && resize.rows > 0
                        {
                            let _ = resize_tx.send((resize.cols, resize.rows)).await;
                        } else if ssh_writer.write_all(txt.as_bytes()).await.is_err() {
                            // 其余文本一律作为终端输入
                            break;
                        }
                    }
                    Message::Binary(d) => {
                        if ssh_writer.write_all(&d).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
                let _ = ssh_writer.flush().await;
            }
            // 空闲超时：既没输入也没输出这么久，主动收尾（先告知前端，再断开）
            _ = tokio::time::sleep_until(last_active + SSH_IDLE_TIMEOUT) => {
                warn!(
                    "SSH terminal idle timeout after {} min (connection {})",
                    SSH_IDLE_TIMEOUT.as_secs() / 60,
                    conn_id
                );
                let _ = ws_tx
                    .send(Message::Text(axum::extract::ws::Utf8Bytes::from(
                        "\r\n[空闲超时，会话已断开]\r\n".to_string(),
                    )))
                    .await;
                break;
            }
            // 会话总时长上限：即便一直有活动也不无限期挂着
            _ = tokio::time::sleep_until(started + SSH_MAX_SESSION) => {
                warn!(
                    "SSH terminal reached max session lifetime {} h (connection {})",
                    SSH_MAX_SESSION.as_secs() / 3600,
                    conn_id
                );
                let _ = ws_tx
                    .send(Message::Text(axum::extract::ws::Utf8Bytes::from(
                        "\r\n[已达单会话时长上限，会话已断开]\r\n".to_string(),
                    )))
                    .await;
                break;
            }
        }

        // 应用窗口尺寸变更：此处未持有 channel 的独占借用
        while let Ok((c, r)) = resize_rx.try_recv() {
            if let Err(e) = channel.window_change(c, r, 0, 0).await {
                warn!("PTY resize to {}x{} failed: {}", c, r, e);
            }
        }
    }

    // Cleanup
    let _ = ws_tx.close().await;
    let _ = channel.close().await;
    let _ = handle
        .disconnect(Disconnect::ByApplication, "", "English")
        .await;
    info!("Terminal WebSocket closed for connection {}", conn_id);
}

async fn send_error_and_close(socket: WebSocket, msg: &str) {
    let (mut sender, _) = socket.split();
    let _ = sender.send(error_frame(msg.trim())).await;
    let _ = sender.close().await;
}

// ── Authentication ─────────────────────────────────────────

/// russh 客户端事件回调。
///
/// 面板只需要「连上去、开终端 / 执行命令」，不消费服务端主动推送的消息，
/// 因此除 `check_server_key` 外全部使用默认实现。
struct SshClient {
    /// 已记录的主机密钥指纹（TOFU）：`None` = 首次连接，暂不比对
    expected: Option<String>,
    /// 本次握手实际看到的指纹，回传给 `ssh_connect` 用于落库 / 报清楚错
    observed: Arc<Mutex<Option<String>>>,
}

impl client::Handler for SshClient {
    type Error = russh::Error;

    /// 主机密钥校验：TOFU（Trust On First Use）。
    ///
    /// - 库里没记过指纹：接受，指纹由 `ssh_connect` 在**认证成功后**才落库
    ///   （握手先于认证，没通过认证的对端还不配被信任，不能急着记下来）；
    /// - 记过且一致：接受；
    /// - 记过但不一致：拒绝。「主机重装换了密钥」和「中间人攻击」无法自动区分，
    ///   所以一律拒绝，并给出可操作的提示。
    ///
    /// 证书（Certificate）由 CA 签发、自带权威校验，不参与指纹比对。
    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key,
            PublicKeyOrCertificate::Certificate(_) => return Ok(true),
        };
        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256).to_string();
        match &self.expected {
            None => {
                *self.observed.lock().unwrap() = Some(fingerprint);
                Ok(true)
            }
            Some(expected) if *expected == fingerprint => Ok(true),
            Some(expected) => {
                warn!("SSH 主机密钥不匹配：已记录 {expected}，实际 {fingerprint}");
                *self.observed.lock().unwrap() = Some(fingerprint);
                Ok(false)
            }
        }
    }
}

/// 建立 SSH 连接并完成认证（密码 / 私钥）。
///
/// 全程异步：russh 基于 tokio 实现，无需再像 libssh2 那样包一层
/// `spawn_blocking` 去隔离 OpenSSL 错误队列（那条链路会污染同线程上的 TLS）。
/// 解析 PEM 私钥：优先 OpenSSH 新格式，失败时按「剥离 PEM 头尾 + base64」兜底。
///
/// 家目录里的密钥常以 `-----END ...-----\n\n` 结尾，ssh-key 的 PEM 解码对此敏感，
/// 先 trim 再解析；仍失败则手工解出二进制交给 `from_bytes`。
fn parse_private_key(content: &str) -> Result<PrivateKey, String> {
    use base64::Engine;

    let trimmed = content.trim();
    if let Ok(key) = PrivateKey::from_openssh(trimmed) {
        return Ok(key);
    }
    let b64: String = trimmed
        .lines()
        .filter(|l| !l.trim_start().starts_with("-----"))
        .map(|l| l.trim())
        .collect();
    let bin = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("私钥 base64 解码失败: {e}"))?;
    PrivateKey::from_bytes(&bin).map_err(|e| format!("SSH 私钥解析失败: {e}"))
}

/// 认证失败限流：返回 `Some(还需等待的秒数)` 表示当前被限流、不该再试。
///
/// 计数以「用户 + 连接」为维度：别人失败不会连累你，你换一条连接也不受影响。
fn ssh_auth_blocked(user_id: i64, conn_id: i64) -> Option<u64> {
    let mut fails = SSH_AUTH_FAILS.lock().unwrap();
    let Some((count, first)) = fails.get(&(user_id, conn_id)) else {
        return None;
    };
    if *count < SSH_FAIL_LIMIT {
        return None;
    }
    let elapsed = first.elapsed();
    if elapsed >= SSH_FAIL_WINDOW {
        // 窗口已过：解除限流，下次从 1 重新计
        fails.remove(&(user_id, conn_id));
        return None;
    }
    Some(SSH_FAIL_WINDOW.as_secs().saturating_sub(elapsed.as_secs()))
}

/// 记一次认证失败（窗口已过期则重新从 1 开始）。
fn note_ssh_auth_failure(user_id: i64, conn_id: i64) {
    let mut fails = SSH_AUTH_FAILS.lock().unwrap();
    let entry = fails.entry((user_id, conn_id)).or_insert((0, Instant::now()));
    if entry.1.elapsed() >= SSH_FAIL_WINDOW {
        *entry = (1, Instant::now());
    } else {
        entry.0 = entry.0.saturating_add(1);
    }
}

/// 认证成功后清零失败计数。
fn clear_ssh_auth_failures(user_id: i64, conn_id: i64) {
    SSH_AUTH_FAILS.lock().unwrap().remove(&(user_id, conn_id));
}

async fn ssh_connect(info: &ConnectionInfo) -> Result<client::Handle<SshClient>, String> {
    let addr = format!("{}:{}", info.host, info.port);
    // 保活：30s 没收到数据就发一次 keepalive，连续 3 次没回应即判定链路已死。
    // 没有它，NAT 超时 / 网络闪断之后这条连接会一直僵着 ——
    // 双方都不发数据，谁也不会先断开，会话就这么悬着。
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });
    // TOFU：库里没记过指纹就先放过去（连成 + 认证成功后由下面落库），记过就必须匹配
    let expected = if info.host_key_fingerprint.is_empty() {
        None
    } else {
        Some(info.host_key_fingerprint.clone())
    };
    let observed = Arc::new(Mutex::new(None));
    let mut handle = match client::connect(
        config,
        addr,
        SshClient {
            expected: expected.clone(),
            observed: Arc::clone(&observed),
        },
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            // 指纹不匹配时 russh 只在握手层报错、看不出是「主机被换掉」，
            // 这里补一句能看懂、也说清怎么办的提示
            let seen = observed.lock().unwrap().clone();
            if expected.is_some()
                && let Some(actual) = seen.as_ref()
            {
                return Err(format!(
                    "SSH 主机密钥指纹不匹配：已记录 {}，实际 {}。\
                     若主机确实重装过 / 换过密钥，请删除并重新添加该连接以重新信任；\
                     否则请警惕中间人攻击",
                    expected.as_deref().unwrap_or_default(),
                    actual
                ));
            }
            return Err(format!("SSH 连接失败: {}", e));
        }
    };

    match info.auth_type.as_str() {
        "password" => {
            let res = handle
                .authenticate_password(&info.username, &info.password)
                .await
                .map_err(|e| format!("密码认证失败: {}", e))?;
            if !res.success() {
                return Err("密码认证失败：用户名或密码错误".to_string());
            }
        }
        "key" => {
            // 私钥已在 load_connection_info 中按归属解析好（家目录 ~/.ssh 文件）
            let key_content = info.ssh_private_key.as_deref().ok_or_else(|| {
                format!(
                    "SSH 密钥 '{}' 不可用：请在「我的密钥」中创建/导入并重新绑定到连接",
                    info.ssh_key_name
                )
            })?;
            // russh 直接吃 OpenSSH 新格式私钥（ed25519 / ECDSA / RSA 均可）
            let key = parse_private_key(key_content)?;
            // 服务端经 server-sig-algs 通告的最优 RSA 哈希；非 RSA 密钥用不到
            let hash_alg = match handle.best_supported_rsa_hash().await {
                Ok(v) => v.flatten(),
                Err(_) => None,
            };
            let res = handle
                .authenticate_publickey(
                    &info.username,
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                )
                .await
                .map_err(|e| format!("密钥认证失败: {}", e))?;
            if !res.success() {
                return Err("密钥认证失败：该主机未授权此密钥".to_string());
            }
        }
        _ => return Err(format!("不支持的认证类型: {}", info.auth_type)),
    }

    // TOFU 落库：此前没记过指纹 **且认证成功**（顺序很重要 —— 握手在认证之前，
    // 没通过认证的对端还不配被信任），把本次看到的指纹写回这条连接。
    // id = 0 是临时连接（「推送公钥」表单，尚未入库），无处记录则跳过。
    let seen = observed.lock().unwrap().clone();
    if expected.is_none() && info.id > 0
        && let Some(fp) = seen.as_ref()
    {
        let pool = db::get_db_pool().await;
        let _ = sqlx::query("UPDATE ssh_connections SET host_key_fingerprint = ? WHERE id = ?")
            .bind(fp)
            .bind(info.id)
            .execute(pool)
            .await;
        info!("SSH 主机密钥已记录（TOFU）: 连接 {} → {}", info.id, fp);
    }
    Ok(handle)
}

struct ConnectionInfo {
    /// 连接 id；`0` = 临时连接（如「推送公钥」表单，尚未入库），TOFU 指纹无处落库
    id: i64,
    host: String,
    port: i32,
    username: String,
    auth_type: String,
    password: String,
    ssh_key_name: String,
    /// 已记录的主机密钥指纹（TOFU）：空 = 尚未信任过该主机密钥，首次连成后写回
    host_key_fingerprint: String,
    /// 已按归属解析好的私钥/公钥内容（key 认证用；在进入阻塞认证前异步加载）
    ssh_private_key: Option<String>,
    ssh_public_key: Option<String>,
}

/// WebSocket 终端 resize 控制消息（前端 fit 后发送），形如
/// `{"type":"resize","cols":120,"rows":40}`。
/// 收到后调用 `Channel::request_pty_size` 向远端发送 window-change，用于动态调整 pty 窗口。
#[derive(Debug, Deserialize)]
struct ResizeMsg {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    cols: u32,
    #[serde(default)]
    rows: u32,
}

/// WebSocket 密码下发消息（未保存密码的连接），形如
/// `{"type":"auth","password":"..."}`，由前端弹窗输入后发送，
/// 仅用于本次会话认证，不会写入数据库。
#[derive(Debug, Deserialize)]
struct AuthMsg {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    password: String,
}

/// 加载连接信息并预解析密钥（家目录 `~/.ssh/zap_<name>`，经 zapexec 读取）。
async fn load_connection_info(id: i64) -> Result<ConnectionInfo, ZapError> {
    let pool = db::get_db_pool().await;
    let row = sqlx::query(
        "SELECT s.id, s.host, s.port, s.username, s.auth_type, s.password, s.ssh_key_name, \
                s.host_key_fingerprint, s.user_id AS owner_id, u.linux_user AS linux_user \
         FROM ssh_connections s LEFT JOIN user u ON u.id = s.user_id \
         WHERE s.id = ? AND s.status = 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            let stored: String = r.try_get("password").unwrap_or_default();
            let auth_type: String = r.get("auth_type");
            let ssh_key_name: String = r.try_get("ssh_key_name").unwrap_or_default();
            let linux_user: String = r.try_get("linux_user").unwrap_or_default();
            let mut info = ConnectionInfo {
                id: r.get("id"),
                host: r.get("host"),
                port: r.get("port"),
                username: r.get("username"),
                auth_type: auth_type.clone(),
                // 解密后用于 SSH 认证；旧明文数据同样兼容
                password: crypto::decrypt_password(&stored),
                ssh_key_name: ssh_key_name.clone(),
                // 存量行可能还没有这一列（升级前插入的），取不到就当作「尚未信任」
                host_key_fingerprint: r.try_get("host_key_fingerprint").unwrap_or_default(),
                ssh_private_key: None,
                ssh_public_key: None,
            };
            if auth_type == "key" {
                // 1) 连接绑定的面板密钥（家目录 ~/.ssh/zap_<name>，经 zapexec 读取）
                // 2) 未绑定或读取失败时，回退到家目录 ~/.ssh 下的默认私钥
                let mut resolved = None;
                if !ssh_key_name.is_empty() {
                    resolved = resolve_key_material(&linux_user, &ssh_key_name).await.ok();
                }
                let (private_key, public_key) =
                    match resolved.or(resolve_default_key(&linux_user).await) {
                        Some(v) => v,
                        None => {
                            return Err(ZapError::New(
                                -1,
                                "未找到可用 SSH 私钥：请在「我的密钥」中创建并绑定到连接，\
                                 或在账号家目录 ~/.ssh 下放置 id_ed25519 / id_ecdsa / id_rsa"
                                    .to_string(),
                            ));
                        }
                    };
                info.ssh_private_key = Some(private_key);
                info.ssh_public_key = Some(public_key);
            }
            Ok(info)
        }
        None => Err(ZapError::New(-1, "连接不存在或已禁用".to_string())),
    }
}

/// 解析连接绑定的密钥内容（私钥 + 公钥）：家目录 `~/.ssh/zap_<name>`
/// （密钥只存文件，root 按需读取）。
async fn resolve_key_material(
    linux_user: &str,
    key_name: &str,
) -> Result<(String, String), ZapError> {
    // 1) 家目录密钥：存在性以磁盘文件为准（库重建不影响），私钥/公钥分别经 zapexec 读取
    if !linux_user.is_empty() {
        let resp = crate::zapexec::call(Request::SshUserKeyPrivateGet {
            linux_user: linux_user.to_string(),
            name: key_name.to_string(),
        })
        .await?;
        if resp.code == 0 {
            let private_key = resp
                .data
                .as_ref()
                .and_then(|d| d.get("private_key"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if !private_key.is_empty() {
                let pub_resp = crate::zapexec::call(Request::SshUserKeyPublicGet {
                    linux_user: linux_user.to_string(),
                    name: key_name.to_string(),
                })
                .await?;
                let public_key = pub_resp
                    .data
                    .as_ref()
                    .and_then(|d| d.get("public_key"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if public_key.is_empty() {
                    return Err(ZapError::New(
                        -1,
                        format!(
                            "密钥 '{key_name}' 的公钥不可用（~/.ssh/zap_{key_name}.pub 缺失或为空）"
                        ),
                    ));
                }
                return Ok((private_key, public_key));
            }
        }
    }
    Err(ZapError::New(
        -1,
        if linux_user.is_empty() {
            format!("连接的归属账号未绑定系统用户，无法读取密钥 '{key_name}'")
        } else {
            format!("SSH 密钥 '{key_name}' 不存在或不可用：请在「我的密钥」中创建/导入并绑定到连接")
        },
    ))
}

/// 从账号家目录 `~/.ssh` 读取默认私钥（id_ed25519 → id_ecdsa → id_rsa）。
///
/// 面板以 zapadm 运行、无权读他人 0600 的私钥，故统一交给 zapexec（root）代读；
/// 只扫描 `linux_user` 自己的家目录，没有就直接失败，绝不回退到其他账户的密钥。
/// 私钥只在本次连接期间留在 zapd 内存中，不落库、不下发前端。
async fn resolve_default_key(linux_user: &str) -> Option<(String, String)> {
    if linux_user.is_empty() {
        return None;
    }
    let resp = crate::zapexec::call(Request::SshUserKeyDefaultGet {
        linux_user: linux_user.to_string(),
    })
    .await
    .ok()?;
    if resp.code != 0 {
        return None;
    }
    let data = resp.data.as_ref()?;
    let private_key = data.get("private_key")?.as_str()?.to_string();
    if private_key.is_empty() {
        return None;
    }
    let public_key = data
        .get("public_key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Some((private_key, public_key))
}

/// 推送公钥前解析公钥内容：本人家目录密钥（`~/.ssh/zap_<name>.pub`）。
async fn resolve_pub_for_push(
    claims: &ValidatedClaims,
    key_name: &str,
) -> Result<String, ZapError> {
    let pool = db::get_db_pool().await;
    let linux_user: String = sqlx::query_scalar("SELECT linux_user FROM user WHERE id = ?")
        .bind(claims.id as i64)
        .fetch_optional(pool)
        .await?
        .unwrap_or_default();
    if !linux_user.is_empty() {
        let resp = crate::zapexec::call(Request::SshUserKeyPublicGet {
            linux_user,
            name: key_name.to_string(),
        })
        .await?;
        if resp.code == 0
            && let Some(p) = resp
                .data
                .as_ref()
                .and_then(|d| d.get("public_key"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        {
            return Ok(p.to_string());
        }
    }
    Err(ZapError::New(
        -1,
        format!("公钥 '{key_name}' 不存在（用户密钥请在「我的密钥」中创建）"),
    ))
}

// ── Test connection ────────────────────────────────────────

#[derive(Deserialize)]
pub struct TestConnectionQuery {
    pub id: i64,
}

pub async fn test_connection(
    claims: ValidatedClaims,
    Query(params): Query<TestConnectionQuery>,
) -> ZapJsonResult {
    connection_in_scope(&claims, params.id).await?;
    // 「测试连接」同样要限流：它是一次完整的认证尝试，也是顺手的爆破入口
    let user_id = claims.id as i64;
    if let Some(wait) = ssh_auth_blocked(user_id, params.id) {
        return Ok(Json(json!({
            "code": 0,
            "success": false,
            "message": format!("认证失败次数过多（{SSH_FAIL_LIMIT} 次），请 {wait} 秒后再试")
        })));
    }
    let conn_info = load_connection_info(params.id).await?;

    match ssh_connect(&conn_info).await {
        Ok(handle) => {
            clear_ssh_auth_failures(user_id, params.id);
            // 只验证「能连上且能认证」，验证完立即断开
            let _ = handle
                .disconnect(Disconnect::ByApplication, "", "English")
                .await;
            Ok(Json(
                json!({ "code": 0, "success": true, "message": "连接成功" }),
            ))
        }
        Err(e) => {
            note_ssh_auth_failure(user_id, params.id);
            Ok(Json(json!({ "code": 0, "success": false, "message": e })))
        }
    }
}

// ── Push public key to host ────────────────────────────────

#[derive(Deserialize)]
pub struct PushKeyPayload {
    pub password: Option<String>,
}

/// 表单直推请求（添加/编辑对话框中「推送公钥」用，连接无需先入库）
#[derive(Deserialize)]
pub struct PushKeyRequest {
    pub host: String,
    pub port: i32,
    pub username: String,
    pub ssh_key_name: String,
    pub password: Option<String>,
}

/// 把字符串包成 shell 单引号字面量（内部单引号按 POSIX 规则转义），
/// 供把公钥安全地拼进远程命令。
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 判断目标主机是否为本地回环（localhost / 127.0.0.1 / ::1）
fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().trim_matches(['[', ']']).to_lowercase();
    h == "localhost" || h == "127.0.0.1" || h == "::1"
}

/// 把连接绑定的公钥推送到主机 ~/.ssh/authorized_keys
///
/// - 本地回环（localhost/127.0.0.1）：直接写入本机系统用户 authorized_keys，
///   需要 root 特权（经 zapexec），仅 admin 角色可操作，无需密码。
/// - 远程主机：使用远程密码做一次性认证（SFTP 写入，密码不保存）。
pub async fn push_key_to_host(
    claims: ValidatedClaims,
    Path(id): Path<i64>,
    Json(payload): Json<PushKeyPayload>,
) -> ZapJsonResult {
    // 归属校验：仅连接归属范围内可推送公钥
    connection_in_scope(&claims, id).await?;
    let conn_info = load_connection_info(id).await?;
    if conn_info.auth_type != "key" {
        return Err(ZapError::New(
            -1,
            "仅密钥认证的连接支持推送公钥".to_string(),
        ));
    }
    if conn_info.ssh_key_name.is_empty() {
        return Err(ZapError::New(-1, "连接未绑定 SSH 密钥".to_string()));
    }
    push_key_core(
        &claims,
        &conn_info.host,
        conn_info.port,
        &conn_info.username,
        &conn_info.ssh_key_name,
        payload.password.as_deref(),
        format!(
            "{}@{}:{}",
            conn_info.username, conn_info.host, conn_info.port
        ),
    )
    .await
}

/// 表单直推（连接尚未入库也可用，供添加/编辑对话框中的「推送公钥」按钮调用）
pub async fn push_key_direct(
    claims: ValidatedClaims,
    Json(payload): Json<PushKeyRequest>,
) -> ZapJsonResult {
    let host = payload.host.trim().trim_matches(['[', ']']).to_string();
    if host.is_empty() {
        return Err(ZapError::New(-1, "主机地址不能为空".to_string()));
    }
    if payload.username.trim().is_empty() {
        return Err(ZapError::New(-1, "用户名不能为空".to_string()));
    }
    if payload.ssh_key_name.is_empty() {
        return Err(ZapError::New(-1, "请选择要推送的 SSH 密钥".to_string()));
    }
    push_key_core(
        &claims,
        &host,
        payload.port,
        &payload.username,
        &payload.ssh_key_name,
        payload.password.as_deref(),
        format!("{}@{}:{}", payload.username, host, payload.port),
    )
    .await
}

/// 推送核心实现：
/// - 本地回环（localhost/127.0.0.1）走 zapexec 写本机系统用户 authorized_keys（仅 admin，无需密码）
/// - 远程主机用密码做一次性认证，经 SFTP 追加公钥
async fn push_key_core(
    claims: &ValidatedClaims,
    host: &str,
    port: i32,
    username: &str,
    ssh_key_name: &str,
    password: Option<&str>,
    target: String,
) -> ZapJsonResult {
    // 按归属解析公钥：本人名下「我的密钥」优先，admin 回退系统级密钥
    let pub_content = resolve_pub_for_push(claims, ssh_key_name).await?;

    // 本地回环主机：root 特权写本机 authorized_keys，仅 admin。
    // 公钥内容由 zapd 鉴权后下发（SshKeyInstallPub），避免 zapadm 直接读用户家目录。
    if is_loopback_host(host) {
        if !crate::zap::jwt::is_admin(claims) {
            return Err(ZapError::New(
                403,
                "仅 admin 角色可以写入本机 SSH 授权".to_string(),
            ));
        }
        let resp = crate::zapexec::call(Request::SshKeyInstallPub {
            username: username.to_string(),
            public_key: pub_content,
        })
        .await?;
        if resp.code != 0 {
            return Err(ZapError::New(resp.code, resp.message));
        }
        audit::log(
            Some(claims),
            None,
            "push_key_local",
            &target,
            "将公钥写入本机用户 authorized_keys",
        )
        .await;
        return Ok(Json(json!({ "code": 0, "message": resp.message })));
    }

    // 远程主机：用一次性密码认证后执行 ssh-copy-id 等价命令追加公钥
    let password = password.ok_or_else(|| ZapError::New(-1, "远程主机密码不能为空".to_string()))?;
    let info = ConnectionInfo {
        // 临时连接：尚未入库，没有可写入指纹的行
        id: 0,
        host: host.to_string(),
        port,
        username: username.to_string(),
        auth_type: "password".to_string(),
        password: password.to_string(),
        ssh_key_name: String::new(),
        host_key_fingerprint: String::new(),
        ssh_private_key: None,
        ssh_public_key: None,
    };
    let handle = ssh_connect(&info).await.map_err(ZapError::Error)?;
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| ZapError::Error(format!("打开 SSH 通道失败: {}", e)))?;

    // 幂等：目录/文件权限就位 → 已含该公钥则跳过 → 否则追加
    let line = shell_single_quote(pub_content.trim());
    let cmd = format!(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && touch ~/.ssh/authorized_keys && \
         chmod 600 ~/.ssh/authorized_keys && \
         grep -qF {line} ~/.ssh/authorized_keys || printf '%s\\n' {line} >> ~/.ssh/authorized_keys"
    );
    channel
        .exec(true, cmd)
        .await
        .map_err(|e| ZapError::Error(format!("远程执行命令失败: {}", e)))?;

    let mut exit_code = None;
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::ExitStatus { exit_status } => {
                exit_code = Some(exit_status);
                break;
            }
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    let _ = channel.close().await;
    let _ = handle
        .disconnect(Disconnect::ByApplication, "", "English")
        .await;

    match exit_code {
        Some(0) => {}
        Some(code) => {
            return Err(ZapError::New(
                -1,
                format!("远程写入 authorized_keys 失败（退出码 {code}）"),
            ));
        }
        None => {
            return Err(ZapError::New(
                -1,
                "远程写入 authorized_keys 失败：未收到退出状态".to_string(),
            ));
        }
    }

    audit::log(
        Some(claims),
        None,
        "push_key",
        &target,
        "推送公钥到远程主机 authorized_keys",
    )
    .await;

    Ok(Json(
        json!({ "code": 0, "message": "公钥已推送到远程主机 ~/.ssh/authorized_keys" }),
    ))
}
