//! 脚本/自动化：计划任务（admin 专属，系统级 root 权限执行）。
//!
//! - cron 表达式为 5 段：`分 时 日 月 周`（标准 crontab 语法）
//! - **存储**：每个管理员一份 `{data}/users/<username>/cron-jobs.yaml`
//!   （YAML 为唯一事实来源，任务归属创建者，与用户 crontab 同构）
//! - **执行**：调度器每分钟扫描一次 enabled 任务，命中即触发脚本运行。
//!   执行链路复用 AppStore 脚本运行：`task_queue` 记录 + 日志监控
//!   （脚本以 `SCRIPT_OWNER`（admin）的脚本目录运行于 zapexec，
//!   日志见运行记录 / 实时日志）。

use std::path::PathBuf;

use chrono::{Datelike, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use tokio::time::Duration;
use tracing::{info, warn};

use crate::zap::ZapError;
use crate::zap::appstore as ast;
use crate::zap::user_cron::{safe_username, users_dir};
use crate::zapexec;
use zap_proto::Request;

/// 单个管理员的任务数量上限。
pub const MAX_JOBS: usize = 100;
/// 立即运行 / 调度触发的最小间隔（秒），用于同一分钟内防重。
const DEDUP_WINDOW: i64 = 50;
/// 计划任务是系统级运维任务：脚本一律取自 admin 的脚本目录
/// （`{data}/users/admin/scripts/`），与「自定义脚本」同源。
pub const SCRIPT_OWNER: &str = "admin";
/// 落盘文件名（位于 `{data}/users/<username>/` 下）。
const FILE_NAME: &str = "cron-jobs.yaml";

// ── 数据结构 ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// 相对脚本目录的路径（如 `scripts/backup.sh`）
    #[serde(default)]
    pub script_path: String,
    /// 5 段标准 cron：`分 时 日 月 周`
    #[serde(default)]
    pub schedule: String,
    #[serde(default)]
    pub remark: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub last_run_at: i64,
    #[serde(default)]
    pub last_run_id: String,
    #[serde(default)]
    pub next_run_at: i64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CronFile {
    #[serde(default = "default_version")]
    pub version: i64,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub jobs: Vec<CronJob>,
}

fn default_true() -> bool {
    true
}

fn default_version() -> i64 {
    1
}

// ── Cron 表达式（5 段）──────────────────────────────────────

pub struct Cron {
    minutes: [bool; 60],
    hours: [bool; 24],
    doms: [bool; 32],   // 1..=31
    months: [bool; 13], // 1..=12
    dows: [bool; 8],    // 0..=6（解析期允许 7，合并到 0）
    dom_all: bool,
    dow_all: bool,
}

impl Cron {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err("cron 表达式需为 5 段：分 时 日 月 周（如 */5 * * * *）".to_string());
        }
        let mut minutes = [false; 60];
        let mut hours = [false; 24];
        let mut doms = [false; 32];
        let mut months = [false; 13];
        let mut dows = [false; 8];

        parse_field(parts[0], 0, 59, &mut minutes, false)?;
        parse_field(parts[1], 0, 23, &mut hours, false)?;
        parse_field(parts[2], 1, 31, &mut doms, false)?;
        parse_field(parts[3], 1, 12, &mut months, false)?;
        parse_field(parts[4], 0, 7, &mut dows, true)?;
        // 周 7 == 周日 0
        dows[0] |= dows[7];
        dows[7] = false;

        let dom_all = parts[2] == "*";
        let dow_all = parts[4] == "*";
        Ok(Self {
            minutes,
            hours,
            doms,
            months,
            dows,
            dom_all,
            dow_all,
        })
    }

    /// 判断当前时间是否命中（日与周同时限制时按 crontab 语义取“或”）。
    pub fn matches(&self, dt: &chrono::DateTime<chrono::Local>) -> bool {
        if !self.minutes[dt.minute() as usize] {
            return false;
        }
        if !self.hours[dt.hour() as usize] {
            return false;
        }
        if !self.months[dt.month() as usize] {
            return false;
        }
        let dom = dt.day() as usize;
        let dow = dt.weekday().num_days_from_sunday() as usize;
        match (self.dom_all, self.dow_all) {
            (true, true) => true,
            (true, false) => self.dows[dow],
            (false, true) => self.doms[dom],
            (false, false) => self.doms[dom] || self.dows[dow],
        }
    }

    /// 计算 `now` 之后（严格大于当前分钟）的下一次命中 Unix 秒；一年内找不到返回 0。
    pub fn next_run_ts_after(&self, now: &chrono::DateTime<chrono::Local>) -> i64 {
        let base = now
            .with_second(0)
            .map(|t| t.timestamp())
            .unwrap_or_else(|| now.timestamp());
        let cap = base + 366 * 24 * 3600;
        let mut cand = base + 60;
        while cand <= cap {
            if let Some(dt) = chrono::Local.timestamp_opt(cand, 0).single()
                && self.matches(&dt)
            {
                return cand;
            }
            cand += 60;
        }
        0
    }
}

