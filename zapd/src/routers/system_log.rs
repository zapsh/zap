// SPDX-License-Identifier: AGPL-3.0-only
//! 系统日志查看器路由（服务器状态 → 日志查看器，admin only）。
//!
//! 两个接口：
//! - `GET /system/logs/sources`：列出可查看的日志源（内核 / 系统 / nginx / php /
//!   mysql / 面板 / 已安装服务）；
//! - `GET /system/logs/read`：按游标读某个日志源的内容（首次取末尾 N 行，之后增量）。
//!
//! 读取本身由 zapexec 以 root 完成：journald 与 `/var/log` 下的服务日志通常
//! 对面板进程不可读，直接读会静默为空（详见 `zapexec::verbs::sys_log`）。
//! 这里只做门禁与转发，**不对路径做白名单**——因为日志源 id 由 zapexec 探测
//! 产出，且管理员本就对所有日志有查看权。

use axum::{Json, extract::Query};
use serde::Deserialize;
use serde_json::json;

use crate::{
    zap::{
        ZapError, ZapJsonResult,
        jwt::{self, Claims, ValidatedClaims},
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

#[derive(Deserialize)]
pub struct LogReadQuery {
    /// 日志源 id：`journal:<unit>` / `journal:` / `kernel:` / `file:<path>`
    source: Option<String>,
    /// 增量游标；空 = 首次，取末尾 `lines` 行
    #[serde(default)]
    cursor: String,
    /// 首次读取的行数（0 = 用 zapexec 侧默认 200）
    lines: Option<u32>,
    /// 关键字过滤（大小写不敏感）
    #[serde(default)]
    keyword: String,
}

/// GET /system/logs/sources
pub async fn sources(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = zapexec::call(Request::SysLogSources).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": resp.data }),
    ))
}

#[derive(Deserialize)]
pub struct CustomAddPayload {
    /// 展示名；留空则用文件名
    #[serde(default)]
    name: String,
    path: String,
}

#[derive(Deserialize)]
pub struct CustomRemovePayload {
    path: String,
}

fn zapexec_err(resp: zap_proto::Response) -> ZapError {
    ZapError::New(resp.code, resp.message)
}

/// GET /system/logs/custom —— 手动添加的自定义日志路径
pub async fn custom_list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = zapexec::call(Request::SysLogCustomList).await?;
    if resp.code != 0 {
        return Err(zapexec_err(resp));
    }
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": resp.data }),
    ))
}

/// POST /system/logs/custom/add
pub async fn custom_add(
    claims: ValidatedClaims,
    Json(payload): Json<CustomAddPayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = zapexec::call(Request::SysLogCustomAdd {
        name: payload.name,
        path: payload.path,
    })
    .await?;
    if resp.code != 0 {
        return Err(zapexec_err(resp));
    }
    Ok(Json(json!({ "code": 0, "message": "OK" })))
}

/// POST /system/logs/custom/remove
pub async fn custom_remove(
    claims: ValidatedClaims,
    Json(payload): Json<CustomRemovePayload>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let resp = zapexec::call(Request::SysLogCustomRemove { path: payload.path }).await?;
    if resp.code != 0 {
        return Err(zapexec_err(resp));
    }
    Ok(Json(json!({ "code": 0, "message": "OK" })))
}

/// GET /system/logs/read?source=...&cursor=...&lines=...&keyword=...
pub async fn read(claims: ValidatedClaims, Query(q): Query<LogReadQuery>) -> ZapJsonResult {
    require_admin(&claims)?;
    let source = q.source.unwrap_or_default().trim().to_string();
    if source.is_empty() {
        return Err(ZapError::New(-1, "缺少日志源".to_string()));
    }
    let resp = zapexec::call(Request::SysLogRead {
        source,
        cursor: q.cursor,
        lines: q.lines.unwrap_or(0),
        keyword: q.keyword,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(
        json!({ "code": 0, "message": "OK", "data": resp.data }),
    ))
}
