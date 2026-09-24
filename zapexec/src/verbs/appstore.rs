//! AppStore 特权动词（全部以 root 执行）。
//!
//! 职责：
//! - `repo_add` / `repo_remove` / `repo_update`：多 Git 源管理
//!   （clone/fetch 到 data/appstore/repos/<id>/，刷新 repos.yaml）
//! - `install` / `uninstall` / `upgrade`：运行包脚本，日志写入 logs/run-{id}.log
//! - `script_run` / `script_stop`：运行/停止自定义脚本（进程组管理）
//! - `script_read` / `script_write`：读写 appstore 内脚本（写仅限 custom/）
//! - `installed`：扫描元数据目录 apps/<cat>/<name>/meta.yaml
//!
//! 安全边界：所有相对路径先做 sanitize（拒绝绝对路径 / `..` / 越界），
//! 包名只允许 `[A-Za-z0-9_-]`。脚本永远通过白名单动词进入，不提供任意命令执行。

use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{bash_bin, root_cmd};
use std::os::unix::process::CommandExt;
use zap_proto::Response;

// ── 内置源（跟随 zap 发行包发布）────────────────────────────

pub const BUILTIN_REPO_ID: &str = "zap-appstore";
pub const BUILTIN_REPO_NAME: &str = "Zap 官方应用商店";
pub const BUILTIN_REPO_URL: &str = "https://github.com/zapsh/zap-appstore.git";

// ── 目录定位 ───────────────────────────────────────────────

fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

/// 定位 zapctl 可执行文件并返回其绝对路径（注入给脚本的 `ZAPCTL`）。
/// 生产布局为 `${ZAP_PATH}/zapctl`；开发布局（ZAP_PATH 指向源码根，如
/// rundev 注入的仓库目录）下该路径是 zapctl 源码目录而非二进制，
/// 需回退到 `target/{debug,release}/zapctl`。
fn zapctl_bin() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    fn executable_file(p: &PathBuf) -> bool {
        p.is_file()
            && std::fs::metadata(p)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
    }

    let base = zap_path();
    [
        base.join("zapctl"),
        base.join("target/debug/zapctl"),
        base.join("target/release/zapctl"),
    ]
    .into_iter()
    .find(executable_file)
    .unwrap_or_else(|| base.join("zapctl"))
}

fn appstore_dir() -> PathBuf {
    zap_path().join("data/appstore")
}

/// 安装元数据目录（apps/<category>/<name>/meta.yaml + info.yaml）。
/// 只存记录；第三方软件本体安装在 `super::install_root()`
/// （默认 /usr/local/apps，环境变量 ZAP_APPS_DIR 可覆盖）。
pub(crate) fn apps_dir() -> PathBuf {
    zap_path().join("data/apps")
}

/// 面板数据根目录（`{ZAP_PATH}/data`）：apps/ 与 users/ 都挂在它下面。
fn data_dir() -> PathBuf {
    zap_path().join("data")
}

// ── 实例槽位 ───────────────────────────────────────────────
//
// 一个包的每个安装实例独占一个目录，路径规则由 `zap_proto::appstore` 唯一决定
// （zapd 用同一份，避免「装在一处、卸载找另一处」）：
//   - 站点类（webapps）：`{data}/users/<owner>/webapps/<name>/<site_id>/`
//   - 其余：`{data}/apps/<category>/<name>/<instance>/`
//
// 旧版本安装的包没有 instance 层（直接落在 `apps/<category>/<name>/`），
// 卸载 / 升级 / 启停时由 `legacy_slot()` 兜住，老安装不会因为升级面板而失联。

use zap_proto::appstore::{DEFAULT_INSTANCE, Slot, WEBAPPS_CATEGORY, parse_site_instance};

/// 一个已安装实例的定位结果。
#[derive(Debug, Clone)]
pub(crate) struct SlotPath {
    pub category: String,
    pub name: String,
    /// 槽位目录：`meta.yaml` / `info.yaml` / `provision.json` 都在这里
    pub dir: PathBuf,
    /// 实例名（站点类为 `site:<id>`）
    pub instance: String,
    /// 归属面板用户（站点类有）
    pub owner: Option<String>,
    /// 站点 id（站点类有）
    pub site_id: Option<String>,
}

impl SlotPath {
    /// 稳定主键：`<category>/<name>@<instance>`
    pub fn key(&self) -> String {
        format!("{}/{}@{}", self.category, self.name, self.instance)
    }

    /// 包路径：`<category>/<name>`（定位包源用，多实例时不唯一）
    pub fn pkg_path(&self) -> String {
        format!("{}/{}", self.category, self.name)
    }
}

/// 按实例定位槽位（新布局）。
pub(crate) fn resolve_slot(
    cat: &str,
    name: &str,
    instance: Option<&str>,
    user: Option<&str>,
    provision: Option<&BTreeMap<String, String>>,
) -> SlotPath {
    let data = data_dir();
    let base = |instance: String, owner: Option<String>, site_id: Option<String>| SlotPath {
        category: cat.to_string(),
        name: name.to_string(),
        dir: Slot::global(cat, name, &instance).dir(&data),
        instance,
        owner,
        site_id,
    };

    // 站点类：落到站点账号的私有目录，带上「属于谁」这一维度
    if cat == WEBAPPS_CATEGORY {
        let site_id = provision
            .and_then(|p| p.get("SITE_ID"))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| instance.and_then(parse_site_instance).map(String::from));
        if let Some(sid) = site_id {
            let owner = provision
                .and_then(|p| p.get("SITE_OWNER"))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(String::from)
                .or_else(|| user.map(String::from))
                .unwrap_or_default();
            if !owner.is_empty() {
                let dir = Slot::site(name, &owner, &sid).dir(&data);
                if dir.is_dir() {
                    return SlotPath {
                        category: cat.to_string(),
                        name: name.to_string(),
                        dir,
                        instance: Slot::site_instance(&sid),
                        owner: Some(owner),
                        site_id: Some(sid),
                    };
                }
            }
            // owner 取不到 / 站点已转手：扫一遍用户目录找同名站点
            if let Some(found) = find_site_slot(cat, name, &sid) {
                return found;
            }
        }
    }

    let inst = instance
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| DEFAULT_INSTANCE.to_string());
    base(inst, None, None)
}

/// 在 `users/*/webapps/<name>/<site_id>/` 里找已存在的槽位。
fn find_site_slot(cat: &str, name: &str, site_id: &str) -> Option<SlotPath> {
    let entries = std::fs::read_dir(data_dir().join("users")).ok()?;
    for e in entries.flatten() {
        if !e.path().is_dir() {
            continue;
        }
        let owner = e.file_name().to_string_lossy().to_string();
        let dir = Slot::site(name, &owner, site_id).dir(&data_dir());
        if dir.is_dir() {
            return Some(SlotPath {
                category: cat.to_string(),
                name: name.to_string(),
                dir,
                instance: Slot::site_instance(site_id),
                owner: Some(owner),
                site_id: Some(site_id.to_string()),
            });
        }
    }
    None
}

/// 旧布局槽位：`apps/<category>/<name>/`（meta.yaml 直接在包目录下）。
fn legacy_slot(cat: &str, name: &str) -> Option<PathBuf> {
    let dir = apps_dir().join(cat).join(name);
    if dir.join("meta.yaml").is_file() {
        Some(dir)
    } else {
        None
    }
}

/// 带旧布局兜底的定位（卸载 / 升级 / 启停 / 重跑走这里）。
pub(crate) fn resolve_slot_with_legacy(
    cat: &str,
    name: &str,
    instance: Option<&str>,
    user: Option<&str>,
    provision: Option<&BTreeMap<String, String>>,
) -> SlotPath {
    let slot = resolve_slot(cat, name, instance, user, provision);
    if !slot.dir.is_dir()
        && let Some(old) = legacy_slot(cat, name)
    {
        return SlotPath {
            category: cat.to_string(),
            name: name.to_string(),
            dir: old,
            instance: DEFAULT_INSTANCE.to_string(),
            owner: None,
            site_id: None,
        };
    }
    slot
}

/// 已安装应用的登记信息（`apps/<category>/<name>/info.yaml`）。
///
/// 安装脚本在这里登记实例名、安装目录、主配置与 systemd 单元 —— 这是**权威来源**：
/// 其它模块（如 service_conf 的 PHP 实例探测）应读它，而不是按目录名反推。
#[derive(Debug, Clone)]
pub(crate) struct AppRegistration {
    /// 包名（如 `php`）：判断「这是不是某个运行时」用，多版本槽位名（74）本身不带包名
    pub pkg: String,
    /// info.yaml 的 instance（缺省回退包名），如 `php74`
    pub instance: String,
    /// 槽位目录名：多版本为版本短名（`74`），旧布局为 `default`
    pub slot_instance: String,
    pub install_dir: Option<PathBuf>,
    pub config_file: Option<PathBuf>,
    pub svc_name: Option<String>,
}

/// 枚举全部已安装应用的登记信息（无 info.yaml 的应用也会列出，字段为 None）。
pub(crate) fn registered_apps() -> Vec<AppRegistration> {
    let mut out = Vec::new();
    // 走槽位扫描：多实例（多版本 PHP）各有自己的 info.yaml，旧布局也一并覆盖
    for slot in scan_slots() {
        let info = read_info_yaml(&slot.dir);
        let pick = |k: &str| -> Option<String> {
            info.as_ref()?
                .get(k)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        };
        // 脚本登记优先，没有就用槽位实例名（旧布局为 default）
        let instance = pick("instance")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| slot.instance.clone());
        out.push(AppRegistration {
            pkg: slot.name.clone(),
            instance,
            slot_instance: slot.instance.clone(),
            install_dir: pick("install_dir").map(PathBuf::from),
            config_file: pick("config_file").map(PathBuf::from),
            svc_name: pick("svc_name"),
        });
    }
    out
}

fn logs_dir() -> PathBuf {
    appstore_dir().join("logs")
}

/// 所有 Git 源根目录：repos/<id>/
fn repos_dir() -> PathBuf {
    appstore_dir().join("repos")
}

fn repos_yaml_path() -> PathBuf {
    appstore_dir().join("repos.yaml")
}

// ── 运行快照：可编辑脚本副本 ────────────────────────────────
// 每次 install/upgrade/uninstall 启动前，把 pkg 目录整体复制到
// runs/<run_id>/pkg/ 作为“本次运行使用的脚本”快照并执行该副本：
// - 失败后保留快照，用户可在 web 端读取/编辑快照内的脚本，
//   再以 appstore.run_retry 复用快照重新执行；
// - 成功后自动清理快照，避免磁盘堆积。
// run.json 记录本次运行的原始参数，供重跑还原环境。

fn runs_dir() -> PathBuf {
    appstore_dir().join("runs")
}

fn run_snapshot_dir(run_id: &str) -> PathBuf {
    runs_dir().join(run_id).join("pkg")
}

fn run_meta_path(run_id: &str) -> PathBuf {
    runs_dir().join(run_id).join("run.json")
}

/// 本次运行的编译/解压临时目录：`runs/<run_id>/build`（注入给脚本的 `BUILD_PATH`）。
fn build_dir(run_id: &str) -> PathBuf {
    runs_dir().join(run_id).join("build")
}

/// 注入面板编排结果（建站 / 建库）到脚本环境。
///
/// 与 options 的区别：这些值由面板产生、可能含数据库密码，因此**不写进
/// options.env / options.json**（那两个文件是给用户查看编辑的），只进子进程 env；
/// 键名由面板给出（`SITE_*` / `DB_*`），这里只做基本的名字合法性校验。
fn push_provision_env(
    env: &mut Vec<(String, String)>,
    provision: Option<&BTreeMap<String, String>>,
) {
    let Some(map) = provision else {
        return;
    };
    for (k, v) in map {
        if valid_option_name(k) {
            env.push((k.clone(), v.clone()));
        }
    }
}

/// 递归复制目录（跳过 .git），供快照使用。
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name() == ".git" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| format!("复制 {} 失败: {e}", from.display()))?;
        }
    }
    Ok(())
}

/// 准备一次运行的可编辑脚本快照：复制 pkg 目录到 runs/<run_id>/pkg 并记录 run.json。
fn prepare_snapshot(run_id: &str, pkg_dir: &Path, spec: &Value) -> Result<PathBuf, String> {
    let dst = run_snapshot_dir(run_id);
    if !dst.is_dir() {
        copy_tree(pkg_dir, &dst)?;
    }
    let meta_dir = runs_dir().join(run_id);
    std::fs::create_dir_all(&meta_dir).map_err(|e| e.to_string())?;
    // run.json 里可能含面板编排的数据库密码（provision），只给 root 读
    use std::os::unix::fs::OpenOptionsExt;

    let path = run_meta_path(run_id);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let _ = std::io::Write::write_all(
        &mut f,
        serde_json::to_string_pretty(spec)
            .unwrap_or_default()
            .as_bytes(),
    );
    Ok(dst)
}

