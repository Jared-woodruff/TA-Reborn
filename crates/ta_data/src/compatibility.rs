//! Content digests cached once at asset load. Network peers compare everything but the maps
//! ([`Compatibility::network`]: the lobby checks the battle's own map, [`Compatibility::map_digest`]);
//! records compare shared rules and only the map/mission used by their battle.
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use ta_vfs::Vfs;

#[derive(Clone, Default)]
pub struct Compatibility {
    global: u64,
    /// Every input but the maps (`maps/`): what multiplayer peers compare.
    network: u64,
    rules: [u8; 32],
    maps: BTreeMap<String, (u64, [u8; 32])>,
    /// The campaign missions' lists of the units they allow (`camps/useonly/*.tdf`), by path.
    use_only: BTreeMap<String, (u64, [u8; 32])>,
    /// The layers (the install's `base`, TA Reborn's own, each mod by its id) that give the game
    /// at least one input outside the maps: those that make [`Compatibility::network`] what it is.
    data_layers: BTreeSet<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_ignore_other_maps_but_reject_changed_battle_inputs() {
        let root = std::env::temp_dir().join(format!("ta_match_hash_{}", std::process::id()));
        let inputs = ["units/armcom.fbi", "maps/battle.tnt", "maps/battle.ota", "maps/mission.ota", "maps/other.tnt", "camps/campaign.tdf"];
        for path in inputs {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"original").unwrap();
        }
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&root, "test").unwrap();
        let original = Compatibility::load(&vfs);
        let battle = original.for_match("Battle", Some("Mission.ota"));
        assert_eq!(battle, original.for_match("battle", Some("mission.ota")));
        for path in inputs {
            std::fs::write(root.join(path), b"changed").unwrap();
            let changed = Compatibility::load(&vfs);
            assert_ne!(original.global(), changed.global(), "network identity remains strict: {path}");
            if matches!(path, "maps/other.tnt" | "camps/campaign.tdf") {
                assert_eq!(battle, changed.for_match("battle", Some("mission.ota")), "unrelated catalogue change: {path}");
            } else {
                assert_ne!(battle, changed.for_match("battle", Some("mission.ota")), "battle input changed: {path}");
            }
            std::fs::write(root.join(path), b"original").unwrap();
        }
        assert_ne!(battle, original.for_match("battle", None));
        assert_ne!(battle, original.for_match("missing", Some("mission.ota")));
        // Save-time queries use immutable digests, with no new archive reads.
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(battle, original.for_match("battle", Some("mission.ota")));
    }

    /// A file that does not read (a damaged chunk of a mod's archive) is digested as such: the
    /// game starts, and every computer with the same files agrees.
    #[test]
    fn a_file_that_does_not_read_is_digested_not_fatal() {
        let root = std::env::temp_dir().join(format!("ta_match_unreadable_{}", std::process::id()));
        for path in ["units/a.fbi", "units/b.fbi"] {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), path).unwrap();
        }
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&root, "test").unwrap();
        let whole = Compatibility::load(&vfs);
        // Mounted, then gone: it no longer reads.
        std::fs::remove_file(root.join("units/b.fbi")).unwrap();
        let damaged = Compatibility::load(&vfs);
        assert_ne!(whole.global(), damaged.global());
        assert_eq!(damaged.global(), Compatibility::load(&vfs).global(), "the same every time");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The network's hash leaves the maps out, and only the maps: a map more, less or changed
    /// keeps it, any other input changes it. A map's own digest tells its contents.
    #[test]
    fn the_network_leaves_the_maps_out_and_the_map_digest_tells_them() {
        let root = std::env::temp_dir().join(format!("ta_network_hash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let files = ["units/armcom.fbi", "weapons/weapons.tdf", "features/all worlds/rocks.tdf", "scripts/armcom.cob", "camps/campaign.tdf", "maps/battle.tnt", "maps/battle.ota"];
        for path in files {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), path).unwrap();
        }
        let load = || {
            let mut vfs = Vfs::new();
            vfs.mount_mod_dir(&root, "test").unwrap();
            Compatibility::load(&vfs)
        };
        let original = load();
        assert_ne!(original.network(), original.global(), "a hash of its own");
        let battle = original.map_digest("Battle").expect("its terrain is there");
        assert_eq!(original.map_digest(" battle "), Some(battle), "by its stem, in any case");
        assert_eq!(original.map_digest("other"), None, "no terrain: no map");
        assert_eq!(original.map_digests(), [("battle".to_string(), battle)]);
        // Another map, as a map pack brings: the network's hash is the same.
        for path in ["maps/pack map.tnt", "maps/pack map.ota"] {
            std::fs::write(root.join(path), path).unwrap();
        }
        let packed = load();
        assert_eq!(packed.network(), original.network());
        assert_ne!(packed.global(), original.global(), "records' and caches' hash still covers it");
        assert_eq!(packed.map_digest("battle"), Some(battle));
        assert_eq!(packed.map_digests().len(), 2);
        // The battle's map changed (its terrain, or its start positions): the network's hash
        // stays, the map's digest does not.
        for path in ["maps/battle.tnt", "maps/battle.ota"] {
            std::fs::write(root.join(path), b"another version").unwrap();
            let changed = load();
            assert_eq!(changed.network(), original.network(), "{path}");
            assert_ne!(changed.map_digest("battle"), Some(battle), "{path}");
            std::fs::write(root.join(path), path).unwrap();
        }
        // Any other input changes it.
        for path in files.iter().filter(|p| !p.starts_with("maps/")) {
            std::fs::write(root.join(path), b"changed").unwrap();
            assert_ne!(load().network(), original.network(), "{path}");
            std::fs::write(root.join(path), path).unwrap();
        }
        // A map without its OTA is another map than one with it.
        std::fs::remove_file(root.join("maps/battle.ota")).unwrap();
        assert_ne!(load().map_digest("battle"), Some(battle));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A converted map's own features (`maps/<stem>.features`) are part of its digest and of its
    /// battles': another version of them is another map. They stay out of the data peers compare
    /// (a pack of such maps keeps nobody out of other battles), and a map without them keeps the
    /// digest it always had.
    #[test]
    fn a_maps_own_features_are_its_and_no_one_elses() {
        let root = std::env::temp_dir().join(format!("ta_map_features_digest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let put = |path: &str, text: &str| {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        };
        put("base/units/armcom.fbi", "[UNITINFO]{}");
        put("base/maps/plain.tnt", "plain");
        put("base/maps/plain.ota", "[GlobalHeader]{}");
        put("pack/maps/ridge (sc).tnt", "ridge");
        put("pack/maps/ridge (sc).ota", "[GlobalHeader]{}");
        let load = || {
            let mut vfs = Vfs::new();
            vfs.mount_mod_dir(&root.join("base"), "base").unwrap();
            vfs.mount_mod_dir(&root.join("pack"), "pack").unwrap();
            Compatibility::load(&vfs)
        };
        let before = load();
        put("pack/maps/ridge (sc).features", "[SC1_Green_T1]{energy=10;}");
        let with = load();
        assert_ne!(with.map_digest("Ridge (SC)"), before.map_digest("Ridge (SC)"));
        assert_ne!(with.for_match("ridge (sc)", None), before.for_match("ridge (sc)", None));
        assert_eq!(with.map_digest("plain"), before.map_digest("plain"));
        assert_eq!(with.for_match("plain", None), before.for_match("plain", None));
        assert_eq!(with.network(), before.network(), "maps' own features are no data peers compare");
        assert!(!with.gives_data("pack"));
        put("pack/maps/ridge (sc).features", "[SC1_Green_T1]{energy=20;}");
        assert_ne!(load().map_digest("ridge (sc)"), with.map_digest("ridge (sc)"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Which layers give the game something besides maps: a map pack of maps alone does not, a
    /// pack that brings features too does, and so does every mod with a unit of its own; a mod
    /// whose every file a later one replaces gives nothing.
    #[test]
    fn layers_that_give_data_besides_maps_are_told() {
        let root = std::env::temp_dir().join(format!("ta_network_layers_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let put = |layer: &str, path: &str| {
            let file = root.join(layer).join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, format!("{layer} {path}")).unwrap();
        };
        put("base", "units/armcom.fbi");
        put("maps-only", "maps/pack.tnt");
        put("maps-only", "maps/pack.ota");
        put("with-features", "maps/trees.tnt");
        put("with-features", "features/trees/trees.tdf");
        put("hidden", "units/armpw.fbi");
        put("unit-mod", "units/armpw.fbi");
        let mut vfs = Vfs::new();
        for layer in ["base", "maps-only", "with-features", "hidden", "unit-mod"] {
            vfs.mount_mod_dir(&root.join(layer), layer).unwrap();
        }
        let c = Compatibility::load(&vfs);
        let gives: Vec<bool> = ["base", "maps-only", "with-features", "hidden", "unit-mod", "absent"].iter().map(|l| c.gives_data(l)).collect();
        assert_eq!(gives, [true, false, true, false, true, false]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The units a campaign mission allows are its battle's input: a save made with one list
    /// does not pass with another. A battle without a list keeps the match's hash.
    #[test]
    fn a_battle_covers_the_units_its_mission_allows() {
        let root = std::env::temp_dir().join(format!("ta_match_useonly_{}", std::process::id()));
        for (path, text) in [("maps/battle.tnt", "map"), ("maps/mission.ota", "mission"), ("camps/useonly/Only Arm.tdf", "[ARMCOM]{}"), ("camps/useonly/other.tdf", "[CORCOM]{}")] {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), text).unwrap();
        }
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&root, "test").unwrap();
        let original = Compatibility::load(&vfs);
        let battle = original.for_battle("battle", Some("mission.ota"), Some("Only Arm"));
        assert_eq!(original.for_battle("battle", Some("mission.ota"), None), original.for_match("battle", Some("mission.ota")));
        assert_ne!(battle, original.for_match("battle", Some("mission.ota")));
        std::fs::write(root.join("camps/useonly/other.tdf"), "[ARMCOM]{}").unwrap();
        assert_eq!(Compatibility::load(&vfs).for_battle("battle", Some("mission.ota"), Some("only arm")), battle, "another mission's list");
        std::fs::write(root.join("camps/useonly/Only Arm.tdf"), "[ARMCOM]{} [ARMPW]{}").unwrap();
        assert_ne!(Compatibility::load(&vfs).for_battle("battle", Some("mission.ota"), Some("Only Arm")), battle, "its own list");
        std::fs::remove_dir_all(&root).unwrap();
    }
}

fn feed(hash: &mut Sha256, path: &str, len: u64, digest: &[u8; 32]) {
    hash.update((path.len() as u64).to_le_bytes());
    hash.update(path.as_bytes());
    hash.update(len.to_le_bytes());
    hash.update(digest);
}

fn short(hash: Sha256) -> u64 {
    u64::from_le_bytes(hash.finalize()[..8].try_into().expect("SHA-256 has eight bytes"))
}

/// What a file that does not read is digested as (no file reads as this: its length is beyond any
/// the reader accepts).
const UNREADABLE: (u64, [u8; 32]) = (u64::MAX, [0xff; 32]);

impl Compatibility {
    /// The digests of everything the battles read. A file that does not read (a damaged chunk of
    /// a mod's archive) is digested as such, the same way on every computer with the same files,
    /// and left out of the game as its loaders leave it out: it never stops the game starting.
    pub fn load(vfs: &Vfs) -> Self {
        const FOLDERS: &[&str] = &["ai", "camps", "download", "features", "gamedata", "guis", "maps", "objects3d", "scripts", "units", "weapons"];
        let paths: Vec<_> = vfs.list("", None, true).into_iter()
            .filter(|path| path.split_once('/').is_some_and(|(folder, _)| FOLDERS.contains(&folder))).collect();
        let read = |path: &String| -> (u64, [u8; 32]) {
            match vfs.read(path) {
                Ok(bytes) => (bytes.len() as u64, Sha256::digest(&bytes).into()),
                Err(e) => {
                    let from = vfs.origin(path).map_or(String::new(), |o| format!(" ({})", o.layer));
                    log::warn!("{e}{from}: left out of the game data");
                    UNREADABLE
                }
            }
        };
        // Bound concurrent decompression buffers, preserving sorted VFS order.
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
        let inputs: Vec<_> = match rayon::ThreadPoolBuilder::new().num_threads(workers).build() {
            Ok(pool) => pool.install(|| paths.par_iter().map(read).collect()),
            Err(_) => paths.iter().map(read).collect(),
        };
        let mut global = Sha256::new();
        global.update(b"TA Reborn simulation inputs v1\0");
        let mut network = Sha256::new();
        network.update(b"TA Reborn network inputs v1\0");
        let mut rules = Sha256::new();
        rules.update(b"TA Reborn record rules v1\0");
        let mut maps = BTreeMap::new();
        let mut use_only = BTreeMap::new();
        let mut data_layers = BTreeSet::new();
        for (path, (len, digest)) in paths.into_iter().zip(inputs) {
            feed(&mut global, &path, len, &digest);
            if path.starts_with("maps/") {
                // (A battle reads its own map alone: the lobby checks that one.)
                maps.insert(path, (len, digest));
                continue;
            }
            feed(&mut network, &path, len, &digest);
            if let Some(origin) = vfs.origin(&path)
                && !data_layers.contains(&origin.layer)
            {
                data_layers.insert(origin.layer.clone());
            }
            if path.starts_with("camps/useonly/") {
                // The units a mission allows: its battle's own (`for_battle`).
                use_only.insert(path, (len, digest));
            } else if !path.starts_with("camps/") {
                // Campaign catalogues select a mission; its file and difficulty are
                // already stored in GameSetup. Other missions do not affect this one.
                feed(&mut rules, &path, len, &digest);
            }
        }
        Self { global: short(global), network: short(network), rules: rules.finalize().into(), maps, use_only, data_layers }
    }

    /// Every input: what records older than version 41 were made with, and what keys caches of
    /// the data.
    pub fn global(&self) -> u64 { self.global }

    /// Every input but the maps (`maps/`): units, weapons, features, scripts, models, the AI,
    /// campaigns, all the rest. Multiplayer peers must match on it (from protocol 83); the map a
    /// lobby chose is checked on its own ([`Compatibility::map_digest`]), so a map pack one player
    /// has and another lacks keeps nobody out of a battle on other maps.
    pub fn network(&self) -> u64 { self.network }

    /// Whether `layer` (a mod's id, `base`, TA Reborn's own) gives the game an input outside the
    /// maps: a mod that does not (a map pack of maps alone, a mod every file of which a later one
    /// replaces) cannot be why two computers' [`Compatibility::network`] differ.
    pub fn gives_data(&self, layer: &str) -> bool {
        self.data_layers.contains(layer)
    }

    /// The digest of a map's contents (its `maps/<name>.tnt` and `.ota`), `name` its file stem in
    /// any case; `None` when there is no such map (no terrain). Two computers with the same map
    /// have the same digest; another version of it (another terrain, start positions, features
    /// placed...) another.
    pub fn map_digest(&self, name: &str) -> Option<u64> {
        let stem = name.trim().replace('\\', "/").to_ascii_lowercase();
        let terrain = self.maps.get(&format!("maps/{stem}.tnt"))?;
        let mut hash = Sha256::new();
        hash.update(b"TA Reborn map contents v1\0");
        feed(&mut hash, "tnt", terrain.0, &terrain.1);
        match self.maps.get(&format!("maps/{stem}.ota")) {
            Some((len, digest)) => { hash.update([1]); feed(&mut hash, "ota", *len, digest); }
            None => hash.update([0]),
        }
        // (Its own features, `maps/<stem>.features`: a converted map's.)
        if let Some((len, digest)) = self.maps.get(&format!("maps/{stem}.features")) {
            hash.update([2]);
            feed(&mut hash, "features", *len, digest);
        }
        Some(short(hash))
    }

    /// Every map's [`Compatibility::map_digest`] by its file stem (lower case), in order: what a
    /// player joining a lobby tells the host, for the host to see who has the battle's map.
    pub fn map_digests(&self) -> Vec<(String, u64)> {
        self.maps.keys()
            .filter_map(|path| path.strip_prefix("maps/")?.strip_suffix(".tnt"))
            .filter(|stem| !stem.contains('/'))
            .filter_map(|stem| Some((stem.to_string(), self.map_digest(stem)?)))
            .collect()
    }

    /// [`Self::for_match`] and the list of the units a campaign mission allows built
    /// (`camps/useonly/<use_only>.tdf`, named in its OTA), which changes what its battle can be:
    /// records from version 45. A battle without one has [`Self::for_match`]'s hash.
    pub fn for_battle(&self, map: &str, mission: Option<&str>, use_only: Option<&str>) -> u64 {
        let base = self.for_match(map, mission);
        let Some(name) = use_only.map(str::trim).filter(|name| !name.is_empty()) else { return base };
        let path = format!("camps/useonly/{}.tdf", name.replace('\\', "/").to_ascii_lowercase());
        let mut hash = Sha256::new();
        hash.update(b"TA Reborn battle use-only list v1\0");
        hash.update(base.to_le_bytes());
        match self.use_only.get(&path) {
            Some((len, digest)) => { hash.update([1]); feed(&mut hash, &path, *len, digest); }
            None => { hash.update([0]); feed(&mut hash, &path, 0, &[0; 32]); }
        }
        short(hash)
    }

    /// Map name is the serialized file stem; mission is the optional OTA filename.
    /// Missing inputs participate in the digest, so removing one cannot pass validation.
    /// No filesystem access or archive decompression occurs when saving a game.
    pub fn for_match(&self, map: &str, mission: Option<&str>) -> u64 {
        let mut paths = vec![format!("maps/{map}.tnt"), format!("maps/{map}.ota")];
        // (A map's own features, where it has them: every other map's battle digest is as it was.)
        let own = format!("maps/{map}.features");
        if self.maps.contains_key(&own.to_ascii_lowercase()) || self.maps.contains_key(&own) {
            paths.push(own);
        }
        if let Some(mission) = mission { paths.push(format!("maps/{mission}")); }
        for path in &mut paths { *path = path.replace('\\', "/").to_ascii_lowercase(); }
        paths.sort_unstable();
        paths.dedup();
        let mut hash = Sha256::new();
        hash.update(b"TA Reborn match inputs v1\0");
        hash.update(self.rules);
        for path in paths {
            match self.maps.get(&path) {
                Some((len, digest)) => { hash.update([1]); feed(&mut hash, &path, *len, digest); }
                None => { hash.update([0]); feed(&mut hash, &path, 0, &[0; 32]); }
            }
        }
        short(hash)
    }
}
