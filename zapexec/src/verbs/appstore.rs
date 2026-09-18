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

use super::root_cmd;
use std::os::unix::process::CommandExt;
use zap_proto::Response;

// ── 内置源（跟随 zap 发行包发布）────────────────────────────

pub const BUILTIN_REPO_ID: &str = "zap-appstore";
pub const BUILTIN_REPO_NAME: &str = "Zap 官方应用商店";
pub const BUILTIN_REPO_URL: &str = "https://github.com/zapj/zap-appstore.git";

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

/// 已安装应用的登记信息（`apps/<category>/<name>/info.yaml`）。
///
/// 安装脚本在这里登记实例名、安装目录、主配置与 systemd 单元 —— 这是**权威来源**：
/// 其它模块（如 service_conf 的 PHP 实例探测）应读它，而不是按目录名反推。
#[derive(Debug, Clone)]
pub(crate) struct AppRegistration {
    /// info.yaml 的 instance（缺省回退包名），如 `php74`
    pub instance: String,
    pub install_dir: Option<PathBuf>,
    pub config_file: Option<PathBuf>,
    pub svc_name: Option<String>,
}

/// 枚举全部已安装应用的登记信息（无 info.yaml 的应用也会列出，字段为 None）。
pub(crate) fn registered_apps() -> Vec<AppRegistration> {
    let mut out = Vec::new();
    let Ok(categories) = std::fs::read_dir(apps_dir()) else {
        return out;
    };
    for cat in categories.flatten() {
        let cat_path = cat.path();
        if !cat_path.is_dir() {
            continue;
        }
        let Ok(apps) = std::fs::read_dir(&cat_path) else {
            continue;
        };
        for app in apps.flatten() {
            let app_path = app.path();
            if !app_path.is_dir() {
                continue;
            }
            let name = app.file_name().to_string_lossy().to_string();
            let info = read_info_yaml(&app_path);
            let pick = |k: &str| -> Option<String> {
                info.as_ref()?
                    .get(k)
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            };
            let instance = pick("instance")
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| name.clone());
            out.push(AppRegistration {
                instance,
                install_dir: pick("install_dir").map(PathBuf::from),
                config_file: pick("config_file").map(PathBuf::from),
                svc_name: pick("svc_name"),
            });
        }
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
    let _ = std::fs::write(
        run_meta_path(run_id),
        serde_json::to_string_pretty(spec).unwrap_or_default(),
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

/// 包脚本文件名：app.yaml 的 `scripts.{install|uninstall|upgrade}` 可覆盖默认约定。
fn script_file(pkg_dir: &Path, key: &str, default: &str) -> Result<PathBuf, String> {
    let mut file = default.to_string();
    if let Ok(content) = std::fs::read_to_string(pkg_dir.join("app.yaml"))
        && let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(&content)
        && let Some(name) = v
            .get("scripts")
            .and_then(|s| s.get(key))
            .and_then(|s| s.as_str())
    {
        file = name.to_string();
    }
    let p = safe_join(pkg_dir, &file)?;
    if !p.is_file() {
        return Err(format!("包脚本不存在: {}", p.display()));
    }
    Ok(p)
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

struct ScriptStep {
    script: PathBuf,
    env: Vec<(String, String)>,
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

/// 后台运行一个或多个脚本（同一 run_id、同一日志追加写）。
/// 每个脚本以 `setsid` 启动独立进程组，pid 写入 run-{id}.pid 供停止使用。
/// 全部成功退出码为 0；任一脚本失败则中断后续步骤。结束后追加 `__ZAP_DONE__ <code>`。
/// 任务受全局队列闸门约束：同一时间仅执行一个脚本任务，其余等待。
fn spawn_background(
    run_id: &str,
    steps: Vec<ScriptStep>,
    on_done: Box<dyn FnOnce(i32) + Send>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(logs_dir()).map_err(|e| e.to_string())?;
    let log_path = logs_dir().join(format!("run-{run_id}.log"));
    let pid_path = logs_dir().join(format!("run-{run_id}.pid"));
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("打开日志失败: {e}"))?;
    let ret_path = log_path.clone();
    let cpu_num = std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "1".into());

    std::thread::spawn(move || {
        use std::io::Write;
        let mut log = log_file;
        let _ = writeln!(log, "── 任务进入执行队列，等待前序任务完成后自动开始 ──");
        // 全局串行闸门：install/uninstall/upgrade/script_run 一次只执行一个
        let _queue_token = QueueToken::new();
        let _ = writeln!(log, "── 开始执行（队列闸门已获得） ──");
        let mut final_code = 0;
        for step in steps {
            let mut cmd = root_cmd("/bin/bash");
            cmd.arg(&step.script)
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
            // 新进程组：pid == pgid，便于停止时 kill(-pid)
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
            match cmd.spawn() {
                Ok(mut child) => {
                    let pid = child.id();
                    let _ = std::fs::write(&pid_path, pid.to_string());
                    let code = child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
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
        let _ = writeln!(log, "\n__ZAP_DONE__ {final_code}");
        on_done(final_code);
    });

    Ok(ret_path)
}

fn base_env() -> Vec<(String, String)> {
    vec![
        ("ZAP_PATH".into(), zap_path().to_string_lossy().into_owned()),
        ("ZAPCTL".into(), zapctl_bin().to_string_lossy().into_owned()),
        (
            "APPS_DIR".into(),
            super::install_root().to_string_lossy().into_owned(),
        ),
    ]
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
        runs_dir()
            .join(run_id)
            .join("build")
            .to_string_lossy()
            .into_owned(),
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
        let _ = writeln!(log, "=== {title} ===");
        match op() {
            Ok(detail) => {
                let _ = writeln!(log, "成功: {detail}");
                let _ = writeln!(log, "\n__ZAP_DONE__ 0");
            }
            Err(e) => {
                let _ = writeln!(log, "失败: {e}");
                let _ = writeln!(log, "\n__ZAP_DONE__ -1");
            }
        }
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
    run_capture(
        "git",
        &["clone", "--depth", "1", url, tmp_clone.to_str().unwrap()],
    )?;
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
    let commit = run_capture("git", &["-C", dir.to_str().unwrap(), "rev-parse", "HEAD"])?;
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
        run_capture(
            "git",
            &[
                "clone",
                "--depth",
                "1",
                &entry.url,
                tmp_clone.to_str().unwrap(),
            ],
        )?;
        std::fs::rename(&tmp_clone, &dir).map_err(|e| format!("移动到源目录失败: {e}"))?;
    } else {
        run_capture("git", &["-C", dir.to_str().unwrap(), "fetch", "origin"])?;
        run_capture(
            "git",
            &["-C", dir.to_str().unwrap(), "reset", "--hard", "FETCH_HEAD"],
        )?;
    }
    let commit = run_capture("git", &["-C", dir.to_str().unwrap(), "rev-parse", "HEAD"])?;
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
    user: Option<String>,
    run_mode: Option<String>,
    run_id: String,
) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (cat, name) = validate_pkg_path(&pkg_path)?;
        let pkg_dir = find_package(&pkg_path, &source, repo_id.as_deref())?;
        // 复制脚本副本到 runs/<run_id>/pkg 并从副本执行，失败后用户可编辑重跑
        let spec = json!({
            "kind": "install",
            "pkg_path": pkg_path.clone(),
            "source": source.clone(),
            "repo_id": repo_id.clone(),
            "version": version.clone(),
            "action": action.clone(),
            "options": options.clone(),
            "user": user.clone(),
            "run_mode": run_mode.clone(),
        });
        let snapshot = prepare_snapshot(&run_id, &pkg_dir, &spec)?;
        let script = script_file(&snapshot, "install", "install.sh")?;
        let app_path = apps_dir().join(&pkg_path);
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
        let log = spawn_background(&run_id, vec![ScriptStep { script, env }], on_done)?;
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
    user: Option<String>,
    run_mode: Option<String>,
    run_id: String,
) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (_, name) = validate_pkg_path(&pkg_path)?;
        let app_path = apps_dir().join(&pkg_path);
        if !app_path.is_dir() {
            return Err("该包未安装".into());
        }
        let (source, repo_id) = read_meta(&app_path)
            .map(|m| (m.source, m.repo_id))
            .unwrap_or_else(|_| ("official".into(), None));
        let pkg_dir = find_package(&pkg_path, &source, repo_id.as_deref())?;
        // 与 install 一致：复制脚本副本到 runs/<run_id>/pkg 并从副本执行
        let spec = json!({
            "kind": "uninstall",
            "pkg_path": pkg_path.clone(),
            "source": source.clone(),
            "repo_id": repo_id.clone(),
            "options": options.clone(),
            "user": user.clone(),
            "run_mode": run_mode.clone(),
        });
        let snapshot = prepare_snapshot(&run_id, &pkg_dir, &spec)?;
        let script = script_file(&snapshot, "uninstall", "uninstall.sh")?;
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
        let done_run_id = run_id.clone();
        let on_done = Box::new(move |code: i32| {
            if code == 0 {
                let _ = std::fs::remove_dir_all(&app_path);
            }
            cleanup_snapshot(&done_run_id, code);
        });
        let log = spawn_background(&run_id, vec![ScriptStep { script, env }], on_done)?;
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
    user: Option<String>,
    run_mode: Option<String>,
    run_id: String,
) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (cat, name) = validate_pkg_path(&pkg_path)?;
        let app_path = apps_dir().join(&pkg_path);
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
        if let Some(a) = action.as_deref()
            && !a.is_empty()
        {
            env.push(("ACTION".into(), a.to_string()));
        }

        let mut steps = Vec::new();
        if snapshot.join("upgrade.sh").is_file() {
            steps.push(ScriptStep {
                script: script_file(&snapshot, "upgrade", "upgrade.sh")?,
                env: env.clone(),
            });
        } else {
            // 缺省升级策略：先卸载（uninstall.sh 自带数据备份）再安装
            steps.push(ScriptStep {
                script: script_file(&snapshot, "uninstall", "uninstall.sh")?,
                env: env.clone(),
            });
            steps.push(ScriptStep {
                script: script_file(&snapshot, "install", "install.sh")?,
                env: env.clone(),
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
        let app_path = apps_dir().join(&pkg_path);
        let name = pkg_path.rsplit('/').next().unwrap_or("").to_string();
        let version = spec["version"].as_str().unwrap_or("").to_string();
        let action = spec["action"].as_str().unwrap_or("").to_string();
        let old_version = spec["old_version"].as_str().unwrap_or("").to_string();

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

        let mut steps = Vec::new();

        let done_run_id = new_run_id.clone();
        // 重跑复用原 run 的快照：成功后清理原快照防堆积；失败保留供继续编辑重试
        let done_old_run = run_id.clone();
        let done_app = app_path.clone();
        let on_done_extra: Option<Box<dyn FnOnce(i32) + Send>> = match kind {
            "uninstall" => {
                let script = script_file(&snapshot, "uninstall", "uninstall.sh")?;
                steps.push(ScriptStep { script, env });
                Some(Box::new(move |code: i32| {
                    if code == 0 {
                        let _ = std::fs::remove_dir_all(&done_app);
                    }
                    cleanup_snapshot(&done_old_run, code);
                }))
            }
            "upgrade" => {
                if snapshot.join("upgrade.sh").is_file() {
                    steps.push(ScriptStep {
                        script: script_file(&snapshot, "upgrade", "upgrade.sh")?,
                        env: env.clone(),
                    });
                } else {
                    steps.push(ScriptStep {
                        script: script_file(&snapshot, "uninstall", "uninstall.sh")?,
                        env: env.clone(),
                    });
                    steps.push(ScriptStep {
                        script: script_file(&snapshot, "install", "install.sh")?,
                        env: env.clone(),
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
                let script = script_file(&snapshot, "install", "install.sh")?;
                steps.push(ScriptStep { script, env });
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
        let log = spawn_background(
            &run_id,
            vec![ScriptStep {
                script: resolved,
                env: base_env(),
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

pub async fn installed() -> Response {
    tokio::task::spawn_blocking(move || {
        let mut items = Vec::new();
        let root = apps_dir();
        if let Ok(cats) = std::fs::read_dir(&root) {
            for cat in cats.flatten() {
                let cat_path = cat.path();
                if !cat_path.is_dir() {
                    continue;
                }
                let category = cat.file_name().to_string_lossy().to_string();
                if let Ok(pkgs) = std::fs::read_dir(&cat_path) {
                    for pkg in pkgs.flatten() {
                        let app_path = pkg.path();
                        if !app_path.is_dir() {
                            continue;
                        }
                        let Ok(meta) = read_meta(&app_path) else {
                            continue;
                        };
                        let name = pkg.file_name().to_string_lossy().to_string();
                        let pkg_path = format!("{category}/{name}");
                        let info = read_info_yaml(&app_path);
                        let state = probe_instance_state(&app_path, info.as_ref());
                        let instance = info
                            .as_ref()
                            .and_then(|i| i.get("instance").and_then(|v| v.as_str()))
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| name.clone());
                        items.push(json!({
                            "pkg_path": pkg_path,
                            "name": meta.name,
                            "version": meta.version,
                            "category": meta.category,
                            "source": meta.source,
                            "repo_id": meta.repo_id,
                            "installed_at": meta.installed_at,
                            "upgraded_from": meta.upgraded_from,
                            "run_id": meta.run_id,
                            "instance": instance,
                            "state": state,
                            "info": info.as_ref().map(yaml_to_json).unwrap_or_else(|| json!({})),
                        }));
                    }
                }
            }
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
        return match root_cmd("systemctl").args(["is-active", svc]).output() {
            Ok(o) => normalize_state(String::from_utf8_lossy(&o.stdout).trim()),
            Err(_) => "unknown".into(),
        };
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
pub async fn instance_action(pkg_path: String, action: String) -> Response {
    let allowed = ["start", "stop", "restart"];
    if !allowed.contains(&action.as_str()) {
        return Response::err(-1, format!("不支持的实例操作: {action}"));
    }
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (cat, name) = validate_pkg_path(&pkg_path)?;
        let app_path = apps_dir().join(&cat).join(&name);
        if !app_path.is_dir() {
            return Err("该应用未安装".into());
        }
        let info = read_info_yaml(&app_path).ok_or("缺少实例信息 info.yaml（由安装脚本登记）")?;
        let svc = info
            .get("svc_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or("未登记 systemd 服务（info.yaml 缺 svc_name），无法通过面板启停")?;
        let out = root_cmd("systemctl")
            .args([action.as_str(), svc.as_str()])
            .output()
            .map_err(|e| format!("执行 systemctl {action} {svc} 失败: {e}"))?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(format!("systemctl {action} {svc} 失败: {msg}"));
        }
        // 操作后回读状态（restart 稍等稳定）
        std::thread::sleep(std::time::Duration::from_millis(300));
        let state = match root_cmd("systemctl").args(["is-active", &svc]).output() {
            Ok(o) => normalize_state(String::from_utf8_lossy(&o.stdout).trim()),
            Err(_) => "unknown".into(),
        };
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

        let install = script_file(&pkg, "install", "install.sh").unwrap();
        assert_eq!(install.file_name().unwrap(), "setup.sh");
        // 未在 yaml 中定义的 key 回退到默认
        let upgrade = script_file(&pkg, "upgrade", "upgrade.sh").unwrap();
        assert_eq!(upgrade.file_name().unwrap(), "upgrade.sh");
        // 覆盖指向不存在的文件 → 报错
        let missing = script_file(&pkg, "install", "install.sh");
        assert!(missing.is_ok());
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
            id_from_url("https://github.com/zapj/zap-appstore.git"),
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
}