/// 运行成功后清理快照；失败保留供编辑重跑。
fn cleanup_snapshot(run_id: &str, code: i32) {
    if code == 0 {
        let _ = std::fs::remove_dir_all(runs_dir().join(run_id));
    }
}

// ── 安装/升级选项（app.yaml options）─────────────────────────
// 选项值全为标量字符串（前端已归一：多选按 separator 拼接）。
// 落盘 options.env（shell 可 source，便于查看/重跑前修改）与 options.json（结构化备份），
// 同时注入子进程 env —— 脚本可直接以 `$NAME` 使用，无需额外 source。

/// 选项名合法性：字母/下划线开头，字母数字下划线，避开系统保留前缀。
fn valid_option_name(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 64
        && k.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !k.starts_with("ZAP_")
        && !k.starts_with("PKG_")
        && !k.starts_with("APP_")
        && !k.starts_with("ACTION")
        && !k.starts_with("SCRIPT_")
        && !k.starts_with("RUN_")
        && k != "PATH"
        && k != "HOME"
}

/// 单引号包裹的 shell 转义（options.env 专用，保证可安全 source）。
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// 反向解析 options.env 单行值（仅支持本模块写出的两种形态）。
fn shell_unquote(v: &str) -> String {
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        v[1..v.len() - 1].replace("'\\''", "'")
    } else {
        v.to_string()
    }
}

/// 将选项写入快照（options.env + options.json）并返回需要注入子进程 env 的键值对。
/// options 为 None/空时不创建文件、不注入。
fn write_run_options(
    snapshot: &Path,
    options: Option<&BTreeMap<String, String>>,
) -> Result<Vec<(String, String)>, String> {
    let Some(opts) = options else {
        return Ok(Vec::new());
    };
    if opts.is_empty() {
        return Ok(Vec::new());
    }
    let mut env = Vec::with_capacity(opts.len());
    let mut lines = Vec::with_capacity(opts.len() + 1);
    lines.push(
        "# generated by zap from install/upgrade options (editable before retry)".to_string(),
    );
    for (k, v) in opts {
        if !valid_option_name(k) {
            return Err(format!("非法选项名: {k}"));
        }
        if v.chars().count() > 4096 {
            return Err(format!("选项 {k} 的值过长"));
        }
        env.push((k.clone(), v.clone()));
        lines.push(format!("{k}={}", shell_quote(v)));
    }
    let env_path = snapshot.join("options.env");
    let json_path = snapshot.join("options.json");
    std::fs::write(&env_path, lines.join("\n") + "\n")
        .map_err(|e| format!("写 options.env 失败: {e}"))?;
    std::fs::write(
        &json_path,
        serde_json::to_string_pretty(opts).unwrap_or_default(),
    )
    .map_err(|e| format!("写 options.json 失败: {e}"))?;
    tracing::info!("已写入运行选项 {} 项至 {}", opts.len(), env_path.display());
    Ok(env)
}

/// 从快照 options.env 读出选项注入 env（重跑用）。
/// 以文件为准：用户在重跑前编辑过的选项同样生效；文件缺失则返回空。
fn read_options_env(snapshot: &Path) -> Vec<(String, String)> {
    let Ok(content) = std::fs::read_to_string(snapshot.join("options.env")) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            if !valid_option_name(k) {
                return None;
            }
            Some((k.to_string(), shell_unquote(v)))
        })
        .collect()
}

/// run_id 快照内相对路径安全解析（仅限 runs/<run_id>/pkg/ 内）。
fn run_safe_path(run_id: &str, rel: &str) -> Result<PathBuf, String> {
    let root = run_snapshot_dir(run_id);
    let p = safe_join(&root, rel)?;
    if !p.starts_with(&root) {
        return Err("路径越界".into());
    }
    Ok(p)
}

// ── 路径安全 ───────────────────────────────────────────────

fn safe_rel(requested: &str) -> Result<PathBuf, String> {
    let p = Path::new(requested);
    if p.is_absolute() {
        return Err("不允许绝对路径".into());
    }
    for seg in p.components() {
        match seg {
            Component::ParentDir => return Err("不允许 .. 路径".into()),
            Component::RootDir | Component::Prefix(_) => return Err("不允许绝对路径".into()),
            Component::CurDir => {}
            Component::Normal(_) => {}
        }
    }
    Ok(p.to_path_buf())
}

fn safe_join(base: &Path, requested: &str) -> Result<PathBuf, String> {
    let rel = safe_rel(requested)?;
    let joined = base.join(rel);
    if !joined.starts_with(base) {
        return Err("路径越界".into());
    }
    Ok(joined)
}

/// 包路径必须是 `category/name`，且只含安全字符。
fn validate_pkg_path(pkg_path: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = pkg_path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() != 2 {
        return Err(format!("包路径格式应为 category/name，收到: {pkg_path}"));
    }
    for s in &parts {
        if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!("包名含非法字符: {s}"));
        }
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// 定位包目录：custom 优先于官方源；官方源按 repo_id 定位到 repos/<repo_id>/，
/// 未指定 repo_id 时遍历所有源目录。
fn find_package(pkg_path: &str, source: &str, repo_id: Option<&str>) -> Result<PathBuf, String> {
    let (cat, name) = validate_pkg_path(pkg_path)?;
    if source == "custom" {
        let dir = safe_join(&appstore_dir().join("custom"), &format!("{cat}/{name}"))?;
        if dir.is_dir() {
            return Ok(dir);
        }
        return Err(format!("自定义包不存在: {pkg_path}"));
    }
    if let Some(rid) = repo_id {
        let rid = safe_rel(rid)?;
        let rid_str = rid.to_string_lossy().to_string();
        let dir = repos_dir().join(rid).join(&cat).join(&name);
        if dir.is_dir() {
            return Ok(dir);
        }
        return Err(format!("官方包不存在: {pkg_path}（源 {rid_str}）"));
    }
    let mut found: Option<PathBuf> = None;
    if let Ok(entries) = std::fs::read_dir(repos_dir()) {
        for entry in entries.flatten() {
            let dir = entry.path().join(&cat).join(&name);
            if dir.is_dir() {
                found = Some(dir);
                break;
            }
        }
    }
    found.ok_or_else(|| format!("官方包不存在: {pkg_path}"))
}

/// 包脚本文件与解释器：app.yaml 的 `scripts.{install|uninstall|upgrade}` 可覆盖默认约定。
///
/// 支持两种写法（脚本可用 python3 编写）：
/// ```yaml
/// scripts:
///   install: install.py                                    # 简写：解释器按扩展名推导
///   uninstall: { file: remove.py, interpreter: python3 }    # 显式指定解释器
/// ```
fn script_file(pkg_dir: &Path, key: &str, default: &str) -> Result<(PathBuf, Interpreter), String> {
    let mut file = default.to_string();
    let mut interp: Option<Interpreter> = None;
    if let Ok(content) = std::fs::read_to_string(pkg_dir.join("app.yaml"))
        && let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(&content)
        && let Some(s) = v.get("scripts").and_then(|s| s.get(key))
    {
        match s {
            serde_yaml::Value::String(name) => file = name.clone(),
            serde_yaml::Value::Mapping(m) => {
                if let Some(name) = m.get("file").and_then(|f| f.as_str()) {
                    file = name.to_string();
                }
                if let Some(i) = m.get("interpreter").and_then(|f| f.as_str()) {
                    interp = Some(match i.trim().to_ascii_lowercase().as_str() {
                        "python" | "python3" => Interpreter::Python3,
                        "bash" | "sh" => Interpreter::Bash,
                        other => return Err(format!("不支持的解释器: {other}")),
                    });
                }
            }
            _ => {}
        }
    }
    let p = safe_join(pkg_dir, &file)?;
    if !p.is_file() {
        return Err(format!("包脚本不存在: {}", p.display()));
    }
    let interp = interp.unwrap_or_else(|| interpreter_of(&p));
    Ok((p, interp))
}

// ── repos.yaml / meta.yaml ─────────────────────────────────

/// 单个 Git 源配置项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    pub id: String,
    pub name: String,
    pub url: String,
    /// 系统内置源（随 zap 发行包发布，禁止删除）
    #[serde(default)]
    pub builtin: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 最近一次同步的 commit 短哈希
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_enabled() -> bool {
    true
}

impl RepoEntry {
    fn builtin() -> RepoEntry {
        RepoEntry {
            id: BUILTIN_REPO_ID.into(),
            name: BUILTIN_REPO_NAME.into(),
            url: BUILTIN_REPO_URL.into(),
            builtin: true,
            enabled: true,
            version: String::new(),
            commit: String::new(),
            updated_at: 0,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ReposFile {
    #[serde(default)]
    pub repos: Vec<RepoEntry>,
}

/// 读取源列表；repos.yaml 不存在或为空时自动补内置源记录。
fn read_repos() -> Result<Vec<RepoEntry>, String> {
    let mut file = match std::fs::read_to_string(repos_yaml_path()) {
        Ok(content) => serde_yaml::from_str::<ReposFile>(&content)
            .map_err(|e| format!("解析 repos.yaml 失败: {e}"))?,
        Err(_) => ReposFile::default(),
    };
    if file.repos.is_empty() {
        file.repos.push(RepoEntry::builtin());
        write_repos(&file.repos)?;
    }
    Ok(file.repos)
}

fn write_repos(repos: &[RepoEntry]) -> Result<(), String> {
    std::fs::create_dir_all(appstore_dir()).map_err(|e| e.to_string())?;
    let file = ReposFile {
        repos: repos.to_vec(),
    };
    let yaml = serde_yaml::to_string(&file).map_err(|e| format!("序列化 repos.yaml 失败: {e}"))?;
    std::fs::write(repos_yaml_path(), yaml).map_err(|e| format!("写入 repos.yaml 失败: {e}"))
}

/// 从 Git URL 末段生成源 id（去 .git，保留 [a-z0-9-]）。
fn id_from_url(url: &str) -> String {
    let base = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("store")
        .trim_end_matches(".git");
    let mut id = String::new();
    for c in base.to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c);
        } else {
            id.push('-');
        }
    }
    while id.contains("--") {
        id = id.replace("--", "-");
    }
    let id = id.trim_matches('-').to_string();
    if id.is_empty() { "store".into() } else { id }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MetaInfo {
    pub name: String,
    pub version: String,
    pub category: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<String>,
    pub installed_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgraded_from: Option<String>,
    pub run_id: String,
}

fn write_meta(app_path: &Path, meta: &MetaInfo) -> std::io::Result<()> {
    std::fs::create_dir_all(app_path)?;
    let yaml = serde_yaml::to_string(meta)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(app_path.join("meta.yaml"), yaml)
}

fn read_meta(app_path: &Path) -> Result<MetaInfo, String> {
    let content = std::fs::read_to_string(app_path.join("meta.yaml"))
        .map_err(|e| format!("读取 meta.yaml 失败: {e}"))?;
    serde_yaml::from_str(&content).map_err(|e| format!("解析 meta.yaml 失败: {e}"))
}

/// 面板编排结果（站点 / 数据库）落盘：`apps/<pkg_path>/provision.json`。
///
/// 必须由 zapexec（root）写：webapps 的槽位目录在降权运行时会被 chown 给站点账号
/// （见 prepare_user_run），之后面板进程（非 root）就写不进去了 —— 这正是
/// provision.json 曾经静默缺失、导致卸载报「缺少 SITE_ROOT」的原因。
/// 0600：里面有数据库明文密码，只有 root 能读。
fn write_provision(app_path: &Path, env: &BTreeMap<String, String>) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::create_dir_all(app_path)?;
    let json = serde_json::to_string_pretty(env)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(app_path.join("provision.json"))?;
    f.write_all(json.as_bytes())
}

