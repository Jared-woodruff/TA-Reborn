//! Bridges running COB scripts to the simulation (`get`/`set` values, effects).

use crate::cob::{CobHost, CobInstance};
use crate::defs::UnitType;
use crate::events::SimEvent;
use crate::fixed::{Fx, FxVec3, atan2};
use crate::unit::{Unit, piece_world_position};
use crate::world::World;

/// COB unit value ids (`get`/`set`).
pub mod value {
    pub const ACTIVATION: i32 = 1;
    pub const STANDING_MOVE_ORDERS: i32 = 2;
    pub const STANDING_FIRE_ORDERS: i32 = 3;
    pub const HEALTH: i32 = 4;
    pub const IN_BUILD_STANCE: i32 = 5;
    pub const BUSY: i32 = 6;
    pub const PIECE_XZ: i32 = 7;
    pub const PIECE_Y: i32 = 8;
    pub const UNIT_XZ: i32 = 9;
    pub const UNIT_Y: i32 = 10;
    pub const UNIT_HEIGHT: i32 = 11;
    pub const XZ_ATAN: i32 = 12;
    pub const XZ_HYPOT: i32 = 13;
    pub const ATAN: i32 = 14;
    pub const HYPOT: i32 = 15;
    pub const GROUND_HEIGHT: i32 = 16;
    pub const BUILD_PERCENT_LEFT: i32 = 17;
    pub const YARD_OPEN: i32 = 18;
    pub const BUGGER_OFF: i32 = 19;
    pub const ARMORED: i32 = 20;
    pub const VETERAN_LEVEL: i32 = 32;
    /// TA 3.1's questions about units by id (the `exthead.h` the mods' scripts include; TA's own
    /// scripts never ask them): the lowest and highest ids a unit may have, the unit's own, and
    /// another's player, how much of it is left to build, whether it is the unit's ally, and
    /// whether this computer owns it. The mods made for the patched engines build on them: TA:
    /// Escalation in 326 of its 550 scripts, Total Mayhem's upgrades (a geothermal plant finds
    /// its upgrade module by going through every id, then takes it on).
    pub const MIN_ID: i32 = 69;
    pub const MAX_ID: i32 = 70;
    pub const MY_ID: i32 = 71;
    pub const UNIT_TEAM: i32 = 72;
    pub const UNIT_BUILD_PERCENT_LEFT: i32 = 73;
    pub const UNIT_ALLIED: i32 = 74;
    pub const UNIT_IS_ON_THIS_COMP: i32 = 75;
    /// Whether the unit is cloaked, from a later patch: Total Mayhem's mines cloak themselves
    /// with it while they wait, and uncloak to go off. (Free: TA's cloak costs the energy the
    /// unit's `CloakCost` says, which a mine's file does not.)
    pub const UNIT_CLOAKED: i32 = 111;
}

/// How much of a unit is left to build, as TA's scripts are told it (`BUILD_PERCENT_LEFT`): 0
/// when it is finished, up to 100 for a frame just begun.
pub fn build_percent_left(progress: Fx) -> i32 {
    let left = Fx::ONE - progress.min(Fx::ONE);
    (left * Fx::from_int(100)).to_int().clamp(0, 100) as i32
}

/// Packs integer world x/z the way TA scripts expect (`PACKXZ`): two 16-bit fields that wrap
/// round, as TA's engine packs them. Scripts subtract one packed point from another and read
/// the difference back (`XZ_ATAN`, `XZ_HYPOT`), which the wrapping keeps right for any two
/// points less than 32,768 px apart on a map of any size; an absolute point is read back
/// near the unit asking ([`unpack_xz_near`]).
pub fn pack_xz(x: Fx, z: Fx) -> i32 {
    let field = |v: Fx| v.to_int() as i16 as u16 as i32;
    (field(x) << 16) | field(z)
}

/// A packed difference (or a point on a map under 32,768 px): signed 16-bit fields.
pub fn unpack_xz(v: i32) -> (Fx, Fx) {
    (Fx::from_int((v >> 16) as i64), Fx::from_int(v as i16 as i64))
}

/// A packed point read back as the point nearest `near` with those fields: exact for points
/// within 32,767 px of it (on TA's maps, any point at all; on bigger ones, any a unit's
/// script asks about near itself).
pub fn unpack_xz_near(v: i32, near: FxVec3) -> (Fx, Fx) {
    let field = |packed: i32, at: Fx| {
        let at = at.to_int();
        Fx::from_int(at + (packed as i64 - at) as i16 as i64)
    };
    (field(v >> 16, near.x), field(v, near.z))
}

