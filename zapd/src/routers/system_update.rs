//! 系统更新路由（系统设置 → 系统更新，admin only）：
//! 状态（版本 / 自动更新配置 / 历史）/ 保存配置 / 检查更新 / 触发升级 / 升级日志。

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{Extension, Path, Query},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    zap::{
        ZapError, ZapJsonResult, appstore as ast, audit, auto_update,
        jwt::{self, Claims, ValidatedClaims},
        updater,
    },
    zapexec,
};
use zap_proto::Request;

fn require_admin(claims: &Claims) -> Result<(), ZapError> {
    if jwt::is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "权限不足，需要管理员权限".to_string()))
    }
}

/// GET /system/update/status
///
/// 只读（门禁：登录用户，见 `access.rs`）：非管理员可以看版本与升级历史，
/// 但检查更新 / 立即升级 / 改配置仍要 admin。
pub async fn status_get(_claims: ValidatedClaims) -> ZapJsonResult {
    // 惰性收尾：zapd 若在升级中被重启，遗留的 running 记录在此补全
    updater::finalize_stale_updates().await;
    let cfg = updater::load_config();
    let zapexec_version = match zapexec::call(Request::UpgradeInfo).await {
        Ok(r) if r.code == 0 => r
            .data
            .as_ref()
            .and_then(|d| d.get("version"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    let current_run = updater::running_run().await;
    let recent_runs = updater::recent_update_runs(10).await;
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "zapd_version": updater::current_zapd_version(),
            "zapexec_version": zapexec_version,
            "config": {
                "auto": cfg.auto != 0,
                "cron": cfg.cron,
                "channel": cfg.channel,
                "last_check_at": cfg.last_check_at,
                "last_check_version": cfg.last_check_version,
                "last_check_has_update": cfg.last_check_has_update != 0,
                "last_error": cfg.last_error,
            },
            "upgrading": current_run.is_some()
                || updater::UPGRADING.load(std::sync::atomic::Ordering::SeqCst),
            "current_run": current_run
                .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
                .unwrap_or(Value::Null),
            "recent_runs": recent_runs,
        }
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateConfigPayload {
    pub auto: bool,
    pub cron: String,
    pub channel: String,
}

/// POST /system/update/config
pub async fn config_save(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<UpdateConfigPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let cron = payload.cron.trim().to_string();
    auto_update::validate_cron(&cron).map_err(|msg| ZapError::New(-1, msg))?;
    let channel = payload.channel.trim().trim_end_matches('/').to_string();
    if !(channel.starts_with("http://") || channel.starts_with("https://")) {
        return Err(ZapError::New(
            -1,
            "更新渠道需为 http(s):// 地址".to_string(),
        ));
    }
    updater::save_config(payload.auto, &cron, &channel)?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "system_update_config",
        &format!("auto={}", payload.auto),
        &format!("cron={cron} channel={channel}"),
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": "自动更新配置已保存",
        "data": {}
    })))
}

/// POST /system/update/check
pub async fn check(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let cfg = updater::load_config();
    let current = updater::current_zapd_version();
    let latest = match updater::check_remote_version(&cfg.channel).await {
        Ok(v) => v,
        Err(e) => {
            updater::record_check("", false, &e.to_string());
            return Err(e);
        }
    };
    let has_update = updater::has_update(current, &latest);
    updater::record_check(&latest, has_update, "");
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "system_update_check",
        &latest,
        &format!("current={current} has_update={has_update}"),
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": { "current": current, "latest": latest, "has_update": has_update }
    })))
}

/// POST /system/update/apply
pub async fn apply(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let username = claims.sub.clone();
    let info = updater::launch_update(&username).await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "system_update_apply",
        &info.latest,
        &info.run_id,
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": "升级已启动",
        "data": {
            "run_id": info.run_id,
            "log_path": info.log_path,
            "latest": info.latest
        }
    })))
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub offset: Option<u64>,
}

/// GET /system/update/log/{run_id}
///
/// 只读（门禁：登录用户）：与 status 一致，看升级日志不算「执行更新」。
/// 仍只认升级记录（`action = zap_update`），拿不到别的任务的日志。
pub async fn log(
    _claims: ValidatedClaims,
    Path(run_id): Path<String>,
    Query(q): Query<LogQuery>,
) -> ZapJsonResult {
    let run = match ast::get_run(&run_id).await {
        Ok(Some(r)) => r,
        _ => return Err(ZapError::New(-1, "升级运行记录不存在".to_string())),
    };
    if run.action != updater::ACTION_ZAP_UPDATE {
        return Err(ZapError::New(-1, "该运行记录不是系统升级".to_string()));
    }
    let (mut text, exit_code, done) = ast::read_log(&run.log_path, q.offset.unwrap_or(0)).await?;
    if done {
        if let Some(idx) = text.rfind(ast::DONE_MARKER) {
            text.truncate(idx);
        }
        text = text.trim_end().to_string();
    }
    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "run_id": run_id,
            "log": text,
            "offset": q.offset.unwrap_or(0) + text.len() as u64,
            "done": done,
            "exit_code": exit_code,
            "status": run.status,
        }
    })))
}
