//! Projectile flight and continuous collision detection, including missile interception.

use super::*;

/// How high a cruise missile keeps over the ground (px): a lower one climbs back to it.
const CRUISE_CLEARANCE: i64 = 128;

/// Where a cruise missile at `pos`, its target `to` away, steers: level towards it, high over
/// everything, until the target is 60° or more below it, then at the target, which it comes
/// down on from above. (Steered straight at the target from the top of their climb, a few
/// degrees above the ground over a long flight, TA's nukes went off against the first hill,
/// building or tall unit in front of where they were aimed: hundreds of pixels short.) TA's
/// own climb out of the tube takes them some 600 px up, well over its highest ground (255);
/// one lower than [`CRUISE_CLEARANCE`] over the ground, under it or just ahead, climbs (at 45°
/// from a clearance short by all of it). A target above the missile is steered at directly.
fn cruise_aim(world: &World, pos: FxVec3, to: FxVec3) -> FxVec3 {
    let flat = to.length_xz();
    // (tan 60° is 1.73.)
    if to.y >= Fx::ZERO || -to.y * 4 >= flat * 7 {
        return to;
    }
    let clearance = Fx::from_int(CRUISE_CLEARANCE);
    let ahead = pos + FxVec3::new(to.x, Fx::ZERO, to.z).scale(clearance / flat);
    let ground = world.height_at(pos.x, pos.z).max(world.height_at(ahead.x, ahead.z));
    let short = (clearance - (pos.y - ground)).clamp(Fx::ZERO, clearance);
    // (Level, or up by the share of the clearance it is short of: 45° from the ground.)
    FxVec3::new(to.x, flat * short / clearance, to.z)
}

#[derive(Clone, Copy)]
struct Impact {
    when: Fx,
    pos: FxVec3,
    direct: Option<UnitId>,
    water: bool,
}

impl Sim {
    fn projectile_hits_unit(&self, p: &Projectile, w: &WeaponType, u: &Unit) -> bool {
        if w.def.interceptor || u.dead || u.carried_by.is_some() || u.id == p.owner || p.hits.contains(&u.id) {
            return false;
        }
        let flame = w.def.render_type == RenderType::Flame;
        let friendly = self.allied(u.owner, p.owner_player);
        if friendly && (w.trajectory == Trajectory::Straight && !w.no_explode || flame) {
            return false;
        }
        let flying = !u.landed && self.types.get(u.type_id).locomotion == Locomotion::Air;
        !(flying && (!w.hits_air || friendly) || !w.water && self.submerged(u))
    }

