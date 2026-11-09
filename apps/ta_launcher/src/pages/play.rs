//! Play page: the Play button and install details when the original game is found, with the
//! player's own battles (the last saved game to continue, the last replay to watch, their
//! last photo behind the banner); setup help (store links, folder picker, rescan) when it is
//! not.

use ta_ui::tooltip::TooltipExt as _;
use std::path::{Path, PathBuf};

use egui::{Align, Color32, Layout, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};
use ta_config::{CameraStyle, WindowMode};
use ta_ui::i18n::{n, t, tf};
use ta_vfs::install::{self, Content, InstallError, InstallSource, TaInstall};

use super::{deck, field, paragraph, path_label, wrapped};
use crate::app::{Action, LauncherApp};
use crate::battles::{self, Record};
use crate::detect;
use crate::launch::GameState;
use crate::modlist::Resolution;
use crate::theme::*;
use crate::widgets::{self, ActionButton, Glyph, Mark, PlayState, caption};
use ta_ui::metal;

/// Thickness of the hero's pipe frame.
const HERO_PIPE: f32 = 4.0;
/// Room kept at the right of a flagged card's top row for the lamp in its corner.
const CORNER_LAMP: f32 = 18.0;
/// A game that stops sooner than this did not get going: it failed to launch.
const FAILED_START: f32 = 10.0;

impl LauncherApp {
    pub(crate) fn play_page(&mut self, ui: &mut Ui) {
        // (The tiles and the battles come in a moment after the banner, as the page does.)
        let since = self.page_since;
        let part = |ui: &mut Ui, index: usize, add: &mut dyn FnMut(&mut Ui)| ta_ui::panel::stagger(ui, since + 0.04, index, 0.05, vec2(0.0, 8.0), add);
        match self.detection.install().cloned() {
            Some(install) => {
                self.ready_hero(ui);
                ui.add_space(space::L);
                self.launch_feedback(ui);
                part(ui, 0, &mut |ui| self.quick_start(ui));
                ui.add_space(space::XL);
                part(ui, 1, &mut |ui| self.battles_card(ui));
                ui.add_space(14.0);
                self.update_card(ui);
                self.news_card(ui);
                self.first_run_card(ui);
                if ui.available_width() < 720.0 {
                    self.install_card(ui, &install, 0.0);
                    ui.add_space(space::L);
                    content_card(ui, &install.content, 0.0);
                } else {
                    // The two cards line up: each is as tall as the taller one's contents (as
                    // measured the frame before), the install card's buttons at its foot.
                    let id = ui.id().with("install_cards");
                    let natural: [f32; 2] = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
                    let tallest = natural[0].max(natural[1]);
                    let measured = widgets::columns(ui, 2, |columns| {
                        [self.install_card(&mut columns[0], &install, tallest - natural[0]), content_card(&mut columns[1], &install.content, tallest)]
                    });
                    if measured.iter().zip(natural).any(|(m, n)| (m - n).abs() > 0.5) {
                        ui.ctx().data_mut(|d| d.insert_temp(id, measured));
                        ui.ctx().request_repaint();
                    }
                }
            }
            None => {
                self.setup_page(ui);
                ui.add_space(space::L);
                self.update_card(ui);
            },
        }
    }

    // ---- Found ---------------------------------------------------------------------------

