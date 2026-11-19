//! 图片管线微基准：与 Python(Pillow) 版同口径对比。
//!
//! 单独运行（自动生成测试图片）：
//!     cargo run --release --example pipeline_bench
//!
//! 与 Python 基线共用同一批图片（推荐，结果严格可比）：
//!     .\.venv\Scripts\python.exe rust\benchmarks\python_baseline.py --make-fixture %TEMP%\jm_bench_fixture
//!     $env:JM_BENCH_DIR = "$env:TEMP\jm_bench_fixture"
//!     cargo run --release --example pipeline_bench
//!     .\.venv\Scripts\python.exe rust\benchmarks\python_baseline.py %TEMP%\jm_bench_fixture
//!
//! 三个场景：
//! 1. 快速路径：无需解密、格式不变 → 直接写原始字节（Python 版仍会解码+重编码）；
//! 2. 解密路径：竖切重排后编码为 WebP 无损（等价于 Pillow 的 crop/paste + save）；
//! 3. PDF：20 页 JPEG 直接嵌入（DCTDecode，无重编码）。

use std::path::{Path, PathBuf};
use std::time::Instant;

use jmcomic_dl::{imageio, pdf};

/// 与正式程序一致的分配器，保证基准可比。
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn make_jpeg(width: u32, height: u32, quality: u8) -> Vec<u8> {
    let img = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([
            ((x * 7 + y * 3) % 255) as u8,
            ((y * 5 + x) % 255) as u8,
            ((x + y) % 255) as u8,
        ])
    });
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode(img.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .expect("encode jpeg");
    out
}

/// 准备测试图片：优先使用 `JM_BENCH_DIR` 下的共享 fixture，否则本地生成。
fn prepare_fixture() -> (PathBuf, Vec<u8>, Vec<PathBuf>) {
    let dir = match std::env::var("JM_BENCH_DIR") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => PathBuf::from(std::env::temp_dir()).join(format!("jm_bench_{}", std::process::id())),
    };
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::create_dir_all(dir.join("pages"));

    let source = dir.join("source.jpg");
    if !source.is_file() {
        std::fs::write(&source, make_jpeg(1200, 1700, 95)).expect("write fixture");
    }

    let mut pages = Vec::new();
    for index in 0..20u8 {
        let page = dir.join("pages").join(format!("p{index:02}.jpg"));
        if !page.is_file() {
            std::fs::write(&page, make_jpeg(1200, 1700, 90 + index % 10)).expect("write page");
        }
        pages.push(page);
    }

    let bytes = std::fs::read(&source).expect("read fixture");
    (dir, bytes, pages)
}

fn main() {
    let (dir, jpeg, page_files) = prepare_fixture();
    let header = imageio::parse_jpeg(&jpeg).expect("jpeg header");
    let (width, height) = (header.width, header.height);
    println!(
        "测试图片：{width}x{height} JPEG，{} KB（目录 {}）",
        jpeg.len() / 1024,
        dir.display()
    );

    let iterations = 30;

    // 场景 1：快速路径（无需解密，保持原格式）
    let target = dir.join("fast.jpg");
    let start = Instant::now();
    for _ in 0..iterations {
        imageio::save_image(
            &jpeg, ".jpg", ".jpg", &target, "421926", "100000", "00001", true,
        )
        .expect("fast path");
    }
    let fast = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;

    // 场景 2：解密路径（竖切 10 段 + WebP 无损重编码）
    let scrambled = dir.join("scrambled.webp");
    let start = Instant::now();
    for _ in 0..iterations {
        imageio::save_image(
            &jpeg, ".jpg", ".webp", &scrambled, "220980", "500000", "00001", true,
        )
        .expect("descramble path");
    }
    let descramble = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;
    let webp_bytes = std::fs::metadata(&scrambled).map(|m| m.len()).unwrap_or(0);

    // 场景 3：PDF（20 页 JPEG 直接嵌入）
    let pages: Vec<pdf::PdfImage> = page_files
        .iter()
        .map(|path| pdf::image_from_file(path).expect("pdf image"))
        .collect();
    let pdf_path = dir.join("bench.pdf");
    let start = Instant::now();
    pdf::write_verified_pdf(&pages, &pdf_path, "bench").expect("pdf");
    let pdf_ms = start.elapsed().as_secs_f64() * 1000.0;
    let pdf_bytes = std::fs::metadata(&pdf_path).map(|m| m.len()).unwrap_or(0);

    println!("快速路径（直写原字节）：{fast:.3} ms/张");
    println!(
        "解密路径（竖切+WebP 无损）：{descramble:.2} ms/张，{} KB",
        webp_bytes / 1024
    );
    println!(
        "PDF 20 页 JPEG 直嵌：{pdf_ms:.1} ms，{} MB",
        pdf_bytes / 1024 / 1024
    );

    // 仅清理自动生成的临时目录
    if std::env::var("JM_BENCH_DIR").is_err() {
        let _ = std::fs::remove_dir_all(Path::new(&dir));
    }
}
