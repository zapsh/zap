//! 守护进程实时事件流（`docker events` 的等价物）。
//!
//! 走 Engine API 的 `/events` 长连接而不是起一个 `docker events` 子进程：
//! - 不需要在 zapexec 里常驻进程，客户端断开即结束
//! - 拿到的是结构化字段（类型 / 动作 / 对象 / 属性），前端不必再解析自由文本
//!
//! 生命周期：
//! - `stream.next()` 结束 → daemon 关闭了事件通道
//! - 客户端断开 → 写帧失败 / 连接关闭，本任务随连接一起收尾

use bollard::models::EventMessage;
use bollard::query_parameters::EventsOptions;
use futures_util::StreamExt;
use serde_json::{Value, json};

use super::docker::client;
use crate::stream::StreamSink;

/// 单条事件流的兜底上限：前端离开页面会断开连接，这里只防止忘记关闭时一直挂着。
const MAX_DURATION: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// 推流守护事件，直到 daemon 关闭通道或客户端断开。
pub async fn run(sink: StreamSink) {
    let d = match client() {
        Ok(d) => d,
        Err(e) => {
            sink.error(e).await;
            return;
        }
    };

    // 不给 `since`：只推送连接建立之后的事件（与 `docker events` 默认行为一致）
    let mut stream = d.events(None::<EventsOptions>);

    if !sink.ready().await {
        return;
    }

    loop {
        tokio::select! {
            item = stream.next() => match item {
                Some(Ok(ev)) => {
                    let payload = serde_json::to_vec(&to_json(ev)).unwrap_or_default();
                    if !sink.out(&payload).await {
                        break;
                    }
                }
                Some(Err(e)) => {
                    sink.error(format!("读取事件流失败: {e}")).await;
                    return;
                }
                None => break,
            },
            _ = tokio::time::sleep(MAX_DURATION) => {
                sink.error("事件流已达时长上限，已断开".to_string()).await;
                return;
            }
        }
    }

    sink.end(0).await;
}

/// 摊平成前端好用的形状：对象的名字落在 `Attributes.name` 里，单独提出来。
fn to_json(ev: EventMessage) -> Value {
    let (id, attrs) = match ev.actor {
        Some(a) => (a.id.unwrap_or_default(), a.attributes.unwrap_or_default()),
        None => (String::new(), std::collections::HashMap::new()),
    };
    let typ = ev
        .typ
        .and_then(|t| serde_json::to_value(t).ok())
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    json!({
        "type": typ,
        "action": ev.action.unwrap_or_default(),
        "id": id,
        "name": attrs.get("name").cloned().unwrap_or_default(),
        "image": attrs.get("image").cloned().unwrap_or_default(),
        "time": ev.time.unwrap_or_default(),
        "attrs": attrs,
    })
}
