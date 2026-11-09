//! TA's explosion animations, put in the sprite atlas as a battle starts: those the weapons
//! name (their blasts on land, in water and in lava) and TA's own blasts Reborn's effects use
//! (`fx/explode2` to `explode4`: a dying unit's, its pieces', a wreck's crash).
//!
//! Every frame goes into the one sprite atlas the map's features use too. Each animation's
//! file is read and its frames decoded one at a time (`GafReader`), each dropped once it is in
//! the atlas. A mod's animations may be painted far bigger than TA's (TA: Escalation's are as
//! wide as their blasts, up to 3240 x 3240, 1.33 billion pixels in all, 5 GiB as textures):
//! their files are read once for their frames' sizes, and the set planned (`gaf::plan`) to take
//! at most `gaf::EXPLOSION_TEXELS` (half the largest atlas the card takes, if less), the largest
//! drawn reduced, each as big as painted with fewer texels; then each frame is read again and
//! reduced as its rows are read. A set that fits is drawn exactly as painted.

use std::collections::{BTreeMap, HashMap};

use rayon::prelude::*;
use ta_formats::gaf::{self, AnimSize, FrameHeader, GafReader, NATIVE_SIDE};
use ta_formats::image::Palette;
use ta_render::SpriteId;
use ta_render::sprites::SpriteAtlas;
use ta_sim::Sim;
use ta_vfs::Vfs;

use super::{AnimLook, LookSum};

/// TA's own blasts Reborn's effects use by name (`Effects::generic_blasts`).
const GENERIC: [&str; 4] = ["explode2", "explode3", "explode4", "explosion"];

/// A file kept from the sizes' reading for the frames' (small ones only, up to this much).
const KEPT_FILE: usize = 4 << 20;
const KEPT_FILES: usize = 64 << 20;

pub(super) struct Explosions {
    /// Each animation's frames, and what it looks like.
    pub anims: Vec<Vec<SpriteId>>,
    pub looks: Vec<AnimLook>,
    /// Per weapon: (land, water, lava) animation indices.
    pub weapon_anims: Vec<[Option<usize>; 3]>,
    /// TA's own blasts ("fx/explode3"...) where they are TA's size (see [`load`]).
    pub generic: HashMap<String, usize>,
}

/// An animation wanted: from which file, by which name; for which weapons or Reborn's own use.
struct Wanted {
    file: String,
    entry: String,
    key: String,
    by_weapon: bool,
    size: Option<AnimSize>,
}

