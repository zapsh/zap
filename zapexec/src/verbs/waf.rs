//! ModSecurity（WAF）—— 可选能力，没装就不能设置
//!
//! 与面板里其它服务不同，WAF **不是一个装上就能用的开关**：ModSecurity v3 由
//! 两部分组成 —— libmodsecurity（规则引擎库）+ ModSecurity-nginx（nginx 连接器），
//! 连接器必须以**动态模块**形式加载，而动态模块要求 nginx 当初以 `--with-compat`
//! 编译（否则模块签名不匹配，`load_module` 会直接拒绝加载）。
//!
//! 因此这里的设计取舍是：
//!
//! - **探测优先**：`status` 如实回答「装没装 / 缺什么 / 能不能自动装」，
//!   未安装时其余动词一律返回"未安装"，不给"看着能设、点了报错"的假界面。
//! - **不替用户重编 nginx**：重编并替换正在承载业务的 nginx 属于高风险操作
//!   （一个参数不对，全站 502）。检测到 nginx 不带 `--with-compat` 时，
//!   面板只给原因与建议，不自动执行。
//! - **装完不立刻拦截**：规则引擎默认 `DetectionOnly`（只记录不拦截），
//!   用户看过审计日志、确认无业务误杀后再自行切到 On。
//!
//! 规则目录 `/etc/zap/nginx/modsecurity`（与四层转发的 `zap-stream.conf` 同级）：
//! 主配置 `modsecurity.conf`、OWASP CRS（`crs-setup.conf` + `rules/*.conf`）
//! 与用户自定义规则都放这里 —— 挂在面板自有的 nginx 目录下，重装 / 升级
//! nginx 都不会把规则带走。

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use zap_proto::Response;

use super::root_cmd;
use super::service_conf::run_blocking;
use super::site::{find_nginx_conf_file, nginx_bin, nginx_running, nginx_test};

/// 未安装时的统一答复（前端据此只显示安装引导，不给配置项）。
const NOT_INSTALLED: &str = "未检测到 ModSecurity（WAF）：该可选组件尚未安装";
/// libmodsecurity 的常见安装位置（源码装 / 发行版包）
const LIB_CANDIDATES: &[&str] = &[
    "/usr/local/modsecurity/lib/libmodsecurity.so.3",
    "/usr/local/lib/libmodsecurity.so.3",
    "/usr/lib/x86_64-linux-gnu/libmodsecurity.so.3",
    "/usr/lib64/libmodsecurity.so.3",
];
/// 模块 .so 的文件名（nginx 动态模块）
const MODULE_SO: &str = "ngx_http_modsecurity_module.so";
/// 规则目录名：挂在面板自有的 nginx 目录（`/etc/zap/nginx`）下，
/// 与四层转发的 `zap-stream.conf` 同级。
const RULES_DIR_NAME: &str = "modsecurity";

// ── 探测 ────────────────────────────────────────────────────

/// nginx 的可执行文件、主配置与编译参数。
struct NginxInfo {
    bin: PathBuf,
    conf: PathBuf,
    version: String,
    /// `nginx -V` 里的 configure arguments 原文
    args: String,
}

/// 跑一次命令取 stdout（失败给空串）。
fn out_str(program: &str, args: &[&str]) -> String {
    match root_cmd(program).args(args).output() {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                String::from_utf8_lossy(&o.stderr).trim().to_string()
            } else {
                s
            }
        }
        Err(_) => String::new(),
    }
}

/// `nginx -V`：版本与 configure 参数（nginx 把 -V 输出到 stderr）。
fn nginx_v(bin: &Path) -> (String, String) {
    let o = match root_cmd(&bin.to_string_lossy()).arg("-V").output() {
        Ok(o) => o,
        Err(_) => return (String::new(), String::new()),
    };
    let text = String::from_utf8_lossy(&o.stderr).to_string();
    let text = if text.trim().is_empty() {
        String::from_utf8_lossy(&o.stdout).to_string()
    } else {
        text
    };
    // `nginx -V` 首行形如 "nginx version: nginx/1.31.5"，剥掉前缀与 `-v` 输出保持一致
    let version = text
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches("nginx version:")
        .trim()
        .to_string();
    let args = text
        .split_once("configure arguments:")
        .map(|(_, rest)| rest.trim().to_string())
        .unwrap_or_default();
    (version, args)
}

/// 定位 nginx；拿不到主配置就视为未安装 nginx（WAF 更无从谈起）。
fn nginx_info() -> Option<NginxInfo> {
    let conf = find_nginx_conf_file()?;
    let bin = nginx_bin(&conf);
    if !bin.is_file() && which_abs(&bin).is_none() {
        return None;
    }
    let (version, args) = nginx_v(&bin);
    Some(NginxInfo {
        bin,
        conf,
        version,
        args,
    })
}

/// 绝对路径直接用；否则按 PATH 找（root_cmd 用的是安全 PATH）。
fn which_abs(bin: &Path) -> Option<PathBuf> {
    if bin.is_absolute() {
        return None;
    }
    let name = bin.to_string_lossy().to_string();
    let p = out_str(super::platform::SHELL, &["-c", &format!("command -v {name}")]);
    (!p.is_empty()).then(|| PathBuf::from(p))
}

fn has_cmd(name: &str) -> bool {
    !out_str(super::platform::SHELL, &["-c", &format!("command -v {name}")]).is_empty()
}

/// 模块目录：`--modules-path=` 优先，否则 `<prefix>/modules`。
fn modules_dir(args: &str) -> Option<PathBuf> {
    if let Some(v) = args.split_whitespace().find_map(|a| a.strip_prefix("--modules-path=")) {
        return Some(PathBuf::from(v));
    }
    args.split_whitespace()
        .find_map(|a| a.strip_prefix("--prefix="))
        .map(|p| PathBuf::from(p).join("modules"))
}

/// 主配置里是否已经 `load_module` 了我们的模块（只看未注释的行）。
fn conf_loads_module(conf: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(conf) else {
        return false;
    };
    content
        .lines()
        .any(|l| l.trim_start().starts_with("load_module") && l.contains(MODULE_SO))
}

/// libmodsecurity 是否在库路径里（ldconfig 或已知候选路径）。
fn libmodsecurity_present() -> bool {
    if LIB_CANDIDATES.iter().any(|p| Path::new(p).exists()) {
        return true;
    }
    out_str("ldconfig", &["-p"])
        .lines()
        .any(|l| l.contains("libmodsecurity.so"))
}

