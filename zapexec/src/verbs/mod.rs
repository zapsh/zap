mod acme;
mod appstore;
mod cred;
mod cron;
mod docker;
mod docker_events;
mod docker_exec;
mod env;
mod file;
mod firewall;
mod fs;
mod logs;
mod network;
mod nginx;
mod php;
mod php_ext;
mod platform;
mod process;
mod resource;
mod service;
mod service_conf;
mod services;
mod site;
mod ssh;
mod ssh_key;
mod ssh_user_key;
mod svc;
mod time;
mod upgrade;
mod user;
mod waf;
mod webconf;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{Mutex, mpsc};
use zap_proto::{Request, Response};

use crate::stream::StreamSink;

/// 软件安装根目录：第三方软件本体安装到此处（与 zap 面板数据解耦）。
/// 默认 `/usr/local/apps`；可用环境变量 `ZAP_APPS_DIR` 覆盖
/// （rundev / systemd / 自定义脚本均可设置）。
/// 注意：安装元数据（meta.yaml / info.yaml）仍在 `{ZAP_PATH}/data/apps`，
/// 两者职责不同。
pub(super) fn install_root() -> PathBuf {
    std::env::var("ZAP_APPS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/apps"))
}

/// 白名单动词分发：这里没有、也不会有任意 shell 执行入口。
pub async fn dispatch(req: Request) -> Response {
    match req {
        Request::TimeSync => time::sync().await,
        Request::TimeSetTimezone { timezone } => time::set_timezone(&timezone).await,
        Request::TimeListTimezones => time::list_timezones().await,
        Request::TimeGet => time::get().await,
        Request::NetworkGet => network::get().await,
        Request::NetworkSetHostname { hostname } => network::set_hostname(&hostname).await,
        Request::NetworkSetResolver {
            nameservers,
            search,
        } => network::set_resolver(&nameservers, &search).await,
        Request::SshStatus => ssh::status().await,
        Request::SshRestart => ssh::restart().await,
        Request::SshInstall { run_id } => ssh::install(run_id).await,
        Request::ServiceList => service::list().await,
        Request::ServiceAction { name, action } => service::action(&name, &action).await,
        Request::ProcessList => process::list().await,
        Request::ProcessKill { pid, signal } => process::kill(pid, signal).await,
        Request::SshKeyInstallPub {
            username,
            public_key,
        } => ssh_key::install_pub(username, public_key).await,
        Request::SshUserKeyGenerate {
            linux_user,
            name,
            key_type,
            bits,
            comment,
        } => ssh_user_key::generate(linux_user, name, key_type, bits, comment).await,
        Request::SshUserKeyImport {
            linux_user,
            name,
            private_key,
            public_key,
            comment,
        } => ssh_user_key::import(linux_user, name, private_key, public_key, comment).await,
        Request::SshUserKeyDelete { linux_user, name } => {
            ssh_user_key::delete(linux_user, name).await
        }
        Request::SshUserKeyPrivateGet { linux_user, name } => {
            ssh_user_key::private_get(linux_user, name).await
        }
        Request::SshUserKeyDefaultGet { linux_user } => ssh_user_key::default_get(linux_user).await,
        Request::SshUserKeyPublicGet { linux_user, name } => {
            ssh_user_key::public_get(linux_user, name).await
        }
        Request::SshUserKeyList { linux_user } => ssh_user_key::list(linux_user).await,
        Request::FileList { path } => file::list(path).await,
        Request::FileRead { path } => file::read(path).await,
        Request::FileWrite {
            path,
            content,
            as_user,
            skip_owner_check,
        } => file::write(path, content, as_user, skip_owner_check).await,
        Request::FileDelete {
            path,
            as_user,
            skip_owner_check,
        } => file::delete(path, as_user, skip_owner_check).await,
        Request::FileMkdir {
            path,
            as_user,
            skip_owner_check,
        } => file::mkdir(path, as_user, skip_owner_check).await,
        Request::FileRename {
            path,
            new_path,
            as_user,
            skip_owner_check,
        } => file::rename(path, new_path, as_user, skip_owner_check).await,
        Request::FileDownload { path } => file::download(path).await,
        Request::FileUpload {
            path,
            name,
            content,
            as_user,
            skip_owner_check,
        } => file::upload(path, name, content, as_user, skip_owner_check).await,
        Request::FileInfo { path } => file::info(path).await,
        Request::FileChmod {
            path,
            mode,
            recursive,
            as_user,
            skip_owner_check,
        } => file::chmod(path, mode, recursive, as_user, skip_owner_check).await,
        Request::FileChown {
            path,
            owner,
            group,
            recursive,
            as_user,
            skip_owner_check,
        } => file::chown(path, owner, group, recursive, as_user, skip_owner_check).await,
        Request::FileCopy {
            path,
            new_path,
            as_user,
            skip_owner_check,
        } => file::copy(path, new_path, as_user, skip_owner_check).await,
        Request::FileArchive {
            paths,
            name,
            base_dir,
            dest_dir,
            as_user,
            skip_owner_check,
        } => file::archive(paths, name, base_dir, dest_dir, as_user, skip_owner_check).await,
        Request::AppstoreRepoAdd { name, url, run_id } => {
            appstore::repo_add(name, url, run_id).await
        }
        Request::AppstoreRepoRemove { id } => appstore::repo_remove(id).await,
        Request::AppstoreRepoUpdate { id, run_id } => appstore::repo_update(id, run_id).await,
        Request::AppstoreInstall {
            pkg_path,
            source,
            repo_id,
            version,
            action,
            options,
            instance,
            provision,
            user,
            run_mode,
            run_id,
        } => {
            appstore::install(
                pkg_path, source, repo_id, version, action, options, instance, provision, user,
                run_mode, run_id,
            )
            .await
        }
        Request::AppstoreUninstall {
            pkg_path,
            options,
            instance,
            provision,
            user,
            run_mode,
            run_id,
        } => {
            appstore::uninstall(
                pkg_path, options, instance, provision, user, run_mode, run_id,
            )
            .await
        }
        Request::AppstoreUpgrade {
            pkg_path,
            source,
            repo_id,
            version,
            old_version,
            action,
            options,
            instance,
            provision,
            user,
            run_mode,
            run_id,
        } => {
            appstore::upgrade(
                pkg_path,
                source,
                repo_id,
                version,
                old_version,
                action,
                options,
                instance,
                provision,
                user,
                run_mode,
                run_id,
            )
            .await
        }
        Request::AppstoreScriptRun {
            path,
            run_id,
            username,
        } => appstore::script_run(path, run_id, username).await,
        Request::AppstoreScriptStop { run_id } => appstore::script_stop(run_id).await,
        Request::AppstoreScriptRead { path, username } => {
            appstore::script_read(path, username).await
        }
        Request::AppstoreScriptWrite {
            path,
            content,
            username,
        } => appstore::script_write(path, content, username).await,
        Request::AppstoreScriptDelete { path, username } => {
            appstore::script_delete(path, username).await
        }
        Request::AppstoreRunFiles { run_id } => appstore::run_files(run_id).await,
        Request::AppstoreRunFileRead { run_id, path } => {
            appstore::run_file_read(run_id, path).await
        }
        Request::AppstoreRunFileWrite {
            run_id,
            path,
            content,
        } => appstore::run_file_write(run_id, path, content).await,
        Request::AppstoreRunRetry { run_id, new_run_id } => {
            appstore::run_retry(run_id, new_run_id).await
        }
        Request::AppstoreInstalled => appstore::installed().await,
        Request::AppstoreInstanceAction {
            pkg_path,
            instance,
            action,
        } => appstore::instance_action(pkg_path, instance, action).await,
        Request::SiteVhostSync {
            site_id,
            name,
            domains,
            enabled,
            mode,
            php_socket,
            web_root,
            log_root,
            owner_user,
            site_type,
            pseudo_static,
            pseudo_custom,
            web_root_custom,
            upstreams,
            locations,
            ssl_fullchain,
            ssl_key,
            force_https,
            ssl_protocols,
            ssl_ciphers,
            ssl_prefer_server_ciphers,
            ssl_http2,
            listen_ipv4,
            listen_ipv6,
        } => {
            site::vhost_sync(site::SiteConfig {
                site_id,
                name,
                domains,
                enabled,
                mode,
                php_socket,
                web_root,
                log_root,
                owner_user,
                site_type,
                pseudo_static,
                pseudo_custom,
                web_root_custom,
                upstreams,
                locations,
                ssl_fullchain,
                ssl_key,
                force_https,
                ssl_protocols,
                ssl_ciphers,
                ssl_prefer_server_ciphers,
                ssl_http2,
                listen_ipv4,
                listen_ipv6,
            })
            .await
        }
        Request::SiteVhostRemove { site_id, name } => site::vhost_remove(site_id, name).await,
        Request::SiteDataRemove {
            web_roots,
            log_roots,
        } => site::data_remove(web_roots, log_roots).await,
        Request::FsBrowseDirs { base } => fs::browse_dirs(base).await,
        Request::FsDiskUsage { paths } => fs::disk_usage(paths).await,
        Request::SiteLogRotate {
            log_roots,
            keep_days,
        } => logs::rotate(log_roots, keep_days).await,
        Request::SiteLogList { log_root } => logs::list(log_root).await,
        Request::SiteLogRead {
            log_root,
            kind,
            archive,
            lines,
            keyword,
            status,
        } => logs::read(log_root, kind, archive, lines, keyword, status).await,
        Request::SiteLogClear { log_root, kind } => logs::clear(log_root, kind).await,
        Request::FirewallStatus { panel_port } => firewall::status(panel_port).await,
        Request::FirewallRuleAdd {
            port,
            proto,
            action,
            source,
            comment,
            panel_port,
        } => firewall::rule_add(port, proto, action, source, comment, panel_port).await,
        Request::FirewallRuleDelete { id, panel_port } => {
            firewall::rule_delete(id, panel_port).await
        }
        Request::FirewallToggle { action } => firewall::toggle(action).await,
        Request::EnvDetect => env::detect().await,
        Request::UserHomeInit { home_dir, owner } => user::home_init(&home_dir, &owner).await,
        Request::UserHomeMigrate {
            src_home,
            dest_home,
            owner,
        } => user::migrate_home(&src_home, &dest_home, &owner).await,
        Request::UserSystemInit {
            linux_user,
            home_dir,
        } => user::system_init(&linux_user, &home_dir).await,
        Request::UserSystemRemove { linux_user } => user::system_remove(&linux_user).await,
        Request::UserQuotaSet {
            linux_user,
            quota_mb,
        } => user::quota_set(&linux_user, quota_mb).await,
        Request::PhpPoolSync {
            php_instance,
            linux_user,
            home_dir,
            spec,
        } => php::pool_sync(php_instance, linux_user, home_dir, spec).await,
        Request::UpgradeInfo => upgrade::info().await,
        Request::UpgradeRun {
            run_id,
            stage_dir,
            log_path,
        } => upgrade::run(run_id, stage_dir, log_path).await,
        Request::NginxStatus => nginx::status().await,
        Request::NginxConfList => nginx::conf_list().await,
        Request::NginxConfRead { path } => nginx::conf_read(path).await,
        Request::NginxConfSave { path, content } => nginx::conf_save(path, content).await,
        Request::NginxControl { action } => nginx::control(&action).await,
        Request::NginxDefaultVhost { enable } => nginx::default_vhost(enable).await,
        Request::NginxStubStatus { enable } => nginx::stub_status(enable).await,
        Request::NginxStreamStatus => nginx::stream_status().await,
        Request::NginxStreamApply { content } => nginx::stream_apply(&content).await,
        Request::ServiceConfStatus { service } => service_conf::status(&service).await,
        Request::ServiceConfList { service } => service_conf::conf_list(&service).await,
        Request::ServiceConfRead { service, path } => service_conf::conf_read(&service, path).await,
        Request::ServiceConfSave {
            service,
            path,
            content,
        } => service_conf::conf_save(&service, path, content).await,
        Request::ServiceConfKeys { service } => service_conf::keys_get(&service).await,
        Request::ServiceConfKeysSave { service, keys } => {
            service_conf::keys_save(&service, keys).await
        }
        Request::ServiceConfControl { service, action } => {
            service_conf::control(&service, &action).await
        }
        Request::ServiceConfInstances { service } => service_conf::instances(&service).await,
        Request::ServiceConfDefs => service_conf::defs_list().await,
        Request::ServiceConfDefault { service, enable } => {
            service_conf::set_default(&service, enable).await
        }
        Request::PhpExtList { service } => php_ext::list(&service).await,
        Request::PhpExtToggle {
            service,
            name,
            enable,
        } => php_ext::toggle(&service, &name, enable).await,
        Request::PhpExtInstall {
            service,
            package,
            version,
            log_path,
        } => php_ext::install(&service, &package, &version, &log_path).await,
        Request::PhpExtRemove {
            service,
            name,
            log_path,
        } => php_ext::remove(&service, &name, &log_path).await,
        // ModSecurity（WAF）：可选能力，未安装时除 status 外一律拒绝
        Request::WafStatus => waf::status().await,
        Request::WafInstall { log_path } => waf::install(&log_path).await,
        Request::WafConfList => waf::conf_list().await,
        Request::WafConfRead { path } => waf::conf_read(&path).await,
        Request::WafConfSave { path, content } => waf::conf_save(&path, &content).await,
        Request::WafAudit { lines } => waf::audit(lines).await,
        Request::ServicesOverview => services::overview().await,
        Request::ServicesControl { svc, action } => services::control(&svc, &action).await,
        Request::ServicesBoot { svc, enable } => services::boot(&svc, enable).await,
        Request::CredRead { service, user } => cred::read(&service, &user).await,
        Request::CronRun {
            run_id,
            linux_user,
            home_dir,
            command,
            kind,
            log_path,
        } => cron::run(run_id, linux_user, home_dir, command, kind, log_path).await,
        // ACME HTTP-01 验证文件托管（Let's Encrypt 申请流程）
        Request::AcmeHttpWrite { entries } => acme::http_write(entries).await,
        Request::AcmeHttpClear { tokens } => acme::http_clear(tokens).await,
        // Docker 容器管理（面板「容器」）
        Request::DockerStatus => docker::status().await,
        Request::DockerContainers { all } => docker::containers(all).await,
        Request::DockerContainerAction { ids, action } => {
            docker::container_action(&ids, &action).await
        }
        Request::DockerContainerRun {
            image,
            name,
            ports,
            restart,
        } => docker::container_run(&image, &name, &ports, &restart).await,
        Request::DockerContainerInspect { id } => docker::container_inspect(&id).await,
        Request::DockerContainerLogs {
            id,
            tail,
            since,
            timestamps,
        } => docker::container_logs(&id, tail, since.as_deref(), timestamps).await,
        Request::DockerStats => docker::stats().await,
        Request::DockerImages => docker::images().await,
        Request::DockerImageAction { id, action } => docker::image_action(&id, &action).await,
        Request::DockerImageInspect { id } => docker::image_inspect(&id).await,
        Request::DockerImageBuild {
            run_id,
            log_path,
            context_dir,
            containerfile,
            tags,
            build_args,
            platform,
            no_cache,
            pull,
        } => {
            docker::image_build(
                run_id,
                log_path,
                context_dir,
                containerfile,
                tags,
                build_args,
                platform,
                no_cache,
                pull,
            )
            .await
        }
        Request::DockerVolumes => docker::volumes().await,
        Request::DockerVolumeAction {
            name,
            action,
            owner_home,
            owner_user,
        } => docker::volume_action(&name, &action, &owner_home, &owner_user).await,
        Request::DockerNetworks => docker::networks().await,
        Request::DockerNetworkAction {
            name,
            action,
            driver,
        } => docker::network_action(&name, &action, driver.as_deref()).await,
        Request::DockerComposeList => docker::compose_list().await,
        Request::DockerComposeAction { project, action } => {
            docker::compose_action(&project, &action).await
        }
        Request::DockerComposeFile { project } => docker::compose_file(&project).await,
        Request::DockerComposeSave {
            project,
            content,
            location,
            home,
            owner,
            path,
        } => docker::compose_save(
            &project,
            &content,
            location.as_deref(),
            home.as_deref(),
            owner.as_deref(),
            path.as_deref(),
        ),
        Request::DockerComposeRemove { project } => docker::compose_remove(&project).await,
        Request::DockerComposeLogs { project, tail } => docker::compose_logs(&project, tail).await,
        // 交互式终端与事件流都是长会话，只能走 `dispatch_stream`（StreamOpen）：
        // 一问一答的通道承载不了持续输入 / 持续输出。
        Request::DockerContainerExec { .. } => {
            Response::err(-1, "容器终端请使用流式会话".to_string())
        }
        Request::DockerEvents => Response::err(-1, "事件流请使用流式会话".to_string()),
    }
}

