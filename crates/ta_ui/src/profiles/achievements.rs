//! The achievements of a service record: how many are earned, of each tier, the latest earned,
//! and the whole list, filtered and sorted, each with its badge in its metal or greyed; and a
//! chosen one's page, with the picture taken the moment it was earned.

use std::collections::BTreeSet;
use std::path::Path;

use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use ta_config::achievements::{self, Achievement, Category, Tier};
use ta_config::profiles::{Career, MatchMode};

use crate::badges::{self, Look};
use crate::i18n::{grouped, n, t, tf};
use crate::theme::*;
use crate::widgets;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status { #[default] All, Earned, InProgress, NotEarned }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort { #[default] Catalogue, Recent, Nearest, Tier }

/// The list's choices, and the achievement whose page is open.
#[derive(Default)]
pub struct AchievementsView {
    pub category: Option<Category>,
    pub status: Status,
    pub sort: Sort,
    pub query: String,
    pub selected: Option<&'static str>,
}

/// What the list is drawn from: a career, and where its pictures are.
pub struct Record<'a> {
    pub career: &'a Career,
    /// The achievements the profile shows (its showcase).
    pub showcase: &'a [String],
    /// The achievements with a picture of their moment, and the folder they are in.
    pub pictures: &'a BTreeSet<String>,
    pub folder: Option<&'a Path>,
}

const CARD_H: f32 = 78.0;

