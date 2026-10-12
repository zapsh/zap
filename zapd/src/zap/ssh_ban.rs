// SPDX-License-Identifier: AGPL-3.0-only
//! IP Ban ：
//!
//! 每条规则 = 一个日志来源 + 一个匹配模式 + 一组阈值：
//! - `auto`            自动探测 sshd 日志（auth.log / secure，缺省转 journal）
//! - `journal:<units>` journalctl 按 unit 逗号分隔增量读（如 `journal:ssh,sshd`）
//! - `file:<path>`     任意文本日志文件（按字节偏移增量读）
//! - `zapd`            面板自身登录失败，由登录流程进程内上报（不解析日志）
//!
//! 匹配模式：留空 = 内置 sshd 失败解析；填正则则按正则匹配行，
//! 可用命名捕获组 `(?<ip>...)` 指定 IP 位置，否则取行内第一个 IP。
//!
//! 规则落在防火墙后端（复用 `firewall.blacklist_*` 动词），计数状态存 KV。
//! KV key 带 `_v2`：结构升级时直接换 key，旧配置（含旧的 ignore_private=true）
//! 一并作废，避免历史值卡住新默认行为。
//!
//! 关键取舍：
//! - 白名单（防火墙白名单 + 设置里的忽略 IP）永不被封；
//! - 默认不忽略内网 / 环回，让功能对 LAN 来源的爆破也生效（跳板机请加白名单）；
//! - 封禁动作走 zapexec，解封时按 IP 反查规则 id，兼容各后端不同的 id 形态。

use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::db::get_db_pool;
use crate::zapexec;
use zap_proto::Request;

/// v2：结构升级换 key，旧配置作废（旧 ignore_private=true 不会再卡住新默认）
const KEY_SETTINGS: &str = "firewall.ban_settings_v2";
const KEY_BANS: &str = "firewall.ban_records_v2";
const KEY_STATE: &str = "firewall.ban_state_v2";
const KEY_WHITELIST: &str = "firewall.whitelist";

/// hits / ignored 的复合键分隔符（rule id 里不会出现）
const KEY_SEP: char = '\u{1}';

fn def_ban_time() -> u32 {
    3600
}
fn def_interval() -> u32 {
    60
}
/// 默认「不忽略内网」：让功能对 LAN 来源的爆破也生效（跳板机请加白名单）。
fn def_ignore_private() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanRule {
    /// 稳定 id（计数 / 展示的键）
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "def_true")]
    pub enabled: bool,
    /// `auto` | `journal:<units>` | `file:<path>` | `zapd`
    #[serde(default)]
    pub source: String,
    /// 匹配正则；留空 = 内置 sshd 失败解析。命名捕获组 `(?<ip>...)` 指定 IP 位置。
    #[serde(default)]
    pub pattern: String,
    /// 窗口内失败几次即封禁
    #[serde(default = "def_max_retry")]
    pub max_retry: u32,
    /// 计数窗口（秒）
    #[serde(default = "def_find_time")]
    pub find_time: u32,
}

