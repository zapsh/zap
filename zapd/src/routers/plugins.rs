//! 插件 HTTP 接口（/api/plugin/*）。
//!
//! 复用应用商店的鉴权与站点归属校验：列表/运行前确认操作者身份与站点归属，
//! 再把解析后的上下文（家目录 / 站点 root / 站点 Linux 账号）通过白名单动词传给 zapexec。
//!
//! 插件统一装在系统级目录 `$ZAP_PATH/plugins/`，仅管理员可安装 / 卸载，所有用户共享、只能使用。
//! 运行时身份由 manifest 的 `scope` 决定：`site`（站点账号）/ `user`（调用方面板用户账号）/ `system`（root）。

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Extension, Json, Multipart, Query};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::Router;
use futures_util::stream;
use serde::Deserialize;
use serde_json::json;
use sqlx;
use zap_proto::Request;

use crate::db;
use crate::zap::ZapError;
use crate::zap::audit;
use crate::zap::jwt::{decode_verified, is_admin, ValidatedClaims};
use crate::routers::site;
use crate::zap::ZapJsonResult;

/// 上传插件包的大小上限（插件就是几个 Lua / YAML 文件，64 MB 绰绰有余）。
const PLUGIN_UPLOAD_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Deserialize)]
pub struct PluginListQuery {
    #[serde(default)]
    pub slot: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Deserialize)]
pub struct PluginUiQuery {
    pub name: String,
}

#[derive(Deserialize)]
pub struct PluginRunPayload {
    pub name: String,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub site_id: Option<i64>,
    #[serde(default)]
    pub options: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct PluginUninstallPayload {
    pub name: String,
}

/// 读取插件自带的 HTML 界面文件内容（前端塞进沙箱 iframe 渲染）。
pub async fn plugin_ui(
    claims: ValidatedClaims,
    Query(q): Query<PluginUiQuery>,
) -> ZapJsonResult {
    let (home, _linux_user) = load_user_home(claims.id as i64).await?;
    let resp = crate::zapexec::call(Request::PluginUi {
        name: q.name.clone(),
        actor: claims.sub.clone(),
        home,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message, "data": resp.data })))
}

pub async fn plugin_list(
    claims: ValidatedClaims,
    Query(q): Query<PluginListQuery>,
) -> ZapJsonResult {
    let (home, _linux_user) = load_user_home(claims.id as i64).await?;
    let resp = crate::zapexec::call(Request::PluginList {
        actor: claims.sub.clone(),
        home,
        slot: q.slot.clone(),
        scope: q.scope.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message, "data": resp.data })))
}

