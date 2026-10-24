// SPDX-License-Identifier: AGPL-3.0-only
//! 通用服务配置（root 执行）：「服务配置」大类下 Nginx 之外的各服务
//! （php / mysql(MySQL/MariaDB 合一，按实例自动识别) / docker …）的配置页后端能力。
//!
//! 设计目标：新增一个服务 = 在 `supported()` 中注册一条 `ServiceDef`，
//! 即获得「状态探测 / 启停控制 / 配置文件读写 / 关键项可视化」四件套，
//! 无需为每个服务重写 verb（Nginx 因校验/重载链复杂仍使用专用 verb nginx.rs）。
//!
//! 端点（均需管理员，见 zapd/system_service_conf.rs）：
//! - service_conf.status      状态探测（未安装时 installed=false）
//! - service_conf.list        列出可编辑配置（主配置 + 配置目录白名单）
//! - service_conf.read        读取指定配置内容
//! - service_conf.save        保存配置（备份 → 原子写入；json 校验）
//! - service_conf.keys        关键项表单定义 + 当前值
//! - service_conf.keys_save   关键项保存（ini 托管块 / json 键合并）
//! - service_conf.control     服务控制 start / stop / restart / reload
//!
//! 安全边界：文件路径必须位于该服务主配置所在目录树内且扩展名白名单；
//! 不执行任何用户输入命令，只运行服务定义内置的命令模板。

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde_json::{Value, json};
use zap_proto::Response;

use super::root_cmd;
use super::svc::{
    Manager, OPENRC_INIT_DIR, OPENRC_UPDATE, SYSTEMD_CTL, act_m, exists_m, is_active_m,
};

/// 单个配置文件体积上限（读 / 写通用）。
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// 从主配置目录树内收集文件的最大深度。
const SCAN_DEPTH: usize = 2;
/// 服务控制允许的动作。
const ALLOWED_ACTIONS: &[&str] = &["start", "stop", "restart", "reload"];
/// 列表型关键项（`FieldKind::List`）的条目数上限。
const MAX_LIST_ITEMS: usize = 20;
/// 列表型关键项单项的字符数上限。
const MAX_ITEM_LEN: usize = 512;

// ── 服务定义 ─────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ConfFormat {
    /// ini 风格：key = value（php.ini 无 section / my.cnf 有 [section]）
    Ini,
    /// JSON 对象（docker daemon.json）
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum FieldKind {
    Text,
    Number,
    Select,
    Bool,
    /// 多行列表：配置里是数组（如 docker 的 `registry-mirrors`），表单里一行一项
    List,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct FieldDef {
    key: String,
    label: String,
    kind: FieldKind,
    #[serde(default)]
    help: String,
    /// ini：写入的 section（None = 文件层/无 section）
    #[serde(default)]
    section: Option<String>,
    /// json：逐级键路径（如 ["log-driver"] / ["log-opts","max-size"]）
    #[serde(default)]
    jpath: Vec<String>,
    #[serde(default)]
    options: Vec<String>,
}

/// 服务定义来源：内置（随包发布）还是管理员自建（落在 `ZAP_SERVICE_CONF_DIR`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize)]
pub(crate) enum DefOrigin {
    #[default]
    Builtin,
    User,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ServiceDef {
    key: String,
    label: String,
    /// 定义文件里登记的服务版本（如 `version: "1.0.0"`）；仅用于展示。
    #[serde(default)]
    version: String,
    /// 服务管理器类型：决定启停 / 自启走哪套实现（systemd / openrc …）。
    /// 省略时按当前系统实际 init 自动识别（见 `default_manager`）。
    #[serde(default = "default_manager")]
    manager: Manager,
    /// systemd unit 名候选（依次取第一个存在的）
    #[serde(default)]
    unit_candidates: Vec<String>,
    /// 探测二进制的可执行文件名候选
    #[serde(default)]
    bin_candidates: Vec<String>,
    /// 取版本时二进制后追加的参数（留空 = `--version`）
    #[serde(default)]
    version_args: Vec<String>,
    /// 版本输出落在 stderr（如 nginx -v；本模块服务多为 stdout）
    #[serde(default)]
    version_in_stderr: bool,
    /// 主配置文件候选（可含一个 `*`，用于版本目录），依次取第一个存在的；
    /// 全部不存在但服务已安装时取首个不含通配的候选（允许从 UI 新建）
    #[serde(default)]
    main_candidates: Vec<String>,
    /// 显式配置文件列表：自定义服务可填写多个（一行一个路径），首个作为主配置；
    /// 列出的文件均可直接编辑（无需 bin / unit 已存在即可编辑）。
    /// 兼容 `main: "/a"`（单值）与 `main: ["/a","/b"]`（多文件）两种写法。
    #[serde(default, deserialize_with = "de_one_or_many")]
    main: Vec<String>,
    /// 可编辑文件的扩展名白名单（不含点）
    #[serde(default)]
    exts: Vec<String>,
    format: ConfFormat,
    /// ini 注释符（my.cnf 用 #，php.ini 用 ;）
    #[serde(default)]
    ini_comment: String,
    #[serde(default)]
    fields: Vec<FieldDef>,
    /// 可选：由面板**自动生成并启用**的单元文件模板（systemd 的 .service /
    /// openrc 的 init.d）。存在该模板即表示管理员希望面板代为落地自启文件，
    /// 经 `service_conf.unit_write` 渲染写入系统目录并 enable。
    #[serde(default)]
    unit: Option<UnitTemplate>,
    /// 来源：内置或被管理员覆盖 / 自建（不来自 yaml，由加载逻辑填写）。
    #[serde(skip)]
    pub(crate) origin: DefOrigin,
}

/// 自动生成服务单元所需的模板字段（结构化，避免让管理员手写整段文件造成注入）。
/// 渲染为 systemd `[Service]` 单元；openrc 暂未实现自动生成。
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
struct UnitTemplate {
    /// Description=（缺省取 def.label）
    #[serde(default)]
    description: Option<String>,
    /// After=（启动顺序依赖的单元名列表）
    #[serde(default)]
    after: Vec<String>,
    /// Wants=
    #[serde(default)]
    wants: Vec<String>,
    /// Requires=
    #[serde(default)]
    requires: Vec<String>,
    /// ExecStart=（systemd 必填）
    #[serde(default)]
    exec_start: Option<String>,
    /// WorkingDirectory=（绝对路径，不含 ..）
    #[serde(default)]
    working_dir: Option<String>,
    /// User=
    #[serde(default)]
    user: Option<String>,
    /// Group=
    #[serde(default)]
    group: Option<String>,
    /// 环境变量列表，每项 "KEY=VALUE"，渲染为多条 Environment=
    #[serde(default)]
    environment: Vec<String>,
    /// 启动前命令（systemd 的 ExecStartPre= / openrc 的 precmd()），一行一条
    #[serde(default)]
    precmd: Vec<String>,
    /// Restart=（no/always/on-failure…）
    #[serde(default)]
    restart: Option<String>,
    /// RestartSec=（秒）
    #[serde(default)]
    restart_sec: Option<u64>,
    /// Type=（simple/exec/forking/oneshot/notify/dbus/idle；缺省 simple）
    #[serde(default, rename = "type")]
    type_: Option<String>,
    /// 自定义附加指令：每行 "Key=Value"，原样注入生成的 systemd 单元
    /// （`StartLimitIntervalSec`/`StartLimitBurst`/`StartLimitBurstSec` 归 [Unit]，其余归 [Service]）。
    /// openrc 不使用该字段。
    #[serde(default)]
    extra: Vec<String>,
    /// StartLimitIntervalSec=（秒，[Unit] 段重启限流窗口；配合 Restart=on-failure）
    #[serde(default)]
    start_limit_interval_sec: Option<u64>,
    /// StartLimitBurst=（[Unit] 段限流窗口内允许的重启次数）
    #[serde(default)]
    start_limit_burst: Option<u32>,
}

// ── 服务定义加载 ──────────────────────────────────────────────
//
// 服务不再写死在代码里：内置定义随包发布（`data/services/*.yaml`，编译进
// 二进制），运行时 `/etc/zap/services/*.yaml` 按 key 覆盖或追加（新增服务
// 丢一个文件进去就有「配置读写 / 关键项 / 启停」三件套）。目录可用
// ZAP_SERVICE_CONF_DIR 覆盖。解析失败的文件跳过并打日志，不影响其它服务。

/// 内置定义（编译期嵌入，随包发布）
const BUILTIN_DEFS: &[&str] = &[
    include_str!("../../../data/services/php.yaml"),
    include_str!("../../../data/services/mysql.yaml"),
    include_str!("../../../data/services/docker.yaml"),
];

/// 运行时覆盖目录
const OVERRIDE_DIR: &str = "/etc/zap/services";

/// 全部服务定义（内置 + 运行时覆盖）。用读写锁包成 Arc，便于管理员新增 / 删除
/// 自定义服务后**免重启**立即生效。
static DEFS: std::sync::OnceLock<RwLock<Arc<Vec<ServiceDef>>>> = std::sync::OnceLock::new();

/// 覆盖目录最近一次 mtime：用于无侵入地探测「有人往 `/etc/zap/services` 丢/改/删了
/// 定义文件」，从而自动重新扫描，免去手动 reload 或重启守护进程。
static OVERRIDE_MTIME: std::sync::OnceLock<RwLock<Option<std::time::SystemTime>>> =
    std::sync::OnceLock::new();

/// 取定义快照（拷贝 Arc 指针，廉价）。
///
/// 若覆盖目录的 mtime 自上次以来变了（手动丢文件、改文件、删文件），先重新扫描磁盘，
/// 保证定义清单始终与 `/etc/zap/services` 实际内容一致——不依赖进程内 `reload_defs`。
fn defs() -> Arc<Vec<ServiceDef>> {
    if let Some(mt) = std::fs::metadata(override_dir()).ok().and_then(|m| m.modified().ok()) {
        let last = OVERRIDE_MTIME.get_or_init(|| RwLock::new(None));
        if let (Ok(mut g), Ok(mut w)) = (last.write(), defs_lock().write()) {
            if *g != Some(mt) {
                *w = Arc::new(load_defs());
                *g = Some(mt);
            }
        }
    }
    defs_lock().read().unwrap().clone()
}

/// 读写锁（懒初始化为首次加载结果）。
fn defs_lock() -> &'static RwLock<Arc<Vec<ServiceDef>>> {
    DEFS.get_or_init(|| RwLock::new(Arc::new(load_defs())))
}

/// 重新扫描内置 + 覆盖目录，替换内存中的定义快照（新增 / 删除自定义服务后调用）。
/// 另外 `defs()` 也会按覆盖目录 mtime 自动重新扫描，所以手动改动文件同样即时生效。
///
/// 加锁顺序与 `defs()` 保持一致（先 `OVERRIDE_MTIME` 后 `defs_lock`），避免反向死锁。
pub(crate) fn reload_defs() {
    let mt = std::fs::metadata(override_dir()).ok().and_then(|m| m.modified().ok());
    let last = OVERRIDE_MTIME.get_or_init(|| RwLock::new(None));
    if let (Ok(mut g), Ok(mut w)) = (last.write(), defs_lock().write()) {
        *w = Arc::new(load_defs());
        *g = mt;
    }
}

/// 服务定义在 YAML 里省略 `manager` 时的缺省值：按当前系统实际 init 自动识别。
/// 供 `serde(default = "default_manager")` 调用，使新建自定义服务无需手动选管理器。
fn default_manager() -> Manager {
    super::svc::active_manager()
}

/// 兼容 `main: "/a"`（单值）与 `main: ["/a","/b"]`（多文件列表）两种写法。
fn de_one_or_many<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    let v: Option<OneOrMany> = serde::Deserialize::deserialize(de)?;
    Ok(match v {
        None => Vec::new(),
        Some(OneOrMany::One(s)) => vec![s],
        Some(OneOrMany::Many(list)) => list,
    })
}

/// 覆盖目录：默认 `/etc/zap/services`，可用 `ZAP_SERVICE_CONF_DIR` 覆盖。
/// 与 [`load_defs`] 保持一致，管理员 CRUD 也写这里。
pub(crate) fn override_dir() -> PathBuf {
    std::env::var("ZAP_SERVICE_CONF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(OVERRIDE_DIR))
}

fn load_defs() -> Vec<ServiceDef> {
    let mut out: Vec<ServiceDef> = Vec::new();
    for raw in BUILTIN_DEFS {
        push_def(&mut out, raw, DefOrigin::Builtin);
    }
    let dir = override_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e == "yaml" || e == "yml")
            })
            .collect();
        files.sort();
        for f in files {
            if let Ok(raw) = std::fs::read_to_string(&f) {
                push_def(&mut out, &raw, DefOrigin::User);
            }
        }
    }
    out
}

/// 解析一份定义：同 key 覆盖，新 key 追加；坏文件跳过（不拖垮其它服务）。
fn push_def(out: &mut Vec<ServiceDef>, raw: &str, origin: DefOrigin) {
    match serde_yaml::from_str::<ServiceDef>(raw) {
        Ok(mut def) => {
            def.origin = origin;
            match out.iter_mut().find(|d| d.key == def.key) {
                Some(existing) => *existing = def,
                None => out.push(def),
            }
        }
        Err(e) => eprintln!("[service-conf] 服务定义（{origin:?}）解析失败，已跳过: {e}"),
    }
}

