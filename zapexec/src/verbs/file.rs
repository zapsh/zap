//! 文件管理器 verb。
//!
//! zapd 已完成授权（admin 全量 / 普通用户路径白名单），这里只做路径 sanitize
//! （防 `..` 穿越）、关键路径保护（禁删 `/`、`/etc`、`/root`、`/boot`）以及
//! 以 root 权限执行实际文件操作。二进制内容（download/upload）用 base64 传输。

use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use serde_json::json;
use zip::{ZipWriter, write::FileOptions};

use zap_proto::{Response, b64_encode};

#[derive(serde::Serialize)]
struct FileInfo {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    modified: String,
    /// 权限文本：八进制 4 位（如 `0755`，含 setuid/setgid/sticky 时为 `4755`）
    permissions: String,
    /// 权限原始数值（仅低 12 位），供前端「修改权限」对话框回填
    mode: u32,
    owner: String,
    group: String,
}

/// 解析 /etc/passwd 或 /etc/group 为 id → 名称映射。
/// 以 `#` 开头的行（uid/gid 溢出 65535 时的 NSS 保留行）忽略。
fn id_name_map(file: &str, name_idx: usize, id_idx: usize) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    if let Ok(content) = std::fs::read_to_string(file) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split(':').collect();
            let (Some(name), Some(id_str)) = (parts.get(name_idx), parts.get(id_idx)) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            if let Ok(id) = id_str.parse::<u32>() {
                map.entry(id).or_insert_with(|| name.to_string());
            }
        }
    }
    map
}

/// id → 名称缓存，带「文件指纹」（mtime + 长度）校验。
/// 常驻进程里 `/etc/passwd`、`/etc/group` 会随新建用户/组而变化，
/// 指纹变了就重新加载，避免新用户一直显示为数字 uid/gid。
struct IdNameCache {
    file: &'static str,
    name_idx: usize,
    id_idx: usize,
    fingerprint: Option<(u64, u32, u64)>,
    map: HashMap<u32, String>,
}

impl IdNameCache {
    fn new(file: &'static str, name_idx: usize, id_idx: usize) -> Self {
        Self {
            file,
            name_idx,
            id_idx,
            fingerprint: None,
            map: HashMap::new(),
        }
    }

    /// 文件指纹：mtime（秒 + 纳秒）+ 文件长度
    fn current_fingerprint(&self) -> Option<(u64, u32, u64)> {
        let metadata = std::fs::metadata(self.file).ok()?;
        let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        Some((modified.as_secs(), modified.subsec_nanos(), metadata.len()))
    }

    /// 查 id → 名称；文件有变化时先重载映射。
    fn get(&mut self, id: u32) -> Option<String> {
        let fingerprint = self.current_fingerprint();
        if self.fingerprint != fingerprint {
            self.map = id_name_map(self.file, self.name_idx, self.id_idx);
            self.fingerprint = fingerprint;
        }
        self.map.get(&id).cloned()
    }
}

fn passwd_cache() -> &'static Mutex<IdNameCache> {
    static CACHE: OnceLock<Mutex<IdNameCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(IdNameCache::new("/etc/passwd", 0, 2)))
}

fn group_cache() -> &'static Mutex<IdNameCache> {
    static CACHE: OnceLock<Mutex<IdNameCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(IdNameCache::new("/etc/group", 0, 2)))
}

/// 查 id → 名称：查不到（或缓存中毒）时回退数字 id
fn resolve_id(cache: &'static Mutex<IdNameCache>, id: u32) -> String {
    cache
        .lock()
        .ok()
        .and_then(|mut cache| cache.get(id))
        .unwrap_or_else(|| id.to_string())
}

/// uid → 用户名（查不到时回退数字 uid）
fn owner_name(uid: u32) -> String {
    resolve_id(passwd_cache(), uid)
}

/// gid → 组名（查不到时回退数字 gid）
fn group_name(gid: u32) -> String {
    resolve_id(group_cache(), gid)
}

/// 渲染权限为八进制 4 位文本：`0755` / `0644`，含 setuid/setgid/sticky（`4755` / `1777`）。
/// 与面板「权限」列展示、「修改权限」对话框回填保持一致（cPanel 风格）。
fn mode_text(mode: u32) -> String {
    format!("{:04o}", mode & 0o7777)
}

/// 操作者身份：面板用户以自己的 Linux 账号名义操作文件。
///
/// zapd 已完成路径授权，这里补上「归属语义」：新建/写入的内容归该账号所有。
/// `enforce_owner` 为 true（普通用户）时还会拒绝删改非本人创建的文件；
/// 管理员归属自己但跳过该校验，因此仍可管理服务器上 root 拥有的文件。
#[derive(Clone, Copy)]
struct Actor {
    uid: u32,
    gid: u32,
    /// true：只能删改本人文件；false：管理员/root，可管理任意文件
    enforce_owner: bool,
}

