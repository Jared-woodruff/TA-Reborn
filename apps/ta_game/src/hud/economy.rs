//! The economy in the modern HUD's top bar: metal and energy, each a gauge of what is stored
//! with the amount in it, what comes in and what is spent, a warning when the store runs dry or
//! overflows and, where the bar has the room, the last three minutes as a sparkline. A click on
//! either opens the economy's panel under the bar: the charts large, what is being built, and
//! the battle's tally (see [`Hud::economy_panel`]).

use egui::{Align2, Color32, CornerRadius, Pos2, Rect, RichText, Stroke, Vec2 as EVec2};
use ta_sim::Fx;
use ta_ui::i18n::{n, t, tf};
use ta_ui::metal;
use ta_vfs::Vfs;

use super::{ENERGY, Hud, METAL, Store, StoreState};
use crate::game::Game;
use crate::hud_skin;
use crate::theme::*;

/// How much of the top bar there is room for, by the width the left end has (points): what it
/// leaves out as the room shortens, the least important first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Tier {
    /// The clock, the idle counters in one, no sparklines, the least gauges.
    Tight,
    /// Everything but the wind.
    Narrow,
    /// Everything but the sparklines.
    Medium,
    /// Everything.
    Wide,
}

impl Tier {
    pub(super) fn of(width: f32) -> Tier {
        if width >= 1400.0 {
            Tier::Wide
        } else if width >= 1240.0 {
            Tier::Medium
        } else if width >= 1000.0 {
            Tier::Narrow
        } else {
            Tier::Tight
        }
    }

    /// How wide a store's gauge is drawn.
    fn gauge(self) -> f32 {
        match self {
            Tier::Wide => 124.0,
            Tier::Medium => 104.0,
            Tier::Narrow => 84.0,
            Tier::Tight => 64.0,
        }
    }
}

/// A resource's last minutes for its sparkline: the readings of `EconomyTrend::readings`, the
/// time they span, and which resource (0 metal, 2 energy: the income's index).
struct Spark<'a> {
    readings: &'a [(f32, [f32; 4])],
    span: f32,
    index: usize,
}

/// A resource: its caption, its store as a gauge with the amount in it, what comes in (green)
/// and what is spent (red when it is more), a warning when it runs dry or overflows, and the
/// sparkline when `spark` is. Returns where it stands.
fn readout(ui: &mut egui::Ui, name: &str, s: &Store, color: Color32, spark: Option<&Spark>, tier: Tier, time: f64) -> Rect {
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(ta_ui::widgets::caption(name).color(color));
        let frac = if s.capacity > 0.0 { (s.amount / s.capacity).clamp(0.0, 1.0) } else { 0.0 };
        let (rect, _) = ui.allocate_exact_size(EVec2::new(tier.gauge(), 17.0), egui::Sense::hover());
        // Recessed, marked in quarters, the amount in it; a stalling store's bar glows red.
        ui.painter().add(hud_skin::gauge(rect, frac, color, 4));
        if s.state == StoreState::Stalling {
            let pulse = (0.55 + 0.45 * (time * 5.0).sin()) as f32;
            ui.painter().rect_stroke(rect.expand(1.5), CornerRadius::same(2), Stroke::new(1.5, RED.gamma_multiply(pulse)), egui::StrokeKind::Outside);
            ui.ctx().request_repaint();
        }
        let amount = ta_ui::widgets::galley(ui, RichText::new(format!("{:.0}", s.amount)).font(strong(12.5)));
        metal::slab_text(ui.painter(), rect.center(), amount, true, TEXT_BRIGHT, false);
        // What comes in, and what goes out: the one that wins in its colour.
        let short = s.income < s.wanted - 0.05;
        ui.label(RichText::new(format!("+{:.1}", s.income)).font(strong(12.0)).color(if s.income > 0.05 { GREEN } else { STEEL }));
        let spend = match s.state {
            StoreState::Stalling => RED,
            StoreState::Draining(secs) if secs < 20 => RED,
            _ if short => AMBER_BRIGHT,
            _ => STEEL,
        };
        ui.label(RichText::new(if s.wanted > 0.05 { format!("-{:.1}", s.wanted) } else { "0.0".to_string() }).font(strong(12.0)).color(spend));
        match s.state {
            StoreState::Stalling => {
                ta_ui::widgets::tag(ui, n("STALLING"), RED);
            }
            StoreState::Full if tier >= Tier::Medium => {
                ta_ui::widgets::tag(ui, n("FULL"), STEEL);
            }
            StoreState::Draining(secs) if secs < 20 => {
                ta_ui::widgets::tag(ui, &tf("EMPTY IN {secs} S", &[("secs", &secs)]), RED);
            }
            _ => {}
        }
        if let Some(spark) = spark {
            let (rect, _) = ui.allocate_exact_size(EVec2::new(64.0, 17.0), egui::Sense::hover());
            sparkline(ui.painter(), rect, spark, color);
        }
    });
    row.response.rect
}

