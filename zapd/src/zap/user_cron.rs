//! 面板用户的计划任务（crontab）。
//!
//! 与 `script_cron`（admin 专属、存 DB、以 root 运行）不同，这里的任务：
//!
//! - **每用户一份**：`{data}/users/<username>/crontab.yaml`（YAML 为唯一事实来源）
//! - **每角色可用**：路由挂 `/terminal/crontab/*`（`Required::User`）
//! - **执行身份**：admin 可为任意面板账号；其他角色强制为其自身 `linux_user`
//! - **调度**：面板内置调度器（复用 `script_cron::Cron` 解析），不依赖 crond
//! - **日志**：`{data}/users/<username>/cron-logs/run-<run_id>.log`
//!
//! 真正的进程执行由 zapexec 的 `cron.run` 动词完成（切 uid + 写日志 + 完成标记）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::time::Duration;
use tracing::{info, warn};

use crate::zap::ZapError;
use crate::zap::appstore as ast;
use crate::zap::jwt::{ValidatedClaims, is_admin};
use crate::zap::script_cron::Cron;
use crate::zapexec;
use zap_proto::Request;

/// 单用户任务数量上限。
pub const MAX_JOBS: usize = 100;
/// 立即运行 / 调度触发的最小间隔（秒），用于同一分钟内防重。
const DEDUP_WINDOW: i64 = 50;

// ── 数据结构 ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// 5 段标准 cron：`分 时 日 月 周`
    pub schedule: String,
    /// `script`（command 为脚本绝对路径）| `command`（command 为 sh 命令体）
    #[serde(default = "default_kind")]
    pub kind: String,
    pub command: String,
    /// 实际执行的 Linux 账号
    #[serde(default)]
    pub exec_user: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub remark: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub last_run_at: i64,
    #[serde(default)]
    pub last_run_id: String,
    /// `running` | `success` | `failed` | ``
    #[serde(default)]
    pub last_status: String,
    #[serde(default)]
    pub next_run_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Crontab {
    #[serde(default = "default_version")]
    pub version: i64,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub jobs: Vec<CronJob>,
}

impl Default for Crontab {
    fn default() -> Self {
        Self {
            version: 1,
            owner: String::new(),
            updated_at: 0,
            jobs: Vec::new(),
        }
    }
}

fn default_kind() -> String {
    "command".to_string()
}

fn default_true() -> bool {
    true
}

fn default_version() -> i64 {
    1
}

// ── 路径 ────────────────────────────────────────────────────

/// `{data}` 目录（与 zap.db 同级）。
///
/// 配置中的 `db.path` 通常是相对路径（如 `data/zap.db`，相对 `zapd` 的
/// WorkingDirectory）。这里统一转成**绝对路径**：zapexec 侧会对日志路径做
/// 「必须位于 `{ZAP_PATH}/data/users/` 之下」的越权校验，相对路径无法比对。
pub(crate) fn data_dir() -> PathBuf {
    let cfg = crate::config::get_config().read().unwrap();
    let db_path = Path::new(&cfg.db.path);
    let dir = match db_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("data"),
    };
    if dir.is_absolute() {
        dir
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&dir))
            .unwrap_or(dir)
    }
}

pub fn users_dir() -> PathBuf {
    data_dir().join("users")
}

/// 面板用户名安全校验：仅允许 `[A-Za-z0-9._-]`，禁止 `..` 与首字符为 `.`。
///
/// 凡是要拼进 `data/users/<user>/` 路径的地方都先过这道校验（crontab、云存储…），
/// 避免用户名里的 `/` 或 `..` 把落盘路径带出用户目录。
pub(crate) fn safe_username(username: &str) -> Result<(), ZapError> {
    let u = username.trim();
    if u.is_empty() || u.len() > 64 {
        return Err(ZapError::New(-1, "用户名不合法".to_string()));
    }
    if u.starts_with('.') || u.contains("..") || u.contains('/') || u.contains('\\') {
        return Err(ZapError::New(-1, "用户名不合法".to_string()));
    }
    if !u
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
    {
        return Err(ZapError::New(-1, "用户名不合法".to_string()));
    }
    Ok(())
}

