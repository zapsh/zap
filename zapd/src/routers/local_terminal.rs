// SPDX-License-Identifier: AGPL-3.0-only
//! 本地终端：编辑器内嵌 Terminal 的后端入口。
//!
//! zapd 以 `zapadm` 运行、本身没有降权能力，因此真正的 PTY spawn 交给 root 侧的
//! `zapexec`（`Request::HostShellExec`）。这里只做三件事：
//! 1) 校验登录态 / 演示 / 只读 / 套餐能力；
//! 2) 把面板用户解析成对应的 Linux 账号；
//! 3) 在浏览器 WebSocket 与 zapexec 流式会话之间转发字节。
//!
//! 权限边界在 zapexec：它固定启动登录 shell 并 `setuid` 到目标账号，**绝不会返回 root**。
//!
//! 帧协议与容器终端一致：
//! - **Binary**：终端字节流（上行 = stdin，下行 = stdout）
//! - **Text**：上行 `{"type":"resize",..}` / `{"type":"close"}`，其余按按键输入；
//!   下行 `{"type":"exit","code":..}` / `{"type":"error","message":..}`

use std::collections::HashMap;
use std::path::Path;

use axum::extract::Query;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use zap_proto::types::Message as ExecMessage;
use zap_proto::Request;

use crate::db;
use crate::zap::jwt::Claims;

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

fn err_response(status: StatusCode, msg: &str) -> Response {
    Response::builder()
        .status(status)
        .body(axum::body::Body::from(msg.to_string()))
        .unwrap()
}

fn ctrl_text(kind: &str, code: Option<i32>, message: Option<&str>) -> Message {
    let mut v = json!({ "type": kind });
    if let Some(c) = code {
        v["code"] = json!(c);
    }
    if let Some(m) = message {
        v["message"] = json!(m);
    }
    Message::Text(Utf8Bytes::from(v.to_string()))
}

enum Ctrl {
    Resize(u16, u16),
    Close,
}

fn parse_ctrl(s: &str) -> Option<Ctrl> {
    if !s.starts_with('{') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    match v.get("type")?.as_str()? {
        "resize" => {
            let cols = v.get("cols")?.as_u64()? as u16;
            let rows = v.get("rows")?.as_u64()? as u16;
            Some(Ctrl::Resize(cols.clamp(20, 500), rows.clamp(5, 200)))
        }
        "close" => Some(Ctrl::Close),
        _ => None,
    }
}

/// GET /terminal/local/ws?token=..&cwd=..&cols=..&rows=..
pub async fn ws_local(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    // 浏览器 WebSocket 不能带自定义头，token 走 query（与 SSH 终端一致）
    let Some(token) = params.get("token") else {
        return err_response(StatusCode::UNAUTHORIZED, "Missing token");
    };
    let Some(claims) = crate::zap::jwt::decode_verified(token) else {
        return err_response(StatusCode::UNAUTHORIZED, "Invalid token");
    };

    // 准入先算出来，但放到升级之后用「错误帧」回给前端：
    // 浏览器 WebSocket 拿不到 403 的响应体，只有升级后才能把原因显示在终端里。
    let gate = check_access(&claims).await;

    let cwd = params.get("cwd").cloned().unwrap_or_default();
    let cols = params
        .get("cols")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_COLS);
    let rows = params
        .get("rows")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_ROWS);

    ws.on_upgrade(move |socket| async move {
        if let Err(msg) = gate {
            let (mut tx, _) = socket.split();
            let _ = tx.send(ctrl_text("error", None, Some(&msg))).await;
            let _ = tx.close().await;
            return;
        }
        let user = match prepare_target(claims.id as i64).await {
            Ok(u) => u,
            Err(msg) => {
                let (mut tx, _) = socket.split();
                let _ = tx.send(ctrl_text("error", None, Some(&msg))).await;
                let _ = tx.close().await;
                return;
            }
        };
        handle(socket, user, cwd, cols, rows, claims).await;
    })
}

/// 准入判定：演示 / 只读 / 套餐能力。
async fn check_access(claims: &Claims) -> Result<(), String> {
    if crate::zap::jwt::is_demo(claims) {
        return Err("演示账号仅支持浏览，不能使用终端".to_string());
    }
    if crate::routers::access::user_is_read_only(claims.id).await {
        return Err("只读账号不能使用终端".to_string());
    }
    // 套餐能力：非管理员必须显式开启「本地终端」（与 SSH 终端一致，未绑定套餐不限制）
    if !crate::zap::jwt::is_admin(claims)
        && let Some(pkg) = crate::routers::package::package_of_user(claims.id as i64).await
        && pkg.allow_terminal != 1
    {
        return Err(format!(
            "当前套餐「{}」未开启本地终端，请联系服务商变更套餐",
            pkg.name
        ));
    }
    Ok(())
}

