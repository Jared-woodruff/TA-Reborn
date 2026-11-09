//! Formations, the rule "modern movement" (`Sim::group_formations`): a group sent somewhere
//! together goes as one body, as in Supreme Commander.
//!
//! - **Layout.** A group order lays its units out in rows ([`rows`]): the short-ranged in
//!   front; the long-ranged, the anti-aircraft, the builders and Commanders behind; each row
//!   spaced by its units' size (their models' reach, barrels and all), each unit keeping the
//!   order across it stands in so that their ways don't cross. Each unit's order goes to its own
//!   place there (`Sim::form`).
//! - **Forming up.** Sent a good way off, the group first takes its formation in front of where
//!   it stands (each unit at its place there, facing the way they go), then marches.
//! - **Marching.** Each unit's way to its place (planned together, from the group's shared field,
//!   leaving it as early as can be: from places side by side to places side by side the ways run
//!   side by side) is paced so that they all get there at once: the slowest one sets the time,
//!   and the rest keep level with it (`Sim::pace_formations`). Those that fight, get stuck or
//!   fall far behind are let go at their own pace; an attack-move with a good part of it
//!   fighting isn't paced at all.
//!
//! All of it is fixed point and ordered by slot: the same on every machine.

use crate::defs::{Locomotion, UnitType, WeaponTypes};
use crate::fixed::{Fx, FxVec3, heading_of, heading_vector};
use crate::sim::Sim;
use crate::unit::{Order, Unit, UnitId};
use crate::world::World;

/// Room a unit takes across a row and along the formation: its size (the larger of its
/// footprint's and its model's reach) this many times across, and [`GAP`] more.
const PITCH: Fx = Fx::from_ratio(9, 4);
const GAP: i64 = 12;
/// Rows about this many times as wide as the formation is deep (as a fraction).
const ASPECT: (usize, usize) = (5, 2);
/// Closer than this (px) to where it is sent, a group keeps the way it faces.
const NEAR: i64 = 32;

/// Where a unit stands in a formation: across its front (px right of the middle) and along it
/// (px ahead of the middle).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Place {
    pub across: Fx,
    pub along: Fx,
}

/// Where in a formation a kind of unit goes, the front first: its class (fighters, then
/// anti-aircraft, then builders and Commanders, then the rest), its reach on the ground in steps
/// of 32 px (the short-ranged in front, the artillery behind), and the toughest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rank {
    class: u8,
    reach: i64,
    frailty: i64,
}

/// A kind of unit's [`Rank`].
pub fn rank(t: &UnitType, weapons: &WeaponTypes) -> Rank {
    let (mut ground, mut air) = (Fx::ZERO, false);
    for w in t.weapons.iter().flatten().map(|&w| weapons.get(w)).filter(|w| !w.command_fire) {
        if w.hits_ground {
            ground = ground.max(w.range);
        } else if w.hits_air {
            air = true;
        }
    }
    let class = if t.is_builder {
        2
    } else if ground > Fx::ZERO {
        0
    } else if air {
        1
    } else {
        3
    };
    Rank { class, reach: (ground / 32).to_int(), frailty: -t.max_health.to_int() }
}

/// A unit's size in a formation (px): the larger of its footprint's radius and its model's reach.
pub fn size(t: &UnitType) -> Fx {
    t.radius.max(t.model.reach)
}

/// How a group order lays its units out.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Shape {
    /// In rows by [`Rank`], facing the way they go (or `facing`, TA angle units, when given).
    Rows { facing: Option<i32> },
    /// As they stand round their middle, turned to face the way they go.
    Keep,
    /// At these points (a line the player drew), the units taking them in order across.
    Spots(#[serde(deserialize_with = "crate::serde_limits::command_list")] Vec<FxVec3>),
}

/// A unit as the layout sees it: where it starts from (where it stands, or the end of the
/// orders queued before), its size and its rank.
#[derive(Clone, Copy, Debug)]
pub struct Member {
    pub id: UnitId,
    pub from: FxVec3,
    pub size: Fx,
    pub rank: Rank,
}

/// The middle of some points (on the ground plane).
pub fn centre(points: impl Iterator<Item = FxVec3>) -> FxVec3 {
    let (mut x, mut z, mut n) = (Fx::ZERO, Fx::ZERO, 0i64);
    for p in points {
        x += p.x;
        z += p.z;
        n += 1;
    }
    if n == 0 { FxVec3::ZERO } else { FxVec3::new(x / n, Fx::ZERO, z / n) }
}

/// The way a group at `from` faces going to `to`; `otherwise` when it is already there.
pub fn facing(from: FxVec3, to: FxVec3, otherwise: i32) -> i32 {
    let d = (to - from).xz();
    if d.within_xz(FxVec3::ZERO, Fx::from_int(NEAR)) { otherwise } else { heading_of(d) }
}

