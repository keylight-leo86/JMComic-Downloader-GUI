//! Windows 11 风格图形界面（egui/eframe）。
//!
//! 设计要点：
//! - 跟随系统浅色/深色模式与系统强调色（注册表读取，可在界面上覆盖）；
//! - 圆角卡片、细分隔线、留白与 Win11 一致的信息层级；
//! - 中英文分别使用 Segoe UI / 微软系中文字体，避免方块字；
//! - 下载在独立线程的 tokio 运行时中执行，界面始终可交互；
//! - 日志按 100 ms 批量刷新，避免高频日志拖慢渲染。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, FontFamily, Margin, RichText, Stroke};

use crate::config::{self, ClientKind, ImageFormat, Kind};
use crate::download::{self, JobSpec};
use crate::events::{Failure, LogSink, Progress, ProgressSnapshot, StopFlag, Summary};
use crate::naming;
use crate::option;
use crate::settings::{normalize_ui_scale, Settings};
use crate::win_theme::{self, SystemAppearance, ThemeMode};

const PROXY_SYSTEM_LABEL: &str = "跟随系统";
const PROXY_NONE_LABEL: &str = "不使用代理";
const LOG_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

pub fn launch() -> Result<(), String> {
    let title = format!("{} {}", config::APP_NAME, config::APP_VERSION);
    let appearance = win_theme::detect();
    let icon = app_icon(appearance);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 880.0])
            .with_min_inner_size([1040.0, 740.0])
            .with_title(title.clone())
            .with_app_id("JMComic.Downloader.GUI")
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(&title, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
        .map_err(|e| e.to_string())
}

/// 调色板：浅色 / 深色两套，强调色来自系统主题色。
#[derive(Debug, Clone, Copy)]
struct Palette {
    bg: Color32,
    panel: Color32,
    panel_soft: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    accent: Color32,
    accent_soft: Color32,
    on_accent: Color32,
    success: Color32,
    danger: Color32,
    log_bg: Color32,
    log_text: Color32,
}

impl Palette {
    fn light(accent: (u8, u8, u8)) -> Self {
        let accent = rgb(accent);
        Self {
            bg: Color32::from_rgb(0xf3, 0xf3, 0xf3),
            panel: Color32::from_rgb(0xff, 0xff, 0xff),
            panel_soft: Color32::from_rgb(0xf7, 0xf7, 0xf7),
            border: Color32::from_rgb(0xe3, 0xe3, 0xe3),
            text: Color32::from_rgb(0x1b, 0x1b, 0x1b),
            muted: Color32::from_rgb(0x5d, 0x5d, 0x5d),
            accent,
            accent_soft: blend(accent, Color32::WHITE, 0.86),
            on_accent: color_on(accent),
            success: Color32::from_rgb(0x0f, 0x7b, 0x4f),
            danger: Color32::from_rgb(0xc4, 0x2b, 0x1c),
            log_bg: Color32::from_rgb(0xfa, 0xfa, 0xfa),
            log_text: Color32::from_rgb(0x24, 0x29, 0x2e),
        }
    }

