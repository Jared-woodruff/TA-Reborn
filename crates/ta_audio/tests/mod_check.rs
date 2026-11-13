//! The mod check judges a sound as the game's decoder reads it
//! (`ta_data::validation::sound_problem`): what one plays, the other passes.

/// A RIFF WAVE file of one format: its tag, channels and bits, and `data` bytes of samples.
fn wav(tag: u16, channels: u16, bits: u16, data: &[u8]) -> Vec<u8> {
    let mut b = b"RIFF".to_vec();
    b.extend((36 + data.len() as u32).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(tag.to_le_bytes());
    b.extend(channels.to_le_bytes());
    b.extend(11_025u32.to_le_bytes());
    let align = (channels * bits.div_ceil(8)).max(1);
    b.extend((11_025u32 * align as u32).to_le_bytes());
    b.extend(align.to_le_bytes());
    b.extend(bits.to_le_bytes());
    b.extend(b"data");
    b.extend((data.len() as u32).to_le_bytes());
    b.extend(data);
    b
}

#[test]
fn the_mod_check_judges_sounds_as_the_decoder_reads_them() {
    let samples = [0u8, 64, 128, 192, 255, 128, 64, 0];
    let tone: Vec<u8> = (0..2000).map(|i| (128.0 + 90.0 * (i as f64 / 20.0).sin()) as u8).collect();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("16-bit PCM", wav(1, 1, 16, &samples)),
        ("8-bit stereo PCM", wav(1, 2, 8, &samples)),
        ("32-bit float", wav(3, 1, 32, &samples)),
        ("A-law", wav(6, 1, 8, &samples)),
        ("µ-law", wav(7, 1, 8, &samples)),
        ("40-bit PCM", wav(1, 1, 40, &samples)),
        ("3-bit IMA ADPCM", wav(0x11, 1, 3, &samples)),
        ("no channels", wav(1, 0, 16, &samples)),
        ("33 channels", wav(1, 33, 8, &samples)),
        ("MP3 in a WAV", wav(0x55, 1, 16, &samples)),
        ("headerless 8-bit samples", tone),
        ("Ogg Vorbis", std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/tone_440hz_quarter_second.ogg")).unwrap()),
        ("an Ogg header alone", b"OggS".to_vec()),
        ("a DIGI block without its samples", b"DIGI\0\0\0\x10HSHD\0\0\0\x08".to_vec()),
        ("a DIGI block with its samples", [b"DIGI\0\0\0\x1cHSHD\0\0\0\x08SDAT\0\0\0\x0c".as_slice(), &[128u8; 4]].concat()),
        ("a DIGI block cut off before its samples", b"DIGI\0\0\0\x1cHSHD\0\0\0\x08SDAT".to_vec()),
        ("a TALK block without its samples", b"TALK\0\0\0\x08".to_vec()),
        ("an Ogg page of an Opus stream", [b"OggS\0\x02".as_slice(), &[0; 20], &[1, 9], b"OpusHead\x01"].concat()),
        ("words", b"just some words, not a sound at all".to_vec()),
    ];
    for (what, bytes) in cases {
        let plays = ta_audio::decode::decode(&bytes).is_ok();
        let passes = ta_data::validation::sound_problem(&bytes).is_none();
        assert_eq!(passes, plays, "{what}: the decoder {} it, the check {}", if plays { "plays" } else { "refuses" }, if passes { "passes it" } else { "refuses it" });
    }
}
