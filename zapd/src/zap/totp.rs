// SPDX-License-Identifier: AGPL-3.0-only
//! TOTP（RFC 6238）两步验证实现。
//!
//! 为避免引入较重的外部依赖（二维码/OTP 库），此处为纯 Rust 手写实现：
//! - HMAC-SHA1 + 30 秒时间步长
//! - 6 位动态码，允许 ±1 步时间窗（90 秒）
//! - Base32 密钥生成与编码

use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// 动态码位数
pub const TOTP_DIGITS: u32 = 6;
/// 时间步长（秒）
const STEP: u64 = 30;

// ── Base32（RFC 4648，无填充） ─────────────────────────────

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 8).div_ceil(5));
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &byte in data {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

fn base32_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut buffer: u32 = 0;
    let mut bits = 0;
    let mut out = Vec::new();
    for ch in input.chars() {
        if ch == '=' || ch == ' ' {
            continue;
        }
        let val = match ch.to_ascii_uppercase() {
            'A'..='Z' => ch.to_ascii_uppercase() as u32 - b'A' as u32,
            '2'..='7' => ch as u32 - b'2' as u32 + 26,
            _ => return Err(format!("非法 Base32 字符: {ch}")),
        };
        buffer = (buffer << 5) | val;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

/// 生成 20 字节随机密钥（160 位，符合 RFC 4226 推荐）。
pub fn generate_secret() -> String {
    let mut secret = [0u8; 20];
    getrandom::getrandom(&mut secret).expect("系统随机数不可用");
    base32_encode(&secret)
}

/// 计算指定时间戳（Unix 秒）对应的动态码。
fn totp_code(secret: &str, timestamp: u64) -> Result<String, String> {
    let secret_bytes = base32_decode(secret)?;
    if secret_bytes.is_empty() {
        return Err("密钥为空".to_string());
    }
    let counter = timestamp / STEP;
    let mut mac = HmacSha1::new_from_slice(&secret_bytes).map_err(|e| e.to_string())?;
    mac.update(&counter.to_be_bytes());
    let result = mac.finalize().into_bytes();
    let offset = (result[19] & 0x0f) as usize;
    let bin_code = ((result[offset] as u32 & 0x7f) << 24)
        | ((result[offset + 1] as u32) << 16)
        | ((result[offset + 2] as u32) << 8)
        | result[offset + 3] as u32;
    let code = bin_code % 10u32.pow(TOTP_DIGITS);
    Ok(format!("{:0width$}", code, width = TOTP_DIGITS as usize))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 校验验证码（允许 ±1 时间步）。
pub fn verify(secret: &str, code: &str) -> bool {
    let code = code.trim();
    if code.is_empty() || code.len() != TOTP_DIGITS as usize {
        return false;
    }
    let now = now_secs();
    for offset in 0..=1u64 {
        let ts = if offset == 0 {
            now
        } else {
            now.saturating_sub(offset * STEP)
        };
        if let Ok(c) = totp_code(secret, ts)
            && c == code
        {
            return true;
        }
    }
    false
}

/// 生成 otpauth URL（供前端生成二维码）。
pub fn otpauth_url(secret: &str, username: &str) -> String {
    format!(
        "otpauth://totp/Zap:{}?secret={}&issuer=Zap&algorithm=SHA1&digits={}&period={}",
        username, secret, TOTP_DIGITS, STEP
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 官方测试向量（SHA1）
    #[test]
    fn rfc6238_vectors() {
        // 密钥 "12345678901234567890"（ASCII）=> Base32: GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let cases = [
            (59u64, "94287082"),
            (1111111109, "07081804"),
            (1111111111, "14050471"),
            (1234567890, "89005924"),
            (2000000000, "69279037"),
            (20000000000, "65353130"),
        ];
        for (ts, expected) in cases {
            let actual = totp_code(secret, ts).unwrap();
            let exp = &expected[expected.len() - TOTP_DIGITS as usize..];
            assert_eq!(actual, exp, "timestamp={ts}");
        }
    }

    #[test]
    fn base32_roundtrip() {
        let data = b"hello world";
        let enc = base32_encode(data);
        assert_eq!(base32_decode(&enc).unwrap(), data);
    }

    #[test]
    fn verify_accepts_valid_code() {
        let secret = generate_secret();
        let now = now_secs();
        let code = totp_code(&secret, now).unwrap();
        assert!(verify(&secret, &code));
        // 前一个时间步也接受
        let prev = totp_code(&secret, now.saturating_sub(STEP)).unwrap();
        assert!(verify(&secret, &prev));
    }

    #[test]
    fn verify_rejects_wrong_code() {
        let secret = generate_secret();
        assert!(!verify(&secret, "000000"));
        assert!(!verify(&secret, ""));
        assert!(!verify(&secret, "12345"));
    }

    #[test]
    fn otpauth_url_format() {
        let url = otpauth_url("SECRET", "alice");
        assert!(url.starts_with("otpauth://totp/Zap:alice?secret=SECRET"));
        assert!(url.contains("issuer=Zap"));
    }
}
