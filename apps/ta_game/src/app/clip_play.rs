//! Social media clips in the app (`clips.rs` chooses and directs them, `clip_dialog.rs` offers
//! them, `film.rs` makes them): the clip's camera through the film, the EXPORT CLIP dialog's
//! pictures of each stretch's climax, and the capture switch.
//!
//! The pictures ([`ClipPreviews`]): the dialog asks for one at each climax it offers; the replay
//! is taken to a moment before it (fast-forwarded, or started over), played up to it so its
//! blasts and smoke are under way, paused, and drawn with the clip's camera in both shapes. The
//! battle moves behind the dialog meanwhile, its sound held down. When the dialog closes without
//! a clip the replay goes back where it was, its camera and pace as they were; a clip begun
//! keeps that for when it is done.
//!
//! Captures: `TA_CLIP=30v|30h|60v|60h` makes the replay's best clip of that length and shape as
//! it opens (once its scan is done), `TA_CLIP_PICK=n` the n-th offered instead, `TA_CLIP_DIALOG=1`
//! opens the dialog on it instead, `TA_CLIP_MUSIC=1` with the music, `TA_CLIP_SOUND=0` silent,
//! `TA_CLIP_TITLES=0` and `TA_CLIP_SCOREBOARD=0` without those.

use glam::{Vec2, Vec3};

use super::*;
use crate::clip_dialog::PreviewShot;
use crate::clips::Shape;

/// Ticks the replay plays before a climax's picture is taken (its blasts under way).
const LEAD: u64 = 24;

/// The EXPORT CLIP dialog's pictures being taken (see the module documentation).
pub(super) struct ClipPreviews {
    /// Those still to take, the earliest first.
    shots: Vec<PreviewShot>,
    current: Option<(PreviewShot, Step)>,
    back: Back,
}

/// How far a picture has got: the replay going to it, playing up to it, there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Seeking,
    Playing,
    Shoot,
}

/// How the replay was before the pictures: where, its camera and its pace.
pub(super) struct Back {
    tick: u64,
    pub(super) camera: Camera,
    pub(super) speed: f32,
    paused: bool,
}

/// A clip's camera (`shape`) looking at `at` (world x and z, the ground `ground` high there)
/// where the action spreads `spread`, `seconds` into a clip `length` long, for a picture `height`
/// pixels high: made from `window`, the window's camera, whose viewport it keeps (a picture's is
/// set as it is drawn, `Renderer::render_image`).
pub(super) fn clip_camera(window: &Camera, shape: Shape, at: Vec2, spread: f32, seconds: f32, length: f32, ground: f32, height: u32) -> Camera {
    let (w, h) = shape.size();
    let mut cam = window.clone();
    cam.mode = CameraMode::Perspective;
    cam.focus = Vec3::new(at.x, ground, at.y);
    cam.yaw = crate::clips::yaw(seconds, length);
    cam.pitch = shape.pitch();
    (cam.roll, cam.lift) = (0.0, 0.0);
    cam.zoom = shape.zoom(spread) * cam.viewport.y / h as f32;
    // (The action in the middle of the part of the picture the apps leave clear: pixels of the
    // picture, as it is drawn.)
    let safe = shape.safe();
    let k = height as f32 / h as f32;
    cam.view_offset = Vec2::new(((safe.left + 1.0 - safe.right) / 2.0 - 0.5) * w as f32 * k, ((safe.top + 1.0 - safe.bottom) / 2.0 - 0.5) * h as f32 * k);
    cam
}

impl State {
    /// The ground's height at `at` (world x and z), the sea's where it is higher.
    fn clip_ground(&self, at: Vec2) -> f32 {
        self.renderer.terrain.as_ref().map_or(0.0, |t| t.field.sample(at.x, at.y).max(t.field.sea_level))
    }