pub async fn plugin_run(
    claims: ValidatedClaims,
    Json(payload): Json<PluginRunPayload>,
) -> ZapJsonResult {
    let action = payload.action.unwrap_or_else(|| "run".into());
    let (home, linux_user) = load_user_home(claims.id as i64).await?;
    let (site_root, site_linux_user) = match payload.site_id {
        Some(sid) => {
            site::site_in_scope(&claims, sid).await?;
            let (root, owner) = load_site_ctx(sid).await?;
            (Some(root), Some(owner))
        }
        None => (None, None),
    };
    let caller_user = if linux_user.is_empty() { None } else { Some(linux_user) };
    let resp = crate::zapexec::call(Request::PluginRun {
        name: payload.name,
        actor: claims.sub.clone(),
        home,
        user: caller_user,
        site_id: payload.site_id,
        site_root,
        site_linux_user,
        action,
        options: payload.options,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    Ok(Json(json!({ "code": 0, "message": resp.message, "data": resp.data })))
}

// ── 安装 / 卸载 ────────────────────────────────────────────

/// 系统级插件目录落在 zapexec 侧（`$ZAP_PATH/plugins`），这里只负责收上传包。
fn plugin_upload_dir() -> PathBuf {
    let base = std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"));
    base.join("data/plugins/tmp/uploads")
}

/// 系统级插件只有管理员能装 / 卸：它们可以以 root 身份运行。
fn require_admin_for(claims: &ValidatedClaims, level: &str) -> Result<(), ZapError> {
    if level == "system" && !is_admin(claims) {
        return Err(ZapError::New(-1, "仅管理员可安装 / 卸载系统级插件".into()));
    }
    Ok(())
}

/// 上传插件包安装（multipart）：字段 `force` / `name`，文件字段为 zip / tar.gz。
/// 插件统一装到系统目录 `$ZAP_PATH/plugins`，无需指定级别。
///
/// 文件先落到面板数据盘再交给 zapexec 解包，避免把整个包读进内存。
/// 建议前端把文件字段放在最后，这样前面的 `force` 等字段已先被读到。
pub async fn plugin_install_upload(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    mut multipart: Multipart,
) -> ZapJsonResult {
    let mut force = false;
    let mut name = String::new();
    let mut staged: Option<PathBuf> = None;

    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            // 请求体超限或连接中断：把已落盘的临时文件清掉，别留垃圾
            Err(e) => {
                if let Some(p) = &staged {
                    let _ = tokio::fs::remove_file(p).await;
                }
                return Err(ZapError::New(-1, format!("上传中断：{e}")));
            }
        };
        let file_name = field.file_name().unwrap_or("").to_string();
        if file_name.is_empty() {
            match field.name().unwrap_or("") {
                "force" => {
                    let v = field.text().await.unwrap_or_default();
                    force = v == "true" || v == "1" || v == "on";
                }
                "name" => name = field.text().await.unwrap_or_default(),
                _ => {}
            }
            continue;
        }
        // 文件字段：按扩展名落盘（原始文件名不可信，不参与路径拼接）
        let ext = std::path::Path::new(&file_name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("zip")
            .to_ascii_lowercase();
        let dir = plugin_upload_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| ZapError::New(-1, format!("创建上传目录失败: {e}")))?;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = dir.join(format!("{}-{}.{ext}", claims.id, ts));
        let mut out = tokio::fs::File::create(&path)
            .await
            .map_err(|e| ZapError::New(-1, format!("创建临时文件失败: {e}")))?;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| ZapError::New(-1, format!("读取上传内容失败: {e}")))?
        {
            use tokio::io::AsyncWriteExt;
            out.write_all(&chunk)
                .await
                .map_err(|e| ZapError::New(-1, format!("写入临时文件失败: {e}")))?;
        }
        staged = Some(path);
    }

    let Some(path) = staged else {
        return Err(ZapError::New(-1, "没有收到插件包文件".into()));
    };
    require_admin_for(&claims, "system")?;

    let (home, _linux_user) = load_user_home(claims.id as i64).await?;
    let src = path.display().to_string();
    let resp = crate::zapexec::call(Request::PluginInstall {
        name: name.trim().to_string(),
        actor: claims.sub.clone(),
        home,
        source: "archive".to_string(),
        src: src.clone(),
        force,
    })
    .await;
    // 无论成败，临时包都不再需要
    let _ = tokio::fs::remove_file(&path).await;

    let resp = resp?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&*claims),
        Some(client_addr.ip().to_string().as_str()),
        "plugin_install",
        &format!("system:archive"),
        &src,
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": resp.message, "data": resp.data })))
}

/// 卸载插件（删除整个插件目录）。
pub async fn plugin_uninstall(
    claims: ValidatedClaims,
    Extension(client_addr): Extension<SocketAddr>,
    Json(payload): Json<PluginUninstallPayload>,
) -> ZapJsonResult {
    require_admin_for(&claims, "system")?;
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ZapError::New(-1, "缺少插件名".into()));
    }
    let (home, _linux_user) = load_user_home(claims.id as i64).await?;
    let resp = crate::zapexec::call(Request::PluginUninstall {
        name: name.clone(),
        actor: claims.sub.clone(),
        home,
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    audit::log(
        Some(&*claims),
        Some(client_addr.ip().to_string().as_str()),
        "plugin_uninstall",
        &format!("system:{name}"),
        "",
    )
    .await;
    Ok(Json(json!({ "code": 0, "message": resp.message, "data": resp.data })))
}

pub(crate) async fn load_user_home(uid: i64) -> Result<(String, String), ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(String, String)> =
        sqlx::query_as::<_, (String, String)>("SELECT home_dir, linux_user FROM user WHERE id = ?")
            .bind(uid)
            .fetch_optional(pool)
            .await?;
    let (mut home, lu) = row.ok_or_else(|| ZapError::New(-1, "用户不存在".to_string()))?;
    if home.trim().is_empty() {
        // 回退：用 Linux 账号的 OS home（getent passwd 第 6 字段），覆盖 home_dir 未配置的情况
        if let Some(h) = os_home_of(&lu).await {
            home = h;
        }
    }
    if home.trim().is_empty() {
        return Err(ZapError::New(-1, "用户家目录未配置".to_string()));
    }
    Ok((home, lu))
}

/// 取 Linux 账号的真实家目录（getent passwd 第 6 字段），用于 home_dir 为空时的回退。
async fn os_home_of(linux_user: &str) -> Option<String> {
    if linux_user.is_empty() {
        return None;
    }
    let out = tokio::process::Command::new("getent")
        .args(["passwd", linux_user])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let field = line.split(':').nth(5)?.trim().to_string();
    if field.is_empty() { None } else { Some(field) }
}