/// An angle as TA gives it to a script (`get XZ_ATAN`, `get ATAN`; the heading and pitch it
/// calls `AimPrimary`..`AimTertiary` and `StartBuilding` with, the wind's heading in
/// `SetDirection`): a 16-bit angle, 0..65535, never negative. TA's own scripts only turn
/// pieces to these (where either way round is the same turn), but mods' scripts compare them.
/// The walkers that keep their feet on the ground by searching for the spot (TA: Enhanced's
/// Cavor, TA: Escalation's walking cruisers, Total Mayhem's Amphibious Construction Sub) take
/// `XZ_ATAN(spot - foot) > 16380 AND < 49140` for "the spot is ahead of the foot" and
/// `ATAN(spot_y - foot_y, distance) < 16380` for "it is above it": with angles from
/// -32768 they never found the ground and held their legs up in the air. Transports' unloading
/// arms (TA: Escalation's, TA: Enhanced's SpiderBot) take 32768 and up as the other half turn.
/// Aims pick their guns by the heading's quarter (Total Mayhem's shipyards and flagships:
/// `heading > 24575 AND <= 40959` for one behind) or keep a gun to its arc (TA: Escalation's
/// `heading > 16384 AND < 49152`, behind), and the SpiderBot's laser stops at 50 degrees down
/// (`pitch > 65536 - <50>`).
pub fn script_angle(a: i32) -> i32 {
    a as u16 as i32
}

/// Read-only information about other units that scripts may query.
#[derive(Clone, Copy)]
pub struct OtherUnit {
    pub pos: FxVec3,
    pub height: Fx,
    /// Its player.
    pub owner: u8,
    /// How much of it is left to build ([`build_percent_left`]).
    pub build_left: i32,
}

impl OtherUnit {
    /// What a script may ask about the unit `u`.
    pub fn of(u: &Unit, types: &crate::defs::UnitTypes) -> OtherUnit {
        let t = types.get(u.type_id);
        OtherUnit { pos: u.model_pos(t), height: t.model.height, owner: u.owner, build_left: build_percent_left(u.build_progress) }
    }
}

/// World and callback requests made by a script, applied by the simulation once
/// the script has run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostRequest {
    /// Script-driven activation changes also invoke Activate/Deactivate.
    Activation { unit: crate::unit::UnitId, on: bool },
    /// Update a building's passable yard cells after the script changes YARD_OPEN.
    Yard { unit: crate::unit::UnitId, open: bool },
    /// Carry `unit` (script id) on COB `piece` (-1 = stowed inside, hidden).
    Attach { transport: crate::unit::UnitId, unit: i32, piece: i32 },
    /// Set `unit` (script id) down where it hangs.
    Drop { transport: crate::unit::UnitId, unit: i32 },
}

pub struct UnitHost<'a> {
    pub unit: &'a mut Unit,
    pub requests: &'a mut Vec<HostRequest>,
    pub unit_type: &'a UnitType,
    pub world: &'a World,
    pub events: &'a mut Vec<SimEvent>,
    pub now_ms: i64,
    /// Lookup of other units by script id.
    pub lookup: &'a dyn Fn(i32) -> Option<OtherUnit>,
    /// The highest script id a unit may have (`MAX_ID`): as many as the battle has places for
    /// units.
    pub max_id: i32,
    /// The players allied with the unit's, its own among them (bit n: player n), for
    /// `UNIT_ALLIED`.
    pub allies: u64,
}

impl UnitHost<'_> {
    fn piece_pos(&self, instance: &CobInstance, cob_piece: i32) -> Option<FxVec3> {
        let mp = instance.program.piece_map.get(usize::try_from(cob_piece).ok()?).copied().flatten()?;
        Some(piece_world_position(self.unit, self.unit_type, Some(instance), mp))
    }
}

