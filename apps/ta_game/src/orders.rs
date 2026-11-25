//! The orders the selection can give, as the command plate lists them: each with its glyph, the
//! action its key is bound to and the group it belongs to. One list for every place that offers
//! orders (the plate, the radial menu), so that they never differ.

use ta_sim::UnitId;
use ta_ui::i18n::{t, tf};

use crate::game::{CommandMode, Game};
use crate::hud_icons::Glyph;

/// What an order does when chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Stop what the selection is doing.
    Stop,
    /// A targeted order: the next click on the battlefield gives it.
    Mode(CommandMode),
    /// Perspective: into this unit, to ride along.
    Perspective(UnitId),
    /// Upgrade in place: the selected units that upgrade become the next step of their type's
    /// upgrades (with Shift, queued after those ordered).
    Upgrade,
}

/// The rows the plate sets the orders in: moving and fighting; work (mending, reclaiming,
/// taking over, raising wrecks, extractors); and the special ones (cargo, the D-gun, Perspective).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Command,
    Work,
    Special,
}

/// One order the selection can give.
#[derive(Clone, Copy, Debug)]
pub struct Def {
    pub kind: Kind,
    pub glyph: Glyph,
    /// The key binding that gives it (`Keys::text` names the key).
    pub action: &'static str,
    pub group: Group,
}

impl Def {
    /// Its key as the player has it bound.
    pub fn key(&self, game: &Game) -> String {
        game.keys.text(self.action).to_string()
    }

    /// Its name, key and what it does, for the pointer.
    pub fn tip(&self, game: &Game) -> String {
        let key = self.key(game);
        match self.kind {
            Kind::Stop => format!("{} ({key})", t("Stop")),
            Kind::Mode(mode) => format!("{} ({key}): {}", t(mode.label_for(game)), t(mode.hint_for(game))),
            Kind::Perspective(_) => format!("{} ({key}): {}", t("Perspective"), t("See through its eyes as it goes on with its orders. In your own ground units, hovercraft and ships, E takes the controls. Escape leaves.")),
            Kind::Upgrade => upgrade_tip(game, &key),
        }
    }
}

/// UPGRADE's words: into what, its cost and how long it takes on the unit's own build power,
/// what it does meanwhile, and how many of the selection upgrade.
/// What a unit of type `from` upgrades into, by name: the new type's, with its tech level where
/// the two share a name (Supreme Commander's factories are all "Land Factory": "Land Factory T2"),
/// or, where they share that too, its step along the upgrades (the Cybran's Hives are all an
/// engineering station of tech 2, "The Hive": "The Hive II", "The Hive III").
pub fn upgrade_name(sim: &ta_sim::Sim, from: usize, to: usize) -> String {
    let name = crate::chat::unit_label(sim, to);
    if name != crate::chat::unit_label(sim, from) {
        return name;
    }
    // (Its tech level by its category's LEVELn or LEVn, as the computer players read it.)
    let level = |t: usize| {
        sim.types.get(t).def.category.split_whitespace().filter_map(|token| {
            let token = token.to_ascii_uppercase();
            token.strip_prefix("LEVEL").or_else(|| token.strip_prefix("LEV"))?.parse::<u8>().ok()
        }).max()
    };
    match level(to) {
        Some(n) if level(from) != Some(n) => tf("{name} T{n}", &[("name", &name), ("n", &n)]),
        _ => match upgrade_step(sim, to) {
            Some(step) => tf("{name} {step}", &[("name", &name), ("step", &step)]),
            None => name,
        },
    }
}

/// How far along its upgrades a type is, in Roman numerals (the second "II", the third "III"):
/// none for one nothing upgrades into.
fn upgrade_step(sim: &ta_sim::Sim, type_id: usize) -> Option<&'static str> {
    const STEPS: [&str; 9] = ["II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    let mut at = type_id;
    let mut before = 0;
    // (Back down the chain to the type nothing upgrades into, a step at a time.)
    while let Some(from) = sim.types.types.iter().find(|t| t.upgrades_to == Some(at) && t.id != type_id) {
        before += 1;
        at = from.id;
        if before >= STEPS.len() {
            break;
        }
    }
    before.checked_sub(1).and_then(|i| STEPS.get(i).copied())
}