/// Ahead and to the right for a facing (heading 0 faces +z, whose right is -x).
pub fn frame(facing: i32) -> (FxVec3, FxVec3) {
    let ahead = heading_vector(facing);
    (ahead, FxVec3::new(-ahead.z, Fx::ZERO, ahead.x))
}

/// Where `place` is on the ground, for a formation whose middle is at `middle` facing `facing`.
pub fn spot(middle: FxVec3, facing: i32, place: Place) -> FxVec3 {
    let (ahead, right) = frame(facing);
    middle + right.scale(place.across) + ahead.scale(place.along)
}

/// How far each member stands ahead of the members' middle and to the right of it, for a
/// formation facing `facing`.
fn as_they_stand(members: &[Member], facing: i32) -> Vec<Place> {
    let (ahead, right) = frame(facing);
    let middle = centre(members.iter().map(|m| m.from));
    members
        .iter()
        .map(|m| {
            let d = (m.from - middle).xz();
            Place { across: d.dot(right), along: d.dot(ahead) }
        })
        .collect()
}

/// Units across a row for `n` units: rows about [`ASPECT`] times as wide as they are deep.
fn columns(n: usize) -> usize {
    let want = n * ASPECT.0;
    let mut c = 1;
    while c * c * ASPECT.1 < want {
        c += 1;
    }
    c.min(n).max(1)
}

/// The room a unit of `size` takes across a row and along the formation (px).
pub fn room(size: Fx) -> Fx {
    size * 2 * PITCH / 2 + Fx::from_int(GAP)
}

/// The places of `members` (in their order) in rows facing `facing`, the formation's middle at
/// 0. Front to back by [`Rank`]; among those of a rank, those already ahead take the front
/// rows, and each row keeps the order its units stand in from left to right, so that the ways
/// to their places cross as little as can be.
pub fn rows(members: &[Member], facing: i32) -> Vec<Place> {
    let n = members.len();
    let now = as_they_stand(members, facing);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| members[a].rank.cmp(&members[b].rank).then(now[b].along.cmp(&now[a].along)).then(members[a].id.slot.cmp(&members[b].id.slot)));
    let rows: Vec<Vec<usize>> = order
        .chunks(columns(n))
        .map(|row| {
            let mut row = row.to_vec();
            row.sort_by(|&a, &b| now[a].across.cmp(&now[b].across).then(members[a].id.slot.cmp(&members[b].id.slot)));
            row
        })
        .collect();
    let depths: Vec<Fx> = rows.iter().map(|row| row.iter().map(|&i| room(members[i].size)).max().unwrap_or(Fx::ZERO)).collect();
    let mut ahead = depths.iter().fold(Fx::ZERO, |a, &d| a + d) / 2;
    let mut places = vec![Place::default(); n];
    for (row, &depth) in rows.iter().zip(&depths) {
        let along = ahead - depth / 2;
        ahead -= depth;
        let width = row.iter().fold(Fx::ZERO, |a, &i| a + room(members[i].size));
        let mut left = -width / 2;
        for &i in row {
            let w = room(members[i].size);
            places[i] = Place { across: left + w / 2, along };
            left += w;
        }
    }
    places
}

/// The places of `members` keeping the shape they stand in, turned to face `facing`.
pub fn kept(members: &[Member], facing: i32) -> Vec<Place> {
    as_they_stand(members, facing)
}

/// The places of `members` at `points` (their middle the formation's), facing `facing`: the
/// units in order across take the points in order across. Points beyond the members are left
/// over; members beyond the points stand at the middle.
pub fn at_points(members: &[Member], points: &[FxVec3], facing: i32) -> Vec<Place> {
    let (ahead, right) = frame(facing);
    let middle = centre(points.iter().copied());
    let now = as_they_stand(members, facing);
    let mut units: Vec<usize> = (0..members.len()).collect();
    units.sort_by(|&a, &b| now[a].across.cmp(&now[b].across).then(members[a].id.slot.cmp(&members[b].id.slot)));
    let mut spots: Vec<Place> = points
        .iter()
        .map(|&p| {
            let d = (p - middle).xz();
            Place { across: d.dot(right), along: d.dot(ahead) }
        })
        .collect();
    spots.sort_by(|a, b| a.across.cmp(&b.across).then(a.along.cmp(&b.along)));
    let mut places = vec![Place::default(); members.len()];
    for (&i, &p) in units.iter().zip(&spots) {
        places[i] = p;
    }
    places
}

/// The places of `members` for `shape` facing `facing`.
pub fn layout(members: &[Member], shape: &Shape, facing: i32) -> Vec<Place> {
    match shape {
        Shape::Rows { .. } => rows(members, facing),
        Shape::Keep => kept(members, facing),
        Shape::Spots(points) => at_points(members, points, facing),
    }
}

/// Passes of [`uncross`] at most.
const UNCROSS_PASSES: usize = 8;

