//! `zapctl` —— ZAP 服务器/VPS 管理系统控制工具。
//!
//! 职责：管理 `zapd`（业务主进程）与 `zapexec`（root 特权守护进程）的
//! systemd 服务（start / stop / restart / status / enable / disable / logs）；
//! 备份（backup：zap 数据库/配置、用户数据）、用户（user / passwd）、配置文件键值（config）。
//!
//! 后续运维能力（如远程任务下发、一键健康检查等）在 [`Command`] 中追加子命令即可。

mod backup;
mod config;
mod cred;
mod env;
mod svc;
mod user;

use std::process::Command as ProcessCommand;

use clap::{Parser, Subcommand, ValueEnum};

// ── 服务登记表（后续新增服务在此处登记）───────────────────────
// 不带 `.service` 后缀：由 [`svc::unit_name`] 按平台补（OpenBSD 没有后缀概念）。
const UNIT_ZAPD: &str = "zapd";
const UNIT_ZAPEXEC: &str = "zapexec";

// ── 终端颜色（与 install.sh / rundev.sh 风格一致）─────────────
pub const GREEN: &str = "\x1b[0;32m";
pub const RED: &str = "\x1b[0;31m";
pub const YELLOW: &str = "\x1b[1;33m";
pub const BLUE: &str = "\x1b[0;34m";
pub const NC: &str = "\x1b[0m";

pub fn info(msg: &str) {
    println!("{BLUE}[*]{NC} {msg}");
}
pub fn ok(msg: &str) {
    println!("{GREEN}[✓]{NC} {msg}");
}

#[derive(Parser)]
#[command(
    name = "zapctl",
    version,
    about = "ZAP 服务器管理工具（管理 zapd / zapexec 服务）",
    long_about = None
)]
struct Cli {
    /// 覆盖数据库路径（默认从 zap.yaml 的 db.path 读取）
    #[arg(long, global = true)]
    db: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 启动服务
    Start {
        /// 目标服务，默认全部
        #[arg(value_enum, default_value_t = Service::All)]
        service: Service,
    },
    /// 停止服务
    Stop {
        #[arg(value_enum, default_value_t = Service::All)]
        service: Service,
    },
    /// 重启服务
    Restart {
        #[arg(value_enum, default_value_t = Service::All)]
        service: Service,
    },
    /// 查看服务运行状态
    Status {
        #[arg(value_enum, default_value_t = Service::All)]
        service: Service,
    },
    /// 设置开机自启
    Enable {
        #[arg(value_enum, default_value_t = Service::All)]
        service: Service,
    },
    /// 取消开机自启
    Disable {
        #[arg(value_enum, default_value_t = Service::All)]
        service: Service,
    },
    /// 查看服务日志（journalctl）
    Logs {
        #[arg(value_enum, default_value_t = Service::All)]
        service: Service,
        /// 持续跟踪日志输出（等效 journalctl -f）
        #[arg(short, long)]
        follow: bool,
        /// 显示最近 N 行
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: u32,
    },
    /// 备份 / 还原 / 管理归档：zap（zap）、用户（user / users）、还原（restore）、列出（list）、清理（prune）
    Backup {
        #[command(subcommand)]
        cmd: backup::BackupCommand,
    },
    /// 用户管理
    User {
        #[command(subcommand)]
        cmd: user::UserCommand,
    },
    /// 修改指定用户的密码（校验旧密码）
    Passwd {
        /// 用户名
        username: String,
    },
    /// 密码生成与服务凭据（加密存储于 /etc/zap/credentials，权限 0400）
    Cred {
        #[command(subcommand)]
        cmd: cred::CredCommand,
    },
    /// 查看 / 新增 / 修改 / 删除配置文件（zap.yaml）键值（不带子命令时等价于 `config get`，打印全部配置）
    Config {
        /// 目标配置文件（默认：ZAP_CONFIG > /etc/zap/zap.yaml > conf/zap.yaml）
        #[arg(short = 'f', long)]
        file: Option<String>,
        #[command(subcommand)]
        cmd: Option<config::ConfigCommand>,
    },
    /// 运行环境键值（server_env 表）：列出（list）、读取（get）、设置（set）、删除（unset）、导入（import）
    Env {
        #[command(subcommand)]
        cmd: Option<env::EnvCommand>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Service {
    /// 业务主进程
    Zapd,
    /// 特权守护进程（root）
    Zapexec,
    /// 全部服务
    All,
}

impl Service {
    fn units(self) -> Vec<String> {
        match self {
            Service::Zapd => vec![svc::unit_name(UNIT_ZAPD)],
            Service::Zapexec => vec![svc::unit_name(UNIT_ZAPEXEC)],
            Service::All => vec![svc::unit_name(UNIT_ZAPD), svc::unit_name(UNIT_ZAPEXEC)],
        }
    }
}

impl std::fmt::Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Service::Zapd => "zapd",
            Service::Zapexec => "zapexec",
            Service::All => "all",
        })
    }
}

