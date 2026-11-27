// SPDX-License-Identifier: AGPL-3.0-only
//! 多节点管理：受管节点注册 + 跨节点监控查询。
//!
//! 凭据模型（与「共享 JWT」方案相对）：每台节点存一个**该节点自己签发的 API Token**
//! （`zap_` 开头），加密落库、接口永不回传原文。好处是单节点可独立吊销、
//! 有 `last_used_at` 审计，且一个节点泄露不影响其它节点。
//!
//! 采集只打节点的 `/api/system/info`（节点侧 `Required::User`），所以 Token 可以
//! 来自普通用户，不必在节点上开管理员；历史曲线全部由主控采样落库，查询不再访问节点。

use axum::Json;
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::db;
use crate::zap::{ZapError, ZapJsonResult, audit, crypto, jwt::ValidatedClaims, node};

#[derive(Deserialize)]
pub struct AddPayload {
    pub name: String,
    pub base_url: String,
    /// 节点上签发的 API Token（`zap_...`）
    pub api_token: String,
    #[serde(default)]
    pub remark: Option<String>,
    /// 容忍自签证书，默认 true
    #[serde(default)]
    pub insecure_tls: Option<bool>,
}

#[derive(Deserialize)]
pub struct UpdatePayload {
    pub id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    /// 留空 = 不修改已存 Token
    #[serde(default)]
    pub api_token: Option<String>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub insecure_tls: Option<bool>,
    #[serde(default)]
    pub status: Option<i64>,
}

#[derive(Deserialize)]
pub struct IdsPayload {
    pub ids: Vec<i64>,
}

#[derive(Deserialize)]
pub struct TestPayload {
    pub base_url: String,
    #[serde(default)]
    pub api_token: Option<String>,
    #[serde(default)]
    pub insecure_tls: Option<bool>,
    /// 编辑态测试：Token 留空时复用库里已存的密文
    #[serde(default)]
    pub id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct NodeRow {
    id: i64,
    name: String,
    base_url: String,
    remark: String,
    insecure_tls: i64,
    status: i64,
    online: i64,
    last_seen: i64,
    last_error: String,
    os_name: String,
    host_name: String,
    version: String,
    public_ip: String,
    uptime: String,
    cpu_num: i64,
    memory_total_b: i64,
    disk_total_b: i64,
    disk_json: String,
    // 这两列建表时没带 NOT NULL，用 Option 接，避免脏行让整张列表查不出来
    created_at: Option<i64>,
    updated_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct OverviewRow {
    id: i64,
    name: String,
    base_url: String,
    remark: String,
    status: i64,
    online: i64,
    last_seen: i64,
    last_error: String,
    os_name: String,
    host_name: String,
    version: String,
    public_ip: String,
    uptime: String,
    cpu_num: i64,
    memory_total_b: i64,
    disk_total_b: i64,
    disk_json: String,
    cpu_usage: Option<f64>,
    memory_usage: Option<f64>,
    swap_usage: Option<f64>,
    disk_usage: Option<f64>,
    loadavg_one: Option<f64>,
    loadavg_five: Option<f64>,
    loadavg_fifteen: Option<f64>,
    sampled_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct PointRow {
    created_at: Option<i64>,
    cpu_usage: Option<f64>,
    memory_usage: Option<f64>,
    swap_usage: Option<f64>,
    disk_usage: Option<f64>,
    loadavg_one: Option<f64>,
    loadavg_five: Option<f64>,
    loadavg_fifteen: Option<f64>,
}

fn err(msg: impl Into<String>) -> ZapError {
    ZapError::New(-1, msg.into())
}

/// `disk_json` 存的是字符串，出接口时还原成数组，前端不用再 JSON.parse。
fn disks_to_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!([]))
}

/// 节点列表（不含 Token）。
pub async fn list(_: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let rows: Vec<NodeRow> = sqlx::query_as(
        "SELECT id, name, base_url, remark, insecure_tls, status, online, last_seen, \
         last_error, os_name, host_name, version, public_ip, uptime, cpu_num, \
         memory_total_b, disk_total_b, disk_json, created_at, updated_at \
         FROM nodes ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await?;
    let data: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id, "name": r.name, "base_url": r.base_url, "remark": r.remark,
                "insecure_tls": r.insecure_tls == 1, "status": r.status,
                "online": r.online == 1, "last_seen": r.last_seen,
                "last_error": r.last_error, "os_name": r.os_name, "host_name": r.host_name,
                "version": r.version, "public_ip": r.public_ip, "uptime": r.uptime,
                "cpu_num": r.cpu_num, "memory_total_b": r.memory_total_b,
                "disk_total_b": r.disk_total_b, "disks": disks_to_value(&r.disk_json),
                "created_at": r.created_at, "updated_at": r.updated_at,
            })
        })
        .collect();
    Ok(Json(json!({"code": 0, "message": "OK", "data": data})))
}