impl AchievementsView {
    pub fn show(&mut self, ui: &mut Ui, record: &Record) -> Option<super::CareerAction> {
        if let Some(id) = self.selected {
            match achievements::find(id) {
                Some(a) => return self.page(ui, a, record),
                None => self.selected = None,
            }
        }
        let career = record.career;
        // On a short page the choices take one row of drop-down lists, and the list keeps the
        // rest; otherwise they stay in sight above it, and the summary scrolls with it.
        let compact = ui.available_height() < 320.0;
        let categories = [(None, n("ALL")), (Some(Category::Campaign), badges::category_label(Category::Campaign)),
            (Some(Category::Skirmish), badges::category_label(Category::Skirmish)), (Some(Category::Multiplayer), badges::category_label(Category::Multiplayer)),
            (Some(Category::Battle), badges::category_label(Category::Battle)), (Some(Category::Industry), badges::category_label(Category::Industry)),
            (Some(Category::Service), badges::category_label(Category::Service))];
        let statuses = [(Status::All, n("ALL")), (Status::Earned, n("EARNED")), (Status::InProgress, n("IN PROGRESS")), (Status::NotEarned, n("NOT EARNED"))];
        let sorts = [(Sort::Catalogue, n("In list order")), (Sort::Recent, n("Recently earned")), (Sort::Nearest, n("Nearest to earning")), (Sort::Tier, n("Rarest tier first"))];
        let sort_width = widgets::combo_width(ui, sorts.iter().map(|(_, l)| t(l)), 120.0);
        if compact {
            ui.horizontal(|ui| {
                drop_down(ui, "achievement_category", &mut self.category, &categories);
                drop_down(ui, "achievement_status", &mut self.status, &statuses);
                widgets::search_field(ui, &mut self.query, t("Search achievements…"), ui.available_width().max(80.0), None);
            });
        } else {
            widgets::segmented_fill(ui, &mut self.category, &categories);
            widgets::segmented(ui, &mut self.status, &statuses);
            let search_width = ui.available_width() - sort_width - ui.spacing().item_spacing.x;
            if search_width >= 220.0 {
                ui.horizontal(|ui| {
                    widgets::search_field(ui, &mut self.query, t("Search achievements…"), search_width, None);
                    drop_down(ui, "achievement_sort", &mut self.sort, &sorts);
                });
            } else {
                drop_down(ui, "achievement_sort", &mut self.sort, &sorts);
                widgets::search_field(ui, &mut self.query, t("Search achievements…"), ui.available_width(), None);
            }
        }
        let list = self.list(career);
        let area = egui::ScrollArea::vertical().id_salt("achievement_results").max_height(ui.available_height().max(96.0))
            .min_scrolled_height(96.0).auto_shrink([false, false]);
        crate::scroll::show(ui, area, crate::scroll::Behind::Plate, |ui| {
            if !compact {
                summary(ui, career);
                self.recent(ui, career);
            }
            if list.is_empty() {
                widgets::card(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(t("No achievements match your filters.")).font(strong(17.0)).color(TEXT_BRIGHT)).wrap());
                    ui.add(egui::Label::new(RichText::new(t("Try another search or show locked achievements too.")).color(STEEL)).wrap());
                });
            }
            let columns = if ui.available_width() >= 600.0 { 2 } else { 1 };
            let gap = 8.0;
            let width = (ui.available_width() - gap * (columns - 1) as f32) / columns as f32;
            for row in list.chunks(columns) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for a in row {
                        if card(ui, a, career, width).clicked() { self.selected = Some(a.id); }
                    }
                });
            }
        });
        None
    }

    /// The achievements shown: filtered, searched and sorted.
    fn list(&self, career: &Career) -> Vec<&'static Achievement> {
        let query = self.query.trim().to_lowercase();
        let mut list: Vec<&'static Achievement> = achievements::ALL.iter()
            .filter(|a| self.category.is_none_or(|c| a.category == c))
            .filter(|a| {
                let earned = a.is_unlocked(career);
                match self.status {
                    Status::All => true,
                    Status::Earned => earned,
                    Status::InProgress => !earned && a.progress(career) > 0,
                    Status::NotEarned => !earned,
                }
            })
            .filter(|a| query.is_empty() || {
                let (name, description) = shown_text(a, a.is_unlocked(career));
                name.to_lowercase().contains(&query) || description.to_lowercase().contains(&query)
            })
            .collect();
        let earned_at = |a: &Achievement| career.achievements.get(a.id).copied();
        let fraction = |a: &Achievement| if a.is_unlocked(career) || a.hidden { -1.0 } else { a.progress(career) as f64 / a.target.max(1) as f64 };
        match self.sort {
            Sort::Catalogue => {}
            Sort::Recent => list.sort_by_key(|a| std::cmp::Reverse(earned_at(a).map_or(0, |at| at + 1))),
            Sort::Nearest => list.sort_by(|a, b| fraction(b).total_cmp(&fraction(a))),
            Sort::Tier => list.sort_by_key(|a| std::cmp::Reverse(a.tier)),
        }
        list
    }

    /// The latest earned, as a strip of their badges and names.
    fn recent(&mut self, ui: &mut Ui, career: &Career) {
        // (Those earned in battle; those earned from the service record, all at one moment,
        // only when there are no others: the rarest first.)
        let all: Vec<(&'static Achievement, u64)> = achievements::ALL.iter()
            .filter_map(|a| career.achievements.get(a.id).map(|at| (a, *at))).collect();
        let earlier = |a: &Achievement| career.unlocks.get(a.id).is_some_and(|u| u.earlier);
        let mut earned: Vec<_> = all.iter().copied().filter(|(a, _)| !earlier(a)).collect();
        if earned.is_empty() { earned = all; }
        if earned.is_empty() { return; }
        earned.sort_by_key(|(a, at)| (std::cmp::Reverse(*at), std::cmp::Reverse(a.tier), a.index()));
        ui.label(widgets::caption("RECENTLY EARNED"));
        let count = ((ui.available_width() + 8.0) / 180.0).floor().clamp(1.0, 4.0) as usize;
        let width = (ui.available_width() - 8.0 * (count - 1) as f32) / count as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (a, at) in earned.iter().take(count) {
                let (rect, response) = ui.allocate_exact_size(vec2(width, 52.0), Sense::click());
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t(a.name)));
                widgets::reveal_focus(ui, &response);
                let painter = ui.painter();
                painter.rect_filled(rect, 5.0, if response.hovered() { PANEL_HI } else { Color32::from_black_alpha(70) });
                badges::paint(ui.ctx(), painter, Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(44.0, 44.0)), a.index(), Look::Earned);
                let text = Rect::from_min_max(rect.min + vec2(54.0, 7.0), rect.max - vec2(6.0, 6.0));
                let name = painter.layout_job(elided(t(a.name), strong(13.0), TEXT_BRIGHT, text.width(), 1));
                painter.galley(text.min, name, TEXT_BRIGHT);
                let date = painter.layout_job(elided(super::career::utc_date(*at), body(11.5), STEEL, text.width(), 1));
                painter.galley(pos2(text.min.x, text.max.y - 15.0), date, STEEL);
                widgets::focus_ring(ui, &response, rect);
                if response.clicked() { self.selected = Some(a.id); }
            }
        });
        ui.add_space(2.0);
    }

    /// One achievement's page: its badge large, what it asks, and its moment.
    fn page(&mut self, ui: &mut Ui, a: &'static Achievement, record: &Record) -> Option<super::CareerAction> {
        let career = record.career;
        let earned = a.is_unlocked(career);
        let secret = a.hidden && !earned;
        let mut action = None;
        ui.horizontal(|ui| {
            if ui.add(widgets::ActionButton::secondary("BACK").height(28.0)).clicked() || ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
                self.selected = None;
            }
            ui.label(RichText::new(t(badges::category_label(a.category))).font(strong(11.5)).color(STEEL));
            // (An earned achievement can be shown on the profile, up to five.)
            if earned {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let shown = record.showcase.iter().any(|id| id == a.id);
                    let full = record.showcase.len() >= ta_config::profiles::MAX_SHOWCASE;
                    let label = if shown { n("REMOVE FROM YOUR PROFILE") } else { n("SHOW ON YOUR PROFILE") };
                    let button = ui.add_enabled(shown || !full, widgets::ActionButton::secondary(label).height(28.0))
                        .on_disabled_hover_text(t("Your profile shows up to five achievements: remove one first."));
                    if button.clicked() {
                        let mut list = record.showcase.to_vec();
                        if shown { list.retain(|id| id != a.id); } else { list.push(a.id.to_string()); }
                        action = Some(super::CareerAction::Showcase(list));
                    }
                });
            }
        });
        if self.selected.is_none() { return action; }
        let area = egui::ScrollArea::vertical().id_salt(("achievement_page", a.id)).max_height(ui.available_height().max(96.0))
            .auto_shrink([false, false]);
        crate::scroll::show(ui, area, crate::scroll::Behind::Plate, |ui| {
            let wide = ui.available_width() >= 420.0;
            let size = if wide { 128.0 } else { 96.0 };
            let (name, description) = shown_text(a, earned);
            let head = |ui: &mut Ui| {
                ui.add(egui::Label::new(RichText::new(name.clone()).font(strong(24.0)).color(TEXT_BRIGHT)).wrap());
                ui.horizontal_wrapped(|ui| {
                    widgets::tag(ui, badges::tier_label(a.tier), badges::tier_color(a.tier));
                    if a.hidden { widgets::tag(ui, n("SECRET"), AMBER); }
                });
                ui.add(egui::Label::new(RichText::new(description.clone()).font(body(14.0)).color(TEXT)).wrap());
                ui.add_space(4.0);
                status(ui, a, career);
            };
            if wide {
                ui.horizontal_top(|ui| {
                    badges::badge(ui, a, size, Look::of(a, earned));
                    ui.add_space(8.0);
                    ui.vertical(|ui| { ui.set_width((ui.available_width()).max(120.0)); head(ui); });
                });
            } else {
                badges::badge(ui, a, size, Look::of(a, earned));
                head(ui);
            }
            ui.add_space(10.0);
            if earned {
                moment(ui, a, record);
            } else if secret {
                quiet(ui, t("This achievement is a secret until you earn it."));
            }
        });
        action
    }
}

