//! 图片下载后的处理：禁漫竖切解密、格式转换、落盘、PDF 页源解析。
//!
//! 与 Python 版相比的关键优化：
//! - 无需解密且格式未变时**直接写原始字节**，不再走解码/重编码（更快且真正无损）；
//! - 需要解密时只做一次 RGB 解码 + 一次编码；
//! - JPEG 保留原始字节供 PDF 以 `DCTDecode` 嵌入。

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{ExtendedColorType, ImageEncoder, RgbImage};

use crate::config::{SCRAMBLE_268850, SCRAMBLE_421926};
use crate::crypto::md5_hex;
use crate::error::{JmError, Result};

/// 已保存图片的信息。
#[derive(Debug, Clone)]
pub struct SavedImage {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub descrambled: bool,
}

/// 计算禁漫图片的切割段数（见上游 `JmImageTool.get_num`）。
pub fn descramble_num(scramble_id: &str, aid: &str, stem: &str) -> usize {
    let scramble = scramble_id.trim().parse::<u64>().unwrap_or(0);
    let aid = aid.trim().parse::<u64>().unwrap_or(0);

    if aid < scramble {
        return 0;
    }
    if aid < SCRAMBLE_268850 {
        return 10;
    }

    let modulus = if aid < SCRAMBLE_421926 { 10 } else { 8 };
    let digest = md5_hex(&format!("{aid}{stem}"));
    let last = digest.as_bytes().last().copied().unwrap_or(b'0') as u64;
    ((last % modulus) * 2 + 2) as usize
}

/// 竖切重排（等价于 Python 版 `decode_and_save`）。
pub fn descramble_rgb(source: &RgbImage, num: usize) -> RgbImage {
    let (width, height) = source.dimensions();
    if num == 0 || height == 0 || num > height as usize {
        return source.clone();
    }

    let mut result = RgbImage::new(width, height);
    let over = height as usize % num;
    let step = height as usize / num;
    let row_bytes = width as usize * 3;
    let src = source.as_raw();
    let dst = result.as_mut();

    for index in 0..num {
        let mut move_rows = step;
        let y_src = height as usize - step * (index + 1) - over;
        let mut y_dst = step * index;

        if index == 0 {
            move_rows += over;
        } else {
            y_dst += over;
        }

        if y_src + move_rows > height as usize || y_dst + move_rows > height as usize {
            continue;
        }

        for row in 0..move_rows {
            let from = (y_src + row) * row_bytes;
            let to = (y_dst + row) * row_bytes;
            dst[to..to + row_bytes].copy_from_slice(&src[from..from + row_bytes]);
        }
    }

    result
}

/// 把 RGB8 像素编码为目标格式。
///
/// - JPEG：质量 95（禁漫的 JPEG 若需重新竖切，只能重新编码）；
/// - WebP：无损；
/// - 其它：PNG 无损。
pub fn encode_rgb_image(data: &[u8], width: u32, height: u32, suffix: &str) -> Result<Vec<u8>> {
    match suffix.to_ascii_lowercase().as_str() {
        ".jpg" | ".jpeg" => encode_jpeg(data, width, height),
        ".webp" => encode_webp(data, width, height),
        _ => {
            let mut out: Vec<u8> = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new(&mut out);
            encoder
                .write_image(data, width, height, ExtendedColorType::Rgb8)
                .map_err(|e| JmError::Image(format!("PNG 编码失败: {e}")))?;
            Ok(out)
        }
    }
}

fn encode_jpeg(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95);
    encoder
        .encode(data, width, height, ExtendedColorType::Rgb8)
        .map_err(|e| JmError::Image(format!("JPEG 编码失败: {e}")))?;
    Ok(out)
}

/// WebP 无损编码。
fn encode_webp(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
    encoder
        .encode(data, width, height, ExtendedColorType::Rgb8)
        .map_err(|e| JmError::Image(format!("WebP 编码失败: {e}")))?;
    Ok(out)
}

