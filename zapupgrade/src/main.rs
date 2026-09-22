//! ZAP 系统升级器（一次性进程，以 root 运行）。
//!
//! 两种用法：
//!
//! 1. **内部模式**（面板触发）：由 zapexec 用 `systemd-run --no-block` 拉入
//!    独立 transient unit 执行：`zapupgrade --stage <dir> --dir <安装根> --log <文件>`。
//!    本进程**不随 zapd / zapexec 的重启而终止**，因此可以安全地：
//!    校验升级包 → 备份当前二进制 → 原子替换 → 依次 `systemctl restart`
//!    zapexec / zapd，最后在日志中写入 `__ZAP_DONE__ <code>` 结束标记
//!    （zapd 侧轮询该标记以收尾运行记录）。
//! 2. **命令行模式**：`zapupgrade upgrade --to <版本|latest>` 自行下载发行包、
//!    sha256 校验、解包后复用上面同一套替换流程；`zapupgrade rollback`
//!    把历史备份恢复回去。面板不可用 / 服务起不来时的自救入口。

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use sha2::Digest;

/// 随发行包分发的二进制清单（stage 内存在者即视为需要更新）。
const BINS: [&str; 4] = ["zapd", "zapexec", "zapctl", "zapupgrade"];

/// 与 zapd `zap::appstore::DONE_MARKER` 保持一致的日志结束标记。
const DONE_MARKER: &str = "__ZAP_DONE__";

/// 建目录，并把属主对齐到数据区 `{dir}/data` 的属主。
///
/// 升级数据区会被两种身份写入：本程序（root）与 zapd（zapadm）。
/// 谁先建目录谁就是属主，另一方立刻 `EACCES 13` —— root 抢先建了 `stage/`，
/// zapd 就再也写不进去，升级必然失败。所以 root 建完要把属主还回去。
fn ensure_dir(path: &Path, dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    align_owner(path, &dir.join("data"));
    Ok(())
}

/// 仅 root 生效：把 `path` 的 uid/gid 设成 `data_dir` 的 uid/gid。
/// 非 root 做不了 chown，也没必要 —— zapd 以 zapadm 运行，建的目录本就归自己。
fn align_owner(path: &Path, data_dir: &Path) {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = fs::metadata(data_dir) else {
        return;
    };
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return;
    };
    unsafe {
        libc::chown(c.as_ptr(), meta.uid(), meta.gid());
    }
}

/// 发行线标记文件：install.sh 写入，本工具升级成功后回写（见 persist_edition）。
const EDITION_FILE: &str = "/etc/zap/edition";

/// 默认更新渠道（与 zapd `zap::update_config::DEFAULT_CHANNEL` 保持一致）。
const DEFAULT_CHANNEL: &str = "https://mirrors.zap.cn/zap/releases";

/// 日志是否同时打到 stdout。
///
/// 命令行模式打开（人要看进度），内部模式关闭（systemd-run 的 stdout 进 journal，
/// 与面板轮询的日志文件重复，没必要）。
static ECHO: AtomicBool = AtomicBool::new(false);

#[derive(Parser, Debug)]
#[command(
    name = "zapupgrade",
    about = "ZAP 系统升级器：升级 / 回滚 zapd、zapexec、zapctl、zapupgrade",
    version,
    arg_required_else_help = true
)]
struct Cli {
    /// 已下载解包的 stage 目录（内部模式，由 zapexec 传入；命令行模式自动准备）
    #[arg(long, global = true)]
    stage: Option<String>,
    /// ZAP 安装根目录（生产为 /usr/local/zap）
    #[arg(long, global = true, default_value = "/usr/local/zap")]
    dir: String,
    /// 升级日志文件（追加写；命令行模式缺省时自动放到 data/upgrade/logs/）
    #[arg(long, global = true)]
    log: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 下载并安装指定版本（缺省升级到最新版）
    Upgrade {
        /// 目标版本：`latest`、`v1.2.3` 或 `1.2.3`
        #[arg(long, default_value = "latest")]
        to: String,
        /// 更新渠道（镜像地址，与面板「系统更新」中的渠道一致）
        #[arg(long, default_value = DEFAULT_CHANNEL)]
        channel: String,
        /// 目标版本不高于当前版本时仍然安装
        #[arg(long, action)]
        force: bool,
        /// 升级到商业版 Zap Pro（包名带 -pro）；缺省时按 /etc/zap/edition 判断
        #[arg(long, action)]
        pro: bool,
        /// 回退到社区版（包名无后缀）；与 --pro 互斥，缺省时按 /etc/zap/edition 判断
        #[arg(long, action, conflicts_with = "pro")]
        community: bool,
    },
    /// 回滚到历史备份（升级时自动备份到 data/upgrade/backup/）
    Rollback {
        /// 只列出可回滚的备份，不做恢复
        #[arg(long, action)]
        list: bool,
        /// 指定备份名（data/upgrade/backup 下的目录名），缺省回滚最新一个
        #[arg(long)]
        to: Option<String>,
    },
}