fn main() {
    let cli = Cli::parse();

    let db_path = config::db_path(cli.db.as_deref());

    let result = match cli.command {
        Command::Start { service } => manage("start", service),
        Command::Stop { service } => manage("stop", service),
        Command::Restart { service } => manage("restart", service),
        Command::Enable { service } => manage("enable", service),
        Command::Disable { service } => manage("disable", service),
        Command::Status { service } => status(service),
        Command::Logs {
            service,
            follow,
            lines,
        } => logs(service, follow, lines),
        Command::Backup { cmd } => backup::dispatch(cmd, &db_path),
        Command::User { cmd } => user::dispatch(cmd, &db_path),
        Command::Passwd { username } => user::cmd_self_passwd(&db_path, &username),
        Command::Cred { cmd } => cred::dispatch(cmd),
        Command::Config { file, cmd } => {
            // 缺省子命令时等价于 `config get`（打印整个配置文件）
            let cmd = cmd.unwrap_or(config::ConfigCommand::Get { key: None });
            config::dispatch(cmd, file.as_deref())
        }
        Command::Env { cmd } => {
            // 缺省子命令时等价于 `env list`（列出全部 scope）
            let cmd = cmd.unwrap_or(env::EnvCommand::List {
                scope: None,
                json: false,
            });
            env::dispatch(cmd, &db_path)
        }
    };

    if let Err(e) = result {
        eprintln!("{RED}[✗]{NC} {e}");
        std::process::exit(1);
    }
}

// ── 子命令实现 ────────────────────────────────────────────────

/// 变更类操作（start/stop/restart/enable/disable）需要 root。
fn manage(verb: &str, service: Service) -> Result<(), String> {
    ensure_root()?;

    let units = service.units();
    info(&format!("{verb} {}", units.join(" ")));
    // 逐个执行：两个管理器都支持一次传多个服务名，但这样报错能落到具体服务上
    for unit in &units {
        svc::act(verb, unit)?;
    }
    ok(&format!("{} {}", verb, units.join(" ")));
    Ok(())
}

/// 查看状态，无需 root。
fn status(service: Service) -> Result<(), String> {
    println!(
        "{:<10} {:<24} {:<10} {:<8}",
        "SERVICE", "STATE", "ENABLED", "PID"
    );
    println!("{}", "-".repeat(10 + 1 + 24 + 1 + 10 + 1 + 8));

    for unit in service.units() {
        let st = svc::state_of(&unit)?;
        let name = svc::display_name(&unit);

        let load = st.load.as_str();
        let active = st.active.as_str();
        let sub = st.sub.as_str();
        let enabled = st.enabled.as_str();
        let pid = st.pid.as_str();

        let (state, color) = if load == "not-found" {
            ("not-installed".to_string(), RED)
        } else if active == "active" {
            (format!("active ({sub})"), GREEN)
        } else if active == "failed" {
            ("failed".to_string(), RED)
        } else {
            (format!("{active} ({sub})"), YELLOW)
        };

        let pid = if pid.is_empty() || pid == "0" {
            "-".to_string()
        } else {
            pid.to_string()
        };
        let enabled = if enabled.is_empty() {
            "-".to_string()
        } else {
            enabled.to_string()
        };

        println!("{name:<10} {color}{state:<24}{NC} {enabled:<10} {pid:<8}");
    }

    Ok(())
}

/// 查看日志。`-f` 时使用继承 stdio 的方式以支持持续输出。
///
/// 注意：`journalctl` 是 systemd 专有。OpenBSD 没有它，日志在 `/var/log/messages`
/// 里按行滚动——到时候在这里按平台分支（`-f` 对应 `tail -f` 并自行按标识过滤）。
fn logs(service: Service, follow: bool, lines: u32) -> Result<(), String> {
    let mut args: Vec<String> = Vec::new();
    for unit in service.units() {
        args.push("-u".to_string());
        args.push(unit.to_string());
    }
    if follow {
        args.push("-f".to_string());
    }
    args.push("-n".to_string());
    args.push(lines.to_string());

    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    run_inherit("journalctl", &args_ref)
}

// ── 底层工具 ──────────────────────────────────────────────────

pub fn ensure_root() -> Result<(), String> {
    let euid = unsafe { libc::geteuid() };
    if euid == 0 {
        Ok(())
    } else {
        Err("该操作需要 root 权限，请使用：sudo zapctl <命令>".to_string())
    }
}

/// 继承 stdio 执行（用于 journalctl -f 等需要流式/交互输出的命令）。
fn run_inherit(program: &str, args: &[&str]) -> Result<(), String> {
    let status = ProcessCommand::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("无法执行 {program}: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} {} 执行失败（退出码 {}）",
            args.join(" "),
            status
                .code()
                .map_or_else(|| "?".to_string(), |c| c.to_string())
        ))
    }
}
