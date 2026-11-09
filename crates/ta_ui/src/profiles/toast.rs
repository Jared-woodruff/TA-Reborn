//! The career's notices: an achievement earned, a card that slides in at the top right of the
//! view with the badge (a shine passing over it), the tier, its name and what it asked; and a
//! promotion, with the new rank's insignia. One at a time, each for a few seconds. Achievements
//! earned from the service record (met by play before they were in the list) come together in
//! one notice.

use std::collections::VecDeque;

use egui::{Color32, Context, Id, Order, Rect, RichText, Shape, Stroke, StrokeKind, pos2, vec2};
use ta_config::achievements::{self, Achievement};
use ta_config::profiles::Rank;

use crate::badges::{self, Look};
use crate::i18n::{t, tf};
use crate::theme::*;

/// Seconds a notice stays, and slides in and out.
const SHOW: f64 = 5.5;
const SLIDE: f64 = 0.35;
const BADGE: f32 = 72.0;

#[derive(Clone)]
enum Notice {
    One { achievement: &'static Achievement, earlier: bool },
    Earlier(Vec<&'static Achievement>),
    Promotion(Rank),
}

#[derive(Default)]
pub struct CareerNotices {
    queue: VecDeque<Notice>,
    /// The notice shown, and when it began (once drawn).
    current: Option<(Notice, Option<f64>)>,
}

impl CareerNotices {
    /// Queues the notice of a promotion to `rank` (a new title, or a service star).
    pub fn push_promotion(&mut self, rank: Rank) {
        self.queue.push_back(Notice::Promotion(rank));
    }

    /// Queues the notices of an award: `unlocked` in turn; those of them also in `earlier`
    /// (met by earlier play) together, after them.
    pub fn push(&mut self, unlocked: &[String], earlier: &[String]) {
        let found = |id: &String| achievements::find(id);
        for achievement in unlocked.iter().filter(|id| !earlier.contains(id)).filter_map(found) {
            self.queue.push_back(Notice::One { achievement, earlier: false });
        }
        let old: Vec<_> = unlocked.iter().filter(|id| earlier.contains(id)).filter_map(found).collect();
        match old.len() {
            0 => {}
            1 | 2 => self.queue.extend(old.into_iter().map(|achievement| Notice::One { achievement, earlier: true })),
            _ => self.queue.push_back(Notice::Earlier(old)),
        }
    }

    pub fn is_empty(&self) -> bool { self.queue.is_empty() && self.current.is_none() }

    /// Draws the notice of the moment at the top right of `room` (the view clear of the
    /// interface's bars), and moves on to the next when its time is up.
    pub fn show(&mut self, ctx: &Context, room: Rect) {
        if self.current.is_none() {
            self.current = self.queue.pop_front().map(|n| (n, None));
        }
        let Some((notice, started)) = &mut self.current else { return };
        let now = ctx.input(|i| i.time);
        let start = *started.get_or_insert_with(|| {
            crate::sound::play(ctx, crate::sound::UiSound::Achievement);
            now
        });
        let age = now - start;
        if age >= SHOW {
            self.current = None;
            ctx.request_repaint();
            return;
        }
        let still = crate::motion::still(ctx);
        // In with an easing, out with a fade (held still: a fade only).
        let enter = ((age / SLIDE).min(1.0)) as f32;
        let leave = (((SHOW - age) / SLIDE).min(1.0)) as f32;
        let ease = 1.0 - (1.0 - enter).powi(3);
        let opacity = if still { enter.min(leave) } else { leave };
        let width = 380.0_f32.min(room.width() - 32.0).max(220.0);
        let notice = notice.clone();
        let slide = if still { 0.0 } else { (1.0 - ease) * (width + 40.0) };
        let at = pos2(room.max.x - 16.0 - width + slide, room.min.y + 16.0);
        egui::Area::new(Id::new("ta_ui::achievement_toast")).order(Order::Foreground).fixed_pos(at).interactable(false).show(ctx, |ui| {
            ui.set_width(width);
            ui.multiply_opacity(opacity);
            paint(ui, &notice, width, age as f32, still);
        });
        ctx.request_repaint();
    }
}

fn paint(ui: &mut egui::Ui, notice: &Notice, width: f32, age: f32, still: bool) {
    let (tier, caption, title, detail, row) = match notice {
        Notice::One { achievement, earlier } => (
            achievement.tier,
            if *earlier { t("FROM YOUR SERVICE RECORD") } else { t("ACHIEVEMENT UNLOCKED") },
            t(achievement.name).to_string(),
            t(achievement.description).to_string(),
            Vec::new(),
        ),
        Notice::Earlier(list) => (
            list.iter().map(|a| a.tier).max().unwrap_or(achievements::Tier::Bronze),
            t("FROM YOUR SERVICE RECORD"),
            tf("{count} achievements earned", &[("count", &list.len())]),
            String::new(),
            list.clone(),
        ),
        Notice::Promotion(rank) => (
            achievements::Tier::Gold,
            t("PROMOTED"),
            super::rank_line(rank),
            super::next_rank_text(rank),
            Vec::new(),
        ),
    };
    let promotion = matches!(notice, Notice::Promotion(_));
    let accent = if promotion { AMBER_BRIGHT } else { badges::tier_color(tier) };
    let pad = 12.0;
    let text_x = pad + BADGE + 12.0;
    let text_w = width - text_x - pad;
    // "ACHIEVEMENT UNLOCKED · GOLD", the tier in its metal (wrapping where a language is long).
    let mut job = egui::text::LayoutJob::default();
    let format = |color: Color32| egui::TextFormat { font_id: strong(11.0), color, extra_letter_spacing: crate::i18n::tracking(1.2), ..Default::default() };
    job.append(caption, 0.0, format(AMBER));
    if !promotion {
        job.append(" · ", 0.0, format(STEEL));
        job.append(t(badges::tier_label(tier)), 0.0, format(accent));
    }
    job.wrap.max_width = text_w;
    let caption = ui.painter().layout_job(job);
    let title = crate::widgets::wrapped_galley(ui, RichText::new(title).font(strong(17.0)).color(TEXT_BRIGHT), text_w);
    let detail = (!detail.is_empty()).then(|| crate::widgets::wrapped_galley(ui, RichText::new(detail).font(body(12.5)).color(STEEL), text_w));
    let small = 26.0;
    let text_h = caption.size().y + 3.0 + title.size().y + detail.as_ref().map_or(0.0, |d| 3.0 + d.size().y) + if row.is_empty() { 0.0 } else { 6.0 + small };
    let height = (BADGE + 2.0 * pad).max(text_h + 2.0 * pad);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    let painter = ui.painter();
    // A plate with a shadow under it and the tier's colour down its edge.
    painter.add(Shape::rect_filled(rect.translate(vec2(0.0, 4.0)).expand(2.0), 8.0, Color32::from_black_alpha(90)));
    painter.add(crate::metal::plate(ui.ctx(), rect, PANEL));
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, accent.gamma_multiply(0.55)), StrokeKind::Inside);
    painter.rect_filled(Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(3.0, rect.height() - 12.0)), 1.5, accent);
    let badge = Rect::from_min_size(pos2(rect.min.x + pad, rect.center().y - BADGE / 2.0), vec2(BADGE, BADGE));
    // A glow behind the badge as it arrives.
    let glow = (1.0 - (age / 1.2).min(1.0)).max(0.0);
    if glow > 0.0 && !still { crate::widgets::soft_glow(painter, badge.center(), BADGE * 0.3, BADGE * 0.75, accent.gamma_multiply(0.5 * glow)); }
    match notice {
        Notice::Promotion(rank) => {
            badges::paint_insignia(ui.ctx(), painter, badge, rank.grade);
            if !still { shine(painter, badge, age); }
        }
        _ => {
            let first = match notice { Notice::One { achievement, .. } => Some(*achievement), Notice::Earlier(list) => list.first().copied(), Notice::Promotion(_) => None };
            if let Some(achievement) = first {
                badges::paint(ui.ctx(), painter, badge, achievement.index(), Look::Earned);
                if !still { shine(painter, badge, age); }
            }
        }
    }
    let mut y = rect.min.y + pad;
    let x = rect.min.x + text_x;
    painter.galley(pos2(x, y), caption.clone(), AMBER);
    y += caption.size().y + 3.0;
    painter.galley(pos2(x, y), title.clone(), TEXT_BRIGHT);
    y += title.size().y;
    if let Some(detail) = detail {
        y += 3.0;
        painter.galley(pos2(x, y), detail, STEEL);
    }
    if !row.is_empty() {
        y += 6.0;
        for (i, achievement) in row.iter().take(((text_w + 4.0) / (small + 4.0)) as usize).enumerate() {
            let r = Rect::from_min_size(pos2(x + i as f32 * (small + 4.0), y), vec2(small, small));
            badges::paint(ui.ctx(), painter, r, achievement.index(), Look::Earned);
        }
    }
}