/// 添加节点：先实测连通再落库，避免存一个连不上的地址。
pub async fn add(claims: ValidatedClaims, Json(p): Json<AddPayload>) -> Json<Value> {
    let name = p.name.trim();
    if name.is_empty() {
        return Json(json!({"code": -1, "message": "名称不能为空"}));
    }
    let base = match node::normalize_base(&p.base_url) {
        Ok(b) => b,
        Err(e) => return Json(json!({"code": -1, "message": e})),
    };
    let insecure = p.insecure_tls.unwrap_or(true);
    let snap = match node::test_connection(&base, &p.api_token, insecure).await {
        Ok(s) => s,
        Err(e) => return Json(json!({"code": -1, "message": format!("节点连接失败：{}", e)})),
    };

    let pool = db::get_db_pool().await;
    let enc = match crypto::encrypt_password(p.api_token.trim()) {
        Ok(v) => v,
        Err(e) => return Json(json!({"code": -1, "message": e})),
    };
    let now = chrono::Local::now().timestamp();
    let r = sqlx::query(
        "INSERT INTO nodes (name, base_url, api_token, remark, insecure_tls, status, online, \
         last_seen, last_error, os_name, host_name, version, public_ip, uptime, cpu_num, \
         memory_total_b, disk_total_b, disk_json, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,1,1,$6,'',$7,$8,$9,$10,$11,$12,$13,$14,$15,$6,$6)",
    )
    .bind(name)
    .bind(&base)
    .bind(&enc)
    .bind(p.remark.unwrap_or_default())
    .bind(insecure as i64)
    .bind(now)
    .bind(&snap.os_name)
    .bind(&snap.host_name)
    .bind(&snap.version)
    .bind(&snap.public_ip)
    .bind(&snap.uptime)
    .bind(snap.cpu_num)
    .bind(snap.memory_total_b)
    .bind(snap.disk_total_b)
    .bind(&snap.disk_json)
    .execute(pool)
    .await;

    match r {
        Ok(a) => {
            audit::log(Some(&claims), None, "node_create", &base, name).await;
            Json(json!({"code": 0, "message": "OK", "data": {"id": a.last_insert_rowid()}}))
        }
        Err(e) => {
            let msg = if format!("{}", e).contains("UNIQUE") {
                "节点名称或地址已存在".to_string()
            } else {
                format!("保存失败: {}", e)
            };
            Json(json!({"code": -1, "message": msg}))
        }
    }
}

pub async fn update(claims: ValidatedClaims, Json(p): Json<UpdatePayload>) -> Json<Value> {
    let pool = db::get_db_pool().await;
    let exist: Option<(String, String)> =
        sqlx::query_as("SELECT name, base_url FROM nodes WHERE id = ?")
            .bind(p.id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
    let Some((old_name, old_base)) = exist else {
        return Json(json!({"code": -1, "message": "节点不存在"}));
    };

    let base = match p.base_url.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => match node::normalize_base(v) {
            Ok(b) => Some(b),
            Err(e) => return Json(json!({"code": -1, "message": e})),
        },
        _ => None,
    };
    let new_name = p.name.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let new_token = p
        .api_token
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let enc_token = match new_token {
        Some(t) => match crypto::encrypt_password(t) {
            Ok(enc) => Some(enc),
            Err(e) => return Json(json!({"code": -1, "message": e})),
        },
        None => None,
    };
    // 全列 COALESCE：没传的字段传 NULL 即保持原值。
    // 不用拼接动态 SET —— sqlx 会拦 dynamic SQL，且字面量更好审。
    let q = sqlx::query(
        "UPDATE nodes SET name = COALESCE($1, name), base_url = COALESCE($2, base_url), \
         remark = COALESCE($3, remark), insecure_tls = COALESCE($4, insecure_tls), \
         status = COALESCE($5, status), api_token = COALESCE($6, api_token), \
         updated_at = $7 WHERE id = $8",
    )
    .bind(new_name.map(str::to_string))
    .bind(base.clone())
    .bind(p.remark.clone())
    .bind(p.insecure_tls.map(|v| v as i64))
    .bind(p.status)
    .bind(enc_token)
    .bind(chrono::Local::now().timestamp())
    .bind(p.id);

    match q.execute(pool).await {
        Ok(_) => {
            // 地址 / 凭据 / 开关变了都要让采集器立刻重算退避
            node::reset_node_state(p.id).await;
            audit::log(
                Some(&claims),
                None,
                "node_update",
                base.as_deref().unwrap_or(&old_base),
                new_name.unwrap_or(&old_name),
            )
            .await;
            Json(json!({"code": 0, "message": "OK"}))
        }
        Err(e) => Json(json!({"code": -1, "message": format!("更新失败: {}", e)})),
    }
}

