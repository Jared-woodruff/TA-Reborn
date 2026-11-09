//! A shared, read-only commander dossier. Hosts load the snapshot off the UI thread.
use std::io::{self, Read};
use egui::{RichText, Ui};
use ta_config::profiles::{Career, MatchMode, MatchRecord, Profile, ProfileStore};
use crate::i18n::{grouped, n, t, tf};
use crate::theme::*;
use crate::widgets;

#[derive(Clone, Default)]
pub struct CampaignSummary {
    pub id: String,
    pub title: String,
    pub missions: Vec<String>,
}

#[derive(Clone, Default)]
pub struct CareerData {
    pub career: Career,
    pub history: Vec<MatchRecord>,
    pub progress: ta_config::CampaignProgress,
    pub campaigns: Vec<CampaignSummary>,
    pub avatar_path: Option<std::path::PathBuf>,
    pub warning: Option<String>,
    /// The achievements with a picture of their moment, and the folder they are kept in.
    pub pictures: std::collections::BTreeSet<String>,
    pub picture_folder: Option<std::path::PathBuf>,
}

impl CareerData {
    /// Read once on opening, never while painting a frame. Legacy campaign unlocks
    /// stay separate from verified wins, which are the achievement source.
    pub fn load(store: &ProfileStore, id: &str) -> io::Result<Self> {
        let (career, history) = store.career_with_history(id)?;
        let mut data = Self { career, history,
            avatar_path: store.avatar_path(id).ok(), ..Default::default() };
        match store.achievement_pictures(id) {
            Ok(pictures) => data.pictures = pictures,
            Err(error) => log::warn!("achievement pictures: {error}"),
        }
        data.picture_folder = store.data_dir(id).ok().map(|dir| dir.join("achievements"));
        let path = store.data_dir(id)?.join("progress.toml");
        match std::fs::File::open(path) {
            Ok(file) => {
                let mut text = String::new();
                match file.take(1024 * 1024 + 1).read_to_string(&mut text)
                    .and_then(|_| if text.len() > 1024 * 1024 { Err(io::Error::other("campaign progress exceeds the size limit")) }
                    else { toml::from_str(&text).map_err(io::Error::other) }) {
                    Ok(progress) => data.progress = progress,
                    Err(error) => data.warning = Some(error.to_string()),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => data.warning = Some(error.to_string()),
        }
        Ok(data)
    }
}

/// The parts of a service record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CareerTab { #[default] Overview, Achievements, Campaign, History }

/// What a viewer of a service record asked for, for its host to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CareerAction {
    /// The achievements the profile shows, as they are now to be.
    Showcase(Vec<String>),
}

#[derive(Default)]
pub struct CareerView {
    pub tab: CareerTab,
    achievements: super::achievements::AchievementsView,
    history_page: usize,
}

/// The game's dossier controller. The launcher supplies its own loading controller
/// because it also discovers installed campaigns; neither host blocks a frame on IO.
#[derive(Default)]
pub struct CareerWindow {
    open: bool,
    view: CareerView,
    data: Option<CareerData>,
    pending: Option<std::sync::mpsc::Receiver<Result<CareerData, String>>>,
    error: Option<String>,
}

impl CareerWindow {
    pub fn is_open(&self) -> bool { self.open }

