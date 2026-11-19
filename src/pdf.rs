//! 无额外依赖的 PDF 生成器。
//!
//! 设计目标与 Python 版 `img2pdf` 一致：
//! - JPEG 直接以 `/DCTDecode` 嵌入，不做二次编码（几乎无损）；
//! - 其他格式解码为像素后以 `/FlateDecode` 无损嵌入；
//! - 生成后重新检查页数，校验通过才替换目标文件。

use std::fs;
use std::path::{Path, PathBuf};

use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;

use crate::error::{JmError, Result};

/// 一张待写入 PDF 的图片。
pub enum PdfImageKind {
    /// 原始 JPEG 字节（直接嵌入，不重新压缩）。
    Jpeg { data: Vec<u8>, components: u8 },
    /// 原始 RGB 像素（将被压缩后嵌入）。
    Rgb { data: Vec<u8> },
}

pub struct PdfImage {
    pub width: u32,
    pub height: u32,
    /// 页面旋转角度：0 / 90 / 180 / 270。
    pub rotate: u16,
    pub kind: PdfImageKind,
}

impl PdfImage {
    pub fn jpeg(data: Vec<u8>, width: u32, height: u32, components: u8, rotate: u16) -> Self {
        Self {
            width,
            height,
            rotate,
            kind: PdfImageKind::Jpeg { data, components },
        }
    }

    pub fn rgb(data: Vec<u8>, width: u32, height: u32, rotate: u16) -> Self {
        Self {
            width,
            height,
            rotate,
            kind: PdfImageKind::Rgb { data },
        }
    }
}

/// 图片像素 → PDF 点（与 img2pdf 默认 96 DPI 一致）。
fn px_to_pt(pixels: u32) -> f32 {
    pixels as f32 * 72.0 / 96.0
}

struct ObjectWriter {
    buf: Vec<u8>,
    offsets: Vec<usize>,
}

impl ObjectWriter {
    fn new(object_count: usize) -> Self {
        Self {
            buf: Vec::with_capacity(object_count * 2048),
            offsets: vec![0; object_count + 1],
        }
    }

    fn begin(&mut self, id: u32) {
        let index = id as usize;
        if index < self.offsets.len() {
            self.offsets[index] = self.buf.len();
        }
        self.write(format!("{id} 0 obj\n"));
    }

    fn end(&mut self) {
        self.write("\nendobj\n");
    }

    fn write(&mut self, text: impl AsRef<[u8]>) {
        self.buf.extend_from_slice(text.as_ref());
    }
}

