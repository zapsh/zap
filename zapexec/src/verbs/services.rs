//! 服务总览（root 执行）：「服务配置 → 总览」页的数据源。
//!
//! 只认「应用商店安装、且安装脚本在 `info.yaml` 里登记了 `svc_name`」的应用实例 ——
//! 只有这类实例能在面板里被启停、管理开机自启。unit 名不在这里硬编码：各发行版
//! 叫法不同（php-fpm-8.3 / mysql / nginx），由安装脚本自己登记。
//!
//! 端点：
//! - `services.overview`  服务卡片列表（运行状态 / 开机自启 / 版本）
//! - `services.control`   启停：start / stop / restart / reload
//! - `services.boot`      开机自启开关（systemctl enable / disable）
//!
//! 安全边界：动作只作用于 overview 扫描得到的 unit 集合，不接受任意 systemd unit。

use std::collections::BTreeSet;

use serde_json::{Value, json};
use zap_proto::Response;

use super::appstore;
use super::svc;

/// 允许的动作（与 systemd 一一对应，不含 enable/disable —— 那是 boot 的事）。
const ALLOWED_ACTIONS: &[&str] = &["start", "stop", "restart", "reload"];

/// 已登记的服务 unit 集合：扫一遍应用商店已装应用的 `info.yaml`。
///
/// 每次动作前重算（目录扫描，成本可忽略），避免缓存过期后放过已卸载应用的 unit。
fn known_units() -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for slot in appstore::scan_slots() {
        let Some(info) = appstore::read_info_yaml(&slot.dir) else {
            continue;
        };
        if let Some(s) = info.get("svc_name").and_then(|v| v.as_str()) {
            if !s.trim().is_empty() {
                set.insert(s.trim().to_string());
            }
        }
    }
    set
}

/// 白名单校验：unit 必须是某个已装应用登记的，否则拒绝。
fn validate_unit(svc: &str) -> Result<String, String> {
    if svc.is_empty() {
        return Err("服务名为空".to_string());
    }
    if !known_units().contains(svc) {
        return Err(format!(
            "服务 {svc} 不在已安装应用登记的服务列表内，已拒绝该操作"
        ));
    }
    Ok(svc.to_string())
}

fn str_of<'a>(v: &'a Value, k: &'a str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

/// 服务卡片列表：一张卡 = 一个登记了服务的已装应用实例。
pub async fn overview() -> Response {
    let items = tokio::task::spawn_blocking(|| {
        let mut items: Vec<Value> = Vec::new();
        for slot in appstore::scan_slots() {
            let Ok(meta) = appstore::read_meta(&slot.dir) else {
                continue;
            };
            let Some(info) = appstore::read_info_yaml(&slot.dir) else {
                continue;
            };
            // 没登记 svc_name 的应用（composer / webapps …）不进服务页
            let Some(svc) = info.get("svc_name").and_then(|v| v.as_str()) else {
                continue;
            };
            let svc = svc.trim();
            if svc.is_empty() {
                continue;
            }
            items.push(json!({
                "key": slot.key(),
                "pkg": slot.name,
                "name": meta.name,
                "version": meta.version,
                "category": meta.category,
                "instance": slot.instance,
                "svc": svc,
                "state": appstore::normalize_state(&svc::raw_state(svc)),
                "enabled": svc::is_enabled(svc),
                "exists": svc::exists(svc),
            }));
        }
        items.sort_by(|a, b| {
            str_of(a, "category")
                .cmp(str_of(b, "category"))
                .then_with(|| str_of(a, "name").cmp(str_of(b, "name")))
                .then_with(|| str_of(a, "instance").cmp(str_of(b, "instance")))
        });
        items
    })
    .await;

    match items {
        Ok(items) => Response::ok(
            "ok",
            Some(json!({
                "items": items,
                // 平台不支持 systemd 时前端整页降级为「需手动操作」
                "systemd": svc::supported(),
            })),
        ),
        Err(e) => Response::err(-1, format!("扫描已安装应用失败: {e}")),
    }
}

/// 启停服务：action ∈ start / stop / restart / reload。
pub async fn control(svc_name: &str, action: &str) -> Response {
    if !ALLOWED_ACTIONS.contains(&action) {
        return Response::err(-1, format!("不支持的服务动作：{action}"));
    }
    let Ok(unit) = validate_unit(svc_name) else {
        return Response::err(
            -1,
            format!("服务 {svc_name} 不在已安装应用登记的服务列表内，已拒绝该操作"),
        );
    };
    let action = action.to_string();
    let done = tokio::task::spawn_blocking(move || svc::act(&action, &unit)).await;
    match done {
        Ok(Ok(())) => Response::ok("ok", Some(json!({ "state": svc::raw_state(svc_name) }))),
        Ok(Err(e)) => Response::err(-1, e),
        Err(e) => Response::err(-1, format!("任务执行失败: {e}")),
    }
}

/// 开机自启：`enable` → `systemctl enable`，否则 `disable`。
pub async fn boot(svc_name: &str, enable: bool) -> Response {
    let Ok(unit) = validate_unit(svc_name) else {
        return Response::err(
            -1,
            format!("服务 {svc_name} 不在已安装应用登记的服务列表内，已拒绝该操作"),
        );
    };
    let action = if enable { "enable" } else { "disable" };
    let done = tokio::task::spawn_blocking(move || svc::act(action, &unit)).await;
    match done {
        Ok(Ok(())) => Response::ok(
            "ok",
            Some(json!({ "enabled": svc::is_enabled(svc_name) })),
        ),
        Ok(Err(e)) => Response::err(-1, e),
        Err(e) => Response::err(-1, format!("任务执行失败: {e}")),
    }
}
