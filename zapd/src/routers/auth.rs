use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::Query;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::{Extension, Json};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::query_as;
use tracing::warn;

use crate::db;
use crate::zap::{
    self, ZapError, ZapJsonResult, audit, jwt::ValidatedClaims, login_history, session, totp,
};

/// 面板会话 Cookie 名。
///
/// 前端把 token 存在 sessionStorage（见 `web/src/utils/auth.ts`），
/// 浏览器直接打开 `/webapps/*` 这类页面时不会携带任何凭据，
/// 因此登录/续期时同步下发一个 HttpOnly Cookie 作为**页面级**登录态。
/// 它只用于页面路由鉴权；`/api/*` 仍以 Authorization 头为主。
pub const SESSION_COOKIE: &str = "zap_token";

/// Web 应用（`/webapps/*`）页面会话有效期（24 小时）。
///
/// 面板 JWT 默认只有 1 小时，而页面会话若用同一个有效期，
/// 用户登录一小时后新开标签页时 Cookie 已过期（浏览器不再发送），
/// 就会被提示「需要登录」。`/webapps/*` 每次成功访问都会滑动续期。
pub const WEBAPP_SESSION_SECS: u64 = 24 * 3600;

/// 生成会话 Cookie（HttpOnly + SameSite=Lax，有效期与 Cookie 内 JWT 一致）。
pub fn session_cookie(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    // 不设 SameSite：个别浏览器/版本对 Lax 的判定存在差异（新标签直接打开可能不发送），
    // 去掉后回到最宽松的兼容行为（同站请求一律携带）。
    // 刻意不加 Secure：面板常以自签证书 + IP 访问，浏览器不把这类站点视为
    // 「安全来源」，带 Secure 的 Cookie 会被保存但永不回传（实测：请求里只出现
    // 不带 Secure 的 Cookie），导致 Web 应用始终判定为未登录。链路仍是 HTTPS。
    let value = format!(
        "{SESSION_COOKIE}={token}; Path=/webapps/; HttpOnly; SameSite=Lax; Max-Age={WEBAPP_SESSION_SECS}"
    );
    if let Ok(v) = HeaderValue::from_str(&value) {
        headers.insert(header::SET_COOKIE, v);
    }
    headers
}

/// 为页面会话签发长有效期 JWT（仅供 Cookie 使用；面板 access_token 不受影响）。
fn webapp_session_token(username: String, id: u64, roles: &str) -> Option<String> {
    zap::jwt::generate_jwt_token_with_expire(username, id, roles, WEBAPP_SESSION_SECS).ok()
}

/// 清除会话 Cookie（登出 / 改密后失效）。
fn clear_session_cookie() -> HeaderMap {
    let mut headers = HeaderMap::new();
    // 同时清两个 Path：历史版本用的是 Path=/，而浏览器按
    // (域名, 路径, 名称) 三元组区分 Cookie，只清一个会留下另一个残留。
    for path in ["/", "/webapps/"] {
        let value = format!("{SESSION_COOKIE}=; Path={path}; HttpOnly; SameSite=Lax; Max-Age=0");
        if let Ok(v) = HeaderValue::from_str(&value) {
            headers.append(header::SET_COOKIE, v);
        }
    }
    headers
}

/// Rate limiter state: Maps IP -> (attempt_count, window_start)
static LOGIN_RATE_LIMITER: Lazy<Mutex<HashMap<IpAddr, (u32, Instant)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

const MAX_LOGIN_ATTEMPTS: u32 = 5;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

/// 持久化失败锁定策略（针对账号+IP）
const MAX_DB_FAILED_ATTEMPTS: i64 = 5;
const LOCK_DURATION_SECS: i64 = 15 * 60;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UserLoginData {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub totp_code: Option<String>,
}

#[derive(sqlx::FromRow, Debug)]
struct UserRecord {
    id: u64,
    username: String,
    password: String,
    roles: String,
    totp_secret: String,
    totp_enabled: i32,
}

