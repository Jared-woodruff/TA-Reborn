//! Typed game data loaded from the virtual file system.
//!
//! [`GameData::load`] reads every unit (`units/*.fbi`), weapon (`weapons/*.tdf`), feature
//! (`features/**/*.tdf`), movement class (`gamedata/moveinfo.tdf`) and side
//! (`gamedata/sidedata.tdf`), and assembles build menus. Parsing is tolerant: a broken file
//! in a mod is logged and skipped rather than aborting the load.

mod compatibility;
pub use compatibility::Compatibility;

pub mod feature;
pub mod map;
pub mod mission;
pub mod unit;
pub mod weapon;
pub mod validation;
mod skin_catalogue;
pub use skin_catalogue::skin_catalogue;

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use ta_formats::image::Palette;
use ta_formats::tdf::{self, TdfDocument};
use ta_vfs::Vfs;

pub use feature::FeatureDef;
pub use map::MapInfo;
pub use unit::UnitDef;
pub use weapon::{RenderType, WeaponDef};

/// Largest supported unit, feature or movement-class side, in 16-pixel map cells.
/// Keeps occupancy/yard arrays and per-placement work bounded for untrusted mods.
pub const MAX_FOOTPRINT_SIDE: u32 = 256;

fn footprint_side(section: &tdf::TdfSection, key: &str) -> u32 {
    let value = section.get_i64_or(key, 1);
    if value > MAX_FOOTPRINT_SIDE as i64 {
        log::warn!("[{}] {key}={value} exceeds the {MAX_FOOTPRINT_SIDE}-cell footprint limit; clamped", section.name);
    }
    value.clamp(1, MAX_FOOTPRINT_SIDE as i64) as u32
}

/// Stable compatibility digest of the VFS inputs used by simulation and match setup.
/// Includes raw scripts/models and every map/mission as well as definitions and build
/// menus. Only winning VFS files matter: archive layout, source paths and overridden
/// copies do not. Presentation-only sounds, textures, movies and pictures are excluded.
/// Scenarios loaded outside the VFS carry their source in the serialized game setup.
/// (Multiplayer compares [`Compatibility::network`], the same without the maps.)
pub fn compatibility_fingerprint(vfs: &Vfs) -> ta_vfs::Result<u64> {
    Ok(Compatibility::load(vfs).global())
}

/// A movement class from `gamedata/moveinfo.tdf`.
#[derive(Clone, Debug, Default)]
pub struct MoveClass {
    pub name: String,
    pub footprint_x: u32,
    pub footprint_z: u32,
    pub max_water_depth: f64,
    pub min_water_depth: f64,
    pub max_slope: f64,
    pub max_water_slope: f64,
}

/// A playable side from `gamedata/sidedata.tdf`.
#[derive(Clone, Debug, Default)]
pub struct SideDef {
    pub name: String,
    pub name_prefix: String,
    pub commander: String,
    pub interface_gaf: String,
    pub energy_color: u8,
    pub metal_color: u8,
}

pub struct GameData {
    pub palette: Palette,
    pub units: Vec<UnitDef>,
    pub weapons: Vec<WeaponDef>,
    pub features: Vec<FeatureDef>,
    pub move_classes: Vec<MoveClass>,
    pub sides: Vec<SideDef>,
    unit_index: FxHashMap<String, usize>,
    weapon_index: FxHashMap<String, usize>,
    feature_index: FxHashMap<String, usize>,
    move_index: FxHashMap<String, usize>,
}

fn read_tdf(vfs: &Vfs, path: &str) -> Option<TdfDocument> {
    vfs.read_opt(path).map(|b| tdf::parse_bytes(&b))
}

impl GameData {
    /// A diagnostic hash of typed definitions. For multiplayer and records, use
    /// [`compatibility_fingerprint`], which also covers scripts, models and match data.
    pub fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |s: &str| {
            for b in s.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        for u in &self.units {
            feed(&format!("{u:?}"));
        }
        for w in &self.weapons {
            feed(&format!("{w:?}"));
        }
        for f in &self.features {
            feed(&format!("{f:?}"));
        }
        for m in &self.move_classes {
            feed(&format!("{m:?}"));
        }
        h
    }

