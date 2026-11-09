//! The modern HUD's command deck, along the foot of the screen, where TA's side panel took a
//! column of it: up to three plates of steel, each as large as what it holds, the battlefield
//! between and above them. (The minimap is not on the deck: it stands at the top right, under the
//! top bar, in a plate of its own, and the economy's readout is in the top bar.)
//!
//! * At the left, the player's forces by group, a click away each, the group the selection is
//!   (or is in) lit.
//! * In the middle, the selection: its picture, what it is doing, its health and rank, beside
//!   the kinds of a mixed selection (a click keeps one), the switches that narrow it, and a
//!   factory's or a constructor's queue. With nothing selected there is no middle plate.
//! * At the right, when something of ours is selected, its orders (in rows by kind) and beside
//!   them its stance (the standing orders, in words), and its build menu, grouped by purpose on
//!   tabs, the costs on the pictures, as many across as fit in two rows (on pages past that).
//!
//! The other overlays keep above the deck ([`DECK`], [`GAP`]: the HUD's clearance).

use std::collections::{HashMap, VecDeque};

use egui::{Align2, Color32, CornerRadius, Pos2, Rect, RichText, Stroke, Vec2 as EVec2};
use glam::Vec3;
use ta_ai::Duty;
use ta_render::Camera;
use ta_sim::{Command, Fx, Locomotion, Order, Seen, Target, Unit, UnitId, UnitType};
use ta_ui::i18n::{n, t, tf};
use ta_ui::metal::{self, Slab};
use ta_vfs::Vfs;

use super::{ACCENT, ENERGY, Hud, METAL, count, duration, pic_button, pic_frame, unit_facts};
use crate::game::Game;
use crate::hud_skin;
use crate::theme::*;

/// Seconds of economy the charts show.
const HISTORY: usize = 180;
/// The deck's plates: their height (points) at the most and the least (a quarter of the
/// screen's between them), the padding inside them, and the room between them and round them.
const DECK_MOST: f32 = 180.0;
const DECK_LEAST: f32 = 150.0;
const PAD: f32 = 8.0;
pub(super) const GAP: f32 = 8.0;
/// The command plate's column of orders; the stance (the standing orders) beside it, in words
/// ([`STANCE`]) or, where there is not the room, as glyphs alone ([`STATES`]); the middle plate
/// at its widest and narrowest, and wide enough to stand its parts side by side.
const ORDERS: f32 = 220.0;
const STANCE: f32 = 134.0;
const STATES: f32 = 34.0;
/// The room between the order buttons, and either side of the seam between them and the stance.
const ORDER_GAP: f32 = 4.0;
const MIDDLE_MOST: f32 = 620.0;
const MIDDLE_LEAST: f32 = 250.0;
const MIDDLE_SPLIT: f32 = 430.0;
/// The room the selection's plate is given before the others give up their words.
const MIDDLE_WANTED: f32 = 330.0;
/// The forces' plate: one column of groups' width, the shortest a group's row is drawn, and
/// the room its caption takes.
const FORCE_COLUMN: f32 = 176.0;
const FORCE_COLUMN_NARROW: f32 = 140.0;
const FORCE_ROW_LEAST: f32 = 25.0;
/// Ticks the groups of the forces' plate are kept (their counts, health and idle marks).
const FORCES_EVERY: u64 = 6;
const FORCE_CAPTION: f32 = 22.0;
/// The build menu's pictures: three across at the least, the largest, the smallest before they
/// go on pages, the gap between them; and the most across.
const COLUMNS: usize = 3;
const TILE: f32 = 66.0;
const TILE_MIN: f32 = 46.0;
const GRID_GAP: f32 = 4.0;
const COLUMNS_MOST: usize = 9;
/// The pager under a build menu on pages, with the gap over it; and the points of a
/// trackpad's scrolling that make a notch of the wheel (a page).
const PAGER: f32 = 18.0;
const WHEEL_STEP: f32 = 40.0;
/// Orders that fight, in the activity line.
const FIGHTING: Color32 = Color32::from_rgb(236, 128, 96);

/// The economy's last minutes: metal in and out, energy in and out (per second), one sample
/// a second of game time, each the mean of the frames drawn in that second; and the economy as
/// it stands this frame (what the top bar reads).
#[derive(Default)]
pub(super) struct EconomyTrend {
    samples: VecDeque<[f32; 4]>,
    sum: [f32; 4],
    frames: u32,
    now: [f32; 4],
    /// How far into its second the game is.
    fraction: f32,
    /// The game second being summed, and whose economy.
    second: u64,
    player: u8,
}

/// What the deck says of the selection as a whole (see `Hud::selection_contents`).
#[derive(Clone, Default)]
pub(super) struct SelectionSummary {
    /// The types selected, most first.
    kinds: Vec<(usize, usize)>,
    many: usize,
    health: f32,
    max_health: f32,
    idle: usize,
    damaged: usize,
    /// What the constructors in it do by themselves.
    duty: crate::game::duty::SelectionDuty,
}

impl SelectionSummary {
    fn of(game: &Game) -> Self {
        let units: Vec<&Unit> = game.selection.iter().filter_map(|&id| game.sim.unit(id)).collect();
        let mut kinds: Vec<(usize, usize)> = Vec::new();
        for o in &units {
            match kinds.iter_mut().find(|(k, _)| *k == o.type_id) {
                Some((_, k)) => *k += 1,
                None => kinds.push((o.type_id, 1)),
            }
        }
        kinds.sort_by_key(|&(k, many)| (std::cmp::Reverse(many), k));
        let (health, max_health) = units.iter().fold((0.0f32, 0.0f32), |(h, m), o| (h + o.health.to_f32(), m + game.sim.unit_type(o).max_health.to_f32()));
        // (One on duty has its own work, and waits for it when there is none.)
        let idle = units.iter().filter(|o| o.orders.is_empty() && o.build.is_none() && o.factory_queue.is_empty() && !game.on_duty(o.id)).count();
        let damaged = units.iter().filter(|o| o.health < game.sim.unit_type(o).max_health).count();
        SelectionSummary { kinds, many: units.len(), health, max_health, idle, damaged, duty: game.selection_duty() }
    }
}

impl EconomyTrend {
    pub(super) fn sample(&mut self, game: &Game) {
        let player = game.hud_player();
        let second = game.sim.tick / 30;
        // A new game, a replay started over, another player watched: start again.
        if second < self.second || player != self.player {
            *self = EconomyTrend { player, second, ..Default::default() };
        }
        if second > self.second && self.frames > 0 {
            let frames = self.frames as f32;
            self.samples.push_back(self.sum.map(|v| v / frames));
            while self.samples.len() > HISTORY {
                self.samples.pop_front();
            }
            (self.sum, self.frames) = ([0.0; 4], 0);
        }
        self.second = second;
        self.fraction = (game.sim.tick % 30) as f32 / 30.0;
        let Some(p) = game.sim.players.get(player as usize) else { return };
        let e = &p.economy;
        self.now = [e.metal_income, e.metal_expense, e.energy_income, e.energy_expense].map(|v| v.to_f32());
        for (sum, v) in self.sum.iter_mut().zip(self.now) {
            *sum += v;
        }
        self.frames += 1;
    }

    /// The readings to chart, each at its time (seconds from the chart's left edge), and the
    /// time the chart spans: the seconds gone, each at its end, and the one under way so far,
    /// where it has got to (from the first frame of a battle, a level line at the first).
    pub(super) fn readings(&self) -> (Vec<(f32, [f32; 4])>, f32) {
        let under_way = if self.frames > 0 { self.sum.map(|v| v / self.frames as f32) } else { self.now };
        let span = (self.samples.len() as f32 + self.fraction).max(1.0);
        let mut readings = vec![(0.0, self.samples.front().copied().unwrap_or(under_way))];
        readings.extend(self.samples.iter().enumerate().map(|(i, s)| ((i + 1) as f32, *s)));
        readings.push((span, under_way));
        (readings, span)
    }
}

/// Where a unit belongs in the forces overview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Commander,
    Army,
    Builders,
    Factories,
    Aircraft,
    Navy,
    Defences,
}

impl Group {
    const ALL: [Group; 7] = [Group::Commander, Group::Army, Group::Builders, Group::Factories, Group::Aircraft, Group::Navy, Group::Defences];

    fn of(ty: &UnitType) -> Option<Group> {
        let armed = ty.weapons.iter().any(|w| w.is_some());
        Some(if ty.def.is_commander {
            Group::Commander
        } else if ty.is_factory {
            Group::Factories
        } else if ty.locomotion == Locomotion::Static {
            if armed { Group::Defences } else { return None }
        } else if ty.is_builder {
            Group::Builders
        } else if ty.locomotion == Locomotion::Air {
            Group::Aircraft
        } else if ty.locomotion == Locomotion::Naval {
            Group::Navy
        } else {
            Group::Army
        })
    }

    fn label(self) -> &'static str {
        match self {
            Group::Commander => n("COMMANDER"),
            Group::Army => n("ARMY"),
            // (As the selection's filter and the top bar call them; shorter in a narrow row.)
            Group::Builders => n("BUILDERS"),
            Group::Factories => n("FACTORIES"),
            Group::Aircraft => n("AIRCRAFT"),
            Group::Navy => n("NAVY"),
            Group::Defences => n("DEFENCES"),
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Group::Commander => n("Click to select the Commander and look at it (or press {key})"),
            Group::Army => n("Click to select the army: every unit on land or hovering that isn't a constructor"),
            Group::Builders => n("Click to select every constructor; click the idle count for the next idle one"),
            Group::Factories => n("Click to select every factory; click the idle count for the next idle one"),
            Group::Aircraft => n("Click to select every aircraft"),
            Group::Navy => n("Click to select every ship and submarine"),
            Group::Defences => n("Click to select every tower and battery"),
        }
    }
}

/// One group of the player's units in the overview.
pub(super) struct Force {
    group: Group,
    ids: Vec<UnitId>,
    /// Its unit types and how many of each (the commonest pictures the group).
    types: Vec<(usize, usize)>,
    /// Constructors or factories with nothing to do.
    idle: usize,
    health: f32,
    max_health: f32,
    /// How many of its units the selection holds (all of them: it is the selection).
    selected: usize,
}

impl Force {
    fn picture(&self) -> Option<usize> {
        self.types.iter().max_by_key(|&&(ty, k)| (k, std::cmp::Reverse(ty))).map(|&(ty, _)| ty)
    }
}

/// What a click on a group asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ForceClick {
    Select,
    NextIdle,
    Look,
}

/// What a building is for: the headings of a constructor's build menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    Economy,
    Factories,
    Defence,
    Sensors,
    Other,
}

impl Purpose {
    pub(crate) const ALL: [Purpose; 5] = [Purpose::Economy, Purpose::Factories, Purpose::Defence, Purpose::Sensors, Purpose::Other];

    pub(crate) fn of(ty: &UnitType) -> Purpose {
        let d = &ty.def;
        let makes = ty.extracts_metal > Fx::ZERO || ty.makes_metal > Fx::ZERO || ty.metal_make > Fx::ZERO || ty.energy_make > ty.energy_use;
        if ty.is_factory {
            Purpose::Factories
        } else if ty.locomotion != Locomotion::Static {
            Purpose::Other
        } else if ty.weapons.iter().any(|w| w.is_some()) || d.tedclass.eq_ignore_ascii_case("FORT") {
            Purpose::Defence
        } else if makes || ty.wind_generator > Fx::ZERO || ty.tidal_generator > Fx::ZERO || ty.metal_storage > Fx::ZERO || ty.energy_storage > Fx::ZERO {
            Purpose::Economy
        } else if d.radar_distance > 0.0 || d.sonar_distance > 0.0 || d.radar_jam > 0.0 || d.sonar_jam > 0.0 {
            Purpose::Sensors
        } else {
            Purpose::Other
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Purpose::Economy => n("Economy: metal, energy and storage"),
            Purpose::Factories => n("Factories: where units are made"),
            Purpose::Defence => n("Defence: towers, batteries and walls"),
            Purpose::Sensors => n("Sensors: radar, sonar and jammers"),
            Purpose::Other => n("Other buildings"),
        }
    }

    /// The tab's name.
    pub(crate) fn short(self) -> &'static str {
        match self {
            Purpose::Economy => n("ECONOMY"),
            Purpose::Factories => n("FACTORY"),
            Purpose::Defence => n("DEFENCE"),
            Purpose::Sensors => n("SENSORS"),
            Purpose::Other => n("OTHER"),
        }
    }
}

/// A click in the build menu: the type, whether with the right button, whether with Shift.
type BuildClick = (usize, bool, bool);