/// Check rate limit for the given IP. Returns Ok(()) if allowed, Err if rate limited.
fn check_rate_limit(ip: IpAddr) -> Result<(), ZapError> {
    let mut map = LOGIN_RATE_LIMITER.lock().unwrap();
    let now = Instant::now();

    let entry = map.entry(ip).or_insert((0, now));

    // Reset window if expired
    if now.duration_since(entry.1) > RATE_LIMIT_WINDOW {
        *entry = (1, now);
        return Ok(());
    }

    entry.0 += 1;
    if entry.0 > MAX_LOGIN_ATTEMPTS {
        warn!("Rate limit exceeded for IP: {}", ip);
        return Err(ZapError::New(
            -1,
            "登录尝试过于频繁，请60秒后再试".to_string(),
        ));
    }

    Ok(())
}

/// 检查持久化锁定（账号+IP 维度，失败 5 次锁定 15 分钟）。
async fn check_db_lock(ip: &str, username: &str) -> Result<(), ZapError> {
    let pool = db::get_db_pool().await;
    let now = chrono::Utc::now().timestamp();
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT locked_until FROM login_attempts
         WHERE username = ? AND ip = ? AND locked_until > ?",
    )
    .bind(username)
    .bind(ip)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    if let Some((until,)) = row {
        let remaining = until - now;
        return Err(ZapError::New(
            -1,
            format!(
                "登录失败次数过多，账号已锁定，请约 {} 分钟后再试",
                remaining / 60 + 1
            ),
        ));
    }
    Ok(())
}

/// 记录一次登录失败，达到阈值后锁定。
async fn record_failed_login(ip: &str, username: &str) {
    let pool = db::get_db_pool().await;
    let now = chrono::Utc::now().timestamp();
    let lock_until = now + LOCK_DURATION_SECS;
    let _ = sqlx::query(
        r#"INSERT INTO login_attempts (username, ip, failed_count, locked_until, updated_at)
           VALUES (?, ?, 1, 0, ?)
           ON CONFLICT(username, ip) DO UPDATE SET
             failed_count = login_attempts.failed_count + 1,
             locked_until = CASE
               WHEN login_attempts.failed_count + 1 >= ? THEN ?
               ELSE 0
             END,
             updated_at = excluded.updated_at"#,
    )
    .bind(username)
    .bind(ip)
    .bind(now)
    .bind(MAX_DB_FAILED_ATTEMPTS)
    .bind(lock_until)
    .bind(now)
    .execute(pool)
    .await;
}

/// 登录成功后清除失败记录。
async fn clear_login_attempts(ip: &str, username: &str) {
    let pool = db::get_db_pool().await;
    let _ = sqlx::query("DELETE FROM login_attempts WHERE username = ? AND ip = ?")
        .bind(username)
        .bind(ip)
        .execute(pool)
        .await;
}

