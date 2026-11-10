//! A replay made into a film: an MP4 to share (YouTube and the like). The replay bar's EXPORT
//! VIDEO opens its dialog ([`dialog`]): what of the battle (all of it, a stretch, or its big
//! moments one after another, as the replay's scan found them: `replay_scan.rs`), the picture's
//! size, rate and quality, the camera (the cast's director from above, a cinematic one low and
//! circling as the highlights' is, or the view as it was left), the cast's scoreboard and kill
//! feed over it, titles (the battle's card at the start, its big moments named as they come),
//! the battle's sounds and its music.
//!
//! The film is made on the game's own frames (`app.rs`, `film_*`). The replay goes to each
//! stretch's start (fast-forwarded, or started over), then every frame moves it on 1/fps of a
//! second, draws the picture at the film's size (`Renderer::render_image_with`) with the cast
//! and the titles over it (a pass of the interface at that size, [`Film::overlay`]), mixes that
//! much of the sound offline (`GameAudio::film`) and hands both to the writer (`ta_mp4`), which
//! encodes on a thread of its own. The film fades in from black and out to it, and dips to
//! black between stretches; the sound rises and falls with the picture. Meanwhile the window
//! shows how it goes ([`Film::page`]: the latest picture now and then, how long is left,
//! CANCEL), and at the end where the film was saved, to show in its folder or watch.
//!
//! A social media clip (`clips.rs`, the replay bar's EXPORT CLIP) is made the same way, a film
//! of one stretch ([`FilmSettings::clip`]) with a camera, titles and sound of its own
//! ([`ClipFilm`]): it opens on the action rather than on black, and ends on its end tag.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align, Align2, Color32, Layout, Rect, Shape, Stroke, StrokeKind, UiBuilder, pos2, vec2};
use glam::Vec3;
use ta_ui::i18n::{n, t, tf};
use ta_ui::widgets::ActionButton;

use crate::highlights::{Moment, MomentKind};
use crate::theme::*;

/// The film's sound: samples a second, and AAC's bits a second.
pub const RATE: u32 = 48_000;
const SOUND_BITS: u32 = 192_000;
/// The picture sizes offered (a name, width and height) and the rates.
pub const SIZES: [(&str, u32, u32); 4] = [("720p", 1280, 720), ("1080p", 1920, 1080), ("1440p", 2560, 1440), ("4K", 3840, 2160)];
/// A film of big moments: the worthiest so many, each from a little before it to a little after
/// (ticks), those that come this close joined.
const MOMENTS: usize = 10;
const LEAD: u64 = 3 * 30;
const TAIL: u64 = 4 * 30;
const JOIN: u64 = 2 * 30;
/// Seconds: the fade in from black at the start, the dip to black between stretches, the fade
/// out at the end; how long the battle's card and a moment's name stay up.
const FADE_IN: f32 = 1.0;
const DIP: f32 = 0.5;
const FADE_OUT: f32 = 1.5;
const CARD: f32 = 5.5;
const CAPTION: f32 = 4.5;
/// How often the window's picture of the film is renewed, and how wide it is drawn (pixels);
/// how often the window is drawn while pictures are made.
const PREVIEW_EVERY: Duration = Duration::from_millis(250);
const WINDOW_EVERY: Duration = Duration::from_millis(50);
const PREVIEW_WIDTH: usize = 640;

/// What of the battle the film shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Whole,
    Stretch,
    Moments,
}

/// The film's camera: the cast's director from above, a cinematic one (low, circling the fight
/// the director chose), or the view as it was left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shot {
    Director,
    Cinematic,
    AsSet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Standard,
    High,
    Best,
}

impl Quality {
    /// Bits a pixel a picture.
    fn bits(self) -> f64 {
        match self {
            Quality::Standard => 0.08,
            Quality::High => 0.12,
            Quality::Best => 0.2,
        }
    }
}

/// A film as chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct FilmSettings {
    pub span: Span,
    /// The stretch filmed (ticks), when that is what is filmed.
    pub from: u64,
    pub to: u64,
    pub size: (u32, u32),
    pub fps: u32,
    pub quality: Quality,
    pub shot: Shot,
    /// The cast's scoreboard and kill feed over the picture.
    pub scoreboard: bool,
    /// The battle's card at the start, its big moments named as they come.
    pub titles: bool,
    pub sounds: bool,
    pub music: bool,
    /// A social media clip of the stretch chosen, in place of what the rest chooses.
    pub clip: Option<ClipChoice>,
}

/// A social media clip as chosen (`clips.rs`): its length and shape, its stretch (ticks) and
/// where its climax is.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipChoice {
    pub spec: crate::clips::ClipSpec,
    pub from: u64,
    pub to: u64,
    pub climax: u64,
    pub focus: glam::Vec2,
}

impl Default for FilmSettings {
    fn default() -> FilmSettings {
        FilmSettings { span: Span::Whole, from: 0, to: 0, size: (1920, 1080), fps: 60, quality: Quality::High, shot: Shot::Director, scoreboard: true, titles: true, sounds: true, music: true, clip: None }
    }
}

impl Shot {
    /// How much of a fight's spread the camera frames (the cast's and the films' are closer in
    /// than the highlights', the cinematic camera's closer still).
    pub fn framing(self) -> f32 {
        match self {
            Shot::Cinematic => 0.7,
            _ => 0.8,
        }
    }
}

impl FilmSettings {
    /// The pictures' bits a second.
    pub fn bitrate(&self) -> u32 {
        if let Some(clip) = &self.clip {
            return clip.spec.pictures().bitrate;
        }
        let (w, h) = self.size;
        (w as f64 * h as f64 * self.fps as f64 * self.quality.bits()).round().clamp(2.0e6, 1.0e8) as u32
    }

    /// The pictures as the writer takes them.
    pub fn pictures(&self) -> ta_mp4::Pictures {
        if let Some(clip) = &self.clip {
            return clip.spec.pictures();
        }
        ta_mp4::Pictures { width: self.size.0, height: self.size.1, fps: self.fps, bitrate: self.bitrate() }
    }

    /// A social media clip's settings for `choice`, the rest kept from `self` (its titles,
    /// scoreboard and sound).
    pub fn for_clip(&self, choice: ClipChoice) -> FilmSettings {
        let pictures = choice.spec.pictures();
        FilmSettings { span: Span::Stretch, from: choice.from, to: choice.to, size: (pictures.width, pictures.height), fps: pictures.fps, clip: Some(choice), ..self.clone() }
    }

    pub fn sound(&self) -> Option<ta_mp4::Sound> {
        (self.sounds || self.music).then_some(ta_mp4::Sound { rate: RATE, bitrate: SOUND_BITS })
    }

    /// About how large a film this long is (bytes).
    pub fn bytes(&self, seconds: f32) -> f64 {
        (self.bitrate() as f64 + if self.sound().is_some() { SOUND_BITS as f64 } else { 0.0 }) / 8.0 * seconds as f64
    }
}

