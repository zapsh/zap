use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, info, warn};

use zap_proto::{auth, frame, types::Message};

use crate::stream::decode_input;
use crate::verbs;

#[derive(Clone, Copy, Debug)]
pub struct ClientIdentity {
    pub uid: u32,
    pub gid: u32,
}

/// 面板进程（zapd）的运行身份，启动时记录一次，供动词层使用。
///
/// 面板自己的数据（`crontab.yaml` / `cloud/` / `docker-build-logs/` / `scripts/`）
/// 都落在 `{data}/users/<user>/` 下、且由非 root 的 zapd 直接读写；
/// 这些目录常常是 root（zapexec）先建出来的，需要把属主交还给面板进程，
/// 所以动词层要能拿到它的 uid/gid。
static PANEL_IDENTITY: OnceLock<ClientIdentity> = OnceLock::new();

/// 记录面板进程身份（重复调用无副作用）。
pub fn set_panel_identity(identity: ClientIdentity) {
    let _ = PANEL_IDENTITY.set(identity);
}

/// 面板进程身份；未初始化时为 `None`（此时不做属主调整）。
pub fn panel_identity() -> Option<ClientIdentity> {
    PANEL_IDENTITY.get().copied()
}

pub async fn serve(socket: &Path, secret: &[u8], identity: ClientIdentity) {
    set_panel_identity(identity);
    if let Some(dir) = socket.parent() {
        let _ = std::fs::create_dir_all(dir);
        set_owner_mode(dir, identity, 0o750);
    }
    if socket.exists() {
        let _ = std::fs::remove_file(socket);
    }

    let listener = match UnixListener::bind(socket) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("绑定 {} 失败: {e}", socket.display());
            return;
        }
    };
    set_owner_mode(socket, identity, 0o660);

    info!("监听 {}", socket.display());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let secret = secret.to_vec();
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(stream, &secret, identity.uid).await {
                        debug!("连接结束: {e}");
                    }
                });
            }
            Err(e) => warn!("accept 错误: {e}"),
        }
    }
}

