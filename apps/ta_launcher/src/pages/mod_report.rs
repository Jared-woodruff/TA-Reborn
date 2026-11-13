//! Local validation and curated observations have different evidence and never share a
//! certification badge. The complete, copyable report keeps both sources explicit.
//!
//! The automated report: what counts at the top (errors, warnings, notes, each a filter with
//! its count), what is inherited from the original game, Reborn or a dependency hidden until
//! asked for, a search, and the findings grouped by what they concern, each with how to fix it
//! a click away.

use std::collections::BTreeSet;

use ta_ui::tooltip::TooltipExt as _;
use egui::{Color32, CornerRadius, Margin, RichText, Sense, Stroke, Ui, pos2, vec2};
use ta_data::validation::{Issue, Origin, Severity, Status};
use ta_ui::i18n::{grouped, n, t, tf};
use ta_ui::metal::{self, Slab};
use ta_ui::panel::{Exit, Panel};

use crate::app::{Action, LauncherApp};
use crate::catalogue::{self, Entry, Fit};
use crate::theme::*;
use crate::validation::{SavedReport, ValidationStatus};
use crate::widgets::{self, ActionButton};

/// Findings a group shows before SHOW ALL.
const GROUP_PREVIEW: usize = 25;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReportTab { Automated, Review }

/// What the findings concern, in the order the report lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Group { Check, Packaging, Files, Units, Weapons, Features, Maps, Scripts, Models, Animations, Sounds, Menus, Other }

impl Group {
    fn of(issue: &Issue) -> Group {
        let code = issue.code.as_str();
        if code.starts_with("validation.") || matches!(code, "DEPENDENCY" | "MANIFEST_CHANGED" | "BASE_MISSING" | "SOURCE_UNREADABLE" | "BASE_MOUNT" | "BUILTIN_MOUNT" | "MOD_MOUNT") {
            return Group::Check;
        }
        if code.starts_with("packaging.") { return Group::Packaging; }
        if code == "ARCHIVE_UNREADABLE" { return Group::Files; }
        if code.starts_with("render.sprite") { return Group::Animations; }
        if code.starts_with("render.texture") { return Group::Models; }
        match issue.path.split('/').next().unwrap_or("") {
            "units" | "unitpics" => Group::Units,
            "weapons" => Group::Weapons,
            "features" => Group::Features,
            "maps" => Group::Maps,
            "scripts" => Group::Scripts,
            "objects3d" | "textures" => Group::Models,
            "anims" => Group::Animations,
            "sounds" => Group::Sounds,
            "guis" | "download" => Group::Menus,
            "gamedata" => match issue.path.as_str() {
                "gamedata/sound.tdf" => Group::Sounds,
                "gamedata/sidedata.tdf" => Group::Menus,
                _ => Group::Units,
            },
            _ if matches!(code, "file.read" | "format.invalid") => Group::Files,
            _ => Group::Other,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Group::Check => n("THE CHECK"),
            Group::Packaging => n("PACKAGING"),
            Group::Files => n("FILES"),
            Group::Units => n("UNITS"),
            Group::Weapons => n("WEAPONS"),
            Group::Features => n("FEATURES"),
            Group::Maps => n("MAPS"),
            Group::Scripts => n("SCRIPTS"),
            Group::Models => n("MODELS & TEXTURES"),
            Group::Animations => n("ANIMATIONS & EFFECTS"),
            Group::Sounds => n("SOUNDS"),
            Group::Menus => n("MENUS & SIDES"),
            Group::Other => n("OTHER"),
        }
    }
}

/// Which findings the report shows: errors, warnings and notes that count, and the inherited.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Shown { errors: bool, warnings: bool, notes: bool, inherited: bool }

impl Shown {
    fn shows(&self, issue: &Issue) -> bool {
        if !issue.affects_status { return self.inherited; }
        match issue.severity { Severity::Error => self.errors, Severity::Warning => self.warnings, Severity::Info => self.notes }
    }
}

#[derive(Clone)]
struct ReportView {
    tab: ReportTab,
    copied_until: f64,
    shown: Shown,
    search: String,
    /// Groups folded away, and groups shown whole (past GROUP_PREVIEW).
    folded: BTreeSet<Group>,
    whole: BTreeSet<Group>,
    /// Findings (by index) whose fix is open, in the report checked at `checked` (a new report,
    /// RUN AGAIN's, closes them: its findings are others).
    fixes: BTreeSet<usize>,
    checked: u64,
}

