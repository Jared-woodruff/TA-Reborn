//! The About page, made to be enjoyed: the nameplate (the emblem being built by two nano-lathes on
//! a drawing board, the name with the version and the build stamped under it, and where this
//! version stands, with the way to a new one); the ways out to the web as tiles; the credits as a
//! roll of plaques, roles over names, with the game's own credits film a click away; the project
//! in three facts; the licences as three certificates; and the updates' settings beside the
//! launcher's files and folders and this computer (copied for a report in one click).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use egui::{Align, Color32, Layout, Rangef, Rect, RichText, Sense, Shape, Stroke, Ui, UiBuilder, WidgetInfo, WidgetType, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::metal;
use ta_ui::panel::{setting, switch_row};
use ta_ui::tooltip::TooltipExt as _;
use ta_vfs::install;

use super::about_art::{self, Mark, Plaque};
use super::wrapped;
use crate::app::{Action, LauncherApp};
use crate::launch;
use crate::theme::*;
use crate::update::UpdateState;
use crate::widgets::{self, ActionButton};

/// The nameplate's pipe frame (the Play page's banner's).
const PLATE_PIPE: f32 = 4.0;
/// Between the page's parts, and between the tiles of one.
const PART_GAP: f32 = space::XL;
const TILE_GAP: f32 = space::M;
/// The nameplate's updates, in a well at its right in a wide window: how wide.
const UPDATES_WIDTH: f32 = 290.0;
/// How tall a plaque of the credits is, and a tile of the links.
const PLAQUE_HEIGHT: f32 = 88.0;
const LINK_HEIGHT: f32 = 54.0;

impl LauncherApp {
    pub(crate) fn about_page(&mut self, ui: &mut Ui) {
        // (The parts come in a moment after the nameplate, as the Play page's do.)
        let since = self.page_since;
        let part = |ui: &mut Ui, index: usize, add: &mut dyn FnMut(&mut Ui)| ta_ui::panel::stagger(ui, since + 0.04, index, 0.05, vec2(0.0, 8.0), add);
        self.nameplate(ui);
        ui.add_space(space::L);
        part(ui, 0, &mut |ui| self.web_links(ui));
        ui.add_space(PART_GAP);
        part(ui, 1, &mut |ui| self.credits_roll(ui, since));
        ui.add_space(PART_GAP);
        part(ui, 2, &mut |ui| project_facts(ui));
        ui.add_space(PART_GAP);
        part(ui, 3, &mut |ui| self.licences(ui));
        ui.add_space(PART_GAP);
        part(ui, 4, &mut |ui| self.service(ui));
    }

    // ---- The nameplate ---------------------------------------------------------------------

