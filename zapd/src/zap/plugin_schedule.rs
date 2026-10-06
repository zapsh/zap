//! 插件定时任务与 Webhook 触发。
//!
//! 插件只有在用户点按钮时才会跑，这在很多场景不够用：每天半夜 `git fetch`、仓库备份、
//! CI 推送完自动拉取……面板侧因此补一层**触发器**：
//!
//! - `cron`：五段 cron 表达式（`*/30 * * * *`），由本模块每 30 秒轮询到期项（与
//!   `backup_scheduler` 同一节拍）。**所有权归属创建者**：以创建者的 Linux 账号与
//!   角色运行，管理员看不到别人的任务，避免「借别人的身份定时跑 root 插件」。
//! - `webhook`：给任务生成一个随机 token，外部系统 POST
//!   `/api/plugin/hook/<token>` 即可触发（该路径在权限矩阵里是 Public，靠 token 鉴权）。
//!
//! 存储：`{data}/plugins/schedules.yaml`（面板用户 zapadm 可读写）。
//! 任务本身只是「记一条意志」，真正执行仍走 `zapexec` 的 `plugin.run`，
//! scope / 降权 / dangerous 拦截与手动点按钮完全一致。
//!
//! 为什么不用 `tokio-cron-scheduler`：任务列表会被前端随时改写（增删改 / 开关），
//! 轮询方案改完即时生效，也不会留下需要显式清理的孤儿调度器
//! （同 `auto_update.rs` / `script_cron.rs` 的做法）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use chrono;
use serde::{Deserialize, Serialize};
use zap_proto::Request;

use crate::zap::ZapError;

fn bool_true() -> bool {
    true
}

fn default_trigger() -> String {
    "cron".to_string()
}

/// 一条触发任务。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScheduleItem {
    pub id: String,
    /// 插件名
    pub plugin: String,
    /// 要跑的 action（缺省 `run`）
    pub action: String,
    /// 归属用户的面板账号 —— 定时器 / Webhook 都代表它执行
    pub owner: String,
    pub owner_uid: i64,
    #[serde(default)]
    pub site_id: Option<i64>,
    #[serde(default)]
    pub options: HashMap<String, String>,
    /// `cron` 或 `webhook`
    #[serde(default = "default_trigger")]
    pub trigger: String,
    /// cron 时必填：五段表达式
    #[serde(default)]
    pub cron: String,
    /// webhook 时的随机令牌（同时当作 Webhook URL 的一段）
    #[serde(default)]
    pub token: String,
    #[serde(default = "bool_true")]
    pub enabled: bool,
    #[serde(default)]
    pub last_run: i64,
    /// 上一次执行是否成功（还没跑过为 None）
    #[serde(default)]
    pub last_ok: Option<bool>,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ScheduleFile {
    #[serde(default)]
    items: Vec<ScheduleItem>,
}

fn store_path() -> PathBuf {
    crate::zap::appstore::data_dir()
        .join("plugins")
        .join("schedules.yaml")
}

/// 读任务列表（文件不存在 = 还没有任务）。
pub async fn load() -> Result<Vec<ScheduleItem>, ZapError> {
    let path = store_path();
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ZapError::Error(format!("读取插件任务列表失败: {e}"))),
    };
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let doc: ScheduleFile = serde_yaml::from_str(&text)
        .map_err(|e| ZapError::Error(format!("schedules.yaml 解析失败: {e}")))?;
    Ok(doc.items)
}

/// 原子写回（tmp + rename），与 `mirror.rs` 等既有配置写法一致。
async fn save(items: &[ScheduleItem]) -> Result<(), ZapError> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ZapError::Error(format!("创建任务目录失败: {e}")))?;
    }
    let doc = ScheduleFile {
        items: items.to_vec(),
    };
    let text = serde_yaml::to_string(&doc)
        .map_err(|e| ZapError::Error(format!("序列化插件任务失败: {e}")))?;
    let tmp = path.with_extension("yaml.tmp");
    tokio::fs::write(&tmp, text)
        .await
        .map_err(|e| ZapError::Error(format!("写插件任务失败: {e}")))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| ZapError::Error(format!("插件任务落盘失败: {e}")))?;
    Ok(())
}

fn new_id(plugin: &str) -> String {
    let ts = chrono::Utc::now().timestamp_millis();
    format!("{plugin}-{ts}")
}

