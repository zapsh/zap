// SPDX-License-Identifier: AGPL-3.0-only
//! 本机系统用户的 SSH 公钥授权 verb。
//!
//! 面板用户的 SSH 密钥**只存在于本人家目录** `~/.ssh/zap_<name>`（私钥/公钥），
//! 本模块不参与密钥的生成、导入与存储，只负责在「推送公钥」时把公钥内容写入
//! 本机某个系统用户的 `~/.ssh/authorized_keys`（root 特权）。
//!
//! authorized_keys 的属主必须是登录用户本人（sshd 严格校验），故写入后以 root
//! 身份 chown 并按 0600 保存。全程只涉及公钥，不涉及私钥。

use std::path::PathBuf;

use zap_proto::Response;

/// 校验系统用户名，防路径穿越。
fn valid_username(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= 64
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

/// 从 /etc/passwd 解析用户 uid/gid/home。
fn user_info(username: &str) -> Option<(u32, u32, PathBuf)> {
    if username == "root" {
        return Some((0, 0, PathBuf::from("/root")));
    }
    let content = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in content.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 6 && fields[0] == username {
            let uid: u32 = fields[2].parse().ok()?;
            let gid: u32 = fields[3].parse().ok()?;
            return Some((uid, gid, PathBuf::from(fields[5])));
        }
    }
    None
}

/// 追加指定公钥内容到本机系统用户的 `~/.ssh/authorized_keys`。
/// 用于「用户自己的家目录密钥」的本地回环授权：zapd 完成归属校验后下发公钥内容，
/// 由 root 写入目标系统用户，避免 zapd（zapadm）直接读取用户家目录。
pub async fn install_pub(username: String, public_key: String) -> Response {
    tokio::task::spawn_blocking(move || {
        match append_pub_to_local_user(&username, public_key.trim()) {
            Ok(msg) => Response::ok(msg, None),
            Err(e) => Response::err(-1, e),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 把单行公钥追加到本机用户 authorized_keys 的公共实现。
fn append_pub_to_local_user(username: &str, pub_line: &str) -> Result<String, String> {
    if !valid_username(username) {
        return Err("无效的系统用户名".to_string());
    }
    if pub_line.trim().is_empty() {
        return Err("公钥内容为空".to_string());
    }
    let (uid, gid, home) =
        user_info(username).ok_or_else(|| format!("系统用户 '{username}' 不存在"))?;

    let ssh_dir = home.join(".ssh");
    if ssh_dir.exists() {
        let meta = std::fs::metadata(&ssh_dir).map_err(|e| format!("读取 ~/.ssh 失败: {e}"))?;
        if !meta.is_dir() {
            return Err("~/.ssh 不是目录".to_string());
        }
    } else {
        std::fs::create_dir_all(&ssh_dir).map_err(|e| format!("创建 ~/.ssh 失败: {e}"))?;
    }
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&ssh_dir, std::fs::Permissions::from_mode(0o700));
    if let Ok(c) = std::ffi::CString::new(ssh_dir.as_os_str().as_encoded_bytes()) {
        unsafe {
            libc::chown(c.as_ptr(), uid, gid);
        }
    }

    let auth_path = ssh_dir.join("authorized_keys");
    let mut existing = if auth_path.exists() {
        std::fs::read_to_string(&auth_path).unwrap_or_default()
    } else {
        String::new()
    };
    if existing.lines().any(|l| l.trim() == pub_line) {
        return Ok(format!("公钥已存在于 {username} 的 authorized_keys"));
    }
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(pub_line);
    existing.push('\n');
    std::fs::write(&auth_path, &existing).map_err(|e| format!("写入 authorized_keys 失败: {e}"))?;
    let _ = std::fs::set_permissions(&auth_path, std::fs::Permissions::from_mode(0o600));
    if let Ok(c) = std::ffi::CString::new(auth_path.as_os_str().as_encoded_bytes()) {
        unsafe {
            libc::chown(c.as_ptr(), uid, gid);
        }
    }
    Ok(format!(
        "公钥已写入本机 {username} 的 ~/.ssh/authorized_keys"
    ))
}
