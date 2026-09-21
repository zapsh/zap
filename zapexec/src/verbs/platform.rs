//! 平台差异层：把 OS 之间不一致的系统管理命令收敛到一处。
//!
//! 目标平台横跨 Debian/RHEL/Alpine 系 Linux 与 OpenBSD，命令名、参数语义、
//! 甚至「有没有这个命令」都不一样。动词层只调用这里的**语义化**接口，
//! 不再直接拼命令——否则移植时要在几十个动词里挨个翻。
//!
//! | 能力           | Linux                    | OpenBSD                          |
//! |----------------|--------------------------|----------------------------------|
//! | 脚本解释器     | `bash`                   | `/bin/sh`（系统默认没有 bash）    |
//! | 查询组         | 解析 `/etc/group`        | 同（不用 `getent`，那是 glibc 专有）|
//! | 建组 / 删组    | `groupadd` / `groupdel`  | 同名命令，参数一致                |
//! | 建账号         | `useradd -M -s -d -g`    | `useradd -s -d -g`（无 `-M`）     |
//! | 删账号         | `userdel`                | 同名命令                          |
//! | nologin 兜底   | `/usr/sbin/nologin`      | `/sbin/nologin`                   |
//!
//! 刻意用「解析 `/etc/passwd` / `/etc/group`」替代 `getent` / `id`：前者是 POSIX
//! 文件、任何平台都在，后者是 glibc 工具集，OpenBSD 没有。代价是看不到 LDAP/NSS
//! 后端——面板管理的账号恒为本地账号，不受影响。

use super::root_cmd;

/// 执行脚本片段的解释器。
///
/// OpenBSD 默认不装 bash（系统 shell 是 ksh），所以用 `/bin/sh`。这意味着
/// **传进来的脚本必须保持 POSIX 兼容**，不能依赖 `[[ ]]`、`pipefail`、
/// `mapfile` 这类 bashism。
#[cfg(target_os = "linux")]
pub(crate) const SHELL: &str = "bash";

/// 同 [`SHELL`]，非 Linux 平台。
#[cfg(not(target_os = "linux"))]
pub(crate) const SHELL: &str = "/bin/sh";

/// `nologin` 的兜底路径（`command -v nologin` 找不到时用）。
#[cfg(target_os = "linux")]
pub(crate) const NOLOGIN_FALLBACK: &str = "/usr/sbin/nologin";

/// 同 [`NOLOGIN_FALLBACK`]，OpenBSD —— 它的 nologin 在 /sbin 下。
#[cfg(target_os = "openbsd")]
pub(crate) const NOLOGIN_FALLBACK: &str = "/sbin/nologin";

/// 同 [`NOLOGIN_FALLBACK`]，其余平台（FreeBSD、macOS 等）沿用 /usr/sbin。
#[cfg(not(any(target_os = "linux", target_os = "openbsd")))]
pub(crate) const NOLOGIN_FALLBACK: &str = "/usr/sbin/nologin";

/// `/etc/group` 中的一条记录。
pub(crate) struct GroupEntry {
    pub gid: u32,
    /// 附加成员（不含主组为该组的用户）。
    pub members: Vec<String>,
}

/// 按组名查 `/etc/group`。
pub(crate) fn group_entry(name: &str) -> Option<GroupEntry> {
    let content = std::fs::read_to_string("/etc/group").ok()?;
    for line in content.lines() {
        let mut f = line.split(':');
        match (f.next(), f.next(), f.next(), f.next()) {
            (Some(gname), _, Some(raw_gid), Some(raw_members)) if gname == name => {
                return Some(GroupEntry {
                    gid: raw_gid.parse().ok()?,
                    members: raw_members
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect(),
                });
            }
            _ => continue,
        }
    }
    None
}

/// 按 gid 反查组名。
pub(crate) fn group_name_of_gid(gid: u32) -> Option<String> {
    let content = std::fs::read_to_string("/etc/group").ok()?;
    for line in content.lines() {
        let mut f = line.split(':');
        if let (Some(gname), _, Some(raw_gid)) = (f.next(), f.next(), f.next())
            && raw_gid.parse::<u32>().ok() == Some(gid)
        {
            return Some(gname.to_string());
        }
    }
    None
}

/// 账号是否存在。
pub(crate) fn user_exists(user: &str) -> bool {
    let name = match std::ffi::CString::new(user) {
        Ok(n) => n,
        Err(_) => return false,
    };
    unsafe {
        let buflen = libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX).max(4096) as usize;
        let mut buf: Vec<libc::c_char> = vec![0; buflen];
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        libc::getpwnam_r(
            name.as_ptr(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut out,
        ) == 0
            && !out.is_null()
    }
}

/// 创建组（已存在时返回 Err，由调用方决定语义）。
pub(crate) fn create_group(name: &str) -> Result<(), String> {
    run_checked("groupadd", &[name])
}

/// 删除组（best-effort：组仍被引用时系统会拒绝，忽略即可）。
pub(crate) fn delete_group(name: &str) {
    let _ = root_cmd("groupdel").arg(name).output();
}

/// 创建账号。家目录**不由这里创建**：目录由 `user.home_init` 建好并赋权。
pub(crate) fn create_user(
    user: &str,
    home: &str,
    login_shell: &str,
    group: &str,
) -> Result<(), String> {
    // Linux 的 `-M` 表示不自动建家目录；OpenBSD 的 useradd 没有这个选项
    // （默认也不建），所以只在该平台上省略。
    #[cfg(target_os = "linux")]
    let args: Vec<&str> = vec!["-M", "-s", login_shell, "-d", home, "-g", group, user];
    #[cfg(not(target_os = "linux"))]
    let args: Vec<&str> = vec!["-s", login_shell, "-d", home, "-g", group, user];
    run_checked("useradd", &args)
}

/// 删除账号（不删家目录，家目录由站点清理流程单独处理）。
pub(crate) fn delete_user(user: &str) -> Result<(), String> {
    run_checked("userdel", &[user])
}

/// 执行管理命令，非零退出时把 stderr/stdout 作为错误描述返回。
fn run_checked(program: &str, args: &[&str]) -> Result<(), String> {
    let o = root_cmd(program)
        .args(args)
        .output()
        .map_err(|e| format!("执行 {program} 失败: {e}"))?;
    if o.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&o.stderr).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&o.stdout).trim().to_string();
    }
    if text.is_empty() {
        text = format!("{program} 返回非零");
    }
    Err(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gid 0 在任何平台上都存在（Linux 是 `root`，OpenBSD 是 `wheel`），
    /// 所以只做「反查 → 正查」的往返校验，不写死组名。
    #[test]
    fn group_lookup_roundtrip() {
        let name = group_name_of_gid(0).expect("gid 0 的组应当存在");
        let entry = group_entry(&name).expect("按名字应能查回同一条记录");
        assert_eq!(entry.gid, 0);
    }

    #[test]
    fn unknown_group_is_none() {
        assert!(group_entry("zap-no-such-group-xyz").is_none());
    }
}
