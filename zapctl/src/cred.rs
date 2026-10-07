// SPDX-License-Identifier: AGPL-3.0-only
//! `zapctl cred` —— 密码生成与服务凭据（加密存储）。
//!
//! 用法：
//!   zapctl cred gen                     # 只生成密码并输出到屏幕
//!   zapctl cred gen mysql root          # 生成并加密保存到 /etc/zap/credentials/mysql_root.cred
//!   zapctl cred show mysql root         # 解密输出（root，便于脚本取值）
//!   zapctl cred exists mysql root       # 存在退出码 0，否则 1
//!   zapctl cred ls                      # 列出已保存凭据
//!   zapctl cred rm mysql root           # 删除凭据
//!   zapctl cred set mysql root <密码>   # 录入服务侧既有密码（存在则覆盖；缺省从 stdin 读取）
//!
//! 设计要点：
//! - 明文密码**只输出到 stdout**，提示信息走 stderr，`$(zapctl cred ...)` 可直接取值；
//! - 文件 `/etc/zap/credentials/{service}_{user}.cred`，权限 0400，目录 0700；
//! - 内容用 `zap-crypto`（AES-256-GCM）加密，`zapexec` 用同一套密钥可解密。

use clap::Subcommand;

use crate::{GREEN, NC, YELLOW, info, ok};

#[derive(Subcommand)]
pub enum CredCommand {
    /// 生成密码：不带参数仅输出到屏幕；带 <服务> <用户> 时加密保存
    Gen {
        /// 服务名（如 mysql）
        service: Option<String>,
        /// 用户名（如 root / zapadm）
        user: Option<String>,
        /// 密码长度（默认 20，下限 8）
        #[arg(long, default_value_t = 20)]
        len: usize,
        /// 允许特殊字符（默认仅字母数字，避免 shell / SQL 转义问题）
        #[arg(long)]
        symbols: bool,
        /// 已存在时覆盖（默认拒绝，防止覆盖正在使用的凭据）
        #[arg(long)]
        force: bool,
    },
    /// 读取并解密指定凭据（明文输出到 stdout）
    Show {
        /// 服务名（如 mysql）
        service: String,
        /// 用户名（如 root）
        user: String,
    },
    /// 检查凭据是否存在（存在退出码 0，不存在 1）
    Exists {
        /// 服务名
        service: String,
        /// 用户名
        user: String,
    },
    /// 列出已保存的凭据
    Ls,
    /// 删除凭据
    Rm {
        /// 服务名
        service: String,
        /// 用户名
        user: String,
    },
    /// 录入服务侧已存在的密码（如管理员手动建库 / 改密后同步），存在则覆盖
    Set {
        /// 服务名（如 mysql）
        service: String,
        /// 用户名（如 root）
        user: String,
        /// 明文密码；缺省从 stdin 读取首行（避免留在 shell 历史里）
        password: Option<String>,
    },
}

pub fn dispatch(cmd: CredCommand) -> Result<(), String> {
    match cmd {
        CredCommand::Gen {
            service,
            user,
            len,
            symbols,
            force,
        } => cmd_gen(service, user, len, symbols, force),
        CredCommand::Show { service, user } => show(&service, &user),
        CredCommand::Exists { service, user } => exists(&service, &user),
        CredCommand::Ls => ls(),
        CredCommand::Rm { service, user } => rm(&service, &user),
        CredCommand::Set {
            service,
            user,
            password,
        } => set(&service, &user, password),
    }
}

