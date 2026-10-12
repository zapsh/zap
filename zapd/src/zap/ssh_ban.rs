// SPDX-License-Identifier: AGPL-3.0-only
//! SSH 爆破防护（面板内置 fail2ban）：
//!
//! 周期性解析 sshd 认证失败日志 → 窗口内达到阈值即封禁 → 到期自动解封。
//! 规则本身落在防火墙后端（复用 `firewall.blacklist_*` 动词），这里只负责
//! 「什么时候封、封多久、封过谁」，状态存 `global_settings` KV。
//!
//! 关键取舍：
//! - 白名单（防火墙白名单 + 设置里的忽略 IP）永不被封；
//! - 默认忽略内网 / 环回，避免误伤运维跳板机（可在设置里关掉）；
//! - 封禁动作走 zapexec，解封时按 IP 反查规则 id，兼容各后端不同的 id 形态。

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::db::get_db_pool;
use crate::zapexec;
use zap_proto::Request;

const KEY_SETTINGS: &str = "firewall.ban_settings";
const KEY_BANS: &str = "firewall.ban_records";
const KEY_STATE: &str = "firewall.ban_state";
const KEY_WHITELIST: &str = "firewall.whitelist";

/// 计数窗口内单 IP 的失败次数上限之外的字段默认值。
fn def_max_retry() -> u32 {
    5
}
fn def_find_time() -> u32 {
    600
}
fn def_ban_time() -> u32 {
    3600
}
fn def_interval() -> u32 {
    60
}
fn def_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanSettings {
    #[serde(default)]
    pub enabled: bool,
    /// 窗口内失败几次即封禁
    #[serde(default = "def_max_retry")]
    pub max_retry: u32,
    /// 计数窗口（秒）
    #[serde(default = "def_find_time")]
    pub find_time: u32,
    /// 封禁时长（秒），到期自动解封
    #[serde(default = "def_ban_time")]
    pub ban_time: u32,
    /// 扫描间隔（秒）
    #[serde(default = "def_interval")]
    pub interval: u32,
    /// 忽略内网 / 环回地址（默认开，防止误伤跳板机）
    #[serde(default = "def_true")]
    pub ignore_private: bool,
    /// 额外忽略的 IP / CIDR（逗号分隔写在 JSON 数组里）
    #[serde(default)]
    pub ignored_ips: Vec<String>,
    /// 指定日志文件；留空 = 自动探测（auth.log / secure）
    #[serde(default)]
    pub log_file: String,
}

