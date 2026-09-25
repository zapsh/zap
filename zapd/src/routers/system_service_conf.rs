//! 通用服务配置端点（「服务配置」大类：php / mysql(MySQL/MariaDB 合一) / docker …）。
//!
//! 端点（均需管理员，均透传 zapexec 返回）：
//! - GET  /system/service-conf/status           状态探测（Query: service）
//! - GET  /system/service-conf/list             列出可编辑配置（Query: service）
//! - GET  /system/service-conf/read             读取配置内容（Query: service, path）
//! - POST /system/service-conf/save             保存配置（body: service, path, content）
//! - GET  /system/service-conf/keys             关键项表单（Query: service）
//! - POST /system/service-conf/keys/save        保存关键项（body: service, keys）
//! - POST /system/service-conf/control          服务控制（body: service, action）

use std::collections::BTreeMap;
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
    Ok(Json(
        json!({ "code": 0, "message": "ok", "data": resp.data }),
    ))
}

/// yaml 注册的服务 key 缓存：新增一份服务定义后重启 zapd 生效。
///
/// 服务定义在 zapexec 侧（内置 yaml + /etc/zap/services 覆盖），zapd 只能通过
/// 一次 exec 拿到清单；这里进程内缓存一次，避免每次读写配置都多打一次 exec。
static SERVICE_KEYS: tokio::sync::OnceCell<Vec<String>> = tokio::sync::OnceCell::const_new();

/// 取 exec 侧注册的服务 key 列表（失败时回退内置三项，不至于整页不可用）。
async fn registered_services() -> &'static Vec<String> {
    SERVICE_KEYS
        .get_or_init(|| async {
            match crate::zapexec::call(Request::ServiceConfDefs).await {
                Ok(resp) if resp.code == 0 => resp
                    .data
                    .as_ref()
                    .and_then(|d| d.get("items"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|it| it.get("key").and_then(|k| k.as_str()))
                            .map(|k| k.to_string())
                            .collect()
                    })
                    .unwrap_or_else(|| fallback_keys()),
                _ => fallback_keys(),
            }
        })
        .await
}

fn fallback_keys() -> Vec<String> {
    vec![
        "php".to_string(),
        "mysql".to_string(),
        "docker".to_string(),
    ]
}

/// 校验服务名：必须在 exec 侧 yaml 注册的服务里（PHP 版本实例额外放行）。
///
/// 服务清单来自 yaml 注册，这里是异步的（第一次调用拉一次 exec 并缓存）。
async fn validate_service(service: &str) -> Result<String, ZapError> {
    // 服务名会拼进 shell 命令与文件路径，字符先收紧
    let well_formed = !service.is_empty()
        && service.len() <= 32
        && service
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    // PHP 多版本实例 svc：php74 / php81 …（zapexec 按实例定位配置/unit）
    let php_inst = is_php_instance_svc(service);
    let known = registered_services().await.iter().any(|k| k == service);
    if well_formed && (known || php_inst) {
        Ok(service.to_string())
    } else {
        Err(ZapError::New(-1, "不支持的服务类型".to_string()))
    }
}

/// 是否为 PHP 版本实例 svc（php74 / php81 …，排除类型级 "php"）。
fn is_php_instance_svc(service: &str) -> bool {
    service
        .strip_prefix("php")
        .is_some_and(|r| !r.is_empty() && r.len() <= 3 && r.chars().all(|c| c.is_ascii_digit()))
}

#[derive(Debug, Deserialize)]
pub struct ServiceQuery {
    pub service: String,
}

/// GET /system/service-conf/instances
pub async fn instances(claims: ValidatedClaims, Query(q): Query<ServiceQuery>) -> ZapJsonResult {
    require_admin(&claims)?;
    let service = validate_service(&q.service).await?;
    exec(Request::ServiceConfInstances { service }).await
}

#[derive(Debug, Deserialize)]
pub struct ServiceDefaultBody {
    pub service: String,
    pub enable: bool,
}