/// 开启一个流式会话（容器 exec 终端 / 守护事件流）。
///
/// 会话任务直接往 `wr` 写输出帧，主循环负责把 stdin / resize 转发进来；
/// `stdin_rx` / `resize_rx` 被丢弃即代表客户端断开，会话随之结束。
pub async fn dispatch_stream(
    id: String,
    req: Request,
    stdin_rx: mpsc::Receiver<Vec<u8>>,
    resize_rx: mpsc::Receiver<(u16, u16)>,
    wr: Arc<Mutex<OwnedWriteHalf>>,
) {
    match req {
        Request::DockerContainerExec {
            id: container,
            cmd,
            user,
            cols,
            rows,
        } => {
            let sink = StreamSink::new(&id, wr);
            docker_exec::run(container, cmd, user, cols, rows, stdin_rx, resize_rx, sink).await;
        }
        // 事件流只有下行：stdin / resize 通道用不上，丢弃即代表客户端断开
        Request::DockerEvents => {
            let sink = StreamSink::new(&id, wr);
            drop(stdin_rx);
            drop(resize_rx);
            docker_events::run(sink).await;
        }
        other => {
            let sink = StreamSink::new(&id, wr);
            sink.error(format!("该请求不支持流式会话: {other:?}")).await;
        }
    }
}

