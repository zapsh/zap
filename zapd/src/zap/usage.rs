//! 用户资源用量采集（磁盘 / 带宽）与站点流量分析。
//!
//! - **磁盘**：定时对 `user.home_dir` 统计占用，写回
//!   `user.disk_used_bytes` / `disk_stat_at`；
//!   面板以 `zapadm` 运行，用户家目录多为 0700（属主为各自的 Linux 账号），
//!   因此统一委托 `zapexec`（root）执行 `du`，本地 du 仅作兜底；
//! - **带宽**：增量解析站点 `access.log`（nginx main / combined 的
//!   `$body_bytes_sent`），按站点记 `site.traffic_*`，再按归属用户汇总
//!   写入 `user.bandwidth_used_bytes`（按月，`bandwidth_period` 为 YYYYMM）；
//! - **分析**：同一次解析顺带产出按天流量（`site_traffic_daily`）与
//!   按天 Top URL（`site_traffic_path`），供站点「流量」页使用。
//!
//! 日志路径取 `site.log_root/access.log`（不依赖目录命名）。

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use chrono::Local;
use tokio::process::Command;
use tracing::{debug, warn};

use crate::db::get_db_pool;
use zap_proto::Request;

/// 统计周期（YYYYMM）：跨月自动重置本月计数
fn period_now() -> String {
    Local::now().format("%Y%m").to_string()
}

/// 单日 Top URL 保留条数（超出按 hits 修剪，避免表无限膨胀）
const TOP_PATH_KEEP: i64 = 200;

/// access.log 单行解析结果（均为行内切片，避免逐行分配）
struct LineStat<'a> {
    /// 响应字节数（$body_bytes_sent）
    bytes: u64,
    /// 请求日期 YYYYMMDD（解析失败为空串）
    day: String,
    /// 请求路径（去掉 query string）
    path: &'a str,
}

/// nginx 时间字段 `15/Sep/2026:10:00:00` → `20260915`；解析失败返回空串。
fn day_of_nginx_time(t: &str) -> String {
    let date = t.split(':').next().unwrap_or(t);
    let mut it = date.split('/');
    let dd = it.next().unwrap_or("");
    let mon = it.next().unwrap_or("");
    let yyyy = it.next().unwrap_or("");
    let mm = match mon {
        "Jan" => "01",
        "Feb" => "02",
        "Mar" => "03",
        "Apr" => "04",
        "May" => "05",
        "Jun" => "06",
        "Jul" => "07",
        "Aug" => "08",
        "Sep" => "09",
        "Oct" => "10",
        "Nov" => "11",
        "Dec" => "12",
        _ => "",
    };
    if mm.is_empty() || dd.len() != 2 || yyyy.len() != 4 {
        return String::new();
    }
    format!("{yyyy}{mm}{dd}")
}

/// 解析一行：`ip - - [time] "GET /path?x=1 HTTP/1.1" 200 1234 "ref" "ua"`
///
/// 取请求行内的路径与结束引号后的第二个字段（`$body_bytes_sent`），
/// 可容忍 referer / UA 中的空格；解析失败按 0 / 空值处理。
fn parse_line(line: &str) -> LineStat<'_> {
    let mut stat = LineStat {
        bytes: 0,
        day: String::new(),
        path: "",
    };
    // 时间：[15/Sep/2026:10:00:00 +0800]
    if let (Some(s), Some(e)) = (line.find('['), line.find(']'))
        && s < e
    {
        stat.day = day_of_nginx_time(&line[s + 1..e]);
    }
    let Some(start) = line.find('"') else {
        return stat;
    };
    let rest = &line[start + 1..];
    let Some(end) = rest.find('"') else {
        return stat;
    };
    // 请求行：METHOD path PROTO
    let req = &rest[..end];
    let mut ri = req.split_whitespace();
    let _method = ri.next();
    if let Some(p) = ri.next() {
        stat.path = match p.find('?') {
            Some(i) => &p[..i],
            None => p,
        };
    }
    // 请求行之后：status bytes "referer" ...
    let mut it = rest[end + 1..].split_whitespace();
    let _status = it.next();
    if let Some(v) = it.next() {
        stat.bytes = v.parse::<u64>().unwrap_or(0);
    }
    stat
}

/// 站点流量采集游标（字段顺序与下方 SELECT 一一对应）
#[derive(sqlx::FromRow)]
struct SiteTrafficRow {
    id: i64,
    log_root: String,
    /// 已解析到的字节偏移
    traffic_offset: i64,
    /// 上次解析时的 inode（变化 = 日志被轮转 / 重建）
    traffic_inode: String,
    /// 月计数所属周期 YYYYMM
    traffic_month: String,
    traffic_month_bytes: i64,
    traffic_total_bytes: i64,
}

