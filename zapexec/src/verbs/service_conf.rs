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

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use zap_proto::Response;

use super::root_cmd;

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfFormat {
    /// ini 风格：key = value（php.ini 无 section / my.cnf 有 [section]）
    Ini,
    /// JSON 对象（docker daemon.json）
    Json,
}

#[derive(Clone, Copy)]
enum FieldKind {
    Text,
    Number,
    Select,
    Bool,
    /// 多行列表：配置里是数组（如 docker 的 `registry-mirrors`），表单里一行一项
    List,
}

struct FieldDef {
    key: &'static str,
    label: &'static str,
    kind: FieldKind,
    help: &'static str,
    /// ini：写入的 section（None = 文件层/无 section）
    section: Option<&'static str>,
    /// json：逐级键路径（如 ["log-driver"] / ["log-opts","max-size"]）
    jpath: &'static [&'static str],
    options: &'static [&'static str],
}

struct ServiceDef {
    key: &'static str,
    label: &'static str,
    /// systemd unit 名候选（依次取第一个存在的）
    unit_candidates: &'static [&'static str],
    /// 探测二进制的可执行文件名候选
    bin_candidates: &'static [&'static str],
    /// 取版本时二进制后追加的参数（留空 = `--version`）
    version_args: &'static [&'static str],
    /// 版本输出落在 stderr（如 nginx -v；本模块服务多为 stdout）
    version_in_stderr: bool,
    /// 主配置文件候选（可含一个 `*`，用于版本目录），依次取第一个存在的；
    /// 全部不存在但服务已安装时取首个不含通配的候选（允许从 UI 新建）
    main_candidates: &'static [&'static str],
    /// 可编辑文件的扩展名白名单（不含点）
    exts: &'static [&'static str],
    format: ConfFormat,
    /// ini 注释符（my.cnf 用 #，php.ini 用 ;）
    ini_comment: &'static str,
    fields: &'static [FieldDef],
}

const PHP_FIELDS: &[FieldDef] = &[
    FieldDef {
        key: "memory_limit",
        label: "memory_limit",
        kind: FieldKind::Text,
        help: "单个 PHP 进程可用内存上限，如 128M / 256M / 512M",
        section: None,
        jpath: &[],
        options: &[],
    },
    FieldDef {
        key: "upload_max_filesize",
        label: "upload_max_filesize",
        kind: FieldKind::Text,
        help: "上传文件大小上限，如 20M / 50M（需同时放大 post_max_size）",
        section: None,
        jpath: &[],
        options: &[],
    },
    FieldDef {
        key: "post_max_size",
        label: "post_max_size",
        kind: FieldKind::Text,
        help: "POST 数据大小上限，建议略大于 upload_max_filesize",
        section: None,
        jpath: &[],
        options: &[],
    },
    FieldDef {
        key: "max_execution_time",
        label: "max_execution_time",
        kind: FieldKind::Number,
        help: "单个脚本最大执行时间（秒），CLI 默认不受限",
        section: None,
        jpath: &[],
        options: &[],
    },
    FieldDef {
        key: "max_input_time",
        label: "max_input_time",
        kind: FieldKind::Number,
        help: "解析输入数据的最长时间（秒）",
        section: None,
        jpath: &[],
        options: &[],
    },
    FieldDef {
        key: "date.timezone",
        label: "date.timezone",
        kind: FieldKind::Text,
        help: "时区，如 Asia/Shanghai",
        section: None,
        jpath: &[],
        options: &[],
    },
    FieldDef {
        key: "display_errors",
        label: "display_errors",
        kind: FieldKind::Select,
        help: "是否把错误输出到页面（生产环境建议 Off）",
        section: None,
        jpath: &[],
        options: &["Off", "On"],
    },
    FieldDef {
        key: "opcache.enable",
        label: "opcache.enable",
        kind: FieldKind::Select,
        help: "是否启用 opcache（PHP >= 5.5 内置）",
        section: None,
        jpath: &[],
        options: &["1", "0"],
    },
];

