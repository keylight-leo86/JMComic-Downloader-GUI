//! JMComic Downloader（Rust 版）核心库。
//!
//! 模块分层与 Python 版对应关系：
//! - `config`      ← jm_config.py / jmcomic_gui.py 常量
//! - `crypto`      ← JmCryptoTool
//! - `api`         ← JmApiClient / JmHtmlClient 的请求与解析
//! - `html_client` ← JmcomicText / JmPageTool 的网页端正则解析
//! - `types`       ← JmAlbumDetail / JmPhotoDetail
//! - `imageio`     ← JmImageTool（解密、保存）
//! - `pdf`         ← img2pdf + pypdf 校验
//! - `download`    ← JmDownloader + 并发布局
//! - `settings`    ← GUI 设置持久化

pub mod api;
pub mod config;
pub mod crypto;
pub mod download;
pub mod error;
pub mod events;
pub mod html_client;
pub mod http;
pub mod imageio;
pub mod naming;
pub mod option;
pub mod pdf;
pub mod selftest;
pub mod settings;
pub mod types;
pub mod win_theme;

#[cfg(feature = "gui")]
pub mod gui;

/// 版本号（来自 Cargo.toml）。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