    pub fn load(vfs: &Vfs) -> GameData {
        let palette = vfs
            .read_opt("palettes/palette.pal")
            .and_then(|b| Palette::parse(&b).ok())
            .unwrap_or_default();

        // Units: one FBI per unit; later files with the same name override earlier ones.
        let unit_files = vfs.list("units", Some("fbi"), false);
        let parsed: Vec<Option<UnitDef>> = unit_files
            .par_iter()
            .map(|p| {
                let doc = read_tdf(vfs, p)?;
                let info = doc.section("UNITINFO")?;
                let def = UnitDef::from_tdf(info);
                if def.is_none() {
                    log::warn!("{p}: missing UnitName");
                }
                def
            })
            .collect();
        let mut units: Vec<UnitDef> = Vec::new();
        let mut unit_index = FxHashMap::default();
        for u in parsed.into_iter().flatten() {
            match unit_index.get(&u.name) {
                Some(&i) => units[i] = u,
                None => {
                    unit_index.insert(u.name.clone(), units.len());
                    units.push(u);
                }
            }
        }

        // Weapons: every top-level section of every weapons/*.tdf.
        let mut weapons: Vec<WeaponDef> = Vec::new();
        let mut weapon_index = FxHashMap::default();
        for p in vfs.list("weapons", Some("tdf"), false) {
            let Some(doc) = read_tdf(vfs, &p) else { continue };
            for s in &doc.sections {
                let w = WeaponDef::from_tdf(s);
                match weapon_index.get(&w.name) {
                    Some(&i) => weapons[i] = w,
                    None => {
                        weapon_index.insert(w.name.clone(), weapons.len());
                        weapons.push(w);
                    }
                }
            }
        }

        // Features: features/<world>/*.tdf (plus features/*.tdf for world-independent ones).
        let mut features: Vec<FeatureDef> = Vec::new();
        let mut feature_index = FxHashMap::default();
        for p in vfs.list("features", Some("tdf"), true) {
            let world = p.split('/').nth(1).filter(|_| p.matches('/').count() >= 2).unwrap_or("all").to_string();
            let Some(doc) = read_tdf(vfs, &p) else { continue };
            for s in &doc.sections {
                let f = FeatureDef::from_tdf(s, &world);
                let key = f.name.to_ascii_lowercase();
                match feature_index.get(&key) {
                    Some(&i) => features[i] = f,
                    None => {
                        feature_index.insert(key, features.len());
                        features.push(f);
                    }
                }
            }
        }

        let mut move_classes = Vec::new();
        let mut move_index = FxHashMap::default();
        if let Some(doc) = read_tdf(vfs, "gamedata/moveinfo.tdf") {
            for s in &doc.sections {
                let Some(name) = s.get("Name") else { continue };
                let mc = MoveClass {
                    name: name.trim().to_ascii_uppercase(),
                    footprint_x: footprint_side(s, "FootprintX"),
                    footprint_z: footprint_side(s, "FootprintZ"),
                    max_water_depth: s.get_f64_or("MaxWaterDepth", 10000.0),
                    min_water_depth: s.get_f64_or("MinWaterDepth", -10000.0),
                    max_slope: s.get_f64_or("MaxSlope", 255.0),
                    max_water_slope: s.get_f64_or("MaxWaterSlope", 255.0),
                };
                move_index.insert(mc.name.clone(), move_classes.len());
                move_classes.push(mc);
            }
        }

        let mut sides = Vec::new();
        let side_doc = read_tdf(vfs, "gamedata/sidedata.tdf");
        if let Some(doc) = &side_doc {
            for s in doc.sections.iter().filter(|s| s.name.to_ascii_uppercase().starts_with("SIDE")) {
                sides.push(SideDef {
                    name: s.get_str("name", "").to_ascii_uppercase(),
                    name_prefix: s.get_str("nameprefix", "").to_ascii_uppercase(),
                    commander: s.get_str("commander", "").to_ascii_uppercase(),
                    interface_gaf: s.get_str("intgaf", ""),
                    energy_color: s.get_i32_or("energycolor", 208).clamp(0, 255) as u8,
                    metal_color: s.get_i32_or("metalcolor", 224).clamp(0, 255) as u8,
                });
            }
        }

        let mut data = GameData {
            palette,
            units,
            weapons,
            features,
            move_classes,
            sides,
            unit_index,
            weapon_index,
            feature_index,
            move_index,
        };
        data.build_menus(vfs, side_doc.as_ref());
        log::info!(
            "game data: {} units, {} weapons, {} features, {} movement classes, {} sides",
            data.units.len(),
            data.weapons.len(),
            data.features.len(),
            data.move_classes.len(),
            data.sides.len()
        );
        data
    }

