//! The frame-by-frame GAF reader against the real game data: every frame of every GAF of the
//! install, read whole one at a time, is the frame `Gaf::parse` reads; read through
//! (`GafReader::check`) it is sound, and the file's cost decoded whole is what `Gaf::parse`
//! takes. Skipped when no TA install is found.

use ta_formats::gaf::{Gaf, GafReader};
use ta_formats::image::Palette;

fn open_install() -> Option<ta_vfs::Vfs> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    Some(vfs)
}

#[test]
fn every_installed_gaf_reads_frame_by_frame_as_it_reads_whole() {
    let Some(vfs) = open_install() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let palette = vfs.read("palettes/palette.pal").ok().and_then(|b| Palette::parse(&b).ok()).unwrap_or_default();
    let files = vfs.list("", Some("gaf"), true);
    assert!(files.len() > 400, "the install has its animations and textures: {}", files.len());
    let (mut frames, mut read) = (0usize, 0usize);
    for path in &files {
        let Ok(bytes) = vfs.read(path) else { continue };
        let Ok(whole) = Gaf::parse(&bytes) else {
            // (TA's own anims/vismasks.gaf is no GAF.)
            assert!(GafReader::open(&bytes).is_err(), "{path}: the reader refuses it too");
            continue;
        };
        let reader = GafReader::open(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(reader.entries.len(), whole.entries.len(), "{path}");
        let mut costs = Vec::new();
        for (e, w) in reader.entries.iter().zip(&whole.entries) {
            assert_eq!(e.name, w.name, "{path}");
            assert_eq!(e.frames.len(), w.frames.len(), "{path} {}", e.name);
            let mut entry_costs = Vec::new();
            for (i, (f, wf)) in e.frames.iter().zip(&w.frames).enumerate() {
                let one = reader.frame(f).unwrap_or_else(|err| panic!("{path} {} frame {i}: {err}", e.name));
                assert!(one.image == wf.image && (one.origin_x, one.origin_y, one.duration) == (wf.origin_x, wf.origin_y, wf.duration), "{path} {} frame {i}", e.name);
                entry_costs.push(reader.check(f).unwrap_or_else(|err| panic!("{path} {} frame {i}: {err}", e.name)));
                frames += 1;
            }
            costs.push(entry_costs);
        }
        assert!(reader.whole(&costs), "{path}: parse read it whole");
        // A reduced reading of the first big frame comes out of its frame's size, as painted.
        if let Some(f) = reader.entries.iter().flat_map(|e| &e.frames).find(|f| f.width >= 40 && f.height >= 40) {
            let r = reader.reduced(f, 3.0, &palette).unwrap_or_else(|err| panic!("{path}: {err}"));
            assert_eq!((r.width, r.height), (f.width.div_ceil(3), f.height.div_ceil(3)), "{path}");
        }
        read += 1;
    }
    eprintln!("{read} GAF files, {frames} frames read one at a time as whole");
    assert!(read > 400);
}
