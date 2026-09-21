//! 服务管理后端：把 systemd、FreeBSD 的 service+sysrc、OpenBSD 的 rcctl
//! 三者差异收敛在这里。
//!
//! 动作动词在各平台上一致（`start` / `stop` / `restart` / `reload` / `enable` /
//! `disable`），差异集中在查询类与控制器的就绪判断：
//!
//! | 语义           | systemd                       | FreeBSD                          | OpenBSD                    |
//! |----------------|-------------------------------|----------------------------------|----------------------------|
//! | 服务是否存在   | `list-unit-files <n>.service` | rc.d 脚本文件是否存在            | `ls all` 里能查到该 daemon |
//! | 是否运行中     | `is-active [-q] <n>`          | `service <n> onestatus`          | `check <n>`                |
//! | 是否开机自启   | `is-enabled [-q] <n>`         | `sysrc -n <n>_enable` 为 YES     | `ls on` 里能查到该 daemon  |
//! | 管理器是否就绪 | `is-system-running`           | 恒为真（无中央管理器可挂）       | 同上                       |
//! | 列出所有服务   | `list-units --type=service`   | 扫 rc.d 目录 + 逐个 `onestatus`  | `ls all` + 逐个 `check`    |
//!
//! FreeBSD 另有两个坑，改动时容易踩：启停必须带 `one` 前缀（否则 rc.conf 未开启
//! `n_enable` 时 `service` 会直接拒绝执行），且开机自启归 `sysrc` 管而不归
//! `service` 管。
//!
//! **服务名的平台差异不在这里处理**：nginx / sshd / php-fpm 在各发行版、各 BSD 上叫法
//! 不同（Debian 的 `ssh` vs RHEL 的 `sshd`、OpenBSD 的 `php82_fpm`），那是各动词自己的
//! 业务知识——它们按候选名依次探测，本层只负责"给定一个名字去操作"。

use super::root_cmd;

/// 服务管理器命令（运行控制）。
#[cfg(target_os = "linux")]
pub(crate) const CTL: &str = "systemctl";

/// 同 [`CTL`]，FreeBSD —— 它既没有 systemctl 也没有 rcctl。
#[cfg(target_os = "freebsd")]
pub(crate) const CTL: &str = "service";

/// 同 [`CTL`]，OpenBSD —— 它有 rcctl。
#[cfg(target_os = "openbsd")]
pub(crate) const CTL: &str = "rcctl";

/// 兜底值：macOS 等不在支持范围内的平台，仅为通过类型检查。运行期
/// [`supported()`] 恒为 false，所有服务操作都会在调用前返回「不支持」，
/// 不会真的去执行这个不存在的命令。
#[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd")))]
pub(crate) const CTL: &str = "rcctl";

/// 开机自启开关命令。
///
/// 只有 FreeBSD 把它与运行控制分成了两个程序：`service` 管启停，`sysrc` 管
/// rc.conf 里的 `xxx_enable`。其余平台两个动作共用同一个命令。
#[cfg(target_os = "freebsd")]
pub(crate) const ENABLE_CTL: &str = "sysrc";
#[cfg(not(target_os = "freebsd"))]
pub(crate) const ENABLE_CTL: &str = CTL;

/// 当前平台的自动服务管理是否受支持。
///
/// 只覆盖 Linux(systemd)、FreeBSD(service+sysrc)、OpenBSD(rcctl)。
/// macOS 用 launchctl、DragonFly 又是另一套，都不在范围内——在那些平台上本函数
/// 返回 false，上层应降级为「需手动操作」而不是报命令找不到。
pub(crate) const fn supported() -> bool {
    cfg!(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd"
    ))
}

/// rc.d 脚本目录。FreeBSD 的第三方服务装在 `/usr/local/etc/rc.d/`（系统自带的
/// 才在 `/etc/rc.d/`），OpenBSD 一律在 `/etc/rc.d/`。
#[cfg(target_os = "freebsd")]
fn rcd_dirs() -> Vec<&'static str> {
    vec!["/etc/rc.d", "/usr/local/etc/rc.d"]
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