/// 解析单个 cron 字段（支持 `*`、`*/n`、`a-b`、`a-b/n`、`n`、逗号列表）。
fn parse_field(
    s: &str,
    min: usize,
    max: usize,
    out: &mut [bool],
    sunday_seven: bool,
) -> Result<(), String> {
    for item in s.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (range_part, step) = match item.split_once('/') {
            Some((r, st)) => {
                let st: usize = st
                    .trim()
                    .parse()
                    .map_err(|_| format!("cron 步进非法: {item}"))?;
                if st == 0 {
                    return Err(format!("cron 步进不能为 0: {item}"));
                }
                (r, st)
            }
            None => (item, 1),
        };
        let (lo, hi) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let a: usize = a
                .trim()
                .parse()
                .map_err(|_| format!("cron 数值非法: {item}"))?;
            let b: usize = b
                .trim()
                .parse()
                .map_err(|_| format!("cron 数值非法: {item}"))?;
            (a, b)
        } else {
            let v: usize = range_part
                .trim()
                .parse()
                .map_err(|_| format!("cron 数值非法: {item}"))?;
            (v, v)
        };
        if lo > hi || lo < min || hi > max {
            return Err(format!("cron 数值超出范围[{min}-{max}]: {item}"));
        }
        let mut v = lo;
        while v <= hi {
            let idx = if sunday_seven { v % 8 } else { v };
            out[idx] = true;
            v += step;
        }
    }
    Ok(())
}

// ── 路径与读写 ──────────────────────────────────────────────

/// 某管理员的计划任务文件：`{data}/users/<username>/cron-jobs.yaml`。
pub fn cron_file_path(username: &str) -> PathBuf {
    users_dir().join(username).join(FILE_NAME)
}

/// 运行记录在 `task_queue` 表中的任务归属键。
///
/// 带上用户名是因为任务按管理员隔离（`cron-jobs.yaml` 各存一份），
/// 不同管理员的任务 id 不保证全局唯一。
pub fn job_key(username: &str, job_id: &str) -> String {
    format!("cron:{username}:{job_id}")
}

fn empty_file(username: &str) -> CronFile {
    CronFile {
        version: 1,
        owner: username.to_string(),
        updated_at: 0,
        jobs: Vec::new(),
    }
}

pub fn load_sync(username: &str) -> Result<CronFile, ZapError> {
    safe_username(username)?;
    let path = cron_file_path(username);
    if !path.exists() {
        return Ok(empty_file(username));
    }
    let text = std::fs::read_to_string(&path)?;
    if text.trim().is_empty() {
        return Ok(empty_file(username));
    }
    let mut file: CronFile = serde_yaml::from_str(&text)
        .map_err(|e| ZapError::New(-1, format!("{FILE_NAME} 解析失败: {e}")))?;
    file.owner = username.to_string();
    Ok(file)
}

pub async fn load(username: &str) -> Result<CronFile, ZapError> {
    let username = username.to_string();
    tokio::task::spawn_blocking(move || load_sync(&username))
        .await
        .unwrap_or_else(|e| Err(ZapError::New(-1, format!("读取失败: {e}"))))
}

/// 写入 cron-jobs.yaml（临时文件 + rename 原子替换）。
pub fn save_sync(username: &str, file: &CronFile) -> Result<(), ZapError> {
    safe_username(username)?;
    let path = cron_file_path(username);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = file.clone();
    out.version = 1;
    out.owner = username.to_string();
    out.updated_at = chrono::Local::now().timestamp();
    let text = serde_yaml::to_string(&out)
        .map_err(|e| ZapError::New(-1, format!("{FILE_NAME} 序列化失败: {e}")))?;
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub async fn save(username: &str, file: &CronFile) -> Result<(), ZapError> {
    let username = username.to_string();
    let file = file.clone();
    tokio::task::spawn_blocking(move || save_sync(&username, &file))
        .await
        .unwrap_or_else(|e| Err(ZapError::New(-1, format!("保存失败: {e}"))))
}

fn new_job_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let rand_part: u64 = rand::random::<u64>() & 0xffff_ffff;
    format!("{millis:x}{rand_part:08x}")
}

fn next_ts(schedule: &str) -> i64 {
    Cron::parse(schedule)
        .map(|e| e.next_run_ts_after(&chrono::Local::now()))
        .unwrap_or(0)
}

// ── CRUD ────────────────────────────────────────────────────

