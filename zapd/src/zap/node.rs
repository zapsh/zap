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

/// 代理等到响应头的超时（秒）：必须短于前端 axios 的 15s，否则浏览器先 abort，
/// 用户看到的是「网络错误」而不是节点给出的真实原因。
///
/// 只约束「响应头到达」：SSE 日志跟随、备份包下载、终端 WS 在拿到 200/101 之后
/// 按流转发，不受这个限制——写成一个整体 timeout 会在 12s 掐断它们。
const PROXY_TIMEOUT_SECS: u64 = 12;

/// 要改写凭据的 JSON body 上限：超过它就不解析（原样流式转发）。
/// 带 `token` 字段的 body 只有「插件取消任务」这类小 JSON，64 KB 绰绰有余；
/// 文件上传走 multipart，体积可能很大，必须流式、也不含会话凭据。
pub const PROXY_JSON_SNIFF_BYTES: u64 = 64 * 1024;

/// 主控自有面：这些前缀的接口**永远打主控**，即使在节点视图里。
///
/// - `auth`：登录 / 登出 / 刷新 / 验证码。会话属于主控，节点没有这个会话；
///   代理过去等于拿远端身份去刷新本机会话，会把登录态彻底搅乱。
/// - `node` / `nodes`：主控的节点台账与跨节点监控，代理过去就是 A→B→A 的递归代理。
/// - `dev`：API Token 控制台，管的始终是「谁在调用这个面板」。
/// - `health` / `docs`：主控进程自己的探针与内置文档（两台机器上是同一份）。
///
/// 刻意**不含** `user` 与 `system/menus`：接管的目标是「像直接登录那台机器一样」，
/// 身份（`/user/info` 的 roles / permissions）和菜单树必须来自节点，前端才会按
/// 远端 Token 归属账号的权限渲染菜单与按钮，而不是「主控 admin 看得见按钮、
/// 点下去节点报 403」。
const PROXY_DENY_ROOTS: &[&str] = &["auth", "node", "nodes", "dev", "health", "docs"];

/// 拒绝原因：只剩「这条请求不该由节点回答」，不再区分只读与写。
pub enum ProxyDeny {
    /// 路径非法（空 / 含 `.` `..`）
    BadPath,
    /// 属于主控自有面
    MasterOwned,
}

impl ProxyDeny {
    pub fn message(&self) -> &'static str {
        match self {
            ProxyDeny::BadPath => "代理路径不合法",
            ProxyDeny::MasterOwned => {
                "该接口属于主控自身（会话 / 节点台账 / Token 控制台），不经节点代理"
            }
        }
    }
}

/// 节点视图代理放行判定：**除主控自有面外全部转发，任意 HTTP 方法**。
pub fn proxy_deny_reason(rest: &str) -> Option<ProxyDeny> {
    let rest = rest.trim_start_matches('/');
    let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() || segments.iter().any(|s| *s == "." || *s == "..") {
        return Some(ProxyDeny::BadPath);
    }
    // 按段边界比对而不是字符串前缀，免得 `nodex`、`/devtools` 这类同前缀不同段的路径被误伤
    if PROXY_DENY_ROOTS
        .iter()
        .any(|root| segments.starts_with(&root.split('/').collect::<Vec<_>>()[..]))
    {
        return Some(ProxyDeny::MasterOwned);
    }
    None
}

/// 节点视图代理是否转发该路径。
pub fn proxy_allowed(rest: &str) -> bool {
    proxy_deny_reason(rest).is_none()
}

/// 代理目标：节点名 + 地址 + 解密后的 API Token + TLS 容忍开关。
///
/// `name` 只用于审计与错误文案（「无法连接到节点 web-01」），不参与请求构造。
pub struct ProxyTarget {
    pub name: String,
    pub base: String,
    pub token: String,
    pub insecure: bool,
}

/// 按 id 解析代理目标；节点不存在 / 已停用都拒绝（不落任何转发）。
pub async fn resolve_target(id: i64) -> Result<ProxyTarget, String> {
    let pool = get_db_pool().await;
    let row: Option<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT name, base_url, api_token, insecure_tls, status FROM nodes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询节点失败: {e}"))?;
    let Some((name, base, enc, insecure, status)) = row else {
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
        name,
        base,
        token,
        insecure: insecure != 0,
    })
}

