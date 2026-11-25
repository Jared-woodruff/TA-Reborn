//! Air transports with slots (a unit type's `TransportSlots`, `ta_data::TransportSlotDef`: TA
//! Reborn's own; Supreme Commander's transports use them). TA's air transports carry one unit,
//! whatever their files say (its Atlas says 5), and so do those that name no slots. One that names
//! them carries a unit in each: a passenger of class `k` (its `TransportClass`) takes the first free
//! slot of class `k` in the list's order, else the first of the smallest larger class free, and
//! hangs under that slot's piece, its top there, drawn ([`Unit::carry_slot`], its `carry_piece` the
//! slot's). A larger slot overlaps the smaller slot of each smaller class nearest it at rest (its
//! [`Slot::overlaps`], both ways): while one is held, the others are not free. A load order goes on
//! to the next free slot ([`Sim::room_for`]: the transport is full when none fits the unit); an
//! unload order sets every passenger down at once, round the place it was given, each where the
//! layout of the slots puts it (twice as far apart as they ride), on the nearest ground it can
//! stand on clear of the others (`Sim::unload_slotted`). The script is told `BeginTransport` and
//! `EndTransport` as TA's are; `QueryTransport` is not asked.
//!
//! [`Unit::carry_slot`]: crate::unit::Unit::carry_slot

use crate::defs::{Locomotion, ModelDef, UnitType};
use crate::events::SimEvent;
use crate::fixed::{Fx, FxVec3, rotate_y};
use crate::unit::{Unit, UnitId};
use crate::world::World;

use super::Sim;

/// One of an air transport's slots, compiled: the model piece its passenger hangs from, its class,
/// the piece's place at rest (in the model, its heading 0), and the slots that may not be held with
/// it (the larger slots it is under, and the smaller under it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub piece: usize,
    pub class: u32,
    pub rest: FxVec3,
    pub overlaps: Vec<u8>,
}

/// A model's pieces' places at rest (each its parents' offsets and its own).
fn rest_places(model: &ModelDef) -> Vec<FxVec3> {
    let mut out: Vec<FxVec3> = Vec::with_capacity(model.pieces.len());
    for p in &model.pieces {
        let base = p.parent.and_then(|q| out.get(q).copied()).unwrap_or(FxVec3::ZERO);
        out.push(base + p.offset);
    }
    out
}

/// An air transport's slots (`TransportSlots`), compiled: those whose pieces its model has, each
/// overlapping the slot of each larger class nearest it (the first in the list of those as near).
/// None for a unit that does not fly, as TA Reborn plays slots for air transports alone.
pub(crate) fn compile_slots(u: &ta_data::UnitDef, model: &ModelDef, locomotion: Locomotion) -> Vec<Slot> {
    if locomotion != Locomotion::Air || u.transport_slots.is_empty() {
        return Vec::new();
    }
    let rest = rest_places(model);
    let mut slots: Vec<Slot> = Vec::new();
    for s in &u.transport_slots {
        match model.piece_index(&s.piece) {
            Some(piece) => slots.push(Slot { piece, class: s.class, rest: rest.get(piece).copied().unwrap_or(FxVec3::ZERO), overlaps: Vec::new() }),
            None => log::warn!("{}: its transport slot names piece {}, which its model has not; left out", u.name, s.piece),
        }
    }
    let far = |a: FxVec3, b: FxVec3| {
        let d = a - b;
        let (x, y, z) = (d.x.0 as i128, d.y.0 as i128, d.z.0 as i128);
        x * x + y * y + z * z
    };
    for i in 0..slots.len() {
        let mut larger: Vec<u32> = slots.iter().map(|s| s.class).filter(|&c| c > slots[i].class).collect();
        larger.sort_unstable();
        larger.dedup();
        for class in larger {
            // (The nearest slot of that class, the first of those as near.)
            let Some(near) = (0..slots.len()).filter(|&j| slots[j].class == class).min_by_key(|&j| (far(slots[i].rest, slots[j].rest), j)) else { continue };
            for (a, b) in [(i, near), (near, i)] {
                if !slots[a].overlaps.contains(&(b as u8)) {
                    slots[a].overlaps.push(b as u8);
                }
            }
        }
    }
    for s in &mut slots {
        s.overlaps.sort_unstable();
    }
    slots
}

/// The slot a passenger of `class` takes, with `taken` held: the first free one of its class in
/// the list's order, else of the smallest larger class with one free. None: no room.
pub fn slot_for(slots: &[Slot], taken: &[u8], class: u32) -> Option<u8> {
    let free = |k: usize| !taken.contains(&(k as u8)) && slots[k].overlaps.iter().all(|o| !taken.contains(o));
    (0..slots.len()).filter(|&k| slots[k].class >= class && free(k)).min_by_key(|&k| (slots[k].class, k)).map(|k| k as u8)
}

