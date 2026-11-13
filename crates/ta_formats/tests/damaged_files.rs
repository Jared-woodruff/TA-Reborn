//! Every reader against damaged files: each truncation of a small valid file of its format, and
//! thousands of copies of it with a few bytes changed (by a seeded generator: the same damage
//! every run), must come back as a result, most of them errors, never a panic. Mods ship files
//! of every kind, and some are damaged; what a reader accepts is used too, so it is.
use ta_formats::cob::{self, CobScript};
use ta_formats::fnt::Font;
use ta_formats::gaf::{self, Gaf, GafEntry, GafFrame};
use ta_formats::hpi::HpiArchive;
use ta_formats::image::{IndexedImage, Palette};
use ta_formats::model3do::Model3do;
use ta_formats::pcx::PcxImage;
use ta_formats::tdf;
use ta_formats::tnt::{FEATURE_NONE, TntMap};

fn put(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Appends `bytes`, returning where they start.
fn add(data: &mut Vec<u8>, bytes: &[u8]) -> u32 {
    let at = data.len() as u32;
    data.extend_from_slice(bytes);
    at
}

/// A seeded xorshift generator.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// `read` over every truncation of `valid` and `rounds` copies of it with one to four bytes
/// changed: to a random value, or to 0x00 or 0xFF (the values that make counts and offsets
/// extreme).
fn damage(seed: u64, valid: &[u8], rounds: usize, read: impl Fn(&[u8])) {
    for len in 0..valid.len() {
        read(&valid[..len]);
    }
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed);
    for _ in 0..rounds {
        let mut bytes = valid.to_vec();
        for _ in 0..1 + rng.below(4) {
            let at = rng.below(bytes.len());
            bytes[at] = match rng.below(4) {
                0 => 0,
                1 => 0xFF,
                _ => rng.next() as u8,
            };
        }
        read(&bytes);
    }
}

/// A model of two pieces (a base and its turret) sharing a textured triangle.
fn model() -> Vec<u8> {
    let mut b = vec![0u8; 104];
    let base = add(&mut b, b"base\0");
    let turret = add(&mut b, b"turret\0");
    let texture = add(&mut b, b"Metal01\0");
    let vertices: Vec<u8> = [[0i32, 0, 0], [65536, 0, 0], [0, 65536, 0]].iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    let vertex_ptr = add(&mut b, &vertices);
    let indices: Vec<u8> = [0i16, 1, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
    let index_ptr = add(&mut b, &indices);
    let mut primitive = vec![0u8; 32];
    put(&mut primitive, 0, 5);
    put(&mut primitive, 4, 3);
    put(&mut primitive, 12, index_ptr);
    put(&mut primitive, 16, texture);
    let primitive_ptr = add(&mut b, &primitive);
    for (at, name, child) in [(0usize, base, 52u32), (52, turret, 0)] {
        put(&mut b, at, 1);
        put(&mut b, at + 4, 3);
        put(&mut b, at + 8, 1);
        put(&mut b, at + 12, u32::MAX);
        put(&mut b, at + 16, 65536 * (at as u32 / 52));
        put(&mut b, at + 28, name);
        put(&mut b, at + 36, vertex_ptr);
        put(&mut b, at + 40, primitive_ptr);
        put(&mut b, at + 48, child);
    }
    b
}

/// A script of two functions over two pieces.
fn script() -> Vec<u8> {
    let mut b = vec![0u8; 44];
    put(&mut b, 0, 4);
    put(&mut b, 4, 2);
    put(&mut b, 8, 2);
    put(&mut b, 12, 6);
    put(&mut b, 16, 1);
    let offsets = add(&mut b, &[0u32, 3].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>());
    let names = add(&mut b, &[0u8; 8]);
    let pieces = add(&mut b, &[0u8; 8]);
    let code = add(&mut b, &[cob::op::PUSH_CONSTANT, 7, cob::op::RETURN, cob::op::TURN_NOW, 1, 2].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>());
    let strings = [add(&mut b, b"Create\0"), add(&mut b, b"AimPrimary\0"), add(&mut b, b"base\0"), add(&mut b, b"turret\0")];
    for (i, s) in strings.iter().enumerate() {
        let table = if i < 2 { names } else { pieces };
        put(&mut b, table as usize + (i % 2) * 4, *s);
    }
    put(&mut b, 24, offsets);
    put(&mut b, 28, names);
    put(&mut b, 32, pieces);
    put(&mut b, 36, code);
    b
}

/// An animation of two frames, one a different size, with runs and single pixels.
fn animation() -> Vec<u8> {
    let frame = |w: u32, h: u32, seed: u8| {
        let mut image = IndexedImage::new(w, h, 9);
        for (i, p) in image.pixels.iter_mut().enumerate() {
            *p = if i % 7 < 3 { 9 } else { seed.wrapping_add(i as u8) };
        }
        GafFrame { image, origin_x: 2, origin_y: -1, transparency_index: 9, duration: 3 }
    };
    let gaf = Gaf { entries: vec![GafEntry { name: "boom".into(), frames: vec![frame(6, 5, 1), frame(3, 7, 40)] }, GafEntry { name: "smoke".into(), frames: vec![frame(2, 2, 90)] }] };
    gaf.write().unwrap()
}

/// A picture two pixels wide, one high, run-length encoded, with its palette.
fn picture() -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[0] = 0x0A;
    b[1] = 5;
    b[2] = 1;
    b[3] = 8;
    b[8] = 1;
    b[65] = 1;
    b[66..68].copy_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&[0xC2, 7]);
    b.push(0x0C);
    b.extend((0..768).map(|n| (n % 256) as u8));
    b
}

