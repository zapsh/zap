#![recursion_limit = "256"]
// SPDX-License-Identifier: AGPL-3.0-only

use std::{env, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::Request,
    http::{HeaderValue, StatusCode, header},
};
use clap::Parser;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tower_service::Service;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod db;
mod routers;
pub mod storage;
pub mod zap;
pub mod zapexec;

/// Zap Pro（商业模块）。
#[cfg(feature = "commercial")]
#[path = "../../zappro/src/mod.rs"]
mod pro;

#[derive(clap::Parser)]
struct Cli {
    #[clap(short, long, action)]
    version: bool,

    #[clap(long, action)]
    edition: bool,

    /// 初始化管理员账号后退出（install.sh 部署完成后调用，不启动面板服务）。
    /// 库不存在则建库建表；已有管理员则原样不动。
    #[clap(long, value_name = "USER")]
    init_admin: Option<String>,

    /// `--init-admin` 的密码；省略时生成随机密码并打印到标准输出
    #[clap(long, value_name = "PASSWORD", requires = "init_admin")]
    admin_password: Option<String>,

    /// Zap Pro：接入主控（主控基址，**含前缀**，如 https://ctrl.example.com:2600/zap）。
    /// 成功后凭据加密存本地，由后台循环持续上报。
    #[cfg(feature = "commercial")]
    #[clap(long, value_name = "URL")]
    join_url: Option<String>,

    /// `--join-url` 附带的注册口令（有且有效 → 主控当场通过，无需管理员审批）
    #[cfg(feature = "commercial")]
    #[clap(long, value_name = "CODE", requires = "join_url")]
    join_token: Option<String>,

    /// 在主控列表里显示的节点名（默认用主机名）
    #[cfg(feature = "commercial")]
    #[clap(long, value_name = "NAME", requires = "join_url")]
    join_name: Option<String>,

    /// 主控是自签证书：只信任**第一次**接入看到的证书，随后把它的指纹钉死
    /// （不再是一路放行；主控换证书将被拒绝，需重新接入）
    #[cfg(feature = "commercial")]
    #[clap(long, action, requires = "join_url")]
    join_insecure: bool,

    /// Zap Pro：打印本机接入主控的状态后退出
    #[cfg(feature = "commercial")]
    #[clap(long, action)]
    join_status: bool,

    /// Zap Pro：断开与主控的接入（清本地凭据，不影响主控记录）
    #[cfg(feature = "commercial")]
    #[clap(long, action)]
    unjoin: bool,
}

/// Default Log Level（ENV `RUST_LOG` Override）：
/// - debug ：tower_http、axum rejection
/// - release ：info+
#[cfg(debug_assertions)]
const DEFAULT_LOG: &str = "zapd=debug,tower_http=debug,axum::rejection=trace";
#[cfg(not(debug_assertions))]
const DEFAULT_LOG: &str = "zapd=info,tower_http=info";

