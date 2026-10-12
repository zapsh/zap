// SPDX-License-Identifier: AGPL-3.0-only
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
        if raw.is_empty() || !raw.contains("port port=\"") {
            // 无端口的 drop 富规则是 IP 黑名单条目，由 firewalld_blacklist 列出
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
        // 第一个字段通常是 80/tcp 或 80；非端口形态（如黑名单 `Anywhere DENY IN <ip>`）跳过
        let first = body.split_whitespace().next().unwrap_or("");
        let first_port = first.split('/').next().unwrap_or("");
        if first_port.is_empty() || !first_port.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
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
        // dport 可能是单端口，也可能是集合 `{ a,b,c }`（一键加固白名单）——逐端口展开
        let rest = l.split("dport ").nth(1).unwrap_or("");
        let ports: Vec<u16> = if rest.trim_start().starts_with('{') {
            let inner = rest
                .split('{')
                .nth(1)
                .and_then(|s| s.split('}').next())
                .unwrap_or("");
            inner
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect()
        } else {
            vec![rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)]
        };
        let proto = if l.contains("udp dport") {
            "udp"
        } else {
            "tcp"
        };
        let action = if l.contains("accept") {
            "accept"
        } else {
            "drop"
        };
        let source = l
            .split("saddr ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("")
            .to_string();
        let comment = l
            .split("comment \"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("")
            .to_string();
        for port in ports {
            out.push(Rule {
                id: handle.clone(),
                port,
                proto: proto.to_string(),
                action: action.to_string(),
                source: source.clone(),
                comment: comment.clone(),
            });
        }
    }
    out
}

