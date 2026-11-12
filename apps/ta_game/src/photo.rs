//! Photo mode (F11, or PHOTO MODE in the pause menu): the battle holds still, slows or plays on
//! (single player; in multiplayer it goes on), the interface steps aside and the camera roams
//! free, closer, farther and lower than in play. A panel of three tabs sets the picture up:
//!
//! - CAMERA: the time of day, the weather (the world's own, none, a shower), the lens (its
//!   field of view, the eye staying where it is), a tilt of the horizon, and a depth of field (a
//!   lens focused at a distance, or tilt-shift's miniature look); clicking the picture focuses
//!   there.
//! - LOOK: a look (natural, cinematic, a war film's bleach bypass, noir, a vintage print,
//!   arctic, golden hour, a thermal camera, a night sight), each a starting point for the
//!   exposure, contrast, colour, warmth, vignette, film grain and a lens's colour fringe.
//! - FRAME: the picture's shape (the window's, 16:9, 2.39:1, 4:3, square, 4:5, 9:16: the photo
//!   is cut to it, the rest of the screen shaded), composition guides, the size, and a caption
//!   stamped in its corner.
//!
//! TAKE PHOTO (or F12) saves the picture without the panel, at up to twice the window's
//! resolution, to Pictures\TA Reborn (see [`ta_config::pictures_dir`]); the last ones taken show
//! under the button, a click away from their folder.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use egui::{Align2, Color32, Id, LayerId, Order, Rect, RichText, Sense, Slider, Stroke, pos2, vec2};
use glam::{Vec2, Vec3};
use ta_render::post::{DepthOfField, DofMode, Look, PostSettings, Sight};
use ta_render::{Camera, CameraMode, Screenshot};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use crate::game::Game;
use crate::theme::*;

/// The largest photo, in pixels (4K): past it the scene's antialiased targets need more
/// memory than many graphics cards have.
const MAX_PIXELS: f64 = 3840.0 * 2160.0;
/// The largest blur (strength 1), as a fraction of the picture's height.
const MAX_BLUR: f32 = 0.024;
/// How long a saved photo's notice stays (seconds).
const NOTICE_TIME: f64 = 8.0;
/// The panel's width (its content's).
const WIDTH: f32 = 320.0;
/// The last photos kept under the button, and their thumbnails' width (pixels).
const SHOTS: usize = 4;
const THUMB: u32 = 192;
/// The panel's keys, under its button.
const KEYS: &str = n("F12 takes a photo · H hides this panel · Esc returns");
/// The battle in slow motion: its speed.
const SLOW: f32 = 0.15;
/// The lens: its field of view, vertical degrees (a long lens to a wide one).
const FOV_RANGE: std::ops::RangeInclusive<f32> = 8.0..=90.0;

/// How the picture is focused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// All sharp.
    Off,
    /// A lens: sharp at one distance, blurring nearer and farther.
    Lens,
    /// A sharp band across the picture: the world as a miniature.
    TiltShift,
}

/// The panel's tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Camera,
    Look,
    Frame,
}

/// The weather in the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sky {
    /// The world's own, as the battle has it.
    Own,
    /// None at all: no rain, snow, embers or dust.
    Clear,
    /// The middle of a shower, on any world.
    Rain,
}

/// How the battle goes on (single player).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Frozen,
    Slow,
    Live,
}

/// The picture's shape: the window's, or a frame of its own the photo is cut to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Window,
    Wide,
    Cinema,
    Classic,
    Square,
    Portrait,
    Tall,
}

impl Frame {
    pub const ALL: [Frame; 7] = [Frame::Window, Frame::Wide, Frame::Cinema, Frame::Classic, Frame::Square, Frame::Portrait, Frame::Tall];

    /// Width over height (none: the window's).
    pub fn aspect(self) -> Option<f32> {
        match self {
            Frame::Window => None,
            Frame::Wide => Some(16.0 / 9.0),
            Frame::Cinema => Some(2.39),
            Frame::Classic => Some(4.0 / 3.0),
            Frame::Square => Some(1.0),
            Frame::Portrait => Some(4.0 / 5.0),
            Frame::Tall => Some(9.0 / 16.0),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Frame::Window => n("WINDOW"),
            Frame::Wide => "16:9",
            Frame::Cinema => "2.39:1",
            Frame::Classic => "4:3",
            Frame::Square => "1:1",
            Frame::Portrait => "4:5",
            Frame::Tall => "9:16",
        }
    }
}

/// Lines over the picture to compose it by (on the screen only, never in the photo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guides {
    Off,
    Thirds,
    Golden,
    Centre,
}

/// The look presets: each a starting point for the look's sliders (and its tints and sight,
/// which have none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Natural,
    Cinematic,
    Bleach,
    Noir,
    Vintage,
    Arctic,
    Golden,
    Thermal,
    Night,
}

/// What a look sets: the picture's grade, its grain and its fringe, its tints, its sight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grade {
    pub exposure: f32,
    pub saturation: f32,
    pub vignette: f32,
    pub contrast: f32,
    pub warmth: f32,
    pub grain: f32,
    pub fringe: f32,
    pub mono: f32,
    pub lift: f32,
    pub shadows: [f32; 4],
    pub highlights: [f32; 4],
    pub special: Sight,
}

impl Preset {
    pub const ALL: [Preset; 9] = [Preset::Natural, Preset::Cinematic, Preset::Bleach, Preset::Noir, Preset::Vintage, Preset::Arctic, Preset::Golden, Preset::Thermal, Preset::Night];

    fn label(self) -> &'static str {
        match self {
            Preset::Natural => n("NATURAL"),
            Preset::Cinematic => n("CINEMATIC"),
            Preset::Bleach => n("WAR FILM"),
            Preset::Noir => n("NOIR"),
            Preset::Vintage => n("VINTAGE"),
            Preset::Arctic => n("ARCTIC"),
            Preset::Golden => n("GOLDEN HOUR"),
            Preset::Thermal => n("THERMAL"),
            Preset::Night => n("NIGHT SIGHT"),
        }
    }

    /// Its grade, over the battle's own picture (`base`: its exposure, colour and vignette).
    pub fn grade(self, base: &PostSettings) -> Grade {
        let none = [0.5, 0.5, 0.5, 0.0];
        let g = Grade {
            exposure: base.exposure,
            saturation: base.saturation,
            vignette: base.vignette,
            contrast: 1.0,
            warmth: 0.0,
            grain: 0.0,
            fringe: 0.0,
            mono: 0.0,
            lift: 0.0,
            shadows: none,
            highlights: none,
            special: Sight::Eye,
        };
        match self {
            Preset::Natural => g,
            // Teal shadows, warm highlights, a little more of everything.
            Preset::Cinematic => Grade { saturation: base.saturation * 1.08, vignette: 0.3, contrast: 1.12, warmth: 0.12, grain: 0.12, fringe: 0.15, shadows: [0.1, 0.45, 0.55, 0.32], highlights: [0.95, 0.62, 0.32, 0.28], ..g },
            // A war film's bleach bypass: half the colour, hard contrast, grit.
            Preset::Bleach => Grade { saturation: base.saturation * 0.42, vignette: 0.38, contrast: 1.38, warmth: -0.05, grain: 0.35, lift: 0.015, ..g },
            Preset::Noir => Grade { vignette: 0.52, contrast: 1.42, grain: 0.42, mono: 1.0, ..g },
            // A faded sepia print.
            Preset::Vintage => Grade { vignette: 0.55, contrast: 0.9, warmth: 0.5, grain: 0.55, fringe: 0.28, mono: 0.88, lift: 0.07, shadows: [0.42, 0.28, 0.16, 0.5], highlights: [0.92, 0.78, 0.52, 0.55], ..g },
            Preset::Arctic => Grade { saturation: base.saturation * 0.78, vignette: 0.28, contrast: 1.06, warmth: -0.6, shadows: [0.18, 0.34, 0.62, 0.3], highlights: [0.8, 0.93, 1.0, 0.16], ..g },
            Preset::Golden => Grade { exposure: base.exposure * 1.05, saturation: base.saturation * 1.15, vignette: 0.3, contrast: 1.05, warmth: 0.55, highlights: [1.0, 0.78, 0.38, 0.36], shadows: [0.55, 0.32, 0.22, 0.18], ..g },
            Preset::Thermal => Grade { vignette: 0.2, grain: 0.18, special: Sight::Thermal, ..g },
            Preset::Night => Grade { exposure: base.exposure * 1.1, vignette: 0.62, grain: 0.5, special: Sight::Night, ..g },
        }
    }
}

