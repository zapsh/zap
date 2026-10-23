// SPDX-License-Identifier: AGPL-3.0-only
//! 服务管理后端：按 init 系统抽象（目前 systemd / openrc）。
//!
//! 动作动词与 init 系统一一对应（`start` / `stop` / `restart` / `reload` /
//! `enable` / `disable`），查询也直接沿用对应管理器的接口。
//!
//! | 语义           | systemd                       | openrc (Alpine 等)        |
//! |----------------|-------------------------------|---------------------------|
//! | 服务是否存在   | `list-unit-files <n>.service` | `/etc/init.d/<n>` 存在    |
//! | 是否运行中     | `is-active [-q] <n>`          | `rc-service <n> status`   |
//! | 是否开机自启   | `is-enabled [-q] <n>`         | `rc-update show default`  |
//!
//! **服务名的发行版差异不在这里处理**：nginx / sshd / php-fpm 在各发行版上叫法不同
//! （Debian 的 `ssh` vs RHEL 的 `sshd`、Alpine 的 `php-fpm83`），那是各动词自己的
//! 业务知识——它们按候选名依次探测，本层只负责"给定一个名字去操作"。
//!
//! 非 Linux 平台 [`supported()`] 恒为 false：查询类一律返回「不存在 / 未运行」，
//! 动作类返回「不支持」，不会真的去执行管理器命令。
//!
//! 新增 init 系统（如 sys-v / runit）只需补一个 [`ServiceManager`] 实现并在
//! [`manager_impl`] 注册，无需改动上层调用方。

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use super::root_cmd;

/// 服务管理器命令（systemd）。
pub(crate) const SYSTEMD_CTL: &str = "systemctl";
/// openrc 自启管理命令（rc-update add/del <svc> default）。
pub(crate) const OPENRC_UPDATE: &str = "rc-update";
/// openrc 初始化脚本目录。
pub(crate) const OPENRC_INIT_DIR: &str = "/etc/init.d";

/// 服务管理器类型：决定 exists / is_active / is_enabled / act 走哪一套实现。
///
/// 默认 systemd（绝大多数 Linux 发行版）；openrc 用于 Alpine 等。未来再加
/// sys-v / runit 也只是新增一个 `ServiceManager` 实现。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Manager {
    #[default]
    Systemd,
    Openrc,
}

/// init 系统的统一抽象：给定服务名，提供存在 / 运行 / 自启 / 动作四类查询。
pub(crate) trait ServiceManager {
    fn exists(&self, name: &str) -> bool;
    fn is_active(&self, name: &str) -> bool;
    fn is_enabled(&self, name: &str) -> bool;
    fn act(&self, action: &str, name: &str) -> Result<(), String>;
}

/// 平台不支持自动服务管理时的统一提示。
fn unsupported_msg() -> String {
    format!(
        "当前平台（{}）不支持自动服务管理，请手动操作",
        std::env::consts::OS
    )
}

fn run_ok(cmd: &str, args: &[&str]) -> bool {
    root_cmd(cmd)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── systemd 实现 ───────────────────────────────────────────────

struct Systemd;
impl ServiceManager for Systemd {
    fn exists(&self, name: &str) -> bool {
        systemd_exists(name)
    }
    fn is_active(&self, name: &str) -> bool {
        systemd_active(name)
    }
    fn is_enabled(&self, name: &str) -> bool {
        systemd_enabled(name)
    }
    fn act(&self, action: &str, name: &str) -> Result<(), String> {
        act_with(SYSTEMD_CTL, &[action, name])
    }
}

/// 服务是否存在（unit 已注册）。
fn systemd_exists(name: &str) -> bool {
    if !supported() {
        return false;
    }
    root_cmd(SYSTEMD_CTL)
        .args(["list-unit-files", "--no-legend", &format!("{name}.service")])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.trim().starts_with(name))
        })
        .unwrap_or(false)
}

