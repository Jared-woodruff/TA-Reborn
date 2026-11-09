//! TA's unit information (F1): what the selected unit, or the one under the pointer, is and
//! what it can do. Its picture, cost and build time, health, how it moves, what it sees,
//! its weapons and what else it can do, from the game data (mods' units too).

use egui::{Align2, Color32, RichText, Vec2 as EVec2};
use ta_sim::{Fx, Locomotion, UnitType, WeaponType};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_vfs::Vfs;

use super::{Clearance, ENERGY, Hud, METAL, duration};
use crate::game::Game;
use crate::theme::*;

const WIDTH: f32 = 420.0;
pub(crate) const PICTURE: f32 = 96.0;
/// Space kept over the page (under the interface's top bar) and under it.
const MARGIN: f32 = 20.0;

impl Hud {
    pub(super) fn unit_info_panel(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, clear: Clearance) {
        let Some(info) = game.unit_info else { return };
        if info.type_id >= game.sim.types.types.len() {
            game.unit_info = None;
            return;
        }
        // The unit itself while it lives and is in sight; its kind after that.
        let unit = info.unit.and_then(|id| game.sim.unit(id)).filter(|u| u.type_id == info.type_id && game.seen(u) == ta_sim::Seen::Visible);
        let ty = game.sim.types.get(info.type_id);
        let title = ty.def.display_name.to_uppercase();
        let key = game.keys.text("help").to_string();
        let subtitle = match unit.and_then(|u| game.sim.players.get(u.owner as usize).map(|p| (u, p))) {
            Some((u, _)) if u.owner == game.local_player && !game.spectator => tf("YOURS · {key} CLOSES", &[("key", &key)]),
            Some((u, p)) => {
                let who = game.player_label(u.owner).to_uppercase();
                let ally = !game.spectator && game.sim.allied(u.owner, game.local_player);
                let side = p.side.to_uppercase();
                if ally { tf("ALLY {player} · {side} · {key} CLOSES", &[("player", &who), ("side", &side), ("key", &key)]) } else { tf("{player} · {side} · {key} CLOSES", &[("player", &who), ("side", &side), ("key", &key)]) }
            }
            None => tf("{key} CLOSES", &[("key", &key)]),
        };
        // (In its owner's colours; a kind of unit in the player's.)
        let owner = unit.map_or(game.hud_player(), |u| u.owner);
        let pic = self.unit_pic_of(ctx, vfs, game, info.type_id, PICTURE, owner);
        // As tall as the room between the interface's bars allows, as much under the page as over
        // it: the page's frame and header round its list as laid out last frame (a guess before).
        let chrome_id = egui::Id::new("unit_info_chrome");
        let chrome = ctx.data(|d| d.get_temp::<f32>(chrome_id)).unwrap_or(130.0);
        let max_height = (ctx.content_rect().height() - clear.top - clear.bottom - 2.0 * MARGIN - chrome).max(220.0);
        let mut close = false;
        egui::Area::new(egui::Id::new("unit_info")).anchor(Align2::CENTER_TOP, [clear.left / 2.0, clear.top + MARGIN]).order(egui::Order::Foreground).show(ctx, |ui| {
            let page = Panel::new(&title).subtitle(&subtitle).width(WIDTH).exit(Exit::Close);
            let mut list = egui::Rect::NOTHING;
            let response = page.show(ui, |ui| {
                // (An area offers only the room it took last frame: ask for the height outright.)
                let area = egui::ScrollArea::vertical().max_height(max_height).min_scrolled_height(max_height).auto_shrink([false, true]);
                list = ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Plate, |ui| {
                    ui.set_width(ui.available_width());
                    head(ui, game, ty, unit, pic);
                    facts(ui, game, ty);
                })
                .inner_rect;
            });
            if list.is_positive() {
                ctx.data_mut(|d| d.insert_temp(chrome_id, response.rect.height() - list.height()));
            }
            close = response.exit;
        });
        if close {
            game.unit_info = None;
        }
    }
}

/// The room between the picture and what is beside it.
const BESIDE_GAP: f32 = 10.0;
/// The figures sit beside the picture where it leaves them this much room, else under it.
const BESIDE_MIN: f32 = 200.0;
/// A row of the figures, and the health bar.
pub(crate) const FIGURE_ROW: f32 = 19.0;
const BAR: f32 = 18.0;
/// The gap between a figure's two halves of a row, and the least between a caption and its
/// value.
const FIGURE_GAP: f32 = 14.0;
const CAPTION_GAP: f32 = 10.0;

