use axum::{
    Json,
    body::Body,
    extract::{Extension, Multipart, Query},
    http::header,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use crate::db;
use crate::zap::{
    ZapError, ZapJsonResult, audit,
    jwt::{Claims, is_admin},
};
use zap_proto::Request;

// ── request types ──────────────────────────────────────────

#[derive(Deserialize)]
pub struct PathQuery {
    path: Option<String>,
}

#[derive(Deserialize)]
pub struct FileOpPayload {
    path: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    new_path: String,
}

#[derive(Deserialize)]
pub struct MkdirPayload {
    path: String,
}

#[derive(Deserialize)]
pub struct DeletePayload {
    path: String,
}

#[derive(Deserialize)]
pub struct ChmodPayload {
    path: String,
    /// 目标权限：八进制数值（如 0755 传 493），仅低 12 位有效
    mode: u32,
    /// 递归应用到目录下所有子项
    #[serde(default)]
    recursive: bool,
}

#[derive(Deserialize)]
pub struct ChownPayload {
    path: String,
    /// 新属主（Linux 用户名）；不传表示保持不变
    #[serde(default)]
    owner: Option<String>,
    /// 新属组（Linux 组名）；不传表示保持不变
    #[serde(default)]
    group: Option<String>,
    /// 递归应用到目录下所有子项
    #[serde(default)]
    recursive: bool,
}

#[derive(Deserialize)]
pub struct CopyPayload {
    path: String,
    new_path: String,
}

#[derive(Deserialize)]
pub struct ArchivePayload {
    paths: Vec<String>,
    name: String,
    base_dir: String,
    /// 目标目录：传了就把压缩包写到该目录，不传则返回内容供下载
    #[serde(default)]
    dest_dir: Option<String>,
}

// ── path helpers ───────────────────────────────────────────
// 授权（基于 JWT 角色）仍在 zapd 完成；实际文件操作转发给 zapexec（root）。

/// Resolve and sanitize a path, preventing directory traversal.
pub(crate) fn resolve_path(requested: &str) -> Result<PathBuf, ZapError> {
    let clean = requested
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != "..")
        .collect::<Vec<_>>()
        .join("/");

    let resolved = PathBuf::from("/").join(&clean);

    // Canonicalize if the path exists, otherwise just normalize
    match resolved.canonicalize() {
        Ok(canonical) => Ok(canonical),
        Err(_) => {
            let mut normalized = PathBuf::from("/");
            for seg in clean.split('/').filter(|s| !s.is_empty()) {
                normalized.push(seg);
            }
            if normalized.starts_with("/") {
                Ok(normalized)
            } else {
                Err(ZapError::New(-1, "非法路径".to_string()))
            }
        }
    }
}

/// 规范化上传文件名：允许 `dir/sub/a.txt` 这类相对路径（目录上传），
/// 过滤空段、`.` 与 `..`，避免越出目标目录；结果为空时回退 `unnamed`。
fn sanitize_relative(name: &str) -> String {
    let clean = name
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != "." && *seg != "..")
        .collect::<Vec<_>>()
        .join("/");
    if clean.is_empty() {
        "unnamed".to_string()
    } else {
        clean
    }
}

/// 上传临时文件目录（面板数据盘）：大文件边收边落盘，收完由 zapexec 搬走。
fn upload_tmp_dir() -> Result<PathBuf, ZapError> {
    let dir = crate::zap::appstore::data_dir().join("tmp").join("upload");
    std::fs::create_dir_all(&dir)
        .map_err(|e| ZapError::New(-1, format!("创建上传临时目录失败: {}", e)))?;
    Ok(dir)
}

/// 非管理员可访问的私有目录前缀（自己的 home 与私有临时目录）。
/// home 跟随 `user.home_dir`（迁移挂载点后文件管理自动切到新路径），
/// 无记录时回退 `/home/{username}`。
pub(crate) async fn user_private_prefixes(claims: &Claims) -> (String, String) {
    let pool = db::get_db_pool().await;
    let home: Option<String> =
        sqlx::query_scalar("SELECT home_dir FROM user WHERE username = ? AND home_dir != ''")
            .bind(&claims.sub)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let home = home.unwrap_or_else(|| format!("/home/{}", claims.sub));
    (home, format!("/tmp/zap-{}", claims.sub))
}

