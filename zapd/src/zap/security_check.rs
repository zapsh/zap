// SPDX-License-Identifier: AGPL-3.0-only
//! 安全检测：检查 Zap 运行环境的权限与配置安全。
//!
//! 设计目标：
//! - 给管理员一份「运行环境是否安全」的清单：运行身份是否 root、关键目录/文件权限、
//!   特权执行器(zapexec)是否可用等；
//! - 可扩展：后续新增「用户环境权限」「家目录权限」等检测，只需在 [`run_all`] 里加一类
//!   `category`（目前仅 `zap`），前端按 `category` 分组展示即可。
//!
//! 每条检查返回 [`SecurityCheck`]：前端按 `name_key` / `detail_key` / `suggestion_key`
//! 走 i18n，动态值通过 `*_params` 注入，做到中英文双语一致。

use std::os::unix::fs::MetadataExt;
use std::path::Path;

use serde::Serialize;
use serde_json::json;

use crate::config;
use crate::zap::appstore;
use crate::zap::user_cron;

/// 当前实现的分类（前端按此分组；后续新增分类在此登记）。
const CAT_ZAP: &str = "zap";

/// 单条检查结果（前端据此渲染）。
#[derive(Debug, Serialize, Clone)]
pub struct SecurityCheck {
    /// 稳定标识（前端可按需定制展示）。
    pub id: String,
    /// 原始分类 id（如 `zap`），用于分组。
    pub category: String,
    /// 分类标题的 i18n key。
    pub category_key: String,
    /// 检查项名称的 i18n key。
    pub name_key: String,
    /// 状态：`pass` / `warn` / `fail` / `info`。
    pub status: String,
    /// 详情模板的 i18n key。
    pub detail_key: String,
    /// 详情模板插值参数。
    pub detail_params: serde_json::Value,
    /// 修复建议模板的 i18n key（无建议则为 null）。
    pub suggestion_key: Option<String>,
    /// 修复建议模板插值参数。
    pub suggestion_params: serde_json::Value,
}

/// 构造一条检查结果。
struct MkArgs<'a> {
    id: &'a str,
    category: &'a str,
    category_key: &'a str,
    name_key: &'a str,
    status: &'a str,
    detail_key: &'a str,
    detail_params: serde_json::Value,
    suggestion_key: Option<&'a str>,
    suggestion_params: serde_json::Value,
}

fn mk(a: MkArgs) -> SecurityCheck {
    let MkArgs {
        id,
        category,
        category_key,
        name_key,
        status,
        detail_key,
        detail_params,
        suggestion_key,
        suggestion_params,
    } = a;
    SecurityCheck {
        id: id.to_string(),
        category: category.to_string(),
        category_key: category_key.to_string(),
        name_key: name_key.to_string(),
        status: status.to_string(),
        detail_key: detail_key.to_string(),
        detail_params,
        suggestion_key: suggestion_key.map(|s| s.to_string()),
        suggestion_params,
    }
}

/// 运行全部检测（当前仅 zap 分类；后续在内部追加 user / home 分类）。
pub async fn run_all() -> Vec<SecurityCheck> {
    let mut out = Vec::new();
    out.extend(zap_checks().await);
    // 后续：out.extend(user_env_checks().await);
    // 后续：out.extend(home_dir_checks().await);
    out
}

