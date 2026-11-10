//! Battle highlights. While a battle is played (or watched), its losses are tallied: every
//! unit destroyed adds its worth where and when it fell, and nuclear strikes and fallen
//! Commanders count extra. Every 20 seconds of game time a copy of the game is kept (the
//! simulation and its computer players, a few megabytes); the copies before the heaviest
//! losses survive, the rest make room. After the battle, WATCH HIGHLIGHTS plays its decisive
//! moments in the order they happened: each from the copy before it, fast-forwarded to a
//! few seconds before the fighting, then at normal speed with a slowly circling camera on it
//! (the moment a nuclear missile lands, or a Commander falls, in slow motion).

use std::sync::Arc;

use egui::{Align2, Color32, Id, LayerId, Order, Rect, RichText, pos2, vec2};
use glam::{Vec2, Vec3};
use ta_ai::SkirmishAi;
use ta_render::{Camera, CameraMode};
use ta_sim::{Sim, SimEvent};
use ta_ui::i18n::{n, t, tf};
use ta_ui::widgets::ActionButton;

use crate::game::Game;
use crate::replay::Record;
use crate::theme::*;

/// Ticks between copies of the game.
const SNAPSHOT_TICKS: u64 = 600;
/// What the copies may take together (bytes, estimated); at least `MIN_SNAPSHOTS` stay, as long
/// as they take no more than `SNAPSHOT_CEILING` (a battle of tens of thousands of units makes
/// copies of a hundred megabytes), and two in any case.
const SNAPSHOT_BUDGET: usize = 160 << 20;
const SNAPSHOT_CEILING: usize = 384 << 20;
const MIN_SNAPSHOTS: usize = 6;
const MAX_SNAPSHOTS: usize = 30;
/// A copy is worth the losses from 4 to 30 seconds after it (the fights it can show).
const WORTH_FROM: u64 = 120;
const WORTH_TO: u64 = 900;
/// Worth added by a nuclear strike and by a fallen Commander (metal).
const NUKE_WORTH: f32 = 6000.0;
const COMMANDER_WORTH: f32 = 5000.0;
/// Losses this close (world units, ticks) belong to one fight.
const FIGHT_RADIUS: f32 = 520.0;
const FIGHT_TICKS: u64 = 450;
/// The most moments shown (the costliest fights), and the least a fight must cost.
const MAX_MOMENTS: usize = 7;
const MIN_WORTH: f32 = 350.0;
/// Speed of the slow-motion parts.
const SLOW_MOTION: f32 = 0.35;

/// A copy of the game.
pub struct Snapshot {
    pub tick: u64,
    pub sim: Sim,
    pub ais: Vec<SkirmishAi>,
    /// The losses in the fights it can show (see `WORTH_FROM`).
    worth: f32,
}

/// A unit lost: when, where, its worth (metal, and energy at 60 to 1).
#[derive(Clone, Copy, Debug)]
struct Loss {
    tick: u64,
    pos: Vec3,
    worth: f32,
    kind: MomentKind,
}

/// What a moment shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MomentKind {
    Battle,
    Nuke,
    Commander,
}

/// One of the battle's decisive moments.
#[derive(Clone, Debug)]
pub struct Moment {
    pub start: u64,
    pub end: u64,
    /// Where the camera looks, and how far out (the fight's spread).
    pub focus: Vec3,
    pub spread: f32,
    pub kind: MomentKind,
    /// Units lost in it, and their worth.
    pub units: u32,
    pub worth: f32,
    /// Ticks played in slow motion.
    pub slow: Option<(u64, u64)>,
    /// The biggest battle of all.
    pub biggest: bool,
}

/// The battle's losses and copies (see the module documentation).
pub struct Tally {
    /// Off for the menu's backdrop and for the highlights themselves.
    pub enabled: bool,
    losses: Vec<Loss>,
    pub snapshots: Vec<Snapshot>,
    /// Who destroyed what, for the battle's result (`result.rs`).
    pub chronicle: crate::chronicle::Chronicle,
    /// Memory is short (`app/memory_guard.rs`): no more copies kept than the two newest.
    lean: bool,
}