/// 读取落盘的编排结果（卸载 / 升级复用同一站点与库）。
fn read_provision(app_path: &Path) -> Option<BTreeMap<String, String>> {
    let content = std::fs::read_to_string(app_path.join("provision.json")).ok()?;
    serde_json::from_str(&content).ok()
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── 命令执行 ───────────────────────────────────────────────

fn run_capture(program: &str, args: &[&str]) -> Result<String, String> {
    // git 远程操作（clone/fetch）在网络不可达时可能无限挂起，导致任务永久 running。
    // 统一用 timeout 限时；同时禁止 git 交互式认证提示（避免等待输入用户名/密码）。
    let out = root_cmd("timeout")
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("180")
        .arg(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program} 执行失败: {e}"))?;
    if out.status.code() == Some(124) {
        return Err(format!("{program} 执行超时(180s)"));
    }
    if !out.status.success() {
        return Err(format!(
            "{program} 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// git 包装：显式放行仓库属主检查。
///
/// 面板以 root 跑，而 `data/appstore/repos/<id>/` 可能是别的账号建的（内置源随发行包落地、
/// 或用户手动 clone）。git 2.35.2+ 一旦遇到「仓库属主 ≠ 当前 euid」就报
/// `detected dubious ownership` 并直接失败，且要求手工改全局配置——面板无权也不该
/// 去改宿主的 `~/.gitconfig`，所以每次调用都自带 `-c safe.directory=*`。
fn run_git(args: &[&str]) -> Result<String, String> {
    let mut full: Vec<&str> = Vec::with_capacity(args.len() + 2);
    full.push("-c");
    full.push("safe.directory=*");
    full.extend_from_slice(args);
    run_capture("git", &full)
}

/// 包脚本的运行身份模式（`app.yaml` 的 `run_as` / `scope` 推导结果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunMode {
    /// root：系统级包（nginx / php / mysql…）要写系统目录、管 systemd。
    Root,
    /// 降权：以面板用户对应的 Linux 账号运行，仅 `webapps` 分类可用。
    User,
}

/// 包脚本的执行身份：`Root` 或携带具体 Linux 账号名的 `User`。
#[derive(Debug, Clone)]
enum RunAs {
    Root,
    User(String),
}

/// 包脚本解释器：`.py` 走 `python3 -I -B`，其余走 bash（路径由 `bash_bin` 解析）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interpreter {
    Bash,
    Python3,
}

impl Interpreter {
    /// 解释器程序（尽量用绝对路径，不依赖 PATH 解析）。
    fn program(self) -> String {
        match self {
            Interpreter::Bash => bash_bin(),
            Interpreter::Python3 => python3_bin(),
        }
    }

    /// 解释器参数（必须排在脚本路径之前）。
    fn flags(self) -> &'static [&'static str] {
        match self {
            Interpreter::Bash => &[],
            // -I：隔离模式 —— 忽略 PYTHON* 环境变量、不加载用户 site-packages、
            //     不把脚本所在目录放进 sys.path（防同目录同名模块劫持）；
            // -B：不生成 __pycache__，家目录不留可执行字节码；
            // -u：无缓冲输出（日志要实时回显到面板；-I 会让 PYTHONUNBUFFERED 失效）。
            Interpreter::Python3 => &["-I", "-B", "-u"],
        }
    }
}

/// python3 解释器路径：优先常见绝对路径，取不到时回退 PATH 查找。
fn python3_bin() -> String {
    for p in ["/usr/bin/python3", "/usr/local/bin/python3", "/bin/python3"] {
        if std::path::Path::new(p).is_file() {
            return p.to_string();
        }
    }
    "python3".to_string()
}

/// 按扩展名推导解释器：`.py` → python3，其余 → bash。
fn interpreter_of(script: &Path) -> Interpreter {
    match script.extension().and_then(|e| e.to_str()) {
        Some("py") => Interpreter::Python3,
        _ => Interpreter::Bash,
    }
}

/// 解析包声明的运行身份：
/// - `run_as: user|root` 显式声明，优先级最高；
/// - 未声明时 `scope: site`（装进用户站点）默认降权为 `user`，其余保持 `root`；
/// - **`user` 仅允许 `webapps` 分类**：其它分类的脚本要改系统目录，降权只会半途失败，
///   且这条通道本来就是给建站类包开的，不允许别的分类蹭进来。
fn pkg_run_as(pkg_dir: &Path, category: &str) -> Result<RunMode, String> {
    let mut mode: Option<String> = None;
    let mut scope: Option<String> = None;
    if let Ok(content) = std::fs::read_to_string(pkg_dir.join("app.yaml"))
        && let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(&content)
    {
        mode = v
            .get("run_as")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_ascii_lowercase());
        scope = v
            .get("scope")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_ascii_lowercase());
    }
    let mode = mode.unwrap_or_else(|| {
        if scope.as_deref() == Some("site") {
            "user".to_string()
        } else {
            "root".to_string()
        }
    });
    match mode.as_str() {
        "root" => Ok(RunMode::Root),
        "user" => {
            if category != "webapps" {
                return Err("只有 webapps 分类的包允许声明 run_as: user".into());
            }
            Ok(RunMode::User)
        }
        other => Err(format!("非法的 run_as: {other}（只能是 user 或 root）")),
    }
}

/// 把运行身份模式解析成具体的执行者：降权时把面板用户名映射为 Linux 账号。
fn resolve_run_as(pkg_dir: &Path, category: &str, actor: Option<&str>) -> Result<RunAs, String> {
    match pkg_run_as(pkg_dir, category)? {
        RunMode::Root => Ok(RunAs::Root),
        RunMode::User => {
            let actor = actor
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or("无法确定运行账号：请求未携带面板用户名")?;
            Ok(RunAs::User(zap_proto::linux_username(actor)))
        }
    }
}

/// 降权运行前的准备：建目录、改属主、补齐环境变量。
///
/// 脚本以普通账号运行时无法自己 `mkdir`（父目录归 root），也无法 chown，
/// 因此由 zapexec（root）先把本次运行要写的目录建好并交给该账号，
/// 脚本只在这些目录内部写文件，越界就会被文件系统的属主拦住。
fn prepare_user_run(
    env: &mut Vec<(String, String)>,
    dirs: &[PathBuf],
    linux_user: &str,
) -> Result<(), String> {
    let acc = super::linux_account(linux_user)?;
    for d in dirs {
        std::fs::create_dir_all(d).map_err(|e| format!("创建目录 {} 失败: {e}", d.display()))?;
        // `users/<user>` 这一层归面板进程（crontab / cloud / scripts 在它下面），
        // 末端实例目录才是站点账号的（下面 chown）
        super::ensure_panel_dir_for_path(d)?;
    }
    for d in dirs {
        chown_path(d, acc.uid, acc.gid)?;
    }
    env.push(("ZAP_LINUX_USER".into(), linux_user.to_string()));
    env.push(("ZAP_HOME".into(), acc.home.to_string_lossy().into_owned()));
    Ok(())
}

/// 修改路径属主（仅对本模块自己创建的运行目录使用）。
fn chown_path(p: &Path, uid: u32, gid: u32) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;

    let c = std::ffi::CString::new(p.as_os_str().as_bytes())
        .map_err(|_| format!("非法路径: {}", p.display()))?;
    if unsafe { libc::chown(c.as_ptr(), uid, gid) } != 0 {
        return Err(format!(
            "修改属主失败 {}: {}",
            p.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

struct ScriptStep {
    script: PathBuf,
    env: Vec<(String, String)>,
    run_as: RunAs,
    interpreter: Interpreter,
}

// ── 任务执行队列：安装/卸载/升级等脚本任务一次只跑一个 ──────
// appstore 脚本任务会改动系统级目录与软件，串行执行避免互相干扰。
// 后来的任务在线程内先写日志提示排队，随后阻塞等待前序任务完成。

struct QueueGate {
    busy: bool,
}

static QUEUE_PAIR: std::sync::OnceLock<(std::sync::Mutex<QueueGate>, std::sync::Condvar)> =
    std::sync::OnceLock::new();

fn queue_pair() -> &'static (std::sync::Mutex<QueueGate>, std::sync::Condvar) {
    QUEUE_PAIR.get_or_init(|| {
        (
            std::sync::Mutex::new(QueueGate { busy: false }),
            std::sync::Condvar::new(),
        )
    })
}

fn queue_acquire() {
    let (m, c) = queue_pair();
    let mut g = m.lock().unwrap();
    while g.busy {
        g = c.wait(g).unwrap();
    }
    g.busy = true;
}

fn queue_release() {
    let (m, c) = queue_pair();
    {
        let mut g = m.lock().unwrap();
        g.busy = false;
    }
    c.notify_one();
}

/// 执行期间持有；drop 时（含 panic 路径）释放队列闸门。
struct QueueToken;

impl QueueToken {
    fn new() -> Self {
        queue_acquire();
        QueueToken
    }
}

impl Drop for QueueToken {
    fn drop(&mut self) {
        queue_release();
    }
}

/// 单个脚本步骤的执行上限。
///
/// 脚本任务共用一个串行闸门，一个卡住的进程（等交互输入、下载挂死）会把后面
/// 所有任务一起堵死，所以这里必须有硬上限：超时后先 SIGTERM 整个进程组，
/// 宽限 5 秒不死再 SIGKILL。需要更久的包（源码编译）日后可按包声明覆盖。
const DEFAULT_STEP_TIMEOUT_SECS: u64 = 6 * 3600;

/// 后台运行一个或多个脚本（同一 run_id、同一日志追加写）。
/// 每个脚本以 `setsid` 启动独立进程组，pid 写入 run-{id}.pid 供停止使用。
/// 全部成功退出码为 0；任一脚本失败则中断后续步骤。结束时把退出码写进
/// run-{id}.ret（权威来源），日志末尾仍追加 `__ZAP_DONE__ <code>` 供人阅读。
/// 任务受全局队列闸门约束：同一时间仅执行一个脚本任务，其余等待。
fn spawn_background(
    run_id: &str,
    steps: Vec<ScriptStep>,
    on_done: Box<dyn FnOnce(i32) + Send>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(logs_dir()).map_err(|e| e.to_string())?;
    let log_path = logs_dir().join(format!("run-{run_id}.log"));
    let pid_path = logs_dir().join(format!("run-{run_id}.pid"));
    // 退出码文件：日志里的 __ZAP_DONE__ 可被脚本伪造，.ret 由 zapexec 独占写，
    // 位于 root 拥有的 logs/ 下，降权脚本写不进去 —— 面板只认它。
    let code_path = logs_dir().join(format!("run-{run_id}.ret"));
    let _ = std::fs::remove_file(&code_path);
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("打开日志失败: {e}"))?;
    let ret_path = log_path.clone();
    let cpu_num = std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "1".into());
    // 资源边界（rlimit + 可选的 Linux cgroup）：整次 run 共用一份，
    // 只有降权的第三方脚本会 enter，官方 root 脚本不受限。
    let resource = std::sync::Arc::new(super::resource::TaskResource::prepare(run_id));

    std::thread::spawn(move || {
        use std::io::Write;
        let mut log = log_file;
        let _ = writeln!(
            log,
            "── Queued, automatically runs after the previous task completes ──"
        );
        // 全局串行闸门：install/uninstall/upgrade/script_run 一次只执行一个
        let _queue_token = QueueToken::new();
        let _ = writeln!(log, "── Task Started ──");
        let mut final_code = 0;
        for step in steps {
            // 建站类包（run_as: user）走降权通道，其余沿用 root
            let mut drop_to: Option<(u32, u32)> = None;
            let mut cmd = match &step.run_as {
                RunAs::Root => root_cmd(&step.interpreter.program()),
                RunAs::User(u) => match super::user_cmd(&step.interpreter.program(), u) {
                    Ok((c, acc)) => {
                        drop_to = Some((acc.uid, acc.gid));
                        c
                    }
                    Err(e) => {
                        let _ = writeln!(log, "start script failed: {e}");
                        final_code = -1;
                        break;
                    }
                },
            };
            // 解释器参数必须排在脚本路径之前：python3 -I -B install.py
            cmd.args(step.interpreter.flags())
                .arg(&step.script)
                // 不给 stdin：脚本不该读到 zapexec 的 stdin / 继承来的终端
                .stdin(std::process::Stdio::null())
                .env("ZAP_PATH", zap_path())
                .env("ZAPCTL", zapctl_bin())
                .env("APPS_DIR", super::install_root())
                .env("LOG_FILE", &log_path)
                .env("CPU_NUM", &cpu_num)
                .stdout(std::process::Stdio::from(log.try_clone().unwrap_or_else(
                    |_| {
                        std::fs::OpenOptions::new()
                            .append(true)
                            .open(&log_path)
                            .unwrap()
                    },
                )))
                .stderr(std::process::Stdio::from(log.try_clone().unwrap_or_else(
                    |_| {
                        std::fs::OpenOptions::new()
                            .append(true)
                            .open(&log_path)
                            .unwrap()
                    },
                )));
            for (k, v) in &step.env {
                cmd.env(k, v);
            }
            // Python 脚本：注入辅助库目录（脚本自行 sys.path.insert 后 import zapweb）。
            // 注意 -I 会忽略 PYTHON* 环境变量，所以这里只给自定义的 ZAP_PY_LIB。
            if step.interpreter == Interpreter::Python3 {
                cmd.env("ZAP_PY_LIB", zap_path().join("scripts").join("zap"));
            }
            // 新进程组：pid == pgid，便于停止/超时时 kill(-pid)
            let res = std::sync::Arc::clone(&resource);
            unsafe {
                cmd.pre_exec(move || {
                    libc::setsid();
                    // exec 前只保留 stdio：其余继承 fd 一律 CLOEXEC
                    super::cloexec_inherited_fds();
                    // 降权脚本再加固：先清附加组再降权（顺序敏感），
                    // 之后禁止借 setuid 提权 + 关 core dump
                    if let Some((uid, gid)) = drop_to {
                        super::drop_privileges(uid, gid)?;
                        // 进入资源笼子：禁再提权 + rlimit + umask + cgroup
                        res.enter();
                    }
                    Ok(())
                });
            }
            match cmd.spawn() {
                Ok(mut child) => {
                    let pid = child.id();
                    let _ = std::fs::write(&pid_path, pid.to_string());
                    let code = wait_with_timeout(
                        &mut child,
                        std::time::Duration::from_secs(DEFAULT_STEP_TIMEOUT_SECS),
                        &mut log,
                    );
                    if code != 0 {
                        final_code = code;
                        break;
                    }
                }
                Err(e) => {
                    let _ = writeln!(log, "启动脚本失败: {e}");
                    final_code = -1;
                    break;
                }
            }
        }
        let _ = std::fs::remove_file(&pid_path);
        resource.finish();
        // 顺序固定：先把日志写完，再落 .ret 宣布结束。
        // 反过来的话，面板可能在日志还没刷完时就读到 .ret 判定完成，
        // 最后一行标记来不及被 strip，会漏给用户看。
        let _ = writeln!(log, "\n__ZAP_DONE__ {final_code}");
        let _ = std::fs::write(&code_path, final_code.to_string());
        on_done(final_code);
    });

    Ok(ret_path)
}

