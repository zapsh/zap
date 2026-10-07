// SPDX-License-Identifier: AGPL-3.0-only
//! 面板用户自己的 SSH 密钥管理 verb。
//!
//! 与系统级密钥（`/etc/zap/ssh`）不同，面板用户的密钥**只放在用户自己的家目录**
//! `~/.ssh/zap_<name>`（私钥）与 `~/.ssh/zap_<name>.pub`（公钥），目录 0700、
//! 私钥 0600，属主均为该 linux 用户本人。zapd（zapadm）无法也不应直接读取，
//! 连接时由本模块（root）按需读出私钥内容交回 zapd 做内存认证，私钥不落 DB。
//!
//! 安全约束：所有路径只由 /etc/passwd 解析出的家目录 + 白名单密钥名构成，
//! 杜绝路径穿越；密钥名前缀 `zap_` 避免覆盖用户自己的手工密钥。

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;

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

/// 校验密钥名：字母数字开头，仅允许字母数字 `-` `_`，长度 1..=64。
/// 避免 `.`（隐藏文件/`..` 穿越）等风险字符。
fn valid_key_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    first_ok
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn key_path(home: &Path, name: &str) -> PathBuf {
    home.join(".ssh").join(format!("zap_{name}"))
}

fn pub_key_path(home: &Path, name: &str) -> PathBuf {
    home.join(".ssh").join(format!("zap_{name}.pub"))
}

/// 确保 `~/.ssh` 存在且 0700、属主为用户本人。
fn ensure_ssh_dir(home: &Path, uid: u32, gid: u32) -> Result<(), String> {
    let dir = home.join(".ssh");
    if dir.exists() {
        let meta = std::fs::metadata(&dir).map_err(|e| format!("读取 ~/.ssh 失败: {e}"))?;
        if !meta.is_dir() {
            return Err("~/.ssh 不是目录".to_string());
        }
    } else {
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建 ~/.ssh 失败: {e}"))?;
    }
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    if let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()) {
        unsafe {
            libc::chown(c.as_ptr(), uid, gid);
        }
    }
    Ok(())
}

/// 设权限 + 属主（0600 私钥 / 0644 公钥，属主为用户本人）。
fn chown_key_file(path: &Path, uid: u32, gid: u32, mode: u32) {
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    if let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
        unsafe {
            libc::chown(c.as_ptr(), uid, gid);
        }
    }
}

