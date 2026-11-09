//! `--film <script.toml>`: the launcher filmed for TA Reborn's videos. A script of pointer moves,
//! clicks, wheel turns, pages and mod downloads plays on a steady clock (`fps` frames a second,
//! however long each takes to draw and save); every frame is saved as a PNG
//! (`<out>/f_00000.png`...), the interface's sounds are written down in the game's sound log
//! format (`<out>/sounds.jsonl`, which `scratch/fan/sfx/mix.py` mixes with `--with interface`),
//! and the launcher closes. The real mouse and keyboard are left out; a pointer is drawn where the
//! script has it. Nothing is launched or opened.
//!
//! ```toml
//! fps = 60
//! seconds = 20            # the film's length
//! pixels = [3840, 2160]   # the frames (the window's inside, without a frame), with the pages
//! points = [1280, 720]    #   laid out as on a window of `points`
//! at = [2560, 0]          # where the window goes (points of the desktop; centred when left out)
//! out = "frames"          # beside the script
//! pointer = [700, 420]    # where the pointer starts (points)
//! lead = 0.5              # seconds the launcher runs, everything loaded, before the first frame
//!
//! [[step]]                # each `t` seconds into the film
//! t = 0.4
//! do = "move"             # the pointer goes to `to` in `seconds` (0.6), as a hand moves it
//! to = [100, 179]
//! [[step]]
//! t = 1.1
//! do = "click"            # pressed, and let go `hold` seconds later (0.08); `button` "secondary"
//!                         #   for a right-click (a library's menu), "middle", else "primary"
//! [[step]]
//! t = 1.6
//! do = "key"              # a key pressed and let go (egui's name for it: "Escape", "F10",
//! key = "F10"             #   "ArrowDown", "Enter"...), with `shift`, `ctrl` or `alt` held
//! shift = true
//! [[step]]
//! t = 2.0
//! do = "wheel"            # the wheel turned `down` points down the page (up when negative)
//! down = 600              #   over `seconds` (0.8)
//! [[step]]
//! t = 0
//! do = "page"             # straight to a page: play, mods, settings or about (and the Mods
//! page = "mods"           #   page's `tab`: get or installed)
//! [[step]]
//! t = 5.0
//! do = "download"         # a catalogue mod's download played over `seconds` (3), then installed
//! id = "totalmayhem"      #   for real from `file` (the film waits for it, its clock stopped)
//! file = "C:/Users/me/Downloads/TotalM1130.zip"
//! ```

use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use egui::{
    Color32, ColorImage, Event, Id, LayerId, Mesh, Modifiers, MouseWheelUnit, Order, Pos2,
    PointerButton, Shape, Stroke, TouchPhase, UserData, Vec2, ViewportCommand, pos2, vec2,
};
use serde::Deserialize;

use crate::catalogue::{self, Entry};
use crate::getmods::{Installer, Stage};

/// Threads saving frames, and frames waiting for them.
const SAVERS: usize = 6;
const QUEUED: usize = 12;
/// The film gives up when the window sends no frame back for this long.
const STALLED: Duration = Duration::from_secs(30);
/// A download's other stages, in seconds of the film.
const CONNECTING: f64 = 0.35;
const CHECKING: f64 = 0.4;
const UNPACKING: f64 = 0.5;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Script {
    #[serde(default = "default_fps")]
    fps: u32,
    seconds: f64,
    pixels: [u32; 2],
    points: [f32; 2],
    #[serde(default)]
    at: Option<[f32; 2]>,
    #[serde(default = "default_out")]
    out: PathBuf,
    #[serde(default)]
    pointer: Option<[f32; 2]>,
    #[serde(default = "default_lead")]
    lead: f64,
    #[serde(default, rename = "step")]
    steps: Vec<Step>,
}

fn default_fps() -> u32 {
    60
}

fn default_out() -> PathBuf {
    PathBuf::from("frames")
}

fn default_lead() -> f64 {
    0.5
}

fn move_seconds() -> f64 {
    0.6
}

fn click_hold() -> f64 {
    0.08
}