    /// The nameplate: on a drawing board, the emblem being built by two nano-lathes in its gauge
    /// ring; the name, what it is, and the version, the build and the licence stamped under it;
    /// and where this version stands (WHAT'S NEW and CHECK NOW, or the way to a new one) in a well
    /// at its right, or under the words in a narrower window. In a very narrow one the emblem
    /// stands over the words, unless the window is short too (a large interface scale): then it
    /// stands smaller beside them, and the page's parts come into sight sooner.
    fn nameplate(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let short = ui.clip_rect().height() < 460.0;
        let side = width >= 780.0;
        let art_left = width >= 520.0 || (short && width >= 360.0);
        let scale = if width >= 1080.0 { 0.78 } else if side { 0.68 } else if short { 0.46 } else if art_left { 0.6 } else { 0.54 };
        let reach = ta_ui::loading::EMBLEM_AT_WORK_REACH * scale;
        let art = 2.0 * reach;
        let inset = metal::pipe_inset(PLATE_PIPE) + space::L;
        // The words' height the frame before (the plate is made to hold them, the words set in
        // the middle of the emblem's height).
        let measured = ui.id().with(("about_nameplate", side, art_left));
        let words_height: f32 = ui.ctx().data(|d| d.get_temp(measured)).unwrap_or(art);
        let body = if art_left { words_height.max(art) } else { art + space::M + words_height };
        let (rect, _) = ui.allocate_exact_size(vec2(width, body + 2.0 * inset), Sense::hover());
        let ctx = ui.ctx().clone();
        let inside = rect.shrink(PLATE_PIPE + 2.0);
        // (Beside the words and the updates, in the middle of them; over the updates under the
        // words, by the words.)
        let centre = if art_left { pos2(rect.min.x + inset + reach, rect.min.y + inset + if side { body / 2.0 } else { reach + space::S }) } else { pos2(rect.center().x, rect.min.y + inset + reach) };
        // Its ground: the plate, the drawing board laid out round the emblem, the emblem at work.
        let painter = ui.painter().clone();
        painter.add(metal::plate(&ctx, rect, PANEL));
        let board = painter.with_clip_rect(painter.clip_rect().intersect(inside));
        about_art::drawing_board(&board, inside, Some((centre, reach * 0.82)));
        ta_ui::loading::paint_emblem_at_work(&board, centre, scale, self.ambient, ta_ui::motion::still(&ctx));
        ta_ui::motion::next_frame(&ctx, std::time::Duration::from_millis(33));
        // The words, beside the emblem (or under it).
        let content = if art_left {
            Rect::from_min_max(pos2(rect.min.x + inset + art + space::XL, rect.min.y + inset), pos2(rect.max.x - inset, rect.max.y - inset))
        } else {
            Rect::from_min_max(pos2(rect.min.x + inset, rect.min.y + inset + art + space::M), pos2(rect.max.x - inset, rect.max.y - inset))
        };
        let mut words = ui.new_child(UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Min)));
        if art_left && side {
            words.add_space(((body - words_height) / 2.0).max(0.0));
        }
        let top = words.cursor().min.y;
        let well = words.painter().add(Shape::Noop);
        let updates = if side {
            words.horizontal_top(|ui| {
                let room = (ui.available_width() - UPDATES_WIDTH - space::XXL).max(160.0);
                ui.allocate_ui_with_layout(vec2(room, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(room);
                    self.nameplate_words(ui);
                });
                ui.add_space(space::XXL - ui.spacing().item_spacing.x);
                ui.allocate_ui_with_layout(vec2(UPDATES_WIDTH, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(UPDATES_WIDTH);
                    self.nameplate_updates(ui);
                })
                .response
                .rect
            })
            .inner
        } else {
            self.nameplate_words(&mut words);
            words.add_space(space::L);
            words.scope(|ui| self.nameplate_updates(ui)).response.rect
        };
        if side {
            // (The updates in a well of their own, as tall as the words beside them.)
            let words_bottom = words.min_rect().max.y;
            let r = Rect::from_min_max(pos2(updates.min.x - space::M, top - space::S), pos2(updates.max.x + space::M, updates.max.y.max(words_bottom) + space::S));
            words.painter().set(well, metal::well(r, Color32::from_black_alpha(80)));
        }
        let height = words.min_rect().max.y - top;
        if (height - words_height).abs() > 0.5 {
            ctx.data_mut(|d| d.insert_temp(measured, height));
            ctx.request_repaint();
        }
        painter.add(metal::pipe_frame(rect, PLATE_PIPE));
    }

    /// The nameplate's words: the page's name, the game's as the loading screen sets it, what it
    /// is, and the version, the build and the licence stamped under them.
    fn nameplate_words(&mut self, ui: &mut Ui) {
        ui.label(RichText::new(t("ABOUT")).font(strong(12.0)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(2.4)));
        let size = if ui.available_width() >= 420.0 { 40.0 } else { 32.0 };
        let title = widgets::fitted_galley(ui, |s| RichText::new(t("TOTAL ANNIHILATION")).font(display(s)).extra_letter_spacing(ta_ui::i18n::tracking(1.4)), size, ui.available_width(), 0.55);
        let (at, name) = ui.allocate_exact_size(title.size(), Sense::hover());
        name.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, "Total Annihilation Reborn"));
        metal::embossed(ui.painter(), at.min, title, TEXT_BRIGHT);
        ui.add_space(-6.0);
        let reborn = widgets::fitted_galley(ui, |s| RichText::new(t("REBORN")).font(display(s)).extra_letter_spacing(ta_ui::i18n::tracking(7.0)), size * 0.68, ui.available_width(), 0.6);
        let (at, _) = ui.allocate_exact_size(reborn.size(), Sense::hover());
        metal::embossed(ui.painter(), at.min, reborn, AMBER);
        ui.add_space(space::XS);
        widgets::pretty_label(ui, RichText::new(t("An unofficial reimplementation of Total Annihilation")).font(body(14.5)).color(TEXT));
        ui.add_space(space::S);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            stamp(ui, &tf("VERSION {version}", &[("version", &env!("CARGO_PKG_VERSION"))]), CREAM);
            let commit = env!("TA_BUILD_COMMIT");
            if !commit.is_empty() {
                stamp(ui, &tf("BUILD {commit}", &[("commit", &commit)]), STEEL);
            }
            stamp(ui, "GPL-3.0-or-later", STEEL);
        });
    }

    /// Where this version stands: a lamp and its words, and the way on: WHAT'S NEW and CHECK NOW,
    /// or the way to a new version (the Play page's controls).
    fn nameplate_updates(&mut self, ui: &mut Ui) {
        let available = match &self.update {
            UpdateState::Available(r) => Some(r.clone()),
            _ => None,
        };
        let offline = self.offline && !self.config.updates.url.trim_start().starts_with("file:");
        let colour = match &self.update {
            UpdateState::Available(_) => SKY,
            _ if offline => STEEL,
            UpdateState::Failed(_) => RED,
            UpdateState::UpToDate => GREEN,
            UpdateState::Checking(_) => AMBER,
            _ => STEEL,
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (lamp, _) = ui.allocate_exact_size(vec2(12.0, 16.0), Sense::hover());
            ui.painter().add(metal::lamp(lamp.center(), 4.0, colour, if colour == STEEL { 0.35 } else { 1.0 }));
            ui.label(RichText::new(t("UPDATES")).font(strong(12.0)).color(if colour == STEEL { STEEL } else { colour }).extra_letter_spacing(ta_ui::i18n::tracking(2.0)));
        });
        ui.add_space(2.0);
        if offline && available.is_none() {
            wrapped(ui, t("Offline mode: online update checks are paused for this session."), STEEL);
        } else {
            wrapped(ui, &self.update.describe(), if colour == STEEL { STEEL } else { TEXT });
        }
        if let Some(r) = &available
            && !r.title.is_empty()
        {
            wrapped(ui, &r.title, TEXT_BRIGHT);
        }
        ui.add_space(space::S);
        if let Some(r) = &available {
            self.update_controls(ui, r);
        } else {
            ui.horizontal_wrapped(|ui| {
                if ui.add(ActionButton::secondary("WHAT'S NEW")).hover_tip(t("Everything this version brings")).clicked() {
                    self.open_whats_new();
                }
                let checking = matches!(self.update, UpdateState::Checking(_));
                let source = !self.config.updates.url.trim().is_empty() || !crate::update::DEFAULT_SOURCE.is_empty();
                if ui.add_enabled(!offline && source && !checking && self.download.is_none(), ActionButton::secondary("CHECK NOW")).clicked() {
                    self.push(Action::CheckUpdates);
                }
            });
        }
    }

    // ---- The web --------------------------------------------------------------------------

    /// The ways out to the web: the website, the source, and the original game's stores, as
    /// tiles (four across, two in a narrower window, one in the narrowest).
    fn web_links(&mut self, ui: &mut Ui) {
        let tiles = [
            (Mark::Globe, n("WEBSITE"), crate::app::WEBSITE_URL),
            (Mark::Source, n("SOURCE CODE"), crate::app::PROJECT_URL),
            (Mark::Store, "GOG.COM", install::GOG_STORE_URL),
            (Mark::Store, "STEAM", install::STEAM_STORE_URL),
        ];
        let width = ui.available_width();
        let across = if width >= 760.0 { 4 } else if width >= 360.0 { 2 } else { 1 };
        let size = vec2(((width - TILE_GAP * (across - 1) as f32) / across as f32).floor(), LINK_HEIGHT);
        for (k, row) in tiles.chunks(across).enumerate() {
            if k > 0 {
                ui.add_space(TILE_GAP - ui.spacing().item_spacing.y);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = TILE_GAP;
                for &(mark, label, url) in row {
                    if about_art::link_tile(ui, mark, label, &address(url), size).hover_tip(url).clicked() {
                        self.push(Action::OpenUrl(url.into()));
                    }
                }
            });
        }
    }

    // ---- The credits ------------------------------------------------------------------------

    /// The credits as a roll of plaques, roles over names: TA Reborn's team (a role's names on
    /// one plaque; the project lead's page a click away), its music, and the game it plays; each
    /// built in by a nano-lathe as it first comes into sight. Over them, the way to the game's own
    /// credits film (every team, TA's and TA Reborn's, in battle and to its song); under them what
    /// it is built with, and thanks.
    fn credits_roll(&mut self, ui: &mut Ui, since: f64) {
        use ta_ui::credits::{BAND, PROJECT_LEAD_URL, SONG, TEAM};
        let installed = self.detection.install().is_some();
        let running = self.game.is_running();
        let mut watch = false;
        section_heading(ui, "CREDITS", |ui| {
            if installed {
                let button = ui.add_enabled(!running, ActionButton::secondary("WATCH THE CREDITS").height(28.0));
                let button = if running {
                    button.disabled_tip(t("The game is running. Close it to launch again."))
                } else {
                    button.hover_tip(t("Opens the game on its credits: every team, TA's and TA Reborn's, filmed in battle to their song"))
                };
                watch = button.clicked();
            }
        });
        if watch {
            self.push(Action::PlayWith(vec!["--open".into(), "credits".into()]));
        }
        let page = PROJECT_LEAD_URL.trim_start_matches("https://");
        let song = format!("\u{201c}{SONG}\u{201d}");
        let original = t("Total Annihilation — Cavedog Entertainment, 1997. The game's credits (main menu) roll its teams, read from your install");
        let mut plaques: Vec<Plaque> = Vec::new();
        for (i, credit) in TEAM.iter().enumerate() {
            if i > 0 && TEAM[i - 1].role == credit.role
                && let Some(last) = plaques.last_mut()
            {
                last.names.push(credit.name);
                last.spoken = format!("{}; {}", last.spoken, credit.text());
                continue;
            }
            let lead = credit.name == ta_ui::credits::PROJECT_LEAD;
            plaques.push(Plaque {
                mark: match credit.role {
                    "PROJECT LEAD" => Mark::Lead,
                    "DEV" => Mark::Dev,
                    "QA" => Mark::Qa,
                    _ => Mark::Voice,
                },
                role: t(credit.role),
                names: vec![credit.name],
                detail: lead.then_some(page),
                link: lead.then_some(PROJECT_LEAD_URL),
                spoken: if lead { format!("{}, {page}", credit.text()) } else { credit.text() },
                tip: lead.then_some(PROJECT_LEAD_URL),
            });
        }
        plaques.push(Plaque { mark: Mark::Music, role: t("MUSIC"), names: vec![BAND], detail: Some(&song), link: None, spoken: format!("{}: {BAND}, {song}", t("MUSIC")), tip: None });
        plaques.push(Plaque {
            mark: Mark::Disc,
            role: t("ORIGINAL GAME"),
            names: vec!["Total Annihilation"],
            detail: Some("Cavedog Entertainment · 1997"),
            link: None,
            spoken: format!("{}: {original}", t("ORIGINAL GAME")),
            tip: Some(original),
        });
        // (Three across where each is wide enough for its names at their size, else two, else one.)
        let width = ui.available_width();
        let across = (1..=3).rev().find(|&k| (width - TILE_GAP * (k - 1) as f32) / k as f32 >= if k == 1 { 0.0 } else { 330.0 }).unwrap_or(1);
        let size = vec2(((width - TILE_GAP * (across - 1) as f32) / across as f32).floor(), PLAQUE_HEIGHT);
        // (Every plaque's names one size: the largest at which the longest fits.)
        let widest = plaques.iter().flat_map(|p| &p.names).map(|name| widgets::galley(ui, RichText::new(*name).font(strong(17.0))).size().x).fold(1.0, f32::max);
        let name_size = (17.0 * (about_art::plaque_room(size.x) / widest)).clamp(13.0, 17.0);
        let mut open_page = false;
        for (r, row) in plaques.chunks(across).enumerate() {
            if r > 0 {
                ui.add_space(TILE_GAP - ui.spacing().item_spacing.y);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = TILE_GAP;
                for (c, plaque) in row.iter().enumerate() {
                    let k = r * across + c;
                    // (Built in turn as they come into sight: along a row, then the next.)
                    let built = about_art::built(ui, ui.id().with(("about_plaque", k)), Rect::from_min_size(ui.cursor().min, size), since, 0.07 * k as f32);
                    let response = about_art::plaque(ui, plaque, size, name_size, built);
                    let response = match plaque.tip {
                        Some(tip) => response.hover_tip(tip),
                        None => response,
                    };
                    if plaque.link.is_some() && response.clicked() {
                        open_page = true;
                    }
                }
            });
        }
        if open_page {
            self.push(Action::OpenUrl(PROJECT_LEAD_URL.into()));
        }
        ui.add_space(space::L);
        let labels = [n("BUILT WITH"), n("THANKS")];
        let width = super::caption_width(ui, &labels);
        super::field_with(ui, labels[0], width, |ui| {
            wrapped(ui, "Rust · wgpu · winit · egui / eframe · symphonia · cpal", TEXT);
        });
        super::field_with(ui, labels[1], width, |ui| {
            wrapped(ui, t("The Total Annihilation community, whose decades of file-format research made this possible"), TEXT);
        });
    }

    // ---- The licences ------------------------------------------------------------------------

    /// The licences as three certificates side by side (one over another in a narrow window): TA
    /// Reborn's own (GPL-3.0-or-later; the licence and the third-party notices the package ships,
    /// when they are there), Total Annihilation's owners', and that no asset of the game is in it.
    fn licences(&mut self, ui: &mut Ui) {
        section_heading(ui, "LEGAL", |_| {});
        let (licence, notices) = legal_files(&launch::app_dir().unwrap_or_default());
        let statements = [
            (Mark::Licence, n("FREE SOFTWARE"), n("TA Reborn is free software under the GNU General Public License, version 3 or any later version: you may share and change it on its terms. It comes with no warranty. The open-source libraries it is built with keep their own licences, listed in THIRD-PARTY-NOTICES.txt.")),
            (Mark::Seal, n("TOTAL ANNIHILATION"), n("Total Annihilation is © its respective owners. TA Reborn is an unofficial, non-commercial fan project; it is not affiliated with or endorsed by the owners of Total Annihilation.")),
            (Mark::Disc, n("NO ORIGINAL ASSETS"), n("This project contains no original game assets — no units, maps, artwork, sound, music or video. It requires a legitimately owned copy of the game, such as \"Total Annihilation: Commander Pack\" from GOG.com or Steam.")),
        ];
        let across = if ui.available_width() >= 720.0 { 3 } else { 1 };
        let mut open = None;
        even_rows(ui, "about_certificates", statements.len(), across, |i, ui, least| {
            let (mark, title, text) = statements[i];
            certificate(ui, mark, title, text, least, |ui| {
                if i == 0 && (licence.is_file() || notices.is_file()) {
                    ui.add_space(space::S);
                    ui.horizontal_wrapped(|ui| {
                        if licence.is_file() && ui.add(ActionButton::secondary("LICENCE")).hover_tip(licence.display().to_string()).clicked() {
                            open = Some(licence.clone());
                        }
                        if notices.is_file() && ui.add(ActionButton::secondary("THIRD-PARTY NOTICES")).hover_tip(notices.display().to_string()).clicked() {
                            open = Some(notices.clone());
                        }
                    });
                }
            })
        });
        if let Some(path) = open {
            self.push(Action::OpenPath(path));
        }
    }

    // ---- The service panel ---------------------------------------------------------------------

    /// The updates' settings beside the launcher's files and folders and this computer (two
    /// cards side by side, one over the other in a narrow window).
    fn service(&mut self, ui: &mut Ui) {
        let across = if ui.available_width() >= 720.0 { 2 } else { 1 };
        even_rows(ui, "about_service", 2, across, |i, ui, least| {
            let margin = 2.0 * widgets::card_margin();
            widgets::card(ui, |ui| {
                let top = ui.cursor().min.y;
                if i == 0 {
                    self.updates_settings(ui);
                    ui.add_space(space::L);
                    self.this_computer(ui);
                } else {
                    self.files_and_folders(ui);
                }
                stretch(ui, top, least - margin) + margin
            })
            .inner
        });
    }

    /// Whether the launcher looks for updates, and where (for testers and builds of one's own).
    fn updates_settings(&mut self, ui: &mut Ui) {
        widgets::card_title(ui, "UPDATES");
        if switch_row(ui, "CHECK FOR UPDATES", "When the launcher starts: asks the update source for the newest version", &mut self.config.updates.check) {
            self.push(Action::RefreshUpdates);
        }
        egui::CollapsingHeader::new(widgets::caption("ADVANCED")).id_salt("updates_advanced").default_open(!self.config.updates.url.is_empty()).show(ui, |ui| {
            setting(ui, "UPDATE SOURCE", Some("A TOML file with version and url, or a GitHub latest-release API address; empty: the build's own"), |ui| {
                let hint = if crate::update::DEFAULT_SOURCE.is_empty() { t("None set") } else { crate::update::DEFAULT_SOURCE };
                let field = ui.add(egui::TextEdit::singleline(&mut self.config.updates.url).hint_text(hint).desired_width(ui.available_width().min(460.0)));
                // (Asked again once the address is typed.)
                if field.lost_focus() {
                    self.push(Action::RefreshUpdates);
                }
            });
        });
    }

    /// The launcher's files and folders, each with its way to the folder (the path kept to one
    /// line, its end in sight).
    fn files_and_folders(&mut self, ui: &mut Ui) {
        // (label, shown path, folder opened by the button)
        let rows = [
            (n("SETTINGS"), self.store.path.clone(), self.store.path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf()),
            (n("USER MODS"), self.store.dir().join("mods"), self.store.dir().join("mods")),
            (n("GAME LOG"), self.log_dir().join("game.log"), self.log_dir()),
            (n("PROGRAM"), launch::app_dir().unwrap_or_default(), launch::app_dir().unwrap_or_default()),
        ];
        widgets::card_title(ui, "FILES & FOLDERS");
        let labels: Vec<&str> = rows.iter().map(|r| r.0).collect();
        let width = super::caption_width(ui, &labels);
        for (label, path, folder) in rows {
            super::field_with(ui, label, width, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if ui.add(ActionButton::secondary("OPEN").height(26.0)).hover_tip(folder.display().to_string()).clicked() {
                        self.push(Action::OpenFolder(folder.clone()));
                    }
                    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                        let full = path.display().to_string();
                        let shown = path_tail(ui, &full, ui.available_width());
                        ui.add(egui::Label::new(RichText::new(shown).font(mono(12.5)).color(TEXT)).truncate()).hover_tip(&full);
                    });
                });
            });
        }
    }

    /// This computer as the launcher found it, and what a bug report wants to know copied in a
    /// click.
    fn this_computer(&mut self, ui: &mut Ui) {
        widgets::card_title(ui, "THIS COMPUTER");
        let computer = computer(ui.ctx());
        // (As the benchmark's report has them; "…" until the computer is looked at.)
        let gb = |mb: u64| tf("{gb} GB", &[("gb", &format!("{:.0}", mb as f32 / 1024.0))]);
        let (card, processor, memory) = computer.map_or_else(Default::default, |h| {
            (
                if h.vram_mb > 0 { format!("{} · {}", h.gpu, gb(h.vram_mb)) } else { h.gpu.clone() },
                if h.threads > 0 { tf("{cpu} · {threads} threads", &[("cpu", &h.cpu), ("threads", &h.threads)]) } else { h.cpu.clone() },
                if h.ram_mb > 0 { gb(h.ram_mb) } else { String::new() },
            )
        });
        let labels = [n("GRAPHICS CARD"), n("PROCESSOR"), n("MEMORY")];
        let width = super::caption_width(ui, &labels);
        for (label, value) in labels.into_iter().zip([card, processor, memory]) {
            super::field_with(ui, label, width, |ui| {
                wrapped(ui, if value.trim_start_matches(" · ").is_empty() { "…" } else { &value }, TEXT);
            });
        }
        ui.add_space(space::XS);
        if ui.add(ActionButton::secondary("COPY SYSTEM INFO")).hover_tip(t("The version, this computer, the game's folder and the mods in use, for a bug report")).clicked() {
            let report = self.system_report(computer);
            self.push(Action::Copy(report));
        }
    }

    /// What a bug report wants to know, in English (it is for the project): the version, this
    /// computer, the game's data, the settings that matter and the mods in use.
    fn system_report(&self, computer: Option<&ta_config::hardware::Hardware>) -> String {
        let gb = |mb: u64| format!("{:.0} GB", mb as f64 / 1024.0);
        let mut lines = vec![format!("TA Reborn {}", crate::version_label()), format!("System: {} {}", std::env::consts::OS, std::env::consts::ARCH)];
        if let Some(hw) = computer {
            if !hw.gpu.is_empty() {
                lines.push(format!("Graphics card: {}{}", hw.gpu, if hw.vram_mb >= 1024 { format!(" ({})", gb(hw.vram_mb)) } else { String::new() }));
            }
            if !hw.cpu.is_empty() || hw.threads > 0 {
                lines.push(format!("Processor: {} ({} threads)", if hw.cpu.is_empty() { "?" } else { hw.cpu.as_str() }, hw.threads));
            }
            if hw.ram_mb > 0 {
                lines.push(format!("Memory: {}", gb(hw.ram_mb)));
            }
            if let Some((w, h)) = hw.screen {
                lines.push(format!("Screen: {w}x{h}{}", if hw.refresh_hz > 0 { format!(" at {} Hz", hw.refresh_hz) } else { String::new() }));
            }
        }
        lines.push(match self.detection.install() {
            Some(i) => format!(
                "Game data: {} ({}{}{})",
                i.root.display(),
                i.source,
                i.store.name().map(|s| format!(", {s}")).unwrap_or_default(),
                i.build.as_ref().map(|b| format!(" build {b}")).unwrap_or_default()
            ),
            None => "Game data: not found".into(),
        });
        let g = &self.config.graphics;
        lines.push(format!("Graphics: {} at {}x{}, {:?}", g.preset.label(), g.width, g.height, g.window_mode));
        let language = &self.config.gameplay.language;
        lines.push(format!("Language: {}, interface scale {:.0}%", if language.is_empty() { "English" } else { language }, self.config.gameplay.ui_scale * 100.0));
        let mods = &self.config.mods.enabled;
        lines.push(if mods.is_empty() { "Mods: none".into() } else { format!("Mods: {}", mods.join(", ")) });
        lines.join("\n") + "\n"
    }
}

