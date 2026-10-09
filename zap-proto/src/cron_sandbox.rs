// SPDX-License-Identifier: AGPL-3.0-only
//! 用户级定时任务（crontab）命令沙箱校验。
//!
//! 同时被 zapexec（真正执行处，权威）与 zapd（保存前预校验）调用，保证两处规则一致。
//!
//! 设计目标（用户级定时任务「沙箱限制」）：
//! - 保留任意命令的自由度，但把**文件访问收敛到用户家目录内**；
//! - 禁止提权指令（sudo / su / pkexec / doas / runuser / sudoedit / fakeroot / gosu）；
//! - 收紧执行环境（PATH 不含 sbin、cwd/home 收敛）由调用方负责，本模块只做命令文本校验。
//!
//! 规则：
//! 1. 提权令牌：命令里出现上述词作为独立词即拒绝（避免误伤路径里的同名片段）。
//! 2. 重定向目标（`>` / `>>` 后的文件名）若为绝对路径，必须落在 home 内（/dev 设备除外）。
//! 3. 命令中出现的绝对路径（含脚本路径）必须落在 home 内，或属于白名单系统二进制目录
//!    （/usr/bin、/bin、/usr/local/bin），或 /dev；`~/...` 视为 home 内；含 `..` 一律拒绝。
//!
//! 已知局限：基于文本扫描，无法对抗 `$(...)` / 变量拼接等刻意绕过；真正的兜底仍是
//! 「以非特权 linux_user 运行 + cwd/home 收敛」，本模块只拦常见误用与明文越权。

/// 禁止的提权指令（作为独立词出现即拒绝）。
const PRIV_TOKENS: &[&str] = &[
    "sudo", "su", "pkexec", "doas", "runuser", "sudoedit", "fakeroot", "gosu",
];

/// 允许引用的系统二进制目录（命令里以绝对路径调用这些目录下的程序不算越权）。
const ALLOWED_BIN_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/local/bin"];

/// 命令分词时的分隔符（shell 元字符与空白）。
const SEPARATORS: &[char] = &[
    ' ', '\t', '\n', '\r', '|', '&', ';', '(', ')', '<', '>', '\'', '"', '{', '}', '$', '*', '?',
    '[', ']', '!', '\\', '`',
];

fn is_sep(c: char) -> bool {
    SEPARATORS.contains(&c)
}

/// 把命令拆成词（按分隔符），并记录每个词是否紧跟在重定向符 `>` 之后。
struct Token {
    text: String,
    after_redirect: bool,
}

