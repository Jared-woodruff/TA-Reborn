//! The companion screen's page (its window is `companion.rs`'s): the battle's map at the size of
//! the screen, beside the intel a commander keeps glancing at. Across the top, the map's name,
//! the battle's clock and pace and whose battle it is. The map is the minimap's, as large as the
//! screen has room for: the units, the fog, the view's footprint, attacks and pings; a click looks
//! there, a right-click sends the selection, Alt+click pings. Beside it: the economy's charts and
//! what is being built; the forces by group (a click selects them); the factories, what each
//! makes and how far along, the idle ones first, and the idle constructors (a click on a factory
//! selects it and looks, NEXT the next idle constructor); the battle's events, newest first, an
//! attack's with LOOK. Watching (a replay, spectating), the players head to head in place of the
//! factories. Outside a battle, a card says what will be there.
//!
//! The page is the HUD's own panels drawn again in the companion's interface: a second `Hud`,
//! whose pictures and skin are loaded there.

use egui::{Align, Align2, Color32, Layout, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder, pos2, vec2};
use ta_render::Camera;
use ta_ui::i18n::{n, t, tf};
use ta_ui::metal;
use ta_vfs::Vfs;

use super::deck::{force_list, section};
use super::{ACCENT, Hud};
use crate::game::Game;
use crate::minimap::Minimap;
use crate::theme::*;

/// Points: round the page, between plates, inside them; the header's height; the narrowest a
/// column of plates is drawn.
const MARGIN: f32 = 10.0;
const GAP: f32 = 8.0;
const PAD: f32 = 10.0;
const HEADER: f32 = 42.0;
const COLUMN_LEAST: f32 = 230.0;
/// A factory's row and an event's.
const ROW: f32 = 30.0;
const EVENT_ROW: f32 = 22.0;

