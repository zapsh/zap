use std::time::{self, UNIX_EPOCH};

use axum::{
    Json, RequestPartsExt,
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use jsonwebtoken::{
    DecodingKey, EncodingKey, Header, Validation, decode, encode,
    errors::{Error, ErrorKind},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use sha2::{Digest, Sha256};

use crate::{config, db};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub id: u64,       // uid
    pub iat: u64,      // 签发时间
    pub sub: String,   // 签发给
    pub iss: String,   // 发布者
    pub exp: u64,      // 过期时间
    pub roles: String, // 用户角色，逗号分隔
    /// 会话版本号（对应 `user.token_version`，序列化为 `tv`）。
    ///
    /// 小于库中当前值的 token 一律视为已下线，用于「下线所有设备」。
    /// 带 `default` 是为了兼容**本次改动之前签发**的 token（payload 里没有该字段）：
    /// 它们反序列化后得到 0，而存量用户的版本号同样从 0 起算，因此不会被误杀。
    #[serde(default, rename = "tv")]
    pub token_version: i64,
    /// 凭据作用域（`api_token.scope`）：`''` 普通用户 Token，`'cluster'` 集群节点机器凭据。
    ///
    /// 带 `default` 是为了兼容本次改动之前签发的 JWT（payload 里没有该字段）。
    /// `cluster` 凭据由 `access::guard` 收口，只能访问 `/pro/cluster/agent/**`。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
}

/// Wrapper around Claims：签名与会话版本号（tokenVersion）校验通过即放行。
pub struct ValidatedClaims(pub Claims);

impl std::ops::Deref for ValidatedClaims {
    type Target = Claims;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
pub enum AuthError {
    ExpiredSignature,
    WrongCredentials,
    MissingCredentials,
    TokenCreation,
    InvalidToken,
    /// 会话版本号落后于库中当前值：该 token 已被「下线所有设备」作废。
    TokenRevoked,
}

#[derive(Debug, Serialize)]
pub struct AuthBody {
    access_token: String,
    token_type: String,
}

pub fn generate_jwt_token(username: String, id: u64, roles: &str) -> Result<String, Error> {
    let expire = config::get_config().read().unwrap().jwt.jwt_expire;
    generate_jwt_token_with_expire(username, id, roles, expire)
}

/// 指定有效期的签发。
///
/// 用于 Web 应用（`/webapps/*`）会话 Cookie 这类需要比面板 JWT 更长生命周期的场景：
/// 面板 token 保持短有效期（如 1 小时），而页面级会话可以更长，
/// 否则用户登录一小时后再打开页面就会提示需要重新登录。
pub fn generate_jwt_token_with_expire(
    username: String,
    id: u64,
    roles: &str,
    expire: u64,
) -> Result<String, Error> {
    let now_secs = time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = Claims {
        iat: now_secs,
        sub: username,
        iss: "Zap".to_string(),
        id,
        exp: now_secs + expire,
        roles: roles.to_string(),
        token_version: crate::zap::session::version_of(id),
        scope: String::new(),
    };
    let secure_key = &config::get_config().read().unwrap().jwt.jwt_secure;
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secure_key.as_ref()),
    )
}

/// 凭据作用域：一次性终端票据（v3 一键 SSH，由**被控端自己**签发）。
///
/// 带这个 scope 的 token 只能访问终端 WebSocket，见 `access::guard`。
pub const SSH_SCOPE: &str = "ssh";

/// 签发一枚**限定作用域**的短时效 JWT。
///
/// 与 [`generate_jwt_token_with_expire`] 的唯一区别是可以指定 `scope` 与 `sub`：
/// 终端票据用它把凭据绑死在一条 SSH 连接上（`sub = "ssh:{conn_id}"`），
/// 并让 `access::guard` 能把它收口到终端接口。
pub fn generate_scoped_jwt(
    id: u64,
    sub: &str,
    roles: &str,
    expire: u64,
    scope: &str,
) -> Result<String, Error> {
    let now_secs = time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = Claims {
        iat: now_secs,
        sub: sub.to_string(),
        iss: "Zap".to_string(),
        id,
        exp: now_secs + expire,
        roles: roles.to_string(),
        token_version: crate::zap::session::version_of(id),
        scope: scope.to_string(),
    };
    let secure_key = &config::get_config().read().unwrap().jwt.jwt_secure;
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secure_key.as_ref()),
    )
}

/// Check if the claims contain the admin role
pub fn is_admin(claims: &Claims) -> bool {
    claims.roles.split(',').any(|r| r.trim() == "admin")
}

/// Check if the claims contain the reseller role
pub fn is_reseller(claims: &Claims) -> bool {
    claims.roles.split(',').any(|r| r.trim() == "reseller")
}

/// Check if the claims contain the demo role（只读演示账号）
pub fn is_demo(claims: &Claims) -> bool {
    claims.roles.split(',').any(|r| r.trim() == "demo")
}

/// 静态 API Token 前缀（`zap_` 开头，用于区分 JWT）
pub const API_TOKEN_PREFIX: &str = "zap_";

/// SHA-256 十六进制摘要（API Token 在 DB 中仅存哈希，不落明文）
pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

/// 解析并校验一个 JWT：签名、有效期，外加会话版本号（tokenVersion）。
///
/// 与 [`Claims`] extractor 走同一套规则，供不走 extractor 的入口复用——
/// WebSocket 握手、下载/流式接口用的 `?token=` 等都是自己 decode 的，
/// 统一调这里才不会漏掉「已下线」的校验。
pub fn decode_verified(raw_token: &str) -> Option<Claims> {
    let secure_key = config::get_config().read().ok()?.jwt.jwt_secure.clone();
    let claims = decode::<Claims>(
        raw_token,
        &DecodingKey::from_secret(secure_key.as_bytes()),
        &Validation::default(),
    )
    .ok()?
    .claims;
    if crate::zap::session::version_of(claims.id) > claims.token_version {
        return None;
    }
    Some(claims)
}