/// 解码任意图片为 RGB8。
///
/// 说明：曾尝试用 libwebp / libmozjpeg 的原生实现，实测在本项目负载下
/// 并无收益（libwebp 解码同速、编码慢一个数量级；mozjpeg 编码慢 10 倍），
/// 因此统一使用纯 Rust 实现，构建时也不需要 C 工具链。
pub fn decode_rgb(bytes: &[u8]) -> Result<RgbImage> {
    let decoded =
        image::load_from_memory(bytes).map_err(|e| JmError::Image(format!("图片解码失败: {e}")))?;
    Ok(decoded.to_rgb8())
}

/// 判断文件后缀与真实图片格式是否一致（用于决定能否直接写原始字节）。
fn suffix_matches_format(suffix: &str, format: image::ImageFormat) -> bool {
    use image::ImageFormat;
    match suffix.to_ascii_lowercase().as_str() {
        ".jpg" | ".jpeg" => format == ImageFormat::Jpeg,
        ".png" => format == ImageFormat::Png,
        ".webp" => format == ImageFormat::WebP,
        ".gif" => format == ImageFormat::Gif,
        _ => false,
    }
}

/// 保存一张图片。
///
/// - `source_suffix`：图片 URL 的后缀（例如 `.webp`）
/// - `target_suffix`：实际要保存的后缀（可能因“统一格式”设置而不同）
/// - `decode_enabled`：是否执行禁漫解密（gif 不处理）
pub fn save_image(
    bytes: &[u8],
    source_suffix: &str,
    target_suffix: &str,
    target: &Path,
    scramble_id: &str,
    aid: &str,
    stem: &str,
    decode_enabled: bool,
) -> Result<SavedImage> {
    if bytes.is_empty() {
        return Err(JmError::Image("图片响应为空".to_string()));
    }

    let num = if decode_enabled {
        descramble_num(scramble_id, aid, stem)
    } else {
        0
    };
    let same_suffix = source_suffix.eq_ignore_ascii_case(target_suffix);

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            JmError::Io(std::io::Error::new(
                e.kind(),
                format!("无法创建图片目录 {}: {e}", parent.display()),
            ))
        })?;
    }

    // 快速路径：无需解密且格式未变 → 直接写原始字节
    if num == 0 && same_suffix {
        let reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| JmError::Image("响应不是可识别的图片格式".to_string()))?;
        let detected = reader
            .format()
            .ok_or_else(|| JmError::Image("响应不是可识别的图片格式".to_string()))?;
        // 实际内容与目标后缀一致时才直写，避免服务端返回与扩展名不符的图片
        if suffix_matches_format(target_suffix, detected) {
            let (width, height) = reader.into_dimensions().unwrap_or((0, 0));
            std::fs::write(target, bytes).map_err(|e| {
                JmError::Io(std::io::Error::new(
                    e.kind(),
                    format!("写入图片失败 {}: {e}", target.display()),
                ))
            })?;
            return Ok(SavedImage {
                path: target.to_path_buf(),
                width,
                height,
                bytes: bytes.len() as u64,
                descrambled: false,
            });
        }
    }

    let rgb = decode_rgb(bytes)?;
    let (width, height) = rgb.dimensions();

    let encoded = if num > 0 {
        encode_rgb_image(
            descramble_rgb(&rgb, num).as_raw(),
            width,
            height,
            target_suffix,
        )?
    } else {
        encode_rgb_image(rgb.as_raw(), width, height, target_suffix)?
    };

    std::fs::write(target, &encoded).map_err(|e| {
        JmError::Io(std::io::Error::new(
            e.kind(),
            format!("写入图片失败 {}: {e}", target.display()),
        ))
    })?;

    Ok(SavedImage {
        path: target.to_path_buf(),
        width,
        height,
        bytes: encoded.len() as u64,
        descrambled: num > 0,
    })
}

/// JPEG 头部信息。
#[derive(Debug, Clone, Copy)]
pub struct JpegHeader {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub orientation: u16,
}