/// 自定义服务（origin=user）当前可管控的 systemd / openrc unit 集合。
///
/// 供 `services` 总览的启停 / 开机自启白名单复用：自定义服务在总览里「看得见」也要
/// 「点得动」，否则只展示却不能管理反而更困惑。未检测到（没有任何候选 unit 命中、
/// 且 key 本身也不是已存在的 unit）的服务不会进入此集合，从而保持安全边界。
pub(crate) fn custom_active_units() -> BTreeSet<String> {
    defs()
        .iter()
        .filter(|d| d.origin == DefOrigin::User)
        .filter_map(|d| active_unit(d))
        .collect()
}

/// 自定义服务总览摘要（给 `services.overview` 用）：返回
/// `(key, 显示名, 用作 svc 的 unit 名或退化为 key, 是否已检测到 unit, 定义里登记的版本)`。
pub(crate) fn custom_def_summaries() -> Vec<(String, String, String, bool, String)> {
    defs()
        .iter()
        .filter(|d| d.origin == DefOrigin::User)
        .map(|d| {
            let u = active_unit(d);
            let detected = u.is_some();
            (
                d.key.clone(),
                d.label.clone(),
                u.unwrap_or_else(|| d.key.clone()),
                detected,
                d.version.clone(),
            )
        })
        .collect()
}

/// 服务注册表：内置定义 + `/etc/zap/services/*.yaml`。
/// php<版本号>（php74 / php81 …）作为 PHP 实例 svc 一并识别，复用 php 定义的
/// 字段 / 白名单 / 关键项，探测与路径在 `php_inst` / `installed_info` 覆写。
///
/// 返回 owned（`ServiceDef` 较小），调用方无需持有缓存锁；新增 / 删除后快照可热更新。
fn supported(key: &str) -> Option<ServiceDef> {
    let target = if php_inst_svc(key).is_some() {
        "php"
    } else {
        key
    };
    defs().iter().find(|d| d.key == target).cloned()
}

/// PHP 实例短名识别：svc = "php" + 版本号数字（php74 / php81 …，
/// 对应应用商店实例安装目录 `{ZAP_APPS_DIR}/php-74` 与 unit
/// `php-fpm-74`）。多版本实例的配置路径/unit 与类型级 "php" 不同。
fn php_inst_svc(svc: &str) -> Option<String> {
    // 容忍 `php74` 与 `php-74` 两种写法（svc 规范为前者，后者仅作兼容）
    let rest = svc.strip_prefix("php")?.trim_start_matches('-');
    if rest.is_empty() || rest.len() > 3 || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(rest.to_string())
}

// ── 探测工具 ─────────────────────────────────────────────────

fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

fn quote_shell(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `command -v name` 找到可执行文件。
fn which(name: &str) -> Option<PathBuf> {
    let o = root_cmd(super::platform::SHELL)
        .args(["-c"])
        .arg(format!("command -v {} 2>/dev/null", quote_shell(name)))
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if p.is_empty() {
        return None;
    }
    Some(PathBuf::from(p))
}

/// 依次探测二进制：PATH 内 → 常见安装前缀（覆盖 /usr/local/apps 型安装）。
fn find_bin(d: &ServiceDef) -> Option<PathBuf> {
    for name in &d.bin_candidates {
        if let Some(p) = which(name) {
            return Some(p);
        }
    }
    // 安装根（默认 /usr/local/apps，ZAP_APPS_DIR 可覆盖）下常见位置
    let install_root = super::install_root();
    for name in &d.bin_candidates {
        for prefix in [
            install_root.clone(),
            install_root.join(&d.key),
            PathBuf::from(format!("/usr/local/{}", d.key)),
        ] {
            for sub in ["bin", "sbin"] {
                let p = prefix.join(sub).join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// 取版本号（首个二进制 + 固定参数的首行输出）。
fn detect_version(d: &ServiceDef, bin: &Path) -> String {
    let mut cmd = root_cmd(
        bin.to_str()
            .unwrap_or(bin.file_name().and_then(|n| n.to_str()).unwrap_or("")),
    );
    for a in &d.version_args {
        cmd.arg(a);
    }
    if d.version_args.is_empty() {
        cmd.arg("--version");
    }
    let o = cmd.output();
    let text = match o {
        Ok(o) => {
            let out = if d.version_in_stderr || o.stdout.is_empty() {
                String::from_utf8_lossy(&o.stderr)
            } else {
                String::from_utf8_lossy(&o.stdout)
            };
            out.trim().to_string()
        }
        Err(_) => String::new(),
    };
    text.lines().next().unwrap_or("").trim().to_string()
}

/// 主配置探测：候选含一个 `*` 时按 glob 展开（多命中取版本号最大者）；
/// 否则取第一个存在的文件；全部不存在时回退首个普通候选（允许从 UI 新建）。
fn probe_main(d: &ServiceDef) -> Option<(PathBuf, PathBuf, bool)> {
    let mut first_plain: Option<&str> = None;
    for cand in &d.main_candidates {
        if cand.contains('*') {
            if let Some(p) = glob_first(cand) {
                return Some((
                    p.clone(),
                    p.parent().unwrap_or(Path::new("/")).to_path_buf(),
                    true,
                ));
            }
            continue;
        }
        if first_plain.is_none() {
            first_plain = Some(cand);
        }
        let p = PathBuf::from(cand);
        if p.is_file() {
            return Some((
                p.clone(),
                p.parent().unwrap_or(Path::new("/")).to_path_buf(),
                true,
            ));
        }
    }
    // 全部不存在：服务已安装时允许回退到首个不含通配的候选（UI 可新建）
    if let Some(cand) = first_plain {
        let p = PathBuf::from(cand);
        if !cand.contains('*') {
            let dir = p.parent().unwrap_or(Path::new("/")).to_path_buf();
            return Some((p, dir, false));
        }
    }
    None
}

/// 支持单个 `*` 的极简 glob，两种模式：
///
/// - **目录名通配**（`*` 后紧跟路径分隔符，如 `/etc/php/*/fpm/php.ini`）：
///   在 `/etc/php` 下遍历版本目录（7.4 / 8.1 / 8.2 …），取 `<版本目录>/fpm/php.ini`。
/// - **文件名通配**（`*` 位于文件名中，如 `/etc/app/*.conf`、`/etc/app/pre*fix.conf`）：
///   匹配该目录下名字符合 `前缀*后缀` 的文件。
///
/// 兼容 `*` 前带目录名前缀的写法（如 `/etc/php-*/x.ini`）：前缀参与过滤。
/// 仅支持**一个** `*`；多个 `*` 时只展开第一个，其余按字面处理。
fn glob_matches(pattern: &str) -> Vec<(PathBuf, String)> {
    let Some((head, raw_tail)) = pattern.split_once('*') else {
        return vec![];
    };
    let (dir, prefix): (PathBuf, String) = if head.ends_with('/') {
        (PathBuf::from(head), String::new())
    } else {
        let base = Path::new(head);
        (
            base.parent().unwrap_or(Path::new("/")).to_path_buf(),
            base.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
        )
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    // `*` 后以 `/` 开头 → `*` 匹配一层目录名；否则 `*` 属于文件名 → 文件名通配
    let dir_mode = raw_tail.starts_with('/');
    let mut hits: Vec<(PathBuf, String)> = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) {
            continue;
        }
        if dir_mode {
            if !path.is_dir() {
                continue;
            }
            let target = path.join(raw_tail.trim_start_matches('/'));
            if target.is_file() {
                hits.push((target, name));
            }
        } else {
            if !path.is_file() {
                continue;
            }
            // 文件名通配：前缀（prefix）+ `*` + 后缀（raw_tail）
            if name.ends_with(raw_tail) {
                hits.push((path, name));
            }
        }
    }
    hits
}

/// 取通配命中的**版本号最大**的一个（用于主配置探测）；无命中返回 `None`。
fn glob_first(pattern: &str) -> Option<PathBuf> {
    // 版本号最大的优先；无法解析为数字的名字退化为字典序
    let mut hits = glob_matches(pattern);
    hits.sort_by(|a, b| version_key(&a.1).cmp(&version_key(&b.1)));
    hits.pop().map(|(p, _)| p)
}

/// 返回**所有**命中的文件（用于自定义服务 `main` 里的通配配置列表展开）。
fn glob_all(pattern: &str) -> Vec<PathBuf> {
    glob_matches(pattern)
        .into_iter()
        .map(|(p, _)| p)
        .collect()
}

/// 兜底探测：用 `php --ini` 自报的 `Loaded Configuration File` 定位 php.ini。
///
/// 静态候选无法覆盖所有布局（Remi 源 `/etc/opt/remi/php81`、源码安装、
/// 应用商店装到 `/usr/local/apps/php-74` 却未被识别为实例等），
/// 而 `php --ini` 是权威答案——它直接给出该二进制实际加载的 ini。
/// 仅在静态候选全部落空时使用，避免与「优先 fpm 配置」的顺序冲突。
fn php_ini_from_bin(bin: &Path) -> Option<PathBuf> {
    let o = root_cmd(bin.to_str()?).args(["--ini"]).output().ok()?;
    let out = String::from_utf8_lossy(&o.stdout);

    let mut loaded: Option<PathBuf> = None;
    let mut conf_dir: Option<PathBuf> = None;
    for line in out.lines() {
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let val = val.trim();
        if val.is_empty() {
            continue;
        }
        match key.trim() {
            "Loaded Configuration File" if val != "(none)" => loaded = Some(PathBuf::from(val)),
            "Configuration File (php.ini) Path" => conf_dir = Some(PathBuf::from(val)),
            _ => {}
        }
    }

    // 已加载：直接返回（即使文件被删也返回原名，由调用方标 exists=false 提示"缺少"）
    // 未加载（(none)）：返回"预期路径"= 配置目录/php.ini，UI 提示可保存创建
    loaded.or_else(|| conf_dir.map(|d| d.join("php.ini")))
}

/// 系统级配置目录：主配置直接落在这里时**只放通 `.ini`**。
///
/// 否则把 `.conf` 也放开会扫出大量与 PHP 无关的文件（/etc/sysctl.conf、
/// /etc/security/*.conf …），等于把整个 /etc 暴露给在线编辑器。
const SYSTEM_CONF_DIRS: &[&str] = &["/", "/etc", "/usr/local/etc", "/etc/opt"];

/// PHP 可编辑扩展名：按主配置所在目录收窄。
///
/// - 应用商店实例（`/usr/local/apps/php-74/etc`）或版本目录（`/etc/php/8.1/fpm`）：
///   放通 `php-fpm.conf` 与 `php-fpm.d/*.conf`（FPM 池配置确实需要改）；
/// - 主配置直接在 /etc 等系统目录（`/etc/php.ini`）：仅 `.ini`。
fn service_exts<'a>(d: &'a ServiceDef, root: &Path) -> Vec<&'a str> {
    if d.key != "php" {
        return d.exts.iter().map(|s| s.as_str()).collect();
    }
    let p = root.to_string_lossy().to_string();
    if SYSTEM_CONF_DIRS.iter().any(|x| p == *x) {
        vec!["ini"]
    } else {
        vec!["ini", "conf"]
    }
}

/// 目录名 → 可比较的版本键：`8.1` → `(8, 1, "8.1")`，非数字 → `(0, 0, name)`。
fn version_key(name: &str) -> (u64, u64, &str) {
    let mut parts = name.split(['.', '-', '_']);
    let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, name)
}

/// 通过 /proc 扫描判断是否存在 cmdline 包含二进制名的进程。
fn running_by_proc(bin: &Path) -> bool {
    let needle = bin
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    if needle.is_empty() {
        return false;
    }
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return false;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
            continue;
        };
        let cmdline = std::fs::read_to_string(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        if cmdline.contains(&needle) {
            return true;
        }
    }
    false
}

/// PHP 实例探测上下文：由 svc（php74）定位到安装目录。
pub(crate) struct PhpInst {
    pub(crate) digits: String,
    pub(crate) dir: PathBuf,
}

/// 实例目录名候选：`php-74` 与 `php74` 都接受（历史两种写法都出现过）。
fn inst_dir_names(digits: &str) -> [String; 2] {
    [format!("php-{digits}"), format!("php{digits}")]
}

/// 应用商店登记的 PHP 实例（authoritative）：`instance` / `install_dir` / `config_file` / `svc_name`。
struct PhpReg {
    svc: String,
    dir: PathBuf,
    conf: Option<PathBuf>,
    unit: Option<String>,
}

/// 读取全部 PHP 实例登记项（instance 形如 php74 / php81）。
fn php_registrations() -> Vec<PhpReg> {
    super::appstore::registered_apps()
        .into_iter()
        .filter(|a| php_inst_svc(&a.instance).is_some())
        .filter_map(|a| {
            let digits = php_inst_svc(&a.instance)?;
            let dir = match a.install_dir.filter(|d| d.is_dir()) {
                Some(d) => d,
                None => inst_dir_names(&digits)
                    .iter()
                    .map(|n| super::install_root().join(n))
                    .find(|d| d.is_dir())?,
            };
            Some(PhpReg {
                svc: a.instance,
                dir,
                conf: a.config_file,
                unit: a.svc_name,
            })
        })
        .collect()
}

fn php_reg_of(svc: &str) -> Option<PhpReg> {
    php_registrations().into_iter().find(|r| r.svc == svc)
}

/// PHP 实例解析：svc=php74 → **优先**取 info.yaml 登记的 install_dir，
/// 兜底按目录名 `php-74` / `php74` 在应用安装根下查找（不存在返回 None）。
pub(crate) fn php_inst(svc: &str) -> Option<PhpInst> {
    let digits = php_inst_svc(svc)?;
    if let Some(r) = php_reg_of(svc) {
        return Some(PhpInst { digits, dir: r.dir });
    }
    inst_dir_names(&digits)
        .iter()
        .map(|n| super::install_root().join(n))
        .find(|d| d.is_dir())
        .map(|dir| PhpInst { digits, dir })
}

