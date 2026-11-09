//! Audits the audio data of the Total Annihilation install and the engine that plays it:
//!
//! 1. decodes every WAV (`sounds/`, `camps/briefs/`, `installres/`) and reports formats,
//!    durations, levels and failures,
//! 2. cross-checks `gamedata/sound.tdf`, `gamedata/allsound.tdf`, unit `SoundCategory=` keys
//!    and weapon sound keys against the files that exist,
//! 3. decodes every music track completely,
//! 4. renders a scripted 12-second battle through the real engine offline (music, merged spam,
//!    distant explosions, a unit response, a loop that goes out of range and comes back, a
//!    victory crossfade) and measures the result.
//!
//! ```text
//! cargo run --release -p ta_audio --example survey -- [--mix out.wav] [--skip-music]
//! ```
//!
//! Exits with status 1 if anything failed to decode.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use ta_audio::decode::{self, MusicDecoder, SourceFormat};
use ta_audio::{
    AudioEngine, AudioSettings, Mood, PlayParams, SoundBank, SoundCategories, SoundEvent, SoundSource, UiSounds,
    WeaponSounds, sound_path,
};
use ta_vfs::Vfs;

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mix_path = args.iter().position(|a| a == "--mix").and_then(|i| args.get(i + 1)).cloned();
    let skip_music = args.iter().any(|a| a == "--skip-music");

    let install = ta_vfs::install::discover(None).context("no Total Annihilation install found")?;
    let mut vfs = Vfs::new();
    vfs.mount_game_dir(&install.root, "base")?;
    let vfs = Arc::new(vfs);
    println!("install: {} ({} files in the VFS)", install.root.display(), vfs.file_count());

    let mut failures = Vec::new();
    survey_sounds(&vfs, &mut failures);
    survey_tables(&vfs)?;
    if !skip_music {
        survey_music(&vfs, &mut failures);
    }
    offline_mix(vfs.clone(), mix_path.as_deref(), &mut failures)?;

    println!("\nfailures: {}", failures.len());
    for f in &failures {
        println!("  {f}");
    }
    if !failures.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn survey_sounds(vfs: &Vfs, failures: &mut Vec<String>) {
    for dir in ["sounds", "camps/briefs", "installres"] {
        let files = vfs.list(dir, Some("wav"), false);
        let mut formats: BTreeMap<String, (usize, f64)> = BTreeMap::new();
        let mut levels = Vec::new();
        let (mut total, mut full_scale, mut pcm_bytes) = (0.0, 0, 0usize);
        let started = Instant::now();
        for path in &files {
            let bytes = match vfs.read(path) {
                Ok(b) => b,
                Err(e) => {
                    failures.push(format!("{path}: {e}"));
                    continue;
                }
            };
            match decode::decode(&bytes) {
                Ok(d) => {
                    let s = &d.sound;
                    let layout = if s.channels() == 1 { "mono" } else { "stereo" };
                    let entry = formats.entry(format!("{}, {} Hz, {layout}", d.format, s.sample_rate())).or_default();
                    entry.0 += 1;
                    entry.1 += s.duration();
                    total += s.duration();
                    pcm_bytes += s.memory_bytes();
                    full_scale += (s.peak() >= 0.99) as usize;
                    levels.push(db(s.rms()));
                    if s.rms() < 1e-4 {
                        failures.push(format!("{path}: decodes to silence"));
                    }
                    if !matches!(d.format, SourceFormat::Wav(_)) {
                        println!("  note: {path} is {} ({} Hz, {:.2} s)", d.format, s.sample_rate(), s.duration());
                    }
                }
                Err(e) => failures.push(format!("{path}: {e}")),
            }
        }
        levels.sort_by(f32::total_cmp);
        println!(
            "\n{dir}/*.wav: {} files, {:.1} s of audio, {:.1} MB as f32, decoded in {:.0} ms",
            files.len(),
            total,
            pcm_bytes as f64 / 1e6,
            started.elapsed().as_secs_f64() * 1000.0
        );
        for (format, (n, secs)) in &formats {
            println!("  {n:>4} × {format} ({secs:.1} s)");
        }
        if let (Some(lo), Some(hi)) = (levels.first(), levels.last()) {
            println!(
                "  RMS level min {lo:.1} / median {:.1} / max {hi:.1} dBFS; {full_scale} files reach full scale",
                levels[levels.len() / 2]
            );
        }
    }
}

