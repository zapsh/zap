//! 防火墙管理（root 执行，白名单动词，不执行任何来自面板的任意命令）。
//!
//! 多系统抽象：对外只有「状态 / 放行 / 拒绝 / 删除 / 启停」五种语义，
//! 内部按后端翻译成具体命令。当前实现 Linux 四种后端：
//!
//! ```text
//! firewalld  firewall-cmd [--permanent] --add-port / --add-rich-rule
//! ufw        ufw allow|deny ... comment / ufw --force delete <num>
//! nftables   nft（面板自管 inet zap_fw 表，避免改动系统既有规则集）
//! iptables   iptables -I/-D INPUT（按行号）
//! ```
//!
//! 后端选择：正在运行的优先；都没运行时按 firewalld > ufw > nftables > iptables
//! 取已安装的那个，避免把规则写到没启用的后端上。

use serde_json::{Value, json};

use super::root_cmd;
use zap_proto::Response;

// ── 后端探测 ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Backend {
    Firewalld,
    Ufw,
    Nftables,
    Iptables,
    Unsupported,
}

impl Backend {
    fn name(self) -> &'static str {
        match self {
            Backend::Firewalld => "firewalld",
            Backend::Ufw => "ufw",
            Backend::Nftables => "nftables",
            Backend::Iptables => "iptables",
            Backend::Unsupported => "none",
        }
    }

    /// 对应 systemd 服务名（ufw 也是 systemd 单元；iptables/nft 无服务时返回空）
    fn service(self) -> Option<&'static str> {
        match self {
            Backend::Firewalld => Some("firewalld"),
            Backend::Ufw => Some("ufw"),
            Backend::Nftables => Some("nftables"),
            Backend::Iptables => Some("iptables"),
            _ => None,
        }
    }
}

