//! `--screenshot <png>`: render a few frames, save the window contents and exit. Used to
//! check the launcher's look in automated runs.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use egui::{ColorImage, Event, UserData, ViewportCommand};

/// Gives up if the backend never delivers the screenshot.
const TIMEOUT: Duration = Duration::from_secs(20);

pub struct Capture {
    path: PathBuf,
    frames_left: u32,
    requested: bool,
    started: Instant,
    timeout: Duration,
}

impl Capture {
    pub fn new(path: PathBuf, frames: u32) -> Self {
        Capture { path, frames_left: frames, requested: false, started: Instant::now(), timeout: TIMEOUT }
    }

    /// Report captures may include a first-time scan of a large installed mod.
    pub fn wait_for_validation(&mut self) { self.timeout = Duration::from_secs(180); }

    /// Call once per frame, after the UI was built; the frames are counted once `ready` (what
    /// the page shows has loaded).
    pub fn tick(&mut self, ctx: &egui::Context, ready: bool) {
        ctx.request_repaint();
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            match save_png(&self.path, &image) {
                Ok(()) => log::info!("screenshot saved to {}", self.path.display()),
                Err(e) => log::error!("screenshot failed: {e:#}"),
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
        } else if self.started.elapsed() > self.timeout {
            log::error!("screenshot timed out");
            ctx.send_viewport_cmd(ViewportCommand::Close);
        } else if ready && self.frames_left > 0 {
            self.frames_left -= 1;
        } else if ready && !self.requested {
            self.requested = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
        }
    }
}

fn save_png(path: &Path, image: &ColorImage) -> anyhow::Result<()> {
    let [width, height] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let buffer = image::RgbaImage::from_raw(width as u32, height as u32, rgba).context("screenshot has an invalid size")?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    buffer.save(path).with_context(|| format!("writing {}", path.display()))
}
