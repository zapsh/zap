//! Docker 容器管理动词（面板「容器」页面的执行后端）。
//!
//! 与其余动词一致：**只提供白名单子命令**，不接受任意 shell 片段。
//! 每个动作显式枚举 docker 子命令与参数，容器 / 镜像 id 仅作为被操作对象传入，
//! 不参与命令构造。
//!
//! 数据一律通过 `docker ... --format json`（或 `--format {{json .}}`）取结构化输出，
//! 不解析人类可读表格，避免列宽变化、版本差异导致字段错位。

use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;
use zap_proto::Response;

/// 常规查询命令超时（列表 / inspect / 日志）
const SHORT_TIMEOUT: Duration = Duration::from_secs(20);
/// 常规写动作超时（启停 / 删除 / 创建）
const ACTION_TIMEOUT: Duration = Duration::from_secs(60);
/// 耗时动作超时（镜像拉取、compose up/pull）
const LONG_TIMEOUT: Duration = Duration::from_secs(600);

/// 构造一个清空环境、仅带安全 PATH 的 docker 命令。
///
/// 不继承面板进程的环境变量（其中可能含业务配置项），且 docker CLI 不依赖 HOME。
fn docker_cmd(args: &[&str]) -> Command {
    let mut cmd = Command::new("docker");
    cmd.args(args).kill_on_drop(true).env_clear().env(
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    );
    cmd
}

/// 执行 docker 子命令，返回 `(是否成功, stdout, stderr)`。
///
/// 超时与「docker 不存在」统一收敛成 `Err`，由上层决定是返回 0 还是错误码。
async fn run(args: &[&str], timeout: Duration) -> Result<(bool, String, String), String> {
    let out = tokio::time::timeout(timeout, docker_cmd(args).output())
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

/// 逐行 JSON（`{{json .}}`）输出解析。
fn json_lines(raw: &str) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(v) => items.push(v),
            Err(e) => return Err(format!("解析 docker 输出失败: {e}")),
        }
    }
    Ok(items)
}

/// 从 docker 的 label 串（`k1=v1,k2=v2`）里取值。
fn label_value(labels: &str, key: &str) -> Option<String> {
    labels.split(',').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k.trim() == key).then(|| v.to_string())
    })
}

/// 把列表接口变成 `Response`：`Ok(items)` → `{"items": [...]}`。
fn list_response(result: Result<Vec<Value>, String>) -> Response {
    match result {
        Ok(items) => Response::ok("ok", Some(json!({ "items": items }))),
        Err(e) => Response::err(-1, e),
    }
}

/// 把 stdout / stderr 合并成的文本动作结果变成 `Response`。
fn action_response(result: Result<(bool, String, String), String>) -> Response {
    match result {
        Ok((true, stdout, stderr)) => Response::ok(
            "ok",
            Some(json!({ "output": merge_output(&stdout, &stderr) })),
        ),
        Ok((false, stdout, stderr)) => {
            Response::err(-1, trim_output(&merge_output(&stdout, &stderr)))
        }
        Err(e) => Response::err(-1, e),
    }
}

/// 合并输出优先看 stdout（成功信息），失败时通常只有 stderr。
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

fn trim_output(s: &str) -> String {
    s.trim().to_string()
}

// ── 环境探测 ──────────────────────────────────────────────

/// `docker --version` 的版本号；命令不存在时返回 Err。
async fn cli_version() -> Result<String, String> {
    let (ok, stdout, stderr) = run(&["--version"], SHORT_TIMEOUT).await?;
    if !ok {
        return Err(trim_output(&merge_output(&stdout, &stderr)));
    }
    Ok(stdout.trim().to_string())
}

/// compose 插件是否可用（`docker compose version`）。
async fn compose_available() -> bool {
    run(&["compose", "version", "--short"], SHORT_TIMEOUT)
        .await
        .is_ok_and(|(ok, _, _)| ok)
}

