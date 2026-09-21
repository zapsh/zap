//! CLI 侧的服务管理后端（systemctl）。
//!
//! 与服务端 `zapexec::verbs::svc` 语义一致，这里是精简版：只需要「查状态、执行动作」，
//! 不需要列举全量服务。之所以不复用服务端那份，是因为两者的执行器不同——服务端走
//! `root_cmd`，CLI 用普通 `std::process::Command`。
//!
//! 命令对照（全部走 systemctl）：
//!
//! | 语义       | 命令                    |
//! |------------|-------------------------|
//! | 启停       | `systemctl start|stop`  |
//! | 开机开关   | `systemctl enable|disable` |
//! | 状态查询   | `is-active` / `show`    |
//!
//! 非 Linux 平台 [`supported()`] 恒为 false：[`run_cmd`] 会在执行前统一拦掉，
//! 调用方拿到的都是「失败 / 未知」。

use std::process::Command as ProcessCommand;

/// 服务管理器命令（systemd）。
const CTL: &str = "systemctl";

/// 当前平台的自动服务管理是否受支持（Linux + systemd）。
///
/// macOS 用 launchctl，不在支持范围内——在那些平台上本函数返回 false，上层应
/// 降级为「需手动操作」。
pub fn supported() -> bool {
    cfg!(target_os = "linux")
}

/// 平台不支持自动服务管理时的统一提示。
fn unsupported_msg() -> String {
    format!(
        "当前平台（{}）不支持自动服务管理，请手动操作",
        std::env::consts::OS
    )
}

/// 服务名：`<name>.service`。
pub fn unit_name(base: &str) -> String {
    format!("{base}.service")
}

/// [`unit_name`] 的逆操作，用于展示。
pub fn display_name(unit: &str) -> &str {
    unit.trim_end_matches(".service")
}

/// 服务是否在运行。
pub fn is_active(unit: &str) -> bool {
    run(&["is-active", "--quiet", unit])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 执行服务动作：start / stop / restart / enable / disable。
pub fn act(verb: &str, unit: &str) -> Result<(), String> {
    act_with(CTL, &[verb, unit])
}

/// `zapctl service status` 需要的一组字段，语义沿用 systemd 的 show 属性。
pub struct State {
    /// `loaded` / `not-found`
    pub load: String,
    /// `active` / `inactive` / `failed`
    pub active: String,
    /// 细分状态（如 running）
    pub sub: String,
    /// `enabled` / `disabled`
    pub enabled: String,
    /// 主进程 pid，未知时为空
    pub pid: String,
}

pub fn state_of(unit: &str) -> Result<State, String> {
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

/// 执行动作并翻译失败信息。
fn act_with(cmd: &str, args: &[&str]) -> Result<(), String> {
    let o = run_cmd(cmd, args)?;
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
        text = format!("{cmd} {} 失败（退出码 {code}）", args.join(" "));
    }
    // 启动失败的 stderr 可能是几十 KB 的日志，截断以免刷屏
    Err(text.chars().take(2000).collect())
}

fn run(args: &[&str]) -> Result<std::process::Output, String> {
    run_cmd(CTL, args)
}

fn run_cmd(cmd: &str, args: &[&str]) -> Result<std::process::Output, String> {
    // 统一入口：不支持的平台在这里就拦掉，调用方拿到的都是「失败/未知」
    if !supported() {
        return Err(unsupported_msg());
    }
    ProcessCommand::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("无法执行 {cmd} {}: {e}", args.join(" ")))
}

/// `systemctl show` 的输出（失败时给 stderr）。
fn capture(args: &[&str]) -> Result<String, String> {
    let o = run(args)?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}