/// 列出当前管理员的任务，并计算（不落盘）下次运行时间。
pub async fn list(username: &str) -> Result<Vec<CronJob>, ZapError> {
    let mut file = load(username).await?;
    let now = chrono::Local::now().timestamp();
    for job in file.jobs.iter_mut() {
        if !job.enabled {
            job.next_run_at = 0;
            continue;
        }
        if job.next_run_at > now {
            continue;
        }
        job.next_run_at = next_ts(&job.schedule);
    }
    Ok(file.jobs)
}

pub async fn get(username: &str, id: &str) -> Result<Option<CronJob>, ZapError> {
    Ok(load(username).await?.jobs.into_iter().find(|j| j.id == id))
}

pub async fn add(
    username: &str,
    name: &str,
    script_path: &str,
    schedule: &str,
    remark: &str,
) -> Result<String, ZapError> {
    let mut file = load(username).await?;
    if file.jobs.len() >= MAX_JOBS {
        return Err(ZapError::New(
            -1,
            format!("任务数量已达上限（{MAX_JOBS} 条）"),
        ));
    }
    let now = chrono::Local::now().timestamp();
    let job = CronJob {
        id: new_job_id(),
        name: name.trim().to_string(),
        script_path: script_path.trim().to_string(),
        schedule: schedule.trim().to_string(),
        remark: remark.trim().to_string(),
        enabled: true,
        last_run_at: 0,
        last_run_id: String::new(),
        next_run_at: next_ts(schedule),
        created_at: now,
        updated_at: now,
    };
    let id = job.id.clone();
    file.jobs.push(job);
    save(username, &file).await?;
    Ok(id)
}

pub async fn update(
    username: &str,
    id: &str,
    name: &str,
    script_path: &str,
    schedule: &str,
    remark: &str,
    enabled: bool,
) -> Result<(), ZapError> {
    let mut file = load(username).await?;
    let job = file
        .jobs
        .iter_mut()
        .find(|j| j.id == id)
        .ok_or_else(|| ZapError::New(-1, "计划任务不存在".to_string()))?;
    job.name = name.trim().to_string();
    job.script_path = script_path.trim().to_string();
    job.schedule = schedule.trim().to_string();
    job.remark = remark.trim().to_string();
    job.enabled = enabled;
    job.updated_at = chrono::Local::now().timestamp();
    job.next_run_at = if enabled { next_ts(schedule) } else { 0 };
    save(username, &file).await
}

pub async fn delete(username: &str, id: &str) -> Result<Option<CronJob>, ZapError> {
    let mut file = load(username).await?;
    let idx = file.jobs.iter().position(|j| j.id == id);
    let removed = idx.map(|i| file.jobs.remove(i));
    if removed.is_some() {
        save(username, &file).await?;
    }
    Ok(removed)
}

pub async fn set_enabled(username: &str, id: &str, enabled: bool) -> Result<(), ZapError> {
    let mut file = load(username).await?;
    let job = file
        .jobs
        .iter_mut()
        .find(|j| j.id == id)
        .ok_or_else(|| ZapError::New(-1, "计划任务不存在".to_string()))?;
    job.enabled = enabled;
    job.updated_at = chrono::Local::now().timestamp();
    job.next_run_at = if enabled { next_ts(&job.schedule) } else { 0 };
    save(username, &file).await
}

/// 记录一次运行（last_run_at / last_run_id）。
pub async fn mark_last_run(username: &str, id: &str, run_id: &str) -> Result<(), ZapError> {
    let mut file = load(username).await?;
    if let Some(job) = file.jobs.iter_mut().find(|j| j.id == id) {
        job.last_run_at = chrono::Local::now().timestamp();
        job.last_run_id = run_id.to_string();
        job.updated_at = chrono::Local::now().timestamp();
    }
    save(username, &file).await
}

/// 仅当 `last_run_id` 仍是本次（失败）运行的 `run_id` 时清空它，
/// 避免误删之后已经成功登记的新运行引用。
pub async fn clear_last_run(username: &str, id: &str, run_id: &str) {
    if let Ok(mut file) = load(username).await {
        let mut changed = false;
        if let Some(job) = file.jobs.iter_mut().find(|j| j.id == id)
            && job.last_run_id == run_id
        {
            job.last_run_id = String::new();
            changed = true;
        }
        if changed {
            let _ = save(username, &file).await;
        }
    }
}

/// 清空某任务的运行历史后断开 `last_run_id` 引用。
pub async fn reset_last_run(username: &str, id: &str) {
    if let Ok(mut file) = load(username).await {
        let mut changed = false;
        if let Some(job) = file.jobs.iter_mut().find(|j| j.id == id) {
            job.last_run_id = String::new();
            job.last_run_at = 0;
            changed = true;
        }
        if changed {
            let _ = save(username, &file).await;
        }
    }
}

