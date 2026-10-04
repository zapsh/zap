//! 插件运行时（mlua 嵌入 zapexec）。
//!
//! 插件只装在 `$ZAP_PATH/plugins/<name>/`（系统级，仅管理员可安装）。普通用户只能「使用」插件
//! （运行 / 查看界面），不能自行安装，也不能再把插件放进自己的家目录。`main.lua` 定义 `on_run(ctx)`，通过全局表 `zap`
//! 调用受限能力：
//!   - `zap.log(msg)`                   打印日志（回传前端）
//!   - `zap.option(name)`               读取运行选项
//!   - `zap.run(prog, {args})`          按 scope 自动选 root / 站点账号 / 用户账号执行
//!   - `zap.exec(prog, {args})`         以 root 执行（仅 scope=system）
//!   - `zap.exec_as_user(prog, {args})` 以站点 Linux 账号（scope=site）/ 面板用户账号（scope=user）执行
//!   - `zap.site_root()` / `zap.site_linux_user()`  当前站点上下文（仅 scope=site）
//!   - `zap.home_dir()`               当前执行身份的家目录（scope=system 为调用方 home，scope=site 为站点账号 home，scope=user 为面板用户 home）
//!   - `zap.read_file(p)` / `zap.write_file(p, s)`  按 scope 降权读写文件
//!   - `zap.json_encode(v)` / `zap.json_decode(s)`
//!
//! 沙箱：只启用 `table` / `string` / `math` / `utf8` / `coroutine`。
//! 刻意不含 `io` / `os` / `package` / `debug` —— 插件跑在 zapexec 进程里，
//! 放开 `io` 就能以 root 身份读写任意文件，直接绕过 `scope=site` 的降权
//! （`drop_privileges` 只对子进程生效）。这些能力改由公共函数库提供。
//!
//! 安全边界：
//!   - 插件只能声明结构化 UI（manifest），不能注入前端代码；
//!   - 执行身份由 scope 决定：scope=site 降到站点账号；scope=user 降到调用方面板用户的 Linux 账号；
//!     scope=system 才以 root 执行（仅管理员安装的插件才能声明）；
//!   - 插件统一由管理员装进 `$ZAP_PATH/plugins`，运行时 `scope` 只决定降到哪个身份，
//!     不跨出插件目录、不拿比声明更高的权限；
//!   - 插件子进程套用资源笼子（`resource::TaskResource`：rlimit + NO_NEW_PRIVS + 可选 cgroup），
//!     防 fork 炸弹 / 写满磁盘 / 吃满 CPU；同步运行另有墙钟超时兜底。
//!   - 插件名只允许 `[A-Za-z0-9_-]`，目录越界即拒绝。

use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tracing::{info, warn};
use zap_proto::Response;

fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

/// 同步插件运行的墙钟超时（秒）：超时后放弃等待并杀掉子进程（组）。
/// 资源笼子的 `RLIMIT_CPU` 是兜底，这里防止请求线程被一个卡住的插件永久占住。
/// 长任务请改用 `async: true` 的插件（由前端取消，不占同步线程）。
const PLUGIN_SYNC_TIMEOUT_SECS: u64 = 1800;

/// 资源笼子 run_id 用的进程内自增序号，保证每次 `zap.run` 的 cgroup 叶子目录不重名。
static PLUGIN_RES_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn is_plugin_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !s.contains("..")
}

/// 共享函数库目录 / 隐藏目录：`_lib`、`lib`、`.git` 这类不是插件，扫描时跳过。
fn is_reserved_dir(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('_')
}

fn read_manifest(dir: &Path) -> Result<serde_yaml::Value, String> {
    for name in ["manifest.yaml", "manifest.yml"] {
        let p = dir.join(name);
        if p.is_file() {
            let txt = std::fs::read_to_string(&p).map_err(|e| format!("{e}"))?;
            return serde_yaml::from_str(&txt).map_err(|e| format!("manifest 解析失败: {e}"));
        }
    }
    Err("未找到 manifest.yaml".into())
}

/// 插件只装在系统级目录 `$ZAP_PATH/plugins`（`system`），仅管理员可写、所有用户共享。
/// 已移除用户级（`~/.zap/plugins`）：避免普通用户自行放置插件并以 root 执行。
fn plugin_base() -> PathBuf {
    zap_path().join("plugins")
}

/// 旧版安装记录文件名：曾写在插件目录内供列表页展示来源 / 安装时间。
/// 现已不再写入，这里仅保留以便 `read_install_meta` 兼容仍带此文件的旧插件。
const INSTALL_META: &str = ".zap-install.json";

fn read_install_meta(dir: &Path) -> Option<Value> {
    let p = dir.join(INSTALL_META);
    let txt = std::fs::read_to_string(&p).ok()?;
    serde_json::from_str::<Value>(&txt).ok()
}

/// 把安装来源 / 时间写回插件自身的 `manifest.yaml`，落在顶层 `zap_install:` 下。
///
/// 不再单独维护注册表 / Meta 文件：列表直接读 manifest 即可拿到来源与安装时间。
/// 用独立顶层键避免与插件自身字段冲突；整个 manifest 解析为 `Value` 再写回，
/// 保留作者原有的其它字段（仅会丢失 YAML 注释）。旧插件若仍带 `.zap-install.json`
/// 由 `describe` 兜底读取，这里不依赖它。
pub(crate) fn write_install_meta(
    dir: &Path,
    source: &str,
    src: &str,
    installed_at: i64,
) -> Result<(), String> {
    // 安装来源已移除 git，现在只有 `archive`（上传包）/ `appstore`（应用商店）两种合法值；
    // 做归一化兜底，任何非二者的值（含历史 `git`）都统一写成 `archive`，写回不再保留 git 来源。
    let source = match source {
        "archive" | "appstore" => source,
        _ => "archive",
    };
    let mut path = None;
    for name in ["manifest.yaml", "manifest.yml"] {
        let p = dir.join(name);
        if p.is_file() {
            path = Some(p);
            break;
        }
    }
    let path = path.ok_or_else(|| "未找到 manifest.yaml".to_string())?;
    let txt = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    let mut doc: serde_yaml::Value =
        serde_yaml::from_str(&txt).map_err(|e| format!("manifest 解析失败: {e}"))?;
    let map = doc
        .as_mapping_mut()
        .ok_or_else(|| "manifest 不是顶层映射，无法写入安装信息".to_string())?;
    let mut meta = serde_yaml::Mapping::new();
    meta.insert(
        serde_yaml::Value::String("source".into()),
        serde_yaml::Value::String(source.to_string()),
    );
    meta.insert(
        serde_yaml::Value::String("src".into()),
        serde_yaml::Value::String(src.to_string()),
    );
    meta.insert(
        serde_yaml::Value::String("installed_at".into()),
        serde_yaml::Value::Number(serde_yaml::Number::from(installed_at)),
    );
    map.insert(
        serde_yaml::Value::String("zap_install".into()),
        serde_yaml::Value::Mapping(meta),
    );
    let out = serde_yaml::to_string(&doc).map_err(|e| format!("manifest 写回失败: {e}"))?;
    std::fs::write(&path, out).map_err(|e| format!("写 manifest 失败: {e}"))?;
    Ok(())
}

