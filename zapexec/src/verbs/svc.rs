//! 服务管理后端：只面向 systemd（zap 仅支持 Linux）。
//!
//! 动作动词与 systemd 一一对应（`start` / `stop` / `restart` / `reload` /
//! `enable` / `disable`），查询也直接沿用 systemd 的接口：
//!
//! | 语义           | systemd                       |
//! |----------------|-------------------------------|
//! | 服务是否存在   | `list-unit-files <n>.service` |
//! | 是否运行中     | `is-active [-q] <n>`          |
//! | 是否开机自启   | `is-enabled [-q] <n>`         |
//! | 管理器是否就绪 | `is-system-running`           |
//! | 列出所有服务   | `list-units --type=service`   |
//!
//! **服务名的发行版差异不在这里处理**：nginx / sshd / php-fpm 在各发行版上叫法不同
//! （Debian 的 `ssh` vs RHEL 的 `sshd`、Alpine 的 `php-fpm83`），那是各动词自己的
//! 业务知识——它们按候选名依次探测，本层只负责"给定一个名字去操作"。
//!
//! 非 Linux 平台 [`supported()`] 恒为 false：查询类一律返回「不存在 / 未运行」，
//! 动作类返回「不支持」，不会真的去执行 systemctl。

use super::root_cmd;

/// 服务管理器命令（systemd）。
pub(crate) const CTL: &str = "systemctl";

/// 当前平台的自动服务管理是否受支持（Linux + systemd）。
///
/// macOS 用 launchctl，不在支持范围内——在那些平台上本函数返回 false，上层应
/// 降级为「需手动操作」而不是报命令找不到。
pub(crate) const fn supported() -> bool {
    cfg!(target_os = "linux")
}

/// 平台不支持自动服务管理时的统一提示。
fn unsupported_msg() -> String {
    format!(
        "当前平台（{}）不支持自动服务管理，请手动操作",
        std::env::consts::OS
    )
}

/// `service.list` 的一行。
pub(crate) struct Row {
    pub name: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

/// 服务是否存在（unit 已注册）。
pub(crate) fn exists(name: &str) -> bool {
    if !supported() {
        return false;
    }
    root_cmd(CTL)
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
pub(crate) fn is_active(name: &str) -> bool {
    if !supported() {
        return false;
    }
    root_cmd(CTL)
        .args(["is-active", "--quiet", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 服务是否开机自启。
pub(crate) fn is_enabled(name: &str) -> bool {
    if !supported() {
        return false;
    }
    root_cmd(CTL)
        .args(["is-enabled", "--quiet", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 服务状态原始串，词汇沿用 systemd 的
/// `active` / `inactive` / `failed` / `activating` / `deactivating` / `unknown`，
/// 便于上层复用自己的归一化逻辑。
pub(crate) fn raw_state(name: &str) -> String {
    if !supported() {
        return "unknown".into();
    }
    root_cmd(CTL)
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
    root_cmd(CTL)
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
    act_with(CTL, &[action, name])
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
    let o = root_cmd(CTL)
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