/// The picture, and beside it what the unit is and its key figures: TA's word for it, how it
/// moves, its health (a bar on one in play), what it costs, how long it takes to build and
/// what it sees. The figures are a table of two to a row (a figure too wide for half a row
/// has one of its own), beside the picture where there is room and across the card under it
/// where there is not. The picture's frame stands as tall as what is beside it; what is
/// beside a taller picture is centred on it.
pub(crate) fn head(ui: &mut egui::Ui, game: &Game, ty: &UnitType, unit: Option<&ta_sim::Unit>, pic: Option<super::UnitPic>) {
    let width = ui.available_width();
    let picture = (width * 0.23).clamp(48.0, PICTURE);
    let column = width - picture - BESIDE_GAP;
    let painter = ui.painter().clone();

    // What it is (where TA's description says more than its name) and how it moves.
    let mut lines = Vec::new();
    let description = ty.def.description.trim();
    if !description.is_empty() && !description.eq_ignore_ascii_case(ty.def.display_name.trim()) {
        lines.push(painter.layout(description.to_owned(), body(13.5), TEXT_BRIGHT, column));
    }
    lines.push(painter.layout(moves(ty), body(12.5), TEXT, column));
    // A unit in play: its health as a bar, how far it is built and its rank.
    let bar = unit.map(|u| {
        let max = ty.max_health.to_f32();
        let health = u.health.to_f32().max(0.0);
        let mut state = Vec::new();
        if !u.is_complete() {
            state.push(tf("{pct}% built", &[("pct", &format!("{:.0}", (u.build_progress.to_f32() * 100.0).floor()))]));
        }
        if u.kills > 0 {
            let rank = [n("Recruit"), n("Veteran"), n("Hardened"), n("Elite")][usize::from(u.rank().min(3))];
            state.push(if u.kills == 1 { tf("{rank} · 1 kill", &[("rank", &t(rank))]) } else { tf("{rank} · {n} kills", &[("rank", &t(rank)), ("n", &u.kills)]) });
        }
        let state = (!state.is_empty()).then(|| painter.layout(state.join("  ·  "), body(12.0), TEXT, column));
        ((health / max.max(1.0)).clamp(0.0, 1.0), tf("Health {now} / {max}", &[("now", &figure(health)), ("max", &figure(max))]), state)
    });
    let figures = key_figures(game, ty, unit.is_none());
    let beside = column >= BESIDE_MIN;
    let rows = figure_rows(&painter, &figures, if beside { column } else { width });

    // How tall what stands beside the picture is.
    let mut stack = lines.iter().map(|l| l.size().y).sum::<f32>() + 3.0 * (lines.len() - 1) as f32;
    if let Some((_, _, state)) = &bar {
        stack += 7.0 + BAR + state.as_ref().map_or(0.0, |s| 3.0 + s.size().y);
    }
    if beside {
        stack += 7.0 + rows.len() as f32 * FIGURE_ROW;
    }
    // (A frame a little taller than the picture keeps it whole, the ground round it.)
    let frame = stack.clamp(picture, picture * 1.4);
    let (rect, _) = ui.allocate_exact_size(EVec2::new(width, frame.max(stack)), egui::Sense::hover());
    let plate = egui::Rect::from_min_size(rect.min, EVec2::new(picture, frame));
    let square = egui::Rect::from_center_size(plate.center(), EVec2::splat(picture));
    match pic {
        Some(p) => {
            p.paint_framed(&painter, plate, square, Color32::WHITE);
            painter.rect_stroke(plate.expand(1.0), 0.0, egui::Stroke::new(1.0, LINE_HI), egui::StrokeKind::Outside);
        }
        None => crate::encyclopaedia::no_picture(&painter, plate, &ty.def.display_name),
    }

    let left = plate.max.x + BESIDE_GAP;
    let mut y = rect.min.y + ((frame - stack) / 2.0).max(0.0);
    for (i, l) in lines.into_iter().enumerate() {
        if i > 0 {
            y += 3.0;
        }
        let height = l.size().y;
        painter.galley(egui::pos2(left, y), l, TEXT);
        y += height;
    }
    if let Some((frac, label, state)) = bar {
        y += 7.0;
        health_bar(&painter, egui::Rect::from_min_size(egui::pos2(left, y), EVec2::new(column, BAR)), frac, label);
        y += BAR;
        if let Some(s) = state {
            y += 3.0;
            let height = s.size().y;
            painter.galley(egui::pos2(left, y), s, TEXT);
            y += height;
        }
    }
    if beside {
        paint_figures(ui, egui::pos2(left, y + 7.0), column, &figures, &rows);
    } else {
        ui.add_space(6.0);
        let (table, _) = ui.allocate_exact_size(EVec2::new(width, rows.len() as f32 * FIGURE_ROW), egui::Sense::hover());
        paint_figures(ui, table.min, width, &figures, &rows);
    }
}

