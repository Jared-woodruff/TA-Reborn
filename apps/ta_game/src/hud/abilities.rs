//! TA Reborn's abilities in words (docs/MODDING.md, "Abilities"), for the information page (F1) and
//! the enhancements' lists: a type's or an enhancement's jamming, stealth fields, remote viewing and
//! regeneration aura, what an enhancement makes and takes, a weapon's minimum range and stun, a
//! weapon toggle.

use ta_sim::defs::{Abilities, Enhancement};
use ta_sim::{Fx, UnitType, WeaponType};
use ta_ui::i18n::tf;

fn whole(v: Fx) -> String {
    format!("{:.0}", v.to_f32())
}

/// The lines of `abilities` that TA's own keys do not tell (`with_jamming`: an enhancement's
/// jamming too; a type's is told from its file's keys).
pub(crate) fn lines(abilities: &Abilities, with_jamming: bool) -> Vec<String> {
    let mut out = Vec::new();
    if with_jamming && abilities.radar_jam > 0 {
        out.push(tf("Jams radar {range}", &[("range", &abilities.radar_jam)]));
    }
    if with_jamming && abilities.sonar_jam > 0 {
        out.push(tf("Jams sonar {range}", &[("range", &abilities.sonar_jam)]));
    }
    if abilities.radar_field > 0 {
        out.push(tf("Stealth field {range}: its side's units round it hidden from radar", &[("range", &abilities.radar_field)]));
    }
    if abilities.sonar_field > 0 {
        out.push(tf("Sonar stealth field {range}: its side's units round it hidden from sonar", &[("range", &abilities.sonar_field)]));
    }
    if let Some(view) = abilities.remote_view {
        out.push(tf("Views from afar: {range} round a place, for {energy} energy a second", &[("range", &whole(view.radius)), ("energy", &whole(view.energy * 30))]));
    }
    if let Some(aura) = abilities.aura {
        let rate = if aura.share > Fx::ZERO {
            let share = format!("{:.1}", aura.share.to_f32() * 100.0);
            if aura.rate > Fx::ZERO { tf("{share}% of their health a second, {rate} at most", &[("share", &share), ("rate", &whole(aura.rate))]) } else { tf("{share}% of their health a second", &[("share", &share)]) }
        } else {
            tf("{rate} hit points a second", &[("rate", &whole(aura.rate))])
        };
        out.push(tf("Mends its side's units within {range}: {rate}", &[("range", &whole(aura.radius)), ("rate", &rate)]));
    }
    out
}

/// A type's weapon toggle, in words.
pub(crate) fn toggle_line(ty: &UnitType) -> Option<String> {
    let toggle = ty.def.toggle.as_ref().filter(|_| ty.abilities.toggle.is_some())?;
    Some(tf("Weapon toggle: {name}", &[("name", &toggle.name)]))
}

/// What an enhancement makes and takes a second.
pub(crate) fn production_lines(e: &Enhancement) -> Vec<String> {
    let mut out = Vec::new();
    if e.metal_make > Fx::ZERO {
        out.push(tf("Makes {rate} metal a second", &[("rate", &format!("{:.1}", (e.metal_make * 30).to_f32()))]));
    }
    if e.energy_make > Fx::ZERO {
        out.push(tf("Makes {rate} energy a second", &[("rate", &whole(e.energy_make * 30))]));
    }
    if e.upkeep > Fx::ZERO {
        out.push(tf("Uses {rate} energy a second", &[("rate", &whole(e.upkeep * 30))]));
    }
    out
}

/// What is special about a weapon of TA Reborn's: its minimum range, its stun.
pub(crate) fn weapon_lines(w: &WeaponType) -> Vec<String> {
    let mut out = Vec::new();
    if w.min_range > Fx::ZERO {
        out.push(tf("Fires no nearer than {range}", &[("range", &whole(w.min_range))]));
    }
    if let Some(stun) = w.stun {
        out.push(tf("Stuns for {secs} s", &[("secs", &format!("{:.1}", stun.ticks as f32 / 30.0))]));
    }
    out
}
