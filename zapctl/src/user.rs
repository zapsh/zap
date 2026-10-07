// SPDX-License-Identifier: AGPL-3.0-only
//! `zapctl user` 子命令与改密。

use clap::{Subcommand, ValueEnum};
use rusqlite::{Connection, params};

use crate::{GREEN, NC, RED, ensure_root, ok};

const ADMIN_USERNAME: &str = "admin";

#[derive(Subcommand)]
pub enum UserCommand {
    /// 列出所有用户
    List,
    /// 新建用户
    Add {
        /// 用户名（唯一）
        username: String,
        /// 邮箱（唯一）
        #[arg(long)]
        email: Option<String>,
        /// 角色
        #[arg(long, value_enum, default_value_t = Role::User)]
        role: Role,
        /// 手机号（可选，默认自动生成唯一值）
        #[arg(long)]
        phone: Option<String>,
        /// 密码（可选，缺省时交互输入）
        #[arg(long)]
        password: Option<String>,
    },
    /// 启用用户
    Enable { username: String },
    /// 禁用用户
    Disable { username: String },
    /// 删除用户（禁止删除 admin）
    Delete { username: String },
    /// 重置用户密码（免旧密码，管理员运维场景）
    Passwd {
        username: String,
        /// 新密码（可选，缺省时交互输入）
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Role {
    Admin,
    User,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::User => "user",
        }
    }
}

pub fn dispatch(cmd: UserCommand, db_path: &str) -> Result<(), String> {
    match cmd {
        UserCommand::List => cmd_list(db_path),
        UserCommand::Add {
            username,
            email,
            role,
            phone,
            password,
        } => cmd_add(
            db_path,
            &username,
            email.as_deref(),
            role,
            phone.as_deref(),
            password.as_deref(),
        ),
        UserCommand::Enable { username } => set_status(db_path, &username, 1),
        UserCommand::Disable { username } => set_status(db_path, &username, 0),
        UserCommand::Delete { username } => cmd_delete(db_path, &username),
        UserCommand::Passwd { username, password } => {
            cmd_reset_passwd(db_path, &username, password.as_deref())
        }
    }
}

fn open(db_path: &str) -> Result<Connection, String> {
    Connection::open(db_path).map_err(|e| format!("无法打开数据库 {db_path}: {e}"))
}

fn cmd_list(db_path: &str) -> Result<(), String> {
    let conn = open(db_path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, username, email, nickname, status, roles, created_at FROM user ORDER BY id",
        )
        .map_err(|e| e.to_string())?;

    println!(
        "{:<4} {:<16} {:<28} {:<12} {:<8} {:<12} {:<12}",
        "ID", "USERNAME", "EMAIL", "NICKNAME", "STATUS", "ROLES", "CREATED"
    );
    println!("{}", "-".repeat(96));

    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        let (id, username, email, nickname, status, roles, created_at) =
            row.map_err(|e| e.to_string())?;
        let status_str = if status == 1 { "启用" } else { "禁用" };
        let color = if status == 1 { GREEN } else { RED };
        let created = format_time(created_at);
        println!(
            "{id:<4} {username:<16} {email:<28} {:<12} {color}{status_str:<8}{NC} {roles:<12} {created:<12}",
            nickname.unwrap_or_default()
        );
    }
    Ok(())
}

fn cmd_add(
    db_path: &str,
    username: &str,
    email: Option<&str>,
    role: Role,
    phone: Option<&str>,
    password: Option<&str>,
) -> Result<(), String> {
    ensure_root()?;

    if username.is_empty() {
        return Err("用户名不能为空".to_string());
    }
    let email = email.ok_or("请用 --email 提供邮箱")?;
    if email.is_empty() {
        return Err("邮箱不能为空".to_string());
    }

    let password = match password {
        Some(p) => p.to_string(),
        None => read_new_password()?,
    };
    validate_password(&password)?;

    let hashed =
        bcrypt::hash(&password, bcrypt::DEFAULT_COST).map_err(|e| format!("密码加密失败: {e}"))?;

    // phone 是 schema 中的 NOT NULL UNIQUE 列；未提供时生成唯一值
    let phone = match phone {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => format!(
            "u{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ),
    };

    let now = chrono::Utc::now().timestamp();
    let nickname = username.to_string();

    let conn = open(db_path)?;
    let result = conn.execute(
        "INSERT INTO user (username, password, email, phone, nickname, roles, permissions, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', 1, ?7, ?7)",
        params![username, hashed, email, phone, nickname, role.as_str(), now],
    );

    match result {
        Ok(_) => {
            ok(&format!("用户 {username} 创建成功"));
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") {
                Err("用户名、邮箱或手机号已存在".to_string())
            } else {
                Err(format!("创建失败: {msg}"))
            }
        }
    }
}

