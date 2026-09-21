//! `zapctl backup` 子命令：备份 / 还原 / 管理 zap 数据。
//!
//! 备份产物为 tar.gz 归档（仅 root 可读，0600），默认目录 `/usr/local/zap/backup`（`--path` 覆盖）：
//! - `backup zap`          → `zap-backup-<时间戳>.tar.gz`，内含 zap.db（VACUUM INTO 一致性快照）、zap.yaml、
//!   主密钥 secret.key 与凭据目录 credentials/（随库携带，换机/迁移还原后加密数据仍可解密）
//! - `backup user <用户>`  → `user-<用户名>-<时间戳>.tar.gz`，内含 user.json（user 表记录）与 home/（家目录）
//! - `backup users`        → `users-<时间戳>.tar.gz`，全部用户归档于 users/<用户名>/ 下
//! - `backup restore <归档>` → 按归档内容自动识别类型并还原（还原前自动备份当前状态，必要时自动停/启服务）
//! - `backup list`         → 列出目录下的归档（名称 / 大小 / 时间）
//! - `backup prune --keep N` → 每类归档（zap / users / 每个 user-<用户名>）各保留最近 N 份，删除更旧的

use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use clap::Subcommand;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, params, params_from_iter};
use serde_json::{Map, Value, json};

use crate::{NC, YELLOW, ensure_root, info, ok};

/// 默认备份目录（zap 安装根下的 backup）
pub const DEFAULT_BACKUP_DIR: &str = "/usr/local/zap/backup";

const DB_FILE: &str = "zap.db";
const CONFIG_FILE: &str = "zap.yaml";
/// 归档内运行环境状态文件名（对应 {data}/server_env.yaml）
const ENV_FILE: &str = "server_env.yaml";
/// 归档内主密钥文件名（顶层，对应生产路径 /etc/zap/secret.key）
const KEY_FILE: &str = "secret.key";
/// 归档内凭据目录名（顶层，对应 /etc/zap/credentials 的整目录拷贝）
const CRED_DIR_ARCHIVE: &str = "credentials";

// 不带 `.service` 后缀：由 [`crate::svc::unit_name`] 按平台补
const UNIT_ZAPD: &str = "zapd";
const UNIT_ZAPEXEC: &str = "zapexec";

#[derive(Subcommand)]
pub enum BackupCommand {
    /// 备份 zap：数据库一致性快照 + zap.yaml + 主密钥 secret.key 与凭据 credentials/
    Zap {
        /// 备份输出目录（默认 /usr/local/zap/backup）
        #[arg(long)]
        path: Option<String>,
    },
    /// 备份单个用户（user 表记录 + 家目录文件）
    User {
        /// 用户名
        username: String,
        /// 备份输出目录（默认 /usr/local/zap/backup）
        #[arg(long)]
        path: Option<String>,
    },
    /// 备份全部用户（合并为一个归档）
    Users {
        /// 备份输出目录（默认 /usr/local/zap/backup）
        #[arg(long)]
        path: Option<String>,
    },
    /// 从备份归档还原数据（自动识别 zap / user / users 归档；还原前自动备份当前状态）
    Restore {
        /// 备份归档文件（zap-backup-*.tar.gz / user-*.tar.gz / users-*.tar.gz）
        archive: PathBuf,
        /// 还原前自动备份的输出目录（默认 /usr/local/zap/backup）
        #[arg(long)]
        path: Option<String>,
    },
    /// 列出备份目录中的归档（名称 / 大小 / 时间）
    List {
        /// 备份目录（默认 /usr/local/zap/backup）
        #[arg(long)]
        path: Option<String>,
    },
    /// 清理过期归档：每类（zap / users / 每个 user-<用户名>）各保留最近 N 份
    Prune {
        /// 每类归档保留的最近份数
        #[arg(long)]
        keep: u32,
        /// 备份目录（默认 /usr/local/zap/backup）
        #[arg(long)]
        path: Option<String>,
    },
}

pub fn dispatch(cmd: BackupCommand, db_path: &str) -> Result<(), String> {
    ensure_root()?;
    match cmd {
        BackupCommand::Zap { path } => backup_zap(db_path, path.as_deref()),
        BackupCommand::User { username, path } => backup_user(db_path, path.as_deref(), &username),
        BackupCommand::Users { path } => backup_users(db_path, path.as_deref()),
        BackupCommand::Restore { archive, path } => cmd_restore(db_path, &archive, path.as_deref()),
        BackupCommand::List { path } => cmd_list(path.as_deref()),
        BackupCommand::Prune { keep, path } => cmd_prune(path.as_deref(), keep),
    }
}

