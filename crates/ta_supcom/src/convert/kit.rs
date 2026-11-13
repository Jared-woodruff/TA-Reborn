//! Which of TA's worlds a map becomes ([`Kit`]: its planet, and the features of TA's that stand
//! in for the game's trees, bushes, rocks, deposits and vents), and the features of the map's
//! own: TA's trees, bushes and rocks under names of their own (`SC1_Green_T1`), as passable as
//! the game's forests were and worth what its props were, written beside the map
//! (`maps/<map>.features`). Their names carry [`super::FEATURES_VERSION`], so a map always
//! names the features it was made with.

use std::collections::BTreeMap;

use ta_formats::tdf::{TdfDocument, TdfSection};

use crate::scmap::ScMap;

/// TA's own features (from the original game alone, mods left out): every section by its name
/// in lower case, with the world (its `features/<world>/` folder) it is of.
#[derive(Clone, Default)]
pub struct Templates {
    pub by_name: BTreeMap<String, (String, TdfSection)>,
}

impl Templates {
    pub fn get(&self, name: &str) -> Option<&(String, TdfSection)> {
        self.by_name.get(&name.to_ascii_lowercase())
    }

    pub fn has(&self, name: &str) -> bool {
        self.by_name.contains_key(&name.to_ascii_lowercase())
    }

    /// A feature's footprint (cells across, its larger side).
    pub fn footprint(&self, name: &str) -> u32 {
        self.get(name).map_or(1, |(_, s)| s.get_i64_or("footprintx", 1).max(s.get_i64_or("footprintz", 1)).clamp(1, 16) as u32)
    }

    /// TA's features as the original game has them: `vfs` should hold the game alone (no mods,
    /// nor TA Reborn's own), so that every player's maps are made from the same.
    pub fn load(vfs: &ta_vfs::Vfs) -> Templates {
        let docs = vfs.list("features", Some("tdf"), true).into_iter().filter_map(|p| {
            let world = p.split('/').nth(1).filter(|_| p.matches('/').count() >= 2).unwrap_or("all").to_string();
            vfs.read_opt(&p).map(|b| (world, ta_formats::tdf::parse_bytes(&b)))
        });
        Templates::from_documents(docs)
    }

    /// The templates of `files` (TDF documents, in the order read, each with its world).
    pub fn from_documents(docs: impl IntoIterator<Item = (String, TdfDocument)>) -> Templates {
        let mut by_name = BTreeMap::new();
        for (world, doc) in docs {
            for s in doc.sections {
                by_name.insert(s.name.to_ascii_lowercase(), (world.clone(), s));
            }
        }
        Templates { by_name }
    }
}

/// A world of TA's a map becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kit {
    /// In the map's own features' names (`SC1_Green_T1`).
    pub name: &'static str,
    /// The OTA's `planet`: what the renderer, the sky and the weather go by.
    pub planet: &'static str,
    /// Living trees (the first ones there are are used), bushes, rocks, deposits, vents.
    pub trees: &'static [&'static str],
    pub bushes: &'static [&'static str],
    pub rocks: &'static [&'static str],
    /// The deposit on land and under water (each about 2 metal a second to an extractor, as a
    /// mass point gives in the game).
    pub metal: &'static str,
    pub water_metal: &'static str,
    pub vents: &'static [&'static str],
}

const GREEN_TREES: &[&str] = &["Tree1", "Tree2", "Tree3", "Tree4", "Tree5", "Tree6", "Tree7"];
const GREEN_BUSHES: &[&str] = &["Shrub1", "Shrub2", "Shrub3"];
const GREEN_ROCKS: &[&str] = &["Rock", "Rock1a", "Rock2", "Rock2a", "Rock3", "Rock3a", "Rock4", "Rock4a", "Rock05"];