impl ReportView {
    fn new(tab: ReportTab) -> Self {
        Self { tab, copied_until: 0.0, shown: Shown { errors: true, warnings: true, notes: true, inherited: false }, search: String::new(), folded: BTreeSet::new(), whole: BTreeSet::new(), fixes: BTreeSet::new(), checked: 0 }
    }
}

pub(super) fn validation_badge(status: &ValidationStatus) -> (&'static str, Color32) {
    match status {
        ValidationStatus::Unknown => (n("NOT CHECKED"), STEEL),
        ValidationStatus::Queued => (n("CHECK QUEUED"), SKY),
        ValidationStatus::Checking => (n("CHECKING MOD…"), SKY),
        ValidationStatus::Stale => (n("RECHECK NEEDED"), AMBER),
        // (The check could not run: not the mod's errors, which are ERRORS FOUND.)
        ValidationStatus::Failed(_) => (n("CHECK FAILED"), RED),
        ValidationStatus::Ready(Status::Errors) => (n("ERRORS FOUND"), RED),
        ValidationStatus::Ready(Status::Clean) => (n("DATA CHECKS PASSED"), GREEN),
        ValidationStatus::Ready(Status::Issues) => (n("ISSUES FOUND"), AMBER),
        ValidationStatus::Ready(Status::Incomplete) => (n("CHECK INCOMPLETE"), AMBER),
        ValidationStatus::Ready(Status::Empty) => (n("NOTHING TO CHECK"), STEEL),
    }
}

/// Both visual and screen-reader users get a labelled button; no font-dependent ⓘ.
pub(super) fn info_button(ui: &mut Ui) -> egui::Response {
    let response = ui.add(ActionButton::secondary("MOD INFO").height(28.0).min_width(92.0));
    response.hover_tip(t("View validation results and known compatibility issues"))
}

impl LauncherApp {
    pub(crate) fn mod_report_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.mod_validation.selected.clone() else { return };
        let entry = self.mods.get(&id);
        let reviewed = catalogue::get(&id);
        let name = entry.map(|e| e.info.manifest.name.as_str())
            .or_else(|| reviewed.map(|e| e.name)).unwrap_or(&id);
        let installed = entry.is_some();
        let status = self.mod_validation.status(&id);
        let report = self.mod_validation.report(&id);
        let progress = self.mod_validation.progress(&id);
        // Why RUN AGAIN cannot be pressed now, if it cannot.
        let blocked = if self.game_running() { Some(t("Wait for the game to close.")) }
            else if self.installer.busy() { Some(t("Wait for the installs to finish.")) }
            else if self.maps.busy() { Some(t("Wait for the map library to finish.")) }
            else if matches!(status, ValidationStatus::Queued | ValidationStatus::Checking) { Some(t("A check is under way.")) }
            else { None };
        let action = report_dialog(ctx, &id, name, installed, &status, report, reviewed, blocked, progress.as_ref());
        // A report can contain thousands of diagnostics. Format it only when requested,
        // never during ordinary frames or cursor animation.
        if action.copy {
            ctx.copy_text(copied_report(name, &id, self.mod_validation.text(&id), &status, reviewed));
        }
        if action.close { self.mod_validation.selected = None; }
        if action.validate { self.push(Action::ValidateMod(id)); }
    }
}

fn copied_report(name: &str, id: &str, automatic: Option<String>, status: &ValidationStatus, reviewed: Option<&Entry>) -> String {
    let mut text = automatic.unwrap_or_else(|| {
        format!("TA Reborn mod compatibility report\nMod: {name} ({id})\nAutomated check: not available\n")
    });
    if let ValidationStatus::Failed(error) = status {
        text.push_str(&format!("\nValidation could not complete: {error}\n"));
    }
    if let Some(review) = reviewed { text.push('\n'); text.push_str(&review.review_text()); }
    text
}

#[derive(Default)]
struct ReportAction {
    close: bool, validate: bool, copy: bool,
    #[cfg(test)] rect: Option<egui::Rect>,
}