/// 文件操作的执行身份 `(linux 账号, 是否跳过属主校验)`：
/// - 管理员：以自己的 Linux 账号执行，跳过属主校验（可管理 root 拥有的文件）；
/// - 普通用户：以自己的 Linux 账号执行，且只能删改本人文件。
///
/// 这里只决定「以谁的名义执行」与「允许操作哪些文件」，**不决定文件属主**。
/// 属主规则统一放在 zapexec 侧：新建的内容归操作者，修改已有文件时保持原属主。
/// 后者尤其关键 —— 编辑 /etc 下的系统配置若把属主改成操作者本人，
/// 依赖 root 属主读取配置的服务就会起不来。
pub(crate) async fn actor_identity(claims: &Claims) -> Result<(Option<String>, bool), ZapError> {
    let pool = db::get_db_pool().await;
    let lu: Option<String> = sqlx::query_scalar("SELECT linux_user FROM user WHERE id = ?")
        .bind(claims.id as i64)
        .fetch_optional(pool)
        .await?;
    let lu = lu.filter(|u| !u.trim().is_empty());
    if is_admin(claims) {
        return Ok((lu, true));
    }
    match lu {
        Some(u) => Ok((Some(u), false)),
        None => Err(ZapError::New(
            -1,
            "当前账号未绑定系统用户，无法执行文件操作".to_string(),
        )),
    }
}

/// `path` 是否位于 `prefix` 之内（`prefix` 自身也算在内）。
///
/// 用于 home / 私有 tmp 这类「目录前缀」判断。必须比对到 `/` 边界，
/// 否则 `/home/admin` 会把 `/home/admin-tools` 也一并算进去。
fn path_within(path: &Path, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return false;
    }
    let p = path.to_string_lossy();
    p == prefix || p.starts_with(&format!("{prefix}/"))
}

/// Check if user has read access to a path.
///
/// 权限模型：
/// - 管理员：任意路径；
/// - 普通用户：仅自己 home（`user.home_dir`，回退 `/home/{username}`）与
///   私有临时目录（`/tmp/zap-{username}`）。
///   彻底移除此前"所有人可读 /var/www、/var/log"的越权隐患。
///
/// home/tmp 前缀由调用方异步查询后传入。
pub(crate) fn check_access(
    claims: &Claims,
    path: &Path,
    home: &str,
    tmp: &str,
) -> Result<(), ZapError> {
    if is_admin(claims) {
        return Ok(());
    }
    if path_within(path, home) || path_within(path, tmp) {
        return Ok(());
    }
    Err(ZapError::New(-1, "没有访问该路径的权限".to_string()))
}

/// Check if user can write to a path（与读权限同一套隔离规则）。
fn check_write_access(claims: &Claims, path: &Path, home: &str, tmp: &str) -> Result<(), ZapError> {
    check_access(claims, path, home, tmp)
}

/// 特殊位（Set UID / Set GID / Sticky）仅 admin 可改动：
/// admin 恒放行；其余角色要求请求值与文件当前特殊位一致（即原样保留，不能新增也不能清除）。
fn special_bits_allowed(claims: &Claims, requested: u32, current: u32) -> bool {
    is_admin(claims) || (requested & 0o7000) == (current & 0o7000)
}

