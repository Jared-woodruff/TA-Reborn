//! Small, bounded fixtures for hostile headers: no test needs to exhaust memory or
//! overflow its own stack to verify the parser refuses the dangerous request.
use ta_formats::{cob::CobScript, fnt::Font, gaf::Gaf, hpi::HpiArchive, model3do::Model3do, pcx::PcxImage, tdf};

fn put(data: &mut [u8], at: usize, value: u32) { data[at..at + 4].copy_from_slice(&value.to_le_bytes()); }

fn archive(name: &str, size: u32, compression: u8, payload: &[u8]) -> Vec<u8> {
    let record = 37 + name.len() + 1;
    let end = record + 9;
    let mut data = vec![0; end];
    data[..4].copy_from_slice(b"HAPI");
    put(&mut data, 4, 0x10000);
    put(&mut data, 8, end as u32);
    put(&mut data, 16, 20);
    put(&mut data, 20, 1);
    put(&mut data, 24, 28);
    put(&mut data, 28, 37);
    put(&mut data, 32, record as u32);
    data[37..37 + name.len()].copy_from_slice(name.as_bytes());
    put(&mut data, record, end as u32);
    put(&mut data, record + 4, size);
    data[record + 8] = compression;
    data.extend_from_slice(payload);
    data
}

#[test]
fn hpi_paths_cycles_and_allocations_are_checked_before_reading() {
    for name in ["../outside", "/absolute", "C:/outside", "maps\\..\\outside", "NUL", "bad:stream"] {
        assert!(HpiArchive::from_bytes("test", archive(name, 1, 0, b"x")).is_err(), "{name}");
    }
    let mut cycle = archive("loop", 0, 0, &[]);
    cycle[36] = 1;
    put(&mut cycle, 32, 20);
    assert!(HpiArchive::from_bytes("test", cycle).is_err());
    for (size, compression, payload) in [(u32::MAX, 0, &[][..]), (65536, 2, &u32::MAX.to_le_bytes()[..]), (100, 0, &[][..])] {
        let a = HpiArchive::from_bytes("test", archive("safe", size, compression, payload)).unwrap();
        assert!(a.read(&a.files()[0]).is_err());
    }
}

/// An archive of files all at one place, the payload's start: (name, size, compression) each.
fn archive_at_one_place(files: &[(&str, u32, u8)], payload: &[u8]) -> Vec<u8> {
    let names = 28 + 9 * files.len();
    let records = names + files.iter().map(|(name, _, _)| name.len() + 1).sum::<usize>();
    let end = records + 9 * files.len();
    let mut data = vec![0; end];
    data[..4].copy_from_slice(b"HAPI");
    put(&mut data, 4, 0x10000);
    put(&mut data, 8, end as u32);
    put(&mut data, 16, 20);
    put(&mut data, 20, files.len() as u32);
    put(&mut data, 24, 28);
    let mut name_at = names;
    for (i, (name, size, compression)) in files.iter().enumerate() {
        let (entry, record) = (28 + 9 * i, records + 9 * i);
        put(&mut data, entry, name_at as u32);
        put(&mut data, entry + 4, record as u32);
        data[name_at..name_at + name.len()].copy_from_slice(name.as_bytes());
        name_at += name.len() + 1;
        put(&mut data, record, end as u32);
        put(&mut data, record + 4, *size);
        data[record + 8] = *compression;
    }
    data.extend_from_slice(payload);
    data
}

#[test]
fn an_empty_file_may_share_its_place_but_compressed_data_may_not() {
    use std::io::Write;
    let plain = b"Total Mayhem's packer gives an empty file the next file's place. ".repeat(40);
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&plain).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut chunk = vec![0; 19];
    chunk[..4].copy_from_slice(b"SQSH");
    chunk[5] = 2;
    put(&mut chunk, 7, compressed.len() as u32);
    put(&mut chunk, 11, plain.len() as u32);
    put(&mut chunk, 15, compressed.iter().map(|b| *b as u32).sum());
    chunk.extend_from_slice(&compressed);
    let mut payload = (chunk.len() as u32).to_le_bytes().to_vec();
    payload.extend_from_slice(&chunk);
    let size = plain.len() as u32;
    let a = HpiArchive::from_bytes("test", archive_at_one_place(&[("anims/EMPTY1.GAF", 0, 2), ("anims/EMPTY2.GAF", 0, 1), ("anims/REAL.GAF", size, 2)], &payload)).unwrap();
    assert_eq!(a.read(&a.files()[0]).unwrap(), b"");
    assert_eq!(a.read(&a.files()[1]).unwrap(), b"");
    assert_eq!(a.read(&a.files()[2]).unwrap(), plain);
    // Two files inflating the same data: a small archive could fill memory.
    assert!(HpiArchive::from_bytes("test", archive_at_one_place(&[("one.gaf", size, 2), ("two.gaf", size, 2)], &payload)).is_err());
}

