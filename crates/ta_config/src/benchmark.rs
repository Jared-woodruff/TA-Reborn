//! The game's benchmark (`ta-reborn --benchmark <file>`, which the launcher starts): what a run
//! measured, as the game writes it (JSON) and the launcher reads it; its statistics
//! ([`summarize`]), the scores made of it ([`Report::scores`]) and the settings it recommends
//! ([`Report::recommend`]).
//!
//! A full run is eight staged scenes on TA Reborn's own worlds at the player's settings (the
//! frame rate unlimited), each showing off what the game draws there: a dawn sea battle, a ride
//! in a tank's cockpit through a forest, a battle in a thunderstorm, a station in the void, a base
//! rising under the nano-lathes, twenty thousand units on a metal world, a lava world's nightfall
//! with a nuclear strike, and a town's fireworks at midnight. After the storm, two tests replay
//! its same seconds: the settings test, at each graphics level and once more at High with about
//! 1080p's pixels drawn (the reference pass, whatever the screen); and the effects test, at High
//! with one effect turned off at a time (what each costs). A quick run plays the storm's first
//! seconds, the settings test and the twenty thousand: the scores' own measurements, so its
//! scores are a full run's.
//!
//! The scores compare this computer with the reference one (an RTX 4080, a Ryzen 9 5950X, 64
//! GB), which scores 10,000 in each: the graphics card by its own time on the reference pass (as
//! at 1080p with 4x antialiasing), the processor by the simulation of the largest battle and the
//! rest of a frame's work, the memory by copying, by reading where it cannot be foreseen, and by
//! how much there is. On the reference computer the scores came within a few percent of 10,000
//! at every antialiasing level and at 1080p, 1440p and 4K (`python tools/benchmark/summary.py`
//! sums up a report, for setting `reference` again after a change to what a frame draws).
//! Reports of the first benchmark (three battles) keep the scores they were given.

use serde::{Deserialize, Serialize};

use crate::{Graphics, Preset};

/// The report's format: a launcher reads its own and later ones' fields it knows.
pub const FORMAT: u32 = 2;

/// What the benchmark plays: its scenes and tests. Runs of different content are not compared
/// (reports before it was kept, the first, read as 0).
pub const CONTENT: u32 = 2;

/// A full run's parts, in the order they play (`settings` and `effects` are the tests).
pub const FULL_PARTS: [&str; 10] = ["sapphire", "cockpit", "highland", "settings", "effects", "space", "build", "swarm", "cinder", "winter"];

/// A quick run's: the storm's first seconds, the settings test, the twenty thousand.
pub const QUICK_PARTS: [&str; 3] = ["highland", "settings", "swarm"];

/// The first benchmark's parts (reports of content 0 and 1).
pub const FIRST_PARTS: [&str; 4] = ["sapphire", "highland", "settings", "cinder"];

/// The parts that are tests rather than scenes.
pub const TESTS: [&str; 2] = ["settings", "effects"];

/// How long a run is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Every scene and both tests: about five minutes.
    #[default]
    Full,
    /// What the scores are made of: about a minute and a half.
    Quick,
}

impl Mode {
    pub fn parts(self) -> &'static [&'static str] {
        match self {
            Mode::Full => &FULL_PARTS,
            Mode::Quick => &QUICK_PARTS,
        }
    }

    /// As the command line names it (`--benchmark-mode`).
    pub fn name(self) -> &'static str {
        match self {
            Mode::Full => "full",
            Mode::Quick => "quick",
        }
    }

    pub fn from_name(name: &str) -> Option<Mode> {
        match name {
            "full" => Some(Mode::Full),
            "quick" => Some(Mode::Quick),
            _ => None,
        }
    }
}

/// A frame this long or longer (ms), and three times the part's middle frame or more, is a
/// hitch: a stutter the eye catches.
pub const HITCH_MS: f32 = 50.0;

/// The frame-time histogram's buckets: their upper edges (ms) — 120 frames a second and more,
/// 90, 60, 40, 30, 20, and slower.
pub const FRAME_BUCKETS: [f32; 6] = [1000.0 / 120.0, 1000.0 / 90.0, 1000.0 / 60.0, 1000.0 / 40.0, 1000.0 / 30.0, 1000.0 / 20.0];

/// What the effects test turns off, one at a time, from High (`High` is High itself).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    #[default]
    High,
    RayTracing,
    Reflections,
    Shadows,
    AmbientOcclusion,
    Trees,
    HdSurfaces,
    Weather,
    HeatHaze,
    RenderScale,
}

impl Effect {
    /// Every one the test plays, in order (ray tracing only where it is on).
    pub const ALL: [Effect; 10] = [Effect::High, Effect::RayTracing, Effect::Reflections, Effect::Shadows, Effect::AmbientOcclusion, Effect::Trees, Effect::HdSurfaces, Effect::Weather, Effect::HeatHaze, Effect::RenderScale];

    /// High with this effect turned off (shadows to Low, the render scale to 75%; ray tracing
    /// is the renderer's own, outside the settings).
    pub fn apply(self, g: &mut Graphics) {
        Preset::High.apply(g);
        match self {
            Effect::High | Effect::RayTracing => {}
            Effect::Reflections => g.water_reflections = crate::Reflections::Off,
            Effect::Shadows => g.shadows = crate::Quality::Low,
            Effect::AmbientOcclusion => g.ambient_occlusion = false,
            Effect::Trees => g.trees_3d = false,
            Effect::HdSurfaces => g.hd_surfaces = false,
            Effect::Weather => g.weather = false,
            Effect::HeatHaze => g.heat_haze = false,
            Effect::RenderScale => g.render_scale = 0.75,
        }
    }
}

/// The effects test at one effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectRun {
    pub effect: Effect,
    pub stats: Stats,
}

/// A renderer's pass and its time on the graphics card (ms a frame, on average).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PassTime {
    pub pass: String,
    pub ms: f32,
}

