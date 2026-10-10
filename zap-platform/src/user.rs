use std::ffi::{CStr, CString};

use crate::{PlatformError, Result};

#[derive(Debug, Clone)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
    pub shell: String,
}

#[derive(Debug, Clone)]
pub struct Group {
    pub name: String,
    pub gid: u32,
}

/// 用户与权限抽象（跨 UNIX）。
pub trait UserManager {
    fn current_uid(&self) -> u32;
    fn current_gid(&self) -> u32;
    fn lookup_user(&self, name: &str) -> Result<User>;
    fn lookup_group(&self, name: &str) -> Result<Group>;
    /// 由 root 降权到目标 uid/gid（清空附加组 + setgid + setuid）。
    fn drop_privileges(&self, uid: u32, gid: u32) -> Result<()>;
}

pub struct UnixUser;

impl UnixUser {
    fn lookup_user_inner(name: &str) -> Result<User> {
        let cname =
            CString::new(name).map_err(|_| PlatformError::Other("用户名含非法字节".into()))?;
        unsafe {
            let mut pwd: libc::passwd = std::mem::zeroed();
            let mut result: *mut libc::passwd = std::ptr::null_mut();
            let mut buf = vec![0u8; 4096];
            if libc::getpwnam_r(
                cname.as_ptr(),
                &mut pwd,
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                &mut result,
            ) != 0
            {
                return Err(PlatformError::Io(std::io::Error::last_os_error()));
            }
            if result.is_null() {
                return Err(PlatformError::NotFound(name.into()));
            }
            Ok(User {
                name: CStr::from_ptr(pwd.pw_name).to_string_lossy().into_owned(),
                uid: pwd.pw_uid,
                gid: pwd.pw_gid,
                home: CStr::from_ptr(pwd.pw_dir).to_string_lossy().into_owned(),
                shell: CStr::from_ptr(pwd.pw_shell).to_string_lossy().into_owned(),
            })
        }
    }

    fn lookup_group_inner(name: &str) -> Result<Group> {
        let cname =
            CString::new(name).map_err(|_| PlatformError::Other("组名含非法字节".into()))?;
        unsafe {
            let mut gr: libc::group = std::mem::zeroed();
            let mut result: *mut libc::group = std::ptr::null_mut();
            let mut buf = vec![0u8; 4096];
            if libc::getgrnam_r(
                cname.as_ptr(),
                &mut gr,
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                &mut result,
            ) != 0
            {
                return Err(PlatformError::Io(std::io::Error::last_os_error()));
            }
            if result.is_null() {
                return Err(PlatformError::NotFound(name.into()));
            }
            Ok(Group {
                name: CStr::from_ptr(gr.gr_name).to_string_lossy().into_owned(),
                gid: gr.gr_gid,
            })
        }
    }
}

impl UserManager for UnixUser {
    fn current_uid(&self) -> u32 {
        unsafe { libc::getuid() }
    }
    fn current_gid(&self) -> u32 {
        unsafe { libc::getgid() }
    }
    fn lookup_user(&self, name: &str) -> Result<User> {
        Self::lookup_user_inner(name)
    }
    fn lookup_group(&self, name: &str) -> Result<Group> {
        Self::lookup_group_inner(name)
    }
    fn drop_privileges(&self, uid: u32, gid: u32) -> Result<()> {
        unsafe {
            // 先清空附加组，避免降权后仍能借组权限
            if libc::setgroups(0, std::ptr::null()) != 0 {
                return Err(PlatformError::Io(std::io::Error::last_os_error()));
            }
            if libc::setgid(gid) != 0 {
                return Err(PlatformError::Io(std::io::Error::last_os_error()));
            }
            if libc::setuid(uid) != 0 {
                return Err(PlatformError::Io(std::io::Error::last_os_error()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn root_lookup_works() {
        let u = UnixUser;
        assert!(u.lookup_user("root").is_ok());
        assert!(u.lookup_group("root").is_ok());
        assert_eq!(u.current_uid(), unsafe { libc::getuid() });
    }
}