fn cmd_ok(program: &str, args: &[&str]) -> bool {
    root_cmd(program)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn cmd_out(program: &str, args: &[&str]) -> String {
    root_cmd(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn has_cmd(name: &str) -> bool {
    cmd_ok("which", &[name])
}

fn service_active(name: &str) -> bool {
    super::svc::is_active(name)
}

fn service_enabled(name: &str) -> bool {
    super::svc::is_enabled(name)
}

/// 探测结果：`running` = 该后端确实在过滤流量；`installed` = 只是装了命令，并未生效
fn detect_kind() -> (Backend, &'static str) {
    // 1) 真正在生效的优先（避免写到没启用的后端上）
    //    顺序与兜底一致：nftables 排在 iptables 之后 —— 多数发行版实际通过
    //    iptables / iptables-nft 管理规则，iptables 命令才是用户认可的入口。
    for b in [
        Backend::Firewalld,
        Backend::Ufw,
        Backend::Iptables,
        Backend::Nftables,
    ] {
        if backend_active(b) {
            return (b, "running");
        }
    }
    // 2) 都没生效：按 firewalld > ufw > iptables > nftables 取已安装的命令。
    //    iptables 排在 nftables 之前：多数发行版默认走 iptables / iptables-nft，
    //    而我们对 nftables 只写自建的 inet zap_fw 表，若系统本身没用 nft 则规则形同虚设。
    if has_cmd("firewall-cmd") {
        return (Backend::Firewalld, "installed");
    }
    if has_cmd("ufw") {
        return (Backend::Ufw, "installed");
    }
    if has_cmd("iptables") {
        return (Backend::Iptables, "installed");
    }
    if has_cmd("nft") {
        return (Backend::Nftables, "installed");
    }
    (Backend::Unsupported, "none")
}

fn detect() -> Backend {
    detect_kind().0
}

// ── 统一规则结构 ────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Rule {
    /// 各后端自解释的删除标识（firewalld=port:80/tcp 或 rich:<rule>；ufw=序号；
    /// nftables=handle；iptables=行号）
    id: String,
    port: u16,
    proto: String,
    action: String,
    source: String,
    comment: String,
}

fn rule_json(r: &Rule, protected: bool) -> Value {
    json!({
        "id": r.id,
        "port": r.port,
        "proto": r.proto,
        "action": r.action,
        "source": r.source,
        "comment": r.comment,
        "protected": protected,
    })
}

// ── 各后端实现 ──────────────────────────────────────────────

fn firewalld_rules() -> Vec<Rule> {
    let mut out = Vec::new();
    for tok in cmd_out("firewall-cmd", &["--list-ports"]).split_whitespace() {
        let t = tok.trim().to_string();
        if let Some((p, proto)) = t.split_once('/') {
            out.push(Rule {
                id: format!("port:{t}"),
                port: p.parse().unwrap_or(0),
                proto: proto.to_string(),
                action: "accept".to_string(),
                source: String::new(),
                comment: String::new(),
            });
        }
    }
    for line in cmd_out("firewall-cmd", &["--list-rich-rules"]).lines() {
        let raw = line.trim().trim_matches('\'').to_string();
        if raw.is_empty() {
            continue;
        }
        let port = raw
            .split("port port=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let proto = raw
            .split("protocol=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("tcp")
            .to_string();
        let source = raw
            .split("source address=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("")
            .to_string();
        let action = if raw.contains("reject") || raw.contains("drop") {
            "drop"
        } else {
            "accept"
        }
        .to_string();
        out.push(Rule {
            id: format!("rich:{raw}"),
            port,
            proto,
            action,
            source,
            comment: String::new(),
        });
    }
    out
}

fn ufw_rules() -> Vec<Rule> {
    // 形如：[ 1] 80/tcp                     ALLOW IN    Anywhere
    let mut out = Vec::new();
    for line in cmd_out("ufw", &["status", "numbered"]).lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix('[') else {
            continue;
        };
        let Some((num, body)) = rest.split_once(']') else {
            continue;
        };
        let num = num.trim().to_string();
        if num.is_empty() {
            continue;
        }
        let body = body.trim();
        let action = if body.contains("DENY") || body.contains("REJECT") {
            "drop"
        } else {
            "accept"
        }
        .to_string();
        // 第一个字段通常是 80/tcp 或 80
        let first = body.split_whitespace().next().unwrap_or("");
        let (port, proto) = match first.split_once('/') {
            Some((p, pr)) => (p.parse().unwrap_or(0), pr.to_string()),
            None => (first.parse().unwrap_or(0), "tcp".to_string()),
        };
        let source = body
            .split(" from ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("")
            .to_string();
        let source = if source == "Anywhere" {
            String::new()
        } else {
            source
        };
        out.push(Rule {
            id: num,
            port,
            proto,
            action,
            source,
            comment: String::new(),
        });
    }
    out
}

fn nft_rules() -> Vec<Rule> {
    // 面板自管链：inet zap_fw input（不存在时视为空）
    let mut out = Vec::new();
    for line in cmd_out("nft", &["-a", "list", "chain", "inet", "zap_fw", "input"]).lines() {
        let l = line.trim();
        if !l.contains("dport") {
            continue;
        }
        let handle = l
            .split("# handle ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("")
            .to_string();
        let port = l
            .split("dport ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let proto = if l.contains("udp dport") {
            "udp"
        } else {
            "tcp"
        }
        .to_string();
        let action = if l.contains("accept") {
            "accept"
        } else {
            "drop"
        }
        .to_string();
        let source = l
            .split("saddr ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("")
            .to_string();
        out.push(Rule {
            id: handle,
            port,
            proto,
            action,
            source,
            comment: String::new(),
        });
    }
    out
}

fn iptables_rules() -> Vec<Rule> {
    // 只看 INPUT 链里带 dport 的规则，取行号作为删除 id
    let mut out = Vec::new();
    for line in cmd_out("iptables", &["-L", "INPUT", "-n", "--line-numbers"]).lines() {
        let l = line.trim();
        if !l.contains("dpt:") {
            continue;
        }
        let mut it = l.split_whitespace();
        let num = it.next().unwrap_or("").to_string();
        if !num.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let action = it.next().unwrap_or("").to_lowercase();
        let action = if action.contains("accept") {
            "accept"
        } else {
            "drop"
        }
        .to_string();
        let proto = if l.contains(" udp ") { "udp" } else { "tcp" }.to_string();
        let port = l
            .split("dpt:")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let source = it.next().unwrap_or("0.0.0.0/0").to_string();
        let source = if source == "0.0.0.0/0" {
            String::new()
        } else {
            source
        };
        out.push(Rule {
            id: num,
            port,
            proto,
            action,
            source,
            comment: String::new(),
        });
    }
    out
}

/// nft 规则集是否真的有内容（仅用于展示与排障，不作为后端判定依据）。
/// 注意：`nft list ruleset` 在空规则集时也返回 0（输出为空），
/// 只看退出码会把「装了 nft 但没用」误判成 nftables 生效（WSL 上尤其常见）。
fn nft_has_ruleset() -> bool {
    !cmd_out("nft", &["list", "ruleset"]).trim().is_empty()
}

/// 状态接口里回传的诊断信息：帮助判断"为什么选了这个后端"。
fn diag() -> Value {
    json!({
        "nft_ruleset": nft_has_ruleset(),
        "nftables_service_active": service_active("nftables"),
        "nftables_service_enabled": service_enabled("nftables"),
        "iptables_rules": iptables_has_rules(),
        "firewalld_running": cmd_out("firewall-cmd", &["--state"]).trim() == "running",
        "ufw_active": cmd_out("ufw", &["status"]).contains("Status: active"),
        "systemd": super::svc::manager_running(),
    })
}

/// iptables 是否存在真实规则（`-P` 只是链默认策略，不算规则）。
fn iptables_has_rules() -> bool {
    cmd_out("iptables", &["-S"])
        .lines()
        .any(|l| l.trim_start().starts_with("-A") || l.trim_start().starts_with("-I"))
}

/// WSL：使用共享内核，iptables / nft 规则可能不生效或仅当前会话有效，需要显式提示。
fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|s| {
            let s = s.to_lowercase();
            s.contains("microsoft") || s.contains("wsl")
        })
        .unwrap_or(false)
}

/// 后端是否真正在生效。
/// 注意不能只看 systemd：ufw.service 在 Debian/Ubuntu 上是
/// `Type=oneshot + RemainAfterExit`，ufw 实际关闭时 systemd 仍显示 active，
/// 必须以各后端自己的查询命令为准。
/// nftables 是否真正在管理防火墙，需同时满足三个条件：
///
/// 1. 服务 active 且 enabled（开机自启）——它是 oneshot 加载型服务，
///    单纯 `active`（RemainAfterExit）不代表在管防火墙；
/// 2. 规则集里确实有规则（空规则集 = 装了没用，WSL / 桌面发行版很常见）；
/// 3. `nft list ruleset` 有内容本身也不能作数（iptables-nft、Docker、libvirt 都会写表）。
fn nftables_in_control() -> bool {
    service_active("nftables") && service_enabled("nftables") && nft_has_ruleset()
}

fn backend_active(b: Backend) -> bool {
    match b {
        Backend::Firewalld => cmd_out("firewall-cmd", &["--state"]).trim() == "running",
        Backend::Ufw => cmd_out("ufw", &["status"]).contains("Status: active"),
        Backend::Nftables => nftables_in_control(),
        // iptables 没有守护进程：存在真实规则才算生效（命令可用 ≠ 在过滤流量）
        Backend::Iptables => iptables_has_rules(),
        Backend::Unsupported => false,
    }
}

/// 后端是否开机自启。ufw 额外参考 /etc/ufw/ufw.conf 的 ENABLED=yes。
fn backend_enabled(b: Backend) -> bool {
    match b {
        Backend::Firewalld => service_enabled("firewalld"),
        Backend::Ufw => service_enabled("ufw") || ufw_conf_enabled(),
        Backend::Nftables => service_enabled("nftables"),
        Backend::Iptables => service_enabled("iptables") || service_enabled("netfilter-persistent"),
        Backend::Unsupported => false,
    }
}

/// /etc/ufw/ufw.conf 中 ENABLED=yes
fn ufw_conf_enabled() -> bool {
    std::fs::read_to_string("/etc/ufw/ufw.conf")
        .map(|s| {
            s.lines().any(|l| {
                let l = l.trim().to_lowercase();
                l.starts_with("enabled=") && l.contains("yes")
            })
        })
        .unwrap_or(false)
}

fn list_rules(b: Backend) -> Vec<Rule> {
    match b {
        Backend::Firewalld => firewalld_rules(),
        Backend::Ufw => ufw_rules(),
        Backend::Nftables => nft_rules(),
        Backend::Iptables => iptables_rules(),
        _ => Vec::new(),
    }
}

// ── 动作 ────────────────────────────────────────────────────

fn firewalld_apply(r: &Rule) -> Result<(), String> {
    if r.action == "accept" && r.source.is_empty() {
        let port = format!("{}/{}", r.port, r.proto);
        let (ok, out) = run_capture("firewall-cmd", &["--permanent", "--add-port", &port]);
        if !ok {
            return Err(format!("firewall-cmd --add-port {port} 执行失败：{out}"));
        }
    } else {
        let verb = if r.action == "accept" {
            "accept"
        } else {
            "reject"
        };
        let src = if r.source.is_empty() {
            String::new()
        } else {
            format!(" source address=\"{}\"", r.source)
        };
        let rule = format!(
            "rule family=\"ipv4\"{} port port=\"{}\" protocol=\"{}\" {}",
            src, r.port, r.proto, verb
        );
        let (ok, out) = run_capture("firewall-cmd", &["--permanent", "--add-rich-rule", &rule]);
        if !ok {
            return Err(format!("firewall-cmd --add-rich-rule 执行失败：{out}"));
        }
    }
    let (ok, out) = run_capture("firewall-cmd", &["--reload"]);
    if !ok {
        return Err(format!("firewall-cmd --reload 执行失败：{out}"));
    }
    Ok(())
}

/// 构造 ufw 参数（纯函数，便于单测）：
/// - 不限来源：`ufw allow|deny 80/tcp`（`ufw allow port 80 proto tcp` 是非法语法）
/// - 指定来源：`ufw allow|deny from 1.2.3.4 to any port 80 proto tcp`
/// - 可选备注：`... comment xxx`（空格替换为 _）
fn ufw_args(r: &Rule, with_comment: bool) -> Vec<String> {
    let verb = if r.action == "accept" {
        "allow"
    } else {
        "deny"
    };
    let mut args: Vec<String> = vec![verb.to_string()];
    if r.source.is_empty() {
        args.push(format!("{}/{}", r.port, r.proto));
    } else {
        args.extend([
            "from".to_string(),
            r.source.clone(),
            "to".to_string(),
            "any".to_string(),
            "port".to_string(),
            r.port.to_string(),
            "proto".to_string(),
            r.proto.clone(),
        ]);
    }
    if with_comment && !r.comment.is_empty() {
        args.push("comment".to_string());
        args.push(r.comment.replace(' ', "_"));
    }
    args
}

/// 执行命令并返回 (是否成功, stdout+stderr)，失败时把后端输出带回去便于排障。
fn run_capture(program: &str, args: &[&str]) -> (bool, String) {
    match root_cmd(program).args(args).output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stderr).to_string();
            text.push_str(&String::from_utf8_lossy(&o.stdout));
            (o.status.success(), text.trim().to_string())
        }
        Err(e) => (false, e.to_string()),
    }
}

fn ufw_apply(r: &Rule) -> Result<(), String> {
    // 先按带备注执行；部分 ufw 版本不接受 comment 时退化为不带备注重试
    let with_comment = !r.comment.is_empty();
    let args = ufw_args(r, with_comment);
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, out) = run_capture("ufw", &refs);
    if ok {
        return Ok(());
    }
    if with_comment {
        let args = ufw_args(r, false);
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (ok2, out2) = run_capture("ufw", &refs);
        if ok2 {
            return Ok(());
        }
        return Err(format!("ufw 执行失败（{}）：{}", args.join(" "), out2));
    }
    Err(format!("ufw 执行失败（{}）：{}", args.join(" "), out))
}

fn nft_apply(r: &Rule) -> Result<(), String> {
    // 面板自管表/链，避免污染系统既有规则集
    let _ = root_cmd("nft")
        .args(["add", "table", "inet", "zap_fw"])
        .output();
    let _ = root_cmd("nft")
        .args([
            "add",
            "chain",
            "inet",
            "zap_fw",
            "input",
            "{ type filter hook input priority 0 ; }",
        ])
        .output();
    let action = if r.action == "accept" {
        "accept"
    } else {
        "drop"
    };
    let mut expr = String::new();
    if !r.source.is_empty() {
        expr.push_str(&format!("ip saddr {} ", r.source));
    }
    expr.push_str(&format!("{} dport {} {}", r.proto, r.port, action));
    let out = root_cmd("nft")
        .args(["add", "rule", "inet", "zap_fw", "input", &expr])
        .output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "nft 执行失败: {}",
            String::from_utf8_lossy(&o.stderr)
        )),
        Err(e) => Err(format!("nft 执行失败: {e}")),
    }
}

