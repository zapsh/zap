//! 系统二进制升级（zapd / zapexec / zapctl / zapupgrade）。
//!
//! 本模块只负责**安全地拉起独立升级器 zapupgrade**：真正的下载、校验、
//! 备份、原子替换与 `systemctl restart` 都在 zapupgrade（一次性进程，
//! 运行于独立 systemd transient unit）中完成，因此升级/重启 zapd 或
//! zapexec 自身都不会中断升级流程。

use std::path::{Path, PathBuf};

use serde_json::json;

use super::root_cmd;
use zap_proto::Response;

/// 与 appstore::zap_path() 一致：ZAP 程序安装根目录（systemd 里由
/// `Environment=ZAP_PATH=/usr/local/zap` 提供）。
fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

/// 合法 run_id / systemd unit 名：仅 ASCII 字母数字与 `-`/`_`，长度 1..=48。
fn valid_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// 校验 `p` 位于 `root` 之下（不含 root 自身）。
fn within(root: &Path, p: &Path) -> bool {
    match p.strip_prefix(root) {
        Ok(rest) => rest.components().count() > 0,
        Err(_) => false,
    }
}

/// `upgrade.info`：返回 zapexec 自身版本与安装目录（供系统更新页展示）。
pub async fn info() -> Response {
    Response::ok(
        "ok",
        Some(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "zap_path": zap_path().to_string_lossy(),
        })),
    )
}

/// `upgrade.run`：校验升级包目录与日志路径位于数据区后，
/// 用 `systemd-run --no-block` 把 zapupgrade 放入独立 unit 中异步执行并立即返回。
/// 没有 systemd 时（开发机/容器）退化为直接 spawn 子进程，语义相同。
pub async fn run(run_id: String, stage_dir: String, log_path: String) -> Response {
    if !valid_token(&run_id) {
        return Response::err(-1, format!("run_id 非法: {run_id}"));
    }
    let zap = zap_path();
    // stage / log 必须位于 {ZAP_PATH}/data/upgrade/ 下，杜绝越权读写任意路径
    let stage_root = zap.join("data/upgrade/stage");
    let log_root = zap.join("data/upgrade/logs");
    let stage = Path::new(&stage_dir);
    let log = Path::new(&log_path);
    if !within(&stage_root, stage) {
        return Response::err(-1, format!("升级包目录越权: {stage_dir}"));
    }
    if !within(&log_root, log) {
        return Response::err(-1, format!("升级日志路径越权: {log_path}"));
    }

    // 优先执行 stage 内随包分发的 zapupgrade（保证与目标版本配套）；
    // 未来若系统已有 zapupgrade 也可执行系统里的，但随包版更可预期。
    let runner = stage.join("zapupgrade");
    if !runner.is_file() {
        return Response::err(-1, format!("升级包缺少 zapupgrade: {}", runner.display()));
    }
    let runner_path = runner.to_string_lossy().into_owned();

    // 注意：zapupgrade 的 CLI 是扁平参数（--stage / --dir / --log），**没有子命令**。
    // 不能额外传 "apply"，否则 clap 直接解析失败、升级器启动即退出（表现为日志一直为空）。
    // 参数列表不含程序自身，避免再用下标取日志路径（此前 args[7] 依赖参数个数）。
    let args: Vec<String> = vec![
        "--stage".to_string(),
        stage_dir.clone(),
        "--dir".to_string(),
        zap.to_string_lossy().into_owned(),
        "--log".to_string(),
        log_path.clone(),
    ];

    if !systemd_available() {
        let runner_path = runner_path.clone();
        let args = args.clone();
        let log_for_spawn = log_path.clone();
        let out = tokio::task::spawn_blocking(move || {
            let log_file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_for_spawn)
                .ok();
            let mut c = root_cmd(&runner_path);
            let mut spawn = c.args(&args).stdin(std::process::Stdio::null());
            if let Some(f) = log_file {
                let stdout = f.try_clone().ok();
                let stderr = Some(f);
                if let Some(s) = stdout {
                    spawn = spawn.stdout(std::process::Stdio::from(s));
                }
                if let Some(s) = stderr {
                    spawn = spawn.stderr(std::process::Stdio::from(s));
                }
            }
            spawn.spawn()
        })
        .await;
        return match out {
            Ok(Ok(_child)) => {
                // detach：不等待，升级器在日志中写 __ZAP_DONE__ 结束标记
                Response::ok("升级已启动", Some(json!({ "run_id": run_id })))
            }
            Ok(Err(e)) => Response::err(-1, format!("拉起升级器失败: {e}")),
            Err(e) => Response::err(-1, format!("任务执行失败: {e}")),
        };
    }

    let unit = format!("zap-upgrade-{run_id}");
    let out = tokio::task::spawn_blocking(move || {
        root_cmd("systemd-run")
            .arg("--no-block")
            .arg("--unit")
            .arg(&unit)
            .arg("--collect")
            .arg("--property=Type=oneshot")
            .arg("--")
            .arg(&runner_path)
            .args(&args)
            .output()
    })
    .await;

    match out {
        Ok(Ok(o)) if o.status.success() => {
            Response::ok("升级已启动", Some(json!({ "run_id": run_id })))
        }
        Ok(Ok(o)) => Response::err(
            -1,
            format!(
                "启动升级器失败: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        ),
        Ok(Err(e)) => Response::err(-1, format!("执行 systemd-run 失败: {e}")),
        Err(e) => Response::err(-1, format!("任务执行失败: {e}")),
    }
}

/// systemd 是否正在运行（作为 PID 1 且可交互）。
/// 判定依据：/run/systemd/system 仅由运行中的 systemd 创建；
/// 开发机裸进程 / Docker 容器一般不存在该目录。
/// 非 Linux 平台恒为 false —— 那里走直接 spawn 的分支。
fn systemd_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new("/run/systemd/system").exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}
