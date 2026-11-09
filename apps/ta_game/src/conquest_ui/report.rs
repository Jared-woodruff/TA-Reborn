//! What a battle came to, when the galaxy opens again after it (and on REPORT): the system taken
//! or not, how long it took, who came home (the newly ranked among them), who fell, the
//! Commander's rank before and after, and what holding the system brings. And the campaign's
//! end: the galaxy conquered, its tally and its best veterans, a new conquest or the record.

use egui::{Align2, Color32, RichText, Sense, Stroke, Vec2, pos2, vec2};
use ta_ui::i18n::{t, tf};
use ta_ui::panel::{self, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use super::{ConquestAction, ConquestPage, EmpireTab, animation_time, galaxy, metal_steel, page_width, panels, rank_name};
use crate::conquest::{Conquest, Reward, Veteran};
use crate::palette::team_color;
use crate::setup::Assets;
use crate::theme::*;

impl ConquestPage {
    /// The last battle's report, over the galaxy.
    pub(super) fn debrief_modal(&mut self, ctx: &egui::Context, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, c: &Conquest) {
        let Some(r) = c.last.clone() else {
            self.debrief = false;
            return;
        };
        let name = c.systems.get(r.system).map_or(String::new(), |s| s.name.to_uppercase());
        let (verdict, accent) = if r.won { (t("VICTORY"), GREEN) } else { (t("DEFEAT"), RED) };
        let headline = match (r.defence, r.won) {
            (true, true) => tf("{name} HELD", &[("name", &name)]),
            (true, false) => tf("{name} LOST", &[("name", &name)]),
            (false, true) if r.system == c.capital => tf("{name}, THE ENEMY CAPITAL, HAS FALLEN", &[("name", &name)]),
            (false, true) => tf("{name} TAKEN", &[("name", &name)]),
            (false, false) => tf("{name} HOLDS", &[("name", &name)]),
        };
        let entry = c.history.last().filter(|entry| entry.report == r);
        let subtitle = match entry {
            Some(entry) => tf("BATTLE {n} · {duration}", &[("n", &entry.battle), ("duration", &panels::duration(entry.duration_ticks))]),
            None => tf("BATTLE {n}", &[("n", &c.battles)]),
        };
        let room = ctx.content_rect();
        let width = page_width(room.width(), 940.0);
        let page = Panel::new(&subtitle).width(width).max_height(room.height() - 40.0).accent(accent);
        let response = panel::modal_with_footer(ctx, "conquest_debrief", page, true, 44.0, |ui| {
            let short = ui.available_height() < 420.0;
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(verdict).font(display(if short { 34.0 } else { 44.0 })).color(accent));
                w::pretty_label(ui, RichText::new(&headline).font(strong(15.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
            });
            ui.add_space(8.0);
            // The tally: home, newly ranked, fallen.
            let tiles: [(String, String, Color32); 3] = [
                (r.returned.to_string(), t("VETERANS HOME").to_string(), Color32::from_rgb(150, 190, 240)),
                (r.new_veterans.to_string(), t("NEWLY RANKED").to_string(), AMBER_BRIGHT),
                (r.lost.to_string(), t("FALLEN").to_string(), if r.lost > 0 { RED } else { STEEL }),
            ];
            stat_tiles(ui, &tiles);
            ui.add_space(6.0);
            // Who they were beside what it means: two columns where there is room.
            let units = |page: &mut ConquestPage, ui: &mut egui::Ui, portraits: &mut crate::encyclopaedia::Portraits| {
                let Some(d) = &r.details else { return };
                if !d.returned.is_empty() {
                    panel::section(ui, "HOME FROM THE BATTLE");
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                        let mut promoted = d.promoted.clone();
                        for v in &d.returned {
                            let response = page.veteran_portrait(ui, assets, portraits, v, 44.0, false, false);
                            // (The newly ranked: a star on them.)
                            if let Some(k) = promoted.iter().position(|p| p == v) {
                                promoted.remove(k);
                                new_badge(ui.painter(), response.rect);
                                response.on_hover_text(t("Newly ranked in this battle"));
                            }
                        }
                    });
                }
                if !d.fallen.is_empty() {
                    panel::section(ui, "FALLEN");
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                        for v in &d.fallen {
                            let response = page.veteran_portrait(ui, assets, portraits, v, 44.0, true, false);
                            panels::fallen_mark(ui.painter(), response.rect);
                        }
                    });
                }
                if d.returned.is_empty() && d.fallen.is_empty() {
                    panel::section(ui, "THE VETERANS");
                    w::pretty_label(ui, RichText::new(t("The Commander went in alone.")).color(STEEL));
                }
            };
            let meaning = |ui: &mut egui::Ui| {
                if let Some(d) = &r.details {
                    panel::section(ui, "THE COMMANDER");
                    let [before, after] = d.commander;
                    let (was, now) = (Veteran { unit: String::new(), xp: before }.rank(), Veteran { unit: String::new(), xp: after }.rank());
                    let xp = format!("{:.1}", after.to_f64());
                    let line = if !r.won {
                        tf("Rebuilt at the border with half its experience: {rank}, {xp} XP.", &[("rank", &rank_name(now)), ("xp", &xp)])
                    } else if now > was {
                        tf("Promoted to {rank} ({xp} XP).", &[("rank", &rank_name(now)), ("xp", &xp)])
                    } else {
                        tf("{rank}, {xp} XP.", &[("rank", &rank_name(now)), ("xp", &xp)])
                    };
                    w::pretty_label(ui, RichText::new(line).color(if now > was && r.won { AMBER_BRIGHT } else { TEXT }));
                    // A promotion earns a perk, chosen once the report is read.
                    if now > was && c.perks_due() > 0 && !c.won {
                        w::pretty_label(ui, RichText::new(t("Promoted: a perk to choose")).color(AMBER_BRIGHT));
                    }
                }
                panel::section(ui, "THE CAMPAIGN");
                let mut lines: Vec<(String, Color32)> = Vec::new();
                let system = |i: usize| c.systems.get(i).map_or(String::new(), |s| s.name.clone());
                let reward = |i: usize| c.reward(i);
                let gained = |reward: Reward| tf("Gained {reward}: {effect}", &[("reward", &t(reward.label())), ("effect", &panels::reward_effect(assets, c, reward))]);
                let lost = |reward: Reward| tf("Lost {reward}: {effect}", &[("reward", &t(reward.label())), ("effect", &panels::reward_effect(assets, c, reward))]);
                match (r.defence, r.won) {
                    (false, true) if reward(r.system) != Reward::None => lines.push((gained(reward(r.system)), AMBER_BRIGHT)),
                    (true, false) => {
                        lines.push((t("The system has fallen. Retake it, or strike elsewhere.").to_string(), RED));
                        if reward(r.system) != Reward::None {
                            lines.push((lost(reward(r.system)), RED));
                        }
                    }
                    (false, false) => lines.push((t("The system holds. Regroup, choose your veterans and try again, or strike elsewhere.").to_string(), TEXT)),
                    _ => {}
                }
                // What else the war came to as the battle ended.
                if let Some(broken) = r.broken {
                    lines.push((tf("Taking it broke up the enemy's attack on {system}.", &[("system", &system(broken))]), GREEN));
                }
                if let Some(fell) = r.fell {
                    lines.push((tf("{system} fell to the enemy's counter-attack.", &[("system", &system(fell))]), RED));
                    if reward(fell) != Reward::None {
                        lines.push((lost(reward(fell)), RED));
                    }
                }
                lines.push((tf("{held} of {total} systems held", &[("held", &c.held()), ("total", &c.systems.len())]), TEXT));
                if r.won || r.fell.is_some() {
                    let (metal, energy) = c.bonus();
                    lines.push((tf("Supply now +{metal} metal and +{energy} energy at the start of each battle", &[("metal", &metal), ("energy", &energy)]), TEXT));
                }
                if let Some(&[before, after]) = r.details.as_ref().map(|d| &d.berths) {
                    if after > before {
                        lines.push((tf("A new veteran berth: {n} may join each battle", &[("n", &after)]), AMBER_BRIGHT));
                    } else if after < before {
                        lines.push((tf("Berths lost: {n} veterans may join each battle", &[("n", &after)]), RED));
                    }
                }
                if r.won && !c.won {
                    match c.remaining_jumps() {
                        1 => lines.push((t("The enemy capital is within reach").to_string(), AMBER)),
                        n => lines.push((tf("The capital is {n} battles away", &[("n", &n)]), AMBER)),
                    }
                }
                for (line, color) in lines {
                    w::pretty_label(ui, RichText::new(line).color(color));
                }
                // The enemy strikes back.
                if let Some(threat) = r.threat {
                    panel::section(ui, "COUNTER-ATTACK");
                    w::pretty_label(ui, RichText::new(tf("The enemy strikes back at {system}.", &[("system", &system(threat))])).color(RED));
                    if let Some(counter) = c.counter.filter(|counter| counter.system == threat) {
                        w::pretty_label(ui, RichText::new(panels::counter_when(c, counter)).color(TEXT_BRIGHT));
                        w::pretty_label(ui, RichText::new(t("Beat it off in a battle there, or take the system it strikes from first.")).color(STEEL));
                    }
                }
            };
            if ui.available_width() >= 700.0 {
                // (Each column laid out from its left: in egui's own, justified, a line that
                // wraps had its first line stretched across, its letters spread apart.)
                let plain = egui::Layout::top_down(egui::Align::Min);
                w::columns(ui, 2, |columns| {
                    columns[0].with_layout(plain, |ui| meaning(ui));
                    columns[1].with_layout(plain, |ui| units(self, ui, portraits));
                });
            } else {
                units(self, ui, portraits);
                meaning(ui);
            }
        }, |ui| {
            // CONTINUE (or Enter), in sight however the report scrolls.
            let continued = ui.vertical_centered(|ui| ui.add(ActionButton::primary("CONTINUE").size(220.0, 44.0)).clicked()).inner;
            continued || ui.input_mut(|inp| inp.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        });
        if response.inner.1 || response.exit {
            self.debrief = false;
            // (A system taken rings out on the map.)
            if r.won && !r.defence && !c.won {
                self.capture = Some((r.system, ctx.input(|i| i.time)));
            }
        }
    }

    /// The galaxy conquered: our emblem and the tally of the campaign, its best veterans, and
    /// what next.
    pub(super) fn campaign_complete(&mut self, ui: &mut egui::Ui, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, c: &Conquest) -> ConquestAction {
        let mut action = ConquestAction::None;
        let ctx = ui.ctx().clone();
        let screen = ui.max_rect();
        let time = animation_time(&ctx);
        galaxy::paint_space(ui.painter(), screen, time);
        ta_ui::motion::next_frame(&ctx, std::time::Duration::from_millis(33));
        let ours = team_color(0);
        let width = page_width(screen.width(), 760.0);
        // (A short window: a smaller emblem and title, so that all of it shows.)
        let short = screen.height() < 640.0;
        egui::Area::new(egui::Id::new("conquest_complete")).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(&ctx, |ui| {
            ui.set_width(width);
            ui.vertical_centered(|ui| {
                // Our emblem in a glow of our colour.
                let size = if short { (screen.height() * 0.13).clamp(56.0, 90.0) } else { (screen.height() * 0.17).clamp(90.0, 200.0) };
                let (emblem, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
                let pulse = 0.85 + 0.15 * (time * 1.3).sin();
                w::soft_glow(ui.painter(), emblem.center(), 0.0, screen.height() * 0.32, Color32::from_rgba_unmultiplied(ours.r(), ours.g(), ours.b(), (70.0 * pulse) as u8));
                self.paint_emblem(ui, assets, &c.side, emblem, metal_steel().lerp_to_gamma(ours, 0.85), 1.0);
                ui.add_space(if short { 4.0 } else { 14.0 });
                ui.label(RichText::new(t("GALAXY CONQUERED")).font(display(if short { 40.0 } else { 52.0 })).color(AMBER_BRIGHT));
                w::pretty_label(ui, RichText::new(tf("The enemy capital has fallen: the {side} rule the galaxy.", &[("side", &c.side)])).font(strong(14.0)).color(TEXT_BRIGHT));
            });
            ui.add_space(if short { 10.0 } else { 16.0 });
            let tiles: [(String, String, Color32); 6] = [
                (format!("{} / {}", c.held(), c.systems.len()), t("SYSTEMS").to_string(), Color32::from_rgb(150, 190, 240)),
                (c.battles.to_string(), t("BATTLES").to_string(), TEXT_BRIGHT),
                (c.wins.to_string(), t("VICTORIES").to_string(), GREEN),
                (c.battles.saturating_sub(c.wins).to_string(), t("DEFEATS").to_string(), if c.battles > c.wins { RED } else { STEEL }),
                (panels::duration(c.journal_ticks()), t("TIME IN BATTLE").to_string(), TEXT_BRIGHT),
                (rank_name(Veteran { unit: String::new(), xp: c.commander_xp }.rank()).to_string(), t("COMMANDER").to_string(), AMBER_BRIGHT),
            ];
            stat_tiles(ui, &tiles[..3]);
            ui.add_space(6.0);
            stat_tiles(ui, &tiles[3..]);
            if !c.roster.is_empty() {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    ui.label(w::caption("THE VETERANS OF THE CAMPAIGN"));
                });
                let shown = c.roster.len().min(12);
                let row = shown as f32 * 48.0;
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - row) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for v in c.roster.iter().take(shown) {
                        self.veteran_portrait(ui, assets, portraits, v, 44.0, false, false);
                    }
                });
            }
            ui.add_space(if short { 14.0 } else { 20.0 });
            ui.horizontal(|ui| {
                let button = ((ui.available_width() - 2.0 * 12.0) / 3.0).min(200.0);
                ui.add_space(((ui.available_width() - 3.0 * button - 2.0 * 12.0) / 2.0).max(0.0));
                ui.spacing_mut().item_spacing.x = 12.0;
                if ui.add(ActionButton::primary("NEW CONQUEST").size(button, 44.0)).clicked() {
                    self.abandon_at(&Conquest::path());
                }
                if ui.add(ActionButton::secondary("CAMPAIGN RECORD").size(button, 44.0)).clicked() {
                    self.victory_reviewed = true;
                    self.selected = None;
                    self.empire_tab = EmpireTab::Journal;
                }
                if ui.add(ActionButton::secondary("MAIN MENU").size(button, 44.0)).clicked() {
                    action = ConquestAction::Back;
                }
            });
            if let Some(error) = &self.error {
                ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap());
            }
        });
        action
    }
}