/// How many passengers of the smallest class (1) the slots carry at once, filled in order.
pub(crate) fn slot_capacity(slots: &[Slot]) -> u32 {
    let mut taken: Vec<u8> = Vec::new();
    while let Some(s) = slot_for(slots, &taken, 1) {
        taken.push(s);
    }
    taken.len() as u32
}

impl UnitType {
    /// Whether this transport has slots (`TransportSlots`): it carries a unit in each.
    pub fn has_slots(&self) -> bool {
        !self.transport_slots.is_empty()
    }

    /// Whether passengers of these classes, taken aboard in this order, all find a slot (a
    /// transport without slots: no more of them than it carries). What the computer players ask
    /// before sending passengers to one.
    pub fn fits(&self, classes: &[u32]) -> bool {
        if !self.has_slots() {
            return classes.len() as u32 <= self.transport_capacity;
        }
        let mut taken: Vec<u8> = Vec::with_capacity(classes.len());
        for &class in classes {
            let Some(s) = slot_for(&self.transport_slots, &taken, class) else { return false };
            taken.push(s);
        }
        true
    }
}

impl Sim {
    /// The slots a transport's passengers hold.
    fn taken_slots(&self, t: &Unit) -> Vec<u8> {
        t.cargo.iter().filter_map(|&c| self.unit(c)).filter(|c| c.carried_by == Some(t.id)).filter_map(|c| c.carry_slot).collect()
    }

    /// Whether transport `t` has room aboard for a unit of type `passenger` now: a free slot that
    /// takes it (one with slots), else fewer aboard than it carries. (Whether it can carry such a
    /// unit at all is `UnitType::can_carry`.)
    pub fn room_for(&self, t: &Unit, passenger: &UnitType) -> bool {
        let tt = self.types.get(t.type_id);
        if tt.has_slots() {
            slot_for(&tt.transport_slots, &self.taken_slots(t), passenger.transport_class).is_some()
        } else {
            (t.cargo.len() as u32) < tt.transport_capacity
        }
    }

    /// Takes `unit` aboard `transport`: into the slot that takes it (one with slots), else stowed
    /// inside, as a mission puts its passengers aboard. False when there is no room for it.
    pub(crate) fn board(&mut self, transport: UnitId, unit: UnitId) -> bool {
        let types = self.types.clone();
        let (Some(t), Some(u)) = (self.unit(transport), self.unit(unit)) else { return false };
        let tt = types.get(t.type_id);
        if !self.room_for(t, types.get(u.type_id)) {
            return false;
        }
        if !tt.has_slots() {
            self.attach(transport, unit, None);
            return self.unit(unit).is_some_and(|u| u.carried_by == Some(transport));
        }
        let Some(slot) = slot_for(&tt.transport_slots, &self.taken_slots(t), types.get(u.type_id).transport_class) else { return false };
        self.attach(transport, unit, Some(tt.transport_slots[slot as usize].piece));
        let Some(u) = self.unit_mut(unit).filter(|u| u.carried_by == Some(transport)) else { return false };
        u.carry_slot = Some(slot);
        true
    }

    /// How far under its slot's piece a passenger of a transport with slots hangs (its top there):
    /// its model's height. Nothing for another.
    pub(super) fn hang(&self, u: &Unit) -> Fx {
        if u.carry_slot.is_some() { self.types.get(u.type_id).model.height } else { Fx::ZERO }
    }

    /// How high over the ground a transport with slots holds to take a unit `top` high under its
    /// slot `slot`: the slot's piece a little over the unit's top.
    pub(super) fn slot_pickup_height(&self, t: &UnitType, top: Fx) -> Fx {
        let lowest = t.transport_slots.iter().map(|s| s.rest.y).min().unwrap_or(Fx::ZERO);
        top + Fx::from_int(8) - lowest.min(Fx::ZERO)
    }