impl Hud {
    /// The command deck: the forces' plate, the middle plate and the command plate (see the
    /// module's notes), and the minimap's plate at the top right (see [`Hud::minimap_plate`]).
    pub(super) fn deck(&mut self, root: &mut egui::Ui, vfs: &Vfs, game: &mut Game, camera: &mut Camera, minimap: &mut crate::minimap::Minimap) {
        let ctx = root.ctx().clone();
        let selected = game.selection.iter().any(|&id| game.sim.unit(id).is_some());
        let commanding = selected && !game.spectator;
        // The forces (in the cast, the scoreboard has those).
        let forces_key = (game.hud_player(), game.sim.tick / FORCES_EVERY);
        let forces = if game.cast { None } else { Some(self.forces_now(game, forces_key)) };
        let layout = deck_layout(ctx.content_rect(), forces.as_ref().map(|f| f.len()), selected, commanding, if commanding { self.build_width(game) } else { 0.0 });
        self.minimap_plate(&ctx, game, camera, minimap);
        if let (Some(plate), Some(forces)) = (layout.forces, forces.as_deref()) {
            self.plate(&ctx, "deck_forces", plate, true, PAD, |hud, ui| hud.forces_plate(ui, &ctx, vfs, game, forces, layout.force_columns));
        }
        // The selection.
        if let Some(middle) = layout.middle {
            self.plate(&ctx, "deck_middle", middle, true, PAD, |hud, ui| {
                let wide = ui.available_width() >= MIDDLE_SPLIT;
                if wide {
                    let room = ui.available_rect_before_wrap();
                    let head = (room.width() * 0.48).floor();
                    let mut left = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(room.min, EVec2::new(head, room.height()))));
                    hud.selection_contents(&mut left, &ctx, vfs, game, Part::Head);
                    let rest = Rect::from_min_max(Pos2::new(room.min.x + head + PAD + 4.0, room.min.y), room.max);
                    ui.painter().add(metal::seam_down(rest.left() - PAD / 2.0 - 2.0, room.y_range().shrink(4.0)));
                    let mut right = ui.new_child(egui::UiBuilder::new().max_rect(rest));
                    egui::ScrollArea::vertical().id_salt("deck_group").auto_shrink([false, false]).show(&mut right, |ui| hud.selection_contents(ui, &ctx, vfs, game, Part::Group));
                } else {
                    egui::ScrollArea::vertical().id_salt("deck_selection").auto_shrink([false, false]).show(ui, |ui| {
                        hud.selection_contents(ui, &ctx, vfs, game, Part::Head);
                        hud.selection_contents(ui, &ctx, vfs, game, Part::Group);
                    });
                }
            });
        }
        // Its orders and stance, and its build menu beside them.
        if let Some(command) = layout.command {
            self.plate(&ctx, "deck_command", command, true, PAD, |hud, ui| {
                let room = ui.available_rect_before_wrap();
                let orders = Rect::from_min_size(room.min, EVec2::new(layout.orders, room.height()));
                let mut column = ui.new_child(egui::UiBuilder::new().max_rect(orders));
                hud.order_buttons(&mut column, game, layout.stance);
                if layout.build > 0.0 {
                    let build = Rect::from_min_size(Pos2::new(orders.right() + GAP, room.min.y), EVec2::new(layout.build, room.height()));
                    ui.painter().add(metal::seam_down(orders.right() + GAP / 2.0, room.y_range().shrink(4.0)));
                    let mut column = ui.new_child(egui::UiBuilder::new().max_rect(build));
                    hud.selection_contents(&mut column, &ctx, vfs, game, Part::Build);
                }
            });
        }
        self.forces_cache = forces.map(|f| (forces_key, f));
    }

    /// The minimap in its bezel, in a plate of its own at the top right, under the top bar (its
    /// place and size: [`minimap_layout`]); the economy's panel and the idle alerts keep clear of
    /// it, as do the panels that hang at the right (`Clearance::right_top`).
    fn minimap_plate(&mut self, ctx: &egui::Context, game: &mut Game, camera: &mut Camera, minimap: &mut crate::minimap::Minimap) {
        let aspect = game.map.height_px() as f32 / game.map.width_px().max(1) as f32;
        let Some(rect) = self.minimap_rect else { return };
        self.plate(ctx, "deck_map", rect, true, MAP_PAD, |_, ui| {
            const BEZEL: f32 = MAP_BEZEL;
            let room = ui.available_rect_before_wrap().shrink(BEZEL + 1.0);
            // (A map wider than the plate's shape: in the middle of it, top to bottom.)
            let room = Rect::from_center_size(room.center(), EVec2::new(room.width(), (room.width() * aspect).min(room.height())));
            let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(room));
            let map = minimap.draw(&mut inner, game, camera, room.width(), room.height());
            ui.painter().add(hud_skin::bezel(map, BEZEL, true));
        });
    }

    /// One of the deck's plates at `rect`: its metal (riveted when `rivets`), and what `add`
    /// puts in it, inside `pad` points of steel.
    fn plate(&mut self, ctx: &egui::Context, id: &str, rect: Rect, rivets: bool, pad: f32, add: impl FnOnce(&mut Hud, &mut egui::Ui)) {
        let back = self.skin.deck(ctx, rect, rivets);
        egui::Area::new(egui::Id::new(id)).fixed_pos(rect.min).movable(false).constrain(false).order(egui::Order::Middle).show(ctx, |ui| {
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            ui.painter().add(back);
            let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(pad)).layout(egui::Layout::top_down(egui::Align::Min)));
            inner.set_clip_rect(rect.shrink(2.0));
            add(self, &mut inner);
        });
    }

    /// How wide the selection's build menu would be (none for no build menu): as many of its
    /// fullest tab's pictures across as two rows need, between three and [`COLUMNS_MOST`].
    fn build_width(&self, game: &Game) -> f32 {
        let units: Vec<&Unit> = game.selection.iter().filter_map(|&id| game.sim.unit(id)).collect();
        let Some(first) = units.first() else { return 0.0 };
        if units.iter().any(|u| u.type_id != first.type_id) {
            return 0.0;
        }
        let ty = game.sim.unit_type(first);
        let options: Vec<usize> = ty.build_options.iter().copied().filter(|&o| game.sim.is_allowed(o)).collect();
        if options.is_empty() {
            return 0.0;
        }
        let fullest = if ty.is_factory {
            options.len()
        } else {
            Purpose::ALL.iter().map(|&p| options.iter().filter(|&&o| Purpose::of(game.sim.types.get(o)) == p).count()).max().unwrap_or(0)
        };
        let columns = fullest.div_ceil(2).clamp(COLUMNS, COLUMNS_MOST);
        columns as f32 * TILE + (columns - 1) as f32 * GRID_GAP
    }
}

/// The hud player's units by group (those with none left out), as the forces' plate and the
/// companion screen show them.
pub(super) fn force_list(game: &Game) -> Vec<Force> {
    let owner = game.hud_player();
    let mut forces: Vec<Force> = Group::ALL.iter().map(|&group| Force { group, ids: Vec::new(), types: Vec::new(), idle: 0, health: 0.0, max_health: 0.0, selected: 0 }).collect();
    for u in game.sim.units() {
        if u.owner != owner || u.dead || !u.is_complete() || u.carried_by.is_some() {
            continue;
        }
        let ty = game.sim.unit_type(u);
        let Some(f) = Group::of(ty).and_then(|g| forces.iter_mut().find(|f| f.group == g)) else { continue };
        f.ids.push(u.id);
        match f.types.iter_mut().find(|(k, _)| *k == u.type_id) {
            Some((_, k)) => *k += 1,
            None => f.types.push((u.type_id, 1)),
        }
        // (As the idle counters on the top bar count them.)
        f.idle += match f.group {
            Group::Builders => usize::from(u.orders.is_empty() && u.build.is_none() && !game.on_duty(u.id)),
            Group::Factories => usize::from(u.factory_queue.is_empty() && u.build.is_none()),
            _ => 0,
        };
        f.health += u.health.to_f32();
        f.max_health += ty.max_health.to_f32();
    }
    forces.retain(|f| !f.ids.is_empty());
    mark_selected(&mut forces, game);
    forces
}

/// How many units of each group the selection holds (a pass over the selection alone, so that
/// the groups can be kept for a few ticks and the lit one still follow the selection).
fn mark_selected(forces: &mut [Force], game: &Game) {
    let owner = game.hud_player();
    for f in forces.iter_mut() {
        f.selected = 0;
    }
    for &id in &game.selection {
        let Some(u) = game.sim.unit(id).filter(|u| u.owner == owner) else { continue };
        if let Some(g) = Group::of(game.sim.unit_type(u))
            && let Some(f) = forces.iter_mut().find(|f| f.group == g)
        {
            f.selected += 1;
        }
    }
}

impl Hud {
    /// The player's forces for this frame: the groups as they were within the last few ticks (a
    /// pass over every unit of a big battle was not to be made every frame), the selection's
    /// share as it is now. (`deck` keeps them again when it has drawn them.)
    fn forces_now(&mut self, game: &Game, key: (u8, u64)) -> Vec<Force> {
        let mut forces = match self.forces_cache.take() {
            Some((k, forces)) if k == key => forces,
            _ => force_list(game),
        };
        mark_selected(&mut forces, game);
        forces
    }
}

impl Hud {
    /// The forces' plate: the caption, and the groups under it in as many `columns` as the room
    /// asks (see [`forces_columns`]); a click selects one.
    fn forces_plate(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, forces: &[Force], columns: usize) {
        section(ui, "FORCES");
        if forces.is_empty() {
            ui.label(RichText::new(t("No units left.")).font(body(12.0)).color(STEEL_DIM));
            return;
        }
        self.force_grid(ui, ctx, vfs, game, forces, columns, FORCE_ROW_LEAST);
    }

    /// The forces by group, a row each (two columns of them past four); a click selects one.
    pub(super) fn forces(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, forces: &[Force]) {
        section(ui, "FORCES");
        if forces.is_empty() {
            ui.label(RichText::new(t("No units left.")).font(body(12.0)).color(STEEL_DIM));
            return;
        }
        // (Two columns past four groups, unless one column of them fits: the companion screen's.)
        let one_fits = forces.len() as f32 * 35.0 <= ui.available_height();
        let columns = if forces.len() > 4 && !one_fits { 2 } else { 1 };
        self.force_grid(ui, ctx, vfs, game, forces, columns, 32.0);
    }