impl Default for Tally {
    fn default() -> Self {
        Tally { enabled: true, losses: Vec::new(), snapshots: Vec::new(), chronicle: Default::default(), lean: false }
    }
}

impl Tally {
    /// Notes a simulation event.
    pub fn note(&mut self, e: &SimEvent, sim: &Sim) {
        if !self.enabled {
            return;
        }
        self.chronicle.note(e, sim);
        let (pos, worth, kind) = match e {
            SimEvent::UnitDestroyed { type_id, pos, .. } => {
                let t = sim.types.get(*type_id);
                if t.def.is_feature {
                    return;
                }
                let worth = t.cost_metal.to_f32() + t.cost_energy.to_f32() / 60.0;
                if t.def.is_commander { (pos, worth + COMMANDER_WORTH, MomentKind::Commander) } else { (pos, worth, MomentKind::Battle) }
            }
            SimEvent::Explosion { weapon, pos, .. } => {
                let w = sim.weapon_types.get(*weapon);
                if !w.def.stockpile || w.def.interceptor {
                    return;
                }
                (pos, NUKE_WORTH, MomentKind::Nuke)
            }
            _ => return,
        };
        let loss = Loss { tick: sim.tick, pos: Vec3::from_array(pos.to_f32()), worth, kind };
        for s in &mut self.snapshots {
            if loss.tick >= s.tick + WORTH_FROM && loss.tick <= s.tick + WORTH_TO {
                s.worth += worth;
            }
        }
        self.losses.push(loss);
    }

    /// After a tick: every `SNAPSHOT_TICKS`, a copy of the game, and the least useful copies
    /// dropped beyond the budget.
    pub fn after_tick(&mut self, sim: &Sim, ais: &[SkirmishAi]) {
        if !self.enabled || sim.tick == 0 || !sim.tick.is_multiple_of(SNAPSHOT_TICKS) || self.snapshots.last().is_some_and(|s| s.tick >= sim.tick) {
            return;
        }
        self.snapshots.push(Snapshot { tick: sim.tick, sim: sim.clone(), ais: ais.to_vec(), worth: 0.0 });
        // (The copy's shared terrain costs nothing; its cells' occupants, its units.)
        let bytes = (sim.world.width * sim.world.height) as usize * 30 + sim.unit_count() * 4096 + (1 << 20);
        let least = MIN_SNAPSHOTS.min(SNAPSHOT_CEILING / bytes).max(2);
        let cap = if self.lean { 2 } else { (SNAPSHOT_BUDGET / bytes).clamp(least, MAX_SNAPSHOTS) };
        while self.snapshots.len() > cap {
            // The newest are still gathering their worth.
            let settled = self.snapshots.len() - 2;
            let Some(worst) = (0..settled).min_by(|&a, &b| self.snapshots[a].worth.total_cmp(&self.snapshots[b].worth)) else { break };
            // (Let go on a thread of its own: a copy of a big battle takes tens of milliseconds
            // to free.)
            let gone = self.snapshots.remove(worst);
            std::thread::spawn(move || drop(gone));
        }
    }

    /// Memory is short: the copies of the game let go but the two newest (they are the most the
    /// battle keeps, and the highlights only lose their earlier moments), and no more kept from
    /// now on; how many went. Freed on a thread of their own.
    pub fn shed(&mut self) -> usize {
        self.lean = true;
        let n = self.snapshots.len().saturating_sub(2);
        let gone: Vec<Snapshot> = self.snapshots.drain(..n).collect();
        if !gone.is_empty() {
            std::thread::spawn(move || drop(gone));
        }
        n
    }

    /// Losses per minute and the copies kept (for the log).
    pub fn summary(&self) -> String {
        let mut minutes: std::collections::BTreeMap<u64, (u32, f32)> = Default::default();
        for l in &self.losses {
            let e = minutes.entry(l.tick / 1800).or_default();
            e.0 += 1;
            e.1 += l.worth;
        }
        let losses: Vec<String> = minutes.iter().map(|(m, (n, w))| format!("{m}:{n}/{w:.0}")).collect();
        let copies: Vec<String> = self.snapshots.iter().map(|s| format!("{}:{:.0}", s.tick / 30, s.worth)).collect();
        format!("losses by minute (units/worth) {}; copies (second:worth) {}", losses.join(" "), copies.join(" "))
    }