fn tokenize(cmd: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut after_redirect = false;
    let mut pending_redirect = false;
    for c in cmd.chars() {
        if is_sep(c) {
            if !cur.is_empty() {
                out.push(Token {
                    text: std::mem::take(&mut cur),
                    after_redirect,
                });
                after_redirect = false;
            }
            // `>` 既是分隔符，也是重定向标记：下一个词标记为重定向目标。
            // `>>` 由两个 `>` 组成，pending_redirect 连续置位即可。
            if c == '>' {
                pending_redirect = true;
            }
            continue;
        }
        if pending_redirect {
            // 上一个分隔符是 `>`，且当前开始一个新词：这个词是重定向目标。
            after_redirect = true;
            pending_redirect = false;
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(Token {
            text: cur,
            after_redirect,
        });
    }
    out
}

/// 判断 `path` 是否落在 `home` 之内（home 为空时恒为 false，fail-closed）。
fn under_home(path: &str, home: &str) -> bool {
    if home.is_empty() {
        return false;
    }
    let home = home.trim_end_matches('/');
    path == home || path.starts_with(&format!("{home}/"))
}

/// 路径是否允许：home 内 / 白名单 bin 目录内 / /dev 设备。
fn path_allowed(path: &str, home: &str, is_redirect: bool) -> bool {
    let p = path.trim_matches(|c| c == '\'' || c == '"');
    if p.is_empty() {
        return true;
    }
    // 任何 `..` 段都拒绝（即便拼在家目录里也会逃出）
    if p.contains("/../") || p.ends_with("/..") || p == ".." || p.starts_with("../") {
        return false;
    }
    // `~/...` 视为家目录内
    let p = if let Some(rest) = p.strip_prefix("~/") {
        format!("{}/{}", home.trim_end_matches('/'), rest)
    } else {
        p.to_string()
    };
    // 设备文件（含 /dev/null）总是安全的
    if p.starts_with("/dev/") {
        return true;
    }
    if under_home(&p, home) {
        return true;
    }
    if is_redirect {
        // 重定向只允许写回家目录（/dev 已在上方放行）
        return false;
    }
    for d in ALLOWED_BIN_DIRS {
        if p == *d || p.starts_with(&format!("{d}/")) {
            return true;
        }
    }
    false
}

/// 校验命令是否满足沙箱约束。不满足时返回人类可读的中文错误。
pub fn check(command: &str, home: &str) -> Result<(), String> {
    let cmd = command.trim();
    if cmd.is_empty() {
        return Err("执行内容不能为空".into());
    }

    for t in tokenize(cmd) {
        let word = t.text.trim_matches(|c| c == '\'' || c == '"');
        if word.is_empty() {
            continue;
        }
        // 1. 提权令牌
        if PRIV_TOKENS.contains(&word) {
            return Err(format!(
                "命令包含禁止的提权指令: {word}（定时任务以非特权用户运行，不允许提权）"
            ));
        }
        // 2. 重定向目标：绝对路径必须落在 home 内
        if t.after_redirect {
            if word.starts_with('/') && !path_allowed(word, home, true) {
                return Err(format!(
                    "命令禁止重定向到系统路径: {word}（定时任务仅允许写入家目录内）"
                ));
            }
            continue;
        }
        // 3. 普通参数中的绝对路径（含脚本路径）
        if word.starts_with('/') && !path_allowed(word, home, false) {
            return Err(format!(
                "命令引用了家目录之外的路径: {word}（定时任务仅允许访问家目录内的文件）"
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/alice";

    #[test]
    fn allows_relative_and_home_paths() {
        assert!(check("ls -la", HOME).is_ok());
        assert!(check("cat ~/notes.txt", HOME).is_ok());
        assert!(check("tar czf backup.tgz data", HOME).is_ok());
        assert!(check("cat /home/alice/x.txt", HOME).is_ok());
        assert!(check("mysqldump -u root db > dump.sql", HOME).is_ok());
    }

    #[test]
    fn blocks_privilege_escalation() {
        for c in [
            "sudo ls",
            "su -",
            "pkexec x",
            "doas y",
            "runuser -u root",
            "sudoedit f",
        ] {
            assert!(check(c, HOME).is_err(), "应拒绝: {c}");
        }
        // 路径里恰巧含同名片段不应误伤（在自家目录内）
        assert!(check("cat /home/alice/su/x.txt", HOME).is_ok());
        // 但其它用户的家目录仍然越权，正常拦截
        assert!(check("cat /home/su/x.txt", HOME).is_err());
        assert!(check("echo nosudo", HOME).is_ok());
    }

    #[test]
    fn blocks_paths_outside_home() {
        assert!(check("cat /etc/passwd", HOME).is_err());
        assert!(check("ls /", HOME).is_err());
        assert!(check("cat /home/bob/x", HOME).is_err());
        assert!(check("cat /home/alice/../bob/x", HOME).is_err());
    }

    #[test]
    fn blocks_redirect_outside_home() {
        assert!(check("echo hi > /tmp/x", HOME).is_err());
        assert!(check("echo hi > /etc/x", HOME).is_err());
        assert!(check("echo hi > out.txt", HOME).is_ok());
        assert!(check("echo hi > /home/alice/out.txt", HOME).is_ok());
        assert!(check("echo hi > /dev/null", HOME).is_ok());
    }

    #[test]
    fn allows_system_binaries() {
        assert!(check("cat /usr/bin/rsync", HOME).is_ok());
        assert!(check("/bin/ls -l", HOME).is_ok());
    }
}
