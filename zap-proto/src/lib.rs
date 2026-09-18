//! `zapd`（非特权）与 `zapexec`（root）之间的共享 wire 协议。
//!
//! - 传输：Unix domain socket，长度前缀 + JSON 帧
//! - 认证：连接建立后做挑战/响应 HMAC-SHA256（共享密钥），
//!   服务端另外做 SO_PEERCRED 校验（仅允许 `zapadm` 连接）
//!
//! 注意：`Request` 只包含白名单动词，**没有**任意 shell 执行入口。

pub mod auth;
pub mod frame;
pub mod types;

pub use types::{
    AcmeChallengeEntry, DockerBuildArg, HeaderSpec, LocationSpec, Message, Request, Response,
    UpstreamServer, UpstreamSpec, docker_namespace, linux_username, normalize_image_tag,
    sanitize_site_name, valid_image_ref,
};

/// base64 编码（文件内容传输用）。
pub fn b64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// base64 解码。
pub fn b64_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s)
}
