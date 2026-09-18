//! `zapctl env` 子命令：管理 `{data}/server_env.yaml`（运行环境状态）。
//!
//! 数据分两层：
//! - `conf`：管理员维护的全局配置（webserver / php_default / database /
//!   vhost_mode / fpm_pool_defaults / user_home_root / basic_* 等）；
//! - `auto`：zapexec 自动探测的快照（payload），由面板自动刷新，**只读**。
//!
//! YAML 为唯一事实来源（面板启动时加载，变更即时写回）。本子命令直接读写该文件，
//! 写操作需 root；zapd 会在下次读取时按 mtime 自动感知改动，无需重启。
//! 仅对少数「写错即功能异常」的键做轻量校验，其余交给面板逻辑兜底。

use std::io::Read;
use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{NC, YELLOW, ensure_root, ok};

/// 键名最大长度。
const MAX_KEY_LEN: usize = 128;
/// 值最大长度（64 KiB）。
const MAX_VALUE_LEN: usize = 64 * 1024;
/// 状态文件名（与 zapd 保持一致）。
const FILE_NAME: &str = "server_env.yaml";

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Scope {
    /// 管理员全局配置
    Conf,
    /// 自动探测快照（只读）
    Auto,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Scope::Conf => "conf",
            Scope::Auto => "auto",
        }
    }
}

