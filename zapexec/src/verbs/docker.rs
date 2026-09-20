//! Docker 容器管理动词（面板「容器」页面的执行后端）。
//!
//! 走 **Docker Engine API**（[`bollard`]）而不是 `docker` CLI：
//! - 不要求宿主机安装 docker 客户端，只要 daemon 与其 socket 可达（zapexec 以 root 运行）
//! - 结构化数据直接来自 Engine API，不解析 `--format` 文本，不受 CLI 版本/列宽差异影响
//! - exec 终端能拿到真正的双向流（见 [`crate::verbs::docker_exec`]）
//! - 依旧是白名单动作：容器 / 镜像 id 只作为**被操作对象**传入，不参与命令构造
//!
//! 例外：`docker compose` 是 CLI 插件，Engine API 没有对应端点，
//! 因此 compose 段仍调用 CLI，且只调用固定子命令（见文件末尾）。

use std::collections::HashMap;
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use bollard::API_DEFAULT_VERSION;
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::errors::Error as DockerError;
use bollard::models::{NetworkCreateRequest, Volume, VolumeCreateRequest};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, KillContainerOptions, ListContainersOptions,
    ListImagesOptions, ListNetworksOptions, ListVolumesOptions, LogsOptions, PruneImagesOptions,
    PruneVolumesOptions, RemoveContainerOptions, RemoveImageOptions, RemoveVolumeOptions,
    RestartContainerOptions, StatsOptions, StopContainerOptions,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::timeout;
use zap_proto::{DockerBuildArg, Response, normalize_image_tag, valid_image_ref};

/// 把日志 / exec 的输出帧统一取出字节（三种变体都是同一载荷）。
pub fn log_bytes(chunk: LogOutput) -> Vec<u8> {
    let (LogOutput::StdOut { message }
    | LogOutput::StdErr { message }
    | LogOutput::Console { message }) = chunk
    else {
        return Vec::new();
    };
    message.to_vec()
}

/// 默认 daemon socket；可用 `DOCKER_HOST` 覆盖（`unix://` / `tcp://`）。
const DEFAULT_SOCKET: &str = "/var/run/docker.sock";
/// 查询类调用超时
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// 写动作超时（启停 / 删除 / 创建）
const ACTION_TIMEOUT: Duration = Duration::from_secs(60);
/// 耗时动作超时（镜像拉取）
const LONG_TIMEOUT: Duration = Duration::from_secs(600);
/// compose 动作超时（up / pull 需要拉镜像）
const COMPOSE_TIMEOUT: Duration = Duration::from_secs(600);

static DOCKER: OnceLock<Docker> = OnceLock::new();

/// 取 daemon 客户端（进程内复用一个连接池）。
fn docker() -> Result<&'static Docker, String> {
    client()
}

/// 对外暴露的客户端（exec 会话复用到同一个连接池）。
pub fn client() -> Result<&'static Docker, String> {
    if let Some(d) = DOCKER.get() {
        return Ok(d);
    }
    let host = std::env::var("DOCKER_HOST").unwrap_or_default();
    let d = if let Some(path) = host.strip_prefix("unix://") {
        Docker::connect_with_unix(path, CALL_TIMEOUT.as_secs(), API_DEFAULT_VERSION)
    } else if !host.is_empty() {
        Docker::connect_with_host(&host)
    } else {
        Docker::connect_with_unix(DEFAULT_SOCKET, CALL_TIMEOUT.as_secs(), API_DEFAULT_VERSION)
    }
    .map_err(|e| format!("连接 Docker daemon 失败: {e}"))?;
    Ok(DOCKER.get_or_init(|| d))
}

/// daemon socket 路径（用于「是否已安装 / 可访问」探测）。
fn socket_path() -> String {
    let host = std::env::var("DOCKER_HOST").unwrap_or_default();
    if let Some(p) = host.strip_prefix("unix://") {
        return p.to_string();
    }
    DEFAULT_SOCKET.to_string()
}

/// 统一超时 + 错误文案：bollard 的错误里已带 daemon 给出的原始原因。
async fn call<T, F>(dur: Duration, what: &str, fut: F) -> Result<T, String>
where
    F: Future<Output = Result<T, DockerError>>,
{
    match timeout(dur, fut).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("{what}失败: {e}")),
        Err(_) => Err(format!("{what}超时")),
    }
}

/// 字节数转人类可读（与 `docker ls` 的观感保持一致）。
fn human_size(n: u64) -> String {
    const KB: f64 = 1024.0;
    let b = n as f64;
    if b >= KB * KB * KB {
        format!("{:.2}GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.2}MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.2}KB", b / KB)
    } else {
        format!("{n}B")
    }
}

