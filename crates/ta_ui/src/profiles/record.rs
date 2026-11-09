//! What a battle did for the career, on its result's page: the rank (its insignia and title, its
//! bar filling from where the battle began to where it ends, through any promotion on the way),
//! the XP it earned by source, and the achievements earned in it.

use egui::{RichText, Sense, Ui, vec2};
use ta_config::profiles::{XpBreakdown, rank};

use crate::badges::{self, Look};
use crate::i18n::{grouped, n, t, tf};
use crate::theme::*;
use crate::widgets;

/// A battle's part in the career: the game keeps it up to date at each checkpoint.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BattleRecord {
    /// The career's XP of every kind when the battle began, and now.
    pub xp_before: u64,
    pub xp_now: u64,
    /// The battle's own XP, by source when it was followed from its beginning.
    pub battle_xp: u64,
    pub breakdown: Option<XpBreakdown>,
    /// The achievements earned in it, and their XP.
    pub achievements: Vec<String>,
    pub achievement_xp: u64,
    /// Cheats were used in it: it earned nothing, and what it had was taken back.
    pub voided: bool,
}

impl BattleRecord {
    /// Its XP by source, those that earned any: (label, XP).
    pub fn sources(&self) -> Vec<(&'static str, u64)> {
        let mut out = Vec::new();
        if let Some(b) = &self.breakdown {
            out.extend([(n("Kills"), b.combat), (n("Construction"), b.construction), (n("Captures"), b.captures),
                (n("Economy"), b.economy), (n("Time played"), b.time), (n("Result"), b.completion + b.victory)]);
        } else if self.battle_xp > 0 {
            out.push((n("Battle"), self.battle_xp));
        }
        out.push((n("Achievements"), self.achievement_xp));
        out.retain(|(_, xp)| *xp > 0);
        out
    }
}

/// Seconds the rank's bar takes to fill from the battle's start to its end, after a moment.
const FILL: f64 = 1.6;
const WAIT: f64 = 0.4;

/// How far a record's bar has filled (0 to 1) since it was first drawn under `id`, eased.
fn filled(ui: &Ui, id: egui::Id) -> (f64, f64) {
    let now = ui.input(|i| i.time);
    let start = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
    let still = crate::motion::still(ui.ctx());
    let t_fill = if still { 1.0 } else { ((now - start - WAIT) / FILL).clamp(0.0, 1.0) };
    if t_fill < 1.0 { ui.ctx().request_repaint(); }
    (t_fill, 1.0 - (1.0 - t_fill).powi(3))
}

/// The battle's record: drawn in a result's page (the full width given).
pub fn battle_record(ui: &mut Ui, record: &BattleRecord) {
    let (t_fill, ease) = filled(ui, ui.id().with("ta_ui::battle_record"));
    let gained = record.xp_now.saturating_sub(record.xp_before);
    let shown = record.xp_before + (gained as f64 * ease) as u64;
    let rank_shown = rank(shown);
    let (first, last) = (rank(record.xp_before), rank(record.xp_now));
    let promoted = (last.grade, last.stars) > (first.grade, first.stars);
    let wide = ui.available_width() >= 560.0;
    let rank_column = |ui: &mut Ui| {
        ui.horizontal_top(|ui| {
            badges::insignia(ui, rank_shown.grade, 60.0);
            ui.vertical(|ui| {
                ui.add(egui::Label::new(widgets::caption("SERVICE RECORD")).wrap());
                ui.add(egui::Label::new(RichText::new(super::rank_line(&rank_shown)).font(strong(17.0)).color(AMBER)).wrap());
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width().min(320.0), 12.0), Sense::hover());
                crate::progress::paint(ui, rect, Some(rank_shown.progress), false);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, super::progress_text(&last)));
                ui.add(egui::Label::new(RichText::new(super::progress_text(&rank_shown)).font(body(12.0)).color(STEEL)).wrap());
                if promoted && t_fill >= 1.0 {
                    ui.add(egui::Label::new(RichText::new(tf("Promoted: {rank}", &[("rank", &super::rank_line(&last))])).font(strong(13.0)).color(GREEN)).wrap());
                }
            });
        });
    };
    let xp_column = |ui: &mut Ui| {
        if record.voided {
            ui.add(egui::Label::new(RichText::new(t("No XP or achievements: cheats were used")).font(strong(17.0)).color(RED)).wrap());
            return;
        }
        let total = record.battle_xp + record.achievement_xp;
        ui.label(RichText::new(tf("+{xp} XP", &[("xp", &grouped((total as f64 * ease) as u64))])).font(strong(26.0)).color(AMBER_BRIGHT));
        ui.horizontal_wrapped(|ui| {
            for (label, xp) in record.sources() {
                widgets::tag(ui, &tf("{source} +{xp}", &[("source", &t(label)), ("xp", &grouped(xp))]), if label == "Achievements" { AMBER } else { STEEL });
            }
        });
    };
    if wide {
        ui.horizontal_top(|ui| {
            let half = ui.available_width() * 0.52;
            ui.vertical(|ui| { ui.set_width(half); rank_column(ui); });
            ui.vertical(|ui| xp_column(ui));
        });
    } else {
        rank_column(ui);
        xp_column(ui);
    }
    if !record.achievements.is_empty() {
        ui.add_space(4.0);
        ui.add(egui::Label::new(widgets::caption("ACHIEVEMENTS EARNED IN THIS BATTLE")).wrap());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(12.0, 6.0);
            for a in record.achievements.iter().filter_map(|id| ta_config::achievements::find(id)) {
                // (One piece each, badge and words, so that a row wraps between them.)
                let room = (ui.available_width() - 40.0).max(60.0);
                let name = widgets::wrapped_galley(ui, RichText::new(t(a.name)).font(strong(13.0)).color(TEXT_BRIGHT), room);
                let tier = widgets::galley(ui, RichText::new(t(badges::tier_label(a.tier))).font(strong(10.0)).color(badges::tier_color(a.tier)));
                let width = 40.0 + name.size().x.max(tier.size().x);
                let (rect, response) = ui.allocate_exact_size(vec2(width, 34.0_f32.max(name.size().y + tier.size().y)), Sense::hover());
                let painter = ui.painter();
                badges::paint(ui.ctx(), painter, egui::Rect::from_min_size(rect.min, vec2(34.0, 34.0)), a.index(), Look::Earned);
                painter.galley(rect.min + vec2(40.0, 0.0), name.clone(), TEXT_BRIGHT);
                painter.galley(rect.min + vec2(40.0, name.size().y), tier, badges::tier_color(a.tier));
                response.on_hover_text(t(a.description));
            }
        });
    }
}