/// Zap 运行环境 / 权限相关检测。
async fn zap_checks() -> Vec<SecurityCheck> {
    let mut v = Vec::new();
    let uid = current_uid();

    // 1) 运行身份：是否以 root 运行
    if uid == 0 {
        v.push(mk(MkArgs {
            id: "zap_runtime_user",
            category: CAT_ZAP,
            category_key: "zapCfg.secCatZap",
            name_key: "zapCfg.secRuntimeUser",
            status: "fail",
            detail_key: "zapCfg.secRuntimeUserRoot",
            detail_params: json!({ "uid": uid }),
            suggestion_key: Some("zapCfg.secRuntimeUserRootSug"),
            suggestion_params: json!({}),
        }));
    } else {
        let name = username_of(uid);
        v.push(mk(MkArgs {
            id: "zap_runtime_user",
            category: CAT_ZAP,
            category_key: "zapCfg.secCatZap",
            name_key: "zapCfg.secRuntimeUser",
            status: "pass",
            detail_key: "zapCfg.secRuntimeUserOk",
            detail_params: json!({ "name": name, "uid": uid }),
            suggestion_key: None,
            suggestion_params: json!({}),
        }));
    }

    // 2) 数据目录可写（面板要落盘配置 / 数据）
    let data = user_cron::data_dir();
    let test_file = data.join(".zap_sec_test");
    let writable = match std::fs::write(&test_file, b"") {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_file);
            true
        }
        Err(_) => false,
    };
    if writable {
        v.push(mk(MkArgs {
            id: "zap_data_writable",
            category: CAT_ZAP,
            category_key: "zapCfg.secCatZap",
            name_key: "zapCfg.secDataWritable",
            status: "pass",
            detail_key: "zapCfg.secDataWritableOk",
            detail_params: json!({ "path": data.display().to_string() }),
            suggestion_key: None,
            suggestion_params: json!({}),
        }));
    } else {
        v.push(mk(MkArgs {
            id: "zap_data_writable",
            category: CAT_ZAP,
            category_key: "zapCfg.secCatZap",
            name_key: "zapCfg.secDataWritable",
            status: "fail",
            detail_key: "zapCfg.secDataWritableFail",
            detail_params: json!({ "path": data.display().to_string(), "uid": uid }),
            suggestion_key: Some("zapCfg.secDataWritableSug"),
            suggestion_params: json!({ "uid": uid }),
        }));
    }

    // 3) 配置文件（zap.yaml）权限：含 JWT 密钥，不应被其他用户读取/改写
    let cfg = config::config_path();
    if let Ok(meta) = std::fs::metadata(&cfg) {
        let mode = meta.mode();
        let modestr = mode_str(mode);
        if mode & 0o002 != 0 {
            v.push(mk(MkArgs {
                id: "zap_config_perms",
                category: CAT_ZAP,
                category_key: "zapCfg.secCatZap",
                name_key: "zapCfg.secConfigPerms",
                status: "fail",
                detail_key: "zapCfg.secConfigPermsWorldWrite",
                detail_params: json!({ "path": cfg.display().to_string(), "mode": modestr }),
                suggestion_key: Some("zapCfg.secConfigPermsSug"),
                suggestion_params: json!({}),
            }));
        } else if mode & 0o004 != 0 {
            v.push(mk(MkArgs {
                id: "zap_config_perms",
                category: CAT_ZAP,
                category_key: "zapCfg.secCatZap",
                name_key: "zapCfg.secConfigPerms",
                status: "warn",
                detail_key: "zapCfg.secConfigPermsWorldRead",
                detail_params: json!({ "path": cfg.display().to_string(), "mode": modestr }),
                suggestion_key: Some("zapCfg.secConfigPermsSug"),
                suggestion_params: json!({}),
            }));
        } else {
            v.push(mk(MkArgs {
                id: "zap_config_perms",
                category: CAT_ZAP,
                category_key: "zapCfg.secCatZap",
                name_key: "zapCfg.secConfigPerms",
                status: "pass",
                detail_key: "zapCfg.secConfigPermsOk",
                detail_params: json!({ "path": cfg.display().to_string(), "mode": modestr }),
                suggestion_key: None,
                suggestion_params: json!({}),
            }));
        }
    } else {
        v.push(mk(MkArgs {
            id: "zap_config_perms",
            category: CAT_ZAP,
            category_key: "zapCfg.secCatZap",
            name_key: "zapCfg.secConfigPerms",
            status: "info",
            detail_key: "zapCfg.secConfigPermsMissing",
            detail_params: json!({ "path": cfg.display().to_string() }),
            suggestion_key: None,
            suggestion_params: json!({}),
        }));
    }

    // 4) 面板私钥权限：最敏感，应仅属主可读
    let key_file = {
        let g = config::get_config().read().unwrap();
        g.server.key_file.clone()
    };
    if let Ok(meta) = std::fs::metadata(&key_file) {
        let mode = meta.mode();
        let modestr = mode_str(mode);
        if mode & 0o002 != 0 {
            v.push(mk(MkArgs {
                id: "zap_cert_key_perms",
                category: CAT_ZAP,
                category_key: "zapCfg.secCatZap",
                name_key: "zapCfg.secCertKey",
                status: "fail",
                detail_key: "zapCfg.secCertKeyWorldWrite",
                detail_params: json!({ "path": key_file, "mode": modestr }),
                suggestion_key: Some("zapCfg.secCertKeySug"),
                suggestion_params: json!({}),
            }));
        } else if mode & 0o004 != 0 {
            v.push(mk(MkArgs {
                id: "zap_cert_key_perms",
                category: CAT_ZAP,
                category_key: "zapCfg.secCatZap",
                name_key: "zapCfg.secCertKey",
                status: "warn",
                detail_key: "zapCfg.secCertKeyWorldRead",
                detail_params: json!({ "path": key_file, "mode": modestr }),
                suggestion_key: Some("zapCfg.secCertKeySug"),
                suggestion_params: json!({}),
            }));
        } else {
            v.push(mk(MkArgs {
                id: "zap_cert_key_perms",
                category: CAT_ZAP,
                category_key: "zapCfg.secCatZap",
                name_key: "zapCfg.secCertKey",
                status: "pass",
                detail_key: "zapCfg.secCertKeyOk",
                detail_params: json!({ "path": key_file, "mode": modestr }),
                suggestion_key: None,
                suggestion_params: json!({}),
            }));
        }
    } else {
        v.push(mk(MkArgs {
            id: "zap_cert_key_perms",
            category: CAT_ZAP,
            category_key: "zapCfg.secCatZap",
            name_key: "zapCfg.secCertKey",
            status: "info",
            detail_key: "zapCfg.secCertKeyMissing",
            detail_params: json!({ "path": key_file }),
            suggestion_key: None,
            suggestion_params: json!({}),
        }));
    }

    // 5) 全局可写扫描：数据目录 / 应用商店目录 / 用户目录及其关键子目录
    let mut bad = Vec::new();
    let scan: [std::path::PathBuf; 7] = [
        data.clone(),
        appstore::appstore_dir(),
        user_cron::users_dir(),
        appstore::appstore_dir().join("runs"),
        appstore::appstore_dir().join("cache"),
        appstore::appstore_dir().join("logs"),
        cfg.clone(),
    ];
    for p in &scan {
        if let Ok(meta) = std::fs::metadata(p)
            && meta.mode() & 0o002 != 0
        {
            bad.push(p.display().to_string());
        }
    }
    if bad.is_empty() {
        v.push(mk(MkArgs {
            id: "zap_world_writable",
            category: CAT_ZAP,
            category_key: "zapCfg.secCatZap",
            name_key: "zapCfg.secWorldWritable",
            status: "pass",
            detail_key: "zapCfg.secWorldWritableOk",
            detail_params: json!({}),
            suggestion_key: None,
            suggestion_params: json!({}),
        }));
    } else {
        v.push(mk(MkArgs {
            id: "zap_world_writable",
            category: CAT_ZAP,
            category_key: "zapCfg.secCatZap",
            name_key: "zapCfg.secWorldWritable",
            status: "warn",
            detail_key: "zapCfg.secWorldWritableWarn",
            detail_params: json!({ "count": bad.len(), "paths": bad.join("; ") }),
            suggestion_key: Some("zapCfg.secWorldWritableSug"),
            suggestion_params: json!({}),
        }));
    }

    // 6) 特权执行器(zapexec) IPC 套接字是否就绪
    let sock = {
        let g = config::get_config().read().unwrap();
        g.exec.socket_path.clone()
    };
    if Path::new(&sock).exists() {
        v.push(mk(MkArgs {
            id: "zap_zapexec",
            category: CAT_ZAP,
            category_key: "zapCfg.secCatZap",
            name_key: "zapCfg.secZapexec",
            status: "pass",
            detail_key: "zapCfg.secZapexecOk",
            detail_params: json!({ "path": sock }),
            suggestion_key: None,
            suggestion_params: json!({}),
        }));
    } else {
        v.push(mk(MkArgs {
            id: "zap_zapexec",
            category: CAT_ZAP,
            category_key: "zapCfg.secCatZap",
            name_key: "zapCfg.secZapexec",
            status: "fail",
            detail_key: "zapCfg.secZapexecFail",
            detail_params: json!({ "path": sock }),
            suggestion_key: Some("zapCfg.secZapexecSug"),
            suggestion_params: json!({}),
        }));
    }

    v
}

/// 当前进程有效 uid（Linux：读 `/proc/self/status` 的 `Uid:` 字段，避免引入额外依赖）。
fn current_uid() -> u32 {
    if let Ok(s) = std::fs::read_to_string("/proc/self/status")
        && let Some(line) = s.lines().find(|l| l.starts_with("Uid:"))
    {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2
            && let Ok(u) = parts[1].parse::<u32>()
        {
            return u;
        }
    }
    0
}

/// 由 uid 反查用户名（解析 `/etc/passwd`，失败回落 `uid=N`）。
fn username_of(uid: u32) -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() >= 3
                && let Ok(u) = f[2].parse::<u32>()
                && u == uid
            {
                return f[0].to_string();
            }
        }
    }
    format!("uid={uid}")
}

/// 权限位格式化为 `0644` 形式（仅取低 9 位）。
fn mode_str(mode: u32) -> String {
    format!("{:04o}", mode & 0o777)
}
