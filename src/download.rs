//! 下载编排：本子 → 章节 → 图片 → PDF → 清理。
//!
//! 并发模型（比 Python 版更细粒度）：
//! - 本子级并发：同时处理多个车号；
//! - 章节级并发：同一个本子的多个章节并行；
//! - 图片级并发：同一章节的图片并行；
//! - CPU 密集的解密/编码/PDF 放进 `spawn_blocking`，不阻塞异步调度。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::api::JmClient;
use crate::config::{self, ClientKind, ImageFormat, Kind};
use crate::error::{brief, JmError, Result};
use crate::events::{log, Failure, LogSink, Progress, StopFlag, Summary};
use crate::imageio;
use crate::naming;
use crate::pdf;
use crate::types::{Album, Episode, Photo};

/// 一次下载任务的完整参数。
#[derive(Debug, Clone)]
pub struct JobSpec {
    pub kind: Kind,
    pub ids: Vec<String>,
    pub output_dir: PathBuf,
    pub client: ClientKind,
    pub image_format: ImageFormat,
    pub image_concurrency: usize,
    pub photo_concurrency: usize,
    pub album_concurrency: usize,
    pub proxy: String,
    pub retry: u32,
    pub decode_images: bool,
    pub download_cache: bool,
    /// 自定义接口域名（为空时自动探测/使用内置列表）
    pub domains: Option<Vec<String>>,
    /// 自定义图片 CDN 域名（为空时使用内置列表）
    pub image_domains: Option<Vec<String>>,
}

impl Default for JobSpec {
    fn default() -> Self {
        Self {
            kind: Kind::Album,
            ids: Vec::new(),
            output_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            client: ClientKind::Api,
            image_format: ImageFormat::Original,
            image_concurrency: config::DEFAULT_IMAGE_THREADS,
            photo_concurrency: config::DEFAULT_PHOTO_THREADS,
            album_concurrency: config::DEFAULT_ALBUM_THREADS,
            proxy: config::PROXY_SYSTEM.to_string(),
            retry: config::DEFAULT_RETRY_TIMES,
            decode_images: true,
            download_cache: true,
            domains: None,
            image_domains: None,
        }
    }
}

/// 同时进行的 PDF 生成数（CPU + 磁盘密集，2 个足够跑满带宽）。
const PDF_CONCURRENCY: usize = 2;

#[derive(Default)]
struct Stats {
    albums: usize,
    photos: usize,
    images: usize,
    pdfs: usize,
    failures: Vec<Failure>,
}

struct Shared {
    spec: JobSpec,
    client: Arc<JmClient>,
    sink: LogSink,
    stop: StopFlag,
    progress: Arc<Progress>,
    output_root: PathBuf,
    pdf_dir: PathBuf,
    photo_sem: Arc<Semaphore>,
    image_sem: Arc<Semaphore>,
    /// 限制同时生成的 PDF 数：每个 PDF 会顺序读取整章图片，并发过高会让磁盘抖动。
    pdf_sem: Arc<Semaphore>,
    stats: Mutex<Stats>,
}

impl Shared {
    fn is_stopped(&self) -> bool {
        self.stop.is_stopped()
    }

    fn fail(&self, scope: impl Into<String>, error: impl Into<String>) {
        let scope = scope.into();
        let error = error.into();
        log(&self.sink, format!("[失败] {scope}：{error}"));
        if let Ok(mut stats) = self.stats.lock() {
            stats.failures.push(Failure { scope, error });
        }
    }

    fn bump<F>(&self, update: F)
    where
        F: FnOnce(&mut Stats),
    {
        if let Ok(mut stats) = self.stats.lock() {
            update(&mut stats);
        }
    }

    fn failure_count(&self) -> usize {
        self.stats
            .lock()
            .map(|stats| stats.failures.len())
            .unwrap_or(0)
    }
}

/// 运行下载任务，失败项写入 `Summary.failures`，不会静默丢失。
pub async fn run(spec: JobSpec, sink: LogSink, stop: StopFlag, progress: Arc<Progress>) -> Summary {
    let error_sink = sink.clone();
    match run_inner(spec, sink, stop, progress).await {
        Ok(summary) => summary,
        Err(err) => {
            log(&error_sink, format!("任务异常结束：{}", brief(&err)));
            Summary {
                failures: vec![Failure {
                    scope: "任务".to_string(),
                    error: brief(&err),
                }],
                ..Default::default()
            }
        }
    }
}