    /// The banner: the player's last photo behind it, else the map of their newest battle (or
    /// TA Reborn's map of the day) under a radar's scan, else a radar scope between the words
    /// and the button; the state, what the game will start with, and the Play button. As tall
    /// as a good third of the page in sight.
    fn ready_hero(&mut self, ui: &mut Ui) {
        let stacked = ui.available_width() < 620.0;
        let tall = (ui.clip_rect().height() * 0.36).clamp(188.0, 300.0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), if stacked { tall + 104.0 } else { tall }), Sense::hover());
        let painter = ui.painter().clone();
        painter.add(metal::plate(ui.ctx(), rect, PANEL));
        let inside = rect.shrink(HERO_PIPE + 2.0);
        // (The idle animations' clock: it holds while they are held, so nothing jumps.)
        let time = self.ambient;
        let still = ta_ui::motion::still(ui.ctx());
        let photo = self.battles.backdrop.as_ref().map(|(path, texture)| (path.clone(), texture.clone()));
        let map = if photo.is_none() { self.hero.shown.as_ref().map(|shown| (shown.map.clone(), shown.source, shown.texture.clone(), shown.level)) } else { None };
        // A map just made fades in over the radar scope that stood there while it was drawn.
        let appear = match self.hero.shown.as_ref().filter(|_| map.is_some()) {
            Some(shown) if !ta_ui::motion::reduced(ui.ctx()) => (((ui.input(|i| i.time) - shown.since) / 0.8) as f32).clamp(0.0, 1.0),
            Some(_) => 1.0,
            None => 0.0,
        };
        if appear > 0.0 && appear < 1.0 {
            ui.ctx().request_repaint();
        }
        // The scope goes where the words and the button leave room, so it is painted once they
        // are laid out, into this slot under the frame.
        let scope = painter.add(egui::Shape::Noop);
        let mut backdrop = painter.with_clip_rect(inside);
        if let Some((_, texture)) = &photo {
            paint_photo(&backdrop, inside, texture, time);
        } else if let Some((_, _, texture, level)) = &map {
            backdrop.multiply_opacity(egui::emath::easing::cubic_in_out(appear));
            deck::paint_map(&backdrop, inside, texture, *level, time);
            deck::paint_scan(&backdrop, inside, time, !still);
            deck::paint_shade(&backdrop, inside);
        }
        ta_ui::motion::next_frame(ui.ctx(), std::time::Duration::from_millis(33));
        painter.add(metal::pipe_frame(rect, HERO_PIPE));

        let running = self.game.is_running();
        let summary = self.settings_summary();
        let mods = self.mods_summary();
        // A child that lays out inside the already-allocated hero without moving our cursor.
        let compact = rect.width() < 720.0;
        let inset = metal::pipe_inset(HERO_PIPE) + space::XL;
        // Where the picture came from, small in the corner: the photo's name (a link to the
        // pictures), or the map's.
        let caption = match (&photo, &map) {
            (Some((path, _)), _) => Some((tf("YOUR PHOTO · {name}", &[("name", &ta_ui::i18n::upper(&path.file_stem().map_or(String::new(), |s| s.to_string_lossy().to_string())))]), true)),
            (None, Some((name, source, _, _))) => Some((match source {
                crate::hero::Source::LastBattle => tf("LAST BATTLE · {map}", &[("map", &ta_ui::i18n::upper(name))]),
                crate::hero::Source::OfTheDay => tf("MAP OF THE DAY · {map}", &[("map", &ta_ui::i18n::upper(name))]),
            }, false)),
            _ => None,
        };
        if let Some((label, pictures)) = caption && !stacked {
            let caption_rect = Rect::from_min_max(pos2(inside.max.x - inside.width() * 0.42, inside.min.y + 6.0), pos2(inside.max.x - 10.0, inside.min.y + 28.0));
            let mut caption = ui.new_child(UiBuilder::new().max_rect(caption_rect).layout(Layout::right_to_left(Align::Center)));
            caption.set_clip_rect(caption.clip_rect().intersect(caption_rect));
            // (A map's name comes with its picture.)
            if !pictures {
                caption.multiply_opacity(appear);
            }
            let response = egui::Frame::new().fill(Color32::from_black_alpha(150)).corner_radius(2.0).inner_margin(egui::Margin::symmetric(5, 2))
                .show(&mut caption, |ui| ui.add(egui::Label::new(RichText::new(label).font(strong(10.5)).color(if pictures { STEEL } else { GREEN.gamma_multiply(0.85) }).extra_letter_spacing(ta_ui::i18n::tracking(1.2))).truncate().sense(if pictures { Sense::click() } else { Sense::hover() })))
                .inner;
            if pictures {
                let response = response.hover_tip(t("Open your pictures")).on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.clicked() { self.push(Action::OpenFolder(self.battles.pictures_dir.clone())); }
            }
        }
        let mut content = ui.new_child(
            UiBuilder::new().max_rect(rect.shrink2(vec2(inset, inset - space::S))).layout(if stacked { Layout::top_down(Align::Min) } else { Layout::left_to_right(Align::Center) }),
        );
        let ui = &mut content;
        // (The words in the middle of the banner's height: as tall as they were the frame before.)
        let measured = egui::Id::new(("launcher_hero_words", stacked));
        let words_height: f32 = ui.ctx().data(|d| d.get_temp(measured)).unwrap_or(0.0);
        let words = ui.vertical(|ui| {
            if !stacked {
                ui.set_max_width((ui.available_width() - if compact { 240.0 } else { 300.0 }).max(200.0));
                ui.add_space(((ui.available_height() - words_height) / 2.0).max(0.0));
            }
            let top = ui.cursor().min.y;
            // A status lamp: green and steady when ready, brass and pulsing while running.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (lamp, _) = ui.allocate_exact_size(vec2(12.0, 16.0), Sense::hover());
                let pulsing = running && !ta_ui::motion::still(ui.ctx());
                let on = if pulsing { (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.3 + 0.7 } else { 1.0 };
                if pulsing {
                    ta_ui::motion::next_frame(ui.ctx(), std::time::Duration::from_millis(33));
                }
                let wrong = self.game.went_wrong();
                let colour = if running || wrong { AMBER } else { GREEN };
                ui.painter().add(metal::lamp(lamp.center(), 4.0, colour, on));
                let status = match self.game.running_for() {
                    Some(time) => tf("OPERATION IN PROGRESS · {time}", &[("time", &battles::length(time.as_secs()))]),
                    None if wrong => t("THE LAST GAME WENT WRONG · SEE BELOW").to_string(),
                    None => t("ALL SYSTEMS GO").to_string(),
                };
                if running {
                    // (The time ticks over each second.)
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                }
                ui.label(RichText::new(status).font(strong(12.5)).color(colour).extra_letter_spacing(ta_ui::i18n::tracking(2.2)));
                // A game that stopped responding can be ended from here (asked first).
                if running && widgets::text_link(ui, "END GAME").hover_tip(t("Close the game at once, if it has stopped responding")).clicked() {
                    self.push(Action::EndGame);
                }
            });
            // (Larger in a taller banner.)
            let size = 40.0 + 8.0 * ((tall - 210.0) / 90.0).clamp(0.0, 1.0);
            widgets::fitted_title(ui, t(if running { "GAME RUNNING" } else { "READY FOR DEPLOYMENT" }), size, TEXT_BRIGHT);
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(&summary).color(TEXT)).truncate()).hover_tip(&summary);
            let last = ui.add(egui::Label::new(RichText::new(&mods).color(TEXT)).truncate()).hover_tip(&mods);
            let height = last.rect.bottom() - top;
            if (height - words_height).abs() > 0.5 {
                ui.ctx().data_mut(|d| d.insert_temp(measured, height));
                ui.ctx().request_repaint();
            }
        }).response.rect;
        if stacked { ui.add_space(space::S); }
        let mut button = Rect::NOTHING;
        ui.with_layout(if stacked { Layout::left_to_right(Align::Center) } else { Layout::right_to_left(Align::Center) }, |ui| {
            let state = if running { PlayState::Running } else { PlayState::Ready };
            let response = widgets::play_button(ui, state, compact);
            button = response.rect;
            if response.clicked() {
                self.push(Action::Play);
            }
            if running {
                response.hover_tip(t("The game is running. Close it to launch again."));
            } else {
                response.hover_tip(t("Start the game (Enter)"));
            }
        });
        // The scope, whole and clear of both, when there is room for one worth the name (and no
        // picture behind the words; fading as one comes).
        if photo.is_none() && appear < 1.0 {
            let room = if stacked { Rect::from_x_y_ranges(button.right() + 24.0..=inside.right() - 16.0, button.y_range()) }
                else { Rect::from_x_y_ranges(words.right() + 24.0..=button.left() - 24.0, inside.y_range().shrink(10.0)) };
            let r = (room.width().min(room.height()) / 2.0 - 3.0).min(70.0);
            if r >= 34.0 {
                let mut faded = painter.with_clip_rect(inside);
                faded.multiply_opacity(1.0 - appear);
                faded.set(scope, radar(room.center(), r, time));
            }
        }
    }

    /// The ways into the game as tiles (three across, each with a line on what it is; two
    /// across with their names alone in a narrow window), using the same saved settings and
    /// launch feedback as Play. A commander yet to finish a battle is led to the tutorial first:
    /// its tile comes first, in brass.
    fn quick_start(&mut self, ui: &mut Ui) {
        use deck::Mode;
        let ready = !self.game.is_running();
        let newcomer = self.newcomer();
        widgets::card_title(ui, "QUICK START");
        if newcomer {
            ui.label(RichText::new(t("New to Total Annihilation? Start with the tutorial: five short lessons.")).color(TEXT));
            ui.add_space(2.0);
        }
        let tutorial = (Mode::Tutorial, n("TUTORIAL"), n("Learn the controls and practise commanding your army"), &["--skip-intro", "--open", "tutorial"][..]);
        let modes: Vec<_> = newcomer.then_some(tutorial).into_iter().chain([
            (Mode::Skirmish, n("SKIRMISH"), n("Choose a map and opponents for a new battle"), &["--skip-intro", "--open-skirmish"][..]),
            (Mode::Campaign, n("CAMPAIGN"), n("Choose an Arm or Core campaign mission"), &["--skip-intro", "--open-campaign"][..]),
            (Mode::Scenarios, n("SCENARIOS"), n("Battles set up in advance, each with a goal of its own: hold out until dawn, bring down the Titans"), &["--skip-intro", "--open", "scenarios"][..]),
            (Mode::Conquest, n("CONQUEST"), n("Take a galaxy system by system, your Commander rising with every battle"), &["--skip-intro", "--open", "conquest"][..]),
            (Mode::Multiplayer, n("MULTIPLAYER"), n("Host a battle or join one over the network"), &["--skip-intro", "--open", "multiplayer"][..]),
        ]).chain((!newcomer).then_some(tutorial)).collect();
        let width = ui.available_width();
        let full = width >= 560.0;
        let (columns, gap) = (if full { 3 } else { 2 }, 12.0);
        let size = vec2(((width - gap * (columns - 1) as f32) / columns as f32).floor(), if full { 76.0 } else { 56.0 });
        for row in modes.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for &(mode, label, hint, args) in row {
                    let response = ui.add_enabled_ui(ready, |ui| deck::mode_tile(ui, mode, label, full.then_some(hint), size, newcomer && mode == Mode::Tutorial)).inner;
                    // (Its line is on the tile when there is room; else in its tooltip.)
                    let response = if full { response } else { response.hover_tip(t(hint)) };
                    if response.disabled_tip(t("The game is running. Close it to launch again.")).clicked() {
                        self.push(Action::PlayWith(args.iter().map(Into::into).collect()));
                    }
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
    }

    fn settings_summary(&self) -> String {
        let g = &self.config.graphics;
        let mode = match g.window_mode {
            WindowMode::Windowed => t("windowed"),
            WindowMode::Borderless => t("borderless"),
            WindowMode::Fullscreen => t("fullscreen"),
        };
        let camera = match g.camera {
            CameraStyle::Modern => t("modern camera"),
            CameraStyle::Classic => t("classic camera"),
        };
        tf(
            "{width} × {height} {mode}  ·  {camera}  ·  unit limit {limit}",
            &[("width", &g.width), ("height", &g.height), ("mode", &mode), ("camera", &camera), ("limit", &self.config.gameplay.unit_limit)],
        )
    }

    fn mods_summary(&self) -> String {
        match (&self.mods.resolution, self.config.mods.enabled.len()) {
            (_, 0) => t("No mods enabled — original content only").to_owned(),
            (Resolution::Err(_), n) => tf("{n} mod(s) enabled — load order has a problem, see Mods", &[("n", &n)]),
            (Resolution::Ok(order), _) => {
                let names: Vec<&str> = order.iter().map(|id| self.mods.get(id).map_or(id.as_str(), |m| m.info.manifest.name.as_str())).collect();
                tf("Mods: {names}", &[("names", &names.join(" → "))])
            }
        }
    }

    /// After an update: what is new, once (until dismissed).
    fn news_card(&mut self, ui: &mut Ui) {
        let Some(release) = self.news.clone() else { return };
        let mut done = false;
        let mut open = false;
        widgets::card_accent(ui, Some(SKY), |ui| {
            ui.horizontal(|ui| {
                widgets::status_mark(ui, Mark::Ok);
                ui.label(RichText::new(tf("UPDATED TO v{version}", &[("version", &release.version)])).font(strong(13.0)).color(SKY).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(CORNER_LAMP);
                    done = widgets::icon_button(ui, Glyph::Close, true).hover_tip(t("Dismiss")).clicked();
                });
            });
            // The changelog is in English: its words shown as they are in English, and a word
            // of the update in other languages.
            if ta_ui::i18n::language().is_empty() && !release.summary.is_empty() {
                wrapped(ui, &crate::news::brief(&release.summary, 3), TEXT);
            } else {
                paragraph(ui, "TA Reborn has been updated. The changelog tells what is new in this version.");
            }
            ui.add_space(4.0);
            open = ui.add(ActionButton::secondary("WHAT'S NEW")).hover_tip(t("Everything this version brings")).clicked();
        });
        ui.add_space(14.0);
        if open {
            self.open_whats_new();
        }
        if done || open {
            self.news = None;
            self.config.launcher.seen_version = env!("CARGO_PKG_VERSION").to_string();
        }
    }

    /// The first start: what it found on this computer and how it set the game up.
    fn first_run_card(&mut self, ui: &mut Ui) {
        let Some(first) = &self.first_run else { return };
        let mut dismiss = false;
        let mut settings = false;
        widgets::card_accent(ui, Some(GREEN), |ui| {
            ui.horizontal(|ui| {
                widgets::status_mark(ui, Mark::Ok);
                ui.label(RichText::new(t("SET UP FOR THIS COMPUTER")).font(strong(13.0)).color(GREEN).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // (Clear of the card's lamp in the corner.)
                    ui.add_space(CORNER_LAMP);
                    dismiss = widgets::icon_button(ui, Glyph::Close, true).hover_tip(t("Dismiss")).clicked();
                });
            });
            if !first.computer.is_empty() {
                wrapped(ui, &first.computer, TEXT);
            }
            let (w, h) = first.size;
            // ("HIGH" in a sentence: "High".)
            let level: String = t(first.level.label()).chars().enumerate().map(|(i, c)| if i == 0 { c } else { c.to_lowercase().next().unwrap_or(c) }).collect();
            wrapped(ui, &tf("{level} graphics, {width} × {height}, full screen. They are yours to change in Settings.", &[("level", &level), ("width", &w), ("height", &h)]), TEXT);
            if let Some(language) = &first.language {
                wrapped(ui, &tf("The menus are in {language}, as Windows is.", &[("language", language)]), TEXT);
            }
            ui.add_space(4.0);
            settings = ui.add(ActionButton::secondary("SETTINGS")).clicked();
        });
        ui.add_space(14.0);
        if settings {
            self.page = crate::app::Page::Settings;
        }
        if dismiss || settings {
            self.first_run = None;
        }
    }

    /// The game failed to start, crashed, or stopped with an error: what happened, and its
    /// crash report and log for a report.
    fn launch_feedback(&mut self, ui: &mut Ui) {
        let (title, text, log, crash) = match &self.game {
            GameState::Failed(message) => (t("LAUNCH FAILED"), message.clone(), None, None),
            GameState::Exited { success: false, code, ran_for, log, crash } => {
                let secs = ran_for.as_secs_f32();
                let title = if crash.is_some() {
                    t("THE GAME CRASHED")
                } else if secs < FAILED_START {
                    t("LAUNCH FAILED")
                } else {
                    t("THE GAME STOPPED")
                };
                let length = if secs < 60.0 { format!("{secs:.1} s") } else { battles::length(ran_for.as_secs()) };
                let text = match code {
                    Some(c) => tf("The game stopped after {time} with exit code {code}.", &[("time", &length), ("code", c)]),
                    None => tf("The game stopped after {time}.", &[("time", &length)]),
                };
                (title, text, log.clone(), crash.clone())
            }
            _ => return,
        };
        widgets::card_accent(ui, Some(RED), |ui| {
            ui.horizontal(|ui| {
                widgets::status_mark(ui, Mark::Error);
                ui.label(RichText::new(title).font(strong(13.0)).color(RED).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(CORNER_LAMP);
                    if widgets::icon_button(ui, Glyph::Close, true).hover_tip(t("Dismiss")).clicked() {
                        self.game = GameState::Idle;
                        ui.ctx().request_repaint();
                    }
                });
            });
            wrapped(ui, &text, TEXT);
            if let Some(crash) = &crash {
                ui.add(egui::Label::new(RichText::new(&crash.message).font(mono(12.0)).color(STEEL)).wrap());
                wrapped(ui, t("Its crash report has what is needed to look into it: please include it if you report the problem."), STEEL);
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                // (Mods that cannot be loaded together: where to put them right.)
                if crash.is_none() && !self.config.mods.enabled.is_empty() && matches!(self.mods.resolution, Resolution::Err(_))
                    && ui.add(ActionButton::secondary("LOAD ORDER")).hover_tip(t("The mods that are on, in the order they load: what is wrong is shown there, with what puts it right")).clicked()
                {
                    self.page = crate::app::Page::Mods;
                    self.mods_tab = crate::app::ModsTab::LoadOrder;
                }
                if let Some(crash) = &crash {
                    if ui.add(ActionButton::secondary("OPEN CRASH REPORT")).clicked() {
                        self.push(Action::OpenPath(crash.report.clone()));
                    }
                    let details = format!("TA Reborn {}\n{}\n{}", crate::version_label(), crash.message, crash.report.display());
                    if ui.add(ActionButton::secondary("COPY DETAILS")).hover_tip(t("The version and the crash, to paste into a report")).clicked() {
                        self.push(Action::Copy(details));
                    }
                }
                if let Some(log) = log
                    && ui.add(ActionButton::secondary("OPEN GAME LOG")).clicked()
                {
                    self.push(Action::OpenPath(log));
                }
                let issues = format!("{}/issues/new", crate::app::PROJECT_URL);
                if ui.add(ActionButton::secondary("REPORT A PROBLEM ↗")).hover_tip(t("Tell us what happened on the project's page (copy the details first)")).clicked() {
                    self.push(Action::OpenUrl(issues));
                }
            });
        });
        ui.add_space(14.0);
    }

    /// Recent saved games and replays, each a click away; the folders they are in.
    fn battles_card(&mut self, ui: &mut Ui) {
        let (save, replay) = (self.battles.save.clone(), self.battles.replay.clone());
        let (older_saves, older_replays) = (self.battles.older_saves.clone(), self.battles.older_replays.clone());
        let ready = !self.game.is_running();
        let folders = [
            (t("SAVES"), self.battles.saves_dir.clone()),
            (t("REPLAYS"), self.battles.replays_dir.clone()),
            (t("PICTURES"), self.battles.pictures_dir.clone()),
        ];
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "YOUR BATTLES");
            let mut chosen = None;
            if save.is_none() && replay.is_none() {
                wrapped(ui, t("Your battles come here: single-player battles save themselves every five minutes, and every battle is recorded as a replay. Continue the last one, or watch it again, with one click."), STEEL);
            } else {
                let mods = &self.mods;
                if ui.available_width() < 660.0 {
                    if let Some(record) = record_slot(ui, t("RECENT SAVES"), save.as_ref(), &older_saves, t("CONTINUE"), "--load", ready, mods, None) { chosen = Some((record, "--load")); }
                    ui.add_space(space::M);
                    if let Some(record) = record_slot(ui, t("RECENT REPLAYS"), replay.as_ref(), &older_replays, t("WATCH"), "--replay", ready, mods, None) { chosen = Some((record, "--replay")); }
                } else {
                    let row = egui::Id::new("launcher_recent_slots");
                    widgets::columns(ui, 2, |columns| {
                        if let Some(record) = record_slot(&mut columns[0], t("RECENT SAVES"), save.as_ref(), &older_saves, t("CONTINUE"), "--load", ready, mods, Some(row)) {
                            chosen = Some((record, "--load"));
                        }
                        if let Some(record) = record_slot(&mut columns[1], t("RECENT REPLAYS"), replay.as_ref(), &older_replays, t("WATCH"), "--replay", ready, mods, Some(row)) {
                            chosen = Some((record, "--replay"));
                        }
                    });
                    // The taller of the two, for both next frame (once more now if it changed).
                    let ctx = ui.ctx();
                    let tallest = ["--load", "--replay"].iter().map(|flag| ctx.data(|d| d.get_temp::<f32>(row.with(flag))).unwrap_or(0.0)).fold(0.0, f32::max);
                    if ctx.data(|d| d.get_temp::<f32>(row)).is_none_or(|was| (was - tallest).abs() > 0.5) {
                        ctx.data_mut(|d| d.insert_temp(row, tallest));
                        ctx.request_repaint();
                    }
                }
            }
            if let Some((record, flag)) = chosen {
                self.open_battle(record, flag);
            }
            // Their folders, quietly at the foot.
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.label(widgets::caption("OPEN"));
                for (label, dir) in &folders {
                    if ui.link(RichText::new(*label).font(strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(1.0))).hover_tip(dir.display().to_string()).clicked() {
                        self.push(Action::OpenFolder(dir.clone()));
                    }
                }
            });
        });
    }

    /// A battle chosen to continue or watch: opened at once when its mods are as they were (or it
    /// names none), else asked about first ([`LauncherApp::battle_dialog`]).
    pub(crate) fn open_battle(&mut self, record: Record, flag: &'static str) {
        match battles::mod_check(&record, &self.mods).filter(battles::asks) {
            Some(check) => self.battle_prompt = Some(battles::BattlePrompt { record, flag, check }),
            None => self.push(Action::PlayWith(vec![flag.into(), record.path.into_os_string()])),
        }
    }

    /// A battle made with other mods than those on: which, and how they stand (as the game says
    /// it); its mods turned on (and the others off) and it opened, a missing one got, or opened as
    /// it is, or not.
    pub(crate) fn battle_dialog(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &self.battle_prompt else { return };
        let save = prompt.flag == "--load";
        let lines = ta_ui::record_mods::lines(&prompt.check);
        let accent = if lines.iter().any(|l| l.tone == ta_ui::record_mods::Tone::Bad) { RED } else { AMBER };
        let missing: Option<String> = prompt.check.missing().next().map(|m| m.id.clone());
        let catalogue = missing.as_deref().and_then(crate::catalogue::get);
        let switch = prompt.check.switchable();
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, 600.0);
        let mut choice = None;
        let response = ta_ui::panel::modal(ctx, "launcher_battle_mods", ta_ui::panel::Panel::new("MADE WITH OTHER MODS").exit(ta_ui::panel::Exit::Close).width(width).accent(accent), true, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add(egui::Label::new(RichText::new(&prompt.record.title).font(strong(15.0)).color(TEXT_BRIGHT)).wrap());
            ta_ui::record_mods::show(ui, &lines, 8);
            ui.add_space(2.0);
            let why = if save {
                t("A saved game is played over from its start to where it was saved. With other mods its orders can build other units, or none: it may not come out as the battle you saved.")
            } else {
                t("A replay is its battle played over from the orders given in it. With other mods they can lead elsewhere: it may not show the battle it recorded.")
            };
            ui.add(egui::Label::new(RichText::new(why).color(TEXT)).wrap());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                if switch {
                    let label = if save { n("USE ITS MODS AND CONTINUE") } else { n("USE ITS MODS AND WATCH") };
                    if ui.add(ActionButton::primary(label)).hover_tip(t("Turns its mods on, in its order, and the others off (map packs stay on)")).clicked() {
                        choice = Some(BattleChoice::Switch);
                    }
                }
                if let Some(id) = &missing {
                    let primary = !switch;
                    let (label, hover) = match catalogue {
                        Some(entry) if entry.file.is_some_and(|f| f.can_download()) && !self.offline => (n("GET IT"), t("Downloads and installs it from the catalogue; then continue the battle again")),
                        _ => (n("SHOW IN MODS"), t("Install it on the Mods page (its card says how, or from a file you have), then continue the battle again")),
                    };
                    let button = if primary { ActionButton::primary(label) } else { ActionButton::secondary(label) };
                    if ui.add(button).hover_tip(hover).clicked() {
                        choice = Some(BattleChoice::Get(id.clone()));
                    }
                }
                let anyway = if save { n("CONTINUE ANYWAY") } else { n("WATCH ANYWAY") };
                if ui.add(ActionButton::secondary(anyway)).hover_tip(t("Its orders are played over with the mods on now")).clicked() {
                    choice = Some(BattleChoice::Anyway);
                }
                if ui.add(ActionButton::secondary("CANCEL")).clicked() {
                    choice = Some(BattleChoice::Cancel);
                }
            });
        });
        if response.exit && choice.is_none() {
            choice = Some(BattleChoice::Cancel);
        }
        if let Some(choice) = choice {
            self.choose_for_battle(choice);
        }
    }

    /// What the player chose for the battle asked about: its mods turned on and it opened, a
    /// missing mod got (or its card shown), it opened as it is, or nothing.
    fn choose_for_battle(&mut self, choice: BattleChoice) {
        let Some(prompt) = self.battle_prompt.take() else { return };
        let args = |anyway: bool| {
            let mut args = vec![prompt.flag.into(), prompt.record.path.clone().into_os_string()];
            if anyway { args.push("--anyway".into()); }
            args
        };
        match choice {
            BattleChoice::Switch => {
                log::info!("{}: its mods turned on ({:?})", prompt.record.path.display(), prompt.check.load_order());
                self.config.mods.enabled = prompt.check.load_order();
                self.mods.refresh(&self.config.mods.enabled);
                self.push(Action::PlayWith(args(false)));
            }
            BattleChoice::Get(id) => {
                if self.show_wanted_mod(&id)
                    && let Some(entry) = crate::catalogue::get(&id).filter(|e| e.file.is_some_and(|f| f.can_download()))
                {
                    self.push(Action::GetMod(entry.id));
                }
            }
            BattleChoice::Anyway => self.push(Action::PlayWith(args(true))),
            BattleChoice::Cancel => {}
        }
    }

    /// The install's details, `spare` taller than its contents (its buttons at the foot);
    /// returns the contents' height.
    fn install_card(&mut self, ui: &mut Ui, install: &TaInstall, spare: f32) -> f32 {
        let spare = spare.max(0.0);
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "ORIGINAL GAME");
            field(ui, ta_ui::i18n::n("LOCATION"), |ui| {
                path_label(ui, &install.root, TEXT_BRIGHT);
            });
            field(ui, ta_ui::i18n::n("DETECTED VIA"), |ui| {
                wrapped(ui, t(detect::source_label(install.source)), TEXT);
            });
            field(ui, ta_ui::i18n::n("EDITION"), |ui| {
                wrapped(ui, t(detect::edition_name(&install.content)), TEXT);
            });
            // TA's version (the 3.1 patch's), and the store it came from (GOG's or Steam's): the
            // store's build number reads as debug output, so it is kept for a look on hover.
            if install.content.patch_3_1 || install.store.name().is_some() {
                field(ui, ta_ui::i18n::n("VERSION"), |ui| {
                    let version = if install.content.patch_3_1 { "3.1" } else { "—" };
                    let shown = match install.store.name() {
                        Some(store) => format!("{version} · {store}"),
                        None => version.to_string(),
                    };
                    let row = wrapped(ui, &shown, TEXT);
                    if let (Some(store), Some(build)) = (install.store.name(), &install.build) {
                        row.hover_tip(tf("{store} build {build}", &[("store", &store), ("build", build)]));
                    }
                });
            }
            ui.add_space(space::M + spare);
            ui.horizontal_wrapped(|ui| {
                if ui.add(ActionButton::secondary("OPEN FOLDER")).clicked() {
                    self.push(Action::OpenPath(install.root.clone()));
                }
                if ui.add(ActionButton::secondary("CHANGE…")).hover_tip(t("Use a different install folder")).clicked() {
                    self.push(Action::BrowseInstall);
                }
                if ui.add(ActionButton::secondary("RESCAN")).hover_tip(t("Look for the game again (F5)")).clicked() {
                    self.push(Action::Rescan);
                }
                if install.source == InstallSource::Explicit
                    && ui.add(ActionButton::secondary("AUTO-DETECT")).hover_tip(t("Forget the chosen folder and detect the install automatically")).clicked()
                {
                    self.push(Action::UseAutoDetect);
                }
            });
            self.browse_error_note(ui);
            ui.min_rect().height() - spare
        })
        .inner
    }

    // ---- Not found / invalid ---------------------------------------------------------------

    fn setup_page(&mut self, ui: &mut Ui) {
        let invalid = match &self.detection.result {
            Err(InstallError::Invalid { path, missing }) => Some((path.clone(), missing.clone())),
            _ => None,
        };
        let suggestion = self.detection.suggestion.as_ref().map(|s| (s.root.clone(), s.source));
        widgets::card_accent(ui, Some(RED), |ui| {
            let compact = ui.available_width() < 700.0;
            let stacked = ui.available_width() < 620.0;
            ui.with_layout(if stacked { Layout::top_down(Align::Min) } else { Layout::left_to_right(Align::Center) }, |ui| {
                ui.vertical(|ui| {
                    if !stacked { ui.set_max_width((ui.available_width() - if compact { 240.0 } else { 300.0 }).max(280.0)); }
                    match &invalid {
                        None => not_found_text(ui),
                        Some((path, missing)) => invalid_folder_text(ui, path, missing, suggestion.as_ref()),
                    }
                });
                if !stacked {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        widgets::play_button(ui, PlayState::Unavailable, compact).hover_tip(t("Play becomes available once the original game has been found."));
                    });
                }
            });
            ui.add_space(14.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                match (&invalid, &suggestion) {
                    // The configured folder broke, but another install exists: one click fixes it.
                    (Some(_), Some(_)) => {
                        if ui.add(ActionButton::primary("USE DETECTED INSTALL")).clicked() {
                            self.push(Action::UseAutoDetect);
                        }
                        if ui.add(ActionButton::secondary("BROWSE FOR ANOTHER FOLDER…")).clicked() {
                            self.push(Action::BrowseInstall);
                        }
                    }
                    // The configured folder broke and nothing else was found.
                    (Some(_), None) => {
                        if ui.add(ActionButton::primary("BROWSE FOR THE GAME FOLDER…")).clicked() {
                            self.push(Action::BrowseInstall);
                        }
                        self.store_buttons(ui, false);
                        if ui.add(ActionButton::secondary("FORGET THIS FOLDER")).hover_tip(t("Go back to automatic detection")).clicked() {
                            self.push(Action::UseAutoDetect);
                        }
                    }
                    // Nothing found: the game from either store, or a folder already installed.
                    (None, _) => {
                        self.store_buttons(ui, true);
                        if ui.add(ActionButton::secondary("BROWSE FOR EXISTING INSTALL…")).clicked() {
                            self.push(Action::BrowseInstall);
                        }
                    }
                }
                if ui.add(ActionButton::secondary("RESCAN")).hover_tip(t("Look for the game again (F5)")).clicked() {
                    self.push(Action::Rescan);
                }
            });
            ui.add_space(6.0);
            wrapped(ui, t("Or drop the game's folder on this window."), STEEL_DIM);
            self.browse_error_note(ui);
        });

        if invalid.is_some() && suggestion.is_some() {
            return;
        }
        ui.add_space(14.0);
        let steps = [
            ("01", n("GET THE GAME"), n("Total Annihilation: Commander Pack, from GOG.com or Steam: the base game plus The Core Contingency and Battle Tactics.")),
            ("02", n("INSTALL IT"), n("With GOG Galaxy, GOG's offline installer or Steam, into any folder. Already yours? Open it in your store's app:")),
            ("03", n("RESCAN & PLAY"), n("The launcher finds the install by itself. Nothing is copied or modified — the data is read in place.")),
        ];
        // (The second step's own buttons: the game in GOG Galaxy or in Steam's library. Side by
        // side, the steps are as tall as the tallest was last frame.)
        let mut open = None;
        let tallest_id = egui::Id::new("launcher_setup_steps_tallest");
        let side_by_side = ui.available_width() >= 720.0;
        let least = if side_by_side { ui.ctx().data(|d| d.get_temp::<f32>(tallest_id)).unwrap_or(108.0) } else { 108.0 };
        let mut tallest: f32 = 0.0;
        let mut step = |ui: &mut Ui, (number, title, text): (&str, &str, &str)| {
            let height = step_card(ui, number, title, text, least, |ui| {
                if number != "02" {
                    return;
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.add(ActionButton::secondary("GOG GALAXY")).hover_tip(t("Shows the game in GOG Galaxy, where you can install it if you own it")).clicked() {
                        open = Some(Action::OpenGalaxy);
                    }
                    if ui.add(ActionButton::secondary("STEAM")).hover_tip(t("Shows the game in your Steam library, where you can install it if you own it")).clicked() {
                        open = Some(Action::OpenSteam);
                    }
                });
            });
            tallest = tallest.max(height);
        };
        if !side_by_side {
            for s in steps {
                step(ui, s);
                ui.add_space(space::M);
            }
        } else {
            widgets::columns(ui, 3, |columns| {
                for (column, s) in columns.iter_mut().zip(steps) { step(column, s); }
            });
        }
        if side_by_side && (tallest - least).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(tallest_id, tallest));
            ui.ctx().request_repaint();
        }
        if let Some(action) = open {
            self.push(action);
        }

        if let Err(InstallError::NotFound { searched }) = &self.detection.result {
            ui.add_space(14.0);
            let searched = searched.clone();
            widgets::card(ui, |ui| {
                egui::CollapsingHeader::new(RichText::new(tf("SEARCHED {n} LOCATIONS", &[("n", &searched.len())])).font(strong(11.5)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(1.6)))
                    .id_salt("searched_locations")
                    .show(ui, |ui| {
                        for path in &searched {
                            path_label(ui, path, STEEL);
                        }
                    });
            });
        }
    }

    /// The game's pages in both its stores, GOG.com's and Steam's (gold when it is the way on).
    fn store_buttons(&mut self, ui: &mut Ui, first: bool) {
        for (label, url) in [(n("GET IT ON GOG"), install::GOG_STORE_URL), (n("GET IT ON STEAM"), install::STEAM_STORE_URL)] {
            let button = if first { ActionButton::primary(label) } else { ActionButton::secondary(label) };
            if ui.add(button).hover_tip(url).clicked() {
                self.push(Action::OpenUrl(url.into()));
            }
        }
    }

    /// Explains why the last folder picked (or dropped) is not the game's.
    fn browse_error_note(&mut self, ui: &mut Ui) {
        let Some((path, missing)) = &self.browse_error else { return };
        ui.add_space(10.0);
        ui.horizontal_top(|ui| {
            widgets::status_mark(ui, Mark::Error);
            wrapped(ui, &tf("{path} is not a Total Annihilation folder (missing {files}).", &[("path", &path.display()), ("files", &missing.join(", "))]), RED);
        });
        wrapped(ui, t(r"Pick the folder that contains totala1.hpi, such as C:\GOG Games\Total Annihilation, or Total Annihilation in Steam's steamapps\common folder."), STEEL);
    }
}