    /// The groups in `columns` of rows between `least` and 42 points tall, as the room lets them
    /// be; a click on one acts on it.
    #[allow(clippy::too_many_arguments)]
    fn force_grid(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, forces: &[Force], columns: usize, least: f32) {
        let mut action = None;
        let columns = columns.clamp(1, forces.len().max(1));
        let width = ((ui.available_width() - 4.0 * (columns - 1) as f32) / columns as f32).floor();
        // (Taller with a few, into the room there is.)
        let per_column = forces.len().div_ceil(columns);
        let tall = ((ui.available_height() + 3.0) / per_column as f32 - 3.0).floor().clamp(least, 42.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = EVec2::new(4.0, 3.0);
            for (c, chunk) in forces.chunks(per_column).enumerate() {
                ui.allocate_ui_with_layout(EVec2::new(width, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(width);
                    for (k, f) in chunk.iter().enumerate() {
                        if let Some(a) = self.force_row(ui, ctx, vfs, game, f, tall) {
                            action = Some((c * per_column + k, a));
                        }
                    }
                });
            }
        });
        if let Some((i, a)) = action {
            let f = &forces[i];
            match a {
                ForceClick::NextIdle if f.group == Group::Factories => game.select_next_idle_factory(),
                ForceClick::NextIdle => game.select_next_idle_builder(),
                ForceClick::Select | ForceClick::Look => {
                    game.selection = f.ids.clone();
                    if f.group == Group::Commander || a == ForceClick::Look {
                        game.focus_request = middle_of(game, &f.ids);
                    }
                }
            }
        }
    }

    /// The economy's charts, metal over energy, and under them what is being built (or the
    /// battle so far): the economy panel's and the companion screen's; the charts at most
    /// `tallest` high.
    pub(super) fn economy_charts(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, tallest: f32) {
        let owner = game.hud_player();
        let building = game.sim.units().any(|u| u.owner == owner && !u.dead && !u.is_complete());
        // (The charts share the room the row under them leaves.)
        let under = if building { 52.0 } else { 34.0 };
        let chart = ((ui.available_height() - under - 4.0) / 2.0).floor().clamp(30.0, tallest.max(30.0));
        self.trend_chart(ui, n("METAL"), METAL, 0, chart);
        ui.add_space(4.0);
        self.trend_chart(ui, n("ENERGY"), ENERGY, 2, chart);
        if building {
            self.under_construction(ui, ctx, vfs, game);
        } else if let Some(p) = game.sim.players.get(owner as usize) {
            ui.add_space(4.0);
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
    }

    /// What the player's constructors and factories are building, nearest done first: a
    /// click selects those building it and looks at it.
    pub(super) fn under_construction(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, vfs: &Vfs, game: &mut Game) {
        const SHOWN: usize = 12;
        let owner = game.hud_player();
        let mut frames: Vec<(UnitId, usize, f32)> = game.sim.units().filter(|u| u.owner == owner && !u.dead && !u.is_complete()).map(|u| (u.id, u.type_id, u.build_progress.to_f32().clamp(0.0, 1.0))).collect();
        if frames.is_empty() {
            return;
        }
        frames.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
        ui.add_space(4.0);
        section(ui, "UNDER CONSTRUCTION");
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = EVec2::new(3.0, 3.0);
            for &(id, kind, done) in frames.iter().take(SHOWN) {
                let size = EVec2::splat(28.0);
                let label = crate::chat::unit_label(&game.sim, kind);
                let resp = match self.unit_pic(ctx, vfs, game, kind, size.x) {
                    Some(p) => pic_button(ui, &p, size),
                    None => ui.add_sized(size, egui::Button::new(RichText::new(&label).size(8.0))),
                };
                let r = resp.rect;
                let bar = Rect::from_min_max(Pos2::new(r.min.x + 1.0, r.max.y - 5.0), Pos2::new(r.max.x - 1.0, r.max.y - 1.0));
                ui.painter().rect_filled(bar, CornerRadius::ZERO, Color32::from_black_alpha(180));
                ui.painter().rect_filled(Rect::from_min_size(bar.min, EVec2::new(bar.width() * done, bar.height())), CornerRadius::ZERO, GREEN);
                let pct = format!("{:.0}", (done * 100.0).floor());
                if resp.on_hover_text(tf("{name}: {pct}% built. Click to select those building it and look at it", &[("name", &label), ("pct", &pct)])).clicked() {
                    chosen = Some(id);
                }
            }
            if frames.len() > SHOWN {
                ui.label(RichText::new(format!("+{}", frames.len() - SHOWN)).font(strong(12.0)).color(TEXT));
            }
        });
        if let Some(id) = chosen.filter(|_| !game.spectator) {
            let builders: Vec<UnitId> = game.sim.units().filter(|u| u.owner == owner && u.build.is_some_and(|b| b.target == id)).map(|u| u.id).collect();
            game.selection = if builders.is_empty() { vec![id] } else { builders };
            game.focus_request = game.sim.unit(id).map(|u| Vec3::from_array(u.pos.to_f32()));
        }
    }

    /// A group in the overview, `height` tall: its picture, name and size, how many are idle (a
    /// click on that picks the next of them), and its health along the foot.
    fn force_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, vfs: &Vfs, game: &Game, f: &Force, height: f32) -> Option<ForceClick> {
        let active = !game.spectator;
        let (rect, resp) = ui.allocate_exact_size(EVec2::new(ui.available_width(), height), if active { egui::Sense::click() } else { egui::Sense::hover() });
        if !ui.is_rect_visible(rect) {
            return None;
        }
        let hot = active && resp.hovered();
        ui.painter().add(metal::slab(ctx, rect, Slab::Slot, if hot { 1.0 } else { 0.0 }, active && resp.is_pointer_button_down_on()));
        // The group the selection is (a brass edge) or has some of (a dimmer one).
        if f.selected > 0 {
            let all = f.selected >= f.ids.len();
            ui.painter().rect_stroke(rect.shrink(1.0), CornerRadius::same(2), Stroke::new(if all { 1.6 } else { 1.0 }, if all { AMBER_BRIGHT } else { AMBER_DIM }), egui::StrokeKind::Inside);
        }
        // (A narrow row's picture smaller, to leave the name room.)
        let side = (rect.height() - 8.0).min(if rect.width() < 150.0 { 22.0 } else { f32::MAX });
        let pic = Rect::from_center_size(Pos2::new(rect.left() + 4.0 + side / 2.0, rect.center().y), EVec2::splat(side));
        match f.picture().and_then(|ty| self.unit_pic(ctx, vfs, game, ty, pic.width())) {
            Some(p) => p.paint(ui.painter(), pic, Color32::WHITE),
            None => {
                ui.painter().rect_filled(pic, CornerRadius::same(1), INSET);
            }
        }
        ui.painter().add(pic_frame(pic, false));
        let painter = ui.painter();
        let x = pic.right() + 7.0;
        let mid = rect.center().y - 1.0;
        let name = t(f.group.label()).to_string();
        let name_color = if hot { AMBER_BRIGHT } else { CREAM };
        let name_width = painter.layout_no_wrap(name.clone(), strong(11.0), name_color).size().x;
        let fraction = if f.max_health > 0.0 { (f.health / f.max_health).clamp(0.0, 1.0) } else { 1.0 };
        let foot = Rect::from_min_max(Pos2::new(x, rect.bottom() - 7.0), Pos2::new(rect.right() - 7.0, rect.bottom() - 5.0));
        painter.rect_filled(foot, CornerRadius::ZERO, Color32::from_black_alpha(150));
        painter.rect_filled(Rect::from_min_size(foot.min, EVec2::new(foot.width() * fraction, foot.height())), CornerRadius::ZERO, hud_skin::health_color(fraction));
        // The size at the end (the Commander's health instead, when there is one).
        let size = if f.group == Group::Commander && f.ids.len() == 1 { format!("{:.0}%", (fraction * 100.0).floor()) } else { f.ids.len().to_string() };
        let size = painter.text(Pos2::new(rect.right() - 8.0, mid), Align2::RIGHT_CENTER, size, display(16.0), TEXT_BRIGHT);
        let mut idle = None;
        if f.idle > 0 {
            // (The mark's word goes where the name would not fit beside it: its lamp and number.)
            let galley = painter.layout_no_wrap(tf("{n} idle", &[("n", &f.idle)]), strong(10.5), AMBER_BRIGHT);
            let galley = if size.left() - 27.0 - galley.size().x - x >= name_width { galley } else { painter.layout_no_wrap(f.idle.to_string(), strong(10.5), AMBER_BRIGHT) };
            let text = Align2::RIGHT_CENTER.anchor_size(Pos2::new(size.left() - 9.0, mid), galley.size());
            let lamp = Pos2::new(text.left() - 6.0, text.center().y);
            let area = Rect::from_min_max(Pos2::new(lamp.x - 6.0, rect.top() + 4.0), Pos2::new(text.right() + 4.0, rect.bottom() - 9.0));
            if active && resp.hover_pos().is_some_and(|p| area.contains(p)) {
                painter.rect_filled(area, CornerRadius::same(2), Color32::from_white_alpha(18));
            }
            painter.add(metal::lamp(lamp, 2.5, AMBER, 1.0));
            painter.galley(text.min, galley, AMBER_BRIGHT);
            idle = Some(area);
        }
        // The name in the room the count and the idle mark leave it: smaller (to 9 points), then
        // cut short with an ellipsis. (It was clipped mid-letter: "CONSTI", "FACTOR".)
        let room = (idle.map_or(size.left(), |a| a.left()) - 6.0 - x).max(1.0);
        let points = if name_width <= room { 11.0 } else { (11.0 * room / name_width).max(9.0) };
        let mut job = egui::text::LayoutJob::simple(name, strong(points), name_color, room);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let label = painter.layout_job(job);
        painter.galley(Pos2::new(x, mid - label.size().y / 2.0), label, name_color);
        if !active {
            return None;
        }
        let tip = if f.group == Group::Commander { tf(f.group.tip(), &[("key", &game.keys.text("select_commander"))]) } else { t(f.group.tip()).to_string() };
        let resp = resp.on_hover_text(format!("{tip}\n{}", t("Double-click to look at them")));
        if resp.double_clicked() {
            Some(ForceClick::Look)
        } else if resp.clicked() {
            let on_idle = idle.zip(resp.interact_pointer_pos()).is_some_and(|(area, p)| area.contains(p));
            Some(if on_idle { ForceClick::NextIdle } else { ForceClick::Select })
        } else {
            None
        }
    }

    /// A resource's income (filled) and spending (a red line) over the last three minutes, in a
    /// well `height` points tall: named at its top left, the net rate now at its top right (as
    /// the top bar reads it), the chart under them.
    pub(super) fn trend_chart(&self, ui: &mut egui::Ui, label: &str, color: Color32, index: usize, height: f32) {
        let (income, spending) = (self.trend.now[index], self.trend.now[index + 1]);
        let net = income - spending;
        let (rect, resp) = ui.allocate_exact_size(EVec2::new(ui.available_width(), height), egui::Sense::hover());
        let name = ta_ui::widgets::galley(ui, ta_ui::widgets::caption(label).font(strong(10.0)).color(color));
        let text = if net >= 0.0 { format!("+{net:.1}") } else { format!("{net:.1}") };
        // Growing, steady or shrinking: green, grey, amber (the top bar warns when it runs dry).
        let tone = if net > 0.05 { GREEN } else if net < -0.05 { AMBER_BRIGHT } else { STEEL };
        let rate = ta_ui::widgets::galley(ui, RichText::new(tf("{rate}/s", &[("rate", &text)])).font(strong(11.5)).color(tone));
        let painter = ui.painter();
        painter.add(metal::well(rect, INSET));
        let inner = Rect::from_min_max(Pos2::new(rect.left() + 3.0, rect.top() + 3.0 + name.size().y.max(rate.size().y)), rect.max - EVec2::splat(3.0));
        for k in 1..4 {
            painter.hline(inner.x_range(), (inner.top() + inner.height() * k as f32 / 4.0).round() + 0.5, Stroke::new(1.0, Color32::from_white_alpha(9)));
        }
        // (The battle so far across the whole chart, the newest reading at the right edge: a
        // level line from the first frame, its shape coming as the seconds go by; after three
        // minutes, the last three.)
        let (readings, span) = self.trend.readings();
        let top = readings.iter().map(|(_, s)| s[index].max(s[index + 1])).fold(1.0f32, f32::max) * 1.15;
        let x = |t: f32| inner.left() + inner.width() * t / span;
        let y = |v: f32| inner.bottom() - inner.height() * (v / top).clamp(0.0, 1.0);
        let made: Vec<Pos2> = readings.iter().map(|(t, s)| Pos2::new(x(*t), y(s[index]))).collect();
        let mut fill = egui::Mesh::default();
        let shade = color.gamma_multiply(0.22);
        for (i, p) in made.iter().enumerate() {
            fill.colored_vertex(*p, shade);
            fill.colored_vertex(Pos2::new(p.x, inner.bottom()), shade);
            if i > 0 {
                let k = 2 * i as u32;
                fill.add_triangle(k - 2, k - 1, k);
                fill.add_triangle(k - 1, k + 1, k);
            }
        }
        painter.add(egui::Shape::mesh(fill));
        painter.add(egui::Shape::line(made, Stroke::new(1.5, color)));
        let spent: Vec<Pos2> = readings.iter().map(|(t, s)| Pos2::new(x(*t), y(s[index + 1]))).collect();
        painter.add(egui::Shape::line(spent, Stroke::new(1.2, RED)));
        painter.galley(rect.left_top() + EVec2::new(6.0, 3.0), name, color);
        painter.galley(Pos2::new(rect.right() - 6.0 - rate.size().x, rect.top() + 2.0), rate, tone);
        resp.on_hover_text(tf(
            "Income {income}/s, spending {spending}/s. The chart shows up to three minutes: income filled, spending in red.",
            &[("income", &format!("{income:.1}")), ("spending", &format!("{spending:.1}"))],
        ));
    }

    /// One `part` of the selection: its card, its group and queue, its build menu.
    fn selection_contents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, part: Part) {
        let Some(u) = game.selection.iter().find_map(|&id| game.sim.unit(id)) else { return };
        let (first, owner) = (u.id, u.owner);
        let ty = game.sim.unit_type(u).clone();
        // The sums over the selection, once a frame (each part of the deck asks).
        let pass = ctx.cumulative_pass_nr();
        let summary = match &self.selection_summary {
            Some((at, s)) if *at == pass => s.clone(),
            _ => {
                let s = SelectionSummary::of(game);
                self.selection_summary = Some((pass, s.clone()));
                s
            }
        };
        let SelectionSummary { kinds, many, health, max_health, idle, damaged, duty: duty_sel } = summary;
        let all_same = kinds.len() == 1;
        let doing = (many == 1).then(|| activity(game, u));
        let rank = (many == 1 && u.kills > 0).then(|| (u.rank(), u.kills));
        let stockpile = crate::hud::stockpile(&game.sim, u);
        let cargo = (many == 1 && ty.transport_capacity > 0).then_some((u.cargo.len(), ty.transport_capacity));
        let queue: Vec<usize> = u.factory_queue.iter().copied().collect();
        let queued: HashMap<usize, usize> = queue.iter().fold(HashMap::new(), |mut m, &q| {
            *m.entry(q).or_default() += 1;
            m
        });
        // The unit on the pad and how far along it is.
        let producing = queue.first().map(|&q| (q, u.build.and_then(|b| game.sim.unit(b.target)).map_or(0.0, |b| b.build_progress.to_f32().clamp(0.0, 1.0))));
        let repeat_on = u.factory_repeat;
        let name = crate::chat::unit_label(&game.sim, ty.id);

        if part == Part::Head {
            // The card: picture, name, what it is or they are, what it is doing; what the type is
            // good for on the pointer.
            let facts = if all_same { unit_facts(&game.sim, &ty) } else { Vec::new() };
            let card = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 9.0;
                ui.add_space(3.0);
                let (r, _) = ui.allocate_exact_size(EVec2::splat(56.0), egui::Sense::hover());
                if all_same {
                    match self.unit_pic_of(ctx, vfs, game, kinds[0].0, r.width(), owner) {
                        Some(pic) => pic.paint(ui.painter(), r, Color32::WHITE),
                        None => {
                            ui.painter().rect_filled(r, CornerRadius::ZERO, INSET);
                        }
                    }
                    ui.painter().add(hud_skin::bezel(r, 3.0, false));
                    if many > 1 {
                        count(ui, r.right_bottom() - EVec2::new(4.0, 2.0), Align2::RIGHT_BOTTOM, many, 16.0);
                    }
                } else {
                    // The three commonest types as a stack of pictures, the commonest in front.
                    let shown = kinds.len().min(3);
                    let step = 8.0;
                    let side = r.width() - step * (shown - 1) as f32;
                    for i in (0..shown).rev() {
                        let card = Rect::from_min_size(r.min + EVec2::splat(step * (shown - 1 - i) as f32), EVec2::splat(side));
                        ui.painter().add(egui::Shadow { offset: [2, 2], blur: 5, spread: 0, color: Color32::from_black_alpha(140) }.as_shape(card, 1.0));
                        match self.unit_pic_of(ctx, vfs, game, kinds[i].0, card.width(), owner) {
                            Some(pic) => pic.paint(ui.painter(), card, if i == 0 { Color32::WHITE } else { Color32::from_gray(170) }),
                            None => {
                                ui.painter().rect_filled(card, CornerRadius::ZERO, INSET);
                            }
                        }
                        ui.painter().add(pic_frame(card, false));
                    }
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let title = if many == 1 {
                        name.clone()
                    } else if all_same {
                        tf("{name} × {n}", &[("name", &name), ("n", &many)])
                    } else {
                        tf("{n} units", &[("n", &many)])
                    };
                    embossed_wrapped(ui, &title, strong(14.0), CREAM);
                    let about = if all_same {
                        let d = ty.def.description.trim();
                        (!d.is_empty() && !d.eq_ignore_ascii_case(&name)).then(|| d.to_string())
                    } else {
                        Some(tf("{n} kinds of unit", &[("n", &kinds.len())]))
                    };
                    if let Some(about) = about {
                        ui.label(RichText::new(about).font(body(11.5)).color(STEEL));
                    }
                    match &doing {
                        Some((text, color)) => {
                            ui.label(RichText::new(text).font(strong(11.5)).color(*color));
                        }
                        None => {
                            let mut parts = Vec::new();
                            if idle > 0 {
                                parts.push(tf("{n} idle", &[("n", &idle)]));
                            }
                            if damaged > 0 {
                                parts.push(tf("{n} damaged", &[("n", &damaged)]));
                            }
                            if duty_sel.idle > 0 {
                                parts.push(tf("{n} on duty ({idle} idle)", &[("n", &duty_sel.on_duty), ("idle", &duty_sel.idle)]));
                            } else if duty_sel.on_duty > 0 {
                                parts.push(tf("{n} on duty", &[("n", &duty_sel.on_duty)]));
                            }
                            if !parts.is_empty() {
                                ui.label(RichText::new(parts.join("  ·  ")).font(strong(11.5)).color(STEEL));
                            }
                        }
                    }
                    // A constructor sent to work by itself.
                    if many == 1
                        && let Some(duty) = game.duty_of(first)
                    {
                        ui.label(RichText::new(tf("ON DUTY · {duty}", &[("duty", &t(crate::game::duty::label(duty)))])).font(strong(11.5)).color(SKY));
                    }
                    // Health (a group's together).
                    ui.add_space(2.0);
                    let fraction = if max_health > 0.0 { (health / max_health).clamp(0.0, 1.0) } else { 1.0 };
                    let (bar, _) = ui.allocate_exact_size(EVec2::new(ui.available_width(), 16.0), egui::Sense::hover());
                    ui.painter().add(hud_skin::gauge(bar, fraction, hud_skin::health_color(fraction), 1));
                    let text = ta_ui::widgets::galley(ui, RichText::new(format!("{health:.0} / {max_health:.0}")).font(strong(11.5)));
                    metal::slab_text(ui.painter(), bar.center(), text, true, TEXT_BRIGHT, false);
                });
            });
            if !facts.is_empty() {
                card.response.on_hover_text(facts.join("\n"));
            }
            // Rank: three stars, lit as far as it has come.
            if let Some((rank, kills)) = rank {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    let (r, _) = ui.allocate_exact_size(EVec2::new(40.0, 14.0), egui::Sense::hover());
                    for i in 0..3u8 {
                        let lit = i < rank;
                        star(ui.painter(), Pos2::new(r.left() + 7.0 + 13.0 * f32::from(i), r.center().y), 5.5, if lit { AMBER_BRIGHT } else { Color32::from_black_alpha(90) }, if lit { AMBER_DIM } else { STEEL_DIM });
                    }
                    let title = t([n("Recruit"), n("Veteran"), n("Hardened"), n("Elite")][usize::from(rank.min(3))]);
                    let line = if kills == 1 { tf("{rank}  ·  1 kill", &[("rank", &title)]) } else { tf("{rank}  ·  {n} kills", &[("rank", &title), ("n", &kills)]) };
                    ui.label(RichText::new(line).font(strong(11.5)).color(ACCENT));
                });
            }
            if let Some((text, next)) = stockpile {
                ui.add_space(2.0);
                let (bar, _) = ui.allocate_exact_size(EVec2::new(ui.available_width(), 15.0), egui::Sense::hover());
                ui.painter().add(hud_skin::gauge(bar, next, Color32::from_rgb(200, 120, 60), 4));
                let galley = ta_ui::widgets::galley(ui, RichText::new(text).font(strong(10.5)));
                metal::slab_text(ui.painter(), bar.center(), galley, true, TEXT_BRIGHT, false);
            }
            if let Some((aboard, room)) = cargo {
                ui.label(RichText::new(tf("Cargo: {n} of {room}", &[("n", &aboard), ("room", &room)])).font(strong(11.5)).color(ACCENT));
            }
            ui.add_space(3.0);
        }
        if part == Part::Group {
            // Mixed selections: one picture per type with its count; a click keeps only that
            // type, Shift+click drops it.
            if !all_same {
                let mut keep: Option<(usize, bool)> = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = EVec2::new(2.0, 2.0);
                    for &(kind, k) in kinds.iter().take(24) {
                        let size = EVec2::splat(34.0);
                        let label = crate::chat::unit_label(&game.sim, kind);
                        let resp = match self.unit_pic_of(ctx, vfs, game, kind, size.x, owner) {
                            Some(p) => pic_button(ui, &p, size),
                            None => ui.add_sized(size, egui::Button::new(RichText::new(&label).size(8.0))),
                        };
                        count(ui, resp.rect.right_bottom() - EVec2::new(3.0, 2.0), Align2::RIGHT_BOTTOM, k, 12.0);
                        let resp = resp.on_hover_text(tf("{name}: click to keep only these, Shift+click to drop them", &[("name", &label)]));
                        if resp.clicked() {
                            keep = Some((kind, ta_ui::widgets::click_modifiers(&resp, egui::PointerButton::Primary).shift));
                        }
                    }
                });
                if let Some((kind, drop)) = keep {
                    let sim = &game.sim;
                    game.selection.retain(|&id| sim.unit(id).is_some_and(|o| (o.type_id == kind) != drop));
                    return;
                }
            }
            // Narrow a bigger selection down: four switches across the room, or two rows of two
            // when their names do not fit four across (a long translation).
            if many > 1 {
                use crate::game::SelectionFilter as F;
                let mut chosen = None;
                let filters = [
                    (F::Combat, n("COMBAT"), n("Keep only the armed units")),
                    (F::Builders, n("BUILDERS"), n("Keep only the constructors")),
                    (F::Damaged, n("DAMAGED"), n("Keep only the damaged units")),
                    (F::Idle, n("IDLE"), n("Keep only the units with nothing to do")),
                ];
                let painter = ui.painter().clone();
                let fit = |across: usize| {
                    let width = ((ui.available_width() - 4.0 * (across - 1) as f32) / across as f32).floor();
                    let size = crate::hud_icons::family_size(&painter, filters.iter().map(|&(_, label, _)| (t(label), width - 8.0)));
                    let fits = filters.iter().all(|&(_, label, _)| painter.layout_no_wrap(t(label).to_string(), strong(size), Color32::WHITE).size().x <= width - 8.0);
                    (width, size, fits)
                };
                let (across, (width, size, _)) = match fit(4) {
                    four @ (_, _, true) => (4, four),
                    _ => (2, fit(2)),
                };
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing = EVec2::new(4.0, 4.0);
                    for row in filters.chunks(across) {
                        ui.horizontal(|ui| {
                            for &(f, label, tip) in row {
                                if filter_chip(ui, t(label), width, size).on_hover_text(t(tip)).clicked() {
                                    chosen = Some(f);
                                }
                            }
                        });
                    }
                });
                if let Some(f) = chosen {
                    game.filter_selection(f);
                    return;
                }
            }
        }
        if part == Part::Group {
            // A factory's queue in order: the unit in production first with its progress; a click
            // moves one up to be built next, right-click removes it.
            if ty.is_factory && !queue.is_empty() {
                ui.add_space(4.0);
                section(ui, "QUEUE");
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = EVec2::new(2.0, 2.0);
                    for (i, &q) in queue.iter().take(14).enumerate() {
                        let size = EVec2::splat(30.0);
                        let label = crate::chat::unit_label(&game.sim, q);
                        let resp = match self.unit_pic_of(ctx, vfs, game, q, size.x, owner) {
                            Some(p) => pic_button(ui, &p, size),
                            None => ui.add_sized(size, egui::Button::new(RichText::new(&label).size(8.0))),
                        };
                        if i == 0 {
                            let r = resp.rect;
                            let done = producing.map_or(0.0, |(_, p)| p);
                            ui.painter().rect_filled(Rect::from_min_size(Pos2::new(r.min.x, r.max.y - 3.0), EVec2::new(r.width() * done, 3.0)), 0.0, GREEN);
                        }
                        let resp = resp.on_hover_text(tf("{name}: click to build it next, right-click to remove it", &[("name", &label)]));
                        if resp.secondary_clicked() {
                            game.issue(Command::Produce { player: game.local_player, factory: first, type_id: q, count: -1 });
                        } else if resp.clicked() && i > 0 {
                            game.issue(Command::EditQueue { player: game.local_player, unit: first, index: i as u32, type_id: q, pos: None, to_front: true });
                        }
                    }
                    if queue.len() > 14 {
                        ui.label(RichText::new(format!("+{}", queue.len() - 14)).font(strong(12.0)).color(TEXT));
                    }
                });
            }
            // A constructor's queued buildings: click one to do it next, right-click to drop it.
            if !ty.is_factory && ty.is_builder && many == 1 {
                let later = game.builder_queue(first);
                if later.len() > 1 {
                    ui.add_space(4.0);
                    section(ui, "BUILD QUEUE");
                    let mut edit = None;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = EVec2::new(2.0, 2.0);
                        for (i, &(k, q)) in later.iter().take(14).enumerate() {
                            let size = EVec2::splat(30.0);
                            let label = crate::chat::unit_label(&game.sim, q);
                            let resp = match self.unit_pic_of(ctx, vfs, game, q, size.x, owner) {
                                Some(p) => pic_button(ui, &p, size),
                                None => ui.add_sized(size, egui::Button::new(RichText::new(&label).size(8.0))),
                            };
                            if i == 1 {
                                self.anchors.queue = Some(resp.rect);
                            }
                            let resp = resp.on_hover_text(tf("{name}: click to build it next, right-click to drop it", &[("name", &label)]));
                            if resp.secondary_clicked() {
                                edit = Some((k, false));
                            } else if resp.clicked() && i > 0 {
                                edit = Some((k, true));
                            }
                        }
                        if later.len() > 14 {
                            ui.label(RichText::new(format!("+{}", later.len() - 14)).font(strong(12.0)).color(TEXT));
                        }
                    });
                    if let Some((k, to_front)) = edit {
                        game.edit_builder_queue(first, k, to_front);
                    }
                }
            }
            // One kind with nothing queued: what it is good for at a glance, and its key for the
            // information page.
            let queues = if ty.is_factory { !queue.is_empty() } else { ty.is_builder && many == 1 && game.builder_queue(first).len() > 1 };
            if all_same && !queues {
                ui.add_space(4.0);
                let mut more = false;
                section_with(ui, n("DETAILS"), |ui| {
                    more = key_button(ui, game.keys.text("help")).on_hover_text(t("Information on the selected unit (with none, the controls reference)")).clicked();
                });
                let figures = super::info::summary_figures(game, &ty);
                let width = ui.available_width();
                let rows = super::info::figure_rows(ui.painter(), &figures, width);
                let (table, _) = ui.allocate_exact_size(EVec2::new(width, rows.len() as f32 * super::info::FIGURE_ROW), egui::Sense::hover());
                super::info::paint_figures(ui, table.min, width, &figures, &rows);
                let economy = super::info::economy_lines(&ty);
                if !economy.is_empty() {
                    ui.add_space(2.0);
                    ui.label(RichText::new(economy.join("  ·  ")).font(body(11.5)).color(STEEL));
                }
                if more {
                    game.unit_info = Some(crate::game::UnitInfo { type_id: ty.id, unit: (many == 1).then_some(first) });
                }
            }
        }
        if part == Part::Build && all_same && !ty.build_options.is_empty() && !game.spectator {
            ui.add_space(4.0);
            let mut repeat = repeat_on;
            let mut repeat_changed = false;
            // (A constructor's tabs say what they hold; a factory's list gets its caption and
            // the repeat switch.)
            if ty.is_factory {
                section_with(ui, n("PRODUCTION"), |ui| {
                    let switch = ui.checkbox(&mut repeat, t("Repeat"));
                    self.anchors.repeat = Some(switch.rect);
                    repeat_changed = switch.on_hover_text(t("Finished units go back on the end of the queue")).changed();
                });
            } else {
                ui.add_space(2.0);
            }
            if repeat_changed {
                game.set_repeat(repeat);
            }
            if let Some((opt, secondary, shift)) = self.build_menu(ui, ctx, vfs, game, &ty, &queued, producing) {
                if ty.is_factory {
                    game.produce(opt, if secondary { -1 } else { 1 } * if shift { 5 } else { 1 });
                } else if !secondary {
                    game.begin_placement(opt);
                }
            }
        }
    }

    /// The orders menu: under their own caption the orders the selection can give, each with its
    /// key, in rows by kind (moving and fighting; work; the special ones), as tall as the room
    /// lets them be; and beside them under STANCE the standing orders in words, what each is set
    /// to on the button and what it does on the pointer (a click changes it for the whole
    /// selection), self-destruct last, apart and in rust. A narrow plate keeps the stance as a
    /// column of glyphs alone (`stance` is its width).
    fn order_buttons(&mut self, ui: &mut egui::Ui, game: &mut Game, stance: f32) {
        use crate::hud_icons::{Glyph, order_button, order_slots, stance_button, state_icon_button};
        use crate::orders::Kind;
        // Every order the selection can give, in rows by group.
        let defs = crate::orders::available(game);
        const GAP: f32 = ORDER_GAP;
        let room = ui.available_rect_before_wrap();
        let wide = stance > STATES + 1.0;
        let grid = Rect::from_min_max(room.min, Pos2::new(room.right() - stance - 2.0 * GAP - 1.0, room.bottom()));
        let mut left = ui.new_child(egui::UiBuilder::new().max_rect(grid).layout(egui::Layout::top_down(egui::Align::Min)));
        left.spacing_mut().item_spacing = EVec2::new(GAP, GAP);
        section(&mut left, "ORDERS");
        let slots = order_slots(grid.width());
        let rows = crate::orders::rows(&defs, slots);
        // (As wide as in a full row: a building's lone Stop is a button, not a bar.)
        let button = ((grid.width() - GAP * (slots - 1) as f32) / slots as f32).floor();
        let breaks = rows.windows(2).filter(|w| w[0][0].group != w[1][0].group).count() as f32;
        let height = ((left.available_height() - 2.0 * breaks + GAP) / rows.len().max(1) as f32 - GAP).floor().clamp(30.0, 40.0);
        let mut chosen = None;
        for (i, row) in rows.iter().enumerate() {
            // The groups a little apart.
            if i > 0 && rows[i - 1][0].group != row[0].group {
                left.add_space(2.0);
            }
            left.horizontal(|ui| {
                for def in row {
                    let on = matches!(def.kind, Kind::Mode(m) if game.command_mode == Some(m));
                    let resp = order_button(ui, def.glyph, on, &def.key(game), button, height);
                    self.anchors.order(def.action, resp.rect);
                    if resp.on_hover_text(def.tip(game)).clicked() {
                        chosen = Some(def.kind);
                    }
                }
            });
        }
        match chosen {
            Some(Kind::Stop) => {
                game.stop_selection();
                game.command_mode = None;
            }
            Some(Kind::Mode(mode)) => game.set_command_mode(if game.command_mode == Some(mode) { None } else { Some(mode) }),
            Some(Kind::Perspective(id)) => game.perspective_request = Some(id),
            None => {}
        }
        let Some(u) = game.selection.first().and_then(|&id| game.sim.unit(id)) else { return };
        // Each standing order shows when any unit of the selection has it (the keys set it for
        // all of them), as the first such unit has it set.
        let units: Vec<&ta_sim::Unit> = game.selection.iter().filter_map(|&id| game.sim.unit(id)).collect();
        let first = |f: &dyn Fn(&ta_sim::UnitType) -> bool| units.iter().copied().find(|x| f(game.sim.unit_type(x)));
        let gunner = first(&|t| t.weapons.iter().any(|w| w.is_some()));
        let mover = first(&|t| t.weapons.iter().any(|w| w.is_some()) && t.locomotion != Locomotion::Static);
        let cloaker = first(&|t| t.def.cloak_cost > 0.0);
        let switch = first(&|t| t.def.on_offable);
        let (armed, mobile) = (gunner.is_some(), mover.is_some());
        let fire = gunner.map_or(0, |x| x.fire_state);
        let movement = mover.map_or(0, |x| x.move_state);
        let cloak_on = cloaker.is_some_and(|x| x.cloak_on);
        let active = switch.is_some_and(|x| x.activated);
        let counting = u.self_destruct_at.is_some() && u.self_destruct_manual;
        let cloak = cloaker.map(|x| {
            let ty = game.sim.unit_type(x);
            let (cost, moving) = (format!("{:.0}", ty.def.cloak_cost), format!("{:.0}", ty.def.cloak_cost_moving.max(ty.def.cloak_cost)));
            let args: [(&str, &dyn std::fmt::Display); 3] = [("key", &game.keys.text("cloak")), ("cost", &cost), ("moving", &moving)];
            if cloak_on { tf("Cloaked ({key}): click to show yourself. Cloaking costs {cost} energy/s ({moving} while moving)", &args) } else { tf("Visible ({key}): click to cloak, for {cost} energy/s ({moving} while moving)", &args) }
        });
        let destruct = if counting {
            t("Counting down: click to cancel").to_string()
        } else {
            tf("Self-destruct: blow up the selection after a five-second countdown ({key}); again to cancel", &[("key", &game.keys.text("self_destruct"))])
        };
        let on_offable = switch.is_some();
        /// What a standing order's button does.
        enum State {
            Fire,
            Move,
            Cloak,
            Power,
            Duty,
            Destruct,
        }
        /// A standing order's button: what it does, its glyph, what it is set to and what that
        /// does, lit (a cloak that is on), rusted (a countdown running), which of its settings it
        /// is at.
        struct Button {
            state: State,
            glyph: Glyph,
            label: String,
            tip: String,
            lit: bool,
            warn: bool,
            steps: Option<(u8, u8)>,
        }
        let mut states: Vec<Button> = Vec::new();
        if armed {
            let (glyph, label, what) = [
                (Glyph::HoldFire, n("HOLD FIRE"), n("Hold fire: never shoots")),
                (Glyph::ReturnFire, n("RETURN FIRE"), n("Return fire: shoots back when shot at")),
                (Glyph::FireAtWill, n("FIRE AT WILL"), n("Fire at will: shoots whatever comes in range")),
            ][usize::from(fire.min(2))];
            let tip = format!("{}\n{}", t(what), tf("Click for the next fire order ({key})", &[("key", &game.keys.text("fire_orders"))]));
            states.push(Button { state: State::Fire, glyph, label: t(label).to_string(), tip, lit: false, warn: false, steps: Some((fire.min(2), 3)) });
        }
        if armed && mobile {
            let (glyph, label, what) = [
                (Glyph::HoldPosition, n("HOLD POSITION"), n("Hold position: stays where it is")),
                (Glyph::Maneuver, n("MANEUVER"), n("Maneuver: moves to answer fire nearby")),
                (Glyph::Roam, n("ROAM"), n("Roam: chases the enemy anywhere")),
            ][usize::from(movement.min(2))];
            let tip = format!("{}\n{}", t(what), tf("Click for the next move order ({key})", &[("key", &game.keys.text("move_orders"))]));
            states.push(Button { state: State::Move, glyph, label: t(label).to_string(), tip, lit: false, warn: false, steps: Some((movement.min(2), 3)) });
        }
        if let Some(tip) = cloak {
            let (glyph, label) = if cloak_on { (Glyph::Cloaked, t("CLOAKED")) } else { (Glyph::Visible, t("VISIBLE")) };
            states.push(Button { state: State::Cloak, glyph, label: label.to_string(), tip, lit: cloak_on, warn: false, steps: None });
        }
        if on_offable {
            let tip = if active { tf("On: click to turn off ({key})", &[("key", &game.keys.text("on_off"))]) } else { tf("Off: click to turn on ({key})", &[("key", &game.keys.text("on_off"))]) };
            states.push(Button { state: State::Power, glyph: Glyph::Power, label: if active { t("ON") } else { t("OFF") }.to_string(), tip, lit: active, warn: false, steps: None });
        }
        // The constructors' duty: what they do by themselves (a menu of them opens from it).
        if let Some(duty) = game.duty_control() {
            states.push(Button { state: State::Duty, glyph: Glyph::Duty, label: duty.label, tip: duty.tip, lit: duty.lit, warn: false, steps: None });
        }
        // Self-destruct last, quiet (red while it counts down).
        let label = if counting { t("CANCEL") } else { t("SELF-DESTRUCT") };
        states.push(Button { state: State::Destruct, glyph: Glyph::SelfDestruct, label: label.to_string(), tip: destruct, lit: false, warn: counting, steps: None });
        ui.painter().add(metal::seam_down(grid.right() + GAP + 0.5, room.y_range().shrink(4.0)));
        let column = Rect::from_min_max(Pos2::new(room.right() - stance, room.top()), room.max);
        let mut right = ui.new_child(egui::UiBuilder::new().max_rect(column).layout(egui::Layout::top_down(egui::Align::Min)));
        right.spacing_mut().item_spacing = EVec2::new(GAP, GAP);
        if wide {
            section(&mut right, "STANCE");
        }
        let apart = if wide { 3.0 } else { 0.0 };
        let tall = ((right.available_height() - apart + GAP) / states.len() as f32 - GAP).floor().clamp(if states.len() > 4 { 19.0 } else { 22.0 }, 30.0);
        // The lettering one size for them all, in the room each leaves it: its glyph and lamps
        // out of the button's width.
        let text_size = if wide {
            let painter = right.painter().clone();
            crate::hud_icons::family_size(&painter, states.iter().map(|b| (b.label.as_str(), stance - 18.0 - 6.0 * 3.0 - 20.0)))
        } else {
            10.5
        };
        let mut clicked = None;
        let mut duty_button = None;
        let last = states.len() - 1;
        for (i, b) in states.iter().enumerate() {
            if wide && i == last && i > 0 {
                right.add_space(apart);
            }
            let resp = if wide {
                stance_button(&mut right, b.glyph, &b.label, EVec2::new(stance, tall), b.lit, b.warn, b.steps, text_size)
            } else {
                state_icon_button(&mut right, b.glyph, EVec2::new(stance, tall), b.lit, b.warn, b.steps)
            };
            if matches!(b.state, State::Fire) {
                self.anchors.order("fire_state", resp.rect);
            }
            if matches!(b.state, State::Duty) {
                duty_button = Some(resp.clone());
            }
            if resp.on_hover_text(format!("{}\n{}", b.label, b.tip)).clicked() {
                clicked = Some(&b.state);
            }
        }
        match clicked {
            Some(State::Fire) => game.set_states(Some((fire + 1) % 3), None),
            Some(State::Move) => game.set_states(None, Some((movement + 1) % 3)),
            Some(State::Cloak) => game.toggle_cloak(),
            Some(State::Power) => game.set_active(!active),
            Some(State::Destruct) => game.self_destruct_selection(),
            // (The duty button opens its menu by itself.)
            Some(State::Duty) | None => {}
        }
        if let Some(button) = duty_button {
            duty_menu(&button, game);
        }
    }

    /// The build menu: a factory's units in TA's order, a constructor's buildings by purpose on
    /// tabs; each picture with its costs along the foot, in whole rows as large as the room lets
    /// them be (see [`BuildGrid::fit`]), a page at a time when they do not all fit. Returns a
    /// click.
    #[allow(clippy::too_many_arguments)]
    fn build_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, vfs: &Vfs, game: &Game, builder: &UnitType, queued: &HashMap<usize, usize>, producing: Option<(usize, f32)>) -> Option<BuildClick> {
        let options: Vec<usize> = builder.build_options.iter().copied().filter(|&o| game.sim.is_allowed(o)).collect();
        let groups: Vec<(Option<Purpose>, Vec<usize>)> = if builder.is_factory {
            vec![(None, options)]
        } else {
            Purpose::ALL
                .iter()
                .map(|&p| (Some(p), options.iter().copied().filter(|&o| Purpose::of(game.sim.types.get(o)) == p).collect::<Vec<_>>()))
                .filter(|(_, list)| !list.is_empty())
                .collect()
        };
        let placing = game.placement.as_ref().map(|p| p.type_id);
        // A constructor's buildings one purpose at a time, on tabs (the one last looked at for
        // that kind of builder, or the one holding what is being placed).
        let tabs: Vec<Purpose> = groups.iter().filter_map(|(p, _)| *p).collect();
        let mut shown = self.build_tab.get(&builder.id).copied().filter(|p| tabs.contains(p)).or_else(|| tabs.first().copied());
        // Placing something picked elsewhere (a key, the classic panel): its tab, once.
        if placing != self.build_tab_placing {
            self.build_tab_placing = placing;
            if let Some(pl) = placing.map(|o| Purpose::of(game.sim.types.get(o))).filter(|p| tabs.contains(p)) {
                shown = Some(pl);
            }
        }
        if tabs.len() > 1 {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let width = ((ui.available_width() - 3.0 * (tabs.len() - 1) as f32) / tabs.len() as f32).floor();
                let lettering = tab_lettering(ui, &tabs.iter().map(|p| t(p.short())).collect::<Vec<_>>(), width);
                for &p in &tabs {
                    let resp = build_tab(ui, t(p.short()), width, lettering, shown == Some(p));
                    if shown != Some(p) {
                        let holds = groups.iter().find(|(g, _)| *g == Some(p)).map(|(_, list)| list.clone()).unwrap_or_default();
                        self.anchors.tab(resp.rect, holds);
                    }
                    if resp.on_hover_text(t(p.label())).clicked() {
                        shown = Some(p);
                    }
                }
            });
            ui.add_space(3.0);
        }
        if let Some(p) = shown {
            self.build_tab.insert(builder.id, p);
        }
        // The pictures of the tab shown, laid out in the room left: whole rows only, as large as
        // fits them all, else a page of them at a time over a pager; laid out for the fullest
        // tab, so that they stay as they are from tab to tab.
        let list: Vec<usize> = groups.iter().filter(|(p, _)| p.is_none() || *p == shown).flat_map(|(_, list)| list.iter().copied()).collect();
        let fullest = groups.iter().map(|(_, list)| list.len()).max().unwrap_or(0);
        let grid = BuildGrid::fit(fullest, ui.available_width(), ui.available_height());
        let per_page = grid.columns * grid.rows;
        let pages = list.len().div_ceil(per_page).max(1);
        // The page: the first for another builder or tab; the one holding what is being placed
        // when that starts (as the tab turns to it).
        let key = (builder.id, shown);
        if self.build_page.0 != key {
            self.build_page = (key, 0);
        }
        if placing != self.build_page_placing {
            self.build_page_placing = placing;
            if let Some(i) = placing.and_then(|o| list.iter().position(|&q| q == o)) {
                self.build_page.1 = i / per_page;
            }
        }
        let mut page = self.build_page.1.min(pages - 1);
        self.anchors.pages(list.iter().enumerate().map(|(i, &o)| (o, i / per_page)), page);
        let mut click = None;
        // The costs lettered in one size across the menu, every tab's (fitted picture by picture,
        // the larger figures came out smaller than their neighbours').
        let costs: Vec<(f32, f32)> = groups.iter().flat_map(|(_, list)| list.iter()).map(|&o| game.sim.types.get(o)).map(|k| (k.cost_metal.to_f32(), k.cost_energy.to_f32())).collect();
        let cost_size = cost_lettering(ui.painter(), &costs, grid.tile);
        let width = grid.columns as f32 * (grid.tile + GRID_GAP) - GRID_GAP;
        let indent = ((ui.available_width() - width) / 2.0).floor().max(0.0);
        let shown_rows = ui.scope(|ui| {
            ui.spacing_mut().item_spacing = EVec2::new(GRID_GAP, GRID_GAP);
            for row in list.chunks(grid.columns).skip(page * grid.rows).take(grid.rows) {
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    for &opt in row {
                        let on_pad = producing.filter(|&(q, _)| q == opt).map(|(_, p)| p);
                        if let Some(c) = self.build_tile(ui, ctx, vfs, game, builder, opt, queued.get(&opt).copied(), on_pad, placing == Some(opt), grid.tile, cost_size) {
                            click = Some(c);
                        }
                    }
                });
            }
        });
        if pages > 1 {
            // The pager across the foot, a page's height down whatever the last page holds; the
            // wheel over the pictures turns the pages too.
            let top = shown_rows.response.rect.top() + grid.rows as f32 * (grid.tile + GRID_GAP);
            ui.add_space((top - ui.cursor().top()).max(0.0));
            let (turn, prev, next) = pager(ui, page, pages);
            self.anchors.pager(prev, next);
            let over = Rect::from_min_max(shown_rows.response.rect.min, Pos2::new(shown_rows.response.rect.right(), top));
            // (In notches: a trackpad's points add up to them.)
            let wheel: f32 = ui.input(|i| {
                i.events
                    .iter()
                    .map(|e| match e {
                        egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, .. } => delta.y / WHEEL_STEP,
                        egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta, .. } => delta.y,
                        egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Page, delta, .. } => delta.y * 3.0,
                        _ => 0.0,
                    })
                    .sum()
            });
            self.build_wheel = if ui.rect_contains_pointer(over) { self.build_wheel + wheel } else { 0.0 };
            let step = if self.build_wheel <= -1.0 { 1 } else if self.build_wheel >= 1.0 { -1 } else { 0 };
            if step != 0 {
                self.build_wheel = 0.0;
            }
            match turn.unwrap_or(step) {
                1 if page + 1 < pages => page += 1,
                -1 if page > 0 => page -= 1,
                _ => {}
            }
        }
        self.build_page.1 = page;
        click
    }

    /// One picture in the build menu: costs along its foot (lettered `cost_size`), how many are
    /// queued, the progress of the one on the pad, brass while it is being placed.
    #[allow(clippy::too_many_arguments)]
    fn build_tile(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, vfs: &Vfs, game: &Game, builder: &UnitType, opt: usize, queued: Option<usize>, on_pad: Option<f32>, placing: bool, tile: f32, cost_size: f32) -> Option<BuildClick> {
        let kind = game.sim.types.get(opt);
        let label = crate::chat::unit_label(&game.sim, opt);
        let size = EVec2::splat(tile);
        let resp = match self.unit_pic(ctx, vfs, game, opt, tile) {
            Some(p) => pic_button(ui, &p, size),
            None => ui.add_sized(size, egui::Button::new(RichText::new(&label).size(10.0)).wrap()),
        };
        let rect = resp.rect;
        self.anchors.picture(opt, rect);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            cost_strip(painter, rect, kind.cost_metal.to_f32(), kind.cost_energy.to_f32(), cost_size);
            if let Some(done) = on_pad {
                let bar = Rect::from_min_size(rect.min + EVec2::new(1.0, 1.0), EVec2::new(rect.width() - 2.0, 4.0));
                painter.rect_filled(bar, CornerRadius::ZERO, Color32::from_black_alpha(170));
                painter.rect_filled(Rect::from_min_size(bar.min, EVec2::new(bar.width() * done, bar.height())), CornerRadius::ZERO, GREEN);
            }
            if placing {
                painter.rect_stroke(rect.shrink(1.0), CornerRadius::same(1), Stroke::new(2.0, AMBER_BRIGHT), egui::StrokeKind::Inside);
                painter.add(metal::lamp(rect.left_top() + EVec2::new(8.0, 8.0), 3.0, GREEN, 1.0));
            }
            if let Some(k) = queued {
                count(ui, rect.right_top() + EVec2::new(-4.0, 5.0), Align2::RIGHT_TOP, k, 17.0);
            }
        }
        // How long this builder takes on its own, resources allowing.
        let secs = if builder.worker_rate > Fx::ZERO { (kind.build_time / builder.worker_rate).to_f32() / 30.0 } else { 0.0 };
        let mut tip = vec![label.clone()];
        if !kind.def.description.trim().is_empty() {
            tip.push(kind.def.description.trim().to_string());
        }
        let (metal_cost, energy_cost, time) = (format!("{:.0}", kind.cost_metal.to_f32()), format!("{:.0}", kind.cost_energy.to_f32()), format!("{:.0}", kind.build_time.to_f32()));
        tip.push(tf(
            "Metal {metal}  ·  Energy {energy}  ·  Build time {time} ({duration})",
            &[("metal", &metal_cost), ("energy", &energy_cost), ("time", &time), ("duration", &duration(secs))],
        ));
        tip.extend(unit_facts(&game.sim, kind));
        if builder.is_factory {
            tip.push(t("Click: one more  ·  Shift+click: five  ·  Right-click: fewer").to_string());
        }
        let resp = resp.on_hover_text(tip.join("\n"));
        if resp.clicked() {
            Some((opt, false, ta_ui::widgets::click_modifiers(&resp, egui::PointerButton::Primary).shift))
        } else if resp.secondary_clicked() {
            Some((opt, true, ta_ui::widgets::click_modifiers(&resp, egui::PointerButton::Secondary).shift))
        } else {
            None
        }
    }
}

