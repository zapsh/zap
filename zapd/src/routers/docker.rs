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

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{Extension, Query};
use serde::Deserialize;
use serde_json::json;

use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;
use crate::zap::jwt::is_demo;
use zap_proto::Request;

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
