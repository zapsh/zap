//! CLI 侧的服务管理后端（systemctl / service+sysrc / rcctl）。
//!
//! 与服务端 `zapexec::verbs::svc` 语义一致，这里是精简版：只需要「查状态、执行动作」，
//! 不需要列举全量服务。之所以不复用服务端那份，是因为两者的执行器不同——服务端走
//! `root_cmd`，CLI 用普通 `std::process::Command`。
//!
//! 四个受支持平台的命令对照（macOS 等其余平台一律 unsupported）：
//!
//! | 平台           | 启停                | 开机开关                 | 状态查询           |
//! |----------------|---------------------|--------------------------|--------------------|
//! | Linux          | systemctl start     | systemctl enable         | is-active          |
//! | FreeBSD        | service X onestart  | sysrc X_enable=YES       | service X onestatus|
//! | OpenBSD        | rcctl start         | rcctl enable             | rcctl check        |
//!
//! FreeBSD 有两处与其他平台不同的地方，修改时容易踩：
//! 1. 开关和运行控制不是一个程序（`service` 只管启停，开机自启要写 rc.conf，用 `sysrc`）；
//! 2. `service X start` 在 rc.conf 没设 `X_enable` 时会直接拒绝执行，所以启停一律用
//!    `one` 前缀（`onestart`），语义才与 systemctl 一致。

use std::process::Command as ProcessCommand;

/// 运行控制命令。
#[cfg(target_os = "linux")]
const CTL: &str = "systemctl";

/// 同 `CTL`，FreeBSD —— 它既没有 systemctl 也没有 rcctl。
#[cfg(target_os = "freebsd")]
const CTL: &str = "service";

/// 同 `CTL`，OpenBSD —— 它有 rcctl。
#[cfg(target_os = "openbsd")]
const CTL: &str = "rcctl";

/// 兜底值：macOS 等不在支持范围内的平台，仅为通过类型检查。运行期 [`supported()`]
/// 恒为 false，命令不会真的被执行。
#[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd")))]
const CTL: &str = "rcctl";

/// 开机自启开关命令。
///
/// 只有 FreeBSD 把它与运行控制分成了两个程序：`service` 管启停，`sysrc` 管 rc.conf
/// 里的 `xxx_enable`。其余平台两个动作都是同一个命令。
#[cfg(target_os = "freebsd")]
const ENABLE_CTL: &str = "sysrc";
#[cfg(not(target_os = "freebsd"))]
const ENABLE_CTL: &str = CTL;

/// 当前平台的自动服务管理是否受支持。
///
/// 只覆盖 Linux(systemd)、FreeBSD(service+sysrc)、OpenBSD(rcctl)。
/// macOS 用 launchctl、DragonFly 又是另一套，都不在范围内——在那些平台上本函数
/// 返回 false，上层应降级为「需手动操作」。
pub fn supported() -> bool {
    cfg!(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd"
    ))
}

/// 平台不支持自动服务管理时的统一提示。
fn unsupported_msg() -> String {
    format!(
        "当前平台（{}）不支持自动服务管理，请手动操作",
        std::env::consts::OS
    )
}

/// 服务名：Linux 用 `<name>.service`，BSD 直接用 daemon 名。
#[cfg(target_os = "linux")]
pub fn unit_name(base: &str) -> String {
    format!("{base}.service")
}

/// 同 [`unit_name`]，BSD 直接用 daemon 名。
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
        // FreeBSD 必须带 one 前缀，否则 rc.conf 未开启时 service 会直接拒绝
        #[cfg(target_os = "freebsd")]
        {
            run(&["onestatus", unit])
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        // rcctl check：在跑返回 0，没跑或不存在都非零
        #[cfg(not(target_os = "freebsd"))]
        {
            run(&["check", unit])
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
    }
}

/// 执行服务动作：start / stop / restart / enable / disable。
pub fn act(verb: &str, unit: &str) -> Result<(), String> {
    // FreeBSD 的开关与启停分属两个命令，且启停要加 one 前缀
    #[cfg(target_os = "freebsd")]
    {
        match verb {
            "enable" => return act_with(ENABLE_CTL, &[&format!("{unit}_enable=YES")]),
            "disable" => return act_with(ENABLE_CTL, &[&format!("{unit}_enable=NO")]),
            _ => return act_with(CTL, &[&format!("one{verb}"), unit]),
        }
    }
    #[cfg(not(target_os = "freebsd"))]
    {
        // 其余平台开关与启停是同一个命令（ENABLE_CTL 即 CTL），参数形式一致
        match verb {
            "enable" | "disable" => act_with(ENABLE_CTL, &[verb, unit]),
            _ => act_with(CTL, &[verb, unit]),
        }
    }
}

/// `zapctl service status` 需要的一组字段，语义沿用 systemd 的 show 属性。
pub struct State {
    /// `loaded` / `not-found`
    pub load: String,
    /// `active` / `inactive` / `failed`
    pub active: String,
    /// 细分状态（如 running）；BSD 上无对应，恒为空
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
        if !supported() {
            return Err(unsupported_msg());
        }
        // BSD 只有「在跑 / 没跑」两态，细分状态字段留空
        Ok(State {
            load: if script_exists(unit) {
                "loaded".to_string()
            } else {
                "not-found".to_string()
            },
            active: if is_active(unit) {
                "active".to_string()
            } else {
                "inactive".to_string()
            },
            sub: String::new(),
            enabled: if is_enabled(unit) {
                "enabled".to_string()
            } else {
                "disabled".to_string()
            },
            pid: std::fs::read_to_string(format!("/var/run/{unit}.pid"))
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
        })
    }
}

/// rc.d 脚本是否已部署。
///
/// FreeBSD 的第三方服务脚本在 `/usr/local/etc/rc.d/`（系统自带的才在 `/etc/rc.d`），
/// OpenBSD 一律在 `/etc/rc.d/`。
#[cfg(not(target_os = "linux"))]
fn script_exists(unit: &str) -> bool {
    let mut dirs = vec!["/etc/rc.d"];
    #[cfg(target_os = "freebsd")]
    dirs.push("/usr/local/etc/rc.d");
    dirs.iter()
        .any(|d| std::path::Path::new(d).join(unit).exists())
}

/// 服务是否开机自启。
#[cfg(not(target_os = "linux"))]
fn is_enabled(unit: &str) -> bool {
    // sysrc -n 只输出值（"YES" / "NO" / 空）；变量未设置时返回非零
    #[cfg(target_os = "freebsd")]
    {
        run_cmd(ENABLE_CTL, &["-n", &format!("{unit}_enable")])
            .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "YES")
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "freebsd"))]
    {
        ls_set("on").contains(&unit.to_string())
    }
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

/// 仅 Linux 的 `systemctl show` 解析用到（BSD 侧走另一套查询），限死平台以免
/// 在其它目标上产生 dead_code 警告。
#[cfg(target_os = "linux")]
fn capture(args: &[&str]) -> Result<String, String> {
    let o = run(args)?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// `rcctl ls <all|on>` 的输出 → daemon 名集合（容忍一行多个）。
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
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
