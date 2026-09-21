//! 第三方脚本的资源边界。
//!
//! 目标只有一个：**一个建站脚本不该有能力拖垮整机**。
//!
//! 这一层是跨平台差异最大的地方，能力按平台自适应，且**一律尽力而为**——
//! 拿不到某一档不算失败，任务成败只由退出码决定（见 `run-<id>.ret`）。
//!
//! | 档位             | Linux | 说明                                    |
//! |------------------|-------|-----------------------------------------|
//! | rlimit           | ✅    | POSIX，真正的基线，本文件主体            |
//! | cgroup v2 slice  | 可选  | 需部署 `zap-task.slice`，没部署就自动跳过 |
//!
//! 刻意避开的两种做法：
//!
//! - **不用 `RLIMIT_AS` 限内存**：Python 解释器会大块 mmap（模块加载 + 线程栈
//!   预留），虚拟地址空间用量与真实内存不成比例，设小了必然误伤。
//! - **`memory.max` 默认不开**：超限是 OOM kill，表现为「任务莫名失败、日志里
//!   毫无异常」。在能按包声明内存之前，误伤风险大于收益。

use std::ffi::CString;
use std::path::{Path, PathBuf};

use tracing::{debug, warn};

/// zap 任务专用 slice 的 cgroup v2 路径。
///
/// 是否启用**完全由部署决定**：只有 `install.sh` 部署了 `zap-task.slice` 并且
/// 内核是 unified hierarchy（v2）时这个目录才存在。没部署的机器（以及所有
/// 非 systemd 发行版、容器）自动跳过，代码里不需要额外开关，也绝不会
/// 去根 cgroup 下乱建目录污染宿主机。
const CGROUP_SLICE: &str = "/sys/fs/cgroup/zap-task.slice";

/// 单个任务允许的进程/线程数（防 fork 炸弹）。
///
/// 注意 Linux 的 `RLIMIT_NPROC` 是按 real uid 计数的：同一站点账号的 php-fpm
/// 也占名额，所以给得比直觉宽松——fork 炸弹几秒内就能撞到几千，1024 足够挡住，
/// 又不会误伤正常站点。
const MAX_PROCS: libc::rlim_t = 1024;

/// 单进程 fd 上限。
const MAX_NOFILE: libc::rlim_t = 1024;

/// 单个文件大小上限（2 GiB）：挡住「下载/日志把盘写满」。
///
/// 只对降权脚本生效。2 GiB 对 WordPress 这类包（下载 + 解压不足 200 MB）绰绰
/// 有余，也给备份恢复留了余量；再小就有误伤风险。
const MAX_FSIZE: libc::rlim_t = 2 * 1024 * 1024 * 1024;

/// CPU 时间上限（秒）。
///
/// 注意是**累计 CPU 秒**而非墙钟：下载是 IO 密集，几乎不消耗 CPU 时间，所以这个
/// 值非常宽松，只作为死循环的兜底。墙钟超时由 `wait_with_timeout` 负责，
/// 两者互补——墙钟超时只在 zapexec 存活时有效，rlimit 连孤儿进程也能兜住。
const MAX_CPU_SECS: libc::rlim_t = 3600;

/// 单次任务的资源边界句柄。
///
/// 生命周期分三段，是因为 `pre_exec` 里不能做分配：
/// `prepare`（spawn 前，可以随意 IO）→ `enter`（`pre_exec` 内，只走 syscall）
/// → `finish`（任务结束后回收）。
pub(crate) struct TaskResource {
    cgroup: Option<CgroupLeaf>,
}

/// cgroup v2 叶子目录（Linux 可选档）。
struct CgroupLeaf {
    /// `{leaf}/cgroup.procs`，预先 CString 化，供 `pre_exec` 内零分配写入。
    procs: CString,
    leaf: PathBuf,
}

impl TaskResource {
    /// 为一次任务准备资源边界。任何失败都降级为「仅 rlimit」，绝不阻断任务。
    pub(crate) fn prepare(run_id: &str) -> Self {
        Self {
            cgroup: CgroupLeaf::prepare(run_id),
        }
    }

    /// 降权脚本进入资源笼子：**在 `pre_exec` 内、降权之后调用**。
    ///
    /// 这一档只对第三方脚本（`run_as: user`）生效，官方 root 脚本要编译安装，
    /// 不能套这些限制。
    pub(crate) fn enter(&self) {
        no_new_privs();
        apply_rlimits();
        // 固定 umask：脚本创建的文件不该对同组可写，避免站点之间互相改写。
        unsafe {
            libc::umask(0o022);
        }
        if let Some(cg) = &self.cgroup {
            cg.join();
        }
    }

    /// 回收资源；cgroup 目录还在（非空）说明有子进程残留，会打日志提示。
    pub(crate) fn finish(&self) {
        if let Some(cg) = &self.cgroup {
            cg.remove();
        }
    }
}