fn wheel_seconds() -> f64 {
    0.8
}

fn download_seconds() -> f64 {
    3.0
}

/// One thing the script does.
#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Step {
    Move {
        t: f64,
        to: [f32; 2],
        #[serde(default = "move_seconds")]
        seconds: f64,
    },
    Click {
        t: f64,
        #[serde(default = "click_hold")]
        hold: f64,
        #[serde(default)]
        button: Button,
    },
    Key {
        t: f64,
        key: String,
        #[serde(default)]
        shift: bool,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
    },
    Wheel {
        t: f64,
        down: f32,
        #[serde(default = "wheel_seconds")]
        seconds: f64,
    },
    Page {
        t: f64,
        page: String,
        #[serde(default)]
        tab: Option<String>,
    },
    Download {
        t: f64,
        id: String,
        file: PathBuf,
        #[serde(default = "download_seconds")]
        seconds: f64,
    },
}

impl Step {
    fn t(&self) -> f64 {
        match self {
            Step::Move { t, .. } | Step::Click { t, .. } | Step::Key { t, .. } | Step::Wheel { t, .. } | Step::Page { t, .. } | Step::Download { t, .. } => *t,
        }
    }
}

/// The mouse button a `click` step presses.
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    #[default]
    Primary,
    Secondary,
    Middle,
}

impl From<Button> for PointerButton {
    fn from(button: Button) -> Self {
        match button {
            Button::Primary => PointerButton::Primary,
            Button::Secondary => PointerButton::Secondary,
            Button::Middle => PointerButton::Middle,
        }
    }
}

/// A move of the pointer, from where it was when the move began.
struct Glide {
    t: f64,
    seconds: f64,
    from: Pos2,
    to: Pos2,
}

/// A download being played: its stages set each frame, then the real install waited for.
struct Download {
    entry: &'static Entry,
    file: PathBuf,
    t: f64,
    seconds: f64,
    installing: bool,
}

pub struct Film {
    script: Script,
    out: PathBuf,
    glides: Vec<Glide>,
    start: Pos2,
    /// egui's clock: it moves on a frame at a time while the film warms up and rolls, and stands
    /// still while it waits.
    time: f64,
    /// The film's frame on screen now (`None` while warming up).
    frame: Option<u64>,
    /// Frames drawn with everything loaded, before the first.
    settled: u32,
    /// The film's seconds up to which the script's input has been given to egui.
    told_to: f64,
    /// Where egui was last told the pointer is, and when its button last went down.
    pointer: Option<Pos2>,
    pressed_at: Option<f64>,
    /// Pages and downloads due, for the launcher to carry out.
    due: Vec<Step>,
    downloads: Vec<Download>,
    /// A real install under way: the clock stands still.
    waiting: bool,
    /// When the window was last asked for the film's size.
    resized_at: Option<f64>,
    opened: Instant,
    saving: Option<SyncSender<(u64, Arc<ColorImage>)>>,
    savers: Vec<JoinHandle<()>>,
    kept: u64,
    /// When a frame last came back (or the film began, or waited).
    last_news: Instant,
    sounds: BufWriter<File>,
    voices: u64,
    finished: bool,
}