fn warn(msg: &str) {
    println!("{YELLOW}[!]{NC} {msg}");
}

// ── 通用工具 ──────────────────────────────────────────────────

/// 解析并创建备份输出目录（`--path` 缺省用默认目录），返回绝对路径。
fn resolve_root(output: Option<&str>) -> Result<PathBuf, String> {
    let dir = match output {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => PathBuf::from(DEFAULT_BACKUP_DIR),
    };
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("创建备份目录失败 {}: {e}", dir.display()))?;
    dir.canonicalize()
        .map_err(|e| format!("无法解析备份目录 {}: {e}", dir.display()))
}

fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn open_db(db_path: &str) -> Result<Connection, String> {
    Connection::open(db_path).map_err(|e| format!("无法打开数据库 {db_path}: {e}"))
}

/// 创建打包用的临时目录（staging），避免备份中途残留脏文件。
fn make_staging(root: &Path, label: &str) -> Result<PathBuf, String> {
    let staging = root.join(format!(".staging-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("创建临时目录失败: {e}"))?;
    Ok(staging)
}

/// 创建还原用的工作目录（系统临时目录下，与备份目录隔离）。
fn make_workdir(label: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("zap-restore-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败: {e}"))?;
    Ok(dir)
}

/// SQLite 一致性快照（VACUUM INTO，目标文件不可已存在）。
fn vacuum_into(db_path: &str, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        return Err(format!("目标文件已存在: {}", dest.display()));
    }
    let conn = open_db(db_path)?;
    let escaped = dest.to_string_lossy().replace('\'', "''");
    conn.execute_batch(&format!("VACUUM INTO '{escaped}'"))
        .map_err(|e| format!("数据库备份失败: {e}"))
}

/// 将 staging 目录内容打为 tar.gz 并收紧为仅 root 可读。
fn pack(out: &Path, staging: &Path) -> Result<(), String> {
    let status = ProcessCommand::new("tar")
        .arg("-czf")
        .arg(out)
        .arg("-C")
        .arg(staging)
        .arg(".")
        .status()
        .map_err(|e| format!("无法执行 tar: {e}"))?;
    if !status.success() {
        return Err(format!(
            "tar 打包失败（退出码 {}）",
            status.code().map_or_else(|| "?".into(), |c| c.to_string())
        ));
    }
    // 归档内含口令哈希 / 配置密钥，仅 root 可读
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("设置备份文件权限失败 {}: {e}", out.display()))?;
    Ok(())
}

/// 递归复制目录（cp -a，保留权限与软链）。
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    let status = ProcessCommand::new("cp")
        .arg("-a")
        .arg(src)
        .arg(dst)
        .status()
        .map_err(|e| format!("无法执行 cp: {e}"))?;
    if !status.success() {
        return Err(format!("复制 {} 失败", src.display()));
    }
    Ok(())
}

/// 将 `src` 目录的内容复制进已存在的 `dst` 目录（cp -a，保留属主与权限）。
fn copy_dir_contents(src: &Path, dst: &Path) -> Result<(), String> {
    let status = ProcessCommand::new("cp")
        .arg("-a")
        .arg(src.join("."))
        .arg(dst.join("."))
        .status()
        .map_err(|e| format!("无法执行 cp: {e}"))?;
    if !status.success() {
        return Err(format!(
            "复制 {} 内容到 {} 失败",
            src.display(),
            dst.display()
        ));
    }
    Ok(())
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut idx = 0;
    while value >= 1024.0 && idx < UNITS.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[idx])
    }
}

// ── 归档读取 / 识别 ──────────────────────────────────────────