#[allow(clippy::too_many_arguments)]
fn report_dialog(ctx: &egui::Context, id: &str, name: &str, installed: bool, status: &ValidationStatus,
    report: Option<&SavedReport>, reviewed: Option<&Entry>, blocked: Option<&str>, progress: Option<&(String, Option<f32>)>) -> ReportAction
{
    let state_id = egui::Id::new(("mod_report_view", id));
    let mut view = ctx.data(|data| data.get_temp::<ReportView>(state_id)).unwrap_or_else(|| ReportView::new(if installed { ReportTab::Automated } else { ReportTab::Review }));
    if reviewed.is_none() { view.tab = ReportTab::Automated; }
    let mut action = ReportAction::default();
    let panel = Panel::new("MOD COMPATIBILITY").width((ctx.content_rect().width() - 76.0).clamp(180.0, 800.0))
        .max_height((ctx.content_rect().height() - 30.0).clamp(120.0, 760.0)).exit(Exit::Close);
    let response = ta_ui::panel::modal(ctx, ("mod_compatibility", id), panel, true, |ui| {
        ui.set_min_height(ui.available_height());
        ui.add(egui::Label::new(RichText::new(name).font(strong(18.0)).color(TEXT_BRIGHT)).truncate()).hover_tip(name);
        ui.horizontal_wrapped(|ui| {
            if installed {
                let (label, color) = validation_badge(status);
                widgets::tag(ui, label, color);
            }
            if reviewed.is_some_and(|e| e.fit == Fit::Partly) { widgets::tag(ui, "KNOWN GAPS", AMBER); }
            if !installed { widgets::tag(ui, "NOT INSTALLED", STEEL); }
        });
        ui.horizontal_wrapped(|ui| {
            if copy_report_button(ui, ctx.input(|i| i.time) < view.copied_until).clicked() {
                action.copy = true;
                view.copied_until = ctx.input(|i| i.time) + 3.0;
                ctx.request_repaint_after(std::time::Duration::from_secs(3));
            }
            if installed {
                let label = if report.is_some() { n("RUN AGAIN") } else { n("VALIDATE MOD") };
                let button = ui.add_enabled(blocked.is_none(), ActionButton::primary(label).height(30.0));
                let button = match blocked { Some(why) => button.on_disabled_hover_text(why), None => button };
                if button.clicked() { action.validate = true; }
            }
        });
        if reviewed.is_some() {
            widgets::segmented(ui, &mut view.tab, &[(ReportTab::Automated, "AUTOMATED CHECK"), (ReportTab::Review, "PROJECT REVIEW")]);
        } else {
            view.tab = ReportTab::Automated;
            ui.label(widgets::caption("AUTOMATED CHECK"));
        }
        ta_ui::panel::rule(ui);
        // The filters and the search stay above the findings as they scroll.
        if view.tab == ReportTab::Automated && let Some(saved) = report && !saved.report.issues.is_empty() {
            filters(ui, id, &saved.report.issues, &mut view);
        }
        let room = ui.available_height().max(32.0);
        let area = egui::ScrollArea::vertical().id_salt(("mod_report_body", id, view.tab == ReportTab::Automated))
            .max_height(room).min_scrolled_height(0.0).auto_shrink([false, false]);
        ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Plate, |ui| {
            ui.set_min_width(ui.available_width());
            match view.tab {
                ReportTab::Automated => automated_report(ui, installed, status, report, progress, &mut view),
                ReportTab::Review => if let Some(entry) = reviewed { project_review(ui, entry); },
            }
        });
    });
    action.close = response.exit;
    #[cfg(test)] { action.rect = Some(response.rect); }
    ctx.data_mut(|data| data.insert_temp(state_id, view));
    action
}

fn copy_report_button(ui: &mut Ui, copied: bool) -> egui::Response {
    // Feedback must not move RUN AGAIN onto a different row in compact translations.
    let width = [t("COPY FULL REPORT"), t("COPIED")].into_iter().map(|label| {
        widgets::galley(ui, RichText::new(label).font(strong(13.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.8))).size().x + 30.0
    }).fold(72.0, f32::max);
    ui.add(ActionButton::secondary(if copied { "COPIED" } else { "COPY FULL REPORT" }).height(30.0).min_width(width))
}

fn wrapped(ui: &mut Ui, text: impl Into<String>, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text.into()).color(color)).wrap());
}

