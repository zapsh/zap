// SPDX-License-Identifier: AGPL-3.0-only
//! SSL 证书自动续期调度器。
//!
//! 每天扫描一次 ACME 签发的证书（证书按天到期，无需高频扫描），做两件事：
//!   1. **到期提醒**：剩余天数进入提醒窗口即通知（带冷却，与是否开启自动续期无关，
//!      手动申请的证书同样需要提醒）；
//!   2. **自动续期 + 部署**：剩余天数进入续期窗口且已开启自动续期时，重新签发
//!      （就地更新原证书，站点绑定不变）→ 重新部署到所有绑定站点（落盘 + nginx 重载）
//!      → 按结果发送「部署成功 / 部署失败」通知。
//!
//! 自动续期只对**无需人工介入**的验证方式生效：http-01（面板自动落盘验证文件）与
//! dns-01 + 自动 DNS（面板调服务商 API 建 TXT）。dns-01 手动需要人工加解析记录，
//! 无法自动完成，因此只提醒、不自动续期。

use std::time::Duration;

use tracing::{info, warn};

use crate::db::get_db_pool;
use crate::zap::{acme, notify};

/// 扫描周期：证书续期是按天计的事（有效期 90 天、续期窗口 30 天），一天扫一次足够
const SCAN_INTERVAL_SECS: i64 = 24 * 3600;
/// 空闲节拍：仅用于判断「到没到下次扫描时间」，不做任何扫描动作（一次 KV 读，开销可忽略）
const IDLE_TICK_SECS: u64 = 3600;
/// 续期窗口：剩余天数 ≤ 该值即触发续期（LE 证书 90 天，默认提前 30 天续签）
const DEFAULT_RENEW_DAYS: i64 = 30;
/// 到期提醒窗口：剩余天数 ≤ 该值即开始提醒
const DEFAULT_WARN_DAYS: i64 = 20;
/// 续期失败后的重试间隔。
///
/// 取 36h 而非更短，是卡着 Let's Encrypt「同一组标识符每 7 天 5 张」这条限制的
/// 回充速率来的：该限制每 34 小时回充 1 张，重试间隔大于 34h 就永远不会把配额耗光；
/// 否则连续失败几天后配额见底，等到真需要续期时反而被限流卡住。
/// （走 ARI 的续期豁免全部限制，但降级 / 老证书仍受此约束，故按最坏情况设。）
const RETRY_GAP_SECS: i64 = 36 * 3600;
/// 队列内两张证书续期之间的间隔：串行也别贴太紧，给服务端喘息
const RENEW_GAP_SECS: u64 = 60;
/// 等待单次签发完成的超时（DNS 传播 + 服务端重试可能较久）
const WAIT_TIMEOUT: Duration = Duration::from_secs(25 * 60);
/// 等待期间的轮询间隔
const POLL_SECS: u64 = 5;

/// 全局策略键：续期窗口天数
const K_RENEW_DAYS: &str = "cert_renew_days";
/// 全局策略键：到期提醒窗口天数
const K_WARN_DAYS: &str = "cert_expire_warn_days";
/// 全局策略键：上次扫描时间（持久化，避免面板重启后重复扫描）
const K_LAST_SCAN: &str = "cert_renew_last_scan";

/// 启动调度循环（后台任务，不阻塞主流程）。
///
/// 每天真正扫描一次：距上次扫描满 `SCAN_INTERVAL_SECS` 才执行 [`tick`]，
/// 其余时间只做一次空判断（读一个 KV），不做任何查询。
pub fn start() {
    tokio::spawn(async {
        // 等服务与建库初始化完成后再开始判断，避免与启动流程抢跑
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            maybe_scan().await;
            tokio::time::sleep(Duration::from_secs(IDLE_TICK_SECS)).await;
        }
    });
}