/// 构造 PDF 字节流。
pub fn build_pdf(pages: &[PdfImage], title: &str) -> Result<Vec<u8>> {
    if pages.is_empty() {
        return Err(JmError::Pdf("没有可用于生成 PDF 的图片".to_string()));
    }

    let page_count = pages.len();
    let object_count = 3 + page_count * 3;
    // 1: Catalog, 2: Pages, 3: Info, 之后每页 3 个对象
    let mut writer = ObjectWriter::new(object_count);

    // 头部（第二行的二进制注释可让部分工具正确识别为二进制文件）
    writer.write(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

    // 1 - Catalog
    writer.begin(1);
    writer.write("<< /Type /Catalog /Pages 2 0 R >>");
    writer.end();

    // 2 - Pages
    writer.begin(2);
    let kids: Vec<String> = (0..page_count)
        .map(|index| format!("{} 0 R", 4 + index * 3))
        .collect();
    writer.write(format!(
        "<< /Type /Pages /Count {page_count} /Kids [{}] >>",
        kids.join(" ")
    ));
    writer.end();

    // 3 - Info
    writer.begin(3);
    writer.write(format!(
        "<< /Title {} /Creator ({}) /Producer (jmcomic-rs {}) >>",
        pdf_text_string(title),
        escape_pdf_string(crate::config::APP_NAME),
        crate::config::APP_VERSION
    ));
    writer.end();

    for (index, page) in pages.iter().enumerate() {
        let page_id = 4 + index as u32 * 3;
        let content_id = page_id + 1;
        let image_id = page_id + 2;

        // 页面
        writer.begin(page_id);
        writer.write(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] \
             /Resources << /XObject << /Im0 {image_id} 0 R >> >> /Contents {content_id} 0 R",
            px_to_pt(page.width),
            px_to_pt(page.height)
        ));
        if matches!(page.rotate, 90 | 180 | 270) {
            writer.write(format!(" /Rotate {}", page.rotate));
        }
        writer.write(" >>");
        writer.end();

        // 内容流：把图片铺满页面
        writer.begin(content_id);
        let content = format!(
            "q\n{:.2} 0 0 {:.2} 0 0 cm\n/Im0 Do\nQ\n",
            px_to_pt(page.width),
            px_to_pt(page.height)
        );
        writer.write(format!("<< /Length {} >>\nstream\n", content.len()));
        writer.write(content);
        writer.write("endstream");
        writer.end();

        // 图片 XObject
        writer.begin(image_id);
        match &page.kind {
            PdfImageKind::Jpeg { data, components } => {
                let (color_space, decode) = match components {
                    1 => ("/DeviceGray", ""),
                    4 => ("/DeviceCMYK", " /Decode [1 0 1 0 1 0 1 0]"),
                    _ => ("/DeviceRGB", ""),
                };
                writer.write(format!(
                    "<< /Type /XObject /Subtype /Image /Width {} /Height {} \
                     /ColorSpace {color_space} /BitsPerComponent 8 /Filter /DCTDecode{decode} \
                     /Length {} >>\nstream\n",
                    page.width,
                    page.height,
                    data.len()
                ));
                writer.write(data);
                writer.write("\nendstream");
                writer.end();
            }
            PdfImageKind::Rgb { data } => {
                let expected = page.width as usize * page.height as usize * 3;
                if data.len() < expected {
                    return Err(JmError::Pdf(format!(
                        "第 {} 页像素数据长度不足: {} < {}",
                        index + 1,
                        data.len(),
                        expected
                    )));
                }
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
                encoder
                    .write_all(&data[..expected])
                    .map_err(|e| JmError::Pdf(format!("像素压缩失败: {e}")))?;
                let compressed = encoder
                    .finish()
                    .map_err(|e| JmError::Pdf(format!("像素压缩失败: {e}")))?;

                writer.write(format!(
                    "<< /Type /XObject /Subtype /Image /Width {} /Height {} \
                     /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode \
                     /Length {} >>\nstream\n",
                    page.width,
                    page.height,
                    compressed.len()
                ));
                writer.write(compressed);
                writer.write("\nendstream");
                writer.end();
            }
        }
    }

    // xref
    let xref_offset = writer.buf.len();
    writer.write(format!("xref\n0 {}\n", object_count + 1));
    writer.write("0000000000 65535 f \n");
    for id in 1..=object_count {
        let offset = writer.offsets[id];
        writer.write(format!("{offset:010} 00000 n \n"));
    }
    writer.write(format!(
        "trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
        object_count + 1
    ));

    Ok(writer.buf)
}

/// 重新解析 PDF，统计页数（用于生成后的校验）。
pub fn count_pages(bytes: &[u8]) -> usize {
    const NEEDLE: &[u8] = b"/Type /Page";
    let mut count = 0usize;
    let mut start = 0usize;
    while let Some(pos) = find_from(bytes, NEEDLE, start) {
        let after = pos + NEEDLE.len();
        let next = bytes.get(after).copied();
        if next != Some(b's') {
            count += 1;
        }
        start = after;
    }
    count
}

fn find_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|pos| pos + from)
}

/// 生成 PDF 并校验页数；校验失败时删除临时文件并返回错误。
pub fn write_verified_pdf(pages: &[PdfImage], target: &Path, title: &str) -> Result<usize> {
    let bytes = build_pdf(pages, title)?;
    let expected = pages.len();

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            JmError::Io(std::io::Error::new(
                e.kind(),
                format!("无法创建 PDF 目录 {}: {e}", parent.display()),
            ))
        })?;
    }

    let file_name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "output.pdf".to_string());
    let temp: PathBuf = target.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));

    fs::write(&temp, &bytes).map_err(|e| {
        JmError::Io(std::io::Error::new(
            e.kind(),
            format!("写入 PDF 临时文件失败 {}: {e}", temp.display()),
        ))
    })?;

    let written = fs::read(&temp).unwrap_or_default();
    let pages_found = count_pages(&written);
    if pages_found != expected {
        let _ = fs::remove_file(&temp);
        return Err(JmError::Pdf(format!(
            "PDF 页数校验失败：应有 {expected} 页，实际 {pages_found} 页"
        )));
    }

    // 目标文件可能已存在，使用 replace 语义
    if let Err(err) = fs::rename(&temp, target) {
        // 某些 Windows 场景下 rename 无法覆盖，退化为先删除再重命名
        if target.exists() {
            let _ = fs::remove_file(target);
        }
        if let Err(second) = fs::rename(&temp, target) {
            let _ = fs::remove_file(&temp);
            return Err(JmError::Io(std::io::Error::new(
                err.kind(),
                format!("替换 PDF 文件失败 {}: {second}", target.display()),
            )));
        }
    }

    Ok(expected)
}

