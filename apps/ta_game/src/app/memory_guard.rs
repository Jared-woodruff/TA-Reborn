//! A watch on the game's own memory, after a player's report of 2026-11-11 ("it used a ton of
//! RAM and kept increasing", until the game crashed):
//!
//! * In a battle, a line in the log every minute (`memory: ...`): a report's log shows how the
//!   memory went, and from when.
//! * Past a warning level (see [`level`]: half the computer's memory, three fifths of a small
//!   one's (8 GB at most), or 4 GB
//!   more than the battle had after its first minute; again at every 2 GB past the last): what
//!   the game holds is logged, the highlights' copies of the game are let go but the two newest
//!   (the most memory a battle keeps that it can do without), a single-player battle is saved (the
//!   autosave), and the player is told.
//! * Past a limit (four fifths of the computer's memory, or under 1 GB of it free while the game
//!   holds a quarter or more): the same, and a single-player battle is paused, the player told to
//!   start the game again from the autosave, before the system runs out and takes the battle with
//!   it.
//!
//! Captures: `TA_MEMORY_TOTAL_MB=<n>` takes the computer to have that much memory (all of it
//! free but what the game holds), to try the warnings on any computer.

use std::time::Duration;

use super::*;

/// Seconds between looks at the memory, and between the log's lines in a battle.
const LOOK: Duration = Duration::from_secs(2);
const LINE: Duration = Duration::from_secs(60);
/// A further warning comes at every this many MB past the last.
const AGAIN_MB: u64 = 2048;
/// The battle's memory is taken as its own after this many ticks (a minute).
const BASELINE_TICKS: u64 = 1800;

/// How the game's memory stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    Fine,
    Warning,
    Critical,
}

/// The level for `private_mb` of the game's own (see [`ta_config::hardware::ProcessMemory`]),
/// `baseline_mb` its battle had after its first minute, on a computer of `total_mb` with
/// `free_mb` free for programs.
pub(super) fn level(private_mb: u64, baseline_mb: Option<u64>, total_mb: u64, free_mb: u64) -> Level {
    if total_mb == 0 {
        return Level::Fine;
    }
    if private_mb >= total_mb / 5 * 4 || (free_mb < 1024 && private_mb >= total_mb / 4) {
        return Level::Critical;
    }
    // (Half the computer's memory; on one of under 16 GB, three fifths of it, to come before
    // the limit.)
    let high = (total_mb / 2).max((total_mb / 5 * 3).min(8 * 1024));
    if private_mb >= high || baseline_mb.is_some_and(|b| private_mb >= b + 4096) {
        return Level::Warning;
    }
    Level::Fine
}

/// What the watch has seen (see the module's documentation).
#[derive(Default)]
pub(super) struct Guard {
    next_look: Option<Instant>,
    next_line: Option<Instant>,
    /// The battle's tick at the last look (an earlier one: another battle, watched afresh).
    tick: Option<u64>,
    baseline: Option<u64>,
    /// The memory at the last warning, and whether the limit was met.
    warned_at: Option<u64>,
    critical: bool,
}

/// `TA_MEMORY_TOTAL_MB`, read once.
fn pretended_total() -> Option<u64> {
    static TOTAL: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *TOTAL.get_or_init(|| std::env::var("TA_MEMORY_TOTAL_MB").ok().and_then(|v| v.trim().parse().ok()))
}

impl State {
    /// Looks at the memory every two seconds (see the module's documentation).
    pub(super) fn memory_watch(&mut self, now: Instant) {
        if self.memory.next_look.is_some_and(|at| now < at) {
            return;
        }
        self.memory.next_look = Some(now + LOOK);
        if self.screen != Screen::Playing {
            return;
        }
        let Some(m) = ta_config::hardware::process_memory() else { return };
        let (total, free) = match pretended_total() {
            Some(total) => (total, total.saturating_sub(m.private_mb)),
            None => ta_config::hardware::system_memory_mb().unwrap_or((0, 0)),
        };
        let tick = self.game.sim.tick;
        if self.memory.tick.is_some_and(|t| tick < t) {
            self.memory = Guard { next_look: self.memory.next_look, ..Default::default() };
        }
        self.memory.tick = Some(tick);
        if self.memory.baseline.is_none() && tick >= BASELINE_TICKS {
            self.memory.baseline = Some(m.private_mb);
        }
        if self.memory.next_line.is_none_or(|at| now >= at) {
            self.memory.next_line = Some(now + LINE);
            log::info!(
                "memory: {}:{:02} of battle, the game's own {} MB (in use {} MB, the most {} MB), the computer's {} MB free of {} MB; {} units, {} highlight copies",
                tick / 1800,
                tick / 30 % 60,
                m.private_mb,
                m.working_set_mb,
                m.peak_working_set_mb,
                free,
                total,
                self.game.sim.unit_count(),
                self.game.tally.snapshots.len()
            );
        }
        match level(m.private_mb, self.memory.baseline, total, free) {
            Level::Fine => {}
            Level::Warning if self.memory.warned_at.is_none_or(|w| m.private_mb >= w + AGAIN_MB) => self.memory_warning(m, total, free, false),
            Level::Critical if !self.memory.critical => {
                self.memory.critical = true;
                self.memory_warning(m, total, free, true);
            }
            Level::Warning | Level::Critical => {}
        }
    }