    /// Whether the battle has anything to show.
    pub fn has_moments(&self) -> bool {
        self.enabled && !self.losses.is_empty() && !self.snapshots.is_empty()
    }

    /// The battle's decisive moments, in the order they happened (only those a copy of the
    /// game comes before).
    pub fn moments(&self) -> Vec<Moment> {
        find_moments(&self.losses, self.snapshots.first().map_or(u64::MAX, |s| s.tick), MAX_MOMENTS)
    }

    /// Its `most` costliest moments from its beginning, in the order they happened: a replay's
    /// marks on its timeline (`replay_scan.rs`).
    pub fn marks(&self, most: usize) -> Vec<Moment> {
        find_moments(&self.losses, 0, most)
    }
}

/// The decisive moments among `losses` after `first_copy` (see [`Tally::moments`]), `most` at
/// the most.
fn find_moments(losses: &[Loss], first_copy: u64, most: usize) -> Vec<Moment> {
    let mut out = fights(losses.iter().copied().filter(|l| l.tick >= first_copy.saturating_add(60)).collect(), most);
    // In the order they happened; one fight found twice is merged, and of two at once in
    // different places the earlier gives way (or the lesser, if it would be too short).
    out.sort_by_key(|m| m.start);
    let mut merged: Vec<Moment> = Vec::new();
    for m in out {
        match merged.last_mut() {
            Some(last) if m.start <= last.end + 90 && m.focus.distance(last.focus) < FIGHT_RADIUS * 2.0 => {
                last.end = last.end.max(m.end).min(last.start + 30 * 30);
                last.units += m.units;
                last.worth += m.worth;
                if m.kind != MomentKind::Battle && last.kind == MomentKind::Battle {
                    last.kind = m.kind;
                    last.slow = m.slow;
                }
            }
            Some(last) if m.start < last.end => {
                if m.start >= last.start + 6 * 30 {
                    last.end = m.start;
                    merged.push(m);
                } else if m.worth > last.worth {
                    *last = m;
                }
            }
            _ => merged.push(m),
        }
    }
    if let Some(top) = merged.iter_mut().filter(|m| m.kind == MomentKind::Battle).max_by(|a, b| a.worth.total_cmp(&b.worth)) {
        top.biggest = true;
    }
    merged
}

/// The costliest fights among `left` (`most` at most, the costliest first): the loss with the
/// most worth lost around it and those losses, then the same among the rest.
fn fights(left: Vec<Loss>, most: usize) -> Vec<Moment> {
    let mut out: Vec<Moment> = Vec::new();
    let near = |a: &Loss, b: &Loss| a.tick.abs_diff(b.tick) <= FIGHT_TICKS && a.pos.distance(b.pos) <= FIGHT_RADIUS;
    // The losses by box of a fight's reach in place and time: those near one lie in the boxes
    // round its own (a battle of tens of thousands of units loses as many).
    let key = |l: &Loss| ((l.pos.x / FIGHT_RADIUS).floor() as i32, (l.pos.z / FIGHT_RADIUS).floor() as i32, (l.tick / FIGHT_TICKS) as i64);
    let mut boxes: rustc_hash::FxHashMap<(i32, i32, i64), Vec<usize>> = Default::default();
    for (i, l) in left.iter().enumerate() {
        boxes.entry(key(l)).or_default().push(i);
    }
    let round = |(x, z, t): (i32, i32, i64)| (-1..=1).flat_map(move |dt| (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (x + dx, z + dz, t + dt))));
    let mut alive = vec![true; left.len()];
    // Each loss's worth lost around it (summed in order), worked out again only for those
    // near a fight taken out (all of them, every time, took seconds in a big battle).
    let mut around: Vec<usize> = Vec::new();
    let mut score = |i: usize, alive: &[bool]| {
        around.clear();
        for b in round(key(&left[i])) {
            around.extend(boxes.get(&b).into_iter().flatten().copied().filter(|&j| alive[j] && near(&left[i], &left[j])));
        }
        around.sort_unstable();
        around.iter().map(|&j| left[j].worth).sum::<f32>()
    };
    let mut scores: Vec<f32> = (0..left.len()).map(|i| score(i, &alive)).collect();
    while out.len() < most {
        // The loss with the most worth lost around it...
        let Some((seed, worth)) = (0..left.len()).filter(|&i| alive[i]).map(|i| (i, scores[i])).max_by(|a, b| a.1.total_cmp(&b.1)) else { break };
        if worth < MIN_WORTH {
            break;
        }
        // ...and the fight: the losses around it (in their order).
        let centre = left[seed];
        let mut taken: Vec<usize> = round(key(&centre)).flat_map(|b| boxes.get(&b).into_iter().flatten().copied()).filter(|&j| alive[j] && near(&centre, &left[j])).collect();
        taken.sort_unstable();
        for &j in &taken {
            alive[j] = false;
        }
        let fight: Vec<Loss> = taken.iter().map(|&j| left[j]).collect();
        out.push(moment_of(&fight, centre));
        // Those that had one of the fight's losses near them, scored again.
        let mut again: Vec<usize> = taken.iter().flat_map(|&j| round(key(&left[j]))).flat_map(|b| boxes.get(&b).into_iter().flatten().copied()).filter(|&i| alive[i]).collect();
        again.sort_unstable();
        again.dedup();
        for i in again {
            scores[i] = score(i, &alive);
        }
    }
    out
}

