//! 面板「镜像构建」的落盘约定、命名空间规则与路径边界。
//!
//! 构建是**长任务**（分钟级起步），所以和 appstore / crontab 用同一套约定：
//!
//! - 日志：`{data}/users/<username>/docker-build-logs/run-<run_id>.log`
//!   （zapexec 只允许写 `data/users/` 之下、且以 `.log` 结尾的路径）
//! - 运行记录：登记进 `appstore_runs`，`job_key = docker-build:<username>`，
//!   于是 `/appstore/runs` 能看到自己的构建历史、`/appstore/ws/{run_id}` 能流式看日志
//!   （该端点已带「同一用户 / admin 才可读」的校验）
//!
//! 两条多用户边界，都在这里收敛：
//!
//! 1. **命名空间**：非 admin 的目标镜像强制落在 `<命名空间>/...` 下，命名空间由面板
//!    用户名派生（`zap_proto::docker_namespace`）。多个用户各自构建 `nginx` 时不会互相
//!    覆盖，管理员也能一眼看出镜像归属。
//! 2. **路径**：非 admin 的构建上下文 / Containerfile 必须位于**自己的家目录**内；
//!    admin 可选任意系统路径（沿用文件管理器对 admin 的放开程度）。

use std::path::PathBuf;

use crate::zap::ZapError;
use crate::zap::appstore as ast;
use crate::zap::jwt::{ValidatedClaims, is_admin};
use crate::zap::user_cron;

/// 单次构建最多接受的镜像名（含主名）。
pub const MAX_TAGS: usize = 8;

/// 构建日志目录：`{data}/users/<username>/docker-build-logs`。
pub fn logs_dir(username: &str) -> PathBuf {
    user_cron::users_dir()
        .join(username)
        .join("docker-build-logs")
}

/// 单次构建的日志文件路径（传给 zapexec 落盘）。
pub fn log_path(username: &str, run_id: &str) -> String {
    logs_dir(username)
        .join(format!("run-{run_id}.log"))
        .to_string_lossy()
        .into_owned()
}

/// 运行记录在 `appstore_runs` 中的归属键：同一用户的构建历史串在一起。
pub fn job_key(username: &str) -> String {
    format!("docker-build:{username}")
}

/// 运行记录里的动作名（前端据此筛出自己的构建历史）。
pub const RUN_ACTION: &str = "docker_image_build";

/// 给镜像名补上用户命名空间；已经带该前缀则原样返回。
///
/// 「幂等」是刻意的：前端会把前缀直接拼进输入框的回显里，用户也可能自己再写一遍，
/// 两种写法都必须落到同一个镜像名。
fn with_namespace(ns: &str, tag: &str) -> String {
    let prefix = format!("{ns}/");
    if tag.starts_with(&prefix) {
        tag.to_string()
    } else {
        format!("{prefix}{tag}")
    }
}

/// 目标镜像名收敛：补 `:latest`、去重、限个数，非 admin 强制加命名空间。
pub fn scope_tags(
    claims: &ValidatedClaims,
    name: &str,
    extra: &[String],
) -> Result<Vec<String>, ZapError> {
    let ns = zap_proto::docker_namespace(&claims.sub);
    let free_namespace = is_admin(claims);
    let mut out: Vec<String> = Vec::new();
    for raw in std::iter::once(name).chain(extra.iter().map(String::as_str)) {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let tag = zap_proto::normalize_image_tag(raw);
        if !zap_proto::valid_image_ref(&tag) {
            return Err(ZapError::New(
                -1,
                format!("镜像名不合法: {raw}（只允许小写字母数字与 . _ - /，可带 :tag）"),
            ));
        }
        let tag = if free_namespace {
            tag
        } else {
            with_namespace(&ns, &tag)
        };
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    if out.is_empty() {
        return Err(ZapError::New(-1, "镜像名不能为空".to_string()));
    }
    if out.len() > MAX_TAGS {
        return Err(ZapError::New(
            -1,
            format!("镜像名过多（上限 {MAX_TAGS} 个）"),
        ));
    }
    Ok(out)
}

/// 校验通过后的构建路径。
pub struct BuildPaths {
    /// 构建上下文目录（已解析符号链接）
    pub context: PathBuf,
    /// Containerfile（已解析符号链接，且一定位于 `context` 之内）
    pub containerfile: PathBuf,
}

/// 解析并校验构建上下文 / Containerfile。
///
/// 一律走 `canonicalize`（解析符号链接）后再做包含判断：否则用户可以在家目录里放一个
/// 指向 `/` 的软链，绕开「只能选家目录」的限制。
pub fn resolve_paths(
    claims: &ValidatedClaims,
    home_dir: &str,
    context_dir: &str,
    containerfile: &str,
) -> Result<BuildPaths, ZapError> {
    let context_in = context_dir.trim();
    if context_in.is_empty() {
        return Err(ZapError::New(-1, "构建上下文目录不能为空".to_string()));
    }
    let context = canonical_dir(context_in, "构建上下文目录")?;

    // Containerfile 留空时按 docker 的习惯自动找：Dockerfile 优先，其次 Containerfile
    let containerfile_in = containerfile.trim();
    let containerfile = if containerfile_in.is_empty() {
        let mut found = None;
        for name in ["Dockerfile", "Containerfile"] {
            let candidate = context.join(name);
            if candidate.is_file() {
                found = Some(candidate);
                break;
            }
        }
        match found {
            Some(p) => p,
            None => {
                return Err(ZapError::New(
                    -1,
                    format!(
                        "{} 下没有 Dockerfile / Containerfile，请手动指定",
                        context.display()
                    ),
                ));
            }
        }
    } else {
        canonical_file(containerfile_in, "Containerfile")?
    };

    if !containerfile.starts_with(&context) {
        return Err(ZapError::New(
            -1,
            "Containerfile 必须位于构建上下文目录之内".to_string(),
        ));
    }

    // 普通用户只能在自家目录里构建
    if !is_admin(claims) {
        let home = home_dir.trim();
        if home.is_empty() {
            return Err(ZapError::New(
                -1,
                "当前账号的家目录尚未初始化，无法构建镜像".to_string(),
            ));
        }
        let home = canonical_dir(home, "家目录")?;
        if !context.starts_with(&home) {
            return Err(ZapError::New(
                -1,
                format!("普通用户只能在自家目录内构建镜像: {}", home.display()),
            ));
        }
    }

    Ok(BuildPaths {
        context,
        containerfile,
    })
}

/// 绝对 + 存在 + 是目录，并解析符号链接。
fn canonical_dir(path: &str, what: &str) -> Result<PathBuf, ZapError> {
    let p = PathBuf::from(path.trim());
    if !p.is_absolute() {
        return Err(ZapError::New(-1, format!("{what}必须是绝对路径: {path}")));
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ZapError::New(-1, format!("{what}路径不合法: {path}")));
    }
    let real = std::fs::canonicalize(&p)
        .map_err(|e| ZapError::New(-1, format!("{what}不可访问: {path}（{e}）")))?;
    if !real.is_dir() {
        return Err(ZapError::New(-1, format!("{what}不是目录: {path}")));
    }
    Ok(real)
}

