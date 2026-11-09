//! The panel beside the galaxy: a system's (its map, the enemy there, what it brings, the force
//! that would go in, ATTACK; for one of ours under attack, the attack and DEFEND; for one
//! uncharted, what little is known) or the empire's, in three tabs (the campaign's progress,
//! the veterans, the journal of battles).

use egui::{Color32, Rect, RichText, Sense, Shape, Stroke, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use super::{ConquestPage, EmpireTab, enemy_line, galaxy, rank_name};
use crate::conquest::{Conquest, Counter, Owner, Reward, Veteran};
use crate::setup::Assets;
use crate::theme::*;

impl ConquestPage {
    /// The panel, `width` wide inside its chrome, as tall as `height` at most (its body
    /// scrolls past that).
    pub(super) fn details(&mut self, ui: &mut egui::Ui, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, c: &Conquest, width: f32, height: Option<f32>) {
        let charted = c.charted();
        let title = match self.selected {
            Some(i) if !charted[i] && i != c.capital => t("UNCHARTED SYSTEM").to_string(),
            Some(i) => c.systems[i].name.to_uppercase(),
            None => t("YOUR EMPIRE").to_string(),
        };
        let subtitle = match self.selected {
            Some(i) if !charted[i] => if i == c.capital { t("THE ENEMY CAPITAL · UNCHARTED") } else { t("BEYOND YOUR SENSORS") }.to_string(),
            Some(i) if c.defendable(i) => tf("{world} · UNDER ATTACK", &[("world", &c.systems[i].planet.to_uppercase())]),
            Some(i) if i == c.capital && c.systems[i].owner == Owner::Enemy => tf("{world} · THE ENEMY CAPITAL", &[("world", &c.systems[i].planet.to_uppercase())]),
            Some(i) if i == c.home => tf("{world} · YOUR HOME", &[("world", &c.systems[i].planet.to_uppercase())]),
            Some(i) => c.systems[i].planet.to_uppercase(),
            None => tf("{side} · COMMANDER RANK {rank}", &[("side", &c.side), ("rank", &rank_name(Veteran { unit: String::new(), xp: c.commander_xp }.rank()))]),
        };
        let mut panel = Panel::new(&title).width(width);
        // (A map that names no world has no line under its name.)
        if !subtitle.is_empty() { panel = panel.subtitle(&subtitle); }
        if let Some(height) = height { panel = panel.max_height(height); }
        // A system's panel closes with its cross, as the empire's does in a sheet.
        if self.selected.is_some() || self.sheet { panel = panel.exit(Exit::Close); }
        let response = panel.show(ui, |ui| {
            if let Some(error) = &self.error { ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap()); }
            if let Some(pending) = self.pending_deployment.clone() {
                ui.horizontal_wrapped(|ui| {
                    if ui.add(ActionButton::primary("RETRY")).clicked() { self.set_deployment_at(c, pending, &crate::conquest::Conquest::path()); }
                    if ui.add(ActionButton::secondary("RELOAD")).clicked() { self.reload(); }
                });
                w::pretty_label(ui, RichText::new(t("Save the veteran selection or reload the campaign before attacking.")).color(AMBER));
            }
            match self.selected {
                Some(i) => self.system_panel(ui, assets, portraits, c, i, height),
                None => self.empire_panel(ui, assets, c, portraits),
            }
        });
        if response.exit {
            self.selected = None;
            self.sheet = false;
        }
    }

    /// A system: its map's picture and facts, who holds it and how strongly, the force that
    /// would go in and ATTACK (which briefs the battle), and the battles fought there.
    fn system_panel(&mut self, ui: &mut egui::Ui, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, c: &Conquest, i: usize, height: Option<f32>) {
        let s = &c.systems[i];
        // Uncharted: what holds it and what it brings are learnt by taking a system beside it.
        if !c.charted()[i] {
            let (frame, _) = ui.allocate_exact_size(vec2(ui.available_width(), 110.0), Sense::hover());
            ui.painter().add(ta_ui::metal::well(frame, INSET));
            ui.painter().text(frame.center(), egui::Align2::CENTER_CENTER, "?", display(52.0), STEEL_DIM);
            ui.add_space(6.0);
            if i == c.capital {
                w::pretty_label(ui, RichText::new(tf("Capture {name}, the enemy capital, to win the campaign.", &[("name", &s.name)])).color(AMBER));
            }
            w::pretty_label(ui, RichText::new(t("Uncharted: take a system beside it to learn what holds it and what it brings.")).color(TEXT));
            if c.held_count(Reward::Sensors) == 0 {
                w::pretty_label(ui, RichText::new(t("A sensor array charts systems two jumps beyond yours.")).color(STEEL));
            }
            return;
        }
        let map = assets.maps.iter().find(|m| m.name == s.map);
        // The map's picture, always: above the facts in a tall panel, beside them in a short one.
        let width = ui.available_width();
        let short = height.is_some_and(|h| h < 640.0);
        let room = if short { (width * 0.4).clamp(110.0, 170.0) } else { height.map_or(220.0, |h| (h - 420.0).clamp(140.0, 260.0)) };
        let picture = |ui: &mut egui::Ui, across: f32| match crate::map_preview::picture(ui.ctx(), assets, &s.map, across.max(room)) {
            Some(picture) => {
                let tall = room.min((across - 10.0) / picture.aspect());
                let (frame, _) = ui.allocate_exact_size(vec2(across, tall + 10.0), Sense::hover());
                crate::map_preview::show(ui.painter(), &picture, frame);
            }
            None => {
                let (frame, _) = ui.allocate_exact_size(vec2(across, room * 0.6), Sense::hover());
                ui.painter().add(ta_ui::metal::well(frame, INSET));
                ui.painter().text(frame.center(), egui::Align2::CENTER_CENTER, t("Battlefield unavailable"), body(13.0), STEEL);
            }
        };
        let facts = |ui: &mut egui::Ui| {
            if let Some(m) = map {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    w::tag(ui, &m.size, STEEL);
                    w::tag(ui, &tf("UP TO {n} PLAYERS", &[("n", &m.start_positions)]), STEEL);
                    if i == c.capital { w::tag(ui, "CAPITAL", AMBER); }
                    if i == c.home { w::tag(ui, "HOME", Color32::from_rgb(150, 190, 240)); }
                });
            }
            ui.add_space(4.0);
            match s.owner {
                Owner::Player => {
                    w::pretty_label(ui, RichText::new(t(if i == c.home { "Your home system." } else { "Held by your forces." })).color(GREEN));
                }
                Owner::Enemy => {
                    ui.label(w::caption("THE ENEMY"));
                    // (The words beside the pips, wrapping under themselves, not under the pips.)
                    ui.horizontal_top(|ui| {
                        let line = ui.text_style_height(&egui::TextStyle::Body);
                        let (rect, _) = ui.allocate_exact_size(vec2(42.0, line), Sense::hover());
                        galaxy::paint_threat(ui.painter(), pos2(rect.min.x + 1.0, rect.center().y), s.threat, 11.0);
                        ui.add(egui::Label::new(RichText::new(enemy_line(c, i, Some(&assets.maps))).color(TEXT)).wrap());
                    });
                }
            }
        };
        if short {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                picture(ui, room);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    facts(ui);
                });
            });
        } else {
            picture(ui, width);
            ui.add_space(6.0);
            facts(ui);
        }
        // Under attack: from where, and how long there is to answer.
        if let Some(counter) = c.counter.filter(|counter| counter.system == i && c.defendable(i)) {
            ui.add_space(6.0);
            alert(ui, |ui| {
                ui.label(w::caption("UNDER ATTACK").color(RED));
                w::pretty_label(ui, RichText::new(tf("The enemy strikes from {from}.", &[("from", &c.systems[counter.from].name)])).color(TEXT_BRIGHT));
                w::pretty_label(ui, RichText::new(counter_when(c, counter)).color(TEXT_BRIGHT));
                w::pretty_label(ui, RichText::new(t("Beat it off in a battle here, or take the system it strikes from first.")).color(STEEL));
            });
        }
        // The system the enemy strikes from: taking it ends that.
        if let Some(counter) = c.counter.filter(|counter| counter.from == i) {
            ui.add_space(4.0);
            w::pretty_label(ui, RichText::new(tf("Taking it breaks up the enemy's attack on {system}.", &[("system", &c.systems[counter.system].name)])).color(AMBER));
        }
        if let Some(m) = map
            && !m.description.is_empty()
        {
            ui.add_space(4.0);
            w::pretty_label(ui, RichText::new(&m.description).color(STEEL));
        }
        if c.reward(i) != Reward::None {
            panel::section(ui, if s.owner == Owner::Player { n("IT BRINGS") } else { n("TAKING IT BRINGS") });
            reward_row(ui, assets, c, c.reward(i));
        }
        ui.add_space(4.0);
        if c.attackable(i) || c.defendable(i) {
            panel::section(ui, "YOUR FORCE");
            self.force_row(ui, assets, portraits, c, 34.0);
            let (metal, energy) = c.bonus();
            w::pretty_label(ui, RichText::new(tf("+{metal} metal and +{energy} energy at the start", &[("metal", &metal), ("energy", &energy)])).color(STEEL));
        }
        // The battles fought here, and a fall to a counter-attack.
        let lines: Vec<String> = c.history.iter().rev().filter_map(|entry| {
            let (r, n) = (&entry.report, &entry.battle);
            if r.system == i {
                Some(match (r.defence, r.won) {
                    (false, true) => tf("Battle {n}: taken", &[("n", n)]),
                    (false, false) => tf("Battle {n}: defeat", &[("n", n)]),
                    (true, true) => tf("Battle {n}: held", &[("n", n)]),
                    (true, false) => tf("Battle {n}: lost", &[("n", n)]),
                })
            } else {
                (r.fell == Some(i)).then(|| tf("Battle {n}: fell to a counter-attack", &[("n", n)]))
            }
        }).take(3).collect();
        if !lines.is_empty() {
            w::pretty_label(ui, RichText::new(lines.join(" · ")).color(STEEL_DIM));
        }
        if s.owner == Owner::Enemy {
            ui.add_space(4.0);
            if c.attackable(i) {
                if c.setup(i, &assets.maps).is_none() {
                    ui.add(egui::Label::new(RichText::new(t("This battlefield is unavailable. Enable its map pack, then return to Conquest.")).color(AMBER)).wrap());
                }
            } else if c.won {
                w::pretty_label(ui, RichText::new(t("The enemy capital has fallen. This campaign is complete.")).color(GREEN));
            } else {
                w::pretty_label(ui, RichText::new(t("Out of reach: take a neighbouring system first.")).color(STEEL_DIM));
            }
        }
    }

    /// The height the bar under a system's panel takes (one within reach, or of ours under
    /// attack, has one).
    pub(super) fn bar_height(&self, c: &Conquest) -> f32 {
        if self.selected.is_some_and(|i| c.attackable(i) || c.defendable(i)) { 62.0 } else { 0.0 }
    }

    /// Under the panel of a system within reach, in sight however the panel scrolls: ATTACK
    /// (DEFEND for one of ours under attack), which briefs the battle, and the veterans.
    pub(super) fn action_bar(&mut self, ui: &mut egui::Ui, assets: &Assets, c: &Conquest, width: f32) {
        let Some(i) = self.selected.filter(|&i| i < c.systems.len() && (c.attackable(i) || c.defendable(i))) else { return };
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let ready = c.setup(i, &assets.maps).is_some() && self.pending_deployment.is_none();
            let veterans = (width * 0.34).clamp(110.0, 150.0);
            let go = if c.defendable(i) { n("DEFEND") } else { n("ATTACK") };
            if panel::hero_button(ui, go, vec2(width - veterans - 8.0, 54.0), ready).clicked() {
                self.briefing = Some(i);
            }
            if ui.add(ActionButton::secondary("VETERANS").size(veterans, 54.0)).clicked() {
                self.selected = None;
                self.empire_tab = EmpireTab::Veterans;
            }
        });
    }

    /// The force that would go in: the Commander and the veterans deploying, as portraits
    /// `size` points across, ranks marked, with how many there are of how many berths.
    pub(super) fn force_row(&mut self, ui: &mut egui::Ui, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, c: &Conquest, size: f32) {
        let veterans = c.selected_veterans();
        let fresh = c.reinforcements();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            let commander = crate::setup::resolve_side(&assets.data, &c.side).1.map(|s| s.commander.clone());
            if let Some(commander) = commander {
                self.veteran_portrait(ui, assets, portraits, &Veteran { unit: commander, xp: c.commander_xp }, size, false, true);
            }
            for v in &veterans {
                self.veteran_portrait(ui, assets, portraits, v, size, false, false);
            }
            // The systems' reinforcements, fresh: a blue cross on each.
            for unit in &fresh {
                let response = self.veteran_portrait(ui, assets, portraits, &Veteran { unit: unit.clone(), xp: ta_sim::Fx::ZERO }, size, false, false);
                reinforcement_mark(ui.painter(), response.rect);
                response.on_hover_text(t("A reinforcement from your systems"));
            }
        });
        let commander = rank_name(Veteran { unit: String::new(), xp: c.commander_xp }.rank());
        let line = if veterans.is_empty() {
            tf("The Commander ({rank}) alone; {slots} veteran berths", &[("rank", &commander), ("slots", &c.slots())])
        } else {
            tf("The Commander ({rank}) and {n} of {slots} veterans", &[("rank", &commander), ("n", &veterans.len()), ("slots", &c.slots())])
        };
        w::pretty_label(ui, RichText::new(line).color(TEXT));
        if !fresh.is_empty() {
            w::pretty_label(ui, RichText::new(tf("Reinforcements from your systems: {n}", &[("n", &fresh.len())])).color(Color32::from_rgb(140, 186, 242)));
        }
    }

    fn empire_panel(&mut self, ui: &mut egui::Ui, assets: &Assets, c: &Conquest, portraits: &mut crate::encyclopaedia::Portraits) {
        w::segmented_fill(ui, &mut self.empire_tab, &[
            (EmpireTab::Progress, "PROGRESS"), (EmpireTab::Veterans, "VETERANS"), (EmpireTab::Journal, "JOURNAL"),
        ]);
        ui.add_space(10.0);
        match self.empire_tab {
            EmpireTab::Progress => self.progress_panel(ui, assets, c),
            EmpireTab::Veterans => self.veterans_panel(ui, assets, c, portraits),
            EmpireTab::Journal => self.journal_panel(ui, assets, c, portraits),
        }
    }

    fn progress_panel(&mut self, ui: &mut egui::Ui, assets: &Assets, c: &Conquest) {
        // A counter-attack under way, first.
        if let Some(counter) = c.counter {
            alert(ui, |ui| {
                ui.label(w::caption("UNDER ATTACK").color(RED));
                w::pretty_label(ui, RichText::new(tf("{system} is under attack from {from}.", &[("system", &c.systems[counter.system].name), ("from", &c.systems[counter.from].name)])).color(TEXT_BRIGHT));
                w::pretty_label(ui, RichText::new(counter_when(c, counter)).color(TEXT_BRIGHT));
                if ui.add(ActionButton::primary("DEFEND").height(30.0)).clicked() {
                    self.selected = Some(counter.system);
                }
            });
            ui.add_space(6.0);
        }
        self.commander_section(ui, c);
        ui.add_space(8.0);
        panel::section(ui, "CAMPAIGN OBJECTIVE");
        let capital = &c.systems[c.capital].name;
        let objective = if c.won { t("The enemy capital has fallen. This campaign is complete.").to_string() }
            else { tf("Capture {name}, the enemy capital, to win the campaign.", &[("name", capital)]) };
        w::pretty_label(ui, RichText::new(objective).color(if c.won { GREEN } else { TEXT }));
        if !c.won {
            let route = match c.remaining_jumps() {
                1 => t("The enemy capital is within reach").to_string(),
                n => tf("Shortest route: {n} battles from your territory to the capital.", &[("n", &n)]),
            };
            w::pretty_label(ui, RichText::new(route).color(AMBER));
            ui.horizontal_wrapped(|ui| {
                if ui.add(ActionButton::secondary("VIEW CAPITAL")).clicked() { self.selected = Some(c.capital); }
                let mut route = self.show_route;
                if w::toggle_named(ui, &mut route, t("Show the route")).changed() { self.show_route = route; }
                ui.label(RichText::new(t("Show the route")).color(TEXT));
            });
        }
        ui.add_space(8.0);
        panel::section(ui, "TERRITORY");
        w::pretty_label(ui, RichText::new(tf("{held} of {total} systems secured", &[("held", &c.held()), ("total", &c.systems.len())])).color(TEXT));
        panel::progress_bar(ui, c.held() as f32 / c.systems.len() as f32, ui.available_width());
        w::pretty_label(ui, RichText::new(tf("{battles} battles · {wins} victories · {losses} defeats", &[("battles", &c.battles), ("wins", &c.wins), ("losses", &c.battles.saturating_sub(c.wins))])).color(STEEL));
        ui.add_space(8.0);
        let (metal, energy) = c.bonus();
        panel::section(ui, "SUPPLY");
        w::pretty_label(ui, RichText::new(tf("+{metal} metal and +{energy} energy at the start of each battle", &[("metal", &metal), ("energy", &energy)])).color(TEXT));
        w::pretty_label(ui, RichText::new(tf("{n} veterans may join each battle", &[("n", &c.slots())])).color(TEXT));
        if !c.won {
            let milestone = c.next_slot_in().map_or_else(|| t("All transport berths available in this galaxy are unlocked.").to_string(), |n|
                tf("Secure {n} more systems to unlock another veteran berth.", &[("n", &n)]));
            w::pretty_label(ui, RichText::new(milestone).color(AMBER));
        }
        // What the systems held bring.
        ui.add_space(8.0);
        panel::section(ui, "REWARDS");
        let held: Vec<(Reward, usize)> = Reward::ALL.iter().map(|&r| (r, c.held_count(r))).filter(|&(_, count)| count > 0).collect();
        if held.is_empty() {
            w::pretty_label(ui, RichText::new(t("Every system brings a reward once taken: the badges on the map show which.")).color(STEEL));
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 6.0);
                // (Each chip one widget of its measured width, so that the row wraps.)
                for (reward, count) in held {
                    let color = galaxy::reward_color(reward);
                    let text = w::galley(ui, RichText::new(format!("{} ×{count}", t(reward.label()))).font(strong(11.0)).color(color));
                    let (rect, response) = ui.allocate_exact_size(vec2(22.0 + text.size().x, text.size().y.max(18.0)), Sense::hover());
                    galaxy::paint_reward(ui.painter(), pos2(rect.min.x + 9.0, rect.center().y), 15.0, reward);
                    ui.painter().galley(pos2(rect.min.x + 22.0, rect.center().y - text.size().y / 2.0), text, color);
                    response.on_hover_text(reward_effect(assets, c, reward));
                }
            });
            let fresh = c.reinforcements();
            if !fresh.is_empty() {
                let names: Vec<String> = fresh.iter().map(|u| unit_name(assets, u)).collect();
                w::pretty_label(ui, RichText::new(tf("Joining each battle: {units}", &[("units", &names.join(", "))])).color(TEXT));
            }
            if c.held_count(Reward::Sensors) > 0 {
                w::pretty_label(ui, RichText::new(t("Charts systems two jumps beyond yours")).color(TEXT));
            }
        }
        ui.add_space(8.0);
        if ui.add(ActionButton::secondary("MANAGE VETERANS")).clicked() { self.empire_tab = EmpireTab::Veterans; }
        if !c.won {
            w::pretty_label(ui, RichText::new(t("Pick a system on the map: the gold rings are within reach.")).color(STEEL_DIM));
        }
    }

    fn veterans_panel(&mut self, ui: &mut egui::Ui, assets: &Assets, c: &Conquest, portraits: &mut crate::encyclopaedia::Portraits) {
        let selected = c.deployment_indices();
        w::pretty_label(ui, RichText::new(tf("{selected} / {slots} selected · {reserve} in reserve", &[("selected", &selected.len()), ("slots", &c.slots()), ("reserve", &c.roster.len().saturating_sub(selected.len()))])).color(AMBER));
        let mode = if c.deployment.is_none() { t("Automatic: the most experienced veterans deploy first.") }
            else { t("Custom selection for the next battle. Automatic selection resumes after the battle.") };
        w::pretty_label(ui, RichText::new(mode).color(STEEL));
        let mut change = None;
        ui.add_enabled_ui(!c.won, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add(ActionButton::secondary("AUTO: STRONGEST")).clicked() { change = Some(None); }
                if ui.add(ActionButton::secondary("COMMANDER ONLY")).clicked() { change = Some(Some(Vec::new())); }
            });
        });
        ui.add_space(8.0);
        if c.roster.is_empty() {
            ui.add(egui::Label::new(RichText::new(t("None yet. Units that earn a rank in battle and survive it join the roster.")).color(STEEL)).wrap());
        } else {
            let full = ui.available_width();
            for (k, veteran) in c.roster.iter().enumerate() {
                let available = assets.types.id(&veteran.unit);
                let name = available.map_or_else(|| veteran.unit.clone(), |id| assets.types.get(id).def.display_name.clone());
                let mut deploy = selected.contains(&k);
                ui.push_id(k, |ui| {
                    egui::Frame::new().fill(if deploy { PANEL_HI } else { INSET }).inner_margin(6.0).show(ui, |ui| {
                        ui.set_width(full - 12.0);
                        ui.horizontal(|ui| {
                            let can_change = !c.won && (deploy || (available.is_some() && selected.len() < c.slots()));
                            let checkbox = ui.add_enabled_ui(can_change, |ui| w::check_box_named(ui, &mut deploy, &tf("Deploy {unit}", &[("unit", &name)]))).inner;
                            if checkbox.changed() {
                                let mut next = selected.clone();
                                if deploy { next.push(k); } else { next.retain(|&index| index != k); }
                                next.sort_unstable();
                                change = Some(Some(next));
                            }
                            self.veteran_portrait(ui, assets, portraits, veteran, 44.0, !selected.contains(&k), false);
                            ui.vertical(|ui| {
                                ui.set_max_width(ui.available_width());
                                w::pretty_label(ui, RichText::new(&name).color(TEXT_BRIGHT));
                                let xp = format!("{:.1}", veteran.xp.to_f64());
                                w::pretty_label(ui, RichText::new(tf("{rank} · {xp} XP", &[("rank", &rank_name(veteran.rank())), ("xp", &xp)])).color(AMBER));
                                let status = if available.is_none() { t("Unavailable in the active game data; stays in reserve.") }
                                    else if selected.contains(&k) { t("Selected for deployment") } else { t("In reserve") };
                                w::pretty_label(ui, RichText::new(status).color(STEEL));
                            });
                        });
                    });
                });
            }
            w::pretty_label(ui, RichText::new(t("Veterans that cannot deploy on a battlefield stay in reserve. Defeat loses the veterans that actually deployed.")).color(STEEL_DIM));
        }
        if let Some(deployment) = change { self.set_deployment_at(c, deployment, &crate::conquest::Conquest::path()); }
    }

    /// A unit's portrait `size` points square (TA's picture of it while the portrait is made),
    /// its rank in chevrons, dimmed when it stays behind; the Commander's in brass.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn veteran_portrait(&mut self, ui: &mut egui::Ui, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, veteran: &Veteran, size: f32, reserve: bool, commander: bool) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        if !ui.is_rect_visible(rect) { return response; }
        let portrait = assets.types.id(&veteran.unit).and_then(|id| portraits.get(ui.ctx(), id, 0, size));
        if let Some(portrait) = portrait { portrait.paint(ui.painter(), rect, Color32::WHITE); }
        else if let Some(picture) = self.unit_pic(ui.ctx(), assets, &veteran.unit) {
            ui.painter().image(picture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else { ui.painter().rect_filled(rect, 2.0, INSET); }
        if reserve { ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(120)); }
        let edge = if commander { AMBER } else if reserve { LINE } else { AMBER_DIM };
        ui.painter().rect_stroke(rect, 1.0, Stroke::new(if commander { 1.6 } else { 1.0 }, edge), egui::StrokeKind::Inside);
        let s = size / 44.0;
        for n in 0..veteran.rank() {
            let y = rect.max.y - (6.5 + n as f32 * 5.0) * s;
            ui.painter().add(Shape::line(vec![pos2(rect.min.x + 5.0 * s, y), pos2(rect.min.x + 8.5 * s, y - 3.0 * s), pos2(rect.min.x + 12.0 * s, y)], Stroke::new(1.8 * s, AMBER)));
        }
        let name = assets.types.id(&veteran.unit).map_or_else(|| veteran.unit.clone(), |id| assets.types.get(id).def.display_name.clone());
        response.on_hover_text(tf("{unit} · {rank}", &[("unit", &name), ("rank", &rank_name(veteran.rank()))]))
    }

    fn journal_panel(&mut self, ui: &mut egui::Ui, assets: &Assets, c: &Conquest, portraits: &mut crate::encyclopaedia::Portraits) {
        if c.history.is_empty() {
            w::pretty_label(ui, RichText::new(t("Battle reports will appear here after your next battle.")).color(STEEL));
            if let Some(report) = c.last.clone() { self.battle_card(ui, assets, portraits, c, &report, None); }
            return;
        }
        w::pretty_label(ui, RichText::new(tf("Recent battles: {n} of {total}", &[("n", &c.history.len()), ("total", &c.battles)])).color(STEEL));
        for entry in c.history.iter().rev() {
            let heading = tf("BATTLE {n} · {duration}", &[("n", &entry.battle), ("duration", &duration(entry.duration_ticks))]);
            self.battle_card(ui, assets, portraits, c, &entry.report, Some(&heading));
        }
    }

    /// A battle's card in the journal: when and how long, the system taken or lost there (SHOW
    /// finds it on the map), who came back and who fell.
    fn battle_card(&mut self, ui: &mut egui::Ui, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, c: &Conquest, report: &crate::conquest::Report, heading: Option<&str>) {
        ui.add_space(6.0);
        let full = ui.available_width();
        egui::Frame::new().fill(INSET).inner_margin(8.0).show(ui, |ui| {
            ui.set_width(full - 16.0);
            ui.horizontal(|ui| {
                if let Some(heading) = heading {
                    ui.label(RichText::new(heading).font(strong(10.5)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(ActionButton::secondary("SHOW").height(24.0)).clicked() {
                        self.selected = Some(report.system);
                    }
                });
            });
            let (head, color) = super::report_head(c, report);
            w::pretty_label(ui, RichText::new(head).font(strong(13.5)).color(color));
            let detail = if report.won {
                tf("{returned} veterans came back ({new} new), {lost} lost", &[("returned", &report.returned), ("new", &report.new_veterans), ("lost", &report.lost)])
            } else { tf("{lost} veterans lost; the Commander was rebuilt with half its experience", &[("lost", &report.lost)]) };
            w::pretty_label(ui, RichText::new(detail).color(TEXT));
            if let Some(d) = &report.details
                && !(d.returned.is_empty() && d.fallen.is_empty())
            {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                    for v in &d.returned {
                        self.veteran_portrait(ui, assets, portraits, v, 26.0, false, false);
                    }
                    for v in &d.fallen {
                        let r = self.veteran_portrait(ui, assets, portraits, v, 26.0, true, false);
                        fallen_mark(ui.painter(), r.rect);
                    }
                });
            }
        });
    }
}

