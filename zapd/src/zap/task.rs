//! 通用任务队列内核：任务登记 / 状态机 / 并发组 / 日志读取 / 可见性。
//!
//! 面板里所有"跑一会儿"的动作都是任务：应用商店安装与编译、Docker 镜像构建、
//! 家目录备份、系统升级、计划任务执行。它们共用一张表 `task_queue`、同一套状态机，
//! 也共用同一个日志协议（子进程写文件 + `__ZAP_DONE__ <code>` 收尾标记）。
//!
//! 这个模块只管"任务本身"（登记、查、改状态、读日志、并发准入），**不负责**
//! 具体怎么把任务跑起来 —— 那是各业务模块（appstore / docker_build / updater ...）的事：
//! 它们调用 [`enqueue`] 拿一个任务号与准入结论，交给 zapexec 执行，结束后 [`finish`]。
//!
//! 多用户边界：日志里有脚本输出、绝对路径甚至凭据回显，因此任务一律按归属用户隔离
//! （admin 看全部 / reseller 看自己 + 名下客户 / 普通用户只看自己），见 [`ensure_access`]。

use serde::Serialize;
use serde_json::json;
use std::path::Path;

use tracing::{info, warn};

use crate::{
    config, db,
    zap::{ZapError, jwt},
};
use zap_proto::Request;

/// 日志结束标记：`__ZAP_DONE__ <exit_code>`（由 zapexec 写入日志末尾）。
pub const DONE_MARKER: &str = "__ZAP_DONE__";

// ── 任务大类（管理页按它分组；action 是类内具体动作）──────────
pub const KIND_APPSTORE: &str = "appstore";
pub const KIND_DOCKER: &str = "docker";
pub const KIND_BACKUP: &str = "backup";
pub const KIND_SYSTEM: &str = "system";
pub const KIND_CRON: &str = "cron";
pub const KIND_CRONTAB: &str = "crontab";
pub const KIND_SITE: &str = "site";

// ── 状态机 ────────────────────────────────────────────────
/// 排队中：并发组槽位已满，等前一个任务结束。
pub const STATUS_PENDING: &str = "pending";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_SUCCESS: &str = "success";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_CANCELED: &str = "canceled";

/// 控制指令（管理页写入 / 执行侧消费）：请求取消、请求暂停。
pub const CONTROL_CANCEL: &str = "cancel";
pub const CONTROL_PAUSE: &str = "pause";

/// 任务是否已在终态（不再变化）。
pub fn is_final(status: &str) -> bool {
    matches!(status, STATUS_SUCCESS | STATUS_FAILED | STATUS_CANCELED)
}

/// 一次任务记录。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Task {
    pub id: i64,
    pub task_id: String,
    pub kind: String,
    pub action: String,
    /// 任务对象：包名 / 镜像名 / 脚本路径 / 备份目标，由 kind 自行解释。
    pub pkg: String,
    pub username: String,
    pub status: String,
    pub exit_code: i64,
    pub log_path: String,
    /// 归属键：同一次触发（计划任务、某用户的构建历史）的多次运行串在一起。
    pub job_key: String,
    /// 并发互斥组；为空表示不限制并发。
    pub group_key: String,
    /// 组内并行上限；0 表示不限制。
    pub group_limit: i64,
    /// 管理页展示用标题。
    pub title: String,
    /// 进度 0-100；-1 表示不适用（大多数任务是流式输出，没有百分比）。
    pub progress: i64,
    /// 控制指令：'' / 'cancel' / 'pause'。
    pub control: String,
    /// 启动参数：序列化后的执行请求（排队任务靠它在放行时启动）。
    pub payload: String,
    pub started_at: i64,
    pub finished_at: i64,
    pub updated_at: i64,
}

impl Task {
    /// 对外 JSON：老接口沿用 `run_id` 字段名，前端不必跟着改。
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "task_id": self.task_id,
            "run_id": self.task_id,
            "kind": self.kind,
            "action": self.action,
            "pkg": self.pkg,
            "username": self.username,
            "status": self.status,
            "exit_code": self.exit_code,
            "log_path": self.log_path,
            "job_key": self.job_key,
            "group_key": self.group_key,
            "title": self.title,
            "progress": self.progress,
            "control": self.control,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
        })
    }
}