/// `as_user`：Some(linux 账号) = 以该账号名义操作；None = root。
/// `skip_owner_check`：true = 管理员（内容归属自己，但不校验属主）。
fn resolve_actor(as_user: Option<String>, skip_owner_check: bool) -> Result<Option<Actor>, String> {
    match as_user {
        None => Ok(None),
        Some(name) => match user_lookup(&name) {
            Some(actor) => Ok(Some(Actor {
                enforce_owner: !skip_owner_check,
                ..actor
            })),
            // 管理员的绑定账号缺失时退回 root，不影响其管理能力
            None if skip_owner_check => Ok(None),
            None => Err(format!("系统账号不存在: {name}")),
        },
    }
}

/// 解析 /etc/passwd 取 (uid, gid)。
fn user_lookup(name: &str) -> Option<Actor> {
    let content = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in content.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 4 && parts[0] == name {
            return Some(Actor {
                uid: parts[2].parse().ok()?,
                gid: parts[3].parse().ok()?,
                enforce_owner: true,
            });
        }
    }
    None
}

/// Linux 用户名 → uid（解析 /etc/passwd；用于「修改属主」）。
fn user_id_by_name(name: &str) -> Option<u32> {
    let content = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in content.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 4 && parts[0] == name {
            return parts[2].parse().ok();
        }
    }
    None
}

/// Linux 组名 → gid（解析 /etc/group；用于「修改属组」）。
fn group_id_by_name(name: &str) -> Option<u32> {
    let content = std::fs::read_to_string("/etc/group").ok()?;
    for line in content.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[0] == name {
            return parts[2].parse().ok();
        }
    }
    None
}

/// 普通用户只能删改自己名下的文件；管理员与 root 不受限。
fn ensure_owner(actor: Option<Actor>, path: &Path) -> Result<(), String> {
    let Some(actor) = actor else { return Ok(()) };
    if !actor.enforce_owner {
        return Ok(());
    }
    let md = std::fs::metadata(path).map_err(|e| format!("路径不存在: {e}"))?;
    if md.uid() == actor.uid {
        Ok(())
    } else {
        Err(format!(
            "没有权限：{} 不属于当前用户（属主 uid {}）",
            path.display(),
            md.uid()
        ))
    }
}

/// 把**新建**的内容归到操作者名下（避免留下 root 拥有的文件）。
///
/// 只用于「新建」场景；修改已有文件时不调用，以保持其原有属主 ——
/// 编辑 /etc 下的系统配置若把属主改成操作者本人，服务可能就读不到了。
fn apply_owner(actor: Option<Actor>, path: &Path) -> Result<(), String> {
    let Some(actor) = actor else { return Ok(()) };
    std::os::unix::fs::chown(path, Some(actor.uid), Some(actor.gid))
        .map_err(|e| format!("设置属主失败 {}: {e}", path.display()))
}

/// 逐级创建目录（含缺失的中间层），新建出来的每一层都归操作者所有。
fn create_dirs_owned(actor: Option<Actor>, path: &Path) -> Result<(), String> {
    let mut missing: Vec<PathBuf> = Vec::new();
    let mut cur = Some(path);
    while let Some(p) = cur {
        if p.exists() {
            break;
        }
        missing.push(p.to_path_buf());
        cur = p.parent();
    }
    for dir in missing.iter().rev() {
        std::fs::create_dir(dir).map_err(|e| format!("创建目录失败 {}: {e}", dir.display()))?;
        apply_owner(actor, dir)?;
    }
    Ok(())
}

/// 规范化上传用的相对路径（目录上传会带上 `dir/sub/a.txt`）：
/// 逐段过滤空段、`.` 与 `..`，保证结果始终落在目标目录之内。
fn sanitize_relative(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for seg in name
        .split('/')
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
    {
        out.push(seg);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn resolve_path(requested: &str) -> PathBuf {
    let clean = requested
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != "..")
        .collect::<Vec<_>>()
        .join("/");
    let resolved = PathBuf::from("/").join(&clean);
    match resolved.canonicalize() {
        Ok(c) => c,
        Err(_) => resolved,
    }
}

fn file_info(path: &Path) -> Option<FileInfo> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| {
            chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let mode = metadata.permissions().mode() & 0o7777;
    Some(FileInfo {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string()),
        path: path.to_string_lossy().to_string(),
        is_dir: metadata.is_dir(),
        size: metadata.len(),
        modified,
        permissions: mode_text(mode),
        mode,
        owner: owner_name(metadata.uid()),
        group: group_name(metadata.gid()),
    })
}

