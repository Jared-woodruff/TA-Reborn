//! Authored starting formations. Legacy scenario sources still use the original scatter.

use anyhow::{Result, ensure};
use ta_sim::{Fx, FxVec3, Locomotion, Sim, UnitId, UnitType, world::World};

use super::Placement;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Formation {
    Line,
    Grid,
    Deposits,
}

/// How far inside the map's edge a footprint (and a factory's apron) keeps.
const EDGE: f32 = 48.0;

fn position((x, z): (f32, f32)) -> FxVec3 {
    FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z))
}

/// Axis-aligned footprints, including a factory's south-facing assembly/exit apron.
fn bounds(t: &UnitType, p: FxVec3) -> (f32, f32, f32, f32) {
    let (x, z) = (p.x.to_f32(), p.z.to_f32());
    let (w, h) = (t.footprint.0 as f32 * 8.0, t.footprint.1 as f32 * 8.0);
    (
        x - w,
        z - h,
        x + w,
        z + h + if t.is_factory { 144.0 } else { 0.0 },
    )
}

fn overlaps(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32), gap: f32) -> bool {
    a.0 < b.2 + gap && a.2 + gap > b.0 && a.1 < b.3 + gap && a.3 + gap > b.1
}

/// Nothing else within `gap` of the footprint (a factory's apron included). Aircraft start
/// parked on the ground, so they keep clear of everything as well.
fn clear(sim: &Sim, t: &UnitType, pos: FxVec3) -> bool {
    let rect = bounds(t, pos);
    sim.units().filter(|u| !u.dead).all(|u| {
        let other = sim.unit_type(u);
        let gap = if t.extracts_metal > Fx::ZERO && other.extracts_metal > Fx::ZERO {
            96.0
        } else {
            24.0
        };
        !overlaps(rect, bounds(other, u.pos), gap)
    })
}

/// A building keeps a cell clear of trees, rocks and wrecks all round (their crowns and
/// shapes reach past the cells they block); an extractor stands where the metal is, beside
/// whatever the map put there.
pub(super) fn clear_of_features(sim: &Sim, t: &UnitType, pos: FxVec3) -> bool {
    if t.locomotion != Locomotion::Static || t.extracts_metal > Fx::ZERO {
        return true;
    }
    let (x0, z0) = World::footprint_origin(World::pos_to_anchor(pos, t.footprint), t.footprint);
    (z0 - 1..=z0 + t.footprint.1 as i32).all(|z| {
        (x0 - 1..=x0 + t.footprint.0 as i32)
            .all(|x| !matches!(sim.world.occupant(x, z), ta_sim::world::Occupant::Feature(_)))
    })
}

/// A building other than an extractor leaves the metal deposits to the extractors (a metal
/// world's plain ground is all metal: anywhere will do there).
pub(super) fn off_the_ore(sim: &Sim, t: &UnitType, pos: FxVec3) -> bool {
    if t.locomotion != Locomotion::Static || t.extracts_metal > Fx::ZERO || sim.world.metal_world() {
        return true;
    }
    let origin = World::footprint_origin(World::pos_to_anchor(pos, t.footprint), t.footprint);
    !sim.world.ore_under(origin, t.footprint)
}

pub(super) fn factory_exit_clear(sim: &Sim, t: &UnitType, pos: FxVec3) -> bool {
    if !t.is_factory {
        return true;
    }
    // Check the largest ground product's route out as well as the building footprint.
    let product = t
        .build_options
        .iter()
        .copied()
        .filter(|&i| {
            !matches!(
                sim.types.get(i).locomotion,
                Locomotion::Air | Locomotion::Static
            )
        })
        .max_by_key(|&i| {
            let fp = sim.types.get(i).footprint;
            fp.0 * fp.1
        });
    let Some(product) = product else {
        return true;
    };
    let spec = sim.move_spec(product);
    [48, 80, 112, 144].into_iter().all(|distance| {
        let p = pos + FxVec3::from_ints(0, 0, t.footprint.1 as i64 * 8 + distance);
        sim.world
            .passable(&spec, World::pos_to_anchor(p, spec.footprint), None)
    })
}

