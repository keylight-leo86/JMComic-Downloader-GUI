//! 设置持久化（与 Python 版共用同一个 settings.json 路径）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{
    ClientKind, ImageFormat, DEFAULT_ALBUM_THREADS, DEFAULT_IMAGE_THREADS, DEFAULT_PHOTO_THREADS,
    MAX_ALBUM_THREADS, MAX_IMAGE_THREADS, MAX_PHOTO_THREADS, PROXY_SYSTEM,
};

pub const APP_FOLDER: &str = "JMComic-Downloader-GUI";
pub const LEGACY_APP_FOLDER: &str = "JMComicDownloader";

/// 界面缩放的默认值与范围（相对基准，1.0 即界面上的 100%）。
pub const DEFAULT_UI_SCALE: f32 = 1.0;
pub const MIN_UI_SCALE: f32 = 0.8;
pub const MAX_UI_SCALE: f32 = 2.0;

/// 缩放基准：界面上的 100% 实际按 1.25 倍渲染（即旧版本的 125%）。
pub const UI_BASE_SCALE: f32 = 1.25;

/// 把缩放值收敛到合法范围，并按 5% 对齐。
pub fn normalize_ui_scale(scale: f32) -> f32 {
    if !scale.is_finite() {
        return DEFAULT_UI_SCALE;
    }
    let clamped = scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
    (clamped * 20.0).round() / 20.0
}

/// 相对缩放 → egui 的实际 zoom_factor。
pub fn effective_zoom(relative: f32) -> f32 {
    (normalize_ui_scale(relative) * UI_BASE_SCALE).clamp(0.6, 2.5)
}

/// EXE 所在目录；开发模式下是 `target/debug` 或 `target/release`。
pub fn app_directory() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn default_download_directory() -> PathBuf {
    app_directory().join("下载")
}

fn local_app_data() -> PathBuf {
    if let Ok(value) = std::env::var("LOCALAPPDATA") {
        if !value.trim().is_empty() {
            return PathBuf::from(value);
        }
    }
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
        .join("AppData")
        .join("Local")
}

pub fn settings_path() -> PathBuf {
    local_app_data().join(APP_FOLDER).join("settings.json")
}

pub fn legacy_settings_path() -> PathBuf {
    local_app_data()
        .join(LEGACY_APP_FOLDER)
        .join("settings.json")
}