/// The graphics levels the settings test plays, in order (the reference pass after them).
pub const LEVELS: [Preset; 4] = [Preset::Low, Preset::Medium, Preset::High, Preset::Ultra];

/// The frame rate the recommended level keeps to in its slowest frames (a battle's pace: 30
/// ticks a second, drawn twice each).
pub const TARGET_FPS: f32 = 60.0;

/// The reference computer's results: what scores 10,000 (this content's: the storm's seconds,
/// the twenty thousand). Measured in quick runs at 1080p, 1440p and 4K and at each antialiasing
/// level (`tools/benchmark/calibrate.sh`, `fit.py`); the processor's figures are their average (at 4K the
/// graphics driver's own threads take a little of the processor: a few percent slower there).
mod reference {
    /// The graphics card's time on a frame of the reference pass, at 1080p with 4x
    /// antialiasing.
    pub const GPU_MS: f32 = 5.32;
    /// The pixels the reference pass draws (1080p's).
    pub const PIXELS: f32 = 1920.0 * 1080.0;
    /// A simulation tick of the largest battle (twenty thousand units).
    pub const TICK_MS: f32 = 14.8;
    /// The rest of a frame's work on the processor in the reference pass (the game, the
    /// interface, preparing the picture: the simulation aside).
    pub const PREP_MS: f32 = 6.7;
    /// Copying in memory, every thread at once.
    pub const BANDWIDTH_GBS: f32 = 39.6;
    /// A read the processor cannot foresee.
    pub const LATENCY_NS: f32 = 96.0;
}

/// The first benchmark's reference (its reports keep their scores). Rendering baselines were
/// remeasured after the stable foliage LOD pass (4K/8x, normalized to 1080p/4x); simulation and
/// memory retain the original six-run averages.
mod first {
    pub const GPU_MS: f32 = 5.46;
    pub const TICK_MS: f32 = 4.15;
    pub const PREP_MS: f32 = 6.3;

    /// The reference pass's cost at a size (see [`super::pass_cost`]).
    pub fn pass_cost(drawn: f32, shown: f32) -> f32 {
        1.97 + 1.09 * drawn + 0.18 * shown
    }

    /// Antialiasing's cost against 4x (see [`super::msaa_cost`]).
    pub fn msaa_cost(samples: u32) -> f32 {
        match samples {
            0 | 1 => 0.78,
            2 => 0.91,
            3..=4 => 1.0,
            _ => 1.11,
        }
    }
}

/// What scores 10,000: a reference's figures and the costs its graphics card's time is read by.
#[derive(Clone, Copy)]
struct Reference {
    gpu_ms: f32,
    tick_ms: f32,
    prep_ms: f32,
    pass_cost: fn(f32, f32) -> f32,
    msaa_cost: fn(u32) -> f32,
}

const REFERENCE: Reference = Reference { gpu_ms: reference::GPU_MS, tick_ms: reference::TICK_MS, prep_ms: reference::PREP_MS, pass_cost, msaa_cost };
const FIRST_REFERENCE: Reference = Reference { gpu_ms: first::GPU_MS, tick_ms: first::TICK_MS, prep_ms: first::PREP_MS, pass_cost: first::pass_cost, msaa_cost: first::msaa_cost };

/// What the reference pass costs the graphics card at a size: a part whatever the size (the
/// shadows, the world's geometry), a part for each megapixel drawn and one for each shown (what
/// is drawn over the scene at the screen's size). Measured on the reference computer (at
/// 1080p, 1440p and 4K, within 1.5%; the part a megapixel drawn as the first benchmark
/// measured it: the pass draws about 1080p's pixels on all of them); `drawn` and `shown` in
/// megapixels.
fn pass_cost(drawn: f32, shown: f32) -> f32 {
    2.45 + 1.09 * drawn + 0.326 * shown
}

/// The render scale of the reference pass in a picture of `width` x `height`: about 1080p's
/// pixels drawn (the renderer's scale goes from a half to the whole).
pub fn reference_scale(width: u32, height: u32) -> f32 {
    let pixels = (width as f32 * height as f32).max(1.0);
    (reference::PIXELS / pixels).sqrt().clamp(0.5, 1.0)
}