/// 面板凭据参数名：WS 握手、EventSource、下载链接都靠 `?token=` 带凭据
/// （浏览器给这些请求加不了自定义请求头）。
const TOKEN_PARAM: &str = "token";

/// 拼节点侧完整 URL：`{base}/api/{rest}[?{query}]`。
fn node_url(target: &ProxyTarget, rest: &str, query: Option<&str>) -> String {
    let mut url = format!("{}/api/{}", target.base, rest.trim_start_matches('/'));
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        url = format!("{url}?{q}");
    }
    url
}

/// 把查询串里的凭据换成节点自己的 API Token。
///
/// 浏览器带上来的是**主控会话 JWT**。原样转出去有两个后果：节点不认它（必然 401），
/// 更要紧的是把主控凭据泄露给了受控端——那台机器拿着它就能反过来调主控。
///
/// 是「替换」而不是「删掉」：终端、docker exec、插件日志这些 handler 自己从
/// query 取凭据（走 `jwt::claims_from_token`，已支持 API Token），删掉等于把它们判死。
pub fn swap_query_token(query: Option<&str>, node_token: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for pair in query.unwrap_or("").split('&') {
        if pair.is_empty() {
            continue;
        }
        // 只认键名，不 URL 解码：token 是 JWT 或 `zap_` + 十六进制，无需转义字符
        if pair.split('=').next().unwrap_or("") != TOKEN_PARAM {
            kept.push(pair.to_string());
        }
    }
    kept.push(format!("{TOKEN_PARAM}={node_token}"));
    kept.join("&")
}

/// 小 JSON body 里的 `token` 字段同样置换（`/plugin/cancel` 属于这类）。
///
/// 解析失败、不是对象、字段不是字符串，一律原样返回——代理只换凭据，
/// 不该改写业务载荷，更不该因为 body 长得不像预期就把请求打断。
pub fn swap_body_token(body: &[u8], node_token: &str) -> Vec<u8> {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return body.to_vec();
    };
    let Some(mut obj) = value.as_object().cloned() else {
        return body.to_vec();
    };
    if obj.get(TOKEN_PARAM).and_then(Value::as_str).is_none() {
        return body.to_vec();
    }
    obj.insert(
        TOKEN_PARAM.to_string(),
        Value::String(node_token.to_string()),
    );
    serde_json::to_vec(&Value::Object(obj)).unwrap_or_else(|_| body.to_vec())
}

/// 把请求转发到 `{base}/api/{rest}`，返回**尚未读体**的响应，由调用方按流回给浏览器。
///
/// 两端都不缓冲：接管后既有几 KB 的 JSON，也有备份包下载、multipart 上传、
/// SSE 日志跟随。缓冲式实现要么把主控内存吃掉，要么把大文件截断——上一版
/// 那个 16 MB 的响应上限就是缓冲的代价，现在按流走，连上限都不需要了。
///
/// 凭据在这里换：query 走 `swap_query_token`，请求头用节点 Token 覆盖
/// （节点的 `access::token_from_request` 优先读 `Authorization`）。
pub async fn forward(
    target: &ProxyTarget,
    rest: &str,
    query: Option<&str>,
    method: reqwest::Method,
    body: Option<reqwest::Body>,
    content_type: Option<String>,
) -> Result<reqwest::Response, String> {
    let q = swap_query_token(query, &target.token);
    let url = node_url(target, rest, Some(&q));
    let mut req = http(target.insecure)
        .request(method, &url)
        .bearer_auth(&target.token);
    // Content-Type 必须带上：multipart 的 boundary、JSON 的 charset 都在里面，
    // 丢了远端按 form 解析 JSON 会直接 400。
    if let Some(ct) = content_type {
        req = req.header(reqwest::header::CONTENT_TYPE, ct);
    }
    if let Some(b) = body {
        req = req.body(b);
    }
    // 只给「响应头到达」限时：整体 timeout 会掐断 SSE 与大文件下载
    tokio::time::timeout(Duration::from_secs(PROXY_TIMEOUT_SECS), req.send())
        .await
        .map_err(|_| format!("节点 {PROXY_TIMEOUT_SECS}s 内未返回响应"))?
        .map_err(|e| short_reqwest_err(&e))
}

/// 已握手的到节点 WebSocket：裸流由 reqwest 提供，帧协议由 tungstenite 负责。
pub type NodeWs = tokio_tungstenite::WebSocketStream<reqwest::Upgraded>;