    fn dark(accent: (u8, u8, u8)) -> Self {
        let accent = lighten(rgb(accent), 0.18);
        Self {
            bg: Color32::from_rgb(0x20, 0x20, 0x20),
            panel: Color32::from_rgb(0x2b, 0x2b, 0x2b),
            panel_soft: Color32::from_rgb(0x32, 0x32, 0x32),
            border: Color32::from_rgb(0x3d, 0x3d, 0x3d),
            text: Color32::from_rgb(0xf5, 0xf5, 0xf5),
            muted: Color32::from_rgb(0xa8, 0xa8, 0xa8),
            accent,
            accent_soft: blend(accent, Color32::from_rgb(0x2b, 0x2b, 0x2b), 0.78),
            on_accent: color_on(accent),
            success: Color32::from_rgb(0x3d, 0xbe, 0x7a),
            danger: Color32::from_rgb(0xff, 0x6b, 0x5e),
            log_bg: Color32::from_rgb(0x1b, 0x1b, 0x1b),
            log_text: Color32::from_rgb(0xe3, 0xe3, 0xe3),
        }
    }
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |x: u8, y: u8| -> u8 {
        (x as f32 * (1.0 - t) + y as f32 * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

fn lighten(color: Color32, amount: f32) -> Color32 {
    blend(color, Color32::WHITE, amount)
}

/// 根据强调色亮度选择前景色，保证浅色强调色上的文字仍然清晰。
fn color_on(color: Color32) -> Color32 {
    let luminance =
        0.2126 * color.r() as f32 + 0.7152 * color.g() as f32 + 0.0722 * color.b() as f32;
    if luminance > 150.0 {
        Color32::from_rgb(0x14, 0x14, 0x14)
    } else {
        Color32::WHITE
    }
}

enum UiEvent {
    Log(String),
    Finished(Summary, Duration),
}

pub struct App {
    settings: Settings,
    mode: Kind,
    ids_text: String,
    output: String,
    client: ClientKind,
    format: ImageFormat,
    image_threads: usize,
    photo_threads: usize,
    album_threads: usize,
    proxy: String,
    option_path: String,

    log: String,
    pending_log: String,
    last_flush: Instant,
    status: String,
    notice: String,
    busy: bool,
    stop: StopFlag,
    progress: Arc<Progress>,
    receiver: Option<Receiver<UiEvent>>,
    worker: Option<std::thread::JoinHandle<()>>,
    started_at: Option<Instant>,
    summary: Option<Summary>,
    last_progress: ProgressSnapshot,

    theme_mode: ThemeMode,
    appearance: SystemAppearance,
    applied_dark: Option<bool>,
    palette: Palette,
    last_theme_check: Instant,
    /// 界面缩放（字号比例），1.0 = 100%
    ui_scale: f32,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        let appearance = win_theme::detect();
        let theme_mode = ThemeMode::from_key(&settings.theme);
        let ui_scale = normalize_ui_scale(settings.ui_scale);
        let dark = effective_dark(theme_mode, appearance);
        let palette = if dark {
            Palette::dark(accent_of(&appearance))
        } else {
            Palette::light(accent_of(&appearance))
        };

        install_fonts(&cc.egui_ctx);
        apply_visuals(&cc.egui_ctx, &palette, dark);
        cc.egui_ctx.set_zoom_factor(ui_scale);

        let mut proxy = settings.proxy.clone();
        proxy = match proxy.trim() {
            "system" => PROXY_SYSTEM_LABEL.to_string(),
            "none" => PROXY_NONE_LABEL.to_string(),
            other => other.to_string(),
        };

        let mut app = Self {
            mode: if settings.mode == "photo" {
                Kind::Photo
            } else {
                Kind::Album
            },
            ids_text: String::new(),
            output: settings.initial_output_directory().display().to_string(),
            client: settings.client_kind(),
            format: settings.image_format(),
            image_threads: settings.image_threads,
            photo_threads: settings.photo_threads,
            album_threads: settings.album_threads,
            proxy,
            option_path: settings.option.clone(),

            log: String::new(),
            pending_log: String::new(),
            last_flush: Instant::now(),
            status: "就绪".to_string(),
            notice: String::new(),
            busy: false,
            stop: StopFlag::new(),
            progress: Progress::new(),
            receiver: None,
            worker: None,
            started_at: None,
            summary: None,
            last_progress: ProgressSnapshot::default(),

            theme_mode,
            appearance,
            applied_dark: Some(dark),
            palette,
            last_theme_check: Instant::now(),
            ui_scale,
            settings,
        };
        app.push_log("等待任务。输入车号后点击“开始下载”（Ctrl+Enter 也可开始）。");
        app.flush_log();
        app
    }

    fn proxy_arg(&self) -> String {
        match self.proxy.trim() {
            PROXY_SYSTEM_LABEL | "system" | "" => config::PROXY_SYSTEM.to_string(),
            PROXY_NONE_LABEL | "none" => config::PROXY_NONE.to_string(),
            other => other.to_string(),
        }
    }

    fn push_log(&mut self, line: impl AsRef<str>) {
        self.pending_log.push_str(line.as_ref());
        self.pending_log.push('\n');
        if self.pending_log.len() > 256_000 {
            self.flush_log();
        }
    }

    fn flush_log(&mut self) {
        if self.pending_log.is_empty() {
            return;
        }
        self.log.push_str(&self.pending_log);
        self.pending_log.clear();
        self.last_flush = Instant::now();

        if self.log.len() > 800_000 {
            let mut cut = self.log.len() - 500_000;
            while cut < self.log.len() && !self.log.is_char_boundary(cut) {
                cut += 1;
            }
            self.log = self.log[cut..].to_string();
        }
    }

    fn save_settings(&mut self) {
        let mut settings = self.settings.clone();
        settings.mode = match self.mode {
            Kind::Album => "album".to_string(),
            Kind::Photo => "photo".to_string(),
        };
        settings.client = self.client.key().to_string();
        settings.format = self.format.key().to_string();
        settings.image_threads = self.image_threads;
        settings.photo_threads = self.photo_threads;
        settings.album_threads = self.album_threads;
        settings.proxy = self.proxy_arg();
        settings.option = self.option_path.trim().to_string();
        settings.theme = self.theme_mode.key().to_string();
        settings.ui_scale = self.ui_scale;
        settings.apply_output(&self.output);
        let _ = settings.save();
        self.settings = settings;
    }

    fn start(&mut self, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        self.notice.clear();

        let ids = match naming::parse_ids(&self.ids_text) {
            Ok(ids) => ids,
            Err(err) => {
                self.notice = err.to_string();
                return;
            }
        };

        let output = self.output.trim().to_string();
        if output.is_empty() {
            self.notice = "请选择下载保存位置".to_string();
            return;
        }

        let option_path = self.option_path.trim().to_string();
        if !option_path.is_empty() && !Path::new(&option_path).is_file() {
            self.notice = "高级 YAML 配置文件不存在".to_string();
            return;
        }

        let mut spec = JobSpec {
            kind: self.mode,
            ids: ids.clone(),
            output_dir: PathBuf::from(&output),
            client: self.client,
            image_format: self.format,
            image_concurrency: self.image_threads,
            photo_concurrency: self.photo_threads,
            album_concurrency: self.album_threads,
            proxy: self.proxy_arg(),
            retry: config::DEFAULT_RETRY_TIMES,
            decode_images: true,
            download_cache: true,
            domains: None,
            image_domains: None,
        };

        if !option_path.is_empty() {
            match option::load(Path::new(&option_path)) {
                Ok((overrides, notes)) => {
                    self.push_log(format!("已载入高级配置：{option_path}"));
                    for note in notes {
                        self.push_log(format!("提示：{note}"));
                    }
                    overrides.apply_to(&mut spec);
                }
                Err(err) => {
                    self.notice = format!("读取高级配置失败：{err}");
                    return;
                }
            }
        }

        spec.image_concurrency = spec.image_concurrency.clamp(1, config::MAX_IMAGE_THREADS);
        spec.photo_concurrency = spec.photo_concurrency.clamp(1, config::MAX_PHOTO_THREADS);
        spec.album_concurrency = spec.album_concurrency.clamp(1, config::MAX_ALBUM_THREADS);

        self.save_settings();
        self.log.clear();
        self.pending_log.clear();
        self.summary = None;
        self.last_progress = ProgressSnapshot::default();
        self.push_log(format!("已提交 {} 项下载任务。", ids.len()));
        self.push_log(format!(
            "并发配置：本子 {} / 章节 {} / 图片 {}",
            spec.album_concurrency, spec.photo_concurrency, spec.image_concurrency
        ));
        self.flush_log();

        let (tx, rx) = mpsc::channel::<UiEvent>();
        self.receiver = Some(rx);
        self.progress = Progress::new();
        self.stop = StopFlag::new();
        self.busy = true;
        self.started_at = Some(Instant::now());
        self.status = "正在下载".to_string();

        let tx_log = tx.clone();
        let ctx_log = ctx.clone();
        let sink: LogSink = Arc::new(move |line: String| {
            let _ = tx_log.send(UiEvent::Log(line));
            ctx_log.request_repaint();
        });

        let stop = self.stop.clone();
        let progress = self.progress.clone();
        let tx_done = tx;
        let ctx_done = ctx.clone();

        self.worker = Some(std::thread::spawn(move || {
            let started = Instant::now();
            let summary = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(download::run(spec, sink, stop, progress)),
                Err(err) => Summary {
                    failures: vec![Failure {
                        scope: "异步运行时".to_string(),
                        error: err.to_string(),
                    }],
                    ..Default::default()
                },
            };
            let _ = tx_done.send(UiEvent::Finished(summary, started.elapsed()));
            ctx_done.request_repaint();
        }));
    }

    fn poll_events(&mut self) {
        let Some(receiver) = self.receiver.take() else {
            return;
        };

        let mut finished: Option<(Summary, Duration)> = None;
        let mut disconnected = false;

        loop {
            match receiver.try_recv() {
                Ok(UiEvent::Log(line)) => self.push_log(line),
                Ok(UiEvent::Finished(summary, elapsed)) => {
                    finished = Some((summary, elapsed));
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if finished.is_none() && !disconnected {
            self.receiver = Some(receiver);
        }

        if let Some((summary, elapsed)) = finished {
            self.busy = false;
            self.receiver = None;
            self.worker = None;
            self.summary = Some(summary.clone());
            self.last_progress = self.progress.snapshot();
            self.status = if summary.stopped {
                "任务已停止".to_string()
            } else if summary.failures.is_empty() {
                format!(
                    "完成 · {} 个 PDF · 用时 {:.1}s",
                    summary.pdfs,
                    elapsed.as_secs_f64()
                )
            } else {
                format!("存在 {} 项失败，请查看日志", summary.failures.len())
            };
            self.push_log(format!(
                "任务结束：本子 {}，章节 {}，图片 {}，PDF {}，失败 {}。",
                summary.albums,
                summary.photos,
                summary.images,
                summary.pdfs,
                summary.failures.len()
            ));
            self.flush_log();
        } else if disconnected && self.busy {
            self.busy = false;
            self.receiver = None;
            self.status = "下载线程异常结束".to_string();
            self.push_log("下载线程异常结束，请查看日志。");
            self.flush_log();
        }
    }

    fn open_pdf_dir(&mut self) {
        let base = self.output.trim();
        let base = if base.is_empty() {
            crate::settings::default_download_directory()
        } else {
            PathBuf::from(base)
        };
        let path = base.join("PDF");
        let _ = std::fs::create_dir_all(&path);
        let _ = std::process::Command::new("explorer").arg(&path).spawn();
    }

    /// 主题变化时重建样式（跟随系统模式下，系统切换主题可即时生效）。
    fn sync_theme(&mut self, ctx: &egui::Context) {
        // 注册表读取需要拉起子进程，最多每 2 秒检查一次
        if self.theme_mode == ThemeMode::Auto
            && self.last_theme_check.elapsed() > Duration::from_secs(2)
        {
            self.last_theme_check = Instant::now();
            self.appearance = win_theme::detect();
        }

        let dark = effective_dark(self.theme_mode, self.appearance);
        if self.applied_dark == Some(dark) {
            return;
        }
        self.applied_dark = Some(dark);
        self.palette = if dark {
            Palette::dark(accent_of(&self.appearance))
        } else {
            Palette::light(accent_of(&self.appearance))
        };
        apply_visuals(ctx, &self.palette, dark);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_theme(ctx);
        self.poll_events();

        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if !self.pending_log.is_empty() && self.last_flush.elapsed() >= LOG_FLUSH_INTERVAL {
            self.flush_log();
        }

        if ctx.input(|input| input.viewport().close_requested()) {
            if self.busy {
                self.stop.stop();
            }
            self.save_settings();
        }

        if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::Enter)) {
            self.start(ctx);
        }

        // 快捷键缩放：Ctrl+= 放大、Ctrl+- 缩小、Ctrl+0 复位
        if ctx.input(|input| input.modifiers.ctrl) {
            let (zoom_in, zoom_out, reset) = ctx.input(|input| {
                (
                    input.key_pressed(egui::Key::Plus) || input.key_pressed(egui::Key::Equals),
                    input.key_pressed(egui::Key::Minus),
                    input.key_pressed(egui::Key::Num0),
                )
            });
            if zoom_in {
                self.set_ui_scale(ctx, self.ui_scale + 0.05);
            }
            if zoom_out {
                self.set_ui_scale(ctx, self.ui_scale - 0.05);
            }
            if reset {
                self.set_ui_scale(ctx, 1.0);
            }
        }

        self.header(ctx);
        self.footer(ctx);
        self.sidebar(ctx);
        self.log_panel(ctx);
    }
}

impl App {
    fn header(&mut self, ctx: &egui::Context) {
        let palette = self.palette;
        let mut theme_changed = false;
        let mut scale_changed: Option<f32> = None;

        egui::TopBottomPanel::top("titlebar")
            .frame(
                egui::Frame::default()
                    .fill(palette.panel)
                    .inner_margin(Margin::symmetric(22, 14)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("JMComic Downloader")
                                .size(20.0)
                                .strong()
                                .color(palette.text),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new("批量下载 · 近乎无损 PDF · 自动清理 · 实时日志")
                                .size(12.5)
                                .color(palette.muted),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        version_chip(ui, &palette);
                        ui.add_space(10.0);

                        // 界面缩放（字号）
                        let mut scale = self.ui_scale;
                        egui::ComboBox::from_id_salt("ui_scale")
                            .selected_text(format!("{:.0}%", scale * 100.0))
                            .width(78.0)
                            .show_ui(ui, |ui| {
                                for value in [1.0f32, 1.1, 1.25, 1.5, 1.75] {
                                    ui.selectable_value(
                                        &mut scale,
                                        value,
                                        format!("{:.0}%", value * 100.0),
                                    );
                                }
                            });
                        if (scale - self.ui_scale).abs() > 0.001 {
                            scale_changed = Some(scale);
                        }
                        ui.add_space(4.0);
                        ui.label(RichText::new("缩放").size(12.5).color(palette.muted));
                        ui.add_space(10.0);

                        let mut selected = self.theme_mode;
                        egui::ComboBox::from_id_salt("theme")
                            .selected_text(selected.label())
                            .width(104.0)
                            .show_ui(ui, |ui| {
                                for mode in [ThemeMode::Auto, ThemeMode::Light, ThemeMode::Dark] {
                                    ui.selectable_value(&mut selected, mode, mode.label());
                                }
                            });
                        if selected != self.theme_mode {
                            self.theme_mode = selected;
                            theme_changed = true;
                        }
                        ui.add_space(4.0);
                        ui.label(RichText::new("主题").size(12.5).color(palette.muted));
                    });
                });

                let rect = ui.max_rect();
                ui.painter().hline(
                    rect.x_range(),
                    rect.bottom() + 14.0,
                    Stroke::new(1.0, palette.border),
                );
            });

        if theme_changed {
            self.applied_dark = None; // 强制下一帧重建样式
            self.save_settings();
        }
        if let Some(scale) = scale_changed {
            self.set_ui_scale(ctx, scale);
        }
    }

    /// 调整界面缩放并持久化（1.0 = 100%，范围 80%–200%）。
    fn set_ui_scale(&mut self, ctx: &egui::Context, scale: f32) {
        let scale = normalize_ui_scale(scale);
        if (scale - self.ui_scale).abs() < 0.001 {
            return;
        }
        self.ui_scale = scale;
        ctx.set_zoom_factor(scale);
        self.save_settings();
    }

    fn footer(&mut self, ctx: &egui::Context) {
        let palette = self.palette;
        let snapshot = if self.busy {
            self.progress.snapshot()
        } else {
            self.last_progress
        };
        let elapsed = self
            .started_at
            .map(|start| start.elapsed())
            .unwrap_or_default();

        egui::TopBottomPanel::bottom("footer")
            .frame(
                egui::Frame::default()
                    .fill(palette.panel)
                    .inner_margin(Margin::symmetric(22, 12)),
            )
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().hline(
                    rect.x_range(),
                    rect.top() - 12.0,
                    Stroke::new(1.0, palette.border),
                );

                ui.horizontal(|ui| {
                    let (fraction, text) = progress_text(snapshot, self.busy);
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(220.0)
                            .animate(self.busy && fraction <= 0.0),
                    );
                    ui.add_space(6.0);
                    ui.label(RichText::new(text).size(12.5).color(palette.muted));

                    if self.busy {
                        let speed = if elapsed.as_secs_f64() > 0.2 {
                            snapshot.bytes as f64 / elapsed.as_secs_f64() / (1024.0 * 1024.0)
                        } else {
                            0.0
                        };
                        ui.label(
                            RichText::new(format!(
                                "{:.1} MB/s · {}",
                                speed,
                                format_duration(elapsed)
                            ))
                            .size(12.5)
                            .color(palette.muted),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let start_enabled = !self.busy;
                        let start = ui.add_enabled(
                            start_enabled,
                            egui::Button::new(
                                RichText::new("开始下载").color(palette.on_accent).strong(),
                            )
                            .fill(if start_enabled {
                                palette.accent
                            } else {
                                palette.border
                            })
                            .corner_radius(CornerRadius::same(6))
                            .min_size(egui::vec2(122.0, 34.0)),
                        );
                        if start.clicked() {
                            let ctx = ui.ctx().clone();
                            self.start(&ctx);
                        }

                        let stop = ui.add_enabled(
                            self.busy,
                            egui::Button::new(RichText::new("停止").color(palette.danger))
                                .corner_radius(CornerRadius::same(6))
                                .min_size(egui::vec2(78.0, 34.0)),
                        );
                        if stop.clicked() {
                            self.stop.stop();
                            self.status = "正在停止…".to_string();
                            self.push_log("已请求停止任务。");
                        }

                        if ui
                            .add(
                                egui::Button::new("打开 PDF 目录")
                                    .corner_radius(CornerRadius::same(6))
                                    .min_size(egui::vec2(126.0, 34.0)),
                            )
                            .clicked()
                        {
                            self.open_pdf_dir();
                        }

                        ui.add_space(6.0);
                        let failed = self
                            .summary
                            .as_ref()
                            .map(|summary| !summary.failures.is_empty())
                            .unwrap_or(false);
                        status_chip(ui, &palette, &self.status, self.busy, failed);
                    });
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let palette = self.palette;
        egui::SidePanel::left("sidebar")
            .exact_width(438.0)
            .resizable(false)
            .frame(
                egui::Frame::default()
                    .fill(palette.bg)
                    .inner_margin(Margin::symmetric(16, 14)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // 让内容严格贴合面板宽度，避免横向溢出把面板撑宽
                        ui.set_max_width(ui.available_width());

                        self.card(ui, |app, ui| app.mode_card(ui));
                        self.card(ui, |app, ui| app.ids_card(ui));
                        self.card(ui, |app, ui| app.output_card(ui));
                        self.card(ui, |app, ui| app.settings_card(ui));
                        self.card(ui, |app, ui| app.network_card(ui));
                        self.card(ui, |app, ui| app.advanced_card(ui));

                        if !self.notice.is_empty() {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(self.notice.clone())
                                    .size(12.5)
                                    .color(palette.danger),
                            );
                        }

                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!(
                                "默认保存到 EXE 目录下的“下载” · 设置：{}",
                                crate::settings::settings_path().display()
                            ))
                            .size(10.5)
                            .color(palette.muted),
                        );
                    });
            });
    }

    fn card(&mut self, ui: &mut egui::Ui, add: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let palette = self.palette;
        egui::Frame::default()
            .fill(palette.panel)
            .corner_radius(CornerRadius::same(10))
            .stroke(Stroke::new(1.0, palette.border))
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(self, ui);
            });
        ui.add_space(10.0);
    }

    fn mode_card(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        section_title(ui, &palette, "下载类型");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.mode, Kind::Album, "  本子  ");
            ui.selectable_value(&mut self.mode, Kind::Photo, "  章节  ");
            ui.add_space(6.0);
            ui.label(
                RichText::new(match self.mode {
                    Kind::Album => "下载整本（含全部章节）",
                    Kind::Photo => "只下载输入的章节",
                })
                .size(11.5)
                .color(palette.muted),
            );
        });
    }

    fn ids_card(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        section_title(ui, &palette, "车号或链接");
        ui.add_space(2.0);
        ui.label(
            RichText::new("支持多个，以空格、逗号、分号或换行分隔")
                .size(11.0)
                .color(palette.muted),
        );
        ui.add_space(6.0);
        ui.add(
            egui::TextEdit::multiline(&mut self.ids_text)
                .desired_rows(4)
                .desired_width(ui.available_width())
                .hint_text("1455254\nJM1455254\nhttps://devapp.18comic.cc/comic/detail?id=1455254")
                .font(egui::TextStyle::Monospace),
        );
    }

    fn output_card(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        section_title(ui, &palette, "保存位置");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let row_width = ui.available_width();
            ui.add(
                egui::TextEdit::singleline(&mut self.output)
                    .desired_width((row_width - 78.0).max(120.0))
                    .hint_text("选择下载目录"),
            );
            if ui
                .add(egui::Button::new("选择…").corner_radius(CornerRadius::same(6)))
                .clicked()
            {
                let dialog = rfd::FileDialog::new()
                    .set_title("选择下载目录")
                    .set_directory(initial_dir(&self.output));
                if let Some(dir) = dialog.pick_folder() {
                    self.output = dir.display().to_string();
                }
            }
        });
    }

    fn settings_card(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        section_title(ui, &palette, "下载设置");
        ui.add_space(6.0);

        let column = ((ui.available_width() - 24.0) / 2.0).max(120.0);

        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                field_label(ui, &palette, "客户端");
                field_label(ui, &palette, "图片格式");
                ui.end_row();

                egui::ComboBox::from_id_salt("client")
                    .selected_text(self.client.label())
                    .width(column)
                    .show_ui(ui, |ui| {
                        for kind in [ClientKind::Api, ClientKind::Html] {
                            ui.selectable_value(&mut self.client, kind, kind.label());
                        }
                    });
                egui::ComboBox::from_id_salt("format")
                    .selected_text(self.format.label())
                    .width(column)
                    .show_ui(ui, |ui| {
                        for format in [
                            ImageFormat::Original,
                            ImageFormat::Jpg,
                            ImageFormat::Png,
                            ImageFormat::Webp,
                        ] {
                            ui.selectable_value(&mut self.format, format, format.label());
                        }
                    });
                ui.end_row();

                field_label(ui, &palette, "图片并发");
                field_label(ui, &palette, "章节并发");
                ui.end_row();

                concurrency_input(ui, &mut self.image_threads, config::MAX_IMAGE_THREADS);
                concurrency_input(ui, &mut self.photo_threads, config::MAX_PHOTO_THREADS);
                ui.end_row();

                field_label(ui, &palette, "本子并发（批量车号）");
                field_label(ui, &palette, "解密图片");
                ui.end_row();

                concurrency_input(ui, &mut self.album_threads, config::MAX_ALBUM_THREADS);
                ui.label(
                    RichText::new("自动（按 scramble_id 判断）")
                        .size(11.5)
                        .color(palette.muted),
                );
                ui.end_row();
            });
    }

    fn network_card(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        section_title(ui, &palette, "网络代理");
        ui.add_space(6.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.proxy)
                .desired_width(ui.available_width())
                .hint_text("跟随系统 / 不使用代理 / 127.0.0.1:7890"),
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for preset in [
                PROXY_SYSTEM_LABEL,
                PROXY_NONE_LABEL,
                "127.0.0.1:7890",
                "127.0.0.1:10809",
            ] {
                if ui
                    .add(
                        egui::Button::new(RichText::new(preset).size(11.5))
                            .corner_radius(CornerRadius::same(10)),
                    )
                    .clicked()
                {
                    self.proxy = preset.to_string();
                }
            }
        });
    }

    fn advanced_card(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        section_title(ui, &palette, "高级 YAML 配置");
        ui.add_space(2.0);
        ui.label(
            RichText::new("可选；支持 dir_rule / download / client 等下载相关配置")
                .size(11.0)
                .color(palette.muted),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let row_width = ui.available_width();
            ui.add(
                egui::TextEdit::singleline(&mut self.option_path)
                    .desired_width((row_width - 150.0).max(110.0))
                    .hint_text("留空表示不使用"),
            );
            if ui
                .add(egui::Button::new("选择…").corner_radius(CornerRadius::same(6)))
                .clicked()
            {
                let dialog = rfd::FileDialog::new()
                    .set_title("选择 JMComic 配置文件")
                    .add_filter("YAML 配置", &["yml", "yaml"])
                    .set_directory(crate::settings::app_directory());
                if let Some(file) = dialog.pick_file() {
                    self.option_path = file.display().to_string();
                }
            }
            if ui
                .add(egui::Button::new("清除").corner_radius(CornerRadius::same(6)))
                .clicked()
            {
                self.option_path.clear();
            }
        });
    }

    fn log_panel(&mut self, ctx: &egui::Context) {
        let palette = self.palette;
        if !self.pending_log.is_empty() {
            self.flush_log();
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(palette.bg)
                    .inner_margin(Margin {
                        left: 0,
                        right: 16,
                        top: 14,
                        bottom: 8,
                    }),
            )
            .show(ctx, |ui| {
                egui::Frame::default()
                    .fill(palette.panel)
                    .corner_radius(CornerRadius::same(10))
                    .stroke(Stroke::new(1.0, palette.border))
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            section_title(ui, &palette, "运行日志");
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new("可选中复制").size(11.0).color(palette.muted),
                                    );
                                },
                            );
                        });
                        ui.add_space(8.0);

                        egui::Frame::default()
                            .fill(palette.log_bg)
                            .corner_radius(CornerRadius::same(6))
                            .stroke(Stroke::new(1.0, palette.border))
                            .inner_margin(Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                let height = (ui.available_height() - 8.0).max(120.0);
                                ui.set_min_height(height);
                                egui::ScrollArea::vertical()
                                    .stick_to_bottom(true)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(self.log.as_str())
                                                    .monospace()
                                                    .size(12.5)
                                                    .color(palette.log_text),
                                            )
                                            .wrap(),
                                        );
                                    });
                            });
                    });
            });
    }
}