/// 列出归档内条目（tar -tzf）。
fn tar_list(archive: &Path) -> Result<Vec<String>, String> {
    let out = ProcessCommand::new("tar")
        .arg("-tzf")
        .arg(archive)
        .output()
        .map_err(|e| format!("无法执行 tar: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "无法读取归档 {}: {}",
            archive.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// 归档内容识别结果。
enum ArchiveKind {
    /// 顶层含 zap.db：数据库 + 配置
    Zap,
    /// 顶层含 user.json：单个用户
    User,
    /// 含 users/<用户名>/user.json：全部用户
    Users,
}

fn detect_kind(entries: &[String]) -> Result<ArchiveKind, String> {
    // tar 以 `.` 打包，条目可能带 `./` 前缀（如 `./zap.db`），先归一化
    let norm: Vec<&str> = entries
        .iter()
        .map(|e| e.strip_prefix("./").unwrap_or(e))
        .collect();
    let has_zap = norm.contains(&DB_FILE);
    let has_user = norm.contains(&"user.json");
    let has_users = norm
        .iter()
        .any(|e| e.starts_with("users/") && e.ends_with("/user.json"));
    if has_zap {
        Ok(ArchiveKind::Zap)
    } else if has_user {
        Ok(ArchiveKind::User)
    } else if has_users {
        Ok(ArchiveKind::Users)
    } else {
        Err(format!(
            "无法识别归档类型（期望包含 zap.db / user.json / users/<用户名>/user.json），首条目: {}",
            norm.first().copied().unwrap_or("（空归档）")
        ))
    }
}

/// 解压归档到 `dest`（解压前校验条目路径，防止路径穿越）。
fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    let entries = tar_list(archive)?;
    for e in &entries {
        let p = Path::new(e);
        if p.is_absolute()
            || p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!("归档包含不安全路径，已中止: {e}"));
        }
    }
    let status = ProcessCommand::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
        .map_err(|e| format!("无法执行 tar: {e}"))?;
    if !status.success() {
        return Err(format!("归档解压失败: {}", archive.display()));
    }
    Ok(())
}

// ── 服务协调（还原数据库期间避免进程占用）────────────────────

// ── 用户数据导出 ──────────────────────────────────────────────

/// user 表记录 JSON 对象 + 家目录路径。
type UserRow = (Map<String, Value>, Option<String>);

/// 读取 user 表整行并序列化为 JSON 对象；返回 (记录, 家目录路径)。
fn fetch_user(conn: &Connection, username: &str) -> Result<Option<UserRow>, String> {
    let mut stmt = conn
        .prepare("SELECT * FROM user WHERE username = ?1")
        .map_err(|e| e.to_string())?;
    let cols: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let mut rows = stmt.query(params![username]).map_err(|e| e.to_string())?;
    let Some(row) = rows.next().map_err(|e| e.to_string())? else {
        return Ok(None);
    };

    let mut obj = Map::new();
    for (i, col) in cols.iter().enumerate() {
        let v = if let Ok(Some(n)) = row.get::<_, Option<i64>>(i) {
            json!(n)
        } else if let Ok(Some(n)) = row.get::<_, Option<f64>>(i) {
            json!(n)
        } else if let Ok(Some(s)) = row.get::<_, Option<String>>(i) {
            json!(s)
        } else {
            Value::Null
        };
        obj.insert(col.clone(), v);
    }
    let home_dir = obj
        .get("home_dir")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Ok(Some((obj, home_dir)))
}

fn write_user_json(dir: &Path, obj: &Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(obj).map_err(|e| format!("序列化用户数据失败: {e}"))?;
    std::fs::write(dir.join("user.json"), text).map_err(|e| format!("写入 user.json 失败: {e}"))
}

/// 将家目录复制到 `<dir>/home/`；返回是否实际包含文件。
fn copy_home(dir: &Path, home_dir: Option<String>) -> Result<bool, String> {
    match home_dir {
        Some(h) if !h.trim().is_empty() => {
            let src = Path::new(&h);
            if src.is_dir() {
                copy_tree(src, &dir.join("home"))?;
                Ok(true)
            } else {
                warn(&format!("家目录不存在，跳过文件备份: {h}"));
                Ok(false)
            }
        }
        _ => {
            warn("该用户无家目录记录，仅备份数据库记录");
            Ok(false)
        }
    }
}

// ── 各备份动作 ────────────────────────────────────────────────