fn survey_tables(vfs: &Arc<Vfs>) -> Result<()> {
    let source: &dyn SoundSource = vfs.as_ref();
    let exists = |name: &str| vfs.exists(&sound_path(name));

    let categories = SoundCategories::load(source)?;
    let referenced = categories.referenced_sounds();
    let missing: Vec<&String> = referenced.iter().filter(|n| !exists(n)).collect();
    let mut events: BTreeMap<String, usize> = BTreeMap::new();
    for name in categories.names() {
        for event in categories.category(name).events() {
            *events.entry(event.key()).or_default() += 1;
        }
    }
    println!("\n{}: {} categories, {} distinct sounds", SoundCategories::PATH, categories.len(), referenced.len());
    println!("  events: {}", events.iter().map(|(k, n)| format!("{k}×{n}")).collect::<Vec<_>>().join(" "));
    let missing_list = missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
    println!("  referenced but not shipped ({}): {missing_list}", missing.len());

    let mut none = 0;
    let mut repaired: BTreeMap<String, usize> = BTreeMap::new();
    let mut unresolved: BTreeMap<String, usize> = BTreeMap::new();
    let units = vfs.list("units", Some("fbi"), false);
    for path in &units {
        let doc = ta_formats::tdf::parse_bytes(&vfs.read(path)?);
        let Some(cat) = doc.section("UNITINFO").and_then(|u| u.get("SoundCategory")) else { continue };
        let names = categories.names();
        if cat.eq_ignore_ascii_case("NONE") {
            none += 1;
        } else if categories.try_category(cat).is_none() {
            *unresolved.entry(cat.to_ascii_uppercase()).or_default() += 1;
        } else if !names.contains(&cat.to_ascii_uppercase().as_str()) {
            *repaired.entry(cat.to_ascii_uppercase()).or_default() += 1;
        }
    }
    println!(
        "  {} unit FBIs: {none} use NONE, repaired via Core prefix alias {repaired:?}, unresolved {unresolved:?}",
        units.len()
    );
    let arm_com = categories.category("ARM_COM");
    println!(
        "  ARM_COM: select={:?} ok={:?} build={:?} underattack={:?} count5={:?}",
        arm_com.get(SoundEvent::Select),
        arm_com.get(SoundEvent::Ok),
        arm_com.get(SoundEvent::Build),
        arm_com.get(SoundEvent::UnderAttack),
        arm_com.get(SoundEvent::Count(5))
    );

    let ui = UiSounds::load(source)?;
    let ui_missing: Vec<&str> = ui.iter().filter(|(_, s)| !exists(s)).map(|(k, _)| k).collect();
    println!("\n{}: {} interface events, missing sounds: {ui_missing:?}", UiSounds::PATH, ui.len());

    let weapon_files = vfs.list("weapons", Some("tdf"), false);
    let (mut sections, mut start, mut hit, mut water, mut trigger) = (0, 0, 0, 0, 0);
    let mut weapon_sounds = std::collections::BTreeSet::new();
    for path in &weapon_files {
        let doc = ta_formats::tdf::parse_bytes(&vfs.read(path)?);
        for section in &doc.sections {
            let w = WeaponSounds::from_section(section);
            sections += 1;
            start += w.start.is_some() as usize;
            hit += w.hit.is_some() as usize;
            water += w.water.is_some() as usize;
            trigger += w.trigger as usize;
            weapon_sounds.extend([w.start, w.hit, w.water].into_iter().flatten().map(|s| s.to_ascii_lowercase()));
        }
    }
    let weapon_missing: Vec<&String> = weapon_sounds.iter().filter(|s| !exists(s)).collect();
    println!(
        "weapons/*.tdf: {} files, {sections} weapons: soundstart {start}, soundhit {hit}, soundwater {water}, \
         soundtrigger=1 {trigger}; {} distinct sounds, missing {weapon_missing:?}",
        weapon_files.len(),
        weapon_sounds.len()
    );
    Ok(())
}

