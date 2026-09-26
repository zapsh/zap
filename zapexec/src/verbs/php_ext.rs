//! PHP 扩展管理（服务配置 → PHP → 各实例）
//!
//! 一个 PHP 实例（php74 / php81 …）装了哪些扩展、能不能装新的，取决于它的
//! 安装形态：应用商店编译的实例自带 `phpize` / `php-config`，系统包实例的
//! 扩展由发行版包管理。这里只做三件事，并把「装不了」的原因讲清楚：
//!
//! - `list`：`php -m`（已启用）+ 扩展目录扫描（已编译未启用）+ 内置扩展，
//!   以及可用的安装工具（pie / pecl / phpize / 编译器）与选路结论。
//! - `toggle`：写 / 删 `{php.d}/zap-ext-<name>.ini`，改完重载 php-fpm。
//! - `install` / `remove`：长任务（编译分钟级），日志追加到 zapd 给的
//!   `log_path`，结束写 `__ZAP_DONE__ <code>`（与 appstore / docker 同一套约定）。
//!
//! 安装选路：`PIE`（PHP >= 8.1 且有 pie）→ `pecl` → 源码编译（phpize +
//! configure + make）。前两条都不成立时（如 7.4 实例没装 pear）走源码，
//! 从 pecl.php.net 取稳定版 tarball 就地编译 —— 这条路径不依赖任何包管理器。
//!
//! 安全边界：扩展名 / 包名只放行 `[a-z0-9_./-]`；所有命令以 argv 数组下发，
//! 只有「拼接好的构建脚本」会进 bash -c，且脚本里的变量值均来自本文件常量或
//! 已校验的扩展名。

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use zap_proto::Response;

use super::root_cmd;
use super::service_conf::php_inst;
use super::svc;

/// 日志里的完成标记（zapd 的 task 盯守认它，详见 zapd/zap/task.rs）。
const DONE_MARKER: &str = "__ZAP_DONE__";
/// PIE 自身要求 PHP >= 8.1（低于此版本的实例直接走 pecl / 源码）。
const PIE_MIN_MAJOR: u32 = 8;
const PIE_MIN_MINOR: u32 = 1;
/// zend 扩展必须这样声明（opcache / xdebug 用 extension= 会被忽略）。
const ZEND_EXTS: &[&str] = &["opcache", "xdebug", "ioncube", "zend"];
/// 内置扩展（编译进解释器，无 .so，无法禁用）。
const BUILTIN_HINT: &str = "内置";

// ── 上下文：找到实例的 php / phpize / 扩展目录 ──────────────────

/// 一个 PHP 实例的可执行文件与路径信息。
struct PhpCtx {
    /// php 主程序（CLI）
    bin: PathBuf,
    /// `extension=xxx.so` 的查找目录
    ext_dir: PathBuf,
    /// 额外 ini 扫描目录（`php --ini` 的 Scan for additional .ini files）
    scan_dir: Option<PathBuf>,
    /// 主 php.ini
    ini: Option<PathBuf>,
    /// php-config（源码编译扩展时用）
    php_config: Option<PathBuf>,
    /// 版本串（如 7.4.33）
    version: String,
    /// systemd unit（php-fpm-<ver> 等，改完要重载）
    unit: Option<String>,
}

/// 实例 svc → php 可执行文件。应用商店实例取 `{dir}/bin/php`，类型级 php 取 PATH 里的 php。
fn php_bin(svc: &str) -> Option<PathBuf> {
    if let Some(inst) = php_inst(svc) {
        let b = inst.dir.join("bin").join("php");
        if b.is_file() {
            return Some(b);
        }
    }
    which("php")
}

/// `command -v name` 定位可执行文件。
fn which(name: &str) -> Option<PathBuf> {
    let o = root_cmd(super::platform::SHELL)
        .args(["-c"])
        .arg(format!("command -v {name} 2>/dev/null"))
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!p.is_empty()).then(|| PathBuf::from(p))
}