/// A photo taken: its thumbnail (once saved, as a texture) and where it went, or why not.
struct Shot {
    saving: Option<mpsc::Receiver<Saved>>,
    thumb: Option<egui::TextureHandle>,
    path: Option<PathBuf>,
}

/// What the saving thread sends back.
struct Saved {
    result: Result<PathBuf, String>,
    thumb: Option<egui::ColorImage>,
}

/// A photo's caption: two lines in its lower left corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Caption {
    pub title: String,
    pub line: String,
}

pub struct PhotoMode {
    /// Restored on leaving.
    was_paused: bool,
    speed_before: f32,
    post_before: PostSettings,
    camera_before: Camera,
    weather_before: Option<crate::effects::WeatherState>,
    pub tab: Tab,
    /// The time of day the picture shows (hours), or none for the battle's own.
    pub hours: Option<f32>,
    pub sky: Sky,
    pub pace: Pace,
    /// Asked for since the last frame (the app applies them: see [`PhotoMode::sync`]).
    sky_changed: bool,
    pace_changed: bool,
    /// The lens: vertical field of view (degrees), and the horizon's tilt (degrees, positive
    /// banks right).
    pub fov: f32,
    pub roll: f32,
    pub focus: Focus,
    /// Lens: the distance in focus, as a multiple of the distance to the middle of the screen.
    pub lens: f32,
    /// Tilt-shift: the sharp band's centre (0 top, 1 bottom) and half height (fractions of the
    /// picture's height).
    pub band_at: f32,
    pub band: f32,
    /// How strongly what is out of focus blurs (0..1).
    pub blur: f32,
    pub preset: Preset,
    pub grade: Grade,
    pub frame: Frame,
    pub guides: Guides,
    /// Twice the window's resolution (up to 4K).
    pub large: bool,
    pub caption: bool,
    /// The panel is hidden (H).
    pub hide_panel: bool,
    /// Asked for this frame.
    pub take: bool,
    pub leave: bool,
    shots: Vec<Shot>,
    /// The last one saved or failed (message, folder, since when).
    notice: Option<(String, Option<PathBuf>, f64)>,
    /// The last click that set the focus (where on the screen, in points; when).
    marker: Option<(egui::Pos2, f64)>,
    /// Until when the tilt-shift band's edges show (after a change).
    guides_until: f64,
    /// When the last photo was taken (the shutter's flash).
    flash: Option<f64>,
}

impl PhotoMode {
    /// Enters photo mode: a single-player battle (or a replay) pauses.
    pub fn enter(game: &mut Game, post: PostSettings, camera: &Camera) -> Self {
        let was_paused = game.paused;
        let speed_before = game.speed;
        if game.net.is_none() {
            game.set_pace(Some(true), None);
        }
        PhotoMode {
            was_paused,
            speed_before,
            post_before: post,
            camera_before: camera.clone(),
            weather_before: None,
            tab: Tab::Camera,
            hours: None,
            sky: Sky::Own,
            pace: Pace::Frozen,
            sky_changed: false,
            pace_changed: false,
            fov: camera.fov_y.to_degrees().clamp(*FOV_RANGE.start(), *FOV_RANGE.end()),
            roll: camera.roll.to_degrees(),
            focus: Focus::Off,
            lens: 1.0,
            band_at: 0.55,
            band: 0.1,
            blur: 0.5,
            preset: Preset::Natural,
            grade: Preset::Natural.grade(&post),
            frame: Frame::Window,
            guides: Guides::Off,
            large: true,
            caption: false,
            hide_panel: false,
            take: false,
            leave: false,
            shots: Vec::new(),
            notice: None,
            marker: None,
            guides_until: 0.0,
            flash: None,
        }
    }

    /// Leaves photo mode: the battle, its weather, the picture's settings and the camera as
    /// they were.
    pub fn leave(self, game: &mut Game, post: &mut PostSettings, camera: &mut Camera) {
        if game.net.is_none() {
            game.set_pace(Some(self.was_paused), Some(self.speed_before));
        }
        if let Some(state) = self.weather_before {
            game.effects.set_weather_state(state);
        }
        *post = self.post_before;
        let viewport = camera.viewport;
        *camera = self.camera_before;
        camera.viewport = viewport;
    }

    /// What was asked of the battle since the last frame: its pace (single player) and its
    /// weather (`now`: game seconds).
    pub fn sync(&mut self, game: &mut Game, now: f32) {
        if std::mem::take(&mut self.pace_changed) && game.net.is_none() {
            match self.pace {
                Pace::Frozen => game.set_pace(Some(true), None),
                Pace::Slow => game.set_pace(Some(false), Some(SLOW)),
                Pace::Live => game.set_pace(Some(false), Some(self.speed_before)),
            }
        }
        if std::mem::take(&mut self.sky_changed) {
            let before = *self.weather_before.get_or_insert_with(|| game.effects.weather_state());
            match self.sky {
                Sky::Own => game.effects.set_weather_state(before),
                Sky::Clear => game.effects.photo_weather(false, now),
                Sky::Rain => {
                    game.effects.photo_weather(true, now);
                    if game.paused {
                        game.effects.photo_rain(&game.sim, now);
                    }
                }
            }
        }
    }

    /// The lens on the camera (the classic camera has none).
    pub fn apply_lens(&self, camera: &mut Camera) {
        if camera.mode == CameraMode::Perspective {
            set_lens(camera, self.fov, self.roll);
        }
    }

    /// A choice made for a capture (`--open photo-<word>-...`): a look (`cinematic`, `noir`...),
    /// a tab (`look`, `frame`), a frame's shape (`square`, `cinema`, `portrait`, `tall`), the
    /// guides (`thirds`), the caption (`caption`), the weather (`rain`, `clear`), the pace
    /// (`slow`, `live`), a long or wide lens (`long`, `wide`) or a tilt (`tilted`). False for
    /// none of those.
    pub fn choose(&mut self, word: &str) -> bool {
        let preset = match word {
            "natural" => Some(Preset::Natural),
            "cinematic" => Some(Preset::Cinematic),
            "warfilm" => Some(Preset::Bleach),
            "noir" => Some(Preset::Noir),
            "vintage" => Some(Preset::Vintage),
            "arctic" => Some(Preset::Arctic),
            "golden" => Some(Preset::Golden),
            "thermal" => Some(Preset::Thermal),
            "night" => Some(Preset::Night),
            _ => None,
        };
        if let Some(preset) = preset {
            self.preset = preset;
            self.grade = preset.grade(&self.post_before);
            return true;
        }
        match word {
            "camera" => self.tab = Tab::Camera,
            "look" => self.tab = Tab::Look,
            "frame" => self.tab = Tab::Frame,
            "square" => self.frame = Frame::Square,
            "cinema" => self.frame = Frame::Cinema,
            "portrait" => self.frame = Frame::Portrait,
            "tall" => self.frame = Frame::Tall,
            "thirds" => self.guides = Guides::Thirds,
            "caption" => self.caption = true,
            "rain" | "clear" => {
                self.sky = if word == "rain" { Sky::Rain } else { Sky::Clear };
                self.sky_changed = true;
            }
            "slow" | "live" => {
                self.pace = if word == "slow" { Pace::Slow } else { Pace::Live };
                self.pace_changed = true;
            }
            "long" => self.fov = 12.0,
            "wide" => self.fov = 70.0,
            "tilted" => self.roll = 8.0,
            _ => return false,
        }
        true
    }