/// 服务是否存在（unit 已注册 / daemon 已登记）。
pub(crate) fn exists(name: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
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
    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(target_os = "freebsd")]
        {
            rcd_dirs()
                .iter()
                .any(|d| std::path::Path::new(d).join(name).exists())
        }
        #[cfg(not(target_os = "freebsd"))]
        {
            ls_set("all").contains(&name.to_string())
        }
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
        if !supported() {
            return false;
        }
        // FreeBSD 必须带 one 前缀，否则 rc.conf 未开启时 service 直接拒绝执行；
        // rcctl check 则是「在跑返回 0，没跑或不存在都返回非 0」
        #[cfg(target_os = "freebsd")]
        let args = ["onestatus", name];
        #[cfg(not(target_os = "freebsd"))]
        let args = ["check", name];
        root_cmd(CTL)
            .args(args)
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
        #[cfg(target_os = "freebsd")]
        {
            // sysrc -n 只输出值（YES / NO / 空）；变量未设置时返回非零
            root_cmd(ENABLE_CTL)
                .args(["-n", &format!("{name}_enable")])
                .output()
                .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "YES")
                .unwrap_or(false)
        }
        #[cfg(not(target_os = "freebsd"))]
        {
            ls_set("on").contains(&name.to_string())
        }
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
        if !supported() {
            return false;
        }
        // OpenBSD 没有常驻管理器：rc.d 脚本由 rc 直接调用，谈不上"管理器未就绪"
        true
    }
}

/// 执行服务动作：`start` / `stop` / `restart` / `reload` / `enable` / `disable`。
/// 失败时返回 stderr（为空则回落 stdout）。
pub(crate) fn act(action: &str, name: &str) -> Result<(), String> {
    if !supported() {
        return Err(unsupported_msg());
    }
    // FreeBSD 的开关与启停分属两个命令，且启停必须带 one 前缀
    #[cfg(target_os = "freebsd")]
    {
        match action {
            "enable" => return act_with(ENABLE_CTL, &[&format!("{name}_enable=YES")]),
            "disable" => return act_with(ENABLE_CTL, &[&format!("{name}_enable=NO")]),
            _ => return act_with(CTL, &[&format!("one{action}"), name]),
        }
    }
    #[cfg(not(target_os = "freebsd"))]
    {
        // 其余平台开关与启停是同一个命令（ENABLE_CTL 即 CTL），参数形式一致
        match action {
            "enable" | "disable" => act_with(ENABLE_CTL, &[action, name]),
            _ => act_with(CTL, &[action, name]),
        }
    }
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
        if !supported() {
            return Err(unsupported_msg());
        }
        #[cfg(target_os = "freebsd")]
        {
            // 扫两个 rc.d 目录拿服务名，再逐个问状态。规模通常只有几十个，开销可接受。
            let mut names: Vec<String> = Vec::new();
            for dir in rcd_dirs() {
                let Ok(rd) = std::fs::read_dir(dir) else {
                    continue;
                };
                for entry in rd.flatten() {
                    let Some(n) = entry.file_name().to_str().map(str::to_string) else {
                        continue;
                    };
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
            }
            names.sort();
            Ok(names
                .into_iter()
                .map(|name| Row {
                    load: if is_enabled(&name) {
                        "enabled"
                    } else {
                        "disabled"
                    }
                    .into(),
                    active: if is_active(&name) {
                        "active"
                    } else {
                        "inactive"
                    }
                    .into(),
                    sub: String::new(),
                    description: String::new(),
                    name,
                })
                .collect())
        }
        #[cfg(not(target_os = "freebsd"))]
        {
            // OpenBSD：rcctl 一次给出三类集合，省掉逐个查询
            let started = ls_set("started");
            let enabled = ls_set("on");
            Ok(ls_set("all")
                .into_iter()
                .map(|name| Row {
                    active: if started.contains(&name) {
                        "active"
                    } else {
                        "inactive"
                    }
                    .into(),
                    load: if enabled.contains(&name) {
                        "enabled"
                    } else {
                        "disabled"
                    }
                    .into(),
                    sub: String::new(),
                    description: String::new(),
                    name,
                })
                .collect())
        }
    }
}

/// `rcctl ls <all|on|started>` 的输出 → daemon 名集合。
///
/// 输出可能一行一个、也可能一行多个（空白分隔），两种都容忍。
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn ls_set(kind: &str) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    // 不支持的平台直接给空集：调用方（exists / is_enabled）据此返回 false
    if !supported() {
        return set;
    }
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
