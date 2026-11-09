//! The benchmark (`--benchmark <file>`, which the launcher starts; `--benchmark-mode quick` for
//! the quick run): staged scenes on TA Reborn's own worlds (`benchmark/*.toml`, written by
//! `tools/benchmark/scenes.py`) at the player's settings, the frame rate unlimited, each showing
//! off what the game draws there. After the storm (`highland`) come two tests from a copy of its
//! battle at [`TEST_FROM`]: the settings test, those seconds again at each graphics level and then
//! at High with about 1080p's pixels drawn for the graphics card's score (all with the
//! antialiasing the game started with: it changes only then); and the effects test, at High
//! with one effect turned off at a time (what each costs). A quick run plays the storm to the
//! copy, the settings test and the twenty thousand: the scores' own measurements.
//!
//! Every frame is measured (its time, the processor's work, the graphics card's and each of its
//! passes', the simulation's); each scene's picture is taken (that frame not measured) and kept
//! beside the report; at the end the report (`ta_config::benchmark::Report`) goes to the file
//! and the game closes. The quit key stops it, with no report.
//!
//! The scenes play the same battle on every computer (the simulation is deterministic, and a
//! scene's orders and camera keep to its own clock): what differs is how fast it is drawn.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use ta_config::Preset;
use ta_config::benchmark::{self as report, Effect, EffectRun, Frame, LEVELS, LevelRun, Memory, Mode, Part, PassTime, Report, Stats};
use ta_ui::i18n::n;

use crate::stage::{Stage, StageRun};

/// A scene of the benchmark: its id (its part's in the report, and its caption's in
/// `ta_ui::benchmark`), its file, how long it plays (seconds of its own clock), and when its
/// picture is taken.
pub struct SceneDef {
    pub id: &'static str,
    text: &'static str,
    pub seconds: f32,
    picture_at: f32,
}

impl SceneDef {
    /// What its caption says.
    pub fn info(&self) -> &'static ta_ui::benchmark::SceneInfo {
        ta_ui::benchmark::scene(self.id, false).expect("every benchmark scene has its caption")
    }
}

/// The scenes, in the order a full run plays them.
pub const SCENES: [SceneDef; 8] = [
    SceneDef { id: "sapphire", text: include_str!("../benchmark/sapphire.toml"), seconds: 28.0, picture_at: 3.0 },
    SceneDef { id: "cockpit", text: include_str!("../benchmark/cockpit.toml"), seconds: 25.0, picture_at: 12.5 },
    SceneDef { id: "highland", text: include_str!("../benchmark/highland.toml"), seconds: 30.0, picture_at: 21.0 },
    SceneDef { id: "space", text: include_str!("../benchmark/space.toml"), seconds: 25.0, picture_at: 11.0 },
    SceneDef { id: "build", text: include_str!("../benchmark/build.toml"), seconds: 25.0, picture_at: 6.0 },
    SceneDef { id: "swarm", text: include_str!("../benchmark/swarm.toml"), seconds: 30.0, picture_at: 8.5 },
    SceneDef { id: "cinder", text: include_str!("../benchmark/cinder.toml"), seconds: 25.0, picture_at: 20.3 },
    SceneDef { id: "winter", text: include_str!("../benchmark/winter.toml"), seconds: 25.0, picture_at: 19.0 },
];

/// A scene's first second is not measured (its pictures and the drawing's pipelines settling).
const SETTLE: f32 = 1.0;
/// The tests: from this second of the storm each pass (the levels, the reference pass, the
/// effects) settles, then is measured.
pub const TEST_FROM: f32 = 16.0;
const TEST_SETTLE: f32 = 1.5;
const TEST_MEASURE: f32 = 5.0;
const EFFECT_SETTLE: f32 = 0.8;
const EFFECT_MEASURE: f32 = 2.4;
/// A scene whose clock falls far behind (the simulation slower than its battle) ends after this
/// many times its length of real time, measured as far as it got.
const WALL_FACTOR: f32 = 3.0;
/// A scene's caption: when it comes in, how long it stays.
const CAPTION_IN: f32 = 0.4;
const CAPTION_FOR: f32 = 4.5;
/// The telemetry's graph: this many seconds of frames.
const GRAPH_SECONDS: f32 = 6.0;
/// A scene's picture: this wide (px), as a JPEG of this quality.
const PICTURE_WIDTH: u32 = 640;
const PICTURE_QUALITY: u8 = 85;

static RUNNING: AtomicBool = AtomicBool::new(false);
static ICONS: AtomicBool = AtomicBool::new(false);

/// `TA_BENCHMARK_SHOTS=1` (QA): each scene's picture taken as its caption shows, the
/// benchmark's own panels in it.
fn qa_shots() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("TA_BENCHMARK_SHOTS").is_ok_and(|v| v == "1"))
}

/// Whether a benchmark is running (the battle's interface stays hidden).
pub fn running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// Whether the benchmark's scene playing shows strategic icons when far out (its `icons`); none
/// while no benchmark runs.
pub fn scene_icons() -> Option<bool> {
    running().then(|| ICONS.load(Ordering::Relaxed))
}

/// The battle copied as the tests start each pass from it.
pub struct Copy {
    pub sim: ta_sim::Sim,
    pub ais: Vec<ta_ai::SkirmishAi>,
    pub run: StageRun,
}