/// The filters (each a lamp, its name and how many) and the search.
fn filters(ui: &mut Ui, id: &str, issues: &[Issue], view: &mut ReportView) {
    let count = |severity: Severity| issues.iter().filter(|i| i.affects_status && i.severity == severity).count();
    let inherited = issues.iter().filter(|i| !i.affects_status).count();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        let shown = &mut view.shown;
        filter_chip(ui, n("ERRORS"), count(Severity::Error), RED, &mut shown.errors).hover_tip(t("Errors in the mod, or caused by it: content the game cannot use"));
        filter_chip(ui, n("WARNINGS"), count(Severity::Warning), AMBER, &mut shown.warnings).hover_tip(t("Things the game works round, or that show: worth putting right"));
        filter_chip(ui, n("NOTES"), count(Severity::Info), SKY, &mut shown.notes).hover_tip(t("Worth knowing; they do not count against the mod"));
        if inherited > 0 {
            filter_chip(ui, n("INHERITED"), inherited, STEEL, &mut shown.inherited).hover_tip(t("Findings in the original game, Reborn or a dependency that are there without this mod: not its doing"));
        }
        let width = ui.available_width().clamp(140.0, 260.0);
        crate::library::search(ui, &mut view.search, egui::Id::new(("mod_report_search", id)), t("Search findings…"), width, false);
    });
    ui.add_space(4.0);
}

