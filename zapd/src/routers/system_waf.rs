//! ModSecurity（WAF）端点（管理员）——**可选能力**
//!
//! 端点：
//! - GET  /system/waf/status        能力探测：装没装 / 缺什么 / 能不能自动装
//! - POST /system/waf/install       安装（长任务，返回 run_id 看编译日志）
//! - GET  /system/waf/conf/list     规则文件清单
//! - GET  /system/waf/conf/read     读取规则（Query: path）
//! - POST /system/waf/conf/save     保存规则（body: path, content）
//! - GET  /system/waf/audit         审计日志尾部（Query: lines）
//!
//! 与别的服务不同，WAF **可能压根装不上**（nginx 需要 `--with-compat` 才能加载
//! 动态模块）。所以这里不做任何"默认值兜底"：zapexec 判定未安装时，除 status
//! 外的动词一律返回错误，前端据此只显示安装引导，不给设置项 —— 宁可少一个按钮，
//! 也不给一个点了就报错的开关。

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{Extension, Query};
use serde::Deserialize;
use serde_json::json;

use crate::zap::ZapError;
use crate::zap::ZapJsonResult;
use crate::zap::audit;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::jwt::is_admin;
use zap_proto::Request;

fn require_admin(claims: &ValidatedClaims) -> Result<(), ZapError> {
    if is_admin(claims) {
        Ok(())
    } else {
        Err(ZapError::New(-1, "仅管理员可访问".to_string()))
    }
}

/// 执行一次 zapexec 请求，透传其 code/message/data。
async fn exec(req: Request) -> Result<Json<serde_json::Value>, ZapError> {
    let resp = crate::zapexec::call(req).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": "ok", "data": resp.data })))
}

/// GET /system/waf/status
pub async fn status(
    claims: ValidatedClaims,
    Extension(addr): Extension<SocketAddr>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let _ = addr;
    exec(Request::WafStatus).await
}

/// POST /system/waf/install：登记长任务（编译 libmodsecurity 是分钟级的）。
///
/// 先同步问一次 status：不可安装时直接把原因返回给前端，不起任务、不写日志。
pub async fn install(
    claims: ValidatedClaims,
    Extension(addr): Extension<SocketAddr>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let task_id = crate::zap::task::new_id();
    let log_path = crate::zap::task::log_path_in(&crate::zap::task::logs_dir(), &task_id);
    let payload = serde_json::to_string(&Request::WafInstall {
        log_path: log_path.clone(),
    })
    .map_err(|e| ZapError::New(-1, format!("任务参数序列化失败: {e}")))?;
    let t = crate::zap::task::enqueue(crate::zap::task::NewTask {
        task_id: task_id.clone(),
        kind: "waf".to_string(),
        action: "waf_install".to_string(),
        pkg: "ModSecurity".to_string(),
        username: claims.sub.clone(),
        title: "安装 ModSecurity（WAF）".to_string(),
        log_path,
        job_key: String::new(),
        // 编译只跑一个：并发编译会把机器压满
        group_key: "waf".to_string(),
        group_limit: 1,
        payload,
    })
    .await?;
    let queued = t.status == crate::zap::task::STATUS_PENDING;
    let position = if queued {
        crate::zap::task::queue_position(&t).await
    } else {
        0
    };
    // 拿到槽位就下发：zapexec 自己会再判一次可安装性，装不了就往日志写原因并收尾
    if !queued {
        crate::zap::task::launch(&t).await?;
    }
    audit::log(
        Some(&claims),
        Some(addr.ip().to_string().as_str()),
        "waf_install",
        "waf",
        "提交 WAF 安装任务",
    )
    .await;
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": { "task_id": task_id, "run_id": task_id, "queued": queued, "position": position },
    })))
}

/// GET /system/waf/conf/list
pub async fn conf_list(
    claims: ValidatedClaims,
    Extension(addr): Extension<SocketAddr>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let _ = addr;
    exec(Request::WafConfList).await
}

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub path: String,
}

/// GET /system/waf/conf/read?path=...
pub async fn conf_read(
    claims: ValidatedClaims,
    Extension(addr): Extension<SocketAddr>,
    Query(q): Query<PathQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let _ = addr;
    exec(Request::WafConfRead { path: q.path }).await
}

#[derive(Debug, Deserialize)]
pub struct SaveBody {
    pub path: String,
    pub content: String,
}

/// POST /system/waf/conf/save
pub async fn conf_save(
    claims: ValidatedClaims,
    Extension(addr): Extension<SocketAddr>,
    Json(body): Json<SaveBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let result = exec(Request::WafConfSave {
        path: body.path.clone(),
        content: body.content,
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(addr.ip().to_string().as_str()),
            "waf_conf_save",
            "waf",
            &format!("保存 WAF 规则 {}", body.path),
        )
        .await;
    }
    result
}

#[derive(Debug, Deserialize)]
pub struct EngineBody {
    pub mode: String,
}

/// POST /system/waf/engine：切换规则引擎形态（On 拦截 / DetectionOnly 只检测 / Off）。
///
/// 引擎形态直接决定「拦不拦业务请求」，所以每次切换都进审计。
pub async fn engine(
    claims: ValidatedClaims,
    Extension(addr): Extension<SocketAddr>,
    Json(body): Json<EngineBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let result = exec(Request::WafSetEngine {
        mode: body.mode.clone(),
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(addr.ip().to_string().as_str()),
            "waf_engine",
            "waf",
            &format!("切换 WAF 规则引擎为 {}", body.mode),
        )
        .await;
    }
    result
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub lines: Option<u32>,
}

/// GET /system/waf/audit?lines=200
pub async fn audit_log(
    claims: ValidatedClaims,
    Extension(addr): Extension<SocketAddr>,
    Query(q): Query<AuditQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let _ = addr;
    exec(Request::WafAudit {
        lines: q.lines.unwrap_or(200).clamp(1, 2000),
    })
    .await
}
