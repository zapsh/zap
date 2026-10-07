// SPDX-License-Identifier: AGPL-3.0-only
//! `zapctl config` 子命令：查看 / 新增 / 修改 / 删除 zap.yaml 的键值内容。
//!
//! 配置文件查找顺序与 `zapd/src/config/mod.rs` 保持一致：
//! 1. 环境变量 `ZAP_CONFIG`
//! 2. 生产默认 `/etc/zap/zap.yaml`
//! 3. 开发回退 `conf/zap.yaml`
//!
//! 另提供 `db.path` 只读解析，供其它子命令定位数据库。

use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{NC, YELLOW, ok};

const DEFAULT_DB_PATH: &str = "data/zap.db";

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub db: DbSection,
}

#[derive(Debug, Deserialize)]
pub struct DbSection {
    #[serde(default = "default_db_path")]
    pub path: String,
}

fn default_db_path() -> String {
    DEFAULT_DB_PATH.to_string()
}

impl Default for DbSection {
    fn default() -> Self {
        Self {
            path: default_db_path(),
        }
    }
}

/// 解析配置文件路径（与 zapd 一致）。
pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("ZAP_CONFIG")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    let prod = PathBuf::from("/etc/zap/zap.yaml");
    if prod.exists() {
        return prod;
    }
    PathBuf::from("conf/zap.yaml")
}

/// 计算数据库路径。`override_path`（来自 `--db`）优先级最高。
pub fn db_path(override_path: Option<&str>) -> String {
    if let Some(p) = override_path
        && !p.is_empty()
    {
        return p.to_string();
    }

    let cfg_file = config_path();
    if let Ok(content) = std::fs::read_to_string(&cfg_file)
        && let Ok(cfg) = serde_yaml::from_str::<Config>(&content)
        && !cfg.db.path.is_empty()
    {
        return cfg.db.path;
    }

    DEFAULT_DB_PATH.to_string()
}

// ══ `zapctl config` 键值编辑子命令 ══════════════════════════

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// 查看配置：不指定键打印整个文件；指定键（点分路径）打印该值
    Get {
        /// 键路径（点分，如 server.port；缺省打印全部配置）
        key: Option<String>,
    },
    /// 新增或修改键值（值类型自动识别：true/false → 布尔，纯数字 → 数字，其余 → 字符串）
    Set {
        /// 键路径（点分，如 server.port）
        key: String,
        /// 值
        value: String,
        /// 按 YAML 语法解析 value（支持数组 / 映射等复杂结构）
        #[arg(short, long)]
        yaml: bool,
    },
    /// 删除键（可删除整段，如 jwt）
    Unset {
        /// 键路径（点分，如 server.port）
        key: String,
    },
}

pub fn dispatch(cmd: ConfigCommand, file: Option<&str>) -> Result<(), String> {
    let path = match file {
        Some(f) if !f.trim().is_empty() => PathBuf::from(f),
        _ => config_path(),
    };
    match cmd {
        ConfigCommand::Get { key } => cmd_get(&path, key.as_deref()),
        ConfigCommand::Set { key, value, yaml } => cmd_set(&path, &key, &value, yaml),
        ConfigCommand::Unset { key } => cmd_unset(&path, &key),
    }
}

/// 读取并解析配置文件（根节点需为键值映射时由调用方校验；空文件按空对象处理）
fn read_doc(path: &Path) -> Result<Value, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("无法读取配置文件 {}: {e}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_yaml::from_str(&content).map_err(|e| format!("解析配置文件 {} 失败: {e}", path.display()))
}

fn save_doc(path: &Path, doc: &Value) -> Result<(), String> {
    let out = serde_yaml::to_string(doc).map_err(|e| format!("序列化 YAML 失败: {e}"))?;
    std::fs::write(path, out).map_err(|e| format!("写入配置文件 {} 失败: {e}", path.display()))?;
    Ok(())
}

/// 将点分键路径拆为段（示例：`server.port` → ["server", "port"]）
fn parse_key_path(key: &str) -> Result<Vec<String>, String> {
    let parts: Vec<String> = key
        .split('.')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if parts.is_empty() {
        return Err("键路径不能为空（示例：server.port）".to_string());
    }
    Ok(parts)
}

