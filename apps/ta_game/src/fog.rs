//! The viewer's fog of war as it is drawn, and what the viewer remembers of the enemy's buildings.
//!
//! Presentation only: both read the simulation's vision and never feed back into it (what
//! `Sim::seen_by` says is seen, what targeting and the AI may use, is untouched).
//!
//! * [`FogView`] keeps what the renderer shows of the viewer's sight, a texel a vision cell: in
//!   sight, explored, in radar or sonar range, each 0..1, eased from one state to the next (a
//!   place coming into sight lights up in a third of a second, one going out of it dims over a
//!   second and a half) instead of popping with every rebuild of the vision, and spread over the
//!   cells round it so the edges of what is seen are soft slopes that slide as units move. The
//!   shaders shape them further (`fog_state` in common.wgsl).
//! * [`Memory`] remembers the enemy structures the viewer has seen: drawn as ghosts where they
//!   were last seen, as they stood then, until the place is in sight again without them (what a
//!   player would remember; nothing a player has not seen is ever shown).

use std::collections::BTreeMap;

use glam::{Mat4, Vec3};
use ta_sim::vision::{EXPLORED, LOS, RADAR, SONAR};
use ta_sim::{Fx, FxVec3, Locomotion, Seen, Sim, UnitId};

/// Seconds a place takes to go from out of sight to in sight, and from in sight to out of it
/// (the light lingers).
const SIGHT_IN: f32 = 0.35;
const SIGHT_OUT: f32 = 1.5;
/// Seconds a place takes to show as explored once it is seen.
const EXPLORED_IN: f32 = 0.6;
/// Seconds a place takes to come into radar range, and to leave it.
const RADAR_IN: f32 = 0.5;
const RADAR_OUT: f32 = 1.1;
/// Grids of more cells than this are changed at once and left sharp: easing and spreading them
/// would cost more than it shows.
const EASED_CELLS: usize = 1 << 18;

/// One cell's state: in sight, explored, in radar or sonar range.
type Cell = [f32; 3];

/// The viewer's sight, as drawn.
#[derive(Default)]
pub struct FogView {
    width: usize,
    height: usize,
    /// What is drawn, and what it is heading for.
    shown: Vec<Cell>,
    target: Vec<Cell>,
    texels: Vec<u8>,
    /// The viewer and the vision version the target is of.
    key: Option<(u8, u64)>,
    /// What is drawn has got where it is heading.
    settled: bool,
}

impl FogView {
    /// Brings what is drawn `dt` seconds on towards `viewer`'s sight as `sim` has it now; true
    /// when the texels changed (to be uploaded: [`FogView::texels`]).
    pub fn update(&mut self, sim: &Sim, viewer: u8, dt: f32) -> bool {
        let vision = &sim.vision;
        let (width, height) = (vision.width, vision.height);
        let key = (viewer, vision.version);
        let mut changed = false;
        if self.key != Some(key) {
            let cells = vision.player_cells(sim.vision_slot(viewer));
            // (A new viewer, another map's grid or a clock gone back, as a replay's seek does it:
            // drawn as it is at once.)
            let fresh = (self.width, self.height) != (width, height) || self.key.is_none_or(|k| k.0 != viewer || key.1 < k.1);
            (self.width, self.height) = (width, height);
            self.target = targets(cells, width, height);
            if fresh || width * height > EASED_CELLS {
                self.shown.clone_from(&self.target);
                changed = true;
            }
            self.key = Some(key);
            self.settled = false;
        }
        if !self.settled {
            self.settled = ease(&mut self.shown, &self.target, dt);
            changed = true;
        }
        if changed {
            texels(&self.shown, &mut self.texels);
        }
        changed
    }

    /// The grid's size in cells and its texels: four bytes a cell, row by row (sight, explored,
    /// radar, 255).
    pub fn texels(&self) -> (usize, usize, &[u8]) {
        (self.width, self.height, &self.texels)
    }
}