async fn run_inner(
    spec: JobSpec,
    sink: LogSink,
    stop: StopFlag,
    progress: Arc<Progress>,
) -> Result<Summary> {
    let output_root = spec.output_dir.clone();
    std::fs::create_dir_all(&output_root).map_err(|e| {
        JmError::Io(std::io::Error::new(
            e.kind(),
            format!("无法创建保存目录 {}: {e}", output_root.display()),
        ))
    })?;
    let pdf_dir = output_root.join("PDF");
    std::fs::create_dir_all(&pdf_dir).map_err(|e| {
        JmError::Io(std::io::Error::new(
            e.kind(),
            format!("无法创建 PDF 目录 {}: {e}", pdf_dir.display()),
        ))
    })?;

    log(
        &sink,
        format!(
            "启动 JMComic 下载核心（Rust {}，客户端 {}）",
            config::APP_VERSION,
            spec.client.key()
        ),
    );
    log(&sink, format!("下载类型：{}", spec.kind.label()));
    log(&sink, format!("任务 ID：{}", spec.ids.join(", ")));
    log(&sink, format!("保存位置：{}", output_root.display()));
    log(&sink, format!("PDF 目录：{}", pdf_dir.display()));
    log(
        &sink,
        format!(
            "并发配置：本子 {} / 章节 {} / 图片 {}",
            spec.album_concurrency, spec.photo_concurrency, spec.image_concurrency
        ),
    );

    let client = JmClient::new(
        spec.client,
        &spec.proxy,
        spec.retry,
        sink.clone(),
        spec.domains.clone(),
        spec.image_domains.clone(),
    )
    .await?;

    progress.set_albums_total(spec.ids.len());

    let shared = Arc::new(Shared {
        photo_sem: Arc::new(Semaphore::new(spec.photo_concurrency.max(1))),
        image_sem: Arc::new(Semaphore::new(spec.image_concurrency.max(1))),
        pdf_sem: Arc::new(Semaphore::new(PDF_CONCURRENCY)),
        spec,
        client,
        sink: sink.clone(),
        stop: stop.clone(),
        progress,
        output_root,
        pdf_dir,
        stats: Mutex::new(Stats::default()),
    });

    let album_sem = Arc::new(Semaphore::new(shared.spec.album_concurrency.max(1)));
    let kind = shared.spec.kind;

    let mut tasks = JoinSet::new();
    for id in shared.spec.ids.clone() {
        let shared = shared.clone();
        let album_sem = album_sem.clone();
        tasks.spawn(async move {
            let _permit = album_sem.acquire_owned().await.ok();
            if shared.is_stopped() {
                return;
            }
            match kind {
                Kind::Album => process_album(shared.clone(), id).await,
                Kind::Photo => process_photo(shared.clone(), id, None, 0).await,
            }
        });
    }
    while tasks.join_next().await.is_some() {}

    let (summary, failures) = {
        let stats = shared.stats.lock().unwrap_or_else(|e| e.into_inner());
        (
            Summary {
                albums: stats.albums,
                photos: stats.photos,
                images: stats.images,
                pdfs: stats.pdfs,
                failures: stats.failures.clone(),
                stopped: shared.is_stopped(),
            },
            stats.failures.len(),
        )
    };

    if summary.stopped {
        log(&sink, "任务已被用户停止。");
    }
    if failures > 0 {
        log(
            &sink,
            format!(
                "任务结束，共处理 {} 项，生成 {} 个 PDF，失败 {} 项。",
                shared.spec.ids.len(),
                summary.pdfs,
                failures
            ),
        );
    } else {
        log(
            &sink,
            format!(
                "全部完成，共处理 {} 项，生成 {} 个 PDF。",
                shared.spec.ids.len(),
                summary.pdfs
            ),
        );
    }

    Ok(summary)
}

