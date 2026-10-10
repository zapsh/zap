use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::{PlatformError, Result};

/// 目录项元信息（跨平台统一视图）。
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_file: bool,
    pub size: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

/// 文件系统操作抽象（跨 UNIX）。
pub trait FileSystem {
    fn read_to_string(&self, path: &Path) -> Result<String>;
    fn write_string(&self, path: &Path, content: &str) -> Result<()>;
    fn set_permissions(&self, path: &Path, mode: u32) -> Result<()>;
    /// 设置属主（uid/gid）；传 u32::MAX 表示该侧沿用原值。
    fn set_owner(&self, path: &Path, uid: u32, gid: u32) -> Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>>;
    fn remove(&self, path: &Path) -> Result<()>;
}

pub struct UnixFs;

impl FileSystem for UnixFs {
    fn read_to_string(&self, path: &Path) -> Result<String> {
        Ok(std::fs::read_to_string(path)?)
    }
    fn write_string(&self, path: &Path, content: &str) -> Result<()> {
        std::fs::write(path, content)?;
        Ok(())
    }
    fn set_permissions(&self, path: &Path, mode: u32) -> Result<()> {
        let mut p = std::fs::metadata(path)?.permissions();
        p.set_mode(mode);
        std::fs::set_permissions(path, p)?;
        Ok(())
    }
    fn set_owner(&self, path: &Path, uid: u32, gid: u32) -> Result<()> {
        let cpath = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| PlatformError::Other("路径含 NUL 字节".into()))?;
        // -1 (即 u32::MAX) 让系统沿用原值
        unsafe {
            if libc::chown(cpath.as_ptr(), uid, gid) != 0 {
                return Err(PlatformError::Io(std::io::Error::last_os_error()));
            }
        }
        Ok(())
    }
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
    fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(path)? {
            let e = e?;
            let meta = e.metadata()?;
            out.push(DirEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                path: e.path(),
                is_dir: meta.is_dir(),
                is_file: meta.is_file(),
                size: meta.size(),
                mode: meta.mode(),
                uid: meta.uid(),
                gid: meta.gid(),
            });
        }
        Ok(out)
    }
    fn remove(&self, path: &Path) -> Result<()> {
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn write_read_mode_roundtrip() {
        let p = std::env::temp_dir().join(format!("zap_platform_test_{}.txt", std::process::id()));
        let fs = UnixFs;
        fs.write_string(&p, "hello").unwrap();
        assert_eq!(fs.read_to_string(&p).unwrap(), "hello");
        fs.set_permissions(&p, 0o600).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        fs.remove(&p).unwrap();
        assert!(!p.exists());
    }
}
