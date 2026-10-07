// SPDX-License-Identifier: AGPL-3.0-only
//! 最小 FastCGI 客户端：把 HTTP 请求转交给 PHP-FPM 执行。
//!
//! 仅实现 RESPONDER 角色需要的记录类型，采用「一次请求一条连接」
//! （不复用连接、不做多路复用），换取实现简单与故障隔离：
//! 单个请求异常不会污染后续请求。
//!
//! 记录格式（8 字节头 + 内容 + 填充）：
//! ```text
//! version(1) type(1) requestId(2) contentLength(2) paddingLength(1) reserved(1)
//! ```

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

const FCGI_VERSION_1: u8 = 1;
const FCGI_BEGIN_REQUEST: u8 = 1;
const FCGI_END_REQUEST: u8 = 3;
const FCGI_PARAMS: u8 = 4;
const FCGI_STDIN: u8 = 5;
const FCGI_STDOUT: u8 = 6;
const FCGI_STDERR: u8 = 7;

/// RESPONDER：常规请求响应角色（与 nginx fastcgi_pass 一致）。
const FCGI_RESPONDER: u16 = 1;
/// 协议层正常结束（应用状态另看 appStatus）。
const FCGI_REQUEST_COMPLETE: u8 = 0;
/// 单条记录内容上限（contentLength 为 u16）。
const MAX_CONTENT: usize = 65535;
/// FPM 无输出时的兜底响应体上限，避免异常脚本把内存打满。
const MAX_STDOUT: usize = 64 * 1024 * 1024;

/// FastCGI 响应：HTTP 状态 + 响应头 + 响应体。
pub struct FcgiResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// 向 PHP-FPM 发起一次请求。
///
/// - `socket`：FPM 监听的 unix socket 路径（如 `/run/php-fpm-74.sock`）
/// - `params`：CGI/FastCGI 参数（SCRIPT_FILENAME、QUERY_STRING、HTTP_* …）
/// - `body`：请求体（POST / 上传），无体传空切片
pub async fn request(
    socket: &str,
    params: &[(String, String)],
    body: &[u8],
) -> Result<FcgiResponse, String> {
    let mut stream = UnixStream::connect(socket)
        .await
        .map_err(|e| format!("连接 PHP-FPM 失败（{socket}）: {e}"))?;

    // BEGIN_REQUEST：role=RESPONDER，flags=0（不保持连接）
    let mut begin = [0u8; 8];
    begin[0..2].copy_from_slice(&FCGI_RESPONDER.to_be_bytes());
    write_record(&mut stream, FCGI_BEGIN_REQUEST, &begin).await?;

    // PARAMS：参数流 + 一条空记录表示结束
    let encoded = encode_params(params);
    write_stream(&mut stream, FCGI_PARAMS, &encoded).await?;
    write_record(&mut stream, FCGI_PARAMS, &[]).await?;

    // STDIN：请求体（自动分片）+ 空记录表示结束
    write_stream(&mut stream, FCGI_STDIN, body).await?;
    write_record(&mut stream, FCGI_STDIN, &[]).await?;

    read_response(&mut stream).await
}

/// 写单条记录（内容需 ≤ 65535）。
async fn write_record(stream: &mut UnixStream, rtype: u8, content: &[u8]) -> Result<(), String> {
    if content.len() > MAX_CONTENT {
        return Err("FastCGI 记录超出 65535 字节".to_string());
    }
    let mut head = [0u8; 8];
    head[0] = FCGI_VERSION_1;
    head[1] = rtype;
    head[2..4].copy_from_slice(&1u16.to_be_bytes()); // requestId 恒为 1
    head[4..6].copy_from_slice(&(content.len() as u16).to_be_bytes());
    head[6] = 0; // paddingLength
    head[7] = 0; // reserved
    stream
        .write_all(&head)
        .await
        .map_err(|e| format!("写入 FastCGI 头失败: {e}"))?;
    if !content.is_empty() {
        stream
            .write_all(content)
            .await
            .map_err(|e| format!("写入 FastCGI 内容失败: {e}"))?;
    }
    Ok(())
}

/// 按 65535 字节分片写入一个流（PARAMS / STDIN）。
async fn write_stream(stream: &mut UnixStream, rtype: u8, data: &[u8]) -> Result<(), String> {
    for chunk in data.chunks(MAX_CONTENT) {
        write_record(stream, rtype, chunk).await?;
    }
    Ok(())
}