pub const GREEN: Kit = Kit { name: "Green", planet: "Green planet", trees: GREEN_TREES, bushes: GREEN_BUSHES, rocks: GREEN_ROCKS, metal: "RockMetal3", water_metal: "GreenAquaOre3", vents: &["GreenVent01", "GreenVent02", "GreenVent03"] };
pub const ARCHI: Kit = Kit { name: "Archi", planet: "Archipelago", trees: &["ArchiTree01", "ArchiTree02", "ArchiTree03", "ArchiTree04", "ArchiTree05", "ArchiTree06"], bushes: &["ArchiFoliage01", "ArchiFoliage02", "ArchiFoliage03", "ArchiFoliage04"], rocks: GREEN_ROCKS, metal: "ArchMetal3", water_metal: "ArchMetal3", vents: &["ArchiVent01", "ArchiVent02"] };
pub const LUSH: Kit = Kit { name: "Lush", planet: "Lush", trees: &["LushTree01", "Lush02", "Lush03", "Lush04", "Lush05", "Lush06"], bushes: &["lushfoliage01", "lushfoliage02", "lushfoliage03", "lushfoliage04", "lushfoliage05"], rocks: &["LushRock01", "LushRock02", "LushRock03", "LushRock04", "LushRock05", "LushRock06", "LushRock07"], metal: "LushMetal3", water_metal: "AquaOre3", vents: &["LushVent01", "LushVent02", "LushVent03"] };
pub const DESERT: Kit = Kit { name: "Desert", planet: "Desert", trees: &[], bushes: &[], rocks: &["DryRock01", "DryRock02", "DryRock03", "DryRock04", "DryRock05", "DryRock06", "DryRock07", "DryRock08", "DryRock09"], metal: "DryMetal03", water_metal: "DryMetal03", vents: &["DryVent01", "DryVent02", "DryVent03", "DryVent04"] };
pub const ICE: Kit = Kit { name: "Ice", planet: "Ice", trees: GREEN_TREES, bushes: &[], rocks: &["IceChunk01", "IceChunk02", "IceChunk07", "IceChunk08", "IceChunk09", "IceChunk10", "IceChunk13", "IceChunk14"], metal: "IceMetal03", water_metal: "IceAquaOre3", vents: &["IceVent01", "IceVent02", "IceVent03", "IceVent04"] };
pub const LAVA: Kit = Kit { name: "Lava", planet: "Lava", trees: &["Tree1Dead", "Tree2Dead", "Tree3Dead", "Tree4Dead", "Tree5Dead", "Tree6Dead"], bushes: &[], rocks: &["LavaRockC012", "LavaRockC015", "LavaRockC008", "LavaRockC005", "LavaRockB017", "LavaRockB012", "LavaRockB011", "LavaRockA016"], metal: "LavaMetal3", water_metal: "LavaMetal3", vents: &["Lavavent001", "Lavavent002", "Lavavent003", "Lavavent004"] };
pub const MARS: Kit = Kit { name: "Mars", planet: "Red Planet", trees: &["MarsPlant02", "MarsPlant03", "MarsPlant04", "MarsPlant05", "MarsPlant06"], bushes: &["MarsPlant01", "MarsPlant07"], rocks: &["MarsRock01", "MarsRock02", "MarsRock03", "MarsRock04", "MarsRock05", "MarsRock06", "MarsRock07", "MarsRock08"], metal: "MarsMetal03", water_metal: "MarsMetal03", vents: &["MarsVent01", "MarsVent02", "MarsVent03", "MarsVent04"] };
pub const CRYSTAL: Kit = Kit { name: "Crystal", planet: "Crystal", trees: &[], bushes: &[], rocks: &["SlateRock01", "SlateRock02", "SlateRock03", "SlateRock04", "SlateRock05", "SlateRock06", "SlateRock07", "SlateRock08"], metal: "CrystalMetal3", water_metal: "CrystalAquaOre3", vents: &["CrystalVent01", "CrystalVent02", "CrystalVent03"] };
pub const SLATE: Kit = Kit { name: "Slate", planet: "Slate", trees: &["Tree1Dead", "Tree2Dead", "Tree3Dead"], bushes: &[], rocks: &["SlateRock01", "SlateRock02", "SlateRock03", "SlateRock04", "SlateRock05", "SlateRock06", "SlateRock07", "SlateRock08"], metal: "SlateMetal4", water_metal: "SlateMetal4", vents: &["SlateVent01", "SlateVent02", "SlateVent03"] };

/// The kit of a Supreme Commander environment (`/env/<world>/...`).
fn kit_of_env(env: &str) -> Option<Kit> {
    Some(match env {
        "evergreen" | "evergreen2" => GREEN,
        "tropical" | "paradise" => ARCHI,
        "swamp" => LUSH,
        "desert" => DESERT,
        "tundra" => ICE,
        "lava" => LAVA,
        "redrocks" | "red barrens" | "redbarrens" => MARS,
        "crystalline" => CRYSTAL,
        "geothermal" => SLATE,
        _ => return None,
    })
}

/// The environment a path is of (`/env/Evergreen/Layers/x.dds` is `evergreen`).
fn env_of(path: &str) -> Option<String> {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    let rest = p.trim_start_matches('/').strip_prefix("env/")?;
    Some(rest.split('/').next()?.to_string())
}