fn crontab_path(username: &str) -> PathBuf {
    users_dir().join(username).join("crontab.yaml")
}

pub fn logs_dir(username: &str) -> PathBuf {
    users_dir().join(username).join("cron-logs")
}

pub fn log_path(username: &str, run_id: &str) -> String {
    logs_dir(username)
        .join(format!("run-{run_id}.log"))
        .to_string_lossy()
        .into_owned()
}

/// 运行记录在 `task_queue` 表中的任务归属键。
///
/// 带上用户名是因为 crontab 按用户隔离（`crontab.yaml` 各存一份），
/// 不同用户的任务 id 不保证全局唯一。
pub fn job_key(username: &str, job_id: &str) -> String {
    format!("crontab:{username}:{job_id}")
}

// ── 读写 ────────────────────────────────────────────────────

pub fn load_sync(username: &str) -> Result<Crontab, ZapError> {
    safe_username(username)?;
    let path = crontab_path(username);
    if !path.exists() {
        return Ok(Crontab {
            owner: username.to_string(),
            ..Default::default()
        });
    }
    let text = std::fs::read_to_string(&path)?;
    if text.trim().is_empty() {
        return Ok(Crontab {
            owner: username.to_string(),
            ..Default::default()
        });
    }
    let mut ct: Crontab = serde_yaml::from_str(&text)
        .map_err(|e| ZapError::New(-1, format!("crontab.yaml 解析失败: {e}")))?;
    ct.owner = username.to_string();
    Ok(ct)
}

pub async fn load(username: &str) -> Result<Crontab, ZapError> {
    let username = username.to_string();
    tokio::task::spawn_blocking(move || load_sync(&username))
        .await
        .unwrap_or_else(|e| Err(ZapError::New(-1, format!("读取失败: {e}"))))
}

/// 写入 crontab.yaml（临时文件 + rename 原子替换）。
pub fn save_sync(username: &str, ct: &Crontab) -> Result<(), ZapError> {
    safe_username(username)?;
    let path = crontab_path(username);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = ct.clone();
    out.version = 1;
    out.owner = username.to_string();
    out.updated_at = chrono::Local::now().timestamp();
    let text = serde_yaml::to_string(&out)
        .map_err(|e| ZapError::New(-1, format!("crontab.yaml 序列化失败: {e}")))?;
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub async fn save(username: &str, ct: &Crontab) -> Result<(), ZapError> {
    let username = username.to_string();
    let ct = ct.clone();
    tokio::task::spawn_blocking(move || save_sync(&username, &ct))
        .await
        .unwrap_or_else(|e| Err(ZapError::New(-1, format!("保存失败: {e}"))))
}

// ── 校验与身份 ──────────────────────────────────────────────

/// 解析「执行用户」：
/// - admin：允许指定（须为面板中已存在的 `linux_user`）
/// - 其他角色：忽略入参，强制为其自身 `linux_user`
pub async fn resolve_exec_user(
    claims: &ValidatedClaims,
    requested: &str,
) -> Result<String, ZapError> {
    let pool = crate::db::get_db_pool().await;
    if is_admin(claims) {
        let want = requested.trim();
        if want.is_empty() {
            return Err(ZapError::New(-1, "请选择执行用户".to_string()));
        }
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT linux_user FROM user WHERE linux_user = ? AND linux_user <> '' LIMIT 1",
        )
        .bind(want)
        .fetch_optional(pool)
        .await?;
        exists.ok_or_else(|| ZapError::New(-1, format!("执行用户不存在: {want}")))
    } else {
        let own: Option<String> = sqlx::query_scalar(
            "SELECT linux_user FROM user WHERE id = ? AND linux_user <> '' LIMIT 1",
        )
        .bind(claims.id as i64)
        .fetch_optional(pool)
        .await?;
        own.ok_or_else(|| ZapError::New(-1, "当前账号未分配运行用户，请联系管理员".to_string()))
    }
}

