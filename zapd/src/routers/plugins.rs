//! 插件 HTTP 接口（/api/plugin/*）。
//!
//! 复用应用商店的鉴权与站点归属校验：列表/运行前确认操作者身份与站点归属，
//! 再把解析后的上下文（家目录 / 站点 root / 站点 Linux 账号）通过白名单动词传给 zapexec。

use axum::extract::{Json, Query};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::json;
use sqlx;
use zap_proto::Request;

use crate::db;
use crate::zap::ZapError;
use crate::zap::jwt::ValidatedClaims;
use crate::routers::site;
use crate::zap::ZapJsonResult;

#[derive(Deserialize)]
pub struct PluginListQuery {
    #[serde(default)]
    pub slot: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Deserialize)]
pub struct PluginRunPayload {
    pub name: String,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub site_id: Option<i64>,
    #[serde(default)]
    pub options: std::collections::HashMap<String, String>,
}

pub async fn plugin_list(
    claims: ValidatedClaims,
    Query(q): Query<PluginListQuery>,
) -> ZapJsonResult {
    let (home, _linux_user) = load_user_home(claims.id as i64).await?;
    let resp = crate::zapexec::call(Request::PluginList {
        actor: claims.sub.clone(),
        home,
        slot: q.slot.clone(),
        scope: q.scope.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message, "data": resp.data })))
}

pub async fn plugin_run(
    claims: ValidatedClaims,
    Json(payload): Json<PluginRunPayload>,
) -> ZapJsonResult {
    let action = payload.action.unwrap_or_else(|| "run".into());
    let (home, _linux_user) = load_user_home(claims.id as i64).await?;
    let (site_root, site_linux_user) = match payload.site_id {
        Some(sid) => {
            site::site_in_scope(&claims, sid).await?;
            let (root, owner) = load_site_ctx(sid).await?;
            (Some(root), Some(owner))
        }
        None => (None, None),
    };
    let resp = crate::zapexec::call(Request::PluginRun {
        name: payload.name,
        actor: claims.sub.clone(),
        home,
        site_id: payload.site_id,
        site_root,
        site_linux_user,
        action,
        options: payload.options,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message, "data": resp.data })))
}

async fn load_user_home(uid: i64) -> Result<(String, String), ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(String, String)> =
        sqlx::query_as::<_, (String, String)>("SELECT home_dir, linux_user FROM user WHERE id = ?")
            .bind(uid)
            .fetch_optional(pool)
            .await?;
    let (mut home, lu) = row.ok_or_else(|| ZapError::New(-1, "用户不存在".to_string()))?;
    if home.trim().is_empty() {
        // 回退：用 Linux 账号的 OS home（getent passwd 第 6 字段），覆盖 home_dir 未配置的情况
        if let Some(h) = os_home_of(&lu).await {
            home = h;
        }
    }
    if home.trim().is_empty() {
        return Err(ZapError::New(-1, "用户家目录未配置".to_string()));
    }
    Ok((home, lu))
}

/// 取 Linux 账号的真实家目录（getent passwd 第 6 字段），用于 home_dir 为空时的回退。
async fn os_home_of(linux_user: &str) -> Option<String> {
    if linux_user.is_empty() {
        return None;
    }
    let out = tokio::process::Command::new("getent")
        .args(["passwd", linux_user])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let field = line.split(':').nth(5)?.trim().to_string();
    if field.is_empty() { None } else { Some(field) }
}

async fn load_site_ctx(site_id: i64) -> Result<(String, String), ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(String, String, String)> = sqlx::query_as::<_, (String, String, String)>(
        "SELECT s.web_root, u.linux_user, u.username \
         FROM site s JOIN user u ON u.id = s.user_id WHERE s.id = ?",
    )
    .bind(site_id)
    .fetch_optional(pool)
    .await?;
    let (web_root, linux_user, username) =
        row.ok_or_else(|| ZapError::New(-1, "站点不存在".to_string()))?;
    let owner = if linux_user.trim().is_empty() {
        username
    } else {
        linux_user
    };
    if owner.trim().is_empty() || owner == "root" {
        return Err(ZapError::New(-1, "站点无可用运行用户".to_string()));
    }
    Ok((web_root, owner))
}

pub fn routers() -> Router {
    Router::new()
        .route("/list", get(plugin_list))
        .route("/run", post(plugin_run))
}