/// The income (filled) and the spending (a red line) over the last three minutes, small, in a
/// recessed well: what the economy's chart shows, to see at a glance which way it is going.
fn sparkline(painter: &egui::Painter, rect: Rect, spark: &Spark, color: Color32) {
    painter.add(metal::well(rect, INSET));
    let inner = rect.shrink(2.5);
    let (i, o) = (spark.index, spark.index + 1);
    let top = spark.readings.iter().map(|(_, s)| s[i].max(s[o])).fold(1.0f32, f32::max) * 1.15;
    let x = |t: f32| inner.left() + inner.width() * t / spark.span.max(1.0);
    let y = |v: f32| inner.bottom() - inner.height() * (v / top).clamp(0.0, 1.0);
    let made: Vec<Pos2> = spark.readings.iter().map(|(t, s)| Pos2::new(x(*t), y(s[i]))).collect();
    let mut fill = egui::Mesh::default();
    let shade = color.gamma_multiply(0.25);
    for (k, p) in made.iter().enumerate() {
        fill.colored_vertex(*p, shade);
        fill.colored_vertex(Pos2::new(p.x, inner.bottom()), shade);
        if k > 0 {
            let v = 2 * k as u32;
            fill.add_triangle(v - 2, v - 1, v);
            fill.add_triangle(v - 1, v + 1, v);
        }
    }
    painter.add(egui::Shape::mesh(fill));
    painter.add(egui::Shape::line(made, Stroke::new(1.2, color)));
    let spent: Vec<Pos2> = spark.readings.iter().map(|(t, s)| Pos2::new(x(*t), y(s[o]))).collect();
    painter.add(egui::Shape::line(spent, Stroke::new(1.0, RED)));
}