/// 候选探测：取第一个存在的 systemd unit 名（无 .service 后缀）。
fn active_unit(d: &ServiceDef) -> Option<String> {
    // 优先按 unit_candidates 探测（内置 / 外部服务的真实单元名）
    if let Some(u) = d
        .unit_candidates
        .iter()
        .find(|u| exists_m(d.manager, u))
    {
        return Some(u.to_string());
    }
    // 兜底：本服务定义的 key 自身即为单元名（由面板生成并落盘的 <key>.service）
    exists_m(d.manager, &d.key).then_some(d.key.clone())
}

/// 实际使用的 systemd unit 名（无 .service 后缀）。
/// PHP 实例 svc 使用实例 unit `php-fpm-<ver>`；其余走候选探测。
fn effective_unit(d: &ServiceDef, svc: &str) -> Option<String> {
    if let Some(inst) = php_inst(svc) {
        // 优先 info.yaml 登记的 svc_name，兜底 php-fpm-<版本>
        if let Some(u) = php_reg_of(svc).and_then(|r| r.unit)
            && exists_m(d.manager, &u)
        {
            return Some(u);
        }
        let u = format!("php-fpm-{}", inst.digits);
        return exists_m(d.manager, &u).then_some(u);
    }
    active_unit(d)
}

/// 服务当前运行态。
fn service_running(d: &ServiceDef, svc: &str, bin: Option<&Path>) -> bool {
    if let Some(unit) = effective_unit(d, svc) {
        if is_active_m(d.manager, &unit) {
            return true;
        }
        // unit 存在但未 active 即未运行（不继续看进程，避免误判）
        return false;
    }
    if let Some(bin) = bin {
        return running_by_proc(bin);
    }
    false
}

// ── 文件操作 ─────────────────────────────────────────────────

/// 服务配置备份目录：`{ZAP_PATH}/data/backups/<svc>`（svc 为实例名，如
/// php74 / php81 / mysql 等）。统一约定：nginx 与各服务应用的备份共用
/// `data/backups/` 根目录，每服务一个子目录，便于统一浏览与容量管理。
fn backup_dir(svc: &str) -> PathBuf {
    zap_path().join("data/backups").join(svc)
}

fn backup_file(svc: &str, path: &Path) -> Result<PathBuf, String> {
    let dir = backup_dir(svc);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建备份目录失败: {e}"))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("conf");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = dir.join(format!("{ts}-{name}.bak"));
    if path.is_file() {
        std::fs::copy(path, &dest).map_err(|e| format!("备份失败: {e}"))?;
    }
    let mut kept: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    kept.sort();
    while kept.len() > 20 {
        if let Some(old) = kept.first() {
            let _ = std::fs::remove_file(old);
            kept.remove(0);
        }
    }
    Ok(dest)
}

/// 校验待编辑路径：必须位于主配置目录树内且扩展名白名单。
#[allow(clippy::too_many_arguments)]
fn validate_path(
    d: &ServiceDef,
    exts: &[&str],
    main: &Path,
    root: &Path,
    raw: &str,
) -> Result<(PathBuf, bool), String> {
    let main_canon = main.canonicalize().unwrap_or_else(|_| main.to_path_buf());
    let p = PathBuf::from(raw);
    let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
    // 显式登记的自定义配置（main 列表，含 * 通配展开）视为白名单路径，不受目录/扩展名限制
    let mut whitelist: Vec<PathBuf> = Vec::new();
    for m in &d.main {
        if m.contains('*') {
            whitelist.extend(glob_all(m));
        } else {
            whitelist.push(PathBuf::from(m));
        }
    }
    let explicit_pos = whitelist.iter().position(|mp| {
        mp.canonicalize().unwrap_or_else(|_| mp.clone()) == canon
    });
    let is_main = canon == main_canon || explicit_pos == Some(0);
    if explicit_pos.is_none() {
        if !canon.starts_with(root) {
            return Err(format!(
                "仅允许编辑 {} 主配置目录（{}）内的配置文件",
                d.label,
                root.display()
            ));
        }
        let fname = canon
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let ok_ext = exts.iter().any(|e| fname.ends_with(&format!(".{e}")));
        if !ok_ext {
            return Err(format!(
                "仅支持编辑 {}",
                exts.iter()
                    .map(|e| format!(".{e}"))
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        }
        if fname.starts_with("zap-")
            || fname.ends_with(".bak")
            || fname.ends_with(".tmp")
            || fname.contains(".zap")
        {
            return Err("该文件为面板托管/备份文件，请勿直接编辑".to_string());
        }
    }
    Ok((canon, is_main))
}

fn file_entry(path: &Path, base: &Path, is_main: bool, size: u64, exists: bool) -> Value {
    let rel = path
        .strip_prefix(base)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string());
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    json!({
        "path": path.display().to_string(),
        "rel": rel,
        "name": path.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
        "is_main": is_main,
        "size": size,
        "mtime": mtime,
        "exists": exists,
    })
}

fn collect_files(exts: &[&str], root: &Path, out: &mut Vec<Value>) {
    fn walk(exts: &[&str], dir: &Path, base: &Path, depth: usize, out: &mut Vec<Value>) {
        if depth > SCAN_DEPTH {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                !p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            })
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(exts, &p, base, depth + 1, out);
                continue;
            }
            let fname = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let ok_ext = exts.iter().any(|e| fname.ends_with(&format!(".{e}")));
            if !ok_ext
                || fname.starts_with("zap-")
                || fname.ends_with(".bak")
                || fname.ends_with(".tmp")
                || fname.contains(".zap")
            {
                continue;
            }
            if let Ok(meta) = std::fs::metadata(&p) {
                if meta.len() > MAX_FILE_BYTES {
                    continue;
                }
                out.push(file_entry(&p, base, false, meta.len(), true));
            }
        }
    }
    walk(exts, root, root, 0, out);
}

// ── ini 关键项托管 ───────────────────────────────────────────

/// 生成托管块起始 / 结束标记行。
fn marker_begin(comment: &str, svc: &str) -> String {
    format!("{comment} ==== zap-managed ({svc}) begin ====")
}
fn marker_end(comment: &str, svc: &str) -> String {
    format!("{comment} ==== zap-managed ({svc}) end ====")
}