/// The moment showing `fight` (around `centre`, the loss it was found from).
fn moment_of(fight: &[Loss], centre: Loss) -> Moment {
    let worth: f32 = fight.iter().map(|l| l.worth).sum();
    let weight = fight.iter().map(|l| l.worth.max(1.0)).sum::<f32>();
    let focus = fight.iter().map(|l| l.pos * l.worth.max(1.0)).sum::<Vec3>() / weight;
    let spread = fight.iter().map(|l| Vec2::new(l.pos.x - focus.x, l.pos.z - focus.z).length()).fold(0.0f32, f32::max);
    let first = fight.iter().map(|l| l.tick).min().unwrap_or(centre.tick);
    let last = fight.iter().map(|l| l.tick).max().unwrap_or(centre.tick);
    // A special loss leads: the strike or the fall, and slow motion around it.
    let special = fight.iter().filter(|l| l.kind != MomentKind::Battle).max_by(|a, b| a.worth.total_cmp(&b.worth)).copied();
    let (kind, start, end, slow) = match special {
        Some(s) if s.kind == MomentKind::Nuke => (s.kind, s.tick.saturating_sub(7 * 30), s.tick + 5 * 30, Some((s.tick.saturating_sub(40), s.tick + 60))),
        Some(s) => (s.kind, s.tick.saturating_sub(8 * 30), s.tick + 4 * 30, Some((s.tick.saturating_sub(30), s.tick + 60))),
        None => {
            // Four seconds before the first loss to two after the last, at most 16 seconds
            // (around the heaviest part).
            let (mut start, mut end) = (first.saturating_sub(4 * 30), last + 2 * 30);
            if end - start > 16 * 30 {
                start = centre.tick.saturating_sub(9 * 30).max(start);
                end = start + 16 * 30;
            }
            (MomentKind::Battle, start, end.max(start + 9 * 30), None)
        }
    };
    Moment { start, end, focus, spread: spread.clamp(120.0, 700.0), kind, units: fight.iter().filter(|l| l.kind != MomentKind::Nuke).count() as u32, worth, slow, biggest: false }
}

/// Where the reel is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Going to the moment: back to the copy before it.
    Cue,
    /// Fast-forwarding to its start.
    Seeking,
    /// Playing it (seconds in).
    Playing(f32),
    Finished,
}

/// What the viewer asked for from the reel's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReelAction {
    None,
    Again,
    FullReplay,
    Leave,
}

/// Highlights being watched.
pub struct Reel {
    moments: Vec<Moment>,
    snapshots: Vec<Snapshot>,
    pub record: Arc<Record>,
    index: usize,
    phase: Phase,
    /// The camera's orbit around the moment.
    yaw: f32,
    /// The interface's clock when the current moment began to play (its title card fades
    /// out after it).
    card_since: Option<f64>,
    skip: bool,
    pub action: ReelAction,
}

