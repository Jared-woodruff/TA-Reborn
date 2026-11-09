//! Retry completed progress independently of the live battle. Snapshot results once;
//! storage failure must not lose an unlock or rescore a battle that keeps running.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use crate::conquest::{BattleResult, Conquest};

const RETRY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct PendingWrites {
    pending: Vec<Pending>,
    retry_at: Option<Instant>,
    last_error: Option<String>,
}

struct Pending { path: PathBuf, value: Value }
enum Value { Progress(Vec<Update>), Conquest(BattleResult) }

/// Only changes made by this game, not its potentially stale copy of the whole file.
/// Keeping the scenario policy with the result makes retries independent of the UI.
#[derive(Clone)]
pub(super) enum Update {
    Difficulty(String),
    Mission { campaign: String, index: usize },
    Tutorial(usize),
    Scenario { id: String, difficulty: String, won: bool, seconds: u32, endless: bool },
}

impl Update {
    fn apply(&self, progress: &mut ta_config::CampaignProgress) {
        match self {
            Self::Difficulty(value) => progress.difficulty = value.clone(),
            Self::Mission { campaign, index } => progress.complete(campaign, *index),
            Self::Tutorial(lesson) => progress.complete_tutorial(*lesson),
            Self::Scenario { id, difficulty, won, seconds, endless } => { progress.record_scenario(id, difficulty, *won, *seconds, *endless); }
        }
    }
}

#[derive(Default)]
struct Written {
    progress: Option<ta_config::CampaignProgress>,
    conquest_changed: bool,
}

#[derive(Default)]
pub(super) struct Flushed {
    pub error: Option<String>,
    pub conquest_changed: bool,
    pub progress: Option<ta_config::CampaignProgress>,
}

impl PendingWrites {
    pub fn progress(&mut self, update: Update) {
        self.progress_at(ta_config::CampaignProgress::path(), update);
    }

    fn progress_at(&mut self, path: PathBuf, update: Update) {
        if let Some(Pending { value: Value::Progress(updates), .. }) = self.pending.iter_mut().find(|p| p.path == path && matches!(p.value, Value::Progress(_))) {
            updates.push(update);
        } else {
            self.pending.push(Pending { path, value: Value::Progress(vec![update]) });
        }
    }

    pub fn conquest(&mut self, result: BattleResult) {
        self.conquest_at(Conquest::path(), result);
    }

    fn conquest_at(&mut self, path: PathBuf, result: BattleResult) {
        if self.pending.iter().any(|p| p.path == path && matches!(&p.value, Value::Conquest(r) if (r.seed, r.battle) == (result.seed, result.battle))) { return; }
        self.pending.push(Pending { path, value: Value::Conquest(result) });
    }

    pub fn flush(&mut self, force: bool) -> Flushed {
        self.flush_with(Instant::now(), force, Pending::write)
    }

    fn flush_with(&mut self, now: Instant, force: bool, mut write: impl FnMut(&Pending) -> std::io::Result<Written>) -> Flushed {
        if self.pending.is_empty() || !force && self.retry_at.is_some_and(|next| now < next) { return Flushed::default(); }
        let mut first_error = None;
        let mut conquest_changed = false;
        let mut progress = None;
        self.pending.retain(|pending| match write(pending) {
            Ok(written) => {
                conquest_changed |= written.conquest_changed;
                if written.progress.is_some() { progress = written.progress; }
                false
            }
            Err(error) => {
                if first_error.is_none() { first_error = Some(format!("{}: {error}", pending.path.display())); }
                true
            }
        });
        self.retry_at = (!self.pending.is_empty()).then_some(now + RETRY);
        let error = first_error.as_ref().filter(|e| Some(*e) != self.last_error.as_ref()).cloned();
        self.last_error = first_error;
        Flushed { error, conquest_changed, progress }
    }
}

