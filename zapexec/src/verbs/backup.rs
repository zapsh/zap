// SPDX-License-Identifier: AGPL-3.0-only
//! 备份动词（root 特权）：目录打包 / 数据库导出 / 列表 / 删除 / 还原 / 磁盘查询。
//!
//! 归档统一落在「备份根目录」之内（默认 `{ZAP_PATH}/data/backup`，可由面板设置改到
//! 其它磁盘/挂载点），删除/解包也只允许落在备份根内，避免把任意路径写穿。归档属主为
//! root、权限 `0600`（与 `zapctl backup` 一致），面板侧（zapd 以 zapadm 运行）只能经本
//! 动词读写，不能直接碰备份目录。
//!
//! 执行真正的写盘前会先查目标文件系统可用空间，不足（含安全余量）时直接报错，避免写到
//! 一半磁盘满导致归档损坏。

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use zap_proto::Response;

/// 安全余量：预计所需空间之外，再预留这么多才允许开写。
const DISK_MARGIN: u64 = 256 * 1024 * 1024; // 256 MiB

/// 备份根目录：优先用面板下发的 `backup_root`，空则兜底默认 `{ZAP_PATH}/data/backup`。
fn effective_root(backup_root: &str) -> PathBuf {
    if !backup_root.trim().is_empty() {
        PathBuf::from(backup_root.trim())
    } else {
        let base = std::env::var("ZAP_PATH").unwrap_or_else(|_| "/usr/local/zap".to_string());
        PathBuf::from(base).join("data").join("backup")
    }
}

/// 解析输出目录：空 → 备份根；相对 → 备份根下的子目录；绝对 → 直接使用（可指向其它磁盘）。
fn resolve_dest_dir(dest_dir: &str, backup_root: &str) -> Result<PathBuf, String> {
    let root = effective_root(backup_root);
    std::fs::create_dir_all(&root).map_err(|e| format!("创建备份根目录失败: {e}"))?;
    let root = canonical(&root)?;

    let dest = if dest_dir.trim().is_empty() {
        root.clone()
    } else {
        let p = PathBuf::from(dest_dir.trim());
        let p = if p.is_absolute() { p } else { root.join(&p) };
        std::fs::create_dir_all(&p).map_err(|e| format!("创建输出目录失败: {e}"))?;
        canonical(&p)?
    };

    // 归档属主 root、权限 0700（备份根本身）
    let _ = std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700));
    Ok(dest)
}

/// 校验待删除/还原的归档路径位于备份根内。
fn assert_in_root(path: &str, backup_root: &str) -> Result<PathBuf, String> {
    let root = canonical(&effective_root(backup_root))?;
    let p = canonical(Path::new(path))?;
    if !p.starts_with(&root) {
        return Err("归档路径必须位于备份根目录内".to_string());
    }
    Ok(p)
}

fn canonical(p: &Path) -> Result<PathBuf, String> {
    p.canonicalize()
        .map_err(|e| format!("路径解析失败 {}: {e}", p.display()))
}

/// 查询某路径所在文件系统的（可用, 总）字节数；失败返回 None。
fn free_total(path: &Path) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes().to_vec()).ok()?;
    let mut vfs: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut vfs) } != 0 {
        return None;
    }
    let total = vfs.f_blocks * vfs.f_frsize;
    let free = vfs.f_bavail * vfs.f_frsize;
    Some((free, total))
}

/// 递归统计目录（含子目录）下所有普通文件大小，符号链接不计、避免环。
fn dir_size(paths: &[String]) -> u64 {
    let mut total = 0u64;
    for p in paths {
        total += dir_size_one(Path::new(p.trim()));
    }
    total
}

fn dir_size_one(p: &Path) -> u64 {
    let meta = match std::fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(_) => return 0,
    };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let mut stack = vec![p.to_path_buf()];
    let mut sum = 0u64;
    let mut guard = 0usize;
    while let Some(dir) = stack.pop() {
        guard += 1;
        if guard > 200_000 {
            break; // 安全上限，避免异常目录拖死
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let mp = match std::fs::symlink_metadata(e.path()) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if mp.is_file() {
                sum += mp.len();
            } else if mp.is_dir() {
                stack.push(e.path());
            }
        }
    }
    sum
}

/// 人类可读字节数。
fn human(b: u64) -> String {
    const U: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", v, U[i])
}

/// 写盘前检查目标文件系统可用空间是否足够（needed + 安全余量）。不足直接报错。
fn ensure_disk_space(dest: &Path, needed: u64) -> Result<(), String> {
    if let Some((free, _total)) = free_total(dest) {
        let required = needed + DISK_MARGIN;
        if free < required {
            return Err(format!(
                "磁盘空间不足：目标可用 {}，预计需要 {}（含 {} 安全余量）",
                human(free),
                human(required),
                human(DISK_MARGIN),
            ));
        }
    }
    Ok(())
}