    /// What the game holds logged, the highlights' copies let go, the battle saved (and paused,
    /// `critical`), the player told.
    fn memory_warning(&mut self, m: ta_config::hardware::ProcessMemory, total: u64, free: u64, critical: bool) {
        self.memory.warned_at = Some(m.private_mb);
        let textures = self.egui.ctx.tex_manager().read().num_allocated();
        let copies = self.game.tally.shed();
        log::warn!(
            "memory {}: the game's own {} MB (the most in use {} MB), the computer's {} MB free of {} MB, after the battle's first minute {}; holding {} units, {} interface textures; {copies} highlight copies let go",
            if critical { "running out" } else { "high" },
            m.private_mb,
            m.peak_working_set_mb,
            free,
            total,
            self.memory.baseline.map_or_else(|| "not yet known".to_string(), |b| format!("{b} MB")),
            self.game.sim.unit_count(),
            textures,
        );
        let saved = self.save_before_closing();
        let gb = format!("{:.1}", m.private_mb as f64 / 1024.0);
        let single = self.game.net.is_none() && self.game.playback.is_none();
        if critical && single {
            self.game.set_pace(Some(true), None);
        }
        let text = match (critical, saved) {
            (true, true) => ta_ui::i18n::n("Memory is running out: TA Reborn holds {gb} GB. The battle was saved and paused: start TA Reborn again and load the autosave."),
            (true, false) => ta_ui::i18n::n("Memory is running out: TA Reborn holds {gb} GB. Save what you can, then start TA Reborn again."),
            (false, true) => ta_ui::i18n::n("TA Reborn is using {gb} GB of memory, far more than it should. The battle was saved (the autosave)."),
            (false, false) => ta_ui::i18n::n("TA Reborn is using {gb} GB of memory, far more than it should."),
        };
        self.game.trouble(ta_ui::i18n::tf(text, &[("gb", &gb)]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024;

    #[test]
    fn an_ordinary_battle_is_fine_on_any_computer() {
        // (A big battle at Ultra: 3 GB of its own.)
        for total in [8 * GB, 16 * GB, 48 * GB, 128 * GB] {
            assert_eq!(level(3 * GB, Some(2 * GB), total, total / 2), Level::Fine, "{total} MB");
        }
        // Nothing known of the computer: nothing to say.
        assert_eq!(level(40 * GB, None, 0, 0), Level::Fine);
    }

    #[test]
    fn memory_climbing_past_what_the_battle_began_with_is_told() {
        // On a computer of 48 GB, half of it is far off: 4 GB more than after the first minute
        // is the warning.
        assert_eq!(level(6 * GB, Some(2 * GB), 48 * GB, 30 * GB), Level::Warning);
        assert_eq!(level(5 * GB, Some(2 * GB), 48 * GB, 30 * GB), Level::Fine);
        // Before the first minute is over, half the computer's memory.
        assert_eq!(level(24 * GB, None, 48 * GB, 20 * GB), Level::Warning);
        assert_eq!(level(23 * GB, None, 48 * GB, 20 * GB), Level::Fine);
        // On a small computer three fifths of it, so that the warning comes before the limit.
        assert_eq!(level(5 * GB, None, 8 * GB, 3 * GB), Level::Warning);
        assert_eq!(level(4 * GB, None, 8 * GB, 3 * GB), Level::Fine);
        assert_eq!(level(7 * GB, None, 8 * GB, 2 * GB), Level::Critical, "past four fifths of 8 GB");
        for total in [4 * GB, 8 * GB, 12 * GB, 16 * GB, 32 * GB, 48 * GB, 128 * GB] {
            let first = (0..=total).step_by(64).find(|&m| level(m, None, total, total).ne(&Level::Fine)).unwrap();
            assert_eq!(level(first, None, total, total), Level::Warning, "{total} MB: warned before the limit");
        }
    }

    #[test]
    fn the_limit_comes_before_the_computer_runs_out() {
        assert_eq!(level(39 * GB, Some(2 * GB), 48 * GB, 6 * GB), Level::Critical, "four fifths of the computer");
        // Little left free while the game holds a quarter or more: the limit too.
        assert_eq!(level(12 * GB, Some(2 * GB), 48 * GB, 900), Level::Critical);
        // Little left free because of other programs, the game modest: not the game's doing.
        assert_eq!(level(3 * GB, Some(2 * GB), 48 * GB, 900), Level::Fine);
    }
}
