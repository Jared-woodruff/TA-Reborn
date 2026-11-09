//! The head of a service record: the commander's picture and name, their motto, allegiance and
//! time served, the rank's insignia and its bar, and the achievements they show (the showcase).

use egui::{RichText, Sense, Stroke, Ui, vec2};
use ta_config::achievements::{self, Achievement};
use ta_config::profiles::{Profile, Rank};

use super::career::{CareerData, duration, utc_date};
use crate::badges::{self, Look};
use crate::i18n::{grouped, t, tf};
use crate::theme::*;
use crate::widgets;

/// How much room the head has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeroSize {
    /// A short dialog: the picture, the name, the rank and its bar.
    Compact,
    /// A dialog, or the launcher's page in a window of its usual size.
    Normal,
    /// A page with room to spare.
    Large,
}

impl HeroSize {
    /// The size for a dialog this high.
    pub fn for_dialog(height: f32) -> Self {
        if height < 480.0 { Self::Compact } else { Self::Normal }
    }
}

/// Draws the head. Returns the showcased achievement clicked, if any.
pub fn hero(ui: &mut Ui, profile: &Profile, data: &CareerData, size: HeroSize) -> Option<&'static str> {
    let career = &data.career;
    let width = ui.available_width();
    let (large, compact) = (size == HeroSize::Large, size == HeroSize::Compact);
    let texture = data.avatar_path.as_ref().filter(|_| profile.has_avatar).and_then(|p| crate::portrait::texture_at(ui.ctx(), p));
    let picture = match size {
        HeroSize::Large => if width < 520.0 { 72.0 } else { 112.0 },
        HeroSize::Normal => if width < 420.0 { 54.0 } else { 80.0 },
        HeroSize::Compact => 48.0,
    };
    let insignia = if width < 360.0 { 0.0 } else {
        match size {
            HeroSize::Large => if width < 520.0 { 64.0 } else { 96.0 },
            HeroSize::Normal => if width < 420.0 { 54.0 } else { 72.0 },
            HeroSize::Compact => 48.0,
        }
    };
    let shown = showcased(profile, data);
    // (A wide head shows the showcase beside the name, a narrower one under the bar.)
    let badge = if large { 52.0 } else { 40.0 };
    let beside = !compact && !shown.is_empty() && width >= 760.0;
    let rail = if beside { shown.len().max(3) as f32 * (badge + 10.0) } else { 0.0 };
    // (Wide, the rank's bar shares its row with what it counts and the rank to come.)
    let one_row = !compact && width >= 640.0;
    let spacing = ui.spacing().item_spacing.y;
    if compact { ui.spacing_mut().item_spacing.y = 4.0; }
    let rank = career.rank();
    let mut clicked = None;
    ui.horizontal_top(|ui| {
        super::avatar_picture(ui, texture.as_ref(), Some(profile.allegiance), picture);
        ui.vertical(|ui| {
            ui.set_width((width - picture - insignia - rail - if beside { 40.0 } else { 24.0 }).max(60.0));
            let name = match size { HeroSize::Large => display(30.0), HeroSize::Normal => display(24.0), HeroSize::Compact => strong(18.0) };
            ui.add(egui::Label::new(RichText::new(&profile.name).font(name).color(TEXT_BRIGHT)).wrap());
            if !profile.motto.is_empty() && !compact {
                ui.add(egui::Label::new(RichText::new(format!("\u{201c}{}\u{201d}", profile.motto)).font(body(if large { 15.0 } else { 13.5 })).italics().color(CREAM)).wrap());
            }
            ui.add(egui::Label::new(RichText::new(super::rank_text(career)).font(strong(if compact { 12.0 } else if large { 17.0 } else { 15.0 })).color(AMBER)).wrap());
            let mut facts = vec![tf("{faction} allegiance", &[("faction", &profile.allegiance.label())])];
            if profile.created_at > 0 && !compact { facts.push(tf("Serving since {date}", &[("date", &utc_date(profile.created_at))])); }
            if career.played_seconds > 0 { facts.push(tf("{time} played", &[("time", &duration(career.played_seconds))])); }
            if career.achievement_xp() > 0 {
                facts.push(tf("{xp} career XP, {achievements} from achievements", &[("xp", &grouped(career.total_xp())), ("achievements", &grouped(career.achievement_xp()))]));
            } else {
                facts.push(tf("{xp} career XP", &[("xp", &grouped(career.total_xp()))]));
            }
            ui.add(egui::Label::new(RichText::new(facts.join(" \u{b7} ")).color(STEEL).size(12.0)).wrap());
            if !compact && !one_row { ui.add(egui::Label::new(RichText::new(super::next_rank_text(&rank)).color(STEEL).size(12.0)).wrap()); }
        });
        if beside {
            ui.vertical(|ui| {
                ui.set_width(rail);
                ui.add_space(4.0);
                ui.label(widgets::caption("SHOWCASE"));
                ui.horizontal(|ui| clicked = badges_row(ui, &shown, badge));
            });
        }
        if insignia > 0.0 { super::rank_insignia(ui, career, insignia); }
    });
    if one_row { progress_row(ui, &rank); } else { super::xp_progress(ui, career); }
    if !beside && !compact {
        if !shown.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(widgets::caption("SHOWCASE"));
                clicked = badges_row(ui, &shown, badge);
            });
        } else if achievements::ALL.iter().any(|a| a.is_unlocked(career)) {
            ui.add(egui::Label::new(RichText::new(t("Show up to five achievements here: open one you have earned and choose SHOW ON YOUR PROFILE."))
                .color(STEEL).size(12.0)).wrap());
        }
    }
    ui.spacing_mut().item_spacing.y = spacing;
    clicked
}

