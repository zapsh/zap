//! 服务器运行环境管理。
//!
//! 状态存放于 `{data}/server_env.yaml`（见 [`crate::zap::server_env`]），分两层：
//! - `auto`：zapexec(root) 自动探测的快照（payload=整份 JSON），记录 detected 时间；
//! - `conf`：管理员手写的全局默认配置（webserver / php_default / database 等），
//!   供后续建站默认 PHP、SSL 签发等流程读取。
//!
//! 端点（均需管理员）：
//! - GET  /system/env             读快照+默认配置；快照超 60s 自动刷新
//! - POST /system/env/refresh     强制重测并写入 server_env.yaml
//! - POST /system/env/defaults    保存全局默认配置

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::Json;
use axum::extract::Extension;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;
use crate::zap::server_env;

/// 快照超过该秒数后在 GET 时自动重测。
const SNAPSHOT_STALE_SECS: i64 = 60;

// ── 内部存取 ────────────────────────────────────────────────

async fn probe_payload() -> Result<Value, ZapError> {
    server_env::probe().await.map_err(|e| ZapError::New(-1, e))
}

/// 保存探测快照（写入 server_env.yaml 的 auto 区，detected_at 即检测时间）。
fn save_snapshot(payload: &Value) -> i64 {
    server_env::save_snapshot(payload)
}

/// 读取快照 (payload, detected_at)。
fn load_snapshot() -> (Option<Value>, i64) {
    server_env::snapshot()
}

fn load_conf() -> HashMap<String, String> {
    server_env::conf_all().into_iter().collect()
}

/// 面板默认 PHP-FPM pool 规格（JSON 对象；用户未自定义时的兜底）。
pub fn default_fpm_spec() -> serde_json::Map<String, Value> {
    [
        ("pm", "dynamic"),
        ("max_children", "10"),
        ("start_servers", "3"),
        ("min_spare_servers", "2"),
        ("max_spare_servers", "5"),
        ("max_requests", "1000"),
        ("request_terminate_timeout", "300"),
        ("memory_limit", "256M"),
        ("post_max_size", "128M"),
        ("upload_max_filesize", "128M"),
        ("max_execution_time", "300"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
    .collect()
}

fn default_fpm_spec_json() -> String {
    serde_json::Value::Object(default_fpm_spec()).to_string()
}

/// 虚拟主机运行模式：**固定为独立系统用户**。
///
/// 每个面板用户对应一个 Linux 账号（nologin），家目录与站点目录归该账号，
/// PHP-FPM 以「每用户 × 每 PHP 版本」独立 pool 运行。
/// 历史上的「统一 www 用户」模式已移除，该常量仅用于向脚本注入 `ZAP_RUN_MODE`。
pub const VHOST_MODE: &str = "system";

fn conf_json(conf: &HashMap<String, String>) -> Value {
    json!({
        "webserver": conf.get("webserver").cloned().unwrap_or_default(),
        "php_default": conf.get("php_default").cloned().unwrap_or_default(),
        "database": conf.get("database").cloned().unwrap_or_default(),
        "fpm_pool_defaults": conf.get("fpm_pool_defaults").cloned().unwrap_or_else(default_fpm_spec_json),
        "user_home_root": conf.get("user_home_root").cloned().unwrap_or_else(|| "/home".into()),
    })
}

/// 拼装 GET / refresh 的返回体。
fn build_env_data(
    payload: Option<Value>,
    detected_at: i64,
    refreshed: bool,
    error: Option<String>,
) -> Value {
    let conf = load_conf();
    json!({
        "payload": payload.unwrap_or(Value::Null),
        "conf": conf_json(&conf),
        "detected_at": detected_at,
        "refreshed": refreshed,
        "error": error,
    })
}

// ── handlers ────────────────────────────────────────────────

/// GET /system/env：读快照；超时则自动重测。
pub async fn env_get(claims: ValidatedClaims) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可查看运行环境".to_string()));
    }
    let now = chrono::Local::now().timestamp();
    let (payload, detected_at) = load_snapshot();
    let stale = payload.is_none() || now - detected_at > SNAPSHOT_STALE_SECS;

    if stale {
        match probe_payload().await {
            Ok(v) => {
                let t = save_snapshot(&v);
                Ok(Json(
                    json!({ "code": 0, "data": build_env_data(Some(v), t, true, None) }),
                ))
            }
            Err(e) => {
                let msg = e.to_string();
                Ok(Json(
                    json!({ "code": 0, "data": build_env_data(payload, detected_at, false, Some(msg)) }),
                ))
            }
        }
    } else {
        Ok(Json(
            json!({ "code": 0, "data": build_env_data(payload, detected_at, false, None) }),
        ))
    }
}

