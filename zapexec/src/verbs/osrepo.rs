// SPDX-License-Identifier: AGPL-3.0-only
//! 系统软件源管理（root 特权）。
//!
//! 与面板其它「配置写入」动词同样的三道防线：
//! 1. **路径白名单** —— 只允许 `/etc/apt/sources.list`、`/etc/apt/sources.list.d/`、
//!    `/etc/yum.repos.d/`、`/etc/zypp/repos.d/`、`/etc/dnf/` 下的文件，扩展名按
//!    发行版约定收敛（`.list` / `.sources` / `.repo`），挡掉路径穿越与误改系统文件；
//! 2. **先备份再落盘** —— 备份统一进 `{ZAP_PATH}/data/backups/osrepo/`，
//!    保留最近 20 份，`os_repo.restore` 可一键回到「最初的源」；
//! 3. **原子替换** —— 同目录临时文件 + `rename`，中途失败不会留下半截源文件。
//!
//! 包管理器只有 `apt-get update` / `dnf makecache` / `yum makecache` 三个白名单动作，
//! 参数由 exec 侧拼死，不接受调用方传命令串。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::verbs::root_cmd;
use zap_proto::Response;

/// 源文件大小上限：1MB 足够容纳任何正常 sources / repo 文件。
const MAX_FILE_BYTES: usize = 1 << 20;
/// 每个源文件保留的备份份数。
const BACKUP_KEEP: usize = 20;
/// 备份目录下列出条数上限。
const BACKUP_LIST_KEEP: usize = 50;

const APT_MAIN: &str = "/etc/apt/sources.list";
const APT_DIR: &str = "/etc/apt/sources.list.d";
const DNF_DIR: &str = "/etc/yum.repos.d";
const ZYPP_DIR: &str = "/etc/zypp/repos.d";
const DNF_CONF_DIR: &str = "/etc/dnf";
const YUM_CONF: &str = "/etc/yum.conf";

fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

fn backups_dir() -> PathBuf {
    zap_path().join("data/backups/osrepo")
}