/// The world a map is of: the environment its ground's textures and its props come from most
/// (a stratum counting as much as a hundred props), as a kit whose features TA has (else
/// TA's green world's, which every copy of TA has).
pub fn choose(map: &ScMap, templates: &Templates) -> Kit {
    let mut votes: BTreeMap<&'static str, (u64, Kit)> = BTreeMap::new();
    let mut vote = |path: &str, weight: u64| {
        if let Some(kit) = env_of(path).and_then(|e| kit_of_env(&e)) {
            votes.entry(kit.name).or_insert((0, kit)).0 += weight;
        }
    };
    for layer in &map.albedo {
        vote(&layer.path, 100);
    }
    for prop in &map.props {
        vote(&prop.blueprint, 1);
    }
    // The most votes, ties to the name first in order (the same on every computer).
    let best = votes.values().fold(None::<(u64, Kit)>, |best, &(n, k)| match best {
        Some((m, _)) if m >= n => best,
        _ => Some((n, k)),
    });
    let kit = best.map_or(GREEN, |(_, k)| k);
    if templates.has(kit.metal) && kit.vents.iter().any(|v| templates.has(v)) { kit } else { GREEN }
}

/// What a prop of the game becomes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Class {
    Tree,
    /// A group of trees in one prop: so many trees.
    Trees(u32),
    Bush,
    /// A rock: its size (world units, its larger side), the mass it is worth, and whether the
    /// game's units had to go round it (most rocks; a few small ones they drove over).
    Rock { size: f32, mass: f32, blocks: bool },
    /// Nothing TA has (markers, emitters, geysers, the cities' clutter).
    None,
}

/// What a prop blueprint is, from its path and what the blueprint says (`bp`: its reclaim and
/// size, when it could be read).
pub fn classify(path: &str, bp: Option<&crate::lua::Table>) -> Class {
    use crate::lua::Value;
    let p = path.replace('\\', "/").to_ascii_lowercase();
    let num = |keys: &[&str]| -> f32 {
        let mut t = match bp { Some(t) => t, None => return 0.0 };
        for k in &keys[..keys.len() - 1] {
            match t.get_ci(k).and_then(Value::table) {
                Some(n) => t = n,
                None => return 0.0,
            }
        }
        t.get_ci(keys[keys.len() - 1]).and_then(Value::num).unwrap_or(0.0) as f32
    };
    if ["/emitters/", "/geysers/", "/mudpots/", "/markers/", "/structures/", "/devtest/", "bridge"].iter().any(|s| p.contains(s)) {
        return Class::None;
    }
    let energy = num(&["Economy", "ReclaimEnergyMax"]);
    let mass = num(&["Economy", "ReclaimMassMax"]);
    let script = bp.and_then(|t| t.get_ci("ScriptClass")).and_then(Value::str).unwrap_or("").to_ascii_lowercase();
    let blocks = bp.and_then(|t| t.get_ci("Physics")).and_then(Value::table).and_then(|t| t.get_ci("BlockPath")).and_then(Value::bool);
    // The file's own name, which the worlds' props are named by (`boulder05`, `icerocksm04`,
    // `pine06_groupa`): the folders are not the same in every world.
    let file = p.rsplit('/').next().unwrap_or("");
    let named = |words: &[&str]| words.iter().any(|w| file.contains(w));
    if script == "treegroup" || p.contains("/trees/groups/") || (p.contains("/trees/") && file.contains("group")) {
        // (A group is worth about as much as the trees in it, each 5 to 15.)
        return Class::Trees(((energy / 10.0).round() as u32).clamp(2, 6));
    }
    if script == "tree" || p.contains("/trees/") || named(&["tree", "pine", "palm", "oak", "birch", "cypress", "ficus"]) {
        return Class::Tree;
    }
    let rock_named = named(&["rock", "boulder", "stone", "iceberg", "crystal", "pod", "lavaberg", "cliff"]);
    if p.contains("/rocks/") || p.contains("/boulders/") || p.contains("/crystals/") || rock_named || (blocks == Some(true) && mass >= 2.0) || (p.contains("/clutter/") && mass >= 5.0) {
        if mass <= 0.0 && !rock_named && !p.contains("/rocks/") {
            return Class::None;
        }
        let size = num(&["SizeX"]).max(num(&["SizeZ"])).max(0.5);
        return Class::Rock { size, mass: mass.max(1.0), blocks: blocks != Some(false) };
    }
    if p.contains("/bush") || p.contains("/logs/") || p.contains("/ferns") || named(&["bush", "fern", "shrub", "log", "cactus", "manzanita", "weed", "grass"]) || (energy > 0.0 && mass <= 0.0 && blocks != Some(true)) {
        return Class::Bush;
    }
    Class::None
}

