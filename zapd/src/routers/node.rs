// SPDX-License-Identifier: AGPL-3.0-only
//! 集群管理：受管节点注册 + 跨节点监控查询。
//!
//! 凭据模型（与「共享 JWT」方案相对）：每台节点存一个**该节点自己签发的 API Token**
//! （`zap_` 开头），加密落库、接口永不回传原文。好处是单节点可独立吊销、
//! 有 `last_used_at` 审计，且一个节点泄露不影响其它节点。
//!
//! 采集只打节点的 `/api/system/info`（节点侧 `Required::User`），所以 Token 可以
//! 来自普通用户，不必在节点上开管理员；历史曲线全部由主控采样落库，查询不再访问节点。

use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts, Path, Query};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{QueryBuilder, Sqlite};
use std::collections::HashMap;
use std::net::SocketAddr;

use crate::db;
use crate::zap::{ZapError, ZapJsonResult, audit, crypto, jwt, jwt::ValidatedClaims, node};

#[derive(Deserialize)]
pub struct AddPayload {
    pub name: String,
    pub base_url: String,
    /// 节点上签发的 API Token（`zap_...`）；留空则用下面的账号密码现场引导签发
    #[serde(default)]
    pub api_token: String,
    /// 引导凭据：节点面板的登录账号。只用于本次登录 + 签发，不落库
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    /// 节点账号启用了两步验证时的 6 位验证码
    #[serde(default)]
    pub totp_code: Option<String>,
    #[serde(default)]
    pub remark: Option<String>,
    /// 容忍自签证书，默认 true
    #[serde(default)]
    pub insecure_tls: Option<bool>,
    /// 分组 id（0 = 未分组）
    #[serde(default)]
    pub group_id: Option<i64>,
    /// 归属用户 id（仅管理员可指定；默认归当前用户）
    #[serde(default)]
    pub owner_id: Option<i64>,
    /// 引导签发 Token 的有效天数；<= 0 表示用节点默认策略
    #[serde(default)]
    pub cred_expire_days: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdatePayload {
    pub id: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    /// 留空 = 不修改已存 Token（除非同时提供了下面的引导账号密码，则重新签发）
    #[serde(default)]
    pub api_token: Option<String>,
    /// 引导凭据：用于「节点上 Token 被删/过期后重连」——填了就用账号密码重新签一个 Token
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub totp_code: Option<String>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub insecure_tls: Option<bool>,
    #[serde(default)]
    pub status: Option<i64>,
    #[serde(default)]
    pub group_id: Option<i64>,
    #[serde(default)]
    pub owner_id: Option<i64>,
    #[serde(default)]
    pub cred_expire_days: Option<i64>,
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
    /// 引导凭据测试：只验证「能不能登录」，不签发 Token（见 test 的注释）
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub totp_code: Option<String>,
    #[serde(default)]
    pub insecure_tls: Option<bool>,
    /// 编辑态测试：Token 留空时复用库里已存的密文
    #[serde(default)]
    pub id: Option<i64>,
}

/// 解析后的凭据以及用于「到期提醒 / 轮换提示」的元数据。
struct ResolvedCred {
    token: String,
    expires_at: i64,
    issued_by: String,
}

/// 解析接入凭据：优先手工 Token；留空且有账号密码时，登录节点现场引导签发。
async fn resolve_api_token(
    api_token: &str,
    username: Option<&str>,
    password: Option<&str>,
    totp_code: Option<&str>,
    base: &str,
    insecure: bool,
    expire_days: i64,
) -> Result<ResolvedCred, String> {
    let manual = api_token.trim();
    if !manual.is_empty() {
        return Ok(ResolvedCred {
            token: manual.to_string(),
            expires_at: 0,
            issued_by: "manual".to_string(),
        });
    }
    let user = username.unwrap_or("").trim();
    if user.is_empty() || password.unwrap_or("").is_empty() {
        return Err("API Token 与账号密码至少填一项".to_string());
    }
    let (token, expires_at) = node::bootstrap_api_token(
        base,
        user,
        password.unwrap_or(""),
        totp_code.unwrap_or("").trim(),
        insecure,
        expire_days,
    )
    .await?;
    Ok(ResolvedCred {
        token,
        expires_at,
        issued_by: user.to_string(),
    })
}

/// 节点归属校验：管理员看全部，其他人只能操作归自己的节点（owner_id = 0 为管理员公有）。
fn node_in_scope(claims: &jwt::Claims, owner_id: i64) -> bool {
    jwt::is_admin(claims) || owner_id == claims.id as i64
}

/// 取节点归属；节点不存在返回 None。
async fn node_owner(pool: &sqlx::SqlitePool, id: i64) -> Option<i64> {
    sqlx::query_scalar::<_, i64>("SELECT owner_id FROM nodes WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
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
    group_id: i64,
    group_name: Option<String>,
    owner_id: i64,
    cred_issued_at: i64,
    cred_expires_at: i64,
    cred_issued_by: String,
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
    group_id: i64,
    group_name: Option<String>,
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
    net_rx_bps: Option<f64>,
    net_tx_bps: Option<f64>,
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
    net_rx_bps: Option<f64>,
    net_tx_bps: Option<f64>,
}

/// 大屏趋势的一次批量查询行：指标列可空（手工写入 / 半截行都可能留 NULL）。
#[derive(sqlx::FromRow)]
struct TrendRow {
    node_id: i64,
    created_at: Option<i64>,
    cpu_usage: Option<f64>,
    memory_usage: Option<f64>,
    loadavg_one: Option<f64>,
}

fn err(msg: impl Into<String>) -> ZapError {
    ZapError::New(-1, msg.into())
}

/// `disk_json` 存的是字符串，出接口时还原成数组，前端不用再 JSON.parse。
fn disks_to_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!([]))
}

#[derive(Deserialize)]
pub struct ListQuery {
    keyword: Option<String>,
    group_id: Option<i64>,
    status: Option<i64>,
    online: Option<i64>,
    sort: Option<String>,
}