/// 登记一个新任务。
///
/// `task_id` 由调用方生成：日志路径与 zapexec 的请求参数都要先拿到它，
/// 所以不能等落库后再回读。
#[derive(Debug, Clone)]
pub struct NewTask {
    pub task_id: String,
    pub kind: String,
    pub action: String,
    pub pkg: String,
    pub username: String,
    pub title: String,
    pub log_path: String,
    /// 归属键：计划任务 / 某用户的构建历史；手动触发传空串。
    pub job_key: String,
    /// 并发互斥组（如 `appstore:compile`）；空串 = 不限制。
    pub group_key: String,
    /// 组内并行上限（1 = 组内同时只能跑一个）。
    pub group_limit: i64,
    /// 启动参数（序列化后的执行请求 JSON）：排队任务必须有它，否则放行时无法启动。
    pub payload: String,
}

impl NewTask {
    /// 带上启动参数（排队任务需要）。
    pub fn with_payload(mut self, payload: &str) -> Self {
        self.payload = payload.to_string();
        self
    }
}

impl NewTask {
    /// 最常用形态：不限并发的手动任务。
    pub fn simple(task_id: &str, kind: &str, action: &str, pkg: &str, username: &str) -> Self {
        Self {
            task_id: task_id.to_string(),
            kind: kind.to_string(),
            action: action.to_string(),
            pkg: pkg.to_string(),
            username: username.to_string(),
            title: String::new(),
            log_path: String::new(),
            job_key: String::new(),
            group_key: String::new(),
            group_limit: 0,
            payload: String::new(),
        }
    }
}

/// 生成一个任务号：毫秒时间戳 + 随机数（十六进制，不含路径分隔符，可安全拼进文件名）。
pub fn new_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let rand_part: u64 = rand::random();
    format!("{millis:x}{rand_part:x}")
}

/// 默认日志目录：`{data}/task-logs`。
///
/// 历史任务的日志散落在 `data/appstore/logs`、`data/users/<u>/docker-build-logs`，
/// 落库时都以 `log_path` 记了绝对路径，读日志一律以 DB 为准，这里只用于新任务。
pub fn logs_dir() -> std::path::PathBuf {
    let cfg = config::get_config().read().unwrap();
    Path::new(&cfg.db.path)
        .parent()
        .map(|p| p.join("task-logs"))
        .unwrap_or_else(|| std::path::PathBuf::from("data/task-logs"))
}

/// 指定目录下某个任务的日志文件路径。
pub fn log_path_in(dir: &Path, task_id: &str) -> String {
    dir.join(format!("run-{task_id}.log"))
        .to_string_lossy()
        .into_owned()
}

// ── 登记 / 状态流转 ───────────────────────────────────────

/// 登记任务并写入 `task_queue`。
///
/// 返回值里的 `status` 是**准入结论**：
/// - `running`：拿到槽位了，调用方可以立即启动；
/// - `pending`：并发组已满（如应用商店同一时刻只允许一个编译任务），
///   调用方**不要**启动它，等 [`next_pending`] 轮到它时再启动。
pub async fn enqueue(t: NewTask) -> Result<Task, ZapError> {
    let now = chrono::Utc::now().timestamp();
    let status = if admits_now(&t.group_key, t.group_limit).await {
        STATUS_RUNNING
    } else {
        STATUS_PENDING
    };
    let pool = db::get_db_pool().await;
    sqlx::query(
        "INSERT INTO task_queue \
         (task_id, kind, action, pkg, username, status, exit_code, log_path, job_key, \
          group_key, group_limit, title, progress, control, payload, started_at, finished_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, -1, ?, ?, ?, ?, ?, -1, '', ?, ?, 0, ?)",
    )
    .bind(&t.task_id)
    .bind(&t.kind)
    .bind(&t.action)
    .bind(&t.pkg)
    .bind(&t.username)
    .bind(status)
    .bind(&t.log_path)
    .bind(&t.job_key)
    .bind(&t.group_key)
    .bind(t.group_limit)
    .bind(&t.title)
    .bind(&t.payload)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    // 并发窗口补救：两个请求可能同时看到空槽位并都登记成 running。
    // 登记后按"实际有几个在跑"复核一次，超了就把自己退回排队。
    if status == STATUS_RUNNING && t.group_limit > 0 && !t.group_key.is_empty() {
        let running: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM task_queue WHERE group_key = ? AND status = 'running'",
        )
        .bind(&t.group_key)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        if running > t.group_limit {
            let _ =
                sqlx::query("UPDATE task_queue SET status = ?, updated_at = ? WHERE task_id = ?")
                    .bind(STATUS_PENDING)
                    .bind(now)
                    .bind(&t.task_id)
                    .execute(pool)
                    .await;
        }
    }
    get(&t.task_id)
        .await?
        .ok_or_else(|| ZapError::New(-1, "任务登记失败".to_string()))
}