/// 从文本中剔除旧的托管块（含标记行），返回清理后的文本。
fn strip_managed_block(content: &str, comment: &str, svc: &str) -> String {
    let begin = marker_begin(comment, svc);
    let end = marker_end(comment, svc);
    let mut out = String::new();
    let mut skipping = false;
    for line in content.lines() {
        if line.trim_start().starts_with(&begin) {
            skipping = true;
            continue;
        }
        if skipping && line.trim_start().starts_with(&end) {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// ini 块内容行（key = value）。注释符与 `strip_managed_block` / 读取逻辑保持一致。
fn block_lines(comment: &str, svc: &str, entries: &[(&str, String)]) -> String {
    let mut s = String::new();
    s.push_str(&marker_begin(comment, svc));
    s.push('\n');
    for (k, v) in entries {
        s.push_str(&format!("{k} = {v}\n"));
    }
    s.push_str(&marker_end(comment, svc));
    s.push('\n');
    s
}

/// 把托管块插入到指定 [section] 内的末尾（该 section 内后续出现的同键才会覆盖托管值，
/// 因此放在 section 末尾以尽量保证托管值生效）。
fn insert_block(content: &str, section: Option<&str>, block: &str) -> String {
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    if let Some(sec) = section {
        let header = format!("[{sec}]");
        // 定位 section 起始行
        let start = lines
            .iter()
            .position(|l| l.trim().eq_ignore_ascii_case(&header));
        let Some(start) = start else {
            // 无该 section：末尾补 section + 块
            let mut s = String::new();
            if !content.is_empty() {
                s.push('\n');
            }
            s.push_str(&header);
            s.push('\n');
            s.push_str(block);
            return s;
        };
        // 该 section 内末尾 = 下一个 [ 起始行前
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.trim_start().starts_with('['));
        let insert_at = match end {
            Some(rel) => start + 1 + rel,
            None => lines.len(),
        };
        // 清理 section 内部与末尾的空行，避免插入位置前堆积空行
        let block_owned: Vec<String> = block.lines().map(|l| l.to_string()).collect();
        lines.splice(insert_at..insert_at, block_owned);
        let mut out = lines.join("\n");
        out.push('\n');
        return out;
    }
    // 无 section（php.ini 等）：整体追加到文件末尾
    let mut out = String::new();
    if !content.is_empty() {
        out.push_str(content.trim_end());
        out.push('\n');
    }
    out.push_str(block);
    out
}

/// 从 ini 文本中读取 key 值（优先托管块内，其次全文最后一次未注释赋值）。
fn ini_read_values(
    content: &str,
    comment: &str,
    svc: &str,
    fields: &[FieldDef],
) -> std::collections::BTreeMap<String, Value> {
    let mut values = std::collections::BTreeMap::new();
    // 1) 先取托管块内赋值
    let begin = marker_begin(comment, svc);
    let end = marker_end(comment, svc);
    let mut in_block = false;
    let mut in_block_values: std::collections::BTreeMap<String, String> = Default::default();
    for line in content.lines() {
        let t = line.trim_start();
        if t.starts_with(&begin) {
            in_block = true;
            continue;
        }
        if in_block && t.starts_with(&end) {
            in_block = false;
            continue;
        }
        if in_block && let Some((k, v)) = parse_ini_line(t) {
            in_block_values.insert(k, v);
        }
    }
    // 2) 全文扫描未注释赋值，记最后出现的值作为回退
    let mut fallback: std::collections::BTreeMap<String, String> = Default::default();
    for line in content.lines() {
        let t = line.trim_start();
        if t.is_empty() || t.starts_with(comment) || t.starts_with('#') || t.starts_with(';') {
            continue;
        }
        if let Some((k, v)) = parse_ini_line(t) {
            fallback.insert(k, v);
        }
    }
    for f in fields {
        let v = in_block_values
            .get(f.key.as_str())
            .or_else(|| fallback.get(f.key.as_str()));
        values.insert(
            f.key.to_string(),
            v.cloned().map_or(Value::Null, Value::String),
        );
    }
    values
}

fn parse_ini_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    // key[空格]*=[空格]*value
    let eq = line.find('=')?;
    let key = line[..eq].trim().to_string();
    if key.is_empty() {
        return None;
    }
    let val = line[eq + 1..]
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string();
    Some((key, val))
}

// ── json 关键项 ──────────────────────────────────────────────

fn json_read_values(
    content: &str,
    fields: &[FieldDef],
) -> std::collections::BTreeMap<String, Value> {
    let mut values = std::collections::BTreeMap::new();
    let parsed: Value = serde_json::from_str(content.trim()).unwrap_or(Value::Null);
    for f in fields {
        let mut cur = &parsed;
        let mut found = true;
        for seg in &f.jpath {
            match cur.get(seg.as_str()) {
                Some(v) => cur = v,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if found && !cur.is_null() {
            values.insert(f.key.to_string(), json_value_to_text(cur));
        } else {
            values.insert(f.key.to_string(), Value::Null);
        }
    }
    values
}

/// 关键项取值统一摊平成表单里的字符串：数组（registry-mirrors 之类）一行一项，
/// 标量与嵌套对象原样保留，交给前端按 kind 渲染。
fn json_value_to_text(v: &Value) -> Value {
    let Value::Array(items) = v else {
        return v.clone();
    };
    let lines: Vec<String> = items
        .iter()
        .map(|i| match i {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect();
    Value::String(lines.join("\n"))
}

/// 表单里的一行一项 → 条目列表（同时吃换行和逗号两种分隔）。
///
/// 去空白行、去每项首尾空格；超过 `MAX_LIST_ITEMS` 条直接报错，避免把配置文件撑爆。
fn parse_list_items(raw: &str) -> Result<Vec<String>, String> {
    let items: Vec<String> = raw
        .split(['\n', ','])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        return Ok(items);
    }
    if items.len() > MAX_LIST_ITEMS {
        return Err(format!("最多 {MAX_LIST_ITEMS} 项，当前 {} 项", items.len()));
    }
    for it in &items {
        if it.len() > MAX_ITEM_LEN {
            return Err(format!("单项长度不能超过 {MAX_ITEM_LEN} 字符"));
        }
    }
    Ok(items)
}

/// 删除 JSON 指定路径上的键（清空列表字段时用它，而不是写个空数组进去）。
fn remove_json_path(obj: &mut Value, path: &[&str]) {
    if path.is_empty() || !obj.is_object() {
        return;
    }
    let o = obj.as_object_mut().expect("object");
    if path.len() == 1 {
        o.remove(path[0]);
        return;
    }
    if let Some(next) = o.get_mut(path[0]) {
        remove_json_path(next, &path[1..]);
    }
}

/// 镜像源地址校验：写进 daemon.json 之前挡一道，
/// 否则一个手滑的地址会让 dockerd 起不来（且面板拿不到更有用的报错）。
fn validate_registry_mirrors(items: &[String]) -> Result<(), String> {
    for it in items {
        if !it.starts_with("https://") && !it.starts_with("http://") {
            return Err(format!("镜像源地址必须以 https:// 或 http:// 开头: {it}"));
        }
    }
    Ok(())
}

fn set_json_path(obj: &mut Value, path: &[&str], value: Value) {
    if path.is_empty() {
        return;
    }
    if !obj.is_object() {
        *obj = json!({});
    }
    let o = obj.as_object_mut().expect("object");
    if path.len() == 1 {
        o.insert(path[0].to_string(), value);
        return;
    }
    let entry = o.entry(path[0].to_string()).or_insert_with(|| json!({}));
    set_json_path(entry, &path[1..], value);
}

// ── verbs ────────────────────────────────────────────────────

pub(crate) async fn run_blocking<F>(f: F) -> Response
where
    F: FnOnce() -> Result<Response, String> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
        .unwrap_or_else(|e| Response::err(-1, e))
}

/// installed_info 返回的组装信息（status / list 共用）。
type InstalledInfo = (
    Option<PathBuf>,
    Option<String>,
    Option<(PathBuf, PathBuf, bool)>,
);

/// 组装常见安装信息（status / list 共用）。
/// PHP 实例 svc（php74 / php81 …）直接定位安装目录内路径与实例 unit：
/// bin = {dir}/bin/php、主配置 = {dir}/etc/php.ini（目录 etc 为可编辑根）。
fn installed_info(d: &ServiceDef, svc: &str) -> InstalledInfo {
    if let Some(inst) = php_inst(svc) {
        let bin = inst.dir.join("bin/php");
        let bin = bin.is_file().then_some(bin);
        let unit = effective_unit(d, svc);
        let main = if bin.is_some() {
            // 主配置优先用 info.yaml 登记的 config_file，兜底 {dir}/etc/php.ini
            let m = php_reg_of(svc)
                .and_then(|r| r.conf)
                .unwrap_or_else(|| inst.dir.join("etc/php.ini"));
            let exists = m.is_file();
            let root = m.parent().unwrap_or(&inst.dir).to_path_buf();
            Some((m, root, exists))
        } else {
            None
        };
        return (bin, unit, main);
    }
    let bin = find_bin(d);
    let unit = active_unit(d);
    let installed = bin.is_some() || unit.is_some();
    let mut main = if installed { probe_main(d) } else { None };
    // 自定义服务可在定义里显式指定配置文件列表（main），首个作为主配置；无需 bin/unit 探测即可编辑
    if main.is_none() {
        if let Some(m) = d.main.first() {
            let resolved: Option<(PathBuf, PathBuf, bool)> = if m.contains('*') {
                glob_first(m).map(|p| {
                    (
                        p.clone(),
                        p.parent().unwrap_or(Path::new("/")).to_path_buf(),
                        true,
                    )
                })
            } else {
                let p = PathBuf::from(m);
                Some((
                    p.clone(),
                    p.parent().unwrap_or(Path::new("/")).to_path_buf(),
                    p.is_file(),
                ))
            };
            if let Some(t) = resolved {
                main = Some(t);
            }
        }
    }
    // PHP 兜底：静态候选一个都没命中（main_exists=false）时，改用二进制自报的 php.ini
    if d.key == "php"
        && let Some(b) = bin.as_deref()
        && main.as_ref().is_some_and(|(_, _, exists)| !exists)
        && let Some(p) = php_ini_from_bin(b)
    {
        let root = p.parent().unwrap_or(Path::new("/")).to_path_buf();
        let exists = p.is_file();
        if exists {
            tracing::info!(
                "service_conf: php 主配置由 `php --ini` 定位到 {}",
                p.display()
            );
        } else {
            // 常见原因：php.ini 未随安装脚本拷贝（PHP 会退回内置默认值仍能运行）
            tracing::warn!(
                "service_conf: php 已安装但主配置缺失：{}（PHP 正使用内置默认值）",
                p.display()
            );
        }
        main = Some((p, root, exists));
    }
    (bin, unit, main)
}

/// 数据库引擎识别（svc=mysql 同时覆盖 MySQL / MariaDB 两种安装）：
/// 优先以应用商店登记为准（info.yaml 的 instance 形如 `mysql-8.0` / `mariadb-10.11`，
/// 安装目录也因此不同：mysql-* 或 mariadb-*）；兜底看版本输出
/// （MariaDB 的客户端/服务端版本输出均含 "MariaDB" 字样）。
fn db_engine(version: &str) -> Option<&'static str> {
    for a in super::appstore::registered_apps() {
        let inst = a.instance.as_str();
        if inst == "mysql" || inst.starts_with("mysql-") {
            return Some("mysql");
        }
        if inst == "mariadb" || inst.starts_with("mariadb-") {
            return Some("mariadb");
        }
    }
    if version.contains("MariaDB") {
        return Some("mariadb");
    }
    (!version.is_empty()).then_some("mysql")
}

/// MySQL / MariaDB 安装目录：引擎不同目录不同（`${APPS_DIR}/mysql-<ver>` 或
/// `${APPS_DIR}/mariadb-<ver>`）。优先应用商店登记（info.yaml 的 install_dir），
/// 兜底按实际二进制（/usr/local/mysql/bin/mysql 软链）回溯到安装目录。
fn db_install_dir(bin: Option<&Path>) -> Option<PathBuf> {
    for a in super::appstore::registered_apps() {
        let inst = a.instance.as_str();
        let is_db = inst == "mysql"
            || inst.starts_with("mysql-")
            || inst == "mariadb"
            || inst.starts_with("mariadb-");
        if is_db
            && let Some(dir) = a.install_dir
            && dir.is_dir()
        {
            return Some(dir);
        }
    }
    let b = bin?.canonicalize().ok()?;
    let dir = b.parent()?.parent()?;
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    (dir.is_dir() && (name.starts_with("mysql-") || name.starts_with("mariadb-")))
        .then(|| dir.to_path_buf())
}

/// service_conf.status
pub async fn status(svc: &str) -> Response {
    let svc = svc.to_string();
    run_blocking(move || {
        let Some(d) = supported(&svc) else {
            return Err(format!("不支持的服务类型: {svc}"));
        };
        let (bin, unit, main) = installed_info(&d, &svc);
        if bin.is_none() && unit.is_none() {
            return Ok(Response::ok(
                "ok",
                Some(json!({ "installed": false, "service": svc })),
            ));
        }
        let version = bin
            .as_deref()
            .map(|b| detect_version(&d, b))
            .unwrap_or_default();
        let (conf_file, conf_dir, main_exists) = match &main {
            Some((m, dir, exists)) => (
                Some(m.display().to_string()),
                Some(dir.display().to_string()),
                *exists,
            ),
            None => (None, None, false),
        };
        // 引擎与安装目录只对数据库服务（svc=mysql，同时覆盖 MySQL / MariaDB）判定；
        // db_engine/db_install_dir 看的是「系统里装了哪种数据库实例」，
        // 若不加服务判定，已装 MySQL 时会把它的引擎与目录错误挂到 php 等服务上。
        let engine = (svc == "mysql").then(|| db_engine(&version)).flatten();
        let dir = engine
            .is_some()
            .then(|| db_install_dir(bin.as_deref()))
            .flatten()
            .map(|d| d.display().to_string());
        let running = service_running(&d, &svc, bin.as_deref());
        let bin_path = bin.as_deref().map(|b| b.display().to_string());
        // 探测失败时把尝试过的候选路径一起回传，便于定位"没检测到配置文件"的原因
        // （实例安装走固定路径，静态候选不适用，故仅在非实例时返回）
        let conf_candidates: Option<Vec<&str>> = if php_inst(&svc).is_some() {
            None
        } else {
            Some(d.main_candidates.iter().map(|s| s.as_str()).collect())
        };
        let manager = match d.manager {
            Manager::Systemd => "systemd",
            Manager::Openrc => "openrc",
        };
        Ok(Response::ok(
            "ok",
            Some(json!({
                "installed": true,
                "service": svc,
                "label": d.label,
                "manager": manager,
                "bin": bin_path,
                "engine": engine,
                "dir": dir,
                "version": version,
                "unit": unit,
                "running": running,
                "systemd": unit.is_some(),
                "conf_file": conf_file,
                "conf_dir": conf_dir,
                "main_exists": main_exists,
                "conf_candidates": conf_candidates,
            })),
        ))
    })
    .await
}

/// 主配置置首 + 目录扫描结果按路径去重。
///
/// 旧实现先 push 主配置、再对整个列表 `retain(seen.insert)` —— 主配置的路径
/// 已在集合里，`insert` 返回 false，**主配置自己被当成重复项删掉了**，
/// 表现为列表里唯独没有 php.ini（截图实锤）。
fn merge_main_and_scanned(
    main: &Path,
    root: &Path,
    main_exists: bool,
    scanned: Vec<Value>,
) -> Vec<Value> {
    let mut files = vec![file_entry(
        main,
        root,
        true,
        std::fs::metadata(main).map(|m| m.len()).unwrap_or(0),
        main_exists,
    )];
    let mut seen: std::collections::HashSet<String> = Default::default();
    seen.insert(main.display().to_string());
    for f in scanned {
        let p = f
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        // 扫描再次命中主配置时丢弃该条（主配置已在首位且带 is_main 标记）
        if seen.insert(p) {
            files.push(f);
        }
    }
    files
}

/// service_conf.list
pub async fn conf_list(svc: &str) -> Response {
    let svc = svc.to_string();
    run_blocking(move || {
        let Some(d) = supported(&svc) else {
            return Err(format!("不支持的服务类型: {svc}"));
        };
        let (bin, unit, main) = installed_info(&d, &svc);
        // 自定义服务即便未安装（无 bin/unit），只要登记了配置文件也应列出以便编辑
        if bin.is_none() && unit.is_none() && d.main.is_empty() {
            return Ok(Response::ok(
                "ok",
                Some(json!({ "installed": false, "files": [] })),
            ));
        }
        let Some((main, root, main_exists)) = main else {
            return Ok(Response::ok(
                "ok",
                Some(json!({
                    "installed": true,
                    "conf_file": null,
                    "conf_dir": null,
                    "main_exists": false,
                    "files": [],
                })),
            ));
        };
        // 显式登记的配置文件（可多个，支持 * 通配），首个即主配置；其余也一并列出
        let mut explicit: Vec<PathBuf> = Vec::new();
        for m in &d.main {
            if m.contains('*') {
                explicit.extend(glob_all(m));
            } else {
                explicit.push(PathBuf::from(m));
            }
        }
        if explicit.is_empty() {
            explicit.push(main.clone());
        }
        let scanned = if root.is_dir() {
            let mut scanned: Vec<Value> = Vec::new();
            collect_files(&service_exts(&d, &root), &root, &mut scanned);
            scanned
        } else {
            Vec::new()
        };
        let mut files: Vec<Value> = Vec::new();
        let mut seen: std::collections::HashSet<String> = Default::default();
        for (i, p) in explicit.iter().enumerate() {
            if seen.insert(p.display().to_string()) {
                let exists = p.is_file();
                let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                files.push(file_entry(p, &root, i == 0, size, exists));
            }
        }
        for f in scanned {
            let p = f
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if seen.insert(p) {
                files.push(f);
            }
        }
        Ok(Response::ok(
            "ok",
            Some(json!({
                "installed": true,
                "conf_file": main.display().to_string(),
                "conf_dir": root.display().to_string(),
                "main_exists": main_exists,
                "files": files,
            })),
        ))
    })
    .await
}

/// service_conf.read
pub async fn conf_read(svc: &str, path: String) -> Response {
    let svc = svc.to_string();
    run_blocking(move || {
        let Some(d) = supported(&svc) else {
            return Err(format!("不支持的服务类型: {svc}"));
        };
        let (_, _, Some((main, root, _))) = installed_info(&d, &svc) else {
            return Err(format!("{} 未安装或未探测到配置，请先在应用商店安装", d.label));
        };
        let (canon, is_main) =
            validate_path(&d, &service_exts(&d, &root), &main, &root, &path)?;
        if !canon.exists() {
            return Ok(Response::ok(
                "ok",
                Some(json!({
                    "path": canon.display().to_string(),
                    "is_main": is_main,
                    "content": "",
                    "size": 0,
                    "mtime": 0,
                    "missing": true,
                })),
            ));
        }
        let meta = std::fs::metadata(&canon).map_err(|e| format!("读取文件失败: {e}"))?;
        if meta.len() > MAX_FILE_BYTES {
            return Err("文件过大，不支持在线编辑".to_string());
        }
        let content = std::fs::read_to_string(&canon).map_err(|e| format!("读取文件失败: {e}"))?;
        Ok(Response::ok(
            "ok",
            Some(json!({
                "path": canon.display().to_string(),
                "is_main": is_main,
                "content": content,
                "size": meta.len(),
                "mtime": meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0),
            })),
        ))
    })
    .await
}

/// service_conf.save（json 格式先做合法性校验）。
pub async fn conf_save(svc: &str, path: String, content: String) -> Response {
    let svc = svc.to_string();
    run_blocking(move || {
        let Some(d) = supported(&svc) else {
            return Err(format!("不支持的服务类型: {svc}"));
        };
        if content.len() as u64 > MAX_FILE_BYTES {
            return Err("内容超过 2MB，请精简后重试".to_string());
        }
        let (_, _, Some((main, root, _))) = installed_info(&d, &svc) else {
            return Err(format!("{} 未安装或未探测到配置", d.label));
        };
        let (canon, is_main) = validate_path(&d, &service_exts(&d, &root), &main, &root, &path)?;
        if d.format == ConfFormat::Json && !content.trim().is_empty() {
            serde_json::from_str::<Value>(&content)
                .map_err(|e| format!("JSON 语法错误，未保存：{e}"))?;
        }
        if !is_main {
            // 编辑非主文件
            if !canon.exists() {
                return Err("文件不存在".to_string());
            }
        }
        let backup = backup_file(&svc, &canon)?;
        if let Some(parent) = canon.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        std::fs::write(&canon, &content).map_err(|e| format!("写入失败: {e}"))?;
        Ok(Response::ok(
            "ok",
            Some(json!({
                "path": canon.display().to_string(),
                "backup": backup.display().to_string(),
                "saved": true,
                "reason": "配置已保存，重启或重载服务后生效".to_string(),
            })),
        ))
    })
    .await
}

/// service_conf.defs：yaml 里注册的服务定义清单。
///
/// 前端据此生成「其它服务」入口：新增服务只要丢一份 yaml 进去，不需要改代码。
/// `origin` 标出是内置还是管理员自建，便于 UI 区分是否可编辑 / 删除。
pub async fn defs_list() -> Response {
    let items: Vec<Value> = defs()
        .iter()
        .map(|d| {
            json!({
                "key": d.key,
                "label": d.label,
                "version": d.version,
                "format": if d.format == ConfFormat::Json { "json" } else { "ini" },
                "exts": d.exts,
                "fields": d.fields.len(),
                "manager": match d.manager { Manager::Systemd => "systemd", Manager::Openrc => "openrc" },
                "origin": match d.origin { DefOrigin::Builtin => "builtin", DefOrigin::User => "user" },
                // 是否已注册为系统服务（unit 文件存在）；has_unit 表示定义里是否带 unit 模板（无则不能注册）
                "registered": active_unit(d).is_some(),
                "has_unit": d.unit.is_some(),
            })
        })
        .collect();
    Response::ok("ok", Some(json!({ "items": items })))
}

/// service_conf.def_write：写入 / 覆盖一份自定义服务定义（落盘到 `ZAP_SERVICE_CONF_DIR`）。
///
/// 仅校验结构与文件名安全，不执行任何命令；写完后热重载，前端无需重启即可看到新服务。
pub async fn def_write(key: String, yaml: String) -> Response {
    if !super::valid_token(&key) {
        return Response::err(
            -1,
            "非法的服务名（仅允许字母数字与 -/_，长度 1..=48）".to_string(),
        );
    }
    let mut def: ServiceDef = match serde_yaml::from_str(&yaml) {
        Ok(d) => d,
        Err(e) => return Response::err(-1, format!("服务定义解析失败: {e}")),
    };
    if def.key != key {
        return Response::err(-1, format!("yaml 里的 key（{}）与文件名不符（{}）", def.key, key));
    }
    // 若带了 unit 模板，校验其合法性（避免生成阶段才报错）
    if let Some(u) = &def.unit {
        if let Err(e) = validate_unit(u, def.manager) {
            return Response::err(-1, format!("unit 模板校验失败: {e}"));
        }
    }
    let dir = override_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Response::err(-1, format!("创建服务定义目录失败 {}: {e}", dir.display()));
    }
    let path = dir.join(format!("{key}.yaml"));
    // 原子写：临时文件 + rename，避免半截文件被读到
    let tmp = path.with_extension("yaml.tmp");
    if let Err(e) = std::fs::write(&tmp, &yaml) {
        return Response::err(-1, format!("写入临时文件失败 {}: {e}", tmp.display()));
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        return Response::err(-1, format!("落盘服务定义失败 {}: {e}", path.display()));
    }
    def.origin = DefOrigin::User;
    reload_defs();
    Response::ok(
        "ok",
        Some(json!({ "key": key, "path": path.display().to_string(), "saved": true })),
    )
}

