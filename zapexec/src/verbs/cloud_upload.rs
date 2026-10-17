// SPDX-License-Identifier: AGPL-3.0-only
//! 备份异地上传（opendal）：由 zapexec（root）读取 root 属主的归档并推到云存储。
//!
//! 复用与 zapd `zap::cloud` 完全一致的 opendal 接入（同一份 `CloudStore` JSON 落盘格式与
//! `zap_crypto` 凭据加密方案），但与面板进程分离：备份归档权限为 `0600` root，zapd（zapadm）
//! 无权直读，因此上传必须在 root 侧完成。这里只实现「建算子 + 流式上传」所需的最小子集，
//! 不引入 list/download/rename 等面板侧能力。

use std::path::{Path, PathBuf};

use opendal::Operator;
use opendal::services;
use zap_proto::Response;

/// 同 zapd `zap::cloud::CloudService`（serde snake_case），仅用于解析 stores JSON。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum CloudService {
    AwsS3,
    Oss,
    Cos,
    S3Compat,
}

/// 同 zapd `zap::cloud::Credentials`：落盘密文解密后的明文凭据。
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct Credentials {
    access_key_id: String,
    secret_access_key: String,
    #[serde(default)]
    security_token: String,
}

/// 同 zapd `zap::cloud::CloudStore`：落盘结构 `stores/<id>.json`（只取上传所需字段，
/// 其余字段由 serde 自动忽略）。
#[derive(Debug, Clone, serde::Deserialize)]
struct CloudStore {
    #[serde(default)]
    name: String,
    service: CloudService,
    #[serde(default)]
    endpoint: String,
    #[serde(default)]
    region: String,
    bucket: String,
    #[serde(default)]
    root: String,
    #[serde(default)]
    virtual_host_style: bool,
    /// 加密后的凭据（`v1:<nonce>:<ct>`），与 zapd 同一套 `zap_crypto` 方案。
    secret: String,
}

impl CloudStore {
    /// 解密凭据（明文只在内存中短暂存在）。
    fn credentials(&self) -> Result<Credentials, String> {
        let plain = zap_crypto::decrypt(&self.secret).map_err(|e| format!("凭据解密失败：{e}"))?;
        if plain.is_empty() {
            return Err("凭据为空，请重新保存该云存储的密钥".to_string());
        }
        serde_json::from_str(&plain).map_err(|e| format!("凭据解析失败：{e}"))
    }
}

/// 安全校验用户名（云存储目录名）：仅允许字母数字与 `. - _`，防路径穿越。
fn safe_owner(name: &str) -> Result<&str, String> {
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Err("云存储所属用户名不合法".to_string());
    }
    Ok(name)
}

/// 校验云存储 ID：16 位十六进制（与 zapd `safe_store_id` 一致）。
fn safe_store_id(id: &str) -> Result<String, String> {
    let id = id.trim();
    if id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(id.to_ascii_lowercase())
    } else {
        Err("云存储 ID 不合法".to_string())
    }
}

/// 计算 stores 文件路径：`{ZAP_PATH}/data/users/<owner>/cloud/stores/<id>.json`。
fn store_path(owner: &str, id: &str) -> Result<PathBuf, String> {
    let owner = safe_owner(owner)?;
    let id = safe_store_id(id)?;
    let base = std::env::var("ZAP_PATH").unwrap_or_else(|_| "/usr/local/zap".to_string());
    Ok(Path::new(&base)
        .join("data")
        .join("users")
        .join(owner)
        .join("cloud")
        .join("stores")
        .join(format!("{id}.json")))
}

/// 读取并解析云存储配置（root 可读任意用户目录）。
fn read_store(owner: &str, id: &str) -> Result<CloudStore, String> {
    let path = store_path(owner, id)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取云存储配置失败：{e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("解析云存储配置失败：{e}"))
}

/// 安装 opendal 进程级默认 HTTP 传输层（幂等，与 zapd 一致）。
fn ensure_http_transport() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(opendal::install_default);
}