/// 凭据状态：用于列表里的「到期 / 失效」提醒，不回传 Token 本身。
fn cred_status(r: &NodeRow) -> &'static str {
    let now = chrono::Local::now().timestamp();
    if r.cred_expires_at > 0 && r.cred_expires_at <= now {
        return "expired";
    }
    let low = r.last_error.to_lowercase();
    if low.contains("401")
        || low.contains("unauthorized")
        || low.contains("token")
        || low.contains("凭据")
        || low.contains("过期")
        || low.contains("失效")
    {
        return "invalid";
    }
    if r.cred_issued_at == 0 {
        "unknown"
    } else {
        "valid"
    }
}

/// 节点列表（不含 Token）：支持搜索 / 分组 / 状态过滤与排序，按归属隔离。
pub async fn list(claims: ValidatedClaims, Query(q): Query<ListQuery>) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT n.id, n.name, n.base_url, n.remark, n.insecure_tls, n.status, n.online, \
         n.last_seen, n.last_error, n.os_name, n.host_name, n.version, n.public_ip, n.uptime, \
         n.cpu_num, n.memory_total_b, n.disk_total_b, n.disk_json, \
         n.group_id, g.name AS group_name, n.owner_id, n.cred_issued_at, n.cred_expires_at, \
         n.cred_issued_by, n.created_at, n.updated_at \
         FROM nodes n \
         LEFT JOIN node_groups g ON g.id = n.group_id \
         LEFT JOIN node_stats s ON s.id = (\
             SELECT id FROM node_stats WHERE node_id = n.id ORDER BY created_at DESC LIMIT 1) \
         WHERE 1 = 1",
    );
    if !jwt::is_admin(&claims) {
        qb.push(" AND n.owner_id = ").push_bind(claims.id as i64);
    }
    if let Some(kw) = q
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        qb.push(" AND (n.name LIKE ")
            .push_bind(format!("%{kw}%"))
            .push(" OR n.base_url LIKE ")
            .push_bind(format!("%{kw}%"))
            .push(" OR n.public_ip LIKE ")
            .push_bind(format!("%{kw}%"))
            .push(" OR n.remark LIKE ")
            .push_bind(format!("%{kw}%"))
            .push(")");
    }
    if let Some(gid) = q.group_id {
        qb.push(" AND n.group_id = ").push_bind(gid);
    }
    if let Some(status) = q.status {
        qb.push(" AND n.status = ").push_bind(status);
    }
    if let Some(online) = q.online {
        qb.push(" AND n.online = ").push_bind(online);
    }
    // 排序字段走白名单，避免把用户输入拼进 SQL
    let order = match q.sort.as_deref().unwrap_or("id") {
        "name" => "n.name ASC, n.id ASC",
        "cpu" => "s.cpu_usage DESC NULLS LAST, n.id ASC",
        "memory" => "s.memory_usage DESC NULLS LAST, n.id ASC",
        "online" => "n.online DESC, n.last_seen DESC, n.id ASC",
        "last_seen" => "n.last_seen DESC, n.id ASC",
        _ => "n.id ASC",
    };
    qb.push(" ORDER BY ").push(order);
    let rows: Vec<NodeRow> = qb.build_query_as().fetch_all(pool).await?;
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
                "group_id": r.group_id, "group_name": r.group_name,
                "owner_id": r.owner_id, "cred_issued_at": r.cred_issued_at,
                "cred_expires_at": r.cred_expires_at, "cred_issued_by": r.cred_issued_by,
                "cred_status": cred_status(r),
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
    let pool = db::get_db_pool().await;
    let group_id = p.group_id.unwrap_or(0);
    if group_id != 0
        && sqlx::query_scalar::<_, i64>("SELECT id FROM node_groups WHERE id = ?")
            .bind(group_id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None)
            .is_none()
    {
        return Json(json!({"code": -1, "message": "节点分组不存在"}));
    }
    // 非管理员不能把节点挂到别人名下；owner_id = 0 是管理员公有池
    let owner_id = if jwt::is_admin(&claims) {
        p.owner_id.unwrap_or(0)
    } else {
        claims.id as i64
    };
    let insecure = p.insecure_tls.unwrap_or(true);
    // 手工 Token 直接用；留空则用账号密码在节点上引导签发（凭据本身不落库）
    let cred = match resolve_api_token(
        &p.api_token,
        p.username.as_deref(),
        p.password.as_deref(),
        p.totp_code.as_deref(),
        &base,
        insecure,
        p.cred_expire_days.unwrap_or(0),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return Json(json!({"code": -1, "message": format!("凭据引导失败：{e}")})),
    };
    // 网络 / 磁盘 / 进程采集要打节点的 Admin 接口，普通用户 Token 当场拒掉
    if let Err(e) = node::check_admin(&base, &cred.token, insecure).await {
        return Json(json!({"code": -1, "message": format!("节点凭据权限不足：{e}")}));
    }
    let snap = match node::test_connection(&base, &cred.token, insecure).await {
        Ok(s) => s,
        Err(e) => return Json(json!({"code": -1, "message": format!("节点连接失败：{}", e)})),
    };

    let enc = match crypto::encrypt_password(&cred.token) {
        Ok(v) => v,
        Err(e) => return Json(json!({"code": -1, "message": e})),
    };
    let now = chrono::Local::now().timestamp();
    let r = sqlx::query(
        "INSERT INTO nodes (name, base_url, api_token, remark, insecure_tls, status, online, \
         last_seen, last_error, os_name, host_name, version, public_ip, uptime, cpu_num, \
         memory_total_b, disk_total_b, disk_json, group_id, owner_id, \
         cred_issued_at, cred_expires_at, cred_issued_by, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,1,1,$6,'',$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$6,$6)",
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
    .bind(group_id)
    .bind(owner_id)
    .bind(now)
    .bind(cred.expires_at)
    .bind(&cred.issued_by)
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
    let exist: Option<(String, String, i64)> =
        sqlx::query_as("SELECT name, base_url, owner_id FROM nodes WHERE id = ?")
            .bind(p.id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
    let Some((old_name, old_base, old_owner)) = exist else {
        return Json(json!({"code": -1, "message": "节点不存在"}));
    };
    if !node_in_scope(&claims, old_owner) {
        return Json(json!({"code": -1, "message": "只能修改自己名下的节点"}));
    }

    let base = match p.base_url.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => match node::normalize_base(v) {
            Ok(b) => Some(b),
            Err(e) => return Json(json!({"code": -1, "message": e})),
        },
        _ => None,
    };
    let new_name = p.name.as_deref().map(str::trim).filter(|v| !v.is_empty());
    if let Some(gid) = p.group_id
        && gid != 0
        && sqlx::query_scalar::<_, i64>("SELECT id FROM node_groups WHERE id = ?")
            .bind(gid)
            .fetch_optional(pool)
            .await
            .unwrap_or(None)
            .is_none()
    {
        return Json(json!({"code": -1, "message": "节点分组不存在"}));
    }
    let new_owner = if jwt::is_admin(&claims) {
        p.owner_id
    } else {
        None
    };
    // Token 优先级：手工填写 > 账号密码重新签发 > 保持原值。
    // 「节点上 Token 被删/过期」正是靠第二条恢复：编辑里填一次账号密码即可。
    let new_cred = match p.api_token.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => Some(ResolvedCred {
            token: v.to_string(),
            expires_at: 0,
            issued_by: "manual".to_string(),
        }),
        _ => {
            let user = p.username.as_deref().unwrap_or("").trim();
            let has_cred = !user.is_empty() && !p.password.as_deref().unwrap_or("").is_empty();
            if has_cred {
                let base_now = base.clone().unwrap_or_else(|| old_base.clone());
                match node::bootstrap_api_token(
                    &base_now,
                    user,
                    p.password.as_deref().unwrap_or(""),
                    p.totp_code.as_deref().unwrap_or("").trim(),
                    p.insecure_tls.unwrap_or(true),
                    p.cred_expire_days.unwrap_or(0),
                )
                .await
                {
                    Ok((token, expires_at)) => Some(ResolvedCred {
                        token,
                        expires_at,
                        issued_by: user.to_string(),
                    }),
                    Err(e) => {
                        return Json(json!({"code": -1, "message": format!("凭据引导失败：{e}")}));
                    }
                }
            } else {
                None
            }
        }
    };

    let mut enc_token = None;
    let mut cred_meta: Option<(i64, i64, String)> = None;
    if let Some(c) = new_cred {
        let base_now = base.clone().unwrap_or_else(|| old_base.clone());
        if let Err(e) = node::check_admin(&base_now, &c.token, p.insecure_tls.unwrap_or(true)).await
        {
            return Json(json!({"code": -1, "message": format!("节点凭据权限不足：{e}")}));
        }
        enc_token = match crypto::encrypt_password(&c.token) {
            Ok(enc) => Some(enc),
            Err(e) => return Json(json!({"code": -1, "message": e})),
        };
        cred_meta = Some((chrono::Local::now().timestamp(), c.expires_at, c.issued_by));
    }
    let (issued_at, expires_at, issued_by) = match cred_meta {
        Some((a, b, c)) => (Some(a), Some(b), Some(c)),
        None => (None, None, None),
    };
    // 全列 COALESCE：没传的字段传 NULL 即保持原值。
    // 不用拼接动态 SET —— sqlx 会拦 dynamic SQL，且字面量更好审。
    let q = sqlx::query(
        "UPDATE nodes SET name = COALESCE($1, name), base_url = COALESCE($2, base_url), \
         remark = COALESCE($3, remark), insecure_tls = COALESCE($4, insecure_tls), \
         status = COALESCE($5, status), api_token = COALESCE($6, api_token), \
         group_id = COALESCE($7, group_id), owner_id = COALESCE($8, owner_id), \
         cred_issued_at = COALESCE($9, cred_issued_at), \
         cred_expires_at = COALESCE($10, cred_expires_at), \
         cred_issued_by = COALESCE($11, cred_issued_by), \
         updated_at = $12 WHERE id = $13",
    )
    .bind(new_name.map(str::to_string))
    .bind(base.clone())
    .bind(p.remark.clone())
    .bind(p.insecure_tls.map(|v| v as i64))
    .bind(p.status)
    .bind(enc_token)
    .bind(p.group_id)
    .bind(new_owner)
    .bind(issued_at)
    .bind(expires_at)
    .bind(issued_by)
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
    for id in &p.ids {
        match node_owner(pool, *id).await {
            Some(owner) if node_in_scope(&claims, owner) => {}
            _ => {
                return Json(json!({"code": -1, "message": format!("节点 {id} 不存在或无权操作")}));
            }
        }
    }
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
    if token.is_empty()
        && let Some(id) = p.id
    {
        let pool = db::get_db_pool().await;
        token = sqlx::query_scalar::<_, String>("SELECT api_token FROM nodes WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None)
            .map(|enc| crypto::decrypt_password(&enc))
            .unwrap_or_default();
    }
    // 账号密码模式：**只验证能否登录，不签发 Token**。「测试连接」可能被连点，
    // 每次都签一个新 Token 会在节点上留下一堆孤儿凭据；签发只在真正保存时发生。
    if token.is_empty() {
        let user = p.username.unwrap_or_default().trim().to_string();
        let pass = p.password.unwrap_or_default();
        if !user.is_empty() && !pass.is_empty() {
            return match node::login_for_jwt(
                &p.base_url,
                &user,
                &pass,
                p.totp_code.unwrap_or_default().trim(),
                insecure,
            )
            .await
            {
                Ok(jwt) => match node::check_admin(&p.base_url, &jwt, insecure).await {
                    Ok(()) => Json(json!({
                        "code": 0, "message": "账号密码可用，保存时会自动在节点上签发 API Token",
                        "data": null
                    })),
                    Err(e) => Json(json!({"code": -1, "message": format!("账号权限不足：{e}")})),
                },
                Err(e) => Json(json!({"code": -1, "message": e})),
            };
        }
    }
    if !token.is_empty()
        && let Err(e) = node::check_admin(&p.base_url, &token, insecure).await
    {
        return Json(json!({"code": -1, "message": format!("节点凭据权限不足：{e}")}));
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
pub async fn overview(claims: ValidatedClaims, Query(q): Query<ListQuery>) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT n.id, n.name, n.base_url, n.remark, n.group_id, g.name AS group_name, \
         n.status, n.online, n.last_seen, n.last_error, n.os_name, n.host_name, n.version, \
         n.public_ip, n.uptime, n.cpu_num, n.memory_total_b, n.disk_total_b, n.disk_json, \
         s.cpu_usage, s.memory_usage, s.swap_usage, s.disk_usage, \
         s.loadavg_one, s.loadavg_five, s.loadavg_fifteen, s.net_rx_bps, s.net_tx_bps, \
         s.created_at AS sampled_at \
         FROM nodes n \
         LEFT JOIN node_groups g ON g.id = n.group_id \
         LEFT JOIN node_stats s ON s.id = (\
             SELECT id FROM node_stats WHERE node_id = n.id ORDER BY created_at DESC LIMIT 1) \
         WHERE n.status = 1",
    );
    if !jwt::is_admin(&claims) {
        qb.push(" AND n.owner_id = ").push_bind(claims.id as i64);
    }
    if let Some(gid) = q.group_id {
        qb.push(" AND n.group_id = ").push_bind(gid);
    }
    qb.push(" ORDER BY n.group_id ASC, n.id ASC");
    let rows: Vec<OverviewRow> = qb.build_query_as().fetch_all(pool).await?;

    // 趋势一次取回（每节点约 120 点），避免 N 次查询
    let since = chrono::Local::now().timestamp() - 1800;
    // 单行 NULL 会让整次查询解码失败、所有节点趋势一起变空，故 TrendRow 各指标用 Option 接
    let trend_rows: Vec<TrendRow> = sqlx::query_as(
        "SELECT node_id, created_at, cpu_usage, memory_usage, loadavg_one \
         FROM node_stats WHERE created_at >= ? ORDER BY node_id ASC, created_at ASC",
    )
    .bind(since)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut trend: HashMap<i64, Vec<Value>> = HashMap::new();
    for row in trend_rows {
        let Some(ts) = row.created_at else { continue };
        trend.entry(row.node_id).or_default().push(json!({
            "t": ts,
            "cpu": row.cpu_usage,
            "mem": row.memory_usage,
            "load": row.loadavg_one,
        }));
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
                "group_id": r.group_id, "group_name": r.group_name,
                "status": r.status, "online": is_online, "last_seen": r.last_seen,
                "last_error": r.last_error, "os_name": r.os_name, "host_name": r.host_name,
                "version": r.version, "public_ip": r.public_ip, "uptime": r.uptime,
                "cpu_num": r.cpu_num, "memory_total_b": r.memory_total_b,
                "disk_total_b": r.disk_total_b, "disks": disks_to_value(&r.disk_json),
                "cpu_usage": r.cpu_usage, "memory_usage": r.memory_usage,
                "swap_usage": r.swap_usage, "disk_usage": r.disk_usage,
                "loadavg_one": r.loadavg_one, "loadavg_five": r.loadavg_five,
                "loadavg_fifteen": r.loadavg_fifteen,
                "net_rx_bps": r.net_rx_bps, "net_tx_bps": r.net_tx_bps,
                "sampled_at": r.sampled_at,
                "trend": trend.remove(&r.id).unwrap_or_default(),
            })
        })
        .collect();
    let total = list.len();
    // 分组汇总：大屏按组折叠时用，数据来自同一批 rows，不额外查库
    let mut group_acc: HashMap<i64, (String, usize, usize, f64)> = HashMap::new();
    for r in &rows {
        let entry = group_acc
            .entry(r.group_id)
            .or_insert_with(|| (r.group_name.clone().unwrap_or_default(), 0, 0, 0.0));
        entry.1 += 1;
        if r.online == 1 {
            entry.2 += 1;
            entry.3 += r.cpu_usage.unwrap_or(0.0);
        }
    }
    let mut group_summary: Vec<Value> = group_acc
        .into_iter()
        .map(|(id, (name, count, online, cpu))| {
            json!({
                "group_id": id, "name": name, "total": count, "online": online,
                "offline": count - online,
                "avg_cpu_usage": if online > 0 { cpu / online as f64 } else { 0.0 },
            })
        })
        .collect();
    group_summary.sort_by_key(|v| v["group_id"].as_i64().unwrap_or(0));

    Ok(Json(json!({
        "code": 0, "message": "OK",
        "data": {
            "nodes": list,
            "group_summary": group_summary,
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
pub async fn status(claims: ValidatedClaims, q: Query<HashMap<String, String>>) -> ZapJsonResult {
    let id: i64 = q
        .get("id")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| err("缺少节点 id"))?;
    let pool = db::get_db_pool().await;
    match node_owner(pool, id).await {
        Some(owner) if node_in_scope(&claims, owner) => {}
        _ => return Err(err("节点不存在或无权查看")),
    }
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
             avg(loadavg_fifteen) AS loadavg_fifteen, avg(net_rx_bps) AS net_rx_bps, \
             avg(net_tx_bps) AS net_tx_bps \
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
             loadavg_one, loadavg_five, loadavg_fifteen, net_rx_bps, net_tx_bps \
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
                "loadavg_five": p.loadavg_five, "loadavg_fifteen": p.loadavg_fifteen,
                "net_rx_bps": p.net_rx_bps, "net_tx_bps": p.net_tx_bps
            })
        })
        .collect();

    let top_processes: String = sqlx::query_scalar("SELECT top_processes FROM nodes WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "[]".to_string());
    let top_processes: Value = serde_json::from_str(&top_processes).unwrap_or_else(|_| json!([]));
    Ok(Json(json!({
        "code": 0, "message": "OK",
        "data": {"node_id": id, "name": name, "range": if bucket > 0 { "history" } else { "live" },
                 "points": data, "top_processes": top_processes}
    })))
}

/// 回给浏览器的响应头白名单。
///
/// 不整体透传：`Set-Cookie` 会把节点自己的会话 cookie 种到主控域名下，
/// 之后每个请求都带一份对主控无用的凭据；`Transfer-Encoding` 由 axum 重算。
/// 带上 `Content-Disposition` / `Last-Modified` 是为了接管后还能下载备份包与文件。
const PASSTHROUGH_HEADERS: [header::HeaderName; 6] = [
    header::CONTENT_TYPE,
    header::CONTENT_DISPOSITION,
    header::CONTENT_LENGTH,
    header::ETAG,
    header::LAST_MODIFIED,
    header::CACHE_CONTROL,
];

/// `/api/nodes/{id}/**` 反向代理：节点视图的远端请求入口。
///
/// 前端把 API_BASE / wsUrl 切到 `/api/nodes/{id}` 后，整个面板即以那台机器为
/// 当前主机，业务代码零改动。先过 access.rs 门禁（admin + `node:edit`），
/// 再排除主控自有面，最后以**节点自己的 API Token** 转发；浏览器带来的主控
/// JWT 不会出现在发往节点的请求里（query、header、小 JSON body 三处都置换）。
///
/// 写操作与 WebSocket 都会转发——「像直接登录那台机器」离不开这两样。
/// 能不能执行由节点按它自己的权限矩阵判定；主控侧另外对非 GET 与终端会话记审计。
/// 手工从 parts 取参数，而不是把它们写成提取器：axum 0.8 **没有**给
/// `Option<T>` 实现提取器，`Option<WebSocketUpgrade>` 编不过，
/// 而这条路由必须「是 WS 就当 WS 升级、否则当普通请求」——只能自己 parse。
#[axum::debug_handler]
pub async fn proxy(req: axum::extract::Request) -> Response {
    let (mut parts, body) = req.into_parts();
    let (id, rest) = match Path::<(i64, String)>::from_request_parts(&mut parts, &()).await {
        Ok(Path(v)) => v,
        Err(_) => return proxy_err(StatusCode::BAD_REQUEST, "节点 id 或转发路径不合法"),
    };
    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let headers = parts.headers.clone();
    let ip = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_default();
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if let Some(reason) = node::proxy_deny_reason(&rest) {
        return proxy_denied("master_owned", reason.message());
    }
    // 升级握手必须在节点侧完成：解析成功即这是一条 WS 请求
    let upgrade = axum::extract::ws::WebSocketUpgrade::from_request_parts(&mut parts, &())
        .await
        .ok();
    let target = match node::resolve_target(id).await {
        Ok(t) => t,
        Err(e) => return proxy_err(StatusCode::NOT_FOUND, &e),
    };
    let caller = ProxyCaller {
        claims: proxy_claims(&headers, &uri).await,
        ip,
        ua,
    };
    // 归属校验：管理员可接管全部，其它人只能接管自己名下的节点。
    // 必须先于任何转发（含只读 GET），否则「看监控」也会越过归属隔离。
    if !caller
        .claims
        .as_ref()
        .is_some_and(|c| node_in_scope(c, target.owner_id))
    {
        return proxy_denied("perm", "这台节点不属于你，无法接管");
    }
    // WS 用的也是 GET，但它是「直接操作那台机器」，与写操作同级对待
    let is_read = upgrade.is_none() && (method == Method::GET || method == Method::HEAD);

    // 只读接管沿用 `system.monitor:view`（与监控大屏同级）；写与终端另要求
    // `node:edit`——能把机器加进集群的人，才算有权以它的身份改那台机器。
    // 不能直接把 `/nodes` 整条规则抬到 node:edit：`/nodes/{id}/history` 属于
    // 监控大屏，只读角色也要能画曲线。
    if !is_read {
        let granted = match caller.claims.as_ref() {
            // 面板凭据取不到时直接拒：审计归属不明却放行写操作是最坏组合
            None => false,
            Some(c) => {
                let map = crate::routers::access::perm_map().await;
                crate::zap::jwt::is_admin(c)
                    || crate::routers::access::role_has_perm(&map, c, "node:edit")
            }
        };
        if !granted {
            return proxy_denied(
                "perm",
                "节点视图的写操作与终端需要「管理节点」权限（node:edit）",
            );
        }
    }

    // WebSocket：终端 / docker exec / 任务实时日志 / 插件日志跟随。
    // 必须先拨通节点再回 101：升级之后再失败就只能凭空断开，
    // 浏览器只能看到「连不上」，拿不到节点给的原因。
    if let Some(upgrade) = upgrade {
        let opened = node::open_node_ws(&target, &rest, uri.query()).await;
        audit_proxy(&caller, &target.name, id, "WS", &rest, opened.is_ok()).await;
        return match opened {
            Ok(stream) => upgrade.on_upgrade(move |socket| node::pump_ws(socket, stream)),
            Err(e) => proxy_err(
                StatusCode::BAD_GATEWAY,
                &format!("无法连接到节点 {}：{e}", target.name),
            ),
        };
    }

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    // 请求体按流转发：上传站点包、备份文件都不占主控内存。
    // 唯一例外是小 JSON——`/plugin/cancel` 这类接口把凭据放在 body 的 `token`
    // 字段里，不置换就等于把主控会话凭据交给受控端。
    let req_body = if is_read {
        None
    } else if is_small_json(content_type.as_deref(), &headers) {
        match axum::body::to_bytes(body, node::PROXY_JSON_SNIFF_BYTES as usize).await {
            Ok(b) => Some(reqwest::Body::from(node::swap_body_token(
                &b,
                &target.token,
            ))),
            Err(_) => {
                return proxy_err(StatusCode::PAYLOAD_TOO_LARGE, "JSON 请求体超过代理上限");
            }
        }
    } else {
        // axum 的 Body 只实现 http_body::Body，要转成 data stream 才满足
        // reqwest 的 TryStream 约束
        Some(reqwest::Body::wrap_stream(body.into_data_stream()))
    };

    match node::forward(
        &target,
        &rest,
        uri.query(),
        method.clone(),
        req_body,
        content_type,
    )
    .await
    {
        Ok(resp) => {
            let status = resp.status();
            // 头要在把 body 流搬走之前取
            let passthrough: Vec<(header::HeaderName, axum::http::HeaderValue)> =
                PASSTHROUGH_HEADERS
                    .iter()
                    .filter_map(|n| resp.headers().get(n).cloned().map(|v| (n.clone(), v)))
                    .collect();
            if !is_read {
                audit_proxy(
                    &caller,
                    &target.name,
                    id,
                    method.as_str(),
                    &rest,
                    status.is_success(),
                )
                .await;
            }
            let mut out = Response::new(axum::body::Body::from_stream(resp.bytes_stream()));
            *out.status_mut() = status;
            // 「这个响应确实由节点应答」的唯一凭据。只看 URL 分不开两种 401：
            // 节点拒绝了它的 API Token，与主控在同一条代理 URL 上拒绝了浏览器的
            // 会话——前者只能去「节点管理」换凭据，后者必须走刷新 / 重新登录。
            // 误判成前者的后果是接管态永久卡死在登录页外（既不刷新也不登出）。
            out.headers_mut().insert(
                header::HeaderName::from_static("x-zap-node-proxy"),
                axum::http::HeaderValue::from_static("1"),
            );
            for (k, v) in passthrough {
                out.headers_mut().insert(k, v);
            }
            out
        }
        // 不可达 / 超时：明确 502 + 短原因（不转发底层错误体）
        Err(e) => {
            if !is_read {
                audit_proxy(&caller, &target.name, id, method.as_str(), &rest, false).await;
            }
            proxy_err(
                StatusCode::BAD_GATEWAY,
                &format!("无法连接到节点 {}：{e}", target.name),
            )
        }
    }
}

/// 取本次代理的操作者，仅用于审计归属。
///
/// 不能直接用 `Claims` extractor：它只认 `Authorization` 请求头，而浏览器给
/// WebSocket / EventSource 加不了请求头，凭据在 `?token=` 里。
async fn proxy_claims(headers: &HeaderMap, uri: &Uri) -> Option<crate::zap::jwt::Claims> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").unwrap_or(v).to_string())
        .or_else(|| {
            uri.query()?
                .split('&')
                .find_map(|kv| kv.strip_prefix("token=").map(str::to_string))
        })?;
    crate::zap::jwt::claims_from_token(&raw).await
}