/// 以节点 Token 打开到 `{base}/api/{rest}` 的 WebSocket。
///
/// 握手刻意复用 reqwest 而不是 `tokio_tungstenite::client_async`：节点面板多是
/// 自签证书，`http(insecure)` 已经把 TLS 与 `insecure_tls` 走通；自己拨号还得再配
/// 一套 rustls 危险校验器。拿到 101 之后用 `from_raw_socket` 接管已升级的裸流。
pub async fn open_node_ws(
    target: &ProxyTarget,
    rest: &str,
    query: Option<&str>,
) -> Result<NodeWs, String> {
    use base64::Engine as _;
    let q = swap_query_token(query, &target.token);
    let url = node_url(target, rest, Some(&q));
    // Sec-WebSocket-Key 由客户端随机生成，节点据此回 Sec-WebSocket-Accept
    let mut raw = [0u8; 16];
    rand::Rng::fill(&mut rand::thread_rng(), &mut raw);
    let key = base64::engine::general_purpose::STANDARD.encode(raw);

    let resp = http(target.insecure)
        .get(&url)
        .header(reqwest::header::CONNECTION, "Upgrade")
        .header(reqwest::header::UPGRADE, "websocket")
        .header("Sec-WebSocket-Key", key)
        .header("Sec-WebSocket-Version", "13")
        .bearer_auth(&target.token)
        .send()
        .await
        .map_err(|e| short_reqwest_err(&e))?;
    if resp.status() != reqwest::StatusCode::SWITCHING_PROTOCOLS {
        return Err(format!("节点拒绝升级（HTTP {}）", resp.status().as_u16()));
    }
    let upgraded = resp
        .upgrade()
        .await
        .map_err(|e| format!("连接升级失败: {e}"))?;
    Ok(NodeWs::from_raw_socket(
        upgraded,
        tokio_tungstenite::tungstenite::protocol::Role::Client,
        None,
    )
    .await)
}

