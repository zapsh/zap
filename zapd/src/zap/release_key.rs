// SPDX-License-Identifier: AGPL-3.0-only
//! 发行包签名校验：用编译期 pin 的 Ed25519 公钥验证 `{pkg}.sig` 分离签名。
//!
//! 自动更新以 root / zapadm 权限执行升级器，是整机的信任根。仅靠 sha256 不够
//! （中间人只要让 `.sha256` 取不到即可静默跳过），因此引入**经私钥签名的发布包**：
//! 二进制内固化发布公钥，下载阶段先验证签名再解包。
//!
//! - 私钥由 CI 以 `ZAP_SIGN_KEY`（Ed25519 PEM 路径）持有，绝不入仓库；
//! - 公钥默认固化于 [`DEFAULT_RELEASE_PUBKEY_HEX`]，构建时可用环境变量
//!   `ZAP_RELEASE_PUBKEY`（hex）覆盖以完成密钥轮换；
//! - 若构建/运行环境均未提供公钥，则跳过签名校验（仅余 sha256），并告警。

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use tracing::warn;

/// 默认发布公钥（Ed25519 原始 32 字节，hex）。
///
/// 对应私钥（CI secret `ZAP_SIGN_KEY`）：
/// ```text
/// -----BEGIN PRIVATE KEY-----
/// MC4CAQAwBQYDK2VwBCIEIHBAMzHq/0U+0lvvG2FVVdka7wF/6O9/pvWc7UB8OOvs
/// -----END PRIVATE KEY-----
/// ```
const DEFAULT_RELEASE_PUBKEY_HEX: &str =
    "6e1e6ed0fdef020b8e70982adae5160bc605f1f211cac2640f8c286d8eb42df9";

/// 返回编译期 pin 的发布公钥（32 字节）；环境变量 `ZAP_RELEASE_PUBKEY` 可覆盖默认。
pub fn release_pubkey() -> Option<[u8; 32]> {
    let hex = option_env!("ZAP_RELEASE_PUBKEY").unwrap_or(DEFAULT_RELEASE_PUBKEY_HEX);
    match hex::decode(hex) {
        Ok(b) if b.len() == 32 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(&b);
            Some(k)
        }
        _ => {
            warn!("发布公钥格式非法（应为 32 字节 hex），跳过签名校验");
            None
        }
    }
}

/// 校验发行包（原始字节）的 Ed25519 分离签名（hex 编码）。
pub fn verify_release_signature(data: &[u8], sig_hex: &str, pk: &[u8; 32]) -> Result<(), String> {
    let pk = VerifyingKey::from_bytes(pk).map_err(|e| format!("公钥非法: {e}"))?;
    let sig_bytes = hex::decode(sig_hex.trim()).map_err(|e| format!("签名 hex 解码失败: {e}"))?;
    let sig = Signature::from_slice(&sig_bytes).map_err(|e| format!("签名长度非法: {e}"))?;
    pk.verify(data, &sig)
        .map_err(|e| format!("签名不匹配: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pubkey_parses_to_32_bytes() {
        assert!(release_pubkey().is_some_and(|k| k.len() == 32));
    }

    // 离线用发布私钥（CI secret ZAP_SIGN_KEY）对固定消息签名得到的 hex，
    // 与编译期 pin 的公钥配对：验证「verify 通过 + 篡改包体失败 + 非法签名长度失败」。
    const MSG: &[u8] = b"zap-test-package-bytes";
    const SIG: &str = "09be644e603ac25e942e87f15f696914da13fd43d06eb175dccee84c8b66215a86f9a854975de131dc5c91ff6fe2e9fa3539a6acec5b1994f1c300affda9e90d";

    #[test]
    fn verify_with_pinned_key_ok_and_tamper_fails() {
        let pk = release_pubkey().expect("pinned pubkey present");
        assert!(verify_release_signature(MSG, SIG, &pk).is_ok());

        // 篡改包体 → 签名校验必须失败
        let mut tampered = MSG.to_vec();
        tampered.push(b'X');
        assert!(verify_release_signature(&tampered, SIG, &pk).is_err());

        // 签名 hex 长度非法 → 失败
        assert!(verify_release_signature(MSG, "00", &pk).is_err());
    }
}