/// Which of the selection's parts to draw (see [`Hud::selection_contents`]): its card (picture,
/// health, rank), its group (the kinds of a mixed selection, the switches that narrow it, its
/// queue), its build menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Head,
    Group,
    Build,
}

/// Where the deck's plates stand this frame: the forces' plate at the left and how many columns
/// of groups it holds, the middle plate, the command plate; and how wide the command plate's
/// parts are (none for none): its orders with the stance beside them, the stance alone, and the
/// build menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DeckLayout {
    pub forces: Option<Rect>,
    pub force_columns: usize,
    pub middle: Option<Rect>,
    pub command: Option<Rect>,
    pub orders: f32,
    pub stance: f32,
    pub build: f32,
}

/// The deck on `screen`: the forces' plate at the left when there are `forces` groups to show
/// (none: no plate); the command plate at the right when `commanding`, its orders and stance
/// beside its build menu `build` wide (none for none, or as wide as the room leaves the middle at
/// its narrowest, the build menu at three pictures across at the least); the middle plate between
/// them when something is `selected`, no wider than [`MIDDLE_MOST`] and centred on the screen
/// where it can be (left out when even narrower than its least).
///
/// Where the room is short the plates give up their words in turn, the least needed first: the
/// stance goes from words ([`STANCE`]) to glyphs ([`STATES`]), then the forces' columns narrow
/// ([`FORCE_COLUMN_NARROW`]), until the selection's plate has [`MIDDLE_WANTED`] (or none does).
pub(super) fn deck_layout(screen: Rect, forces: Option<usize>, selected: bool, commanding: bool, build: f32) -> DeckLayout {
    let deck = deck_height(screen);
    let top = screen.bottom() - GAP - deck;
    let force_columns = forces.map_or(1, |count| forces_columns(count, deck));
    let least_build = COLUMNS as f32 * TILE_MIN + (COLUMNS - 1) as f32 * GRID_GAP;
    let across = |stance: f32| ORDERS + stance + 2.0 * ORDER_GAP + 1.0;
    let command_width = |stance: f32, build: f32| across(stance) + if build > 0.0 { GAP + build } else { 0.0 } + 2.0 * PAD;
    // The plates in a row, the gaps between them and the screen's margins taken off.
    let plates = usize::from(forces.is_some()) + usize::from(selected) + usize::from(commanding);
    let room = screen.width() - 2.0 * GAP - GAP * plates.saturating_sub(1) as f32;
    let forces_wide = |narrow: bool| forces.map_or(0.0, |_| forces_width(force_columns, narrow));
    // The first of these to leave the selection its wanted room: (narrow forces, the stance's width).
    let (narrow, stance) = [(false, STANCE), (false, STATES), (true, STATES)]
        .into_iter()
        .find(|&(narrow, stance)| !selected || room - forces_wide(narrow) - if commanding { command_width(stance, build) } else { 0.0 } >= MIDDLE_WANTED)
        .unwrap_or((true, STATES));
    let plate = forces.map(|_| Rect::from_min_size(Pos2::new(screen.left() + GAP, top), EVec2::new(forces_wide(narrow), deck)));
    let left = plate.map_or(screen.left() + GAP, |p| p.right() + GAP);
    let (command, orders, stance, build) = if commanding {
        // What the command plate may take, the middle plate keeping its least.
        let orders = across(stance);
        let most = (screen.right() - GAP - left - MIDDLE_LEAST - GAP - 2.0 * PAD).max(orders + GAP + least_build);
        let build = if build > 0.0 { build.min(most - orders - GAP).max(least_build) } else { 0.0 };
        let width = command_width(stance, build);
        (Some(Rect::from_min_size(Pos2::new(screen.right() - GAP - width, top), EVec2::new(width, deck))), orders, stance, build)
    } else {
        (None, 0.0, 0.0, 0.0)
    };
    let right = command.map_or(screen.right() - GAP, |c| c.left() - GAP);
    let width = (right - left).min(MIDDLE_MOST);
    let middle = (selected && width >= MIDDLE_LEAST * 0.8).then(|| {
        let x = (screen.center().x - width / 2.0).clamp(left, right - width);
        Rect::from_min_size(Pos2::new(x, top), EVec2::new(width, deck))
    });
    DeckLayout { forces: plate, force_columns, middle, command, orders, stance, build }
}