/// 可供 admin 选择的执行用户列表（面板内已分配的 Linux 账号）。
pub async fn list_exec_users() -> Result<Vec<String>, ZapError> {
    let pool = crate::db::get_db_pool().await;
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT linux_user FROM user WHERE linux_user <> '' ORDER BY linux_user",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn home_dir_of(linux_user: &str) -> String {
    let pool = crate::db::get_db_pool().await;
    sqlx::query_scalar::<_, String>(
        "SELECT home_dir FROM user WHERE linux_user = ? AND home_dir <> '' LIMIT 1",
    )
    .bind(linux_user)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// 校验任务字段（不含身份）。
pub fn validate_job(name: &str, schedule: &str, kind: &str, command: &str) -> Result<(), ZapError> {
    if name.trim().is_empty() {
        return Err(ZapError::New(-1, "任务名称不能为空".to_string()));
    }
    if name.chars().count() > 64 {
        return Err(ZapError::New(
            -1,
            "任务名称过长（上限 64 字符）".to_string(),
        ));
    }
    Cron::parse(schedule).map_err(|e| ZapError::New(-1, format!("cron 表达式错误：{e}")))?;
    let cmd = command.trim();
    if cmd.is_empty() {
        return Err(ZapError::New(-1, "执行内容不能为空".to_string()));
    }
    if cmd.len() > 4096 {
        return Err(ZapError::New(
            -1,
            "执行内容过长（上限 4096 字节）".to_string(),
        ));
    }
    match kind {
        "script" => {
            if !cmd.starts_with('/') {
                return Err(ZapError::New(-1, "脚本路径必须是绝对路径".to_string()));
            }
            if cmd.contains("..") {
                return Err(ZapError::New(-1, "脚本路径不合法".to_string()));
            }
        }
        "command" => {}
        other => {
            return Err(ZapError::New(
                -1,
                format!("不支持的任务类型: {other}（仅 script / command）"),
            ));
        }
    }
    Ok(())
}

fn new_job_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let rand_part: u64 = rand::random::<u64>() & 0xffff_ffff;
    format!("{millis:x}{rand_part:08x}")
}

// ── CRUD ────────────────────────────────────────────────────

/// 列出任务，并计算（不落盘）下次运行时间。
pub async fn list(username: &str) -> Result<Vec<CronJob>, ZapError> {
    let mut ct = load(username).await?;
    let now = chrono::Local::now();
    for job in ct.jobs.iter_mut() {
        if !job.enabled {
            job.next_run_at = 0;
            continue;
        }
        if job.next_run_at > now.timestamp() {
            continue;
        }
        job.next_run_at = Cron::parse(&job.schedule)
            .map(|e| e.next_run_ts_after(&now))
            .unwrap_or(0);
    }
    Ok(ct.jobs)
}

