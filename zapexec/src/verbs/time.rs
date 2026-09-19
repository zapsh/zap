use std::path::Path;

use serde_json::json;

use crate::verbs::root_cmd;
use zap_proto::Response;

pub async fn sync() -> Response {
    tokio::task::spawn_blocking(|| {
        let chrony_ok = root_cmd("chronyc")
            .args(["-a", "makestep"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if chrony_ok {
            return Response::ok("已通过 chrony 同步时间", None);
        }

        match root_cmd("ntpdate").args(["pool.ntp.org"]).output() {
            Ok(o) if o.status.success() => Response::ok("已通过 ntpdate 同步时间", None),
            Ok(o) => Response::err(
                -1,
                format!("ntpdate 失败: {}", String::from_utf8_lossy(&o.stderr)),
            ),
            Err(e) => Response::err(-1, format!("未找到时间同步工具: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn set_timezone(timezone: &str) -> Response {
    if timezone.is_empty() {
        return Response::err(-1, "时区不能为空");
    }
    let tz = timezone.to_string();
    tokio::task::spawn_blocking(move || {
        match root_cmd("timedatectl").args(["set-timezone", &tz]).output() {
            Ok(o) if o.status.success() => {
                // 多数发行版不会同步改写 /etc/timezone，补一次，避免面板/其他工具读到旧值
                let _ = write_etc_timezone(&tz);
                Response::ok("时区设置成功", None)
            }
            // 没有 systemd（容器等）时退化为直接改写 /etc/localtime
            Ok(o) => {
                let detail = String::from_utf8_lossy(&o.stderr).trim().to_string();
                match apply_timezone_files(&tz) {
                    Ok(()) => Response::ok("时区设置成功", None),
                    Err(e) => Response::err(
                        -1,
                        format!("时区设置失败: {detail}；直接写入时区文件也失败: {e}"),
                    ),
                }
            }
            Err(e) => match apply_timezone_files(&tz) {
                Ok(()) => Response::ok("时区设置成功", None),
                Err(e2) => Response::err(
                    -1,
                    format!("未找到 timedatectl（{e}），直接写入时区文件也失败: {e2}"),
                ),
            },
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 无 systemd 时的兜底：把 /etc/localtime 指向 zoneinfo 里对应的时区文件。
fn apply_timezone_files(tz: &str) -> Result<(), String> {
    let zone = Path::new("/usr/share/zoneinfo").join(tz);
    if !zone.exists() {
        return Err(format!("时区文件不存在: {}", zone.display()));
    }
    // 先建临时软链再 rename，避免中途失败留下坏链接
    let tmp = Path::new("/etc/localtime.zap.tmp");
    let _ = std::fs::remove_file(tmp);
    std::os::unix::fs::symlink(&zone, tmp).map_err(|e| format!("创建软链失败: {e}"))?;
    if let Err(e) = std::fs::rename(tmp, "/etc/localtime") {
        let _ = std::fs::remove_file(tmp);
        return Err(format!("替换 /etc/localtime 失败: {e}"));
    }
    let _ = write_etc_timezone(tz);
    Ok(())
}

fn write_etc_timezone(tz: &str) -> std::io::Result<()> {
    std::fs::write("/etc/timezone", format!("{tz}\n"))
}

pub async fn list_timezones() -> Response {
    tokio::task::spawn_blocking(|| {
        match root_cmd("timedatectl").args(["list-timezones"]).output() {
            Ok(o) if o.status.success() => {
                let zones: Vec<String> = String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                Response::ok("ok", Some(json!(zones)))
            }
            Ok(o) => Response::err(-1, String::from_utf8_lossy(&o.stderr).to_string()),
            Err(e) => Response::err(-1, format!("命令执行失败: {e}")),
        }
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

pub async fn get() -> Response {
    tokio::task::spawn_blocking(|| {
        let now = chrono::Local::now();
        let tz = current_timezone();
        Response::ok(
            "ok",
            Some(json!({
                "datetime": now.format("%Y-%m-%d %H:%M:%S").to_string(),
                "timestamp": now.timestamp(),
                "timezone": tz,
                "timezone_offset": now.offset().to_string(),
            })),
        )
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 当前时区名。
///
/// 顺序不能反：面板走的是 `timedatectl set-timezone`，它只保证 /etc/localtime 正确，
/// 而 /etc/timezone 在多数发行版不会被同步改写（本机就仍写着 Etc/UTC），
/// 先读它的话改完时区面板会一直显示旧值。
fn current_timezone() -> String {
    if let Some(tz) = timedatectl_timezone() {
        return tz;
    }
    if let Some(tz) = localtime_timezone() {
        return tz;
    }
    if let Ok(raw) = std::fs::read_to_string("/etc/timezone") {
        let tz = raw.trim();
        if !tz.is_empty() {
            return tz.to_string();
        }
    }
    "Unknown".to_string()
}

fn timedatectl_timezone() -> Option<String> {
    let o = root_cmd("timedatectl")
        .args(["show", "--property=Timezone", "--value"])
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let tz = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!tz.is_empty()).then_some(tz)
}

/// /etc/localtime 通常是 zoneinfo 下某个时区文件的软链，取其相对路径即为时区名。
fn localtime_timezone() -> Option<String> {
    let link = std::fs::read_link("/etc/localtime").ok()?;
    zoneinfo_name(&link.to_string_lossy())
}

fn zoneinfo_name(path: &str) -> Option<String> {
    let tz = path
        .replace('\\', "/")
        .split_once("zoneinfo/")?
        .1
        .trim()
        .to_string();
    (!tz.is_empty()).then_some(tz)
}

#[cfg(test)]
mod tests {
    use super::zoneinfo_name;

    #[test]
    fn zoneinfo_name_from_abs_link() {
        assert_eq!(
            zoneinfo_name("/usr/share/zoneinfo/Asia/Shanghai").as_deref(),
            Some("Asia/Shanghai")
        );
    }

    #[test]
    fn zoneinfo_name_from_relative_link() {
        assert_eq!(
            zoneinfo_name("../usr/share/zoneinfo/Etc/UTC").as_deref(),
            Some("Etc/UTC")
        );
    }

    #[test]
    fn zoneinfo_name_of_plain_file() {
        assert_eq!(zoneinfo_name("/etc/localtime"), None);
    }
}
