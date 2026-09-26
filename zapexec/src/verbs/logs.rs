//! 站点 nginx 日志：轮转 / 读取 / 归档列表 / 清空（root 执行）。
//!
//! 面板自管日志轮转，替代外部 logrotate：
//! - **轮转**：按天把 `access.log` / `error.log` 切成 `{kind}.log-YYYYMMDD` 并 gzip；
//!   删除超过保留天数的归档；最后 `nginx -s reopen`（USR1）让 nginx 重新打开日志
//!   —— 比 reload 更轻，不断开现有连接。
//! - **读取**：只回扫文件尾部（大日志避免整体读入），支持关键词 / 状态码过滤。
//! - **清空**：truncate 当前日志后同样 reopen。

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::Local;
use serde_json::json;
use zap_proto::Response;

use super::root_cmd;
use super::site;

/// 单次读取最多返回的行数（防止超大结果拖垮连接）
const MAX_LINES: usize = 2000;
/// 尾部回扫字节数：日志很大时只读最后一段
const TAIL_SCAN_BYTES: u64 = 8 * 1024 * 1024;
/// 归档解压上限（压缩后体积），超过则拒绝在线查看
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

// ── 工具 ──────────────────────────────────────────────────

/// 日志目录校验：必须是绝对路径且不含 `..`
fn safe_log_root(raw: &str) -> Result<PathBuf, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("日志目录为空".to_string());
    }
    if !s.starts_with('/') {
        return Err("日志目录必须是绝对路径".to_string());
    }
    if s.contains("..") {
        return Err("日志目录不允许包含 ..".to_string());
    }
    Ok(PathBuf::from(s))
}

/// 归一化日志类型：error | access
fn kind_of(kind: &str) -> &'static str {
    let k = kind.trim();
    if k.eq_ignore_ascii_case("error") {
        "error"
    } else if k.eq_ignore_ascii_case("waf") {
        // WAF 独立审计日志：开启站点 WAF 审计时由 ModSecurity 写入
        "waf"
    } else {
        "access"
    }
}

fn current_log(dir: &Path, kind: &str) -> PathBuf {
    dir.join(format!("{kind}.log"))
}

fn gz_of(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(".gz");
    PathBuf::from(s)
}

/// 归档名校验：只允许日志目录内的普通文件名（禁止路径穿越）
fn archive_path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let n = name.trim();
    if n.is_empty() || n.contains('/') || n.contains("..") || n.starts_with('.') {
        return Err("归档文件名不合法".to_string());
    }
    let p = dir.join(n);
    if !p.is_file() {
        return Err("归档文件不存在".to_string());
    }
    Ok(p)
}

/// 从 `access.log-20260915.gz` / `error.log-20260915` 取日期串（YYYYMMDD）
fn archive_date(name: &str) -> Option<String> {
    let base = name.strip_suffix(".gz").unwrap_or(name);
    let (_, tail) = base.split_once("-")?;
    let d = tail.split('-').next().unwrap_or("");
    if d.len() == 8 && d.chars().all(|c| c.is_ascii_digit()) {
        Some(d.to_string())
    } else {
        None
    }
}

/// 取 access.log 行的状态码（请求行结束引号后的第一个字段）
fn status_of(line: &str) -> &str {
    let Some(start) = line.find('"') else {
        return "";
    };
    let rest = &line[start + 1..];
    let Some(end) = rest.find('"') else {
        return "";
    };
    rest[end + 1..].split_whitespace().next().unwrap_or("")
}