// ---- The parts' pieces ----------------------------------------------------------------------

/// A part's heading: its title in cream capitals after a rivet (as a card's), and a pipe running
/// on from it to what `right` adds at the far end (a button), or to the edge.
fn section_heading(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    let row = Rect::from_min_size(ui.cursor().min, vec2(width, 30.0));
    let mut left = ui.new_child(UiBuilder::new().max_rect(row).layout(Layout::top_down(Align::Min)));
    left.add_space(7.0);
    widgets::card_title(&mut left, title);
    let title_end = left.min_rect().max.x;
    let mut end = ui.new_child(UiBuilder::new().max_rect(row).layout(Layout::right_to_left(Align::Center)));
    right(&mut end);
    let used = end.min_rect();
    let rule_end = if used.width() > 1.0 { used.min.x - space::M } else { row.max.x };
    let start = title_end + space::M;
    if rule_end > start + 16.0 {
        ui.painter().add(metal::pipe_rule(Rangef::new(start, rule_end), row.min.y + 15.0, 2.0, None));
    }
    ui.allocate_exact_size(vec2(width, row.height()), Sense::hover());
    ui.add_space(space::S);
}

/// A word stamped on the nameplate: engraved letters in a recessed slot.
fn stamp(ui: &mut Ui, text: &str, colour: Color32) {
    let galley = widgets::galley(ui, RichText::new(text).font(strong(11.0)).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, 22.0), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    let painter = ui.painter();
    painter.add(metal::well(rect, Color32::from_black_alpha(90)));
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, colour.gamma_multiply(0.35)), egui::StrokeKind::Inside);
    metal::embossed(painter, (rect.center() - galley.size() / 2.0).round(), galley, colour);
}