/// What antialiasing costs the graphics card in the reference pass, against 4x (measured on
/// the reference computer; other cards differ a little).
pub fn msaa_cost(samples: u32) -> f32 {
    match samples {
        0 | 1 => 0.795,
        2 => 0.903,
        3..=4 => 1.0,
        _ => 1.128,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Report {
    pub format: u32,
    /// What it played ([`CONTENT`]; 0 for the first benchmark's reports), and how long a run.
    pub content: u32,
    pub mode: Mode,
    /// The game's version ("2.2.0").
    pub version: String,
    /// When it ran: seconds since 1970.
    pub when: u64,
    pub system: System,
    /// The settings it ran at (the player's; the settings test aside).
    pub graphics: Graphics,
    /// Actual paint accepted by the renderer. Older reports always rendered Reborn,
    /// even when their graphics settings recorded a different selected skin.
    pub rendered_unit_skin: Option<String>,
    /// Actual enemy paint. Older skin-enabled reports used one pack for both sides.
    pub rendered_enemy_unit_skin: Option<String>,
    /// Accessibility effects change the rendering workload; retain them with each run.
    pub reduced_motion: bool,
    pub reduced_flashes: bool,
    /// The picture's size in pixels, and the screen's refresh rate (0: not known).
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f32,
    /// The parts as they played ([`PARTS`]).
    pub parts: Vec<Part>,
    /// The run, a sample every quarter of a second.
    pub timeline: Vec<Sample>,
    /// The settings test: each level ([`LEVELS`]) on the same seconds.
    pub levels: Vec<LevelRun>,
    /// The effects test (a full run's): High, then High with each effect off ([`Effect::ALL`]).
    pub effects: Vec<EffectRun>,
    /// The scenes' frames by their time: how many fell in each of [`FRAME_BUCKETS`] (and the
    /// last, slower).
    pub histogram: Vec<u32>,
    /// The scenes' frames together (the tests aside), as one stretch (none in the first
    /// benchmark's reports: see [`Report::overall`]).
    pub all: Stats,
    /// The reference pass: High, about 1080p's pixels drawn ([`reference_scale`]).
    pub reference: Stats,
    pub reference_scale: f32,
    /// The antialiasing the settings test kept (it changes only when the game starts).
    pub tested_msaa: u32,
    pub memory: Memory,
    /// Whether the graphics card timed its work (else its times are 0).
    pub gpu_timed: bool,
    /// Unit types the scenes would have had that are not installed (an expansion missing).
    pub left_out: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct System {
    pub gpu: String,
    /// The graphics interface drawn through ("Vulkan", "Dx12") and the driver.
    pub backend: String,
    pub driver: String,
    pub cpu: String,
    pub threads: u32,
    pub ram_mb: u64,
    pub vram_mb: u64,
}

/// A part of the run: one of the scenes, or a test.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Part {
    /// One of [`Mode::parts`].
    pub id: String,
    /// When it started in the run, and how long it played (seconds).
    pub start: f32,
    pub seconds: f32,
    pub stats: Stats,
    /// The graphics card's time a frame in each of the renderer's passes (when timed).
    pub passes: Vec<PassTime>,
    /// A picture of the scene as this computer drew it: a file in the report's folder (see
    /// [`pictures_dir`]), if one was taken.
    pub picture: Option<String>,
}

/// Where a report's pictures are: a folder beside it, named as it is.
pub fn pictures_dir(report: &std::path::Path) -> std::path::PathBuf {
    report.with_extension("")
}

/// What a stretch of frames came to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub frames: u32,
    /// Frames a second on average, and of the slowest 1% and 0.1% of frames (their average).
    pub fps: f32,
    pub low1: f32,
    pub low01: f32,
    /// A frame's time on average and at the 99th percentile; at the middle and the 90th.
    pub frame_ms: f32,
    pub frame_p99: f32,
    pub frame_p50: f32,
    pub frame_p90: f32,
    /// Frames that hitched (see [`HITCH_MS`]).
    pub hitches: u32,
    /// A frame's work on average: the processor's (the game, the interface, preparing the
    /// picture) and the graphics card's (0 when not timed).
    pub cpu_ms: f32,
    pub gpu_ms: f32,
    /// A simulation tick on average, and the ticks a second it played (30: it kept up).
    pub tick_ms: f32,
    pub ticks_per_second: f32,
    /// The most units at once.
    pub units: u32,
}

impl Stats {
    /// The processor's work on a frame besides the simulation's.
    pub fn prep_ms(&self) -> f32 {
        (self.cpu_ms - self.tick_ms * self.ticks_per_second / self.fps.max(1.0)).max(0.1)
    }
}

/// A moment of the run: a quarter of a second's frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sample {
    /// Seconds into the run (the window's start).
    pub t: f32,
    /// The part (an index into [`Report::parts`]).
    pub part: u8,
    pub fps: f32,
    /// The slowest frame's rate.
    pub low: f32,
    pub cpu_ms: f32,
    pub gpu_ms: f32,
    pub tick_ms: f32,
    pub units: u32,
}

/// The settings test at one graphics level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LevelRun {
    pub level: Preset,
    pub stats: Stats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    /// The game's most memory in use (its working set), MB.
    pub peak_mb: u64,
    /// Copying in memory, every thread at once: GB a second, read and written.
    pub bandwidth_gbs: f32,
    /// A read the processor cannot foresee: nanoseconds.
    pub latency_ns: f32,
}

/// A frame as the game measured it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// Seconds into the run.
    pub t: f32,
    pub ms: f32,
    pub cpu_ms: f32,
    /// None: no new timing came from the graphics card for it.
    pub gpu_ms: Option<f32>,
    /// The simulation's ticks in the frame and their time.
    pub ticks: u32,
    pub tick_ms: f32,
    pub units: u32,
}

/// What `frames` came to (none: all zero).
pub fn summarize(frames: &[Frame]) -> Stats {
    if frames.is_empty() {
        return Stats::default();
    }
    let n = frames.len();
    let total_ms: f32 = frames.iter().map(|f| f.ms).sum();
    let mut sorted: Vec<f32> = frames.iter().map(|f| f.ms).collect();
    sorted.sort_by(|a, b| b.total_cmp(a));
    // (The slowest share's frames, at least one: their average rate.)
    let low = |share: f32| {
        let k = ((n as f32 * share).ceil() as usize).clamp(1, n);
        let ms = sorted[..k].iter().sum::<f32>() / k as f32;
        if ms > 0.0 { 1000.0 / ms } else { 0.0 }
    };
    // (Slowest first: the 99th percentile is 1% down from the top.)
    let at = |share: f32| sorted[((n as f32 * share).floor() as usize).min(n - 1)];
    let (p99, p90, p50) = (at(0.01), at(0.10), at(0.5));
    let hitches = frames.iter().filter(|f| f.ms >= HITCH_MS && f.ms >= 3.0 * p50).count() as u32;
    let gpu: Vec<f32> = frames.iter().filter_map(|f| f.gpu_ms).collect();
    let ticks: u32 = frames.iter().map(|f| f.ticks).sum();
    let tick_ms: f32 = frames.iter().map(|f| f.tick_ms).sum();
    let seconds = total_ms / 1000.0;
    Stats {
        frames: n as u32,
        fps: if total_ms > 0.0 { n as f32 * 1000.0 / total_ms } else { 0.0 },
        low1: low(0.01),
        low01: low(0.001),
        frame_ms: total_ms / n as f32,
        frame_p99: p99,
        frame_p50: p50,
        frame_p90: p90,
        hitches,
        cpu_ms: frames.iter().map(|f| f.cpu_ms).sum::<f32>() / n as f32,
        gpu_ms: if gpu.is_empty() { 0.0 } else { gpu.iter().sum::<f32>() / gpu.len() as f32 },
        tick_ms: if ticks > 0 { tick_ms / ticks as f32 } else { 0.0 },
        ticks_per_second: if seconds > 0.0 { ticks as f32 / seconds } else { 0.0 },
        units: frames.iter().map(|f| f.units).max().unwrap_or(0),
    }
}

