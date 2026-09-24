use axum::Json;
use axum::extract::Query;
use serde_json::json;
use std::collections::HashMap;

use crate::zap::{self, ZapJsonResult, jwt::ValidatedClaims};

pub async fn system_info(_: ValidatedClaims) -> ZapJsonResult {
    zap::system_info::get_system_info().await
}

pub async fn system_status(
    claims: ValidatedClaims,
    q: Query<HashMap<String, String>>,
) -> ZapJsonResult {
    zap::system_info::get_system_status(claims, q).await
}

pub async fn system_overview(_: ValidatedClaims) -> ZapJsonResult {
    zap::system_info::get_system_overview().await
}

/// 读取构建期注入的 `VERGEN_*` 变量（见 build.rs）。
///
/// 缺失 / vergen 容错占位值统一回退 `unknown`，避免在 About 卡片上显示
/// `VERGEN_IDEMPOTENT_OUTPUT` 这类内部占位串。
fn build_meta(value: Option<&'static str>) -> &'static str {
    match value {
        Some(v) if !v.is_empty() && v != "VERGEN_IDEMPOTENT_OUTPUT" => v,
        _ => "unknown",
    }
}

/// 面板自身信息（About Zap）：版本、构建日期、git 提交、编译器版本等。
/// 静态信息，无需频繁刷新。
pub async fn about(_: ValidatedClaims) -> ZapJsonResult {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    Ok(Json(json!({
        "code": 0,
        "message": "OK",
        "data": {
            "version": env!("CARGO_PKG_VERSION"),
            "build_date": build_meta(option_env!("VERGEN_BUILD_DATE")),
            "build_timestamp": build_meta(option_env!("VERGEN_BUILD_TIMESTAMP")),
            "git_branch": build_meta(option_env!("VERGEN_GIT_BRANCH")),
            "git_sha": build_meta(option_env!("VERGEN_GIT_SHA")),
            "git_describe": build_meta(option_env!("VERGEN_GIT_DESCRIBE")),
            "git_commit_date": build_meta(option_env!("VERGEN_GIT_COMMIT_DATE")),
            "git_dirty": build_meta(option_env!("VERGEN_GIT_DIRTY")),
            "rustc_version": build_meta(option_env!("VERGEN_RUSTC_SEMVER")),
            "rustc_channel": build_meta(option_env!("VERGEN_RUSTC_CHANNEL")),
            "target_triple": build_meta(option_env!("VERGEN_CARGO_TARGET_TRIPLE")),
            "profile": profile,
            "license": env!("CARGO_PKG_LICENSE"),
            "docs_path": "/docs/manual",
            "api_docs_path": "/dev/api-docs",
        }
    })))
}