/// GET 环境状态：Docker 是否安装、守护进程是否在运行、compose 是否可用。
///
/// 探测失败也是**正常返回**（installed / daemon = false），前端据此展示引导，
/// 而不是弹一个看不懂的错误。
pub async fn status() -> Response {
    let cli = cli_version().await;

    let (installed, cli_version) = match cli {
        Ok(v) => (true, v),
        Err(e) => {
            return Response::ok(
                "ok",
                Some(json!({
                    "installed": false,
                    "daemon": false,
                    "version": "",
                    "compose": false,
                    "error": e,
                })),
            );
        }
    };

    // server 版本拿不到 = daemon 没起来（典型报错：permission denied / connection refused）
    let (daemon, server_version, error) = match run(
        &["version", "--format", "{{.Server.Version}}"],
        SHORT_TIMEOUT,
    )
    .await
    {
        Ok((true, stdout, _)) => (true, stdout.trim().to_string(), String::new()),
        Ok((false, _, stderr)) => (false, String::new(), trim_output(&stderr)),
        Err(e) => (false, String::new(), e),
    };

    Response::ok(
        "ok",
        Some(json!({
            "installed": installed,
            "daemon": daemon,
            "version": if daemon { server_version } else { cli_version },
            "compose": daemon && compose_available().await,
            "error": error,
        })),
    )
}

// ── 容器 ─────────────────────────────────────────────────

/// 容器列表（`docker container ls`，`all` 含已停止的）。
///
/// 额外补 `project` 字段：从 compose label 里取出所属项目，供前端分组筛选。
pub async fn containers(all: bool) -> Response {
    let mut args = vec!["container", "ls", "--no-trunc", "--format", "{{json .}}"];
    if all {
        args.insert(2, "--all");
    }

    match run(&args, SHORT_TIMEOUT).await {
        Ok((true, stdout, _)) => match json_lines(&stdout) {
            Ok(mut items) => {
                for item in &mut items {
                    let project = item
                        .get("Labels")
                        .and_then(|v| v.as_str())
                        .and_then(|l| label_value(l, "com.docker.compose.project"))
                        .unwrap_or_default();
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert("project".to_string(), json!(project));
                    }
                }
                list_response(Ok(items))
            }
            Err(e) => Response::err(-1, e),
        },
        Ok((false, stdout, stderr)) => {
            Response::err(-1, trim_output(&merge_output(&stdout, &stderr)))
        }
        Err(e) => Response::err(-1, e),
    }
}

/// 容器动作的docker 参数表：只接受白名单动作。
fn container_args<'a>(action: &'a str, id: &'a str) -> Option<Vec<&'a str>> {
    let args = match action {
        "start" => vec!["container", "start", id],
        "stop" => vec!["container", "stop", id],
        "restart" => vec!["container", "restart", id],
        "pause" => vec!["container", "pause", id],
        "unpause" => vec!["container", "unpause", id],
        "kill" => vec!["container", "kill", id],
        // 面板上删除的多半是运行中的容器，-f 省掉「先停再删」两步
        "remove" => vec!["container", "rm", "-f", id],
        _ => return None,
    };
    Some(args)
}

/// 批量容器动作：逐个执行，返回失败明细（一个失败不影响其余）。
pub async fn container_action(ids: &[String], action: &str) -> Response {
    let mut failed: Vec<Value> = Vec::new();
    let mut succeeded = 0usize;

    for id in ids {
        let Some(args) = container_args(action, id) else {
            return Response::err(-1, format!("不支持的容器操作: {action}"));
        };
        match run(&args, ACTION_TIMEOUT).await {
            Ok((true, _, _)) => succeeded += 1,
            Ok((false, stdout, stderr)) => failed.push(json!({
                "id": id,
                "error": trim_output(&merge_output(&stdout, &stderr)),
            })),
            Err(e) => failed.push(json!({ "id": id, "error": e })),
        }
    }

    Response::ok(
        "ok",
        Some(json!({ "succeeded": succeeded, "failed": failed })),
    )
}