/// Whether the ways `a`→`b` and `c`→`d` cross (on the ground plane; touching doesn't count).
fn ways_cross(a: FxVec3, b: FxVec3, c: FxVec3, d: FxVec3) -> bool {
    let side = |p: FxVec3, q: FxVec3, r: FxVec3| -> i128 { (q.x - p.x).0 as i128 * (r.z - p.z).0 as i128 - (q.z - p.z).0 as i128 * (r.x - p.x).0 as i128 };
    let (s1, s2) = (side(a, b, c).signum(), side(a, b, d).signum());
    let (s3, s4) = (side(c, d, a).signum(), side(c, d, b).signum());
    s1 * s2 < 0 && s3 * s4 < 0
}

/// `places` (of `members`, for a formation whose middle is at `middle` facing `facing`) with
/// those whose ways there cross swapped, two of a kind (a rank and a size) at a time, until none do
/// (or [`UNCROSS_PASSES`] passes): sorted across and along, units that stood level, a column
/// becoming a row as the group turns, took their places in any order and crossed each other's
/// way. (Uncrossed, the ways are shorter: each swap shortens them.)
pub fn uncross(members: &[Member], places: &mut [Place], middle: FxVec3, facing: i32, any: bool) {
    let n = members.len().min(places.len());
    for _ in 0..UNCROSS_PASSES {
        let mut swapped = false;
        for i in 0..n {
            for j in i + 1..n {
                if !(any || members[i].rank == members[j].rank && members[i].size == members[j].size) {
                    continue;
                }
                let (a, b) = (spot(middle, facing, places[i]), spot(middle, facing, places[j]));
                if ways_cross(members[i].from, a, members[j].from, b) {
                    places.swap(i, j);
                    swapped = true;
                }
            }
        }
        if !swapped {
            break;
        }
    }
}

/// Groups bigger than this keep the shape they stand in (rows of thousands would reach across
/// the map).
pub const ROWS_MAX: usize = 400;
/// A group forms up only when sent further than twice its formation's depth and this (px),
/// and with at least this share of its places in front of it standing room (as a fraction).
const FORM_TRIP: i64 = 320;
const FORM_ROOM: (usize, usize) = (4, 5);
/// Forming up ends with this share of the group at its places there (as a fraction; the rest
/// catch up on the march), or after the longest walk to one at full speed and this (ticks).
const FORMED: (usize, usize) = (3, 4);
const FORM_GRACE: u64 = 30;
/// The time a march takes: its slowest unit's, and this share more (spare speed for the others
/// to make up what their ways cost them); put back only when a unit would be later than
/// `LATE` ticks at full speed, and never by one later than `LOST` (it is let go).
const SLACK: Fx = Fx::from_ratio(11, 10);
const LATE: i64 = 15;
const LOST: i64 = 90;
/// A unit that hasn't come nearer its place for this long (ticks) is let go.
const STILL: u32 = 60;
/// An attack-move with more than one in this many fighting isn't paced: it fights.
const FIGHTING: usize = 3;

/// What a formation is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Taking its formation in front of where it stood, until every unit is at its place
    /// there, or this tick.
    Forming { until: u64 },
    /// On its way, to get there at this tick (set once their ways are planned).
    Marching { due: Option<u64> },
}

/// A unit of a formation.
#[derive(Clone, Debug)]
pub struct Mate {
    pub id: UnitId,
    pub place: Place,
    /// Where it ends: its place where the group was sent, where its order goes.
    pub spot: FxVec3,
    /// Where it forms up, while the formation does.
    pub form: Option<FxVec3>,
    /// Let go at its own pace: it fought, got stuck, or fell far behind.
    pub out: bool,
    /// The least way it had left to its place, and the ticks since it came nearer.
    best: Fx,
    still: u32,
}

/// A group sent somewhere together (`Sim::form`).
#[derive(Clone, Debug)]
pub struct Formation {
    pub id: u32,
    pub owner: u8,
    /// Where it was sent (its middle there) and the way it faces there.
    pub target: FxVec3,
    pub facing: i32,
    /// An attack-move: while a good part of it fights, it isn't paced.
    pub fight: bool,
    pub mates: Vec<Mate>,
    pub phase: Phase,
}

impl Formation {
    /// The schedule and each member's retained progress affect future pacing.
    pub(crate) fn hash_state(&self, mix: &mut impl FnMut(i64)) {
        let point = |mix: &mut dyn FnMut(i64), p: FxVec3| { mix(p.x.0); mix(p.y.0); mix(p.z.0); };
        mix(self.id as i64);
        mix(self.owner as i64);
        point(mix, self.target);
        mix(self.facing as i64);
        mix(self.fight as i64);
        match self.phase {
            Phase::Forming { until } => { mix(0); mix(until as i64); }
            Phase::Marching { due } => { mix(1); mix(due.map_or(-1, |t| t as i64)); }
        }
        mix(self.mates.len() as i64);
        for m in &self.mates {
            mix(m.id.slot as i64); mix(m.id.generation as i64);
            mix(m.place.across.0); mix(m.place.along.0);
            point(mix, m.spot);
            mix(m.form.is_some() as i64);
            if let Some(p) = m.form { point(mix, p); }
            mix(m.out as i64); mix(m.best.0); mix(m.still as i64);
        }
    }
}