/// bash 解释器绝对路径：多数发行版自带 `/bin/bash`，少数在 `/usr/bin/bash`
/// （AppStore 包脚本、计划任务、用户脚本都用它拉起，故统一走绝对路径）。
/// 找不到时回退 PATH 查找（`root_cmd` 的安全 PATH 已含 /usr/local/bin）。
pub(crate) fn bash_bin() -> String {
    for p in ["/bin/bash", "/usr/bin/bash"] {
        if Path::new(p).is_file() {
            return p.to_string();
        }
    }
    "bash".to_string()
}

/// 长任务日志：追加一行（PHP 扩展编译、WAF 安装等分钟级任务共用）。
///
/// 每一步都即时落盘，前端 WebSocket 才能边跑边看；任务收尾靠 [`finish_log`]。
pub(crate) fn log_line(path: &str, text: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{text}");
    }
}

/// 跑一步 shell：输出实时追加到日志，返回退出码。
///
/// 退出码由 bash 自己 echo 出来（而非 Rust 侧判断），这样脚本里出现 `exit`
/// 之外的失败路径也能拿到真实结果。日志与命令输出都走 `>>`，不进内存。
pub(crate) fn run_step(log: &str, title: &str, script: &str) -> i32 {
    log_line(log, &format!("── {title} ──"));
    let out = root_cmd(crate::verbs::platform::SHELL)
        .args(["-c"])
        .arg(format!(
            "{{ {script}; }} >> {log} 2>&1; echo \"__ZAP_STEP__$?\""
        ))
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .rsplit("__ZAP_STEP__")
            .next()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(-1),
        Err(_) => -1,
    }
}

