//! JMComic Downloader GUI（Rust 版）入口。
//!
//! 用法：
//! - 无参数：启动图形界面；
//! - `--cli --kind album --id 1455254 --output D:\下载`：命令行模式；
//! - `--self-test`：离线自检（不联网）。

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use jmcomic_dl::config::{self, ClientKind, ImageFormat, Kind};
use jmcomic_dl::download::{self, JobSpec};
use jmcomic_dl::events::{log, LogSink, Progress, StopFlag, Summary};
use jmcomic_dl::option;
use jmcomic_dl::settings::Settings;

/// 更快的通用分配器：图片解码/编码会产生大量小块分配。
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Debug, Parser)]
#[command(
    name = "JMComic-Downloader-GUI",
    version,
    about = "JMComic 批量下载器（Rust）：多级并发下载、近乎无损 PDF、自动清理图片",
    long_about = None
)]
struct Cli {
    /// 不启动图形界面，直接执行下载任务
    #[arg(long)]
    cli: bool,

    /// 离线自检：不联网验证加密、图片解密与 PDF 生成
    #[arg(long = "self-test")]
    self_test: bool,

    /// 下载类型：album（本子）或 photo（章节）
    #[arg(long, value_name = "album|photo")]
    kind: Option<String>,

    /// 车号或链接，可重复传入；一个参数内也可用空格/逗号分隔多个
    #[arg(long = "id", value_name = "ID")]
    ids: Vec<String>,

    /// 保存位置
    #[arg(long, value_name = "DIR")]
    output: Option<PathBuf>,

    /// 客户端实现：api（默认）或 html
    #[arg(long)]
    client: Option<String>,

    /// 图片格式：original / jpg / png / webp
    #[arg(long = "image-format", value_name = "FORMAT")]
    image_format: Option<String>,

    /// 图片并发（1-128）
    #[arg(long = "image-threads", value_name = "N")]
    image_threads: Option<usize>,

    /// 章节并发（1-64）
    #[arg(long = "photo-threads", value_name = "N")]
    photo_threads: Option<usize>,

    /// 本子并发（1-32）
    #[arg(long = "album-threads", value_name = "N")]
    album_threads: Option<usize>,

    /// 代理：system / none / host:port
    #[arg(long, value_name = "PROXY")]
    proxy: Option<String>,

    /// 可选 YAML 配置
    #[arg(long, value_name = "FILE")]
    option: Option<PathBuf>,

    /// 额外把日志写入文件
    #[arg(long = "log-file", value_name = "FILE")]
    log_file: Option<PathBuf>,

    /// 自定义接口域名（可重复；用 host、host:port 或完整 URL）
    #[arg(long = "api-domain", value_name = "DOMAIN")]
    api_domains: Vec<String>,

    /// 自定义图片 CDN 域名（可重复）
    #[arg(long = "image-domain", value_name = "DOMAIN")]
    image_domains: Vec<String>,
}

fn main() {
    let cli = Cli::parse();

    if cli.self_test {
        let code = jmcomic_dl::selftest::run_to_file(cli.log_file.as_deref());
        std::process::exit(code);
    }

    let wants_job = cli.cli || !cli.ids.is_empty();
    if wants_job {
        std::process::exit(run_headless(cli));
    }

    #[cfg(feature = "gui")]
    {
        match jmcomic_dl::gui::launch() {
            Ok(()) => std::process::exit(0),
            Err(err) => {
                eprintln!("图形界面启动失败：{err}");
                eprintln!("可以改用命令行模式：--cli --kind album --id 车号 --output 目录");
                std::process::exit(1);
            }
        }
    }

    #[cfg(not(feature = "gui"))]
    {
        eprintln!("本构建未包含图形界面，请使用：--cli --kind album --id 车号 --output 目录");
        std::process::exit(1);
    }
}