fn not_found_text(ui: &mut Ui) {
    widgets::pill(ui, "ORIGINAL GAME NOT FOUND", RED);
    ui.add_space(4.0);
    widgets::fitted_title(ui, t("TOTAL ANNIHILATION REQUIRED"), 38.0, TEXT_BRIGHT);
    ui.add_space(2.0);
    paragraph(
        ui,
        "TA Reborn is a new engine for Total Annihilation. It ships none of the original game's \
         content — no units, maps, artwork, sound or music — and plays the data files of your own copy, \
         read in place from its install folder.",
    );
    ui.add_space(2.0);
    paragraph(ui, "Get Total Annihilation: Commander Pack from GOG or Steam (it includes both expansions), install it, then press Rescan. Already installed elsewhere? Browse for the folder.");
}

/// Explains that the configured folder is unusable, and shows the detected install if any.
fn invalid_folder_text(ui: &mut Ui, path: &Path, missing: &[String], suggestion: Option<&(PathBuf, InstallSource)>) {
    widgets::pill(ui, "GAME FOLDER INVALID", RED);
    ui.add_space(4.0);
    widgets::fitted_title(ui, t("GAME FOLDER NOT USABLE"), 38.0, TEXT_BRIGHT);
    ui.add_space(2.0);
    paragraph(
        ui,
        "The folder in your settings no longer contains Total Annihilation's data files — it may have been \
         moved or uninstalled.",
    );
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        widgets::status_mark(ui, Mark::Error);
        ui.vertical(|ui| {
            path_label(ui, path, TEXT_BRIGHT);
            ui.label(RichText::new(tf("Missing {files}", &[("files", &missing.join(", "))])).color(RED));
        });
    });
    if let Some((root, source)) = suggestion {
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            widgets::status_mark(ui, Mark::Ok);
            ui.vertical(|ui| {
                path_label(ui, root, TEXT_BRIGHT);
                ui.label(RichText::new(tf("Valid install detected · {source}", &[("source", &t(detect::source_label(*source)))])).color(GREEN));
            });
        });
    }
}