/// 生成密码。
/// - 无 service/user：只输出明文密码（供人看或脚本取用）
/// - 有 service/user：加密落盘，明文仍输出到 stdout
///
/// 注意：`gen` 是 Rust 2024 保留字，故函数名用 `cmd_gen`。
fn cmd_gen(
    service: Option<String>,
    user: Option<String>,
    len: usize,
    symbols: bool,
    force: bool,
) -> Result<(), String> {
    let password = zap_crypto::generate_password(len, symbols)?;

    match (service, user) {
        (None, None) => {
            // 仅生成，不落盘
            println!("{password}");
            Ok(())
        }
        (Some(s), Some(u)) => {
            crate::ensure_root()?;
            if zap_crypto::cred_exists(&s, &u) && !force {
                return Err(format!(
                    "凭据 {s}_{u} 已存在（避免覆盖正在使用的密码）；如需重新生成请加 --force"
                ));
            }
            let path = zap_crypto::save_cred(&s, &u, &password)?;
            // 明文单独占一行 stdout，便于 `$(...)` 取值；提示信息走 stderr
            println!("{password}");
            eprintln!("{GREEN}[✓]{NC} 已加密保存: {}", path.display());
            eprintln!("{YELLOW}[!]{NC} 文件权限 0400，仅 root 可读；zapexec 可用同一密钥解密");
            Ok(())
        }
        _ => Err("参数不完整：请同时提供 <服务> <用户>，或都不提供（只输出密码）".to_string()),
    }
}

/// 解密并输出明文。
fn show(service: &str, user: &str) -> Result<(), String> {
    crate::ensure_root()?;
    let password = zap_crypto::read_cred(service, user)?;
    println!("{password}");
    Ok(())
}

/// 存在性检查：不存在时以退出码 1 结束，便于脚本 `if zapctl cred exists ...`。
fn exists(service: &str, user: &str) -> Result<(), String> {
    if zap_crypto::cred_exists(service, user) {
        ok(&format!("凭据存在: {service}_{user}"));
        Ok(())
    } else {
        info(&format!("凭据不存在: {service}_{user}"));
        std::process::exit(1);
    }
}

fn ls() -> Result<(), String> {
    crate::ensure_root()?;
    let items = zap_crypto::list_creds()?;
    if items.is_empty() {
        info(&format!("暂无凭据（目录: {}）", zap_crypto::CRED_DIR));
        return Ok(());
    }
    println!("{:<32} {:<10} PATH", "CREDENTIAL", "SIZE");
    println!("{}", "-".repeat(32 + 1 + 10 + 1 + 40));
    for (name, path) in items {
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        println!("{name:<32} {size:<10} {}", path.display());
    }
    Ok(())
}

fn rm(service: &str, user: &str) -> Result<(), String> {
    crate::ensure_root()?;
    zap_crypto::remove_cred(service, user)?;
    ok(&format!("已删除凭据: {service}_{user}"));
    Ok(())
}

/// 录入服务侧已存在的密码（区别于 `gen` 的随机生成）。存在则覆盖，不回显明文。
fn set(service: &str, user: &str, password: Option<String>) -> Result<(), String> {
    crate::ensure_root()?;

    let password = match password {
        Some(p) if !p.is_empty() => p,
        Some(_) => return Err("密码不能为空".to_string()),
        None => {
            // 缺省从 stdin 读取一行：密码不留在命令行 / shell 历史里
            use std::io::{BufRead, IsTerminal};
            if std::io::stdin().is_terminal() {
                eprintln!("{YELLOW}[?]{NC} 请输入密码（回车确认）:");
            }
            let mut line = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut line)
                .map_err(|e| format!("读取 stdin 失败: {e}"))?;
            let p = line.trim_end_matches(['\r', '\n']);
            if p.is_empty() {
                return Err(
                    "密码不能为空：请以参数提供（注意会留在 shell 历史），或经管道传入 stdin"
                        .to_string(),
                );
            }
            p.to_string()
        }
    };

    if password.len() < 8 {
        eprintln!("{YELLOW}[!]{NC} 密码长度不足 8 位，建议使用强密码");
    }

    let path = zap_crypto::save_cred(service, user, &password)?;
    // 不回显明文；状态信息走 stderr，保持 stdout 干净
    eprintln!("{GREEN}[✓]{NC} 已加密保存: {}", path.display());
    eprintln!("{YELLOW}[!]{NC} 文件权限 0400，仅 root 可读；zapexec 可用同一密钥解密");
    Ok(())
}
