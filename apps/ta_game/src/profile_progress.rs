//! Local career bookkeeping. Nothing here changes the simulation or its recorded orders.
use ta_config::profiles::{CampaignMission, MatchContext, MatchCounters, MatchMode};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
use ta_sim::{Locomotion, Sim, SimEvent};

/// A mobile unit at least this dear in metal (a Commander aside) is an experimental: a Krogoth.
const EXPERIMENTAL_METAL: i64 = 8000;

/// Stable identity and reconstructed event counters for one battle. Saves carry the ID,
/// while event totals are reconstructed by the same catch-up that restores the battle.
#[derive(Clone, Debug)]
pub struct MatchProgress {
    pub id: String,
    pub disqualified: bool,
    /// Cheats having been used, what the battle earned was taken back from the career (see
    /// `ProfileStore::void_match`): in this session, the once.
    pub voided: bool,
    pub needs_baseline: bool,
    pub resume_at_tick: u64,
    buildings: u64,
    captures: u64,
    unit_builds: BTreeMap<String, u64>,
    unit_names: BTreeMap<String, String>,
    /// Events by name (`MatchCounters::tallies`), and the features our builders began to
    /// reclaim (one destroyed while on the list was reclaimed).
    tallies: BTreeMap<String, u64>,
    reclaiming: BTreeSet<usize>,
    /// The army at its largest: the worth (metal, and energy at 60 to 1) of our armed
    /// mobile units, the Commander aside, looked at once a second.
    army: u64,
    /// What the ledger holds of this match: the last counters a checkpoint saved.
    pub saved_counters: Option<MatchCounters>,
    resumed: bool,
    checkpoint_tick: u64,
    outcome_saved: bool,
    retry_at: Option<Instant>,
    /// Historical and terminal snapshots are captured before I/O, never recomputed
    /// after a storage failure while the player continues the battle.
    baseline: Option<MatchCounters>,
    completed: Option<MatchCounters>,
}

impl Default for MatchProgress {
    fn default() -> Self {
        Self { id: ta_config::profiles::new_id(), disqualified: false, voided: false, needs_baseline: false, resume_at_tick: 0, buildings: 0, captures: 0, unit_builds: BTreeMap::new(), unit_names: BTreeMap::new(),
            tallies: BTreeMap::new(), reclaiming: BTreeSet::new(), army: 0, saved_counters: None, resumed: false, checkpoint_tick: 0, outcome_saved: false, retry_at: None, baseline: None, completed: None }
    }
}

impl MatchProgress {
    pub fn resume(&mut self, id: String, disqualified: bool) {
        self.id = id;
        self.disqualified |= disqualified;
        self.needs_baseline = true;
        self.resumed = true;
    }

    /// Tracked from its beginning in this session (a loaded battle's records may be another's).
    pub fn full_match(&self) -> bool { !self.resumed }

    /// Notes what a checkpoint saved: what the ledger holds of this match from now on.
    pub fn remember_saved(&mut self, counters: &MatchCounters) {
        self.saved_counters = Some(self.completed.clone().unwrap_or_else(|| counters.clone()));
    }

