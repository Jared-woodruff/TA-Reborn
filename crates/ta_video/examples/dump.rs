//! Decodes Smacker movies end to end and dumps sample frames and audio for inspection.
//!
//! ```text
//! cargo run -p ta_video --release --example dump -- [FILE.zrb ...] [--out DIR]
//!     [--every N] [--sheet N]
//! ```
//!
//! Without file arguments every `.zrb` in the install's `Data` directory is processed. The
//! install is taken from `TA_REBORN_GAME_DIR` or the default GOG location; if neither
//! exists the example says so and exits successfully.
//!
//! For each movie it prints the header information, decodes every frame, writes frames 0,
//! 100, 300 and the last one (plus every Nth frame with `--every N`) as display-size PNGs
//! and writes audio track 0 as a 16-bit WAV, reporting duration and simple signal
//! statistics. `--sheet N` also writes a contact sheet of quarter-size thumbnails of every
//! Nth frame. Output goes to `scratch/video/` in the workspace unless `--out` is given.
//! Set `RUST_LOG=ta_video=debug` to see decoder diagnostics.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ta_video::{SmackerVideo, VideoFrame};

const DEFAULT_GAME_DIR: &str = r"C:\GOG Games\Total Annihilation";
const GAME_DIR_ENV: &str = "TA_REBORN_GAME_DIR";

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let mut files = Vec::new();
    let mut out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch/video");
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => match args.next() {
                Some(dir) => out_dir = PathBuf::from(dir),
                None => return usage("--out needs a directory"),
            },
            "--every" | "--sheet" => {
                let Some(n) = args.next().and_then(|n| n.parse::<usize>().ok()).filter(|&n| n > 0)
                else {
                    return usage(&format!("{arg} needs a positive frame interval"));
                };
                if arg == "--every" {
                    options.every = Some(n);
                } else {
                    options.sheet = Some(n);
                }
            }
            _ => files.push(PathBuf::from(arg)),
        }
    }

    if files.is_empty() {
        files = install_movies();
        if files.is_empty() {
            println!(
                "No Total Annihilation install with .zrb movies found (set {GAME_DIR_ENV}); \
                 nothing to do."
            );
            return ExitCode::SUCCESS;
        }
    }
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("cannot create {}: {e}", out_dir.display());
        return ExitCode::FAILURE;
    }
    let out_dir = std::path::absolute(&out_dir).unwrap_or(out_dir);

    let mut failed = 0;
    for file in &files {
        match dump(file, &out_dir, &options) {
            Ok(0) => {}
            Ok(errors) => {
                eprintln!("{}: {errors} frame(s) failed to decode", file.display());
                failed += 1;
            }
            Err(e) => {
                eprintln!("{}: {e}", file.display());
                failed += 1;
            }
        }
    }
    println!("\nOutput written to {}", out_dir.display());
    if failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

#[derive(Default)]
struct Options {
    /// Also save every Nth frame as a full-size PNG.
    every: Option<usize>,
    /// Write a contact sheet with every Nth frame.
    sheet: Option<usize>,
}

fn usage(problem: &str) -> ExitCode {
    eprintln!("{problem}\nusage: dump [FILE.zrb ...] [--out DIR] [--every N] [--sheet N]");
    ExitCode::FAILURE
}