/// unix 秒 → 本地时间字符串。
fn fmt_time(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

/// 把任意 serde 值转成字符串（用于枚举字段，避免绑定具体模型类型）。
fn as_text(v: &impl serde::Serialize) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|x| x.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// 列表响应：{"items": [...]}。
fn list_response(result: Result<Vec<Value>, String>) -> Response {
    match result {
        Ok(items) => Response::ok("ok", Some(json!({ "items": items }))),
        Err(e) => Response::err(-1, e),
    }
}

/// 动作响应：{"output": ...}。
fn action_response(result: Result<String, String>) -> Response {
    match result {
        Ok(output) => Response::ok("ok", Some(json!({ "output": output }))),
        Err(e) => Response::err(-1, e),
    }
}

// ── 环境探测 ──────────────────────────────────────────────

/// compose 插件是否可用（`docker compose version --short`）。
///
/// compose 是 CLI 插件，Engine API 没有等价端点，只能这样探测。
async fn compose_available() -> bool {
    let Ok(out) = timeout(
        CALL_TIMEOUT,
        docker_cmd(["compose", "version", "--short"]).output(),
    )
    .await
    else {
        return false;
    };
    matches!(out, Ok(o) if o.status.success())
}

/// GET 环境状态：socket 是否可达、daemon 是否在跑、compose 插件是否可用。
///
/// 探测失败也是**正常返回**（`installed` / `daemon` = false），前端据此展示引导，
/// 而不是弹一个看不懂的错误。
pub async fn status() -> Response {
    // socket 不存在基本等于「没装 Docker / 没启动」，先给出这个判断，
    // 再尝试连一次拿到真实原因（权限被拒、daemon 未启动…）。
    let installed = std::path::Path::new(&socket_path()).exists();

    let version = match docker() {
        Ok(d) => call(CALL_TIMEOUT, "读取 Docker 版本", d.version()).await,
        Err(e) => Err(e),
    };

    match version {
        Ok(v) => Response::ok(
            "ok",
            Some(json!({
                "installed": true,
                "daemon": true,
                "version": v.version,
                "api_version": v.api_version,
                "compose": compose_available().await,
                "error": "",
            })),
        ),
        Err(e) => Response::ok(
            "ok",
            Some(json!({
                "installed": installed,
                "daemon": false,
                "version": "",
                "api_version": "",
                "compose": false,
                "error": e,
            })),
        ),
    }
}

// ── 容器 ─────────────────────────────────────────────────

/// 端口映射的文本形式（`0.0.0.0:8080->80/tcp`），无宿主机端口时只显示容器端口。
fn ports_text(ports: &Option<Vec<bollard::models::PortSummary>>) -> String {
    let Some(ports) = ports else {
        return String::new();
    };
    ports
        .iter()
        .map(|p| {
            let proto = as_text(&p.typ);
            match (p.ip.as_deref().filter(|s| !s.is_empty()), p.public_port) {
                (Some(ip), Some(public)) => {
                    format!("{ip}:{public}->{}/{}", p.private_port, proto)
                }
                (None, Some(public)) => format!("{public}->{}/{}", p.private_port, proto),
                _ => format!("{}/{}", p.private_port, proto),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 容器列表（`all = true` 含已停止的）。
///
/// 额外补 `project` 字段：从 compose label 取所属项目，供前端分组筛选。
pub async fn containers(all: bool) -> Response {
    let result = async {
        let d = docker()?;
        let list = call(
            CALL_TIMEOUT,
            "查询容器列表",
            d.list_containers(Some(ListContainersOptions {
                all,
                ..Default::default()
            })),
        )
        .await?;

        Ok(list
            .iter()
            .map(|c| {
                // 名称带前导 `/`（历史原因），去掉后前端直接展示
                let names: Vec<String> = c
                    .names
                    .clone()
                    .unwrap_or_default()
                    .iter()
                    .map(|n| n.trim_start_matches('/').to_string())
                    .collect();
                let project = c
                    .labels
                    .as_ref()
                    .and_then(|m| m.get("com.docker.compose.project"))
                    .cloned()
                    .unwrap_or_default();
                json!({
                    "ID": c.id,
                    "Names": names.join(","),
                    "Image": c.image,
                    "State": as_text(&c.state),
                    "Status": c.status,
                    "Ports": ports_text(&c.ports),
                    "CreatedAt": c.created.map(fmt_time).unwrap_or_default(),
                    "project": project,
                })
            })
            .collect::<Vec<Value>>())
    }
    .await;
    list_response(result)
}

/// 单个容器动作（白名单）。
async fn container_one(id: &str, action: &str) -> Result<String, String> {
    let d = docker()?;
    match action {
        "start" => call(ACTION_TIMEOUT, "启动容器", d.start_container(id, None)).await,
        // 先发 SIGTERM 等 10 秒，超时才由 daemon 强杀；`signal` 留空即走默认行为
        "stop" => {
            call(
                ACTION_TIMEOUT,
                "停止容器",
                d.stop_container(
                    id,
                    Some(StopContainerOptions {
                        t: Some(10),
                        ..Default::default()
                    }),
                ),
            )
            .await
        }
        "restart" => {
            call(
                ACTION_TIMEOUT,
                "重启容器",
                d.restart_container(
                    id,
                    Some(RestartContainerOptions {
                        t: Some(10),
                        ..Default::default()
                    }),
                ),
            )
            .await
        }
        "pause" => call(ACTION_TIMEOUT, "暂停容器", d.pause_container(id)).await,
        "unpause" => call(ACTION_TIMEOUT, "恢复容器", d.unpause_container(id)).await,
        "kill" => {
            call(
                ACTION_TIMEOUT,
                "强制停止容器",
                d.kill_container(
                    id,
                    Some(KillContainerOptions {
                        signal: "SIGKILL".to_string(),
                    }),
                ),
            )
            .await
        }
        // 面板上删除的多半是运行中的容器，force 省掉「先停再删」两步
        "remove" => {
            call(
                ACTION_TIMEOUT,
                "删除容器",
                d.remove_container(
                    id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                ),
            )
            .await
        }
        _ => Err(format!("不支持的容器操作: {action}")),
    }?;
    Ok(id.to_string())
}

/// 批量容器动作：逐个执行，返回失败明细（一个失败不影响其余）。
pub async fn container_action(ids: &[String], action: &str) -> Response {
    let mut failed: Vec<Value> = Vec::new();
    let mut succeeded = 0usize;

    for id in ids {
        match container_one(id, action).await {
            Ok(_) => succeeded += 1,
            Err(e) => failed.push(json!({ "id": id, "error": e })),
        }
    }

    Response::ok(
        "ok",
        Some(json!({ "succeeded": succeeded, "failed": failed })),
    )
}

/// 容器详情：透传 `inspect` 的原始 JSON 对象。
pub async fn container_inspect(id: &str) -> Response {
    let result = async {
        let d = docker()?;
        let info = call(CALL_TIMEOUT, "查询容器详情", d.inspect_container(id, None)).await?;
        serde_json::to_value(info).map_err(|e| format!("序列化容器详情失败: {e}"))
    }
    .await;
    match result {
        Ok(v) => Response::ok("ok", Some(v)),
        Err(e) => Response::err(-1, e),
    }
}

/// 把 `--since` 的相对时间（10m / 1h / 24h）换算成 unix 秒。
fn since_to_unix(since: &str) -> i32 {
    let s = since.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let Ok(n) = num.parse::<i64>() else {
        return 0;
    };
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86400,
        _ => n * 60, // 无单位按分钟处理，与 docker CLI 的默认行为一致
    };
    (chrono::Utc::now().timestamp() - secs) as i32
}

/// 容器日志尾部（一次性拉取；前端按需轮询实现跟随）。
pub async fn container_logs(
    id: &str,
    tail: Option<u32>,
    since: Option<&str>,
    timestamps: bool,
) -> Response {
    let result = async {
        let d = docker()?;
        let opts = LogsOptions {
            stdout: true,
            stderr: true,
            timestamps,
            tail: tail.unwrap_or(200).to_string(),
            since: since.map(since_to_unix).unwrap_or(0),
            ..Default::default()
        };
        let mut stream = d.logs(id, Some(opts));
        let mut buf: Vec<u8> = Vec::new();
        while let Some(item) = timeout(CALL_TIMEOUT, stream.next())
            .await
            .map_err(|_| "读取容器日志超时".to_string())?
        {
            // 只要进程还在输出就续等，超时按「单帧等待」计
            match item {
                Ok(chunk) => buf.extend_from_slice(&log_bytes(chunk)),
                Err(e) => return Err(format!("读取容器日志失败: {e}")),
            }
        }
        Ok(String::from_utf8_lossy(&buf).to_string())
    }
    .await;
    match result {
        Ok(log) => Response::ok("ok", Some(json!({ "log": log }))),
        Err(e) => Response::err(-1, e),
    }
}

/// 实时资源快照：对每个运行中的容器取一次 `stats`（等价 `docker stats --no-stream`）。
///
/// CPU 百分比按「自容器启动以来的累计占用」计算——与 `docker stats --no-stream`
/// 的首帧口径一致（CLI 也是拿不到上一次采样，只能算累计值）。
pub async fn stats() -> Response {
    let result = async {
        let d = docker()?;
        let running = call(
            CALL_TIMEOUT,
            "查询运行中的容器",
            d.list_containers(Some(ListContainersOptions::default())),
        )
        .await?;

        let mut tasks = tokio::task::JoinSet::new();
        for c in running {
            let id = match c.id.clone() {
                Some(id) => id,
                None => continue,
            };
            let name = c
                .names
                .clone()
                .unwrap_or_default()
                .first()
                .map(|n| n.trim_start_matches('/').to_string())
                .unwrap_or_else(|| id.clone());
            let d = d.clone();
            tasks.spawn(async move {
                let mut stream = d.stats(
                    &id,
                    Some(StatsOptions {
                        stream: false,
                        one_shot: false,
                    }),
                );
                let one = stream.next().await;
                (name, one)
            });
        }

        let mut items: Vec<Value> = Vec::new();
        while let Some(res) = tasks.join_next().await {
            let Ok((name, one)) = res else { continue };
            let Some(Ok(s)) = one else { continue };
            // 走 JSON 取值：不绑定具体模型字段，API 版本差异不会编译失败
            let v = serde_json::to_value(&s).unwrap_or_default();
            let cpu = {
                let total = v["cpu_stats"]["cpu_usage"]["total_usage"]
                    .as_u64()
                    .unwrap_or(0);
                let system = v["cpu_stats"]["system_cpu_usage"].as_u64().unwrap_or(0);
                let cpus = v["cpu_stats"]["online_cpus"].as_u64().unwrap_or(1).max(1);
                if system > 0 {
                    total as f64 / system as f64 * cpus as f64 * 100.0
                } else {
                    0.0
                }
            };
            let mem = v["memory_stats"]["usage"].as_u64().unwrap_or(0);
            let limit = v["memory_stats"]["limit"].as_u64().unwrap_or(0);
            let mem_perc = if limit > 0 {
                mem as f64 / limit as f64 * 100.0
            } else {
                0.0
            };
            items.push(json!({
                "Name": name,
                "CPUPerc": format!("{cpu:.2}%"),
                "MemUsage": format!("{} / {}", human_size(mem), human_size(limit)),
                "MemPerc": format!("{mem_perc:.2}%"),
            }));
        }
        Ok(items)
    }
    .await;
    list_response(result)
}

// ── 镜像 ─────────────────────────────────────────────────

/// 镜像列表（含悬空镜像）。
pub async fn images() -> Response {
    let result = async {
        let d = docker()?;
        let list = call(
            CALL_TIMEOUT,
            "查询镜像列表",
            d.list_images(Some(ListImagesOptions {
                all: true,
                ..Default::default()
            })),
        )
        .await?;
        // API 不直接给「被多少容器使用」，用容器列表统计
        let containers = call(
            CALL_TIMEOUT,
            "查询容器列表",
            d.list_containers(Some(ListContainersOptions {
                all: true,
                ..Default::default()
            })),
        )
        .await?;

        // 一个镜像 ID 可以挂多个名字（`docker tag`、构建时多个 `-t`）：
        // daemon 只返回一条记录（repo_tags 里带着全部名字），若只取第一个，
        // 其余名字在面板里就凭空消失了 —— 故每个名字单独一行，与 `docker images` 观感一致。
        let mut items: Vec<Value> = Vec::new();
        for img in list.iter() {
            let short = img.id.strip_prefix("sha256:").unwrap_or(img.id.as_str());
            let in_use = containers
                .iter()
                .filter(|c| {
                    c.image_id.as_deref() == Some(img.id.as_str())
                        || c.image
                            .as_deref()
                            .map(|s| {
                                s == short
                                    || s.strip_prefix("sha256:")
                                        .unwrap_or(s)
                                        .starts_with(&short[..short.len().min(12)])
                            })
                            .unwrap_or(false)
                })
                .count();
            let size = human_size(img.size as u64);
            let created = fmt_time(img.created);
            let named: Vec<&String> = img
                .repo_tags
                .iter()
                .filter(|t| !t.is_empty() && !t.starts_with("<none>"))
                .collect();

            // 悬空镜像：没有任何名字，ID 就是它唯一的引用
            if named.is_empty() {
                items.push(json!({
                    "ID": img.id,
                    "Repository": "<none>",
                    "Tag": "<none>",
                    "Ref": img.id,
                    "SharedTags": 0,
                    "Size": size,
                    "CreatedAt": created,
                    "Containers": in_use.to_string(),
                }));
                continue;
            }

            for full in named.iter() {
                // `host:5000/nginx:latest` 这类带端口的引用从最后一个冒号切，
                // 仓库名里的冒号不能被当成 tag 分隔符
                let (repo, tag) = full
                    .rsplit_once(':')
                    .map(|(r, t)| (r.to_string(), t.to_string()))
                    .unwrap_or_else(|| ((*full).clone(), "latest".to_string()));
                items.push(json!({
                    "ID": img.id,
                    "Repository": repo,
                    "Tag": tag,
                    // 删除 / 运行都按「名字」走：同一个 ID 有多个名字时，
                    // 用 ID 删除会把其余名字一起干掉（force 下更是不留余地）
                    "Ref": *full,
                    "SharedTags": named.len(),
                    "Size": size,
                    "CreatedAt": created,
                    "Containers": in_use.to_string(),
                }));
            }
        }
        Ok(items)
    }
    .await;
    list_response(result)
}

/// 镜像动作：pull（引用）/ remove（镜像 ID 或引用）/ prune（清理悬空镜像）。
pub async fn image_action(id: &str, action: &str) -> Response {
    let result = async {
        let d = docker()?;
        match action {
            "pull" => {
                let (from_image, tag) = match id.rsplit_once(':') {
                    // 注意区分 `nginx:latest` 与 `host:5000/nginx`（后者带端口，不是 tag）
                    Some((img, t)) if !t.contains('/') => (img.to_string(), t.to_string()),
                    _ => (id.to_string(), "latest".to_string()),
                };
                let mut stream = d.create_image(
                    Some(CreateImageOptions {
                        from_image: Some(from_image),
                        tag: Some(tag),
                        ..Default::default()
                    }),
                    None,
                    None,
                );
                let mut last = String::new();
                while let Some(item) = timeout(LONG_TIMEOUT, stream.next())
                    .await
                    .map_err(|_| "拉取镜像超时".to_string())?
                {
                    match item {
                        Ok(info) => {
                            if let Some(status) = info.status {
                                last = status;
                            }
                        }
                        Err(e) => return Err(format!("拉取镜像失败: {e}")),
                    }
                }
                Ok(last)
            }
            "remove" => {
                call(
                    ACTION_TIMEOUT,
                    "删除镜像",
                    d.remove_image(
                        id,
                        Some(RemoveImageOptions {
                            force: true,
                            ..Default::default()
                        }),
                        None,
                    ),
                )
                .await?;
                Ok(String::new())
            }
            "prune" => {
                let mut filters = HashMap::new();
                filters.insert("dangling".to_string(), vec!["true".to_string()]);
                let res = call(
                    ACTION_TIMEOUT,
                    "清理悬空镜像",
                    d.prune_images(Some(PruneImagesOptions {
                        filters: Some(filters),
                    })),
                )
                .await?;
                let deleted = res.images_deleted.map(|v| v.len()).unwrap_or(0);
                Ok(format!(
                    "已清理 {deleted} 个悬空镜像，释放 {}",
                    human_size(res.space_reclaimed.unwrap_or(0) as u64)
                ))
            }
            _ => Err(format!("不支持的镜像操作: {action}")),
        }
    }
    .await;
    action_response(result)
}

// ── 容器快启 / 镜像详情 / 镜像构建 ───────────────────────

/// 容器名白名单：字母或数字开头，其余为字母数字与 `. _ -`，最长 63。
fn valid_container_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// 解析端口映射：`[IP:]宿主机端口:容器端口[/tcp|udp|sctp]`。
///
/// 宿主机端口允许留空（由 daemon 随机分配），与 `docker run -p` 的语义保持一致。
/// 返回 `(容器侧 "port/proto", PortBinding)`。
fn parse_port_spec(spec: &str) -> Result<(String, bollard::models::PortBinding), String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("端口映射不能为空".into());
    }
    let (body, proto) = match spec.split_once('/') {
        Some((b, p)) => {
            let p = p.to_ascii_lowercase();
            if !matches!(p.as_str(), "tcp" | "udp" | "sctp") {
                return Err(format!("不支持的端口协议: {spec}"));
            }
            (b, p)
        }
        None => (spec, "tcp".to_string()),
    };
    let parts: Vec<&str> = body.split(':').collect();
    let (ip, host, container) = match parts.as_slice() {
        // 只写容器端口时宿主机端口留空，由 daemon 随机分配（同 `docker run -p 80`）
        [c] => (None, "", *c),
        [h, c] => (None, *h, *c),
        [i, h, c] => (Some(*i), *h, *c),
        _ => {
            return Err(format!(
                "端口映射格式应为 [IP:]宿主机端口:容器端口，收到: {spec}"
            ));
        }
    };
    let container = container.trim();
    if container.is_empty() || container.parse::<u16>().is_err() {
        return Err(format!("容器端口不合法: {spec}"));
    }
    let host = host.trim();
    if !host.is_empty() && host.parse::<u16>().is_err() {
        return Err(format!("宿主机端口不合法: {spec}"));
    }
    if let Some(ip) = ip {
        let ip = ip.trim();
        if ip.is_empty() {
            return Err(format!("绑定地址不合法: {spec}"));
        }
        // 只接受点分 IPv4 / 简单 IPv6，避免塞入奇怪字符
        if !ip
            .chars()
            .all(|c| c.is_ascii_hexdigit() || matches!(c, '.' | ':'))
        {
            return Err(format!("绑定地址不合法: {spec}"));
        }
    }
    Ok((
        format!("{container}/{proto}"),
        bollard::models::PortBinding {
            host_ip: ip.map(|s| s.trim().to_string()),
            host_port: (!host.is_empty()).then(|| host.to_string()),
        },
    ))
}

/// 重启策略白名单。
fn parse_restart_policy(restart: &str) -> Result<Option<bollard::models::RestartPolicy>, String> {
    use bollard::models::{RestartPolicy, RestartPolicyNameEnum};
    let name = match restart.trim() {
        "" => return Ok(None),
        "no" => RestartPolicyNameEnum::NO,
        "always" => RestartPolicyNameEnum::ALWAYS,
        "unless-stopped" => RestartPolicyNameEnum::UNLESS_STOPPED,
        "on-failure" => RestartPolicyNameEnum::ON_FAILURE,
        other => return Err(format!("不支持的重启策略: {other}")),
    };
    Ok(Some(RestartPolicy {
        name: Some(name),
        maximum_retry_count: None,
    }))
}

/// 容器快启：等价 `docker run -d`（创建 + 启动一步到位），返回新容器 ID。
///
/// 面板「镜像」列表的启动按钮用。只接受白名单参数，参数走 Engine API 而不拼命令：
/// 端口映射解析成 `HostConfig.port_bindings`，`name` 只作为创建参数。
pub async fn container_run(image: &str, name: &str, ports: &[String], restart: &str) -> Response {
    let result = async {
        use bollard::models::{ContainerCreateBody, HostConfig, PortMap};

        let d = docker()?;
        let image = image.trim();
        if image.is_empty() {
            return Err("镜像不能为空".to_string());
        }
        let name = name.trim();
        if !name.is_empty() && !valid_container_name(name) {
            return Err("容器名不合法（字母数字开头，可含 . _ -，最长 63 字符）".to_string());
        }
        if ports.len() > 32 {
            return Err("端口映射过多（上限 32 条）".to_string());
        }

        let mut exposed_ports: Vec<String> = Vec::new();
        let mut port_bindings: PortMap = HashMap::new();
        for spec in ports {
            let (key, binding) = parse_port_spec(spec)?;
            if !exposed_ports.contains(&key) {
                exposed_ports.push(key.clone());
            }
            let entry = port_bindings.entry(key).or_insert_with(|| Some(Vec::new()));
            if let Some(list) = entry.as_mut() {
                list.push(binding);
            }
        }
        let policy = parse_restart_policy(restart)?;

        let config = ContainerCreateBody {
            image: Some(image.to_string()),
            exposed_ports: (!exposed_ports.is_empty()).then_some(exposed_ports),
            host_config: Some(HostConfig {
                port_bindings: (!port_bindings.is_empty()).then_some(port_bindings),
                restart_policy: policy,
                ..Default::default()
            }),
            ..Default::default()
        };
        let created = call(
            ACTION_TIMEOUT,
            "创建容器",
            d.create_container(
                Some(CreateContainerOptions {
                    name: (!name.is_empty()).then(|| name.to_string()),
                    ..Default::default()
                }),
                config,
            ),
        )
        .await?;
        let id = created.id;
        if id.is_empty() {
            return Err("daemon 未返回容器 ID".to_string());
        }
        call(ACTION_TIMEOUT, "启动容器", d.start_container(&id, None)).await?;

        Ok(json!({
            "id": id,
            "name": if name.is_empty() { short_id(&id) } else { name.to_string() },
            "image": image,
        }))
    }
    .await;

    match result {
        Ok(v) => Response::ok("容器已启动", Some(v)),
        Err(e) => Response::err(-1, e),
    }
}

/// 容器 ID 短形式（12 位，和 `docker ps` 的观感一致）。
fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

/// 镜像详情：原始 inspect JSON + 构建历史 + 一份前端直接可用的摘要。
///
/// inspect 整份透传（体积大但一次拿全，前端「Inspect」页签要的就是原文），
/// 摘要字段从 JSON 里取，避免绑定 bollard 各版本的模型字段类型。
pub async fn image_inspect(id: &str) -> Response {
    let result = async {
        let d = docker()?;
        let id = id.trim();
        if id.is_empty() {
            return Err("镜像 ID 不能为空".to_string());
        }
        let insp = call(CALL_TIMEOUT, "查询镜像详情", d.inspect_image(id)).await?;
        let insp = serde_json::to_value(&insp)
            .map_err(|e| format!("序列化镜像详情失败: {e}"))?;
        let history = call(CALL_TIMEOUT, "查询镜像历史", d.image_history(id)).await?;

        let str_of = |v: &Value, key: &str| -> String {
            v.get(key)
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let size = insp.get("Size").and_then(|v| v.as_i64()).unwrap_or(0);
        let created = str_of(&insp, "Created");
        let config = insp.get("Config").cloned().unwrap_or(Value::Null);
        let cmd = config
            .get("Cmd")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let entrypoint = config
            .get("Entrypoint")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let layers = insp
            .get("RootFS")
            .and_then(|v| v.get("Layers"))
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        let history: Vec<Value> = history
            .iter()
            .map(|h| {
                json!({
                    "Id": h.id,
                    "Created": h.created,
                    "CreatedAt": fmt_time(h.created),
                    "CreatedBy": h.created_by,
                    "Tags": h.tags.join(", "),
                    "Size": h.size.max(0) as u64,
                    "SizeText": human_size(h.size.max(0) as u64),
                    "Comment": h.comment,
                })
            })
            .collect();

        Ok(json!({
            "inspect": insp,
            "history": history,
            "summary": {
                "id": str_of(&insp, "Id"),
                "tags": insp.get("RepoTags").and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
                    .unwrap_or_default(),
                "size": size.max(0) as u64,
                "sizeText": human_size(size.max(0) as u64),
                "created": created,
                "architecture": str_of(&insp, "Architecture"),
                "os": str_of(&insp, "Os"),
                "dockerVersion": str_of(&insp, "DockerVersion"),
                "author": str_of(&insp, "Author"),
                "comment": str_of(&insp, "Comment"),
                "cmd": cmd,
                "entrypoint": entrypoint,
                "workdir": config.get("WorkingDir").and_then(|v| v.as_str()).unwrap_or_default(),
                "env": config.get("Env").cloned().unwrap_or(Value::Array(vec![])),
                "exposedPorts": config.get("ExposedPorts").cloned().unwrap_or(Value::Object(Default::default())),
                "labels": config.get("Labels").cloned().unwrap_or(Value::Object(Default::default())),
                "layers": layers,
            }
        }))
    }
    .await;

    match result {
        Ok(v) => Response::ok("ok", Some(v)),
        Err(e) => Response::err(-1, e),
    }
}

/// build-arg 名字白名单：`[A-Za-z_][A-Za-z0-9_]*`。
fn valid_build_arg_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

/// 平台串白名单：`os/arch[/variant]`，仅允许小写字母数字与 `/ . _ -`。
fn valid_platform(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 64
        && p.contains('/')
        && p.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '/' | '.' | '_' | '-')
        })
}

fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 用 Containerfile / Dockerfile 构建镜像（长任务，后台执行）。
///
/// 校验全部前置完成后才起进程，之后接口立即返回；输出逐行落到 `log_path`，
/// 进程退出时追加 `__ZAP_DONE__ <exit_code>`（与 `cron::run` 同一套约定），
/// 前端据此看实时日志 / 判定成败。
///
/// 为什么用 CLI 而不是 Engine API：构建要走 BuildKit，Engine API 的 build 端点
/// 需要 SDK 侧拼 tar 上下文，代价大且拿不到和 `docker build` 一致的输出流。
#[allow(clippy::too_many_arguments)]
pub async fn image_build(
    run_id: String,
    log_path: String,
    context_dir: String,
    containerfile: String,
    tags: Vec<String>,
    build_args: Vec<DockerBuildArg>,
    platform: String,
    no_cache: bool,
    pull: bool,
) -> Response {
    use std::io::Write;

    // ── 校验（失败直接同步返回，前端能立刻看到原因）──
    let log_path = match super::cron::safe_log_path(&log_path) {
        Ok(p) => p,
        Err(e) => return Response::err(-1, e),
    };
    if tags.is_empty() {
        return err_response("至少需要一个镜像名（tag）");
    }
    if tags.len() > 8 {
        return err_response("镜像名过多（上限 8 个）");
    }
    for t in &tags {
        if !valid_image_ref(t) {
            return err_response(&format!(
                "镜像名不合法: {t}（只允许小写字母数字与 . _ - /，可带 :tag）"
            ));
        }
    }
    if !platform.trim().is_empty() && !valid_platform(platform.trim()) {
        return err_response(&format!("构建平台不合法: {platform}"));
    }
    if build_args.len() > 64 {
        return err_response("构建参数过多（上限 64 个）");
    }
    for a in &build_args {
        if !valid_build_arg_key(&a.key) {
            return err_response(&format!("构建参数名不合法: {}", a.key));
        }
        if a.value.len() > 8192 || a.value.contains('\0') {
            return err_response(&format!("构建参数值过长或含非法字符: {}", a.key));
        }
    }

    // 上下文 / Containerfile：绝对路径、无 `..`、真实存在，且 Containerfile 必须在上下文内
    let ctx = match safe_existing_dir(&context_dir) {
        Ok(p) => p,
        Err(e) => return err_response(&e),
    };
    let file = match safe_existing_file(&containerfile) {
        Ok(p) => p,
        Err(e) => return err_response(&e),
    };
    if !file.starts_with(&ctx) {
        return err_response("Containerfile 必须位于构建上下文目录内");
    }

    // `safe_log_path` 已保证父目录在 data/users 下，这里只是补建出来
    if let Some(parent) = log_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Response::err(-1, format!("创建日志目录失败: {e}"));
        }
        // 用户目录属主交给面板进程（面板要自己写 crontab / 云存储配置）
        if let Err(e) = super::ensure_panel_dir_for_path(parent) {
            return Response::err(-1, format!("修改日志目录属主失败: {e}"));
        }
    }

    // ── 组装参数（全部作为独立 argv 传递，无 shell 参与，无注入面）──
    let tags: Vec<String> = tags.iter().map(|t| normalize_image_tag(t)).collect();
    // `--progress=plain` 只在 BuildKit 下有效，用环境变量表达更稳妥
    let mut args: Vec<String> = vec!["build".to_string()];
    args.push("--file".to_string());
    args.push(file.to_string_lossy().to_string());
    for t in &tags {
        args.push("--tag".to_string());
        args.push(t.clone());
    }
    for a in &build_args {
        args.push("--build-arg".to_string());
        args.push(format!("{}={}", a.key, a.value));
    }
    if !platform.trim().is_empty() {
        args.push("--platform".to_string());
        args.push(platform.trim().to_string());
    }
    if no_cache {
        args.push("--no-cache".to_string());
    }
    if pull {
        args.push("--pull".to_string());
    }
    args.push(ctx.to_string_lossy().to_string());

    // ── 打开日志并起进程 ──
    let mut log = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => f,
        Err(e) => return Response::err(-1, format!("打开日志失败: {e}")),
    };
    let _ = writeln!(log, "=== [{}] docker image build {run_id} ===", now_stamp());
    let _ = writeln!(log, "context : {}", ctx.display());
    let _ = writeln!(log, "file    : {}", file.display());
    let _ = writeln!(log, "tags    : {}", tags.join(", "));
    if !build_args.is_empty() {
        // 只记参数名：build-arg 的值常常是密钥，不落盘（docker 自己的输出里若回显，需自行注意）
        let keys: Vec<&str> = build_args.iter().map(|a| a.key.as_str()).collect();
        let _ = writeln!(log, "args    : {}", keys.join(", "));
    }
    if !platform.trim().is_empty() {
        let _ = writeln!(log, "platform: {}", platform.trim());
    }
    let _ = writeln!(log, "cmd     : docker {}", args.join(" "));
    let _ = writeln!(log, "---");
    let _ = log.flush();
    let log_stderr = match log.try_clone() {
        Ok(f) => f,
        Err(e) => return Response::err(-1, format!("准备日志失败: {e}")),
    };

    let child = docker_cmd(args)
        .env("BUILDKIT_PROGRESS", "plain")
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_stderr))
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                "未检测到 docker 命令，请先在宿主机安装 Docker".to_string()
            } else {
                format!("启动 docker build 失败: {e}")
            };
            append_log(&log_path, &format!("\n!!! {msg}"), Some(-1));
            return Response::err(-1, msg);
        }
    };
    let pid = child.id().unwrap_or(0);

    // 等待进程结束并落完成标记：前端 / zapd 都靠它判定状态
    let log_for_done = log_path.clone();
    tokio::spawn(async move {
        let code = match child.wait().await {
            Ok(status) => status.code().unwrap_or(-1),
            Err(_) => -1,
        };
        append_log(&log_for_done, "", Some(code));
    });

    Response::ok(
        "镜像构建已开始",
        Some(json!({
            "run_id": run_id,
            "pid": pid,
            "log": log_path.to_string_lossy(),
            "tags": tags,
        })),
    )
}

