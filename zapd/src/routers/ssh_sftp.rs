// SPDX-License-Identifier: AGPL-3.0-only
//! SFTP 文件管理：复用终端那条 SSH 凭据开 sftp 子系统，
//! 对远端目录做列目录 / 上传 / 下载 / 新建目录 / 删除 / 重命名。
//!
//! 设计取舍：**每次请求现开一条 SSH 连接、用完即断**。
//! 无状态就不会漏连接、也不会串会话，代价是每次操作都要握手一遍 ——
//! 文件管理是低频交互，这个代价可以接受（换来的是不残留任何长连接）。

use std::cmp::Ordering;

use axum::Json;
use axum::extract::Query;
use base64::Engine;
use russh::Disconnect;
use russh_sftp::client::SftpSession;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::routers::ssh_terminal;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::{ZapError, ZapJsonResult};

/// 单文件体积上限：内容走 JSON + base64，全程在内存里，不能放开
const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;

/// 列目录 / 下载：`path` 缺省表示远端登录用户的家目录。
#[derive(Deserialize)]
pub struct SftpPathQuery {
    pub id: i64,
    pub path: Option<String>,
}

#[derive(Deserialize)]
pub struct MkdirPayload {
    pub id: i64,
    pub path: String,
}

#[derive(Deserialize)]
pub struct RemovePayload {
    pub id: i64,
    pub path: String,
    /// 删目录还是删文件；缺省按文件处理
    pub is_dir: Option<bool>,
}

#[derive(Deserialize)]
pub struct RenamePayload {
    pub id: i64,
    pub old_path: String,
    pub new_path: String,
}

#[derive(Deserialize)]
pub struct UploadPayload {
    pub id: i64,
    /// 目标目录（空 = 家目录）
    pub path: String,
    pub name: String,
    /// 文件内容（base64）；见 `MAX_FILE_BYTES`
    pub content: String,
}

// ── 连接 ────────────────────────────────────────────────────

/// 建立 SSH 连接并打开 sftp 子系统；返回的 handle 用于操作完断开。
async fn open_sftp(conn_id: i64) -> Result<(ssh_terminal::SshConn, SftpSession), String> {
    let info = ssh_terminal::load_connection_info(conn_id)
        .await
        .map_err(|e| match &e {
            ZapError::New(_, m) => m.clone(),
            other => other.to_string(),
        })?;
    // 复用终端那套：TOFU 主机密钥校验 + 15s 建连超时 + 认证失败限流
    let handle = tokio::time::timeout(
        ssh_terminal::SSH_CONNECT_TIMEOUT,
        ssh_terminal::ssh_connect(&info),
    )
    .await
    .map_err(|_| {
        format!(
            "SSH 建连超时（{} 秒）",
            ssh_terminal::SSH_CONNECT_TIMEOUT.as_secs()
        )
    })??;
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("打开 SSH 通道失败: {e}"))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| format!("打开 sftp 子系统失败（远端可能禁用了 SFTP）: {e}"))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| format!("SFTP 会话初始化失败: {e}"))?;
    Ok((handle, sftp))
}

/// 操作完收尾：断开 SSH（每次请求一条连接，不留长连接）
async fn close(handle: ssh_terminal::SshConn) {
    let _ = handle
        .disconnect(Disconnect::ByApplication, "", "English")
        .await;
}

/// 统一的准入检查（与终端同一套规则），失败时直接给出业务错误响应。
macro_rules! need_access {
    ($claims:expr, $id:expr) => {
        if let Err((_, msg)) = ssh_terminal::check_terminal_access($claims, $id).await {
            return Ok(Json(json!({ "code": -1, "message": msg })));
        }
    };
}

// ── 接口 ────────────────────────────────────────────────────