/// The project in three facts, side by side (one over another in a narrow window): written anew,
/// what modern hardware allows, and the player's own copy, as it was.
fn project_facts(ui: &mut Ui) {
    section_heading(ui, "THE PROJECT", |_| {});
    let facts = [
        (Mark::Nib, n("WRITTEN ANEW"), n("A clean-room reimplementation of the Total Annihilation engine, written from scratch in Rust, that keeps the 1997 classic's rules and content.")),
        (Mark::Chip, n("BUILT FOR TODAY"), n("High resolutions, a free perspective camera, dynamic lighting, larger armies and first-class mod support.")),
        (Mark::Lock, n("YOUR COPY, UNTOUCHED"), n("It reads the archives of your own installation directly — nothing is copied, converted or modified, and the original game keeps working as before.")),
    ];
    let across = if ui.available_width() >= 720.0 { 3 } else { 1 };
    even_rows(ui, "about_facts", facts.len(), across, |i, ui, least| {
        let (mark, title, text) = facts[i];
        certificate(ui, mark, title, text, least, |_| {})
    });
}

/// A certificate: a riveted plate with its mark in a well beside its title, its words under them,
/// and what `more` adds at its foot (buttons); at least `least` tall. Says how tall it is.
fn certificate(ui: &mut Ui, mark: Mark, title: &str, text: &str, least: f32, more: impl FnOnce(&mut Ui)) -> f32 {
    let background = ui.painter().add(Shape::Noop);
    let pad = space::L + 2.0;
    let inner = egui::Frame::new().inner_margin(egui::Margin::same(pad as i8)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        let top = ui.cursor().min.y;
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::M;
                let (well, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                about_art::mark_well(ui.painter(), mark, well.center(), 15.0, 0.0);
                let label = t(title);
                let galley = widgets::wrapped_galley(ui, RichText::new(label).font(strong(13.5)).extra_letter_spacing(ta_ui::i18n::tracking(1.6)), ui.available_width());
                let (at, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label));
                metal::embossed(ui.painter(), at.min, galley, CREAM);
            });
            ui.add_space(space::S);
            widgets::pretty_label(ui, RichText::new(t(text)).color(TEXT));
            more(ui);
        });
        stretch(ui, top, least - 2.0 * pad) + 2.0 * pad
    });
    ui.painter().set(background, metal::riveted_plate(ui.ctx(), inner.response.rect, PANEL, 0.6));
    inner.inner
}