#[axum::debug_handler]
pub async fn login(
    Extension(client_addr): Extension<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<UserLoginData>,
) -> Result<(HeaderMap, Json<serde_json::Value>), ZapError> {
    // 第一道防线：内存滑动窗口限流
    check_rate_limit(client_addr.ip())?;

    let ip = client_addr.ip().to_string();
    // 登录记录要留来源设备，原始 UA 原样存，不做解析（前端展示时截断）
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let username = payload.username.trim().to_string();
    if username.is_empty() {
        return Err(ZapError::New(-1, "用户名不能为空".to_string()));
    }

    // 第二道防线：持久化失败锁定（账号+IP）
    check_db_lock(&ip, &username).await?;

    let pool = db::get_db_pool().await;
    let record: Result<UserRecord, sqlx::Error> = query_as(
        "SELECT id, username, password, roles, totp_secret, totp_enabled
         FROM user WHERE username = ?",
    )
    .bind(&username)
    .fetch_one(pool)
    .await;

    if let Ok(row) = record
        && let Ok(true) = bcrypt::verify(&payload.password, &row.password)
    {
        // TOTP 两步验证（已启用时校验）
        if row.totp_enabled == 1 {
            let code = payload.totp_code.unwrap_or_default();
            if code.is_empty() {
                // 密码正确但未提供验证码：通知前端进入第二步（展示验证码输入）
                return Err(ZapError::New(
                    1002,
                    "该账号已启用两步验证，请输入验证码".to_string(),
                ));
            }
            if !totp::verify(&row.totp_secret, &code) {
                audit::log(None, Some(&ip), "login_2fa_failed", &row.username, "").await;
                login_history::record(
                    row.id as i64,
                    &row.username,
                    &ip,
                    &user_agent,
                    login_history::STATUS_2FA_FAILED,
                )
                .await;
                return Err(ZapError::New(-1, "两步验证码错误或已失效".to_string()));
            }
        }

        if let Ok(token) = zap::jwt::generate_jwt_token(row.username.clone(), row.id, &row.roles) {
            clear_login_attempts(&ip, &username).await;
            // 更新最后登录信息
            let now = chrono::Local::now().timestamp();
            let _ =
                sqlx::query("UPDATE user SET last_login_time = ?, last_login_ip = ? WHERE id = ?")
                    .bind(now)
                    .bind(&ip)
                    .bind(row.id as i64)
                    .execute(pool)
                    .await;
            audit::log(None, Some(&ip), "login_success", &row.username, "").await;
            login_history::record(
                row.id as i64,
                &row.username,
                &ip,
                &user_agent,
                login_history::STATUS_SUCCESS,
            )
            .await;
            // 登录成功站内信（是否发送取决于该用户通知偏好，默认不发送）
            crate::zap::notify::login_success(row.id as i64, &row.username, &ip).await;
            // Cookie 使用更长有效期的 JWT（面板 access_token 仍为 1 小时）
            let cookie_token = webapp_session_token(row.username.clone(), row.id, &row.roles)
                .unwrap_or_else(|| token.clone());
            let headers = session_cookie(&cookie_token);
            return Ok((
                headers,
                Json(json!({
                    "code": 0,
                    "access_token": token,
                    "token_type": "Bearer",
                    "message": "登陆成功",
                    "expire_in": crate::config::get_config().read().unwrap().jwt.jwt_expire,
                })),
            ));
        }
    }
    record_failed_login(&ip, &username).await;
    audit::log(None, Some(&ip), "login_failed", &username, "").await;
    // 失败尝试记在用户名下（user_id=0：账号未必存在，无法归属）
    login_history::record(0, &username, &ip, &user_agent, login_history::STATUS_FAILED).await;
    Err(ZapError::New(-1, "用户名或密码错误".to_string()))
}

pub async fn logout() -> Result<(HeaderMap, Json<serde_json::Value>), ZapError> {
    Ok((
        clear_session_cookie(),
        Json(json!({
            "code": 0,
            "message": "退出成功"
        })),
    ))
}

// ── 登录记录 / 下线所有设备 ───────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LoginHistoryQuery {
    #[serde(default = "default_login_page")]
    pub page: i64,
    #[serde(default = "default_login_page_size")]
    pub page_size: i64,
}

fn default_login_page() -> i64 {
    1
}

fn default_login_page_size() -> i64 {
    10
}

/// GET /user/login_history?page=1&page_size=10
///
/// 当前用户**自己**的登录记录（按时间倒序）。成功与失败都记：用户看到
/// 「有人试过我的密码」比事后翻审计日志直观得多。
pub async fn login_history(
    claims: ValidatedClaims,
    Query(query): Query<LoginHistoryQuery>,
) -> ZapJsonResult {
    let (rows, total) = login_history::list(claims.id as i64, query.page, query.page_size).await?;
    Ok(Json(json!({
        "code": 0,
        "data": rows,
        "total": total,
    })))
}