    pub fn open(&mut self, ctx: &egui::Context, profile: &Profile, campaigns: Vec<CampaignSummary>, achievements: bool) {
        self.open = true;
        self.view = CareerView::default();
        if achievements { self.view.achievements(); }
        self.data = None;
        self.error = None;
        let id = profile.id.clone();
        let root = ta_config::config_dir();
        let ctx = ctx.clone();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = ProfileStore::open(root).and_then(|s| CareerData::load(&s, &id)).map(|mut d| { d.campaigns = campaigns; d }).map_err(|e| e.to_string());
            let _ = send.send(result);
            ctx.request_repaint();
        });
        self.pending = Some(receive);
    }

    pub fn show(&mut self, ctx: &egui::Context, profile: &Profile) -> Option<CareerAction> {
        if !self.open { return None; }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(Ok(data)) => { self.data = Some(data); self.pending = None; }
                Ok(Err(error)) => { self.error = Some(error); self.pending = None; }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => { self.error = Some(t("Could not load profile.").to_string()); self.pending = None; }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let panel = crate::panel::Panel::new("COMMANDER DOSSIER").width((ctx.content_rect().width() - 76.0).clamp(180.0, 760.0))
            .max_height((ctx.content_rect().height() - 30.0).clamp(120.0, 760.0)).exit(crate::panel::Exit::Close);
        let mut action = None;
        let response = crate::panel::modal(ctx, "game_commander_dossier", panel, true, |ui| {
            ui.set_min_height(ui.available_height());
            if let Some(data) = &self.data { action = self.view.show(ui, profile, data); }
            else if let Some(error) = &self.error {
                ui.add(egui::Label::new(RichText::new(tf("Could not load profile: {error}", &[("error", error)])).color(RED)).wrap());
            } else { ui.label(t("Loading service record…")); crate::panel::busy_bar(ui); }
        });
        if response.exit { self.open = false; self.pending = None; }
        action
    }
}

