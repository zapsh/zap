use serde_json::json;

use zap_proto::Response;

use super::svc;

const ALLOWED_ACTIONS: &[&str] = &["start", "stop", "restart", "reload", "enable", "disable"];

/// 列出系统所有服务（unit / rc.d daemon）
pub async fn list() -> Response {
    tokio::task::spawn_blocking(|| match svc::list() {
        Ok(rows) => {
            let services: Vec<_> = rows
                .into_iter()
                .map(|r| {
                    json!({
                        "name": r.name,
                        "load": r.load,
                        "active": r.active,
                        "sub": r.sub,
                        "description": r.description,
                    })
                })
                .collect();
            Response::ok("ok", Some(json!({ "services": services })))
        }
        Err(e) => Response::err(-1, e),
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}

/// 对服务执行 start/stop/restart/reload/enable/disable
pub async fn action(name: &str, action: &str) -> Response {
    if !ALLOWED_ACTIONS.contains(&action) {
        return Response::err(-1, format!("不支持的操作: {action}"));
    }
    let name = name.to_string();
    let action = action.to_string();
    tokio::task::spawn_blocking(move || match svc::act(&action, &name) {
        Ok(()) => {
            // 读取操作后的实际状态
            let status = svc::raw_state(&name);
            Response::ok(
                "ok",
                Some(json!({ "name": name, "action": action, "status": status })),
            )
        }
        Err(msg) => Response::err(-1, format!("操作失败: {msg}")),
    })
    .await
    .unwrap_or_else(|e| Response::err(-1, format!("任务执行失败: {e}")))
}