/// 一次日志解析的聚合结果
#[derive(Default)]
struct LogAgg {
    /// 总响应字节数
    bytes: u64,
    /// day -> (bytes, requests)
    days: HashMap<String, (u64, u64)>,
    /// (day, path) -> (hits, bytes)
    paths: HashMap<(String, String), (u64, u64)>,
}

/// 目录字节数（本地 `du`）：`du -sb` 优先，不支持 `-b` 时回退 `du -sk` × 1024。
///
/// 仅作兜底：面板以 zapadm 运行，用户家目录（0700）读不到，
/// 正常路径走 [`du_batch_root`]（root 执行）。
async fn du_bytes(dir: &str) -> Option<u64> {
    async fn run(args: [&str; 2]) -> Option<String> {
        let out = Command::new("du").args(args).output().await.ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    }

    if let Some(s) = run(["-sb", dir]).await
        && let Ok(v) = s.split_whitespace().next().unwrap_or("").parse::<u64>()
    {
        return Some(v);
    }
    let s = run(["-sk", dir]).await?;
    let kb: u64 = s.split_whitespace().next()?.parse().ok()?;
    Some(kb.saturating_mul(1024))
}

/// 单批路径上限：与 `zapexec` 的 `DISK_USAGE_MAX` 对齐并留余量
const DU_BATCH: usize = 200;

/// root 批量统计结果：`usage` 为「目录 → 字节数」，`missing` 为 root 也 stat 不到的
/// 目录（站点尚未创建等），一律按 0 计。
#[derive(Default)]
struct RootUsage {
    usage: HashMap<String, u64>,
    missing: HashSet<String>,
}

