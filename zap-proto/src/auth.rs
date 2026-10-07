// SPDX-License-Identifier: AGPL-3.0-only
use std::io;

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const SECRET_LEN: usize = 32;

/// 读取共享密钥文件（64 个 hex 字符 = 32 字节）。
pub fn load_secret(path: &str) -> io::Result<Vec<u8>> {
    let s = std::fs::read_to_string(path)
        .map_err(|e| io::Error::new(e.kind(), format!("读取密钥 {path}: {e}")))?;
    let bytes = hex::decode(s.trim()).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("密钥不是合法 hex: {e}"))
    })?;
    if bytes.len() != SECRET_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "密钥必须为 32 字节（64 hex）",
        ));
    }
    Ok(bytes)
}

/// 生成全新随机密钥（32 字节 -> 64 hex）。
pub fn generate_secret_hex() -> io::Result<String> {
    let mut buf = [0u8; SECRET_LEN];
    getrandom::getrandom(&mut buf).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(hex::encode(buf))
}

/// 生成随机挑战（16 字节 -> 32 hex）。
pub fn challenge_hex() -> io::Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(hex::encode(buf))
}

/// 计算 HMAC-SHA256 并输出 hex。
pub fn hmac_hex(secret: &[u8], data: &[u8]) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret).expect("hmac 接受任意长度密钥");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

/// 常量时间校验 hex 形式的 HMAC。
pub fn verify_hex(secret: &[u8], data: &[u8], mac_hex: &str) -> bool {
    let Ok(expected) = hex::decode(mac_hex) else {
        return false;
    };
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret).expect("hmac 接受任意长度密钥");
    mac.update(data);
    mac.verify_slice(&expected).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时目录辅助：每个测试独立目录，避免并发冲突。
    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zap-proto-auth-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn secret_hex_is_32_bytes() {
        let s = generate_secret_hex().unwrap();
        assert_eq!(s.len(), 64, "必须是 64 个 hex 字符");
        assert_eq!(hex::decode(&s).unwrap().len(), SECRET_LEN);
    }

    #[test]
    fn secret_is_random() {
        let a = generate_secret_hex().unwrap();
        let b = generate_secret_hex().unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn challenge_is_16_bytes_hex() {
        let c = challenge_hex().unwrap();
        assert_eq!(c.len(), 32);
        assert_eq!(hex::decode(&c).unwrap().len(), 16);
    }

    #[test]
    fn load_secret_accepts_valid_hex() {
        let dir = temp_dir();
        let path = dir.join("secret.key");
        std::fs::write(&path, "abcd".repeat(16)).unwrap(); // 32 字节
        assert_eq!(
            load_secret(path.to_str().unwrap()).unwrap().len(),
            SECRET_LEN
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_secret_rejects_bad_hex() {
        let dir = temp_dir();
        let path = dir.join("secret.key");
        std::fs::write(&path, "zzzzzzzz").unwrap();
        let err = load_secret(path.to_str().unwrap()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_secret_rejects_wrong_len() {
        let dir = temp_dir();
        let path = dir.join("secret.key");
        std::fs::write(&path, "abcd").unwrap(); // 只有 2 字节
        let err = load_secret(path.to_str().unwrap()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hmac_is_deterministic() {
        let secret = b"0123456789abcdef";
        assert_eq!(hmac_hex(secret, b"data"), hmac_hex(secret, b"data"));
        assert_ne!(hmac_hex(secret, b"data"), hmac_hex(secret, b"data2"));
    }

    #[test]
    fn verify_accepts_correct_and_rejects_wrong() {
        let secret = b"0123456789abcdef";
        let mac = hmac_hex(secret, b"challenge");
        assert!(verify_hex(secret, b"challenge", &mac));
        // 数据不同
        assert!(!verify_hex(secret, b"challenge-x", &mac));
        // 密钥不同
        assert!(!verify_hex(b"0123456789abcdeg", b"challenge", &mac));
    }

    #[test]
    fn verify_rejects_tampered_mac() {
        let secret = b"0123456789abcdef";
        let mac = hmac_hex(secret, b"challenge");
        let mut bad: Vec<u8> = mac.clone().into_bytes();
        bad[0] = if bad[0] == b'0' { b'1' } else { b'0' };
        let bad = String::from_utf8(bad).unwrap();
        assert!(!verify_hex(secret, b"challenge", &bad));
    }

    #[test]
    fn verify_rejects_non_hex_mac() {
        let secret = b"0123456789abcdef";
        assert!(!verify_hex(secret, b"challenge", "not-hex"));
        assert!(!verify_hex(secret, b"challenge", ""));
    }
}
