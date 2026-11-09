//! The intro plays on a CRT in a teenager's bedroom late at night, in the autumn of 1997: a
//! beige computer on a wooden desk, a modem blinking on the tower, the keyboard and the mouse on
//! its mat, a lava lamp glowing under a shelf with the game's boxes, posters on the wall,
//! moonlight in through the blinds. The monitor comes on from a line with a degaussing swirl,
//! TA's movies play on it and light the room in their colours, and the view drifts with the
//! mouse; at the end the camera leans in to the screen and the game begins.
//!
//! The room is a model made for TA Reborn (`ta_render::room`, built by
//! `tools/room/build.py`); its pictures are painted on a worker thread from the install's art
//! (`intro_room_art.rs`). Units are metres in the model's space: x to the right, y up, z
//! towards the viewer.

use std::sync::mpsc;

use glam::{Vec2, Vec3};
use ta_render::room::{FestiveLight, GLOWS, LAYER_SIZE, RoomFrame, RoomModel, RoomScene, Season};

use crate::intro_room_art as art;

/// Seconds the room is dark before the monitor comes on, and the tube takes to warm up.
const DARK: f32 = 0.7;
const WARM_UP: f32 = 1.1;
/// Seconds the degaussing's swirl takes to settle, from when the picture opens.
const DEGAUSS: f32 = 1.4;
/// Seconds the camera takes to lean in to the screen at the end.
const LEAN_IN: f32 = 1.3;
/// Seconds the camera takes to come to the seat as the room comes up: from a step back, higher,
/// looking across at the moonlight on the wall, turning to the monitor as it comes on; and
/// where it starts from, and looks, from the seat's (metres, the model's space).
const ARRIVE: f32 = 5.5;
const ARRIVE_EYE: Vec3 = Vec3::new(-0.2, 0.22, 0.62);
const ARRIVE_LOOK: Vec3 = Vec3::new(-0.5, 0.06, 0.0);
/// The main intro (widescreen, letterboxed) is watched closer: seconds the camera takes to come
/// in, how far from the glass it watches (metres), and its field of view.
const FOCUS_IN: f32 = 3.5;
const FOCUS_DISTANCE: f32 = 0.45;
const FOV_FOCUS: f32 = 46.0;
/// Seconds the room's light takes to follow the picture's colours.
const GLOW_EASE: f32 = 0.12;
/// The lens's depth of field (its largest blur, of the picture's height), and as the camera
/// leans in.
const DOF: f32 = 0.0011;
const DOF_FOCUS: f32 = 0.0035;
const DOF_CLOSE: f32 = 0.007;
/// The view's vertical field of view (degrees; as `tools/room/build.py` previews it), and as
/// the camera leans in to the screen.
const FOV: f32 = 50.0;
const FOV_CLOSE: f32 = 36.0;

/// Where the camera sits and looks, and the tube's glass, from the model.
#[derive(Clone, Copy)]
struct View {
    eye: Vec3,
    target: Vec3,
    screen: Vec3,
    facing: Vec3,
    /// Half the glass's height, and how far it bulges.
    screen_half: f32,
    bulge: f32,
}

/// The modem's lamps: high speed, auto answer, carrier, off hook, receive, send, terminal
/// ready, modem ready.
const MODEM: usize = 8;

pub struct IntroRoom {
    building: Option<mpsc::Receiver<Option<(RoomScene, View)>>>,
    /// The room is up in the renderer.
    pub ready: bool,
    /// Building it failed (the intro plays full screen instead).
    failed: bool,
    /// Seconds since the room went up.
    time: f32,
    /// The mouse's pull on the view (-1..1 each way), eased.
    look: Vec2,
    /// The picture's regional colours (linear), eased towards the newest frame's.
    glow: [[f32; 3]; 9],
    regions: [[f32; 3]; 9],
    /// The drive lamp: on until.
    drive_until: f32,
    /// The modem's data lamps: on until.
    data_until: [f32; 2],
    rng: u32,
    /// When the camera started leaning in to the screen.
    closing: Option<f32>,
    /// Lean in at this time (captures).
    lean_at: Option<f32>,
    /// When the camera started coming closer to watch the main intro.
    focus_at: Option<f32>,
    view: View,
    /// What the room is dressed for, and whether its lights hold steady and the fireworks leave
    /// the walls alone (reduced flashes).
    season: Season,
    quiet: bool,
}