fn concurrency_input(ui: &mut egui::Ui, value: &mut usize, max: usize) {
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(value)
                .range(1..=max)
                .speed(0.4)
                .fixed_decimals(0),
        );
        ui.label(
            RichText::new(format!("/ {max}"))
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
    });
}

fn field_label(ui: &mut egui::Ui, palette: &Palette, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(palette.muted));
}

fn section_title(ui: &mut egui::Ui, palette: &Palette, text: &str) {
    ui.label(RichText::new(text).size(14.0).strong().color(palette.text));
}

fn version_chip(ui: &mut egui::Ui, palette: &Palette) {
    egui::Frame::default()
        .fill(palette.accent_soft)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("v{}", config::APP_VERSION))
                    .size(11.0)
                    .color(palette.accent),
            );
        });
}

fn status_chip(ui: &mut egui::Ui, palette: &Palette, status: &str, busy: bool, failed: bool) {
    let color = if busy {
        palette.accent
    } else if failed {
        palette.danger
    } else {
        palette.success
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, color);
        ui.label(RichText::new(status).size(12.5).color(palette.text));
    });
}

fn progress_text(snapshot: ProgressSnapshot, busy: bool) -> (f32, String) {
    if snapshot.photos_total == 0 && snapshot.images_total == 0 {
        return (
            0.0,
            if busy {
                "正在获取任务信息…".to_string()
            } else {
                "就绪".to_string()
            },
        );
    }

    let fraction = if snapshot.images_total > 0 {
        snapshot.images_done as f32 / snapshot.images_total as f32
    } else {
        snapshot.photos_done as f32 / snapshot.photos_total.max(1) as f32
    };

    let text = format!(
        "图片 {}/{} · 章节 {}/{} · PDF {}",
        snapshot.images_done,
        snapshot.images_total,
        snapshot.photos_done,
        snapshot.photos_total,
        snapshot.pdfs_done
    );
    (fraction.clamp(0.0, 1.0), text)
}