fn command_exists(name: &str) -> bool {
    root_cmd("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 探测包管理器：apt-get / dnf / yum（找不到时按 apt 报错，错误信息里说明）。
fn detect_manager() -> String {
    for m in ["apt-get", "dnf", "yum"] {
        if command_exists(m) {
            return m.to_string();
        }
    }
    "apt-get".to_string()
}

/// 读取 /etc/os-release 的 (ID, VERSION_ID, PRETTY_NAME)。
fn os_release() -> (String, String, String) {
    let content = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut id = String::new();
    let mut version = String::new();
    let mut pretty = String::new();
    for line in content.lines() {
        let value = |line: &str, key: &str| -> Option<String> {
            line.strip_prefix(key)
                .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        };
        if let Some(v) = value(line, "ID=") {
            id = v;
        } else if let Some(v) = value(line, "VERSION_ID=") {
            version = v;
        } else if let Some(v) = value(line, "PRETTY_NAME=") {
            pretty = v;
        }
    }
    (id, version, pretty)
}

/// 源文件归属的目录白名单（主文件本身也算一项）。
fn repo_roots() -> Vec<PathBuf> {
    [APT_MAIN, APT_DIR, DNF_DIR, ZYPP_DIR, DNF_CONF_DIR, YUM_CONF]
        .iter()
        .map(PathBuf::from)
        .collect()
}

/// 按目录约定收敛扩展名；主 sources.list 与 /etc/dnf 下的 .conf 单独放行。
fn allowed_ext(canon: &Path) -> bool {
    let name = canon
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let in_dir = |dir: &str| {
        canon
            .parent()
            .and_then(|p| p.to_str())
            .map(|p| p == dir)
            .unwrap_or(false)
    };
    if in_dir(APT_DIR) {
        return name.ends_with(".list") || name.ends_with(".sources");
    }
    if in_dir(DNF_DIR) || in_dir(ZYPP_DIR) {
        return name.ends_with(".repo");
    }
    if in_dir(DNF_CONF_DIR) {
        return name.ends_with(".conf");
    }
    // /etc/apt/sources.list 与 /etc/yum.conf 主文件
    canon == Path::new(APT_MAIN) || canon == Path::new(YUM_CONF)
}

/// 路径白名单校验：绝对路径、规范化后必须落在源目录内、扩展名符合约定、
/// 且不接受面板自己的备份/临时文件。
fn validate_repo_path(raw: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(raw);
    if !p.is_absolute() {
        return Err("源文件路径必须为绝对路径".to_string());
    }
    let canon = match p.canonicalize() {
        Ok(c) => c,
        // 文件还不存在（新建源文件）时退回父目录判定
        Err(_) => {
            let parent = p.parent().ok_or("源文件路径不合法".to_string())?;
            let pc = parent
                .canonicalize()
                .map_err(|_| "源文件所在目录不存在".to_string())?;
            pc.join(p.file_name().ok_or("源文件路径不合法".to_string())?)
        }
    };
    if !repo_roots().iter().any(|root| {
        let rc = root.canonicalize().unwrap_or_else(|_| root.clone());
        canon == rc || canon.starts_with(&rc)
    }) {
        return Err(
            "仅允许管理系统软件源目录（/etc/apt/sources.list(.d)、/etc/yum.repos.d 等）下的文件"
                .to_string(),
        );
    }
    if !allowed_ext(&canon) {
        return Err("源文件扩展名不符合该目录的约定（.list/.sources/.repo/.conf）".to_string());
    }
    let name = canon
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name.ends_with(".bak") || name.ends_with(".tmp") || name.contains(".zap") {
        return Err("该文件为面板托管/备份文件，请勿直接编辑".to_string());
    }
    Ok(canon)
}

/// 备份（保留最近 `BACKUP_KEEP` 份；文件不存在时跳过）。
fn backup_file(path: &Path) -> Option<PathBuf> {
    if !path.is_file() {
        return None;
    }
    let dir = backups_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = dir.join(format!("{ts}-{name}.bak"));
    std::fs::copy(path, &dest).ok()?;

    let mut kept: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    kept.sort();
    while kept.len() > BACKUP_KEEP {
        if let Some(old) = kept.first() {
            let _ = std::fs::remove_file(old);
            kept.remove(0);
        }
    }
    Some(dest)
}

/// 同目录临时文件 + rename 原子替换，权限收敛到 0644。
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("源文件路径不合法".to_string())?;
    let tmp = parent.join(format!(
        ".{}.zap-tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("source")
    ));
    std::fs::write(&tmp, content).map_err(|e| format!("写入临时文件失败: {e}"))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换源文件失败: {e}"));
    }
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644));
    Ok(())
}

