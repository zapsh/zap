// SPDX-License-Identifier: AGPL-3.0-only
//! 敏感数据加密（AES-256-GCM）。
//!
//! 底层实现统一由 `zap-crypto` crate 提供：密钥文件 `/etc/zap/secret.key`
//! （开发回退 `conf/secret.key`）、密文格式 `v1:<base64(nonce)>:<base64(ciphertext)>`
//! 与 `zapctl` / `zapexec` 完全一致（同一把密钥、同一种编码，彼此可互通解密）。
//! 本模块只保留 zapd 侧的易用封装。

use tracing::warn;

pub use zap_crypto::{decrypt, encrypt};

/// 掩码展示：长值保留头尾各 2 位（`ab****yz`），短值一律 `******`。
///
/// 用于「已保存密钥/密码」的回显提示：只给足够辨识度，不泄露完整值，
/// 短值连长度也不暴露（`******`）。
pub fn mask_secret(plain: &str) -> String {
    let chars: Vec<char> = plain.chars().collect();
    if chars.len() <= 8 {
        return if chars.is_empty() {
            String::new()
        } else {
            "******".to_string()
        };
    }
    let head: String = chars[..2].iter().collect();
    let tail: String = chars[chars.len() - 2..].iter().collect();
    format!("{head}****{tail}")
}

/// 密码加密入口。
///
/// **绝不回退明文**：加密失败时返回 `Err`，调用侧据此拒绝落库（而不是把明文写进
/// 数据库）。明文存储会让任何能读库/备份的人直接拿到凭据，故这里 fail-closed。
pub fn encrypt_password(pwd: &str) -> Result<String, String> {
    encrypt(pwd).map_err(|e| {
        warn!("密码加密失败，已拒绝以明文保存: {e}");
        format!("密码加密失败：{e}（请检查主密钥文件 /etc/zap/secret.key 权限）")
    })
}

/// 密码解密入口（解密失败返回空串并告警，调用侧应拒绝使用空密码继续）。
pub fn decrypt_password(pwd: &str) -> String {
    match decrypt(pwd) {
        Ok(s) => s,
        Err(e) => {
            warn!("SSH 密码解密失败: {e}");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let plain = "s3cr3t-p@ssw0rd";
        let enc = encrypt(plain).unwrap();
        assert!(enc.starts_with("v1:"));
        assert_ne!(enc, plain);
        assert_eq!(decrypt(&enc).unwrap(), plain);
    }

    #[test]
    fn ciphertext_is_randomized() {
        // 同一明文两次加密结果不同（随机 nonce）
        let a = encrypt("same").unwrap();
        let b = encrypt("same").unwrap();
        assert_ne!(a, b);
        assert_eq!(decrypt(&a).unwrap(), decrypt(&b).unwrap());
    }

    #[test]
    fn decrypt_rejects_legacy_plaintext() {
        // 历史明文（无 v1: 前缀）必须被拒绝，而非原样回退
        assert!(decrypt("old-plain-password").is_err());
    }

    #[test]
    fn decrypt_empty() {
        assert_eq!(decrypt("").unwrap(), "");
        assert_eq!(encrypt("").unwrap(), "");
    }

    #[test]
    fn decrypt_tampered_fails() {
        let enc = encrypt("hello").unwrap();
        let tampered = format!("{}X", enc);
        assert!(decrypt(&tampered).is_err());
    }

    #[test]
    fn mask_secret_hides_value() {
        assert_eq!(mask_secret(""), "");
        // 短值不暴露长度
        assert_eq!(mask_secret("abc"), "******");
        assert_eq!(mask_secret("12345678"), "******");
        // 长值只留头尾各 2 位
        assert_eq!(mask_secret("MyP@ssw0rd123"), "My****23");
    }

    #[test]
    fn password_helpers_degrade_gracefully() {
        assert_eq!(decrypt_password(""), ""); // 明文空串原样返回
        // 不可解密的密文 → 空串（调用侧拒用）
        let bad = format!("v1:{}{}", "A".repeat(30), ":YQ==");
        assert_eq!(decrypt_password(&bad), "");
    }
}