/// The point an order sends a unit to (its formation's place goes there).
pub fn order_point(o: &Order) -> Option<FxVec3> {
    match o {
        Order::Move(p) | Order::Fight(p) | Order::Patrol(p) => Some(*p),
        _ => None,
    }
}

/// The same order to another point.
fn order_to(o: &Order, p: FxVec3) -> Order {
    match o {
        Order::Move(_) => Order::Move(p),
        Order::Fight(_) => Order::Fight(p),
        Order::Patrol(_) => Order::Patrol(p),
        other => other.clone(),
    }
}

/// Whether a unit marches in formations: finished ground, hover and naval units.
fn marches(u: &Unit, t: &UnitType) -> bool {
    u.is_complete() && !u.dead && u.carried_by.is_none() && matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval)
}

/// The formation a unit marches in on its way to `dest` (its order's point), if any.
fn formation_for(u: &Unit, dest: FxVec3) -> Option<u32> {
    u.formations.iter().find(|&&(_, s)| s == dest).map(|&(f, _)| f)
}

/// The way a unit has left to go (px): along its route to its goal.
fn way_left(u: &Unit, t: &UnitType) -> Fx {
    let Some(goal) = u.path_goal else { return Fx::ZERO };
    let mut at = u.pos;
    let mut left = Fx::ZERO;
    for &c in u.path.iter().take(u.path.len().saturating_sub(1)) {
        let p = crate::sim::anchor_middle(c, t.footprint);
        left += at.distance_xz(p);
        at = p;
    }
    left + at.distance_xz(goal)
}

/// A group order laid out: each unit's place and where that is, the way the group faces, and
/// the units that go as they are told (not ground, hover or naval ones).
pub struct Plan {
    pub facing: i32,
    pub places: Vec<(UnitId, Place, FxVec3)>,
    pub others: Vec<UnitId>,
}

/// Cells already taken by places, each unit's whole footprint.
type Taken = rustc_hash::FxHashSet<(i32, i32)>;

impl Sim {
    /// Where `player`'s `units` would each go for an order to `target` laid out as `shape`
    /// (`queue`: from the ends of their orders): what a group order does with them, and what
    /// the interface shows while one is given. Places on ground a unit can't stand on, out of
    /// its reach, taken already or where another of the player's units stands go to the
    /// nearest cell that will do (six cells off at most).
    pub fn formation_plan(&self, player: u8, units: &[UnitId], target: FxVec3, shape: &Shape, queue: bool) -> Plan {
        let types = &self.types;
        let mut members: Vec<Member> = Vec::new();
        let mut others = Vec::new();
        // (The group's slots, looked up as each unit and each place's neighbours are: in a
        // list, thousands of units took millions of comparisons.)
        let mut seen: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for &id in units {
            let Some(u) = self.unit(id).filter(|u| u.owner == player && u.is_complete()) else { continue };
            // Reserve two slots (a new patrol also records its implicit start). Do not
            // retain formation metadata for a waypoint the unit cannot enqueue.
            if queue && (u.orders.len() > crate::sim::ORDER_QUEUE_MAX - 2 || u.formations.len() >= crate::sim::ORDER_QUEUE_MAX) {
                continue;
            }
            if !seen.insert(id.slot) {
                continue;
            }
            let t = types.get(u.type_id);
            if !marches(u, t) {
                others.push(id);
                continue;
            }
            let from = if queue { u.orders.iter().rev().find_map(order_point).unwrap_or(u.pos) } else { u.pos };
            members.push(Member { id, from, size: size(t), rank: rank(t, &self.weapon_types) });
        }
        if members.len() < 2 {
            others.extend(members.iter().map(|m| m.id));
            return Plan { facing: 0, places: Vec::new(), others };
        }
        let keep = Shape::Keep;
        let shape = if members.len() > ROWS_MAX && matches!(shape, Shape::Rows { facing: None }) { &keep } else { shape };
        let middle = centre(members.iter().map(|m| m.from));
        let otherwise = self.unit(members[0].id).map_or(0, |u| u.heading);
        let facing = match shape {
            Shape::Rows { facing: Some(f) } => *f,
            Shape::Spots(points) => facing(middle, centre(points.iter().copied()), otherwise),
            _ => facing(middle, target, otherwise),
        };
        let target = match shape {
            Shape::Spots(points) => centre(points.iter().copied()),
            _ => target,
        };
        let mut places = layout(&members, shape, facing);
        // (Every pair, up to eight times: past `ROWS_MAX`, a line drawn for thousands of units
        // held up the tick it was given in.)
        if !matches!(shape, Shape::Keep) && members.len() <= ROWS_MAX {
            uncross(&members, &mut places, target, facing, matches!(shape, Shape::Spots(_)));
        }
        let seen: Vec<u32> = seen.into_iter().collect();
        let mut taken = Taken::default();
        let (w, h) = self.world.size_px();
        let edge = Fx::from_int(16);
        let on_map = |p: FxVec3| FxVec3::new(p.x.clamp(edge, w - edge), Fx::ZERO, p.z.clamp(edge, h - edge));
        let places = members
            .iter()
            .zip(places)
            .map(|(m, place)| {
                let u = self.unit(m.id).unwrap();
                let want = on_map(spot(target, facing, place));
                let at = self.free_spot(u, want, &seen, &mut taken).unwrap_or(want);
                (m.id, place, at)
            })
            .collect();
        Plan { facing, places, others }
    }