#[test]
fn zlib_output_is_bounded_by_the_chunk_header() {
    use std::io::Write;
    let plain = vec![42; 1024];
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&plain).unwrap();
    let compressed = encoder.finish().unwrap();
    for declared in [8, 1024] {
        let mut chunk = vec![0; 19];
        chunk[..4].copy_from_slice(b"SQSH");
        chunk[5] = 2;
        put(&mut chunk, 7, compressed.len() as u32);
        put(&mut chunk, 11, declared);
        put(&mut chunk, 15, compressed.iter().map(|b| *b as u32).sum());
        chunk.extend_from_slice(&compressed);
        let mut payload = (chunk.len() as u32).to_le_bytes().to_vec();
        payload.extend_from_slice(&chunk);
        let a = HpiArchive::from_bytes("test", archive("safe", declared, 2, &payload)).unwrap();
        let result = a.read(&a.files()[0]);
        if declared == 8 { assert!(result.is_err()); } else { assert_eq!(result.unwrap(), plain); }
    }
}

#[test]
fn independent_zlib_chunks_reset_state_and_check_their_trailers() {
    use std::io::Write;
    let parts = [vec![17; 65536], (0..1234).map(|i| (i % 251) as u8).collect()];
    let mut chunks = Vec::new();
    for part in &parts {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(part).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut chunk = vec![0; 19];
        chunk[..4].copy_from_slice(b"SQSH");
        chunk[5] = 2;
        put(&mut chunk, 7, compressed.len() as u32);
        put(&mut chunk, 11, part.len() as u32);
        put(&mut chunk, 15, compressed.iter().map(|b| *b as u32).sum());
        chunk.extend_from_slice(&compressed);
        chunks.push(chunk);
    }
    let encoded = |chunks: &[Vec<u8>]| {
        let mut payload = Vec::new();
        for chunk in chunks { payload.extend_from_slice(&(chunk.len() as u32).to_le_bytes()); }
        for chunk in chunks { payload.extend_from_slice(chunk); }
        archive("safe", parts.iter().map(Vec::len).sum::<usize>() as u32, 2, &payload)
    };
    let a = HpiArchive::from_bytes("test", encoded(&chunks)).unwrap();
    assert_eq!(a.read(&a.files()[0]).unwrap(), parts.concat());
    // A valid SQSH checksum must not hide a corrupt Adler32 trailer in the next stream.
    *chunks[1].last_mut().unwrap() ^= 1;
    let checksum = chunks[1][19..].iter().map(|b| *b as u32).sum();
    put(&mut chunks[1], 15, checksum);
    let a = HpiArchive::from_bytes("test", encoded(&chunks)).unwrap();
    assert!(a.read(&a.files()[0]).is_err());
}

#[test]
fn binary_counts_cannot_allocate_from_tiny_headers() {
    let mut model = vec![0; 64];
    put(&mut model, 0, 1);
    put(&mut model, 4, i32::MAX as u32);
    assert!(Model3do::parse(&model).is_err());
    put(&mut model, 4, 0);
    put(&mut model, 8, i32::MAX as u32);
    assert!(Model3do::parse(&model).is_err());
    let mut cycle = vec![0; 104];
    put(&mut cycle, 0, 1);
    put(&mut cycle, 48, 52);
    put(&mut cycle, 52, 1);
    put(&mut cycle, 100, 52); // child points to itself
    assert!(Model3do::parse(&cycle).is_err());
    let mut cob = vec![0; 44];
    put(&mut cob, 0, 4);
    put(&mut cob, 16, i32::MAX as u32);
    assert!(CobScript::parse(&cob).is_err());
    let mut gaf = vec![0; 88];
    put(&mut gaf, 0, 0x10100);
    put(&mut gaf, 4, 1);
    put(&mut gaf, 12, 16);
    gaf[16] = 1;
    put(&mut gaf, 56, 64);
    put(&mut gaf, 64, u32::MAX); // 65535 x 65535 pixels
    assert!(Gaf::parse(&gaf).is_err());
    put(&mut gaf, 4, u32::MAX);
    assert!(ta_formats::gaf::entry_names(&gaf).is_err());
    let mut pcx = vec![0; 128];
    pcx[0] = 10;
    pcx[2] = 1;
    pcx[3] = 8;
    pcx[8..12].copy_from_slice(&[254, 255, 255, 255]);
    pcx[65] = 3;
    pcx[66..68].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(PcxImage::parse(&pcx).is_err());
}