    pub fn note(&mut self, event: &SimEvent, sim: &Sim, player: u8) {
        let ours = |id: &ta_sim::UnitId| sim.unit(*id).is_some_and(|u| u.owner == player);
        let commander = |id: &ta_sim::UnitId| sim.unit(*id).is_some_and(|u| sim.unit_type(u).def.is_commander);
        match event {
            SimEvent::UnitCompleted { id, type_id } if ours(id) => {
                let ty = sim.types.get(*type_id);
                let def = &ty.def;
                if def.is_building() { self.buildings = self.buildings.saturating_add(1); }
                if self.unit_builds.contains_key(&def.name) || self.unit_builds.len() < 1024 {
                    let count = self.unit_builds.entry(def.name.clone()).or_default();
                    *count = count.saturating_add(1);
                    self.unit_names.entry(def.name.clone()).or_insert_with(|| def.display_name.clone());
                }
                match ty.locomotion {
                    Locomotion::Air => self.add("built.air"),
                    Locomotion::Naval => self.add("built.naval"),
                    _ => {}
                }
                if experimental(ty) { self.add("built.experimental"); }
            },
            SimEvent::UnitCaptured { to, .. } if *to == player => self.captures = self.captures.saturating_add(1),
            // Enemies destroyed by our own units (a killer that died with them goes uncounted).
            SimEvent::UnitDestroyed { type_id, owner, killer: Some(killer), .. } if sim.is_enemy(player, *owner) && ours(killer) => {
                let ty = sim.types.get(*type_id);
                if ty.def.is_feature { return; }
                if ty.def.is_commander { self.add("kills.commander"); }
                match ty.locomotion {
                    Locomotion::Air => self.add("kills.air"),
                    Locomotion::Naval => self.add("kills.naval"),
                    Locomotion::Static => self.add("kills.building"),
                    _ => {}
                }
                if experimental(ty) { self.add("kills.experimental"); }
                if commander(killer) {
                    self.add("commander.kills");
                    if ty.def.is_commander { self.add("commander.commanders"); }
                }
            }
            SimEvent::WeaponFired { unit, weapon, .. } if ours(unit) => {
                let def = &sim.weapon_types.get(*weapon).def;
                if def.stockpile && !def.interceptor { self.add("launched.missile"); }
            }
            SimEvent::UnitLoaded { transport, unit } if ours(transport) => {
                self.add("transported");
                if ours(unit) && commander(unit) { self.add("transported.commander"); }
            }
            SimEvent::Reclaiming { builder, feature } if ours(builder) && self.reclaiming.len() < 4096 => { self.reclaiming.insert(*feature); }
            SimEvent::FeatureDestroyed { index, .. } if self.reclaiming.remove(index) => self.add("reclaimed"),
            _ => {}
        }
    }

    /// Once a second of battle: the army's worth, for its record.
    pub fn sample(&mut self, sim: &Sim, player: u8) {
        if !sim.tick.is_multiple_of(30) { return; }
        let worth: i64 = sim.units().filter(|u| u.owner == player && !u.dead && u.is_complete()).map(|u| sim.unit_type(u))
            .filter(|t| t.locomotion != Locomotion::Static && !t.def.is_commander && t.weapons.iter().any(Option::is_some))
            .map(|t| (t.cost_metal + t.cost_energy / 60).to_int()).sum();
        self.army = self.army.max(worth.max(0) as u64);
    }

    fn add(&mut self, key: &str) {
        let count = self.tallies.entry(key.to_string()).or_default();
        *count = count.saturating_add(1);
    }

    pub fn counters(&self, game: &crate::game::Game) -> MatchCounters {
        let Some(player) = game.sim.players.get(game.local_player as usize) else { return MatchCounters::default() };
        let stats = &player.stats;
        MatchCounters {
            played_seconds: game.sim.tick / 30,
            kills: stats.kills,
            losses: stats.lost,
            units_built: stats.built,
            buildings_built: self.buildings.min(stats.built),
            units_captured: self.captures,
            metal_gathered: nonnegative_resource(stats.metal_produced),
            energy_gathered: nonnegative_resource(stats.energy_produced),
            unit_builds: self.unit_builds.clone(),
            unit_names: self.unit_names.clone(),
            finished: game.outcome.is_some(),
            won: game.outcome == Some(crate::game::Outcome::Victory),
            tallies: self.tallies.clone(),
            peaks: peaks(player, self.army),
        }
    }

    /// Capture a resume baseline or the first completed result even while a previous
    /// failed write is waiting to retry. Ordinary live counters need no per-frame copy.
    pub fn needs_snapshot(&self, finished: bool) -> bool {
        self.needs_baseline && self.baseline.is_none() || finished && self.completed.is_none()
    }

    pub fn snapshot(&mut self, counters: &MatchCounters) {
        if self.needs_baseline && self.baseline.is_none() { self.baseline = Some(counters.clone()); }
        if counters.finished && self.completed.is_none() { self.completed = Some(counters.clone()); }
    }