/// 解析 `SecRuleEngine` 的当前值（On / Off / DetectionOnly）。
fn engine_of(main_conf: &Path) -> String {
    let Ok(content) = std::fs::read_to_string(main_conf) else {
        return String::new();
    };
    content
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("SecRuleEngine"))
        .map(|v| v.trim().to_string())
        .next()
        .unwrap_or_default()
}

/// 解析 `SecAuditLog` 指向的文件（取第一个非相对路径项）。
fn audit_log_of(main_conf: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(main_conf).ok()?;
    content
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("SecAuditLog"))
        .map(|v| v.trim().trim_matches('"').to_string())
        .find(|v| v.starts_with('/'))
        .map(PathBuf::from)
}

/// WAF 在当前机器上的完整画像。
struct WafEnv {
    installed: bool,
    /// 已编译出的 nginx 模块 .so
    module_so: Option<PathBuf>,
    libmodsecurity: bool,
    /// 规则目录（已存在或建议位置）
    rules_dir: PathBuf,
    /// 主配置文件（`modsecurity.conf`）
    main_conf: Option<PathBuf>,
    /// http 上下文的启用文件（缺它则模块不会被任何请求用到）
    enabled: Option<PathBuf>,
    crs: bool,
    engine: String,
    audit_log: Option<PathBuf>,
}

/// 规则目录：面板自有目录 `/etc/zap/nginx/modsecurity` 优先；为了兼容早年
/// 按 `{nginx conf}/modsecurity.d` 或发行版默认位置装过的机器，再依次回看。
fn rules_dir_of(conf: &Path) -> PathBuf {
    let suggested = Path::new(super::nginx::ZAP_NGINX_DIR).join(RULES_DIR_NAME);
    let legacy = conf
        .parent()
        .unwrap_or(Path::new("/etc/nginx"))
        .join("modsecurity.d");
    for dir in [
        suggested.clone(),
        legacy,
        PathBuf::from("/etc/modsecurity"),
        PathBuf::from("/usr/local/modsecurity/conf"),
    ] {
        if dir.is_dir() {
            return dir;
        }
    }
    suggested
}