/// 并发组当前是否还有槽位。
///
/// `group_key` 为空或 `group_limit <= 0` 表示不限制。计数含 `pending`：
/// 排队中的任务也算占坑，否则同时提交 N 个任务会一起挤进运行。
async fn admits_now(group_key: &str, group_limit: i64) -> bool {
    if group_key.is_empty() || group_limit <= 0 {
        return true;
    }
    let pool = db::get_db_pool().await;
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM task_queue \
         WHERE group_key = ? AND status IN ('pending','running')",
    )
    .bind(group_key)
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    active < group_limit
}

/// 并发组内在跑 / 排队的任务数（管理页展示"还有几个在排队"）。
pub async fn active_in_group(group_key: &str) -> i64 {
    if group_key.is_empty() {
        return 0;
    }
    let pool = db::get_db_pool().await;
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM task_queue WHERE group_key = ? AND status IN ('pending','running')",
    )
    .bind(group_key)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// 并发组内最早的一条 `pending` 任务（供调度器放行）。
///
/// 只挑选、不改状态：真正的 pending→running 由 [`start`] 在**启动成功后**改写，
/// 避免"启动失败却已占着 running"的脏状态。
pub async fn next_pending(group_key: &str) -> Option<Task> {
    if group_key.is_empty() {
        return None;
    }
    let pool = db::get_db_pool().await;
    sqlx::query_as::<_, Task>(
        "SELECT * FROM task_queue WHERE group_key = ? AND status = 'pending' \
         ORDER BY id ASC LIMIT 1",
    )
    .bind(group_key)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 把排队任务置为运行中（启动成功后调用）。已被别人抢先启动则返回 false。
pub async fn start(task_id: &str) -> Result<bool, sqlx::Error> {
    let pool = db::get_db_pool().await;
    let res = sqlx::query(
        "UPDATE task_queue SET status = ?, started_at = ?, updated_at = ? \
         WHERE task_id = ? AND status = ?",
    )
    .bind(STATUS_RUNNING)
    .bind(chrono::Utc::now().timestamp())
    .bind(chrono::Utc::now().timestamp())
    .bind(task_id)
    .bind(STATUS_PENDING)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 任务收尾：落定状态与退出码。
///
/// 返回同组内下一个待放行的任务（若有）；真正把它跑起来由 [`spawn_scheduler`]
/// 负责 —— 收尾只管记账，不做调度。
pub async fn finish(task_id: &str, status: &str, exit_code: i64) -> Option<Task> {
    let now = chrono::Utc::now().timestamp();
    let pool = db::get_db_pool().await;
    let group: Option<String> =
        sqlx::query_scalar("SELECT group_key FROM task_queue WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let updated = sqlx::query(
        "UPDATE task_queue SET status = ?, exit_code = ?, finished_at = ?, updated_at = ? \
         WHERE task_id = ?",
    )
    .bind(status)
    .bind(exit_code)
    .bind(now)
    .bind(now)
    .bind(task_id)
    .execute(pool)
    .await;
    if let Err(e) = updated {
        warn!("任务收尾失败 {task_id}: {e}");
    }

    match group {
        Some(g) if !g.is_empty() => next_pending(&g).await,
        _ => None,
    }
}

// ── 排队调度 ────────────────────────────────────────────────

/// 调度轮询间隔：任务结束到下一个排队任务启动之间最坏延迟这么久。
const SCHED_INTERVAL_SECS: u64 = 2;

/// 启动后台调度器（main 里调一次即可）：并发组空出槽位就放行最早排队的任务。
///
/// 做成"轮询数据库"而不是"谁结束谁拉下一个"，是因为后者在每个退出路径上都要
/// 记得放行（正常结束、超时、启动失败、被取消、进程重启……），漏一处排队任务
/// 就永远卡住；轮询以数据库为唯一事实来源，槽位空了就一定会被填上。
pub fn spawn_scheduler() {
    tokio::spawn(async move {
        info!("任务队列调度器已启动");
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(SCHED_INTERVAL_SECS));
        loop {
            timer.tick().await;
            if let Err(e) = schedule_once().await {
                warn!("任务队列调度失败: {e}");
            }
        }
    });
}

/// 一轮调度：每个并发组至多放行一个（剩下的下一轮再看还有没有槽位）。
async fn schedule_once() -> Result<(), sqlx::Error> {
    let pool = db::get_db_pool().await;
    // 只看"没人请求取消"的排队任务：已请求取消的会在原地被置为 canceled
    let groups: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT group_key FROM task_queue \
         WHERE status = 'pending' AND group_key <> '' AND control = ''",
    )
    .fetch_all(pool)
    .await?;

    for g in groups {
        // 组上限在组内各条记录上一致，取最大即可容忍个别脏数据
        let limit: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(group_limit), 0) FROM task_queue WHERE group_key = ?",
        )
        .bind(&g)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        if limit <= 0 || active_in_group(&g).await >= limit {
            continue;
        }
        if let Some(next) = next_pending(&g).await {
            dispatch(&next).await;
        }
    }
    Ok(())
}