    /// The focus in effect: the classic camera has no depth to focus a lens by, so its lens is
    /// a tilt-shift.
    fn focus_for(&self, camera: &Camera) -> Focus {
        match self.focus {
            Focus::Lens if camera.mode == CameraMode::Classic => Focus::TiltShift,
            f => f,
        }
    }

    /// This frame's post-processing.
    pub fn post(&self, camera: &Camera) -> PostSettings {
        let mut post = self.post_before;
        let g = &self.grade;
        post.exposure = g.exposure;
        post.saturation = g.saturation;
        post.vignette = g.vignette;
        let plain = g.contrast == 1.0 && g.warmth == 0.0 && g.grain == 0.0 && g.fringe == 0.0 && g.mono == 0.0 && g.lift == 0.0 && g.shadows[3] == 0.0 && g.highlights[3] == 0.0 && g.special == Sight::Eye;
        post.look = (!plain).then_some(Look {
            contrast: g.contrast,
            warmth: g.warmth,
            grain: g.grain,
            fringe: g.fringe,
            mono: g.mono,
            lift: g.lift,
            shadows: g.shadows,
            highlights: g.highlights,
            special: g.special,
        });
        let blur = self.blur * MAX_BLUR;
        post.dof = match self.focus_for(camera) {
            Focus::Off => None,
            Focus::Lens => Some(DepthOfField { mode: DofMode::Lens, focus: camera.focus_distance() * self.lens, blur, band: 0.0 }),
            Focus::TiltShift => Some(DepthOfField { mode: DofMode::TiltShift, focus: self.band_at, blur, band: self.band }),
        };
        post
    }

    /// A click on the picture at `screen` (physical pixels; `ground` is the ground there):
    /// the lens focuses there, or the tilt-shift's sharp band moves there. `ppp`: physical
    /// pixels per point; `now`: the interface's clock.
    pub fn click(&mut self, screen: Vec2, camera: &Camera, ground: Option<Vec3>, ppp: f32, now: f64) {
        match self.focus_for(camera) {
            Focus::Off => return,
            Focus::Lens => {
                let Some(ground) = ground else { return };
                let distance = camera.view_distance(ground);
                self.lens = (distance / camera.focus_distance().max(1.0)).clamp(0.25, 4.0);
            }
            Focus::TiltShift => {
                self.band_at = (screen.y / camera.viewport.y.max(1.0)).clamp(0.0, 1.0);
                self.guides_until = now + 1.5;
            }
        }
        self.marker = Some((pos2(screen.x / ppp, screen.y / ppp), now));
    }

    /// The photo's size for a window `window` pixels large.
    pub fn size(&self, window: (u32, u32)) -> (u32, u32) {
        photo_size(window, self.large)
    }

    /// Saves `shot` as the next `<stem> NNN.png` in `dir`, cut to the frame and with the caption
    /// (`caption`: none unless it is on): in the background, or before returning with `wait`
    /// (a capture about to end).
    pub fn save(&mut self, shot: Screenshot, dir: PathBuf, stem: String, caption: Option<Caption>, now: f64, wait: bool) {
        let (send, receive) = mpsc::channel();
        let aspect = self.frame.aspect();
        let caption = caption.filter(|_| self.caption);
        let write = move || {
            let mut shot = match aspect {
                Some(a) => crop_to_aspect(shot, a),
                None => shot,
            };
            if let Some(c) = &caption {
                stamp(&mut shot, c);
            }
            let thumb = thumbnail(&shot);
            let result = save_picture(&shot, &dir, &stem).map_err(|e| e.to_string());
            send.send(Saved { result, thumb }).ok();
        };
        if wait {
            write();
        } else {
            let saving = std::thread::spawn(write);
            if let Ok(mut all) = SAVING.lock() {
                all.retain(|t| !t.is_finished());
                all.push(saving);
            }
        }
        self.shots.push(Shot { saving: Some(receive), thumb: None, path: None });
        self.flash = Some(now);
    }

    /// A photo could not be drawn.
    pub fn failed(&mut self, now: f64) {
        self.notice = Some((t("The photo could not be drawn at this size: try the window's size.").to_string(), None, now));
    }