    /// The nearest place to `want` that unit `u` can stand on, in its region, its footprint on
    /// no cell `taken` (which it then takes) and clear of the player's units standing there
    /// (but those of `ours`, the slots of the group, in order), six cells off at most.
    fn free_spot(&self, u: &Unit, want: FxVec3, ours: &[u32], taken: &mut Taken) -> Option<FxVec3> {
        let t = self.types.get(u.type_id);
        let spec = self.move_specs[t.id];
        let region = self.world.component(&spec, World::pos_to_anchor(u.pos, t.footprint));
        let cell = World::pos_to_anchor(want, t.footprint);
        let (fx, fz) = (t.footprint.0.max(1) as i32, t.footprint.1.max(1) as i32);
        let cells = |c: (i32, i32)| (0..fz).flat_map(move |dz| (0..fx).map(move |dx| (c.0 + dx, c.1 + dz)));
        let mut near = Vec::new();
        let standing = |at: FxVec3, near: &mut Vec<u32>| {
            self.grid.query(at, t.radius + Fx::from_int(48), near);
            near.iter().any(|&s| {
                ours.binary_search(&s).is_err()
                    && self.units.get(s as usize).and_then(|o| o.as_ref()).is_some_and(|o| {
                        o.owner == u.owner && o.path_goal.is_none() && !o.dead && o.is_complete() && o.pos.distance_xz(at) < t.radius + self.types.get(o.type_id).radius
                    })
            })
        };
        for r in 0..=6i32 {
            let mut ring: Vec<(i32, i32)> = (-r..=r).flat_map(|dz| (-r..=r).map(move |dx| (dx, dz))).filter(|&(dx, dz)| dx.abs().max(dz.abs()) == r).collect();
            ring.sort_by_key(|&(dx, dz)| (dx * dx + dz * dz, dz, dx));
            for (dx, dz) in ring {
                let c = (cell.0 + dx, cell.1 + dz);
                if cells(c).any(|k| taken.contains(&k)) || !self.world.passable(&spec, c, None) || region != 0 && self.world.component(&spec, c) != region {
                    continue;
                }
                let at = if r == 0 { FxVec3::new(want.x, Fx::ZERO, want.z) } else { self.world.anchor_to_pos(c, t.footprint) };
                if standing(at, &mut near) {
                    continue;
                }
                taken.extend(cells(c));
                return Some(at);
            }
        }
        None
    }

    /// A group order as a formation (`Command::Order` with several units under the rule, or
    /// `Command::Formation`): each ground, hover and naval unit's order goes to its own place,
    /// and they march there together, forming up first when sent a good way off (patrols just go
    /// to their places); the rest are told as they are.
    pub(crate) fn form(&mut self, player: u8, units: &[UnitId], order: &Order, queue: bool, shape: &Shape) {
        let Some(target) = order_point(order) else { return };
        let plan = self.formation_plan(player, units, target, shape, queue);
        for &id in &plan.others {
            self.order_unit(player, id, order, queue);
        }
        if plan.places.is_empty() {
            return;
        }
        let marching = !matches!(order, Order::Patrol(_));
        let id = self.next_formation;
        // Forming up: the formation set down in front of the crowd (its front a row ahead of the
        // foremost), when the group is sent a good way off and stands in no shape of its own.
        let forms = if marching && !queue && !matches!(shape, Shape::Keep) { self.form_up(&plan) } else { None };
        let mut mates = Vec::with_capacity(plan.places.len());
        for (k, &(unit, place, at)) in plan.places.iter().enumerate() {
            self.order_unit(player, unit, &order_to(order, at), queue);
            if !marching {
                continue;
            }
            let Some(u) = self.units[unit.slot as usize].as_mut().filter(|u| u.id == unit) else { continue };
            u.formations.push((id, at));
            mates.push(Mate { id: unit, place, spot: at, form: forms.as_ref().map(|f| f.0[k]), out: false, best: Fx::MAX, still: 0 });
        }
        if mates.len() >= 2 {
            self.next_formation += 1;
            let phase = match forms {
                Some((_, until)) => Phase::Forming { until },
                None => Phase::Marching { due: None },
            };
            self.formations.push(Formation { id, owner: player, target: centre(plan.places.iter().map(|p| p.2)), facing: plan.facing, fight: matches!(order, Order::Fight(_)), mates, phase });
        } else {
            for m in &mates {
                if let Some(u) = self.units[m.id.slot as usize].as_mut() {
                    u.formations.retain(|&(f, _)| f != id);
                }
            }
        }
    }

