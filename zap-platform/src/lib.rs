//! ZAP 跨 UNIX 平台抽象层。
//!
//! 仅覆盖 Linux 与 BSD 系列（macOS / Windows 不在范围内）。提供四个子系统的 trait 抽象
//! 与 UNIX 实现：运行时环境探测（[env]）、文件系统（[fs]）、用户与权限（[user]）、
//! 服务管理（[service]），外加 Linux capabilities 读取（[caps]）。
//!
//! 通过 [Platform::detect] 取得当前平台的统一门面，调用方只依赖 trait 对象，不感知具体 OS。
//!
//! 本 crate 是「先把抽象抽出来」的第一步：已实现 Linux / BSD 后端，但**尚未接入任何现有
//! crate**（zapd / zapexec 等仍沿用各自内联实现，不受本 crate 影响）。

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
)))]
compile_error!("zap-platform 仅支持 Linux 与 BSD 系列（macOS / Windows 暂不支持）");

pub mod caps;
pub mod env;
pub mod error;
pub mod fs;
pub mod service;
pub mod user;

pub use error::{PlatformError, Result};

use env::PlatformEnv;
use fs::FileSystem;
use service::ServiceManager;
use user::UserManager;

/// 当前 UNIX 平台的统一门面：调用方只依赖其中四个 trait 对象，不感知具体 OS。
pub struct Platform {
    pub env: Box<dyn PlatformEnv>,
    pub fs: Box<dyn FileSystem>,
    pub user: Box<dyn UserManager>,
    pub service: Box<dyn ServiceManager>,
}

impl Platform {
    /// 探测并构造当前平台的实现（Linux / BSD）。
    pub fn detect() -> Result<Self> {
        let env = env::UnixEnv::detect()?;
        Ok(Platform {
            env: Box::new(env.clone()),
            fs: Box::new(fs::UnixFs),
            user: Box::new(user::UnixUser),
            service: Box::new(service::UnixService::new(env.init_system())),
        })
    }
}