fn backup_zap(db_path: &str, output: Option<&str>) -> Result<(), String> {
    let root = resolve_root(output)?;
    if !Path::new(db_path).exists() {
        return Err(format!("数据库不存在: {db_path}"));
    }
    info("备份 zap 数据库、配置、主密钥与凭据 ...");

    let staging = make_staging(&root, "zap")?;
    vacuum_into(db_path, &staging.join(DB_FILE))?;

    // 配置文件：ZAP_CONFIG > /etc/zap/zap.yaml > conf/zap.yaml
    let cfg = crate::config::config_path();
    if cfg.exists() {
        std::fs::copy(&cfg, staging.join(CONFIG_FILE))
            .map_err(|e| format!("备份配置文件失败 {}: {e}", cfg.display()))?;
    } else {
        warn("未找到配置文件，仅备份数据库");
    }

    // 运行环境状态（{data}/server_env.yaml）：面板/建站默认配置 + 自动探测快照
    let env_file = crate::env::env_path(db_path);
    if env_file.exists() {
        std::fs::copy(&env_file, staging.join(ENV_FILE))
            .map_err(|e| format!("备份运行环境状态失败 {}: {e}", env_file.display()))?;
        ok(&format!("已纳入运行环境状态: {}", env_file.display()));
    } else {
        info("无 server_env.yaml，跳过");
    }

    // 主密钥：随库一起携带，保证换机/迁移还原后 zap.db 加密列与凭据仍可解密
    if let Some(key) = zap_crypto::active_key_path() {
        std::fs::copy(&key, staging.join(KEY_FILE))
            .map_err(|e| format!("备份主密钥失败 {}: {e}", key.display()))?;
        ok(&format!("已纳入主密钥: {}", key.display()));
    } else {
        warn("未找到主密钥文件（/etc/zap/secret.key）：加密数据将无法在还原后解密");
    }

    // 凭据目录（/etc/zap/credentials，目录 0700 / 文件 0400，cp -a 保留权限）
    let cred_dir = Path::new(zap_crypto::CRED_DIR);
    if cred_dir.is_dir() {
        copy_tree(cred_dir, &staging.join(CRED_DIR_ARCHIVE))?;
        ok(&format!("已纳入凭据目录: {}", cred_dir.display()));
    } else {
        info("无凭据目录，跳过");
    }

    let out = root.join(format!("zap-backup-{}.tar.gz", timestamp()));
    pack(&out, &staging)?;
    cleanup(&staging);
    ok(&format!(
        "备份完成（含主密钥与凭据，可整机迁移）: {}",
        out.display()
    ));
    Ok(())
}

fn backup_user(db_path: &str, output: Option<&str>, username: &str) -> Result<(), String> {
    let root = resolve_root(output)?;
    let conn = open_db(db_path)?;
    let Some((obj, home)) = fetch_user(&conn, username)? else {
        return Err(format!("用户 {username} 不存在"));
    };
    info(&format!("备份用户 {username} ..."));

    let staging = make_staging(&root, &format!("user-{username}"))?;
    write_user_json(&staging, &obj)?;
    copy_home(&staging, home)?;

    let out = root.join(format!("user-{username}-{}.tar.gz", timestamp()));
    pack(&out, &staging)?;
    cleanup(&staging);
    ok(&format!("用户 {username} 已备份: {}", out.display()));
    Ok(())
}

fn backup_users(db_path: &str, output: Option<&str>) -> Result<(), String> {
    let root = resolve_root(output)?;
    let conn = open_db(db_path)?;

    let mut stmt = conn
        .prepare("SELECT username FROM user ORDER BY id")
        .map_err(|e| e.to_string())?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    if names.is_empty() {
        return Err("没有可备份的用户".to_string());
    }
    info(&format!("备份全部用户（共 {} 个）...", names.len()));

    let staging = make_staging(&root, "users")?;
    let users_dir = staging.join("users");
    std::fs::create_dir_all(&users_dir).map_err(|e| format!("创建临时目录失败: {e}"))?;

    for name in &names {
        let dir = users_dir.join(name);
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败: {e}"))?;
        if let Some((obj, home)) = fetch_user(&conn, name)? {
            write_user_json(&dir, &obj)?;
            copy_home(&dir, home)?;
        }
    }

    let out = root.join(format!("users-{}.tar.gz", timestamp()));
    pack(&out, &staging)?;
    cleanup(&staging);
    ok(&format!("全部用户已备份: {}", out.display()));
    Ok(())
}

// ── list / prune：备份目录管理 ────────────────────────────────

struct ArchiveEntry {
    path: PathBuf,
    name: String,
    series: String,
    /// 排序用时间戳（优先取文件名内嵌时间，缺省取文件 mtime，单位：秒）
    eff_time: i64,
    display_time: String,
    size: u64,
}