async fn load_site_ctx(site_id: i64) -> Result<(String, String), ZapError> {
    let pool = db::get_db_pool().await;
    let row: Option<(String, String, String)> = sqlx::query_as::<_, (String, String, String)>(
        "SELECT s.web_root, u.linux_user, u.username \
         FROM site s JOIN user u ON u.id = s.user_id WHERE s.id = ?",
    )
    .bind(site_id)
    .fetch_optional(pool)
    .await?;
    let (web_root, linux_user, username) =
        row.ok_or_else(|| ZapError::New(-1, "站点不存在".to_string()))?;
    let owner = if linux_user.trim().is_empty() {
        username
    } else {
        linux_user
    };
    if owner.trim().is_empty() || owner == "root" {
        return Err(ZapError::New(-1, "站点无可用运行用户".to_string()));
    }
    Ok((web_root, owner))
}

pub fn routers() -> Router {
    Router::new()
        .route("/list", get(plugin_list))
        .route("/ui", get(plugin_ui))
        .route("/run", post(plugin_run))
        .route("/watch", get(plugin_watch))
        .route("/cancel", post(plugin_cancel))
        // 安装 / 卸载：上传包单独放开请求体上限
        .route(
            "/install",
            post(plugin_install_upload).layer(DefaultBodyLimit::max(PLUGIN_UPLOAD_LIMIT)),
        )
        .route("/uninstall", post(plugin_uninstall))
}

/// 取消一个正在运行的异步插件。
///
/// 前端在「运行中」点取消时调用：校验 JWT 后，在 `<ZAP_PATH>/data/plugins/logs/<task_id>.log.cancel`
/// 写一个空哨兵文件；zapexec 的看门狗线程轮询到该文件就会杀掉对应子进程（组），任务随之以
/// 退出码 `-2`（已取消）收尾，SSE 流会把「任务已取消」推给前端。用文件哨兵而非内存信号，
/// 是因为插件实际跑在独立的 zapexec 进程里，文件是跨进程最轻量的通知方式。
#[derive(Deserialize)]
pub struct PluginCancelPayload {
    pub token: String,
    pub task_id: String,
}

pub async fn plugin_cancel(Json(p): Json<PluginCancelPayload>) -> ZapJsonResult {
    let _claims = decode_verified(&p.token).ok_or_else(|| ZapError::New(-1, "未授权".into()))?;
    // 校验 task_id，避免路径穿越（只允许字母数字及 -_.）
    if !p
        .task_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(ZapError::New(-1, "非法 task_id".into()));
    }
    let base = std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"));
    let cancel_path = base
        .join("data/plugins/logs")
        .join(format!("{}.log.cancel", p.task_id));
    std::fs::write(&cancel_path, "")
        .map_err(|e| ZapError::New(-1, format!("写取消哨兵失败: {e}")))?;
    Ok(Json(json!({ "ok": true })))
}

/// 插件异步执行的日志流（SSE）。
///
/// 前端在 `plugin/run` 拿到 `log_path` 后订阅此端点，逐行收到 `{type:"log",line}`，
/// 任务收尾（zapexec 写入 `__ZAP_DONE__ <code>`）时收到 `{type:"done",code}` 后由前端关闭。
/// 因 `EventSource` 不能带自定义请求头，鉴权走 `?token=`（与 WebSocket/下载接口同套 `decode_verified`）。
#[derive(Deserialize)]
pub struct PluginWatchQuery {
    pub token: String,
    pub log_path: String,
}

pub async fn plugin_watch(
    Query(q): Query<PluginWatchQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ZapError> {
    // 鉴权：复用面板既有的 JWT 校验（含「已下线」检查）
    let _claims = decode_verified(&q.token).ok_or_else(|| ZapError::New(-1, "未授权".into()))?;

    // 路径白名单：只允许订阅 <ZAP_PATH>/data/plugins/logs 下的日志，防止越权读取任意文件
    let base = std::env::var("ZAP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/local/zap"));
    let allowed = base.join("data/plugins/logs");
    let path = PathBuf::from(&q.log_path);
    if !path.starts_with(&allowed) {
        return Err(ZapError::New(-1, "非法日志路径".into()));
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);
    tokio::spawn(async move {
        let mut sent_lines = 0usize;
        loop {
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                let lines: Vec<&str> = content.lines().collect();
                // 末尾无换行时最后一行可能还没写完，先不发，等下次轮询
                let complete = if content.ends_with('\n') {
                    lines.len()
                } else {
                    lines.len().saturating_sub(1)
                };
                for i in sent_lines..complete {
                    let line = lines[i];
                    // 完成哨兵：原样透传退出码后结束流
                    if let Some(rest) = line.strip_prefix("__ZAP_DONE__ ") {
                        let code: i64 = rest.trim().parse().unwrap_or(-1);
                        let _ = tx
                            .send(Ok(Event::default().data(
                                serde_json::json!({ "type": "done", "code": code }).to_string(),
                            )))
                            .await;
                        return;
                    }
                    let payload =
                        serde_json::json!({ "type": "log", "line": line }).to_string();
                    let _ = tx.send(Ok(Event::default().data(payload))).await;
                }
                sent_lines = complete;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });

    let s = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(item) => Some((item, rx)),
            None => None,
        }
    });
    Ok(Sse::new(s).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