fn survey_music(vfs: &Vfs, failures: &mut Vec<String>) {
    let mut files: Vec<String> =
        vfs.list("music", None, false).into_iter().filter(|p| p.ends_with(".mp3") || p.ends_with(".ogg")).collect();
    let number = |p: &str| {
        let stem = p.trim_start_matches("music/").split('.').next().unwrap_or_default();
        stem.parse::<u32>().unwrap_or(u32::MAX)
    };
    files.sort_by_key(|p| number(p));
    println!("\nmusic: {} tracks", files.len());
    let mut total = 0.0;
    let mut by_hash: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    let started = Instant::now();
    for path in &files {
        let bytes = match vfs.read(path) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{path}: {e}"));
                continue;
            }
        };
        let ext = path.rsplit('.').next();
        let estimate = MusicDecoder::open(bytes.clone(), ext)
            .ok()
            .and_then(|d| d.total_frames().map(|n| n as f64 / d.sample_rate() as f64));
        let t0 = Instant::now();
        match decode::decode_music(bytes, ext) {
            Ok(s) => {
                let secs = s.duration();
                total += secs;
                let hash = s
                    .samples()
                    .iter()
                    .fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ v.to_bits() as u64).wrapping_mul(0x0100_0000_01b3));
                by_hash.entry(hash).or_default().push(path.clone());
                println!(
                    "  {path:<15} {:>4}:{:04.1}  {} Hz {}ch  RMS {:>5.1} dBFS  peak {:.2}  \
                     (header estimate {:.1} s)  {:>4.0}× real time",
                    (secs / 60.0) as u32,
                    secs % 60.0,
                    s.sample_rate(),
                    s.channels(),
                    db(s.rms()),
                    s.peak(),
                    estimate.unwrap_or(f64::NAN),
                    secs / t0.elapsed().as_secs_f64()
                );
                if s.rms() < 1e-3 {
                    failures.push(format!("{path}: decodes to near silence"));
                }
            }
            Err(e) => failures.push(format!("{path}: {e}")),
        }
    }
    let duplicates: Vec<String> = by_hash.values().filter(|v| v.len() > 1).map(|v| v.join(" = ")).collect();
    println!(
        "  total {:.1} min decoded in {:.1} s; identical decodes: {}",
        total / 60.0,
        started.elapsed().as_secs_f64(),
        if duplicates.is_empty() { "none".to_string() } else { duplicates.join(", ") }
    );
}

/// Tiny deterministic generator for the scripted scene.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn around(&mut self, c: [f32; 3], r: f32) -> [f32; 3] {
        [c[0] + (self.next() * 2.0 - 1.0) * r, 0.0, c[2] + (self.next() * 2.0 - 1.0) * r]
    }
}