/// 从文件名解析出（系列前缀, 内嵌时间戳）：`<系列>-<YYYYmmdd-HHMMSS>.tar.gz`。
fn parse_series(name: &str) -> Option<(String, Option<chrono::NaiveDateTime>)> {
    let stem = name.strip_suffix(".tar.gz")?;
    // 时间戳部分固定 15 位，其前一位为分隔符 `-`
    if stem.len() > 15 && stem.as_bytes().get(stem.len() - 16) == Some(&b'-') {
        let series = &stem[..stem.len() - 16];
        let ts_part = &stem[stem.len() - 15..];
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(ts_part, "%Y%m%d-%H%M%S") {
            return Some((series.to_string(), Some(ndt)));
        }
    }
    Some((stem.to_string(), None))
}

fn is_known_series(series: &str) -> bool {
    series == "zap-backup" || series == "users" || series.starts_with("user-")
}

/// 扫描备份目录中 zapctl 生成的归档。
fn scan_archives(root: &Path) -> Result<Vec<ArchiveEntry>, String> {
    let mut out = Vec::new();
    let rd =
        std::fs::read_dir(root).map_err(|e| format!("读取备份目录失败 {}: {e}", root.display()))?;
    for ent in rd.flatten() {
        let path = ent.path();
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if !name.ends_with(".tar.gz") {
            continue;
        }
        let Some((series, ts)) = parse_series(&name) else {
            continue;
        };
        if !is_known_series(&series) {
            continue;
        }
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        let (eff_time, display_time) = match ts {
            Some(ndt) => (
                ndt.and_utc().timestamp(),
                ndt.format("%Y-%m-%d %H:%M:%S").to_string(),
            ),
            None => {
                use chrono::TimeZone;
                let secs = meta
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let display = chrono::Local
                    .timestamp_opt(secs, 0)
                    .single()
                    .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "-".to_string());
                (secs, display)
            }
        };
        out.push(ArchiveEntry {
            path,
            name,
            series,
            eff_time,
            display_time,
            size: meta.len(),
        });
    }
    Ok(out)
}

fn cmd_list(output: Option<&str>) -> Result<(), String> {
    let root = resolve_root(output)?;
    let mut entries = scan_archives(&root)?;
    if entries.is_empty() {
        println!("备份目录 {} 下暂无归档", root.display());
        return Ok(());
    }
    entries.sort_by(|a, b| {
        b.eff_time
            .cmp(&a.eff_time)
            .then_with(|| b.name.cmp(&a.name))
    });
    let total: u64 = entries.iter().map(|e| e.size).sum();
    println!(
        "备份目录 {}（共 {} 份，总大小 {}）",
        root.display(),
        entries.len(),
        human_size(total)
    );
    let (h_size, h_time, h_file) = ("SIZE", "TIME", "FILE");
    println!("{:>10}  {:<19}  {}", h_size, h_time, h_file);
    for e in &entries {
        println!(
            "{:>10}  {:<19}  {}",
            human_size(e.size),
            e.display_time,
            e.name
        );
    }
    Ok(())
}

fn cmd_prune(output: Option<&str>, keep: u32) -> Result<(), String> {
    if keep == 0 {
        return Err("--keep 必须大于 0".to_string());
    }
    let root = resolve_root(output)?;
    let entries = scan_archives(&root)?;
    if entries.is_empty() {
        info("备份目录下暂无归档，无需清理");
        return Ok(());
    }

    // 按系列分组（zap-backup / users / 每个 user-<用户名>）
    let mut by_series: std::collections::BTreeMap<String, Vec<ArchiveEntry>> =
        std::collections::BTreeMap::new();
    for e in entries {
        by_series.entry(e.series.clone()).or_default().push(e);
    }

    let mut removed: Vec<(String, PathBuf)> = Vec::new();
    let mut kept = 0usize;
    for (series, mut list) in by_series {
        list.sort_by(|a, b| {
            a.eff_time
                .cmp(&b.eff_time)
                .then_with(|| a.name.cmp(&b.name))
        });
        let drop = list.len().saturating_sub(keep as usize);
        if drop > 0 {
            info(&format!(
                "归档类 {series}：共 {} 份，保留最近 {keep} 份，删除最旧的 {drop} 份",
                list.len()
            ));
        }
        for (i, e) in list.into_iter().enumerate() {
            if i < drop {
                removed.push((series.clone(), e.path));
            } else {
                kept += 1;
            }
        }
    }

    if removed.is_empty() {
        ok("无需清理：各类归档均在保留范围内");
        return Ok(());
    }
    for (series, p) in &removed {
        std::fs::remove_file(p).map_err(|e| format!("删除 {} 失败: {e}", p.display()))?;
        println!("  已删除 [{series}] {}", p.display());
    }
    ok(&format!("保留 {kept} 份，删除 {} 份", removed.len()));
    Ok(())
}

