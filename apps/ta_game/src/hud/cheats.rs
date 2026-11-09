//! The cheat panel (single player): TA's cheats and TA Reborn's at a click, and the spawn
//! tool's unit picker, which lists every unit of the game data (mods' included).

use egui::{Align2, Color32, RichText, Vec2 as EVec2};
use ta_sim::Cheat;
use ta_ui::i18n::{t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};
use ta_vfs::Vfs;

use super::{Clearance, Hud};
use crate::chat::{Reveal, SpawnTool, find_unit_types, unit_label};
use crate::controls::Rows;
use crate::game::Game;
use crate::theme::*;

/// The panel's own state: the spawn picker's search, choice, count and owner.
pub(super) struct CheatPanel {
    search: String,
    /// The search the list was made for, and the list.
    listed: Option<(String, Vec<usize>)>,
    chosen: Option<usize>,
    count: u16,
    /// Whose the spawned units are (`None`: the player's).
    owner: Option<u8>,
}

impl Default for CheatPanel {
    fn default() -> CheatPanel {
        CheatPanel { search: String::new(), listed: None, chosen: None, count: 1, owner: None }
    }
}

/// Width of the panel's content: room for the Sight switch's three settings with the switch's
/// full padding (narrower, "AS THE RULES SAY" filled its segment to the edges).
const WIDTH: f32 = 350.0;
/// Rows of the unit list shown at once.
const LIST_HEIGHT: f32 = 168.0;
const ROW: f32 = 34.0;

