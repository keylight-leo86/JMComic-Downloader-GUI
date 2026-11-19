//! 图片编解码微基准：量化解码 / 解密 / 编码各阶段耗时。
//!
//! 运行：`cargo run --release --example image_bench`

use std::time::Instant;

use jmcomic_dl::imageio;

fn make_image(width: u32, height: u32) -> image::RgbImage {
    image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([
            ((x * 7 + y * 3) % 255) as u8,
            ((y * 5 + x) % 255) as u8,
            ((x + y) % 255) as u8,
        ])
    })
}

fn encode(img: &image::RgbImage, format: image::ImageFormat, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    match format {
        image::ImageFormat::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
                .encode(
                    img.as_raw(),
                    img.width(),
                    img.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .unwrap();
        }
        image::ImageFormat::WebP => {
            image::codecs::webp::WebPEncoder::new_lossless(&mut out)
                .encode(
                    img.as_raw(),
                    img.width(),
                    img.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .unwrap();
        }
        _ => panic!("unsupported"),
    }
    out
}

fn main() {
    let (w, h) = (1200u32, 1700u32);
    let iterations = 20;
    let img = make_image(w, h);

    let jpeg = encode(&img, image::ImageFormat::Jpeg, 95);
    let webp = encode(&img, image::ImageFormat::WebP, 100);
    println!(
        "源图 {w}x{h}：JPEG {} KB / WebP {} KB",
        jpeg.len() / 1024,
        webp.len() / 1024
    );

    for (label, bytes) in [("JPEG", &jpeg), ("WebP", &webp)] {
        let start = Instant::now();
        let mut decoded = None;
        for _ in 0..iterations {
            decoded = Some(image::load_from_memory(bytes).unwrap().to_rgb8());
        }
        let decode_ms = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;

        let rgb = decoded.unwrap();
        let start = Instant::now();
        for _ in 0..iterations {
            std::hint::black_box(imageio::descramble_rgb(&rgb, 10));
        }
        let descramble_ms = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;

        let start = Instant::now();
        for _ in 0..iterations {
            std::hint::black_box(encode(&rgb, image::ImageFormat::WebP, 100));
        }
        let encode_webp_ms = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;

        let start = Instant::now();
        for _ in 0..iterations {
            std::hint::black_box(encode(&rgb, image::ImageFormat::Jpeg, 95));
        }
        let encode_jpeg_ms = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;

        // 生产路径（mozjpeg / libwebp）
        let start = Instant::now();
        for _ in 0..iterations {
            std::hint::black_box(
                imageio::encode_rgb_image(rgb.as_raw(), rgb.width(), rgb.height(), ".jpg").unwrap(),
            );
        }
        let native_jpeg_ms = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;

        let start = Instant::now();
        for _ in 0..iterations {
            std::hint::black_box(
                imageio::encode_rgb_image(rgb.as_raw(), rgb.width(), rgb.height(), ".webp")
                    .unwrap(),
            );
        }
        let native_webp_ms = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;

        println!(
            "{label}: 解码 {decode_ms:.2} ms / 竖切 {descramble_ms:.2} ms / 编码WebP(纯Rust) {encode_webp_ms:.2} ms / 编码JPEG(纯Rust) {encode_jpeg_ms:.2} ms"
        );
        println!("       实际路径: JPEG {native_jpeg_ms:.2} ms / WebP {native_webp_ms:.2} ms");
    }
}
