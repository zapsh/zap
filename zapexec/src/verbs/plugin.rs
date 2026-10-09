// SPDX-License-Identifier: AGPL-3.0-only
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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
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
pub(crate) fn write_appstore_plugin_source(cat: &str, name: &str, pkg_path: &str, base: &Path) {
    if cat != "plugins" {
        return;
    }
    let dir = base.join(name);
    if !dir.is_dir() {
        return;
    }
    if let Err(e) = write_install_meta(&dir, "appstore", pkg_path, chrono::Utc::now().timestamp()) {
        warn!("AppStore 插件 {name} 写回安装来源失败: {e}");
    }
}

fn manifest_str<'a>(m: &'a serde_yaml::Value, key: &str) -> Option<&'a str> {
    m.get(key).and_then(|v| v.as_str())
}

/// 是否属于「只读」角色（面板的 demo 演示账号）。
///
/// 只在 zapd 把 roles 传过来时才拦：自检 / CLI 调用这些没有面板角色的场合返回 false。
fn is_readonly_roles(roles: Option<&str>) -> bool {
    roles.unwrap_or("").split(',').any(|r| r.trim() == "demo")
}

/// manifest 里 `ui.placement` 挂哪些位置。
///
/// 允许单个字符串（旧写法）或字符串数组；一个插件可以同时出现在多个槽位
/// （例如既挂文件编辑器工具栏、又挂仪表盘卡片）。未知槽位不拦，宿主只渲染
/// 它认识的那些，这样新增槽位时老插件不至于因为版本号被拒。
fn placements_of(m: &serde_yaml::Value) -> Vec<String> {
    let ui = m.get("ui");
    let Some(v) = ui.and_then(|u| u.get("placement")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match v {
        serde_yaml::Value::Sequence(seq) => {
            for item in seq {
                if let Some(s) = item.as_str() {
                    let s = s.trim();
                    if !s.is_empty() && !out.iter().any(|x| x == s) {
                        out.push(s.to_string());
                    }
                }
            }
        }
        other => {
            if let Some(s) = other.as_str() {
                let s = s.trim();
                if !s.is_empty() {
                    out.push(s.to_string());
                }
            }
        }
    }
    out
}

/// 某个 action 的规格插件网的布尔字段（async / dangerous）。
///
/// `actions` 允许两种写法：
///   actions: { run: 运行 }                                  # 只有文案
///   actions: { clone: { label: 克隆, async: true, dangerous: false } }
fn action_flag(m: &serde_yaml::Value, action: &str, key: &str) -> bool {
    m.get("actions")
        .and_then(|a| a.get(action))
        .and_then(|v| v.as_mapping())
        .and_then(|mp| mp.get(serde_yaml::Value::String(key.to_string())))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// action 的按钮文案：`actions.<name>.label`，或旧写法里直接给的字符串。
fn action_label(m: &serde_yaml::Value, action: &str) -> Option<String> {
    let v = m.get("actions").and_then(|a| a.get(action))?;
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    v.as_mapping()
        .and_then(|mp| mp.get(serde_yaml::Value::String("label".to_string())))
        .and_then(|l| l.as_str())
        .map(|s| s.to_string())
}

/// 把 manifest 的 `actions` 摊平成前端好用的元信息数组：
/// `[{ name, label, async, dangerous }]`。
fn action_specs(m: &serde_yaml::Value) -> Vec<Value> {
    let mut out = Vec::new();
    let Some(actions) = m.get("actions").and_then(|a| a.as_mapping()) else {
        return out;
    };
    for (k, _) in actions {
        let Some(name) = k.as_str() else { continue };
        out.push(json!({
            "name": name,
            "label": action_label(m, name).unwrap_or_else(|| name.to_string()),
            "async": action_flag(m, name, "async"),
            "dangerous": action_flag(m, name, "dangerous"),
        }));
    }
    out
}

/// 把一个插件目录整理成前端需要的描述对象。
/// ── 插件文案国际化 ─────────────────────────────────────────
///
/// manifest 里可以带一张翻译表：
///
/// ```yaml
/// i18n:
///   en-US:
///     title: Git
///     description: Repository operations
///     actions: { status: Status, push: Push }
///     options:
///       - { name: cwd, label: Directory, desc: Repository root }
///   zh-CN: { title: Git 版本库 }
/// ```
///
/// describe() 先按基准字段出一份结果，再按调用方语言把这张表盖上去（缺失的键保留基准值）。
/// 语言名匹配顺序：`zh-CN` → `zh` → 兜底不覆盖，因此只有部分翻译也能用。
/// 取 manifest 里命中 `lang` 的那张翻译表。
fn i18n_table<'a>(m: &'a serde_yaml::Value, lang: Option<&str>) -> Option<&'a serde_yaml::Mapping> {
    let lang = lang.unwrap_or("").trim();
    if lang.is_empty() {
        return None;
    }
    let i18n = m.get("i18n").and_then(|v| v.as_mapping())?;
    // 候选顺序：`zh-CN` 全名 → 面板支持的两个包 `en-US` / `zh-CN`（按主语言）→ 主语言 `zh`。
    // 支持 `en-GB` 这类变体命中 `en-US`，不必作者在 manifest 里穷举所有地区写法。
    let primary = lang.split('-').next().unwrap_or(lang);
    let variant = match primary {
        "en" => Some("en-US"),
        "zh" => Some("zh-CN"),
        _ => None,
    };
    for key in [Some(lang), variant, Some(primary)].into_iter().flatten() {
        if let Some(v) = i18n
            .get(serde_yaml::Value::String(key.to_string()))
            .and_then(|v| v.as_mapping())
        {
            return Some(v);
        }
    }
    None
}

fn i18n_str<'a>(t: &'a serde_yaml::Mapping, key: &str) -> Option<&'a str> {
    t.get(serde_yaml::Value::String(key.to_string()))
        .and_then(|v| v.as_str())
}

/// 把翻译表盖到 `describe()` 的结果上（只覆盖翻译表里真的给了的键）。
fn apply_i18n(info: &mut Value, table: &serde_yaml::Mapping) {
    let Some(obj) = info.as_object_mut() else {
        return;
    };
    for key in ["title", "description", "label", "tab"] {
        if let Some(s) = i18n_str(table, key) {
            obj.insert(key.to_string(), Value::String(s.to_string()));
        }
    }
    // actions：`<name>: 文案` 或 `<name>: { label: 文案 }`
    if let Some(actions) = table
        .get(serde_yaml::Value::String("actions".to_string()))
        .and_then(|v| v.as_mapping())
    {
        let mut labels: Vec<(String, String)> = Vec::new();
        for (k, v) in actions {
            let Some(name) = k.as_str() else { continue };
            let Some(label) = (if let Some(s) = v.as_str() {
                Some(s.to_string())
            } else {
                v.get(serde_yaml::Value::String("label".to_string()))
                    .and_then(|l| l.as_str())
                    .map(|s| s.to_string())
            }) else {
                continue;
            };
            labels.push((name.to_string(), label));
        }
        // 基准 actions 是「名字 → 文案」；action_specs 是同名的数组结构，两处都要覆盖
        if let Some(base) = obj.get_mut("actions").and_then(|v| v.as_object_mut()) {
            for (name, label) in &labels {
                base.insert(name.clone(), Value::String(label.clone()));
            }
        }
        if let Some(specs) = obj.get_mut("action_specs").and_then(|v| v.as_array_mut()) {
            for spec in specs.iter_mut() {
                let Some(sname) = spec.get("name").and_then(|n| n.as_str()) else {
                    continue;
                };
                if let Some((_, label)) = labels.iter().find(|(n, _)| n == sname)
                    && let Some(o) = spec.as_object_mut()
                {
                    o.insert("label".to_string(), Value::String(label.clone()));
                }
            }
        }
    }
    // options：按 name 匹配，覆盖 label / desc / placeholder（结构定义权仍归基准 manifest）
    if let Some(opts) = table
        .get(serde_yaml::Value::String("options".to_string()))
        .and_then(|v| v.as_sequence())
        && let Some(base) = obj.get_mut("options").and_then(|v| v.as_array_mut())
    {
        for ov in opts {
            let Some(om) = ov.as_mapping() else { continue };
            let Some(name) = i18n_str(om, "name") else {
                continue;
            };
            for bo in base.iter_mut() {
                if bo.get("name").and_then(|n| n.as_str()) != Some(name) {
                    continue;
                }
                let Some(bo) = bo.as_object_mut() else {
                    continue;
                };
                for key in ["label", "desc", "placeholder"] {
                    if let Some(s) = i18n_str(om, key) {
                        bo.insert(key.to_string(), Value::String(s.to_string()));
                    }
                }
            }
        }
    }
}