fn run_headless(cli: Cli) -> i32 {
    let settings = Settings::load();

    // 1. 收集车号
    let raw_ids = if cli.ids.is_empty() {
        String::new()
    } else {
        cli.ids.join(" ")
    };
    let ids = match jmcomic_dl::naming::parse_ids(&raw_ids) {
        Ok(ids) => ids,
        Err(err) => {
            eprintln!("{err}");
            return 64;
        }
    };

    // 2. 组装参数
    let kind = match cli.kind.as_deref() {
        Some(text) => match text.trim().to_ascii_lowercase().as_str() {
            "album" => Kind::Album,
            "photo" => Kind::Photo,
            other => {
                eprintln!("未知的下载类型：{other}（应为 album 或 photo）");
                return 64;
            }
        },
        None => {
            if settings.mode == "photo" {
                Kind::Photo
            } else {
                Kind::Album
            }
        }
    };

    let output = cli
        .output
        .clone()
        .unwrap_or_else(|| settings.initial_output_directory());
    let output = if output.is_absolute() {
        output
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(output)
    };

    let mut spec = JobSpec {
        kind,
        ids: ids.clone(),
        output_dir: output.clone(),
        client: cli
            .client
            .as_deref()
            .and_then(ClientKind::from_key)
            .unwrap_or_else(|| settings.client_kind()),
        image_format: cli
            .image_format
            .as_deref()
            .and_then(ImageFormat::from_key)
            .unwrap_or_else(|| settings.image_format()),
        image_concurrency: cli.image_threads.unwrap_or(settings.image_threads),
        photo_concurrency: cli.photo_threads.unwrap_or(settings.photo_threads),
        album_concurrency: cli.album_threads.unwrap_or(settings.album_threads),
        proxy: cli.proxy.clone().unwrap_or_else(|| settings.proxy.clone()),
        retry: config::DEFAULT_RETRY_TIMES,
        decode_images: true,
        download_cache: true,
        domains: if cli.api_domains.is_empty() {
            None
        } else {
            Some(cli.api_domains.clone())
        },
        image_domains: if cli.image_domains.is_empty() {
            None
        } else {
            Some(cli.image_domains.clone())
        },
    };

    // 3. YAML 覆盖
    let option_path = cli.option.clone().or_else(|| {
        let saved = settings.option.trim();
        if saved.is_empty() {
            None
        } else {
            Some(PathBuf::from(saved))
        }
    });

    if let Some(path) = &option_path {
        match option::load(path) {
            Ok((overrides, notes)) => {
                println!("已载入高级配置：{}", path.display());
                for note in notes {
                    println!("提示：{note}");
                }
                overrides.apply_to(&mut spec);
            }
            Err(err) => {
                eprintln!("读取高级配置失败：{err}");
                return 64;
            }
        }
    }

    spec.image_concurrency = spec.image_concurrency.clamp(1, config::MAX_IMAGE_THREADS);
    spec.photo_concurrency = spec.photo_concurrency.clamp(1, config::MAX_PHOTO_THREADS);
    spec.album_concurrency = spec.album_concurrency.clamp(1, config::MAX_ALBUM_THREADS);

    let log_file = cli.log_file.clone();
    let sink: LogSink = Arc::new(move |line: String| {
        println!("{line}");
        let _ = std::io::stdout().flush();
        if let Some(path) = &log_file {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{line}");
            }
        }
    });

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("创建异步运行时失败：{err}");
            return 1;
        }
    };

    let progress = Progress::new();
    let stop = StopFlag::new();
    let summary = runtime.block_on(download::run(spec, sink.clone(), stop, progress));

    report(&sink, &summary)
}

fn report(sink: &LogSink, summary: &Summary) -> i32 {
    if summary.failures.is_empty() {
        log(
            sink,
            format!(
                "完成：本子 {}，章节 {}，图片 {}，PDF {}。",
                summary.albums, summary.photos, summary.images, summary.pdfs
            ),
        );
        return 0;
    }

    log(
        sink,
        format!(
            "存在 {} 项失败（本子 {}，章节 {}，图片 {}，PDF {}）。",
            summary.failures.len(),
            summary.albums,
            summary.photos,
            summary.images,
            summary.pdfs
        ),
    );
    for failure in summary.failures.iter().take(50) {
        log(sink, format!("  {}：{}", failure.scope, failure.error));
    }
    2
}