/// A GAF of one entry listing `records` (indices into `frames`), each frame `(width, height,
/// rows)`: compressed, its rows' bytes (length fields included) given whole.
fn gaf_of(frames: &[(u16, u16, Vec<u8>)], records: &[usize]) -> Vec<u8> {
    let mut data = vec![0u8; 56 + records.len() * 8];
    put(&mut data, 0, 0x10100);
    put(&mut data, 4, 1);
    put(&mut data, 12, 16);
    data[16..18].copy_from_slice(&(records.len() as u16).to_le_bytes());
    data[24..28].copy_from_slice(b"boom");
    let mut headers = Vec::new();
    for (w, h, rows) in frames {
        let at = data.len();
        data.extend_from_slice(&w.to_le_bytes());
        data.extend_from_slice(&h.to_le_bytes());
        data.extend_from_slice(&((*w / 2) as i16).to_le_bytes());
        data.extend_from_slice(&((*h / 2) as i16).to_le_bytes());
        data.extend_from_slice(&[9, 1, 0, 0, 0, 0, 0, 0]);
        data.extend_from_slice(&((at + 24) as u32).to_le_bytes());
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(rows);
        headers.push(at as u32);
    }
    for (i, &f) in records.iter().enumerate() {
        put(&mut data, 56 + i * 8, headers[f]);
        put(&mut data, 60 + i * 8, 2);
    }
    data
}

/// Compressed rows `w` pixels wide: `colour(y)` for each row, all of it (runs of 64), or none
/// of it (an empty row) for `None`.
fn rows_of(w: u16, h: u16, colour: impl Fn(u16) -> Option<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..h {
        let mut row = Vec::new();
        if let Some(c) = colour(y) {
            let mut left = w as usize;
            while left > 0 {
                let n = left.min(64);
                row.extend_from_slice(&[(((n - 1) as u8) << 2) | 2, c]);
                left -= n;
            }
        }
        out.extend_from_slice(&(row.len() as u16).to_le_bytes());
        out.extend_from_slice(&row);
    }
    out
}

#[test]
fn an_animation_too_big_to_decode_whole_is_read_reduced_a_frame_at_a_time() {
    use ta_formats::gaf::{AnimSize, GafReader, MAX_DRAWN_SIDE, plan};
    use ta_formats::image::Palette;
    // Twelve 4000 x 4000 frames, each the top half one colour and the bottom another: 384 MiB
    // decoded whole (more than `Gaf::parse` holds), 6 MB of rows on disk.
    let frames: Vec<(u16, u16, Vec<u8>)> = (0..12u8).map(|i| (4000, 4000, rows_of(4000, 4000, |y| Some(if y < 2000 { 10 + i } else { 30 + i })))).collect();
    let data = gaf_of(&frames, &(0..12).collect::<Vec<_>>());
    assert!(data.len() < 8 << 20, "{}", data.len());
    assert!(Gaf::parse(&data).unwrap_err().to_string().contains("exceeds resource limit"));
    let reader = GafReader::open(&data).unwrap();
    let entry = reader.entry("BOOM").unwrap();
    let sizes = [entry.size()];
    assert_eq!((sizes[0].frames, sizes[0].largest), (12, 4000));
    // Planned within 8 million texels, drawn at one density, each frame as big as painted.
    let p = plan(&sizes, 8 << 20);
    assert!(p.drawn <= 8 << 20 && p.scales[0] > 4000.0 / f64::from(MAX_DRAWN_SIDE), "{p:?}");
    let mut palette = Palette::default();
    for i in 0..12 {
        palette.colors[10 + i] = [200, 40, 0];
        palette.colors[30 + i] = [0, 40, 200];
    }
    for f in &entry.frames {
        let r = reader.reduced(f, p.scales[0], &palette).unwrap();
        assert_eq!((r.full_width, r.full_height, r.origin_x, r.origin_y), (4000, 4000, 2000, 2000));
        assert!(r.width <= MAX_DRAWN_SIDE && f64::from(r.width) * r.scale >= 4000.0, "drawn as big as painted");
        let texel = |x: u32, y: u32| &r.rgba[((y * r.width + x) * 4) as usize..((y * r.width + x) * 4 + 4) as usize];
        assert_eq!(texel(r.width / 2, 2), [200, 40, 0, 255], "the top half's colour");
        assert_eq!(texel(r.width / 2, r.height - 3), [0, 40, 200, 255], "the bottom half's");
    }
    let total: u64 = entry.frames.iter().map(|f| u64::from(f.width) * u64::from(f.height)).sum();
    assert_eq!(AnimSize::of(entry.frames.iter().map(|f| (f.width, f.height))).area, total);
}