/// 把主配置里的 `SecRuleEngine` 改成指定形态：已有该指令就替换（重复的旧指令一并
/// 去掉，免得后写覆盖前写），原本没有就追加到末尾。
fn with_engine(content: &str, mode: &str) -> String {
    let line = format!("SecRuleEngine {mode}");
    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;
    for l in content.lines() {
        if l.trim_start().starts_with("SecRuleEngine") {
            if !replaced {
                out.push(line.clone());
                replaced = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !replaced {
        out.push(line);
    }
    out.join("\n") + "\n"
}

/// 是否已部署 OWASP CRS（`crs-setup.conf` 且 rules/ 下有规则）。
fn crs_present(rules_dir: &Path) -> bool {
    if !rules_dir.join("crs-setup.conf").is_file() {
        return false;
    }
    let rules = rules_dir.join("rules");
    std::fs::read_dir(&rules)
        .map(|rd| rd.flatten().count() > 0)
        .unwrap_or(false)
}

/// 启用文件：http 上下文的 `modsecurity on; modsecurity_rules_file …`。
///
/// 只认 conf.d 下约定俗成的那一个文件（主配置模板已 `include conf.d/*.conf`）——
/// 不去猜用户把指令手写进了哪个站点配置文件，猜错就会让状态与事实不符。
fn enabled_conf(conf: &Path) -> Option<PathBuf> {
    let path = conf.parent()?.join("conf.d").join("modsecurity.conf");
    let text = std::fs::read_to_string(&path).ok()?;
    text.contains("modsecurity_rules_file").then_some(path)
}

/// 组装画像：`installed` 要求「库 + 模块 + 主配置 + 已启用」四项齐备。
///
/// 少了「已启用」这一项就会出假阳性：模块 .so 编出来了、规则也铺好了，但
/// http 上下文里没有 `modsecurity on;` —— nginx 照样不处理任何请求，面板却
/// 显示"已安装"，用户改规则改了半天空转。
fn waf_env(info: &NginxInfo) -> WafEnv {
    let lib = libmodsecurity_present();
    let so = modules_dir(&info.args).map(|d| d.join(MODULE_SO)).filter(|p| p.is_file());
    let rules_dir = rules_dir_of(&info.conf);
    let main_conf = [rules_dir.join("modsecurity.conf"), PathBuf::from("/etc/modsecurity/modsecurity.conf")]
        .into_iter()
        .find(|p| p.is_file());
    let enabled = enabled_conf(&info.conf);
    // 模块 .so 在盘上不等于生效：主配置里没 load_module，nginx 根本不会加载它
    let installed = lib
        && so.is_some()
        && main_conf.is_some()
        && conf_loads_module(&info.conf)
        && enabled.is_some();
    let engine = main_conf.as_ref().map(|p| engine_of(p)).unwrap_or_default();
    let audit_log = main_conf.as_ref().and_then(|p| audit_log_of(p));
    WafEnv {
        installed,
        module_so: so,
        libmodsecurity: lib,
        crs: crs_present(&rules_dir),
        rules_dir,
        main_conf,
        enabled,
        engine,
        audit_log,
    }
}

/// 全局 WAF 是否真的可用：组件齐备、已挂到 nginx、且引擎未被关成 `Off`。
///
/// 站点渲染 `modsecurity on;` 前必须过这一关。全局关掉之后站点配置里若还留着
/// 这条指令，`nginx -t` 会因模块不存在直接失败；引擎 Off 则属于"装了但不拦"，
/// 站点也不该再声明开启（面板有「全局关闭即刷新站点配置」的联动）。
pub(super) fn waf_ready() -> bool {
    let Some(info) = nginx_info() else {
        return false;
    };
    let env = waf_env(&info);
    env.installed && !env.engine.trim().eq_ignore_ascii_case("off")
}

/// 自动安装的前提清单：缺什么列什么（前端逐条展示，不笼统说"不支持"）。
fn blockers(info: &NginxInfo, env: &WafEnv) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if !env.libmodsecurity && !has_cmd("gcc") {
        out.push("缺少编译器（gcc / g++），无法编译 libmodsecurity".to_string());
    }
    if !has_cmd("make") {
        out.push("缺少 make".to_string());
    }
    if !has_cmd("curl") && !has_cmd("wget") {
        out.push("缺少下载工具（curl / wget），无法获取源码与规则集".to_string());
    }
    if !has_cmd("tar") {
        out.push("缺少 tar".to_string());
    }
    // 已编出模块但没挂上：不用重装，手工加一行 load_module 即可
    if env.module_so.is_some() && !conf_loads_module(&info.conf) {
        out.push(
            "模块已编译，但 nginx.conf 顶部缺少 load_module（点「一键开启」自动补上）"
                .to_string(),
        );
    }
    // 模块挂了但 http 上下文没启用：等同没装，给出可照做的一步
    if env.module_so.is_some() && env.enabled.is_none() {
        out.push(
            "模块已加载，但 http 上下文未启用（点「一键开启」自动补上 conf.d 启用配置）"
                .to_string(),
        );
    }
    // 动态模块的硬门槛：nginx 必须带 --with-compat，否则模块签名不匹配加载失败
    if !info.args.contains("--with-compat") {
        out.push(
            "当前 nginx 未以 --with-compat 编译，无法安全加装 ModSecurity 动态模块；\
             重新编译并替换 nginx 会让全站中断，面板不自动执行"
                .to_string(),
        );
    }
    out
}

/// `waf.status`：装没装、缺什么、能不能自动装。
pub async fn status() -> Response {
    run_blocking(|| {
        let Some(info) = nginx_info() else {
            return Ok(Response::ok(
                "ok",
                Some(json!({
                    "installed": false,
                    "nginx_installed": false,
                    "installable": false,
                    "blockers": ["未检测到 nginx，WAF 需要 nginx 承载"],
                    "hint": "请先安装 nginx（应用商店）再启用 WAF",
                })),
            ));
        };
        let env = waf_env(&info);
        let blockers = blockers(&info, &env);
        let files = conf_files(&env.rules_dir);
        Ok(Response::ok(
            "ok",
            Some(json!({
                "installed": env.installed,
                "nginx_installed": true,
                "nginx": {
                    "bin": info.bin.display().to_string(),
                    "conf": info.conf.display().to_string(),
                    "version": info.version,
                    // 动态模块的前提参数，UI 会明确展示"不满足即装不了"
                    "compat": info.args.contains("--with-compat"),
                    "load_module": conf_loads_module(&info.conf),
                    "running": nginx_running(),
                },
                "libmodsecurity": env.libmodsecurity,
                "module": env.module_so.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                "rules_dir": env.rules_dir.display().to_string(),
                "rules_dir_exists": env.rules_dir.is_dir(),
                "main_conf": env.main_conf.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                "enabled": env.enabled.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                "crs": env.crs,
                "engine": env.engine,
                "audit_log": env.audit_log.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                "files": files,
                "installable": !env.installed && blockers.is_empty(),
                // 组件都在、只差挂到 nginx 上 —— 这种情形不用重装，一键开启即可
                "enable_ready": !env.installed
                    && env.libmodsecurity
                    && env.module_so.is_some()
                    && env.main_conf.is_some(),
                "blockers": blockers,
                // 已安装时给一句用法提示，未安装时给结论
                "hint": if env.installed {
                    "规则引擎当前为 DetectionOnly（只记录不拦截）；确认审计日志无误杀后可改为 On".to_string()
                } else if blockers.is_empty() {
                    "可以安装：将编译 libmodsecurity 与 ModSecurity 动态模块，并部署 OWASP CRS".to_string()
                } else {
                    "当前环境无法自动安装，请按下方原因处理（可选功能，未安装时其余设置不可用）".to_string()
                },
            })),
        ))
    })
    .await
}

// ── 规则文件 ─────────────────────────────────────────────────

/// 规则目录下的 `.conf` / `.data`（含 rules 子目录，深度 2）。
fn conf_files(dir: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    if !dir.is_dir() {
        return out;
    }
    let mut stack: Vec<(PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if depth < 2 {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let is_conf = p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == "conf" || e == "data");
            if !is_conf {
                continue;
            }
            let rel = p.strip_prefix(dir).unwrap_or(&p).display().to_string();
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            out.push(json!({ "path": p.display().to_string(), "rel": rel, "size": size }));
        }
    }
    out
}

/// 未安装直接拒绝：这是"可选功能"的硬性边界。
fn require_installed() -> Result<WafEnv, String> {
    let info = nginx_info().ok_or_else(|| "未检测到 nginx".to_string())?;
    let env = waf_env(&info);
    if !env.installed {
        return Err(NOT_INSTALLED.to_string());
    }
    Ok(env)
}

/// 路径必须落在规则目录内（防目录穿越；规则目录是 WAF 唯一可写区域）。
fn validate_rule_path(env: &WafEnv, raw: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(raw.trim());
    if !p.is_absolute() {
        return Err(format!("路径必须是绝对路径: {raw}"));
    }
    let real = match std::fs::canonicalize(&p) {
        Ok(r) => r,
        // 文件尚不存在时（新建规则）校验父目录，避免误杀
        Err(_) => p
            .parent()
            .and_then(|d| std::fs::canonicalize(d).ok())
            .map(|d| d.join(p.file_name().unwrap_or_default()))
            .ok_or_else(|| format!("路径不存在: {raw}"))?,
    };
    let base = std::fs::canonicalize(&env.rules_dir).unwrap_or_else(|_| env.rules_dir.clone());
    if !real.starts_with(&base) {
        return Err(format!("只允许操作规则目录内的文件: {}", base.display()));
    }
    Ok(real)
}

/// `waf.conf_list`：规则文件清单（未安装则拒绝）。
pub async fn conf_list() -> Response {
    run_blocking(|| {
        let env = require_installed()?;
        Ok(Response::ok(
            "ok",
            Some(json!({ "rules_dir": env.rules_dir.display().to_string(), "files": conf_files(&env.rules_dir) })),
        ))
    })
    .await
}

/// `waf.conf_read`：读一个规则文件。
pub async fn conf_read(path: &str) -> Response {
    let path = path.to_string();
    run_blocking(move || {
        let env = require_installed()?;
        let p = validate_rule_path(&env, &path)?;
        let content = std::fs::read_to_string(&p).map_err(|e| format!("读取失败: {e}"))?;
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        Ok(Response::ok(
            "ok",
            Some(json!({ "path": p.display().to_string(), "content": content, "size": size })),
        ))
    })
    .await
}

/// `waf.conf_save`：写规则文件 —— 备份 → 写入 → `nginx -t` → 不过就回滚并重载。
///
/// 规则写错会让**所有站点**在 reload 时失败，所以这里比普通配置保存多一道
/// 校验：宁可回滚，也不留一份过不了 `-t` 的配置在盘上。
pub async fn conf_save(path: &str, content: &str) -> Response {
    let path = path.to_string();
    let content = content.to_string();
    run_blocking(move || {
        let env = require_installed()?;
        let p = validate_rule_path(&env, &path)?;
        let info = nginx_info().ok_or_else(|| "未检测到 nginx".to_string())?;

        let backup = if p.is_file() {
            let bak = p.with_extension("conf.zap.bak");
            std::fs::copy(&p, &bak).map_err(|e| format!("备份失败: {e}"))?;
            Some(bak)
        } else {
            None
        };
        if let Err(e) = std::fs::write(&p, &content) {
            return Err(format!("写入失败: {e}"));
        }
        if let Err(e) = nginx_test(&info.bin) {
            // 回滚：规则不合法时把原文件放回去，避免 nginx 起不来
            if let Some(bak) = &backup {
                let _ = std::fs::copy(bak, &p);
            }
            return Err(format!("配置未通过 nginx -t，已回滚：{e}"));
        }
        let reloaded = if nginx_running() {
            match super::svc::act("reload", "nginx") {
                Ok(_) => "nginx 已重载",
                Err(e) => return Err(format!("已保存，但重载失败：{e}")),
            }
        } else {
            "nginx 未运行，配置将在启动时生效"
        };
        Ok(Response::ok("ok", Some(json!({ "reload": reloaded }))))
    })
    .await
}

/// `waf.enable`：检测到已有 WAF 组件（库 + 模块 + 主配置）但没挂到 nginx 上时的一键开启。
///
/// 两步都只在缺失时才做：主配置顶部补 `load_module` → conf.d 下补启用文件
/// （已有同名文件则追加而不是覆盖，免得冲掉管理员手写的指令）。
/// 每一步都先备份，`nginx -t` 不过就整体回滚 —— 宁可什么都不做，也不留一份起不来的配置。
pub async fn enable() -> Response {
    run_blocking(|| {
        let info = nginx_info().ok_or_else(|| "未检测到 nginx".to_string())?;
        let env = waf_env(&info);
        // 缺库或缺模块属于"没装"，不该由开启流程兜底
        if !env.libmodsecurity || env.module_so.is_none() {
            return Err(
                "尚未检测到 ModSecurity 组件（缺 libmodsecurity 或 nginx 模块），请先安装".to_string(),
            );
        }
        let main_conf = env
            .main_conf
            .ok_or_else(|| "未找到 WAF 主配置（modsecurity.conf）".to_string())?;
        if env.installed {
            return Ok(Response::ok(
                "ok",
                Some(json!({ "enabled": env.enabled.as_ref().map(|p| p.display().to_string()).unwrap_or_default(), "reload": "WAF 已处于开启状态，无需操作" })),
            ));
        }

        // (备份, 目标)：备份为 None 表示新文件，回滚时删掉
        let mut restore: Vec<(Option<PathBuf>, PathBuf)> = Vec::new();

        // ① 主配置顶部补 load_module（缺才补）
        if !conf_loads_module(&info.conf) {
            let original = std::fs::read_to_string(&info.conf)
                .map_err(|e| format!("读取 {} 失败: {e}", info.conf.display()))?;
            let backup = info.conf.with_extension("conf.zap.bak");
            std::fs::copy(&info.conf, &backup).map_err(|e| format!("备份 nginx.conf 失败: {e}"))?;
            let next = format!("load_module modules/{MODULE_SO};\n{original}");
            std::fs::write(&info.conf, next).map_err(|e| format!("写入 nginx.conf 失败: {e}"))?;
            restore.push((Some(backup), info.conf.clone()));
        }

        // ② conf.d 下的启用文件（缺才写，已存在就追加）
        let enabled_path = info
            .conf
            .parent()
            .unwrap_or(Path::new("/etc/nginx"))
            .join("conf.d")
            .join("modsecurity.conf");
        if env.enabled.is_none() {
            if let Some(dir) = enabled_path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("创建 {} 失败: {e}", dir.display()))?;
            }
            let backup = if enabled_path.is_file() {
                let bak = enabled_path.with_extension("conf.zap.bak");
                std::fs::copy(&enabled_path, &bak).map_err(|e| format!("备份启用文件失败: {e}"))?;
                Some(bak)
            } else {
                None
            };
            let mut body = if enabled_path.is_file() {
                std::fs::read_to_string(&enabled_path).unwrap_or_default()
            } else {
                String::new()
            };
            body.push_str(&format!(
                "\n# Generated by Zap Panel — 启用 ModSecurity（http 上下文）— DO NOT EDIT\n\
                 modsecurity on;\n\
                 modsecurity_rules_file {};\n",
                main_conf.display()
            ));
            std::fs::write(&enabled_path, body).map_err(|e| format!("写入启用文件失败: {e}"))?;
            restore.push((backup, enabled_path.clone()));
        }

        // ③ 校验：不过就整体回滚
        if let Err(e) = nginx_test(&info.bin) {
            for (bak, target) in &restore {
                match bak {
                    Some(b) => {
                        let _ = std::fs::copy(b, target);
                    }
                    None => {
                        let _ = std::fs::remove_file(target);
                    }
                }
            }
            return Err(format!("配置未通过 nginx -t，已回滚：{e}"));
        }

        let action = if nginx_running() {
            match super::svc::act("reload", "nginx") {
                Ok(_) => "nginx 已重载，WAF 已生效",
                Err(e) => return Err(format!("已开启，但重载失败：{e}")),
            }
        } else {
            match super::svc::act("start", "nginx") {
                Ok(_) => "nginx 已启动，WAF 已生效",
                Err(e) => return Err(format!("已开启，但启动失败：{e}")),
            }
        };
        Ok(Response::ok(
            "ok",
            Some(json!({ "enabled": enabled_path.display().to_string(), "reload": action })),
        ))
    })
    .await
}

