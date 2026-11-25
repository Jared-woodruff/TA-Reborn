//! Shields in the interface (TA Reborn's: `ta_sim`'s `sim/shield.rs`; docs/MODDING.md, "Shields"):
//! the selection's shields together for the unit card's gauge under its health (how many hit points
//! they have of their most, and for one unit why its shield is down), and a shield's figures in
//! words for the information page and the enhancements' lists.

use egui::Color32;
use ta_sim::defs::Shield;
use ta_sim::{Fx, Sim, Unit};
use ta_ui::i18n::{t, tf};

use crate::game::Game;
use crate::theme::pal;

/// A shield's gauge colour: cool blue, as the domes are drawn bright.
pub(crate) const COLOUR: Color32 = Color32::from_rgb(90, 170, 255);

/// The selection's shields together: their hit points and their most, and for one unit with its
/// shield down why (and in its colour).
pub(crate) struct Reading {
    pub health: f32,
    pub most: f32,
    pub status: Option<(String, Color32)>,
}

/// What a unit's shield stands at now: its hit points (none while it is down after a collapse) and
/// its most, and why it is down, if it is.
fn of(sim: &Sim, u: &Unit) -> Option<Reading> {
    let (make, state) = sim.shield(u)?;
    let most = make.health.to_f32();
    let Some(s) = state else { return Some(Reading { health: most, most, status: None }) };
    let status = (!s.up).then(|| {
        let secs = (s.charge as f32 / 30.0).ceil().max(1.0);
        if s.collapsed {
            (tf("Shield collapsed: back in {secs} s", &[("secs", &format!("{secs:.0}"))]), pal().bad)
        } else if !sim.shield_switched_on(u) {
            (t("Shield off").to_string(), pal().steel)
        } else if !u.upkeep_paid {
            (t("Shield down: short of energy").to_string(), pal().accent_bright)
        } else {
            (tf("Shield coming up in {secs} s", &[("secs", &format!("{secs:.0}"))]), pal().info)
        }
    });
    Some(Reading { health: s.health.to_f32(), most, status })
}

/// The selection's shields together, if any of it has one (finished units, ours or allies' or the
/// enemy's as the card shows them).
pub(crate) fn reading(game: &Game) -> Option<Reading> {
    let mut found = Vec::new();
    for u in game.selection.iter().filter_map(|&id| game.sim.unit(id)).filter(|u| u.is_complete()) {
        if let Some(r) = of(&game.sim, u) {
            found.push(r);
        }
    }
    if found.is_empty() {
        return None;
    }
    let health = found.iter().map(|r| r.health).sum();
    let most = found.iter().map(|r| r.most).sum();
    let status = if found.len() == 1 { found.pop().and_then(|r| r.status) } else { None };
    Some(Reading { health, most, status })
}

fn whole(v: Fx) -> String {
    format!("{:.0}", v.to_f32())
}

/// A shield's figures in words: what kind, its hit points and what it regains, its recharge and
/// its upkeep.
pub(crate) fn lines(shield: &Shield) -> Vec<String> {
    let mut out = Vec::new();
    let regen = whole(shield.regen * 30);
    if shield.bubble() {
        out.push(tf("Shield bubble {across} across, {high} high: stops enemy shots from outside", &[("across", &whole(shield.radius * 2)), ("high", &whole(shield.height))]));
    } else {
        out.push(t("Personal shield: takes the damage first").to_string());
    }
    out.push(tf("Shield: {health} hit points, regains {regen} a second", &[("health", &whole(shield.health)), ("regen", &regen)]));
    out.push(tf("Shield back {secs} s after collapsing", &[("secs", &format!("{:.0}", shield.recharge as f32 / 30.0))]));
    if shield.energy > Fx::ZERO {
        out.push(tf("Shield uses {rate} energy a second", &[("rate", &whole(shield.energy * 30))]));
    }
    out
}