/// name-value 编码：长度 <128 用 1 字节，否则 4 字节（最高位置 1）。
fn encode_params(params: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut len_buf = Vec::new();
    for (k, v) in params {
        encode_len(&mut len_buf, k.len());
        encode_len(&mut len_buf, v.len());
        out.extend_from_slice(&len_buf);
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(v.as_bytes());
        len_buf.clear();
    }
    out
}

fn encode_len(out: &mut Vec<u8>, len: usize) {
    if len < 128 {
        out.push(len as u8);
    } else {
        let bytes = (len as u32).to_be_bytes();
        out.push(bytes[0] | 0x80);
        out.extend_from_slice(&bytes[1..]);
    }
}

/// 读取 STDOUT / STDERR / END_REQUEST，直到协议层结束。
async fn read_response(stream: &mut UnixStream) -> Result<FcgiResponse, String> {
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let mut head = [0u8; 8];

    loop {
        if stream.read_exact(&mut head).await.is_err() {
            break; // FPM 关闭连接：以已收到的内容收尾
        }
        let rtype = head[1];
        let len = u16::from_be_bytes([head[4], head[5]]) as usize + head[6] as usize; // content + padding
        let mut buf = vec![0u8; len];
        if len > 0 && stream.read_exact(&mut buf).await.is_err() {
            break;
        }
        let content = &buf[..len - head[6] as usize];

        match rtype {
            FCGI_STDOUT => {
                if stdout.len() + content.len() <= MAX_STDOUT {
                    stdout.extend_from_slice(content);
                }
            }
            FCGI_STDERR => stderr.extend_from_slice(content),
            FCGI_END_REQUEST => {
                let _app_status =
                    u32::from_be_bytes([content[0], content[1], content[2], content[3]]);
                let protocol = content[4];
                if protocol != FCGI_REQUEST_COMPLETE {
                    let msg = String::from_utf8_lossy(&stderr).trim().to_string();
                    return Err(if msg.is_empty() {
                        "PHP-FPM 返回异常协议状态".to_string()
                    } else {
                        format!("PHP-FPM 执行失败: {msg}")
                    });
                }
                break;
            }
            _ => {}
        }
    }

    if !stderr.is_empty() {
        let msg = String::from_utf8_lossy(&stderr);
        let msg = msg.trim();
        if !msg.is_empty() {
            tracing::warn!("php-fpm stderr: {msg}");
        }
    }

    if stdout.is_empty() {
        return Err("PHP-FPM 未返回任何输出".to_string());
    }
    Ok(parse_response(&stdout))
}

/// 解析 FPM 输出：`Status:` 行 + 响应头 + 空行 + 响应体。
fn parse_response(stdout: &[u8]) -> FcgiResponse {
    // 头体分隔：优先 \r\n\r\n，兼容 \n\n
    let (head_end, body_start) = match find_bytes(stdout, b"\r\n\r\n") {
        Some(i) => (i, i + 4),
        None => match find_bytes(stdout, b"\n\n") {
            Some(i) => (i, i + 2),
            None => (stdout.len(), stdout.len()),
        },
    };
    let head_text = String::from_utf8_lossy(&stdout[..head_end]);
    let body = stdout.get(body_start..).unwrap_or_default().to_vec();

    let mut status: u16 = 200;
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in head_text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(v) = line.strip_prefix("Status:") {
            if let Ok(code) = v.split_whitespace().next().unwrap_or("").parse::<u16>() {
                status = code;
            }
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            if !k.is_empty() {
                headers.push((k.to_string(), v.trim().to_string()));
            }
        }
    }

    FcgiResponse {
        status,
        headers,
        body,
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_and_headers() {
        let raw =
            b"Status: 302 Found\r\nLocation: /index.php\r\nContent-Type: text/html\r\n\r\nbody";
        let r = parse_response(raw);
        assert_eq!(r.status, 302);
        assert!(
            r.headers
                .iter()
                .any(|(k, v)| k == "Location" && v == "/index.php")
        );
        assert_eq!(r.body, b"body".to_vec());
    }

    #[test]
    fn parses_default_status_without_status_line() {
        let raw = b"Content-Type: text/html; charset=utf-8\n\nhello";
        let r = parse_response(raw);
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"hello".to_vec());
    }

    #[test]
    fn encodes_param_lengths() {
        let mut out = Vec::new();
        encode_len(&mut out, 5);
        assert_eq!(out, vec![5]);
        out.clear();
        encode_len(&mut out, 300);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0] & 0x80, 0x80);
    }
}