/// POST /user/logout_all — 下线该用户名下的所有设备。
///
/// 会话版本号 +1，此前签发的凭据（面板 token、Web 应用 Cookie、静态 API Token）
/// 在下次请求时一律被判为已下线。**当前设备不下线**：换发一个带新版本号的
/// token 与会话 Cookie，否则调用者会被自己的操作踢出去。
pub async fn logout_all_devices(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
) -> Result<(HeaderMap, Json<serde_json::Value>), ZapError> {
    let version = session::bump(claims.id as i64).await?;

    // 版本号已递增，这里签发的 token 会自动带上新值（见 jwt::generate_jwt_token）
    let token = zap::jwt::generate_jwt_token(claims.sub.clone(), claims.id, &claims.roles)
        .map_err(|_| ZapError::New(-1, "Token 生成失败".to_string()))?;
    let cookie_token = webapp_session_token(claims.sub.clone(), claims.id, &claims.roles)
        .unwrap_or_else(|| token.clone());

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "logout_all_devices",
        &claims.sub,
        &format!("token_version={version}"),
    )
    .await;

    Ok((
        session_cookie(&cookie_token),
        Json(json!({
            "code": 0,
            "message": "已下线其它所有设备",
            "access_token": token,
            "token_type": "Bearer",
            "expire_in": crate::config::get_config().read().unwrap().jwt.jwt_expire,
        })),
    ))
}

/// Change password — uses `Claims` (not `ValidatedClaims`) so it works even
/// when the user is still on the default password.
#[derive(Debug, Deserialize)]
pub struct ChangePasswordPayload {
    pub old_password: String,
    pub new_password: String,
}

pub async fn change_password(
    claims: zap::jwt::Claims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<ChangePasswordPayload>,
) -> ZapJsonResult {
    if payload.new_password.len() < 6 {
        return Err(ZapError::New(-1, "新密码长度不能少于6位".to_string()));
    }
    if payload.old_password == payload.new_password {
        return Err(ZapError::New(-1, "新密码不能与旧密码相同".to_string()));
    }

    let pool = db::get_db_pool().await;

    // Verify old password
    let row: Result<(String,), sqlx::Error> =
        sqlx::query_as("SELECT password FROM user WHERE id = ?")
            .bind(claims.id as i64)
            .fetch_one(pool)
            .await;

    match row {
        Ok((stored_hash,)) => {
            if !bcrypt::verify(&payload.old_password, &stored_hash).unwrap_or(false) {
                audit::log(
                    Some(&claims),
                    None,
                    "password_change_failed",
                    &claims.sub,
                    "旧密码错误",
                )
                .await;
                return Err(ZapError::New(-1, "旧密码错误".to_string()));
            }
        }
        Err(_) => return Err(ZapError::New(-1, "用户不存在".to_string())),
    }

    // Hash new password
    let new_hash = bcrypt::hash(&payload.new_password, bcrypt::DEFAULT_COST)
        .map_err(|e| ZapError::Error(format!("密码加密失败: {}", e)))?;

    // Update password
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE user SET password = ?, updated_at = ? WHERE id = ?")
        .bind(&new_hash)
        .bind(now)
        .bind(claims.id as i64)
        .execute(pool)
        .await?;

    // 改密后其它设备一并下线：会话版本号 +1（随后换发的 token 带新版本号，
    // 因此当前设备不会被自己踢出去）
    session::bump(claims.id as i64).await?;

    // 改密后换发 token（版本号已在上面推高，新 token 带当前版本号）
    let token = zap::jwt::generate_jwt_token(claims.sub.clone(), claims.id, &claims.roles)
        .map_err(|_| ZapError::New(-1, "Token 生成失败".to_string()))?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "password_change",
        &claims.sub,
        "",
    )
    .await;

    // 密码变更站内信（是否发送取决于该用户通知偏好，默认发送）
    crate::zap::notify::password_changed(claims.id as i64, &claims.sub).await;

    tracing::info!("Password changed for user {}", claims.sub);
    Ok(Json(json!({
        "code": 0,
        "message": "密码修改成功",
        "access_token": token,
        "token_type": "Bearer",
    })))
}

pub async fn reflash_token(
    claims: zap::jwt::Claims,
) -> Result<(HeaderMap, Json<serde_json::Value>), ZapError> {
    // 页面会话（长有效期）先算好：claims 随后会被 move 进 JWT 生成
    let cookie_token = webapp_session_token(claims.sub.clone(), claims.id, &claims.roles);
    if let Ok(token) = zap::jwt::generate_jwt_token(claims.sub, claims.id, &claims.roles) {
        let cookie = cookie_token.unwrap_or_else(|| token.clone());
        let headers = session_cookie(&cookie);
        return Ok((
            headers,
            Json(json!({
                "code": 0,
                "access_token": token,
                "token_type": "Bearer",
                "message": "刷新成功",
                "expire_in": crate::config::get_config().read().unwrap().jwt.jwt_expire
            })),
        ));
    }
    Err(ZapError::New(-1, "刷新失败".to_string()))
}

