use std::path::PathBuf;

use clap::{Args, Subcommand};
use tokio::net::UnixStream;

use zap_proto::{Request, auth, frame, types::Message};

/// 解析命令行传入的八进制权限（支持 `755` / `0755` / `0o755`）。
fn parse_octal_mode(raw: &str) -> u32 {
    let digits = raw.trim().trim_start_matches("0o").trim_start_matches("0O");
    match u32::from_str_radix(digits, 8) {
        Ok(mode) if mode <= 0o7777 => mode,
        _ => {
            eprintln!("权限值非法：{raw}（需 0-7777 的八进制数字，如 0755）");
            std::process::exit(1);
        }
    }
}

#[derive(Args)]
pub struct ClientArgs {
    #[clap(long, default_value_os_t = PathBuf::from(zap_proto::DEFAULT_EXEC_SOCKET))]
    socket: PathBuf,

    #[clap(long, default_value = "/etc/zap/exec.key")]
    secret: PathBuf,

    #[command(subcommand)]
    verb: ClientVerb,
}

#[derive(Subcommand)]
enum ClientVerb {
    TimeGet,
    TimeSync,
    TimeListTimezones,
    TimeSetTimezone {
        timezone: String,
    },
    SshStatus,
    SshRestart,
    SshInstall,
    ServiceList,
    ServiceAction {
        name: String,
        action: String,
    },
    ProcessList,
    ProcessKill {
        pid: u32,
        signal: Option<String>,
    },
    FileList {
        path: String,
    },
    FileRead {
        path: String,
    },
    FileWrite {
        path: String,
        content: String,
    },
    FileDelete {
        path: String,
    },
    FileInfo {
        path: String,
    },
    /// 修改权限：mode 传八进制写法（755 / 0755 / 0o755 均可）
    FileChmod {
        path: String,
        mode: String,
    },
}

pub async fn run(args: ClientArgs) {
    let secret = auth::load_secret(args.secret.to_str().unwrap_or_default()).expect("无法读取密钥");
    let stream = UnixStream::connect(&args.socket)
        .await
        .expect("无法连接 socket");
    let (mut rd, mut wr) = stream.into_split();

    // 握手
    match frame::recv(&mut rd).await.expect("读取挑战失败") {
        Message::Challenge { challenge } => {
            let mac = auth::hmac_hex(&secret, challenge.as_bytes());
            frame::send(&mut wr, &Message::Auth { mac })
                .await
                .expect("发送认证失败");
        }
        _ => panic!("预期收到挑战消息"),
    }
    match frame::recv(&mut rd).await.expect("读取欢迎消息失败") {
        Message::Welcome => {}
        _ => panic!("握手被拒绝"),
    }

    let req = match args.verb {
        ClientVerb::TimeGet => Request::TimeGet,
        ClientVerb::TimeSync => Request::TimeSync,
        ClientVerb::TimeListTimezones => Request::TimeListTimezones,
        ClientVerb::TimeSetTimezone { timezone } => Request::TimeSetTimezone { timezone },
        ClientVerb::SshStatus => Request::SshStatus,
        ClientVerb::SshRestart => Request::SshRestart,
        ClientVerb::SshInstall => Request::SshInstall {
            run_id: format!(
                "cli-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ),
        },
        ClientVerb::ServiceList => Request::ServiceList,
        ClientVerb::ServiceAction { name, action } => Request::ServiceAction { name, action },
        ClientVerb::ProcessList => Request::ProcessList,
        ClientVerb::ProcessKill { pid, signal } => Request::ProcessKill { pid, signal },
        ClientVerb::FileList { path } => Request::FileList { path },
        ClientVerb::FileRead { path } => Request::FileRead { path },
        ClientVerb::FileWrite { path, content } => Request::FileWrite {
            path,
            content,
            as_user: None,
            skip_owner_check: false,
        },
        ClientVerb::FileDelete { path } => Request::FileDelete {
            path,
            as_user: None,
            skip_owner_check: false,
        },
        ClientVerb::FileInfo { path } => Request::FileInfo { path },
        ClientVerb::FileChmod { path, mode } => Request::FileChmod {
            path,
            mode: parse_octal_mode(&mode),
            recursive: false,
            as_user: None,
            skip_owner_check: false,
        },
    };

    frame::send(&mut wr, &Message::Request(Box::new(req)))
        .await
        .expect("发送请求失败");

    match frame::recv(&mut rd).await.expect("读取响应失败") {
        Message::Response(resp) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&resp).expect("序列化响应失败")
            );
        }
        _ => panic!("预期收到响应"),
    }
}