/// How many of `frames` fell in each of [`FRAME_BUCKETS`] (and the last, slower).
pub fn histogram(frames: impl IntoIterator<Item = f32>) -> Vec<u32> {
    let mut counts = vec![0u32; FRAME_BUCKETS.len() + 1];
    for ms in frames {
        let i = FRAME_BUCKETS.iter().position(|&edge| ms < edge).unwrap_or(FRAME_BUCKETS.len());
        counts[i] += 1;
    }
    counts
}

/// The run as samples a quarter of a second apart (`parts` gives each frame's part: the index
/// of the part it played in).
pub fn timeline(frames: &[(u8, Frame)]) -> Vec<Sample> {
    const STEP: f32 = 0.25;
    let mut out: Vec<Sample> = Vec::new();
    let mut window: Vec<Frame> = Vec::new();
    let mut current: Option<(u8, f32)> = None;
    let flush = |window: &mut Vec<Frame>, part: u8, t: f32, out: &mut Vec<Sample>| {
        if window.is_empty() {
            return;
        }
        let s = summarize(window);
        let slowest = window.iter().map(|f| f.ms).fold(0.0f32, f32::max);
        out.push(Sample { t, part, fps: s.fps, low: if slowest > 0.0 { 1000.0 / slowest } else { 0.0 }, cpu_ms: s.cpu_ms, gpu_ms: s.gpu_ms, tick_ms: s.tick_ms, units: s.units });
        window.clear();
    };
    for &(part, f) in frames {
        match current {
            Some((p, start)) if p == part && f.t < start + STEP => {}
            Some((p, start)) => {
                flush(&mut window, p, start, &mut out);
                current = Some((part, if p == part { start + STEP * ((f.t - start) / STEP).floor() } else { f.t }));
            }
            None => current = Some((part, f.t)),
        }
        window.push(f);
    }
    if let Some((p, start)) = current {
        flush(&mut window, p, start, &mut out);
    }
    out
}

/// Copying in memory, every thread at once: GB a second read and written (the best of a few
/// rounds; about a tenth of a second, 256 MB).
pub fn memory_bandwidth() -> f32 {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    let each = (128 << 20) / threads;
    let mut buffers: Vec<(Vec<u8>, Vec<u8>)> = (0..threads).map(|i| (vec![i as u8 | 1; each], vec![0u8; each])).collect();
    let mut best = 0.0f32;
    for _ in 0..6 {
        let start = std::time::Instant::now();
        std::thread::scope(|s| {
            for (from, to) in buffers.iter_mut() {
                s.spawn(move || {
                    to.copy_from_slice(from);
                    std::hint::black_box(&to);
                });
            }
        });
        let seconds = start.elapsed().as_secs_f32();
        if seconds > 0.0 {
            best = best.max((threads * each * 2) as f32 / seconds / 1e9);
        }
    }
    best
}

/// Reading memory where the processor cannot foresee the next place: nanoseconds a read (a
/// chain of reads through 256 MB in a random order, each read's place in the one before;
/// about half a second).
pub fn memory_latency_ns() -> f32 {
    // A cache line each, holding the next one's index: the lines in one random cycle.
    const LINES: usize = (256 << 20) / 64;
    const WORDS: usize = 16;
    let mut order: Vec<u32> = (0..LINES as u32).collect();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for i in (1..LINES).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        order.swap(i, (seed % (i as u64 + 1)) as usize);
    }
    let mut next = vec![0u32; LINES * WORDS];
    for k in 0..LINES {
        next[order[k] as usize * WORDS] = order[(k + 1) % LINES];
    }
    drop(order);
    const READS: usize = 1 << 21;
    let mut at = 0u32;
    let mut best = f32::INFINITY;
    for _ in 0..2 {
        let start = std::time::Instant::now();
        for _ in 0..READS {
            at = next[at as usize * WORDS];
        }
        let ns = start.elapsed().as_secs_f32() * 1e9 / READS as f32;
        best = best.min(ns);
    }
    std::hint::black_box(at);
    best
}

/// The scores: 10,000 is the reference computer's in each.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scores {
    pub overall: u32,
    pub cpu: u32,
    pub gpu: u32,
    pub ram: u32,
}

/// What the overall score says the computer is, in TA's terms (the smallest first).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Peewee,
    Hammer,
    Bulldog,
    Goliath,
    Commander,
    Krogoth,
}

impl Tier {
    pub fn of(overall: u32) -> Tier {
        match overall {
            0..1_500 => Tier::Peewee,
            1_500..3_000 => Tier::Hammer,
            3_000..5_000 => Tier::Bulldog,
            5_000..7_500 => Tier::Goliath,
            7_500..10_000 => Tier::Commander,
            _ => Tier::Krogoth,
        }
    }

    /// Its name (a unit's, as TA has it: not translated).
    pub fn name(self) -> &'static str {
        match self {
            Tier::Peewee => "PEEWEE",
            Tier::Hammer => "HAMMER",
            Tier::Bulldog => "BULLDOG",
            Tier::Goliath => "GOLIATH",
            Tier::Commander => "COMMANDER",
            Tier::Krogoth => "KROGOTH",
        }
    }

    /// The overall score the next tier starts at (none for the last).
    pub fn next_at(self) -> Option<u32> {
        match self {
            Tier::Peewee => Some(1_500),
            Tier::Hammer => Some(3_000),
            Tier::Bulldog => Some(5_000),
            Tier::Goliath => Some(7_500),
            Tier::Commander => Some(10_000),
            Tier::Krogoth => None,
        }
    }
}