/// 到点才扫：上次扫描时间持久化在 `global_settings`，
/// 因此面板重启不会重复扫描，没扫过（或已超期）则会补扫一次。
async fn maybe_scan() {
    let last = gs_i64(K_LAST_SCAN, 0).await;
    let now = chrono::Utc::now().timestamp();
    if now - last < SCAN_INTERVAL_SECS {
        return;
    }
    tick().await;
    gs_set(K_LAST_SCAN, &now.to_string()).await;
}

/// 读全局策略 KV（与 backup_scheduler 同源，各自独立实现避免跨模块依赖）。
async fn gs_get(key: &str) -> String {
    let pool = get_db_pool().await;
    let r: Option<(String,)> = sqlx::query_as("SELECT value FROM global_settings WHERE key=?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    r.map(|x| x.0).unwrap_or_default()
}

/// 读全局策略中的整数配置，缺省或非法时回退 `default`。
async fn gs_i64(key: &str, default: i64) -> i64 {
    gs_get(key)
        .await
        .trim()
        .parse::<i64>()
        .unwrap_or(default)
        .max(0)
}

/// 写全局策略 KV。
async fn gs_set(key: &str, v: &str) {
    let pool = get_db_pool().await;
    let _ = sqlx::query(
        "INSERT INTO global_settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(key)
    .bind(v)
    .execute(pool)
    .await;
}

async fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 读取全局续期阈值：(续期窗口天数, 提醒窗口天数)。
pub async fn config() -> (i64, i64) {
    (
        gs_i64(K_RENEW_DAYS, DEFAULT_RENEW_DAYS).await,
        gs_i64(K_WARN_DAYS, DEFAULT_WARN_DAYS).await,
    )
}

/// 更新全局续期阈值（只覆盖显式传入的项）。
pub async fn set_config(renew_days: Option<i64>, warn_days: Option<i64>) {
    if let Some(d) = renew_days {
        gs_set(K_RENEW_DAYS, &d.max(0).to_string()).await;
    }
    if let Some(d) = warn_days {
        gs_set(K_WARN_DAYS, &d.max(0).to_string()).await;
    }
}

/// 手动触发续期：校验证书存在后交给后台跑全流程（签发 → 部署 → 通知），调用方立即返回。
pub async fn renew_now(cert_id: i64) -> Result<(), String> {
    let pool = get_db_pool().await;
    let row: Option<(i64, String, String)> =
        sqlx::query_as("SELECT user_id, name, domains FROM ssl_cert WHERE id = ?")
            .bind(cert_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("读取证书失败: {e}"))?;
    let (user_id, name, domains) = row.ok_or_else(|| "证书不存在".to_string())?;

    set_renew_state(cert_id, now_secs().await, 0, "手动续期进行中").await;
    tokio::spawn(async move {
        run_renew(cert_id, user_id, name, domains).await;
    });
    Ok(())
}

/// 记录本次续期的执行状态（0 进行中 / 1 成功 / -1 失败）。
async fn set_renew_state(cert_id: i64, ts: i64, status: i64, msg: &str) {
    let pool = get_db_pool().await;
    let _ = sqlx::query(
        "UPDATE ssl_cert SET last_renew_at = ?, renew_status = ?, renew_msg = ? WHERE id = ?",
    )
    .bind(ts)
    .bind(status)
    .bind(msg)
    .bind(cert_id)
    .execute(pool)
    .await;
}

async fn tick() {
    let renew_days = gs_i64(K_RENEW_DAYS, DEFAULT_RENEW_DAYS).await;
    let warn_days = gs_i64(K_WARN_DAYS, DEFAULT_WARN_DAYS).await;
    let now = now_secs().await;

    let pool = get_db_pool().await;
    // 只看 ACME 签发的证书：自签 / 上传的证书没有 ACME 账户，无法自动续签
    let rows: Vec<(i64, i64, String, String, i64, String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, user_id, name, domains, auto_renew, challenge_type, dns_mode,
                not_after, last_renew_at
         FROM ssl_cert
         WHERE status = 1 AND not_after > 0
           AND cert_type IN ('letsencrypt', 'letsencrypt-staging')",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    // 本轮待续期的证书：先进队列，再由单个工作线程串行处理（见 run_queue）
    let mut jobs: Vec<RenewJob> = Vec::new();

    for (id, user_id, name, domains, auto_renew, challenge_type, dns_mode, not_after, last_renew_at) in
        rows
    {
        let days = (not_after - now) / 86400;

        // 1) 到期提醒：无论是否开启自动续期都要提醒（含已过期，days ≤ 0）
        if days <= warn_days {
            notify::cert_expiring(id, user_id, &name, &domains, days, not_after).await;
        }

        // 2) 自动续期
        if auto_renew != 1 || days > renew_days {
            continue;
        }
        // 失败冷却：刚续过（无论成败）就不再重复触发，避免刷 LE 频率限制
        if now - last_renew_at < RETRY_GAP_SECS {
            continue;
        }
        // dns-01 手动需人工加 TXT，自动续期无法完成（签发会一直停在 pending）
        if challenge_type == "dns-01" && dns_mode != "auto" {
            continue;
        }

        info!(cert = id, days = days, "证书进入续期窗口，加入续期队列");
        set_renew_state(id, now, 0, "排队等待续期").await;
        jobs.push(RenewJob {
            cert_id: id,
            user_id,
            name,
            domains,
        });
    }

    if !jobs.is_empty() {
        info!(count = jobs.len(), "本轮续期队列开始处理（串行）");
        tokio::spawn(run_queue(jobs));
    }
}

/// 一条待续期任务。
struct RenewJob {
    cert_id: i64,
    user_id: i64,
    name: String,
    domains: String,
}

/// 单次续期结果。
enum RenewOutcome {
    /// 续期并部署成功
    Ok,
    /// 失败（创建订单失败 / 签发失败 / 部署失败）
    Failed,
    /// 命中 Let's Encrypt 频率限制：后续任务应顺延，别再撞配额
    RateLimited,
}

/// 命中频率限制的判断：签发失败信息经 `friendly_err` 翻译后含「频率限制」，
/// 原始错误里则是 `rateLimited` / HTTP 429。
fn is_rate_limited(msg: &str) -> bool {
    msg.contains("频率限制") || msg.contains("rateLimited") || msg.contains("429")
}

/// 串行处理续期队列：一次只续一张，两张之间留间隔。
///
/// **刻意不并发**：Let's Encrypt 对「同一注册域名每周证书数」「重复证书」「失败验证次数」
/// 都有配额，几十张证书一起冲很容易把配额打满，结果一张都续不上；串行既稳，
/// 又能在命中限流时立刻停手，把剩余任务顺延到下一轮。
async fn run_queue(jobs: Vec<RenewJob>) {
    let total = jobs.len();
    for (i, job) in jobs.into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_secs(RENEW_GAP_SECS)).await;
        }
        let outcome = run_renew(job.cert_id, job.user_id, job.name, job.domains).await;
        if matches!(outcome, RenewOutcome::RateLimited) {
            let left = total - i - 1;
            warn!(
                "命中 Let's Encrypt 频率限制，本轮剩余 {left} 张证书顺延到下一轮再续",
            );
            break;
        }
    }
}

