//! 包下载源配置：`{data}/mirror.yaml`。
//!
//! ## 为什么要有它
//!
//! 应用商店的安装脚本要从镜像取源码包（nginx / php / mysql / pcre2 / …）。
//! `mirrors.zap.sh`、离线机房根本没有外网 —— 那种环境里源是一个**本地目录**
//! （运维把同样的包按相同目录结构预先放进去，如 `/opt/zap-pkg/php/php-8.3.6.tar.gz`）。
//! Mirrors:
//! - 国内：`https://mirrors.zap.cn/pkg`（默认）
//! - Cloudflare：`https://mirrors.zap.sh/pkg`（海外)
//! - 本地目录：`/opt/zap-pkg` 或 `file:///opt/zap-pkg`（离线机房）
//!
//! 与 `update_config.yaml` 同级（`{data}/`，即 zap.db 旁边）：
//! - **zapd 写**：面板「系统设置 → 下载源」；
//! - **zapexec 读**：执行包脚本时注入 `ZAP_PKG_MIRROR`（见 `verbs/appstore::base_env`）。
//!
//! 脚本侧由 `bash_utils::pkg_mirror()` 消费，未注入时回落国内镜像；
//! `fetch_file` 认得 `/path` 与 `file:///path`，本地目录源直接 `cp`，不发网络请求。
//!
//! 缓存策略与 `update_config` 一致：读取前比对 mtime，手工编辑 / `zapctl` 改过无需重启。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::zap::appstore::data_dir;

/// 文件名（位于 `{data}/`）。
pub const FILE_NAME: &str = "mirror.yaml";

/// 国内镜像（默认）。
pub const MIRROR_CN: &str = "https://mirrors.zap.cn/pkg";
/// Cloudflare 镜像：海外 / 国内源不通时的备选。
pub const MIRROR_CF: &str = "https://mirrors.zap.sh/pkg";
/// 未配置时的取值。
pub const DEFAULT_PKG_MIRROR: &str = MIRROR_CN;

const MAX_LEN: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorFile {
    #[serde(default = "default_version")]
    pub version: i64,
    /// 最后一次写入时间。
    #[serde(default)]
    pub updated_at: i64,
    /// 源码包 base：`https://…/pkg` 或本地目录（`/opt/zap-pkg` / `file:///opt/zap-pkg`）。
    #[serde(default = "default_pkg_mirror")]
    pub pkg_mirror: String,
}

impl Default for MirrorFile {
    fn default() -> Self {
        Self {
            version: default_version(),
            updated_at: 0,
            pkg_mirror: default_pkg_mirror(),
        }
    }
}

fn default_version() -> i64 {
    1
}
fn default_pkg_mirror() -> String {
    DEFAULT_PKG_MIRROR.to_string()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── 进程内缓存 ──────────────────────────────────────────────

struct State {
    file: MirrorFile,
    mtime: u128,
    loaded: bool,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            file: MirrorFile::default(),
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
fn read_from(path: &Path) -> MirrorFile {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_yaml::from_str::<MirrorFile>(&text) {
            Ok(f) => normalize(f),
            Err(e) => {
                warn!("{} 解析失败，按默认下载源处理: {e}", path.display());
                MirrorFile::default()
            }
        },
        Err(_) => MirrorFile::default(),
    }
}

fn normalize(mut f: MirrorFile) -> MirrorFile {
    if f.version == 0 {
        f.version = default_version();
    }
    // 手工编辑可能删掉值，也可能填了个不合法的：不合法就退回默认，
    match validate(&f.pkg_mirror) {
        Ok(v) => f.pkg_mirror = v,
        Err(_) => f.pkg_mirror = default_pkg_mirror(),
    }
    f
}

/// 原子写回（tmp + rename），返回写入后的 mtime。
fn write_to(path: &Path, file: &MirrorFile) -> u128 {
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
        // zapd 与 zapexec 都要读（zapexec 以 root 运行，权限对它无碍）
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640));
    }
    mtime_of(path)
}

fn refresh_locked(st: &mut State) {
    let p = path();
    let mt = mtime_of(&p);
    if !st.loaded || mt != st.mtime {
        st.file = read_from(&p);
        st.mtime = mt;
        st.loaded = true;
    }
}

/// 当前生效的下载源（未配置 / 配置损坏时为国内镜像）。
pub fn pkg_mirror() -> String {
    let mut st = lock();
    refresh_locked(&mut st);
    st.file.pkg_mirror.clone()
}