/// 代理请求的调用方上下文：审计要知道「谁、从哪个 IP、用什么客户端」动的那台机器。
struct ProxyCaller {
    claims: Option<crate::zap::jwt::Claims>,
    ip: String,
    ua: String,
}

/// 节点视图的写操作与终端会话记一条主控审计。
///
/// 节点侧也会记（它只看到「某个 API Token 在动」），但那条查不到**是谁从主控点的**。
/// 没有这条，「谁把生产站点的 nginx 配置改坏了」就只能靠时间点对猜。
async fn audit_proxy(
    caller: &ProxyCaller,
    node_name: &str,
    id: i64,
    method: &str,
    rest: &str,
    ok: bool,
) {
    audit::log_with(
        caller.claims.as_ref(),
        Some(caller.ip.as_str()),
        "node_proxy",
        &format!("{node_name}(#{id})"),
        &format!("{method} /{rest}"),
        if ok { "success" } else { "failure" },
        &caller.ua,
    )
    .await;
}

/// 是否「小 JSON」请求体：只有这种才值得缓冲下来置换 body 里的凭据。
///
/// 未带 `Content-Length`（chunked）时按不处理走：那类请求通常是上传，
/// 既不含会话凭据，也不该被缓冲。
fn is_small_json(content_type: Option<&str>, headers: &HeaderMap) -> bool {
    if !content_type.is_some_and(|ct| ct.starts_with("application/json")) {
        return false;
    }
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .is_none_or(|n| n <= node::PROXY_JSON_SNIFF_BYTES)
}