impl Hud {
    /// The top bar's economy: metal and energy side by side, a click on either opening the
    /// economy's panel. Returns where they stand together.
    pub(super) fn economy_readout(&mut self, ui: &mut egui::Ui, game: &Game, tier: Tier, time: f64) {
        let p = &game.sim.players[game.hud_player() as usize];
        let e = &p.economy;
        let rationed = e.build_factor < Fx::from_ratio(97, 100);
        let (readings, span) = if tier == Tier::Wide { self.trend.readings() } else { (Vec::new(), 1.0) };
        let spark = |index| (tier == Tier::Wide).then_some(Spark { readings: &readings, span, index });
        let gap = if tier == Tier::Tight { 12.0 } else { 20.0 };
        let metal_store = Store::of(e.metal, e.metal_capacity, e.metal_income, e.metal_expense, e.metal_wanted, rationed);
        let energy_store = Store::of(e.energy, e.energy_capacity, e.energy_income, e.energy_expense, e.energy_wanted, rationed);
        let metal = readout(ui, n("METAL"), &metal_store, METAL, spark(0).as_ref(), tier, time);
        ui.add_space(gap);
        let energy = readout(ui, n("ENERGY"), &energy_store, ENERGY, spark(2).as_ref(), tier, time);
        self.economy_rect = Some(metal.union(energy));
        for (rect, name, store) in [(metal, "metal", &metal_store), (energy, "energy", &energy_store)] {
            let resp = ui.interact(rect, egui::Id::new(("economy_readout", name)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                self.economy_open = !self.economy_open;
                ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
            }
            resp.on_hover_text(format!("{}\n{}", store.tip(), t("Click for the economy's charts")));
        }
    }

    /// The economy's panel under the top bar, while it is open (a click on the readout opens and
    /// closes it, a click anywhere else closes it): the charts of metal and energy over the last
    /// three minutes, what is being built, and the battle's tally.
    pub(super) fn economy_panel(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &mut Game) {
        let Some(readout) = self.economy_rect.filter(|_| self.economy_open) else { return };
        let screen = ctx.content_rect();
        // (Taller with frames under construction to list.)
        let owner = game.hud_player();
        let building = game.sim.units().any(|u| u.owner == owner && !u.dead && !u.is_complete());
        let size = EVec2::new(392.0, if building { 262.0 } else { 196.0 }).min(screen.size() - EVec2::splat(16.0));
        let at = Pos2::new(readout.left().clamp(screen.left() + 8.0, (screen.right() - size.x - 8.0).max(screen.left())), self.top_bar_bottom.max(34.0) + 4.0);
        let rect = Rect::from_min_size(at, size);
        // A click elsewhere (a press outside it and the readout) closes it.
        let outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.pointer_interact_pos().is_some_and(|p| !rect.contains(p) && !readout.expand(2.0).contains(p));
        if outside {
            self.economy_open = false;
            return;
        }
        let back = self.skin.deck(ctx, rect, true);
        egui::Area::new(egui::Id::new("economy_panel")).fixed_pos(rect.min).movable(false).constrain(false).order(egui::Order::Foreground).show(ctx, |ui| {
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            ui.painter().add(back);
            let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(8.0)).layout(egui::Layout::top_down(egui::Align::Min)));
            inner.set_clip_rect(rect.shrink(2.0));
            let ui = &mut inner;
            super::deck::section(ui, "ECONOMY");
            self.economy_contents(ui, ctx, vfs, game);
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    /// The panel's charts, and under them the battle's tally and what is being built.
    fn economy_contents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, vfs: &Vfs, game: &mut Game) {
        self.trend_chart(ui, n("METAL"), METAL, 0, 58.0);
        ui.add_space(4.0);
        self.trend_chart(ui, n("ENERGY"), ENERGY, 2, 58.0);
        ui.add_space(4.0);
        let owner = game.hud_player();
        if let Some(p) = game.sim.players.get(owner as usize) {
            let (rect, _) = ui.allocate_exact_size(EVec2::new(ui.available_width(), 30.0), egui::Sense::hover());
            let painter = ui.painter();
            let third = rect.width() / 3.0;
            for (i, (label, value)) in [(n("BUILT"), p.stats.built), (n("LOST"), p.stats.lost), (n("KILLS"), p.stats.kills)].into_iter().enumerate() {
                let x = rect.left() + third * (i as f32 + 0.5);
                painter.text(Pos2::new(x, rect.top()), Align2::CENTER_TOP, value.to_string(), display(16.0), TEXT_BRIGHT);
                painter.text(Pos2::new(x, rect.bottom()), Align2::CENTER_BOTTOM, t(label), strong(9.5), STEEL);
                if i > 0 {
                    painter.add(metal::seam_down((rect.left() + third * i as f32).round(), rect.y_range().shrink(3.0)));
                }
            }
        }
        self.under_construction(ui, ctx, vfs, game);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bar gives up its parts as it narrows, in order, and its gauges shrink with them.
    #[test]
    fn the_top_bar_gives_up_parts_as_it_narrows() {
        let tiers: Vec<Tier> = [2000.0, 1400.0, 1300.0, 1240.0, 1100.0, 1000.0, 900.0, 600.0].into_iter().map(Tier::of).collect();
        assert!(tiers.windows(2).all(|w| w[0] >= w[1]), "{tiers:?}");
        assert_eq!((Tier::of(2000.0), Tier::of(1280.0), Tier::of(1100.0), Tier::of(700.0)), (Tier::Wide, Tier::Medium, Tier::Narrow, Tier::Tight));
        assert!(Tier::Wide.gauge() > Tier::Medium.gauge() && Tier::Medium.gauge() > Tier::Narrow.gauge() && Tier::Narrow.gauge() > Tier::Tight.gauge());
    }
}
