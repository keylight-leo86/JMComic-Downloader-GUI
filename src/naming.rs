//! 车号解析与文件名清洗（对照 `jmcomic_gui.py` 中的 `parse_ids` / `safe_pdf_stem`）。

use std::collections::HashSet;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::error::{JmError, Result};
use crate::types::{Album, Photo};

/// 把用户输入拆分为去重后的车号列表。
///
/// 支持：`1455254`、`JM1455254`、`A1455254`、`P1455254`、
/// `https://.../album/1455254`、`https://...?id=1455254`。
pub fn parse_ids(raw: &str) -> Result<Vec<String>> {
    let tokens: Vec<&str> = raw
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '，' | '；'))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect();

    if tokens.is_empty() {
        return Err(JmError::Msg("请至少输入一个 JM 车号或链接".to_string()));
    }

    let album_path = Regex::new(r"(?i)/(?:album|photo)/(?:\?id=)?(\d+)").expect("valid regex");
    let id_query = Regex::new(r"(?i)[?&]id=(\d+)").expect("valid regex");

    let mut result = Vec::with_capacity(tokens.len());
    let mut seen = HashSet::with_capacity(tokens.len());

    for token in tokens {
        let mut value: &str = token;

        if token.len() >= 2 && token[..2].eq_ignore_ascii_case("jm") {
            value = &token[2..];
        } else {
            let mut chars = token.chars();
            if let Some(first) = chars.next() {
                let rest = chars.as_str();
                let lowered = first.to_ascii_lowercase();
                if (lowered == 'a' || lowered == 'p')
                    && !rest.is_empty()
                    && rest.chars().all(|c| c.is_ascii_digit())
                {
                    value = rest;
                }
            }
        }

        if !is_ascii_digits(value) {
            let matched = album_path
                .captures(token)
                .or_else(|| id_query.captures(token));
            match matched {
                Some(caps) => value = caps.get(1).map(|m| m.as_str()).unwrap_or(""),
                None => return Err(JmError::Msg(format!("无法识别：{token}"))),
            }
        }

        if !is_ascii_digits(value) {
            return Err(JmError::Msg(format!("无法识别：{token}")));
        }

        let normalized = strip_leading_zeros(value);
        if seen.insert(normalized.clone()) {
            result.push(normalized);
        }
    }

    Ok(result)
}

fn is_ascii_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn strip_leading_zeros(text: &str) -> String {
    let trimmed = text.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 替换 Windows 文件名非法字符（用于目录名）。
pub fn fix_windir_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let illegal = matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            || (ch as u32) < 0x20;
        out.push(if illegal { '_' } else { ch });
    }
    out.trim().trim_end_matches(['.', ' ']).to_string()
}

/// 清洗 PDF 文件名主体（不含后缀）。
pub fn safe_pdf_stem(value: &str, fallback: &str, max_length: usize) -> String {
    let normalized: String = value.nfkc().collect();

    let mut replaced = String::with_capacity(normalized.len());
    let mut last_space = false;
    for ch in normalized.chars() {
        let illegal = matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            || (ch as u32) < 0x20;
        if illegal {
            replaced.push('_');
            last_space = false;
            continue;
        }
        if ch.is_whitespace() {
            if !last_space {
                replaced.push(' ');
                last_space = true;
            }
            continue;
        }
        replaced.push(ch);
        last_space = false;
    }

    let mut value = replaced.trim_matches([' ', '.']).to_string();
    if value.is_empty() {
        value = fallback.to_string();
    }

    // Windows 保留设备名
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.iter().any(|r| value.eq_ignore_ascii_case(r)) {
        value.insert(0, '_');
    }

    let truncated: String = value.chars().take(max_length).collect();
    let trimmed = truncated.trim_end_matches([' ', '.']).to_string();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed
    }
}