impl CareerView {
    pub fn achievements(&mut self) { self.tab = CareerTab::Achievements; }
    /// Opens the page of one achievement.
    pub fn achievement(&mut self, id: &'static str) { self.tab = CareerTab::Achievements; self.achievements.selected = Some(id); }
    pub fn campaign_progress(&mut self) { self.tab = CareerTab::Campaign; }
    pub fn match_history(&mut self) { self.tab = CareerTab::History; }

    /// The whole record in a dialog: its head, its tabs and the tab chosen.
    pub fn show(&mut self, ui: &mut Ui, profile: &Profile, data: &CareerData) -> Option<CareerAction> {
        ui.spacing_mut().item_spacing.y = if ui.available_height() < 480.0 { 6.0 } else { 10.0 };
        // Reserve the panel body before laying out a tab. Empty records and filtered
        // lists occupy the same window as a full service record; only its contents scroll.
        ui.set_min_height(ui.available_height());
        if let Some(id) = super::hero::hero(ui, profile, data, super::HeroSize::for_dialog(ui.available_height())) { self.achievement(id); }
        widgets::segmented_fill(ui, &mut self.tab, &[(CareerTab::Overview, "OVERVIEW"), (CareerTab::Achievements, "ACHIEVEMENTS"),
            (CareerTab::Campaign, "CAMPAIGN"), (CareerTab::History, "MATCH HISTORY")]);
        self.body(ui, profile, data)
    }

    /// The tab chosen, in the room left: its contents scroll, the rest stays in sight.
    pub fn body(&mut self, ui: &mut Ui, profile: &Profile, data: &CareerData) -> Option<CareerAction> {
        if let Some(error) = &data.warning {
            ui.add(egui::Label::new(RichText::new(tf("Some campaign progress could not be loaded: {error}", &[("error", error)]))
                .color(AMBER)).wrap());
        }
        ui.push_id(profile.id.as_str(), |ui| {
            if self.tab == CareerTab::Achievements {
                let record = super::achievements::Record { career: &data.career, pictures: &data.pictures, folder: data.picture_folder.as_deref(), showcase: &profile.showcase };
                self.achievements.show(ui, &record)
            } else {
                // Each tab remembers its own position, while identity and navigation
                // remain visible. On very short screens the outer panel can also scroll.
                let area = content_scroll(ui, ("dossier_tab", self.tab as u8));
                crate::scroll::show(ui, area, crate::scroll::Behind::Plate, |ui| match self.tab {
                    CareerTab::Overview => super::overview::overview(ui, data),
                    CareerTab::Campaign => self.campaign(ui, data),
                    CareerTab::History => self.history(ui, data),
                    CareerTab::Achievements => unreachable!(),
                });
                None
            }
        }).inner
    }

    fn campaign(&self, ui: &mut Ui, data: &CareerData) {
        let mut campaigns = data.campaigns.clone();
        for id in data.progress.completed.keys().chain(data.career.campaign_completed.keys()) {
            if !campaigns.iter().any(|c| &c.id == id) {
                campaigns.push(CampaignSummary { id: id.clone(), title: id.clone(), missions: Vec::new() });
            }
        }
        if campaigns.is_empty() {
            empty(ui, n("Your campaign story starts here."), n("Complete a campaign mission to add it to your service record."));
            return;
        }
        ui.add(egui::Label::new(RichText::new(t("Campaign progress includes your existing saves. Verified victories and hard victories are tracked separately for achievements."))
            .color(STEEL).size(12.0)).wrap());
        for campaign in &campaigns {
            let legacy = data.progress.unlocked(&campaign.id);
            let verified = data.career.campaign_completed.get(&campaign.id).map_or(0, Vec::len);
            let hard = data.career.hard_completed.get(&campaign.id).map_or(0, Vec::len);
            widgets::card(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(&campaign.title).font(strong(17.0)).color(TEXT_BRIGHT)).wrap());
                if campaign.missions.is_empty() {
                    ui.label(tf("Campaign progress: {count} missions", &[("count", &legacy)]));
                } else {
                    let count = legacy.min(campaign.missions.len());
                    ui.label(tf("{done} / {total} missions complete", &[("done", &count), ("total", &campaign.missions.len())]));
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 13.0), egui::Sense::hover());
                    super::paint_xp_track(ui, rect, count as f32 / campaign.missions.len() as f32);
                    if let Some(next) = campaign.missions.get(count) {
                        ui.add(egui::Label::new(RichText::new(tf("Next mission: {name}", &[("name", next)])).color(STEEL)).wrap());
                    } else { ui.colored_label(GREEN, t("Campaign complete")); }
                }
                ui.label(RichText::new(tf("{wins} verified victories · {hard} on Hard", &[("wins", &verified), ("hard", &hard)])).color(AMBER).size(12.0));
            });
        }
    }

    fn history(&mut self, ui: &mut Ui, data: &CareerData) {
        if data.history.is_empty() {
            empty(ui, n("No matches recorded yet."), n("Your battles will appear here as you play, including their results, statistics and earned XP."));
            return;
        }
        ui.label(RichText::new(t("Most recent matches first")).color(STEEL).size(12.0));
        const PER_PAGE: usize = 25;
        let pages = data.history.len().div_ceil(PER_PAGE).max(1);
        self.history_page = self.history_page.min(pages - 1);
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(self.history_page > 0, widgets::ActionButton::secondary("PREVIOUS").height(28.0)).clicked() { self.history_page -= 1; }
            ui.label(tf("Page {page} of {pages}", &[("page", &(self.history_page + 1)), ("pages", &pages)]));
            if ui.add_enabled(self.history_page + 1 < pages, widgets::ActionButton::secondary("NEXT").height(28.0)).clicked() { self.history_page += 1; }
        });
        for record in data.history.iter().skip(self.history_page * PER_PAGE).take(PER_PAGE) {
            let c = &record.counters;
            let label = if c.won { n("VICTORY") } else if c.finished { n("DEFEAT") } else { n("UNFINISHED") };
            let color = if c.won { GREEN } else if c.finished { RED } else { STEEL };
            widgets::card_accent(ui, Some(color), |ui| {
                ui.horizontal_wrapped(|ui| {
                    widgets::tag(ui, label, color);
                    ui.label(RichText::new(t(mode_label(record.context.mode))).font(strong(13.0)).color(TEXT_BRIGHT));
                    ui.label(RichText::new(tf("+{xp} XP", &[("xp", &grouped(record.earned_xp))])).color(AMBER));
                });
                if !record.context.map.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&record.context.map).font(strong(15.0))).wrap());
                }
                let mut context = vec![duration(c.played_seconds)];
                if !record.context.faction.is_empty() { context.push(record.context.faction.clone()); }
                if !record.context.difficulty.is_empty() { context.push(match record.context.difficulty.to_ascii_lowercase().as_str() {
                    "easy" => t("Easy").to_string(), "normal" | "medium" => t("Normal").to_string(),
                    "hard" => t("Hard").to_string(), _ => record.context.difficulty.clone(),
                }); }
                if record.started_at != 0 { context.push(utc_date(record.started_at)); }
                ui.add(egui::Label::new(RichText::new(context.join(" · ")).color(STEEL).size(12.0)).wrap());
                ui.label(tf("{kills} destroyed · {built} built · {captured} captured", &[("kills", &grouped(c.kills)), ("built", &grouped(c.units_built)), ("captured", &grouped(c.units_captured))]));
            });
        }
    }
}