// ── restore：还原 ─────────────────────────────────────────────

fn cmd_restore(db_path: &str, archive: &Path, output: Option<&str>) -> Result<(), String> {
    if !archive.is_file() {
        return Err(format!("归档文件不存在: {}", archive.display()));
    }
    let entries = tar_list(archive)?;
    let kind = detect_kind(&entries)?;
    let label = match kind {
        ArchiveKind::Zap => "zap",
        ArchiveKind::User => "user",
        ArchiveKind::Users => "users",
    };
    let work = make_workdir(label)?;
    let result = (|| {
        extract_archive(archive, &work)?;
        match kind {
            ArchiveKind::Zap => restore_zap(db_path, &work, output),
            ArchiveKind::User => restore_one_user_dir(db_path, &work),
            ArchiveKind::Users => restore_all_users(db_path, &work),
        }
    })();
    cleanup(&work);
    result
}

/// 还原 zap 数据库与配置。还原前自动备份当前状态，并自动停/启占用数据库的服务。
fn restore_zap(db_path: &str, staging: &Path, output: Option<&str>) -> Result<(), String> {
    if !staging.join(DB_FILE).is_file() {
        return Err("归档中缺少 zap.db".to_string());
    }

    // 1. 还原前自动备份当前状态（误操作可回滚）
    if Path::new(db_path).exists() {
        info("还原前自动备份当前状态 ...");
        backup_zap(db_path, output)?;
    } else {
        info("当前无数据库文件，跳过还原前备份");
    }

    // 2. 停掉占用数据库的服务（仅停当前处于运行状态的）
    let mut was_active: Vec<String> = Vec::new();
    for base in [UNIT_ZAPD, UNIT_ZAPEXEC] {
        let unit = crate::svc::unit_name(base);
        if crate::svc::is_active(&unit) {
            was_active.push(unit);
        }
    }
    if !was_active.is_empty() {
        info(&format!("停止服务：{}", was_active.join(" ")));
        let mut stopped: Vec<&String> = Vec::new();
        for unit in &was_active {
            if let Err(e) = crate::svc::act("stop", unit) {
                for s in &stopped {
                    let _ = crate::svc::act("start", s);
                }
                return Err(e);
            }
            stopped.push(unit);
        }
    }

    // 3. 还原文件
    info(&format!("正在还原数据库 {db_path} ..."));
    let restore = do_zap_restore(db_path, staging);

    // 4. 恢复服务原状态（无论还原成败）。
    // 先 zapexec 后 zapd，与 zapd.service 的 After=zapexec.service 保持一致：
    // 全新机器上 exec.key 由 zapexec 首启生成（缺则自建），zapd 随后才能读到。
    let restart = (|| -> Result<(), String> {
        if was_active.is_empty() {
            return Ok(());
        }
        info(&format!("启动服务：{}", was_active.join(" ")));
        for base in [UNIT_ZAPEXEC, UNIT_ZAPD] {
            let unit = crate::svc::unit_name(base);
            if was_active.contains(&unit) {
                crate::svc::act("start", &unit)?;
            }
        }
        Ok(())
    })();

    match (restore, restart) {
        (Ok(()), Ok(())) => {
            ok("zap 数据还原完成");
            Ok(())
        }
        (Err(e), Ok(())) => Err(e),
        (Ok(()), Err(e)) => Err(format!("数据已还原，但服务启动失败: {e}")),
        (Err(e1), Err(e2)) => Err(format!("{e1}；另外服务启动失败: {e2}")),
    }
}