/// 写完成标记与退出码文件：`.ret` 是权威来源，日志里的 `__ZAP_DONE__` 只是展示协议。
pub(crate) fn finish_log(log: &str, code: i32) {
    log_line(log, &format!("__ZAP_DONE__ {code}"));
    let ret = std::path::Path::new(log).with_extension("ret");
    let _ = std::fs::write(ret, code.to_string());
}

/// 构造一个清空环境、仅带安全 PATH 的 root 子进程命令。
pub(crate) fn root_cmd(program: &str) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    c.env_clear();
    c.env(
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    );
    c
}

/// 面板用户对应的 Linux 系统账号信息。
#[derive(Debug, Clone)]
pub(crate) struct LinuxAccount {
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}

// ── 用户私有目录的属主 ──────────────────────────────────────
//
// `{ZAP_PATH}/data/users/<user>/` 下混着两类数据，属主边界必须分清：
//
//   - **面板自己的数据**：`crontab.yaml` / `cloud/` / `docker-build-logs/` / `scripts/`
//     → 由非 root 的 zapd 直接读写，所以 `users/<user>` 这一层必须归**面板进程**；
//   - **站点应用数据**：`webapps/<name>/<site_id>/`
//     → 归站点账号（见 `appstore::prepare_user_run`），隔离不变。
//
// 坑在于 `users/<user>` 常常是 root（zapexec）装站点应用 / 跑脚本时先建出来的，
// 属主 root 之后面板进程就 Permission denied（「创建配置目录失败: Permission denied」）。
// 因此 root 侧每次触碰用户目录，都顺手把这一层交还给面板进程。

