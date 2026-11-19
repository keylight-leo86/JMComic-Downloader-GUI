//! `--self-test`：不联网的自检，覆盖加密、图片解密、PDF 生成与校验。

use std::path::{Path, PathBuf};

use crate::config::ImageFormat;
use crate::crypto;
use crate::error::{brief, Result};
use crate::imageio;
use crate::naming;
use crate::pdf;

fn temp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("jmcomic_selftest_{tag}_{}", std::process::id()))
}

/// 执行自检，返回检查项说明。
pub fn run() -> Result<Vec<String>> {
    let mut checked = Vec::new();

    // 1. MD5 / token
    let (token, tokenparam) =
        crypto::token_and_tokenparam(1700566805, "2.0.28", "185Hcomic3PAPP7R");
    if token.len() != 32 || tokenparam != "1700566805,2.0.28" {
        return crate::error::JmError::msg("token 计算异常");
    }
    checked.push(format!("token: OK ({token})"));

    // 2. 文件名清洗 / 车号解析
    let ids = naming::parse_ids("JM1455254, https://18comic.vip/album/99/")?;
    if ids != vec!["1455254".to_string(), "99".to_string()] {
        return crate::error::JmError::msg("车号解析异常");
    }
    checked.push("车号解析: OK".to_string());

    // 3. 生成测试图片
    let dir = temp_dir("images");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| crate::error::JmError::Msg(format!("无法创建临时目录: {e}")))?;

    let source = image::RgbImage::from_fn(64, 96, |x, y| {
        image::Rgb([(x * 3) as u8, (y * 2) as u8, ((x + y) % 255) as u8])
    });
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 92)
        .encode(source.as_raw(), 64, 96, image::ExtendedColorType::Rgb8)
        .map_err(|e| crate::error::JmError::Image(format!("生成测试 JPEG 失败: {e}")))?;

    // 4. 快速路径（原始字节落盘）
    let target = dir.join("00001.jpg");
    let saved = imageio::save_image(&jpeg, ".jpg", ".jpg", &target, "0", "100", "00001", false)?;
    let written = std::fs::read(&target)
        .map_err(|e| crate::error::JmError::Msg(format!("回读图片失败: {e}")))?;
    if written != jpeg {
        return crate::error::JmError::msg("图片快速路径写入后字节不一致");
    }
    checked.push(format!(
        "图片保存（无损直写）: OK ({}x{})",
        saved.width, saved.height
    ));

    // 5. 解密路径（切割重排）
    let scrambled = dir.join("00002.png");
    imageio::save_image(
        &jpeg, ".jpg", ".png", &scrambled, "220980", "500000", "00002", true,
    )?;
    let decoded = image::open(&scrambled)
        .map_err(|e| crate::error::JmError::Image(format!("回读解密图片失败: {e}")))?;
    if decoded.width() != 64 || decoded.height() != 96 {
        return crate::error::JmError::msg("解密后的图片尺寸不正确");
    }
    checked.push("图片解密（竖切重排）: OK".to_string());

    // 6. PDF 生成与页数校验
    let pages = vec![
        pdf::image_from_file(&target)?,
        pdf::image_from_file(&scrambled)?,
    ];
    let pdf_path = dir.join("自检.pdf");
    let count = pdf::write_verified_pdf(&pages, &pdf_path, "自检文档")?;
    if count != 2 {
        return crate::error::JmError::msg("PDF 页数校验失败");
    }
    let bytes = std::fs::read(&pdf_path)
        .map_err(|e| crate::error::JmError::Msg(format!("回读 PDF 失败: {e}")))?;
    if pdf::count_pages(&bytes) != 2 {
        return crate::error::JmError::msg("PDF 回读页数不一致");
    }
    checked.push(format!(
        "PDF 生成与校验: OK ({} 页, {} 字节, {})",
        count,
        bytes.len(),
        ImageFormat::Original.label()
    ));

    let _ = std::fs::remove_dir_all(&dir);

    Ok(checked)
}

/// 自检并写入日志文件（GUI/安装包冒烟测试使用）。
pub fn run_to_file(log_file: Option<&Path>) -> i32 {
    let mut lines: Vec<String> = Vec::new();
    let code = match run() {
        Ok(checked) => {
            lines.push(format!("self-test: OK ({} 项检查通过)", checked.len()));
            lines.extend(checked);
            0
        }
        Err(err) => {
            lines.push("self-test: FAILED".to_string());
            lines.push(brief(&err));
            1
        }
    };

    if let Some(path) = log_file {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(path, lines.join("\n")).is_err() {
            return 2;
        }
    } else {
        for line in &lines {
            println!("{line}");
        }
    }

    code
}
