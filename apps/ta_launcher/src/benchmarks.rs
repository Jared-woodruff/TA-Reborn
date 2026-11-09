//! The benchmark's reports: the game writes one per run (`ta-reborn --benchmark <file>`) into
//! `<settings>/benchmarks`, each scene's picture in a folder beside it, where the Benchmark page
//! reads them.

use std::path::{Path, PathBuf};

use ta_config::benchmark::{Mode, Recommendation, Report, Stats, LEVELS};
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};

/// Earlier runs kept (the oldest beyond them are deleted as a new one comes).
const KEPT: usize = 40;
/// A normal five-minute report is a few hundred KiB. Bound corrupt/shared reports before
/// allocating or constructing charts on the launcher thread.
const MAX_REPORT_BYTES: u64 = 4 << 20;
/// A scene's picture: a few tens of KiB; nothing bigger is read.
const MAX_PICTURE_BYTES: u64 = 2 << 20;
const MAX_SAMPLES: usize = 14_400;
static NEXT_REPORT: AtomicU64 = AtomicU64::new(0);

/// The runs there are, and the one under way.
#[derive(Default)]
pub struct Benchmarks {
    pub dir: PathBuf,
    /// The reports, newest first: (file, report).
    pub runs: Vec<(PathBuf, Report)>,
    /// The run shown (an index into `runs`).
    pub shown: usize,
    /// The run under way: where its report is to come, and how long a run it is.
    pub running: Option<PathBuf>,
    pub running_mode: Mode,
    /// The last run ended without a report: why.
    pub stopped: Option<String>,
    /// The scenes' pictures read so far (none: not there, or not a picture).
    pictures: std::collections::HashMap<PathBuf, Option<egui::TextureHandle>>,
}

impl Benchmarks {
    /// The reports in `<settings_dir>/benchmarks`.
    pub fn scan(settings_dir: &Path) -> Benchmarks {
        let dir = settings_dir.join("benchmarks");
        let mut runs: Vec<(PathBuf, Report)> = Vec::new();
        for path in std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|entry| entry.path()) {
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))
                && let Some(report) = read(&path)
            {
                runs.push((path, report));
                runs.sort_by(|a, b| b.1.when.cmp(&a.1.when).then_with(|| b.0.cmp(&a.0)));
                // Keep startup memory bounded even when many reports were copied here.
                runs.truncate(KEPT);
            }
        }
        Benchmarks { dir, runs, ..Default::default() }
    }

    /// The report shown.
    pub fn report(&self) -> Option<&Report> {
        self.runs.get(self.shown).map(|(_, r)| r)
    }

    /// The selected report's file, for opening exactly the run shown on the page.
    pub fn report_path(&self) -> Option<&Path> {
        self.runs.get(self.shown).map(|(path, _)| path.as_path())
    }

    /// Where the next run's report goes (named by when it starts).
    pub fn next_file(&self) -> PathBuf {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let sequence = NEXT_REPORT.fetch_add(1, Ordering::Relaxed);
        self.dir.join(format!("benchmark-{now}-{}-{sequence}.json", std::process::id()))
    }

    /// The run under way ended: its report taken in and shown (true), or why there is none.
    pub fn finished(&mut self, why_not: impl FnOnce() -> String) -> bool {
        let Some(file) = self.running.take() else { return false };
        match read(&file) {
            Some(report) => {
                log::info!("benchmark: {} (graphics card {}, processor {}, memory {}): {}", report.scores().overall, report.scores().gpu, report.scores().cpu, report.scores().ram, file.display());
                // A repeated completion must not retain a duplicate that pruning can later
                // delete from underneath the selected run.
                self.runs.retain(|(path, _)| path != &file);
                self.runs.insert(0, (file, report));
                self.shown = 0;
                self.stopped = None;
                // (The oldest go, their pictures with them.)
                for (path, _) in self.runs.drain(KEPT.min(self.runs.len())..) {
                    let _ = std::fs::remove_dir_all(ta_config::benchmark::pictures_dir(&path));
                    let _ = std::fs::remove_file(path);
                }
                true
            }
            None => {
                let why = why_not();
                log::info!("benchmark: no report ({why})");
                // (The pictures a stopped run took go: no report shows them.)
                let _ = std::fs::remove_dir_all(ta_config::benchmark::pictures_dir(&file));
                self.stopped = Some(why);
                false
            }
        }
    }
}

impl Benchmarks {
    /// The shown run's picture of a part (as its report names it), read once.
    pub fn picture(&mut self, ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
        let report = self.report_path()?;
        let path = ta_config::benchmark::pictures_dir(report).join(name);
        self.pictures.entry(path.clone()).or_insert_with(|| {
            let image = crate::library::read_picture(&path, MAX_PICTURE_BYTES)?;
            Some(ctx.load_texture(path.display().to_string(), image, egui::TextureOptions::LINEAR))
        }).clone()
    }
}

