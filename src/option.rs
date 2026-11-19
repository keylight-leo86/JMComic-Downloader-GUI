//! 可选的 YAML 高级配置（`--option`）。
//!
//! Rust 版只支持与下载相关的配置项；Python 版插件（plugins）不参与执行，
//! 但会在日志中给出提示，避免静默忽略。

use std::path::Path;

use serde_yaml::Value;

use crate::config::ImageFormat;
use crate::error::{JmError, Result};

#[derive(Debug, Clone, Default)]
pub struct OptionOverrides {
    pub image_concurrency: Option<usize>,
    pub photo_concurrency: Option<usize>,
    pub retry_times: Option<u32>,
    pub decode_images: Option<bool>,
    pub download_cache: Option<bool>,
    pub base_dir: Option<String>,
    pub client_impl: Option<String>,
    pub proxies: Option<String>,
    pub image_format: Option<ImageFormat>,
}

fn lookup<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = root;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn as_usize(value: &Value) -> Option<usize> {
    match value {
        Value::Number(number) => number.as_u64().map(|v| v as usize),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// 读取 YAML 配置，返回覆盖项与提示信息。
pub fn load(path: &Path) -> Result<(OptionOverrides, Vec<String>)> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        JmError::Io(std::io::Error::new(
            e.kind(),
            format!("读取 YAML 配置失败 {}: {e}", path.display()),
        ))
    })?;
    let root: Value = serde_yaml::from_str(&text)
        .map_err(|e| JmError::Msg(format!("解析 YAML 配置失败: {e}")))?;

    let mut overrides = OptionOverrides::default();
    let mut notes = Vec::new();

    if let Some(value) = lookup(&root, &["dir_rule", "base_dir"]).and_then(as_string) {
        if !value.trim().is_empty() {
            overrides.base_dir = Some(value);
        }
    }

    if let Some(value) = lookup(&root, &["download", "threading", "image"]).and_then(as_usize) {
        overrides.image_concurrency = Some(value.max(1));
    }
    if let Some(value) = lookup(&root, &["download", "threading", "photo"]).and_then(as_usize) {
        overrides.photo_concurrency = Some(value.max(1));
    }
    if let Some(value) = lookup(&root, &["download", "cache"]).and_then(as_bool) {
        overrides.download_cache = Some(value);
    }
    if let Some(value) = lookup(&root, &["download", "image", "decode"]).and_then(as_bool) {
        overrides.decode_images = Some(value);
    }
    if let Some(value) = lookup(&root, &["download", "image", "suffix"]).and_then(as_string) {
        let suffix = value.trim().trim_start_matches('.');
        if !suffix.is_empty() {
            if let Some(format) = ImageFormat::from_key(suffix) {
                if format != ImageFormat::Original {
                    overrides.image_format = Some(format);
                }
            } else {
                notes.push(format!("YAML 中不支持的图片后缀 [{value}]，已忽略"));
            }
        }
    }

    if let Some(value) = lookup(&root, &["client", "impl"]).and_then(as_string) {
        let key = value.trim().to_ascii_lowercase();
        if key == "api" || key == "html" {
            overrides.client_impl = Some(key);
        } else {
            notes.push(format!("YAML 中不支持的客户端 [{value}]，已忽略"));
        }
    }
    if let Some(value) = lookup(&root, &["client", "retry_times"]).and_then(as_usize) {
        overrides.retry_times = Some(value as u32);
    }

    if let Some(value) = lookup(&root, &["client", "postman", "meta_data", "proxies"]) {
        match value {
            Value::String(text) if !text.trim().is_empty() => {
                overrides.proxies = Some(text.trim().to_string())
            }
            Value::Mapping(map) => {
                let pick = |key: &str| {
                    map.get(Value::String(key.to_string()))
                        .and_then(as_string)
                        .filter(|s| !s.trim().is_empty())
                };
                if let Some(proxy) = pick("https").or_else(|| pick("http")) {
                    overrides.proxies = Some(proxy);
                }
            }
            _ => {}
        }
    }

    if lookup(&root, &["plugins"])
        .map(|value| match value {
            Value::Mapping(map) => !map.is_empty(),
            Value::Sequence(seq) => !seq.is_empty(),
            _ => false,
        })
        .unwrap_or(false)
    {
        notes.push("YAML 中的 plugins 配置在 Rust 版中不会执行（插件机制未迁移）".to_string());
    }

    Ok((overrides, notes))
}

impl OptionOverrides {
    /// 把 YAML 覆盖项应用到任务参数上。
    pub fn apply_to(&self, spec: &mut crate::download::JobSpec) {
        if let Some(value) = self.image_concurrency {
            spec.image_concurrency = value;
        }
        if let Some(value) = self.photo_concurrency {
            spec.photo_concurrency = value;
        }
        if let Some(value) = self.retry_times {
            spec.retry = value;
        }
        if let Some(value) = self.decode_images {
            spec.decode_images = value;
        }
        if let Some(value) = self.download_cache {
            spec.download_cache = value;
        }
        if let Some(value) = self
            .client_impl
            .as_deref()
            .and_then(crate::config::ClientKind::from_key)
        {
            spec.client = value;
        }
        if let Some(value) = &self.proxies {
            spec.proxy = value.clone();
        }
        if let Some(value) = self.image_format {
            spec.image_format = value;
        }
        if let Some(value) = &self.base_dir {
            spec.output_dir = std::path::PathBuf::from(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_supported_fields() {
        let dir = std::env::temp_dir().join(format!("jm_yaml_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("option.yml");
        std::fs::write(
            &path,
            r#"
dir_rule:
  base_dir: D:/Comics
  rule: Bd_Atitle_Pindex
download:
  cache: true
  image:
    decode: true
    suffix: .webp
  threading:
    image: 64
    photo: 12
client:
  impl: html
  retry_times: 3
  postman:
    meta_data:
      proxies:
        https: 127.0.0.1:7890
plugins:
  after_album:
    - plugin: zip
"#,
        )
        .unwrap();

        let (overrides, notes) = load(&path).unwrap();
        assert_eq!(overrides.base_dir.as_deref(), Some("D:/Comics"));
        assert_eq!(overrides.image_concurrency, Some(64));
        assert_eq!(overrides.photo_concurrency, Some(12));
        assert_eq!(overrides.retry_times, Some(3));
        assert_eq!(overrides.client_impl.as_deref(), Some("html"));
        assert_eq!(overrides.image_format, Some(ImageFormat::Webp));
        assert_eq!(overrides.proxies.as_deref(), Some("127.0.0.1:7890"));
        assert!(notes.iter().any(|note| note.contains("plugins")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_broken_yaml() {
        let dir = std::env::temp_dir().join(format!("jm_yaml_bad_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.yml");
        std::fs::write(&path, "a: [unclosed").unwrap();
        assert!(load(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