/// 统一的错误返回（构建校验阶段用，避免到处写 `Response::err`）。
fn err_response(msg: &str) -> Response {
    Response::err(-1, msg.to_string())
}

/// 追加一段文本与完成标记（`code` 为 None 时只追加文本）。
fn append_log(path: &Path, text: &str, code: Option<i32>) {
    use std::io::Write;
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    if !text.is_empty() {
        let _ = write!(f, "{text}");
    }
    if let Some(code) = code {
        let _ = writeln!(f, "\n__ZAP_DONE__ {code}");
    }
    let _ = f.flush();
}

/// 校验绝对目录：无 `..`、存在且为目录。
fn safe_existing_dir(path: &str) -> Result<PathBuf, String> {
    let p = safe_abs_path(path)?;
    if !p.is_dir() {
        return Err(format!("路径不存在或不是目录: {}", p.display()));
    }
    Ok(p)
}

/// 校验绝对文件：无 `..`、存在且为普通文件。
fn safe_existing_file(path: &str) -> Result<PathBuf, String> {
    let p = safe_abs_path(path)?;
    if !p.is_file() {
        return Err(format!("文件不存在: {}", p.display()));
    }
    Ok(p)
}

/// 绝对路径基础校验（不解析符号链接：调用方随后会做前缀包含判断）。
fn safe_abs_path(path: &str) -> Result<PathBuf, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("路径不能为空".to_string());
    }
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err(format!("路径必须是绝对路径: {path}"));
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("路径不合法: {path}"));
    }
    Ok(p)
}

