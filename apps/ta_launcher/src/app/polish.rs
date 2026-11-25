//! Discoverable shortcuts and background activity shared by every launcher page.
use ta_ui::tooltip::TooltipExt as _;
use egui::{Align, Align2, Color32, Modifiers, Rect, RichText, Ui, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{Exit, Panel};
use super::{Action, LauncherApp, Page, ModsTab, page_label};
use crate::getmods::Stage;
use crate::theme::*;
use crate::widgets;
use crate::update::Step;

/// A page's key: Ctrl (a Mac's Cmd) and its place in the rail.
pub(super) fn page_shortcut(ctx: &egui::Context, page: Page) -> String {
    let place = super::PAGES.iter().position(|&p| p == page).unwrap_or(0) + 1;
    keys(ctx, Modifiers::COMMAND, &place.to_string())
}

/// `key` with `modifiers` held, as this computer's keyboard names them: Ctrl+1 on Windows and
/// Linux, Cmd+1 on a Mac (or ⌘1 where the fonts the keys are shown in have the Mac's symbols), and
/// the names in the player's language (Strg+1). `key` is as it is shown (a word, translated).
pub(crate) fn keys(ctx: &egui::Context, modifiers: Modifiers, key: &str) -> String {
    let mac = ctx.os().is_mac();
    let names = if mac && ctx.fonts_mut(|fonts| [mono(11.5), body(13.0)].iter().all(|font| fonts.has_glyphs(font, "⌘⇧⌥⌃"))) {
        egui::ModifierNames::SYMBOLS
    } else {
        egui::ModifierNames { is_short: false, alt: t("Alt"), ctrl: t("Ctrl"), shift: t("Shift"), mac_cmd: t("Cmd"), mac_alt: t("Option"), concat: "+" }
    };
    let mut text = names.format(&modifiers, mac);
    if !text.is_empty() {
        text.push_str(names.concat);
    }
    text.push_str(key);
    text
}

/// Two keys' names, one or the other ("Ctrl+Z / Ctrl+Y").
fn either(one: String, other: String) -> String {
    format!("{one} / {other}")
}

/// Something going on, for the footer: where it is (`page`, `mods_tab`; a Supreme Commander
/// import's dialog at its tab, `dialog`), what it is, how far it has got when that is known, its
/// colour, whether it is the update, and whether it is work under way (a construction rail)
/// rather than a state (an update waiting, the game running).
struct Activity { mods_tab: Option<ModsTab>, page: Page, dialog: Option<crate::supcom::Tab>, text: String, progress: Option<f32>, color: Color32, update: bool, work: bool }
fn fraction(got: u64, total: u64) -> Option<f32> {
    (total > 0).then(|| (got as f64 / total as f64).clamp(0.0, 1.0) as f32)
}

impl LauncherApp {
    pub(crate) fn search_shortcuts_allowed(&self, ui: &Ui) -> bool {
        ui.is_enabled() && !self.modal_open(ui.ctx()) && self.film.is_none()
            && ui.ctx().memory(|m| m.is_above_modal_layer(ui.layer_id()))
    }

    pub(super) fn shortcuts_button(&mut self, ui: &mut Ui) {
        let ready = !self.modal_open(ui.ctx()) && !self.settings_view.capturing();
        if ui.add_enabled_ui(ready, |ui| keycap(ui, "F1")).inner.hover_tip(t("KEYBOARD SHORTCUTS")).clicked() { self.show_shortcuts = true; }
    }

    pub(super) fn shortcuts_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts { return; }
        let response = ta_ui::panel::modal(ctx, "launcher_shortcuts",
            Panel::new("KEYBOARD SHORTCUTS").exit(Exit::Close)
                .width((ctx.content_rect().width() - 64.0).clamp(1.0, 460.0))
                .max_height((ctx.content_rect().height() - 24.0).max(1.0)), true, |ui| {
            let ctx = ui.ctx().clone();
            for page in super::PAGES {
                shortcut_row(ui, &page_shortcut(&ctx, page), page_label(page));
            }
            ui.separator();
            shortcut_row(ui, t("Enter"), n("Start the game from the Play page"));
            shortcut_row(ui, "F5", n("Refresh game files, mods, maps or skins on the current page"));
            shortcut_row(ui, &keys(&ctx, Modifiers::COMMAND, "F"), n("Search mods, maps, skins or settings"));
            shortcut_row(ui, t("Esc"), n("Clear the focused search or close a dialog"));
            shortcut_row(ui, &either(t("Tab").to_string(), keys(&ctx, Modifiers::SHIFT, t("Tab"))), n("Move keyboard focus"));
            shortcut_row(ui, "F1", "KEYBOARD SHORTCUTS");
            ui.separator();
            ui.label(widgets::caption("CONTROLLER"));
            for (buttons, what) in controller_rows() {
                shortcut_row(ui, &buttons, what);
            }
            let here = self.page_keys(&ctx);
            if !here.is_empty() {
                ui.separator();
                ui.label(widgets::caption("ON THIS PAGE"));
                for (keys, what) in here {
                    shortcut_row(ui, &keys, what);
                }
            }
        });
        if response.exit { self.show_shortcuts = false; }
    }

    /// The keys of the page shown (its library, the load order, the skin editor), named as this
    /// computer's keyboard has them.
    fn page_keys(&self, ctx: &egui::Context) -> Vec<(String, &'static str)> {
        let menu = keys(ctx, Modifiers::SHIFT, "F10");
        let library = |favourite: bool| {
            let mut keys = vec![
                ("← ↑ → ↓".to_string(), n("Move from one to the next")),
                (t("Enter").to_string(), n("Open the chosen one's details")),
                (menu.clone(), n("Open its menu, as a right-click does (the Menu key too)")),
                (t("Esc").to_string(), n("Back to the library")),
            ];
            if favourite {
                keys.push(("F".to_string(), n("Mark the chosen map a favourite, or not")));
            }
            keys
        };
        match self.page {
            Page::Mods if self.mods_tab == super::ModsTab::LoadOrder => vec![
                (either(keys(ctx, Modifiers::ALT, "↑"), keys(ctx, Modifiers::ALT, "↓")), n("Move the chosen mod up or down the load order")),
                (either(keys(ctx, Modifiers::ALT, t("Home")), keys(ctx, Modifiers::ALT, t("End"))), n("Move it to the top or the bottom")),
                (menu.clone(), n("Open its menu, as a right-click does (the Menu key too)")),
            ],
            Page::Skins if self.skins.editor.is_some() => vec![
                (either(keys(ctx, Modifiers::COMMAND, "Z"), keys(ctx, Modifiers::COMMAND, "Y")), n("Undo, redo")),
                (keys(ctx, Modifiers::COMMAND, "S"), n("Save the skin")),
                (t("Esc").to_string(), n("Leave the editor")),
            ],
            Page::Mods | Page::Skins => library(false),
            Page::Maps => library(true),
            _ => Vec::new(),
        }
    }

    /// `--show progress`: the construction rails in every state the launcher draws them in,
    /// at their sizes (a capture's check, not a page of the launcher).
    pub(super) fn progress_gallery(&mut self, ctx: &egui::Context) {
        if !self.show_progress { return; }
        let response = ta_ui::panel::modal(ctx, "launcher_progress_gallery",
            Panel::new("PROGRESS").exit(Exit::Close).width((ctx.content_rect().width() - 64.0).clamp(1.0, 720.0))
                .max_height((ctx.content_rect().height() - 24.0).max(1.0)), true, |ui| {
            let rows: [(&str, Option<f32>, bool, f32); 11] = [
                ("downloading 0%", Some(0.0), true, 22.0),
                ("downloading 4%", Some(0.04), true, 22.0),
                ("downloading 38%", Some(0.38), true, 22.0),
                ("downloading 77%", Some(0.77), true, 22.0),
                ("complete", Some(1.0), true, 22.0),
                ("unpacking (unknown length)", None, true, 22.0),
                ("waiting (inactive)", Some(0.45), false, 22.0),
                ("launch overlay (10 px)", None, true, 10.0),
                ("activity rail (8 px)", Some(0.6), true, 8.0),
                ("profile meter (5 px, quiet)", Some(0.52), false, 5.0),
                ("thin rail (3 px)", Some(0.3), true, 3.0),
            ];
            for (label, value, active, height) in rows {
                ui.label(RichText::new(label).font(strong(11.0)).color(STEEL));
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
                ta_ui::progress::paint(ui, rect, value, active);
                ui.add_space(6.0);
            }
        });
        if response.exit { self.show_progress = false; }
    }

    /// Whether anything is going on for the footer's chips.
    pub(super) fn has_activities(&self) -> bool { !self.activities().is_empty() }

    fn activities(&self) -> Vec<Activity> {
        let mut items = Vec::new();
        if let Some(download) = &self.download {
            let progress = match download.step { Step::Downloading { got, total, .. } => fraction(got, total), _ => None };
            // (A setup that did not start, Windows' question answered no, waits for INSTALL NOW:
            // nothing is under way. Nor is anything while one checked is put off till later.)
            let waiting = matches!(download.step, Step::Ready(_)) && !self.update_go;
            let failed = matches!(download.step, Step::Failed(_)) || (waiting && self.update_error.is_some());
            let color = if failed { RED } else { SKY };
            let text = if failed { t("UPDATE FAILED").to_string() }
                else if waiting { tf("UPDATE v{version}", &[("version", &download.version)]) }
                else { tf("UPDATING TO v{version}", &[("version", &download.version)]) };
            let work = !failed && !waiting;
            items.push(Activity { mods_tab: None, page: Page::Play, dialog: None, text, progress, color, update: true, work });
        }
        if self.download.is_none() && let crate::update::UpdateState::Available(release) = &self.update {
            items.push(Activity { mods_tab: None, page: Page::Play, dialog: None, text: tf("UPDATE v{version}", &[("version", &release.number())]),
                progress: None, color: SKY, update: true, work: false });
        }
        let active: Vec<_> = self.installer.stages.iter().filter(|(_, stage)| stage.active()).collect();
        if !active.is_empty() {
            let (_, stage) = active.iter().copied().find(|(_, stage)| !matches!(stage, Stage::Queued)).unwrap_or(active[0]);
            let progress = match stage { Stage::Downloading { got, total, .. } | Stage::Copying { got, total } => fraction(*got, *total), _ => None };
            items.push(Activity { mods_tab: Some(ModsTab::Library), page: Page::Mods, dialog: None, text: tf("MODS · {count} active", &[("count", &active.len())]), progress, color: AMBER, update: false, work: true });
        }
        if let Some(status) = self.maps.status() {
            items.push(Activity { mods_tab: None, page: Page::Maps, dialog: None, text: t(status).to_string(), progress: None, color: AMBER, update: false, work: true });
        }
        // A Supreme Commander import, whether its dialog is shown or not: a click opens it at its tab.
        for (tab, importing, progress, text) in [
            (crate::supcom::Tab::Maps, self.supcom.importing(), &self.supcom.progress, n("SUPREME COMMANDER MAPS")),
            (crate::supcom::Tab::Factions, self.supcom.factions.importing(), &self.supcom.factions.progress, n("SUPREME COMMANDER FACTIONS")),
            (crate::supcom::Tab::Music, self.supcom.music.importing(), &self.supcom.music.progress, n("SUPREME COMMANDER MUSIC")),
        ] {
            if importing {
                let page = if tab == crate::supcom::Tab::Maps { Page::Maps } else { Page::Mods };
                items.push(Activity { mods_tab: None, page, dialog: Some(tab), text: t(text).to_string(), progress: progress.lock().unwrap().fraction(), color: AMBER, update: false, work: true });
            }
        }
        if self.mod_validation.busy() && !self.game_running() && !self.installer.busy() && !self.maps.busy() && !self.supcom.any_importing() {
            items.push(Activity { mods_tab: Some(ModsTab::Library), page: Page::Mods, dialog: None,
                text: t("VALIDATING MODS…").to_string(), progress: self.mod_validation.fraction(), color: SKY, update: false, work: true });
        }
        if let Some(status) = self.skins.status() {
            items.push(Activity { mods_tab: None, page: Page::Skins, dialog: None, text: t(status).to_string(), progress: None, color: AMBER, update: false, work: true });
        }
        if self.game_running() {
            let benchmark = self.benchmarks.running.is_some();
            items.push(Activity { mods_tab: None, page: if benchmark { Page::Benchmark } else { Page::Play }, dialog: None,
                text: t(if benchmark { "BENCHMARK RUNNING" } else { "GAME RUNNING" }).to_string(), progress: None, color: AMBER, update: false, work: false });
        }
        items
    }

    /// The activities as chips in the footer (`ui`: the room between the save status and the
    /// copyright): a lamp, what is going on, and for work its construction rail; a click opens
    /// its page. Those that do not fit are counted on a last chip.
    pub(super) fn activity_chips(&mut self, ui: &mut Ui) {
        let items = self.activities();
        if items.is_empty() { return; }
        let room = ui.available_width();
        let widths: Vec<f32> = items.iter().map(|item| chip_width(ui, item)).collect();
        let more_width = 44.0;
        let mut shown = 0;
        let mut used = 0.0;
        for (i, w) in widths.iter().enumerate() {
            let rest = if i + 1 < items.len() { more_width + 8.0 } else { 0.0 };
            if used + w + rest > room && shown > 0 { break; }
            used += w + 8.0;
            shown += 1;
        }
        ui.spacing_mut().item_spacing.x = 8.0;
        let hidden: Vec<String> = items[shown..].iter().map(|item| item.text.clone()).collect();
        for item in items.into_iter().take(shown) {
            if activity_chip(ui, &item).clicked() {
                if item.update { self.push(Action::ShowUpdate); }
                // (An import's dialog, over the page it was opened from.)
                else if let Some(tab) = item.dialog {
                    self.open_page(item.page);
                    self.push(Action::SupcomOpen(tab));
                }
                else {
                    // As the rail opens it (a page's scan or refresh with it); a page that scrolls
                    // as a whole is shown from its top (the libraries scroll within, and would
                    // leave the request to the next such page).
                    self.open_page(item.page);
                    self.reset_activity_scroll = !matches!(item.page, Page::Mods | Page::Maps | Page::Skins | Page::Profile);
                    if let Some(tab) = item.mods_tab { self.mods_tab = tab; }
                }
            }
        }
        if !hidden.is_empty() {
            let (rect, response) = ui.allocate_exact_size(vec2(more_width, 24.0), egui::Sense::hover());
            ui.painter().add(ta_ui::metal::well(rect, INSET));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, format!("+{}", hidden.len()), strong(11.0), TEXT);
            response.hover_tip(hidden.join("\n"));
        }
    }
}