/// The deck's height on `screen`: a quarter of it, between [`DECK_LEAST`] and [`DECK_MOST`].
pub(super) fn deck_height(screen: Rect) -> f32 {
    (screen.height() * 0.25).round().clamp(DECK_LEAST, DECK_MOST)
}

/// How many columns of groups the forces' plate holds in a deck `deck` points tall: one while
/// they fit one above another (rows of [`FORCE_ROW_LEAST`]), else two.
pub(super) fn forces_columns(count: usize, deck: f32) -> usize {
    let room = deck - 2.0 * PAD - FORCE_CAPTION;
    let rows = (((room + 3.0) / (FORCE_ROW_LEAST + 3.0)).floor() as usize).max(1);
    count.div_ceil(rows).clamp(1, 2)
}

/// How wide the forces' plate is with `columns` of groups, `narrow` ones where the room is short.
fn forces_width(columns: usize, narrow: bool) -> f32 {
    columns as f32 * if narrow { FORCE_COLUMN_NARROW } else { FORCE_COLUMN } + 4.0 * (columns - 1) as f32 + 2.0 * PAD
}

/// How much steel the minimap's plate keeps round its picture and the bezel on it (points).
const MAP_PAD: f32 = 3.0;
/// The minimap's bezel (see [`Hud::minimap_plate`]).
const MAP_BEZEL: f32 = 4.0;