/// service_conf.def_delete：删除一份自定义服务定义（仅能删管理员自建的）。
pub async fn def_delete(key: String) -> Response {
    if !super::valid_token(&key) {
        return Response::err(
            -1,
            "非法的服务名（仅允许字母数字与 -/_，长度 1..=48）".to_string(),
        );
    }
    let path = override_dir().join(format!("{key}.yaml"));
    if !path.is_file() {
        return Response::err(-1, format!("服务定义 {key} 不存在（内置服务不可删除）"));
    }
    if let Err(e) = std::fs::remove_file(&path) {
        return Response::err(-1, format!("删除服务定义失败 {}: {e}", path.display()));
    }
    reload_defs();
    Response::ok("ok", Some(json!({ "key": key, "deleted": true })))
}

/// service_conf.def_read：读取一份自定义服务定义的原文（编辑时回填表单用）。
pub async fn def_read(key: String) -> Response {
    if !super::valid_token(&key) {
        return Response::err(
            -1,
            "非法的服务名（仅允许字母数字与 -/_，长度 1..=48）".to_string(),
        );
    }
    let path = override_dir().join(format!("{key}.yaml"));
    let (exists, content) = if path.is_file() {
        match std::fs::read_to_string(&path) {
            Ok(c) => (true, Some(c)),
            Err(_) => (true, None),
        }
    } else {
        (false, None)
    };
    Response::ok(
        "ok",
        Some(json!({ "key": key, "exists": exists, "content": content })),
    )
}

// ── 单元文件自动生成 ───────────────────────────────────────────
//
// 管理员在定义里附 `unit` 模板后，可由面板一键渲染成系统单元文件并 enable，
// 实现「创建自动启动文件」：systemd 渲染 `<key>.service` 并 `systemctl enable`；
// openrc 渲染 `/etc/init.d/<key>`（init.d 脚本）并 `rc-update add default`。

/// 单元名安全字符（systemd 单元 / openrc 脚本名）。
fn is_unit_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// 用户名 / 组名安全字符。
fn is_safe_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// 环境变量条目必须形如 `KEY=VALUE`。
fn is_env_entry(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    match s.split_once('=') {
        Some((k, _)) => !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        None => false,
    }
}

/// 把多行 / 含换行的文本压成单行（systemd 单元值不支持裸换行）。
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ").trim().to_string()
}

/// 校验 unit 模板：systemd 要求 exec_start；路径 / 名称 / 策略做白名单约束，
/// 防止生成出非法单元文件（或借此注入系统命令）。
fn validate_unit(u: &UnitTemplate, _mgr: Manager) -> Result<(), String> {
    // ExecStart 同时是 systemd 的 ExecStart= 与 openrc 的 command=，两种管理器都必填
    let exec = u.exec_start.as_deref().unwrap_or("").trim();
    if exec.is_empty() {
        return Err("unit 模板缺少 exec_start（启动命令必填）".into());
    }
    if exec.contains(['\n', '\r', '\0']) {
        return Err("exec_start 不能包含换行 / NUL".into());
    }
    if let Some(wd) = &u.working_dir {
        let wd = wd.trim();
        if !wd.starts_with('/') || wd.contains("..") || wd.contains(['\n', '\r']) {
            return Err("working_dir 必须是绝对路径且不含 ..".into());
        }
    }
    for n in [&u.user, &u.group].into_iter().flatten() {
        if !is_safe_name(n) {
            return Err(format!("user/group 含非法字符: {n}"));
        }
    }
    for e in &u.environment {
        if !is_env_entry(e) {
            return Err(format!("环境变量格式应为 KEY=VALUE: {e}"));
        }
    }
    for c in &u.precmd {
        if c.contains(['\n', '\r', '\0']) {
            return Err("precmd 不能包含换行 / NUL".into());
        }
    }
    for dep in u.after.iter().chain(u.wants.iter()).chain(u.requires.iter()) {
        if !is_unit_name(dep) {
            return Err(format!("依赖单元名非法: {dep}"));
        }
    }
    if let Some(r) = &u.restart {
        const OK: &[&str] = &[
            "no", "always", "on-success", "on-failure", "on-abnormal", "on-abort", "on-watchdog",
        ];
        if !OK.contains(&r.as_str()) {
            return Err(format!("非法的 Restart 策略: {r}"));
        }
    }
    if let Some(t) = &u.type_ {
        const TYPES: &[&str] = &[
            "simple", "exec", "forking", "oneshot", "notify", "notify-reload", "dbus", "idle",
        ];
        if !TYPES.contains(&t.as_str()) {
            return Err(format!("非法的 Type 类型: {t}"));
        }
    }
    // 自定义指令：每行必须形如 Key=Value（Key 为字母数字），且不含换行 / NUL；
    // 形如段头（[Service] 等，无 '=' 或 Key 含 '['）会被拒，避免破坏单元结构。
    for line in &u.extra {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if l.contains(['\n', '\r', '\0']) {
            return Err("自定义指令不能包含换行 / NUL".into());
        }
        let ok = match l.split_once('=') {
            Some((k, _)) => {
                !k.is_empty()
                    && k.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                    && k.chars().all(|c| c.is_ascii_alphanumeric())
            }
            None => false,
        };
        if !ok {
            return Err(format!("自定义指令格式应为 Key=Value: {l}"));
        }
    }
    Ok(())
}

/// 渲染 systemd 单元文件内容（结构化字段 → 标准 .service）。
fn render_systemd_unit(d: &ServiceDef, u: &UnitTemplate) -> String {
    let mut s = String::new();
    s.push_str("[Unit]\n");
    let desc = one_line(u.description.as_deref().unwrap_or(d.label.as_str()));
    s.push_str(&format!("Description={desc}\n"));
    if !u.after.is_empty() {
        s.push_str(&format!("After={}\n", u.after.iter().map(|x| x.trim()).collect::<Vec<_>>().join(" ")));
    }
    if !u.wants.is_empty() {
        s.push_str(&format!("Wants={}\n", u.wants.iter().map(|x| x.trim()).collect::<Vec<_>>().join(" ")));
    }
    if !u.requires.is_empty() {
        s.push_str(&format!("Requires={}\n", u.requires.iter().map(|x| x.trim()).collect::<Vec<_>>().join(" ")));
    }
    // 归 [Unit] 段：结构化的重启限流（若设置）
    if let Some(v) = u.start_limit_interval_sec {
        s.push_str(&format!("StartLimitIntervalSec={v}\n"));
    }
    if let Some(v) = u.start_limit_burst {
        s.push_str(&format!("StartLimitBurst={v}\n"));
    }
    // extra 里手写的 [Unit] 指令（StartLimit* 等；与结构化字段重复的跳过，避免重复行）
    for l in &u.extra {
        if l.trim().is_empty() {
            continue;
        }
        let key = l.split_once('=').map(|(k, _)| k.trim()).unwrap_or("");
        if matches!(
            key,
            "StartLimitIntervalSec" | "StartLimitBurst" | "StartLimitBurstSec" | "StartLimitInterval"
        ) {
            if key == "StartLimitIntervalSec" && u.start_limit_interval_sec.is_some() {
                continue;
            }
            if key == "StartLimitBurst" && u.start_limit_burst.is_some() {
                continue;
            }
            s.push_str(&format!("{}\n", one_line(l)));
        }
    }
    s.push('\n');
    s.push_str("[Service]\n");
    // Type=（服务类型，缺省 simple）
    let ty = u
        .type_
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("simple");
    s.push_str(&format!("Type={}\n", one_line(ty)));
    for c in &u.precmd {
        if !c.trim().is_empty() {
            s.push_str(&format!("ExecStartPre={}\n", one_line(c)));
        }
    }
    s.push_str(&format!("ExecStart={}\n", one_line(u.exec_start.as_deref().unwrap_or(""))));
    if let Some(wd) = &u.working_dir {
        if !wd.trim().is_empty() {
            s.push_str(&format!("WorkingDirectory={}\n", one_line(wd)));
        }
    }
    if let Some(user) = &u.user {
        if !user.trim().is_empty() {
            s.push_str(&format!("User={}\n", one_line(user)));
        }
    }
    if let Some(group) = &u.group {
        if !group.trim().is_empty() {
            s.push_str(&format!("Group={}\n", one_line(group)));
        }
    }
    for e in &u.environment {
        if !e.trim().is_empty() {
            s.push_str(&format!("Environment={}\n", one_line(e)));
        }
    }
    if let Some(r) = &u.restart {
        if !r.trim().is_empty() {
            s.push_str(&format!("Restart={}\n", one_line(r)));
        }
    }
    if let Some(rs) = u.restart_sec {
        s.push_str(&format!("RestartSec={rs}\n"));
    }
    // 归 [Service] 段的自定义指令（NoNewPrivileges / ProtectSystem 等高级项）
    for l in &u.extra {
        if l.trim().is_empty() {
            continue;
        }
        let key = l.split_once('=').map(|(k, _)| k.trim()).unwrap_or("");
        if !matches!(
            key,
            "StartLimitIntervalSec" | "StartLimitBurst" | "StartLimitBurstSec" | "StartLimitInterval"
        ) {
            s.push_str(&format!("{}\n", one_line(l)));
        }
    }
    s.push('\n');
    s.push_str("[Install]\n");
    s.push_str("WantedBy=multi-user.target\n");
    s
}

/// 把 systemd 单元名映射为 openrc 服务名：去掉 `.service/.target/.socket/.timer` 后缀，
/// 常见网络目标统一映射到 openrc 的 `net`（openrc 的 depend 用的是 init.d 服务名，不是 unit 名）。
fn openrc_svc_name(unit: &str) -> String {
    let base = unit
        .trim()
        .trim_end_matches(".service")
        .trim_end_matches(".target")
        .trim_end_matches(".socket")
        .trim_end_matches(".timer");
    match base {
        "network-online" | "network" | "network-pre" | "netmount" | "net.lo" => "net".to_string(),
        _ => base.to_string(),
    }
}

