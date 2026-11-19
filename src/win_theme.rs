//! Windows 主题与强调色检测（用于让界面跟随 Win11 的浅色/深色与系统主题色）。

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Auto,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn key(self) -> &'static str {
        match self {
            ThemeMode::Auto => "auto",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    pub fn from_key(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "light" | "浅色" => ThemeMode::Light,
            "dark" | "深色" => ThemeMode::Dark,
            _ => ThemeMode::Auto,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Auto => "跟随系统",
            ThemeMode::Light => "浅色",
            ThemeMode::Dark => "深色",
        }
    }
}

/// 系统外观。
#[derive(Debug, Clone, Copy)]
pub struct SystemAppearance {
    /// 是否深色模式（未取到注册表时按浅色处理）。
    pub dark: bool,
    /// 系统强调色（RGB）。
    pub accent: Option<(u8, u8, u8)>,
}

impl Default for SystemAppearance {
    fn default() -> Self {
        Self {
            dark: false,
            accent: None,
        }
    }
}

/// 解析 `reg query` 输出中的某个值。
pub fn parse_reg_value(output: &str, value: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else { continue };
        if name != value {
            continue;
        }
        // 第二个字段是类型（REG_DWORD / REG_SZ），其余是值（可能含空格）
        let _kind = parts.next();
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() {
            return None;
        }
        return Some(rest.join(" ").trim().to_string());
    }
    None
}

/// 解析 DWORD 文本（支持 0x 前缀）。
pub fn parse_dword(text: &str) -> Option<u32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        text.parse::<u32>().ok()
    }
}

/// Windows 的 `AccentColor`/`ColorizationColor` 都是 32 位颜色，按 ABGR 解析。
pub fn abgr_to_rgb(value: u32) -> (u8, u8, u8) {
    let r = (value & 0xFF) as u8;
    let g = ((value >> 8) & 0xFF) as u8;
    let b = ((value >> 16) & 0xFF) as u8;
    (r, g, b)
}

/// ARGB（`ColorizationColor`）→ RGB。
pub fn argb_to_rgb(value: u32) -> (u8, u8, u8) {
    let r = ((value >> 16) & 0xFF) as u8;
    let g = ((value >> 8) & 0xFF) as u8;
    let b = (value & 0xFF) as u8;
    (r, g, b)
}

fn reg_query(key: &str, value: &str) -> Option<String> {
    let mut command = Command::new("reg");
    command.args(["query", key, "/v", value]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    parse_reg_value(&text, value)
}

/// 读取系统外观；失败时回落到浅色 + 默认强调色。
pub fn detect() -> SystemAppearance {
    let dark = reg_query(
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "AppsUseLightTheme",
    )
    .and_then(|text| parse_dword(&text))
    .map(|value| value == 0)
    .unwrap_or(false);

    let accent = reg_query(r"HKCU\Software\Microsoft\Windows\DWM", "AccentColor")
        .and_then(|text| parse_dword(&text))
        .map(abgr_to_rgb)
        .or_else(|| {
            reg_query(r"HKCU\Software\Microsoft\Windows\DWM", "ColorizationColor")
                .and_then(|text| parse_dword(&text))
                .map(argb_to_rgb)
        })
        // 全黑/全白通常表示未启用主题色，忽略
        .filter(|(r, g, b)| !(*r < 8 && *g < 8 && *b < 8) && !(*r > 247 && *g > 247 && *b > 247));

    SystemAppearance { dark, accent }
}

/// 根据用户选择决定最终是否深色。
pub fn resolve_dark(mode: ThemeMode) -> bool {
    match mode {
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
        ThemeMode::Auto => detect().dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_output() {
        let sample = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\r\n    AppsUseLightTheme    REG_DWORD    0x0\r\n\r\n";
        assert_eq!(
            parse_reg_value(sample, "AppsUseLightTheme").as_deref(),
            Some("0x0")
        );
        assert_eq!(parse_dword("0x0"), Some(0));
        assert_eq!(parse_dword("1"), Some(1));
        assert_eq!(parse_dword("0x1a"), Some(26));
        assert_eq!(parse_reg_value(sample, "Missing"), None);
    }

    #[test]
    fn parses_colors() {
        // 0x00BBGGRR
        assert_eq!(abgr_to_rgb(0x00_6B_85_16), (0x16, 0x85, 0x6b));
        // 0xAARRGGBB
        assert_eq!(argb_to_rgb(0xFF_16_85_6B), (0x16, 0x85, 0x6b));
    }

    #[test]
    fn theme_mode_roundtrip() {
        for mode in [ThemeMode::Auto, ThemeMode::Light, ThemeMode::Dark] {
            assert_eq!(ThemeMode::from_key(mode.key()), mode);
        }
        assert_eq!(ThemeMode::from_key("深色"), ThemeMode::Dark);
    }
}