    /// An unload order of a transport with slots, over its place (`dest`) at the height to set its
    /// cargo down: every passenger set down at once, each where the layout of the slots puts it
    /// round the place (twice as far apart as they ride, turned with the transport), on the
    /// nearest ground it can stand on clear of those set down before it. Its script is told
    /// `EndTransport` once it is empty. False (and the rest aboard) when one found no room.
    pub(super) fn unload_slotted(&mut self, id: UnitId, dest: FxVec3) -> bool {
        let types = self.types.clone();
        let Some(t) = self.unit(id) else { return false };
        let tt = types.get(t.type_id);
        let heading = t.heading;
        let mut aboard: Vec<(u8, UnitId)> = t.cargo.iter().filter_map(|&c| self.unit(c)).filter(|c| c.carried_by == Some(id)).map(|c| (c.carry_slot.unwrap_or(u8::MAX), c.id)).collect();
        aboard.sort_unstable();
        let mut placed: Vec<(FxVec3, Fx)> = Vec::new();
        let mut room = true;
        for (slot, unit) in aboard {
            let Some(type_id) = self.unit(unit).map(|u| u.type_id) else { continue };
            let offset = tt.transport_slots.get(slot as usize).map_or(FxVec3::ZERO, |s| FxVec3::new(s.rest.x * 2, Fx::ZERO, s.rest.z * 2));
            let at = dest + rotate_y(offset, heading);
            let radius = types.get(type_id).radius;
            let Some(spot) = self.drop_spot_clear(type_id, at, &placed, radius) else {
                room = false;
                continue;
            };
            if self.detach(id, unit, spot) {
                placed.push((spot, radius));
            } else {
                room = false;
            }
        }
        if self.unit(id).is_some_and(|u| u.cargo.is_empty()) {
            self.start_script(id, "EndTransport", &[]);
        }
        if !room {
            self.events.push(SimEvent::OrderFailed { unit: id, reason: "no room to unload" });
        }
        room
    }

    /// The nearest spot within a few cells of `at` where a unit of `type_id` (of `radius`) can
    /// stand, clear of the units set down at `placed` (each with its radius).
    fn drop_spot_clear(&self, type_id: usize, at: FxVec3, placed: &[(FxVec3, Fx)], radius: Fx) -> Option<FxVec3> {
        let t = self.types.get(type_id);
        let spec = self.move_specs[type_id];
        let center = World::pos_to_anchor(at, t.footprint);
        for r in 0..=8i32 {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let c = (center.0 + dx, center.1 + dz);
                    if !self.world.passable(&spec, c, None) {
                        continue;
                    }
                    let p = self.world.anchor_to_pos(c, t.footprint);
                    if placed.iter().all(|&(q, qr)| !p.within_xz(q, qr + radius)) {
                        return Some(p);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::PieceDef;

    fn model(pieces: &[(&str, i64, i64)]) -> ModelDef {
        let mut m = ModelDef::default();
        m.pieces.push(PieceDef { name: "base".into(), parent: None, offset: FxVec3::ZERO, top: Fx::ZERO });
        for &(name, x, z) in pieces {
            m.pieces.push(PieceDef { name: name.into(), parent: Some(0), offset: FxVec3::from_ints(x, -4, z), top: Fx::ZERO });
        }
        m
    }

    fn lifter(slots: &str) -> ta_data::UnitDef {
        let doc = ta_formats::tdf::parse(&format!("[UNITINFO]{{UnitName=LIFTER; CanFly=1; TransportSlots={slots};}}"));
        ta_data::UnitDef::from_tdf(doc.section("UNITINFO").unwrap()).unwrap()
    }

    /// Supreme Commander's small transport's layout: three small slots a side, a medium one
    /// between the front two of each side, a large one in the middle: six small, two medium or
    /// one large.
    #[test]
    fn larger_slots_overlap_the_smaller_nearest_them() {
        let m = model(&[("L1", 10, 20), ("L2", 10, 0), ("L3", 10, -20), ("R1", -10, 20), ("R2", -10, 0), ("R3", -10, -20), ("ML", 10, 10), ("MR", -10, 10), ("BIG", 0, 10)]);
        let slots = compile_slots(&lifter("L1 L2 L3 R1 R2 R3 ML:2 MR:2 BIG:3"), &m, Locomotion::Air);
        assert_eq!(slots.len(), 9);
        assert_eq!(slot_capacity(&slots), 6, "six small");
        let fill = |class: u32| {
            let mut taken = Vec::new();
            while let Some(s) = slot_for(&slots, &taken, class) {
                taken.push(s);
            }
            taken
        };
        assert_eq!(fill(2), [6, 7], "two medium");
        assert_eq!(fill(3), [8], "one large");
        // A small one aboard on one side leaves the other side's medium slot free; one on each
        // side leaves none.
        assert_eq!(slot_for(&slots, &[2], 2), Some(7));
        assert_eq!(slot_for(&slots, &[0, 3], 2), None, "both front small slots held: no medium room");
        assert_eq!(slot_for(&slots, &[6], 3), None, "a medium aboard: no large room");
        // A small passenger takes a larger slot only where no small one is free.
        let gunship = compile_slots(&lifter("Hold:2"), &model(&[("Hold", 0, 0)]), Locomotion::Air);
        assert_eq!(slot_for(&gunship, &[], 1), Some(0));
        assert_eq!(slot_for(&gunship, &[], 3), None, "too small for a large one");
        // Not on a ship; a piece the model has not is left out.
        assert!(compile_slots(&lifter("L1"), &m, Locomotion::Naval).is_empty());
        assert_eq!(compile_slots(&lifter("L1 Nowhere"), &m, Locomotion::Air).len(), 1);
    }
}
