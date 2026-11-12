//! Aircraft flight, landing clearance, and reserved repair decks.

use super::*;

/// Ticks an aircraft has had nothing to do before it lands.
const IDLE_BEFORE_LANDING: u32 = 60;
/// How far round it (anchor cells: 256 px) an aircraft that may not set down where it is looks
/// for the nearest place it may. Finding none, it stays in the air (hovering, as over the sea),
/// and looks again every [`LANDING_SEARCH_EVERY`] ticks.
const LANDING_SEARCH: i32 = 16;
const LANDING_SEARCH_EVERY: u32 = 30;
/// A hovering aircraft idle over the sea (a scout, a builder, a transport) loiters round a circle
/// of this radius (px), larger by a step for each of the next three slots (see `update_air`), so
/// that several sent to one place keep apart.
const LOITER_RADIUS: i64 = 128;
const LOITER_STEP: i64 = 32;
/// ...its circle lying to a side of where it came to rest, turned by this angle (the golden
/// angle, 65536 a turn) from one slot to the next.
const LOITER_SIDE: i32 = 25033;
/// An aircraft on a mission (a patrol, an attack-move or an attack) worn below this share of
/// its health (numerator, denominator: under two-thirds) breaks off for a repair pad, as TA's
/// do: the only figure TA's makers and its guides give for it. See [`Sim::start_repair_trip`].
const REPAIR_TRIP_HEALTH: (i64, i64) = (2, 3);
/// How often (ticks; each aircraft on a tick of its own, by its slot) an aircraft on a mission
/// looks at whether it is due a repair trip, and for a free deck when it is.
const REPAIR_TRIP_EVERY: u64 = 30;
/// The furthest past where a plane was sent that it looks for a repair deck to land on (px):
/// as far as it takes to stop from its top speed, but no more than this (a mod's plane that
/// hardly brakes looks no further).
const DECK_OVERSHOOT_MAX: i64 = 256;

/// How far an aircraft of `t` flies on from its top speed before it stops (v² / 2 · brake, px):
/// a plane sent somewhere slows down only once it is there.
fn stopping_distance(t: &UnitType) -> Fx {
    (t.max_speed * t.max_speed / (t.brake * 2)).clamp(Fx::ZERO, Fx::from_int(DECK_OVERSHOOT_MAX))
}

/// A repair deck: a pad or carrier, and the model piece its script names (None for one with no
/// script, or naming none: over its model).
type Deck = (UnitId, Option<usize>);

impl Sim {
    /// Where it may set down with its footprint about `pos` (see `World::air_landing_height`),
    /// at the height it rests at.
    fn air_ground_height(&self, t: &UnitType, pos: FxVec3) -> Option<Fx> {
        self.world.air_landing_height(pos, &t.landing())
    }

    /// Where an aircraft has taken the ground to land on, if it has: landed there (not on a
    /// deck), or with nothing to do flying to a place of its own to set down (its goal), or
    /// past its idle delay over where it is (`true`: coming down, or hovering where there is no
    /// place to land).
    fn parking_spot(&self, o: &Unit) -> Option<(FxVec3, bool)> {
        if o.dead || o.carried_by.is_some() || o.air_pad.is_some() || self.unit_type(o).locomotion != Locomotion::Air {
            return None;
        }
        if o.landed {
            return Some((o.pos, false));
        }
        let idle = o.orders.is_empty() && o.build.is_none() && o.cargo.is_empty() && o.weapons.iter().all(|w| w.target.is_none());
        match o.path_goal {
            _ if !idle || !o.is_complete() => None,
            Some(goal) => Some((goal, false)),
            None => (o.idle_ticks >= IDLE_BEFORE_LANDING).then_some((o.pos, true)),
        }
    }

    /// The places other aircraft have taken within `reach` of `pos` (see [`Sim::parking_spot`]),
    /// their middles and sizes; of those only over theirs, those `first` says go first.
    fn parked_near(&self, id: UnitId, pos: FxVec3, reach: Fx, first: impl Fn(&Unit) -> bool) -> Vec<(FxVec3, Fx)> {
        let mut near = Vec::new();
        self.grid.query(pos, reach, &mut near);
        near.sort_unstable();
        near.into_iter()
            .filter_map(|slot| self.units.get(slot as usize)?.as_ref())
            .filter(|o| o.id != id)
            .filter_map(|o| {
                let (spot, over) = self.parking_spot(o)?;
                (!over || first(o)).then(|| (spot, self.unit_type(o).radius))
            })
            .collect()
    }