/// 解析 Bearer 凭据字符串为 claims（支持 JWT 与静态 API Token，供只读守卫等中间件使用）。
/// 入参应为已剥离 `Bearer ` 前缀的 token，调用方需在 await 前持有 owned String，避免借用跨 await。
pub async fn claims_from_token(raw_token: &str) -> Option<Claims> {
    if raw_token.starts_with(API_TOKEN_PREFIX) {
        return resolve_api_token(raw_token).await;
    }
    decode_verified(raw_token)
}

// ── Claims extractor (allows default-password users through) ────────────────

impl<S> FromRequestParts<S> for Claims
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        extract_claims(parts).await
    }
}

// ── ValidatedClaims extractor ───────────────────────────────

impl<S> FromRequestParts<S> for ValidatedClaims
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let claims = extract_claims(parts).await?;
        Ok(ValidatedClaims(claims))
    }
}

async fn extract_claims(parts: &mut Parts) -> Result<Claims, AuthError> {
    let TypedHeader(Authorization(bearer)) = parts
        .extract::<TypedHeader<Authorization<Bearer>>>()
        .await
        .map_err(|_| AuthError::MissingCredentials)?;

    let raw = bearer.token();

    // 静态 API Token（zap_ 前缀）→ 数据库校验
    if raw.starts_with(API_TOKEN_PREFIX) {
        return match resolve_api_token(raw).await {
            Some(claims) => Ok(claims),
            None => Err(AuthError::InvalidToken),
        };
    }

    let secure_key = &config::get_config().read().unwrap().jwt.jwt_secure;
    let token_data = decode::<Claims>(
        raw,
        &DecodingKey::from_secret(secure_key.as_ref()),
        &Validation::default(),
    )
    .map_err(|e| {
        if *e.kind() == ErrorKind::ExpiredSignature {
            return AuthError::ExpiredSignature;
        }
        AuthError::InvalidToken
    })?;

    let claims = token_data.claims;
    // 「下线所有设备」后，此前签发的 token 版本号落后，一律作废
    if crate::zap::session::version_of(claims.id) > claims.token_version {
        return Err(AuthError::TokenRevoked);
    }
    Ok(claims)
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AuthError::WrongCredentials => (StatusCode::UNAUTHORIZED, "Wrong credentials"),
            AuthError::MissingCredentials => (StatusCode::BAD_REQUEST, "Missing credentials"),
            AuthError::TokenCreation => (StatusCode::INTERNAL_SERVER_ERROR, "Token creation error"),
            AuthError::InvalidToken => (StatusCode::BAD_REQUEST, "Invalid token"),
            AuthError::ExpiredSignature => (StatusCode::UNAUTHORIZED, "Token 已过期，请重新登录"),
            AuthError::TokenRevoked => (StatusCode::UNAUTHORIZED, "已在其它设备下线，请重新登录"),
        };
        let body = Json(json!({
            "code": -1,
            "message": error_message,
        }));
        (status, body).into_response()
    }
}

// ── 静态 API Token 校验 ─────────────────────────────────────

#[derive(sqlx::FromRow)]
struct ApiTokenLookup {
    user_id: i64,
    username: String,
    roles: String,
    token_status: i64,
    user_status: i64,
    expires_at: i64,
    /// 该 Token 记录的会话版本号：创建后不变，「下线所有设备」时统一被推高，
    /// 于是版本号落后的 Token 在下次请求时判为已下线。
    token_version: i64,
    /// 作用域：`''` 普通用户 Token；`'cluster'` 集群节点机器凭据（只能访问集群上报接口）。
    scope: String,
}

/// 校验静态 API Token：按哈希查表，校验 Token/用户状态与有效期，返回等价 Claims。
async fn resolve_api_token(raw: &str) -> Option<Claims> {
    let pool = db::get_db_pool().await;
    let row: Option<ApiTokenLookup> = sqlx::query_as(
        "SELECT t.user_id, u.username, u.roles, t.status AS token_status,
                u.status AS user_status, t.expires_at, t.token_version, t.scope
         FROM api_token t JOIN user u ON u.id = t.user_id
         WHERE t.token_hash = ?",
    )
    .bind(sha256_hex(raw))
    .fetch_optional(pool)
    .await
    .ok()?;
    let r = row?;

    // 注：静态 Token 不参与会话版本号比对 —— 版本号只在「下线所有设备」时
    // 由 `session::bump` 直接推高到库里的行上（集群节点凭据 scope='cluster' 已被排除）。

    let now = time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    if r.token_status != 1 || r.user_status != 1 || (r.expires_at > 0 && r.expires_at <= now) {
        return None;
    }

    // 尽力更新最近使用时间（失败不阻断请求）
    let pool = db::get_db_pool().await;
    let _ = sqlx::query("UPDATE api_token SET last_used_at = ? WHERE token_hash = ?")
        .bind(now)
        .bind(sha256_hex(raw))
        .execute(pool)
        .await;

    let exp = if r.expires_at > 0 {
        r.expires_at as u64
    } else {
        // 永不过期的 Token：赋予足够远的 exp（10 年）
        now as u64 + 10 * 365 * 86400
    };
    Some(Claims {
        id: r.user_id as u64,
        iat: now as u64,
        sub: r.username,
        iss: "Zap".to_string(),
        exp,
        roles: r.roles,
        // 用 Token 自己记录的版本号：它与用户当前版本号相同即有效，
        // 「下线所有设备」把两边一起推高后，未同步的旧 Token 随即失效
        token_version: r.token_version,
        scope: r.scope,
    })
}