/// 属主/属组名称清洗：空白视为未指定（保持原值）。
fn normalize_name(raw: Option<String>) -> Option<String> {
    raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

// ── handlers ───────────────────────────────────────────────

/// 列目录时把请求路径解析成实际要列的目标：
/// - 不传 `path`（打开文件管理）：一律进入个人家目录，管理员也一样，
///   免得一进文件管理就是整个根文件系统；
/// - 普通用户传 `/`：仍落到自己的 home（home/tmp 白名单保护，行为不变）；
/// - 其余显式路径原样透传（管理员想回根目录传 `path=/` 即可）。
fn resolve_list_target(is_admin: bool, requested: &str, home: &str) -> String {
    if requested.is_empty() || (!is_admin && requested == "/") {
        home.to_string()
    } else {
        requested.to_string()
    }
}

/// 列本地目录的公共实现：路径收敛 + 越权校验 + home 兜底与自动创建。
///
/// 文件管理与云存储的「从服务器选择」共用这一套隔离规则（两处必须一致，
/// 否则会出现「文件管理进得去、云存储却选不到」的割裂），因此把逻辑收在这里。
/// 返回的 data 额外带 `home`，供前端做侧栏根节点 / 服务器浏览的起点。
pub(crate) async fn list_local_dir(
    claims: &Claims,
    requested: &str,
) -> Result<serde_json::Value, ZapError> {
    let (home, tmp) = user_private_prefixes(claims).await;
    let raw_path = resolve_list_target(is_admin(claims), requested, &home);
    let resolved = resolve_path(&raw_path)?;
    check_access(claims, &resolved, &home, &tmp)?;

    // 首次访问自己的 home 时自动创建（以该用户名义创建，属主即本人）
    if !is_admin(claims) && resolved.as_path() == Path::new(&home) && !resolved.exists() {
        let (as_user, skip_owner_check) = actor_identity(claims).await?;
        let _ = crate::zapexec::call(Request::FileMkdir {
            path: home.clone(),
            as_user,
            skip_owner_check,
        })
        .await;
    }

    // 管理员的家目录可能压根没建过：默认进 home 失败时退回根目录，
    // 否则文件管理一打开就报「目录不存在」。
    let resolved = if is_admin(claims) && requested.is_empty() && !resolved.exists() {
        PathBuf::from("/")
    } else {
        resolved
    };

    let resp = crate::zapexec::call(Request::FileList {
        path: resolved.to_string_lossy().to_string(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    let mut data = resp.data.unwrap_or_else(|| json!({}));
    if let Some(obj) = data.as_object_mut() {
        obj.insert("home".to_string(), json!(home));
    }
    Ok(data)
}

/// GET /system/files/list?path=/
///
/// 不传 `path` 时进入个人家目录；响应额外带上 `home`，前端用它做侧栏根节点
/// 与地址栏起点（cPanel 风格：文件管理始终从家目录开始）。
pub async fn file_list(claims: Claims, Query(query): Query<PathQuery>) -> ZapJsonResult {
    let data = list_local_dir(&claims, query.path.as_deref().unwrap_or("")).await?;
    Ok(Json(json!({ "code": 0, "message": "ok", "data": data })))
}

/// GET /system/files/read?path=...
pub async fn file_read(
    claims: Claims,
    Query(query): Query<PathQuery>,
) -> Result<Response, ZapError> {
    let raw_path = query.path.as_deref().unwrap_or("");
    if raw_path.is_empty() {
        return Err(ZapError::New(-1, "缺少路径参数".to_string()));
    }
    let (home, tmp) = user_private_prefixes(&claims).await;
    let resolved = resolve_path(raw_path)?;
    check_access(&claims, &resolved, &home, &tmp)?;

    let resp = crate::zapexec::call(Request::FileRead {
        path: resolved.to_string_lossy().to_string(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": "ok", "data": resp.data })).into_response())
}

/// POST /system/files/write
pub async fn file_write(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FileOpPayload>,
) -> ZapJsonResult {
    let (home, tmp) = user_private_prefixes(&claims).await;
    let resolved = resolve_path(&payload.path)?;
    check_write_access(&claims, &resolved, &home, &tmp)?;

    let (as_user, skip_owner_check) = actor_identity(&claims).await?;
    let resp = crate::zapexec::call(Request::FileWrite {
        path: resolved.to_string_lossy().to_string(),
        content: payload.content,
        as_user,
        skip_owner_check,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "file_write",
        &resolved.to_string_lossy(),
        "",
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": resp.message, "data": resp.data }),
    ))
}

/// POST /system/files/delete
pub async fn file_delete(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<DeletePayload>,
) -> ZapJsonResult {
    let (home, tmp) = user_private_prefixes(&claims).await;
    let resolved = resolve_path(&payload.path)?;
    check_write_access(&claims, &resolved, &home, &tmp)?;

    let (as_user, skip_owner_check) = actor_identity(&claims).await?;
    let resp = crate::zapexec::call(Request::FileDelete {
        path: resolved.to_string_lossy().to_string(),
        as_user,
        skip_owner_check,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "file_delete",
        &resolved.to_string_lossy(),
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

/// POST /system/files/mkdir
pub async fn file_mkdir(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<MkdirPayload>,
) -> ZapJsonResult {
    let (home, tmp) = user_private_prefixes(&claims).await;
    let resolved = resolve_path(&payload.path)?;
    check_write_access(&claims, &resolved, &home, &tmp)?;

    let (as_user, skip_owner_check) = actor_identity(&claims).await?;
    let resp = crate::zapexec::call(Request::FileMkdir {
        path: resolved.to_string_lossy().to_string(),
        as_user,
        skip_owner_check,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "file_mkdir",
        &resolved.to_string_lossy(),
        "",
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": resp.message, "data": resp.data }),
    ))
}

/// POST /system/files/rename
pub async fn file_rename(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<FileOpPayload>,
) -> ZapJsonResult {
    let (home, tmp) = user_private_prefixes(&claims).await;
    let old_path = resolve_path(&payload.path)?;
    check_write_access(&claims, &old_path, &home, &tmp)?;

    if !old_path.exists() {
        return Err(ZapError::New(-1, "源文件不存在".to_string()));
    }

    let new_path = resolve_path(&payload.new_path)?;
    check_write_access(&claims, &new_path, &home, &tmp)?;

    let (as_user, skip_owner_check) = actor_identity(&claims).await?;
    let resp = crate::zapexec::call(Request::FileRename {
        path: old_path.to_string_lossy().to_string(),
        new_path: new_path.to_string_lossy().to_string(),
        as_user,
        skip_owner_check,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "file_rename",
        &format!(
            "{} → {}",
            old_path.to_string_lossy(),
            new_path.to_string_lossy()
        ),
        "",
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": resp.message, "data": resp.data }),
    ))
}

/// POST /system/files/chmod
///
/// 修改文件/目录权限（cPanel 风格）：mode 为八进制数值（如 0755 → 493），
/// 仅低 12 位有效（含 setuid/setgid/sticky）。
/// 特殊位（setuid/setgid/sticky）为高危权限，仅 admin 可改动：
/// 非 admin 请求中的特殊位必须与文件当前值一致（即原样保留），否则拒绝。
pub async fn file_chmod(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ChmodPayload>,
) -> ZapJsonResult {
    if payload.mode & !0o7777 != 0 {
        return Err(ZapError::New(
            -1,
            "权限值非法：仅支持 0-7777（八进制）".to_string(),
        ));
    }
    let (home, tmp) = user_private_prefixes(&claims).await;
    let resolved = resolve_path(&payload.path)?;
    check_write_access(&claims, &resolved, &home, &tmp)?;

    if !resolved.exists() {
        return Err(ZapError::New(-1, "路径不存在".to_string()));
    }

    // 特殊位（Set UID / Set GID / Sticky）仅 admin 可设置：
    // 非 admin 只能改 rwx 位，且特殊位必须保持文件当前取值（前端同样不暴露这几个开关，
    // 这里兜底拦住手工构造的请求）。
    let mut mode = payload.mode;
    if !is_admin(&claims) {
        use std::os::unix::fs::PermissionsExt;
        let current_mode = std::fs::metadata(&resolved)
            .map_err(|e| ZapError::New(-1, format!("读取当前权限失败：{e}")))?
            .permissions()
            .mode();
        if !special_bits_allowed(&claims, payload.mode, current_mode) {
            return Err(ZapError::New(
                -1,
                "特殊权限位（Set UID / Set GID / Sticky）仅管理员可设置".to_string(),
            ));
        }
        // 递归时子项的特殊位各不相同、无法逐项校验：非 admin 一律只改 rwx，剥离特殊位
        if payload.recursive {
            mode &= 0o777;
        }
    }

    let (as_user, skip_owner_check) = actor_identity(&claims).await?;
    let resp = crate::zapexec::call(Request::FileChmod {
        path: resolved.to_string_lossy().to_string(),
        mode,
        recursive: payload.recursive,
        as_user,
        skip_owner_check,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "file_chmod",
        &format!("{} ({:04o})", resolved.to_string_lossy(), payload.mode),
        "",
    )
    .await;

    Ok(Json(
        json!({ "code": 0, "message": resp.message, "data": resp.data }),
    ))
}

/// POST /system/files/chown
///
/// 修改文件/目录的属主与属组：仅 admin 可用（普通用户改属主等于越权）。
/// `owner` / `group` 传 Linux 名称，不传表示保持不变；`recursive` 为 true 时递归应用。
pub async fn file_chown(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ChownPayload>,
) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可以修改文件属主".to_string()));
    }
    let owner = normalize_name(payload.owner);
    let group = normalize_name(payload.group);
    if owner.is_none() && group.is_none() {
        return Err(ZapError::New(-1, "请至少指定新的属主或属组".to_string()));
    }
    let resolved = resolve_path(&payload.path)?;
    if !resolved.exists() {
        return Err(ZapError::New(-1, "路径不存在".to_string()));
    }

    let (as_user, skip_owner_check) = actor_identity(&claims).await?;
    let resp = crate::zapexec::call(Request::FileChown {
        path: resolved.to_string_lossy().to_string(),
        owner: owner.clone(),
        group: group.clone(),
        recursive: payload.recursive,
        as_user,
        skip_owner_check,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "file_chown",
        &format!(
            "{} owner={:?} group={:?} recursive={}",
            resolved.to_string_lossy(),
            owner,
            group,
            payload.recursive
        ),
        "",
    )
    .await;

    Ok(Json(
        json!({ "code": 0, "message": resp.message, "data": resp.data }),
    ))
}

/// POST /system/files/copy
pub async fn file_copy(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CopyPayload>,
) -> ZapJsonResult {
    let (home, tmp) = user_private_prefixes(&claims).await;
    let src = resolve_path(&payload.path)?;
    check_access(&claims, &src, &home, &tmp)?;
    if !src.exists() {
        return Err(ZapError::New(-1, "源文件不存在".to_string()));
    }

    let dst = resolve_path(&payload.new_path)?;
    check_write_access(&claims, &dst, &home, &tmp)?;

    let (as_user, skip_owner_check) = actor_identity(&claims).await?;
    let resp = crate::zapexec::call(Request::FileCopy {
        path: src.to_string_lossy().to_string(),
        new_path: dst.to_string_lossy().to_string(),
        as_user,
        skip_owner_check,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "file_copy",
        &format!("{} → {}", src.to_string_lossy(), dst.to_string_lossy()),
        "",
    )
    .await;

    Ok(Json(
        json!({ "code": 0, "message": resp.message, "data": resp.data }),
    ))
}

/// POST /system/files/archive
///
/// 把选中的文件/目录打包成 zip。
/// 传 `dest_dir` 时压缩包写进该目录（打包到目录），否则返回 base64 字节供下载。
pub async fn file_archive(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ArchivePayload>,
) -> ZapJsonResult {
    let (home, tmp) = user_private_prefixes(&claims).await;
    let base = resolve_path(&payload.base_dir)?;
    check_access(&claims, &base, &home, &tmp)?;
    if payload.paths.is_empty() {
        return Err(ZapError::New(-1, "请选择要打包的文件".to_string()));
    }

    // 校验所有源路径的访问权限
    for p in &payload.paths {
        let resolved = resolve_path(p)?;
        check_access(&claims, &resolved, &home, &tmp)?;
    }

    // 打包到目录：目标目录要有写权限，压缩包归当前操作者所有
    let dest_dir = match &payload.dest_dir {
        Some(d) => {
            let resolved = resolve_path(d)?;
            check_write_access(&claims, &resolved, &home, &tmp)?;
            Some(resolved.to_string_lossy().to_string())
        }
        None => None,
    };
    let (as_user, skip_owner_check) = if dest_dir.is_some() {
        actor_identity(&claims).await?
    } else {
        (None, false)
    };

    let resp = crate::zapexec::call(Request::FileArchive {
        paths: payload.paths.clone(),
        name: payload.name.clone(),
        base_dir: base.to_string_lossy().to_string(),
        dest_dir: dest_dir.clone(),
        as_user,
        skip_owner_check,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "file_archive",
        &format!(
            "{}{} ({} 项)",
            base.to_string_lossy(),
            dest_dir.map(|d| format!(" → {d}")).unwrap_or_default(),
            payload.paths.len()
        ),
        "",
    )
    .await;

    Ok(Json(
        json!({ "code": 0, "message": resp.message, "data": resp.data }),
    ))
}

/// GET /system/files/download?path=...
pub async fn file_download(
    claims: Claims,
    Query(query): Query<PathQuery>,
) -> Result<Response, ZapError> {
    let raw_path = query.path.as_deref().unwrap_or("");
    if raw_path.is_empty() {
        return Err(ZapError::New(-1, "缺少路径参数".to_string()));
    }
    let (home, tmp) = user_private_prefixes(&claims).await;
    let resolved = resolve_path(raw_path)?;
    check_access(&claims, &resolved, &home, &tmp)?;

    let resp = crate::zapexec::call(Request::FileDownload {
        path: resolved.to_string_lossy().to_string(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    let data = resp.data.unwrap_or_else(|| json!({}));
    let file_name = data
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("download")
        .to_string();
    let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let bytes = zap_proto::b64_decode(content)
        .map_err(|e| ZapError::Error(format!("内容解码失败: {e}")))?;

    let mime = mime_guess::from_path(&resolved).first_or_octet_stream();

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", file_name),
        )
        .body(Body::from(bytes))
        .unwrap())
}

/// POST /system/files/upload
pub async fn file_upload(
    claims: Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Query(query): Query<PathQuery>,
    mut multipart: Multipart,
) -> Result<Response, ZapError> {
    let target_dir = query.path.as_deref().unwrap_or("/tmp");
    let (home, tmp) = user_private_prefixes(&claims).await;
    let resolved_dir = resolve_path(target_dir)?;
    check_write_access(&claims, &resolved_dir, &home, &tmp)?;

    let (as_user, skip_owner_check) = actor_identity(&claims).await?;
    let mut uploaded: Vec<String> = Vec::new();

    // 边收边写盘：早先是 `field.bytes()` 把整个文件读进内存再 base64 放大 1.33 倍，
    // 大文件会把 zapd 撑爆。落盘目录放在面板数据盘（不是 /tmp，避免大文件撑满内存盘）。
    let run_dir = upload_tmp_dir()?.join(format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        rand::random::<u32>()
    ));
    tokio::fs::create_dir_all(&run_dir)
        .await
        .map_err(|e| ZapError::New(-1, format!("创建上传临时目录失败: {}", e)))?;

    let mut seq = 0u32;
    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            // 请求体超限（未放开 DefaultBodyLimit 时 axum 默认只收 2MB）、连接中断
            // 都会走到这里。以前是 `while let Ok(Some(..))` 直接吞掉，表现为
            // 「点了上传没反应 / 报没有上传文件」，真实原因被藏起来了。
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&run_dir).await;
                return Err(ZapError::New(-1, format!("上传中断：{}", e)));
            }
        };
        let file_name = field.file_name().unwrap_or("unnamed").to_string();
        // 目录上传时浏览器把相对路径放进 filename（`dir/sub/a.txt`），
        // 清洗后交给 zapexec 逐级建目录；越界段由 sanitize_relative 过滤。
        let rel_name = sanitize_relative(&file_name);

        seq += 1;
        let tmp_path = run_dir.join(format!("{seq}.part"));
        let mut out = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|e| ZapError::New(-1, format!("创建临时文件失败: {}", e)))?;
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if let Err(e) = out.write_all(&chunk).await {
                        let _ = tokio::fs::remove_dir_all(&run_dir).await;
                        return Err(ZapError::New(-1, format!("写入临时文件失败: {}", e)));
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = tokio::fs::remove_dir_all(&run_dir).await;
                    return Err(ZapError::New(-1, format!("接收文件「{}」失败：{}", rel_name, e)));
                }
            }
        }
        if let Err(e) = out.flush().await {
            let _ = tokio::fs::remove_dir_all(&run_dir).await;
            return Err(ZapError::New(-1, format!("写入临时文件失败: {}", e)));
        }
        drop(out);

        let resp = crate::zapexec::call(Request::FileUpload {
            path: resolved_dir.to_string_lossy().to_string(),
            name: rel_name.clone(),
            tmp: tmp_path.to_string_lossy().to_string(),
            as_user: as_user.clone(),
            skip_owner_check,
        })
        .await;
        // 成功时 exec 已把临时文件搬走/删除；失败时这里兜底清理，不留半截文件
        let _ = tokio::fs::remove_file(&tmp_path).await;
        let resp = resp?;
        if resp.code != 0 {
            let _ = tokio::fs::remove_dir_all(&run_dir).await;
            return Err(ZapError::New(resp.code, resp.message));
        }
        uploaded.push(rel_name);
    }
    let _ = tokio::fs::remove_dir(&run_dir).await;

    if uploaded.is_empty() {
        return Err(ZapError::New(-1, "没有上传文件".to_string()));
    }

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "file_upload",
        &resolved_dir.to_string_lossy(),
        &uploaded.join(", "),
    )
    .await;

    Ok(Json(json!({
        "code": 0,
        "message": "上传成功",
        "data": { "files": uploaded, "target_dir": resolved_dir.to_string_lossy() }
    }))
    .into_response())
}

/// GET /system/files/info?path=...
pub async fn file_info(claims: Claims, Query(query): Query<PathQuery>) -> ZapJsonResult {
    let raw_path = query.path.as_deref().unwrap_or("/");
    let (home, tmp) = user_private_prefixes(&claims).await;
    let resolved = resolve_path(raw_path)?;
    check_access(&claims, &resolved, &home, &tmp)?;

    let resp = crate::zapexec::call(Request::FileInfo {
        path: resolved.to_string_lossy().to_string(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": resp.data }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zap::jwt::Claims;

    fn claims_for(username: &str) -> Claims {
        Claims {
            id: 1,
            iat: 0,
            sub: username.to_string(),
            iss: "Zap".to_string(),
            exp: 0,
            roles: String::new(),
            token_version: 0,
            scope: String::new(),
        }
    }

    fn admin_claims() -> Claims {
        let mut c = claims_for("admin");
        c.roles = "admin".to_string();
        c
    }

    /// 测试辅助：以 claims.sub 推导 home/tmp 前缀并校验读权限
    fn access(c: &Claims, p: &str) -> Result<(), ZapError> {
        let home = format!("/home/{}", c.sub);
        let tmp = format!("/tmp/zap-{}", c.sub);
        check_access(c, Path::new(p), &home, &tmp)
    }

    /// 测试辅助：以 claims.sub 推导 home/tmp 前缀并校验写权限
    fn waccess(c: &Claims, p: &str) -> Result<(), ZapError> {
        let home = format!("/home/{}", c.sub);
        let tmp = format!("/tmp/zap-{}", c.sub);
        check_write_access(c, Path::new(p), &home, &tmp)
    }

    #[test]
    fn home_prefix_matches_only_full_segments() {
        // 目录自身与其下任意层级的子项都算命中
        assert!(path_within(Path::new("/home/admin"), "/home/admin"));
        assert!(path_within(Path::new("/home/admin/a.txt"), "/home/admin"));
        assert!(path_within(
            Path::new("/home/admin/sub/deep"),
            "/home/admin"
        ));

        // 字符串前缀相同但不是同一个目录：不能算命中，
        // 否则 /home/admin-tools 会被当成 admin 的 home（越权）
        assert!(!path_within(Path::new("/home/admin-tools"), "/home/admin"));
        assert!(!path_within(Path::new("/home/adminx/f"), "/home/admin"));

        // 完全无关的路径
        assert!(!path_within(Path::new("/etc/nginx"), "/home/admin"));
        assert!(!path_within(Path::new("/home/alice"), "/home/admin"));

        // 结尾斜杠宽容处理；空前缀一律不命中（否则会命中所有路径）
        assert!(path_within(Path::new("/home/admin/x"), "/home/admin/"));
        assert!(!path_within(Path::new("/anything"), ""));
    }

    #[test]
    fn admin_can_access_any_path() {
        let c = admin_claims();
        assert!(access(&c, "/etc/shadow").is_ok());
        assert!(access(&c, "/var/www").is_ok());
    }

    #[test]
    fn only_admin_can_change_special_bits() {
        let admin = admin_claims();
        let user = claims_for("alice");
        // admin：新增 / 清除特殊位均放行
        assert!(special_bits_allowed(&admin, 0o4755, 0o0755));
        assert!(special_bits_allowed(&admin, 0o0755, 0o4755));
        // 普通用户：特殊位与文件当前值一致（原样保留）放行，rwx 位可改
        assert!(special_bits_allowed(&user, 0o4755, 0o4755));
        assert!(special_bits_allowed(&user, 0o4777, 0o4755));
        assert!(special_bits_allowed(&user, 0o0755, 0o0755));
        // 普通用户：新增 / 清除 Set UID / Set GID / Sticky 均拒绝
        assert!(!special_bits_allowed(&user, 0o4755, 0o0755));
        assert!(!special_bits_allowed(&user, 0o0755, 0o4755));
        assert!(!special_bits_allowed(&user, 0o2775, 0o0775));
        assert!(!special_bits_allowed(&user, 0o1777, 0o0777));
    }

    #[test]
    fn list_defaults_to_home() {
        // 打开文件管理（不传 path）：管理员与普通用户都进自己的家目录
        assert_eq!(resolve_list_target(true, "", "/home/admin"), "/home/admin");
        assert_eq!(resolve_list_target(false, "", "/home/alice"), "/home/alice");
        // 普通用户点「根」仍落到家目录；管理员可以显式进根目录
        assert_eq!(
            resolve_list_target(false, "/", "/home/alice"),
            "/home/alice"
        );
        assert_eq!(resolve_list_target(true, "/", "/home/admin"), "/");
        // 显式路径原样透传
        assert_eq!(resolve_list_target(true, "/etc", "/home/admin"), "/etc");
        assert_eq!(
            resolve_list_target(false, "/home/alice/www", "/home/alice"),
            "/home/alice/www"
        );
    }

    #[test]
    fn user_can_access_own_home() {
        let c = claims_for("alice");
        assert!(access(&c, "/home/alice").is_ok());
        assert!(access(&c, "/home/alice/www/index.html").is_ok());
    }

    #[test]
    fn home_prefix_follows_given_root() {
        // 迁移到 /home2 后：目标前缀放行，旧前缀拒绝
        let c = claims_for("alice");
        assert!(
            check_access(
                &c,
                Path::new("/home2/alice/www"),
                "/home2/alice",
                "/tmp/zap-alice"
            )
            .is_ok()
        );
        assert!(
            check_access(
                &c,
                Path::new("/home/alice/www"),
                "/home2/alice",
                "/tmp/zap-alice"
            )
            .is_err()
        );
    }

    #[test]
    fn user_cannot_access_others_home() {
        let c = claims_for("alice");
        assert!(access(&c, "/home/bob").is_err());
        assert!(access(&c, "/home/bob/secret").is_err());
        // 前缀混淆攻击：/home/aliceevil 不属于 alice
        assert!(access(&c, "/home/aliceevil").is_err());
    }

    #[test]
    fn user_cannot_access_system_paths() {
        let c = claims_for("alice");
        // 此前普通用户可读 /var/www、/var/log，现已禁止
        for p in ["/var/www", "/var/log", "/etc/passwd", "/root", "/tmp"] {
            assert!(access(&c, p).is_err(), "{p} 应被拒绝");
        }
    }

    #[test]
    fn user_can_access_private_tmp() {
        let c = claims_for("alice");
        assert!(access(&c, "/tmp/zap-alice").is_ok());
        assert!(access(&c, "/tmp/zap-alice/t.txt").is_ok());
        assert!(access(&c, "/tmp/zap-bob/t.txt").is_err());
    }

    #[test]
    fn write_access_same_as_read() {
        let c = claims_for("alice");
        assert!(waccess(&c, "/home/alice/x").is_ok());
        assert!(waccess(&c, "/var/www").is_err());
    }
}
