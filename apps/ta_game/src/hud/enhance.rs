//! Enhancements in words (`[ENHANCEMENTS]`: Supreme Commander's commanders'): what each does, a
//! line each, for its tip in the build menu's ENHANCE tab, the information page and the classic
//! interface's page; and what the player may do with it now.

use ta_sim::{EnhancementFit as Fit, Fx, Unit, UnitType};
use ta_ui::i18n::{t, tf};

use crate::game::Game;

/// What a click on an enhancement asks: install it (queued after the unit's orders), or take it
/// out (an installed one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnhanceClick {
    Install { index: u16, queue: bool },
    Remove { index: u16 },
}

/// An enhancement's name (its id where it has none).
pub(crate) fn name(ty: &UnitType, index: u16) -> String {
    ty.def.enhancements.get(index as usize).map_or_else(String::new, |e| e.name.clone())
}

/// A whole number, as the figures are shown.
fn whole(v: Fx) -> String {
    format!("{:.0}", v.to_f32())
}

/// What enhancement `index` of `ty` does, a line each: its effects as the engine plays them.
pub(crate) fn effects(game: &Game, ty: &UnitType, index: u16) -> Vec<String> {
    let Some(e) = ty.enhancements.get(index as usize) else { return Vec::new() };
    let sim = &game.sim;
    let mut out = Vec::new();
    if !e.build_options.is_empty() {
        let names: Vec<String> = e.build_options.iter().take(4).map(|&o| crate::chat::unit_label(sim, o)).collect();
        let more = e.build_options.len().saturating_sub(4);
        out.push(if more > 0 {
            tf("Builds {n} more kinds: {names} and {more} others", &[("n", &e.build_options.len()), ("names", &names.join(", ")), ("more", &more)])
        } else {
            tf("Builds {n} more kinds: {names}", &[("n", &e.build_options.len()), ("names", &names.join(", "))])
        });
    }
    if e.work_rate > 0 {
        out.push(tf("Build power +{n}", &[("n", &whole(Fx(e.work_rate as i64)))]));
    }
    if e.max_health > Fx::ZERO {
        out.push(tf("Health +{n}", &[("n", &whole(e.max_health))]));
    }
    if e.heal_rate > Fx::ZERO {
        out.push(tf("Mends {n} hit points a second", &[("n", &whole(e.heal_rate))]));
    }
    for slot in 0..e.weapons.len() {
        let Some(w) = e.weapons[slot] else { continue };
        let w = sim.weapon_types.get(w);
        let label = crate::perspective::weapon_name(w);
        let reach = whole(w.range);
        match ty.weapons[slot] {
            Some(old) if old != w.id => {
                let old = sim.weapon_types.get(old);
                if w.range != old.range {
                    out.push(tf("{weapon}: range {from} to {to}", &[("weapon", &label), ("from", &whole(old.range)), ("to", &reach)]));
                } else {
                    out.push(tf("{weapon}: stronger", &[("weapon", &label)]));
                }
            }
            Some(_) => {}
            None => out.push(tf("Weapon: {weapon}, range {range}", &[("weapon", &label), ("range", &reach)])),
        }
    }
    if e.sight > Fx::ZERO {
        out.push(tf("Sight +{n}", &[("n", &whole(e.sight))]));
    }
    if e.radar > Fx::ZERO {
        out.push(tf("Radar +{n}", &[("n", &whole(e.radar))]));
    }
    if e.sonar > Fx::ZERO {
        out.push(tf("Sonar +{n}", &[("n", &whole(e.sonar))]));
    }
    if let Some((cost, _, _)) = e.cloak {
        out.push(tf("Cloaks for {rate} energy a second", &[("rate", &whole(cost))]));
    }
    if e.stealth {
        out.push(t("Stealthy: not on radar").to_string());
    }
    // TA Reborn's abilities: jamming, fields, remote viewing, an aura, what it makes and takes.
    out.extend(super::abilities::lines(&e.abilities, true));
    out.extend(super::abilities::production_lines(e));
    out.extend(super::logistics::enhancement_lines(&game.sim, e));
    out
}

/// The slot an enhancement takes, by name: Supreme Commander's own ids in words (its commanders'
/// "LCH" and "RCH" are their left and right arms, "Back" their back), a mod's other names as it
/// gives them.
pub(crate) fn slot_name(ty: &UnitType, index: u16) -> String {
    let slot = ty.enhancements.get(index as usize).and_then(|e| ty.enhancement_slots.get(e.slot as usize)).cloned().unwrap_or_default();
    match slot.to_ascii_uppercase().as_str() {
        "LCH" => t("Left arm").to_string(),
        "RCH" => t("Right arm").to_string(),
        "BACK" => t("Back").to_string(),
        _ => slot,
    }
}

