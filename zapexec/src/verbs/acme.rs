//! ACME HTTP-01 验证文件托管（Let's Encrypt 申请时由 zapd 调用）。
//!
//! 背景：生产环境 80 端口通常被 nginx 常驻占用，zapd 无法像以往那样临时独占该端口，
//! 因此改为「webroot 模式」：
//!
//! - 每个站点 vhost 都渲染了固定片段（见 `site::render_acme_location`）
//!   `location ^~ /.well-known/acme-challenge/ { alias <验证根>/.well-known/acme-challenge/; }`
//! - 本模块只负责在验证根下**增删 token 文件**（root 特权写，nginx worker 只读），
//!   全过程不需要改动 nginx 配置、也不需要 reload
//!
//! 安全边界：
//! - token 只放行 `[A-Za-z0-9_-]`（长度 ≤ 128），杜绝 `..` / 斜杠等路径穿越；
//! - 写入内容即及以下挑战值，长度上限 4KB；
//! - 目录 0755、文件 0644，保证 nginx worker（www）可以读取。

use std::path::{Path, PathBuf};

use serde_json::json;
use zap_proto::{AcmeChallengeEntry, Response};

use super::site;

/// 单份验证材料的体积上限（key authorization 实际只有几百字节）。
const MAX_ENTRY_BYTES: usize = 4096;
/// token 长度上限（ACME 服务端 token 为 ~43 字符）。
const MAX_TOKEN_LEN: usize = 128;

/// 挑战文件所在目录：`{ZAP_PATH}/data/www/_zap/acme/.well-known/acme-challenge`。
fn challenge_dir() -> PathBuf {
    site::acme_webroot().join(".well-known/acme-challenge")
}

/// token 合法性：仅 `[A-Za-z0-9_-]`，防止拼出越界的相对路径。
fn valid_token(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= MAX_TOKEN_LEN
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 按 tenant 权限创建目录树（0755，nginx worker 走 others 位读取）。
fn ensure_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(dir).map_err(|e| format!("创建 ACME 验证目录失败: {e}"))?;
    let mut cur = dir.to_path_buf();
    for _ in 0..4 {
        let _ = std::fs::set_permissions(&cur, std::fs::Permissions::from_mode(0o755));
        if cur.parent().is_none() || cur == Path::new("/") {
            break;
        }
        let Some(parent) = cur.parent().map(|p| p.to_path_buf()) else {
            break;
        };
        cur = parent;
    }
    Ok(())
}

/// `acme.http_write`：把 token → keyAuth 写入验证目录。
pub async fn http_write(entries: Vec<AcmeChallengeEntry>) -> Response {
    tokio::task::spawn_blocking(move || {
        if entries.is_empty() {
            return Response::err(-1, "没有需要写入的验证材料");
        }
        let dir = challenge_dir();
        if let Err(e) = ensure_dir(&dir) {
            return Response::err(-1, e);
        }
        let mut written = 0usize;
        for e in entries {
            let token = e.token.trim();
            if !valid_token(token) {
                return Response::err(-1, format!("非法的验证 token：'{token}'"));
            }
            if e.key_auth.len() > MAX_ENTRY_BYTES {
                return Response::err(-1, format!("验证内容过长（>{MAX_ENTRY_BYTES} 字节）"));
            }
            let file = dir.join(token);
            if let Err(err) = std::fs::write(&file, e.key_auth.as_bytes()) {
                return Response::err(-1, format!("写入验证文件失败: {err}"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644));
            }
            written += 1;
        }
        Response::ok(
            "ok",
            Some(json!({
                "written": written,
                "challenge_dir": dir.to_string_lossy(),
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// `acme.http_clear`：清理验证文件（幂等，仅在必要时报错）。
pub async fn http_clear(tokens: Vec<String>) -> Response {
    tokio::task::spawn_blocking(move || {
        let dir = challenge_dir();
        let mut removed = 0usize;
        for raw in tokens {
            let token = raw.trim();
            if !valid_token(token) {
                continue;
            }
            let file = dir.join(token);
            if file.is_file() && std::fs::remove_file(&file).is_ok() {
                removed += 1;
            }
        }
        Response::ok("ok", Some(json!({ "removed": removed })))
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_whitelist() {
        assert!(valid_token("aZ09-_"));
        assert!(!valid_token(""));
        assert!(!valid_token("../etc"));
        assert!(!valid_token("a/b"));
        assert!(!valid_token("a.b"));
        assert!(!valid_token(&"x".repeat(MAX_TOKEN_LEN + 1)));
    }

    #[test]
    fn challenge_dir_is_under_webroot() {
        let d = challenge_dir();
        assert!(d.ends_with(".well-known/acme-challenge"));
        assert!(d.starts_with(site::acme_webroot()));
    }
}
