//! PDF 回环校验工具：生成一个包含 RGB（FlateDecode）与 JPEG（DCTDecode）页面的 PDF，
//! 供外部解析器（例如 pypdf）校验像素与页数。
//!
//! 运行：`cargo run --release --example pdf_roundtrip`
//! 输出：`%TEMP%\jm_pdf_roundtrip.pdf`

use std::path::PathBuf;

use jmcomic_dl::pdf::{self, PdfImage};

fn main() {
    // 4x2 的已知图案：每行颜色固定，便于外部逐像素核对
    let mut rgb = Vec::with_capacity(4 * 2 * 3);
    for y in 0..2u8 {
        for x in 0..4u8 {
            rgb.extend_from_slice(&[x * 60, y * 200, 128]);
        }
    }

    let jpeg = {
        let img = image::RgbImage::from_fn(8, 6, |x, y| {
            image::Rgb([(x * 20) as u8, (y * 30) as u8, 200])
        });
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 100)
            .encode(img.as_raw(), 8, 6, image::ExtendedColorType::Rgb8)
            .expect("encode jpeg");
        out
    };

    let pages = vec![
        PdfImage::rgb(rgb, 4, 2, 0),
        PdfImage::jpeg(jpeg, 8, 6, 3, 90),
    ];

    let target: PathBuf = std::env::temp_dir().join("jm_pdf_roundtrip.pdf");
    let count = pdf::write_verified_pdf(&pages, &target, "roundtrip").expect("write pdf");
    println!("pages={count} path={}", target.display());
}
