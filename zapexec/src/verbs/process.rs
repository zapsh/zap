// SPDX-License-Identifier: AGPL-3.0-only
use serde_json::json;

use crate::verbs::root_cmd;
use zap_proto::Response;

/// 需要保护的进程 PID（避免误杀导致系统/自身服务不可用）
fn protected_pids() -> Vec<u32> {
    let mut pids = vec![0, 1];
    // 自身进程（zapexec）
    pids.push(std::process::id());
    // 父进程（zapd 常驻进程）
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("PPid:") {
                if let Ok(ppid) = rest.trim().parse::<u32>() {
                    pids.push(ppid);
                }
                break;
            }
        }
    }
    pids
}

/// 列出系统进程（按 CPU 占用降序）
pub async fn list() -> Response {
    tokio::task::spawn_blocking(|| {
        let out = root_cmd("ps")
            .args([
                "-eo",
                "pid,user,pcpu,pmem,stat,etime,args",
                "--sort=-pcpu",
                "--no-headers",
            ])
            .output();
        match out {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                let processes: Vec<_> = text
                    .lines()
                    .filter_map(|line| {
                        // ps 输出各列以不定数量的空格填充（如 "  28333 root     10.2 ..."），
                        // split_whitespace 会折叠连续空白，避免解析出空字段
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() < 6 {
                            return None;
                        }
                        let pid: u32 = parts[0].parse().ok()?;
                        let user = parts[1].to_string();
                        let pcpu = parts[2].to_string();
                        let pmem = parts[3].to_string();
                        let stat = parts[4].to_string();
                        let etime = parts[5].to_string();
                        let cmdline = parts[6..].join(" ");
                        Some(json!({
                            "pid": pid,
                            "user": user,
                            "pcpu": pcpu,
                            "pmem": pmem,
                            "stat": stat,
                            "etime": etime,
                            "cmd": cmdline,
                        }))
                    })
                    .collect();
                Response::ok("ok", Some(json!({ "processes": processes })))
            }
            Ok(o) => Response::err(
                -1,
                format!(
                    "获取进程列表失败: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
            ),
            Err(e) => Response::err(-1, format!("命令执行失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 终止进程。signal 缺省为 TERM，传 "9" 强制终止（KILL）
pub async fn kill(pid: u32, signal: Option<String>) -> Response {
    tokio::task::spawn_blocking(move || {
        if pid < 2 || protected_pids().contains(&pid) {
            return Response::err(-1, format!("不允许终止受保护进程 (pid={pid})"));
        }
        let sig = signal.unwrap_or_else(|| "TERM".to_string());
        let out = root_cmd("kill")
            .args([format!("-{sig}"), pid.to_string()])
            .output();
        match out {
            Ok(o) if o.status.success() => {
                Response::ok("ok", Some(json!({ "pid": pid, "signal": sig })))
            }
            Ok(o) => Response::err(
                -1,
                format!(
                    "终止进程 {pid} 失败: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
            ),
            Err(e) => Response::err(-1, format!("命令执行失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}