pub fn upgrade_tip(game: &Game, key: &str) -> String {
    let Some((to, count)) = game.upgrade_offer() else { return format!("{} ({key})", t("Upgrade")) };
    let sim = &game.sim;
    let kind = sim.types.get(to);
    let from = game.selection.iter().filter_map(|&id| sim.unit(id)).find(|u| game.upgrade_step(u, false) == Some(to)).map(|u| sim.unit_type(u));
    let name = upgrade_name(sim, from.map_or(to, |f| f.id), to);
    let secs = from.filter(|f| f.worker_rate > ta_sim::Fx::ZERO).map_or(0.0, |f| (kind.build_time / f.worker_rate).to_f32() / 30.0);
    let mut lines = vec![tf("Upgrade ({key}): into {name}", &[("key", &key), ("name", &name)])];
    lines.push(tf(
        "Metal {metal}  ·  Energy {energy}  ·  {duration} on its own build power",
        &[("metal", &format!("{:.0}", kind.cost_metal.to_f32())), ("energy", &format!("{:.0}", kind.cost_energy.to_f32())), ("duration", &crate::hud::duration(secs))],
    ));
    if from.is_some_and(|f| f.is_factory) {
        lines.push(t("A factory makes nothing while it upgrades; Stop calls it off, its metal back").to_string());
    } else {
        lines.push(t("It works on while it upgrades; Stop calls it off, its metal back").to_string());
    }
    if count > 1 {
        lines.push(tf("{n} of the selection upgrade", &[("n", &count)]));
    }
    lines.push(t("Shift: queued after the upgrade under way").to_string());
    lines.join("\n")
}

/// The orders the selection can give, in the order the plate sets them: every group's in turn
/// (Stop always; Perspective for the first selected unit when it can be entered).
pub fn available(game: &Game) -> Vec<Def> {
    const MODES: [(CommandMode, Glyph, &str, Group); 14] = [
        (CommandMode::Move, Glyph::Move, "move", Group::Command),
        (CommandMode::Attack, Glyph::Attack, "attack", Group::Command),
        (CommandMode::Fight, Glyph::Fight, "fight", Group::Command),
        (CommandMode::Patrol, Glyph::Patrol, "patrol", Group::Command),
        (CommandMode::Guard, Glyph::Guard, "guard", Group::Command),
        (CommandMode::Repair, Glyph::Repair, "repair", Group::Work),
        (CommandMode::Reclaim, Glyph::Reclaim, "reclaim", Group::Work),
        (CommandMode::Capture, Glyph::Capture, "capture", Group::Work),
        (CommandMode::Resurrect, Glyph::Resurrect, "resurrect", Group::Work),
        (CommandMode::AreaMex, Glyph::AreaMex, "area_mex", Group::Work),
        (CommandMode::Load, Glyph::Load, "load", Group::Special),
        (CommandMode::Unload, Glyph::Unload, "unload", Group::Special),
        (CommandMode::DGun, Glyph::DGun, "dgun", Group::Special),
        // (Remote viewing: an eye, as what is seen there.)
        (CommandMode::RemoteView, Glyph::Visible, "remote_view", Group::Special),
    ];
    let mut out = vec![Def { kind: Kind::Stop, glyph: Glyph::Stop, action: "stop", group: Group::Command }];
    // (Stop first, then the movers' orders; the work and special rows after.)
    for (mode, glyph, action, group) in MODES {
        if game.mode_available(mode) {
            out.push(Def { kind: Kind::Mode(mode), glyph, action, group });
        }
    }
    if game.upgrade_offer().is_some() {
        out.push(Def { kind: Kind::Upgrade, glyph: Glyph::Upgrade, action: "upgrade", group: Group::Work });
    }
    if let Some(&id) = game.selection.first()
        && crate::perspective::Perspective::can_enter(game, id)
    {
        out.push(Def { kind: Kind::Perspective(id), glyph: Glyph::Perspective, action: "perspective", group: Group::Special });
    }
    out
}