/// An achievement's name and description as shown: a secret not yet earned keeps them.
fn shown_text(a: &Achievement, earned: bool) -> (String, String) {
    if a.hidden && !earned {
        (t("Secret achievement").to_string(), t("Keep playing to discover it.").to_string())
    } else {
        (t(a.name).to_string(), t(a.description).to_string())
    }
}

fn elided(text: impl Into<String>, font: egui::FontId, color: Color32, width: f32, rows: usize) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple(text.into(), font, color, width.max(1.0));
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = false;
    job.wrap.overflow_character = Some('…');
    job
}

/// A drop-down list of `options` (labels translated).
fn drop_down<T: PartialEq + Copy>(ui: &mut Ui, id: &str, value: &mut T, options: &[(T, &str)]) {
    let width = widgets::combo_width(ui, options.iter().map(|(_, l)| t(l)), 90.0).min(ui.available_width());
    let shown = options.iter().find(|(o, _)| o == value).map_or("", |(_, l)| *l);
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).width(width).selected_text(t(shown)).show_ui(ui, |ui| {
        for (option, label) in options {
            if ui.selectable_label(*value == *option, t(label)).clicked() && *value != *option {
                *value = *option;
                changed = true;
            }
        }
    });
    if changed { crate::sound::play(ui.ctx(), crate::sound::UiSound::Toggle); }
}