/// 浏览器 ↔ 节点双向泵帧：任一侧断开即结束，并关掉另一侧。
///
/// 消息按类型原样搬运（文本 / 二进制 / ping / pong / close），终端是二进制或文本
/// 帧、docker exec 走文本帧，都不需要在这里理解内容。
pub async fn pump_ws(mut browser: axum::extract::ws::WebSocket, mut node: NodeWs) {
    use axum::extract::ws::Message as BMsg;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as TMsg;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame as TClose;

    fn to_node(m: BMsg) -> TMsg {
        match m {
            BMsg::Text(t) => TMsg::Text(t.as_str().to_string().into()),
            BMsg::Binary(b) => TMsg::Binary(b),
            BMsg::Ping(p) => TMsg::Ping(p),
            BMsg::Pong(p) => TMsg::Pong(p),
            BMsg::Close(c) => TMsg::Close(c.map(|f| TClose {
                code: f.code.into(),
                reason: f.reason.as_str().to_string().into(),
            })),
        }
    }

    fn to_browser(m: TMsg) -> BMsg {
        match m {
            TMsg::Text(t) => BMsg::Text(t.as_str().to_string().into()),
            TMsg::Binary(b) => BMsg::Binary(b),
            TMsg::Ping(p) => BMsg::Ping(p),
            TMsg::Pong(p) => BMsg::Pong(p),
            TMsg::Close(c) => BMsg::Close(c.map(|f| axum::extract::ws::CloseFrame {
                code: f.code.into(),
                reason: f.reason.as_str().to_string().into(),
            })),
            // 分片帧（Frame）由 tungstenite 自己聚合，正常不会走到这里
            TMsg::Frame(_) => BMsg::Close(None),
        }
    }

    loop {
        tokio::select! {
            biased;
            from_browser = browser.recv() => match from_browser {
                Some(Ok(m)) => {
                    if node.send(to_node(m)).await.is_err() {
                        break;
                    }
                }
                _ => break,
            },
            from_node = node.next() => match from_node {
                Some(Ok(m)) => {
                    if browser.send(to_browser(m)).await.is_err() {
                        break;
                    }
                }
                _ => break,
            },
        }
    }
    let _ = node.close(None).await;
    let _ = browser.close().await;
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

    /// 接管的目标是「像直接登录那台机器」，所以策略反过来：**除主控自有面全放行**，
    /// 且不再区分读写。这里钉住各业务模块的代表性接口，确保它们都走代理，
    /// 而不是悄悄回到主控本机（那正是「切了节点还是在看本机」的根因）。
    #[test]
    fn proxy_policy_covers_business_surfaces() {
        for path in [
            "system/info",
            "/system/status",
            "system/overview",
            "system/config/processes",
            "system/nginx/status",
            "system/files/list",
            "site/list",
            "ssl/cert/list",
            "database/list",
            "docker/containers",
            // 身份与菜单必须来自节点，前端才按远端 Token 的权限渲染菜单与按钮
            "user/info",
            "system/menus",
            // 终端 / 实时日志 / 下载：接管后才算「能直接控制那台机器」
            "terminal/local/ws",
            "terminal/ws/1",
            "docker/exec/ws",
            "task/ws/9",
            "system/files/download",
            "database/export",
        ] {
            assert!(proxy_allowed(path), "{path} 应在节点视图可代理");
        }
    }

    /// 主控自有面：这几类代理过去会**改错对象**，必须永远打主控。
    #[test]
    fn proxy_policy_blocks_master_owned_surfaces() {
        for path in [
            "",
            // 会话属于主控：节点没有这个登录态
            "auth/current",
            "auth/logout",
            // 节点台账：代理过去就是 A→B→A 的递归代理
            "node/list",
            "node/overview",
            "nodes/1/system/info",
            // Token 控制台管的始终是「谁在调用这个面板」
            "dev/api-token/list",
            // 主控进程自己的探针与内置文档
            "health",
            "docs/zap-api",
        ] {
            assert!(!proxy_allowed(path), "{path} 不该经节点代理");
        }
        // 同前缀但不同段的路径不能被误伤（按段边界比对）
        assert!(proxy_allowed("nodex/ping"));
        assert!(proxy_allowed("devtools/panel"));
        assert!(proxy_allowed("authored/list"));
    }

    /// 路径非法（空段 / 穿越）仍然拒绝，且与「主控自有面」区分开，
    /// 否则前端提示会把人引到完全无关的方向。
    #[test]
    fn proxy_policy_rejects_traversal() {
        assert!(matches!(
            proxy_deny_reason("system/overview/../../auth/current"),
            Some(ProxyDeny::BadPath)
        ));
        assert!(matches!(
            proxy_deny_reason("node/list"),
            Some(ProxyDeny::MasterOwned)
        ));
        assert!(proxy_deny_reason("site/add").is_none());
    }

    /// 凭据置换是接管的底线：浏览器带的是**主控会话 JWT**，
    /// 原样转出去等于把主控凭据交给受控端，那台机器可以拿它反打主控。
    #[test]
    fn proxy_swaps_query_credential() {
        let q = swap_query_token(
            Some("token=MASTERJWT&path=%2Fvar&follow=1"),
            "zap_node_token",
        );
        assert!(q.contains("token=zap_node_token"), "{q}");
        assert!(!q.contains("MASTERJWT"), "主控凭据不得外流：{q}");
        // 其余参数原样保留
        assert!(q.contains("path=%2Fvar") && q.contains("follow=1"), "{q}");
        // 没有 token 参数时补一个；query 为 None 也要产出可用串
        assert!(swap_query_token(Some("id=3"), "T").contains("token=T"));
        assert_eq!(swap_query_token(None, "T"), "token=T");
        // 只认键名 `token`，不误伤 `token_id` 这类业务参数
        let q2 = swap_query_token(Some("token_id=7&token=X"), "T");
        assert!(q2.contains("token_id=7") && !q2.contains("token=X"), "{q2}");
    }

    /// body 里带凭据的接口（`/plugin/cancel`）同样置换；
    /// 非 JSON、非对象、字段不是字符串时**原样返回**——代理不该改写业务载荷。
    #[test]
    fn proxy_swaps_body_credential() {
        let out = swap_body_token(br#"{"token":"MASTERJWT","id":3}"#, "zap_node_token");
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["token"], "zap_node_token");
        assert_eq!(v["id"], 3);

        for raw in [
            br#"{"id":3}"#.as_slice(),
            br#"{"token":123}"#.as_slice(),
            br#"[["token","x"]]"#.as_slice(),
            b"not json".as_slice(),
            b"".as_slice(),
        ] {
            assert_eq!(swap_body_token(raw, "T").as_slice(), raw, "不该被改写");
        }
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