fn iptables_apply(r: &Rule) -> Result<(), String> {
    let target = if r.action == "accept" {
        "ACCEPT"
    } else {
        "DROP"
    };
    let mut args: Vec<String> = vec![
        "-I".to_string(),
        "INPUT".to_string(),
        "1".to_string(),
        "-p".to_string(),
        r.proto.clone(),
        "--dport".to_string(),
        r.port.to_string(),
    ];
    if !r.source.is_empty() {
        args.push("-s".to_string());
        args.push(r.source.clone());
    }
    args.push("-j".to_string());
    args.push(target.to_string());
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, out) = run_capture("iptables", &refs);
    if !ok {
        return Err(format!("iptables 执行失败（{}）：{}", args.join(" "), out));
    }
    Ok(())
}

fn apply_rule(b: Backend, r: &Rule) -> Result<(), String> {
    match b {
        Backend::Firewalld => firewalld_apply(r),
        Backend::Ufw => ufw_apply(r),
        Backend::Nftables => nft_apply(r),
        Backend::Iptables => iptables_apply(r),
        _ => Err(format!(
            "当前系统（{}）暂不支持该操作",
            std::env::consts::OS
        )),
    }
}

fn delete_rule(b: Backend, id: &str) -> Result<(), String> {
    match b {
        Backend::Firewalld => {
            let ok = if let Some(port) = id.strip_prefix("port:") {
                cmd_ok("firewall-cmd", &["--permanent", "--remove-port", port])
            } else if let Some(rule) = id.strip_prefix("rich:") {
                cmd_ok("firewall-cmd", &["--permanent", "--remove-rich-rule", rule])
            } else {
                false
            };
            if !ok {
                return Err("firewall-cmd 删除规则失败".to_string());
            }
            if !cmd_ok("firewall-cmd", &["--reload"]) {
                return Err("firewall-cmd --reload 执行失败".to_string());
            }
            Ok(())
        }
        Backend::Ufw => {
            if cmd_ok("ufw", &["--force", "delete", id]) {
                Ok(())
            } else {
                Err("ufw 删除规则失败".to_string())
            }
        }
        Backend::Nftables => {
            let out = root_cmd("nft")
                .args(["delete", "rule", "inet", "zap_fw", "input", "handle", id])
                .output();
            match out {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(format!(
                    "nft 删除失败: {}",
                    String::from_utf8_lossy(&o.stderr)
                )),
                Err(e) => Err(format!("nft 删除失败: {e}")),
            }
        }
        Backend::Iptables => {
            if cmd_ok("iptables", &["-D", "INPUT", id]) {
                Ok(())
            } else {
                Err("iptables 删除规则失败".to_string())
            }
        }
        _ => Err(format!(
            "当前系统（{}）暂不支持该操作",
            std::env::consts::OS
        )),
    }
}

