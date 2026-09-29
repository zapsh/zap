use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::types::Message;

/// 单帧上限，防止恶意超大帧占用内存。
const MAX_FRAME: u32 = 16 * 1024 * 1024;

/// 发送一帧：4 字节大端长度前缀 + JSON 载荷。
pub async fn send<W: AsyncWrite + Unpin>(w: &mut W, msg: &Message) -> io::Result<()> {
    let bytes = serde_json::to_vec(msg).map_err(serde_err)?;
    if bytes.len() > MAX_FRAME as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    w.write_u32(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

/// 读取一帧。
pub async fn recv<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Message> {
    let len = r.read_u32().await?;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    serde_json::from_slice(&buf).map_err(serde_err)
}

fn serde_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Request, Response};

    #[tokio::test]
    async fn round_trip_request() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let msg = Message::Request(Box::new(Request::FileRead {
            path: "/etc/hostname".into(),
            as_user: None,
            skip_owner_check: true,
        }));
        send(&mut a, &msg).await.unwrap();
        let got = recv(&mut b).await.unwrap();
        assert!(matches!(
            got,
            Message::Request(req) if matches!(&*req, Request::FileRead { path, .. } if path == "/etc/hostname")
        ));
    }

    #[tokio::test]
    async fn round_trip_response() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let msg = Message::Response(Box::new(Response::err(7, "boom")));
        send(&mut a, &msg).await.unwrap();
        let got = recv(&mut b).await.unwrap();
        match got {
            Message::Response(r) => {
                assert_eq!(r.code, 7);
                assert_eq!(r.message, "boom");
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[tokio::test]
    async fn oversized_recv_frame_rejected() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        a.write_u32(MAX_FRAME + 1).await.unwrap();
        let err = recv(&mut b).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn oversized_send_payload_rejected() {
        let (mut a, _b) = tokio::io::duplex(64);
        let big = "x".repeat(MAX_FRAME as usize + 1);
        let msg = Message::Response(Box::new(Response::ok(
            "ok",
            Some(serde_json::json!({ "big": big })),
        )));
        let err = send(&mut a, &msg).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn invalid_json_rejected() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        a.write_u32(4).await.unwrap();
        a.write_all(b"nope").await.unwrap();
        let err = recv(&mut b).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn truncated_frame_rejected() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        // 长度前缀声明 100 字节，但只写 3 字节
        a.write_u32(100).await.unwrap();
        a.write_all(b"abc").await.unwrap();
        // 关闭写端：缓冲排空后读端收到 EOF，read_exact 才能返回 UnexpectedEof
        drop(a);
        let err = recv(&mut b).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