pub async fn add(
    username: &str,
    name: &str,
    schedule: &str,
    kind: &str,
    command: &str,
    exec_user: &str,
    remark: &str,
) -> Result<String, ZapError> {
    validate_job(name, schedule, kind, command)?;
    let mut ct = load(username).await?;
    if ct.jobs.len() >= MAX_JOBS {
        return Err(ZapError::New(
            -1,
            format!("任务数量已达上限（{MAX_JOBS} 条）"),
        ));
    }
    let now = chrono::Local::now().timestamp();
    let job = CronJob {
        id: new_job_id(),
        name: name.trim().to_string(),
        schedule: schedule.trim().to_string(),
        kind: kind.to_string(),
        command: command.trim().to_string(),
        exec_user: exec_user.trim().to_string(),
        enabled: true,
        remark: remark.trim().to_string(),
        created_at: now,
        updated_at: now,
        last_run_at: 0,
        last_run_id: String::new(),
        last_status: String::new(),
        next_run_at: Cron::parse(schedule)
            .map(|e| e.next_run_ts_after(&chrono::Local::now()))
            .unwrap_or(0),
    };
    let id = job.id.clone();
    ct.jobs.push(job);
    save(username, &ct).await?;
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
pub async fn update(
    username: &str,
    id: &str,
    name: &str,
    schedule: &str,
    kind: &str,
    command: &str,
    exec_user: &str,
    remark: &str,
    enabled: bool,
) -> Result<(), ZapError> {
    validate_job(name, schedule, kind, command)?;
    let mut ct = load(username).await?;
    let job = ct
        .jobs
        .iter_mut()
        .find(|j| j.id == id)
        .ok_or_else(|| ZapError::New(-1, "任务不存在".to_string()))?;
    job.name = name.trim().to_string();
    job.schedule = schedule.trim().to_string();
    job.kind = kind.to_string();
    job.command = command.trim().to_string();
    job.exec_user = exec_user.trim().to_string();
    job.remark = remark.trim().to_string();
    job.enabled = enabled;
    job.updated_at = chrono::Local::now().timestamp();
    job.next_run_at = if enabled {
        Cron::parse(schedule)
            .map(|e| e.next_run_ts_after(&chrono::Local::now()))
            .unwrap_or(0)
    } else {
        0
    };
    save(username, &ct).await?;
    Ok(())
}

pub async fn delete(username: &str, id: &str) -> Result<Option<CronJob>, ZapError> {
    let mut ct = load(username).await?;
    let idx = ct.jobs.iter().position(|j| j.id == id);
    let removed = idx.map(|i| ct.jobs.remove(i));
    if removed.is_some() {
        save(username, &ct).await?;
    }
    Ok(removed)
}

pub async fn toggle(username: &str, id: &str, enabled: bool) -> Result<(), ZapError> {
    let mut ct = load(username).await?;
    let job = ct
        .jobs
        .iter_mut()
        .find(|j| j.id == id)
        .ok_or_else(|| ZapError::New(-1, "任务不存在".to_string()))?;
    job.enabled = enabled;
    job.updated_at = chrono::Local::now().timestamp();
    job.next_run_at = if enabled {
        Cron::parse(&job.schedule)
            .map(|e| e.next_run_ts_after(&chrono::Local::now()))
            .unwrap_or(0)
    } else {
        0
    };
    save(username, &ct).await?;
    Ok(())
}

pub async fn get(username: &str, id: &str) -> Result<Option<CronJob>, ZapError> {
    Ok(load(username).await?.jobs.into_iter().find(|j| j.id == id))
}

// ── 执行 ────────────────────────────────────────────────────

/// 记录一次运行（last_run_at / run_id / status=running）。
async fn mark_running(username: &str, id: &str, run_id: &str) -> Result<(), ZapError> {
    let mut ct = load(username).await?;
    if let Some(job) = ct.jobs.iter_mut().find(|j| j.id == id) {
        job.last_run_at = chrono::Local::now().timestamp();
        job.last_run_id = run_id.to_string();
        job.last_status = "running".to_string();
        job.updated_at = chrono::Local::now().timestamp();
    }
    save(username, &ct).await
}

/// 调用 zapexec 以 `job.exec_user` 身份运行任务。
async fn launch_exec(
    username: &str,
    job: &CronJob,
    run_id: &str,
    action: &str,
) -> Result<(), ZapError> {
    let log = log_path(username, run_id);
    // 登记运行记录（顺带按保留上限裁剪历史 + 删掉超出的日志文件）；
    // 登记失败只记日志，不阻断任务执行
    let key = job_key(username, &job.id);
    if let Err(e) =
        ast::register_run_with_key(run_id, action, &job.name, username, &log, &key).await
    {
        warn!("登记计划任务运行记录失败: {e}");
        // 记录没入库 → 撤回 last_run_id，否则「查看日志」会指向不存在的运行记录
        clear_last_run(username, &job.id, run_id).await;
    }
    let home = home_dir_of(&job.exec_user).await;
    let resp = zapexec::call(Request::CronRun {
        run_id: run_id.to_string(),
        linux_user: job.exec_user.clone(),
        home_dir: home,
        command: job.command.clone(),
        kind: job.kind.clone(),
        log_path: log.clone(),
    })
    .await?;
    if resp.code != 0 {
        return Err(ZapError::New(resp.code, resp.message));
    }
    watch_run(
        username.to_string(),
        job.id.clone(),
        run_id.to_string(),
        log,
    );
    Ok(())
}

/// 仅当 `last_run_id` 仍是本次（登记失败）运行的 `run_id` 时清空它。
async fn clear_last_run(username: &str, job_id: &str, run_id: &str) {
    if let Ok(mut ct) = load(username).await {
        let mut changed = false;
        if let Some(job) = ct.jobs.iter_mut().find(|j| j.id == job_id)
            && job.last_run_id == run_id
        {
            job.last_run_id = String::new();
            changed = true;
        }
        if changed {
            let _ = save(username, &ct).await;
        }
    }
}

/// 回写某个任务的状态（运行中 -> success / failed）。
async fn update_status(username: &str, job_id: &str, status: &str) {
    if let Ok(mut ct) = load(username).await
        && let Some(job) = ct.jobs.iter_mut().find(|j| j.id == job_id)
    {
        job.last_status = status.to_string();
        job.updated_at = chrono::Local::now().timestamp();
        let _ = save(username, &ct).await;
    }
}

/// 后台轮询日志末尾的完成标记，回写任务状态。
fn watch_run(username: String, job_id: String, run_id: String, log: String) {
    tokio::spawn(async move {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(24 * 3600);
        loop {
            match read_done_marker(&log).await {
                Some(code) => {
                    let status = if code == 0 { "success" } else { "failed" };
                    update_status(&username, &job_id, status).await;
                    ast::finish_run(&run_id, status, code).await;
                    break;
                }
                None => {
                    if std::time::Instant::now() > deadline {
                        update_status(&username, &job_id, "failed").await;
                        ast::finish_run(&run_id, "failed", -1).await;
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                }
            }
        }
    });
}

/// 后台运行一次任务，返回 run_id。
pub async fn run_now(username: &str, id: &str) -> Result<String, ZapError> {
    let job = get(username, id)
        .await?
        .ok_or_else(|| ZapError::New(-1, "任务不存在".to_string()))?;
    if job.exec_user.trim().is_empty() {
        return Err(ZapError::New(-1, "任务未配置执行用户".to_string()));
    }
    let run_id = ast::generate_run_id();
    mark_running(username, id, &run_id).await?;
    if let Err(e) = launch_exec(username, &job, &run_id, "manual").await {
        let mut ct = load(username).await?;
        if let Some(j) = ct.jobs.iter_mut().find(|j| j.id == id) {
            j.last_status = "failed".to_string();
        }
        let _ = save(username, &ct).await;
        ast::finish_run(&run_id, "failed", -1).await;
        return Err(e);
    }
    Ok(run_id)
}

pub async fn read_log(username: &str, run_id: &str) -> Result<(String, Option<i64>), ZapError> {
    if run_id.is_empty()
        || run_id.len() > 64
        || !run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(ZapError::New(-1, "run_id 不合法".to_string()));
    }
    let path = log_path(username, run_id);
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(ZapError::New(-1, format!("读取日志失败: {e}"))),
    };
    Ok((content.clone(), done_code(&content)))
}