/// What the install holds, at least `height` tall; returns the contents' height.
fn content_card(ui: &mut Ui, content: &Content, height: f32) -> f32 {
    widgets::card(ui, |ui| {
        let top = ui.cursor().min.y;
        ui.set_min_height(height);
        widgets::card_title(ui, "DETECTED CONTENT");
        ui.spacing_mut().item_spacing.y = 5.0;
        let items = detect::content_items(content);
        let label = |text: &str, present: bool| RichText::new(text).color(if present { TEXT_BRIGHT } else { STEEL });
        let files = |text: &str| RichText::new(text).font(mono(11.5)).color(STEEL_DIM);
        // The file names are a detail: show them only if every row has room for them.
        let spacing = ui.spacing().item_spacing.x;
        let show_files = items.iter().all(|item| {
            let needed = 18.0 + widgets::galley(ui, label(t(item.label), true)).size().x + widgets::galley(ui, files(item.files)).size().x + 3.0 * spacing + 12.0;
            needed <= ui.available_width()
        });
        for item in items {
            ui.horizontal(|ui| {
                widgets::status_mark(ui, if item.present { Mark::Ok } else { Mark::Absent });
                ui.label(label(t(item.label), item.present));
                if show_files {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(files(item.files));
                    });
                }
            })
            .response
            .hover_tip(item.files);
        }
        // (Where the rows end, whatever the height set above.)
        ui.cursor().min.y - ui.spacing().item_spacing.y - top
    })
    .inner
}

