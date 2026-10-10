use std::io;

/// 平台抽象层统一错误类型。
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("IO 错误：{0}")]
    Io(#[from] io::Error),

    #[error("未找到：{0}")]
    NotFound(String),

    /// 当前平台 / 当前实现不支持的操作（如 macOS 上的 capabilities）。
    #[error("不支持：{0}")]
    NotSupported(String),

    #[error("解析错误：{0}")]
    Parse(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, PlatformError>;