/// The rank's bar in one row: how far into the level, the bar, and the rank to come.
fn progress_row(ui: &mut Ui, rank: &Rank) {
    let value = super::progress_text(rank);
    let next = super::next_rank_text(rank);
    ui.horizontal(|ui| {
        let caption = if rank.xp_for_next_level.is_some() { t("RANK PROGRESS") } else { t("SERVICE STARS") };
        ui.label(RichText::new(caption).font(strong(11.0)).color(STEEL));
        ui.label(RichText::new(&value).font(strong(13.0)).color(TEXT_BRIGHT));
        let after = widgets::galley(ui, RichText::new(&next).size(12.0)).size().x;
        let bar = (ui.available_width() - after - 2.0 * ui.spacing().item_spacing.x).max(60.0);
        let (rect, response) = ui.allocate_exact_size(vec2(bar, 15.0), Sense::hover());
        super::paint_xp_track(ui, rect, rank.progress);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, value.clone()));
        ui.label(RichText::new(&next).color(STEEL).size(12.0));
    });
}

/// The achievements the profile shows (those it has earned).
fn showcased(profile: &Profile, data: &CareerData) -> Vec<&'static Achievement> {
    profile.showcase.iter().filter_map(|id| achievements::find(id)).filter(|a| a.is_unlocked(&data.career)).collect()
}

/// The showcase's badges, each a click away from its page.
fn badges_row(ui: &mut Ui, shown: &[&'static Achievement], size: f32) -> Option<&'static str> {
    let mut clicked = None;
    ui.spacing_mut().item_spacing = vec2(10.0, 6.0);
    for a in shown {
        let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t(a.name)));
        widgets::reveal_focus(ui, &response);
        if response.hovered() || response.has_focus() {
            ui.painter().circle_stroke(rect.center(), size * 0.5, Stroke::new(1.5, AMBER_BRIGHT));
        }
        badges::paint(ui.ctx(), ui.painter(), rect, a.index(), Look::Earned);
        let response = response.on_hover_ui(|ui| {
            ui.label(RichText::new(t(a.name)).font(strong(13.0)).color(badges::tier_color(a.tier)));
            ui.label(RichText::new(t(a.description)).color(STEEL));
        });
        if response.clicked() { clicked = Some(a.id); }
    }
    clicked
}