/// AppStore「plugins」类包（系统级 Lua 插件）经应用商店安装 / 升级后，把来源写回插件
/// `manifest.yaml` 的 `zap_install.source: appstore`，使插件系统在「插件管理」里正确显示
/// 「应用商店」而非兜底成「手动放置」。
///
/// install.sh 每次整目录 `cp` 都会覆盖 manifest，所以必须在脚本 success 后补写（install /
/// upgrade / 重跑 各路径都调它）。非 `plugins` 类包不走插件引擎，直接跳过。
pub(crate) fn write_appstore_plugin_source(
    cat: &str,
    name: &str,
    pkg_path: &str,
    base: &Path,
) {
    if cat != "plugins" {
        return;
    }
    let dir = base.join(name);
    if !dir.is_dir() {
        return;
    }
    if let Err(e) = write_install_meta(
        &dir,
        "appstore",
        pkg_path,
        chrono::Utc::now().timestamp(),
    ) {
        warn!("AppStore 插件 {name} 写回安装来源失败: {e}");
    }
}

fn manifest_str<'a>(m: &'a serde_yaml::Value, key: &str) -> Option<&'a str> {
    m.get(key).and_then(|v| v.as_str())
}

/// 把一个插件目录整理成前端需要的描述对象。
fn describe(dir: &Path, name: &str) -> Result<Value, String> {
    let m = read_manifest(dir)?;
    let ui = m.get("ui");
    let ui_str = |k: &str| {
        ui.and_then(|u| u.get(k))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let placement = ui_str("placement");
    // 安装信息优先读 manifest 里的 `zap_install`（安装时写回），兼容旧版仍带
    // `.zap-install.json` 的插件（read_install_meta 仅作兜底）。
    let install_yaml = m.get("zap_install");
    let meta_json = read_install_meta(dir);
    let meta_str = |k: &str| -> String {
        if let Some(s) = install_yaml
            .and_then(|v| v.get(k))
            .and_then(|v| v.as_str())
        {
            return s.to_string();
        }
        if let Some(s) = meta_json
            .as_ref()
            .and_then(|v| v.get(k))
            .and_then(|v| v.as_str())
        {
            return s.to_string();
        }
        String::new()
    };
    // json! 的对象语法不收块表达式，label 的回落值先算出来
    let label = {
        let s = ui_str("label");
        if s.is_empty() {
            name.to_string()
        } else {
            s
        }
    };
    Ok(json!({
        "name": name,
        "title": manifest_str(&m, "title").unwrap_or(name),
        "async": m.get("async").and_then(|v| v.as_bool()).unwrap_or(false),
        "scope": manifest_str(&m, "scope").unwrap_or("system"),
        "placement": placement,
        "label": label,
        "icon": ui_str("icon"),
        "tab": ui_str("tab"),
        "options": serde_json::to_value(m.get("options").cloned().unwrap_or(serde_yaml::Value::Null)).unwrap_or(Value::Null),
        "actions": serde_json::to_value(m.get("actions").cloned().unwrap_or(serde_yaml::Value::Null)).unwrap_or(Value::Null),
        // 自带 HTML 界面：值为 manifest 里 ui.html 声明的相对文件名，空串表示没有
        "html": ui_html_file(&m).unwrap_or_default(),
        "version": manifest_str(&m, "version").unwrap_or("").to_string(),
        "description": manifest_str(&m, "description").unwrap_or("").to_string(),
        "author": manifest_str(&m, "author").unwrap_or("").to_string(),
        "homepage": manifest_str(&m, "homepage").unwrap_or("").to_string(),
        "source": meta_str("source"),
        "src": meta_str("src"),
        "installed_at": install_yaml
            .and_then(|v| v.get("installed_at"))
            .and_then(|v| v.as_i64())
            .or_else(|| {
                meta_json
                    .as_ref()
                    .and_then(|v| v.get("installed_at"))
                    .and_then(|v| v.as_i64())
            })
            .unwrap_or(0),
    }))
}

/// manifest 里 `ui.html` 声明的界面文件名（相对插件目录）。
///
/// 只接受插件目录下的单个 `.html` 文件：挡掉 `..`、绝对路径、反斜杠与子目录，
/// 免得插件借这个字段把插件目录以外的文件（`/etc/shadow` 之类）读出去。
fn ui_html_file(m: &serde_yaml::Value) -> Option<String> {
    let s = m
        .get("ui")
        .and_then(|u| u.get("html"))
        .and_then(|v| v.as_str())?
        .trim();
    if s.is_empty() || s.starts_with('/') || s.contains("..") || s.contains('\\') {
        return None;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return None;
    }
    if !s.to_ascii_lowercase().ends_with(".html") {
        return None;
    }
    Some(s.to_string())
}

/// 读取插件自带的 HTML 界面文件内容。
///
/// 前端把它塞进 `sandbox="allow-scripts"` 的 iframe 里渲染 —— 不含
/// `allow-same-origin`，所以这份 HTML 拿不到面板的 DOM / Cookie / localStorage，
/// 也发不出带凭据的请求；它要调后端只能走 `postMessage` 让父页面代跑
/// `plugin.run`，权限与 scope 仍旧由后端把关。
pub async fn plugin_ui(_actor: String, _home: String, name: String) -> Response {
    if !is_plugin_name(&name) {
        return Response::err(-1, "非法插件名".to_string());
    }
    let base = plugin_base();
    let dir = base.join(&name);
    let m = match read_manifest(&dir) {
        Ok(m) => m,
        Err(e) => return Response::err(-1, e),
    };
    let Some(file) = ui_html_file(&m) else {
        return Response::err(-1, "该插件没有声明 ui.html 界面".to_string());
    };
    let path = dir.join(&file);
    // 兜底：canonicalize 后必须仍在插件目录内
    let (Ok(canon_file), Ok(canon_dir)) = (path.canonicalize(), dir.canonicalize()) else {
        return Response::err(-1, "界面文件不存在".to_string());
    };
    if !canon_file.starts_with(&canon_dir) {
        return Response::err(-1, "界面文件路径越界，已拒绝".to_string());
    }
    match std::fs::read_to_string(&canon_file) {
        Ok(html) => Response::ok("ok", Some(json!({ "html": html }))),
        Err(e) => Response::err(-1, format!("读取界面文件失败: {e}")),
    }
}

/// 列出插件（统一在系统级 `$ZAP_PATH/plugins`），按 placement 槽位 / scope 过滤。
pub async fn plugin_list(
    _actor: String,
    home: String,
    slot: Option<String>,
    scope: Option<String>,
) -> Response {
    let zap = zap_path();
    // 插件只装在系统级目录，所有用户共享；不再扫描任何用户家目录
    let bases: Vec<PathBuf> = vec![zap.join("plugins")];
    info!(
        "plugin_list: home={home:?} slot={slot:?} scope={scope:?} zap={}",
        zap.display()
    );
    for base in &bases {
        info!("  base={} exists={}", base.display(), base.exists());
    }
    let mut items = Vec::new();
    for base in &bases {
        let Ok(rd) = std::fs::read_dir(base) else {
            warn!("  read_dir 失败（跳过）: {}", base.display());
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let name = match p.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if is_reserved_dir(&name) || !is_plugin_name(&name) {
                continue;
            }
            let Ok(m) = read_manifest(&p) else {
                warn!("  插件 {name} 读取 manifest 失败（跳过）: {}", p.display());
                continue;
            };
            let pl_scope = manifest_str(&m, "scope").unwrap_or("system");
            let placement = m
                .get("ui")
                .and_then(|u| u.get("placement"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(sc) = &scope {
                if pl_scope != sc.as_str() {
                    continue;
                }
            }
            if let Some(sl) = &slot {
                if &placement != sl {
                    continue;
                }
            }
            match describe(&p, &name) {
                Ok(v) => items.push(v),
                Err(e) => warn!("  插件 {name} 描述失败（跳过）: {e}"),
            }
        }
    }
    info!("plugin_list 完成: 返回 {} 个插件", items.len());
    Response::ok("ok", Some(json!(items)))
}

// ── 安装 / 卸载 ────────────────────────────────────────────

/// 安装插件：`source` 目前仅支持 `archive`（已落盘的 zip / tar.gz 包，由面板上传而来）。
///
/// 流程：解包到临时目录 → 定位插件根 → 校验 manifest.yaml + main.lua →
/// 落地到 `<base>/<name>` → 把来源 / 安装时间写回 manifest.yaml → 放开读权限。
///
/// 不写任何注册表 / 独立 Meta 文件：列表靠扫描系统目录 + 读 manifest.yaml 里的 `zap_install`。
/// 插件只装在系统级目录（`$ZAP_PATH/plugins`），仅管理员可安装，普通用户只能使用。
pub async fn plugin_install(
    _actor: String,
    _home: String,
    name: String,
    source: String,
    src: String,
    force: bool,
) -> Response {
    // 名称可留空：此时以 manifest 里的 name 为准（上传 zip 时通常不知道里面叫什么）
    if !name.is_empty() && !is_plugin_name(&name) {
        return Response::err(-1, "非法插件名（只允许字母数字、下划线、连字符）");
    }
    let base = plugin_base();

    let zap = zap_path();
    let tmp_root = zap.join("data/plugins/tmp");
    let work = tmp_root.join(format!("install-{}", chrono::Utc::now().timestamp_millis()));
    let unpack = work.join("src");
    if let Err(e) = std::fs::create_dir_all(&unpack) {
        return Response::err(-1, format!("创建临时目录失败: {e}"));
    }

    let res: Result<String, String> = (|| {
        match source.as_str() {
            "archive" => extract_archive(Path::new(&src), &unpack)?,
            other => return Err(format!("未知的插件来源类型: {other}（仅支持上传包安装）")),
        }
        let root = locate_plugin_root(&unpack)?;

        // 校验：manifest 能解析出 ui.placement，且 main.lua 存在
        let m = read_manifest(&root)?;
        if m.get("ui").and_then(|u| u.get("placement")).is_none() {
            return Err("manifest 缺少 ui.placement，无法在前端呈现入口".into());
        }
        // 校验 scope 取值，避免未知值被静默当成 root 执行
        let scope_decl = manifest_str(&m, "scope").unwrap_or("system");
        if scope_decl != "site" && scope_decl != "user" && scope_decl != "system" {
            return Err(
                format!("manifest 的 scope 非法: {scope_decl}（应为 site / user / system）").into(),
            );
        }
        if !root.join("main.lua").is_file() {
            return Err("插件目录缺少 main.lua".into());
        }

        // 定名：优先用调用方给的名字，其次用 manifest 的 name
        let effective = if !name.is_empty() {
            name.clone()
        } else {
            let n = manifest_str(&m, "name").unwrap_or("").trim().to_string();
            if !is_plugin_name(&n) {
                return Err(
                    "manifest 缺少合法的 name 字段（只允许字母数字、下划线、连字符）".into(),
                );
            }
            n
        };
        if let Some(n) = manifest_str(&m, "name") {
            if n != effective {
                return Err(format!("manifest 里的 name（{n}）与插件名（{effective}）不一致"));
            }
        }

        let target = base.join(&effective);
        if target.exists() {
            if !force {
                return Err(format!(
                    "插件 {effective} 已存在于 system 级，如需覆盖请勾选「覆盖安装」"
                ));
            }
            std::fs::remove_dir_all(&target).map_err(|e| format!("移除旧版本失败: {e}"))?;
        }

        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(|e| format!("移除旧版本失败: {e}"))?;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建插件目录失败: {e}"))?;
        }
        // 跨设备时 rename 会失败，退回递归复制
        if std::fs::rename(&root, &target).is_err() {
            copy_dir_all(&root, &target)?;
            let _ = std::fs::remove_dir_all(&root);
        }

        // 把安装来源 / 时间写回 manifest.yaml 的顶层 `zap_install:`（列表读取时直接解析，
        // 不再单独维护 Meta 文件）。失败不阻断安装，仅告警。
        if let Err(e) = write_install_meta(
            &target,
            &source,
            &src,
            chrono::Utc::now().timestamp(),
        ) {
            warn!("写入插件安装信息失败（已忽略）: {e}");
        }

        // 解包出来的文件可能带着打包机的权限（如 0600），放开「可读」让站点账号能读；
        // `X` 只给目录和本来就可执行的文件加执行位，不会误开可执行权限。
        let _ = std::process::Command::new("chmod")
            .args(["-R", "a+rX", &target.display().to_string()])
            .output();
        Ok(effective)
    })();

    // 无论成败都清掉临时目录
    let _ = std::fs::remove_dir_all(&work);

    match res {
        Ok(effective) => match describe(&base.join(&effective), &effective) {
            Ok(info) => {
                info!("插件安装完成: {effective} (system) 来自 {source}");
                Response::ok("插件安装完成", Some(info))
            }
            Err(e) => Response::err(-1, format!("插件已安装但读取信息失败: {e}")),
        },
        Err(e) => {
            warn!("插件安装失败: {name}: {e}");
            Response::err(-1, e)
        }
    }
}

/// 卸载插件：删除 `<base>/<name>` 整个目录。
pub async fn plugin_uninstall(
    _actor: String,
    _home: String,
    name: String,
) -> Response {
    if !is_plugin_name(&name) {
        return Response::err(-1, "非法插件名");
    }
    let base = plugin_base();
    let target = base.join(&name);
    if !target.is_dir() {
        return Response::err(-1, format!("system 级插件不存在: {name}"));
    }
    // 路径穿越兜底：canonicalize 后必须仍在 base 之内
    let (Ok(canon_target), Ok(canon_base)) = (target.canonicalize(), base.canonicalize()) else {
        return Response::err(-1, "插件路径解析失败".to_string());
    };
    if !canon_target.starts_with(&canon_base) {
        return Response::err(-1, "插件路径越界，已拒绝".to_string());
    }
    match std::fs::remove_dir_all(&canon_target) {
        Ok(()) => {
            info!("插件已卸载: {name} (system)");
            Response::ok("插件已卸载", None)
        }
        Err(e) => Response::err(-1, format!("卸载失败: {e}")),
    }
}

/// 解压 zip / tar.gz / tgz / tar.bz2 到 `dest`。
fn extract_archive(file: &Path, dest: &Path) -> Result<(), String> {
    let lower = file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if lower.ends_with(".zip") {
        return extract_zip(file, dest);
    }
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return extract_tar(file, dest, true);
    }
    if lower.ends_with(".tar") {
        return extract_tar(file, dest, false);
    }
    Err("只支持 .zip / .tar.gz / .tgz / .tar 格式的插件包".into())
}

fn extract_zip(file: &Path, dest: &Path) -> Result<(), String> {
    let f = std::fs::File::open(file).map_err(|e| format!("打开插件包失败: {e}"))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("解析 zip 失败: {e}"))?;
    for i in 0..z.len() {
        let mut entry = z
            .by_index(i)
            .map_err(|e| format!("读取 zip 条目失败: {e}"))?;
        // enclosed_name 会挡掉 `../` 与绝对路径
        let out = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => {
                warn!("跳过越界的 zip 条目: {}", entry.name());
                continue;
            }
        };
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| format!("建目录失败: {e}"))?;
        } else {
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("建目录失败: {e}"))?;
            }
            let mut w = std::fs::File::create(&out).map_err(|e| format!("写文件失败: {e}"))?;
            std::io::copy(&mut entry, &mut w).map_err(|e| format!("解压失败: {e}"))?;
        }
    }
    Ok(())
}