/// What the recommendation says besides the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    /// The graphics card sets the pace.
    GpuBound,
    /// The processor sets the pace in the largest battles.
    CpuBound,
    /// Neither holds the other back much.
    Balanced,
    /// Even the lowest level falls short of the target: the scene drawn at a lower resolution.
    BelowTarget,
    /// Little memory, or the game used most of it.
    Memory,
    /// The game runs faster than the screen refreshes: vertical sync for a steady picture.
    Vsync,
    /// The graphics card's times were not measured (the scores lean on the frame times).
    Untimed,
    /// Units of an expansion not installed were left out of the scenes.
    LeftOut,
}

/// The settings a report recommends.
#[derive(Clone, Debug, PartialEq)]
pub struct Recommendation {
    pub level: Preset,
    pub msaa: u32,
    pub render_scale: f32,
    pub vsync: bool,
    /// The most units a player the processor keeps up with (two players at it, the simulation
    /// at full speed): a unit limit above it comes down to it (one below stays: it is the
    /// player's choice of battle).
    pub unit_limit: u32,
    /// The frame rate aimed for ([`TARGET_FPS`]), and the one the recommended level held in the
    /// settings test (its slowest 1% of frames).
    pub target_fps: f32,
    pub expected_fps: f32,
    pub notes: Vec<Note>,
}

impl Recommendation {
    /// The recommended settings put into `g` (and `unit_limit` brought down to what the
    /// processor keeps up with).
    pub fn apply(&self, g: &mut Graphics, unit_limit: &mut u32) {
        self.level.apply(g);
        g.msaa = self.msaa;
        g.render_scale = self.render_scale;
        g.vsync = self.vsync;
        g.preset = g.matching_preset();
        *unit_limit = (*unit_limit).min(self.unit_limit);
    }

    /// Whether `g` (and `unit_limit`) already are these settings.
    pub fn applied(&self, g: &Graphics, unit_limit: u32) -> bool {
        let mut wanted = g.clone();
        let mut limit = unit_limit;
        self.apply(&mut wanted, &mut limit);
        wanted == *g && limit == unit_limit
    }
}

impl Report {
    /// Skin used by this run; reports predating skin-enabled benchmarks used Reborn.
    pub fn unit_skin(&self) -> &str {
        self.rendered_unit_skin.as_deref().unwrap_or("reborn")
    }

    pub fn enemy_unit_skin(&self) -> &str {
        self.rendered_enemy_unit_skin.as_deref().unwrap_or_else(|| self.unit_skin())
    }

