//! TA Reborn's logistics in words (docs/MODDING.md: air transports with slots, teleporting, drones,
//! factories that move), for the information page (F1) and the enhancements' lists.

use ta_sim::defs::Enhancement;
use ta_sim::sim::drones::DroneSpec;
use ta_sim::sim::teleport::Teleport;
use ta_sim::{Fx, Sim, Unit, UnitType};
use ta_ui::i18n::{t, tf};

fn whole(v: Fx) -> String {
    format!("{:.0}", v.to_f32())
}

/// A teleport's charge, in words: its energy and seconds (and so much more a pixel of the way).
fn teleport_line(teleport: &Teleport) -> String {
    let secs = format!("{:.1}", teleport.ticks.to_f32() / 30.0);
    if teleport.energy_per_px > Fx::ZERO || teleport.ticks_per_px > Fx::ZERO {
        tf("Teleports anywhere: {energy} energy and {secs} s, more the farther it goes", &[("energy", &whole(teleport.energy)), ("secs", &secs)])
    } else {
        tf("Teleports anywhere: {energy} energy and {secs} s", &[("energy", &whole(teleport.energy)), ("secs", &secs)])
    }
}

/// Drones kept, in words: how many of what, and how long one lost takes to make again.
fn drone_line(sim: &Sim, d: &DroneSpec) -> String {
    let name = crate::chat::unit_label(sim, d.unit_type);
    let secs = format!("{:.0}", d.rebuild_work as f64 / 65536.0 / 30.0);
    tf("Keeps {n} drones ({name}): one lost made again in {secs} s", &[("n", &d.count), ("name", &name), ("secs", &secs)])
}

/// The lines of a type's (or a unit's, with its enhancements) logistics: an air transport's slots,
/// its teleport, its drones, a factory that moves.
pub(crate) fn lines(sim: &Sim, ty: &UnitType, unit: Option<&Unit>) -> Vec<String> {
    let mut out = Vec::new();
    if ty.has_slots() {
        let count = |from: u32, to: u32| ty.transport_slots.iter().filter(|s| s.class >= from && s.class <= to).count();
        out.push(tf(
            "Its passengers hang under it, in slots: {small} small, {medium} medium, {large} large",
            &[("small", &count(1, 1)), ("medium", &count(2, 2)), ("large", &count(3, u32::MAX))],
        ));
    }
    let teleport = match unit {
        Some(u) => sim.teleport_of(u),
        None => ty.teleport,
    };
    if let Some(teleport) = teleport {
        out.push(teleport_line(&teleport));
    }
    let drones = match unit {
        Some(u) => sim.wanted_drones(u),
        None => ty.drones.clone(),
    };
    out.extend(drones.iter().map(|d| drone_line(sim, d)));
    if ty.mobile_factory {
        out.push(t("A factory that moves: builds its units on itself as it goes").to_string());
    }
    out
}

/// What an enhancement gives of the logistics: a teleport, drones.
pub(crate) fn enhancement_lines(sim: &Sim, e: &Enhancement) -> Vec<String> {
    e.teleport.as_ref().map(teleport_line).into_iter().chain(e.drones.iter().map(|d| drone_line(sim, d))).collect()
}