/// GET /terminal/sftp/list —— 列出远端目录
pub async fn list_dir(claims: ValidatedClaims, Query(q): Query<SftpPathQuery>) -> ZapJsonResult {
    need_access!(&claims, q.id);
    let path = q
        .path
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| ".".to_string());

    let (handle, sftp) = match open_sftp(q.id).await {
        Ok(v) => v,
        Err(e) => return Ok(Json(json!({ "code": -1, "message": e }))),
    };
    // 相对路径解析成绝对路径，前端据此显示「当前在哪」
    let cwd = sftp
        .canonicalize(&path)
        .await
        .unwrap_or_else(|_| path.clone());
    let result = match sftp.read_dir(&path).await {
        Ok(dir) => {
            let mut items: Vec<Value> = Vec::new();
            for entry in dir {
                let meta = entry.metadata();
                items.push(json!({
                    "name": entry.file_name(),
                    "path": entry.path(),
                    "is_dir": meta.is_dir(),
                    "size": meta.size.unwrap_or(0),
                    "mtime": meta.mtime.unwrap_or(0),
                    "mode": meta.permissions.unwrap_or(0),
                }));
            }
            // 目录在前、其余按名字排（和常见文件管理器一致）
            items.sort_by(|a, b| {
                let dirs = b["is_dir"]
                    .as_bool()
                    .unwrap_or(false)
                    .cmp(&a["is_dir"].as_bool().unwrap_or(false));
                if dirs != Ordering::Equal {
                    return dirs;
                }
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["name"].as_str().unwrap_or(""))
            });
            json!({ "code": 0, "data": { "cwd": cwd, "items": items } })
        }
        Err(e) => json!({ "code": -1, "message": format!("读取目录失败: {e}") }),
    };
    close(handle).await;
    Ok(Json(result))
}

/// POST /terminal/sftp/mkdir —— 新建目录
pub async fn mkdir(claims: ValidatedClaims, Json(p): Json<MkdirPayload>) -> ZapJsonResult {
    need_access!(&claims, p.id);
    let (handle, sftp) = match open_sftp(p.id).await {
        Ok(v) => v,
        Err(e) => return Ok(Json(json!({ "code": -1, "message": e }))),
    };
    let result = match sftp.create_dir(&p.path).await {
        Ok(_) => {
            audit::log(
                Some(&claims),
                None,
                "sftp_mkdir",
                &p.path,
                &format!("SFTP 新建目录（连接 #{}）", p.id),
            )
            .await;
            json!({ "code": 0, "message": "目录已创建" })
        }
        Err(e) => json!({ "code": -1, "message": format!("新建目录失败: {e}") }),
    };
    close(handle).await;
    Ok(Json(result))
}

/// POST /terminal/sftp/remove —— 删除文件或目录
pub async fn remove(claims: ValidatedClaims, Json(p): Json<RemovePayload>) -> ZapJsonResult {
    need_access!(&claims, p.id);
    let (handle, sftp) = match open_sftp(p.id).await {
        Ok(v) => v,
        Err(e) => return Ok(Json(json!({ "code": -1, "message": e }))),
    };
    let is_dir = p.is_dir.unwrap_or(false);
    let result = if is_dir {
        match sftp.remove_dir(&p.path).await {
            Ok(_) => json!({ "code": 0, "message": "目录已删除" }),
            Err(e) => json!({ "code": -1, "message": format!("删除目录失败: {e}") }),
        }
    } else {
        match sftp.remove_file(&p.path).await {
            Ok(_) => json!({ "code": 0, "message": "文件已删除" }),
            Err(e) => json!({ "code": -1, "message": format!("删除文件失败: {e}") }),
        }
    };
    if result["code"] == 0 {
        audit::log(
            Some(&claims),
            None,
            "sftp_remove",
            &p.path,
            &format!(
                "SFTP 删除{}（连接 #{}）",
                if is_dir { "目录" } else { "文件" },
                p.id
            ),
        )
        .await;
    }
    close(handle).await;
    Ok(Json(result))
}