/// 放行一条排队任务：先抢状态（防并发重入），再把请求下发给执行侧。
async fn dispatch(t: &Task) {
    match start(&t.task_id).await {
        Ok(true) => {}
        // 已被别的轮次放行：本轮放弃，下一轮再评估
        Ok(false) => return,
        Err(e) => {
            warn!("放行任务 {} 失败: {e}", t.task_id);
            return;
        }
    }
    info!(
        "排队任务 {} 已放行启动（{} {}）",
        t.task_id, t.action, t.pkg
    );
    if let Err(e) = launch(t).await {
        warn!("启动排队任务 {} 失败: {e}", t.task_id);
    }
}

/// 把任务交给执行侧：下发请求 → 成功则盯日志收尾，失败立刻落 failed。
///
/// 已 running 的任务也能用它（安装接口"拿到槽位就立刻启动"走的就是这里）。
pub async fn launch(t: &Task) -> Result<(), ZapError> {
    if t.payload.is_empty() {
        let _ = finish(&t.task_id, STATUS_FAILED, -1).await;
        return Err(ZapError::New(-1, "任务缺少启动参数，无法执行".to_string()));
    }
    let req: Request = serde_json::from_str(&t.payload)
        .map_err(|e| ZapError::New(-1, format!("任务启动参数无法解析: {e}")))?;
    match crate::zapexec::call(req).await {
        Ok(resp) if resp.code != 0 => {
            let _ = finish(&t.task_id, STATUS_FAILED, resp.code).await;
            return Err(ZapError::New(resp.code, resp.message));
        }
        Ok(_) => {}
        Err(e) => {
            let _ = finish(&t.task_id, STATUS_FAILED, -1).await;
            return Err(e);
        }
    }
    watch_log(
        t.task_id.clone(),
        t.log_path.clone(),
        watch_timeout(&t.kind),
    );
    Ok(())
}

/// 各类任务的日志盯守超时：构建这类长任务给 6 小时，其余 24 小时。
fn watch_timeout(kind: &str) -> u64 {
    match kind {
        KIND_DOCKER => 6 * 3600,
        _ => 24 * 3600,
    }
}