/// 生成章节对应的 PDF 文件名。
pub fn pdf_filename_for_photo(album: &Album, photo: &Photo) -> String {
    let album_name = safe_pdf_stem(&album.name, &format!("JM{}", album.id), 110);

    if album.is_single_chapter() {
        return format!("{album_name}.pdf");
    }

    let chapter_index = photo.index.max(1);
    let chapter_name = photo.name.trim();
    let mut suffix = format!("第{chapter_index}话");
    if !chapter_name.is_empty() && chapter_name != album.name.trim() {
        suffix.push(' ');
        suffix.push_str(chapter_name);
    }

    let stem = safe_pdf_stem(
        &format!("{album_name} - {suffix}"),
        &format!("{album_name} - 第{chapter_index}话"),
        180,
    );
    format!("{stem}.pdf")
}

/// 图片所在目录名（本子标题）。
pub fn album_dir_name(album: &Album) -> String {
    let fixed = fix_windir_name(&album.name);
    if fixed.is_empty() {
        format!("JM{}", album.id)
    } else {
        fixed.chars().take(120).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_inputs() {
        assert_eq!(parse_ids("1455254").unwrap(), vec!["1455254"]);
        assert_eq!(parse_ids("JM1455254").unwrap(), vec!["1455254"]);
        assert_eq!(parse_ids("jm1455254").unwrap(), vec!["1455254"]);
        assert_eq!(parse_ids("A1455254").unwrap(), vec!["1455254"]);
        assert_eq!(
            parse_ids("https://devapp.18comic.cc/comic/detail?id=1455254").unwrap(),
            vec!["1455254"]
        );
        assert_eq!(
            parse_ids("https://18comic.vip/album/1455254/").unwrap(),
            vec!["1455254"]
        );
        assert_eq!(
            parse_ids("1,2；3 1455254\nJM99").unwrap(),
            vec!["1", "2", "3", "1455254", "99"]
        );
        // 去重 + 去前导零
        assert_eq!(parse_ids("01455254 1455254").unwrap(), vec!["1455254"]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_ids("").is_err());
        assert!(parse_ids("abc").is_err());
        assert!(parse_ids("https://example.com/xxx").is_err());
    }

    #[test]
    fn pdf_stem_cleans_illegal_chars() {
        assert_eq!(safe_pdf_stem("a/b:c*?", "fb", 50), "a_b_c__");
        assert_eq!(safe_pdf_stem("   ", "fallback", 50), "fallback");
        assert_eq!(safe_pdf_stem("CON", "fb", 50), "_CON");
        assert_eq!(safe_pdf_stem("名字  很长", "fb", 2), "名字");
        // 全角斜杠等 NFKC 归一化
        assert_eq!(safe_pdf_stem("ＡＢＣ", "fb", 10), "ABC");
    }

    #[test]
    fn pdf_names_follow_album_shape() {
        let album = Album {
            id: "100".into(),
            name: "单章本子".into(),
            authors: vec![],
            tags: vec![],
            description: String::new(),
            page_count: 0,
            scramble_id: "0".into(),
            episodes: vec![crate::types::Episode {
                photo_id: "100".into(),
                index: 1,
                name: "单章本子".into(),
            }],
        };
        let photo = Photo {
            id: "100".into(),
            name: "单章本子".into(),
            album_id: "100".into(),
            index: 1,
            sort: 2,
            series_id: 0,
            tags: vec![],
            scramble_id: "0".into(),
            image_domain: "cdn".into(),
            images: vec![],
            album: Some(album.clone()),
        };
        assert_eq!(pdf_filename_for_photo(&album, &photo), "单章本子.pdf");

        let mut multi = album.clone();
        multi.episodes.push(crate::types::Episode {
            photo_id: "101".into(),
            index: 2,
            name: "终章".into(),
        });
        let mut photo2 = photo.clone();
        photo2.index = 2;
        photo2.name = "终章".into();
        assert_eq!(
            pdf_filename_for_photo(&multi, &photo2),
            "单章本子 - 第2话 终章.pdf"
        );
    }
}
