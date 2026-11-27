// SPDX-License-Identifier: AGPL-3.0-only
//! 多节点：受管节点的凭据、只读快照采集与指标保留。
//!
//! 采集只打节点的 `/api/system/info` —— 该接口在 `access.rs` 里是
//! `Required::User`，所以**一个普通用户的 API Token 就够**，不需要在节点上
//! 开 admin、也不需要节点暴露 `/system/status`（那个是 Admin）。
//!
//! 历史曲线由**主控**每 15s 采样写进自己的 `node_stats` 表：
//! - 大屏一次 SQL 就能出全部节点，不会被某个慢节点卡住整屏；
//! - `created_at` 用主控时间，避免各节点时钟漂移把曲线拉歪；
//! - 节点掉线时曲线自然断点，而不是拿到一堆错误值。

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use once_cell::sync::Lazy;
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::warn;

use crate::db::get_db_pool;
use crate::zap::crypto;

/// 采集间隔（秒）。节点自身每 10s 采样一次，15s 拉一次刚好错开重复值。
pub const COLLECT_INTERVAL_SECS: u64 = 15;

/// 节点指标保留天数。比本机监控的 30 天短：N 台 × 每 15s 一行，
/// 20 台一个月就是 350 万行，压到 14 天再配小时聚合才稳。
const NODE_RETENTION_DAYS: i64 = 14;

/// 单节点请求超时：一台挂掉不能拖慢整轮。
const FETCH_TIMEOUT_SECS: u64 = 5;

/// 连续失败后的最大退避（秒），避免节点重启期间被反复打死。
const MAX_BACKOFF_SECS: u64 = 300;

/// 节点静态信息（版本）多久重新取一次。
const VERSION_TTL_SECS: u64 = 6 * 3600;

/// node_id -> (连续失败次数, 下次允许采集的 unix 秒)
static BACKOFF: Lazy<RwLock<HashMap<i64, (u32, i64)>>> = Lazy::new(|| RwLock::new(HashMap::new()));
/// node_id -> 上次取版本的 unix 秒
static VERSION_AT: Lazy<RwLock<HashMap<i64, i64>>> = Lazy::new(|| RwLock::new(HashMap::new()));

/// 复用连接池的 HTTP 客户端：严格校验 / 容忍自签两套。
fn http(insecure: bool) -> &'static reqwest::Client {
    static STRICT: OnceLock<reqwest::Client> = OnceLock::new();
    static INSECURE: OnceLock<reqwest::Client> = OnceLock::new();
    let cell = if insecure { &INSECURE } else { &STRICT };
    cell.get_or_init(|| {
        let b = reqwest::Client::builder()
            .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
            .danger_accept_invalid_certs(insecure);
        b.build().unwrap_or_else(|e| {
            warn!("构建节点 HTTP 客户端失败，回退默认配置: {}", e);
            reqwest::Client::new()
        })
    })
}

/// 归一化节点地址：补 scheme、去尾斜杠、去掉误填的 `/api`。
///
/// 存的是**面板根地址**（如 `https://10.0.0.5:8443` 或 `https://x/zap`），
/// 拼路径时再补 `/api/...`，这样将来加代理路由不用改库里的值。
pub fn normalize_base(raw: &str) -> Result<String, String> {
    let mut s = raw.trim().trim_end_matches('/').to_string();
    if s.is_empty() {
        return Err("节点地址不能为空".to_string());
    }
    if !s.starts_with("http://") && !s.starts_with("https://") {
        s = format!("https://{}", s);
    }
    if s.ends_with("/api") {
        s = s[..s.len() - 4].to_string();
    }
    // 先校验 host，否则 reqwest 的报错对普通用户太难懂
    let rest = s.split_once("://").map(|x| x.1).unwrap_or("");
    let host = rest
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    if host.is_empty() || !(host.contains('.') || host == "localhost") {
        return Err("节点地址格式不正确，示例：https://10.0.0.5:8443".to_string());
    }
    Ok(s)
}