/// 跑一次 php，返回 stdout（失败给空串）。
fn php_out(bin: &Path, args: &[&str]) -> String {
    let o = root_cmd(bin.to_string_lossy().as_ref())
        .args(args)
        .output();
    match o {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => String::new(),
    }
}

/// 解析 `php --ini`：`Loaded Configuration File` 与 `Scan for additional .ini files in`。
fn parse_ini_paths(bin: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let out = php_out(bin, &["--ini"]);
    let mut ini = None;
    let mut scan = None;
    for line in out.lines() {
        if let Some(v) = line.split_once(':') {
            let key = v.0.trim();
            let val = v.1.trim();
            if val.is_empty() || val == "(none)" {
                continue;
            }
            if key.starts_with("Loaded Configuration File") {
                ini = Some(PathBuf::from(val));
            } else if key.starts_with("Scan for additional") {
                scan = Some(PathBuf::from(val));
            }
        }
    }
    (ini, scan)
}

/// 组装实例上下文；拿不到 php 即视为未安装。
fn ctx(svc: &str) -> Result<PhpCtx, String> {
    let bin = php_bin(svc).ok_or_else(|| format!("未找到 {svc} 的 php 可执行文件"))?;
    let version = php_out(&bin, &["-r", "echo PHP_VERSION;"]);
    let ext_dir = php_out(&bin, &["-r", "echo ini_get('extension_dir');"]);
    if ext_dir.is_empty() {
        return Err("无法读取 extension_dir（php 不可用？）".to_string());
    }
    let (ini, scan) = parse_ini_paths(&bin);
    let php_config = php_inst(svc)
        .map(|i| i.dir.join("bin").join("php-config"))
        .filter(|p| p.is_file())
        .or_else(|| which("php-config"));
    // 实例的 fpm unit：php-fpm-<ver> / info.yaml 登记的 svc_name
    let unit = if let Some(inst) = php_inst(svc) {
        let u = format!("php-fpm-{}", inst.digits);
        svc::exists(&u).then_some(u)
    } else {
        None
    };
    Ok(PhpCtx {
        bin,
        ext_dir: PathBuf::from(ext_dir),
        scan_dir: scan,
        ini,
        php_config,
        version,
        unit,
    })
}

/// 版本串的主次版本号（7.4.33 → (7, 4)）。
fn major_minor(v: &str) -> (u32, u32) {
    let mut it = v.split('.');
    let major = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor)
}

// ── 清单 ────────────────────────────────────────────────────

/// 已启用扩展名（`php -m`，跳过首行 `[PHP Modules]`）。
fn enabled_exts(bin: &Path) -> Vec<String> {
    php_out(bin, &["-m"])
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('['))
        .collect()
}

