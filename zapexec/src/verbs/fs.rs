//! 受限的文件系统只读操作（root 执行）。
//!
//! - `browse_dirs`：列出某个目录下的直接子目录（供站点「选择已有站点目录」）；
//! - `disk_usage`：批量统计目录占用字节数（面板自身以 zapadm 运行，
//!   用户家目录 0700 读不到，磁盘用量必须 root 代跑）；
//!   `du` 不可用时由 root 进程内递归统计兜底。
//!
//! 安全边界：只读、不返回文件内容，路径须为绝对路径且不含 `..`。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::json;
use zap_proto::Response;

use super::root_cmd;

/// 单批最多统计的目录数（命令行长度与单次 du 耗时的上限）
const DISK_USAGE_MAX: usize = 256;

/// 列出 `base` 下的直接子目录名（字典序；不含点目录；跳过符号链接目录）。
/// base 不存在 / 非目录时返回错误。
pub async fn browse_dirs(base: String) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        if !base.starts_with('/') {
            return Err("浏览路径必须是绝对路径".to_string());
        }
        if base.split('/').any(|s| s == "..") {
            return Err("浏览路径不允许包含 ..".to_string());
        }
        let p = Path::new(&base);
        if !p.is_dir() {
            return Err(format!("目录不存在或不是目录：{base}"));
        }
        let mut dirs: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(p)
            .map_err(|e| format!("读取目录失败：{e}"))?
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let ft = entry
                .file_type()
                .map_err(|e| format!("读取目录项类型失败：{e}"))?;
            // 只列目录；符号链接一律跳过（避免被链到站外目录的目录误导/越权浏览）
            if ft.is_dir() {
                dirs.push(name);
            }
        }
        dirs.sort();
        Ok(Response::ok(
            "OK",
            Some(json!({ "base": base, "dirs": dirs })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 批量统计目录占用字节数（root）：`du -sb` 优先，不支持 `-b` 回退 `du -sk` × 1024，
/// 两者都不可用时由 root 进程内递归统计兜底（不依赖外部命令）。
///
/// 返回 `{ usage: { 目录: 字节数 }, missing: [不存在的目录], error: 说明 }`。
/// **目录不存在属正常情况**（站点尚未创建 / 已删除）：不算失败，这类目录进
/// `missing`（调用方按 0 计），`error` 给出 du 的首行提示。
/// 存在性只有 root 判得准 —— 面板（zapadm）stat 不到 0700 家目录，不能自行判断。
/// 非法路径（非绝对 / 含 `..`）直接丢弃，超出 `DISK_USAGE_MAX` 的部分截断。
pub async fn disk_usage(paths: Vec<String>) -> Response {
    tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let mut wanted: Vec<String> = Vec::new();
        for p in paths {
            if !p.starts_with('/') || p.split('/').any(|s| s == "..") {
                continue;
            }
            if !wanted.contains(&p) {
                wanted.push(p);
            }
        }
        wanted.truncate(DISK_USAGE_MAX);
        if wanted.is_empty() {
            return Ok(Response::ok(
                "OK",
                Some(json!({ "usage": {}, "missing": [], "error": "" })),
            ));
        }
        let (mut usage, mut err) = du_try("-sb", &wanted, 1);
        if usage.is_empty() {
            // busybox 等不支持 -b：按 1KB 块换算
            let (u2, e2) = du_try("-sk", &wanted, 1024);
            if !u2.is_empty() {
                usage = u2;
                err = String::new();
            } else if !e2.is_empty() {
                err = e2;
            }
        }
        if usage.is_empty() {
            // du 彻底不可用（缺命令 / 参数不兼容）：root 进程内递归统计
            usage = walk_usage(&wanted);
            if !usage.is_empty() {
                err = String::new();
            }
        }
        // du 没统计到、root 也 stat 不到的目录 = 确实不存在（按 0 计）；
        // 存在但 du 报错的不算 missing，留给调用方决定是否保留上次结果
        let missing: Vec<String> = wanted
            .iter()
            .filter(|p| !usage.contains_key(*p) && !Path::new(p).exists())
            .cloned()
            .collect();
        Ok(Response::ok(
            "OK",
            Some(json!({ "usage": usage, "missing": missing, "error": err })),
        ))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 执行 `du {flag} -- {paths}`：返回（「目录 → 字节数」，du 报错首行）。
/// `scale` 为块→字节换算系数；spawn 失败时 `error` 给出原因。
fn du_try(flag: &str, paths: &[String], scale: u64) -> (HashMap<String, u64>, String) {
    match root_cmd("du").arg(flag).arg("--").args(paths).output() {
        Ok(out) => (
            parse_du_out(&String::from_utf8_lossy(&out.stdout), scale),
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string(),
        ),
        Err(e) => (HashMap::new(), format!("无法执行 du: {e}")),
    }
}

/// root 进程内递归统计目录占用（du 不可用时的兜底）：累加 `st_blocks × 512`，
/// 与 `du -sk` 同口径（磁盘实际占用）。不跟随符号链接，按 `(dev, ino)` 去重防循环；
/// 目录不存在（一个条目都 stat 不到）返回 `None`。
fn walk_usage(paths: &[String]) -> HashMap<String, u64> {
    let mut m = HashMap::new();
    for p in paths {
        if let Some(v) = walk_bytes(Path::new(p)) {
            m.insert(p.clone(), v);
        }
    }
    m
}

fn walk_bytes(root: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;

    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut total: u64 = 0;
    let mut any = false;
    while let Some(p) = stack.pop() {
        let Ok(md) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        total = total.saturating_add(md.blocks().saturating_mul(512));
        any = true;
        if md.file_type().is_symlink() || !md.is_dir() {
            continue;
        }
        if !seen.insert((md.dev(), md.ino())) {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&p) {
            for e in rd.flatten() {
                stack.push(e.path());
            }
        }
    }
    any.then_some(total)
}

/// 解析 `du` 输出（`{字节数}\t{路径}` 逐行）：路径中的空格不影响切分（按首个制表符切）。
fn parse_du_out(stdout: &str, scale: u64) -> HashMap<String, u64> {
    let mut m = HashMap::new();
    for line in stdout.lines() {
        let mut it = line.splitn(2, '\t');
        let (Some(n), Some(p)) = (it.next(), it.next()) else {
            continue;
        };
        if let Ok(v) = n.trim().parse::<u64>() {
            m.insert(p.to_string(), v.saturating_mul(scale));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_du_output() {
        let out = "4096\t/home/alice\n8192\t/home/bob\n";
        let m = parse_du_out(out, 1);
        assert_eq!(m.get("/home/alice"), Some(&4096));
        assert_eq!(m.get("/home/bob"), Some(&8192));

        // du -sk：块数 × 1024
        let m2 = parse_du_out("4\t/home/alice\n", 1024);
        assert_eq!(m2.get("/home/alice"), Some(&4096));

        // 路径含空格 / 异常行
        let m3 = parse_du_out("12\t/home/a b\ngarbage\n", 1);
        assert_eq!(m3.get("/home/a b"), Some(&12));
        assert_eq!(m3.len(), 1);
    }

    /// 目录不存在：不算失败（code 0），进 `missing` 供调用方按 0 计
    /// —— 站点尚未创建时每轮都会遇到，不能刷 WARN
    #[tokio::test]
    async fn disk_usage_missing_dir_is_not_error() {
        let dir = "/nonexistent-zap-du-test".to_string();
        let r = disk_usage(vec![dir.clone()]).await;
        assert_eq!(r.code, 0, "message={}", r.message);
        let empty = r
            .data
            .as_ref()
            .and_then(|d| d.get("usage"))
            .and_then(|u| u.as_object())
            .map(|o| o.is_empty())
            .unwrap_or(false);
        assert!(empty);
        // 明确告知「不存在」，调用方据此写 0
        let missing: Vec<String> = serde_json::from_value(
            r.data
                .as_ref()
                .and_then(|d| d.get("missing"))
                .cloned()
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        assert_eq!(missing, vec![dir]);
    }

    /// 真实目录能统计出用量（du 或进程内兜底任一命中）
    #[tokio::test]
    async fn disk_usage_real_dir() {
        let base = std::env::temp_dir().join(format!("zap-du-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("f.txt"), vec![0u8; 1024]).unwrap();

        let dir = base.to_string_lossy().to_string();
        let r = disk_usage(vec![dir.clone()]).await;
        assert_eq!(r.code, 0, "message={}", r.message);
        let v = r
            .data
            .as_ref()
            .and_then(|d| d.get("usage"))
            .and_then(|u| u.get(&dir))
            .and_then(|v| v.as_u64());
        assert!(v.unwrap_or(0) > 0);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// du 不可用时的兜底：能统计出真实目录，不存在的目录直接缺席（不算失败）
    #[test]
    fn walks_dirs_without_du() {
        let base = std::env::temp_dir().join(format!("zap-du-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("sub").join("a.txt"), vec![0u8; 4096]).unwrap();

        let dir = base.to_string_lossy().to_string();
        let missing = base.join("nope").to_string_lossy().to_string();
        let m = walk_usage(&[dir.clone(), missing.clone()]);
        assert!(m.get(&dir).copied().unwrap_or(0) > 0);
        assert!(!m.contains_key(&missing));

        let _ = std::fs::remove_dir_all(&base);
    }
}