impl FilmSettings {
    /// Captures: `TA_FILM=whole|moments|<from>-<to>` (seconds) makes a replay into a film as it
    /// opens (once its scan is done), `TA_FILM_SIZE=<w>x<h>` (1280x720), `TA_FILM_FPS` (30),
    /// `TA_FILM_SHOT=director|cinematic|asset`, `TA_FILM_QUALITY=standard|high|best`, and
    /// `TA_FILM_CAST=0`, `TA_FILM_TITLES=0`, `TA_FILM_SOUND=0` leave those out.
    pub fn from_env() -> Option<FilmSettings> {
        let var = |name: &str| std::env::var(name).ok().map(|v| v.trim().to_ascii_lowercase());
        let off = |name: &str| var(name).is_some_and(|v| v == "0");
        let mut s = FilmSettings { size: (1280, 720), fps: 30, ..Default::default() };
        match var("TA_FILM")?.as_str() {
            "whole" | "1" => s.span = Span::Whole,
            "moments" => s.span = Span::Moments,
            range => {
                let (from, to) = range.split_once('-')?;
                (s.span, s.from, s.to) = (Span::Stretch, from.trim().parse::<u64>().ok()? * 30, to.trim().parse::<u64>().ok()? * 30);
            }
        }
        if let Some((w, h)) = var("TA_FILM_SIZE").and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))) {
            s.size = (w, h);
        }
        s.fps = var("TA_FILM_FPS").and_then(|v| v.parse().ok()).unwrap_or(s.fps);
        s.shot = match var("TA_FILM_SHOT").as_deref() {
            Some("cinematic") => Shot::Cinematic,
            Some("asset" | "as-set") => Shot::AsSet,
            _ => Shot::Director,
        };
        s.quality = match var("TA_FILM_QUALITY").as_deref() {
            Some("standard") => Quality::Standard,
            Some("best") => Quality::Best,
            _ => Quality::High,
        };
        (s.scoreboard, s.titles) = (!off("TA_FILM_CAST"), !off("TA_FILM_TITLES"));
        (s.sounds, s.music) = (!off("TA_FILM_SOUND"), !off("TA_FILM_SOUND"));
        Some(s)
    }
}

/// A stretch of the battle in the film (ticks), and for a big moment where it is and how far
/// it spreads (the camera starts there).
#[derive(Clone, Debug, PartialEq)]
pub struct Stretch {
    pub from: u64,
    pub to: u64,
    pub focus: Option<(Vec3, f32)>,
}

/// The stretches a film as `settings` chose shows of a replay `end` ticks long, given its big
/// moments `marks` (in order). A film of moments takes the worthiest few, each from a little
/// before it to a little after, joining those that meet.
pub fn stretches(settings: &FilmSettings, end: u64, marks: &[Moment]) -> Vec<Stretch> {
    if let Some(clip) = &settings.clip {
        let from = clip.from.min(end.saturating_sub(30));
        return vec![Stretch { from, to: clip.to.clamp(from + 30, end.max(from + 30)), focus: None }];
    }
    match settings.span {
        Span::Whole => vec![Stretch { from: 0, to: end, focus: None }],
        Span::Stretch => {
            let from = settings.from.min(end.saturating_sub(30));
            vec![Stretch { from, to: settings.to.clamp(from + 30, end.max(from + 30)), focus: None }]
        }
        Span::Moments => {
            let mut chosen: Vec<&Moment> = marks.iter().collect();
            chosen.sort_by(|a, b| b.worth.total_cmp(&a.worth).then(a.start.cmp(&b.start)));
            chosen.truncate(MOMENTS);
            chosen.sort_by_key(|m| m.start);
            let mut out: Vec<Stretch> = Vec::new();
            for m in chosen {
                let (from, to) = (m.start.saturating_sub(LEAD), (m.end + TAIL).min(end));
                match out.last_mut() {
                    Some(last) if from <= last.to + JOIN => last.to = last.to.max(to),
                    _ if to > from => out.push(Stretch { from, to, focus: Some((m.focus, m.spread)) }),
                    _ => {}
                }
            }
            out
        }
    }
}

/// Seconds of film the stretches make.
pub fn length(stretches: &[Stretch]) -> f32 {
    stretches.iter().map(|s| s.to.saturating_sub(s.from)).sum::<u64>() as f32 / 30.0
}

/// When a big moment is at its height: a battle's first losses, a strike's landing, a
/// Commander's fall (moments start a little before, see `highlights.rs`).
pub(crate) fn peak(m: &Moment) -> u64 {
    m.start
        + match m.kind {
            MomentKind::Nuke => 7 * 30,
            MomentKind::Commander => 8 * 30,
            MomentKind::Battle => 4 * 30,
        }
}

/// "12:40" of battle time (ticks); "1:02:05" past the hour.
fn clock(ticks: u64) -> String {
    let s = ticks / 30;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

/// "3:25" of film (seconds).
fn length_text(seconds: f32) -> String {
    let s = seconds.max(0.0).round() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// "420 MB", "1.6 GB".
pub(crate) fn size_text(bytes: f64) -> String {
    if bytes >= 1.0e9 { tf("{size} GB", &[("size", &format!("{:.1}", bytes / 1.0e9))]) } else { tf("{size} MB", &[("size", &((bytes / 1.0e6).round().max(1.0) as u64))]) }
}

/// Where a film of a battle on the map `stem` goes in `dir`: "Coast To Coast 001.mp4", the first
/// number free (as photos are named), nothing in it a file name may not hold.
pub fn film_path(dir: &Path, stem: &str) -> PathBuf {
    let stem: String = stem.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c }).collect();
    let stem = stem.split_whitespace().collect::<Vec<_>>().join(" ");
    let stem = if stem.is_empty() { "TA Reborn".to_string() } else { stem };
    (1..1000).map(|n| dir.join(format!("{stem} {n:03}.mp4"))).find(|p| !p.exists()).unwrap_or_else(|| dir.join(format!("{stem}.mp4")))
}

/// The battle's card: the map and who fought (name, side, colour).
pub struct Card {
    pub map: String,
    pub players: Vec<(String, String, Color32)>,
}

/// How the replay was before the film, to go back to (paused where the film ended).
pub struct Restore {
    pub camera: ta_render::Camera,
    pub speed: f32,
    pub cast: bool,
    pub action_camera: bool,
}

pub enum Phase {
    /// Going to the stretch's start.
    Seeking,
    Rolling,
    /// The writer completing the file.
    Finishing,
    Done(Outcome),
}

pub enum Outcome {
    Saved { seconds: f32, bytes: u64 },
    Failed(String),
}

/// What the window's page asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageAction {
    Cancel,
    ShowFolder,
    Watch,
    Back,
}

/// After a picture: roll on, go to the next stretch's start, or the film is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Roll,
    Seek(u64),
    Finished,
}

/// The UI pass drawn into the exported picture. Sharing the window's context would
/// clear its pointer state and hit rectangles between a Cancel press and release.
pub fn picture_context(window: &egui::Context) -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_compact(&ctx);
    // (At once, not at its first pass: `theme::set_zoom`.)
    crate::theme::set_zoom(&ctx, window.zoom_factor());
    ta_ui::motion::set_reduced(&ctx, ta_ui::motion::reduced(window));
    ta_ui::motion::set_reduced_flashes(&ctx, ta_ui::motion::reduced_flashes(window));
    ctx
}

/// The exported overlay's textures and HUD belong to its own egui renderer.
/// Only the underlying GPU portrait views may be shared with another interface.
pub struct PictureUi {
    pub ctx: egui::Context,
    pub renderer: egui_wgpu::Renderer,
    pub hud: crate::hud::Hud,
}

