mod acme;
mod appstore;
mod cred;
mod cron;
mod env;
mod file;
mod firewall;
mod fs;
mod logs;
mod network;
mod nginx;
mod php;
mod process;
mod service;
mod service_conf;
mod site;
mod ssh;
mod ssh_key;
mod ssh_user_key;
mod time;
mod upgrade;
mod user;
mod webconf;

use std::path::PathBuf;

use zap_proto::{Request, Response};

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
            user,
            run_mode,
            run_id,
        } => {
            appstore::install(
                pkg_path, source, repo_id, version, action, options, user, run_mode, run_id,
            )
            .await
        }
        Request::AppstoreUninstall {
            pkg_path,
            options,
            user,
            run_mode,
            run_id,
        } => appstore::uninstall(pkg_path, options, user, run_mode, run_id).await,
        Request::AppstoreUpgrade {
            pkg_path,
            source,
            repo_id,
            version,
            old_version,
            action,
            options,
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
        Request::AppstoreInstanceAction { pkg_path, action } => {
            appstore::instance_action(pkg_path, action).await
        }
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
        Request::ServiceConfDefault { service, enable } => {
            service_conf::set_default(&service, enable).await
        }
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
    }
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