    /// Where each unit of a group laid out as `plan` forms up, and the tick forming up ends at
    /// the latest; None when it doesn't (sent close by, or no room in front of it).
    fn form_up(&self, plan: &Plan) -> Option<(Vec<FxVec3>, u64)> {
        let (ahead, _) = frame(plan.facing);
        let units: Vec<&Unit> = plan.places.iter().filter_map(|p| self.unit(p.0)).collect();
        let middle = centre(units.iter().map(|u| u.pos));
        let target = centre(plan.places.iter().map(|p| p.2));
        let (front, back) = plan.places.iter().fold((Fx::MIN, Fx::MAX), |(f, b), p| (f.max(p.1.along), b.min(p.1.along)));
        let row = units.iter().map(|u| room(size(self.types.get(u.type_id)))).max().unwrap_or(Fx::ZERO);
        let depth = front - back + row;
        if (target - middle).xz().length() <= depth * 2 + Fx::from_int(FORM_TRIP) {
            return None;
        }
        // Its front a row ahead of the foremost unit.
        let foremost = units.iter().map(|u| (u.pos - middle).xz().dot(ahead)).max().unwrap_or(Fx::ZERO);
        let at = middle + ahead.scale(foremost + row - front);
        let mut ours: Vec<u32> = plan.places.iter().map(|p| p.0.slot).collect();
        ours.sort_unstable();
        let mut taken = Taken::default();
        let mut room_for = 0;
        let mut walk = 0i64;
        let forms: Vec<FxVec3> = plan
            .places
            .iter()
            .zip(&units)
            .map(|(&(_, place, _), u)| {
                let want = spot(at, plan.facing, place);
                let p = self.free_spot(u, want, &ours, &mut taken);
                room_for += p.is_some() as usize;
                let p = p.unwrap_or(u.pos);
                let t = self.types.get(u.type_id);
                walk = walk.max((u.pos.distance_xz(p) / t.max_speed.max(Fx::from_ratio(1, 8))).to_int());
                p
            })
            .collect();
        (room_for * FORM_ROOM.1 >= forms.len() * FORM_ROOM.0).then_some((forms, self.tick + walk as u64 + FORM_GRACE))
    }

    /// Where unit `id` on its way to `dest` (its order's point) is to go now: its place in the
    /// formation forming up (true), or `dest`.
    pub(crate) fn formation_goal(&self, id: UnitId, dest: FxVec3) -> (FxVec3, bool) {
        let form = self.unit(id).and_then(|u| formation_for(u, dest)).and_then(|fid| self.formations.iter().find(|f| f.id == fid)).and_then(|f| match f.phase {
            Phase::Forming { .. } => f.mates.iter().find(|m| m.id == id).and_then(|m| m.form),
            Phase::Marching { .. } => None,
        });
        form.map_or((dest, false), |p| (p, true))
    }

    /// Whether unit `id` going to `dest` is in a formation: its way to its place leaves the
    /// group's shared field as early as it can (`FlowField::route_to_with`), and it comes all the
    /// way to its place (not a crowd's reach of it).
    pub(crate) fn in_formation(&self, id: UnitId, dest: FxVec3) -> bool {
        self.unit(id).is_some_and(|u| formation_for(u, dest).is_some())
    }

