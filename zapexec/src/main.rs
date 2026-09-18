use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod client;
mod server;
mod stream;
mod verbs;

use server::ClientIdentity;

#[derive(Parser)]
#[command(name = "zapexec", about = "ZAP Executor Daemon (run as root)")]
struct Cli {
    /// zapd <-> zapexec 的 Unix socket 路径
    #[clap(long, env = "ZAP_EXEC_SOCKET", default_value = "/run/zap/exec.sock")]
    socket: PathBuf,

    /// HMAC 共享密钥文件（64 hex）
    #[clap(long, env = "ZAP_EXEC_SECRET", default_value = "/etc/zap/exec.key")]
    secret: PathBuf,

    /// 允许连接的 Unix 用户（socket 组归属）
    #[clap(long, env = "ZAP_EXEC_CLIENT_USER", default_value = "zapadm")]
    client_user: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 开发/联调用：以客户端身份连接并发送一个动词
    Client(client::ClientArgs),
}

/// 默认日志级别（可用环境变量 `RUST_LOG` 覆盖）：
/// debug 构建保留详细日志，release 只输出 info 及以上。
#[cfg(debug_assertions)]
const DEFAULT_LOG: &str = "zapexec=debug";
#[cfg(not(debug_assertions))]
const DEFAULT_LOG: &str = "zapexec=info";

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| DEFAULT_LOG.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    if let Some(Command::Client(args)) = cli.command {
        client::run(args).await;
        return;
    }

    let identity = resolve_client(&cli.client_user)
        .unwrap_or_else(|| panic!("无法解析用户 {}，请确认其存在", cli.client_user));
    let secret = load_or_create_secret(&cli.secret, identity);

    let zap_path = std::env::var("ZAP_PATH").unwrap_or_else(|_| "/usr/local/zap".to_string());
    info!(
        socket = %cli.socket.display(),
        uid = identity.uid,
        gid = identity.gid,
        zap_path = %zap_path,
        "zapexec 以 root 启动"
    );

    server::serve(&cli.socket, &secret, identity).await;
}

fn load_or_create_secret(path: &Path, identity: ClientIdentity) -> Vec<u8> {
    if let Ok(secret) = zap_proto::auth::load_secret(path.to_str().unwrap_or_default()) {
        return secret;
    }
    let hex = zap_proto::auth::generate_secret_hex().expect("生成密钥失败");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, hex.as_bytes()).expect("写入密钥文件失败");
    server::set_owner_mode(path, identity, 0o640);
    tracing::warn!("已生成新的 exec 密钥: {}", path.display());
    zap_proto::auth::load_secret(path.to_str().unwrap_or_default()).expect("加载新密钥失败")
}

fn resolve_client(name: &str) -> Option<ClientIdentity> {
    let cname = std::ffi::CString::new(name).ok()?;
    unsafe {
        let pw = libc::getpwnam(cname.as_ptr());
        if pw.is_null() {
            return None;
        }
        Some(ClientIdentity {
            uid: (*pw).pw_uid,
            gid: (*pw).pw_gid,
        })
    }
}