fn file_mtime(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// apt 源是否启用：存在未注释的 deb/deb-src 行。
fn apt_enabled(content: &str) -> bool {
    content.lines().any(|l| {
        let l = l.trim();
        (l.starts_with("deb ") || l.starts_with("deb-src ")) && !l.starts_with('#')
    })
}

/// rpm 源是否启用：`enabled=0` 视为停用。
fn rpm_enabled(content: &str) -> bool {
    !content
        .lines()
        .any(|l| l.trim().replace(' ', "").starts_with("enabled=0"))
}

/// 扫目录收集源文件条目。
fn collect(dir: &str, kind: &str, out: &mut Vec<serde_json::Value>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if name.ends_with(".bak") || name.contains(".zap") {
            continue;
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let enabled = if kind == "apt" {
            apt_enabled(&content)
        } else {
            rpm_enabled(&content)
        };
        out.push(json!({
            "path": path.to_string_lossy(),
            "name": name,
            "kind": kind,
            "size": file_size(&path),
            "mtime": file_mtime(&path),
            "enabled": enabled,
        }));
    }
}

/// 解析 deb822（`.sources`）的一个 stanza，按 Types × URIs × Suites 展开成条目。
fn push_deb822_stanza(
    out: &mut Vec<serde_json::Value>,
    file: &str,
    types: &str,
    uris: &str,
    suites: &str,
    comps: &str,
    enabled: bool,
) {
    if uris.is_empty() || suites.is_empty() {
        return;
    }
    for typ in types.split_whitespace() {
        if typ != "deb" && typ != "deb-src" {
            continue;
        }
        for uri in uris.split_whitespace() {
            for suite in suites.split_whitespace() {
                out.push(json!({
                    "file": file,
                    "type": typ,
                    "uri": uri,
                    "suite": suite,
                    "components": comps,
                    "enabled": enabled,
                }));
            }
        }
    }
}

/// 解析单个 apt 源文件里的条目（供前端展示「系统已存在的源」）。
///
/// 支持两种格式：单行式（`sources.list` / `*.list`，`deb [选项] URI suite components...`）
/// 与 deb822（`*.sources`，`Types/URIs/Suites/Components/Enabled` 块）。
/// 只保留 `deb` / `deb-src` 条目，注释行跳过；每条带来源文件名，便于前端定位编辑。
fn parse_apt_entries(file: &str, content: &str, out: &mut Vec<serde_json::Value>) {
    if file.ends_with(".sources") {
        // deb822：空行分隔 stanza
        let mut types = String::new();
        let mut uris = String::new();
        let mut suites = String::new();
        let mut comps = String::new();
        let mut enabled = true;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                push_deb822_stanza(out, file, &types, &uris, &suites, &comps, enabled);
                types.clear();
                uris.clear();
                suites.clear();
                comps.clear();
                enabled = true;
            } else if let Some(v) = line.strip_prefix("Types:") {
                types = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("URIs:") {
                uris = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Suites:") {
                suites = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Components:") {
                comps = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Enabled:") {
                enabled = !v.trim().eq_ignore_ascii_case("no");
            }
            // 其余字段（Signed-By / Target / 语言相关）与条目展示无关，忽略
        }
        push_deb822_stanza(out, file, &types, &uris, &suites, &comps, enabled);
        return;
    }
    // 单行式
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (typ, rest) = match line.split_once(char::is_whitespace) {
            Some((t, r)) => (t, r.trim()),
            None => continue,
        };
        if typ != "deb" && typ != "deb-src" {
            continue;
        }
        // 跳过 `[arch=amd64 trusted=yes]` 这类选项段
        let rest = match rest.strip_prefix('[') {
            Some(s) => match s.find(']') {
                Some(i) => s[i + 1..].trim(),
                None => rest,
            },
            None => rest,
        };
        let mut it = rest.split_whitespace();
        let (Some(uri), Some(suite)) = (it.next(), it.next()) else {
            continue;
        };
        let components: Vec<&str> = it.collect();
        out.push(json!({
            "file": file,
            "type": typ,
            "uri": uri,
            "suite": suite,
            "components": components.join(" "),
            "enabled": true,
        }));
    }
}

/// 汇总 apt 系所有源条目：主 `sources.list` + `/etc/apt/sources.list.d/` 下
/// 的 `.list` / `.sources`（跳过面板备份文件，与 `collect` 口径一致）。
fn apt_all_entries() -> Vec<serde_json::Value> {
    let mut entries = Vec::new();
    if let Ok(c) = std::fs::read_to_string(APT_MAIN) {
        parse_apt_entries("sources.list", &c, &mut entries);
    }
    let mut names: Vec<String> = match std::fs::read_dir(APT_DIR) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|n| !n.ends_with(".bak") && !n.contains(".zap"))
            .collect(),
        Err(_) => return entries,
    };
    names.sort();
    for name in names {
        if let Ok(c) = std::fs::read_to_string(Path::new(APT_DIR).join(&name)) {
            parse_apt_entries(&name, &c, &mut entries);
        }
    }
    entries
}