/// 绝对 + 存在 + 是普通文件，并解析符号链接。
fn canonical_file(path: &str, what: &str) -> Result<PathBuf, ZapError> {
    let p = PathBuf::from(path.trim());
    if !p.is_absolute() {
        return Err(ZapError::New(-1, format!("{what}必须是绝对路径: {path}")));
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ZapError::New(-1, format!("{what}路径不合法: {path}")));
    }
    let real = std::fs::canonicalize(&p)
        .map_err(|e| ZapError::New(-1, format!("{what}不存在: {path}（{e}）")))?;
    if !real.is_file() {
        return Err(ZapError::New(-1, format!("{what}不是文件: {path}")));
    }
    Ok(real)
}

/// 解析日志尾部的完成标记：`__ZAP_DONE__ <exit_code>`。
fn read_done_marker(log: &str) -> Option<i64> {
    let content = std::fs::read_to_string(log).ok()?;
    content.lines().rev().find_map(|line| {
        line.trim()
            .strip_prefix("__ZAP_DONE__ ")?
            .trim()
            .parse::<i64>()
            .ok()
    })
}

/// 后台兜底：盯住日志，出现完成标记就落定运行记录状态。
///
/// `/appstore/ws/{run_id}` 在流结束时也会更新状态，但那要求「有人正在看日志」；
/// 用户点完构建就关掉抽屉、或直接刷新页面的场景需要这里兜底，
/// 否则记录会一直停在 `running`。
pub fn watch_run(run_id: String, log: String) {
    tokio::spawn(async move {
        // 构建可能很久（拉基础镜像 + 编译），给足 6 小时再判超时
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(6 * 3600);
        loop {
            if let Some(code) = read_done_marker(&log) {
                ast::finish_run(&run_id, if code == 0 { "success" } else { "failed" }, code).await;
                break;
            }
            if tokio::time::Instant::now() > deadline {
                ast::finish_run(&run_id, "failed", -1).await;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zap::jwt::Claims;

    fn claims(username: &str, roles: &str) -> ValidatedClaims {
        ValidatedClaims(Claims {
            id: 1,
            iat: 0,
            sub: username.to_string(),
            iss: "zap".to_string(),
            exp: 0,
            roles: roles.to_string(),
            pwd_is_default: false,
        })
    }

    #[test]
    fn admin_keeps_name_as_is() {
        let tags = scope_tags(
            &claims("admin", "admin"),
            "nginx",
            &["nginx:1.27".to_string()],
        )
        .unwrap();
        assert_eq!(tags, vec!["nginx:latest", "nginx:1.27"]);
    }

    #[test]
    fn normal_user_gets_namespace_prefix() {
        let tags = scope_tags(&claims("demo", "user"), "nginx", &[]).unwrap();
        assert_eq!(tags, vec!["demo/nginx:latest"]);
    }

    #[test]
    fn namespace_prefix_is_idempotent() {
        // 前端回显里已经带了前缀时，不能变成 demo/demo/nginx
        let tags = scope_tags(&claims("Demo", "user"), "demo/nginx:latest", &[]).unwrap();
        assert_eq!(tags, vec!["demo/nginx:latest"]);
    }

    #[test]
    fn scope_tags_dedupes_and_normalizes() {
        let tags = scope_tags(
            &claims("demo", "user"),
            "nginx",
            &[
                "nginx".to_string(),
                "nginx:1.27".to_string(),
                "  ".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(tags, vec!["demo/nginx:latest", "demo/nginx:1.27"]);
    }

    #[test]
    fn scope_tags_rejects_bad_names() {
        assert!(scope_tags(&claims("demo", "user"), "Nginx", &[]).is_err());
        assert!(scope_tags(&claims("demo", "user"), "nginx:bad tag", &[]).is_err());
        assert!(scope_tags(&claims("demo", "user"), "", &[]).is_err());
    }

    #[test]
    fn scope_tags_caps_tag_count() {
        let extra: Vec<String> = (0..MAX_TAGS).map(|i| format!("nginx:{i}")).collect();
        assert!(scope_tags(&claims("demo", "user"), "nginx", &extra).is_err());
    }
}
