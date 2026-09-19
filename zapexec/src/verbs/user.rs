//! 面板用户家目录骨架 / Linux 系统账号管理（root 执行）。
//!
//! 契约：每个面板用户在 `user.home_dir`（通常为 `/home/{linux_user}`）下拥有
//! 私有空间：
//! - `{home_dir}/www/{sanitize(site)}-{site_id}` —— 站点文档根（web tree）
//! - `{home_dir}/logs/{site_id}-{sanitize(site)}` —— 站点 access/error 日志（log tree）
//! - `{home_dir}/tmp` —— PHP session / 上传临时目录（open_basedir 白名单）
//!
//! 运行账号 `owner` 为该面板用户对应的 Linux 系统账号（nologin）：
//! web tree 归 `{u}:{u 主组}`、目录 755（nginx worker 走 others 位读取静态文件，
//! 站点文件不归属 www 组），PHP-FPM 以「该用户 × 该 PHP 版本」独立 pool 运行。
//!
//! 安全边界：家目录只接受 `/home/` 下绝对路径；禁止 `..`；Linux 账号名白名单校验。

use std::path::{Path, PathBuf};

use serde_json::json;
use zap_proto::Response;

use super::root_cmd;

/// 家目录路径校验：必须是绝对路径、不含 `..`、且不是文件系统根。
/// 允许任意挂载点前缀（/home、/home2、/data/home 等），便于磁盘扩容迁移。
fn home_dir_ok(home: &str) -> bool {
    PathBuf::from(home).is_absolute()
        && !home.split('/').any(|s| s == "..")
        && home != "/"
        && !home.contains(' ')
}

pub(crate) fn linux_user_ok(u: &str) -> bool {
    if u.is_empty() || u.len() > 32 {
        return false;
    }
    let mut chars = u.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 账号实际主组名（`id -gn`）。
///
/// 账号主组不再假设与账号同名：同名组被系统占用时账号会落在 `zap_<user>` 专属组，
/// 故一律以系统记录为准。账号不存在 / 查询失败时回退同名组。
pub(crate) fn run_group_of(linux_user: &str) -> String {
    root_cmd("id")
        .args(["-gn", linux_user])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| linux_user.to_string())
}

