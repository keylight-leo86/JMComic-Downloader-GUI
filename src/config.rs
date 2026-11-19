//! 全局常量与运行配置（对照 Python 版 `jm_config.py` / `jmcomic_gui.py`）。

use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "JMComic-Downloader-GUI";
pub const APP_VERSION: &str = "2.0.0";

/// 与上游一致的协议前缀。
pub const PROT: &str = "https://";

/// 禁漫移动端密钥。
pub mod secret {
    pub const APP_TOKEN_SECRET: &str = "185Hcomic3PAPP7R";
    pub const APP_TOKEN_SECRET_2: &str = "18comicAPPContent";
    pub const APP_DATA_SECRET: &str = "185Hcomic3PAPP7R";
    pub const API_DOMAIN_SERVER_SECRET: &str = "diosfjckwpqpdfjkvnqjsik";
}

/// 内置 APP 版本号，运行时会用 `/setting` 返回的版本号覆盖（若更新）。
pub const APP_VERSION_API_FALLBACK: &str = "2.0.28";

/// 移动端 API 域名（可在失败时轮换）。
pub const DEFAULT_API_DOMAINS: &[&str] = &[
    "www.cdnhjk.net",
    "www.cdngwc.cc",
    "www.cdngwc.net",
    "www.cdngwc.club",
];

/// 移动端图片 CDN 域名。
pub const DEFAULT_IMAGE_DOMAINS: &[&str] = &[
    "cdn-msp.jmapiproxy1.cc",
    "cdn-msp.jmapiproxy2.cc",
    "cdn-msp2.jmapiproxy2.cc",
    "cdn-msp3.jmapiproxy2.cc",
    "cdn-msp.jmapinodeudzn.net",
    "cdn-msp3.jmapinodeudzn.net",
];

/// 用于自动更新 API 域名的服务器列表。
pub const API_DOMAIN_SERVER_LIST: &[&str] = &[
    "https://rup4a04-c01.tos-ap-southeast-1.bytepluses.com/newsvr-2025.txt",
    "https://rup4a04-c02.tos-cn-hongkong.bytepluses.com/newsvr-2025.txt",
    "https://rup4a04-c03.tos-cn-beijing.bytepluses.com.cn/newsvr-2025.txt",
];

/// 网页端域名引导地址。
pub const JM_REDIRECT_URL: &str = "https://jm365.work/3YeBdF";
pub const JM_PUB_URL: &str = "https://jmcomicgo.org";
pub const DEFAULT_HTML_DOMAINS: &[&str] = &["18comic.vip"];

/// 图片切割阈值。
pub const SCRAMBLE_220980: u64 = 220980;
pub const SCRAMBLE_268850: u64 = 268850;
pub const SCRAMBLE_421926: u64 = 421926;

/// 错误网页中出现的特殊文本。
pub const ERROR_RESPONSE_TEXT: &[(&str, &str)] = &[
    (
        "Could not connect to mysql! Please check your database settings!",
        "禁漫服务器内部报错",
    ),
    (
        "Restricted Access!",
        "禁漫拒绝你所在ip地区的访问，你可以选择: 换域名/换代理",
    ),
];

/// 特殊 HTTP 状态码。
pub fn error_status_desc(code: u16) -> Option<&'static str> {
    match code {
        403 => Some("ip地区禁止访问/爬虫被识别"),
        500 => Some("500: 禁漫服务器内部异常（可能是服务器过载，可以切换ip或稍后重试）"),
        520 => Some("520: Web server is returning an unknown error (禁漫服务器内部报错)"),
        524 => Some(
            "524: The origin web server timed out responding to this request. (禁漫服务器处理超时)",
        ),
        _ => None,
    }
}

/// 图片后缀。
pub const IMAGE_SUFFIXES: &[&str] = &[".jpg", ".webp", ".png", ".gif"];

/// 目录名 / 文件名过长时的截断上限。
pub const FILE_NAME_LENGTH_LIMIT: usize = 100;