impl Default for BanSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            max_retry: def_max_retry(),
            find_time: def_find_time(),
            ban_time: def_ban_time(),
            interval: def_interval(),
            ignore_private: def_true(),
            ignored_ips: Vec::new(),
            log_file: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanRecord {
    pub ip: String,
    pub reason: String,
    /// 触发时的失败次数
    pub hits: u32,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BanState {
    #[serde(default)]
    pub log_file: String,
    /// 已读到的日志字节偏移（增量扫描，避免每轮重读全文件）
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub last_run: u64,
    #[serde(default)]
    pub total_banned: u64,
    #[serde(default)]
    pub last_error: String,
    /// 窗口内各 IP 的失败计数 `ip -> (次数, 最近一次时间)`，随 find_time 滑动
    #[serde(default)]
    pub hits: std::collections::HashMap<String, (u32, u64)>,
    /// journal 模式：上次读到的游标（`journalctl --after-cursor` 增量读）
    #[serde(default)]
    pub journal_cursor: String,
    /// 命中失败但被策略跳过（内网 / 白名单）的来源 `ip -> (次数, 原因, 最近一次)`，
    /// 用于面板解释「为什么这个 IP 没被封」——内网默认忽略最容易踩这个坑。
    #[serde(default)]
    pub ignored: std::collections::HashMap<String, (u32, String, u64)>,
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
    // 参数收敛：防止把扫描间隔设成 0 导致空转烧 CPU
    let mut s = s.clone();
    s.max_retry = s.max_retry.clamp(1, 100);
    s.find_time = s.find_time.clamp(30, 86_400);
    s.ban_time = s.ban_time.clamp(60, 7 * 86_400);
    s.interval = s.interval.clamp(10, 3_600);
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

// ── 日志解析 ───────────────────────────────────────────────────

fn journal_available() -> bool {
    std::process::Command::new("journalctl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// journal 模式的展示名（面板上显示「日志来源」用）。
const JOURNAL_LABEL: &str = "journalctl -u ssh";

/// 读 journal 里 sshd 的新条目，返回 `(消息行, 新游标)`。
///
/// 用游标增量读，等价于文件模式的字节偏移；首次（无游标）只取最近若干条，
/// 避免开机以来全部历史被重放导致误封。
fn read_journal(cursor: &str, limit: usize) -> Result<(Vec<String>, String), String> {
    let mut cmd = std::process::Command::new("journalctl");
    cmd.args([
        "-u",
        "ssh",
        "-u",
        "sshd",
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
            return read_journal("", limit);
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
        // 多行消息 journal 会拆成数组，这里拼回来，保证 parse_failure 看到完整一行
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

/// 显式要求走 journal（Debian 默认不装 rsyslog，auth.log 不存在）。
fn wants_journal(configured: &str) -> bool {
    matches!(
        configured.trim().to_ascii_lowercase().as_str(),
        "journal" | "journalctl" | "systemd-journal"
    )
}

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

/// 内网 / 环回 / 链路本地：默认不封（运维跳板机与容器网段常见）。
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

/// 启动爆破防护循环（后台任务）。默认关闭，用户在面板开启后生效。
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

async fn tick(s: &BanSettings) {
    let mut st = state().await;
    let ts = now();
    st.last_run = ts;

    // 日志来源：优先文件（RHEL / 装了 rsyslog 的 Debian），
    // 没有 auth.log/secure 时（Debian 默认形态）转 journalctl。
    let journal_mode = wants_journal(&s.log_file)
        || (s.log_file.trim().is_empty() && detect_log_file("").is_none() && journal_available());
    if !journal_mode && detect_log_file(&s.log_file).is_none() {
        st.last_error = if journal_available() {
            "未找到 sshd 日志文件，且未指定 journal".to_string()
        } else {
            "未找到 sshd 日志（auth.log / secure），系统也没有 journalctl".to_string()
        };
        kv_set(KEY_STATE, &serde_json::to_string(&st).unwrap_or_default()).await;
        return;
    }

    let find_time = s.find_time.max(30) as u64;
    // 窗口滑动：淘汰超过 find_time 的旧计数，只保留仍在窗口内的
    st.hits.retain(|_, (_, at)| ts.saturating_sub(*at) <= find_time);

    // 取这一轮的失败行（文件按字节偏移增量读；journal 按游标增量读）
    let lines: Vec<String> = if journal_mode {
        st.log_file = JOURNAL_LABEL.to_string();
        match read_journal(&st.journal_cursor, 500) {
            Ok((ls, cursor)) => {
                st.journal_cursor = cursor;
                ls
            }
            Err(e) => {
                st.last_error = e;
                kv_set(KEY_STATE, &serde_json::to_string(&st).unwrap_or_default()).await;
                return;
            }
        }
    } else {
        let path = detect_log_file(&s.log_file).unwrap_or_default();
        // 日志轮转 / 文件被清空：偏移比文件还大就从头读
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if st.log_file != path || st.offset > len {
            st.log_file = path.clone();
            st.offset = 0;
        }
        let content = match std::fs::File::open(&path).and_then(|mut f| {
            use std::io::{Read, Seek, SeekFrom};
            f.seek(SeekFrom::Start(st.offset))?;
            let mut buf = String::new();
            f.read_to_string(&mut buf)?;
            Ok(buf)
        }) {
            Ok(c) => c,
            Err(e) => {
                st.last_error = format!("读取 {path} 失败: {e}");
                kv_set(KEY_STATE, &serde_json::to_string(&st).unwrap_or_default()).await;
                return;
            }
        };
        // 只按完整行推进偏移，避免半行被截断后下一轮解析出错
        let consumed = match content.rfind('\n') {
            Some(i) => i + 1,
            None => 0,
        };
        st.offset += consumed as u64;
        content[..consumed].lines().map(|l| l.to_string()).collect()
    };

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
            warn!("爆破防护解封 {ip} 失败: {e}");
            st.last_error = format!("解封 {ip} 失败: {e}");
        } else {
            info!("爆破防护到期解封: {ip}");
        }
    }

    // 累加窗口计数
    for line in &lines {
        if let Some(ip) = parse_failure(line) {
            let e = st.hits.entry(ip).or_insert((0, ts));
            e.0 += 1;
            e.1 = ts;
        }
    }
    // 被策略跳过的来源单独记账，面板据此解释「为什么没封它」
    st.ignored
        .retain(|_, (_, _, at)| ts.saturating_sub(*at) <= find_time);
    for (ip, (count, _)) in st.hits.clone() {
        if let Some(reason) = skip_reason(&ip, s, &wl).await {
            let e = st.ignored.entry(ip.clone()).or_insert((0, reason, ts));
            e.0 = e.0.max(count);
            e.2 = ts;
            // 已跳过的 IP 不参与封禁计数
            st.hits.remove(&ip);
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
    for (ip, (count, _)) in st.hits.clone() {
        if count < s.max_retry {
            debug!("爆破防护：{ip} 失败 {count} 次，未达阈值 {}", s.max_retry);
            continue;
        }
        if already.contains(&ip) || skip_reason(&ip, s, &wl).await.is_some() {
            continue;
        }
        match ban_firewall(&ip).await {
            Ok(()) => {
                list.push(BanRecord {
                    ip: ip.clone(),
                    reason: format!("ssh 认证失败 {count} 次 / {}s", s.find_time),
                    hits: count,
                    created_at: ts,
                    expires_at: ts + s.ban_time as u64,
                });
                st.total_banned += 1;
                new_bans += 1;
                // 已封的 IP 不再重复计数
                st.hits.remove(&ip);
                info!("爆破防护封禁 {ip}（失败 {count} 次）");
            }
            Err(e) => {
                warn!("爆破防护封禁 {ip} 失败: {e}");
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
    fn skip_reason_explains_why_not_banned() {
        let s = BanSettings::default(); // ignore_private = true
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            // 内网 → 命中「忽略内网」
            assert!(skip_reason("192.168.1.11", &s, &[]).await.is_some());
            // 公网 → 不跳过
            assert!(skip_reason("1.2.3.4", &s, &[]).await.is_none());
            // 白名单 CIDR 命中
            let r = skip_reason("1.2.3.4", &s, &["1.2.3.0/24".to_string()]).await;
            assert!(r.is_some());
            assert!(r.unwrap().contains("1.2.3.0/24"));
            // 关掉忽略内网后，内网仍可封
            let mut s2 = s.clone();
            s2.ignore_private = false;
            assert!(skip_reason("192.168.1.11", &s2, &[]).await.is_none());
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