/// A step of getting the game, at least `least` points tall (its contents', returned).
fn step_card(ui: &mut Ui, number: &str, title: &str, text: &str, least: f32, more: impl FnOnce(&mut Ui)) -> f32 {
    widgets::card(ui, |ui| {
        let top = ui.cursor().top();
        ui.set_min_height(least);
        ui.horizontal(|ui| {
            ui.label(RichText::new(number).font(display(30.0)).color(AMBER));
            ui.add_space(4.0);
            ui.label(RichText::new(t(title)).font(strong(14.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
        });
        ui.add(egui::Label::new(RichText::new(t(text)).color(STEEL)).wrap());
        more(ui);
        ui.cursor().top() - top
    })
    .inner
}

/// A saved game or replay in a well. The newest is selected initially; a compact chooser
/// offers recent alternatives and keeps the deliberate choice until its file disappears.
/// Side by side in a `row`, the slots are as tall as the tallest of them, their buttons level
/// at the foot.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn record_slot(ui: &mut Ui, heading: &str, latest: Option<&Record>, older: &[Record], button: &str, flag: &str, ready: bool, mods: &crate::modlist::ModList, row: Option<egui::Id>) -> Option<Record> {
    let mut pressed = None;
    let records: Vec<_> = latest.into_iter().chain(older.iter()).collect();
    // A profile owns its save/replay folder. Scope the choice to that folder, and keep it
    // independent of the responsive column layout so resizing never switches the battle.
    let id = egui::Id::new(("launcher_recent_battle", flag, latest.and_then(|r| r.path.parent())));
    let chosen = ui.ctx().data(|data| data.get_temp::<PathBuf>(id));
    let mut selected = selected_record(&records, chosen.as_deref());
    let frame = egui::Frame::new().fill(INSET).stroke(egui::Stroke::new(1.0, LINE)).corner_radius(egui::CornerRadius::same(3)).inner_margin(egui::Margin::symmetric(14, 12));
    // How much shorter than the row's tallest this slot was last frame: room left above its button.
    let spare = row.map_or(0.0, |row| ui.ctx().data(|d| {
        (d.get_temp::<f32>(row).unwrap_or(0.0) - d.get_temp::<f32>(row.with(flag)).unwrap_or(f32::INFINITY)).max(0.0)
    }));
    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.set_min_height(62.0);
        ui.spacing_mut().item_spacing.y = 3.0;
        let top = ui.min_rect().top();
        ui.label(caption(heading));
        if let Some(index) = selected {
            if records.len() > 1 {
                let width = ui.available_width();
                egui::ComboBox::from_id_salt(("recent_record", flag)).width(width).truncate()
                    .height((ui.ctx().content_rect().height() * 0.65).clamp(100.0, 360.0))
                    .selected_text(RichText::new(&records[index].title).font(strong(15.0)).color(TEXT_BRIGHT))
                    .show_ui(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        for (index, record) in records.iter().enumerate() {
                            let details = record_details(record);
                            let label = format!("{}\n{}", record.title, details);
                            if ui.selectable_label(selected == Some(index), label).hover_tip(record.path.display().to_string()).clicked() {
                                selected = Some(index);
                                ui.ctx().data_mut(|data| data.insert_temp(id, record.path.clone()));
                            }
                        }
                    }).response.hover_tip(t("Choose a recent battle"));
            } else {
                ui.add(egui::Label::new(RichText::new(&records[index].title).font(strong(15.0)).color(TEXT_BRIGHT)).truncate())
                    .hover_tip(records[index].path.display().to_string());
            }
            // Resolve after the chooser so Continue/Watch can never launch the previous
            // selection in the same frame as a keyboard or pointer choice.
            let record = records[selected.unwrap_or(index)];
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(format!("{}  ·  {}", battles::length(record.length), battles::age(record.when))).color(STEEL));
                if record.autosave { widgets::tag(ui, "AUTOSAVE", STEEL); }
                if !record.playable {
                    widgets::tag(ui, "EARLIER VERSION", AMBER);
                }
            });
            if !record.playable {
                ui.add(egui::Label::new(RichText::new(t("Made by an earlier version of the game: it no longer plays")).color(AMBER).size(13.0)).wrap());
            } else if let Some(line) = battles::mod_check(record, mods).and_then(|check| mods_line(&check)) {
                // Its mods: which, when they are as they were; else what is not.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let fine = line.tone == ta_ui::record_mods::Tone::Fine;
                    if !fine {
                        let (rect, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
                        ui.painter().add(metal::lamp(rect.center(), 3.5, line.tone.color(), 1.0));
                    }
                    ui.add(egui::Label::new(RichText::new(&line.text).color(if fine { STEEL_DIM } else { line.tone.color() }).size(13.0)).truncate());
                });
            }
            ui.add_space(4.0 + spare);
            let response = ui.add_enabled(ready && record.playable, ActionButton::secondary(button).min_width(120.0).height(34.0));
            let response = if !ready {
                response.disabled_tip(t("The game is running"))
            } else if !record.playable {
                response.disabled_tip(t("Made by an earlier version of the game: it no longer plays"))
            } else { response };
            if response.clicked() { pressed = Some(record.clone()); }
        } else {
            ui.label(RichText::new(t("None yet")).color(STEEL_DIM));
            ui.add_space(spare);
        }
        if let Some(row) = row {
            let natural = ui.min_rect().bottom() - top - spare;
            ui.ctx().data_mut(|d| d.insert_temp(row.with(flag), natural));
        }
    });
    pressed
}

