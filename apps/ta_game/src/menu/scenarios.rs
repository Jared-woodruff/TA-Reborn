//! The Scenarios page: battles set up in advance, each with a goal of its own (see
//! `scenario.rs`); a briefing, the ground, the difficulty and the records.

use egui::{Align2, Color32, RichText, Sense, Stroke, vec2};
use ta_ai::Difficulty;
use ta_ui::i18n::{t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::scroll::{self, Behind};
use ta_ui::widgets as w;

use super::{Menu, MenuAction, Page};
use crate::palette::team_color;
use crate::scenario::{Entry, GoalSpec, Ground, Kind, difficulty_key};
use crate::setup::Assets;
use crate::theme::*;

fn clock(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// What a difficulty's tag adds to its name in a scenario's records: its best time (the
/// fastest win, or an endless siege's longest stand: those are never won), a tick for a win
/// without one, nothing (shown dim) for a difficulty not won or held out on.
fn difficulty_record(won: bool, best: Option<u32>) -> Option<String> {
    match (won, best) {
        (_, Some(best)) => Some(format!("· {}", clock(best))),
        (true, None) => Some("✔".to_string()),
        (false, None) => None,
    }
}

/// What a scenario's time of day comes to, for its card: night if it reaches the dark hours,
/// else dusk or dawn (none by day).
fn time_tag(time: &crate::stage::Hours) -> Option<&'static str> {
    let hours: Vec<f32> = match time {
        crate::stage::Hours::At(h) => vec![*h],
        crate::stage::Hours::Keys(keys) => keys.iter().map(|k| k[1]).collect(),
    };
    let hours: Vec<f32> = hours.iter().map(|h| h.rem_euclid(24.0)).collect();
    if hours.iter().any(|&h| !(5.5..20.0).contains(&h)) {
        Some("NIGHT")
    } else if hours.iter().any(|&h| h >= 17.0) {
        Some("DUSK")
    } else if hours.iter().any(|&h| h < 8.0) {
        Some("DAWN")
    } else {
        None
    }
}

/// What a goal asks, in words, before the battle (`several` enemies: their Commanders).
fn goal_text(g: &GoalSpec, assets: &Assets, endless: bool, several: bool) -> String {
    let unit = |name: &str| assets.types.id(name).map_or(name.to_string(), |t| {
        let def = &assets.types.get(t).def;
        if def.display_name.trim().is_empty() { name.to_string() } else { def.display_name.trim().to_string() }
    });
    match g {
        GoalSpec::Survive { survive } => tf("Survive for {time}", &[("time", &clock(*survive))]),
        GoalSpec::Hold { seconds, .. } => tf("Hold the marked ground for {time} in all", &[("time", &clock(*seconds))]),
        GoalSpec::EnemyHolds { seconds, .. } => tf("The enemy holds it for {time} in all", &[("time", &clock(*seconds))]),
        GoalSpec::Reach { unit: which, .. } => match which.as_deref() {
            Some(u) if u.eq_ignore_ascii_case("commander") => t("Get your Commander to the marked point").into(),
            Some(u) => tf("Get a {unit} to the marked point", &[("unit", &unit(u))]),
            None => t("Get a unit to the marked point").into(),
        },
        GoalSpec::Destroy { destroy } => match destroy.to_ascii_lowercase().as_str() {
            "commander" if several => t("Destroy the enemy Commanders").into(),
            "commander" => t("Destroy the enemy Commander").into(),
            "everything" => t("Destroy every enemy unit").into(),
            "army" => t("Destroy the enemy's army").into(),
            _ => tf("Destroy every {unit}", &[("unit", &unit(destroy))]),
        },
        GoalSpec::Metal { metal } => tf("Make {metal} metal first", &[("metal", metal)]),
        GoalSpec::EnemyMetal { enemy_metal } => tf("The enemy makes {metal} metal first", &[("metal", enemy_metal)]),
        GoalSpec::Time { time } => tf("{time} go by", &[("time", &clock(*time))]),
        GoalSpec::Lose { lose } => match lose.to_ascii_lowercase().as_str() {
            "commander" if endless => t("Your Commander is destroyed").into(),
            "commander" => t("Your Commander is destroyed").into(),
            "everything" => t("You lose everything").into(),
            _ => tf("You lose a {unit}", &[("unit", &unit(lose))]),
        },
    }
}

impl Menu {
    /// Opens the page (the scenarios are read again: new files in the folder show up).
    pub fn open_scenarios(&mut self, assets: &Assets) {
        (self.scenarios, self.scenarios_unread) = crate::scenario::catalogue_and_unread(assets);
        self.scenario_selected = self.scenario_selected.min(self.scenarios.len().saturating_sub(1));
        self.page = Page::Scenarios;
    }

    /// Chooses the scenario `id` on the page, its row brought into view.
    pub fn select_scenario(&mut self, id: &str) {
        if let Some(i) = self.scenarios.iter().position(|e| e.scenario.id.eq_ignore_ascii_case(id)) {
            self.scenario_selected = i;
            self.reveal = true;
        }
    }

    pub(super) fn scenarios_page(&mut self, ui: &mut egui::Ui, assets: &Assets, progress: &ta_config::CampaignProgress) -> MenuAction {
        let mut action = MenuAction::None;
        let ctx = ui.ctx().clone();
        let page = Panel::new("SCENARIOS").subtitle("BATTLES SET UP IN ADVANCE, EACH WITH A GOAL OF ITS OWN").width(1100.0).exit(Exit::Back);
        let reveal = std::mem::take(&mut self.reveal);
        let response = panel::page(&ctx, "scenarios", page, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 24.0;
                // The list.
                ui.vertical(|ui| {
                    ui.set_width(300.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    panel::section(ui, &format!("{} · {}", t("SCENARIOS"), self.scenarios.len()));
                    let height = scroll::whole_rows(panel::fit_height(&ctx, 290.0, 300.0, 560.0), panel::LIST_ROW + 1.0, 1.0);
                    let area = egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, false]);
                    scroll::well(ui, area, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for (i, e) in self.scenarios.iter().enumerate() {
                            let won = progress.scenarios.get(&e.scenario.id).is_some_and(|r| !r.won.is_empty() || (e.scenario.kind == Kind::Endless && !r.best.is_empty()));
                            let mut label = RichText::new(if won { format!("✔ {}", t(&e.scenario.title)) } else { t(&e.scenario.title).to_string() });
                            let mut tag = RichText::new(t(e.scenario.kind.label())).font(strong(10.5));
                            // One that can't be played here is greyed, but can still be chosen:
                            // its page says why (under it, START SCENARIO stays dark).
                            if e.problem.is_some() {
                                (label, tag) = (label.color(STEEL_DIM), tag.color(STEEL_DIM));
                            }
                            let row = panel::list_row(ui, i == self.scenario_selected, true, label, Some(tag));
                            let row = match &e.problem {
                                Some(problem) => row.on_hover_text(problem),
                                None => row,
                            };
                            if reveal && i == self.scenario_selected {
                                row.scroll_to_me(Some(egui::Align::Center));
                            }
                            if row.clicked() {
                                self.scenario_selected = i;
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new(t("Scenarios of your own go in the settings folder's scenarios folder.")).font(body(12.0)).color(STEEL_DIM));
                    // Those of the folder that could not be read, why on the pointer (they were
                    // left out without a word), and the folder.
                    if !self.scenarios_unread.is_empty() {
                        let why: Vec<String> = self.scenarios_unread.iter().map(|(file, error)| format!("{file}: {error}")).collect();
                        ui.label(RichText::new(tf("{n} of its files could not be read", &[("n", &self.scenarios_unread.len())])).font(body(12.0)).color(RED)).on_hover_text(why.join("\n"));
                    }
                    if ui.add(w::ActionButton::secondary("OPEN FOLDER")).on_hover_text(t("The folder your own scenarios are read from")).clicked() {
                        let dir = ta_config::config_dir().join("scenarios");
                        let _ = std::fs::create_dir_all(&dir);
                        crate::photo::open_folder(&dir);
                    }
                });
                let Some(entry) = self.scenarios.get(self.scenario_selected).cloned() else { return };
                let s = &entry.scenario;
                // The ground.
                ui.vertical(|ui| {
                    ui.set_width(340.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    panel::section(ui, "BATTLEFIELD");
                    self.scenario_map(ui, assets, &entry);
                    ui.add_space(10.0);
                    w::fitted_title(ui, t(&s.title), 26.0, TEXT_BRIGHT);
                    w::pretty_label(ui, RichText::new(t(&s.tagline)).font(strong(13.0)).color(AMBER));
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        w::tag(ui, s.kind.label(), AMBER);
                        let map = assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(&s.map) || m.title.eq_ignore_ascii_case(&s.map)).map_or(s.map.clone(), |m| m.title.clone());
                        w::tag(ui, &map.to_uppercase(), STEEL);
                        if s.minutes > 0 {
                            w::tag(ui, &tf("ABOUT {n} MIN", &[("n", &s.minutes)]), STEEL);
                        }
                        if !s.allies.is_empty() {
                            w::tag(ui, "WITH ALLIES", SKY);
                        }
                        if let Some(tag) = s.time.as_ref().and_then(time_tag) {
                            w::tag(ui, tag, STEEL);
                        }
                        if s.rain == Some(true) {
                            w::tag(ui, "RAIN", STEEL);
                        }
                        if entry.custom {
                            w::tag(ui, "YOUR OWN", SKY);
                        }
                    });
                    // What its mutators make of the battle.
                    let mutators = crate::menu::mutator_tags(&s.mutators.unwrap_or_default());
                    if !mutators.is_empty() {
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                            for tag in &mutators {
                                w::tag(ui, tag, AMBER);
                            }
                        });
                    }
                });
                // Briefing, goals, difficulty, records.
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.x = 8.0;
                    panel::section(ui, "BRIEFING");
                    panel::well(ui, |ui| {
                        // As tall as the words, up to the room the page has; cut between two
                        // lines when they need more.
                        let text = RichText::new(t(s.briefing.trim())).font(body(13.5)).color(TEXT);
                        let wrap = ui.available_width() - scroll::LANE - 20.0;
                        let words = egui::WidgetText::from(text.clone()).into_galley(ui, Some(egui::TextWrapMode::Wrap), wrap, egui::TextStyle::Body);
                        let line = words.rows.first().map_or(16.0, |r| r.height());
                        let room = panel::fit_height(&ctx, 470.0, 110.0, 300.0);
                        let whole = words.size().y + 16.0;
                        let height = if whole <= room { whole } else { scroll::whole_lines(room, line, 8.0) };
                        let area = egui::ScrollArea::vertical().id_salt("scenario_briefing").max_height(height).min_scrolled_height(height).auto_shrink([false, true]);
                        scroll::show(ui, area, Behind::Well, |ui| {
                            egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(egui::Label::new(text).wrap());
                            });
                        });
                    });
                    ui.add_space(10.0);
                    panel::section(ui, "OBJECTIVES");
                    let endless = s.kind == Kind::Endless;
                    if endless {
                        goal_line(ui, t("Hold out as long as you can: your best time is kept"), GREEN);
                    }
                    let several = s.enemies.len() > 1;
                    for g in &s.victory {
                        goal_line(ui, &goal_text(g, assets, endless, several), GREEN);
                    }
                    for g in &s.defeat {
                        goal_line(ui, &tf("Lost if: {goal}", &[("goal", &goal_text(g, assets, endless, several))]), RED);
                    }
                    ui.add_space(10.0);
                    ui.label(w::caption("DIFFICULTY"));
                    w::segmented(ui, &mut self.scenario_difficulty, &[(Difficulty::Easy, "EASY"), (Difficulty::Normal, "NORMAL"), (Difficulty::Hard, "HARD")]);
                    ui.add_space(8.0);
                    if let Some(r) = progress.scenarios.get(&s.id) {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                            for d in [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard] {
                                let key = difficulty_key(d);
                                let best = r.best.get(key).copied();
                                let won = r.won.iter().any(|x| x == key);
                                let name = t(match d {
                                    Difficulty::Easy => "EASY",
                                    Difficulty::Normal => "NORMAL",
                                    Difficulty::Hard => "HARD",
                                });
                                match difficulty_record(won, best) {
                                    Some(record) => w::tag(ui, &format!("{name} {record}"), GREEN),
                                    None => w::tag(ui, name, STEEL_DIM),
                                };
                            }
                        });
                        ui.add_space(6.0);
                    }
                    if let Some(problem) = &entry.problem {
                        ui.label(RichText::new(problem).color(RED));
                        ui.add_space(4.0);
                    }
                    if panel::hero_button(ui, "START SCENARIO", vec2(ui.available_width(), 58.0), entry.problem.is_none()).clicked() {
                        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                        match s.setup(&entry.source, self.scenario_difficulty, assets, self.seed ^ 0x5CE7) {
                            Ok(setup) => action = MenuAction::Start(setup),
                            Err(e) => {
                                log::warn!("scenario {}: {e:#}", s.id);
                                self.failure = Some((ta_ui::i18n::n("SCENARIO NOT STARTED"), tf("The scenario could not start: {error}", &[("error", &format!("{e:#}"))])));
                            }
                        }
                    }
                });
            });
        });
        if response.exit {
            self.page = Page::Title;
        }
        action
    }

    /// The scenario's map with where each side starts and the ground its goals mark.
    fn scenario_map(&mut self, ui: &mut egui::Ui, assets: &Assets, entry: &Entry) {
        let ctx = ui.ctx().clone();
        let s = &entry.scenario;
        let map = assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(&s.map) || m.title.eq_ignore_ascii_case(&s.map)).cloned();
        let picture = map.as_ref().and_then(|m| crate::map_preview::picture(&ctx, assets, &m.name, 330.0));
        // As tall as the picture is across the column (a square at most): the title comes
        // right under it, a wide map not adrift in a square.
        let height = picture.as_ref().map_or(340.0, |p| (330.0 / p.aspect() + 10.0).min(340.0));
        let (frame, _) = ui.allocate_exact_size(vec2(340.0, height), Sense::hover());
        if let (Some(m), Some(picture)) = (map, picture) {
            let painter = ui.painter();
            let placed = crate::map_preview::show(painter, &picture, frame);
            let starts: Vec<(f32, f32)> = m.info.schemas.first().map(|sc| sc.start_positions.iter().map(|&(_, x, z)| (x, z)).collect()).unwrap_or_default();
            let ground = Ground { size: picture.area, starts };
            let at = |(x, z): (f32, f32)| placed.at(x, z);
            let scale = placed.scale();
            // The goals' ground: a hill, a pick-up point.
            for g in s.victory.iter().chain(&s.defeat) {
                let (place, radius) = match g {
                    GoalSpec::Hold { hold, radius, .. } => (hold, radius),
                    GoalSpec::Reach { reach, radius, .. } => (reach, radius),
                    _ => continue,
                };
                if let Ok(p) = ground.place(place) {
                    let c = at(p);
                    painter.circle(c, (radius * scale).max(6.0), AMBER.gamma_multiply(0.25), Stroke::new(1.5, AMBER_BRIGHT));
                }
            }
            // Where the waves come from (reinforcements in green).
            for wave in &s.waves {
                let colour = if wave.player || wave.ally.is_some() { GREEN } else { RED };
                for from in wave.from.split('|') {
                    if let Ok(p) = ground.place(from) {
                        let c = at(p);
                        painter.circle(c, 4.0, colour.gamma_multiply(0.6), Stroke::new(1.0, Color32::from_black_alpha(200)));
                    }
                }
            }
            // The sides (kept on the picture), in the colours the battle gives them: you, the
            // enemies (!), the allies (+).
            let sides = std::iter::once((&s.player, 0u8, t("YOU"), true))
                .chain(s.enemies.iter().enumerate().map(|(i, f)| (f, [1u8, 3, 5, 7, 2][i % 5], "!", false)))
                .chain(s.allies.iter().enumerate().map(|(i, f)| (f, [8u8, 6, 4, 9][i % 4], "+", false)));
            for (f, colour, mark, you) in sides {
                if let Ok((x, z)) = ground.place(&f.start) {
                    let c = placed.marker_at(x, z, 10.5);
                    painter.circle(c, 9.0, team_color(colour), Stroke::new(1.5, Color32::from_black_alpha(220)));
                    painter.text(c, Align2::CENTER_CENTER, mark, strong(if you { 8.5 } else { 11.0 }), Color32::WHITE);
                }
            }
        } else {
            ui.painter().add(ta_ui::metal::well(frame.shrink(5.0), INSET));
        }
    }
}

fn goal_line(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
        ui.painter().circle_filled(dot.center(), 3.5, color);
        ui.add(egui::Label::new(RichText::new(text).font(body(13.5)).color(TEXT_BRIGHT)).wrap());
    });
}

#[cfg(test)]
mod tests {
    use super::difficulty_record;

    /// A scenario's records by difficulty: an endless siege held out on Normal only is no record
    /// on Easy or Hard (each showed a tick, as though held out on them too).
    #[test]
    fn only_difficulties_won_or_held_out_on_show_a_record() {
        // An endless siege: never won, its longest stand kept.
        assert_eq!(difficulty_record(false, Some(125)).as_deref(), Some("· 2:05"));
        assert_eq!(difficulty_record(false, None), None, "an endless siege not played on this difficulty");
        // The others: the fastest win, or a tick for a win without a time.
        assert_eq!(difficulty_record(true, Some(600)).as_deref(), Some("· 10:00"));
        assert_eq!(difficulty_record(true, None).as_deref(), Some("✔"));
    }
}