/// `.zrb` files in the install's `Data` directory, sorted by name.
fn install_movies() -> Vec<PathBuf> {
    let root = std::env::var_os(GAME_DIR_ENV)
        .or_else(|| std::env::var_os("TA_REMASTERED_GAME_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    let entries = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default()
    };
    let has_name = |p: &Path, name: &str| {
        p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
    };
    let Some(data) = entries(&root).into_iter().find(|p| p.is_dir() && has_name(p, "data")) else {
        return Vec::new();
    };
    let mut movies: Vec<PathBuf> = entries(&data)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zrb")))
        .collect();
    movies.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
    movies
}

/// Decodes one movie; returns the number of frames that failed.
fn dump(path: &Path, out_dir: &Path, options: &Options) -> Result<usize, Box<dyn Error>> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    let mut video = SmackerVideo::open_file(path)?;
    let header = video.header();
    println!("\n== {}", path.display());
    println!(
        "  {:?}  {}x{} stored, {}x{} displayed ({:?}), flags {:#x}, ring frame: {}",
        header.version,
        video.width(),
        video.height(),
        video.width(),
        video.display_height(),
        video.y_scale(),
        header.flags,
        header.has_ring_frame(),
    );
    println!(
        "  {} frames × {:.3} ms (rate field {}) = {:.3} fps, running time {:.2} s",
        video.frame_count(),
        video.frame_duration().as_secs_f64() * 1000.0,
        header.frame_rate,
        video.frame_rate(),
        video.duration().as_secs_f64(),
    );
    for t in video.audio_tracks() {
        println!(
            "  audio track {}: {:?}, {} Hz, {} channel(s), {}-bit source, largest chunk {} bytes",
            t.track, t.codec, t.sample_rate, t.channels, t.bits_per_sample, t.max_chunk_bytes
        );
    }

    let last = video.frame_count().saturating_sub(1);
    let wanted = |i: usize| {
        let extra = options.every.is_some_and(|n| i.is_multiple_of(n));
        matches!(i, 0 | 100 | 300) || i == last || extra
    };
    let mut sheet =
        options.sheet.map(|n| ContactSheet::new(n, video.width(), video.display_height()));
    let mut track0: Vec<i16> = Vec::new();
    let mut chunk_lengths: Vec<usize> = Vec::new();
    let mut errors = 0usize;
    let mut decode_time = Duration::ZERO;
    let mut palette_changes = 0usize;
    let mut saved = Vec::new();
    loop {
        let started = Instant::now();
        let result = video.next_frame();
        decode_time += started.elapsed();
        match result {
            Ok(Some(frame)) => {
                palette_changes += usize::from(frame.palette_changed);
                if let Some(audio) = frame.audio.first() {
                    track0.extend_from_slice(audio);
                    chunk_lengths.push(audio.len());
                }
                if wanted(frame.index) {
                    let file = out_dir.join(format!("{stem}_frame{:04}.png", frame.index));
                    save_png(&frame, &file)?;
                    saved.push(frame.index);
                }
                if let Some(sheet) = &mut sheet {
                    sheet.offer(&frame);
                }
            }
            Ok(None) => break,
            Err(e) => {
                errors += 1;
                eprintln!("  error: {e}");
            }
        }
    }
    let decoded = video.position();
    println!(
        "  decoded {decoded} frames in {:.2} s ({:.2} ms/frame), {errors} error(s), \
         {palette_changes} palette change(s)",
        decode_time.as_secs_f64(),
        decode_time.as_secs_f64() * 1000.0 / decoded.max(1) as f64,
    );
    println!("  saved frames {saved:?} as {stem}_frameNNNN.png");
    if let Some(sheet) = sheet {
        let file = out_dir.join(format!("{stem}_sheet.png"));
        let count = sheet.thumbnails.len();
        sheet.save(&file)?;
        println!("  contact sheet of {count} frames: {}", file.display());
    }

    if let Some(track) = video.audio_tracks().first() {
        let channels = usize::from(track.channels);
        let per_channel = track0.len() / channels;
        let audio_secs = per_channel as f64 / f64::from(track.sample_rate);
        let with_audio: Vec<usize> = chunk_lengths.iter().copied().filter(|&n| n > 0).collect();
        println!(
            "  track 0: {} samples/channel = {:.3} s (video {:.3} s, difference {:+.3} s)",
            per_channel,
            audio_secs,
            video.duration().as_secs_f64(),
            audio_secs - video.duration().as_secs_f64(),
        );
        println!(
            "  track 0 chunks: {} frames carry audio; first chunk {} samples, \
             later chunks {}..={} samples",
            with_audio.len(),
            with_audio.first().copied().unwrap_or(0) / channels,
            with_audio.iter().skip(1).min().copied().unwrap_or(0) / channels,
            with_audio.iter().skip(1).max().copied().unwrap_or(0) / channels,
        );
        for ch in 0..channels {
            let s = signal_stats(track0.iter().skip(ch).step_by(channels).copied());
            let (inside, boundary) = step_sizes(&track0, &chunk_lengths, channels, ch);
            println!(
                "  channel {ch}: RMS {:.1} dBFS, peak {:.1} dBFS, DC {:+.0}, \
                 zero crossings {:.4}/sample ({:.0}/s), lag-1 autocorrelation {:.3}",
                s.rms_dbfs,
                s.peak_dbfs,
                s.mean,
                s.zero_crossing_rate,
                s.zero_crossing_rate * f64::from(track.sample_rate),
                s.lag1_autocorrelation,
            );
            println!(
                "             mean |step| inside chunks {inside:.1}, \
                 across chunk boundaries {boundary:.1}"
            );
        }
        let wav = out_dir.join(format!("{stem}_track0.wav"));
        write_wav(&wav, &track0, track.sample_rate, track.channels)?;
        println!("  wrote {}", wav.display());
    }
    Ok(errors)
}

fn save_png(frame: &VideoFrame, path: &Path) -> Result<(), Box<dyn Error>> {
    let image = image::RgbaImage::from_raw(frame.width, frame.display_height(), frame.to_rgba())
        .ok_or("frame buffer has the wrong size")?;
    image.save(path)?;
    Ok(())
}

/// Quarter-size thumbnails of every Nth frame laid out in a grid.
struct ContactSheet {
    interval: usize,
    thumb_w: u32,
    thumb_h: u32,
    thumbnails: Vec<image::RgbaImage>,
}