/// 容器详情（`docker container inspect`，透传第一个对象的原始 JSON）。
pub async fn container_inspect(id: &str) -> Response {
    match run(&["container", "inspect", id], SHORT_TIMEOUT).await {
        Ok((true, stdout, _)) => match serde_json::from_str::<Vec<Value>>(&stdout) {
            Ok(items) => match items.into_iter().next() {
                Some(obj) => Response::ok("ok", Some(obj)),
                None => Response::err(-1, "容器不存在".to_string()),
            },
            Err(e) => Response::err(-1, format!("解析 inspect 结果失败: {e}")),
        },
        Ok((false, stdout, stderr)) => {
            Response::err(-1, trim_output(&merge_output(&stdout, &stderr)))
        }
        Err(e) => Response::err(-1, e),
    }
}

/// 容器日志尾部：`docker container logs --tail N [--since T] [--timestamps] <id>`。
pub async fn container_logs(
    id: &str,
    tail: Option<u32>,
    since: Option<&str>,
    timestamps: bool,
) -> Response {
    let tail_arg = tail.unwrap_or(200).to_string();
    let mut args = vec!["container", "logs", "--tail", tail_arg.as_str()];
    if let Some(s) = since
        && !s.trim().is_empty()
    {
        args.push("--since");
        args.push(s);
    }
    if timestamps {
        args.push("--timestamps");
    }
    args.push(id);

    match run(&args, SHORT_TIMEOUT).await {
        // docker logs 把容器侧 stdout/stderr 都并到自身的 stdout/stderr 上，
        // 成功与否都可能有内容，统一按文本回传。
        Ok((_, stdout, stderr)) => {
            Response::ok("ok", Some(json!({ "log": merge_output(&stdout, &stderr) })))
        }
        Err(e) => Response::err(-1, e),
    }
}

/// 实时资源占用（`docker stats --no-stream`）。
pub async fn stats() -> Response {
    match run(
        &[
            "stats",
            "--no-stream",
            "--no-trunc",
            "--format",
            "{{json .}}",
        ],
        SHORT_TIMEOUT,
    )
    .await
    {
        Ok((true, stdout, _)) => list_response(json_lines(&stdout)),
        Ok((false, stdout, stderr)) => {
            Response::err(-1, trim_output(&merge_output(&stdout, &stderr)))
        }
        Err(e) => Response::err(-1, e),
    }
}

// ── 镜像 ─────────────────────────────────────────────────

/// 镜像列表（含悬空镜像）。
pub async fn images() -> Response {
    match run(
        &["image", "ls", "--all", "--format", "{{json .}}"],
        SHORT_TIMEOUT,
    )
    .await
    {
        Ok((true, stdout, _)) => list_response(json_lines(&stdout)),
        Ok((false, stdout, stderr)) => {
            Response::err(-1, trim_output(&merge_output(&stdout, &stderr)))
        }
        Err(e) => Response::err(-1, e),
    }
}

/// 镜像动作：pull（拉取引用）/ remove（删除镜像）/ prune（清理悬空镜像）。
pub async fn image_action(id: &str, action: &str) -> Response {
    let result = match action {
        "pull" => run(&["image", "pull", id], LONG_TIMEOUT).await,
        "remove" => run(&["image", "rm", "-f", id], ACTION_TIMEOUT).await,
        "prune" => run(&["image", "prune", "-f"], ACTION_TIMEOUT).await,
        _ => return Response::err(-1, format!("不支持的镜像操作: {action}")),
    };
    action_response(result)
}

// ── 数据卷 ───────────────────────────────────────────────

/// 数据卷列表。
pub async fn volumes() -> Response {
    match run(&["volume", "ls", "--format", "{{json .}}"], SHORT_TIMEOUT).await {
        Ok((true, stdout, _)) => list_response(json_lines(&stdout)),
        Ok((false, stdout, stderr)) => {
            Response::err(-1, trim_output(&merge_output(&stdout, &stderr)))
        }
        Err(e) => Response::err(-1, e),
    }
}

