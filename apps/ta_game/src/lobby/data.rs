//! What the network compares of the game data, and what a player is told when it differs: our
//! mods as a host tells them to a player it turns away (`ta_net::NetMod`), the maps we tell a host
//! we have (`ta_net::Msg::Maps`), and a refusal for other data in the player's words: which of
//! their mods differ from the host's and what to do, or, when none does, that the installs differ.

use ta_config::records::RecordMod;
use ta_config::records::mods::Need;
use ta_net::NetMod;
use ta_ui::i18n::{t, tf};

use crate::setup::Assets;

/// The most lines a refusal names; then how many more.
const MOST: usize = 6;

/// Our mods in load order, as a host tells them: each with whether it gives the game anything
/// besides maps (waits for their digests, worked out as the game starts, if they are not done).
pub(super) fn our_mods(assets: &Assets) -> Vec<NetMod> {
    assets.mods.record().into_iter().map(|m| NetMod { data: assets.compatibility.gives_data(&m.id), id: m.id, name: m.name, version: m.version, digest: m.digest }).collect()
}

/// The maps we have, each by its file stem with the digest of its contents, as a host is told
/// them (at most [`ta_net::MAX_MAPS`], in at most [`ta_net::BATCH_BYTES`] on the wire: well within
/// a message).
pub(super) fn our_maps(assets: &Assets) -> Vec<(String, u64)> {
    within_limits(assets.compatibility.map_digests())
}

fn within_limits(maps: Vec<(String, u64)>) -> Vec<(String, u64)> {
    let mut bytes = 0;
    maps.into_iter()
        .filter(|(name, _)| name.len() <= ta_net::MAP_NAME_MAX)
        .take(ta_net::MAX_MAPS)
        .take_while(|(name, _)| {
            // (Each as bincode writes it: the name's length, the name, the digest.)
            bytes += 16 + name.len() as u64;
            bytes <= ta_net::BATCH_BYTES
        })
        .collect()
}

fn record(m: &NetMod) -> RecordMod {
    RecordMod { id: m.id.clone(), name: m.name.clone(), version: m.version.clone(), digest: m.digest }
}

/// What differs between the host's mods (`host`, in its load order) and ours (`ours`, those on,
/// in ours), with every mod installed here (`installed`, on or off), a line each with what to do.
/// Mods of maps alone are left out on either side: the network leaves the maps out, so they are
/// never why. When no mod is why, a line says so: other game files, so the installs differ.
pub(super) fn differences(host: &[NetMod], ours: &[NetMod], installed: &[RecordMod]) -> Vec<String> {
    let gives = |id: &str, list: &[NetMod]| list.iter().any(|m| m.id == id && m.data);
    let wanted: Vec<RecordMod> = host.iter().filter(|m| m.data || gives(&m.id, ours)).map(record).collect();
    let on: Vec<RecordMod> = ours.iter().map(record).collect();
    let check = ta_config::records::mods::check(&wanted, &on, installed);
    let mut lines = Vec::new();
    for (m, need) in &check.needed {
        let name = m.label();
        let line = match need {
            Need::Same => continue,
            Need::Missing => tf("{mod} is not installed here, and the host plays with it: install it, then turn it on in the launcher's Mods page.", &[("mod", &name)]),
            Need::Off { version, .. } if !version.is_empty() && *version != m.version => {
                tf("Turn on {mod} in the launcher's Mods page: the host plays with it (yours is version {version}: you both need the same one).", &[("mod", &name), ("version", version)])
            }
            Need::Off { .. } => tf("Turn on {mod} in the launcher's Mods page: the host plays with it.", &[("mod", &name)]),
            Need::OtherVersion(version) if version.is_empty() => tf("The host plays {mod}, and yours is another copy of it: you both need the same one.", &[("mod", &name)]),
            Need::OtherVersion(version) => tf("The host plays {mod}, and yours is version {version}: you both need the same version.", &[("mod", &name), ("version", version)]),
            Need::OtherFiles => tf("Your copy of {mod} has other files than the host's: you both need the same copy of it.", &[("mod", &name)]),
        };
        lines.push(line);
    }
    for m in check.extra.iter().filter(|m| gives(&m.id, ours)) {
        lines.push(tf("Turn off {mod} in the launcher's Mods page: the host plays without it.", &[("mod", &m.label())]));
    }
    if check.reordered {
        let order = wanted.iter().map(RecordMod::label).collect::<Vec<_>>().join(", ");
        lines.push(tf("Your mods are on in another order than the host's ({mods}): put them in that order in the launcher's Mods page.", &[("mods", &order)]));
    }
    if lines.is_empty() {
        lines.push(t("Your mods are the host's, so other game files differ: your copies of Total Annihilation (or of TA Reborn) are not the same.").to_string());
    }
    lines
}

