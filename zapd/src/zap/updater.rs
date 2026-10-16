// SPDX-License-Identifier: AGPL-3.0-only
//! 系统更新（zapd / zapexec 自升级）服务层：
//! 自动更新配置读写、远端版本检查、发行包下载/校验/解包规整、
//! 升级触发（RPC → zapexec → 独立 zapupgrade）、运行记录收尾。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::Value;

use crate::zap::{ZapError, appstore as ast, release_key, update_config};
use crate::{config, db, zapexec};
use sha2::Digest;
use tracing::warn;
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

/// 渠道地址是否允许：insecure 模式放行任意 scheme；否则必须 `https://`。
///
/// 自动更新以 root / zapadm 权限执行升级器，明文 http 渠道可被中间人直接投毒，
/// 因此默认强制 https，仅在 `update_config.yaml` 显式 `allow_insecure_update: 1`
/// （离线 / 内网可信源）时放开。
pub fn channel_allowed(channel: &str, allow_insecure: bool) -> bool {
    allow_insecure || channel.starts_with("https://")
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
    // 渠道强制 https（insecure 模式除外）：与下载路径一致
    if !channel_allowed(&channel, load_config().allow_insecure_update != 0) {
        return Err(ZapError::New(-1, "更新渠道必须为 https://".to_string()));
    }
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
/// 覆盖回去。以本二进制的编译期特性（`commercial` feature）为准 —— 二进制本身就是
/// 发行线的唯一真相源，不需要外部文件记录。
fn pkg_suffix() -> &'static str {
    if cfg!(feature = "commercial") {
        "-pro"
    } else {
        ""
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
    // 渠道强制 https（insecure 模式除外）：避免明文渠道被中间人投毒
    let allow_insecure = load_config().allow_insecure_update != 0;
    if !channel_allowed(&channel, allow_insecure) {
        return Err(ZapError::New(
            -1,
            "更新渠道必须为 https://（离线 / 内网 http 源请在 update_config.yaml 设置 allow_insecure_update: 1）"
                .to_string(),
        ));
    }
    let version = version.trim_start_matches('v').to_string();
    let arch = target_arch()?.to_string();
    let pkg = format!("zap-v{version}{}-linux-{arch}.tar.gz", pkg_suffix());
    let stage = stage_dir_for(run_id);
    tokio::task::spawn_blocking(move || -> Result<PathBuf, ZapError> {
        let base = format!("{channel}/{pkg}");
        let data = http_get_bytes(&base)?;

        // ── Tier 2：发布签名（Ed25519 分离签名 {pkg}.sig）──
        // 签名由 CI 私钥签署，二进制内固化发布公钥；比 sha256 更强（抗中间人替换包体）。
        if let Some(pk) = release_key::release_pubkey() {
            match http_get_text(&format!("{base}.sig")) {
                Ok(sig_hex) => release_key::verify_release_signature(&data, &sig_hex, &pk)
                    .map_err(|e| ZapError::New(-1, format!("发布签名校验失败：{e}")))?,
                Err(_) => {
                    if allow_insecure {
                        warn!("未找到发布签名 {base}.sig，insecure 模式：跳过签名校验");
                    } else {
                        return Err(ZapError::New(
                            -1,
                            format!(
                                "缺少发布签名文件 {base}.sig，拒绝安装（开启 allow_insecure_update 可跳过）"
                            ),
                        ));
                    }
                }
            }
        } else {
            warn!("未配置发布公钥，跳过签名校验（仅依赖 sha256）");
        }

        // ── Tier 1：sha256 强校验（fail-closed，缺失 / 不匹配均拒绝）──
        // 旧实现用 unwrap_or_default() 在 .sha256 取不到时静默跳过，已被绕过；
        // 现在缺失即拒绝安装（insecure 模式除外）。
        let expected = http_get_text(&format!("{base}.sha256"))
            .map(|s| s.trim().to_string())
            .map_err(|e| ZapError::New(-1, format!("下载校验文件 .sha256 失败: {e}")))?;
        if expected.is_empty() {
            if allow_insecure {
                warn!("缺少 .sha256，insecure 模式：跳过完整性校验");
            } else {
                return Err(ZapError::New(
                    -1,
                    format!("缺少校验文件 {base}.sha256，拒绝安装（开启 allow_insecure_update 可跳过）"),
                ));
            }
        } else {
            let actual = to_hex(&sha2::Sha256::digest(&data));
            if !actual.eq_ignore_ascii_case(&expected) {
                return Err(ZapError::New(
                    -1,
                    format!("发行包校验失败：sha256 不匹配（期望 {expected}，实际 {actual}）"),
                ));
            }
        }
        // 解包（发行包来自可信渠道，Archive::unpack 足够）
        std::fs::create_dir_all(&stage).map_err(|e| {
            // 只说「Permission denied」没人修得动：升级区被 zapd（zapadm）与
            // zapupgrade（root）两种身份混写，属主一旦不一致就是这里炸。
            ZapError::New(
                -1,
                format!(
                    "创建升级目录失败: {e}（{}）—— 该目录必须对运行 zapd 的账号可写；\
                     修复：chown -R zapadm:zapadm {}",
                    stage.display(),
                    upgrade_dir().display()
                ),
            )
        })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_allowed_requires_https_unless_insecure() {
        // 默认（insecure=false）只允许 https
        assert!(channel_allowed(
            "https://mirrors.zap.cn/zap/releases",
            false
        ));
        assert!(!channel_allowed(
            "http://mirrors.zap.cn/zap/releases",
            false
        ));
        assert!(!channel_allowed("ftp://example.com/releases", false));
        assert!(!channel_allowed("mirrors.zap.cn/zap/releases", false));
        // insecure 模式放行任意 scheme
        assert!(channel_allowed("http://10.0.0.5/releases", true));
        assert!(channel_allowed("https://mirrors.zap.cn/zap/releases", true));
    }
}
