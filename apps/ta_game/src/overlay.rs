//! Map overlays: what the ground shows over its art when the player asks (F2 where the
//! selected units can go, F3 heights, F4 metal, L sight and radar). Metal, movement and
//! sight are colours per heightmap cell built here and uploaded to the terrain
//! (`TerrainGpu::set_overlay`); heights are drawn by the terrain shader itself.

use ta_render::MapOverlay;
use ta_ui::i18n::n;
use ta_sim::{Fx, Locomotion, Sim, vision};

/// The overlays a player can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// Where the selected units (or the Commander, or a tank) can drive.
    Path,
    Height,
    Metal,
    /// Line of sight, radar and sonar coverage, and explored ground.
    Sight,
}

impl Overlay {
    pub fn label(self) -> &'static str {
        match self {
            Overlay::Path => n("MOVEMENT"),
            Overlay::Height => n("HEIGHT"),
            Overlay::Metal => n("METAL"),
            Overlay::Sight => n("SIGHT"),
        }
    }

    /// A line explaining the colours.
    pub fn legend(self) -> &'static str {
        match self {
            Overlay::Path if crate::palette::colour_blind() => n("Blue: the selection can go there · orange: it cannot"),
            Overlay::Path => n("Green: the selection can go there · red: it cannot"),
            Overlay::Height => n("Blue low ground to white peaks · a line every 16 units"),
            Overlay::Metal => n("Brighter is richer · dark: no metal"),
            Overlay::Sight => n("Clear: in sight · blue: radar · teal: sonar · dark: out of view"),
        }
    }

    pub fn action(self) -> &'static str {
        match self {
            Overlay::Path => "overlay_path",
            Overlay::Height => "overlay_height",
            Overlay::Metal => "overlay_metal",
            Overlay::Sight => "overlay_sight",
        }
    }

    pub const ALL: [Overlay; 4] = [Overlay::Path, Overlay::Height, Overlay::Metal, Overlay::Sight];

    /// What the renderer draws for it.
    pub fn render_mode(self) -> MapOverlay {
        match self {
            Overlay::Height => MapOverlay::Height,
            _ => MapOverlay::Cells,
        }
    }
}

/// Metal per heightmap cell: dark where there is none (or only the map's everyday trace),
/// then amber to white by richness above that, up to the richest cell on the map. On a metal
/// world, where all plain ground is metal, that ground only takes a faint warm tint and its
/// deposits stand out above it, so the battlefield stays in view.
pub fn metal(sim: &Sim) -> Vec<u8> {
    let w = &sim.world;
    let max = w.metal.iter().copied().max().unwrap_or(0).max(1);
    if w.metal_world() {
        let plain = w.surface_metal;
        return w.metal.iter().flat_map(|&m| metal_world_cell(m, plain, max)).collect();
    }
    // Many maps put a little metal under every cell; the deposits are what stands out.
    let mut sorted: Vec<u32> = w.metal.clone();
    sorted.sort_unstable();
    let base = sorted.get(sorted.len() * 7 / 10).copied().unwrap_or(0).min(max.saturating_sub(1));
    w.metal
        .iter()
        .flat_map(|&m| {
            if m <= base {
                [0, 0, 0, 115]
            } else {
                let t = ((m - base) as f32 / (max - base).max(1) as f32).sqrt();
                let c = |a: f32, b: f32, c: f32| {
                    if t < 0.5 {
                        a + (b - a) * t * 2.0
                    } else {
                        b + (c - b) * (t - 0.5) * 2.0
                    }
                };
                [c(120.0, 245.0, 255.0) as u8, c(60.0, 170.0, 250.0) as u8, c(10.0, 40.0, 200.0) as u8, (150.0 + 90.0 * t) as u8]
            }
        })
        .collect()
}