/// 以 root 身份跑一条 shell 命令（脚本走安全 PATH，环境清空）。
fn root_shell(script: &str) -> Command {
    let mut c = crate::verbs::root_cmd(crate::verbs::platform::SHELL);
    c.arg("-c").arg(script);
    c
}

// ── 磁盘查询 ──────────────────────────────────────────────────────

/// 查询备份目录所在文件系统的可用 / 总空间。
pub async fn disk(dir: String) -> Response {
    let p = if dir.trim().is_empty() {
        effective_root("")
    } else {
        PathBuf::from(dir.trim().to_string())
    };
    let _ = std::fs::create_dir_all(&p);
    let (free, total) = match free_total(&p) {
        Some(v) => v,
        None => return Response::err(-1, "无法获取磁盘信息".to_string()),
    };
    Response::ok(
        "ok",
        Some(serde_json::json!({
            "path": p.to_string_lossy(),
            "disk_free": free,
            "disk_total": total,
        })),
    )
}

// ── 列表 ────────────────────────────────────────────────────────

pub async fn list(dir: String, backup_root: String) -> Response {
    let dest = match resolve_dest_dir(&dir, &backup_root) {
        Ok(d) => d,
        Err(e) => return Response::err(-1, e),
    };
    let mut items = Vec::new();
    let entries = match std::fs::read_dir(&dest) {
        Ok(e) => e,
        Err(e) => return Response::err(-1, format!("读取备份目录失败: {e}")),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        items.push(serde_json::json!({
            "name": name,
            "path": path.to_string_lossy(),
            "size": meta.len(),
            "mtime": meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            "ext": ext,
        }));
    }
    items.sort_by(|a, b| {
        let ma = a["mtime"].as_i64().unwrap_or(0);
        let mb = b["mtime"].as_i64().unwrap_or(0);
        mb.cmp(&ma)
    });
    let (disk_free, disk_total) = free_total(&dest).unwrap_or((0, 0));
    Response::ok(
        "ok",
        Some(serde_json::json!({
            "dir": dest.to_string_lossy(),
            "items": items,
            "disk_free": disk_free,
            "disk_total": disk_total,
        })),
    )
}

// ── 目录打包 ──────────────────────────────────────────────────────

pub async fn dir(
    name: String,
    paths: Vec<String>,
    dest_dir: String,
    as_user: Option<String>,
    _skip_owner_check: bool,
    backup_root: String,
    exclude: Vec<String>,
    exclude_file: Option<String>,
) -> Response {
    if name.is_empty() {
        return Response::err(-1, "归档名不能为空".to_string());
    }
    if paths.is_empty() {
        return Response::err(-1, "待打包路径不能为空".to_string());
    }
    let dest = match resolve_dest_dir(&dest_dir, &backup_root) {
        Ok(d) => d,
        Err(e) => return Response::err(-1, e),
    };
    // 合并排除模式：管理员通用清单（inline）+ 用户自定义文件（root 读取，含注释/空行）
    let mut excludes = exclude;
    if let Some(f) = exclude_file.as_deref().filter(|s| !s.is_empty())
        && let Ok(content) = std::fs::read_to_string(f) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                excludes.push(line.to_string());
            }
        }
    // 写盘前先确认磁盘空间够用
    let needed = dir_size(&paths);
    if let Err(e) = ensure_disk_space(&dest, needed) {
        return Response::err(-1, e);
    }
    let archive = dest.join(format!("{name}.tar.gz"));
    if archive.exists() {
        let _ = std::fs::remove_file(&archive);
    }

    let mut cmd = crate::verbs::root_cmd("tar");
    cmd.arg("-czf").arg(&archive).current_dir("/");
    for pat in &excludes {
        let p = pat.trim();
        if !p.is_empty() {
            cmd.arg("--exclude").arg(p);
        }
    }
    for p in &paths {
        let rel = p.trim_start_matches('/');
        if rel.is_empty() {
            return Response::err(-1, format!("非法路径: {p}"));
        }
        cmd.arg(rel);
    }
    match cmd.output() {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return Response::err(
                -1,
                format!("打包失败: {}", String::from_utf8_lossy(&o.stderr).trim()),
            );
        }
        Err(e) => return Response::err(-1, format!("执行 tar 失败: {e}")),
    }

    // 归档权限 0600（root 属主，避免别的账号读到备份内容）
    let _ = std::fs::set_permissions(&archive, std::fs::Permissions::from_mode(0o600));
    // 若指定了属主，把归档交还该账号（不改变 root 读权限下的安全性，仅便于该用户管理）
    if let Some(user) = as_user.as_deref()
        && !user.is_empty()
            && let Ok(acc) = crate::verbs::linux_account(user)
                && let Ok(c) = std::ffi::CString::new(archive.to_str().unwrap_or_default()) {
                    unsafe {
                        libc::chown(c.as_ptr(), acc.uid, acc.gid);
                    }
                }

    let size = std::fs::metadata(&archive).map(|m| m.len()).unwrap_or(0);
    Response::ok(
        "打包完成",
        Some(serde_json::json!({ "path": archive.to_string_lossy(), "size": size })),
    )
}