/// `{ZAP_PATH}/data/users`。
pub(super) fn users_root() -> PathBuf {
    zap_path().join("data").join("users")
}

fn zap_path() -> PathBuf {
    std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"))
}

/// 把路径属主改为面板进程（未记录身份时跳过）。
fn chown_to_panel(path: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;

    let Some(id) = crate::server::panel_identity() else {
        return Ok(());
    };
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("非法路径: {}", path.display()))?;
    if unsafe { libc::chown(c.as_ptr(), id.uid, id.gid) } != 0 {
        return Err(format!(
            "修改属主失败 {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// 从 `{data}/users/<user>/…` 取出 `<user>`。
pub(super) fn panel_user_of(path: &Path) -> Option<String> {
    let rel = path.strip_prefix(users_root()).ok()?;
    match rel.components().next()? {
        std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
        _ => None,
    }
}

/// 确保 `{data}/users/<user>` 存在且归面板进程所有。
pub(super) fn ensure_panel_user_dir(username: &str) -> Result<PathBuf, String> {
    let dir = users_root().join(username);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("创建用户目录失败 {}: {e}", dir.display()))?;
    chown_to_panel(&dir)?;
    // `data/users` 本身：面板进程要能在其中创建新用户目录
    chown_to_panel(&users_root())?;
    Ok(dir)
}

/// 按路径定位用户私有目录并修正属主；路径不在 `data/users/` 下时什么都不做。
pub(super) fn ensure_panel_dir_for_path(path: &Path) -> Result<(), String> {
    match panel_user_of(path) {
        Some(user) => ensure_panel_user_dir(&user).map(|_| ()),
        None => Ok(()),
    }
}

/// 启动时修一遍历史遗留：`data/users` 与其中已存在的每个用户目录。
///
/// 老版本装出来的这些目录属主是 root，升级后不修就会一直 Permission denied，
/// 而且成功与否不影响执行端可用性，所以只记日志、不返回错误。
pub(crate) fn fixup_user_dirs() {
    let root = users_root();
    if let Err(e) = std::fs::create_dir_all(&root) {
        tracing::warn!("创建 {} 失败: {e}", root.display());
        return;
    }
    if let Err(e) = chown_to_panel(&root) {
        tracing::warn!("{e}");
    }
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        if let Err(e) = chown_to_panel(&dir) {
            tracing::warn!("{e}");
        }
    }
}

/// 查询 Linux 账号（uid / gid / 家目录），账号不存在或名非法时报错。
///
/// 一律走 `getpwnam_r`，不 shell 出去拼 `id` 命令；账号名复用 `user` 动词的
/// 白名单校验（字母/下划线开头，长度 ≤ 32），杜绝把用户名拼进其它地方。
pub(crate) fn linux_account(user: &str) -> Result<LinuxAccount, String> {
    if !user::linux_user_ok(user) {
        return Err(format!("非法的 Linux 账号名: {user}"));
    }
    let name = std::ffi::CString::new(user).map_err(|_| format!("非法的 Linux 账号名: {user}"))?;
    unsafe {
        let buflen = libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX).max(4096) as usize;
        let mut buf: Vec<libc::c_char> = vec![0; buflen];
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwnam_r(
            name.as_ptr(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut out,
        );
        if rc != 0 || out.is_null() {
            return Err(format!("Linux 账号不存在: {user}"));
        }
        let home = std::ffi::CStr::from_ptr(pwd.pw_dir)
            .to_string_lossy()
            .into_owned();
        Ok(LinuxAccount {
            uid: pwd.pw_uid,
            gid: pwd.pw_gid,
            home: PathBuf::from(home),
        })
    }
}

/// 构造以指定 Linux 账号身份运行的子进程命令（与 `root_cmd` 对称）。
///
/// 额外收紧四点，用于执行第三方建站脚本（WordPress 之类）：
/// 1. 环境全清，只给最小集合 —— **PATH 不含 sbin**，避免继承 zapd/zapexec 的
///    任何进程环境（里面可能有 JWT 密钥、数据库凭据）；
/// 2. `cwd` 固定为账号家目录，脚本无法借相对路径落到系统目录；
/// 3. 主/属组在 exec 前降到该账号（nologin 账号），见 `drop_privileges`；
/// 4. 配合调用方 `pre_exec` 里的 `resource::TaskResource::enter`：禁再提权 +
///    关 core dump + 补齐 rlimit（跨平台，见 `resource` 模块文档）。
///
/// 返回命令 + 账号信息：**这里不降权**。降权必须先清附加组再 setgid/setuid，
/// 而 `CommandExt::uid/gid` 的降权时机由标准库内部决定，无法保证排在
/// `pre_exec` 之前；为了让顺序可控，整段降权放到调用方的 `pre_exec` 里做。
pub(crate) fn user_cmd(
    program: &str,
    user: &str,
) -> Result<(std::process::Command, LinuxAccount), String> {
    let acc = linux_account(user)?;
    let mut c = std::process::Command::new(program);
    c.env_clear();
    c.env("PATH", "/usr/local/bin:/usr/bin:/bin");
    c.env("HOME", &acc.home);
    c.env("USER", user);
    c.env("LOGNAME", user);
    // 家目录下的 tmp（home_init 已建）供脚本做临时目录，避免共用 /tmp 被其它账号窥探
    let tmp = acc.home.join("tmp");
    if tmp.is_dir() {
        c.env("TMPDIR", &tmp);
    }
    c.current_dir(&acc.home);
    Ok((c, acc))
}

/// 在 exec 前把子进程降到指定账号（在 `pre_exec` 内调用）。
///
/// 顺序**必须**是 清附加组 → setgid → setuid：`setgroups` 需要特权，uid 一旦
/// 降下去就没有第二次机会，而子进程默认会继承 zapexec 的附加组，等于多出一份
/// 本不该有的文件访问权。这也是不用 `CommandExt::uid/gid` 的原因——它和
/// `pre_exec` 的先后顺序不受调用方控制。
/// 任一步失败都返回 Err，让 `spawn` 直接失败：宁可任务起不来，也不能让脚本
/// 悄悄以 zapexec 自己的身份跑（那比降权失败更危险）。
pub(crate) fn drop_privileges(uid: u32, gid: u32) -> std::io::Result<()> {
    unsafe {
        if libc::setgroups(0, std::ptr::null()) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::setgid(gid) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::setuid(uid) != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// 给 0/1/2 之外继承来的 fd 打上 CLOEXEC（在 `pre_exec` 内、exec 之前调用）。
///
/// 标准库只重定向 stdio，其余 fd 一律继承：zapexec 是长驻进程，手上可能握着
/// socket / 日志文件 / 状态文件的 fd，脚本不该看见它们。
///
/// 只打标记而不直接 close —— Rust 用一条管道把 exec 失败的 errno 传回父进程，
/// 提前 close 会让「脚本不存在」这类错误退化成一个没有原因的退出码。
/// 只用 syscall，符合 `pre_exec` 的 async-signal-safe 约束。
pub(crate) fn cloexec_inherited_fds() {
    unsafe {
        let mut rl: libc::rlimit = std::mem::zeroed();
        let max = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) == 0 && rl.rlim_cur > 0 {
            (rl.rlim_cur as libc::c_int).min(4096)
        } else {
            256
        };
        for fd in 3..max {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::bash_bin;

    /// 多数发行版的 bash 在 /bin/bash，少数在 /usr/bin/bash：
    /// 无论命中哪条候选，都必须解析到 bash 本身。
    #[test]
    fn bash_bin_points_to_bash() {
        let b = bash_bin();
        assert!(b.ends_with("bash"), "解析出的解释器不是 bash: {b}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bash_bin_is_bin_bash_on_linux() {
        assert_eq!(bash_bin(), "/bin/bash");
    }
}