async fn process_album(shared: Arc<Shared>, id: String) {
    log(&shared.sink, format!("开始处理本子：{id}"));

    let album = match shared.client.album(&id).await {
        Ok(album) => album,
        Err(err) => {
            shared.fail(format!("本子 {id}"), brief(&err));
            return;
        }
    };

    log(
        &shared.sink,
        format!(
            "本子获取成功：{} [{}]，作者：{}，章节数：{}",
            album.name,
            album.id,
            album.author(),
            album.episodes.len()
        ),
    );
    shared.progress.add_photos_total(album.episodes.len());

    let mut tasks = JoinSet::new();
    for episode in album.episodes.clone() {
        let shared = shared.clone();
        let album = album.clone();
        tasks.spawn(async move {
            process_photo(
                shared,
                episode.photo_id,
                Some(album),
                episode.index as usize,
            )
            .await;
        });
    }
    while tasks.join_next().await.is_some() {}

    shared.progress.add_album_done();
    shared.bump(|stats| stats.albums += 1);
    log(
        &shared.sink,
        format!("本子处理完成：{} [{}]", album.name, album.id),
    );
}

/// 章节任务入口：统一负责并发许可、进度与统计。
async fn process_photo(
    shared: Arc<Shared>,
    photo_id: String,
    album_hint: Option<Album>,
    index_hint: usize,
) {
    let permit = shared.photo_sem.clone().acquire_owned().await;
    if !shared.is_stopped() {
        let failures_before = shared.failure_count();
        download_photo_inner(shared.clone(), photo_id, album_hint, index_hint).await;
        // 只有整章没有新增失败时才计入“成功章节”
        if shared.failure_count() == failures_before {
            shared.bump(|stats| stats.photos += 1);
        }
    }
    drop(permit);
    shared.progress.add_photo_done();
}

async fn download_photo_inner(
    shared: Arc<Shared>,
    photo_id: String,
    album_hint: Option<Album>,
    index_hint: usize,
) {
    let mut photo = match shared.client.photo_detail(&photo_id, album_hint).await {
        Ok(photo) => photo,
        Err(err) => {
            shared.fail(format!("章节 {photo_id}"), brief(&err));
            return;
        }
    };
    if index_hint > 0 {
        // 单章本子的 sort 通常为 2，但语义序号应为 1（与 Python 版一致）
        let hinted = index_hint as u32;
        photo.index = if photo.is_single_album() && hinted == 2 {
            1
        } else {
            hinted
        };
    }

    if photo.images.is_empty() {
        shared.fail(
            format!("章节 {photo_id}"),
            "章节没有可下载的图片（可能已下架或需要登录）",
        );
        return;
    }

    let album = photo
        .album
        .clone()
        .unwrap_or_else(|| fallback_album(&photo));
    let image_dir = shared
        .output_root
        .join(naming::album_dir_name(&album))
        .join(photo.index.max(1).to_string());

    log(
        &shared.sink,
        format!(
            "开始下载章节 {}（{} 张，序号 {}）：{}",
            photo.id,
            photo.images.len(),
            photo.index,
            photo.name
        ),
    );

    let photo = Arc::new(photo);
    let image_dir = Arc::new(image_dir);
    let total = photo.images.len();
    shared.progress.add_images_total(total);

    let mut tasks = JoinSet::new();
    for index in 0..total {
        let shared = shared.clone();
        let photo = photo.clone();
        let image_dir = image_dir.clone();
        tasks.spawn(async move {
            let permit = shared.image_sem.clone().acquire_owned().await;
            let result = download_one_image(&shared, &photo, index, &image_dir).await;
            drop(permit);
            (index, result)
        });
    }

    let mut paths: Vec<Option<PathBuf>> = vec![None; total];
    let mut failed = 0usize;
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok((index, Ok(path))) => paths[index] = Some(path),
            Ok((index, Err(err))) => {
                failed += 1;
                shared.fail(format!("章节 {} 第{}张", photo.id, index + 1), brief(&err));
            }
            Err(join_error) => {
                failed += 1;
                shared.fail(
                    format!("章节 {}", photo.id),
                    format!("图片任务异常：{join_error}"),
                );
            }
        }
    }

    if failed > 0 {
        log(
            &shared.sink,
            format!(
                "章节 {} 有 {} 张图片失败，跳过 PDF 并保留已下载图片，便于排查或重试。",
                photo.id, failed
            ),
        );
    } else if shared.is_stopped() {
        log(
            &shared.sink,
            format!("章节 {} 已停止，跳过 PDF 生成。", photo.id),
        );
    } else {
        build_pdf_and_cleanup(&shared, &photo, &album, &paths).await;
    }
}

