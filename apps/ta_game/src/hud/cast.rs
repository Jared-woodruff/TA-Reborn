//! The cast: a broadcast's view of a battle watched (a replay, or spectating). Across the top,
//! the players head to head: their colours, names and sides, the clock between them; the
//! armies' worth as a tug of war (among more players, a bar each); what each makes a second,
//! how many units they have and have destroyed; what their factories are building. Down the
//! right, the notable kills as they come (Commanders, costly units and buildings:
//! `Game::cast_kills`), killer and killed in their colours. The action camera directs
//! (`app.rs`); the replay's timeline marks its big moments (`replay_scan.rs`).

use std::collections::VecDeque;

use egui::{Align2, Color32, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use ta_ui::i18n::{n, t};
use ta_vfs::Vfs;

use super::{Clearance, ENERGY, Hud, METAL, UnitPic};
use crate::game::{Game, KillNote};
use crate::theme::*;

/// The scoreboard's widest; how long a kill stays in the feed (seconds), and how many show.
const BOARD: f32 = 680.0;
const FEED_SECONDS: f64 = 8.0;
const FEED_LINES: usize = 6;
/// Kills older than this (ticks) when the feed takes them up are left out (the cast turned on
/// late, a jump ahead).
const FEED_FRESH: u64 = 150;

/// The feed's kills shown: each with when it came in (the interface's clock).
#[derive(Default)]
pub(super) struct Feed {
    lines: VecDeque<(KillNote, f64)>,
}

/// A player's standing, for the scoreboard (and the companion screen's players).
pub(super) struct Standing {
    pub(super) player: u8,
    pub(super) name: String,
    pub(super) side: String,
    pub(super) colour: Color32,
    pub(super) army: f32,
    pub(super) metal: f32,
    pub(super) energy: f32,
    pub(super) units: u32,
    pub(super) kills: u64,
    pub(super) defeated: bool,
    /// What its factories are building: the kind and how far along, the furthest first.
    pub(super) making: Vec<(usize, f32)>,
}

/// Every player's standing now.
pub(super) fn standings(game: &Game) -> Vec<Standing> {
    let sim = &game.sim;
    let mut out: Vec<Standing> = sim
        .players
        .iter()
        .map(|p| Standing {
            player: p.index,
            name: game.player_label(p.index),
            side: p.side.to_uppercase(),
            colour: crate::palette::team_color(p.color),
            army: 0.0,
            metal: p.economy.metal_income.to_f32(),
            energy: p.economy.energy_income.to_f32(),
            units: 0,
            kills: p.stats.kills,
            defeated: p.defeated,
            making: Vec::new(),
        })
        .collect();
    for u in sim.units() {
        if u.dead || !u.is_complete() {
            continue;
        }
        let Some(s) = out.get_mut(u.owner as usize) else { continue };
        let ty = sim.unit_type(u);
        s.units += 1;
        if ty.locomotion != ta_sim::Locomotion::Static && !ty.def.is_commander && ty.weapons.iter().any(|w| w.is_some()) {
            s.army += ty.cost_metal.to_f32() + ty.cost_energy.to_f32() / 60.0;
        }
        if ty.is_factory && let Some(&kind) = u.factory_queue.front() {
            let done = u.build.and_then(|b| sim.unit(b.target)).map_or(0.0, |b| b.build_progress.to_f32().clamp(0.0, 1.0));
            s.making.push((kind, done));
        }
    }
    for s in &mut out {
        s.making.sort_by(|a, b| b.1.total_cmp(&a.1));
        s.making.truncate(5);
    }
    out
}

/// "12:40" of battle.
fn battle_clock(game: &Game) -> String {
    let s = game.sim.tick / 30;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// A player's colour, lit enough to read on the dark plates.
pub(super) fn readable(c: Color32) -> Color32 {
    c.lerp_to_gamma(Color32::WHITE, 0.38)
}

/// 12,345 as the cast shows a worth or a count.
fn grouped(v: f32) -> String {
    ta_ui::i18n::grouped(v.max(0.0).round() as u64)
}

impl Hud {
    /// The cast's scoreboard and kill feed (when the cast is on).
    pub(super) fn cast(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, clear: Clearance) {
        self.cast_at(ctx, vfs, game, clear, ctx.input(|i| i.time));
    }

    /// [`Hud::cast`] on a clock of its own (`now`, seconds: a film's, whose pictures are not made
    /// on the interface's clock).
    pub(super) fn cast_at(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, clear: Clearance, now: f64) {
        // The feed takes up the kills that came (the fresh ones), whether or not it shows them.
        let tick = game.sim.tick;
        for kill in game.cast_kills.drain(..) {
            if tick.saturating_sub(kill.tick) <= FEED_FRESH {
                self.feed.lines.push_front((kill, now));
            }
        }
        self.feed.lines.retain(|(_, at)| (0.0..FEED_SECONDS).contains(&(now - at)));
        self.feed.lines.truncate(FEED_LINES);
        if !game.cast {
            self.board_share = None;
            return;
        }
        let players = standings(game);
        let playing: Vec<&Standing> = players.iter().filter(|s| game.setup.players.get(s.player as usize).is_some()).collect();
        self.scoreboard(ctx, vfs, game, &playing, clear);
        self.kill_feed(ctx, vfs, game, clear, now);
        ctx.request_repaint();
    }

    /// The pictures of what `players`' factories build, drawn first (they need the HUD).
    fn making_pictures(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &Game, players: &[&Standing]) -> Vec<(usize, u8, Option<UnitPic>)> {
        let mut pics: Vec<(usize, u8, Option<UnitPic>)> = Vec::new();
        for s in players {
            for &(kind, _) in &s.making {
                if !pics.iter().any(|(k, p, _)| *k == kind && *p == s.player) {
                    pics.push((kind, s.player, self.unit_pic_of(ctx, vfs, game, kind, 22.0, s.player)));
                }
            }
        }
        pics
    }

    /// The players head to head across the top (two), or a row each (more).
    fn scoreboard(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &Game, players: &[&Standing], clear: Clearance) {
        let screen = ctx.content_rect();
        // (Narrower in a small window, to keep clear of the minimap at the top right.)
        let clear_of_map = self.minimap_rect.filter(|_| !self.classic).map_or(f32::MAX, |m| 2.0 * (m.left() - 8.0 - screen.center().x));
        let width = BOARD.min(screen.width() - 32.0).min(clear_of_map).max(420.0);
        let at = pos2(screen.center().x - width / 2.0, screen.top() + clear.top + 8.0);
        let pics = self.making_pictures(ctx, vfs, game, players);
        let pic = |kind: usize, player: u8| pics.iter().find(|(k, p, _)| *k == kind && *p == player).and_then(|(_, _, pic)| *pic);
        let clock = battle_clock(game);
        // (Not faded in when shown: a film's pictures draw it in a pass of their own between the
        // window's, which egui would take for its being shown anew every time.)
        let board = egui::Area::new(egui::Id::new("cast_scoreboard")).order(egui::Order::Middle).fixed_pos(at).fade_in(false).interactable(false).show(ctx, |ui| {
            if players.len() == 2 {
                head_to_head(ui, width, players[0], players[1], &clock, &pic);
            } else {
                table(ui, width, players, &clock, &pic);
            }
        });
        self.board_share = Some(((board.response.rect.bottom() - screen.top()) / screen.height().max(1.0)).clamp(0.0, 1.0));
    }

    /// The notable kills, newest first, down the right under the bar (under the scoreboard,
    /// where the window is too narrow for both side by side).
    fn kill_feed(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &Game, clear: Clearance, now: f64) {
        if self.feed.lines.is_empty() {
            return;
        }
        let lines: Vec<(KillNote, f64)> = self.feed.lines.iter().copied().collect();
        let mut pics: Vec<((usize, u8), Option<UnitPic>)> = Vec::new();
        for (k, _) in &lines {
            for key in [Some((k.victim, k.victim_owner)), k.killer].into_iter().flatten() {
                if !pics.iter().any(|(p, _)| *p == key) {
                    pics.push((key, self.unit_pic_of(ctx, vfs, game, key.0, 26.0, key.1)));
                }
            }
        }
        let pic = |key: (usize, u8)| pics.iter().find(|(p, _)| *p == key).and_then(|(_, pic)| *pic);
        let colour = |player: u8| game.sim.players.get(player as usize).map_or(STEEL, |p| readable(crate::palette::team_color(p.color)));
        let screen = ctx.content_rect();
        let width = 240.0f32.min(screen.width() * 0.3);
        let board = ctx.memory(|m| m.area_rect(egui::Id::new("cast_scoreboard")));
        let top = match board {
            Some(b) if screen.right() - 16.0 - width < b.right() + 8.0 => b.bottom() - screen.top() + 10.0,
            _ => clear.top + clear.right_top + 12.0,
        };
        egui::Area::new(egui::Id::new("cast_feed")).order(egui::Order::Middle).anchor(Align2::RIGHT_TOP, [-16.0, top]).fade_in(false).interactable(false).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for (k, at) in lines {
                let age = now - at;
                let fade = (age / 0.25).min(1.0).min((FEED_SECONDS - age) / 0.6) as f32;
                let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), egui::Sense::hover());
                let p = ui.painter();
                let commander = game.sim.types.get(k.victim).def.is_commander;
                p.rect_filled(rect, 3.0, Color32::from_black_alpha((170.0 * fade) as u8));
                if commander {
                    p.rect_stroke(rect, 3.0, Stroke::new(1.5, AMBER_BRIGHT.gamma_multiply(fade)), StrokeKind::Inside);
                }
                let tile = |p: &egui::Painter, r: Rect, key: Option<(usize, u8)>| {
                    match key.and_then(pic) {
                        Some(pic) => pic.paint(p, r, Color32::WHITE.gamma_multiply(fade)),
                        None => {
                            p.rect_filled(r, 2.0, INSET.gamma_multiply(fade));
                        }
                    }
                    let edge = key.map_or(STEEL_DIM, |(_, owner)| colour(owner));
                    p.rect_stroke(r, 2.0, Stroke::new(1.5, edge.gamma_multiply(fade)), StrokeKind::Outside);
                };
                // Killer, a mark, killed; the killed one's name in its colour.
                let left = Rect::from_min_size(rect.min + vec2(4.0, 3.0), vec2(26.0, 26.0));
                tile(p, left, k.killer);
                let mark = pos2(left.right() + 10.0, rect.center().y);
                p.add(Shape::convex_polygon(vec![mark + vec2(-4.0, -5.0), mark + vec2(4.0, 0.0), mark + vec2(-4.0, 5.0)], STEEL.gamma_multiply(fade), Stroke::NONE));
                let right = Rect::from_min_size(pos2(mark.x + 10.0, left.top()), vec2(26.0, 26.0));
                tile(p, right, Some((k.victim, k.victim_owner)));
                let name = crate::chat::unit_label(&game.sim, k.victim);
                let room = rect.right() - right.right() - 16.0;
                let galley = p.layout(name, strong(if commander { 13.5 } else { 12.5 }), colour(k.victim_owner).gamma_multiply(fade), room);
                p.galley(pos2(right.right() + 9.0, rect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
            }
        });
    }
}