/// A row of figures in wells, each with its caption under it, centred.
fn stat_tiles(ui: &mut egui::Ui, tiles: &[(String, String, Color32)]) {
    let tile = vec2(150.0, 64.0);
    let gap = 10.0;
    let total = tiles.len() as f32 * tile.x + (tiles.len().saturating_sub(1)) as f32 * gap;
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
        ui.spacing_mut().item_spacing.x = gap;
        for (figure, caption, color) in tiles {
            let (rect, _) = ui.allocate_exact_size(tile, Sense::hover());
            let painter = ui.painter();
            painter.add(ta_ui::metal::well(rect, INSET));
            painter.text(pos2(rect.center().x, rect.min.y + 26.0), Align2::CENTER_CENTER, figure, display(26.0), *color);
            painter.text(pos2(rect.center().x, rect.max.y - 13.0), Align2::CENTER_CENTER, caption, strong(10.0), STEEL);
        }
    });
}

/// A NEW tag on a portrait's top edge.
fn new_badge(painter: &egui::Painter, rect: egui::Rect) {
    let centre = pos2(rect.max.x - 4.0, rect.min.y + 4.0);
    painter.circle(centre, 7.5, Color32::from_black_alpha(210), Stroke::new(1.0, AMBER_DIM));
    // (A star is not convex: its middle, then each point.)
    let point = |k: usize, r: f32| {
        let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
        centre + vec2(a.cos(), a.sin()) * r
    };
    painter.add(egui::Shape::convex_polygon((0..5).map(|j| point(2 * j + 1, 2.3)).collect(), AMBER_BRIGHT, Stroke::NONE));
    for j in 0..5 {
        painter.add(egui::Shape::convex_polygon(vec![point((2 * j + 9) % 10, 2.3), point(2 * j, 5.6), point(2 * j + 1, 2.3)], AMBER_BRIGHT, Stroke::NONE));
    }
}