pub async fn remove(claims: ValidatedClaims, Json(p): Json<IdsPayload>) -> Json<Value> {
    if p.ids.is_empty() {
        return Json(json!({"code": -1, "message": "请选择要删除的节点"}));
    }
    let pool = db::get_db_pool().await;
    let mut n = 0u64;
    for id in &p.ids {
        let _ = sqlx::query("DELETE FROM node_stats WHERE node_id = ?")
            .bind(id)
            .execute(pool)
            .await;
        match sqlx::query("DELETE FROM nodes WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await
        {
            Ok(r) => n += r.rows_affected(),
            Err(e) => return Json(json!({"code": -1, "message": format!("删除失败: {}", e)})),
        }
        node::reset_node_state(*id).await;
    }
    audit::log(
        Some(&claims),
        None,
        "node_delete",
        &format!("{:?}", p.ids),
        "",
    )
    .await;
    Json(json!({"code": 0, "message": "OK", "data": {"deleted": n}}))
}

/// 连通性测试（不落库）。
pub async fn test(Json(p): Json<TestPayload>) -> Json<Value> {
    let insecure = p.insecure_tls.unwrap_or(true);
    let mut token = p.api_token.unwrap_or_default().trim().to_string();
    if token.is_empty() {
        if let Some(id) = p.id {
            let pool = db::get_db_pool().await;
            token = sqlx::query_scalar::<_, String>("SELECT api_token FROM nodes WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await
                .unwrap_or(None)
                .map(|enc| crypto::decrypt_password(&enc))
                .unwrap_or_default();
        }
    }
    match node::test_connection(&p.base_url, &token, insecure).await {
        Ok(s) => Json(json!({
            "code": 0, "message": "连接成功",
            "data": {
                "os_name": s.os_name, "host_name": s.host_name, "version": s.version,
                "cpu_num": s.cpu_num, "memory_total_b": s.memory_total_b,
                "uptime": s.uptime, "public_ip": s.public_ip
            }
        })),
        Err(e) => Json(json!({"code": -1, "message": e})),
    }
}

/// 立即采集一轮（添加后不必等 15s）。
pub async fn refresh(_: ValidatedClaims) -> Json<Value> {
    node::collect_round().await;
    Json(json!({"code": 0, "message": "OK"}))
}

/// 监控大屏：全部节点最新值 + 集群汇总 + 近 30 分钟趋势。
pub async fn overview(_: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let rows: Vec<OverviewRow> = sqlx::query_as(
        "SELECT n.id, n.name, n.base_url, n.remark, n.status, n.online, n.last_seen, \
         n.last_error, n.os_name, n.host_name, n.version, n.public_ip, n.uptime, n.cpu_num, \
         n.memory_total_b, n.disk_total_b, n.disk_json, \
         s.cpu_usage, s.memory_usage, s.swap_usage, s.disk_usage, \
         s.loadavg_one, s.loadavg_five, s.loadavg_fifteen, s.created_at AS sampled_at \
         FROM nodes n \
         LEFT JOIN node_stats s ON s.id = (\
             SELECT id FROM node_stats WHERE node_id = n.id ORDER BY created_at DESC LIMIT 1) \
         WHERE n.status = 1 ORDER BY n.id ASC",
    )
    .fetch_all(pool)
    .await?;

    // 趋势一次取回（每节点约 120 点），避免 N 次查询
    let since = chrono::Local::now().timestamp() - 1800;
    // 指标列可空（手工写入 / 半截行都可能留 NULL），用 Option 接，
    // 否则单行 NULL 会让整次查询解码失败、所有节点趋势一起变空
    let trend_rows: Vec<(i64, Option<i64>, Option<f64>, Option<f64>, Option<f64>)> =
        sqlx::query_as(
            "SELECT node_id, created_at, cpu_usage, memory_usage, loadavg_one \
             FROM node_stats WHERE created_at >= ? ORDER BY node_id ASC, created_at ASC",
        )
        .bind(since)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    let mut trend: HashMap<i64, Vec<Value>> = HashMap::new();
    for (nid, ts, cpu, mem, load) in trend_rows {
        let Some(ts) = ts else { continue };
        trend
            .entry(nid)
            .or_default()
            .push(json!({"t": ts, "cpu": cpu, "mem": mem, "load": load}));
    }

    let mut online = 0usize;
    let (mut mem_total, mut mem_used, mut disk_total, mut disk_used) = (0i64, 0i64, 0i64, 0i64);
    let (mut cpu_sum, mut load_sum, mut max_load) = (0f64, 0f64, 0f64);

    let list: Vec<Value> = rows
        .iter()
        .map(|r| {
            let is_online = r.online == 1;
            if is_online {
                online += 1;
                let cpu = r.cpu_usage.unwrap_or(0.0);
                let mem = r.memory_usage.unwrap_or(0.0);
                let load = r.loadavg_one.unwrap_or(0.0);
                cpu_sum += cpu;
                load_sum += load;
                max_load = max_load.max(load);
                mem_total += r.memory_total_b;
                mem_used += (r.memory_total_b as f64 * mem / 100.0) as i64;
                disk_total += r.disk_total_b;
                disk_used += (r.disk_total_b as f64 * r.disk_usage.unwrap_or(0.0) / 100.0) as i64;
            }
            json!({
                "id": r.id, "name": r.name, "base_url": r.base_url, "remark": r.remark,
                "status": r.status, "online": is_online, "last_seen": r.last_seen,
                "last_error": r.last_error, "os_name": r.os_name, "host_name": r.host_name,
                "version": r.version, "public_ip": r.public_ip, "uptime": r.uptime,
                "cpu_num": r.cpu_num, "memory_total_b": r.memory_total_b,
                "disk_total_b": r.disk_total_b, "disks": disks_to_value(&r.disk_json),
                "cpu_usage": r.cpu_usage, "memory_usage": r.memory_usage,
                "swap_usage": r.swap_usage, "disk_usage": r.disk_usage,
                "loadavg_one": r.loadavg_one, "loadavg_five": r.loadavg_five,
                "loadavg_fifteen": r.loadavg_fifteen, "sampled_at": r.sampled_at,
                "trend": trend.remove(&r.id).unwrap_or_default(),
            })
        })
        .collect();
    let total = list.len();

    Ok(Json(json!({
        "code": 0, "message": "OK",
        "data": {
            "nodes": list,
            "interval": node::COLLECT_INTERVAL_SECS,
            "summary": {
                "total": total,
                "online": online,
                "offline": total - online,
                "avg_cpu_usage": if online > 0 { cpu_sum / online as f64 } else { 0.0 },
                "avg_load": if online > 0 { load_sum / online as f64 } else { 0.0 },
                "max_load": max_load,
                "memory_total_b": mem_total,
                "memory_used_b": mem_used,
                "disk_total_b": disk_total,
                "disk_used_b": disk_used
            }
        }
    })))
}

/// 单节点历史曲线（数据全部来自主控采样，不访问节点）。
pub async fn status(_: ValidatedClaims, q: Query<HashMap<String, String>>) -> ZapJsonResult {
    let id: i64 = q
        .get("id")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| err("缺少节点 id"))?;
    let pool = db::get_db_pool().await;
    let name: Option<(String,)> = sqlx::query_as("SELECT name FROM nodes WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let name = name.ok_or_else(|| err("节点不存在"))?.0;

    // 桶宽与本机 /system/status 一致，前端时间范围组件可直接复用
    let (window_secs, bucket): (i64, i64) = match q.get("range").map(|s| s.as_str()) {
        Some("1h") => (3600, 30),
        Some("6h") => (6 * 3600, 180),
        Some("24h") => (24 * 3600, 600),
        Some("7d") => (7 * 86400, 3600),
        Some("30d") => (30 * 86400, 14400),
        _ => (1800, 0),
    };
    let since = chrono::Local::now().timestamp() - window_secs;

    // 两个分支的 $2 含义不同（桶宽 / 起始时间），分开绑定
    let points: Vec<PointRow> = if bucket > 0 {
        sqlx::query_as(
            "SELECT (created_at / $2) * $2 AS created_at, \
             avg(cpu_usage) AS cpu_usage, avg(memory_usage) AS memory_usage, \
             avg(swap_usage) AS swap_usage, avg(disk_usage) AS disk_usage, \
             avg(loadavg_one) AS loadavg_one, avg(loadavg_five) AS loadavg_five, \
             avg(loadavg_fifteen) AS loadavg_fifteen \
             FROM node_stats WHERE node_id = $1 AND created_at >= $3 \
             GROUP BY created_at / $2 ORDER BY created_at / $2",
        )
        .bind(id)
        .bind(bucket)
        .bind(since)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT created_at, cpu_usage, memory_usage, swap_usage, disk_usage, \
             loadavg_one, loadavg_five, loadavg_fifteen \
             FROM node_stats WHERE node_id = $1 AND created_at >= $2 ORDER BY created_at",
        )
        .bind(id)
        .bind(since)
        .fetch_all(pool)
        .await?
    };

    let data: Vec<Value> = points
        .iter()
        .filter(|p| p.created_at.is_some())
        .map(|p| {
            json!({
                "created_at": p.created_at, "cpu_usage": p.cpu_usage,
                "memory_usage": p.memory_usage, "swap_usage": p.swap_usage,
                "disk_usage": p.disk_usage, "loadavg_one": p.loadavg_one,
                "loadavg_five": p.loadavg_five, "loadavg_fifteen": p.loadavg_fifteen
            })
        })
        .collect();

    Ok(Json(json!({
        "code": 0, "message": "OK",
        "data": {"node_id": id, "name": name, "range": if bucket > 0 { "history" } else { "live" },
                 "points": data}
    })))
}

/// `/api/nodes/{id}/**` 白名单反向代理：节点视图的远端请求入口。
///
/// 前端把 API_BASE / wsUrl 切到 `/api/nodes/{id}` 后，整个面板即以该节点为
/// 当前机器；这里先过 access.rs 门禁（admin + system.monitor:view），
/// 再校验方法 / 白名单 / 节点状态，最后以节点自己的 API Token 转发。
/// 远端响应原样带回（含 `{code,message,data}` 信封），前端拦截器无感。
pub async fn proxy(
    Path((id, rest)): Path<(i64, String)>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    // WS 升级请求：终端 / 实时日志的交互代理在后续版本开放，这里显式拒绝，
    // 避免 axum 把带 Upgrade 头的请求当普通 GET 转发后得到莫名其妙的响应
    if headers.get(header::UPGRADE).is_some() {
        return proxy_err(
            StatusCode::BAD_REQUEST,
            "WebSocket 代理暂未开放，请退出节点视图后使用终端 / 实时日志",
        );
    }
    if method != Method::GET && method != Method::HEAD {
        return proxy_err(
            StatusCode::METHOD_NOT_ALLOWED,
            "节点代理仅允许只读请求（GET）",
        );
    }
    if !node::proxy_allowed(&rest) {
        return proxy_err(StatusCode::FORBIDDEN, "该接口不在节点代理白名单内");
    }
    let target = match node::resolve_target(id).await {
        Ok(t) => t,
        Err(e) => return proxy_err(StatusCode::NOT_FOUND, &e),
    };
    match node::forward(&target, &rest, uri.query()).await {
        Ok((status, ctype, body)) => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            [(header::CONTENT_TYPE, ctype)],
            body,
        )
            .into_response(),
        Err(e) => proxy_err(StatusCode::BAD_GATEWAY, &format!("节点不可达：{}", e)),
    }
}

fn proxy_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "code": -1, "message": msg }))).into_response()
}