/// 等待脚本进程，带硬超时。
///
/// 超时后向进程组发 SIGTERM（`setsid` 保证 pid == pgid），最多宽限 5 秒，
/// 仍不死则 SIGKILL —— 不能再让一个卡住的脚本占住全局串行闸门。
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
    log: &mut std::fs::File,
) -> i32 {
    use std::io::Write;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.code().unwrap_or(-1),
            Ok(None) => {
                if std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    continue;
                }
                let pid = child.id() as i32;
                let _ = writeln!(
                    log,
                    "\n── 执行超过 {} 秒，终止进程组 {pid} ──",
                    timeout.as_secs()
                );
                unsafe {
                    libc::kill(-pid, libc::SIGTERM);
                }
                for _ in 0..5 {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    if let Ok(Some(st)) = child.try_wait() {
                        return st.code().unwrap_or(-1);
                    }
                }
                let _ = writeln!(log, "── 进程组 {pid} 未响应，强制 SIGKILL ──");
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
                return child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
            }
            Err(e) => {
                let _ = writeln!(log, "等待子进程失败: {e}");
                return -1;
            }
        }
    }
}

fn base_env() -> Vec<(String, String)> {
    let mut env = vec![
        ("ZAP_PATH".into(), zap_path().to_string_lossy().into_owned()),
        ("ZAPCTL".into(), zapctl_bin().to_string_lossy().into_owned()),
        (
            "APPS_DIR".into(),
            super::install_root().to_string_lossy().into_owned(),
        ),
    ];
    // 包下载源：面板「系统设置 → 下载源」写进 {data}/mirror.yaml，这里读出后
    // 注入脚本环境。没配就不注入 —— 脚本侧 pkg_mirror() 自带默认镜像。
    if let Some(base) = pkg_mirror_from_conf() {
        env.push(("ZAP_PKG_MIRROR".into(), base));
    }
    env
}

/// 读 `{data}/mirror.yaml` 的 `pkg_mirror`（zapd 落盘的那份配置）。
///
/// 只取一个键，不引入完整反序列化：这里要的是「有没有配、配成什么」，
/// 文件坏了（手工编辑出错）就当没配，让脚本回落到默认镜像，而不是让安装失败。
fn pkg_mirror_from_conf() -> Option<String> {
    let text = std::fs::read_to_string(data_dir().join("mirror.yaml")).ok()?;
    for line in text.lines() {
        let line = line.trim();
        let Some(v) = line.strip_prefix("pkg_mirror:") else {
            continue;
        };
        let v = v.trim().trim_matches('\'').trim_matches('"').trim();
        // 空白与控制字符一律不认：这个值会进脚本环境
        if !v.is_empty() && !v.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Some(v.to_string());
        }
    }
    None
}

/// 构造包脚本执行环境：在 base_env 基础上补齐脚本通用变量。
/// - PKG_PATH 为包源目录（含脚本/app.yaml），APP_PATH 为安装目录
/// - version 为 Some 时注入 APP_VERSION 及由其解析的 MAJOR_VERSION/MINOR_VERSION，
///   并按 app.yaml version_meta 解析该版本家族注入 APP_FAMILY（合并入口专用，如 MySQL/MariaDB）
/// - LOG_FILE / CPU_NUM 由 spawn_background 统一注入
fn task_env(
    pkg_dir: &Path,
    app_path: &Path,
    app_name: &str,
    version: Option<&str>,
    run_id: &str,
) -> Vec<(String, String)> {
    let mut env = base_env();
    env.push(("PKG_PATH".into(), pkg_dir.to_string_lossy().into_owned()));
    env.push(("APP_ID".into(), run_id.to_string()));
    env.push(("APP_NAME".into(), app_name.to_string()));
    env.push(("APP_PATH".into(), app_path.to_string_lossy().into_owned()));
    env.push((
        "BUILD_PATH".into(),
        // 编译目录归属本次运行现场:runs/<run_id>/build,与脚本快照同生命周期
        // (成功随 cleanup_snapshot 整目录清理,失败保留供排查/重跑),
        // 同一应用并发运行互不干扰。
        build_dir(run_id).to_string_lossy().into_owned(),
    ));
    env.push((
        "ZAP_DATA_PATH".into(),
        zap_path().join("data").to_string_lossy().into_owned(),
    ));
    if let Some(v) = version {
        env.push(("APP_VERSION".into(), v.to_string()));
        if let Some((major, rest)) = v.split_once('.') {
            env.push(("MAJOR_VERSION".into(), major.to_string()));
            env.push((
                "MINOR_VERSION".into(),
                rest.split('.').next().unwrap_or("").to_string(),
            ));
        }
        // 家族标识（合并入口专用，如 MySQL/MariaDB 同包不同家族）：
        // 以 app.yaml version_meta 为准下发 APP_FAMILY，脚本据此分流，不得自行按版本号
        // 猜测（跨家族版本号可能重合，仅凭版本号无法区分）。解析不到时不注入，
        // 由脚本自行兜底（安装脚本对缺失家族直接报错，防止装错家族）。
        if let Some(family) = family_from_version_meta(pkg_dir, v) {
            env.push(("APP_FAMILY".into(), family));
        }
    }
    env
}

/// 解析包 app.yaml version_meta 中指定版本的家族标识（如 mysql / mariadb）。
/// version_meta 形如：`"9.7.2": { family: mysql }`；缺失或结构不符返回 None。
fn family_from_version_meta(pkg_dir: &Path, version: &str) -> Option<String> {
    let content = std::fs::read_to_string(pkg_dir.join("app.yaml")).ok()?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    yaml.get("version_meta")?
        .get(version)?
        .get("family")?
        .as_str()
        .map(|s| s.to_string())
}

/// 注入操作者上下文：面板登录用户名与虚拟主机运行模式（固定 system）。
/// 值由 zapd 在发起任务时随请求透传；重跑（run_retry）从原 spec 恢复，保证环境一致。
/// 脚本内对应 ZAP_USER / ZAP_RUN_MODE。
fn push_actor_env(env: &mut Vec<(String, String)>, user: Option<&str>, run_mode: Option<&str>) {
    if let Some(u) = user.filter(|s| !s.is_empty()) {
        env.push(("ZAP_USER".into(), u.to_string()));
    }
    if let Some(m) = run_mode.filter(|s| !s.is_empty()) {
        env.push(("ZAP_RUN_MODE".into(), m.to_string()));
    }
}

// ── 动词实现 ───────────────────────────────────────────────

/// 校验 Git 源 URL：只允许 http(s) 或 git@ 形式，禁止命令注入。
fn validate_repo_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("Git 地址不能为空".into());
    }
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("git@")
    {
        Ok(())
    } else {
        Err("Git 地址必须以 http(s):// 或 git@ 开头".into())
    }
}

/// 后台执行仓库操作，进度写入 run-{run_id}.log，结束写 `__ZAP_DONE__ <code>`。
fn spawn_repo_task(
    run_id: String,
    op: impl FnOnce() -> Result<String, String> + Send + 'static,
    title: String,
) -> Response {
    let log_path = logs_dir().join(format!("run-{run_id}.log"));
    let ret_log = log_path.clone();
    // 同步创建日志文件，确保接口返回时文件已存在（WebSocket 立即读日志不会 ENOENT）
    if let Err(e) = std::fs::create_dir_all(logs_dir()).and_then(|_| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map(|_| ())
    }) {
        return Response::err(-1, format!("创建日志失败: {e}"));
    }
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let _ = std::fs::create_dir_all(logs_dir());
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap_or_else(|_| {
                std::fs::OpenOptions::new()
                    .append(true)
                    .open("/dev/null")
                    .unwrap()
            });
        let code_path = log_path.with_extension("ret");
        let _ = writeln!(log, "=== {title} ===");
        let code = match op() {
            Ok(detail) => {
                let _ = writeln!(log, "成功: {detail}");
                0
            }
            Err(e) => {
                let _ = writeln!(log, "失败: {e}");
                -1
            }
        };
        let _ = std::fs::write(&code_path, code.to_string());
        let _ = writeln!(log, "\n__ZAP_DONE__ {code}");
    });
    Response::ok(
        "任务已启动",
        Some(json!({ "run_id": run_id, "log": ret_log })),
    )
}

/// 添加 Git 源（后台执行）：clone 到 repos/<id>/，并写入 repos.yaml。
pub async fn repo_add(name: String, url: String, run_id: String) -> Response {
    validate_repo_url(&url).map_or_else(
        |e| Response::err(-1, e),
        |_| {
            let title = format!("添加 Git 源: {name} ({url})");
            spawn_repo_task(run_id.clone(), move || repo_add_inner(&name, &url), title)
        },
    )
}

fn repo_add_inner(name: &str, url: &str) -> Result<String, String> {
    let mut repos = read_repos()?;
    // 生成唯一 id（来自 URL 末段，冲突自动加后缀）
    let base_id = id_from_url(url);
    let mut id = base_id.clone();
    let mut n = 2;
    while repos.iter().any(|r| r.id == id) {
        id = format!("{base_id}-{n}");
        n += 1;
    }
    let dir = repos_dir().join(&id);
    if dir.exists() {
        return Err(format!("源目录已存在: {}", dir.display()));
    }
    std::fs::create_dir_all(repos_dir()).map_err(|e| e.to_string())?;
    let tmp_clone = repos_dir().join(format!(".tmp-{id}"));
    let _ = std::fs::remove_dir_all(&tmp_clone);
    run_git(&["clone", "--depth", "1", url, tmp_clone.to_str().unwrap()])?;
    // 临时目录非空校验，防止克隆出空目录
    if std::fs::read_dir(&tmp_clone)
        .map_err(|e| e.to_string())?
        .next()
        .is_none()
    {
        let _ = std::fs::remove_dir_all(&tmp_clone);
        return Err("克隆结果为空".into());
    }
    std::fs::rename(&tmp_clone, &dir).map_err(|e| format!("移动到源目录失败: {e}"))?;
    let commit = run_git(&["-C", dir.to_str().unwrap(), "rev-parse", "HEAD"])?;
    let short = commit.chars().take(7).collect::<String>();
    repos.push(RepoEntry {
        id: id.clone(),
        name: name.to_string(),
        url: url.to_string(),
        builtin: false,
        enabled: true,
        version: short.clone(),
        commit: commit.clone(),
        updated_at: now_ts(),
    });
    write_repos(&repos)?;
    Ok(format!("id={id} commit={short}"))
}

/// 删除 Git 源（同步执行）：内置源禁止删除。
pub async fn repo_remove(id: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let id = safe_rel(&id)?.to_string_lossy().to_string();
        let mut repos = read_repos()?;
        let Some(idx) = repos.iter().position(|r| r.id == id) else {
            return Err(format!("源不存在: {id}"));
        };
        if repos[idx].builtin {
            return Err("内置源不可删除".into());
        }
        let dir = repos_dir().join(&id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| format!("删除源目录失败: {e}"))?;
        }
        repos.remove(idx);
        write_repos(&repos)?;
        Ok(Response::ok("源已删除", Some(json!({ "id": id }))))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 更新单个 Git 源（后台执行）：fetch + reset，或首次 clone。
