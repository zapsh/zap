//! 系统更新（zapd / zapexec 自升级）服务层：
//! 自动更新配置读写、远端版本检查、发行包下载/校验/解包规整、
//! 升级触发（RPC → zapexec → 独立 zapupgrade）、运行记录收尾。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::Value;

use crate::zap::{ZapError, appstore as ast, update_config};
use crate::{config, db, zapexec};
use sha2::Digest;
use zap_proto::Request;

/// 升级启动阶段互斥（防止手动/自动并发触发）；
/// 一旦 zapupgrade 已在独立 unit 中运行即释放（运行状态由 task_queue 记录）。
pub static UPGRADING: AtomicBool = AtomicBool::new(false);

/// 随发行包分发的二进制清单（与 build.sh / zapupgrade 保持一致）。
pub const UPGRADE_BINS: [&str; 4] = ["zapd", "zapexec", "zapctl", "zapupgrade"];
/// task_queue 中系统升级运行的 action 标识。
pub const ACTION_ZAP_UPDATE: &str = "zap_update";
/// 默认更新渠道（与 build.sh 上传目录一致）。
pub const DEFAULT_CHANNEL: &str = update_config::DEFAULT_CHANNEL;

/// 自动更新配置（来自 `{data}/update_config.yaml`）。
pub use update_config::UpdateConfigFile as UpdateConfig;

#[derive(Debug, Clone, Serialize)]
pub struct LaunchInfo {
    pub run_id: String,
    pub log_path: String,
    pub latest: String,
}

// ── 目录（与 zap.db 同级的 upgrade/ 数据区；zapexec 端校验前缀一致）────

pub fn upgrade_dir() -> PathBuf {
    let cfg = config::get_config().read().unwrap();
    let db_path = Path::new(&cfg.db.path);
    db_path
        .parent()
        .map(|p| p.join("upgrade"))
        .unwrap_or_else(|| PathBuf::from("data/upgrade"))
}

pub fn stage_dir_for(run_id: &str) -> PathBuf {
    upgrade_dir().join("stage").join(run_id)
}

pub fn logs_dir() -> PathBuf {
    upgrade_dir().join("logs")
}

pub fn log_path_for(run_id: &str) -> String {
    logs_dir()
        .join(format!("run-{run_id}.log"))
        .to_string_lossy()
        .into_owned()
}

// ── 配置读写（`{data}/update_config.yaml`）──────────────────

/// 当前自动更新配置（同步：直接读 YAML 缓存，无需 DB）。
pub fn load_config() -> UpdateConfig {
    update_config::load()
}

/// 保存自动更新开关 / cron / 渠道。
pub fn save_config(auto: bool, cron: &str, channel: &str) -> Result<(), ZapError> {
    update_config::save(auto, cron, channel);
    Ok(())
}

/// 记录最近一次远端检查结果（不覆盖 auto/cron/channel）。
pub fn record_check(version: &str, has_update: bool, error: &str) {
    update_config::record_check(version, has_update, error);
}

// ── 版本比较 ────────────────────────────────────────────────

/// 版本号数值化（容忍 `v` 前缀与非数字尾）。
fn parse_version(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split('.')
        .filter_map(|seg| {
            let digits: String = seg.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                None
            } else {
                digits.parse::<u64>().ok()
            }
        })
        .collect()
}

/// latest 是否严格大于 current。
pub fn has_update(current: &str, latest: &str) -> bool {
    let a = parse_version(current);
    let b = parse_version(latest);
    if a.is_empty() || b.is_empty() {
        return latest != current;
    }
    b > a
}

pub fn current_zapd_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ── 网络与下载 ──────────────────────────────────────────────

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(120))
        .timeout_write(std::time::Duration::from_secs(120))
        .build()
}

fn http_get_text(url: &str) -> Result<String, ZapError> {
    let resp = http_agent()
        .get(url)
        .call()
        .map_err(|e| ZapError::New(-1, format!("请求失败 {url}: {e}")))?;
    resp.into_string()
        .map_err(|e| ZapError::New(-1, format!("读取响应失败 {url}: {e}")))
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, ZapError> {
    let resp = http_agent()
        .get(url)
        .call()
        .map_err(|e| ZapError::New(-1, format!("下载失败 {url}: {e}")))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| ZapError::New(-1, format!("读取响应体失败 {url}: {e}")))?;
    Ok(buf)
}