/// 从日志内容中提取 `__ZAP_DONE__ <code>` 的退出码。
fn done_code(content: &str) -> Option<i64> {
    content
        .rsplit(ast::DONE_MARKER)
        .next()
        .filter(|_| content.contains(ast::DONE_MARKER))
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|c| c.parse().ok())
}

async fn read_done_marker(log_path: &str) -> Option<i64> {
    let content = tokio::fs::read_to_string(log_path).await.ok()?;
    done_code(&content)
}

// ── 调度器 ──────────────────────────────────────────────────

/// 启动用户计划任务调度器（后台，按分钟对齐评估一次）。
pub fn start() {
    tokio::spawn(async move {
        info!("用户计划任务（crontab）调度器已启动");
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        let mut last_minute: i64 = -1;
        loop {
            interval.tick().await;
            let now = chrono::Local::now();
            let minute_key = now.timestamp() / 60;
            if minute_key == last_minute {
                continue;
            }
            last_minute = minute_key;
            tick_once(&now).await;
        }
    });
}

/// 枚举所有存在 crontab.yaml 的面板用户。
fn scan_users() -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(users_dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || !path.join("crontab.yaml").is_file() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && safe_username(name).is_ok()
        {
            out.push(name.to_string());
        }
    }
    out
}

async fn tick_once(now: &chrono::DateTime<chrono::Local>) {
    let ts = now.timestamp();
    for username in scan_users() {
        let mut ct = match load(&username).await {
            Ok(ct) => ct,
            Err(e) => {
                warn!("读取用户 {username} 的 crontab 失败: {e}");
                continue;
            }
        };
        let mut dirty = false;
        for job in ct.jobs.iter_mut() {
            if !job.enabled {
                continue;
            }
            let expr = match Cron::parse(&job.schedule) {
                Ok(e) => e,
                Err(e) => {
                    warn!("用户 {username} 任务 {} 表达式非法: {}", job.id, e);
                    continue;
                }
            };
            if expr.matches(now) && job.last_run_at < ts - DEDUP_WINDOW {
                let run_id = ast::generate_run_id();
                job.last_run_at = ts;
                job.last_run_id = run_id.clone();
                job.last_status = "running".to_string();
                dirty = true;
                let username = username.clone();
                let job = job.clone();
                tokio::spawn(async move {
                    match launch_exec(&username, &job, &run_id, "cron").await {
                        Ok(()) => info!("用户 {username} 计划任务 {} 已触发", job.id),
                        Err(e) => warn!("用户 {username} 计划任务 {} 触发失败: {e}", job.id),
                    }
                });
            }
        }
        if dirty && let Err(e) = save(&username, &ct).await {
            warn!("回写用户 {username} 的 crontab 失败: {e}");
        }
    }
}