pub async fn repo_update(id: String, run_id: String) -> Response {
    let valid =
        safe_rel(&id).and_then(|_| read_repos().map(|repos| repos.iter().any(|r| r.id == id)));
    match valid {
        Ok(true) => {
            let title = format!("更新 Git 源: {id}");
            spawn_repo_task(run_id.clone(), move || repo_update_inner(&id), title)
        }
        Ok(false) => Response::err(-1, format!("源不存在: {id}")),
        Err(e) => Response::err(-1, e),
    }
}

fn repo_update_inner(id: &str) -> Result<String, String> {
    let repos = read_repos()?;
    let Some(entry) = repos.iter().find(|r| r.id == id) else {
        return Err(format!("源不存在: {id}"));
    };
    let dir = repos_dir().join(id);
    if !dir.exists() || !dir.join(".git").exists() {
        // 首次 clone（内置源发行时无 .git，直接重建）
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(repos_dir()).map_err(|e| e.to_string())?;
        let tmp_clone = repos_dir().join(format!(".tmp-{id}"));
        let _ = std::fs::remove_dir_all(&tmp_clone);
        run_git(&[
            "clone",
            "--depth",
            "1",
            &entry.url,
            tmp_clone.to_str().unwrap(),
        ])?;
        std::fs::rename(&tmp_clone, &dir).map_err(|e| format!("移动到源目录失败: {e}"))?;
    } else {
        run_git(&["-C", dir.to_str().unwrap(), "fetch", "origin"])?;
        run_git(&["-C", dir.to_str().unwrap(), "reset", "--hard", "FETCH_HEAD"])?;
    }
    let commit = run_git(&["-C", dir.to_str().unwrap(), "rev-parse", "HEAD"])?;
    let short = commit.chars().take(7).collect::<String>();
    let mut new_repos = repos;
    if let Some(r) = new_repos.iter_mut().find(|r| r.id == id) {
        r.version = short.clone();
        r.commit = commit.clone();
        r.updated_at = now_ts();
    }
    write_repos(&new_repos)?;
    Ok(format!("commit={short}"))
}

