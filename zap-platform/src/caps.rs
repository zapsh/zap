use crate::{PlatformError, Result};

/// 读取当前进程的有效 capabilities 集合（Linux 的 /proc/self/status CapEff）。
/// 其它 UNIX（BSD 等）无此概念，返回 NotSupported。
#[cfg(target_os = "linux")]
pub fn read_effective() -> Result<u64> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    for line in status.lines() {
        if let Some(hex) = line.trim().strip_prefix("CapEff:") {
            let hex = hex.trim();
            return u64::from_str_radix(hex, 16)
                .map_err(|e| PlatformError::Parse(format!("CapEff 解析失败：{e}")));
        }
    }
    Err(PlatformError::NotFound("CapEff 未找到".into()))
}

#[cfg(not(target_os = "linux"))]
pub fn read_effective() -> Result<u64> {
    Err(PlatformError::NotSupported(
        "capabilities 仅 Linux 支持".into(),
    ))
}

/// 设置有效 capabilities（需 libcap / CAP_SETFCAP）。当前仅占位，统一跨平台接口。
pub fn set_effective(_caps: u64) -> Result<()> {
    Err(PlatformError::NotSupported(
        "set_effective 尚未实现（需 libcap 绑定）".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(target_os = "linux")]
    fn cap_eff_readable() {
        assert!(read_effective().is_ok());
    }
    #[test]
    #[cfg(not(target_os = "linux"))]
    fn cap_eff_unsupported() {
        assert!(read_effective().is_err());
    }
}