/// Room added under what was drawn since `top` to make it `least` tall; how tall it was itself.
fn stretch(ui: &mut Ui, top: f32, least: f32) -> f32 {
    let own = ui.min_rect().max.y - top;
    if least > own {
        ui.add_space(least - own);
    }
    own
}

/// `count` parts in rows of `across`, each as tall as the tallest of its row's contents (as
/// measured the frame before): `add` draws part `i` at least `least` tall and says how tall its
/// own contents are.
fn even_rows(ui: &mut Ui, id: &str, count: usize, across: usize, mut add: impl FnMut(usize, &mut Ui, f32) -> f32) {
    let across = across.max(1);
    let key = ui.id().with((id, across));
    let natural: Vec<f32> = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_else(|| vec![0.0; count]);
    let mut measured = vec![0.0; count];
    for start in (0..count).step_by(across) {
        let end = (start + across).min(count);
        if across == 1 {
            measured[start] = add(start, ui, 0.0);
        } else {
            let tallest = natural[start..end].iter().copied().fold(0.0, f32::max);
            let heights = widgets::columns(ui, end - start, |columns| (start..end).zip(columns.iter_mut()).map(|(i, column)| add(i, column, tallest)).collect::<Vec<f32>>());
            measured[start..end].copy_from_slice(&heights);
        }
        if end < count {
            ui.add_space(TILE_GAP);
        }
    }
    if measured.iter().zip(&natural).any(|(m, n)| (m - n).abs() > 0.5) {
        ui.ctx().data_mut(|d| d.insert_temp(key, measured));
        ui.ctx().request_repaint();
    }
}