async fn download_one_image(
    shared: &Arc<Shared>,
    photo: &Arc<Photo>,
    index: usize,
    image_dir: &Path,
) -> Result<PathBuf> {
    if shared.is_stopped() {
        return Err(JmError::Msg("任务已停止".to_string()));
    }

    let source_suffix = photo.image_suffix(index).to_string();
    let gif = photo.is_gif(index);
    let target_suffix = if gif {
        source_suffix.clone()
    } else {
        shared
            .spec
            .image_format
            .suffix()
            .map(str::to_string)
            .unwrap_or_else(|| source_suffix.clone())
    };
    let stem = photo.image_stem(index).to_string();
    let file_name = format!("{}{}", naming::fix_windir_name(&stem), target_suffix);
    let target = image_dir.join(file_name);

    // 断点续传：已存在且非空的图片直接跳过
    if shared.spec.download_cache {
        if let Ok(meta) = std::fs::metadata(&target) {
            if meta.is_file() && meta.len() > 0 {
                shared.progress.add_image_done();
                shared.progress.add_bytes(meta.len());
                shared.bump(|stats| stats.images += 1);
                log(
                    &shared.sink,
                    format!("图片已存在，跳过：{}", target.display()),
                );
                return Ok(target);
            }
        }
    }

    let bytes = shared.client.fetch_image(photo, index).await?;

    let scramble_id = photo.scramble_id.clone();
    let aid = photo.id.clone();
    let decode = shared.spec.decode_images && !gif;
    let source_for_task = source_suffix.clone();
    let target_suffix_for_task = target_suffix.clone();
    let target_for_task = target.clone();
    let stem_for_task = stem.clone();

    let saved = tokio::task::spawn_blocking(move || {
        imageio::save_image(
            &bytes,
            &source_for_task,
            &target_suffix_for_task,
            &target_for_task,
            &scramble_id,
            &aid,
            &stem_for_task,
            decode,
        )
    })
    .await
    .map_err(|e| JmError::Msg(format!("图片处理任务异常：{e}")))??;

    shared.progress.add_image_done();
    shared.progress.add_bytes(saved.bytes);
    shared.bump(|stats| stats.images += 1);
    log(
        &shared.sink,
        format!(
            "图片完成 {}/{}：{} ({}x{}, {:.0} KB{})",
            index + 1,
            photo.images.len(),
            saved
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            saved.width,
            saved.height,
            saved.bytes as f64 / 1024.0,
            if saved.descrambled {
                "，已解密"
            } else {
                ""
            }
        ),
    );

    Ok(saved.path)
}

async fn build_pdf_and_cleanup(
    shared: &Arc<Shared>,
    photo: &Arc<Photo>,
    album: &Album,
    paths: &[Option<PathBuf>],
) {
    let files: Vec<PathBuf> = paths.iter().flatten().cloned().collect();
    if files.is_empty() {
        shared.fail(
            format!("章节 {}", photo.id),
            "没有可用于生成 PDF 的图片".to_string(),
        );
        return;
    }

    // 限制同时进行的 PDF 生成数量
    let _pdf_permit = shared.pdf_sem.clone().acquire_owned().await;

    let pdf_name = naming::pdf_filename_for_photo(album, photo);
    let pdf_path = shared.pdf_dir.join(&pdf_name);
    let title = pdf_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| pdf_name.clone());

    let pdf_path_for_task = pdf_path.clone();
    let files_for_task = files.clone();
    let task = tokio::task::spawn_blocking(move || -> Result<usize> {
        let mut pages = Vec::with_capacity(files_for_task.len());
        for file in &files_for_task {
            pages.push(pdf::image_from_file(file)?);
        }
        pdf::write_verified_pdf(&pages, &pdf_path_for_task, &title)
    })
    .await;

    match task {
        Ok(Ok(page_count)) => {
            let removed = remove_downloaded_images(&files, &shared.output_root, &shared.pdf_dir)
                .unwrap_or_else(|err| {
                    log(&shared.sink, format!("清理中间图片失败：{}", brief(&err)));
                    0
                });
            shared.progress.add_pdf_done();
            shared.bump(|stats| stats.pdfs += 1);
            log(
                &shared.sink,
                format!(
                    "PDF 生成完成：{}（{} 页，已清理 {} 张中间图片）",
                    pdf_path.display(),
                    page_count,
                    removed
                ),
            );
        }
        Ok(Err(err)) => {
            shared.fail(format!("PDF {pdf_name}"), brief(&err));
        }
        Err(join_error) => {
            shared.fail(
                format!("PDF {pdf_name}"),
                format!("PDF 任务异常：{join_error}"),
            );
        }
    }
}