/// A filter: a slot with a lamp lit while its findings show, its name and how many.
fn filter_chip(ui: &mut Ui, label: &str, count: usize, color: Color32, on: &mut bool) -> egui::Response {
    let words = format!("{}  {}", t(label), grouped(count as u64));
    let text = widgets::galley(ui, RichText::new(&words).font(strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
    let (rect, response) = ui.allocate_exact_size(vec2(text.size().x + 34.0, 26.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *on, &words));
    if response.clicked() {
        *on = !*on;
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
    }
    let hot = if response.hovered() { 1.0 } else { 0.0 };
    let painter = ui.painter();
    painter.add(metal::slab(ui.ctx(), rect, Slab::Slot, hot, false));
    painter.add(metal::lamp(pos2(rect.min.x + 13.0, rect.center().y), 3.5, if *on { color } else { STEEL_DIM }, if *on { 1.0 } else { 0.15 }));
    painter.galley(pos2(rect.min.x + 25.0, rect.center().y - text.size().y / 2.0), text, if *on { TEXT_BRIGHT } else { STEEL });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn automated_report(ui: &mut Ui, installed: bool, status: &ValidationStatus, saved: Option<&SavedReport>, progress: Option<&(String, Option<f32>)>, view: &mut ReportView) {
    match status {
        ValidationStatus::Checking => {
            wrapped(ui, t("Checking mod files in the background. You can close this report and keep using the launcher."), SKY);
            match progress {
                Some((text, Some(fraction))) => {
                    wrapped(ui, text, STEEL);
                    let width = ui.available_width();
                    ta_ui::panel::progress_bar(ui, *fraction, width);
                }
                Some((text, None)) => { wrapped(ui, text, STEEL); ta_ui::panel::busy_bar(ui); }
                None => { ta_ui::panel::busy_bar(ui); }
            }
            ui.add_space(8.0);
        }
        ValidationStatus::Queued => wrapped(ui, t("Waiting for background validation."), SKY),
        ValidationStatus::Stale => wrapped(ui, t("This saved report is out of date. Run validation again for the current files and Reborn version."), AMBER),
        ValidationStatus::Failed(error) => wrapped(ui, tf("Validation could not complete: {error}", &[("error", error)]), RED),
        ValidationStatus::Unknown => wrapped(ui, if installed { t("No local check yet. Choose VALIDATE MOD to check this installation.") }
            else { t("Install this mod to validate its files. The project review is available before installation.") }, TEXT),
        ValidationStatus::Ready(_) => {}
    }
    let Some(saved) = saved else {
        ui.add_space(6.0);
        wrapped(ui, t("Checks installed data, references and asset limits. It does not certify gameplay, balance, multiplayer or visual quality."), STEEL);
        return;
    };
    if view.checked != saved.checked_unix {
        view.checked = saved.checked_unix;
        view.fixes.clear();
    }
    // Which version was checked, and when; the findings; then what the check covered.
    let version = if saved.version.is_empty() { t("Unknown").to_string() } else { saved.version.clone() };
    let when = std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(saved.checked_unix)).map_or(String::new(), crate::battles::age);
    wrapped(ui, tf("Version {version} · checked {when} · Reborn {reborn}", &[("version", &version), ("when", &when), ("reborn", &saved.reborn_version)]), STEEL);
    if !saved.report.complete { wrapped(ui, t("The check did not cover every file. See the findings below before relying on this result."), AMBER); }
    // (Clean: nothing that counts, the original game's findings hidden. Every report holds
    // some of those, so "no findings match" would greet every clean mod.)
    if saved.report.complete && !saved.report.issues.iter().any(|i| i.affects_status) && !view.shown.inherited {
        ui.add_space(12.0);
        wrapped(ui, t("No issues found by these data checks. Review known gameplay and rendering observations separately."), GREEN);
        coverage(ui, saved, view);
        return;
    }
    // The findings shown, by group, in the report's order within each.
    let search = view.search.trim().to_lowercase();
    let mut groups: std::collections::BTreeMap<Group, Vec<(usize, &Issue)>> = std::collections::BTreeMap::new();
    for (index, issue) in saved.report.issues.iter().enumerate() {
        if !view.shown.shows(issue) { continue; }
        if !search.is_empty() && ![&issue.message, &issue.path, &issue.code, &issue.source].iter().any(|s| s.to_lowercase().contains(&search)) { continue; }
        groups.entry(Group::of(issue)).or_default().push((index, issue));
    }
    if groups.is_empty() {
        ui.add_space(10.0);
        wrapped(ui, t("No findings match the filters or the search."), STEEL);
        coverage(ui, saved, view);
        return;
    }
    for (group, issues) in groups {
        ui.add_space(8.0);
        let folded = view.folded.contains(&group);
        if group_header(ui, group, issues.len(), folded).clicked() {
            if folded { view.folded.remove(&group); } else { view.folded.insert(group); }
        }
        if folded { continue; }
        let whole = view.whole.contains(&group) || issues.len() <= GROUP_PREVIEW + 5;
        let shown = if whole { issues.len() } else { GROUP_PREVIEW };
        for &(index, issue) in issues.iter().take(shown) {
            ui.add_space(4.0);
            let open = view.fixes.contains(&index);
            if issue_card(ui, issue, open) {
                if open { view.fixes.remove(&index); } else { view.fixes.insert(index); }
            }
        }
        if !whole {
            ui.add_space(4.0);
            let label = tf("SHOW ALL {n}", &[("n", &grouped(issues.len() as u64))]);
            if ui.add(ActionButton::secondary(&label).height(26.0)).clicked() { view.whole.insert(group); }
        }
    }
    coverage(ui, saved, view);
}

/// Under the findings: what the check covered and claims, and what the filters hide.
fn coverage(ui: &mut Ui, saved: &SavedReport, view: &ReportView) {
    ui.add_space(14.0);
    ta_ui::panel::rule(ui);
    let counts = &saved.report.counts;
    let layers: Vec<String> = saved.scope.iter().map(|layer| scope_label(layer)).collect();
    wrapped(ui, tf("Scope: {layers}", &[("layers", &layers.join(" → "))]), STEEL_DIM);
    wrapped(ui, tf("{files} files · {units} units · {maps} maps · {sounds} sounds", &[("files", &grouped(counts.files_checked)), ("units", &grouped(counts.units)), ("maps", &grouped(counts.maps)), ("sounds", &grouped(counts.sounds))]), STEEL_DIM);
    let hidden = saved.report.issues.iter().filter(|i| !i.affects_status).count();
    if !view.shown.inherited && hidden > 0 {
        wrapped(ui, tf("{n} findings in the original game, Reborn or a dependency are there without this mod too, and hidden: INHERITED shows them.", &[("n", &grouped(hidden as u64))]), STEEL_DIM);
    }
    wrapped(ui, t("Checks installed data, references and asset limits. It does not certify gameplay, balance, multiplayer or visual quality."), STEEL_DIM);
}

/// A layer of a report's scope as the reader reads it: the original game's and TA Reborn's own
/// (kept in English in the report, which is shared), the mods' by their names.
fn scope_label(layer: &str) -> String {
    match layer {
        "Original game" => t("Base game").to_string(),
        "Reborn built-in content" => "TA Reborn".to_string(),
        other => other.to_string(),
    }
}

/// A group's heading: folded or open, its title and how many findings it shows.
fn group_header(ui: &mut Ui, group: Group, count: usize, folded: bool) -> egui::Response {
    let title = format!("{}  ·  {}", t(group.title()), grouped(count as u64));
    let text = widgets::galley(ui, RichText::new(&title).font(strong(12.5)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, !folded, &title));
    let painter = ui.painter();
    if response.hovered() { painter.rect_filled(rect, CornerRadius::same(2), Color32::from_white_alpha(6)); }
    // A chevron, down while open.
    let c = pos2(rect.min.x + 8.0, rect.center().y);
    let points = if folded { vec![c + vec2(-2.5, -4.5), c + vec2(2.5, 0.0), c + vec2(-2.5, 4.5)] } else { vec![c + vec2(-4.5, -2.5), c + vec2(0.0, 2.5), c + vec2(4.5, -2.5)] };
    painter.add(egui::Shape::line(points, Stroke::new(1.8, AMBER)));
    // A rule from the title to the edge.
    let rule = rect.min.x + 28.0 + text.size().x;
    painter.galley(pos2(rect.min.x + 20.0, rect.center().y - text.size().y / 2.0), text, TEXT_BRIGHT);
    if rule < rect.max.x { painter.hline(rule..=rect.max.x, rect.center().y, Stroke::new(1.0, LINE)); }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One finding: its severity's bar and word, the message, where it is (file, line, archive)
/// and HOW TO FIX, opened on a click. Returns whether the fix was toggled.
fn issue_card(ui: &mut Ui, issue: &Issue, open: bool) -> bool {
    let colour = if !issue.affects_status { STEEL } else { match issue.severity { Severity::Error => RED, Severity::Warning => AMBER, Severity::Info => SKY } };
    let mut toggled = false;
    let frame = egui::Frame::new().fill(INSET).inner_margin(Margin { left: 14, right: 10, top: 8, bottom: 8 }).corner_radius(3).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal_wrapped(|ui| {
            widgets::tag(ui, match issue.severity { Severity::Error => n("ERROR"), Severity::Warning => n("WARNING"), Severity::Info => n("NOTE") }, colour);
            match issue.origin {
                Origin::Caused => { widgets::tag(ui, "CAUSED BY THIS MOD", RED).hover_tip(t("In a file this mod does not provide, and only there with the mod: it changes something this file relies on.")); }
                Origin::Inherited => { widgets::tag(ui, "INHERITED", STEEL).hover_tip(t("There without this mod too: the original game's, Reborn's or a dependency's, and not counted against this mod.")); }
                Origin::Mod => {}
            }
        });
        ui.add(egui::Label::new(RichText::new(&issue.message).color(if issue.affects_status { TEXT } else { STEEL })).wrap());
        // Where: the file, its line, the archive it came from; the code for searching.
        let mut place = Vec::new();
        if !issue.path.is_empty() { place.push(issue.path.clone()); }
        if let Some(line) = issue.line { place.push(tf("line {n}", &[("n", &line)])); }
        if !issue.source.is_empty() { place.push(tf("in {archive}", &[("archive", &issue.source)])); }
        place.push(issue.code.clone());
        ui.add(egui::Label::new(RichText::new(place.join("  ·  ")).font(mono(11.5)).color(STEEL_DIM)).wrap());
        if !issue.suggestion.is_empty() {
            let label = if open { t("HIDE THE FIX") } else { t("HOW TO FIX") };
            let link = ui.add(egui::Label::new(RichText::new(label).font(strong(11.0)).color(SKY).extra_letter_spacing(ta_ui::i18n::tracking(1.0))).sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if link.clicked() { toggled = true; }
            if open { wrapped(ui, &issue.suggestion, SKY); }
        }
    });
    // The severity's bar down the card's left edge.
    let rect = frame.response.rect;
    ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(3.0, rect.height())), CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 }, colour);
    toggled
}