fn def_true() -> bool {
    true
}
fn def_max_retry() -> u32 {
    5
}
fn def_find_time() -> u32 {
    600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanSettings {
    #[serde(default)]
    pub enabled: bool,
    /// 封禁时长（秒），到期自动解封
    #[serde(default = "def_ban_time")]
    pub ban_time: u32,
    /// 扫描间隔（秒）
    #[serde(default = "def_interval")]
    pub interval: u32,
    /// 忽略内网 / 环回地址（默认关：让功能对 LAN 爆破也生效，跳板机请加白名单）
    #[serde(default = "def_ignore_private")]
    pub ignore_private: bool,
    /// 额外忽略的 IP / CIDR
    #[serde(default)]
    pub ignored_ips: Vec<String>,
    /// 多条封禁规则（sshd / 面板 / 自定义日志）
    #[serde(default)]
    pub rules: Vec<BanRule>,
}

impl Default for BanSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            ban_time: def_ban_time(),
            interval: def_interval(),
            ignore_private: def_ignore_private(),
            ignored_ips: Vec::new(),
            rules: vec![
                BanRule {
                    id: "sshd".into(),
                    name: "SSH 登录失败".into(),
                    enabled: true,
                    source: "auto".into(),
                    pattern: String::new(),
                    max_retry: 5,
                    find_time: 600,
                },
                BanRule {
                    id: "zapd".into(),
                    name: "面板登录失败".into(),
                    enabled: true,
                    source: "zapd".into(),
                    pattern: String::new(),
                    max_retry: 10,
                    find_time: 600,
                },
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanRecord {
    pub ip: String,
    /// 触发封禁的规则名
    #[serde(default)]
    pub rule: String,
    pub reason: String,
    /// 触发时的失败次数
    pub hits: u32,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BanState {
    pub last_run: u64,
    pub total_banned: u64,
    pub last_error: String,
    /// 窗口内各 (规则, IP) 的失败计数 `"{rule}\u{1}{ip}" -> (次数, 最近一次时间)`
    #[serde(default)]
    pub hits: std::collections::HashMap<String, (u32, u64)>,
    /// 命中失败但被策略跳过（内网 / 白名单）的来源，用于面板解释「为什么没封」
    #[serde(default)]
    pub ignored: std::collections::HashMap<String, (u32, String, u64)>,
    /// 各 journal 来源的读取游标 `source_key -> cursor`
    #[serde(default)]
    pub cursors: std::collections::HashMap<String, String>,
    /// 各日志文件的字节偏移 `path -> offset`
    #[serde(default)]
    pub offsets: std::collections::HashMap<String, u64>,
    /// 各规则最近的实际日志来源（面板展示）`rule_id -> 描述`
    #[serde(default)]
    pub sources: std::collections::HashMap<String, String>,
}

fn hit_key(rule: &str, ip: &str) -> String {
    format!("{rule}{KEY_SEP}{ip}")
}

/// 拆复合键；兼容无分隔符的历史键
fn split_key(k: &str) -> (&str, &str) {
    k.split_once(KEY_SEP).unwrap_or(("", k))
}

/// 供状态接口拆复合键（rule, ip）
pub fn split_hit_key(k: &str) -> (&str, &str) {
    split_key(k)
}

// ── zapd 进程内上报 ────────────────────────────────────────────

/// 面板登录失败上报队列：(rule_id, ip, 附加行文本)。
/// 登录处理路径里只做内存入队，实际计数与封禁由 tick 统一处理。
static REPORTS: LazyLock<Mutex<Vec<(String, String, String)>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// 面板登录失败时调用（auth 流程），rule_id 用 `zapd`。
pub fn report_failure(rule_id: &str, ip: &str, line: &str) {
    if let Ok(mut q) = REPORTS.lock() {
        q.push((rule_id.to_string(), ip.to_string(), line.to_string()));
        // 队列兜底限长，异常情况下也不至于无限涨
        if q.len() > 10_000 {
            let drop = q.len() - 10_000;
            q.drain(..drop);
        }
    }
}

fn drain_reports() -> Vec<(String, String, String)> {
    REPORTS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

// ── KV ─────────────────────────────────────────────────────────

async fn kv_get(key: &str) -> String {
    let pool = get_db_pool().await;
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM global_settings WHERE key=?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    row.map(|r| r.0).unwrap_or_default()
}

async fn kv_set(key: &str, value: &str) {
    let pool = get_db_pool().await;
    let _ = sqlx::query(
        "INSERT INTO global_settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await;
}

// ── 对外读写 ───────────────────────────────────────────────────

pub async fn settings() -> BanSettings {
    serde_json::from_str(&kv_get(KEY_SETTINGS).await).unwrap_or_default()
}

pub async fn save_settings(s: &BanSettings) {
    let mut s = s.clone();
    s.ban_time = s.ban_time.clamp(60, 7 * 86_400);
    s.interval = s.interval.clamp(10, 3_600);
    // 规则收敛：id 去空去重、阈值夹紧、非法正则的规则直接丢弃
    let mut seen = std::collections::HashSet::new();
    s.rules.retain_mut(|r| {
        r.id = r.id.trim().to_string();
        if r.id.is_empty() || !seen.insert(r.id.clone()) {
            return false;
        }
        r.name = r.name.trim().to_string();
        if r.name.is_empty() {
            r.name = r.id.clone();
        }
        r.max_retry = r.max_retry.clamp(1, 100);
        r.find_time = r.find_time.clamp(30, 86_400);
        r.pattern = r.pattern.trim().to_string();
        if !r.pattern.is_empty() && Regex::new(&r.pattern).is_err() {
            return false; // 非法正则
        }
        true
    });
    if let Ok(v) = serde_json::to_string(&s) {
        kv_set(KEY_SETTINGS, &v).await;
    }
}

pub async fn records() -> Vec<BanRecord> {
    serde_json::from_str(&kv_get(KEY_BANS).await).unwrap_or_default()
}

async fn save_records(list: &[BanRecord]) {
    if let Ok(v) = serde_json::to_string(list) {
        kv_set(KEY_BANS, &v).await;
    }
}

pub async fn state() -> BanState {
    serde_json::from_str(&kv_get(KEY_STATE).await).unwrap_or_default()
}

/// 白名单 IP 集合（面板级，与防火墙白名单是两回事：
/// 这里是「不允许被封禁」的策略清单，含设置里追加的忽略 IP）。
pub async fn protected_ips() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&kv_get(KEY_WHITELIST).await) {
        for it in list {
            if let Some(ip) = it.get("ip").and_then(|v| v.as_str()) {
                out.push(ip.to_string());
            }
        }
    }
    for ip in settings().await.ignored_ips {
        out.push(ip);
    }
    out
}

/// 手动提前解封（面板按钮）：删防火墙规则 + 清记录。
pub async fn unban(ip: &str) -> Result<(), String> {
    let ip = ip.trim().to_string();
    if ip.is_empty() {
        return Err("缺少 IP".to_string());
    }
    remove_firewall_rule(&ip).await?;
    let list: Vec<BanRecord> = records().await.into_iter().filter(|b| b.ip != ip).collect();
    save_records(&list).await;
    Ok(())
}

// ── 日志来源解析 ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Src {
    /// sshd 自动探测：有文件走文件，否则走 journal（unit: ssh / sshd）
    Auto,
    Journal(Vec<String>),
    File(String),
}

fn resolve_source(source: &str) -> Src {
    let t = source.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("auto") {
        return Src::Auto;
    }
    if let Some(units) = t.strip_prefix("journal:") {
        let units: Vec<String> = units
            .split(',')
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .collect();
        return if units.is_empty() {
            Src::Auto
        } else {
            Src::Journal(units)
        };
    }
    if wants_journal(t) {
        return Src::Journal(vec!["ssh".into(), "sshd".into()]);
    }
    if let Some(p) = t.strip_prefix("file:") {
        let p = p.trim();
        return if p.is_empty() { Src::Auto } else { Src::File(p.into()) };
    }
    // 兼容：直接填了路径
    Src::File(t.to_string())
}

fn journal_available() -> bool {
    std::process::Command::new("journalctl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 显式要求走 journal（Debian 默认不装 rsyslog，auth.log 不存在）。
fn wants_journal(configured: &str) -> bool {
    matches!(
        configured.trim().to_ascii_lowercase().as_str(),
        "journal" | "journalctl" | "systemd-journal"
    )
}

/// 读 journal 里指定 units 的新条目，返回 `(消息行, 新游标)`。
///
/// 用游标增量读，等价于文件模式的字节偏移；首次（无游标）只取最近若干条，
/// 避免开机以来全部历史被重放导致误封。
fn read_journal(units: &[String], cursor: &str, limit: usize) -> Result<(Vec<String>, String), String> {
    let mut cmd = std::process::Command::new("journalctl");
    for u in units {
        cmd.arg("-u").arg(u);
    }
    cmd.args([
        "-o",
        "json",
        "--no-pager",
        "-n",
        &limit.to_string(),
    ]);
    cmd.env("LC_ALL", "C");
    if !cursor.is_empty() {
        cmd.arg("--after-cursor").arg(cursor);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("执行 journalctl 失败: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // 游标失效（日志被 vacuum / 轮转清掉）时丢游标重试一次，取最近若干条
        if !cursor.is_empty() {
            return read_journal(units, "", limit);
        }
        return Err(if err.is_empty() {
            "journalctl 返回非零".to_string()
        } else {
            err
        });
    }
    let mut lines = Vec::new();
    let mut new_cursor = cursor.to_string();
    for raw in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
            continue;
        };
        if let Some(c) = v.get("__CURSOR").and_then(|x| x.as_str()) {
            new_cursor = c.to_string();
        }
        // 多行消息 journal 会拆成数组，这里拼回来，保证解析看到完整一行
        let msg = match v.get("MESSAGE") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => continue,
        };
        if !msg.is_empty() {
            lines.push(msg);
        }
    }
    Ok((lines, new_cursor))
}

/// 探测 sshd 文本日志（RHEL / 装了 rsyslog 的 Debian）。
/// 留空时按常见路径探测；填了路径但不存在则返回 None（交给调用方报错）。
fn detect_log_file(configured: &str) -> Option<String> {
    if !configured.trim().is_empty() {
        let p = std::path::Path::new(configured.trim());
        return p.exists().then(|| p.to_string_lossy().to_string());
    }
    for p in [
        "/var/log/auth.log",
        "/var/log/secure",
        "/var/log/auth.log.1",
        "/var/log/secure.1",
    ] {
        if std::path::Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    None
}

// ── 行匹配 ─────────────────────────────────────────────────────

/// 该行是否是 sshd 认证失败，并取出失败来源 IP。
///
/// 覆盖常见形态（Debian / RHEL / Alpine 的 sshd 日志措辞基本一致）：
/// - `Failed password for [invalid user] x from 1.2.3.4 port 22 ssh2`
/// - `Invalid user x from 1.2.3.4 port 22`
/// - `Maximum authentication attempts exceeded for ... from 1.2.3.4`
/// - `Disconnected from authenticating user x 1.2.3.4 port 22 [preauth]`
/// - `Connection closed by authenticating user x 1.2.3.4 port 22 [preauth]`
fn parse_failure(line: &str) -> Option<String> {
    let l = line.to_ascii_lowercase();
    let hit = l.contains("failed password")
        || l.contains("failed publickey")
        || l.contains("failed none")
        || l.contains("failed keyboard-interactive")
        || l.contains("invalid user")
        || l.contains("authentication failure")
        || l.contains("more authentication failures")
        || l.contains("maximum authentication attempts exceeded")
        || l.contains("connection closed by authenticating user")
        || l.contains("disconnected from authenticating user")
        || l.contains("timeout before authentication");
    if !hit {
        return None;
    }
    // 形态一：`... from <ip> port ...`（绝大多数）
    if let Some(after) = l.split(" from ").nth(1) {
        let tok = after
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(|c: char| c == ',' || c == ';');
        if tok.parse::<std::net::IpAddr>().is_ok() {
            return Some(tok.to_string());
        }
    }
    // 形态二：`... authenticating user <name> <ip> port ... [preauth]`（没有 from 字段）
    if let Some(idx) = l.find("authenticating user") {
        let rest = &l[idx + "authenticating user".len()..];
        for tok in rest.split_whitespace().skip(1) {
            let tok = tok.trim_end_matches(|c: char| c == ',' || c == ';');
            if tok.parse::<std::net::IpAddr>().is_ok() {
                return Some(tok.to_string());
            }
        }
    }
    // 形态三：PAM 行 `... rhost=1.2.3.4 user=...`（没有 from / authenticating user）
    if let Some(idx) = l.find("rhost=") {
        let tok = l[idx + "rhost=".len()..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(|c: char| c == ',' || c == ';');
        if tok.parse::<std::net::IpAddr>().is_ok() {
            return Some(tok.to_string());
        }
    }
    None
}

/// 行内第一个看起来像 IP 的 token（正则没写捕获组时的兜底提取）。
fn first_ip_in_line(line: &str) -> Option<String> {
    for tok in line.split_whitespace() {
        let tok = tok.trim_end_matches(|c: char| c == ',' || c == ';' || c == ':');
        if tok.parse::<std::net::IpAddr>().is_ok() {
            return Some(tok.to_string());
        }
    }
    None
}

/// 按规则匹配一行，返回来源 IP。
/// pattern 为空 → 内置 sshd 解析；有 pattern → 正则匹配，
/// 命名捕获组 `(?<ip>...)` 优先，否则取行内第一个 IP。
fn apply_line(re: Option<&Regex>, line: &str) -> Option<String> {
    match re {
        None => parse_failure(line),
        Some(re) => {
            let caps = re.captures(line)?;
            if let Some(m) = caps.name("ip") {
                let ip = m.as_str();
                return ip.parse::<std::net::IpAddr>().is_ok().then(|| ip.to_string());
            }
            first_ip_in_line(line)
        }
    }
}

// ── 策略 ───────────────────────────────────────────────────────

/// 内网 / 环回 / 链路本地。
fn is_private_ip(ip: &str) -> bool {
    let Ok(a) = ip.parse::<std::net::IpAddr>() else {
        return true; // 解析不了的一律按需保护处理
    };
    match a {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.octets()[0] == 0
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

fn ip_in_cidr(ip: &str, cidr: &str) -> bool {
    let cidr = cidr.trim();
    if cidr == ip {
        return true;
    }
    let Some((net, bits)) = cidr.split_once('/') else {
        return false;
    };
    let Ok(bits) = bits.parse::<u32>() else {
        return false;
    };
    match (ip.parse::<std::net::IpAddr>(), net.parse::<std::net::IpAddr>()) {
        (Ok(std::net::IpAddr::V4(a)), Ok(std::net::IpAddr::V4(b))) => {
            if bits > 32 {
                return false;
            }
            let mask: u32 = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
            u32::from(a) & mask == u32::from(b) & mask
        }
        (Ok(std::net::IpAddr::V6(a)), Ok(std::net::IpAddr::V6(b))) => {
            if bits > 128 {
                return false;
            }
            let mask: u128 = if bits == 0 { 0 } else { u128::MAX << (128 - bits) };
            u128::from(a) & mask == u128::from(b) & mask
        }
        _ => false,
    }
}

/// 跳过原因：None = 不跳过；Some(原因) = 命中保护策略，不会封禁。
async fn skip_reason(ip: &str, s: &BanSettings, wl: &[String]) -> Option<String> {
    if ip == "127.0.0.1" || ip == "::1" {
        return Some("环回地址".to_string());
    }
    if s.ignore_private && is_private_ip(ip) {
        return Some("内网地址（已开启忽略内网）".to_string());
    }
    if let Some(w) = wl.iter().find(|w| ip_in_cidr(ip, w)) {
        return Some(format!("白名单 / 忽略清单（{w}）"));
    }
    None
}

// ── 防火墙动作 ─────────────────────────────────────────────────

/// 封禁：直接调 zapexec 动词（不走面板 handler，避免「不能封当前登录 IP」的
/// 自锁保护把爆破源挡在门外；自锁防护由白名单承担）。
async fn ban_firewall(ip: &str) -> Result<(), String> {
    let resp = zapexec::call(Request::FirewallBlacklistAdd {
        ip: ip.to_string(),
        comment: format!("zap:ban:{ip}"),
    })
    .await
    .map_err(|e| e.to_string())?;
    if resp.code != 0 {
        return Err(resp.message);
    }
    Ok(())
}

/// 解封：各后端的规则 id 形态不同（nft=handle、firewalld=富规则串、其余=IP），
/// 因此先拉一次状态按 IP 反查 id 再删。
async fn remove_firewall_rule(ip: &str) -> Result<(), String> {
    let panel_port = crate::config::get_config()
        .read()
        .map(|c| c.server.port)
        .unwrap_or(0);
    let resp = zapexec::call(Request::FirewallStatus {
        panel_port,
    })
    .await
    .map_err(|e| e.to_string())?;
    if resp.code != 0 {
        return Err(resp.message);
    }
    let data = resp.data.unwrap_or(serde_json::Value::Null);
    let Some(list) = data.get("blacklist").and_then(|v| v.as_array()) else {
        return Err("读取防火墙黑名单失败".to_string());
    };
    let Some(item) = list.iter().find(|x| {
        x.get("ip")
            .and_then(|v| v.as_str())
            .map(|s| s == ip)
            .unwrap_or(false)
    }) else {
        // 规则已不在（手工删过 / 防火墙重启丢规则）：视为已解封
        return Ok(());
    };
    let id = item
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or(ip)
        .to_string();
    let resp = zapexec::call(Request::FirewallBlacklistDelete { id })
        .await
        .map_err(|e| e.to_string())?;
    if resp.code != 0 {
        return Err(resp.message);
    }
    Ok(())
}

// ── 主循环 ─────────────────────────────────────────────────────

/// 启动 IP Ban 循环（后台任务）。默认关闭，用户在面板开启后生效。
pub fn start() {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(5)).await;
        loop {
            let s = settings().await;
            if s.enabled {
                tick(&s).await;
            }
            tokio::time::sleep(Duration::from_secs(s.interval.max(10) as u64)).await;
        }
    });
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn rule_max_retry(r: &BanRule) -> u32 {
    if r.max_retry == 0 { 5 } else { r.max_retry }
}
fn rule_find_time(r: &BanRule) -> u64 {
    (if r.find_time == 0 { 600 } else { r.find_time }) as u64
}

/// 读一个文件来源的新行（按字节偏移增量），返回 (行, 是否出错)
fn read_file_source(path: &str, st: &mut BanState) -> Result<Vec<String>, String> {
    // 日志轮转 / 文件被清空：偏移比文件还大就从头读
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let offset = st.offsets.get(path).copied().unwrap_or(0);
    let offset = if offset > len { 0 } else { offset };
    let content = std::fs::File::open(path)
        .and_then(|mut f| {
            use std::io::{Read, Seek, SeekFrom};
            f.seek(SeekFrom::Start(offset))?;
            let mut buf = String::new();
            f.read_to_string(&mut buf)?;
            Ok(buf)
        })
        .map_err(|e| format!("读取 {path} 失败: {e}"))?;
    // 只按完整行推进偏移，避免半行被截断后下一轮解析出错
    let consumed = content.rfind('\n').map(|i| i + 1).unwrap_or(0);
    st.offsets.insert(path.to_string(), offset + consumed as u64);
    Ok(content[..consumed].lines().map(|l| l.to_string()).collect())
}

async fn tick(s: &BanSettings) {
    let mut st = state().await;
    let ts = now();
    st.last_run = ts;
    let wl = protected_ips().await;

    // 到期解封
    let mut list = records().await;
    let mut expired: Vec<String> = Vec::new();
    list.retain(|b| {
        if b.expires_at <= ts {
            expired.push(b.ip.clone());
            return false;
        }
        true
    });
    for ip in &expired {
        if let Err(e) = remove_firewall_rule(ip).await {
            warn!("IP Ban 解封 {ip} 失败: {e}");
            st.last_error = format!("解封 {ip} 失败: {e}");
        } else {
            info!("IP Ban 到期解封: {ip}");
        }
    }

    // 每条启用规则编译一次正则
    let mut new_failures: Vec<(String, String)> = Vec::new(); // (rule_id, ip)
    let mut rules_err: Vec<String> = Vec::new();

    for rule in s.rules.iter().filter(|r| r.enabled) {
        let re = if rule.pattern.is_empty() {
            None
        } else {
            match Regex::new(&rule.pattern) {
                Ok(re) => Some(re),
                Err(e) => {
                    rules_err.push(format!("规则 {} 正则无效: {e}", rule.name));
                    continue;
                }
            }
        };

        if rule.source.trim() == "zapd" {
            // 面板登录失败：从进程内队列取（line = 用户名，可被正则二次过滤）
            for (rid, ip, line) in drain_reports() {
                if rid != rule.id {
                    continue;
                }
                if let Some(re) = &re {
                    if !re.is_match(&line) {
                        continue;
                    }
                }
                new_failures.push((rule.id.clone(), ip));
            }
            st.sources.insert(rule.id.clone(), "面板登录".to_string());
            continue;
        }

        // 日志来源
        let lines = match resolve_source(&rule.source) {
            Src::Auto => {
                if let Some(path) = detect_log_file("") {
                    st.sources.insert(rule.id.clone(), path.clone());
                    read_file_source(&path, &mut st)
                } else if journal_available() {
                    let units = vec!["ssh".to_string(), "sshd".to_string()];
                    st.sources
                        .insert(rule.id.clone(), "journalctl -u ssh".to_string());
                    read_journal(&units, st.cursors.get("auto").map(|s| s.as_str()).unwrap_or(""), 500)
                        .map(|(ls, cur)| {
                            st.cursors.insert("auto".to_string(), cur);
                            ls
                        })
                } else {
                    Err("未找到 sshd 日志（auth.log / secure），系统也没有 journalctl".to_string())
                }
            }
            Src::Journal(units) => {
                let key = format!("journal:{}", units.join(","));
                st.sources
                    .insert(rule.id.clone(), format!("journalctl -u {}", units.join(" -u ")));
                read_journal(&units, st.cursors.get(&key).map(|s| s.as_str()).unwrap_or(""), 500)
                    .map(|(ls, cur)| {
                        st.cursors.insert(key, cur);
                        ls
                    })
            }
            Src::File(path) => {
                st.sources.insert(rule.id.clone(), path.clone());
                read_file_source(&path, &mut st)
            }
        };

        match lines {
            Ok(ls) => {
                for line in ls {
                    if let Some(ip) = apply_line(re.as_ref(), &line) {
                        new_failures.push((rule.id.clone(), ip));
                    }
                }
            }
            Err(e) => rules_err.push(format!("规则 {}（{}）: {e}", rule.name, rule.source)),
        }
    }

    if !rules_err.is_empty() {
        st.last_error = rules_err.join("；");
    }

    // 累加窗口计数
    for (rid, ip) in new_failures {
        let e = st.hits.entry(hit_key(&rid, &ip)).or_insert((0, ts));
        e.0 += 1;
        e.1 = ts;
    }
    // 窗口滑动：按各规则自己的 find_time 淘汰旧计数
    st.hits.retain(|k, (_, at)| {
        let (rid, _) = split_key(k);
        let ft = s
            .rules
            .iter()
            .find(|r| r.id == rid)
            .map(rule_find_time)
            .unwrap_or(600);
        ts.saturating_sub(*at) <= ft
    });

    // 被策略跳过的来源单独记账，面板据此解释「为什么没封它」
    st.ignored
        .retain(|k, (_, _, at)| {
            let (rid, _) = split_key(k);
            let ft = s
                .rules
                .iter()
                .find(|r| r.id == rid)
                .map(rule_find_time)
                .unwrap_or(600);
            ts.saturating_sub(*at) <= ft
        });
    for (key, (count, _)) in st.hits.clone() {
        let (_, ip) = split_key(&key);
        if let Some(reason) = skip_reason(ip, s, &wl).await {
            let e = st.ignored.entry(key.clone()).or_insert((0, reason, ts));
            e.0 = e.0.max(count);
            e.2 = ts;
            // 已跳过的来源不参与封禁计数
            st.hits.remove(&key);
        }
    }
    // ignored 表兜底限长，避免异常日志打爆存储：只保留失败次数最多的前 100 个
    if st.ignored.len() > 100 {
        let mut ranked: Vec<(String, u32)> = st
            .ignored
            .iter()
            .map(|(k, (n, _, _))| (k.clone(), *n))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        let keep: std::collections::HashSet<String> =
            ranked.into_iter().take(100).map(|(k, _)| k).collect();
        st.ignored.retain(|k, _| keep.contains(k));
    }

    let mut new_bans = 0u32;
    let already: Vec<String> = list.iter().map(|b| b.ip.clone()).collect();
    for (key, (count, _)) in st.hits.clone() {
        let (rid, ip) = split_key(&key);
        let ip = ip.to_string();
        let Some(rule) = s.rules.iter().find(|r| r.id == rid) else {
            continue;
        };
        let threshold = rule_max_retry(rule);
        if count < threshold {
            debug!("IP Ban：{ip} 失败 {count} 次，未达规则 {} 阈值 {threshold}", rule.name);
            continue;
        }
        if already.contains(&ip) || skip_reason(&ip, s, &wl).await.is_some() {
            continue;
        }
        match ban_firewall(&ip).await {
            Ok(()) => {
                list.push(BanRecord {
                    ip: ip.clone(),
                    rule: rule.name.clone(),
                    reason: format!("[{}] 认证失败 {count} 次 / {}s", rule.name, rule.find_time),
                    hits: count,
                    created_at: ts,
                    expires_at: ts + s.ban_time as u64,
                });
                st.total_banned += 1;
                new_bans += 1;
                // 已封的 IP 不再重复计数
                st.hits.remove(&key);
                info!("IP Ban 封禁 {ip}（规则 {}，失败 {count} 次）", rule.name);
            }
            Err(e) => {
                warn!("IP Ban 封禁 {ip} 失败: {e}");
                st.last_error = format!("封禁 {ip} 失败: {e}");
            }
        }
    }

    if new_bans > 0 {
        st.last_error = String::new();
    }
    save_records(&list).await;
    kv_set(KEY_STATE, &serde_json::to_string(&st).unwrap_or_default()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_failed_password_invalid_user() {
        let line = "Jun 12 10:00:01 host sshd[123]: Failed password for invalid user root from 1.2.3.4 port 54321 ssh2";
        assert_eq!(parse_failure(line).as_deref(), Some("1.2.3.4"));
    }

    #[test]
    fn parses_invalid_user_line() {
        let line = "Jun 12 10:00:02 host sshd[124]: Invalid user admin from 2001:db8::1 port 22";
        assert_eq!(parse_failure(line).as_deref(), Some("2001:db8::1"));
    }

    #[test]
    fn parses_max_attempts_and_preauth() {
        assert_eq!(
            parse_failure("error: maximum authentication attempts exceeded for root from 8.8.8.8 port 22 ssh2 [preauth]").as_deref(),
            Some("8.8.8.8"),
        );
        assert_eq!(
            parse_failure("Connection closed by authenticating user root 9.9.9.9 port 5555 [preauth]").as_deref(),
            Some("9.9.9.9"),
        );
        assert_eq!(
            parse_failure("Disconnected from authenticating user admin 10.1.2.3 port 4444 [preauth]").as_deref(),
            Some("10.1.2.3"),
        );
    }

    #[test]
    fn parses_journal_json_message() {
        // journalctl -o json 的真实形态：MESSAGE 是字段，失败原因在其中
        let raw = r#"{"__CURSOR":"c=1","PRIORITY":"3","SYSLOG_IDENTIFIER":"sshd","MESSAGE":"Failed password for invalid user root from 1.2.3.4 port 51000 ssh2"}"#;
        let v: serde_json::Value = serde_json::from_str(raw).unwrap();
        let msg = v.get("MESSAGE").and_then(|x| x.as_str()).unwrap();
        assert_eq!(parse_failure(msg).as_deref(), Some("1.2.3.4"));
    }

    #[test]
    fn journal_magic_values_are_recognized() {
        assert!(wants_journal("journal"));
        assert!(wants_journal("JournalCtl"));
        assert!(wants_journal(" systemd-journal "));
        assert!(!wants_journal(""));
        assert!(!wants_journal("/var/log/auth.log"));
    }

    #[test]
    fn ignores_success_and_noise() {
        assert!(parse_failure("Accepted password for root from 1.2.3.4 port 22 ssh2").is_none());
        assert!(parse_failure("Server listening on 0.0.0.0 port 22.").is_none());
        assert!(parse_failure("Failed password for root from not-an-ip port 22").is_none());
    }

    #[test]
    fn parses_pam_rhost_and_timeout_lines() {
        // PAM 累计行：没有 from 字段，来源在 rhost=
        assert_eq!(
            parse_failure("pam_unix(sshd:auth): authentication failure; logname= uid=0 euid=0 tty=ssh ruser= rhost=192.168.1.11 user=zap").as_deref(),
            Some("192.168.1.11"),
        );
        assert_eq!(
            parse_failure("PAM 2 more authentication failures; logname= uid=0 euid=0 tty=ssh ruser= rhost=10.0.0.9 user=zap").as_deref(),
            Some("10.0.0.9"),
        );
        assert_eq!(
            parse_failure("Timeout before authentication for connection from 8.8.4.4 to 8.8.4.4 port 4310").as_deref(),
            Some("8.8.4.4"),
        );
        assert_eq!(
            parse_failure("Failed keyboard-interactive/pam for zap from 2001:db8::9 port 22 ssh2").as_deref(),
            Some("2001:db8::9"),
        );
    }

    #[test]
    fn regex_pattern_with_named_ip_group() {
        let re = Regex::new(r#"login failed from (?<ip>\S+) user"#).unwrap();
        assert_eq!(
            apply_line(Some(&re), "auth: login failed from 5.6.7.8 user bob").as_deref(),
            Some("5.6.7.8"),
        );
        // 命名组是权威：组内不是 IP 就视为不匹配（不回退，避免误封无关 IP）
        assert_eq!(
            apply_line(Some(&re), "auth: login failed from host user bob 9.9.9.9").as_deref(),
            None,
        );
        // 不匹配则无 IP
        assert!(apply_line(Some(&re), "login ok").is_none());
    }

    #[test]
    fn regex_without_group_falls_back_to_first_ip() {
        let re = Regex::new("failed").unwrap();
        assert_eq!(
            apply_line(Some(&re), "failed login 4.4.4.4 done").as_deref(),
            Some("4.4.4.4"),
        );
        assert!(apply_line(Some(&re), "failed login no-ip").is_none());
    }

    #[test]
    fn empty_pattern_uses_builtin_sshd_parser() {
        assert_eq!(
            apply_line(None, "Failed password for zap from 192.168.11.134 port 60602 ssh2").as_deref(),
            Some("192.168.11.134"),
        );
    }

    #[test]
    fn source_strings_resolve() {
        assert_eq!(resolve_source(""), Src::Auto);
        assert_eq!(resolve_source("auto"), Src::Auto);
        assert_eq!(
            resolve_source("journal:ssh,sshd"),
            Src::Journal(vec!["ssh".into(), "sshd".into()])
        );
        assert_eq!(resolve_source("journal"), Src::Journal(vec!["ssh".into(), "sshd".into()]));
        assert_eq!(
            resolve_source("file:/var/log/nginx/error.log"),
            Src::File("/var/log/nginx/error.log".into())
        );
        assert_eq!(
            resolve_source("/var/log/my.log"),
            Src::File("/var/log/my.log".into())
        );
    }

    #[test]
    fn hit_keys_roundtrip() {
        let k = hit_key("sshd", "1.2.3.4");
        assert_eq!(split_key(&k), ("sshd", "1.2.3.4"));
        assert_eq!(split_key("1.2.3.4"), ("", "1.2.3.4"));
    }

    #[test]
    fn zapd_reports_are_queued_and_drained() {
        report_failure("zapd", "3.3.3.3", "bob");
        report_failure("zapd", "3.3.3.3", "bob");
        let drained = drain_reports();
        assert_eq!(drained.len(), 2);
        assert!(drain_reports().is_empty());
    }

    #[test]
    fn default_settings_have_sshd_and_zapd_rules() {
        let s = BanSettings::default();
        assert_eq!(s.rules.len(), 2);
        assert_eq!(s.rules[0].id, "sshd");
        assert_eq!(s.rules[0].source, "auto");
        assert_eq!(s.rules[1].id, "zapd");
        assert_eq!(s.rules[1].source, "zapd");
        // 默认不忽略内网：LAN 测试 IP 也会被封
        assert!(!s.ignore_private);
    }

    #[test]
    fn skip_reason_explains_why_not_banned() {
        // 默认 ignore_private=false：内网也会封，只有白名单 / 环回才跳过
        let mut s = BanSettings::default();
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            // 默认不开忽略内网：内网 → 不跳过
            assert!(skip_reason("192.168.1.11", &s, &[]).await.is_none());
            // 公网 → 不跳过
            assert!(skip_reason("1.2.3.4", &s, &[]).await.is_none());
            // 环回 → 跳过
            assert!(skip_reason("127.0.0.1", &s, &[]).await.is_some());
            // 白名单 CIDR 命中
            let r = skip_reason("1.2.3.4", &s, &["1.2.3.0/24".to_string()]).await;
            assert!(r.is_some());
            assert!(r.unwrap().contains("1.2.3.0/24"));
            // 打开忽略内网后，内网才被跳过
            s.ignore_private = true;
            assert!(skip_reason("192.168.1.11", &s, &[]).await.is_some());
        });
    }

    #[test]
    fn private_ranges_are_ignored_by_default() {
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("::1"));
        assert!(is_private_ip("192.168.1.10"));
        assert!(is_private_ip("10.0.0.1"));
        assert!(is_private_ip("172.16.5.5"));
        assert!(!is_private_ip("1.2.3.4"));
        assert!(!is_private_ip("2001:db8::1"));
    }

    #[test]
    fn cidr_match_supports_both_families() {
        assert!(ip_in_cidr("1.2.3.4", "1.2.3.0/24"));
        assert!(!ip_in_cidr("1.2.4.4", "1.2.3.0/24"));
        assert!(ip_in_cidr("1.2.3.4", "1.2.3.4"));
        assert!(ip_in_cidr("2001:db8::5", "2001:db8::/32"));
        assert!(!ip_in_cidr("2001:db9::5", "2001:db8::/32"));
        // 族不匹配不得误判
        assert!(!ip_in_cidr("1.2.3.4", "::/0"));
    }
}
