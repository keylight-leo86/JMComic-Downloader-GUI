//! 事件、日志与进度：核心层与 GUI / CLI 的唯一接口。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// 日志输出函数。GUI 收集到队列，CLI 直接打印。
pub type LogSink = Arc<dyn Fn(String) + Send + Sync>;

/// 带时间戳写一行日志。
pub fn log(sink: &LogSink, message: impl AsRef<str>) {
    let line = format!("[{}] {}", now_clock(), message.as_ref());
    sink(line);
}

pub fn now_clock() -> String {
    let now = chrono::Local::now();
    now.format("%H:%M:%S").to_string()
}

/// 停止标记：用户点击“停止”后，未开始的任务不再启动。
#[derive(Clone, Default)]
pub struct StopFlag(Arc<AtomicBool>);

impl StopFlag {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// 进度快照。
#[derive(Debug, Clone, Copy, Default)]
pub struct ProgressSnapshot {
    pub albums_done: usize,
    pub albums_total: usize,
    pub photos_done: usize,
    pub photos_total: usize,
    pub images_done: usize,
    pub images_total: usize,
    pub pdfs_done: usize,
    /// 已写入磁盘（或命中缓存）的图片字节数，用于估算下载速度。
    pub bytes: u64,
}

/// 进度计数器（无锁原子操作，可被多个任务并发更新）。
#[derive(Debug, Default)]
pub struct Progress {
    albums_done: AtomicUsize,
    albums_total: AtomicUsize,
    photos_done: AtomicUsize,
    photos_total: AtomicUsize,
    images_done: AtomicUsize,
    images_total: AtomicUsize,
    pdfs_done: AtomicUsize,
    bytes: AtomicU64,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_albums_total(&self, value: usize) {
        self.albums_total.store(value, Ordering::Relaxed);
    }

    pub fn add_album_done(&self) {
        self.albums_done.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_photos_total(&self, value: usize) {
        self.photos_total.fetch_add(value, Ordering::Relaxed);
    }

    pub fn add_photo_done(&self) {
        self.photos_done.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_images_total(&self, value: usize) {
        self.images_total.fetch_add(value, Ordering::Relaxed);
    }

    pub fn add_image_done(&self) {
        self.images_done.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_pdf_done(&self) {
        self.pdfs_done.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_bytes(&self, value: u64) {
        self.bytes.fetch_add(value, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            albums_done: self.albums_done.load(Ordering::Relaxed),
            albums_total: self.albums_total.load(Ordering::Relaxed),
            photos_done: self.photos_done.load(Ordering::Relaxed),
            photos_total: self.photos_total.load(Ordering::Relaxed),
            images_done: self.images_done.load(Ordering::Relaxed),
            images_total: self.images_total.load(Ordering::Relaxed),
            pdfs_done: self.pdfs_done.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
        }
    }
}

/// 一条失败记录。
#[derive(Debug, Clone)]
pub struct Failure {
    /// 失败对象（车号 / 章节 / 图片）。
    pub scope: String,
    pub error: String,
}

/// 任务总结。
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub albums: usize,
    pub photos: usize,
    pub images: usize,
    pub pdfs: usize,
    pub failures: Vec<Failure>,
    pub stopped: bool,
}

impl Summary {
    pub fn has_failures(&self) -> bool {
        !self.failures.is_empty()
    }
}