impl PictureUi {
    pub fn new(window: &egui::Context, device: &wgpu::Device, format: wgpu::TextureFormat, classic: bool) -> Option<Self> {
        let renderer = crate::loading::ui_renderer(device, format,
            egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, dithering: true, ..Default::default() })?;
        let mut hud = crate::hud::Hud::default();
        hud.classic = classic;
        Some(Self { ctx: picture_context(window), renderer, hud })
    }

    pub fn set_portrait(&mut self, device: &wgpu::Device, key: crate::hud::PortraitKey, view: Option<&wgpu::TextureView>) {
        let id = view.map(|view| self.renderer.register_native_texture(device, view, wgpu::FilterMode::Linear));
        self.hud.set_portrait(key, id);
    }
}

/// What a social media clip has of its own (`clips.rs`): the camera's plan, its titles, and its
/// sound's limiter.
pub struct ClipFilm {
    pub plan: crate::clips::CameraPlan,
    pub look: crate::clips::ClipLook,
    pub limiter: crate::clips::Limiter,
}

/// A clip's picture fades to black over its last moments only, and its sound comes in over its
/// first (no click) and goes out over its last (seconds).
const CLIP_PICTURE_OUT: f32 = 0.35;
const CLIP_SOUND_IN: f32 = 0.1;
const CLIP_SOUND_OUT: f32 = 1.6;

/// A film being made (see the module documentation).
pub struct Film {
    pub settings: FilmSettings,
    pub path: PathBuf,
    pub card: Card,
    pub stretches: Vec<Stretch>,
    pub stretch: usize,
    pub phase: Phase,
    pub restore: Restore,
    /// A social media clip's own camera, titles and sound.
    pub clip: Option<ClipFilm>,
    /// The speakers' sound, set aside while the film's is mixed (`app.rs`).
    pub speakers: Option<crate::audio::GameAudio>,
    /// The cinematic camera's heading, turning slowly.
    pub yaw: f32,
    writer: Option<ta_mp4::Mp4Writer>,
    finishing: Option<std::thread::JoinHandle<Result<(), ta_mp4::Mp4Error>>>,
    /// The big moments, by when they are at their height; the next to name; the one named and
    /// the film's second it came at.
    marks: Vec<Moment>,
    next_mark: usize,
    caption: Option<(Moment, f32)>,
    /// Pictures written, in all and in this stretch; pictures the film will have.
    frames: u64,
    stretch_frames: u64,
    total: u64,
    /// Sound frames written.
    samples: u64,
    /// Pictures made a second (smoothed), for the time left.
    rate: f32,
    last_picture: Option<Instant>,
    preview: Option<egui::TextureHandle>,
    preview_due: Instant,
    window_shown: Option<Instant>,
    /// Times the replay was sent again to this stretch's start, having arrived elsewhere.
    reseeks: u8,
}

/// The most times a film sends the replay again to a stretch's start (see [`Film::arrived_at`]).
const RESEEKS: u8 = 3;

impl Film {
    pub fn new(settings: FilmSettings, path: PathBuf, card: Card, stretches: Vec<Stretch>, mut marks: Vec<Moment>, restore: Restore, writer: ta_mp4::Mp4Writer) -> Film {
        let fps = settings.fps as u64;
        let total = stretches.iter().map(|s| s.to.saturating_sub(s.from) * fps / 30).sum();
        marks.sort_by_key(peak);
        Film {
            settings,
            path,
            card,
            stretches,
            stretch: 0,
            phase: Phase::Seeking,
            restore,
            clip: None,
            speakers: None,
            yaw: 0.6,
            writer: Some(writer),
            finishing: None,
            marks,
            next_mark: 0,
            caption: None,
            frames: 0,
            stretch_frames: 0,
            total,
            samples: 0,
            rate: 0.0,
            last_picture: None,
            preview: None,
            preview_due: Instant::now(),
            window_shown: None,
            reseeks: 0,
        }
    }

    /// Whether the replay, done seeking at `tick`, is where the stretch starts (a tick either
    /// way); else it is to be sent there again (a seek asked for by something else before the
    /// film began may have come between), a few times at most: after them the film rolls from
    /// where it is rather than wait for ever.
    pub fn arrived_at(&mut self, tick: u64) -> bool {
        if tick.abs_diff(self.current().from) <= 1 || self.reseeks >= RESEEKS {
            return true;
        }
        self.reseeks += 1;
        log::warn!("film: the replay stopped at {tick}, not at its stretch's start {}: going there again", self.current().from);
        false
    }

    /// Whether the window is drawn this frame: some twenty times a second while pictures are
    /// made (waiting on the screen would hold them back), every frame otherwise.
    pub fn window_due(&mut self) -> bool {
        let now = Instant::now();
        if !self.rolling() || self.window_shown.is_none_or(|t| now - t >= WINDOW_EVERY) {
            self.window_shown = Some(now);
            return true;
        }
        false
    }

    /// The game's step a picture (seconds).
    pub fn dt(&self) -> f32 {
        1.0 / self.settings.fps as f32
    }

    pub fn current(&self) -> &Stretch {
        &self.stretches[self.stretch.min(self.stretches.len() - 1)]
    }

    pub fn rolling(&self) -> bool {
        matches!(self.phase, Phase::Rolling)
    }

    /// Seconds of film so far.
    pub fn seconds(&self) -> f32 {
        self.frames as f32 / self.settings.fps as f32
    }

    /// The replay is at the stretch's start: the pictures begin (the moments before it are
    /// not named).
    pub fn roll(&mut self) {
        self.phase = Phase::Rolling;
        self.stretch_frames = 0;
        let from = self.current().from;
        while self.marks.get(self.next_mark).is_some_and(|m| peak(m) < from) {
            self.next_mark += 1;
        }
    }

    /// Stereo frames of sound for the next picture (they keep with the pictures exactly).
    pub fn sound_frames(&self) -> usize {
        let fps = self.settings.fps as u64;
        ((self.frames + 1) * RATE as u64 / fps).saturating_sub(self.samples) as usize
    }

    /// A social media clip's own camera, titles and sound (see [`ClipFilm`]).
    pub fn with_clip(mut self, clip: ClipFilm) -> Film {
        self.clip = Some(clip);
        self
    }

    /// How far faded in the picture and its sound are at `tick`: 0 black, 1 clear. (A clip opens
    /// on the action, which apps take for its cover, and darkens under its end tag.)
    pub fn fade(&self, tick: u64) -> f32 {
        let left = self.current().to.saturating_sub(tick) as f32 / 30.0;
        if self.clip.is_some() {
            return (left / CLIP_PICTURE_OUT).clamp(0.0, 1.0);
        }
        let into = self.stretch_frames as f32 / self.settings.fps as f32;
        let fade_in = if self.stretch == 0 { FADE_IN } else { DIP };
        let fade_out = if self.stretch + 1 == self.stretches.len() { FADE_OUT } else { DIP };
        (into / fade_in).min(left / fade_out).clamp(0.0, 1.0)
    }

    /// How loud the sound is at `tick`, 0 to 1: as the picture fades, or for a clip in at once and
    /// out over its end tag.
    pub fn sound_level(&self, tick: u64) -> f32 {
        if self.clip.is_none() {
            return self.fade(tick);
        }
        let into = self.stretch_frames as f32 / self.settings.fps as f32;
        let left = self.current().to.saturating_sub(tick) as f32 / 30.0;
        (into / CLIP_SOUND_IN).min(left / CLIP_SOUND_OUT).clamp(0.0, 1.0)
    }