/// What the game does next for the benchmark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Next {
    On,
    /// The battle copied now, for the tests ([`Benchmark::keep`]).
    Copy,
    /// Scene `n` loaded (after the tests, the player's settings back first); then
    /// [`Benchmark::loaded`].
    Load(usize),
    /// The settings test: this level's settings, and the copied battle again.
    Level(Preset),
    /// The settings test's reference pass: High, about 1080p's pixels drawn
    /// (`ta_config::benchmark::reference_scale`), and the copied battle again.
    Reference,
    /// The effects test: High with this effect off ([`Effect::apply`]), and the copied battle
    /// again.
    Effect(Effect),
    /// The scene's picture: the next frame read back (and not measured), then
    /// [`Benchmark::picture`].
    Picture,
    /// The report is due ([`Benchmark::report`]): the game writes it and closes.
    Finish,
}

/// A part of the run.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    /// A scene played to `until` (seconds of its clock); `copy`: its battle copied at
    /// [`TEST_FROM`] for the tests.
    Scene { scene: usize, until: f32, copy: bool },
    Settings,
    Effects,
}

impl Step {
    fn id(self) -> &'static str {
        match self {
            Step::Scene { scene, .. } => SCENES[scene].id,
            Step::Settings => "settings",
            Step::Effects => "effects",
        }
    }
}

fn scene_index(id: &str) -> usize {
    SCENES.iter().position(|s| s.id == id).expect("a benchmark scene")
}

/// A scene's stage as its file has it (nothing left out).
#[cfg(test)]
pub(crate) fn scene_stage(scene: &SceneDef) -> Stage {
    Stage::parse(scene.text).expect("a benchmark scene")
}

/// The parts of a run in `mode` (as `ta_config::benchmark::Mode::parts` names them).
fn steps(mode: Mode) -> Vec<Step> {
    mode.parts()
        .iter()
        .enumerate()
        .map(|(i, &id)| match id {
            "settings" => Step::Settings,
            "effects" => Step::Effects,
            id => {
                let scene = scene_index(id);
                // (The storm copied for the tests; a quick run goes to them from the copy.)
                let copy = mode.parts().get(i + 1) == Some(&"settings");
                let until = if copy && mode == Mode::Quick { TEST_FROM } else { SCENES[scene].seconds };
                Step::Scene { scene, until, copy }
            }
        })
        .collect()
}