impl Reel {
    pub fn new(mut moments: Vec<Moment>, snapshots: Vec<Snapshot>, record: Arc<Record>) -> Reel {
        // The deciding loss often comes on the record's final tick. Its normal outro
        // extends beyond that tick, which playback cannot reach: without this bound
        // the last highlight never advances to the end panel.
        // (Nor starts before the earliest copy kept, and a second on: WATCH AGAIN goes back to
        // a copy, and one too late skipped the moment.)
        if let Some(first) = snapshots.iter().map(|s| s.tick).min() {
            for moment in &mut moments {
                moment.start = moment.start.max(first + 30);
            }
        }
        bound_moments(&mut moments, record.end_tick);
        Reel { moments, snapshots, record, index: 0, phase: Phase::Cue, yaw: 0.0, card_since: None, skip: false, action: ReelAction::None }
    }

    /// From the first moment again.
    pub fn restart(&mut self) {
        self.index = 0;
        self.phase = Phase::Cue;
        self.action = ReelAction::None;
    }

    /// Straight to the end (and its choices).
    pub fn skip_to_end(&mut self, game: &mut Game) {
        if self.phase != Phase::Finished {
            self.index = self.moments.len();
            self.phase = Phase::Finished;
            game.set_pace(Some(true), Some(1.0));
        }
    }

    /// A frame of the reel (not while the replay fast-forwards): goes to each moment in turn
    /// and flies the camera around it.
    pub fn update(&mut self, game: &mut Game, camera: &mut Camera, renderer: &mut ta_render::Renderer, dt: f32) {
        let Some(m) = self.moments.get(self.index).cloned() else {
            self.phase = Phase::Finished;
            return;
        };
        match self.phase {
            Phase::Finished => {}
            Phase::Cue => {
                // The latest copy before the moment; none needed when the replay is already
                // between it and the moment.
                let copy = self.snapshots.iter().rev().find(|s| s.tick + 30 <= m.start);
                let here = game.sim.tick;
                match copy {
                    Some(s) if here > m.start || here < s.tick => game.restore(&s.sim, &s.ais, s.tick, Some(&self.record), renderer),
                    None if here > m.start => {
                        // Nothing to go back to (cannot happen: moments come after a copy).
                        self.next(game);
                        return;
                    }
                    _ => {}
                }
                game.seek_replay(m.start);
                game.set_pace(Some(false), Some(1.0));
                self.card_since = None;
                self.phase = Phase::Seeking;
            }
            Phase::Seeking => {
                // Arrived: the camera is there as the moment begins.
                self.yaw = [0.5f32, -0.9, 2.2, -2.4, 1.3, -0.2, 2.9][self.index % 7];
                camera.mode = CameraMode::Perspective;
                camera.focus = m.focus;
                camera.yaw = self.yaw;
                camera.pitch = 0.72;
                camera.zoom = zoom_for(camera, m.spread);
                self.phase = Phase::Playing(0.0);
            }
            // Paused: still, until NEXT.
            Phase::Playing(_) if game.paused => {
                if std::mem::take(&mut self.skip) {
                    game.set_pace(Some(false), None);
                    self.next(game);
                }
            }
            Phase::Playing(clock) => {
                let clock = clock + dt;
                self.phase = Phase::Playing(clock);
                // A slow circle around the fight, drifting to where it is heaviest.
                self.yaw += dt * 0.045;
                let k = 1.0 - (-dt * 1.5).exp();
                camera.yaw += (self.yaw - camera.yaw) * k;
                camera.focus = camera.focus.lerp(m.focus, k * 0.5);
                camera.zoom += (zoom_for(camera, m.spread) - camera.zoom) * k;
                let slow = m.slow.is_some_and(|(a, b)| (a..=b).contains(&game.sim.tick));
                let speed = if slow { SLOW_MOTION } else { 1.0 };
                if (game.speed - speed).abs() > 1e-3 {
                    game.set_pace(None, Some(speed));
                }
                if game.sim.tick >= m.end || std::mem::take(&mut self.skip) {
                    self.next(game);
                }
            }
        }
    }

    fn next(&mut self, game: &mut Game) {
        self.index += 1;
        if self.index >= self.moments.len() {
            self.phase = Phase::Finished;
            game.set_pace(Some(true), Some(1.0));
        } else {
            self.phase = Phase::Cue;
        }
    }

