// SPDX-License-Identifier: AGPL-3.0-only
//! 云存储接口：多存储配置管理 + 对象浏览 / 上传 / 下载 / 删除 / 重命名。
//!
//! ## 隔离与安全
//!
//! - **按用户隔离**：配置读写只发生在 `{ZAP_PATH}/data/users/<user>/cloud/` 下，
//!   用户名取自 JWT（`claims.sub`），不同用户互不可见（admin 也不例外，
//!   需要看别人的存储就直接看服务器上的目录）。
//! - **桶内路径**：只做「相对逻辑根」的处理，穿越（`..`）由
//!   [`cloud::normalize_path`] 收敛，不会越出 `root` 配置的范围。
//! - **凭据**：接口只返回脱敏提示（`LTAI****cdef`），密文永不出后端。
//! - **审计**：写操作（增删改传）逐条记审计，含云存储名与对象路径。
//!
//! ## 上传
//!
//! 走 multipart 分片 → opendal `Writer` 流式落盘（[`cloud::UploadSession`]），
//! 内存占用与文件大小无关；`/system/cloud/upload` 单独放开了请求体上限
//! （axum 默认 2 MB，云存储动辄上传大文件）。

use axum::{
    Json,
    body::Body,
    extract::{Extension, Multipart, Query},
    http::{HeaderValue, header},
    response::Response,
};
use futures_util::TryStreamExt;
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;

use super::system_file;
use crate::zap::{ZapError, ZapJsonResult, audit, cloud, jwt::Claims};

// ── request types ──────────────────────────────────────────

#[derive(Deserialize)]
pub struct CloudQuery {
    /// 云存储 ID
    pub id: String,
    /// 相对逻辑根的路径（缺省 = 根）
    #[serde(default)]
    pub path: String,
    /// 是否显示以 `.` 开头的隐藏对象
    #[serde(default)]
    pub hidden: Option<bool>,
}

#[derive(Deserialize)]
pub struct StoreIdPayload {
    pub id: String,
}

#[derive(Deserialize)]
pub struct CloudPathPayload {
    pub id: String,
    pub path: String,
}

#[derive(Deserialize)]
pub struct CloudRenamePayload {
    pub id: String,
    pub path: String,
    pub new_path: String,
}

#[derive(Deserialize)]
pub struct LocalListQuery {
    /// 服务器上的目录（缺省 = 当前用户家目录）
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Deserialize)]
pub struct LocalUploadPayload {
    /// 云存储 ID
    pub id: String,
    /// 云端目标目录（相对逻辑根，缺省 = 根）
    #[serde(default)]
    pub path: String,
    /// 服务器上的本地文件路径（限自身 home 与私有 tmp，admin 不限）
    pub files: Vec<String>,
}

// ── 存储配置 ────────────────────────────────────────────────

/// GET /system/cloud/stores —— 当前用户的云存储列表 + 表单预设
pub async fn store_list(claims: Claims) -> ZapJsonResult {
    let stores = cloud::list_stores(&claims.sub)?;
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": { "stores": stores, "services": cloud::service_catalog() },
    })))
}

/// POST /system/cloud/store/save —— 新增 / 编辑云存储
pub async fn store_save(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<cloud::StoreInput>,
) -> ZapJsonResult {
    let creating = payload.id.trim().is_empty();
    let view = cloud::save_store(&claims.sub, payload)?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        if creating {
            "cloud_store_create"
        } else {
            "cloud_store_update"
        },
        &view.name,
        &format!(
            "{} endpoint={} bucket={} root={}",
            view.service_label, view.endpoint, view.bucket, view.root
        ),
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": if creating { "云存储已创建" } else { "云存储已更新" },
        "data": { "store": view },
    })))
}

/// POST /system/cloud/store/delete —— 删除云存储配置（不动桶内数据）
pub async fn store_delete(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<StoreIdPayload>,
) -> ZapJsonResult {
    let view = cloud::delete_store(&claims.sub, &payload.id)?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "cloud_store_delete",
        &view.name,
        &format!("{} bucket={}", view.service_label, view.bucket),
    )
    .await;

    Ok(Json(
        json!({ "code": 0, "message": "云存储已删除", "data": {} }),
    ))
}

/// GET /system/cloud/test?id= —— 连通性测试（列 5 个对象）
pub async fn store_test(claims: Claims, Query(query): Query<StoreIdPayload>) -> ZapJsonResult {
    let store = cloud::get_store(&claims.sub, &query.id)?;
    let outcome = cloud::test_connection(&store).await?;
    Ok(Json(json!({
        "code": 0,
        // 带上列到的对象数：0 个也说明鉴权与网络是通的，能区分「连不上」与「桶是空的」
        "message": format!(
            "连接成功（{} ms，读取到 {} 个对象）",
            outcome.elapsed_ms, outcome.entries
        ),
        "data": { "entries": outcome.entries, "elapsed_ms": outcome.elapsed_ms },
    })))
}