/// What a chip needs across: its lamp, its words, its rail and its figure.
fn chip_width(ui: &Ui, item: &Activity) -> f32 {
    let text = widgets::galley(ui, RichText::new(&item.text).font(strong(10.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.8)));
    let rail = if item.work { 12.0 + CHIP_RAIL } else { 0.0 };
    let figure = if item.progress.is_some() { 40.0 } else { 0.0 };
    (22.0 + text.size().x + rail + figure + 12.0).ceil()
}

/// The width of a chip's construction rail.
const CHIP_RAIL: f32 = 92.0;

/// An activity's chip: a dark plate, its lamp, its words, its construction rail while it works
/// and its figure when it counts.
fn activity_chip(ui: &mut Ui, item: &Activity) -> egui::Response {
    let width = chip_width(ui, item).min(ui.available_width().max(60.0));
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), egui::Sense::click());
    let percent = item.progress.map(|value| format!("{:.0}%", value * 100.0));
    let words = match &percent { Some(p) => format!("{} · {p}", item.text), None => item.text.clone() };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &words));
    if ui.is_rect_visible(rect) {
        let hot = response.hovered() || response.has_focus();
        let painter = ui.painter();
        painter.add(ta_ui::metal::well(rect, if hot { PANEL_HI } else { INSET }));
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, item.color.gamma_multiply(if hot { 0.55 } else { 0.3 })), egui::StrokeKind::Inside);
        painter.add(ta_ui::metal::lamp(egui::pos2(rect.left() + 11.0, rect.center().y), 3.2, item.color, 1.0));
        let room = (rect.width() - 34.0 - if item.work { 12.0 + CHIP_RAIL } else { 0.0 } - if percent.is_some() { 40.0 } else { 0.0 }).max(1.0);
        let text = egui::WidgetText::from(RichText::new(&item.text).font(strong(10.5)).color(if hot { TEXT_BRIGHT } else { item.color }).extra_letter_spacing(ta_ui::i18n::tracking(0.8)))
            .into_galley(ui, Some(egui::TextWrapMode::Truncate), room, egui::TextStyle::Body);
        let text_end = rect.left() + 22.0 + text.size().x;
        ui.painter().galley(egui::pos2(rect.left() + 22.0, rect.center().y - text.size().y / 2.0), text, item.color);
        let mut x = text_end + 12.0;
        if item.work {
            let rail = Rect::from_min_size(egui::pos2(x, rect.center().y - 5.0), vec2(CHIP_RAIL, 10.0));
            ta_ui::progress::paint(ui, rail, item.progress, true);
            x = rail.right() + 8.0;
        }
        if let Some(p) = &percent {
            ui.painter().text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, p, strong(10.5), TEXT_BRIGHT);
        }
        widgets::focus_ring(ui, &response, rect);
    }
    response.hover_tip_ui(|ui| { ui.label(&words); ui.label(RichText::new(t("Open activity")).color(STEEL)); })
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A key of a keyboard as a button (the footer's F1): a dark keycap on a lighter rim, its face
/// lit along the top, the legend in cream; it warms to brass under the pointer and sinks when
/// pressed. `key` is not translated.
fn keycap(ui: &mut Ui, key: &str) -> egui::Response {
    let legend = widgets::galley(ui, RichText::new(key).font(strong(10.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.6)));
    let (rect, response) = ui.allocate_exact_size(vec2((legend.size().x + 16.0).max(30.0), 20.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), t("KEYBOARD SHORTCUTS")));
    if response.clicked() { ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Button); }
    if ui.is_rect_visible(rect) {
        let hot = ui.ctx().animate_bool_with_time(response.id.with("hot"), ui.is_enabled() && (response.hovered() || response.has_focus()), 0.12);
        let down = response.is_pointer_button_down_on();
        let painter = ui.painter();
        // The rim the key sits in, then its face (a little shorter: the key's depth below it).
        painter.rect_filled(rect, 3.0, ta_ui::metal::STEEL_EDGE);
        let face = Rect::from_min_max(rect.min + vec2(1.0, if down { 2.0 } else { 1.0 }), rect.max - vec2(1.0, if down { 1.0 } else { 3.0 }));
        painter.rect_filled(Rect::from_min_max(pos2(face.min.x, face.max.y - 1.0), rect.max - vec2(1.0, 1.0)), 2.0, Color32::from_rgb(24, 23, 20));
        painter.add(ta_ui::metal::vgrad(face, Color32::from_rgb(64, 62, 55).lerp_to_gamma(AMBER_DIM, 0.35 * hot), Color32::from_rgb(40, 39, 34)));
        painter.hline(face.x_range().shrink(1.0), face.min.y + 0.5, egui::Stroke::new(1.0, Color32::from_white_alpha((34.0 + 40.0 * hot) as u8)));
        painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, LINE_HI.lerp_to_gamma(AMBER, hot)), egui::StrokeKind::Inside);
        let at = face.center() - legend.size() / 2.0;
        painter.galley(at + vec2(0.0, 1.0), legend.clone(), Color32::from_black_alpha(170));
        painter.galley(at, legend, CREAM.lerp_to_gamma(AMBER_BRIGHT, hot));
        widgets::focus_ring(ui, &response, rect);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A gamepad's buttons in the launcher, as the shortcuts list shows them (`crate::pad`).