/// 解析 JPEG 的 SOF 段（宽高、颜色分量）与 EXIF 方向。
pub fn parse_jpeg(bytes: &[u8]) -> Option<JpegHeader> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }

    let mut orientation = 1u16;
    let mut index = 2usize;

    while index + 4 <= bytes.len() {
        if bytes[index] != 0xFF {
            index += 1;
            continue;
        }

        // 跳过连续的 0xFF 填充
        let mut marker_pos = index + 1;
        while marker_pos < bytes.len() && bytes[marker_pos] == 0xFF {
            marker_pos += 1;
        }
        if marker_pos >= bytes.len() {
            break;
        }
        let marker = bytes[marker_pos];
        index = marker_pos + 1;

        match marker {
            0xD8 | 0x01 | 0xD0..=0xD7 => continue,
            0xD9 => break,
            _ => {}
        }

        if index + 2 > bytes.len() {
            break;
        }
        let length = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
        if length < 2 {
            break;
        }
        let segment_end = index + length;
        if segment_end > bytes.len() {
            break;
        }

        let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof && segment_end >= index + 8 {
            let height = u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
            let components = bytes[index + 7];
            return Some(JpegHeader {
                width,
                height,
                components,
                orientation,
            });
        }

        if marker == 0xE1 && segment_end > index + 2 {
            if let Some(value) = parse_exif_orientation(&bytes[index + 2..segment_end]) {
                orientation = value;
            }
        }

        index = segment_end;
    }

    None
}

fn parse_exif_orientation(payload: &[u8]) -> Option<u16> {
    if payload.len() < 8 || &payload[0..6] != b"Exif\0\0" {
        return None;
    }
    let tiff = &payload[6..];
    if tiff.len() < 8 {
        return None;
    }

    let little_endian = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };

    let read16 = |buffer: &[u8], offset: usize| -> u16 {
        if offset + 2 > buffer.len() {
            return 0;
        }
        if little_endian {
            u16::from_le_bytes([buffer[offset], buffer[offset + 1]])
        } else {
            u16::from_be_bytes([buffer[offset], buffer[offset + 1]])
        }
    };
    let read32 = |buffer: &[u8], offset: usize| -> u32 {
        if offset + 4 > buffer.len() {
            return 0;
        }
        if little_endian {
            u32::from_le_bytes([
                buffer[offset],
                buffer[offset + 1],
                buffer[offset + 2],
                buffer[offset + 3],
            ])
        } else {
            u32::from_be_bytes([
                buffer[offset],
                buffer[offset + 1],
                buffer[offset + 2],
                buffer[offset + 3],
            ])
        }
    };

    if read16(tiff, 2) != 42 {
        return None;
    }

    let ifd_offset = read32(tiff, 4) as usize;
    if ifd_offset + 2 > tiff.len() {
        return None;
    }
    let count = read16(tiff, ifd_offset) as usize;

    for index in 0..count {
        let entry = ifd_offset + 2 + index * 12;
        if entry + 12 > tiff.len() {
            return None;
        }
        if read16(tiff, entry) == 0x0112 {
            return Some(read16(tiff, entry + 8));
        }
    }

    None
}