/// The orders split into the rows they stand in: each group's, in `columns` to a row (a group
/// with more than that goes on a second row), the groups apart.
pub fn rows(defs: &[Def], columns: usize) -> Vec<Vec<Def>> {
    let columns = columns.max(1);
    let mut rows = Vec::new();
    for group in [Group::Command, Group::Work, Group::Special] {
        let of: Vec<Def> = defs.iter().copied().filter(|d| d.group == group).collect();
        rows.extend(of.chunks(columns).map(<[Def]>::to_vec));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(group: Group, glyph: Glyph) -> Def {
        Def { kind: Kind::Stop, glyph, action: "stop", group }
    }

    /// The groups stand in rows of their own, in order, and one wider than a row goes on.
    #[test]
    fn the_orders_stand_in_rows_by_group() {
        let defs = [
            def(Group::Command, Glyph::Stop),
            def(Group::Special, Glyph::Load),
            def(Group::Command, Glyph::Move),
            def(Group::Work, Glyph::Repair),
            def(Group::Command, Glyph::Attack),
        ];
        let rows = rows(&defs, 6);
        let glyphs: Vec<Vec<Glyph>> = rows.iter().map(|r| r.iter().map(|d| d.glyph).collect()).collect();
        assert_eq!(glyphs, [vec![Glyph::Stop, Glyph::Move, Glyph::Attack], vec![Glyph::Repair], vec![Glyph::Load]]);
        // Six to a row at most: a seventh goes on a row of its own.
        let seven: Vec<Def> = (0..7).map(|_| def(Group::Command, Glyph::Move)).collect();
        assert_eq!(super::rows(&seven, 6).iter().map(Vec::len).collect::<Vec<_>>(), [6, 1]);
        assert!(super::rows(&[], 6).is_empty());
    }

    /// What an upgrade is called: the new type's name where it has its own, its tech level where
    /// only that tells the two apart (the factories), its step along the upgrades where neither
    /// does (the Hives, all of tech 2).
    #[test]
    fn an_upgrade_is_named_by_what_tells_it_apart() {
        use std::sync::Arc;
        let dir = std::env::temp_dir().join(format!("ta-game-upgrade-names-{}", std::process::id()));
        for (name, keys) in [
            ("HIVE1", "Name=The Hive; Category=CORE LEVEL2; UpgradesTo=HIVE2;"),
            ("HIVE2", "Name=The Hive; Category=CORE LEVEL2; UpgradesTo=HIVE3;"),
            ("HIVE3", "Name=The Hive; Category=CORE LEVEL2;"),
            ("LAB1", "Name=Land Factory; Category=ARM LEVEL1; UpgradesTo=LAB2;"),
            ("LAB2", "Name=Land Factory; Category=ARM LEVEL2;"),
            ("SOLAR", "Name=Solar Collector; UpgradesTo=FUSION;"),
            ("FUSION", "Name=Fusion Reactor;"),
        ] {
            let path = dir.join("units").join(format!("{}.fbi", name.to_ascii_lowercase()));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("[UNITINFO]\n{{\nUnitName={name};\nObjectname={name};\nWorkerTime=10;\n{keys}\n}}\n")).unwrap();
        }
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let data = Arc::new(ta_data::GameData::load(&vfs));
        std::fs::remove_dir_all(&dir).ok();
        let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let map = ta_formats::tnt::TntMap { width: 16, height: 16, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![0; 256], features: vec![ta_formats::tnt::FEATURE_NONE; 256], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
        let player = ta_sim::PlayerSetup { side: "ARM".into(), color: 0, start: ta_sim::FxVec3::ZERO, metal: ta_sim::Fx::ZERO, energy: ta_sim::Fx::ZERO, ai: false };
        let sim = ta_sim::Sim::new(types, data, &map, &Default::default(), 0, &[player], 7);
        let id = |name: &str| sim.types.id(name).unwrap();
        assert_eq!(upgrade_name(&sim, id("HIVE1"), id("HIVE2")), "The Hive II");
        assert_eq!(upgrade_name(&sim, id("HIVE2"), id("HIVE3")), "The Hive III");
        assert_eq!(upgrade_name(&sim, id("HIVE1"), id("HIVE3")), "The Hive III", "a step queued past the next");
        assert_eq!(upgrade_name(&sim, id("LAB1"), id("LAB2")), "Land Factory T2");
        assert_eq!(upgrade_name(&sim, id("SOLAR"), id("FUSION")), "Fusion Reactor");
    }
}