/// POST /system/service-conf/default
pub async fn set_default(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<ServiceDefaultBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let service = validate_service(&body.service).await?;
    if !is_php_instance_svc(&service) {
        return Err(ZapError::New(
            -1,
            "全局默认访问仅支持 PHP 版本实例（php74 / php81 …）".to_string(),
        ));
    }
    let result = exec(Request::ServiceConfDefault {
        service,
        enable: body.enable,
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "service_conf_default",
            "service-conf",
            &format!(
                "{} PHP 实例 {} 的全局默认访问",
                if body.enable { "开启" } else { "取消" },
                body.service
            ),
        )
        .await;
    }
    result
}

/// GET /system/service-conf/defs：yaml 注册的服务定义清单（前端动态生成入口用）
pub async fn defs_list(claims: ValidatedClaims) -> ZapJsonResult {
    require_admin(&claims)?;
    exec(Request::ServiceConfDefs).await
}

/// GET /system/service-conf/status
pub async fn status(claims: ValidatedClaims, Query(q): Query<ServiceQuery>) -> ZapJsonResult {
    require_admin(&claims)?;
    let service = validate_service(&q.service).await?;
    exec(Request::ServiceConfStatus { service }).await
}

/// GET /system/service-conf/list
pub async fn conf_list(claims: ValidatedClaims, Query(q): Query<ServiceQuery>) -> ZapJsonResult {
    require_admin(&claims)?;
    let service = validate_service(&q.service).await?;
    exec(Request::ServiceConfList { service }).await
}

#[derive(Debug, Deserialize)]
pub struct ServiceReadQuery {
    pub service: String,
    pub path: String,
}

/// GET /system/service-conf/read?service=..&path=..
pub async fn conf_read(
    claims: ValidatedClaims,
    Query(q): Query<ServiceReadQuery>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let service = validate_service(&q.service).await?;
    exec(Request::ServiceConfRead {
        service,
        path: q.path,
    })
    .await
}

#[derive(Debug, Deserialize)]
pub struct ServiceSaveBody {
    pub service: String,
    pub path: String,
    pub content: String,
}

/// POST /system/service-conf/save
pub async fn conf_save(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<ServiceSaveBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let service = validate_service(&body.service).await?;
    let result = exec(Request::ServiceConfSave {
        service,
        path: body.path.clone(),
        content: body.content.clone(),
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "service_conf_save",
            "service-conf",
            &format!("保存 {} 配置文件 {}", body.service, body.path),
        )
        .await;
    }
    result
}

/// GET /system/service-conf/keys
pub async fn keys_get(claims: ValidatedClaims, Query(q): Query<ServiceQuery>) -> ZapJsonResult {
    require_admin(&claims)?;
    let service = validate_service(&q.service).await?;
    exec(Request::ServiceConfKeys { service }).await
}

#[derive(Debug, Deserialize)]
pub struct ServiceKeysSaveBody {
    pub service: String,
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
}

/// POST /system/service-conf/keys/save
pub async fn keys_save(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<ServiceKeysSaveBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let service = validate_service(&body.service).await?;
    let result = exec(Request::ServiceConfKeysSave {
        service,
        keys: body.keys.clone(),
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "service_conf_keys_save",
            "service-conf",
            &format!("保存 {} 关键配置", body.service),
        )
        .await;
    }
    result
}

#[derive(Debug, Deserialize)]
pub struct ServiceControlBody {
    pub service: String,
    pub action: String,
}

/// POST /system/service-conf/control
pub async fn control(
    claims: ValidatedClaims,
    client_addr: Extension<SocketAddr>,
    Json(body): Json<ServiceControlBody>,
) -> ZapJsonResult {
    require_admin(&claims)?;
    let service = validate_service(&body.service).await?;
    if !matches!(
        body.action.as_str(),
        "start" | "stop" | "restart" | "reload"
    ) {
        return Err(ZapError::New(
            -1,
            "仅支持 start / stop / restart / reload".to_string(),
        ));
    }
    let result = exec(Request::ServiceConfControl {
        service,
        action: body.action.clone(),
    })
    .await;
    if result.is_ok() {
        audit::log(
            Some(&claims),
            Some(client_addr.ip().to_string().as_str()),
            "service_conf_control",
            "service-conf",
            &format!("{} 服务操作 {}", body.service, body.action),
        )
        .await;
    }
    result
}