/// TLS ClientHello
const PEEK_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() {
    // rustls ring
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::parse();
    if cli.version {
        println!("zapd version {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if cli.edition {
        println!(
            "{}",
            if cfg!(feature = "commercial") {
                "pro"
            } else {
                "community"
            }
        );
        return;
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| DEFAULT_LOG.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cfg_path = config::config_path();
    let cfg_display = cfg_path.canonicalize().unwrap_or_else(|_| cfg_path.clone());
    if cfg_path.exists() {
        info!("using config file: {}", cfg_display.display());
    } else {
        warn!("default config file : {}", cfg_display.display());
    }

    let (cert_file, key_file, bind, web_port) = {
        let cfg = config::get_config().read().unwrap();
        (
            cfg.server.cert_file.clone(),
            cfg.server.key_file.clone(),
            format!("{}:{}", cfg.server.address, cfg.server.port),
            cfg.server.port,
        )
    };
    // server.url_prefix
    let url_prefix = config::url_prefix();

    // `zapd --init-admin <用户> [--admin-password <密码>]`：建库 + 写初始管理员后退出，
    // 不绑端口、不碰证书（install.sh 以 root 在首次启动服务前调用它）
    if let Some(username) = cli.init_admin.as_deref() {
        let code =
            zap::admin_bootstrap::init_admin_cli(username, cli.admin_password.as_deref()).await;
        std::process::exit(code);
    }

    // init db
    db::init_db::init_schema().await;

    #[cfg(feature = "commercial")]
    pro::init().await;

    #[cfg(feature = "commercial")]
    if let Some(code) = pro::cluster::cli::run_cli(&cli).await {
        std::process::exit(code)
    }

    if let Err(e) = config::init_config() {
        error!("Configuration initialization failed. Zapd start failed : {e}");
        std::process::exit(1);
    }

    if !zap::certmgr::ensure_certs(&cert_file, &key_file) {
        error!(
            "HTTPS certificate unavailable ({} / {}) , Please confirm that the Zapd running user (zapd.service User=)\
             has write permissions for the directory where the certificate files are located, or place the certificate manually and restart.",
            cert_file, key_file
        );
        std::process::exit(1);
    }

    let tls_acceptor = create_tls_acceptor(&cert_file, &key_file);
    let tcp_listener = TcpListener::bind(&bind).await.unwrap();
    let primary_ip = local_ip_address::local_ip()
        .unwrap_or_else(|_| std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)));
    info!("listening on https://{}:{}", primary_ip, web_port);
    info!("Zap server listening on https://{}/{}.", bind, url_prefix);

    // `zapd --init-admin` , init admin account if not exist, admin / 123456
    db::init_db::ensure_initial_admin().await;
    // 会话版本号全量入内存：否则老库里已「下线过所有设备」的用户会被当成 0，
    // 签发出立刻失效的 token（见 zap::session）
    zap::session::load_all().await;

    // 日志目录命名自检：历史 {name}-{id} → {id}-{name}（含目录 rename 与 vhost 重同步）
    routers::site::migrate_log_roots().await;

    // 运行环境状态（{data}/server_env.yaml）：加载 + 启动时重新探测（过期/缺失时）
    zap::server_env::init();
    zap::server_env::refresh_on_startup(zap::server_env::STARTUP_STALE_SECS).await;
    // 自动更新配置（{data}/update_config.yaml）：加载，缺失则写默认值
    zap::update_config::init();

    // 管理员的 Linux 账号 / 家目录缺失时自动补齐（后台执行，失败仅告警）
    tokio::spawn(zap::admin_bootstrap::ensure_admin_home());

    // init job scheduler for system monitoring
    zap::job::init_system_jobs().await;
    // 通用任务队列调度器：并发组空出槽位就放行排队的任务（应用商店编译等）
    zap::task::spawn_scheduler();
    // init cron scheduler for 脚本/自动化 计划任务
    zap::script_cron::start();
    // init cron scheduler for 面板用户计划任务（crontab.yaml）
    zap::user_cron::start();
    // 备份任务调度器（目录 / 数据库定时备份）
    zap::backup_scheduler::start();
    // 自动更新（zapd/zapexec 系统升级）定时调度
    zap::auto_update::start();
    // IP Ban（默认关闭，面板开启后生效）
    zap::ip_ban::start();
    // 插件定时任务（cron）调度：Webhook 触发不走这里，由请求直接命中
    zap::plugin_schedule::start();
    // 回收上次运行遗留的僵死 ACME 订单（重启会丢掉内存里的签发任务）
    zap::acme::recover_stale_orders().await;
    // SSL 证书自动续期：扫描临近到期的 ACME 证书 → 自动续签 → 部署到站点 → 通知结果
    zap::cert_renew::start();

    // 全局请求超时：文件上传/下载、云存储与本地互传都属于「一口气传完」的长任务，
    // 10 秒会误杀（响应还没生成就被判超时）。这里放宽到 30 分钟只做兜底，
    // 具体服务的连接/读取超时交给各自客户端（zapexec、opendal/reqwest）。
    // 全局安全响应头：防点击劫持 / MIME 嗅探 / 传输降级 / XSS 注入。
    // CSP 此处保留 `unsafe-inline`/`unsafe-eval` 以兼容 Vue SPA 的内联脚本与部分
    // 运行时库；要更严格需先对前端做内联脚本审计（改用 nonce），否则会搞崩面板。
    const SECURITY_CSP: &str = "default-src 'self'; \
        script-src 'self' 'unsafe-inline' 'unsafe-eval'; \
        style-src 'self' 'unsafe-inline'; \
        img-src 'self' data: blob:; \
        font-src 'self' data:; \
        connect-src 'self'; \
        frame-ancestors 'none'; \
        base-uri 'self'; \
        form-action 'self'; \
        object-src 'none'";
    let app = Router::new().merge(routers::routers()).layer((
        TraceLayer::new_for_http(),
        // `TimeoutLayer::new` 已废弃，`with_status_code` 是它的等价写法（超时返回 408）
        TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(1800)),
        CompressionLayer::new(),
        SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ),
        SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ),
        SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
        SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(SECURITY_CSP),
        ),
        SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ),
    ));

    loop {
        let (stream, client_addr) = match tcp_listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("failed to accept connection: {}", e);
                continue;
            }
        };

        // SO_LINGER
        #[allow(deprecated)]
        stream.set_linger(Some(Duration::from_secs(30))).ok();

        let tls_acceptor = tls_acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            serve_connection(stream, client_addr, tls_acceptor, app).await;
        });
    }
}

