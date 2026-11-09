//! Signals to allies: a ping with a meaning (look, attack, defend, on my way, I need metal or
//! energy). Alt+click the ground pings it; Alt+drag opens a wheel of the meanings, the one
//! under the pointer given on release. Computer allies act on them and answer in the chat
//! ([`answer_text`]); people see them on the ground and the minimap, with a word under the
//! top bar, and F3 looks at the latest.

use egui::{Color32, Pos2, Stroke, Vec2};
use glam::Vec3;
use ta_sim::SignalKind;
use ta_ui::i18n::{n, t, tf};

/// A signal on the map: where, who, when (seconds of game time), and what they meant.
#[derive(Clone, Debug)]
pub struct Ping {
    pub at: Vec3,
    pub from: String,
    pub time: f32,
    pub kind: SignalKind,
}

impl Ping {
    /// Seconds it stays on the map: a plain ping briefly, a call for a while.
    pub fn life(&self) -> f32 {
        if self.kind == SignalKind::Look { 6.0 } else { 12.0 }
    }

    /// How far through its life it is (0 new, 1 gone), at `now` (seconds of game time).
    pub fn age(&self, now: f32) -> f32 {
        (now - self.time) / self.life()
    }
}

/// The wheel of meanings while Alt+drag holds it: where it opened (window pixels), the ground
/// it signals, and the meaning under the pointer (none: the plain ping).
#[derive(Clone, Copy, Debug)]
pub struct Wheel {
    pub centre: glam::Vec2,
    pub at: Vec3,
    pub kind: Option<SignalKind>,
}

/// The wheel's meanings, clockwise from the top.
const ORDER: [SignalKind; 6] = [SignalKind::Attack, SignalKind::OnMyWay, SignalKind::NeedEnergy, SignalKind::Defend, SignalKind::NeedMetal, SignalKind::Look];
/// How far the pointer goes (pixels) before a meaning is picked, and the wheel's size.
const DEAD: f32 = 22.0;
const RADIUS: f32 = 86.0;

/// The meaning under the pointer, `offset` from the wheel's centre (window pixels).
pub fn pick(offset: glam::Vec2) -> Option<SignalKind> {
    if offset.length() < DEAD {
        return None;
    }
    // (Clockwise from the top: the screen's y grows downwards.)
    let angle = offset.x.atan2(-offset.y).rem_euclid(std::f32::consts::TAU);
    let sector = ((angle + std::f32::consts::TAU / 12.0) / (std::f32::consts::TAU / 6.0)) as usize % 6;
    Some(ORDER[sector])
}

/// A meaning's word on the map and the wheel.
pub fn label(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Look => n("LOOK"),
        SignalKind::Attack => n("ATTACK"),
        SignalKind::Defend => n("DEFEND"),
        SignalKind::OnMyWay => n("ON MY WAY"),
        SignalKind::NeedMetal => n("NEED METAL"),
        SignalKind::NeedEnergy => n("NEED ENERGY"),
    }
}

