//! 服务器运行环境状态：`{data}/server_env.yaml`。
//!
//! 这里取代了原来的 `server_env` 表，YAML 为唯一事实来源：
//! - `conf`：管理员维护的全局配置（`webserver` / `php_default` / `database` /
//!   `fpm_pool_defaults` / `user_home_root` / `basic_*` / `zap_ssl_*` …）；
//! - `auto`：zapexec 自动探测的快照（payload），由面板刷新。
//!
//! 行为约定：
//! - **启动时加载**：文件缺失则创建；快照过期（[`STARTUP_STALE_SECS`]）会在启动阶段重新探测；
//! - **默认值自动补全**：探测落地（或带快照启动）后，`conf` 里还没设过的项按探测结果直接写上
//!   （webserver / php_default / database / user_home_root）。只补「缺或空」，管理员手工设过的
//!   一律不覆盖（见 [`backfill_conf_from_payload`]）；
//! - **变更即时落盘**：保存配置 / 刷新快照后立即原子写回（`tmp` + `rename`）；
//! - **外部改动自动感知**：每次读取前比对 mtime，`zapctl env` 或手工编辑后无需重启 zapd。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};

use crate::zap::user_cron::data_dir;
use crate::zapexec;
use zap_proto::Request;

/// 文件名（位于 `{data}/`，即 zap.db 同级目录）。
pub const FILE_NAME: &str = "server_env.yaml";

/// 启动阶段：快照超过该秒数（或缺快照）则重新探测一次。
pub const STARTUP_STALE_SECS: i64 = 300;

// ── 数据结构 ────────────────────────────────────────────────

/// conf 区单条记录。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfEntry {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub remark: String,
    #[serde(default)]
    pub updated_at: i64,
}

impl ConfEntry {
    fn new(value: &str, remark: &str) -> Self {
        Self {
            value: value.to_string(),
            remark: remark.to_string(),
            updated_at: now(),
        }
    }
}

/// 自动探测区。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutoSection {
    /// 最近一次探测成功时间（秒）。
    #[serde(default)]
    pub detected_at: i64,
    /// zapexec `EnvDetect` 返回的原始 JSON。
    #[serde(default)]
    pub payload: Option<Value>,
    /// 最近一次探测失败原因（成功时清空）。
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvFile {
    #[serde(default = "default_version")]
    pub version: i64,
    /// 文件最后一次写入时间。
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub conf: BTreeMap<String, ConfEntry>,
    #[serde(default)]
    pub auto: AutoSection,
}