fn is_critical_path(path: &Path) -> bool {
    matches!(
        path.to_string_lossy().as_ref(),
        "/" | "/etc" | "/root" | "/boot"
    )
}

// ── 动词实现 ───────────────────────────────────────────────

pub async fn list(path: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let resolved = resolve_path(&path);
        let md = match std::fs::metadata(&resolved) {
            Ok(m) => m,
            Err(e) => return Response::err(-1, format!("路径不存在: {e}")),
        };
        if !md.is_dir() {
            return Response::err(-1, "路径不是目录");
        }
        let mut entries: Vec<FileInfo> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&resolved) {
            for entry in rd.flatten() {
                if let Some(info) = file_info(&entry.path()) {
                    entries.push(info);
                }
            }
        }
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Response::ok(
            "ok",
            Some(json!({
                "current_path": resolved.to_string_lossy(),
                "parent_path": resolved
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "/".to_string()),
                "entries": entries,
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn read(path: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let resolved = resolve_path(&path);
        let md = match std::fs::metadata(&resolved) {
            Ok(m) => m,
            Err(e) => return Response::err(-1, format!("路径不存在: {e}")),
        };
        if md.is_dir() {
            return Response::err(-1, "不能读取目录");
        }
        match std::fs::read_to_string(&resolved) {
            Ok(content) => Response::ok(
                "ok",
                Some(json!({
                    "path": resolved.to_string_lossy(),
                    "content": content,
                    "size": content.len(),
                })),
            ),
            Err(e) => Response::err(-1, format!("读取文件失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn write(
    path: String,
    content: String,
    as_user: Option<String>,
    skip_owner_check: bool,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let actor = match resolve_actor(as_user, skip_owner_check) {
            Ok(a) => a,
            Err(e) => return Response::err(-1, e),
        };
        let resolved = resolve_path(&path);
        if let Ok(md) = std::fs::metadata(&resolved)
            && md.is_dir()
        {
            return Response::err(-1, "不能覆盖目录");
        }
        // 覆盖已有文件仅限本人文件（root 建的文件普通用户改不了）
        let existed = resolved.exists();
        if existed && let Err(e) = ensure_owner(actor, &resolved) {
            return Response::err(-1, e);
        }
        if let Some(parent) = resolved.parent()
            && let Err(e) = create_dirs_owned(actor, parent)
        {
            return Response::err(-1, e);
        }
        match std::fs::write(&resolved, &content) {
            Ok(_) => {
                // 仅新建的内容归操作者；修改已有文件时保持其原有属主，
                // 否则编辑 /etc 下的系统配置会把属主改成操作者本人，
                // 依赖 root 属主读取配置的服务就会起不来。
                let owner = if existed {
                    Ok(())
                } else {
                    apply_owner(actor, &resolved)
                };
                match owner {
                    Ok(_) => Response::ok(
                        "保存成功",
                        Some(json!({ "path": resolved.to_string_lossy() })),
                    ),
                    Err(e) => Response::err(-1, e),
                }
            }
            Err(e) => Response::err(-1, format!("写入失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn delete(path: String, as_user: Option<String>, skip_owner_check: bool) -> Response {
    tokio::task::spawn_blocking(move || {
        let actor = match resolve_actor(as_user, skip_owner_check) {
            Ok(a) => a,
            Err(e) => return Response::err(-1, e),
        };
        let resolved = resolve_path(&path);
        if is_critical_path(&resolved) {
            return Response::err(-1, "不能删除系统关键目录");
        }
        let md = match std::fs::metadata(&resolved) {
            Ok(m) => m,
            Err(e) => return Response::err(-1, format!("路径不存在: {e}")),
        };
        // 普通用户不能删除系统（root）创建的文件
        if let Err(e) = ensure_owner(actor, &resolved) {
            return Response::err(-1, e);
        }
        let result = if md.is_dir() {
            std::fs::remove_dir_all(&resolved)
        } else {
            std::fs::remove_file(&resolved)
        };
        match result {
            Ok(_) => Response::ok("删除成功", None),
            Err(e) => Response::err(-1, format!("删除失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn mkdir(path: String, as_user: Option<String>, skip_owner_check: bool) -> Response {
    tokio::task::spawn_blocking(move || {
        let actor = match resolve_actor(as_user, skip_owner_check) {
            Ok(a) => a,
            Err(e) => return Response::err(-1, e),
        };
        let resolved = resolve_path(&path);
        if resolved.exists() {
            return Response::err(-1, "目录已存在");
        }
        // 新建目录（含中间层）归操作者所有，不再默认 root
        match create_dirs_owned(actor, &resolved) {
            Ok(_) => Response::ok(
                "创建成功",
                Some(json!({ "path": resolved.to_string_lossy() })),
            ),
            Err(e) => Response::err(-1, format!("创建失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn rename(
    path: String,
    new_path: String,
    as_user: Option<String>,
    skip_owner_check: bool,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let actor = match resolve_actor(as_user, skip_owner_check) {
            Ok(a) => a,
            Err(e) => return Response::err(-1, e),
        };
        let old_path = resolve_path(&path);
        let new_path = resolve_path(&new_path);
        if !old_path.exists() {
            return Response::err(-1, "源文件不存在");
        }
        // 只能重命名本人文件
        if let Err(e) = ensure_owner(actor, &old_path) {
            return Response::err(-1, e);
        }
        if new_path.exists() {
            return Response::err(-1, "目标已存在");
        }
        match std::fs::rename(&old_path, &new_path) {
            Ok(_) => Response::ok(
                "重命名成功",
                Some(json!({
                    "old_path": old_path.to_string_lossy(),
                    "new_path": new_path.to_string_lossy(),
                })),
            ),
            Err(e) => Response::err(-1, format!("重命名失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn download(path: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let resolved = resolve_path(&path);
        let md = match std::fs::metadata(&resolved) {
            Ok(m) => m,
            Err(e) => return Response::err(-1, format!("路径不存在: {e}")),
        };
        if md.is_dir() {
            return Response::err(-1, "不能下载目录");
        }
        let file_name = resolved
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "download".to_string());
        match std::fs::read(&resolved) {
            Ok(bytes) => Response::ok(
                "ok",
                Some(json!({
                    "name": file_name,
                    "content": b64_encode(&bytes),
                })),
            ),
            Err(e) => Response::err(-1, format!("读取文件失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn upload(
    path: String,
    name: String,
    tmp: String,
    as_user: Option<String>,
    skip_owner_check: bool,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let actor = match resolve_actor(as_user, skip_owner_check) {
            Ok(a) => a,
            Err(e) => return Response::err(-1, e),
        };
        let dir = resolve_path(&path);
        // 目录不存在时按操作者身份创建（中间层同样归属该账号）
        if !dir.exists()
            && let Err(e) = create_dirs_owned(actor, &dir)
        {
            return Response::err(-1, e);
        }
        let md = match std::fs::metadata(&dir) {
            Ok(m) => m,
            Err(e) => return Response::err(-1, format!("目标路径不存在: {e}")),
        };
        if !md.is_dir() {
            return Response::err(-1, "目标路径不是目录");
        }
        // 目录上传时 name 形如 `dir/sub/a.txt`，逐级补建目录还原结构
        let rel = match sanitize_relative(&name) {
            Some(r) => r,
            None => return Response::err(-1, "非法的文件名"),
        };
        // 临时文件由 zapd 流式写入（边收边落盘），这里只搬移 —— 大文件不进内存
        let tmp_path = PathBuf::from(&tmp);
        if !tmp_path.is_file() {
            return Response::err(-1, format!("上传临时文件不存在: {}", tmp_path.display()));
        }
        let dest = dir.join(&rel);
        if let Some(parent) = dest.parent()
            && !parent.exists()
            && let Err(e) = create_dirs_owned(actor, parent)
        {
            let _ = std::fs::remove_file(&tmp_path);
            return Response::err(-1, e);
        }
        // 覆盖同名文件仅限本人文件；新上传的文件归操作者所有
        let existed = dest.exists();
        if existed && let Err(e) = ensure_owner(actor, &dest) {
            let _ = std::fs::remove_file(&tmp_path);
            return Response::err(-1, e);
        }
        // 同设备用 rename（原子、无需额外空间）；跨设备（/data 与 /home 分盘）退回复制
        if std::fs::rename(&tmp_path, &dest).is_err() {
            if let Err(e) = std::fs::copy(&tmp_path, &dest) {
                let _ = std::fs::remove_file(&tmp_path);
                return Response::err(-1, format!("写入文件失败: {e}"));
            }
        }
        // rename 已消耗源文件；复制分支才需要删。两种都调一次，不存在时静默
        let _ = std::fs::remove_file(&tmp_path);
        // 与早先 `std::fs::write` 的落盘权限对齐，避免临时文件把 0600 带进目标
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o644));
        // 与 write 同一规则：只有新建的才归操作者，覆盖已有文件时保持原属主
        match if existed { Ok(()) } else { apply_owner(actor, &dest) } {
            Ok(_) => Response::ok("上传成功", Some(json!({ "name": rel.to_string_lossy() }))),
            Err(e) => Response::err(-1, e),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn info(path: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let resolved = resolve_path(&path);
        match file_info(&resolved) {
            Some(info) => Response::ok("ok", Some(json!(info))),
            None => Response::err(-1, "文件不存在"),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 成功响应：带上根路径的最新信息（前端用它回填权限 / 属主列）。
fn ok_with_info(path: &Path, msg: &str) -> Response {
    match file_info(path) {
        Some(info) => Response::ok(msg, Some(json!(info))),
        None => Response::ok(msg, None),
    }
}

/// 修改文件/目录权限（八进制，仅低 12 位，含 setuid/setgid/sticky）。
///
/// `recursive=true` 时递归应用到目录下所有子项；普通用户只能改自己名下文件，
/// 递归过程中遇到非本人文件会跳过并计数，避免整棵树因个别异主文件整体失败。
pub async fn chmod(
    path: String,
    mode: u32,
    recursive: bool,
    as_user: Option<String>,
    skip_owner_check: bool,
) -> Response {
    tokio::task::spawn_blocking(move || {
        if mode & !0o7777 != 0 {
            return Response::err(-1, "权限值非法：仅支持 0-7777（八进制）");
        }
        let actor = match resolve_actor(as_user, skip_owner_check) {
            Ok(a) => a,
            Err(e) => return Response::err(-1, e),
        };
        let resolved = resolve_path(&path);
        if is_critical_path(&resolved) {
            return Response::err(-1, "不能修改系统关键目录的权限");
        }
        if !resolved.exists() {
            return Response::err(-1, "路径不存在");
        }

        let set_mode =
            |p: &Path| std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode));

        if !recursive {
            // 只能修改本人文件的权限
            if let Err(e) = ensure_owner(actor, &resolved) {
                return Response::err(-1, e);
            }
            return match set_mode(&resolved) {
                Ok(_) => ok_with_info(&resolved, "权限修改成功"),
                Err(e) => Response::err(-1, format!("修改权限失败: {e}")),
            };
        }

        let mut changed = 0usize;
        let mut skipped = 0usize;
        let mut stack = vec![resolved.clone()];
        while let Some(p) = stack.pop() {
            if ensure_owner(actor, &p).is_err() {
                skipped += 1;
            } else if let Err(e) = set_mode(&p) {
                return Response::err(-1, format!("修改权限失败 {}: {e}", p.display()));
            } else {
                changed += 1;
            }
            if p.is_dir()
                && let Ok(rd) = std::fs::read_dir(&p)
            {
                for entry in rd.flatten() {
                    stack.push(entry.path());
                }
            }
        }
        let msg = if skipped > 0 {
            format!("权限修改成功（{changed} 项，跳过 {skipped} 项无权限）")
        } else {
            format!("权限修改成功（{changed} 项）")
        };
        ok_with_info(&resolved, &msg)
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 修改文件/目录属主与属组（仅 admin 调用；角色校验在 zapd 完成）。
///
/// `owner` / `group` 传 Linux 名称，None 表示保持不变；`recursive=true` 递归应用。
pub async fn chown(
    path: String,
    owner: Option<String>,
    group: Option<String>,
    recursive: bool,
    as_user: Option<String>,
    skip_owner_check: bool,
) -> Response {
    tokio::task::spawn_blocking(move || {
        if owner.is_none() && group.is_none() {
            return Response::err(-1, "请至少指定新的属主或属组");
        }
        let uid = if let Some(name) = owner.as_deref() {
            match user_id_by_name(name) {
                Some(uid) => Some(uid),
                None => return Response::err(-1, format!("系统用户不存在: {name}")),
            }
        } else {
            None
        };
        let gid = if let Some(name) = group.as_deref() {
            match group_id_by_name(name) {
                Some(gid) => Some(gid),
                None => return Response::err(-1, format!("系统用户组不存在: {name}")),
            }
        } else {
            None
        };
        let _actor = match resolve_actor(as_user, skip_owner_check) {
            Ok(a) => a,
            Err(e) => return Response::err(-1, e),
        };
        let resolved = resolve_path(&path);
        if is_critical_path(&resolved) {
            return Response::err(-1, "不能修改系统关键目录的属主");
        }
        if !resolved.exists() {
            return Response::err(-1, "路径不存在");
        }

        let do_chown = |p: &Path| std::os::unix::fs::chown(p, uid, gid).map(|_| ());

        if !recursive {
            return match do_chown(&resolved) {
                Ok(()) => ok_with_info(&resolved, "属主修改成功"),
                Err(e) => Response::err(-1, format!("修改属主失败: {e}")),
            };
        }

        let mut changed = 0usize;
        let mut stack = vec![resolved.clone()];
        while let Some(p) = stack.pop() {
            if let Err(e) = do_chown(&p) {
                return Response::err(-1, format!("修改属主失败 {}: {e}", p.display()));
            }
            changed += 1;
            if p.is_dir()
                && let Ok(rd) = std::fs::read_dir(&p)
            {
                for entry in rd.flatten() {
                    stack.push(entry.path());
                }
            }
        }
        ok_with_info(&resolved, &format!("属主修改成功（{changed} 项）"))
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 递归复制文件或目录。
///
/// 副本**原样继承源的权限与属主/属组**（等价 `cp -a`）：复制只是多出一份内容，
/// 不该改变它"是谁的"。唯一例外是过程中新建出来的中间目录 —— 源里没有对应物，
/// 归操作者所有。
fn copy_recursive(src: &Path, dst: &Path, actor: Option<Actor>) -> Result<(), String> {
    let md = std::fs::metadata(src).map_err(|e| format!("读取源失败 {}: {e}", src.display()))?;
    if md.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| format!("创建目录失败 {}: {e}", dst.display()))?;
        // 先定属主再定权限：让 set_permissions 收口，保证最终权限与源一致
        std::os::unix::fs::chown(dst, Some(md.uid()), Some(md.gid()))
            .map_err(|e| format!("设置属主失败 {}: {e}", dst.display()))?;
        std::fs::set_permissions(dst, md.permissions())
            .map_err(|e| format!("设置权限失败 {}: {e}", dst.display()))?;
        for entry in std::fs::read_dir(src)
            .map_err(|e| format!("读取目录失败 {}: {e}", src.display()))?
            .flatten()
        {
            let name = entry.file_name();
            copy_recursive(&entry.path(), &dst.join(name), actor)?;
        }
    } else {
        // 父目录不存在时逐级创建：中间层是新建的，归操作者所有
        if let Some(parent) = dst.parent() {
            create_dirs_owned(actor, parent)?;
        }
        std::fs::copy(src, dst).map_err(|e| format!("复制文件失败 {}: {e}", dst.display()))?;
        // std::fs::copy 已保留权限模式，这里再把属主/属组也原样带过来
        std::os::unix::fs::chown(dst, Some(md.uid()), Some(md.gid()))
            .map_err(|e| format!("设置属主失败 {}: {e}", dst.display()))?;
    }
    Ok(())
}

/// 复制文件/目录到目标路径。
pub async fn copy(
    path: String,
    new_path: String,
    as_user: Option<String>,
    skip_owner_check: bool,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let actor = match resolve_actor(as_user, skip_owner_check) {
            Ok(a) => a,
            Err(e) => return Response::err(-1, e),
        };
        let src = resolve_path(&path);
        if !src.exists() {
            return Response::err(-1, "源文件不存在");
        }
        let dst = resolve_path(&new_path);
        if is_critical_path(&dst) {
            return Response::err(-1, "不能覆盖系统关键目录");
        }
        if dst.exists() {
            return Response::err(-1, "目标已存在");
        }
        // 复制需要读取源 + 在目标父目录写入
        if let Err(e) = ensure_owner(actor, &src) {
            return Response::err(-1, e);
        }
        if let Some(parent) = dst.parent()
            && !parent.exists()
            && let Err(e) = create_dirs_owned(actor, parent)
        {
            return Response::err(-1, e);
        }
        match copy_recursive(&src, &dst, actor) {
            Ok(_) => match file_info(&dst) {
                Some(info) => Response::ok("复制成功", Some(json!(info))),
                None => Response::ok("复制成功", Some(json!({ "path": dst.to_string_lossy() }))),
            },
            Err(e) => Response::err(-1, e),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 递归把文件/目录加入 zip。
fn archive_add<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    base: &Path,
    path: &Path,
) -> Result<(), String> {
    let md = std::fs::metadata(path).map_err(|e| format!("读取失败 {}: {e}", path.display()))?;
    let name_in_zip = path
        .strip_prefix(base)
        .map_err(|_| format!("路径 {} 不在基目录 {} 下", path.display(), base.display()))?
        .to_string_lossy()
        .to_string();
    if md.is_dir() {
        // 目录名保证以 / 结尾
        let dir_name = if name_in_zip.ends_with('/') {
            name_in_zip
        } else {
            format!("{name_in_zip}/")
        };
        zip.add_directory::<_, ()>(dir_name, FileOptions::<()>::default())
            .map_err(|e| format!("添加目录失败: {e}"))?;
        for entry in std::fs::read_dir(path)
            .map_err(|e| format!("读取目录失败: {e}"))?
            .flatten()
        {
            archive_add(zip, base, &entry.path())?;
        }
    } else {
        zip.start_file(name_in_zip, FileOptions::<()>::default())
            .map_err(|e| format!("添加文件失败: {e}"))?;
        let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
        zip.write_all(&bytes)
            .map_err(|e| format!("写入 zip 失败: {e}"))?;
    }
    Ok(())
}

/// 把多个文件/目录打包成 zip。
///
/// 给了 `dest_dir` 就把压缩包写进该目录（打包到指定位置），
/// 否则返回 zip 的 base64 内容，由调用方下载。
pub async fn archive(
    paths: Vec<String>,
    name: String,
    base_dir: String,
    dest_dir: Option<String>,
    as_user: Option<String>,
    skip_owner_check: bool,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let actor = match resolve_actor(as_user, skip_owner_check) {
            Ok(a) => a,
            Err(e) => return Response::err(-1, e),
        };
        let base = resolve_path(&base_dir);
        if !base.exists() {
            return Response::err(-1, "基目录不存在");
        }
        let resolved_paths: Vec<PathBuf> = paths.into_iter().map(|p| resolve_path(&p)).collect();
        for p in &resolved_paths {
            if !p.exists() {
                return Response::err(-1, format!("路径不存在: {}", p.display()));
            }
            if !p.starts_with(&base) {
                return Response::err(
                    -1,
                    format!("路径 {} 不在基目录 {} 下", p.display(), base.display()),
                );
            }
        }

        // 压缩包名只取最后一段并过滤 `..`/空段，避免被写到目标目录之外
        let zip_name = match sanitize_relative(&name)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        {
            Some(n) if !n.is_empty() => n,
            _ => return Response::err(-1, "压缩包名称无效"),
        };
        let zip_name = if zip_name.to_lowercase().ends_with(".zip") {
            zip_name
        } else {
            format!("{zip_name}.zip")
        };

        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buf);
            for p in &resolved_paths {
                if let Err(e) = archive_add(&mut zip, &base, p) {
                    return Response::err(-1, e);
                }
            }
            if let Err(e) = zip.finish() {
                return Response::err(-1, format!("打包失败: {e}"));
            }
        }
        let bytes = buf.into_inner();

        let Some(dest_dir) = dest_dir else {
            return Response::ok(
                "ok",
                Some(json!({
                    "name": zip_name,
                    "content": b64_encode(&bytes),
                })),
            );
        };

        // 目标目录不存在时按操作者身份创建，压缩包同样归操作者所有
        let dir = resolve_path(&dest_dir);
        if !dir.exists()
            && let Err(e) = create_dirs_owned(actor, &dir)
        {
            return Response::err(-1, e);
        }
        let dest = dir.join(&zip_name);
        // 覆盖同名压缩包仅限本人文件
        if dest.exists()
            && let Err(e) = ensure_owner(actor, &dest)
        {
            return Response::err(-1, e);
        }
        match std::fs::write(&dest, &bytes) {
            Ok(_) => match apply_owner(actor, &dest) {
                Ok(_) => Response::ok(
                    "打包成功",
                    Some(json!({ "name": zip_name, "path": dest.to_string_lossy() })),
                ),
                Err(e) => Response::err(-1, e),
            },
            Err(e) => Response::err(-1, format!("写入压缩包失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 上传把 zapd 落好的临时文件搬到目标位置：内容一致、临时文件清掉，
    /// 且 `dir/sub/a.txt` 这种目录结构要逐级补建（浏览器上传目录时会带上）。
    #[tokio::test]
    async fn upload_moves_tmp_into_place() {
        let root = std::env::temp_dir().join("zap-upload-move-test");
        let _ = std::fs::remove_dir_all(&root);
        let target = root.join("target");
        std::fs::create_dir_all(&target).unwrap();

        let tmp = root.join("1.part");
        std::fs::write(&tmp, b"hello zap").unwrap();
        let resp = upload(
            target.display().to_string(),
            "a.txt".to_string(),
            tmp.display().to_string(),
            None,
            true,
        )
        .await;
        assert_eq!(resp.code, 0, "{}", resp.message);
        assert_eq!(std::fs::read(target.join("a.txt")).unwrap(), b"hello zap");
        assert!(!tmp.exists(), "搬完必须清掉临时文件");

        // 目录上传：name 带子目录，应在目标下还原结构
        let tmp2 = root.join("2.part");
        std::fs::write(&tmp2, b"nested").unwrap();
        let resp = upload(
            target.display().to_string(),
            "sub/dir/b.txt".to_string(),
            tmp2.display().to_string(),
            None,
            true,
        )
        .await;
        assert_eq!(resp.code, 0, "{}", resp.message);
        assert_eq!(
            std::fs::read(target.join("sub/dir/b.txt")).unwrap(),
            b"nested"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 临时文件不在（zapd 写盘失败 / 已被清理）时必须报错，不能写出空文件。
    #[tokio::test]
    async fn upload_rejects_missing_tmp() {
        let root = std::env::temp_dir().join("zap-upload-missing-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let resp = upload(
            root.display().to_string(),
            "a.txt".to_string(),
            root.join("nope.part").display().to_string(),
            None,
            true,
        )
        .await;
        assert_ne!(resp.code, 0);
        assert!(!root.join("a.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 缓存必须能感知文件变化：常驻进程里新建用户后应立刻显示用户名，
    /// 而不是一直回退成数字 uid。
    #[test]
    fn id_name_cache_reloads_after_file_change() {
        let file = "/tmp/zap_id_name_cache_test";
        std::fs::write(file, "alice:x:1000:1000::/home/alice:/bin/bash\n").unwrap();

        let mut cache = IdNameCache::new(file, 0, 2);
        assert_eq!(cache.get(1000).as_deref(), Some("alice"));
        assert_eq!(cache.get(1001), None);

        // 模拟新建用户（追加一行：长度与 mtime 都变化）
        std::fs::write(
            file,
            "alice:x:1000:1000::/home/alice:/bin/bash\nbob:x:1001:1001::/home/bob:/bin/bash\n",
        )
        .unwrap();

        assert_eq!(cache.get(1001).as_deref(), Some("bob"));
        let _ = std::fs::remove_file(file);
    }

    fn is_root() -> bool {
        unsafe { libc::geteuid() == 0 }
    }

    /// 构造"普通用户"操作者（uid 用数值模拟，不依赖真实账号）
    fn user_actor(uid: u32) -> Actor {
        Actor {
            uid,
            gid: uid,
            enforce_owner: true,
        }
    }

    /// 普通用户（以 Linux 账号名义）只能删改本人文件；root 不受限。
    #[test]
    fn owner_guard_restricts_to_own_files() {
        if !is_root() {
            return;
        }
        let file = "/tmp/zap_owner_guard_test";
        std::fs::write(file, b"x").unwrap();
        std::os::unix::fs::chown(file, Some(1005), Some(1005)).unwrap();

        let path = Path::new(file);
        assert!(ensure_owner(Some(user_actor(1005)), path).is_ok());
        assert!(ensure_owner(Some(user_actor(1006)), path).is_err());
        // 管理员（root）不受归属限制
        assert!(ensure_owner(None, path).is_ok());

        let _ = std::fs::remove_file(file);
    }

    /// 逐级创建目录时，新建的中间层也要归操作者所有（不留 root 目录）。
    #[test]
    fn created_dirs_are_owned_by_actor() {
        if !is_root() {
            return;
        }
        let base = "/tmp/zap_mkdir_owner_test";
        let _ = std::fs::remove_dir_all(base);
        let dir = format!("{base}/a/b");
        create_dirs_owned(Some(user_actor(1005)), Path::new(&dir)).unwrap();

        for p in [format!("{base}/a"), dir] {
            let md = std::fs::metadata(&p).unwrap();
            assert_eq!(md.uid(), 1005, "{p} 应归操作者所有");
        }
        let _ = std::fs::remove_dir_all(base);
    }

    /// `as_user` 解析：不存在的系统账号直接报错（管理员则退回 root），不静默降权。
    #[test]
    fn actor_resolution_from_passwd() {
        assert!(resolve_actor(None, false).unwrap().is_none());
        let root = user_lookup("root").expect("root 应存在于 /etc/passwd");
        assert_eq!(root.uid, 0);
        assert!(resolve_actor(Some("zap_no_such_user_xyz".into()), false).is_err());
        // 管理员：绑定账号缺失时退回 root，保持全量管理能力
        assert!(
            resolve_actor(Some("zap_no_such_user_xyz".into()), true)
                .unwrap()
                .is_none()
        );
    }

    /// 管理员模式：跳过属主校验（可管服务器上 root 的文件），但仍是"归属自己"的执行者。
    #[test]
    fn admin_actor_can_manage_root_files() {
        if !is_root() {
            return;
        }
        let file = "/tmp/zap_admin_actor_test";
        std::fs::write(file, b"x").unwrap(); // 属主 root(0)
        let path = Path::new(file);

        // 普通用户模式：别人的（root 的）文件删改应被拒
        assert!(ensure_owner(Some(user_actor(1005)), path).is_err());

        // 管理员模式（skip_owner_check=true）：归属自己 + 允许管理 root 文件
        let admin = resolve_actor(Some("root".into()), true).unwrap().unwrap();
        assert_eq!(admin.uid, 0);
        assert!(!admin.enforce_owner);
        assert!(ensure_owner(Some(admin), path).is_ok());
        assert!(apply_owner(Some(admin), path).is_ok());

        let _ = std::fs::remove_file(file);
    }
}