/// 历史遗留清理：清掉所有 `cron-jobs.yaml` 里指向已删除运行记录的 `last_run_id`，
/// 否则「查看日志」连不上。
pub async fn clear_dangling_last_run_ids() {
    let pool = crate::db::get_db_pool().await;
    for username in scan_users() {
        let mut file = match load(&username).await {
            Ok(f) => f,
            Err(_) => continue,
        };
        let mut changed = false;
        for job in file.jobs.iter_mut() {
            if job.last_run_id.is_empty() {
                continue;
            }
            let alive: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_queue WHERE task_id = ?)")
                    .bind(&job.last_run_id)
                    .fetch_one(pool)
                    .await
                    .unwrap_or(true);
            if !alive {
                job.last_run_id = String::new();
                changed = true;
            }
        }
        if changed {
            let _ = save(&username, &file).await;
        }
    }
}

// ── 执行与调度 ──────────────────────────────────────────────

/// 执行一次脚本运行（与「自定义脚本 → 运行」同一链路）。
///
/// `action` 用于运行记录区分（cron / manual）；`username` 是任务归属的
/// 管理员（决定数据文件与运行记录归属），脚本一律以 `SCRIPT_OWNER` 身份执行。
pub async fn launch_script_run(
    path: &str,
    action: &str,
    username: &str,
    job_id: &str,
) -> Result<String, ZapError> {
    let run_id = ast::generate_run_id();
    launch_script_run_with_id(path, action, username, job_id, &run_id).await
}

/// 用指定的 `run_id` 执行一次脚本运行。
///
/// 调度器需要先回写 `last_run_at` 做防重（同步进行，避免同一分钟重复触发），
/// 因此必须复用同一个 `run_id`：否则 `last_run_id` 会指向一条从未登记的运行
/// 记录，前端点「查看日志」时 WebSocket 握手会被拒，表现为「连接错误」。
pub async fn launch_script_run_with_id(
    path: &str,
    action: &str,
    username: &str,
    job_id: &str,
    run_id: &str,
) -> Result<String, ZapError> {
    let log_path = ast::log_path_for(run_id);
    let key = job_key(username, job_id);
    ast::register_run_with_key(run_id, action, path, username, &log_path, &key).await?;
    let resp = zapexec::call(Request::AppstoreScriptRun {
        path: path.to_string(),
        run_id: run_id.to_string(),
        username: SCRIPT_OWNER.to_string(),
    })
    .await?;
    if resp.code != 0 {
        ast::finish_run(run_id, "failed", resp.code as i64).await;
        return Err(ZapError::New(resp.code, resp.message));
    }
    ast::watch_log(run_id.to_string(), log_path);
    Ok(run_id.to_string())
}

/// 启动计划任务调度器（后台每分钟评估一次）。
pub fn start() {
    tokio::spawn(async move {
        info!("脚本计划任务调度器已启动");
        // 历史遗留：清掉指向已删除运行记录的 last_run_id，否则「查看日志」连不上
        clear_dangling_last_run_ids().await;
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

/// 枚举所有存在 cron-jobs.yaml 的面板用户。
fn scan_users() -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(users_dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || !path.join(FILE_NAME).is_file() {
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
        let mut file = match load(&username).await {
            Ok(f) => f,
            Err(e) => {
                warn!("读取 {username} 的计划任务失败: {e}");
                continue;
            }
        };
        let mut dirty = false;
        for job in file.jobs.iter_mut() {
            if !job.enabled {
                continue;
            }
            let expr = match Cron::parse(&job.schedule) {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        "{username} 计划任务 {} 表达式非法({}): {}",
                        job.id, job.schedule, e
                    );
                    continue;
                }
            };
            // 命中：防重（上一次运行须在 DEDUP_WINDOW 秒前，避免同一分钟内重复触发）
            if expr.matches(now) && job.last_run_at < ts - DEDUP_WINDOW {
                let run_id = ast::generate_run_id();
                job.last_run_at = ts;
                job.last_run_id = run_id.clone();
                dirty = true;
                let username = username.clone();
                let job = job.clone();
                tokio::spawn(async move {
                    match launch_script_run_with_id(
                        &job.script_path,
                        "cron",
                        &username,
                        &job.id,
                        &run_id,
                    )
                    .await
                    {
                        Ok(rid) => {
                            info!(
                                "{username} 计划任务 {} 已触发: {} ({rid})",
                                job.id, job.script_path
                            );
                        }
                        Err(e) => {
                            // 运行登记/启动失败：撤掉悬空的 last_run_id
                            clear_last_run(&username, &job.id, &run_id).await;
                            warn!(
                                "{username} 计划任务 {} 触发失败: {}: {e}",
                                job.id, job.script_path
                            );
                        }
                    }
                });
            }
        }
        if dirty && let Err(e) = save(&username, &file).await {
            warn!("回写 {username} 的计划任务失败: {e}");
        }
    }
}