fn paths_equal(left: &str, right: &Path) -> bool {
    let left = Path::new(left);
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => {
            let a = left.to_string_lossy().to_lowercase();
            let b = right.to_string_lossy().to_lowercase();
            a == b
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `album` 或 `photo`
    pub mode: String,
    /// `default` 或 `custom`
    pub output_mode: Option<String>,
    pub output: String,
    pub client: String,
    pub format: String,
    pub image_threads: usize,
    pub photo_threads: usize,
    pub album_threads: usize,
    pub proxy: String,
    pub option: String,
    /// 界面主题：`auto` / `light` / `dark`
    pub theme: String,
    /// 界面缩放（字号比例），1.0 表示 100%
    pub ui_scale: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: "album".to_string(),
            output_mode: None,
            output: String::new(),
            client: ClientKind::Api.key().to_string(),
            format: ImageFormat::Original.key().to_string(),
            image_threads: DEFAULT_IMAGE_THREADS,
            photo_threads: DEFAULT_PHOTO_THREADS,
            album_threads: DEFAULT_ALBUM_THREADS,
            proxy: PROXY_SYSTEM.to_string(),
            option: String::new(),
            theme: "auto".to_string(),
            ui_scale: DEFAULT_UI_SCALE,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        for path in [settings_path(), legacy_settings_path()] {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(mut settings) = serde_json::from_str::<Settings>(&text) else {
                continue;
            };
            settings.normalize();
            return settings;
        }
        Settings::default()
    }

    /// 兼容 Python 版写入的中文标签，并收敛非法数值。
    pub fn normalize(&mut self) {
        self.client = match self.client.trim() {
            "移动端 API（推荐）" | "移动端 API" | "api" => "api".to_string(),
            "网页端 HTML" | "html" => "html".to_string(),
            other => {
                if other.is_empty() {
                    "api".to_string()
                } else {
                    other.to_string()
                }
            }
        };

        self.format = match self.format.trim() {
            "保持原格式" | "original" => "original".to_string(),
            "统一为 JPG" | "jpg" => "jpg".to_string(),
            "统一为 PNG" | "png" => "png".to_string(),
            "统一为 WebP" | "webp" => "webp".to_string(),
            other => {
                if other.is_empty() {
                    "original".to_string()
                } else {
                    other.to_string()
                }
            }
        };

        self.mode = match self.mode.trim() {
            "photo" => "photo".to_string(),
            _ => "album".to_string(),
        };

        self.image_threads = self.image_threads.clamp(1, MAX_IMAGE_THREADS);
        self.photo_threads = self.photo_threads.clamp(1, MAX_PHOTO_THREADS);
        self.album_threads = self.album_threads.clamp(1, MAX_ALBUM_THREADS);
        if self.proxy.trim().is_empty() {
            self.proxy = PROXY_SYSTEM.to_string();
        }

        self.theme = match self.theme.trim().to_ascii_lowercase().as_str() {
            "light" | "dark" => self.theme.trim().to_ascii_lowercase(),
            _ => "auto".to_string(),
        };

        self.ui_scale = normalize_ui_scale(self.ui_scale);
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string());
        std::fs::write(path, text)
    }

    /// 初始保存位置（兼容旧设置：默认目录跟随当前 EXE）。
    pub fn initial_output_directory(&self) -> PathBuf {
        let default = default_download_directory();
        let saved = self.output.trim();

        match self.output_mode.as_deref() {
            Some("custom") if !saved.is_empty() => PathBuf::from(saved),
            Some("default") => default,
            _ => {
                if !saved.is_empty()
                    && Path::new(saved)
                        .file_name()
                        .map(|n| n != "下载")
                        .unwrap_or(false)
                {
                    PathBuf::from(saved)
                } else {
                    default
                }
            }
        }
    }

    /// 保存时决定 output / output_mode。
    pub fn apply_output(&mut self, output: &str) {
        let trimmed = output.trim();
        let default = default_download_directory();
        if trimmed.is_empty() || paths_equal(trimmed, &default) {
            self.output_mode = Some("default".to_string());
            self.output = String::new();
        } else {
            self.output_mode = Some("custom".to_string());
            self.output = trimmed.to_string();
        }
    }

    pub fn client_kind(&self) -> ClientKind {
        ClientKind::from_key(&self.client).unwrap_or(ClientKind::Api)
    }

    pub fn image_format(&self) -> ImageFormat {
        ImageFormat::from_key(&self.format).unwrap_or(ImageFormat::Original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_scale_is_normalized() {
        assert_eq!(normalize_ui_scale(1.0), 1.0);
        assert_eq!(normalize_ui_scale(0.1), MIN_UI_SCALE);
        assert_eq!(normalize_ui_scale(9.0), MAX_UI_SCALE);
        assert_eq!(normalize_ui_scale(1.23), 1.25);
        assert_eq!(normalize_ui_scale(f32::NAN), DEFAULT_UI_SCALE);
    }

    #[test]
    fn effective_zoom_uses_125_percent_baseline() {
        assert_eq!(effective_zoom(1.0), 1.25);
        assert_eq!(effective_zoom(1.25), 1.5625);
        assert_eq!(effective_zoom(1.5), 1.875);
        assert_eq!(effective_zoom(2.0), 2.5);
        assert_eq!(effective_zoom(0.8), 1.0);
    }

    #[test]
    fn default_output_follows_app_dir() {
        let settings = Settings::default();
        assert_eq!(
            settings.initial_output_directory(),
            default_download_directory()
        );
    }

    #[test]
    fn custom_output_is_kept() {
        let mut settings = Settings {
            output_mode: Some("custom".to_string()),
            output: "D:/Comics".to_string(),
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(
            settings.initial_output_directory(),
            PathBuf::from("D:/Comics")
        );
    }

    #[test]
    fn legacy_default_is_migrated() {
        let settings = Settings {
            output_mode: None,
            output: "D:/OldPackage/下载".to_string(),
            ..Default::default()
        };
        assert_eq!(
            settings.initial_output_directory(),
            default_download_directory()
        );
    }

    #[test]
    fn labels_are_normalized() {
        let mut settings = Settings {
            client: "网页端 HTML".to_string(),
            format: "统一为 WebP".to_string(),
            image_threads: 9999,
            photo_threads: 0,
            album_threads: 100,
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.client, "html");
        assert_eq!(settings.format, "webp");
        assert_eq!(settings.image_threads, MAX_IMAGE_THREADS);
        assert_eq!(settings.photo_threads, 1);
        assert_eq!(settings.album_threads, MAX_ALBUM_THREADS);
    }

    #[test]
    fn apply_output_tracks_default() {
        let mut settings = Settings::default();
        settings.apply_output(&default_download_directory().to_string_lossy());
        assert_eq!(settings.output_mode.as_deref(), Some("default"));

        settings.apply_output("E:/Comics");
        assert_eq!(settings.output_mode.as_deref(), Some("custom"));
        assert_eq!(settings.output, "E:/Comics");
    }
}