// ── 对象操作 ────────────────────────────────────────────────

/// GET /system/cloud/list?id=&path=&hidden= —— 列目录（单层）
pub async fn file_list(claims: Claims, Query(query): Query<CloudQuery>) -> ZapJsonResult {
    let store = cloud::get_store(&claims.sub, &query.id)?;
    let data = cloud::list(&store, &query.path, query.hidden.unwrap_or(false)).await?;
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": data,
    })))
}

/// GET /system/cloud/download?id=&path= —— 下载对象（流式转发）
pub async fn file_download(
    claims: Claims,
    Query(query): Query<CloudQuery>,
) -> Result<Response, ZapError> {
    let store = cloud::get_store(&claims.sub, &query.id)?;
    let download = cloud::download(&store, &query.path).await?;

    // opendal 的错误统一转成 io::Error，交给 axum 边读边转发，
    // 大文件不在内存里攒整份（与 Local 端下载的差别就在这里）。
    let stream = download
        .stream
        .map_err(|e| std::io::Error::other(e.to_string()));

    let mut response = Response::new(Body::from_stream(stream));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&download.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition(&download.name))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    Ok(response)
}

/// POST /system/cloud/upload?id=&path=$(目录) —— multipart 上传（可多文件）
pub async fn file_upload(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Query(query): Query<CloudQuery>,
    mut multipart: Multipart,
) -> ZapJsonResult {
    let store = cloud::get_store(&claims.sub, &query.id)?;
    let dir = cloud::normalize_path(&query.path)?;

    let mut uploaded: Vec<String> = Vec::new();
    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            // 多数是「请求体超限」或「连接中断」，直接报给前端比静默结束好排查
            Err(e) => return Err(ZapError::New(-1, format!("上传中断：{e}"))),
        };

        let raw_name = field.file_name().unwrap_or("unnamed").to_string();
        let name = safe_file_name(&raw_name)?;
        let target = if dir.is_empty() {
            name.clone()
        } else {
            format!("{dir}/{name}")
        };

        let mut session = cloud::UploadSession::create(&store, &target).await?;
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if let Err(e) = session.write(chunk).await {
                        // 写失败就丢弃半截对象，别在桶里留个残缺文件
                        session.abort().await;
                        return Err(e);
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    session.abort().await;
                    return Err(ZapError::New(-1, format!("上传「{name}」失败：{e}")));
                }
            }
        }
        session.finish().await?;
        uploaded.push(target);
    }

    if uploaded.is_empty() {
        return Err(ZapError::New(-1, "没有收到上传文件".to_string()));
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "cloud_upload",
        &store.name,
        &uploaded.join(", "),
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": format!("已上传 {} 个文件", uploaded.len()),
        "data": { "files": uploaded },
    })))
}

/// GET /system/cloud/local/list?path= —— 浏览服务器上「当前用户可访问」的目录
///
/// 供上传弹窗里的「从服务器选择」用：与文件管理共用同一套隔离规则
/// （非 admin 仅自己 home 与私有 tmp），不传 path 时进入家目录。
pub async fn local_list(claims: Claims, Query(query): Query<LocalListQuery>) -> ZapJsonResult {
    // 直接复用文件管理的列目录实现：同一套家目录收敛与隔离规则
    let data = system_file::list_local_dir(&claims, query.path.as_deref().unwrap_or("")).await?;
    Ok(Json(json!({ "code": 0, "message": "ok", "data": data })))
}