/// A link's address as a tile shows it: no scheme, no "www.", a store's page by its site alone.
fn address(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r).trim_start_matches("www.").trim_end_matches('/');
    match rest.split_once('/') {
        Some((host, _)) if host.contains("gog.com") || host.contains("steampowered") => host.to_string(),
        _ => rest.to_string(),
    }
}

/// `path` cut to fit `room` (in a path's font) from its start, its end kept in sight: "…\TA
/// Reborn\mods".
fn path_tail(ui: &Ui, path: &str, room: f32) -> String {
    let fits = |text: &str| widgets::galley(ui, RichText::new(text).font(mono(12.5))).size().x <= room;
    if fits(path) {
        return path.to_string();
    }
    let separator = if path.contains('\\') { '\\' } else { '/' };
    let parts: Vec<&str> = path.split(separator).collect();
    for k in 1..parts.len() {
        let tail = format!("…{separator}{}", parts[k..].join(&separator.to_string()));
        if fits(&tail) {
            return tail;
        }
    }
    parts.last().map_or_else(String::new, |last| format!("…{separator}{last}"))
}

/// This computer, looked at once (on a thread of its own: it asks Windows) the first time the
/// page wants it; nothing until it is known.
fn computer(ctx: &egui::Context) -> Option<&'static ta_config::hardware::Hardware> {
    static FOUND: OnceLock<ta_config::hardware::Hardware> = OnceLock::new();
    static ASKED: std::sync::Once = std::sync::Once::new();
    if let Some(found) = FOUND.get() {
        return Some(found);
    }
    ASKED.call_once(|| {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = FOUND.set(ta_config::hardware::Hardware::detect());
            ctx.request_repaint();
        });
    });
    None
}

