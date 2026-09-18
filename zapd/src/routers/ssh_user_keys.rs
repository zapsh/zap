//! 「我的 SSH 密钥」：面板用户自建 SSH 密钥（普通用户到 admin 均可）。
//!
//! 密钥**只以家目录文件的形式存在**：`~/.ssh/zap_<name>`（私钥）与
//! `~/.ssh/zap_<name>.pub`（公钥），**不落数据库**。列表一律由 zapexec 扫描磁盘得到，
//! 因此库重建/迁移后「我的密钥」依旧完整，DB 与家目录不会失联。
//!
//! 可见性规则与连接一致（严格隔离）：任何角色只管理自己名下的密钥
//! （归属的 Linux 账号由 `user.linux_user` 决定）。

use axum::Json;
use axum::extract::Query;
use serde::Deserialize;
use serde_json::{Value, json};

use zap_proto::Request;

use crate::db;
use crate::zap::jwt::ValidatedClaims;
use crate::zap::{ZapError, ZapJsonResult};

/// 校验密钥名：字母数字开头，仅允许字母数字 `-` `_`，最长 64（与 zapexec 一致）。
fn valid_key_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    first_ok
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 当前登录用户绑定的系统用户（linux_user）。无绑定则拒绝密钥管理。
async fn require_linux_user(claims: &ValidatedClaims) -> Result<String, ZapError> {
    let pool = db::get_db_pool().await;
    let lu: Option<String> = sqlx::query_scalar("SELECT linux_user FROM user WHERE id = ?")
        .bind(claims.id as i64)
        .fetch_optional(pool)
        .await?;
    match lu {
        Some(u) if !u.is_empty() => Ok(u),
        _ => Err(ZapError::New(
            -1,
            "当前账号未绑定系统用户，无法管理 SSH 密钥".to_string(),
        )),
    }
}

// ── GET /terminal/keys ─────────────────────────────────────

/// 我的密钥列表：本人（家目录扫描）+ (admin) 系统级密钥。
/// data.items：密钥列表（运行模式固定为独立系统用户，每个用户都有家目录 ~/.ssh）。
pub async fn list_keys(claims: ValidatedClaims) -> ZapJsonResult {
    let mut items: Vec<Value> = Vec::new();

    // 本人「我的密钥」：由 zapexec 扫描家目录 ~/.ssh/zap_*.pub 得出（不查 DB，
    // 密钥只以文件形式存在，库重建后列表依然完整）
    if let Ok(linux_user) = require_linux_user(&claims).await
        && let Ok(resp) = crate::zapexec::call(Request::SshUserKeyList { linux_user }).await
        && resp.code == 0
        && let Some(arr) = resp
            .data
            .as_ref()
            .and_then(|d| d.get("items"))
            .and_then(|v| v.as_array())
    {
        for v in arr {
            let name = v.get("name").and_then(|x| x.as_str()).unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            items.push(json!({
                "name": name,
                "scope": "user",
                "comment": v.get("comment").and_then(|x| x.as_str()).unwrap_or_default(),
                "fingerprint": v.get("fingerprint").and_then(|x| x.as_str()).unwrap_or_default(),
                "created_at": v.get("created_at").and_then(|x| x.as_i64()).unwrap_or(0),
            }));
        }
    }

    Ok(Json(json!({
        "code": 0,
        "data": {
            "items": items,
        }
    })))
}

// ── POST /terminal/keys/generate ───────────────────────────

#[derive(Debug, Deserialize)]
pub struct GeneratePayload {
    pub name: String,
    #[serde(default)]
    pub key_type: Option<String>,
    #[serde(default)]
    pub bits: Option<u32>,
    #[serde(default)]
    pub comment: Option<String>,
}

pub async fn generate_key(
    claims: ValidatedClaims,
    Json(payload): Json<GeneratePayload>,
) -> ZapJsonResult {
    let name = payload.name.trim().to_string();
    if !valid_key_name(&name) {
        return Err(ZapError::New(
            -1,
            "无效的密钥名称（仅允许字母/数字/-/_，最长 64 字符）".to_string(),
        ));
    }
    let linux_user = require_linux_user(&claims).await?;
    // 同名校验由 zapexec 完成（家目录 ~/.ssh/zap_<name> 是否已存在）：密钥只存文件，不落 DB
    let resp = crate::zapexec::call(Request::SshUserKeyGenerate {
        linux_user: linux_user.clone(),
        name: name.clone(),
        key_type: payload.key_type,
        bits: payload.bits,
        comment: payload.comment,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    audit_log(
        &claims,
        "ssh_user_key_generate",
        &format!("生成密钥 {name}（{linux_user}）"),
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "密钥已生成并保存到我的家目录 ~/.ssh" }),
    ))
}

