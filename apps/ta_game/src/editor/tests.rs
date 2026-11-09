//! The editor's document, tools, undo and saving; and a map made, edited, saved as a mod,
//! mounted through the VFS and played for a few ticks (skipped without a TA install).

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::{FEATURE_NONE, FEATURE_VOID, MAX_TILES, TILE_SIZE, TntMap};

use super::doc::{CELL_PX, MapDoc, NewMap, Rect};
use super::history::{History, Snapshot};
use super::save;
use super::tools::{Clip, Ground, HeightOp, HeightStroke, PasteWith, Scatter, TileBlock};

/// A small map whose tiles are flat colours `0..count`.
fn palette_map(count: u8) -> TntMap {
    let tiles: Vec<Box<[u8; TILE_SIZE * TILE_SIZE]>> = (0..count).map(|c| Box::new([c; TILE_SIZE * TILE_SIZE])).collect();
    let (w, h) = (32u32, 32u32);
    TntMap {
        width: w,
        height: h,
        sea_level: 0,
        tile_map: (0..(w / 2 * (h / 2))).map(|i| (i % count as u32) as u16).collect(),
        tiles,
        heights: vec![50; (w * h) as usize],
        features: vec![FEATURE_NONE; (w * h) as usize],
        feature_names: Vec::new(),
        minimap: Default::default(),
    }
}

fn blank() -> MapDoc {
    let new = NewMap { name: "Test Map".into(), width_steps: 2, height_steps: 1, sea_level: 20, height: 40, palette_map: "x".into() };
    MapDoc::new_blank(&new, &palette_map(3), Some(&MapInfo::default()))
}

fn empty_data() -> GameData {
    GameData::load(&ta_vfs::Vfs::new())
}

