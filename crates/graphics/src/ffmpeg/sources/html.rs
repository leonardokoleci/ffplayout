//! Live HTML graphics rendered by a headless browser over the DevTools protocol.
//! Each source owns one browser process; a tracked worker captures screenshots
//! continuously while the layer is visible and parks when it is not.
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use arc_swap::ArcSwapOption;
use ffmpeg_next::frame::Video;
use headless_chrome::{Browser, LaunchOptions, Tab};
use url::Url;

use crate::{
    bitmap::StraightRgbaBitmap,
    preparation::{
        Cancellation, MediaTask, PreparationService, PreparationWeak, ResourceBudget, ResourceLease,
    },
    surface::{ColorEncoding, Resource, ResourceKey, SurfaceDescriptor},
};

use crate::ffmpeg::{
    compositor::{
        metadata::yuva_descriptor,
        overlay::{OverlayFrame, alpha_bounds, chroma_alpha},
        scene::CpuScene,
    },
    text::rgba_to_yuva420p,
};

use super::animation::PreparedSurface;

/// The rendered page is bounded to keep screenshots and composition bounded.
const MAX_SIZE: u32 = 8192;
const DEFAULT_SIZE: (u32, u32) = (1280, 720);
/// Cooperative preparation deadline, matching the other source preparations.
const PREPARE_TIMEOUT: Duration = Duration::from_secs(10);
const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
/// Screenshots cost real render work; pacing avoids burning CPU for tiny pages.
const CAPTURE_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug)]
pub enum HtmlTarget<'a> {
    File(&'a Path),
    Url(&'a str),
}

impl HtmlTarget<'_> {
    fn navigate_url(self) -> Result<String> {
        match self {
            Self::File(path) => {
                let url = Url::from_file_path(path)
                    .map_err(|_| anyhow::anyhow!("invalid HTML file path {path:?}"))?;

                Ok(url.to_string())
            }
            Self::Url(url) => {
                ensure!(
                    url.to_ascii_lowercase().starts_with("http://")
                        || url.to_ascii_lowercase().starts_with("https://"),
                    "HTML graphic URLs must use http or https"
                );

                Ok(url.to_string())
            }
        }
    }
}

struct Shared {
    latest: ArcSwapOption<OverlayFrame>,
    inactive: AtomicBool,
    failed: AtomicBool,
}

pub(in crate::ffmpeg) struct HtmlSource {
    shared: Arc<Shared>,
    poster: Arc<OverlayFrame>,
    task: MediaTask,
    wanted: AtomicBool,
}

struct CaptureTarget {
    budget: ResourceBudget,
    frame_bytes: usize,
    width: u32,
    height: u32,
}

impl HtmlSource {
    fn start(
        tab: Arc<Tab>,
        browser: Browser,
        poster: Arc<OverlayFrame>,
        target: CaptureTarget,
        service: &PreparationService,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            latest: ArcSwapOption::new(Some(poster.clone())),
            inactive: AtomicBool::new(true),
            failed: AtomicBool::new(false),
        });
        let state = shared.clone();
        let context = CaptureContext {
            tab,
            shared: state,
            budget: target.budget,
            frame_bytes: target.frame_bytes,
            width: target.width,
            height: target.height,
        };
        let task = service.spawn_media(move |stop| run_capture_loop(context, browser, stop))?;

        Ok(Self {
            shared,
            poster,
            task,
            wanted: AtomicBool::new(false),
        })
    }

    pub(in crate::ffmpeg) fn at(&self, _elapsed_us: i64) -> Option<Arc<OverlayFrame>> {
        self.wanted.store(true, Ordering::Release);
        if self.shared.inactive.swap(false, Ordering::AcqRel) {
            self.task.wake();
        }

        self.shared.latest.load_full()
    }

    pub(in crate::ffmpeg) fn poster(&self) -> Arc<OverlayFrame> {
        self.poster.clone()
    }

    pub(in crate::ffmpeg) fn prewarm(&self, elapsed_us: i64) {
        self.at(elapsed_us);
    }

    pub(in crate::ffmpeg) fn begin_frame(&self, _epoch: i64) {
        self.wanted.store(false, Ordering::Release);
    }

    pub(in crate::ffmpeg) fn end_frame(&self) {
        if !self.wanted.load(Ordering::Acquire) {
            self.suspend();
        }
    }

    pub(in crate::ffmpeg) fn suspend(&self) {
        self.begin_frame(i64::MIN);
        if self.shared.inactive.swap(true, Ordering::AcqRel) {
            self.task.wake();
        }
    }

    pub(in crate::ffmpeg) fn stop(&self) {
        self.shared.inactive.store(true, Ordering::Release);
        self.task.cancel();
    }
}