/// A map of 4 × 4 cells: two tiles, heights, a feature and a minimap.
fn terrain() -> Vec<u8> {
    let mut features = vec![FEATURE_NONE; 16];
    features[5] = 0;
    let map = TntMap {
        width: 4,
        height: 4,
        sea_level: 20,
        tiles: vec![Box::new([1; 1024]), Box::new([2; 1024])],
        tile_map: vec![0, 1, 1, 0],
        heights: (0..16).map(|h| h * 10).collect(),
        features,
        feature_names: vec!["Tree01".into()],
        minimap: IndexedImage::new(4, 4, 3),
        flags: Vec::new(),
    };
    map.write().unwrap()
}

/// An archive holding one compressed file.
fn archive() -> Vec<u8> {
    use std::io::Write;
    let plain: Vec<u8> = (0..1500).map(|i| (i % 13) as u8).collect();
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&plain).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut chunk = vec![0u8; 19];
    chunk[..4].copy_from_slice(b"SQSH");
    chunk[5] = 2;
    put(&mut chunk, 7, compressed.len() as u32);
    put(&mut chunk, 11, plain.len() as u32);
    put(&mut chunk, 15, compressed.iter().map(|b| *b as u32).sum());
    chunk.extend_from_slice(&compressed);
    let mut payload = (chunk.len() as u32).to_le_bytes().to_vec();
    payload.extend_from_slice(&chunk);
    let name = "units/file.fbi";
    let record = 37 + name.len() + 1;
    let end = record + 9;
    let mut data = vec![0u8; end];
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
    put(&mut data, record + 4, plain.len() as u32);
    data[record + 8] = 2;
    data.extend_from_slice(&payload);
    data
}

/// A font of two glyphs.
fn font() -> Vec<u8> {
    let mut b = vec![0u8; 516];
    b[..2].copy_from_slice(&8u16.to_le_bytes());
    let glyph = |b: &mut Vec<u8>, width: u8| {
        let at = b.len() as u16;
        b.push(width);
        b.extend(std::iter::repeat_n(0xA5, (width as usize * 8).div_ceil(8)));
        at
    };
    let a = glyph(&mut b, 5);
    let z = glyph(&mut b, 7);
    b[4 + b'A' as usize * 2..6 + b'A' as usize * 2].copy_from_slice(&a.to_le_bytes());
    b[4 + b'Z' as usize * 2..6 + b'Z' as usize * 2].copy_from_slice(&z.to_le_bytes());
    b
}