    /// Build options come from three places, merged in menu order:
    /// 1. the per-builder GUI menus `guis/<BUILDER><page>.gui` (button order = menu order),
    /// 2. `[CANBUILD]` in sidedata.tdf (3.1 patch),
    /// 3. `download/*.tdf` `[MENUENTRYn]` records (Core Contingency, Battle Tactics, UFOs).
    fn build_menus(&mut self, vfs: &Vfs, side_doc: Option<&TdfDocument>) {
        let mut menus: FxHashMap<String, Vec<(u32, u32, String)>> = FxHashMap::default();
        let add = |menus: &mut FxHashMap<String, Vec<(u32, u32, String)>>, builder: &str, page: u32, slot: u32, unit: &str| {
            let list = menus.entry(builder.to_ascii_uppercase()).or_default();
            let unit = unit.trim().to_ascii_uppercase();
            if !list.iter().any(|(_, _, u)| *u == unit) {
                list.push((page, slot, unit));
            }
        };
        for u in &self.units {
            // Any unit with build power has the menu its pages lay out, whatever its Builder flag:
            // the mods' buildings make their own upgrade modules so (Total Mayhem's geothermal
            // plants and moho mines, TA: Escalation's Aegis: `Builder=0` with a `WorkerTime`).
            if !u.builder && u.worker_time <= 0.0 {
                continue;
            }
            for page in 1..=16u32 {
                let path = format!("guis/{}{}.gui", u.name.to_ascii_lowercase(), page);
                let Some(doc) = read_tdf(vfs, &path) else {
                    if page > 1 {
                        break;
                    }
                    continue;
                };
                let mut slot = 0;
                for g in &doc.sections {
                    let Some(common) = g.section("COMMON") else { continue };
                    // Build buttons are gadgets of type 1 whose name is a unit, marked as unit
                    // production (commonattribs bit 4). The panel's own tab and order buttons
                    // aren't: Core's BUILD tab is named CORBUILD, which is also a campaign
                    // unit's name (the Hydration Plant), so every Core builder offered it. A
                    // hand-made page without the attribute keeps the old reading.
                    if common.get_i32_or("id", -1) != 1 || common.get_i32_or("commonattribs", 4) & 4 == 0 {
                        continue;
                    }
                    let name = common.get("name").unwrap_or("");
                    if self.unit_index.contains_key(&name.trim().to_ascii_uppercase()) {
                        add(&mut menus, &u.name, page, slot, name);
                        slot += 1;
                    }
                }
            }
        }
        if let Some(cb) = side_doc.and_then(|d| d.section("CANBUILD")) {
            for builder in &cb.sections {
                for (i, (_, unit)) in builder.entries.iter().enumerate() {
                    add(&mut menus, &builder.name, 100, i as u32, unit);
                }
            }
        }
        for p in vfs.list("download", Some("tdf"), false) {
            let Some(doc) = read_tdf(vfs, &p) else { continue };
            for e in &doc.sections {
                let (Some(builder), Some(unit)) = (e.get("UNITMENU"), e.get("UNITNAME")) else { continue };
                let page = e.get_i32_or("MENU", 1).max(1) as u32;
                let slot = e.get_i32_or("BUTTON", 0).max(0) as u32;
                add(&mut menus, builder, page, slot, unit);
            }
        }
        for (builder, mut list) in menus {
            let Some(&i) = self.unit_index.get(&builder) else { continue };
            // (Without build power a unit builds nothing, whatever its pages and lists name: Total
            // Mayhem's and TA: Escalation's silos list missile pseudo-units for their computer
            // players. None of TA's own units has a Builder flag without a WorkerTime.)
            if self.units[i].worker_time <= 0.0 {
                continue;
            }
            list.sort_by_key(|(page, slot, _)| (*page, *slot));
            self.units[i].build_options =
                list.into_iter().map(|(_, _, u)| u).filter(|u| self.unit_index.contains_key(u)).collect();
        }
    }

    pub fn unit(&self, name: &str) -> Option<&UnitDef> {
        self.unit_index.get(&name.to_ascii_uppercase()).map(|&i| &self.units[i])
    }

    pub fn unit_id(&self, name: &str) -> Option<usize> {
        self.unit_index.get(&name.to_ascii_uppercase()).copied()
    }

    pub fn weapon(&self, name: &str) -> Option<&WeaponDef> {
        self.weapon_index.get(&name.to_ascii_uppercase()).map(|&i| &self.weapons[i])
    }

    pub fn weapon_id(&self, name: &str) -> Option<usize> {
        self.weapon_index.get(&name.to_ascii_uppercase()).copied()
    }

    pub fn feature(&self, name: &str) -> Option<&FeatureDef> {
        self.feature_index.get(&name.to_ascii_lowercase()).map(|&i| &self.features[i])
    }

    pub fn feature_id(&self, name: &str) -> Option<usize> {
        self.feature_index.get(&name.to_ascii_lowercase()).copied()
    }

