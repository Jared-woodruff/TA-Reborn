//! What the mods of now make of those a record was made with ([`RecordMod`], kept from record
//! version 46): which it needs that are missing, off or another copy, and which are on that it
//! was made without. The game's saved games and replays and the launcher's battles tell a player
//! from this, in their lists and as a record is opened.
//!
//! A record's data hash says whether its battle can be played back as it was; this says why not,
//! where mods are why, and what would put it right.

use super::RecordMod;

/// What became of a mod a record was made with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Need {
    /// On, the same copy: the same files (or, where the files could not be compared, the same
    /// version).
    Same,
    /// On, the same version, but its files differ.
    OtherFiles,
    /// On, another version: this one.
    OtherVersion(String),
    /// Installed but off: its version, and whether its files are the record's (`None`: not known).
    Off { version: String, same: Option<bool> },
    /// Not installed.
    Missing,
}

/// A record's mods against those of now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModCheck {
    /// The record's mods in its load order, each with what became of it.
    pub needed: Vec<(RecordMod, Need)>,
    /// Mods on now that the record was made without, in today's load order (map packs of the
    /// Maps page among them, which change no battle but one on their maps: [`ModCheck::others`]
    /// leaves them out).
    pub extra: Vec<RecordMod>,
    /// Its mods are on, but in another load order.
    pub reordered: bool,
}

impl ModCheck {
    /// The mods of now are the record's, in its order (copies as they were, where that can be
    /// told; map packs it was made without aside).
    pub fn matches(&self) -> bool {
        self.needed.iter().all(|(_, need)| *need == Need::Same) && self.others().next().is_none() && !self.reordered
    }

    /// The mods on now that it was made without, but map packs ([`is_map_pack`]).
    pub fn others(&self) -> impl Iterator<Item = &RecordMod> {
        self.extra.iter().filter(|m| !is_map_pack(&m.id))
    }

    /// The mods it needs that are not installed.
    pub fn missing(&self) -> impl Iterator<Item = &RecordMod> {
        self.needed.iter().filter(|(_, need)| *need == Need::Missing).map(|(m, _)| m)
    }

    /// Every mod it needs is installed, and some are off, or others on, or the order is
    /// another: turning on its mods, in its order, and only those ([`ModCheck::load_order`])
    /// gives the battle its mods back (copies as installed: another version stays another).
    pub fn switchable(&self) -> bool {
        !self.matches()
            && self.missing().next().is_none()
            && (self.reordered || self.others().next().is_some() || self.needed.iter().any(|(_, need)| matches!(need, Need::Off { .. })))
    }

    /// The load order that gives the battle its mods: the record's, as the settings keep it, and
    /// after them the map packs on now that it was made without ([`is_map_pack`]: they change no
    /// battle but one on their maps, which would have named them).
    pub fn load_order(&self) -> Vec<String> {
        self.needed.iter().map(|(m, _)| m.id.clone())
            .chain(self.extra.iter().filter(|m| is_map_pack(&m.id)).map(|m| m.id.clone()))
            .collect()
    }

    /// Of the mods it needs, those on now but not as they were (another version, other files).
    pub fn changed(&self) -> impl Iterator<Item = &(RecordMod, Need)> {
        self.needed.iter().filter(|(_, need)| matches!(need, Need::OtherFiles | Need::OtherVersion(_)))
    }
}

/// Whether a mod is maps the launcher's Maps page imported (`map-pack-<digest>`): maps alone,
/// which change no battle but one on them.
pub fn is_map_pack(id: &str) -> bool {
    id.starts_with("map-pack-")
}

/// Whether two copies of a mod have the same files: `None` when either could not be read.
pub fn same_copy(a: &RecordMod, b: &RecordMod) -> Option<bool> {
    (a.digest_known() && b.digest_known()).then(|| a.digest == b.digest)
}

