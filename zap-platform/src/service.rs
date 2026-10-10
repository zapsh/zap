use std::process::Command;

use crate::env::InitSystem;
use crate::{PlatformError, Result};

#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub active: bool,
    pub enabled: bool,
    pub raw: String,
}

/// 服务管理抽象（跨 UNIX）。
pub trait ServiceManager {
    fn start(&self, name: &str) -> Result<()>;
    fn stop(&self, name: &str) -> Result<()>;
    fn restart(&self, name: &str) -> Result<()>;
    fn reload(&self, name: &str) -> Result<()>;
    fn enable(&self, name: &str) -> Result<()>;
    fn disable(&self, name: &str) -> Result<()>;
    fn status(&self, name: &str) -> Result<ServiceStatus>;
    fn is_enabled(&self, name: &str) -> Result<bool>;
}

// ── 纯函数：命令构建（便于单测，不真正执行） ──
pub fn systemd_args(action: &str, name: &str) -> Vec<String> {
    vec!["systemctl".to_string(), action.to_string(), name.to_string()]
}
pub fn sv_args(action: &str, name: &str) -> Vec<String> {
    vec!["sv".to_string(), action.to_string(), name.to_string()]
}
pub fn script_args(script: &str, action: &str) -> Vec<String> {
    vec![script.to_string(), action.to_string()]
}
/// 解析 BSD rc.conf 中 `<name>_enable="YES"`（纯函数，忽略注释与引号）。
pub fn bsd_rc_enabled(rc_conf: &str, name: &str) -> bool {
    let key = format!("{}_enable", name);
    for line in rc_conf.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some(v) = line.strip_prefix(&key) {
            if let Some(v) = v.strip_prefix('=') {
                let v = v.trim().trim_matches('"').trim_matches('\'');
                return v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true");
            }
        }
    }
    false
}

pub struct UnixService {
    init: InitSystem,
}

impl UnixService {
    pub fn new(init: InitSystem) -> Self {
        Self { init }
    }

    fn run(&self, args: &[String]) -> Result<()> {
        let (prog, rest) = args
            .split_first()
            .ok_or_else(|| PlatformError::Other("空命令".into()))?;
        let status = Command::new(prog).args(rest).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(PlatformError::Other(format!(
                "命令失败（{}）：退出码 {:?}",
                args.join(" "),
                status.code()
            )))
        }
    }

    fn is_active_systemd(&self, name: &str) -> bool {
        Command::new("systemctl")
            .args(["is-active", name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    fn is_enabled_systemd(&self, name: &str) -> bool {
        Command::new("systemctl")
            .args(["is-enabled", name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

impl ServiceManager for UnixService {
    fn start(&self, name: &str) -> Result<()> {
        match self.init {
            InitSystem::Systemd => self.run(&systemd_args("start", name)),
            InitSystem::Runit => self.run(&sv_args("start", name)),
            InitSystem::SysV | InitSystem::OpenRc => {
                self.run(&script_args(&format!("/etc/init.d/{}", name), "start"))
            }
            InitSystem::BsdRc => self.run(&script_args(&format!("/etc/rc.d/{}", name), "start")),
            _ => Err(PlatformError::NotSupported(format!("start 未实现：{:?}", self.init))),
        }
    }
    fn stop(&self, name: &str) -> Result<()> {
        match self.init {
            InitSystem::Systemd => self.run(&systemd_args("stop", name)),
            InitSystem::Runit => self.run(&sv_args("stop", name)),
            InitSystem::SysV | InitSystem::OpenRc => {
                self.run(&script_args(&format!("/etc/init.d/{}", name), "stop"))
            }
            InitSystem::BsdRc => self.run(&script_args(&format!("/etc/rc.d/{}", name), "stop")),
            _ => Err(PlatformError::NotSupported(format!("stop 未实现：{:?}", self.init))),
        }
    }
    fn restart(&self, name: &str) -> Result<()> {
        match self.init {
            InitSystem::Systemd => self.run(&systemd_args("restart", name)),
            InitSystem::Runit => self.run(&sv_args("restart", name)),
            InitSystem::SysV | InitSystem::OpenRc => {
                self.run(&script_args(&format!("/etc/init.d/{}", name), "restart"))
            }
            InitSystem::BsdRc => self.run(&script_args(&format!("/etc/rc.d/{}", name), "restart")),
            _ => Err(PlatformError::NotSupported(format!("restart 未实现：{:?}", self.init))),
        }
    }
    fn reload(&self, name: &str) -> Result<()> {
        match self.init {
            InitSystem::Systemd => self.run(&systemd_args("reload", name)),
            InitSystem::Runit => self.run(&sv_args("reload", name)),
            InitSystem::SysV | InitSystem::OpenRc => {
                self.run(&script_args(&format!("/etc/init.d/{}", name), "reload"))
            }
            InitSystem::BsdRc => self.run(&script_args(&format!("/etc/rc.d/{}", name), "reload")),
            _ => Err(PlatformError::NotSupported(format!("reload 未实现：{:?}", self.init))),
        }
    }
    fn enable(&self, name: &str) -> Result<()> {
        match self.init {
            InitSystem::Systemd => self.run(&systemd_args("enable", name)),
            InitSystem::BsdRc => Err(PlatformError::NotSupported(
                "BSD 需手动在 rc.conf 设置 `<name>_enable=YES`".into(),
            )),
            _ => Err(PlatformError::NotSupported(format!("enable 未实现：{:?}", self.init))),
        }
    }
    fn disable(&self, name: &str) -> Result<()> {
        match self.init {
            InitSystem::Systemd => self.run(&systemd_args("disable", name)),
            InitSystem::BsdRc => Err(PlatformError::NotSupported(
                "BSD 需手动在 rc.conf 注释 `<name>_enable`".into(),
            )),
            _ => Err(PlatformError::NotSupported(format!("disable 未实现：{:?}", self.init))),
        }
    }
    fn status(&self, name: &str) -> Result<ServiceStatus> {
        let (active, raw) = match self.init {
            InitSystem::Systemd => (self.is_active_systemd(name), String::new()),
            _ => {
                let script = match self.init {
                    InitSystem::BsdRc => format!("/etc/rc.d/{}", name),
                    InitSystem::Runit => format!("/etc/sv/{}/run", name),
                    _ => format!("/etc/init.d/{}", name),
                };
                match Command::new(&script).arg("status").output() {
                    Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned()),
                    Err(_) => (false, String::new()),
                }
            }
        };
        let enabled = self.is_enabled(name)?;
        Ok(ServiceStatus { active, enabled, raw })
    }
    fn is_enabled(&self, name: &str) -> Result<bool> {
        match self.init {
            InitSystem::Systemd => Ok(self.is_enabled_systemd(name)),
            InitSystem::BsdRc => {
                let rc = std::fs::read_to_string("/etc/rc.conf")
                    .or_else(|_| std::fs::read_to_string("/etc/rc.conf.local"))
                    .unwrap_or_default();
                Ok(bsd_rc_enabled(&rc, name))
            }
            _ => Err(PlatformError::NotSupported(format!("is_enabled 未实现：{:?}", self.init))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn systemd_cmd() {
        assert_eq!(systemd_args("start", "nginx"), vec!["systemctl", "start", "nginx"]);
    }
    #[test]
    fn bsd_rc_parse() {
        let conf = "nginx_enable=\"YES\"\n# sshd_enable=\"NO\"\n";
        assert!(bsd_rc_enabled(conf, "nginx"));
        assert!(!bsd_rc_enabled(conf, "sshd"));
    }
}