    /// Whether a place for an aircraft of `t` with its middle at `point` overlaps one another
    /// aircraft has taken (its footprint's size apart from it at least: the crowd's separation
    /// then never has to push them apart onto other ground).
    fn spot_taken(taken: &[(FxVec3, Fx)], t: &UnitType, point: FxVec3) -> bool {
        taken.iter().any(|&(at, r)| point.within_xz(at, t.radius + r - Fx::ONE))
    }

    /// The nearest place round an aircraft where it may set down, clear of those other aircraft
    /// have taken: its middle, at the height it would rest at. Ring by ring of anchor cells out
    /// to [`LANDING_SEARCH`], the nearest middle to where it is (of equals, the northernmost,
    /// then the westernmost), so every copy of the game picks the same.
    fn nearby_air_landing(&self, id: UnitId, t: &UnitType) -> Option<FxVec3> {
        let pos = self.unit(id)?.pos;
        let landing = t.landing();
        let here = World::pos_to_anchor(pos, t.footprint);
        // (Those flying in to places of their own may come from as far again.)
        let taken = self.parked_near(id, pos, Fx::from_int(LANDING_SEARCH as i64 * CELL * 2 + 64), |_| true);
        let mut best: Option<(i128, i32, i32, FxVec3)> = None;
        for r in 0..=LANDING_SEARCH {
            // (A middle in ring r is at least r - 1 cells from it: nothing further out is nearer.)
            let floor = Fx::from_int((r as i64 - 1).max(0) * CELL).0 as i128;
            if best.is_some_and(|(d, ..)| floor * floor > d) {
                break;
            }
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let (ax, az) = (here.0 + dx, here.1 + dz);
                    let point = anchor_middle((ax, az), t.footprint);
                    let d = (point - pos).xz().length_sq_raw();
                    if best.is_some_and(|(bd, bz, bx, _)| (d, az, ax) >= (bd, bz, bx)) || Self::spot_taken(&taken, t, point) {
                        continue;
                    }
                    if let Some(y) = self.world.air_landing_height(point, &landing) {
                        best = Some((d, az, ax, FxVec3::new(point.x, y, point.z)));
                    }
                }
            }
        }
        best.map(|(.., spot)| spot)
    }

    /// The highest ground (or sea) under the straight way from `from` to `to`, every half cell:
    /// an aircraft flying to its place to land keeps over it.
    fn highest_on_way(world: &World, from: FxVec3, to: FxVec3) -> Fx {
        let way = (to - from).xz();
        let steps = (way.length_xz().to_int() / (CELL / 2)).clamp(1, 1024);
        (0..=steps)
            .map(|i| from + way.scale(Fx::from_ratio(i, steps)))
            .map(|p| world.height_at(p.x, p.z))
            .fold(world.sea_level, Fx::max)
    }

    /// Whether `p` is a repair deck for `owner`'s aircraft: an allied pad or carrier
    /// (`IsAirBase`), finished, not carried, and switched on (TA's are `OnOffable`: switched off,
    /// one takes no aircraft and mends none).
    pub(super) fn usable_air_base(&self, owner: u8, p: &Unit) -> bool {
        let t = self.unit_type(p);
        t.def.is_airbase && !p.dead && p.is_complete() && p.carried_by.is_none() && self.allied(owner, p.owner) && (p.activated || !t.def.on_offable)
    }

    /// Deck geometry follows the model hierarchy and the platform's current position.
    fn air_pad_point(&self, owner: u8, (id, piece): Deck) -> Option<FxVec3> {
        let pad = self.unit(id).filter(|u| self.usable_air_base(owner, u))?;
        let t = self.unit_type(pad);
        Some(match piece {
            Some(piece) => piece_world_position(pad, t, pad.cob.as_ref(), piece) + FxVec3::new(Fx::ZERO, t.model.pieces.get(piece)?.top, Fx::ZERO),
            None => pad.pos + FxVec3::new(Fx::ZERO, t.model.height, Fx::ZERO),
        })
    }

    /// A platform's decks: the model pieces its script names (`QueryLandingPad`), in its order;
    /// a platform without a script, or naming none, has one over its model.
    fn deck_pieces(&mut self, pad: UnitId) -> Vec<Option<usize>> {
        let params = self.call_script(pad, "QueryLandingPad", &[0, 0]);
        let mut pieces: Vec<_> = params.map(|p| p.into_iter().filter_map(|p| self.cob_to_model_piece(pad, p)).map(Some).collect()).unwrap_or_default();
        if pieces.is_empty() { pieces.push(None); }
        pieces.dedup();
        pieces
    }

    /// The first deck of `pads` (in the order given; a platform's in its script's order) free
    /// for aircraft `id` of `owner`, and the aircraft holding it that gives it up, if one does:
    /// distinct aircraft never hold one deck, but with `takes` (a damaged aircraft looking) a deck
    /// held by an idle aircraft that needs no mending counts as free (see [`Sim::take_deck`]).
    /// One on a repair trip, or being mended, keeps its deck.
    fn free_deck(&mut self, id: UnitId, owner: u8, pads: &[UnitId], takes: bool) -> Option<(Deck, Option<UnitId>)> {
        if pads.is_empty() {
            return None;
        }
        // (Who holds which deck, and whether it would give it up. Any order but a repair trip's
        // releases a deck, so one holding a deck off a trip has nothing else to do.)
        let holders: Vec<(Deck, UnitId, bool)> = self.units.iter().flatten()
            .filter(|u| !u.dead && u.id != id)
            .filter_map(|u| Some((u.air_pad?, u.id, !u.repair_trip && u.health >= self.unit_type(u).max_health)))
            .collect();
        for &pad in pads {
            for piece in self.deck_pieces(pad) {
                let deck = (pad, piece);
                if self.air_pad_point(owner, deck).is_none() {
                    continue;
                }
                match holders.iter().find(|h| h.0 == deck) {
                    None => return Some((deck, None)),
                    Some(&(_, holder, gives_up)) if takes && gives_up => return Some((deck, Some(holder))),
                    Some(_) => {}
                }
            }
        }
        None
    }

    /// Aircraft `id` takes `deck`. The idle aircraft that held it (`holder`, see
    /// [`Sim::free_deck`]) gives it up: it takes off if it had landed there, and lands elsewhere
    /// as an idle aircraft does.
    fn take_deck(&mut self, id: UnitId, deck: Deck, holder: Option<UnitId>) {
        if let Some(h) = holder {
            let took_off = self.unit_mut(h).is_some_and(|o| {
                o.air_pad = None;
                if o.orders.is_empty() {
                    o.path_goal = None;
                    o.move_request = None;
                }
                std::mem::take(&mut o.landed)
            });
            if took_off {
                self.set_active(h, true);
            }
        }
        if let Some(u) = self.unit_mut(id) {
            u.air_pad = Some(deck);
        }
    }

    /// Query authored pads once when reserving one. Platforms without a script have
    /// one slot over their model; distinct aircraft cannot reserve the same piece.
    /// The pads within reach: their size and 24 px, and for a plane as far again as it flies on
    /// past where it was sent until it stops ([`stopping_distance`]: TA's Thunder and Hurricane,
    /// sent to a pad's middle, come to a stop 70 to 85 px beyond it, out of that reach). A
    /// damaged aircraft takes a deck from an idle one that needs no mending ([`Sim::free_deck`]).
    fn reserve_air_pad(&mut self, id: UnitId, t: &UnitType) -> Option<(Deck, Option<UnitId>)> {
        if self.types.airbase_reach <= Fx::ZERO { return None; }
        let u = self.unit(id)?;
        let (pos, owner, damaged) = (u.pos, u.owner, u.health < t.max_health);
        let further = if t.is_plane() { stopping_distance(t) } else { Fx::ZERO };
        let mut near = Vec::new();
        self.grid.query(pos, self.types.airbase_reach + further, &mut near);
        let mut pads: Vec<_> = near.into_iter().filter_map(|slot| self.units.get(slot as usize)?.as_ref())
            .filter(|p| p.id != id && self.usable_air_base(owner, p) && p.pos.within_xz(pos, self.unit_type(p).radius + Fx::from_int(24) + further))
            .map(|p| ((p.pos - pos).xz().length_sq_raw(), p.id)).collect();
        pads.sort_unstable();
        let pads: Vec<UnitId> = pads.into_iter().map(|(_, p)| p).collect();
        self.free_deck(id, owner, &pads, damaged)
    }

    /// Repair trips: an aircraft on a mission (patrol, attack-move or attack, its order now)
    /// worn below [`REPAIR_TRIP_HEALTH`] of its health, in the air with no deck, cargo or build
    /// of its own, breaks off for the nearest free deck (by distance, then the platform's id) of
    /// an allied repair pad or carrier that is finished and switched on, however far: a deck
    /// held by an idle aircraft that needs no mending counts as free ([`Sim::free_deck`]). Its
    /// orders wait (`unit_orders`) while it flies there and lands as an aircraft with nothing to
    /// do does (`update_air`); mended, it carries on ([`Sim::end_repair_trip`]). Every
    /// [`REPAIR_TRIP_EVERY`] ticks: none free, it carries on with its mission and looks again.
    fn start_repair_trip(&mut self, id: UnitId, t: &UnitType) {
        if self.types.airbase_reach <= Fx::ZERO || !(self.tick + id.slot as u64).is_multiple_of(REPAIR_TRIP_EVERY) {
            return;
        }
        let Some(u) = self.unit(id) else { return };
        let on_mission = matches!(u.orders.front(), Some(Order::Patrol(_) | Order::Fight(_) | Order::Attack(_)));
        let worn = u.health * REPAIR_TRIP_HEALTH.1 < t.max_health * REPAIR_TRIP_HEALTH.0;
        if !on_mission || !worn || u.repair_trip || u.landed || u.air_pad.is_some() || u.carried_by.is_some() || !u.cargo.is_empty() || u.build.is_some() {
            return;
        }
        let (pos, owner) = (u.pos, u.owner);
        let mut pads: Vec<(i128, UnitId)> = self.units.iter().flatten()
            .filter(|p| p.id != id && self.usable_air_base(owner, p))
            .map(|p| ((p.pos - pos).xz().length_sq_raw(), p.id))
            .collect();
        pads.sort_unstable();
        let pads: Vec<UnitId> = pads.into_iter().map(|(_, p)| p).collect();
        let Some((deck, holder)) = self.free_deck(id, owner, &pads, true) else { return };
        self.take_deck(id, deck, holder);
        if let Some(u) = self.unit_mut(id) {
            u.repair_trip = true;
            // (Off its attack run and its way: to the deck from where it is.)
            u.air_pass = false;
            u.pass_runout = false;
            u.path.clear();
            u.path_goal = None;
            u.move_request = None;
        }
    }

    /// A repair trip ends once the aircraft is mended, or its deck is lost (destroyed, taken,
    /// switched off): it gives the deck up, takes off and carries on with the orders it held.
    /// Whether it was landed (it takes off).
    fn end_repair_trip(&mut self, id: UnitId, t: &UnitType) -> bool {
        let Some(u) = self.unit(id).filter(|u| u.repair_trip) else { return false };
        let lost = u.air_pad.is_none_or(|deck| self.air_pad_point(u.owner, deck).is_none());
        if !lost && u.health < t.max_health {
            return false;
        }
        let Some(u) = self.unit_mut(id) else { return false };
        u.repair_trip = false;
        u.air_pad = None;
        u.path_goal = None;
        u.move_request = None;
        std::mem::take(&mut u.landed)
    }

    /// A parked aircraft rides a moving carrier after the ground-motion phase.
    pub(super) fn follow_air_pad(&mut self, id: UnitId) {
        let Some(u) = self.unit(id).filter(|u| !u.dead && u.landed) else { return };
        let Some(pad) = u.air_pad else { return };
        let Some(point) = self.air_pad_point(u.owner, pad) else { return };
        let heading = self.unit(pad.0).map_or(0, |p| p.heading);
        if let Some(u) = self.unit_mut(id) { u.pos = point; u.heading = heading; u.speed = Fx::ZERO; }
    }

    /// Aircraft flight: climb to cruise altitude, fly towards the goal (banking into
    /// turns), land when idle over land and take off again when given something to do.
    /// Attack passes keep full speed through the goal.
    /// Whether an aircraft of this type with nothing to do over the sea, where it cannot set
    /// down, circles round where it came to rest (as TA's planes do): any but a gunship
    /// (`HoverAttack`), which hovers where it is.
    pub(super) fn circles_at_sea(t: &UnitType) -> bool {
        t.locomotion == Locomotion::Air && !t.hover_attack
    }

    pub(super) fn update_air(&mut self, id: UnitId, t: &UnitType) {
        // The factory that released it, if still around: where its yard is.
        let yard = self.unit(id).and_then(|u| u.factory).map(|f| self.unit(f).map(|f| (f.pos, self.types.get(f.type_id).footprint)));
        // A repair trip ends (mended, or its deck lost), or one begins.
        let mut released = self.end_repair_trip(id, t);
        self.start_repair_trip(id, t);
        let Some(current) = self.unit(id) else { return };
        // (On a repair trip its orders wait, and what its weapons aim at does not keep it off its
        // deck: it flies there and lands as an aircraft with nothing to do does.)
        let trip = current.repair_trip;
        let external = !trip && (!current.orders.is_empty() || current.build.is_some() || !current.cargo.is_empty() || current.weapons.iter().any(|w| w.target.is_some()));
        let old_pad = current.air_pad;
        let lost_pad = old_pad.is_some_and(|pad| self.air_pad_point(current.owner, pad).is_none());
        if old_pad.is_some() && (external || lost_pad) && let Some(u) = self.unit_mut(id) {
            u.air_pad = None;
            released |= u.landed;
            u.landed = false;
            if lost_pad && u.orders.is_empty() { u.path_goal = None; u.move_request = None; }
        }
        let Some(current) = self.unit(id) else { return };
        let mut ground_landing = self.air_ground_height(t, current.pos);
        if current.landed && current.air_pad.is_none() && ground_landing.is_none() && let Some(u) = self.unit_mut(id) {
            u.landed = false;
            released = true;
        }
        let Some(current) = self.unit(id) else { return };
        if !external && !current.landed && current.air_pad.is_none() && current.path_goal.is_none() && current.idle_ticks >= IDLE_BEFORE_LANDING {
            let (pos, idle) = (current.pos, current.idle_ticks);
            if let Some((deck, holder)) = self.reserve_air_pad(id, t) {
                self.take_deck(id, deck, holder);
            } else {
                // Not on a place another aircraft has taken: landed there, flying there, or
                // coming down over it already (the lower first: the one coming down goes on, the
                // one that has just come is the one to go elsewhere; of two as low, the first).
                let first = |o: &Unit| (o.pos.y, o.id.slot) < (pos.y, id.slot);
                // (Over the sea an aircraft circles where it is, as TA's planes do, rather than
                // fly off to the nearest land: see `update_air`'s circling.)
                let circles_here = Self::circles_at_sea(t) && ground_landing.is_none() && self.world.depth_at(pos.x, pos.z) > Fx::ZERO;
                if ground_landing.is_some() && Self::spot_taken(&self.parked_near(id, pos, t.radius + self.types.target_reach + Fx::from_int(CELL), first), t, pos) {
                    ground_landing = None;
                }
                // Else the nearest place it may set down; none near, it stays up and looks
                // again a second later.
                if ground_landing.is_none() && !circles_here && (idle - IDLE_BEFORE_LANDING).is_multiple_of(LANDING_SEARCH_EVERY)
                    && let Some(spot) = self.nearby_air_landing(id, t) && let Some(u) = self.unit_mut(id) {
                    u.path_goal = Some(spot);
                    u.move_request = Some(spot);
                }
            }
        }
        let Some(current) = self.unit(id) else { return };
        let pad = current.air_pad.and_then(|pad| self.air_pad_point(current.owner, pad));
        // (An aircraft idle over a friendly carrier's deck waits there to set down on it, as one
        // over land waits for its place: it does not circle away from it.)
        let circles = Self::circles_at_sea(t);
        let over_airbase = circles && current.air_pad.is_none() && !external && self.types.airbase_reach > Fx::ZERO && {
            let mut near = Vec::new();
            self.grid.query(current.pos, self.types.airbase_reach, &mut near);
            near.into_iter().filter_map(|slot| self.units.get(slot as usize)?.as_ref()).any(|p| {
                p.id != id && self.usable_air_base(current.owner, p) && p.pos.within_xz(current.pos, self.unit_type(p).radius + Fx::from_int(24))
            })
        };
        // Flying to a place of its own to land on (or a deck): over the highest ground on its way
        // there, so that neither the flight nor the way down meets a hill (it climbs first,
        // standing).
        let parking = (!external).then_some(current.path_goal).flatten().map(|goal| Self::highest_on_way(&self.world, current.pos, goal));
        let aim = (!t.is_plane() && !trip).then(|| {
            let target = match current.orders.front() {
                Some(Order::Attack(t) | Order::ManualFire(t)) => Some(*t),
                _ => current.weapons.iter().find_map(|w| w.target),
            }?;
            self.target_point(target).filter(|_| self.target_known(target, current.owner))
        }).flatten();
        let world = &self.world;
        let Some(u) = self.units.get_mut(id.slot as usize).and_then(|u| u.as_mut()) else { return };
        if let Some(point) = pad {
            if u.landed { u.pos = point; }
            else {
                u.path_goal = (!u.pos.within_xz(point, Fx::from_int(4))).then_some(point);
                u.move_request = u.path_goal;
            }
        }
        let ground_h = world.height_at(u.pos.x, u.pos.z);
        let under = parking.map_or(ground_h, |way| way.max(ground_h));
        let landing_height = pad.map(|p| p.y).or(ground_landing);
        let can_land = landing_height.is_some();
        // (Its own cruising height, a staged scene's, or its type's.)
        let cruise_alt = u.cruise_alt.unwrap_or(t.cruise_alt);
        let cruise = under.max(world.sea_level) + cruise_alt;
        if u.air_pass && !matches!(u.orders.front(), Some(Order::Attack(_) | Order::ManualFire(_) | Order::Fight(_) | Order::Patrol(_))) {
            u.air_pass = false;
        }
        // Carrying a passenger keeps the aircraft in flight even after Move or Stop.
        // Landing would drag the unit hanging below it through the terrain. (On a repair trip
        // only its way to the deck keeps it up.)
        let busy = u.path_goal.is_some() || !trip && (!u.orders.is_empty() || u.build.is_some() || !u.cargo.is_empty() || u.weapons.iter().any(|w| w.target.is_some()));
        // A self-chosen parking approach is part of landing, not a new external
        // task that restarts the idle delay after reaching the clear footprint.
        u.idle_ticks = if external { 0 } else { u.idle_ticks.saturating_add(1) };
        // New aircraft already enter this path as not landed. Initialize their flight
        // callback too: waiting for a landed -> flying transition skipped Activate on
        // their first sortie, leaving folding wings shut until they landed once.
        let mut activate = if released { Some(true) } else { (u.move_rate == u8::MAX).then_some(!u.landed) };
        if u.landed && busy {
            u.landed = false;
            activate = Some(true);
        }
        let landing = !external && can_land && (pad.is_some() || !busy && u.idle_ticks > IDLE_BEFORE_LANDING);
        // Horizontal flight only once clear of the ground (take-off climbs first; on its way to
        // land, clear of all the ground on the way).
        let airborne = u.pos.y >= under + (cruise_alt / 3).min(Fx::from_int(30));
        // Going to its place to land (on the ground or a deck), a plane moves as it hovers when
        // idle: a place nearer than its turning circle it would circle for ever.
        let plane = t.is_plane() && (external || u.path_goal.is_none());
        match u.path_goal {
            Some(goal) if !u.landed => {
                let to = goal - u.pos;
                let dist = to.length_xz();
                if dist <= u.speed.max(Fx::from_int(4)) && !u.air_pass {
                    // Parking needs the exact clear footprint/deck position. Merely
                    // clearing a nearby goal can leave a wing over the blocking cell.
                    // A VTOL sent there stops there too, hovering: drifting on along its
                    // heading, a transport over the place to set its cargo down swung back
                    // over it and out again, wider than the 10 px it lets go within, and
                    // circled it for ever with the cargo still hanging under it.
                    if !external || !plane {
                        u.pos.x = goal.x;
                        u.pos.z = goal.z;
                        u.speed = Fx::ZERO;
                    }
                    u.path_goal = None;
                } else {
                    let desired = heading_of(to);
                    let diff = wrap_angle(desired - u.heading);
                    u.heading = wrap_angle(u.heading + diff.clamp(-t.turn_rate, t.turn_rate));
                    let cruise_speed = if u.air_pass {
                        t.max_speed
                    } else {
                        let stop_speed = (Fx::from_int(2) * t.brake * dist).sqrt();
                        // Hovering aircraft slow down to turn tighter rather than circling a
                        // goal their turn radius can't reach (radius = speed / turn rate).
                        let turn_rad = Fx::from_int(t.turn_rate.max(1) as i64) * 6 / 65536;
                        let turn_speed = if diff.abs() > 4096 && plane { (turn_rad * dist / 2).max(Fx::from_ratio(1, 2)) } else { t.max_speed };
                        t.max_speed.min(stop_speed.max(Fx::from_ratio(1, 2))).min(turn_speed)
                    };
                    let target = if airborne { cruise_speed } else { Fx::ZERO };
                    // (A VTOL slows down at its brake rate, as `stop_speed` reckons it does:
                    // slowing at its acceleration, a transport (CORVALK: 0.02 a tick, braking 4)
                    // came over its goal at full speed.)
                    let rate = if !plane && target < u.speed { t.brake } else { t.acceleration };
                    u.speed = u.speed.approach(target, rate);
                    // Bank into turns.
                    u.roll = (diff.clamp(-8192, 8192) * 3) / 4;
                }
                if !u.moving {
                    u.moving = true;
                    if let Some(c) = &mut u.cob {
                        c.start("StartMoving", &[], ThreadTag::None);
                    }
                }
            }
            // An aircraft with nothing to do over the sea, where it cannot set down, circles round
            // where it came to rest, as TA's planes do, rather than hang there (several sent to
            // one place hung there on top of each other); a gunship hovers. (Over land it waits
            // for a place to land, as below.)
            _ if circles && !u.landed && !busy && !can_land && !over_airbase && airborne && world.depth_at(u.pos.x, u.pos.z) > Fx::ZERO => {
                let (speed, radius) = if t.is_plane() {
                    let speed = t.max_speed * 3 / 4;
                    // (A little wider than it can turn at that speed: radius = speed / turn rate.)
                    (speed, (speed * 10430 / t.turn_rate.max(1) as i64 * 5 / 4).max(Fx::from_int(64)))
                } else {
                    // A hovering one (a scout, a builder, a transport) loiters round a small
                    // circle, its own size of it, as fast as it turns round that (at most three
                    // quarters of its top speed): a slow-turning builder slowly, not round a circle
                    // hundreds of px across at speed.
                    let radius = Fx::from_int(LOITER_RADIUS + (id.slot % 4) as i64 * LOITER_STEP);
                    let fastest = (t.max_speed * 3 / 4).max(Fx::from_ratio(1, 4));
                    (((radius * t.turn_rate.max(1) as i64) / 10430 * 4 / 5).clamp(Fx::from_ratio(1, 4), fastest), radius)
                };
                // (Its circle on the map: the map's edge would hold it there.)
                let (w, h) = world.size_px();
                let inside = |v: Fx, size: Fx| if size > radius * 2 { v.clamp(radius, size - radius) } else { size / 2 };
                // (A hovering one's circle lies half its radius to a side of where it came to rest,
                // the side its slot's, golden angles apart: two that came to rest together, of a
                // size, do not share one.)
                let rest = if t.is_plane() { u.pos } else { u.pos + heading_vector(wrap_angle(id.slot as i32 * LOITER_SIDE)).scale(radius / 2) };
                let center = *u.orbit.get_or_insert(FxVec3::new(inside(rest.x, w), u.pos.y, inside(rest.z, h)));
                let to = (center - u.pos).xz();
                let dist = to.length_xz();
                let desired = if dist < Fx::ONE {
                    wrap_angle(u.heading + 16384)
                } else {
                    // Along the circle (anticlockwise seen from above), drawn back onto it.
                    let n = dist.to_int().max(1);
                    let (inward, along) = (FxVec3::new(to.x / n, Fx::ZERO, to.z / n), FxVec3::new(-to.z / n, Fx::ZERO, to.x / n));
                    let pull = ((dist - radius) / radius).clamp(-Fx::ONE, Fx::ONE);
                    heading_of(along + inward.scale(pull))
                };
                let diff = wrap_angle(desired - u.heading);
                u.heading = wrap_angle(u.heading + diff.clamp(-t.turn_rate, t.turn_rate));
                u.speed = u.speed.approach(speed, t.acceleration);
                u.roll = (diff.clamp(-8192, 8192) * 3) / 4;
                if !u.moving {
                    u.moving = true;
                    if let Some(c) = &mut u.cob {
                        c.start("StartMoving", &[], ThreadTag::None);
                    }
                }
            }
            _ => {
                u.speed = u.speed.approach(Fx::ZERO, t.brake);
                u.roll = u.roll * 7 / 8;
                if let Some(point) = aim {
                    let diff = wrap_angle(heading_of(point - u.pos) - u.heading);
                    u.heading = wrap_angle(u.heading + diff.clamp(-t.turn_rate, t.turn_rate));
                }
                if u.speed.is_zero() && u.moving {
                    u.moving = false;
                    if let Some(c) = &mut u.cob {
                        c.start("StopMoving", &[], ThreadTag::None);
                    }
                }
            }
        }
        // (Circling only while it has nothing to do and nowhere to set down.)
        if busy || u.landed || can_land {
            u.orbit = None;
        }
        // Planes fly where they point; VTOLs (transports, builders, gunships) slide
        // straight towards their goal while turning to face it (planes too, to their place to
        // land).
        let vtol = !plane;
        let dir = match (vtol, u.path_goal) {
            (true, Some(goal)) => {
                let to = (goal - u.pos).xz();
                let len = to.length_xz();
                if len > Fx::ONE { FxVec3::new(to.x / len.to_int().max(1), Fx::ZERO, to.z / len.to_int().max(1)) } else { FxVec3::ZERO }
            }
            _ => heading_vector(u.heading),
        };
        let step = if vtol && let Some(goal) = u.path_goal {
            let to = (goal - u.pos).xz();
            let len = to.length_xz();
            if len <= u.speed { to } else { FxVec3::new(to.x * u.speed / len, Fx::ZERO, to.z * u.speed / len) }
        } else {
            dir.scale(u.speed)
        };
        u.pos.x += step.x;
        u.pos.z += step.z;
        // (A transport's height to pick up or set down at holds while it does so: one whose
        // order was replaced first flew on at it, through the hills.)
        let hover_alt = u.hover_alt.filter(|_| matches!(u.orders.front(), Some(Order::Load(_) | Order::Unload(_))));
        let over_pad = pad.is_none_or(|p| u.pos.within_xz(p, Fx::from_int(4)));
        let target_y = if u.landed || landing && over_pad { landing_height.unwrap_or(ground_h) } else { hover_alt.unwrap_or(cruise) };
        // Climbing at 2 px a tick, it rises with the ground at the least: a transport off a
        // beach flew into the cliff behind it, under the ground until it had climbed out.
        u.pos.y = u.pos.y.approach(target_y, Fx::from_int(2)).max(world.height_at(u.pos.x, u.pos.z));
        if landing && over_pad && !u.landed && u.pos.y <= target_y + Fx::ONE && u.speed.is_zero() {
            u.landed = true;
            u.roll = 0;
            activate = Some(false);
        }
        let (w, h) = world.size_px();
        u.pos.x = u.pos.x.clamp(Fx::ZERO, w);
        u.pos.z = u.pos.z.clamp(Fx::ZERO, h);
        // Leaving the factory: once clear of its yard it no longer holds up the next unit
        // (as ground units do in `ground_step`).
        if let Some(yard) = yard && yard.is_none_or(|(p, fp)| !u.pos.within_xz(p, Fx::from_int(fp.0.max(fp.1) as i64 * CELL / 2) + t.radius)) {
            u.factory = None;
        }
        // Wings and jets follow the speed (TA's MoveRate0..3: a third of top speed each,
        // planes topping out at 2; taking off or landing counts as 1).
        let rate = if u.landed {
            0
        } else if !airborne {
            1
        } else {
            let top = if t.is_plane() { 2 } else { 3 };
            (u.speed * 3 / t.max_speed.max(Fx::from_ratio(1, 100))).to_int().clamp(0, top) as u8
        };
        let new_rate = (rate != u.move_rate).then_some(rate);
        if new_rate.is_some() {
            u.move_rate = rate;
        }
        // Aircraft scripts open/close wings and gear on Activate/Deactivate.
        if let Some(on) = activate {
            self.set_active(id, on);
        }
        if let Some(r) = new_rate.filter(|&r| t.callins.move_rate[r as usize]) {
            self.start_script(id, ["MoveRate0", "MoveRate1", "MoveRate2", "MoveRate3"][r as usize], &[]);
        }
    }

}