fn fingerprint_of(pub_path: &Path) -> String {
    Command::new("ssh-keygen")
        .args(["-lf", &pub_path.to_string_lossy()])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let out = String::from_utf8_lossy(&o.stdout);
                out.split_whitespace().nth(1).map(|s| s.to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn read_pub_line(pub_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(pub_path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// 解析家目录 + 校验用户名，返回 (home, uid, gid)。
fn home_of(home_owner: &str) -> Result<(PathBuf, u32, u32), Response> {
    if !valid_username(home_owner) {
        return Err(Response::err(-1, "无效的系统用户名"));
    }
    match user_info(home_owner) {
        Some((uid, gid, home)) => Ok((home, uid, gid)),
        None => Err(Response::err(-1, format!("系统用户 '{home_owner}' 不存在"))),
    }
}

/// 解析家目录 + 校验用户名/密钥名，返回 (home, uid, gid)。
fn account(home_owner: &str, name: &str) -> Result<(PathBuf, u32, u32), Response> {
    if !valid_key_name(name) {
        return Err(Response::err(
            -1,
            "无效的密钥名称（仅允许字母/数字/-/_，最长 64 字符）",
        ));
    }
    home_of(home_owner)
}

fn finish_json(
    name: &str,
    key_type: &str,
    priv_path: &Path,
    pub_path: &Path,
    comment: &str,
) -> Response {
    let fingerprint = fingerprint_of(pub_path);
    let public_key = read_pub_line(pub_path).unwrap_or_default();
    let mut key_size: u32 = 0;
    if let Some(size) = fingerprint
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
    {
        key_size = size;
    }
    Response::ok(
        "密钥已保存到用户家目录 ~/.ssh",
        Some(json!({
            "name": name,
            "key_type": key_type,
            "bits": key_size,
            "fingerprint": fingerprint,
            "comment": comment,
            "public_key": public_key,
            "private_key_path": priv_path.to_string_lossy(),
        })),
    )
}

/// 生成新的密钥对。
pub async fn generate(
    linux_user: String,
    name: String,
    key_type: Option<String>,
    bits: Option<u32>,
    comment: Option<String>,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let (home, uid, gid) = match account(&linux_user, &name) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        if let Err(e) = ensure_ssh_dir(&home, uid, gid) {
            return Response::err(-1, e);
        }
        let priv_path = key_path(&home, &name);
        let pub_path = pub_key_path(&home, &name);
        if priv_path.exists() || pub_path.exists() {
            return Response::err(-1, "同名密钥已存在");
        }

        let key_type = key_type.unwrap_or_else(|| "ed25519".to_string());
        if key_type != "rsa" && key_type != "ed25519" && key_type != "ecdsa" {
            return Response::err(-1, "不支持的密钥类型（rsa / ed25519 / ecdsa）");
        }
        let comment = comment.unwrap_or_else(|| format!("{linux_user}@zap"));
        let mut cmd = Command::new("ssh-keygen");
        cmd.args(["-t", &key_type])
            .args(["-f", &priv_path.to_string_lossy()])
            .args(["-C", &comment])
            .args(["-N", ""])
            .arg("-q");
        if key_type == "rsa" {
            cmd.args(["-b", &bits.unwrap_or(4096).to_string()]);
        } else if key_type == "ecdsa" {
            cmd.args(["-b", &bits.unwrap_or(256).to_string()]);
        }

        let out = match cmd.output() {
            Ok(o) => o,
            Err(e) => return Response::err(-1, format!("ssh-keygen 执行失败: {e}")),
        };
        if !out.status.success() {
            let _ = std::fs::remove_file(&priv_path);
            let _ = std::fs::remove_file(&pub_path);
            return Response::err(
                -1,
                format!("密钥生成失败: {}", String::from_utf8_lossy(&out.stderr)),
            );
        }
        chown_key_file(&priv_path, uid, gid, 0o600);
        chown_key_file(&pub_path, uid, gid, 0o644);
        let pub_content = read_pub_line(&pub_path).unwrap_or_default();
        if pub_content.is_empty() {
            return Response::err(-1, "密钥生成成功但公钥内容为空");
        }
        finish_json(&name, &key_type, &priv_path, &pub_path, &comment)
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 导入已有私钥（可附公钥，缺失时由 root 用 ssh-keygen 从私钥推导）。
pub async fn import(
    linux_user: String,
    name: String,
    private_key: String,
    public_key: Option<String>,
    comment: Option<String>,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let (home, uid, gid) = match account(&linux_user, &name) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        if private_key.trim().is_empty() {
            return Response::err(-1, "私钥内容不能为空");
        }
        // 只接受 OpenSSH 私钥格式（BEGIN ... PRIVATE KEY）
        if !private_key.contains("PRIVATE KEY") {
            return Response::err(-1, "私钥格式不正确（请粘贴 OpenSSH 格式私钥）");
        }
        if let Err(e) = ensure_ssh_dir(&home, uid, gid) {
            return Response::err(-1, e);
        }
        let priv_path = key_path(&home, &name);
        let pub_path = pub_key_path(&home, &name);
        if priv_path.exists() || pub_path.exists() {
            return Response::err(-1, "同名密钥已存在");
        }

        let mut key_text = private_key;
        if !key_text.ends_with('\n') {
            key_text.push('\n');
        }
        if let Err(e) = std::fs::write(&priv_path, &key_text) {
            return Response::err(-1, format!("写入私钥失败: {e}"));
        }
        chown_key_file(&priv_path, uid, gid, 0o600);

        let pub_line = match public_key.filter(|p| !p.trim().is_empty()) {
            Some(p) => p.trim().to_string(),
            None => {
                let out = Command::new("ssh-keygen")
                    .args(["-y", "-f", &priv_path.to_string_lossy()])
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        String::from_utf8_lossy(&o.stdout).trim().to_string()
                    }
                    _ => {
                        let _ = std::fs::remove_file(&priv_path);
                        return Response::err(
                            -1,
                            "私钥无效或受口令保护，无法推导公钥（请同时粘贴公钥）",
                        );
                    }
                }
            }
        };
        if pub_line.is_empty() {
            let _ = std::fs::remove_file(&priv_path);
            return Response::err(-1, "公钥内容为空");
        }
        let key_type = pub_line
            .split_whitespace()
            .next()
            .unwrap_or("unknown")
            .to_string();
        let mut pub_text = pub_line;
        pub_text.push('\n');
        if let Err(e) = std::fs::write(&pub_path, &pub_text) {
            let _ = std::fs::remove_file(&priv_path);
            return Response::err(-1, format!("写入公钥失败: {e}"));
        }
        chown_key_file(&pub_path, uid, gid, 0o644);

        let comment = comment.unwrap_or_else(|| format!("{linux_user}@zap"));
        finish_json(&name, &key_type, &priv_path, &pub_path, &comment)
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 删除密钥对（文件缺失也视为成功，便于清理脏数据）。
pub async fn delete(linux_user: String, name: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let (home, _, _) = match account(&linux_user, &name) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        let priv_path = key_path(&home, &name);
        let pub_path = pub_key_path(&home, &name);
        if priv_path.exists() {
            let _ = std::fs::remove_file(&priv_path);
        }
        if pub_path.exists() {
            let _ = std::fs::remove_file(&pub_path);
        }
        Response::ok("密钥已删除", None)
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 读取私钥内容（仅终端连接认证用，经内部通道返回，前端不展示）。
pub async fn private_get(linux_user: String, name: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let (home, _, _) = match account(&linux_user, &name) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        let priv_path = key_path(&home, &name);
        match std::fs::read_to_string(&priv_path) {
            Ok(c) => Response::ok("ok", Some(json!({ "name": name, "private_key": c }))),
            Err(_) => Response::err(-1, format!("密钥 'zap_{name}' 不存在")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 家目录 `~/.ssh` 下 OpenSSH 默认私钥的探测顺序（与 ssh 客户端一致）
const DEFAULT_KEY_NAMES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// 读取用户家目录 ~/.ssh 下的默认私钥（id_ed25519 → id_ecdsa → id_rsa）。
///
/// 面板由 zapadm 运行、无权读他人 0600 私钥，故一律由 zapexec（root）代读；
/// 只扫描 `linux_user` 自己的家目录，缺失即失败，不回退到其他账户的密钥。
pub async fn default_get(linux_user: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let (home, _, _) = match home_of(&linux_user) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        let ssh_dir = home.join(".ssh");
        for name in DEFAULT_KEY_NAMES {
            let priv_path = ssh_dir.join(name);
            let Ok(content) = std::fs::read_to_string(&priv_path) else {
                continue;
            };
            if !content.contains("PRIVATE KEY") {
                continue;
            }
            let public_key =
                read_pub_line(&ssh_dir.join(format!("{name}.pub"))).unwrap_or_default();
            return Response::ok(
                "ok",
                Some(json!({
                    "name": name,
                    "private_key": content,
                    "public_key": public_key,
                })),
            );
        }
        Response::err(
            -1,
            format!(
                "{linux_user} 的家目录 ~/.ssh 下没有可用的默认私钥（id_ed25519 / id_ecdsa / id_rsa）"
            ),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 读取公钥内容。
pub async fn public_get(linux_user: String, name: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let (home, _, _) = match account(&linux_user, &name) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        let pub_path = pub_key_path(&home, &name);
        match read_pub_line(&pub_path) {
            Some(pub_line) => {
                let comment = pub_line.split_whitespace().nth(2).unwrap_or_default();
                Response::ok(
                    "ok",
                    Some(json!({
                        "name": name,
                        "public_key": pub_line,
                        "comment": comment,
                        "fingerprint": fingerprint_of(&pub_path),
                    })),
                )
            }
            None => Response::err(-1, format!("公钥 'zap_{name}.pub' 不存在")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 扫描家目录列出全部面板密钥（`~/.ssh/zap_<name>.pub`）。
///
/// 这是「我的密钥」列表的**唯一权威来源**：密钥只以家目录文件存在，不落数据库，
/// 因此库重建/迁移后列表依然完整。只读 `.pub`，私钥内容绝不外传。
pub async fn list(linux_user: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let (home, _, _) = match home_of(&linux_user) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        let dir = home.join(".ssh");
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // ~/.ssh 还不存在 = 尚未创建任何密钥
            Err(_) => return Response::ok("ok", Some(json!({ "items": [] }))),
        };
        let mut items: Vec<(String, serde_json::Value)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(name) = file_name
                .strip_prefix("zap_")
                .and_then(|s| s.strip_suffix(".pub"))
            else {
                continue;
            };
            if !valid_key_name(name) {
                continue;
            }
            let Some(pub_line) = read_pub_line(&path) else {
                continue;
            };
            let mut parts = pub_line.split_whitespace();
            let key_type = parts.next().unwrap_or("unknown").to_string();
            let _blob = parts.next();
            let comment = parts.next().unwrap_or_default().to_string();
            let created_at = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            items.push((
                name.to_string(),
                json!({
                    "name": name,
                    "key_type": key_type,
                    "comment": comment,
                    "fingerprint": fingerprint_of(&path),
                    "public_key": pub_line,
                    "created_at": created_at,
                }),
            ));
        }
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let list: Vec<serde_json::Value> = items.into_iter().map(|(_, v)| v).collect();
        Response::ok("ok", Some(json!({ "items": list })))
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}