/// `waf.set_engine`：切换规则引擎形态 —— On（拦截）/ DetectionOnly（只记录）/ Off。
///
/// 装完默认 DetectionOnly：先看审计日志确认没有正常业务被误判，再切 On。
/// 与主配置保存同一套纪律：备份 → 写入 → `nginx -t` → 不过就回滚。
pub async fn set_engine(mode: &str) -> Response {
    // 白名单：mode 会被拼进配置文件，不能放行任意字符串
    let mode = match mode {
        "On" | "DetectionOnly" | "Off" => mode.to_string(),
        other => {
            return Response::err(
                -1,
                format!("不支持的规则引擎形态 '{other}'（可选：On / DetectionOnly / Off）"),
            )
        }
    };
    run_blocking(move || {
        let env = require_installed()?;
        let main = env
            .main_conf
            .ok_or_else(|| "未找到 WAF 主配置（modsecurity.conf）".to_string())?;
        let original =
            std::fs::read_to_string(&main).map_err(|e| format!("读取 {} 失败: {e}", main.display()))?;
        let next = with_engine(&original, &mode);
        if next == original {
            return Ok(Response::ok(
                "ok",
                Some(json!({ "engine": mode, "reload": "形态未变化，无需重载" })),
            ));
        }
        let backup = main.with_extension("conf.zap.bak");
        std::fs::copy(&main, &backup).map_err(|e| format!("备份失败: {e}"))?;
        if let Err(e) = std::fs::write(&main, &next) {
            return Err(format!("写入失败: {e}"));
        }
        let info = nginx_info().ok_or_else(|| "未检测到 nginx".to_string())?;
        if let Err(e) = nginx_test(&info.bin) {
            let _ = std::fs::copy(&backup, &main);
            return Err(format!("配置未通过 nginx -t，已回滚：{e}"));
        }
        let reloaded = if nginx_running() {
            match super::svc::act("reload", "nginx") {
                Ok(_) => "nginx 已重载",
                Err(e) => return Err(format!("已保存，但重载失败：{e}")),
            }
        } else {
            "nginx 未运行，配置将在启动时生效"
        };
        Ok(Response::ok(
            "ok",
            Some(json!({ "engine": mode, "reload": reloaded })),
        ))
    })
    .await
}