/// POST /terminal/sftp/rename —— 重命名 / 移动
pub async fn rename(claims: ValidatedClaims, Json(p): Json<RenamePayload>) -> ZapJsonResult {
    need_access!(&claims, p.id);
    let (handle, sftp) = match open_sftp(p.id).await {
        Ok(v) => v,
        Err(e) => return Ok(Json(json!({ "code": -1, "message": e }))),
    };
    let result = match sftp.rename(&p.old_path, &p.new_path).await {
        Ok(_) => {
            audit::log(
                Some(&claims),
                None,
                "sftp_rename",
                &p.new_path,
                &format!("SFTP 重命名：{} → {}", p.old_path, p.new_path),
            )
            .await;
            json!({ "code": 0, "message": "已重命名" })
        }
        Err(e) => json!({ "code": -1, "message": format!("重命名失败: {e}") }),
    };
    close(handle).await;
    Ok(Json(result))
}

/// POST /terminal/sftp/upload —— 上传文件（内容 base64）
pub async fn upload(claims: ValidatedClaims, Json(p): Json<UploadPayload>) -> ZapJsonResult {
    need_access!(&claims, p.id);
    let data = match base64::engine::general_purpose::STANDARD.decode(p.content.trim()) {
        Ok(d) => d,
        Err(_) => {
            return Ok(Json(
                json!({ "code": -1, "message": "内容 base64 解码失败" }),
            ));
        }
    };
    if data.is_empty() {
        return Ok(Json(json!({ "code": -1, "message": "文件内容为空" })));
    }
    if data.len() > MAX_FILE_BYTES {
        return Ok(Json(json!({
            "code": -1,
            "message": format!("文件超过 {} MB 上限", MAX_FILE_BYTES / 1024 / 1024)
        })));
    }
    // 目录 + 文件名拼成远端路径
    let dir = p.path.trim_end_matches('/');
    let target = if dir.is_empty() {
        p.name.clone()
    } else {
        format!("{dir}/{}", p.name)
    };

    let (handle, sftp) = match open_sftp(p.id).await {
        Ok(v) => v,
        Err(e) => return Ok(Json(json!({ "code": -1, "message": e }))),
    };
    let result = match sftp.write(&target, &data).await {
        Ok(_) => {
            audit::log(
                Some(&claims),
                None,
                "sftp_upload",
                &target,
                &format!("SFTP 上传 {} 字节（连接 #{}）", data.len(), p.id),
            )
            .await;
            json!({ "code": 0, "message": "上传完成", "data": { "path": target } })
        }
        Err(e) => json!({ "code": -1, "message": format!("上传失败: {e}") }),
    };
    close(handle).await;
    Ok(Json(result))
}

/// GET /terminal/sftp/download —— 下载文件（返回 base64 内容）
pub async fn download(claims: ValidatedClaims, Query(q): Query<SftpPathQuery>) -> ZapJsonResult {
    need_access!(&claims, q.id);
    let Some(path) = q.path.clone().filter(|s| !s.trim().is_empty()) else {
        return Ok(Json(json!({ "code": -1, "message": "缺少文件路径" })));
    };
    let (handle, sftp) = match open_sftp(q.id).await {
        Ok(v) => v,
        Err(e) => return Ok(Json(json!({ "code": -1, "message": e }))),
    };
    let result = match sftp.read(&path).await {
        Ok(data) => {
            if data.len() > MAX_FILE_BYTES {
                json!({
                    "code": -1,
                    "message": format!("文件超过 {} MB 上限，无法下载", MAX_FILE_BYTES / 1024 / 1024)
                })
            } else {
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                audit::log(
                    Some(&claims),
                    None,
                    "sftp_download",
                    &path,
                    &format!("SFTP 下载 {} 字节（连接 #{}）", data.len(), q.id),
                )
                .await;
                json!({
                    "code": 0,
                    "data": {
                        "name": name,
                        "content": base64::engine::general_purpose::STANDARD.encode(&data),
                    }
                })
            }
        }
        Err(e) => json!({ "code": -1, "message": format!("下载失败: {e}") }),
    };
    close(handle).await;
    Ok(Json(result))
}