/// 一次采集得到的节点快照（已折算成百分比，与本机 `system_stats` 同量纲）。
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub os_name: String,
    pub host_name: String,
    pub public_ip: String,
    pub uptime: String,
    pub cpu_num: i64,
    pub version: String,
    pub cpu_usage: f64,
    pub memory_usage: f64,
    pub memory_total_b: i64,
    pub swap_usage: f64,
    pub disk_usage: f64,
    pub disk_total_b: i64,
    pub disk_json: String,
    pub loadavg_one: f64,
    pub loadavg_five: f64,
    pub loadavg_fifteen: f64,
}

/// 只统计真实磁盘：tmpfs / overlay / cgroup 之类算进使用率会让水位虚高。
fn is_real_disk(fs: &str, mount: &str) -> bool {
    const SKIP_FS: &[&str] = &[
        "tmpfs",
        "devtmpfs",
        "squashfs",
        "overlay",
        "proc",
        "sysfs",
        "devpts",
        "cgroup",
        "cgroup2",
        "pstore",
        "bpf",
        "tracefs",
        "securityfs",
        "autofs",
        "mqueue",
        "hugetlbfs",
        "debugfs",
        "fusectl",
        "configfs",
        "binfmt_misc",
        "nsfs",
        "rpc_pipefs",
    ];
    const SKIP_MOUNT: &[&str] = &["/proc", "/sys", "/dev", "/run", "/var/lib/docker"];
    let fs_low = fs.to_ascii_lowercase();
    if SKIP_FS.iter().any(|k| fs_low == *k || fs_low.contains(k)) {
        return false;
    }
    !SKIP_MOUNT.iter().any(|p| mount.starts_with(p))
}

fn f64_of(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

fn i64_of(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(|x| x.as_i64()).unwrap_or(0)
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// GET 节点 `{base}/api{path}`，返回 `data` 字段。
async fn api_get(base: &str, token: &str, insecure: bool, path: &str) -> Result<Value, String> {
    let url = format!("{}{}{}", base, "/api", path);
    let resp = http(insecure)
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", short_reqwest_err(&e)))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(format!(
            "凭据被拒（HTTP {}）：请确认 Token 有效且未被停用",
            status.as_u16()
        ));
    }
    // 404 最常见的原因是节点开了访问前缀而地址没带前缀（前缀之外的路径一律 404）
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(
            "HTTP 404：接口不存在——若该节点启用了访问前缀，节点地址需带上前缀（如 https://10.0.0.5:2663/zap）"
                .to_string(),
        );
    }
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("响应不是合法 JSON: {}", e))?;
    if body.get("code").and_then(|c| c.as_i64()).unwrap_or(0) != 0 {
        return Err(str_of(&body, "message"));
    }
    Ok(body.get("data").cloned().unwrap_or(Value::Null))
}

/// reqwest 的错误串带 backtrace 式长句，这里只留最有用的一段。
fn short_reqwest_err(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return format!("超时（>{}s）", FETCH_TIMEOUT_SECS);
    }
    if e.is_connect() {
        return format!("无法连接: {}", e);
    }
    if e.is_decode() {
        return format!("解析响应失败: {}", e);
    }
    e.to_string()
}

