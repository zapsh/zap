// SPDX-License-Identifier: AGPL-3.0-only
use std::{
    fs,
    path::PathBuf,
    sync::{OnceLock, RwLock},
};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[derive(Debug, Deserialize, Serialize)]
pub struct ZapConfig {
    pub server: ServerConfig,
    pub jwt: JWTConfig,
    #[serde(default)]
    pub exec: ExecConfig,
    #[serde(default)]
    pub db: DbConfig,
    /// 备份存储配置（备份根目录等）。
    #[serde(default)]
    pub backup: BackupConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ServerConfig {
    pub address: String,
    pub port: u16,
    pub cert_file: String,
    pub key_file: String,
    /// The "url_prefix" is used to hide the backend access path. Users can customize the prefix.
    /// default is empty string, which means no prefix.
    #[serde(default)]
    pub url_prefix: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JWTConfig {
    pub jwt_secure: String,
    pub jwt_expire: u64,
}

/// `zapd` 与 `zapexec` 之间的 IPC 配置。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExecConfig {
    pub socket_path: String,
    pub secret_path: String,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            socket_path: zap_proto::DEFAULT_EXEC_SOCKET.to_string(),
            secret_path: "/etc/zap/exec.key".to_string(),
        }
    }
}

/// SQLite 数据库配置（`zapd` 与 `zapctl` 共用）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DbConfig {
    pub path: String,
    /// 集群时序库（快照历史 / 聚合 / 告警事件 / 投递日志），独立于主库。
    #[serde(default)]
    pub cluster_path: String,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            path: "data/zap.db".to_string(),
            cluster_path: String::new(),
        }
    }
}

/// 备份存储配置。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BackupConfig {
    /// 备份根目录（归档统一落在此目录下）。留空 = 默认 `{ZAP_PATH}/data/backup`。
    /// 该目录可指向另一块磁盘 / 挂载点，缓解系统盘空间不足。
    #[serde(default)]
    pub backup_dir: String,
}

const DEFAULT_JWT_SECURE: &str = "secure-key-zap-default";

/// 全局配置（惰性初始化一次）。`get_config` 兜底加载；`init_config` 在服务器启动期
/// 先行做 fail-closed 校验并写入，避免以默认/临时密钥运行。
static GLOBAL_ZAP_CONFIG: OnceLock<RwLock<ZapConfig>> = OnceLock::new();

/// 规范化 URL 前缀：去掉首尾空白与斜杠，并过滤掉空白段。
///
/// `"/zap/"` → `"zap"`，`"zap"` → `"zap"`，`""` → `""`（不启用前缀）。
/// 拼接时再补上 `/`，即 `/{prefix}`。
pub fn normalize_url_prefix(raw: &str) -> String {
    raw.trim()
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim())
        .collect::<Vec<_>>()
        .join("/")
}

/// 读取规范化后的 URL 前缀（不带首尾斜杠）。空串表示未启用前缀。
pub fn url_prefix() -> String {
    get_config()
        .read()
        .map(|c| c.server.url_prefix.clone())
        .unwrap_or_default()
}

/// 拼好的前缀路径：`/zap`；未启用时为空串（便于直接拼接路径）。
pub fn url_prefix_path() -> String {
    let p = url_prefix();
    if p.is_empty() {
        String::new()
    } else {
        format!("/{p}")
    }
}

pub fn new() -> ZapConfig {
    ZapConfig {
        server: ServerConfig {
            address: "0.0.0.0".to_string(),
            port: 2663,
            cert_file: "conf/zap.crt".to_string(),
            key_file: "conf/zap.key".to_string(),
            url_prefix: String::new(),
        },
        jwt: JWTConfig {
            jwt_secure: DEFAULT_JWT_SECURE.to_string(),
            jwt_expire: 3600,
        },
        exec: ExecConfig::default(),
        db: DbConfig::default(),
        backup: BackupConfig::default(),
    }
}

/// 配置文件路径（启动时打印，便于排查"改了配置没生效"）：
/// 1. 环境变量 `ZAP_CONFIG`（若设置）
/// 2. 生产默认 `/etc/zap/zap.yaml`（若存在）——注意它优先于 `conf/zap.yaml`
/// 3. 开发回退 `conf/zap.yaml`
pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("ZAP_CONFIG")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    let prod = PathBuf::from("/etc/zap/zap.yaml");
    if prod.exists() {
        return prod;
    }
    PathBuf::from("conf/zap.yaml")
}

/// Generate a random hex string of given byte length
fn generate_random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).expect("failed to generate random bytes");
    hex::encode(buf)
}

/// Check if JWT secret needs rotation (still using default)
fn needs_jwt_rotation(secret: &str) -> bool {
    secret == DEFAULT_JWT_SECURE || secret.is_empty()
}

/// 持久化配置到 zap.yaml；返回写盘结果（失败原因），交由调用方决定 fail-closed 行为。
fn write_config_to_file(config: &ZapConfig) -> Result<(), String> {
    save_config(config)
}