fn proxy_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "code": -1, "message": msg }))).into_response()
}

/// 主控侧代理策略拒绝：额外带一个响应头，让前端区分「主控没放行」和
/// 「远端 Token 权限不足」——两者的下一步完全不同（前者要么换接口、要么找主控
/// 管理员要 `node:edit`，后者去节点管理换 Token）。不带这个头，403 会被统一
/// 解释成 Token 权限问题，把用户引到错误的排查方向。
///
/// `code`：`master_owned`（该接口本就不代理）/ `perm`（主控账号没有 node:edit）。
fn proxy_denied(code: &str, msg: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        [("x-zap-node-denied", code)],
        Json(json!({ "code": -1, "message": msg })),
    )
        .into_response()
}

// ── 分组管理 ────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct GroupPayload {
    name: String,
    #[serde(default)]
    sort_order: i64,
}

#[derive(Deserialize)]
pub struct GroupUpdatePayload {
    id: i64,
    name: String,
    #[serde(default)]
    sort_order: i64,
}

#[derive(Deserialize)]
pub struct IdPayload {
    id: i64,
}

/// 分组列表（带节点数），侧栏筛选与弹窗共用。
pub async fn group_list(_: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let rows: Vec<(i64, String, i64, i64)> = sqlx::query_as(
        "SELECT g.id, g.name, g.sort_order, COUNT(n.id) \
         FROM node_groups g LEFT JOIN nodes n ON n.group_id = g.id \
         GROUP BY g.id ORDER BY g.sort_order ASC, g.id ASC",
    )
    .fetch_all(pool)
    .await?;
    let data: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, sort_order, count)| {
            json!({"id": id, "name": name, "sort_order": sort_order, "count": count})
        })
        .collect();
    Ok(Json(json!({"code": 0, "message": "OK", "data": data})))
}