    /// The big moments reached by `tick` named (with the titles on).
    pub fn note_moments(&mut self, tick: u64) {
        while let Some(m) = self.marks.get(self.next_mark).filter(|m| peak(m) <= tick) {
            if self.settings.titles {
                self.caption = Some((m.clone(), self.seconds()));
            }
            self.next_mark += 1;
        }
    }

    /// The picture (`width` × `height` RGBA, `fade` of the way in from black) and its sound
    /// written; now and then the window's picture of the film renewed from it (not from one
    /// fading, which would leave it dark at the end, nor from a clip's end tag: the page shows
    /// its action).
    pub fn write(&mut self, ctx: &egui::Context, rgba: Vec<u8>, width: u32, height: u32, fade: f32, sound: Vec<f32>) -> Result<(), String> {
        let now = Instant::now();
        let end_tag = self.clip.as_ref().is_some_and(|c| self.seconds() > c.look.seconds - crate::clips::END_TAG);
        if now >= self.preview_due && fade > 0.9 && !end_tag {
            self.preview_due = now + PREVIEW_EVERY;
            self.renew_preview(ctx, &rgba, width as usize, height as usize);
        }
        let writer = self.writer.as_mut().ok_or_else(|| t("The video's writer stopped").to_string())?;
        writer.picture(rgba).map_err(|e| e.to_string())?;
        self.samples += (sound.len() / 2) as u64;
        if self.settings.sound().is_some() {
            writer.sound(sound).map_err(|e| e.to_string())?;
        }
        self.frames += 1;
        self.stretch_frames += 1;
        if let Some(last) = self.last_picture {
            let r = 1.0 / (now - last).as_secs_f32().max(1e-4);
            self.rate = if self.rate == 0.0 { r } else { self.rate * 0.97 + r * 0.03 };
        }
        self.last_picture = Some(now);
        Ok(())
    }

    /// The window's picture: the film's, every `step`-th pixel.
    fn renew_preview(&mut self, ctx: &egui::Context, rgba: &[u8], width: usize, height: usize) {
        let step = (width / PREVIEW_WIDTH).max(1);
        let (w, h) = (width / step, height / step);
        let mut pixels = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let row = &rgba[y * step * width * 4..];
            for x in 0..w {
                pixels.extend_from_slice(&row[x * step * 4..x * step * 4 + 4]);
            }
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
        match &mut self.preview {
            Some(handle) => handle.set(image, egui::TextureOptions::LINEAR),
            None => self.preview = Some(ctx.load_texture("film_preview", image, egui::TextureOptions::LINEAR)),
        }
    }

    /// After the picture at `tick`: on, the next stretch, or the end (the file completed on a
    /// thread of its own).
    pub fn next(&mut self, tick: u64) -> Next {
        if tick < self.current().to {
            return Next::Roll;
        }
        if self.stretch + 1 < self.stretches.len() {
            self.stretch += 1;
            self.phase = Phase::Seeking;
            self.caption = None;
            self.reseeks = 0;
            return Next::Seek(self.current().from);
        }
        self.phase = Phase::Finishing;
        if let Some(writer) = self.writer.take() {
            match std::thread::Builder::new().name("film finish".into()).spawn(move || writer.finish()) {
                Ok(handle) => self.finishing = Some(handle),
                Err(e) => self.phase = Phase::Done(Outcome::Failed(e.to_string())),
            }
        }
        Next::Finished
    }

    /// The file completed (or not) by now?
    pub fn poll(&mut self) {
        if !matches!(self.phase, Phase::Finishing) || !self.finishing.as_ref().is_some_and(|h| h.is_finished()) {
            return;
        }
        let outcome = match self.finishing.take().map(|h| h.join()) {
            Some(Ok(Ok(()))) => {
                let bytes = std::fs::metadata(&self.path).map_or(0, |m| m.len());
                log::info!("film: {} ({} pictures, {:.1} s, {bytes} bytes)", self.path.display(), self.frames, self.seconds());
                Outcome::Saved { seconds: self.seconds(), bytes }
            }
            Some(Ok(Err(e))) => Outcome::Failed(e.to_string()),
            _ => Outcome::Failed(t("The video's writer stopped").to_string()),
        };
        self.phase = Phase::Done(outcome);
    }

    /// The film failed: what was written is let go (and the file with it).
    pub fn fail(&mut self, message: String) {
        log::warn!("film: {message}");
        self.writer = None;
        self.phase = Phase::Done(Outcome::Failed(message));
    }