    /// While a clip is filmed: the camera where its plan has it.
    pub(super) fn clip_film_camera(&mut self) {
        let Some(film) = self.film.as_ref() else { return };
        let Some(clip) = film.clip.as_ref() else { return };
        let (at, spread) = clip.plan.at(self.game.sim.tick as f32);
        let (shape, seconds, length) = (clip.look.shape, film.seconds(), clip.look.seconds);
        let ground = self.clip_ground(at);
        self.camera = clip_camera(&self.camera, shape, at, spread, seconds, length, ground, shape.size().1);
    }

    /// The dialog's new pictures to take, queued (the replay's place and pace kept the first time).
    pub(super) fn queue_clip_previews(&mut self, shots: Vec<PreviewShot>) {
        if shots.is_empty() || self.film.is_some() {
            return;
        }
        let back = Back { tick: self.game.sim.tick, camera: self.camera.clone(), speed: self.game.speed, paused: self.game.paused };
        let previews = self.clip_previews.get_or_insert_with(|| ClipPreviews { shots: Vec::new(), current: None, back });
        previews.shots.extend(shots);
        previews.shots.sort_by_key(|s| s.tick);
        log::info!("clip pictures: {} to take", previews.shots.len());
    }

    /// After the frame's ticks: the next picture's climax gone to, played up to and paused at, the
    /// camera on it (as the vertical picture frames it, the one reaching furthest up and down).
    pub(super) fn clip_previews_after_update(&mut self) {
        if self.clip_previews.is_none() {
            return;
        }
        if self.screen != Screen::Playing || self.game.playback.as_ref().is_none_or(|p| p.resume) || self.film.is_some() {
            self.clip_previews = None;
            return;
        }
        let arrived = self.game.catch_up_progress().is_none() && self.game.request_rewind.is_none() && self.pending.is_none();
        let Some(previews) = self.clip_previews.as_mut() else { return };
        match previews.current {
            None => {
                if previews.shots.is_empty() {
                    return;
                }
                // (Those still ahead first: the replay goes on to them without starting over.)
                let here = self.game.sim.tick;
                let next = previews.shots.iter().position(|s| s.tick.saturating_sub(LEAD) >= here).unwrap_or(0);
                let shot = previews.shots.remove(next);
                previews.current = Some((shot, Step::Seeking));
                self.game.seek_replay(shot.tick.saturating_sub(LEAD));
                self.game.set_pace(Some(false), Some(1.0));
            }
            Some((shot, Step::Seeking)) if arrived => previews.current = Some((shot, Step::Playing)),
            Some((shot, Step::Playing)) if self.game.sim.tick >= shot.tick || self.game.replay_finished() => {
                self.game.set_pace(Some(true), None);
                previews.current = Some((shot, Step::Shoot));
            }
            _ => {}
        }
        if let Some((shot, step)) = previews.current
            && step != Step::Seeking
        {
            let ground = self.clip_ground(shot.focus);
            let seconds = shot.tick.saturating_sub(shot.from) as f32 / 30.0;
            self.camera = clip_camera(&self.camera, Shape::Vertical, shot.focus, shot.spread * 1.15, seconds, shot.seconds, ground, Shape::Vertical.size().1);
        }
    }

    /// Before the frame is drawn: the picture of the climax arrived at, in both shapes, for the
    /// dialog.
    pub(super) fn clip_preview_pictures(&mut self) {
        let Some(previews) = self.clip_previews.as_mut() else { return };
        let Some((shot, Step::Shoot)) = previews.current else { return };
        previews.current = None;
        let left = previews.shots.len();
        let ground = self.clip_ground(shot.focus);
        let seconds = shot.tick.saturating_sub(shot.from) as f32 / 30.0;
        for shape in [Shape::Vertical, Shape::Horizontal] {
            let (w, h) = shape.size();
            let k = crate::clip_dialog::PICTURE_LONG as f32 / w.max(h) as f32;
            let (pw, ph) = (((w as f32 * k) as u32) & !1, ((h as f32 * k) as u32) & !1);
            let cam = clip_camera(&self.camera, shape, shot.focus, shot.spread, seconds, shot.seconds, ground, ph);
            match self.renderer.render_image(&cam, pw, ph) {
                Some(picture) => self.hud.put_clip_picture(&self.egui.ctx, shot.tick, shape, &picture.rgba, picture.width as usize, picture.height as usize),
                None => log::warn!("clip pictures: the picture at {} could not be drawn", shot.tick),
            }
        }
        if left == 0 {
            log::info!("clip pictures: all taken");
        }
    }