/// 通知 nginx 重新打开日志文件（USR1）：不断连接，比 reload 更轻。
fn reopen_nginx() -> bool {
    let Some(conf) = site::find_nginx_conf_file() else {
        return false;
    };
    if !site::nginx_running() {
        return false;
    }
    let bin = site::nginx_bin(&conf);
    let quoted = bin.to_string_lossy().replace('\'', "'\\''");
    let ok = root_cmd(super::platform::SHELL)
        .args(["-c"])
        .arg(format!("'{quoted}' -s reopen 2>&1"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok {
        return true;
    }
    // 兜底：直接给 master 进程发 USR1
    match std::fs::read_to_string("/var/run/nginx.pid") {
        Ok(pid_text) => {
            let pid = pid_text.trim().to_string();
            if pid.is_empty() {
                return false;
            }
            root_cmd("kill")
                .args(["-USR1", &pid])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        Err(_) => false,
    }
}

/// 删除超过保留天数的归档，返回删除数量
fn prune_archives(dir: &Path, keep_days: u32) -> usize {
    let cutoff = (Local::now() - chrono::Duration::days(keep_days as i64))
        .format("%Y%m%d")
        .to_string();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0usize;
    for e in rd.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let Some(date) = archive_date(&name) else {
            continue;
        };
        if date < cutoff && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// 读取文件尾部文本（大文件只回扫最后 `TAIL_SCAN_BYTES`）
fn tail_text(path: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("打开日志失败: {e}"))?;
    let size = f
        .metadata()
        .map_err(|e| format!("读取日志信息失败: {e}"))?
        .len();
    let start = size.saturating_sub(TAIL_SCAN_BYTES);
    f.seek(SeekFrom::Start(start))
        .map_err(|e| format!("定位日志失败: {e}"))?;
    let mut buf = Vec::new();
    f.take(TAIL_SCAN_BYTES)
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取日志失败: {e}"))?;
    let text = String::from_utf8_lossy(&buf).to_string();
    // 非从头开始时首行可能被截断，丢弃
    if start > 0 {
        return Ok(text
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or("")
            .to_string());
    }
    Ok(text)
}

/// 读取归档（.gz 走 `gzip -dc`）
fn archived_text(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("读取归档失败: {e}"))?;
    if meta.len() > MAX_ARCHIVE_BYTES {
        return Err("归档过大，请到服务器本地查看".to_string());
    }
    if path.extension().and_then(|e| e.to_str()) == Some("gz") {
        let o = root_cmd("gzip")
            .args(["-dc"])
            .arg(path)
            .output()
            .map_err(|e| format!("解压归档失败: {e}"))?;
        if !o.status.success() {
            return Err("解压归档失败".to_string());
        }
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("读取归档失败: {e}"))
    }
}

/// 取尾部 `want` 行（按关键词 / 状态码过滤），保持原始时间顺序
fn pick_lines(text: &str, want: usize, keyword: &str, status: &str) -> Vec<String> {
    let kw = keyword.trim();
    let st = status.trim();
    let mut out: Vec<String> = text
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| kw.is_empty() || l.contains(kw))
        .filter(|l| st.is_empty() || status_of(l) == st)
        .take(want)
        .map(|l| l.to_string())
        .collect();
    out.reverse();
    out
}

// ── verbs ─────────────────────────────────────────────────