/// `waf.audit`：审计日志尾部（`SecAuditLog` 指向的文件，只读）。
pub async fn audit(lines: u32) -> Response {
    run_blocking(move || {
        let env = require_installed()?;
        let Some(log) = env.audit_log.clone() else {
            return Err("未配置 SecAuditLog，或审计日志文件不存在".to_string());
        };
        if !log.is_file() {
            return Err(format!("审计日志不存在: {}", log.display()));
        }
        let n = lines.clamp(1, 2000);
        let text = out_str("tail", &["-n", &n.to_string(), &log.display().to_string()]);
        Ok(Response::ok(
            "ok",
            Some(json!({ "path": log.display().to_string(), "content": text })),
        ))
    })
    .await
}

// ── 安装（长任务）───────────────────────────────────────────

const LIBMODSEC_VERSION: &str = "3.0.16";
/// ModSecurity-nginx 连接器版本（v1.x 对应 libmodsecurity v3）
const CONNECTOR_VERSION: &str = "1.0.4";
/// OWASP CRS 版本（下载源上提供的是 minimal 包）
const CRS_VERSION: &str = "4.29.0";
/// 源码工作目录
const SRC_DIR: &str = "/usr/local/src";

/// 下载工具：有 curl 用 curl，没有用 wget。
fn downloader() -> String {
    if has_cmd("curl") {
        "curl -fsSL {url} -o {out}"
    } else {
        "wget -qO {out} {url}"
    }
    .to_string()
}

/// 包下载源（与 appstore 安装脚本同源）：面板「系统设置 → 下载源」写入
/// `{data}/mirror.yaml`，没配就退回内置默认源。
///
/// WAF 的三个包（libmodsecurity / 连接器 / CRS）都从 `<源>/modsecurity/` 取，
/// **不访问 GitHub** —— 与 install.sh 保持一致的取包约定。
fn pkg_mirror() -> String {
    super::appstore::pkg_mirror_from_conf()
        .unwrap_or_else(|| "https://mirrors.zap.cn/pkg".to_string())
        .trim_end_matches('/')
        .to_string()
}

/// 从下载源的某个子目录取包：逐个试候选文件名（各镜像命名习惯不同），
/// 命中即下载到 `dest`；一个都没有就写清"去哪儿放包"，而不是甩一句下载失败。
fn fetch_from_mirror(mirror: &str, subdir: &str, dest: &str, candidates: &[String]) -> String {
    let list: Vec<String> = candidates.iter().map(|c| format!("'{c}'")).collect();
    let list = list.join(" ");
    let plain = candidates.join(" ");
    // 离线源（本地目录 / file://）：直接判文件是否存在并 cp，不发任何网络请求
    if mirror.starts_with('/') || mirror.starts_with("file://") {
        let base = mirror.strip_prefix("file://").unwrap_or(mirror);
        return format!(
            "ok=0; for n in {list}; do \
               if [ -f {base}/{subdir}/$n ]; then \
                 echo \"取包(本地源): {base}/{subdir}/$n\"; \
                 cp -f {base}/{subdir}/$n {dest} && ok=1 && break; \
               fi; \
             done; \
             if [ \"$ok\" != 1 ]; then \
               echo \"本地源 {base}/{subdir}/ 下没有可用的包（已试: {plain}）\"; \
               echo \"请把对应源码包放进该目录\"; exit 1; \
             fi"
        );
    }
    let get = downloader()
        .replace("{url}", &format!("{mirror}/{subdir}/$n"))
        .replace("{out}", dest);
    format!(
        "ok=0; for n in {list}; do \
           if curl -sfI -m 10 {mirror}/{subdir}/$n >/dev/null 2>&1 \
              || wget -q --spider -T 10 {mirror}/{subdir}/$n >/dev/null 2>&1; then \
             echo \"取包: {mirror}/{subdir}/$n\"; {get} && ok=1 && break; \
           fi; \
         done; \
         if [ \"$ok\" != 1 ]; then \
           echo \"下载源 {mirror}/{subdir}/ 下没有可用的包（已试: {plain}）\"; \
           echo \"请把对应源码包放进该目录\"; exit 1; \
         fi"
    )
}