pub async fn group_add(claims: ValidatedClaims, Json(p): Json<GroupPayload>) -> Json<Value> {
    if !jwt::is_admin(&claims) {
        return Json(json!({"code": -1, "message": "只有管理员能管理分组"}));
    }
    let name = p.name.trim();
    if name.is_empty() {
        return Json(json!({"code": -1, "message": "分组名不能为空"}));
    }
    let pool = db::get_db_pool().await;
    let r = sqlx::query("INSERT INTO node_groups (name, sort_order, created_at) VALUES (?, ?, ?)")
        .bind(name)
        .bind(p.sort_order)
        .bind(chrono::Local::now().timestamp())
        .execute(pool)
        .await;
    match r {
        Ok(a) => {
            audit::log(Some(&claims), None, "node_group_add", name, "").await;
            Json(json!({"code": 0, "message": "OK", "data": {"id": a.last_insert_rowid()}}))
        }
        Err(e) => {
            let msg = if format!("{e}").contains("UNIQUE") {
                "分组名已存在".to_string()
            } else {
                format!("保存失败: {e}")
            };
            Json(json!({"code": -1, "message": msg}))
        }
    }
}

pub async fn group_update(
    claims: ValidatedClaims,
    Json(p): Json<GroupUpdatePayload>,
) -> Json<Value> {
    if !jwt::is_admin(&claims) {
        return Json(json!({"code": -1, "message": "只有管理员能管理分组"}));
    }
    let name = p.name.trim();
    if name.is_empty() {
        return Json(json!({"code": -1, "message": "分组名不能为空"}));
    }
    let pool = db::get_db_pool().await;
    match sqlx::query("UPDATE node_groups SET name = ?, sort_order = ? WHERE id = ?")
        .bind(name)
        .bind(p.sort_order)
        .bind(p.id)
        .execute(pool)
        .await
    {
        Ok(_) => {
            audit::log(
                Some(&claims),
                None,
                "node_group_update",
                name,
                &format!("id={}", p.id),
            )
            .await;
            Json(json!({"code": 0, "message": "OK"}))
        }
        Err(e) => Json(json!({"code": -1, "message": format!("保存失败: {e}")})),
    }
}