impl Hud {
    /// The companion screen's page in `ui` (the whole of its window): the battle's, or outside
    /// one (`waiting`, with the key that opens and closes it) a card saying what will be there.
    pub fn companion_page(&mut self, ui: &mut egui::Ui, vfs: &Vfs, game: &mut Game, camera: &mut Camera, minimap: &mut Minimap, waiting: Option<&str>) {
        let ctx = ui.ctx().clone();
        let screen = ctx.content_rect();
        ui.painter().rect_filled(screen, 0.0, Color32::from_rgb(10, 11, 12));
        if let Some(key) = waiting {
            waiting_card(ui, screen, key);
            return;
        }
        let side = game.sim.players.get(game.hud_player() as usize).map_or_else(String::new, |p| p.side.clone());
        self.skin.load(&ctx, vfs, &side);
        self.trend.sample(game);
        let header = Rect::from_min_size(screen.min + vec2(MARGIN, MARGIN), vec2(screen.width() - 2.0 * MARGIN, HEADER));
        self.companion_plate(ui, header, false, |hud, ui| hud.companion_close |= header_line(ui, game));
        // The map as large as the room leaves it beside two columns of plates (one where the
        // screen is too narrow for two); a wide map leaves room under it, where the events go.
        let page = Rect::from_min_max(pos2(header.left(), header.bottom() + GAP), pos2(header.right(), screen.bottom() - MARGIN));
        let aspect = game.map.height_px() as f32 / game.map.width_px().max(1) as f32;
        // (The map's plate: the map drawn as large as it may be, and its legend under it. The
        // plates take what the map leaves filling the page's height, two columns of them at
        // least where a map of 40% of the width leaves room for two, else one.)
        let legend = 18.0;
        let chrome = legend + 2.0 * PAD + 6.0;
        let map_wanted = (page.height() - chrome) / aspect.max(0.2) + 2.0 * PAD;
        let least = 2.0 * COLUMN_LEAST + GAP;
        let two = page.width() - least - GAP >= page.width() * 0.4;
        let plates_width = if two { (page.width() - map_wanted - GAP).clamp(least, page.width() * 0.5) } else { (page.width() * 0.34).max(COLUMN_LEAST) };
        let map_room = page.width() - plates_width - GAP;
        let map_height = (map_room - 2.0 * PAD) * aspect.max(0.2) + chrome;
        let events_under = page.height() - map_height - GAP >= 100.0;
        let map = Rect::from_min_size(page.min, vec2(map_room, if events_under { map_height } else { page.height() }));
        let rest = Rect::from_min_max(pos2(map.right() + GAP, page.top()), page.max);
        self.companion_plate(ui, map, true, |_, ui| {
            let room = ui.available_rect_before_wrap();
            let mut inner = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(room.min, vec2(room.width(), room.height() - legend))));
            let drawn = minimap.draw(&mut inner, game, camera, room.width(), room.height() - legend);
            ui.painter().add(crate::hud_skin::bezel(drawn, 4.0, true));
            ui.painter().text(pos2(room.center().x, room.bottom() - 2.0), Align2::CENTER_BOTTOM, t("Click: look · Right-click: send the selection · Alt+click: ping"), body(11.5), STEEL);
        });
        let watching = game.spectator || game.playback.is_some();
        // The columns of plates: (what, its share of its column's height).
        let mut first = Vec::new();
        let mut second = Vec::new();
        if watching {
            first.push((Plate::Players, 0.42));
            first.push((Plate::Economy, 0.58));
            second.push((Plate::Forces, 0.55));
        } else {
            first.push((Plate::Economy, 0.45));
            first.push((Plate::Forces, 0.55));
            second.push((Plate::Production, 0.55));
        }
        if !events_under {
            second.push((Plate::Events, 0.45));
        }
        let columns: Vec<Vec<(Plate, f32)>> = if two { vec![first, second] } else { vec![first.into_iter().chain(second).collect()] };
        let width = (rest.width() - GAP * (columns.len() as f32 - 1.0)) / columns.len() as f32;
        let mut placed: Vec<(Plate, Rect)> = Vec::new();
        for (c, column) in columns.iter().enumerate() {
            let x = rest.left() + c as f32 * (width + GAP);
            let total: f32 = column.iter().map(|(_, share)| share).sum();
            let room = rest.height() - GAP * (column.len() as f32 - 1.0);
            let mut y = rest.top();
            for &(plate, share) in column {
                let height = (room * share / total).floor();
                placed.push((plate, Rect::from_min_size(pos2(x, y), vec2(width, height))));
                y += height + GAP;
            }
        }
        if events_under {
            placed.push((Plate::Events, Rect::from_min_max(pos2(map.left(), map.bottom() + GAP), pos2(map.right(), page.bottom()))));
        }
        for (plate, rect) in placed {
            self.companion_plate(ui, rect, true, |hud, ui| match plate {
                Plate::Players => hud.players_plate(ui, vfs, game),
                Plate::Economy => hud.companion_economy(ui, vfs, game),
                Plate::Forces => {
                    let ctx = ui.ctx().clone();
                    let forces = force_list(game);
                    hud.forces(ui, &ctx, vfs, game, &forces);
                }
                Plate::Production => hud.production(ui, vfs, game),
                Plate::Events => events(ui, game),
            });
        }
    }

    /// A plate of the page at `rect`: the side's panel metal (riveted when `rivets`), and what
    /// `add` puts in it inside its padding.
    fn companion_plate(&mut self, ui: &mut egui::Ui, rect: Rect, rivets: bool, add: impl FnOnce(&mut Hud, &mut egui::Ui)) {
        let back = self.skin.deck(ui.ctx(), rect, rivets);
        ui.painter().add(back);
        let mut inner = ui.new_child(UiBuilder::new().max_rect(rect.shrink(PAD)).layout(Layout::top_down(Align::Min)));
        inner.set_clip_rect(rect.shrink(2.0));
        add(self, &mut inner);
    }

    /// The economy: what is in store and how it goes, as the top bar has it, over the charts
    /// (taller than the deck's) and what is being built.
    fn companion_economy(&mut self, ui: &mut egui::Ui, vfs: &Vfs, game: &mut Game) {
        section(ui, "ECONOMY");
        let ctx = ui.ctx().clone();
        if let Some(p) = game.sim.players.get(game.hud_player() as usize) {
            let e = &p.economy;
            let rationed = e.build_factor < ta_sim::Fx::from_ratio(97, 100);
            let time = ctx.input(|i| i.time);
            let width = (ui.available_width() - 190.0).clamp(60.0, 260.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            super::resource_bar(ui, n("METAL"), super::Store::of(e.metal, e.metal_capacity, e.metal_income, e.metal_wanted, rationed), super::METAL, width, time, false);
            super::resource_bar(ui, n("ENERGY"), super::Store::of(e.energy, e.energy_capacity, e.energy_income, e.energy_wanted, rationed), super::ENERGY, width, time, false);
            ui.add_space(6.0);
        }
        self.economy_charts(ui, &ctx, vfs, game, 96.0);
    }

    /// The players (a replay, spectating): a card each, their colour, name and side and their
    /// army's worth (a bar against the largest) over what they make a second, their units and
    /// kills and what their factories build.
    fn players_plate(&mut self, ui: &mut egui::Ui, vfs: &Vfs, game: &Game) {
        section(ui, "PLAYERS");
        let ctx = ui.ctx().clone();
        let players = super::cast::standings(game);
        let playing: Vec<&super::cast::Standing> = players.iter().filter(|s| game.setup.players.get(s.player as usize).is_some()).collect();
        let most = playing.iter().map(|s| s.army).fold(1.0f32, f32::max);
        let width = ui.available_width();
        let card = 46.0;
        let fit = ((ui.available_height() + 4.0) / (card + 4.0)).floor().max(1.0) as usize;
        ui.spacing_mut().item_spacing.y = 4.0;
        for s in playing.iter().take(fit) {
            let pics: Vec<(Option<super::UnitPic>, f32)> = s.making.iter().take(3).map(|&(kind, done)| (self.unit_pic_of(&ctx, vfs, game, kind, 18.0, s.player), done)).collect();
            let (rect, _) = ui.allocate_exact_size(vec2(width, card), Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, 3.0, Color32::from_black_alpha(70));
            p.rect_filled(Rect::from_min_size(rect.min + vec2(0.0, 4.0), vec2(4.0, card - 8.0)), 1.5, s.colour);
            let top = rect.top() + 13.0;
            let name_colour = if s.defeated { STEEL_DIM } else { super::cast::readable(s.colour) };
            let name = p.layout_no_wrap(s.name.clone(), strong(13.5), name_colour);
            let name_width = name.size().x;
            p.galley(pos2(rect.left() + 12.0, top - name.size().y / 2.0), name, Color32::WHITE);
            p.text(pos2(rect.left() + 18.0 + name_width, top + 1.0), Align2::LEFT_CENTER, &s.side, strong(10.0), STEEL);
            p.text(pos2(rect.right() - 8.0, top), Align2::RIGHT_CENTER, grouped(s.army), strong(13.0), TEXT_BRIGHT);
            // The army's worth against the largest, under the name.
            let bar = Rect::from_min_max(pos2(rect.left() + 12.0, top + 9.0), pos2(rect.right() - 8.0, top + 12.0));
            p.rect_filled(bar, 1.0, INSET);
            p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * (s.army / most).clamp(0.0, 1.0), bar.height())), 1.0, s.colour);
            let line = rect.bottom() - 11.0;
            let mut x = rect.left() + 12.0;
            for (text, colour) in [(format!("+{:.1}", s.metal), super::METAL), (format!("+{:.0}", s.energy), super::ENERGY), (tf("{count} units", &[("count", &s.units)]), TEXT), (tf("{count} kills", &[("count", &s.kills)]), AMBER)] {
                let g = p.layout_no_wrap(text, strong(11.0), colour);
                let w = g.size().x;
                p.galley(pos2(x, line - g.size().y / 2.0), g, colour);
                x += w + 10.0;
            }
            let mut px = rect.right() - 8.0 - 18.0;
            for (pic, done) in pics.iter().rev() {
                if px < x {
                    break;
                }
                let r = Rect::from_min_size(pos2(px, line - 9.0), vec2(18.0, 18.0));
                match pic {
                    Some(pic) => pic.paint(p, r, Color32::WHITE),
                    None => {
                        p.rect_filled(r, 2.0, INSET);
                    }
                }
                p.rect_filled(Rect::from_min_size(pos2(r.left(), r.bottom() - 2.0), vec2(r.width() * done, 2.0)), 0.0, GREEN);
                px -= 21.0;
            }
        }
        if playing.len() > fit {
            ui.label(RichText::new(tf("{count} more", &[("count", &(playing.len() - fit))])).font(body(11.5)).color(STEEL));
        }
    }

    /// The factories, what each makes and how far along (the idle first), a click on one
    /// selecting it and looking; under them the idle constructors and NEXT.
    fn production(&mut self, ui: &mut egui::Ui, vfs: &Vfs, game: &mut Game) {
        section(ui, "PRODUCTION");
        let ctx = ui.ctx().clone();
        let owner = game.hud_player();
        struct Row {
            id: ta_sim::UnitId,
            kind: usize,
            making: Option<(usize, f32)>,
            queued: usize,
        }
        let mut rows: Vec<Row> = Vec::new();
        let mut idle_builders = 0;
        for u in game.sim.units() {
            if u.owner != owner || u.dead || !u.is_complete() {
                continue;
            }
            let ty = game.sim.unit_type(u);
            if ty.is_factory {
                let making = u.factory_queue.front().map(|&kind| (kind, u.build.and_then(|b| game.sim.unit(b.target)).map_or(0.0, |b| b.build_progress.to_f32().clamp(0.0, 1.0))));
                rows.push(Row { id: u.id, kind: u.type_id, making, queued: u.factory_queue.len() });
            } else if ty.is_builder && !ty.def.is_commander && u.orders.is_empty() && u.build.is_none() && !game.on_duty(u.id) {
                idle_builders += 1;
            }
        }
        // (The idle first: they want something to do; then the furthest along.)
        rows.sort_by(|a, b| a.making.is_some().cmp(&b.making.is_some()).then(b.making.map_or(0.0, |m| m.1).total_cmp(&a.making.map_or(0.0, |m| m.1))).then(a.id.cmp(&b.id)));
        let footer = 30.0;
        let fit = (((ui.available_height() - footer) / (ROW + 3.0)).floor().max(0.0)) as usize;
        if rows.is_empty() {
            ui.label(RichText::new(t("No factories.")).font(body(12.5)).color(STEEL_DIM));
        }
        ui.spacing_mut().item_spacing.y = 3.0;
        let width = ui.available_width();
        let shown = if rows.len() > fit { fit.saturating_sub(1).max(1) } else { rows.len() };
        for row in rows.iter().take(shown) {
            let (rect, response) = ui.allocate_exact_size(vec2(width, ROW), Sense::click());
            let factory = self.unit_pic(&ctx, vfs, game, row.kind, 26.0);
            let making = row.making.and_then(|(kind, _)| self.unit_pic(&ctx, vfs, game, kind, 22.0));
            let p = ui.painter();
            p.rect_filled(rect, 3.0, if response.hovered() { Color32::from_white_alpha(14) } else { Color32::from_black_alpha(60) });
            let tile = Rect::from_min_size(rect.min + vec2(2.0, 2.0), vec2(26.0, 26.0));
            match factory {
                Some(pic) => pic.paint(p, tile, Color32::WHITE),
                None => {
                    p.rect_filled(tile, 2.0, INSET);
                }
            }
            let name = crate::chat::unit_label(&game.sim, row.kind);
            let name = p.layout(name, strong(12.0), TEXT, (rect.width() * 0.45).max(40.0));
            p.galley(pos2(tile.right() + 8.0, rect.center().y - name.size().y / 2.0), name, TEXT);
            match row.making {
                Some((_, done)) => {
                    let bar = Rect::from_min_size(pos2(rect.right() - 92.0, rect.center().y + 5.0), vec2(56.0, 4.0));
                    p.add(metal::well(bar.expand(1.0), INSET));
                    p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * done, bar.height())), 1.0, GREEN);
                    let tile = Rect::from_min_size(pos2(rect.right() - 92.0 + 17.0, rect.top() + 2.0), vec2(20.0, 16.0));
                    if let Some(pic) = making {
                        pic.paint(p, Rect::from_center_size(tile.center(), vec2(18.0, 18.0)), Color32::WHITE);
                    }
                    if row.queued > 1 {
                        p.text(pos2(rect.right() - 6.0, rect.center().y), Align2::RIGHT_CENTER, format!("×{}", row.queued), strong(12.0), STEEL);
                    }
                }
                None => {
                    p.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, t("Idle"), strong(12.0), AMBER_BRIGHT);
                }
            }
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                game.select_and_look(row.id);
            }
        }
        if rows.len() > shown {
            ui.label(RichText::new(tf("{count} more", &[("count", &(rows.len() - shown))])).font(body(11.5)).color(STEEL));
        }
        // The idle constructors: a click on the line selects the next one.
        let left = ui.available_height();
        if left > footer - 4.0 {
            ui.add_space((left - footer).max(0.0));
        }
        let some = idle_builders > 0;
        let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), if some { Sense::click() } else { Sense::hover() });
        let p = ui.painter();
        p.rect_filled(rect, 3.0, if some && response.hovered() { Color32::from_rgb(70, 52, 30) } else { Color32::from_black_alpha(60) });
        if some {
            p.rect_stroke(rect, 3.0, Stroke::new(1.0, AMBER.gamma_multiply(0.5)), StrokeKind::Inside);
        }
        let colour = if some { AMBER_BRIGHT } else { STEEL };
        p.text(pos2(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, tf("Idle constructors: {count}", &[("count", &idle_builders)]), strong(12.0), colour);
        if some {
            let tip = pos2(rect.right() - 12.0, rect.center().y);
            p.add(egui::Shape::convex_polygon(vec![tip + vec2(-6.0, -5.0), tip + vec2(2.0, 0.0), tip + vec2(-6.0, 5.0)], colour, Stroke::NONE));
            if response.on_hover_text(t("Next idle constructor")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                game.select_next_idle_builder();
            }
        }
    }
}

