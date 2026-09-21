use std::net::SocketAddr;
use std::sync::Arc;

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
use tracing::{error, info, warn};

use zap_proto::Request;

use crate::db;
use crate::zap::audit;
use crate::zap::crypto;
use crate::zap::jwt::{Claims, ValidatedClaims};
use crate::zap::{ZapError, ZapJsonResult};

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
    let rows: Vec<sqlx::sqlite::SqliteRow> = sqlx::query(&format!(
        "{sel} WHERE s.user_id = ? ORDER BY s.sort_order, s.id"
    ))
    .bind(claims.id as i64)
    .fetch_all(pool)
    .await?;

    let connections: Vec<SshConnection> = rows.iter().map(row_to_conn).collect();
    Ok(Json(json!({ "code": 0, "data": connections })))
}

pub async fn get_connection(claims: ValidatedClaims, Path(id): Path<i64>) -> ZapJsonResult {
    connection_in_scope(&claims, id).await?;
    let pool = db::get_db_pool().await;
    let row = sqlx::query(&format!(
        "SELECT {CONN_SEL_COLS}, u.username AS owner_name \
         FROM ssh_connections s LEFT JOIN user u ON u.id = s.user_id \
         WHERE s.id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            let mut conn = row_to_conn(&r);
            conn.password = String::new(); // 脱敏
            Ok(Json(json!({ "code": 0, "data": conn })))
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

    // 密码加密后入库，杜绝明文存储
    let encrypted_password = crypto::encrypt_password(&payload.password);

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
        sqlx::query("UPDATE ssh_connections SET host = ?, updated_at = ? WHERE id = ?")
            .bind(&v)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(v) = payload.port {
        sqlx::query("UPDATE ssh_connections SET port = ?, updated_at = ? WHERE id = ?")
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
        // 密码加密后入库
        let encrypted = crypto::encrypt_password(&v);
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

    ws.on_upgrade(move |socket| handle_terminal(socket, id, rows, cols))
}

async fn handle_terminal(socket: WebSocket, conn_id: i64, rows: u32, cols: u32) {
    info!("Terminal WebSocket connected for connection {}", conn_id);

    let conn_info = match load_connection_info(conn_id).await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load connection {}: {}", conn_id, e);
            // 把可读原因直接回显到终端（如 www 模式下「我的密钥」不可用），而非静默断开
            let msg = match &e {
                ZapError::New(_, m) => m.clone(),
                other => other.to_string(),
            };
            send_error_and_close(socket, &format!("{msg}\r\n")).await;
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
                .send(Message::Text(axum::extract::ws::Utf8Bytes::from(
                    "密码输入超时或已取消，连接已关闭\r\n",
                )))
                .await;
            let _ = tx.close().await;
            return;
        }
        temporary_password = Some(pwd);
        (tx, rx)
    } else {
        socket.split()
    };

    // 认证：临时密码优先（空密码连接由前端下发），否则使用库中保存的凭据
    let mut auth_info = conn_info;
    if let Some(pwd) = temporary_password {
        auth_info.password = pwd;
    }
    let handle = match ssh_connect(&auth_info).await {
        Ok(h) => h,
        Err(e) => {
            error!("SSH authentication failed: {}", e);
            let _ = ws_tx
                .send(Message::Text(axum::extract::ws::Utf8Bytes::from(format!(
                    "认证失败: {}\r\n",
                    e
                ))))
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
                .send(Message::Text(axum::extract::ws::Utf8Bytes::from(format!(
                    "打开通道失败: {}\r\n",
                    e
                ))))
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
        let _ = ws_tx
            .send(Message::Text(axum::extract::ws::Utf8Bytes::from(format!(
                "请求 PTY 失败: {}\r\n",
                e
            ))))
            .await;
        let _ = ws_tx.close().await;
        return;
    }

    if let Err(e) = channel.request_shell(true).await {
        error!("Failed to start shell: {}", e);
        let _ = ws_tx
            .send(Message::Text(axum::extract::ws::Utf8Bytes::from(format!(
                "启动 shell 失败: {}\r\n",
                e
            ))))
            .await;
        let _ = ws_tx.close().await;
        return;
    }

    info!("SSH terminal started for connection {}", conn_id);

    // Channel: WebSocket → PTY resize (cols, rows)
    let (resize_tx, mut resize_rx) = tokio::sync::mpsc::channel::<(u32, u32)>(16);

    // 主循环：远端输出 → WebSocket，WebSocket 输入 → 远端。
    // `ssh_writer` 是 'static 的写端，不占用 channel 借用，便于与 wait() 并发。
    let mut ssh_writer = channel.make_writer();

    loop {
        tokio::select! {
            msg = channel.wait() => {
                let Some(msg) = msg else { break };
                match msg {
                    ChannelMsg::Data { data } => {
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
                    }
                    _ => {}
                }
            }
            ws_msg = ws_rx.next() => {
                let Some(Ok(msg)) = ws_msg else { break };
                match msg {
                    Message::Text(t) => {
                        let txt = t.as_ref();
                        // resize 控制消息：前端 fit 后自动同步窗口尺寸
                        if let Ok(resize) = serde_json::from_str::<ResizeMsg>(txt)
                            && resize.kind == "resize"
                            && resize.cols > 0
                            && resize.rows > 0
                        {
                            let _ = resize_tx.send((resize.cols, resize.rows)).await;
                            continue;
                        }
                        // 其余文本一律作为终端输入
                        if ssh_writer.write_all(txt.as_bytes()).await.is_err() {
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
    let _ = sender
        .send(Message::Text(axum::extract::ws::Utf8Bytes::from(
            msg.to_string(),
        )))
        .await;
    let _ = sender.close().await;
}

// ── Authentication ─────────────────────────────────────────

/// russh 客户端事件回调。
///
/// 面板只需要「连上去、开终端 / 执行命令」，不消费服务端主动推送的消息，
/// 因此除 `check_server_key` 外全部使用默认实现。
struct SshClient;

impl client::Handler for SshClient {
    type Error = russh::Error;

    /// 不做 known_hosts 校验：面板里的主机由用户自己录入，
    /// 语义等同于 OpenSSH 首次连接时接受该主机指纹。
    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
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

async fn ssh_connect(info: &ConnectionInfo) -> Result<client::Handle<SshClient>, String> {
    let addr = format!("{}:{}", info.host, info.port);
    let config = Arc::new(client::Config::default());
    let mut handle = client::connect(config, addr, SshClient)
        .await
        .map_err(|e| format!("SSH 连接失败: {}", e))?;

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
    Ok(handle)
}

struct ConnectionInfo {
    host: String,
    port: i32,
    username: String,
    auth_type: String,
    password: String,
    ssh_key_name: String,
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
        "SELECT s.host, s.port, s.username, s.auth_type, s.password, s.ssh_key_name, \
                s.user_id AS owner_id, u.linux_user AS linux_user \
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
                host: r.get("host"),
                port: r.get("port"),
                username: r.get("username"),
                auth_type: auth_type.clone(),
                // 解密后用于 SSH 认证；旧明文数据同样兼容
                password: crypto::decrypt_password(&stored),
                ssh_key_name: ssh_key_name.clone(),
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
    let conn_info = load_connection_info(params.id).await?;

    match ssh_connect(&conn_info).await {
        Ok(handle) => {
            // 只验证「能连上且能认证」，验证完立即断开
            let _ = handle
                .disconnect(Disconnect::ByApplication, "", "English")
                .await;
            Ok(Json(
                json!({ "code": 0, "success": true, "message": "连接成功" }),
            ))
        }
        Err(e) => Ok(Json(json!({ "code": 0, "success": false, "message": e }))),
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
        host: host.to_string(),
        port,
        username: username.to_string(),
        auth_type: "password".to_string(),
        password: password.to_string(),
        ssh_key_name: String::new(),
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