    /// Collects the photos saved since the last frame (their thumbnails onto the screen).
    fn poll(&mut self, ctx: &egui::Context, now: f64) {
        for (i, shot) in self.shots.iter_mut().enumerate() {
            let Some(saving) = &shot.saving else { continue };
            match saving.try_recv() {
                Ok(Saved { result, thumb }) => {
                    shot.saving = None;
                    self.notice = Some(match result {
                        Ok(path) => {
                            log::info!("saved photo {}", path.display());
                            let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                            shot.path = Some(path.clone());
                            (tf("Saved {name}", &[("name", &name)]), path.parent().map(Path::to_path_buf), now)
                        }
                        Err(e) => {
                            log::error!("could not save a photo: {e}");
                            (tf("Could not save the photo: {error}", &[("error", &e)]), None, now)
                        }
                    });
                    if let Some(image) = thumb.filter(|_| shot.path.is_some()) {
                        shot.thumb = Some(ctx.load_texture(format!("photo_thumb_{i}_{now}"), image, egui::TextureOptions::LINEAR));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => shot.saving = None,
            }
        }
        // (Those that failed go; the last few kept.)
        self.shots.retain(|s| s.saving.is_some() || s.path.is_some());
        let extra = self.shots.len().saturating_sub(SHOTS);
        self.shots.drain(..extra);
    }

    /// Draws the panel and the marks over the picture. `window`: the window's size in pixels;
    /// `battle_hours`: the battle's own time of day (none: TA's light); `multiplayer`: the
    /// battle goes on meanwhile.
    pub fn draw(&mut self, ctx: &egui::Context, camera: &Camera, window: (u32, u32), battle_hours: Option<f32>, multiplayer: bool) {
        let now = ctx.input(|i| i.time);
        self.poll(ctx, now);
        if self.notice.as_ref().is_some_and(|n| now - n.2 > NOTICE_TIME) {
            self.notice = None;
        }
        self.marks(ctx, camera, now);
        if self.hide_panel {
            panel::strip(ctx, "photo_hint", Align2::CENTER_BOTTOM, vec2(0.0, -18.0), |ui| {
                ui.label(RichText::new(t("PHOTO MODE · H shows the panel · F12 takes a photo · Esc returns")).font(strong(12.5)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(0.8)));
            });
            if let Some((text, _, _)) = &self.notice {
                let text = text.clone();
                panel::strip(ctx, "photo_notice", Align2::CENTER_TOP, vec2(0.0, 18.0), |ui| {
                    ui.label(RichText::new(text).font(strong(13.0)).color(TEXT_BRIGHT));
                });
            }
            return;
        }
        let subtitle = match (multiplayer, self.pace) {
            (true, _) => n("THE BATTLE GOES ON"),
            (false, Pace::Frozen) => n("THE BATTLE HOLDS STILL"),
            (false, Pace::Slow) => n("THE BATTLE IN SLOW MOTION"),
            (false, Pace::Live) => n("THE BATTLE GOES ON"),
        };
        egui::Area::new(Id::new("photo_panel")).anchor(Align2::RIGHT_TOP, vec2(-14.0, 14.0)).order(Order::Foreground).show(ctx, |ui| {
            // (Its body scrolls on a short screen; the button and the shots stay in sight.)
            let room = ctx.content_rect().height() - 28.0;
            let page = Panel::new("PHOTO MODE").subtitle(subtitle).width(WIDTH).exit(Exit::Close).max_height(room);
            // (The footer has the shots and the notice to itself meanwhile.)
            let height = self.footer_height(ctx);
            let (shots, notice) = (std::mem::take(&mut self.shots), self.notice.take());
            let response = page.show_with_footer(ui, height, |ui| self.controls(ui, camera, window, battle_hours, multiplayer, now), |ui| footer(ui, &shots, notice.as_ref()));
            (self.shots, self.notice) = (shots, notice);
            self.take |= response.inner.1;
            if response.exit {
                self.leave = true;
            }
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui, camera: &Camera, window: (u32, u32), battle_hours: Option<f32>, multiplayer: bool, now: f64) {
        ui.spacing_mut().slider_width = WIDTH - 104.0;
        w::segmented_fill(ui, &mut self.tab, &[(Tab::Camera, "CAMERA"), (Tab::Look, "STYLE"), (Tab::Frame, "FRAMING")]);
        ui.add_space(space::M);
        match self.tab {
            Tab::Camera => self.camera_tab(ui, camera, battle_hours, multiplayer, now),
            Tab::Look => self.look_tab(ui),
            Tab::Frame => self.frame_tab(ui, window),
        }
    }

    fn camera_tab(&mut self, ui: &mut egui::Ui, camera: &Camera, battle_hours: Option<f32>, multiplayer: bool, now: f64) {
        // How the battle goes on (single player).
        if !multiplayer {
            panel::setting(ui, "THE BATTLE", None, |ui| {
                if w::segmented_fill(ui, &mut self.pace, &[(Pace::Frozen, "FROZEN"), (Pace::Slow, "SLOW"), (Pace::Live, "LIVE")]).changed() {
                    self.pace_changed = true;
                }
            });
        }
        panel::setting(ui, "TIME OF DAY", None, |ui| {
            let mut hours = self.hours.or(battle_hours).unwrap_or(12.0);
            if ui.add(Slider::new(&mut hours, 0.0..=24.0).step_by(0.25).custom_formatter(|v, _| clock(v as f32))).changed() {
                self.hours = Some(hours % 24.0);
            }
            ui.add_enabled_ui(self.hours.is_some(), |ui| {
                if ui.add(ActionButton::secondary("THE BATTLE'S OWN").size(150.0, 26.0)).clicked() {
                    self.hours = None;
                }
            });
        });
        panel::setting(ui, "WEATHER", None, |ui| {
            if w::segmented_fill(ui, &mut self.sky, &[(Sky::Own, "THE WORLD'S"), (Sky::Clear, "CLEAR SKY"), (Sky::Rain, "RAIN")]).changed() {
                self.sky_changed = true;
            }
        });
        let perspective = camera.mode == CameraMode::Perspective;
        if perspective {
            panel::rule(ui);
            ui.add_space(space::S);
            panel::setting(ui, "FOCAL LENGTH", Some("Long lenses close in; wide ones take in more."), |ui| {
                ui.add(Slider::new(&mut self.fov, FOV_RANGE).logarithmic(true).custom_formatter(|v, _| format!("{:.0} mm", lens_mm(v as f32))));
            });
            panel::setting(ui, "TILT", None, |ui| {
                ui.add(Slider::new(&mut self.roll, -35.0..=35.0).step_by(0.5).custom_formatter(|v, _| format!("{v:.1}°")));
                ui.add_enabled_ui(self.roll != 0.0, |ui| {
                    if ui.add(ActionButton::secondary("STRAIGHTEN").size(150.0, 26.0)).clicked() {
                        self.roll = 0.0;
                    }
                });
            });
        }
        panel::rule(ui);
        ui.add_space(space::S);
        // Focus.
        let classic = camera.mode == CameraMode::Classic;
        panel::setting(ui, "FOCUS", None, |ui| {
            let mut focus = self.focus_for(camera);
            let options: &[(Focus, &str)] =
                if classic { &[(Focus::Off, "OFF"), (Focus::TiltShift, "TILT-SHIFT")] } else { &[(Focus::Off, "OFF"), (Focus::Lens, "LENS"), (Focus::TiltShift, "TILT-SHIFT")] };
            if w::segmented_fill(ui, &mut focus, options).changed() {
                self.focus = focus;
                if focus == Focus::TiltShift {
                    self.guides_until = now + 1.5;
                }
            }
        });
        match self.focus_for(camera) {
            Focus::Off => {}
            Focus::Lens => {
                panel::setting(ui, "DISTANCE", Some("Click the picture to focus there."), |ui| {
                    ui.add(Slider::new(&mut self.lens, 0.25..=4.0).logarithmic(true).custom_formatter(|v, _| format!("{v:.2}×")));
                });
            }
            Focus::TiltShift => {
                panel::setting(ui, "SHARP BAND", Some("Click the picture to move the band there."), |ui| {
                    let r = ui.add(Slider::new(&mut self.band, 0.02..=0.3).custom_formatter(|v, _| format!("{:.0}%", v * 200.0)));
                    if r.changed() || r.hovered() {
                        self.guides_until = now + 1.0;
                    }
                });
            }
        }
        if self.focus_for(camera) != Focus::Off {
            panel::setting(ui, "BLUR", None, |ui| {
                ui.add(Slider::new(&mut self.blur, 0.05..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            });
        }
    }

    fn look_tab(&mut self, ui: &mut egui::Ui) {
        // The looks, three by three, each its own picture of the colours it makes.
        let gap = 6.0;
        let tile = vec2((WIDTH - 2.0 * gap) / 3.0, 50.0);
        for row in Preset::ALL.chunks(3) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for &preset in row {
                    if look_tile(ui, preset, preset == self.preset, &self.post_before, tile).clicked() {
                        self.preset = preset;
                        self.grade = preset.grade(&self.post_before);
                    }
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
        ui.add_space(space::S);
        let special = self.grade.special != Sight::Eye;
        let g = &mut self.grade;
        panel::setting(ui, "EXPOSURE", None, |ui| {
            ui.add(Slider::new(&mut g.exposure, 0.4..=2.5).logarithmic(true).custom_formatter(|v, _| format!("{v:.2}")));
        });
        if !special {
            panel::setting(ui, "CONTRAST", None, |ui| {
                ui.add(Slider::new(&mut g.contrast, 0.6..=1.8).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            });
            panel::setting(ui, "COLOUR", None, |ui| {
                ui.add(Slider::new(&mut g.saturation, 0.0..=1.8).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            });
            panel::setting(ui, "WARMTH", None, |ui| {
                ui.add(Slider::new(&mut g.warmth, -1.0..=1.0).custom_formatter(|v, _| format!("{:+.0}", v * 100.0)));
            });
        }
        panel::setting(ui, "VIGNETTE", None, |ui| {
            ui.add(Slider::new(&mut g.vignette, 0.0..=0.9).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        panel::setting(ui, "FILM GRAIN", None, |ui| {
            ui.add(Slider::new(&mut g.grain, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        panel::setting(ui, "COLOUR FRINGE", Some("A lens's colours parting towards its corners."), |ui| {
            ui.add(Slider::new(&mut g.fringe, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        if self.grade != self.preset.grade(&self.post_before) && ui.add(ActionButton::secondary("RESET THE LOOK").size(150.0, 26.0)).clicked() {
            self.grade = self.preset.grade(&self.post_before);
        }
    }

    fn frame_tab(&mut self, ui: &mut egui::Ui, window: (u32, u32)) {
        panel::setting(ui, "SHAPE", Some("The photo is cut to it; the rest of the screen is shaded."), |ui| {
            for row in Frame::ALL.chunks(4) {
                let options: Vec<(Frame, &str)> = row.iter().map(|&f| (f, f.label())).collect();
                w::segmented_fill(ui, &mut self.frame, &options);
            }
        });
        panel::setting(ui, "GUIDES", Some("Shown on the screen only, never in the photo."), |ui| {
            w::segmented_fill(ui, &mut self.guides, &[(Guides::Off, "OFF"), (Guides::Thirds, "THIRDS"), (Guides::Golden, "GOLDEN"), (Guides::Centre, "CENTRE")]);
        });
        // Its size.
        let single = framed(photo_size(window, false), self.frame.aspect());
        let double = framed(photo_size(window, true), self.frame.aspect());
        panel::setting(ui, "SIZE", None, |ui| {
            ui.horizontal(|ui| {
                if double != single {
                    w::segmented(ui, &mut self.large, &[(false, "WINDOW"), (true, "LARGE")]);
                }
                let (pw, ph) = if self.large { double } else { single };
                ui.label(RichText::new(format!("{pw} × {ph}")).font(mono(12.5)).color(STEEL));
            });
        });
        panel::setting(ui, "CAPTION", Some("TA Reborn, the map and the date, in the photo's corner."), |ui| {
            w::segmented_fill(ui, &mut self.caption, &[(false, "OFF"), (true, "ON")]);
        });
    }

    /// The footer's height: the button, its keys (on as many lines as their translation takes),
    /// the shots and the notice.
    fn footer_height(&self, ctx: &egui::Context) -> f32 {
        let keys = ctx.fonts_mut(|f| f.layout(t(KEYS).to_string(), body(12.0), STEEL_DIM, WIDTH).size().y);
        let shots = if self.shots.is_empty() { 0.0 } else { 70.0 };
        let notice = if self.notice.is_some() { 64.0 } else { 0.0 };
        44.0 + 10.0 + keys + shots + notice
    }

    /// The frame's shade and its guides, the focus marker, the tilt-shift band's edges and the
    /// shutter's flash.
    fn marks(&mut self, ctx: &egui::Context, camera: &Camera, now: f64) {
        let painter = ctx.layer_painter(LayerId::new(Order::Background, Id::new("photo_marks")));
        let screen = ctx.content_rect();
        // The frame: what the photo keeps, the rest shaded.
        let frame = frame_rect(screen, self.frame.aspect());
        if frame != screen {
            let shade = Color32::from_black_alpha(178);
            for r in [
                Rect::from_min_max(screen.min, pos2(screen.max.x, frame.min.y)),
                Rect::from_min_max(pos2(screen.min.x, frame.max.y), screen.max),
                Rect::from_min_max(pos2(screen.min.x, frame.min.y), pos2(frame.min.x, frame.max.y)),
                Rect::from_min_max(pos2(frame.max.x, frame.min.y), pos2(screen.max.x, frame.max.y)),
            ] {
                if r.width() > 0.5 && r.height() > 0.5 {
                    painter.rect_filled(r, 0.0, shade);
                }
            }
            painter.rect_stroke(frame, 0.0, Stroke::new(1.0, AMBER.gamma_multiply(0.6)), egui::StrokeKind::Outside);
        }
        let guide = Stroke::new(1.0, Color32::from_white_alpha(70));
        let line = |a: egui::Pos2, b: egui::Pos2| painter.line_segment([a, b], guide);
        let at = |fx: f32, fy: f32| pos2(frame.min.x + fx * frame.width(), frame.min.y + fy * frame.height());
        match self.guides {
            Guides::Off => {}
            Guides::Thirds | Guides::Golden => {
                let k = if self.guides == Guides::Thirds { 1.0 / 3.0 } else { 0.382 };
                for f in [k, 1.0 - k] {
                    line(at(f, 0.0), at(f, 1.0));
                    line(at(0.0, f), at(1.0, f));
                }
            }
            Guides::Centre => {
                let c = frame.center();
                let r = frame.height() * 0.03;
                line(pos2(c.x - 3.0 * r, c.y), pos2(c.x + 3.0 * r, c.y));
                line(pos2(c.x, c.y - 3.0 * r), pos2(c.x, c.y + 3.0 * r));
                painter.circle_stroke(c, r, guide);
            }
        }
        if self.focus_for(camera) == Focus::TiltShift && now < self.guides_until {
            let fade = ((self.guides_until - now) / 0.4).clamp(0.0, 1.0) as f32;
            let color = AMBER.gamma_multiply(0.85 * fade);
            for edge in [self.band_at - self.band, self.band_at + self.band] {
                let y = screen.top() + edge.clamp(0.0, 1.0) * screen.height();
                let mut x = screen.left();
                while x < screen.right() {
                    painter.line_segment([pos2(x, y), pos2((x + 14.0).min(screen.right()), y)], Stroke::new(1.5, color));
                    x += 24.0;
                }
            }
        }
        if let Some((at, since)) = self.marker {
            let age = (now - since) as f32;
            if age > 1.2 {
                self.marker = None;
            } else {
                let fade = (1.0 - age / 1.2).clamp(0.0, 1.0);
                let color = AMBER_BRIGHT.gamma_multiply(fade);
                let r = 16.0 + 6.0 * (1.0 - fade);
                painter.circle_stroke(at, r, Stroke::new(1.5, color));
                for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
                    painter.line_segment([at + d * (r - 5.0), at + d * (r + 5.0)], Stroke::new(1.5, color));
                }
            }
        }
        if let Some(since) = self.flash {
            let age = (now - since) as f32;
            if age > 0.35 {
                self.flash = None;
            } else {
                let flash = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("photo_flash")));
                flash.rect_filled(screen, 0.0, Color32::from_white_alpha((150.0 * (1.0 - age / 0.35)) as u8));
                ctx.request_repaint();
            }
        }
    }
}

/// The panel's footer: TAKE PHOTO (true when clicked), its keys, the last photos taken (newest
/// first, a click away from their folder) and the notice of the last one saved.
fn footer(ui: &mut egui::Ui, shots: &[Shot], notice: Option<&(String, Option<PathBuf>, f64)>) -> bool {
    let busy = shots.iter().any(|s| s.saving.is_some());
    let take = panel::hero_button(ui, "TAKE PHOTO", vec2(ui.available_width(), 44.0), !busy).clicked();
    ui.add_space(space::XS);
    ui.add(egui::Label::new(RichText::new(t(KEYS)).font(body(12.0)).color(STEEL_DIM)).wrap());
    if !shots.is_empty() {
        ui.add_space(space::S);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let size = vec2((WIDTH - 3.0 * 6.0) / SHOTS as f32, 56.0);
            for shot in shots.iter().rev() {
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                let painter = ui.painter();
                painter.add(ta_ui::metal::well(rect, INSET));
                let inner = rect.shrink(2.0);
                match &shot.thumb {
                    Some(texture) => {
                        // (Cut to fill the tile: its middle.)
                        let [tw, th] = texture.size().map(|v| v as f32);
                        let (want, have) = (inner.width() / inner.height(), tw / th.max(1.0));
                        let uv = if have > want {
                            let k = want / have;
                            Rect::from_min_max(pos2(0.5 - k / 2.0, 0.0), pos2(0.5 + k / 2.0, 1.0))
                        } else {
                            let k = have / want;
                            Rect::from_min_max(pos2(0.0, 0.5 - k / 2.0), pos2(1.0, 0.5 + k / 2.0))
                        };
                        painter.image(texture.id(), inner, uv, Color32::WHITE);
                    }
                    None => {
                        painter.text(inner.center(), Align2::CENTER_CENTER, "…", strong(16.0), STEEL);
                    }
                }
                if response.hovered() {
                    painter.rect_stroke(rect, 2.0, Stroke::new(1.5, AMBER), egui::StrokeKind::Inside);
                }
                if let Some(path) = &shot.path {
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    if response.on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        reveal_file(path);
                    }
                }
            }
        });
    }
    if let Some((text, folder, _)) = notice {
        ui.add_space(space::S);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(text.as_str()).font(strong(12.5)).color(if folder.is_some() { GREEN } else { RED }));
        });
        if let Some(folder) = folder
            && ui.add(ActionButton::secondary("OPEN FOLDER").size(140.0, 26.0)).clicked()
        {
            open_folder(folder);
        }
    }
    take
}

/// A look's tile: a strip of a battlefield's colours (grass, earth, sand, sky, a fire, the
/// light) as the look makes them, its name on a band under it; the one chosen in brass.
fn look_tile(ui: &mut egui::Ui, preset: Preset, selected: bool, base: &PostSettings, size: egui::Vec2) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let grade = preset.grade(base);
    const FIELD: [[f32; 3]; 6] = [[0.05, 0.11, 0.04], [0.16, 0.12, 0.07], [0.48, 0.38, 0.22], [0.32, 0.5, 0.78], [0.95, 0.42, 0.1], [0.9, 0.88, 0.82]];
    let strip = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y - 16.0));
    let each = strip.width() / FIELD.len() as f32;
    for (i, c) in FIELD.iter().enumerate() {
        let [r, g, b] = graded(*c, &grade);
        let band = Rect::from_min_size(pos2(strip.min.x + i as f32 * each, strip.min.y), vec2(each + 0.5, strip.height()));
        painter.rect_filled(band, 0.0, Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8));
    }
    let label = Rect::from_min_max(pos2(rect.min.x, strip.max.y), rect.max);
    painter.rect_filled(label, 0.0, if selected { Color32::from_rgb(58, 48, 26) } else { Color32::from_black_alpha(170) });
    painter.text(label.center(), Align2::CENTER_CENTER, t(preset.label()), strong(10.5), if selected { AMBER_BRIGHT } else { TEXT });
    let hot = response.hovered();
    let outline = if selected { Stroke::new(2.0, AMBER) } else if hot { Stroke::new(1.0, AMBER.gamma_multiply(0.6)) } else { Stroke::new(1.0, Color32::from_black_alpha(160)) };
    painter.rect_stroke(rect, 2.0, outline, egui::StrokeKind::Inside);
    w::reveal_focus(ui, &response);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A colour (linear) as a look makes it, in display values (as the composite's `photo_look`,
/// without the grain): for the looks' tiles.
fn graded(c: [f32; 3], g: &Grade) -> [f32; 3] {
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let c = c.map(|v| v * g.exposure);
    let l = luma(c);
    let c = c.map(|v| (l + (v - l) * g.saturation).max(0.0));
    let mut d = c.map(|v| v.clamp(0.0, 1.0).powf(1.0 / 2.2));
    match g.special {
        Sight::Night => {
            let l = (luma(d).powf(0.85) * 1.1).clamp(0.0, 1.0);
            let l3 = l * l * l;
            d = [0.3 * l + 0.35 * l3, l + 0.02, 0.38 * l + 0.3 * l3];
        }
        Sight::Thermal => {
            const STOPS: [[f32; 3]; 6] = [[0.02, 0.02, 0.09], [0.2, 0.04, 0.5], [0.7, 0.06, 0.45], [0.97, 0.33, 0.06], [1.0, 0.8, 0.16], [1.0, 1.0, 0.92]];
            let x = luma(d).powf(1.7).clamp(0.0, 1.0) * 5.0;
            let i = (x.floor() as usize).min(4);
            let f = x - i as f32;
            d = [0, 1, 2].map(|k| STOPS[i][k] + (STOPS[i + 1][k] - STOPS[i][k]) * f);
        }
        Sight::Eye => {
            d = d.map(|v| g.lift + (1.0 - g.lift) * v);
            d = d.map(|v| (v - 0.5) * g.contrast + 0.5);
            let l = luma(d);
            d = d.map(|v| v + (l - v) * g.mono);
            d = [d[0] * (1.0 + 0.14 * g.warmth), d[1] * (1.0 + 0.02 * g.warmth), d[2] * (1.0 - 0.14 * g.warmth)];
            let l = luma(d);
            let smooth = |a: f32, b: f32, x: f32| {
                let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            let (lows, highs) = (1.0 - smooth(0.0, 0.75, l), smooth(0.25, 1.0, l));
            let (sl, hl) = (luma([g.shadows[0], g.shadows[1], g.shadows[2]]), luma([g.highlights[0], g.highlights[1], g.highlights[2]]));
            d = [0, 1, 2].map(|k| d[k] + (g.shadows[k] - sl) * lows * g.shadows[3] + (g.highlights[k] - hl) * highs * g.highlights[3]);
        }
    }
    d.map(|v| v.clamp(0.0, 1.0))
}

/// The perspective camera through a lens of `fov` (vertical degrees), tilted `roll` degrees: the
/// eye stays where it is, so the picture closes in or widens round its middle.
fn set_lens(camera: &mut Camera, fov: f32, roll: f32) {
    let fov = fov.to_radians();
    if (camera.fov_y - fov).abs() > 1e-5 {
        let distance = camera.distance();
        camera.fov_y = fov;
        camera.zoom = camera.viewport.y / (2.0 * (fov * 0.5).tan() * distance.max(1.0));
    }
    camera.roll = roll.to_radians();
}

/// A lens's focal length on a full-frame camera (millimetres) for a vertical field of view
/// (degrees): what a photographer would call it.
fn lens_mm(fov: f32) -> f32 {
    12.0 / (fov.to_radians() * 0.5).tan()
}

/// The largest rectangle of `aspect` (width over height; none: all of it) in the middle of
/// `rect`.
fn frame_rect(rect: Rect, aspect: Option<f32>) -> Rect {
    let Some(aspect) = aspect else { return rect };
    let (w, h) = (rect.width(), rect.height());
    let size = if w / h.max(1.0) > aspect { vec2(h * aspect, h) } else { vec2(w, w / aspect.max(1e-3)) };
    Rect::from_center_size(rect.center(), size)
}

/// A picture `size` large cut to `aspect`: its size then.
fn framed(size: (u32, u32), aspect: Option<f32>) -> (u32, u32) {
    let r = frame_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(size.0 as f32, size.1 as f32)), aspect);
    (r.width().round().max(1.0) as u32, r.height().round().max(1.0) as u32)
}

/// The middle of a picture, cut to `aspect`.
fn crop_to_aspect(shot: Screenshot, aspect: f32) -> Screenshot {
    let (w, h) = framed((shot.width, shot.height), Some(aspect));
    if (w, h) == (shot.width, shot.height) {
        return shot;
    }
    let (x0, y0) = ((shot.width - w) / 2, (shot.height - h) / 2);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..y0 + h {
        let start = ((y * shot.width + x0) * 4) as usize;
        rgba.extend_from_slice(&shot.rgba[start..start + (w * 4) as usize]);
    }
    Screenshot { width: w, height: h, rgba }
}

/// A photo's thumbnail (for the panel's strip).
fn thumbnail(shot: &Screenshot) -> Option<egui::ColorImage> {
    let image = image::RgbaImage::from_raw(shot.width, shot.height, shot.rgba.clone())?;
    let height = (THUMB as f32 * shot.height as f32 / shot.width.max(1) as f32).round().max(1.0) as u32;
    let small = image::imageops::thumbnail(&image, THUMB, height);
    Some(egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw()))
}

/// The caption stamped into a photo's lower left corner: the corner darkened a little behind it,
/// a brass rule, the title in brass, the line under it in white, each with a soft shadow. Sized
/// by the picture's height.
fn stamp(shot: &mut Screenshot, caption: &Caption) {
    use ab_glyph::{Font, FontVec, PxScale, ScaleFont, VariableFont, point};
    use ta_ui::theme::{Weight, ui_face};
    // The interface's faces: its regular for the line, its condensed bold for the title.
    let font_of = |weight: Weight| {
        let face = ui_face(weight);
        let mut font = FontVec::try_from_vec(face.bytes.to_vec()).ok()?;
        for (tag, value) in face.coords {
            let _ = font.set_variation(tag, *value);
        }
        Some(font)
    };
    let (Some(line_font), Some(title_font)) = (font_of(Weight::Regular), font_of(Weight::Display)) else { return };
    let h = shot.height as f32;
    let margin = (h * 0.035).round();
    let (title_px, line_px) = (h * 0.034, h * 0.021);
    let (w, hh) = (shot.width as i64, shot.height as i64);
    // The corner behind the caption darkened (a soft rise of shade towards it).
    let shade_w = (h * 0.62).min(shot.width as f32);
    let shade_h = h * 0.2;
    for y in (hh - shade_h as i64).max(0)..hh {
        for x in 0..(shade_w as i64).min(w) {
            let fx = 1.0 - x as f32 / shade_w;
            let fy = 1.0 - (hh - 1 - y) as f32 / shade_h;
            let k = 1.0 - 0.45 * (fx * fy).powf(0.8);
            let i = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                shot.rgba[i + c] = (shot.rgba[i + c] as f32 * k) as u8;
            }
        }
    }
    let mut blend = |x: i64, y: i64, colour: [f32; 3], alpha: f32| {
        if x < 0 || y < 0 || x >= w || y >= hh || alpha <= 0.0 {
            return;
        }
        let i = ((y * w + x) * 4) as usize;
        for (old, new) in shot.rgba[i..i + 3].iter_mut().zip(colour) {
            *old = (*old as f32 + (new * 255.0 - *old as f32) * alpha.min(1.0)) as u8;
        }
    };
    // A line of text from `x` with its baseline at `y`, letter-spaced, in `colour` over a
    // shadow; its width.
    let write = |font: &FontVec, text: &str, px: f32, x: f32, y: f32, tracking: f32, colour: [f32; 3], blend: &mut dyn FnMut(i64, i64, [f32; 3], f32)| -> f32 {
        let scaled = font.as_scaled(PxScale::from(px));
        let shadow = (px * 0.06).max(1.0);
        let mut caret = x;
        let mut last = None;
        for c in text.chars() {
            let id = font.glyph_id(c);
            if let Some(prev) = last {
                caret += scaled.kern(prev, id);
            }
            for (dx, dy, tint, k) in [(shadow, shadow, [0.0, 0.0, 0.0], 0.55), (0.0, 0.0, colour, 1.0)] {
                let glyph = id.with_scale_and_position(PxScale::from(px), point(caret + dx, y + dy));
                if let Some(outline) = font.outline_glyph(glyph) {
                    let b = outline.px_bounds();
                    outline.draw(|gx, gy, cover| blend(b.min.x as i64 + gx as i64, b.min.y as i64 + gy as i64, tint, cover * k));
                }
            }
            caret += scaled.h_advance(id) + tracking;
            last = Some(id);
        }
        caret - x
    };
    let line_base = h - margin;
    let title_base = line_base - line_px * 1.45;
    write(&line_font, &caption.line.to_uppercase(), line_px, margin, line_base, line_px * 0.12, [0.95, 0.94, 0.9], &mut blend);
    let width = write(&title_font, &caption.title.to_uppercase(), title_px, margin, title_base, title_px * 0.16, [0.95, 0.82, 0.49], &mut blend);
    // The brass rule over them.
    let rule_y = (title_base - title_px * 1.05) as i64;
    let thick = (h * 0.0025).max(1.0) as i64;
    for y in rule_y..rule_y + thick {
        for x in margin as i64..(margin + width.max(h * 0.1)) as i64 {
            blend(x, y, [0.85, 0.7, 0.34], 0.9);
        }
    }
}

/// Photos being saved (their threads), waited for before the program ends.
static SAVING: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());

/// Waits (up to `timeout`) for the photos still being saved: quitting right after TAKE PHOTO
/// does not leave half a picture.
pub fn wait_for_saves(timeout: std::time::Duration) {
    let until = std::time::Instant::now() + timeout;
    let saving = SAVING.lock().map(|mut all| std::mem::take(&mut *all)).unwrap_or_default();
    for thread in saving {
        while !thread.is_finished() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

/// Reserve a name before encoding: two background saves (or two game processes) can choose
/// the same next name, but only one may create it. Encoding uses that open file throughout.
fn save_picture(shot: &Screenshot, dir: &Path, stem: &str) -> image::ImageResult<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let (path, file) = loop {
        let path = ta_config::next_picture(dir, stem);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => break (path, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    };
    let mut writer = BufWriter::new(file);
    let result = image::write_buffer_with_format(&mut writer, &shot.rgba, shot.width, shot.height, image::ColorType::Rgba8, image::ImageFormat::Png)
        .and_then(|_| writer.flush().map_err(image::ImageError::from));
    // Close before removing on Windows. A failed encode must not leave an empty photo.
    drop(writer);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

/// The photo's size for a window `window` pixels large: the window's, or up to twice it
/// (within 4K) when `large`.
pub fn photo_size(window: (u32, u32), large: bool) -> (u32, u32) {
    let (w, h) = (window.0.max(1) as f64, window.1.max(1) as f64);
    let k = if large { (MAX_PIXELS / (w * h)).sqrt().clamp(1.0, 2.0) } else { 1.0 };
    ((w * k).round() as u32, (h * k).round() as u32)
}

/// "06:45".
fn clock(hours: f32) -> String {
    let minutes = (hours.rem_euclid(24.0) * 60.0).round() as u32 % (24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Today's date, as the caption gives it ("2026-10-28").
pub fn today() -> String {
    let (y, m, d) = ta_ui::seasonal::local_date();
    format!("{y:04}-{m:02}-{d:02}")
}

/// Shows a folder in the system's file browser.
pub fn open_folder(folder: &Path) {
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    if let Err(e) = std::process::Command::new(program).arg(folder).spawn() {
        log::warn!("could not open {}: {e}", folder.display());
    }
}

/// Shows a file in the system's file browser, chosen (where the browser can; else its folder).
fn reveal_file(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn().is_ok() {
            return;
        }
    }
    #[cfg(target_os = "macos")]
    {
        if std::process::Command::new("open").arg("-R").arg(path).spawn().is_ok() {
            return;
        }
    }
    if let Some(folder) = path.parent() {
        open_folder(folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PictureDir(PathBuf);

    impl PictureDir {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("ta-photo-{}-{time}-{serial}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for PictureDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn concurrent_photos_keep_every_image_and_existing_files() {
        const COUNT: u8 = 12;
        let dir = PictureDir::new();
        let existing = dir.0.join("Coast To Coast 001.png");
        std::fs::write(&existing, b"existing photo").unwrap();
        let start = std::sync::Barrier::new(COUNT as usize);
        let saved = std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for i in 0..COUNT {
                let (dir, start) = (&dir.0, &start);
                workers.push(scope.spawn(move || {
                    let pixel = [i, 255 - i, 7, 255];
                    let shot = Screenshot { width: 1, height: 1, rgba: pixel.to_vec() };
                    start.wait();
                    (save_picture(&shot, dir, "Coast To Coast").unwrap(), pixel)
                }));
            }
            workers.into_iter().map(|w| w.join().unwrap()).collect::<Vec<_>>()
        });
        let mut paths = std::collections::HashSet::new();
        for (path, pixel) in saved {
            assert!(paths.insert(path.clone()), "every save needs its own name");
            assert_eq!(image::open(path).unwrap().into_rgba8().as_raw().as_slice(), pixel);
        }
        assert_eq!(std::fs::read(existing).unwrap(), b"existing photo");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), COUNT as usize + 1);
    }

    #[test]
    fn failed_photo_encoding_removes_its_reserved_file() {
        let dir = PictureDir::new();
        let shot = Screenshot { width: 0, height: 1, rgba: Vec::new() };
        assert!(save_picture(&shot, &dir.0, "Invalid").is_err());
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
    }

    #[test]
    fn photos_are_up_to_twice_the_window_within_4k() {
        assert_eq!(photo_size((1920, 1080), false), (1920, 1080));
        assert_eq!(photo_size((1920, 1080), true), (3840, 2160));
        assert_eq!(photo_size((1280, 720), true), (2560, 1440));
        // A larger window gets what fits in 4K, and never less than itself.
        assert_eq!(photo_size((2560, 1440), true), (3840, 2160));
        assert_eq!(photo_size((5120, 2880), true), (5120, 2880));
    }

    #[test]
    fn the_clock_reads_hours_and_minutes() {
        assert_eq!(clock(6.75), "06:45");
        assert_eq!(clock(24.0), "00:00");
        assert_eq!(clock(18.5), "18:30");
    }

    /// A frame cuts the middle of the picture to its shape, in the window or a photo alike.
    #[test]
    fn frames_cut_the_middle_to_their_shape() {
        assert_eq!(framed((1920, 1080), None), (1920, 1080));
        assert_eq!(framed((1920, 1080), Frame::Square.aspect()), (1080, 1080));
        assert_eq!(framed((1920, 1080), Frame::Tall.aspect()), (608, 1080));
        assert_eq!(framed((3840, 2160), Frame::Cinema.aspect()), (3840, 1607));
        // The pixels kept are the middle's, row by row.
        let shot = Screenshot { width: 4, height: 2, rgba: (0..32).collect() };
        let cut = crop_to_aspect(shot, 1.0);
        assert_eq!((cut.width, cut.height), (2, 2));
        assert_eq!(cut.rgba, [4, 5, 6, 7, 8, 9, 10, 11, 20, 21, 22, 23, 24, 25, 26, 27]);
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 900.0));
        let r = frame_rect(screen, Some(4.0 / 3.0));
        assert!((r.width() - 1200.0).abs() < 0.01 && r.height() == 900.0 && (r.center().x - 800.0).abs() < 0.01);
    }

    /// Each look makes its own colours of a battlefield's, and the natural one leaves them as
    /// they are (bar the display's curve); the specials are their own palettes.
    #[test]
    fn looks_grade_the_picture_their_own_way() {
        let base = PostSettings::default();
        let grass = [0.05, 0.11, 0.04];
        let natural = graded(grass, &Preset::Natural.grade(&base));
        let looks: Vec<[f32; 3]> = Preset::ALL.iter().map(|p| graded(grass, &p.grade(&base))).collect();
        for (i, a) in looks.iter().enumerate() {
            for b in &looks[i + 1..] {
                assert!(a.iter().zip(b).any(|(x, y)| (x - y).abs() > 0.01), "two looks alike: {a:?} {b:?}");
            }
        }
        // Noir is grey; the night sight green; the thermal camera's darks blue-purple.
        let noir = graded([0.5, 0.1, 0.1], &Preset::Noir.grade(&base));
        assert!((noir[0] - noir[1]).abs() < 0.02 && (noir[1] - noir[2]).abs() < 0.02, "{noir:?}");
        let night = graded(grass, &Preset::Night.grade(&base));
        assert!(night[1] > night[0] * 2.0 && night[1] > night[2] * 2.0, "{night:?}");
        let thermal = graded([0.01, 0.01, 0.01], &Preset::Thermal.grade(&base));
        assert!(thermal[2] >= thermal[1], "{thermal:?}");
        // (Natural changes nothing but the curve.)
        assert!((natural[1] - (0.11f32 * base.exposure).powf(1.0 / 2.2)).abs() < 0.05, "{natural:?}");
    }

    /// The lens closes in round the middle of the picture: the eye stays where it is.
    #[test]
    fn the_lens_keeps_the_eye_where_it_is() {
        let mut camera = Camera::new(glam::Vec2::new(1920.0, 1080.0));
        camera.focus = glam::Vec3::new(2000.0, 0.0, 2000.0);
        camera.pitch = 0.6;
        let eye = camera.eye();
        let mut long = camera.clone();
        set_lens(&mut long, 10.0, 5.0);
        assert!((long.eye() - eye).length() < 0.5, "{:?} {:?}", long.eye(), eye);
        assert!((long.fov_y.to_degrees() - 10.0).abs() < 1e-3 && (long.roll.to_degrees() - 5.0).abs() < 1e-3);
        assert!(long.zoom > camera.zoom * 2.0, "a long lens magnifies");
        let mut wide = long.clone();
        set_lens(&mut wide, 80.0, 0.0);
        assert!((wide.eye() - eye).length() < 0.5 && wide.zoom < camera.zoom, "a wide lens takes in more from the same place");
        // (A 50 mm lens's field: about 27 degrees.)
        assert!((lens_mm(27.0) - 50.0).abs() < 1.5);
    }

    /// The caption darkens its corner and writes over it; the rest of the photo is untouched.
    #[test]
    fn the_caption_stamps_its_corner_only() {
        let (w, h) = (640u32, 360u32);
        let mut shot = Screenshot { width: w, height: h, rgba: vec![128; (w * h * 4) as usize] };
        stamp(&mut shot, &Caption { title: "TA Reborn".into(), line: "Sapphire Reach · 2026-10-28".into() });
        let px = |x: u32, y: u32| &shot.rgba[((y * w + x) * 4) as usize..((y * w + x) * 4 + 3) as usize];
        assert_eq!(px(w - 2, 2), [128, 128, 128], "the top right untouched");
        let corner: Vec<&[u8]> = (h - 40..h - 8).flat_map(|y| (10..200).map(move |x| (x, y))).map(|(x, y)| px(x, y)).collect();
        assert!(corner.iter().any(|p| p[0] > 150), "lettering written in the corner");
        assert!(corner.iter().any(|p| p[0] < 110), "the corner darkened behind it");
    }
}