/// A meaning's colour (`alpha` 0-255).
pub fn colour(kind: SignalKind, alpha: u8) -> Color32 {
    let (r, g, b) = match kind {
        SignalKind::Look => (120, 255, 255),
        SignalKind::Attack => (255, 90, 70),
        SignalKind::Defend => (110, 170, 255),
        SignalKind::OnMyWay => (120, 230, 120),
        SignalKind::NeedMetal => (200, 210, 225),
        SignalKind::NeedEnergy => (255, 215, 90),
    };
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

/// What a signal says under the top bar, from `who`.
pub fn notice(kind: SignalKind, who: &str) -> String {
    match kind {
        SignalKind::Look => tf("{player} pinged the map", &[("player", &who)]),
        SignalKind::Attack => tf("{player}: attack here!", &[("player", &who)]),
        SignalKind::Defend => tf("{player}: defend here!", &[("player", &who)]),
        SignalKind::OnMyWay => tf("{player} is on the way", &[("player", &who)]),
        SignalKind::NeedMetal => tf("{player} needs metal", &[("player", &who)]),
        SignalKind::NeedEnergy => tf("{player} needs energy", &[("player", &who)]),
    }
}

/// A computer ally's answer, in its own words.
pub fn answer_text(answer: ta_ai::Answer) -> String {
    use ta_ai::Answer as A;
    match answer {
        A::Defending { .. } => t("Sending units to defend there.").to_string(),
        A::CannotDefend { .. } => t("I have nothing to send right now.").to_string(),
        A::CannotReach { .. } => t("My units cannot get there.").to_string(),
        A::WillAttack { .. } => t("Understood: my next attack goes there.").to_string(),
        A::Attacking { .. } => t("Attacking there now!").to_string(),
        A::Sent { metal, .. } if metal > 0 => tf("Sending you {n} metal.", &[("n", &metal)]),
        A::Sent { energy, .. } => tf("Sending you {n} energy.", &[("n", &energy)]),
        A::NothingToSpare { .. } => t("I have nothing to spare right now.").to_string(),
    }
}

/// Who an answer is for.
pub fn answer_to(answer: ta_ai::Answer) -> u8 {
    use ta_ai::Answer as A;
    match answer {
        A::Defending { to } | A::CannotDefend { to } | A::CannotReach { to } | A::WillAttack { to } | A::Attacking { to } | A::Sent { to, .. } | A::NothingToSpare { to } => to,
    }
}

/// A meaning's number on the wire (multiplayer pings between people), and back.
pub fn index(kind: SignalKind) -> u8 {
    SignalKind::ALL.iter().position(|k| *k == kind).unwrap_or(0) as u8
}

pub fn of_index(i: u8) -> Option<SignalKind> {
    SignalKind::ALL.get(i as usize).copied()
}

/// The wheel while it is held: the meanings round it, the one under the pointer lit, its word
/// in the middle.
pub fn draw_wheel(ctx: &egui::Context, wheel: &Wheel) {
    let ppp = ctx.pixels_per_point();
    let centre = Pos2::new(wheel.centre.x / ppp, wheel.centre.y / ppp);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("signal_wheel")));
    painter.circle_filled(centre, RADIUS + 8.0, Color32::from_black_alpha(150));
    painter.circle_stroke(centre, RADIUS + 8.0, Stroke::new(1.0, Color32::from_white_alpha(40)));
    for (i, kind) in ORDER.iter().enumerate() {
        let angle = i as f32 * std::f32::consts::TAU / 6.0;
        let dir = Vec2::new(angle.sin(), -angle.cos());
        let lit = wheel.kind == Some(*kind);
        let at = centre + dir * (RADIUS * 0.66);
        if lit {
            painter.circle_filled(at, 30.0, colour(*kind, 70));
        }
        painter.circle_filled(at, 7.0, colour(*kind, if lit { 255 } else { 170 }));
        painter.text(at + Vec2::new(0.0, 10.0), egui::Align2::CENTER_TOP, t(label(*kind)), crate::theme::strong(11.0), colour(*kind, if lit { 255 } else { 190 }));
    }
    painter.circle_filled(centre, 4.0, Color32::from_white_alpha(200));
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_picks_a_meaning_by_direction() {
        assert_eq!(pick(glam::Vec2::new(0.0, -5.0)), None, "near the centre: the plain ping");
        assert_eq!(pick(glam::Vec2::new(0.0, -60.0)), Some(SignalKind::Attack), "up");
        assert_eq!(pick(glam::Vec2::new(52.0, -30.0)), Some(SignalKind::OnMyWay), "up and right");
        assert_eq!(pick(glam::Vec2::new(0.0, 60.0)), Some(SignalKind::Defend), "down");
        assert_eq!(pick(glam::Vec2::new(-52.0, -30.0)), Some(SignalKind::Look), "up and left");
        for kind in SignalKind::ALL {
            assert_eq!(of_index(index(kind)), Some(kind), "each meaning's number on the wire reads back");
        }
    }
}