    /// Draws the reel: the letterbox, the moment's title and caption, the buttons, and at
    /// the end the way on.
    pub fn draw(&mut self, ctx: &egui::Context, game: &Game) {
        let now = ctx.input(|i| i.time);
        let screen = ctx.content_rect();
        let bar = (screen.height() * 0.075).round();
        let painter = ctx.layer_painter(LayerId::new(Order::Background, Id::new("reel_bars")));
        painter.rect_filled(Rect::from_min_size(screen.min, vec2(screen.width(), bar)), 0.0, Color32::BLACK);
        painter.rect_filled(Rect::from_min_max(pos2(screen.min.x, screen.max.y - bar), screen.max), 0.0, Color32::BLACK);
        let count = self.moments.len();
        if self.phase == Phase::Finished {
            self.end_panel(ctx, count);
            return;
        }
        let Some(m) = self.moments.get(self.index) else { return };
        let (title, detail) = describe(m);
        // The title card: while the replay fast-forwards, then fading as the moment plays.
        let seeking = matches!(self.phase, Phase::Cue | Phase::Seeking) || game.catch_up_progress().is_some();
        if seeking {
            self.card_since = None;
        }
        let since = *self.card_since.get_or_insert(now);
        let card = if seeking { 1.0 } else { (1.0 - ((now - since) as f32 - 1.0) / 0.6).clamp(0.0, 1.0) };
        if card > 0.0 {
            let veil = ctx.layer_painter(LayerId::new(Order::Background, Id::new("reel_veil")));
            let alpha = if seeking { 1.0 } else { card };
            veil.rect_filled(screen.shrink2(vec2(0.0, bar)), 0.0, Color32::from_black_alpha((235.0 * alpha) as u8));
            egui::Area::new(Id::new("reel_card")).anchor(Align2::CENTER_CENTER, vec2(0.0, -20.0)).order(Order::Foreground).interactable(false).show(ctx, |ui| {
                ui.set_opacity(card);
                ui.vertical_centered(|ui| {
                    let header = tf("HIGHLIGHT {n} OF {count}", &[("n", &(self.index + 1)), ("count", &count)]);
                    ui.label(RichText::new(header).font(strong(14.0)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(2.0)));
                    ui.add_space(6.0);
                    ta_ui::panel::glow_title(ui, title, 46.0, if m.kind == MomentKind::Battle { AMBER } else { RED });
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("{} · {detail}", clock(m.start))).font(strong(15.0)).color(TEXT).extra_letter_spacing(ta_ui::i18n::tracking(0.6)));
                    if let Some(p) = game.catch_up_progress() {
                        ui.add_space(14.0);
                        ta_ui::panel::progress_bar(ui, p, 260.0);
                    }
                });
            });
        }
        // The caption, low on the left over a shadow, and the way out, top right.
        if !seeking {
            let shade = Rect::from_min_max(pos2(screen.min.x, screen.max.y - bar - 150.0), pos2(screen.min.x + 760.0, screen.max.y - bar));
            let mut mesh = egui::Mesh::default();
            let dark = Color32::from_black_alpha((150.0 * (1.0 - card)) as u8);
            for (p, c) in [(shade.left_top(), Color32::TRANSPARENT), (shade.right_top(), Color32::TRANSPARENT), (shade.left_bottom(), dark), (shade.right_bottom(), Color32::TRANSPARENT)] {
                mesh.colored_vertex(p, c);
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(1, 3, 2);
            painter.add(egui::Shape::mesh(mesh));
            egui::Area::new(Id::new("reel_caption")).anchor(Align2::LEFT_BOTTOM, vec2(28.0, -bar - 18.0)).order(Order::Foreground).interactable(false).show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                ui.set_opacity(1.0 - card);
                ui.label(RichText::new(tf("HIGHLIGHT {n} OF {count}", &[("n", &(self.index + 1)), ("count", &count)])).font(strong(12.0)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(1.6)));
                ui.label(RichText::new(t(title)).font(display(30.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
                ui.label(RichText::new(format!("{} · {detail}", clock(game.sim.tick))).font(strong(13.0)).color(TEXT));
            });
        }
        egui::Area::new(Id::new("reel_buttons")).anchor(Align2::RIGHT_TOP, vec2(-18.0, bar + 14.0)).order(Order::Foreground).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.add(ActionButton::secondary(if self.index + 1 < count { "NEXT ›" } else { "FINISH" })).clicked() {
                    self.skip = true;
                }
                if ui.add(ActionButton::secondary("LEAVE")).clicked() {
                    self.action = ReelAction::Leave;
                }
            });
        });
    }

    fn end_panel(&mut self, ctx: &egui::Context, count: usize) {
        use ta_ui::panel::{self, Panel};
        let subtitle = if count == 1 { t("ONE MOMENT OF THE BATTLE").to_string() } else { tf("{count} MOMENTS OF THE BATTLE", &[("count", &count)]) };
        panel::modal(ctx, "reel_end", Panel::new("END OF HIGHLIGHTS").subtitle(&subtitle).width(420.0), false, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let width = ui.available_width();
            if ui.add(ActionButton::primary("WATCH AGAIN").size(width, 38.0)).clicked() {
                self.action = ReelAction::Again;
            }
            if ui.add(ActionButton::secondary("WATCH THE WHOLE BATTLE").size(width, 38.0)).clicked() {
                self.action = ReelAction::FullReplay;
            }
            if ui.add(ActionButton::secondary("MAIN MENU").size(width, 38.0)).clicked() {
                self.action = ReelAction::Leave;
            }
        });
    }
}