/// 服务当前是否在运行。
fn systemd_active(name: &str) -> bool {
    if !supported() {
        return false;
    }
    root_cmd(SYSTEMD_CTL)
        .args(["is-active", "--quiet", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 服务是否开机自启。
fn systemd_enabled(name: &str) -> bool {
    if !supported() {
        return false;
    }
    root_cmd(SYSTEMD_CTL)
        .args(["is-enabled", "--quiet", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── openrc 实现 ────────────────────────────────────────────────

struct Openrc;
impl ServiceManager for Openrc {
    fn exists(&self, name: &str) -> bool {
        Path::new(&format!("/etc/init.d/{name}")).is_file()
    }
    fn is_active(&self, name: &str) -> bool {
        run_ok("rc-service", &[name, "status"])
    }
    fn is_enabled(&self, name: &str) -> bool {
        match Command::new("rc-update").args(["show", "default"]).output() {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout);
                s.lines()
                    .any(|l| l.split_whitespace().any(|w| w == name))
            }
            Err(_) => false,
        }
    }
    fn act(&self, action: &str, name: &str) -> Result<(), String> {
        act_with("rc-service", &[name, action])
    }
}

/// 按类型取实现（装箱，便于在统一入口按 `manager` 分发）。
pub(crate) fn manager_impl(m: Manager) -> Box<dyn ServiceManager> {
    match m {
        Manager::Systemd => Box::new(Systemd),
        Manager::Openrc => Box::new(Openrc),
    }
}

/// 当前系统的实际 init 管理器（运行时探测）。
///
/// 服务定义在 YAML 里若不显式写 `manager`，就按此自动识别，避免让管理员在表单里选。
/// 主流发行版是 systemd；Alpine 等用 openrc（以 `/etc/init.d` 目录存在判定）。
pub(crate) fn active_manager() -> Manager {
    if Path::new("/run/systemd/system").exists() {
        return Manager::Systemd;
    }
    if Path::new(OPENRC_INIT_DIR).exists() {
        return Manager::Openrc;
    }
    Manager::Systemd
}

/// 按 manager 查服务是否存在。
pub(crate) fn exists_m(m: Manager, name: &str) -> bool {
    manager_impl(m).exists(name)
}
/// 按 manager 查服务是否在运行。
pub(crate) fn is_active_m(m: Manager, name: &str) -> bool {
    manager_impl(m).is_active(name)
}
/// 按 manager 执行动作。
pub(crate) fn act_m(m: Manager, action: &str, name: &str) -> Result<(), String> {
    manager_impl(m).act(action, name)
}

/// 当前平台的自动服务管理是否受支持（Linux + systemd）。
///
/// macOS 用 launchctl，不在支持范围内——在那些平台上本函数返回 false，上层应
/// 降级为「需手动操作」而不是报命令找不到。
pub(crate) const fn supported() -> bool {
    cfg!(target_os = "linux")
}

/// `service.list` 的一行。
pub(crate) struct Row {
    pub name: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

/// 服务是否存在（unit 已注册）—— systemd（总览页用，与管理器类型无关）。
pub(crate) fn exists(name: &str) -> bool {
    systemd_exists(name)
}

/// 服务当前是否在运行 —— systemd。
pub(crate) fn is_active(name: &str) -> bool {
    systemd_active(name)
}

/// 服务是否开机自启 —— systemd。
pub(crate) fn is_enabled(name: &str) -> bool {
    systemd_enabled(name)
}

/// 服务状态原始串，词汇沿用 systemd 的
/// `active` / `inactive` / `failed` / `activating` / `deactivating` / `unknown`，
/// 便于上层复用自己的归一化逻辑。
pub(crate) fn raw_state(name: &str) -> String {
    if !supported() {
        return "unknown".into();
    }
    root_cmd(SYSTEMD_CTL)
        .args(["is-active", name])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// 管理器本身是否可用。
pub(crate) fn manager_running() -> bool {
    if !supported() {
        return false;
    }
    root_cmd(SYSTEMD_CTL)
        .args(["is-system-running", "--quiet"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 执行服务动作：`start` / `stop` / `restart` / `reload` / `enable` / `disable`。
/// 失败时返回 stderr（为空则回落 stdout）。
pub(crate) fn act(action: &str, name: &str) -> Result<(), String> {
    if !supported() {
        return Err(unsupported_msg());
    }
    // 开关与启停在 systemd 下是同一个命令、同一种参数形式
    act_with(SYSTEMD_CTL, &[action, name])
}

/// 执行 `cmd <args...>`，失败时把 stderr（为空则回落 stdout）收敛成一行。
fn act_with(cmd: &str, args: &[&str]) -> Result<(), String> {
    let o = root_cmd(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("执行 {cmd} {} 失败: {e}", args.join(" ")))?;
    if o.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&o.stderr).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&o.stdout).trim().to_string();
    }
    if text.is_empty() {
        text = format!("{cmd} {} 返回非零", args.join(" "));
    }
    // 启动失败的 stderr 可能是几十 KB 的日志，截断以免响应膨胀
    Err(text.chars().take(2000).collect())
}

/// 列出系统所有服务。
pub(crate) fn list() -> Result<Vec<Row>, String> {
    if !supported() {
        return Err(unsupported_msg());
    }
    let o = root_cmd(SYSTEMD_CTL)
        .args([
            "list-units",
            "--type=service",
            "--all",
            "--no-pager",
            "--no-legend",
            "--plain",
        ])
        .output()
        .map_err(|e| format!("获取服务列表失败: {e}"))?;
    if !o.status.success() {
        let text = String::from_utf8_lossy(&o.stderr).trim().to_string();
        return Err(if text.is_empty() {
            "获取服务列表失败".into()
        } else {
            text
        });
    }
    let text = String::from_utf8_lossy(&o.stdout);
    Ok(text
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut it = line.split_whitespace();
            let name = it.next()?.to_string();
            if !name.ends_with(".service") {
                return None;
            }
            let load = it.next().unwrap_or("").to_string();
            let active = it.next().unwrap_or("").to_string();
            let sub = it.next().unwrap_or("").to_string();
            let description: Vec<&str> = it.collect();
            Some(Row {
                name,
                load,
                active,
                sub,
                description: description.join(" "),
            })
        })
        .collect())
}