/// 12,345.
fn grouped(v: f32) -> String {
    ta_ui::i18n::grouped(v.max(0.0).round() as u64)
}

/// What the page shows of the battle in a plate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plate {
    Players,
    Economy,
    Forces,
    Production,
    Events,
}

/// The header: the map and the clock, the pace, whose battle it is, and the screen's close
/// button (a display it fills has no other); true when that was pressed.
fn header_line(ui: &mut egui::Ui, game: &Game) -> bool {
    let whole = ui.available_rect_before_wrap();
    let close = Rect::from_center_size(pos2(whole.right() - 13.0, whole.center().y), vec2(26.0, 26.0));
    let closed = ui.scope_builder(UiBuilder::new().max_rect(close), |ui| ta_ui::widgets::icon_button(ui, ta_ui::widgets::Glyph::Close, true)).inner.clicked();
    let rect = Rect::from_min_max(whole.min, pos2(close.left() - 16.0, whole.max.y));
    let p = ui.painter();
    let mid = rect.center().y;
    p.text(pos2(rect.left(), mid - 7.0), Align2::LEFT_CENTER, t("COMPANION SCREEN"), strong(10.5), ACCENT);
    p.text(pos2(rect.left(), mid + 8.0), Align2::LEFT_CENTER, &game.setup.map, strong(15.0), TEXT_BRIGHT);
    let s = game.sim.tick / 30;
    let clock = if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) };
    // (The clock in the middle of the whole header.)
    p.text(pos2(whole.center().x, mid), Align2::CENTER_CENTER, clock, display(22.0), TEXT_BRIGHT);
    let pace = if game.paused { t("PAUSED").to_string() } else if (game.speed - 1.0).abs() > 0.01 { format!("{:.2}×", game.speed).replace(".00", "") } else { String::new() };
    if !pace.is_empty() {
        p.text(pos2(whole.center().x + 52.0, mid), Align2::LEFT_CENTER, pace, strong(12.0), if game.paused { AMBER_BRIGHT } else { STEEL });
    }
    // Whose battle: their colour, name and side.
    let player = game.hud_player();
    if let Some(pl) = game.sim.players.get(player as usize) {
        let colour = crate::palette::team_color(pl.color);
        let name = p.layout_no_wrap(game.player_label(player), strong(14.0), colour.lerp_to_gamma(Color32::WHITE, 0.35));
        let side = p.layout_no_wrap(pl.side.to_uppercase(), strong(10.5), STEEL);
        let right = rect.right();
        let side_x = right - side.size().x;
        let name_x = side_x - 6.0 - name.size().x;
        p.rect_filled(Rect::from_center_size(pos2(name_x - 10.0, mid), vec2(5.0, 22.0)), 1.5, colour);
        p.galley(pos2(name_x, mid - name.size().y / 2.0), name, Color32::WHITE);
        p.galley(pos2(side_x, mid - side.size().y / 2.0 + 1.0), side, Color32::WHITE);
    }
    closed
}

