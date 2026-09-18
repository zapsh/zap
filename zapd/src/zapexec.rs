//! `zapexec` 特权守护进程的客户端。
//!
//! `zapd` 以 `zapadm` 身份运行，需要 root 权限的操作通过本模块
//! 转发给 `zapexec`（以 root 运行的常驻进程）。

use std::path::Path;

use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::zap::ZapError;
use zap_proto::{
    auth, frame,
    types::{Message, Request, Response},
};

/// 建立连接并完成挑战/响应握手。
pub async fn connect(socket: &Path, secret_path: &Path) -> Result<ExecClient, ZapError> {
    let secret = auth::load_secret(secret_path.to_str().unwrap_or_default())
        .map_err(|e| ZapError::Error(format!("加载 exec 密钥失败: {e}")))?;

    let stream = UnixStream::connect(socket)
        .await
        .map_err(|e| ZapError::Error(format!("连接 zapexec 失败: {e}")))?;
    let (mut rd, mut wr) = stream.into_split();

    match frame::recv(&mut rd).await.map_err(io_err)? {
        Message::Challenge { challenge } => {
            let mac = auth::hmac_hex(&secret, challenge.as_bytes());
            frame::send(&mut wr, &Message::Auth { mac })
                .await
                .map_err(io_err)?;
        }
        _ => return Err(ZapError::Error("zapexec 握手失败：未收到挑战".to_string())),
    }

    match frame::recv(&mut rd).await.map_err(io_err)? {
        Message::Welcome => {}
        _ => return Err(ZapError::Error("zapexec 握手失败：认证被拒绝".to_string())),
    }

    Ok(ExecClient { rd, wr })
}

/// 单次请求/响应：连接、发送一个请求、读取响应后断开。
///
/// 适合偶发的短操作；需要长时间保持连接时（如文件传输）使用 [`connect`]。
pub async fn call_once(
    socket: &Path,
    secret_path: &Path,
    req: Request,
) -> Result<Response, ZapError> {
    let mut client = connect(socket, secret_path).await?;
    client.call(req).await
}

/// 从配置读取 exec 段并执行一次请求。
pub async fn call(req: Request) -> Result<Response, ZapError> {
    let exec_cfg = {
        let cfg = crate::config::get_config().read().unwrap();
        cfg.exec.clone()
    };
    call_once(
        Path::new(&exec_cfg.socket_path),
        Path::new(&exec_cfg.secret_path),
        req,
    )
    .await
}

/// 已认证的连接。
pub struct ExecClient {
    rd: tokio::net::unix::OwnedReadHalf,
    wr: tokio::net::unix::OwnedWriteHalf,
}

impl ExecClient {
    pub async fn call(&mut self, req: Request) -> Result<Response, ZapError> {
        frame::send(&mut self.wr, &Message::Request(Box::new(req)))
            .await
            .map_err(io_err)?;
        match frame::recv(&mut self.rd).await.map_err(io_err)? {
            Message::Response(resp) => Ok(*resp),
            _ => Err(ZapError::Error("zapexec 协议错误".to_string())),
        }
    }

    /// 拆成读写两半：流式会话（容器 exec 终端）要一边收服务端帧、一边发 stdin，
    /// 由调用方自己调度（例如 axum WebSocket 的双向转发）。
    pub fn into_parts(self) -> (OwnedReadHalf, OwnedWriteHalf) {
        (self.rd, self.wr)
    }
}

/// 在已认证连接上发送一帧（流式会话用）。
pub async fn send(wr: &mut OwnedWriteHalf, msg: &Message) -> Result<(), ZapError> {
    frame::send(wr, msg).await.map_err(io_err)
}

/// 从已认证连接读取一帧（流式会话用）。
pub async fn recv(rd: &mut OwnedReadHalf) -> Result<Message, ZapError> {
    frame::recv(rd).await.map_err(io_err)
}

fn io_err(e: std::io::Error) -> ZapError {
    ZapError::Error(format!("zapexec I/O 错误: {e}"))
}