/// 渲染 openrc 初始化脚本（结构化字段 → `/etc/init.d/<key>` 的 shell 脚本）。
///
/// openrc 用 `command=` / `command_args=` / `command_user=` / `directory=` /
/// `env_*` / `pidfile` 等变量托管前台进程；`rc-update add <key> default` 即开机自启。
fn render_openrc_unit(d: &ServiceDef, u: &UnitTemplate) -> String {
    let mut s = String::new();
    s.push_str("#!/sbin/openrc-run\n\n");
    let desc = one_line(u.description.as_deref().unwrap_or(d.label.as_str()));
    s.push_str(&format!("description=\"{desc}\"\n\n"));
    // ExecStart 拆成 command（首个 token）+ command_args（其余）
    let exec = one_line(u.exec_start.as_deref().unwrap_or(""));
    let mut parts = exec.split_whitespace();
    let command = parts.next().unwrap_or("");
    let args: Vec<&str> = parts.collect();
    s.push_str(&format!("command=\"{command}\"\n"));
    if !args.is_empty() {
        s.push_str(&format!("command_args=\"{}\"\n", args.join(" ")));
    }
    // 运行用户 / 组（openrc 用 command_user="user[:group]"）
    if let Some(user) = &u.user {
        if !user.trim().is_empty() {
            let mut cu = one_line(user);
            if let Some(g) = &u.group {
                if !g.trim().is_empty() {
                    cu.push(':');
                    cu.push_str(&one_line(g));
                }
            }
            s.push_str(&format!("command_user=\"{cu}\"\n"));
        }
    }
    if let Some(wd) = &u.working_dir {
        if !wd.trim().is_empty() {
            s.push_str(&format!("directory=\"{}\"\n", one_line(wd)));
        }
    }
    s.push_str(&format!("pidfile=\"/run/{}.pid\"\n", d.key));
    // 环境变量：openrc 用 env_KEY="value" 透传
    for e in &u.environment {
        if let Some((k, v)) = e.split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                s.push_str(&format!("env_{}=\"{}\"\n", k, one_line(v)));
            }
        }
    }
    // 重启策略：openrc 的 supervise-daemon 默认会在进程退出时拉起；
    // 仅当显式要求 always / on-failure / on-abnormal 时给出 respawn_delay。
    match u.restart.as_deref() {
        Some("always") | Some("on-failure") | Some("on-abnormal") => {
            let sec = u.restart_sec.unwrap_or(1);
            s.push_str(&format!("respawn_delay={sec}\n"));
        }
        _ => {}
    }
    // 依赖映射（systemd → openrc，按依赖强度对应）：
    //   Requires= → need （强依赖，缺了起不来）
    //   Wants=     → use  （弱依赖，有则用）
    //   After=     → after（仅排序，不构成依赖）
    // 单元名去后缀后作为 openrc 服务名；常见网络目标统一为 net。
    let need: Vec<String> = u
        .requires
        .iter()
        .map(|x| openrc_svc_name(x))
        .filter(|x| !x.is_empty())
        .collect();
    let soft: Vec<String> = u
        .wants
        .iter()
        .map(|x| openrc_svc_name(x))
        .filter(|x| !x.is_empty())
        .collect();
    let after: Vec<String> = u
        .after
        .iter()
        .map(|x| openrc_svc_name(x))
        .filter(|x| !x.is_empty())
        .collect();
    if !need.is_empty() || !soft.is_empty() || !after.is_empty() {
        s.push_str("\ndepend() {\n");
        if !need.is_empty() {
            s.push_str(&format!("\tneed {}\n", need.join(" ")));
        }
        if !soft.is_empty() {
            s.push_str(&format!("\tuse {}\n", soft.join(" ")));
        }
        if !after.is_empty() {
            s.push_str(&format!("\tafter {}\n", after.join(" ")));
        }
        s.push_str("}\n");
    }
    // 启动前命令：openrc 用 precmd() 钩子（在 start 之前执行）
    if !u.precmd.is_empty() {
        s.push_str("\nprecmd() {\n");
        for c in &u.precmd {
            if !c.trim().is_empty() {
                s.push_str(&format!("\t{}\n", one_line(c)));
            }
        }
        s.push_str("}\n");
    }
    s
}