fn group_exists(name: &str) -> bool {
    root_cmd("getent")
        .args(["group", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 同名组已被系统占用（如发行版预置的 admin / sudo 组）时使用的专属组名。
/// 不把面板账号并入系统组以避免继承额外权限；组名长度上限 32。
fn dedicated_group_name(linux_user: &str) -> String {
    let mut g = format!("zap_{linux_user}");
    g.truncate(32);
    g
}

/// 账号移除后清理其运行组：仅当该组是普通组（gid ≥ 1000）且已无附加成员时才删。
/// 系统组（gid < 1000，如预置的 admin / sudo）绝不触碰。
fn remove_run_group(name: &str) {
    let Ok(o) = root_cmd("getent").args(["group", name]).output() else {
        return;
    };
    if !o.status.success() {
        return;
    }
    let line = String::from_utf8_lossy(&o.stdout).trim().to_string();
    let mut fields = line.split(':');
    let gid: u32 = fields.nth(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let members = fields.next().unwrap_or("").trim();
    if gid >= 1000 && members.is_empty() {
        let _ = root_cmd("groupdel").arg(name).output();
    }
}

fn cmd_err(o: &std::process::Output, fallback: &str) -> String {
    let text = String::from_utf8_lossy(&o.stderr).trim().to_string();
    let text = if text.is_empty() {
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    } else {
        text
    };
    if text.is_empty() {
        fallback.to_string()
    } else {
        text
    }
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ── user.home_init ──────────────────────────────────────────

pub async fn home_init(home_dir: &str, owner: &str) -> Response {
    let home_dir = home_dir.to_string();
    let owner = owner.to_string();
    tokio::task::spawn_blocking(move || home_init_inner(&home_dir, &owner))
        .await
        .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
        .unwrap_or_else(|e| Response::err(-1, e))
}

fn run_bash(script: &str) -> Result<(), String> {
    let o = root_cmd("bash")
        .args(["-c", script])
        .output()
        .map_err(|e| format!("执行命令失败: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(cmd_err(&o, "命令执行失败"))
    }
}

fn home_init_inner(home_dir: &str, owner: &str) -> Result<Response, String> {
    let home = home_dir.trim();
    if home.is_empty() {
        return Err("home_dir 不能为空".to_string());
    }
    if !home_dir_ok(home) {
        return Err(format!(
            "home_dir 非法（必须为挂载点下的绝对路径，不含 ..）: {home}"
        ));
    }
    if !linux_user_ok(owner) {
        return Err(format!("非法的 Linux 账号名: {owner}"));
    }
    let run = owner;
    // 进程主组：账号独立组（不放进 www 组，避免跨用户读 php 源码）；
    // 以系统实际主组为准（同名组被系统占用时账号落在 zap_<user> 专属组）
    let run_group = run_group_of(run);
    let home_p = PathBuf::from(home);
    std::fs::create_dir_all(&home_p).map_err(|e| format!("创建家目录失败 {home_p:?}: {e}"))?;
    let mut created: Vec<String> = Vec::new();
    for sub in ["www", "logs", "tmp"] {
        let d = home_p.join(sub);
        std::fs::create_dir_all(&d).map_err(|e| format!("创建子目录失败 {d:?}: {e}"))?;
        created.push(d.to_string_lossy().to_string());
    }
    let q = |p: &str| sh_quote(p);

    // web tree（www）：递归归 {run}:{run_group}，目录 755
    // （nginx worker 走 others 位读取静态文件，站点文件不归属 www 组）
    run_bash(&format!(
        "chown -R {}:{} {}",
        q(run),
        q(&run_group),
        q(&format!("{home}/www"))
    ))?;
    run_bash(&format!("chmod 755 {}", q(&format!("{home}/www"))))?;
    // log tree（logs）：nginx 写 access/error.log，恒归 www:www，组可写
    run_bash(&format!("chown -R www:www {}", q(&format!("{home}/logs"))))?;
    run_bash(&format!("chmod 770 {}", q(&format!("{home}/logs"))))?;
    // session/upload 临时目录：进程身份独占
    run_bash(&format!(
        "chown -R {}:{} {}",
        q(run),
        q(&run_group),
        q(&format!("{home}/tmp"))
    ))?;
    run_bash(&format!("chmod 700 {}", q(&format!("{home}/tmp"))))?;
    // 家目录顶层：归该账号，o+x（nginx 可进入但不可列）
    run_bash(&format!("chown {}:{} {}", q(run), q(&run_group), q(home)))?;
    run_bash(&format!("chmod 711 {}", q(home)))?;

    Ok(Response::ok(
        format!("家目录已就绪：{home}（运行账号 {run}）"),
        Some(json!({ "home_dir": home, "dirs": created, "owner": run, "mode": "system" })),
    ))
}

// ── user.system_init（创建 Linux 账号，幂等）──────────────────

pub async fn system_init(linux_user: &str, home_dir: &str) -> Response {
    let linux_user = linux_user.to_string();
    let home_dir = home_dir.to_string();
    tokio::task::spawn_blocking(move || system_init_inner(&linux_user, &home_dir))
        .await
        .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
        .unwrap_or_else(|e| Response::err(-1, e))
}

fn system_init_inner(linux_user: &str, home_dir: &str) -> Result<Response, String> {
    if !linux_user_ok(linux_user) {
        return Err(format!("非法的 Linux 账号名: {linux_user}"));
    }
    if !home_dir_ok(home_dir) {
        return Err(format!(
            "home_dir 非法（必须为挂载点下的绝对路径，不含 ..）: {home_dir}"
        ));
    }
    let id = root_cmd("id")
        .args(["-u", linux_user])
        .output()
        .map_err(|e| format!("执行 id 失败: {e}"))?;
    if id.status.success() {
        return Ok(Response::ok(
            format!("Linux 账号 {linux_user} 已存在（跳过创建）"),
            None,
        ));
    }
    // nologin shell（Debian/Ubuntu 通常在 /usr/sbin/nologin）
    let shell = root_cmd("bash")
        .args([
            "-c",
            "command -v nologin 2>/dev/null || echo /usr/sbin/nologin",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/sbin/nologin".to_string());
    // 账号主组：优先同名组；同名组已被系统占用（如发行版预置的 admin / sudo 组）时
    // 改用专属组 zap_<user>，避免把面板账号并入系统管理组而继承额外权限。
    // 后续家目录属组与 PHP-FPM pool 的 group 都按账号实际主组解析（id -gn）。
    let gname = if group_exists(linux_user) {
        dedicated_group_name(linux_user)
    } else {
        linux_user.to_string()
    };
    if !group_exists(&gname) {
        let g = root_cmd("groupadd")
            .arg(&gname)
            .output()
            .map_err(|e| format!("执行 groupadd 失败: {e}"))?;
        if !g.status.success() {
            return Err(format!(
                "创建运行组失败：{}",
                cmd_err(&g, "groupadd 返回非零")
            ));
        }
    }
    // -M：不自动创建家目录（目录由 user.home_init 建好并赋权）
    let o = root_cmd("useradd")
        .args(["-M", "-s", &shell, "-d", home_dir, "-g", &gname, linux_user])
        .output()
        .map_err(|e| format!("执行 useradd 失败: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "创建 Linux 账号失败：{}",
            cmd_err(&o, "useradd 返回非零")
        ));
    }
    Ok(Response::ok(
        format!("Linux 账号 {linux_user} 已创建（home={home_dir}, group={gname}）"),
        Some(json!({
            "linux_user": linux_user,
            "home_dir": home_dir,
            "shell": shell,
            "run_group": gname,
        })),
    ))
}

// ── user.system_remove（移除 Linux 账号，幂等）────────────────

pub async fn system_remove(linux_user: &str) -> Response {
    // 先清该用户在所有 PHP 实例中的 pool 并 reload（否则删账号后 fpm -t 报错）
    let clean = super::php::pool_clean(linux_user.to_string()).await;
    if clean.code != 0 {
        return clean;
    }
    let linux_user = linux_user.to_string();
    tokio::task::spawn_blocking(move || system_remove_inner(&linux_user))
        .await
        .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
        .unwrap_or_else(|e| Response::err(-1, e))
}

fn system_remove_inner(linux_user: &str) -> Result<Response, String> {
    if !linux_user_ok(linux_user) {
        return Err(format!("非法的 Linux 账号名: {linux_user}"));
    }
    let id = root_cmd("id")
        .args(["-u", linux_user])
        .output()
        .map_err(|e| format!("执行 id 失败: {e}"))?;
    if !id.status.success() {
        return Ok(Response::ok(
            format!("Linux 账号 {linux_user} 不存在（跳过）"),
            None,
        ));
    }
    let o = root_cmd("userdel")
        .arg(linux_user)
        .output()
        .map_err(|e| format!("执行 userdel 失败: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "移除 Linux 账号失败：{}",
            cmd_err(&o, "userdel 返回非零")
        ));
    }
    // 账号运行组随之清理（系统预置组 / 仍被引用的组由 remove_run_group 自动跳过）
    remove_run_group(linux_user);
    remove_run_group(&dedicated_group_name(linux_user));
    Ok(Response::ok(
        format!("Linux 账号 {linux_user} 已移除"),
        None,
    ))
}

// ── user.home_migrate（家目录跨挂载点迁移）──────────────────

pub async fn migrate_home(src_home: &str, dest_home: &str, owner: &str) -> Response {
    let src_home = src_home.to_string();
    let dest_home = dest_home.to_string();
    let owner = owner.to_string();
    tokio::task::spawn_blocking(move || migrate_home_inner(&src_home, &dest_home, &owner))
        .await
        .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
        .unwrap_or_else(|e| Response::err(-1, e))
}

fn migrate_home_inner(src_home: &str, dest_home: &str, owner: &str) -> Result<Response, String> {
    let src = src_home.trim().trim_end_matches('/').to_string();
    let dest = dest_home.trim().trim_end_matches('/').to_string();
    for (name, p) in [("源家目录", src.as_str()), ("目标家目录", dest.as_str())] {
        if p.is_empty() || !home_dir_ok(p) {
            return Err(format!(
                "{name}非法（必须为挂载点下的绝对路径，不含 ..）: {p}"
            ));
        }
    }
    if src == dest {
        return Err("源与目标家目录相同，无需迁移".to_string());
    }
    let src_name = src.rsplit('/').next().unwrap_or("");
    let dest_name = dest.rsplit('/').next().unwrap_or("");
    if src_name != dest_name {
        return Err(format!(
            "目标家目录名称必须与源一致（源为 {src_name}）: {dest_name}"
        ));
    }

    let q = |p: &str| sh_quote(p);
    let src_p = Path::new(&src);
    let dest_p = Path::new(&dest);
    if !src_p.exists() {
        return Err(format!("源家目录不存在: {src}"));
    }
    if dest_p.exists() {
        // 目标已存在：仅允许空目录（数据不能覆盖），清空后继续
        let entries = dest_p
            .read_dir()
            .map_err(|e| format!("读取目标目录失败 {dest}: {e}"))?
            .next()
            .transpose()
            .map_err(|e| format!("读取目标目录失败 {dest}: {e}"))?;
        if entries.is_some() {
            return Err(format!("目标家目录已存在且非空，拒绝覆盖: {dest}"));
        }
        run_bash(&format!("rmdir {}", q(&dest)))?;
    }
    // 目标挂载点目录须已存在（挂载检查由上层/系统负责），仅创建到目标家目录
    if let Some(parent) = dest_p.parent()
        && !parent.exists()
    {
        run_bash(&format!("mkdir -p {}", q(&parent.to_string_lossy())))?;
    }

    // 优先 mv（同文件系统瞬时完成）；跨文件系统（EXDEV）自动回退 cp -a 后删除源。
    // 回退场景下属主会变为 root，随后统一由 home_init 重置归属。
    run_bash(&format!(
        "mv {} {} 2>/dev/null || (mkdir -p {} && cp -a {}/. {}/ && rm -rf {})",
        q(&src),
        q(&dest),
        q(&dest),
        q(&src),
        q(&dest),
        q(&src),
    ))?;

    // 按运行账号重置家目录骨架属主/权限（幂等），并补全 www/logs/tmp 子目录。
    // 文件搬移已成功，权限重置失败仅追加提示、不阻断迁移结果（避免数据与记录状态不一致）。
    let mut warn_msg = String::new();
    if let Err(e) = home_init_inner(&dest, owner) {
        warn_msg = format!("（权限重置提示：{e}）");
    }

    // 同步 Linux 系统账号家目录指针（账号可能不存在，失败不阻断）
    let _ = run_bash(&format!(
        "usermod -d {} {} 2>/dev/null || true",
        q(&dest),
        q(owner)
    ));

    Ok(Response::ok(
        format!("数据迁移完成：{src} → {dest}{warn_msg}"),
        Some(json!({ "src_home": src, "dest_home": dest })),
    ))
}

// ── user.quota_set（磁盘配额，best-effort）───────────────────

/// 设置 Linux 账号在其家目录所在文件系统上的块配额。
/// quota_mb = 0 → 取消配额（不限）。自动适配 xfs 与 ext 系列。
pub async fn quota_set(linux_user: &str, quota_mb: i64) -> Response {
    let linux_user = linux_user.to_string();
    tokio::task::spawn_blocking(move || quota_set_inner(&linux_user, quota_mb))
        .await
        .unwrap_or_else(|e| Ok(Response::err(-1, format!("任务执行失败: {e}"))))
        .unwrap_or_else(|e| Response::err(-1, e))
}

/// 执行 bash 并返回 stdout（trim）；失败时用 stderr/stdout 作为错误描述
fn sh_out(script: &str) -> Result<String, String> {
    let o = root_cmd("bash")
        .args(["-c", script])
        .output()
        .map_err(|e| format!("执行命令失败: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(cmd_err(&o, "命令执行失败"))
    }
}

fn quota_set_inner(linux_user: &str, quota_mb: i64) -> Result<Response, String> {
    if !linux_user_ok(linux_user) {
        return Err(format!("非法的 Linux 账号名: {linux_user}"));
    }
    let uq = sh_quote(linux_user);
    // 1) 账号家目录（passwd 第 6 字段）
    let home = sh_out(&format!("getent passwd {uq} | cut -d: -f6"))?;
    if !home.starts_with('/') || home == "/nonexistent" {
        return Err(format!(
            "账号 {linux_user} 没有有效家目录（{home}），无法设置磁盘配额"
        ));
    }
    // 2) 家目录所在挂载点
    let hq = sh_quote(&home);
    let mount = sh_out(&format!("df -P {hq} | awk 'NR==2{{print $6}}'"))?;
    if !mount.starts_with('/') {
        return Err(format!("无法解析家目录 {home} 所在挂载点（df 输出异常）"));
    }
    // 3) 文件系统类型：xfs 走 xfs_quota，其余（ext2/3/4 等）走 setquota
    let mq = sh_quote(&mount);
    let fstype = sh_out(&format!("stat -f -c %T {mq} 2>/dev/null || true")).unwrap_or_default();
    let is_xfs = fstype.eq_ignore_ascii_case("xfs");

    if is_xfs {
        if sh_out("command -v xfs_quota 2>/dev/null || true")?.is_empty() {
            return Err("未找到 xfs_quota（请安装 quota 工具包），无法设置磁盘配额".to_string());
        }
        let limit = if quota_mb <= 0 {
            "bsoft=0 bhard=0".to_string()
        } else {
            format!("bsoft={quota_mb}m bhard={quota_mb}m")
        };
        run_bash(&format!("xfs_quota -x -c 'limit {limit} {uq}' {mq}"))?;
    } else {
        if sh_out("command -v setquota 2>/dev/null || true")?.is_empty() {
            return Err("未找到 setquota（请安装 quota 工具包），无法设置磁盘配额".to_string());
        }
        // setquota 以 1KB 块为单位：软限制 = 硬限制 = quota_mb * 1024
        let kb = quota_mb.max(0) * 1024;
        run_bash(&format!("setquota -u {uq} {kb} {kb} 0 0 {mq}"))?;
    }

    let desc = if quota_mb <= 0 {
        "不限".to_string()
    } else {
        format!("{quota_mb} MB")
    };
    Ok(Response::ok(
        format!("已设置 {linux_user} 磁盘配额：{desc}（挂载点 {mount}，类型 {fstype}）"),
        Some(json!({
            "linux_user": linux_user,
            "quota_mb": quota_mb,
            "home_dir": home,
            "mount": mount,
            "fstype": fstype,
        })),
    ))
}
