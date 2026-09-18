//! 容器内交互式终端（`docker exec -it` 的等价物）。
//!
//! 走 Engine API 的 exec 端点而不是 `docker exec` 子进程：
//! - stdout / stdin 是真正的双向流，不需要把 zapexec 变成 PTY 中转站
//! - 支持窗口尺寸热更新（`resize_exec`），`vim` / `top` 这类程序才能正常重绘
//! - 会话结束能拿到容器内命令的退出码
//!
//! 生命周期由三条输入决定：
//! - `output.next()` 结束 → 容器内进程退出
//! - `stdin_rx` 关闭（客户端断开 / 发 `StreamClose`）→ 关闭 stdin，等待进程退出
//! - `resize_rx` → 调整 TTY 尺寸

use bollard::exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::docker::{client, log_bytes};
use crate::stream::StreamSink;

/// 会话空闲上限：容器内命令跑飞时不能一直挂着（zapd 侧的 WS 由前端保活）。
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3600);

/// 运行一个 exec 会话，直到容器内命令退出或客户端断开。
#[allow(clippy::too_many_arguments)]
pub async fn run(
    container: String,
    cmd: Vec<String>,
    user: Option<String>,
    cols: u16,
    rows: u16,
    mut stdin_rx: mpsc::Receiver<Vec<u8>>,
    mut resize_rx: mpsc::Receiver<(u16, u16)>,
    sink: StreamSink,
) {
    // 空 cmd 退回到 sh：绝大多数镜像（含 alpine）都有
    let cmd: Vec<String> = if cmd.is_empty() {
        vec!["sh".to_string()]
    } else {
        cmd
    };

    let d = match client() {
        Ok(d) => d,
        Err(e) => {
            sink.error(e).await;
            return;
        }
    };

    // 1) 创建 exec 实例
    let exec = match tokio::time::timeout(
        std::time::Duration::from_secs(20),
        d.create_exec(
            &container,
            CreateExecOptions {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                attach_stdin: Some(true),
                tty: Some(true),
                cmd: Some(cmd),
                user,
                ..Default::default()
            },
        ),
    )
    .await
    {
        Ok(Ok(e)) => e,
        Ok(Err(e)) => {
            sink.error(format!("无法进入容器: {e}")).await;
            return;
        }
        Err(_) => {
            sink.error("进入容器超时").await;
            return;
        }
    };

    // 2) attach：拿到输出流与输入写入器
    let attached = match d
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: false,
                tty: true,
                ..Default::default()
            }),
        )
        .await
    {
        Ok(bollard::exec::StartExecResults::Attached { output, input }) => (output, input),
        Ok(bollard::exec::StartExecResults::Detached) => {
            sink.error("attach 失败：daemon 返回了 detached 结果").await;
            return;
        }
        Err(e) => {
            sink.error(format!("无法进入容器: {e}")).await;
            return;
        }
    };
    let mut output = attached.0;
    let mut input = attached.1;

    // 3) 首帧就把窗口尺寸同步过去，否则全屏程序会按 80x24 绘制
    let _ = d
        .resize_exec(
            &exec.id,
            ResizeExecOptions {
                height: rows,
                width: cols,
            },
        )
        .await;

    if !sink.ready().await {
        return;
    }

    // 4) 双向泵
    loop {
        tokio::select! {
            item = output.next() => match item {
                Some(Ok(chunk)) => {
                    if !sink.out(&log_bytes(chunk)).await {
                        break;
                    }
                }
                Some(Err(e)) => {
                    sink.error(format!("读取容器输出失败: {e}")).await;
                    return;
                }
                None => break,
            },
            data = stdin_rx.recv() => match data {
                Some(bytes) => {
                    if input.write_all(&bytes).await.is_err() || input.flush().await.is_err() {
                        break;
                    }
                }
                // 客户端断开 / 主动关闭：关掉 stdin，让容器内命令自己退出
                None => break,
            },
            size = resize_rx.recv() => {
                if let Some((cols, rows)) = size {
                    let _ = d.resize_exec(&exec.id, ResizeExecOptions { height: rows, width: cols }).await;
                }
            }
            _ = tokio::time::sleep(IDLE_TIMEOUT) => {
                sink.error("会话空闲超时，已断开").await;
                return;
            }
        }
    }

    // 5) 收尾：取退出码后结束会话
    let code = match d.inspect_exec(&exec.id).await {
        Ok(info) => info.exit_code.unwrap_or(0) as i32,
        Err(e) => {
            sink.error(format!("读取退出码失败: {e}")).await;
            return;
        }
    };
    sink.end(code).await;
}