pub fn app_headers() -> Vec<(&'static str, &'static str)> {
    vec![(
        "user-agent",
        "Mozilla/5.0 (Linux; Android 9; V1938CT Build/PQ3A.190705.11211812; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/91.0.4472.114 Safari/537.36",
    )]
}

pub fn app_image_headers() -> Vec<(&'static str, String)> {
    vec![
        (
            "Accept",
            "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8".to_string(),
        ),
        ("X-Requested-With", "com.JMComic3.app".to_string()),
        ("Referer", format!("https://{}", DEFAULT_API_DOMAINS[0])),
        (
            "Accept-Language",
            "zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7".to_string(),
        ),
    ]
}

pub fn html_headers(domain: &str) -> Vec<(String, String)> {
    let mut v = vec![
        (
            "accept".to_string(),
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7".to_string(),
        ),
        ("accept-language".to_string(), "zh-CN,zh;q=0.9".to_string()),
        ("cache-control".to_string(), "no-cache".to_string()),
        ("pragma".to_string(), "no-cache".to_string()),
        (
            "user-agent".to_string(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36".to_string(),
        ),
        ("referer".to_string(), format!("https://{domain}")),
        ("origin".to_string(), format!("https://{domain}")),
    ];
    v.push(("authority".to_string(), domain.to_string()));
    v
}

/// 下载类型：整本 / 单章节。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Album,
    Photo,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Album => "本子",
            Kind::Photo => "章节",
        }
    }
}

/// 客户端实现：移动端 API / 网页端 HTML。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Api,
    Html,
}

impl ClientKind {
    pub fn key(self) -> &'static str {
        match self {
            ClientKind::Api => "api",
            ClientKind::Html => "html",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim().to_ascii_lowercase().as_str() {
            "api" => Some(ClientKind::Api),
            "html" => Some(ClientKind::Html),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ClientKind::Api => "移动端 API（推荐）",
            ClientKind::Html => "网页端 HTML",
        }
    }
}

/// 图片保存格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Original,
    Jpg,
    Png,
    Webp,
}

impl ImageFormat {
    /// 返回强制后缀；`None` 表示保持原格式。
    pub fn suffix(self) -> Option<&'static str> {
        match self {
            ImageFormat::Original => None,
            ImageFormat::Jpg => Some(".jpg"),
            ImageFormat::Png => Some(".png"),
            ImageFormat::Webp => Some(".webp"),
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ImageFormat::Original => "original",
            ImageFormat::Jpg => "jpg",
            ImageFormat::Png => "png",
            ImageFormat::Webp => "webp",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim().to_ascii_lowercase().as_str() {
            "original" => Some(ImageFormat::Original),
            "jpg" | "jpeg" => Some(ImageFormat::Jpg),
            "png" => Some(ImageFormat::Png),
            "webp" => Some(ImageFormat::Webp),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Original => "保持原格式",
            ImageFormat::Jpg => "统一为 JPG",
            ImageFormat::Png => "统一为 PNG",
            ImageFormat::Webp => "统一为 WebP",
        }
    }
}

/// 默认并发参数。Rust 版默认比 Python 更高：异步 I/O 没有线程栈与 GIL 开销。
pub const DEFAULT_IMAGE_THREADS: usize = 32;
pub const DEFAULT_PHOTO_THREADS: usize = 8;
pub const DEFAULT_ALBUM_THREADS: usize = 4;

pub const MAX_IMAGE_THREADS: usize = 128;
pub const MAX_PHOTO_THREADS: usize = 64;
pub const MAX_ALBUM_THREADS: usize = 32;

pub const DEFAULT_RETRY_TIMES: u32 = 5;

/// 默认代理："跟随系统"。
pub const PROXY_SYSTEM: &str = "system";
pub const PROXY_NONE: &str = "none";

/// 拼接域名与路径。
///
/// 域名允许带协议前缀（例如 `http://127.0.0.1:8080`），便于本地回环测试和自定义镜像。
pub fn join_url(domain: &str, path: &str) -> String {
    if domain.contains("://") {
        format!("{domain}{path}")
    } else {
        format!("{PROT}{domain}{path}")
    }
}