/// 生成一条任务的 id / token（token 用 zap-crypto 的随机串生成器）。
fn new_token() -> Result<String, ZapError> {
    zap_crypto::generate_password(24, false)
        .map_err(|e| ZapError::Error(format!("生成 Webhook 令牌失败: {e}")))
}

/// 取当前用户可见的任务（管理员看全部，普通用户只看自己的）。
pub async fn list_for(username: &str, is_admin: bool) -> Result<Vec<ScheduleItem>, ZapError> {
    let items = load().await?;
    Ok(if is_admin {
        items
    } else {
        items.into_iter().filter(|i| i.owner == username).collect()
    })
}

/// 创建（或更新）一条任务：按 id 找不到的就新建。
pub async fn upsert(
    username: &str,
    uid: i64,
    is_admin: bool,
    mut patch: ScheduleItem,
) -> Result<ScheduleItem, ZapError> {
    let mut items = load().await?;
    if let Some(idx) = items.iter().position(|i| i.id == patch.id) {
        let existing = &mut items[idx];
        // 改别人的任务只有管理员能改
        if existing.owner != username && !is_admin {
            return Err(ZapError::Error("只能修改自己的定时任务".into()));
        }
        patch.owner = existing.owner.clone();
        patch.owner_uid = existing.owner_uid;
        patch.created_at = existing.created_at;
        patch.last_run = existing.last_run;
        patch.last_ok = existing.last_ok;
        // 令牌不跟着表单空值走：只有显式要求轮换时才会重新生成，
        // 否则改个 cron 就会把已经配到 CI 里的 Webhook 地址弄失效。
        if patch.trigger == "webhook" && patch.token.is_empty() {
            patch.token = existing.token.clone();
        }
        *existing = patch.clone();
        save(&items).await?;
        return Ok(patch);
    }
    // 新建：token 只在 webhook 触发时才生成
    if patch.trigger == "webhook" && patch.token.is_empty() {
        patch.token = new_token()?;
    }
    if patch.id.is_empty() {
        patch.id = new_id(&patch.plugin);
    }
    if patch.created_at == 0 {
        patch.created_at = chrono::Utc::now().timestamp();
    }
    patch.owner = username.to_string();
    patch.owner_uid = uid;
    if patch.trigger == "cron" && patch.cron.trim().is_empty() {
        return Err(ZapError::Error("定时触发必须给出 cron 表达式".into()));
    }
    if patch.trigger == "cron" {
        // 表达式非法要当场报出来，别等到永远不触发才发现
        crate::zap::script_cron::Cron::parse(&patch.cron)
            .map_err(|e| ZapError::Error(format!("cron 表达式非法: {e}")))?;
    }
    items.push(patch.clone());
    save(&items).await?;
    Ok(patch)
}

/// 轮换 Webhook 令牌：旧地址立即失效（泄露时用）。
pub async fn rotate_token(id: &str, username: &str, is_admin: bool) -> Result<String, ZapError> {
    let mut items = load().await?;
    let Some(it) = items.iter_mut().find(|i| i.id == id) else {
        return Err(ZapError::Error("任务不存在".into()));
    };
    if it.owner != username && !is_admin {
        return Err(ZapError::Error("只能操作自己的定时任务".into()));
    }
    if it.trigger != "webhook" {
        return Err(ZapError::Error("只有 Webhook 触发器才有令牌".into()));
    }
    it.token = new_token()?;
    let out = it.token.clone();
    save(&items).await?;
    Ok(out)
}

/// 删除任务（普通用户只能删自己的）。
pub async fn remove(id: &str, username: &str, is_admin: bool) -> Result<(), ZapError> {
    let mut items = load().await?;
    let before = items.len();
    // 删除条件：id 匹配「且」调用者有权（owner 本人或管理员）。
    // 因此保留下来的是：id 不匹配，或 id 匹配但调用者无权删除的项。
    items.retain(|i| i.id != id || (i.owner != username && !is_admin));
    if items.len() == before {
        return Err(ZapError::Error("任务不存在或无权删除".into()));
    }
    save(&items).await
}

/// 按 token 找 Webhook 任务。
pub async fn find_by_token(token: &str) -> Result<Option<ScheduleItem>, ZapError> {
    Ok(load()
        .await?
        .into_iter()
        .find(|i| i.trigger == "webhook" && !i.token.is_empty() && i.token == token))
}

/// 记录一次执行结果（失败原因也留痕，前端能看到「上次失败」）。
pub async fn record(id: &str, ok: bool) {
    let Ok(mut items) = load().await else { return };
    if let Some(it) = items.iter_mut().find(|i| i.id == id) {
        it.last_run = chrono::Utc::now().timestamp();
        it.last_ok = Some(ok);
    }
    let _ = save(&items).await;
}