/// A report, if the file holds one.
pub fn read(path: &Path) -> Option<Report> {
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_REPORT_BYTES { return None; }
    let mut text = String::new();
    file.take(MAX_REPORT_BYTES + 1).read_to_string(&mut text).ok()?;
    if text.len() as u64 > MAX_REPORT_BYTES { return None; }
    match serde_json::from_str::<Report>(&text) {
        Ok(report) if valid(&report) => Some(report),
        Ok(_) => None,
        Err(e) => {
            log::warn!("{}: not a benchmark report: {e}", path.display());
            None
        }
    }
}

/// Scores remain viewable for older/partial reports, but settings are actionable only
/// after every preset has a measured result. Missing data must never recommend Low/50%.
pub fn recommendation(report: &Report) -> Option<Recommendation> {
    LEVELS.iter().all(|&level| report.level(level).is_some_and(|run|
        run.stats.frames > 0 && run.stats.fps > 0.0 && run.stats.low1 > 0.0))
        .then(|| report.recommend())
}

fn valid(report: &Report) -> bool {
    let bounded = |value: f32| value.is_finite() && (0.0..=1e9).contains(&value);
    let stats = |s: &Stats| [s.fps, s.low1, s.low01, s.frame_ms, s.frame_p99, s.frame_p50, s.frame_p90,
        s.cpu_ms, s.gpu_ms, s.tick_ms, s.ticks_per_second].into_iter().all(bounded);
    // (A picture is a file in the report's own folder: nothing that leads out of it.)
    let picture = |name: &str| !name.is_empty() && name.len() <= 64 && name.ends_with(".jpg") && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) && !name.contains("..");
    report.format >= 1 && !report.parts.is_empty() && report.parts.len() <= 256
        && report.levels.len() <= 16 && report.effects.len() <= 16 && report.histogram.len() <= 16 && report.timeline.len() <= MAX_SAMPLES
        && report.effects.iter().all(|run| stats(&run.stats)) && stats(&report.all)
        && report.parts.iter().all(|part| part.passes.len() <= 32 && part.passes.iter().all(|p| p.pass.len() <= 64 && bounded(p.ms)) && part.picture.as_deref().is_none_or(picture))
        && std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(report.when)).is_some()
        && report.histogram.iter().try_fold(0u32, |sum, count| sum.checked_add(*count)).is_some()
        && report.parts.iter().try_fold(0u32, |sum, part| sum.checked_add(part.stats.frames)).is_some()
        && report.parts.iter().try_fold(0u32, |sum, part| sum.checked_add(part.stats.hitches)).is_some()
        && report.parts.iter().all(|part| bounded(part.start) && bounded(part.seconds) && stats(&part.stats))
        && report.levels.iter().all(|run| stats(&run.stats)) && stats(&report.reference)
        && [report.refresh_hz, report.reference_scale, report.memory.bandwidth_gbs, report.memory.latency_ns,
            report.graphics.render_scale].into_iter().all(bounded)
        && report.timeline.iter().all(|sample| (sample.part as usize) < report.parts.len() && (0.0..=86_400.0).contains(&sample.t)
            && [sample.t, sample.fps, sample.low, sample.cpu_ms, sample.gpu_ms, sample.tick_ms].into_iter().all(bounded))
        && report.timeline.windows(2).all(|pair| pair[0].t <= pair[1].t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_config::benchmark::Part;

    fn report(when: u64) -> Report {
        Report { format: 1, when, parts: vec![Part { id: "sapphire".into(), ..Default::default() }], ..Default::default() }
    }

    /// Reports are read newest first; a run's report is taken in when it ends, and a run with
    /// none is told; the oldest go beyond those kept.
    #[test]
    fn runs_come_and_go() {
        let dir = std::env::temp_dir().join(format!("ta_launcher_bench_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("benchmarks")).unwrap();
        for when in [100, 300, 200] {
            std::fs::write(dir.join("benchmarks").join(format!("benchmark-{when}.json")), serde_json::to_string(&report(when)).unwrap()).unwrap();
        }
        std::fs::write(dir.join("benchmarks").join("broken.json"), "{ nope").unwrap();
        let mut b = Benchmarks::scan(&dir);
        assert_eq!(b.runs.iter().map(|(_, r)| r.when).collect::<Vec<_>>(), [300, 200, 100]);
        // A run that left no report: the pictures it took go.
        b.running = Some(b.dir.join("benchmark-400.json"));
        std::fs::create_dir_all(b.dir.join("benchmark-400")).unwrap();
        std::fs::write(b.dir.join("benchmark-400").join("sapphire.jpg"), b"not shown").unwrap();
        assert!(!b.finished(|| "stopped".into()));
        assert_eq!(b.stopped.as_deref(), Some("stopped"));
        assert!(!b.dir.join("benchmark-400").exists());
        // One that did.
        b.shown = 2;
        assert_eq!(b.report().map(|r| r.when), Some(100));
        assert_eq!(b.report_path(), Some(b.dir.join("benchmark-100.json").as_path()));
        let file = b.dir.join("benchmark-500.json");
        std::fs::write(&file, serde_json::to_string(&report(500)).unwrap()).unwrap();
        b.running = Some(file);
        assert!(b.finished(|| unreachable!()));
        assert_eq!((b.shown, b.report().map(|r| r.when), b.stopped.as_deref()), (0, Some(500), None));
        assert_eq!(b.report_path(), Some(b.dir.join("benchmark-500.json").as_path()));
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn incomplete_settings_tests_never_override_player_settings() {
        use ta_config::benchmark::LevelRun;
        let mut run = report(0);
        assert!(recommendation(&run).is_none());
        run.levels = LEVELS.iter().map(|&level| LevelRun { level,
            stats: Stats { frames: 100, fps: 100.0, low1: 80.0, ..Default::default() } }).collect();
        assert!(recommendation(&run).is_some());
        run.levels[2].stats.frames = 0;
        assert!(recommendation(&run).is_none());
        run.levels.pop();
        assert!(recommendation(&run).is_none());
    }

    #[test]
    fn malformed_reports_cannot_overflow_dates_or_charts() {
        use ta_config::benchmark::Sample;
        let mut run = report(0);
        assert!(valid(&run), "old reports with omitted optional fields remain readable");
        run.when = u64::MAX;
        assert!(!valid(&run));
        run.when = 0;
        run.timeline = vec![Sample { fps: f32::MAX, ..Default::default() }];
        assert!(!valid(&run));
        run.timeline[0].fps = -1.0;
        assert!(!valid(&run));
        run.timeline[0].fps = 60.0;
        run.timeline[0].part = 1;
        assert!(!valid(&run));
        run.timeline[0].part = 0;
        run.timeline[0].t = 2.0;
        run.timeline.push(Sample { t: 1.0, ..Default::default() });
        assert!(!valid(&run));
        run.timeline.clear();
        run.parts[0].stats.frames = u32::MAX;
        run.parts.push(Part { stats: Stats { frames: 1, ..Default::default() }, ..Default::default() });
        assert!(!valid(&run));
        // A scene's picture is a file of the report's own folder, nothing that leads out of it.
        let mut shared = report(0);
        shared.parts[0].picture = Some("sapphire.jpg".into());
        assert!(valid(&shared));
        for name in ["../escape.jpg", "a/b.jpg", "C:\\x.jpg", "x.png", "", "..jpg"] {
            shared.parts[0].picture = Some(name.into());
            assert!(!valid(&shared), "{name:?}");
        }
    }

    #[test]
    fn report_loading_is_bounded_and_new_runs_have_distinct_paths() {
        let root = std::env::temp_dir().join(format!("ta_launcher_bench_bounds_{}", std::process::id()));
        std::fs::create_dir_all(root.join("benchmarks")).unwrap();
        let oversized = root.join("benchmarks/oversized.json");
        std::fs::File::create(&oversized).unwrap().set_len(MAX_REPORT_BYTES + 1).unwrap();
        assert!(read(&oversized).is_none());
        for when in 0..45 {
            std::fs::write(root.join(format!("benchmarks/benchmark-{when}.json")), serde_json::to_string(&report(when)).unwrap()).unwrap();
        }
        let mut history = Benchmarks::scan(&root);
        assert_eq!(history.runs.len(), KEPT);
        assert_eq!(history.runs.first().unwrap().1.when, 44);
        assert_eq!(history.runs.last().unwrap().1.when, 5);
        assert_ne!(history.next_file(), history.next_file(), "fast cancellation/retry must never read an earlier run's report");
        let selected = history.runs[0].0.clone();
        history.running = Some(selected.clone());
        assert!(history.finished(|| unreachable!()));
        assert_eq!(history.runs.len(), KEPT);
        assert!(selected.exists());
        assert_eq!(history.runs.iter().filter(|(path, _)| path == &selected).count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_with_overflowing_histograms_or_hitch_totals_are_not_shown() {
        let root = std::env::temp_dir().join(format!("ta_launcher_bench_aggregate_limits_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("report.json");
        let mut histogram = report(0);
        histogram.histogram = vec![u32::MAX, 1];
        let mut hitches = report(0);
        hitches.parts[0].stats = Stats { frames: 1, hitches: u32::MAX, ..Default::default() };
        hitches.parts.push(Part { stats: Stats { frames: 1, hitches: 1, ..Default::default() }, ..Default::default() });
        for corrupt in [histogram, hitches] {
            std::fs::write(&path, serde_json::to_string(&corrupt).unwrap()).unwrap();
            assert!(read(&path).is_none(), "a report whose chart/summary overflows must be rejected before the page renders it");
        }
        let mut boundary = report(0);
        boundary.histogram = vec![u32::MAX - 1, 1];
        boundary.parts[0].stats = Stats { frames: u32::MAX, hitches: u32::MAX, ..Default::default() };
        std::fs::write(&path, serde_json::to_string(&boundary).unwrap()).unwrap();
        assert!(read(&path).is_some(), "representable aggregate totals remain supported");
        std::fs::remove_dir_all(root).unwrap();
    }

}