/// What a vision grid's cells call for: in sight, explored, in radar or sonar range; each spread
/// over the cells round it (a 3 x 3 filter, 1 2 1 either way), so that the edges of what is seen
/// are slopes a cell or two wide instead of steps.
fn targets(cells: &[u8], width: usize, height: usize) -> Vec<Cell> {
    let flags = |c: u8| -> Cell { [(c & LOS != 0) as u8 as f32, (c & (LOS | EXPLORED) != 0) as u8 as f32, (c & (RADAR | SONAR) != 0) as u8 as f32] };
    let mut out: Vec<Cell> = (0..width * height).map(|i| flags(cells.get(i).copied().unwrap_or(0))).collect();
    if width * height > EASED_CELLS || width < 3 || height < 3 {
        return out;
    }
    // (Along the rows, then down the columns; the edges of the grid repeat their cells.)
    let mut rows = out.clone();
    for y in 0..height {
        for x in 0..width {
            let (l, c, r) = (out[y * width + x.saturating_sub(1)], out[y * width + x], out[y * width + (x + 1).min(width - 1)]);
            rows[y * width + x] = std::array::from_fn(|k| (l[k] + 2.0 * c[k] + r[k]) * 0.25);
        }
    }
    for y in 0..height {
        for x in 0..width {
            let (u, c, d) = (rows[y.saturating_sub(1) * width + x], rows[y * width + x], rows[(y + 1).min(height - 1) * width + x]);
            out[y * width + x] = std::array::from_fn(|k| (u[k] + 2.0 * c[k] + d[k]) * 0.25);
        }
    }
    out
}

/// Moves what is `shown` `dt` seconds towards the `target`, each channel at its own pace (a place
/// comes into sight quicker than it goes out of it, and what has been explored stays so). True
/// when it has got there.
fn ease(shown: &mut [Cell], target: &[Cell], dt: f32) -> bool {
    let steps = [(SIGHT_IN, SIGHT_OUT), (EXPLORED_IN, f32::INFINITY), (RADAR_IN, RADAR_OUT)];
    let mut arrived = true;
    for (s, t) in shown.iter_mut().zip(target) {
        for k in 0..3 {
            let (up, down) = steps[k];
            if t[k] > s[k] {
                s[k] = (s[k] + dt / up).min(t[k]);
            } else if t[k] < s[k] {
                s[k] = (s[k] - dt / down).max(t[k]);
            }
            arrived &= s[k] == t[k];
        }
    }
    arrived
}

/// The texels of a grid: four bytes a cell.
fn texels(cells: &[Cell], out: &mut Vec<u8>) {
    out.clear();
    out.extend(cells.iter().flat_map(|c| [c[0], c[1], c[2], 1.0].map(|v| (v * 255.0 + 0.5) as u8)));
}

/// An enemy structure as the viewer last saw it.
pub struct Remembered {
    pub type_id: usize,
    /// Where it stood (as drawn), its radius, how far its construction had got and the colour of
    /// its player.
    pub pos: Vec3,
    pub radius: f32,
    pub build: f32,
    pub color: u32,
    /// The world matrices of its pieces as it stood (its doors as the script had them).
    pub matrices: Vec<Mat4>,
    /// The tick it was last seen.
    seen: u64,
}

/// Ticks between two looks at a structure that stays in sight (its doors and turrets move; its
/// remembered pose follows them at a third of a second's pace).
const REFRESH: u64 = 10;

/// The enemy structures the viewer has seen, by where they were last seen.
#[derive(Default)]
pub struct Memory {
    seen: BTreeMap<(u32, u32), Remembered>,
    viewer: Option<u8>,
    tick: u64,
    /// The viewer and tick the memory was last brought up to date at (once each is plenty).
    done: Option<(Option<u8>, u64)>,
    /// Bumped whenever the memory changes.
    pub version: u64,
}

/// Whether a structure remembered is no longer there to the viewer: its place is in sight and it
/// is not seen in it. (A structure that is gone out of sight stays remembered as it was: that is
/// what the viewer knows.)
pub fn forgotten(place_in_sight: bool, seen_now: bool) -> bool {
    place_in_sight && !seen_now
}

