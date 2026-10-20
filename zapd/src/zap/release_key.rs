// SPDX-License-Identifier: AGPL-3.0-only
//! 发行包签名校验：用编译期 pin 的 Ed25519 公钥验证 `{pkg}.sig` 分离签名。
//!
//! 自动更新以 root / zapadm 权限执行升级器，是整机的信任根。仅靠 sha256 不够
//! （中间人只要让 `.sha256` 取不到即可静默跳过），因此引入**经私钥签名的发布包**：
//! 二进制内固化发布公钥，下载阶段先验证签名再解包。
//!
//! - 私钥由 CI 以 `ZAP_SIGN_KEY`（Ed25519 PEM）持有，是 Repository secret，
//!   **绝不入仓库**（公钥才可公开）。历史上曾在源码注释里误提交过私钥，
//!   该私钥已视为泄露并轮换；请勿再把任何私钥写回仓库。
//! - 公钥默认固化于 [`DEFAULT_RELEASE_PUBKEY_HEX`]，构建时可用环境变量
//!   `ZAP_RELEASE_PUBKEY`（hex）覆盖以完成密钥轮换；
//! - 若构建/运行环境均未提供公钥，则跳过签名校验（仅余 sha256），并告警。

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use tracing::warn;

/// 默认发布公钥（Ed25519 原始 32 字节，hex）。
///
/// 仅用于**验签**，可安全公开编译进二进制。对应**私钥**由 CI 以 `ZAP_SIGN_KEY`
/// （Ed25519 PEM）持有，是 Repository secret，**绝不入仓库**（历史上曾在注释里
/// 误提交过私钥，已移除并轮换；见下方模块文档）。
const DEFAULT_RELEASE_PUBKEY_HEX: &str =
    "2bc517806267debebabcde93f8f54bdc950135e358eef02fed97a04a0b279a31";

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
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn pubkey_parses_to_32_bytes() {
        assert!(release_pubkey().is_some_and(|k| k.len() == 32));
    }

    // 自包含：临时生成 Ed25519 密钥对，对固定消息签名，验证 verify_release_signature
    // 的「通过 + 篡改失败 + 签名长度非法失败」三态。不绑定生产私钥，避免密钥轮换时改测试。
    #[test]
    fn verify_roundtrip_ok_and_tamper_fails() {
        // 任意 32 字节都可作为 ed25519 私钥种子（clamping 在内部完成），仅用于测试。
        let secret: [u8; 32] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f,
        ];
        let sk = SigningKey::from_bytes(&secret);
        let pk = sk.verifying_key();
        let msg = b"zap-test-package-bytes";
        let sig = hex::encode(sk.sign(msg).to_bytes());

        assert!(verify_release_signature(msg, &sig, &pk.to_bytes()).is_ok());

        // 篡改包体 → 签名校验必须失败
        let mut tampered = msg.to_vec();
        tampered.push(b'X');
        assert!(verify_release_signature(&tampered, &sig, &pk.to_bytes()).is_err());

        // 签名 hex 长度非法 → 失败
        assert!(verify_release_signature(msg, "00", &pk.to_bytes()).is_err());
    }
}