    /// KEEP WATCHING may clear the live outcome and switch to spectator mode after
    /// defeat. Its already observed result is still owed to the original player.
    pub fn has_completed(&self) -> bool { self.completed.is_some() }

    pub fn due(&self, tick: u64, finished: bool, force: bool) -> bool {
        self.due_at(tick, finished, force, Instant::now())
    }

    fn due_at(&self, tick: u64, finished: bool, force: bool, now: Instant) -> bool {
        if self.outcome_saved { return false; }
        if let Some(after) = self.retry_at { return force || now >= after; }
        self.needs_baseline || force || tick.saturating_sub(self.checkpoint_tick) >= 30 * 30 || finished
    }

    pub fn retry_after(&mut self) { self.retry_at = Some(Instant::now() + Duration::from_secs(5)); }

    /// Whether to try voiding the battle now (not while a failed attempt waits to retry).
    pub fn void_due(&self) -> bool { self.retry_at.is_none_or(|after| Instant::now() >= after) }

    /// A failed import write always uses the original reconstructed baseline; finished
    /// games always use their first result, even if simulation has continued since then.
    pub fn award(&self, store: &ta_config::profiles::ProfileStore, profile: &str, counters: &MatchCounters, context: &MatchContext) -> std::io::Result<ta_config::profiles::Award> {
        if self.needs_baseline {
            let baseline = self.baseline.as_ref().ok_or_else(|| std::io::Error::other("resume baseline has not been captured"))?;
            store.baseline(profile, &self.id, baseline)?;
        }
        store.award_match(profile, &self.id, self.completed.as_ref().unwrap_or(counters), context)
    }

    pub fn saved(&mut self, tick: u64, finished: bool) {
        self.retry_at = None;
        self.checkpoint_tick = tick;
        self.outcome_saved |= finished || self.has_completed();
        self.needs_baseline = false;
        self.baseline = None;
    }
}

/// Mode is taken from the running game rather than inferred from player count in a
/// saved setup. In particular, offline human slots cannot award multiplayer feats.
pub fn context(game: &crate::game::Game) -> MatchContext {
    MatchContext { world: game.world.clone(), ..context_for_setup(&game.setup, game.net.is_some(), game.tutorial) }
}

fn context_for_setup(setup: &crate::setup::GameSetup, connected: bool, tutorial: bool) -> MatchContext {
    use crate::setup::Controller;
    let local = setup.local_player.and_then(|i| setup.players.get(i as usize));
    let enemies: Vec<_> = setup.players.iter().filter(|p| local.is_some_and(|me| p.team != me.team)).collect();
    let allies: Vec<_> = setup.players.iter().enumerate()
        .filter(|(i, p)| setup.local_player != Some(*i as u8) && local.is_some_and(|me| p.team == me.team)).map(|(_, p)| p).collect();
    let human_opponent = connected && enemies.iter().any(|p| p.controller == Controller::Human);
    let human_ally = connected && allies.iter().any(|p| p.controller == Controller::Human);
    let mode = if tutorial { MatchMode::Tutorial }
        else if connected { MatchMode::Multiplayer }
        else if setup.mission.is_some() { MatchMode::Campaign }
        else if setup.scenario.is_some() { MatchMode::Scenario }
        else if setup.conquest.is_some() { MatchMode::Conquest }
        else { MatchMode::Skirmish };
    let difficulty = if let Some(mission) = &setup.mission { mission.difficulty.clone() }
        else if let Some(scenario) = &setup.scenario { format!("{:?}", scenario.difficulty) }
        else if !enemies.is_empty() && enemies.iter().all(|p| p.controller == Controller::Ai(ta_ai::Difficulty::Hard)) { "hard".to_string() }
        else if !enemies.is_empty() && enemies.iter().all(|p| p.controller == Controller::Ai(ta_ai::Difficulty::Easy)) { "easy".to_string() }
        else { "normal".to_string() };
    MatchContext {
        mode, map: setup.map.clone(), faction: local.map_or_else(String::new, |p| p.side.clone()), difficulty, human_opponent,
        mission: setup.mission.as_ref().map(|m| CampaignMission { campaign: m.campaign.clone(), index: m.index.min(u32::MAX as usize) as u32 }),
        enemies: enemies.len() as u32, allies: allies.len() as u32, human_ally, world: String::new(),
    }
}