/// The battle's events, newest first, as the history keeps them; an attack's with LOOK.
fn events(ui: &mut egui::Ui, game: &mut Game) {
    section(ui, "EVENTS");
    let fit = ((ui.available_height() / (EVENT_ROW + 2.0)).floor().max(1.0)) as usize;
    let entries: Vec<(u64, String, u32)> = game.event_history.entries().rev().take(fit).map(|e| (e.tick, e.text.clone(), e.repeats)).collect();
    if entries.is_empty() {
        ui.label(RichText::new(t("Nothing yet.")).font(body(12.5)).color(STEEL_DIM));
        return;
    }
    let now = game.sim.tick;
    ui.spacing_mut().item_spacing.y = 2.0;
    let width = ui.available_width();
    for (tick, text, repeats) in entries {
        // (An attack: the alert posted with it, to look at.)
        let at = game.alerts.iter().find(|(_, t)| ((t * 30.0).round() as u64).abs_diff(tick) <= 30).map(|(p, _)| *p);
        let (rect, _) = ui.allocate_exact_size(vec2(width, EVENT_ROW), Sense::hover());
        let fresh = now.saturating_sub(tick) < 30 * 10;
        let p = ui.painter();
        let s = tick / 30;
        p.text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, format!("{:02}:{:02}", s / 60, s % 60), strong(11.0), if fresh { AMBER_BRIGHT } else { STEEL });
        let look = 44.0;
        let text = if repeats > 1 { format!("{text} ×{repeats}") } else { text };
        let room = rect.width() - 46.0 - if at.is_some() { look + 6.0 } else { 0.0 };
        let galley = p.layout(text, body(12.0), if fresh { TEXT_BRIGHT } else { TEXT }, room.max(40.0));
        // (One line: a longer one is cut short.)
        let clip = Rect::from_min_max(pos2(rect.left() + 44.0, rect.top()), pos2(rect.left() + 44.0 + room, rect.bottom()));
        p.with_clip_rect(clip).galley(pos2(clip.left(), rect.center().y - galley.rows.first().map_or(galley.size().y, |r| r.height()) / 2.0), galley, TEXT);
        if let Some(pos) = at {
            let button = Rect::from_min_size(pos2(rect.right() - look, rect.top() + 2.0), vec2(look, EVENT_ROW - 4.0));
            let response = ui.interact(button, ui.id().with(("look", tick)), Sense::click());
            let p = ui.painter();
            p.rect_filled(button, 2.0, if response.hovered() { Color32::from_rgb(70, 52, 30) } else { Color32::from_rgb(48, 38, 24) });
            p.rect_stroke(button, 2.0, Stroke::new(1.0, AMBER.gamma_multiply(0.6)), StrokeKind::Inside);
            p.text(button.center(), Align2::CENTER_CENTER, t("LOOK"), strong(10.5), AMBER_BRIGHT);
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                game.focus_request = Some(pos);
            }
        }
    }
}