struct PreparedFrame {
    surface: Arc<OverlayFrame>,
    descriptor: SurfaceDescriptor,
}

struct CaptureContext {
    tab: Arc<Tab>,
    shared: Arc<Shared>,
    budget: ResourceBudget,
    frame_bytes: usize,
    width: u32,
    height: u32,
}

fn run_capture_loop(context: CaptureContext, browser: Browser, stop: Cancellation) {
    let CaptureContext {
        tab,
        shared,
        budget,
        frame_bytes,
        width,
        height,
    } = context;
    // Dropping the browser terminates the child process when the loop exits.
    let _keep_alive = browser;
    while !stop.is_cancelled() {
        if shared.inactive.load(Ordering::Acquire) {
            // Release the prepared pixels; the poster stays available in HtmlSource.
            shared.latest.store(None);
            thread::park();
            continue;
        }
        if let Ok(lease) = budget.reserve(frame_bytes) {
            let captured = capture_overlay(&tab, width, height, lease)
                .inspect(|overlay| shared.latest.store(Some(overlay.clone())));
            if let Err(error) = captured
                && !shared.failed.swap(true, Ordering::AcqRel)
            {
                log::warn!("HTML graphic capture failed; retaining ready pixels: {error:#}");
            }
        }
        thread::park_timeout(CAPTURE_INTERVAL);
    }
}

fn capture_overlay(
    tab: &Tab,
    width: u32,
    height: u32,
    lease: ResourceLease,
) -> Result<Arc<OverlayFrame>> {
    let frame = capture_yuva(tab, width, height)?;
    let alpha = chroma_alpha(&frame, width, height);
    let bounds = alpha_bounds(&frame);
    let overlay = Arc::new(OverlayFrame {
        lease: Some(lease),
        frame,
        bounds,
        chroma_alpha: alpha,
        width,
        height,
        x: 0,
        y: 0,
        opacity: 255,
    });

    Ok(overlay)
}

fn capture_yuva(tab: &Tab, width: u32, height: u32) -> Result<Video> {
    let format = headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png;
    let png = tab.capture_screenshot(format, None, None, true)?;
    let decoded = image::load_from_memory(&png)
        .context("decoding browser screenshot")?
        .into_rgba8();
    let (decoded_width, decoded_height) = (decoded.width(), decoded.height());
    ensure!(
        decoded_width == width && decoded_height == height,
        "browser screenshot is {decoded_width}x{decoded_height}, expected {width}x{height}"
    );

    let bitmap = StraightRgbaBitmap {
        width,
        height,
        pixels: decoded.into_raw(),
    };

    rgba_to_yuva420p(&bitmap)
}

fn launch_browser(width: u32, height: u32, configured: Option<&Path>) -> Result<Browser> {
    let mut builder = LaunchOptions::default_builder();
    if let Some(path) = browser_binary(configured) {
        builder.path(Some(path));
    }
    builder.window_size(Some((width, height)));
    builder.args(vec![
        OsStr::new("--disable-gpu"),
        OsStr::new("--hide-scrollbars"),
        OsStr::new("--mute-audio"),
        OsStr::new("--no-first-run"),
    ]);
    let options = builder
        .build()
        .map_err(|error| anyhow::anyhow!("building headless browser options failed: {error}"))?;

    Browser::new(options).context(
        "launching a headless browser for HTML graphics (install Chrome, Chromium or Edge, \
         or set FFP_HTML_BROWSER to the browser executable)",
    )
}

fn browser_binary(configured: Option<&Path>) -> Option<PathBuf> {
    configured
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os("FFP_HTML_BROWSER")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
}

fn screenshot_asset(
    tab: &Tab,
    width: u32,
    height: u32,
    budget: Option<&ResourceBudget>,
) -> Result<PreparedFrame> {
    let frame = capture_yuva(tab, width, height)?;
    let alpha = chroma_alpha(&frame, width, height);
    let bounds = alpha_bounds(&frame);
    let descriptor = yuva_descriptor(&frame, bounds, alpha.len(), ColorEncoding::Bt601Limited)?;
    let lease = budget
        .map(|budget| budget.reserve(descriptor.storage_bytes))
        .transpose()?;
    let surface = Arc::new(OverlayFrame {
        lease,
        frame,
        bounds,
        chroma_alpha: alpha,
        width,
        height,
        x: 0,
        y: 0,
        opacity: 255,
    });

    Ok(PreparedFrame {
        surface,
        descriptor,
    })
}

