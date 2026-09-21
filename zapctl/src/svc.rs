//! CLI 侧的服务管理后端（systemctl / rcctl）。
//!
//! 与服务端 `zapexec::verbs::svc` 语义一致，这里是精简版：只需要「查状态、执行动作」，
//! 不需要列举全量服务。之所以不复用服务端那份，是因为两者的执行器不同——服务端走
//! `root_cmd`，CLI 用普通 `std::process::Command`。

use std::process::Command as ProcessCommand;

#[cfg(target_os = "linux")]
const CTL: &str = "systemctl";

/// 同 `CTL`，非 Linux 平台。
#[cfg(not(target_os = "linux"))]
const CTL: &str = "rcctl";

/// 服务名：Linux 用 `<name>.service`，OpenBSD 直接用 daemon 名。
#[cfg(target_os = "linux")]
pub fn unit_name(base: &str) -> String {
    format!("{base}.service")
}

/// 同 [`unit_name`]，非 Linux 平台。
#[cfg(not(target_os = "linux"))]
pub fn unit_name(base: &str) -> String {
    base.to_string()
}

/// [`unit_name`] 的逆操作，用于展示。
pub fn display_name(unit: &str) -> &str {
    unit.trim_end_matches(".service")
}

/// 服务是否在运行。
pub fn is_active(unit: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        run(&["is-active", "--quiet", unit])
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // rcctl check：在跑返回 0，没跑或不存在都非零
        run(&["check", unit])
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

/// 执行服务动作：start / stop / restart / enable / disable。
pub fn act(verb: &str, unit: &str) -> Result<(), String> {
    let o = run(&[verb, unit])?;
    if o.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&o.stderr).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&o.stdout).trim().to_string();
    }
    if text.is_empty() {
        let code = o
            .status
            .code()
            .map_or_else(|| "?".to_string(), |c| c.to_string());
        text = format!("{CTL} {verb} {unit} 失败（退出码 {code}）");
    }
    Err(text.chars().take(2000).collect())
}

/// `zapctl service status` 需要的一组字段，语义沿用 systemd 的 show 属性。
pub struct State {
    /// `loaded` / `not-found`
    pub load: String,
    /// `active` / `inactive` / `failed`
    pub active: String,
    /// 细分状态（如 running）；OpenBSD 上无对应，恒为空
    pub sub: String,
    /// `enabled` / `disabled`
    pub enabled: String,
    /// 主进程 pid，未知时为空
    pub pid: String,
}

pub fn state_of(unit: &str) -> Result<State, String> {
    #[cfg(target_os = "linux")]
    {
        use std::collections::HashMap;
        let out = capture(&[
            "show",
            "-p",
            "LoadState",
            "-p",
            "ActiveState",
            "-p",
            "SubState",
            "-p",
            "UnitFileState",
            "-p",
            "MainPID",
            unit,
        ])?;
        let mut map: HashMap<String, String> = HashMap::new();
        for line in out.lines() {
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        Ok(State {
            load: map.remove("LoadState").unwrap_or_default(),
            active: map.remove("ActiveState").unwrap_or_default(),
            sub: map.remove("SubState").unwrap_or_default(),
            enabled: map.remove("UnitFileState").unwrap_or_default(),
            pid: map.remove("MainPID").unwrap_or_default(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        // OpenBSD 只有「在跑 / 没跑」两态，细分状态字段留空
        let load = if ls_set("all").contains(&unit.to_string()) {
            "loaded"
        } else {
            "not-found"
        };
        let active = if is_active(unit) { "active" } else { "inactive" };
        let enabled = if ls_set("on").contains(&unit.to_string()) {
            "enabled"
        } else {
            "disabled"
        };
        let pid = std::fs::read_to_string(format!("/var/run/{unit}.pid"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        Ok(State {
            load: load.to_string(),
            active: active.to_string(),
            sub: String::new(),
            enabled: enabled.to_string(),
            pid,
        })
    }
}

fn run(args: &[&str]) -> Result<std::process::Output, String> {
    ProcessCommand::new(CTL)
        .args(args)
        .output()
        .map_err(|e| format!("无法执行 {CTL} {}: {e}", args.join(" ")))
}

fn capture(args: &[&str]) -> Result<String, String> {
    let o = run(args)?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// `rcctl ls <all|on>` 的输出 → daemon 名集合（容忍一行多个）。
#[cfg(not(target_os = "linux"))]
fn ls_set(kind: &str) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let Ok(o) = ProcessCommand::new(CTL).args(["ls", kind]).output() else {
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
