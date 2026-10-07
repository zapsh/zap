// SPDX-License-Identifier: AGPL-3.0-only
//! 「文档」菜单后端。
//!
//! 把 `{data}/www/html/<file>.md` 渲染成 HTML 返回给前端：
//!
//! - 文件来源：开发阶段由 `build.sh` 把根目录的 `CHANGELOG.md` 拷到
//!   `data/www/html/`，后续可在该目录放下其他 `.md`（用户手册 / FAQ / 升级指南）。
//! - `name` 取自白名单 `DOCS` 表（`changelog` / `manual` / `faq` / `upgrade`），
//!   严禁通过 `..` 越权读其他目录。
//!
//! ## 接口
//!
//! - `GET /api/docs/list`  —— 返回所有可用文档的元信息（id + 中英文 title）
//! - `GET /api/docs/:name` —— 读取对应 `.md` 并渲染为 HTML，
//!   `Content-Type: text/html; charset=utf-8`
//!
//! 两个端点均要求登录（`Required::User`，路由表 `access.rs` 登记）。

use std::path::{Path, PathBuf};

use axum::{
    extract::{Path as AxPath, Query},
    http::{StatusCode, header},
    response::Response,
};
use pulldown_cmark::{Options, Parser, html as cmk_html};
use serde::{Deserialize, Serialize};

use crate::{config::get_config, zap::ZapError, zap::jwt::ValidatedClaims};

/// 文档白名单：(id, md 文件名)。
///
/// id 与前端菜单路径一致（如 `changelog` → `/docs/changelog`）；
/// 文件名是 `data/www/html/` 下的相对路径，由 build.sh / 运维提前放好。
const DOCS: &[(&str, &str)] = &[
    ("changelog", "CHANGELOG.md"),
    ("manual", "USER_MANUAL.md"),
    ("faq", "FAQ.md"),
    ("upgrade", "UPGRADE.md"),
];

/// `{data}/www/html/` 的绝对路径。与 `zap::user_cron::data_dir()` 同样算法：
/// 取 `cfg.db.path` 的父目录，再与 CWD 拼接。
fn docs_html_dir() -> PathBuf {
    let cfg = get_config().read().unwrap();
    let db_path = Path::new(&cfg.db.path);
    let dir = match db_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("data"),
    };
    if dir.is_absolute() {
        dir.join("www").join("html")
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&dir).join("www").join("html"))
            .unwrap_or_else(|_| dir.join("www").join("html"))
    }
}

/// 列表项：前端菜单用 id 拼路径，title 双重语用于卡片展示。
///
/// 注意：i18n 由前端 `menu.docs.*` + `docs.*` 命名空间负责，后端只兜底中文标题。
#[derive(Serialize)]
pub struct DocMeta {
    pub id: String,
    /// 兜底标题（前端 i18n 优先；缺失时回落到这里）
    pub title_zh: String,
    pub title_en: String,
}

/// `GET /api/docs/list` —— 列出白名单中的所有文档。
pub async fn docs_list(_claims: ValidatedClaims) -> axum::Json<serde_json::Value> {
    let metas: Vec<DocMeta> = DOCS
        .iter()
        .map(|(id, _)| {
            let (zh, en) = match *id {
                "changelog" => ("更新日志", "Changelog"),
                "manual" => ("用户手册", "User Manual"),
                "faq" => ("常见问题", "FAQ"),
                "upgrade" => ("升级指南", "Upgrade Guide"),
                _ => ("", ""),
            };
            DocMeta {
                id: (*id).to_string(),
                title_zh: zh.to_string(),
                title_en: en.to_string(),
            }
        })
        .collect();
    axum::Json(serde_json::json!({ "code": 0, "message": "OK", "data": metas }))
}