fn bound_moments(moments: &mut Vec<Moment>, end_tick: u64) {
    for moment in moments.iter_mut() {
        moment.end = moment.end.min(end_tick);
        moment.slow = moment.slow.and_then(|(start, end)| {
            let (start, end) = (start.max(moment.start), end.min(moment.end));
            (start <= end).then_some((start, end))
        });
    }
    moments.retain(|moment| moment.start < moment.end);
}

/// Zoom that frames a fight `spread` world units across.
pub(crate) fn zoom_for(camera: &Camera, spread: f32) -> f32 {
    (camera.viewport.y / (spread * 3.2)).clamp(0.55, 2.4)
}

/// Worth lost (metal) that makes a fight heavy fighting rather than a skirmish.
const HEAVY: f32 = 1500.0;

/// A moment's title and what it cost.
pub(crate) fn describe(m: &Moment) -> (&'static str, String) {
    let title = match m.kind {
        MomentKind::Nuke => n("NUCLEAR STRIKE"),
        MomentKind::Commander => n("COMMANDER DOWN"),
        MomentKind::Battle if m.biggest && m.worth >= HEAVY => n("THE BIGGEST BATTLE"),
        MomentKind::Battle if m.worth >= HEAVY => n("HEAVY FIGHTING"),
        MomentKind::Battle => n("SKIRMISH"),
    };
    let worth = (m.worth.min(1.0e7) as u32 / 10) * 10;
    let detail = if m.units == 1 {
        tf("1 unit lost · {metal} metal", &[("metal", &worth)])
    } else {
        tf("{units} units lost · {metal} metal", &[("units", &m.units), ("metal", &worth)])
    };
    (title, detail)
}