/// 一次「替换 + 重启」执行所需的三件套（内部模式与命令行模式共用）。
struct RunCtx {
    stage: String,
    dir: String,
    log: String,
}

fn log_line(log_path: &str, s: &str) {
    if ECHO.load(Ordering::SeqCst) {
        println!("{s}");
    }
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(f, "{s}");
    }
}

/// 服务管理器：Linux 是 systemd。
const SVC_CTL: &str = "systemctl";

/// 当前平台的自动服务管理是否受支持（Linux + systemd）。
/// macOS 用 launchctl，不在范围内。
fn supported() -> bool {
    cfg!(target_os = "linux")
}

/// 服务名：`<name>.service`。
fn service_unit(svc: &str) -> String {
    format!("{svc}.service")
}

/// 重启服务的命令参数。
fn restart_args(unit: &str) -> [&str; 2] {
    ["restart", unit]
}

fn svc_cmd(args: &[&str]) -> bool {
    std::process::Command::new(SVC_CTL)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 目标服务是否已注册到本机服务管理器？
/// 开发/容器环境（rundev.sh 裸进程、docker）没有注册服务，返回 false；
/// 平台本身不支持自动服务管理时同样返回 false，上层据此提示「手动重启」。
fn unit_managed(unit: &str) -> bool {
    if !supported() {
        return false;
    }
    // 没有 /run/systemd/system 就不是 systemd 在管（容器里常见）
    if !std::path::Path::new("/run/systemd/system").exists() {
        return false;
    }
    // 判据用 LoadState 而不是 Id：单元不存在时 `systemctl show` 照样返回 0，
    // 而且某些版本会把请求的名字原样回显成 `Id=<unit>`（看着像已注册）。
    // 判错就会走到 restart → 必失败 → 回滚，表现是「升级成功、二进制又变回旧的」。
    // 单元不存在时 LoadState=not-found，其余（loaded 等）才算注册过。
    std::process::Command::new(SVC_CTL)
        .args(["show", unit, "--property=LoadState", "--no-pager"])
        .output()
        .map(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|l| l.trim() == "LoadState=loaded")
        })
        .unwrap_or(false)
}

/// 从 argv 中取出 `--log` 的值（供参数解析失败时兜底写日志）。
fn log_from_argv() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.windows(2)
        .find(|w| w[0] == "--log")
        .map(|w| w[1].clone())
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            // --help / --version 是 clap 的正常输出，不能当成解析失败写进升级日志
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                let _ = e.print();
                std::process::exit(0);
            }
            // 参数解析失败也必须落到升级日志并写入结束标记：
            // 否则面板只能看到空日志，且运行记录一直停留在「运行中」。
            let msg = format!("升级器参数解析失败: {e}");
            if let Some(log) = log_from_argv() {
                log_line(&log, &msg);
                log_line(&log, &format!("{DONE_MARKER} 1"));
            }
            eprintln!("{msg}");
            std::process::exit(1);
        }
    };

    let dir = PathBuf::from(&cli.dir);
    // 只给了 --stage / --dir / --log 就是内部模式（zapexec 拉起），否则是人在敲命令
    let internal = cli.stage.is_some();
    ECHO.store(!internal, Ordering::SeqCst);
    let log = cli.log.clone().unwrap_or_else(|| default_log_path(&dir));

    let code = match (&cli.stage, &cli.command) {
        (Some(stage), _) => {
            log_line(&log, "==== ZAP 系统升级开始 ====");
            let code = run_upgrade(&RunCtx {
                stage: stage.clone(),
                dir: cli.dir.clone(),
                log: log.clone(),
            });
            // 结束标记只有内部模式需要：zapd 靠它收尾面板里的运行记录，
            // 命令行模式写了反而会混进面板的日志目录。
            log_line(&log, &format!("{DONE_MARKER} {code}"));
            code
        }
        (
            None,
            Some(Command::Upgrade {
                to,
                channel,
                force,
                pro,
                community,
            }),
        ) => cmd_upgrade(to, channel, *force, *pro, *community, &dir, &log),
        (None, Some(Command::Rollback { list, to })) => cmd_rollback(*list, to, &dir, &log),
        (None, None) => {
            eprintln!(
                "缺少参数：内部模式需 --stage，命令行模式需 upgrade / rollback 子命令（--help）"
            );
            1
        }
    };
    std::process::exit(code);
}