fn format_duration(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn initial_dir(output: &str) -> PathBuf {
    let path = PathBuf::from(output.trim());
    if path.is_dir() {
        path
    } else {
        crate::settings::default_download_directory()
    }
}

fn effective_dark(mode: ThemeMode, appearance: SystemAppearance) -> bool {
    match mode {
        ThemeMode::Auto => appearance.dark,
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
    }
}

fn accent_of(appearance: &SystemAppearance) -> (u8, u8, u8) {
    appearance.accent.unwrap_or((0x16, 0x85, 0x6b))
}

/// 应用调色板到 egui 视觉样式（圆角、间距、控件配色）。
fn apply_visuals(ctx: &egui::Context, palette: &Palette, dark: bool) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.panel_fill = palette.panel;
    visuals.window_fill = palette.panel;
    visuals.extreme_bg_color = palette.panel_soft;
    visuals.faint_bg_color = palette.panel_soft;
    visuals.override_text_color = Some(palette.text);
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.accent;
    visuals.selection.stroke = Stroke::new(1.0, palette.on_accent);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);

    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
        &mut visuals.widgets.noninteractive,
    ] {
        widget.corner_radius = CornerRadius::same(6);
    }

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.scroll.bar_width = 8.0;
    style.interaction.selectable_labels = true;
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(11.5, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::new(13.5, FontFamily::Monospace),
    );
    ctx.set_style(style);
}

