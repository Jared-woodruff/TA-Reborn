//! Stock startup/parse benchmark without a GPU or copied game assets.
//! Run: cargo run --release -p ta_data --example load_benchmark -- [game-dir] [rounds]
use std::hint::black_box;
use std::time::Instant;

use sha2::{Digest, Sha256};
use ta_data::{GameData, compatibility_fingerprint};
use ta_formats::tdf;
use ta_vfs::Vfs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = match std::env::args().nth(1) {
        Some(path) => std::path::PathBuf::from(path),
        None => ta_vfs::install::discover(None)?.root,
    };
    let rounds: usize = std::env::args().nth(2).map(|n| n.parse()).transpose()?.unwrap_or(5);
    let start = Instant::now();
    let mut vfs = Vfs::new();
    vfs.mount_game_dir(&root, "base")?;
    println!("mount_ms={:.3} files={}", start.elapsed().as_secs_f64() * 1000.0, vfs.file_count());
    let text: Vec<_> = vfs.list("", None, true).into_iter()
        .filter(|p| p.rsplit_once('.').is_some_and(|(_, e)| ["tdf", "fbi", "gui", "ota"].contains(&e)))
        .map(|p| vfs.read(&p)).collect::<Result<_, _>>()?;
    println!("tdf_files={} input_bytes={}", text.len(), text.iter().map(Vec::len).sum::<usize>());
    // Structural comparison is outside the measured parse loops.
    let mut text_hash = Sha256::new();
    for bytes in &text {
        text_hash.update(format!("{:?}", tdf::parse_bytes(bytes)));
    }
    println!("tdf_sha256={:x}", text_hash.finalize());
    for round in 0..rounds {
        let start = Instant::now();
        let fingerprint = compatibility_fingerprint(&vfs)?;
        let fingerprint_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let data = GameData::load(&vfs);
        let definitions_ms = start.elapsed().as_secs_f64() * 1000.0;
        let definition_hash = data.fingerprint();
        let start = Instant::now();
        for bytes in &text { black_box(tdf::parse_bytes(black_box(bytes))); }
        let tdf_ms = start.elapsed().as_secs_f64() * 1000.0;
        println!("round={round} fingerprint_ms={fingerprint_ms:.3} definitions_ms={definitions_ms:.3} tdf_ms={tdf_ms:.3} fingerprint={fingerprint:016x} definitions={definition_hash:016x}");
    }
    Ok(())
}