/// 命令行模式的默认日志：`{dir}/data/upgrade/logs/run-cli-{ts}.log`。
fn default_log_path(dir: &Path) -> String {
    let p = dir
        .join("data/upgrade/logs")
        .join(format!("run-cli-{}.log", now_secs()));
    if let Some(parent) = p.parent() {
        let _ = ensure_dir(parent, dir);
    }
    p.to_string_lossy().into_owned()
}

fn run_upgrade(ctx: &RunCtx) -> i32 {
    let dir = PathBuf::from(&ctx.dir);
    let stage = PathBuf::from(&ctx.stage);

    // 1) 目标版本（zapd 下载时写入 stage/version）
    let version = fs::read_to_string(stage.join("version"))
        .unwrap_or_default()
        .trim()
        .to_string();
    log_line(&ctx.log, &format!("目标版本: {version}"));

    // 2) 待更新二进制 = stage 中存在的清单项
    let mut targets: Vec<&str> = Vec::new();
    for b in BINS {
        if stage.join(b).is_file() {
            targets.push(b);
        }
    }
    if targets.is_empty() {
        log_line(&ctx.log, "升级包内没有可更新的二进制，中止");
        return 1;
    }
    log_line(&ctx.log, &format!("待更新二进制: {}", targets.join(", ")));

    // 3) 备份当前二进制到 data/upgrade/backup/{ts}-{version}/
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_dir = dir
        .join("data/upgrade/backup")
        .join(format!("{ts}-{version}"));
    if ensure_dir(&backup_dir, &dir).is_err() {
        log_line(&ctx.log, "创建备份目录失败");
        return 1;
    }
    let mut backed_up: Vec<&str> = Vec::new();
    for b in &targets {
        let cur = dir.join(b);
        if cur.exists() {
            match fs::copy(&cur, backup_dir.join(b)) {
                Ok(_) => {
                    backed_up.push(b);
                    log_line(&ctx.log, &format!("已备份 {b}"));
                }
                Err(e) => {
                    log_line(&ctx.log, &format!("备份 {b} 失败: {e}，中止"));
                    return 1;
                }
            }
        }
    }

    // 4) 原子替换（先写临时文件再 rename，避免半写状态）
    for b in &targets {
        let tmp = dir.join(format!(".{b}.upgrade-new"));
        let dst = dir.join(b);
        if let Err(e) = fs::copy(stage.join(b), &tmp) {
            let _ = fs::remove_file(&tmp);
            log_line(&ctx.log, &format!("写入 {b} 失败: {e}，中止"));
            return 1;
        }
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755));
        if let Err(e) = fs::rename(&tmp, &dst) {
            let _ = fs::remove_file(&tmp);
            log_line(&ctx.log, &format!("替换 {b} 失败: {e}，中止"));
            return 1;
        }
        log_line(&ctx.log, &format!("已替换 {b}"));
    }

    // 4.5) 同步发行包内的文档 md：面板「文档」菜单只读 {dir}/data/www/html/*.md，
    //      升级若只换二进制，旧机器会一直缺文档（新装的站才会由 install.sh 铺）。
    sync_docs(&stage, &dir, &ctx.log);

    // 5) 重启受影响的进程：zapexec 先、zapd 后（zapd 最后保证面板进程即最新版本）。
    //    仅对 systemd 托管的服务执行自动 restart（生产环境）；
    //    开发/容器环境（rundev.sh 裸进程、docker）二进制已替换成功，
    //    记录为「需手动重启」而不是失败回滚。
    let mut failed: Vec<String> = Vec::new();
    let mut manual: Vec<String> = Vec::new();
    for svc in ["zapexec", "zapd"] {
        if !targets.contains(&svc) {
            continue;
        }
        let unit = service_unit(svc);
        if !unit_managed(&unit) {
            log_line(
                &ctx.log,
                &format!(
                    "{unit} 未注册为系统服务（开发/容器环境），新二进制已替换，请手动重启 {svc} 生效"
                ),
            );
            manual.push(svc.to_string());
            continue;
        }
        if svc_cmd(&restart_args(&unit)) {
            log_line(&ctx.log, &format!("{unit} 已重启"));
        } else {
            log_line(&ctx.log, &format!("重启 {unit} 失败，尝试回滚 {svc}"));
            failed.push(svc.to_string());
        }
    }

    // 6) 回滚：对重启失败的服务恢复备份并再试一次
    for svc in &failed {
        if let Some(src) = backup_dir.join(svc).exists().then(|| backup_dir.join(svc)) {
            let dst = dir.join(svc);
            if fs::copy(&src, &dst).is_ok() {
                let _ = fs::set_permissions(&dst, fs::Permissions::from_mode(0o755));
                log_line(&ctx.log, &format!("已回滚 {svc} 到旧版本"));
            }
        }
        let unit = service_unit(svc);
        if svc_cmd(&restart_args(&unit)) {
            log_line(&ctx.log, &format!("回滚后 {unit} 已重启"));
        } else {
            log_line(&ctx.log, &format!("{unit} 回滚后仍无法启动，请人工介入"));
        }
    }

    if failed.is_empty() {
        if manual.is_empty() {
            log_line(&ctx.log, "升级完成");
        } else {
            log_line(
                &ctx.log,
                &format!("升级完成（{} 需手动重启后生效）", manual.join(", ")),
            );
        }
        0
    } else {
        log_line(
            &ctx.log,
            &format!("升级未完全成功（失败服务: {}）", failed.join(", ")),
        );
        1
    }
}

