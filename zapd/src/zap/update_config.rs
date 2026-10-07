// SPDX-License-Identifier: AGPL-3.0-only
//! 自动更新配置：`{data}/update_config.yaml`。
//!
//! 这里取代了原来的 `update_config` 单行表，YAML 为唯一事实来源：
//! - `auto` / `cron` / `channel`：面板「系统设置 → 系统更新」保存的自动更新开关与调度；
//! - `last_check_*`：最近一次远端版本检查结果（由检查/升级流程回写）。
//!
//! 行为约定（与 `server_env.yaml` 一致）：
//! - **启动时加载**：文件缺失则按默认值创建；
//! - **变更即时落盘**：保存配置 / 记录检查结果后立即原子写回（`tmp` + `rename`）；
//! - **外部改动自动感知**：每次读取前比对 mtime，`zapctl` 或手工编辑后无需重启 zapd。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::zap::user_cron::data_dir;

/// 文件名（位于 `{data}/`，即 zap.db 同级目录）。
pub const FILE_NAME: &str = "update_config.yaml";

/// 默认更新渠道（与 build.sh 上传目录一致）。
pub const DEFAULT_CHANNEL: &str = "https://mirrors.zap.cn/zap/releases";
/// 默认调度：每天凌晨 3 点。
pub const DEFAULT_CRON: &str = "0 3 * * *";

// ── 数据结构 ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfigFile {
    #[serde(default = "default_version")]
    pub version: i64,
    /// 文件最后一次写入时间。
    #[serde(default)]
    pub updated_at: i64,
    /// 自动更新开关（0/1）。
    #[serde(default)]
    pub auto: i64,
    #[serde(default = "default_cron")]
    pub cron: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    #[serde(default)]
    pub last_check_at: i64,
    #[serde(default)]
    pub last_check_version: String,
    #[serde(default)]
    pub last_check_has_update: i64,
    #[serde(default)]
    pub last_error: String,
}

impl Default for UpdateConfigFile {
    fn default() -> Self {
        Self {
            version: default_version(),
            updated_at: 0,
            auto: 0,
            cron: default_cron(),
            channel: default_channel(),
            last_check_at: 0,
            last_check_version: String::new(),
            last_check_has_update: 0,
            last_error: String::new(),
        }
    }
}

fn default_version() -> i64 {
    1
}

fn default_cron() -> String {
    DEFAULT_CRON.to_string()
}