/// The minimap's plate on `screen`, at the top right under the top bar (`bar_bottom`: its lower
/// edge): the map of shape `aspect` (its height over its width) as large as fits in a box a
/// quarter of the screen's height tall and a third as wide again, inside the steel round it.
pub(super) fn minimap_layout(screen: Rect, bar_bottom: f32, aspect: f32) -> Rect {
    let side = (screen.height() * 0.26).clamp(150.0, 320.0);
    let aspect = aspect.clamp(0.2, 5.0);
    let width = (side / aspect).min(side * 1.3).round();
    let height = (width * aspect).round();
    let frame = 2.0 * (MAP_PAD + MAP_BEZEL + 1.0);
    let size = EVec2::new(width + frame, height + frame);
    Rect::from_min_size(Pos2::new(screen.right() - GAP - size.x, bar_bottom.max(34.0) + GAP), size)
}

/// How the build menu's pictures are laid out: so many across, so big, so many rows to a page.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BuildGrid {
    columns: usize,
    tile: f32,
    rows: usize,
}

impl BuildGrid {
    /// `count` pictures in `width` by `height` points, in whole rows: of the layouts from three
    /// to [`COLUMNS_MOST`] across that show them all, the one with the largest pictures (up to
    /// [`TILE`]; of two alike, the fewer across); if none shows them all at [`TILE_MIN`] or more,
    /// a page of them at a time over a pager, as many to a page as fit at the largest size that
    /// keeps three across.
    fn fit(count: usize, width: f32, height: f32) -> BuildGrid {
        let count = count.max(1);
        let across = |columns: usize| ((width - GRID_GAP * (columns - 1) as f32) / columns as f32).floor().min(TILE);
        let all = (COLUMNS..=COLUMNS_MOST)
            .map(|columns| {
                let rows = count.div_ceil(columns);
                BuildGrid { columns, tile: across(columns).min(((height + GRID_GAP) / rows as f32 - GRID_GAP).floor()), rows }
            })
            .filter(|g| g.tile >= TILE_MIN)
            .fold(None, |best: Option<BuildGrid>, g| match best {
                Some(b) if b.tile >= g.tile => Some(b),
                _ => Some(g),
            });
        if let Some(grid) = all {
            return grid;
        }
        (COLUMNS..=COLUMNS_MOST)
            .map(|columns| {
                let tile = across(columns);
                let rows = (((height - PAGER + GRID_GAP) / (tile + GRID_GAP)).floor() as usize).max(1);
                BuildGrid { columns, tile, rows }
            })
            .filter(|g| g.tile >= TILE_MIN)
            .max_by(|a, b| (a.columns * a.rows).cmp(&(b.columns * b.rows)).then(a.tile.total_cmp(&b.tile)))
            .unwrap_or(BuildGrid { columns: COLUMNS, tile: TILE_MIN, rows: 1 })
    }
}