/// 启停 / 开机自启。ufw 走自己的 `ufw enable|disable`，
/// 其余后端走 systemd（否则会出现 systemd 显示 active 而 ufw 实际关闭的错位）。
fn toggle_service(b: Backend, action: &str) -> Result<(), String> {
    match (b, action) {
        (Backend::Ufw, "start") => {
            if cmd_ok("ufw", &["--force", "enable"]) {
                Ok(())
            } else {
                Err("ufw enable 执行失败".to_string())
            }
        }
        (Backend::Ufw, "stop") => {
            if cmd_ok("ufw", &["disable"]) {
                Ok(())
            } else {
                Err("ufw disable 执行失败".to_string())
            }
        }
        _ => {
            let Some(svc) = b.service() else {
                return Err(format!("后端 {} 无对应服务可{}", b.name(), action));
            };
            super::svc::act(action, svc).map_err(|e| format!("{action} {svc} 执行失败：{e}"))
        }
    }
}

// ── 对外的纯函数（便于单测） ────────────────────────────────

/// 规则是否命中面板自身端口（受保护，不允许拒绝/删除导致把自己锁在外面）
pub(super) fn hits_panel_port(rule_port: u16, panel_port: u16) -> bool {
    panel_port > 0 && rule_port == panel_port
}

fn validate_rule_input(port: u16, proto: &str, action: &str, source: &str) -> Result<(), String> {
    if port == 0 {
        return Err("端口必须在 1 - 65535 之间".to_string());
    }
    if !matches!(proto, "tcp" | "udp") {
        return Err("协议仅支持 tcp / udp".to_string());
    }
    if !matches!(action, "accept" | "drop") {
        return Err("动作仅支持 accept / drop".to_string());
    }
    validate_source(source)?;
    Ok(())
}