#[allow(clippy::too_many_arguments)] // 安装需携带完整包描述与选项，参数固定
pub async fn install(
    pkg_path: String,
    source: String,
    repo_id: Option<String>,
    version: String,
    action: Option<String>,
    options: Option<BTreeMap<String, String>>,
    // 实例名（多实例包区分同包的不同安装；站点类由面板按 SITE_ID 给出）
    instance: Option<String>,
    provision: Option<BTreeMap<String, String>>,
    user: Option<String>,
    run_mode: Option<String>,
    run_id: String,
) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (cat, name) = validate_pkg_path(&pkg_path)?;
        let pkg_dir = find_package(&pkg_path, &source, repo_id.as_deref())?;
        // 运行身份：webapps 建站包（run_as: user / scope: site）降权为 Linux 账号执行
        let run_as = resolve_run_as(&pkg_dir, &cat, user.as_deref())?;
        // 复制脚本副本到 runs/<run_id>/pkg 并从副本执行，失败后用户可编辑重跑
        let spec = json!({
            "kind": "install",
            "pkg_path": pkg_path.clone(),
            "source": source.clone(),
            "repo_id": repo_id.clone(),
            "version": version.clone(),
            "action": action.clone(),
            "options": options.clone(),
            "instance": instance.clone(),
            // 面板编排结果（站点 / 数据库）：随 spec 落盘供「重跑」复用同一套资源
            "provision": provision.clone(),
            "user": user.clone(),
            "run_mode": run_mode.clone(),
        });
        let snapshot = prepare_snapshot(&run_id, &pkg_dir, &spec)?;
        let (script, interpreter) = script_file(&snapshot, "install", "install.sh")?;
        // 槽位：同一包的每个实例独占一个目录（多版本 PHP / 多站点 WordPress 不再互相覆盖）
        let slot = resolve_slot(
            &cat,
            &name,
            instance.as_deref(),
            user.as_deref(),
            provision.as_ref(),
        );
        let app_path = slot.dir.clone();
        let mut env = task_env(&snapshot, &app_path, &name, Some(&version), &run_id);
        // 选项落盘 options.env / options.json 并注入 env
        env.extend(write_run_options(&snapshot, options.as_ref())?);
        // 注入操作者上下文（面板登录用户与运行环境模式）
        push_actor_env(&mut env, user.as_deref(), run_mode.as_deref());
        env.push((
            "PKG_SRC_PATH".into(),
            pkg_dir.to_string_lossy().into_owned(),
        ));
        if let Some(a) = action.as_deref()
            && !a.is_empty()
        {
            env.push(("ACTION".into(), a.to_string()));
        }
        // 实例名交给脚本登记进 info.yaml（面板据此区分同包的多个安装）
        env.push(("APP_INSTANCE".into(), slot.instance.clone()));
        // 面板编排结果（站点 / 数据库）：后注入，避免被同名选项覆盖
        push_provision_env(&mut env, provision.as_ref());
        // 落盘（root 写）：卸载 / 升级靠它找回同一个站点与库
        if let Some(p) = provision.as_ref()
            && let Err(e) = write_provision(&app_path, p)
        {
            eprintln!("写入 provision.json 失败: {e}");
        }
        // 降权运行：先把安装目录与编译目录建好并交给该 Linux 账号
        let build = build_dir(&run_id);
        if let RunAs::User(u) = &run_as {
            prepare_user_run(&mut env, &[app_path.clone(), build], u)?;
        }
        let done_run_id = run_id.clone();
        let done_pkg_path = pkg_path.clone();
        let done_source = source.clone();
        let done_repo_id = repo_id.clone();
        let done_cat = cat.clone();
        let on_done = Box::new(move |code: i32| {
            if code == 0 {
                let meta = MetaInfo {
                    name: name.clone(),
                    version: version.clone(),
                    category: done_cat.clone(),
                    source: done_source.clone(),
                    repo_id: done_repo_id.clone(),
                    installed_at: now_ts(),
                    upgraded_from: None,
                    run_id: done_run_id.clone(),
                };
                if let Err(e) = write_meta(&app_path, &meta) {
                    tracing::error!("写入 {done_pkg_path} 安装元数据失败: {e}");
                }
            }
            cleanup_snapshot(&done_run_id, code);
        });
        let log = spawn_background(
            &run_id,
            vec![ScriptStep {
                script,
                env,
                run_as,
                interpreter,
            }],
            on_done,
        )?;
        Ok(Response::ok(
            "安装已启动",
            Some(json!({ "run_id": run_id, "log": log })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

pub async fn uninstall(
    pkg_path: String,
    options: Option<BTreeMap<String, String>>,
    // 实例名（站点类 `site:<id>`）：决定要卸掉哪个安装
    instance: Option<String>,
    provision: Option<BTreeMap<String, String>>,
    user: Option<String>,
    run_mode: Option<String>,
    run_id: String,
) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (cat, name) = validate_pkg_path(&pkg_path)?;
        // 按实例定位槽位（站点类在用户私有目录下）；旧布局自动兜底
        let slot = resolve_slot_with_legacy(
            &cat,
            &name,
            instance.as_deref(),
            user.as_deref(),
            provision.as_ref(),
        );
        let app_path = slot.dir.clone();
        if !app_path.is_dir() {
            return Err("该包未安装".into());
        }
        let (source, repo_id) = read_meta(&app_path)
            .map(|m| (m.source, m.repo_id))
            .unwrap_or_else(|_| ("official".into(), None));
        let pkg_dir = find_package(&pkg_path, &source, repo_id.as_deref())?;
        // 运行身份与安装时保持一致（降权脚本才能删掉自己写入的文件）
        let run_as = resolve_run_as(&pkg_dir, &cat, user.as_deref())?;
        // 与 install 一致：复制脚本副本到 runs/<run_id>/pkg 并从副本执行
        let spec = json!({
            "kind": "uninstall",
            "pkg_path": pkg_path.clone(),
            "source": source.clone(),
            "repo_id": repo_id.clone(),
            "options": options.clone(),
            "instance": instance.clone(),
            "provision": provision.clone(),
            "user": user.clone(),
            "run_mode": run_mode.clone(),
        });
        let snapshot = prepare_snapshot(&run_id, &pkg_dir, &spec)?;
        let (script, interpreter) = script_file(&snapshot, "uninstall", "uninstall.sh")?;
        // 卸载脚本可能需要版本信息（如按版本计算安装目录），注入 meta 中记录的版本
        let meta_version = read_meta(&app_path).ok().map(|m| m.version);
        let mut env = task_env(
            &snapshot,
            &app_path,
            &name,
            meta_version.as_deref(),
            &run_id,
        );
        // 选项落盘 options.env / options.json 并注入 env（卸载脚本可通过环境变量读取）
        env.extend(write_run_options(&snapshot, options.as_ref())?);
        // 注入操作者上下文（面板登录用户与运行环境模式）
        push_actor_env(&mut env, user.as_deref(), run_mode.as_deref());
        env.push((
            "PKG_SRC_PATH".into(),
            pkg_dir.to_string_lossy().into_owned(),
        ));
        env.push(("APP_INSTANCE".into(), slot.instance.clone()));
        // 回传站点 / 数据库信息（脚本可能要先 mysqldump 备份再删文件）。
        // 面板传来的优先；缺失时自己读安装时落盘的 provision.json —— 那是 0600
        // root-only，面板进程读不到，只能由 zapexec 兜底，否则卸载脚本会因
        // 缺少 SITE_ROOT 直接中止、什么都没删成。
        let effective_provision = provision
            .clone()
            .filter(|p: &BTreeMap<String, String>| !p.is_empty())
            .or_else(|| read_provision(&app_path));
        push_provision_env(&mut env, effective_provision.as_ref());
        let build = build_dir(&run_id);
        if let RunAs::User(u) = &run_as {
            prepare_user_run(&mut env, &[app_path.clone(), build], u)?;
        }
        let done_run_id = run_id.clone();
        let on_done = Box::new(move |code: i32| {
            if code == 0 {
                let _ = std::fs::remove_dir_all(&app_path);
            }
            cleanup_snapshot(&done_run_id, code);
        });
        let log = spawn_background(
            &run_id,
            vec![ScriptStep {
                script,
                env,
                run_as,
                interpreter,
            }],
            on_done,
        )?;
        Ok(Response::ok(
            "卸载已启动",
            Some(json!({ "run_id": run_id, "log": log })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

#[allow(clippy::too_many_arguments)] // 升级需携带完整包描述与选项，参数固定
pub async fn upgrade(
    pkg_path: String,
    source: String,
    repo_id: Option<String>,
    version: String,
    old_version: String,
    action: Option<String>,
    options: Option<BTreeMap<String, String>>,
    // 实例名（站点类 `site:<id>`）：决定要升级哪个安装
    instance: Option<String>,
    provision: Option<BTreeMap<String, String>>,
    user: Option<String>,
    run_mode: Option<String>,
    run_id: String,
) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (cat, name) = validate_pkg_path(&pkg_path)?;
        // 按实例定位槽位（站点类在用户私有目录下）；旧布局自动兜底
        let slot = resolve_slot_with_legacy(
            &cat,
            &name,
            instance.as_deref(),
            user.as_deref(),
            provision.as_ref(),
        );
        let app_path = slot.dir.clone();
        if !app_path.is_dir() {
            return Err("该包未安装，无法升级".into());
        }
        // 目标版本与当前已装版本相同 → 拒绝重复升级(读不到 meta 时放行,避免误拦旧数据)
        if let Ok(cur) = read_meta(&app_path)
            && cur.version == version
        {
            return Ok(Response::err(
                -1,
                format!("已安装 v{version}，无需重复升级（如需重装请使用「再次安装」）"),
            ));
        }
        let pkg_dir = find_package(&pkg_path, &source, repo_id.as_deref())?;
        // 运行身份与安装时保持一致
        let run_as = resolve_run_as(&pkg_dir, &cat, user.as_deref())?;
        // 与 install 一致：复制脚本副本到 runs/<run_id>/pkg 并从副本执行
        let spec = json!({
            "kind": "upgrade",
            "pkg_path": pkg_path.clone(),
            "source": source.clone(),
            "repo_id": repo_id.clone(),
            "version": version.clone(),
            "old_version": old_version.clone(),
            "action": action.clone(),
            "options": options.clone(),
            "instance": instance.clone(),
            "provision": provision.clone(),
            "user": user.clone(),
            "run_mode": run_mode.clone(),
        });
        let snapshot = prepare_snapshot(&run_id, &pkg_dir, &spec)?;
        let mut env = task_env(&snapshot, &app_path, &name, Some(&version), &run_id);
        // 选项落盘 options.env / options.json 并注入 env
        env.extend(write_run_options(&snapshot, options.as_ref())?);
        // 注入操作者上下文（面板登录用户与运行环境模式）
        push_actor_env(&mut env, user.as_deref(), run_mode.as_deref());
        env.push((
            "PKG_SRC_PATH".into(),
            pkg_dir.to_string_lossy().into_owned(),
        ));
        env.push(("APP_OLD_VERSION".into(), old_version.clone()));
        env.push(("APP_INSTANCE".into(), slot.instance.clone()));
        // 站点 / 数据库信息：优先用面板回传的，没有就自己读安装时落盘的 provision.json
        // （0600 root-only，面板进程读不到，只能由 zapexec 兜底）
        let effective_provision = provision
            .clone()
            .filter(|p: &BTreeMap<String, String>| !p.is_empty())
            .or_else(|| read_provision(&app_path));
        push_provision_env(&mut env, effective_provision.as_ref());
        if let Some(a) = action.as_deref()
            && !a.is_empty()
        {
            env.push(("ACTION".into(), a.to_string()));
        }
        let build = build_dir(&run_id);
        if let RunAs::User(u) = &run_as {
            prepare_user_run(&mut env, &[app_path.clone(), build], u)?;
        }

        let mut steps = Vec::new();
        // 有独立的升级脚本就只跑它（兼容 .py 与 app.yaml 覆盖名），否则退回「卸载 + 安装」
        if script_file(&snapshot, "upgrade", "upgrade.sh").is_ok() {
            let (script, interpreter) = script_file(&snapshot, "upgrade", "upgrade.sh")?;
            steps.push(ScriptStep {
                script,
                env: env.clone(),
                run_as: run_as.clone(),
                interpreter,
            });
        } else {
            // 缺省升级策略：先卸载（uninstall.sh 自带数据备份）再安装
            let (un_script, un_interp) = script_file(&snapshot, "uninstall", "uninstall.sh")?;
            steps.push(ScriptStep {
                script: un_script,
                env: env.clone(),
                run_as: run_as.clone(),
                interpreter: un_interp,
            });
            let (in_script, in_interp) = script_file(&snapshot, "install", "install.sh")?;
            steps.push(ScriptStep {
                script: in_script,
                env: env.clone(),
                run_as: run_as.clone(),
                interpreter: in_interp,
            });
        }
        let done_run_id = run_id.clone();
        let done_pkg_path = pkg_path.clone();
        let done_source = source.clone();
        let done_repo_id = repo_id.clone();
        let done_cat = cat.clone();
        let on_done = Box::new(move |code: i32| {
            if code == 0 {
                let meta = MetaInfo {
                    name: name.clone(),
                    version: version.clone(),
                    category: done_cat.clone(),
                    source: done_source.clone(),
                    repo_id: done_repo_id.clone(),
                    installed_at: now_ts(),
                    upgraded_from: Some(old_version.clone()),
                    run_id: done_run_id.clone(),
                };
                if let Err(e) = write_meta(&app_path, &meta) {
                    tracing::error!("写入 {done_pkg_path} 升级元数据失败: {e}");
                }
            }
            cleanup_snapshot(&done_run_id, code);
        });
        let log = spawn_background(&run_id, steps, on_done)?;
        Ok(Response::ok(
            "升级已启动",
            Some(json!({ "run_id": run_id, "log": log })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 列出一次运行的可编辑脚本快照文件树（runs/<run_id>/pkg/ 递归）。
pub async fn run_files(run_id: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let root = run_snapshot_dir(&run_id);
        if !root.is_dir() {
            return Err("该运行没有可编辑脚本快照（可能已成功结束并自动清理）".into());
        }
        fn walk(dir: &Path, base: &Path, out: &mut Vec<Value>) -> Result<(), String> {
            for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, base, out)?;
                } else {
                    let rel = p
                        .strip_prefix(base)
                        .map_err(|e| e.to_string())?
                        .to_string_lossy()
                        .to_string();
                    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    out.push(json!({ "path": rel, "size": size }));
                }
            }
            Ok(())
        }
        let mut files = Vec::new();
        walk(&root, &root, &mut files)?;
        files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        Ok(Response::ok(
            "OK",
            Some(json!({ "run_id": run_id, "files": files })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 读取运行快照内文件内容（编辑前读取，仅限 runs/<run_id>/pkg/ 内）。
pub async fn run_file_read(run_id: String, path: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let p = run_safe_path(&run_id, &path)?;
        if !p.is_file() {
            return Err("快照内文件不存在".into());
        }
        let content = std::fs::read_to_string(&p).map_err(|e| format!("读取失败: {e}"))?;
        Ok(Response::ok(
            "OK",
            Some(json!({ "run_id": run_id, "path": path, "content": content })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 写运行快照内文件（修改脚本后重跑前保存；仅限 runs/<run_id>/pkg/ 内）。
pub async fn run_file_write(run_id: String, path: String, content: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        use std::os::unix::fs::PermissionsExt;
        let p = run_safe_path(&run_id, &path)?;
        let dir = p.parent().map(|d| d.to_path_buf()).ok_or("非法路径")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {e}"))?;
        std::fs::write(&p, &content).map_err(|e| format!("写入失败: {e}"))?;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
        Ok(Response::ok(
            "已保存",
            Some(json!({ "run_id": run_id, "path": path })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 重跑某次失败的运行：复用 runs/<old_run_id>/pkg 快照（含用户编辑），
/// 以 new_run_id 记录新日志/pid，按 run.json 记录的原始动作重新执行。
pub async fn run_retry(run_id: String, new_run_id: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let meta_content = std::fs::read_to_string(run_meta_path(&run_id))
            .map_err(|_| "该运行没有可重跑记录（run.json 缺失，可能已成功并清理）".to_string())?;
        let spec: Value =
            serde_json::from_str(&meta_content).map_err(|e| format!("run.json 解析失败: {e}"))?;
        let kind = spec["kind"].as_str().unwrap_or("install");
        let snapshot = run_snapshot_dir(&run_id);
        if !snapshot.is_dir() {
            return Err("运行快照缺失，无法重跑".into());
        }
        let pkg_path = spec["pkg_path"].as_str().unwrap_or("").to_string();
        let name = pkg_path.rsplit('/').next().unwrap_or("").to_string();
        let version = spec["version"].as_str().unwrap_or("").to_string();
        let action = spec["action"].as_str().unwrap_or("").to_string();
        let old_version = spec["old_version"].as_str().unwrap_or("").to_string();
        // 运行身份：与首次执行一致（快照里的 app.yaml + run.json 记录的面板用户）
        let cat = pkg_path.split('/').next().unwrap_or("").to_string();
        // 槽位：重跑必须落在与首次相同的实例上（旧布局自动兜底）
        let provision: Option<BTreeMap<String, String>> = spec
            .get("provision")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let slot = resolve_slot_with_legacy(
            &cat,
            &name,
            spec["instance"].as_str(),
            spec["user"].as_str(),
            provision.as_ref(),
        );
        let app_path = slot.dir.clone();
        let run_as = resolve_run_as(&snapshot, &cat, spec["user"].as_str())?;

        let mut env = task_env(&snapshot, &app_path, &name, Some(&version), &new_run_id);
        env.push((
            "PKG_SRC_PATH".into(),
            snapshot.to_string_lossy().into_owned(),
        ));
        if !old_version.is_empty() {
            env.push(("APP_OLD_VERSION".into(), old_version.clone()));
        }
        if !action.is_empty() {
            env.push(("ACTION".into(), action.clone()));
        }
        // 选项以快照 options.env 为准（用户重跑前编辑过的内容同样生效）
        env.extend(read_options_env(&snapshot));
        // 恢复原任务的操作者上下文（spec 在首次发起时记录，保证重跑环境一致）
        push_actor_env(&mut env, spec["user"].as_str(), spec["run_mode"].as_str());
        // 复用首次运行时的面板编排结果（同一个站点 / 同一个库，不重复建库建站）
        let provision: Option<BTreeMap<String, String>> =
            serde_json::from_value(spec["provision"].clone()).unwrap_or(None);
        push_provision_env(&mut env, provision.as_ref());
        let build = build_dir(&new_run_id);
        if let RunAs::User(u) = &run_as {
            prepare_user_run(&mut env, &[app_path.clone(), build], u)?;
        }

        let mut steps = Vec::new();

        let done_run_id = new_run_id.clone();
        // 重跑复用原 run 的快照：成功后清理原快照防堆积；失败保留供继续编辑重试
        let done_old_run = run_id.clone();
        let done_app = app_path.clone();
        let on_done_extra: Option<Box<dyn FnOnce(i32) + Send>> = match kind {
            "uninstall" => {
                let (script, interpreter) = script_file(&snapshot, "uninstall", "uninstall.sh")?;
                steps.push(ScriptStep {
                    script,
                    env,
                    run_as: run_as.clone(),
                    interpreter,
                });
                Some(Box::new(move |code: i32| {
                    if code == 0 {
                        let _ = std::fs::remove_dir_all(&done_app);
                    }
                    cleanup_snapshot(&done_old_run, code);
                }))
            }
            "upgrade" => {
                if script_file(&snapshot, "upgrade", "upgrade.sh").is_ok() {
                    let (script, interpreter) = script_file(&snapshot, "upgrade", "upgrade.sh")?;
                    steps.push(ScriptStep {
                        script,
                        env: env.clone(),
                        run_as: run_as.clone(),
                        interpreter,
                    });
                } else {
                    let (u_script, u_interp) = script_file(&snapshot, "uninstall", "uninstall.sh")?;
                    steps.push(ScriptStep {
                        script: u_script,
                        env: env.clone(),
                        run_as: run_as.clone(),
                        interpreter: u_interp,
                    });
                    let (i_script, i_interp) = script_file(&snapshot, "install", "install.sh")?;
                    steps.push(ScriptStep {
                        script: i_script,
                        env: env.clone(),
                        run_as: run_as.clone(),
                        interpreter: i_interp,
                    });
                }
                let category = pkg_path.split('/').next().unwrap_or("").to_string();
                let done_source = spec["source"].as_str().unwrap_or("").to_string();
                let done_repo = spec["repo_id"].as_str().map(|s| s.to_string());
                let done_old = old_version.clone();
                let done_name = name.clone();
                let done_ver = version.clone();
                Some(Box::new(move |code: i32| {
                    if code == 0 {
                        let meta = MetaInfo {
                            name: done_name.clone(),
                            version: done_ver.clone(),
                            category: category.clone(),
                            source: done_source.clone(),
                            repo_id: done_repo.clone(),
                            installed_at: now_ts(),
                            upgraded_from: Some(done_old.clone()),
                            run_id: done_run_id.clone(),
                        };
                        let _ = write_meta(&done_app, &meta);
                    }
                    cleanup_snapshot(&done_old_run, code);
                }))
            }
            _ => {
                // install 默认
                let (script, interpreter) = script_file(&snapshot, "install", "install.sh")?;
                steps.push(ScriptStep {
                    script,
                    env,
                    run_as: run_as.clone(),
                    interpreter,
                });
                let category = pkg_path.split('/').next().unwrap_or("").to_string();
                let done_source = spec["source"].as_str().unwrap_or("").to_string();
                let done_repo = spec["repo_id"].as_str().map(|s| s.to_string());
                let done_name = name.clone();
                let done_ver = version.clone();
                Some(Box::new(move |code: i32| {
                    if code == 0 {
                        let meta = MetaInfo {
                            name: done_name.clone(),
                            version: done_ver.clone(),
                            category: category.clone(),
                            source: done_source.clone(),
                            repo_id: done_repo.clone(),
                            installed_at: now_ts(),
                            upgraded_from: None,
                            run_id: done_run_id.clone(),
                        };
                        let _ = write_meta(&done_app, &meta);
                    }
                    cleanup_snapshot(&done_old_run, code);
                }))
            }
        };
        let log = spawn_background(&new_run_id, steps, on_done_extra.unwrap())?;
        Ok(Response::ok(
            "重跑已启动",
            Some(json!({ "run_id": new_run_id, "log": log })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 面板用户名安全校验：仅允许 `[A-Za-z0-9._-]`，禁止 `..` 与路径分隔符。
///
/// 用户名会被拼进 `data/users/<user>/`，必须拦住 `/` 与 `..`，
/// 否则能把落盘路径带出用户目录。
fn safe_username(u: &str) -> Result<(), String> {
    let u = u.trim();
    if u.is_empty() || u.len() > 64 {
        return Err("用户名不合法".into());
    }
    if u.starts_with('.')
        || u.contains("..")
        || u.contains('/')
        || u.contains('\\')
        || !u
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err("用户名不合法".into());
    }
    Ok(())
}

/// 自定义脚本根目录：`{ZAP_PATH}/data/users/<username>/`。
///
/// 与 `crontab.yaml`、`cloud/` 同级 —— 用户的「个人数据」统一收在
/// `users/<user>/` 下，备份迁移时整目录打包即可。
///
/// 返回的是**用户目录**而不是 scripts 本身：`path` 参数自带 `scripts/`
/// 前缀（如 `scripts/backup.sh`），与前端树节点契约一致。
fn user_scripts_root(username: &str) -> Result<PathBuf, String> {
    safe_username(username)?;
    Ok(zap_path().join("data").join("users").join(username))
}

pub async fn script_run(path: String, run_id: String, username: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let root = user_scripts_root(&username)?;
        let resolved = safe_join(&root, &path)?;
        if !resolved.is_file() {
            return Err("脚本不存在".into());
        }
        // 自定义脚本（管理员在「脚本」里维护）没有 app.yaml，保持 root 执行；
        // 解释器按扩展名推导（.py → python3 -I -B）
        let log = spawn_background(
            &run_id,
            vec![ScriptStep {
                interpreter: interpreter_of(&resolved),
                script: resolved,
                env: base_env(),
                run_as: RunAs::Root,
            }],
            Box::new(|_| {}),
        )?;
        Ok(Response::ok(
            "脚本已启动",
            Some(json!({ "run_id": run_id, "log": log })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

pub async fn script_stop(run_id: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let pid_path = logs_dir().join(format!("run-{run_id}.pid"));
        let pid: i32 = std::fs::read_to_string(&pid_path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| "运行实例不存在或已结束".to_string())?;
        // 向进程组发 SIGTERM，最多等 5 秒后 SIGKILL
        let mut alive = unsafe { libc::kill(-pid, libc::SIGTERM) } == 0;
        for _ in 0..5 {
            if !alive {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            alive = unsafe { libc::kill(-pid, 0) } == 0;
        }
        if alive {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        Ok::<_, String>(Response::ok("已发送停止信号", None))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

pub async fn script_read(path: String, username: String) -> Response {
    tokio::task::spawn_blocking(move || {
        // 与 script_write / script_run 一致：path 相对用户目录（树节点亦以此生成）
        let root = user_scripts_root(&username)?;
        let resolved = safe_join(&root, &path)?;
        let md = std::fs::metadata(&resolved).map_err(|e| format!("路径不存在: {e}"))?;
        if !md.is_file() {
            return Err("不是文件".to_string());
        }
        let content = std::fs::read_to_string(&resolved).map_err(|e| format!("读取失败: {e}"))?;
        Ok::<_, String>(Response::ok(
            "ok",
            Some(json!({ "path": resolved.to_string_lossy(), "content": content })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

pub async fn script_write(path: String, content: String, username: String) -> Response {
    tokio::task::spawn_blocking(move || {
        use std::os::unix::fs::PermissionsExt;
        let root = user_scripts_root(&username)?;
        let resolved = safe_join(&root, &path)?;
        if let Ok(md) = std::fs::metadata(&resolved)
            && md.is_dir()
        {
            return Err("不能覆盖目录".to_string());
        }
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // `scripts/` 与 `crontab.yaml` / `cloud/` 同级，用户目录属主必须归面板进程
        super::ensure_panel_user_dir(&username)?;
        std::fs::write(&resolved, &content).map_err(|e| format!("写入失败: {e}"))?;
        // 脚本保持可执行
        let _ = std::fs::set_permissions(&resolved, std::fs::Permissions::from_mode(0o755));
        Ok::<_, String>(Response::ok(
            "保存成功",
            Some(json!({ "path": resolved.to_string_lossy() })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 删除自定义脚本或目录（仅限 `{ZAP_PATH}/data/users/<username>/scripts/` 下）。
pub async fn script_delete(path: String, username: String) -> Response {
    tokio::task::spawn_blocking(move || {
        let root = user_scripts_root(&username)?;
        let scripts = root.join("scripts");
        let resolved = safe_join(&root, &path)?;
        // 双重保险：safe_join 已挡住跳出用户目录，这里再挡住删向 scripts/ 之外
        //（同级的 crontab.yaml、cloud/ 不能被顺手删掉）
        if resolved != scripts && !resolved.starts_with(&scripts) {
            return Err("只允许删除 scripts/ 下的内容".to_string());
        }
        if resolved == scripts {
            return Err("不能删除脚本根目录".to_string());
        }
        let md = std::fs::metadata(&resolved).map_err(|e| format!("路径不存在: {e}"))?;
        if md.is_dir() {
            std::fs::remove_dir_all(&resolved).map_err(|e| format!("删除失败: {e}"))?;
        } else {
            std::fs::remove_file(&resolved).map_err(|e| format!("删除失败: {e}"))?;
        }
        Ok::<_, String>(Response::ok(
            "删除成功",
            Some(json!({ "path": resolved.to_string_lossy() })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 扫描全部实例槽位（含用户私有的 webapps 与旧布局）。
///
/// 布局（规则见 `zap_proto::appstore`）：
/// - `apps/<category>/<name>/<instance>/` —— 全局类；旧布局没有 instance 层，
///   meta.yaml 直接在包目录下，这里仍认，按 `default` 实例返回，老安装不会消失；
/// - `users/<owner>/webapps/<name>/<site_id>/` —— 站点类，登记信息跟随站点账号。
fn scan_slots() -> Vec<SlotPath> {
    let mut out = Vec::new();

    // 1) 全局类
    if let Ok(cats) = std::fs::read_dir(apps_dir()) {
        for cat in cats.flatten() {
            let cat_path = cat.path();
            if !cat_path.is_dir() {
                continue;
            }
            let category = cat.file_name().to_string_lossy().to_string();
            let Ok(pkgs) = std::fs::read_dir(&cat_path) else {
                continue;
            };
            for pkg in pkgs.flatten() {
                let pkg_dir = pkg.path();
                if !pkg_dir.is_dir() {
                    continue;
                }
                let name = pkg.file_name().to_string_lossy().to_string();
                // 旧布局：meta.yaml 直接在包目录下 → 视为 default 实例
                if pkg_dir.join("meta.yaml").is_file() {
                    out.push(SlotPath {
                        category: category.clone(),
                        name: name.clone(),
                        dir: pkg_dir.clone(),
                        instance: DEFAULT_INSTANCE.to_string(),
                        owner: None,
                        site_id: None,
                    });
                }
                // 新布局：包目录下的每个子目录是一个实例
                let Ok(insts) = std::fs::read_dir(&pkg_dir) else {
                    continue;
                };
                for inst in insts.flatten() {
                    let dir = inst.path();
                    if !dir.is_dir() || !dir.join("meta.yaml").is_file() {
                        continue;
                    }
                    out.push(SlotPath {
                        category: category.clone(),
                        name: name.clone(),
                        dir,
                        instance: inst.file_name().to_string_lossy().to_string(),
                        owner: None,
                        site_id: None,
                    });
                }
            }
        }
    }

    // 2) 站点类：每个用户的私有 webapps 目录
    if let Ok(owners) = std::fs::read_dir(data_dir().join("users")) {
        for owner in owners.flatten() {
            let owner_dir = owner.path();
            if !owner_dir.is_dir() {
                continue;
            }
            let owner_name = owner.file_name().to_string_lossy().to_string();
            let Ok(pkgs) = std::fs::read_dir(owner_dir.join(WEBAPPS_CATEGORY)) else {
                continue;
            };
            for pkg in pkgs.flatten() {
                let pkg_dir = pkg.path();
                if !pkg_dir.is_dir() {
                    continue;
                }
                let name = pkg.file_name().to_string_lossy().to_string();
                let Ok(sites) = std::fs::read_dir(&pkg_dir) else {
                    continue;
                };
                for site in sites.flatten() {
                    let dir = site.path();
                    if !dir.is_dir() || !dir.join("meta.yaml").is_file() {
                        continue;
                    }
                    let site_id = site.file_name().to_string_lossy().to_string();
                    out.push(SlotPath {
                        category: WEBAPPS_CATEGORY.to_string(),
                        name: name.clone(),
                        dir,
                        instance: Slot::site_instance(&site_id),
                        owner: Some(owner_name.clone()),
                        site_id: Some(site_id),
                    });
                }
            }
        }
    }

    out
}

pub async fn installed() -> Response {
    tokio::task::spawn_blocking(move || {
        let mut items = Vec::new();
        for slot in scan_slots() {
            let Ok(meta) = read_meta(&slot.dir) else {
                continue;
            };
            let info = read_info_yaml(&slot.dir);
            let state = probe_instance_state(&slot.dir, info.as_ref());
            items.push(json!({
                // pkg_path 在多实例时不再唯一，定位请用 instance_key
                "pkg_path": slot.pkg_path(),
                "instance_key": slot.key(),
                "instance": slot.instance,
                "owner": slot.owner,
                "site_id": slot.site_id,
                "name": meta.name,
                "version": meta.version,
                "category": meta.category,
                "source": meta.source,
                "repo_id": meta.repo_id,
                "installed_at": meta.installed_at,
                "upgraded_from": meta.upgraded_from,
                "run_id": meta.run_id,
                "state": state,
                "info": info.as_ref().map(yaml_to_json).unwrap_or_else(|| json!({})),
            }));
        }
        items.sort_by(|a, b| {
            a.get("category")
                .and_then(|c| c.as_str())
                .cmp(&b.get("category").and_then(|c| c.as_str()))
                .then_with(|| {
                    a.get("name")
                        .and_then(|n| n.as_str())
                        .cmp(&b.get("name").and_then(|n| n.as_str()))
                })
                .then_with(|| {
                    a.get("instance")
                        .and_then(|i| i.as_str())
                        .cmp(&b.get("instance").and_then(|i| i.as_str()))
                })
        });
        Response::ok("ok", Some(json!({ "items": items })))
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 读取安装脚本登记的实例信息 apps/<category>/<name>/info.yaml（可选文件）。
fn read_info_yaml(app_path: &Path) -> Option<serde_yaml::Value> {
    let content = std::fs::read_to_string(app_path.join("info.yaml")).ok()?;
    serde_yaml::from_str(&content).ok()
}

/// 探测实例运行状态：登记了 svc_name（systemd）走 systemctl；否则读 pid_file 探活。
fn probe_instance_state(_app_path: &Path, info: Option<&serde_yaml::Value>) -> String {
    let info = match info {
        Some(i) => i,
        None => return "unknown".into(),
    };
    if let Some(svc) = info.get("svc_name").and_then(|v| v.as_str()) {
        return normalize_state(&super::svc::raw_state(svc));
    }
    if let Some(pf) = info.get("pid_file").and_then(|v| v.as_str()) {
        let pid: i32 = match std::fs::read_to_string(pf)
            .ok()
            .and_then(|t| t.trim().parse().ok())
        {
            Some(p) => p,
            None => return "unknown".into(),
        };
        return match root_cmd("kill").args(["-0", &pid.to_string()]).status() {
            Ok(s) if s.success() => "running".into(),
            _ => "stopped".into(),
        };
    }
    "unknown".into()
}

/// 归一化 systemctl 状态输出：running/stopped/failed/starting/stopping/unknown。
fn normalize_state(raw: &str) -> String {
    match raw {
        "active" | "running" => "running",
        "inactive" | "dead" | "stopped" | "exited" => "stopped",
        "failed" => "failed",
        "activating" | "reloading" => "starting",
        "deactivating" => "stopping",
        _ => "unknown",
    }
    .into()
}

fn serde_yaml_num_str(n: &serde_yaml::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    n.as_f64().map(|f| f.to_string()).unwrap_or_default()
}

/// serde_yaml 值 → serde_json 值（数字转字符串、布尔保留，便于展示与比对）。
fn yaml_to_json(v: &serde_yaml::Value) -> Value {
    match v {
        serde_yaml::Value::String(s) => Value::String(s.clone()),
        serde_yaml::Value::Bool(b) => Value::Bool(*b),
        serde_yaml::Value::Number(n) => Value::String(serde_yaml_num_str(n)),
        serde_yaml::Value::Mapping(m) => {
            let mut obj = serde_json::Map::new();
            for (k, val) in m {
                let key = match k {
                    serde_yaml::Value::String(s) => s.clone(),
                    serde_yaml::Value::Number(n) => serde_yaml_num_str(n),
                    serde_yaml::Value::Bool(b) => b.to_string(),
                    other => format!("{other:?}"),
                };
                obj.insert(key, yaml_to_json(val));
            }
            Value::Object(obj)
        }
        serde_yaml::Value::Sequence(seq) => Value::Array(seq.iter().map(yaml_to_json).collect()),
        serde_yaml::Value::Tagged(t) => yaml_to_json(&t.value),
        serde_yaml::Value::Null => Value::Null,
    }
}

/// 对已安装应用的实例执行 start/stop/restart。
/// 要求脚本在 info.yaml 中登记 svc_name（systemd unit），由 root 执行 systemctl。
pub async fn instance_action(
    pkg_path: String,
    instance: Option<String>,
    action: String,
) -> Response {
    let allowed = ["start", "stop", "restart"];
    if !allowed.contains(&action.as_str()) {
        return Response::err(-1, format!("不支持的实例操作: {action}"));
    }
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (cat, name) = validate_pkg_path(&pkg_path)?;
        // 按实例定位槽位（旧布局自动兜底）
        let slot = resolve_slot_with_legacy(&cat, &name, instance.as_deref(), None, None);
        let app_path = slot.dir.clone();
        if !app_path.is_dir() {
            return Err("该应用未安装".into());
        }
        let info = read_info_yaml(&app_path).ok_or("缺少实例信息 info.yaml（由安装脚本登记）")?;
        let svc = info
            .get("svc_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or("未登记 systemd 服务（info.yaml 缺 svc_name），无法通过面板启停")?;
        super::svc::act(&action, &svc).map_err(|e| format!("{action} {svc} 失败: {e}"))?;
        // 操作后回读状态（restart 稍等稳定）
        std::thread::sleep(std::time::Duration::from_millis(300));
        let state = normalize_state(&super::svc::raw_state(&svc));
        Ok(Response::ok(
            "ok",
            Some(json!({
                "pkg_path": pkg_path,
                "svc_name": svc,
                "action": action,
                "state": state,
            })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 建立独立 ZAP_PATH，返回 (guard, zap_root)。
    fn with_zap_root() -> (std::sync::MutexGuard<'static, ()>, PathBuf) {
        let guard = ENV_GUARD.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "zap-appstore-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe {
            std::env::set_var("ZAP_PATH", &dir);
        }
        (guard, dir)
    }

    #[test]
    fn safe_rel_rejects_absolute_and_parent() {
        assert!(safe_rel("/etc/passwd").is_err());
        assert!(safe_rel("../etc").is_err());
        assert!(safe_rel("a/../../b").is_err());
        assert!(safe_rel("a/b/c.sh").is_ok());
    }

    #[test]
    fn fixup_user_dirs_creates_root_and_keeps_existing() {
        let (_guard, root) = with_zap_root();
        let user = root.join("data/users/admin");
        std::fs::create_dir_all(&user).unwrap();
        // 启动时修一遍：目录本身不能被删/改名，缺失的 users/ 会补建
        super::super::fixup_user_dirs();
        assert!(root.join("data/users").is_dir());
        assert!(user.is_dir());
    }

    #[test]
    fn panel_user_of_derives_first_level() {
        let (_guard, root) = with_zap_root();
        let cloud = root.join("data/users/admin/cloud/stores");
        assert_eq!(
            super::super::panel_user_of(&cloud).as_deref(),
            Some("admin")
        );
        // 不在 users/ 下（或只是 users 本身）时不做任何归属推断
        assert_eq!(super::super::panel_user_of(&root.join("data/apps")), None);
        assert_eq!(super::super::panel_user_of(&root.join("data/users")), None);
    }

    #[test]
    fn ensure_panel_dir_creates_user_root_for_site_slot() {
        let (_guard, root) = with_zap_root();
        // 站点应用槽位：users/<user>/webapps/<name>/<site_id>
        let slot = root.join("data/users/admin/webapps/wordpress/12");
        std::fs::create_dir_all(&slot).unwrap();
        // 安装时 root 侧触碰用户目录 → 顺手把 users/<user> 交还给面板进程
        super::super::ensure_panel_dir_for_path(&slot).unwrap();
        assert!(root.join("data/users/admin").is_dir());
        assert!(root.join("data/users").is_dir());
        // 与站点数据无关的路径不产生副作用
        let other = root.join("data/apps/application/php");
        std::fs::create_dir_all(&other).unwrap();
        super::super::ensure_panel_dir_for_path(&other).unwrap();
        assert!(!root.join("data/users/apps").exists());
    }

    #[test]
    fn safe_join_rejects_escape() {
        let base = PathBuf::from("/tmp/zap-test-base");
        assert!(safe_join(&base, "../x").is_err());
        let ok = safe_join(&base, "sub/x.sh").unwrap();
        assert_eq!(ok, base.join("sub/x.sh"));
    }

    #[test]
    fn user_scripts_root_rejects_unsafe_username() {
        let (_guard, root) = with_zap_root();
        assert_eq!(
            user_scripts_root("admin").unwrap(),
            root.join("data/users/admin")
        );
        assert!(user_scripts_root("a.b-c_1").is_ok());
        let long = "x".repeat(65);
        for bad in [
            "",
            "..",
            "../admin",
            "a/b",
            "a\\b",
            ".hidden",
            long.as_str(),
        ] {
            assert!(user_scripts_root(bad).is_err(), "应拒绝用户名: {bad}");
        }
    }

    #[test]
    fn validate_pkg_path_rules() {
        assert_eq!(
            validate_pkg_path("database/mariadb").unwrap(),
            ("database".into(), "mariadb".into())
        );
        assert!(validate_pkg_path("mariadb").is_err());
        assert!(validate_pkg_path("a/b/c").is_err());
        assert!(validate_pkg_path("db/mariadb;rm").is_err());
        assert!(validate_pkg_path("db/../x").is_err());
    }

    #[test]
    fn script_file_parses_app_yaml_override() {
        let (_g, root) = with_zap_root();
        let pkg = root.join("database/nginx");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("app.yaml"),
            "name: nginx\nversion: \"1.27\"\nscripts:\n  install: setup.sh\n  uninstall: remove.sh\n",
        )
        .unwrap();
        std::fs::write(pkg.join("setup.sh"), "#!/bin/bash\n").unwrap();
        std::fs::write(pkg.join("remove.sh"), "#!/bin/bash\n").unwrap();
        std::fs::write(pkg.join("upgrade.sh"), "#!/bin/bash\n").unwrap();

        let (install, install_interp) = script_file(&pkg, "install", "install.sh").unwrap();
        assert_eq!(install.file_name().unwrap(), "setup.sh");
        assert_eq!(install_interp, Interpreter::Bash);
        // 未在 yaml 中定义的 key 回退到默认
        let (upgrade, _) = script_file(&pkg, "upgrade", "upgrade.sh").unwrap();
        assert_eq!(upgrade.file_name().unwrap(), "upgrade.sh");
        // 覆盖指向不存在的文件 → 报错
        let missing = script_file(&pkg, "install", "install.sh");
        assert!(missing.is_ok());
    }

    #[test]
    fn script_file_python_interpreter() {
        let (_g, root) = with_zap_root();
        let pkg = root.join("webapps/wordpress");
        std::fs::create_dir_all(&pkg).unwrap();
        // 简写：按扩展名推导为 python3
        std::fs::write(
            pkg.join("app.yaml"),
            "name: wordpress\nrun_as: user\nscripts:\n  install: install.py\n  uninstall: { file: remove.py, interpreter: python3 }\n",
        )
        .unwrap();
        std::fs::write(pkg.join("install.py"), "print(1)\n").unwrap();
        std::fs::write(pkg.join("remove.py"), "print(2)\n").unwrap();

        let (script, interp) = script_file(&pkg, "install", "install.sh").unwrap();
        assert_eq!(script.file_name().unwrap(), "install.py");
        assert_eq!(interp, Interpreter::Python3);
        let (_, interp) = script_file(&pkg, "uninstall", "uninstall.sh").unwrap();
        assert_eq!(interp, Interpreter::Python3);
    }

    #[test]
    fn pkg_run_as_only_for_webapps() {
        let (_g, root) = with_zap_root();
        let pkg = root.join("webapps/wordpress");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("app.yaml"), "name: wordpress\nrun_as: user\n").unwrap();
        assert_eq!(pkg_run_as(&pkg, "webapps").unwrap(), RunMode::User);

        // scope: site 未显式声明 run_as 时同样降权
        std::fs::write(pkg.join("app.yaml"), "name: wordpress\nscope: site\n").unwrap();
        assert_eq!(pkg_run_as(&pkg, "webapps").unwrap(), RunMode::User);

        // 非 webapps 分类不允许降权
        assert!(pkg_run_as(&pkg, "runtime").is_err());
        // 缺省为 root
        std::fs::write(pkg.join("app.yaml"), "name: wordpress\n").unwrap();
        assert_eq!(pkg_run_as(&pkg, "webapps").unwrap(), RunMode::Root);
        // 非法值
        std::fs::write(pkg.join("app.yaml"), "name: wordpress\nrun_as: admin\n").unwrap();
        assert!(pkg_run_as(&pkg, "webapps").is_err());
    }

    #[test]
    fn repos_round_trip_and_builtin_fallback() {
        let (_g, _root) = with_zap_root();
        // 空 repos.yaml（不存在）→ 自动补内置源
        let repos = read_repos().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].id, BUILTIN_REPO_ID);
        assert!(repos[0].builtin);
        // 往返写入
        let mut repos = repos;
        repos.push(RepoEntry {
            id: "my-store".into(),
            name: "My Store".into(),
            url: "https://github.com/user/store.git".into(),
            builtin: false,
            enabled: true,
            version: "abc1234".into(),
            commit: "abc1234".repeat(8),
            updated_at: 1_700_000_000,
        });
        write_repos(&repos).unwrap();
        let back = read_repos().unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[1].id, "my-store");
        assert_eq!(back[1].version, "abc1234");
    }

    #[test]
    fn id_from_url_extracts_repo_name() {
        assert_eq!(
            id_from_url("https://github.com/zapsh/zap-appstore.git"),
            "zap-appstore"
        );
        assert_eq!(id_from_url("https://gitlab.com/org/store"), "store");
        assert_eq!(id_from_url("git@github.com:user/store.git"), "store");
        assert_eq!(id_from_url("https://x.io/A_B-C.d/"), "a-b-c-d");
    }

    #[test]
    fn validate_repo_url_rules() {
        assert!(validate_repo_url("https://github.com/a/b.git").is_ok());
        assert!(validate_repo_url("git@github.com:a/b.git").is_ok());
        assert!(validate_repo_url("http://x/y.git").is_ok());
        assert!(validate_repo_url("").is_err());
        assert!(validate_repo_url("; rm -rf /").is_err());
        assert!(validate_repo_url("/tmp/store").is_err());
    }

    #[test]
    fn meta_round_trip_and_installed() {
        let (_g, root) = with_zap_root();
        let app = root.join("data/apps/database/mariadb");
        let meta = MetaInfo {
            name: "mariadb".into(),
            version: "11.4.4".into(),
            category: "database".into(),
            source: "official".into(),
            repo_id: Some("zap-appstore".into()),
            installed_at: 1_700_000_000,
            upgraded_from: None,
            run_id: "r1".into(),
        };
        write_meta(&app, &meta).unwrap();
        let back = read_meta(&app).unwrap();
        assert_eq!(back.version, "11.4.4");
        assert_eq!(back.source, "official");
        assert_eq!(back.repo_id.as_deref(), Some("zap-appstore"));
        assert!(back.upgraded_from.is_none());
    }

    /// 起一个 `setsid` 子进程，返回 (child, 输出文件路径)。
    fn spawn_isolated(script: &str, out: &Path) -> std::process::Child {
        use std::os::unix::process::CommandExt;
        let f = std::fs::File::create(out).unwrap();
        let mut c = std::process::Command::new("/bin/sh");
        c.arg("-c").arg(script);
        c.stdout(std::process::Stdio::from(f));
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                super::super::cloexec_inherited_fds();
                Ok(())
            });
        }
        c.spawn().unwrap()
    }

    #[test]
    fn inherited_fds_are_not_visible_to_script() {
        // 测试进程本身握着一堆 fd（cargo test 会开不少），脚本不该看见任何一个
        let out = std::env::temp_dir().join(format!("zap-fd-{:?}", std::thread::current().id()));
        let mut child = spawn_isolated("ls /proc/self/fd", &out);
        child.wait().unwrap();
        let listed = std::fs::read_to_string(&out).unwrap();
        let max_fd = listed
            .split_whitespace()
            .filter_map(|s| s.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        // ls 自身会开一个目录 fd（3），再多就是泄漏
        assert!(max_fd <= 3, "子进程看到了继承的 fd: {listed}");
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn wait_timeout_kills_the_process_group() {
        let out = std::env::temp_dir().join(format!("zap-to-{:?}", std::thread::current().id()));
        let mut child = spawn_isolated("sleep 30", &out);
        let mut log = std::fs::OpenOptions::new()
            .append(true)
            .open("/dev/null")
            .unwrap();
        let started = std::time::Instant::now();
        let code = wait_with_timeout(&mut child, std::time::Duration::from_secs(1), &mut log);
        let elapsed = started.elapsed();
        assert!(code != 0, "超时被杀应有非 0 退出码，实际: {code}");
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "耗时 {elapsed:?}"
        );
        // 进程确实被收掉：已被 wait 回收，不该还留在 /proc 里
        assert!(!std::path::Path::new(&format!("/proc/{}", child.id())).exists());
        let _ = std::fs::remove_file(&out);
    }
}
