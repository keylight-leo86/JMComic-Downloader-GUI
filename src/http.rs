//! HTTP 客户端构建：连接池、Cookie、代理、重试基础。

use std::time::Duration;

use reqwest::Client;

use crate::config::{PROXY_NONE, PROXY_SYSTEM};
use crate::error::{JmError, Result};

/// 代理配置解析结果（已规范化成 reqwest 可用的 URL）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxySetting {
    None,
    Url(String),
}

impl ProxySetting {
    pub fn describe(&self) -> String {
        match self {
            ProxySetting::None => "不使用代理".to_string(),
            ProxySetting::Url(url) => url.clone(),
        }
    }
}

/// 把界面/命令行里的代理文本解析为具体设置。
///
/// - `system` / 空 → 读取环境变量与 Windows 系统代理
/// - `none` → 直连
/// - `127.0.0.1:7890` / `http://host:port` → 指定代理
pub fn resolve_proxy(raw: &str) -> ProxySetting {
    let text = raw.trim();
    if text.is_empty() || text.eq_ignore_ascii_case(PROXY_SYSTEM) {
        return match detect_system_proxy() {
            Some(url) => ProxySetting::Url(url),
            None => ProxySetting::None,
        };
    }
    if text.eq_ignore_ascii_case(PROXY_NONE) {
        return ProxySetting::None;
    }
    ProxySetting::Url(normalize_proxy_url(text))
}

pub fn normalize_proxy_url(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

/// 读取系统代理：优先环境变量，其次 Windows 注册表 Internet Settings。
pub fn detect_system_proxy() -> Option<String> {
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(normalize_proxy_url(value));
            }
        }
    }
    detect_windows_proxy()
}

/// 解析 `ProxyServer` 的值：可能是 `host:port`，也可能是 `http=a:1;https=b:2`。
pub fn parse_proxy_server(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if value.contains('=') {
        let mut http = None;
        let mut https = None;
        for part in value.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (scheme, addr) = match part.split_once('=') {
                Some(pair) => pair,
                None => continue,
            };
            let scheme = scheme.trim().to_ascii_lowercase();
            let addr = addr.trim();
            if addr.is_empty() {
                continue;
            }
            match scheme.as_str() {
                "https" => https = Some(addr.to_string()),
                "http" => http = Some(addr.to_string()),
                "socks" | "socks5" => {
                    return Some(format!("socks5://{addr}"));
                }
                _ => {}
            }
        }
        return https.or(http).map(|addr| normalize_proxy_url(&addr));
    }

    Some(normalize_proxy_url(value))
}

#[cfg(windows)]
fn detect_windows_proxy() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    let output = std::process::Command::new("reg")
        .args(["query", key])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut enabled = false;
    let mut server: Option<String> = None;

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("ProxyEnable") {
            let value = rest.split_whitespace().last().unwrap_or("0");
            enabled = value.trim() == "0x1" || value.trim() == "1";
        } else if let Some(rest) = line.strip_prefix("ProxyServer") {
            // ProxyServer    REG_SZ    127.0.0.1:7890
            let mut parts = rest.split_whitespace();
            let _reg_type = parts.next();
            let value: Vec<&str> = parts.collect();
            if !value.is_empty() {
                server = Some(value.join(" "));
            }
        }
    }

    if !enabled {
        return None;
    }
    server.as_deref().and_then(parse_proxy_server)
}

#[cfg(not(windows))]
fn detect_windows_proxy() -> Option<String> {
    None
}

/// 构建共享的 HTTP 客户端。
///
/// 关键调优点：
/// - `pool_max_idle_per_host(128)`：图片来自 CDN，需要更多长连接复用；
/// - 开启 Cookie 存储：移动端 API 必须携带 cookies；
/// - 开启 gzip/brotli 压缩。
pub fn build_http_client(proxy_text: &str) -> Result<Client> {
    let mut builder = Client::builder()
        .cookie_store(true)
        .pool_max_idle_per_host(128)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_nodelay(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::limited(10));

    match resolve_proxy(proxy_text) {
        ProxySetting::None => {}
        ProxySetting::Url(url) => {
            let proxy = reqwest::Proxy::all(&url)
                .map_err(|e| JmError::Msg(format!("代理配置无效 [{url}]: {e}")))?;
            builder = builder.proxy(proxy);
        }
    }

    builder
        .build()
        .map_err(|e| JmError::Msg(format!("HTTP 客户端初始化失败: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_proxy_text() {
        assert_eq!(
            normalize_proxy_url("127.0.0.1:7890"),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_url("socks5://127.0.0.1:1080"),
            "socks5://127.0.0.1:1080"
        );
    }

    #[test]
    fn parses_proxy_server_formats() {
        assert_eq!(
            parse_proxy_server("127.0.0.1:7890").as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            parse_proxy_server("http=127.0.0.1:8080;https=127.0.0.1:8443").as_deref(),
            Some("http://127.0.0.1:8443")
        );
        assert_eq!(
            parse_proxy_server("socks=127.0.0.1:1080").as_deref(),
            Some("socks5://127.0.0.1:1080")
        );
        assert_eq!(parse_proxy_server("   "), None);
    }

    #[test]
    fn none_and_manual_resolution() {
        assert_eq!(resolve_proxy("none"), ProxySetting::None);
        assert!(matches!(
            resolve_proxy("127.0.0.1:7890"),
            ProxySetting::Url(_)
        ));
    }
}