/// Checks a record's mods (`record`, in its load order) against the mods on now (`on`, in load
/// order) and every mod installed (`installed`: on or off; digests may be unknown).
pub fn check(record: &[RecordMod], on: &[RecordMod], installed: &[RecordMod]) -> ModCheck {
    let needed: Vec<(RecordMod, Need)> = record.iter().map(|wanted| {
        let need = if let Some(now) = on.iter().find(|m| m.id == wanted.id) {
            match same_copy(wanted, now) {
                // (The same files under another version's name are the same mod to the battle.)
                Some(true) => Need::Same,
                Some(false) if now.version == wanted.version => Need::OtherFiles,
                None if now.version == wanted.version => Need::Same,
                _ => Need::OtherVersion(now.version.clone()),
            }
        } else if let Some(there) = installed.iter().find(|m| m.id == wanted.id) {
            Need::Off { version: there.version.clone(), same: same_copy(wanted, there) }
        } else {
            Need::Missing
        };
        (wanted.clone(), need)
    }).collect();
    let extra = on.iter().filter(|m| !record.iter().any(|r| r.id == m.id)).cloned().collect();
    // (The order of the mods both have, each way.)
    let theirs: Vec<&str> = record.iter().filter(|r| on.iter().any(|m| m.id == r.id)).map(|r| r.id.as_str()).collect();
    let ours: Vec<&str> = on.iter().filter(|m| record.iter().any(|r| r.id == m.id)).map(|m| m.id.as_str()).collect();
    ModCheck { needed, extra, reordered: theirs != ours }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, version: &str, digest: u8) -> RecordMod {
        RecordMod { id: id.into(), name: id.to_uppercase(), version: version.into(), digest: [digest; 16] }
    }

    #[test]
    fn the_same_mods_in_the_same_order_match() {
        let mods = [m("a", "1", 1), m("b", "2", 2)];
        let check = check(&mods, &mods, &mods);
        assert!(check.matches() && !check.switchable());
        assert!(super::check(&[], &[], &[m("a", "1", 1)]).matches(), "made without mods, none on: whatever is installed");
    }

    #[test]
    fn missing_off_and_other_copies_are_told_apart() {
        let record = [m("gone", "1", 1), m("off", "2", 2), m("newer", "3", 3), m("edited", "4", 4), m("unread", "5", 5)];
        let on = [m("newer", "3.1", 9), m("edited", "4", 8), m("unread", "5", 0)];
        let installed = [m("off", "2", 2), on[0].clone(), on[1].clone(), on[2].clone()];
        let check = check(&record, &on, &installed);
        let needs: Vec<&Need> = check.needed.iter().map(|(_, n)| n).collect();
        assert_eq!(needs, [
            &Need::Missing,
            &Need::Off { version: "2".into(), same: Some(true) },
            &Need::OtherVersion("3.1".into()),
            &Need::OtherFiles,
            // (Its files could not be read now: the same version is taken for the same copy.)
            &Need::Same,
        ]);
        assert_eq!(check.missing().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["gone"]);
        assert_eq!(check.changed().count(), 2);
        assert!(!check.matches());
        assert!(!check.switchable(), "a mod it needs is not installed");
    }

    #[test]
    fn the_same_files_under_another_version_are_the_same_mod() {
        let check = check(&[m("a", "1.0", 1)], &[m("a", "1.0-final", 1)], &[]);
        assert!(check.matches());
    }

    #[test]
    fn mods_it_was_made_without_and_another_order_are_told() {
        let record = [m("a", "1", 1), m("b", "1", 2)];
        let installed = [m("a", "1", 1), m("b", "1", 2), m("c", "1", 3)];
        // Made with a and b; b and a are on now, and c.
        let check = check(&record, &[m("b", "1", 2), m("a", "1", 1), m("c", "1", 3)], &installed);
        assert!(check.reordered);
        assert_eq!(check.extra, [m("c", "1", 3)]);
        assert!(check.switchable());
        assert_eq!(check.load_order(), ["a", "b"]);
        // Made without mods, one on now.
        let none = super::check(&[], &[m("c", "1", 3)], &installed);
        assert!(!none.matches() && none.switchable() && none.load_order().is_empty());
        // Maps imported on the launcher's Maps page stay on: they change no other battle.
        let packs = super::check(&record[..1], &[m("map-pack-1f", "", 7), m("a", "1", 1), m("c", "1", 3)], &installed);
        assert_eq!(packs.load_order(), ["a", "map-pack-1f"]);
        assert!(is_map_pack("map-pack-1f") && !is_map_pack("mappack"));
        let pack_only = super::check(&record[..1], &[m("a", "1", 1), m("map-pack-1f", "", 7)], &installed);
        assert!(pack_only.matches() && !pack_only.switchable(), "a map pack more is no other battle");
    }

    #[test]
    fn a_mod_off_can_be_turned_on_and_an_other_version_on_cannot_be_switched_to() {
        let record = [m("a", "1", 1)];
        let off = check(&record, &[], &[m("a", "1", 1)]);
        assert_eq!(off.needed[0].1, Need::Off { version: "1".into(), same: Some(true) });
        assert!(off.switchable());
        let other_off = check(&record, &[], &[m("a", "2", 2)]);
        assert_eq!(other_off.needed[0].1, Need::Off { version: "2".into(), same: Some(false) });
        assert!(other_off.switchable(), "turned on, it is at least the mod");
        let other_on = check(&record, &[m("a", "2", 2)], &[m("a", "2", 2)]);
        assert!(!other_on.switchable(), "on already: only another copy would do");
    }
}