async fn handle_conn(stream: UnixStream, secret: &[u8], expected_uid: u32) -> std::io::Result<()> {
    // 1) 对端凭据：Linux 走 SO_PEERCRED；OpenBSD 等平台没有该机制，返回 None
    //    表示跳过（认证由 socket 文件权限 + 下面的 HMAC 挑战把关）。
    let uid = match authorize_peer(stream.as_raw_fd(), expected_uid) {
        Ok(u) => u,
        Err(()) => return Ok(()),
    };

    let (mut rd, mut wr) = stream.into_split();

    // 2) 挑战/响应 HMAC 握手
    let challenge = auth::challenge_hex()?;
    frame::send(
        &mut wr,
        &Message::Challenge {
            challenge: challenge.clone(),
        },
    )
    .await?;

    match frame::recv(&mut rd).await? {
        Message::Auth { mac } => {
            if !auth::verify_hex(secret, challenge.as_bytes(), &mac) {
                warn!("对端认证失败（uid: {uid:?}）");
                return Ok(());
            }
        }
        _ => {
            warn!("握手阶段收到非法消息");
            return Ok(());
        }
    }
    frame::send(&mut wr, &Message::Welcome).await?;

    // 3) 消息循环
    //
    // 两种负载共用同一条已认证连接：
    // - `Request` / `Response`：一问一答（zapd 的 `zapexec::call`、zapctl），
    //   其客户端是"单请求-断开"模型，读到 EOF 是预期的正常结束。
    // - `StreamOpen` …：长会话（容器 exec 终端）。stdin / resize 由本循环转发给
    //   会话任务，stdout 由会话任务直接写回同一条连接，因此写端需要共享。
    let wr = Arc::new(Mutex::new(wr));
    let mut streams: HashMap<String, SessionHandle> = HashMap::new();

    loop {
        match frame::recv(&mut rd).await {
            Ok(Message::Request(req)) => {
                let resp = verbs::dispatch(*req).await;
                let mut w = wr.lock().await;
                if frame::send(&mut *w, &Message::Response(Box::new(resp)))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(Message::StreamOpen { id, req }) => {
                let (stdin_tx, stdin_rx) = mpsc::channel::<Vec<u8>>(64);
                let (resize_tx, resize_rx) = mpsc::channel::<(u16, u16)>(8);
                streams.insert(
                    id.clone(),
                    SessionHandle {
                        stdin_tx,
                        resize_tx,
                    },
                );
                let wr = wr.clone();
                tokio::spawn(async move {
                    verbs::dispatch_stream(id, *req, stdin_rx, resize_rx, wr).await;
                });
            }
            // 终端按键：base64 承载二进制
            Ok(Message::StreamIn { id, data }) => {
                if let Some(h) = streams.get(&id)
                    && h.stdin_tx.send(decode_input(&data)).await.is_err()
                {
                    // 会话已结束（任务退出），清掉残留 handle
                    streams.remove(&id);
                }
            }
            Ok(Message::StreamResize { id, cols, rows }) => {
                if let Some(h) = streams.get(&id) {
                    let _ = h.resize_tx.send((cols, rows)).await;
                }
            }
            Ok(Message::StreamClose { id }) => {
                // 丢弃 handle 即关闭 stdin 通道，会话任务随后收尾并结束 exec
                streams.remove(&id);
            }
            // 其余帧（Response / StreamOut / …）不该由客户端发来
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // 对端正常断开，不属于错误，静默结束连接
                break;
            }
            Err(e) => {
                debug!("读取错误: {e}");
                break;
            }
        }
    }

    // 连接断开：丢弃全部 handle → 会话任务的 stdin 通道关闭 → exec 结束，不会遗留孤儿进程
    drop(streams);
    Ok(())
}

/// 一个活跃流式会话的输入侧（stdin / resize 由消息循环转发）。
struct SessionHandle {
    stdin_tx: mpsc::Sender<Vec<u8>>,
    resize_tx: mpsc::Sender<(u16, u16)>,
}

/// 校验连接方 uid。
///
/// - Linux：`SO_PEERCRED` 能拿到对端 uid，不一致直接拒绝；
/// - 其它平台（OpenBSD 等）：没有等价机制，返回 `Ok(None)` 跳过。这不是降级——
///   socket 父目录 `0750 root:zapadm` + socket `0660` 已经在内核层把连接方限定为
///   zapadm，达到的判定与 uid 校验完全等价，后面还有 HMAC 挑战做第二道。
fn authorize_peer(
    #[cfg_attr(not(target_os = "linux"), allow(unused))] fd: std::os::unix::io::RawFd,
    #[cfg_attr(not(target_os = "linux"), allow(unused))] expected_uid: u32,
) -> Result<Option<u32>, ()> {
    #[cfg(target_os = "linux")]
    {
        match peer_uid(fd) {
            Some(u) if u == expected_uid => Ok(Some(u)),
            Some(u) => {
                warn!("拒绝来自 uid {u} 的连接（期望 {expected_uid}）");
                Err(())
            }
            None => {
                warn!("无法获取对端凭据，拒绝连接");
                Err(())
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(None)
    }
}

/// Linux 专有：取 unix socket 对端 uid。
#[cfg(target_os = "linux")]
fn peer_uid(fd: std::os::unix::io::RawFd) -> Option<u32> {
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        );
        if rc == 0 { Some(cred.uid) } else { None }
    }
}

pub(crate) fn set_owner_mode(path: &Path, identity: ClientIdentity, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
        return;
    };
    unsafe {
        // best-effort：root 下将属组设为 zapadm；非 root（开发）下静默失败
        libc::chown(cpath.as_ptr(), 0, identity.gid);
    }
}