// ── TOTP 2FA ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TotpCodePayload {
    pub code: String,
}

/// GET /auth/totp/setup — 生成密钥与 otpauth URL（未启用时）。
pub async fn totp_setup(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let (enabled, existing): (i32, String) =
        sqlx::query_as("SELECT totp_enabled, totp_secret FROM user WHERE id = ?")
            .bind(claims.id as i64)
            .fetch_one(pool)
            .await?;
    if enabled == 1 {
        return Err(ZapError::New(
            -1,
            "两步验证已启用，如需更换请先关闭".to_string(),
        ));
    }
    // 复用已有未启用密钥，避免每次打开都更换
    if existing.is_empty() {
        let secret = totp::generate_secret();
        let _ = sqlx::query("UPDATE user SET totp_secret = ? WHERE id = ?")
            .bind(&secret)
            .bind(claims.id as i64)
            .execute(pool)
            .await;
        let url = totp::otpauth_url(&secret, &claims.sub);
        return Ok(Json(
            json!({ "code": 0, "data": { "secret": secret, "otpauth_url": url } }),
        ));
    }
    let url = totp::otpauth_url(&existing, &claims.sub);
    Ok(Json(
        json!({ "code": 0, "data": { "secret": existing, "otpauth_url": url } }),
    ))
}

/// POST /auth/totp/verify — 校验验证码并启用两步验证。
pub async fn totp_verify(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<TotpCodePayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let (enabled, secret): (i32, String) =
        sqlx::query_as("SELECT totp_enabled, totp_secret FROM user WHERE id = ?")
            .bind(claims.id as i64)
            .fetch_one(pool)
            .await?;
    if enabled == 1 {
        return Err(ZapError::New(-1, "两步验证已启用".to_string()));
    }
    if secret.is_empty() {
        return Err(ZapError::New(-1, "请先获取验证密钥".to_string()));
    }
    if !totp::verify(&secret, &payload.code) {
        return Err(ZapError::New(-1, "验证码错误或已失效".to_string()));
    }
    sqlx::query("UPDATE user SET totp_enabled = 1 WHERE id = ?")
        .bind(claims.id as i64)
        .execute(pool)
        .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "totp_enable",
        &claims.sub,
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "两步验证已启用" })))
}

/// POST /auth/totp/disable — 校验验证码后关闭两步验证。
pub async fn totp_disable(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<TotpCodePayload>,
) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let (enabled, secret): (i32, String) =
        sqlx::query_as("SELECT totp_enabled, totp_secret FROM user WHERE id = ?")
            .bind(claims.id as i64)
            .fetch_one(pool)
            .await?;
    if enabled == 0 {
        return Err(ZapError::New(-1, "两步验证未启用".to_string()));
    }
    if !totp::verify(&secret, &payload.code) {
        return Err(ZapError::New(-1, "验证码错误或已失效".to_string()));
    }
    sqlx::query("UPDATE user SET totp_enabled = 0, totp_secret = '' WHERE id = ?")
        .bind(claims.id as i64)
        .execute(pool)
        .await?;

    audit::log(
        Some(&claims),
        Some(client_addr.ip().to_string().as_str()),
        "totp_disable",
        &claims.sub,
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": "两步验证已关闭" })))
}

/// GET /auth/totp/status — 查询两步验证开启状态。
pub async fn totp_status(claims: ValidatedClaims) -> ZapJsonResult {
    let pool = db::get_db_pool().await;
    let (enabled,): (i32,) = sqlx::query_as("SELECT totp_enabled FROM user WHERE id = ?")
        .bind(claims.id as i64)
        .fetch_one(pool)
        .await?;
    Ok(Json(
        json!({ "code": 0, "data": { "enabled": enabled == 1 } }),
    ))
}