/// Two players head to head: names and sides at either end with the clock between; the armies'
/// worth as a tug of war under them; each one's income, units and kills; what they build.
fn head_to_head(ui: &mut egui::Ui, width: f32, a: &Standing, b: &Standing, clock: &str, pic: &dyn Fn(usize, u8) -> Option<UnitPic>) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 80.0), egui::Sense::hover());
    let p = ui.painter();
    p.add(Shape::rect_filled(rect.translate(vec2(0.0, 3.0)).expand(2.0), 6.0, Color32::from_black_alpha(90)));
    p.add(ta_ui::metal::plate(ui.ctx(), rect, PANEL));
    p.rect_stroke(rect, 4.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
    let pad = 12.0;
    // The players' colours down the ends.
    for (s, x) in [(a, rect.left()), (b, rect.right() - 5.0)] {
        p.rect_filled(Rect::from_min_size(pos2(x, rect.top() + 6.0), vec2(5.0, rect.height() - 12.0)), 1.5, s.colour);
    }
    // Names and sides, the clock between.
    let name_row = rect.top() + 18.0;
    for (s, left) in [(a, true), (b, false)] {
        let name = p.layout_no_wrap(s.name.clone(), strong(16.0), if s.defeated { STEEL_DIM } else { readable(s.colour) });
        let side = p.layout_no_wrap(s.side.clone(), strong(11.0), STEEL);
        let (nx, sx) = if left { (rect.left() + pad + 6.0, rect.left() + pad + 12.0 + name.size().x) } else { (rect.right() - pad - 6.0 - name.size().x, rect.right() - pad - 12.0 - name.size().x - side.size().x) };
        p.galley(pos2(nx, name_row - name.size().y / 2.0), name, Color32::WHITE);
        p.galley(pos2(sx, name_row - side.size().y / 2.0 + 1.0), side, Color32::WHITE);
    }
    p.text(pos2(rect.center().x, name_row), Align2::CENTER_CENTER, clock, display(17.0), TEXT_BRIGHT);
    // The tug of war: each army's share of the two, its worth at its end.
    let bar = Rect::from_min_max(pos2(rect.left() + pad + 6.0, rect.top() + 33.0), pos2(rect.right() - pad - 6.0, rect.top() + 48.0));
    let total = (a.army + b.army).max(1.0);
    let share = if a.army + b.army <= 0.0 { 0.5 } else { a.army / total };
    p.add(ta_ui::metal::well(bar, INSET));
    let inner = bar.shrink(2.0);
    let split = inner.left() + inner.width() * share;
    p.add(ta_ui::metal::vgrad(Rect::from_min_max(inner.min, pos2(split, inner.max.y)), a.colour.lerp_to_gamma(Color32::WHITE, 0.15), a.colour.lerp_to_gamma(Color32::BLACK, 0.35)));
    p.add(ta_ui::metal::vgrad(Rect::from_min_max(pos2(split, inner.min.y), inner.max), b.colour.lerp_to_gamma(Color32::WHITE, 0.15), b.colour.lerp_to_gamma(Color32::BLACK, 0.35)));
    p.vline(split, inner.y_range().expand(2.0), Stroke::new(2.0, Color32::WHITE.gamma_multiply(0.85)));
    let army = t(n("ARMY"));
    p.text(pos2(inner.left() + 6.0, inner.center().y), Align2::LEFT_CENTER, grouped(a.army), strong(12.0), TEXT_BRIGHT);
    p.text(pos2(inner.right() - 6.0, inner.center().y), Align2::RIGHT_CENTER, grouped(b.army), strong(12.0), TEXT_BRIGHT);
    p.text(pos2(bar.center().x, bar.center().y), Align2::CENTER_CENTER, army, strong(10.5), Color32::from_white_alpha(200));
    // Income, units, kills at either end; what they build in from the middle.
    let stats_row = rect.top() + 64.0;
    for (s, left) in [(a, true), (b, false)] {
        let pieces = [
            (format!("+{:.1}", s.metal), METAL),
            (format!("+{:.0}", s.energy), ENERGY),
            (format!("{} {}", s.units, t(n("UNITS"))), TEXT),
            (format!("{} {}", s.kills, t(n("KILLS"))), TEXT),
        ];
        let mut x = if left { rect.left() + pad + 6.0 } else { rect.right() - pad - 6.0 };
        for (text, colour) in pieces {
            let g = p.layout_no_wrap(text, strong(12.0), colour);
            let w = g.size().x;
            let gx = if left { x } else { x - w };
            p.galley(pos2(gx, stats_row - g.size().y / 2.0), g, colour);
            x = if left { x + w + 12.0 } else { x - w - 12.0 };
        }
        // What its factories build, toward the middle.
        let mut px = if left { rect.center().x - 14.0 - 22.0 } else { rect.center().x + 14.0 };
        for &(kind, done) in s.making.iter().take(3) {
            let r = Rect::from_min_size(pos2(px, stats_row - 11.0), vec2(22.0, 22.0));
            match pic(kind, s.player) {
                Some(pic) => pic.paint(p, r, Color32::WHITE),
                None => {
                    p.rect_filled(r, 2.0, INSET);
                }
            }
            p.rect_stroke(r, 1.0, Stroke::new(1.0, s.colour), StrokeKind::Outside);
            p.rect_filled(Rect::from_min_size(pos2(r.left(), r.bottom() - 3.0), vec2(r.width() * done, 3.0)), 0.0, GREEN);
            px = if left { px - 26.0 } else { px + 26.0 };
        }
    }
}