/// How long `u` takes to install it on its own build power.
pub(crate) fn seconds(u: &Unit, ty: &UnitType, index: u16) -> f32 {
    let Some(e) = ty.enhancements.get(index as usize) else { return 0.0 };
    let rate = u.work_rate(ty).max(1);
    (e.build_work as f64 / rate as f64 / 30.0) as f32
}

/// An enhancement's tip: its name, slot, price and time, what it needs, what it does, and what a
/// click does now.
pub(crate) fn tip(game: &Game, u: &Unit, ty: &UnitType, index: u16) -> String {
    let Some(e) = ty.enhancements.get(index as usize) else { return String::new() };
    let mut lines = vec![name(ty, index)];
    if let Some(d) = ty.def.enhancements.get(index as usize).map(|d| d.description.trim()).filter(|d| !d.is_empty()) {
        lines.push(d.to_string());
    }
    lines.push(tf(
        "Slot {slot}  ·  Metal {metal}  ·  Energy {energy}  ·  {duration} on its own build power",
        &[("slot", &slot_name(ty, index)), ("metal", &whole(e.cost_metal)), ("energy", &whole(e.cost_energy)), ("duration", &super::duration(seconds(u, ty, index)))],
    ));
    if let Some(p) = e.prerequisite {
        lines.push(tf("Needs {name} first", &[("name", &name(ty, p))]));
    }
    lines.extend(effects(game, ty, index));
    lines.push(match game.sim.enhancement_fit(u, index) {
        Fit::Installed => t("Installed  ·  Right-click: take it out").to_string(),
        Fit::Installing => t("Being installed  ·  Stop calls it off, its metal back").to_string(),
        Fit::Needs(p) => tf("Click: {name} first, then this one  ·  Shift: queued", &[("name", &name(ty, p))]),
        Fit::Free { replaces: Some(r) } if Some(r) != e.prerequisite => tf("Click: install it in place of {name}  ·  Shift: queued", &[("name", &name(ty, r))]),
        Fit::Free { .. } => t("Click: install it (the unit stands meanwhile)  ·  Shift: queued").to_string(),
        Fit::No => String::new(),
    });
    lines.retain(|l| !l.is_empty());
    lines.join("\n")
}