impl Memory {
    /// What the viewer knows of the enemy's structures as of now: those seen are remembered as
    /// they stand; those whose place is in sight and which are not seen there are forgotten.
    pub fn update(&mut self, sim: &Sim, viewer: Option<u8>) {
        let viewer = viewer.filter(|_| sim.los_enabled);
        if self.done == Some((viewer, sim.tick)) {
            return;
        }
        self.done = Some((viewer, sim.tick));
        if viewer != self.viewer || sim.tick < self.tick {
            if !self.seen.is_empty() {
                self.version += 1;
            }
            self.seen.clear();
            self.viewer = viewer;
        }
        self.tick = sim.tick;
        let Some(player) = viewer else { return };
        for u in sim.units() {
            if u.dead || u.carried_by.is_some() || !sim.is_enemy(player, u.owner) {
                continue;
            }
            let t = sim.unit_type(u);
            if t.locomotion != Locomotion::Static || sim.seen_by(u, player) != Seen::Visible {
                continue;
            }
            let key = (u.id.slot, u.id.generation);
            if self.seen.get(&key).is_some_and(|r| sim.tick < r.seen + REFRESH) {
                continue;
            }
            let record = self.seen.entry(key).or_insert_with(|| Remembered {
                type_id: u.type_id,
                pos: Vec3::ZERO,
                radius: t.radius.to_f32(),
                build: 0.0,
                color: 0,
                matrices: Vec::new(),
                seen: 0,
            });
            let mut pos = Vec3::from_array(u.pos.to_f32());
            pos.y += crate::scene::ground_offset(&sim.world, pos);
            (record.pos, record.build, record.seen) = (pos, u.build_progress.to_f32().clamp(0.0, 1.0), sim.tick);
            record.color = sim.players.get(u.owner as usize).map_or(0, |p| p.color as u32);
            crate::scene::standing_matrices(sim, u, &mut record.matrices);
            self.version += 1;
        }
        let before = self.seen.len();
        self.seen.retain(|&(slot, generation), r| {
            let unit = sim.unit(UnitId { slot, generation });
            // (One captured since, or given to the viewer's side, is no enemy's to be remembered.)
            if unit.is_some_and(|u| !sim.is_enemy(player, u.owner)) {
                return false;
            }
            let seen_now = unit.is_some_and(|u| sim.seen_by(u, player) == Seen::Visible);
            let place = FxVec3::new(Fx::from_f32(r.pos.x), Fx::from_f32(r.pos.y), Fx::from_f32(r.pos.z));
            !forgotten(sim.in_sight(player, place), seen_now)
        });
        if self.seen.len() != before {
            self.version += 1;
        }
    }