/// 排队位次（1 = 下一个就轮到）；不在排队中返回 0。
pub async fn queue_position(t: &Task) -> i64 {
    if t.status != STATUS_PENDING || t.group_key.is_empty() {
        return 0;
    }
    let pool = db::get_db_pool().await;
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM task_queue WHERE group_key = ? AND status = 'pending' AND id <= ?",
    )
    .bind(&t.group_key)
    .bind(t.id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// 删除任务记录（并发组超发时撤销 / 管理页清理）。
pub async fn delete(task_id: &str) -> Result<bool, sqlx::Error> {
    let pool = db::get_db_pool().await;
    let res = sqlx::query("DELETE FROM task_queue WHERE task_id = ?")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// 请求取消 / 暂停：只写控制位，不直接杀进程。
///
/// 真正的停止要由执行侧（zapexec）对进程组发信号，zapd 不持有子进程句柄；
/// 这里先落库，执行侧轮询到指令后再动作，管理页也能立刻看到"已请求取消"。
pub async fn set_control(task_id: &str, control: &str) -> Result<bool, sqlx::Error> {
    let pool = db::get_db_pool().await;
    let res = sqlx::query(
        "UPDATE task_queue SET control = ?, updated_at = ? \
         WHERE task_id = ? AND status IN ('pending','running')",
    )
    .bind(control)
    .bind(chrono::Utc::now().timestamp())
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 更新进度（0-100；-1 表示不适用）。仅供有百分比语义的任务调用。
pub async fn set_progress(task_id: &str, progress: i64) -> Result<(), sqlx::Error> {
    let pool = db::get_db_pool().await;
    sqlx::query("UPDATE task_queue SET progress = ?, updated_at = ? WHERE task_id = ?")
        .bind(progress)
        .bind(chrono::Utc::now().timestamp())
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── 查询 ──────────────────────────────────────────────────

pub async fn get(task_id: &str) -> Result<Option<Task>, sqlx::Error> {
    let pool = db::get_db_pool().await;
    sqlx::query_as::<_, Task>("SELECT * FROM task_queue WHERE task_id = ?")
        .bind(task_id)
        .fetch_optional(pool)
        .await
}

/// 列表筛选条件；字段为 None 表示不筛选。
#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub kind: Option<String>,
    pub action: Option<String>,
    /// 状态筛选；支持逗号分隔的多值（如 `pending,running` 取"未结束的"）。
    pub status: Option<String>,
    /// 归属用户精确匹配（普通用户列表由 [`list_for`] 强制收敛为自己的）。
    pub username: Option<String>,
    /// 并发组（如只看应用商店编译队列）。
    pub group_key: Option<String>,
    /// 归属键（如某个计划任务的历次运行）。
    pub job_key: Option<String>,
    /// 标题 / 对象 / 任务号模糊匹配。
    pub keyword: Option<String>,
}

impl Filter {
    /// 拼出 WHERE 子句与绑定值；`scope` 是调用方给的前置条件（可见范围）。
    fn where_clause(&self, scope: Option<&str>) -> (String, Vec<String>) {
        let mut conds: Vec<String> = Vec::new();
        let mut binds: Vec<String> = Vec::new();
        if let Some(s) = scope {
            conds.push(s.to_string());
        }
        if let Some(v) = &self.kind {
            conds.push("kind = ?".to_string());
            binds.push(v.clone());
        }
        if let Some(v) = &self.action {
            conds.push("action = ?".to_string());
            binds.push(v.clone());
        }
        if let Some(v) = &self.status {
            let list: Vec<&str> = v
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            match list.as_slice() {
                [] => {}
                [one] => {
                    conds.push("status = ?".to_string());
                    binds.push((*one).to_string());
                }
                many => {
                    let ph = many.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
                    conds.push(format!("status IN ({ph})"));
                    for s in many {
                        binds.push((*s).to_string());
                    }
                }
            }
        }
        if let Some(v) = &self.username {
            conds.push("username = ?".to_string());
            binds.push(v.clone());
        }
        if let Some(v) = &self.group_key {
            conds.push("group_key = ?".to_string());
            binds.push(v.clone());
        }
        if let Some(v) = &self.job_key {
            conds.push("job_key = ?".to_string());
            binds.push(v.clone());
        }
        if let Some(v) = &self.keyword
            && !v.trim().is_empty()
        {
            conds.push("(title LIKE ? OR pkg LIKE ? OR task_id LIKE ?)".to_string());
            let like = format!("%{}%", v.trim());
            binds.push(like.clone());
            binds.push(like.clone());
            binds.push(like);
        }
        if conds.is_empty() {
            (String::new(), binds)
        } else {
            (format!(" WHERE {}", conds.join(" AND ")), binds)
        }
    }
}

/// 分页列出任务（管理员视角，不带可见范围收敛）。
pub async fn list(
    filter: Filter,
    page: i64,
    page_size: i64,
) -> Result<(Vec<Task>, i64), sqlx::Error> {
    query_page(filter, None, page, page_size).await
}

/// 各状态的任务数（管理页 / 面板角标用），同样按可见范围收敛。
pub async fn status_counts(claims: &jwt::Claims) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let pool = db::get_db_pool().await;
    if jwt::is_admin(claims) {
        return sqlx::query_as::<_, (String, i64)>(
            "SELECT status, COUNT(*) FROM task_queue GROUP BY status",
        )
        .fetch_all(pool)
        .await;
    }
    if jwt::is_reseller(claims) {
        return sqlx::query_as::<_, (String, i64)>(
            "SELECT status, COUNT(*) FROM task_queue \
             WHERE username = ? OR username IN (SELECT username FROM user WHERE owner_id = ?) \
             GROUP BY status",
        )
        .bind(&claims.sub)
        .bind(claims.id as i64)
        .fetch_all(pool)
        .await;
    }
    sqlx::query_as::<_, (String, i64)>(
        "SELECT status, COUNT(*) FROM task_queue WHERE username = ? GROUP BY status",
    )
    .bind(&claims.sub)
    .fetch_all(pool)
    .await
}

/// reseller 的可见范围：自己 + 名下客户（对应 WHERE 前缀的两个绑定参数）。
struct ResellerScope {
    username: String,
    owner_id: i64,
}

const RESELLER_SCOPE: &str =
    "(username = ? OR username IN (SELECT username FROM user WHERE owner_id = ?))";

/// 按可见范围分页列出任务：admin 全部；reseller 自己 + 名下客户；普通用户仅自己。
pub async fn list_for(
    claims: &jwt::Claims,
    filter: Filter,
    page: i64,
    page_size: i64,
) -> Result<(Vec<Task>, i64), sqlx::Error> {
    if jwt::is_admin(claims) {
        return query_page(filter, None, page, page_size).await;
    }
    if jwt::is_reseller(claims) {
        return query_page(
            filter,
            Some(ResellerScope {
                username: claims.sub.clone(),
                owner_id: claims.id as i64,
            }),
            page,
            page_size,
        )
        .await;
    }
    // 普通用户：无论传入什么 username，一律收敛为自己
    let mut filter = filter;
    filter.username = Some(claims.sub.clone());
    query_page(filter, None, page, page_size).await
}

async fn query_page(
    filter: Filter,
    scope: Option<ResellerScope>,
    page: i64,
    page_size: i64,
) -> Result<(Vec<Task>, i64), sqlx::Error> {
    let pool = db::get_db_pool().await;
    let page = page.max(1);
    let page_size = page_size.clamp(1, 200);
    let offset = (page - 1) * page_size;
    let (clause, binds) = filter.where_clause(scope.as_ref().map(|_| RESELLER_SCOPE));

    let count_sql = format!("SELECT COUNT(*) FROM task_queue{clause}");
    let mut count = sqlx::query_scalar::<_, i64>(&count_sql);
    // 可见范围参数在 WHERE 最前面，必须先于筛选参数绑定
    if let Some(s) = &scope {
        count = count.bind(&s.username).bind(s.owner_id);
    }
    for b in &binds {
        count = count.bind(b);
    }
    let total = count.fetch_one(pool).await.unwrap_or(0);

    let list_sql = format!("SELECT * FROM task_queue{clause} ORDER BY id DESC LIMIT ? OFFSET ?");
    let mut q = sqlx::query_as::<_, Task>(&list_sql);
    if let Some(s) = &scope {
        q = q.bind(&s.username).bind(s.owner_id);
    }
    for b in &binds {
        q = q.bind(b);
    }
    let rows = q.bind(page_size).bind(offset).fetch_all(pool).await?;
    Ok((rows, total))
}

// ── 可见性 ────────────────────────────────────────────────

/// `username` 是否落在 claims 的可见范围内（与站点 / 证书归属同一套规则）。
pub async fn user_visible(claims: &jwt::Claims, username: &str) -> Result<bool, ZapError> {
    if jwt::is_admin(claims) {
        return Ok(true);
    }
    if username == claims.sub {
        return Ok(true);
    }
    if jwt::is_reseller(claims) {
        let pool = db::get_db_pool().await;
        let (cnt,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM user WHERE username = ? AND owner_id = ?")
                .bind(username)
                .bind(claims.id as i64)
                .fetch_one(pool)
                .await?;
        return Ok(cnt > 0);
    }
    Ok(false)
}

/// 取任务并校验归属：越权一律拒绝，且不区分"不存在"与"无权访问"以外的细节。
pub async fn ensure_access(claims: &jwt::Claims, task_id: &str) -> Result<Task, ZapError> {
    let task = get(task_id)
        .await?
        .ok_or_else(|| ZapError::New(-1, "任务不存在".to_string()))?;
    if !user_visible(claims, &task.username).await? {
        return Err(ZapError::New(
            -1,
            "无权访问该任务：任务日志按归属用户隔离".to_string(),
        ));
    }
    Ok(task)
}

// ── 日志 ──────────────────────────────────────────────────

/// 后台盯日志：出现 `__ZAP_DONE__ <code>` 就落定状态；超时判失败。
///
/// `/task/ws/{id}` 之类的流式接口在断开时也会收尾，但用户"点了就关页面"的场景
/// 需要这里兜底，否则记录一直停在 running。`timeout_secs` 由业务给
/// （构建这类长任务给几小时，脚本任务 24 小时）。
pub fn watch_log(task_id: String, log_path: String, timeout_secs: u64) {
    tokio::spawn(async move {
        let interval = std::time::Duration::from_millis(500);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        loop {
            if let Some(code) = read_done_marker(&log_path).await {
                let status = if code == 0 {
                    STATUS_SUCCESS
                } else {
                    STATUS_FAILED
                };
                let _ = finish(&task_id, status, code).await;
                break;
            }
            if std::time::Instant::now() > deadline {
                let _ = finish(&task_id, STATUS_FAILED, -2).await;
                break;
            }
            tokio::time::sleep(interval).await;
        }
    });
}

/// 从日志末尾探测完成标记，返回退出码。
pub async fn read_done_marker(log_path: &str) -> Option<i64> {
    let content = tokio::fs::read_to_string(log_path).await.ok()?;
    let tail = content.rsplit(DONE_MARKER).next()?.trim();
    let code: i64 = tail.split_whitespace().next()?.parse().ok()?;
    Some(code)
}

/// 读取日志 offset 之后的内容，同时返回是否已完成。
pub async fn read_log(
    log_path: &str,
    offset: u64,
) -> Result<(String, Option<i64>, bool), ZapError> {
    let content = match tokio::fs::read_to_string(log_path).await {
        Ok(c) => c,
        // 日志文件尚未生成（后台任务刚启动的竞态窗口），视为空日志，
        // 由调用方（WebSocket / HTTP 轮询）继续等待而非直接失败。
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let bytes = content.as_bytes();
    let start = (offset as usize).min(bytes.len());
    let text = String::from_utf8_lossy(&bytes[start..]).to_string();
    let done = read_done_marker(log_path).await;
    Ok((text, done, done.is_some()))
}

/// 去掉日志尾部完成标记，供最终展示。
pub fn strip_done_marker(content: &str) -> String {
    match content.rfind(DONE_MARKER) {
        Some(idx) => content[..idx].trim_end().to_string(),
        None => content.to_string(),
    }
}

// ── 清理 ──────────────────────────────────────────────────

/// 删除某次任务留下的产物：日志文件（路径以 DB 的 `log_path` 为准）。
///
/// 任务快照目录（如应用商店的 `runs/<task_id>/`）由各业务自行清理：
/// 内核不知道快照放在哪，也不该替业务决定。
pub fn remove_log(task: &Task) {
    if !task.log_path.is_empty()
        && let Err(e) = std::fs::remove_file(&task.log_path)
    {
        warn!("删除任务日志失败 {}: {e}", task.log_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_statuses() {
        assert!(is_final(STATUS_SUCCESS));
        assert!(is_final(STATUS_FAILED));
        assert!(is_final(STATUS_CANCELED));
        assert!(!is_final(STATUS_RUNNING));
        assert!(!is_final(STATUS_PENDING));
    }

    #[test]
    fn done_marker_is_stripped() {
        assert_eq!(strip_done_marker("hello\n__ZAP_DONE__ 0\n"), "hello");
        assert_eq!(strip_done_marker("hello"), "hello");
    }

    #[test]
    fn ids_are_path_safe() {
        // 任务号会拼进文件名，不能出现分隔符
        for _ in 0..32 {
            let id = new_id();
            assert!(!id.contains('/') && !id.is_empty());
        }
    }
}
