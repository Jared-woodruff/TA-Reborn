//! Tests against the user's Total Annihilation install. Each test is skipped (passes with a
//! note) when no install is found, so the suite also runs on machines without the game.

use std::sync::Arc;

use ta_audio::decode::{self, MusicDecoder, SourceFormat};
use ta_audio::{
    AudioEngine, AudioSettings, Mood, PlayParams, SoundBank, SoundCategories, SoundEvent, SoundSource, UiSounds,
    WeaponSounds, sound_path,
};
use ta_vfs::Vfs;

fn vfs() -> Option<Arc<Vfs>> {
    let install = match ta_vfs::install::discover(None) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("skipping: {e}");
            return None;
        }
    };
    let mut vfs = Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    Some(Arc::new(vfs))
}

#[test]
fn every_wav_decodes_to_sound() {
    let Some(vfs) = vfs() else { return };
    let mut failures = Vec::new();
    let mut total = 0;
    for dir in ["sounds", "camps/briefs", "installres"] {
        for path in vfs.list(dir, Some("wav"), false) {
            total += 1;
            match decode::decode(&vfs.read(&path).unwrap()) {
                Ok(d) if d.sound.frames() > 0 && d.sound.rms() > 1e-4 => {}
                Ok(_) => failures.push(format!("{path}: silent")),
                Err(e) => failures.push(format!("{path}: {e}")),
            }
        }
    }
    assert!(total >= 461 + 79 + 4, "only {total} WAV files found");
    assert!(failures.is_empty(), "{failures:#?}");

    // The two non-RIFF oddities in sounds/.
    let sing = decode::decode(&vfs.read("sounds/sing.wav").unwrap()).unwrap();
    assert_eq!((sing.format, sing.sound.sample_rate()), (SourceFormat::HumongousDigi, 11_025));
    let honk = decode::decode(&vfs.read("sounds/honk.wav").unwrap()).unwrap();
    assert_eq!(honk.format, SourceFormat::RawPcm8);
}

#[test]
fn sound_tables_resolve_against_the_data() {
    let Some(vfs) = vfs() else { return };
    let source: &dyn SoundSource = vfs.as_ref();
    let categories = SoundCategories::load(source).unwrap();
    assert_eq!(categories.len(), 120);
    let com = categories.category("ARM_COM");
    assert_eq!(com.get(SoundEvent::Select), Some("kcarmsel"));
    assert_eq!(com.get(SoundEvent::Build), Some("nanlath1"));
    assert_eq!(com.get(SoundEvent::Count(0)), Some("count6"));

    // Every unit's SoundCategory resolves (TA's CORE_/COR_ mix-ups included) except NONE.
    for path in vfs.list("units", Some("fbi"), false) {
        let doc = ta_formats::tdf::parse_bytes(&vfs.read(&path).unwrap());
        let Some(cat) = doc.section("UNITINFO").and_then(|u| u.get("SoundCategory")) else { continue };
        assert!(cat.eq_ignore_ascii_case("NONE") || categories.try_category(cat).is_some(), "{path}: {cat}");
    }

    let ui = UiSounds::load(source).unwrap();
    assert!(ui.len() >= 40);
    for (event, sound) in ui.iter() {
        assert!(vfs.exists(&sound_path(sound)), "{event}: {sound}");
    }

    let mut checked = 0;
    for path in vfs.list("weapons", Some("tdf"), false) {
        let doc = ta_formats::tdf::parse_bytes(&vfs.read(&path).unwrap());
        for section in &doc.sections {
            let w = WeaponSounds::from_section(section);
            for sound in [&w.start, &w.hit, &w.water].into_iter().flatten() {
                assert!(vfs.exists(&sound_path(sound)), "{path} [{}]: {sound}", section.name);
                checked += 1;
            }
        }
    }
    assert!(checked > 380, "{checked} weapon sound references");
}

#[test]
fn music_tracks_open_and_stream() {
    let Some(vfs) = vfs() else { return };
    let tracks: Vec<String> = vfs.list("music", Some("mp3"), false);
    assert_eq!(tracks.len(), 18);
    for path in &tracks {
        let mut decoder = MusicDecoder::open(vfs.read(path).unwrap(), Some("mp3")).unwrap();
        assert_eq!(decoder.sample_rate(), 44_100, "{path}");
        let seconds = decoder.total_frames().unwrap() as f64 / 44_100.0;
        assert!((60.0..240.0).contains(&seconds), "{path}: {seconds} s");
        // Decode the first five seconds and check it is music, not silence or noise.
        let mut frames = Vec::new();
        while frames.len() < 5 * 44_100 && decoder.next_frames(&mut frames).unwrap() {}
        let energy: f64 = frames.iter().map(|f| (f[0] as f64).powi(2) + (f[1] as f64).powi(2)).sum();
        let rms = (energy / (2.0 * frames.len() as f64)).sqrt();
        assert!(rms > 0.003 && rms < 0.5, "{path}: rms {rms}");
    }
}

#[test]
fn offline_engine_mixes_real_sounds_and_music() {
    let Some(vfs) = vfs() else { return };
    const RATE: u32 = 44_100;
    let bank = Arc::new(SoundBank::from_vfs(vfs));
    let settings = AudioSettings { master: 1.0, music: 0.5, effects: 1.0, voice: 1.0, ..Default::default() };
    let mut engine = AudioEngine::new_offline(settings, bank, RATE);
    engine.set_listener([1000.0, 0.0, 1000.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], 1280.0);
    assert!(engine.music_mut().play_track("3"));
    engine.update(0.0);
    let boom = engine.play("xplolrg1", PlayParams::at([1200.0, 0.0, 1000.0], 1.0));
    let ack = engine.play_voice("kcarmsel", 1.0);
    assert!(!boom.is_none() && !ack.is_none());

    let mut out = vec![0.0f32; 2 * RATE as usize * 3];
    for block in out.chunks_mut(2 * 441) {
        engine.render_offline(block);
        engine.update(441.0 / RATE as f32);
    }
    let rms = |x: &[f32]| (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt() as f32;
    let second = |s: usize| &out[s * 2 * RATE as usize..(s + 1) * 2 * RATE as usize];
    // Explosion plus music is loud; music alone keeps playing after the effects end.
    assert!(rms(second(0)) > 0.05, "first second rms {}", rms(second(0)));
    assert!(rms(second(2)) > 0.01, "music continues: {}", rms(second(2)));
    assert!(out.iter().all(|s| s.abs() <= 1.0));
    assert_eq!(engine.music().now_playing(), Some("music/3.mp3"));
    assert_eq!(engine.music().mood(), Some(Mood::Briefing));
    let stats = engine.stats();
    assert_eq!(stats.music_underrun_frames, 0);
    assert!(!engine.is_playing(ack), "the acknowledgement has ended");
}