/// How many are earned, and of each tier.
fn summary(ui: &mut Ui, career: &Career) {
    let total = achievements::ALL.len();
    let earned = achievements::ALL.iter().filter(|a| a.is_unlocked(career)).count();
    let tally = |tier: Tier| {
        let of = achievements::ALL.iter().filter(|a| a.tier == tier);
        (of.clone().filter(|a| a.is_unlocked(career)).count(), of.count())
    };
    let wide = ui.available_width() >= 560.0;
    let left = |ui: &mut Ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{earned} / {total}")).font(strong(26.0)).color(AMBER));
            ui.label(RichText::new(t("ACHIEVEMENTS EARNED")).font(strong(11.0)).color(STEEL));
        });
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(320.0), 10.0), Sense::hover());
        super::paint_xp_track(ui, rect, earned as f32 / total.max(1) as f32);
    };
    let tiers = |ui: &mut Ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for tier in Tier::ALL.iter().rev() {
                let (done, of) = tally(*tier);
                let mut job = egui::text::LayoutJob::default();
                job.append(&format!("{done}/{of}"), 0.0, egui::TextFormat { font_id: strong(13.0), color: TEXT_BRIGHT, ..Default::default() });
                if wide {
                    job.append(t(badges::tier_label(*tier)), 5.0, egui::TextFormat { font_id: strong(10.0), color: STEEL,
                        extra_letter_spacing: crate::i18n::tracking(0.8), valign: egui::Align::Center, ..Default::default() });
                }
                let text = ui.painter().layout_job(job);
                let (rect, response) = ui.allocate_exact_size(vec2(16.0 + text.size().x, text.size().y.max(14.0)), Sense::hover());
                let dot = pos2(rect.min.x + 6.0, rect.center().y);
                ui.painter().circle_filled(dot, 5.5, badges::tier_color(*tier));
                ui.painter().circle_stroke(dot, 5.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
                ui.painter().galley(pos2(rect.min.x + 16.0, rect.center().y - text.size().y / 2.0), text, TEXT_BRIGHT);
                let label = tf("{tier}: {done} of {total}", &[("tier", &t(badges::tier_label(*tier))), ("done", &done), ("total", &of)]);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone()));
            }
        });
    };
    if wide {
        ui.horizontal(|ui| {
            ui.vertical(|ui| { ui.set_width(ui.available_width() * 0.55); left(ui); });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.vertical(tiers));
        });
    } else {
        left(ui);
        tiers(ui);
    }
}