/// How it moves, and how fast (or that it is a building).
fn moves(ty: &UnitType) -> String {
    let d = &ty.def;
    let speed = format!("{:.1}", d.max_velocity);
    match ty.locomotion {
        Locomotion::Static => t("A building").to_string(),
        // (TA's categories say which: KBOT walks, TANK drives; the Commander walks. Its
        // movement classes don't: kbots have TANK ones.)
        Locomotion::Ground => {
            let says = |word: &str| d.category.split_whitespace().any(|c| c.eq_ignore_ascii_case(word));
            let deep = d.max_water_depth >= 200.0;
            match (says("KBOT") || d.is_commander, says("TANK"), deep) {
                (true, _, true) => tf("Walks, under water too · speed {speed}", &[("speed", &speed)]),
                (true, _, false) => tf("Walks · speed {speed}", &[("speed", &speed)]),
                (false, true, true) => tf("Drives, under water too · speed {speed}", &[("speed", &speed)]),
                (false, true, false) => tf("Drives · speed {speed}", &[("speed", &speed)]),
                (false, false, true) => tf("Walks or drives, under water too · speed {speed}", &[("speed", &speed)]),
                (false, false, false) => tf("Walks or drives · speed {speed}", &[("speed", &speed)]),
            }
        }
        Locomotion::Hover => tf("Hovers over land and water · speed {speed}", &[("speed", &speed)]),
        Locomotion::Naval => tf("Sails · speed {speed}", &[("speed", &speed)]),
        Locomotion::Air => tf("Flies · speed {speed}", &[("speed", &speed)]),
    }
}

/// A figure of the head's table: its caption, its value in its colour, and what hovering over
/// it says.
pub(crate) struct KeyFigure {
    caption: &'static str,
    value: String,
    color: Color32,
    hover: Option<String>,
}

/// What it costs and how long it takes to build, its health (`health`: a kind of unit's; one
/// in play shows its own as a bar) and how far it sees, by eye, radar and sonar.
fn key_figures(game: &Game, ty: &UnitType, health: bool) -> Vec<KeyFigure> {
    let d = &ty.def;
    let plain = |caption: &'static str, value: String| KeyFigure { caption, value, color: TEXT_BRIGHT, hover: None };
    let mut out = vec![
        KeyFigure { caption: n("METAL"), value: figure(ty.cost_metal.to_f32()), color: METAL, hover: None },
        KeyFigure { caption: n("ENERGY"), value: figure(ty.cost_energy.to_f32()), color: ENERGY, hover: None },
    ];
    if let Some(builder) = usual_builder(&game.sim, ty) {
        let time = duration((ty.build_work as f64 / builder.work_rate.max(1) as f64 / 30.0) as f32);
        let hover = tf("Build time {time} ({builder})", &[("time", &time), ("builder", &unbroken(&builder.def.display_name))]);
        out.push(KeyFigure { hover: Some(hover), ..plain(n("BUILD TIME"), time) });
    }
    if health {
        out.push(plain(n("HEALTH"), figure(ty.max_health.to_f32())));
    }
    out.push(plain(n("SIGHT"), figure(d.sight_distance as f32)));
    if d.radar_distance > 0.0 {
        out.push(plain(n("RADAR"), figure(d.radar_distance as f32)));
    }
    if d.sonar_distance > 0.0 {
        out.push(plain(n("SONAR"), figure(d.sonar_distance as f32)));
    }
    out
}