impl Pending {
    fn write(&self) -> std::io::Result<Written> {
        match &self.value {
            Value::Progress(updates) => {
                let progress = ta_config::CampaignProgress::update_at(&self.path, |progress| {
                    for update in updates { update.apply(progress); }
                })?;
                Ok(Written { progress: Some(progress), ..Default::default() })
            }
            Value::Conquest(result) => {
                // Load, validate and publish under one transaction shared with
                // starting/abandoning a galaxy and other running games.
                let changed = Conquest::record_at(&self.path, result.clone())?;
                Ok(Written { conquest_changed: changed, ..Default::default() })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conquest::{Level, Size, Veteran};
    use crate::setup::MapEntry;
    use ta_sim::Fx;

    fn temp_progress() -> PathBuf {
        std::env::temp_dir().join(format!("ta-progress-retry-{}", ta_config::profiles::new_id())).join("progress.toml")
    }

    #[test]
    fn failed_progress_retries_frozen_results_without_rescoring_or_spamming() {
        let path = temp_progress();
        let mut pending = PendingWrites::default();
        pending.progress_at(path.clone(), Update::Mission { campaign: "arm".into(), index: 2 });
        pending.progress_at(path.clone(), Update::Tutorial(1));
        let mut seconds = 120;
        pending.progress_at(path.clone(), Update::Scenario { id: "endless".into(), difficulty: "normal".into(), won: false, seconds, endless: true });
        seconds = 999; // simulation continues independently of the queued result
        let now = Instant::now();
        let failed = pending.flush_with(now, false, |_| Err(std::io::Error::other("disk full")));
        assert!(failed.error.is_some());
        pending.flush_with(now + Duration::from_secs(1), false, |_| panic!("retry cadence ignored"));
        assert!(pending.flush_with(now + RETRY, false, |_| Err(std::io::Error::other("disk full"))).error.is_none(), "repeat failures do not flood messages");
        let saved = pending.flush_with(now + RETRY + Duration::from_secs(1), true, Pending::write).progress.unwrap();
        assert_eq!((saved.unlocked("arm"), saved.tutorials.as_slice()), (3, &[1][..]));
        assert_eq!(saved.scenarios["endless"].best["normal"], 120);
        assert_ne!(saved.scenarios["endless"].best["normal"], seconds);
        assert!(pending.pending.is_empty());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn delayed_progress_merges_other_sessions_and_preserves_scenario_policies() {
        let path = temp_progress();
        let mut pending = PendingWrites::default();
        pending.progress_at(path.clone(), Update::Mission { campaign: "arm".into(), index: 0 });
        pending.progress_at(path.clone(), Update::Tutorial(2));
        pending.progress_at(path.clone(), Update::Scenario { id: "race".into(), difficulty: "hard".into(), won: true, seconds: 90, endless: false });
        pending.progress_at(path.clone(), Update::Scenario { id: "siege".into(), difficulty: "normal".into(), won: false, seconds: 500, endless: true });
        assert_eq!(pending.pending.len(), 1, "one atomic batch retains every pending unlock");
        pending.flush_with(Instant::now(), false, |_| Err(std::io::Error::other("disk full")));
        ta_config::CampaignProgress::update_at(&path, |other| {
            other.complete("core", 4);
            other.complete("arm", 3);
            other.complete_tutorial(1);
            other.difficulty = "Hard".into();
            other.record_scenario("race", "hard", true, 80, false);
            other.record_scenario("siege", "normal", false, 600, true);
        }).unwrap();
        let saved = pending.flush(true).progress.unwrap();
        assert_eq!((saved.unlocked("arm"), saved.unlocked("core")), (4, 5));
        assert_eq!(saved.tutorials, [1, 2]);
        assert_eq!(saved.difficulty, "Hard", "an unlock must not restore this process's stale difficulty");
        assert_eq!(saved.scenarios["race"].best["hard"], 80);
        assert_eq!(saved.scenarios["siege"].best["normal"], 600);
        assert!(saved.scenarios["race"].won.contains(&"hard".to_string()));
        pending.progress_at(path.clone(), Update::Difficulty("Easy".into()));
        assert_eq!(pending.flush(true).progress.unwrap().difficulty, "Easy", "a deliberate preference change still applies");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn conquest_retry_uses_frozen_survivors_and_cannot_repeat_or_replace_a_campaign() {
        let dir = std::env::temp_dir().join(format!("ta_progress_retry_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("conquest.toml");
        let maps: Vec<_> = (0..16).map(|i| MapEntry { name: format!("map{i}"), title: format!("Map {i}"), description: String::new(), planet: "Green Planet".into(), size: "10 x 10".into(), start_positions: 2, info: Default::default() }).collect();
        let initial = Conquest::new(&maps, "ARM", Level::Standard, Size::Small, 123).unwrap();
        let target = (0..initial.systems.len()).find(|&i| initial.attackable(i)).unwrap();
        ta_config::write_atomic(&path, &toml::to_string(&initial).unwrap()).unwrap();
        let mut result = BattleResult { seed: initial.seed, battle: 0, system: target, won: true, selected: vec![], duration_ticks: 900, undeployed: vec![], survivors: vec![Veteran { unit: "ARMPW".into(), xp: Fx::from_int(2) }], commander_xp: Some(Fx::from_int(5)) };
        let mut pending = PendingWrites::default();
        pending.conquest_at(path.clone(), result.clone());
        result.survivors.clear(); // continued play cannot change the queued outcome
        assert!(pending.flush_with(Instant::now(), false, |_| Err(std::io::Error::other("disk full"))).error.is_some());
        assert!(pending.flush(true).conquest_changed);
        let saved: Conquest = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((saved.battles, saved.wins, saved.roster.len()), (1, 1, 1));
        pending.conquest_at(path.clone(), result.clone());
        assert!(!pending.flush(true).conquest_changed);
        assert_eq!(toml::from_str::<Conquest>(&std::fs::read_to_string(&path).unwrap()).unwrap(), saved);
        pending.conquest_at(path.clone(), result);
        let replacement = Conquest::new(&maps, "CORE", Level::Standard, Size::Small, 456).unwrap();
        ta_config::write_atomic(&path, &toml::to_string(&replacement).unwrap()).unwrap();
        pending.flush(true);
        assert_eq!(toml::from_str::<Conquest>(&std::fs::read_to_string(&path).unwrap()).unwrap(), replacement);
        pending.conquest_at(path.clone(), BattleResult { seed: replacement.seed, battle: 0,
            system: (0..replacement.systems.len()).find(|&i| replacement.attackable(i)).unwrap(),
            won: true, selected: vec![], duration_ticks: 900, survivors: vec![], undeployed: vec![], commander_xp: None });
        Conquest::delete_at(&path).unwrap();
        let flushed = pending.flush(true);
        assert!(flushed.error.is_none() && !flushed.conquest_changed && pending.pending.is_empty(), "abandoned battles do not retry forever or resurrect a campaign");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