pub fn target_arch() -> Result<&'static str, ZapError> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        other => Err(ZapError::New(
            -1,
            format!("暂不支持该架构自动升级: {other}"),
        )),
    }
}

/// 查询远端渠道的 `latest.txt` 得到最新版本号。
pub async fn check_remote_version(channel: &str) -> Result<String, ZapError> {
    let channel = channel.trim_end_matches('/').to_string();
    tokio::task::spawn_blocking(move || {
        let url = format!("{channel}/latest.txt");
        let text = http_get_text(&url)?;
        let v = text.trim().to_string();
        if v.is_empty() {
            return Err(ZapError::New(-1, format!("{url} 返回空内容")));
        }
        Ok(v)
    })
    .await
    .map_err(|e| ZapError::New(-1, format!("检查更新任务异常: {e}")))?
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// tar 包内可能带 `zap/` 顶层目录：把其中同名二进制规整到 stage 根。
fn normalize_stage(stage: &Path) {
    for bin in UPGRADE_BINS {
        let dst = stage.join(bin);
        if dst.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(stage) {
            for e in entries.flatten() {
                let src = e.path().join(bin);
                if src.is_file() {
                    let _ = std::fs::copy(&src, &dst);
                    break;
                }
            }
        }
    }
}

/// 包名里的「发行线」后缀：商业版是 `-pro`，社区版没有。
///
/// 商业版与社区版同版本号、不同包名：装的是哪条线就一直升哪条线，Pro 不会被社区版包
/// 覆盖回去。以 `/etc/zap/edition`（install.sh 写入、zapupgrade 换线后回写）为准 ——
/// 它是「这台机器跟哪条线」的唯一记录，能覆盖「二进制被回滚回旧版」这类不一致；
/// 文件缺失时（老机器、或 /etc/zap 不可读）按本二进制的编译期特性兜底。
fn pkg_suffix() -> &'static str {
    match std::fs::read_to_string("/etc/zap/edition") {
        Ok(s) if s.trim() == "pro" => "-pro",
        Ok(s) if s.trim() == "community" => "",
        _ if cfg!(feature = "commercial") => "-pro",
        _ => "",
    }
}