/// The command deck's figures for a kind of unit at a glance: what it fights with (the damage
/// a second of its ordinary weapons together, their longest range), how fast it goes, how hard
/// it builds; then what it costs and sees, as the head of the information page has them.
pub(crate) fn summary_figures(game: &Game, ty: &UnitType) -> Vec<KeyFigure> {
    let plain = |caption: &'static str, value: String| KeyFigure { caption, value, color: TEXT_BRIGHT, hover: None };
    let mut out = Vec::new();
    let (mut dps, mut range) = (0.0f32, 0.0f32);
    for w in ty.weapons.iter().flatten().map(|&w| game.sim.weapon_types.get(w)) {
        if w.command_fire || w.def.stockpile || w.paralyzer {
            continue;
        }
        dps += w.default_damage.to_f32() * w.burst.max(1) as f32 * 30.0 / w.reload_ticks.max(1) as f32;
        range = range.max(w.range.to_f32());
    }
    if dps > 0.0 {
        out.push(KeyFigure { caption: n("DAMAGE"), value: tf("{rate}/s", &[("rate", &figure(dps))]), color: Color32::from_rgb(236, 128, 96), hover: None });
        out.push(plain(n("RANGE"), figure(range)));
    }
    if ty.locomotion != Locomotion::Static {
        out.push(KeyFigure { hover: Some(moves(ty)), ..plain(n("SPEED"), format!("{:.1}", ty.def.max_velocity)) });
    }
    if ty.worker_rate > Fx::ZERO && (ty.is_builder || ty.is_factory) {
        out.push(plain(n("BUILD POWER"), figure(ty.def.worker_time as f32)));
    }
    out.extend(key_figures(game, ty, false));
    out
}

fn figure_caption(painter: &egui::Painter, caption: &str) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        ta_ui::i18n::upper(t(caption)),
        egui::TextFormat { font_id: strong(10.5), color: STEEL_DIM, extra_letter_spacing: ta_ui::i18n::tracking(1.1), ..Default::default() },
    );
    job.wrap.max_rows = 1;
    painter.layout_job(job)
}

fn figure_value(painter: &egui::Painter, f: &KeyFigure) -> std::sync::Arc<egui::Galley> {
    painter.layout_no_wrap(f.value.clone(), strong(13.5), f.color)
}

/// The figures (by index) in rows `width` across: two to a row, a figure whose caption and
/// value are too wide for half a row on one of its own.
pub(crate) fn figure_rows(painter: &egui::Painter, figures: &[KeyFigure], width: f32) -> Vec<Vec<usize>> {
    let half = (width - FIGURE_GAP) / 2.0;
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut open = false;
    for (i, f) in figures.iter().enumerate() {
        let wide = figure_caption(painter, f.caption).size().x + CAPTION_GAP + figure_value(painter, f).size().x > half;
        if wide {
            rows.push(vec![i]);
            open = false;
        } else if open {
            rows.last_mut().expect("an open row").push(i);
            open = false;
        } else {
            rows.push(vec![i]);
            open = true;
        }
    }
    rows
}

/// The figures' table from `top_left`, `width` across: each caption at its cell's left, its
/// value at the right, the capitals of both on one line, a faint rule between the rows.
pub(crate) fn paint_figures(ui: &egui::Ui, top_left: egui::Pos2, width: f32, figures: &[KeyFigure], rows: &[Vec<usize>]) {
    let painter = ui.painter();
    let half = (width - FIGURE_GAP) / 2.0;
    for (r, row) in rows.iter().enumerate() {
        let y = top_left.y + r as f32 * FIGURE_ROW;
        if r > 0 {
            painter.hline(top_left.x..=top_left.x + width, y.round() + 0.5, egui::Stroke::new(1.0, LINE.gamma_multiply(0.7)));
        }
        for (k, &i) in row.iter().enumerate() {
            let f = &figures[i];
            let x = top_left.x + k as f32 * (half + FIGURE_GAP);
            let caption = figure_caption(painter, f.caption);
            let value = figure_value(painter, f);
            // (A figure alone in its row keeps to the left half where it fits there.)
            let wide = row.len() == 1 && caption.size().x + CAPTION_GAP + value.size().x > half;
            let cell = egui::Rect::from_min_size(egui::pos2(x, y), EVec2::new(if wide { width } else { half }, FIGURE_ROW));
            // (Both lines of lettering centred on their capitals, in the middle of the row.)
            let middle = cell.center().y + 0.5;
            let (top, foot) = ta_ui::metal::cap_band(&caption);
            painter.galley(egui::pos2(cell.min.x, (middle - (top + foot) / 2.0).round()), caption, STEEL_DIM);
            let (top, foot) = ta_ui::metal::cap_band(&value);
            painter.galley(egui::pos2(cell.max.x - value.size().x, (middle - (top + foot) / 2.0).round()), value, f.color);
            if let Some(hover) = &f.hover {
                ui.interact(cell, ui.id().with(("key_figure", i)), egui::Sense::hover()).on_hover_text(hover);
            }
        }
    }
}