/// The host's random match seed and agreed setup are unchanged by reconnecting.
/// This identity is presentation-only: it is never inserted into a network message.
pub fn network_id(setup: &crate::setup::GameSetup, data_hash: u64) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"TA Reborn local multiplayer career");
    hash.update(data_hash.to_le_bytes());
    if let Ok(bytes) = bincode::serialize(setup) { hash.update(bytes); }
    hash.finalize()[..16].iter().map(|b| format!("{b:02x}")).collect()
}

fn nonnegative_resource(value: ta_sim::Fx) -> u64 {
    value.to_int().max(0) as u64
}

/// The picture of an achievement's moment, from a frame's pixels (RGBA): no wider than 1280,
/// as a JPEG.
pub fn unlock_picture(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or("the picture's size and pixels disagree")?;
    let mut image = image::DynamicImage::ImageRgba8(image);
    if width > 1280 {
        image = image.resize(1280, 1280 * height / width.max(1) + 1, image::imageops::FilterType::Triangle);
    }
    let rgb = image.to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 86).encode_image(&rgb).map_err(|e| e.to_string())?;
    Ok(jpeg)
}

fn experimental(ty: &ta_sim::UnitType) -> bool {
    ty.locomotion != Locomotion::Static && !ty.def.is_commander && ty.cost_metal.to_int() >= EXPERIMENTAL_METAL
}

/// The battle's highest incomes (a second), from the statistics' samples and the economy of the
/// moment, and its army at its largest (its worth in metal).
fn peaks(player: &ta_sim::Player, army: u64) -> BTreeMap<String, u64> {
    let stats = &player.stats;
    let highest = |sample: fn(&ta_sim::StatSample) -> ta_sim::Fx, now: ta_sim::Fx| {
        stats.history.iter().map(|s| nonnegative_resource(sample(s))).chain([nonnegative_resource(now)]).max().unwrap_or(0)
    };
    [("metal_income", highest(|s| s.metal_income, player.economy.metal_income)),
        ("energy_income", highest(|s| s.energy_income, player.economy.energy_income)),
        ("army", army)]
        .into_iter().filter(|(_, v)| *v > 0).map(|(k, v)| (k.to_string(), v)).collect()
}

/// Technical runs have no player career, even if their scripted input later empties.
pub fn ordinary_run(args: &crate::Args) -> bool {
    !scripted_args(args)
        && !["TA_UI_CLICKS", "TA_STAGE", "TA_REVEAL", "TA_CAPTURE_FPS", "TA_RENDER_FROM", "TA_GALLERY_TOUR", "TA_CAMERA_PATH"]
            .iter().any(|name| std::env::var_os(name).is_some())
}

fn scripted_args(args: &crate::Args) -> bool {
    args.benchmark.is_some() || args.demo.is_some() || args.screenshot.is_some()
        || args.net_check_tick.is_some() || args.auto_start.is_some()
        || args.editor_open.is_some() || scripted_open(args.open.as_deref())
}

