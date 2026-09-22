use std::{env, sync::Arc, time::Duration};

use axum::{Router, extract::Request, http::StatusCode};
use clap::Parser;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower_http::compression::CompressionLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tower_service::Service;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod db;
mod routers;
pub mod zap;
pub mod zapexec;

/// Zap Pro（商业模块）。
///
/// 源码不在本仓库：clone 到仓库根目录的 `zappro/` 后，用 `#[path]` 以 zapd 的
/// 一个模块身份编译进来 —— 模块内可以像内置业务模块一样 `use crate::db::…`，
/// 不需要任何额外的 trait / 门面抽象。
///
/// 未 clone 时目录不存在，只要不开 `commercial`，这段声明不会参与编译。
#[cfg(feature = "commercial")]
#[path = "../../zappro/src/mod.rs"]
mod pro;

#[derive(clap::Parser)]
struct Cli {
    #[clap(short, long, action)]
    version: bool,

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

    /// 主控是自签证书时跳过证书校验（默认拒绝）
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

/// 默认日志级别（可用环境变量 `RUST_LOG` 覆盖）：
/// - debug 构建：保留详细日志（含 tower_http、axum rejection），便于本地联调
/// - release 构建：只输出 info 及以上，避免线上 journal 被 debug 日志刷屏
#[cfg(debug_assertions)]
const DEFAULT_LOG: &str = "zapd=debug,tower_http=debug,axum::rejection=trace";
#[cfg(not(debug_assertions))]
const DEFAULT_LOG: &str = "zapd=info,tower_http=info";

/// 等待 TLS ClientHello 首字节的上限。
///
/// 探测扫描、浏览器预连接、被中间设备掐断的握手都会出现「TCP 连上了却不发数据」，
/// 而 `peek` 会一直等到有数据或对方关闭。留一点余量后直接断开，避免这类连接
/// 长期占着 fd（配合下面的「每连接独立任务」，单个坏连接不会拖慢其它连接）。
const PEEK_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if cli.version {
        println!("zapd version {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| DEFAULT_LOG.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 打印实际生效的配置文件：生产环境 /etc/zap/zap.yaml 优先于 conf/zap.yaml，
    // rundev.sh 则通过 ZAP_CONFIG 指向 data/run/zap.dev.yaml。
    // 显示绝对路径 + 存在性，便于排查"改了配置但没生效"。
    let cfg_path = config::config_path();
    let cfg_display = cfg_path.canonicalize().unwrap_or_else(|_| cfg_path.clone());
    if cfg_path.exists() {
        info!("using config file: {}", cfg_display.display());
    } else {
        warn!(
            "配置文件不存在，将使用内置默认值（url_prefix 等设置不会生效）: {}",
            cfg_display.display()
        );
    }

    // 一次性读取配置并转为 owned 值：配置读写锁不跨 await 持有
    let (cert_file, key_file, bind, web_port) = {
        let cfg = config::get_config().read().unwrap();
        (
            cfg.server.cert_file.clone(),
            cfg.server.key_file.clone(),
            format!("{}:{}", cfg.server.address, cfg.server.port),
            cfg.server.port,
        )
    };
    // 统一 URL 前缀（server.url_prefix）：留空则不启用
    let url_prefix = config::url_prefix();

    // `zapd --init-admin <用户> [--admin-password <密码>]`：建库 + 写初始管理员后退出，
    // 不绑端口、不碰证书（install.sh 以 root 在首次启动服务前调用它）
    if let Some(username) = cli.init_admin.as_deref() {
        let code =
            zap::admin_bootstrap::init_admin_cli(username, cli.admin_password.as_deref()).await;
        std::process::exit(code);
    }

    // Ensure TLS certificates exist (generate self-signed if missing)
    // 面板只提供 HTTPS（HTTP 请求一律 301 跳转），没有证书就无法建立 TLS acceptor，
    // 因此这里直接以明确错误退出，而不是带着坏证书继续跑成崩溃重启循环。
    if !zap::certmgr::ensure_certs(&cert_file, &key_file) {
        error!(
            "面板 HTTPS 证书不可用（{} / {}）：请确认 zapd 运行用户（zapd.service 的 User=）\
             对证书文件所在目录有写权限，或手工放置证书后重启 zapd",
            cert_file, key_file
        );
        std::process::exit(1);
    }

    let tls_acceptor = create_tls_acceptor(&cert_file, &key_file);
    let tcp_listener = TcpListener::bind(&bind).await.unwrap();
    let primary_ip = local_ip_address::local_ip()
        .unwrap_or_else(|_| std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)));
    info!("listening on https://{}:{}", primary_ip, web_port);
    info!("Zap server listening on https://{}.", bind);
    if url_prefix.is_empty() {
        info!("URL prefix: (none) — 页面在 / ，接口在 /api/");
    } else {
        info!(
            "URL prefix: /{} — 页面在 /{}/ ，接口在 /{}/api/",
            url_prefix, url_prefix, url_prefix
        );
    }