    /// What goes over the film's picture (an interface pass at the film's size): the battle's
    /// card at the start, the big moments named as they come, and the fade; a clip's own titles
    /// (the battle at `tick`, its armies' worth `armies`).
    pub fn overlay(&self, ctx: &egui::Context, fade: f32, tick: u64, armies: &[f32]) {
        let screen = ctx.content_rect();
        let seconds = self.seconds();
        if let Some(clip) = &self.clip {
            clip.look.draw(ctx, seconds, tick, armies);
        } else if self.settings.titles {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("film_titles")));
            if self.stretch == 0 && seconds < CARD {
                card(&painter, screen, &self.card, seconds);
            }
            if let Some((m, at)) = &self.caption
                && seconds - at < CAPTION
            {
                caption(&painter, screen, m, seconds - at);
            }
        }
        if fade < 1.0 {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("film_fade")));
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(((1.0 - fade) * 255.0).round() as u8));
        }
    }

    /// The window while the film is made, drawn into the whole of `ui` (not in a window of its
    /// own: the film's own interface pass comes between the window's, and egui would take a
    /// window there as newly shown every time). `seeking`: how far the replay has got to the
    /// stretch's start.
    pub fn page(&mut self, ui: &mut egui::Ui, seeking: Option<f32>) -> Option<PageAction> {
        let screen = ui.ctx().content_rect();
        ui.painter().rect_filled(screen, 0.0, Color32::from_rgb(9, 10, 12));
        let width = (screen.width() - 48.0).clamp(280.0, 640.0);
        // (A tall picture, a vertical clip's, as high as leaves room for the rest.)
        let aspect = self.settings.size.1 as f32 / self.settings.size.0.max(1) as f32;
        let picture_height = (width * aspect).min((screen.height() - 300.0).max(120.0));
        let picture_width = picture_height / aspect;
        let height = picture_height + 250.0;
        let column = Rect::from_center_size(screen.center(), vec2(width, height.min(screen.height() - 16.0)));
        let mut ui = ui.new_child(UiBuilder::new().max_rect(column).layout(Layout::top_down(Align::Center)));
        let done = matches!(self.phase, Phase::Done(_));
        let clip = self.clip.is_some();
        let (heading, colour) = match &self.phase {
            Phase::Done(Outcome::Saved { .. }) => (if clip { t("CLIP SAVED") } else { t("VIDEO SAVED") }, GREEN),
            Phase::Done(Outcome::Failed(_)) => (if clip { t("THE CLIP COULD NOT BE MADE") } else { t("THE VIDEO COULD NOT BE MADE") }, RED),
            _ => (if clip { t("EXPORTING CLIP") } else { t("EXPORTING VIDEO") }, AMBER),
        };
        ui.label(egui::RichText::new(heading).font(display(26.0)).color(colour));
        ui.add_space(10.0);
        // The film's latest picture.
        let (rect, _) = ui.allocate_exact_size(vec2(picture_width, picture_height), egui::Sense::hover());
        let p = ui.painter();
        p.add(ta_ui::metal::well(rect.expand(4.0), INSET));
        match &self.preview {
            Some(picture) => {
                p.image(picture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.text(rect.center(), Align2::CENTER_CENTER, t("Getting ready…"), body(14.0), STEEL);
            }
        }
        ui.add_space(14.0);
        // How far it has got.
        let share = match &self.phase {
            Phase::Done(Outcome::Saved { .. }) | Phase::Finishing => 1.0,
            _ => self.frames as f32 / self.total.max(1) as f32,
        };
        let (bar, _) = ui.allocate_exact_size(vec2(width, 16.0), egui::Sense::hover());
        let p = ui.painter();
        p.add(ta_ui::metal::well(bar, INSET));
        let fill = bar.shrink(2.0);
        if share > 0.0 {
            p.add(ta_ui::metal::vgrad(Rect::from_min_size(fill.min, vec2(fill.width() * share.clamp(0.0, 1.0), fill.height())), AMBER_BRIGHT, AMBER.gamma_multiply(0.7)));
        }
        p.text(bar.center(), Align2::CENTER_CENTER, format!("{}%", (share * 100.0).floor() as u32), strong(11.5), TEXT_BRIGHT);
        ui.add_space(8.0);
        // What it is doing, and how long is left.
        let line = |ui: &mut egui::Ui, text: String, colour: Color32| {
            ui.label(egui::RichText::new(text).font(body(14.0)).color(colour));
        };
        match &self.phase {
            Phase::Seeking => {
                let to = clock(self.current().from);
                let text = match seeking {
                    Some(f) => tf("Going to {time} in the replay… {percent}%", &[("time", &to), ("percent", &((f * 100.0) as u32))]),
                    None => tf("Going to {time} in the replay…", &[("time", &to)]),
                };
                line(&mut ui, text, TEXT);
            }
            Phase::Rolling => {
                line(&mut ui, tf("Picture {frame} of {total}", &[("frame", &ta_ui::i18n::grouped(self.frames)), ("total", &ta_ui::i18n::grouped(self.total))]), TEXT);
                if self.rate > 0.0 {
                    let left = self.total.saturating_sub(self.frames) as f32 / self.rate;
                    line(&mut ui, tf("About {time} left · {rate} pictures a second", &[("time", &length_text(left)), ("rate", &(self.rate.round() as u32))]), STEEL);
                }
            }
            Phase::Finishing => line(&mut ui, t("Finishing the file…").to_string(), TEXT),
            Phase::Done(Outcome::Saved { seconds, bytes }) => {
                let name = self.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().to_string());
                line(&mut ui, name, TEXT_BRIGHT);
                line(&mut ui, tf("{length} of video · {size}", &[("length", &length_text(*seconds)), ("size", &size_text(*bytes as f64))]), STEEL);
            }
            Phase::Done(Outcome::Failed(message)) => {
                ui.add(egui::Label::new(egui::RichText::new(message).font(body(14.0)).color(TEXT)).wrap());
            }
        }
        // What the film is.
        let (w, h) = self.settings.size;
        match &self.settings.clip {
            Some(choice) => {
                let shape = crate::clip_dialog::shape_name(choice.spec.shape);
                line(&mut ui, tf("{width} × {height} · {seconds} seconds · {shape}", &[("width", &w), ("height", &h), ("seconds", &choice.spec.seconds), ("shape", &t(shape))]), STEEL_DIM);
            }
            None => {
                let shot = match self.settings.shot {
                    Shot::Director => t("Director's camera"),
                    Shot::Cinematic => t("Cinematic camera"),
                    Shot::AsSet => t("The camera as set"),
                };
                line(&mut ui, tf("{width} × {height} · {fps} pictures a second · {camera}", &[("width", &w), ("height", &h), ("fps", &self.settings.fps), ("camera", &shot)]), STEEL_DIM);
            }
        }
        ui.add_space(14.0);
        let mut action = None;
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        ui.horizontal(|ui| {
            match &self.phase {
                Phase::Done(Outcome::Saved { .. }) => {
                    let spare = (width - 3.0 * 196.0 - 2.0 * 8.0).max(0.0) / 2.0;
                    ui.add_space(spare);
                    if ui.add(ActionButton::secondary(n("SHOW IN FOLDER")).size(196.0, 34.0)).clicked() {
                        action = Some(PageAction::ShowFolder);
                    }
                    if ui.add(ActionButton::secondary(n("WATCH IT")).size(196.0, 34.0)).clicked() {
                        action = Some(PageAction::Watch);
                    }
                    if ui.add(ActionButton::primary(n("BACK TO THE REPLAY")).size(196.0, 34.0)).clicked() || escape {
                        action = Some(PageAction::Back);
                    }
                }
                Phase::Done(Outcome::Failed(_)) => {
                    ui.add_space((width - 220.0).max(0.0) / 2.0);
                    if ui.add(ActionButton::primary(n("BACK TO THE REPLAY")).size(220.0, 34.0)).clicked() || escape {
                        action = Some(PageAction::Back);
                    }
                }
                Phase::Finishing => {}
                _ => {
                    ui.add_space((width - 180.0).max(0.0) / 2.0);
                    if ui.add(ActionButton::danger(n("CANCEL")).size(180.0, 34.0)).clicked() || escape {
                        action = Some(PageAction::Cancel);
                    }
                }
            }
        });
        if !done {
            ui.ctx().request_repaint();
        }
        action
    }
}

/// The battle's card over the film's first seconds: the game's name, the map, who fought.
fn card(painter: &egui::Painter, screen: Rect, card: &Card, seconds: f32) {
    let shown = ((seconds - 0.5) / 0.7).min((CARD - seconds) / 0.8).clamp(0.0, 1.0);
    if shown <= 0.0 {
        return;
    }
    let mid = screen.center().y;
    let band = Rect::from_x_y_ranges(screen.x_range(), (mid - 84.0)..=(mid + 70.0));
    let dark = Color32::from_black_alpha((205.0 * shown) as u8);
    painter.add(ta_ui::metal::vgrad(Rect::from_x_y_ranges(band.x_range(), (band.top() - 40.0)..=band.top()), Color32::TRANSPARENT, dark));
    painter.rect_filled(band, 0.0, dark);
    painter.add(ta_ui::metal::vgrad(Rect::from_x_y_ranges(band.x_range(), band.bottom()..=(band.bottom() + 40.0)), dark, Color32::TRANSPARENT));
    // (Brass rules drawn out from the middle as it comes in.)
    let reach = screen.width() * 0.32 * shown;
    for y in [band.top() + 10.0, band.bottom() - 10.0] {
        painter.hline((screen.center().x - reach)..=(screen.center().x + reach), y, Stroke::new(1.0, AMBER.gamma_multiply(0.7 * shown)));
    }
    let rise = (1.0 - shown) * 10.0;
    painter.text(pos2(screen.center().x, mid - 52.0 + rise), Align2::CENTER_CENTER, t("TOTAL ANNIHILATION REBORN"), strong(13.0), AMBER.gamma_multiply(shown));
    painter.text(pos2(screen.center().x, mid - 12.0 + rise), Align2::CENTER_CENTER, &card.map, display(44.0), TEXT_BRIGHT.gamma_multiply(shown));
    // Who fought, in their colours: "A vs B", or a list.
    let between = if card.players.len() == 2 { t("VS") } else { "·" };
    let pieces: Vec<(String, egui::FontId, Color32)> = card
        .players
        .iter()
        .enumerate()
        .flat_map(|(i, (name, side, colour))| {
            let mut v = Vec::new();
            if i > 0 {
                v.push((format!("  {between}  "), strong(14.0), STEEL));
            }
            v.push((name.clone(), strong(18.0), colour.lerp_to_gamma(Color32::WHITE, 0.35)));
            v.push((format!(" {side}"), strong(12.0), STEEL));
            v
        })
        .collect();
    let galleys: Vec<_> = pieces.into_iter().map(|(text, font, colour)| painter.layout_no_wrap(text, font, colour.gamma_multiply(shown))).collect();
    let total: f32 = galleys.iter().map(|g| g.size().x).sum();
    let mut x = screen.center().x - total / 2.0;
    let y = mid + 36.0 + rise;
    for g in galleys {
        let size = g.size();
        painter.galley(pos2(x, y - size.y / 2.0), g, Color32::WHITE);
        x += size.x;
    }
}