/// 等待订单签发完成的终态。
enum WaitResult {
    Issued,
    Failed(String),
    Timeout,
}

/// 轮询订单直到签发 / 失败：签发由 `acme::process_order` 在后台推进，这里只等终态。
async fn wait_issued(order_id: i64) -> WaitResult {
    let deadline = tokio::time::Instant::now() + WAIT_TIMEOUT;
    loop {
        match acme::view(order_id).await {
            Ok(o) => match o.status.as_str() {
                "issued" => return WaitResult::Issued,
                "failed" | "cancelled" => {
                    let e = if o.error.trim().is_empty() {
                        "订单签发失败".to_string()
                    } else {
                        o.error.clone()
                    };
                    return WaitResult::Failed(e);
                }
                _ => {}
            },
            Err(e) => return WaitResult::Failed(e),
        }
        if tokio::time::Instant::now() >= deadline {
            return WaitResult::Timeout;
        }
        tokio::time::sleep(Duration::from_secs(POLL_SECS)).await;
    }
}

/// 把续期后的证书重新部署到所有绑定站点：落盘证书文件 → nginx -t → reload。
///
/// 返回 (部署成功站点, 部署失败「站点名 — 原因」)。
async fn deploy(cert_id: i64) -> (Vec<String>, Vec<String>) {
    let pool = get_db_pool().await;
    let sites: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, name FROM site WHERE ssl_cert_id = ?")
            .bind(cert_id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();

    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for (sid, sname) in sites {
        match crate::routers::site::sync_one_site(sid).await {
            Ok(_) => ok.push(sname),
            Err(e) => bad.push(format!("{sname} — {e}")),
        }
    }
    (ok, bad)
}

/// 读取续期后的新有效期（用于通知文案）。
async fn expire_of(cert_id: i64) -> i64 {
    let pool = get_db_pool().await;
    sqlx::query_scalar::<_, i64>("SELECT not_after FROM ssl_cert WHERE id = ?")
        .bind(cert_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// 单次续期全流程：创建订单 → 等签发 → 部署 → 通知。
async fn run_renew(
    cert_id: i64,
    user_id: i64,
    name: String,
    domains: String,
) -> RenewOutcome {
    let now = now_secs().await;

    // 1) 创建续期订单（签发成功后会就地更新原证书，站点绑定不变）
    let order = match acme::renew_cert(cert_id).await {
        Ok(o) => o,
        Err(e) => {
            let msg = format!("创建续期订单失败：{e}");
            warn!(cert = cert_id, "{}", msg);
            set_renew_state(cert_id, now, -1, &msg).await;
            notify::cert_renew_fail(user_id, &name, &domains, &msg).await;
            return if is_rate_limited(&msg) {
                RenewOutcome::RateLimited
            } else {
                RenewOutcome::Failed
            };
        }
    };

    // 2) 等签发完成（只等一次，拿到终态）
    let msg = match wait_issued(order.id).await {
        WaitResult::Issued => String::new(),
        WaitResult::Failed(e) => format!("签发失败：{e}"),
        WaitResult::Timeout => "等待签发超时（25 分钟），请检查域名解析与验证方式".to_string(),
    };
    if !msg.is_empty() {
        warn!(cert = cert_id, order = order.id, "{}", msg);
        set_renew_state(cert_id, now, -1, &msg).await;
        notify::cert_renew_fail(user_id, &name, &domains, &msg).await;
        // 命中限流要让上层停手，剩余任务顺延到下一轮
        return if is_rate_limited(&msg) {
            RenewOutcome::RateLimited
        } else {
            RenewOutcome::Failed
        };
    }

    // 3) 部署到绑定站点
    let (deployed, failed) = deploy(cert_id).await;
    let expire = expire_of(cert_id).await;

    if failed.is_empty() {
        let msg = if deployed.is_empty() {
            "续期成功（未绑定站点，无需部署）".to_string()
        } else {
            format!("续期成功，已部署 {} 个站点", deployed.len())
        };
        info!(cert = cert_id, "{}", msg);
        set_renew_state(cert_id, now, 1, &msg).await;
        notify::cert_renew_ok(user_id, &name, &domains, expire, &deployed).await;
        RenewOutcome::Ok
    } else {
        let msg = format!("证书已签发，但站点部署失败：{}", failed.join("；"));
        warn!(cert = cert_id, "{}", msg);
        set_renew_state(cert_id, now, -1, &msg).await;
        notify::cert_renew_fail(user_id, &name, &domains, &msg).await;
        RenewOutcome::Failed
    }
}