    /// The structures remembered, to be drawn where they stood.
    pub fn ghosts(&self) -> impl Iterator<Item = &Remembered> {
        self.seen.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid `w` x `h` of cells with the given flags set at the given places.
    fn grid(w: usize, h: usize, set: &[(usize, usize, u8)]) -> Vec<u8> {
        let mut cells = vec![0u8; w * h];
        for &(x, y, f) in set {
            cells[y * w + x] |= f;
        }
        cells
    }

    #[test]
    fn what_is_seen_is_explored_and_a_radar_cell_is_in_range() {
        let t = targets(&grid(2, 2, &[(0, 0, LOS), (1, 0, EXPLORED), (0, 1, RADAR), (1, 1, SONAR | EXPLORED)]), 2, 2);
        assert_eq!(t[0], [1.0, 1.0, 0.0], "in sight is explored too");
        assert_eq!(t[1], [0.0, 1.0, 0.0]);
        assert_eq!(t[2], [0.0, 0.0, 1.0], "radar range does not explore");
        assert_eq!(t[3], [0.0, 1.0, 1.0], "sonar counts as range");
    }

    #[test]
    fn an_edge_is_spread_over_the_cells_round_it_and_nothing_is_lost() {
        // One cell in sight in the middle of 5 x 5: spread over its eight neighbours, the weights
        // 1 2 1 either way, adding up to what it was.
        let t = targets(&grid(5, 5, &[(2, 2, LOS)]), 5, 5);
        assert_eq!(t[2 * 5 + 2][0], 0.25);
        assert_eq!(t[2 * 5 + 3][0], 0.125);
        assert_eq!(t[3 * 5 + 3][0], 0.0625);
        assert_eq!(t[0][0], 0.0);
        let total: f32 = t.iter().map(|c| c[0]).sum();
        assert!((total - 1.0).abs() < 1e-6, "{total}");
        // A place wholly in sight stays so.
        let full = targets(&vec![LOS; 25], 5, 5);
        assert!(full.iter().all(|c| *c == [1.0, 1.0, 0.0]));
    }

    #[test]
    fn a_place_lights_up_quick_and_dims_slow_and_stays_explored() {
        let mut shown = vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]];
        let target = vec![[1.0, 1.0, 1.0], [0.0, 1.0, 0.0]];
        // 0.1 s: the first a third of the way in, the second a fifteenth of the way out.
        assert!(!ease(&mut shown, &target, 0.1));
        assert!((shown[0][0] - 0.1 / SIGHT_IN).abs() < 1e-5 && (shown[1][0] - (1.0 - 0.1 / SIGHT_OUT)).abs() < 1e-5, "{shown:?}");
        assert!(shown[0][0] > 1.0 - shown[1][0], "in is quicker than out");
        assert_eq!(shown[1][1], 1.0, "explored is not lost");
        // Left to run, both arrive, exactly.
        assert!(ease(&mut shown, &target, 5.0));
        assert_eq!(shown, target);
        // And a pause (no time passes) moves nothing.
        let mut still = vec![[0.5, 0.5, 0.5]];
        assert!(!ease(&mut still, &[[1.0, 1.0, 1.0]], 0.0));
        assert_eq!(still[0], [0.5, 0.5, 0.5]);
    }

    #[test]
    fn texels_are_four_bytes_a_cell_rounded() {
        let mut out = Vec::new();
        texels(&[[0.0, 0.5, 1.0], [1.0, 0.0, 0.25]], &mut out);
        assert_eq!(out, [0, 128, 255, 255, 255, 0, 64, 255]);
    }

    /// Game data from TDF files (path, text), loaded as a mod's folder is.
    fn data_of(files: &[(&str, &str)]) -> ta_data::GameData {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("ta-fog-memory-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let data = ta_data::GameData::load(&vfs);
        std::fs::remove_dir_all(&dir).ok();
        data
    }

    #[test]
    fn an_enemy_structure_seen_is_remembered_until_its_place_is_seen_without_it() {
        let data = std::sync::Arc::new(data_of(&[
            ("units/eye.fbi", "[UNITINFO]{UnitName=EYE;MaxDamage=1000;BuildTime=100;FootprintX=1;FootprintZ=1;SightDistance=300;}"),
            ("units/tower.fbi", "[UNITINFO]{UnitName=TOWER;MaxDamage=1000;BuildTime=100;FootprintX=2;FootprintZ=2;SightDistance=0;}"),
        ]));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let map = ta_formats::tnt::TntMap { width: 128, height: 128, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![10; 128 * 128], features: vec![ta_formats::tnt::FEATURE_NONE; 128 * 128], feature_names: Vec::new(), minimap: Default::default() };
        let players: Vec<_> = (0..2).map(|color| ta_sim::PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
        let mut sim = Sim::new(types, data, &map, &ta_data::MapInfo::default(), 0, &players, 7);
        let (eye, tower) = (sim.types.id("EYE").unwrap(), sim.types.id("TOWER").unwrap());
        let at = |x: i64| FxVec3::from_ints(x, 0, 600);
        let watch = |sim: &mut Sim| (0..8).for_each(|_| sim.step(&[]));
        let first = sim.spawn_unit(eye, 0, at(300), 0, true).unwrap();
        let structure = sim.spawn_unit(tower, 1, at(500), 0, true).unwrap();
        let mut memory = Memory::default();
        // Before anything is looked at: nothing remembered.
        memory.update(&sim, Some(0));
        assert_eq!(memory.ghosts().count(), 0);
        watch(&mut sim);
        assert!(sim.seen_by(sim.unit(structure).unwrap(), 0) == Seen::Visible, "the eye sees the tower 200 away");
        memory.update(&sim, Some(0));
        let ghosts: Vec<_> = memory.ghosts().collect();
        assert_eq!(ghosts.len(), 1, "the enemy's structure is remembered, the viewer's own is not");
        assert!((ghosts[0].pos.x - 500.0).abs() < 32.0 && ghosts[0].type_id == tower, "where it stands (on its footprint's grid), what it is");
        // Out of sight again: it stays where it was last seen, even destroyed out there.
        sim.remove_unit(first);
        watch(&mut sim);
        memory.update(&sim, Some(0));
        assert_eq!(memory.ghosts().count(), 1, "out of sight it is remembered as it was");
        sim.remove_unit(structure);
        watch(&mut sim);
        memory.update(&sim, Some(0));
        assert_eq!(memory.ghosts().count(), 1, "what happens to it out of sight is not known");
        // Looked at again: it is not there.
        sim.spawn_unit(eye, 0, at(400), 0, true).unwrap();
        watch(&mut sim);
        memory.update(&sim, Some(0));
        assert_eq!(memory.ghosts().count(), 0, "its place in sight, it is not there: forgotten");
        // Another viewer starts afresh.
        sim.spawn_unit(tower, 1, at(700), 0, true).unwrap();
        sim.spawn_unit(eye, 0, at(600), 0, true).unwrap();
        watch(&mut sim);
        memory.update(&sim, Some(0));
        assert_eq!(memory.ghosts().count(), 1);
        memory.update(&sim, Some(1));
        assert_eq!(memory.ghosts().count(), 0, "a new viewer remembers nothing of another's, and the tower is its own");
    }

    #[test]
    fn only_a_place_in_sight_without_its_structure_forgets_it() {
        assert!(forgotten(true, false), "looked at and not there");
        assert!(!forgotten(true, true), "there");
        assert!(!forgotten(false, false), "out of sight stays as it was");
        assert!(!forgotten(false, true));
    }
}