// ── 数据库导出 ────────────────────────────────────────────────────

pub async fn db(
    name: String,
    engine: String,
    db_name: String,
    user: String,
    password: String,
    host: String,
    port: i32,
    socket: Option<String>,
    dest_dir: String,
    db_path: Option<String>,
    backup_root: String,
) -> Response {
    if name.is_empty() {
        return Response::err(-1, "归档名不能为空".to_string());
    }
    let dest = match resolve_dest_dir(&dest_dir, &backup_root) {
        Ok(d) => d,
        Err(e) => return Response::err(-1, e),
    };

    let engine = engine.to_lowercase();
    // 预估所需空间：sqlite 用源库文件大小；mysql 无法精确预估，保守按 1GiB 预留
    let needed = if engine == "sqlite" {
        let p = db_path.as_deref().filter(|s| !s.is_empty()).unwrap_or("");
        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    } else {
        1u64 << 30
    };
    if let Err(e) = ensure_disk_space(&dest, needed) {
        return Response::err(-1, e);
    }

    let archive = dest.join(format!("{name}.sql.gz"));
    if archive.exists() {
        let _ = std::fs::remove_file(&archive);
    }

    let result = if engine == "sqlite" {
        let db_path = match db_path.as_deref() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return Response::err(-1, "sqlite 备份需要 db_path".to_string()),
        };
        // gzip -dc 的反向：sqlite3 $1 .dump | gzip > $2
        let script = "sqlite3 \"$1\" .dump | gzip > \"$2\"";
        let mut c = root_shell(script);
        c.arg("x")
            .arg(&db_path)
            .arg(archive.to_str().unwrap_or_default());
        run_status(c, "sqlite 导出失败")
    } else {
        // mysql / mariadb
        let mut c = crate::verbs::root_cmd("mysqldump");
        c.arg("--single-transaction")
            .arg("--routines")
            .arg("--events");
        // 优先走本机 socket（zapadm@localhost 直连最稳）；否则回退 TCP 回环
        if let Some(sock) = socket.as_deref().filter(|s| !s.is_empty()) {
            c.arg("--socket").arg(sock);
        } else {
            let host: &str = if host.is_empty() { "127.0.0.1" } else { &host };
            let port = if port <= 0 { 3306 } else { port };
            c.arg("-h").arg(host).arg("-P").arg(port.to_string());
        }
        c.arg("-u")
            .arg(&user)
            .arg(&db_name)
            .env("MYSQL_PWD", password.as_str());
        // mysqldump 输出经 gzip 落盘
        let gz_file = match std::fs::File::create(&archive) {
            Ok(f) => f,
            Err(e) => return Response::err(-1, format!("创建归档失败: {e}")),
        };
        let mut gzip = match std::process::Command::new("gzip")
            .arg("-c")
            .stdin(Stdio::piped())
            .stdout(Stdio::from(gz_file))
            .spawn()
        {
            Ok(g) => g,
            Err(e) => return Response::err(-1, format!("启动 gzip 失败: {e}")),
        };
        c.stdout(Stdio::piped());
        let mut dump = match c.spawn() {
            Ok(d) => d,
            Err(e) => return Response::err(-1, format!("启动 mysqldump 失败: {e}")),
        };
        if let (Some(mut out), Some(mut gin)) = (dump.stdout.take(), gzip.stdin.take()) {
            let _ = std::io::copy(&mut out, &mut gin);
        }
        let d_ok = dump.wait().map(|s| s.success()).unwrap_or(false);
        let g_ok = gzip.wait().map(|s| s.success()).unwrap_or(false);
        if d_ok && g_ok {
            Ok(())
        } else {
            Err("mysqldump/gzip 执行失败".to_string())
        }
    };

    if let Err(e) = result {
        let _ = std::fs::remove_file(&archive);
        return Response::err(-1, e);
    }
    let _ = std::fs::set_permissions(&archive, std::fs::Permissions::from_mode(0o600));
    let size = std::fs::metadata(&archive).map(|m| m.len()).unwrap_or(0);
    Response::ok(
        "数据库导出完成",
        Some(serde_json::json!({ "path": archive.to_string_lossy(), "size": size })),
    )
}

