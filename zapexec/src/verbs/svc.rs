//! 服务管理后端：把 systemd 与 OpenBSD rcctl 的差异收敛在这里。
//!
//! 好消息是两套管理器的**动作动词一致**（`start` / `stop` / `restart` / `reload` /
//! `enable` / `disable`），差异集中在查询类与控制器的就绪判断：
//!
//! | 语义           | systemd                        | OpenBSD rcctl                  |
//! |----------------|--------------------------------|--------------------------------|
//! | 服务是否存在   | `list-unit-files <n>.service`  | `ls all` 里能查到该 daemon     |
//! | 是否运行中     | `is-active [-q] <n>`           | `check <n>`                    |
//! | 是否开机自启   | `is-enabled [-q] <n>`          | `ls on` 里能查到该 daemon      |
//! | 管理器是否就绪 | `is-system-running`            | 恒为真（无中央管理器可挂）     |
//! | 列出所有服务   | `list-units --type=service`    | `ls all` + 逐个 `check`        |
//!
//! **服务名的平台差异不在这里处理**：nginx / sshd / php-fpm 在各发行版、各 BSD 上叫法
//! 不同（Debian 的 `ssh` vs RHEL 的 `sshd`、OpenBSD 的 `php82_fpm`），那是各动词自己的
//! 业务知识——它们按候选名依次探测，本层只负责"给定一个名字去操作"。

use super::root_cmd;

/// 服务管理器命令。
#[cfg(target_os = "linux")]
pub(crate) const CTL: &str = "systemctl";

/// 同 [`CTL`]，非 Linux 平台。
#[cfg(not(target_os = "linux"))]
pub(crate) const CTL: &str = "rcctl";

/// `service.list` 的一行。
pub(crate) struct Row {
    pub name: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

/// 服务是否存在（unit 已注册 / daemon 已登记）。
pub(crate) fn exists(name: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        root_cmd(CTL)
            .args([
                "list-unit-files",
                "--no-legend",
                &format!("{name}.service"),
            ])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|l| l.trim().starts_with(name))
            })
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        ls_set("all").contains(&name.to_string())
    }
}

/// 服务当前是否在运行。
pub(crate) fn is_active(name: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        root_cmd(CTL)
            .args(["is-active", "--quiet", name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // rcctl check：daemon 在跑返回 0，没跑或不存在都返回非 0
        root_cmd(CTL)
            .args(["check", name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

/// 服务是否开机自启。
pub(crate) fn is_enabled(name: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        root_cmd(CTL)
            .args(["is-enabled", "--quiet", name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        ls_set("on").contains(&name.to_string())
    }
}

/// 服务状态原始串，词汇沿用 systemd 的
/// `active` / `inactive` / `failed` / `activating` / `deactivating` / `unknown`，
/// 便于上层复用自己的归一化逻辑。
pub(crate) fn raw_state(name: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        root_cmd(CTL)
            .args(["is-active", name])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "unknown".into())
    }
    #[cfg(not(target_os = "linux"))]
    {
        // rcctl 只有「在跑 / 没跑」两态，没有 starting/failed 的细分
        if !exists(name) {
            return "unknown".into();
        }
        if is_active(name) {
            "active".into()
        } else {
            "inactive".into()
        }
    }
}

/// 管理器本身是否可用。
pub(crate) fn manager_running() -> bool {
    #[cfg(target_os = "linux")]
    {
        root_cmd(CTL)
            .args(["is-system-running", "--quiet"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // OpenBSD 没有常驻管理器：rc.d 脚本由 rc 直接调用，谈不上"管理器未就绪"
        true
    }
}

/// 执行服务动作：`start` / `stop` / `restart` / `reload` / `enable` / `disable`。
/// 失败时返回 stderr（为空则回落 stdout）。
pub(crate) fn act(action: &str, name: &str) -> Result<(), String> {
    let o = root_cmd(CTL)
        .args([action, name])
        .output()
        .map_err(|e| format!("执行 {CTL} {action} {name} 失败: {e}"))?;
    if o.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&o.stderr).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&o.stdout).trim().to_string();
    }
    if text.is_empty() {
        text = format!("{CTL} {action} {name} 返回非零");
    }
    // 启动失败的 stderr 可能是几十 KB 的日志，截断以免响应膨胀
    Err(text.chars().take(2000).collect())
}

/// 列出系统所有服务。
pub(crate) fn list() -> Result<Vec<Row>, String> {
    #[cfg(target_os = "linux")]
    {
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
    #[cfg(not(target_os = "linux"))]
    {
        // OpenBSD：先列 daemon，再逐个问状态。服务规模通常只有几十个，开销可接受。
        let started = ls_set("started");
        let enabled = ls_set("on");
        Ok(ls_set("all")
            .into_iter()
            .map(|name| Row {
                active: if started.contains(&name) { "active" } else { "inactive" }.into(),
                load: if enabled.contains(&name) { "enabled" } else { "disabled" }.into(),
                sub: String::new(),
                description: String::new(),
                name,
            })
            .collect())
    }
}

/// `rcctl ls <all|on|started>` 的输出 → daemon 名集合。
///
/// 输出可能一行一个、也可能一行多个（空白分隔），两种都容忍。
#[cfg(not(target_os = "linux"))]
fn ls_set(kind: &str) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let Ok(o) = root_cmd(CTL).args(["ls", kind]).output() else {
        return set;
    };
    for line in String::from_utf8_lossy(&o.stdout).lines() {
        for word in line.split_whitespace() {
            if !word.is_empty() {
                set.insert(word.to_string());
            }
        }
    }
    set
}