/// EXIF orientation → PDF 页面旋转角。
pub fn exif_rotation(orientation: u16) -> u16 {
    match orientation {
        3 | 4 => 180,
        5 | 6 => 90,
        7 | 8 => 270,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn descramble_num_thresholds() {
        assert_eq!(descramble_num("220980", "100000", "00001"), 0);
        assert_eq!(descramble_num("220980", "240000", "00001"), 10);

        let num = descramble_num("220980", "500000", "00001");
        assert!((2..=20).contains(&num), "num={num}");
        assert_eq!(num % 2, 0);
    }

    #[test]
    fn descramble_reverses_bands() {
        // 4 行，num=2 → 输出应为 [2,3,0,1]
        let img = RgbImage::from_fn(1, 4, |_x, y| Rgb([y as u8, y as u8, y as u8]));
        let out = descramble_rgb(&img, 2);
        let values: Vec<u8> = out.pixels().map(|p| p[0]).collect();
        assert_eq!(values, vec![2, 3, 0, 1]);
    }

    #[test]
    fn descramble_handles_remainder() {
        // 5 行，num=2：step=2, over=1
        // i=0: y_src=5-2-1=2, dst=0, rows=3 → copy 2,3,4 → 0,1,2
        // i=1: y_src=5-4-1=0, dst=2+1=3, rows=2 → copy 0,1 → 3,4
        let img = RgbImage::from_fn(1, 5, |_x, y| Rgb([y as u8, 0, 0]));
        let out = descramble_rgb(&img, 2);
        let values: Vec<u8> = out.pixels().map(|p| p[0]).collect();
        assert_eq!(values, vec![2, 3, 4, 0, 1]);
    }

    #[test]
    fn fast_path_preserves_original_bytes() {
        let img = RgbImage::from_fn(8, 8, |x, y| Rgb([(x * 8) as u8, (y * 8) as u8, 0]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 88)
            .encode(img.as_raw(), 8, 8, ExtendedColorType::Rgb8)
            .unwrap();

        let dir = std::env::temp_dir().join(format!("jm_img_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("00001.jpg");

        let saved = save_image(&jpeg, ".jpg", ".jpg", &target, "0", "1", "00001", false).unwrap();
        assert_eq!(saved.width, 8);
        assert_eq!(saved.height, 8);
        assert_eq!(std::fs::read(&target).unwrap(), jpeg);
        assert!(!saved.descrambled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn converts_when_suffix_differs() {
        let img = RgbImage::from_fn(6, 6, |x, _y| Rgb([(x * 20) as u8, 40, 60]));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), 6, 6, ExtendedColorType::Rgb8)
            .unwrap();

        let dir = std::env::temp_dir().join(format!("jm_img_conv_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("00001.jpg");

        let saved = save_image(&png, ".png", ".jpg", &target, "0", "1", "00001", true).unwrap();
        assert_eq!(saved.width, 6);
        let written = std::fs::read(&target).unwrap();
        assert!(written.starts_with(&[0xFF, 0xD8, 0xFF]), "应写出 JPEG");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mismatch_between_suffix_and_content_is_reencoded() {
        // 服务端返回 PNG 内容，但 URL/目标后缀都是 .jpg → 不应直接写原始字节
        let img = RgbImage::from_fn(5, 5, |x, _y| Rgb([(x * 40) as u8, 20, 30]));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), 5, 5, ExtendedColorType::Rgb8)
            .unwrap();

        let dir = std::env::temp_dir().join(format!("jm_img_mismatch_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("00001.jpg");

        save_image(&png, ".jpg", ".jpg", &target, "0", "1", "00001", true).unwrap();
        let written = std::fs::read(&target).unwrap();
        assert!(
            written.starts_with(&[0xFF, 0xD8, 0xFF]),
            "内容与后缀不符时应重新编码为真实 JPEG"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_jpeg_header() {
        let img = RgbImage::from_fn(13, 7, |_, _| Rgb([1, 2, 3]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80)
            .encode(img.as_raw(), 13, 7, ExtendedColorType::Rgb8)
            .unwrap();

        let header = parse_jpeg(&jpeg).expect("header");
        assert_eq!(header.width, 13);
        assert_eq!(header.height, 7);
        assert_eq!(header.components, 3);
        assert_eq!(header.orientation, 1);
        assert_eq!(exif_rotation(1), 0);
        assert_eq!(exif_rotation(6), 90);
        assert_eq!(exif_rotation(8), 270);
    }

    #[test]
    fn rejects_non_image_bytes() {
        let dir = std::env::temp_dir().join(format!("jm_img_bad_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("bad.jpg");
        assert!(save_image(
            b"not an image",
            ".jpg",
            ".jpg",
            &target,
            "0",
            "1",
            "bad",
            false
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