/// A big moment named in the film's lower third, sliding in from the left.
fn caption(painter: &egui::Painter, screen: Rect, m: &Moment, age: f32) {
    let shown = (age / 0.35).min((CAPTION - age) / 0.5).clamp(0.0, 1.0);
    if shown <= 0.0 {
        return;
    }
    let (title, detail) = crate::highlights::describe(m);
    let colour = match m.kind {
        MomentKind::Nuke => Color32::from_rgb(255, 96, 70),
        MomentKind::Commander => Color32::from_rgb(240, 236, 220),
        MomentKind::Battle => AMBER_BRIGHT,
    };
    let title = painter.layout_no_wrap(t(title).to_string(), display(28.0), TEXT_BRIGHT.gamma_multiply(shown));
    let detail = painter.layout_no_wrap(format!("{} · {}", clock(peak(m)), detail), body(15.0), TEXT.gamma_multiply(shown));
    let width = title.size().x.max(detail.size().x) + 44.0;
    let slide = (1.0 - shown) * -40.0;
    let plate = Rect::from_min_size(pos2(screen.left() + 40.0 + slide, screen.bottom() - screen.height() * 0.2 - 70.0), vec2(width, 70.0));
    painter.add(Shape::rect_filled(plate, 3.0, Color32::from_black_alpha((180.0 * shown) as u8)));
    painter.rect_filled(Rect::from_min_size(plate.min, vec2(5.0, plate.height())), 0.0, colour.gamma_multiply(shown));
    painter.rect_stroke(plate, 3.0, Stroke::new(1.0, LINE.gamma_multiply(shown)), StrokeKind::Inside);
    painter.galley(plate.min + vec2(22.0, 8.0), title, Color32::WHITE);
    painter.galley(plate.min + vec2(22.0, 44.0), detail, Color32::WHITE);
}

/// Shows the film in its folder, chosen.
pub fn show_in_folder(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if let Err(e) = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn() {
            log::warn!("could not show {}: {e}", path.display());
        }
    }
    #[cfg(not(windows))]
    if let Some(folder) = path.parent() {
        crate::photo::open_folder(folder);
    }
}

/// Plays the film in the system's own player.
pub fn watch(path: &Path) {
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    if let Err(e) = std::process::Command::new(program).arg(path).spawn() {
        log::warn!("could not play {}: {e}", path.display());
    }
}

/// The export dialog's column of labels (points).
const LABELS: f32 = 118.0;

/// The export dialog's choices (kept while the game runs).
pub struct FilmDialog {
    pub settings: FilmSettings,
}

/// What the dialog was told.
pub enum Answer {
    Stay,
    Close,
    Export(FilmSettings),
}

/// The export dialog over the replay `end` ticks long, now at `now`, with its scan's big
/// moments (`scan`, while it looks for them: how far it has got).
pub fn dialog(ctx: &egui::Context, d: &mut FilmDialog, end: u64, now: u64, scan: Option<&crate::replay_scan::ScanState>) -> Answer {
    use ta_ui::widgets::{check_box_named, segmented};
    let s = &mut d.settings;
    if s.to == 0 || s.to > end || s.from >= s.to {
        (s.from, s.to) = (now.saturating_sub(30 * 30).min(end.saturating_sub(30)), (now + 60 * 30).min(end));
    }
    let marks = scan.map_or(&[][..], |s| &s.marks[..]);
    let scanning = scan.is_some_and(|s| !s.finished);
    let planned = stretches(s, end, marks);
    let seconds = length(&planned);
    let ready = !planned.is_empty() && seconds >= 1.0 && !(s.span == Span::Moments && scanning);
    // (Notes under the choices start where the choices do.)
    let indent = |ui: &mut egui::Ui| ui.add_space(LABELS + ui.spacing().item_spacing.x);
    let row = |ui: &mut egui::Ui, name: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            let (label, _) = ui.allocate_exact_size(vec2(LABELS, 28.0), egui::Sense::hover());
            ui.painter().text(label.left_center(), Align2::LEFT_CENTER, t(name), strong(11.5), STEEL);
            add(ui);
        });
        ui.add_space(3.0);
    };
    let about = if ready {
        tf("About {length} of video, about {size}, saved in your Videos folder", &[("length", &length_text(seconds)), ("size", &size_text(s.bytes(seconds)))])
    } else {
        String::new()
    };
    let panel = ta_ui::panel::Panel::new(t("Export video")).width(560.0).exit(ta_ui::panel::Exit::Close).max_height((ctx.content_rect().height() - 24.0).max(240.0));
    let mut export = false;
    let mut cancel = false;
    let response = ta_ui::panel::modal_with_footer(
        ctx,
        "film_dialog",
        panel,
        true,
        74.0,
        |ui| {
            row(ui, n("WHAT"), &mut |ui| {
                segmented(ui, &mut s.span, &[(Span::Whole, n("The whole battle")), (Span::Stretch, n("A stretch of it")), (Span::Moments, n("Its big moments"))]);
            });
            match s.span {
                Span::Whole => {}
                Span::Stretch => {
                    stretch_track(ui, &mut s.from, &mut s.to, end, now, marks);
                    ui.horizontal(|ui| {
                        indent(ui);
                        ui.label(egui::RichText::new(tf("From {from} to {to}", &[("from", &clock(s.from)), ("to", &clock(s.to))])).font(body(13.5)).color(TEXT));
                        if ui.add(ActionButton::secondary(n("FROM HERE")).height(26.0)).clicked() {
                            s.from = now.min(s.to.saturating_sub(30));
                        }
                        if ui.add(ActionButton::secondary(n("TO HERE")).height(26.0)).clicked() {
                            s.to = now.max(s.from + 30).min(end);
                        }
                    });
                    ui.add_space(3.0);
                }
                Span::Moments => {
                    ui.horizontal(|ui| {
                        indent(ui);
                        let text = if scanning {
                            tf("Finding the replay's big moments… {percent}%", &[("percent", &scan.map_or(0, |s| (s.done * 100 / end.max(1)) as u32))])
                        } else if planned.is_empty() {
                            t("This replay has no big moments to show: nobody fought.").to_string()
                        } else {
                            tf("{count} of its biggest moments, one after another, each named", &[("count", &planned.len())])
                        };
                        ui.label(egui::RichText::new(text).font(body(13.5)).color(TEXT));
                    });
                    ui.add_space(3.0);
                }
            }
            let mut size = SIZES.iter().position(|z| (z.1, z.2) == s.size).unwrap_or(1);
            row(ui, n("PICTURE"), &mut |ui| {
                let options: Vec<(usize, &str)> = SIZES.iter().enumerate().map(|(i, z)| (i, z.0)).collect();
                ta_ui::widgets::segmented_raw(ui, &mut size, &options);
                ui.add_space(8.0);
                segmented(ui, &mut s.fps, &[(30, n("30 a second")), (60, n("60 a second"))]);
            });
            s.size = (SIZES[size].1, SIZES[size].2);
            row(ui, n("QUALITY"), &mut |ui| {
                segmented(ui, &mut s.quality, &[(Quality::Standard, n("Standard")), (Quality::High, n("High")), (Quality::Best, n("Best"))]);
            });
            row(ui, n("CAMERA"), &mut |ui| {
                segmented(ui, &mut s.shot, &[(Shot::Director, n("Director")), (Shot::Cinematic, n("Cinematic")), (Shot::AsSet, n("As you set it"))]);
            });
            ui.horizontal(|ui| {
                indent(ui);
                let about = match s.shot {
                    Shot::Director => t("From above, cutting and gliding to the biggest fight"),
                    Shot::Cinematic => t("Low and circling slowly round the biggest fight"),
                    Shot::AsSet => t("Your view of the battle, as you left it"),
                };
                ui.label(egui::RichText::new(about).font(body(12.5)).color(STEEL));
            });
            ui.add_space(4.0);
            row(ui, n("ON THE PICTURE"), &mut |ui| {
                check_box_named(ui, &mut s.scoreboard, n("Scoreboard and kill feed"));
                ui.label(egui::RichText::new(t("Scoreboard and kill feed")).font(body(13.5)).color(TEXT));
                ui.add_space(10.0);
                check_box_named(ui, &mut s.titles, n("Titles"));
                ui.label(egui::RichText::new(t("Titles")).font(body(13.5)).color(TEXT));
            });
            row(ui, n("SOUND"), &mut |ui| {
                check_box_named(ui, &mut s.sounds, n("Battle sounds"));
                ui.label(egui::RichText::new(t("Battle sounds")).font(body(13.5)).color(TEXT));
                ui.add_space(10.0);
                check_box_named(ui, &mut s.music, n("Music"));
                ui.label(egui::RichText::new(t("Music")).font(body(13.5)).color(TEXT));
            });
        },
        |ui| {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(about).font(body(12.5)).color(STEEL));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add(ActionButton::secondary(n("CANCEL"))).clicked() {
                    cancel = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(ready, ActionButton::primary(n("EXPORT VIDEO"))).clicked() {
                        export = true;
                    }
                });
            });
        },
    );
    if export {
        Answer::Export(d.settings.clone())
    } else if cancel || response.exit {
        Answer::Close
    } else {
        Answer::Stay
    }
}