/// A card of the list: its badge, name, what it asks and how far it is.
fn card(ui: &mut Ui, a: &'static Achievement, career: &Career, width: f32) -> egui::Response {
    let earned = a.is_unlocked(career);
    let (rect, response) = ui.allocate_exact_size(vec2(width, CARD_H), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let (name, description) = shown_text(a, earned);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{name}: {description}")));
    widgets::reveal_focus(ui, &response);
    if !ui.is_rect_visible(rect) { return response; }
    let painter = ui.painter();
    let accent = badges::tier_color(a.tier);
    painter.rect_filled(rect, 5.0, if response.hovered() || response.has_focus() { PANEL_HI } else { Color32::from_black_alpha(80) });
    painter.rect_stroke(rect, 5.0, Stroke::new(1.0, if earned { accent.gamma_multiply(0.45) } else { LINE }), StrokeKind::Inside);
    let badge = Rect::from_min_size(rect.min + vec2(8.0, (CARD_H - 60.0) / 2.0), vec2(60.0, 60.0));
    badges::paint(ui.ctx(), painter, badge, a.index(), Look::of(a, earned));
    let text = Rect::from_min_max(pos2(badge.max.x + 10.0, rect.min.y + 8.0), rect.max - vec2(10.0, 8.0));
    let title = painter.layout_job(elided(name, strong(14.5), if earned { TEXT_BRIGHT } else { TEXT }, text.width() - 16.0, 1));
    painter.galley(text.min, title.clone(), TEXT_BRIGHT);
    painter.circle_filled(pos2(text.max.x - 4.0, text.min.y + title.size().y / 2.0), 4.0, if earned { accent } else { accent.gamma_multiply(0.35) });
    let detail = painter.layout_job(elided(description, body(12.0), STEEL, text.width(), 2));
    painter.galley(pos2(text.min.x, text.min.y + title.size().y + 2.0), detail, STEEL);
    let bottom = Rect::from_min_max(pos2(text.min.x, text.max.y - 8.0), text.max);
    if earned {
        let at = career.achievements.get(a.id).copied().unwrap_or(0);
        let line = painter.layout_job(elided(tf("Earned {date}", &[("date", &super::career::utc_date(at))]), body(11.5), accent, text.width(), 1));
        painter.galley(pos2(text.min.x, text.max.y - line.size().y + 1.0), line, accent);
    } else if !a.hidden {
        let value = a.progress(career);
        let label = painter.layout_no_wrap(progress_text(a, value), body(11.0), STEEL);
        let bar = Rect::from_min_max(bottom.min + vec2(0.0, 2.0), pos2(bottom.max.x - label.size().x - 8.0, bottom.max.y));
        painter.add(crate::metal::well(bar, BG_DEEP));
        let fill = (value as f32 / a.target.max(1) as f32).clamp(0.0, 1.0);
        if fill > 0.0 {
            painter.rect_filled(Rect::from_min_size(bar.min + vec2(1.0, 1.0), vec2((bar.width() - 2.0) * fill, bar.height() - 2.0)), 1.0, accent.gamma_multiply(0.8));
        }
        painter.galley(pos2(bottom.max.x - label.size().x, bar.center().y - label.size().y / 2.0), label, STEEL);
    }
    widgets::focus_ring(ui, &response, rect);
    response
}

/// "37 / 100", with hours for the achievements of time.
fn progress_text(a: &Achievement, value: u64) -> String {
    if a.counts_seconds() {
        tf("{done} / {total} h", &[("done", &grouped(value / 3600)), ("total", &grouped(a.target / 3600))])
    } else {
        format!("{} / {}", grouped(value), grouped(a.target))
    }
}

/// Earned, and when; or how far along.
fn status(ui: &mut Ui, a: &Achievement, career: &Career) {
    if let Some(at) = career.achievements.get(a.id) {
        ui.label(RichText::new(tf("Earned {date}", &[("date", &super::career::utc_date(*at))])).font(strong(14.0)).color(badges::tier_color(a.tier)));
    } else if !a.hidden {
        let value = a.progress(career);
        ui.label(RichText::new(progress_text(a, value)).font(strong(14.0)).color(TEXT_BRIGHT));
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(360.0), 12.0), Sense::hover());
        super::paint_xp_track(ui, rect, value as f32 / a.target.max(1) as f32);
    }
}