/// Sums of each pass's time while a part is measured.
#[derive(Default)]
struct PassSums(Vec<(&'static str, f64, u32)>);

impl PassSums {
    fn add(&mut self, passes: &[ta_render::PassTime]) {
        // (Not the frame's own totals.)
        for p in passes.iter().filter(|p| !matches!(p.pass, "frame" | "full")) {
            match self.0.iter_mut().find(|s| s.0 == p.pass) {
                Some(s) => {
                    s.1 += p.ms as f64;
                    s.2 += 1;
                }
                None => self.0.push((p.pass, p.ms as f64, 1)),
            }
        }
    }

    fn averages(&self) -> Vec<PassTime> {
        self.0.iter().map(|&(pass, total, n)| PassTime { pass: pass.to_string(), ms: (total / n.max(1) as f64) as f32 }).collect()
    }
}

pub struct Benchmark {
    /// Where the report goes.
    pub out: PathBuf,
    pub mode: Mode,
    scenes: Vec<Stage>,
    steps: Vec<Step>,
    /// The part playing (an index into `steps`).
    step: usize,
    /// A scene is loading: its frames are not measured.
    loading: bool,
    /// When the scene playing began (real time), for [`WALL_FACTOR`].
    began: Option<Instant>,
    /// The run's clock: the measured frames' time added up (the loading left out).
    clock: f32,
    frames: Vec<(u8, Frame)>,
    passes: Vec<PassSums>,
    copy: Option<Copy>,
    /// Whether it was taken, and the scene's second it was taken at.
    copied: bool,
    copy_at: f32,
    /// The test playing: its pass, its frames, and what the passes came to.
    pass: usize,
    pass_frames: Vec<Frame>,
    levels: Vec<LevelRun>,
    reference: Stats,
    effects: Vec<EffectRun>,
    /// The effects the effects test turns off (ray tracing only where it is on).
    effect_list: Vec<Effect>,
    /// The graphics card's newest timing taken (its frame).
    gpu_frame: u64,
    /// Each part's picture: asked for (the next frame read back, not measured), and its file.
    picture_asked: bool,
    picture_taken: bool,
    pictures: Vec<Option<String>>,
    /// Unit types of the expansions not installed, left out of the scenes.
    pub left_out: Vec<String>,
    memory: Memory,
    /// The player's settings, while the tests play.
    pub saved: Option<ta_config::Graphics>,
    /// The last frames (for the telemetry): seconds of the run's real time, and the frame.
    recent: VecDeque<(f32, Frame)>,
    started: Instant,
}

impl Benchmark {
    /// A run of `mode` whose report goes to `out`: its scenes read (and what is not installed of
    /// them left out); `ray_tracing`: whether the renderer traces rays (the effects test turns it
    /// off then).
    pub fn new(out: PathBuf, mode: Mode, ray_tracing: bool, installed: impl Fn(&str) -> bool) -> anyhow::Result<Benchmark> {
        let mut scenes = Vec::new();
        let mut left_out = Vec::new();
        for s in &SCENES {
            let mut stage = Stage::parse(s.text).map_err(|e| anyhow::anyhow!("benchmark scene {}: {e}", s.id))?;
            left_out.extend(stage.prune_missing(&installed));
            scenes.push(stage);
        }
        left_out.sort();
        left_out.dedup();
        if !left_out.is_empty() {
            log::warn!("benchmark: not installed, left out of the scenes: {}", left_out.join(", "));
        }
        let steps = steps(mode);
        let effect_list = Effect::ALL.into_iter().filter(|&e| e != Effect::RayTracing || ray_tracing).collect();
        RUNNING.store(true, Ordering::Relaxed);
        let mut b = Benchmark {
            out,
            mode,
            scenes,
            passes: steps.iter().map(|_| PassSums::default()).collect(),
            pictures: vec![None; steps.len()],
            steps,
            step: 0,
            loading: false,
            began: None,
            clock: 0.0,
            frames: Vec::new(),
            copy: None,
            copied: false,
            copy_at: TEST_FROM,
            pass: 0,
            pass_frames: Vec::new(),
            levels: Vec::new(),
            reference: Stats::default(),
            effects: Vec::new(),
            effect_list,
            gpu_frame: 0,
            picture_asked: false,
            picture_taken: false,
            left_out,
            memory: Memory::default(),
            saved: None,
            recent: VecDeque::new(),
            started: Instant::now(),
        };
        b.scene_shown();
        Ok(b)
    }

    /// The memory measured (before the first scene: about two thirds of a second).
    pub fn measure_memory(&mut self) {
        self.memory.bandwidth_gbs = report::memory_bandwidth();
        self.memory.latency_ns = report::memory_latency_ns();
        log::info!("benchmark: memory copies at {:.1} GB/s, reads at random in {:.0} ns; report to {}", self.memory.bandwidth_gbs, self.memory.latency_ns, self.out.display());
    }

    /// The scene of the part playing, or of the tests (the storm's).
    fn scene_index(&self) -> usize {
        let step = self.step.min(self.steps.len() - 1);
        self.steps[..=step].iter().rev().find_map(|s| match s {
            Step::Scene { scene, .. } => Some(*scene),
            _ => None,
        }).unwrap_or(0)
    }

    /// The scene playing (or loading).
    pub fn stage(&self) -> &Stage {
        &self.scenes[self.scene_index()]
    }

    /// Scene `n`'s stage.
    pub fn scene(&self, n: usize) -> &Stage {
        &self.scenes[n]
    }

    /// The icons the scene playing shows (see [`scene_icons`]).
    fn scene_shown(&mut self) {
        ICONS.store(self.stage().icons, Ordering::Relaxed);
    }

    /// The loading is over: frames are measured again.
    pub fn loaded(&mut self) {
        self.loading = false;
        self.began = None;
        self.scene_shown();
    }

    /// The copy the tests start from, taken.
    pub fn keep(&mut self, copy: Copy, seconds: f32) {
        self.copy = Some(copy);
        self.copied = true;
        self.copy_at = seconds;
    }

    pub fn copy(&self) -> Option<&Copy> {
        self.copy.as_ref()
    }

    /// The graphics card's time for a frame, when its newest timing (of `frame`) is new.
    pub fn gpu_sample(&mut self, frame: u64, ms: Option<f32>) -> Option<f32> {
        if frame == self.gpu_frame {
            return None;
        }
        self.gpu_frame = frame;
        ms
    }

    /// The scene's picture, read back: kept (as a JPEG beside the report, written on a thread of
    /// its own).
    pub fn picture(&mut self, shot: ta_render::Screenshot) {
        let Some(Step::Scene { scene, .. }) = self.steps.get(self.step).copied() else { return };
        let name = format!("{}.jpg", SCENES[scene].id);
        let dir = report::pictures_dir(&self.out);
        let path = dir.join(&name);
        self.pictures[self.step] = Some(name);
        // (QA's whole.)
        let width = if qa_shots() { shot.width } else { PICTURE_WIDTH };
        std::thread::spawn(move || {
            let Some(image) = image::RgbaImage::from_raw(shot.width, shot.height, shot.rgba) else { return };
            let height = (width as f32 * shot.height as f32 / shot.width.max(1) as f32).round().max(1.0) as u32;
            let small = image::imageops::resize(&image::DynamicImage::ImageRgba8(image).to_rgb8(), width, height, image::imageops::FilterType::Triangle);
            let saved = std::fs::create_dir_all(&dir).map_err(anyhow::Error::from).and_then(|()| {
                let file = std::io::BufWriter::new(std::fs::File::create(&path)?);
                image::codecs::jpeg::JpegEncoder::new_with_quality(file, PICTURE_QUALITY).encode_image(&small)?;
                Ok(())
            });
            if let Err(e) = saved {
                log::warn!("benchmark: could not keep the picture {}: {e}", path.display());
            }
        });
    }

    /// A frame done (`seconds`: its scene's clock, none while there is no scene; `passes`: the
    /// graphics card's newest timing of each pass, when it is new): measured, and what comes next.
    pub fn frame(&mut self, f: Frame, passes: Option<&[ta_render::PassTime]>, seconds: Option<f32>) -> Next {
        let now = self.started.elapsed().as_secs_f32();
        self.recent.push_back((now, f));
        while self.recent.front().is_some_and(|(t, _)| now - t > GRAPH_SECONDS) {
            self.recent.pop_front();
        }
        let Some(seconds) = seconds.filter(|_| !self.loading) else { return Next::On };
        // (The frame that read the picture back is not measured.)
        if std::mem::take(&mut self.picture_asked) {
            return Next::On;
        }
        match self.steps[self.step] {
            Step::Scene { scene, until, copy } => {
                if (SETTLE..until).contains(&seconds) {
                    self.record(f, passes);
                }
                if copy && !self.copied && seconds >= TEST_FROM {
                    return Next::Copy;
                }
                let picture_at = if qa_shots() { CAPTION_IN + 2.0 } else { SCENES[scene].picture_at.min(until - 1.0) };
                if !self.picture_taken && seconds >= picture_at {
                    self.picture_taken = true;
                    self.picture_asked = true;
                    return Next::Picture;
                }
                let began = *self.began.get_or_insert_with(Instant::now);
                let late = began.elapsed().as_secs_f32() > SCENES[scene].seconds * WALL_FACTOR;
                if late {
                    log::warn!("benchmark: {} ended at {seconds:.1} s of {until} (its battle runs slower than its clock)", SCENES[scene].id);
                }
                if seconds < until && !late {
                    return Next::On;
                }
                self.advance()
            }
            Step::Settings => {
                let from = self.copy_at + TEST_SETTLE;
                if (from..from + TEST_MEASURE).contains(&seconds) {
                    let f = self.record(f, passes);
                    self.pass_frames.push(f);
                }
                if seconds < from + TEST_MEASURE {
                    return Next::On;
                }
                let stats = report::summarize(&std::mem::take(&mut self.pass_frames));
                match LEVELS.get(self.pass) {
                    Some(&level) => {
                        log::info!("benchmark: settings test at {level:?}: {:.0} fps", stats.fps);
                        self.levels.push(LevelRun { level, stats });
                    }
                    None => {
                        log::info!("benchmark: reference pass: {:.0} fps, the graphics card {:.2} ms a frame", stats.fps, stats.gpu_ms);
                        self.reference = stats;
                    }
                }
                self.pass += 1;
                match LEVELS.get(self.pass) {
                    Some(&level) => Next::Level(level),
                    None if self.pass == LEVELS.len() => Next::Reference,
                    None => self.advance(),
                }
            }
            Step::Effects => {
                let from = self.copy_at + EFFECT_SETTLE;
                if (from..from + EFFECT_MEASURE).contains(&seconds) {
                    let f = self.record(f, passes);
                    self.pass_frames.push(f);
                }
                if seconds < from + EFFECT_MEASURE {
                    return Next::On;
                }
                let stats = report::summarize(&std::mem::take(&mut self.pass_frames));
                let effect = self.effect_list[self.pass];
                log::info!("benchmark: effects test, {effect:?}: the graphics card {:.2} ms a frame", stats.gpu_ms);
                self.effects.push(EffectRun { effect, stats });
                self.pass += 1;
                match self.effect_list.get(self.pass) {
                    Some(&e) => Next::Effect(e),
                    None => self.advance(),
                }
            }
        }
    }

    /// On to the next part: a scene loaded, a test begun, or the report.
    fn advance(&mut self) -> Next {
        self.step += 1;
        self.pass = 0;
        self.picture_taken = false;
        match self.steps.get(self.step) {
            None => Next::Finish,
            Some(Step::Scene { scene, .. }) => {
                self.loading = true;
                self.began = None;
                Next::Load(*scene)
            }
            Some(Step::Settings) => Next::Level(LEVELS[0]),
            Some(Step::Effects) => Next::Effect(self.effect_list[0]),
        }
    }

    /// The frame put on the run's clock and kept (and its passes' times with its part's).
    fn record(&mut self, mut f: Frame, passes: Option<&[ta_render::PassTime]>) -> Frame {
        f.t = self.clock;
        self.clock += f.ms / 1000.0;
        self.frames.push((self.step as u8, f));
        if let Some(p) = passes {
            self.passes[self.step].add(p);
        }
        f
    }

    /// The report: the parts, the run's timeline, the tests, the memory.
    pub fn report(&self, system: report::System, graphics: &ta_config::Graphics, unit_skin: &str, enemy_unit_skin: &str, gameplay: &ta_config::Gameplay, size: (u32, u32), refresh_hz: f32, tested_msaa: u32) -> Report {
        let parts = self
            .steps
            .iter()
            .enumerate()
            .map(|(i, step)| {
                let frames: Vec<Frame> = self.frames.iter().filter(|(p, _)| *p as usize == i).map(|&(_, f)| f).collect();
                Part {
                    id: step.id().to_string(),
                    start: frames.first().map_or(0.0, |f| f.t),
                    seconds: frames.iter().map(|f| f.ms).sum::<f32>() / 1000.0,
                    stats: report::summarize(&frames),
                    passes: self.passes[i].averages(),
                    picture: self.pictures[i].clone(),
                }
            })
            .collect();
        let scenes = |p: u8| matches!(self.steps.get(p as usize), Some(Step::Scene { .. }));
        Report {
            format: report::FORMAT,
            content: report::CONTENT,
            mode: self.mode,
            version: env!("CARGO_PKG_VERSION").to_string(),
            when: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            system,
            graphics: graphics.clone(),
            rendered_unit_skin: Some(unit_skin.to_string()),
            rendered_enemy_unit_skin: Some(enemy_unit_skin.to_string()),
            reduced_motion: gameplay.reduced_motion,
            reduced_flashes: gameplay.reduced_flashes,
            width: size.0,
            height: size.1,
            refresh_hz,
            parts,
            timeline: report::timeline(&self.frames),
            levels: self.levels.clone(),
            effects: self.effects.clone(),
            histogram: report::histogram(self.frames.iter().filter(|(p, _)| scenes(*p)).map(|(_, f)| f.ms)),
            all: report::summarize(&self.frames.iter().filter(|(p, _)| scenes(*p)).map(|&(_, f)| f).collect::<Vec<_>>()),
            reference: self.reference,
            reference_scale: report::reference_scale(size.0, size.1),
            tested_msaa,
            memory: Memory { peak_mb: ta_config::hardware::process_memory_mb().map_or(0, |(_, peak)| peak), ..self.memory },
            gpu_timed: self.frames.iter().any(|(_, f)| f.gpu_ms.is_some()),
            left_out: self.left_out.clone(),
        }
    }

    /// How long each part plays (seconds of the clock it is measured on).
    fn lengths(&self) -> Vec<f32> {
        self.steps
            .iter()
            .map(|s| match s {
                Step::Scene { until, .. } => *until,
                Step::Settings => (LEVELS.len() + 1) as f32 * (TEST_SETTLE + TEST_MEASURE),
                Step::Effects => self.effect_list.len() as f32 * (EFFECT_SETTLE + EFFECT_MEASURE),
            })
            .collect()
    }

    /// How far the part playing has got (0 to 1), `seconds` into it.
    fn part_progress(&self, seconds: f32) -> f32 {
        let lengths = self.lengths();
        let length = lengths[self.step.min(lengths.len() - 1)].max(0.01);
        let here = match self.steps.get(self.step) {
            Some(Step::Scene { .. }) => seconds,
            Some(Step::Settings) => self.pass as f32 * (TEST_SETTLE + TEST_MEASURE) + (seconds - self.copy_at).clamp(0.0, TEST_SETTLE + TEST_MEASURE),
            Some(Step::Effects) => self.pass as f32 * (EFFECT_SETTLE + EFFECT_MEASURE) + (seconds - self.copy_at).clamp(0.0, EFFECT_SETTLE + EFFECT_MEASURE),
            None => length,
        };
        (here / length).clamp(0.0, 1.0)
    }

    /// The panels over the run (none on a frame whose picture is taken): the plate in the
    /// corner (the part, the run's way through its parts, how to stop it: `stop`, the quit key's
    /// name), a scene's caption as it begins, and the telemetry (the frame rate, the frames'
    /// times, the processor's, the graphics card's and the simulation's work, the units).
    pub fn overlay(&self, ctx: &egui::Context, seconds: Option<f32>, fps: f32, stop: &str) {
        if self.picture_asked && !qa_shots() {
            return;
        }
        self.plate(ctx, seconds, stop);
        if let (Some(Step::Scene { scene, .. }), Some(s), false) = (self.steps.get(self.step), seconds, self.loading) {
            self.caption(ctx, SCENES[*scene].info(), s);
        }
        self.telemetry(ctx, fps);
    }

    fn plate(&self, ctx: &egui::Context, seconds: Option<f32>, stop: &str) {
        use crate::theme::*;
        use ta_ui::i18n::{t, tf};
        let step = self.steps.get(self.step.min(self.steps.len() - 1)).copied().unwrap_or(Step::Settings);
        let (title, subtitle) = match step {
            Step::Scene { scene, .. } => (SCENES[scene].info().title.to_string(), t(SCENES[scene].info().subtitle).to_string()),
            Step::Settings => {
                let pass = self.pass.min(LEVELS.len());
                let name = LEVELS.get(pass).map_or_else(|| t("Graphics card"), |l| t(l.label()));
                (t("SETTINGS TEST").to_string(), tf("{level} · {n} of {count}", &[("level", &name), ("n", &(pass + 1)), ("count", &(LEVELS.len() + 1))]))
            }
            Step::Effects => {
                let effect = self.effect_list[self.pass.min(self.effect_list.len() - 1)];
                let what = if effect == Effect::High { t("High, everything on").to_string() } else { tf("High, {effect} off", &[("effect", &t(ta_ui::benchmark::effect_label(effect)))]) };
                (t("EFFECTS TEST").to_string(), tf("{what} · {n} of {count}", &[("what", &what), ("n", &(self.pass + 1)), ("count", &self.effect_list.len())]))
            }
        };
        let progress = seconds.map_or(0.0, |s| self.part_progress(s));
        let mode = match self.mode {
            Mode::Full => n("BENCHMARK"),
            Mode::Quick => n("QUICK BENCHMARK"),
        };
        egui::Area::new(egui::Id::new("benchmark_plate")).anchor(egui::Align2::LEFT_TOP, egui::vec2(20.0, 20.0)).interactable(false).show(ctx, |ui| {
            let background = ui.painter().add(egui::Shape::Noop);
            let shown = egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 18, top: 10, bottom: 16 }).show(ui, |ui| {
                ui.set_min_width(320.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (lamp, _) = ui.allocate_exact_size(egui::vec2(10.0, 16.0), egui::Sense::hover());
                    ui.painter().add(ta_ui::metal::lamp(lamp.center(), 3.5, AMBER, 1.0));
                    ui.label(ta_ui::widgets::caption(mode).color(AMBER_BRIGHT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let n = self.step.min(self.steps.len() - 1) + 1;
                        ui.label(egui::RichText::new(tf("{n} / {count}", &[("n", &n), ("count", &self.steps.len())])).font(strong(12.0)).color(STEEL_DIM));
                    });
                });
                ui.add_space(2.0);
                ui.label(egui::RichText::new(title).font(display(22.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
                ui.label(egui::RichText::new(subtitle).font(body(14.0)).color(TEXT));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tf("{key} stops it", &[("key", &stop)])).font(body(12.0)).color(STEEL_DIM));
            });
            let r = shown.response.rect;
            // The run's parts along the foot: those done lit, the one playing filling.
            let foot = egui::Rect::from_min_max(egui::pos2(r.left() + 6.0, r.bottom() - 7.0), egui::pos2(r.right() - 6.0, r.bottom() - 4.0));
            let lengths = self.lengths();
            let total: f32 = lengths.iter().sum::<f32>().max(0.01);
            let mut shapes = vec![
                egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: egui::Color32::from_black_alpha(140) }.as_shape(r, 3.0).into(),
                egui::Shape::rect_filled(r.expand(1.0), 3.0, ta_ui::metal::STEEL_EDGE),
                ta_ui::metal::plate(ctx, r, PANEL),
                egui::Shape::hline(r.x_range().shrink(1.0), r.top() + 1.0, egui::Stroke::new(1.0, egui::Color32::from_white_alpha(20))),
            ];
            let mut x = foot.left();
            for (i, length) in lengths.iter().enumerate() {
                let w = foot.width() * length / total;
                let seg = egui::Rect::from_min_max(egui::pos2(x + 1.0, foot.top()), egui::pos2(x + w - 1.0, foot.bottom()));
                shapes.push(egui::Shape::rect_filled(seg, 0.0, egui::Color32::from_black_alpha(130)));
                let done = if i < self.step { 1.0 } else if i == self.step { progress } else { 0.0 };
                if done > 0.0 {
                    shapes.push(egui::Shape::rect_filled(egui::Rect::from_min_size(seg.min, egui::vec2(seg.width() * done, seg.height())), 0.0, if i < self.step { AMBER.gamma_multiply(0.7) } else { AMBER }));
                }
                x += w;
            }
            ui.painter().set(background, egui::Shape::Vec(shapes));
        });
    }

    /// A scene's caption as it begins: the world, the moment, what it shows; in, held, out.
    fn caption(&self, ctx: &egui::Context, scene: &ta_ui::benchmark::SceneInfo, seconds: f32) {
        use crate::theme::*;
        use ta_ui::i18n::t;
        let shown = ((seconds - CAPTION_IN) / 0.5).clamp(0.0, 1.0) * (1.0 - ((seconds - CAPTION_IN - CAPTION_FOR) / 0.8).clamp(0.0, 1.0));
        if shown <= 0.0 {
            return;
        }
        let slide = (1.0 - shown) * 16.0;
        egui::Area::new(egui::Id::new("benchmark_caption")).anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(40.0 - slide, -48.0)).interactable(false).fade_in(false).show(ctx, |ui| {
            ui.set_opacity(shown);
            let rect = ui.max_rect();
            let bar = egui::Rect::from_min_size(rect.min, egui::vec2(4.0, 74.0));
            ui.painter().rect_filled(bar, 1.0, AMBER);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(scene.title).font(display(34.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(2.0)));
                    ui.label(egui::RichText::new(t(scene.subtitle)).font(body(17.0)).color(TEXT));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for f in scene.features {
                            egui::Frame::new().fill(egui::Color32::from_black_alpha(150)).stroke(egui::Stroke::new(1.0, AMBER.gamma_multiply(0.6))).corner_radius(2.0).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
                                ui.label(egui::RichText::new(t(f)).font(strong(12.0)).color(AMBER_BRIGHT));
                            });
                        }
                    });
                });
            });
        });
    }

    /// The telemetry in the other corner: the frame rate; the last seconds' frames as a graph
    /// (60 frames a second marked); the processor's, the graphics card's and the simulation's
    /// work a frame; the units.
    fn telemetry(&self, ctx: &egui::Context, fps: f32) {
        use crate::theme::*;
        use ta_ui::i18n::{t, tf};
        let Some(&(_, last)) = self.recent.back() else { return };
        let n = self.recent.len().max(1) as f32;
        let avg = |f: &dyn Fn(&Frame) -> f32| self.recent.iter().map(|(_, fr)| f(fr)).sum::<f32>() / n;
        let gpu: Vec<f32> = self.recent.iter().filter_map(|(_, f)| f.gpu_ms).collect();
        let gpu_ms = (!gpu.is_empty()).then(|| gpu.iter().sum::<f32>() / gpu.len() as f32);
        let cpu_ms = avg(&|f| f.cpu_ms);
        let ticks: u32 = self.recent.iter().map(|(_, f)| f.ticks).sum();
        let tick_ms = if ticks > 0 { self.recent.iter().map(|(_, f)| f.tick_ms).sum::<f32>() / ticks as f32 } else { 0.0 };
        egui::Area::new(egui::Id::new("benchmark_telemetry")).anchor(egui::Align2::RIGHT_TOP, egui::vec2(-20.0, 20.0)).interactable(false).show(ctx, |ui| {
            let background = ui.painter().add(egui::Shape::Noop);
            let shown = egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 12 }).show(ui, |ui| {
                ui.set_width(250.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{:.0}", fps.max(0.0))).font(display(30.0)).color(TEXT_BRIGHT));
                    ui.vertical(|ui| {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(t("FPS")).font(strong(12.0)).color(AMBER_BRIGHT));
                        ui.label(egui::RichText::new(tf("{ms} ms", &[("ms", &format!("{:.1}", last.ms))])).font(body(12.0)).color(STEEL_DIM));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                        ui.vertical(|ui| {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(format!("{}", last.units)).font(strong(15.0)).color(TEXT_BRIGHT));
                            ui.label(egui::RichText::new(t("UNITS")).font(strong(10.0)).color(STEEL_DIM));
                        });
                    });
                });
                ui.add_space(4.0);
                // The frames' times, the newest on the right; 60 a second marked.
                let (graph, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), egui::Sense::hover());
                let p = ui.painter();
                p.rect_filled(graph, 2.0, egui::Color32::from_black_alpha(110));
                let top_ms = 50.0f32;
                let y = |ms: f32| graph.bottom() - (ms / top_ms).clamp(0.0, 1.0) * graph.height();
                p.hline(graph.x_range(), y(1000.0 / 60.0), egui::Stroke::new(1.0, GREEN.gamma_multiply(0.45)));
                let now = self.recent.back().map_or(0.0, |r| r.0);
                let points: Vec<egui::Pos2> = self.recent.iter().map(|(at, f)| egui::pos2(graph.right() - (now - at) / GRAPH_SECONDS * graph.width(), y(f.ms))).collect();
                if points.len() > 1 {
                    p.add(egui::Shape::line(points, egui::Stroke::new(1.4, AMBER)));
                }
                ui.add_space(6.0);
                // The work a frame: bars against 33 ms.
                let rows = [(t("Processor"), Some(cpu_ms), GREEN), (t("Graphics card"), gpu_ms, SKY), (t("Simulation tick"), Some(tick_ms), CREAM)];
                for (name, ms, colour) in rows {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(name).font(body(12.0)).color(TEXT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(ms.map_or_else(|| "–".to_string(), |ms| tf("{ms} ms", &[("ms", &format!("{ms:.1}"))]))).font(strong(12.0)).color(TEXT_BRIGHT));
                        });
                    });
                    let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.0), egui::Sense::hover());
                    ui.painter().rect_filled(bar, 1.0, egui::Color32::from_black_alpha(120));
                    let share = ms.map_or(0.0, |ms| (ms / 33.3).clamp(0.0, 1.0));
                    ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * share, bar.height())), 1.0, colour);
                }
            });
            let r = shown.response.rect;
            ui.painter().set(
                background,
                egui::Shape::Vec(vec![
                    egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: egui::Color32::from_black_alpha(140) }.as_shape(r, 3.0).into(),
                    egui::Shape::rect_filled(r.expand(1.0), 3.0, ta_ui::metal::STEEL_EDGE),
                    ta_ui::metal::plate(ctx, r, PANEL),
                ]),
            );
        });
    }
}