/// What a slot says of its battle's mods: which, when they are as they were (map packs the
/// battle was made without aside); else the first that is not, and how many more.
fn mods_line(check: &ta_config::records::mods::ModCheck) -> Option<ta_ui::record_mods::Line> {
    ta_ui::record_mods::summary(check)
}

/// What the player chose in [`LauncherApp::battle_dialog`].
enum BattleChoice {
    Switch,
    Get(String),
    Anyway,
    Cancel,
}

fn selected_record(records: &[&Record], chosen: Option<&Path>) -> Option<usize> {
    chosen.and_then(|path| records.iter().position(|record| record.path == path))
        .or_else(|| records.iter().position(|record| record.playable))
        .or_else(|| (!records.is_empty()).then_some(0))
}

fn record_details(record: &Record) -> String {
    let mut details = format!("{}  ·  {}", battles::length(record.length), battles::age(record.when));
    if record.autosave { details.push_str(&format!("  ·  {}", t("AUTOSAVE"))); }
    if !record.playable { details.push_str(&format!("  ·  {}", t("EARLIER VERSION"))); }
    details
}

/// The player's photo behind the banner: filling it (cropped, drifting slowly closer and back
/// on the idle animations' clock `time`), darkened under the words on the left and along the
/// foot.
fn paint_photo(painter: &egui::Painter, rect: Rect, texture: &egui::TextureHandle, time: f64) {
    let size = texture.size_vec2();
    let (want, have) = (rect.width() / rect.height().max(1.0), size.x / size.y.max(1.0));
    let (mut u, mut v) = (1.0f32, 1.0f32);
    if have > want {
        u = want / have;
    } else {
        v = have / want;
    }
    let drift = (0.5 - 0.5 * (time / 30.0 * std::f64::consts::TAU).cos()) as f32;
    let zoom = 1.0 - 0.06 * drift;
    let (u, v) = (u * zoom, v * zoom);
    let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(u, v));
    painter.image(texture.id(), rect, uv, Color32::from_gray(225));
    let words = Rect::from_min_max(rect.min, pos2(rect.min.x + rect.width() * 0.7, rect.max.y));
    widgets::gradient_h(painter, words, Color32::from_black_alpha(230), Color32::from_black_alpha(25));
    widgets::gradient_v(painter, Rect::from_min_max(pos2(rect.min.x, rect.max.y - rect.height() * 0.45), rect.max), Color32::TRANSPARENT, Color32::from_black_alpha(120));
    widgets::gradient_v(painter, Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + 24.0)), Color32::from_black_alpha(90), Color32::TRANSPARENT);
}