/// `GET /api/docs/:name[?lang=zh-CN]` —— 读取 md 并渲染为 HTML 返回。
///
/// **多语言回退顺序**（受 `?lang` 控制）：
/// 1. `<FILE>_<lang>.md`       例：`USER_MANUAL_zh-CN.md`
/// 2. `<FILE>_<lang_short>.md` 例：`USER_MANUAL_zh.md`（lang 取 `-` 之前的短码）
/// 3. `<FILE>.md`              兜底默认
///
/// 未传 `lang` 或 `lang=` 为空时直接走兜底。
///
/// `lang` 走白名单字符集（`[a-zA-Z0-9-]`，≤16）防路径穿越。
pub async fn docs_get(
    AxPath(name): AxPath<String>,
    Query(q): Query<DocsQuery>,
    _claims: ValidatedClaims,
) -> Result<Response, ZapError> {
    // 双层白名单 + 字符集校验，杜绝路径穿越（`..` / `/` / `\` / 其它字符）。
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(ZapError::New(
            -1,
            "文档名称非法（仅允许小写字母、数字、`-` 与 `_`）".to_string(),
        ));
    }
    let file_name = DOCS
        .iter()
        .find(|(id, _)| *id == name)
        .map(|(_, f)| *f)
        .ok_or_else(|| ZapError::New(-1, "文档不存在".to_string()))?;

    let dir = docs_html_dir();
    let stem = file_name.trim_end_matches(".md");
    // 校验 lang 字符集（`[a-zA-Z0-9-]`，≤16）。非法直接 400，避免路径穿越。
    let lang = match q.lang.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(l) if !is_safe_lang(l) => {
            return Err(ZapError::New(
                -1,
                "lang 非法（仅允许字母、数字与 `-`）".to_string(),
            ));
        }
        Some(l) => Some(l.to_string()),
        None => None,
    };
    let candidates = resolve_candidates(&dir, stem, file_name, lang.as_deref());

    let mut md: Option<String> = None;
    for p in &candidates {
        match tokio::fs::read_to_string(p).await {
            Ok(content) => {
                md = Some(content);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(ZapError::New(
                    -1,
                    format!("读取文档失败（{}）：{}", p.display(), e),
                ));
            }
        }
    }
    let md = md.ok_or_else(|| {
        // 全候选都 NotFound，给出明确排查指引
        let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
        ZapError::New(
            -1,
            format!(
                "文档不存在（尝试过：{}）。请确认 build.sh 已将 {}（含多语言变体 *_zh-CN.md 等）拷贝到 data/www/html/ 下",
                tried.join("、"),
                file_name
            ),
        )
    })?;

    let html_body = render_md(&md);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(axum::body::Body::from(html_body))
        .unwrap())
}

/// `?lang=...` 查询参数：控制多语言 md 回退。`None` 表示直接走兜底。
#[derive(Deserialize, Default)]
pub struct DocsQuery {
    #[serde(default)]
    pub lang: Option<String>,
}

/// 字符集白名单：`[a-zA-Z0-9-]`，长度 1..=16。专门校验 `lang` 与文件名后缀，
/// 防 `..`、`/`、`\`、空字节等注入。
fn is_safe_lang(s: &str) -> bool {
    !s.is_empty() && s.len() <= 16 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// 生成多语言候选路径列表。回退顺序：`<FILE>_<lang>.md` → `<FILE>_<short>.md` → `<FILE>.md`。
///
/// `lang` 必须已经在调用方通过 `is_safe_lang` 校验过；本函数只负责按规则
/// 列出候选路径（不读盘），便于将来加单元测试。
fn resolve_candidates(
    dir: &Path,
    stem: &str,
    default_file: &str,
    lang: Option<&str>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(lang) = lang {
        out.push(dir.join(format!("{}_{}.md", stem, lang)));
        let short = lang.split('-').next().unwrap_or(lang);
        if short != lang {
            out.push(dir.join(format!("{}_{}.md", stem, short)));
        }
    }
    out.push(dir.join(default_file));
    out
}

/// Markdown → HTML 渲染。开启表格 / 删除线 / 任务列表 / 脚注。
///
/// 输出是**片段**而非完整页面：外层样式由前端 `docs/doc.vue` 通过 `:deep()`
/// 给 `article.docs-md` 加作用域样式，确保与全站主题色一致。
fn render_md(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(md, opts);
    let mut out = String::with_capacity(md.len() * 2);
    cmk_html::push_html(&mut out, parser);
    out
}