/// 应用 rlimit 边界（POSIX 通用档）。
///
/// 在 `pre_exec` 内调用，只用 `setrlimit`，不做分配。
fn apply_rlimits() {
    // 用宏而不是函数：`setrlimit` 第一个参数的类型在不同实现上未必一致
    //（Linux 是 `__rlimit_resource_t`），写死类型会编译不过，交给宏推导最稳。
    macro_rules! limit {
        ($res:expr, $val:expr) => {{
            let rl = libc::rlimit {
                rlim_cur: $val,
                rlim_max: $val,
            };
            libc::setrlimit($res, &rl);
        }};
    }
    unsafe {
        // core dump 里可能躺着数据库密码
        limit!(libc::RLIMIT_CORE, 0);
        limit!(libc::RLIMIT_NPROC, MAX_PROCS);
        limit!(libc::RLIMIT_NOFILE, MAX_NOFILE);
        limit!(libc::RLIMIT_FSIZE, MAX_FSIZE);
        limit!(libc::RLIMIT_CPU, MAX_CPU_SECS);
    }
}

/// 禁止子进程借 setuid 程序再提权。
///
/// 走 Linux 的 `PR_SET_NO_NEW_PRIVS`（prctl）；非 Linux 平台没有等价机制，退化成
/// 空实现。缺这一层不致命：脚本本来就已经降到无登录能力的站点账号，且附加组已清空。
#[cfg(target_os = "linux")]
fn no_new_privs() {
    unsafe {
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    }
}

#[cfg(not(target_os = "linux"))]
fn no_new_privs() {}

impl CgroupLeaf {
    /// 创建本次任务的叶子 cgroup 并写入限额；不可用或失败时返回 `None`。
    fn prepare(run_id: &str) -> Option<Self> {
        let slice = Path::new(CGROUP_SLICE);
        // cgroup.controllers 存在 = unified hierarchy（v2）且 slice 已部署
        if !slice.join("cgroup.controllers").exists() {
            debug!("{CGROUP_SLICE} 未部署，本次任务仅受 rlimit 约束");
            return None;
        }

        let leaf = slice.join(format!("run-{run_id}"));
        // 上一次崩溃可能留下同名目录；空目录能删掉则复用，删不掉（还有进程）也
        // 继续用——新进程会和残留进程共享同一份限额，不会更糟。
        let _ = std::fs::remove_dir(&leaf);
        if let Err(e) = std::fs::create_dir_all(&leaf) {
            warn!("创建任务 cgroup 失败，降级为仅 rlimit: {e}");
            return None;
        }

        // 只设进程数与 CPU 配额，不设 memory.max（见模块文档）。
        // 写失败不致命：控制器没启用时这些文件不存在，跳过即可。
        write_ctrl(&leaf, "pids.max", "1024");
        // 4 核上限：核数少于 4 的机器上等于不限，正好符合「只削峰、不误伤」。
        write_ctrl(&leaf, "cpu.max", "400% 100");

        let procs = leaf.join("cgroup.procs");
        let procs = CString::new(procs.to_string_lossy().as_bytes()).ok()?;
        Some(Self { procs, leaf })
    }

    /// 把当前进程放进叶子 cgroup（在 `pre_exec` 内调用）。
    ///
    /// 只用 `open` / `write` / `close`，pid 手工转十进制写进栈缓冲，全程零分配，
    /// 符合 `pre_exec` 的 async-signal-safe 约束。子进程会自动继承，所以整棵
    /// 脚本进程树都在笼子里。
    fn join(&self) {
        unsafe {
            let fd = libc::open(self.procs.as_ptr(), libc::O_WRONLY);
            if fd < 0 {
                return;
            }
            let mut buf = [0u8; 16];
            let mut n = libc::getpid() as u64; // pid 恒为正
            let mut i = buf.len();
            loop {
                i -= 1;
                buf[i] = b'0' + (n % 10) as u8;
                n /= 10;
                if n == 0 {
                    break;
                }
            }
            libc::write(fd, buf[i..].as_ptr().cast(), buf.len() - i);
            libc::close(fd);
        }
    }

    /// 回收叶子目录。cgroup v2 的空目录可以 rmdir；删不掉说明里面还有进程残留。
    fn remove(&self) {
        if let Err(e) = std::fs::remove_dir(&self.leaf) {
            warn!(
                "回收任务 cgroup 失败 {}: {e}（可能有子进程残留）",
                self.leaf.display()
            );
        }
    }
}

fn write_ctrl(leaf: &Path, name: &str, val: &str) {
    if let Err(e) = std::fs::write(leaf.join(name), val) {
        debug!("设置 cgroup {name} 失败（跳过）: {e}");
    }
}