fn run_status(mut c: Command, err_msg: &str) -> Result<(), String> {
    match c.status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{err_msg}: 退出码 {}", s.code().unwrap_or(-1))),
        Err(e) => Err(format!("{err_msg}: {e}")),
    }
}

// ── 删除 ─────────────────────────────────────────────────────────

pub async fn delete(path: String, backup_root: String) -> Response {
    let p = match assert_in_root(&path, &backup_root) {
        Ok(p) => p,
        Err(e) => return Response::err(-1, e),
    };
    if !p.is_file() {
        return Response::err(-1, "归档不存在或不是文件".to_string());
    }
    if let Err(e) = std::fs::remove_file(&p) {
        return Response::err(-1, format!("删除失败: {e}"));
    }
    Response::ok("已删除", None)
}

// ── 目录还原 ──────────────────────────────────────────────────────

pub async fn restore_dir(
    path: String,
    target_dir: String,
    to_original: bool,
    _as_user: Option<String>,
    _skip_owner_check: bool,
    backup_root: String,
) -> Response {
    let p = match assert_in_root(&path, &backup_root) {
        Ok(p) => p,
        Err(e) => return Response::err(-1, e),
    };
    if !p.is_file() {
        return Response::err(-1, "归档不存在或不是文件".to_string());
    }
    if to_original {
        // 归档内为相对路径（如 home/u/www/...），以 root 运行 tar 并 -C / 解包，
        // 即可写回原绝对位置，且保留原始属主。
        let mut cmd = crate::verbs::root_cmd("tar");
        cmd.arg("-xzf").arg(&p).arg("-C").arg("/");
        return match cmd.status() {
            Ok(s) if s.success() => Response::ok("已还原到原路径", None),
            Ok(s) => Response::err(-1, format!("解包失败: 退出码 {}", s.code().unwrap_or(-1))),
            Err(e) => Response::err(-1, format!("执行 tar 失败: {e}")),
        };
    }
    let target = PathBuf::from(&target_dir);
    if let Err(e) = std::fs::create_dir_all(&target) {
        return Response::err(-1, format!("创建还原目标目录失败: {e}"));
    }
    let mut cmd = crate::verbs::root_cmd("tar");
    cmd.arg("-xzf").arg(&p).arg("-C").arg(&target);
    match cmd.status() {
        Ok(s) if s.success() => Response::ok("目录还原完成", None),
        Ok(s) => Response::err(-1, format!("解包失败: 退出码 {}", s.code().unwrap_or(-1))),
        Err(e) => Response::err(-1, format!("执行 tar 失败: {e}")),
    }
}

// ── 数据库还原 ────────────────────────────────────────────────────

pub async fn restore_db(
    path: String,
    engine: String,
    db_name: String,
    user: String,
    password: String,
    host: String,
    port: i32,
    db_path: Option<String>,
    backup_root: String,
) -> Response {
    let p = match assert_in_root(&path, &backup_root) {
        Ok(p) => p,
        Err(e) => return Response::err(-1, e),
    };
    if !p.is_file() {
        return Response::err(-1, "归档不存在或不是文件".to_string());
    }
    let engine = engine.to_lowercase();
    let result = if engine == "sqlite" {
        let db_path = match db_path.as_deref() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return Response::err(-1, "sqlite 还原需要 db_path".to_string()),
        };
        // gzip -dc $1 | sqlite3 $2
        let script = "gzip -dc \"$1\" | sqlite3 \"$2\"";
        let mut c = root_shell(script);
        c.arg("x").arg(&p).arg(&db_path);
        run_status(c, "sqlite 还原失败")
    } else {
        let host: &str = if host.is_empty() { "127.0.0.1" } else { &host };
        let port = if port <= 0 { 3306 } else { port };
        // gzip -dc $1 | mysql -h$2 -P$3 -u$4 $5
        let script = "gzip -dc \"$1\" | mysql -h\"$2\" -P\"$3\" -u\"$4\" \"$5\"";
        let mut c = root_shell(script);
        c.arg("x")
            .arg(&p)
            .arg(host)
            .arg(port.to_string())
            .arg(&user)
            .arg(&db_name)
            .env("MYSQL_PWD", password.as_str());
        run_status(c, "mysql 还原失败")
    };
    match result {
        Ok(()) => Response::ok("数据库还原完成", None),
        Err(e) => Response::err(-1, e),
    }
}