/// 来源校验：空串（不限）或单个 IPv4/IPv6 地址 / CIDR
fn validate_source(source: &str) -> Result<(), String> {
    let s = source.trim();
    if s.is_empty() {
        return Ok(());
    }
    if s.chars()
        .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '/')))
    {
        return Err("来源地址不合法（支持 IP 或 CIDR）".to_string());
    }
    let addr = s.split('/').next().unwrap_or("");
    if addr.parse::<std::net::IpAddr>().is_err() {
        return Err("来源地址不合法（支持 IP 或 CIDR）".to_string());
    }
    if let Some((_, mask)) = s.split_once('/')
        && mask.parse::<u8>().is_err()
    {
        return Err("来源地址掩码不合法".to_string());
    }
    Ok(())
}

// ── handlers ────────────────────────────────────────────────

pub async fn status(panel_port: u16) -> Response {
    let (b, via) = detect_kind();
    let wsl = is_wsl();
    if b == Backend::Unsupported {
        return Response::ok(
            "未检测到受支持的防火墙（firewalld / ufw / nftables / iptables）",
            Some(json!({
                "backend": "none",
                "detected": "none",
                "active": false,
                "enabled": false,
                "wsl": wsl,
                "panel_port": panel_port,
                "rules": [],
            })),
        );
    }
    let active = backend_active(b);
    let enabled = backend_enabled(b);
    let rules: Vec<Value> = list_rules(b)
        .iter()
        .map(|r| rule_json(r, hits_panel_port(r.port, panel_port)))
        .collect();
    Response::ok(
        "OK",
        Some(json!({
            "backend": b.name(),
            // running=确实在过滤流量；installed=只装了命令、当前并未生效
            "detected": via,
            "active": active,
            "enabled": enabled,
            "wsl": wsl,
            "diag": diag(),
            "panel_port": panel_port,
            "rules": rules,
        })),
    )
}