fn content_scroll(ui: &Ui, id: impl egui::AsIdSalt) -> egui::ScrollArea {
    egui::ScrollArea::vertical().id_salt(id).max_height(ui.available_height().max(96.0))
        .min_scrolled_height(96.0).auto_shrink([false, false])
}

pub(super) fn mode_label(mode: MatchMode) -> &'static str {
    match mode {
        MatchMode::Unknown => n("Legacy match"), MatchMode::Campaign => n("Campaign"), MatchMode::Skirmish => n("Skirmish"),
        MatchMode::Multiplayer => n("Multiplayer"), MatchMode::Scenario => n("Scenario"), MatchMode::Conquest => n("Conquest"),
        MatchMode::Tutorial => n("Tutorial"),
    }
}

fn empty(ui: &mut Ui, title: &str, detail: &str) {
    widgets::card(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(t(title)).font(strong(17.0)).color(TEXT_BRIGHT)).wrap());
        ui.add(egui::Label::new(RichText::new(t(detail)).color(STEEL)).wrap());
    });
}

pub(super) fn duration(seconds: u64) -> String {
    tf("{hours}h {minutes}m", &[("hours", &(seconds / 3600)), ("minutes", &(seconds / 60 % 60))])
}

/// Civil UTC date without platform APIs or a timezone-dependent ordering. A label
/// names UTC so that a late-night match isn't presented as a local calendar date.
pub(super) fn utc_date(timestamp: u64) -> String {
    let days = (timestamp.min(253402300799) / 86400) as i64 + 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_dates_are_stable_at_month_year_and_leap_boundaries() {
        assert_eq!(utc_date(0), "1970-01-01 UTC");
        assert_eq!(utc_date(1709164800), "2024-02-29 UTC");
        assert_eq!(utc_date(1735689600), "2025-01-01 UTC");
        assert_eq!(utc_date(u64::MAX), "9999-12-31 UTC");
    }

    #[test]
    fn dossier_modal_keeps_its_bounds_across_tabs_filters_and_loading() {
        for viewport in [egui::vec2(1280.0, 900.0), egui::vec2(720.0, 540.0), egui::vec2(427.0, 360.0)] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            crate::motion::set_reduced(&ctx, true);
            let profile = Profile { id: "test".into(), name: "Commander".into(), allegiance: ta_config::profiles::Allegiance::Arm, has_avatar: false, created_at: 0, ..Default::default() };
            let data = CareerData::default();
            let mut view = CareerView::default();
            let mut baseline: Option<egui::Rect> = None;
            // Includes the loading placeholder, all four dossier tabs, and empty
            // achievement results. A short tab must not move the header/close button.
            for state in 0..7 {
                view.tab = match state { 1 => CareerTab::Overview, 2 => CareerTab::Campaign, 3 => CareerTab::History, _ => CareerTab::Achievements };
                view.achievements.category = (state == 5).then_some(ta_config::achievements::Category::Multiplayer);
                view.achievements.status = if state == 6 { super::super::achievements::Status::Earned } else { super::super::achievements::Status::All };
                let mut rect = egui::Rect::NOTHING;
                for _ in 0..4 {
                    ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)), ..Default::default() }, |_| {
                        let panel = crate::panel::Panel::new("COMMANDER DOSSIER")
                            .width((viewport.x - 76.0).clamp(180.0, 760.0))
                            .max_height((viewport.y - 30.0).clamp(120.0, 760.0)).exit(crate::panel::Exit::Close);
                        rect = crate::panel::modal(&ctx, "stable_dossier_test", panel, true, |ui| {
                            ui.set_min_height(ui.available_height());
                            if state == 0 { ui.label(t("Loading service record…")); }
                            else { view.show(ui, &profile, &data); }
                        }).rect;
                    }).drop_without_applying_deltas();
                }
                assert!(rect.width() <= viewport.x && rect.height() <= viewport.y, "state {state}: {rect:?} exceeds {viewport:?}");
                if let Some(expected) = baseline {
                    assert!((rect.min - expected.min).length() < 1.0 && (rect.max - expected.max).length() < 1.0,
                        "state {state} at {viewport:?}: {rect:?} moved from {expected:?}");
                } else { baseline = Some(rect); }
            }
        }
    }

    #[test]
    fn compact_achievements_leave_room_for_the_first_result() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        crate::motion::set_reduced(&ctx, true);
        let profile = Profile { id: "test".into(), name: "QA Commander".into(), allegiance: ta_config::profiles::Allegiance::Arm, has_avatar: false, created_at: 0, ..Default::default() };
        let mut view = CareerView { tab: CareerTab::Achievements, ..Default::default() };
        let mut visible = false;
        for _ in 0..4 {
            let output = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(671.0, 514.0))), ..Default::default() }, |_| {
                let panel = crate::panel::Panel::new("COMMANDER DOSSIER").width(595.0).max_height(484.0).exit(crate::panel::Exit::Close);
                crate::panel::modal(&ctx, "compact_achievements_test", panel, true, |ui| view.show(ui, &profile, &CareerData::default()));
            });
            fn visible_name(shape: &egui::Shape, clip: egui::Rect) -> bool {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == "First Orders" => clip.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())),
                    egui::Shape::Vec(shapes) => shapes.iter().any(|s| visible_name(s, clip)),
                    _ => false,
                }
            }
            visible = output.shapes.iter().any(|s| visible_name(&s.shape, s.clip_rect));
            output.drop_without_applying_deltas();
        }
        assert!(visible, "identity and filters must leave a visible achievement at high UI scale");
    }

    #[test]
    fn dossier_tabs_reflow_without_widening_small_viewports() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        crate::motion::set_reduced(&ctx, true);
        let profile = Profile { id: "test".into(), name: "A commander with a name".into(), allegiance: ta_config::profiles::Allegiance::Arm, has_avatar: false, created_at: 0, ..Default::default() };
        let mut data = CareerData::default();
        data.campaigns.push(CampaignSummary { id: "arm campaign".into(), title: "Arm Campaign — Core Contingency".into(), missions: vec!["A Long First Mission Name".into(); 25] });
        data.history.push(MatchRecord { id: "game".into(), context: ta_config::profiles::MatchContext { mode: MatchMode::Skirmish, map: "A Battlefield With A Long Map Name".into(), faction: "ARM".into(), difficulty: "hard".into(), ..Default::default() },
            counters: ta_config::profiles::MatchCounters { won: true, finished: true, ..Default::default() }, earned_xp: 400, started_at: 1709164800, updated_at: 1709164800 });
        for width in [240.0, 330.0, 520.0, 760.0] {
            for tab in [CareerTab::Overview, CareerTab::Campaign, CareerTab::History, CareerTab::Achievements] {
                let mut view = CareerView { tab, ..Default::default() };
                let mut actual = 0.0;
                for _ in 0..2 {
                    ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 700.0))), ..Default::default() }, |ui| {
                        egui::ScrollArea::vertical().max_height(680.0).show(ui, |ui| {
                            view.show(ui, &profile, &data);
                            actual = ui.min_rect().width();
                        });
                    }).drop_without_applying_deltas();
                }
                assert!(actual <= width + 1.0, "tab {} widened {width} to {actual}", tab as u8);
            }
        }
    }
}