#[test]
fn the_samples_are_valid() {
    let m = Model3do::parse(&model()).unwrap();
    assert_eq!((m.pieces.len(), m.pieces[0].children.as_slice(), m.texture_names()), (2, &[1][..], vec!["Metal01".to_string()]));
    let s = CobScript::parse(&script()).unwrap();
    assert_eq!((s.function("create"), s.function("AIMPRIMARY"), s.pieces.len()), (Some(0), Some(1), 2));
    assert_eq!(Gaf::parse(&animation()).unwrap().entries.len(), 2);
    assert_eq!(PcxImage::parse(&picture()).unwrap().to_rgba(), [21, 22, 23, 255, 21, 22, 23, 255], "index 7 of its palette");
    assert_eq!(TntMap::parse(&terrain()).unwrap().feature_names, ["Tree01"]);
    let a = HpiArchive::from_bytes("test.hpi", archive()).unwrap();
    assert_eq!(a.read(&a.files()[0]).unwrap().len(), 1500);
    assert!(Font::parse(&font()).unwrap().glyph('A').is_some());
}

#[test]
fn models_scripts_and_animations_survive_damage() {
    damage(1, &model(), 4000, |b| {
        if let Ok(m) = Model3do::parse(b) {
            let _ = (m.texture_names(), m.piece_index("turret"));
            for p in &m.pieces {
                assert!(p.parent.is_none_or(|q| q < m.pieces.len()) && p.children.iter().all(|&c| c < m.pieces.len()));
                assert!(p.primitives.iter().all(|pr| pr.indices.iter().all(|&i| (i as usize) < p.vertices.len())));
            }
        }
    });
    damage(2, &script(), 4000, |b| {
        if let Ok(s) = CobScript::parse(b) {
            assert!(s.functions.iter().all(|f| (f.offset as usize) < s.code.len()));
            let _ = cob::disassemble(&s);
        }
    });
    damage(3, &animation(), 4000, |b| {
        let _ = gaf::entry_names(b);
        let whole = Gaf::parse(b);
        if let Ok(g) = &whole {
            for f in g.entries.iter().flat_map(|e| &e.frames) {
                assert_eq!(f.image.pixels.len(), (f.image.width * f.image.height) as usize);
            }
        }
        // Read a frame at a time, whole, through, or reduced: a result, never a panic; and a
        // frame read whole is the one the whole file's reading has, where both read it.
        if let Ok(reader) = gaf::GafReader::open(b) {
            for (e, entry) in reader.entries.iter().enumerate() {
                for (i, f) in entry.frames.iter().enumerate() {
                    let one = reader.frame(f);
                    if let (Ok(one), Ok(g)) = (&one, &whole) {
                        assert_eq!(one.image, g.entries[e].frames[i].image);
                    }
                    let _ = reader.check(f);
                    if let Ok(r) = reader.reduced(f, 1.5, &Palette::default()) {
                        assert_eq!(r.rgba.len(), (r.width * r.height * 4) as usize);
                    }
                }
            }
        }
    });
}

#[test]
fn pictures_maps_archives_fonts_and_text_survive_damage() {
    damage(4, &picture(), 4000, |b| {
        if let Ok(p) = PcxImage::parse(b) {
            let _ = p.to_rgba();
        }
        let _ = Palette::parse(b);
    });
    damage(5, &terrain(), 3000, |b| {
        let _ = TntMap::size(b);
        let _ = TntMap::preview(b);
        if let Ok(m) = TntMap::parse(b) {
            assert_eq!(m.heights.len(), (m.width * m.height) as usize);
            let _ = (m.render_indices(), m.height_at(m.width.saturating_sub(1), m.height.saturating_sub(1)));
        }
    });
    damage(6, &archive(), 3000, |b| {
        if let Ok(a) = HpiArchive::from_bytes("test.hpi", b.to_vec()) {
            for f in a.files() {
                let _ = a.read(f);
            }
        }
    });
    damage(7, &font(), 3000, |b| {
        if let Ok(f) = Font::parse(b) {
            let _ = f.width("AZ?");
        }
    });
    let text = b"[UNITINFO]\n{\n  UnitName=ARMPW;\n  [WEAPON]{ range=300; }\n  // a comment\n  Side=ARM;\n}\n";
    damage(8, text, 3000, |b| {
        let doc = tdf::parse_bytes(b);
        let _ = doc.to_text();
    });
}
