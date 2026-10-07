// SPDX-License-Identifier: AGPL-3.0-only
use std::path::Path;
use std::process::Command;

use tracing::{info, warn};

/// Check if certificate files exist, generate self-signed certs if missing.
/// Returns true if certificates are ready (either existed or were generated).
///
/// 注意：zapd 通常以 zapadm 身份运行（见 zapd.service 的 `User=`），
/// 因此 cert_file / key_file 所在目录必须对该用户可写，否则会生成失败。
pub fn ensure_certs(cert_file: &str, key_file: &str) -> bool {
    if Path::new(cert_file).exists() && Path::new(key_file).exists() {
        return true;
    }

    info!(
        "TLS certificate files not found, generating self-signed certificate: {} / {}",
        cert_file, key_file
    );

    // openssl / rcgen 都不会自动建目录，先补齐父目录
    if let Err(e) = ensure_parent(cert_file).and_then(|_| ensure_parent(key_file)) {
        warn!(
            "创建证书目录失败（请确认 zapd 运行用户对该目录有写权限）: {}",
            e
        );
        return false;
    }

    // Try using openssl command first
    if try_openssl_gen(cert_file, key_file) {
        tighten_mode(cert_file, key_file);
        info!("Self-signed certificate generated successfully using openssl");
        return true;
    }

    // Fallback: try using rcgen crate
    if try_rcgen_gen(cert_file, key_file) {
        tighten_mode(cert_file, key_file);
        info!("Self-signed certificate generated successfully using rcgen");
        return true;
    }

    warn!(
        "生成自签证书失败：请确认 zapd 运行用户对 {} / {} 所在目录有写权限，或手工放置证书后重启 zapd",
        cert_file, key_file
    );
    false
}

/// 确保目标文件的父目录存在（相对路径 / 绝对路径都支持）。
fn ensure_parent(file: &str) -> std::io::Result<()> {
    match Path::new(file).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => std::fs::create_dir_all(parent),
        _ => Ok(()),
    }
}

/// 收紧证书文件权限：私钥 600、证书 644。
/// rcgen 落盘用的是默认 umask（私钥会变成 644），私钥必须只对自己可读。
fn tighten_mode(cert_file: &str, key_file: &str) {
    #[cfg(unix)]
    {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;
        // 私钥优先收紧：失败也要继续处理证书
        let _ = std::fs::set_permissions(key_file, Permissions::from_mode(0o600));
        let _ = std::fs::set_permissions(cert_file, Permissions::from_mode(0o644));
    }
    #[cfg(not(unix))]
    {
        let _ = (cert_file, key_file);
    }
}

fn try_openssl_gen(cert_file: &str, key_file: &str) -> bool {
    let output = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:4096",
            "-keyout",
            key_file,
            "-out",
            cert_file,
            "-days",
            "3650",
            "-nodes",
            "-subj",
            "/CN=zap-local",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            warn!("openssl failed: {}", String::from_utf8_lossy(&o.stderr));
            false
        }
        Err(e) => {
            warn!("openssl not available: {}", e);
            false
        }
    }
}

fn try_rcgen_gen(cert_file: &str, key_file: &str) -> bool {
    // Use rcgen to generate a self-signed certificate
    let mut params = rcgen::CertificateParams::default();
    params
        .subject_alt_names
        .push(rcgen::SanType::DnsName("localhost".try_into().unwrap()));
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(std::net::IpAddr::V4(
            std::net::Ipv4Addr::new(127, 0, 0, 1),
        )));

    let key_pair = match rcgen::KeyPair::generate() {
        Ok(kp) => kp,
        Err(e) => {
            warn!("rcgen key generation failed: {}", e);
            return false;
        }
    };

    let cert = match params.self_signed(&key_pair) {
        Ok(c) => c,
        Err(e) => {
            warn!("rcgen cert generation failed: {}", e);
            return false;
        }
    };

    // Write private key
    match std::fs::write(key_file, key_pair.serialize_pem()) {
        Ok(_) => {}
        Err(e) => {
            warn!("failed to write key file: {}", e);
            return false;
        }
    }

    // Write certificate
    match std::fs::write(cert_file, cert.pem()) {
        Ok(_) => {}
        Err(e) => {
            warn!("failed to write cert file: {}", e);
            return false;
        }
    }

    true
}