/// service_conf.unit_write：渲染并落盘单元文件（systemd 的 .service / openrc 的 init.d），
/// 再按管理器使能（开机自启）。
///
/// 仅处理以 key 命名的单元（`<key>.service` 或 `/etc/init.d/<key>`），路径由已校验的
/// key 拼接，不可能越权写其它路径；只有定义了 `unit` 模板的服务才允许生成。
pub async fn unit_write(key: String) -> Response {
    if !super::valid_token(&key) {
        return Response::err(
            -1,
            "非法的服务名（仅允许字母数字与 -/_，长度 1..=48）".to_string(),
        );
    }
    let def = match supported(&key) {
        Some(d) => d,
        None => return Response::err(-1, format!("服务定义 {key} 不存在")),
    };
    let unit = match &def.unit {
        Some(u) => u,
        None => {
            return Response::err(
                -1,
                format!("服务 {key} 未配置 unit 模板，无法生成单元文件（请在定义里加 unit 段）"),
            )
        }
    };
    if let Err(e) = validate_unit(unit, def.manager) {
        return Response::err(-1, format!("unit 模板校验失败: {e}"));
    }
    let (content, unit_name, path) = match def.manager {
        Manager::Systemd => {
            let content = render_systemd_unit(&def, unit);
            let name = format!("{key}.service");
            (content, name.clone(), Path::new("/etc/systemd/system").join(name))
        }
        Manager::Openrc => {
            let content = render_openrc_unit(&def, unit);
            (
                content,
                key.clone(),
                Path::new(OPENRC_INIT_DIR).join(&key),
            )
        }
    };
    let tmp = match def.manager {
        Manager::Systemd => path.with_extension("service.tmp"),
        Manager::Openrc => path.with_extension("tmp"),
    };
    if let Err(e) = std::fs::write(&tmp, &content) {
        return Response::err(-1, format!("写入临时单元失败 {}: {e}", tmp.display()));
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        return Response::err(-1, format!("落盘单元文件失败 {}: {e}", path.display()));
    }
    let mut notes = vec![format!("已生成 {}", path.display())];
    match def.manager {
        Manager::Systemd => {
            // 重新加载 systemd 并使能（best-effort，失败也告知）
            let reload = root_cmd(SYSTEMD_CTL).arg("daemon-reload").output();
            let enable = root_cmd(SYSTEMD_CTL).arg("enable").arg(&unit_name).output();
            match reload {
                Ok(o) if o.status.success() => notes.push("daemon-reload 成功".into()),
                Ok(o) => notes.push(format!(
                    "daemon-reload 失败: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => notes.push(format!("daemon-reload 执行失败: {e}")),
            }
            match enable {
                Ok(o) if o.status.success() => notes.push(format!("已 enable {unit_name}")),
                Ok(o) => notes.push(format!(
                    "enable 失败: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => notes.push(format!("enable 执行失败: {e}")),
            }
        }
        Manager::Openrc => {
            // init.d 脚本必须可执行
            if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)) {
                notes.push(format!("设置可执行权限失败: {e}"));
            }
            // rc-update add <key> default：开机自启（best-effort）
            let add = root_cmd(OPENRC_UPDATE).arg("add").arg(&unit_name).arg("default").output();
            match add {
                Ok(o) if o.status.success() => {
                    notes.push(format!("已 rc-update add {unit_name}（开机自启）"))
                }
                Ok(o) => notes.push(format!(
                    "rc-update add 失败: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => notes.push(format!("rc-update 执行失败: {e}")),
            }
        }
    }
    Response::ok("ok", Some(json!({ "unit": unit_name, "notes": notes })))
}

/// service_conf.unit_remove：disable 并删除由面板生成的单元文件
/// （systemd 的 `<key>.service` 或 openrc 的 `/etc/init.d/<key>`）。
///
/// 按实际落盘位置识别管理器（不依赖定义是否仍在），路径由已校验的 key 拼接，
/// 不会误删包管理器自带的同名单元（那些服务本就没有 unit 模板，不会走到生成逻辑）。
pub async fn unit_remove(key: String) -> Response {
    if !super::valid_token(&key) {
        return Response::err(
            -1,
            "非法的服务名（仅允许字母数字与 -/_，长度 1..=48）".to_string(),
        );
    }
    let sysd = Path::new("/etc/systemd/system").join(format!("{key}.service"));
    let openrc = Path::new(OPENRC_INIT_DIR).join(&key);
    let (path, name, is_sysd) = if sysd.is_file() {
        (sysd, format!("{key}.service"), true)
    } else if openrc.is_file() {
        (openrc, key.clone(), false)
    } else {
        return Response::err(
            -1,
            format!("单元文件 {key}.service / {OPENRC_INIT_DIR}/{key} 均不存在（无需移除）"),
        );
    };
    // 先 best-effort 取消自启
    if is_sysd {
        let _ = root_cmd(SYSTEMD_CTL).arg("disable").arg(&name).output();
    } else {
        let _ = root_cmd(OPENRC_UPDATE).arg("del").arg(&name).arg("default").output();
    }
    if let Err(e) = std::fs::remove_file(&path) {
        return Response::err(-1, format!("删除单元文件失败 {}: {e}", path.display()));
    }
    if is_sysd {
        let _ = root_cmd(SYSTEMD_CTL).arg("daemon-reload").output();
    }
    Response::ok("ok", Some(json!({ "unit": name, "removed": true })))
}

/// service_conf.keys：关键项表单定义 + 当前值。
pub async fn keys_get(svc: &str) -> Response {
    let svc = svc.to_string();
    run_blocking(move || {
        let Some(d) = supported(&svc) else {
            return Err(format!("不支持的服务类型: {svc}"));
        };
        let (bin, unit, main) = installed_info(&d, &svc);
        if bin.is_none() && unit.is_none() {
            return Ok(Response::ok(
                "ok",
                Some(json!({
                    "installed": false,
                    "fields": [],
                    "values": {},
                })),
            ));
        }
        let fields: Vec<Value> = d
            .fields
            .iter()
            .map(|f| {
                let kind = match f.kind {
                    FieldKind::Text => "text",
                    FieldKind::Number => "number",
                    FieldKind::Select => "select",
                    FieldKind::Bool => "bool",
                    FieldKind::List => "list",
                };
                json!({
                    "key": f.key,
                    "label": f.label,
                    "kind": kind,
                    "help": f.help,
                    "section": f.section,
                    "options": f.options,
                })
            })
            .collect();
        let (values, main_path, main_exists) = match &main {
            Some((m, _, exists)) => {
                let content = std::fs::read_to_string(m).unwrap_or_default();
                let values = match d.format {
                    ConfFormat::Ini => ini_read_values(&content, &d.ini_comment, &svc, &d.fields),
                    ConfFormat::Json => json_read_values(&content, &d.fields),
                };
                (values, Some(m.display().to_string()), *exists)
            }
            None => (Default::default(), None, false),
        };
        Ok(Response::ok(
            "ok",
            Some(json!({
                "installed": true,
                "service": svc,
                "label": d.label,
                "format": if d.format == ConfFormat::Json { "json" } else { "ini" },
                "main": main_path,
                "main_exists": main_exists,
                "fields": fields,
                "values": values,
            })),
        ))
    })
    .await
}

/// service_conf.keys_save：把关键项写回主配置文件（ini 托管块 / json 键合并）。
pub async fn keys_save(svc: &str, keys: std::collections::BTreeMap<String, String>) -> Response {
    let svc = svc.to_string();
    run_blocking(move || {
        let Some(d) = supported(&svc) else {
            return Err(format!("不支持的服务类型: {svc}"));
        };
        let (_, _, Some((main, _, _))) = installed_info(&d, &svc) else {
            return Err(format!("{} 未安装或未探测到配置", d.label));
        };
        let old = if main.exists() {
            std::fs::read_to_string(&main).unwrap_or_default()
        } else {
            String::new()
        };
        let patched = match d.format {
            ConfFormat::Ini => {
                // 仅接收注册过的 key，且非空才写入（空 = 从托管区移除该键）
                let cleaned = strip_managed_block(&old, &d.ini_comment, &svc);
                // 托管块内部为行式 key = value
                let entries: Vec<(&str, String)> = d
                    .fields
                    .iter()
                    .filter_map(|f| {
                        let v = keys.get(f.key.as_str())?.trim().to_string();
                        if v.is_empty() {
                            return None;
                        }
                        Some((f.key.as_str(), v))
                    })
                    .collect();
                if entries.is_empty() {
                    cleaned
                } else {
                    let block = block_lines(&d.ini_comment, &svc, &entries);
                    insert_block(
                        &cleaned,
                        d.fields.first().and_then(|f| f.section.as_deref()),
                        &block,
                    )
                }
            }
            ConfFormat::Json => {
                let mut obj: Value = if old.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&old).unwrap_or_else(|_| json!({}))
                };
                for f in &d.fields {
                    let jpath: Vec<&str> = f.jpath.iter().map(|s| s.as_str()).collect();
                    if let Some(raw) = keys.get(f.key.as_str()) {
                        let raw = raw.trim();
                        if raw.is_empty() {
                            // 表单每次都提交全部字段，所以列表留空是用户主动清空
                            // （registry-mirrors 之类必须真删掉，留个空数组 dockerd 一样不认）；
                            // 其余类型留空 = 保持原文件中的既有键不动（不写入托管值）
                            if matches!(f.kind, FieldKind::List) {
                                remove_json_path(&mut obj, &jpath);
                            }
                            continue;
                        }
                        let v = match f.kind {
                            FieldKind::Bool => {
                                Value::Bool(raw.eq_ignore_ascii_case("true") || raw == "1")
                            }
                            FieldKind::List => {
                                let items = parse_list_items(raw)?;
                                if f.key == "registry_mirrors" {
                                    validate_registry_mirrors(&items)?;
                                }
                                Value::Array(items.into_iter().map(Value::String).collect())
                            }
                            FieldKind::Number => raw
                                .parse::<i64>()
                                .map(Value::from)
                                .unwrap_or_else(|_| Value::String(raw.to_string())),
                            _ => Value::String(raw.to_string()),
                        };
                        set_json_path(&mut obj, &jpath, v);
                    }
                }
                serde_json::to_string_pretty(&obj).map_err(|e| format!("序列化 JSON 失败: {e}"))?
            }
        };
        let backup = backup_file(&svc, &main)?;
        if let Some(parent) = main.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        std::fs::write(&main, &patched).map_err(|e| format!("写入失败: {e}"))?;
        Ok(Response::ok(
            "ok",
            Some(json!({
                "path": main.display().to_string(),
                "backup": backup.display().to_string(),
                "saved": true,
                "reason": "关键配置已保存，重启或重载服务后生效".to_string(),
            })),
        ))
    })
    .await
}

/// service_conf.control：start / stop / restart / reload。
pub async fn control(svc: &str, action: &str) -> Response {
    let svc = svc.to_string();
    let action = action.to_string();
    run_blocking(move || {
        if !ALLOWED_ACTIONS.contains(&action.as_str()) {
            return Err(format!(
                "不支持的操作: {action}（仅支持 {}）",
                ALLOWED_ACTIONS.join(" / ")
            ));
        }
        let Some(d) = supported(&svc) else {
            return Err(format!("不支持的服务类型: {svc}"));
        };
        let bin = find_bin(&d);
        // 优先已注册/存在的 unit（实例 svc 走 php-fpm-<ver>）；其次用二进制的 systemd 名做一次尝试
        let unit = effective_unit(&d, &svc).or_else(|| {
            bin.as_ref()
                .and_then(|b| b.file_stem())
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        });
        if unit.is_none() {
            return Err(format!("{} 未安装或未检测到对应服务", d.label));
        }
        let unit = unit.unwrap();
        let method = if exists_m(d.manager, &unit) {
            act_m(d.manager, &action, &unit).map_err(|e| format!("{action} {unit} 失败：{e}"))?;
            if action == "restart" {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            // method 值沿用历史命名（面板依赖）
            "systemd"
        } else {
            return Err(format!("未找到服务 {}，请确认服务安装方式", unit));
        };
        if action == "start" || action == "restart" {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        let running = service_running(&d, &svc, bin.as_deref());
        let state = if running { "running" } else { "stopped" };
        Ok(Response::ok(
            "ok",
            Some(json!({
                "action": action,
                "unit": unit,
                "method": method,
                "state": state,
            })),
        ))
    })
    .await
}

// ── PHP 多版本实例：列表 + 全局默认访问 ─────────────────────────

/// 全局命令注册目录（install.sh 把实例注册到此处供所有用户直接使用）。
const GLOBAL_BIN_DIR: &str = "/usr/local/bin";
/// 全局默认访问注册的命令名（与应用商店 php install.sh 一致）。
const GLOBAL_LINK_NAMES: &[&str] = &["php", "php-cgi", "pear", "pecl"];

fn global_bin_link(name: &str) -> PathBuf {
    PathBuf::from(GLOBAL_BIN_DIR).join(name)
}

/// 宽松等价比较：优先 canonicalize，失败回退字符串相等（失效链接场景）。
fn paths_equal(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// 当前 /usr/local/bin/php 全局默认指向的目标（无注册链接则 None）。
fn global_default_bin() -> Option<PathBuf> {
    std::fs::read_link(global_bin_link("php")).ok()
}

/// 单个 PHP 实例的概要（instances 列表项），字段与 status 对齐。
fn instance_summary(svc: &str) -> Result<Value, String> {
    let Some(d) = supported(svc) else {
        return Err(format!("不支持的服务类型: {svc}"));
    };
    let (bin, unit, main) = installed_info(&d, svc);
    let installed = bin.is_some() || unit.is_some();
    let (conf_file, main_exists) = match &main {
        Some((m, _, exists)) => (Some(m.display().to_string()), *exists),
        None => (None, false),
    };
    let version = if installed {
        bin.as_deref()
            .map(|b| detect_version(&d, b))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let is_default = bin
        .as_deref()
        .is_some_and(|b| global_default_bin().is_some_and(|t| paths_equal(&t, b)));
    let dir = php_inst(svc).map(|i| i.dir.display().to_string());
    Ok(json!({
        "svc": svc,
        "instance": svc,
        "label": d.label,
        "version": version,
        "dir": dir,
        "installed": installed,
        "running": installed && service_running(&d, svc, bin.as_deref()),
        "bin": bin.as_deref().map(|b| b.display().to_string()),
        "unit": unit,
        "systemd": unit.is_some(),
        "conf_file": conf_file,
        "main_exists": main_exists,
        "is_default": is_default,
    }))
}

/// service_conf.instances：列出某服务的全部已安装版本实例。
/// 目前支持 php（应用商店 php-<ver> 目录实例）；无目录实例时回退类型级
/// php（系统包），保证既有「服务配置 → PHP」页能力不回归。
pub async fn instances(svc: &str) -> Response {
    let svc = svc.to_string();
    run_blocking(move || {
        if svc != "php" {
            return Err("实例列表目前仅支持 php 服务".to_string());
        }
        let mut list: Vec<Value> = Vec::new();
        let mut seen: std::collections::HashSet<String> = Default::default();
        let mut push = |svc: String| {
            if !seen.insert(svc.clone()) {
                return;
            }
            if let Ok(v) = instance_summary(&svc)
                && v.get("installed")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false)
            {
                list.push(v);
            }
        };
        // 闭包借用 list/seen，独立作用域避免与后续借用冲突
        {
            let mut try_push = |svc: String| push(svc);
            // 1) 应用商店登记优先（instance / install_dir / config_file 都来自 info.yaml）
            for r in php_registrations() {
                try_push(r.svc);
            }
            // 2) 目录兜底：php-74 与 php74 两种命名都扫，覆盖未登记 / 手工安装
            if let Ok(rd) = std::fs::read_dir(super::install_root()) {
                let mut dirs: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_dir()
                            && p.file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| n.starts_with("php"))
                    })
                    .collect();
                dirs.sort();
                for dir in dirs {
                    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    let digits = name
                        .trim_start_matches("php-")
                        .trim_start_matches("php")
                        .to_string();
                    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    try_push(format!("php{digits}"));
                }
            }
        }
        // 未发现目录实例（非应用商店安装）时，兼容系统包 PHP
        if list.is_empty()
            && let Ok(v) = instance_summary("php")
            && v.get("installed")
                .and_then(|x| x.as_bool())
                .unwrap_or(false)
        {
            list.push(v);
        }
        // 回传实例扫描根目录：php-* 没被识别时，可一眼看出是否装到了别的 APPS_DIR
        Ok(Response::ok(
            "ok",
            Some(json!({
                "instances": list,
                "apps_dir": super::install_root().display().to_string(),
            })),
        ))
    })
    .await
}

/// service_conf.default：设置 / 取消某 PHP 实例的「全局默认访问」。
///
/// - enable=true：把该实例注册到 `/usr/local/bin`（php / php-cgi / pear /
///   pecl），系统所有用户执行 `php` 默认使用本版本；`/usr/bin/php` 沿用
///   install.sh 语义仅在不存在时补链，避免抢占 Debian alternatives。
/// - enable=false：仅当 `/usr/local/bin/php` 当前指向本实例时移除注册
///   （已被其它实例接管则拒绝取消，避免误删别家链接）。
pub async fn set_default(svc: &str, enable: bool) -> Response {
    let svc = svc.to_string();
    run_blocking(move || {
        let Some(inst) = php_inst(&svc) else {
            return Err(format!("未找到 PHP 实例 {svc}（安装目录不存在）"));
        };
        let bin_dir = inst.dir.join("bin");
        if enable {
            let mut registered: Vec<String> = Vec::new();
            for name in GLOBAL_LINK_NAMES {
                let src = bin_dir.join(name);
                if !src.is_file() {
                    continue;
                }
                let link = global_bin_link(name);
                let _ = std::fs::remove_file(&link);
                std::os::unix::fs::symlink(&src, &link)
                    .map_err(|e| format!("注册 /usr/local/bin/{name} 失败: {e}"))?;
                registered.push(name.to_string());
            }
            if registered.is_empty() {
                return Err(format!("实例 {svc} 的 bin 目录为空，无法注册全局命令"));
            }
            let ub = PathBuf::from("/usr/bin/php");
            if !ub.exists() && bin_dir.join("php").is_file() {
                let _ = std::os::unix::fs::symlink(bin_dir.join("php"), &ub);
            }
            Ok(Response::ok(
                "ok",
                Some(json!({
                    "enabled": true,
                    "service": svc,
                    "registered": registered,
                })),
            ))
        } else {
            let mine = bin_dir.join("php");
            let cur = std::fs::read_link(global_bin_link("php")).unwrap_or_default();
            if !paths_equal(&cur, &mine) {
                return Err(
                    "当前 /usr/local/bin/php 不指向该实例（未注册或被其它实例接管），无需取消"
                        .to_string(),
                );
            }
            let mut removed: Vec<String> = Vec::new();
            for name in GLOBAL_LINK_NAMES {
                let src = bin_dir.join(name);
                let link = global_bin_link(name);
                let owned = std::fs::read_link(&link).unwrap_or_default();
                if paths_equal(&owned, &src) {
                    let _ = std::fs::remove_file(&link);
                    removed.push(name.to_string());
                }
            }
            Ok(Response::ok(
                "ok",
                Some(json!({
                    "enabled": false,
                    "service": svc,
                    "removed": removed,
                })),
            ))
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("zap-svcconf-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    /// 旧实现会把 `/etc/php/` 的末段当目录名前缀去扫 `/etc`，
    /// 导致 `/etc/php/<ver>/fpm/php.ini` 永远探测不到（PHP 配置页空白）。
    #[test]
    fn glob_matches_version_dir_and_prefers_highest() {
        let base = fixture_root("php");
        fs::create_dir_all(base.join("7.4/fpm")).unwrap();
        fs::create_dir_all(base.join("8.1/fpm")).unwrap();
        fs::create_dir_all(base.join("8.2/fpm")).unwrap();
        fs::write(base.join("7.4/fpm/php.ini"), "; 7.4").unwrap();
        fs::write(base.join("8.1/fpm/php.ini"), "; 8.1").unwrap();
        fs::write(base.join("8.2/fpm/php.ini"), "; 8.2").unwrap();

        let pattern = format!("{}/*/fpm/php.ini", base.display());
        assert_eq!(
            glob_first(&pattern).as_deref(),
            Some(base.join("8.2/fpm/php.ini").as_path())
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn glob_returns_none_when_tail_missing() {
        let base = fixture_root("missing");
        fs::create_dir_all(base.join("8.1/cli")).unwrap();
        fs::write(base.join("8.1/cli/other.ini"), "x").unwrap();

        let pattern = format!("{}/*/fpm/php.ini", base.display());
        assert!(glob_first(&pattern).is_none());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn glob_supports_dir_name_prefix() {
        let base = fixture_root("prefix");
        fs::create_dir_all(base.join("php-81")).unwrap();
        fs::write(base.join("php-81/php.ini"), "; remi").unwrap();

        let pattern = format!("{}/php-*/php.ini", base.display());
        assert_eq!(
            glob_first(&pattern).as_deref(),
            Some(base.join("php-81/php.ini").as_path())
        );

        let _ = fs::remove_dir_all(&base);
    }

    /// `glob_all` 返回所有命中文件（不止版本号最大的一个）。
    #[test]
    fn glob_all_expands_every_match() {
        let base = fixture_root("globall");
        fs::create_dir_all(base.join("app-1.0")).unwrap();
        fs::create_dir_all(base.join("app-2.0")).unwrap();
        fs::write(base.join("app-1.0/x.ini"), "; a").unwrap();
        fs::write(base.join("app-2.0/x.ini"), "; b").unwrap();
        // 后缀不同的文件不应命中
        fs::write(base.join("app-1.0/y.txt"), "no").unwrap();

        let pattern = format!("{}/app-*/x.ini", base.display());
        let hits = glob_all(&pattern);
        assert_eq!(hits.len(), 2);
        assert!(hits.contains(&base.join("app-1.0/x.ini")));
        assert!(hits.contains(&base.join("app-2.0/x.ini")));

        let _ = fs::remove_dir_all(&base);
    }

    /// 文件名通配 `/dir/*.conf` 能匹配同目录下所有 .conf（也支持 `pre*fix`）。
    #[test]
    fn glob_all_supports_filename_wildcard() {
        let base = fixture_root("globfile");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("a.conf"), "1").unwrap();
        fs::write(base.join("b.conf"), "2").unwrap();
        fs::write(base.join("note.txt"), "no").unwrap();

        let all = format!("{}/*.conf", base.display());
        let hits = glob_all(&all);
        assert_eq!(hits.len(), 2, "应命中 a.conf / b.conf");
        assert!(hits.contains(&base.join("a.conf")));
        assert!(hits.contains(&base.join("b.conf")));
        // 目录名通配模式仍可用：`*` 后接 `/` 表示匹配一层目录
        fs::create_dir_all(base.join("v1")).unwrap();
        fs::write(base.join("v1/x.conf"), "3").unwrap();
        let dirs = glob_all(&format!("{}/*/x.conf", base.display()));
        assert_eq!(dirs, vec![base.join("v1/x.conf")]);

        let _ = fs::remove_dir_all(&base);
    }

    /// 覆盖目录里管理员自写的定义能被正确解析（含 unit 模板），并标为 User。
    #[test]
    fn user_def_from_override_dir_is_loaded() {
        let dir = fixture_root("userdef");
        fs::create_dir_all(&dir).unwrap();
        let yaml = r#"
key: "myhello"
label: "myhello"
version: "1.0.0"
manager: "systemd"
format: "ini"
unit:
  exec_start: "python3 -m http.server 4000"
"#;
        fs::write(dir.join("myhello.yaml"), yaml).unwrap();
        unsafe { std::env::set_var("ZAP_SERVICE_CONF_DIR", &dir) };

        let defs = load_defs();
        let found = defs.iter().find(|d| d.key == "myhello");
        assert!(found.is_some(), "myhello 应被解析加载");
        let d = found.unwrap();
        assert_eq!(d.origin, DefOrigin::User);
        assert_eq!(d.manager, Manager::Systemd);
        assert_eq!(d.format, ConfFormat::Ini);
        assert_eq!(d.version, "1.0.0", "version 应被解析供总览展示");
        assert!(d.unit.is_some(), "unit 模板应被解析");
        assert_eq!(
            d.unit.as_ref().unwrap().exec_start.as_deref(),
            Some("python3 -m http.server 4000")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// 省略 manager 时回退到系统实际管理器；`main` 被解析；`unit.precmd` 渲染为
    /// systemd 的 ExecStartPre= 与 openrc 的 precmd() 钩子。
    #[test]
    fn unit_precmd_rendered_and_main_parsed() {
        let yaml = r#"
key: "precmdtest"
label: "precmdtest"
format: "ini"
main: "/etc/precmdtest/app.conf"
unit:
  exec_start: "myapp run"
  precmd:
    - "mkdir -p /run/precmdtest"
    - "chown app /run/precmdtest"
"#;
        let def: ServiceDef = serde_yaml::from_str(yaml).expect("解析自定义服务定义");
        assert_eq!(
            def.main.first().map(|s| s.as_str()),
            Some("/etc/precmdtest/app.conf")
        );
        // manager 省略 → 回退到系统管理器（合法值，不 panic）
        let _m = def.manager;
        let u = def.unit.as_ref().expect("unit 模板");
        assert_eq!(u.precmd.len(), 2);

        let sd = render_systemd_unit(&def, &u);
        assert!(sd.contains("ExecStartPre=mkdir -p /run/precmdtest"));
        assert!(sd.contains("ExecStartPre=chown app /run/precmdtest"));
        assert!(sd.contains("ExecStart=myapp run"));

        let orc = render_openrc_unit(&def, &u);
        assert!(orc.contains("precmd() {"));
        assert!(orc.contains("\tmkdir -p /run/precmdtest"));
        assert!(orc.contains("\tchown app /run/precmdtest"));
    }

    /// openrc 依赖映射：Requires→need、Wants→use、After→after；单元名去后缀，网络目标→net。
    #[test]
    fn openrc_depend_mapping() {
        let yaml = r#"
key: "deptest"
label: "deptest"
format: "ini"
unit:
  exec_start: "myapp run"
  after:
    - "network-online.target"
    - "nginx.service"
  wants:
    - "logging.service"
  requires:
    - "docker.service"
"#;
        let def: ServiceDef = serde_yaml::from_str(yaml).expect("解析自定义服务定义");
        let u = def.unit.as_ref().expect("unit 模板");
        let orc = render_openrc_unit(&def, u);
        assert!(orc.contains("depend() {"), "应生成 depend() 块");
        assert!(orc.contains("\tneed docker\n"), "Requires→need（docker.service→docker）");
        assert!(orc.contains("\tuse logging\n"), "Wants→use（logging.service→logging）");
        assert!(
            orc.contains("\tafter net nginx\n"),
            "After→after，network-online.target→net、nginx.service→nginx"
        );
    }

    /// 自定义指令：普通 Key=Value 注入 [Service]，StartLimit* 归到 [Unit]。
    #[test]
    fn systemd_extra_directives_routed() {
        let yaml = r#"
key: "extratest"
label: "extratest"
format: "ini"
unit:
  exec_start: "myapp run"
  restart: "on-failure"
  restart_sec: 5
  extra:
    - "NoNewPrivileges=yes"
    - "ProtectSystem=strict"
    - "ProtectHome=yes"
    - "PrivateTmp=yes"
    - "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6"
    - "StartLimitIntervalSec=60s"
    - "StartLimitBurst=5"
"#;
        let def: ServiceDef = serde_yaml::from_str(yaml).expect("解析自定义服务定义");
        let u = def.unit.as_ref().expect("unit 模板");
        let sd = render_systemd_unit(&def, u);
        let unit_idx = sd.find("[Unit]").expect("[Unit]");
        let svc_idx = sd.find("[Service]").expect("[Service]");
        // 加固项落在 [Service]
        assert!(sd.contains("NoNewPrivileges=yes"));
        assert!(sd.contains("ProtectSystem=strict"));
        assert!(sd.contains("ProtectHome=yes"));
        assert!(sd.contains("PrivateTmp=yes"));
        assert!(sd.contains("RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6"));
        for key in ["NoNewPrivileges=yes", "ProtectSystem=strict"] {
            let i = sd.find(key).expect(key);
            assert!(i > svc_idx, "{key} 应在 [Service] 段");
        }
        // StartLimit* 归到 [Unit]
        for key in ["StartLimitIntervalSec=60s", "StartLimitBurst=5"] {
            let i = sd.find(key).expect(key);
            assert!(i > unit_idx && i < svc_idx, "{key} 应在 [Unit] 段");
        }
    }

    /// 结构化的 StartLimitIntervalSec / StartLimitBurst 渲染到 [Unit]；
    /// extra 中与结构化字段重复的 StartLimitBurst 被跳过，避免重复行。
    #[test]
    fn systemd_start_limit_structured() {
        let yaml = r#"
key: "sltest"
label: "sltest"
format: "ini"
unit:
  exec_start: "myapp run"
  restart: "on-failure"
  restart_sec: 5
  start_limit_interval_sec: 60
  start_limit_burst: 5
  extra:
    - "StartLimitBurst=99"
"#;
        let def: ServiceDef = serde_yaml::from_str(yaml).expect("解析自定义服务定义");
        let u = def.unit.as_ref().expect("unit 模板");
        let sd = render_systemd_unit(&def, u);
        assert!(sd.contains("StartLimitIntervalSec=60"));
        assert!(sd.contains("StartLimitBurst=5"));
        assert!(
            !sd.contains("StartLimitBurst=99"),
            "extra 里重复的 StartLimitBurst 应被跳过"
        );
    }

    /// 主配置必须保留在列表首位；扫描到的重复条目被去掉（旧实现相反，
    /// 把主配置删了、列表里唯独没有 php.ini）。
    #[test]
    fn merge_keeps_main_and_dedups_scanned() {
        let base = fixture_root("merge");
        fs::create_dir_all(&base).unwrap();
        let main = base.join("php.ini");
        fs::write(&main, "; ini").unwrap();
        fs::write(base.join("pear.conf"), "# pear").unwrap();

        let scanned = vec![
            file_entry(&base.join("pear.conf"), &base, false, 7, true),
            file_entry(&main, &base, false, 5, true), // 扫描再次命中主配置
        ];
        let files = merge_main_and_scanned(&main, &base, true, scanned);

        assert_eq!(
            files.len(),
            2,
            "主配置 + pear.conf，扫描到的主配置条目应去重"
        );
        assert_eq!(
            files[0]["path"].as_str(),
            Some(main.display().to_string()).as_deref(),
            "主配置必须排在首位"
        );
        assert_eq!(files[0]["is_main"], Value::Bool(true));
        assert_eq!(
            files[1]["path"].as_str(),
            Some(base.join("pear.conf").to_string_lossy()).as_deref()
        );

        let _ = fs::remove_dir_all(&base);
    }

    /// 主配置不存在（探测失败回退的预期路径）时仍要出现在列表里，标 exists=false。
    #[test]
    fn merge_keeps_missing_main_visible() {
        let base = fixture_root("merge-missing");
        fs::create_dir_all(&base).unwrap();
        let main = base.join("php.ini"); // 不创建
        fs::write(base.join("php-fpm.conf"), "# fpm").unwrap();

        let scanned = vec![file_entry(
            &base.join("php-fpm.conf"),
            &base,
            false,
            6,
            true,
        )];
        let files = merge_main_and_scanned(&main, &base, false, scanned);

        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["is_main"], Value::Bool(true));
        assert_eq!(files[0]["exists"], Value::Bool(false));

        let _ = fs::remove_dir_all(&base);
    }

    /// instance 用无连字符（php74），目录用连字符（php-74）——两种写法都要认。
    #[test]
    fn instance_naming_is_tolerant() {
        assert_eq!(php_inst_svc("php74").as_deref(), Some("74"));
        assert_eq!(php_inst_svc("php-74").as_deref(), Some("74"));
        assert_eq!(php_inst_svc("php"), None, "类型级 php 不算实例");
        assert_eq!(php_inst_svc("php8x"), None);
        assert_eq!(inst_dir_names("74"), ["php-74", "php74"]);
    }

    /// 服务定义来自 yaml（内置 + /etc/zap/services 覆盖），内置三件套必须能解析出来
    #[test]
    fn yaml_defs_are_loaded() {
        let defs = defs();
        let keys: Vec<&str> = defs.iter().map(|d| d.key.as_str()).collect();
        for k in ["php", "mysql", "docker"] {
            assert!(keys.contains(&k), "内置定义缺少 {k}");
        }
        let php = supported("php").expect("php 定义");
        assert!(
            php.main_candidates
                .iter()
                .any(|c| c == "/etc/php/*/fpm/php.ini")
        );
        assert!(php.main_candidates.iter().any(|c| c == "/etc/php.ini"));
        assert!(
            php.main_candidates
                .iter()
                .any(|c| c == "/etc/opt/remi/*/php.ini")
        );
        assert_eq!(php.format, ConfFormat::Ini);
        assert_eq!(php.ini_comment, ";");
        assert_eq!(php.fields.len(), 8);

        let mysql = supported("mysql").expect("mysql 定义");
        assert_eq!(mysql.ini_comment, "#");
        assert!(
            mysql
                .fields
                .iter()
                .all(|f| f.section.as_deref() == Some("mysqld"))
        );
        assert_eq!(mysql.fields.len(), 6);

        let docker = supported("docker").expect("docker 定义");
        assert_eq!(docker.format, ConfFormat::Json);
        assert_eq!(docker.fields.len(), 13);
        assert!(
            docker
                .fields
                .iter()
                .any(|f| f.kind == FieldKind::List && f.jpath == vec!["registry-mirrors"])
        );

        // php 多实例 svc 复用 php 定义；未知服务不在注册表里
        assert!(supported("php74").is_some());
        assert!(supported("redis").is_none());
    }

    /// 一份合法 yaml + 一份坏 yaml：好的覆盖内置，坏的跳过且不拖垮其它服务
    #[test]
    fn yaml_override_and_bad_file() {
        let mut out: Vec<ServiceDef> = Vec::new();
        push_def(
            &mut out,
            "key: php\nlabel: PHP 覆盖\nformat: ini\nini_comment: \";\"\n",
            DefOrigin::User,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "PHP 覆盖");
        // 同 key 再推一次 = 覆盖
        push_def(
            &mut out,
            "key: php\nlabel: 再覆盖\nformat: ini\n",
            DefOrigin::User,
        );
        assert_eq!(out.len(), 1, "同 key 应覆盖而不是追加");
        assert_eq!(out[0].label, "再覆盖");
        // 坏文件：跳过
        push_def(&mut out, "key: [broken\n", DefOrigin::User);
        assert_eq!(out.len(), 1, "坏定义应被跳过");
    }

    /// 列表字段（镜像源）在表单里是文本框，换行和逗号都得认，空行要丢掉
    #[test]
    fn list_items_accept_newline_and_comma() {
        assert_eq!(
            parse_list_items("https://a.example\nhttps://b.example,https://c.example\n\n").unwrap(),
            vec![
                "https://a.example".to_string(),
                "https://b.example".to_string(),
                "https://c.example".to_string()
            ]
        );
        assert!(parse_list_items("  \n , ").unwrap().is_empty());
    }

    #[test]
    fn list_items_rejects_overflow() {
        let many: Vec<String> = (0..MAX_LIST_ITEMS + 1)
            .map(|i| format!("https://m{i}.example"))
            .collect();
        assert!(parse_list_items(&many.join("\n")).is_err());
    }

    /// 镜像源写错会让 dockerd 起不来，保存前必须挡住
    #[test]
    fn mirrors_must_be_http_urls() {
        assert!(validate_registry_mirrors(&["https://a.mirror.aliyuncs.com".to_string()]).is_ok());
        assert!(validate_registry_mirrors(&["ftp://x.example".to_string()]).is_err());
        assert!(validate_registry_mirrors(&["a.mirror.aliyuncs.com".to_string()]).is_err());
    }

    /// registry-mirrors 在 daemon.json 里是数组，读出来要摊平成多行文本
    #[test]
    fn json_array_becomes_multiline_text() {
        let v = json!({ "registry-mirrors": ["https://a.example", "https://b.example"] });
        let text = json_value_to_text(v.get("registry-mirrors").unwrap());
        assert_eq!(text, json!("https://a.example\nhttps://b.example"));
    }

    /// 清空列表字段 = 删掉这个键，而不是留个空数组
    #[test]
    fn remove_json_path_removes_key() {
        let mut v = json!({ "log-opts": { "max-size": "20m" }, "registry-mirrors": ["https://a"] });
        remove_json_path(&mut v, &["registry-mirrors"]);
        assert!(v.get("registry-mirrors").is_none());
        remove_json_path(&mut v, &["log-opts", "max-size"]);
        assert_eq!(v.get("log-opts"), Some(&json!({})));
    }
}