/// A fallen unit's portrait crossed out in red.
pub(super) fn fallen_mark(painter: &egui::Painter, rect: Rect) {
    let r = rect.shrink(rect.width() * 0.22);
    let stroke = Stroke::new((rect.width() / 14.0).max(1.4), Color32::from_rgb(210, 60, 46));
    painter.line_segment([r.left_top(), r.right_bottom()], stroke);
    painter.line_segment([r.right_top(), r.left_bottom()], stroke);
}

/// A battle's length as h:mm:ss.
pub(super) fn duration(ticks: u64) -> String {
    let seconds = ticks / 30;
    format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// A box that calls for attention: a red wash and rim round `add`.
fn alert(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    egui::Frame::new().fill(Color32::from_rgba_unmultiplied(120, 24, 18, 64)).stroke(Stroke::new(1.0, RED.gamma_multiply(0.6))).inner_margin(8.0).corner_radius(3).show(ui, |ui| {
        ui.set_width(width - 16.0);
        add(ui);
    });
}

/// When a counter-attack lands, in words.
pub(super) fn counter_when(c: &Conquest, counter: Counter) -> String {
    match counter.due.saturating_sub(c.battles) {
        0 | 1 => t("It lands at the end of your next battle.").to_string(),
        n => tf("It lands when {n} more battles have been fought.", &[("n", &n)]),
    }
}

/// A unit's name as the game data has it.
pub(super) fn unit_name(assets: &Assets, unit: &str) -> String {
    assets.types.id(unit).map_or_else(|| unit.to_string(), |id| assets.types.get(id).def.display_name.clone())
}

/// What a reward does, in words.
pub(super) fn reward_effect(assets: &Assets, c: &Conquest, reward: Reward) -> String {
    let unit = || crate::conquest::reinforcement(&c.side, reward).map_or_else(String::new, |u| unit_name(assets, u));
    match reward {
        Reward::None => String::new(),
        Reward::Mines => tf("+{metal} metal at the start of each battle", &[("metal", &crate::conquest::MINES_METAL)]),
        Reward::Reactors => tf("+{energy} energy at the start of each battle", &[("energy", &crate::conquest::REACTORS_ENERGY)]),
        Reward::Foundry | Reward::Airfield => tf("A {unit} joins each battle", &[("unit", &unit())]),
        Reward::Academy => t("The Commander gains experience after each victory").to_string(),
        Reward::Sensors => t("Charts systems two jumps beyond yours").to_string(),
        Reward::Depot => t("One more veteran berth").to_string(),
    }
}

/// A reward's line: its glyph, its name and what it does.
pub(super) fn reward_row(ui: &mut egui::Ui, assets: &Assets, c: &Conquest, reward: Reward) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
        ui.painter().circle(rect.center(), 12.5, Color32::from_rgba_unmultiplied(8, 10, 14, 200), Stroke::new(1.0, galaxy::reward_color(reward).gamma_multiply(0.6)));
        galaxy::paint_reward(ui.painter(), rect.center(), 17.0, reward);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(t(reward.label())).font(strong(12.0)).color(galaxy::reward_color(reward)).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
            w::pretty_label(ui, RichText::new(reward_effect(assets, c, reward)).color(TEXT));
        });
    });
}

/// A reinforcement's mark on its portrait: a blue cross in its corner.
fn reinforcement_mark(painter: &egui::Painter, rect: Rect) {
    let c = pos2(rect.max.x - 6.0, rect.min.y + 6.0);
    painter.circle_filled(c, 5.5, Color32::from_rgba_unmultiplied(8, 10, 14, 220));
    let blue = Color32::from_rgb(140, 186, 242);
    painter.line_segment([c - vec2(3.0, 0.0), c + vec2(3.0, 0.0)], Stroke::new(1.8, blue));
    painter.line_segment([c - vec2(0.0, 3.0), c + vec2(0.0, 3.0)], Stroke::new(1.8, blue));
}