fn scripted_open(page: Option<&str>) -> bool {
    matches!(page, Some("conquest-demo" | "conquest-target" | "conquest-briefing" | "conquest-defence" | "conquest-perks" | "conquest-report" | "conquest-won" | "conquest-battle" | "conquest-victory" | "conquest-defeat"
        | "victory" | "defeat" | "loading" | "space" | "highlights" | "profile"))
        || page.is_some_and(|p| p.starts_with("photo"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn technical_runs_never_earn_career_xp() {
        for flags in [vec!["--benchmark", "report.json"], vec!["--demo", "battle"], vec!["--screenshot", "frame.png"], vec!["--open", "victory"], vec!["--editor-open", "demo"]] {
            let args = crate::Args::try_parse_from(std::iter::once("ta-reborn").chain(flags)).unwrap();
            assert!(scripted_args(&args));
        }
    }

    #[test]
    fn interactive_viewers_allow_later_live_matches_in_the_same_session() {
        // The current screen/playback/spectator checks exclude these activities themselves.
        // Returning to the menu and starting a normal battle must restore ordinary progress.
        for flags in [vec!["--replay", "test.tarep"], vec!["--editor"], vec!["--join", "localhost", "--watch"]] {
            let args = crate::Args::try_parse_from(std::iter::once("ta-reborn").chain(flags)).unwrap();
            assert!(!scripted_args(&args));
        }
    }

    #[test]
    fn launcher_shortcuts_are_real_play_not_capture_modes() {
        for page in ["tutorial", "conquest", "scenarios", "single-player", "multiplayer", "records", "options"] {
            assert!(!scripted_open(Some(page)), "{page} is a normal navigation shortcut");
        }
        for page in ["victory", "defeat", "conquest-battle", "highlights", "photo-take", "space"] {
            assert!(scripted_open(Some(page)));
        }
    }

    #[test]
    fn checkpoint_is_periodic_and_outcome_is_written_once() {
        let mut p = MatchProgress::default();
        assert!(!p.due(899, false, false));
        assert!(p.due(900, false, false));
        p.saved(900, false);
        assert!(!p.due(901, false, false));
        assert!(p.due(901, true, false));
        p.saved(901, true);
        assert!(!p.due(902, true, false));
        assert!(!p.due(902, true, true), "post-outcome simulation cannot farm more XP");
    }

    #[test]
    fn resume_preserves_identity_and_needs_historical_baseline() {
        let mut p = MatchProgress::default();
        let id = ta_config::profiles::new_id();
        p.resume(id.clone(), true);
        assert_eq!(p.id, id);
        assert!(p.disqualified && p.needs_baseline);
        assert!(p.due(0, false, false));
        p.saved(42, false);
        assert!(!p.needs_baseline);
        p.resume(id, false);
        assert!(p.disqualified, "loading may not undo a disqualification");
    }

    #[test]
    fn reconnects_keep_identity_but_new_matches_do_not() {
        let setup = crate::setup::GameSetup::duel("test");
        assert_eq!(network_id(&setup, 123), network_id(&setup.clone(), 123));
        let mut next = setup.clone();
        next.seed += 1;
        assert_ne!(network_id(&setup, 123), network_id(&next, 123));
        assert_ne!(network_id(&setup, 123), network_id(&setup, 456));
    }

    #[test]
    fn copied_save_and_repeated_resume_share_one_career_checkpoint() {
        use crate::replay::{Record, RecordKind};
        use ta_config::profiles::{Allegiance, ProfileStore};
        let root = std::env::temp_dir().join(format!("ta-game-career-{}", ta_config::profiles::new_id()));
        let store = ProfileStore::open(root.clone()).unwrap();
        let profile = store.create("First commander", Allegiance::Arm, None, &ta_config::Config::default()).unwrap();
        let id = ta_config::profiles::new_id();
        let before = MatchCounters { played_seconds: 60, kills: 2, units_built: 1, ..Default::default() };
        let initial = store.award(&profile.id, &id, &before).unwrap();
        assert!(initial.xp > 0);
        let record = Record::new(RecordKind::Save, 123, Vec::new(), crate::setup::GameSetup::duel("test"), vec![], 60 * 30, "checkpoint".into());
        let mut bytes = bincode::serialize(&record).unwrap();
        ta_config::records::append_mods(&mut bytes, record.mods.as_deref().unwrap_or_default()).unwrap();
        ta_config::records::append_metadata(&mut bytes, record.metadata.as_ref().unwrap()).unwrap();
        ta_config::profiles::append_record_metadata(&mut bytes, &id, false).unwrap();
        let path = root.join("copied-save.tasav");
        std::fs::write(&path, bytes).unwrap();
        for _ in 0..3 {
            let loaded = Record::read(&path).unwrap();
            let (resumed_id, _) = loaded.profile_match.unwrap();
            store.baseline(&profile.id, &resumed_id, &before).unwrap();
            assert_eq!(store.award(&profile.id, &resumed_id, &before).unwrap().xp, 0);
        }
        let after = MatchCounters { kills: 3, ..before.clone() };
        assert_eq!(store.award(&profile.id, &id, &after).unwrap().xp, 15);
        let recipient = store.create("Second commander", Allegiance::Core, None, &ta_config::Config::default()).unwrap();
        store.baseline(&recipient.id, &id, &before).unwrap();
        assert_eq!(store.award(&recipient.id, &id, &before).unwrap().xp, 0, "an imported save must not grant historic progress");
        assert_eq!(store.award(&recipient.id, &id, &after).unwrap().xp, 15);
        assert_eq!(store.career(&recipient.id).unwrap().kills, 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_career_writes_retry_while_simulation_is_paused() {
        let now = Instant::now();
        let mut progress = MatchProgress::default();
        progress.saved(900, false);
        progress.retry_at = Some(now + Duration::from_secs(5));
        assert!(!progress.due_at(900, true, false, now));
        assert!(!progress.due_at(900, true, false, now + Duration::from_secs(4)));
        assert!(progress.due_at(900, true, false, now + Duration::from_secs(5)), "paused ticks must not prevent storage recovery");
        assert!(progress.due_at(900, false, false, now + Duration::from_secs(5)), "a failed forced checkpoint also retries below the periodic tick threshold");
        assert!(progress.due_at(900, false, true, now), "exit retries immediately");
        progress.saved(900, true);
        assert!(!progress.due_at(900, true, true, now + Duration::from_secs(10)), "a committed outcome stays final");
    }

    #[test]
    fn failed_import_baseline_does_not_swallow_subsequent_live_progress() {
        use ta_config::profiles::{Allegiance, ProfileStore};
        let root = std::env::temp_dir().join(format!("ta-game-baseline-retry-{}", ta_config::profiles::new_id()));
        let store = ProfileStore::open(root.clone()).unwrap();
        let profile = store.create("Commander", Allegiance::Arm, None, &ta_config::Config::default()).unwrap();
        let ledger = store.data_dir(&profile.id).unwrap().join("career.toml");
        let mut progress = MatchProgress::default();
        progress.resume(ta_config::profiles::new_id(), false);
        let before = MatchCounters { played_seconds: 300, kills: 100, units_built: 10, ..Default::default() };
        let after = MatchCounters { played_seconds: 330, kills: 102, units_built: 11, ..before.clone() };
        let context = context_for_setup(&crate::setup::GameSetup::duel("test"), false, false);
        progress.snapshot(&before);
        std::fs::write(&ledger, "invalid = [").unwrap();
        assert!(progress.award(&store, &profile.id, &before, &context).is_err());
        assert_eq!(std::fs::read_to_string(&ledger).unwrap(), "invalid = [", "failure must preserve the existing ledger");
        progress.snapshot(&after); // live activity happened before the retry
        std::fs::remove_file(&ledger).unwrap();
        let award = progress.award(&store, &profile.id, &after, &context).unwrap();
        assert!(award.xp > 0);
        let career = store.career(&profile.id).unwrap();
        assert_eq!((career.kills, career.units_built, career.played_seconds), (2, 1, 30));
        assert_eq!(progress.award(&store, &profile.id, &after, &context).unwrap().xp, 0, "retrying a published write remains idempotent");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_outcome_checkpoint_keeps_the_first_result_after_continued_play() {
        use ta_config::profiles::{Allegiance, ProfileStore};
        let root = std::env::temp_dir().join(format!("ta-game-outcome-retry-{}", ta_config::profiles::new_id()));
        let store = ProfileStore::open(root.clone()).unwrap();
        let profile = store.create("Commander", Allegiance::Arm, None, &ta_config::Config::default()).unwrap();
        let ledger = store.data_dir(&profile.id).unwrap().join("career.toml");
        let context = context_for_setup(&crate::setup::GameSetup::duel("test"), false, false);
        let mut progress = MatchProgress::default();
        let now = Instant::now();
        progress.retry_at = Some(now + Duration::from_secs(5));
        let outcome = MatchCounters { played_seconds: 60, kills: 2, finished: true, won: true, ..Default::default() };
        assert!(progress.needs_snapshot(true));
        progress.snapshot(&outcome);
        assert!(!progress.due_at(1800, true, false, now), "result is frozen even before retry becomes due");
        std::fs::write(&ledger, "invalid = [").unwrap();
        assert!(progress.award(&store, &profile.id, &outcome, &context).is_err());
        let later = MatchCounters { played_seconds: 360, kills: 200, finished: false, won: false, ..outcome.clone() };
        progress.snapshot(&later);
        std::fs::remove_file(&ledger).unwrap();
        progress.award(&store, &profile.id, &later, &context).unwrap();
        assert_eq!(store.history(&profile.id).unwrap()[0].counters, outcome);
        assert_eq!(store.career(&profile.id).unwrap().kills, 2);
        assert!(progress.has_completed(), "KEEP WATCHING cannot discard the already observed result");
        progress.saved(10_800, false);
        assert!(!progress.due_at(10_800, false, true, now + Duration::from_secs(10)), "a committed result stays final after KEEP WATCHING clears the live outcome");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_achievements_picture_is_a_small_jpeg_of_the_frame() {
        let (w, h) = (1920, 1080);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i % 256) as u8, 90, 160, 255]).collect();
        let jpeg = unlock_picture(&rgba, w, h).unwrap();
        let back = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((back.width(), back.height()), (1280, 720));
        assert!(jpeg.len() < 1024 * 1024);
        assert!(unlock_picture(&rgba[..16], w, h).is_err());
    }

    #[test]
    fn economy_never_wraps_negative_totals() {
        assert_eq!(nonnegative_resource(ta_sim::Fx::from_int(-4)), 0);
        assert_eq!(nonnegative_resource(ta_sim::Fx::from_int(42)), 42);
    }
    #[test]
    fn multiplayer_requires_a_real_connection_and_a_human_opponent() {
        let mut setup = crate::setup::GameSetup::duel("test");
        setup.players[1].controller = crate::setup::Controller::Human;
        assert_eq!(context_for_setup(&setup, false, false).mode, MatchMode::Skirmish);
        assert!(!context_for_setup(&setup, false, false).human_opponent);
        assert_eq!(context_for_setup(&setup, true, false).mode, MatchMode::Multiplayer);
        assert!(context_for_setup(&setup, true, false).human_opponent);
        setup.players[1].team = setup.players[0].team;
        assert_eq!(context_for_setup(&setup, true, false).mode, MatchMode::Multiplayer);
        assert!(!context_for_setup(&setup, true, false).human_opponent);
        setup.players[1].team = 1;
        setup.players[1].controller = crate::setup::Controller::Ai(ta_ai::Difficulty::Hard);
        assert_eq!(context_for_setup(&setup, true, false).mode, MatchMode::Multiplayer);
        assert!(!context_for_setup(&setup, true, false).human_opponent);
        assert_eq!(context_for_setup(&setup, false, true).mode, MatchMode::Tutorial);
    }

    #[test]
    fn hard_skirmish_requires_every_enemy_ai_to_be_hard() {
        let mut setup = crate::setup::GameSetup::duel("test");
        setup.players[1].controller = crate::setup::Controller::Ai(ta_ai::Difficulty::Hard);
        assert_eq!(context_for_setup(&setup, false, false).difficulty, "hard");
        let mut extra = setup.players[1].clone(); extra.controller = crate::setup::Controller::Ai(ta_ai::Difficulty::Easy);
        setup.players.push(extra);
        assert_eq!(context_for_setup(&setup, false, false).difficulty, "normal");
        setup.players[2].team = setup.players[0].team;
        assert_eq!(context_for_setup(&setup, false, false).difficulty, "hard", "allied difficulty does not weaken the enemy condition");
    }

}