/// The build menu's pager: the page before, which of how many, the page after. Returns -1 or
/// 1 for a click on either, and the buttons that can be pressed (before, after).
fn pager(ui: &mut egui::Ui, page: usize, pages: usize) -> (Option<i32>, Option<Rect>, Option<Rect>) {
    let (rect, _) = ui.allocate_exact_size(EVec2::new(ui.available_width(), PAGER - GRID_GAP), egui::Sense::hover());
    let side = EVec2::new((rect.width() * 0.3).floor(), rect.height());
    let mut turn = None;
    let mut buttons = [None, None];
    for (step, r) in [(-1, Rect::from_min_size(rect.min, side)), (1, Rect::from_min_size(Pos2::new(rect.right() - side.x, rect.top()), side))] {
        let usable = if step < 0 { page > 0 } else { page + 1 < pages };
        if usable {
            buttons[usize::from(step > 0)] = Some(r);
        }
        let resp = ui.interact(r, ui.id().with(("build_pager", step)), if usable { egui::Sense::click() } else { egui::Sense::hover() });
        let hot = if usable && resp.hovered() { 1.0 } else { 0.0 };
        ui.painter().add(metal::slab(ui.ctx(), r, Slab::Steel, hot, usable && resp.is_pointer_button_down_on()));
        // A chevron up for the page before, down for the one after.
        let c = r.center() + EVec2::new(0.0, if resp.is_pointer_button_down_on() { 1.0 } else { 0.0 });
        let (w, h) = (6.0, if step < 0 { -3.0 } else { 3.0 });
        let colour = if usable { Slab::Steel.text(hot) } else { STEEL_DIM };
        let chevron = vec![Pos2::new(c.x - w, c.y - h), Pos2::new(c.x, c.y + h), Pos2::new(c.x + w, c.y - h)];
        ui.painter().add(egui::Shape::line(chevron, Stroke::new(2.0, colour)));
        if usable && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Button);
            turn = Some(step);
        }
    }
    // Which page, between them: a lamp for each, the one shown lit (a number past six).
    let middle = Rect::from_min_max(Pos2::new(rect.left() + side.x, rect.top()), Pos2::new(rect.right() - side.x, rect.bottom()));
    if pages <= 6 {
        let spacing = 11.0;
        let first = middle.center().x - spacing * (pages - 1) as f32 / 2.0;
        for k in 0..pages {
            ui.painter().add(metal::lamp(Pos2::new(first + spacing * k as f32, middle.center().y), 2.5, AMBER, if k == page { 1.0 } else { 0.0 }));
        }
    } else {
        ui.painter().text(middle.center(), Align2::CENTER_CENTER, format!("{} / {pages}", page + 1), strong(11.0), TEXT_BRIGHT);
    }
    (turn, buttons[0], buttons[1])
}