/// A radar scope set into the hero: dark glass in a steel bezel, three rings, the beam turning
/// once in six seconds and the contacts it passes glowing after it.
fn radar(c: egui::Pos2, r: f32, time: f64) -> egui::Shape {
    use egui::{Shape, Stroke};
    let green = |a: u8| Color32::from_rgba_unmultiplied(110, 214, 92, a);
    let mut shapes = vec![
        Shape::circle_filled(c, r + 3.0, metal::STEEL_EDGE),
        Shape::circle_filled(c, r, Color32::from_rgb(7, 13, 9)),
    ];
    for k in 1..=3 {
        shapes.push(Shape::circle_stroke(c, r * k as f32 / 3.0, Stroke::new(1.0, green(if k == 3 { 46 } else { 30 }))));
    }
    shapes.push(Shape::line_segment([pos2(c.x - r, c.y), pos2(c.x + r, c.y)], Stroke::new(1.0, green(20))));
    shapes.push(Shape::line_segment([pos2(c.x, c.y - r), pos2(c.x, c.y + r)], Stroke::new(1.0, green(20))));
    let angle = (time / 6.0 % 1.0) as f32 * std::f32::consts::TAU;
    // The beam, with a fading wedge behind it.
    let steps = 24;
    for i in 0..steps {
        let a0 = angle - i as f32 * 0.05;
        let a1 = a0 - 0.05;
        let alpha = (64.0 * (1.0 - i as f32 / steps as f32)) as u8;
        let p0 = c + vec2(a0.cos(), a0.sin()) * r;
        let p1 = c + vec2(a1.cos(), a1.sin()) * r;
        shapes.push(Shape::convex_polygon(vec![c, p0, p1], green(alpha), Stroke::NONE));
    }
    shapes.push(Shape::line_segment([c, c + vec2(angle.cos(), angle.sin()) * r], Stroke::new(1.5, green(150))));
    // A few contacts, lit as the beam passes.
    for (k, (dx, dy)) in [(0.55f32, -0.35f32), (-0.4, 0.5), (0.2, 0.75), (-0.7, -0.2)].iter().enumerate() {
        let p = c + vec2(*dx, *dy) * r;
        let bearing = dy.atan2(*dx);
        let since = (angle - bearing).rem_euclid(std::f32::consts::TAU);
        let glow = (1.0 - since / 3.0).clamp(0.0, 1.0);
        if glow > 0.0 {
            shapes.push(Shape::circle_filled(p, 2.0 + (k % 2) as f32 * 0.8, green((190.0 * glow) as u8)));
        }
    }
    // The bezel's lit rim over the glass's edge.
    shapes.push(Shape::circle_stroke(c, r + 1.5, Stroke::new(1.0, metal::STEEL_SHADE)));
    Shape::Vec(shapes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// A battle made with other mods than those on is asked about before the game starts: its
    /// mods turned on (map packs kept) and it opened, or opened as it is (the game asks nothing
    /// then), or a missing mod's card shown; one made with the mods on opens at once.
    #[test]
    fn a_battle_made_with_other_mods_is_asked_about_before_it_opens() {
        let root = std::env::temp_dir().join(format!("ta_launcher_battle_mods_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (id, version) in [("totalmayhem", "11.3.0"), ("map-pack-1f", "")] {
            std::fs::create_dir_all(root.join("mods").join(id).join("units")).unwrap();
            std::fs::write(root.join("mods").join(id).join("mod.toml"), format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\n")).unwrap();
        }
        let config = root.join("config.toml");
        let mut seed = ta_config::Config::default();
        seed.mods.enabled = vec!["map-pack-1f".into()];
        seed.save_to(&config).unwrap();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]);
        let ctx = egui::Context::default();
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        let mayhem = ta_config::records::RecordMod { id: "totalmayhem".into(), name: "Total Mayhem".into(), version: "11.3.0".into(), digest: [1; 16] };
        let save = Record { path: root.join("saves/mayhem.tasav"), title: "With Mayhem".into(), length: 300, when: std::time::SystemTime::now(), autosave: false, playable: true, mods: Some(vec![mayhem.clone()]) };
        let plays = |app: &mut LauncherApp| app.actions.drain(..).filter_map(|a| match a { Action::PlayWith(args) => Some(args), _ => None }).collect::<Vec<_>>();
        // Off: asked; its mods turned on (the map pack kept) and it opened.
        app.open_battle(save.clone(), "--load");
        assert!(app.battle_prompt.as_ref().is_some_and(|p| p.check.switchable()));
        assert!(plays(&mut app).is_empty(), "nothing starts before the player says");
        app.choose_for_battle(BattleChoice::Switch);
        assert_eq!(app.config.mods.enabled, ["totalmayhem", "map-pack-1f"]);
        assert_eq!(plays(&mut app), [vec![std::ffi::OsString::from("--load"), save.path.clone().into_os_string()]]);
        // On now (and a map pack it was made without): opened at once.
        app.open_battle(save.clone(), "--load");
        assert!(app.battle_prompt.is_none());
        assert_eq!(plays(&mut app).len(), 1);
        // Another version on: asked; opened as it is, the game told the player chose so.
        let mut newer = save.clone();
        newer.mods = Some(vec![ta_config::records::RecordMod { version: "11.2.0".into(), ..mayhem.clone() }]);
        app.open_battle(newer, "--replay");
        assert!(app.battle_prompt.as_ref().is_some_and(|p| !p.check.switchable()));
        app.choose_for_battle(BattleChoice::Anyway);
        assert_eq!(plays(&mut app)[0].last().map(|a| a.to_string_lossy().to_string()), Some("--anyway".into()));
        // Missing: its card on the Mods page.
        let mut missing = save.clone();
        missing.mods = Some(vec![ta_config::records::RecordMod { id: "escalation".into(), name: "TA: Escalation".into(), version: "Gold 10.2.0".into(), digest: [2; 16] }]);
        app.open_battle(missing, "--load");
        app.choose_for_battle(BattleChoice::Get("escalation".into()));
        assert_eq!((app.page, app.mod_library.selected.as_str(), app.mod_library.details_open), (crate::app::Page::Mods, "escalation", true));
        assert!(plays(&mut app).is_empty());
        // A record that names no mods (made before records did) opens as before.
        app.open_battle(Record { mods: None, ..save }, "--load");
        assert!(app.battle_prompt.is_none() && plays(&mut app).len() == 1);
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recent_battle_selection_survives_reordering_and_falls_back_for_missing_files() {
        let make = |name: &str| Record {
            path: PathBuf::from("profiles/commander/saves").join(name),
            title: "Same map — 2 players".into(), length: 120, when: std::time::SystemTime::UNIX_EPOCH,
            autosave: false, playable: true, mods: None,
        };
        let newest = make("newest.tasav");
        let older = make("older.tasav");
        let choices = [&newest, &older];
        assert_eq!(selected_record(&choices, None), Some(0));
        assert_eq!(selected_record(&choices, Some(&older.path)), Some(1));
        assert_eq!(selected_record(&[&older, &newest], Some(&older.path)), Some(0));
        assert_eq!(selected_record(&choices, Some(Path::new("profiles/another/saves/older.tasav"))), Some(0));
        assert_eq!(selected_record(&choices, Some(Path::new("deleted.tasav"))), Some(0));
        assert_eq!(selected_record(&[], Some(&older.path)), None);
    }

    #[test]
    fn compact_play_and_install_help_keep_content_inside_the_viewport() {
        compact_play_layout(false);
        // Exercise the bundled Linux/macOS faces even on Windows, where the
        // narrower system display font would otherwise hide layout regressions.
        compact_play_layout(true);
    }

    fn compact_play_layout(fallback_fonts: bool) {
        let root = std::env::temp_dir().join(format!("ta_launcher_play_compact_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let config = root.join("config.toml");
        let mut seed = ta_config::Config::default();
        seed.graphics.preset = ta_config::Preset::High;
        seed.save_to(&config).unwrap();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]);
        let ctx = egui::Context::default();
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        if fallback_fonts {
            // Finish the initial language-change font refresh before substituting
            // the fixture; otherwise the begin-pass hook restores the host font.
            ctx.run_ui(Default::default(), |_| {}).drop_without_applying_deltas();
            let mut fonts = egui::FontDefinitions::default();
            let regular = fonts.families[&egui::FontFamily::Proportional].clone();
            fonts.families.insert(egui::FontFamily::Name(STRONG_FAMILY.into()), regular.clone());
            fonts.families.insert(egui::FontFamily::Name(DISPLAY_FAMILY.into()), regular);
            ctx.set_fonts(fonts);
        }
        app.skins.shutdown();
        let recent = Record {
            path: root.join("saves/latest.tasav"), title: "A very long battlefield title with a complete saved game description that should fit the chooser".into(),
            length: 3725, when: std::time::SystemTime::now(), autosave: true, playable: true, mods: None,
        };
        app.battles.save = Some(recent.clone());
        app.battles.older_saves = vec![Record { path: root.join("saves/older.tasav"), playable: false, ..recent }];
        for ready in [true, false] {
            for _ in 0..3 {
                let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(420.0, 1800.0))), ..Default::default() };
                let mut output = ctx.run_ui(input, |ui| {
                    if ready { app.ready_hero(ui); app.battles_card(ui); } else { app.setup_page(ui); }
                    assert!(ui.min_rect().right() <= 421.0, "page grew beyond the compact viewport (ready={ready}, fallback_fonts={fallback_fonts}): {:?}", ui.min_rect());
                });
                output.textures_delta.clear();
                for clipped in &output.shapes {
                    if let egui::Shape::Text(text) = &clipped.shape {
                        let rect = text.galley.rect.translate(text.pos.to_vec2());
                        assert!(rect.right().min(clipped.clip_rect.right()) <= 421.0, "text escaped viewport: {rect:?}");
                    }
                }
            }
        }
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
}