/// 把节点 `/system/info` 的 `data` 折算成 `Snapshot`（纯函数，便于单测）。
pub fn parse_info(data: &Value) -> Snapshot {
    let total = i64_of(&data, "memory_total_b");
    let avail = i64_of(&data, "available_memory_b");
    let swap_total = i64_of(&data, "swap_total");
    let swap_used = i64_of(&data, "swap_used");

    let mut disk_total: i64 = 0;
    let mut disk_avail: i64 = 0;
    let mut disks: Vec<Value> = Vec::new();
    if let Some(list) = data.get("disk_info").and_then(|d| d.as_array()) {
        for d in list {
            let mount = str_of(d, "mount_point");
            let fs = str_of(d, "file_system");
            let t = i64_of(d, "total_space");
            let a = i64_of(d, "available_space");
            if !is_real_disk(&fs, &mount) || t <= 0 {
                continue;
            }
            disk_total += t;
            disk_avail += a.min(t);
            disks.push(serde_json::json!({
                "name": str_of(d, "name"),
                "mount_point": mount,
                "file_system": fs,
                "total_space": t,
                "available_space": a,
            }));
        }
    }

    let mut snap = Snapshot {
        os_name: str_of(&data, "os_name_version"),
        host_name: str_of(&data, "host_name"),
        public_ip: str_of(&data, "public_ip"),
        uptime: str_of(&data, "uptime"),
        cpu_num: i64_of(&data, "cpu_num"),
        memory_total_b: total,
        cpu_usage: f64_of(&data, "cpu_usage"),
        loadavg_one: f64_of(&data, "loadavg_one"),
        loadavg_five: f64_of(&data, "loadavg_five"),
        loadavg_fifteen: f64_of(&data, "loadavg_fifteen"),
        disk_total_b: disk_total,
        disk_json: serde_json::to_string(&disks).unwrap_or_else(|_| "[]".to_string()),
        ..Default::default()
    };
    snap.memory_usage = if total > 0 {
        ((total - avail).max(0) as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    snap.swap_usage = if swap_total > 0 {
        (swap_used as f64 / swap_total as f64) * 100.0
    } else {
        0.0
    };
    snap.disk_usage = if disk_total > 0 {
        ((disk_total - disk_avail).max(0) as f64 / disk_total as f64) * 100.0
    } else {
        0.0
    };

    snap
}

/// 拉取节点快照。`need_version` 为 true 时额外打一次 `/system/about`。
pub async fn fetch_snapshot(
    base: &str,
    token: &str,
    insecure: bool,
    need_version: bool,
) -> Result<Snapshot, String> {
    let data = api_get(base, token, insecure, "/system/info").await?;
    let mut snap = parse_info(&data);
    if need_version {
        if let Ok(about) = api_get(base, token, insecure, "/system/about").await {
            snap.version = str_of(&about, "version");
        }
    }
    Ok(snap)
}

/// 连通性测试（添加 / 编辑节点时用），失败也返回可读原因。
pub async fn test_connection(base: &str, token: &str, insecure: bool) -> Result<Snapshot, String> {
    let base = normalize_base(base)?;
    if token.trim().is_empty() {
        return Err("API Token 不能为空".to_string());
    }
    fetch_snapshot(&base, token, insecure, true).await
}

// ── 节点视图反向代理（/api/nodes/{id}/**）───────────────────────

/// 代理转发超时（秒）：比采集宽松，监控曲线接口偶尔慢一轮不值得整屏报错。
const PROXY_TIMEOUT_SECS: u64 = 10;

/// 代理白名单：只放行**只读、且与节点监控 / 基础信息相关**的接口路径。
///
/// 有意排除的：
/// - 写操作与交互式接口（终端 / 文件 / 站点管理…）——凭据是节点侧的 API Token，
///   放开等于把节点的可写面经主控转发，必须等逐接口的交互代理（含 WS）设计好再开；
/// - `node/*`——防止 A→B→A 的代理递归；
/// - `auth/*`、`user/*`——会话语义属于本机，代理过去只会拿节点 Token 的身份混淆视听。
///
/// 远端仍会按**它自己的**权限矩阵校验转发的 Token，这里只是主控侧的第一道闸。
pub const PROXY_ALLOW: &[&str] = &["system/info", "system/status", "system/about"];

/// 白名单匹配：按路径段边界，`system/info` 放行 `system/info` 与 `system/info/x`，
/// 但不放行 `system/infoo`。
pub fn proxy_allowed(rest: &str) -> bool {
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return false;
    }
    PROXY_ALLOW
        .iter()
        .any(|p| rest == *p || rest.starts_with(&format!("{p}/")))
}

/// 代理目标：节点地址 + 解密后的 API Token + TLS 容忍开关。
pub struct ProxyTarget {
    pub base: String,
    pub token: String,
    pub insecure: bool,
}

/// 按 id 解析代理目标；节点不存在 / 已停用都拒绝（不落任何转发）。
pub async fn resolve_target(id: i64) -> Result<ProxyTarget, String> {
    let pool = get_db_pool().await;
    let row: Option<(String, String, i64, i64)> =
        sqlx::query_as("SELECT base_url, api_token, insecure_tls, status FROM nodes WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("查询节点失败: {e}"))?;
    let Some((base, enc, insecure, status)) = row else {
        return Err("节点不存在".to_string());
    };
    if status != 1 {
        return Err("节点已停用".to_string());
    }
    let token = crypto::decrypt_password(&enc);
    if token.is_empty() {
        return Err("节点凭据不可用（解密失败），请重新保存节点".to_string());
    }
    Ok(ProxyTarget {
        base,
        token,
        insecure: insecure != 0,
    })
}

/// 把只读请求转发到 `{base}/api/{rest}`，原样带回（状态码 / Content-Type / body）。
///
/// 不流式：白名单接口都是小 JSON，整体缓冲换实现简单；将来放开大响应接口再改流式。
pub async fn forward(
    target: &ProxyTarget,
    rest: &str,
    query: Option<&str>,
) -> Result<(u16, String, Vec<u8>), String> {
    let mut url = format!("{}/api/{}", target.base, rest.trim_start_matches('/'));
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        url = format!("{url}?{q}");
    }
    let resp = http(target.insecure)
        .get(&url)
        .timeout(Duration::from_secs(PROXY_TIMEOUT_SECS))
        .bearer_auth(&target.token)
        .send()
        .await
        .map_err(|e| format!("{}", short_reqwest_err(&e)))?;
    let status = resp.status().as_u16();
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let body = resp
        .bytes()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?
        .to_vec();
    Ok((status, ctype, body))
}

struct NodeRow {
    id: i64,
    base_url: String,
    api_token: String,
    insecure_tls: i64,
    version: String,
}

/// 一轮采集：并发拉取所有启用节点，各自超时互不影响。
///
/// 由 `job.rs` 的 cron 每 15s 触发。整轮不返回错误——单个节点失败只影响它自己
/// 的 `online` 与退避窗口，否则一台掉线会让任务在调度器里一直报错。
pub async fn collect_round() {
    let pool = get_db_pool().await;
    let rows: Vec<(i64, String, String, i64, String)> = sqlx::query_as(
        "SELECT id, base_url, api_token, insecure_tls, version FROM nodes WHERE status = 1",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    if rows.is_empty() {
        return;
    }
    let now = chrono::Local::now().timestamp();

    // 退避窗口内的节点本轮跳过（掉线节点不做无谓重试）
    let mut targets: Vec<NodeRow> = Vec::new();
    {
        let back = BACKOFF.read().await;
        for (id, base_url, api_token, insecure_tls, version) in rows {
            if back.get(&id).is_some_and(|(_, next)| *next > now) {
                continue;
            }
            targets.push(NodeRow {
                id,
                base_url,
                api_token,
                insecure_tls,
                version,
            });
        }
    }
    if targets.is_empty() {
        return;
    }

    // 版本超过 TTL 才重新取，正常轮次每节点只 1 个请求
    let mut handles = Vec::with_capacity(targets.len());
    for t in targets {
        let need_version = t.version.is_empty()
            || !VERSION_AT
                .read()
                .await
                .get(&t.id)
                .is_some_and(|ts| now - ts < VERSION_TTL_SECS as i64);
        handles.push(tokio::spawn(async move {
            let token = crypto::decrypt_password(&t.api_token);
            let r = fetch_snapshot(&t.base_url, &token, t.insecure_tls != 0, need_version).await;
            (t, need_version, r)
        }));
    }

    let pool = get_db_pool().await;
    for h in handles {
        let Ok((node, need_version, result)) = h.await else {
            continue;
        };
        match result {
            Ok(s) => {
                let _ = sqlx::query(
                    "INSERT INTO node_stats \
                     (node_id,cpu_usage,memory_usage,swap_usage,disk_usage,\
                      loadavg_one,loadavg_five,loadavg_fifteen,created_at) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                )
                .bind(node.id)
                .bind(s.cpu_usage)
                .bind(s.memory_usage)
                .bind(s.swap_usage)
                .bind(s.disk_usage)
                .bind(s.loadavg_one)
                .bind(s.loadavg_five)
                .bind(s.loadavg_fifteen)
                .bind(now)
                .execute(pool)
                .await;

                let upd = sqlx::query(
                    "UPDATE nodes SET online=1, last_seen=$1, last_error='', \
                     os_name=$2, host_name=$3, public_ip=$4, uptime=$5, cpu_num=$6, \
                     memory_total_b=$7, disk_total_b=$8, disk_json=$9, updated_at=$1 \
                     WHERE id=$10",
                )
                .bind(now)
                .bind(&s.os_name)
                .bind(&s.host_name)
                .bind(&s.public_ip)
                .bind(&s.uptime)
                .bind(s.cpu_num)
                .bind(s.memory_total_b)
                .bind(s.disk_total_b)
                .bind(&s.disk_json)
                .bind(node.id);
                if need_version && !s.version.is_empty() {
                    let _ = sqlx::query("UPDATE nodes SET version=$1 WHERE id=$2")
                        .bind(&s.version)
                        .bind(node.id)
                        .execute(pool)
                        .await;
                }
                let _ = upd.execute(pool).await;

                if need_version {
                    VERSION_AT.write().await.insert(node.id, now);
                }
                BACKOFF.write().await.remove(&node.id);
            }
            Err(e) => {
                let _ = sqlx::query("UPDATE nodes SET online=0, last_error=$1 WHERE id=$2")
                    .bind(&e)
                    .bind(node.id)
                    .execute(pool)
                    .await;
                let mut back = BACKOFF.write().await;
                let fails = back.get(&node.id).map(|(f, _)| f + 1).unwrap_or(1);
                let delay = (2u64.pow(fails.min(5)).saturating_mul(COLLECT_INTERVAL_SECS))
                    .min(MAX_BACKOFF_SECS);
                back.insert(node.id, (fails, now + delay as i64));
            }
        }
    }
}

/// 清理超期节点指标。
pub async fn cleanup_node_stats() {
    let pool = get_db_pool().await;
    let cutoff = chrono::Local::now().timestamp() - NODE_RETENTION_DAYS * 24 * 3600;
    match sqlx::query("DELETE FROM node_stats WHERE created_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await
    {
        Ok(a) if a.rows_affected() > 0 => {
            tracing::info!("节点监控数据清理: {} 行", a.rows_affected());
        }
        Err(_) => warn!("节点监控数据清理任务执行异常"),
        _ => {}
    }
}

/// 节点增删改后调用：丢掉退避/版本缓存，让下一轮立刻重采。
pub async fn reset_node_state(node_id: i64) {
    BACKOFF.write().await.remove(&node_id);
    VERSION_AT.write().await.remove(&node_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_base_fills_scheme_and_strips_noise() {
        assert_eq!(
            normalize_base("10.0.0.5:8443").unwrap(),
            "https://10.0.0.5:8443"
        );
        assert_eq!(normalize_base("http://a.b/").unwrap(), "http://a.b");
        // 用户常把整条 API 路径粘进来
        assert_eq!(
            normalize_base("https://x.test/api").unwrap(),
            "https://x.test"
        );
        // 子路径部署要保留
        assert_eq!(
            normalize_base("https://x.test/zap").unwrap(),
            "https://x.test/zap"
        );
        assert_eq!(
            normalize_base("localhost:8443").unwrap(),
            "https://localhost:8443"
        );
    }

    #[test]
    fn normalize_base_rejects_unusable_input() {
        assert!(normalize_base("").is_err());
        assert!(normalize_base("   ").is_err());
        assert!(normalize_base("https://").is_err());
        // 没有点也不是 localhost 的 host 基本是填错了
        assert!(normalize_base("node-a1").is_err());
    }

    #[test]
    fn proxy_allow_matches_segment_boundaries() {
        // 白名单内：精确 + 子路径
        assert!(proxy_allowed("system/info"));
        assert!(proxy_allowed("/system/status"));
        assert!(proxy_allowed("system/about/detail"));
        // 前缀相似但不同段：拒绝
        assert!(!proxy_allowed("system/infoo"));
        assert!(!proxy_allowed("system/status-x"));
        // 白名单外 / 空 / 递归面：全部拒绝
        assert!(!proxy_allowed(""));
        assert!(!proxy_allowed("site/list"));
        assert!(!proxy_allowed("auth/current"));
        assert!(!proxy_allowed("node/overview"));
        assert!(!proxy_allowed("nodes/1/system/info"));
    }

    #[test]
    fn real_disk_filter_skips_pseudo_filesystems() {
        assert!(is_real_disk("ext4", "/"));
        assert!(is_real_disk("xfs", "/data"));
        assert!(!is_real_disk("tmpfs", "/run"));
        assert!(!is_real_disk("overlay", "/var/lib/docker/overlay2/x"));
        assert!(!is_real_disk("btrfs", "/proc/fs"));
        assert!(!is_real_disk("zfs", "/sys"));
    }

    #[test]
    fn parse_info_converts_to_percentages() {
        let snap = parse_info(&json!({
            "os_name_version": "Ubuntu 22.04.3 LTS",
            "host_name": "node-a1",
            "cpu_num": 8,
            "cpu_usage": 42.5,
            "memory_total_b": 1000,
            "available_memory_b": 250,
            "swap_total": 100,
            "swap_used": 25,
            "loadavg_one": 1.5,
            "loadavg_five": 1.0,
            "loadavg_fifteen": 0.5,
            "disk_info": [
                { "name": "sda", "mount_point": "/", "file_system": "ext4",
                  "total_space": 1000, "available_space": 400 },
                // 伪文件系统不得计入容量水位
                { "name": "tmp", "mount_point": "/tmp", "file_system": "tmpfs",
                  "total_space": 500, "available_space": 500 }
            ]
        }));
        assert_eq!(snap.os_name, "Ubuntu 22.04.3 LTS");
        assert_eq!(snap.cpu_usage, 42.5);
        assert!((snap.memory_usage - 75.0).abs() < 1e-9);
        assert!((snap.swap_usage - 25.0).abs() < 1e-9);
        assert_eq!(snap.disk_total_b, 1000);
        assert!((snap.disk_usage - 60.0).abs() < 1e-9);
        assert_eq!(snap.loadavg_one, 1.5);
        // 只保留真实磁盘（断言内容而非序列化顺序）
        let kept: Vec<Value> = serde_json::from_str(&snap.disk_json).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["mount_point"], "/");
    }

    /// 节点上 sysinfo 刚初始化时可能全是 0，绝不能出现除零 NaN
    #[test]
    fn parse_info_survives_empty_payload() {
        let snap = parse_info(&json!({}));
        assert_eq!(snap.memory_usage, 0.0);
        assert_eq!(snap.swap_usage, 0.0);
        assert_eq!(snap.disk_usage, 0.0);
        assert!(snap.disk_usage.is_finite());
        assert_eq!(snap.disk_json, "[]");
    }

    /// `available_space` 大于 `total_space`（部分文件系统会这样）不能算出负使用率
    #[test]
    fn parse_info_clamps_odd_disk_numbers() {
        let snap = parse_info(&json!({
            "disk_info": [
                { "mount_point": "/", "file_system": "ext4",
                  "total_space": 100, "available_space": 180 }
            ]
        }));
        assert_eq!(snap.disk_total_b, 100);
        assert_eq!(snap.disk_usage, 0.0);
    }
}
