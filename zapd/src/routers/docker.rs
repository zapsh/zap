//! Docker 容器管理端点（面板「容器」：容器 / 镜像 / 卷 / 网络 / Compose）。
//!
//! 端点（全部透传 zapexec 返回，角色门禁见 `access.rs` 的 `/docker` 条目）：
//! - GET  /docker/status                环境探测（是否安装 / daemon / compose）
//! - GET  /docker/containers            容器列表（Query: all）
//! - GET  /docker/stats                 资源占用快照
//! - GET  /docker/container/inspect     容器详情（Query: id）
//! - GET  /docker/container/logs        容器日志（Query: id, tail, since, timestamps）
//! - POST /docker/container/action      容器批量动作（body: ids, action）
//! - GET  /docker/images                镜像列表
//! - POST /docker/image/action          镜像动作 pull / remove / prune（body: id, action）
//! - GET  /docker/volumes               数据卷列表
//! - POST /docker/volume/action         卷动作 create / remove / prune（body: name, action）
//! - GET  /docker/networks              网络列表
//! - POST /docker/network/action        网络动作 create / remove / prune（body: name, action, driver）
//! - GET  /docker/compose               Compose 项目列表
//! - POST /docker/compose/action        Compose 动作 up / down / …（body: project, action）
//!
//! 本模块不做白名单校验之外的业务：容器 / 镜像 / 卷 / 网络的名字原样传给
//! `docker` CLI 作为**被操作对象**，命令本身由 zapexec 的 `verbs/docker.rs` 固定构造。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;

use axum::Json;
use axum::body::Bytes;
use axum::extract::ws::{Message as WsMessage, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Query};
use axum::http::StatusCode;
use axum::response::Response as HttpResponse;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{DecodingKey, Validation, decode};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::{info, warn};
use zap_proto::Request;
use zap_proto::types::Message as ExecMessage;

use crate::config;
use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::jwt::Claims;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;
use crate::zap::jwt::is_demo;

/// 容器操作等价于拿到宿主机的 root，两道闸：必须是 admin，且不是演示账号。
///
/// 第二道不是冗余：`Required::Admin` 只看角色列表里有没有 admin，
/// 而演示账号万一同时带 admin 角色（测试库里很容易配出来），
/// 就会真的删除容器 / 数据卷。这里显式挡掉。
fn require_admin(claims: &ValidatedClaims) -> Result<(), ZapError> {
    if is_demo(claims) {
        return Err(ZapError::New(-1, "演示账号不支持容器操作".to_string()));
    }
    if is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "仅管理员可访问".to_string()))
    }
}

/// 执行一次 zapexec 请求，透传其 data。
async fn exec(req: Request) -> ZapJsonResult {
    let resp = crate::zapexec::call(req).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": resp.data }),
    ))
}

/// 写操作：执行成功后记审计日志。
async fn exec_audited(
    claims: &ValidatedClaims,
    addr: &Extension<SocketAddr>,
    action: &str,
    detail: String,
    req: Request,
) -> ZapJsonResult {
    let result = exec(req).await;
    if result.is_ok() {
        audit::log(
            Some(claims),
            Some(addr.ip().to_string().as_str()),
            action,
            "docker",
            &detail,
        )
        .await;
    }
    result
}

/// GET /docker/status
pub async fn status(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::DockerStatus).await
}

#[derive(Debug, Deserialize)]
pub struct ContainersQuery {
    #[serde(default)]
    pub all: bool,
}

/// GET /docker/containers?all=true
pub async fn containers(
    claims: ValidatedClaims,
    Query(q): Query<ContainersQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::DockerContainers { all: q.all }).await
}

/// GET /docker/stats
pub async fn stats(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::DockerStats).await
}

#[derive(Debug, Deserialize)]
pub struct IdQuery {
    pub id: String,
}

/// GET /docker/container/inspect?id=..
pub async fn container_inspect(claims: ValidatedClaims, Query(q): Query<IdQuery>) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::DockerContainerInspect { id: q.id }).await
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    pub id: String,
    pub tail: Option<u32>,
    pub since: Option<String>,
    #[serde(default)]
    pub timestamps: bool,
}

/// GET /docker/container/logs?id=..&tail=200&since=10m&timestamps=true
pub async fn container_logs(claims: ValidatedClaims, Query(q): Query<LogsQuery>) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::DockerContainerLogs {
        id: q.id,
        tail: q.tail,
        since: q.since,
        timestamps: q.timestamps,
    })
    .await
}

#[derive(Debug, Deserialize)]
pub struct ContainerActionBody {
    pub ids: Vec<String>,
    pub action: String,
}

/// POST /docker/container/action
pub async fn container_action(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<ContainerActionBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if body.ids.is_empty() {
        return Err(ZapError::New(-1, "请先选择容器".to_string()));
    }
    if !matches!(
        body.action.as_str(),
        "start" | "stop" | "restart" | "pause" | "unpause" | "kill" | "remove"
    ) {
        return Err(ZapError::New(-1, "不支持的容器操作".to_string()));
    }
    exec_audited(
        &claims,
        &addr,
        "docker_container_action",
        format!("容器 {} → {}", body.ids.join(", "), body.action),
        Request::DockerContainerAction {
            ids: body.ids.clone(),
            action: body.action.clone(),
        },
    )
    .await
}