/// 删除分组：节点保留，group_id 归零（等价于「未分组」）。
pub async fn group_delete(claims: ValidatedClaims, Json(p): Json<IdPayload>) -> Json<Value> {
    if !jwt::is_admin(&claims) {
        return Json(json!({"code": -1, "message": "只有管理员能管理分组"}));
    }
    let pool = db::get_db_pool().await;
    let _ = sqlx::query("UPDATE nodes SET group_id = 0 WHERE group_id = ?")
        .bind(p.id)
        .execute(pool)
        .await;
    match sqlx::query("DELETE FROM node_groups WHERE id = ?")
        .bind(p.id)
        .execute(pool)
        .await
    {
        Ok(_) => {
            audit::log(
                Some(&claims),
                None,
                "node_group_delete",
                "",
                &format!("id={}", p.id),
            )
            .await;
            Json(json!({"code": 0, "message": "OK"}))
        }
        Err(e) => Json(json!({"code": -1, "message": format!("删除失败: {e}")})),
    }
}

// ── 批量操作 / 归属 / 告警 ──────────────────────────────────

#[derive(Deserialize)]
pub struct BatchUpdatePayload {
    ids: Vec<i64>,
    status: Option<i64>,
    group_id: Option<i64>,
}

/// 批量启停 / 改分组：逐个校验归属，任何一台越权就整批拒绝。
pub async fn batch_update(
    claims: ValidatedClaims,
    Json(p): Json<BatchUpdatePayload>,
) -> Json<Value> {
    if p.ids.is_empty() {
        return Json(json!({"code": -1, "message": "请先选择节点"}));
    }
    if p.status.is_some_and(|v| v != 0 && v != 1) {
        return Json(json!({"code": -1, "message": "状态只能是 0 或 1"}));
    }
    let pool = db::get_db_pool().await;
    for id in &p.ids {
        match node_owner(pool, *id).await {
            Some(owner) if node_in_scope(&claims, owner) => {}
            _ => {
                return Json(json!({"code": -1, "message": format!("节点 {id} 不存在或无权操作")}));
            }
        }
    }
    if let Some(gid) = p.group_id {
        if gid != 0
            && sqlx::query_scalar::<_, i64>("SELECT id FROM node_groups WHERE id = ?")
                .bind(gid)
                .fetch_optional(pool)
                .await
                .unwrap_or(None)
                .is_none()
        {
            return Json(json!({"code": -1, "message": "节点分组不存在"}));
        }
        for id in &p.ids {
            let _ = sqlx::query("UPDATE nodes SET group_id = ?, updated_at = ? WHERE id = ?")
                .bind(gid)
                .bind(chrono::Local::now().timestamp())
                .bind(id)
                .execute(pool)
                .await;
        }
    }
    if let Some(status) = p.status {
        for id in &p.ids {
            let _ = sqlx::query("UPDATE nodes SET status = ?, updated_at = ? WHERE id = ?")
                .bind(status)
                .bind(chrono::Local::now().timestamp())
                .bind(id)
                .execute(pool)
                .await;
            node::reset_node_state(*id).await;
        }
    }
    audit::log(
        Some(&claims),
        None,
        "node_batch_update",
        &format!("{:?}", p.ids),
        &format!("status={:?}, group_id={:?}", p.status, p.group_id),
    )
    .await;
    Json(json!({"code": 0, "message": "OK", "data": {"updated": p.ids.len()}}))
}