fn set_status(db_path: &str, username: &str, status: i64) -> Result<(), String> {
    ensure_root()?;
    let conn = open(db_path)?;
    let now = chrono::Utc::now().timestamp();
    let n = conn
        .execute(
            "UPDATE user SET status = ?1, updated_at = ?2 WHERE username = ?3",
            params![status, now, username],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err(format!("用户 {username} 不存在"));
    }
    let verb = if status == 1 { "启用" } else { "禁用" };
    ok(&format!("已{verb}用户 {username}"));
    Ok(())
}

fn cmd_delete(db_path: &str, username: &str) -> Result<(), String> {
    ensure_root()?;
    if username == ADMIN_USERNAME {
        return Err("禁止删除内置 admin 用户".to_string());
    }
    let conn = open(db_path)?;
    let n = conn
        .execute("DELETE FROM user WHERE username = ?1", params![username])
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err(format!("用户 {username} 不存在"));
    }
    ok(&format!("已删除用户 {username}"));
    Ok(())
}

/// 管理员重置密码（免旧密码）。
fn cmd_reset_passwd(db_path: &str, username: &str, password: Option<&str>) -> Result<(), String> {
    ensure_root()?;
    let new_password = match password {
        Some(p) => p.to_string(),
        None => read_new_password()?,
    };
    validate_password(&new_password)?;
    let hashed = bcrypt::hash(&new_password, bcrypt::DEFAULT_COST)
        .map_err(|e| format!("密码加密失败: {e}"))?;

    let conn = open(db_path)?;
    let now = chrono::Utc::now().timestamp();
    let n = conn
        .execute(
            "UPDATE user SET password = ?1, updated_at = ?2 WHERE username = ?3",
            params![hashed, now, username],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err(format!("用户 {username} 不存在"));
    }
    ok(&format!("用户 {username} 密码已重置"));
    Ok(())
}

/// 自助改密（校验旧密码）。
pub fn cmd_self_passwd(db_path: &str, username: &str) -> Result<(), String> {
    ensure_root()?;

    let conn = open(db_path)?;
    let stored: Option<String> = conn
        .query_row(
            "SELECT password FROM user WHERE username = ?1",
            params![username],
            |r| r.get(0),
        )
        .ok();
    let stored = stored.ok_or_else(|| format!("用户 {username} 不存在"))?;

    let old_password =
        rpassword::prompt_password("旧密码: ").map_err(|e| format!("读取旧密码失败: {e}"))?;
    if !bcrypt::verify(&old_password, &stored).unwrap_or(false) {
        return Err("旧密码错误".to_string());
    }

    let new_password = read_new_password()?;
    validate_password(&new_password)?;
    if new_password == old_password {
        return Err("新密码不能与旧密码相同".to_string());
    }

    let hashed = bcrypt::hash(&new_password, bcrypt::DEFAULT_COST)
        .map_err(|e| format!("密码加密失败: {e}"))?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE user SET password = ?1, updated_at = ?2 WHERE username = ?3",
        params![hashed, now, username],
    )
    .map_err(|e| e.to_string())?;

    ok(&format!("用户 {username} 密码已修改"));
    Ok(())
}

fn validate_password(pw: &str) -> Result<(), String> {
    if pw.len() < 6 {
        return Err("密码长度不能少于 6 位".to_string());
    }
    Ok(())
}

fn read_new_password() -> Result<String, String> {
    let p1 = rpassword::prompt_password("新密码: ").map_err(|e| format!("读取密码失败: {e}"))?;
    let p2 =
        rpassword::prompt_password("确认新密码: ").map_err(|e| format!("读取密码失败: {e}"))?;
    if p1 != p2 {
        return Err("两次输入的密码不一致".to_string());
    }
    if p1.is_empty() {
        return Err("密码不能为空".to_string());
    }
    Ok(p1)
}

fn format_time(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts.to_string())
}