impl CobHost for UnitHost<'_> {
    fn now_ms(&self) -> i64 {
        self.now_ms
    }

    /// (The unit's own random numbers: see [`Unit::rng`].)
    fn random(&mut self, lo: i32, hi: i32) -> i32 {
        self.unit.rng.range_i32(lo, hi)
    }

    fn get_value(&mut self, instance: &CobInstance, id: i32, args: [i32; 4]) -> i32 {
        use value::*;
        let u = &*self.unit;
        match id {
            ACTIVATION => u.activated as i32,
            STANDING_MOVE_ORDERS => u.move_state as i32,
            STANDING_FIRE_ORDERS => u.fire_state as i32,
            HEALTH => u.health_percent(self.unit_type),
            IN_BUILD_STANCE => u.in_build_stance as i32,
            BUSY => u.busy as i32,
            PIECE_XZ => self.piece_pos(instance, args[0]).map_or(0, |p| pack_xz(p.x, p.z)),
            PIECE_Y => self.piece_pos(instance, args[0]).map_or(0, |p| p.y.to_cob()),
            UNIT_XZ => {
                if args[0] == u.id.script_id() || args[0] == 0 {
                    pack_xz(u.pos.x, u.pos.z)
                } else {
                    (self.lookup)(args[0]).map_or(0, |o| pack_xz(o.pos.x, o.pos.z))
                }
            }
            UNIT_Y => {
                if args[0] == u.id.script_id() || args[0] == 0 {
                    u.model_pos(self.unit_type).y.to_cob()
                } else {
                    (self.lookup)(args[0]).map_or(0, |o| o.pos.y.to_cob())
                }
            }
            UNIT_HEIGHT => {
                if args[0] == u.id.script_id() || args[0] == 0 {
                    self.unit_type.model.height.to_cob()
                } else {
                    (self.lookup)(args[0]).map_or(0, |o| o.height.to_cob())
                }
            }
            // The heading, relative to the unit's, that points a piece along -(dx, dz): TA's
            // boom transports aim with XZ_ATAN(piece - target) (see docs/FORMATS.md). As TA
            // gives it, 0..65535 ([`script_angle`]).
            XZ_ATAN => {
                let (dx, dz) = unpack_xz(args[0]);
                script_angle(atan2(-dx, -dz) - u.heading)
            }
            XZ_HYPOT => {
                let (dx, dz) = unpack_xz(args[0]);
                FxVec3::new(dx, Fx::ZERO, dz).length_xz().to_cob()
            }
            // ATAN(a, b): the angle of a over b (a slope: a up, b along), 0..65535 too: a slope
            // down is just short of a whole turn, not negative ([`script_angle`]).
            ATAN => script_angle(atan2(Fx(args[0] as i64), Fx(args[1] as i64))),
            HYPOT => {
                // (Squares of two i32s overflow an i64 together; the root saturates.)
                let (a, b) = (args[0] as i128, args[1] as i128);
                crate::fixed::isqrt_u128((a * a + b * b) as u128).min(i32::MAX as u128) as i32
            }
            GROUND_HEIGHT => {
                let (x, z) = unpack_xz_near(args[0], u.pos);
                self.world.height_at(x, z).to_cob()
            }
            BUILD_PERCENT_LEFT => build_percent_left(u.build_progress),
            YARD_OPEN => u.yard_open as i32,
            BUGGER_OFF => 0,
            ARMORED => u.armored as i32,
            VETERAN_LEVEL => u.kills.saturating_mul(100).min(i32::MAX as u64) as i32,
            UNIT_CLOAKED => u.cloak_on as i32,
            // Script ids are the units' places, from 1.
            MIN_ID => 1,
            MAX_ID => self.max_id,
            MY_ID => u.id.script_id(),
            UNIT_TEAM | UNIT_BUILD_PERCENT_LEFT | UNIT_ALLIED | UNIT_IS_ON_THIS_COMP => {
                // The unit asked about (no id, as with UNIT_XZ: the unit itself); no unit there:
                // 0 to every question.
                let (owner, left) = if args[0] == u.id.script_id() || args[0] == 0 {
                    (u.owner, build_percent_left(u.build_progress))
                } else {
                    match (self.lookup)(args[0]) {
                        Some(o) => (o.owner, o.build_left),
                        None => return 0,
                    }
                };
                match id {
                    UNIT_TEAM => owner as i32,
                    UNIT_BUILD_PERCENT_LEFT => left,
                    UNIT_ALLIED => (self.allies >> owner.min(63) & 1) as i32,
                    // TA's engine ran each player's units on that player's computer, and scripts
                    // made changes there alone (taking a unit on, letting it go). Every computer
                    // plays the whole battle here, so every unit is this computer's: each makes
                    // the change in its own copy, as the owner's did in TA.
                    _ => 1,
                }
            }
            _ => 0,
        }
    }

    fn set_value(&mut self, id: i32, v: i32) {
        use value::*;
        let u = &mut *self.unit;
        match id {
            ACTIVATION => {
                if u.activated != (v != 0) {
                    u.activated = v != 0;
                    self.requests.push(HostRequest::Activation { unit: u.id, on: u.activated });
                }
            }
            STANDING_MOVE_ORDERS => u.move_state = v.clamp(0, 2) as u8,
            STANDING_FIRE_ORDERS => u.fire_state = v.clamp(0, 2) as u8,
            IN_BUILD_STANCE => u.in_build_stance = v != 0,
            BUSY => u.busy = v != 0,
            YARD_OPEN => {
                if u.yard_open != (v != 0) {
                    u.yard_open = v != 0;
                    self.requests.push(HostRequest::Yard { unit: u.id, open: u.yard_open });
                }
            }
            ARMORED => u.armored = v != 0,
            UNIT_CLOAKED => u.cloak_on = v != 0,
            _ => {}
        }
    }

    fn attach_unit(&mut self, unit: i32, piece: i32) {
        self.requests.push(HostRequest::Attach { transport: self.unit.id, unit, piece });
    }

    fn drop_unit(&mut self, unit: i32) {
        self.requests.push(HostRequest::Drop { transport: self.unit.id, unit });
    }

    fn explode(&mut self, instance: &CobInstance, piece: usize, flags: i32) {
        let Some(model_piece) = instance.program.piece_map.get(piece).copied().flatten() else { return };
        let pos = piece_world_position(self.unit, self.unit_type, Some(instance), model_piece);
        self.events.push(SimEvent::PieceExplode {
            unit: self.unit.id,
            type_id: self.unit.type_id,
            model_piece,
            pos,
            heading: self.unit.heading,
            flags,
        });
    }

    fn emit_sfx(&mut self, instance: &CobInstance, piece: usize, kind: i32) {
        if let Some(pos) = self.piece_pos(instance, piece as i32) {
            self.events.push(SimEvent::Sfx { unit: self.unit.id, pos, kind });
        }
    }
}