#[test]
fn publishing_preserves_builtins_and_refreshes_compatibility() {
    let root = std::env::temp_dir().join(format!("ta_editor_builtin_{}", std::process::id()));
    let base = root.join("base");
    let builtin = root.join("builtin");
    let mods = root.join("mods");
    std::fs::create_dir_all(base.join("gamedata")).unwrap();
    std::fs::write(base.join("gamedata/sidedata.tdf"), b"[side0]{name=ARM;commander=ARMCOM;}").unwrap();
    std::fs::create_dir_all(builtin.join("maps")).unwrap();
    std::fs::write(builtin.join("maps/builtin.tnt"), palette_map(3).write().unwrap()).unwrap();
    std::fs::write(builtin.join("maps/builtin.ota"), b"[GlobalHeader]{missionname=Builtin;[Schema 0]{Type=Network 2;}}").unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&base, "base").unwrap();
    vfs.mount_mod_dir(&builtin, ta_vfs::BUILTIN_LAYER).unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let initial = ta_data::compatibility_fingerprint(&vfs).unwrap();
    let mut assets = crate::setup::Assets {
        vfs: Arc::new(vfs), data: data.clone(), types,
        maps: Vec::new(), selectable_maps: Vec::new(), campaigns: Vec::new(), data_hash: initial, compatibility: Default::default(), mods: Default::default(),
    };
    let mut config = ta_config::Config::default();
    let mut doc = blank();
    let saved = save::write_mod(&doc, &data.palette, &mods, "test").unwrap();
    save::publish_in(&saved, &mut assets, &mut config, std::slice::from_ref(&mods), false).unwrap();
    assert_eq!(config.mods.enabled, [saved.id.clone()]);
    assert_eq!(assets.vfs.origin("maps/builtin.tnt").unwrap().layer, ta_vfs::BUILTIN_LAYER);
    assert!(assets.maps.iter().any(|m| m.title == "Test Map"));
    assert_ne!(assets.data_hash, initial);
    let published = assets.data_hash;
    doc.tnt.heights[0] += 1;
    save::write_mod(&doc, &data.palette, &mods, "test").unwrap();
    save::publish_in(&saved, &mut assets, &mut config, std::slice::from_ref(&mods), false).unwrap();
    assert_ne!(assets.data_hash, published, "overwriting an already mounted map updates its hash");
    assert_eq!(assets.data_hash, ta_data::compatibility_fingerprint(&assets.vfs).unwrap());
    assert!(assets.vfs.exists("maps/builtin.tnt"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn new_maps_have_the_asked_size_sea_and_height() {
    let doc = blank();
    assert_eq!((doc.tnt.width, doc.tnt.height), (64, 32), "1024 × 512 px");
    assert_eq!(doc.tnt.sea_level, 20);
    assert!(doc.tnt.heights.iter().all(|&h| h == 40));
    assert_eq!(doc.tnt.tile_map.len(), 32 * 16);
    assert_eq!(doc.tnt.tiles.len(), 3, "the palette map's tiles");
    assert_eq!(doc.starts.len(), 2);
}

#[test]
fn a_brush_stroke_raises_the_ground_and_undo_and_redo_replay_it() {
    let mut doc = blank();
    let mut history = History::default();
    let before = Snapshot::of(&doc);
    let mut stroke = HeightStroke::new(&doc);
    let mut changed = None;
    for _ in 0..30 {
        changed = Rect::merge(changed, stroke.apply(&mut doc, 512.0, 256.0, 100.0, 1.0, HeightOp::Raise, 1.0 / 30.0));
    }
    let changed = changed.expect("heights changed");
    let centre = (16 * 64 + 32) as usize;
    assert!(doc.tnt.heights[centre] > 100, "the centre rose by about 90 in a second: {}", doc.tnt.heights[centre]);
    assert_eq!(doc.tnt.heights[0], 40, "far away nothing changed");
    assert!(changed.x0 >= 32 - 7 && changed.x1 <= 32 + 8, "the changed cells stay within the radius: {changed:?}");
    let raised = doc.tnt.heights.clone();
    let c = history.record(before, &doc, "Raise").expect("recorded");
    assert!(c.heights.is_some() && c.tiles.is_none() && !c.features);
    history.undo(&mut doc).unwrap();
    assert!(doc.tnt.heights.iter().all(|&h| h == 40), "undo restores the flat ground");
    history.redo(&mut doc).unwrap();
    assert_eq!(doc.tnt.heights, raised, "redo raises it again");
    assert!(history.undo(&mut doc).is_some() && history.undo(&mut doc).is_none(), "one edit only");
}

#[test]
fn smoothing_flattening_and_levelling_move_towards_their_targets() {
    let mut doc = blank();
    let i = (16 * 64 + 32) as usize;
    doc.tnt.heights[i] = 200;
    let mut stroke = HeightStroke::new(&doc);
    for _ in 0..20 {
        stroke.apply(&mut doc, 512.0, 256.0, 64.0, 1.0, HeightOp::Smooth, 1.0 / 30.0);
    }
    assert!(doc.tnt.heights[i] < 120, "a spike is smoothed away: {}", doc.tnt.heights[i]);
    let mut stroke = HeightStroke::new(&doc);
    for _ in 0..60 {
        stroke.apply(&mut doc, 512.0, 256.0, 64.0, 1.0, HeightOp::Flatten(40.0), 1.0 / 30.0);
    }
    assert!((doc.tnt.heights[i] as i32 - 40).abs() <= 2, "flattened to 40: {}", doc.tnt.heights[i]);
    let mut stroke = HeightStroke::new(&doc);
    stroke.apply(&mut doc, 512.0, 256.0, 96.0, 1.0, HeightOp::Level(123.0), 1.0 / 30.0);
    assert_eq!(doc.tnt.heights[i], 123, "set level is exact inside the brush");
    // Its edge eases into the ground round it (no cliff where the brush ends).
    let edge = (16 * 64 + 32 + 5) as usize;
    assert!(doc.tnt.heights[edge] > 40 && doc.tnt.heights[edge] < 123, "the edge is between: {}", doc.tnt.heights[edge]);
}

#[test]
fn tiles_from_another_map_join_the_set_once_and_stamps_undo() {
    let mut doc = blank();
    let mut other = palette_map(6);
    *other.tiles[5] = [77; TILE_SIZE * TILE_SIZE];
    let block = TileBlock::import(&mut doc, &other, Rect::new(0, 0, 6, 1)).unwrap();
    assert_eq!((block.w, block.h), (6, 1));
    assert_eq!(doc.tnt.tiles.len(), 6, "three shared tiles, three new ones");
    let again = TileBlock::import(&mut doc, &other, Rect::new(0, 0, 6, 1)).unwrap();
    assert_eq!(again, block, "importing again reuses the tiles");
    assert_eq!(doc.tnt.tiles.len(), 6);
    let mut history = History::default();
    let before = Snapshot::of(&doc);
    let changed = block.stamp(&mut doc, 30, 15).expect("stamped");
    assert_eq!(changed, Rect::new(31, 15, 32, 16), "the first tile is there already; the rest is clipped at the corner");
    assert_eq!(doc.tnt.tile_map[15 * 32 + 31], block.tiles[1]);
    history.record(before, &doc, "Paint tiles").unwrap();
    history.undo(&mut doc);
    assert_eq!(doc.tnt.tile_map[15 * 32 + 31], blank().tnt.tile_map[15 * 32 + 31], "undone");
}

#[test]
fn blocks_come_from_a_tile_set_too() {
    let mut doc = blank();
    let other = palette_map(5);
    // Set laid out 2 a row: places (1, 0) and (1, 1) are tiles 1 and 3; (1, 2) is past the end.
    let block = TileBlock::from_set(&mut doc, Some(&other), Rect::new(1, 0, 2, 3), 2).unwrap();
    assert_eq!((block.w, block.h), (1, 3));
    assert_eq!(doc.tnt.tiles.len(), 5, "tiles 3 and 4 joined the three shared ones");
    assert_eq!(block.tiles, vec![1, 3, 4], "past the end: the set's last tile");
    let own = TileBlock::from_set(&mut doc, None, Rect::new(0, 0, 2, 1), 32).unwrap();
    assert_eq!(own.tiles, vec![0, 1], "this map's own set, by index");
}

#[test]
fn tile_import_exhaustion_preserves_the_map_and_rolls_back_partial_brushes() {
    let mut doc = blank();
    // One remaining slot, but the selected block needs two new colours.
    doc.tnt.tiles.resize_with(MAX_TILES - 1, || Box::new([0; TILE_SIZE * TILE_SIZE]));
    let before = doc.tnt.tiles.len();
    let tile_map = doc.tnt.tile_map.clone();
    let mut source = palette_map(2);
    *source.tiles[0] = [77; TILE_SIZE * TILE_SIZE];
    *source.tiles[1] = [78; TILE_SIZE * TILE_SIZE];
    assert!(TileBlock::import(&mut doc, &source, Rect::new(0, 0, 2, 1)).is_err());
    assert_eq!(doc.tnt.tiles.len(), before);
    assert_eq!(doc.tnt.tile_map, tile_map);
    assert!(TileBlock::from_set(&mut doc, Some(&source), Rect::new(0, 0, 2, 1), 2).is_err());
    assert_eq!(doc.tnt.tiles.len(), before);
    let single = TileBlock::import(&mut doc, &source, Rect::new(0, 0, 1, 1)).unwrap();
    assert_eq!(doc.tnt.tiles[single.tiles[0] as usize][0], 77, "retry must not reuse a rolled-back lookup entry");
    assert_eq!(doc.tnt.tiles.len(), MAX_TILES);
    assert_eq!(single.tiles, [u16::MAX], "the final index is usable");
    // Saving a map that references every tile and repeats the final index must
    // not interpret that final index as an unassigned remapping sentinel.
    doc.tnt.width = 512;
    doc.tnt.height = 514;
    let cells = (doc.tnt.width * doc.tnt.height) as usize;
    doc.tnt.heights.resize(cells, 40);
    doc.tnt.features.resize(cells, FEATURE_NONE);
    doc.tnt.tile_map = (0..cells / 4).map(|i| i.min(u16::MAX as usize) as u16).collect();
    let compact = doc.compacted(&ta_formats::image::Palette::default());
    assert_eq!(compact.tiles.len(), MAX_TILES);
    assert_eq!(compact.tile_map, doc.tnt.tile_map);
    assert_eq!(compact.tiles[u16::MAX as usize][0], 77);
}

#[test]
fn feature_indices_never_alias_reserved_markers_or_the_next_row() {
    let data = empty_data();
    let mut doc = blank();
    doc.tnt.feature_names = vec!["Tree".into(); u16::MAX as usize + 1];
    assert_eq!(doc.feature_name_at(0, 0), None, "FEATURE_NONE remains empty even in an oversized name table");
    assert_eq!(doc.features().count(), 0);
    assert!(!doc.place_feature(&data, "NewTree", (0, 0)), "reserved values cannot become feature IDs");
    assert!(doc.place_feature(&data, "Tree", (0, 1)), "existing representable names can still be placed");
    assert_eq!(doc.feature_name_at(doc.tnt.width, 0), None, "off-edge lookup cannot wrap into the next row");
    assert!(!doc.remove_feature(&data, (doc.tnt.width, 0)));
    assert_eq!(doc.feature_name_at(0, 1), Some("Tree"));
    assert!(!doc.place_feature(&data, "Tree", (u32::MAX, 0)));
    let compact = doc.compacted(&ta_formats::image::Palette::default());
    assert_eq!(compact.feature_names, ["Tree"]);
    assert_eq!(compact.features[0], FEATURE_NONE);
}

#[test]
fn a_copied_region_pastes_its_tiles_and_heights() {
    let mut doc = blank();
    for z in 0..4 {
        for x in 0..4 {
            doc.tnt.heights[z * 64 + x] = 90 + (x + z) as u8;
        }
    }
    doc.tnt.tile_map[0] = 2;
    let data = empty_data();
    let clip = Clip::copy(&doc, Rect::new(0, 0, 2, 2));
    let with = PasteWith { heights: true, relative: false, features: true };
    let pasted = clip.paste(&mut doc, &data, 10, 5, with);
    assert_eq!(pasted.tiles.map(|r| (r.x0, r.z0)), Some((10, 5)));
    assert!(pasted.cells.is_some());
    assert!(!pasted.features, "nothing there to copy");
    assert_eq!(doc.tnt.tile_map[5 * 32 + 10], 2);
    assert_eq!(doc.tnt.heights[(10 * 64 + 20 + 3) as usize], 93, "heights move with their tiles (cell = 2 × tile)");
    let pasted = clip.paste(&mut doc, &data, 20, 5, PasteWith { heights: false, ..with });
    assert!(pasted.cells.is_none(), "tiles only");
}

#[test]
fn a_copy_takes_its_features_and_impassable_ground_along() {
    let (data, dir) = data_with_big_rock("copy");
    let mut doc = blank();
    let w = doc.tnt.width;
    // In tiles (0, 0)-(4, 4), cells (0, 0)-(8, 8): a rock, a tree and a void cell.
    assert!(doc.place_feature(&data, "BigRock", (1, 1)));
    assert!(doc.place_feature(&data, "TreeA", (6, 2)));
    doc.tnt.features[(7 * w + 7) as usize] = FEATURE_VOID;
    // Where it goes, tiles (10, 4) on: a feature of its own and a void cell, which give way.
    assert!(doc.place_feature(&data, "RockB", (22, 9)));
    doc.tnt.features[(10 * w + 25) as usize] = FEATURE_VOID;
    let clip = Clip::copy(&doc, Rect::new(0, 0, 4, 4));
    assert_eq!(clip.features.len(), 2);
    assert_eq!(clip.void, [(7, 7)]);
    let with = PasteWith { heights: false, relative: false, features: true };
    let pasted = clip.paste(&mut doc, &data, 10, 4, with);
    assert!(pasted.features);
    assert_eq!(doc.feature_name_at(21, 9), Some("BigRock"), "cell (1, 1) + (20, 8)");
    assert_eq!(doc.feature_name_at(26, 10), Some("TreeA"));
    assert!(doc.void_at(27, 15));
    assert_eq!(doc.feature_name_at(22, 9), None, "the feature there made way");
    assert!(!doc.void_at(25, 10), "and its impassable ground");
    // Without features, those there stay.
    let before = doc.tnt.features.clone();
    let pasted = clip.paste(&mut doc, &data, 10, 4, PasteWith { features: false, ..with });
    assert!(!pasted.features);
    assert_eq!(doc.tnt.features, before);
    // A paste half off the map keeps what fits.
    let pasted = clip.paste(&mut doc, &data, -2, 0, with);
    assert!(pasted.features);
    assert_eq!(doc.feature_name_at(2, 2), Some("TreeA"), "cell (6, 2) - (4, 0)");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn heights_pasted_on_the_ground_there_meet_it_at_the_edges() {
    let data = empty_data();
    let mut doc = blank();
    let w = doc.tnt.width as usize;
    // A 2 × 2 tile copy (4 × 4 cells) of a plateau at 90 with a peak at 100 in it.
    for z in 0..4 {
        for x in 0..4 {
            doc.tnt.heights[z * w + x] = 90;
        }
    }
    doc.tnt.heights[w + 1] = 100;
    let clip = Clip::copy(&doc, Rect::new(0, 0, 2, 2));
    let with = PasteWith { heights: true, relative: true, features: false };
    // Laid on the ground at 50 (blank's level is 40; set a patch to 50).
    for z in 10..14 {
        for x in 20..24 {
            doc.tnt.heights[z * w + x] = 50;
        }
    }
    clip.paste(&mut doc, &data, 10, 5, with);
    assert_eq!(doc.tnt.heights[10 * w + 20], 50, "the edge meets the ground");
    assert_eq!(doc.tnt.heights[11 * w + 21], 60, "the peak rises as far over it as it did");
    clip.paste(&mut doc, &data, 20, 5, PasteWith { relative: false, ..with });
    assert_eq!(doc.tnt.heights[10 * w + 40], 90, "as it was copied");
}

#[test]
fn alt_click_finds_the_feature_whose_footprint_covers_the_cell() {
    let (data, dir) = data_with_big_rock("covering");
    let mut doc = blank();
    assert!(doc.place_feature(&data, "BigRock", (10, 10)));
    assert!(doc.place_feature(&data, "TreeA", (14, 10)));
    assert_eq!(doc.feature_covering(&data, 13, 12), Some(((10, 10), "BigRock")), "its far corner");
    assert_eq!(doc.feature_covering(&data, 10, 10), Some(((10, 10), "BigRock")));
    assert_eq!(doc.feature_covering(&data, 14, 10), Some(((14, 10), "TreeA")));
    assert_eq!(doc.feature_covering(&data, 15, 11), None, "a 1 × 1 tree covers its cell only");
    assert_eq!(doc.feature_covering(&data, 9, 10), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_scatter_brush_keeps_features_apart_and_on_their_ground() {
    let (data, dir) = data_with_big_rock("scatter");
    let rock = data.feature_id("BigRock").unwrap();
    // The left half under the sea (20), the right half land (40).
    let shore = |doc: &mut MapDoc| {
        let w = doc.tnt.width as usize;
        for z in 0..doc.tnt.height as usize {
            for x in 0..w / 2 {
                doc.tnt.heights[z * w + x] = 5;
            }
        }
    };
    let middles = |doc: &MapDoc| -> Vec<(f32, f32)> { doc.features().map(|((x, z), _)| ((x as f32 + 2.0) * CELL_PX, (z as f32 + 2.0) * CELL_PX)).collect() };
    let nearest = |m: &[(f32, f32)]| -> f32 {
        let mut least = f32::MAX;
        for (i, a) in m.iter().enumerate() {
            for b in &m[i + 1..] {
                least = least.min((a.0 - b.0).hypot(a.1 - b.1));
            }
        }
        least
    };
    let mut seed = 0x1234_5678;
    let thick = Scatter { radius: 2000.0, density: 10.0, ground: Ground::Land };
    let mut doc = blank();
    shore(&mut doc);
    let placed = thick.spray(&mut doc, &data, &mut seed, (512.0, 256.0), 600, |_| rock);
    let m = middles(&doc);
    assert_eq!(m.len(), placed);
    assert!(placed >= 8, "{placed} placed");
    assert!(m.iter().all(|p| p.0 > 488.0), "on land only: {m:?}");
    assert!(nearest(&m) >= 64.0, "rocks four cells wide, a size apart at the least");

    let mut sparse = blank();
    shore(&mut sparse);
    let few = Scatter { density: 1.0, ..thick }.spray(&mut sparse, &data, &mut seed, (512.0, 256.0), 600, |_| rock);
    let m = middles(&sparse);
    assert!(few >= 1 && few < placed, "{few} against {placed}");
    assert!(nearest(&m) >= 4.0 * CELL_PX * 5.5, "five and a half sizes apart at density 1");

    let mut wet = blank();
    shore(&mut wet);
    Scatter { ground: Ground::Water, ..thick }.spray(&mut wet, &data, &mut seed, (512.0, 256.0), 600, |_| rock);
    let m = middles(&wet);
    assert!(!m.is_empty() && m.iter().all(|p| p.0 < 536.0), "under the sea only: {m:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn features_are_placed_removed_and_compacted_away() {
    let data = empty_data();
    let mut doc = blank();
    assert!(doc.place_feature(&data, "TreeA", (3, 4)));
    assert!(doc.place_feature(&data, "RockB", (5, 4)));
    assert!(doc.place_feature(&data, "TreeA", (7, 4)));
    assert_eq!(doc.tnt.feature_names, ["TreeA", "RockB"]);
    assert_eq!(doc.feature_name_at(7, 4), Some("TreeA"));
    assert!(!doc.place_feature(&data, "TreeA", (64, 0)), "off the map");
    assert!(doc.remove_feature(&data, (5, 4)));
    assert_eq!(doc.feature_name_at(5, 4), None);
    let palette = ta_formats::image::Palette::default();
    let out = doc.compacted(&palette);
    assert_eq!(out.feature_names, ["TreeA"], "unused names are dropped");
    assert_eq!(out.tiles.len(), 1, "the blank map uses one of its three tiles");
    assert_eq!(out.minimap.width, 252, "a minimap is drawn");
    // What is written reads back the same.
    let back = TntMap::parse(&out.write().unwrap()).unwrap();
    assert_eq!(back.features, out.features);
    assert_eq!(back.heights, doc.tnt.heights);
}

#[test]
fn the_ota_carries_the_properties_and_start_positions() {
    let mut doc = blank();
    doc.props.name = "Twin Rivers".into();
    doc.props.description = "Two rivers; one ford".into();
    doc.props.planet = "Red Planet".into();
    doc.props.min_wind = 500.0;
    doc.props.max_wind = 2500.0;
    doc.props.tidal = 17.0;
    doc.props.gravity = 90.0;
    doc.props.surface_metal = 4.0;
    doc.starts = vec![[100.0, 200.0], [900.0, 300.0], [500.0, 400.0]];
    let text = doc.to_ota().to_bytes();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&text));
    assert_eq!(info.name, "Twin Rivers");
    assert_eq!(info.description, "Two rivers one ford", "semicolons can't be written");
    assert_eq!(info.planet, "Red Planet");
    assert_eq!((info.min_wind, info.max_wind, info.tidal_strength, info.gravity), (500.0, 2500.0, 17.0, 90.0));
    assert_eq!(info.num_players, "2, 3");
    assert_eq!(info.schemas.len(), 1);
    assert_eq!(info.schemas[0].surface_metal, 4.0);
    assert_eq!(info.schemas[0].start_positions, vec![(1, 100.0, 200.0), (2, 900.0, 300.0), (3, 500.0, 400.0)]);
}

#[test]
fn map_names_become_safe_file_names_and_mod_ids() {
    assert_eq!(save::file_stem("  Isle: of \"Doom\"? . "), "Isle of Doom");
    assert_eq!(save::file_stem("a/b\\c"), "abc");
    assert!(save::mod_id("Isle of Doom (Edited)").starts_with("map-isle-of-doom-edited-"));
    assert_eq!(save::mod_id("Twin Rivers"), save::mod_id("twin rivers"));
    assert_ne!(save::mod_id("Twin Rivers"), save::mod_id("Twin-Rivers"));
    assert_ne!(save::mod_id("沙漠"), save::mod_id("雪原"));
    assert!(save::name_problem(&ta_vfs::Vfs::new(), "CON.txt", &std::env::temp_dir()).is_some());
}

#[test]
fn undo_and_redo_return_to_saved_state_without_reusing_branch_versions() {
    let mut doc = blank();
    let mut history = History::default();
    let original = history.version;
    let before = Snapshot::of(&doc);
    assert!(before.matches(&doc));
    doc.props.description = "First revision".into();
    assert!(!before.matches(&doc));
    history.record(before, &doc, "Properties").unwrap();
    let saved = history.version;
    history.undo(&mut doc).unwrap();
    assert_eq!(history.version, original);
    history.redo(&mut doc).unwrap();
    assert_eq!(history.version, saved, "redo to the saved document is clean again");
    history.undo(&mut doc).unwrap();
    let before = Snapshot::of(&doc);
    doc.props.description = "A different revision".into();
    history.record(before, &doc, "Properties").unwrap();
    assert_ne!(history.version, saved, "a new branch must not look like the saved state");
    assert!(history.redo(&mut doc).is_none());
}

#[test]
fn failed_map_save_preserves_the_entire_previous_map_and_unrelated_content() {
    let root = std::env::temp_dir().join(format!("ta_editor_save_{}", ta_config::profiles::new_id()));
    let mut doc = blank();
    let palette = ta_formats::image::Palette::default();
    let saved = save::write_mod(&doc, &palette, &root, "Tester").unwrap();
    let tnt = saved.dir.join("maps/Test Map.tnt");
    let ota = saved.dir.join("maps/Test Map.ota");
    let old_tnt = std::fs::read(&tnt).unwrap();
    let old_ota = std::fs::read(&ota).unwrap();
    let old_manifest = std::fs::read(saved.dir.join("mod.toml")).unwrap();
    std::fs::write(saved.dir.join("README.txt"), b"user notes").unwrap();
    std::fs::remove_file(&ota).unwrap();
    std::fs::create_dir(&ota).unwrap();
    doc.tnt.heights[0] += 10;
    doc.props.description = "Changed metadata".into();
    assert!(save::write_mod(&doc, &palette, &root, "Other author").is_err());
    assert_eq!(std::fs::read(&tnt).unwrap(), old_tnt, "an OTA failure must not publish the new terrain first");
    assert_eq!(std::fs::read(saved.dir.join("mod.toml")).unwrap(), old_manifest);
    assert!(ota.is_dir());
    assert_eq!(std::fs::read(saved.dir.join("README.txt")).unwrap(), b"user notes");
    assert!(!std::fs::read_dir(&root).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().ends_with(".installing")));
    std::fs::remove_dir(&ota).unwrap();
    std::fs::write(&ota, old_ota).unwrap();
    let retried = save::write_mod(&doc, &palette, &root, "Tester").unwrap();
    assert_eq!(retried.id, saved.id);
    assert_ne!(std::fs::read(&tnt).unwrap(), old_tnt);
    assert_eq!(std::fs::read(saved.dir.join("README.txt")).unwrap(), b"user notes");

    let unrelated = root.join("Other Map");
    std::fs::create_dir_all(&unrelated).unwrap();
    std::fs::write(unrelated.join("canary"), b"unrelated installed mod").unwrap();
    doc.props.name = "Other Map".into();
    assert!(save::name_problem(&ta_vfs::Vfs::new(), &doc.props.name, &root).is_some());
    assert!(save::write_mod(&doc, &palette, &root, "Tester").is_err());
    assert_eq!(std::fs::read(unrelated.join("canary")).unwrap(), b"unrelated installed mod");
    assert!(!unrelated.join("maps").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn editor_saves_keep_legacy_ids_and_distinguish_colliding_names() {
    let root = std::env::temp_dir().join(format!("ta_editor_ids_{}", ta_config::profiles::new_id()));
    let mut doc = blank();
    let palette = ta_formats::image::Palette::default();
    let first = save::write_mod(&doc, &palette, &root, "Tester").unwrap();
    let path = first.dir.join("mod.toml");
    let legacy = std::fs::read_to_string(&path).unwrap().replace(&first.id, "map-test-map");
    std::fs::write(&path, legacy).unwrap();
    let legacy = save::write_mod(&doc, &palette, &root, "Tester").unwrap();
    assert_eq!(legacy.id, "map-test-map", "already enabled maps keep their identity");
    doc.props.name = "Test-Map".into();
    let second = save::write_mod(&doc, &palette, &root, "Tester").unwrap();
    assert_ne!(legacy.id, second.id);
    let (mods, errors) = ta_vfs::mods::scan(&root);
    assert!(errors.is_empty());
    assert_eq!(mods.len(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

/// Makes a map, edits it, saves it as a mod in a temporary mods folder, mounts that over the
/// install like the game does, and plays it for a few ticks.
#[test]
fn a_new_map_is_saved_mounted_listed_and_played() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let (palette_name, palette_doc) = ["Painted Desert", "Coast To Coast"].into_iter().find_map(|n| MapDoc::open(&vfs, n).ok().map(|d| (n, d))).expect("a stock map");
    let info = MapInfo::from_ota(palette_doc.ota.as_ref().unwrap());
    let new = NewMap { name: "Editor Test Plateau".into(), width_steps: 2, height_steps: 2, sea_level: 10, height: 40, palette_map: palette_name.into() };
    let mut doc = MapDoc::new_blank(&new, &palette_doc.tnt, Some(&info));
    assert_eq!((doc.tnt.width, doc.tnt.height), (64, 64));

    // A hill in the middle, a trench along the west.
    let mut stroke = HeightStroke::new(&doc);
    for _ in 0..30 {
        stroke.apply(&mut doc, 512.0, 512.0, 160.0, 1.0, HeightOp::Raise, 1.0 / 30.0);
        stroke.apply(&mut doc, 160.0, 700.0, 96.0, 1.0, HeightOp::Lower, 1.0 / 30.0);
    }
    assert!(doc.tnt.heights[32 * 64 + 32] > 100);
    // A tree (or a rock on worlds without trees) and a metal deposit of the planet.
    let planet = doc.props.planet.to_ascii_lowercase();
    let find = |group| (0..data.features.len()).find(|&i| super::feature_group(&data.features[i]) == group && super::world_matches(&planet, &data.features[i].world));
    let tree = find(super::FeatureGroup::Trees).or_else(|| find(super::FeatureGroup::Rocks)).expect("a tree or rock of the planet");
    let metal = find(super::FeatureGroup::Metal).expect("a deposit of the planet");
    let (tree_name, metal_name) = (data.features[tree].name.clone(), data.features[metal].name.clone());
    assert!(doc.place_feature(&data, &tree_name, (10, 50)));
    assert!(doc.place_feature(&data, &metal_name, (48, 12)));
    doc.starts = vec![[200.0, 200.0], [830.0, 830.0]];

    // Saved into a mods folder of its own.
    let mods = std::env::temp_dir().join(format!("ta-editor-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&mods);
    assert_eq!(save::name_problem(&vfs, &doc.props.name, &mods), None);
    assert!(save::name_problem(&vfs, "Coast To Coast", &mods).is_some(), "TA's own maps can't be replaced");
    let saved = save::write_mod(&doc, &data.palette, &mods, "Tester").unwrap();
    assert!(saved.dir.join("mod.toml").is_file() && saved.dir.join("maps").join("Editor Test Plateau.tnt").is_file());

    // Published as the game does after saving: enabled, mounted over the install (a new file
    // system), listed for skirmish.
    let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let maps = crate::setup::MapEntry::catalogue(&vfs);
    let mut assets = crate::setup::Assets { vfs: Arc::new(vfs), data: data.clone(), types: types.clone(), selectable_maps: maps.clone(), maps, campaigns: Vec::new(), data_hash: 0, compatibility: Default::default(), mods: Default::default() };
    let mut settings = ta_config::Config::default();
    assert!(!assets.maps.iter().any(|m| m.title == "Editor Test Plateau"));
    save::publish_in(&saved, &mut assets, &mut settings, std::slice::from_ref(&mods), false).unwrap();
    assert_eq!(settings.mods.enabled, vec![saved.id.clone()], "enabled for later sessions");
    let vfs = assets.vfs.clone();
    assert_eq!(vfs.origin("maps/Editor Test Plateau.tnt").map(|o| o.layer.clone()), Some(saved.id.clone()));
    let entry = assets.maps.iter().find(|m| m.title == "Editor Test Plateau").expect("the skirmish page lists it");
    assert_eq!(entry.start_positions, 2);
    // Saving again replaces the files; the mod is mounted already.
    save::write_mod(&doc, &data.palette, &mods, "Tester").unwrap();
    save::publish_in(&saved, &mut assets, &mut settings, std::slice::from_ref(&mods), false).unwrap();
    assert!(Arc::ptr_eq(&vfs, &assets.vfs), "not mounted twice");
    assert_eq!(settings.mods.enabled.len(), 1);
    let map = TntMap::parse(&vfs.read("maps/Editor Test Plateau.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Editor Test Plateau.ota").unwrap()));
    assert_eq!(map.heights, doc.tnt.heights, "the edited heights");
    assert_eq!(map.sea_level, 10);
    assert!(map.tiles.len() < palette_doc.tnt.tiles.len(), "only the tiles in use are written");
    assert_eq!(info.name, "Editor Test Plateau");
    assert_eq!(info.schemas[0].start_positions, vec![(1, 200.0, 200.0), (2, 830.0, 830.0)]);

    // Played: the features are in the world, commanders walk.
    let players: Vec<ta_sim::PlayerSetup> = info.schemas[0]
        .start_positions
        .iter()
        .map(|&(_, x, z)| ta_sim::PlayerSetup {
            side: "ARM".into(),
            color: 0,
            start: ta_sim::FxVec3::new(ta_sim::Fx::from_f32(x), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(z)),
            metal: ta_sim::Fx::from_int(1000),
            energy: ta_sim::Fx::from_int(1000),
            ai: false,
        })
        .collect();
    let mut sim = ta_sim::Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 7);
    let defs: Vec<usize> = sim.features.iter().map(|f| f.def).collect();
    assert!(defs.contains(&tree) && defs.contains(&metal), "the tree and the deposit stand on the map");
    assert!(sim.world.metal_under((48, 12), (1, 1)) >= data.features[metal].metal as i64, "the deposit feeds the metal map");
    let com = types.id("ARMCOM").unwrap();
    let a = sim.spawn_unit(com, 0, players[0].start, 0, true).unwrap();
    sim.spawn_unit(com, 1, players[1].start, 0, true).unwrap();
    let dest = ta_sim::FxVec3::from_ints(300, 0, 260);
    sim.step(&[ta_sim::Command::Order { player: 0, units: vec![a], order: ta_sim::Order::Move(dest), queue: false }]);
    let from = sim.unit(a).unwrap().pos;
    for _ in 0..60 {
        sim.step(&[]);
    }
    let to = sim.unit(a).unwrap().pos;
    assert!((to - from).xz().length_sq_raw() > 0, "the commander walks on the new map");
    let _ = std::fs::remove_dir_all(&mods);
}

/// Game data with one feature, a rock 4 × 4 cells (and the folder it was read from).
fn data_with_big_rock(name: &str) -> (GameData, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("ta_editor_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("features/all worlds")).unwrap();
    std::fs::write(dir.join("features/all worlds/rock.tdf"), "[BigRock]\n{\n world=all worlds;\n description=Rock;\n footprintx=4;\n footprintz=4;\n blocking=1;\n}\n").unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    (GameData::load(&vfs), dir)
}

#[test]
fn features_never_cover_one_another_nor_impassable_ground() {
    let (data, dir) = data_with_big_rock("cover");
    assert_eq!(data.feature("BigRock").map(|f| (f.footprint_x, f.footprint_z)), Some((4, 4)));
    let mut doc = blank();
    assert!(doc.place_feature(&data, "BigRock", (10, 10)));
    assert!(!doc.place_feature(&data, "TreeA", (13, 13)), "not on the rock's far corner");
    assert!(!doc.place_feature(&data, "BigRock", (8, 8)), "not overlapping it from above and to the left");
    assert!(doc.place_feature(&data, "TreeA", (14, 10)), "beside it");
    // Impassable ground (TA's void cells) stays as it is.
    let w = doc.tnt.width;
    doc.tnt.features[(20 * w + 20) as usize] = ta_formats::tnt::FEATURE_VOID;
    assert!(!doc.place_feature(&data, "TreeA", (20, 20)));
    assert!(!doc.place_feature(&data, "BigRock", (18, 18)), "its footprint would cover the void cell");
    assert!(doc.void_at(20, 20));
    // The eraser takes a big feature its circle touches, though its middle is outside it.
    assert_eq!(super::tools::erase_features(&mut doc, &data, 10.0 * 16.0 - 8.0, 12.0 * 16.0, 12.0), 1);
    assert_eq!(doc.feature_name_at(10, 10), None);
    assert!(doc.void_at(20, 20), "the eraser leaves impassable ground alone");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_selection_runs_from_where_the_drag_began() {
    let r = super::tools::tile_rect((100, 50), (0, 0), 128, 128, 64);
    assert_eq!((r.x0, r.z0, r.x1, r.z1), (37, 0, 101, 51), "64 tiles at most, ending where the drag began");
    let r = super::tools::tile_rect((10, 10), (200, 12), 128, 128, 64);
    assert_eq!((r.x0, r.z0, r.x1, r.z1), (10, 10, 74, 13), "64 tiles at most, from where it began");
    let r = super::tools::tile_rect((120, 5), (140, 5), 128, 128, 64);
    assert_eq!((r.x0, r.x1), (120, 128), "inside the map");
}

#[test]
fn start_positions_crowded_under_the_sea_or_too_few_are_warned_of() {
    use super::checks::map_problems;
    let mut doc = blank();
    assert!(map_problems(&doc).is_empty(), "{:?}", map_problems(&doc));
    doc.starts = vec![[100.0, 100.0], [150.0, 100.0]];
    assert_eq!(map_problems(&doc).len(), 1, "two starts on top of each other: {:?}", map_problems(&doc));
    doc.starts.truncate(1);
    assert!(map_problems(&doc).iter().any(|p| p.text.contains("two start positions")));
    doc.starts = vec![[100.0, 100.0], [900.0, 400.0]];
    doc.tnt.heights.fill(10);
    let problems = map_problems(&doc);
    assert_eq!(problems.iter().filter(|p| p.at.is_some()).count(), 2, "both starts under the sea, each with its place: {problems:?}");
    assert!(problems.iter().all(|p| !p.blocking), "warnings only");
}

#[test]
fn a_planets_features_are_its_worlds() {
    use super::world_matches;
    assert!(world_matches("Green Planet", "greenworld"));
    assert!(world_matches("Red Planet", "mars"));
    assert!(world_matches("Desert", "All Worlds"));
    assert!(!world_matches("Green Planet", "desert"));
    // TA Reborn's real places: the green, lush, island, desert and ice worlds' features.
    for world in ["greenworld", "lush", "archipelago", "desert", "ice"] {
        assert!(world_matches("Earth", world), "{world}");
    }
    assert!(!world_matches("Earth", "crystal") && !world_matches("Earth", "lava") && !world_matches("Earth", "metal"));
}