/// 覆盖旧库 / 配置（调用前需已停止 zapd / zapexec）。
fn do_zap_restore(db_path: &str, staging: &Path) -> Result<(), String> {
    // 移除旧库及可能的 WAL / SHM，防止残留数据串库
    for suffix in ["", "-wal", "-shm"] {
        let p = format!("{db_path}{suffix}");
        let pp = Path::new(&p);
        if pp.exists() {
            std::fs::remove_file(pp).map_err(|e| format!("删除旧文件失败 {p}: {e}"))?;
        }
    }
    if let Some(parent) = Path::new(db_path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
    }
    std::fs::copy(staging.join(DB_FILE), db_path).map_err(|e| format!("还原数据库失败: {e}"))?;

    if staging.join(CONFIG_FILE).is_file() {
        let cfg = crate::config::config_path();
        if let Some(parent) = cfg.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
        }
        std::fs::copy(staging.join(CONFIG_FILE), &cfg)
            .map_err(|e| format!("还原配置文件失败 {}: {e}", cfg.display()))?;
        ok(&format!("已还原配置文件 {}", cfg.display()));
    } else {
        warn("归档未包含 zap.yaml，仅还原数据库");
    }

    // 运行环境状态：还原到 {data}/server_env.yaml（与 zap.db 同级）
    if staging.join(ENV_FILE).is_file() {
        let dst = crate::env::env_path(db_path);
        if let Some(parent) = dst.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
        }
        std::fs::copy(staging.join(ENV_FILE), &dst)
            .map_err(|e| format!("还原运行环境状态失败: {e}"))?;
        ok(&format!("已还原运行环境状态 {}", dst.display()));
    }

    // 主密钥：归档含 secret.key 时以归档密钥覆盖本机密钥（0600），
    // 保证还原后的 zap.db 加密列与凭据可用同一密钥解密（全量还原语义）
    if staging.join(KEY_FILE).is_file() {
        let key_path = Path::new("/etc/zap/secret.key");
        if let Some(parent) = key_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
        }
        std::fs::copy(staging.join(KEY_FILE), key_path)
            .map_err(|e| format!("还原主密钥失败 {}: {e}", key_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("设置主密钥权限失败: {e}"))?;
        }
        ok(&format!("已还原主密钥: {}", key_path.display()));
    } else {
        warn("归档未包含 secret.key（旧版备份？）：若本机密钥与归档不一致，加密数据将无法解密");
    }

    // 凭据目录：整目录替换（先清旧，避免归档之外的凭据残留）
    if staging.join(CRED_DIR_ARCHIVE).is_dir() {
        let cred_dir = Path::new(zap_crypto::CRED_DIR);
        if cred_dir.exists() {
            std::fs::remove_dir_all(cred_dir).map_err(|e| format!("清理旧凭据目录失败: {e}"))?;
        }
        copy_tree(&staging.join(CRED_DIR_ARCHIVE), cred_dir)?;
        ok(&format!("已还原凭据目录: {}", cred_dir.display()));
    } else {
        warn("归档未包含 credentials/（旧版备份？）：跳过凭据还原");
    }
    Ok(())
}

// ── 用户还原（user / users 归档）──────────────────────────────

/// 还原 users-*.tar.gz 中的全部用户。
fn restore_all_users(db_path: &str, staging: &Path) -> Result<(), String> {
    let base = staging.join("users");
    let mut names: Vec<String> = Vec::new();
    let rd = std::fs::read_dir(&base).map_err(|e| format!("读取归档 users/ 失败: {e}"))?;
    for ent in rd.flatten() {
        if ent.path().is_dir()
            && let Some(n) = ent.file_name().to_str().map(str::to_string)
        {
            names.push(n);
        }
    }
    names.sort();
    if names.is_empty() {
        return Err("归档中未找到任何用户数据".to_string());
    }
    info(&format!("还原全部用户（共 {} 个）...", names.len()));
    let mut restored = 0usize;
    for name in &names {
        let dir = base.join(name);
        if !dir.join("user.json").is_file() {
            warn(&format!("跳过 {name}：缺少 user.json"));
            continue;
        }
        restore_one_user_dir(db_path, &dir)?;
        restored += 1;
    }
    ok(&format!("已还原 {restored} 个用户"));
    Ok(())
}

/// 还原单个用户目录（内含 user.json，可选 home/ 家目录文件）。
fn restore_one_user_dir(db_path: &str, dir: &Path) -> Result<(), String> {
    let json_path = dir.join("user.json");
    let text = std::fs::read_to_string(&json_path)
        .map_err(|e| format!("读取 {} 失败: {e}", json_path.display()))?;
    let obj: Map<String, Value> = serde_json::from_str(&text)
        .map_err(|e| format!("解析 {} 失败: {e}", json_path.display()))?;

    let username = obj
        .get("username")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{} 缺少有效的 username 字段", json_path.display()))?;

    let conn = open_db(db_path)?;
    upsert_user(&conn, &obj, username)?;
    restore_home_files(&conn, dir, username)?;
    Ok(())
}