/// A refusal for other data, told: what differs, a line each (the first [`MOST`]).
pub(super) fn refusal(host: &[NetMod], ours: &[NetMod], installed: &[RecordMod]) -> String {
    let lines = differences(host, ours, installed);
    let mut text = t("Your game data is not the host's:").to_string();
    for line in lines.iter().take(MOST) {
        text.push_str("\n• ");
        text.push_str(line);
    }
    if lines.len() > MOST {
        text.push('\n');
        text.push_str(&tf("And {n} more", &[("n", &(lines.len() - MOST))]));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, version: &str, digest: u8, data: bool) -> NetMod {
        NetMod { id: id.into(), name: format!("Mod {id}"), version: version.into(), digest: [digest; 16], data }
    }

    fn installed(mods: &[NetMod]) -> Vec<RecordMod> {
        mods.iter().map(|m| RecordMod { digest: [0; 16], ..record(m) }).collect()
    }

    #[test]
    fn each_mod_that_differs_is_named_with_what_to_do() {
        let host = [m("esc", "10.2", 1, true), m("off", "1", 2, true), m("gone", "3", 3, true), m("newer", "2", 4, true), m("edited", "5", 5, true)];
        let ours = [m("newer", "1", 9, true), m("edited", "5", 8, true), m("mine", "1", 6, true)];
        let mut here = installed(&ours);
        here.extend(installed(&[m("esc", "10.2", 0, true), m("off", "0.9", 0, true)]));
        let lines = differences(&host, &ours, &here);
        assert_eq!(lines, [
            "Turn on Mod esc 10.2 in the launcher's Mods page: the host plays with it.",
            "Turn on Mod off 1 in the launcher's Mods page: the host plays with it (yours is version 0.9: you both need the same one).",
            "Mod gone 3 is not installed here, and the host plays with it: install it, then turn it on in the launcher's Mods page.",
            "The host plays Mod newer 2, and yours is version 1: you both need the same version.",
            "Your copy of Mod edited 5 has other files than the host's: you both need the same copy of it.",
            "Turn off Mod mine 1 in the launcher's Mods page: the host plays without it.",
        ]);
        let text = refusal(&host, &ours, &here);
        assert!(text.starts_with("Your game data is not the host's:\n• Turn on Mod esc 10.2"), "{text}");
        assert_eq!(text.lines().count(), 7, "a header and a line each: {text}");
    }

    /// A map pack of maps alone is never why (on either side); one that brings features too is.
    /// The same mods in another order are told; no mod at all differing says the installs do.
    #[test]
    fn map_packs_of_maps_alone_are_never_why_and_the_installs_are_told_last() {
        let pack = m("map-pack-1f", "", 7, false);
        let trees = m("map-pack-2e", "", 8, true);
        assert_eq!(differences(&[pack.clone()], &[], &[]), ["Your mods are the host's, so other game files differ: your copies of Total Annihilation (or of TA Reborn) are not the same."]);
        assert_eq!(differences(&[], &[pack.clone()], &installed(&[pack.clone()])).len(), 1, "only the installs' line");
        assert_eq!(differences(&[trees.clone()], &[], &[]), ["Mod map-pack-2e is not installed here, and the host plays with it: install it, then turn it on in the launcher's Mods page."]);
        let (a, b) = (m("a", "1", 1, true), m("b", "1", 2, true));
        let lines = differences(&[a.clone(), b.clone(), pack.clone()], &[pack, b, a], &[]);
        assert_eq!(lines, ["Your mods are on in another order than the host's (Mod a 1, Mod b 1): put them in that order in the launcher's Mods page."]);
    }

    /// The maps told to a host stay within what it takes: names of at most 255 bytes, at most
    /// 16,384 of them, and within a message (more maps, or longer names, are left out).
    #[test]
    fn the_maps_told_stay_within_the_hosts_limits() {
        let few: Vec<(String, u64)> = vec![("coast to coast".into(), 1), ("x".repeat(300), 2), ("lava run".into(), 3)];
        assert_eq!(within_limits(few), [("coast to coast".to_string(), 1), ("lava run".to_string(), 3)]);
        let many: Vec<(String, u64)> = (0..20_000).map(|i| (format!("map {i}"), i)).collect();
        assert_eq!(within_limits(many).len(), ta_net::MAX_MAPS);
        let long: Vec<(String, u64)> = (0..10_000).map(|i| (format!("{i:0>250}"), i)).collect();
        let told = within_limits(long);
        let wire = bincode::serialized_size(&ta_net::Msg::Maps(told.clone())).unwrap();
        assert!(told.len() < 10_000 && wire <= ta_net::BATCH_BYTES + 16, "{} maps, {wire} bytes", told.len());
    }

    #[test]
    fn a_long_list_is_cut_short() {
        let host: Vec<NetMod> = (0..10).map(|i| m(&format!("m{i}"), "1", i, true)).collect();
        let text = refusal(&host, &[], &[]);
        assert_eq!(text.lines().count(), 1 + MOST + 1, "{text}");
        assert!(text.ends_with("And 4 more"), "{text}");
    }
}