/// POST /system/cloud/upload-local —— 把服务器上的文件直接传到云存储
///
/// 与 multipart 上传的差别：文件本来就在服务器上，不必经浏览器来回中转；
/// 后端「读盘 → 写对象」全程流式（[`cloud::upload_from_path`]），内存与文件大小无关。
/// 多个文件逐个处理：单个失败（越权 / 读不到）不影响其余文件，失败项随响应返回。
pub async fn file_upload_local(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<LocalUploadPayload>,
) -> ZapJsonResult {
    let store = cloud::get_store(&claims.sub, &payload.id)?;
    let dir = cloud::normalize_path(&payload.path)?;
    if payload.files.is_empty() {
        return Err(ZapError::New(-1, "没有选择要上传的文件".to_string()));
    }

    let (home, tmp) = system_file::user_private_prefixes(&claims).await;
    let mut uploaded: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for raw in &payload.files {
        let resolved = match system_file::resolve_path(raw) {
            Ok(p) => p,
            Err(e) => {
                failed.push(format!("{raw}：{e}"));
                continue;
            }
        };
        if let Err(e) = system_file::check_access(&claims, &resolved, &home, &tmp) {
            failed.push(format!("{raw}：{e}"));
            continue;
        }
        let name = match resolved.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => {
                failed.push(format!("{raw}：路径不合法"));
                continue;
            }
        };
        if resolved.is_dir() {
            failed.push(format!("{raw}：暂不支持上传目录"));
            continue;
        }

        let target = if dir.is_empty() {
            name.clone()
        } else {
            format!("{dir}/{name}")
        };
        match cloud::upload_from_path(&store, &target, &resolved).await {
            Ok(_) => uploaded.push(target),
            Err(e) => failed.push(format!("{name}：{e}")),
        }
    }

    if uploaded.is_empty() {
        return Err(ZapError::New(
            -1,
            format!("上传失败：{}", failed.join("；")),
        ));
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "cloud_upload_local",
        &store.name,
        &uploaded.join(", "),
    )
    .await;

    let message = if failed.is_empty() {
        format!("已上传 {} 个文件", uploaded.len())
    } else {
        format!("已上传 {} 个文件，{} 个失败", uploaded.len(), failed.len())
    };
    Ok(Json(json!({
        "code": 0,
        "message": message,
        "data": { "files": uploaded, "failed": failed },
    })))
}

/// POST /system/cloud/mkdir —— 新建目录
pub async fn file_mkdir(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CloudPathPayload>,
) -> ZapJsonResult {
    let store = cloud::get_store(&claims.sub, &payload.id)?;
    cloud::create_dir(&store, &payload.path).await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "cloud_mkdir",
        &store.name,
        &payload.path,
    )
    .await;

    Ok(Json(
        json!({ "code": 0, "message": "目录已创建", "data": {} }),
    ))
}

/// POST /system/cloud/delete —— 删除文件或目录（目录递归）
pub async fn file_delete(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CloudPathPayload>,
) -> ZapJsonResult {
    let store = cloud::get_store(&claims.sub, &payload.id)?;
    let is_dir = cloud::delete(&store, &payload.path).await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        if is_dir {
            "cloud_rmdir"
        } else {
            "cloud_delete"
        },
        &store.name,
        &payload.path,
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": if is_dir { "目录已删除" } else { "文件已删除" },
        "data": {},
    })))
}

/// POST /system/cloud/rename —— 重命名 / 移动（目录递归复制后删除）
pub async fn file_rename(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CloudRenamePayload>,
) -> ZapJsonResult {
    let store = cloud::get_store(&claims.sub, &payload.id)?;
    let is_dir = cloud::rename(&store, &payload.path, &payload.new_path).await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "cloud_rename",
        &store.name,
        &format!("{} -> {}", payload.path, payload.new_path),
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": if is_dir { "目录已重命名" } else { "文件已重命名" },
        "data": {},
    })))
}

// ── helpers ────────────────────────────────────────────────

/// 只取文件名部分（浏览器的 `filename` 可能带路径，来自某些老客户端）。
fn safe_file_name(raw: &str) -> Result<String, ZapError> {
    let name = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if name.is_empty() || name == "." || name == ".." || name.contains('\0') {
        return Err(ZapError::New(-1, "文件名不合法".to_string()));
    }
    Ok(name.to_string())
}

/// Content-Disposition 里的文件名：ASCII 兜底 + RFC 5987 的 UTF-8 形式
/// （中文名在 `filename="..."` 里会乱码，浏览器优先取 `filename*`）。
fn content_disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && c != '"' && c != '\\' && !c.is_control() {
                c
            } else {
                '_'
            }
        })
        .collect();

    let mut encoded = String::new();
    for b in name.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*b as char)
            }
            _ => encoded.push_str(&format!("%{b:02X}")),
        }
    }

    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_only_last_segment() {
        assert_eq!(safe_file_name("a/b/c.txt").unwrap(), "c.txt");
        assert_eq!(safe_file_name("c.txt").unwrap(), "c.txt");
        assert!(safe_file_name("..").is_err());
        assert!(safe_file_name("  ").is_err());
    }

    #[test]
    fn disposition_encodes_non_ascii() {
        let value = content_disposition("报表 2026.csv");
        assert!(value.contains("filename*=UTF-8''"));
        assert!(value.contains("%E6%8A%A5%E8%A1%A8"));
        // 兜底名一定是 ASCII（HTTP 头不允许裸中文）
        assert!(value.is_ascii());
    }
}