/// 写回 zap.yaml 时附带的字段说明。
///
/// YAML 序列化无法保留原文件里的注释，因此在文件头补一段说明，
/// 保证运维手工打开配置时仍能看到各字段含义（尤其是 url_prefix）。
const YAML_HEADER: &str = "\
# This is the configuration file for Zap panel (shared by zapd and zapctl)
#
# server.address    Listening address: 0.0.0.0  listen all ip 
# server.port       port
# server.cert_file  https certificate file
# server.key_file   private key file
# server.url_prefix url prefix (optional) , if not empty, the url will be like: https://<server>:2663/{url_prefix}/login 
# jwt.jwt_expire    Login credential expiration time (in seconds)
# jwt.jwt_secure    Issuance key for the credential (automatically generated on the first startup. Do not disclose it)
#
";

/// 将配置写回 zap.yaml（会覆盖原文件，注释由上面的文件头统一说明）。
pub fn save_config(config: &ZapConfig) -> Result<(), String> {
    let mut out = String::from(YAML_HEADER);
    out.push_str(&serde_yaml::to_string(config).map_err(|e| format!("配置序列化失败: {e}"))?);

    let path = config_path();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = fs::create_dir_all(parent)
    {
        return Err(format!("创建配置目录失败 {}: {e}", parent.display()));
    }
    fs::write(&path, out).map_err(|e| format!("写入配置失败 {}: {e}", path.display()))?;

    // 配置文件含 jwt.jwt_secure（签发管理员 JWT 的密钥）等敏感项，
    // 收紧为仅属主读写、同组只读（0o640），避免本地用户读取后伪造管理员 JWT。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(&path, fs::Permissions::from_mode(0o640)) {
            warn!("设置配置文件权限失败 {}: {e}", path.display());
        }
    }
    Ok(())
}

/// 在写锁内修改运行时配置并持久化到 zap.yaml（「Zap 设置」页面保存使用）。
///
/// 注意：端口 / 绑定 IP / 证书 / URL 前缀在进程启动时就已生效，
/// 改完只是落盘 + 更新内存值，真正生效仍需重启 zapd。
pub fn mutate_config<F: FnOnce(&mut ZapConfig)>(f: F) -> Result<(), String> {
    let mut guard = get_config()
        .write()
        .map_err(|e| format!("配置锁不可用: {e}"))?;
    f(&mut guard);
    save_config(&guard)
}

/// Loading configuration: Read the zap.yaml file. If necessary, rotate the default JWT key to a random value.
fn load_config() -> (ZapConfig, Option<String>) {
    let mut default_conf = new();
    let mut needs_write = false;

    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(buffer) => {
            match serde_yaml::from_str::<ZapConfig>(&buffer) {
                Ok(cnf) => {
                    default_conf.server.address = cnf.server.address;
                    default_conf.server.port = cnf.server.port;
                    default_conf.server.cert_file = cnf.server.cert_file;
                    default_conf.server.key_file = cnf.server.key_file;
                    default_conf.server.url_prefix = normalize_url_prefix(&cnf.server.url_prefix);
                    default_conf.exec = cnf.exec;

                    // Rotate JWT secret if still using default
                    if needs_jwt_rotation(&cnf.jwt.jwt_secure) {
                        let new_secret = generate_random_hex(32);
                        info!("JWT secret was using default value, generated new random key");
                        default_conf.jwt.jwt_secure = new_secret;
                        needs_write = true;
                    } else {
                        default_conf.jwt.jwt_secure = cnf.jwt.jwt_secure;
                    }
                    default_conf.jwt.jwt_expire = cnf.jwt.jwt_expire;
                    default_conf.db = cnf.db;
                }
                Err(e) => {
                    info!("failed to parse {} ({}), using defaults", path.display(), e);
                    // Generate random JWT secret for fresh config
                    default_conf.jwt.jwt_secure = generate_random_hex(32);
                    needs_write = true;
                }
            }
        }
        Err(e) => {
            info!("unable to read {} ({}), using defaults", path.display(), e);
            // Generate random JWT secret for new installation
            default_conf.jwt.jwt_secure = generate_random_hex(32);
            needs_write = true;
        }
    };

    // 需要落盘时尝试写入；失败只记录原因，由调用方决定 fail-closed。
    let write_err = if needs_write {
        write_config_to_file(&default_conf).err()
    } else {
        None
    };

    (default_conf, write_err)
}

pub fn get_config() -> &'static RwLock<ZapConfig> {
    GLOBAL_ZAP_CONFIG.get_or_init(|| {
        let (conf, write_err) = load_config();
        if let Some(e) = write_err {
            warn!("Configuration file write failed  : {e}");
        }
        RwLock::new(conf)
    })
}

/// It is called by the main process of `zapd` before binding the port.
pub fn init_config() -> Result<(), String> {
    if GLOBAL_ZAP_CONFIG.get().is_some() {
        return Ok(());
    }

    let (conf, write_err) = load_config();

    if needs_jwt_rotation(&conf.jwt.jwt_secure) {
        return Err(
            "The JWT signature key is still the default value. Please regenerate it.".to_string(),
        );
    }

    if let Some(e) = write_err {
        let path = config_path();
        return Err(format!(
            "The JWT key has been rotated but cannot be written to the configuration file {}: {e}. \
            Please fix the write permissions of zap.yaml and restart.",
            path.display()
        ));
    }

    GLOBAL_ZAP_CONFIG
        .set(RwLock::new(conf))
        .map_err(|_| "Configuration re-initialization".to_string())
}
