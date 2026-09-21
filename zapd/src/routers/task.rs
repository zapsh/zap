//! 通用任务队列路由：任务列表 / 详情 / 日志（轮询 + WebSocket）/ 控制（取消 / 暂停 / 继续）。
//!
//! 面板里所有"跑一会儿"的动作都登记在 `task_queue`（应用商店安装与编译、Docker
//! 镜像构建、家目录备份、系统升级、计划任务执行），前端统一从这里读状态、看日志、
//! 发控制指令，不必再各页面各自拼 `/appstore/runs`。
//!
//! 可见范围是**硬边界**：非管理员只能列出 / 查看 / 操作自己的任务（reseller 含名下
//! 客户），与任务日志里可能含脚本输出、绝对路径、凭据回显的现实相匹配。

use axum::{
    Json,
    extract::{
        Extension, Path, Query,
        ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
};
use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, net::SocketAddr};
use tracing::{error, info, warn};

use crate::{
    zap::{
        ZapError, ZapJsonResult, audit,
        jwt::{self, ValidatedClaims},
        task,
    },
    zapexec,
};
use zap_proto::Request;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    /// 任务大类（appstore / docker / backup / system / cron / crontab / site）
    pub kind: Option<String>,
    pub action: Option<String>,
    pub status: Option<String>,
    /// 归属用户；非管理员传了也会被收敛为自己
    pub username: Option<String>,
    pub group_key: Option<String>,
    pub job_key: Option<String>,
    /// 标题 / 对象 / 任务号模糊匹配
    pub keyword: Option<String>,
}

/// 任务列表（按可见范围收敛，新的在前）。
pub async fn list(claims: ValidatedClaims, Query(q): Query<ListQuery>) -> ZapJsonResult {
    let empty_to_none = |s: Option<String>| s.filter(|v| !v.trim().is_empty());
    let filter = task::Filter {
        kind: empty_to_none(q.kind),
        action: empty_to_none(q.action),
        status: empty_to_none(q.status),
        username: empty_to_none(q.username),
        group_key: empty_to_none(q.group_key),
        job_key: empty_to_none(q.job_key),
        keyword: empty_to_none(q.keyword),
    };
    let (rows, total) = task::list_for(
        &claims,
        filter,
        q.page.unwrap_or(1),
        q.page_size.unwrap_or(20),
    )
    .await?;
    let items: Vec<Value> = rows.iter().map(|r| r.to_json()).collect();
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": { "items": items, "total": total } }),
    ))
}

/// 各状态计数（页面角标 / 顶部统计）。
pub async fn stats(claims: ValidatedClaims) -> ZapJsonResult {
    let rows = task::status_counts(&claims).await?;
    let mut data = json!({
        "pending": 0,
        "running": 0,
        "success": 0,
        "failed": 0,
        "canceled": 0,
    });
    for (status, n) in rows {
        if data.get(&status).is_some() {
            data[&status] = json!(n);
        }
    }
    data["total"] = json!(
        data["pending"].as_i64().unwrap_or(0)
            + data["running"].as_i64().unwrap_or(0)
            + data["success"].as_i64().unwrap_or(0)
            + data["failed"].as_i64().unwrap_or(0)
            + data["canceled"].as_i64().unwrap_or(0)
    );
    Ok(Json(json!({ "code": 0, "message": "OK", "data": data })))
}

#[derive(Debug, Deserialize)]
pub struct TaskIdQuery {
    pub task_id: String,
}

/// 单条任务详情（含归属校验）。
pub async fn detail(claims: ValidatedClaims, Query(q): Query<TaskIdQuery>) -> ZapJsonResult {
    let t = task::ensure_access(&claims, &q.task_id).await?;
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": t.to_json() }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub offset: Option<u64>,
}

/// 增量读日志：传上一轮的 `offset` 只取新增部分，`done` 为 true 表示任务已收尾。
pub async fn log(
    claims: ValidatedClaims,
    Path(task_id): Path<String>,
    Query(q): Query<LogQuery>,
) -> ZapJsonResult {
    let t = task::ensure_access(&claims, &task_id).await?;
    let offset = q.offset.unwrap_or(0);
    let (text, exit_code, done) = task::read_log(&t.log_path, offset).await?;
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "content": text,
            "offset": offset + text.len() as u64,
            "exit_code": exit_code,
            "done": done,
            "status": t.status,
        }
    })))
}

// ── 控制：取消 / 暂停 / 继续 ───────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ControlPayload {
    pub task_id: String,
}

/// 取消任务。
///
/// 排队中的任务还没交给执行侧，直接置为已取消；已启动的任务先请执行侧停止
/// （脚本类有停止通道），再落控制位 —— 管理页能立刻看到"已请求取消"。
pub async fn cancel(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ControlPayload>,
) -> ZapJsonResult {
    let t = task::ensure_access(&claims, &payload.task_id).await?;
    if !jwt::is_admin(&claims) && t.username != claims.sub {
        return Err(ZapError::New(-1, "只能取消自己的任务".to_string()));
    }
    if task::is_final(&t.status) {
        return Err(ZapError::New(-1, "任务已结束，无需取消".to_string()));
    }
    if t.status == task::STATUS_PENDING {
        let _ = task::finish(&t.task_id, task::STATUS_CANCELED, -1).await;
    } else {
        if t.kind == task::KIND_APPSTORE {
            match zapexec::call(Request::AppstoreScriptStop {
                run_id: t.task_id.clone(),
            })
            .await
            {
                Ok(resp) if resp.code != 0 => {
                    warn!("停止任务 {} 失败: {}", t.task_id, resp.message);
                }
                Err(e) => warn!("停止任务 {} 失败: {e}", t.task_id),
                Ok(_) => {}
            }
        }
        task::set_control(&t.task_id, task::CONTROL_CANCEL).await?;
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "task_cancel",
        &t.task_id,
        &format!("kind={} action={}", t.kind, t.action),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "已请求取消任务" })))
}