// ── 数据卷 ───────────────────────────────────────────────

/// 数据卷列表。
pub async fn volumes() -> Response {
    let result = async {
        let d = docker()?;
        let resp = call(
            CALL_TIMEOUT,
            "查询数据卷列表",
            d.list_volumes(Some(ListVolumesOptions::default())),
        )
        .await?;
        Ok(resp
            .volumes
            .unwrap_or_default()
            .iter()
            .map(|v| {
                json!({
                    "Name": v.name,
                    "Driver": v.driver,
                    "Mountpoint": v.mountpoint,
                    "DataDir": volume_data_dir(v),
                    "Scope": as_text(&v.scope),
                    "CreatedAt": v.created_at.as_ref().map(|t| t.to_string()).unwrap_or_default(),
                })
            })
            .collect::<Vec<Value>>())
    }
    .await;
    list_response(result)
}

/// 卷数据真正所在的宿主机目录。
///
/// bind mount 卷（`type=none` + `device=路径`）的 `Mountpoint` 仍指向 daemon 默认目录
/// `/var/lib/docker/volumes/<name>/_data`，真实数据在 `Options.device` 里；
/// 面板展示的是后者，否则用户会以为数据没落到自己的账号目录下。
fn volume_data_dir(v: &Volume) -> String {
    v.options
        .get("device")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| v.mountpoint.clone())
}