    /// Each formation: forming up, until every unit is at its place there (or the time is up);
    /// then marching, each unit's speed held to get it to its place when the rest get to theirs
    /// (`Sim::paces`, a cap on its speed by slot).
    pub(crate) fn pace_formations(&mut self) {
        self.paces.clear();
        if self.formations.is_empty() {
            return;
        }
        self.paces.resize(self.units.len(), None);
        let now = self.tick;
        let mut formations = std::mem::take(&mut self.formations);
        for f in formations.iter_mut() {
            match f.phase {
                Phase::Forming { until } => {
                    let (mut formed, mut on) = (0usize, 0usize);
                    for m in &f.mates {
                        let Some(u) = self.units.get(m.id.slot as usize).and_then(|u| u.as_ref()).filter(|u| u.id == m.id && !u.dead) else { continue };
                        let Some(form) = m.form.filter(|_| u.orders.front().and_then(order_point) == Some(m.spot)) else { continue };
                        on += 1;
                        formed += (u.move_request == Some(form) && u.path_goal.is_none() || u.pos.within_xz(form, Fx::from_int(8))) as usize;
                    }
                    if formed * FORMED.1 >= on * FORMED.0 || now >= until {
                        f.phase = Phase::Marching { due: None };
                        for m in f.mates.iter_mut() {
                            m.form = None;
                        }
                    }
                }
                Phase::Marching { due } => {
                    // Each unit on its way to its place: the way it has left, its top speed, and
                    // the time it needs (ticks: at full speed, getting up to it from rest).
                    let mut going: Vec<(usize, Fx, Fx, Fx, i64)> = Vec::new();
                    let (mut fighting, mut on) = (0usize, 0usize);
                    for m in f.mates.iter_mut() {
                        let Some(u) = self.units.get(m.id.slot as usize).and_then(|u| u.as_ref()).filter(|u| u.id == m.id && !u.dead) else { continue };
                        if u.orders.front().and_then(order_point) != Some(m.spot) {
                            continue;
                        }
                        on += 1;
                        if u.path_goal.is_none() {
                            if u.weapons.iter().any(|w| w.target.is_some()) {
                                fighting += 1;
                                m.out = true;
                            }
                            continue;
                        }
                        if u.move_request != Some(m.spot) {
                            continue;
                        }
                        let t = self.types.get(u.type_id);
                        let left = way_left(u, t);
                        if left + Fx::ONE <= m.best {
                            m.best = left;
                            m.still = 0;
                            // A temporary stall or fight is over. Rejoin the pace if
                            // its arrival is still within schedule (checked below),
                            // rather than running ahead for the rest of the march.
                            m.out = false;
                        } else {
                            m.still += 1;
                            m.out |= m.still >= STILL;
                        }
                        let top = t.max_speed.max(Fx::from_ratio(1, 16));
                        let start = top / (t.acceleration.max(Fx::from_ratio(1, 64)) * 2);
                        let need = (left / top + start).to_int() + 1;
                        if !m.out {
                            going.push((m.id.slot as usize, left, top, t.acceleration, need));
                        }
                    }
                    if going.is_empty() || f.fight && fighting * FIGHTING > on {
                        continue;
                    }
                    // The time to get there: the slowest's, set once; put back for one that would be
                    // late even at full speed (one hopelessly late is let go instead).
                    let slowest = going.iter().map(|g| g.4).max().unwrap_or(0);
                    let mut due = due.unwrap_or_else(|| now + (SLACK * slowest).to_int() as u64);
                    for g in &going {
                        let late = (now + g.4 as u64) as i64 - due as i64;
                        if late > LATE && late <= LOST {
                            due = now + (Fx::from_ratio(21, 20) * g.4).to_int() as u64;
                        }
                    }
                    f.phase = Phase::Marching { due: Some(due) };
                    let ticks = Fx::from_int((due.saturating_sub(now)).max(1) as i64);
                    for (slot, left, top, accel, need) in going {
                        if (now + need as u64) as i64 - due as i64 > LOST {
                            if let Some(m) = f.mates.iter_mut().find(|m| m.id.slot as usize == slot) {
                                m.out = true;
                            }
                            continue;
                        }
                        // Never slower than a quarter of its top speed: one held back still
                        // steers (a tank can't turn standing).
                        let floor = (top / 4).max(accel * 2).min(top);
                        self.paces[slot] = Some((left / ticks).clamp(floor, top));
                    }
                }
            }
        }
        self.formations = formations;
        if now.is_multiple_of(30) {
            self.prune_formations();
        }
    }

    /// Drops the units that have left their formations (arrived, told otherwise, dead) and the
    /// formations left with none.
    fn prune_formations(&mut self) {
        let units = &self.units;
        self.formations.retain_mut(|f| {
            f.mates.retain(|m| units.get(m.id.slot as usize).and_then(|u| u.as_ref()).is_some_and(|u| u.id == m.id && !u.dead && u.formations.iter().any(|&(fid, _)| fid == f.id)));
            !f.mates.is_empty()
        });
    }