/// Loads the explosions into `atlas` (see the module's notes).
pub(super) fn load(vfs: &Vfs, atlas: &mut SpriteAtlas, palette: &Palette, sim: &Sim) -> Explosions {
    let started = std::time::Instant::now();
    let mut wanted: Vec<Wanted> = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut want = |gaf: &str, entry: &str, by_weapon: bool| -> Option<usize> {
        if gaf.trim().is_empty() || entry.trim().is_empty() {
            return None;
        }
        let (file, entry) = (gaf.trim().to_ascii_lowercase(), entry.trim().to_ascii_lowercase());
        let key = format!("{file}/{entry}");
        let i = *by_key.entry(key.clone()).or_insert_with(|| {
            wanted.push(Wanted { file, entry, key, by_weapon: false, size: None });
            wanted.len() - 1
        });
        wanted[i].by_weapon |= by_weapon;
        Some(i)
    };
    let weapon_wants: Vec<[Option<usize>; 3]> = sim
        .weapon_types
        .types
        .iter()
        .map(|w| {
            let d = &w.def;
            [want(&d.explosion_gaf, &d.explosion_art, true), want(&d.water_explosion_gaf, &d.water_explosion_art, true), want(&d.lava_explosion_gaf, &d.lava_explosion_art, true)]
        })
        .collect();
    let generic_wants: Vec<(String, usize)> = GENERIC.iter().filter_map(|e| want("fx", e, false).map(|i| (format!("fx/{e}"), i))).collect();
    let mut files: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, w) in wanted.iter().enumerate() {
        files.entry(w.file.clone()).or_default().push(i);
    }

    // The sizes: each file read once (the small ones kept for the frames).
    let mut kept: HashMap<String, Vec<u8>> = HashMap::new();
    let mut kept_bytes = 0usize;
    for (file, list) in &files {
        let Some(bytes) = vfs.read_opt(&path(file)) else { continue };
        match GafReader::open(&bytes) {
            Ok(reader) => {
                for &i in list {
                    wanted[i].size = reader.entry(&wanted[i].entry).map(|e| e.size());
                }
            }
            Err(e) => log::warn!("anims/{file}.gaf: {e}"),
        }
        if bytes.len() <= KEPT_FILE && kept_bytes + bytes.len() <= KEPT_FILES {
            kept_bytes += bytes.len();
            kept.insert(file.clone(), bytes);
        }
    }
    // TA's blasts in Reborn's own effects are sized for TA's art: a mod's of another kind
    // under their names (Escalation's are shield bubbles 760 to 1140 px across) are not used
    // (nor loaded, unless a weapon names them).
    let generic_wants: Vec<(String, usize)> = generic_wants
        .into_iter()
        .filter(|&(_, i)| match wanted[i].size {
            Some(size) if size.largest > NATIVE_SIDE => {
                log::info!("explosion {}: {} px across, not TA's blast of that name: Reborn's own effects go without it", wanted[i].key, size.largest);
                false
            }
            _ => true,
        })
        .collect();
    for w in wanted.iter_mut().filter(|w| !w.by_weapon) {
        w.size = w.size.filter(|s| s.largest <= NATIVE_SIDE);
    }

    let loading: Vec<usize> = (0..wanted.len()).filter(|&i| wanted[i].size.is_some_and(|s| s.frames > 0)).collect();
    let sizes: Vec<AnimSize> = loading.iter().map(|&i| wanted[i].size.unwrap_or_default()).collect();
    let plan = gaf::plan(&sizes, gaf::explosion_budget(atlas.max_side()));
    let scale_of: HashMap<usize, f64> = loading.iter().zip(&plan.scales).map(|(&i, &s)| (i, s)).collect();

    // The frames, file by file.
    let mut anims: Vec<Vec<SpriteId>> = Vec::new();
    let mut looks: Vec<AnimLook> = Vec::new();
    let mut loaded: HashMap<usize, usize> = HashMap::new();
    for (file, list) in &files {
        let list: Vec<usize> = list.iter().copied().filter(|i| scale_of.contains_key(i)).collect();
        if list.is_empty() {
            continue;
        }
        let Some(bytes) = kept.remove(file).or_else(|| vfs.read_opt(&path(file))) else { continue };
        let Ok(reader) = GafReader::open(&bytes) else { continue };
        for i in list {
            let Some(entry) = reader.entry(&wanted[i].entry) else { continue };
            let (frames, look) = frames_of(&reader, &entry.frames, scale_of[&i], palette, atlas, &wanted[i].key);
            if frames.is_empty() {
                continue;
            }
            log::debug!("explosion {}: {look:?}{}", wanted[i].key, if scale_of[&i] > 1.0 { format!(", drawn reduced by {:.2}", scale_of[&i]) } else { String::new() });
            loaded.insert(i, anims.len());
            anims.push(frames);
            looks.push(look);
        }
    }

    let anim = |w: Option<usize>| w.and_then(|i| loaded.get(&i).copied());
    let weapon_anims = weapon_wants.iter().map(|set| set.map(anim)).collect();
    let generic = generic_wants.into_iter().filter_map(|(key, i)| anim(Some(i)).map(|a| (key, a))).collect();
    let (w, h) = atlas.size();
    log::info!(
        "effects: {} explosion animations in {} ms, {:.0} of {:.0} Mi texels{}; sprite atlas {w} x {h} ({} MiB)",
        anims.len(),
        started.elapsed().as_millis(),
        plan.drawn as f64 / f64::from(1 << 20),
        plan.full as f64 / f64::from(1 << 20),
        if plan.reduced() > 0 { format!(" ({} drawn reduced, at {:.0}% of their size)", plan.reduced(), plan.density * 100.0) } else { String::new() },
        (u64::from(w) * u64::from(h) * 4) >> 20,
    );
    Explosions { anims, looks, weapon_anims, generic }
}

fn path(file: &str) -> String {
    format!("anims/{file}.gaf")
}

/// An animation's frames put in the atlas, as painted (`scale` 1: exactly as TA's are) or
/// reduced (each read on its own thread, then put in in order), and what it looks like.
fn frames_of(reader: &GafReader, frames: &[FrameHeader], scale: f64, palette: &Palette, atlas: &mut SpriteAtlas, key: &str) -> (Vec<SpriteId>, AnimLook) {
    let mut look = LookSum::default();
    let mut ids = Vec::with_capacity(frames.len());
    if scale <= 1.0 {
        for f in frames {
            match reader.frame(f) {
                Ok(frame) => {
                    look.add(&frame, palette);
                    ids.extend(atlas.add_gaf(&frame, palette, false));
                }
                Err(e) => log::warn!("explosion {key}: {e}"),
            }
        }
    } else {
        let reduced: Vec<_> = frames.par_iter().map(|f| reader.reduced(f, scale, palette)).collect();
        for r in reduced {
            match r {
                Ok(frame) => {
                    look.add_reduced(&frame);
                    ids.extend(atlas.add_reduced(&frame));
                }
                Err(e) => log::warn!("explosion {key}: {e}"),
            }
        }
    }
    (ids, look.look())
}