/// GET /docker/images
pub async fn images(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::DockerImages).await
}

#[derive(Debug, Deserialize)]
pub struct ImageActionBody {
    pub id: String,
    pub action: String,
}

/// POST /docker/image/action
pub async fn image_action(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<ImageActionBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if !matches!(body.action.as_str(), "pull" | "remove" | "prune") {
        return Err(ZapError::New(-1, "不支持的镜像操作".to_string()));
    }
    exec_audited(
        &claims,
        &addr,
        "docker_image_action",
        format!("镜像 {} → {}", body.id, body.action),
        Request::DockerImageAction {
            id: body.id.clone(),
            action: body.action.clone(),
        },
    )
    .await
}

/// GET /docker/volumes
pub async fn volumes(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::DockerVolumes).await
}

#[derive(Debug, Deserialize)]
pub struct VolumeActionBody {
    pub name: String,
    pub action: String,
}

/// POST /docker/volume/action
pub async fn volume_action(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<VolumeActionBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if !matches!(body.action.as_str(), "create" | "remove" | "prune") {
        return Err(ZapError::New(-1, "不支持的数据卷操作".to_string()));
    }
    exec_audited(
        &claims,
        &addr,
        "docker_volume_action",
        format!("数据卷 {} → {}", body.name, body.action),
        Request::DockerVolumeAction {
            name: body.name.clone(),
            action: body.action.clone(),
        },
    )
    .await
}

/// GET /docker/networks
pub async fn networks(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::DockerNetworks).await
}

#[derive(Debug, Deserialize)]
pub struct NetworkActionBody {
    pub name: String,
    pub action: String,
    pub driver: Option<String>,
}

/// POST /docker/network/action
pub async fn network_action(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<NetworkActionBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if !matches!(body.action.as_str(), "create" | "remove" | "prune") {
        return Err(ZapError::New(-1, "不支持的网络操作".to_string()));
    }
    exec_audited(
        &claims,
        &addr,
        "docker_network_action",
        format!("网络 {} → {}", body.name, body.action),
        Request::DockerNetworkAction {
            name: body.name.clone(),
            action: body.action.clone(),
            driver: body.driver.clone(),
        },
    )
    .await
}

/// GET /docker/compose
pub async fn compose_list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::DockerComposeList).await
}

#[derive(Debug, Deserialize)]
pub struct ComposeActionBody {
    pub project: String,
    pub action: String,
}

/// POST /docker/compose/action
pub async fn compose_action(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<ComposeActionBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if !matches!(
        body.action.as_str(),
        "up" | "down" | "start" | "stop" | "restart" | "pull"
    ) {
        return Err(ZapError::New(-1, "不支持的 Compose 操作".to_string()));
    }
    exec_audited(
        &claims,
        &addr,
        "docker_compose_action",
        format!("Compose 项目 {} → {}", body.project, body.action),
        Request::DockerComposeAction {
            project: body.project.clone(),
            action: body.action.clone(),
        },
    )
    .await
}

// ── WebSocket 容器终端 ─────────────────────────────────────

/// 前端上行控制消息（Text 帧，JSON）。
#[derive(Debug, Deserialize)]
struct ExecCtrl {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    cols: u16,
    #[serde(default)]
    rows: u16,
}

/// 允许的 shell：容器里跑什么解释器只能是这几个，
/// 否则 `shell=...` 就成了往容器里塞任意命令的口子。
const ALLOWED_SHELLS: [&str; 4] = ["sh", "bash", "ash", "zsh"];

/// 容器 ID / 名称的宽松校验：只放行 docker 允许出现的字符。
fn is_safe_ref(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ':'))
}