/// 数据卷动作：create / remove / prune（清理未使用的卷）。
///
/// `owner_home` / `owner_user` 非空时，create 建的是 bind mount 卷：
/// 数据落在该账号 home 下的 `volumes/{name}`，见 [`prepare_bind_dir`]。
pub async fn volume_action(
    name: &str,
    action: &str,
    owner_home: &str,
    owner_user: &str,
) -> Response {
    let result = async {
        let d = docker()?;
        match action {
            "create" => {
                // 两个落点共用同一套卷名校验：默认目录那边 daemon 也会查，
                // 但提前挡住能给出中文提示，也避免奇怪的名字先被建出来
                validate_volume_name(name)?;
                // 指定了归属就落到用户 home（bind mount）；否则用 daemon 默认位置
                let bind = if !owner_home.is_empty() && !owner_user.is_empty() {
                    Some(prepare_bind_dir(owner_home, name, owner_user)?)
                } else {
                    None
                };
                // 同名卷已存在时 daemon 直接返回旧卷，不会把数据搬去 home：
                // 先探一次，免得提示"已创建"而数据其实还在原处（多半是系统目录）
                if let Ok(v) = call(CALL_TIMEOUT, "查询数据卷", d.inspect_volume(name)).await {
                    return Ok(format!(
                        "数据卷 {} 已存在（数据目录 {}）；要迁到账号目录请先删除该卷再重建",
                        v.name,
                        volume_data_dir(&v)
                    ));
                }
                let driver_opts = bind.as_ref().map(|dir| {
                    let mut m = HashMap::new();
                    // 三件套是 local driver 的 bind mount 约定：type=none + device=路径 + o=bind
                    m.insert("type".to_string(), "none".to_string());
                    m.insert("device".to_string(), dir.clone());
                    m.insert("o".to_string(), "bind".to_string());
                    m
                });
                let v = call(
                    ACTION_TIMEOUT,
                    "创建数据卷",
                    d.create_volume(VolumeCreateRequest {
                        name: Some(name.to_string()),
                        driver: bind.as_ref().map(|_| "local".to_string()),
                        driver_opts,
                        ..Default::default()
                    }),
                )
                .await?;
                Ok(format!(
                    "已创建数据卷 {}（数据目录 {}）",
                    v.name,
                    volume_data_dir(&v)
                ))
            }
            "remove" => {
                call(
                    ACTION_TIMEOUT,
                    "删除数据卷",
                    d.remove_volume(name, None::<RemoveVolumeOptions>),
                )
                .await?;
                Ok(String::new())
            }
            "prune" => {
                let res = call(
                    ACTION_TIMEOUT,
                    "清理未使用数据卷",
                    d.prune_volumes(None::<PruneVolumesOptions>),
                )
                .await?;
                let deleted = res.volumes_deleted.map(|v| v.len()).unwrap_or(0);
                Ok(format!(
                    "已清理 {deleted} 个数据卷，释放 {}",
                    human_size(res.space_reclaimed.unwrap_or(0) as u64)
                ))
            }
            _ => Err(format!("不支持的数据卷操作: {action}")),
        }
    }
    .await;
    action_response(result)
}