fn describe(dir: &Path, name: &str) -> Result<Value, String> {
    describe_lang(dir, name, None)
}

fn describe_lang(dir: &Path, name: &str, lang: Option<&str>) -> Result<Value, String> {
    let m = read_manifest(dir)?;
    let ui = m.get("ui");
    let ui_str = |k: &str| {
        ui.and_then(|u| u.get(k))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let placements = placements_of(&m);
    let placement = placements.first().cloned().unwrap_or_default();
    // 安装信息优先读 manifest 里的 `zap_install`（安装时写回），兼容旧版仍带
    // `.zap-install.json` 的插件（read_install_meta 仅作兜底）。
    let install_yaml = m.get("zap_install");
    let meta_json = read_install_meta(dir);
    let meta_str = |k: &str| -> String {
        if let Some(s) = install_yaml.and_then(|v| v.get(k)).and_then(|v| v.as_str()) {
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
        if s.is_empty() { name.to_string() } else { s }
    };
    let mut info = json!({
        "name": name,
        "title": manifest_str(&m, "title").unwrap_or(name),
        "async": m.get("async").and_then(|v| v.as_bool()).unwrap_or(false),
        "scope": manifest_str(&m, "scope").unwrap_or("system"),
        // placement 保留单值（旧前端只看这个），placements 才是完整的挂载位置列表
        "placement": placement,
        "placements": placements,
        "label": label,
        "icon": ui_str("icon"),
        "tab": ui_str("tab"),
        "options": serde_json::to_value(m.get("options").cloned().unwrap_or(serde_yaml::Value::Null)).unwrap_or(Value::Null),
        // actions 保持「名字 → 文案」的旧形状（前端按钮文案从这里取），
        // action_specs 才带 async / dangerous 这类每个动作自己的开关
        "actions": serde_json::to_value(m.get("actions").cloned().unwrap_or(serde_yaml::Value::Null)).unwrap_or(Value::Null),
        "action_specs": action_specs(&m),
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
    });
    // 面板语言：命中 manifest 的 i18n 表就把文案盖上去（缺失的键保留基准语言）
    if let Some(table) = i18n_table(&m, lang) {
        apply_i18n(&mut info, table);
    }
    Ok(info)
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

/// 插件界面基础层（UIKit）的两个文件，随发行包部署到 `$ZAP_PATH/data/plugins/_lib/`。
const UIKIT_CSS: &str = "ui.css";
const UIKIT_JS: &str = "ui.js";

/// 读取插件自带的 HTML 界面文件内容。
///
/// 前端把它塞进 `sandbox="allow-scripts"` 的 iframe 里渲染 —— 不含
/// `allow-same-origin`，所以这份 HTML 拿不到面板的 DOM / Cookie / localStorage，
/// 也发不出带凭据的请求；它要调后端只能走 `postMessage` 让父页面代跑
/// `plugin.run`，权限与 scope 仍旧由后端把关。
///
/// 返回前把 UIKit（主题 CSS + `zap.ui.*` 运行时，见 `data/plugins/_lib/`）
/// 注入进去：样式插到 `</head>` 前、脚本插到 `</body>` 前，插件不用再自己
/// 从零写一套按钮 / 表格样式。文件缺失时原样返回，不影响老插件。
pub async fn plugin_ui(
    _actor: String,
    _home: String,
    name: String,
    lang: Option<String>,
) -> Response {
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
        Ok(html) => Response::ok(
            "ok",
            Some(json!({
                "html": inject_uikit(
                    &html,
                    &zap_path().join("data/plugins/_lib"),
                    lang.as_deref(),
                    m.get("i18n"),
                ),
                "lang": lang.unwrap_or_default(),
            })),
        ),
        Err(e) => Response::err(-1, format!("读取界面文件失败: {e}")),
    }
}

/// 把 UIKit 的样式 / 脚本插进插件的 HTML：
/// `<style>…</style>` 塞进 `</head>` 前（没有 head 就塞最前面），
/// `<script>…</script>` 塞进 `</body>` 前（没有 body 就塞最后），
/// 无论缺哪个文件都原样返回，保证老插件与未部署 UIKit 的环境不受影响。
fn inject_uikit(
    html: &str,
    lib_dir: &std::path::Path,
    lang: Option<&str>,
    i18n: Option<&serde_yaml::Value>,
) -> String {
    let css = std::fs::read_to_string(lib_dir.join(UIKIT_CSS)).unwrap_or_default();
    let js = std::fs::read_to_string(lib_dir.join(UIKIT_JS)).unwrap_or_default();
    if css.is_empty() && js.is_empty() {
        return html.to_string();
    }
    let mut out = html.to_string();
    // manifest 的 `i18n` 表整份注入成 `window.__ZAP_I18N__`：插件 HTML 里的
    // T() / applyI18n 全靠它查译文，没有它就只能显示基准（中文）文案。
    if let Some(t) = i18n
        && let Ok(json) = serde_json::to_string(t)
        && !matches!(json.as_str(), "null" | "{}")
    {
        // `</` 会提前闭合 script 标签；`<\/` 在 JSON 里等价于 `/`，安全
        let safe = json.replace("</", "<\\/");
        out.insert_str(
            0,
            &format!("<script>window.__ZAP_I18N__={safe};</script>\n"),
        );
    }
    // 面板语言先落地：插在最前面，UIKit 与插件 HTML 都能读到 `zap.ui.lang`。
    // UIKit 里提供的 `zap.ui.t({'zh-CN':…, 'en-US':…})` 就靠它选文案，
    // 插件界面因此能跟随 Element Plus 的语言切换。
    if let Some(l) = lang.filter(|s| !s.trim().is_empty()) {
        out.insert_str(
            0,
            &format!(
                "<script>window.__ZAP_LANG__=\"{}\";window.zap=window.zap||{{}};zap.lang=\"{}\";</script>\n",
                l.replace('\\', "\\\\").replace('"', "\\\""),
                l.replace('\\', "\\\\").replace('"', "\\\"")
            ),
        );
    }
    if !css.is_empty() {
        let block = format!("<style>\n{css}\n</style>\n");
        match out.to_lowercase().rfind("</head>") {
            Some(i) => out.insert_str(i, &block),
            None => out.insert_str(0, &block),
        }
    }
    if !js.is_empty() {
        let block = format!("<script>\n{js}\n</script>\n");
        match out.to_lowercase().rfind("</body>") {
            Some(i) => out.insert_str(i, &block),
            None => out.push_str(&block),
        }
    }
    out
}

// ── 插件级持久化配置（KV）─────────────────────────────────
//
// 插件目录是 root 所有，Lua 侧无法写偏好 / 凭证；这里由 zapexec 代管一份
// 「插件 + 面板用户」维度的 KV：`$ZAP_PATH/data/plugins/config/<plugin>.yaml`，
// 形状是 `<面板用户名>: { <键>: <值> }`。Lua 里通过
// `zap.config.get(k)` / `zap.config.set(k, v)` 读写（见 `zap.lua`）。

fn plugin_config_dir() -> PathBuf {
    zap_path().join("data/plugins/config")
}

/// `<plugin>.yaml` 的路径；插件名已过 `is_plugin_name`，不会穿越目录。
fn plugin_config_file(name: &str) -> Result<PathBuf, String> {
    if !is_plugin_name(name) {
        return Err("非法插件名".to_string());
    }
    Ok(plugin_config_dir().join(format!("{name}.yaml")))
}

/// 读出整个 KV 文件（`<用户>` → {`<键>`: `<值>`}），不存在 / 损坏都当空表。
fn read_config_store(path: &Path) -> serde_yaml::Mapping {
    let Ok(txt) = std::fs::read_to_string(path) else {
        return serde_yaml::Mapping::new();
    };
    if txt.trim().is_empty() {
        return serde_yaml::Mapping::new();
    }
    match serde_yaml::from_str::<serde_yaml::Value>(&txt) {
        Ok(v) => v.as_mapping().cloned().unwrap_or_default(),
        Err(e) => {
            warn!("插件配置文件 {} 解析失败，按空处理: {e}", path.display());
            serde_yaml::Mapping::new()
        }
    }
}

/// 原子写回（tmp + rename），顺手收紧权限：配置里可能放着 token / 偏好。
fn write_config_store(path: &Path, store: &serde_yaml::Mapping) -> Result<(), String> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    let txt = serde_yaml::to_string(store).map_err(|e| format!("序列化配置失败: {e}"))?;
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, txt).map_err(|e| format!("写插件配置失败: {e}"))?;
    #[cfg(unix)]
    let _ = std::fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    std::fs::rename(&tmp, path).map_err(|e| format!("插件配置落盘失败: {e}"))
}