fn default_channel() -> String {
    DEFAULT_CHANNEL.to_string()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── 进程内缓存 ──────────────────────────────────────────────

struct State {
    file: UpdateConfigFile,
    /// 已加载内容对应的文件 mtime（纳秒）。
    mtime: u128,
    loaded: bool,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            file: UpdateConfigFile::default(),
            mtime: 0,
            loaded: false,
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    state().lock().unwrap_or_else(|e| e.into_inner())
}

// ── 文件 IO ─────────────────────────────────────────────────

pub fn path() -> PathBuf {
    data_dir().join(FILE_NAME)
}

fn mtime_of(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// 读取指定路径；不存在 / 损坏时返回默认配置（保证启动不失败）。
fn read_from(path: &Path) -> UpdateConfigFile {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_yaml::from_str::<UpdateConfigFile>(&text) {
            Ok(f) => normalize(f),
            Err(e) => {
                warn!("{} 解析失败，按默认配置处理: {e}", path.display());
                UpdateConfigFile::default()
            }
        },
        Err(_) => UpdateConfigFile::default(),
    }
}

/// 补全空字段（手工编辑可能删掉某些键）。
fn normalize(mut f: UpdateConfigFile) -> UpdateConfigFile {
    if f.version == 0 {
        f.version = default_version();
    }
    if f.cron.trim().is_empty() {
        f.cron = default_cron();
    }
    if f.channel.trim().is_empty() {
        f.channel = default_channel();
    }
    f
}

/// 原子写回（tmp + rename），返回写入后的 mtime。
fn write_to(path: &Path, file: &UpdateConfigFile) -> u128 {
    let mut file = file.clone();
    if file.version == 0 {
        file.version = default_version();
    }
    file.updated_at = now();

    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!("创建目录失败 {}: {e}", parent.display());
        return 0;
    }
    let text = match serde_yaml::to_string(&file) {
        Ok(t) => t,
        Err(e) => {
            warn!("{} 序列化失败: {e}", FILE_NAME);
            return 0;
        }
    };
    let tmp = path.with_extension("yaml.tmp");
    if let Err(e) = std::fs::write(&tmp, &text) {
        warn!("写入 {} 失败: {e}", tmp.display());
        return 0;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        warn!("替换 {} 失败: {e}", path.display());
        return 0;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    mtime_of(path)
}

fn write_file(file: &UpdateConfigFile) -> u128 {
    write_to(&path(), file)
}

/// 文件被外部改写（zapctl / 手工编辑）后重新载入。
fn refresh_locked(st: &mut State) {
    if !st.loaded {
        let p = path();
        st.file = read_from(&p);
        st.mtime = mtime_of(&p);
        st.loaded = true;
        return;
    }
    let p = path();
    let mt = mtime_of(&p);
    if mt != st.mtime {
        st.file = read_from(&p);
        st.mtime = mt;
    }
}

fn persist_locked(st: &mut State) {
    let mt = write_file(&st.file);
    if mt != 0 {
        st.mtime = mt;
    }
    st.loaded = true;
}

// ── 对外 API ────────────────────────────────────────────────

/// 启动时加载；文件不存在则写入默认配置。
pub fn init() {
    let mut st = lock();
    let exists = path().exists();
    refresh_locked(&mut st);
    if !exists {
        persist_locked(&mut st);
        info!("已初始化自动更新配置: {}", path().display());
    }
}

/// 当前配置快照。
pub fn load() -> UpdateConfigFile {
    let mut st = lock();
    refresh_locked(&mut st);
    st.file.clone()
}

/// 保存自动更新开关 / 调度 / 渠道（不覆盖最近检查结果）。
pub fn save(auto: bool, cron: &str, channel: &str) {
    let mut st = lock();
    refresh_locked(&mut st);
    st.file.auto = auto as i64;
    st.file.cron = cron.to_string();
    st.file.channel = channel.to_string();
    persist_locked(&mut st);
}

/// 记录最近一次远端检查结果（不覆盖 auto/cron/channel）。
pub fn record_check(version: &str, has_update: bool, error: &str) {
    let mut st = lock();
    refresh_locked(&mut st);
    st.file.last_check_at = now();
    st.file.last_check_version = version.to_string();
    st.file.last_check_has_update = has_update as i64;
    st.file.last_error = error.to_string();
    persist_locked(&mut st);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_file(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("zap-update-config-test-{name}.yaml"));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn yaml_roundtrip() {
        let p = tmp_file("roundtrip");
        let f = UpdateConfigFile {
            auto: 1,
            cron: "30 4 * * *".to_string(),
            channel: "https://example.com/releases".to_string(),
            last_check_version: "1.2.3".to_string(),
            last_check_has_update: 1,
            ..Default::default()
        };
        write_to(&p, &f);

        let loaded = read_from(&p);
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.auto, 1);
        assert_eq!(loaded.cron, "30 4 * * *");
        assert_eq!(loaded.channel, "https://example.com/releases");
        assert_eq!(loaded.last_check_version, "1.2.3");
        assert_eq!(loaded.last_check_has_update, 1);

        let _ = std::fs::remove_file(&p);
    }

    /// 手工编辑缺失字段时用默认值兜底。
    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let p = tmp_file("missing");
        let _ = std::fs::write(&p, "auto: 1\n");
        let loaded = read_from(&p);
        assert_eq!(loaded.auto, 1);
        assert_eq!(loaded.cron, DEFAULT_CRON);
        assert_eq!(loaded.channel, DEFAULT_CHANNEL);

        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn broken_file_falls_back_to_default() {
        let p = tmp_file("broken");
        let _ = std::fs::write(&p, "auto: [not a scalar\n");
        let loaded = read_from(&p);
        assert_eq!(loaded.auto, 0);
        assert_eq!(loaded.cron, DEFAULT_CRON);

        let _ = std::fs::remove_file(&p);
    }
}