    /// The dialog closed without a clip: the pictures stop, the replay goes back where it was.
    pub(super) fn end_clip_previews(&mut self) {
        let Some(previews) = self.clip_previews.take() else { return };
        let back = previews.back;
        if self.drop_preview_seek() {
            self.game.seek_replay(back.tick);
        }
        self.camera = back.camera;
        self.game.set_pace(Some(back.paused), Some(back.speed));
        log::info!("clip pictures: back to {}", crate::clips::clock(back.tick));
    }

    /// How the replay was before the dialog's pictures, for a clip begun after them: the pictures
    /// stopped, and the seek one of them had under way given up, so that the clip's own (asked
    /// for after this, `start_film`) is the one the replay makes. (A rewind of a picture's already
    /// waiting in `pending` would otherwise make the battle again after the clip's seek was asked
    /// for, and the clip begin where the picture was to be taken.)
    pub(super) fn take_clip_back(&mut self) -> Option<Back> {
        let previews = self.clip_previews.take()?;
        self.drop_preview_seek();
        Some(previews.back)
    }

    /// The seek a picture had under way given up: a fast-forward, a rewind asked for, a rewind
    /// waiting to be made (the dialog is modal: any is the pictures'); the scene brought up to
    /// the tick the replay is at. Whether the replay can seek (it is a replay).
    fn drop_preview_seek(&mut self) -> bool {
        let Some(playback) = self.game.playback.as_mut().filter(|p| !p.resume) else { return false };
        playback.seek_to = None;
        self.game.request_rewind = None;
        if matches!(self.pending, Some(Transition::Rewind(..))) {
            self.pending = None;
        }
        self.game.catch_up(std::time::Duration::ZERO);
        true
    }

    /// Captures: `TA_CLIP`'s clip once the replay's scan is done (or its dialog, `TA_CLIP_DIALOG`).
    pub(super) fn start_clip_capture(&mut self, spec: crate::clips::ClipSpec) {
        let var = |name: &str| std::env::var(name).ok().map(|v| v.trim().to_string());
        if var("TA_CLIP_DIALOG").as_deref() == Some("1") {
            self.hud.open_clip_dialog(spec);
            return;
        }
        let Some(end) = self.game.playback.as_ref().filter(|p| !p.resume).map(|p| p.end_tick) else { return };
        let Some((Some(log), marks)) = self.game.replay_scan.as_ref().and_then(|s| s.state().lock().ok().map(|s| (s.action.clone(), s.marks.clone()))) else {
            log::warn!("TA_CLIP: the replay has no scan");
            return;
        };
        let found = crate::clips::candidates(&log, &marks, end, spec);
        for (i, c) in found.iter().enumerate() {
            log::info!("clip {}: {} {} ({}), score {:.2}, climax {}", i + 1, c.span_text(), c.title, c.detail(), c.score, crate::clips::clock(c.climax));
        }
        let pick = var("TA_CLIP_PICK").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).max(1);
        let Some(c) = found.get(pick - 1).or(found.first()) else {
            log::warn!("TA_CLIP: the replay offers no clip");
            return;
        };
        let off = |name: &str| var(name).as_deref() == Some("0");
        let settings = crate::film::FilmSettings {
            titles: !off("TA_CLIP_TITLES"),
            scoreboard: !off("TA_CLIP_SCOREBOARD"),
            sounds: !off("TA_CLIP_SOUND"),
            music: var("TA_CLIP_MUSIC").as_deref() == Some("1") && !off("TA_CLIP_SOUND"),
            ..Default::default()
        };
        let choice = crate::film::ClipChoice { spec, from: c.from, to: c.to, climax: c.climax, focus: c.focus };
        self.start_film(settings.for_clip(choice));
    }
}