/// 可归属的用户列表：管理员看全部，经销商看自己及自己的成员。
pub async fn owners(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let rows: Vec<(i64, String)> = if jwt::is_admin(&claims) {
        sqlx::query_as("SELECT id, username FROM user ORDER BY id ASC")
            .fetch_all(pool)
            .await?
    } else {
        sqlx::query_as(
            "SELECT id, username FROM user WHERE id = ? OR (owner_id = ? AND user_kind = 1) \
             ORDER BY id ASC",
        )
        .bind(claims.id as i64)
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await?
    };
    let data: Vec<Value> = rows
        .into_iter()
        .map(|(id, name)| json!({"id": id, "username": name}))
        .collect();
    Ok(Json(json!({"code": 0, "message": "OK", "data": data})))
}

const ALERT_KEYS: [(&str, &str); 5] = [
    ("node_alert_cpu", "CPU 阈值（%），0 = 关闭"),
    ("node_alert_mem", "内存阈值（%），0 = 关闭"),
    ("node_alert_disk", "磁盘阈值（%），0 = 关闭"),
    ("node_alert_offline_secs", "离线多少秒后告警，0 = 关闭"),
    ("node_alert_cooldown_secs", "同一节点同一类告警的冷却秒数"),
];

fn alert_num(key: &str, default: i64) -> i64 {
    crate::zap::server_env::conf_get(key)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
}

/// 告警阈值配置（存在 server_env.conf，随配置一起备份）。
pub async fn alert_config_get(_: ValidatedClaims) -> ZapJsonResult {
    Ok(Json(json!({
        "code": 0, "message": "OK",
        "data": {
            "cpu": alert_num("node_alert_cpu", 85),
            "memory": alert_num("node_alert_mem", 90),
            "disk": alert_num("node_alert_disk", 90),
            "offline_seconds": alert_num("node_alert_offline_secs", 300),
            "cooldown_seconds": alert_num("node_alert_cooldown_secs", 1800),
            "email": alert_num("node_alert_email", 0) == 1,
        }
    })))
}