fn escape_pdf_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' | '\r' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

/// PDF 文本字符串：使用 UTF-16BE（带 BOM）以支持中文标题。
fn pdf_text_string(text: &str) -> String {
    let mut bytes: Vec<u8> = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    let mut out = String::with_capacity(bytes.len() * 2 + 2);
    out.push('<');
    for byte in bytes {
        out.push_str(&format!("{byte:02X}"));
    }
    out.push('>');
    out
}

/// 从文件读取图片，转换为 PDF 页源。
pub fn image_from_file(path: &Path) -> Result<PdfImage> {
    let bytes = fs::read(path).map_err(|e| {
        JmError::Io(std::io::Error::new(
            e.kind(),
            format!("读取图片失败 {}: {e}", path.display()),
        ))
    })?;
    if bytes.is_empty() {
        return Err(JmError::Pdf(format!("图片文件为空: {}", path.display())));
    }

    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        if let Some(info) = crate::imageio::parse_jpeg(&bytes) {
            let rotate = crate::imageio::exif_rotation(info.orientation);
            return Ok(PdfImage::jpeg(
                bytes,
                info.width,
                info.height,
                info.components,
                rotate,
            ));
        }
    }

    // 非 JPEG：解码为像素后无损压缩嵌入（WebP 走 libwebp 解码）
    let decoded = crate::imageio::decode_rgb(&bytes)
        .map_err(|e| JmError::Pdf(format!("图片解码失败 {}: {e}", path.display())))?;
    let (width, height) = decoded.dimensions();
    Ok(PdfImage::rgb(decoded.into_raw(), width, height, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_jpeg() -> Vec<u8> {
        let img = image::RgbImage::from_fn(24, 16, |x, y| {
            image::Rgb([(x * 10) as u8, (y * 15) as u8, 128])
        });
        let mut out = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
        encoder
            .encode(img.as_raw(), 24, 16, image::ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    #[test]
    fn builds_pdf_with_dct_and_flate_pages() {
        let jpeg = sample_jpeg();
        let info = crate::imageio::parse_jpeg(&jpeg).expect("jpeg header");
        assert_eq!(info.width, 24);
        assert_eq!(info.height, 16);
        assert_eq!(info.components, 3);

        let rgb = vec![255u8; 8 * 8 * 3];
        let pages = vec![
            PdfImage::jpeg(jpeg, 24, 16, 3, 0),
            PdfImage::rgb(rgb, 8, 8, 90),
        ];

        let bytes = build_pdf(&pages, "测试标题").unwrap();
        assert!(bytes.starts_with(b"%PDF-1.7"));
        assert!(bytes.ends_with(b"%%EOF\n"));
        assert_eq!(count_pages(&bytes), 2);

        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/DCTDecode"));
        assert!(text.contains("/FlateDecode"));
        assert!(text.contains("/Rotate 90"));
        assert!(text.contains("startxref"));
        // 中文标题以 UTF-16BE 十六进制形式写入
        assert!(text.contains("FEFF"));
    }

    #[test]
    fn writes_and_verifies_file() {
        let dir = std::env::temp_dir().join(format!("jm_pdf_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("样例.pdf");

        let pages = vec![PdfImage::rgb(vec![10u8; 4 * 4 * 3], 4, 4, 0)];
        let count = write_verified_pdf(&pages, &target, "样例").unwrap();
        assert_eq!(count, 1);

        let read_back = fs::read(&target).unwrap();
        assert_eq!(count_pages(&read_back), 1);
        // 临时文件不应残留
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_empty_page_list() {
        assert!(build_pdf(&[], "x").is_err());
    }

    #[test]
    fn page_count_ignores_pages_dict() {
        let pages = vec![
            PdfImage::rgb(vec![0u8; 3], 1, 1, 0),
            PdfImage::rgb(vec![0u8; 3], 1, 1, 0),
            PdfImage::rgb(vec![0u8; 3], 1, 1, 0),
        ];
        let bytes = build_pdf(&pages, "t").unwrap();
        assert_eq!(count_pages(&bytes), 3);
    }
}
