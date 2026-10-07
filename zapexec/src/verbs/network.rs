// SPDX-License-Identifier: AGPL-3.0-only
use std::net::IpAddr;
use std::path::Path;

use serde_json::json;

use crate::verbs::root_cmd;
use zap_proto::Response;

const RESOLV_CONF: &str = "/etc/resolv.conf";

/// 主机名是否合法（仅字母数字、点、连字符、下划线，最长 253）
fn valid_hostname(name: &str) -> bool {
    if name.is_empty() || name.len() > 253 {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// 域名搜索项合法性（宽松校验，不含空白即可）
fn valid_search(s: &str) -> bool {
    !s.is_empty() && s.len() <= 253 && !s.chars().any(|c| c.is_whitespace())
}

fn read_first_line(path: &str) -> String {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
        .unwrap_or_default()
}

/// 读取 machine-info 中的 PrettyHostname / IconName 等键值
fn machine_info_value(key: &str) -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/machine-info") {
        for line in content.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix(key)
                && let Some(value) = rest.strip_prefix('=')
            {
                return value.trim().trim_matches('"').trim().to_string();
            }
        }
    }
    String::new()
}

/// 读取 /etc/resolv.conf 的 nameserver / search（跟随符号链接）
fn read_resolv_conf() -> (Vec<String>, Vec<String>, Option<String>) {
    let mut nameservers = Vec::new();
    let mut search = Vec::new();
    if let Ok(content) = std::fs::read_to_string(RESOLV_CONF) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("nameserver") {
                let ns = rest.trim();
                if !ns.is_empty() && ns.parse::<IpAddr>().is_ok() {
                    nameservers.push(ns.to_string());
                }
            } else if let Some(rest) = line.strip_prefix("search") {
                search.extend(rest.split_whitespace().map(|s| s.to_string()));
            }
        }
    }
    // 是否是符号链接（systemd-resolved 等管理）
    let symlink_target = std::fs::read_link(RESOLV_CONF)
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    (nameservers, search, symlink_target)
}

pub async fn get() -> Response {
    tokio::task::spawn_blocking(|| {
        let hostname = read_first_line("/proc/sys/kernel/hostname");
        let static_hostname = read_first_line("/etc/hostname");
        let pretty_hostname = machine_info_value("PrettyHostname");
        let icon_name = machine_info_value("IconName");
        let (nameservers, search, symlink_target) = read_resolv_conf();
        let managed = symlink_target
            .as_deref()
            .is_some_and(|t| t.contains("/run/systemd/resolve/"));
        Response::ok(
            "ok",
            Some(json!({
                "hostname": hostname,
                "static_hostname": static_hostname,
                "pretty_hostname": pretty_hostname,
                "icon_name": icon_name,
                "resolv": {
                    "nameservers": nameservers,
                    "search": search,
                    "symlink_target": symlink_target,
                    "managed": managed,
                }
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn set_hostname(hostname: &str) -> Response {
    if !valid_hostname(hostname) {
        return Response::err(
            -1,
            "主机名不合法：仅允许字母、数字、点、连字符、下划线，最长 253 个字符",
        );
    }
    let name = hostname.to_string();
    tokio::task::spawn_blocking(move || {
        // 1) 优先 hostnamectl
        match root_cmd("hostnamectl")
            .args(["set-hostname", &name])
            .output()
        {
            Ok(o) if o.status.success() => {
                return Response::ok("主机名设置成功", None);
            }
            _ => {}
        }
        // 2) fallback：直接写 /etc/hostname 并刷新内核参数
        if std::fs::write("/etc/hostname", format!("{name}\n")).is_err() {
            return Response::err(-1, "写入 /etc/hostname 失败");
        }
        let _ = root_cmd("sysctl")
            .args(["-w", &format!("kernel.hostname={name}")])
            .output();
        Response::ok(
            "主机名设置成功（无 hostnamectl，已直接修改 /etc/hostname）",
            None,
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn set_resolver(nameservers: &[String], search: &[String]) -> Response {
    if nameservers.is_empty() {
        return Response::err(-1, "至少需要一个 nameserver");
    }
    let mut clean_ns = Vec::new();
    for ns in nameservers {
        let ns = ns.trim().to_string();
        if ns.parse::<IpAddr>().is_err() {
            return Response::err(-1, format!("无效的 nameserver 地址: {ns}"));
        }
        clean_ns.push(ns);
    }
    let clean_search: Vec<String> = search
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    for s in &clean_search {
        if !valid_search(s) {
            return Response::err(-1, format!("无效的 search 域名: {s}"));
        }
    }

    tokio::task::spawn_blocking(move || {
        // 备份旧文件（若存在且不是链接）
        if Path::new(RESOLV_CONF).exists()
            && std::fs::read_link(RESOLV_CONF).is_err()
            && std::fs::copy(RESOLV_CONF, format!("{RESOLV_CONF}.zap.bak")).is_err()
        {
            // 备份失败不致命，继续
        }

        let mut content = String::from("# Managed by zap 网络设置\n");
        if !clean_search.is_empty() {
            content.push_str(&format!("search {}\n", clean_search.join(" ")));
        }
        for ns in &clean_ns {
            content.push_str(&format!("nameserver {ns}\n"));
        }

        // 若原文件是符号链接（systemd-resolved），先移除再写入普通文件
        if std::fs::symlink_metadata(RESOLV_CONF).is_ok()
            && std::fs::read_link(RESOLV_CONF).is_ok()
            && std::fs::remove_file(RESOLV_CONF).is_err()
        {
            return Response::err(-1, "移除旧的 resolv.conf 符号链接失败");
        }

        use std::io::Write;
        let tmp = "/etc/resolv.conf.zap.tmp";
        let result = (|| -> std::io::Result<()> {
            let mut f = std::fs::File::create(tmp)?;
            f.write_all(content.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(tmp, RESOLV_CONF)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(RESOLV_CONF, std::fs::Permissions::from_mode(0o644))?;
            Ok(())
        })();

        match result {
            Ok(()) => Response::ok("DNS 解析器设置成功", None),
            Err(e) => {
                let _ = std::fs::remove_file(tmp);
                Response::err(-1, format!("写入 {RESOLV_CONF} 失败: {e}"))
            }
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}