/// libmodsecurity 的候选包名（镜像里叫什么都有可能，逐个探测）
fn lib_candidates() -> Vec<String> {
    vec![
        // 下载源里的实际命名排前面，其余是别的镜像可能用的写法
        format!("modsecurity-v{LIBMODSEC_VERSION}.tar.gz"),
        format!("libmodsecurity-v{LIBMODSEC_VERSION}.tar.gz"),
        format!("libmodsecurity-{LIBMODSEC_VERSION}.tar.gz"),
        format!("ModSecurity-{LIBMODSEC_VERSION}.tar.gz"),
        format!("ModSecurity-v{LIBMODSEC_VERSION}.tar.gz"),
        format!("modsecurity-v{LIBMODSEC_VERSION}.tar.gz"),
        format!("modsecurity-{LIBMODSEC_VERSION}.tar.gz"),
    ]
}

fn connector_candidates() -> Vec<String> {
    vec![
        format!("ModSecurity-nginx-v{CONNECTOR_VERSION}.tar.gz"),
        "ModSecurity-nginx.tar.gz".to_string(),
        "ModSecurity-nginx-v3-master.tar.gz".to_string(),
        "ModSecurity-nginx-master.tar.gz".to_string(),
        "modsecurity-nginx.tar.gz".to_string(),
        "ngx_http_modsecurity.tar.gz".to_string(),
    ]
}

fn crs_candidates() -> Vec<String> {
    vec![
        // 下载源给的是 minimal 包（不含 tests/ 之类的冗余内容）
        format!("coreruleset-{CRS_VERSION}-minimal.tar.gz"),
        format!("coreruleset-{CRS_VERSION}.tar.gz"),
        format!("coreruleset-v{CRS_VERSION}.tar.gz"),
        format!("OWASP-CRS-{CRS_VERSION}.tar.gz"),
        format!("owasp-crs-{CRS_VERSION}.tar.gz"),
        format!("crs-{CRS_VERSION}.tar.gz"),
    ]
}

/// `waf.install`：仅在 status 判定可安装时允许启动，否则一条都跑不了。
pub async fn install(log_path: &str) -> Response {
    let log_path = log_path.to_string();
    run_blocking(move || {
        let info = match nginx_info() {
            Some(i) => i,
            None => {
                super::log_line(&log_path, "未检测到 nginx，无法安装 WAF");
                super::finish_log(&log_path, 1);
                return Ok(Response::ok(
                    "ok",
                    Some(json!({ "started": false, "reason": "未检测到 nginx" })),
                ));
            }
        };
        let env = waf_env(&info);
        if env.installed {
            return Ok(Response::ok(
                "ok",
                Some(json!({ "started": false, "reason": "已安装" })),
            ));
        }
        let blockers = blockers(&info, &env);
        if !blockers.is_empty() {
            for b in &blockers {
                super::log_line(&log_path, &format!("无法安装：{b}"));
            }
            super::finish_log(&log_path, 1);
            return Ok(Response::ok(
                "ok",
                Some(json!({ "started": false, "reason": blockers.join("；") })),
            ));
        }

        super::log_line(&log_path, "开始安装 ModSecurity（WAF）");
        let log = log_path.clone();
        let version = info.version.clone();
        let args = info.args.clone();
        let conf = info.conf.clone();
        let bin = info.bin.clone();
        let rules_dir = env.rules_dir.clone();
        let modules = modules_dir(&args).unwrap_or_else(|| PathBuf::from("/usr/local/nginx/modules"));
        std::thread::spawn(move || {
            let code = install_inner(&log, &version, &args, &conf, &bin, &rules_dir, &modules);
            super::finish_log(&log, code);
        });
        Ok(Response::ok("ok", Some(json!({ "started": true }))))
    })
    .await
}