/// 卷名白名单：与 docker CLI 一致 —— 字母数字开头，其后可含 `_.-`。
///
/// 卷名会被拼进宿主机路径（bind 卷）或直接交给 daemon，所以这里连 `.` / `..`
/// 与路径分隔符一并挡掉，避免穿越。
fn validate_volume_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 255
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "非法的数据卷名: {name}（只允许字母数字开头，可含 _ . -）"
        ))
    }
}

/// 准备 bind mount 的数据目录 `{home}/volumes/{name}`，返回其绝对路径。
///
/// 多用户环境下卷数据要落在账号自己的 home 里：这样才进得了该用户的磁盘配额，
/// 也能跟着 home 一起被备份，而不是闷在 `/var/lib/docker/volumes`（只有 root 看得到）。
///
/// 拼出来的是**宿主机路径**，而卷名直接来自请求，所以这里必须自己挡住路径穿越：
/// 只接受 docker 允许的字符集（字母数字与 `_.-`），并排除 `.` / `..`。
fn prepare_bind_dir(home: &str, name: &str, owner: &str) -> Result<String, String> {
    validate_volume_name(name)?;
    let home = home.trim_end_matches('/');
    if !home.starts_with('/') || home.contains("..") {
        return Err(format!("非法的家目录: {home}"));
    }

    let root = PathBuf::from(format!("{home}/volumes"));
    if !root.exists() {
        // 首次创建时把 volumes 一并归给该账号，用户在文件管理器里才进得去
        create_owned(&root, owner)?;
    }
    let dir = root.join(name);
    create_owned(&dir, owner)?;
    Ok(dir.to_string_lossy().to_string())
}

/// 建目录并归 `owner` 所有，权限 755。
///
/// 755 + 属主本人：容器进程以 root 运行时照样能写（root 不受 DAC 限制），
/// 以镜像内普通 uid 运行时需要自行 chown —— 与 docker 原生 bind mount 的行为一致。
fn create_owned(dir: &Path, owner: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败 {}: {e}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("设置目录权限失败 {}: {e}", dir.display()))?;
    chown_to(dir, owner)
}

/// 把路径属主设成指定 Linux 账号（uid / gid 以系统记录为准）。
fn chown_to(path: &Path, owner: &str) -> Result<(), String> {
    let cname = std::ffi::CString::new(owner).map_err(|_| format!("非法的账号名: {owner}"))?;
    // SAFETY: getpwnam 返回进程持有的静态结构，这里只当场读两个整数，不保留指针
    let (uid, gid) = unsafe {
        let pw = libc::getpwnam(cname.as_ptr());
        if pw.is_null() {
            return Err(format!("系统账号不存在: {owner}"));
        }
        ((*pw).pw_uid, (*pw).pw_gid)
    };
    std::os::unix::fs::chown(path, Some(uid), Some(gid))
        .map_err(|e| format!("设置属主失败 {}: {e}", path.display()))
}

// ── 网络 ─────────────────────────────────────────────────

/// 网络列表。
pub async fn networks() -> Response {
    let result = async {
        let d = docker()?;
        let list = call(
            CALL_TIMEOUT,
            "查询网络列表",
            d.list_networks(Some(ListNetworksOptions::default())),
        )
        .await?;
        Ok(list
            .iter()
            .map(|n| {
                json!({
                    "Name": n.name,
                    "ID": n.id,
                    "Driver": n.driver,
                    "Scope": n.scope,
                    "IPv6": n.enable_ipv6.unwrap_or(false),
                    "CreatedAt": n.created.as_ref().map(|t| t.to_string()).unwrap_or_default(),
                })
            })
            .collect::<Vec<Value>>())
    }
    .await;
    list_response(result)
}

/// 网络动作：create（可指定 driver）/ remove / prune。
pub async fn network_action(name: &str, action: &str, driver: Option<&str>) -> Response {
    let result = async {
        let d = docker()?;
        match action {
            "create" => {
                let res = call(
                    ACTION_TIMEOUT,
                    "创建网络",
                    d.create_network(NetworkCreateRequest {
                        name: name.to_string(),
                        driver: Some(
                            driver
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .unwrap_or("bridge")
                                .to_string(),
                        ),
                        ..Default::default()
                    }),
                )
                .await?;
                Ok(format!("已创建网络 {}", res.id))
            }
            "remove" => {
                call(ACTION_TIMEOUT, "删除网络", d.remove_network(name)).await?;
                Ok(String::new())
            }
            "prune" => {
                let res = call(ACTION_TIMEOUT, "清理未使用网络", d.prune_networks(None)).await?;
                let deleted = res.networks_deleted.map(|v| v.len()).unwrap_or(0);
                Ok(format!("已清理 {deleted} 个网络"))
            }
            _ => Err(format!("不支持的网络操作: {action}")),
        }
    }
    .await;
    action_response(result)
}

// ── Compose（CLI 插件，Engine API 无对应端点）──────────────

/// 构造一个清空环境、仅带安全 PATH 的 docker 命令。
///
/// 参数写成泛型是为了让「参数需要动态拼接」的场景（如 `docker build` 的多个
/// `--tag` / `--build-arg`）能直接传 `Vec<String>`，而固定子命令仍可传字面量数组。
fn docker_cmd<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = Command::new("docker");
    // 清空环境：不受调用方 `DOCKER_*` / `PATH` 影响，避免被注入额外行为
    cmd.args(args).kill_on_drop(true).env_clear().env(
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    );
    cmd
}

/// 执行 compose 子命令，返回 `(是否成功, stdout, stderr)`。
async fn run_compose(args: &[&str], dur: Duration) -> Result<(bool, String, String), String> {
    let out = timeout(dur, docker_cmd(args).output())
        .await
        .map_err(|_| "命令执行超时".to_string())?;
    let out = match out {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("未检测到 docker 命令，请先安装 Docker".to_string());
        }
        Err(e) => return Err(format!("命令执行失败: {e}")),
    };
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

fn merge_output(stdout: &str, stderr: &str) -> String {
    let stdout = stdout.trim();
    let stderr = stderr.trim();
    if stdout.is_empty() {
        stderr.to_string()
    } else if stderr.is_empty() {
        stdout.to_string()
    } else {
        format!("{stdout}\n{stderr}")
    }
}

fn compose_response(result: Result<(bool, String, String), String>) -> Response {
    match result {
        Ok((true, stdout, stderr)) => Response::ok(
            "ok",
            Some(json!({ "output": merge_output(&stdout, &stderr) })),
        ),
        Ok((false, stdout, stderr)) => {
            Response::err(-1, merge_output(&stdout, &stderr).trim().to_string())
        }
        Err(e) => Response::err(-1, e),
    }
}

