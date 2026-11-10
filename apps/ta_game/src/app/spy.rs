//! `--demo spy`: a player's report of 2026-11-11 played out, as a soak test of the game's memory
//! ("I made the radar plane and used it to spy the enemy CPU... it used a ton of RAM and kept
//! increasing"). A skirmish (`--map`), both sides played by the computer, is fast-forwarded
//! `TA_SPY_MINUTES` (12) minutes so the enemy has a base worth looking at; then radar planes of
//! the player's side (`TA_SPY_PLANES`, 3: the side's flier with the farthest radar) patrol over
//! the enemy's start and back, sent off again when they idle and replaced when shot down, the
//! camera on the first, the planes selected (their radar's range on show) as a player's would
//! be; `TA_SPY_RIDE=1` rides the first in its cockpit, and the next when it is shot down;
//! `TA_SPY_GOD=1` keeps them from being shot down (TA's god mode on our side). Every
//! ten seconds of battle it logs the game's memory (`spy: ...`).
//!
//! `tools/soak/spy.ps1` runs it and fails when the memory keeps growing.

use super::*;

/// Ticks between the planes' orders being looked at, and between the memory's lines.
const EVERY: u64 = 300;

/// Radar planes over the enemy's base (see the module's documentation).
pub(super) struct Spy {
    kind: usize,
    count: usize,
    planes: Vec<ta_sim::UnitId>,
    /// Over the enemy's start, and back over ours.
    target: ta_sim::FxVec3,
    home: ta_sim::FxVec3,
    next_orders: u64,
    next_log: u64,
    /// Planes sent up so far (the first `count`, and those replacing the ones shot down).
    sent: usize,
}

fn env_number<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

impl State {
    /// The battle fast-forwarded, the planes chosen and sent up.
    pub(super) fn spy_begin(&mut self) {
        let minutes: u32 = env_number("TA_SPY_MINUTES", 12);
        if minutes > 0 {
            self.game.fast_forward(minutes * 30 * 60);
        }
        let me = self.game.local_player;
        let sim = &self.game.sim;
        let Some(player) = sim.players.get(me as usize) else { return };
        let prefix: String = player.side.chars().take(3).collect::<String>().to_ascii_uppercase();
        let Some(kind) = (0..sim.types.types.len())
            .filter(|&i| sim.types.get(i).def.can_fly && sim.types.get(i).radar > ta_sim::Fx::ZERO)
            .max_by_key(|&i| (sim.types.get(i).name.to_ascii_uppercase().starts_with(&prefix), sim.types.get(i).radar))
        else {
            log::warn!("spy: no flier with a radar in the game data");
            return;
        };
        let enemy = self.game.first_enemy().filter(|&e| e != me);
        let target = enemy.and_then(|e| sim.players.get(e as usize)).map_or(player.start, |p| p.start);
        let home = self.game.local_commander().and_then(|c| sim.unit(c)).map_or(player.start, |u| u.pos);
        log::info!("spy: {} minutes in, {} radar planes ({}) over the enemy's start", sim.tick / 1800, env_number("TA_SPY_PLANES", 3usize), sim.types.get(kind).name);
        // (`TA_SPY_GOD=1`: our units take no harm, so the planes stay over the enemy's base.)
        if std::env::var("TA_SPY_GOD").is_ok_and(|v| v == "1") {
            self.game.cheat(ta_sim::cheats::Cheat::Invulnerable(true));
        }
        self.spy = Some(Spy { kind, count: env_number("TA_SPY_PLANES", 3usize).max(1), planes: Vec::new(), target, home, next_orders: 0, next_log: 0, sent: 0 });
        self.spy_frame();
    }

    /// Each frame: planes lost replaced, idle ones sent off again, the camera on the first, the
    /// memory logged every ten seconds of battle.
    pub(super) fn spy_frame(&mut self) {
        let Some(spy) = self.spy.as_mut() else { return };
        let me = self.game.local_player;
        spy.planes.retain(|&id| self.game.sim.unit(id).is_some_and(|u| !u.dead && u.owner == me));
        while spy.planes.len() < spy.count {
            let offset = ta_sim::FxVec3::from_ints(64 * (spy.sent % 5) as i64 - 128, 0, 96);
            let Some(id) = self.game.sim.spawn_airborne(spy.kind, me, spy.home + offset, 0, None) else { break };
            spy.planes.push(id);
            spy.sent += 1;
            spy.next_orders = 0;
        }
        let tick = self.game.sim.tick;
        if tick >= spy.next_orders {
            let idle: Vec<ta_sim::UnitId> = spy.planes.iter().copied().filter(|&id| self.game.sim.unit(id).is_some_and(|u| u.orders.is_empty())).collect();
            if !idle.is_empty() {
                self.game.issue(ta_sim::Command::Order { player: me, units: idle.clone(), order: ta_sim::Order::Patrol(spy.target), queue: false });
                self.game.issue(ta_sim::Command::Order { player: me, units: idle, order: ta_sim::Order::Patrol(spy.home), queue: true });
            }
            spy.next_orders = tick + EVERY;
            self.game.selection.clone_from(&spy.planes);
        }
        let ride = std::env::var("TA_SPY_RIDE").is_ok_and(|v| v == "1");
        if !ride && let Some(p) = spy.planes.first().and_then(|&id| self.game.scene.unit_position(id, self.game.alpha())) {
            self.camera.focus = p;
        }
        if tick >= spy.next_log {
            let memory = ta_config::hardware::process_memory();
            log::info!(
                "spy: {} s of battle, {} planes up ({} sent), memory: private {} MB, working set {} MB (peak {} MB)",
                tick / 30,
                spy.planes.len(),
                spy.sent,
                memory.map_or(0, |m| m.private_mb),
                memory.map_or(0, |m| m.working_set_mb),
                memory.map_or(0, |m| m.peak_working_set_mb)
            );
            spy.next_log = tick + EVERY;
            #[cfg(all(feature = "memtrack", windows))]
            crate::memtrack::report(&format!("{} s", tick / 30));
        }
        // (Into the first plane's cockpit; the next one's when that is shot down.)
        if ride && self.perspective.is_none() && let Some(&id) = spy.planes.first() {
            let now = self.egui.ctx.input(|i| i.time);
            self.enter_perspective(id, now);
        }
    }
}