/// 构造 opendal `Operator`（每次操作瞬时构造，配置改动立即生效）。
fn build_operator(store: &CloudStore) -> Result<Operator, String> {
    ensure_http_transport();
    let cred = store.credentials()?;
    let root = if store.root.is_empty() {
        "/".to_string()
    } else {
        format!("/{}/", store.root)
    };
    let op = match store.service {
        CloudService::AwsS3 | CloudService::S3Compat => {
            let mut b = services::S3::default()
                .bucket(&store.bucket)
                .root(&root)
                .access_key_id(&cred.access_key_id)
                .secret_access_key(&cred.secret_access_key)
                // 不读 ~/.aws 配置与实例元数据：存储配置必须自洽
                .disable_config_load()
                .disable_ec2_metadata();
            if !store.endpoint.is_empty() {
                b = b.endpoint(&store.endpoint);
            }
            if !store.region.is_empty() {
                b = b.region(&store.region);
            }
            if store.virtual_host_style {
                b = b.enable_virtual_host_style();
            }
            if !cred.security_token.is_empty() {
                b = b.session_token(&cred.security_token);
            }
            Operator::new(b).map_err(|e| format!("初始化 S3 客户端失败：{e}"))?
        }
        CloudService::Oss => {
            let mut b = services::Oss::default()
                .bucket(&store.bucket)
                .root(&root)
                .endpoint(&store.endpoint)
                .access_key_id(&cred.access_key_id)
                .access_key_secret(&cred.secret_access_key)
                .addressing_style(if store.virtual_host_style {
                    "virtual"
                } else {
                    "path"
                });
            if !cred.security_token.is_empty() {
                b = b.security_token(&cred.security_token);
            }
            Operator::new(b).map_err(|e| format!("初始化 OSS 客户端失败：{e}"))?
        }
        CloudService::Cos => {
            let mut b = services::Cos::default()
                .bucket(&store.bucket)
                .root(&root)
                .endpoint(&store.endpoint)
                .secret_id(&cred.access_key_id)
                .secret_key(&cred.secret_access_key)
                .disable_config_load();
            if !cred.security_token.is_empty() {
                b = b.security_token(&cred.security_token);
            }
            Operator::new(b).map_err(|e| format!("初始化 COS 客户端失败：{e}"))?
        }
    };
    Ok(op)
}

/// 把本地文件流式上传成云对象（与 zapd `upload_from_path` 同款分块策略）。
async fn upload_file(op: &Operator, target: &str, local: &Path) -> Result<u64, String> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(local)
        .await
        .map_err(|e| format!("读取本地文件失败：{e}"))?;
    let mut writer = op
        .writer(target)
        .await
        .map_err(|e| format!("开始写入对象失败：{e}"))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut written = 0u64;
    loop {
        let read = file
            .read(&mut buf)
            .await
            .map_err(|e| format!("读取本地文件失败：{e}"))?;
        if read == 0 {
            break;
        }
        writer
            .write(bytes::Bytes::copy_from_slice(&buf[..read]))
            .await
            .map_err(|e| format!("写入对象失败：{e}"))?;
        written += read as u64;
    }
    writer
        .close()
        .await
        .map_err(|e| format!("写入对象失败：{e}"))?;
    Ok(written)
}

/// 把备份归档推送到绑定的云存储（best-effort 由调用方处理；本函数返回具体错误）。
///
/// 同时上传同目录的 `.sha256` / `.manifest` 侧车，使远端副本也能独立校验完整性。
pub(crate) async fn cloud_push(
    path: String,
    cloud_id: String,
    owner: String,
    backup_root: String,
) -> Response {
    let p = match crate::verbs::backup::assert_in_root(&path, &backup_root) {
        Ok(p) => p,
        Err(e) => return Response::err(-1, e),
    };
    if !p.is_file() {
        return Response::err(-1, "归档不存在或不是文件".to_string());
    }
    let store = match read_store(&owner, &cloud_id) {
        Ok(s) => s,
        Err(e) => return Response::err(-1, format!("解析云存储失败：{e}")),
    };
    let op = match build_operator(&store) {
        Ok(o) => o,
        Err(e) => return Response::err(-1, e),
    };
    let basename = match p.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_string(),
        None => return Response::err(-1, "归档文件名非法".to_string()),
    };
    // 命名空间隔离：每个用户的备份落在 `zap-backup/<owner>/` 下，桶内其它用户不可见。
    let key = format!("zap-backup/{owner}/{basename}");
    match upload_file(&op, &key, &p).await {
        Ok(n) => {
            let mut sidecars = Vec::new();
            for ext in [".sha256", ".manifest"] {
                let sc = p.with_file_name(format!("{basename}{ext}"));
                if sc.is_file() {
                    let sc_key = format!("zap-backup/{owner}/{basename}{ext}");
                    if let Ok(m) = upload_file(&op, &sc_key, &sc).await {
                        sidecars.push(serde_json::json!({ "key": sc_key, "size": m }));
                    }
                }
            }
            Response::ok(
                "已推送到云存储",
                Some(serde_json::json!({
                    "store": store.name,
                    "key": key,
                    "size": n,
                    "sidecars": sidecars,
                })),
            )
        }
        Err(e) => Response::err(-1, format!("推送云存储失败：{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_id_validation() {
        assert!(safe_store_id("0123456789abcdef").is_ok());
        assert!(safe_store_id("0123456789ABCDEF").is_ok());
        assert!(safe_store_id("5").is_err());
        assert!(safe_store_id("xyz").is_err());
        assert!(safe_store_id("0123456789abcdef0").is_err());
    }

    #[test]
    fn owner_validation_rejects_traversal() {
        assert!(safe_owner("alice").is_ok());
        assert!(safe_owner("bob.1").is_ok());
        assert!(safe_owner("../root").is_err());
        assert!(safe_owner("a/b").is_err());
    }
}