/// A streak of light across the medallion, once, as the notice arrives.
fn shine(painter: &egui::Painter, badge: Rect, age: f32) {
    let t = ((age - 0.25) / 0.8).clamp(0.0, 1.0);
    if t <= 0.0 || t >= 1.0 { return; }
    let c = badge.center();
    let r = badge.width() * 0.39;
    let d = std::f32::consts::FRAC_1_SQRT_2;
    let (u, v) = (vec2(d, d), vec2(-d, d));
    let centre = -r + t * 2.0 * r;
    let lines = 9;
    let band = r * 0.42;
    for k in 0..lines {
        let s = centre - band / 2.0 + band * k as f32 / (lines - 1) as f32;
        if s.abs() >= r { continue; }
        let half = (r * r - s * s).sqrt();
        let falloff = 1.0 - ((k as f32 / (lines - 1) as f32) * 2.0 - 1.0).abs();
        let alpha = (70.0 * falloff * (1.0 - (t * 2.0 - 1.0).abs().powi(2))) as u8;
        let p = c + u * s;
        painter.line_segment([p - v * half, p + v * half], Stroke::new(band / lines as f32 + 0.6, Color32::from_white_alpha(alpha)));
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use egui::Pos2;

    #[test]
    fn notices_come_one_at_a_time_and_earlier_ones_together() {
        let mut toasts = CareerNotices::default();
        let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        toasts.push(&ids(&["career_kills_1", "rank_2", "career_built_1", "career_buildings_1", "matches_10"]),
            &ids(&["rank_2", "career_built_1", "career_buildings_1"]));
        assert_eq!(toasts.queue.len(), 3, "two of this moment, then the earlier three together");
        assert!(matches!(toasts.queue[2], Notice::Earlier(ref list) if list.len() == 3));
        toasts.push_promotion(ta_config::profiles::rank(ta_config::profiles::rank_threshold(5)));
        assert!(matches!(toasts.queue[3], Notice::Promotion(ref rank) if rank.title == "Private"));
        let ctx = Context::default();
        crate::theme::install(&ctx);
        crate::motion::set_reduced(&ctx, true);
        let room = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let frame = |time: f64, toasts: &mut CareerNotices| {
            let mut output = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(room), ..Default::default() }, |ui| toasts.show(ui.ctx(), room));
            output.textures_delta.clear();
        };
        frame(0.0, &mut toasts);
        assert_eq!(toasts.queue.len(), 3);
        frame(SHOW + 0.1, &mut toasts);
        frame(SHOW + 0.2, &mut toasts);
        assert_eq!(toasts.queue.len(), 2, "the second follows when the first has gone");
        for k in 3..12 { frame(k as f64 * SHOW, &mut toasts); frame(k as f64 * SHOW + 0.1, &mut toasts); }
        assert!(toasts.is_empty());
    }
}
