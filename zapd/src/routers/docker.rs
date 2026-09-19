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
//! - GET  /docker/image/inspect         镜像详情 inspect + 构建历史（Query: id）
//! - POST /docker/image/build           用 Containerfile 构建镜像（长任务，返回 run_id）
//! - POST /docker/container/run         从镜像快启容器（等价 docker run -d）
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
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{info, warn};
use zap_proto::Request;
use zap_proto::types::Message as ExecMessage;

use crate::config;
use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::appstore as ast;
use crate::zap::audit;
use crate::zap::docker_build;
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

/// 是否持有某个动作级权限点（与访问守卫同源，避免两处判定不一致）。
///
/// 只读账号（`user.read_only`）只看收敛后的生效权限：角色默认权限不再兜底，
/// 否则「角色 ∪ 用户」的并集会让角色自带的权限点整体绕过只读。
async fn has_perm(claims: &ValidatedClaims, key: &str) -> bool {
    if crate::routers::access::user_is_read_only(claims.id).await {
        let user_map = crate::routers::access::user_perm_map().await;
        return crate::routers::access::user_has_perm(user_map.as_ref(), claims.id, key);
    }
    let role_map = crate::routers::access::perm_map().await;
    if crate::routers::access::role_has_perm(role_map.as_ref(), claims, key) {
        return true;
    }
    let user_map = crate::routers::access::user_perm_map().await;
    crate::routers::access::user_has_perm(user_map.as_ref(), claims.id, key)
}

/// 镜像构建的门槛：admin 直通，其余需要**显式授予**的 `docker:build`。
///
/// 为什么不像容器操作那样一刀切限死 admin：多用户面板里，镜像命名空间已经按
/// `<用户名>/...` 隔离（见 `zap::docker_build::scope_tags`），构建上下文也被收敛在
/// 各自家目录内，所以可以按角色放开。但要注意：能构建镜像就等于能跑宿主机的
/// root 命令（Containerfile 的 `RUN` 就是 root shell），因此 `docker:build`
/// **不在任何内置角色的默认权限里**，必须由管理员在「角色权限」里手动勾选。
async fn require_builder(claims: &ValidatedClaims) -> Result<(), ZapError> {
    if is_demo(claims) {
        return Err(ZapError::New(-1, "演示账号不支持镜像相关操作".to_string()));
    }
    if is_admin(claims) || has_perm(claims, "docker:build").await {
        Ok(())
    } else {
        Err(ZapError::New(-1, "未授予镜像构建权限".to_string()))
    }
}