/// Legal documents are package resources: beside portable binaries, or in a macOS
/// bundle's Resources directory. Resolve from the executable, independent of cwd.
fn legal_files(app_dir: &Path) -> (PathBuf, PathBuf) {
    let resources = ta_vfs::bundle_resource_dir(app_dir).unwrap_or_else(|| app_dir.to_path_buf());
    (resources.join("LICENSE.txt"), resources.join("THIRD-PARTY-NOTICES.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_documents_resolve_from_bundle_and_portable_resources() {
        let root = std::env::temp_dir().join(format!("ta_about_resources_{}", std::process::id()));
        let contents = root.join("TA Reborn.app/Contents");
        for resources in [contents.join("Resources"), root.join("portable")] {
            std::fs::create_dir_all(&resources).unwrap();
            for name in ["LICENSE.txt", "THIRD-PARTY-NOTICES.txt"] {
                std::fs::write(resources.join(name), "packaged legal document").unwrap();
            }
        }
        for (programs, resources) in [(contents.join("MacOS"), contents.join("Resources")), (root.join("portable"), root.join("portable"))] {
            let (licence, notices) = legal_files(&programs);
            assert_eq!(licence, resources.join("LICENSE.txt"));
            assert_eq!(notices, resources.join("THIRD-PARTY-NOTICES.txt"));
            assert!(licence.is_file() && notices.is_file(), "both About-page document buttons must be available");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn addresses_read_as_the_tiles_show_them() {
        assert_eq!(address(crate::app::WEBSITE_URL), "ta-reborn.com");
        assert_eq!(address(crate::app::PROJECT_URL), "github.com/Jared-woodruff/TA-Reborn");
        assert_eq!(address(install::GOG_STORE_URL), "gog.com");
        assert_eq!(address(install::STEAM_STORE_URL), "store.steampowered.com");
    }

    /// A launcher on its About page, its settings in a folder of its own.
    fn about_launcher(name: &str) -> (egui::Context, LauncherApp, PathBuf) {
        use clap::Parser as _;
        let dir = std::env::temp_dir().join(format!("ta_about_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.toml");
        let ctx = egui::Context::default();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--page", "about", "--config", config.to_str().unwrap()]);
        let app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        (ctx, app, dir)
    }

    /// What a screen reader is told of the page after `frames` frames in a window `size` across:
    /// each node's name, where it is, and whether the keys can reach it and press it.
    fn spoken(ctx: &egui::Context, app: &mut LauncherApp, size: egui::Vec2, frames: usize) -> Vec<(String, egui::accesskit::Rect, bool)> {
        use eframe::App as _;
        ctx.enable_accesskit();
        let mut frame = eframe::Frame::_new_kittest();
        let mut nodes = Vec::new();
        for _ in 0..frames {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
            let mut output = ctx.run_ui(raw, |ui| {
                app.logic(ui.ctx(), &mut frame);
                app.ui(ui, &mut frame);
            });
            output.textures_delta.clear();
            nodes = output.platform_output.accesskit_update.map(|tree| tree.nodes).unwrap_or_default();
        }
        // (A label's words are its value; a control's, its label.)
        let pressable = |node: &egui::accesskit::Node| node.supports_action(egui::accesskit::Action::Focus) && node.supports_action(egui::accesskit::Action::Click);
        nodes.into_iter().filter_map(|(_, node)| Some((node.label().or(node.value())?.to_string(), node.bounds()?, pressable(&node)))).collect()
    }

    /// Everything the page offers is there at each of the launcher's window sizes, by the name a
    /// screen reader gives it, and inside the window: the ways out (the website, the source, the
    /// stores), the credits as credited (the team's lines, the project lead's page, the music, the
    /// original game), the updates' buttons and setting, the files' folders and the report. What
    /// can be pressed, the keys reach (Tab) and press.
    #[test]
    fn the_about_page_offers_everything_by_name_at_every_size() {
        let (ctx, mut app, dir) = about_launcher("names");
        app.skins.shutdown();
        app.config.gameplay.reduced_motion = true;
        let team = ta_ui::credits::contributors();
        for size in [vec2(940.0, 600.0), vec2(1120.0, 720.0), vec2(1600.0, 900.0), vec2(480.0, 720.0)] {
            let nodes = spoken(&ctx, &mut app, size, 4);
            let find = |name: &str| nodes.iter().find(|(label, _, _)| label.as_str() == name || label.starts_with(&format!("{name}:")) || label.starts_with(&format!("{name},")));
            let mut wanted: Vec<String> = ["WEBSITE: ta-reborn.com", "SOURCE CODE", "GOG.COM: gog.com", "STEAM: store.steampowered.com", "WHAT'S NEW", "CHECK NOW", "CHECK FOR UPDATES", "COPY SYSTEM INFO", "OPEN", "Total Annihilation Reborn"]
                .into_iter()
                .map(String::from)
                .collect();
            wanted.push(format!("{}, github.com/Jared-woodruff", team[0]));
            wanted.extend(team[1..3].iter().cloned());
            wanted.push(format!("{}; {}", team[3], team[4]));
            wanted.push("MUSIC: Hadal Static, \u{201c}The War Reborn\u{201d}".into());
            wanted.push("ORIGINAL GAME".into());
            if app.detection.install().is_some() {
                wanted.push("WATCH THE CREDITS".into());
            }
            for name in &wanted {
                let (_, at, _) = find(name).unwrap_or_else(|| panic!("{size:?}: nothing named {name:?} in {:?}", nodes.iter().map(|n| &n.0).collect::<Vec<_>>()));
                assert!(at.x1 <= size.x as f64 + 0.5 && at.x0 >= -0.5, "{size:?}: {name:?} runs out of the window: {at:?}");
            }
            for name in ["WEBSITE", "SOURCE CODE", "GOG.COM", "STEAM", team[0].as_str(), "WHAT'S NEW", "COPY SYSTEM INFO", "OPEN"] {
                assert!(find(name).is_some_and(|n| n.2), "{size:?}: {name:?} can be reached and pressed with the keys");
            }
        }
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The system report says what a bug report needs: the version, the game's data, the
    /// settings that matter and the mods in use.
    #[test]
    fn the_system_report_holds_what_a_bug_report_needs() {
        let (_ctx, mut app, dir) = about_launcher("report");
        app.skins.shutdown();
        app.config.mods.enabled = vec!["escalation".into()];
        let report = app.system_report(Some(&ta_config::hardware::Hardware { gpu: "Test GPU".into(), vram_mb: 8192, threads: 8, ram_mb: 16384, cpu: "Test CPU".into(), ..Default::default() }));
        for part in [format!("TA Reborn {}", crate::version_label()), "Graphics card: Test GPU (8 GB)".into(), "Processor: Test CPU (8 threads)".into(), "Memory: 16 GB".into(), "Game data: ".into(), "Mods: escalation".into()] {
            assert!(report.contains(&part), "{part:?} in {report}");
        }
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A path cut to fit keeps its end in sight; one that fits is whole.
    #[test]
    fn a_long_path_keeps_its_end() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let path = r"C:\Users\Someone\AppData\Roaming\TA Reborn\profiles\0123456789abcdef\config.toml";
        let mut cut = (String::new(), String::new());
        ctx.run_ui(egui::RawInput::default(), |ui| {
            cut = (path_tail(ui, path, 4000.0), path_tail(ui, path, 220.0));
        })
        .drop_without_applying_deltas();
        assert_eq!(cut.0, path);
        assert!(cut.1.starts_with('…') && cut.1.ends_with(r"\config.toml") && cut.1.len() < path.len(), "{}", cut.1);
    }

    /// Under reduced motion the plaques stand built at once.
    #[test]
    fn plaques_are_built_at_once_under_reduced_motion() {
        let ctx = egui::Context::default();
        ta_ui::motion::set_reduced(&ctx, true);
        let mut built = 0.0;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            built = about_art::built(ui, ui.id(), Rect::from_min_size(egui::Pos2::ZERO, vec2(200.0, 80.0)), 0.0, 0.5);
        })
        .drop_without_applying_deltas();
        assert_eq!(built, 1.0);
    }
}
