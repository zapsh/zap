// SPDX-License-Identifier: AGPL-3.0-only
//! 计划任务（crontab）：面板用户管理自己的定时任务。
//!
//! - **所有角色可用**：路由挂 `/terminal/crontab/*`（`Required::User`）
//! - **执行身份**：admin 可指定（`/terminal/crontab/exec-users` 仅 admin）；
//!   其他角色由 `user_cron::resolve_exec_user` 强制收敛为自身 `linux_user`
//! - **归属**：默认只操作 `claims.sub` 对应的 `crontab.yaml`；admin 可通过
//!   `?username=` 查看他人任务（写操作不开放越权，避免误改他人环境）
//! - **演示账号**：已被 `demo_readonly_guard` 限为只读（仅 GET）

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::{Json, extract::Extension, extract::Query};
use serde::Deserialize;
use serde_json::json;

use crate::zap::{
    ZapError, ZapJsonResult, appstore as ast, audit,
    jwt::{ValidatedClaims, is_admin},
    user_cron,
};

/// 解析操作目标用户名：默认本人；admin 可显式指定他人（只读场景）。
fn target_username(
    claims: &ValidatedClaims,
    requested: Option<&String>,
) -> Result<String, ZapError> {
    match requested.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(u) if is_admin(claims) => Ok(u.to_string()),
        Some(_) => Err(ZapError::New(
            -1,
            "仅管理员可操作其他用户的任务".to_string(),
        )),
        None => Ok(claims.sub.clone()),
    }
}

#[derive(Debug, Deserialize)]
pub struct CronJobPayload {
    pub name: String,
    pub schedule: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    pub command: String,
    #[serde(default)]
    pub exec_user: String,
    #[serde(default)]
    pub remark: String,
}

#[derive(Debug, Deserialize)]
pub struct CronUpdatePayload {
    pub id: String,
    pub name: String,
    pub schedule: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    pub command: String,
    #[serde(default)]
    pub exec_user: String,
    #[serde(default)]
    pub remark: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct CronIdPayload {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct CronTogglePayload {
    pub id: String,
    pub enabled: bool,
}

fn default_kind() -> String {
    "command".to_string()
}

fn default_true() -> bool {
    true
}

/// GET /terminal/crontab/list[?username=]
pub async fn cron_list(
    claims: ValidatedClaims,
    Query(q): Query<HashMap<String, String>>,
) -> ZapJsonResult {
    let username = target_username(&claims, q.get("username"))?;
    let jobs = user_cron::list(&username).await?;
    let can_choose_exec = is_admin(&claims);
    Ok(Json(json!({
        "code": 0,
        "data": {
            "username": username,
            "can_choose_exec": can_choose_exec,
            "exec_user": own_exec_user(&claims).await,
            "jobs": jobs,
        }
    })))
}

/// 当前登录用户自身的 Linux 账号（前端只读展示用）。
async fn own_exec_user(claims: &ValidatedClaims) -> String {
    let pool = crate::db::get_db_pool().await;
    sqlx::query_scalar::<_, String>(
        "SELECT linux_user FROM user WHERE id = ? AND linux_user <> '' LIMIT 1",
    )
    .bind(claims.id as i64)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// GET /terminal/crontab/exec-users（仅 admin）
pub async fn cron_exec_users(claims: ValidatedClaims) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "权限不足，仅管理员可使用".to_string()));
    }
    let users = user_cron::list_exec_users().await?;
    crate::zap::api_ok(json!({ "users": users }))
}

/// POST /terminal/crontab/add
pub async fn cron_add(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CronJobPayload>,
) -> ZapJsonResult {
    let username = claims.sub.clone();
    let exec_user = user_cron::resolve_exec_user(&claims, &payload.exec_user).await?;
    let kind = payload.kind.trim().to_string();
    let id = user_cron::add(
        &username,
        payload.name.trim(),
        payload.schedule.trim(),
        &kind,
        payload.command.trim(),
        &exec_user,
        payload.remark.trim(),
    )
    .await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "crontab_add",
        &payload.name,
        &format!("{}:{}:{}", payload.schedule, kind, exec_user),
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "计划任务已创建", "data": { "id": id } }),
    ))
}

