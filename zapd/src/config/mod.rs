use std::{
    fs,
    path::PathBuf,
    sync::{OnceLock, RwLock},
};

use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Deserialize, Serialize)]
pub struct ZapConfig {
    pub server: ServerConfig,
    pub jwt: JWTConfig,
    #[serde(default)]
    pub exec: ExecConfig,
    #[serde(default)]
    pub db: DbConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ServerConfig {
    pub address: String,
    pub port: u16,
    pub cert_file: String,
    pub key_file: String,
    /// 统一 URL 前缀。配置如 `zap` 后，页面与接口全部位于 `/zap/` 下
    /// （例：`https://host:2600/zap/dashboard`、`/zap/api/auth/login`）。
    /// 留空则不启用前缀，行为与之前完全一致。
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
    ///
    /// 留空 = 主库同目录下的 `cluster.db`。之所以不写死默认值：主库路径常常是
    /// 绝对路径（`/usr/local/zap/data/zap.db`），写死相对路径会随进程 CWD 漂移，
    /// 让两个库跑到不同目录去。
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

const DEFAULT_JWT_SECURE: &str = "secure-key-zap-default";

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
            port: 2600,
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

fn write_config_to_file(config: &ZapConfig) {
    if let Err(e) = save_config(config) {
        info!("failed to write config: {}", e);
    }
}

/// 写回 zap.yaml 时附带的字段说明。
///
/// YAML 序列化无法保留原文件里的注释，因此在文件头补一段说明，
/// 保证运维手工打开配置时仍能看到各字段含义（尤其是 url_prefix）。
const YAML_HEADER: &str = "\
# Zap 面板主配置文件（zapd / zapctl 共用）
#
# server.address    监听地址：0.0.0.0 表示监听全部网卡，也可指定单个 IP
# server.port       监听端口（修改后需重启 zapd 生效）
# server.cert_file  面板 HTTPS 证书路径（相对路径基于 zapd 工作目录）
# server.key_file   面板 HTTPS 私钥路径
# server.url_prefix 统一 URL 前缀，如 zap → 页面 /zap/ 、接口 /zap/api/；留空表示不启用
# jwt.jwt_expire    登录凭证有效期（秒）
# jwt.jwt_secure    凭证签发密钥（首次启动自动生成，请勿外泄）
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
    fs::write(&path, out).map_err(|e| format!("写入配置失败 {}: {e}", path.display()))
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

pub fn get_config() -> &'static RwLock<ZapConfig> {
    static GLOBAL_ZAP_CONFIG: OnceLock<RwLock<ZapConfig>> = OnceLock::new();
    GLOBAL_ZAP_CONFIG.get_or_init(|| {
        let mut default_conf = new();
        let mut needs_write = false;

        // 只读方式读取：运行用户（如 zapadm）对 zap.yaml 可能只有读权限，
        // 若按 read+write 打开，打不开就会静默退回默认配置，
        // 表现为"配置改了没生效"（端口 / 证书路径 / url_prefix 全部走默认值）。
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(buffer) => {
                match serde_yaml::from_str::<ZapConfig>(&buffer) {
                    Ok(cnf) => {
                        default_conf.server.address = cnf.server.address;
                        default_conf.server.port = cnf.server.port;
                        default_conf.server.cert_file = cnf.server.cert_file;
                        default_conf.server.key_file = cnf.server.key_file;
                        default_conf.server.url_prefix =
                            normalize_url_prefix(&cnf.server.url_prefix);
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

        // Persist updated config if changes were made
        if needs_write {
            write_config_to_file(&default_conf);
        }

        RwLock::new(default_conf)
    })
}