fn project_review(ui: &mut Ui, entry: &Entry) {
    let version = entry.release().version;
    wrapped(ui, tf("Reviewed release: {version}", &[("version", &if version.is_empty() { t("Unknown") } else { version })]), TEXT_BRIGHT);
    wrapped(ui, t("These observations come from project testing of the listed release. They are separate from checks of your installed files and may differ for other versions."), STEEL);
    if !entry.note.is_empty() { ui.add_space(8.0); wrapped(ui, t(entry.note), TEXT); }
    if entry.fit == Fit::Unverified || entry.fit == Fit::NoFile {
        wrapped(ui, t("Gameplay compatibility has not been verified for this release."), AMBER);
    } else if entry.review_issues().is_empty() && entry.note.is_empty() {
        wrapped(ui, t("No compatibility gaps are recorded in this project review. This is not a guarantee that every feature has been tested."), TEXT);
    }
    for issue in entry.review_issues() {
        ui.add_space(10.0);
        egui::Frame::new().fill(INSET).inner_margin(Margin::same(10)).corner_radius(3).show(ui, |ui| {
            if issue.inherited { widgets::tag(ui, "INHERITED", STEEL); }
            ui.add(egui::Label::new(RichText::new(t(issue.title)).font(strong(14.0)).color(if issue.inherited { STEEL } else { AMBER })).wrap());
            ui.add(egui::Label::new(RichText::new(issue.code).font(mono(12.0)).color(STEEL)).wrap());
            ui.add(egui::Label::new(RichText::new(issue.path).font(mono(12.0)).color(STEEL)).wrap());
            wrapped(ui, t(issue.detail), TEXT);
            wrapped(ui, tf("Suggested action: {action}", &[("action", &t(issue.suggestion))]), SKY);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    #[test]
    fn automatic_badges_never_claim_playability_and_keep_all_states_distinct() {
        let states = [ValidationStatus::Unknown, ValidationStatus::Queued, ValidationStatus::Checking, ValidationStatus::Stale,
            ValidationStatus::Failed("worker error".into()), ValidationStatus::Ready(Status::Clean), ValidationStatus::Ready(Status::Issues),
            ValidationStatus::Ready(Status::Errors), ValidationStatus::Ready(Status::Incomplete), ValidationStatus::Ready(Status::Empty)];
        let labels: std::collections::HashSet<_> = states.iter().map(|state| validation_badge(state).0).collect();
        assert_eq!(labels.len(), states.len(), "a mod's errors and a check that could not run read differently");
        assert!(labels.iter().all(|label| !label.contains("PLAY")));
    }

    #[test]
    fn findings_are_grouped_by_what_they_concern_and_inherited_ones_wait_to_be_asked_for() {
        let issue = |code: &str, path: &str| Issue { code: code.into(), path: path.into(), ..Issue::default() };
        for (code, path, group) in [
            ("reference.missing", "units/armck.fbi", Group::Units), ("reference.missing", "weapons/w.tdf", Group::Weapons),
            ("format.invalid", "objects3d/x.3do", Group::Models), ("sound.format", "sounds/bang.wav", Group::Sounds),
            ("reference.missing", "gamedata/sound.tdf", Group::Sounds), ("reference.missing", "gamedata/sidedata.tdf", Group::Menus),
            ("packaging.wrapped", "ta content", Group::Packaging), ("ARCHIVE_UNREADABLE", "Broken.ufo", Group::Files),
            ("DEPENDENCY", "", Group::Check), ("render.sprite_capacity", "", Group::Animations), ("format.invalid", "readme.pcx", Group::Files),
        ] {
            assert_eq!(Group::of(&issue(code, path)), group, "{code} {path}");
        }
        let shown = ReportView::new(ReportTab::Automated).shown;
        let counted = Issue { severity: Severity::Warning, ..issue("reference.missing", "units/a.fbi") };
        let inherited = Issue { affects_status: false, origin: Origin::Inherited, ..counted.clone() };
        assert!(shown.shows(&counted) && !shown.shows(&inherited), "what counts shows; what was there before waits for INHERITED");
        assert!(Shown { inherited: true, ..shown }.shows(&inherited));
        assert!(!Shown { warnings: false, ..shown }.shows(&counted));
    }

    #[test]
    fn every_partial_project_review_has_copyable_actionable_findings_for_its_release() {
        for entry in catalogue::MODS.iter().filter(|entry| entry.fit == Fit::Partly) {
            assert!(!entry.review_issues().is_empty(), "{} needs its known gaps listed", entry.id);
            let text = entry.review_text();
            assert!(text.contains(entry.release().version) && text.contains(entry.note));
            for issue in entry.review_issues() {
                assert!(!issue.code.is_empty() && !issue.detail.is_empty() && !issue.suggestion.is_empty());
                assert!(text.contains(issue.code) && text.contains(issue.suggestion));
            }
        }
    }

    #[test]
    fn copy_uses_the_complete_report_with_every_issue_and_preserves_paths() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let automatic = ta_data::validation::Report {
            complete: true,
            issues: (0..137).map(|i| Issue {
                code: format!("reference.missing.{i}"), path: format!("units/ship{i}.fbi"), message: format!("Missing weapon {i}"),
                suggestion: "Provide this weapon or correct its reference.".into(), ..Issue::default()
            }).collect(),
            ..Default::default()
        };
        let report = copied_report("TA: Escalation", "escalation", Some(automatic.to_text()), &ValidationStatus::Ready(Status::Issues), catalogue::get("escalation"));
        let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(440.0, 120.0))), ..Default::default() };
        let mut rect = egui::Rect::NOTHING;
        ctx.run_ui(input(), |ui| { rect = copy_report_button(ui, false).rect; }).drop_without_applying_deltas();
        let mut copied = None;
        for pressed in [true, false] {
            let mut raw = input();
            raw.events = vec![egui::Event::PointerMoved(rect.center()), egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }];
            let mut output = ctx.run_ui(raw, |ui| { if copy_report_button(ui, false).clicked() { ui.ctx().copy_text(report.clone()); } });
            output.textures_delta.clear();
            for command in output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command { copied = Some(text); }
            }
        }
        assert_eq!(copied.as_deref(), Some(report.as_str()));
        for i in 0..137 { assert!(report.contains(&format!("units/ship{i}.fbi")), "copy must include findings beyond the first page"); }
        for issue in catalogue::get("escalation").unwrap().review_issues() {
            assert!(report.contains(issue.code) && report.contains(issue.path) && report.contains(issue.suggestion));
        }
        let inherited = catalogue::get("escalation").unwrap().review_issues().iter().find(|issue| issue.path.contains("terrain.gaf")).unwrap();
        assert!(inherited.inherited && report.contains("(inherited base-game context)"), "stock placeholders must not be blamed on Escalation");
    }

    #[test]
    fn reviewed_and_unknown_mod_reports_fit_compact_and_large_windows_without_resizing() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let saved: SavedReport = serde_json::from_value(serde_json::json!({
            "mod_id": "layout", "name": "TA: Escalation", "version": "Gold 10.2.0", "reborn_version": "2.1.0",
            "checked_unix": 1792790000u64, "scope": ["Original game", "Reborn built-in content", "escalation"],
            "fingerprint": "test", "cache_version": 1,
            "report": ta_data::validation::Report { complete: true, issues: vec![Issue {
                code: "model.missing".into(), path: "objects3d/ship.3do".into(), message: "Missing model".into(), ..Issue::default()
            }], ..Default::default() }
        })).unwrap();
        for (width, height) in [(671.0, 514.0), (1120.0, 850.0)] {
            let mut first: Option<f32> = None;
            for tab in [ReportTab::Automated, ReportTab::Review] {
            for status in [ValidationStatus::Unknown, ValidationStatus::Checking, ValidationStatus::Ready(Status::Issues), ValidationStatus::Failed("Worker could not finish".into())] {
                ctx.data_mut(|data| data.insert_temp(egui::Id::new(("mod_report_view", "layout")), ReportView::new(tab)));
                let mut output = None;
                let mut bounds = None;
                for time in [20.0, 21.0, 22.0] {
                    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, height))), time: Some(time), ..Default::default() };
                    let mut frame = ctx.run_ui(raw, |ui| {
                        bounds = report_dialog(ui.ctx(), "layout", "TA: Escalation", true, &status, Some(&saved), catalogue::get("escalation"), None, Some(&("12 of 40 files".to_string(), Some(0.3)))).rect;
                    });
                    frame.textures_delta.clear();
                    output = Some(frame);
                }
                let bounds = bounds.unwrap();
                assert!(bounds.min.x >= 0.0 && bounds.max.x <= width + 1.0 && bounds.min.y >= 0.0 && bounds.max.y <= height + 1.0, "dialog must fit: {bounds:?}");
                if let Some(first) = first { assert!((bounds.height() - first).abs() <= 1.0, "tab/state changes must not resize the dialog"); }
                else { first = Some(bounds.height()); }
                let mut copy_visible = false;
                for shape in output.unwrap().shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        if text.galley.job.text == "COPY FULL REPORT" {
                            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                            assert!(rect.min.x >= 0.0 && rect.max.x <= width && rect.min.y >= 0.0 && rect.max.y <= height, "copy must be reachable: {rect:?}");
                            copy_visible = true;
                        }
                    }
                }
                assert!(copy_visible);
            }
            }
        }
    }
}