/// 扩展目录里已编译的 `<name>.so`（不含路径，去 .so 后缀）。
fn compiled_exts(ext_dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(ext_dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?;
            name.strip_suffix(".so").map(|n| n.to_string())
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 某个扩展的启用来源：php.d 里我们写的文件 / 其它 ini 文件 / 内置。
fn ini_source(scan: Option<&Path>, ini: Option<&Path>, name: &str) -> Option<String> {
    let want = format!("{name}.so");
    let mut dirs: Vec<&Path> = Vec::new();
    if let Some(s) = scan {
        dirs.push(s);
    }
    if let Some(i) = ini {
        dirs.push(i);
    }
    for d in dirs {
        if !d.is_dir() {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            if f.extension().and_then(|e| e.to_str()) != Some("ini") {
                continue;
            }
            if let Ok(c) = std::fs::read_to_string(&f)
                && c.lines()
                    .any(|l| l.trim_start().starts_with("extension") && l.contains(&want))
            {
                return Some(f.display().to_string());
            }
        }
    }
    None
}

/// 扩展版本号：已启用的直接问解释器，未启用的临时加载 .so 再问。
fn ext_version(c: &PhpCtx, name: &str, enabled: bool) -> String {
    if enabled {
        return php_out(&c.bin, &["-r", &format!("echo phpversion('{name}');")]);
    }
    let so = c.ext_dir.join(format!("{name}.so"));
    if !so.is_file() {
        return String::new();
    }
    php_out(
        &c.bin,
        &[
            "-d",
            &format!("extension={}", so.display()),
            "-r",
            &format!("echo phpversion('{name}');"),
        ],
    )
}

/// `php_ext.list`：清单 + 工具可用性 + 安装选路结论。
pub async fn list(svc: &str) -> Response {
    let svc = svc.to_string();
    super::service_conf::run_blocking(move || {
        let c = match ctx(&svc) {
            Ok(c) => c,
            Err(e) => {
                return Ok(Response::ok(
                    "ok",
                    Some(json!({ "installed": false, "reason": e, "extensions": [] })),
                ))
            }
        };
        let enabled = enabled_exts(&c.bin);
        let compiled = compiled_exts(&c.ext_dir);
        let mut items: Vec<Value> = Vec::new();
        let mut names: Vec<String> = compiled.clone();
        for n in &enabled {
            if !names.iter().any(|x| x == n) {
                names.push(n.clone());
            }
        }
        names.sort();
        for name in names {
            let on = enabled.iter().any(|e| e.eq_ignore_ascii_case(&name));
            let builtin = !compiled.iter().any(|x| x == &name);
            let so = c.ext_dir.join(format!("{name}.so"));
            items.push(json!({
                "name": name,
                "version": ext_version(&c, &name, on),
                "enabled": on,
                // 内置扩展没有 .so，禁用按钮不给出（UI 据此置灰）
                "builtin": builtin,
                "so": if so.is_file() { so.display().to_string() } else { String::new() },
                "ini": ini_source(c.scan_dir.as_deref(), c.ini.as_deref(), &name).unwrap_or_default(),
                "removable": !builtin && so.is_file(),
            }));
        }
        let has = |b: &str| which(b).is_some();
        let (major, minor) = major_minor(&c.version);
        let pie_ok = has("pie") && (major, minor) >= (PIE_MIN_MAJOR, PIE_MIN_MINOR);
        let pecl_ok = has("pecl");
        let phpize_ok = php_inst(&svc)
            .map(|i| i.dir.join("bin").join("phpize").is_file())
            .unwrap_or(false)
            || has("phpize");
        // 选路：PIE → pecl → 源码编译（phpize + 编译器）
        let (installer, hint) = if pie_ok {
            ("pie", "使用 PIE 安装（PHP >= 8.1）")
        } else if pecl_ok {
            ("pecl", "使用 pecl 安装")
        } else if phpize_ok {
            (
                "source",
                "未检测到 pie / pecl，将从 pecl.php.net 取源码就地编译（phpize + make）",
            )
        } else {
            (
                "none",
                "该实例缺少 phpize / pecl / pie 与编译环境，无法安装扩展（发行版系统包请用系统包管理器安装 php-扩展包）",
            )
        };
        Ok(Response::ok(
            "ok",
            Some(json!({
                "installed": true,
                "service": svc,
                "bin": c.bin.display().to_string(),
                "version": c.version,
                "extension_dir": c.ext_dir.display().to_string(),
                "scan_dir": c.scan_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                "ini": c.ini.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                "tools": {
                    "pie": pie_ok,
                    "pecl": pecl_ok,
                    "phpize": phpize_ok,
                    "php_config": c.php_config.is_some(),
                    "gcc": has("gcc") || has("cc"),
                    "make": has("make"),
                },
                "installer": installer,
                "installer_hint": hint,
                "extensions": items,
            })),
        ))
    })
    .await
}

// ── 启用 / 禁用 ─────────────────────────────────────────────

/// 扩展名 / 包名白名单：只允许 `[a-z0-9_./-]`，且不含 `..`。
fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '/' | '-'))
}

