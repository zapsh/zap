// SPDX-License-Identifier: AGPL-3.0-only
//! 主机交互式终端：以指定 Linux 账号运行一个 PTY 登录 shell。
//!
//! 由编辑器内嵌的 Terminal 触发（`Request::HostShellExec`）。zapexec 以 root 起进程，
//! 在 `pre_exec` 里 `setsid` + `TIOCSCTTY` + `setgroups`/`setgid`/`setuid` 降权到目标账号，
//! 因此**调用方拿到的永远只是本人的权限**，不会是 root。
//!
//! 与 `docker_exec` 一样是流式会话：stdin / resize 由主循环转发，输出逐帧回传。

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;

use crate::stream::StreamSink;

/// 会话空闲上限：终端挂着没人用不能永久占资源。
const IDLE_TIMEOUT: Duration = Duration::from_secs(3600);

struct Prepared {
    user: String,
    uid: u32,
    gid: u32,
    home: String,
    shell: String,
    cwd: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    user: String,
    cwd: String,
    cols: u16,
    rows: u16,
    mut stdin_rx: mpsc::Receiver<Vec<u8>>,
    mut resize_rx: mpsc::Receiver<(u16, u16)>,
    sink: StreamSink,
) {
    let cols = cols.clamp(20, 500);
    let rows = rows.clamp(5, 200);

    let prep = match tokio::task::spawn_blocking(move || prepare(&user, &cwd)).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            sink.error(e).await;
            return;
        }
        Err(e) => {
            sink.error(format!("解析账号失败: {e}")).await;
            return;
        }
    };

    let (master, mut child) =
        match tokio::task::spawn_blocking(move || spawn(&prep, cols, rows)).await {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                sink.error(e).await;
                return;
            }
            Err(e) => {
                sink.error(format!("启动终端失败: {e}")).await;
                return;
            }
        };

    // 把 master 设为非阻塞，才能交给 AsyncFd 用 epoll 驱动
    let fd = master.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
    #[allow(deprecated)]
    let pty = match AsyncFd::new(master) {
        Ok(a) => a,
        Err(e) => {
            let _ = child.kill();
            sink.error(format!("初始化终端失败: {e}")).await;
            return;
        }
    };

    if !sink.ready().await {
        let _ = child.kill();
        return;
    }

    let mut resize_open = true;
    loop {
        tokio::select! {
            chunk = read_chunk(&pty) => {
                match chunk {
                    Ok(data) if data.is_empty() => break,
                    Ok(data) => {
                        if !sink.out(&data).await {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            input = stdin_rx.recv() => {
                match input {
                    Some(d) => {
                        if !d.is_empty() && write_all(&pty, &d).await.is_err() {
                            break;
                        }
                    }
                    // 客户端断开（WS 关闭 / StreamClose）
                    None => break,
                }
            }
            size = resize_rx.recv(), if resize_open => {
                match size {
                    Some((c, r)) => resize(&pty, c, r),
                    None => resize_open = false,
                }
            }
            _ = tokio::time::sleep(IDLE_TIMEOUT) => break,
        }
    }

    // 收尾：杀掉 shell 并回收，拿到真实退出码
    let _ = child.kill();
    let code = match tokio::task::spawn_blocking(move || child.wait()).await {
        Ok(Ok(st)) => st.code().unwrap_or(-1),
        _ => -1,
    };
    sink.end(code).await;
}

async fn read_chunk(pty: &AsyncFd<std::fs::File>) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; 8192];
    loop {
        let mut guard = pty.readable().await?;
        match guard.try_io(|inner| {
            let fd = inner.get_ref().as_raw_fd();
            let n = unsafe {
                libc::read(
                    fd,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        }) {
            Ok(Ok(n)) => {
                buf.truncate(n);
                return Ok(buf);
            }
            Ok(Err(e)) => return Err(e),
            Err(_) => continue,
        }
    }
}

async fn write_all(pty: &AsyncFd<std::fs::File>, data: &[u8]) -> io::Result<()> {
    let mut off = 0;
    while off < data.len() {
        let mut guard = pty.writable().await?;
        let chunk = &data[off..];
        match guard.try_io(|inner| {
            let fd = inner.get_ref().as_raw_fd();
            let n = unsafe {
                libc::write(
                    fd,
                    chunk.as_ptr() as *const libc::c_void,
                    chunk.len(),
                )
            };
            if n < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        }) {
            Ok(Ok(0)) => return Ok(()),
            Ok(Ok(n)) => off += n,
            Ok(Err(e)) => return Err(e),
            Err(_) => continue,
        }
    }
    Ok(())
}

fn resize(pty: &AsyncFd<std::fs::File>, cols: u16, rows: u16) {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(pty.get_ref().as_raw_fd(), libc::TIOCSWINSZ, &ws);
    }
}

fn prepare(user: &str, requested_cwd: &str) -> Result<Prepared, String> {
    let user = user.trim();
    if user.is_empty()
        || user.len() > 64
        || !user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err("账号名不合法".to_string());
    }
    if user == "root" {
        return Err("拒绝以 root 身份启动终端".to_string());
    }

    let acc = lookup_account(user).ok_or_else(|| format!("系统账号 {user} 不存在"))?;
    if acc.uid == 0 || acc.gid == 0 {
        return Err("拒绝以 root 身份启动终端".to_string());
    }
    if acc.home.is_empty() {
        return Err("该账号没有家目录".to_string());
    }

    let home = acc.home.clone();
    let cwd = resolve_cwd(&home, requested_cwd);
    Ok(Prepared {
        user: user.to_string(),
        uid: acc.uid,
        gid: acc.gid,
        home,
        shell: resolve_shell(&acc.shell),
        cwd,
    })
}

/// 起始目录必须落在本人（或同账号共享的）家目录内；非法时回退家目录。
fn resolve_cwd(home: &str, requested: &str) -> String {
    let req = requested.trim();
    if req.is_empty() {
        return home.to_string();
    }
    let Ok(home_real) = std::fs::canonicalize(home) else {
        return home.to_string();
    };
    let Ok(real) = std::fs::canonicalize(req) else {
        return home.to_string();
    };
    if real.starts_with(&home_real) && real.is_dir() {
        real.to_string_lossy().to_string()
    } else {
        home.to_string()
    }
}

/// 账号被创建为 nologin（`useradd -s nologin`），这里换成可用的交互 shell。
fn resolve_shell(from_passwd: &str) -> String {
    let bad =
        from_passwd.is_empty() || from_passwd.ends_with("nologin") || from_passwd.ends_with("/false");
    if !bad {
        return from_passwd.to_string();
    }
    for cand in ["/bin/bash", "/usr/bin/bash", "/bin/sh"] {
        if std::path::Path::new(cand).exists() {
            return cand.to_string();
        }
    }
    "/bin/sh".to_string()
}

#[derive(Default)]
struct Account {
    uid: u32,
    gid: u32,
    home: String,
    shell: String,
}

/// `getpwnam_r` 可重入查询，避免 `getpwnam` 的静态缓冲竞争。
fn lookup_account(name: &str) -> Option<Account> {
    let cname = CString::new(name).ok()?;
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let mut buf = vec![0 as libc::c_char; 8192];
    let rc = unsafe {
        libc::getpwnam_r(
            cname.as_ptr(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    let home = unsafe { CStr::from_ptr(pwd.pw_dir) }
        .to_string_lossy()
        .to_string();
    let shell = unsafe { CStr::from_ptr(pwd.pw_shell) }
        .to_string_lossy()
        .to_string();
    Some(Account {
        uid: pwd.pw_uid,
        gid: pwd.pw_gid,
        home,
        shell,
    })
}

fn spawn(p: &Prepared, cols: u16, rows: u16) -> Result<(std::fs::File, Child), String> {
    if unsafe { libc::getuid() } != 0 {
        return Err("zapexec 未以 root 运行，无法降权启动终端".to_string());
    }
    let (master, slave) = unsafe { open_pty(cols, rows)? };
    let slave_file = unsafe { std::fs::File::from_raw_fd(slave) };

    let mut cmd = Command::new(&p.shell);
    cmd.arg("-l");
    cmd.current_dir(&p.cwd);
    cmd.env_clear();
    cmd.env("HOME", &p.home);
    cmd.env("USER", &p.user);
    cmd.env("LOGNAME", &p.user);
    cmd.env("SHELL", &p.shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env(
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    );
    cmd.stdin(Stdio::from(
        slave_file.try_clone().map_err(|e| e.to_string())?,
    ));
    cmd.stdout(Stdio::from(
        slave_file.try_clone().map_err(|e| e.to_string())?,
    ));
    cmd.stderr(Stdio::from(slave_file));

    let uid = p.uid;
    let gid = p.gid;
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            // 新会话 + 把 stdin（std 已在 pre_exec 前 dup2 成 slave）设为控制终端
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            // 顺序：清附加组 → setgid → setuid（降权后没有第二次机会）
            if libc::setgroups(0, std::ptr::null::<libc::gid_t>()) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::setgid(gid) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::setuid(uid) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn().map_err(|e| format!("启动 shell 失败: {e}"))?;
    let master_file = unsafe { std::fs::File::from_raw_fd(master) };
    Ok((master_file, child))
}

/// `posix_openpt` + `grantpt` + `unlockpt` + `ptsname_r`（不依赖 libutil）。
unsafe fn open_pty(cols: u16, rows: u16) -> Result<(libc::c_int, libc::c_int), String> {
    let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
    if master < 0 {
        return Err(format!(
            "创建 PTY 失败: {}",
            io::Error::last_os_error()
        ));
    }
    if unsafe { libc::grantpt(master) } != 0 || unsafe { libc::unlockpt(master) } != 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(master) };
        return Err(format!("初始化 PTY 失败: {err}"));
    }
    let mut name = [0 as libc::c_char; 256];
    if unsafe { libc::ptsname_r(master, name.as_mut_ptr(), name.len()) } != 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(master) };
        return Err(format!("获取 PTY 从端失败: {err}"));
    }
    let path = unsafe { CStr::from_ptr(name.as_ptr()) }
        .to_string_lossy()
        .to_string();
    let cpath = CString::new(path.as_bytes()).map_err(|e| e.to_string())?;
    let slave = unsafe { libc::open(cpath.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    if slave < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(master) };
        return Err(format!("打开 PTY 从端失败: {err}"));
    }
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe { libc::ioctl(master, libc::TIOCSWINSZ, &ws) };
    Ok((master, slave))
}
