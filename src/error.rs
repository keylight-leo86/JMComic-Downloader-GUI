//! 统一错误类型。

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum JmError {
    #[error("网络请求失败: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO 失败: {0}")]
    Io(#[from] std::io::Error),

    #[error("图片处理失败: {0}")]
    Image(String),

    #[error("PDF 生成失败: {0}")]
    Pdf(String),

    #[error("资源不存在或已下架: {0}")]
    Missing(String),

    #[error("{0}")]
    Msg(String),
}

impl JmError {
    pub fn msg<T>(text: impl Into<String>) -> Result<T> {
        Err(JmError::Msg(text.into()))
    }

    pub fn missing<T>(text: impl Into<String>) -> Result<T> {
        Err(JmError::Missing(text.into()))
    }
}

pub type Result<T> = std::result::Result<T, JmError>;

/// 给 IO 错误补充路径上下文。
pub fn io_ctx(path: &PathBuf, err: std::io::Error) -> JmError {
    JmError::Io(std::io::Error::new(
        err.kind(),
        format!("{} ({})", err, path.display()),
    ))
}

/// 只保留 ASCII 可打印字符的安全错误描述（避免日志出现控制字符）。
pub fn brief(err: &JmError) -> String {
    let text = err.to_string();
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\n' || ch == '\r' {
            out.push(' ');
        } else if !ch.is_control() {
            out.push(ch);
        }
    }
    out
}