/// The moment it was earned: the picture taken then, the battle and how far into it.
fn moment(ui: &mut Ui, a: &Achievement, record: &Record) {
    let unlock = record.career.unlocks.get(a.id);
    ui.label(widgets::caption("THE MOMENT"));
    if let Some(u) = unlock.filter(|u| !u.map.is_empty() && !u.earlier) {
        let minutes = u.battle_seconds / 60;
        let clock = format!("{}:{:02}", minutes, u.battle_seconds % 60);
        let mode = t(super::career::mode_label(u.mode));
        let line = if u.mode == MatchMode::Unknown { tf("On {map}, {clock} into the battle", &[("map", &u.map), ("clock", &clock)]) }
            else { tf("{mode} on {map}, {clock} into the battle", &[("mode", &mode), ("map", &u.map), ("clock", &clock)]) };
        ui.add(egui::Label::new(RichText::new(line).color(TEXT)).wrap());
    }
    let picture = record.folder.filter(|_| record.pictures.contains(a.id)).map(|dir| dir.join(format!("{}.jpg", a.id)));
    match picture.as_deref().map(|path| super::pictures::texture(ui.ctx(), path)) {
        Some(Some(texture)) => {
            let [w, h] = texture.size();
            let width = ui.available_width();
            let height = width * h as f32 / w.max(1) as f32;
            let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            ui.painter().image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, LINE_HI), StrokeKind::Inside);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, t("The battle the moment it was earned")));
        }
        Some(None) => {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ui.available_width() * 9.0 / 16.0), Sense::hover());
            ui.painter().rect_filled(rect, 2.0, BG_DEEP);
            let bar = Rect::from_center_size(rect.center(), vec2(rect.width() * 0.4, 10.0));
            crate::progress::paint(ui, bar, None, true);
        }
        None if unlock.is_some_and(|u| u.earlier) => quiet(ui, t("Earned from your service record: no picture of this moment.")),
        None => quiet(ui, t("No picture of this moment.")),
    }
}

fn quiet(ui: &mut Ui, text: &str) {
    widgets::card(ui, |ui| { ui.add(egui::Label::new(RichText::new(text).color(STEEL)).wrap()); });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_grouped_and_hours_shown_as_hours() {
        let hours = achievements::find("career_hours_10").unwrap();
        assert_eq!(progress_text(hours, 7200), "2 / 10 h");
        assert_eq!(progress_text(achievements::find("career_kills_1000").unwrap(), 1500), "1,500 / 1,000");
    }

    #[test]
    fn the_list_filters_searches_and_sorts() {
        let mut career = Career { kills: 400, ..Default::default() };
        career.achievements.insert("career_kills_1".into(), 10);
        career.achievements.insert("career_kills_100".into(), 20);
        let mut view = AchievementsView { status: Status::Earned, sort: Sort::Recent, ..Default::default() };
        assert_eq!(view.list(&career).iter().map(|a| a.id).collect::<Vec<_>>(), ["career_kills_100", "career_kills_1"]);
        view.status = Status::InProgress;
        view.category = Some(Category::Battle);
        view.sort = Sort::Nearest;
        assert_eq!(view.list(&career)[0].id, "career_kills_1000", "400 of 1,000 is the nearest");
        view.status = Status::All;
        view.category = None;
        view.query = "duel".into();
        assert!(view.list(&career).is_empty(), "a secret is not found by what it asks");
        view.query = "secret".into();
        assert!(view.list(&career).iter().all(|a| a.hidden));
    }
}