impl Film {
    /// Reads the script and makes ready its folder (frames from an earlier take removed).
    pub fn open(path: &Path) -> anyhow::Result<Film> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut script: Script = toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
        anyhow::ensure!(script.fps > 0 && script.seconds > 0.0, "the film needs fps and seconds");
        script.steps.sort_by(|a, b| a.t().total_cmp(&b.t()));
        let out = path.parent().unwrap_or(Path::new(".")).join(&script.out);
        std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
        for old in std::fs::read_dir(&out)?.flatten() {
            let name = old.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("f_") && name.ends_with(".png") {
                std::fs::remove_file(old.path())?;
            }
        }
        // The pointer's path: each move from where the one before had it at the move's start.
        let start = script.pointer.map_or(pos2(script.points[0] * 0.7, script.points[1] * 0.6), |[x, y]| pos2(x, y));
        let mut glides: Vec<Glide> = Vec::new();
        for step in &script.steps {
            if let Step::Move { t, to, seconds } = *step {
                let from = pointer_at(&glides, start, t);
                glides.push(Glide { t, seconds: seconds.max(1e-3), from, to: pos2(to[0], to[1]) });
            }
        }
        let mut sounds = BufWriter::new(File::create(out.join("sounds.jsonl")).context("the sound log")?);
        writeln!(sounds, "{}", sound_log_header(script.fps))?;
        // Frames are saved on threads of their own, a few waiting at most.
        let (saving, frames) = sync_channel::<(u64, Arc<ColorImage>)>(QUEUED);
        let frames = Arc::new(Mutex::new(frames));
        let savers = (0..SAVERS)
            .map(|k| {
                let (frames, out) = (frames.clone(), out.clone());
                std::thread::Builder::new().name(format!("film frames {k}")).spawn(move || save_frames(&frames, &out)).expect("a thread saving frames")
            })
            .collect();
        log::info!("filming {} s at {} frames a second into {}", script.seconds, script.fps, out.display());
        Ok(Film {
            out,
            glides,
            start,
            time: 0.0,
            frame: None,
            settled: 0,
            told_to: f64::NEG_INFINITY,
            pointer: None,
            pressed_at: None,
            due: Vec::new(),
            downloads: Vec::new(),
            waiting: false,
            resized_at: None,
            opened: Instant::now(),
            saving: Some(saving),
            savers,
            kept: 0,
            last_news: Instant::now(),
            sounds,
            voices: 0,
            finished: false,
            script,
        })
    }

    /// The window's size when it opens (points), and where it goes.
    pub fn window(&self) -> ([f32; 2], Option<[f32; 2]>) {
        (self.script.points, self.script.at)
    }

    fn dt(&self) -> f64 {
        1.0 / self.script.fps as f64
    }

    fn frames(&self) -> u64 {
        (self.script.seconds * self.script.fps as f64).round() as u64
    }

    /// Seconds into the film of the frame on screen (0 while warming up).
    fn now(&self) -> f64 {
        self.frame.map_or(0.0, |f| f as f64 * self.dt())
    }

    /// Once a frame, before egui reads its input: the clock set, the real input left out and
    /// the script's given instead.
    pub fn input(&mut self, raw: &mut egui::RawInput) {
        raw.events.retain(|e| matches!(e, Event::Screenshot { .. }));
        raw.hovered_files.clear();
        raw.dropped_files.clear();
        // A film supplies the interaction state as well as the pointer and clock.
        // Capturing in the background must not freeze animations or replace the
        // contextual Reborn pointer with the film's system-pointer fallback. This
        // changes only synthetic input; it never focuses or restores the OS window.
        raw.focused = true;
        let viewport = raw.viewports.entry(raw.viewport_id).or_default();
        viewport.focused = Some(true);
        viewport.minimized = Some(false);
        raw.time = Some(self.time);
        raw.predicted_dt = self.dt() as f32;
        let (from, to) = (self.told_to, self.now());
        if to <= from {
            // (The clock stands still: nothing new happens.)
            return;
        }
        self.told_to = to;
        let at = pointer_at(&self.glides, self.start, to);
        if self.pointer != Some(at) {
            raw.events.push(Event::PointerMoved(at));
            self.pointer = Some(at);
        }
        for step in &self.script.steps {
            match *step {
                Step::Click { t, hold, button } => {
                    for (when, pressed) in [(t, true), (t + hold, false)] {
                        if from < when && when <= to {
                            raw.events.push(Event::PointerButton { pos: at, button: button.into(), pressed, modifiers: Modifiers::NONE });
                            if pressed {
                                self.pressed_at = Some(self.time);
                            }
                        }
                    }
                }
                Step::Key { t, ref key, shift, ctrl, alt } if from < t && t <= to => match egui::Key::from_name(key) {
                    Some(key) => {
                        let modifiers = Modifiers { alt, ctrl, shift, mac_cmd: false, command: ctrl };
                        for pressed in [true, false] {
                            raw.events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
                        }
                    }
                    None => log::warn!("film: no key called {key}"),
                },
                Step::Wheel { t, down, seconds } => {
                    let turned = |s: f64| down * smoothstep(((s - t) / seconds.max(1e-3)).clamp(0.0, 1.0)) as f32;
                    let by = turned(to) - turned(from.max(t));
                    if by != 0.0 {
                        raw.events.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, -by), phase: TouchPhase::Move, modifiers: Modifiers::NONE });
                    }
                }
                Step::Page { t, .. } | Step::Download { t, .. } if from < t && t <= to => self.due.push(step.clone()),
                _ => {}
            }
        }
    }

    /// The pages and downloads due now, for the launcher to carry out.
    pub fn take_due(&mut self) -> Vec<Step> {
        std::mem::take(&mut self.due)
    }

    /// Starts playing a catalogue mod's download (a `download` step).
    pub fn download(&mut self, id: &str, file: PathBuf, seconds: f64) {
        match catalogue::get(id) {
            Some(entry) if entry.file.is_some() => self.downloads.push(Download { entry, file, t: self.now(), seconds, installing: false }),
            _ => log::warn!("film: no download for {id}"),
        }
    }

    /// Once a frame, before the pages: the downloads' stages as the film's clock has them; a
    /// finished one installed for real, the clock stopped until it is in.
    pub fn play_downloads(&mut self, installer: &mut Installer) {
        let now = self.now();
        let mut waiting = false;
        self.downloads.retain_mut(|d| {
            let id = d.entry.id;
            let total = d.entry.file.map_or(1, |f| f.bytes);
            let at = now - d.t;
            let downloaded = CONNECTING + d.seconds;
            if d.installing {
                let busy = installer.stages.get(id).is_some_and(Stage::active);
                waiting |= busy;
                return busy;
            }
            let stage = if at < CONNECTING {
                Stage::Connecting
            } else if at < downloaded {
                let gone = at - CONNECTING;
                let got = (total as f64 * (gone / d.seconds).clamp(0.0, 1.0)) as u64;
                // (The page reckons the time left from when the download began.)
                let since = Instant::now().checked_sub(Duration::from_secs_f64(gone)).unwrap_or_else(Instant::now);
                Stage::Downloading { got, total, since }
            } else if at < downloaded + CHECKING {
                Stage::Checking
            } else if at < downloaded + CHECKING + UNPACKING {
                Stage::Unpacking
            } else {
                // The real install, from the file: the page shows it unpacking meanwhile.
                installer.stages.remove(id);
                installer.install(d.entry, Some(d.file.clone()));
                d.installing = true;
                waiting = true;
                Stage::Unpacking
            };
            installer.stages.insert(id.to_string(), stage);
            true
        });
        self.waiting = waiting;
    }

    /// Once a pass, after the frame's interface: the pointer drawn and, once a frame (the last
    /// pass), the frames that came back saved, the sound asked for written down and the clock
    /// moved on. `ready`: everything the film shows has loaded.
    pub fn after(&mut self, ctx: &egui::Context, ready: bool, sound: Option<String>) {
        self.paint_pointer(ctx);
        if ctx.will_discard() || self.finished {
            return;
        }
        ctx.request_repaint();
        let shots: Vec<(u64, Arc<ColorImage>)> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    Event::Screenshot { user_data, image, .. } => user_data.data.as_ref()?.downcast_ref::<u64>().map(|&f| (f, image.clone())),
                    _ => None,
                })
                .collect()
        });
        for (frame, image) in shots {
            if frame == 0 && image.size != self.script.pixels.map(|p| p as usize) {
                log::warn!("film: frames are {:?}, not {:?}", image.size, self.script.pixels);
            }
            if let Some(saving) = &self.saving {
                let _ = saving.send((frame, image));
            }
            self.kept += 1;
            self.last_news = Instant::now();
        }
        if let (Some(frame), Some(path)) = (self.frame, sound) {
            self.write_sound(frame, &path);
        }
        if self.kept >= self.frames() {
            self.finish(ctx);
            return;
        }
        if self.waiting {
            self.last_news = Instant::now();
        } else if self.frame.is_some() && self.last_news.elapsed() > STALLED {
            log::error!("film: no frame came back for {} s; stopping", STALLED.as_secs());
            self.finish(ctx);
            return;
        }
        match self.frame {
            None => {
                let sized = self.fit(ctx);
                // (Something that never loads holds the film up for half a minute at most.)
                let late = self.opened.elapsed() > STALLED;
                if (ready || late) && sized {
                    if !ready && self.settled == 0 {
                        log::warn!("film: starting before everything has loaded");
                    }
                    self.settled += 1;
                }
                self.time += self.dt();
                if f64::from(self.settled) >= self.script.lead * self.script.fps as f64 {
                    self.last_news = Instant::now();
                    self.roll(ctx, 0);
                }
            }
            Some(frame) if !self.waiting && frame + 1 < self.frames() => {
                self.time += self.dt();
                self.roll(ctx, frame + 1);
            }
            _ => {}
        }
    }

    /// The pages at the film's points and the window at its pixels (asked for again until it
    /// is); true once it is.
    fn fit(&mut self, ctx: &egui::Context) -> bool {
        let [w, h] = self.script.pixels.map(|p| p as f32);
        let zoom = w / self.script.points[0] / ctx.native_pixels_per_point().unwrap_or(1.0);
        if (ctx.zoom_factor() - zoom).abs() > 1e-4 {
            ctx.set_zoom_factor(zoom);
            return false;
        }
        let ppp = ctx.pixels_per_point();
        let size = ctx.content_rect().size() * ppp;
        if (size.x.round(), size.y.round()) == (w, h) {
            return true;
        }
        if self.resized_at.is_none_or(|at| self.time - at > 0.25) {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(w, h) / ppp));
            self.resized_at = Some(self.time);
        }
        false
    }

    /// The next frame is the film's `frame`: kept (a screenshot asked for now is of the next
    /// frame drawn).
    fn roll(&mut self, ctx: &egui::Context, frame: u64) {
        self.frame = Some(frame);
        ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(frame)));
    }

    fn write_sound(&mut self, frame: u64, path: &str) {
        let line = sound_line(frame as f64 * self.dt(), frame, self.voices, path);
        self.voices += 1;
        if let Err(e) = writeln!(self.sounds, "{line}") {
            log::warn!("film: the sound log: {e}");
        }
    }

    fn finish(&mut self, ctx: &egui::Context) {
        self.finished = true;
        drop(self.saving.take());
        for saver in self.savers.drain(..) {
            let _ = saver.join();
        }
        let _ = self.sounds.flush();
        log::info!("film: {} frames and {} sounds in {}", self.kept, self.voices, self.out.display());
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    /// The pointer, over everything: Windows' arrow, and a ring opening out at each press.
    fn paint_pointer(&self, ctx: &egui::Context) {
        let Some(at) = self.pointer else { return };
        let painter = ctx.layer_painter(LayerId::new(Order::Debug, Id::new("film pointer")));
        if let Some(pressed) = self.pressed_at {
            let k = ((self.time - pressed) / 0.45) as f32;
            if (0.0..1.0).contains(&k) {
                let open = 1.0 - (1.0 - k).powi(2);
                painter.circle_stroke(at, 4.0 + 16.0 * open, Stroke::new(0.6 + 1.8 * (1.0 - k), Color32::from_rgba_unmultiplied(255, 214, 128, (210.0 * (1.0 - k)) as u8)));
            }
        }
        // The launcher may already have drawn its contextual Reborn pointer.
        if ctx.output(|output| output.cursor_icon == egui::CursorIcon::None) { return; }
        // (The outline, the tip at 0,0; filled as a fan from a point inside that sees all of it.)
        const ARROW: [[f32; 2]; 7] = [[0.0, 0.0], [0.0, 17.2], [4.1, 13.4], [6.9, 19.9], [9.6, 18.8], [6.8, 12.4], [12.2, 12.4]];
        const INSIDE: [f32; 2] = [4.5, 11.6];
        const SCALE: f32 = 1.1;
        let place = |[x, y]: [f32; 2], by: Vec2| at + by + vec2(x, y) * SCALE;
        let fan = |by: Vec2, color: Color32| {
            let mut mesh = Mesh::default();
            mesh.colored_vertex(place(INSIDE, by), color);
            for p in ARROW {
                mesh.colored_vertex(place(p, by), color);
            }
            let n = ARROW.len() as u32;
            for i in 0..n {
                mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
            }
            Shape::mesh(mesh)
        };
        painter.add(fan(vec2(1.0, 1.6), Color32::from_black_alpha(70)));
        painter.add(fan(Vec2::ZERO, Color32::WHITE));
        let outline: Vec<Pos2> = ARROW.iter().map(|&p| place(p, Vec2::ZERO)).collect();
        painter.add(Shape::closed_line(outline, Stroke::new(1.0, Color32::BLACK)));
    }
}