#[test]
fn small_files_claiming_huge_frames_are_still_refused() {
    use ta_formats::gaf::GafReader;
    use ta_formats::image::Palette;
    let palette = Palette::default();
    // A frame of 65535 x 65535 in a file of a hundred bytes: its rows are not there.
    let tiny = gaf_of(&[(u16::MAX, u16::MAX, vec![0; 16])], &[0]);
    assert!(tiny.len() < 200);
    assert!(Gaf::parse(&tiny).is_err());
    let reader = GafReader::open(&tiny).unwrap();
    let f = reader.entries[0].frames[0];
    assert!(reader.check(&f).is_err() && reader.reduced(&f, 64.0, &Palette::default()).is_err());
    // A 4000 x 4000 frame of empty rows (8 KB), listed 65535 times: a 1 MB file claiming a
    // trillion pixels. Read reduced a frame at a time, its reader stops at what the file can
    // hold, never holding more than the frame it reads.
    let empty = (4000u16, 4000u16, rows_of(4000, 4000, |_| None));
    let many = gaf_of(&[empty], &vec![0; 65535]);
    assert!(many.len() < 1 << 20, "{}", many.len());
    assert!(Gaf::parse(&many).is_err());
    let reader = GafReader::open(&many).unwrap();
    let frames = &reader.entries[0].frames;
    assert_eq!(frames.len(), 65535);
    let read = frames.iter().take_while(|f| reader.reduced(f, 4.0, &palette).is_ok()).count();
    assert!((10..=20).contains(&read), "{read} frames read before the work ran out");
    assert!(reader.check(&frames[read]).is_err(), "and nothing more is read");
    // A frame no reduction makes small enough: refused, not drawn larger.
    let wide = gaf_of(&[(u16::MAX, 1, rows_of(u16::MAX, 1, |_| Some(3)))], &[0]);
    let reader = GafReader::open(&wide).unwrap();
    assert!(reader.reduced(&reader.entries[0].frames[0], 4.0, &palette).is_err());
    assert_eq!(reader.reduced(&reader.entries[0].frames[0], 32.0, &palette).unwrap().width, 2048);
}

#[test]
fn repeated_glyphs_have_an_aggregate_budget() {
    let mut data = vec![0; 516 + (255 * 1024usize).div_ceil(8) + 1];
    data[..2].copy_from_slice(&1024u16.to_le_bytes());
    for i in 0..256 { data[4 + i * 2..6 + i * 2].copy_from_slice(&516u16.to_le_bytes()); }
    data[516] = 255;
    assert!(Font::parse(&data).is_err());
}

#[test]
fn deeply_nested_tdf_skips_the_subtree_and_keeps_following_sections() {
    for open in ["[nested]{", "{"] {
        let text = format!("{}hidden=1;{}[after]{{value=2;}}", open.repeat(10000), "}".repeat(10000));
        let doc = tdf::parse(&text);
        assert_eq!(doc.section("after").unwrap().get("value"), Some("2"));
        // Destruction and serialization are also safe because the returned tree is bounded.
        assert!(doc.to_text().contains("after"));
    }
}
