//! End-to-end decoding of the original game's movies, when an install is available.
//!
//! The install is located through `TA_REBORN_GAME_DIR` or the default GOG folder; when
//! neither holds any `.zrb` movies the test only prints a note. Every movie must decode
//! completely, its audio must last as long as its video, and rewinding must reproduce the
//! same frames. For files matching the GOG release byte for byte in size, fingerprints of
//! selected frames and of the whole audio track (taken from output that was checked by
//! eye and ear) guard against regressions.

use std::path::{Path, PathBuf};

use ta_video::SmackerVideo;

const DEFAULT_GAME_DIR: &str = r"C:\GOG Games\Total Annihilation";
const GAME_DIR_ENV: &str = "TA_REBORN_GAME_DIR";

/// Frames whose fingerprints are compared.
const PROBE_FRAMES: [usize; 4] = [0, 100, 300, 500];

struct Release {
    name: &'static str,
    bytes: u64,
    frames: usize,
    /// Width and display height.
    size: (u32, u32),
    /// Fingerprints of `PROBE_FRAMES` (pixels and palette).
    frame_hashes: [u64; 4],
    /// Fingerprint of all decoded track-0 samples.
    audio_hash: u64,
}

/// The movies of the GOG "Commander Pack" release.
const GOG_RELEASE: &[Release] = &[
    Release {
        name: "1.zrb", // Cavedog logo
        bytes: 6_919_100,
        frames: 599,
        size: (640, 480),
        frame_hashes: [
            0xd1c999760377a28b,
            0xb5838a0203059290,
            0xc3dfff339b94ea82,
            0x4f6c3f6e2c0096b0,
        ],
        audio_hash: 0x0b1ca2b986debdb1,
    },
    Release {
        name: "2.zrb", // intro
        bytes: 41_229_000,
        frames: 4058,
        size: (640, 480),
        frame_hashes: [
            0x975238d10d994800,
            0xea53da5533852955,
            0x3afd6fc70e38d281,
            0xebc756faf0fdb914,
        ],
        audio_hash: 0xa6b5f2a53814f8ae,
    },
    Release {
        name: "3.zrb",
        bytes: 9_596_608,
        frames: 889,
        size: (640, 480),
        frame_hashes: [
            0x0c3efed46a514654,
            0x9cb40d4931d0d510,
            0x6f3eb935df66a38e,
            0xdf4bbc9206e3b473,
        ],
        audio_hash: 0xb1769902425950b5,
    },
    Release {
        name: "4.zrb",
        bytes: 17_387_220,
        frames: 1841,
        size: (640, 480),
        frame_hashes: [
            0x921254bf67691ab6,
            0x5585167303e0a903,
            0xc65235cc24e55e77,
            0xb71f5fe0ed9cc7e4,
        ],
        audio_hash: 0xaca60efc8dbda27f,
    },
    Release {
        name: "5.zrb", // credits
        bytes: 34_875_360,
        frames: 4020,
        size: (640, 304),
        frame_hashes: [
            0x3e364046e68734c6,
            0x0e35d12e2177ee4e,
            0x1096a8ae4ec761df,
            0xe3d766eef90aa9e1,
        ],
        audio_hash: 0x18083416a54b57e0,
    },
];

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: impl IntoIterator<Item = u8>) {
        for b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

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
    let is_data = |p: &PathBuf| {
        p.is_dir() && p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("data"))
    };
    let Some(data) = entries(&root).into_iter().find(is_data) else {
        return Vec::new();
    };
    let mut movies: Vec<PathBuf> = entries(&data)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zrb")))
        .collect();
    movies.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
    movies
}

#[test]
fn install_movies_decode_completely() {
    let movies = install_movies();
    if movies.is_empty() {
        eprintln!("no Total Annihilation movies found (set {GAME_DIR_ENV}); skipping");
        return;
    }
    for path in &movies {
        check_movie(path);
    }
}