/// Its weapons, what it makes and what else it can do.
pub(crate) fn facts(ui: &mut egui::Ui, game: &Game, ty: &UnitType) {
    let whole = |v: f64| format!("{v:.0}");
    let d = &ty.def;

    // (Weapons that read the same, in several of its slots, once, with how many it carries.)
    let mut weapons: Vec<(WeaponBlock, usize)> = Vec::new();
    for &id in ty.weapons.iter().flatten() {
        let block = WeaponBlock::of(game.sim.weapon_types.get(id));
        match weapons.iter_mut().find(|(seen, _)| *seen == block) {
            Some((_, count)) => *count += 1,
            None => weapons.push((block, 1)),
        }
    }
    if !weapons.is_empty() {
        panel::section(ui, n("WEAPONS"));
        for (block, count) in weapons {
            block.show(ui, count);
        }
    }

    let economy = economy_lines(ty);
    if !economy.is_empty() {
        panel::section(ui, n("ECONOMY"));
        for e in economy {
            line(ui, &e);
        }
    }

    let mut can = Vec::new();
    if ty.worker_rate > Fx::ZERO && (ty.is_builder || ty.is_factory) {
        let what = if ty.is_factory { tf("Builds units: build power {n}", &[("n", &whole(d.worker_time))]) } else { tf("Builds: build power {n}, reach {range}", &[("n", &whole(d.worker_time)), ("range", &whole(d.build_distance))]) };
        can.push(what);
    }
    if d.can_reclaim && ty.is_builder && !ty.is_factory {
        can.push(t("Repairs and reclaims").to_string());
    }
    if d.can_capture {
        can.push(t("Captures enemy units").to_string());
    }
    if d.can_resurrect {
        can.push(t("Raises wrecks as units").to_string());
    }
    if ty.transport_capacity == 1 {
        can.push(t("Carries 1 unit").to_string());
    } else if ty.transport_capacity > 1 {
        can.push(tf("Carries {n} units", &[("n", &ty.transport_capacity)]));
    }
    if d.cloak_cost > 0.0 {
        can.push(if d.cloak_cost_moving > d.cloak_cost {
            tf("Cloaks for {rate} energy a second, {moving} on the move", &[("rate", &whole(d.cloak_cost)), ("moving", &whole(d.cloak_cost_moving))])
        } else {
            tf("Cloaks for {rate} energy a second", &[("rate", &whole(d.cloak_cost))])
        });
    }
    if d.radar_jam > 0.0 {
        can.push(tf("Jams radar {range}", &[("range", &whole(d.radar_jam))]));
    }
    if d.sonar_jam > 0.0 {
        can.push(tf("Jams sonar {range}", &[("range", &whole(d.sonar_jam))]));
    }
    if d.stealth {
        can.push(t("Stealthy: not on radar").to_string());
    }
    if d.on_offable {
        can.push(t("Switches on and off").to_string());
    }
    if d.is_airbase {
        can.push(t("Repairs aircraft that land on it").to_string());
    }
    if d.immune_to_paralyzer {
        can.push(t("Cannot be paralysed").to_string());
    }
    if d.kamikaze {
        can.push(t("Blows itself up near enemies").to_string());
    }
    if !can.is_empty() {
        panel::section(ui, n("ABILITIES"));
        for c in can {
            line(ui, &c);
        }
    }
}

