//! Plays a sound and a music track through the default audio device.
//!
//! ```text
//! cargo run --release -p ta_audio --example play -- [SOUND] [--music TRACK] [--seconds N]
//! cargo run --release -p ta_audio --example play -- xplolrg1 --music 2 --seconds 8
//! ```
//!
//! The sound is played at the centre, then panned left and right, then fired 20 times in one
//! frame (the anti-spam rule merges them into one voice). Prints the device and engine stats.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ta_audio::{AudioEngine, AudioSettings, SoundBank};

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut sound = "xplolrg1".to_string();
    let mut music = Some("2".to_string());
    let mut seconds = 6.0f32;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--music" => music = args.next(),
            "--no-music" => music = None,
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).context("--seconds needs a number")?,
            other => sound = other.to_string(),
        }
    }

    let install = ta_vfs::install::discover(None).context("no Total Annihilation install found")?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base")?;
    let bank = Arc::new(SoundBank::from_vfs(Arc::new(vfs)));
    let config = ta_config::Config::load();
    let mut engine = AudioEngine::new(AudioSettings::from(&config.audio), bank.clone())?;
    match engine.output_info() {
        Some(info) => println!("output: {info}"),
        None => println!("silent engine: {}", engine.silent_reason().unwrap_or("unknown")),
    }
    let data = bank.get(&sound).with_context(|| format!("sound '{sound}' not found"))?;
    println!(
        "sound: {sound} ({} Hz, {} ch, {:.2} s, peak {:.2})",
        data.sample_rate(),
        data.channels(),
        data.duration(),
        data.peak()
    );

    if let Some(track) = &music && !engine.music_mut().play_track(track) {
        println!("music track {track} not found");
    }

    // The listener looks at (2048, 0, 2048) with TA's fixed orientation.
    let center = [2048.0f32, 0.0, 2048.0];
    engine.set_listener(center, [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], 1280.0);
    let script: [(f32, &str, [f32; 3]); 3] = [
        (0.3, "centre", center),
        (1.8, "left", [center[0] - 500.0, 0.0, center[2]]),
        (3.3, "right", [center[0] + 500.0, 0.0, center[2]]),
    ];
    let mut next = 0;
    let mut spammed = false;
    let started = Instant::now();
    let mut last = started;
    let mut last_report = 0.0;
    while started.elapsed().as_secs_f32() < seconds {
        let t = started.elapsed().as_secs_f32();
        if next < script.len() && t >= script[next].0 {
            let (_, label, pos) = script[next];
            let handle = engine.play_at(&sound, pos, 1.0);
            println!("{t:5.2} s  play {label}: {handle:?}");
            next += 1;
        }
        if !spammed && t >= 4.8 {
            let handles: std::collections::HashSet<_> =
                (0..20).map(|i| engine.play_at(&sound, [center[0] + i as f32, 0.0, center[2]], 1.0)).collect();
            println!("{t:5.2} s  20 requests in one frame -> {} voice(s)", handles.len());
            spammed = true;
        }
        let now = Instant::now();
        engine.update((now - last).as_secs_f32());
        last = now;
        if t - last_report >= 1.0 {
            last_report = t;
            let s = engine.stats();
            println!(
                "{t:5.2} s  callbacks {} frames {} audible {} mixer voices {} peak {:.2} music {:?} underruns {}",
                s.callbacks,
                s.frames_rendered,
                s.audible,
                s.mixer_voices,
                s.peak,
                engine.music().now_playing(),
                s.music_underrun_frames
            );
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let s = engine.stats();
    println!(
        "done: {} callbacks, {} frames ({:.2} s of audio), merged {}, music underrun frames {}",
        s.callbacks,
        s.frames_rendered,
        s.frames_rendered as f64 / engine.sample_rate().unwrap_or(48_000) as f64,
        s.voices.merged,
        s.music_underrun_frames
    );
    Ok(())
}