/// The menu the DUTY button opens: the four duties, each with what it does in a line and a lamp
/// on the one the selection is on, and under them "under my command" (none), as the radial menu's
/// ring has them (`game::duty::CHOICES`). A choice goes to the selected constructors.
fn duty_menu(button: &egui::Response, game: &mut Game) {
    let now = game.selection_duty();
    let mut chosen: Option<Option<Duty>> = None;
    egui::Popup::menu(button).width(310.0).show(|ui| {
        ui.set_width(310.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        for &(duty, glyph) in crate::game::duty::CHOICES.iter() {
            // (A little apart: the last is not a duty.)
            if duty.is_none() {
                ui.add_space(2.0);
            }
            let on = match duty {
                Some(_) => now.shared == duty,
                None => now.on_duty == 0,
            };
            if duty_row(ui, glyph, &t(crate::game::duty::choice_label(duty)), &t(crate::game::duty::choice_blurb(duty)), on).clicked() {
                chosen = Some(duty);
            }
        }
        ui.add_space(4.0);
        let hint = tf("Any order you give a constructor takes it off duty. {key} goes through the duties.", &[("key", &game.keys.text("duty"))]);
        ui.label(RichText::new(hint).font(body(10.5)).color(STEEL_DIM));
    });
    if let Some(duty) = chosen {
        game.set_duty(duty);
    }
}

/// One choice in the duty menu: its glyph, its name and a line on what it does, with a lamp when it
/// is the one the selection is on.
fn duty_row(ui: &mut egui::Ui, glyph: crate::hud_icons::Glyph, title: &str, about: &str, on: bool) -> egui::Response {
    const LEFT: f32 = 36.0;
    let width = ui.available_width();
    let galley = ui.painter().layout(about.to_string(), body(11.0), STEEL, width - LEFT - 26.0);
    let (rect, response) = ui.allocate_exact_size(EVec2::new(width, (26.0 + galley.size().y + 8.0).max(42.0)), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if response.hovered() {
            painter.rect_filled(rect, 3.0, AMBER_DEEP);
        } else if on {
            painter.rect_filled(rect, 3.0, Color32::from_white_alpha(12));
        }
        let ink = if on { AMBER_BRIGHT } else { CREAM };
        crate::hud_icons::paint(painter, Rect::from_center_size(Pos2::new(rect.left() + 18.0, rect.top() + 21.0), EVec2::splat(20.0)), glyph, ink);
        painter.text(Pos2::new(rect.left() + LEFT, rect.top() + 7.0), Align2::LEFT_TOP, title, strong(12.0), if on { AMBER_BRIGHT } else { TEXT_BRIGHT });
        painter.galley(Pos2::new(rect.left() + LEFT, rect.top() + 24.0), galley, STEEL);
        if on {
            painter.add(metal::lamp(Pos2::new(rect.right() - 12.0, rect.top() + 14.0), 2.6, GREEN, 1.0));
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A key as a small steel button (the information page's), lettered with it.
fn key_button(ui: &mut egui::Ui, key: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(key.to_uppercase(), strong(9.5), TEXT_BRIGHT);
    let (rect, resp) = ui.allocate_exact_size(EVec2::new(galley.size().x + 12.0, 16.0), egui::Sense::click());
    let hot = if resp.hovered() { 1.0 } else { 0.0 };
    let down = resp.is_pointer_button_down_on();
    ui.painter().add(metal::slab(ui.ctx(), rect, Slab::Steel, hot, down));
    metal::slab_text(ui.painter(), rect.center(), galley, true, Slab::Steel.text(hot), down);
    if resp.clicked() {
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Button);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One of the switches that narrow a selection down, `width` wide, lettered `size` points high
/// (the size the row shares).
fn filter_chip(ui: &mut egui::Ui, label: &str, width: f32, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(EVec2::new(width, 22.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = if response.hovered() { 1.0 } else { 0.0 };
        let down = response.is_pointer_button_down_on();
        ui.painter().add(metal::slab(ui.ctx(), rect, Slab::Steel, hot, down));
        let galley = crate::hud_icons::lettering(ui.painter(), label, size, rect.width() - 8.0, Slab::Steel.text(hot));
        metal::slab_text(ui.painter(), rect.center(), galley, true, Slab::Steel.text(hot), down);
    }
    if response.clicked() {
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Button);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The build menu's tabs' lettering, `width` wide each: its size and spacing, one for them all
/// (so that they match), fitted to the longest of `labels` with a margin either side: smaller,
/// then set closer, as the tabs narrow (an advanced constructor's five, a language with long
/// words); clipped at the last.
fn tab_lettering(ui: &egui::Ui, labels: &[&str], width: f32) -> (f32, f32) {
    let room = width - 8.0;
    let mut size = if width < 52.0 { 9.0 } else { 10.0 };
    loop {
        let spacing = if size > 8.5 { ta_ui::i18n::tracking(0.4) } else { 0.0 };
        let fits = labels.iter().all(|label| ta_ui::widgets::galley(ui, RichText::new(*label).font(strong(size)).extra_letter_spacing(spacing)).size().x <= room);
        if fits || size <= 7.0 {
            return (size, spacing);
        }
        size -= 0.5;
    }
}

/// A tab of the build menu: gunmetal, pressed in and lettered in brass when it is the one
/// shown; `lettering` is its size and spacing ([`tab_lettering`]).
fn build_tab(ui: &mut egui::Ui, label: &str, width: f32, (size, spacing): (f32, f32), on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(EVec2::new(width, 22.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = if response.hovered() { 1.0 } else { 0.0 };
        ui.painter().add(metal::slab(ui.ctx(), rect, Slab::Steel, hot, on || response.is_pointer_button_down_on()));
        let color = if on { AMBER_BRIGHT } else { Slab::Steel.text(hot) };
        let galley = ta_ui::widgets::galley(ui, RichText::new(label).font(strong(size)).extra_letter_spacing(spacing));
        metal::slab_text(&ui.painter().with_clip_rect(rect.shrink(1.0)), rect.center(), galley, true, color, on);
        if on {
            let bar = Rect::from_min_max(Pos2::new(rect.left() + 6.0, rect.bottom() - 3.0), Pos2::new(rect.right() - 6.0, rect.bottom() - 1.5));
            ui.painter().rect_filled(bar, CornerRadius::ZERO, AMBER);
        }
    }
    if response.clicked() && !on {
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Tab);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A caption in the panel, in brass, with a seam running on to the panel's edge.
pub(super) fn section(ui: &mut egui::Ui, title: &str) {
    section_with(ui, title, |_| {});
}

/// [`section`] with something at the end of its row.
fn section_with(ui: &mut egui::Ui, title: &str, right: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(ta_ui::widgets::caption(title).color(ACCENT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            right(ui);
            let (rect, _) = ui.allocate_exact_size(EVec2::new(ui.available_width(), 10.0), egui::Sense::hover());
            ui.painter().add(metal::seam(rect.x_range(), rect.center().y.round()));
        });
    });
}

/// Embossed lettering wrapped to the width there is (a long unit name).
fn embossed_wrapped(ui: &mut egui::Ui, text: &str, font: egui::FontId, color: Color32) {
    let galley = ui.painter().layout(text.to_string(), font, color, ui.available_width() - 2.0);
    let (rect, _) = ui.allocate_exact_size(galley.size() + EVec2::new(2.0, 2.0), egui::Sense::hover());
    metal::embossed(ui.painter(), rect.min + EVec2::new(1.0, 0.0), galley, color);
}

/// A five-pointed star (a rank).
fn star(painter: &egui::Painter, c: Pos2, r: f32, fill: Color32, edge: Color32) {
    let points: Vec<Pos2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            c + EVec2::new(a.cos(), a.sin()) * if i % 2 == 0 { r } else { r * 0.45 }
        })
        .collect();
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, fill);
    for &p in &points {
        mesh.colored_vertex(p, fill);
    }
    for i in 0..10u32 {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % 10);
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::closed_line(points, Stroke::new(1.0, edge)));
}

/// A cost as a build picture shows it.
fn short_cost(v: f32) -> String {
    if v >= 10_000.0 { format!("{:.0}k", v / 1000.0) } else { format!("{v:.0}") }
}

/// The size the build menu's costs are lettered in, on its pictures `tile` points across: the
/// largest at which each picture's two (`costs`, metal and energy) stay apart along its foot
/// (smaller on small pictures).
fn cost_lettering(painter: &egui::Painter, costs: &[(f32, f32)], tile: f32) -> f32 {
    let band = tile - 2.0;
    let wide = |size: f32, (metal_cost, energy_cost): (f32, f32)| [short_cost(metal_cost), short_cost(energy_cost)].into_iter().map(|s| painter.layout_no_wrap(s, strong(size), Color32::WHITE).size().x).sum::<f32>();
    // (The widest pair at full size is the widest at every size.)
    let Some((_, widest)) = costs.iter().map(|&c| (wide(10.5, c), c)).max_by(|a, b| a.0.total_cmp(&b.0)) else { return 10.5 };
    let mut size = 10.5;
    while size > 8.5 && wide(size, widest) + 6.0 + 6.0 > band {
        size -= 0.5;
    }
    size
}

/// The costs along the foot of a build picture, lettered `size` ([`cost_lettering`]): metal on
/// the left, energy on the right.
fn cost_strip(painter: &egui::Painter, rect: Rect, metal_cost: f32, energy_cost: f32, size: f32) {
    let band = Rect::from_min_max(Pos2::new(rect.left() + 1.0, rect.bottom() - 15.0), Pos2::new(rect.right() - 1.0, rect.bottom() - 1.0));
    painter.add(metal::vgrad(band, Color32::from_black_alpha(40), Color32::from_black_alpha(210)));
    let (metal_text, energy_text) = (short_cost(metal_cost), short_cost(energy_cost));
    let y = band.bottom() - 1.0;
    for (text, at, align, color) in [(metal_text, band.left() + 3.0, Align2::LEFT_BOTTOM, METAL), (energy_text, band.right() - 3.0, Align2::RIGHT_BOTTOM, ENERGY)] {
        painter.text(Pos2::new(at + 1.0, y + 1.0), align, &text, strong(size), Color32::from_black_alpha(230));
        painter.text(Pos2::new(at, y), align, &text, strong(size), color);
    }
}

/// Where to look at a group: the unit nearest its middle.
fn middle_of(game: &Game, ids: &[UnitId]) -> Option<Vec3> {
    let points: Vec<Vec3> = ids.iter().filter_map(|&id| game.sim.unit(id)).map(|u| Vec3::from_array(u.pos.to_f32())).collect();
    let mean = points.iter().copied().sum::<Vec3>() / points.len().max(1) as f32;
    points.into_iter().min_by(|a, b| a.distance_squared(mean).total_cmp(&b.distance_squared(mean)))
}

/// A share as a whole percentage, never 100 before it is done.
fn percent(v: Fx) -> String {
    format!("{:.0}", (v.to_f32() * 100.0).clamp(0.0, 100.0).floor())
}

/// What a unit is doing, in a few words, and the colour to say it in (the deck's card,
/// and Perspective's status screen).
pub(crate) fn activity(game: &Game, u: &Unit) -> (String, Color32) {
    let sim = &game.sim;
    // Another unit by name, when the player can see what it is.
    let known = |id: UnitId| sim.unit(id).filter(|o| o.owner == u.owner || game.seen(o) == Seen::Visible).map(|o| crate::chat::unit_label(sim, o.type_id));
    let named = |id: UnitId, text: &str, unseen: &str| known(id).map_or_else(|| t(unseen).to_string(), |name| tf(text, &[("name", &name)]));
    if !u.is_complete() {
        return (tf("Under construction: {pct}%", &[("pct", &percent(u.build_progress))]), AMBER);
    }
    if u.carried_by.is_some() {
        return (t("Aboard a transport").to_string(), TEXT);
    }
    if u.paralyzed_until > sim.tick {
        return (t("Paralyzed").to_string(), AMBER_BRIGHT);
    }
    if u.self_destruct_manual
        && let Some(at) = u.self_destruct_at
    {
        return (tf("Self-destructing in {secs} s", &[("secs", &at.saturating_sub(sim.tick).div_ceil(30))]), RED);
    }
    let ty = sim.unit_type(u);
    // (A unit taking a wreck or a tree apart works on itself, as far as its build job says.)
    if u.build.is_some_and(|job| job.target == u.id) {
        return (t("Reclaiming").to_string(), GREEN);
    }
    if let Some(job) = u.build
        && let Some(target) = sim.unit(job.target)
    {
        let what = crate::chat::unit_label(sim, target.type_id);
        let pct = percent(target.build_progress);
        let text = if target.is_complete() {
            tf("Repairing: {name}", &[("name", &what)])
        } else if ty.is_factory {
            tf("Producing: {name} {pct}%", &[("name", &what), ("pct", &pct)])
        } else {
            tf("Building: {name} {pct}%", &[("name", &what), ("pct", &pct)])
        };
        return (text, GREEN);
    }
    // (A constructor on duty that is getting away from enemies says so, not just that it moves.)
    if matches!(u.orders.front(), Some(Order::Move(_))) && game.duty_status(u.id) == Some(ta_ai::DutyStatus::GettingAway) {
        return (t("Getting away from enemies").to_string(), AMBER);
    }
    match u.orders.front() {
        Some(Order::Move(_)) => (t("Moving").to_string(), TEXT),
        Some(Order::Attack(Target::Unit(id)) | Order::ManualFire(Target::Unit(id))) => (named(*id, n("Attacking: {name}"), n("Attacking a radar contact")), FIGHTING),
        Some(Order::Attack(_) | Order::ManualFire(_)) => (t("Attacking the ground").to_string(), FIGHTING),
        Some(Order::AttackType(_)) => (t("Hunting").to_string(), FIGHTING),
        Some(Order::Fight(_)) => (t("Fighting its way forward").to_string(), FIGHTING),
        Some(Order::Patrol(_)) => (t("Patrolling").to_string(), TEXT),
        Some(Order::Guard(id)) => (named(*id, n("Guarding: {name}"), n("Guarding")), TEXT),
        Some(Order::Build { type_id, .. }) => (tf("On the way to build: {name}", &[("name", &crate::chat::unit_label(sim, *type_id))]), GREEN),
        Some(Order::Repair(id)) => (named(*id, n("Repairing: {name}"), n("Repairing")), GREEN),
        Some(Order::Reclaim(_) | Order::ReclaimUnit(_)) => (t("Reclaiming").to_string(), GREEN),
        Some(Order::Resurrect(_)) => (t("Resurrecting a wreck").to_string(), GREEN),
        Some(Order::Capture(id)) => (named(*id, n("Capturing: {name}"), n("Capturing")), FIGHTING),
        Some(Order::Load(_)) => (t("Picking a unit up").to_string(), TEXT),
        Some(Order::Unload(_)) => (t("Setting its cargo down").to_string(), TEXT),
        Some(Order::Wait(_) | Order::WaitAttacked(_)) => (t("Waiting").to_string(), TEXT),
        Some(Order::SetStates { .. } | Order::Stop) => (t("Idle").to_string(), STEEL),
        None if ty.is_factory && !u.factory_queue.is_empty() => (t("Starting the next unit").to_string(), GREEN),
        None if ty.is_factory => (t("Idle: nothing queued").to_string(), STEEL),
        None => match u.weapons.iter().find_map(|w| w.target) {
            Some(Target::Unit(id)) => (named(id, n("Engaging: {name}"), n("Engaging a radar contact")), FIGHTING),
            // (On duty with nothing to do: it looks again every few seconds, and says why.)
            _ if game.on_duty(u.id) => (game.duty_line(u.id).unwrap_or_else(|| t("Waiting for work").to_string()), SKY),
            _ => (t("Idle").to_string(), STEEL),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_menu_fills_its_room_in_whole_rows() {
        let grid = |count, width, height| {
            let g = BuildGrid::fit(count, width, height);
            (g.columns, g.tile, g.rows)
        };
        // The deck's build menu, as wide as two rows of eleven need: six across at full size.
        let six = 6.0 * TILE + 5.0 * GRID_GAP;
        assert_eq!(grid(11, six, 140.0), (6, TILE, 2));
        // Narrower (a small window): smaller pictures, all of them still.
        assert_eq!(grid(11, 300.0, 140.0), (6, TILE_MIN, 2));
        // Too many for the room: on pages, as many to a page as fit over the pager.
        let paged = BuildGrid::fit(30, 300.0, 140.0);
        assert!(paged.columns * paged.rows < 30 && paged.tile >= TILE_MIN, "{} x {} at {}", paged.columns, paged.rows, paged.tile);
        // A few: three across, full size.
        assert_eq!(grid(3, 300.0, 140.0), (3, TILE, 1));
        // However short, a row.
        assert_eq!(grid(11, 300.0, 10.0).2, 1);
    }

    /// The deck's plates stand along the foot of any window, in order, apart, inside it; the
    /// command plate only for a selection of ours, its build menu three pictures across at the
    /// least; the middle only for a selection, centred on the screen where it can be; the forces'
    /// plate at the left whatever is selected.
    #[test]
    fn the_deck_keeps_to_the_foot_and_its_plates_stand_apart() {
        let widest = COLUMNS_MOST as f32 * TILE + (COLUMNS_MOST - 1) as f32 * GRID_GAP;
        for (w, h) in [(1297.0, 730.0), (1067.0, 600.0), (2560.0, 1440.0), (1024.0, 600.0)] {
            let screen = Rect::from_min_size(Pos2::ZERO, EVec2::new(w, h));
            for forces in [None, Some(0), Some(3), Some(7)] {
                for (selected, commanding, build) in [(false, false, 0.0), (true, false, 0.0), (true, true, 0.0), (true, true, 416.0), (true, true, widest)] {
                    let d = deck_layout(screen, forces, selected, commanding, build);
                    let plates: Vec<Rect> = [d.forces, d.middle, d.command].into_iter().flatten().collect();
                    for p in &plates {
                        assert!(screen.contains_rect(*p), "{p:?} out of {screen:?}");
                        assert_eq!((p.bottom(), p.height()), (h - GAP, deck_height(screen)));
                    }
                    for pair in plates.windows(2) {
                        assert!(pair[0].right() + GAP <= pair[1].left() + 0.01, "{pair:?}");
                    }
                    assert_eq!(d.forces.is_some(), forces.is_some());
                    assert_eq!(d.command.is_some(), commanding);
                    assert_eq!(d.middle.is_some(), selected, "{w} wide, forces {forces:?}: the selection's plate");
                    if commanding {
                        // The orders keep their 220, the stance its words or its glyphs.
                        assert!(d.stance == STANCE || d.stance == STATES, "{}", d.stance);
                        assert!((d.orders - (ORDERS + d.stance + 2.0 * ORDER_GAP + 1.0)).abs() < 0.01);
                    }
                    if commanding && build > 0.0 {
                        assert!(d.build >= COLUMNS as f32 * TILE_MIN + (COLUMNS - 1) as f32 * GRID_GAP - 0.01, "{}", d.build);
                    }
                    // Centred on the screen where it can be: else against the plate it is pushed by.
                    if let Some(m) = d.middle.filter(|_| w >= 1200.0) {
                        let pushed = d.forces.is_some_and(|p| (m.left() - p.right() - GAP).abs() < 0.01) || d.command.is_some_and(|c| (c.left() - GAP - m.right()).abs() < 0.01);
                        assert!((m.center().x - screen.center().x).abs() < 1.0 || pushed, "{m:?} on {screen:?}");
                    }
                }
            }
        }
    }

    /// The forces' plate holds one column while the groups fit one above another and two when they
    /// do not (all seven groups at the shortest deck), and is wider with two.
    #[test]
    fn the_forces_plate_has_a_column_or_two() {
        assert_eq!(forces_columns(0, DECK_LEAST), 1);
        assert_eq!(forces_columns(4, DECK_LEAST), 1);
        assert_eq!(forces_columns(7, DECK_LEAST), 2);
        assert_eq!(forces_columns(5, DECK_MOST), 1);
        assert_eq!(forces_columns(7, DECK_MOST), 2);
        assert!(forces_width(2, false) > forces_width(1, false) + FORCE_COLUMN - 1.0);
        assert!(forces_width(2, true) < forces_width(2, false));
        // However many groups there are, the plate takes no more than two columns.
        assert_eq!(forces_columns(40, DECK_LEAST), 2);
    }

    /// The minimap stands at the top right, under the top bar, in the window, clear of the deck
    /// (and of the middle of the screen: no more than the screen's third of its width), whatever
    /// the shape of the map.
    #[test]
    fn the_minimap_stands_at_the_top_right_clear_of_the_deck() {
        for (w, h) in [(1297.0, 730.0), (1067.0, 600.0), (2560.0, 1440.0), (900.0, 600.0)] {
            let screen = Rect::from_min_size(Pos2::ZERO, EVec2::new(w, h));
            for aspect in [1.0, 0.5, 2.0, 0.25, 4.0] {
                let m = minimap_layout(screen, 36.0, aspect);
                assert!(screen.contains_rect(m), "{m:?} out of {screen:?}");
                assert_eq!(m.right(), w - GAP, "at the right edge");
                assert!(m.top() >= 36.0 + GAP - 0.01, "under the top bar: {m:?}");
                let deck = Rect::from_min_size(Pos2::new(0.0, h - GAP - deck_height(screen)), EVec2::new(w, deck_height(screen)));
                assert!(m.bottom() < deck.top(), "{w}x{h}, shape {aspect}: {m:?} over the deck");
                assert!(m.width() <= w / 3.0, "{m:?}");
                // The picture keeps the map's shape (to a point's rounding).
                let frame = 2.0 * (MAP_PAD + MAP_BEZEL + 1.0);
                let shape = (m.height() - frame) / (m.width() - frame);
                assert!((shape - aspect.clamp(0.2, 5.0)).abs() < 0.02 * aspect.max(1.0) + 0.02, "{shape} for {aspect}");
            }
        }
        // A window short of the top bar's size's laid out as if it were the top bar's least.
        let screen = Rect::from_min_size(Pos2::ZERO, EVec2::new(1297.0, 730.0));
        assert_eq!(minimap_layout(screen, 0.0, 1.0).top(), 34.0 + GAP);
    }
}