/// The battle's record in a line, for a result's page on a short screen: the rank's insignia and
/// title, its bar filling as [`battle_record`]'s does, the XP the battle earned (by source on
/// hover) and the achievements it earned as badges (named on hover).
pub fn battle_record_line(ui: &mut Ui, record: &BattleRecord) {
    let (t_fill, ease) = filled(ui, ui.id().with("ta_ui::battle_record_line"));
    let gained = record.xp_now.saturating_sub(record.xp_before);
    let rank_shown = rank(record.xp_before + (gained as f64 * ease) as u64);
    let (first, last) = (rank(record.xp_before), rank(record.xp_now));
    let promoted = (last.grade, last.stars) > (first.grade, first.stars) && t_fill >= 1.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        badges::insignia(ui, rank_shown.grade, 34.0);
        // The rank and its bar, as wide as the line leaves them.
        let total = record.battle_xp + record.achievement_xp;
        let xp = if record.voided {
            widgets::galley(ui, RichText::new(t("No XP or achievements: cheats were used")).font(strong(14.0)).color(RED))
        } else {
            widgets::galley(ui, RichText::new(tf("+{xp} XP", &[("xp", &grouped((total as f64 * ease) as u64))])).font(strong(20.0)).color(AMBER_BRIGHT))
        };
        let earned: Vec<_> = record.achievements.iter().filter_map(|id| ta_config::achievements::find(id)).collect();
        // (What the insignia, the XP and the badges leave, with the gaps between them.)
        let gap = ui.spacing().item_spacing.x;
        let room = (ui.available_width() - 34.0 - xp.size().x - 2.0 * gap - earned.len() as f32 * (26.0 + gap)).max(100.0);
        ui.vertical(|ui| {
            ui.set_width(room.min(420.0));
            let line = if promoted { tf("Promoted: {rank}", &[("rank", &super::rank_line(&last))]) } else { super::rank_line(&rank_shown) };
            ui.add(egui::Label::new(RichText::new(line).font(strong(14.0)).color(if promoted { GREEN } else { AMBER })).truncate());
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
            crate::progress::paint(ui, rect, Some(rank_shown.progress), false);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, super::progress_text(&last)));
        });
        let (rect, response) = ui.allocate_exact_size(xp.size(), Sense::hover());
        ui.painter().galley(rect.min, xp, if record.voided { RED } else { AMBER_BRIGHT });
        let sources: Vec<String> = record.sources().into_iter().map(|(label, xp)| tf("{source} +{xp}", &[("source", &t(label)), ("xp", &grouped(xp))])).collect();
        if !sources.is_empty() {
            response.on_hover_text(sources.join("
"));
        }
        for a in earned {
            let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
            badges::paint(ui.ctx(), ui.painter(), rect, a.index(), Look::Earned);
            response.on_hover_text(format!("{}
{}", t(a.name), t(a.description)));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_list_what_earned_xp() {
        let record = BattleRecord { battle_xp: 575, achievement_xp: 250, breakdown: Some(XpBreakdown { combat: 400, time: 75, completion: 100, ..Default::default() }), ..Default::default() };
        assert_eq!(record.sources(), [("Kills", 400), ("Time played", 75), ("Result", 100), ("Achievements", 250)]);
        let resumed = BattleRecord { battle_xp: 120, ..Default::default() };
        assert_eq!(resumed.sources(), [("Battle", 120)]);
    }

    #[test]
    fn the_record_fits_narrow_pages() {
        use egui::Vec2;
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        crate::motion::set_reduced(&ctx, true);
        let record = BattleRecord { xp_before: 400, xp_now: 2600, battle_xp: 1700, achievement_xp: 500,
            breakdown: Some(XpBreakdown { combat: 900, construction: 500, time: 200, completion: 100, ..Default::default() }),
            achievements: vec!["career_kills_100".into(), "rank_2".into()], voided: false };
        for width in [300.0, 900.0] {
            let mut used = 0.0;
            let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 600.0))), ..Default::default() }, |ui| {
                battle_record(ui, &record);
                used = ui.min_rect().width();
            });
            output.textures_delta.clear();
            assert!(used <= width + 1.0, "{width}: {used}");
        }
        // And in a line.
        for width in [420.0, 900.0] {
            let mut used = Vec2::ZERO;
            let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 600.0))), ..Default::default() }, |ui| {
                battle_record_line(ui, &record);
                used = ui.min_rect().size();
            });
            output.textures_delta.clear();
            assert!(used.x <= width + 1.0 && used.y <= 48.0, "{width}: {used:?}");
        }
    }
}