/// 安装西文（Segoe UI / Consolas）与中文（等线 / 黑体）字体。
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let latin = load_system_font(&["segoeui.ttf"]);
    let mono = load_system_font(&["consola.ttf", "cour.ttf"]);
    let cjk = load_system_font(&["Deng.ttf", "simhei.ttf", "simfang.ttf", "simkai.ttf"]);

    let mut primary = Vec::new();
    if let Some(bytes) = latin {
        fonts.font_data.insert(
            "ui-latin".to_string(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        primary.push("ui-latin".to_string());
    }
    let mut mono_primary = Vec::new();
    if let Some(bytes) = mono {
        fonts.font_data.insert(
            "ui-mono".to_string(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        mono_primary.push("ui-mono".to_string());
    }
    if let Some(bytes) = cjk {
        fonts.font_data.insert(
            "ui-cjk".to_string(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        primary.push("ui-cjk".to_string());
        mono_primary.push("ui-cjk".to_string());
    }

    if !primary.is_empty() {
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            let mut merged = primary.clone();
            merged.extend(family.iter().cloned());
            *family = merged;
        }
    }
    if !mono_primary.is_empty() {
        if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
            let mut merged = mono_primary.clone();
            merged.extend(family.iter().cloned());
            *family = merged;
        }
    }

    ctx.set_fonts(fonts);
}

fn load_system_font(candidates: &[&str]) -> Option<Vec<u8>> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();

    for name in candidates {
        let mut paths = vec![PathBuf::from(&windir).join("Fonts").join(name)];
        if !local.is_empty() {
            paths.push(
                PathBuf::from(&local)
                    .join("Microsoft\\Windows\\Fonts")
                    .join(name),
            );
        }
        for path in paths {
            if let Ok(bytes) = std::fs::read(&path) {
                if bytes.len() > 4096 {
                    return Some(bytes);
                }
            }
        }
    }
    None
}

/// 生成应用图标：圆角方块 + 下载箭头。
fn app_icon(appearance: SystemAppearance) -> egui::IconData {
    const SIZE: u32 = 64;
    let (r, g, b) = appearance.accent.unwrap_or((0x16, 0x85, 0x6b));
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];

    let half = SIZE as f32 / 2.0;
    let radius = 14.0f32;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            // 圆角矩形覆盖率（抗锯齿）
            let qx = (px - half).abs() - (half - radius);
            let qy = (py - half).abs() - (half - radius);
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            let inside = qx.max(qy).min(0.0);
            let distance = outside + inside - radius;
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }

            // 上下轻微渐变，贴合 Win11 图标质感
            let t = (py / SIZE as f32) * 0.25;
            let base = [
                (r as f32 * (1.0 - t) + 255.0 * t) as u8,
                (g as f32 * (1.0 - t) + 255.0 * t) as u8,
                (b as f32 * (1.0 - t) + 255.0 * t) as u8,
            ];

            // 白色下载符号：竖条 + 三角箭头 + 底座
            let stem = (28.0..=36.0).contains(&px) && (15.0..=34.0).contains(&py);
            let half_width = ((46.0 - py) * 0.95 + 2.0).clamp(0.0, 12.0);
            let head = py > 34.0 && py <= 46.0 && (px - 32.0).abs() <= half_width;
            let base_bar = (50.0..=54.0).contains(&py) && (18.0..=46.0).contains(&px);
            let white = stem || head || base_bar;

            let index = ((y * SIZE + x) * 4) as usize;
            let (cr, cg, cb) = if white {
                (255u8, 255u8, 255u8)
            } else {
                (base[0], base[1], base[2])
            };
            rgba[index] = cr;
            rgba[index + 1] = cg;
            rgba[index + 2] = cb;
            rgba[index + 3] = (coverage * 255.0) as u8;
        }
    }

    egui::IconData {
        rgba,
        width: SIZE,
        height: SIZE,
    }
}