// ── POST /terminal/keys/import ─────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ImportPayload {
    pub name: String,
    pub private_key: String,
    #[serde(default)]
    pub public_key: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
}

pub async fn import_key(
    claims: ValidatedClaims,
    Json(payload): Json<ImportPayload>,
) -> ZapJsonResult {
    let name = payload.name.trim().to_string();
    if !valid_key_name(&name) {
        return Err(ZapError::New(
            -1,
            "无效的密钥名称（仅允许字母/数字/-/_，最长 64 字符）".to_string(),
        ));
    }
    let linux_user = require_linux_user(&claims).await?;
    // 同名校验由 zapexec 完成（家目录 ~/.ssh/zap_<name> 是否已存在）：密钥只存文件，不落 DB
    let resp = crate::zapexec::call(Request::SshUserKeyImport {
        linux_user: linux_user.clone(),
        name: name.clone(),
        private_key: payload.private_key,
        public_key: payload.public_key,
        comment: payload.comment,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    audit_log(
        &claims,
        "ssh_user_key_import",
        &format!("导入密钥 {name}（{linux_user}）"),
    )
    .await;
    Ok(Json(
        json!({ "code": 0, "message": "密钥已导入并保存到我的家目录 ~/.ssh" }),
    ))
}

// ── POST /terminal/keys/delete ─────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DeletePayload {
    pub name: String,
}

pub async fn delete_key(
    claims: ValidatedClaims,
    Json(payload): Json<DeletePayload>,
) -> ZapJsonResult {
    let name = payload.name.trim().to_string();
    if !valid_key_name(&name) {
        return Err(ZapError::New(
            -1,
            "无效的密钥名称（仅允许字母/数字/-/_，最长 64 字符）".to_string(),
        ));
    }
    // 归属由 linux_user 决定（本人家目录），存在性由 zapexec 校验
    let linux_user = require_linux_user(&claims).await?;
    let resp = crate::zapexec::call(Request::SshUserKeyDelete {
        linux_user,
        name: name.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }

    audit_log(&claims, "ssh_user_key_delete", &format!("删除密钥 {name}")).await;
    Ok(Json(json!({ "code": 0, "message": "密钥已删除" })))
}

// ── GET /terminal/keys/public?name= ────────────────────────

#[derive(Debug, Deserialize)]
pub struct KeyNameQuery {
    pub name: String,
}

pub async fn public_key(claims: ValidatedClaims, Query(q): Query<KeyNameQuery>) -> ZapJsonResult {
    let name = q.name.trim().to_string();
    let linux_user = require_linux_user(&claims).await?;
    let resp = crate::zapexec::call(Request::SshUserKeyPublicGet { linux_user, name }).await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    let data = resp.data.unwrap_or(Value::Null);
    Ok(Json(json!({
        "code": 0,
        "data": {
            "public_key": data.get("public_key").and_then(|v| v.as_str()).unwrap_or_default(),
            "comment": data.get("comment").and_then(|v| v.as_str()).unwrap_or_default(),
            "fingerprint": data.get("fingerprint").and_then(|v| v.as_str()).unwrap_or_default(),
        }
    })))
}

// ── GET /terminal/keys/private?name= ───────────────────────

/// 下载自己的私钥（经 zapexec root 读取家目录文件，仅本人可操作）。
pub async fn private_key(claims: ValidatedClaims, Query(q): Query<KeyNameQuery>) -> ZapJsonResult {
    let name = q.name.trim().to_string();
    let linux_user = require_linux_user(&claims).await?;
    let resp = crate::zapexec::call(Request::SshUserKeyPrivateGet {
        linux_user,
        name: name.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    let data = resp.data.unwrap_or(Value::Null);
    let private_key = data
        .get("private_key")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    Ok(Json(
        json!({ "code": 0, "data": { "name": name, "private_key": private_key } }),
    ))
}

async fn audit_log(claims: &ValidatedClaims, action: &str, detail: &str) {
    crate::zap::audit::log(Some(claims), None, action, detail, "用户 SSH 密钥管理").await;
}