/// 把发行包里的文档 md 同步到 `{dir}/data/www/html/`。
///
/// 面板「文档」菜单（`zapd/src/routers/docs.rs`）只读这一目录，文件不在就 404。
/// 早期版本只有 install.sh 铺文档，升级路径只换二进制，因此老机器升上来仍然缺；
/// 这里在换完二进制后补齐。tar 顶层可能是 `zap/`（发行包布局）也可能已规整到
/// stage 根，两个位置都试。
fn sync_docs(stage: &Path, dir: &Path, log: &str) {
    let src = [stage.join("data/www/html"), stage.join("zap/data/www/html")]
        .into_iter()
        .find(|p| p.is_dir());
    let Some(src) = src else {
        log_line(log, "升级包内未包含文档 md，跳过文档同步");
        return;
    };
    let dst = dir.join("data/www/html");
    if let Err(e) = fs::create_dir_all(&dst) {
        log_line(log, &format!("创建文档目录失败({e})，跳过文档同步"));
        return;
    }
    let mut copied = 0usize;
    if let Ok(entries) = fs::read_dir(&src) {
        for e in entries.flatten() {
            let path = e.path();
            if !path.is_file() {
                continue;
            }
            // 只认 .md：顺带避开 Windows ADS 残留之类的杂项文件
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".md") {
                continue;
            }
            match fs::copy(&path, dst.join(name)) {
                Ok(_) => copied += 1,
                Err(e) => log_line(log, &format!("同步文档 {name} 失败: {e}")),
            }
        }
    }
    if copied > 0 {
        log_line(
            log,
            &format!("已同步 {copied} 份文档 md 到 {}", dst.display()),
        );
    }
}

// ── 命令行模式：upgrade / rollback ──────────────────────────

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 版本号数值化（容忍 `v` 前缀与非数字尾，与 zapd `updater::parse_version` 同逻辑）。
fn parse_version(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split('.')
        .filter_map(|seg| {
            let digits: String = seg.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                None
            } else {
                digits.parse::<u64>().ok()
            }
        })
        .collect()
}