/// The map's own features: what each of its names (`SC1_Green_T1`) is.
#[derive(Default)]
pub struct Own {
    defs: BTreeMap<String, TdfSection>,
}

/// What a rock is worth, in tiers (TA's rocks are 25 to 100; the game's 2.5 to 300).
const ROCK_TIERS: [(f32, &str, u32); 4] = [(10.0, "S", 5), (40.0, "M", 20), (120.0, "L", 60), (f32::MAX, "X", 150)];

impl Own {
    /// The name of the map's own feature standing for a prop of `class` (`variant` choosing
    /// among the kit's trees, bushes or rocks), defined from TA's template; `None` when the kit
    /// has nothing for it. `fp` is the cells a rock's footprint should cover.
    pub fn name(&mut self, kit: &Kit, templates: &Templates, class: Class, variant: usize, fp: u32) -> Option<String> {
        let available = |list: &[&'static str]| -> Vec<&'static str> { list.iter().copied().filter(|n| templates.has(n)).collect() };
        let v = super::FEATURES_VERSION;
        match class {
            Class::Tree | Class::Trees(_) => {
                let trees = available(kit.trees);
                let t = *trees.get(variant % trees.len().max(1))?;
                let name = format!("SC{v}_{}_T{}", kit.name, trees.iter().position(|&n| n == t).unwrap_or(0) + 1);
                self.define(&name, templates, t, &[("blocking", "0"), ("flamable", "0"), ("reclaimable", "1"), ("energy", "10"), ("metal", "1")]);
                Some(name)
            }
            Class::Bush => {
                let bushes = available(kit.bushes);
                let b = *bushes.get(variant % bushes.len().max(1))?;
                let name = format!("SC{v}_{}_B{}", kit.name, bushes.iter().position(|&n| n == b).unwrap_or(0) + 1);
                self.define(&name, templates, b, &[("blocking", "0"), ("flamable", "0"), ("reclaimable", "1"), ("energy", "5"), ("metal", "0")]);
                Some(name)
            }
            Class::Rock { mass, blocks, .. } => {
                let rocks = available(kit.rocks);
                if rocks.is_empty() {
                    return None;
                }
                // The rocks of the footprint nearest `fp`, then the variant among them.
                let best = rocks.iter().map(|r| templates.footprint(r).abs_diff(fp)).min()?;
                let fitting: Vec<&str> = rocks.iter().copied().filter(|r| templates.footprint(r).abs_diff(fp) == best).collect();
                let r = fitting[variant % fitting.len()];
                let &(_, tier, metal) = ROCK_TIERS.iter().find(|(top, _, _)| mass <= *top)?;
                let name = format!("SC{v}_{}_R{}{tier}{}", kit.name, rocks.iter().position(|&n| n == r).unwrap_or(0) + 1, if blocks { "" } else { "P" });
                self.define(&name, templates, r, &[("blocking", if blocks { "1" } else { "0" }), ("reclaimable", "1"), ("metal", &metal.to_string()), ("energy", "0")]);
                Some(name)
            }
            Class::None => None,
        }
    }

    fn define(&mut self, name: &str, templates: &Templates, template: &str, set: &[(&str, &str)]) {
        if self.defs.contains_key(name) {
            return;
        }
        let Some((world, t)) = templates.get(template) else { return };
        let mut s = TdfSection::new(name);
        for (k, v) in &t.entries {
            // (What it burns into, and what reclaiming it leaves, are TA's: kept.)
            s.set(k, v.clone());
        }
        for (k, v) in set {
            s.set(k, *v);
        }
        s.set("rebornworld", world.clone());
        self.defs.insert(name.to_string(), s);
    }

    /// The `maps/<map>.features` file: the definitions, in name order.
    pub fn document(&self) -> TdfDocument {
        TdfDocument { sections: self.defs.values().cloned().collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// TA's templates the tests use: a few of green's and desert's.
    pub(crate) fn templates() -> Templates {
        let green = "[Tree1]{world=greenworld;description=Tree;category=trees;footprintx=1;footprintz=1;height=40;filename=trees;seqname=leaf1;energy=250;blocking=1;flamable=1;}\
            [Tree2]{world=greenworld;description=Tree;category=trees;footprintx=1;footprintz=1;height=40;filename=trees;seqname=leaf2;energy=250;blocking=1;}\
            [Shrub1]{description=Shrub;category=shrubs;footprintx=1;footprintz=1;energy=20;}\
            [Rock2]{description=Rock;category=rocks;footprintx=3;footprintz=3;metal=100;blocking=1;}\
            [Rock4]{description=Rock;category=rocks;footprintx=2;footprintz=2;metal=75;blocking=1;}\
            [RockMetal3]{description=Metal;category=rocks;footprintx=3;footprintz=3;metal=223;indestructible=1;}\
            [GreenVent01]{description=Thermal Vent;category=steamvents;footprintx=1;footprintz=1;geothermal=1;}\
            [DragonsTeeth]{description=Dragon's Teeth;category=dragonteeth;footprintx=2;footprintz=2;metal=8;blocking=1;}";
        Templates::from_documents([("green".to_string(), ta_formats::tdf::parse(green))])
    }

    #[test]
    fn a_maps_world_is_where_its_ground_and_props_come_from() {
        let t = templates();
        let mut m = ScMap { albedo: vec![crate::scmap::Layer { path: "/env/Evergreen/layers/eg_gravel01_albedo.dds".into(), scale: 4.0 }], ..Default::default() };
        assert_eq!(choose(&m, &t), GREEN);
        // A desert's ground, but the desert's features are not in this copy of TA: green.
        m.albedo = vec![crate::scmap::Layer { path: "/env/desert/layers/des_sand_albedo.dds".into(), scale: 4.0 }];
        assert_eq!(choose(&m, &t), GREEN);
        assert_eq!(env_of("/ENV/Red Barrens/Layers/x.dds").as_deref(), Some("red barrens"));
        assert_eq!(env_of("/textures/x.dds"), None);
    }

    #[test]
    fn props_become_passable_trees_and_rocks_worth_what_the_game_made_them() {
        let t = templates();
        assert_eq!(classify("/env/evergreen/props/trees/pine06_s2_prop.bp", None), Class::Tree);
        let bp = crate::lua::parse("PropBlueprint { Economy = { ReclaimEnergyMax = 40, ReclaimMassMax = 4 }, SizeX = 2.5, SizeZ = 1 }").unwrap();
        let bp = crate::lua::top(&bp, "PropBlueprint").and_then(crate::lua::Value::table).cloned();
        assert_eq!(classify("/env/Evergreen/Props/Trees/Groups/Pine06_GroupA_prop.bp", bp.as_ref()), Class::Trees(4));
        assert_eq!(classify("/env/evergreen/props/rocks/rock01_prop.bp", bp.as_ref()), Class::Rock { size: 2.5, mass: 4.0, blocks: true });
        // Red rocks' small rocks, which the game's units drove over.
        let small = crate::lua::parse("PropBlueprint { Economy = { ReclaimMassMax = 1 }, Physics = { BlockPath = false }, SizeX = 1 }").unwrap();
        let small = crate::lua::top(&small, "PropBlueprint").and_then(crate::lua::Value::table).cloned();
        assert_eq!(classify("/env/redrocks/props/rock_sm02_prop.bp", small.as_ref()), Class::Rock { size: 1.0, mass: 1.0, blocks: false });
        assert_eq!(classify("/env/common/props/markers/m_mass_prop.bp", None), Class::None);
        let mut own = Own::default();
        assert_eq!(own.name(&GREEN, &t, Class::Tree, 1, 1).as_deref(), Some("SC1_Green_T2"));
        assert_eq!(own.name(&GREEN, &t, Class::Rock { size: 2.0, mass: 30.0, blocks: true }, 0, 2).as_deref(), Some("SC1_Green_R2M"));
        assert_eq!(own.name(&GREEN, &t, Class::Rock { size: 2.0, mass: 30.0, blocks: false }, 0, 2).as_deref(), Some("SC1_Green_R2MP"));
        assert_eq!(own.document().section("SC1_Green_R2MP").and_then(|s| s.get("blocking")), Some("0"));
        let doc = own.document();
        let tree = doc.section("SC1_Green_T2").unwrap();
        assert_eq!((tree.get("blocking"), tree.get("energy"), tree.get("seqname"), tree.get("rebornworld")), (Some("0"), Some("10"), Some("leaf2"), Some("green")));
        let rock = doc.section("SC1_Green_R2M").unwrap();
        assert_eq!((rock.get("footprintx"), rock.get("metal")), (Some("2"), Some("20")));
        // Desert's trees: none (its sands have no trees of TA's).
        assert_eq!(own.name(&DESERT, &t, Class::Tree, 0, 1), None);
    }
}
