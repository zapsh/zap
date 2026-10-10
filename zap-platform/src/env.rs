use std::fs;
use std::path::Path;

/// 支持的操作系统（仅 UNIX 系列；macOS 不在范围内）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    FreeBsd,
    OpenBsd,
    NetBsd,
}

/// 初始化系统（服务管理器）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitSystem {
    Systemd,
    SysV,
    OpenRc,
    Runit,
    BsdRc,
    Unknown,
}

/// 运行时环境探测抽象。
pub trait PlatformEnv {
    fn os(&self) -> Os;
    fn distro(&self) -> &str;
    fn distro_version(&self) -> &str;
    fn init_system(&self) -> InitSystem;
    /// 运行时目录（Linux 为 /run，部分 BSD 为 /var/run）。
    fn run_dir(&self) -> &str;
    /// 服务单元 / 脚本目录。
    fn service_dir(&self) -> &str;
    fn is_linux(&self) -> bool {
        self.os() == Os::Linux
    }
}

/// 根据已知信号归类 init 系统（纯函数，便于单测）。
pub fn classify_init(
    run_systemd: bool,
    has_initd: bool,
    has_rcd: bool,
    has_openrc: bool,
    has_runit: bool,
) -> InitSystem {
    if run_systemd {
        return InitSystem::Systemd;
    }
    if has_openrc {
        return InitSystem::OpenRc;
    }
    if has_runit {
        return InitSystem::Runit;
    }
    if has_rcd {
        return InitSystem::BsdRc;
    }
    if has_initd {
        return InitSystem::SysV;
    }
    InitSystem::Unknown
}

/// 解析 /etc/os-release 内容（纯函数，便于单测）。
pub fn parse_os_release(content: &str) -> (String, String) {
    let mut id = String::new();
    let mut version = String::new();
    for line in content.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("ID=") {
            id = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("VERSION_ID=") {
            version = v.trim_matches('"').to_string();
        }
    }
    (id, version)
}

#[derive(Debug, Clone)]
pub struct UnixEnv {
    os: Os,
    distro: String,
    distro_version: String,
    init: InitSystem,
}

impl UnixEnv {
    /// 运行期探测当前 UNIX 环境。
    pub fn detect() -> crate::Result<Self> {
        let os = if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "freebsd") {
            Os::FreeBsd
        } else if cfg!(target_os = "openbsd") {
            Os::OpenBsd
        } else if cfg!(target_os = "netbsd") {
            Os::NetBsd
        } else {
            return Err(crate::PlatformError::NotSupported(
                "Not supported (only Linux / FreeBSD / OpenBSD / NetBSD)".into(),
            ));
        };

        let (distro, distro_version) = if os == Os::Linux {
            fs::read_to_string("/etc/os-release")
                .map(|s| parse_os_release(&s))
                .unwrap_or_else(|_| (String::new(), String::new()))
        } else {
            // BSD：用 `uname -s` 的低写为 distro 名
            let out = std::process::Command::new("uname")
                .arg("-s")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            (out.trim().to_lowercase(), String::new())
        };

        let run_systemd = Path::new("/run/systemd/system").exists()
            || fs::read_to_string("/proc/1/comm")
                .map(|c| c.trim() == "systemd")
                .unwrap_or(false);
        let init = classify_init(
            run_systemd,
            Path::new("/etc/init.d").exists(),
            Path::new("/etc/rc.d").exists(),
            Path::new("/sbin/openrc").exists(),
            Path::new("/etc/runit").exists(),
        );

        Ok(UnixEnv {
            os,
            distro,
            distro_version,
            init,
        })
    }
}

impl PlatformEnv for UnixEnv {
    fn os(&self) -> Os {
        self.os
    }
    fn distro(&self) -> &str {
        &self.distro
    }
    fn distro_version(&self) -> &str {
        &self.distro_version
    }
    fn init_system(&self) -> InitSystem {
        self.init
    }
    fn run_dir(&self) -> &str {
        if self.os == Os::Linux {
            "/run"
        } else {
            "/var/run"
        }
    }
    fn service_dir(&self) -> &str {
        match self.init {
            InitSystem::Systemd => "/etc/systemd/system",
            InitSystem::SysV => "/etc/init.d",
            InitSystem::BsdRc => "/etc/rc.d",
            _ => "/etc/init.d",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_os_release_basic() {
        let s = "NAME=\"Debian\"\nID=debian\nVERSION_ID=\"12\"\n";
        assert_eq!(parse_os_release(s), ("debian".to_string(), "12".to_string()));
    }
    #[test]
    fn classify_prefers_systemd() {
        assert_eq!(
            classify_init(true, true, true, true, true),
            InitSystem::Systemd
        );
        assert_eq!(
            classify_init(false, false, true, false, false),
            InitSystem::BsdRc
        );
        assert_eq!(
            classify_init(false, true, false, false, false),
            InitSystem::SysV
        );
        assert_eq!(
            classify_init(false, false, false, false, false),
            InitSystem::Unknown
        );
    }
}