/// site.log_rotate：按天切割 + gzip + 清理超期归档 + nginx reopen
pub async fn rotate(log_roots: Vec<String>, keep_days: u32) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let stamp = Local::now().format("%Y%m%d").to_string();
        let keep = if keep_days == 0 { 30 } else { keep_days };
        let mut rotated: Vec<serde_json::Value> = Vec::new();
        let mut removed = 0usize;
        let mut errors: Vec<String> = Vec::new();

        for raw in &log_roots {
            let dir = match safe_log_root(raw) {
                Ok(d) => d,
                Err(e) => {
                    errors.push(format!("{raw}: {e}"));
                    continue;
                }
            };
            if !dir.is_dir() {
                continue;
            }
            for kind in ["access", "error", "waf"] {
                let cur = current_log(&dir, kind);
                let Ok(meta) = std::fs::metadata(&cur) else {
                    continue;
                };
                if meta.len() == 0 {
                    continue; // 空日志不产生归档
                }
                let mut dest = dir.join(format!("{kind}.log-{stamp}"));
                let mut n = 1;
                while dest.exists() || gz_of(&dest).exists() {
                    dest = dir.join(format!("{kind}.log-{stamp}-{n}"));
                    n += 1;
                }
                if let Err(e) = std::fs::rename(&cur, &dest) {
                    errors.push(format!("{raw} {kind}.log 切割失败: {e}"));
                    continue;
                }
                // gzip 失败不回滚（文件已切割，nginx reopen 后会重建当前日志）
                let archived = match root_cmd("gzip").arg("-f").arg(&dest).output() {
                    Ok(o) if o.status.success() => gz_of(&dest),
                    _ => dest,
                };
                rotated.push(json!({
                    "log_root": raw,
                    "kind": kind,
                    "archive": archived.display().to_string(),
                    "name": archived.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
                    "bytes": meta.len(),
                }));
            }
            removed += prune_archives(&dir, keep);
        }

        let reopened = if rotated.is_empty() {
            false
        } else {
            reopen_nginx()
        };
        Ok(Response::ok(
            "ok",
            Some(json!({
                "rotated": rotated,
                "removed": removed,
                "reopened": reopened,
                "errors": errors,
            })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// site.log_list：列出当前日志与归档
pub async fn list(log_root: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let dir = safe_log_root(&log_root)?;
        let mut current: Vec<serde_json::Value> = Vec::new();
        for kind in ["access", "error", "waf"] {
            let p = current_log(&dir, kind);
            if std::fs::metadata(&p).is_ok() {
                current.push(file_json(&p, kind, "current", ""));
            }
        }
        let mut archives: Vec<serde_json::Value> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut items: Vec<(String, serde_json::Value)> = Vec::new();
            for e in rd.flatten() {
                let path = e.path();
                if !path.is_file() {
                    continue;
                }
                let name = e.file_name().to_string_lossy().to_string();
                let Some(date) = archive_date(&name) else {
                    continue;
                };
                let kind = if name.starts_with("error.log-") {
                    "error"
                } else if name.starts_with("waf.log-") {
                    "waf"
                } else {
                    "access"
                };
                items.push((name.clone(), file_json(&path, kind, "archive", &date)));
            }
            items.sort_by(|a, b| b.0.cmp(&a.0)); // 名称倒序 = 日期倒序
            archives = items.into_iter().map(|(_, v)| v).collect();
        }
        Ok(Response::ok(
            "ok",
            Some(json!({ "current": current, "archives": archives })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

fn file_json(p: &Path, kind: &str, scope: &str, date: &str) -> serde_json::Value {
    let meta = std::fs::metadata(p).ok();
    json!({
        "name": p.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
        "kind": kind,
        "scope": scope,
        "date": date,
        "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
        "mtime": meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0),
    })
}

/// site.log_read：读取当前日志或归档的尾部行
pub async fn read(
    log_root: String,
    kind: String,
    archive: String,
    lines: usize,
    keyword: String,
    status: String,
) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let dir = safe_log_root(&log_root)?;
        let kind = kind_of(&kind);
        let want = lines.clamp(1, MAX_LINES);
        let (text, path) = if archive.trim().is_empty() {
            let p = current_log(&dir, kind);
            if !p.exists() {
                return Ok(Response::ok(
                    "日志尚未生成",
                    Some(json!({ "path": p.display().to_string(), "lines": [], "count": 0 })),
                ));
            }
            (tail_text(&p)?, p)
        } else {
            let p = archive_path(&dir, &archive)?;
            (archived_text(&p)?, p)
        };
        let picked = pick_lines(&text, want, &keyword, &status);
        Ok(Response::ok(
            "ok",
            Some(json!({
                "path": path.display().to_string(),
                "lines": picked,
                "count": picked.len(),
                "size": std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// site.log_clear：清空当前日志（truncate）并 reopen
pub async fn clear(log_root: String, kind: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let dir = safe_log_root(&log_root)?;
        let kinds: Vec<&str> = if kind.trim().is_empty() {
            vec!["access", "error", "waf"]
        } else {
            vec![kind_of(&kind)]
        };
        let mut cleared: Vec<String> = Vec::new();
        for k in kinds {
            let p = current_log(&dir, k);
            if !p.is_file() {
                continue;
            }
            std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&p)
                .map_err(|e| format!("清空 {k}.log 失败: {e}"))?;
            cleared.push(format!("{k}.log"));
        }
        let reopened = if cleared.is_empty() {
            false
        } else {
            reopen_nginx()
        };
        Ok(Response::ok(
            "ok",
            Some(json!({ "cleared": cleared, "reopened": reopened })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_date_is_parsed() {
        assert_eq!(archive_date("access.log-20260915.gz").unwrap(), "20260915");
        assert_eq!(archive_date("error.log-20260915").unwrap(), "20260915");
        assert_eq!(
            archive_date("access.log-20260915-1.gz").unwrap(),
            "20260915"
        );
        assert!(archive_date("access.log").is_none());
    }

    #[test]
    fn status_token_is_extracted() {
        let line = r#"1.2.3.4 - - [15/Sep/2026:10:00:00 +0800] "GET /a HTTP/1.1" 404 12 "-" "UA""#;
        assert_eq!(status_of(line), "404");
    }

    #[test]
    fn tail_pick_keeps_order_and_filters() {
        let text = "l1\nl2 404\nl3\nl4 404\n";
        let all = pick_lines(text, 10, "", "");
        assert_eq!(all, vec!["l1", "l2 404", "l3", "l4 404"]);
        let last2 = pick_lines(text, 2, "", "");
        assert_eq!(last2, vec!["l3", "l4 404"]);
        let kw = pick_lines(text, 10, "404", "");
        assert_eq!(kw, vec!["l2 404", "l4 404"]);
    }
}