    pub fn part(&self, id: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.id == id)
    }

    pub fn level(&self, level: Preset) -> Option<&LevelRun> {
        self.levels.iter().find(|l| l.level == level)
    }

    /// Whether it is the first benchmark's (three battles; scored by its own reference).
    pub fn first(&self) -> bool {
        self.content < 2
    }

    /// The largest battle, the processor's test (the twenty thousand; the first benchmark's
    /// nightfall).
    pub fn largest(&self) -> Stats {
        self.part(if self.first() { "cinder" } else { "swarm" }).map(|p| p.stats).unwrap_or_default()
    }

    /// The scenes (the tests aside).
    pub fn scenes(&self) -> impl Iterator<Item = &Part> {
        self.parts.iter().filter(|p| !TESTS.contains(&p.id.as_str()))
    }

    /// What each effect costs the graphics card at High: its time with the effect on less its
    /// time with it off (ms a frame; the frame's own time where the card was not timed), and the
    /// share of High's frame that is. None without the effects test.
    pub fn effect_costs(&self) -> Vec<(Effect, f32, f32)> {
        let ms = |s: &Stats| if self.gpu_timed && s.gpu_ms > 0.0 { s.gpu_ms } else { s.frame_ms };
        let Some(high) = self.effects.iter().find(|e| e.effect == Effect::High).map(|e| ms(&e.stats)).filter(|&ms| ms > 0.0) else { return Vec::new() };
        self.effects.iter().filter(|e| e.effect != Effect::High).map(|e| {
            let cost = high - ms(&e.stats);
            (e.effect, cost, cost / high)
        }).collect()
    }

    /// The scenes' frames together (the tests aside): as measured, or (the first benchmark's
    /// reports) made up of each scene's figures.
    pub fn overall(&self) -> Stats {
        if self.all.frames > 0 {
            return self.all;
        }
        let scenes: Vec<&Part> = self.scenes().collect();
        let frames: u32 = scenes.iter().map(|p| p.stats.frames).sum();
        if frames == 0 {
            return Stats::default();
        }
        let weighted = |f: fn(&Stats) -> f32| scenes.iter().map(|p| f(&p.stats) * p.stats.frames as f32).sum::<f32>() / frames as f32;
        let seconds: f32 = scenes.iter().map(|p| p.stats.frame_ms * p.stats.frames as f32).sum::<f32>() / 1000.0;
        Stats {
            frames,
            fps: if seconds > 0.0 { frames as f32 / seconds } else { 0.0 },
            low1: scenes.iter().map(|p| p.stats.low1).fold(f32::INFINITY, f32::min),
            low01: scenes.iter().map(|p| p.stats.low01).fold(f32::INFINITY, f32::min),
            frame_ms: weighted(|s| s.frame_ms),
            frame_p99: scenes.iter().map(|p| p.stats.frame_p99).fold(0.0, f32::max),
            frame_p50: weighted(|s| s.frame_p50),
            frame_p90: weighted(|s| s.frame_p90),
            hitches: scenes.iter().map(|p| p.stats.hitches).sum(),
            cpu_ms: weighted(|s| s.cpu_ms),
            gpu_ms: weighted(|s| s.gpu_ms),
            tick_ms: weighted(|s| s.tick_ms),
            ticks_per_second: weighted(|s| s.ticks_per_second),
            units: scenes.iter().map(|p| p.stats.units).max().unwrap_or(0),
        }
    }

    /// The scores (see the module's notes).
    pub fn scores(&self) -> Scores {
        let score = |ratio: f32| if ratio.is_finite() && ratio > 0.0 { (10_000.0 * ratio).round().clamp(1.0, 99_999.0) } else { 0.0 };
        // The graphics card: its time on the reference pass, as at 1080p (drawn and shown) with
        // 4x antialiasing; untimed, the frame's time.
        let r = &self.reference;
        let gpu_ms = if self.gpu_timed && r.gpu_ms > 0.0 { r.gpu_ms } else { r.frame_ms };
        let shown = (self.width as f32 * self.height as f32 / 1e6).max(0.01);
        let drawn = shown * self.reference_scale * self.reference_scale;
        let at_1080p = reference::PIXELS / 1e6;
        // (The first benchmark's reports by its own reference.)
        let Reference { gpu_ms: gpu_ref, tick_ms: tick_ref, prep_ms: prep_ref, pass_cost: cost, msaa_cost: msaa } = if self.first() { FIRST_REFERENCE } else { REFERENCE };
        let gpu = score(gpu_ref / (gpu_ms * cost(at_1080p, at_1080p) / cost(drawn, shown) / msaa(self.tested_msaa)));
        // The processor: the largest battle's simulation, and the rest of a frame's work.
        let big = self.largest();
        let cpu = score(((tick_ref / big.tick_ms.max(0.01)) * (prep_ref / r.prep_ms())).sqrt());
        // The memory: copying and reading at random (each counting as its square root), and
        // how much there is (8 GB a fair way short, 16 GB enough; not known, as enough).
        let m = &self.memory;
        let room = if self.system.ram_mb == 0 { 1.0 } else { (self.system.ram_mb as f32 / 16_384.0).clamp(0.25, 1.0).sqrt() };
        let ram = score((m.bandwidth_gbs / reference::BANDWIDTH_GBS).sqrt() * (reference::LATENCY_NS / m.latency_ns.max(1.0)).sqrt() * room);
        let overall = if gpu > 0.0 && cpu > 0.0 && ram > 0.0 { (0.5 * gpu.ln() + 0.35 * cpu.ln() + 0.15 * ram.ln()).exp().round() } else { 0.0 };
        Scores { overall: overall as u32, cpu: cpu as u32, gpu: gpu as u32, ram: ram as u32 }
    }

    /// The settings for this computer: the highest level whose slowest frames in the settings
    /// test kept to [`TARGET_FPS`] (90% of it), the antialiasing no more than was tested; a unit
    /// limit the processor keeps up with.
    pub fn recommend(&self) -> Recommendation {
        let target = TARGET_FPS;
        let held = |l: &LevelRun| l.stats.low1.min(l.stats.fps);
        let best = LEVELS.iter().rev().filter_map(|&level| self.level(level)).find(|l| held(l) >= target * 0.9);
        let mut notes = Vec::new();
        let (level, expected, render_scale) = match best {
            Some(l) => (l.level, held(l), if l.level == Preset::Low { 0.75 } else { 1.0 }),
            None => {
                notes.push(Note::BelowTarget);
                let low = self.level(Preset::Low).map_or(0.0, held);
                (Preset::Low, low, 0.5_f32.max(0.75 * (low / (target * 0.9)).sqrt()).min(0.75))
            }
        };
        let mut defaults = Graphics::default();
        level.apply(&mut defaults);
        let msaa = defaults.msaa.min(self.tested_msaa.max(1));
        // Who sets the pace: the graphics card or the processor (in the largest battle).
        let big = self.largest();
        let at = self.level(level).map(|l| l.stats).unwrap_or_default();
        if self.gpu_timed && at.gpu_ms > 0.0 {
            notes.push(if big.cpu_ms > big.gpu_ms * 1.15 && big.ticks_per_second < 29.0 {
                Note::CpuBound
            } else if at.gpu_ms > at.cpu_ms * 1.15 {
                Note::GpuBound
            } else if big.cpu_ms > big.gpu_ms * 1.15 {
                Note::CpuBound
            } else {
                Note::Balanced
            });
        } else {
            notes.push(Note::Untimed);
        }
        if self.system.ram_mb > 0 && (self.system.ram_mb < 8_000 || self.memory.peak_mb as f32 > self.system.ram_mb as f32 * 0.6) {
            notes.push(Note::Memory);
        }
        // Vertical sync when the game runs faster than the screen refreshes: a steady picture
        // without tearing, for nothing (else as the player has it).
        let faster = self.refresh_hz > 0.0 && expected >= self.refresh_hz * 0.95;
        if faster {
            notes.push(Note::Vsync);
        }
        let vsync = self.graphics.vsync || faster;
        if !self.left_out.is_empty() {
            notes.push(Note::LeftOut);
        }
        // Each player's units: two players at the limit keep the simulation at full speed (a
        // tick in 25 ms of the 33), from the largest battle's, its cost growing a little faster
        // than the units; in round steps.
        let units = if big.tick_ms > 0.0 { big.units.max(1) as f32 * (25.0 / big.tick_ms).powf(1.0 / 1.2) } else { 40_000.0 };
        let unit_limit = [250, 500, 1_000, 1_500, 2_000, 3_000, 5_000, 10_000, 20_000].into_iter().rev().find(|&n| 2.0 * n as f32 <= units).unwrap_or(250);
        Recommendation { level, msaa, render_scale, vsync, unit_limit, target_fps: target, expected_fps: expected, notes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_provenance_survives_round_trip_and_defaults_legacy_runs_to_reborn() {
        let legacy: Report = toml::from_str("[graphics]\nunit_skin = 'christmas'\n").unwrap();
        assert_eq!(legacy.graphics.unit_skin, "christmas");
        assert_eq!(legacy.unit_skin(), "reborn");
        assert_eq!(legacy.enemy_unit_skin(), "reborn");
        let actual = Report { rendered_unit_skin: Some("winter-armour".into()), ..legacy };
        let loaded: Report = toml::from_str(&toml::to_string(&actual).unwrap()).unwrap();
        assert_eq!(loaded.unit_skin(), "winter-armour");
        assert_eq!(loaded.enemy_unit_skin(), "winter-armour");
        let separate = Report { rendered_enemy_unit_skin: Some("desert".into()), ..loaded };
        let loaded: Report = toml::from_str(&toml::to_string(&separate).unwrap()).unwrap();
        assert_eq!(loaded.unit_skin(), "winter-armour");
        assert_eq!(loaded.enemy_unit_skin(), "desert");
    }

    fn frames(ms: &[f32]) -> Vec<Frame> {
        let mut t = 0.0;
        ms.iter()
            .map(|&ms| {
                t += ms / 1000.0;
                Frame { t, ms, cpu_ms: ms * 0.5, gpu_ms: Some(ms * 0.8), ticks: 1, tick_ms: 2.0, units: 100 }
            })
            .collect()
    }

    #[test]
    fn stats_read_rates_and_lows() {
        // 99 frames at 10 ms and one at 50 ms: about 96 fps, its 1% low the 50 ms frame's 20,
        // and 99% of frames within 10 ms.
        let mut ms = vec![10.0; 99];
        ms.push(50.0);
        let s = summarize(&frames(&ms));
        assert_eq!(s.frames, 100);
        assert!((s.fps - 1000.0 / 10.4).abs() < 0.01, "{}", s.fps);
        assert!((s.low1 - 20.0).abs() < 0.01 && (s.low01 - 20.0).abs() < 0.01);
        assert_eq!(s.frame_p99, 10.0);
        assert_eq!((s.frame_p50, s.frame_p90), (10.0, 10.0));
        // (The 50 ms frame: over the hitch's mark and three times the middle one.)
        assert_eq!(s.hitches, 1);
        assert!((s.gpu_ms - 8.32).abs() < 0.01);
        assert_eq!(s.units, 100);
        assert_eq!(summarize(&[]), Stats::default());
        // Slow throughout is not hitching: a 60 ms frame among 40 ms ones.
        let mut slow = vec![40.0; 99];
        slow.push(60.0);
        assert_eq!(summarize(&frames(&slow)).hitches, 0);
    }

    #[test]
    fn the_histogram_counts_frames_by_their_rate() {
        // 200 fps, 100, 75, 45, 35, 25 and 10 a second: one in each bucket.
        let counts = histogram([5.0, 10.0, 13.3, 22.2, 28.6, 40.0, 100.0]);
        assert_eq!(counts, vec![1; FRAME_BUCKETS.len() + 1]);
        assert_eq!(histogram([]), vec![0; FRAME_BUCKETS.len() + 1]);
    }

    #[test]
    fn modes_and_effects() {
        assert_eq!(Mode::Quick.parts(), ["highland", "settings", "swarm"]);
        assert!(Mode::Full.parts().contains(&"effects") && Mode::Full.parts().len() == 10);
        assert_eq!(Mode::from_name(Mode::Quick.name()), Some(Mode::Quick));
        // A quick run measures what the scores are made of: the settings test and the largest
        // battle.
        assert!(QUICK_PARTS.iter().all(|p| FULL_PARTS.contains(p)));
        // Each effect turns off one thing from High.
        for e in Effect::ALL {
            let mut g = Graphics::default();
            e.apply(&mut g);
            let mut high = Graphics::default();
            Preset::High.apply(&mut high);
            assert_eq!(g.level_options_differ(&high), !matches!(e, Effect::High | Effect::RayTracing), "{e:?}");
        }
        // Their costs: High's frame less each one's.
        let run = |effect, gpu_ms| EffectRun { effect, stats: Stats { gpu_ms, frame_ms: gpu_ms + 1.0, ..Default::default() } };
        let r = Report { gpu_timed: true, effects: vec![run(Effect::High, 10.0), run(Effect::Reflections, 8.5), run(Effect::Trees, 9.5)], ..Default::default() };
        let costs = r.effect_costs();
        assert_eq!(costs.len(), 2);
        assert!((costs[0].1 - 1.5).abs() < 1e-4 && (costs[0].2 - 0.15).abs() < 1e-4);
        assert!(Report::default().effect_costs().is_empty());
    }

    #[test]
    fn the_timeline_takes_quarter_seconds_and_parts() {
        let part0: Vec<(u8, Frame)> = frames(&[100.0; 10]).into_iter().map(|f| (0, f)).collect();
        let mut all = part0;
        let last = all.last().map_or(0.0, |(_, f)| f.t);
        all.extend(frames(&[50.0; 4]).into_iter().map(|f| (1, Frame { t: last + f.t, ..f })));
        let samples = timeline(&all);
        assert!(samples.iter().filter(|s| s.part == 0).count() >= 3);
        assert!(samples.iter().any(|s| s.part == 1 && (s.fps - 20.0).abs() < 0.01));
        assert!(samples.windows(2).all(|w| w[0].t <= w[1].t));
    }

    #[test]
    fn the_reference_pass_draws_about_1080p() {
        assert_eq!(reference_scale(1920, 1080), 1.0);
        assert_eq!(reference_scale(3840, 2160), 0.5);
        assert_eq!(reference_scale(2560, 1440), 0.75);
        // (Smaller screens draw what they have; larger ones no less than half.)
        assert_eq!(reference_scale(1366, 768), 1.0);
        assert_eq!(reference_scale(7680, 4320), 0.5);
    }

    /// A report whose settings test took `gpu_ms` at each level (and the reference pass High's).
    fn report(gpu_ms: [f32; 4], refresh: f32) -> Report {
        let stats = |gpu: f32| Stats { frames: 300, fps: 1000.0 / gpu.max(4.0), low1: 1000.0 / (gpu.max(4.0) * 1.3), frame_ms: gpu.max(4.0), cpu_ms: 4.0, gpu_ms: gpu, tick_ms: 3.0, ticks_per_second: 30.0, units: 600, ..Default::default() };
        Report {
            content: CONTENT,
            width: 1920,
            height: 1080,
            refresh_hz: refresh,
            gpu_timed: true,
            tested_msaa: 4,
            system: System { ram_mb: 32_768, ..Default::default() },
            memory: Memory { peak_mb: 3_000, bandwidth_gbs: reference::BANDWIDTH_GBS, latency_ns: reference::LATENCY_NS },
            parts: vec![Part { id: "swarm".into(), stats: Stats { tick_ms: reference::TICK_MS, cpu_ms: 8.0, gpu_ms: 5.0, ticks_per_second: 30.0, units: 20_000, frames: 100, frame_ms: 8.0, fps: 125.0, ..Default::default() }, ..Default::default() }],
            levels: LEVELS.iter().zip(gpu_ms).map(|(&level, ms)| LevelRun { level, stats: stats(ms) }).collect(),
            reference: stats(gpu_ms[2]),
            reference_scale: 1.0,
            ..Default::default()
        }
    }

    #[test]
    fn the_reference_scores_ten_thousand() {
        let mut r = report([2.0, 3.0, reference::GPU_MS, 6.0], 144.0);
        // (Its frame's own work besides the simulation's, 3 ms a tick at 30 a second.)
        r.reference.cpu_ms = reference::PREP_MS + 3.0 * 30.0 / r.reference.fps;
        let s = r.scores();
        assert_eq!((s.gpu, s.cpu, s.ram, s.overall), (10_000, 10_000, 10_000, 10_000));
        assert_eq!(Tier::of(s.overall), Tier::Krogoth);
        // Memory of a size not known (where only Windows tells it) counts as enough, as 32 GB
        // does; 8 GB is short of it.
        let mut unknown = r.clone();
        unknown.system.ram_mb = 0;
        assert_eq!(unknown.scores().ram, 10_000);
        unknown.system.ram_mb = 8_192;
        assert_eq!(unknown.scores().ram, 7_071);
        // Half the card at the same pixels: half the score; the same card at 4K (drawing a
        // quarter of its pixels, showing them all: a little more work), the same score.
        let half = report([4.0, 6.0, 2.0 * reference::GPU_MS, 12.0], 144.0);
        assert_eq!(half.scores().gpu, 5_000);
        let mut big = r.clone();
        (big.width, big.height, big.reference_scale) = (3840, 2160, 0.5);
        big.reference.gpu_ms = reference::GPU_MS * pass_cost(2.0736, 8.2944) / pass_cost(2.0736, 2.0736);
        assert!(big.scores().gpu.abs_diff(10_000) <= 1, "{}", big.scores().gpu);
        // 8x antialiasing costs more: the same time with it is a faster card.
        big.tested_msaa = 8;
        assert!(big.scores().gpu > 10_000);
    }

    #[test]
    fn the_recommendation_holds_sixty() {
        // Ultra holds it (its slowest frames at 1000 / (6 * 1.3) = 128): Ultra, with no more
        // antialiasing than was tested, vertical sync on a 60 Hz screen (and as the player has it
        // on a faster one).
        let r = report([2.0, 3.0, 4.0, 6.0], 60.0);
        let rec = r.recommend();
        assert_eq!(rec.level, Preset::Ultra);
        assert_eq!(rec.msaa, 4, "no more antialiasing than was tested");
        assert!(rec.vsync && rec.notes.contains(&Note::Vsync));
        let mut fast = report([2.0, 3.0, 4.0, 6.0], 240.0);
        fast.graphics.vsync = false;
        assert!(!fast.recommend().vsync);
        // Ultra falls short (its slowest frames at 1000 / (18 * 1.3) = 43), High holds (55).
        let r = report([6.0, 9.0, 14.0, 18.0], 60.0);
        assert_eq!(r.recommend().level, Preset::High);
        // A slow card: Low's slowest frames, 1000 / (20 * 1.3) = 38, fall short too: Low, at a
        // lower resolution.
        let r = report([20.0, 24.0, 32.0, 48.0], 120.0);
        let rec = r.recommend();
        assert_eq!(rec.level, Preset::Low);
        assert!(rec.render_scale < 0.75 && rec.notes.contains(&Note::BelowTarget));
        // The unit limit: the reference's 14.8 ms a tick for 20,000 units: 31,000 at 25 ms, two
        // players' 10,000 each; a higher limit comes down to it, a lower one stays.
        assert_eq!(rec.unit_limit, 10_000);
        let mut g = Graphics::default();
        let mut limit = 20_000;
        rec.apply(&mut g, &mut limit);
        assert_eq!((g.msaa, limit), (1, 10_000));
        assert!(rec.applied(&g, limit));
        assert!(!rec.applied(&Graphics::default(), limit));
        let mut limit = 1_500;
        rec.apply(&mut g, &mut limit);
        assert_eq!(limit, 1_500);
        assert!(rec.applied(&g, 1_500));
    }

    /// A report of the first benchmark (no content, its largest battle the nightfall) keeps the
    /// scores its reference gave it.
    #[test]
    fn the_first_benchmarks_reports_keep_their_scores() {
        let old = r#"
            format = 1
            version = "2.1.0"
            width = 1920
            height = 1080
            gpu_timed = true
            tested_msaa = 4
            reference_scale = 1.0
            system = { ram_mb = 32768 }
            memory = { bandwidth_gbs = 39.6, latency_ns = 96.0 }
            reference = { frames = 300, fps = 183.15, frame_ms = 5.46, gpu_ms = 5.46, cpu_ms = 6.79, tick_ms = 3.0, ticks_per_second = 30.0 }
            [[parts]]
            id = "cinder"
            stats = { frames = 100, fps = 125.0, frame_ms = 8.0, cpu_ms = 8.0, gpu_ms = 5.0, tick_ms = 4.15, ticks_per_second = 30.0, units = 3544 }
        "#;
        let r: Report = toml::from_str(old).unwrap();
        assert!(r.first() && r.content == 0 && r.mode == Mode::Full);
        assert_eq!(r.largest().units, 3544);
        let s = r.scores();
        assert!(s.gpu.abs_diff(10_000) <= 2 && s.cpu.abs_diff(10_000) <= 2, "{s:?}");
    }
}