/// The enhancements of `u` (of type `ty`) as a list, a row each: its name, slot and price,
/// marked where installed, its progress while being installed, dimmed where it may not be
/// installed now (the classic interface's page of them). A click installs one (Shift: queued;
/// one whose prerequisite is missing, after it), a right-click takes an installed one out.
pub(crate) fn list(ui: &mut egui::Ui, game: &Game, u: &Unit, ty: &UnitType) -> Option<EnhanceClick> {
    let mut click = None;
    let progress = game.sim.self_work_progress(u);
    for (k, e) in ty.enhancements.iter().enumerate() {
        let index = k as u16;
        let fit = game.sim.enhancement_fit(u, index);
        let line = tf(
            "{name}  ·  {slot}  ·  metal {metal}, energy {energy}",
            &[("name", &name(ty, index)), ("slot", &slot_name(ty, index)), ("metal", &whole(e.cost_metal)), ("energy", &whole(e.cost_energy))],
        );
        let line = match fit {
            Fit::Installed => tf("{line}  ·  installed", &[("line", &line)]),
            Fit::Installing => {
                let done = progress.filter(|(j, _)| *j == ta_sim::SelfJob::Enhance(index)).map_or(0.0, |(_, d)| d.to_f32().clamp(0.0, 1.0));
                tf("{line}  ·  {pct}%", &[("line", &line), ("pct", &format!("{:.0}", (done * 100.0).floor()))])
            }
            _ => line,
        };
        let enabled = matches!(fit, Fit::Free { .. } | Fit::Needs(_) | Fit::Installed);
        let on = matches!(fit, Fit::Installed | Fit::Installing);
        let resp = ui.add_enabled(enabled, egui::Button::selectable(on, line)).on_hover_text(tip(game, u, ty, index));
        if resp.clicked() && matches!(fit, Fit::Free { .. } | Fit::Needs(_)) {
            click = Some(EnhanceClick::Install { index, queue: ui.input(|i| i.modifiers.shift) });
        } else if resp.secondary_clicked() && fit == Fit::Installed {
            click = Some(EnhanceClick::Remove { index });
        }
    }
    click
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ta_sim::{Command, Fx, FxVec3, Order, Sim};

    use crate::game::enhancement_orders;

    /// Two Commanders whose lists differ (the second's first is another part), each a part a tick
    /// to install, and a battle on a flat map with two players.
    fn battle() -> Sim {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("ta-enhance-orders-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let com = |name: &str, parts: &str| format!("[UNITINFO]\n{{\nUnitName={name};\nObjectname={name};\nMaxDamage=1000;\nSightDistance=300;\nBuilder=1;\nWorkerTime=100;\nCanMove=1;\nMaxVelocity=1;\n[ENHANCEMENTS]\n{{\n{parts}\n}}\n}}\n");
        let files = [
            ("units/coma.fbi", com("COMA", "[SUITE]\n{\nSlot=LCH;\nBuildCostMetal=1;\nBuildTime=1;\nWorkerTime=100;\n}\n[ARMOUR]\n{\nSlot=Back;\nBuildCostMetal=1;\nBuildTime=1;\nMaxDamage=500;\n}")),
            ("units/comb.fbi", com("COMB", "[RADAR]\n{\nSlot=Back;\nBuildCostMetal=1;\nBuildTime=1;\nRadarDistance=500;\n}\n[SUITE]\n{\nSlot=LCH;\nBuildCostMetal=1;\nBuildTime=1;\nWorkerTime=100;\n}")),
        ];
        for (path, text) in &files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let data = Arc::new(ta_data::GameData::load(&vfs));
        std::fs::remove_dir_all(&dir).ok();
        let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let cells = 64u32;
        let n = (cells * cells) as usize;
        let map = ta_formats::tnt::TntMap { width: cells, height: cells, sea_level: 0, heights: vec![60; n], features: vec![ta_formats::tnt::FEATURE_NONE; n], tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default(), flags: Vec::new() };
        let players: Vec<ta_sim::PlayerSetup> = (0..2).map(|color| ta_sim::PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
        Sim::new(types, data, &map, &ta_data::MapInfo::default(), 0, &players, 7)
    }

    /// Supreme Commander's slot ids read as words; a mod's own slot names as it gives them.
    #[test]
    fn slots_are_named_in_words() {
        let sim = battle();
        let coma = sim.types.get(sim.types.id("COMA").unwrap());
        assert_eq!((super::slot_name(coma, 0), super::slot_name(coma, 1)), ("Left arm".to_string(), "Back".to_string()));
        let mut odd = coma.clone();
        odd.enhancement_slots = vec!["Turret".into(), "Hull".into()];
        assert_eq!(super::slot_name(&odd, 0), "Turret");
    }

    /// A choice from one type's list (the classic interface shows the first selected unit's) goes
    /// to the selected units of that type alone: another Commander's list is its own (its first part
    /// another), and an enemy's is no one's to give. Taking one out, likewise.
    #[test]
    fn an_enhancement_goes_to_the_units_of_the_type_whose_list_it_is() {
        let mut sim = battle();
        let (coma, comb) = (sim.types.id("COMA").unwrap(), sim.types.id("COMB").unwrap());
        let a = sim.spawn_unit(coma, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
        let b = sim.spawn_unit(comb, 0, FxVec3::from_ints(400, 60, 300), 0, true).unwrap();
        let theirs = sim.spawn_unit(coma, 1, FxVec3::from_ints(500, 60, 300), 0, true).unwrap();
        let selection = [a, b, theirs];
        let install = |index| Order::Enhance { index, remove: false };
        // COMA's first (its suite): COMA's alone, not COMB's first (its radar) nor the enemy's.
        assert_eq!(enhancement_orders(&sim, &selection, 0, coma, 0, false, false), vec![(a, vec![install(0)], false)]);
        // COMB's first (its radar), queued: COMB's alone.
        assert_eq!(enhancement_orders(&sim, &selection, 0, comb, 0, false, true), vec![(b, vec![install(0)], true)]);
        // Both installed (a tick each): taking out COMA's first takes out COMA's suite alone.
        sim.step(&[Command::Order { player: 0, units: vec![a, b], order: install(0), queue: false }]);
        for _ in 0..5 {
            sim.step(&[]);
        }
        assert_eq!((sim.unit(a).unwrap().enhancements.clone(), sim.unit(b).unwrap().enhancements.clone()), (vec![0], vec![0]));
        assert_eq!(enhancement_orders(&sim, &selection, 0, coma, 0, true, false), vec![(a, vec![Order::Enhance { index: 0, remove: true }], false)]);
        // Installed already: nothing more to give it.
        assert!(enhancement_orders(&sim, &selection, 0, coma, 0, false, false).is_empty());
    }
}