/// `os_repo.list`：系统识别 + 源文件清单。
pub async fn list() -> Response {
    tokio::task::spawn_blocking(move || {
        let (id, version, pretty) = os_release();
        let manager = detect_manager();
        let mut files: Vec<serde_json::Value> = Vec::new();
        // apt 系额外输出条目级清单（含主 sources.list 里的行），供前端展示
        // 「系统已存在的源」——很多系统的源都写在主文件里，目录下反而是空的。
        let entries = if manager == "apt-get" {
            apt_all_entries()
        } else {
            Vec::new()
        };
        if Path::new(APT_MAIN).is_file() {
            let content = std::fs::read_to_string(APT_MAIN).unwrap_or_default();
            files.push(json!({
                "path": APT_MAIN,
                "name": "sources.list",
                "kind": "main",
                "size": file_size(Path::new(APT_MAIN)),
                "mtime": file_mtime(Path::new(APT_MAIN)),
                "enabled": apt_enabled(&content),
            }));
        }
        collect(APT_DIR, "apt", &mut files);
        collect(DNF_DIR, "rpm", &mut files);
        collect(ZYPP_DIR, "rpm", &mut files);
        files.sort_by(|a, b| {
            a.get("path")
                .and_then(|v| v.as_str())
                .cmp(&b.get("path").and_then(|v| v.as_str()))
        });
        Response::ok(
            "ok",
            Some(json!({
                "distro": {
                    "id": id,
                    "version": version,
                    "prettyName": pretty,
                },
                "manager": manager,
                "files": files,
                "entries": entries,
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// `os_repo.read`：读源文件内容。
pub async fn read(path: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let canon = match validate_repo_path(&path) {
            Ok(p) => p,
            Err(e) => return Response::err(-1, e),
        };
        let meta = std::fs::metadata(&canon).map_err(|e| format!("读取源文件失败: {e}"));
        match meta {
            Ok(m) if m.len() as usize > MAX_FILE_BYTES => {
                return Response::err(-1, "源文件超过 1MB，疑似非软件源文件".to_string());
            }
            Err(e) => return Response::err(-1, e),
            Ok(_) => {}
        }
        let content = match std::fs::read_to_string(&canon) {
            Ok(c) => c,
            Err(e) => return Response::err(-1, format!("读取源文件失败: {e}")),
        };
        Response::ok(
            "ok",
            Some(json!({
                "path": canon.to_string_lossy(),
                "content": content,
                "size": file_size(&canon),
                "mtime": file_mtime(&canon),
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// `os_repo.save`：备份 + 原子写入；dry_run 只做校验。
pub async fn save(path: String, content: String, dry_run: bool) -> Response {
    tokio::task::spawn_blocking(move || {
        let canon = match validate_repo_path(&path) {
            Ok(p) => p,
            Err(e) => return Response::err(-1, e),
        };
        if content.len() > MAX_FILE_BYTES {
            return Response::err(-1, "内容超过 1MB，请精简后重试".to_string());
        }
        if dry_run {
            return Response::ok(
                "校验通过",
                Some(json!({ "dry_run": true, "path": canon.to_string_lossy() })),
            );
        }
        let backup = backup_file(&canon);
        if let Err(e) = atomic_write(&canon, &content) {
            return Response::err(-1, e);
        }
        Response::ok(
            "ok",
            Some(json!({
                "path": canon.to_string_lossy(),
                "backup": backup.map(|b| b.to_string_lossy().to_string()),
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// `os_repo.delete`：备份后删除第三方源文件（主 sources.list 不允许删）。
pub async fn delete(path: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let canon = match validate_repo_path(&path) {
            Ok(p) => p,
            Err(e) => return Response::err(-1, e),
        };
        if canon == Path::new(APT_MAIN) {
            return Response::err(-1, "发行版主源文件不允许删除，可改用换源或编辑".to_string());
        }
        if !canon.is_file() {
            return Response::err(-1, "源文件不存在".to_string());
        }
        let backup = backup_file(&canon);
        if let Err(e) = std::fs::remove_file(&canon) {
            return Response::err(-1, format!("删除失败: {e}"));
        }
        Response::ok(
            "ok",
            Some(json!({
                "path": canon.to_string_lossy(),
                "backup": backup.map(|b| b.to_string_lossy().to_string()),
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// `os_repo.backups`：备份列表（新的在前）。
pub async fn backups() -> Response {
    tokio::task::spawn_blocking(move || {
        let dir = backups_dir();
        let mut items: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
            .ok()
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().is_file())
                    .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
                    .collect()
            })
            .unwrap_or_default();
        items.sort_by(|a, b| b.0.cmp(&a.0));
        items.truncate(BACKUP_LIST_KEEP);
        let list: Vec<serde_json::Value> = items
            .iter()
            .map(|(name, path)| {
                json!({
                    "name": name,
                    "size": file_size(path),
                    "mtime": file_mtime(path),
                })
            })
            .collect();
        Response::ok(
            "ok",
            Some(json!({ "dir": dir.to_string_lossy(), "items": list })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// `os_repo.backup_read`：读一个备份文件的内容（还原前先看一眼）。
pub async fn backup_read(name: String) -> Response {
    tokio::task::spawn_blocking(move || {
        // 只接受纯文件名，防路径穿越读走别处
        if name.contains('/') || name.contains("..") || !name.ends_with(".bak") {
            return Response::err(-1, "备份文件名不合法".to_string());
        }
        let path = backups_dir().join(&name);
        if !path.is_file() {
            return Response::err(-1, "备份文件不存在".to_string());
        }
        match std::fs::metadata(&path) {
            Ok(m) if m.len() as usize > MAX_FILE_BYTES => {
                return Response::err(-1, "备份文件超过 1MB，疑似非软件源备份".to_string());
            }
            Err(e) => return Response::err(-1, format!("读取备份失败: {e}")),
            Ok(_) => {}
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => return Response::err(-1, format!("读取备份失败: {e}")),
        };
        Response::ok(
            "ok",
            Some(json!({
                "name": name,
                "content": content,
                "size": file_size(&path),
                "mtime": file_mtime(&path),
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// `os_repo.restore`：把备份写回源文件。
pub async fn restore(backup: String, path: String) -> Response {
    tokio::task::spawn_blocking(move || {
        // 备份名只接受纯文件名，防路径穿越
        if backup.contains('/') || backup.contains("..") || !backup.ends_with(".bak") {
            return Response::err(-1, "备份文件名不合法".to_string());
        }
        let src = backups_dir().join(&backup);
        if !src.is_file() {
            return Response::err(-1, "备份文件不存在".to_string());
        }
        let canon = match validate_repo_path(&path) {
            Ok(p) => p,
            Err(e) => return Response::err(-1, e),
        };
        let content = match std::fs::read_to_string(&src) {
            Ok(c) => c,
            Err(e) => return Response::err(-1, format!("读取备份失败: {e}")),
        };
        if let Err(e) = atomic_write(&canon, &content) {
            return Response::err(-1, e);
        }
        Response::ok(
            "ok",
            Some(json!({
                "path": canon.to_string_lossy(),
                "backup": src.to_string_lossy(),
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// `os_repo.cache_update`：更新包管理器缓存（白名单三个动作）。
pub async fn cache_update(manager: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let manager = if manager.trim().is_empty() {
            detect_manager()
        } else {
            manager.trim().to_string()
        };
        // 参数在 exec 侧拼死：这里不接受任何调用方传进来的命令串。
        let (bin, args): (&str, &[&str]) = match manager.as_str() {
            "apt" | "apt-get" => ("apt-get", &["update"]),
            "dnf" => ("dnf", &["makecache", "-y"]),
            "yum" => ("yum", &["makecache", "-y"]),
            other => {
                return Response::err(
                    -1,
                    format!("不支持的包管理器: {other}（可用 apt-get / dnf / yum）"),
                );
            }
        };
        if !command_exists(bin) {
            return Response::err(-1, format!("系统上未找到 {bin}，无法更新缓存"));
        }
        let out = root_cmd("timeout").arg("600").arg(bin).args(args).output();
        match out {
            Ok(o) => {
                let code = o.status.code().unwrap_or(-1);
                let mut text = String::from_utf8_lossy(&o.stdout).to_string();
                text.push_str(&String::from_utf8_lossy(&o.stderr));
                if text.len() > 4096 {
                    text = text.split_off(text.len() - 4096);
                }
                if code == 0 {
                    Response::ok(
                        "缓存已更新",
                        Some(json!({ "manager": bin, "code": code, "output": text })),
                    )
                } else {
                    Response {
                        code: code as i64,
                        message: format!("{bin} 更新缓存失败（退出码 {code}），请查看输出"),
                        data: Some(json!({ "manager": bin, "code": code, "output": text })),
                    }
                }
            }
            Err(e) => Response::err(-1, format!("执行 {bin} 失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

#[cfg(test)]
mod tests {
    use super::parse_apt_entries;

    /// 把条目简化成 (type, uri, suite, components, enabled) 便于断言。
    fn brief(v: &serde_json::Value) -> (String, String, String, String, bool) {
        (
            v["type"].as_str().unwrap_or("").to_string(),
            v["uri"].as_str().unwrap_or("").to_string(),
            v["suite"].as_str().unwrap_or("").to_string(),
            v["components"].as_str().unwrap_or("").to_string(),
            v["enabled"].as_bool().unwrap_or(true),
        )
    }

    #[test]
    fn parse_one_line_sources_list() {
        let content = "\
deb http://deb.debian.org/debian trixie main contrib non-free non-free-firmware
deb http://deb.debian.org/debian trixie-updates main contrib non-free non-free-firmware

# 注释与被注释的行都跳过
# deb http://example.com/debian trixie main
deb http://deb.debian.org/debian-security trixie-security main
";
        let mut out = Vec::new();
        parse_apt_entries("sources.list", content, &mut out);
        assert_eq!(out.len(), 3, "注释行不应生成条目");
        let first = brief(&out[0]);
        assert_eq!(first.0, "deb");
        assert_eq!(first.1, "http://deb.debian.org/debian");
        assert_eq!(first.2, "trixie");
        assert_eq!(first.3, "main contrib non-free non-free-firmware");
        assert!(first.4);
        // 带组件间空格的安全源
        let sec = brief(&out[2]);
        assert_eq!(sec.2, "trixie-security");
        assert_eq!(sec.3, "main");
    }

    #[test]
    fn parse_one_line_with_options() {
        // `[arch=amd64 trusted=yes]` 选项段要整体跳过，不影响 URI / suite 解析
        let content = "deb [arch=amd64 signed-by=/etc/apt/keyrings/docker.gpg] \
https://download.example.com/linux/debian bookworm stable\n";
        let mut out = Vec::new();
        parse_apt_entries("docker.list", content, &mut out);
        assert_eq!(out.len(), 1);
        let e = brief(&out[0]);
        assert_eq!(e.1, "https://download.example.com/linux/debian");
        assert_eq!(e.2, "bookworm");
        assert_eq!(e.3, "stable");
    }

    #[test]
    fn parse_deb822_sources() {
        // Debian 12+ 默认的 deb822 格式（/etc/apt/sources.list.d/debian.sources）
        let content = "\
Types: deb deb-src
URIs: http://deb.debian.org/debian
Suites: trixie trixie-updates
Components: main contrib non-free non-free-firmware
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg

Types: deb
URIs: http://deb.debian.org/debian-security
Suites: trixie-security
Components: main contrib non-free non-free-firmware
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg
";
        let mut out = Vec::new();
        parse_apt_entries("debian.sources", content, &mut out);
        // 第一 stanza 展开 2 类型 × 1 URI × 2 套件 = 4 条，第二 stanza 1 条
        assert_eq!(out.len(), 5);
        let e = brief(&out[0]);
        assert_eq!(e.1, "http://deb.debian.org/debian");
        assert_eq!(e.2, "trixie");
        assert!(e.4);
        // deb-src 也应生成条目
        assert_eq!(out[2]["type"], "deb-src");
        // 安全源 stanza
        let sec = brief(&out[4]);
        assert_eq!(sec.2, "trixie-security");
    }

    #[test]
    fn parse_deb822_disabled_stanza() {
        let content = "\
Types: deb
URIs: http://example.com/debian
Suites: trixie
Components: main
Enabled: no
";
        let mut out = Vec::new();
        parse_apt_entries("disabled.sources", content, &mut out);
        assert_eq!(out.len(), 1);
        assert!(!brief(&out[0]).4, "Enabled: no 应反映为 disabled");
    }
}