/// 面板可读的完整信息（含两个内置源的预设，前端直接拿去渲染下拉）。
pub fn info() -> serde_json::Value {
    let mut st = lock();
    refresh_locked(&mut st);
    let local = is_local(&st.file.pkg_mirror);
    serde_json::json!({
        "pkgMirror": st.file.pkg_mirror,
        "updatedAt": st.file.updated_at,
        "path": path().to_string_lossy(),
        "isLocal": local,
        "presets": [
            { "label": "国内镜像（mirrors.zap.cn）", "value": MIRROR_CN },
            { "label": "Cloudflare 镜像（mirrors.zap.sh）", "value": MIRROR_CF },
        ],
    })
}

/// 保存下载源（先校验再落盘）。
pub fn save(raw: &str) -> Result<String, String> {
    let base = validate(raw)?;
    let mut st = lock();
    refresh_locked(&mut st);
    st.file.pkg_mirror = base.clone();
    let mt = write_to(&path(), &st.file);
    if mt == 0 {
        return Err(format!("写入 {} 失败", FILE_NAME));
    }
    st.mtime = mt;
    Ok(base)
}

/// 本地目录源（离线环境）判断：绝对路径或 `file://` 前缀。
pub fn is_local(base: &str) -> bool {
    base.starts_with('/') || base.starts_with("file://")
}

/// 校验并规范化下载源。
///
/// 只允许三种形态：
/// - `https://…` / `http://…`：远端镜像（末尾斜杠会被去掉）；
/// - `/…`：本地目录（离线环境），目录必须**已存在**；
/// - `file:///…`：同上，显式写法。
///
/// 拒绝空白与控制字符：这个值会被写进包脚本的环境变量，脏字符等于注入面。
pub fn validate(raw: &str) -> Result<String, String> {
    let base = raw.trim();
    if base.is_empty() {
        return Err("下载源不能为空".to_string());
    }
    if base.len() > MAX_LEN {
        return Err(format!("下载源过长（≤{MAX_LEN} 字符）"));
    }
    if base.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("下载源不能包含空白或控制字符".to_string());
    }

    let path_part = if let Some(rest) = base.strip_prefix("file://") {
        Some(rest.to_string())
    } else if base.starts_with('/') {
        Some(base.to_string())
    } else {
        None
    };
    if let Some(p) = path_part {
        if p.trim().is_empty() || p == "/" {
            return Err("本地目录源不能是根目录".to_string());
        }
        let d = PathBuf::from(&p);
        if !d.is_dir() {
            return Err(format!(
                "本地目录不存在或不是目录: {p}（离线源需预先放好包）"
            ));
        }
        // 规范写法统一成 file:// 前缀，脚本侧与远端 URL 一眼可分
        return Ok(format!("file://{}", p.trim_end_matches('/')));
    }

    if !base.starts_with("https://") && !base.starts_with("http://") {
        return Err(
            "下载源需为 http(s):// 地址，或本地目录（绝对路径 / file:// 开头）".to_string(),
        );
    }
    let rest = base.split("://").nth(1).unwrap_or("");
    if rest.is_empty() || rest.starts_with('/') {
        return Err("下载源缺少主机名".to_string());
    }
    Ok(base.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 内置源都能通过校验() {
        assert_eq!(validate(MIRROR_CN).unwrap(), MIRROR_CN);
        assert_eq!(validate(MIRROR_CF).unwrap(), MIRROR_CF);
        // 末尾多余斜杠被吃掉：脚本一律拼 "/xxx"
        assert_eq!(validate("https://mirrors.zap.cn/pkg/").unwrap(), MIRROR_CN);
    }

    #[test]
    fn 本地目录源要求目录存在() {
        let dir = std::env::temp_dir().join("zap-mirror-test");
        std::fs::create_dir_all(&dir).unwrap();
        let got = validate(dir.to_string_lossy().as_ref()).unwrap();
        assert!(got.starts_with("file://"));
        assert!(is_local(&got));
        // 不存在的目录：宁可当场报错，也不要让装包时才发现取不到
        assert!(validate("/no/such/dir/for/zap").is_err());
        assert!(validate("file:///no/such/dir").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 脏输入一律拒绝() {
        assert!(validate("").is_err());
        assert!(validate("   ").is_err());
        assert!(validate("ftp://mirrors.zap.cn/pkg").is_err());
        assert!(validate("https://").is_err());
        assert!(validate("https://a b/pkg").is_err());
        assert!(validate("/").is_err());
        // 换行是控制字符：环境变量里出现它就是注入面
        assert!(validate("https://x/pkg\nZAP_USER=root").is_err());
    }

    #[test]
    fn 损坏的配置退回默认源() {
        assert_eq!(
            normalize(MirrorFile {
                pkg_mirror: "ftp://x".into(),
                ..Default::default()
            })
            .pkg_mirror,
            MIRROR_CN
        );
        assert_eq!(
            normalize(MirrorFile {
                pkg_mirror: "".into(),
                ..Default::default()
            })
            .pkg_mirror,
            MIRROR_CN
        );
    }
}