impl Drop for Benchmark {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ms: f32) -> Frame {
        Frame { ms, cpu_ms: ms / 2.0, gpu_ms: Some(ms / 2.0), ticks: 1, tick_ms: 1.0, units: 10, ..Default::default() }
    }

    /// Plays a run as the game would, frames 10 ms apart on each part's clock, doing what each
    /// `Next` asks (as far as the benchmark sees it): the parts it went through in order.
    fn play(b: &mut Benchmark) -> Vec<Next> {
        let mut seen = Vec::new();
        let mut seconds = 0.0f32;
        for _ in 0..200_000 {
            let next = b.frame(frame(10.0), None, Some(seconds));
            seconds += 0.01;
            match next {
                Next::On | Next::Picture => {}
                Next::Copy => {
                    // (The game copies the battle; a mark is enough here.)
                    b.copied = true;
                    b.copy_at = seconds;
                }
                Next::Load(_) => {
                    b.loaded();
                    seconds = 0.0;
                }
                Next::Level(_) | Next::Reference | Next::Effect(_) => seconds = b.copy_at,
                Next::Finish => {
                    seen.push(next);
                    return seen;
                }
            }
            if next != Next::On {
                seen.push(next);
            }
        }
        panic!("the run never finished: {seen:?}");
    }

    fn report(b: &Benchmark) -> Report {
        let gameplay = ta_config::Gameplay { reduced_motion: true, reduced_flashes: true, ..Default::default() };
        let graphics = ta_config::Graphics { unit_skin: "christmas".into(), enemy_unit_skin: "desert".into(), ..Default::default() };
        b.report(report::System::default(), &graphics, "christmas", "desert", &gameplay, (2560, 1440), 60.0, 4)
    }

    /// The scenes read, and a full run goes through its parts in order: each scene, the copy in
    /// the storm, the settings test's levels and reference pass, the effects test's effects (ray
    /// tracing too, where it is on), the report.
    #[test]
    fn a_full_run_goes_through_its_parts() {
        let mut b = Benchmark::new(std::env::temp_dir().join("bench.json"), Mode::Full, true, |_| true).unwrap();
        assert!(b.left_out.is_empty());
        let seen = play(&mut b);
        let loads: Vec<usize> = seen.iter().filter_map(|n| if let Next::Load(s) = n { Some(*s) } else { None }).collect();
        // (The first scene is the battle the game starts with: no load.)
        assert_eq!(loads, [1, 2, 3, 4, 5, 6, 7]);
        assert!(seen.contains(&Next::Copy) && seen.contains(&Next::Reference));
        let effects: Vec<Effect> = seen.iter().filter_map(|n| if let Next::Effect(e) = n { Some(*e) } else { None }).collect();
        assert_eq!(effects, Effect::ALL);
        let r = report(&b);
        assert_eq!((r.content, r.mode), (report::CONTENT, Mode::Full));
        assert_eq!(r.unit_skin(), "christmas");
        assert_eq!(r.enemy_unit_skin(), "desert");
        assert!(r.reduced_motion && r.reduced_flashes);
        assert_eq!(r.parts.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), report::FULL_PARTS);
        assert_eq!(r.levels.iter().map(|l| l.level).collect::<Vec<_>>(), LEVELS);
        assert_eq!(r.effects.iter().map(|e| e.effect).collect::<Vec<_>>(), Effect::ALL);
        assert!(r.parts.iter().all(|p| p.stats.frames > 0), "{:?}", r.parts);
        assert!((r.parts[0].stats.fps - 100.0).abs() < 0.5);
        // (The settings test measures 5 s a level: 500 frames of 10 ms, less the one not
        // measured after a picture where there was one.)
        assert!(r.levels.iter().all(|l| (498..=501).contains(&l.stats.frames)), "{:?}", r.levels);
        assert!((498..=501).contains(&r.reference.frames), "{:?}", r.reference);
        assert_eq!(r.reference_scale, 0.75);
        assert!(r.timeline.len() > 100);
        // The scenes' frames in the histogram and together, the tests' not: all at 100 a second.
        assert_eq!(r.histogram[1], r.scenes().map(|p| p.stats.frames).sum::<u32>());
        assert_eq!(r.overall().frames, r.histogram[1]);
        // A picture asked of every scene.
        assert_eq!(seen.iter().filter(|n| **n == Next::Picture).count(), SCENES.len());
    }

    /// A quick run: the storm to the copy, the settings test, the twenty thousand; no ray
    /// tracing, no effects test.
    #[test]
    fn a_quick_run_measures_what_the_scores_are_made_of() {
        let mut b = Benchmark::new(std::env::temp_dir().join("bench.json"), Mode::Quick, false, |_| true).unwrap();
        assert_eq!(b.stage().map.as_deref(), Some("Highland Rift"));
        let seen = play(&mut b);
        assert_eq!(seen.iter().filter(|n| matches!(n, Next::Load(_))).count(), 1);
        assert!(!seen.iter().any(|n| matches!(n, Next::Effect(_))));
        let r = report(&b);
        assert_eq!(r.parts.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), report::QUICK_PARTS);
        // (The storm only to the copy.)
        assert!(r.parts[0].seconds < TEST_FROM, "{}", r.parts[0].seconds);
        assert!(r.effects.is_empty() && r.levels.len() == LEVELS.len());
        assert!(r.largest().frames > 0);
        assert_eq!(seen.iter().filter(|n| **n == Next::Picture).count(), 2);
    }

    /// Every scene reads, and leaves out what an install without the expansions lacks.
    #[test]
    fn the_scenes_read_and_leave_out_the_expansions() {
        let b = Benchmark::new(std::env::temp_dir().join("bench.json"), Mode::Full, false, |kind| !kind.eq_ignore_ascii_case("CORKROG")).unwrap();
        assert_eq!(b.left_out, ["CORKROG"]);
        assert!(b.scenes.iter().all(|s| s.units.iter().all(|u| !u.kind.eq_ignore_ascii_case("CORKROG"))));
        // Each scene's ids are the report's, and their pictures come inside their lengths.
        assert_eq!(SCENES.map(|s| s.id).to_vec(), report::FULL_PARTS.iter().copied().filter(|p| !report::TESTS.contains(p)).collect::<Vec<_>>());
        assert!(SCENES.iter().all(|s| s.picture_at > SETTLE && s.picture_at < s.seconds));
        // (Each with its caption, in the same order.)
        assert_eq!(SCENES.map(|s| s.id), ta_ui::benchmark::SCENES.map(|s| s.id));
        // The storm's tests come from seconds it plays.
        assert!(TEST_FROM + TEST_SETTLE + TEST_MEASURE < SCENES[scene_index("highland")].seconds);
    }
}
