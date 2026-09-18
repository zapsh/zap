//! 受限的文件系统只读操作（root 执行）。
//!
//! - `browse_dirs`：列出某个目录下的直接子目录（供站点「选择已有站点目录」）；
//! - `disk_usage`：批量统计目录占用字节数（面板自身以 zapadm 运行，
//!   用户家目录 0700 读不到，磁盘用量必须 root 代跑）。
//!
//! 安全边界：只读、不返回文件内容，路径须为绝对路径且不含 `..`。

use std::collections::HashMap;
use std::path::Path;

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

/// 批量统计目录占用字节数（root）：`du -sb`，不支持 `-b` 时回退 `du -sk` × 1024。
///
/// 返回 `{ usage: { 目录: 字节数 } }`：不存在的目录不会出现在结果中；
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
            return Ok(Response::ok("OK", Some(json!({ "usage": {} }))));
        }
        let mut usage = du_map("-sb", &wanted, 1);
        if usage.is_empty() {
            // busybox 等不支持 -b：按 1KB 块换算
            usage = du_map("-sk", &wanted, 1024);
        }
        if usage.is_empty() {
            return Err("du 执行失败或没有任何输出".to_string());
        }
        Ok(Response::ok("OK", Some(json!({ "usage": usage }))))
    })
    .await
    .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
    .unwrap_or_else(|e| Response::err(-1, e))
}

/// 执行 `du {flag} -- {paths}` 并把输出解析为「目录 → 字节数」（`scale` 为块→字节换算系数）。
fn du_map(flag: &str, paths: &[String], scale: u64) -> HashMap<String, u64> {
    let Ok(out) = root_cmd("du").arg(flag).arg("--").args(paths).output() else {
        return HashMap::new();
    };
    parse_du_out(&String::from_utf8_lossy(&out.stdout), scale)
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
}