/// 下载发行包 → sha256 校验 → 解包 → 规整到 `stage/{run_id}/`（含 version 文件）。
/// 返回绝对路径（zapexec 端的前缀白名单校验需要绝对路径）。
pub async fn download_and_stage(
    channel: &str,
    version: &str,
    run_id: &str,
) -> Result<PathBuf, ZapError> {
    let channel = channel.trim_end_matches('/').to_string();
    let version = version.trim_start_matches('v').to_string();
    let arch = target_arch()?.to_string();
    let pkg = format!("zap-v{version}{}-linux-{arch}.tar.gz", pkg_suffix());
    let stage = stage_dir_for(run_id);
    tokio::task::spawn_blocking(move || -> Result<PathBuf, ZapError> {
        let base = format!("{channel}/{pkg}");
        let data = http_get_bytes(&base)?;
        // 校验：发行侧上传 .sha256（build.sh），远端缺失/为空时跳过强校验
        let expected = http_get_text(&format!("{base}.sha256"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let actual = to_hex(&sha2::Sha256::digest(&data));
        if !expected.is_empty() && !actual.eq_ignore_ascii_case(&expected) {
            return Err(ZapError::New(
                -1,
                format!("发行包校验失败：sha256 不匹配（期望 {expected}，实际 {actual}）"),
            ));
        }
        // 解包（发行包来自可信渠道，Archive::unpack 足够）
        std::fs::create_dir_all(&stage)
            .map_err(|e| ZapError::New(-1, format!("创建升级目录失败: {e}")))?;
        let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(data));
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(&stage)
            .map_err(|e| ZapError::New(-1, format!("解压发行包失败: {e}")))?;
        normalize_stage(&stage);
        std::fs::write(stage.join("version"), format!("v{version}"))
            .map_err(|e| ZapError::New(-1, format!("写入版本文件失败: {e}")))?;
        if stage.is_absolute() {
            Ok(stage)
        } else {
            Ok(std::env::current_dir()
                .map_err(|e| ZapError::New(-1, format!("获取工作目录失败: {e}")))?
                .join(stage))
        }
    })
    .await
    .map_err(|e| ZapError::New(-1, format!("下载升级包任务异常: {e}")))?
}

// ── 升级触发与收尾 ──────────────────────────────────────────

/// 启动一次系统升级（手动 / 自动共用）。
/// 流程：查远端最新版 → 下载+校验+解包 stage → 登记运行 → RPC 拉起 zapupgrade。
/// 升级包实际执行在 zapexec 拉起的独立 unit 中，本函数返回即触发完成。
pub async fn launch_update(username: &str) -> Result<LaunchInfo, ZapError> {
    if UPGRADING.swap(true, Ordering::SeqCst) {
        return Err(ZapError::New(
            -1,
            "已有升级任务正在进行中，请稍后再试".to_string(),
        ));
    }
    let result = launch_update_inner(username).await;
    UPGRADING.store(false, Ordering::SeqCst);
    result
}

async fn launch_update_inner(username: &str) -> Result<LaunchInfo, ZapError> {
    let cfg = load_config();
    let channel = cfg.channel.clone();
    let current = current_zapd_version();

    let latest = match check_remote_version(&channel).await {
        Ok(v) => v,
        Err(e) => {
            record_check("", false, &e.to_string());
            return Err(e);
        }
    };
    if !has_update(current, &latest) {
        record_check(&latest, false, "");
        return Err(ZapError::New(-1, format!("当前已是最新版本 v{current}")));
    }
    record_check(&latest, true, "");

    let run_id = ast::generate_run_id();
    let log_path = log_path_for(&run_id);
    if let Some(parent) = Path::new(&log_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let stage = match download_and_stage(&channel, &latest, &run_id).await {
        Ok(s) => s,
        Err(e) => {
            record_check(&latest, true, &e.to_string());
            return Err(e);
        }
    };

    ast::register_run(&run_id, ACTION_ZAP_UPDATE, &latest, username, &log_path).await?;

    let resp = zapexec::call(Request::UpgradeRun {
        run_id: run_id.clone(),
        stage_dir: stage.to_string_lossy().into_owned(),
        log_path: log_path.clone(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(&run_id, "failed", resp.code as i64).await;
        let msg = format!("启动升级器失败: {}", resp.message);
        record_check(&latest, true, &msg);
        return Err(ZapError::New(resp.code, msg));
    }
    ast::watch_log(run_id.clone(), log_path.clone());
    Ok(LaunchInfo {
        run_id,
        log_path,
        latest,
    })
}

/// 当前正在进行的系统升级（`task_queue` 中 action=zap_update 且 status=running 的最新一条）。
pub async fn running_run() -> Option<ast::AppstoreRun> {
    let pool = db::get_db_pool().await;
    sqlx::query_as::<_, ast::AppstoreRun>(
        "SELECT * FROM task_queue WHERE action = ? AND status = 'running' ORDER BY id DESC LIMIT 1",
    )
    .bind(ACTION_ZAP_UPDATE)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 系统升级历史（按时间倒序）。
pub async fn recent_update_runs(limit: i64) -> Vec<Value> {
    let pool = db::get_db_pool().await;
    let rows = sqlx::query_as::<_, ast::AppstoreRun>(
        "SELECT * FROM task_queue WHERE action = ? ORDER BY id DESC LIMIT ?",
    )
    .bind(ACTION_ZAP_UPDATE)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter().map(|r| r.to_json()).collect()
}

/// 惰性收尾：zapd 若在升级中重启，watch_log 任务会丢失，
/// 遗留 running 记录在下次访问状态时据日志 DONE 标记补全状态。
pub async fn finalize_stale_updates() {
    let pool = db::get_db_pool().await;
    let Ok(rows) = sqlx::query_as::<_, (String, String)>(
        "SELECT task_id, log_path FROM task_queue WHERE action = ? AND status = 'running'",
    )
    .bind(ACTION_ZAP_UPDATE)
    .fetch_all(pool)
    .await
    else {
        return;
    };
    for (run_id, log_path) in rows {
        let Ok(content) = std::fs::read_to_string(&log_path) else {
            continue;
        };
        if let Some(code) = content
            .rsplit(ast::DONE_MARKER)
            .next()
            .and_then(|tail| tail.split_whitespace().next())
            .and_then(|c| c.parse::<i64>().ok())
        {
            let status = if code == 0 { "success" } else { "failed" };
            ast::finish_run(&run_id, status, code).await;
        }
    }
}