/// What it makes, uses and stores, a line each.
pub(crate) fn economy_lines(ty: &UnitType) -> Vec<String> {
    let d = &ty.def;
    let mut economy = Vec::new();
    let energy = d.energy_make - d.energy_use;
    let rate = |v: f64| if v < 10.0 { format!("{v:.1}") } else { format!("{v:.0}") };
    if energy >= 0.05 {
        economy.push(tf("Makes {rate} energy a second", &[("rate", &rate(energy))]));
    }
    if d.wind_generator > 0.0 {
        economy.push(t("Energy from the wind").to_string());
    }
    if d.tidal_generator > 0.0 {
        economy.push(t("Energy from the tide").to_string());
    }
    let metal = d.metal_make + d.makes_metal;
    if metal >= 0.05 {
        economy.push(tf("Makes {rate} metal a second", &[("rate", &rate(metal))]));
    }
    if d.extracts_metal > 0.0 {
        economy.push(t("Extracts metal from the ground under it").to_string());
    }
    if energy <= -0.05 {
        economy.push(tf("Uses {rate} energy a second", &[("rate", &rate(-energy))]));
    }
    if d.metal_storage > 0.0 {
        economy.push(tf("Stores {n} metal", &[("n", &figure(d.metal_storage as f32))]));
    }
    if d.energy_storage > 0.0 {
        economy.push(tf("Stores {n} energy", &[("n", &figure(d.energy_storage as f32))]));
    }
    economy
}

/// What a weapon's block says: its name, its damage, range and pace, the damage it does a
/// second (none for weapons fired on command or stockpiled), and what is special about it.
#[derive(PartialEq)]
struct WeaponBlock {
    name: String,
    figures: Vec<String>,
    per_second: Option<f32>,
    special: Vec<String>,
}

impl WeaponBlock {
    fn of(w: &WeaponType) -> WeaponBlock {
        let name = crate::perspective::weapon_name(w);
        let shots = w.burst.max(1);
        let damage = w.default_damage.to_f32();
        let every = w.reload_ticks.max(1) as f32 / 30.0;
        let damage_text = if shots > 1 { tf("Damage {d} x{n}", &[("d", &figure(damage)), ("n", &shots)]) } else { tf("Damage {d}", &[("d", &figure(damage))]) };
        let range = if w.def.interceptor {
            tf("Missile defence {range}", &[("range", &figure(w.interceptor_coverage().to_f32()))])
        } else {
            tf("Range {range}", &[("range", &figure(w.range.to_f32()))])
        };
        let figures = vec![damage_text, range, tf("Every {secs} s", &[("secs", &format!("{every:.1}"))])];
        let per_second = (!w.command_fire && !w.def.stockpile && !w.paralyzer).then(|| damage * shots as f32 / every);
        let mut special = Vec::new();
        if w.paralyzer {
            special.push(t("Paralyses instead of harming"));
        }
        if w.command_fire {
            special.push(t("Fires only on command"));
        }
        if w.def.stockpile {
            special.push(t("Builds up missiles to fire"));
        }
        if w.def.interceptor {
            special.push(t("Shoots down missiles"));
        }
        if w.to_air {
            special.push(t("Aircraft only"));
        }
        if w.water {
            special.push(t("Under water"));
        }
        if w.aoe_radius > Fx::from_int(24) {
            special.push(t("Splash damage"));
        }
        let mut special: Vec<String> = special.into_iter().map(str::to_string).collect();
        if w.energy_per_shot > Fx::ZERO {
            let text = if w.def.stockpile { n("{n} energy per missile") } else { n("{n} energy a shot") };
            special.push(tf(text, &[("n", &format!("{:.0}", w.energy_per_shot.to_f32()))]));
        }
        if w.metal_per_shot > Fx::ZERO {
            let text = if w.def.stockpile { n("{n} metal per missile") } else { n("{n} metal a shot") };
            special.push(tf(text, &[("n", &format!("{:.0}", w.metal_per_shot.to_f32()))]));
        }
        WeaponBlock { name, figures, per_second, special }
    }

    /// The block, for `count` of the weapon: its name with their number, the damage a second
    /// of them all (after the other figures where the line has room, else under them).
    fn show(&self, ui: &mut egui::Ui, count: usize) {
        let name = if count > 1 { tf("{name} x{n}", &[("name", &self.name), ("n", &count)]) } else { self.name.clone() };
        ui.label(RichText::new(name).font(strong(12.0)).color(TEXT_BRIGHT));
        let figures = stats(&self.figures);
        match self.per_second {
            Some(rate) => {
                let rate = tf("{dps} damage a second", &[("dps", &figure(rate * count as f32))]);
                let whole = stats(&[figures.clone(), rate.clone()]);
                if ta_ui::widgets::galley(ui, RichText::new(&whole).font(body(12.5))).size().x <= ui.available_width() {
                    line(ui, &whole);
                } else {
                    line(ui, &figures);
                    line(ui, &rate);
                }
            }
            None => line(ui, &figures),
        }
        if !self.special.is_empty() {
            ui.label(RichText::new(self.special.join("  ·  ")).font(body(11.5)).color(STEEL));
        }
        ui.add_space(3.0);
    }
}