async fn serve_connection(
    stream: tokio::net::TcpStream,
    client_addr: std::net::SocketAddr,
    acceptor: Arc<TlsAcceptor>,
    app: Router,
) {
    // A TLS ClientHello always starts with byte 0x16.
    // Peek one byte to tell HTTPS from plain HTTP on the same port.
    let mut buf = [0; 1];
    let n = match tokio::time::timeout(PEEK_TIMEOUT, stream.peek(&mut buf)).await {
        Ok(Ok(n)) => n,
        Ok(Err(_)) => return,
        Err(_) => {
            debug!(
                "客户端 {} 建连后 {} 秒内未发送任何数据，断开连接",
                client_addr,
                PEEK_TIMEOUT.as_secs()
            );
            return;
        }
    };

    if n > 0 && buf[0] == 0x16 {
        serve_tls_connection(stream, client_addr, acceptor, app).await;
    } else if let Err(e) = serve_plain_http(stream, client_addr).await {
        warn!("Error serving plain HTTP from {}: {}", client_addr, e);
    }
}

/// Serve a TLS connection with the axum app (HTTP/1.1 + HTTP/2 via ALPN).
async fn serve_tls_connection(
    stream: tokio::net::TcpStream,
    client_addr: std::net::SocketAddr,
    acceptor: Arc<TlsAcceptor>,
    app: Router,
) {
    // 握手（含 ALPN 协商）由 rustls 在 accept 内部完成
    let stream = match acceptor.accept(stream).await {
        Ok(stream) => stream,
        Err(e) => {
            error!("Error during TLS handshake from {}: {}", client_addr, e);
            return;
        }
    };

    let stream = TokioIo::new(stream);
    let hyper_service =
        hyper::service::service_fn(move |mut request: Request<hyper::body::Incoming>| {
            request.extensions_mut().insert(client_addr);
            app.clone().call(request)
        });

    if let Err(err) = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(stream, hyper_service)
        .await
    {
        warn!("Error serving TLS connection from {}: {}", client_addr, err);
    }
}

/// Serve a plain HTTP connection: redirect every request to HTTPS (301),
/// preserving the host, path and query string. Uses hyper's standard HTTP/1.1
/// parser instead of manual parsing, so oversized headers and odd requests
/// are handled correctly.
async fn serve_plain_http(
    stream: tokio::net::TcpStream,
    _client_addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    let redirect =
        hyper::service::service_fn(move |req: Request<hyper::body::Incoming>| async move {
            let host = req
                .headers()
                .get(hyper::header::HOST)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .unwrap_or_default();
            let target = req
                .uri()
                .path_and_query()
                .map(|p| p.as_str())
                .unwrap_or("/");

            let response = if host.is_empty() {
                // HTTP/1.1 requires a Host header — reject the request otherwise.
                axum::http::Response::builder()
                    .status(400)
                    .header("Content-Length", "0")
                    .body(axum::body::Body::empty())
                    .unwrap()
            } else {
                axum::http::Response::builder()
                    .status(301)
                    .header("Location", format!("https://{}{}", host, target))
                    .header("Content-Length", "0")
                    .header("Connection", "close")
                    .body(axum::body::Body::empty())
                    .unwrap()
            };
            Ok::<_, std::convert::Infallible>(response)
        });

    hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(stream), redirect)
        .await?;

    Ok(())
}

/// Build a rustls TLS acceptor from the PEM cert/key files.
///
/// ALPN 通告 h2 + http/1.1：浏览器/客户端协商 h2 时使用 HTTP/2，
/// 否则回落到 HTTP/1.1（与 hyper_util auto 的前言探测配合）。
fn create_tls_acceptor(cert: &str, key: &str) -> Arc<TlsAcceptor> {
    // rustls 要求显式选定进程级加密后端：统一使用 ring（纯 Rust 侧依赖，
    // 避免 aws-lc-rs 的 C 工具链）。依赖树里若同时引入了 aws-lc-rs，
    // 不显式安装会在构造 ServerConfig 时 panic。install_default 重复调用无害。
    let _ = rustls::crypto::ring::default_provider().install_default();

    let certs = load_certs(cert).expect("failed to load TLS certificate chain");
    let key = load_private_key(key).expect("failed to load TLS private key");

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("TLS 证书与私钥不匹配，无法构建服务端配置");

    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Arc::new(TlsAcceptor::from(Arc::new(config)))
}

/// 读取 PEM 证书链（fullchain 时含中间证书，按文件顺序全部加载）。
fn load_certs(path: &str) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("打开证书文件 {} 失败: {}", path, e))?;
    let mut reader = std::io::BufReader::new(file);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("解析证书 {} 失败: {}", path, e))?;
    if certs.is_empty() {
        return Err(anyhow::anyhow!("证书文件 {} 中没有证书", path));
    }
    Ok(certs)
}

/// 读取 PEM 私钥（PKCS#8 / PKCS#1 / SEC1 均可）。
fn load_private_key(path: &str) -> anyhow::Result<PrivateKeyDer<'static>> {
    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("打开私钥文件 {} 失败: {}", path, e))?;
    let mut reader = std::io::BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| anyhow::anyhow!("解析私钥 {} 失败: {}", path, e))?
        .ok_or_else(|| anyhow::anyhow!("私钥文件 {} 中没有可用的私钥", path))
}