/// 执行一条任务：以归属用户的身份（Linux 账号 / 角色）调 zapexec 的 plugin.run。
pub async fn run_item(item: &ScheduleItem) -> Result<(), ZapError> {
    let (home, linux_user) = crate::routers::plugins::load_user_home(item.owner_uid).await?;
    let (site_root, site_linux_user) = match item.site_id {
        Some(sid) => {
            let (root, owner) = load_site_ctx(sid).await?;
            (Some(root), Some(owner))
        }
        None => (None, None),
    };
    let roles = roles_of(item.owner_uid).await?;
    let caller_user = if linux_user.is_empty() {
        None
    } else {
        Some(linux_user)
    };
    let resp = crate::zapexec::call(Request::PluginRun {
        name: item.plugin.clone(),
        actor: item.owner.clone(),
        home,
        user: caller_user,
        site_id: item.site_id,
        site_root,
        site_linux_user,
        action: item.action.clone(),
        // 把面板本地地址注入插件 options：让插件（如 redeploy action）能直接
        // 回拨管理 API（/site/app/git-update 等），不必让用户手填面板地址。
        // 同时把调度的 site_id 也塞进 options —— zapexec 不会把顶层 site_id 透传给
        // Lua，而 redeploy 需要它来定位「重新部署哪个站点的应用」。
        options: {
            let mut o = item.options.clone();
            o.entry("panel_url".to_string()).or_insert_with(|| {
                format!(
                    "http://127.0.0.1:{}",
                    crate::config::get_config().read().unwrap().server.port
                )
            });
            if let Some(sid) = item.site_id {
                o.insert("site_id".to_string(), sid.to_string());
            }
            o
        },
        // 带上归属用户的角色：dangerous 动作对 demo 只读账号依然会被后端拦住
        roles: Some(roles),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::Error(resp.message));
    }
    Ok(())
}

async fn roles_of(uid: i64) -> Result<String, ZapError> {
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String,)> =
        sqlx::query_as("SELECT roles FROM user WHERE id = ?")
            .bind(uid)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0).unwrap_or_default())
}

async fn load_site_ctx(site_id: i64) -> Result<(String, String), ZapError> {
    let pool = crate::db::get_db_pool().await;
    let row: Option<(String, String, String)> = sqlx::query_as::<_, (String, String, String)>(
        "SELECT s.web_root, u.linux_user, u.username \
         FROM site s JOIN user u ON u.id = s.user_id WHERE s.id = ?",
    )
    .bind(site_id)
    .fetch_optional(pool)
    .await?;
    let (web_root, linux_user, username) =
        row.ok_or_else(|| ZapError::Error("站点不存在".into()))?;
    let owner = if linux_user.trim().is_empty() {
        username
    } else {
        linux_user
    };
    if owner.trim().is_empty() || owner == "root" {
        return Err(ZapError::Error("站点无可用运行用户".into()));
    }
    Ok((web_root, owner))
}

/// 启动调度循环（后台任务，不阻塞主流程）。
///
/// 30 秒一跳 + 分钟去重：cron 的最小粒度是分钟，同一分钟里多次唤醒只跑一次。
pub fn start() {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let mut last_min = String::new();
        loop {
            tick(&mut last_min).await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

async fn tick(last_min: &mut String) {
    let Ok(items) = load().await else { return };
    let now = chrono::Local::now();
    let minute_key = now.format("%Y%m%d%H%M").to_string();
    if minute_key == *last_min {
        return;
    }
    for item in items {
        if !item.enabled || item.trigger != "cron" || item.cron.trim().is_empty() {
            continue;
        }
        let hit = crate::zap::script_cron::Cron::parse(&item.cron)
            .map(|c| c.matches(&now))
            .unwrap_or(false);
        if !hit {
            continue;
        }
        match run_item(&item).await {
            Ok(()) => {
                record(&item.id, true).await;
                tracing::info!(
                    "插件定时任务执行成功: {} / {}（归属 {}）",
                    item.plugin,
                    item.action,
                    item.owner
                );
            }
            Err(e) => {
                record(&item.id, false).await;
                tracing::warn!(
                    "插件定时任务执行失败: {} / {}: {e}",
                    item.plugin,
                    item.action
                );
            }
        }
    }
    *last_min = minute_key;
}