pub async fn rule_add(
    port: u16,
    proto: String,
    action: String,
    source: String,
    comment: String,
    panel_port: u16,
) -> Response {
    let proto = proto.trim().to_lowercase();
    let action = action.trim().to_lowercase();
    let source = source.trim().to_string();
    if let Err(e) = validate_rule_input(port, &proto, &action, &source) {
        return Response::err(-1, e);
    }
    // 自杀保护：不允许对面板端口下 drop 规则
    if action != "accept" && hits_panel_port(port, panel_port) {
        return Response::err(
            -1,
            format!("端口 {port} 是面板监听端口，拒绝在此端口上添加 drop/reject 规则"),
        );
    }
    let rule = Rule {
        id: String::new(),
        port,
        proto,
        action,
        source,
        comment: comment.trim().to_string(),
    };
    match apply_rule(detect(), &rule) {
        Ok(()) => Response::ok("规则已添加", None),
        Err(e) => Response::err(-1, e),
    }
}

pub async fn rule_delete(id: String, panel_port: u16) -> Response {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Response::err(-1, "缺少规则 id".to_string());
    }
    let b = detect();
    // 自杀保护：不允许删除面板端口的放行规则
    let blocked = list_rules(b)
        .iter()
        .any(|r| r.id == id && r.action == "accept" && hits_panel_port(r.port, panel_port));
    if blocked {
        return Response::err(
            -1,
            format!("端口 {panel_port} 是面板监听端口，已阻止删除该放行规则"),
        );
    }
    match delete_rule(b, &id) {
        Ok(()) => Response::ok("规则已删除", None),
        Err(e) => Response::err(-1, e),
    }
}