/// 委托 `zapexec`（root）批量统计目录占用。
///
/// 面板（zapd）以 zapadm 运行，用户家目录 0700 读不到，`du` 必须由 root 执行；
/// 失败时返回空结果，调用方逐目录回退本地 `du_bytes`。
async fn du_batch_root(dirs: &[String]) -> RootUsage {
    let mut out = RootUsage::default();
    let Ok(resp) = crate::zapexec::call(Request::FsDiskUsage {
        paths: dirs.to_vec(),
    })
    .await
    else {
        return out;
    };
    if resp.code != 0 {
        warn!("磁盘用量采集（root）失败: {}", resp.message);
        return out;
    }
    // 目录不存在 / 读不到不算失败（站点尚未创建是常态）：只在跟踪日志里说明
    if let Some(err) = resp
        .data
        .as_ref()
        .and_then(|d| d.get("error"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        debug!("磁盘用量采集（root）: {}", err);
    }
    if let Some(obj) = resp
        .data
        .as_ref()
        .and_then(|d| d.get("usage"))
        .and_then(|v| v.as_object())
    {
        out.usage = obj
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
            .collect();
    }
    if let Some(arr) = resp
        .data
        .as_ref()
        .and_then(|d| d.get("missing"))
        .and_then(|v| v.as_array())
    {
        out.missing = arr
            .iter()
            .filter_map(|v| Some(v.as_str()?.to_string()))
            .collect();
    }
    out
}

/// 分批（每批 ≤ `DU_BATCH`）委托 root 统计，避免单次请求路径过多被截断。
async fn du_batch_root_all(dirs: &[String]) -> RootUsage {
    let mut out = RootUsage::default();
    for chunk in dirs.chunks(DU_BATCH) {
        let r = du_batch_root(chunk).await;
        out.usage.extend(r.usage);
        out.missing.extend(r.missing);
    }
    out
}

/// 采集全部用户的家目录磁盘用量
pub async fn collect_disk_usage() {
    let pool = get_db_pool().await;
    // 成员（子账号）共享父账号的家目录：跳过，否则父账号的用量会被重复计数
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, home_dir FROM user WHERE home_dir <> '' AND user_kind <> 1")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let now = Local::now().timestamp();
    // 家目录多为 0700（属主为各自 Linux 账号）：zapadm 连 stat 都做不到，
    // 因此不自行过滤，全部交给 root 统计，由 root 判定目录是否存在
    let homes: Vec<String> = rows
        .iter()
        .map(|(_, h)| h.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let root = du_batch_root_all(&homes).await;

    for (id, home_dir) in rows {
        // 家目录不存在（Linux 账号还没建 / 已删除）：用量记 0
        if root.missing.contains(&home_dir) {
            let _ =
                sqlx::query("UPDATE user SET disk_used_bytes = 0, disk_stat_at = ? WHERE id = ?")
                    .bind(now)
                    .bind(id)
                    .execute(pool)
                    .await;
            continue;
        }
        // root 未覆盖（zapexec 不可用 / du 报错）时回退本地 du
        let bytes = match root.usage.get(&home_dir) {
            Some(v) => Some(*v),
            None => du_bytes(&home_dir).await,
        };
        match bytes {
            Some(bytes) => {
                let _ = sqlx::query(
                    "UPDATE user SET disk_used_bytes = ?, disk_stat_at = ? WHERE id = ?",
                )
                .bind(bytes as i64)
                .bind(now)
                .bind(id)
                .execute(pool)
                .await;
            }
            None => warn!("磁盘用量采集失败: user={} home={}", id, home_dir),
        }
    }
}

/// 从 `offset` 起聚合日志（同步 IO，调用方包 spawn_blocking）
fn aggregate(path: &Path, offset: u64) -> std::io::Result<LogAgg> {
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut agg = LogAgg::default();
    let mut reader = BufReader::new(f);
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let stat = parse_line(line.trim_end());
        if stat.day.is_empty() {
            // 时间解析失败的行只累计总字节，不进入按天/Top URL 统计
            agg.bytes = agg.bytes.saturating_add(stat.bytes);
            continue;
        }
        agg.bytes = agg.bytes.saturating_add(stat.bytes);
        let e = agg.days.entry(stat.day.clone()).or_insert((0, 0));
        e.0 = e.0.saturating_add(stat.bytes);
        e.1 += 1;
        if !stat.path.is_empty() {
            let e = agg
                .paths
                .entry((stat.day.clone(), stat.path.to_string()))
                .or_insert((0, 0));
            e.0 += 1;
            e.1 = e.1.saturating_add(stat.bytes);
        }
    }
    Ok(agg)
}

/// 采集站点磁盘占用（web_root + log_root，与用户家目录同口径）
pub async fn collect_site_disk() {
    let pool = get_db_pool().await;
    let rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, web_root, log_root FROM site")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let now = Local::now().timestamp();
    // 站点目录位于用户家目录下（0700），同样委托 root 批量统计；
    // 存在性一律以 root 的判断为准（zapadm stat 不到家目录，不能自行过滤）
    let dirs: Vec<String> = rows
        .iter()
        .flat_map(|(_, w, l)| [w.clone(), l.clone()])
        .filter(|d| !d.trim().is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let root = du_batch_root_all(&dirs).await;

    for (id, web_root, log_root) in rows {
        let mut total: u64 = 0;
        let mut sampled = false;
        for dir in [&web_root, &log_root] {
            if dir.trim().is_empty() {
                continue;
            }
            // 目录不存在（站点还没建 / 已删除）：按 0 计
            if root.missing.contains(dir) {
                sampled = true;
                continue;
            }
            let v = match root.usage.get(dir) {
                Some(v) => Some(*v),
                None => du_bytes(dir).await,
            };
            match v {
                Some(v) => {
                    total = total.saturating_add(v);
                    sampled = true;
                }
                None => warn!("站点磁盘用量采集失败: site={} dir={}", id, dir),
            }
        }
        // 目录存在但采不到时才保留上次结果，避免把有效值覆盖成 0
        if !sampled {
            continue;
        }
        let _ = sqlx::query("UPDATE site SET disk_used_bytes = ?, disk_stat_at = ? WHERE id = ?")
            .bind(total as i64)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await;
    }
}

/// 采集站点流量并汇总到用户（本月出站字节数）
pub async fn collect_bandwidth() {
    let pool = get_db_pool().await;
    let period = period_now();
    let rows: Vec<SiteTrafficRow> = sqlx::query_as(
        "SELECT id, log_root, traffic_offset, traffic_inode, traffic_month, \
                traffic_month_bytes, traffic_total_bytes \
         FROM site WHERE log_root <> ''",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let now = Local::now().timestamp();

    for row in rows {
        let id = row.id;
        let path: PathBuf = Path::new(&row.log_root).join("access.log");
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let cur_inode = meta.ino().to_string();
        let size = meta.len();
        // 日志轮转 / 重建：inode 变化或文件变小 → 从头重新统计
        let start = if cur_inode != row.traffic_inode || size < row.traffic_offset as u64 {
            0
        } else {
            row.traffic_offset as u64
        };
        if size <= start {
            continue;
        }

        let p = path.clone();
        let agg = match tokio::task::spawn_blocking(move || aggregate(&p, start)).await {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                warn!("流量日志解析失败: site={} err={}", id, e);
                continue;
            }
            Err(_) => continue,
        };
        let delta = agg.bytes;
        if delta == 0 {
            debug!("站点 {} 本轮无新增流量", id);
        }

        let new_month_bytes = if row.traffic_month == period {
            row.traffic_month_bytes.saturating_add(delta as i64)
        } else {
            delta as i64
        };
        let _ = sqlx::query(
            "UPDATE site SET traffic_offset = ?, traffic_inode = ?, traffic_total_bytes = ?, \
             traffic_month_bytes = ?, traffic_month = ?, traffic_stat_at = ? WHERE id = ?",
        )
        .bind(size as i64)
        .bind(&cur_inode)
        .bind(row.traffic_total_bytes.saturating_add(delta as i64))
        .bind(new_month_bytes)
        .bind(&period)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await;

        // 按天汇总 + 按天 Top URL（增量累加）
        for (day, (bytes, reqs)) in &agg.days {
            let _ = sqlx::query(
                "INSERT INTO site_traffic_daily (site_id, day, bytes, requests) VALUES (?, ?, ?, ?) \
                 ON CONFLICT(site_id, day) DO UPDATE SET \
                   bytes = bytes + excluded.bytes, requests = requests + excluded.requests",
            )
            .bind(id)
            .bind(day)
            .bind(*bytes as i64)
            .bind(*reqs as i64)
            .execute(pool)
            .await;
        }
        let mut touched_days: Vec<String> = agg.paths.keys().map(|(d, _)| d.clone()).collect();
        touched_days.sort();
        touched_days.dedup();
        for ((day, p), (hits, bytes)) in &agg.paths {
            let _ = sqlx::query(
                "INSERT INTO site_traffic_path (site_id, day, path, hits, bytes) VALUES (?, ?, ?, ?, ?) \
                 ON CONFLICT(site_id, day, path) DO UPDATE SET \
                   hits = hits + excluded.hits, bytes = bytes + excluded.bytes",
            )
            .bind(id)
            .bind(day)
            .bind(p)
            .bind(*hits as i64)
            .bind(*bytes as i64)
            .execute(pool)
            .await;
        }
        for day in touched_days {
            let _ = sqlx::query(
                "DELETE FROM site_traffic_path WHERE site_id = ? AND day = ? AND path NOT IN ( \
                   SELECT path FROM site_traffic_path WHERE site_id = ? AND day = ? \
                   ORDER BY hits DESC, bytes DESC LIMIT ?)",
            )
            .bind(id)
            .bind(&day)
            .bind(id)
            .bind(&day)
            .bind(TOP_PATH_KEEP)
            .execute(pool)
            .await;
        }
    }

    // 按归属用户汇总本月流量（无站点的用户归零）
    let _ = sqlx::query(
        "UPDATE user SET bandwidth_used_bytes = COALESCE( \
            (SELECT SUM(s.traffic_month_bytes) FROM site s \
              WHERE s.user_id = user.id AND s.traffic_month = ?), 0), \
         bandwidth_period = ?, bandwidth_stat_at = ?",
    )
    .bind(&period)
    .bind(&period)
    .bind(now)
    .execute(pool)
    .await;
}

/// 一次跑完磁盘 + 带宽（供定时任务调用）
pub async fn collect_all() {
    collect_disk_usage().await;
    collect_site_disk().await;
    collect_bandwidth().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_line_fields() {
        let line = r#"1.2.3.4 - - [15/Sep/2026:10:00:00 +0800] "GET /a/b?x=1 HTTP/1.1" 200 1234 "http://ref" "Mozilla/5.0 (X11)""#;
        let s = parse_line(line);
        assert_eq!(s.bytes, 1234);
        assert_eq!(s.day, "20260915");
        assert_eq!(s.path, "/a/b");

        // referer / UA 含空格、POST + 404
        let line2 = r#"1.2.3.4 - alice [01/Jan/2026:00:00:00 +0800] "POST /p HTTP/1.0" 404 56 "-" "curl 8.0 x""#;
        let s2 = parse_line(line2);
        assert_eq!(s2.bytes, 56);
        assert_eq!(s2.day, "20260101");
        assert_eq!(s2.path, "/p");

        // 异常行
        let s3 = parse_line("garbage");
        assert_eq!(s3.bytes, 0);
        assert!(s3.day.is_empty());
    }

    #[test]
    fn nginx_time_to_day() {
        assert_eq!(day_of_nginx_time("15/Sep/2026:10:00:00"), "20260915");
        assert_eq!(day_of_nginx_time("9/Feb/2025:1:2:3"), "");
        assert_eq!(day_of_nginx_time("bad"), "");
    }
}