impl IntroRoom {
    /// Starts building the room, dressed for `season`: its model, and its pictures from the
    /// install's art.
    pub fn new(assets: &crate::setup::Assets, season: Season, reduced_flashes: bool) -> IntroRoom {
        let (tx, rx) = mpsc::channel();
        let vfs = assets.vfs.clone();
        let data = assets.data.clone();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let built = build(&vfs, &data, season);
            match &built {
                Ok(_) => log::info!("intro room built in {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0),
                Err(e) => log::warn!("the intro room could not be built: {e:#}"),
            }
            let _ = tx.send(built.ok());
        });
        let view = View { eye: Vec3::new(0.0, 1.2, 0.78), target: Vec3::new(0.0, 1.04, -0.3), screen: Vec3::new(0.0, 1.04, -0.2), facing: Vec3::Z, screen_half: 0.13, bulge: 0.013 };
        IntroRoom {
            building: Some(rx),
            ready: false,
            failed: false,
            time: 0.0,
            look: Vec2::ZERO,
            glow: [[0.0; 3]; 9],
            regions: [[0.0; 3]; 9],
            drive_until: 0.0,
            data_until: [0.0; 2],
            rng: 0x2545_f491,
            closing: None,
            lean_at: None,
            focus_at: None,
            view,
            season,
            quiet: reduced_flashes,
        }
    }

    /// Hands the room to the renderer once it is built; true while it is up.
    pub fn poll(&mut self, renderer: &mut ta_render::Renderer) -> bool {
        if let Some(rx) = &self.building {
            match rx.try_recv() {
                Ok(Some((scene, view))) => {
                    renderer.set_room(Some(&scene));
                    self.view = view;
                    self.building = None;
                    self.ready = true;
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.building = None;
                    self.failed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        self.ready
    }

    /// Building the room failed.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// The monitor is warm: the movies may start.
    pub fn warm(&self) -> bool {
        self.ready && self.time > DARK + WARM_UP * 0.8
    }

    /// The camera leans in to the screen, and then the room is done.
    pub fn close(&mut self) {
        if self.closing.is_none() {
            self.closing = Some(self.time);
        }
    }

    /// Leans in `secs` after the room comes up (captures).
    pub fn close_after(&mut self, secs: f32) {
        self.lean_at = Some(secs);
    }

    /// The main intro starts: the camera comes closer to watch it.
    pub fn focus_on_tube(&mut self) {
        if self.focus_at.is_none() {
            self.focus_at = Some(self.time);
        }
    }

    pub fn closing(&self) -> bool {
        self.closing.is_some()
    }

    /// Leaned all the way in (or never came up).
    pub fn closed(&self) -> bool {
        !self.ready || self.closing.is_some_and(|t| self.time - t >= LEAN_IN)
    }

    /// How dark the view is going at the end (0..1), for a fade over it.
    pub fn fade(&self) -> f32 {
        self.closing.map_or(0.0, |t| smooth(((self.time - t) / LEAN_IN - 0.55) / 0.45))
    }

    /// A movie frame: onto the tube, and its colours into the room (eased in, `frame`).
    pub fn picture(&mut self, renderer: &mut ta_render::Renderer, width: u32, height: u32, rgba: &[u8]) {
        renderer.room_picture(width, height, rgba);
        self.regions = regional_colours(width as usize, height as usize, rgba);
    }

    /// This frame of the room: `dt` seconds on, the mouse at `mouse` (-1..1 across the window
    /// each way, None when it is away), `picture` = a movie is on the tube.
    pub fn frame(&mut self, dt: f32, mouse: Option<Vec2>, picture: bool) -> RoomFrame {
        self.time += dt;
        let t = self.time;
        if self.lean_at.is_some_and(|at| t >= at) {
            self.lean_at = None;
            self.close();
        }
        let pull = mouse.unwrap_or(Vec2::ZERO).clamp(Vec2::splat(-1.0), Vec2::splat(1.0));
        self.look += (pull - self.look) * (1.0 - (-dt * 2.5).exp());
        // The room's light follows the picture, smoothly (a cut lights it at once; the frames'
        // own noise does not flicker it); it fades with the picture.
        if !picture {
            self.regions = [[0.0; 3]; 9];
        }
        let ease = 1.0 - (-dt / GLOW_EASE).exp();
        for (g, r) in self.glow.iter_mut().zip(self.regions) {
            for k in 0..3 {
                g[k] += (r[k] - g[k]) * ease;
            }
        }
        // The drive lamp flickers as the movie streams off the disc; the modem's data lamps
        // as someone's download trickles in.
        if picture && t > self.drive_until + 0.05 && self.rand() < 0.3 {
            self.drive_until = t + 0.03 + self.rand() * 0.12;
        }
        for (i, until) in [0usize, 1].into_iter().zip([0.25f32, 0.06]) {
            if t > self.data_until[i] + 0.04 && self.rand() < until {
                self.data_until[i] = t + 0.02 + self.rand() * 0.09;
            }
        }
        let on = |until: f32| if t < until { 1.0 } else { 0.0 };
        let power = smooth((t - DARK) / WARM_UP);
        let mut glows = [0.0f32; GLOWS];
        // The computer's power lamp, its drive's, the monitor's (dim in standby).
        glows[0] = 1.0;
        glows[1] = if picture { on(self.drive_until) } else { 0.0 };
        glows[2] = 0.25 + 0.75 * power;
        let modem = [1.0, 0.0, 1.0, 1.0, on(self.data_until[0]), on(self.data_until[1]), 1.0, 1.0];
        glows[3..3 + MODEM].copy_from_slice(&modem);
        // The turbo display, the windows across the street, the clock.
        glows[11] = 1.0;
        glows[12] = 1.0;
        glows[13] = 1.0;
        // The tube degausses as its picture opens: the colours swirl and settle.
        let opened = DARK + WARM_UP * 0.55;
        let degauss = if t > opened { (1.0 - (t - opened) / DEGAUSS).clamp(0.0, 1.0).powi(2) } else { 0.0 };
        // Clouds cross the moon now and then; the lava lamp's light breathes with its wax.
        let moon = (0.84 + 0.1 * (t * 0.083).sin() + 0.06 * (t * 0.031 + 1.0).sin()).clamp(0.0, 1.0);
        let lava = 0.93 + 0.05 * (t * 0.61).sin() + 0.02 * (t * 2.3).sin();
        // The view: drifting a little, pulled by the mouse; leaning in to the screen at the end.
        let v = self.view;
        let drift = Vec3::new((t * 0.13).sin() * 0.012, (t * 0.09).sin() * 0.007, (t * 0.07).sin() * 0.01);
        let pull = Vec3::new(self.look.x * 0.09, -self.look.y * 0.045, 0.0);
        let arriving = 1.0 - smooth(t / ARRIVE);
        let mut eye = v.eye + drift + pull + ARRIVE_EYE * arriving;
        let mut target = v.target + pull / 3.0 + ARRIVE_LOOK * arriving;
        let mut fov = FOV.to_radians();
        let mut dof = DOF;
        // The main intro, closer: the glass fills most of the view, a little from above, the
        // drift and the mouse's pull calmer.
        if let Some(start) = self.focus_at {
            let k = smooth((t - start) / FOCUS_IN);
            let near = v.screen + v.facing * FOCUS_DISTANCE + Vec3::Y * 0.035 + (drift + pull) * 0.35;
            eye = eye.lerp(near, k);
            target = target.lerp(v.screen + pull * 0.1, k);
            fov += (FOV_FOCUS.to_radians() - fov) * k;
            dof += (DOF_FOCUS - dof) * k;
        }
        if let Some(start) = self.closing {
            // In until the glass fills the view from top to bottom.
            let k = smooth((t - start) / LEAN_IN);
            let close = v.screen_half / (FOV_CLOSE.to_radians() * 0.5).tan() + v.bulge;
            eye = eye.lerp(v.screen + v.facing * close, k);
            target = target.lerp(v.screen, k);
            fov += (FOV_CLOSE.to_radians() - fov) * k;
            dof += (DOF_CLOSE - DOF) * k;
        }
        // Captures: `TA_ROOM_CAMERA=x,y,z,tx,ty,tz,fov` puts the camera anywhere (metres, the
        // model's space; degrees).
        if let Some(c) = camera_override() {
            (eye, target, fov) = (Vec3::new(c[0], c[1], c[2]), Vec3::new(c[3], c[4], c[5]), c[6].to_radians());
        }
        let (lights, bursts, flash) = self.festive(t, &mut glows);
        RoomFrame { eye, target, fov_y: fov, time: t, power, picture, glow: self.glow, glows, moon, lava, degauss, dof, lights, bursts, flash }
    }

    /// The season's lights at time `t`: the glowing parts it puts up (`glows`), the light the tree
    /// or the party throws, the strings' and the fireworks' over the street and in the window.
    fn festive(&self, t: f32, glows: &mut [f32; GLOWS]) -> ([FestiveLight; 3], [[f32; 4]; 4], [f32; 3]) {
        let mut lights = [FestiveLight::default(); 3];
        let mut bursts = [[0.0, 0.0, -1.0, 0.0]; 4];
        let mut flash = [0.0f32; 3];
        match self.season {
            Season::Normal => {}
            Season::Christmas => {
                // The tree's lights twinkle in four colours, each in its own time; the strings
                // shimmer; the star holds. (All steady with reduced flashes.)
                let colours = [[1.0, 0.1, 0.06], [0.1, 1.0, 0.2], [1.0, 0.68, 0.14], [0.15, 0.32, 1.0]];
                let mut mixed = [0.0f32; 3];
                for (i, colour) in colours.iter().enumerate() {
                    let k = i as f32;
                    let on = if self.quiet { 0.8 } else { 0.62 + 0.38 * (t * (1.1 + 0.23 * k) + k * 1.7).sin() * (0.5 + 0.5 * (t * 0.31 + k * 2.3).sin()).sqrt() };
                    glows[14 + i] = on;
                    for (m, c) in mixed.iter_mut().zip(colour) {
                        *m += c * on * 0.25;
                    }
                }
                let shimmer = if self.quiet { 0.9 } else { 0.86 + 0.14 * (t * 0.6).sin() };
                glows[18] = shimmer;
                glows[19] = 1.0;
                lights = [
                    FestiveLight { color: mixed, strength: 0.3 },
                    FestiveLight { color: [1.0, 0.72, 0.42], strength: 0.16 * shimmer },
                    FestiveLight { color: [1.0, 0.72, 0.42], strength: 0.14 * shimmer },
                ];
            }
            Season::NewYear => {
                // The party: a colour turning slowly over the table, gold over the posters; the
                // fireworks over the street and what they throw in at the window.
                let party = hue(t * 0.045).map(|c| 0.35 + 0.65 * c);
                lights = [FestiveLight { color: party, strength: 0.15 }, FestiveLight { color: [1.0, 0.76, 0.36], strength: 0.14 }, FestiveLight::default()];
                (bursts, flash) = fireworks(t, self.quiet);
            }
        }
        (lights, bursts, flash)
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// The season the room is dressed for: `TA_ROOM_SEASON=normal|christmas|newyear` (captures), else
/// the launcher's seasonal setting, as the rest of the game has it (its own choice, the calendar's
/// when automatic; none with the seasonal effects off): Christmas through December, New Year on 31
/// December and 1 January, the ordinary room the rest of the year.
pub fn season(settings: &ta_config::Launcher) -> Season {
    if let Some(season) = std::env::var("TA_ROOM_SEASON").ok().and_then(|v| Season::parse(&v)) {
        return season;
    }
    let (_, month, day) = ta_ui::seasonal::local_date();
    season_on(settings, month, day)
}

/// The room's season on a day (1-based month and day).
fn season_on(settings: &ta_config::Launcher, month: u16, day: u16) -> Season {
    use ta_config::SeasonalTheme;
    use ta_ui::seasonal::Celebration;
    match ta_ui::seasonal::on(settings, month, day) {
        // The launcher keeps its Christmas on to 6 January; the room's, as asked, is December's.
        Some(Celebration::Christmas) if month == 1 && settings.seasonal_theme == SeasonalTheme::Automatic => Season::Normal,
        Some(Celebration::Christmas) => Season::Christmas,
        Some(Celebration::NewYear) => Season::NewYear,
        // (Halloween has no room of its own.)
        Some(Celebration::Halloween) | None => Season::Normal,
    }
}

/// A colour of the rainbow (hue 0..1, full saturation and value), linear.
fn hue(h: f32) -> [f32; 3] {
    let k = |n: f32| ((((h + n).rem_euclid(1.0)) * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
    [k(0.0), k(2.0 / 3.0), k(1.0 / 3.0)]
}

/// How long a burst of fireworks lasts (seconds).
const BURST: f32 = 2.4;

/// Fireworks over the street at time `t`: four launchers, each bursting every few seconds
/// somewhere new in the sky and in a colour of its own, as (along the sky, up it, age 0 .. 1,
/// hue); and the light that comes in at the window as each opens (none with reduced flashes).
fn fireworks(t: f32, quiet: bool) -> ([[f32; 4]; 4], [f32; 3]) {
    const EVERY: [f32; 4] = [5.3, 6.1, 4.7, 7.0];
    const AT: [f32; 4] = [0.9, 2.9, 4.1, 5.6];
    let unit = |round: i32, k: i32| {
        let mut h = (round as u32).wrapping_mul(0x9e37_79b9) ^ (k as u32).wrapping_mul(0x85eb_ca6b);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        (h & 0xffff) as f32 / 65535.0
    };
    let mut bursts = [[0.0, 0.0, -1.0, 0.0]; 4];
    let mut flash = [0.0f32; 3];
    for i in 0..4 {
        let local = t + AT[i];
        let phase = local.rem_euclid(EVERY[i]);
        if phase >= BURST {
            continue;
        }
        let round = (local / EVERY[i]).floor() as i32 * 4 + i as i32;
        let age = phase / BURST;
        // (Along the sky from the window's front to well behind it, and well above the street's roofs.)
        let (z, y, colour) = (-3.3 + 3.6 * unit(round, 1), 2.5 + 1.4 * unit(round, 2), unit(round, 3));
        bursts[i] = [z, y, age, colour];
        // The window lights up quickly and goes out slowly.
        let k = (1.0 - age).powi(3) * (age / 0.05).min(1.0) * 0.7;
        for (f, c) in flash.iter_mut().zip(hue(colour)) {
            *f += c * k;
        }
    }
    // (Bursts that open together never light the room past a gentle peak.)
    for f in &mut flash {
        *f = f.min(0.8);
    }
    if quiet {
        flash = [0.0; 3];
    }
    (bursts, flash)
}

fn camera_override() -> Option<[f32; 7]> {
    let v: Vec<f32> = std::env::var("TA_ROOM_CAMERA").ok()?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    v.try_into().ok()
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The room: its model, its pictures painted from the install's art, and where the camera
/// and the tube are.
fn build(vfs: &ta_vfs::Vfs, data: &ta_data::GameData, season: Season) -> anyhow::Result<(RoomScene, View)> {
    let model = RoomModel::from_glb(ta_render::room::room_model(), season)?;
    let art = art::Art::load(vfs, data);
    let keys = model.key_legends();
    // (Painted side by side, a few to a thread.)
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let chunk = model.pictures.len().div_ceil(threads).max(1);
    let layers: Vec<Vec<u8>> = std::thread::scope(|scope| {
        let workers: Vec<_> = model.pictures.chunks(chunk).map(|pictures| scope.spawn(|| pictures.iter().map(|p| paint(&art, &p.name, p.aspect, &keys, season)).collect::<Vec<_>>())).collect();
        let mut layers = Vec::new();
        for w in workers {
            layers.extend(w.join().map_err(|_| anyhow::anyhow!("painting the room's pictures failed"))?);
        }
        anyhow::Ok(layers)
    })?;
    // `TA_ROOM_PICTURES=<dir>` keeps them, raw RGBA, for a look.
    if let Ok(dir) = std::env::var("TA_ROOM_PICTURES") {
        for (p, layer) in model.pictures.iter().zip(&layers) {
            let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{}.rgba", p.name)), layer);
        }
    }
    let screen = model.screen().ok_or_else(|| anyhow::anyhow!("the room has no tube"))?;
    let eye = model.point("eye").unwrap_or(Vec3::new(0.0, 1.2, 0.86));
    let target = model.point("target").unwrap_or(screen.center);
    let view = View { eye, target, screen: screen.center, facing: screen.normal(), screen_half: screen.up.length(), bulge: screen.bulge };
    Ok((RoomScene { model, layers }, view))
}

/// A picture of the room by its name, `aspect` wide for its height, as a layer.
fn paint(art: &art::Art, name: &str, aspect: f32, keys: &[ta_render::room::KeyLegend], season: Season) -> Vec<u8> {
    let size = LAYER_SIZE as usize;
    // At its own shape, its longer side the layer's; stretched to fill it.
    let (w, h) = if aspect >= 1.0 { (size, (size as f32 / aspect).round() as usize) } else { ((size as f32 * aspect).round() as usize, size) };
    let (w, h) = (w.max(16), h.max(16));
    let canvas = match name {
        "poster_main" => art::poster_main(art, w, h),
        "poster_guide" => art::poster_guide(art, w, h),
        "poster_jungle" => art::poster_jungle(art, w, h),
        "poster_space" => art::poster_space(art, w, h),
        "poster_outrun" => art::poster_outrun(art, w, h),
        "poster_band" => art::poster_band(art, w, h),
        "mouse_mat" => art::mouse_mat(art, w, h),
        "turbo" => art::turbo_digits(w, h),
        "clock" => art::clock_digits(w, h, match season {
            Season::Normal => [None, Some(1), Some(4), Some(5)],
            // Midnight at the turn of Christmas Eve, a minute to the new year.
            Season::Christmas => [Some(1), Some(2), Some(0), Some(0)],
            Season::NewYear => [Some(1), Some(1), Some(5), Some(9)],
        }),
        "calendar" => art::calendar(art, w, h, match season {
            Season::Normal => &art::SEPTEMBER_1997,
            Season::Christmas => &art::DECEMBER_1997,
            Season::NewYear => &art::JANUARY_1998,
        }),
        "banner" => art::banner(art, w, h),
        "manual" => art::manual(art, w, h),
        "cd" => art::cd_insert(art, w, h),
        "note" => art::sticky_note(art, w, h),
        "plan" => art::plan(art, w, h),
        "badge" => art::badge(art, w, h),
        "keys" => art::key_legends(art, keys, size),
        other => {
            if let Some(i) = other.strip_prefix("box").and_then(|i| i.parse().ok()) {
                art::box_front(art, i, w, h)
            } else if let Some(i) = other.strip_prefix("spine").and_then(|i| i.parse().ok()) {
                art::box_spine(art, i, w, h)
            } else {
                art::Canvas::new(w, h, art::rgb(128, 128, 128))
            }
        }
    };
    canvas.into_square_layer(size)
}

/// The mean colour (linear) of each ninth of a picture, row by row from the top left.
fn regional_colours(w: usize, h: usize, rgba: &[u8]) -> [[f32; 3]; 9] {
    let lin = |c: u8| (c as f32 / 255.0).powf(2.2);
    let mut out = [[0.0f32; 3]; 9];
    if w < 3 || h < 3 || rgba.len() < w * h * 4 {
        return out;
    }
    for (i, o) in out.iter_mut().enumerate() {
        let (cx, cy) = (i % 3, i / 3);
        let (x0, x1) = (cx * w / 3, (cx + 1) * w / 3);
        let (y0, y1) = (cy * h / 3, (cy + 1) * h / 3);
        let mut n = 0.0f32;
        for y in (y0..y1).step_by(6) {
            for x in (x0..x1).step_by(6) {
                let p = (y * w + x) * 4;
                for k in 0..3 {
                    o[k] += lin(rgba[p + k]);
                }
                n += 1.0;
            }
        }
        for c in o.iter_mut() {
            *c /= n.max(1.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regional_colours_are_the_ninths_of_the_picture() {
        // Left third red, the rest black.
        let (w, h) = (30usize, 30usize);
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..10 {
                rgba[(y * w + x) * 4] = 255;
            }
        }
        let g = regional_colours(w, h, &rgba);
        for row in 0..3 {
            assert!(g[row * 3][0] > 0.9, "{:?}", g[row * 3]);
            assert!(g[row * 3 + 1][0] < 0.01 && g[row * 3 + 2][0] < 0.01);
        }
    }

    #[test]
    fn every_picture_of_the_room_is_painted() {
        // (Without the install's art: the pictures come out plain, but every one is a layer.)
        for season in [Season::Normal, Season::Christmas, Season::NewYear] {
            let model = RoomModel::from_glb(ta_render::room::room_model(), season).unwrap();
            let art = art::Art::empty();
            let keys = model.key_legends();
            for p in &model.pictures {
                let layer = paint(&art, &p.name, p.aspect, &keys, season);
                assert_eq!(layer.len(), (LAYER_SIZE * LAYER_SIZE * 4) as usize, "{} at {:?}", p.name, season);
            }
        }
    }

    #[test]
    fn the_room_follows_the_calendar_as_the_launchers_season_does() {
        use ta_config::SeasonalTheme;
        let settings = ta_config::Launcher::default();
        for (month, day, season) in [
            (12, 1, Season::Christmas),
            (12, 24, Season::Christmas),
            (12, 30, Season::Christmas),
            (12, 31, Season::NewYear),
            (1, 1, Season::NewYear),
            // The launcher's Christmas runs to 6 January; the room's is December's.
            (1, 2, Season::Normal),
            (1, 3, Season::Normal),
            (1, 6, Season::Normal),
            (6, 15, Season::Normal),
            (3, 8, Season::Normal),
            (10, 31, Season::Normal),
            (11, 1, Season::Normal),
        ] {
            assert_eq!(season_on(&settings, month, day), season, "{day}/{month}");
        }
        // The seasonal effects off: the ordinary room, whatever the day.
        let off = ta_config::Launcher { seasonal_effects: false, ..ta_config::Launcher::default() };
        assert_eq!(season_on(&off, 12, 25), Season::Normal);
        assert_eq!(season_on(&off, 1, 1), Season::Normal);
        // A theme the player chose holds on any day; Halloween has no room of its own.
        for (theme, season) in [(SeasonalTheme::Christmas, Season::Christmas), (SeasonalTheme::NewYear, Season::NewYear), (SeasonalTheme::Halloween, Season::Normal)] {
            let chosen = ta_config::Launcher { seasonal_theme: theme, ..ta_config::Launcher::default() };
            for (month, day) in [(7, 14), (1, 3), (12, 31)] {
                assert_eq!(season_on(&chosen, month, day), season, "{theme:?} on {day}/{month}");
            }
        }
    }

    #[test]
    fn fireworks_burst_and_light_the_window_gently() {
        let (mut seen, mut peak) = (false, 0.0f32);
        let mut t = 0.0;
        while t < 90.0 {
            let (bursts, flash) = fireworks(t, false);
            for b in bursts {
                assert!(b[2] < 1.0 && b[3] >= 0.0 && b[3] <= 1.0);
                seen |= b[2] >= 0.0;
            }
            for f in flash {
                assert!((0.0..=1.0).contains(&f), "{flash:?}");
                peak = peak.max(f);
            }
            assert_eq!(fireworks(t, true).1, [0.0; 3]);
            t += 0.05;
        }
        assert!(seen && peak > 0.2, "{seen} {peak}");
    }
}