/// `gz=true` 走 GzDecoder（.tar.gz / .tgz），否则按裸 tar 处理。
fn extract_tar(file: &Path, dest: &Path, gz: bool) -> Result<(), String> {
    let f = std::fs::File::open(file).map_err(|e| format!("打开插件包失败: {e}"))?;
    if gz {
        let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
        unpack_tar(&mut ar, dest)
    } else {
        let mut ar = tar::Archive::new(f);
        unpack_tar(&mut ar, dest)
    }
}

fn unpack_tar<R: std::io::Read>(ar: &mut tar::Archive<R>, dest: &Path) -> Result<(), String> {
    for entry in ar
        .entries()
        .map_err(|e| format!("解析 tar 失败: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("读取 tar 条目失败: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("读取 tar 条目路径失败: {e}"))?
            .to_path_buf();
        // 手动挡掉 `..` 与绝对路径（tar crate 不保证替调用方做这件事）
        if path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            warn!("跳过越界的 tar 条目: {}", path.display());
            continue;
        }
        entry
            .unpack(dest.join(&path))
            .map_err(|e| format!("解压 {} 失败: {e}", path.display()))?;
    }
    Ok(())
}

/// 定位插件根：包里常见「顶层只有一个目录」和「直接就是插件内容」两种布局。
fn locate_plugin_root(unpack: &Path) -> Result<PathBuf, String> {
    if unpack.join("manifest.yaml").is_file() || unpack.join("manifest.yml").is_file() {
        return Ok(unpack.to_path_buf());
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(unpack)
        .map_err(|e| format!("读取插件包内容失败: {e}"))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    // 只有一层包装目录时才下钻；多个目录说明包结构不对
    if dirs.len() == 1 {
        let only = dirs.remove(0);
        if only.join("manifest.yaml").is_file() || only.join("manifest.yml").is_file() {
            return Ok(only);
        }
    }
    Err("插件包里没找到 manifest.yaml（需放在包根或唯一的顶层目录内）".into())
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("建目录失败: {e}"))?;
    for e in std::fs::read_dir(src)
        .map_err(|e| format!("读取 {src:?} 失败: {e}"))?
        .flatten()
    {
        let from = e.path();
        let to = dst.join(e.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| format!("复制 {from:?} 失败: {e}"))?;
        }
    }
    Ok(())
}