/// 我们管理的 ini 文件名（`zap-ext-<name>.ini`），改它不碰别人写的配置。
fn managed_ini(c: &PhpCtx, name: &str) -> Result<PathBuf, String> {
    let dir = c
        .scan_dir
        .clone()
        .or_else(|| c.ini.as_ref().and_then(|p| p.parent().map(PathBuf::from)))
        .ok_or_else(|| "未定位到 php.d / php.ini 目录，无法写入扩展配置".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 {} 目录失败: {e}", dir.display()))?;
    Ok(dir.join(format!("zap-ext-{name}.ini")))
}

/// 声明行：zend 扩展用 `zend_extension=`，其余 `extension=`。
fn decl_line(name: &str) -> String {
    if ZEND_EXTS.contains(&name) {
        format!("zend_extension={name}.so\n")
    } else {
        format!("extension={name}.so\n")
    }
}

/// 写盘（先备份同名文件，再原子替换）。
fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Ok(_meta) = std::fs::metadata(path) {
        let bak = format!("{}.bak", path.display());
        let _ = std::fs::copy(path, &bak);
    }
    let tmp = format!("{}.tmp", path.display());
    std::fs::write(&tmp, content).map_err(|e| format!("写入 {tmp} 失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("替换 {} 失败: {e}", path.display()))
}

/// 重载 php-fpm：unit 存在且在运行则 reload，否则只是改了文件（下次启动生效）。
fn reload(c: &PhpCtx) -> String {
    match &c.unit {
        Some(u) if svc::is_active(u) => match super::svc::act("reload", u) {
            Ok(_) => "已重载 php-fpm".to_string(),
            Err(e) => format!("配置已写入，但重载 php-fpm 失败: {e}"),
        },
        Some(u) => format!("配置已写入；{u} 未运行，启动后生效"),
        None => "配置已写入；未找到 php-fpm 服务，需手动重启 PHP".to_string(),
    }
}

/// `php_ext.toggle`：启用 / 禁用一个扩展。
pub async fn toggle(svc: &str, name: &str, enable: bool) -> Response {
    let svc = svc.to_string();
    let name = name.to_string();
    super::service_conf::run_blocking(move || {
        if !name_ok(&name) {
            return Err(format!("扩展名不合法: {name}"));
        }
        let c = ctx(&svc)?;
        let compiled = c.ext_dir.join(format!("{name}.so")).is_file();
        // 内置扩展（无 .so）只能启用（本来就开着），禁用无从下手
        if !compiled && !enable {
            return Err(format!("{name} 是 {BUILTIN_HINT}扩展，无法禁用"));
        }
        if enable && !compiled {
            return Err(format!(
                "扩展目录里没有 {name}.so，请先安装该扩展（{}）",
                c.ext_dir.join(format!("{name}.so")).display()
            ));
        }
        let path = managed_ini(&c, &name)?;
        if enable {
            write_atomic(&path, &decl_line(&name))?;
            // 写进去不等于真生效：zend / 普通声明搞错时 php 会静默忽略，这里验证一次
            let on = enabled_exts(&c.bin)
                .iter()
                .any(|e| e.eq_ignore_ascii_case(&name));
            if !on {
                let alt = if decl_line(&name).starts_with("zend_extension") {
                    format!("extension={name}.so\n")
                } else {
                    format!("zend_extension={name}.so\n")
                };
                write_atomic(&path, &alt)?;
                let on2 = enabled_exts(&c.bin)
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(&name));
                if !on2 {
                    let _ = std::fs::remove_file(&path);
                    return Err(format!(
                        "{name} 已写入 {} 但 php 未加载，可能缺少依赖库或 .so 与当前 PHP 版本不匹配",
                        path.display()
                    ));
                }
            }
            Ok(Response::ok(
                "ok",
                Some(json!({ "enabled": true, "ini": path.display().to_string(), "reload": reload(&c) })),
            ))
        } else {
            let mut removed = false;
            if path.is_file() {
                std::fs::remove_file(&path).map_err(|e| format!("删除 {} 失败: {e}", path.display()))?;
                removed = true;
            }
            // 主 php.ini 里的 extension= 行：注释掉（不删文件，避免误伤手写配置）
            let mut touched = false;
            if let Some(ini) = &c.ini
                && let Ok(content) = std::fs::read_to_string(ini)
            {
                let want = format!("{name}.so");
                let mut out = String::new();
                for line in content.lines() {
                    let t = line.trim_start();
                    if (t.starts_with("extension") || t.starts_with("zend_extension"))
                        && t.contains(&want)
                        && !t.starts_with(';')
                    {
                        out.push_str(&format!("; {line}\n"));
                        touched = true;
                    } else {
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                if touched {
                    write_atomic(ini, &out)?;
                }
            }
            if !removed && !touched {
                return Err(format!("{name} 未在任何配置文件中启用，无需禁用"));
            }
            Ok(Response::ok(
                "ok",
                Some(json!({ "enabled": false, "reload": reload(&c) })),
            ))
        }
    })
    .await
}

// ── 长任务：安装 / 卸载 ──────────────────────────────────────

/// 往日志追加一段（长任务的每一步都实时落盘，前端 WebSocket 才能流式看到）。
fn log_line(path: &str, text: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{text}");
    }
}

/// 跑一步 shell：输出实时追加到日志，返回退出码。
///
/// 退出码由 bash 自己 echo 出来（而非 Rust 侧判断），这样即便脚本里出现
/// `exit` 之外的失败路径也能拿到真实结果。
fn run_step(log: &str, title: &str, script: &str) -> i32 {
    log_line(log, &format!("── {title} ──"));
    let out = root_cmd(super::platform::SHELL)
        .args(["-c"])
        .arg(format!(
            "{{ {script}; }} >> {log} 2>&1; echo \"__ZAP_STEP__$?\""
        ))
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            s.rsplit("__ZAP_STEP__")
                .next()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(-1)
        }
        Err(_) => -1,
    }
}