fn iptables_rules() -> Vec<Rule> {
    // 只看 INPUT 链里带 dpt:/dports 的规则，取行号作为删除 id
    let mut out = Vec::new();
    for line in cmd_out("iptables", &["-L", "INPUT", "-n", "--line-numbers"]).lines() {
        let l = line.trim();
        // 单端口 dpt: 或 multiport dports（一键加固白名单）——逐端口展开
        let ports: Vec<u16> = if l.contains("dports ") {
            l.split("dports ")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
                .unwrap_or_default()
        } else if l.contains("dpt:") {
            vec![l
                .split("dpt:")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)]
        } else {
            continue;
        };
        if ports.is_empty() {
            continue;
        }
        let toks: Vec<&str> = l.split_whitespace().collect();
        let num = toks.first().copied().unwrap_or("").to_string();
        if !num.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let action = toks.get(1).copied().unwrap_or("").to_lowercase();
        let action = if action.contains("accept") {
            "accept"
        } else {
            "drop"
        };
        let proto = if l.contains(" udp ") { "udp" } else { "tcp" };
        // -L 列序：num target prot opt in out source destination [扩展...]
        let source = toks.get(6).copied().unwrap_or("0.0.0.0/0").to_string();
        let source = if source == "0.0.0.0/0" {
            String::new()
        } else {
            source
        };
        let comment = l
            .split("/* ")
            .nth(1)
            .and_then(|s| s.split(" */").next())
            .unwrap_or("")
            .to_string();
        for port in ports {
            out.push(Rule {
                id: num.clone(),
                port,
                proto: proto.to_string(),
                action: action.to_string(),
                source: source.clone(),
                comment: comment.clone(),
            });
        }
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
fn backend_active(b: Backend) -> bool {
    match b {
        Backend::Firewalld => cmd_out("firewall-cmd", &["--state"]).trim() == "running",
        Backend::Ufw => cmd_out("ufw", &["status"]).contains("Status: active"),
        // nftables 是 oneshot 服务：systemd active 即代表正在加载并过滤流量，
        // 不应再要求「已开机自启 + 规则集非空」，否则刚 `start`（未开机自启 / 空规则集）
        // 会被误判为未运行，出现「启动成功却显示没运行」。
        Backend::Nftables => service_active("nftables"),
        // iptables 通常无守护进程：存在真实规则即视为生效；若发行版提供 iptables.service
        // 且其 active，同样视为运行中（避免「启动成功却显示未运行」）
        Backend::Iptables => service_active("iptables") || iptables_has_rules(),
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

// ── 端口转发 / IP 黑名单 ────────────────────────────────────

#[derive(Debug, Clone)]
struct Forward {
    /// 删除标识：nftables=dstnat 链 handle；iptables=`tcp:8080:1.2.3.4:80`；
    /// firewalld=forward-port 参数串
    id: String,
    proto: String,
    listen_port: u16,
    target_addr: String,
    target_port: u16,
    comment: String,
}

#[derive(Debug, Clone)]
struct Black {
    /// 删除标识：nftables=input 链 handle；iptables/firewalld(去前缀)/ufw=IP 或 CIDR；
    /// firewalld=`bl:<rich rule>`
    id: String,
    ip: String,
    comment: String,
}

fn forward_json(f: &Forward) -> Value {
    json!({
        "id": f.id,
        "proto": f.proto,
        "listen_port": f.listen_port,
        "target_addr": f.target_addr,
        "target_port": f.target_port,
        "comment": f.comment,
    })
}

fn black_json(x: &Black) -> Value {
    json!({
        "id": x.id,
        "ip": x.ip,
        "comment": x.comment,
    })
}

/// 白名单（放行优先于一切）：命中后既不被黑名单封禁，也不会被 fail2ban 封禁。
#[derive(Debug, Clone)]
struct White {
    /// 删除标识：nftables=input 链 handle；iptables=IP；firewalld=`wl:<rich rule>`；ufw=IP
    id: String,
    ip: String,
    comment: String,
}

fn white_json(x: &White) -> Value {
    json!({
        "id": x.id,
        "ip": x.ip,
        "comment": x.comment,
    })
}

/// 目标地址是否为 IPv4（用于选择 nft 的 ip / ip6 关键字与 rich rule family）
fn is_ipv4_cidr(s: &str) -> bool {
    let addr = s.split('/').next().unwrap_or("");
    addr.parse::<std::net::IpAddr>()
        .map(|a| a.is_ipv4())
        .unwrap_or(true)
}

/// 内核是否开启 IPv4 转发（端口转发的前提）。
fn ip_forward_enabled() -> bool {
    std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

/// 打开 IPv4 转发：立即 `sysctl -w` 生效 + 写 sysctl.d 持久化（幂等）。
/// 端口转发没有它不生效，属于该功能的必要前置，面板代为开启。
fn ensure_ip_forward() {
    if ip_forward_enabled() {
        return;
    }
    let _ = root_cmd("sysctl")
        .args(["-w", "net.ipv4.ip_forward=1"])
        .output();
    let conf = "/etc/sysctl.d/99-zap-firewall.conf";
    if !std::path::Path::new(conf).exists() {
        let _ = std::fs::write(conf, "# Created by zap panel (firewall forwarding)\nnet.ipv4.ip_forward = 1\n");
    }
}

/// 端口转发是否被当前后端支持（ufw 需改写 before.rules，不做）
fn forward_supported(b: Backend) -> bool {
    matches!(b, Backend::Firewalld | Backend::Nftables | Backend::Iptables)
}

/// 是否已处于「锁服」状态（封禁全部、仅白名单放通）。由各后端的加固标记判定。
fn locked_down(b: Backend) -> bool {
    match b {
        Backend::Nftables => cmd_out("nft", &["list", "chain", "inet", "zap_fw", "input"])
            .contains("zap:lock:drop"),
        Backend::Iptables => {
            cmd_out("iptables", &["-S", "INPUT"]).contains("zap:lock:drop")
                || cmd_out("ip6tables", &["-S", "INPUT"]).contains("zap:lock:drop")
        }
        Backend::Firewalld => cmd_out("firewall-cmd", &["--list-all"]).contains("target: DROP"),
        Backend::Ufw => cmd_out("ufw", &["status", "verbose"]).contains("Default: deny"),
        _ => false,
    }
}

fn list_forwards(b: Backend) -> Vec<Forward> {
    match b {
        Backend::Nftables => nft_forwards(),
        Backend::Iptables => iptables_forwards(),
        Backend::Firewalld => firewalld_forwards(),
        _ => Vec::new(),
    }
}

fn list_blacklist(b: Backend) -> Vec<Black> {
    match b {
        Backend::Nftables => nft_blacklist(),
        Backend::Iptables => iptables_blacklist(),
        Backend::Firewalld => firewalld_blacklist(),
        Backend::Ufw => ufw_blacklist(),
        _ => Vec::new(),
    }
}

fn list_whitelist(b: Backend) -> Vec<White> {
    match b {
        Backend::Nftables => nft_whitelist(),
        Backend::Iptables => iptables_whitelist(),
        Backend::Firewalld => firewalld_whitelist(),
        Backend::Ufw => ufw_whitelist(),
        _ => Vec::new(),
    }
}

// ── nftables：面板自管 zap_fw 表的 dstnat / srcnat 链 ──────

/// 确保 zap_fw 表与 input / dstnat / srcnat 链存在（已存在时 nft 返回错误，忽略即可）。
fn nft_ensure_chains() {
    let _ = root_cmd("nft").args(["add", "table", "inet", "zap_fw"]).output();
    let _ = root_cmd("nft").args([
        "add", "chain", "inet", "zap_fw", "input",
        "{ type filter hook input priority 0 ; }",
    ]).output();
    let _ = root_cmd("nft").args([
        "add", "chain", "inet", "zap_fw", "dstnat",
        "{ type nat hook prerouting priority dstnat ; }",
    ]).output();
    let _ = root_cmd("nft").args([
        "add", "chain", "inet", "zap_fw", "srcnat",
        "{ type nat hook postrouting priority srcnat ; }",
    ]).output();
}

fn nft_forwards() -> Vec<Forward> {
    let mut out = Vec::new();
    for line in cmd_out("nft", &["-a", "list", "chain", "inet", "zap_fw", "dstnat"]).lines() {
        let l = line.trim();
        let Some((main, handle)) = l.split_once("# handle ") else {
            continue;
        };
        let handle = handle.split_whitespace().next().unwrap_or("").to_string();
        // 去掉 comment 尾巴，例如 `... dnat to 1.2.3.4:80 comment "zap:masq:..."`
        let main = main.split(" comment ").next().unwrap_or(main).trim();
        let Some((left, target)) = main.split_once("dnat to ") else {
            continue;
        };
        let proto = if left.contains("udp dport") { "udp" } else { "tcp" };
        let Some(listen_port) = left
            .split("dport ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
        else {
            continue;
        };
        let target = target.trim();
        let (addr, port) = if let Some(rest) = target.strip_prefix('[') {
            // IPv6：[addr]:port
            match rest.split_once("]:") {
                Some((a, p)) => (a.to_string(), p.parse().unwrap_or(0)),
                None => (rest.trim_end_matches(']').to_string(), 0),
            }
        } else {
            match target.split_once(':') {
                Some((a, p)) => (a.to_string(), p.parse().unwrap_or(0)),
                None => (target.to_string(), 0),
            }
        };
        out.push(Forward {
            id: handle,
            proto: proto.to_string(),
            listen_port,
            target_addr: addr,
            target_port: port,
            comment: String::new(),
        });
    }
    out
}

fn nft_forward_add(f: &Forward) -> Result<(), String> {
    nft_ensure_chains();
    ensure_ip_forward();
    let fam = if is_ipv4_cidr(&f.target_addr) {
        "meta nfproto ipv4"
    } else {
        "meta nfproto ipv6"
    };
    let to = if is_ipv4_cidr(&f.target_addr) {
        format!("{}:{}", f.target_addr, f.target_port)
    } else {
        format!("[{}]:{}", f.target_addr, f.target_port)
    };
    let rule = format!(
        "{fam} {proto} dport {lp} dnat to {to}",
        proto = f.proto,
        lp = f.listen_port,
        to = to
    );
    let refs: Vec<&str> = ["add", "rule", "inet", "zap_fw", "dstnat", &rule]
        .iter()
        .copied()
        .collect();
    let (ok, out) = run_capture("nft", &refs);
    if !ok {
        return Err(format!("nft 添加转发规则失败: {out}"));
    }
    // 回程 masquerade：同目标仅加一条（内核较老不支持 inet nat masquerade 时静默忽略）
    if !nft_has_masq(&f.target_addr) {
        let fam_kw = if is_ipv4_cidr(&f.target_addr) { "ip" } else { "ip6" };
        let masq = format!(
            "{fam_kw} daddr {ta} masquerade comment \"zap:masq:{ta}\"",
            ta = f.target_addr
        );
        let mrefs: Vec<&str> = ["add", "rule", "inet", "zap_fw", "srcnat", &masq]
            .iter()
            .copied()
            .collect();
        let _ = root_cmd("nft").args(mrefs).output();
    }
    Ok(())
}

fn nft_has_masq(addr: &str) -> bool {
    cmd_out("nft", &["list", "chain", "inet", "zap_fw", "srcnat"])
        .contains(&format!("zap:masq:{addr}"))
}

fn nft_forward_delete(id: &str) -> Result<(), String> {
    if !cmd_ok("nft", &["delete", "rule", "inet", "zap_fw", "dstnat", "handle", id]) {
        return Err("nft 删除转发规则失败".to_string());
    }
    // 目标已无任何转发规则时，顺手清掉回程 masquerade（按 srcnat 的 comment 反查）
    nft_cleanup_masq();
    Ok(())
}

/// 清理「目标已无转发规则」的 masquerade 规则。
fn nft_cleanup_masq() {
    let live: Vec<String> = nft_forwards()
        .iter()
        .map(|f| f.target_addr.clone())
        .collect();
    for line in cmd_out("nft", &["-a", "list", "chain", "inet", "zap_fw", "srcnat"]).lines() {
        let l = line.trim();
        let Some(start) = l.find("zap:masq:") else {
            continue;
        };
        let ta: String = l[start + 9..]
            .trim_end_matches('"')
            .chars()
            .take_while(|c| *c != '"' && *c != ' ')
            .collect();
        if live.iter().any(|a| *a == ta) {
            continue;
        }
        if let Some((_, h)) = l.split_once("# handle ") {
            let handle = h.split_whitespace().next().unwrap_or("");
            if !handle.is_empty() {
                let _ = root_cmd("nft")
                    .args(["delete", "rule", "inet", "zap_fw", "srcnat", "handle", handle])
                    .output();
            }
        }
    }
}

fn nft_blacklist() -> Vec<Black> {
    let mut out = Vec::new();
    for line in cmd_out("nft", &["-a", "list", "chain", "inet", "zap_fw", "input"]).lines() {
        let l = line.trim();
        if !l.contains("drop") || !l.contains("comment \"zap:bl:") {
            continue;
        }
        let Some((_, h)) = l.split_once("# handle ") else {
            continue;
        };
        let handle = h.split_whitespace().next().unwrap_or("").to_string();
        let ip = l
            .split("saddr ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("")
            .to_string();
        out.push(Black {
            id: handle,
            ip,
            comment: String::new(),
        });
    }
    out
}

fn nft_blacklist_add(ip: &str) -> Result<(), String> {
    nft_ensure_chains();
    let fam = if is_ipv4_cidr(ip) { "ip" } else { "ip6" };
    let rule = format!("{fam} saddr {ip} drop comment \"zap:bl:{ip}\"");
    let refs: Vec<&str> = ["add", "rule", "inet", "zap_fw", "input", &rule]
        .iter()
        .copied()
        .collect();
    let (ok, out) = run_capture("nft", &refs);
    if !ok {
        return Err(format!("nft 添加黑名单失败: {out}"));
    }
    Ok(())
}

/// nft 白名单：accept 规则必须插到链首，否则排在 drop 之后等于没放。
/// 老版本 nft 没有 `insert rule` 时退化为 add（此时优先级依赖链内既有顺序）。
fn nft_whitelist() -> Vec<White> {
    let mut out = Vec::new();
    for line in cmd_out("nft", &["-a", "list", "chain", "inet", "zap_fw", "input"]).lines() {
        let l = line.trim();
        if !l.contains("accept") || !l.contains("comment \"zap:wl:") {
            continue;
        }
        let Some((_, h)) = l.split_once("# handle ") else {
            continue;
        };
        let handle = h.split_whitespace().next().unwrap_or("").to_string();
        let ip = l
            .split("saddr ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("")
            .to_string();
        out.push(White {
            id: handle,
            ip,
            comment: String::new(),
        });
    }
    out
}

fn nft_whitelist_add(ip: &str) -> Result<(), String> {
    nft_ensure_chains();
    let fam = if is_ipv4_cidr(ip) { "ip" } else { "ip6" };
    let rule = format!("{fam} saddr {ip} accept comment \"zap:wl:{ip}\"");
    let refs: Vec<&str> = ["insert", "rule", "inet", "zap_fw", "input", &rule]
        .iter()
        .copied()
        .collect();
    let (ok, out) = run_capture("nft", &refs);
    if !ok {
        // 老版本 nft 不支持 insert rule：退化为追加
        let refs: Vec<&str> = ["add", "rule", "inet", "zap_fw", "input", &rule]
            .iter()
            .copied()
            .collect();
        let (ok2, out2) = run_capture("nft", &refs);
        if !ok2 {
            return Err(format!("nft 添加白名单失败: {out} / {out2}"));
        }
    }
    Ok(())
}

// ── iptables：nat 表 PREROUTING / POSTROUTING ──────────────

fn iptables_forwards() -> Vec<Forward> {
    let mut out = Vec::new();
    for line in cmd_out("iptables", &["-t", "nat", "-S", "PREROUTING"]).lines() {
        let l = line.trim();
        if !l.starts_with("-A PREROUTING") || !l.contains("--to-destination") {
            continue;
        }
        let proto = l
            .split(" -p ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("tcp")
            .to_string();
        let listen_port = l
            .split("--dport ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let to = l
            .split("--to-destination ")
            .nth(1)
            .unwrap_or("")
            .trim()
            .to_string();
        let (addr, port) = match to.split_once(':') {
            Some((a, p)) => (a.to_string(), p.parse().unwrap_or(0)),
            None => (to.clone(), 0),
        };
        out.push(Forward {
            id: format!(
                "{proto}:{lp}:{addr}:{tp}",
                lp = listen_port,
                tp = port
            ),
            proto,
            listen_port,
            target_addr: addr,
            target_port: port,
            comment: String::new(),
        });
    }
    out
}

fn iptables_forward_add(f: &Forward) -> Result<(), String> {
    ensure_ip_forward();
    let to = format!("{}:{}", f.target_addr, f.target_port);
    let refs: Vec<String> = vec![
        "-t".into(), "nat".into(), "-A".into(), "PREROUTING".into(),
        "-p".into(), f.proto.clone(), "--dport".into(), f.listen_port.to_string(),
        "-j".into(), "DNAT".into(), "--to-destination".into(), to,
    ];
    let srefs: Vec<&str> = refs.iter().map(|s| s.as_str()).collect();
    let (ok, out) = run_capture("iptables", &srefs);
    if !ok {
        return Err(format!("iptables 添加转发规则失败：{out}"));
    }
    // 回程 masquerade：同目标仅加一条
    if !cmd_out("iptables", &["-t", "nat", "-S", "POSTROUTING"])
        .lines()
        .any(|l| l.contains(&format!("-d {} ", f.target_addr)) && l.contains("MASQUERADE"))
    {
        let _ = root_cmd("iptables")
            .args(["-t", "nat", "-A", "POSTROUTING", "-d", &f.target_addr, "-j", "MASQUERADE"])
            .output();
    }
    Ok(())
}

fn iptables_forward_delete(id: &str) -> Result<(), String> {
    // id 形如 tcp:8080:1.2.3.4:80，可直接重建精确的删除参数（不依赖易漂移的行号）
    let parts: Vec<&str> = id.split(':').collect();
    if parts.len() != 4 {
        return Err("转发规则 id 不合法".to_string());
    }
    let (proto, lp, ta, tp) = (parts[0], parts[1], parts[2], parts[3]);
    let to = if tp == "0" { ta.to_string() } else { format!("{ta}:{tp}") };
    let ok = cmd_ok(
        "iptables",
        &[
            "-t", "nat", "-D", "PREROUTING", "-p", proto, "--dport", lp, "-j", "DNAT",
            "--to-destination", &to,
        ],
    );
    if !ok {
        return Err("iptables 删除转发规则失败".to_string());
    }
    // 同目标已无其它转发规则时清掉 masquerade
    if !iptables_forwards().iter().any(|f| f.target_addr == ta) {
        let _ = root_cmd("iptables")
            .args(["-t", "nat", "-D", "POSTROUTING", "-d", ta, "-j", "MASQUERADE"])
            .output();
    }
    Ok(())
}

fn iptables_blacklist() -> Vec<Black> {
    let mut out = Vec::new();
    // iptables(IPv4) 与 ip6tables(IPv6) 都要扫：封禁来源两种族都可能有
    for bin in ["iptables", "ip6tables"] {
        for line in cmd_out(bin, &["-S", "INPUT"]).lines() {
            let l = line.trim();
            if !l.starts_with("-A INPUT") || !l.contains("-j DROP") || l.contains("--dport") {
                continue;
            }
            let Some(ip) = l.split(" -s ").nth(1).and_then(|s| s.split_whitespace().next()) else {
                continue;
            };
            out.push(Black {
                id: ip.to_string(),
                ip: ip.to_string(),
                comment: String::new(),
            });
        }
    }
    out
}

fn iptables_blacklist_add(ip: &str) -> Result<(), String> {
    // IPv6 来源必须写 ip6tables，否则规则落错族、封禁无效
    let bin = if is_ipv4_cidr(ip) { "iptables" } else { "ip6tables" };
    let (ok, out) = run_capture(bin, &["-I", "INPUT", "1", "-s", ip, "-j", "DROP"]);
    if !ok {
        return Err(format!("{bin} 添加黑名单失败：{out}"));
    }
    Ok(())
}

/// iptables 白名单：插到链首，优先于黑名单与既有放行规则。
fn iptables_whitelist() -> Vec<White> {
    let mut out = Vec::new();
    for bin in ["iptables", "ip6tables"] {
        for line in cmd_out(bin, &["-S", "INPUT"]).lines() {
            let l = line.trim();
            if !l.starts_with("-A INPUT") || !l.contains("-j ACCEPT") || l.contains("--dport") {
                continue;
            }
            let Some(ip) = l.split(" -s ").nth(1).and_then(|s| s.split_whitespace().next()) else {
                continue;
            };
            if ip.contains('/') && (ip.starts_with("0.0.0.0") || ip == "::/0") {
                continue;
            }
            out.push(White {
                id: ip.to_string(),
                ip: ip.to_string(),
                comment: String::new(),
            });
        }
    }
    out
}

fn iptables_whitelist_add(ip: &str) -> Result<(), String> {
    let bin = if is_ipv4_cidr(ip) { "iptables" } else { "ip6tables" };
    let (ok, out) = run_capture(bin, &["-I", "INPUT", "1", "-s", ip, "-j", "ACCEPT"]);
    if !ok {
        return Err(format!("{bin} 添加白名单失败：{out}"));
    }
    Ok(())
}

// ── firewalld：forward-port + 富规则黑名单 ─────────────────

fn firewalld_forwards() -> Vec<Forward> {
    let mut out = Vec::new();
    for line in cmd_out("firewall-cmd", &["--permanent", "--list-forward-ports"]).lines() {
        let raw = line.trim().trim_matches('\'').to_string();
        if raw.is_empty() {
            continue;
        }
        // port=8080:proto=tcp:toport=80:toaddr=1.2.3.4
        let get = |key: &str| -> String {
            raw.split(&format!("{key}="))
                .nth(1)
                .map(|s| s.split(':').next().unwrap_or("").to_string())
                .unwrap_or_default()
        };
        let listen_port = get("port").parse().unwrap_or(0);
        let proto = {
            let p = get("proto");
            if p.is_empty() { "tcp".to_string() } else { p }
        };
        let target_port: u16 = get("toport").parse().unwrap_or(0);
        let target_addr = get("toaddr");
        if target_addr.is_empty() {
            continue;
        }
        out.push(Forward {
            id: raw,
            proto,
            listen_port,
            target_addr,
            target_port,
            comment: String::new(),
        });
    }
    out
}

fn firewalld_forward_add(f: &Forward) -> Result<(), String> {
    ensure_ip_forward();
    let spec = format!(
        "port={lp}:proto={proto}:toport={tp}:toaddr={ta}",
        lp = f.listen_port,
        proto = f.proto,
        tp = f.target_port,
        ta = f.target_addr
    );
    let _ = run_capture("firewall-cmd", &["--permanent", "--add-masquerade"]);
    let (ok, out) = run_capture("firewall-cmd", &["--permanent", "--add-forward-port", &spec]);
    if !ok {
        return Err(format!("firewall-cmd --add-forward-port 执行失败：{out}"));
    }
    let (ok, out) = run_capture("firewall-cmd", &["--reload"]);
    if !ok {
        return Err(format!("firewall-cmd --reload 执行失败：{out}"));
    }
    Ok(())
}

fn firewalld_forward_delete(id: &str) -> Result<(), String> {
    let (ok, out) = run_capture(
        "firewall-cmd",
        &["--permanent", "--remove-forward-port", id],
    );
    if !ok {
        return Err(format!("firewall-cmd --remove-forward-port 执行失败：{out}"));
    }
    let _ = run_capture("firewall-cmd", &["--reload"]);
    Ok(())
}

/// 富规则里不含 port 的 drop 规则即黑名单条目
fn firewalld_blacklist() -> Vec<Black> {
    let mut out = Vec::new();
    for line in cmd_out("firewall-cmd", &["--list-rich-rules"]).lines() {
        let raw = line.trim().trim_matches('\'').to_string();
        if raw.is_empty() || !raw.contains("drop") || raw.contains("port port=") {
            continue;
        }
        let ip = raw
            .split("source address=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("")
            .to_string();
        if ip.is_empty() {
            continue;
        }
        out.push(Black {
            id: format!("bl:{raw}"),
            ip,
            comment: String::new(),
        });
    }
    out
}

fn firewalld_blacklist_add(ip: &str) -> Result<(), String> {
    let family = if is_ipv4_cidr(ip) { "ipv4" } else { "ipv6" };
    let rule = format!("rule family=\"{family}\" source address=\"{ip}\" drop");
    let (ok, out) = run_capture("firewall-cmd", &["--permanent", "--add-rich-rule", &rule]);
    if !ok {
        return Err(format!("firewall-cmd --add-rich-rule 执行失败：{out}"));
    }
    let (ok, out) = run_capture("firewall-cmd", &["--reload"]);
    if !ok {
        return Err(format!("firewall-cmd --reload 执行失败：{out}"));
    }
    Ok(())
}

/// firewalld 富规则按配置顺序匹配：白名单 accept 若加在 drop 之后就不生效，
/// 因此这里加 accept 的同时删掉同源的 drop，确保该 IP 一定放通。
fn firewalld_whitelist() -> Vec<White> {
    let mut out = Vec::new();
    for line in cmd_out("firewall-cmd", &["--list-rich-rules"]).lines() {
        let raw = line.trim().trim_matches('\'').to_string();
        if raw.is_empty() || !raw.contains("accept") || raw.contains("port port=") {
            continue;
        }
        let ip = raw
            .split("source address=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("")
            .to_string();
        if ip.is_empty() {
            continue;
        }
        out.push(White {
            id: format!("wl:{raw}"),
            ip,
            comment: String::new(),
        });
    }
    out
}

fn firewalld_whitelist_add(ip: &str) -> Result<(), String> {
    let family = if is_ipv4_cidr(ip) { "ipv4" } else { "ipv6" };
    // 先摘掉同源的 drop，再加 accept，避免顺序问题导致白名单失效
    let drop_rule = format!("rule family=\"{family}\" source address=\"{ip}\" drop");
    let _ = run_capture("firewall-cmd", &["--permanent", "--remove-rich-rule", &drop_rule]);
    let rule = format!("rule family=\"{family}\" source address=\"{ip}\" accept");
    let (ok, out) = run_capture("firewall-cmd", &["--permanent", "--add-rich-rule", &rule]);
    if !ok {
        return Err(format!("firewall-cmd --add-rich-rule 执行失败：{out}"));
    }
    let (ok, out) = run_capture("firewall-cmd", &["--reload"]);
    if !ok {
        return Err(format!("firewall-cmd --reload 执行失败：{out}"));
    }
    Ok(())
}

// ── ufw：黑名单用 deny from（端口转发不支持） ───────────────

fn ufw_blacklist() -> Vec<Black> {
    let mut out = Vec::new();
    for line in cmd_out("ufw", &["status", "numbered"]).lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix('[') else {
            continue;
        };
        let Some((_, body)) = rest.split_once(']') else {
            continue;
        };
        let body = body.trim();
        if !body.contains("DENY") {
            continue;
        }
        // 形如 `Anywhere  DENY IN  1.2.3.4`：DENY 后除 IN/OUT 外的 token 即来源
        let after = body.split("DENY").nth(1).unwrap_or("");
        let from = after
            .split_whitespace()
            .find(|t| *t != "IN" && *t != "OUT")
            .unwrap_or("");
        if from.is_empty() || from == "Anywhere" {
            continue;
        }
        out.push(Black {
            id: from.to_string(),
            ip: from.to_string(),
            comment: String::new(),
        });
    }
    out
}

fn ufw_blacklist_add(ip: &str) -> Result<(), String> {
    // insert 1 保证黑名单优先于既有放行规则；带注释标记便于列表识别（老 ufw 不支持则退化）
    let tag = format!("zap:bl_{ip}");
    let (ok, _) = run_capture(
        "ufw",
        &["insert", "1", "deny", "from", ip, "comment", &tag],
    );
    if ok {
        return Ok(());
    }
    let (ok, out) = run_capture("ufw", &["insert", "1", "deny", "from", ip]);
    if !ok {
        return Err(format!("ufw 添加黑名单失败：{out}"));
    }
    Ok(())
}

/// ufw 白名单：`ufw insert 1 allow from <ip>`（IPv4 / IPv6 通吃，ufw 自行分族）。
fn ufw_whitelist() -> Vec<White> {
    let mut out = Vec::new();
    for line in cmd_out("ufw", &["status", "numbered"]).lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix('[') else {
            continue;
        };
        let Some((_, body)) = rest.split_once(']') else {
            continue;
        };
        let body = body.trim();
        if !body.contains("ALLOW") {
            continue;
        }
        let after = body.split("ALLOW").nth(1).unwrap_or("");
        let from = after
            .split_whitespace()
            .find(|t| *t != "IN" && *t != "OUT")
            .unwrap_or("");
        if from.is_empty() || from == "Anywhere" || from.contains('/') {
            continue;
        }
        // 只认 IP 形态的 allow（端口放行规则不是白名单）
        if from.parse::<std::net::IpAddr>().is_err() {
            continue;
        }
        out.push(White {
            id: from.to_string(),
            ip: from.to_string(),
            comment: String::new(),
        });
    }
    out
}

fn ufw_whitelist_add(ip: &str) -> Result<(), String> {
    let (ok, _) = run_capture(
        "ufw",
        &["insert", "1", "allow", "from", ip, "comment", &format!("zap:wl_{ip}")],
    );
    if ok {
        return Ok(());
    }
    let (ok, out) = run_capture("ufw", &["insert", "1", "allow", "from", ip]);
    if !ok {
        return Err(format!("ufw 添加白名单失败：{out}"));
    }
    Ok(())
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
    // 备注写入 nft comment（引号/反斜杠会破坏语法，剔除）
    if !r.comment.is_empty() {
        let c = r.comment.replace('\\', "").replace('"', "'");
        expr.push_str(&format!(" comment \"{c}\""));
    }
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
    // 备注写入 iptables comment
    if !r.comment.is_empty() {
        let c = r.comment.replace('"', "'");
        args.push("-m".to_string());
        args.push("comment".to_string());
        args.push("--comment".to_string());
        args.push(c);
    }
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

/// 端口转发入参校验
fn validate_forward_input(
    proto: &str,
    listen_port: u16,
    target_addr: &str,
    target_port: u16,
    panel_port: u16,
) -> Result<(), String> {
    if !matches!(proto, "tcp" | "udp") {
        return Err("协议仅支持 tcp / udp".to_string());
    }
    if listen_port == 0 || target_port == 0 {
        return Err("端口必须在 1 - 65535 之间".to_string());
    }
    // 自杀保护：把面板端口转发到别处同样会把自己锁在外面
    if hits_panel_port(listen_port, panel_port) {
        return Err(format!(
            "端口 {listen_port} 是面板监听端口，不允许作为转发监听端口"
        ));
    }
    if target_addr.parse::<std::net::IpAddr>().is_err() {
        return Err("目标地址必须是合法 IP（不支持域名 / CIDR）".to_string());
    }
    Ok(())
}

/// 黑名单入参校验
fn validate_blacklist_ip(ip: &str) -> Result<(), String> {
    validate_source(ip)?;
    // 黑名单针对全部端口/协议，拒绝「封禁全世界」的自杀操作
    if matches!(ip, "0.0.0.0/0" | "::/0") {
        return Err("黑名单不允许封禁所有来源（0.0.0.0/0 / ::/0）".to_string());
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
                "forwards": [],
                "blacklist": [],
                "whitelist": [],
                "ip_forward": false,
                "forward_supported": false,
                "locked_down": false,
                })),
        );
    }
    let active = backend_active(b);
    let enabled = backend_enabled(b);
    let rules: Vec<Value> = list_rules(b)
        .iter()
        .map(|r| {
            rule_json(
                r,
                // 仅加固「基础设施」规则（已建连接 / ICMP / 兜底丢弃）受保护不可单独删改；
                // 白名单端口（zap:lock:Ssh 等）与面板端口放行规则均可单独编辑 / 删除，
                // 便于把端口限制为仅允许特定 IP（自杀式改为 drop 仍由下方校验拦截）。
                r.comment.starts_with("zap:lock:established")
                    || r.comment.starts_with("zap:lock:icmp")
                    || r.comment.starts_with("zap:lock:drop"),
            )
        })
        .collect();
    let forwards: Vec<Value> = list_forwards(b).iter().map(forward_json).collect();
    let blacklist: Vec<Value> = list_blacklist(b).iter().map(black_json).collect();
    let whitelist: Vec<Value> = list_whitelist(b).iter().map(white_json).collect();
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
            "forwards": forwards,
            "blacklist": blacklist,
            "whitelist": whitelist,
            "ip_forward": ip_forward_enabled(),
            "forward_supported": forward_supported(b),
            "locked_down": locked_down(b),
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
    let b = detect();
    match apply_rule(b, &rule) {
        Ok(()) => {
            nft_repin_drop_if_locked(b);
            Response::ok("规则已添加", None)
        }
        Err(e) => Response::err(-1, e),
    }
}

/// 加固后，把兜底 drop 规则重新钉到链尾，确保其后追加的放行规则仍可命中。
/// 仅 nftables 需要：nft 的 drop 是「规则」，新规则会被 append 到它之后而失效；
/// iptables / firewalld 用链默认策略 drop，显式放行规则无论位置都能命中，无需处理。
fn nft_repin_drop_if_locked(b: Backend) {
    if b != Backend::Nftables || !locked_down(b) {
        return;
    }
    let mut handle = String::new();
    for line in cmd_out("nft", &["-a", "list", "chain", "inet", "zap_fw", "input"]).lines() {
        let l = line.trim();
        if l.contains("drop comment \"zap:lock:drop\"") {
            if let Some((_, h)) = l.split_once("# handle ") {
                handle = h.split_whitespace().next().unwrap_or("").to_string();
            }
            break;
        }
    }
    if handle.is_empty() {
        return;
    }
    let _ = root_cmd("nft")
        .args(["delete", "rule", "inet", "zap_fw", "input", "handle", &handle])
        .output();
    let _ = root_cmd("nft")
        .args(["add", "rule", "inet", "zap_fw", "input", "drop comment \"zap:lock:drop\""])
        .output();
}

/// 修改规则：先按 id 删除旧规则（含面板端口自杀保护），再新增。
/// 仅加固「基础设施」规则（已建连接 / ICMP / 兜底丢弃）不可单独改；
/// 白名单端口与面板端口放行规则允许编辑（例如把面板端口限制为仅允许特定 IP 访问），
/// 但改成 drop/reject 仍由上方校验拦截，避免把自己锁外面。
pub async fn rule_update(
    id: String,
    port: u16,
    proto: String,
    action: String,
    source: String,
    comment: String,
    panel_port: u16,
) -> Response {
    let id = id.trim().to_string();
    let proto = proto.trim().to_lowercase();
    let action = action.trim().to_lowercase();
    let source = source.trim().to_string();
    if id.is_empty() {
        return Response::err(-1, "缺少规则 id".to_string());
    }
    if let Err(e) = validate_rule_input(port, &proto, &action, &source) {
        return Response::err(-1, e);
    }
    if action != "accept" && hits_panel_port(port, panel_port) {
        return Response::err(
            -1,
            format!("端口 {port} 是面板监听端口，拒绝在此端口上添加 drop/reject 规则"),
        );
    }
    let b = detect();
    // 加固基础设施规则（已建连接 / ICMP / 兜底丢弃）不可单独修改
    let protected = list_rules(b).iter().any(|r| {
        r.id == id
            && (r.comment.starts_with("zap:lock:established")
                || r.comment.starts_with("zap:lock:icmp")
                || r.comment.starts_with("zap:lock:icmp6")
                || r.comment.starts_with("zap:lock:drop"))
    });
    if protected {
        return Response::err(-1, "加固基础设施规则（已建连接 / ICMP / 兜底丢弃）不可单独修改".to_string());
    }
    // 先删后加
    if let Err(e) = delete_rule(b, &id) {
        return Response::err(-1, e);
    }
    let rule = Rule {
        id: String::new(),
        port,
        proto,
        action,
        source,
        comment: comment.trim().to_string(),
    };
    match apply_rule(b, &rule) {
        Ok(()) => {
            nft_repin_drop_if_locked(b);
            Response::ok("规则已修改", None)
        }
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

// ── 转发 / 黑名单 / 导入导出 handlers ───────────────────────

fn apply_forward(b: Backend, f: &Forward) -> Result<(), String> {
    match b {
        Backend::Firewalld => firewalld_forward_add(f),
        Backend::Nftables => nft_forward_add(f),
        Backend::Iptables => iptables_forward_add(f),
        Backend::Ufw => Err("ufw 暂不支持端口转发（需改写 before.rules），请改用 nftables / iptables / firewalld 后端".to_string()),
        _ => Err(format!("当前系统（{}）暂不支持该操作", std::env::consts::OS)),
    }
}

fn delete_forward(b: Backend, id: &str) -> Result<(), String> {
    match b {
        Backend::Firewalld => firewalld_forward_delete(id),
        Backend::Nftables => nft_forward_delete(id),
        Backend::Iptables => iptables_forward_delete(id),
        _ => Err(format!("当前系统（{}）暂不支持该操作", std::env::consts::OS)),
    }
}

fn apply_blacklist(b: Backend, ip: &str) -> Result<(), String> {
    match b {
        Backend::Firewalld => firewalld_blacklist_add(ip),
        Backend::Nftables => nft_blacklist_add(ip),
        Backend::Iptables => iptables_blacklist_add(ip),
        Backend::Ufw => ufw_blacklist_add(ip),
        _ => Err(format!("当前系统（{}）暂不支持该操作", std::env::consts::OS)),
    }
}

fn apply_whitelist(b: Backend, ip: &str) -> Result<(), String> {
    match b {
        Backend::Firewalld => firewalld_whitelist_add(ip),
        Backend::Nftables => nft_whitelist_add(ip),
        Backend::Iptables => iptables_whitelist_add(ip),
        Backend::Ufw => ufw_whitelist_add(ip),
        _ => Err(format!("当前系统（{}）暂不支持该操作", std::env::consts::OS)),
    }
}

fn delete_whitelist(b: Backend, id: &str) -> Result<(), String> {
    match b {
        Backend::Nftables => {
            if cmd_ok("nft", &["delete", "rule", "inet", "zap_fw", "input", "handle", id]) {
                Ok(())
            } else {
                Err("nft 删除白名单失败".to_string())
            }
        }
        Backend::Iptables => {
            // id 即 IP；IPv6 来源走 ip6tables
            let bin = if is_ipv4_cidr(id) { "iptables" } else { "ip6tables" };
            if cmd_ok(bin, &["-D", "INPUT", "-s", id, "-j", "ACCEPT"]) {
                Ok(())
            } else {
                Err(format!("{bin} 删除白名单失败"))
            }
        }
        Backend::Firewalld => {
            let Some(raw) = id.strip_prefix("wl:") else {
                return Err("白名单 id 不合法".to_string());
            };
            if !cmd_ok("firewall-cmd", &["--permanent", "--remove-rich-rule", raw]) {
                return Err("firewall-cmd 删除白名单失败".to_string());
            }
            let _ = run_capture("firewall-cmd", &["--reload"]);
            Ok(())
        }
        Backend::Ufw => {
            if cmd_ok("ufw", &["delete", "allow", "from", id]) {
                Ok(())
            } else {
                Err("ufw 删除白名单失败".to_string())
            }
        }
        _ => Err(format!("当前系统（{}）暂不支持该操作", std::env::consts::OS)),
    }
}

fn delete_blacklist(b: Backend, id: &str) -> Result<(), String> {
    match b {
        Backend::Firewalld => {
            let Some(raw) = id.strip_prefix("bl:") else {
                return Err("黑名单 id 不合法".to_string());
            };
            if !cmd_ok("firewall-cmd", &["--permanent", "--remove-rich-rule", raw]) {
                return Err("firewall-cmd 删除黑名单失败".to_string());
            }
            let _ = run_capture("firewall-cmd", &["--reload"]);
            Ok(())
        }
        Backend::Nftables => {
            if cmd_ok("nft", &["delete", "rule", "inet", "zap_fw", "input", "handle", id]) {
                Ok(())
            } else {
                Err("nft 删除黑名单失败".to_string())
            }
        }
        Backend::Iptables => {
            if cmd_ok("iptables", &["-D", "INPUT", "-s", id, "-j", "DROP"]) {
                Ok(())
            } else {
                Err("iptables 删除黑名单失败".to_string())
            }
        }
        Backend::Ufw => {
            if cmd_ok("ufw", &["--force", "delete", "deny", "from", id]) {
                Ok(())
            } else {
                Err("ufw 删除黑名单失败".to_string())
            }
        }
        _ => Err(format!("当前系统（{}）暂不支持该操作", std::env::consts::OS)),
    }
}

pub async fn forward_add(
    proto: String,
    listen_port: u16,
    target_addr: String,
    target_port: u16,
    comment: String,
    panel_port: u16,
) -> Response {
    let proto = proto.trim().to_lowercase();
    let target_addr = target_addr.trim().to_string();
    if let Err(e) = validate_forward_input(&proto, listen_port, &target_addr, target_port, panel_port) {
        return Response::err(-1, e);
    }
    let f = Forward {
        id: String::new(),
        proto,
        listen_port,
        target_addr,
        target_port,
        comment: comment.trim().to_string(),
    };
    match apply_forward(detect(), &f) {
        Ok(()) => Response::ok("转发规则已添加", None),
        Err(e) => Response::err(-1, e),
    }
}

pub async fn forward_delete(id: String) -> Response {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Response::err(-1, "缺少转发规则 id".to_string());
    }
    match delete_forward(detect(), &id) {
        Ok(()) => Response::ok("转发规则已删除", None),
        Err(e) => Response::err(-1, e),
    }
}

pub async fn blacklist_add(ip: String, _comment: String) -> Response {
    let ip = ip.trim().to_string();
    if let Err(e) = validate_blacklist_ip(&ip) {
        return Response::err(-1, e);
    }
    let b = detect();
    match apply_blacklist(b, &ip) {
        Ok(()) => {
            nft_repin_drop_if_locked(b);
            Response::ok("黑名单已添加", None)
        }
        Err(e) => Response::err(-1, e),
    }
}

pub async fn blacklist_delete(id: String) -> Response {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Response::err(-1, "缺少黑名单 id".to_string());
    }
    match delete_blacklist(detect(), &id) {
        Ok(()) => Response::ok("黑名单已删除", None),
        Err(e) => Response::err(-1, e),
    }
}

/// 白名单列表（放行优先，命中后不再被黑名单 / fail2ban 封禁）。
pub async fn whitelist_list() -> Response {
    let b = detect();
    let list: Vec<Value> = list_whitelist(b).iter().map(white_json).collect();
    Response::ok("ok", Some(json!({ "whitelist": list })))
}

pub async fn whitelist_add(ip: String, _comment: String) -> Response {
    let ip = ip.trim().to_string();
    if let Err(e) = validate_source(&ip) {
        return Response::err(-1, e);
    }
    let b = detect();
    match apply_whitelist(b, &ip) {
        Ok(()) => Response::ok("白名单已添加", None),
        Err(e) => Response::err(-1, e),
    }
}

pub async fn whitelist_delete(id: String) -> Response {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Response::err(-1, "缺少白名单 id".to_string());
    }
    match delete_whitelist(detect(), &id) {
        Ok(()) => Response::ok("白名单已删除", None),
        Err(e) => Response::err(-1, e),
    }
}

/// 导出全部规则为 JSON（可直接保存为文件，导入端原样回传 firewall.import）。
pub async fn export(_panel_port: u16) -> Response {
    let b = detect();
    let port_rules: Vec<Value> = list_rules(b)
        .iter()
        .map(|r| {
            json!({
                "port": r.port,
                "proto": r.proto,
                "action": r.action,
                "source": r.source,
                "comment": r.comment,
            })
        })
        .collect();
    let forwards: Vec<Value> = list_forwards(b)
        .iter()
        .map(|f| {
            json!({
                "proto": f.proto,
                "listen_port": f.listen_port,
                "target_addr": f.target_addr,
                "target_port": f.target_port,
                "comment": f.comment,
            })
        })
        .collect();
    let blacklist: Vec<Value> = list_blacklist(b)
        .iter()
        .map(|x| json!({ "ip": x.ip, "comment": x.comment }))
        .collect();
    Response::ok(
        "OK",
        Some(json!({
            "version": 1,
            "backend": b.name(),
            "port_rules": port_rules,
            "forwards": forwards,
            "blacklist": blacklist,
        })),
    )
}

/// 导入规则（firewall.export 的 data 结构）。逐条校验 + 应用，
/// 单条失败不影响其余，返回 added / skipped / failed 汇总。
pub async fn import(payload: Value, panel_port: u16) -> Response {
    if !payload.is_object() {
        return Response::err(-1, "导入文件格式不正确（需要 JSON 对象）".to_string());
    }
    let b = detect();
    let mut added: u32 = 0;
    let mut skipped: u32 = 0;
    let mut failed: Vec<String> = Vec::new();

    let arr = |key: &str| -> Vec<Value> {
        payload
            .get(key)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };

    // 端口规则
    for item in arr("port_rules") {
        let port = item.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
        let proto = item
            .get("proto")
            .and_then(|v| v.as_str())
            .unwrap_or("tcp")
            .trim()
            .to_lowercase();
        let action = item
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("accept")
            .trim()
            .to_lowercase();
        let source = item
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let comment = item
            .get("comment")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if let Err(e) = validate_rule_input(port, &proto, &action, &source) {
            failed.push(format!("端口 {port}: {e}"));
            continue;
        }
        // 与手工添加一致的面板端口保护
        if action != "accept" && hits_panel_port(port, panel_port) {
            skipped += 1;
            continue;
        }
        let rule = Rule {
            id: String::new(),
            port,
            proto,
            action,
            source,
            comment,
        };
        match apply_rule(b, &rule) {
            Ok(()) => added += 1,
            Err(e) => failed.push(format!("端口 {}: {}", rule.port, e)),
        }
    }

    // 端口转发
    for item in arr("forwards") {
        let proto = item
            .get("proto")
            .and_then(|v| v.as_str())
            .unwrap_or("tcp")
            .trim()
            .to_lowercase();
        let listen_port = item.get("listen_port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
        let target_addr = item
            .get("target_addr")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let target_port = item.get("target_port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
        let comment = item
            .get("comment")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if let Err(e) = validate_forward_input(&proto, listen_port, &target_addr, target_port, panel_port) {
            failed.push(format!("转发 {listen_port}: {e}"));
            continue;
        }
        let f = Forward {
            id: String::new(),
            proto,
            listen_port,
            target_addr,
            target_port,
            comment,
        };
        match apply_forward(b, &f) {
            Ok(()) => added += 1,
            Err(e) => failed.push(format!("转发 {}: {}", f.listen_port, e)),
        }
    }

    // IP 黑名单
    for item in arr("blacklist") {
        let ip = item
            .get("ip")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if let Err(e) = validate_blacklist_ip(&ip) {
            failed.push(format!("黑名单 {ip}: {e}"));
            continue;
        }
        match apply_blacklist(b, &ip) {
            Ok(()) => added += 1,
            Err(e) => failed.push(format!("黑名单 {ip}: {e}")),
        }
    }

    let mut message = format!("导入完成：成功 {added} 条");
    if skipped > 0 {
        message.push_str(&format!("，跳过 {skipped} 条（面板端口保护）"));
    }
    if !failed.is_empty() {
        message.push_str(&format!("，失败 {} 条", failed.len()));
    }
    nft_repin_drop_if_locked(detect());
    Response::ok(
        message,
        Some(json!({
            "added": added,
            "skipped": skipped,
            "failed": failed,
        })),
    )
}

// ── 安全加固（锁服）：封禁全部入站，仅开放白名单 ─────────────
//
// 防自锁铁律：
// 1) 永远最先放通 `ct state established,related`（已建连接），否则当前 SSH / 面板会话
//    会被瞬间掐断 —— 真·锁死。
// 2) 白名单强制包含 SSH(22) 与面板端口（server.port，由调用方传入），即便操作者忘记也加。
// 3) 先加放通规则、最后才设默认丢弃；任一步失败都不落得「策略是 drop 却没放通规则」。
// 4) 全部打 `zap:lock:*` 标记，可幂等重做、可一键恢复。

const LOCK_TAG: &str = "zap:lock:";

/// 计算白名单端口（去重：面板端口若已是 22/80/443 不重复），SSH 永远在列。
fn lock_allow_ports(panel_port: u16) -> Vec<u16> {
    let mut ports: Vec<u16> = vec![22, 80, 443];
    if panel_port != 0 && !ports.contains(&panel_port) {
        ports.push(panel_port);
    }
    ports
}

/// 加固白名单端口在端口规则列表里的备注：22→Ssh，80→http，443→https，面板端口→zap。
fn lock_port_remark(port: u16, panel_port: u16) -> String {
    if port == 22 {
        "Ssh".to_string()
    } else if port == 80 {
        "http".to_string()
    } else if port == 443 {
        "https".to_string()
    } else if port == panel_port {
        "zap".to_string()
    } else {
        //  unlikely：白名单里出现其它端口时直接以端口号标记
        format!("zap:lock:{port}")
    }
}

fn nft_lockdown(unlock: bool, panel_port: u16) -> Result<(), String> {
    nft_ensure_chains();
    // 清掉历史加固规则（幂等）
    for line in cmd_out("nft", &["-a", "list", "chain", "inet", "zap_fw", "input"]).lines() {
        let l = line.trim();
        if !l.contains(LOCK_TAG) {
            continue;
        }
        if let Some((_, h)) = l.split_once("# handle ") {
            let handle = h.split_whitespace().next().unwrap_or("");
            if !handle.is_empty() {
                let _ = root_cmd("nft")
                    .args(["delete", "rule", "inet", "zap_fw", "input", "handle", handle])
                    .output();
            }
        }
    }
    if unlock {
        return Ok(());
    }
    // 1) 已建连接放通（置顶，防自锁）
    let _ = root_cmd("nft")
        .args([
            "insert", "rule", "inet", "zap_fw", "input",
            "ct state established,related accept comment \"zap:lock:established\"",
        ])
        .output();
    // 1.5) 回环（loopback）放通：本地 127.0.0.1 / ::1 服务（如 127.0.0.1:10000）依赖它，
    //      否则锁服的兜底 drop 会一并拦掉本地回环流量
    let _ = root_cmd("nft")
        .args([
            "insert", "rule", "inet", "zap_fw", "input",
            "iifname \"lo\" accept comment \"zap:lock:lo\"",
        ])
        .output();
    // 2) ICMP（诊断 / PMTU，常规放开）
    let _ = root_cmd("nft")
        .args(["add", "rule", "inet", "zap_fw", "input", "ip protocol icmp accept comment \"zap:lock:icmp\""])
        .output();
    let _ = root_cmd("nft")
        .args(["add", "rule", "inet", "zap_fw", "input", "ip6 nexthdr icmpv6 accept comment \"zap:lock:icmp6\""])
        .output();
    // 3) 白名单端口（逐端口，便于单独编辑 / 限制来源；备注 Ssh/http/https/zap）
    for p in lock_allow_ports(panel_port) {
        let remark = lock_port_remark(p, panel_port);
        let comment = if remark.starts_with("zap:lock:") {
            remark
        } else {
            format!("zap:lock:{remark}")
        };
        let allow = format!("tcp dport {p} accept comment \"{comment}\"");
        let arefs: Vec<&str> = ["add", "rule", "inet", "zap_fw", "input", &allow]
            .iter()
            .copied()
            .collect();
        let (ok, out) = run_capture("nft", &arefs);
        if !ok {
            return Err(format!("nft 添加白名单端口 {p} 失败: {out}"));
        }
    }
    // 4) 最后追加「全部丢弃」兜底（位于已建连接 / 白名单之后）
    let _ = root_cmd("nft")
        .args(["add", "rule", "inet", "zap_fw", "input", "drop comment \"zap:lock:drop\""])
        .output();
    Ok(())
}

/// 删除 INPUT 链里带 zap:lock: 标记的规则（按行号从大到小删，避免偏移），并恢复默认 ACCEPT。
fn iptables_clear_lock() {
    for bin in ["iptables", "ip6tables"] {
        let out = cmd_out(bin, &["-L", "INPUT", "--line-numbers"]);
        let mut nums: Vec<u32> = out
            .lines()
            .filter(|l| l.contains(LOCK_TAG))
            .filter_map(|l| l.split_whitespace().next().and_then(|n| n.parse().ok()))
            .collect();
        nums.sort_unstable_by(|a, b| b.cmp(a));
        for n in nums {
            let _ = root_cmd(bin).args(["-D", "INPUT", &n.to_string()]).output();
        }
        let _ = root_cmd(bin).args(["-P", "INPUT", "ACCEPT"]).output();
    }
}

fn iptables_lockdown(unlock: bool, panel_port: u16) -> Result<(), String> {
    iptables_clear_lock();
    if unlock {
        return Ok(());
    }
    // 1) 已建连接放通（置顶）
    for bin in ["iptables", "ip6tables"] {
        let _ = root_cmd(bin)
            .args([
                "-I", "INPUT", "1", "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED",
                "-j", "ACCEPT", "-m", "comment", "--comment", "zap:lock:established",
            ])
            .output();
        // 1.5) 回环（loopback）放通：本地 127.0.0.1 / ::1 服务依赖它，否则锁服会拦掉回环流量
        let _ = root_cmd(bin)
            .args([
                "-I", "INPUT", "1", "-i", "lo", "-j", "ACCEPT", "-m", "comment",
                "--comment", "zap:lock:lo",
            ])
            .output();
    }
    // 2) 白名单（逐端口，便于单独编辑 / 限制来源）
    for bin in ["iptables", "ip6tables"] {
        for p in lock_allow_ports(panel_port) {
            let remark = lock_port_remark(p, panel_port);
            let comment = if remark.starts_with("zap:lock:") {
                remark
            } else {
                format!("zap:lock:{remark}")
            };
            let ok = cmd_ok(
                bin,
                &[
                    "-A", "INPUT", "-p", "tcp", "--dport", &p.to_string(),
                    "-j", "ACCEPT", "-m", "comment", "--comment", &comment,
                ],
            );
            if !ok {
                return Err(format!("{bin} 添加白名单端口 {p} 失败"));
            }
        }
    }
    let _ = cmd_ok("iptables", &["-A", "INPUT", "-p", "icmp", "-j", "ACCEPT", "-m", "comment", "--comment", "zap:lock:icmp"]);
    let _ = cmd_ok("ip6tables", &["-A", "INPUT", "-p", "icmpv6", "-j", "ACCEPT", "-m", "comment", "--comment", "zap:lock:icmp6"]);
    // 3) 默认丢弃（放在放通规则之后；失败则保持放开，不锁死）
    let _ = root_cmd("iptables").args(["-P", "INPUT", "DROP"]).output();
    let _ = root_cmd("ip6tables").args(["-P", "INPUT", "DROP"]).output();
    // 4) 兜底 drop 规则（便于状态检测，且与 nftables 语义一致）
    let _ = cmd_ok(
        "iptables",
        &["-A", "INPUT", "-j", "DROP", "-m", "comment", "--comment", "zap:lock:drop"],
    );
    Ok(())
}

fn firewalld_lockdown(unlock: bool, panel_port: u16) -> Result<(), String> {
    if unlock {
        // 恢复默认 target，并移除我们加的面板端口
        let _ = run_capture(
            "firewall-cmd",
            &["--permanent", "--zone", "public", "--set-target", "default"],
        );
        if panel_port != 0 {
            let _ = run_capture(
                "firewall-cmd",
                &["--permanent", "--zone", "public", "--remove-port", &format!("{panel_port}/tcp")],
            );
        }
        let _ = run_capture("firewall-cmd", &["--reload"]);
        return Ok(());
    }
    // 仅开放白名单：ssh / http / https 服务 + 面板端口；其余默认拒绝（target=DROP）
    for svc in ["ssh", "http", "https"] {
        let _ = run_capture("firewall-cmd", &["--permanent", "--add-service", svc]);
    }
    if panel_port != 0 && !matches!(panel_port, 22 | 80 | 443) {
        let (ok, out) = run_capture(
            "firewall-cmd",
            &["--permanent", "--add-port", &format!("{panel_port}/tcp")],
        );
        if !ok {
            return Err(format!("firewall-cmd 添加面板端口失败：{out}"));
        }
    }
    let (ok, out) = run_capture(
        "firewall-cmd",
        &["--permanent", "--zone", "public", "--set-target", "DROP"],
    );
    if !ok {
        return Err(format!("firewall-cmd 设置默认拒绝失败：{out}"));
    }
    let (ok, out) = run_capture("firewall-cmd", &["--reload"]);
    if !ok {
        return Err(format!("firewall-cmd --reload 执行失败：{out}"));
    }
    Ok(())
}

fn ufw_lockdown(unlock: bool, panel_port: u16) -> Result<(), String> {
    if unlock {
        // 恢复默认放行（已加的 allow 规则保留，不删，避免误删用户既有规则）
        let (ok, out) = run_capture("ufw", &["default", "allow", "incoming"]);
        if !ok {
            return Err(format!("ufw 恢复默认放行失败：{out}"));
        }
        return Ok(());
    }
    // 先放通白名单，再切默认拒绝 —— 顺序错了会锁 SSH
    for p in lock_allow_ports(panel_port) {
        let _ = run_capture("ufw", &["allow", &format!("{p}/tcp")]);
    }
    let (ok, out) = run_capture("ufw", &["default", "deny", "incoming"]);
    if !ok {
        return Err(format!("ufw 设置默认拒绝失败：{out}"));
    }
    Ok(())
}

fn apply_lockdown(b: Backend, unlock: bool, panel_port: u16) -> Result<(), String> {
    match b {
        Backend::Firewalld => firewalld_lockdown(unlock, panel_port),
        Backend::Nftables => nft_lockdown(unlock, panel_port),
        Backend::Iptables => iptables_lockdown(unlock, panel_port),
        Backend::Ufw => ufw_lockdown(unlock, panel_port),
        _ => Err(format!("当前系统（{}）暂不支持该操作", std::env::consts::OS)),
    }
}

/// 一键锁服：封禁全部入站，仅开放 SSH / 80 / 443 / 面板端口。
pub async fn lockdown(unlock: bool, panel_port: u16) -> Response {
    let ports = lock_allow_ports(panel_port);
    match apply_lockdown(detect(), unlock, panel_port) {
        Ok(()) => {
            let msg = if unlock {
                "已恢复默认放行（移除锁服规则）".to_string()
            } else {
                format!("已锁服：仅开放端口 {}", ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" / "))
            };
            Response::ok(msg, None)
        }
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