/// `latest` 是否严格大于 `current`（任一为空串时退化为字符串不相等）。
fn has_update(current: &str, latest: &str) -> bool {
    let a = parse_version(current);
    let b = parse_version(latest);
    if a.is_empty() || b.is_empty() {
        return latest != current;
    }
    b > a
}

/// 当前安装版本：跑一次 `{dir}/zapd --version`（拿不到就返回空串，交由调用方处理）。
fn current_version(dir: &Path) -> String {
    let Ok(out) = std::process::Command::new(dir.join("zapd"))
        .arg("--version")
        .output()
    else {
        return String::new();
    };
    // `split_whitespace` 自带 trim 语义（clippy: trim 是多余的）
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_start_matches('v')
        .to_string()
}

fn target_arch() -> Result<&'static str, String> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        other => Err(format!("暂不支持该架构自动升级: {other}")),
    }
}

/// 超时设置与 zapd `updater::http_agent` 保持一致（连接 10s，读写 120s）。
fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(120))
        .timeout_write(std::time::Duration::from_secs(120))
        .build()
}

fn http_get_text(url: &str) -> Result<String, String> {
    let resp = http_agent()
        .get(url)
        .call()
        .map_err(|e| format!("请求失败 {url}: {e}"))?;
    resp.into_string()
        .map_err(|e| format!("读取响应失败 {url}: {e}"))
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = http_agent()
        .get(url)
        .call()
        .map_err(|e| format!("下载失败 {url}: {e}"))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取响应体失败 {url}: {e}"))?;
    Ok(buf)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 解析目标版本：`latest` 查渠道的 `latest.txt`，否则去掉可能的 `v` 前缀。
fn resolve_version(channel: &str, to: &str) -> Result<String, String> {
    let v = to.trim();
    if v.is_empty() || v == "latest" {
        let channel = channel.trim_end_matches('/');
        let url = format!("{channel}/latest.txt");
        let text = http_get_text(&url)?;
        let s = text.trim().trim_start_matches('v').to_string();
        if s.is_empty() {
            return Err(format!("{url} 返回空内容"));
        }
        return Ok(s);
    }
    Ok(v.trim_start_matches('v').to_string())
}

/// tar 包内可能带 `zap/` 顶层目录：把其中同名二进制规整到 stage 根。
fn normalize_stage(stage: &Path) {
    for bin in BINS {
        let dst = stage.join(bin);
        if dst.exists() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(stage) {
            for e in entries.flatten() {
                let src = e.path().join(bin);
                if src.is_file() {
                    let _ = fs::copy(&src, &dst);
                    break;
                }
            }
        }
    }
}

/// 包名里的「发行线」后缀：商业版是 `-pro`，社区版没有。
///
/// 商业版与社区版同版本号、不同包名，装哪条就得一直升哪条 ——
/// 优先级：`--pro` / `--community` > `/etc/zap/edition`（install.sh 写入、
/// 本工具升级成功后回写）> 社区版。
fn edition_suffix(pro: bool, community: bool) -> &'static str {
    match edition_id(pro, community) {
        "pro" => "-pro",
        _ => "",
    }
}

/// 这次（以及以后缺省时）要跟的发行线 id，与 install.sh 的 EDITION_ID 一致。
fn edition_id(pro: bool, community: bool) -> &'static str {
    if pro {
        return "pro";
    }
    if community {
        return "community";
    }
    match fs::read_to_string(EDITION_FILE) {
        Ok(s) if s.trim() == "pro" => "pro",
        _ => "community",
    }
}

/// 把本次的发行线回写到 `/etc/zap/edition`。
///
/// 不写的话：装社区版时用 `--pro` 升级成功，下一次不带参数的升级（面板或命令行）
/// 又会按文件里的 `community` 把社区版包拉回来，等于白升。
/// 写失败不致命（非 root / 目录不存在）：记一条 warn，本次仍然生效。
fn persist_edition(id: &str, log: &str) {
    match fs::write(EDITION_FILE, format!("{id}\n")) {
        Ok(_) => {
            let _ = fs::set_permissions(EDITION_FILE, fs::Permissions::from_mode(0o644));
            log_line(log, &format!("发行线已记录: {EDITION_FILE} = {id}"));
        }
        Err(e) => log_line(
            log,
            &format!(
                "写入 {EDITION_FILE} 失败（{e}）：本次已生效，但后续不带 --pro 的升级会回到社区版"
            ),
        ),
    }
}