/// GET /docker/exec/ws —— 容器内交互式终端（`docker exec -it` 的等价物）。
///
/// 帧协议沿用面板 SSH 终端：
/// - **Binary**：终端字节流（上行 = stdin，下行 = stdout）
/// - **Text**：JSON 控制消息
///   - 上行 `{"type":"resize","cols":..,"rows":..}` / `{"type":"close"}`
///   - 下行 `{"type":"ready"}` / `{"type":"exit","code":..}` / `{"type":"error","message":..}`
pub async fn ws_exec(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
) -> HttpResponse {
    // 浏览器 WebSocket 不能带自定义头，token 只能走 query（与 SSH 终端一致）
    let Some(token) = params.get("token").cloned() else {
        return plain_status(StatusCode::UNAUTHORIZED, "Missing token");
    };

    // 克隆密钥后立即释放锁：RwLockReadGuard 非 Send，跨 await 会让 future 非 Send
    let secure_key = config::get_config().read().unwrap().jwt.jwt_secure.clone();
    let claims = match decode::<Claims>(
        &token,
        &DecodingKey::from_secret(secure_key.as_bytes()),
        &Validation::default(),
    ) {
        Ok(d) => d.claims,
        Err(_) => return plain_status(StatusCode::UNAUTHORIZED, "Invalid token"),
    };
    if is_demo(&claims) {
        return plain_status(StatusCode::FORBIDDEN, "演示账号不支持容器终端");
    }
    if !is_admin(&claims) {
        return plain_status(StatusCode::FORBIDDEN, "仅管理员可访问");
    }

    let Some(id) = params
        .get("id")
        .map(|s| s.trim().to_string())
        .filter(|s| is_safe_ref(s))
    else {
        return plain_status(StatusCode::BAD_REQUEST, "容器 ID 非法");
    };
    let shell = params
        .get("shell")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "sh".to_string());
    if !ALLOWED_SHELLS.contains(&shell.as_str()) {
        return plain_status(StatusCode::BAD_REQUEST, "不支持的 shell");
    }

    let cols = params
        .get("cols")
        .and_then(|v| v.parse().ok())
        .unwrap_or(120u16)
        .clamp(20, 500);
    let rows = params
        .get("rows")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30u16)
        .clamp(5, 200);

    ws.on_upgrade(move |socket| handle_exec(socket, id, shell, cols, rows))
}

/// 连接建立后的双向转发：WebSocket ↔ zapexec 流式会话。
async fn handle_exec(socket: WebSocket, container: String, shell: String, cols: u16, rows: u16) {
    info!("容器终端会话建立: {container}");

    let exec_cfg = {
        let cfg = config::get_config().read().unwrap();
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
            warn!("连接 zapexec 失败: {e}");
            return;
        }
    };

    let (mut exec_rd, mut exec_wr) = client.into_parts();
    let stream_id = "exec".to_string();

    // 开启会话
    let open = ExecMessage::StreamOpen {
        id: stream_id.clone(),
        req: Box::new(Request::DockerContainerExec {
            id: container.clone(),
            cmd: vec![shell],
            user: None,
            cols,
            rows,
        }),
    };
    if crate::zapexec::send(&mut exec_wr, &open).await.is_err() {
        warn!("开启容器终端会话失败: {container}");
        return;
    }

    // 读循环独立成任务：`frame::recv` 被 `select!` 取消会丢半帧，
    // 因此不能在 select 分支里直接读，这里用 channel 解耦。
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
                        if ws_tx.send(WsMessage::Binary(Bytes::from(bytes))).await.is_err() {
                            break;
                        }
                    }
                    ExecMessage::StreamReady { .. } => {
                        let _ = ws_tx.send(ctrl_text("ready", None, None)).await;
                    }
                    ExecMessage::StreamEnd { code, .. } => {
                        let _ = ws_tx.send(ctrl_text("exit", Some(code), None)).await;
                        break;
                    }
                    ExecMessage::StreamError { message, .. } => {
                        let _ = ws_tx.send(ctrl_text("error", None, Some(message))).await;
                        break;
                    }
                    _ => {}
                }
            }
            // 浏览器 → zapexec
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(WsMessage::Binary(data))) => {
                        let frame = ExecMessage::StreamIn {
                            id: stream_id.clone(),
                            data: BASE64.encode(&data),
                        };
                        if crate::zapexec::send(&mut exec_wr, &frame).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Text(text))) => match serde_json::from_str::<ExecCtrl>(text.as_ref()) {
                        Ok(ctrl) if ctrl.kind == "resize" => {
                            let frame = ExecMessage::StreamResize {
                                id: stream_id.clone(),
                                cols: ctrl.cols.clamp(20, 500),
                                rows: ctrl.rows.clamp(5, 200),
                            };
                            let _ = crate::zapexec::send(&mut exec_wr, &frame).await;
                        }
                        // 其余（含 `close`）一律按关闭处理
                        _ => {
                            let _ = crate::zapexec::send(
                                &mut exec_wr,
                                &ExecMessage::StreamClose { id: stream_id.clone() },
                            )
                            .await;
                            break;
                        }
                    },
                    // 对端关闭 / 出错 / 流结束
                    Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
                    _ => {}
                }
            }
        }
    }

    // 收尾：通知 zapexec 关闭会话（丢弃其 handle 即关闭 stdin，容器内命令随之退出）
    let _ = crate::zapexec::send(&mut exec_wr, &ExecMessage::StreamClose { id: stream_id }).await;
    let _ = ws_tx.close().await;
    info!("容器终端会话结束: {container}");
}

/// 下行控制消息（Text 帧）。
fn ctrl_text(kind: &str, code: Option<i32>, message: Option<String>) -> WsMessage {
    let mut v = json!({ "type": kind });
    if let Some(c) = code {
        v["code"] = json!(c);
    }
    if let Some(m) = message {
        v["message"] = json!(m);
    }
    WsMessage::Text(Utf8Bytes::from(v.to_string()))
}

fn plain_status(status: StatusCode, body: &str) -> HttpResponse {
    HttpResponse::builder()
        .status(status)
        .body(axum::body::Body::from(body.to_string()))
        .unwrap_or_default()
}