/// Where the pointer is `t` seconds into the film: on its way along the move under way (as a
/// hand moves it: quick, then easing in, along a slight arc), else where the last move left it.
fn pointer_at(glides: &[Glide], start: Pos2, t: f64) -> Pos2 {
    let Some(g) = glides.iter().rev().find(|g| g.t <= t) else { return start };
    let k = ((t - g.t) / g.seconds).clamp(0.0, 1.0);
    // The minimum-jerk path a reaching hand takes.
    let s = (k * k * k * (10.0 - 15.0 * k + 6.0 * k * k)) as f32;
    let d = g.to - g.from;
    let bow = vec2(-d.y, d.x) * 0.06 * (std::f32::consts::PI * s).sin();
    g.from + d * s + bow
}

fn smoothstep(k: f64) -> f64 {
    k * k * (3.0 - 2.0 * k)
}

/// The sound log's first line (the game's format, `apps/ta_game/src/sound_log.rs`): the clock,
/// and the volumes of its buses (no music plays).
fn sound_log_header(fps: u32) -> serde_json::Value {
    serde_json::json!({"ev": "log", "version": 1, "rate": 48000, "fps": fps, "master": 1.0, "music": 0.0, "effects": 1.0, "voice": 1.0, "max_voices": 64})
}

/// An interface sound starting `t` seconds into the film, on voice `id`: TA's are mono, played
/// in the middle (the mixer's pan law), on the effects bus as the game plays them.
fn sound_line(t: f64, frame: u64, id: u64, path: &str) -> serde_json::Value {
    let gain = 0.8;
    let side = gain * std::f64::consts::FRAC_1_SQRT_2;
    serde_json::json!({
        "t": t, "frame": frame, "ev": "play", "id": id, "kind": "interface", "path": path,
        "gain": gain, "pan": 0.0, "gains": [side, side], "pitch": 1.0, "loop": false, "at": 0.0, "fade": 0.0, "bus": "effects",
    })
}