impl ContactSheet {
    const SCALE: u32 = 4;
    const COLUMNS: u32 = 8;

    fn new(interval: usize, width: u32, display_height: u32) -> Self {
        ContactSheet {
            interval,
            thumb_w: (width / Self::SCALE).max(1),
            thumb_h: (display_height / Self::SCALE).max(1),
            thumbnails: Vec::new(),
        }
    }

    fn offer(&mut self, frame: &VideoFrame) {
        if !frame.index.is_multiple_of(self.interval) {
            return;
        }
        let rgba = frame.to_rgba();
        let stride = frame.width as usize * 4;
        let s = Self::SCALE as usize;
        // Box filter: average each SCALE×SCALE cell.
        let thumb = image::RgbaImage::from_fn(self.thumb_w, self.thumb_h, |x, y| {
            let mut sum = [0u32; 4];
            for dy in 0..s {
                let row = (y as usize * s + dy) * stride;
                for dx in 0..s {
                    let at = row + (x as usize * s + dx) * 4;
                    for (c, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(rgba[at + c]);
                    }
                }
            }
            image::Rgba(sum.map(|t| (t / (s * s) as u32) as u8))
        });
        self.thumbnails.push(thumb);
    }

    fn save(self, path: &Path) -> Result<(), Box<dyn Error>> {
        let count = self.thumbnails.len() as u32;
        let columns = count.clamp(1, Self::COLUMNS);
        let rows = count.div_ceil(columns).max(1);
        let gap = 2;
        let mut sheet = image::RgbaImage::from_pixel(
            columns * (self.thumb_w + gap),
            rows * (self.thumb_h + gap),
            image::Rgba([40, 40, 40, 255]),
        );
        for (i, thumb) in self.thumbnails.iter().enumerate() {
            let (col, row) = (i as u32 % columns, i as u32 / columns);
            image::imageops::replace(
                &mut sheet,
                thumb,
                i64::from(col * (self.thumb_w + gap)),
                i64::from(row * (self.thumb_h + gap)),
            );
        }
        sheet.save(path)?;
        Ok(())
    }
}

struct SignalStats {
    rms_dbfs: f64,
    peak_dbfs: f64,
    mean: f64,
    zero_crossing_rate: f64,
    lag1_autocorrelation: f64,
}

fn signal_stats(samples: impl Iterator<Item = i16>) -> SignalStats {
    let x: Vec<f64> = samples.map(f64::from).collect();
    let n = x.len().max(1) as f64;
    let mean = x.iter().sum::<f64>() / n;
    let rms = (x.iter().map(|v| v * v).sum::<f64>() / n).sqrt();
    let peak = x.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let centred: Vec<f64> = x.iter().map(|v| v - mean).collect();
    let crossings = centred.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    let variance: f64 = centred.iter().map(|v| v * v).sum();
    let lag1: f64 = centred.windows(2).map(|w| w[0] * w[1]).sum();
    let dbfs = |v: f64| 20.0 * (v.max(1e-9) / 32768.0).log10();
    SignalStats {
        rms_dbfs: dbfs(rms),
        peak_dbfs: dbfs(peak),
        mean,
        zero_crossing_rate: crossings as f64 / n,
        lag1_autocorrelation: if variance > 0.0 { lag1 / variance } else { 0.0 },
    }
}

/// Mean absolute sample-to-sample step of channel `ch` inside chunks and across chunk
/// boundaries. Every chunk restarts its DPCM chain from stored absolute samples, so a
/// decoding error shows up as boundary steps far larger than the steps inside chunks.
fn step_sizes(samples: &[i16], chunk_lengths: &[usize], channels: usize, ch: usize) -> (f64, f64) {
    let (mut inside, mut inside_n, mut boundary, mut boundary_n) = (0.0, 0usize, 0.0, 0usize);
    let mut start = 0usize;
    let mut previous_last: Option<i16> = None;
    for &len in chunk_lengths.iter().filter(|&&n| n > 0) {
        let chunk: Vec<i16> =
            samples[start..start + len].iter().skip(ch).step_by(channels).copied().collect();
        start += len;
        if let (Some(last), Some(&first)) = (previous_last, chunk.first()) {
            boundary += f64::from((i32::from(first) - i32::from(last)).abs());
            boundary_n += 1;
        }
        for w in chunk.windows(2) {
            inside += f64::from((i32::from(w[1]) - i32::from(w[0])).abs());
            inside_n += 1;
        }
        previous_last = chunk.last().copied();
    }
    (inside / inside_n.max(1) as f64, boundary / boundary_n.max(1) as f64)
}

/// Writes interleaved 16-bit PCM as a canonical RIFF/WAVE file.
fn write_wav(path: &Path, samples: &[i16], rate: u32, channels: u16) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let block_align = channels * 2;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out)
}