const MYSQL_FIELDS: &[FieldDef] = &[
    FieldDef {
        key: "port",
        label: "port",
        kind: FieldKind::Number,
        help: "监听端口，默认 3306",
        section: Some("mysqld"),
        jpath: &[],
        options: &[],
    },
    FieldDef {
        key: "bind-address",
        label: "bind-address",
        kind: FieldKind::Text,
        help: "监听地址：127.0.0.1 仅本机；0.0.0.0 对外（请配合防火墙）",
        section: Some("mysqld"),
        jpath: &[],
        options: &[],
    },
    FieldDef {
        key: "max_connections",
        label: "max_connections",
        kind: FieldKind::Number,
        help: "最大并发连接数，建议 200-2000",
        section: Some("mysqld"),
        jpath: &[],
        options: &[],
    },
    FieldDef {
        key: "character-set-server",
        label: "character-set-server",
        kind: FieldKind::Select,
        help: "默认字符集",
        section: Some("mysqld"),
        jpath: &[],
        options: &["utf8mb4", "utf8", "latin1"],
    },
    FieldDef {
        key: "collation-server",
        label: "collation-server",
        kind: FieldKind::Text,
        help: "默认排序规则，utf8mb4 建议 utf8mb4_unicode_ci",
        section: Some("mysqld"),
        jpath: &[],
        options: &[],
    },
    FieldDef {
        key: "innodb_buffer_pool_size",
        label: "innodb_buffer_pool_size",
        kind: FieldKind::Text,
        help: "InnoDB 缓冲池大小，约为内存的 50%-70%，如 1G / 4G",
        section: Some("mysqld"),
        jpath: &[],
        options: &[],
    },
];

const DOCKER_FIELDS: &[FieldDef] = &[
    FieldDef {
        key: "registry_mirrors",
        label: "镜像加速器",
        kind: FieldKind::List,
        help: "清空保存即删除该配置。保存后需重启 Docker 生效",
        section: None,
        jpath: &["registry-mirrors"],
        options: &[],
    },
    FieldDef {
        key: "insecure_registries",
        label: "私有仓库",
        kind: FieldKind::List,
        help: "每行一个不走 HTTPS / 自签证书的仓库地址（insecure-registries），如 192.168.1.10:5000",
        section: None,
        jpath: &["insecure-registries"],
        options: &[],
    },
    FieldDef {
        key: "log_driver",
        label: "日志驱动",
        kind: FieldKind::Select,
        help: "容器日志驱动（logging driver）",
        section: None,
        jpath: &["log-driver"],
        options: &["json-file", "local", "journald", "none", "syslog"],
    },
    FieldDef {
        key: "log_max_size",
        label: "单容器日志上限",
        kind: FieldKind::Text,
        help: "json-file/local 驱动的单文件大小上限，如 20m / 100m",
        section: None,
        jpath: &["log-opts", "max-size"],
        options: &[],
    },
    FieldDef {
        key: "log_max_file",
        label: "日志保留份数",
        kind: FieldKind::Number,
        help: "单个容器保留的日志文件个数（max-file），与上限配合做轮转，如 3",
        section: None,
        jpath: &["log-opts", "max-file"],
        options: &[],
    },
    FieldDef {
        key: "storage_driver",
        label: "存储驱动",
        kind: FieldKind::Select,
        help: "存储驱动，一般保持 overlay2 即可（改动后已有镜像/容器不可见）",
        section: None,
        jpath: &["storage-driver"],
        options: &["overlay2", "overlay", "btrfs", "zfs", "vfs"],
    },
    FieldDef {
        key: "data_root",
        label: "数据目录",
        kind: FieldKind::Text,
        help: "Docker 数据根目录（默认 /var/lib/docker），修改需迁移数据",
        section: None,
        jpath: &["data-root"],
        options: &[],
    },
    FieldDef {
        key: "dns",
        label: "容器 DNS",
        kind: FieldKind::List,
        help: "每行一个 DNS 服务器地址，仅对新创建的容器生效，如 223.5.5.5 / 8.8.8.8",
        section: None,
        jpath: &["dns"],
        options: &[],
    },
    FieldDef {
        key: "exec_opts",
        label: "运行时参数",
        kind: FieldKind::List,
        help: "每行一条 dockerd 运行参数（exec-opts），如 native.cgroupdriver=systemd",
        section: None,
        jpath: &["exec-opts"],
        options: &[],
    },
    FieldDef {
        key: "live_restore",
        label: "守护进程热升级",
        kind: FieldKind::Bool,
        help: "dockerd 重启或升级时保持容器继续运行（live-restore），未设置则跟随 Docker 默认",
        section: None,
        jpath: &["live-restore"],
        options: &["true", "false"],
    },
    FieldDef {
        key: "userland_proxy",
        label: "用户态代理",
        kind: FieldKind::Bool,
        help: "端口映射是否走 docker-proxy，高并发场景可关闭以减少开销",
        section: None,
        jpath: &["userland-proxy"],
        options: &["true", "false"],
    },
    FieldDef {
        key: "debug",
        label: "调试日志",
        kind: FieldKind::Bool,
        help: "是否开启 dockerd 调试日志",
        section: None,
        jpath: &["debug"],
        options: &["true", "false"],
    },
    FieldDef {
        key: "icc",
        label: "容器间互联 icc",
        kind: FieldKind::Bool,
        help: "默认 bridge 网络上容器是否可互相通信",
        section: None,
        jpath: &["icc"],
        options: &["true", "false"],
    },
];