/// 收尾：写完成标记 + 退出码文件（`.ret` 是权威来源，日志标记只是展示协议）。
fn finish_log(log: &str, code: i32) {
    log_line(log, &format!("{DONE_MARKER} {code}"));
    let ret = Path::new(log).with_extension("ret");
    let _ = std::fs::write(ret, code.to_string());
}

/// 安装命令脚本：按 PIE → pecl → 源码选路。
///
/// `pkg` 已校验；`php_config` 用于让 pie / configure 对准**本实例**的 php
/// （系统里可能有多个版本，装错版本等于白装）。
fn install_script(c: &PhpCtx, pkg: &str, version: &str, pie: bool, pecl: bool) -> String {
    let php_config = c
        .php_config
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    if pie {
        // PIE：非交互，`--with-php-config` 指向本实例
        // PIE 包名可为 `vendor/pkg` 或裸名（redis），都交给 pie 自己解析
        let target = pkg.to_string();
        let with_cfg = if php_config.is_empty() {
            String::new()
        } else {
            format!(" --with-php-config={php_config}")
        };
        return format!("pie install {target}{with_cfg}");
    }
    if pecl {
        // pecl 会交互式询问若干编译选项，用空回车把全部选项走默认值
        let ver = if version.is_empty() {
            String::new()
        } else {
            format!("-{version}")
        };
        return format!("printf '\\n\\n\\n\\n\\n' | pecl install {pkg}{ver}");
    }
    // 源码：pecl.php.net 的稳定版 tarball → phpize → configure → make
    let ver = if version.is_empty() {
        String::new()
    } else {
        format!("-{version}")
    };
    let with_cfg = if php_config.is_empty() {
        String::new()
    } else {
        format!(" --with-php-config={php_config}")
    };
    let phpize = php_inst_bin(c, "phpize");
    format!(
        "set -e; \
         work=$(mktemp -d); \
         curl -fsSL https://pecl.php.net/get/{pkg}{ver}.tgz -o \"$work/{pkg}.tgz\" \
           || wget -qO \"$work/{pkg}.tgz\" https://pecl.php.net/get/{pkg}{ver}.tgz; \
         tar xzf \"$work/{pkg}.tgz\" -C \"$work\"; \
         cd \"$work\"/{pkg}* ; \
         {phpize} >/dev/null; \
         ./configure{with_cfg}; \
         make -j$(nproc); \
         make install"
    )
}