/// Outside a battle: the emblem, the screen's name, and what will be there.
fn waiting_card(ui: &mut egui::Ui, screen: Rect, key: &str) {
    let ctx = ui.ctx().clone();
    let id = egui::Id::new("companion_emblem");
    let emblem = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        let image = egui::ColorImage::from_rgba_unmultiplied([128, 128], &ta_ui::icon::emblem_rgba(128));
        let texture = ctx.load_texture("companion_emblem", image, egui::TextureOptions::LINEAR);
        ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
        texture
    });
    let p = ui.painter();
    let mid = screen.center();
    let band = Rect::from_center_size(mid, vec2(screen.width(), 150.0));
    p.image(emblem.id(), Rect::from_center_size(pos2(mid.x, band.top() - 62.0), vec2(96.0, 96.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    p.rect_filled(band, 0.0, Color32::from_rgb(18, 19, 20));
    for y in [band.top(), band.bottom()] {
        p.hline((mid.x - 260.0)..=(mid.x + 260.0), y, Stroke::new(1.0, AMBER.gamma_multiply(0.5)));
    }
    p.text(pos2(mid.x, mid.y - 34.0), Align2::CENTER_CENTER, t("COMPANION SCREEN"), display(30.0), AMBER);
    p.text(pos2(mid.x, mid.y + 10.0), Align2::CENTER_CENTER, t("The battle's map and intel appear here when a battle begins"), body(15.0), TEXT);
    p.text(pos2(mid.x, mid.y + 40.0), Align2::CENTER_CENTER, tf("{key} opens and closes this screen", &[("key", &key)]), body(13.0), STEEL);
}