/// 暂停任务（admin）：写控制位，执行侧据此对进程组发 `SIGSTOP`。
///
/// 注意：当前 zapexec 尚未实现信号下达，接口先把意图落库并让管理页可见，
/// 待执行侧接入后自动生效（见 `task::CONTROL_PAUSE`）。
pub async fn pause(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ControlPayload>,
) -> ZapJsonResult {
    set_control(
        &claims,
        client_addr,
        &payload.task_id,
        task::CONTROL_PAUSE,
        "task_pause",
    )
    .await
}

/// 继续任务（admin）：清空暂停指令，执行侧据此发 `SIGCONT`。
pub async fn resume(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ControlPayload>,
) -> ZapJsonResult {
    set_control(&claims, client_addr, &payload.task_id, "", "task_resume").await
}

async fn set_control(
    claims: &ValidatedClaims,
    client_addr: SocketAddr,
    task_id: &str,
    control: &str,
    audit_name: &str,
) -> ZapJsonResult {
    let t = task::ensure_access(claims, task_id).await?;
    if !task::set_control(&t.task_id, control).await? {
        return Err(ZapError::New(-1, "任务已结束，控制指令不生效".to_string()));
    }
    audit::log(
        Some(claims),
        Some(client_addr.ip().to_string().as_str()),
        audit_name,
        &t.task_id,
        &format!(
            "control={}",
            if control.is_empty() {
                "resume"
            } else {
                control
            }
        ),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "已下发控制指令" })))
}

// ── WebSocket 实时日志 ────────────────────────────────────

/// 任务日志实时流（浏览器不能带自定义头，token 走 query）。
///
/// `/appstore/ws/{run_id}` 也指向这里：两套路径、一份实现，老前端不用改。
pub async fn ws_log(
    ws: WebSocketUpgrade,
    Path(task_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(token) = params.get("token").cloned() else {
        return unauthorized("Missing token");
    };
    // 走统一校验入口：签名 + 有效期 + 会话版本号（被「下线所有设备」作废的一并拒绝）
    let claims = match jwt::decode_verified(&token) {
        Some(c) => c,
        None => return unauthorized("Invalid token"),
    };
    // 归属校验：实时日志同样按归属用户隔离，不能凭 id 串看他人任务输出。
    // 校验失败也照常升级，再回一条可读的 error 帧：直接回 403 时浏览器只会触发
    // onerror，前端拿不到任何原因，只能显示含糊的「连接错误」。
    let access = task::ensure_access(&claims, &task_id).await;
    ws.on_upgrade(move |mut socket| async move {
        match access {
            Ok(_) => handle_ws_log(socket, task_id).await,
            Err(e) => {
                let _ = socket
                    .send(Message::Text(Utf8Bytes::from(
                        json!({ "type": "error", "message": e.to_string() }).to_string(),
                    )))
                    .await;
                let _ = socket.close().await;
            }
        }
    })
}

/// WebSocket 握手失败的统一响应（升级前返回，前端表现为连接失败）。
fn unauthorized(message: &str) -> axum::response::Response {
    axum::response::Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(axum::body::Body::from(message.to_string()))
        .unwrap()
}

async fn handle_ws_log(mut socket: WebSocket, task_id: String) {
    info!("task log WebSocket connected: {task_id}");
    let Some(t) = task::get(&task_id).await.unwrap_or(None) else {
        let _ = socket
            .send(Message::Text(Utf8Bytes::from(
                json!({ "type": "error", "message": "任务不存在" }).to_string(),
            )))
            .await;
        return;
    };
    let log_path = t.log_path;
    let mut offset: u64 = 0;

    loop {
        match task::read_log(&log_path, offset).await {
            Ok((text, exit_code, done)) => {
                if !text.is_empty() {
                    // 去掉完成标记行，避免重复展示
                    let clean = if done {
                        task::strip_done_marker(&text)
                    } else {
                        text.clone()
                    };
                    if !clean.is_empty()
                        && socket
                            .send(Message::Text(Utf8Bytes::from(
                                json!({ "type": "log", "data": clean }).to_string(),
                            )))
                            .await
                            .is_err()
                    {
                        return;
                    }
                    offset += text.len() as u64;
                }
                if done {
                    let status = if exit_code == Some(0) {
                        "success"
                    } else {
                        "failed"
                    };
                    let _ = socket
                        .send(Message::Text(Utf8Bytes::from(
                            json!({ "type": "done", "status": status, "exit_code": exit_code })
                                .to_string(),
                        )))
                        .await;
                    let _ = socket.close().await;
                    return;
                }
            }
            Err(e) => {
                error!("read task log {task_id} failed: {e}");
                let _ = socket.close().await;
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}