/// 读某个插件对当前用户的配置。
pub async fn plugin_config_get(actor: String, _home: String, name: String) -> Response {
    let path = match plugin_config_file(&name) {
        Ok(p) => p,
        Err(e) => return Response::err(-1, e),
    };
    let store = read_config_store(&path);
    let mut out = serde_json::Map::new();
    let user_key = serde_yaml::Value::String(actor);
    if let Some(serde_yaml::Value::Mapping(m)) = store.get(&user_key) {
        for (k, v) in m {
            if let Some(k) = k.as_str() {
                let s = match v {
                    serde_yaml::Value::String(s) => s.clone(),
                    other => other.as_str().unwrap_or("").to_string(),
                };
                out.insert(k.to_string(), Value::String(s));
            }
        }
    }
    Response::ok("ok", Some(json!({ "config": out })))
}

/// 写某个插件对当前用户的配置：传进来的键覆盖旧值，未提到的键保留。
pub async fn plugin_config_set(
    actor: String,
    _home: String,
    name: String,
    config: HashMap<String, String>,
) -> Response {
    let path = match plugin_config_file(&name) {
        Ok(p) => p,
        Err(e) => return Response::err(-1, e),
    };
    let mut store = read_config_store(&path);
    let user_key = serde_yaml::Value::String(actor);
    let mut entry = match store.get(&user_key) {
        Some(serde_yaml::Value::Mapping(m)) => m.clone(),
        _ => serde_yaml::Mapping::new(),
    };
    for (k, v) in config {
        if k.trim().is_empty() || k.len() > 128 {
            return Response::err(-1, "配置键非法（不能为空且不超过 128 字符）".to_string());
        }
        if v.len() > 8192 {
            return Response::err(-1, format!("配置项 {k} 超过 8KB 上限"));
        }
        entry.insert(serde_yaml::Value::String(k), serde_yaml::Value::String(v));
    }
    store.insert(user_key, serde_yaml::Value::Mapping(entry));
    match write_config_store(&path, &store) {
        Ok(()) => Response::ok("配置已保存", None),
        Err(e) => Response::err(-1, e),
    }
}

/// 列出插件（统一在系统级 `$ZAP_PATH/plugins`），按 placement 槽位 / scope 过滤。
pub async fn plugin_list(
    _actor: String,
    home: String,
    slot: Option<String>,
    scope: Option<String>,
    lang: Option<String>,
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
            let placements = placements_of(&m);
            if let Some(sc) = &scope
                && pl_scope != sc.as_str()
            {
                continue;
            }
            if let Some(sl) = &slot {
                // placement 可以是数组：挂在任一个槽位上就算命中
                if !placements.iter().any(|p| p == sl) {
                    continue;
                }
            }
            match describe_lang(&p, &name, lang.as_deref()) {
                Ok(v) => items.push(v),
                Err(e) => warn!("  插件 {name} 描述失败（跳过）: {e}"),
            }
        }
    }
    info!("plugin_list 完成: 返回 {} 个插件", items.len());
    Response::ok("ok", Some(json!(items)))
}

// ── 安装 / 卸载 ────────────────────────────────────────────

// ── 依赖声明与签名校验 ────────────────────────────────────

/// `requires.commands` 声明的命令必须在 PATH 上找得到，缺哪个就拒装。
///
/// 插件作者这样声明：
/// ```yaml
/// requires:
///   commands: [git, curl]
/// ```
/// 比跑起来才报 `command not found` 好排查得多。命令名卡死字符集，避免被拿来拼 shell。
fn check_required_commands(m: &serde_yaml::Value) -> Result<(), String> {
    let Some(req) = m.get("requires") else {
        return Ok(());
    };
    let Some(cmds) = req.get("commands").and_then(|v| v.as_sequence()) else {
        return Ok(());
    };
    let mut missing = Vec::new();
    for c in cmds {
        let Some(c) = c.as_str().map(|s| s.trim()).filter(|s| !s.is_empty()) else {
            continue;
        };
        if !c
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        {
            return Err(format!("requires.commands 含非法命令名: {c}"));
        }
        let out = std::process::Command::new("sh")
            .args(["-c", &format!("command -v '{c}'")])
            .output();
        let found = out.map(|o| o.status.success()).unwrap_or(false);
        if !found {
            missing.push(c.to_string());
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "缺少插件依赖的命令: {}（请先在服务器上安装）",
            missing.join("、")
        ));
    }
    Ok(())
}

/// 参与签名校验的文件：`相对路径 → 绝对路径`，按路径排序（两边算出来才一致）。
///
/// 排除隐藏文件/目录（`.git`）、以及 manifest 自身（它带着 `signature` 字段，
/// 把它算进去就成了「鸡生蛋」）。
fn signed_files(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut out = Vec::new();
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
        let rd = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))?;
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                walk(&p, base, out)?;
                continue;
            }
            if !p.is_file() {
                continue;
            }
            let rel = p
                .strip_prefix(base)
                .map_err(|_| "路径解析失败".to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "manifest.yaml" || rel == "manifest.yml" || rel == "SIGNATURE" {
                continue;
            }
            out.push((rel, p));
        }
        Ok(())
    }
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// 插件文件的规范摘要：每行 `<相对路径>:<sha256hex>`。
fn plugin_digest(root: &Path) -> Result<String, String> {
    let mut lines = Vec::new();
    for (rel, p) in signed_files(root)? {
        let bytes = std::fs::read(&p).map_err(|e| format!("读取 {rel} 失败: {e}"))?;
        let mut h = Sha256::new();
        h.update(&bytes);
        lines.push(format!("{rel}:{:x}", h.finalize()));
    }
    Ok(lines.join("\n"))
}