// ── 运行历史 ────────────────────────────────────────────────

/// 列出某任务最近的运行历史（新的在前）。
pub async fn list_runs(username: &str, job_id: &str) -> Result<Vec<ast::AppstoreRun>, sqlx::Error> {
    ast::list_runs_by_key(&job_key(username, job_id), ast::MAX_RUNS_PER_JOB).await
}

/// 清空某任务的运行历史（DB 记录 + 日志文件），并断开 crontab.yaml 里的
/// last_run_id 引用 —— 否则「上次运行」会指向已被删除的日志。
pub async fn clear_runs(username: &str, job_id: &str) -> Result<i64, ZapError> {
    let n = ast::delete_runs_by_key(&job_key(username, job_id)).await?;
    if let Ok(mut ct) = load(username).await
        && let Some(job) = ct.jobs.iter_mut().find(|j| j.id == job_id)
    {
        job.last_run_id = String::new();
        job.last_run_at = 0;
        job.last_status = String::new();
        let _ = save(username, &ct).await;
    }
    Ok(n)
}

/// 清理历史遗留的孤儿日志：目录下 `run-<id>.log` 里 DB 查不到对应记录的那些。
///
/// 本模块此前只写日志文件、不登记记录，那个时期留下的日志无法归属到任务，
/// 只能按"有没有记录"判断；查库出错时保守处理（当作有记录，不删）。
pub async fn purge_orphan_logs(username: &str) -> Result<i64, ZapError> {
    safe_username(username)?;
    let dir = logs_dir(username);
    let pool = crate::db::get_db_pool().await;
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        // 目录不存在 = 该用户还没跑过任务
        Err(_) => return Ok(0),
    };
    let mut removed = 0i64;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(run_id) = name
            .strip_prefix("run-")
            .and_then(|r| r.strip_suffix(".log"))
        else {
            continue;
        };
        let known: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_queue WHERE task_id = ?)")
                .bind(run_id)
                .fetch_one(pool)
                .await
                .unwrap_or(true);
        if !known && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}