    pub fn move_class(&self, name: &str) -> Option<&MoveClass> {
        self.move_index.get(&name.to_ascii_uppercase()).map(|&i| &self.move_classes[i])
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;

    #[test]
    fn menu_tabs_named_like_units_are_not_build_options() {
        let root = std::env::temp_dir().join(format!("ta_data_menu_tabs_{}", std::process::id()));
        let files = [
            ("units/coraap.fbi", "[UNITINFO]{UnitName=CORAAP;Builder=1;WorkerTime=200;}"),
            ("units/coraca.fbi", "[UNITINFO]{UnitName=CORACA;}"),
            ("units/corbuild.fbi", "[UNITINFO]{UnitName=CORBUILD;Name=Hydration Plant;}"),
            ("units/corhand.fbi", "[UNITINFO]{UnitName=CORHAND;Builder=1;WorkerTime=200;}"),
            // TA's page: the BUILD tab (CORBUILD, no production attribute) and a build button.
            ("guis/coraap1.gui", "[A]{[COMMON]{id=1;name=CORBUILD;attribs=16;commonattribs=0;}} [B]{[COMMON]{id=1;name=CORACA;attribs=32;commonattribs=4;}}"),
            // A hand-made page without the attribute still reads its buttons.
            ("guis/corhand1.gui", "[A]{[COMMON]{id=1;name=CORACA;}}"),
        ];
        for (path, text) in files {
            let p = root.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&root, "test").unwrap();
        let data = GameData::load(&vfs);
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(data.unit("CORAAP").unwrap().build_options, ["CORACA"]);
        assert_eq!(data.unit("CORHAND").unwrap().build_options, ["CORACA"]);
    }

    #[test]
    fn hostile_unit_and_feature_footprints_are_bounded() {
        let doc = tdf::parse("[UNITINFO]{UnitName=HOSTILE;FootprintX=4294967295;FootprintZ=2;} [rock]{footprintx=999999999999999999999;footprintz=-3;}");
        let unit = UnitDef::from_tdf(&doc.sections[0]).unwrap();
        assert_eq!((unit.footprint_x, unit.footprint_z), (MAX_FOOTPRINT_SIDE, 2));
        let feature = FeatureDef::from_tdf(&doc.sections[1], "test");
        assert_eq!((feature.footprint_x, feature.footprint_z), (MAX_FOOTPRINT_SIDE, 1));
    }

    #[test]
    fn compatibility_covers_raw_simulation_inputs_and_vfs_precedence() {
        let root = std::env::temp_dir().join(format!("ta_data_compatibility_{}", std::process::id()));
        let first = root.join("first");
        let second = root.join("second");
        let paths = [
            "ai/default.tdf", "camps/mission.tdf", "download/unit.tdf", "features/rock.tdf",
            "gamedata/sidedata.tdf", "guis/armcom1.gui", "maps/test.tnt", "maps/test.ota",
            "objects3d/armcom.3do", "scripts/armcom.cob", "units/armcom.fbi", "weapons/laser.tdf",
        ];
        for dir in [&first, &second] {
            for path in paths {
                let path = dir.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, b"original").unwrap();
            }
        }
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&first, "base").unwrap();
        let original = compatibility_fingerprint(&vfs).unwrap();
        for path in paths {
            std::fs::write(first.join(path), b"changed").unwrap();
            assert_ne!(compatibility_fingerprint(&vfs).unwrap(), original, "{path} affects compatibility");
            std::fs::write(first.join(path), b"original").unwrap();
        }
        let mut elsewhere = Vfs::new();
        elsewhere.mount_mod_dir(&second, "different layer name").unwrap();
        assert_eq!(compatibility_fingerprint(&elsewhere).unwrap(), original, "filesystem location is irrelevant");
        vfs.mount_mod_dir(&second, "override").unwrap();
        std::fs::write(first.join("maps/test.tnt"), b"hidden changed map").unwrap();
        assert_eq!(compatibility_fingerprint(&vfs).unwrap(), original, "only the winning file matters");
        std::fs::remove_file(second.join("scripts/armcom.cob")).unwrap();
        // An input that does not read never stops the game starting, and is no partial hash: it
        // is digested as unreadable, unlike the file it was and unlike no file at all.
        let unreadable = compatibility_fingerprint(&vfs).unwrap();
        assert_ne!(unreadable, original, "an unreadable input is not passed off as the file it was");
        let mut missing = Vfs::new();
        missing.mount_mod_dir(&second, "missing script").unwrap();
        let without = compatibility_fingerprint(&missing).unwrap();
        assert_ne!(without, original, "a missing file changes the digest");
        assert_ne!(unreadable, without, "nor as a file that was never there");
        std::fs::remove_dir_all(root).unwrap();
    }
}