/// HMAC-SHA256（密钥用 zap-crypto 的主密钥，与凭据同把）。
///
/// 这是**本机完整性**校验：能挡住「插件目录被换了几行代码」，不是第三方公钥签名 ——
/// 密钥就在同一台机器上，攻击者拿到 root 就能重签。跨机器分发请配合包本身的可信来源。
fn hmac_hex(data: &str) -> Result<String, String> {
    let key = zap_crypto::SECRET_KEY
        .as_ref()
        .map_err(|e| format!("读取主密钥失败: {e}"))?;
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(key).map_err(|e| format!("初始化签名器失败: {e}"))?;
    mac.update(data.as_bytes());
    Ok(format!("{:x}", mac.finalize().into_bytes()))
}

/// 给插件目录算签名（写进 manifest 的 `signature`）。测试与 `zapctl` 复用同一套算法。
fn sign_plugin_root(root: &Path) -> Result<String, String> {
    hmac_hex(&plugin_digest(root)?)
}

/// manifest 带了 `signature` 就校验：对不上拒装。
fn verify_signature(root: &Path) -> Result<(), String> {
    let m = read_manifest(root)?;
    let Some(want) = manifest_str(&m, "signature") else {
        return Ok(()); // 没签名 = 作者没启用这项，保持向后兼容
    };
    let got = sign_plugin_root(root)?;
    if got != want.trim() {
        return Err("插件签名校验失败：文件被改动过或与签名不匹配（重新打包再安装）".to_string());
    }
    Ok(())
}

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
            return Err(format!(
                "manifest 的 scope 非法: {scope_decl}（应为 site / user / system）"
            ));
        }
        if !root.join("main.lua").is_file() {
            return Err("插件目录缺少 main.lua".into());
        }
        // 依赖声明 + 签名：都在「解包后、落地前」校验，坏插件不会进到 plugins/ 里
        check_required_commands(&m)?;
        verify_signature(&root)?;

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
        if let Some(n) = manifest_str(&m, "name")
            && n != effective
        {
            return Err(format!(
                "manifest 里的 name（{n}）与插件名（{effective}）不一致"
            ));
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
        if let Err(e) = write_install_meta(&target, &source, &src, chrono::Utc::now().timestamp()) {
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
pub async fn plugin_uninstall(_actor: String, _home: String, name: String) -> Response {
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
    for entry in ar.entries().map_err(|e| format!("解析 tar 失败: {e}"))? {
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
pub(crate) struct PluginRunArgs {
    pub(crate) name: String,
    pub(crate) actor: String,
    pub(crate) home: String,
    pub(crate) user: Option<String>,
    pub(crate) site_id: Option<i64>,
    pub(crate) site_root: Option<String>,
    pub(crate) site_linux_user: Option<String>,
    pub(crate) action: String,
    pub(crate) options: HashMap<String, String>,
    pub(crate) roles: Option<String>,
}

pub async fn plugin_run(a: PluginRunArgs) -> Response {
    let PluginRunArgs {
        name,
        actor,
        home,
        user,
        site_id: _site_id,
        site_root,
        site_linux_user,
        action,
        options,
        roles,
    } = a;
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
                return Response::err(-1, "user 作用域插件需要调用方 Linux 账号（user 字段为空）");
            }
        },
        "system" => (None, None),
        other => {
            return Response::err(
                -1,
                format!("未知 scope: {other}（应为 site / user / system）"),
            );
        }
    };
    // manifest 把某个 action 标了 `dangerous: true` 时，只读演示账号到此为止。
    // 放在后端而不是各插件 UI 里判断，插件作者漏判也拦得住。
    if action_flag(&manifest, &action, "dangerous") && is_readonly_roles(roles.as_deref()) {
        return Response::err(
            -1,
            format!("「{action}」是破坏性操作，只读演示账号不允许执行"),
        );
    }
    let lua_file = dir.join("main.lua");
    let code = match std::fs::read_to_string(&lua_file) {
        Ok(c) => c,
        Err(e) => return Response::err(-1, format!("读取插件脚本失败: {e}")),
    };

    // 异步模式：后台执行，日志实时落盘，立即返回 task_id + log_path，由前端 SSE 订阅。
    // 既支持插件级 `async: true`，也支持 action 级 `actions.<name>.async: true` ——
    // 读写混合的插件（像 Git）可以只把 push / clone 这类长操作异步化。
    let is_async = manifest
        .get("async")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || action_flag(&manifest, &action, "async");
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
        std::thread::spawn(move || {
            loop {
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
            }
        });
        let ctx = RunCtx {
            scope: scope.clone(),
            run_user: run_user.clone(),
            run_root: run_root.clone(),
            home: home.clone(),
            plugin_dir: dir.clone(),
            options: options.clone(),
            action: action.clone(),
            plugin_name: name.clone(),
            actor: actor.clone(),
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
        plugin_name: name,
        actor,
    };
    // 同步运行套一层墙钟超时：超时则置取消标志，看门狗杀掉子进程（组），请求不再被永久占住。
    let cancel = Arc::new(AtomicBool::new(false));
    let child_pid = Arc::new(std::sync::Mutex::new(None::<(u32, bool)>));
    let cancel_w = cancel.clone();
    let child_pid_w = child_pid.clone();
    std::thread::spawn(move || {
        loop {
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
        }
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
    /// 插件名（持久配置 KV 的文件名取自它）
    plugin_name: String,
    /// 调用方面板账号（持久配置按用户分桶）
    actor: String,
}

// ── 冒烟测试 ──────────────────────────────────────────────
//
// 插件可以在自己目录里放一份 `tests.yaml`：
//
// ```yaml
// - action: status
//   options: { cwd: /tmp/demo }
//   user: www                # 可选：scope=site/user 时降权到该 Linux 账号
//   site_root: /tmp/demo     # 可选：scope=site 时当作站点根
//   expect:
//     contains: [branch]     # 日志里必须出现（可给多个，任一命中即算过？不，全部都要有）
//     not_contains: [fatal]
// ```
//
// 逐条跑 `on_<action>`（回落到 `on_run`），再把日志拿去比对断言。作者能在不装面板 UI
// 的情况下验证「每个 action 都没坏」，CI 也能直接调这一个动词。

#[derive(serde::Deserialize, Default)]
struct TestCase {
    #[serde(default)]
    action: String,
    #[serde(default)]
    options: HashMap<String, String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    site_root: Option<String>,
    #[serde(default)]
    expect: Option<TestExpect>,
}

#[derive(serde::Deserialize, Default)]
struct TestExpect {
    /// 日志里必须包含的片段（全部命中才算通过）
    #[serde(default)]
    contains: Vec<String>,
    /// 日志里不允许出现的片段
    #[serde(default)]
    not_contains: Vec<String>,
}

/// 跑插件自带的 `tests.yaml`，返回每条用例的结果（供作者自查 / CI 冒烟）。
pub async fn plugin_test(_actor: String, home: String, name: String) -> Response {
    if !is_plugin_name(&name) {
        return Response::err(-1, "非法插件名".to_string());
    }
    let dir = plugin_base().join(&name);
    if !dir.is_dir() {
        return Response::err(-1, format!("插件不存在: {name}"));
    }
    let manifest = match read_manifest(&dir) {
        Ok(m) => m,
        Err(e) => return Response::err(-1, e),
    };
    let file = dir.join("tests.yaml");
    if !file.is_file() {
        return Response::err(-1, "该插件没有 tests.yaml，无需测试".to_string());
    }
    let cases: Vec<TestCase> = match std::fs::read_to_string(&file)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_yaml::from_str(&t).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => return Response::err(-1, format!("tests.yaml 解析失败: {e}")),
    };
    let scope_str = manifest_str(&manifest, "scope")
        .unwrap_or("system")
        .to_string();
    let code = match std::fs::read_to_string(dir.join("main.lua")) {
        Ok(c) => c,
        Err(e) => return Response::err(-1, format!("读取插件脚本失败: {e}")),
    };

    let mut results = Vec::new();
    for (i, case) in cases.into_iter().enumerate() {
        let action = if case.action.is_empty() {
            "run".to_string()
        } else {
            case.action.clone()
        };
        let label = format!("#{} {}", i + 1, action);
        // scope=site / user 必须有降权账号，跑不了就标 skipped 而不是误判失败
        let (run_user, run_root) = match scope_str.as_str() {
            // 与 plugin_run 一致：测试也不能拿 root 直接跑（root 会绕过所有降权）
            "site" => match (&case.site_root, &case.user) {
                (Some(r), Some(u)) if u == "root" || u.is_empty() => {
                    results.push(json!({
                        "name": label, "ok": null, "skipped": true,
                        "detail": "site 作用域的降权账号不能是 root，已跳过",
                    }));
                    continue;
                }
                (Some(r), Some(u)) => (Some(u.clone()), Some(r.clone())),
                _ => {
                    results.push(json!({
                        "name": label, "ok": null, "skipped": true,
                        "detail": "scope=site 的用例需要同时给出 site_root 与 user，已跳过",
                    }));
                    continue;
                }
            },
            "user" => match &case.user {
                Some(u) if u == "root" || u.is_empty() => {
                    results.push(json!({
                        "name": label, "ok": null, "skipped": true,
                        "detail": "user 作用域的降权账号不能是 root，已跳过",
                    }));
                    continue;
                }
                Some(u) => (Some(u.clone()), None),
                None => {
                    results.push(json!({
                        "name": label, "ok": null, "skipped": true,
                        "detail": "scope=user 的用例需要给出 user（降权账号），已跳过",
                    }));
                    continue;
                }
            },
            _ => (None, None),
        };
        let ctx = RunCtx {
            scope: scope_str.clone(),
            run_user,
            run_root,
            home: home.clone(),
            plugin_dir: dir.clone(),
            options: case.options.clone(),
            action: action.clone(),
            plugin_name: name.clone(),
            actor: String::new(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let child_pid = Arc::new(std::sync::Mutex::new(None::<(u32, bool)>));
        let code_each = code.clone();
        let res =
            tokio::task::spawn_blocking(move || run_lua(&code_each, &ctx, None, cancel, child_pid))
                .await
                .unwrap_or_else(|e| Err(format!("测试线程崩溃: {e}")));
        let expect = case.expect.unwrap_or_default();
        match res {
            Err(e) => results.push(json!({
                "name": label, "ok": false, "skipped": false, "detail": e, "log": "",
            })),
            Ok(log) => {
                let mut failures: Vec<String> = Vec::new();
                for needle in &expect.contains {
                    if !log.contains(needle) {
                        failures.push(format!("缺少输出片段「{needle}」"));
                    }
                }
                for needle in &expect.not_contains {
                    if log.contains(needle) {
                        failures.push(format!("出现不该有的输出「{needle}」"));
                    }
                }
                results.push(json!({
                    "name": label,
                    "ok": failures.is_empty(),
                    "skipped": false,
                    "detail": failures.join("；"),
                    "log": log,
                }));
            }
        }
    }
    let passed = results
        .iter()
        .filter(|r| r.get("ok").and_then(|v| v.as_bool()) == Some(true))
        .count();
    let failed = results
        .iter()
        .filter(|r| r.get("ok").and_then(|v| v.as_bool()) == Some(false))
        .count();
    Response::ok(
        format!("冒烟测试完成：通过 {passed}，失败 {failed}"),
        Some(json!({ "results": results, "passed": passed, "failed": failed })),
    )
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
    vec![zap_path().join("data/plugins/_lib"), plugin_dir.join("lib")]
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
            run_capture(RunCaptureOpts {
                user: None,
                program: &prog,
                args: &table_to_vec(&args),
                stdin: None,
                logf: logf_exec.clone(),
                cancel: cancel_exec.clone(),
                child_pid: child_pid_exec.clone(),
                cwd,
            })
            .map_err(mlua::Error::RuntimeError)
        });
        zap_tbl
            .set("exec", f_exec.map_err(|e| format!("exec 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;

        let scope_run = ctx.scope.clone();
        let run_user_run = ctx.run_user.clone();
        let f_user = lua.create_function(
            move |_, (prog, args, opts): (String, mlua::Table, mlua::Value)| match &run_user {
                Some(u) => {
                    let cwd = extract_cwd(&opts).map_err(mlua::Error::RuntimeError)?;
                    run_capture(RunCaptureOpts {
                        user: Some(u),
                        program: &prog,
                        args: &table_to_vec(&args),
                        stdin: None,
                        logf: logf_user.clone(),
                        cancel: cancel_user.clone(),
                        child_pid: child_pid_user.clone(),
                        cwd,
                    })
                    .map_err(mlua::Error::RuntimeError)
                }
                None => Err(mlua::Error::RuntimeError(
                    "site 作用域插件未提供运行账号".into(),
                )),
            },
        );
        zap_tbl
            .set(
                "exec_as_user",
                f_user.map_err(|e| format!("exec_as_user 注册失败: {e}"))?,
            )
            .map_err(|e| format!("{e}"))?;

        // zap.run：按 scope 自动选 root / 站点账号，插件不必自己判断作用域
        let f_run = lua.create_function(
            move |_, (prog, args, opts): (String, mlua::Table, mlua::Value)| {
                let user = match scope_run.as_str() {
                    "site" | "user" => run_user_run.clone(),
                    _ => None,
                };
                if (scope_run == "site" || scope_run == "user") && user.is_none() {
                    return Err(mlua::Error::RuntimeError(
                        "该作用域插件未提供运行账号".into(),
                    ));
                }
                let cwd = extract_cwd(&opts).map_err(mlua::Error::RuntimeError)?;
                run_capture(RunCaptureOpts {
                    user: user.as_deref(),
                    program: &prog,
                    args: &table_to_vec(&args),
                    stdin: None,
                    logf: logf_run.clone(),
                    cancel: cancel_run.clone(),
                    child_pid: child_pid_run.clone(),
                    cwd,
                })
                .map_err(mlua::Error::RuntimeError)
            },
        );
        zap_tbl
            .set("run", f_run.map_err(|e| format!("run 注册失败: {e}"))?)
            .map_err(|e| format!("{e}"))?;

        // zap.try_run：同上，但失败不抛错，返回 (ok, output)
        let scope_try = ctx.scope.clone();
        let run_user_try = ctx.run_user.clone();
        let logf_try = logf.clone();
        let cancel_try = cancel.clone();
        let child_pid_try = child_pid.clone();
        let f_try = lua.create_function(
            move |_, (prog, args, opts): (String, mlua::Table, mlua::Value)| {
                let user = match scope_try.as_str() {
                    "site" | "user" => run_user_try.clone(),
                    _ => None,
                };
                let argv = table_to_vec(&args);
                let cwd = extract_cwd(&opts).map_err(mlua::Error::RuntimeError)?;
                match run_capture(RunCaptureOpts {
                    user: user.as_deref(),
                    program: &prog,
                    args: &argv,
                    stdin: None,
                    logf: logf_try.clone(),
                    cancel: cancel_try.clone(),
                    child_pid: child_pid_try.clone(),
                    cwd,
                }) {
                    Ok(s) => Ok((true, s)),
                    Err(s) => Ok((false, s)),
                }
            },
        );
        zap_tbl
            .set(
                "try_run",
                f_try.map_err(|e| format!("try_run 注册失败: {e}"))?,
            )
            .map_err(|e| format!("{e}"))?;
    }
    // zap.read_file / zap.write_file / zap.append_file（按 scope 降权的子进程实现）
    {
        let scope = ctx.scope.clone();
        let run_user = ctx.run_user.clone();
        for (name, redirect) in [
            ("read_file", "<"),
            ("write_file", ">"),
            ("append_file", ">>"),
        ] {
            let scope = scope.clone();
            let run_user = run_user.clone();
            let f = lua.create_function(move |_, (path, content): (String, Option<String>)| {
                let script = format!("cat {} \"$1\"", redirect);
                let stdin = if redirect == "<" {
                    None
                } else {
                    Some(content.unwrap_or_default())
                };
                let user = match scope.as_str() {
                    "site" | "user" => run_user.clone(),
                    _ => None,
                };
                if (scope == "site" || scope == "user") && user.is_none() {
                    return Err(mlua::Error::RuntimeError(
                        "该作用域插件未提供运行账号".into(),
                    ));
                }
                if path.trim().is_empty() {
                    return Err(mlua::Error::RuntimeError("路径不能为空".into()));
                }
                run_capture(RunCaptureOpts {
                    user: user.as_deref(),
                    program: "sh",
                    args: &["-c".to_string(), script, "sh".to_string(), path],
                    stdin,
                    logf: None,
                    cancel: Arc::new(AtomicBool::new(false)),
                    child_pid: Arc::new(std::sync::Mutex::new(None)),
                    cwd: None,
                })
                .map_err(mlua::Error::RuntimeError)
            });
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
            .set(
                "home_dir",
                f_home.map_err(|e| format!("home_dir 注册失败: {e}"))?,
            )
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
    // zap.config_get / zap.config_set：插件级持久 KV（按插件 + 面板用户分桶）。
    //
    // 插件目录是 root 所有、且 Lua 侧没有 io/os，作者想存偏好 / token 只能靠这里。
    // 每次读写直接落盘（配置都很小），返回 nil 表示「还没存过」。
    {
        let cfg_path = plugin_config_file(&ctx.plugin_name).ok();
        let cfg_path_set = cfg_path.clone();
        let actor_get = ctx.actor.clone();
        let actor_set = ctx.actor.clone();
        let f_get = lua.create_function(move |lua, key: String| {
            let Some(path) = &cfg_path else {
                return Ok(mlua::Value::Nil);
            };
            let store = read_config_store(path);
            let v = store
                .get(serde_yaml::Value::String(actor_get.clone()))
                .and_then(|m| m.as_mapping())
                .and_then(|m| m.get(serde_yaml::Value::String(key)))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            match v {
                Some(s) => Ok(mlua::Value::String(lua.create_string(s)?)),
                None => Ok(mlua::Value::Nil),
            }
        });
        zap_tbl
            .set("config_get", f_get.map_err(|e| format!("{e}"))?)
            .map_err(|e| format!("{e}"))?;
        let f_set = lua.create_function(move |_, (key, value): (String, Option<String>)| {
            // value 为 nil / 空串 = 删除该键
            let Some(path) = &cfg_path_set else {
                return Err(mlua::Error::RuntimeError("插件名非法，无法写配置".into()));
            };
            if key.trim().is_empty() || key.len() > 128 {
                return Err(mlua::Error::RuntimeError("配置键非法".into()));
            }
            let mut store = read_config_store(path);
            let user_key = serde_yaml::Value::String(actor_set.clone());
            let mut entry = match store.get(&user_key) {
                Some(serde_yaml::Value::Mapping(m)) => m.clone(),
                _ => serde_yaml::Mapping::new(),
            };
            let k = serde_yaml::Value::String(key);
            match value.filter(|v| !v.is_empty()) {
                Some(v) => {
                    entry.insert(k, serde_yaml::Value::String(v));
                }
                None => {
                    entry.remove(&k);
                }
            }
            store.insert(user_key, serde_yaml::Value::Mapping(entry));
            write_config_store(path, &store).map_err(mlua::Error::RuntimeError)?;
            Ok(true)
        });
        zap_tbl
            .set("config_set", f_set.map_err(|e| format!("{e}"))?)
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
                || matches!(
                    key.as_str(),
                    "PATH" | "HOME" | "USER" | "SHELL" | "LANG" | "TMPDIR"
                );
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
            serde_json::to_string(&j).map_err(|e| mlua::Error::RuntimeError(e.to_string()))
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
    ctx_tbl
        .set("action", ctx.action.clone())
        .map_err(|e| format!("{e}"))?;
    ctx_tbl
        .set("scope", ctx.scope.clone())
        .map_err(|e| format!("{e}"))?;
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
        let code =
            std::fs::read_to_string(&p).map_err(|e| format!("读取 {} 失败: {e}", p.display()))?;
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
            String::from_utf8_lossy(s.as_bytes()).to_string(),
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
    for (_, s) in t.clone().pairs::<i64, String>().flatten() {
        v.push(s);
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
        Ok(mlua::Value::String(s)) => String::from_utf8_lossy(s.as_bytes()).to_string(),
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
struct RunCaptureOpts<'a> {
    user: Option<&'a str>,
    program: &'a str,
    args: &'a [String],
    stdin: Option<String>,
    logf: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    child_pid: std::sync::Arc<std::sync::Mutex<Option<(u32, bool)>>>,
    cwd: Option<PathBuf>,
}

fn run_capture(o: RunCaptureOpts) -> Result<String, String> {
    let RunCaptureOpts {
        user,
        program,
        args,
        stdin,
        logf,
        cancel,
        child_pid,
        cwd,
    } = o;
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
    let copy = |r: Option<Box<dyn std::io::Read + Send>>,
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
    let stdout: Option<Box<dyn std::io::Read + Send>> = child
        .stdout
        .take()
        .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>);
    let stderr: Option<Box<dyn std::io::Read + Send>> = child
        .stderr
        .take()
        .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>);
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

    /// `ui.placement` 既可以是单个槽位（旧写法），也可以是槽位数组（一个插件挂多处）。
    #[test]
    fn placement_accepts_single_and_list() {
        let single: serde_yaml::Value =
            serde_yaml::from_str("ui:\n  placement: file.editor\n").unwrap();
        assert_eq!(placements_of(&single), vec!["file.editor".to_string()]);

        let list: serde_yaml::Value = serde_yaml::from_str(
            "ui:\n  placement:\n    - file.editor\n    - file.context\n    - file.editor\n",
        )
        .unwrap();
        // 重复项要去重，否则多槽位插件会在列表里出现两次
        assert_eq!(
            placements_of(&list),
            vec!["file.editor".to_string(), "file.context".to_string()]
        );

        let missing: serde_yaml::Value = serde_yaml::from_str("name: x\n").unwrap();
        assert!(placements_of(&missing).is_empty());
    }

    /// action 既可以是「名字: 文案」，也可以是「名字: {label, async, dangerous}」。
    #[test]
    fn action_spec_parses_both_shapes() {
        let m: serde_yaml::Value = serde_yaml::from_str(
            "actions:\n  push: 推送\n  reset:\n    label: 回退\n    dangerous: true\n  clone:\n    label: 克隆\n    async: true\n",
        )
        .unwrap();
        assert_eq!(action_label(&m, "push").as_deref(), Some("推送"));
        assert_eq!(action_label(&m, "reset").as_deref(), Some("回退"));

        assert!(!action_flag(&m, "push", "dangerous"));
        assert!(action_flag(&m, "reset", "dangerous"));
        assert!(action_flag(&m, "clone", "async"));
        assert!(!action_flag(&m, "reset", "async"));

        let specs = action_specs(&m);
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0]["name"], serde_json::json!("push"));
        assert_eq!(specs[1]["name"], serde_json::json!("reset"));
        assert_eq!(specs[1]["dangerous"], serde_json::json!(true));
        assert_eq!(specs[2]["async"], serde_json::json!(true));
    }

    /// demo（只读演示账号）不能跑标了 dangerous 的动作；没带 roles 的调用方不拦。
    #[test]
    fn readonly_roles_are_detected() {
        assert!(is_readonly_roles(Some("demo")));
        assert!(is_readonly_roles(Some("user,demo")));
        assert!(!is_readonly_roles(Some("user")));
        assert!(!is_readonly_roles(None));
        assert!(!is_readonly_roles(Some("")));
    }

    /// UIKit 注入：样式进 head、脚本进 body，缺 einmal 文件时原样返回。
    #[test]
    fn uikit_is_injected_into_plugin_html() {
        let dir = std::env::temp_dir().join(format!("zap-uikit-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("ui.css"), ":root{--x:1}\n").unwrap();
        std::fs::write(dir.join("ui.js"), "window.zap={}\n").unwrap();

        let html = "<html><head><title>t</title></head><body><p>hi</p></body></html>";
        let out = inject_uikit(html, &dir, None, None);
        assert!(
            out.contains("<style>") && out.contains("--x:1"),
            "样式没注入: {out}"
        );
        assert!(
            out.contains("<script>") && out.contains("window.zap"),
            "脚本没注入: {out}"
        );
        // 位置：style 在 </head> 前，script 在 </body> 前
        assert!(out.find("</style>").unwrap() < out.to_lowercase().find("</head>").unwrap());
        assert!(out.find("</script>").unwrap() < out.to_lowercase().find("</body>").unwrap());

        // 缺 UIKit 时不要破坏原文件（未部署 _lib 的老环境）
        let empty = std::env::temp_dir().join(format!("zap-uikit-empty-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&empty);
        assert_eq!(inject_uikit(html, &empty, None, None), html);

        // 没有 head / body 的片段也要能注入，不能静默丢掉脚本
        let frag = "<div>x</div>";
        let out2 = inject_uikit(frag, &dir, None, None);
        assert!(
            out2.contains("--x:1") && out2.contains("window.zap"),
            "{out2}"
        );

        // 面板语言要先于 UIKit 注入：插件 UI 才能跟着 Element Plus 切中英
        let out3 = inject_uikit(html, &dir, Some("en-US"), None);
        assert!(out3.contains("__ZAP_LANG__=\"en-US\""), "{out3}");
        assert!(out3.find("__ZAP_LANG__").unwrap() < out3.find("--x:1").unwrap());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }

    /// manifest 的 `i18n` 表要整份注入成 `window.__ZAP_I18N__`——插件 HTML 里的
    /// `T()` / `applyI18n` 全靠它查译文；没注入的话界面永远只显示基准中文。
    #[test]
    fn manifest_i18n_is_injected_into_plugin_html() {
        let dir = std::env::temp_dir().join(format!("zap-uikit-i18n-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("ui.css"), ":root{--x:1}\n").unwrap();
        std::fs::write(dir.join("ui.js"), "window.zap={}\n").unwrap();

        let i18n: serde_yaml::Value =
            serde_yaml::from_str("en-US:\n  title: Git manager\n  ui:\n    \"状态\": \"Status\"\n")
                .unwrap();
        let html = "<html><head></head><body><p>hi</p></body></html>";
        let out = inject_uikit(html, &dir, Some("en-US"), Some(&i18n));
        assert!(out.contains("window.__ZAP_I18N__="), "i18n 表没注入: {out}");
        assert!(out.contains("Status"), "译文没进注入内容: {out}");
        assert!(out.contains("__ZAP_LANG__=\"en-US\""), "语言没注入: {out}");
        // 必须在插件自己的脚本之前，否则 T() 读到空表
        assert!(out.find("__ZAP_I18N__").unwrap() < out.find("--x:1").unwrap());

        // 译文里的 `</script>` 不能提前闭合脚本标签
        let evil: serde_yaml::Value =
            serde_yaml::from_str("en-US:\n  ui:\n    \"x\": \"</script><img>\"\n").unwrap();
        let out2 = inject_uikit(html, &dir, None, Some(&evil));
        assert!(!out2.contains("</script><img>"), "脚本被提前闭合: {out2}");

        let _ = std::fs::remove_dir_all(&dir);
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
            plugin_name: "zap-plugin-test".to_string(),
            actor: "tester".to_string(),
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
        assert!(
            out.contains("within=true"),
            "zap.path.within 行为不对: {out}"
        );
        assert!(
            out.contains("site=/tmp/sub"),
            "zap.path.site 行为不对: {out}"
        );
    }

    /// manifest 的 `i18n` 表按面板语言覆盖文案；只有部分翻译时其余键回落基准值。
    #[test]
    fn i18n_overlays_describe_fields() {
        let dir = std::env::temp_dir().join(format!("zap-i18n-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = "\
name: demo
title: 演示插件
description: 这是演示
scope: system
ui:
  placement: site.detail
  label: 打开演示
options:
  - name: path
    label: 路径
    desc: 目标路径
actions:
  run: 运行
  reset:
    label: 回退
i18n:
  en-US:
    title: Demo plugin
    description: Just a demo
    label: Open demo
    actions:
      run: Run
      reset: Revert
    options:
      - name: path
        label: Path
";
        std::fs::write(dir.join("manifest.yaml"), manifest).unwrap();

        let zh = describe_lang(&dir, "demo", Some("zh-CN")).unwrap();
        assert_eq!(zh["title"], serde_json::json!("演示插件"));
        assert_eq!(zh["label"], serde_json::json!("打开演示"));

        let en = describe_lang(&dir, "demo", Some("en-US")).unwrap();
        assert_eq!(en["title"], serde_json::json!("Demo plugin"));
        assert_eq!(en["description"], serde_json::json!("Just a demo"));
        assert_eq!(en["label"], serde_json::json!("Open demo"));
        assert_eq!(en["actions"]["run"], serde_json::json!("Run"));
        assert_eq!(en["options"][0]["label"], serde_json::json!("Path"));
        // action 元信息里的 label 也要跟着换
        let specs = en["action_specs"].as_array().unwrap();
        let reset = specs
            .iter()
            .find(|s| s["name"] == serde_json::json!("reset"))
            .unwrap();
        assert_eq!(reset["label"], serde_json::json!("Revert"));

        // 主语言兜底：`en-GB` 也能命中 en-US
        assert_eq!(
            describe_lang(&dir, "demo", Some("en-GB")).unwrap()["title"],
            serde_json::json!("Demo plugin")
        );
        // 没给语言 = 基准文案
        assert_eq!(
            describe_lang(&dir, "demo", None).unwrap()["title"],
            serde_json::json!("演示插件")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 翻译是**可选的**：没提供 `i18n`（或没提供某个语言）时，界面直接显示基准文案。
    ///
    /// 这是给插件作者省事的约定 —— 只写一种语言的插件不该因为「没翻」而在
    /// 英文界面上变成空白或报错。
    #[test]
    fn missing_translation_falls_back_to_base() {
        let dir = std::env::temp_dir().join(format!("zap-i18n-fb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 1) 完全没有 i18n 表
        let plain = "\
name: demo
title: 演示插件
description: 这是演示
scope: system
ui:
  placement: site.detail
  label: 打开演示
options:
  - name: path
    label: 路径
actions:
  run: 运行
";
        std::fs::write(dir.join("manifest.yaml"), plain).unwrap();
        for lang in [Some("zh-CN"), Some("en-US"), Some("ja-JP"), None] {
            let info = describe_lang(&dir, "demo", lang).unwrap();
            assert_eq!(
                info["title"],
                serde_json::json!("演示插件"),
                "lang={lang:?} 应回落到基准文案"
            );
            assert_eq!(info["label"], serde_json::json!("打开演示"));
            assert_eq!(info["options"][0]["label"], serde_json::json!("路径"));
            assert_eq!(info["actions"]["run"], serde_json::json!("运行"));
        }

        // 2) 只有 zh-CN 一张表：请求英文同样回落到基准文案，而不是渲染失败
        let half = format!(
            "{plain}i18n:\n  zh-CN:\n    title: 演示插件（中文）\n    actions:\n      run: 执行\n"
        );
        std::fs::write(dir.join("manifest.yaml"), half).unwrap();
        let en = describe_lang(&dir, "demo", Some("en-US")).unwrap();
        assert_eq!(en["title"], serde_json::json!("演示插件"));
        assert_eq!(en["actions"]["run"], serde_json::json!("运行"));
        let zh = describe_lang(&dir, "demo", Some("zh-CN")).unwrap();
        assert_eq!(zh["title"], serde_json::json!("演示插件（中文）"));
        assert_eq!(zh["actions"]["run"], serde_json::json!("执行"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 插件级 KV：按「插件 + 用户」分桶，set 做合并、get 只看自己的。
    #[test]
    fn plugin_config_kv_roundtrip() {
        let root = std::env::temp_dir().join(format!("zap-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("demo.yaml");

        let mut store = read_config_store(&path);
        assert!(store.is_empty(), "配置文件不存在时应该为空");

        let mut alice = serde_yaml::Mapping::new();
        alice.insert(
            serde_yaml::Value::String("branch".into()),
            serde_yaml::Value::String("main".into()),
        );
        store.insert(serde_yaml::Value::String("alice".into()), alice.into());
        write_config_store(&path, &store).unwrap();

        // 改 alice 的 branch、再加一个键：合并语义（旧键保留）
        let mut store = read_config_store(&path);
        let mut entry = match store.get(serde_yaml::Value::String("alice".into())) {
            Some(serde_yaml::Value::Mapping(m)) => m.clone(),
            _ => serde_yaml::Mapping::new(),
        };
        entry.insert(
            serde_yaml::Value::String("repo".into()),
            serde_yaml::Value::String("https://x".into()),
        );
        store.insert(serde_yaml::Value::String("alice".into()), entry.into());
        write_config_store(&path, &store).unwrap();

        let store = read_config_store(&path);
        let alice = store
            .get(serde_yaml::Value::String("alice".into()))
            .and_then(|v| v.as_mapping())
            .unwrap();
        assert_eq!(alice.len(), 2, "合并后应保留两个键");
        assert!(store.get(serde_yaml::Value::String("bob".into())).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 依赖声明：manifest 声明的命令缺失时拒装。
    #[test]
    fn requires_commands_are_validated() {
        let m: serde_yaml::Value = serde_yaml::from_str("requires:\n  commands: [git]\n").unwrap();
        // git 在 CI / 开发机上基本都有；没有就用 shell 的 builtin 兜一个必定存在的
        let ok: serde_yaml::Value = serde_yaml::from_str("requires:\n  commands: [sh]\n").unwrap();
        assert!(check_required_commands(&ok).is_ok());
        let missing: serde_yaml::Value =
            serde_yaml::from_str("requires:\n  commands: [definitely-not-a-command-zz]\n").unwrap();
        let err = check_required_commands(&missing).unwrap_err();
        assert!(err.contains("definitely-not-a-command-zz"), "{err}");
        // 没声明 requires 不受影响
        let none: serde_yaml::Value = serde_yaml::from_str("name: x\n").unwrap();
        assert!(check_required_commands(&none).is_ok());
        let _ = m;
    }

    /// 签名：同一目录算出稳定摘要；改了文件就对不上。
    #[test]
    fn plugin_signature_detects_tampering() {
        let dir = std::env::temp_dir().join(format!("zap-sig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("main.lua"), "-- demo\n").unwrap();
        std::fs::write(dir.join("lib/a.lua"), "-- a\n").unwrap();
        std::fs::write(
            dir.join("manifest.yaml"),
            "name: demo\nui:\n  placement: site.detail\n",
        )
        .unwrap();

        let sig = sign_plugin_root(&dir).unwrap();
        assert_eq!(sig, sign_plugin_root(&dir).unwrap(), "摘要必须稳定");
        // 摘要不含 manifest 自身：作者可以先签名、再把 signature 写进 manifest
        std::fs::write(
            dir.join("manifest.yaml"),
            format!("name: demo\nui:\n  placement: site.detail\nsignature: {sig}\n"),
        )
        .unwrap();
        assert!(verify_signature(&dir).is_ok(), "签名一致应通过");

        std::fs::write(dir.join("lib/a.lua"), "-- tampered\n").unwrap();
        assert!(verify_signature(&dir).is_err(), "文件被改后必须拒装");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 随发行包走的示例 / 商店插件 manifest 必须是合法 YAML。
    ///
    /// 这类文件平时没人编译它，缩进错一个空格就要等到用户安装时才炸
    /// （而且报错是「manifest 解析失败」，很难看出是 YAML 问题）。
    #[test]
    fn shipped_manifests_are_valid() {
        let git = include_str!("../../../data/appstore/repos/appstore/plugins/git/manifest.yaml");
        let m: serde_yaml::Value = serde_yaml::from_str(git).expect("git manifest 解析失败");
        assert_eq!(m["name"].as_str(), Some("git"));
        // 依赖声明：缺命令时安装会被拒，拼写错了也一样
        let requires = m["requires"]["commands"].as_sequence().unwrap();
        assert!(requires.iter().any(|c| c.as_str() == Some("git")));

        // i18n 是**可选项**：插件可以完全不提供翻译（那就一直显示基准文案）。
        // 但如果提供了某张表，里面翻译出来的动作名必须都在基准 actions 里 ——
        // 写错名字的孤儿键在面板上永远看不到，纯浪费。
        let base = m["actions"].as_mapping().unwrap();
        for lang in ["zh-CN", "en-US"] {
            let Some(table) = i18n_table(&m, Some(lang)) else {
                continue;
            };
            if let Some(actions) = table
                .get(serde_yaml::Value::String("actions".to_string()))
                .and_then(|v| v.as_mapping())
            {
                for (k, _) in actions {
                    let name = k.as_str().expect("action 名必须是字符串");
                    assert!(
                        base.contains_key(k),
                        "i18n.{lang} 里的 action『{name}』不在基准 actions 中"
                    );
                }
            }
        }

        let demo = include_str!("../../../data/plugins/examples/widgets-demo/manifest.yaml");
        let d: serde_yaml::Value = serde_yaml::from_str(demo).expect("示例插件 manifest 解析失败");
        assert_eq!(d["name"].as_str(), Some("widgets-demo"));
    }

    #[test]
    fn path_helpers() {
        // 插件统一装在系统目录 $ZAP_PATH/plugins；用户级已移除
        let sys = plugin_base();
        assert!(sys.ends_with("plugins"));
    }
}