/// GET /docker/images
///
/// 放宽到 `docker:build`：普通用户构建完得能看到自己的镜像。
/// 容器 / 卷 / 网络 / Compose 仍然限 admin（那些操作直接动的是运行中的服务）。
pub async fn images(claims: ValidatedClaims) -> ZapJsonResult {
    require_builder(&claims).await?;
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

#[derive(Debug, Deserialize)]
pub struct ImageInspectQuery {
    pub id: String,
}

/// GET /docker/image/inspect —— 镜像详情：inspect 原文 + 构建历史 + 摘要。
pub async fn image_inspect(
    claims: ValidatedClaims,
    Query(q): Query<ImageInspectQuery>,
) -> ZapJsonResult {
    require_builder(&claims).await?;
    exec(Request::DockerImageInspect { id: q.id }).await
}

#[derive(Debug, Deserialize)]
pub struct ImageBuildArgBody {
    pub key: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct ImageBuildBody {
    /// 构建上下文目录（普通用户必须是自家目录内的绝对路径）
    pub context_dir: String,
    /// Containerfile 路径；留空则自动找上下文里的 Dockerfile / Containerfile
    #[serde(default)]
    pub containerfile: String,
    /// 目标镜像名（可带 tag）。非 admin 会被强制加上 `<用户名>/` 前缀
    pub name: String,
    /// 额外 tag（同一个镜像多打几个名字）
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub build_args: Vec<ImageBuildArgBody>,
    /// 目标平台，如 `linux/amd64`；空 = 跟随宿主架构
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub no_cache: bool,
    #[serde(default)]
    pub pull: bool,
}

/// POST /docker/image/build
///
/// 长任务：登记运行记录后立刻返回 `run_id`，真正干活的是 zapexec 后台进程，
/// 日志实时写 `data/users/<用户名>/docker-build-logs/run-<run_id>.log`，
/// 前端用 `/appstore/ws/{run_id}` 看实时输出。
///
/// 两条多用户边界在这里收敛（实现见 `zap::docker_build`）：
/// 非 admin 的镜像名强制 `<用户名>/...` 命名空间，构建路径限定在自己家目录内。
pub async fn image_build(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<ImageBuildBody>,
) -> ZapJsonResult {
    require_builder(&claims).await?;
    let username = claims.sub.clone();

    // 先做能同步给出的校验：镜像名、路径边界，避免白起一个必然失败的进程
    let tags = docker_build::scope_tags(&claims, &body.name, &body.tags)?;
    let (home_dir, _linux_user) = volume_owner(&claims).await;
    let paths =
        docker_build::resolve_paths(&claims, &home_dir, &body.context_dir, &body.containerfile)?;

    let build_args: Vec<zap_proto::DockerBuildArg> = body
        .build_args
        .iter()
        .filter(|a| !a.key.trim().is_empty())
        .map(|a| zap_proto::DockerBuildArg {
            key: a.key.trim().to_string(),
            value: a.value.clone(),
        })
        .collect();

    let run_id = ast::generate_run_id();
    let log = docker_build::log_path(&username, &run_id);
    let pkg = tags.first().cloned().unwrap_or_default();
    ast::register_run_with_key(
        &run_id,
        docker_build::RUN_ACTION,
        &pkg,
        &username,
        &log,
        &docker_build::job_key(&username),
    )
    .await?;

    let resp = crate::zapexec::call(Request::DockerImageBuild {
        run_id: run_id.clone(),
        log_path: log.clone(),
        context_dir: paths.context.to_string_lossy().into_owned(),
        containerfile: paths.containerfile.to_string_lossy().into_owned(),
        tags: tags.clone(),
        build_args,
        platform: body.platform.trim().to_string(),
        no_cache: body.no_cache,
        pull: body.pull,
    })
    .await?;
    if resp.code != 0 {
        // 起进程失败（docker 缺失、日志目录不可写…）：把记录标失败，别让它挂着
        ast::finish_run(&run_id, "failed", resp.code as i64).await;
        let msg = if resp.message.is_empty() {
            "镜像构建未能启动".to_string()
        } else {
            resp.message
        };
        return Err(ZapError::New(resp.code, msg));
    }
    // 兜底轮询完成标记：没人开日志抽屉时也能把状态落定
    docker_build::watch_run(run_id.clone(), log.clone());

    audit::log(
        Some(&claims),
        Some(addr.ip().to_string().as_str()),
        "docker_image_build",
        &format!(
            "{} ← {} （{}）",
            tags.join(", "),
            paths.context.display(),
            paths.containerfile.display()
        ),
        &run_id,
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": "镜像构建已开始",
        "data": {
            "run_id": run_id,
            "tags": tags,
            "log": log,
            "context_dir": paths.context.to_string_lossy(),
            "containerfile": paths.containerfile.to_string_lossy(),
        }
    })))
}

#[derive(Debug, Deserialize)]
pub struct ContainerRunBody {
    /// 镜像引用（列表里点的那一行）
    pub image: String,
    /// 容器名，留空由 docker 自动起名
    #[serde(default)]
    pub name: String,
    /// 端口映射，如 `8080:80`、`127.0.0.1:8080:80`、`53:53/udp`
    #[serde(default)]
    pub ports: Vec<String>,
    /// 重启策略：空 / no / always / unless-stopped / on-failure
    #[serde(default)]
    pub restart: String,
}