/// 将 serde_json 值转换为可绑定的 rusqlite 值。
fn json_to_sql(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(*b as i64),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SqlValue::Integer(i)
            } else if let Some(u) = n.as_u64() {
                SqlValue::Integer(u.min(i64::MAX as u64) as i64)
            } else {
                SqlValue::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => SqlValue::Text(s.clone()),
        _ => SqlValue::Null,
    }
}

/// 导入 / 覆盖用户记录：用户名已存在则覆盖其字段，否则新增（不触碰自增 id，避免破坏外键）。
fn upsert_user(conn: &Connection, obj: &Map<String, Value>, username: &str) -> Result<(), String> {
    // 当前表结构列名（归档来自旧版本时可自动裁剪多余字段）
    let mut table_cols: Vec<String> = Vec::new();
    {
        let mut stmt = conn
            .prepare("PRAGMA table_info(user)")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            if let Ok(name) = row.get::<_, String>(1) {
                table_cols.push(name);
            }
        }
    }
    // 归档与表结构共有的可写字段（id 交由自增维护）
    let fields: Vec<String> = obj
        .keys()
        .filter(|k| {
            let key = k.as_str();
            key != "id" && table_cols.iter().any(|c| c.as_str() == key)
        })
        .cloned()
        .collect();

    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM user WHERE username = ?1",
            params![username],
            |_| Ok(()),
        )
        .is_ok();

    if exists {
        warn(&format!("用户 {username} 已存在，将用归档数据覆盖其记录"));
        let set_fields: Vec<&String> = fields.iter().filter(|f| f.as_str() != "username").collect();
        if set_fields.is_empty() {
            ok(&format!("用户 {username} 无需更新"));
            return Ok(());
        }
        let set_sql = set_fields
            .iter()
            .map(|f| format!("{f} = ?"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut values: Vec<SqlValue> = set_fields
            .iter()
            .map(|f| json_to_sql(obj.get(f.as_str()).unwrap_or(&Value::Null)))
            .collect();
        values.push(SqlValue::Text(username.to_string()));
        let sql = format!("UPDATE user SET {set_sql} WHERE username = ?");
        conn.execute(&sql, params_from_iter(values.iter()))
            .map_err(|e| format!("更新用户 {username} 记录失败: {e}"))?;
        ok(&format!("用户 {username} 记录已覆盖"));
    } else {
        let col_sql = fields.join(", ");
        let placeholders = fields.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let values: Vec<SqlValue> = fields
            .iter()
            .map(|f| json_to_sql(obj.get(f).unwrap_or(&Value::Null)))
            .collect();
        let sql = format!("INSERT INTO user ({col_sql}) VALUES ({placeholders})");
        conn.execute(&sql, params_from_iter(values.iter()))
            .map_err(|e| {
                let hint = if e.to_string().contains("UNIQUE") {
                    "（用户名/邮箱/手机号与现有记录冲突）"
                } else {
                    ""
                };
                format!("导入用户 {username} 失败{hint}: {e}")
            })?;
        ok(&format!("用户 {username} 记录已导入"));
    }
    Ok(())
}

/// 将归档中的 home/ 文件还原到该用户当前的 home_dir。
fn restore_home_files(conn: &Connection, dir: &Path, username: &str) -> Result<(), String> {
    if !dir.join("home").is_dir() {
        return Ok(()); // 归档仅含记录，无文件可还原
    }
    let home: Option<String> = conn
        .query_row(
            "SELECT home_dir FROM user WHERE username = ?1",
            params![username],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let Some(home) = home.filter(|h| !h.trim().is_empty()) else {
        warn(&format!(
            "用户 {username} 无 home_dir 记录，跳过家目录文件还原"
        ));
        return Ok(());
    };
    if home == "/" {
        return Err("拒绝将家目录文件还原到文件系统根目录".to_string());
    }
    let target = PathBuf::from(&home);
    if target.is_file() {
        return Err(format!("home_dir 指向的不是目录: {home}"));
    }
    std::fs::create_dir_all(&target).map_err(|e| format!("创建家目录失败 {home}: {e}"))?;
    copy_dir_contents(&dir.join("home"), &target)?;
    ok(&format!("用户 {username} 家目录文件已还原到 {home}"));
    Ok(())
}