/// POST /system/env/refresh：强制探测并刷新快照。
pub async fn env_refresh(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可刷新运行环境".to_string()));
    }
    let payload = probe_payload().await?;
    let t = save_snapshot(&payload);

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "env_refresh",
        "system",
        "手动刷新服务器运行环境快照",
    )
    .await;

    Ok(Json(
        json!({ "code": 0, "message": "运行环境已刷新", "data": build_env_data(Some(payload), t, true, None) }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct EnvDefaultsPayload {
    /// 默认 Web 服务器 flavor：空=跟随探测（auto）
    pub webserver: Option<String>,
    /// 默认 PHP 版本（如 8.3 / php83），建站新增站点时的预选值
    pub php_default: Option<String>,
    /// 默认数据库实例（如 mysql / mariadb）
    pub database: Option<String>,
    /// PHP-FPM 默认 pool 规格（JSON 字符串）
    pub fpm_pool_defaults: Option<String>,
    /// 用户家目录默认挂载点（如 /home /home2），新用户创建时的 home_dir 前缀
    pub user_home_root: Option<String>,
}

/// POST /system/env/defaults：保存全局默认配置（admin only）。
pub async fn env_defaults_save(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(payload): Json<EnvDefaultsPayload>,
) -> ZapJsonResult {
    if !is_admin(&claims) {
        return Err(ZapError::New(-1, "仅管理员可设置默认运行环境".to_string()));
    }

    let mut upserts: Vec<(String, String)> = Vec::new();
    for (key, val) in [
        ("webserver", payload.webserver),
        ("php_default", payload.php_default),
        ("database", payload.database),
    ] {
        if let Some(v) = val {
            let v = v.trim().to_string();
            if v.len() > 64 {
                return Err(ZapError::New(-1, format!("默认{key}长度超限")));
            }
            upserts.push((key.to_string(), v));
        }
    }
    if let Some(v) = payload.fpm_pool_defaults {
        let v = v.trim().to_string();
        if !v.is_empty() && serde_json::from_str::<Value>(&v).map_or(true, |x| !x.is_object()) {
            return Err(ZapError::New(
                -1,
                "fpm_pool_defaults 必须是 JSON 对象".to_string(),
            ));
        }
        upserts.push(("fpm_pool_defaults".to_string(), v));
    }
    if let Some(v) = payload.user_home_root {
        let v = v.trim().trim_end_matches('/').to_string();
        if v.is_empty()
            || !v.starts_with('/')
            || v.contains("..")
            || v.contains(char::is_whitespace)
            || v == "/"
        {
            return Err(ZapError::New(
                -1,
                "user_home_root 必须是合法挂载点绝对路径（如 /home 或 /home2）".to_string(),
            ));
        }
        upserts.push(("user_home_root".to_string(), v));
    }

    server_env::conf_set_many(&upserts, "面板默认配置");

    let conf = load_conf();
    let detail = upserts
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "env_defaults_save",
        "system",
        &detail,
    )
    .await;

    Ok(Json(
        json!({ "code": 0, "message": "默认配置已保存", "data": conf_json(&conf) }),
    ))
}