/// 运行插件。
pub async fn plugin_run(
    name: String,
    _actor: String,
    home: String,
    user: Option<String>,
    _site_id: Option<i64>,
    site_root: Option<String>,
    site_linux_user: Option<String>,
    action: String,
    options: HashMap<String, String>,
) -> Response {
    if !is_plugin_name(&name) {
        return Response::err(-1, "非法插件名");
    }
    // action 会拼进 Lua 函数名（`on_<action>`），必须先卡成合法标识符
    if action.is_empty()
        || action.len() > 32
        || !action
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Response::err(-1, "非法 action（只允许字母数字与下划线，≤32 字符）");
    }
    let zap = zap_path();
    // 插件只装在系统级目录，所有用户共享
    let dir = zap.join("plugins").join(&name);
    if !dir.is_dir() {
        return Response::err(-1, format!("插件不存在: {name}"));
    }
    let manifest = match read_manifest(&dir) {
        Ok(m) => m,
        Err(e) => return Response::err(-1, e),
    };
    let declared = manifest_str(&manifest, "scope").unwrap_or("system");
    let scope = declared.to_string();
    let (run_user, run_root) = match scope.as_str() {
        "site" => match (site_root.clone(), site_linux_user.clone()) {
            (Some(r), Some(u)) => (Some(u), Some(r)),
            _ => return Response::err(-1, "site 作用域插件需要 site_root / site_linux_user"),
        },
        // scope=user：降到调用方面板用户的 Linux 账号，以该用户家目录为工作根；
        // 不依赖站点上下文，因此非站点场景（如管理用户自己的家目录 / 密钥）也能用。
        "user" => match user.clone() {
            Some(u) => (Some(u), None),
            None => {
                return Response::err(-1, "user 作用域插件需要调用方 Linux 账号（user 字段为空）")
            }
        },
        "system" => (None, None),
        other => {
            return Response::err(-1, format!("未知 scope: {other}（应为 site / user / system）"))
        }
    };
    let lua_file = dir.join("main.lua");
    let code = match std::fs::read_to_string(&lua_file) {
        Ok(c) => c,
        Err(e) => return Response::err(-1, format!("读取插件脚本失败: {e}")),
    };

    // 异步模式：后台执行，日志实时落盘，立即返回 task_id + log_path，由前端 SSE 订阅
    let is_async = manifest
        .get("async")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if is_async {
        let task_id = format!("{name}-{}", chrono::Utc::now().timestamp_millis());
        let log_dir = zap.join("data/plugins/logs");
        if let Some(p) = log_dir.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = std::fs::create_dir_all(&log_dir);
        let log_path = log_dir.join(format!("{task_id}.log"));
        // 取消哨兵文件：前端经 zapd 写 <log>.cancel，看门狗线程轮询到就杀掉子进程
        let cancel_path = log_path.with_extension("log.cancel");
        let lp = log_path.clone();
        // 取消标志 + 当前子进程 pid（exec_as_user 走 setsid，pid 即进程组号，可整组杀）
        let cancel = Arc::new(AtomicBool::new(false));
        let child_pid: Arc<std::sync::Mutex<Option<(u32, bool)>>> =
            Arc::new(std::sync::Mutex::new(None));
        let cancel_flag = cancel.clone();
        let child_pid_w = child_pid.clone();
        let cancel_path_w = cancel_path.clone();
        std::thread::spawn(move || loop {
            if cancel_path_w.exists() || cancel_flag.load(Ordering::SeqCst) {
                cancel_flag.store(true, Ordering::SeqCst);
                if let Some((pid, session_leader)) = child_pid_w.lock().unwrap().take() {
                    unsafe {
                        // session_leader（setsid 过）= 杀整个进程组；否则只杀单进程，避免误伤 zapexec
                        if session_leader {
                            libc::kill(-(pid as i32), libc::SIGKILL);
                        } else {
                            libc::kill(pid as i32, libc::SIGKILL);
                        }
                    }
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        });
        let ctx = RunCtx {
            scope: scope.clone(),
            run_user: run_user.clone(),
            run_root: run_root.clone(),
            home: home.clone(),
            plugin_dir: dir.clone(),
            options: options.clone(),
            action: action.clone(),
        };
        tokio::task::spawn_blocking(move || {
            let res = run_lua(
                &code,
                &ctx,
                Some(lp.clone()),
                cancel.clone(),
                child_pid.clone(),
            );
            let cancelled = cancel.load(Ordering::SeqCst);
            // 清掉取消哨兵文件（若存在）
            let _ = std::fs::remove_file(&cancel_path);
            match res {
                Ok(_) => super::finish_log(lp.to_str().unwrap_or(""), 0),
                Err(e) => {
                    if cancelled {
                        super::log_line(lp.to_str().unwrap_or(""), "任务已取消");
                        super::finish_log(lp.to_str().unwrap_or(""), -2);
                    } else {
                        super::log_line(lp.to_str().unwrap_or(""), &format!("插件执行失败: {e}"));
                        super::finish_log(lp.to_str().unwrap_or(""), -1);
                    }
                }
            }
        });
        return Response::ok(
            "ok",
            Some(json!({
                "task_id": task_id,
                "log_path": log_path.display().to_string(),
                "async": true,
            })),
        );
    }

    let ctx = RunCtx {
        scope,
        run_user,
        run_root,
        home,
        plugin_dir: dir,
        options,
        action,
    };
    // 同步运行套一层墙钟超时：超时则置取消标志，看门狗杀掉子进程（组），请求不再被永久占住。
    let cancel = Arc::new(AtomicBool::new(false));
    let child_pid = Arc::new(std::sync::Mutex::new(None::<(u32, bool)>));
    let cancel_w = cancel.clone();
    let child_pid_w = child_pid.clone();
    std::thread::spawn(move || loop {
        if cancel_w.load(Ordering::SeqCst) {
            if let Some((pid, session_leader)) = child_pid_w.lock().unwrap().take() {
                unsafe {
                    if session_leader {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    } else {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                }
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    });
    let cancel_run = cancel.clone();
    let child_pid_run = child_pid.clone();
    let out = tokio::time::timeout(
        Duration::from_secs(PLUGIN_SYNC_TIMEOUT_SECS),
        tokio::task::spawn_blocking(move || -> Result<String, String> {
            run_lua(&code, &ctx, None, cancel_run, child_pid_run)
        }),
    )
    .await;
    let out = match out {
        Ok(join) => join.unwrap_or_else(|e| Err(format!("插件执行线程崩溃: {e}"))),
        Err(_) => {
            cancel.store(true, Ordering::SeqCst);
            // 给看门狗一点时间杀掉子进程（组），避免孤儿进程残留
            std::thread::sleep(Duration::from_millis(300));
            Err(format!(
                "插件同步执行超过 {} 秒被终止（长任务请改用 async 插件）",
                PLUGIN_SYNC_TIMEOUT_SECS
            ))
        }
    };

    match out {
        Ok(log) => Response::ok("插件执行完成", Some(json!({ "log": log }))),
        Err(e) => Response::err(-1, e),
    }
}

/// 一次插件执行的上下文。
struct RunCtx {
    scope: String,
    run_user: Option<String>,
    run_root: Option<String>,
    home: String,
    plugin_dir: PathBuf,
    options: HashMap<String, String>,
    action: String,
}

/// 沙箱启用的 Lua 标准库子集（不含 io / os / package / debug）。
fn sandbox_libs() -> mlua::StdLib {
    mlua::StdLib::TABLE
        | mlua::StdLib::STRING
        | mlua::StdLib::MATH
        | mlua::StdLib::UTF8
        | mlua::StdLib::COROUTINE
}

/// 自动加载的公共函数库目录（按此顺序，后者可覆盖前者）：
/// 系统级 → 插件自带的 `lib/`。
fn shared_lib_dirs(_home: &str, plugin_dir: &Path) -> Vec<PathBuf> {
    vec![
        zap_path().join("data/plugins/_lib"),
        plugin_dir.join("lib"),
    ]
}

/// 在 mlua 沙箱里执行插件主体并调用 on_run。
fn run_lua(
    code: &str,
    ctx: &RunCtx,
    log_file: Option<std::path::PathBuf>,
    cancel: Arc<AtomicBool>,
    child_pid: Arc<std::sync::Mutex<Option<(u32, bool)>>>,
) -> Result<String, String> {
    let lua = mlua::Lua::new_with(sandbox_libs(), mlua::LuaOptions::default())
        .map_err(|e| format!("创建 Lua 沙箱失败: {e}"))?;
    let log = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    // 异步模式：日志同时落盘（zapexec 既有的长任务日志协议）；exec 输出也实时 tee 到同一文件，
    // 这样前端 SSE 能边跑边看 composer 等命令的进度，而不是等到结束才一次性灌出来。
    let logf: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>> = log_file.and_then(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
            .map(std::sync::Mutex::new)
            .map(std::sync::Arc::new)
    });
    let zap_tbl = lua
        .create_table()
        .map_err(|e| format!("创建 zap 表失败: {e}"))?;

    // zap.log
    {
        let log = log.clone();
        let logf = logf.clone();
        let f = lua.create_function(move |_, msg: String| {
            log.borrow_mut().push_str(&msg);
            log.borrow_mut().push('\n');
            if let Some(f) = logf.as_ref() {
                use std::io::Write;
                let _ = f.lock().unwrap().write_all(msg.as_bytes());
                let _ = f.lock().unwrap().write_all(b"\n");
            }
            Ok(())
        });
        zap_tbl
            .set("log", f.map_err(|e| format!("log 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.option
    {
        let options = ctx.options.clone();
        let f = lua.create_function(move |_, key: String| {
            Ok(options.get(&key).cloned().unwrap_or_default())
        });
        zap_tbl
            .set("option", f.map_err(|e| format!("option 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.exec（仅 system）/ zap.exec_as_user（仅 site）/ zap.run（按 scope 自动分派）
    {
        let scope = ctx.scope.clone();
        let run_user = ctx.run_user.clone();
        let logf_exec = logf.clone();
        // 每个闭包各持一份 cancel / child_pid 的 clone（Arc 不 Copy，不能共享同一个绑定）
        let cancel_exec = cancel.clone();
        let cancel_user = cancel.clone();
        let cancel_run = cancel.clone();
        let child_pid_exec = child_pid.clone();
        let child_pid_user = child_pid.clone();
        let child_pid_run = child_pid.clone();
        let logf_user = logf.clone();
        let logf_run = logf.clone();
        let f_exec = lua.create_function(move |_, (prog, args, opts): (String, mlua::Table, mlua::Value)| {
            if scope != "system" {
                return Err(mlua::Error::RuntimeError(
                    "只有 scope: system 才能 zap.exec（以 root 执行），请改用 zap.run 或 zap.exec_as_user".into(),
                ));
            }
            let cwd = extract_cwd(&opts).map_err(mlua::Error::RuntimeError)?;
            run_capture(
                None,
                &prog,
                &table_to_vec(&args),
                None,
                logf_exec.clone(),
                cancel_exec.clone(),
                child_pid_exec.clone(),
                cwd,
            )
            .map_err(mlua::Error::RuntimeError)
        });
        zap_tbl
            .set("exec", f_exec.map_err(|e| format!("exec 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;

        let scope_run = ctx.scope.clone();
        let run_user_run = ctx.run_user.clone();
        let f_user = lua.create_function(move |_, (prog, args, opts): (String, mlua::Table, mlua::Value)| match &run_user
        {
            Some(u) => {
                let cwd = extract_cwd(&opts).map_err(mlua::Error::RuntimeError)?;
                run_capture(
                    Some(u),
                    &prog,
                    &table_to_vec(&args),
                    None,
                    logf_user.clone(),
                    cancel_user.clone(),
                    child_pid_user.clone(),
                    cwd,
                )
                .map_err(mlua::Error::RuntimeError)
            }
            None => Err(mlua::Error::RuntimeError("site 作用域插件未提供运行账号".into())),
        });
        zap_tbl
            .set("exec_as_user", f_user.map_err(|e| format!("exec_as_user 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;

        // zap.run：按 scope 自动选 root / 站点账号，插件不必自己判断作用域
        let f_run = lua.create_function(move |_, (prog, args, opts): (String, mlua::Table, mlua::Value)| {
            let user = match scope_run.as_str() {
                "site" | "user" => run_user_run.clone(),
                _ => None,
            };
            if (scope_run == "site" || scope_run == "user") && user.is_none() {
                return Err(mlua::Error::RuntimeError("该作用域插件未提供运行账号".into()));
            }
            let cwd = extract_cwd(&opts).map_err(mlua::Error::RuntimeError)?;
            run_capture(
                user.as_deref(),
                &prog,
                &table_to_vec(&args),
                None,
                logf_run.clone(),
                cancel_run.clone(),
                child_pid_run.clone(),
                cwd,
            )
            .map_err(mlua::Error::RuntimeError)
        });
        zap_tbl
            .set("run", f_run.map_err(|e| format!("run 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;

        // zap.try_run：同上，但失败不抛错，返回 (ok, output)
        let scope_try = ctx.scope.clone();
        let run_user_try = ctx.run_user.clone();
        let logf_try = logf.clone();
        let cancel_try = cancel.clone();
        let child_pid_try = child_pid.clone();
        let f_try = lua.create_function(move |_, (prog, args, opts): (String, mlua::Table, mlua::Value)| {
            let user = match scope_try.as_str() {
                "site" | "user" => run_user_try.clone(),
                _ => None,
            };
            let argv = table_to_vec(&args);
            let cwd = extract_cwd(&opts).map_err(mlua::Error::RuntimeError)?;
            match run_capture(
                user.as_deref(),
                &prog,
                &argv,
                None,
                logf_try.clone(),
                cancel_try.clone(),
                child_pid_try.clone(),
                cwd,
            ) {
                Ok(s) => Ok((true, s)),
                Err(s) => Ok((false, s)),
            }
        });
        zap_tbl
            .set("try_run", f_try.map_err(|e| format!("try_run 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.read_file / zap.write_file / zap.append_file（按 scope 降权的子进程实现）
    {
        let scope = ctx.scope.clone();
        let run_user = ctx.run_user.clone();
        for (name, redirect) in [("read_file", "<"), ("write_file", ">"), ("append_file", ">>")] {
            let scope = scope.clone();
            let run_user = run_user.clone();
            let f = lua.create_function(
                move |_, (path, content): (String, Option<String>)| {
                    let script = format!("cat {} \"$1\"", redirect);
                    let stdin = if redirect == "<" { None } else { Some(content.unwrap_or_default()) };
                    let user = match scope.as_str() {
                        "site" | "user" => run_user.clone(),
                        _ => None,
                    };
                    if (scope == "site" || scope == "user") && user.is_none() {
                        return Err(mlua::Error::RuntimeError("该作用域插件未提供运行账号".into()));
                    }
                    if path.trim().is_empty() {
                        return Err(mlua::Error::RuntimeError("路径不能为空".into()));
                    }
                    run_capture(
                        user.as_deref(),
                        "sh",
                        &["-c".to_string(), script, "sh".to_string(), path],
                        stdin,
                        None,
                        Arc::new(AtomicBool::new(false)),
                        Arc::new(std::sync::Mutex::new(None)),
                        None,
                    )
                    .map_err(mlua::Error::RuntimeError)
                },
            );
            zap_tbl
                .set(name, f.map_err(|e| format!("{name} 注册失败: {e}"))?)
                .map_err(|e| format!("{e}"))?;
        }
    }
    // zap.site_root / zap.site_linux_user / zap.home_dir / zap.plugin_dir / zap.scope
    {
        let run_root = ctx.run_root.clone();
        let run_user3 = ctx.run_user.clone();
        let scope_for_ctx = ctx.scope.clone();
        let home_dir = ctx.home.clone();
        let plugin_dir = ctx.plugin_dir.display().to_string();
        let scope = ctx.scope.clone();
        // site_root / site_linux_user 仅 scope=site 有值；scope=user 返回空（无站点概念）
        let f_root = {
            let s = scope_for_ctx.clone();
            lua.create_function(move |_, ()| {
                Ok(if s == "site" {
                    run_root.clone().unwrap_or_default()
                } else {
                    String::new()
                })
            })
        };
        zap_tbl
            .set("site_root", f_root.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
        let f_user = {
            let s = scope_for_ctx.clone();
            lua.create_function(move |_, ()| {
                Ok(if s == "site" {
                    run_user3.clone().unwrap_or_default()
                } else {
                    String::new()
                })
            })
        };
        zap_tbl
            .set("site_linux_user", f_user.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
        let f_home = lua.create_function(move |_, ()| Ok(home_dir.clone()));
        zap_tbl
            .set("home_dir", f_home.map_err(|e| format!("home_dir 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;
        let f_dir = lua.create_function(move |_, ()| Ok(plugin_dir.clone()));
        zap_tbl
            .set("plugin_dir", f_dir.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
        let f_scope = lua.create_function(move |_, ()| Ok(scope.clone()));
        zap_tbl
            .set("scope", f_scope.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.canceled：异步插件可轮询判断用户是否点了取消，用于提前自己收尾
    {
        let cancel = cancel.clone();
        let f = lua.create_function(move |_, ()| Ok(cancel.load(Ordering::SeqCst)));
        zap_tbl
            .set("canceled", f.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.time / zap.date
    {
        let f_time = lua.create_function(move |_, ()| Ok(chrono::Utc::now().timestamp()));
        zap_tbl
            .set("time", f_time.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
        let f_date = lua.create_function(move |_, (fmt, ts): (String, Option<i64>)| {
            let t = match ts {
                Some(t) => chrono::DateTime::<chrono::Utc>::from_timestamp(t, 0)
                    .ok_or_else(|| mlua::Error::RuntimeError("时间戳超出范围".into()))?,
                None => chrono::Utc::now(),
            };
            Ok(t.format(&fmt).to_string())
        });
        zap_tbl
            .set("date", f_date.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.env：只放开白名单，避免把 zapexec 进程里的敏感变量（密钥等）泄漏给插件
    {
        let f_env = lua.create_function(move |_, key: String| {
            let allowed = key.starts_with("ZAP_")
                || matches!(key.as_str(), "PATH" | "HOME" | "USER" | "SHELL" | "LANG" | "TMPDIR");
            if !allowed {
                return Ok(String::new());
            }
            Ok(std::env::var(&key).unwrap_or_default())
        });
        zap_tbl
            .set("env", f_env.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
    }
    // zap.json_encode / zap.json_decode
    {
        let f_enc = lua.create_function(move |_, v: mlua::Value| {
            let j = lua_value_to_json(&v).map_err(mlua::Error::RuntimeError)?;
            Ok(serde_json::to_string(&j).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?)
        });
        zap_tbl
            .set("json_encode", f_enc.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
        let f_dec = lua.create_function(move |lua, s: String| {
            let v: Value = serde_json::from_str(&s)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON 解析失败: {e}")))?;
            json_to_lua(lua, &v)
        });
        zap_tbl
            .set("json_decode", f_dec.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
    }

    lua.globals()
        .set("zap", zap_tbl)
        .map_err(|e| format!("注入 zap 失败: {e}"))?;

    // 自动加载公共函数库（系统级 → 插件自带 lib/），先于 main.lua
    for dir in shared_lib_dirs(&ctx.home, &ctx.plugin_dir) {
        if let Err(e) = load_lua_dir(&lua, &dir) {
            return Err(format!("加载公共函数库 {} 失败: {e}", dir.display()));
        }
    }

    // 执行插件主体（定义 on_run）
    lua.load(code)
        .set_name("main.lua")
        .exec()
        .map_err(|e| format!("插件脚本错误: {e}"))?;

    // 入口分发：先找 `on_<action>`（HTML 界面可以定义多个入口），没有再回落到 on_run。
    let on_run: mlua::Function = match lua.globals().get(format!("on_{}", ctx.action)) {
        Ok(f) => f,
        Err(_) => lua
            .globals()
            .get("on_run")
            .map_err(|e| format!("插件未定义 on_run / on_{}: {e}", ctx.action))?,
    };
    let ctx_tbl = lua.create_table().map_err(|e| format!("{e}"))?;
    ctx_tbl.set("action", ctx.action.clone()).map_err(|e| format!("{e}"))?;
    ctx_tbl.set("scope", ctx.scope.clone()).map_err(|e| format!("{e}"))?;
    on_run
        .call::<_, ()>(ctx_tbl)
        .map_err(|e| format!("on_run 执行失败: {e}"))?;

    Ok(log.borrow().clone())
}

/// 把一个目录下的 `*.lua` 按文件名顺序加载进沙箱（公共函数库）。
fn load_lua_dir(lua: &mlua::Lua, dir: &Path) -> Result<(), String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(()); // 目录不存在很正常（公共库可选）
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("lua"))
        .collect();
    files.sort();
    for p in files {
        let code = std::fs::read_to_string(&p)
            .map_err(|e| format!("读取 {} 失败: {e}", p.display()))?;
        lua.load(&code)
            .set_name(p.display().to_string())
            .exec()
            .map_err(|e| format!("{} 执行失败: {e}", p.display()))?;
    }
    Ok(())
}

/// Lua 值 → JSON（沙箱里没有 serde feature，手动转换）。
fn lua_value_to_json(v: &mlua::Value) -> Result<Value, String> {
    match v {
        mlua::Value::Nil => Ok(Value::Null),
        mlua::Value::Boolean(b) => Ok(Value::Bool(*b)),
        mlua::Value::Integer(i) => Ok(json!(i)),
        mlua::Value::Number(n) => Ok(json!(n)),
        mlua::Value::String(s) => Ok(Value::String(
            String::from_utf8_lossy(&s.as_bytes()).to_string(),
        )),
        mlua::Value::Table(t) => {
            // 全是 1..n 的连续整数键 → 数组，否则当对象
            let mut is_array = true;
            let mut max_idx = 0usize;
            for pair in t.clone().pairs::<mlua::Value, mlua::Value>() {
                let (k, _) = pair.map_err(|e| e.to_string())?;
                match k {
                    mlua::Value::Integer(i) if i >= 1 => max_idx = max_idx.max(i as usize),
                    _ => {
                        is_array = false;
                        break;
                    }
                }
            }
            if is_array {
                let mut arr = Vec::with_capacity(max_idx);
                for i in 1..=max_idx as i64 {
                    let item: mlua::Value = t.get(i).map_err(|e| e.to_string())?;
                    arr.push(lua_value_to_json(&item)?);
                }
                Ok(Value::Array(arr))
            } else {
                let mut map = serde_json::Map::new();
                for pair in t.clone().pairs::<String, mlua::Value>() {
                    let (k, val) = pair.map_err(|e| e.to_string())?;
                    map.insert(k, lua_value_to_json(&val)?);
                }
                Ok(Value::Object(map))
            }
        }
        other => Err(format!("无法转成 JSON 的类型: {}", other.type_name())),
    }
}

/// JSON → Lua 值。
fn json_to_lua<'a>(lua: &'a mlua::Lua, v: &Value) -> Result<mlua::Value<'a>, mlua::Error> {
    Ok(match v {
        Value::Null => mlua::Value::Nil,
        Value::Bool(b) => mlua::Value::Boolean(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => mlua::Value::Integer(i),
            None => mlua::Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => mlua::Value::String(lua.create_string(s)?),
        Value::Array(a) => {
            let t = lua.create_table()?;
            for (i, item) in a.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, item)?)?;
            }
            mlua::Value::Table(t)
        }
        Value::Object(m) => {
            let t = lua.create_table()?;
            for (k, item) in m {
                t.set(k.as_str(), json_to_lua(lua, item)?)?;
            }
            mlua::Value::Table(t)
        }
    })
}

fn table_to_vec(t: &mlua::Table) -> Vec<String> {
    let mut v = Vec::new();
    for pair in t.clone().pairs::<i64, String>() {
        if let Ok((_, s)) = pair {
            v.push(s);
        }
    }
    v
}

/// 从 Lua 侧 opts 表（zap.exec/run/try_run 的第三个可选参数）里取 `cwd`，
/// 用于指定子进程工作目录（如 git 需要在仓库目录里跑）。
/// 校验：必须是绝对路径、存在且为目录；空值视为「不指定」。
fn extract_cwd(opts: &mlua::Value) -> Result<Option<PathBuf>, String> {
    let t = match opts {
        mlua::Value::Nil => return Ok(None),
        mlua::Value::Table(t) => t,
        _ => return Err("cwd 选项必须是表，例如 { cwd = '/abs/path' }".into()),
    };
    let raw = match t.get::<&str, mlua::Value>("cwd") {
        Ok(mlua::Value::String(s)) => String::from_utf8_lossy(&s.as_bytes()).to_string(),
        Ok(mlua::Value::Nil) => return Ok(None),
        _ => return Ok(None),
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let p = PathBuf::from(raw.trim());
    if !p.is_absolute() {
        return Err(format!("cwd 必须是绝对路径: {raw}"));
    }
    let canon = std::fs::canonicalize(&p).map_err(|e| format!("cwd 解析失败 {raw}: {e}"))?;
    if !canon.is_dir() {
        return Err(format!("cwd 不是目录: {raw}"));
    }
    Ok(Some(canon))
}

/// 以指定身份执行命令并捕获合并输出（stdout + stderr）。
///
/// `logf` 非空时（异步插件），子进程的标准输出/错误会被实时 tee 到该日志文件，
/// 这样前端 SSE 能边跑边看进度；同步插件传 `None`，行为与原来一致（结束一次性返回）。
/// `stdin` 非空时把内容喂给子进程的标准输入（`write_file` / `append_file` 用）。
/// `cwd` 非空时切换到该目录执行（插件用于「在指定目录里跑命令」，如 git）。
/// `cancel` / `child_pid` 用于异步插件的运行中取消：被取消时看门狗会杀掉本进程（组），
/// 这里检测标志后提前结束拷贝循环。
fn run_capture(
    user: Option<&str>,
    program: &str,
    args: &[String],
    stdin: Option<String>,
    logf: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>>,
    cancel: Arc<AtomicBool>,
    child_pid: Arc<std::sync::Mutex<Option<(u32, bool)>>>,
    cwd: Option<PathBuf>,
) -> Result<String, String> {
    use std::io::{Read, Write};
    let session_leader = user.is_some(); // exec_as_user 走 setsid，pid 即进程组号
    // 资源笼子：降权命令（站点账号 / 面板用户）套一份，防 fork 炸弹 / 写满盘 / 吃满 CPU。
    // root 命令（scope=system，管理员插件）保持官方脚本策略不做限制。
    let resource = user.map(|_| {
        std::sync::Arc::new(super::resource::TaskResource::prepare(&format!(
            "zapplug-{}-{}",
            std::process::id(),
            PLUGIN_RES_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        )))
    });
    let mut cmd = match user {
        Some(u) => {
            let (mut c, acc) = super::user_cmd(program, u).map_err(|e| format!("降权失败: {e}"))?;
            let (uid, gid) = (acc.uid, acc.gid);
            let res_w = resource.clone();
            let _ = unsafe {
                c.pre_exec(move || {
                    libc::setsid();
                    super::cloexec_inherited_fds();
                    super::drop_privileges(uid, gid)?;
                    if let Some(res) = &res_w {
                        res.enter();
                    }
                    Ok(())
                })
            };
            c
        }
        None => super::root_cmd(program),
    };
    if let Some(cwd) = &cwd {
        cmd.current_dir(cwd);
    }
    let mut child = cmd
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("执行 {program} 失败: {e}"))?;
    // 登记 pid，供取消看门狗杀进程（组）
    *child_pid.lock().unwrap() = Some((child.id(), session_leader));

    // 有 stdin 就单开线程写入，写完立刻关闭管道（否则 cat 会一直等 EOF）
    let stdin_thread = match (child.stdin.take(), stdin) {
        (Some(mut sin), Some(data)) => Some(std::thread::spawn(move || {
            let _ = sin.write_all(data.as_bytes());
        })),
        _ => None,
    };

    // 两个线程分别把 stdout / stderr 拷进缓冲区，并（异步时）实时落盘
    let copy =
        |r: Option<Box<dyn std::io::Read + Send>>,
         logf: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>>,
         cancel: Arc<AtomicBool>|
         -> String {
        let mut s = String::new();
        if let Some(mut r) = r {
            let mut buf = [0u8; 4096];
            loop {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                match r.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        s.push_str(&chunk);
                        if let Some(f) = &logf {
                            let _ = f.lock().unwrap().write_all(chunk.as_bytes());
                            let _ = f.lock().unwrap().flush();
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        s
    };
    let logf_out = logf.clone();
    let cancel_out = cancel.clone();
    let stdout: Option<Box<dyn std::io::Read + Send>> =
        child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>);
    let stderr: Option<Box<dyn std::io::Read + Send>> =
        child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>);
    let t_out = std::thread::spawn(move || copy(stdout, logf_out, cancel_out));
    let t_err = std::thread::spawn(move || copy(stderr, logf.clone(), cancel.clone()));

    let status = child
        .wait()
        .map_err(|e| format!("等待 {program} 失败: {e}"))?;
    // 回收资源笼子（删掉本次命令的 cgroup 叶子目录，未部署 slice 时是空操作）
    if let Some(res) = resource {
        res.finish();
    }
    if let Some(t) = stdin_thread {
        let _ = t.join();
    }
    let mut s = t_out.join().unwrap_or_default();
    s.push_str(&t_err.join().unwrap_or_default());

    if !status.success() {
        return Err(format!(
            "命令退出码 {}:\n{}",
            status.code().unwrap_or(-1),
            s
        ));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 随发行包一起走的公共函数库必须能通过编译。
    ///
    /// 只 `load` 不 `exec`：库里引用的 `zap.*` 要真正跑起来才有，
    /// 这里只卡语法（漏了个 `end` 之类的错误就会在 CI 里炸出来，而不是等到插件跑不动）。
    #[test]
    fn shared_lib_compiles() {
        let src = include_str!("../../../data/plugins/_lib/zap.lua");
        let lua = mlua::Lua::new_with(sandbox_libs(), mlua::LuaOptions::default())
            .expect("创建 Lua 沙箱失败");
        lua.load(src)
            .set_name("zap.lua")
            .into_function()
            .expect("data/plugins/_lib/zap.lua 编译失败");
    }

    #[test]
    fn reserved_dirs_are_not_plugins() {
        // `_lib` / `lib` 是公共函数库目录，不能被当成插件扫出来
        assert!(is_reserved_dir("_lib"));
        assert!(is_reserved_dir(".git"));
        assert!(!is_reserved_dir("composer-create"));
    }

    /// `ui.html` 只能指向插件目录内的单个 `.html` 文件。
    #[test]
    fn ui_html_file_rejects_traversal() {
        let mk = |html: &str| -> serde_yaml::Value {
            serde_yaml::from_str(&format!("ui:\n  html: '{html}'\n")).expect("yaml 解析失败")
        };
        assert_eq!(ui_html_file(&mk("ui.html")).as_deref(), Some("ui.html"));
        for bad in [
            "../ui.html",
            "../../etc/passwd",
            "/etc/passwd",
            "sub/ui.html",
            "ui.htm",
            "",
            "a\\b.html",
            "ui.html;rm -rf /",
        ] {
            assert_eq!(ui_html_file(&mk(bad)), None, "{bad} 应被拒绝");
        }
        // 没声明 ui.html 时也不该有值（走结构化表单）
        let m: serde_yaml::Value =
            serde_yaml::from_str("ui:\n  placement: site.detail\n").expect("yaml 解析失败");
        assert_eq!(ui_html_file(&m), None);
    }

    #[test]
    fn plugin_name_validation() {
        assert!(is_plugin_name("composer-create"));
        assert!(is_plugin_name("my_plugin2"));
        // 路径穿越 / 空名必须被挡掉
        assert!(!is_plugin_name(".."));
        assert!(!is_plugin_name(""));
        assert!(!is_plugin_name("a/b"));
        assert!(!is_plugin_name("a b"));
    }

    /// 沙箱不能放开 io / os / package / debug。
    #[test]
    fn sandbox_excludes_dangerous_libs() {
        let libs = sandbox_libs();
        assert!(libs.contains(mlua::StdLib::STRING));
        assert!(libs.contains(mlua::StdLib::TABLE));
        assert!(!libs.contains(mlua::StdLib::IO));
        assert!(!libs.contains(mlua::StdLib::OS));
        assert!(!libs.contains(mlua::StdLib::PACKAGE));
        assert!(!libs.contains(mlua::StdLib::DEBUG));
    }

    /// 公共函数库必须真的对插件可见：跑一个只用 helpers 的插件，检查输出。
    ///
    /// 库文件通过 `<plugin_dir>/lib/` 注入，这样测试不依赖部署环境里
    /// 是否真的存在系统级 `_lib` 目录。
    #[test]
    fn shared_lib_is_visible_to_plugins() {
        let dir = std::env::temp_dir().join(format!("zap-plugin-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).expect("建临时插件目录失败");
        std::fs::write(
            dir.join("lib/zap.lua"),
            include_str!("../../../data/plugins/_lib/zap.lua"),
        )
        .expect("写库文件失败");

        let ctx = RunCtx {
            scope: "site".to_string(),
            run_user: Some("nobody".to_string()),
            run_root: Some("/tmp".to_string()),
            home: "/tmp".to_string(),
            plugin_dir: dir.clone(),
            options: HashMap::new(),
            action: "run".to_string(),
        };
        let code = r#"
            function on_run(ctx)
              zap.log("trim=[" .. zap.str.trim("  hi  ") .. "]")
              zap.log("join=" .. zap.path.join("/a", "b/c"))
              zap.log("count=" .. zap.tbl.count({ 1, 2, 3 }))
              zap.log("quote=" .. zap.q("a b"))
              zap.log("within=" .. tostring(zap.path.within("/site", "/site/x/y")))
              zap.log("site=" .. zap.path.site("sub"))
            end
        "#;
        let out = run_lua(
            code,
            &ctx,
            None,
            Arc::new(AtomicBool::new(false)),
            Arc::new(std::sync::Mutex::new(None)),
        )
        .expect("插件执行失败");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(out.contains("trim=[hi]"), "公共库 zap.str 不可用: {out}");
        assert!(out.contains("join=/a/b/c"), "公共库 zap.path 不可用: {out}");
        assert!(out.contains("count=3"), "公共库 zap.tbl 不可用: {out}");
        assert!(out.contains("quote='a b'"), "公共库 zap.q 不可用: {out}");
        assert!(out.contains("within=true"), "zap.path.within 行为不对: {out}");
        assert!(out.contains("site=/tmp/sub"), "zap.path.site 行为不对: {out}");
    }

    #[test]
    fn path_helpers() {
        // 插件统一装在系统目录 $ZAP_PATH/plugins；用户级已移除
        let sys = plugin_base();
        assert!(sys.ends_with("plugins"));
    }
}