/// POST /docker/container/run —— 镜像列表的「启动」按钮，等价 `docker run -d`。
///
/// 这一步直接让镜像里的进程跑在宿主机内核上（还可能挂载宿主目录），
/// 所以不跟着 `docker:build` 放开，仍然限 admin。
pub async fn container_run(
    claims: ValidatedClaims,
    addr: Extension<SocketAddr>,
    Json(body): Json<ContainerRunBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    exec_audited(
        &claims,
        &addr,
        "docker_container_run",
        format!(
            "镜像 {} → 启动容器 {}（{}）",
            body.image,
            if body.name.is_empty() {
                "自动命名"
            } else {
                body.name.as_str()
            },
            if body.ports.is_empty() {
                "默认网络".to_string()
            } else {
                body.ports.join(", ")
            }
        ),
        Request::DockerContainerRun {
            image: body.image.clone(),
            name: body.name.clone(),
            ports: body.ports.clone(),
            restart: body.restart.clone(),
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
    /// create 时的数据落点：`home`（默认，省略同样按它处理）= 当前账号家目录下的
    /// `volumes/<name>`；`default` = Docker 自己的 `/var/lib/docker/volumes/<name>/_data`。
    ///
    /// 两者各有取舍：账号目录进配额、随 home 备份，但只有该账号（与 root）看得到；
    /// 默认目录由 daemon 托管、多个账号共享，但不进配额也不备份。
    #[serde(default)]
    pub location: String,
}

/// 数据落点：`home` = 账号目录（默认），`default` = Docker 默认目录。
fn volume_location(body: &VolumeActionBody) -> &'static str {
    if body.location.eq_ignore_ascii_case("default") {
        "default"
    } else {
        "home"
    }
}

/// 新建数据卷时的归属：当前登录账号的家目录 + Linux 运行账号。
///
/// 卷数据落在用户自己的 home 下（`{home}/volumes/{name}`），
/// 这样它才进得了该用户的配额、也能跟着 home 一起备份。
///
/// `home_dir` 未配置时回退 `/home/{username}`（与文件管理的私有目录判定一致）；
/// `linux_user` 未配置时按用户名派生（同 `ensure_user_runtime` 的派生规则）。
async fn volume_owner(claims: &ValidatedClaims) -> (String, String) {
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT home_dir, linux_user FROM user WHERE username = ?")
            .bind(&claims.sub)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let (home, linux_user) = row.unwrap_or_default();
    let home = if home.is_empty() {
        format!("/home/{}", claims.sub)
    } else {
        home
    };
    let linux_user = if linux_user.is_empty() {
        zap_proto::linux_username(&claims.sub)
    } else {
        linux_user
    };
    (home, linux_user)
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
    // 只有建在账号目录的新建需要归属（多用户：数据进各自 home）；
    // 选 Docker 默认目录、以及删除 / 清理都与归属无关
    let location = volume_location(&body);
    let (owner_home, owner_user) = if body.action == "create" && location == "home" {
        volume_owner(&claims).await
    } else {
        (String::new(), String::new())
    };
    let location_label = if location == "home" {
        "账号目录"
    } else {
        "Docker 默认目录"
    };
    exec_audited(
        &claims,
        &addr,
        "docker_volume_action",
        format!(
            "数据卷 {} → {}（{}）",
            body.name, body.action, location_label
        ),
        Request::DockerVolumeAction {
            name: body.name.clone(),
            action: body.action.clone(),
            owner_home,
            owner_user,
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

/// WebSocket 升级前的鉴权：管理员且非演示账号。
///
/// 浏览器 WebSocket 不能带自定义头，token 只能走 query（与 SSH 终端一致）；
/// 门禁中间件同样接受 query token，这里是第二道校验。
#[allow(clippy::result_large_err)] // axum 的 Response 本身体量就大，同 access.rs 的 guard
fn ws_guard(params: &HashMap<String, String>) -> Result<(), HttpResponse> {
    let Some(token) = params.get("token") else {
        return Err(plain_status(StatusCode::UNAUTHORIZED, "Missing token"));
    };

    // 克隆密钥后立即释放锁：RwLockReadGuard 非 Send，跨 await 会让 future 非 Send
    let secure_key = config::get_config().read().unwrap().jwt.jwt_secure.clone();
    let claims = match decode::<Claims>(
        token,
        &DecodingKey::from_secret(secure_key.as_bytes()),
        &Validation::default(),
    ) {
        Ok(d) => d.claims,
        Err(_) => return Err(plain_status(StatusCode::UNAUTHORIZED, "Invalid token")),
    };
    if is_demo(&claims) {
        return Err(plain_status(StatusCode::FORBIDDEN, "演示账号不支持该操作"));
    }
    if !is_admin(&claims) {
        return Err(plain_status(StatusCode::FORBIDDEN, "仅管理员可访问"));
    }
    Ok(())
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
    if let Err(resp) = ws_guard(&params) {
        return resp;
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

// ── WebSocket 守护事件流 ───────────────────────────────────

/// GET /docker/events/ws —— 守护进程实时事件流（`docker events` 的等价物）。
///
/// 只有下行，全部是 **Text 帧**：
/// - `{"kind":"event","data":{"type":"container","action":"start",...}}`
/// - `{"kind":"ready"}` / `{"kind":"end"}` / `{"kind":"error","message":".."}`
///
/// 事件载荷自身也带 `type` 字段，控制消息因此统一用 `kind` 区分，避免歧义。
pub async fn ws_events(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
) -> HttpResponse {
    if let Err(resp) = ws_guard(&params) {
        return resp;
    }
    ws.on_upgrade(handle_events)
}

/// 把 zapexec 推来的事件流转成 WebSocket 文本帧。
async fn handle_events(socket: WebSocket) {
    info!("Docker 事件流建立");

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
    let stream_id = "events".to_string();

    let open = ExecMessage::StreamOpen {
        id: stream_id.clone(),
        req: Box::new(Request::DockerEvents),
    };
    if crate::zapexec::send(&mut exec_wr, &open).await.is_err() {
        warn!("开启 Docker 事件流失败");
        return;
    }

    // 同 exec 终端：读循环独立成任务，避免 `frame::recv` 被 select 取消丢半帧
    let (frame_tx, mut frame_rx) = mpsc::channel::<ExecMessage>(128);
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
            frame = frame_rx.recv() => {
                let Some(frame) = frame else { break };
                match frame {
                    ExecMessage::StreamOut { data, .. } => {
                        let text = String::from_utf8(BASE64.decode(&data).unwrap_or_default())
                            .unwrap_or_default();
                        // 事件载荷就是 JSON，解析后再包装，避免手工拼字符串出错
                        let payload = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
                        let msg = json!({ "kind": "event", "data": payload });
                        if ws_tx.send(WsMessage::Text(Utf8Bytes::from(msg.to_string()))).await.is_err() {
                            break;
                        }
                    }
                    ExecMessage::StreamReady { .. } => {
                        let _ = ws_tx.send(ctrl_text("ready", None, None)).await;
                    }
                    ExecMessage::StreamEnd { .. } => {
                        let _ = ws_tx.send(ctrl_text("end", None, None)).await;
                        break;
                    }
                    ExecMessage::StreamError { message, .. } => {
                        let _ = ws_tx.send(ctrl_text("error", None, Some(message))).await;
                        break;
                    }
                    _ => {}
                }
            }
            // 事件流没有上行数据：收到任何文本帧都视为关闭
            msg = ws_rx.next() => match msg {
                Some(Ok(WsMessage::Text(_))) | Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => {
                    let _ = crate::zapexec::send(
                        &mut exec_wr,
                        &ExecMessage::StreamClose { id: stream_id.clone() },
                    )
                    .await;
                    break;
                }
                _ => {}
            },
        }
    }

    let _ = crate::zapexec::send(&mut exec_wr, &ExecMessage::StreamClose { id: stream_id }).await;
    let _ = ws_tx.close().await;
    info!("Docker 事件流结束");
}