fn default_version() -> i64 {
    1
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── 进程内缓存 ──────────────────────────────────────────────

struct State {
    file: EnvFile,
    /// 已加载内容对应的文件 mtime（纳秒）；0 表示尚未加载。
    mtime: u128,
    /// 是否已初始化（避免启动前重复建空文件）。
    loaded: bool,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            file: EnvFile::default(),
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

fn read_file() -> (EnvFile, u128) {
    let path = path();
    let mt = mtime_of(&path);
    (read_from(&path), mt)
}

/// 读取指定路径；不存在 / 损坏时返回空配置（保证启动不失败）。
fn read_from(path: &Path) -> EnvFile {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_yaml::from_str::<EnvFile>(&text) {
            Ok(f) => f,
            Err(e) => {
                warn!("{} 解析失败，按空配置处理: {e}", path.display());
                EnvFile::default()
            }
        },
        Err(_) => EnvFile::default(),
    }
}

/// 原子写回（tmp + rename），返回写入后的 mtime。
fn write_file(file: &EnvFile) -> u128 {
    write_to(&path(), file)
}

fn write_to(path: &Path, file: &EnvFile) -> u128 {
    // 落盘前统一规范：版本号与文件更新时间
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
            warn!("server_env.yaml 序列化失败: {e}");
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
    // 含 Mail 密码等敏感项，收紧为仅属主可读写
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    mtime_of(path)
}

/// 自动补全时写下的 remark（与「面板默认配置」区分开，便于排查）。
const AUTO_REMARK: &str = "自动探测补全";
/// 新建用户的家目录默认挂载点。
const DEFAULT_USER_HOME_ROOT: &str = "/home";

/// 取 JSON 里的标量并转成文本；数字也吃（YAML 里的版本号可能被读成数字）。
fn as_text(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    (!s.is_empty()).then_some(s)
}

/// 按路径逐层取值，如 `&["webserver", "flavor"]`。
fn field(payload: &Value, path: &[&str]) -> Option<String> {
    let mut cur = payload;
    for k in path {
        cur = cur.get(*k)?;
    }
    as_text(cur)
}

/// 探测到的 Web 服务器 flavor；未部署时为 `none`，视同没探测到。
fn detected_webserver(payload: &Value) -> Option<String> {
    field(payload, &["webserver", "flavor"]).filter(|f| f != "none")
}

/// 探测到的系统默认 PHP 版本（如 `8.3`）。
fn detected_php_default(payload: &Value) -> Option<String> {
    field(payload, &["php", "default"])
}

/// 探测到的首选数据库实例名（如 `mysql`）：优先正在运行的，其次列表第一个。
fn detected_database(payload: &Value) -> Option<String> {
    let arr = payload.get("databases")?.as_array()?;
    let all: Vec<(String, bool)> = arr
        .iter()
        .filter_map(|d| {
            Some((
                field(d, &["name"])?,
                d.get("running").and_then(Value::as_bool) == Some(true),
            ))
        })
        .collect();
    all.iter()
        .find(|(_, running)| *running)
        .or_else(|| all.first())
        .map(|(name, _)| name.clone())
}

/// 按探测快照把「还没设过」的 conf 默认值补上，返回是否补写了东西。
///
/// 只对**键不存在或值为空**的项下手：管理员手工设过的（remark 为 `面板默认配置`）一律不覆盖。
/// 探测不到的项保持原状，继续走各处的内置兜底（如 `fpm_pool_defaults`）。
fn backfill_conf_from_payload(file: &mut EnvFile) -> bool {
    let Some(payload) = file.auto.payload.clone() else {
        return false;
    };
    let candidates: Vec<(&str, Option<String>)> = vec![
        ("webserver", detected_webserver(&payload)),
        ("php_default", detected_php_default(&payload)),
        ("database", detected_database(&payload)),
        ("user_home_root", Some(DEFAULT_USER_HOME_ROOT.to_string())),
    ];

    let mut filled: Vec<&str> = Vec::new();
    for (key, val) in candidates {
        let Some(v) = val else { continue };
        let exists = file
            .conf
            .get(key)
            .map(|e| !e.value.trim().is_empty())
            .unwrap_or(false);
        if exists {
            continue;
        }
        file.conf
            .insert(key.to_string(), ConfEntry::new(&v, AUTO_REMARK));
        filled.push(key);
    }
    if !filled.is_empty() {
        info!("已按运行结果补全默认配置: {}", filled.join(", "));
    }
    !filled.is_empty()
}

/// 文件被外部改写（zapctl / 手工编辑）后重新载入。
fn refresh_locked(st: &mut State) {
    if !st.loaded {
        let (file, mtime) = read_file();
        st.file = file;
        st.mtime = mtime;
        st.loaded = true;
        return;
    }
    let mt = mtime_of(&path());
    if mt != st.mtime {
        let (file, mtime) = read_file();
        st.file = file;
        st.mtime = mtime;
    }
}

fn persist_locked(st: &mut State) {
    let mt = write_file(&st.file);
    st.mtime = mt;
    st.loaded = true;
}

// ── 生命周期 ────────────────────────────────────────────────

/// 启动时加载；文件不存在则创建空文件。
///
/// 已有快照但 conf 默认值还是空的（历史数据 / 手工编辑过），顺手按快照补一次。
pub fn init() {
    let mut st = lock();
    let exists = path().exists();
    refresh_locked(&mut st);
    let filled = backfill_conf_from_payload(&mut st.file);
    if filled {
        persist_locked(&mut st);
        info!("已按探测快照补全默认配置: {}", path().display());
    } else if !exists {
        persist_locked(&mut st);
        info!("已初始化运行环境状态文件: {}", path().display());
    }
}

/// 启动阶段重探：快照缺失或超过 `max_age` 秒时调用 zapexec 重新探测。
///
/// zapexec 尚未就绪时只记 warn，不阻塞启动（面板首屏会自行触发刷新）。
pub async fn refresh_on_startup(max_age: i64) {
    if !snapshot_stale(max_age) {
        return;
    }
    match probe().await {
        Ok(v) => {
            save_snapshot(&v);
            info!("启动时已重新探测服务器运行环境");
        }
        Err(e) => warn!("启动阶段运行环境探测失败（稍后可在面板手动刷新）: {e}"),
    }
}

/// 调用 zapexec 探测服务器运行环境。
pub async fn probe() -> Result<Value, String> {
    let resp = zapexec::call(Request::EnvDetect)
        .await
        .map_err(|e| e.to_string())?;
    if resp.code != 0 {
        return Err(resp.message);
    }
    resp.data.ok_or_else(|| "环境探测未返回数据".to_string())
}

// ── conf 区（管理员配置）────────────────────────────────────

/// 全部 conf 键值（value 快照）。
pub fn conf_all() -> BTreeMap<String, String> {
    let mut st = lock();
    refresh_locked(&mut st);
    st.file
        .conf
        .iter()
        .map(|(k, v)| (k.clone(), v.value.clone()))
        .collect()
}

/// conf 区原始条目（含 remark / updated_at），供 `zapctl env list` 之类的展示。
pub fn conf_entries() -> BTreeMap<String, ConfEntry> {
    let mut st = lock();
    refresh_locked(&mut st);
    st.file.conf.clone()
}

/// 读取单个 conf 键；空值视同未设置（与原表行为一致）。
pub fn conf_get(key: &str) -> Option<String> {
    conf_all()
        .get(key)
        .cloned()
        .filter(|s| !s.trim().is_empty())
}

/// 批量写入 conf（upsert），一次落盘。
pub fn conf_set_many(items: &[(String, String)], remark: &str) {
    if items.is_empty() {
        return;
    }
    let mut st = lock();
    refresh_locked(&mut st);
    for (k, v) in items {
        st.file.conf.insert(k.clone(), ConfEntry::new(v, remark));
    }
    persist_locked(&mut st);
}

/// 写入单个 conf 键。
pub fn conf_set(key: &str, value: &str, remark: &str) {
    conf_set_many(&[(key.to_string(), value.to_string())], remark);
}

/// 删除 conf 键；返回是否确实删掉了。
pub fn conf_unset(key: &str) -> bool {
    let mut st = lock();
    refresh_locked(&mut st);
    if st.file.conf.remove(key).is_none() {
        return false;
    }
    persist_locked(&mut st);
    true
}

// ── auto 区（自动探测快照）──────────────────────────────────

/// 读取快照 `(payload, detected_at)`。
pub fn snapshot() -> (Option<Value>, i64) {
    let mut st = lock();
    refresh_locked(&mut st);
    (st.file.auto.payload.clone(), st.file.auto.detected_at)
}

/// 保存探测快照，返回检测时间。
pub fn save_snapshot(payload: &Value) -> i64 {
    let mut st = lock();
    refresh_locked(&mut st);
    let ts = now();
    st.file.auto = AutoSection {
        detected_at: ts,
        payload: Some(payload.clone()),
        error: String::new(),
    };
    // 探测落地后把还没设过的默认值一并写上：跑完首次探测就有可用的默认 Web/PHP/数据库，
    // 不用管理员先进来手工挑一遍。手工设过的键不受影响。
    backfill_conf_from_payload(&mut st.file);
    persist_locked(&mut st);
    ts
}

/// 快照是否缺失 / 超过 `max_age` 秒。
pub fn snapshot_stale(max_age: i64) -> bool {
    let (payload, detected_at) = snapshot();
    payload.is_none() || now() - detected_at > max_age
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp_file(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("zap-server-env-test-{name}.yaml"));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn yaml_roundtrip_conf_and_auto() {
        let p = tmp_file("roundtrip");
        let mut file = EnvFile::default();
        file.conf.insert(
            "php_default".to_string(),
            ConfEntry::new("8.3", "面板默认配置"),
        );
        file.auto = AutoSection {
            detected_at: 1_700_000_000,
            payload: Some(json!({"php": {"default": "8.3"}})),
            error: String::new(),
        };
        write_to(&p, &file);

        let loaded = read_from(&p);
        assert_eq!(loaded.version, 1);
        assert_eq!(
            loaded.conf.get("php_default").map(|e| e.value.as_str()),
            Some("8.3")
        );
        assert_eq!(loaded.auto.detected_at, 1_700_000_000);
        assert_eq!(
            loaded
                .auto
                .payload
                .as_ref()
                .and_then(|v| v["php"]["default"].as_str()),
            Some("8.3")
        );

        let _ = std::fs::remove_file(&p);
    }

    /// 外部改写（zapctl / 手工编辑）后 mtime 变化，能被检出并重新载入。
    #[test]
    fn external_change_is_detected_by_mtime() {
        let p = tmp_file("external");
        let file = EnvFile::default();
        let before = write_to(&p, &file);

        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut outside = read_from(&p);
        outside
            .conf
            .insert("webserver".to_string(), ConfEntry::new("nginx", "外部写入"));
        let after = write_to(&p, &outside);

        assert_ne!(before, after, "外部写入应改变 mtime");
        assert_ne!(mtime_of(&p), before);

        let reloaded = read_from(&p);
        assert_eq!(
            reloaded.conf.get("webserver").map(|e| e.value.as_str()),
            Some("nginx")
        );

        let _ = std::fs::remove_file(&p);
    }

    /// 探测落地后没设过的 conf 默认值会被补上；管理员手工设过的不覆盖。
    #[test]
    fn backfill_defaults_from_payload() {
        let mut file = EnvFile::default();
        file.conf.insert(
            "php_default".to_string(),
            ConfEntry::new("8.3", "面板默认配置"),
        );
        file.auto.payload = Some(json!({
            "webserver": {"flavor": "nginx", "running": true},
            "php": {"default": "7.4"},
            "databases": [
                {"name": "redis", "running": false},
                {"name": "mysql", "running": true},
            ],
        }));

        assert!(backfill_conf_from_payload(&mut file));
        assert_eq!(
            file.conf.get("webserver").map(|e| e.value.as_str()),
            Some("nginx")
        );
        // 手工设过的不覆盖
        assert_eq!(
            file.conf.get("php_default").map(|e| e.value.as_str()),
            Some("8.3")
        );
        // 数据库优先取正在运行的实例
        assert_eq!(
            file.conf.get("database").map(|e| e.value.as_str()),
            Some("mysql")
        );
        assert_eq!(
            file.conf.get("user_home_root").map(|e| e.value.as_str()),
            Some("/home")
        );
        assert_eq!(
            file.conf.get("webserver").map(|e| e.remark.as_str()),
            Some(AUTO_REMARK)
        );

        // 幂等：再跑一次不再改写
        assert!(!backfill_conf_from_payload(&mut file));
    }

    /// 空字符串视同「没设置」；无快照时什么都不做。
    #[test]
    fn backfill_handles_empty_value_and_no_payload() {
        let mut empty = EnvFile::default();
        empty
            .conf
            .insert("webserver".to_string(), ConfEntry::new("", "面板默认配置"));
        empty.auto.payload = Some(json!({"webserver": {"flavor": "openresty"}}));
        assert!(backfill_conf_from_payload(&mut empty));
        assert_eq!(
            empty.conf.get("webserver").map(|e| e.value.as_str()),
            Some("openresty")
        );

        let mut none = EnvFile::default();
        assert!(!backfill_conf_from_payload(&mut none));
        assert!(none.conf.is_empty());
    }

    /// 损坏文件不应导致 panic，退化为空配置。
    #[test]
    fn broken_file_falls_back_to_empty() {
        let p = tmp_file("broken");
        let _ = std::fs::write(&p, "conf: [this is not a map\n");
        let loaded = read_from(&p);
        assert!(loaded.conf.is_empty());

        let _ = std::fs::remove_file(&p);
    }
}
