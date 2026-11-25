//! Shields as the battle shows them (TA Reborn's: `ta_sim`'s `sim/shield.rs`; docs/MODDING.md,
//! "Shields"): each bubble a faint dome in its owner's colour round its unit, rippling out from its
//! latest hits, flashing as it collapses and as it comes back up; each personal shield a thin shell
//! round its unit, seen most as it is hit. Only the shields of units the viewer sees, up (or flashing
//! as they go). Their sounds, as they come up and go down, are the audio's (`audio.rs`,
//! `shield_changed`).

use std::collections::HashMap;

use glam::{Vec2, Vec3};
use ta_render::ShieldInstance;
use ta_render::shields::HITS;
use ta_sim::{Seen, SimEvent, UnitId};

use crate::game::Game;

/// Seconds a collapse's flash takes, and a coming up's.
const COLLAPSE_SECS: f32 = 0.8;
const UP_SECS: f32 = 0.6;
/// How long a hit's ripple runs (seconds; as the shader has it).
const RIPPLE_SECS: f32 = 0.9;
/// A bubble's opacity, and a personal shield's shell's.
const BUBBLE_OPACITY: f32 = 0.42;
const SHELL_OPACITY: f32 = 0.22;

/// What a shield's look keeps between frames: its latest hits (the way from its middle in its round
/// shape, and when), when it last collapsed and came up.
#[derive(Clone, Copy, Debug, Default)]
struct Look {
    hits: [Option<(Vec3, f32)>; HITS],
    next: usize,
    collapsed_at: Option<f32>,
    up_at: Option<f32>,
}

/// The battle's shields' looks, by unit.
#[derive(Default)]
pub struct ShieldFx {
    looks: HashMap<UnitId, Look>,
}

impl ShieldFx {
    /// Forgets every shield's look (the battle sought back or on).
    pub fn clear(&mut self) {
        self.looks.clear();
    }

    /// Takes in a battle's event: a shield's hit, its collapse, its coming up, its unit gone.
    pub fn on_event(&mut self, e: &SimEvent, sim: &ta_sim::Sim, now: f32) {
        match *e {
            SimEvent::ShieldHit { unit, pos, .. } => {
                let Some(u) = sim.unit(unit) else { return };
                let Some((make, _)) = sim.shield(u) else { return };
                let t = sim.unit_type(u);
                let at = Vec3::from_array(pos.to_f32());
                let middle = Vec3::from_array(u.pos.to_f32());
                let way = if make.bubble() {
                    // (The dome's round shape: its height stretched to its radius.)
                    let d = at - middle;
                    Vec3::new(d.x, d.y * make.radius.to_f32() / make.height.to_f32().max(1.0), d.z)
                } else {
                    at - (middle + Vec3::Y * t.model.height.to_f32() * 0.5)
                };
                let way = if way.length_squared() > 1e-6 { way.normalize() } else { Vec3::Y };
                let look = self.looks.entry(unit).or_default();
                look.hits[look.next] = Some((way, now));
                look.next = (look.next + 1) % HITS;
            }
            SimEvent::ShieldChanged { unit, up, collapsed } => {
                let look = self.looks.entry(unit).or_default();
                if up {
                    look.up_at = Some(now);
                } else if collapsed {
                    look.collapsed_at = Some(now);
                    look.hits = [None; HITS];
                }
            }
            SimEvent::UnitDestroyed { id, .. } => {
                self.looks.remove(&id);
            }
            _ => {}
        }
    }

    /// This frame's shields to draw: those of the units the viewer sees within `view` (with a
    /// margin), at their places `alpha` of the way to the next tick, `now` the effects' clock.
    pub fn instances(&self, game: &Game, alpha: f32, now: f32, view: (Vec2, Vec2), out: &mut Vec<ShieldInstance>) {
        let sim = &game.sim;
        if !sim.types.shields {
            return;
        }
        // (Softer flashes where the player asked for them.)
        let flashes = if game.effects.reduced_flashes { 0.35 } else { 1.0 };
        // Each player's colour, once (not once a unit), in linear light.
        let colours: Vec<[f32; 3]> = sim
            .players
            .iter()
            .map(|pl| egui::Rgba::from(crate::palette::team_color(pl.color)))
            .map(|c| [c.r(), c.g(), c.b()])
            .collect();
        for u in sim.units() {
            if u.dead || u.carried_by.is_some() || !u.is_complete() {
                continue;
            }
            let Some((make, state)) = sim.shield(u) else { continue };
            let look = self.looks.get(&u.id).copied().unwrap_or_default();
            let since = |at: Option<f32>| at.map(|t| now - t).filter(|s| *s >= 0.0);
            let collapsing = since(look.collapsed_at).filter(|&s| s < COLLAPSE_SECS);
            let coming_up = since(look.up_at).filter(|&s| s < UP_SECS);
            let up = state.is_some_and(|s| s.up);
            if !up && collapsing.is_none() {
                continue;
            }
            let t = sim.unit_type(u);
            let (radius, height) = if make.bubble() { (make.radius.to_f32(), make.height.to_f32()) } else { (t.radius.to_f32() * 1.25 + 4.0, t.model.height.to_f32() * 0.65 + 6.0) };
            let Some(at) = game.scene.unit_position(u.id, alpha) else { continue };
            let reach = radius.max(height) + 32.0;
            if at.x + reach < view.0.x || at.x - reach > view.1.x || at.z + reach < view.0.y || at.z - reach > view.1.y {
                continue;
            }
            if game.seen(u) != Seen::Visible {
                continue;
            }
            // (A personal shell round the unit's middle; a dome round its foot.)
            let centre = if make.bubble() { at } else { at + Vec3::Y * t.model.height.to_f32() * 0.5 };
            // A collapse bursts outward and fades; coming up, it grows into place, bright.
            let (flash, grow) = match (collapsing, coming_up) {
                (Some(s), _) if !up => (1.0 - s / COLLAPSE_SECS, 1.0 + 0.12 * s / COLLAPSE_SECS),
                (_, Some(s)) => ((1.0 - s / UP_SECS) * 0.8, 0.92 + 0.08 * (s / UP_SECS).sqrt()),
                _ => (0.0, 1.0),
            };
            let share = state.map_or(1.0, |s| (s.health.to_f32() / make.health.to_f32().max(1.0)).clamp(0.0, 1.0));
            let colour = colours.get(u.owner as usize).copied().unwrap_or([0.6, 0.8, 1.0]);
            let mut hits = [[0.0, 0.0, 0.0, -1.0]; HITS];
            for (slot, hit) in hits.iter_mut().zip(look.hits.iter().flatten()) {
                let age = now - hit.1;
                if (0.0..RIPPLE_SECS).contains(&age) {
                    *slot = [hit.0.x, hit.0.y, hit.0.z, age];
                }
            }
            let personal = !make.bubble();
            // (A personal shell barely there till it is hit.)
            let opacity = if personal { SHELL_OPACITY } else { BUBBLE_OPACITY };
            out.push(ShieldInstance {
                centre: [centre.x, centre.y, centre.z, radius * grow],
                shape: [height * grow, flash * flashes, share, if personal { 1.0 } else { 0.0 }],
                color: [colour[0], colour[1], colour[2], opacity],
                hits,
            });
        }
    }
}
