// SPDX-License-Identifier: AGPL-3.0-only
//! 服务凭据读取（root）。
//!
//! 凭据由 `zapctl cred gen <服务> <用户>` 生成，加密存放在
//! `/etc/zap/credentials/{service}_{user}.cred`（文件 0400、目录 0700）。
//! 本模块负责「读文件 + 解密」，供 zapd 在创建数据库 / 初始化服务时取回密码。
//!
//! 安全边界：service / user 只允许 `[A-Za-z0-9._-]`（`zap_crypto::sanitize_name`），
//! 因此不存在路径穿越；只读取、不写入，也不传递任何用户输入命令。

use serde_json::json;
use zap_proto::Response;

/// cred.read：读取并解密指定凭据。
pub async fn read(service: &str, user: &str) -> Response {
    match zap_crypto::read_cred(service, user) {
        Ok(password) => Response::ok(
            "ok",
            Some(json!({
                "service": service,
                "user": user,
                "password": password,
            })),
        ),
        Err(e) => Response::err(-1, e),
    }
}