impl Hud {
    pub(super) fn cheat_panel(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &mut Game, clear: Clearance) {
        if game.spawn_tool.is_some() {
            spawn_hint(ctx, game, clear);
        }
        if !game.cheat_panel {
            return;
        }
        if !game.cheats_available() {
            game.cheat_panel = false;
            return;
        }
        // The screen's height less the top bar, the panel's header and frame, and a gap below;
        // the list's foot between two rows, as laid out last frame (through one, it showed the
        // top of a setting's name, its switch hidden).
        let rows_id = egui::Id::new("cheat_rows");
        let max_height = (ctx.content_rect().height() - clear.top - clear.right_top - clear.bottom - 210.0).max(200.0);
        let max_height = ctx.data(|d| d.get_temp::<Rows>(rows_id)).map_or(max_height, |rows| rows.cut(max_height));
        let mut close = false;
        egui::Area::new(egui::Id::new("cheat_panel")).anchor(Align2::RIGHT_TOP, [-14.0, clear.top + clear.right_top + 12.0]).order(egui::Order::Foreground).show(ctx, |ui| {
            let page = Panel::new("CHEATS").subtitle("SINGLE PLAYER · +HELP ON THE CHAT LINE").width(WIDTH).exit(Exit::Close).accent(RED);
            let response = page.show(ui, |ui| {
                // (An area offers only the room it took last frame: ask for the height outright.)
                let area = egui::ScrollArea::vertical().max_height(max_height).min_scrolled_height(max_height).auto_shrink([false, true]);
                ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Plate, |ui| {
                    ui.set_width(ui.available_width());
                    let mut rows = Rows::new(ui);
                    self.cheat_rows(ui, vfs, game, &mut rows);
                    ui.ctx().data_mut(|d| d.insert_temp(rows_id, rows));
                });
            });
            close = response.exit;
        });
        if close {
            game.cheat_panel = false;
        }
    }

    /// The panel's rows (each kept in `rows`, a heading with the row after it).
    fn cheat_rows(&mut self, ui: &mut egui::Ui, vfs: &Vfs, game: &mut Game, rows: &mut Rows) {
        // (As wide as the panel leaves: less a scroll bar's lane when the rows run past it.)
        let width = ui.available_width();
        let mine = game.my_cheats();
        let half = (width - 8.0) / 2.0;

        // What cheats cost: the battle's XP and achievements (taken back as one is used).
        rows.start(ui);
        let (note, colour) = if game.cheated() {
            (t("Cheats used: this battle earns no XP or achievements"), RED)
        } else {
            (t("A battle with cheats earns no XP or achievements, and what it has earned is taken back."), AMBER)
        };
        ui.add(egui::Label::new(RichText::new(note).font(body(12.5)).color(colour)).wrap());
        rows.end(ui);
        ui.add_space(4.0);

        rows.start(ui);
        panel::section(ui, "ECONOMY");
        if ui.add(ActionButton::secondary("FILL STORAGE").size(width, 32.0)).on_hover_text(t("Metal and energy to the brim (TA's +atm)")).clicked() {
            game.cheat(Cheat::Resources);
        }
        rows.end(ui);
        ui.add_space(8.0);
        rows.start(ui);
        let mut unlimited = mine.unlimited;
        if panel::switch_row(ui, "UNLIMITED RESOURCES", "Storage stays full, whatever you spend", &mut unlimited) {
            game.cheat(Cheat::Unlimited(unlimited));
        }
        rows.end(ui);

        rows.start(ui);
        panel::section(ui, "BATTLE");
        let mut god = mine.invulnerable;
        if panel::switch_row(ui, "GOD MODE", "Your units take no harm", &mut god) {
            game.cheat(Cheat::Invulnerable(god));
        }
        rows.end(ui);
        rows.start(ui);
        let mut instant = mine.instant_build;
        if panel::switch_row(ui, "INSTANT BUILD", "Construction is done at once, for nothing", &mut instant) {
            game.cheat(Cheat::InstantBuild(instant));
        }
        rows.end(ui);
        rows.start(ui);
        ui.label(w::caption("YOUR WEAPONS"));
        let mut damage = mine.damage;
        if w::segmented(ui, &mut damage, &[(-1, "HALF"), (0, "NORMAL"), (1, "DOUBLE")]).on_hover_text(t("TA's +halfshot and +doubleshot")).changed() {
            game.cheat(Cheat::Damage(damage));
        }
        rows.end(ui);
        ui.add_space(12.0);

        rows.start(ui);
        panel::section(ui, "SIGHT");
        let mut reveal = match game.view_cheats.reveal {
            Reveal::Player(_) => Reveal::Off,
            r => r,
        };
        if w::segmented(ui, &mut reveal, &[(Reveal::Off, "AS THE RULES SAY"), (Reveal::Radar, "RADAR"), (Reveal::All, "EVERYTHING")]).on_hover_text(t("TA's +radar and +nowisee")).changed() {
            game.set_reveal(reveal);
        }
        rows.end(ui);
        ui.add_space(10.0);
        rows.start(ui);
        panel::switch_row(ui, "NO SCREEN SHAKE", "Blasts leave the camera still (TA's +noshake)", &mut game.view_cheats.no_shake);
        rows.end(ui);
        rows.start(ui);
        panel::switch_row(ui, "SINGING UNITS", "Acknowledgements to a tune (TA's +sing)", &mut game.view_cheats.sing);
        rows.end(ui);

        let selected = game.selection.clone();
        let n = selected.len();
        let heading = if n == 0 { t("SELECTED UNITS").to_string() } else { tf("SELECTED UNITS · {n}", &[("n", &n)]) };
        rows.start(ui);
        panel::section(ui, &heading);
        ui.add_enabled_ui(n > 0, |ui| {
            ui.horizontal(|ui| {
                let third = (width - 16.0) / 3.0;
                if ui.add(ActionButton::secondary("MEND").size(third, 32.0)).on_hover_text(t("Back to full health (+heal)")).clicked() {
                    game.cheat(Cheat::Heal(selected.clone()));
                }
                if ui.add(ActionButton::secondary("ELITE").size(third, 32.0)).on_hover_text(t("The top veteran rank (+elite)")).clicked() {
                    game.cheat(Cheat::Promote(selected.clone()));
                }
                if ui.add(ActionButton::danger("DESTROY").size(third, 32.0)).on_hover_text(t("They blow up (+kill)")).clicked() {
                    game.cheat(Cheat::Destroy(selected.clone()));
                }
            });
        });
        rows.end(ui);
        if n == 0 {
            rows.start(ui);
            ui.label(RichText::new(t("Select units to mend, promote or destroy them.")).font(body(12.5)).color(STEEL_DIM));
            rows.end(ui);
        }
        ui.add_space(12.0);

        rows.start(ui);
        panel::section(ui, "SPAWN UNITS");
        self.spawn_picker(ui, vfs, game, rows);

        rows.start(ui);
        panel::section(ui, "THE GAME");
        ui.horizontal(|ui| {
            let restricted = game.sim.allowed.is_some();
            if ui.add_enabled(restricted, ActionButton::secondary("UNLOCK ALL").size(half, 32.0)).on_hover_text(t("Every unit may be built, whatever the mission allows (+unlock)")).clicked() {
                game.cheat(Cheat::Unlock);
            }
            if ui.add(ActionButton::danger("WIN NOW").size(half, 32.0)).on_hover_text(t("Your enemies surrender (+win)")).clicked() {
                game.cheat(Cheat::Win);
            }
        });
        rows.end(ui);
        ui.add_space(6.0);
    }

    /// The spawn tool: a unit (searched by name or key), how many, for whom (its rows kept in
    /// `rows`, which has the first open, under the heading).
    fn spawn_picker(&mut self, ui: &mut egui::Ui, vfs: &Vfs, game: &mut Game, rows: &mut Rows) {
        let width = ui.available_width();
        let ctx = ui.ctx().clone();
        ui.add(egui::TextEdit::singleline(&mut self.cheats.search).hint_text(t("Search units by name")).desired_width(width).margin(egui::Margin::symmetric(8, 5)));
        rows.end(ui);
        ui.add_space(4.0);
        rows.start(ui);
        let query = self.cheats.search.trim().to_string();
        if self.cheats.listed.as_ref().is_none_or(|(q, _)| *q != query) {
            let found = find_unit_types(&game.sim, &query);
            self.cheats.listed = Some((query, found));
        }
        let list = self.cheats.listed.as_ref().map(|(_, l)| l.clone()).unwrap_or_default();
        panel::well(ui, |ui| {
            let area = egui::ScrollArea::vertical().id_salt("spawn_list").max_height(LIST_HEIGHT).auto_shrink([false, false]);
            ta_ui::scroll::show_rows(ui, area, ta_ui::scroll::Behind::Well, ROW, list.len(), |ui, rows| {
                ui.set_width(ui.available_width());
                for &type_id in &list[rows] {
                    let chosen = self.cheats.chosen == Some(type_id);
                    let pic = self.cameo(&ctx, vfs, game, type_id);
                    let ty = game.sim.types.get(type_id);
                    let detail = format!("{} · {}", ty.name, ty.def.side);
                    let (rect, response) = ui.allocate_exact_size(EVec2::new(ui.available_width(), ROW), egui::Sense::click());
                    let painter = ui.painter();
                    if chosen {
                        painter.rect_filled(rect, 2.0, AMBER_DEEP);
                    } else if response.hovered() {
                        painter.rect_filled(rect, 2.0, PANEL_HI);
                    }
                    let icon = egui::Rect::from_min_size(rect.min + EVec2::new(3.0, 2.0), EVec2::splat(ROW - 4.0));
                    match pic {
                        Some(tex) => {
                            painter.image(tex.id(), icon, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        None => {
                            painter.rect_filled(icon, 2.0, INSET);
                        }
                    }
                    let x = icon.right() + 8.0;
                    painter.text(egui::pos2(x, rect.top() + 3.0), Align2::LEFT_TOP, unit_label(&game.sim, type_id), strong(13.0), if chosen { AMBER_BRIGHT } else { TEXT_BRIGHT });
                    painter.text(egui::pos2(x, rect.bottom() - 3.0), Align2::LEFT_BOTTOM, detail, body(11.0), STEEL_DIM);
                    if response.on_hover_text(ty.def.description.clone()).clicked() {
                        self.cheats.chosen = Some(type_id);
                    }
                }
            });
        });
        if list.is_empty() {
            ui.label(RichText::new(t("No unit by that name.")).font(body(12.5)).color(STEEL_DIM));
        }
        rows.end(ui);
        ui.add_space(8.0);
        rows.start(ui);
        ui.label(w::caption("HOW MANY"));
        w::segmented(ui, &mut self.cheats.count, &[(1, "1"), (5, "5"), (10, "10"), (25, "25")]);
        rows.end(ui);
        ui.add_space(8.0);
        rows.start(ui);
        ui.label(w::caption("FOR"));
        let me = game.local_player;
        let owner = self.cheats.owner.filter(|&o| (o as usize) < game.sim.players.len()).unwrap_or(me);
        let label = |g: &Game, p: u8| format!("{} · {}", g.player_label(p), g.sim.players[p as usize].side);
        egui::ComboBox::from_id_salt("spawn_owner").selected_text(label(game, owner)).width(width - 8.0).show_ui(ui, |ui| {
            for p in 0..game.sim.players.len() as u8 {
                if ui.selectable_label(owner == p, label(game, p)).clicked() {
                    self.cheats.owner = Some(p);
                }
            }
        });
        rows.end(ui);
        ui.add_space(10.0);
        rows.start(ui);
        let chosen = self.cheats.chosen;
        let placing = game.spawn_tool.is_some();
        let text = if placing { ta_ui::i18n::n("STOP PLACING") } else { ta_ui::i18n::n("PLACE ON THE MAP") };
        let button = if placing { ActionButton::secondary(text) } else { ActionButton::primary(text) };
        if ui.add_enabled(chosen.is_some() || placing, button.size(width, 36.0)).on_hover_text(t("Each click on the map places them; right-click to stop")).clicked() {
            game.spawn_tool = match (placing, chosen) {
                (false, Some(type_id)) => Some(SpawnTool { type_id, count: self.cheats.count, owner }),
                _ => None,
            };
        }
        // A change of mind while placing carries over to the tool.
        if let (Some(tool), Some(type_id)) = (game.spawn_tool.as_mut(), chosen) {
            *tool = SpawnTool { type_id, count: self.cheats.count, owner };
        }
        rows.end(ui);
        ui.add_space(12.0);
    }
}

/// While the spawn tool is out: what a click places, and how to stop.
fn spawn_hint(ctx: &egui::Context, game: &Game, clear: Clearance) {
    let Some(tool) = game.spawn_tool else { return };
    let unit = unit_label(&game.sim, tool.type_id);
    let text = if tool.owner == game.local_player {
        tf("Click the map to place {count} × {unit} · right-click to stop", &[("count", &tool.count), ("unit", &unit)])
    } else {
        tf("Click the map to place {count} × {unit} for {player} · right-click to stop", &[("count", &tool.count), ("unit", &unit), ("player", &game.player_label(tool.owner))])
    };
    panel::strip(ctx, "spawn_hint", Align2::CENTER_BOTTOM, EVec2::new(clear.center, -clear.bottom - 60.0), |ui| {
        ui.label(RichText::new(text).font(strong(13.5)).color(AMBER_BRIGHT));
    });
}