impl CpuScene {
    /// Prepare a live HTML graphic on a preparation worker: launch a headless
    /// browser, load the page, capture the poster and start the capture worker.
    /// The browser process is owned by the worker and terminates with it.
    pub fn prepare_html(
        &mut self,
        key: ResourceKey,
        target: HtmlTarget<'_>,
        size: Option<(u32, u32)>,
        browser: Option<&Path>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Arc<Resource>> {
        let (width, height) = size.unwrap_or(DEFAULT_SIZE);
        ensure!(
            width > 0
                && height > 0
                && width <= MAX_SIZE
                && height <= MAX_SIZE
                && width.is_multiple_of(2)
                && height.is_multiple_of(2),
            "HTML size must be even and within 2..{MAX_SIZE}"
        );
        let url = target.navigate_url()?;
        let started = Instant::now();
        let cancelled = || cancelled() || started.elapsed() > PREPARE_TIMEOUT;
        ensure!(!cancelled(), "HTML graphic preparation cancelled");
        let browser = launch_browser(width, height, browser)?;
        let tab = browser.new_tab()?;
        tab.set_default_timeout(LOAD_TIMEOUT);
        tab.set_transparent_background_color()?;
        tab.navigate_to(&url)?;
        tab.wait_until_navigated()?;
        // Give scripts a short settle window before the poster capture.
        thread::sleep(Duration::from_millis(100));
        ensure!(!cancelled(), "HTML graphic preparation cancelled");
        let asset = screenshot_asset(&tab, width, height, self.budget.as_ref())?;
        let poster = asset.surface.clone();
        let frame_bytes = asset.descriptor.storage_bytes;
        let Some(service) = self.preparation.as_ref().and_then(PreparationWeak::upgrade) else {
            // Without a channel service there is no worker slot; publish the
            // static capture rather than failing the take.
            return self.surfaces.insert(
                key,
                asset.descriptor,
                Arc::new(PreparedSurface::still(poster)),
            );
        };

        let live = HtmlSource::start(
            tab,
            browser,
            poster,
            CaptureTarget {
                budget: service.budget(),
                frame_bytes,
                width,
                height,
            },
            &service,
        )?;
        let poster = live.poster();

        self.surfaces.insert(
            key,
            asset.descriptor,
            Arc::new(PreparedSurface::live(live, poster)),
        )
    }
}

impl Drop for HtmlSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn storage() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests_assets/storage")
    }

    #[test]
    fn html_sizes_and_targets_are_validated_before_any_browser_is_launched() {
        let mut cpu = CpuScene::new(1, 1024 * 1024);
        let file = storage().join("html_test_page.html");
        for size in [Some((3, 4)), Some((0, 8)), Some((8194, 8))] {
            assert!(
                cpu.prepare_html(
                    ResourceKey { id: 1, revision: 1 },
                    HtmlTarget::File(&file),
                    size,
                    None,
                    || false,
                )
                .is_err()
            );
        }
        assert!(
            cpu.prepare_html(
                ResourceKey { id: 1, revision: 1 },
                HtmlTarget::Url("ftp://example.invalid/overlay.html"),
                Some((8, 8)),
                None,
                || false,
            )
            .is_err()
        );
    }

    /// Browser rendering depends on an installed Chrome/Chromium/Edge binary;
    /// the remaining behavior is exercised when one is available.
    #[test]
    fn local_html_file_prepares_a_live_endless_source() {
        let service = crate::preparation::PreparationService::default();
        let mut cpu = CpuScene::new(1, 64 * 1024 * 1024).with_preparation(&service);
        let Ok(resource) = cpu.prepare_html(
            ResourceKey { id: 1, revision: 1 },
            HtmlTarget::File(&storage().join("html_test_page.html")),
            Some((64, 32)),
            None,
            || false,
        ) else {
            eprintln!("skipping: no headless browser available");
            assert!(service.shutdown(Duration::from_secs(1)));
            return;
        };
        let prepared = cpu.surfaces.get(&resource).unwrap();
        assert!(prepared.end_us().is_none(), "a live page never ends");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if prepared.select(0, (0, 1, 0)).is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "HTML source never produced pixels"
            );
            thread::sleep(Duration::from_millis(5));
        }
        cpu.stop_streams();
        assert!(service.shutdown(Duration::from_secs(2)));
    }
}