/// 实例自带的构建工具（phpize / php-config），没有则回落到 PATH。
fn php_inst_bin(c: &PhpCtx, name: &str) -> String {
    c.bin
        .parent()
        .map(|d| d.join(name))
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| name.to_string())
}

/// `php_ext.install`：后台编译安装，日志写 `log_path`，立即返回（zapd 盯日志收尾）。
pub async fn install(svc: &str, package: &str, version: &str, log_path: &str) -> Response {
    let svc = svc.to_string();
    let package = package.to_string();
    let version = version.to_string();
    let log_path = log_path.to_string();
    if !name_ok(&package) {
        return Response::err(-1, format!("包名不合法: {package}"));
    }
    if !version.is_empty() && !name_ok(&version) {
        return Response::err(-1, format!("版本号不合法: {version}"));
    }
    // 先同步探明路径与选路（失败直接返回，不必起后台任务）
    let c = match ctx(&svc) {
        Ok(c) => c,
        Err(e) => {
            log_line(&log_path, &e);
            finish_log(&log_path, 1);
            return Response::ok("ok", Some(json!({ "started": false, "reason": e })));
        }
    };
    let (major, minor) = major_minor(&c.version);
    let pie = which("pie").is_some() && (major, minor) >= (PIE_MIN_MAJOR, PIE_MIN_MINOR);
    let pecl = which("pecl").is_some();
    let phpize_ok = c
        .bin
        .parent()
        .map(|d| d.join("phpize").is_file())
        .unwrap_or(false)
        || which("phpize").is_some();
    if !pie && !pecl && !phpize_ok {
        let e = "该实例没有 pie / pecl / phpize，无法编译安装扩展";
        log_line(&log_path, e);
        finish_log(&log_path, 1);
        return Response::ok("ok", Some(json!({ "started": false, "reason": e })));
    }
    log_line(
        &log_path,
        &format!(
            "安装 PHP 扩展 {}（PHP {}，方式：{}）",
            package,
            c.version,
            if pie { "PIE" } else if pecl { "pecl" } else { "源码编译" }
        ),
    );
    std::thread::spawn(move || {
        let code = install_inner(&c, &package, &version, &log_path, pie, pecl);
        finish_log(&log_path, code);
    });
    Response::ok("ok", Some(json!({ "started": true })))
}

/// 安装主体（后台线程）：编译 → 定位新 .so → 写 ini → 重载。
fn install_inner(
    c: &PhpCtx,
    package: &str,
    version: &str,
    log: &str,
    pie: bool,
    pecl: bool,
) -> i32 {
    // 短名：PIE 的 `vendor/pkg` 取末段（redis / imagick），pecl 与源码都用短名
    let short = package.rsplit('/').next().unwrap_or(package).to_string();
    let script = install_script(c, package, version, pie, pecl);
    if run_step(log, "编译安装", &script) != 0 {
        log_line(log, "安装失败：见上方输出");
        return 1;
    }
    // 装完的 .so 落在扩展目录（pecl / 源码都会 make install 到这里）
    let so = c.ext_dir.join(format!("{short}.so"));
    if !so.is_file() && which("php").is_some() {
        log_line(
            log,
            &format!("未能在 {} 找到 {short}.so，尝试按 php -m 复核", c.ext_dir.display()),
        );
    }
    // 写 ini 并重载（复用 toggle 的写入与验证逻辑）
    let path = match managed_ini(c, &short) {
        Ok(p) => p,
        Err(e) => {
            log_line(log, &e);
            return 1;
        }
    };
    if let Err(e) = write_atomic(&path, &decl_line(&short)) {
        log_line(log, &e);
        return 1;
    }
    // zend / 普通声明二选一：以 php -m 是否真的加载为准
    if !enabled_exts(&c.bin)
        .iter()
        .any(|e| e.eq_ignore_ascii_case(&short))
    {
        let alt = if decl_line(&short).starts_with("zend_extension") {
            format!("extension={short}.so\n")
        } else {
            format!("zend_extension={short}.so\n")
        };
        let _ = write_atomic(&path, &alt);
    }
    log_line(log, &format!("已写入 {}", path.display()));
    log_line(log, &reload(c));
    if enabled_exts(&c.bin)
        .iter()
        .any(|e| e.eq_ignore_ascii_case(&short))
    {
        log_line(log, &format!("{short} 已启用"));
        0
    } else {
        log_line(log, &format!("{short} 未出现在 php -m 中，请检查上方输出"));
        1
    }
}