/// Compose 项目列表（`docker compose ls -a`）。
pub async fn compose_list() -> Response {
    match run_compose(
        &["compose", "ls", "--all", "--format", "json"],
        CALL_TIMEOUT,
    )
    .await
    {
        Ok((true, stdout, _)) => match serde_json::from_str::<Vec<Value>>(stdout.trim()) {
            Ok(items) => list_response(Ok(items)),
            Err(e) => Response::err(-1, format!("解析 compose 列表失败: {e}")),
        },
        Ok((false, stdout, stderr)) => {
            Response::err(-1, merge_output(&stdout, &stderr).trim().to_string())
        }
        Err(e) => Response::err(-1, e),
    }
}

/// 查项目对应的 compose 配置文件路径。
///
/// 只从 `compose ls` 的结果里反查，不接受前端传入任意路径：
/// 否则就成了「指定任意文件启动容器」的提权口子。
async fn compose_config_file(project: &str) -> Result<String, String> {
    let (ok, stdout, stderr) = run_compose(
        &["compose", "ls", "--all", "--format", "json"],
        CALL_TIMEOUT,
    )
    .await?;
    if !ok {
        return Err(merge_output(&stdout, &stderr).trim().to_string());
    }
    let items: Vec<Value> =
        serde_json::from_str(stdout.trim()).map_err(|e| format!("解析 compose 列表失败: {e}"))?;

    for item in items {
        if item.get("Name").and_then(|v| v.as_str()) != Some(project) {
            continue;
        }
        // ConfigFiles 可能含多个文件（`-f a -f b`），取第一个即可，
        // compose 会自行加载同一目录剩余文件。
        if let Some(files) = item.get("ConfigFiles").and_then(|v| v.as_str())
            && let Some(first) = files
                .split(',')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        {
            return Ok(first.to_string());
        }
        return Err(format!("项目 {project} 未登记配置文件"));
    }
    Err(format!("未找到 Compose 项目 {project}"))
}

/// Compose 动作：up / down / start / stop / restart / pull。
pub async fn compose_action(project: &str, action: &str) -> Response {
    let args_tail: Vec<&str> = match action {
        "up" => vec!["up", "-d"],
        "down" => vec!["down"],
        "start" => vec!["start"],
        "stop" => vec!["stop"],
        "restart" => vec!["restart"],
        "pull" => vec!["pull"],
        _ => return Response::err(-1, format!("不支持的 Compose 操作: {action}")),
    };

    let file = match compose_config_file(project).await {
        Ok(f) => f,
        Err(e) => return Response::err(-1, e),
    };

    // 工作目录取配置文件所在目录，`build` / `env_file` 等相对路径才能命中
    let dir = match std::path::Path::new(&file).parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_string_lossy().to_string(),
        _ => "/".to_string(),
    };

    let mut args = vec![
        "compose",
        "-f",
        file.as_str(),
        "--project-directory",
        dir.as_str(),
    ];
    args.extend(args_tail);

    let dur = if matches!(action, "up" | "pull") {
        COMPOSE_TIMEOUT
    } else {
        ACTION_TIMEOUT
    };
    compose_response(run_compose(&args, dur).await)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_port_spec, prepare_bind_dir, valid_build_arg_key, valid_container_name,
        valid_platform, validate_volume_name,
    };

    /// 卷名会被拼进宿主机路径，穿越必须挡住（这些用例都在建目录之前就返回，不碰文件系统）
    #[test]
    fn bind_dir_rejects_unsafe_names() {
        for name in ["../evil", "a/b", "", ".", "..", "a..b", "/abs", "a b"] {
            assert!(
                prepare_bind_dir("/home/u", name, "u").is_err(),
                "应拒绝卷名 {name:?}"
            );
        }
    }

    #[test]
    fn bind_dir_rejects_unsafe_home() {
        for home in ["home/u", "", "/home/../etc", "relative"] {
            assert!(
                prepare_bind_dir(home, "data", "u").is_err(),
                "应拒绝家目录 {home:?}"
            );
        }
    }

    /// 容器名会被 daemon 用作路径与 DNS 名，字符集要收紧
    #[test]
    fn container_name_whitelist() {
        for name in [
            "",
            ".bad",
            "_bad",
            "/bad",
            "bad name",
            "bad/ok",
            &"x".repeat(64),
        ] {
            assert!(!valid_container_name(name), "应拒绝容器名 {name:?}");
        }
        for name in ["nginx", "my_app", "my.app", "a-b-1", &"x".repeat(63)] {
            assert!(valid_container_name(name), "应接受容器名 {name:?}");
        }
    }

    /// 端口映射语法：`[IP:]宿主机端口:容器端口[/协议]`
    #[test]
    fn port_spec_parsing() {
        let (key, binding) = parse_port_spec("8080:80").unwrap();
        assert_eq!(key, "80/tcp");
        assert_eq!(binding.host_port.as_deref(), Some("8080"));
        assert_eq!(binding.host_ip, None);

        // 只写容器端口：交给 daemon 随机分配宿主机端口
        let (key, binding) = parse_port_spec("80").unwrap();
        assert_eq!(key, "80/tcp");
        assert_eq!(binding.host_port, None);

        let (key, binding) = parse_port_spec("127.0.0.1:5353:53/udp").unwrap();
        assert_eq!(key, "53/udp");
        assert_eq!(binding.host_ip.as_deref(), Some("127.0.0.1"));

        // 宿主机端口留空（含 `80` 与 `:80` 两种写法）都表示「随机分配」，不是错误
        assert!(parse_port_spec(":80").is_ok());

        for bad in [
            "",
            "   ",
            "abc:80",
            "8080:abc",
            "1:2:3:4",
            "80/http",
            "1:2/99999",
        ] {
            assert!(parse_port_spec(bad).is_err(), "应拒绝端口映射 {bad:?}");
        }
    }

    /// 卷名会拼进宿主机路径，字符集与 docker CLI 保持一致
    #[test]
    fn volume_name_whitelist() {
        for name in ["", ".", "..", "a/b", "../etc", "-x", "_x", &"x".repeat(256)] {
            assert!(validate_volume_name(name).is_err(), "应拒绝卷名 {name:?}");
        }
        for name in ["data", "my_data", "my.data", "v-1", "A1"] {
            assert!(validate_volume_name(name).is_ok(), "应接受卷名 {name:?}");
        }
    }

    #[test]
    fn platform_and_arg_whitelist() {
        assert!(valid_platform("linux/amd64"));
        assert!(valid_platform("linux/arm64/v8"));
        for bad in ["linux", "linux/amd64;rm -rf /", "", &"x".repeat(65)] {
            assert!(!valid_platform(bad), "应拒绝平台 {bad:?}");
        }

        assert!(valid_build_arg_key("VERSION"));
        assert!(valid_build_arg_key("_v1"));
        for bad in ["", "1abc", "a-b", "a b", "A$B"] {
            assert!(!valid_build_arg_key(bad), "应拒绝参数名 {bad:?}");
        }
    }
}