fn controller_rows() -> [(String, &'static str); 9] {
    [
        (format!("{} / {}", t("D-pad"), t("Left stick")), n("Move between controls")),
        ("A".into(), n("Use the control with the focus")),
        ("B / Start".into(), n("Go back, or close a dialog")),
        ("LB / RB".into(), n("The previous or next page")),
        (t("Right stick").to_string(), n("Move the pointer (A then clicks)")),
        ("LT / RT".into(), n("Scroll")),
        ("X".into(), n("Open the chosen one's menu")),
        ("Y".into(), n("Search mods, maps, skins or settings")),
        ("View".into(), n("Open this list")),
    ]
}

/// A key (as shown: [`keys`]) and what it does (`label`, translated here).
fn shortcut_row(ui: &mut Ui, key: &str, label: &str) {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(vec2(112.0, 18.0), egui::Layout::top_down(Align::Min), |ui| {
            ui.set_width(112.0);
            ui.add(egui::Label::new(RichText::new(key).font(mono(11.5)).color(AMBER)).wrap());
        });
        ui.add(egui::Label::new(RichText::new(t(label)).color(TEXT)).wrap());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn activity_routes_to_the_transfer_being_shown_and_keeps_update_discovery() {
        let root = std::env::temp_dir().join(format!("ta_activity_route_{}", std::process::id()));
        let config = root.join("config.toml");
        let ctx = egui::Context::default();
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx),
            crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]));
        app.skins.shutdown();
        app.installer.stages.insert("catalogue".into(), Stage::Downloading { got: 40, total: 100, since: std::time::Instant::now() });
        app.installer.stages.insert("custom".into(), Stage::Queued);
        app.installer.packages.insert("custom".into(), "Custom mod".into());
        let item = app.activities().into_iter().find(|item| item.page == Page::Mods).unwrap();
        assert_eq!(item.progress, Some(0.4));
        assert_eq!(item.mods_tab, Some(ModsTab::Library));
        app.installer.stages.insert("catalogue".into(), Stage::Done { units: 1, maps: 0, id: "catalogue".into() });
        let item = app.activities().into_iter().find(|item| item.page == Page::Mods).unwrap();
        assert_eq!(item.mods_tab, Some(ModsTab::Library));
        app.show("update");
        app.update_hidden = true;
        assert!(app.activities().iter().any(|item| item.update && item.text.contains("2.2.0")));
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A Supreme Commander import shows in the footer, its dialog open or not: how far it has got,
    /// and a click opens the dialog at its tab, over its page.
    #[test]
    fn a_supreme_commander_import_has_its_chip_in_the_footer() {
        let root = std::env::temp_dir().join(format!("ta_activity_supcom_{}", std::process::id()));
        let config = root.join("config.toml");
        let ctx = egui::Context::default();
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()),
            crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]));
        app.skins.shutdown();
        app.supcom.look.canned = Some(std::sync::Arc::new(|| (crate::supcom::Found::default(), Default::default())));
        app.supcom.pretend_importing(&[]);
        {
            let mut p = app.supcom.progress.lock().unwrap();
            (p.phase, p.done, p.total) = (crate::supcom::Phase::Making, 3, 12);
        }
        let item = app.activities().into_iter().find(|item| item.dialog.is_some()).expect("its chip");
        assert_eq!((item.dialog, item.page, item.text.as_str(), item.progress), (Some(crate::supcom::Tab::Maps), Page::Maps, "SUPREME COMMANDER MAPS", Some(0.25)));
        assert!(item.work && !app.supcom.open);
        app.apply(Action::SupcomOpen(crate::supcom::Tab::Factions), &eframe::Frame::_new_kittest());
        assert!(app.supcom.open && app.supcom.tab == crate::supcom::Tab::Maps, "at the tab whose import is under way");
        app.supcom.stop();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.supcom.importing() && std::time::Instant::now() < until {
            app.supcom.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.activities().iter().all(|item| item.dialog.is_none()));
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The keys are named as the computer's keyboard has them: Ctrl on Windows and Linux; on a Mac
    /// Cmd (or ⌘, where the fonts have it) and Option, never Ctrl. The shortcuts' list names them so.
    #[test]
    fn keys_are_named_as_the_computers_keyboard_has_them() {
        use egui::os::OperatingSystem;
        let root = std::env::temp_dir().join(format!("ta_shortcut_names_{}", std::process::id()));
        let config = root.join("config.toml");
        let ctx = egui::Context::default();
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()),
            crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]));
        app.skins.shutdown();
        app.show_shortcuts = true;
        // (Every text a frame of the shortcuts' list painted.)
        let painted = |app: &mut LauncherApp| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.shortcuts_dialog(ui.ctx()));
            output.textures_delta.clear();
            fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                    _ => {}
                }
            }
            let mut texts = Vec::new();
            output.shapes.iter().for_each(|clipped| collect(&clipped.shape, &mut texts));
            texts
        };
        ctx.set_os(OperatingSystem::Windows);
        painted(&mut app);
        assert_eq!(page_shortcut(&ctx, Page::Mods), "Ctrl+2");
        assert_eq!(keys(&ctx, Modifiers::SHIFT, "F10"), "Shift+F10");
        let texts = painted(&mut app);
        for row in ["Ctrl+1", "Ctrl+8", "Ctrl+F", "Tab / Shift+Tab", "Enter", "Esc"] {
            assert!(texts.iter().any(|text| text == row), "{row}: {texts:?}");
        }
        ctx.set_os(OperatingSystem::Mac);
        let mods = page_shortcut(&ctx, Page::Mods);
        assert!(mods == "Cmd+2" || mods == "⌘2", "{mods}");
        let option = keys(&ctx, Modifiers::ALT, "↑");
        assert!(option == "Option+↑" || option == "⌥↑", "{option}");
        let texts = painted(&mut app);
        assert!(!texts.iter().any(|text| text.contains("Ctrl")), "no Ctrl on a Mac: {texts:?}");
        assert!(texts.iter().any(|text| text == &keys(&ctx, Modifiers::COMMAND, "F")), "{texts:?}");
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// An update whose setup did not start (Windows' question answered no) is not shown as one
    /// under way: the footer says it failed until INSTALL NOW is pressed again.
    #[test]
    fn an_update_whose_setup_did_not_start_is_not_shown_as_updating() {
        let root = std::env::temp_dir().join(format!("ta_activity_update_{}", std::process::id()));
        let config = root.join("config.toml");
        let ctx = egui::Context::default();
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx),
            crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]));
        app.skins.shutdown();
        let update = |app: &LauncherApp| app.activities().into_iter().find(|item| item.update).unwrap();
        app.download = Some(crate::update::Download::shown("9.9.9", Step::Ready(root.join("setup.exe"))));
        app.update_go = true;
        let starting = update(&app);
        assert!(starting.work && starting.text.contains("9.9.9"), "{}", starting.text);
        app.update_go = false;
        app.update_error = Some("Windows' permission to install was not given".into());
        let refused = update(&app);
        assert!(!refused.work && refused.color == RED && !refused.text.contains("9.9.9"), "{}", refused.text);
        app.download = None;
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
}