/// `php_ext.remove`：后台卸载（先禁用，再去 .so；能用 pecl / pie 则交给它们）。
pub async fn remove(svc: &str, name: &str, log_path: &str) -> Response {
    let svc = svc.to_string();
    let name = name.to_string();
    let log_path = log_path.to_string();
    if !name_ok(&name) {
        return Response::err(-1, format!("扩展名不合法: {name}"));
    }
    let c = match ctx(&svc) {
        Ok(c) => c,
        Err(e) => {
            log_line(&log_path, &e);
            finish_log(&log_path, 1);
            return Response::ok("ok", Some(json!({ "started": false, "reason": e })));
        }
    };
    let installed = c.ext_dir.join(format!("{name}.so")).is_file()
        || enabled_exts(&c.bin)
            .iter()
            .any(|e| e.eq_ignore_ascii_case(&name));
    if !installed {
        let e = format!("{name} 未安装");
        log_line(&log_path, &e);
        finish_log(&log_path, 1);
        return Response::ok("ok", Some(json!({ "started": false, "reason": e })));
    }
    log_line(&log_path, &format!("卸载 PHP 扩展 {name}"));
    std::thread::spawn(move || {
        let code = remove_inner(&c, &name, &log_path);
        finish_log(&log_path, code);
    });
    Response::ok("ok", Some(json!({ "started": true })))
}