    /// Unit `id`'s order to `dest` is done: it leaves the formation it marched there in, and,
    /// with nothing more to do, turns to face the way the formation faces.
    pub(crate) fn arrived(&mut self, id: UnitId, dest: FxVec3) {
        let Some(fid) = self.unit(id).and_then(|u| formation_for(u, dest)) else { return };
        let facing = self.formations.iter().find(|f| f.id == fid).map(|f| f.facing);
        if let Some(u) = self.unit_mut(id) {
            u.formations.retain(|&(f, s)| !(f == fid && s == dest));
            if u.orders.is_empty() {
                u.face = facing;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(slot: u32, x: i64, z: i64, size: i64, class: u8, reach: i64) -> Member {
        Member { id: UnitId { slot, generation: 0 }, from: FxVec3::from_ints(x, 0, z), size: Fx::from_int(size), rank: Rank { class, reach, frailty: 0 } }
    }

    /// Two places far enough apart for two units of these sizes, with room between them.
    fn apart(a: Place, b: Place, sa: i64, sb: i64) -> bool {
        let (dx, dz) = ((a.across - b.across).to_f64(), (a.along - b.along).to_f64());
        (dx * dx + dz * dz).sqrt() >= (sa + sb) as f64 + 8.0
    }

    #[test]
    fn rows_are_about_two_and_a_half_times_as_wide_as_deep() {
        assert_eq!([1, 2, 4, 10, 30, 100].map(columns), [1, 2, 4, 5, 9, 16]);
    }

    /// The short-ranged in front, artillery behind them, builders at the back; nobody on top
    /// of anybody else.
    #[test]
    fn rows_put_the_short_ranged_in_front_and_space_everyone() {
        // Facing +z (heading 0): ahead is +z, right is -x.
        let mut members = Vec::new();
        for k in 0..12 {
            let (class, reach) = match k % 4 {
                0 | 1 => (0, 6),
                2 => (0, 30),
                _ => (2, 0),
            };
            members.push(member(k, (k as i64 % 4) * 20, (k as i64 / 4) * 20, 16 + (k as i64 % 3) * 8, class, reach));
        }
        let places = rows(&members, 0);
        let along = |cls: u8, reach: i64| -> Vec<Fx> { members.iter().zip(&places).filter(|(m, _)| m.rank.class == cls && m.rank.reach == reach).map(|(_, p)| p.along).collect() };
        let fighters = along(0, 6);
        let artillery = along(0, 30);
        let builders = along(2, 0);
        assert!(fighters.iter().all(|f| artillery.iter().all(|a| a <= f)), "artillery behind the fighters");
        assert!(artillery.iter().all(|a| builders.iter().all(|b| b <= a)), "builders behind the artillery");
        for i in 0..members.len() {
            for j in i + 1..members.len() {
                assert!(apart(places[i], places[j], members[i].size.to_int(), members[j].size.to_int()), "{i} and {j} too close: {:?} {:?}", places[i], places[j]);
            }
        }
        // Centred on the middle.
        let mid = centre(places.iter().map(|p| FxVec3::new(p.across, Fx::ZERO, p.along)));
        assert!(mid.x.abs() < Fx::from_int(40) && mid.z.abs() < Fx::from_int(40), "{mid:?}");
    }

    /// Units keep their order across, so their ways to their places don't cross.
    #[test]
    fn a_row_keeps_the_order_its_units_stand_in() {
        // Four of a kind (one row) in a line along x, facing +z: right is -x, so the one
        // furthest towards +x stands furthest left.
        let members: Vec<Member> = (0..4).map(|k| member(k, k as i64 * 50, 0, 16, 0, 6)).collect();
        let places = rows(&members, 0);
        for k in 1..4 {
            assert!(places[k].across < places[k - 1].across, "{places:?}");
        }
        // Eight in a line along x, facing +x (two rows): those furthest ahead take the front.
        let members: Vec<Member> = (0..8).map(|k| member(k, k as i64 * 50, 0, 16, 0, 6)).collect();
        let places = rows(&members, 16384);
        let front = places.iter().map(|p| p.along).max().unwrap();
        let (ahead, behind): (Vec<usize>, Vec<usize>) = (0..8).partition(|&k| places[k].along == front);
        assert!(ahead.iter().all(|&a| behind.iter().all(|&b| a > b)), "{places:?}");
    }

    #[test]
    fn kept_shapes_turn_with_the_facing() {
        let members = vec![member(0, 0, 0, 16, 0, 6), member(1, 100, 0, 16, 0, 6)];
        // Facing +z, the one at +x is on the left; facing +x, it is ahead.
        let p = kept(&members, 0);
        assert!(p[1].across < Fx::ZERO && p[1].along.abs() < Fx::ONE, "{p:?}");
        let p = kept(&members, 16384);
        assert!(p[1].along > Fx::ZERO && p[1].across.abs() < Fx::ONE, "{p:?}");
        // On the ground again, where they stood (to within rounding).
        let middle = FxVec3::from_ints(50, 0, 0);
        let s = spot(middle, 16384, p[1]);
        assert!(s.distance_xz(FxVec3::from_ints(100, 0, 0)) < Fx::ONE, "{s:?}");
    }

    #[test]
    fn units_take_the_points_of_a_line_in_order_across() {
        // A line along x ahead of them (+z), units standing in the other order.
        let members = vec![member(0, 200, -300, 16, 0, 6), member(1, 0, -300, 16, 0, 6), member(2, 100, -300, 16, 0, 6)];
        let points = [FxVec3::from_ints(0, 0, 0), FxVec3::from_ints(100, 0, 0), FxVec3::from_ints(200, 0, 0)];
        let places = at_points(&members, &points, 0);
        let middle = centre(points.iter().copied());
        for (m, p) in members.iter().zip(&places) {
            let s = spot(middle, 0, *p);
            assert!(s.distance_xz(FxVec3::new(m.from.x, Fx::ZERO, Fx::ZERO)) < Fx::ONE, "{m:?} to {s:?}");
        }
    }
}