/// "08:41" of battle time.
pub(crate) fn clock(tick: u64) -> String {
    let s = tick / 30;
    format!("{:02}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loss(tick: u64, x: f32, worth: f32) -> Loss {
        Loss { tick, pos: Vec3::new(x, 0.0, 0.0), worth, kind: MomentKind::Battle }
    }

    #[test]
    fn deciding_losses_finish_the_highlight_at_the_recorded_end() {
        for kind in [MomentKind::Battle, MomentKind::Commander, MomentKind::Nuke] {
            let deciding = Loss { kind, ..loss(9000, 0.0, 6000.0) };
            let mut moments = vec![moment_of(&[deciding], deciding)];
            assert!(moments[0].end > 9000, "normal outros extend past the loss");
            bound_moments(&mut moments, 9000);
            assert_eq!(moments.len(), 1);
            let moment = &moments[0];
            assert_eq!(moment.end, 9000, "the replay must be able to reach the highlight's end");
            assert!(moment.start < moment.end);
            assert!(moment.slow.is_none_or(|(a, b)| a >= moment.start && b <= moment.end));
            bound_moments(&mut moments, 0);
            assert!(moments.is_empty(), "nothing may seek past a zero-length record");
        }
    }

    #[test]
    fn fights_become_moments_in_the_order_they_happened() {
        let losses = vec![
            // A skirmish at x = 0 early on...
            loss(1000, 0.0, 200.0),
            loss(1050, 30.0, 250.0),
            // ...a big battle far away later...
            loss(6000, 4000.0, 900.0),
            loss(6100, 4050.0, 800.0),
            loss(6200, 3990.0, 700.0),
            // ...and a nuclear strike at the end.
            Loss { tick: 9000, pos: Vec3::new(2000.0, 0.0, 0.0), worth: NUKE_WORTH, kind: MomentKind::Nuke },
            // A lone loss is not worth showing.
            loss(3000, -3000.0, 5.0),
        ];
        // Without copies nothing can be shown; with one at the start, everything after it.
        let tally = Tally { losses: losses.clone(), ..Default::default() };
        assert!(tally.moments().is_empty());
        let moments = find_moments(&losses, 600, MAX_MOMENTS);
        assert_eq!(moments.len(), 3, "{moments:?}");
        assert_eq!(moments.iter().map(|m| m.kind).collect::<Vec<_>>(), [MomentKind::Battle, MomentKind::Battle, MomentKind::Nuke]);
        assert!(moments.windows(2).all(|w| w[0].end <= w[1].start), "in order, apart");
        assert!(moments[1].biggest && !moments[0].biggest);
        assert!((moments[1].focus.x - 4013.0).abs() < 30.0, "the battle's heart: {}", moments[1].focus.x);
        let nuke = &moments[2];
        assert!(nuke.start < 9000 && nuke.end > 9000 && nuke.slow.is_some_and(|(a, b)| a < 9000 && b > 9000));
    }

    /// The fights found by box are those a look at every loss round every loss finds, and a
    /// battle of tens of thousands of losses takes moments to go through.
    #[test]
    fn many_losses_make_the_same_fights() {
        let mut rng = ta_sim::rng::Rng::new(11);
        let mut losses: Vec<Loss> = (0..3000)
            .map(|_| Loss { tick: rng.range_i32(0, 20_000) as u64, pos: Vec3::new(rng.range_i32(0, 6000) as f32, 0.0, rng.range_i32(0, 6000) as f32), worth: rng.range_i32(1, 400) as f32, kind: MomentKind::Battle })
            .collect();
        // (As they were found before: every loss's neighbourhood summed over them all.)
        let mut left = losses.clone();
        let near = |a: &Loss, b: &Loss| a.tick.abs_diff(b.tick) <= FIGHT_TICKS && a.pos.distance(b.pos) <= FIGHT_RADIUS;
        let mut expected = Vec::new();
        while expected.len() < MAX_MOMENTS && !left.is_empty() {
            let score = |i: usize| left.iter().filter(|l| near(&left[i], l)).map(|l| l.worth).sum::<f32>();
            let Some((seed, worth)) = (0..left.len()).map(|i| (i, score(i))).max_by(|a, b| a.1.total_cmp(&b.1)) else { break };
            if worth < MIN_WORTH {
                break;
            }
            let centre = left[seed];
            let (fight, rest): (Vec<Loss>, Vec<Loss>) = left.iter().partition(|l| near(&centre, l));
            left = rest;
            expected.push(moment_of(&fight, centre));
        }
        let found = fights(losses.clone(), MAX_MOMENTS);
        assert_eq!(found.len(), expected.len());
        for (f, e) in found.iter().zip(&expected) {
            assert_eq!((f.start, f.end, f.units, f.worth), (e.start, e.end, e.units, e.worth));
        }
        losses.extend((0..40_000).map(|i| Loss { tick: 30_000 + i as u64 / 4, pos: Vec3::new((i % 200) as f32 * 30.0, 0.0, (i / 200 % 200) as f32 * 30.0), worth: 50.0, kind: MomentKind::Battle }));
        let started = std::time::Instant::now();
        assert!(!find_moments(&losses, 0, MAX_MOMENTS).is_empty());
        assert!(started.elapsed().as_secs_f32() < 20.0, "{:?}", started.elapsed());
    }
}