pub(super) fn place(
    sim: &mut Sim,
    type_id: usize,
    owner: u8,
    p: &Placement,
    at: (f32, f32),
    face: (f32, f32),
    bound: (f32, f32),
) -> Result<Vec<UnitId>> {
    let types = sim.types.clone();
    let t = types.get(type_id);
    let formation = p.formation.unwrap();
    let static_unit = t.locomotion == Locomotion::Static;
    let spacing = p
        .spacing
        .unwrap_or(if static_unit { 128.0 } else { 64.0 })
        .max(t.footprint.0.max(t.footprint.1) as f32 * 16.0 + 32.0);
    let direction = (face.0 - at.0, face.1 - at.1);
    let length = direction.0.hypot(direction.1);
    let right = if length < 1.0 {
        (1.0, 0.0)
    } else {
        (direction.1 / length, -direction.0 / length)
    };
    let columns = (p.count as f32).sqrt().ceil().max(1.0) as u16;
    let rows = p.count.div_ceil(columns);
    // TA builds every building the same way round, square to the grid with its yard to the
    // south (as a constructor builds one in the game); units face the way they look.
    let heading = if static_unit {
        0
    } else {
        ta_sim::fixed::heading_of(position(face) - position(at))
    };
    let spec = sim.move_spec(type_id);
    let region = sim
        .world
        .component(&spec, World::pos_to_anchor(position(bound), spec.footprint));
    let mut placed: Vec<UnitId> = Vec::new();
    for k in 0..p.count {
        let ideal = match formation {
            Formation::Line => {
                let offset = (k as f32 - (p.count as f32 - 1.0) / 2.0) * spacing;
                (at.0 + right.0 * offset, at.1 + right.1 * offset)
            }
            Formation::Grid => (
                at.0 + (f32::from(k % columns) - f32::from(columns - 1) / 2.0) * spacing,
                at.1 + (f32::from(k / columns) - f32::from(rows.saturating_sub(1)) / 2.0) * spacing,
            ),
            Formation::Deposits => at,
        };
        let centre = World::pos_to_anchor(position(ideal), t.footprint);
        let radius = if formation == Formation::Deposits {
            64i32
        } else {
            24
        };
        let mut candidates: Vec<_> = (-radius..=radius)
            .flat_map(|z| (-radius..=radius).map(move |x| (x * x + z * z, x, z)))
            .collect();
        candidates.sort_unstable();
        let mut best = None;
        // Extractors: the productive sites, nearest first.
        let mut sites: Vec<(FxVec3, Fx)> = Vec::new();
        let (w, h) = sim.world.size_px();
        for (_, x, z) in candidates {
            let pos = sim
                .world
                .anchor_to_pos((centre.0 + x, centre.1 + z), t.footprint);
            // The whole footprint (a factory's apron too) well inside the map's edge.
            let (x0, z0, x1, z1) = bounds(t, pos);
            if x0 < EDGE || z0 < EDGE || x1 > w.to_f32() - EDGE || z1 > h.to_f32() - EDGE {
                continue;
            }
            let legal = if static_unit {
                sim.can_place(type_id, pos)
            } else if t.locomotion == Locomotion::Air {
                // Parked where an aircraft of its kind lands: level, dry ground nothing stands on.
                sim.world.air_landing_height(pos, &t.landing()).is_some()
            } else {
                sim.world
                    .passable(&spec, World::pos_to_anchor(pos, spec.footprint), None)
            };
            if !legal
                || !clear(sim, t, pos)
                || !clear_of_features(sim, t, pos)
                || !off_the_ore(sim, t, pos)
                || !factory_exit_clear(sim, t, pos)
            {
                continue;
            }
            // Terrain may shift a slot, but must not collapse its row into a cluster.
            if formation != Formation::Deposits
                && placed.iter().any(|&id| {
                    sim.unit(id)
                        .unwrap()
                        .pos
                        .within_xz(pos, Fx::from_f32(spacing * 0.75))
                })
            {
                continue;
            }
            if !static_unit
                && t.locomotion != Locomotion::Air
                && region != 0
                && sim
                    .world
                    .component(&spec, World::pos_to_anchor(pos, spec.footprint))
                    != region
            {
                continue;
            }
            if formation == Formation::Deposits {
                let yield_at = sim.extraction_at(type_id, pos);
                if yield_at > Fx::ZERO {
                    sites.push((pos, yield_at));
                }
            } else {
                best = Some(pos);
                break;
            }
        }
        if formation == Formation::Deposits {
            // The nearest deposit worth at least half the richest in reach (the base's own,
            // not a richer one out in the open), and on it the site that takes the most.
            let richest = sites.iter().map(|s| s.1).max().unwrap_or(Fx::ZERO);
            best = sites.iter().find(|s| s.1 * 2i64 >= richest).map(|&(near, _)| {
                sites
                    .iter()
                    .filter(|s| s.0.within_xz(near, Fx::from_int(48)))
                    .fold((near, Fx::ZERO), |a, s| if s.1 > a.1 { *s } else { a })
                    .0
            });
        }
        let Some(pos) = best else {
            anyhow::bail!("no usable site for unit {} of {}", k + 1, p.count);
        };
        let id = sim.spawn_unit(type_id, owner, pos, heading, true);
        let Some(id) = id else {
            anyhow::bail!("could not spawn unit {} of {}", k + 1, p.count);
        };
        ensure!(
            clear_except(sim, type_id, id),
            "unit {} of {} overlaps another unit or factory exit",
            k + 1,
            p.count
        );
        placed.push(id);
    }
    Ok(placed)
}

pub(super) fn clear_except(sim: &Sim, type_id: usize, id: UnitId) -> bool {
    let unit = sim.unit(id).unwrap();
    let t = sim.types.get(type_id);
    sim.units()
        .filter(|u| !u.dead && u.id != id)
        .all(|u| !overlaps(bounds(t, unit.pos), bounds(sim.unit_type(u), u.pos), 0.0))
}