/// A metal world's cell: none (dark), plain metal ground (a faint warm tint) or a deposit
/// richer than that (amber to white, as on other maps, but lighter).
fn metal_world_cell(m: u32, plain: u32, max: u32) -> [u8; 4] {
    if m == 0 {
        [0, 0, 0, 115]
    } else if m <= plain {
        [200, 140, 60, 36]
    } else {
        let t = ((m - plain) as f32 / max.saturating_sub(plain).max(1) as f32).sqrt();
        [(200.0 + 55.0 * t) as u8, (150.0 + 100.0 * t) as u8, (60.0 + 140.0 * t) as u8, (70.0 + 110.0 * t) as u8]
    }
}

/// Where units of `type_id` can stand, per heightmap cell: faint green where they can, red
/// where they cannot (buildings, cliffs, water or land as the case may be); blue and orange
/// in the colour-blind colours.
pub fn path(sim: &Sim, type_id: usize) -> Vec<u8> {
    let spec = sim.move_spec(type_id);
    let ok = sim.world.passable_all(&spec);
    let (yes, no) = (crate::palette::allowed_rgba(), crate::palette::blocked_rgba());
    ok.iter().flat_map(|&p| if p { yes } else { no }).collect()
}

/// Sight and sensor coverage of `player` per heightmap cell (vision cells are 32 px, two
/// heightmap cells).
pub fn sight(sim: &Sim, player: u8) -> Vec<u8> {
    let v = &sim.vision;
    let cells = v.player_cells(sim.vision_slot(player));
    let (w, h) = (sim.world.width as usize, sim.world.height as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    for z in 0..h {
        for x in 0..w {
            let (vx, vz) = ((x * 16 / vision::VISION_CELL as usize).min(v.width - 1), (z * 16 / vision::VISION_CELL as usize).min(v.height - 1));
            let f = cells.get(vz * v.width + vx).copied().unwrap_or(0);
            let c = if f & vision::LOS != 0 {
                [0, 0, 0, 0]
            } else if f & vision::RADAR != 0 {
                [60, 130, 255, 110]
            } else if f & vision::SONAR != 0 {
                [40, 200, 190, 110]
            } else if f & vision::EXPLORED != 0 {
                [0, 0, 10, 140]
            } else {
                [0, 0, 10, 200]
            };
            out.extend_from_slice(&c);
        }
    }
    out
}

/// The type whose movement the path overlay shows: the first selected mobile unit's, else
/// the player's Commander's, else the first ground unit type.
pub fn path_type(sim: &Sim, selection: &[ta_sim::UnitId], player: u8) -> Option<usize> {
    let mobile = |t: &ta_sim::UnitType| matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval) && t.max_speed > Fx::ZERO;
    selection
        .iter()
        .filter_map(|&id| sim.unit(id))
        .map(|u| sim.unit_type(u))
        .find(|t| mobile(t))
        .map(|t| t.id)
        .or_else(|| sim.units().find(|u| u.owner == player && sim.unit_type(u).def.is_commander).map(|u| u.type_id))
        .or_else(|| sim.types.types.iter().find(|t| t.locomotion == Locomotion::Ground && t.max_speed > Fx::ZERO).map(|t| t.id))
}

#[cfg(test)]
mod tests {
    use super::metal_world_cell;

    #[test]
    fn a_metal_worlds_plain_ground_only_tints_and_its_deposits_stand_out() {
        let (none, plain, rich) = (metal_world_cell(0, 255, 400), metal_world_cell(255, 255, 400), metal_world_cell(400, 255, 400));
        assert_eq!(none[3], 115, "no metal: dark");
        assert!(plain[3] <= 40, "plain metal ground keeps the battlefield in view: {plain:?}");
        assert!(rich[3] > plain[3] && rich[2] > plain[2], "a deposit stands out: {rich:?}");
        // Where every cell is plain metal (the richest is plain too), all of it is the tint.
        assert_eq!(metal_world_cell(255, 255, 255), plain);
    }
}