pub async fn toggle(action: String) -> Response {
    let action = action.trim().to_lowercase();
    if !matches!(action.as_str(), "start" | "stop" | "enable" | "disable") {
        return Response::err(-1, "动作仅支持 start / stop / enable / disable".to_string());
    }
    let b = detect();
    match toggle_service(b, &action) {
        Ok(()) => Response::ok(format!("防火墙已{action}"), None),
        Err(e) => Response::err(-1, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_port_protection() {
        assert!(hits_panel_port(2600, 2600));
        assert!(!hits_panel_port(80, 2600));
        // 未配置面板端口时不保护任何规则
        assert!(!hits_panel_port(2600, 0));
    }

    #[test]
    fn rule_input_validation() {
        assert!(validate_rule_input(80, "tcp", "accept", "").is_ok());
        assert!(validate_rule_input(80, "tcp", "accept", "1.2.3.4").is_ok());
        assert!(validate_rule_input(80, "tcp", "accept", "10.0.0.0/8").is_ok());
        assert!(validate_rule_input(0, "tcp", "accept", "").is_err());
        assert!(validate_rule_input(80, "sctp", "accept", "").is_err());
        assert!(validate_rule_input(80, "tcp", "reject", "").is_err());
        assert!(validate_rule_input(80, "tcp", "accept", "not-an-ip").is_err());
        // 来源里混入 shell 元字符必须被拒
        assert!(validate_rule_input(80, "tcp", "accept", "1.2.3.4; rm -rf /").is_err());
    }

    #[test]
    fn ufw_argument_shapes() {
        let base = Rule {
            id: String::new(),
            port: 80,
            proto: "tcp".to_string(),
            action: "accept".to_string(),
            source: String::new(),
            comment: String::new(),
        };
        // 不限来源：必须是 80/tcp，而不是 `port 80 proto tcp`
        assert_eq!(ufw_args(&base, false), vec!["allow", "80/tcp"]);
        // 指定来源
        let src = Rule {
            source: "1.2.3.4".to_string(),
            ..base.clone()
        };
        assert_eq!(
            ufw_args(&src, false),
            vec![
                "allow", "from", "1.2.3.4", "to", "any", "port", "80", "proto", "tcp"
            ]
        );
        // 拒绝
        let deny = Rule {
            action: "drop".to_string(),
            ..base.clone()
        };
        assert_eq!(ufw_args(&deny, false), vec!["deny", "80/tcp"]);
        // 备注（空格替换为下划线）
        let cmt = Rule {
            comment: "web server".to_string(),
            ..base.clone()
        };
        assert_eq!(
            ufw_args(&cmt, true),
            vec!["allow", "80/tcp", "comment", "web_server"]
        );
    }

    #[test]
    fn backend_names() {
        assert_eq!(Backend::Firewalld.name(), "firewalld");
        assert_eq!(Backend::Ufw.service(), Some("ufw"));
        assert_eq!(Backend::Unsupported.name(), "none");
    }
}