/// The stretch filmed, over the replay `end` ticks long: its two ends dragged (or the nearer one
/// moved to a click), where the replay is now marked, and its big moments.
fn stretch_track(ui: &mut egui::Ui, from: &mut u64, to: &mut u64, end: u64, now: u64, marks: &[Moment]) {
    ui.horizontal(|ui| {
        ui.add_space(LABELS + ui.spacing().item_spacing.x);
        let width = ui.available_width().max(160.0);
        let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), egui::Sense::click_and_drag());
        let track = Rect::from_x_y_ranges(rect.x_range().shrink(6.0), (rect.center().y - 5.0)..=(rect.center().y + 5.0));
        let x_of = |tick: u64| track.left() + track.width() * (tick as f32 / end.max(1) as f32).clamp(0.0, 1.0);
        let tick_at = |x: f32| (((x - track.left()) / track.width()).clamp(0.0, 1.0) * end as f32) as u64;
        if let Some(pointer) = response.interact_pointer_pos()
            && (response.dragged() || response.clicked())
        {
            let tick = tick_at(pointer.x);
            // The nearer end moves (kept a second apart).
            let which = ui.memory(|m| m.data.get_temp::<bool>(response.id)).filter(|_| response.dragged() && !response.drag_started());
            let moving_from = which.unwrap_or((pointer.x - x_of(*from)).abs() < (pointer.x - x_of(*to)).abs());
            ui.memory_mut(|m| m.data.insert_temp(response.id, moving_from));
            if moving_from {
                *from = tick.min(to.saturating_sub(30));
            } else {
                *to = tick.max(*from + 30).min(end);
            }
        }
        let p = ui.painter();
        p.add(ta_ui::metal::well(track, INSET));
        let chosen = Rect::from_x_y_ranges(x_of(*from)..=x_of(*to), track.y_range().shrink(2.0));
        p.add(ta_ui::metal::vgrad(chosen, AMBER_BRIGHT, AMBER.gamma_multiply(0.6)));
        for m in marks {
            let x = x_of(m.start);
            let colour = match m.kind {
                MomentKind::Nuke => Color32::from_rgb(255, 96, 70),
                MomentKind::Commander => Color32::from_rgb(240, 236, 220),
                MomentKind::Battle => AMBER_BRIGHT,
            };
            p.add(Shape::convex_polygon(vec![pos2(x - 3.0, track.top() - 7.0), pos2(x + 3.0, track.top() - 7.0), pos2(x, track.top() - 2.0)], colour, Stroke::NONE));
        }
        p.vline(x_of(now), track.y_range().expand(5.0), Stroke::new(1.5, Color32::from_white_alpha(170)));
        for x in [x_of(*from), x_of(*to)] {
            let handle = Rect::from_center_size(pos2(x, track.center().y), vec2(7.0, 22.0));
            p.rect_filled(handle, 2.0, TEXT_BRIGHT);
            p.rect_stroke(handle, 2.0, Stroke::new(1.0, Color32::from_black_alpha(180)), StrokeKind::Outside);
        }
        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    });
    ui.add_space(1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_film(phase: Phase) -> Film {
        Film {
            settings: FilmSettings::default(), path: PathBuf::from("test.mp4"),
            card: Card { map: "Test".into(), players: Vec::new() },
            stretches: vec![Stretch { from: 0, to: 300, focus: None }], stretch: 0, phase,
            restore: Restore { camera: ta_render::Camera::new(glam::Vec2::new(1024.0, 768.0)), speed: 1.0, cast: false, action_camera: false }, clip: None,
            speakers: None, yaw: 0.0, writer: None, finishing: None, marks: Vec::new(),
            next_mark: 0, caption: None, frames: 0, stretch_frames: 0, total: 300, samples: 0,
            rate: 0.0, last_picture: None, preview: None, preview_due: Instant::now(), window_shown: None, reseeks: 0,
        }
    }

    fn page_frame(ctx: &egui::Context, film: &mut Film, events: Vec<egui::Event>) -> (egui::FullOutput, Option<PageAction>) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1024.0, 768.0))),
            focused: true, events, ..Default::default()
        };
        let mut action = None;
        let output = ctx.run_ui(raw, |ui| { action = film.page(ui, None); });
        (output, action)
    }

    fn text_center(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => Some(Rect::from_min_size(text.pos, text.galley.size()).center()),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_center(shape, label)),
            _ => None,
        }
    }

    #[test]
    fn cancel_click_survives_export_picture_passes_between_window_frames() {
        let window = egui::Context::default();
        crate::theme::install_compact(&window);
        let picture = picture_context(&window);
        let mut film = page_film(Phase::Rolling);
        let (output, _) = page_frame(&window, &mut film, Vec::new());
        let at = output.shapes.iter().find_map(|s| text_center(&s.shape, "CANCEL")).expect("the cancel button is visible");
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            // Export frames carry no real pointer input and use the video's own dimensions.
            for _ in 0..3 {
                let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(640.0, 360.0))), focused: true, ..Default::default() };
                picture.run_ui(raw, |ui| film.overlay(ui.ctx(), 0.5, 0, &[])).drop_without_applying_deltas();
            }
            let events = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }];
            let (output, action) = page_frame(&window, &mut film, events);
            output.drop_without_applying_deltas();
            assert_eq!(action, (!pressed).then_some(PageAction::Cancel), "a cancel click must reach the export controller");
        }
    }

    #[test]
    fn escape_cancels_both_seeking_and_rolling_exports() {
        for phase in [Phase::Seeking, Phase::Rolling] {
            let ctx = egui::Context::default();
            crate::theme::install_compact(&ctx);
            let mut film = page_film(phase);
            let (output, _) = page_frame(&ctx, &mut film, Vec::new());
            output.drop_without_applying_deltas();
            let (output, action) = page_frame(&ctx, &mut film, vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
            output.drop_without_applying_deltas();
            assert_eq!(action, Some(PageAction::Cancel));
        }
    }

    fn moment(start: u64, end: u64, worth: f32) -> Moment {
        Moment { start, end, focus: Vec3::new(start as f32, 0.0, 0.0), spread: 300.0, kind: MomentKind::Battle, units: 3, worth, slow: None, biggest: false }
    }

    #[test]
    fn a_film_of_moments_takes_the_worthiest_in_order_and_joins_those_that_meet() {
        let settings = FilmSettings { span: Span::Moments, ..Default::default() };
        // Fourteen moments, the later the worthier...
        let mut marks: Vec<Moment> = (0..14).map(|i| moment(1000 + i * 2000, 1300 + i * 2000, 100.0 + i as f32)).collect();
        // ...and the worthiest of all just after the last: the two meet, and are joined.
        marks.push(moment(27_400, 27_600, 900.0));
        let films = stretches(&settings, 40_000, &marks);
        assert!(films.windows(2).all(|w| w[0].to < w[1].from), "in order, apart: {films:?}");
        assert_eq!(films.last().map(|s| (s.from, s.to)), Some((27_000 - LEAD, 27_600 + TAIL)));
        // The ten worthiest (the five least worthy left out), the joined pair one stretch.
        assert_eq!(films.first().map(|s| s.from), Some(1000 + 5 * 2000 - LEAD));
        assert_eq!(films.len(), MOMENTS - 1);
        assert!(films.iter().all(|s| s.focus.is_some()));
    }

    #[test]
    fn a_clip_is_its_stretch_at_its_shape() {
        use crate::clips::{ClipSpec, Shape};
        let choice = ClipChoice { spec: ClipSpec { seconds: 30, shape: Shape::Vertical }, from: 3000, to: 3900, climax: 3600, focus: glam::Vec2::new(500.0, 700.0) };
        let settings = FilmSettings { music: false, ..Default::default() }.for_clip(choice);
        let marks = vec![moment(100, 300, 900.0)];
        assert_eq!(stretches(&settings, 40_000, &marks).iter().map(|s| (s.from, s.to)).collect::<Vec<_>>(), [(3000, 3900)]);
        // Ending with the replay.
        assert_eq!(stretches(&settings, 3600, &marks)[0].to, 3600);
        let pictures = settings.pictures();
        assert_eq!((pictures.width, pictures.height, pictures.fps), (1080, 1920, 30));
        assert!(!settings.music && settings.sounds, "the rest kept");
        assert!((7_000_000..8_000_000).contains(&settings.bitrate()));
    }

    #[test]
    fn a_film_rolls_only_from_its_stretch_start() {
        // (Its stretch: ticks 0 to 300.)
        let mut film = page_film(Phase::Seeking);
        assert!(film.arrived_at(0) && film.arrived_at(1), "at the start, or a tick off");
        // Elsewhere (another seek came between): sent there again, a few times at most.
        for _ in 0..RESEEKS {
            assert!(!film.arrived_at(4500));
        }
        assert!(film.arrived_at(4500), "after a few tries it rolls rather than wait for ever");
    }

    #[test]
    fn a_clip_opens_on_the_action_and_fades_at_its_end() {
        let mut film = page_film(Phase::Rolling);
        film.settings.fps = 30;
        film.clip = Some(ClipFilm {
            plan: crate::clips::plan(&Default::default(), 0, 300, 150, glam::Vec2::ZERO),
            look: crate::clips::ClipLook { shape: crate::clips::Shape::Vertical, seconds: 10.0, start: 0, map: "Test".into(), players: Vec::new(), callouts: Vec::new(), titles: true, scoreboard: true },
            limiter: crate::clips::Limiter::new(RATE),
        });
        // The first picture is clear (apps take it for the cover); the sound comes in at once.
        assert_eq!(film.fade(0), 1.0);
        assert_eq!(film.sound_level(0), 0.0);
        film.stretch_frames = 3;
        assert_eq!(film.sound_level(3), 1.0);
        // Both go out at the end.
        assert!(film.fade(299) < 0.2 && film.fade(280) == 1.0);
        assert!(film.sound_level(285) < 1.0 && film.sound_level(300) == 0.0);
    }

    #[test]
    fn a_stretch_keeps_within_the_replay_and_a_second_long() {
        let settings = FilmSettings { span: Span::Stretch, from: 900, to: 400, ..Default::default() };
        let s = stretches(&settings, 3000, &[]);
        assert_eq!((s[0].from, s[0].to), (900, 930));
        let settings = FilmSettings { span: Span::Stretch, from: 5000, to: 9000, ..Default::default() };
        let s = stretches(&settings, 3000, &[]);
        assert!(s[0].from < s[0].to && s[0].to <= 3000);
    }

    #[test]
    fn the_sound_keeps_with_the_pictures() {
        // 30 or 60 a second at 48,000: every picture has its exact share, and over a second they
        // add up to the second.
        for fps in [30, 60] {
            let per = RATE as u64 / fps as u64;
            assert_eq!(per * fps as u64, RATE as u64);
        }
    }

    #[test]
    fn the_bitrate_follows_the_picture_and_stays_in_bounds() {
        let film = |size, fps, quality| FilmSettings { size, fps, quality, ..Default::default() }.bitrate();
        assert!(film((1920, 1080), 60, Quality::High) > film((1920, 1080), 30, Quality::High));
        assert!(film((3840, 2160), 60, Quality::Best) <= 100_000_000);
        assert!(film((1280, 720), 30, Quality::Standard) >= 2_000_000);
        let high = film((1920, 1080), 60, Quality::High) as f64 / 1.0e6;
        assert!((10.0..20.0).contains(&high), "YouTube's 1080p60 is about 12 Mbit/s: {high}");
    }

    #[test]
    fn films_are_numbered_and_named_as_files_may_be() {
        let dir = std::env::temp_dir().join(format!("ta-film-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = |p: PathBuf| p.file_name().unwrap().to_string_lossy().to_string();
        let first = film_path(&dir, "Coast To Coast: AI <battle>?");
        assert_eq!(name(first.clone()), "Coast To Coast AI battle 001.mp4");
        std::fs::write(&first, b"").unwrap();
        assert_eq!(name(film_path(&dir, "Coast To Coast: AI <battle>?")), "Coast To Coast AI battle 002.mp4");
        assert_eq!(name(film_path(&dir, " / ")), "TA Reborn 001.mp4");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