async fn load_linux_user(
    pool: &sqlx::SqlitePool,
    user_id: i64,
) -> Result<String, String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT linux_user FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    row.map(|(lu,)| lu)
        .ok_or_else(|| "用户不存在".to_string())
}

/// 解析面板用户对应的 Linux 账号；老账号缺运行实体时补齐。
async fn prepare_target(user_id: i64) -> Result<String, String> {
    let pool = db::get_db_pool().await;
    let mut linux_user = load_linux_user(pool, user_id).await?;
    if linux_user.trim().is_empty() {
        crate::routers::user::ensure_user_runtime(user_id).await?;
        linux_user = load_linux_user(pool, user_id).await?;
    }
    if linux_user.trim().is_empty() {
        return Err(
            "该账号尚未初始化 Linux 运行账号，请先在「用户管理」中重新保存一次".to_string(),
        );
    }
    Ok(linux_user)
}

async fn handle(
    socket: WebSocket,
    user: String,
    cwd: String,
    cols: u16,
    rows: u16,
    claims: Claims,
) {
    let exec_cfg = {
        let cfg = crate::config::get_config().read().unwrap();
        cfg.exec.clone()
    };
    let client = match crate::zapexec::connect(
        Path::new(&exec_cfg.socket_path),
        Path::new(&exec_cfg.secret_path),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            let (mut tx, _) = socket.split();
            let _ = tx
                .send(ctrl_text(
                    "error",
                    None,
                    Some(&format!("终端服务不可用（zapexec 连接失败: {e}）")),
                ))
                .await;
            let _ = tx.close().await;
            return;
        }
    };

    let (mut exec_rd, mut exec_wr) = client.into_parts();
    let stream_id = format!("local-{}", claims.id);

    let open = ExecMessage::StreamOpen {
        id: stream_id.clone(),
        req: Box::new(Request::HostShellExec {
            user: user.clone(),
            cwd: cwd.clone(),
            cols,
            rows,
        }),
    };
    if crate::zapexec::send(&mut exec_wr, &open).await.is_err() {
        let (mut tx, _) = socket.split();
        let _ = tx
            .send(ctrl_text("error", None, Some("无法开启终端会话")))
            .await;
        let _ = tx.close().await;
        return;
    }

    // 读循环独立成任务：`frame::recv` 被 `select!` 取消会丢半帧
    let (frame_tx, mut frame_rx) = mpsc::channel::<ExecMessage>(64);
    tokio::spawn(async move {
        while let Ok(msg) = crate::zapexec::recv(&mut exec_rd).await {
            if frame_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    let (mut ws_tx, mut ws_rx) = socket.split();

    loop {
        tokio::select! {
            // zapexec → 浏览器
            frame = frame_rx.recv() => {
                let Some(frame) = frame else { break };
                match frame {
                    ExecMessage::StreamOut { data, .. } => {
                        let bytes = BASE64.decode(&data).unwrap_or_default();
                        if ws_tx.send(Message::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    ExecMessage::StreamReady { .. } => {}
                    ExecMessage::StreamEnd { code, .. } => {
                        let _ = ws_tx.send(ctrl_text("exit", Some(code), None)).await;
                        break;
                    }
                    ExecMessage::StreamError { message, .. } => {
                        let _ = ws_tx.send(ctrl_text("error", None, Some(&message))).await;
                        break;
                    }
                    _ => {}
                }
            }
            // 浏览器 → zapexec
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        let frame = ExecMessage::StreamIn {
                            id: stream_id.clone(),
                            data: BASE64.encode(&data),
                        };
                        if crate::zapexec::send(&mut exec_wr, &frame).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        let s = text.as_str();
                        match parse_ctrl(s) {
                            Some(Ctrl::Resize(c, r)) => {
                                let frame = ExecMessage::StreamResize {
                                    id: stream_id.clone(),
                                    cols: c,
                                    rows: r,
                                };
                                let _ = crate::zapexec::send(&mut exec_wr, &frame).await;
                            }
                            Some(Ctrl::Close) => {
                                let _ = crate::zapexec::send(
                                    &mut exec_wr,
                                    &ExecMessage::StreamClose {
                                        id: stream_id.clone(),
                                    },
                                )
                                .await;
                                break;
                            }
                            None => {
                                let frame = ExecMessage::StreamIn {
                                    id: stream_id.clone(),
                                    data: BASE64.encode(s.as_bytes()),
                                };
                                if crate::zapexec::send(&mut exec_wr, &frame).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    _ => {}
                }
            }
        }
    }

    // 收尾：通知 zapexec 关闭会话（丢弃 stdin 即让 shell 退出），再关 WS
    let _ = crate::zapexec::send(
        &mut exec_wr,
        &ExecMessage::StreamClose {
            id: stream_id,
        },
    )
    .await;
    let _ = ws_tx.close().await;

    crate::zap::audit::log(
        Some(&claims),
        None,
        "local_terminal_open",
        &format!("user={user}"),
        &format!("cwd={cwd}"),
    )
    .await;
}