/// POST /terminal/crontab/update
pub async fn cron_update(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CronUpdatePayload>,
) -> ZapJsonResult {
    let username = claims.sub.clone();
    let exec_user = user_cron::resolve_exec_user(&claims, &payload.exec_user).await?;
    let kind = payload.kind.trim().to_string();
    user_cron::update(
        &username,
        payload.id.trim(),
        payload.name.trim(),
        payload.schedule.trim(),
        &kind,
        payload.command.trim(),
        &exec_user,
        payload.remark.trim(),
        payload.enabled,
    )
    .await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "crontab_update",
        &payload.name,
        &format!("{}:{}:{}", payload.schedule, kind, exec_user),
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "已保存" })))
}

/// POST /terminal/crontab/delete
pub async fn cron_delete(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CronIdPayload>,
) -> ZapJsonResult {
    let username = claims.sub.clone();
    let removed = user_cron::delete(&username, payload.id.trim()).await?;
    if let Some(job) = removed {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "crontab_delete",
            &job.name,
            &job.command,
        )
        .await;
    }
    Ok(Json(json!({ "code": 0, "message": "已删除" })))
}

/// POST /terminal/crontab/toggle
pub async fn cron_toggle(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CronTogglePayload>,
) -> ZapJsonResult {
    let username = claims.sub.clone();
    user_cron::toggle(&username, payload.id.trim(), payload.enabled).await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        if payload.enabled {
            "crontab_enable"
        } else {
            "crontab_disable"
        },
        &payload.id,
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "已更新" })))
}

/// POST /terminal/crontab/run_now
pub async fn cron_run_now(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CronIdPayload>,
) -> ZapJsonResult {
    let username = claims.sub.clone();
    let id = payload.id.trim().to_string();
    let run_id = user_cron::run_now(&username, &id).await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "crontab_run_now",
        &id,
        &run_id,
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": "已触发运行",
        "data": { "run_id": run_id }
    })))
}

/// GET /terminal/crontab/log[?run_id=&username=]
pub async fn cron_log(
    claims: ValidatedClaims,
    Query(q): Query<HashMap<String, String>>,
) -> ZapJsonResult {
    let username = target_username(&claims, q.get("username"))?;
    let run_id = q.get("run_id").cloned().unwrap_or_default();
    let (content, done) = user_cron::read_log(&username, &run_id).await?;
    Ok(Json(json!({
        "code": 0,
        "data": { "log": content, "done": done.is_some(), "exit_code": done }
    })))
}

/// GET /terminal/crontab/runs[?id=&username=] —— 某任务最近的运行历史
pub async fn cron_runs(
    claims: ValidatedClaims,
    Query(q): Query<HashMap<String, String>>,
) -> ZapJsonResult {
    let username = target_username(&claims, q.get("username"))?;
    let id = q.get("id").cloned().unwrap_or_default();
    if id.trim().is_empty() {
        return Err(ZapError::New(-1, "缺少任务 id".to_string()));
    }
    let runs = user_cron::list_runs(&username, id.trim()).await?;
    Ok(Json(json!({
        "code": 0,
        "data": { "runs": runs, "keep": ast::MAX_RUNS_PER_JOB }
    })))
}

/// POST /terminal/crontab/runs_clear —— 清空某任务的运行历史
pub async fn cron_runs_clear(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<CronIdPayload>,
) -> ZapJsonResult {
    let username = claims.sub.clone();
    let id = payload.id.trim();
    let job = user_cron::get(&username, id)
        .await?
        .ok_or_else(|| ZapError::New(-1, "计划任务不存在".to_string()))?;
    let n = user_cron::clear_runs(&username, id).await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "crontab_runs_clear",
        &job.name,
        &n.to_string(),
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": "已清空运行历史",
        "data": { "deleted": n }
    })))
}

/// POST /terminal/crontab/logs_purge —— 清理无归属的历史遗留日志
pub async fn cron_logs_purge(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
) -> ZapJsonResult {
    let username = claims.sub.clone();
    let n = user_cron::purge_orphan_logs(&username).await?;
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "crontab_logs_purge",
        &username,
        &n.to_string(),
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": "已清理历史遗留日志",
        "data": { "deleted": n }
    })))
}