/// 安装主体（后台线程）：库 → 连接器 → 动态模块 → 规则集 → 挂载 → 校验。
///
/// 每一步失败即停，日志里留现场；最后一步 `nginx -t` 不过会整体回滚
/// （撤掉 load_module 与 include），确保 nginx 始终可启动。
#[allow(clippy::too_many_arguments)]
fn install_inner(
    log: &str,
    nginx_version: &str,
    args: &str,
    conf: &Path,
    bin: &Path,
    rules_dir: &Path,
    modules: &Path,
) -> i32 {
    let mirror = pkg_mirror();
    // 从 `nginx/1.31.5` 里取版本号，用于下载对应源码编动态模块
    let ver = nginx_version.split('/').last().unwrap_or("").trim().to_string();

    // 1) 构建依赖（有 apt 才装，别的发行版假定已具备）
    if has_cmd("apt-get") {
        if super::run_step(
            log,
            "安装编译依赖",
            "apt-get update -qq && apt-get install -y -qq --no-install-recommends \
             libtool autoconf automake g++ make pkg-config libpcre3-dev libxml2-dev \
             libcurl4-openssl-dev libgeoip-dev libyajl-dev flex bison",
        ) != 0
        {
            super::log_line(log, "依赖安装失败，中止（后续编译大概率也过不去）");
            return 1;
        }
    }

    // 2) libmodsecurity（规则引擎）—— 从下载源 pkg/modsecurity/ 取，不碰 GitHub
    let lib_script = format!(
        "set -e; cd {SRC_DIR}; \
         {fetch}; \
         src=$(tar -tzf libmodsecurity.tar.gz | head -1 | cut -d/ -f1); \
         tar xzf libmodsecurity.tar.gz; \
         cd \"$src\"; \
         ./build.sh; ./configure --prefix=/usr/local/modsecurity --without-lmdb; \
         make -j$(nproc); make install",
        fetch = fetch_from_mirror(&mirror, "modsecurity", "libmodsecurity.tar.gz", &lib_candidates()),
    );
    if super::run_step(log, "编译 libmodsecurity", &lib_script) != 0 {
        super::log_line(log, "libmodsecurity 编译失败");
        return 1;
    }

    // 3) nginx 源码 + 动态模块（要求 --with-compat，已在 blockers 里校验过）
    let mod_script = format!(
        "set -e; cd {SRC_DIR}; \
         {fetch_nginx}; tar xzf nginx.tar.gz; \
         {fetch_conn}; mkdir -p connector; \
         tar xzf connector.tar.gz --strip-components=1 -C connector; \
         cd nginx-{ver}; \
         ./configure {args} --with-compat --add-dynamic-module={SRC_DIR}/connector; \
         make -j$(nproc) modules; \
         mkdir -p {modules}; \
         cp objs/{MODULE_SO} {modules}/",
        // nginx 源码与安装脚本同源：源的 nginx/ 目录
        fetch_nginx = fetch_from_mirror(
            &mirror,
            "nginx",
            "nginx.tar.gz",
            &[format!("nginx-{ver}.tar.gz")]
        ),
        fetch_conn = fetch_from_mirror(
            &mirror,
            "modsecurity",
            "connector.tar.gz",
            &connector_candidates()
        ),
        modules = modules.display(),
    );
    if super::run_step(log, "编译 ModSecurity nginx 模块", &mod_script) != 0 {
        super::log_line(log, "动态模块编译失败（常见原因：nginx 源码版本与当前 nginx 不一致）");
        return 1;
    }

    // 4) 规则集：主配置 + OWASP CRS（默认 DetectionOnly，先观察再拦截）
    let rules = rules_dir.display().to_string();
    let crs_script = format!(
        // 一行一条语句（用 \n 分隔），避免 shell 续行反斜杠与 Rust 转义互相纠缠
        "set -e\nmkdir -p {rules}/rules\ncd {SRC_DIR}\n{fetch_crs}\n\
         tar xzf crs.tar.gz --strip-components=1 -C {rules}\n\
         if [ ! -f {rules}/crs-setup.conf ] && [ -f {rules}/crs-setup.conf.example ]; then cp {rules}/crs-setup.conf.example {rules}/crs-setup.conf; fi\n\
         printf '%s\\n' 'SecRuleEngine DetectionOnly' 'SecRequestBodyAccess On' \
         'SecAuditEngine RelevantOnly' 'SecAuditLogRelevantStatus \"^(?:5|4(?!04))\"' \
         'SecAuditLogParts ABIJDEFHZ' 'SecAuditLogType Serial' \
         'SecAuditLog /var/log/modsec_audit.log' > {rules}/modsecurity.conf\n\
         if [ -f {rules}/crs-setup.conf ]; then echo 'Include {rules}/crs-setup.conf' >> {rules}/modsecurity.conf; fi\n\
         if [ -d {rules}/rules ]; then echo 'Include {rules}/rules/*.conf' >> {rules}/modsecurity.conf; fi",
        fetch_crs = fetch_from_mirror(&mirror, "modsecurity", "crs.tar.gz", &crs_candidates()),
    );
    if super::run_step(log, "部署 OWASP CRS 规则集", &crs_script) != 0 {
        super::log_line(log, "规则集部署失败");
        return 1;
    }

    // 5) 挂载到 nginx：load_module 必须在主配置最外层（events/http 之前）
    let Ok(content) = std::fs::read_to_string(conf) else {
        super::log_line(log, "读取 nginx.conf 失败");
        return 1;
    };
    let bak = conf.with_extension("conf.zap.bak");
    if std::fs::copy(conf, &bak).is_err() {
        super::log_line(log, "备份 nginx.conf 失败，为安全起见中止");
        return 1;
    }
    // 启用：http 上下文指令必须落在 http 块内。主配置模板已 include conf.d/*.conf，
    // 单独成文件即可，不用去动模板结构。
    let conf_d = conf
        .parent()
        .map(|p| p.join("conf.d"))
        .unwrap_or_else(|| PathBuf::from("/etc/nginx/conf.d"));
    if std::fs::create_dir_all(&conf_d).is_err() {
        super::log_line(log, &format!("创建 {} 失败", conf_d.display()));
        return 1;
    }
    let enabled = conf_d.join("modsecurity.conf");
    let enabled_text = format!(
        "# zap 生成：WAF 启用（http 上下文）\nmodsecurity on;\nmodsecurity_rules_file {rules}/modsecurity.conf;\n"
    );
    if std::fs::write(&enabled, enabled_text).is_err() {
        super::log_line(log, &format!("写入 {} 失败", enabled.display()));
        return 1;
    }

    let so = modules.join(MODULE_SO);
    let mut new_conf = String::new();
    new_conf.push_str(&format!("load_module {};\n", so.display()));
    new_conf.push_str(&content);
    if std::fs::write(conf, &new_conf).is_err() {
        super::log_line(log, "写入 nginx.conf 失败");
        return 1;
    }
    if let Err(e) = nginx_test(bin) {
        // 回滚：模块加载不了就恢复原配置，绝不留下起不来的 nginx
        let _ = std::fs::copy(&bak, conf);
        let _ = std::fs::remove_file(&enabled);
        super::log_line(log, &format!("nginx -t 未通过，已回滚 nginx.conf：{e}"));
        return 1;
    }
    super::log_line(log, "已加载 ModSecurity 模块（规则引擎：DetectionOnly，只记录不拦截）");
    super::log_line(
        log,
        "提示：在规则目录的 modsecurity.conf 里把 SecRuleEngine 改为 On 才会真正拦截",
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_dir_prefers_modules_path() {
        assert_eq!(
            modules_dir("--prefix=/opt/nginx --modules-path=/opt/nginx/mod"),
            Some(PathBuf::from("/opt/nginx/mod"))
        );
        // 没有显式 modules-path 时退回 <prefix>/modules
        assert_eq!(
            modules_dir("--prefix=/opt/nginx --with-http_ssl_module"),
            Some(PathBuf::from("/opt/nginx/modules"))
        );
    }

    #[test]
    fn compat_is_required_for_dynamic_module() {
        // 本机 nginx 的实际情况：无 --with-compat → 必须被判为不可自动安装
        let info = NginxInfo {
            bin: PathBuf::from("/usr/local/apps/nginx/sbin/nginx"),
            conf: PathBuf::from("/usr/local/apps/nginx/conf/nginx.conf"),
            version: "nginx/1.31.5".to_string(),
            args: "--prefix=/usr/local/apps/nginx-1.31.5 --with-http_ssl_module".to_string(),
        };
        let env = WafEnv {
            installed: false,
            module_so: None,
            libmodsecurity: false,
            rules_dir: PathBuf::from("/usr/local/apps/nginx/conf/modsecurity.d"),
            main_conf: None,
            crs: false,
            engine: String::new(),
            enabled: None,
            audit_log: None,
        };
        let b = blockers(&info, &env);
        assert!(!env.installed);
        assert!(
            b.iter().any(|x| x.contains("--with-compat")),
            "无 --with-compat 必须列为不可安装原因：{b:?}"
        );
    }

    #[test]
    fn rule_path_must_stay_in_rules_dir() {
        let env = WafEnv {
            installed: true,
            module_so: None,
            libmodsecurity: true,
            rules_dir: std::env::temp_dir().join("zap-waf-rules-test"),
            main_conf: None,
            crs: false,
            engine: "DetectionOnly".to_string(),
            enabled: None,
            audit_log: None,
        };
        std::fs::create_dir_all(&env.rules_dir).unwrap();
        assert!(validate_rule_path(&env, "/etc/passwd").is_err());
        let inside = env.rules_dir.join("zap-test.conf").display().to_string();
        assert!(validate_rule_path(&env, &inside).is_ok());
        let _ = std::fs::remove_dir_all(&env.rules_dir);
    }

    #[test]
    fn module_without_load_module_is_not_installed() {
        // 编出 .so 但主配置没 load_module = 没生效，且应给出可操作指引（而非要求重装）
        let dir = std::env::temp_dir().join("zap-waf-mod-test");
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("modsecurity.conf");
        std::fs::write(&main, "SecRuleEngine DetectionOnly\n").unwrap();
        // 主配置也用临时文件：否则 conf_loads_module 读到的是真实 nginx.conf，
        // 机器上挂没挂 load_module 会直接决定这条断言的成败（本机 WAF 已开启即失败）。
        let conf = dir.join("nginx.conf");
        std::fs::write(&conf, "worker_processes  1;\n").unwrap();
        let info = NginxInfo {
            bin: PathBuf::from("/usr/local/apps/nginx/sbin/nginx"),
            conf,
            version: "nginx/1.31.5".to_string(),
            args: "--prefix=/usr/local/apps/nginx --with-compat".to_string(),
        };
        let env = WafEnv {
            installed: false,
            module_so: Some(PathBuf::from(
                "/usr/local/apps/nginx/modules/ngx_http_modsecurity_module.so",
            )),
            libmodsecurity: true,
            rules_dir: dir.clone(),
            main_conf: Some(main),
            crs: false,
            engine: "DetectionOnly".to_string(),
            enabled: None,
            audit_log: None,
        };
        let b = blockers(&info, &env);
        assert!(
            b.iter().any(|x| x.contains("load_module")),
            "应提示补 load_module 而不是要求重装：{b:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 下载源 pkg/modsecurity/ 里的实际文件名必须落在候选里：版本号一改就失配，
    /// 失配的后果是安装直接报"源里没有可用的包"，所以钉死这条。
    #[test]
    fn mirror_candidates_cover_source_layout() {
        for (name, cands) in [
            (
                format!("modsecurity-v{LIBMODSEC_VERSION}.tar.gz"),
                lib_candidates(),
            ),
            (
                format!("ModSecurity-nginx-v{CONNECTOR_VERSION}.tar.gz"),
                connector_candidates(),
            ),
            (
                format!("coreruleset-{CRS_VERSION}-minimal.tar.gz"),
                crs_candidates(),
            ),
        ] {
            assert!(cands.contains(&name), "候选里缺少 {name}：{cands:?}");
        }
    }

    /// 规则目录跟面板自有的 nginx 目录走（与 zap-stream.conf 同级）
    #[test]
    fn rules_dir_lives_in_panel_nginx_dir() {
        let suggested = Path::new(super::super::nginx::ZAP_NGINX_DIR).join(RULES_DIR_NAME);
        assert_eq!(suggested, PathBuf::from("/etc/zap/nginx/modsecurity"));
    }

    /// 切换引擎:已有指令就替换(且只留一行),其余指令原样保留
    #[test]
    fn engine_switch_rewrites_in_place() {
        let out = super::with_engine(
            "# zap 生成\nSecRuleEngine DetectionOnly\nSecRequestBodyAccess On\n",
            "On",
        );
        assert!(out.contains("SecRuleEngine On"), "{out}");
        assert!(!out.contains("DetectionOnly"), "{out}");
        assert_eq!(out.matches("SecRuleEngine").count(), 1, "{out}");
        assert!(out.contains("SecRequestBodyAccess On"), "{out}");
    }

    /// 重复写了两行 SecRuleEngine 时,只保留一行新值
    #[test]
    fn engine_switch_collapses_duplicates() {
        let out =
            super::with_engine("SecRuleEngine On\nSecRuleEngine DetectionOnly\n", "Off");
        assert_eq!(out.matches("SecRuleEngine").count(), 1, "{out}");
        assert!(out.contains("SecRuleEngine Off"), "{out}");
    }

    /// 原本没有该指令:追加到末尾
    #[test]
    fn engine_switch_appends_when_missing() {
        let out = super::with_engine("SecRequestBodyAccess On\n", "DetectionOnly");
        assert!(out.trim_end().ends_with("SecRuleEngine DetectionOnly"), "{out}");
        assert!(out.contains("SecRequestBodyAccess On"), "{out}");
    }

    #[test]
    fn mirror_fetch_scripts_are_valid_shell() {
        // 生成的脚本直接喂给 bash 执行，语法错了会在安装时才炸；这里提前挡住
        for script in [
            fetch_from_mirror(
                "https://mirrors.example.com/pkg",
                "modsecurity",
                "x.tar.gz",
                &lib_candidates(),
            ),
            fetch_from_mirror("/opt/local-mirror", "modsecurity", "x.tar.gz", &crs_candidates()),
        ] {
            let out = std::process::Command::new("bash")
                .args(["-n", "-c", &script])
                .output()
                .expect("bash 可用");
            assert!(
                out.status.success(),
                "取包脚本语法错误: {}\n{}",
                script,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// 本地目录源要真的能把包取出来（离线环境没有网络，靠 cp）。
    #[test]
    fn local_mirror_picks_existing_package() {
        let root = std::env::temp_dir().join("zap-waf-mirror-test");
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("modsecurity");
        std::fs::create_dir_all(&dir).unwrap();
        let name = &crs_candidates()[1]; // 故意放第二个候选：验证是"逐个试"而非只认第一个
        std::fs::write(dir.join(name), b"packet").unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();

        let script = format!(
            "cd {}; {}; test -s crs.tar.gz",
            root.join("work").display(),
            fetch_from_mirror(
                &root.display().to_string(),
                "modsecurity",
                "crs.tar.gz",
                &crs_candidates()
            )
        );
        let out = std::process::Command::new("bash")
            .args(["-c", &script])
            .output()
            .expect("bash 可用");
        assert!(
            out.status.success(),
            "本地源取包失败: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nginx_source_version_is_parsed() {
        let v = "nginx/1.31.5";
        assert_eq!(v.split('/').last().unwrap(), "1.31.5");
    }
}