#[derive(Subcommand)]
pub enum EnvCommand {
    /// 列出记录（缺省列出全部 scope）
    List {
        /// 仅列出指定 scope
        #[arg(long, value_enum)]
        scope: Option<Scope>,
        /// 以 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 读取单个键
    Get {
        /// 键名
        key: String,
        /// 目标 scope
        #[arg(long, value_enum, default_value_t = Scope::Conf)]
        scope: Scope,
        /// 以 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 新增或修改键值（upsert）
    Set {
        /// 键名
        key: String,
        /// 值；传 `-` 表示从标准输入读取
        value: String,
        /// 目标 scope
        #[arg(long, value_enum, default_value_t = Scope::Conf)]
        scope: Scope,
        /// 备注（缺省保留原备注）
        #[arg(long)]
        remark: Option<String>,
        /// 以 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 删除键
    #[command(alias = "rm", alias = "delete")]
    Unset {
        /// 键名
        key: String,
        /// 目标 scope
        #[arg(long, value_enum, default_value_t = Scope::Conf)]
        scope: Scope,
        /// 以 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 从文件批量导入（逐行 `key=value`，忽略空行与 # 注释）
    Import {
        /// 文件路径（`-` 表示标准输入）
        file: String,
        /// 目标 scope
        #[arg(long, value_enum, default_value_t = Scope::Conf)]
        scope: Scope,
        /// 仅校验并预览，不写入
        #[arg(long)]
        dry_run: bool,
        /// 跳过非法行（默认遇错中止）
        #[arg(long)]
        skip_errors: bool,
        /// 以 JSON 输出
        #[arg(long)]
        json: bool,
    },
}

pub fn dispatch(cmd: EnvCommand, db_path: &str) -> Result<(), String> {
    match cmd {
        EnvCommand::List { scope, json } => cmd_list(db_path, scope, json),
        EnvCommand::Get { key, scope, json } => cmd_get(db_path, &key, scope, json),
        EnvCommand::Set {
            key,
            value,
            scope,
            remark,
            json,
        } => cmd_set(db_path, &key, &value, scope, remark.as_deref(), json),
        EnvCommand::Unset { key, scope, json } => cmd_unset(db_path, &key, scope, json),
        EnvCommand::Import {
            file,
            scope,
            dry_run,
            skip_errors,
            json,
        } => cmd_import(db_path, &file, scope, dry_run, skip_errors, json),
    }
}

// ── 状态文件读写 ──────────────────────────────────────────────

/// `{data}/server_env.yaml`：与 zap.db 同级目录。
///
/// 与 zapd 的 `data_dir()` 算法一致（相对路径相对当前工作目录解析）。
pub fn env_path(db_path: &str) -> PathBuf {
    let p = Path::new(db_path);
    let dir = match p.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("data"),
    };
    dir.join(FILE_NAME)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ConfEntry {
    #[serde(default)]
    value: String,
    #[serde(default)]
    remark: String,
    #[serde(default)]
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AutoSection {
    #[serde(default)]
    detected_at: i64,
    #[serde(default)]
    payload: Option<Value>,
    #[serde(default)]
    error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct EnvFile {
    #[serde(default)]
    version: i64,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    conf: std::collections::BTreeMap<String, ConfEntry>,
    #[serde(default)]
    auto: AutoSection,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn load(path: &Path) -> Result<EnvFile, String> {
    if !path.exists() {
        return Ok(EnvFile {
            version: 1,
            ..Default::default()
        });
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("无法读取 {}: {e}", path.display()))?;
    let mut f: EnvFile =
        serde_yaml::from_str(&text).map_err(|e| format!("{} 格式错误: {e}", path.display()))?;
    if f.version == 0 {
        f.version = 1;
    }
    Ok(f)
}

/// 原子写回（tmp + rename）。
fn save(path: &Path, file: &EnvFile) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
    }
    let mut file = file.clone();
    file.version = 1;
    file.updated_at = now();
    let text = serde_yaml::to_string(&file).map_err(|e| format!("序列化失败: {e}"))?;
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, &text).map_err(|e| format!("写入 {} 失败: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("替换 {} 失败: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

// ── 校验 ──────────────────────────────────────────────────────

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("键名不能为空".to_string());
    }
    if key.len() > MAX_KEY_LEN {
        return Err(format!("键名长度超限（最大 {MAX_KEY_LEN} 字符）"));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err("键名仅支持字母 / 数字 / 下划线 / 点 / 短横线".to_string());
    }
    Ok(())
}

fn validate_value(value: &str) -> Result<(), String> {
    if value.len() > MAX_VALUE_LEN {
        return Err(format!("值长度超限（最大 {MAX_VALUE_LEN} 字节）"));
    }
    Ok(())
}

/// 对少数「写错即功能异常」的键做轻量校验（与面板写入规则保持一致）。
fn validate_known_key(key: &str, value: &str) -> Result<(), String> {
    match key {
        // 运行模式已固定为独立系统用户（已移除统一 www 模式）
        "vhost_mode" if value != "system" => {
            Err("vhost_mode 已固定为 system（独立系统用户）".to_string())
        }
        "fpm_pool_defaults"
            if !value.is_empty()
                && !matches!(serde_json::from_str::<Value>(value), Ok(Value::Object(_))) =>
        {
            Err("fpm_pool_defaults 必须是 JSON 对象".to_string())
        }
        "basic_mail_encryption" if !["ssl", "tls", "none"].contains(&value) => {
            Err("basic_mail_encryption 仅支持 ssl / tls / none".to_string())
        }
        _ => Ok(()),
    }
}

/// 写操作前的公共检查：禁止写 auto 快照。
fn ensure_writable(scope: Scope) -> Result<(), String> {
    if scope == Scope::Auto {
        return Err("scope=auto 为自动探测快照（只读），不允许写入".to_string());
    }
    Ok(())
}

// ── 子命令实现 ────────────────────────────────────────────────

fn cmd_list(db_path: &str, scope: Option<Scope>, json_out: bool) -> Result<(), String> {
    let path = env_path(db_path);
    let file = load(&path)?;

    #[derive(serde::Serialize)]
    struct Row {
        scope: &'static str,
        key: String,
        value: String,
        remark: String,
        updated_at: i64,
    }

    let mut rows: Vec<Row> = Vec::new();
    if matches!(scope, None | Some(Scope::Conf)) {
        for (k, e) in &file.conf {
            rows.push(Row {
                scope: "conf",
                key: k.clone(),
                value: e.value.clone(),
                remark: e.remark.clone(),
                updated_at: e.updated_at,
            });
        }
    }
    if matches!(scope, None | Some(Scope::Auto)) {
        let preview = match &file.auto.payload {
            Some(v) => v.to_string(),
            None => String::new(),
        };
        rows.push(Row {
            scope: "auto",
            key: "payload".to_string(),
            value: preview,
            remark: "zapexec 自动探测快照".to_string(),
            updated_at: file.auto.detected_at,
        });
    }

    if json_out {
        let arr: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "scope": r.scope,
                    "key": r.key,
                    "value": r.value,
                    "remark": r.remark,
                    "updated_at": r.updated_at,
                })
            })
            .collect();
        println!("{}", to_json(&Value::Array(arr))?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("(无记录)");
        return Ok(());
    }

    println!(
        "{:<6} {:<28} {:<40} {:<20} {:<20}",
        "SCOPE", "KEY", "VALUE", "REMARK", "UPDATED"
    );
    println!("{}", "-".repeat(118));
    for r in &rows {
        println!(
            "{:<6} {:<28} {:<40} {:<20} {:<20}",
            r.scope,
            ellipsis(&r.key, 28),
            ellipsis(&r.value, 40),
            ellipsis(&r.remark, 20),
            format_time(r.updated_at),
        );
    }
    Ok(())
}

fn cmd_get(db_path: &str, key: &str, scope: Scope, json_out: bool) -> Result<(), String> {
    validate_key(key)?;
    let path = env_path(db_path);
    let file = load(&path)?;

    let (value, remark, ts) = match scope {
        Scope::Conf => {
            let Some(e) = file.conf.get(key) else {
                return Err(format!("{}:{key} 不存在", scope.as_str()));
            };
            (e.value.clone(), e.remark.clone(), e.updated_at)
        }
        Scope::Auto => {
            if key != "payload" {
                return Err(format!("{}:{key} 不存在", scope.as_str()));
            }
            (
                file.auto
                    .payload
                    .clone()
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                "zapexec 自动探测快照".to_string(),
                file.auto.detected_at,
            )
        }
    };

    if json_out {
        let out = json!({
            "scope": scope.as_str(),
            "key": key,
            "value": value,
            "remark": remark,
            "updated_at": ts,
        });
        println!("{}", to_json(&out)?);
    } else {
        println!("{value}");
    }
    Ok(())
}

fn cmd_set(
    db_path: &str,
    key: &str,
    value: &str,
    scope: Scope,
    remark: Option<&str>,
    json_out: bool,
) -> Result<(), String> {
    ensure_root()?;
    ensure_writable(scope)?;
    validate_key(key)?;

    let value = if value == "-" {
        read_stdin()?
    } else {
        value.to_string()
    };
    validate_value(&value)?;
    validate_known_key(key, value.trim())?;

    let path = env_path(db_path);
    let mut file = load(&path)?;
    let entry = file.conf.entry(key.to_string()).or_default();
    entry.value = value.clone();
    entry.updated_at = now();
    if let Some(r) = remark {
        entry.remark = r.to_string();
    }
    save(&path, &file)?;

    if json_out {
        let out = json!({
            "ok": true,
            "action": "set",
            "scope": scope.as_str(),
            "key": key,
            "value": value,
            "remark": remark,
        });
        println!("{}", to_json(&out)?);
    } else {
        ok(&format!("已设置 {}:{key}", scope.as_str()));
        println!("{YELLOW}[!]{NC} 面板与建站流程按 mtime 自动感知改动，无需重启 zapd");
    }
    Ok(())
}

fn cmd_unset(db_path: &str, key: &str, scope: Scope, json_out: bool) -> Result<(), String> {
    ensure_root()?;
    ensure_writable(scope)?;
    validate_key(key)?;

    let path = env_path(db_path);
    let mut file = load(&path)?;
    if file.conf.remove(key).is_none() {
        return Err(format!("{}:{key} 不存在", scope.as_str()));
    }
    save(&path, &file)?;

    if json_out {
        let out = json!({
            "ok": true,
            "action": "unset",
            "scope": scope.as_str(),
            "key": key,
            "removed": 1,
        });
        println!("{}", to_json(&out)?);
    } else {
        ok(&format!("已删除 {}:{key}", scope.as_str()));
    }
    Ok(())
}

fn cmd_import(
    db_path: &str,
    file: &str,
    scope: Scope,
    dry_run: bool,
    skip_errors: bool,
    json_out: bool,
) -> Result<(), String> {
    ensure_writable(scope)?;
    if !dry_run {
        ensure_root()?;
    }

    let content = if file == "-" {
        read_stdin()?
    } else {
        std::fs::read_to_string(file).map_err(|e| format!("无法读取文件 {file}: {e}"))?
    };

    let mut items: Vec<(String, String)> = Vec::new();
    let mut skipped = 0usize;
    for (idx, raw) in content.lines().enumerate() {
        let lineno = idx + 1;
        let parsed = match parse_env_line(raw) {
            Ok(Some(pair)) => {
                let (k, v) = pair;
                match validate_key(k)
                    .and_then(|()| validate_value(v))
                    .and_then(|()| validate_known_key(k, v.trim()))
                {
                    Ok(()) => Ok(Some((k.to_string(), v.to_string()))),
                    Err(e) => Err(e),
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        };

        match parsed {
            Ok(Some(item)) => items.push(item),
            Ok(None) => {}
            Err(e) => {
                if skip_errors {
                    skipped += 1;
                    println!("{YELLOW}[!]{NC} 跳过第 {lineno} 行：{e}");
                } else {
                    return Err(format!("第 {lineno} 行：{e}（可加 --skip-errors 跳过）"));
                }
            }
        }
    }

    if dry_run {
        for (k, v) in &items {
            println!("{k}={v}");
        }
        if json_out {
            let out = json!({
                "ok": true,
                "action": "import",
                "dry_run": true,
                "scope": scope.as_str(),
                "imported": 0,
                "skipped": skipped,
                "preview": items.iter().map(|(k, v)| json!({"key": k, "value": v})).collect::<Vec<_>>(),
            });
            println!("{}", to_json(&out)?);
        } else {
            println!(
                "{YELLOW}[!]{NC} 预览模式，未写入任何数据（共 {} 条）",
                items.len()
            );
        }
        return Ok(());
    }

    let path = env_path(db_path);
    let mut env_file = load(&path)?;
    let ts = now();
    for (k, v) in &items {
        let entry = env_file.conf.entry(k.clone()).or_default();
        entry.value = v.clone();
        entry.updated_at = ts;
    }
    save(&path, &env_file)?;

    if json_out {
        let out = json!({
            "ok": true,
            "action": "import",
            "dry_run": false,
            "scope": scope.as_str(),
            "imported": items.len(),
            "skipped": skipped,
        });
        println!("{}", to_json(&out)?);
    } else {
        ok(&format!(
            "已导入 {} 条到 scope={}（跳过 {} 条）",
            items.len(),
            scope.as_str(),
            skipped
        ));
    }
    Ok(())
}

// ── 工具 ──────────────────────────────────────────────────────

/// 解析一行 `key=value`；`Ok(None)` 表示空行 / 注释。
fn parse_env_line(line: &str) -> Result<Option<(&str, &str)>, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let line = line
        .strip_prefix("export ")
        .map(str::trim_start)
        .unwrap_or(line);
    let Some((k, v)) = line.split_once('=') else {
        return Err(format!("缺少 '='：{line}"));
    };
    let k = k.trim();
    if k.is_empty() {
        return Err(format!("键名为空：{line}"));
    }
    Ok(Some((k, strip_quotes(v.trim()))))
}

/// 去除值两端的成对引号（单 / 双引号）。
fn strip_quotes(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn read_stdin() -> Result<String, String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("读取标准输入失败: {e}"))?;
    Ok(buf.trim_end_matches(['\n', '\r']).to_string())
}

fn to_json(v: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(v).map_err(|e| format!("序列化 JSON 失败: {e}"))
}

/// 按字符截断过长的展示字段。
fn ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn format_time(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}