/// Three or more players: a row each under the clock and the columns' headings: colour, name
/// and side, the army's worth as a bar against the largest, metal and energy a second, units,
/// kills, what they build.
fn table(ui: &mut egui::Ui, width: f32, players: &[&Standing], clock: &str, pic: &dyn Fn(usize, u8) -> Option<UnitPic>) {
    let row_h = 28.0;
    let head = 26.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, head + row_h * players.len() as f32 + 8.0), egui::Sense::hover());
    let p = ui.painter();
    p.add(Shape::rect_filled(rect.translate(vec2(0.0, 3.0)).expand(2.0), 6.0, Color32::from_black_alpha(90)));
    p.add(ta_ui::metal::plate(ui.ctx(), rect, PANEL));
    p.rect_stroke(rect, 4.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
    // The columns, as shares of the width.
    let at = |share: f32| rect.left() + width * share;
    let (bar_from, bar_to, worth, metal, energy, units, kills) = (at(0.33), at(0.50), at(0.51), at(0.60), at(0.685), at(0.775), at(0.855));
    let heading = rect.top() + head / 2.0 + 2.0;
    p.text(pos2(rect.left() + 20.0, heading), Align2::LEFT_CENTER, clock, display(16.0), TEXT_BRIGHT);
    for (x, align, word) in [((bar_from + bar_to) / 2.0, Align2::CENTER_CENTER, n("ARMY")), (metal, Align2::LEFT_CENTER, n("METAL")), (energy, Align2::LEFT_CENTER, n("ENERGY")), (units, Align2::LEFT_CENTER, n("UNITS")), (kills, Align2::LEFT_CENTER, n("KILLS"))] {
        p.text(pos2(x, heading), align, t(word), strong(10.0), STEEL);
    }
    let most = players.iter().map(|s| s.army).fold(1.0f32, f32::max);
    for (i, s) in players.iter().enumerate() {
        let top = rect.top() + head + row_h * i as f32;
        let mid = top + row_h / 2.0;
        p.add(ta_ui::metal::seam(rect.x_range().shrink(10.0), top.round()));
        p.rect_filled(Rect::from_min_size(pos2(rect.left() + 8.0, top + 5.0), vec2(5.0, row_h - 10.0)), 1.5, s.colour);
        let name_room = width * 0.20;
        let name = p.layout_no_wrap(s.name.clone(), strong(14.0), if s.defeated { STEEL_DIM } else { readable(s.colour) });
        let name = if name.size().x > name_room { p.layout(s.name.clone(), strong(12.0), readable(s.colour), name_room) } else { name };
        p.galley(pos2(rect.left() + 20.0, mid - name.size().y / 2.0), name, Color32::WHITE);
        p.text(pos2(rect.left() + 20.0 + name_room + 6.0, mid), Align2::LEFT_CENTER, &s.side, strong(10.5), STEEL);
        // The army against the largest.
        let bar = Rect::from_min_max(pos2(bar_from, mid - 6.0), pos2(bar_to, mid + 6.0));
        p.add(ta_ui::metal::well(bar, INSET));
        let fill = bar.shrink(2.0);
        p.rect_filled(Rect::from_min_size(fill.min, vec2(fill.width() * (s.army / most).clamp(0.0, 1.0), fill.height())), 1.0, s.colour);
        p.text(pos2(worth, mid), Align2::LEFT_CENTER, grouped(s.army), strong(11.5), TEXT_BRIGHT);
        for (x, text, colour) in [(metal, format!("+{:.1}", s.metal), METAL), (energy, format!("+{:.0}", s.energy), ENERGY), (units, format!("{}", s.units), TEXT), (kills, format!("{}", s.kills), AMBER)] {
            p.text(pos2(x, mid), Align2::LEFT_CENTER, text, strong(12.0), colour);
        }
        let mut px = rect.right() - 10.0 - 20.0;
        for &(kind, done) in s.making.iter().take(3) {
            let r = Rect::from_min_size(pos2(px, mid - 10.0), vec2(20.0, 20.0));
            match pic(kind, s.player) {
                Some(pic) => pic.paint(p, r, Color32::WHITE),
                None => {
                    p.rect_filled(r, 2.0, INSET);
                }
            }
            p.rect_filled(Rect::from_min_size(pos2(r.left(), r.bottom() - 3.0), vec2(r.width() * done, 3.0)), 0.0, GREEN);
            px -= 24.0;
        }
    }
}