fn offline_mix(vfs: Arc<Vfs>, out_path: Option<&str>, failures: &mut Vec<String>) -> Result<()> {
    const RATE: u32 = 48_000;
    const BLOCK: usize = 480; // 10 ms, like a game frame driving update()
    let seconds = 12.0f32;
    let bank = Arc::new(SoundBank::from_vfs(vfs.clone()));
    // A tight voice budget so the priority rules have to work during the barrage.
    let settings =
        AudioSettings { master: 1.0, music: 0.6, effects: 0.9, voice: 1.0, max_voices: 16, ..Default::default() };
    let mut engine = AudioEngine::new_offline(settings, bank.clone(), RATE);
    let categories = SoundCategories::load(vfs.as_ref())?;
    let center = [2048.0f32, 0.0, 2048.0];
    let north = [0.0, 0.0, -1.0];
    let east = [1.0, 0.0, 0.0];
    engine.set_listener(center, north, east, 1280.0);
    engine.set_mood(Mood::Battle);

    let mut rng = Lcg(42);
    let mut mix: Vec<f32> = Vec::with_capacity((seconds * RATE as f32) as usize * 2);
    let mut buffer = vec![0.0f32; BLOCK * 2];
    let mut fired = [false; 8];
    let mut once = |i: usize, t: f32, at: f32| -> bool {
        if t >= at && !fired[i] {
            fired[i] = true;
            true
        } else {
            false
        }
    };
    let mut hum = None;
    let mut spam = (0, 0, 0);
    let mut most_audible = 0;
    let mut response_audible = false;
    let mut first_track = None;
    let started = Instant::now();
    let blocks = (seconds * RATE as f32) as usize / BLOCK;
    for b in 0..blocks {
        let t = (b * BLOCK) as f32 / RATE as f32;
        if once(0, t, 0.5) {
            engine.play_ui("button2", 1.0);
        }
        if once(1, t, 1.0) {
            // 40 identical laser shots from one squad in one tick, and their scattered impacts.
            let shots: Vec<_> = (0..40).map(|_| engine.play_at("lasrfir3", rng.around(center, 20.0), 0.8)).collect();
            let hits: Vec<_> = (0..40).map(|_| engine.play_at("lasrhit2", rng.around(center, 700.0), 0.8)).collect();
            spam.0 = shots.iter().collect::<std::collections::HashSet<_>>().len();
            spam.1 = hits.iter().filter(|h| !h.is_none()).collect::<std::collections::HashSet<_>>().len();
        }
        if (1.5..4.5).contains(&t) && b % 5 == 0 {
            let name = ["xplomed2", "xplolrg1", "xplosml3"][b % 3];
            let h = engine.play_at(name, rng.around(center, 1800.0), 1.0);
            spam.2 += h.is_none() as usize;
        }
        if once(2, t, 2.0) {
            let h = engine.play_unit_sound(categories.category("ARM_COM"), SoundEvent::Select, center, 1.0);
            response_audible = engine.is_audible(h);
        }
        if once(3, t, 3.0) {
            hum = Some(engine.play("nanlath1", PlayParams::at([center[0] + 250.0, 0.0, center[2]], 0.7).looping(true)));
        }
        if once(4, t, 5.0) {
            engine.set_listener([center[0] + 4000.0, 0.0, center[2]], north, east, 1280.0);
        }
        if once(5, t, 7.0) {
            engine.set_listener(center, north, east, 1280.0);
        }
        if once(6, t, 8.0) {
            first_track = engine.music().now_playing().map(str::to_string);
            engine.set_mood(Mood::Victory);
            if let Some(h) = hum {
                engine.stop(h);
            }
        }
        if once(7, t, 9.0) {
            engine.play_at("xplonuk4", center, 1.0);
        }
        engine.render_offline(&mut buffer);
        mix.extend_from_slice(&buffer);
        engine.update(BLOCK as f32 / RATE as f32);
        most_audible = most_audible.max(engine.stats().audible);
    }
    let render_time = started.elapsed().as_secs_f64();
    let stats = engine.stats();

    let speed = seconds as f64 / render_time;
    println!("\noffline mix: {seconds} s at {RATE} Hz rendered in {render_time:.2} s ({speed:.0}× real time)");
    println!("  music: {} → {}", first_track.as_deref().unwrap_or("-"), engine.music().now_playing().unwrap_or("-"));
    println!(
        "  40 laser shots → {} voice(s); 40 scattered impacts → {} voice(s); distant explosions skipped: {}",
        spam.0, spam.1, spam.2
    );
    println!(
        "  voices: merged {} stolen {} culled {} rejected {} virtualized {} resumed {}; \
         music underrun frames {}; bank holds {} sounds",
        stats.voices.merged,
        stats.voices.stolen,
        stats.voices.culled,
        stats.voices.rejected,
        stats.voices.virtualized,
        stats.voices.resumed,
        stats.music_underrun_frames,
        stats.cached_sounds
    );
    println!(
        "  most sounds audible at once: {most_audible} (limit 16); \
         unit response during the barrage audible: {response_audible}"
    );
    if most_audible > 16 || !response_audible {
        failures.push("offline mix: voice limit or priorities not respected".into());
    }
    println!("  second   RMS L/R (dBFS)   peak");
    let mut overall_peak = 0.0f32;
    for (s, window) in mix.chunks(RATE as usize * 2).enumerate() {
        let side = |c: usize| {
            let v: Vec<f32> = window.iter().skip(c).step_by(2).copied().collect();
            ((v.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt()) as f32
        };
        let peak = window.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        overall_peak = overall_peak.max(peak);
        println!("  {s:>4}     {:>6.1} / {:>6.1}      {peak:.3}", db(side(0)), db(side(1)));
        if side(0) < 1e-3 {
            failures.push(format!("offline mix: second {s} is silent"));
        }
    }
    if overall_peak > 1.0 {
        failures.push(format!("offline mix clips (peak {overall_peak})"));
    }
    if stats.music_underrun_frames > 0 {
        failures.push(format!("offline mix: {} music underrun frames", stats.music_underrun_frames));
    }
    if let Some(path) = out_path {
        write_wav(path, RATE, &mix)?;
        println!("  wrote {path}");
    }
    Ok(())
}

/// Writes interleaved stereo `f32` as 16-bit PCM WAV.
fn write_wav(path: &str, rate: u32, samples: &[f32]) -> Result<()> {
    let data: Vec<u8> =
        samples.iter().flat_map(|&s| ((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()).collect();
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::write(path, out)?;
    Ok(())
}