/// 自动识别 YAML 标量类型：布尔 / 整数 / 浮点 / 字符串
fn infer_scalar(s: &str) -> Value {
    let t = s.trim();
    if t.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if t.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(i) = t.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(f) = t.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(f)
    {
        return Value::Number(n);
    }
    Value::String(t.to_string())
}

fn cmd_get(path: &Path, key: Option<&str>) -> Result<(), String> {
    let doc = read_doc(path)?;
    match key {
        None => {
            let text = serde_yaml::to_string(&doc).map_err(|e| e.to_string())?;
            print!("{}", text.trim_end());
            if !text.trim_end().is_empty() {
                println!();
            }
            Ok(())
        }
        Some(key) => {
            let parts = parse_key_path(key)?;
            let mut cur = &doc;
            for seg in &parts {
                cur = cur
                    .get(seg)
                    .ok_or_else(|| format!("键 {} 不存在", parts.join(".")))?;
            }
            if cur.is_object() || cur.is_array() {
                let text = serde_yaml::to_string(cur).map_err(|e| e.to_string())?;
                print!("{}", text.trim_end());
                println!();
            } else {
                match cur {
                    Value::String(s) => println!("{s}"),
                    Value::Null => println!("(null)"),
                    other => println!("{other}"),
                }
            }
            Ok(())
        }
    }
}

fn cmd_set(path: &Path, key: &str, value: &str, as_yaml: bool) -> Result<(), String> {
    let mut doc = if path.exists() {
        read_doc(path)?
    } else {
        // 文件不存在：按“新建配置”处理（显式执行 set 视为创建意图）
        Value::Object(Map::new())
    };
    if !doc.is_object() {
        return Err("配置文件根节点必须是键值映射（mapping），不支持写入".to_string());
    }

    let val: Value = if as_yaml {
        let yv: serde_yaml::Value =
            serde_yaml::from_str(value).map_err(|e| format!("value 不是合法的 YAML 值: {e}"))?;
        serde_json::to_value(&yv).map_err(|e| format!("value 无法转换为配置值: {e}"))?
    } else {
        infer_scalar(value)
    };

    let parts = parse_key_path(key)?;
    let (last, parents) = parts.split_last().ok_or_else(|| "键路径为空".to_string())?;
    let root = doc.as_object_mut().ok_or("配置根节点非映射")?;
    let mut cur = root;
    for seg in parents {
        if !cur.contains_key(seg) {
            cur.insert(seg.clone(), Value::Object(Map::new()));
        }
        cur = cur
            .get_mut(seg)
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| format!("路径段 {seg} 不是键值映射，无法继续写入"))?;
    }
    let existed = cur.contains_key(last);
    cur.insert(last.clone(), val);

    save_doc(path, &doc)?;
    if existed {
        ok(&format!("已更新 {key}"));
    } else {
        ok(&format!("已新增 {key}"));
    }
    println!("{YELLOW}[!]{NC} 修改需重启 zapd 后生效：zapctl restart zapd");
    Ok(())
}

fn cmd_unset(path: &Path, key: &str) -> Result<(), String> {
    let mut doc = read_doc(path)?;
    if !doc.is_object() {
        return Err("配置文件根节点必须是键值映射（mapping）".to_string());
    }
    let parts = parse_key_path(key)?;
    let (last, parents) = parts.split_last().ok_or_else(|| "键路径为空".to_string())?;
    let root = doc.as_object_mut().ok_or("配置根节点非映射")?;
    let mut cur = root;
    for seg in parents {
        cur = cur
            .get_mut(seg)
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| format!("键 {} 不存在", parts.join(".")))?;
    }
    match cur.remove(last.as_str()) {
        Some(_) => {
            save_doc(path, &doc)?;
            ok(&format!("已删除 {key}"));
            println!("{YELLOW}[!]{NC} 修改需重启 zapd 后生效：zapctl restart zapd");
            Ok(())
        }
        None => Err(format!("键 {} 不存在", parts.join("."))),
    }
}
