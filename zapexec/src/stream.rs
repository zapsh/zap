//! 流式会话（容器 exec 终端）的服务端写侧封装。
//!
//! 一问一答的 `Request` / `Response` 承载不了交互式终端：stdout 需要增量回传。
//! 流式会话复用**同一条已认证的 Unix 连接**，由 `id` 区分会话。
//!
//! 写端同时被主循环（发响应帧）和会话任务（发输出帧）持有，
//! 用 `Mutex` 串行化，保证一帧写完再写下一帧（不会交错出半个帧）。

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::Mutex;
use zap_proto::{Message, frame};

/// 一个活跃会话的输出通道。
pub struct StreamSink {
    id: String,
    wr: Arc<Mutex<OwnedWriteHalf>>,
}

impl StreamSink {
    pub fn new(id: impl Into<String>, wr: Arc<Mutex<OwnedWriteHalf>>) -> Self {
        Self { id: id.into(), wr }
    }

    async fn send(&self, msg: &Message) -> bool {
        let mut w = self.wr.lock().await;
        frame::send(&mut *w, msg).await.is_ok()
    }

    /// 会话就绪：exec 已 attach 成功，可以开始收发数据。
    pub async fn ready(&self) -> bool {
        self.send(&Message::StreamReady {
            id: self.id.clone(),
        })
        .await
    }

    /// 会话输出。终端字节流不是有效 UTF-8 的场合很常见（分页、乱码），
    /// 因此按 **base64** 编码承载，不在传输层做有损转换。
    pub async fn out(&self, bytes: &[u8]) -> bool {
        self.send(&Message::StreamOut {
            id: self.id.clone(),
            data: BASE64.encode(bytes),
        })
        .await
    }

    /// 会话正常结束，携带容器内命令的退出码。
    pub async fn end(&self, code: i32) -> bool {
        self.send(&Message::StreamEnd {
            id: self.id.clone(),
            code,
        })
        .await
    }

    /// 会话异常结束。
    pub async fn error(&self, message: impl Into<String>) -> bool {
        self.send(&Message::StreamError {
            id: self.id.clone(),
            message: message.into(),
        })
        .await
    }
}

/// 解码客户端送来的 base64 输入；非法输入直接丢弃（不影响会话其余部分）。
pub fn decode_input(data: &str) -> Vec<u8> {
    BASE64.decode(data).unwrap_or_default()
}