/// PHP 类型服务的静态定义（type 级 "php"，面向系统包安装）。
const PHP_DEF: ServiceDef = ServiceDef {
    key: "php",
    label: "PHP",
    unit_candidates: &["php-fpm"],
    bin_candidates: &["php-fpm", "php"],
    version_args: &["-v"],
    version_in_stderr: false,
    // 覆盖常见布局：Debian/Ubuntu（/etc/php/<ver>/fpm/php.ini）、
    // Remi 源（/etc/opt/remi/php<ver>/php.ini）、源码安装（/usr/local/etc/php）、
    // RHEL/CentOS 系统包（/etc/php.ini）。实例安装（php-85 目录）由 php_inst 单独定位。
    main_candidates: &[
        "/etc/php/*/fpm/php.ini",
        "/etc/php/*/cli/php.ini",
        "/etc/opt/remi/*/php.ini",
        "/etc/php.ini",
        "/usr/local/etc/php/php.ini",
    ],
    exts: &["ini"],
    format: ConfFormat::Ini,
    ini_comment: ";",
    fields: PHP_FIELDS,
};

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

/// 服务注册表：新增服务在此追加即可（key 需与前端菜单/API 一致）。
/// php<版本号>（php74 / php81 …）作为 PHP 实例 svc 一并识别，复用 PHP_DEF
/// 的字段 / 白名单 / 关键项，探测与路径在 `php_inst` / `installed_info` 覆写。
fn supported(key: &str) -> Option<&'static ServiceDef> {
    if php_inst_svc(key).is_some() {
        return Some(&PHP_DEF);
    }
    Some(match key {
        "php" => &PHP_DEF,
        // MySQL / MariaDB 合一（svc=mysql）：应用商店同时只允许安装其中一个
        // （mysql 或 mariadb 应用），两者都软链 /usr/local/mysql、共用
        // /etc/mysql/my.cnf 与 mysql.service unit；引擎由 status 按登记/版本自动识别。
        "mysql" => &ServiceDef {
            key: "mysql",
            label: "MySQL / MariaDB",
            unit_candidates: &["mysql", "mysqld"],
            bin_candidates: &["mysqld", "mysql"],
            version_args: &["--version"],
            version_in_stderr: false,
            main_candidates: &["/etc/mysql/my.cnf", "/etc/my.cnf"],
            exts: &["cnf", "conf"],
            format: ConfFormat::Ini,
            ini_comment: "#",
            fields: MYSQL_FIELDS,
        },
        "docker" => &ServiceDef {
            key: "docker",
            label: "Docker",
            unit_candidates: &["docker"],
            bin_candidates: &["dockerd", "docker"],
            version_args: &["--version"],
            version_in_stderr: false,
            main_candidates: &["/etc/docker/daemon.json"],
            exts: &["json"],
            format: ConfFormat::Json,
            ini_comment: "#",
            fields: DOCKER_FIELDS,
        },
        _ => return None,
    })
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
    let o = root_cmd("bash")
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
    for name in d.bin_candidates {
        if let Some(p) = which(name) {
            return Some(p);
        }
    }
    // 安装根（默认 /usr/local/apps，ZAP_APPS_DIR 可覆盖）下常见位置
    let install_root = super::install_root();
    for name in d.bin_candidates {
        for prefix in [
            install_root.clone(),
            install_root.join(d.key),
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
    for a in d.version_args {
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
    for cand in d.main_candidates {
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

/// 支持单个 `*` 的极简 glob：`*` 匹配**一层目录名**。
///
/// - `/etc/php/*/fpm/php.ini` → 在 `/etc/php` 下遍历版本目录（7.4 / 8.1 / 8.2 …），
///   取 `<版本目录>/fpm/php.ini`；多个命中时取**版本号最大**的一个。
/// - 兼容 `*` 前带目录名前缀的写法（如 `/etc/php-*/x.ini`）：前缀参与过滤。
///
/// 注意：`*` 前必须以 `/` 结尾才是"遍历该目录下的子目录"，否则最后一个路径段
/// 视为目录名前缀（旧实现把 `/etc/php/` 的最后一段当成前缀、去扫 `/etc`，
/// 导致 Debian/Ubuntu 的 `/etc/php/8.1/fpm/php.ini` 永远探测不到）。
fn glob_first(pattern: &str) -> Option<PathBuf> {
    let (head, tail) = pattern.split_once('*')?;
    let tail = tail.trim_start_matches('/');
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

    let rd = std::fs::read_dir(&dir).ok()?;
    let mut hits: Vec<(PathBuf, String)> = Vec::new();
    for entry in rd.flatten() {
        let sub = entry.path();
        if !sub.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) {
            continue;
        }
        let target = sub.join(tail);
        if target.is_file() {
            hits.push((target, name));
        }
    }
    // 版本号最大的优先；无法解析为数字的目录名退化为字典序
    hits.sort_by(|a, b| version_key(&a.1).cmp(&version_key(&b.1)));
    hits.pop().map(|(p, _)| p)
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
fn service_exts(d: &ServiceDef, root: &Path) -> &'static [&'static str] {
    if d.key != "php" {
        return d.exts;
    }
    let p = root.to_string_lossy().to_string();
    if SYSTEM_CONF_DIRS.iter().any(|x| p == *x) {
        &["ini"]
    } else {
        &["ini", "conf"]
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
struct PhpInst {
    digits: String,
    dir: PathBuf,
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
fn php_inst(svc: &str) -> Option<PhpInst> {
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
    d.unit_candidates
        .iter()
        .find(|u| super::svc::exists(u))
        .map(|u| u.to_string())
}

/// 实际使用的 systemd unit 名（无 .service 后缀）。
/// PHP 实例 svc 使用实例 unit `php-fpm-<ver>`；其余走候选探测。
fn effective_unit(d: &ServiceDef, svc: &str) -> Option<String> {
    if let Some(inst) = php_inst(svc) {
        // 优先 info.yaml 登记的 svc_name，兜底 php-fpm-<版本>
        if let Some(u) = php_reg_of(svc).and_then(|r| r.unit)
            && super::svc::exists(&u)
        {
            return Some(u);
        }
        let u = format!("php-fpm-{}", inst.digits);
        return super::svc::exists(&u).then_some(u);
    }
    active_unit(d)
}

/// 服务当前运行态。
fn service_running(d: &ServiceDef, svc: &str, bin: Option<&Path>) -> bool {
    if let Some(unit) = effective_unit(d, svc) {
        if super::svc::is_active(&unit) {
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
    let is_main = canon == main_canon;
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
        let v = in_block_values.get(f.key).or_else(|| fallback.get(f.key));
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
        for seg in f.jpath {
            match cur.get(*seg) {
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

async fn run_blocking<F>(f: F) -> Response
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
        let (bin, unit, main) = installed_info(d, &svc);
        if bin.is_none() && unit.is_none() {
            return Ok(Response::ok(
                "ok",
                Some(json!({ "installed": false, "service": svc })),
            ));
        }
        let version = bin
            .as_deref()
            .map(|b| detect_version(d, b))
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
        let running = service_running(d, &svc, bin.as_deref());
        let bin_path = bin.as_deref().map(|b| b.display().to_string());
        // 探测失败时把尝试过的候选路径一起回传，便于定位"没检测到配置文件"的原因
        // （实例安装走固定路径，静态候选不适用，故仅在非实例时返回）
        let conf_candidates: Option<Vec<&str>> = if php_inst(&svc).is_some() {
            None
        } else {
            Some(d.main_candidates.to_vec())
        };
        Ok(Response::ok(
            "ok",
            Some(json!({
                "installed": true,
                "service": svc,
                "label": d.label,
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
        let (bin, unit, main) = installed_info(d, &svc);
        if bin.is_none() && unit.is_none() {
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
        let scanned = if root.is_dir() {
            let mut scanned: Vec<Value> = Vec::new();
            collect_files(service_exts(d, &root), &root, &mut scanned);
            scanned
        } else {
            Vec::new()
        };
        let files = merge_main_and_scanned(&main, &root, main_exists, scanned);
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
        let (_, _, Some((main, root, _))) = installed_info(d, &svc) else {
            return Err(format!("{} 未安装或未探测到配置，请先在应用商店安装", d.label));
        };
        let (canon, is_main) = validate_path(d, service_exts(d, &root), &main, &root, &path)?;
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
        let (_, _, Some((main, root, _))) = installed_info(d, &svc) else {
            return Err(format!("{} 未安装或未探测到配置", d.label));
        };
        let (canon, is_main) = validate_path(d, service_exts(d, &root), &main, &root, &path)?;
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

/// service_conf.keys：关键项表单定义 + 当前值。
pub async fn keys_get(svc: &str) -> Response {
    let svc = svc.to_string();
    run_blocking(move || {
        let Some(d) = supported(&svc) else {
            return Err(format!("不支持的服务类型: {svc}"));
        };
        let (bin, unit, main) = installed_info(d, &svc);
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
                    ConfFormat::Ini => ini_read_values(&content, d.ini_comment, &svc, d.fields),
                    ConfFormat::Json => json_read_values(&content, d.fields),
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
        let (_, _, Some((main, _, _))) = installed_info(d, &svc) else {
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
                let cleaned = strip_managed_block(&old, d.ini_comment, &svc);
                // 托管块内部为行式 key = value
                let entries: Vec<(&str, String)> = d
                    .fields
                    .iter()
                    .filter_map(|f| {
                        let v = keys.get(f.key)?.trim().to_string();
                        if v.is_empty() {
                            return None;
                        }
                        Some((f.key, v))
                    })
                    .collect();
                if entries.is_empty() {
                    cleaned
                } else {
                    let block = block_lines(d.ini_comment, &svc, &entries);
                    insert_block(&cleaned, d.fields.first().and_then(|f| f.section), &block)
                }
            }
            ConfFormat::Json => {
                let mut obj: Value = if old.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&old).unwrap_or_else(|_| json!({}))
                };
                for f in d.fields {
                    if let Some(raw) = keys.get(f.key) {
                        let raw = raw.trim();
                        if raw.is_empty() {
                            // 表单每次都提交全部字段，所以列表留空是用户主动清空
                            // （registry-mirrors 之类必须真删掉，留个空数组 dockerd 一样不认）；
                            // 其余类型留空 = 保持原文件中的既有键不动（不写入托管值）
                            if matches!(f.kind, FieldKind::List) {
                                remove_json_path(&mut obj, f.jpath);
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
                        set_json_path(&mut obj, f.jpath, v);
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
        let bin = find_bin(d);
        // 优先已注册/存在的 unit（实例 svc 走 php-fpm-<ver>）；其次用二进制的 systemd 名做一次尝试
        let unit = effective_unit(d, &svc).or_else(|| {
            bin.as_ref()
                .and_then(|b| b.file_stem())
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        });
        if unit.is_none() {
            return Err(format!("{} 未安装或未检测到对应服务", d.label));
        }
        let unit = unit.unwrap();
        let method = if super::svc::exists(&unit) {
            super::svc::act(&action, &unit).map_err(|e| format!("{action} {unit} 失败：{e}"))?;
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
        let running = service_running(d, &svc, bin.as_deref());
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
    let (bin, unit, main) = installed_info(d, svc);
    let installed = bin.is_some() || unit.is_some();
    let (conf_file, main_exists) = match &main {
        Some((m, _, exists)) => (Some(m.display().to_string()), *exists),
        None => (None, false),
    };
    let version = if installed {
        bin.as_deref()
            .map(|b| detect_version(d, b))
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
        "running": installed && service_running(d, svc, bin.as_deref()),
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

    #[test]
    fn php_def_covers_common_layouts() {
        assert!(PHP_DEF.main_candidates.contains(&"/etc/php/*/fpm/php.ini"));
        assert!(PHP_DEF.main_candidates.contains(&"/etc/php.ini"));
        assert!(PHP_DEF.main_candidates.contains(&"/etc/opt/remi/*/php.ini"));
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