/// 安装 / 卸载的产物文件名与选路是纯逻辑，这里钉住行为（避免以后改脚本时改坏）。
#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_stub() -> PhpCtx {
        PhpCtx {
            bin: PathBuf::from("/usr/local/apps/php-74/bin/php"),
            ext_dir: PathBuf::from("/usr/local/apps/php-74/lib/php/extensions/no-debug-non-zts-20190902"),
            scan_dir: Some(PathBuf::from("/usr/local/apps/php-74/etc/php.d")),
            ini: Some(PathBuf::from("/usr/local/apps/php-74/etc/php.ini")),
            php_config: Some(PathBuf::from("/usr/local/apps/php-74/bin/php-config")),
            version: "7.4.33".to_string(),
            unit: Some("php-fpm-74".to_string()),
        }
    }

    #[test]
    fn ext_name_whitelist() {
        assert!(name_ok("redis"));
        assert!(name_ok("ext-redis/redis"));
        assert!(name_ok("swoole-5.1.1"));
        // 目录穿越与 shell 元字符一律拒绝
        assert!(!name_ok("../redis"));
        assert!(!name_ok("redis;rm -rf /"));
        assert!(!name_ok("$(id)"));
        assert!(!name_ok(""));
    }

    #[test]
    fn version_parsing() {
        assert_eq!(major_minor("7.4.33"), (7, 4));
        assert_eq!(major_minor("8.1.0"), (8, 1));
        assert_eq!(major_minor(""), (0, 0));
    }

    #[test]
    fn zend_extensions_declared_as_zend() {
        assert_eq!(decl_line("opcache"), "zend_extension=opcache.so\n");
        assert_eq!(decl_line("xdebug"), "zend_extension=xdebug.so\n");
        assert_eq!(decl_line("redis"), "extension=redis.so\n");
    }

    #[test]
    fn managed_ini_is_namespaced() {
        let c = ctx_stub();
        let p = managed_ini(&c, "redis").unwrap();
        assert_eq!(p.file_name().unwrap(), "zap-ext-redis.ini");
    }

    #[test]
    fn installer_route_pie_pecl_source() {
        let c = ctx_stub();
        // PIE：非交互，且带本实例的 php-config（系统里可能有多版本 PHP）
        let s = install_script(&c, "redis", "", true, false);
        assert!(s.starts_with("pie install redis"), "{s}");
        assert!(s.contains("--with-php-config=/usr/local/apps/php-74/bin/php-config"), "{s}");
        // pecl：交互选项用空回车走默认值；版本拼在包名后
        let s = install_script(&c, "redis", "5.3.7", false, true);
        assert!(s.contains("pecl install redis-5.3.7"), "{s}");
        assert!(s.contains("printf"), "{s}");
        // 源码兜底：pecl.php.net 的 tarball + phpize + make
        let s = install_script(&c, "redis", "", false, false);
        assert!(s.contains("pecl.php.net/get/redis.tgz"), "{s}");
        assert!(s.contains("phpize"), "{s}");
        assert!(s.contains("--with-php-config=/usr/local/apps/php-74/bin/php-config"), "{s}");
    }

    #[test]
    fn pie_needs_php_81() {
        // 7.4 实例即使装了 pie.phar 也不能用（PIE 自身要求 8.1+）
        assert!((7, 4) < (PIE_MIN_MAJOR, PIE_MIN_MINOR));
        assert!((8, 1) >= (PIE_MIN_MAJOR, PIE_MIN_MINOR));
        assert!((8, 3) >= (PIE_MIN_MAJOR, PIE_MIN_MINOR));
    }
}

/// 卸载主体：删 ini（含注释主 php.ini 的行）→ pecl uninstall → 删 .so → 重载。
fn remove_inner(c: &PhpCtx, name: &str, log: &str) -> i32 {
    if let Ok(p) = managed_ini(c, name)
        && p.is_file()
    {
        let _ = std::fs::remove_file(&p);
        log_line(log, &format!("已删除 {}", p.display()));
    }
    if let Some(ini) = &c.ini
        && let Ok(content) = std::fs::read_to_string(ini)
    {
        let want = format!("{name}.so");
        let mut touched = false;
        let mut out = String::new();
        for line in content.lines() {
            let t = line.trim_start();
            if (t.starts_with("extension") || t.starts_with("zend_extension"))
                && t.contains(&want)
                && !t.starts_with(';')
            {
                out.push_str(&format!("; {line}\n"));
                touched = true;
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        if touched && write_atomic(ini, &out).is_ok() {
            log_line(log, &format!("已注释 {ini} 中的 {name} 声明", ini = ini.display()));
        }
    }
    if which("pecl").is_some() {
        let _ = run_step(log, "pecl uninstall", &format!("pecl uninstall {name}"));
    }
    let so = c.ext_dir.join(format!("{name}.so"));
    if so.is_file() {
        if std::fs::remove_file(&so).is_ok() {
            log_line(log, &format!("已删除 {}", so.display()));
        } else {
            log_line(log, &format!("删除 {} 失败", so.display()));
        }
    }
    log_line(log, &reload(c));
    if enabled_exts(&c.bin)
        .iter()
        .any(|e| e.eq_ignore_ascii_case(name))
    {
        log_line(log, &format!("{name} 仍在 php -m 中，请检查上方输出"));
        1
    } else {
        log_line(log, &format!("{name} 已卸载"));
        0
    }
}