    // init db
    db::init_db::init_schema().await;
    // Zap Pro：读取授权文件、定下「是否生效」（必须在路由组装之前定论）
    #[cfg(feature = "commercial")]
    pro::init().await;

    // Zap Pro 集群的一次性命令：不绑端口、不启服务，做完就退出。
    // 放在 init_schema + pro::init 之后 —— 它们要读写 `cluster_self` 表。
    #[cfg(feature = "commercial")]
    if let Some(code) = pro::cluster::cli::run_cli(&cli).await {
        std::process::exit(code)
    }

    // 全新库还没有管理员时补一条（默认 admin / 123456；安装脚本会在此之前用
    // `zapd --init-admin` 指定实际凭据，那时这里什么也不做）
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
    // 自动更新（zapd/zapexec 系统升级）定时调度
    zap::auto_update::start();

    // 全局请求超时：文件上传/下载、云存储与本地互传都属于「一口气传完」的长任务，
    // 10 秒会误杀（响应还没生成就被判超时）。这里放宽到 30 分钟只做兜底，
    // 具体服务的连接/读取超时交给各自客户端（zapexec、opendal/reqwest）。
    let app = Router::new().merge(routers::routers()).layer((
        TraceLayer::new_for_http(),
        // `TimeoutLayer::new` 已废弃，`with_status_code` 是它的等价写法（超时返回 408）
        TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(1800)),
        CompressionLayer::new(),
    ));

    loop {
        let (stream, client_addr) = match tcp_listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("failed to accept connection: {}", e);
                continue;
            }
        };

        // SO_LINGER 非零值会阻塞线程，tokio 因此标记了废弃，但这里是刻意保留的：
        // 关闭连接时若接收缓冲里还有没读完的数据，内核会直接发 RST，客户端看到的是
        // 「连接被重置」而不是响应内容（POST 被拒的 4xx 响应最容易踩到）；设了 linger
        // 才会走正常的四次挥手把已写数据送完。tokio 承诺该 API 不会被移除。
        #[allow(deprecated)]
        stream.set_linger(Some(Duration::from_secs(30))).ok();

        // 「读首字节 + 分发」必须放进独立任务：peek 会一直等到对端发来第一个字节，
        // 若放在 accept 循环里，一个只建 TCP 不发数据的连接（端口探测、预连接、
        // 被中断的握手）就会把整个 accept 循环挂住，表现为所有请求一起超时。
        let tls_acceptor = tls_acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            serve_connection(stream, client_addr, tls_acceptor, app).await;
        });
    }
}

/// 单连接的分发：先看首字节区分 HTTPS / 明文 HTTP，再交给对应的处理函数。
///
/// 在独立任务里执行（见 main 的 accept 循环），因此某个连接卡在 `peek` 上
/// 不会影响其它连接；`peek` 超时（对端始终不发数据）则直接断开。
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