#[derive(Deserialize)]
pub struct AlertConfigPayload {
    cpu: i64,
    memory: i64,
    disk: i64,
    offline_seconds: i64,
    cooldown_seconds: i64,
    email: bool,
}

pub async fn alert_config_set(
    claims: ValidatedClaims,
    Json(p): Json<AlertConfigPayload>,
) -> Json<Value> {
    if !jwt::is_admin(&claims) {
        return Json(json!({"code": -1, "message": "只有管理员能修改告警配置"}));
    }
    let values = [
        p.cpu.max(0),
        p.memory.max(0),
        p.disk.max(0),
        p.offline_seconds.max(0),
        p.cooldown_seconds.max(60),
    ];
    let items: Vec<(String, String)> = ALERT_KEYS
        .iter()
        .zip(values)
        .map(|((k, _remark), v)| (k.to_string(), v.to_string()))
        .collect();
    crate::zap::server_env::conf_set_many(&items, "cluster alert thresholds");
    crate::zap::server_env::conf_set(
        "node_alert_email",
        if p.email { "1" } else { "0" },
        "cluster alert email",
    );
    audit::log(Some(&claims), None, "node_alert_config", "", "").await;
    Json(json!({"code": 0, "message": "OK"}))
}

/// 最近的告警事件（按节点归属隔离）。
pub async fn alert_events(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT e.id, e.node_id, e.node_name, e.kind, e.message, e.created_at \
         FROM node_alert_events e JOIN nodes n ON n.id = e.node_id WHERE 1 = 1",
    );
    if !jwt::is_admin(&claims) {
        qb.push(" AND n.owner_id = ").push_bind(claims.id as i64);
    }
    qb.push(" ORDER BY e.id DESC LIMIT 200");
    let rows: Vec<(i64, i64, String, String, String, i64)> =
        qb.build_query_as().fetch_all(pool).await?;
    let data: Vec<Value> = rows
        .into_iter()
        .map(|(id, node_id, node_name, kind, message, created_at)| {
            json!({
                "id": id, "node_id": node_id, "node_name": node_name,
                "kind": kind, "message": message, "created_at": created_at,
            })
        })
        .collect();
    Ok(Json(json!({"code": 0, "message": "OK", "data": data})))
}

#[derive(Deserialize)]
pub struct BatchScriptPayload {
    ids: Vec<i64>,
    path: String,
}

/// 批量下发脚本（节点上已有的「应用商店脚本」，不执行任意命令）。
/// 每台节点独立返回结果，部分失败不影响其它节点。
pub async fn batch_script(
    claims: ValidatedClaims,
    Json(p): Json<BatchScriptPayload>,
) -> Json<Value> {
    let path = p.path.trim();
    if path.is_empty() {
        return Json(json!({"code": -1, "message": "脚本路径不能为空"}));
    }
    let pool = db::get_db_pool().await;
    let mut results = Vec::new();
    for id in &p.ids {
        let owner = node_owner(pool, *id).await;
        if !owner.is_some_and(|o| node_in_scope(&claims, o)) {
            results.push(json!({"id": id, "ok": false, "message": "节点不存在或无权操作"}));
            continue;
        }
        let target = match node::resolve_target(*id).await {
            Ok(t) => t,
            Err(e) => {
                results.push(json!({"id": id, "ok": false, "message": e}));
                continue;
            }
        };
        let v = node::api_post(
            &target.base,
            "/appstore/script/run",
            Some(&target.token),
            json!({"path": path}),
            target.insecure,
        )
        .await;
        match v {
            Ok(v) if v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1) == 0 => {
                results.push(json!({"id": id, "ok": true, "message": "已下发"}));
            }
            Ok(v) => results.push(json!({
                "id": id, "ok": false,
                "message": v.get("message").and_then(|m| m.as_str()).unwrap_or("节点拒绝执行"),
            })),
            Err(e) => results.push(json!({"id": id, "ok": false, "message": e})),
        }
    }
    audit::log(
        Some(&claims),
        None,
        "node_batch_script",
        path,
        &format!("{:?}", p.ids),
    )
    .await;
    Json(json!({"code": 0, "message": "OK", "data": {"results": results}}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(id: u64, roles: &str) -> jwt::Claims {
        jwt::Claims {
            id,
            iat: 0,
            sub: "tester".to_string(),
            iss: "zap".to_string(),
            exp: 0,
            roles: roles.to_string(),
            token_version: 0,
            scope: String::new(),
        }
    }

    fn row() -> NodeRow {
        NodeRow {
            id: 1,
            name: "n1".to_string(),
            base_url: "https://n1".to_string(),
            remark: String::new(),
            insecure_tls: 1,
            status: 1,
            online: 1,
            last_seen: 0,
            last_error: String::new(),
            os_name: String::new(),
            host_name: String::new(),
            version: String::new(),
            public_ip: String::new(),
            uptime: String::new(),
            cpu_num: 1,
            memory_total_b: 0,
            disk_total_b: 0,
            disk_json: "[]".to_string(),
            group_id: 0,
            group_name: None,
            owner_id: 7,
            cred_issued_at: 100,
            cred_expires_at: 0,
            cred_issued_by: "manual".to_string(),
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn node_scope_separates_admin_and_owner() {
        assert!(node_in_scope(&claims(1, "admin"), 99));
        assert!(node_in_scope(&claims(7, "user"), 7));
        assert!(!node_in_scope(&claims(7, "user"), 8));
        assert!(!node_in_scope(&claims(7, "user"), 0));
    }

    #[test]
    fn credential_status_flags_expiry_and_token_errors() {
        let mut r = row();
        assert_eq!(cred_status(&r), "valid");
        r.cred_expires_at = 1;
        assert_eq!(cred_status(&r), "expired");
        r.cred_expires_at = 0;
        r.last_error = "节点 Token 无效".to_string();
        assert_eq!(cred_status(&r), "invalid");
        r.last_error.clear();
        r.cred_issued_at = 0;
        assert_eq!(cred_status(&r), "unknown");
    }
}
