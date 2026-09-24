use std::net::SocketAddr;

use axum::{
    Json,
    extract::{Extension, Path, Query},
};
use serde::Deserialize;
use serde_json::json;
use tracing::info;

use crate::zap::appstore as ast;
use crate::zap::{
    ZapError, ZapJsonResult, audit,
    jwt::{self, ValidatedClaims},
};
use zap_proto::Request;

/// 系统级配置（时间 / SSH / 服务 / 进程 / 网络）全部为管理员专属操作。
///
/// 说明：路由层已有 `routers::access` 权限矩阵中间件统一把关（默认 admin），
/// 这里保留 handler 内校验作为最后防御。
fn require_admin(claims: &jwt::Claims) -> Result<(), ZapError> {
    if jwt::is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "权限不足，需要管理员权限".to_string()))
    }
}

// ── Time ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SetTimezonePayload {
    pub timezone: String,
}

/// Get server time info
pub async fn get_time(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(Request::TimeGet).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "data": resp.data })))
}

/// Sync time via NTP
pub async fn sync_time(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(Request::TimeSync).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

/// Set system timezone
pub async fn set_timezone(
    claims: ValidatedClaims,
    Json(payload): Json<SetTimezonePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if payload.timezone.is_empty() {
        return Err(ZapError::New(-1, "时区不能为空".to_string()));
    }

    let resp = crate::zapexec::call(Request::TimeSetTimezone {
        timezone: payload.timezone,
    })
    .await?;

    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

/// Get list of available timezones
pub async fn list_timezones(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(Request::TimeListTimezones).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "data": resp.data })))
}

// ── SSH ────────────────────────────────────────────────────

/// Get SSH server status
pub async fn ssh_status(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(Request::SshStatus).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "data": resp.data })))
}

/// Restart SSH server
pub async fn ssh_restart(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(Request::SshRestart).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

// ── System Services ─────────────────────────────────────────

/// Get list of system services (systemd)
pub async fn list_services(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(Request::ServiceList).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "data": resp.data })))
}

#[derive(Debug, Deserialize)]
pub struct ServiceActionPayload {
    pub name: String,
    pub action: String,
}

/// start / stop / restart / reload / enable / disable a service
pub async fn service_action(
    claims: ValidatedClaims,
    Json(payload): Json<ServiceActionPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    if payload.name.is_empty() {
        return Err(ZapError::New(-1, "服务名称不能为空".to_string()));
    }
    let resp = crate::zapexec::call(Request::ServiceAction {
        name: payload.name,
        action: payload.action,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": resp.message, "data": resp.data }),
    ))
}

// ── Process Management ─────────────────────────────────────

/// 获取运行中的进程列表
pub async fn list_processes(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(Request::ProcessList).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "data": resp.data })))
}

#[derive(Debug, Deserialize)]
pub struct ProcessKillPayload {
    pub pid: u32,
    #[serde(default)]
    pub signal: Option<String>,
}

/// 终止进程（缺省 TERM，signal=9 为 KILL）
pub async fn process_kill(
    claims: ValidatedClaims,
    Json(payload): Json<ProcessKillPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(Request::ProcessKill {
        pid: payload.pid,
        signal: payload.signal,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": resp.message, "data": resp.data }),
    ))
}

// ── SSH Install ──────────────────────────────────────────────

/// 安装 openssh-server（后台异步，日志写入 run 记录，供前端轮询）
pub async fn ssh_install(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let run_id = ast::generate_run_id();
    let log_path = ast::log_path_for(&run_id);
    ast::register_run(
        &run_id,
        "ssh_install",
        "openssh-server",
        &claims.sub,
        &log_path,
    )
    .await?;

    let resp = crate::zapexec::call(Request::SshInstall {
        run_id: run_id.clone(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(&run_id, "failed", resp.code as i64).await;
        return Err(ZapError::New(resp.code, resp.message));
    }
    ast::watch_log(run_id.clone(), log_path.clone());
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "ssh_install",
        &claims.sub,
        "openssh-server",
    )
    .await;
    info!("SSH install started: {run_id}");
    Ok(Json(json!({
        "code": 0,
        "message": "安装已启动",
        "data": { "run_id": run_id, "log": log_path }
    })))
}

#[derive(Debug, Deserialize)]
pub struct SshInstallLogQuery {
    pub offset: Option<u64>,
}

/// 轮询安装日志
pub async fn ssh_install_log(
    claims: ValidatedClaims,
    Path(run_id): Path<String>,
    Query(q): Query<SshInstallLogQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let run = ast::get_run(&run_id)
        .await?
        .ok_or_else(|| ZapError::New(-1, "任务不存在".to_string()))?;
    let (content, exit_code, done) = ast::read_log(&run.log_path, q.offset.unwrap_or(0)).await?;
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": { "content": content, "exit_code": exit_code, "done": done, "status": run.status }
    })))
}

// ── Network（主机名 / DNS Resolver）─────────────────────────

/// 读取主机名与 DNS 解析器配置
pub async fn network_get(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = crate::zapexec::call(Request::NetworkGet).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "data": resp.data })))
}

#[derive(Debug, Deserialize)]
pub struct SetHostnamePayload {
    pub hostname: String,
}

/// 设置主机名
pub async fn network_set_hostname(
    claims: ValidatedClaims,
    Json(payload): Json<SetHostnamePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let hostname = payload.hostname.trim().to_string();
    if hostname.is_empty() {
        return Err(ZapError::New(-1, "主机名不能为空".to_string()));
    }
    let resp = crate::zapexec::call(Request::NetworkSetHostname { hostname }).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

#[derive(Debug, Deserialize)]
pub struct SetResolverPayload {
    pub nameservers: Vec<String>,
    #[serde(default)]
    pub search: Vec<String>,
}

/// 设置 DNS Resolver（nameserver / search）
pub async fn network_set_resolver(
    claims: ValidatedClaims,
    Json(payload): Json<SetResolverPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let nameservers: Vec<String> = payload
        .nameservers
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let search: Vec<String> = payload
        .search
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let resp = crate::zapexec::call(Request::NetworkSetResolver {
        nameservers,
        search,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message })))
}

// ── 包下载源（应用商店取源码包 / 离线本地目录）───────────────

/// 当前下载源与可选预设（管理员）。
pub async fn mirror_get(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    Ok(Json(
        json!({ "code": 0, "data": crate::zap::mirror::info() }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct SetMirrorPayload {
    pub pkg_mirror: String,
}

/// 设置下载源：`https://…/pkg` 或本地目录（绝对路径 / file:// 开头）。
///
/// 只落 zapd 自己的配置文件，不惊动 zapexec：包脚本每次执行时才读它，
/// 因此改完立即对**下一次**安装生效，跑着的任务不受影响。
pub async fn mirror_set(
    claims: ValidatedClaims,
    Json(payload): Json<SetMirrorPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let base = crate::zap::mirror::save(&payload.pkg_mirror).map_err(|e| ZapError::New(-1, e))?;
    audit::log(Some(&claims), None, "system_mirror_set", &base, "").await;
    info!(mirror = %base, "包下载源已更新");
    Ok(Json(json!({ "code": 0, "data": { "pkgMirror": base } })))
}