/// 数据卷动作：create / remove / prune（清理未使用的卷）。
pub async fn volume_action(name: &str, action: &str) -> Response {
    let result = match action {
        "create" => run(&["volume", "create", name], ACTION_TIMEOUT).await,
        "remove" => run(&["volume", "rm", "-f", name], ACTION_TIMEOUT).await,
        "prune" => run(&["volume", "prune", "-f"], ACTION_TIMEOUT).await,
        _ => return Response::err(-1, format!("不支持的数据卷操作: {action}")),
    };
    action_response(result)
}

// ── 网络 ─────────────────────────────────────────────────

/// 网络列表。
pub async fn networks() -> Response {
    match run(&["network", "ls", "--format", "{{json .}}"], SHORT_TIMEOUT).await {
        Ok((true, stdout, _)) => list_response(json_lines(&stdout)),
        Ok((false, stdout, stderr)) => {
            Response::err(-1, trim_output(&merge_output(&stdout, &stderr)))
        }
        Err(e) => Response::err(-1, e),
    }
}

/// 网络动作：create（可指定 driver）/ remove / prune。
pub async fn network_action(name: &str, action: &str, driver: Option<&str>) -> Response {
    let result = match action {
        "create" => match driver {
            Some(d) if !d.trim().is_empty() => {
                run(&["network", "create", "--driver", d, name], ACTION_TIMEOUT).await
            }
            _ => run(&["network", "create", name], ACTION_TIMEOUT).await,
        },
        "remove" => run(&["network", "rm", name], ACTION_TIMEOUT).await,
        "prune" => run(&["network", "prune", "-f"], ACTION_TIMEOUT).await,
        _ => return Response::err(-1, format!("不支持的网络操作: {action}")),
    };
    action_response(result)
}

// ── Compose ──────────────────────────────────────────────

/// Compose 项目列表（`docker compose ls -a`）。
pub async fn compose_list() -> Response {
    match run(
        &["compose", "ls", "--all", "--format", "json"],
        SHORT_TIMEOUT,
    )
    .await
    {
        Ok((true, stdout, _)) => match serde_json::from_str::<Vec<Value>>(stdout.trim()) {
            Ok(items) => list_response(Ok(items)),
            // compose v1（`docker-compose`）不支持 `--format json`，给出可操作的提示
            Err(e) => Response::err(-1, format!("解析 compose 列表失败: {e}")),
        },
        Ok((false, stdout, stderr)) => {
            Response::err(-1, trim_output(&merge_output(&stdout, &stderr)))
        }
        Err(e) => Response::err(-1, e),
    }
}

/// 查项目对应的 compose 配置文件路径。
///
/// 只从 `compose ls` 的结果里反查，不接受前端传入任意路径：
/// 否则就成了「指定任意文件启动容器」的提权口子。
async fn compose_config_file(project: &str) -> Result<String, String> {
    let (ok, stdout, stderr) = run(
        &["compose", "ls", "--all", "--format", "json"],
        SHORT_TIMEOUT,
    )
    .await?;
    if !ok {
        return Err(trim_output(&merge_output(&stdout, &stderr)));
    }
    let items: Vec<Value> =
        serde_json::from_str(stdout.trim()).map_err(|e| format!("解析 compose 列表失败: {e}"))?;

    for item in items {
        if item.get("Name").and_then(|v| v.as_str()) != Some(project) {
            continue;
        }
        // ConfigFiles 可能含多个文件（`-f a -f b`），这里取第一个即可，
        // compose 会自行加载同一目录剩余文件。
        if let Some(files) = item.get("ConfigFiles").and_then(|v| v.as_str()) {
            if let Some(first) = files
                .split(',')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Ok(first.to_string());
            }
        }
        return Err(format!("项目 {project} 未登记配置文件"));
    }
    Err(format!("未找到 Compose 项目 {project}"))
}

/// Compose 动作：up / down / start / stop / restart / pull。
///
/// 用 `-f <配置文件> --project-directory <所在目录>` 定位项目，
/// 与「面板手动 docker compose up」的行为保持一致。
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

    let timeout = if matches!(action, "up" | "pull") {
        LONG_TIMEOUT
    } else {
        ACTION_TIMEOUT
    };
    action_response(run(&args, timeout).await)
}