/// 下载发行包 → sha256 校验 → 解包 → 规整到 `{dir}/data/upgrade/stage/cli-{ts}/`。
fn download_and_stage(
    channel: &str,
    version: &str,
    dir: &Path,
    pro: bool,
    community: bool,
    log: &str,
) -> Result<PathBuf, String> {
    let channel = channel.trim_end_matches('/');
    let arch = target_arch()?;
    let base = format!(
        "{channel}/zap-v{version}{}-linux-{arch}.tar.gz",
        edition_suffix(pro, community)
    );
    // 包名打进日志：升级完发现版本线不对（比如 --pro 没生效）时，一眼能看出拉的是哪个包
    log_line(log, &format!("下载发行包: {base}"));

    let data = http_get_bytes(&base)?;
    // 校验：发行侧上传同名 .sha256（build.sh），远端缺失/为空时跳过强校验
    let expected = http_get_text(&format!("{base}.sha256"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let actual = to_hex(&sha2::Sha256::digest(&data));
    if !expected.is_empty() && !actual.eq_ignore_ascii_case(&expected) {
        return Err(format!(
            "发行包校验失败：sha256 不匹配（期望 {expected}，实际 {actual}）"
        ));
    }

    let stage = dir
        .join("data/upgrade/stage")
        .join(format!("cli-{}-v{version}", now_secs()));
    ensure_dir(&stage, dir).map_err(|e| {
        format!(
            "创建升级目录失败: {e}（{}）—— 该目录必须对运行 zapd 的账号（zapadm）可写；\
             修复：chown -R zapadm:zapadm {}",
            stage.display(),
            dir.join("data/upgrade").display()
        )
    })?;
    let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(data));
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(&stage)
        .map_err(|e| format!("解压发行包失败: {e}"))?;
    normalize_stage(&stage);
    fs::write(stage.join("version"), format!("v{version}"))
        .map_err(|e| format!("写入版本文件失败: {e}"))?;
    Ok(stage)
}

/// `zapupgrade upgrade --to <版本>`：下载 → 校验 → 走与面板同一套替换流程。
fn cmd_upgrade(
    to: &str,
    channel: &str,
    force: bool,
    pro: bool,
    community: bool,
    dir: &Path,
    log: &str,
) -> i32 {
    log_line(log, "==== ZAP 系统升级开始（命令行）====");
    log_line(
        log,
        &format!(
            "发行版: {}",
            match edition_id(pro, community) {
                "pro" => "Zap Pro",
                _ => "社区版",
            }
        ),
    );
    let cur = current_version(dir);
    log_line(
        log,
        &format!(
            "当前版本: {}",
            if cur.is_empty() {
                "未知（未安装或 zapd 不可执行）".to_string()
            } else {
                format!("v{cur}")
            }
        ),
    );

    let version = match resolve_version(channel, to) {
        Ok(v) => v,
        Err(e) => {
            log_line(log, &format!("确定目标版本失败: {e}"));
            return 1;
        }
    };
    if !force && !cur.is_empty() && !has_update(&cur, &version) {
        log_line(
            log,
            &format!(
                "当前已是 v{cur}，目标 v{version} 不高于当前版本，无需升级（要重装请加 --force）"
            ),
        );
        return 0;
    }
    log_line(log, &format!("目标版本: v{version}"));

    let stage = match download_and_stage(channel, &version, dir, pro, community, log) {
        Ok(s) => s,
        Err(e) => {
            log_line(log, &format!("下载升级包失败: {e}"));
            return 1;
        }
    };
    log_line(log, &format!("升级包已就绪: {}", stage.display()));

    let code = run_upgrade(&RunCtx {
        stage: stage.to_string_lossy().into_owned(),
        dir: dir.to_string_lossy().into_owned(),
        log: log.to_string(),
    });
    // 解包目录是一次性的：成功就清掉，失败时保留便于排查
    if code == 0 {
        // 换线（社区版 → Pro，或 Pro → 社区版）必须落盘，否则下次升级又会按旧发行线拉包
        persist_edition(edition_id(pro, community), log);
        let _ = fs::remove_dir_all(&stage);
    }
    code
}

/// `zapupgrade rollback`：列出或恢复 `data/upgrade/backup/` 下的历史备份。
fn cmd_rollback(list: bool, to: &Option<String>, dir: &Path, log: &str) -> i32 {
    let root = dir.join("data/upgrade/backup");
    // 目录名形如 {ts}-{version}，ts 定长，字典序即时间序
    let mut entries: Vec<(String, PathBuf)> = fs::read_dir(&root)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    if entries.is_empty() {
        log_line(log, &format!("没有可用的备份（{}）", root.display()));
        return 1;
    }
    if list {
        log_line(log, &format!("可用备份（{}，由旧到新）：", root.display()));
        for (name, p) in &entries {
            let bins: Vec<&str> = BINS
                .iter()
                .filter(|b| p.join(b).is_file())
                .copied()
                .collect();
            log_line(log, &format!("  {name}  [{}]", bins.join(", ")));
        }
        return 0;
    }

    let chosen = match to {
        Some(name) => match entries.into_iter().find(|(n, _)| n == name) {
            Some(x) => x,
            None => {
                log_line(
                    log,
                    &format!("找不到备份 {name}（用 rollback --list 查看可用备份）"),
                );
                return 1;
            }
        },
        // entries 已确认非空
        None => entries.pop().unwrap_or_else(|| unreachable!()),
    };
    log_line(log, &format!("回滚到备份: {}", chosen.0));

    let mut restored: Vec<&str> = Vec::new();
    for b in BINS {
        let src = chosen.1.join(b);
        if !src.is_file() {
            continue;
        }
        let dst = dir.join(b);
        let tmp = dir.join(format!(".{b}.rollback-new"));
        if fs::copy(&src, &tmp).is_err() {
            let _ = fs::remove_file(&tmp);
            log_line(log, &format!("恢复 {b} 失败"));
            return 1;
        }
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755));
        if fs::rename(&tmp, &dst).is_err() {
            let _ = fs::remove_file(&tmp);
            log_line(log, &format!("替换 {b} 失败"));
            return 1;
        }
        restored.push(b);
    }
    if restored.is_empty() {
        log_line(log, "该备份中没有可恢复的二进制");
        return 1;
    }

    // 与升级同序：zapexec 先、zapd 后
    for svc in ["zapexec", "zapd"] {
        if !restored.contains(&svc) {
            continue;
        }
        let unit = service_unit(svc);
        if !unit_managed(&unit) {
            log_line(
                log,
                &format!("{unit} 未注册为系统服务，旧二进制已恢复，请手动重启 {svc} 生效"),
            );
            continue;
        }
        if svc_cmd(&restart_args(&unit)) {
            log_line(log, &format!("{unit} 已重启"));
        } else {
            log_line(log, &format!("重启 {unit} 失败，请人工介入"));
            return 1;
        }
    }
    log_line(log, "回滚完成");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(has_update("1.0.11", "1.0.12"));
        assert!(!has_update("1.0.12", "1.0.12"));
        assert!(!has_update("1.1.0", "1.0.12"));
        // 带 v 前缀 / 非数字尾都容忍
        assert!(has_update("v1.0.11", "v1.0.12-beta"));
        // 无法解析时退化为字符串比较
        assert!(has_update("", "1.0.12"));
        assert!(!has_update("1.0.12", "1.0.12"));
    }

    #[test]
    fn backup_entries_sort_by_name() {
        // 备份目录名 {ts}-{version}，ts 定长 → 字典序即时间序
        let mut names = [
            "1700000000-v1.0.12",
            "1600000000-v1.0.10",
            "1650000000-v1.0.11",
        ];
        names.sort();
        assert_eq!(*names.last().unwrap(), "1700000000-v1.0.12");
    }

    #[test]
    fn target_arch_is_release_arch() {
        let arch = target_arch();
        match std::env::consts::ARCH {
            "x86_64" => assert_eq!(arch.unwrap(), "amd64"),
            "aarch64" => assert_eq!(arch.unwrap(), "arm64"),
            _ => assert!(arch.is_err()),
        }
    }
}