/// A thread's share of the frames, saved as PNGs (fast compression: there are many).
fn save_frames(frames: &Mutex<Receiver<(u64, Arc<ColorImage>)>>, out: &Path) {
    use image::ImageEncoder as _;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    loop {
        let Some((frame, image)) = frames.lock().ok().and_then(|f| f.recv().ok()) else { return };
        let [w, h] = image.size;
        let rgb: Vec<u8> = image.pixels.iter().flat_map(|c| {
            let [r, g, b, _] = c.to_srgba_unmultiplied();
            [r, g, b]
        }).collect();
        let path = out.join(format!("f_{frame:05}.png"));
        let saved = File::create(&path).map_err(anyhow::Error::from).and_then(|f| {
            PngEncoder::new_with_quality(BufWriter::new(f), CompressionType::Fast, FilterType::Adaptive)
                .write_image(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8)
                .map_err(anyhow::Error::from)
        });
        if let Err(e) = saved {
            log::error!("film: {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_input_keeps_contextual_cursors_and_animation_independent_of_os_focus() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("ta_launcher_film_focus_{}_{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let script = root.join("scene.toml");
        std::fs::write(&script, "fps = 12\nseconds = 1.0\npixels = [800,600]\npoints = [800.0,600.0]\npointer = [50.0,60.0]\n").unwrap();
        let mut film = Film::open(&script).unwrap();
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let mut raw = egui::RawInput { focused: false, screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800., 600.))),
                events: vec![Event::Text("real keyboard input".into())], ..Default::default() };
            let viewport = raw.viewports.get_mut(&raw.viewport_id).unwrap();
            viewport.focused = Some(false);
            viewport.minimized = Some(true);
            film.input(&mut raw);
            assert!(raw.focused);
            assert_eq!(raw.viewports[&raw.viewport_id].focused, Some(true));
            assert_eq!(raw.viewports[&raw.viewport_id].minimized, Some(false));
            assert!(!raw.events.iter().any(|event| matches!(event, Event::Text(_))));
            let output = ctx.run_ui(raw, |ui| {
                let focused = ui.input(|i| i.viewport().focused.unwrap_or(false));
                let minimized = ui.input(|i| i.viewport().minimized.unwrap_or(true));
                ta_ui::motion::set_still(ui.ctx(), !focused || minimized);
                assert!(!ta_ui::motion::still(ui.ctx()));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                crate::cursor::show(ui.ctx(), &ta_config::Config::default());
            });
            assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::None);
            output.drop_without_applying_deltas();
        }
        film.finish(&ctx);
        drop(film);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A script's right-click and keys reach egui as the window's would: the button pressed and
    /// let go, the key pressed and let go with its modifiers.
    #[test]
    fn a_script_right_clicks_and_presses_keys() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("ta_launcher_film_keys_{}_{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let script = root.join("scene.toml");
        std::fs::write(&script, "fps = 10\nseconds = 1.0\npixels = [800,600]\npoints = [800.0,600.0]\npointer = [50.0,60.0]\n\
            [[step]]\nt = 0.1\ndo = \"click\"\nbutton = \"secondary\"\nhold = 0.05\n\
            [[step]]\nt = 0.25\ndo = \"key\"\nkey = \"F10\"\nshift = true\n").unwrap();
        let mut film = Film::open(&script).unwrap();
        let mut events = Vec::new();
        for frame in 0..4 {
            film.frame = Some(frame);
            let mut raw = egui::RawInput::default();
            film.input(&mut raw);
            events.extend(raw.events);
        }
        let buttons: Vec<(PointerButton, bool)> = events.iter().filter_map(|e| match e { Event::PointerButton { button, pressed, .. } => Some((*button, *pressed)), _ => None }).collect();
        assert_eq!(buttons, [(PointerButton::Secondary, true), (PointerButton::Secondary, false)]);
        let keys: Vec<(egui::Key, bool, bool)> = events.iter().filter_map(|e| match e { Event::Key { key, pressed, modifiers, .. } => Some((*key, *pressed, modifiers.shift)), _ => None }).collect();
        assert_eq!(keys, [(egui::Key::F10, true, true), (egui::Key::F10, false, true)]);
        film.finish(&egui::Context::default());
        drop(film);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// An interface sound is written down as the game writes one: on a bus the header gives a
    /// volume for (the game's sound log has effects, music and voice), as an interface sound.
    #[test]
    fn sounds_go_on_the_buses_the_log_has() {
        let header = sound_log_header(60);
        let line = sound_line(1.5, 90, 3, "sounds/button9.wav");
        let bus = line["bus"].as_str().unwrap();
        assert!(["effects", "music", "voice"].contains(&bus), "{bus}");
        assert!(header[bus].is_number(), "no volume for the {bus} bus in {header}");
        assert_eq!(line["kind"], "interface");
        assert_eq!((line["t"].as_f64(), line["frame"].as_u64(), line["id"].as_u64()), (Some(1.5), Some(90), Some(3)));
    }
}