/// Figures in a line that wraps between them, never inside one: each one's spaces don't
/// break, nor the one before the dot after it.
fn stats(parts: &[String]) -> String {
    parts.iter().map(|p| p.replace(' ', "\u{a0}")).collect::<Vec<_>>().join("\u{a0}\u{a0}·\u{a0} ")
}

/// A whole number, its digits grouped in threes by narrow spaces from 10 000 up (as SI writes
/// them, the same in every language).
pub(crate) fn figure(v: f32) -> String {
    let digits = format!("{:.0}", v.max(0.0));
    if digits.len() < 5 {
        return digits;
    }
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    out
}

/// A unit's name kept on one line when the text it is in wraps (egui never breaks a line at a
/// no-break space).
pub(crate) fn unbroken(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join("\u{a0}")
}

fn line(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).font(body(12.5)).color(TEXT)).wrap());
}

/// A unit's health as a bar over `rect`, labelled: the label's capitals centred in it.
fn health_bar(p: &egui::Painter, rect: egui::Rect, frac: f32, text: String) {
    p.rect_filled(rect, 2.0, INSET);
    let color = if frac > 0.6 { GREEN } else if frac > 0.3 { AMBER } else { RED };
    let mut fill = rect.shrink(2.0);
    fill.set_width(fill.width() * frac);
    p.rect_filled(fill, 1.0, color.gamma_multiply(0.75));
    let label = p.layout_no_wrap(text, strong(11.5), Color32::WHITE);
    let (top, foot) = ta_ui::metal::cap_band(&label);
    let pos = egui::pos2(rect.center().x - label.size().x / 2.0, rect.center().y - (top + foot) / 2.0).round();
    p.galley_with_override_text_color(pos + EVec2::new(0.0, 1.0), label.clone(), Color32::from_black_alpha(150));
    p.galley(pos, label, Color32::WHITE);
}

/// Who usually builds it, for its build time: a factory, else the Commander, else the first
/// constructor that can.
fn usual_builder<'a>(sim: &'a ta_sim::Sim, ty: &UnitType) -> Option<&'a UnitType> {
    let builders: Vec<&UnitType> = sim.types.types.iter().filter(|b| b.worker_rate > Fx::ZERO && b.build_options.contains(&ty.id)).collect();
    builders.iter().find(|b| b.is_factory).or_else(|| builders.iter().find(|b| b.def.is_commander)).or(builders.first()).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missile_cards_show_both_construction_costs_and_the_defended_range() {
        let mut data = ta_data::GameData::load(&ta_vfs::Vfs::new());
        let doc = ta_formats::tdf::parse("[ANTI]{ stockpile=1; interceptor=1; range=32000; coverage=2000; energypershot=10000; metalpershot=200; reloadtime=120; }");
        data.weapons.push(ta_data::WeaponDef::from_tdf(&doc.sections[0]));
        let weapons = ta_sim::WeaponTypes::compile(&data);
        let block = WeaponBlock::of(weapons.get(0));
        assert!(block.special.contains(&"10000 energy per missile".to_string()));
        assert!(block.special.contains(&"200 metal per missile".to_string()));
        assert!(block.figures.contains(&"Missile defence 2000".to_string()));
        assert!(!block.figures.iter().any(|s| s.contains("32000")));
    }

    #[test]
    fn figures_are_grouped_in_threes_from_ten_thousand() {
        assert_eq!(figure(0.0), "0");
        assert_eq!(figure(9999.4), "9999");
        assert_eq!(figure(10_000.0), "10\u{202f}000");
        assert_eq!(figure(1_234_567.0), "1\u{202f}234\u{202f}567");
        assert_eq!(figure(999_999.6), "1\u{202f}000\u{202f}000", "rounded first");
        assert_eq!(figure(-5.0), "0", "never below nothing");
        assert_eq!(figure(f32::NAN), "0");
    }

    #[test]
    fn names_and_lines_of_figures_keep_together() {
        assert_eq!(unbroken("Big  Bertha "), "Big\u{a0}Bertha");
        assert_eq!(stats(&["12 kills".into(), "3 lost".into()]), "12\u{a0}kills\u{a0}\u{a0}·\u{a0} 3\u{a0}lost");
    }
}