fn check_movie(path: &Path) {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let bytes = std::fs::read(path).unwrap();
    let file_len = bytes.len() as u64;
    let mut video = SmackerVideo::open(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    let tracks = video.audio_tracks().to_vec();
    assert!(video.frame_count() > 0, "{name}: no frames");

    let mut frame_hashes = [0u64; 4];
    let mut audio_hash = Fnv::new();
    let mut samples = vec![0usize; tracks.len()];
    let mut most_colours = 0;
    while let Some(frame) = video.next_frame().unwrap_or_else(|e| panic!("{name}: {e}")) {
        for (total, pcm) in samples.iter_mut().zip(&frame.audio) {
            *total += pcm.len();
        }
        if let Some(pcm) = frame.audio.first() {
            audio_hash.write(pcm.iter().flat_map(|s| s.to_le_bytes()));
        }
        if let Some(slot) = PROBE_FRAMES.iter().position(|&p| p == frame.index) {
            frame_hashes[slot] = fingerprint(&frame);
        }
        if frame.index.is_multiple_of(50) {
            let mut used = [false; 256];
            for &p in &frame.pixels {
                used[usize::from(p)] = true;
            }
            most_colours = most_colours.max(used.iter().filter(|&&u| u).count());
        }
    }
    assert_eq!(video.position(), video.frame_count());
    assert!(most_colours >= 32, "{name}: pictures never use more than {most_colours} colours");

    let video_secs = video.duration().as_secs_f64();
    for (track, &count) in tracks.iter().zip(&samples) {
        if !track.is_decodable() {
            continue;
        }
        let secs = count as f64 / f64::from(track.channels) / f64::from(track.sample_rate);
        assert!(
            (secs - video_secs).abs() < 0.1,
            "{name}: track {} lasts {secs:.3} s but the video {video_secs:.3} s",
            track.track
        );
    }

    // Rewinding restarts from a clean state and reproduces the same pictures.
    video.rewind();
    let replay = (0..=PROBE_FRAMES[1]).filter_map(|_| video.next_frame().unwrap()).last();
    if let Some(frame) = replay.filter(|f| f.index == PROBE_FRAMES[1]) {
        assert_eq!(fingerprint(&frame), frame_hashes[1], "{name}: rewind changed frame 100");
    }

    let audio_hash = audio_hash.0;
    println!(
        "{name}: {file_len} bytes, {} frames, {}x{}, \
         frames {frame_hashes:016x?}, audio {audio_hash:016x}",
        video.frame_count(),
        video.width(),
        video.display_height(),
    );
    let release = GOG_RELEASE
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(&name) && r.bytes == file_len);
    match release {
        Some(r) => {
            assert_eq!(video.frame_count(), r.frames, "{name}: frame count");
            assert_eq!((video.width(), video.display_height()), r.size, "{name}: size");
            assert_eq!(frame_hashes, r.frame_hashes, "{name}: frame fingerprints");
            assert_eq!(audio_hash, r.audio_hash, "{name}: audio fingerprint");
        }
        None => println!("{name}: not a known GOG file; fingerprints not compared"),
    }
}

/// Randomly damaged copies of a real movie must decode to errors or garbage, never panic,
/// and the decoder must carry on after a damaged frame.
#[test]
fn damaged_install_movie_never_panics() {
    let smallest = install_movies().into_iter().min_by_key(|p| p.metadata().map_or(0, |m| m.len()));
    let Some(path) = smallest else {
        eprintln!("no Total Annihilation movies found (set {GAME_DIR_ENV}); skipping");
        return;
    };
    let original = std::fs::read(&path).unwrap();
    let clean = SmackerVideo::open(original.clone()).unwrap();
    let header = clean.header();
    let decoded = clean.frame_count().min(150);
    let table_count = clean.frame_count() + usize::from(header.has_ring_frame());
    let trees_start = ta_video::SmackerHeader::LEN + 5 * table_count;
    let trees_end = trees_start + header.trees_size as usize;
    let frames_end = trees_end + (original.len() - trees_end) * decoded / clean.frame_count();
    // Damage the header and frame table, the Huffman trees, or the decoded frames.
    let regions = [(0, trees_start), (trees_start, trees_end), (trees_end, frames_end)];

    // xorshift64*: deterministic and dependency-free.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut random = move |bound: usize| {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % bound
    };
    for variant in 0..12 {
        let (start, end) = regions[variant % regions.len()];
        let mut data = original.clone();
        for _ in 0..1 + random(24) {
            data[start + random(end - start)] ^= 1 << random(8);
        }
        let Ok(mut video) = SmackerVideo::open(data) else {
            println!("variant {variant}: rejected when opened");
            continue;
        };
        let mut damaged = 0;
        for _ in 0..decoded {
            match video.next_frame() {
                Ok(Some(frame)) => assert!(frame.to_rgba().len().is_multiple_of(4)),
                Ok(None) => break,
                Err(_) => damaged += 1,
            }
        }
        assert!(video.position() == decoded || video.is_finished(), "decoder stopped early");
        println!("variant {variant}: {damaged} of {decoded} frames reported damage");
    }
}

fn fingerprint(frame: &ta_video::VideoFrame) -> u64 {
    let mut h = Fnv::new();
    h.write(frame.pixels.iter().copied());
    h.write(frame.palette.iter().flatten().copied());
    h.0
}