    /// First solid contact on the planned flight. Interception may only precede this:
    /// a missile that has already hit a unit, terrain or water cannot be rescued later.
    fn projectile_impact(&self, p: &Projectile, w: &WeaponType, near: &mut Vec<u32>) -> Option<Impact> {
        let mut first: Option<(Fx, UnitId)> = None;
        if !w.no_explode && !w.def.interceptor {
            self.grid.query(p.pos, p.vel.length() + self.types.collision_reach, near);
            for &slot in near.iter() {
                let Some(u) = self.units.get(slot as usize).and_then(|u| u.as_ref()) else { continue };
                if !self.projectile_hits_unit(p, w, u) { continue; }
                let (centre, radius) = hit_sphere(u, self);
                if let Some(when) = segment_entry(p.prev_pos, p.pos, centre, radius)
                    && first.is_none_or(|(before, _)| when < before)
                {
                    first = Some((when, u.id));
                }
            }
        }
        let (a, b, sea) = (p.prev_pos, p.pos, self.world.sea_level);
        let armed = p.age > 2;
        let launching_submerged = w.def.fire_submersed && a.y < sea && b.y > a.y;
        let wet = (armed && b.y < sea && !w.water && !w.no_explode && !launching_submerged)
            .then(|| if a.y > sea { (a.y - sea) / (a.y - b.y) } else { Fx::ZERO });
        let landed = (armed && !w.no_explode).then(|| ground_entry(&self.world, a, b)).flatten();
        let landing = match (wet, landed) {
            (Some(s), Some(g)) => Some(s.min(g)),
            (s, g) => s.or(g),
        };
        let (width, height) = self.world.size_px();
        let mut exit: Option<Fx> = None;
        for (start, finish, size) in [(a.x, b.x, width), (a.z, b.z, height)] {
            let crossing = if start < Fx::ZERO || start > size { Some(Fx::ZERO) }
                else if finish < Fx::ZERO { Some(-start / (finish - start)) }
                else if finish > size { Some((size - start) / (finish - start)) }
                else { None };
            if let Some(when) = crossing { exit = Some(exit.map_or(when, |before| before.min(when))); }
        }
        let end = match (landing, exit) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) };
        let hit = first.filter(|&(when, _)| end.is_none_or(|end| when <= end));
        if hit.is_none() && end.is_none() && p.age < p.ttl {
            return None;
        }
        let (when, direct) = match (hit, end) {
            (Some((when, id)), _) => (when, Some(id)),
            (None, Some(when)) => (when, None),
            _ => (Fx::ONE, None),
        };
        let at = a + (b - a).scale(when);
        let pos = match (direct, landing.filter(|&land| land == when)) {
            (None, Some(l)) if landed == Some(l) => FxVec3::new(at.x, self.world.height_at(at.x, at.z), at.z),
            (None, Some(_)) => FxVec3::new(at.x, sea, at.z),
            _ => at,
        };
        Some(Impact { when, pos, direct, water: pos.y <= sea && self.world.depth_at(pos.x, pos.z) > Fx::ZERO })
    }

    /// Plan every flight from the same snapshot, then resolve swept interceptions before
    /// ordinary impacts. All geometry and ordering use fixed-point simulation values.
    pub(crate) fn update_projectiles(&mut self) {
        let weapons = self.weapon_types.clone();
        let mut projectiles = std::mem::take(&mut self.projectiles);
        let mut near = std::mem::take(&mut self.scratch);
        self.spent.clear();
        let live: rustc_hash::FxHashMap<u32, (usize, FxVec3, u8, bool)> = if projectiles.iter().any(|p| matches!(p.target, Some(Target::Projectile(_)))) {
            projectiles.iter().enumerate().filter(|(_, p)| !p.dead)
                .map(|(i, p)| (p.id, (i, p.pos, p.owner_player, weapons.get(p.weapon).def.targetable))).collect()
        } else { Default::default() };

        for p in &mut projectiles {
            if p.dead { continue; }
            let w = weapons.get(p.weapon);
            p.prev_pos = p.pos;
            p.age += 1;
            match p.target {
                Some(Target::Projectile(target)) => match live.get(&target) {
                    Some(&(_, pos, owner, targetable)) if !w.def.interceptor || targetable && self.is_enemy(p.owner_player, owner) => p.target_pos = pos,
                    // An interceptor without a live quarry has no ground target. Retire
                    // it without a blast rather than harming units at the old intercept point.
                    _ if w.def.interceptor => {
                        p.dead = true;
                        self.spent.push((p.clone(), Fx::ZERO));
                        continue;
                    }
                    _ => p.target = None,
                },
                // (A missile sent at a unit follows it; TA's nukes were sent at the ground under it
                // as they left the tube instead, and keep to that: `Sim::launch_target`.)
                Some(target) => {
                    if let Some(pos) = self.weapon_target_point(w, target) { p.target_pos = pos; }
                    else if w.torpedo { p.target = None; }
                }
                None => {}
            }
            if w.torpedo && p.pos.y > self.world.sea_level {
                // A launched/dropped torpedo falls into the water before its seeker
                // can steer it; it cannot fly a guided path through the air.
                p.vel.y -= self.gravity;
            } else if w.trajectory == Trajectory::Guided || w.torpedo && (w.def.tracks || w.def.guidance) {
                if w.vlaunch && p.age < w.launch_ticks {
                    p.vel = FxVec3::new(Fx::ZERO, (p.vel.y + w.acceleration).min(w.velocity), Fx::ZERO);
                } else {
                    let to = p.target_pos - p.pos;
                    // TA's `cruise` missiles launched upright (its strategic ones: the nukes, the
                    // EMP and tactical missiles) cross the map level, high over everything, and
                    // dive onto their target from above (see `cruise_aim`).
                    let aim = if w.vlaunch && w.def.cruise { cruise_aim(&self.world, p.pos, to) } else { to };
                    let speed = (p.vel.length() + w.acceleration).min(w.velocity);
                    let turn = w.turn_rate.max(if w.torpedo { 1 } else { 200 });
                    // (Straight up, at the top of its climb, a missile has no heading: it takes
                    // its target's and turns over towards it. Taking north's, it turned over
                    // northwards and hooked round to its target.)
                    let cur_h = if p.vel.x.is_zero() && p.vel.z.is_zero() { heading_of(aim) } else { heading_of(p.vel) };
                    let dh = wrap_angle(heading_of(aim) - cur_h).clamp(-turn, turn);
                    let cur_p = atan2(p.vel.y, p.vel.length_xz());
                    let dp = wrap_angle(atan2(aim.y, aim.length_xz()) - cur_p).clamp(-turn, turn);
                    let dir = rotate_y(rotate_x(FxVec3::new(Fx::ZERO, Fx::ZERO, Fx::ONE), -(cur_p + dp)), cur_h + dh);
                    p.vel = dir.scale(speed);
                }
            } else if matches!(w.trajectory, Trajectory::Ballistic | Trajectory::Dropped) {
                p.vel.y -= self.gravity;
            }
            p.pos += p.vel;
            if w.torpedo && p.prev_pos.y <= self.world.sea_level && p.pos.y >= self.world.sea_level {
                p.pos.y = self.world.sea_level - Fx::ONE;
                p.vel.y = p.vel.y.min(Fx::ZERO);
            }
        }

        let intercepting = projectiles.iter().any(|p| !p.dead && weapons.get(p.weapon).def.interceptor);
        // Ordinary battles keep their existing sequential contact checks. Only missile
        // pairs need contacts prepared together to compare their times of interception.
        let impacts: Vec<_> = if intercepting { projectiles.iter().map(|p| {
            let w = weapons.get(p.weapon);
            if p.dead || !(w.def.interceptor || w.def.targetable) { None }
            else { self.projectile_impact(p, w, &mut near) }
        }).collect() } else { Vec::new() };
        let mut candidates: Vec<_> = projectiles.iter().enumerate().filter_map(|(i, p)| {
            if p.dead || !weapons.get(p.weapon).def.interceptor { return None; }
            let Some(Target::Projectile(target)) = p.target else { return None };
            let &(j, ..) = live.get(&target)?;
            let q = &projectiles[j];
            if q.dead || !weapons.get(q.weapon).def.targetable || !self.is_enemy(p.owner_player, q.owner_player) { return None; }
            // Relative motion catches crossing missiles, and rejects missiles flying apart.
            let when = segment_entry(p.prev_pos - q.prev_pos, p.pos - q.pos, FxVec3::ZERO, Fx::from_int(40))?;
            if [impacts[i], impacts[j]].iter().flatten().any(|impact| impact.when < when) { return None; }
            Some((when, i, j))
        }).collect();
        candidates.sort_by_key(|&(when, i, j)| (when, projectiles[j].id, projectiles[i].id));
        let mut interceptions = Vec::new();
        for (when, i, j) in candidates {
            if projectiles[i].dead || projectiles[j].dead { continue; }
            let at = projectiles[j].prev_pos + (projectiles[j].pos - projectiles[j].prev_pos).scale(when);
            for index in [i, j] {
                projectiles[index].dead = true;
                self.spent.push((projectiles[index].clone(), when));
            }
            interceptions.push((i, j, at, when));
        }

        for (index, p) in projectiles.iter_mut().enumerate() {
            if p.dead { continue; }
            let w = weapons.get(p.weapon);
            if w.no_explode && !w.def.interceptor {
                self.grid.query(p.pos, p.vel.length() + self.types.collision_reach + w.aoe_radius, &mut near);
                for &slot in &near {
                    let Some(u) = self.units.get(slot as usize).and_then(|u| u.as_ref()) else { continue };
                    if !self.projectile_hits_unit(p, w, u) { continue; }
                    let (centre, radius) = hit_sphere(u, self);
                    if segment_distance(p.prev_pos, p.pos, centre) <= radius + w.aoe_radius {
                        p.hits.push(u.id);
                        if !self.ceasefire_spares_side(p.owner_player, u.owner) {
                            self.damage_unit(u.id, w.damage_to(u.type_id), p.owner, p.vel);
                        }
                    }
                }
                self.damage_features_along(p.prev_pos, p.pos, w);
            }
            let impact = if intercepting && (w.def.interceptor || w.def.targetable) { impacts[index] }
                else { self.projectile_impact(p, w, &mut near) };
            if let Some(impact) = impact {
                if !(p.age >= p.ttl && w.no_explode) || impact.direct.is_some() {
                    let flame = w.def.render_type == RenderType::Flame;
                    self.explode_at(w, impact.pos, p.owner, Some(p.owner_player), impact.direct, impact.water, false, flame.then_some(p.owner_player), impact.when);
                }
                p.dead = true;
                self.spent.push((p.clone(), impact.when));
            }
        }
        // An intercepted warhead never contributes its nuclear blast.
        for (index, quarry, at, when) in interceptions {
            let p = &projectiles[index];
            self.explode_at(weapons.get(p.weapon), at, p.owner, Some(p.owner_player), None, false, false, None, when);
            self.events.push(SimEvent::Intercepted { pos: at, defender: p.owner_player, attacker: projectiles[quarry].owner_player });
        }
        projectiles.retain(|p| !p.dead);
        projectiles.append(&mut self.projectiles);
        self.intercepted.clear();
        self.intercepted.extend(projectiles.iter().filter_map(|p| match p.target {
            Some(Target::Projectile(target)) if weapons.get(p.weapon).def.interceptor => Some(target),
            _ => None,
        }));
        self.projectiles = projectiles;
        self.scratch = near;
    }
}