/// 校验通过后删除图片，并清理空目录；绝不删除下载根目录与 PDF 目录中的文件。
fn remove_downloaded_images(
    paths: &[PathBuf],
    output_root: &Path,
    pdf_dir: &Path,
) -> Result<usize> {
    let root = output_root
        .canonicalize()
        .unwrap_or_else(|_| output_root.to_path_buf());
    let pdf_root = pdf_dir
        .canonicalize()
        .unwrap_or_else(|_| pdf_dir.to_path_buf());

    let mut deleted = 0usize;
    let mut parents: Vec<PathBuf> = Vec::new();

    for path in paths {
        let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());

        if !resolved.starts_with(&root) {
            return Err(JmError::Msg(format!(
                "拒绝删除下载目录外的图片：{}",
                resolved.display()
            )));
        }
        if resolved.starts_with(&pdf_root) {
            return Err(JmError::Msg(format!(
                "拒绝删除 PDF 目录内的文件：{}",
                resolved.display()
            )));
        }

        if let Some(parent) = resolved.parent() {
            parents.push(parent.to_path_buf());
        }

        match std::fs::remove_file(&resolved) {
            Ok(()) => deleted += 1,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(JmError::Io(std::io::Error::new(
                    err.kind(),
                    format!("删除图片失败 {}: {err}", resolved.display()),
                )))
            }
        }
    }

    parents.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    parents.dedup();
    for parent in parents {
        let mut current = parent;
        while current != root && current.starts_with(&root) {
            if std::fs::remove_dir(&current).is_err() {
                break;
            }
            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => break,
            }
        }
    }

    Ok(deleted)
}

fn fallback_album(photo: &Photo) -> Album {
    Album {
        id: photo.album_id.clone(),
        name: if photo.name.is_empty() {
            format!("JM{}", photo.album_id)
        } else {
            photo.name.clone()
        },
        authors: Vec::new(),
        tags: Vec::new(),
        description: String::new(),
        page_count: 0,
        scramble_id: photo.scramble_id.clone(),
        episodes: vec![Episode {
            photo_id: photo.id.clone(),
            index: photo.index,
            name: photo.name.clone(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_refuses_paths_outside_root() {
        let dir = std::env::temp_dir().join(format!("jm_cleanup_{}", std::process::id()));
        let root = dir.join("下载");
        let pdf_dir = root.join("PDF");
        std::fs::create_dir_all(&pdf_dir).unwrap();

        let outside = dir.join("outside.png");
        std::fs::write(&outside, b"x").unwrap();

        let err = remove_downloaded_images(&[outside.clone()], &root, &pdf_dir).unwrap_err();
        assert!(err.to_string().contains("拒绝删除"));

        // PDF 目录内的文件同样被拒绝
        let inside_pdf = pdf_dir.join("a.pdf");
        std::fs::write(&inside_pdf, b"x").unwrap();
        let err = remove_downloaded_images(&[inside_pdf], &root, &pdf_dir).unwrap_err();
        assert!(err.to_string().contains("拒绝删除"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_deletes_images_and_empty_dirs() {
        let dir = std::env::temp_dir().join(format!("jm_cleanup_ok_{}", std::process::id()));
        let root = dir.join("下载");
        let pdf_dir = root.join("PDF");
        let image_dir = root.join("本子").join("1");
        std::fs::create_dir_all(&image_dir).unwrap();
        std::fs::create_dir_all(&pdf_dir).unwrap();

        let image = image_dir.join("00001.jpg");
        std::fs::write(&image, b"data").unwrap();

        let deleted =
            remove_downloaded_images(std::slice::from_ref(&image), &root, &pdf_dir).unwrap();
        assert_eq!(deleted, 1);
        assert!(!image.exists());
        // 章节目录与本子目录被清理，但根目录保留
        assert!(root.exists());
        assert!(!root.join("本子").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
