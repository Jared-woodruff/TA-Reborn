//! The editor's panels in the game's look (`ta_ui`): the bar across the top, the tool palette
//! on the left, the dialogs (new map, open, properties, tile picker, unsaved changes) and the
//! tools' marks painted over the 3D view. Clicks become [`Requests`] carried out by the next
//! [`Editor::update`].

use egui::{Align2, Color32, CornerRadius, Pos2, Rect as ERect, RichText, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use glam::Vec3;
use ta_formats::image::Palette;
use ta_formats::tnt::{TILE_SIZE, TntMap};
use ta_render::Camera;
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::scroll::{self, Behind};
use ta_ui::widgets::{self as w, ActionButton};

use super::doc::{CELL_PX, MAX_SIZE_STEPS, MAX_STARTS, MapDoc, SIZE_STEP, TILE_PX};
use super::history::Snapshot;
use super::view::{Projector, ground_ring};
use super::{Dialog, Editor, FeatureGroup, Pending, Tool};

/// A map in the Open dialog: by the name its OTA gives it (a mission's without its number),
/// else its file's, title-cased.
#[derive(Clone)]
pub(super) struct OpenEntry {
    /// The file's stem.
    pub name: String,
    pub title: String,
    pub planet: String,
    pub size: String,
    pub description: String,
    pub players: usize,
}

impl OpenEntry {
    /// Every map there is (each with its art), by its title.
    pub fn list(assets: &Assets) -> Vec<OpenEntry> {
        let mut out: Vec<OpenEntry> = assets
            .vfs
            .list("maps", Some("tnt"), false)
            .into_iter()
            .map(|path| {
                let stem = path.trim_start_matches("maps/").trim_end_matches(".tnt").to_string();
                let info = assets.vfs.read_opt(&format!("maps/{stem}.ota")).map(|b| ta_data::map::MapInfo::from_ota(&ta_formats::tdf::parse_bytes(&b))).unwrap_or_default();
                let name = crate::campaign::strip_number(info.name.trim()).to_string();
                // (A name with no letters in it, "???", is no name.)
                let title = if name.chars().any(char::is_alphanumeric) { name } else { title_case(&stem) };
                let players = info.schemas.first().map_or(0, |s| s.start_positions.len());
                let designed = info.num_players.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse::<usize>().ok()).max();
                OpenEntry { name: stem, title, planet: info.planet.trim().to_string(), size: info.size.trim().to_string(), description: info.description.trim().to_string(), players: designed.map_or(players, |d| d.min(players)) }
            })
            .collect();
        out.sort_by(|a, b| a.title.to_ascii_lowercase().cmp(&b.title.to_ascii_lowercase()).then(a.name.cmp(&b.name)));
        out
    }
}

/// "a gentle time" → "A Gentle Time".
fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |first| first.to_uppercase().chain(c).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A start position's badge's radius with the start positions' tool in hand (points).
pub(super) const START_BADGE: f32 = 11.0;

/// A start position's number on its colour: dark on the light ones (white is lost on them).
fn numeral_on(color: Color32) -> Color32 {
    let light = color.r() as f32 * 0.299 + color.g() as f32 * 0.587 + color.b() as f32 * 0.114 > 150.0;
    if light { Color32::from_rgb(20, 20, 20) } else { Color32::WHITE }
}
use crate::palette::team_color;
use crate::setup::Assets;
use crate::theme::*;

/// Width of the tool palette.
const PANEL_WIDTH: f32 = 292.0;
/// A feature's tile in the grid: at least so wide, so tall; the gap between tiles; and the side
/// its picture is made small to.
const FEATURE_TILE: egui::Vec2 = egui::vec2(52.0, 48.0);
const FEATURE_GAP: f32 = 4.0;
const FEATURE_THUMB: f32 = 52.0;
/// Space kept between a dialog and the bar above it or the screen's edges (and a little more
/// once one has to be shown smaller to fit).
const DIALOG_MARGIN: f32 = 6.0;

impl Editor {
    /// Draws the editor's interface for this frame.
    pub fn ui(&mut self, ui: &mut egui::Ui, assets: &Assets, camera: &Camera) {
        let ctx = ui.ctx().clone();
        // The marks on the ground first: the panels share their layer and cover them.
        self.overlays(&ctx, camera, &assets.data);
        self.top_bar(ui, assets);
        self.status_bar(ui, assets);
        // Dialogs keep to the screen between the bars, a margin all round (they ran over its
        // buttons).
        let room = ui.available_rect_before_wrap().shrink(DIALOG_MARGIN);
        self.tool_panel(ui, assets);
        // What is left is the view: its switches at the top, the map in miniature at the foot.
        let view = ui.available_rect_before_wrap();
        self.view_bar(&ctx, view);
        if let Some(point) = self.navigator.show(ui, view, 220.0, &self.doc, &assets.data.palette, camera) {
            self.requests.look_at = Some(point);
        }
        self.dialogs(&ctx, assets, room);
        if self.help {
            self.help_page(&ctx, room);
        }
        self.messages_ui(&ctx, view);
        if self.loading.as_ref().is_some_and(|p| !matches!(p, Pending::Exit | Pending::Quit)) {
            let what = match &self.loading {
                Some(Pending::Open(name)) => name.to_uppercase(),
                _ => "NEW MAP".into(),
            };
            panel::modal_within(&ctx, "editor_loading", Panel::new("LOADING MAP").subtitle(&what).width(380.0), false, room, |ui| {
                ui.label(RichText::new(t("Reading the map and building its terrain.")).color(TEXT));
                ui.add_space(10.0);
                panel::busy_bar(ui);
            });
        }
    }

    fn top_bar(&mut self, root: &mut egui::Ui, assets: &Assets) {
        let frame = egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 10, top: 5, bottom: 6 });
        let unsaved = self.unsaved();
        let bar = egui::Panel::top("editor_bar").frame(frame).show(root, |ui| {
            // (The plate goes here, behind, once the bar's size is known.)
            let background = ui.painter().add(Shape::Noop);
            // The buttons first, from the right; the map's name and state in what is left (cut
            // short when the window is narrow). A narrow window takes New, Open, Save As and
            // Properties into the MAP menu.
            let compact = ui.ctx().content_rect().width() < 1500.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let button = |ui: &mut egui::Ui, b: ActionButton| ui.add(b.height(26.0).min_width(56.0));
                    if button(ui, ActionButton::secondary("EXIT")).on_hover_text(t("Back to the main menu")).clicked() {
                        self.requests.exit = true;
                    }
                    if button(ui, ActionButton::secondary("TEST PLAY")).on_hover_text(t("Save the map and play a battle on it against the computer; then back here (F5)")).clicked() {
                        self.requests.test_play = true;
                    }
                    ui.add_space(8.0);
                    let mut new_map = false;
                    if compact {
                        let menu = button(ui, ActionButton::secondary("MAP ▾")).on_hover_text(t("New, open, save as, properties, pictures in and out"));
                        egui::Popup::menu(&menu).width(220.0).show(|ui| {
                            let item = |ui: &mut egui::Ui, label: &str, keys: &str| {
                                ui.add(egui::Button::new(RichText::new(t(label)).font(strong(13.0))).shortcut_text(keys).min_size(vec2(200.0, 28.0))).clicked()
                            };
                            if item(ui, "NEW", "Ctrl+N") { new_map = true; ui.close(); }
                            if item(ui, "OPEN", "Ctrl+O") { self.dialog = Dialog::Open; ui.close(); }
                            if item(ui, "SAVE AS…", "Ctrl+Shift+S") { self.open_save_as(); ui.close(); }
                            if item(ui, "PROPERTIES", "") { self.open_properties(); ui.close(); }
                            if item(ui, n("IMPORT A PICTURE…"), "") { self.open_import(super::import_ui::ImportAs::Art); ui.close(); }
                            if item(ui, n("EXPORT HEIGHTS…"), "") { self.requests.export_heights = true; ui.close(); }
                        });
                        if button(ui, ActionButton::primary("SAVE")).on_hover_text(t("Save as a mod in your mods folder (Ctrl+S)")).clicked() {
                            self.requests.save = true;
                        }
                    } else {
                        if button(ui, ActionButton::secondary("IMPORT…")).on_hover_text(t("A picture as the ground's heights, its art, or a brush of tiles (or drop one on the window)")).clicked() {
                            self.open_import(super::import_ui::ImportAs::Art);
                        }
                        if button(ui, ActionButton::secondary("PROPERTIES")).on_hover_text(t("Name, planet, wind, tides, gravity, metal, sea level")).clicked() {
                            self.open_properties();
                        }
                        if button(ui, ActionButton::secondary("SAVE AS…")).on_hover_text(t("Save under another name, as a map of its own (Ctrl+Shift+S)")).clicked() {
                            self.open_save_as();
                        }
                        if button(ui, ActionButton::primary("SAVE")).on_hover_text(t("Save as a mod in your mods folder (Ctrl+S)")).clicked() {
                            self.requests.save = true;
                        }
                        if button(ui, ActionButton::secondary("OPEN")).on_hover_text(t("Start from one of the maps (Ctrl+O)")).clicked() {
                            self.dialog = Dialog::Open;
                        }
                        new_map = button(ui, ActionButton::secondary("NEW")).on_hover_text(t("A new, flat map (Ctrl+N)")).clicked();
                    }
                    if new_map {
                        if !assets.maps.iter().any(|m| m.name.eq_ignore_ascii_case(&self.new_form.palette_map)) {
                            self.new_form.palette_map = super::default_new(assets).palette_map;
                        }
                        self.dialog = Dialog::NewMap;
                    }
                    ui.add_space(12.0);
                    // What is left, from the left: the map's name and state.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
                        if ui.available_width() > 760.0 {
                            ui.label(RichText::new(t("MAP EDITOR")).font(display(20.0)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.5)));
                            ui.add_space(6.0);
                        }
                        // (The name cut short to leave the state's chips their room.)
                        let reserve = 240.0;
                        ui.scope(|ui| {
                            ui.set_max_width((ui.available_width() - reserve).max(80.0));
                            ui.add(egui::Label::new(RichText::new(&self.doc.props.name).font(strong(15.0)).color(TEXT_BRIGHT)).truncate());
                        });
                        let left = ui.available_width() - reserve;
                        if left > 110.0 {
                            w::tag(ui, &format!("{} × {}", self.doc.width_px() as u32, self.doc.height_px() as u32), STEEL);
                        }
                        if left > 250.0 && !self.doc.props.planet.is_empty() {
                            w::tag(ui, &self.doc.props.planet.to_uppercase(), STEEL);
                        }
                        if unsaved {
                            w::tag(ui, "UNSAVED", AMBER);
                        }
                        self.problems_chip(ui);
                        // Where it came from, unless its name says so already ("Gods of War (Edited)").
                        let origin = self.doc.origin.trim();
                        let from = if self.doc.tiles_from.is_some() {
                            tf("new, with the tiles of {map}", &[("map", &origin)])
                        } else if origin.is_empty() || self.doc.props.name.to_lowercase().contains(&origin.to_lowercase()) {
                            String::new()
                        } else {
                            tf("from {map}", &[("map", &origin)])
                        };
                        if !from.is_empty() && ui.available_width() > 120.0 {
                            ui.add(egui::Label::new(RichText::new(from).font(body(11.5)).color(STEEL_DIM)).truncate());
                        }
                    });
                });
            });
            background
        });
        let r = bar.response.rect;
        let plate = Shape::Vec(vec![
            ta_ui::metal::plate(root.ctx(), r, BG_DEEP),
            Shape::hline(r.x_range(), r.max.y - 2.5, Stroke::new(1.0, Color32::from_white_alpha(34))),
            Shape::hline(r.x_range(), r.max.y - 1.0, Stroke::new(2.0, Color32::from_black_alpha(220))),
        ]);
        root.painter().set(bar.inner, plate);
    }

    fn tool_panel(&mut self, root: &mut egui::Ui, assets: &Assets) {
        let panel = egui::Panel::left("editor_tools").exact_size(PANEL_WIDTH).frame(egui::Frame::new().inner_margin(10.0)).show(root, |ui| {
            // (The plate goes here, behind, once the panel's size is known.)
            let background = ui.painter().add(Shape::Noop);
            crate::theme::roomy(ui.style_mut());
            ui.spacing_mut().item_spacing.y = 5.0;
            // Sliders leave room for their value box.
            panel::section(ui, "TERRAIN");
            let row = |ui: &mut egui::Ui, tools: &[(usize, Tool)], current: &mut Tool| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let width = ((ui.available_width() - 4.0 * (tools.len() as f32 - 1.0)) / tools.len() as f32).floor();
                    for &(k, tool) in tools {
                        let hint = format!("{} ({})\n{}", t(tool.label()), k + 1, t(tool_hint(tool)));
                        if tool_button(ui, tool, *current == tool, k + 1, width).on_hover_text(hint).clicked() {
                            *current = tool;
                        }
                    }
                });
            };
            let tools: Vec<(usize, Tool)> = Tool::ALL.iter().copied().enumerate().collect();
            row(ui, &tools[..5], &mut self.tool);
            ui.add_space(4.0);
            panel::section(ui, "PAINT & PLACE");
            row(ui, &tools[5..], &mut self.tool);
            ui.add_space(6.0);
            panel::rule(ui);
            ui.label(RichText::new(t(self.tool.label())).font(display(21.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
            ui.add(egui::Label::new(RichText::new(t(tool_hint(self.tool))).font(body(12.5)).color(STEEL)).wrap());
            ui.add_space(4.0);
            // The tool's options down to the block at the foot (as tall as it was laid out last).
            let foot_id = egui::Id::new("editor_tools_foot");
            let foot = ui.ctx().data(|d| d.get_temp::<f32>(foot_id));
            let room = ui.available_height() - foot.unwrap_or(132.0) - ui.spacing().item_spacing.y;
            let area = egui::ScrollArea::vertical().max_height(room).auto_shrink([false, true]);
            scroll::show(ui, area, Behind::Plate, |ui| match self.tool {
                Tool::Tiles => self.tiles_options(ui, assets),
                Tool::Copy => self.copy_options(ui, assets),
                Tool::Features => self.feature_options(ui, assets, room),
                Tool::Starts => self.start_options(ui),
                _ => self.brush_options(ui),
            });
            // Undo, redo and the controls at the bottom.
            let block = ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 8.0) / 2.0;
                    let undo = self.history.next_undo().map(|l| tf("Undo {edit} (Ctrl+Z)", &[("edit", &t(l))]));
                    let redo = self.history.next_redo().map(|l| tf("Redo {edit} (Ctrl+Y)", &[("edit", &t(l))]));
                    if ui.add_enabled(undo.is_some(), ActionButton::secondary("UNDO").size(width, 32.0)).on_hover_text(undo.unwrap_or_default()).clicked() {
                        self.requests.undo = true;
                    }
                    if ui.add_enabled(redo.is_some(), ActionButton::secondary("REDO").size(width, 32.0)).on_hover_text(redo.unwrap_or_default()).clicked() {
                        self.requests.redo = true;
                    }
                });
                panel::rule(ui);
            });
            let height = block.response.rect.height();
            if foot.is_none_or(|f| (f - height).abs() > 0.5) {
                ui.ctx().data_mut(|d| d.insert_temp(foot_id, height));
                ui.ctx().request_discard("the editor's tool options follow the block under them");
            }
            background
        });
        let r = panel.response.rect;
        let plate = Shape::Vec(vec![
            ta_ui::metal::plate(root.ctx(), r, PANEL),
            Shape::vline(r.max.x - 2.5, r.y_range(), Stroke::new(1.0, Color32::from_white_alpha(34))),
            Shape::vline(r.max.x - 1.0, r.y_range(), Stroke::new(2.0, Color32::from_black_alpha(220))),
        ]);
        root.painter().set(panel.inner, plate);
    }

    fn brush_options(&mut self, ui: &mut egui::Ui) {
        let value_box = brush_value_box(ui);
        panel::setting(ui, "RADIUS", None, |ui| {
            w::value_slider(ui, egui::Slider::new(&mut self.radius, 16.0..=1024.0).logarithmic(true).custom_formatter(|v, _| format!("{v:.0} px")), value_box);
        });
        if self.tool != Tool::Level {
            panel::setting(ui, "STRENGTH", None, |ui| {
                w::value_slider(ui, egui::Slider::new(&mut self.strength, 0.05..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)), value_box);
            });
        } else {
            panel::setting(ui, "LEVEL", Some("Ctrl+click on the map takes its height"), |ui| {
                w::value_slider(ui, egui::Slider::new(&mut self.level, 0.0..=255.0).step_by(1.0), value_box);
            });
        }
        brush_profile(ui, self.tool, self.radius, self.strength, self.level);
        ui.add_space(4.0);
        let sea = self.doc.tnt.sea_level;
        ui.label(RichText::new(tf("Sea level {sea}: ground below it is under water.", &[("sea", &sea)])).font(body(12.0)).color(STEEL_DIM));
        ui.add_space(6.0);
        panel::switch_row(ui, "HEIGHT MAP", "Colours by height, a contour every 16 (H)", &mut self.height_map);
    }

    fn tiles_options(&mut self, ui: &mut egui::Ui, assets: &Assets) {
        if ui.add(ActionButton::primary("CHOOSE TILES…").size(ui.available_width(), 32.0)).on_hover_text(t("Pick a block of tiles from this map or another")).clicked() {
            // A new map's art is plain at first: its palette map's shows the tiles in use.
            let source = if self.picker.chosen { self.picker.source.clone() } else { self.doc.tiles_from.clone() };
            self.open_picker(source);
        }
        if ui.add(ActionButton::secondary("FROM A PICTURE…").size(ui.available_width(), 30.0)).on_hover_text(t("A picture of your own as a brush of tiles, in TA's colours (or drop one on the window)")).clicked() {
            self.open_import(super::import_ui::ImportAs::Brush);
        }
        ui.add_space(6.0);
        match &self.block {
            Some(block) => {
                let (bw, bh) = (block.w, block.h);
                if self.block_texture.as_ref().is_none_or(|(_, w, h)| (*w, *h) != (bw, bh)) {
                    let image = block_picture(&self.doc, block, &assets.data.palette);
                    self.block_texture = Some((ui.ctx().load_texture("editor-block", image, egui::TextureOptions::NEAREST), bw, bh));
                }
                ui.label(w::caption(&tf("BRUSH · {width} × {height} TILES", &[("width", &bw), ("height", &bh)])));
                if let Some((tex, _, _)) = &self.block_texture {
                    block_frame(ui, tex, 180.0);
                }
            }
            None => {
                empty_frame(ui, t("No tiles chosen yet: choose some, or Ctrl+drag over the map."));
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new(tf("{n} tiles in this map's set.", &[("n", &self.doc.tnt.tiles.len())])).font(body(12.0)).color(STEEL_DIM));
    }

    fn copy_options(&mut self, ui: &mut egui::Ui, assets: &Assets) {
        match &self.clip {
            Some(clip) => {
                ui.label(w::caption(&tf("COPIED · {width} × {height} TILES", &[("width", &clip.tiles.w), ("height", &clip.tiles.h)])));
                // The region copied, as the tiles' brush shows its block.
                let tex = self.clip_texture.get_or_insert_with(|| ui.ctx().load_texture("editor-clip", block_picture(&self.doc, &clip.tiles, &assets.data.palette), egui::TextureOptions::NEAREST));
                block_frame(ui, tex, 96.0);
                ui.add_space(4.0);
                ui.label(RichText::new(t("Click to paste; Ctrl+drag copies another region.")).font(body(12.5)).color(TEXT));
            }
            None => {
                empty_frame(ui, t("Drag over the map to copy a region."));
            }
        }
        ui.add_space(6.0);
        // What else it holds.
        if let Some(clip) = &self.clip {
            let mut held = Vec::new();
            if !clip.features.is_empty() {
                held.push(tf("Features: {n}", &[("n", &clip.features.len())]));
            }
            if !clip.void.is_empty() {
                held.push(tf("Impassable cells: {n}", &[("n", &clip.void.len())]));
            }
            if !held.is_empty() {
                ui.add(egui::Label::new(RichText::new(held.join("  ·  ")).font(body(12.0)).color(STEEL_DIM)).wrap());
            }
        }
        // What comes with the tiles, chips as the feature groups are.
        ui.label(w::caption("PASTE WITH"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            if chip(ui, "HEIGHTS", 0.0, self.paste_heights).on_hover_text(t("Paste the ground's shape too")).clicked() {
                self.paste_heights = !self.paste_heights;
            }
            if self.paste_heights && chip(ui, "RELATIVE", 0.0, self.paste_relative).on_hover_text(t("Its rises and dips added to the ground it lands on, its edges meeting it (else its heights as they were)")).clicked() {
                self.paste_relative = !self.paste_relative;
            }
            if chip(ui, "FEATURES", 0.0, self.paste_features).on_hover_text(t("Its trees, rocks, metal and impassable ground, in place of those there")).clicked() {
                self.paste_features = !self.paste_features;
            }
        });
        ui.add_space(4.0);
        if self.clip.is_some() && ui.add(ActionButton::secondary("FORGET COPY")).clicked() {
            self.clip = None;
        }
    }

    /// `room`: the height of the area the options are shown in.
    fn feature_options(&mut self, ui: &mut egui::Ui, assets: &Assets, room: f32) {
        let data = &assets.data;
        let top = ui.min_rect().min.y;
        // The groups, three to a row, all one width.
        let groups = [
            (FeatureGroup::Trees, n("TREES")),
            (FeatureGroup::Rocks, n("ROCKS")),
            (FeatureGroup::Metal, n("METAL")),
            (FeatureGroup::Vents, n("VENTS")),
            (FeatureGroup::Wrecks, n("WRECKS")),
            (FeatureGroup::Other, n("OTHER")),
        ];
        let chip_width = ((ui.available_width() - 8.0) / 3.0).floor();
        for row in groups.chunks(3) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for &(group, label) in row {
                    if chip(ui, label, chip_width, self.feature_group == group).clicked() && self.feature_group != group {
                        self.feature_group = group;
                        self.feature_choice = self.feature_list(data).first().copied();
                    }
                }
            });
        }
        if ui.add(egui::TextEdit::singleline(&mut self.feature_search).hint_text(t("Search features…")).desired_width(ui.available_width()).margin(egui::Margin::symmetric(6, 4))).changed() {
            self.feature_choice = self.feature_list(data).first().copied();
        }
        let list = self.feature_list(data);
        // How many, and the switch to every world's features, a chip as the groups are.
        ui.horizontal(|ui| {
            let count = if self.feature_all_worlds {
                tf("{n} of every world", &[("n", &list.len())])
            } else {
                let planet = if self.doc.props.planet.is_empty() { t("this map").to_string() } else { self.doc.props.planet.clone() };
                tf("{n} for {planet}", &[("n", &list.len()), ("planet", &planet)])
            };
            ui.label(RichText::new(count).font(body(11.5)).color(STEEL_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if chip(ui, "ALL WORLDS", 0.0, self.feature_all_worlds).on_hover_text(t("Features of every world, not only this planet's")).clicked() {
                    self.feature_all_worlds = !self.feature_all_worlds;
                    let list = self.feature_list(data);
                    if self.feature_choice.is_none_or(|c| !list.contains(&c)) {
                        self.feature_choice = list.first().copied();
                    }
                }
            });
        });
        // The pictures, as many to a row as fit (whole rows, two at least), down to the options
        // under them as tall as they were laid out last: those stay in view, and the well keeps
        // on under its last row down to them.
        let under_id = egui::Id::new("editor_features_under");
        let under = ui.ctx().data(|d| d.get_temp::<f32>(under_id)).unwrap_or(70.0);
        let stride = FEATURE_TILE.y + FEATURE_GAP;
        let left = room - (ui.cursor().min.y - top) - 12.0 - ui.spacing().item_spacing.y - under;
        let height = scroll::whole_rows(left.max(stride), stride, FEATURE_GAP);
        let area = egui::ScrollArea::vertical().id_salt("features").max_height(height).min_scrolled_height(height).auto_shrink([false, false]);
        let ppp = ui.ctx().pixels_per_point();
        let mut clicked = None;
        panel::well(ui, |ui| {
            ui.set_min_height(left.max(0.0).floor());
            scroll::show(ui, area, Behind::Well, |ui| {
                if list.is_empty() {
                    ui.add(egui::Label::new(RichText::new(t("Nothing here: try another group or search, or ALL WORLDS.")).font(body(12.0)).color(STEEL_DIM)).wrap());
                    return;
                }
                ui.spacing_mut().item_spacing.y = FEATURE_GAP;
                let width = ui.available_width();
                let cols = (((width + FEATURE_GAP) / (FEATURE_TILE.x + FEATURE_GAP)).floor() as usize).max(2);
                let cell_w = ((width - FEATURE_GAP * (cols - 1) as f32) / cols as f32).floor();
                for row in list.chunks(cols) {
                    let (rect, _) = ui.allocate_exact_size(vec2(width, FEATURE_TILE.y), Sense::hover());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    for (k, &i) in row.iter().enumerate() {
                        let f = &data.features[i];
                        let cell = ERect::from_min_size(pos2(rect.min.x + k as f32 * (cell_w + FEATURE_GAP), rect.min.y), vec2(cell_w, FEATURE_TILE.y));
                        // (Each picture read as it first comes into view.)
                        let picture = self
                            .feature_pictures
                            .entry(i)
                            .or_insert_with(|| {
                                feature_picture(&assets.vfs, f, &data.palette).map(|img| {
                                    let small = thumbnail(&img, (FEATURE_THUMB * ppp) as usize);
                                    (ui.ctx().load_texture(format!("editor-feature-{i}-small"), small, egui::TextureOptions::LINEAR), ui.ctx().load_texture(format!("editor-feature-{i}"), img, egui::TextureOptions::LINEAR))
                                })
                            })
                            .clone();
                        let response = ui.interact(cell, ui.id().with(("feature", i)), Sense::click());
                        let title = feature_title(&f.name);
                        let response = feature_tile(ui, response, picture.as_ref().map(|p| &p.0), &title, &format!("{}×{}", f.footprint_x, f.footprint_z), self.feature_choice == Some(i));
                        let response = response.on_hover_ui(|ui| {
                            if let Some((_, tex)) = &picture {
                                let size = tex.size_vec2();
                                let scale = (128.0 / size.x.max(size.y).max(1.0)).min(2.0);
                                ui.image((tex.id(), size * scale));
                            }
                            ui.label(RichText::new(&title).font(strong(13.0)).color(TEXT_BRIGHT));
                            let what = if f.metal > 0.0 { format!("{} · metal {:.0}", f.description, f.metal) } else { f.description.clone() };
                            if !what.trim().is_empty() {
                                ui.label(RichText::new(what).color(STEEL));
                            }
                            ui.label(RichText::new(tf("{w} × {h} cells · {world}", &[("w", &f.footprint_x), ("h", &f.footprint_z), ("world", &f.world)])).font(body(12.0)).color(STEEL_DIM));
                        });
                        if response.clicked() {
                            clicked = Some(i);
                        }
                    }
                }
            });
        });
        if let Some(i) = clicked {
            if self.feature_choice != Some(i) || self.feature_random {
                ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
            }
            // (A feature picked is the one placed, not any of the list.)
            self.feature_choice = Some(i);
            self.feature_random = false;
        }
        // How they are placed: one where clicked (or a row along a drag), or scattered round the
        // brush while the button is held; the one chosen, or any of the list.
        let start = ui.cursor().min.y;
        let value_box = brush_value_box(ui);
        ui.horizontal(|ui| {
            let random = w::galley(ui, RichText::new(t("RANDOM")).font(strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.8))).size().x + 16.0;
            let room = (ui.available_width() - random - 8.0).max(0.0);
            ui.allocate_ui(vec2(room, 28.0), |ui| {
                w::segmented_fill(ui, &mut self.scatter, &[(false, n("PLACE")), (true, n("SCATTER"))])
                    .on_hover_text(t("Place: one where you click, or a row of them along a drag. Scatter: many at random round the brush while the button is held, on land or under water as where you begin."));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if chip(ui, "RANDOM", 0.0, self.feature_random).on_hover_text(t("Each placement picks any feature of the group")).clicked() {
                    self.feature_random = !self.feature_random;
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label(w::caption(if self.scatter { "RADIUS" } else { "ERASER RADIUS" }));
            let hint = if self.scatter { t("The brush's circle; Ctrl+drag removes features in it") } else { t("Ctrl+drag removes features") };
            w::value_slider(ui, egui::Slider::new(&mut self.radius, 16.0..=1024.0).logarithmic(true).custom_formatter(|v, _| format!("{v:.0} px")), value_box).on_hover_text(hint);
        });
        if self.scatter {
            ui.horizontal(|ui| {
                ui.label(w::caption("DENSITY"));
                w::value_slider(ui, egui::Slider::new(&mut self.density, 1.0..=10.0).step_by(1.0).custom_formatter(|v, _| format!("{v:.0}")), value_box).on_hover_text(t("How close together: at 10 as close as they fit"));
            });
        }
        if self.view.unknown_features > 0 {
            ui.label(RichText::new(tf("{n} features on the map are unknown to the game data (kept as they are).", &[("n", &self.view.unknown_features)])).font(body(12.0)).color(AMBER_DIM));
        }
        let measured = ui.min_rect().max.y - start;
        if (measured - under).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(under_id, measured));
            ui.ctx().request_discard("the feature grid follows the options under it");
        }
    }

    fn start_options(&mut self, ui: &mut egui::Ui) {
        let n = self.doc.starts.len();
        for (i, s) in self.doc.starts.clone().iter().enumerate() {
            ui.horizontal(|ui| {
                let (badge, _) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::hover());
                ui.painter().rect(badge, CornerRadius::same(2), team_color(i as u8), Stroke::new(1.0, Color32::from_black_alpha(200)), StrokeKind::Inside);
                ui.painter().text(badge.center(), Align2::CENTER_CENTER, format!("{}", i + 1), strong(13.0), numeral_on(team_color(i as u8)));
                ui.label(RichText::new(format!("X {:.0} · Z {:.0}", s[0], s[1])).font(strong(12.5)).color(TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::icon_button(ui, w::Glyph::Close, true).on_hover_text(t("Remove")).clicked() {
                        self.requests.remove_start = Some(i);
                    }
                    if w::icon_button(ui, w::Glyph::Down, i + 1 < n).on_hover_text(t("Later number")).clicked() {
                        self.requests.reorder_start = Some((i, i + 1));
                    }
                    if w::icon_button(ui, w::Glyph::Up, i > 0).on_hover_text(t("Earlier number")).clicked() {
                        self.requests.reorder_start = Some((i - 1, i));
                    }
                });
            });
        }
        ui.add_space(4.0);
        if n < 2 {
            ui.label(RichText::new(t("Skirmish needs at least two start positions.")).font(body(12.5)).color(RED));
        }
        ui.label(RichText::new(format!("{n} of {MAX_STARTS}")).font(body(12.0)).color(STEEL_DIM));
    }

    /// The tools' marks on the ground: brush ring, tile outline, selection, start positions,
    /// the map's edge.
    fn overlays(&mut self, ctx: &egui::Context, camera: &Camera, data: &ta_data::GameData) {
        let painter = ctx.layer_painter(egui::LayerId::background());
        let projector = Projector::new(camera, ctx.pixels_per_point());
        self.ui_scale = ctx.pixels_per_point();
        let doc = &self.doc;
        // A polyline on the ground, broken where points fall behind the camera.
        let line = |points: &[Vec3], stroke: Stroke| {
            let mut run: Vec<Pos2> = Vec::new();
            for p in points.iter().map(|&p| projector.project(p)).chain(std::iter::once(None)) {
                match p {
                    Some(q) => run.push(q),
                    None => {
                        if run.len() >= 2 {
                            painter.add(Shape::line(std::mem::take(&mut run), stroke));
                        }
                        run.clear();
                    }
                }
            }
        };
        // The map's edge.
        let (mw, mh) = (doc.width_px(), doc.height_px());
        let edge: Vec<Vec3> = (0..=64)
            .map(|i| i as f32 / 64.0)
            .map(|t| (mw * t, 0.0))
            .chain((0..=64).map(|i| (mw, mh * i as f32 / 64.0)))
            .chain((0..=64).map(|i| (mw * (1.0 - i as f32 / 64.0), mh)))
            .chain((0..=64).map(|i| (0.0, mh * (1.0 - i as f32 / 64.0))))
            .map(|(x, z)| Vec3::new(x, doc.ground(x.min(mw - 1.0), z.min(mh - 1.0)).max(doc.tnt.sea_level as f32) + 1.0, z))
            .collect();
        line(&edge, Stroke::new(1.5, AMBER_DIM.gamma_multiply(0.8)));
        // The tiles' grid (G) where the view shows the map: the art's tile edges, where the
        // art shows them on the ground; every second or fourth line when there are too many.
        if self.layers.grid {
            let (min, max) = camera.ground_bounds(255.0);
            let (ax0, ax1) = (min.x.max(0.0), max.x.min(mw));
            let (ay0, ay1) = ((min.y - 128.0).max(0.0), max.y.min(mh));
            if ax1 > ax0 && ay1 > ay0 {
                let lines = ((ax1 - ax0).max(ay1 - ay0) / TILE_PX) as u32;
                let every = (lines / 96).max(1).next_power_of_two() as f32;
                let step = TILE_PX * every;
                // (Dark under light: it shows on sand and on sea alike.)
                let stroke = Stroke::new(1.0, Color32::from_white_alpha(70));
                let shade = Stroke::new(2.4, Color32::from_black_alpha(70));
                let sample = (step / 2.0).max(CELL_PX);
                let on_ground = |ax: f32, ay: f32| {
                    let g = doc.ground_for_art(ax.clamp(0.0, mw), ay.clamp(0.0, mh));
                    Vec3::new(g[0], g[1].max(doc.tnt.sea_level as f32) + 1.0, g[2])
                };
                let mut x = (ax0 / step).ceil() * step;
                while x <= ax1 {
                    let n = ((ay1 - ay0) / sample).ceil() as usize;
                    let points: Vec<Vec3> = (0..=n).map(|i| on_ground(x, ay0 + (ay1 - ay0) * i as f32 / n.max(1) as f32)).collect();
                    line(&points, shade);
                    line(&points, stroke);
                    x += step;
                }
                let mut y = (ay0 / step).ceil() * step;
                while y <= ay1 {
                    let n = ((ax1 - ax0) / sample).ceil() as usize;
                    let points: Vec<Vec3> = (0..=n).map(|i| on_ground(ax0 + (ax1 - ax0) * i as f32 / n.max(1) as f32, y)).collect();
                    line(&points, shade);
                    line(&points, stroke);
                    y += step;
                }
            }
        }
        // Impassable ground (TA's void cells) where the view shows the map: red, a run of them
        // a row at a time.
        if self.layers.blocked {
            let (min, max) = camera.ground_bounds(255.0);
            let (w, h) = (doc.tnt.width, doc.tnt.height);
            let (x0, x1) = (((min.x / CELL_PX).floor().max(0.0) as u32).min(w), ((max.x / CELL_PX).ceil().max(0.0) as u32).min(w));
            let (z0, z1) = (((min.y / CELL_PX).floor().max(0.0) as u32).min(h), ((max.y / CELL_PX).ceil().max(0.0) as u32).min(h));
            let sea = doc.tnt.sea_level as f32;
            let corner = |x: u32, z: u32| {
                let (px, pz) = (x as f32 * CELL_PX, z as f32 * CELL_PX);
                projector.project(Vec3::new(px, doc.ground(px.min(mw - 1.0), pz.min(mh - 1.0)).max(sea) + 1.0, pz))
            };
            let fill = Color32::from_rgba_unmultiplied(220, 40, 30, 70);
            for z in z0..z1 {
                let mut x = x0;
                while x < x1 {
                    if !doc.void_at(x, z) {
                        x += 1;
                        continue;
                    }
                    let start = x;
                    while x < x1 && doc.void_at(x, z) {
                        x += 1;
                    }
                    if let (Some(a), Some(b), Some(c), Some(d)) = (corner(start, z), corner(x, z), corner(x, z + 1), corner(start, z + 1)) {
                        painter.add(Shape::convex_polygon(vec![a, b, c, d], fill, Stroke::NONE));
                    }
                }
            }
        }
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        let art_rect = |tx: i32, tz: i32, tw: u32, th: u32| -> Vec<Vec3> {
            let (x0, z0) = (tx as f32 * TILE_PX, tz as f32 * TILE_PX);
            let (x1, z1) = (x0 + tw as f32 * TILE_PX, z0 + th as f32 * TILE_PX);
            let n = 12;
            let mut pts = Vec::new();
            for (a, b) in [((x0, z0), (x1, z0)), ((x1, z0), (x1, z1)), ((x1, z1), (x0, z1)), ((x0, z1), (x0, z0))] {
                for i in 0..n {
                    let t = i as f32 / n as f32;
                    let (ax, ay) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                    let g = doc.ground_for_art(ax.clamp(0.0, mw), ay.clamp(0.0, mh));
                    pts.push(Vec3::new(g[0], g[1].max(doc.tnt.sea_level as f32) + 1.5, g[2]));
                }
            }
            if let Some(&first) = pts.first() {
                pts.push(first);
            }
            pts
        };
        let alt = ctx.input(|i| i.modifiers.alt);
        // A footprint's outline on the ground.
        let footprint = |ox: u32, oz: u32, fx: u32, fz: u32, color: Color32| {
            let (x0, z0, x1, z1) = (ox as f32 * CELL_PX, oz as f32 * CELL_PX, (ox + fx) as f32 * CELL_PX, (oz + fz) as f32 * CELL_PX);
            let pts: Vec<Vec3> = [(x0, z0), (x1, z0), (x1, z1), (x0, z1), (x0, z0)].iter().map(|&(x, z)| Vec3::new(x, doc.ground(x.min(mw - 1.0), z.min(mh - 1.0)) + 2.0, z)).collect();
            line(&pts, Stroke::new(2.0, color));
        };
        if let Some(p) = self.cursor {
            let brush = |radius: f32, color: Color32| {
                line(&ground_ring(doc, p.x, p.z, radius, 72), Stroke::new(2.0, color));
                line(&ground_ring(doc, p.x, p.z, radius * 0.5, 48), Stroke::new(1.0, color.gamma_multiply(0.5)));
            };
            match self.tool {
                Tool::Raise | Tool::Lower | Tool::Smooth | Tool::Flatten | Tool::Level => brush(self.radius, AMBER),
                Tool::Features if ctrl => brush(self.radius, RED),
                Tool::Features if alt => {
                    let cell = ((p.x / CELL_PX).max(0.0) as u32, (p.z / CELL_PX).max(0.0) as u32);
                    if let Some(((ox, oz), name)) = doc.feature_covering(data, cell.0, cell.1) {
                        let (fx, fz) = MapDoc::footprint(data, name);
                        footprint(ox, oz, fx, fz, SKY);
                    }
                }
                Tool::Features if self.scatter => brush(self.radius, GREEN),
                Tool::Features => {
                    if let Some((fx, fz)) = self.feature_choice.map(|i| (data.features[i].footprint_x.max(1), data.features[i].footprint_z.max(1))) && let Some((ox, oz)) = super::tools::footprint_origin(doc, p.x, p.z, fx, fz) {
                        let free = self.feature_random || !doc.taken(data, (ox, oz), (fx, fz));
                        footprint(ox, oz, fx, fz, if free { GREEN } else { RED });
                    }
                }
                Tool::Tiles if self.selecting.is_none() => {
                    let art = doc.art_tile_at(p.x, p.y, p.z);
                    let (bw, bh) = self.block.as_ref().map_or((1, 1), |b| (b.w, b.h));
                    let color = if self.block.is_some() && !ctrl { AMBER } else { SKY };
                    let mut at = (art.0 - bw as i32 / 2, art.1 - bh as i32 / 2);
                    if let Some(o) = self.stroke.as_ref().and_then(|s| s.first_tile) {
                        let (w, h) = (bw.max(1) as i32, bh.max(1) as i32);
                        at = (o.0 + ((at.0 - o.0) as f32 / w as f32).round() as i32 * w, o.1 + ((at.1 - o.1) as f32 / h as f32).round() as i32 * h);
                    }
                    line(&art_rect(at.0, at.1, bw, bh), Stroke::new(2.0, color));
                }
                Tool::Copy if self.selecting.is_none() => {
                    let art = doc.art_tile_at(p.x, p.y, p.z);
                    match (&self.clip, ctrl) {
                        (Some(clip), false) => line(&art_rect(art.0 - clip.tiles.w as i32 / 2, art.1 - clip.tiles.h as i32 / 2, clip.tiles.w, clip.tiles.h), Stroke::new(2.0, AMBER)),
                        _ => line(&art_rect(art.0, art.1, 1, 1), Stroke::new(2.0, SKY)),
                    }
                }
                _ => {}
            }
        }
        if let Some((from, to)) = self.selecting {
            let r = super::tools::tile_rect(from, to, doc.tiles_wide(), doc.tiles_high(), 64);
            line(&art_rect(r.x0 as i32, r.z0 as i32, r.width(), r.height()), Stroke::new(2.0, SKY));
        }
        // Start positions: numbered badges on poles, moved apart where they would cover one
        // another (their poles still standing on their spots), the one held drawn last. (Hidden
        // with the view's switch, unless their tool is in hand.)
        let active = self.tool == Tool::Starts;
        if !active && !self.layers.starts {
            return;
        }
        let r = if active { START_BADGE } else { 9.0 };
        let feet: Vec<Option<(Pos2, Pos2)>> = doc
            .starts
            .iter()
            .map(|s| {
                let ground = Vec3::new(s[0], doc.ground(s[0], s[1]).max(doc.tnt.sea_level as f32), s[1]);
                projector.project(ground).zip(projector.project(ground + Vec3::Y * 24.0))
            })
            .collect();
        let badges = super::view::badge_layout(&feet.iter().map(|f| f.map(|(_, head)| head)).collect::<Vec<_>>(), r);
        let mut order: Vec<usize> = (0..doc.starts.len()).collect();
        order.sort_by_key(|&i| self.dragging_start == Some(i));
        if active {
            for (i, s) in doc.starts.iter().enumerate() {
                line(&ground_ring(doc, s[0], s[1], 64.0, 40), Stroke::new(1.5, team_color(i as u8).gamma_multiply(0.8)));
            }
        }
        for &i in &order {
            let (Some((foot, _)), Some(badge)) = (feet[i], badges[i]) else { continue };
            painter.line_segment([foot, badge], Stroke::new(2.0, Color32::from_black_alpha(200)));
            painter.circle_filled(foot, 2.5, Color32::from_black_alpha(200));
        }
        for &i in &order {
            let Some(badge) = badges[i] else { continue };
            let color = team_color(i as u8);
            painter.circle(badge + vec2(0.0, 1.5), r + 1.5, Color32::from_black_alpha(130), Stroke::NONE);
            painter.circle(badge, r, color, Stroke::new(1.5, if self.dragging_start == Some(i) { AMBER_BRIGHT } else { Color32::from_black_alpha(220) }));
            painter.text(badge, Align2::CENTER_CENTER, format!("{}", i + 1), strong(if active { 13.0 } else { 11.0 }), numeral_on(color));
        }
    }

    fn messages_ui(&mut self, ctx: &egui::Context, view: ERect) {
        if self.messages.is_empty() {
            return;
        }
        // (Left of the map in miniature, at the foot of the view.)
        let at = pos2(view.center().x - 120.0, view.max.y - 14.0);
        egui::Area::new(egui::Id::new("editor_messages")).pivot(Align2::CENTER_BOTTOM).fixed_pos(at).interactable(false).order(egui::Order::Foreground).show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                for (text, left, error) in &self.messages {
                    let fade = left.min(1.0);
                    let color = if *error { RED } else { TEXT_BRIGHT };
                    egui::Frame::new()
                        .fill(Color32::from_black_alpha((180.0 * fade) as u8))
                        .stroke(Stroke::new(1.0, (if *error { RED } else { AMBER_DIM }).gamma_multiply(0.6 * fade)))
                        .inner_margin(egui::Margin::symmetric(14, 5))
                        .corner_radius(2)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(text).font(strong(13.5)).color(color.gamma_multiply(fade))).wrap_mode(egui::TextWrapMode::Extend));
                        });
                    ui.add_space(4.0);
                }
            });
        });
    }

    /// The dialog open, in the middle of `room` (the screen under the bar).
    fn dialogs(&mut self, ctx: &egui::Context, assets: &Assets, room: ERect) {
        match self.dialog.clone() {
            Dialog::None => {}
            Dialog::NewMap => self.new_map_dialog(ctx, assets, room),
            Dialog::Open => self.open_dialog(ctx, assets, room),
            Dialog::Properties => self.properties_dialog(ctx, assets, room),
            Dialog::TilePicker => self.picker_dialog(ctx, assets, room),
            Dialog::SaveAs => self.save_as_dialog(ctx, assets, room),
            Dialog::Import => self.import_dialog(ctx, room),
            Dialog::Replace(stem) => {
                let page = Panel::new("REPLACE YOUR MAP?").subtitle("ANOTHER MAP OF YOURS HAS THIS NAME").width(500.0).accent(AMBER).exit(Exit::Close);
                let response = panel::modal_within(ctx, "editor_replace", page, true, room, |ui| {
                    ui.add(egui::Label::new(RichText::new(tf("You already have a map called \"{name}\". Saving this one replaces it.", &[("name", &stem)])).color(TEXT)).wrap());
                    ui.add_space(14.0);
                    button_row(ui, 36.0, |ui| {
                        if ui.add(ActionButton::primary("REPLACE IT").height(36.0).min_width(150.0)).clicked() {
                            self.replace_ok = Some(stem.clone());
                            // (Then played, if a test battle asked for the save.)
                            if std::mem::take(&mut self.test_after_replace) { self.requests.test_play = true } else { self.requests.save = true }
                            self.dialog = Dialog::None;
                        }
                        if ui.add(ActionButton::secondary("SAVE AS…").height(36.0).min_width(130.0)).on_hover_text(t("Keep both: save this one under another name")).clicked() {
                            self.test_after_replace = false;
                            self.open_save_as();
                        }
                        if ui.add(ActionButton::secondary("CANCEL").height(36.0).min_width(110.0)).clicked() {
                            self.test_after_replace = false;
                            self.dialog = Dialog::None;
                        }
                    });
                });
                if response.exit {
                    self.test_after_replace = false;
                    self.dialog = Dialog::None;
                }
            }
            Dialog::Discard(pending) => {
                let page = Panel::new("UNSAVED CHANGES").subtitle("THE MAP HAS CHANGED SINCE IT WAS SAVED").width(460.0).accent(RED).exit(Exit::Close);
                let response = panel::modal_within(ctx, "editor_discard", page, true, room, |ui| {
                    ui.label(RichText::new(t("Leave it anyway? The changes will be lost.")).color(TEXT));
                    ui.add_space(14.0);
                    button_row(ui, 36.0, |ui| {
                        if ui.add(ActionButton::danger("DISCARD CHANGES").size(180.0, 36.0)).clicked() {
                            self.discard_for(pending.clone());
                        }
                        if ui.add(ActionButton::secondary("KEEP EDITING").size(150.0, 36.0)).clicked() {
                            self.dialog = Dialog::None;
                        }
                    });
                });
                if response.exit {
                    self.dialog = Dialog::None;
                }
            }
        }
    }

    /// Save under another name: the map becomes a map of its own (the one saved before stays).
    fn save_as_dialog(&mut self, ctx: &egui::Context, assets: &Assets, room: ERect) {
        let page = Panel::new("SAVE AS").subtitle("A MAP OF ITS OWN, UNDER ANOTHER NAME").width(460.0).exit(Exit::Close);
        let mut save = false;
        let problem = super::save::name_problem(&assets.vfs, &self.save_as_name, &super::save::user_mods_dir());
        let response = panel::modal_within(ctx, "editor_save_as", page, true, room, |ui| {
            ui.label(w::caption("NAME"));
            let field = ui.add(egui::TextEdit::singleline(&mut self.save_as_name).char_limit(64).desired_width(ui.available_width())
                .margin(egui::Margin::symmetric(8, 5)));
            if !field.has_focus() && ui.memory(|m| m.focused().is_none()) {
                field.request_focus();
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.label(RichText::new(t("Also the file name; the map is saved as a mod of its own in your mods folder.")).font(body(12.0)).color(STEEL));
            if let Some(problem) = &problem {
                ui.add(egui::Label::new(RichText::new(problem).font(body(12.5)).color(RED)).wrap());
            }
            ui.add_space(10.0);
            button_row(ui, 34.0, |ui| {
                save = ui.add_enabled(problem.is_none(), ActionButton::primary("SAVE").height(34.0).min_width(120.0)).clicked() || enter && problem.is_none();
                if ui.add(ActionButton::secondary("CANCEL").height(34.0).min_width(110.0)).clicked() {
                    self.dialog = Dialog::None;
                }
            });
        });
        if response.exit {
            self.dialog = Dialog::None;
        }
        if save {
            let before = Snapshot::of(&self.doc);
            self.doc.props.name = self.save_as_name.trim().to_string();
            self.requests.props_done = Some(before);
            self.requests.save = true;
            self.dialog = Dialog::None;
        }
    }

    fn new_map_dialog(&mut self, ctx: &egui::Context, assets: &Assets, room: ERect) {
        let page = Panel::new("NEW MAP").subtitle("SIZE, SEA AND GROUND, AND THE TILES TO PAINT WITH").width(520.0).exit(Exit::Close);
        let mut create = false;
        let response = panel::modal_within(ctx, "editor_new", page, true, room, |ui| {
            let f = &mut self.new_form;
            panel::setting(ui, "NAME", None, |ui| {
                ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(ui.available_width()).margin(egui::Margin::symmetric(10, 6)).min_size(egui::vec2(0.0, w::CONTROL_HEIGHT)).vertical_align(egui::Align::Center));
            });
            let size = |v: f64, _| format!("{} px", v as u32 * SIZE_STEP);
            // (One value box width for the form, its widest value's.)
            let value_box = w::value_box_width(ui, &[&format!("{} px", MAX_SIZE_STEPS * SIZE_STEP), "255"]);
            pair(
                ui,
                |ui| {
                    panel::setting(ui, "WIDTH", None, |ui| {
                        w::value_slider(ui, egui::Slider::new(&mut f.width_steps, 1..=MAX_SIZE_STEPS).custom_formatter(size), value_box);
                    })
                },
                |ui| {
                    panel::setting(ui, "HEIGHT", None, |ui| {
                        w::value_slider(ui, egui::Slider::new(&mut f.height_steps, 1..=MAX_SIZE_STEPS).custom_formatter(size), value_box);
                    })
                },
            );
            pair(
                ui,
                |ui| {
                    panel::setting(ui, "SEA LEVEL", Some("0: no sea"), |ui| {
                        w::value_slider(ui, egui::Slider::new(&mut f.sea_level, 0..=255), value_box);
                    })
                },
                |ui| {
                    panel::setting(ui, "STARTING HEIGHT", Some("Above the sea level is land"), |ui| {
                        w::value_slider(ui, egui::Slider::new(&mut f.height, 0..=255), value_box);
                    })
                },
            );
            let title = assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(&f.palette_map)).map_or(f.palette_map.clone(), |m| format!("{} · {}", m.title, m.planet));
            panel::setting(ui, "TILES FROM", Some("The new map paints with this map's tiles and takes its planet"), |ui| {
                egui::ComboBox::from_id_salt("editor_new_palette").selected_text(title).width(ui.available_width()).height(360.0).show_ui(ui, |ui| {
                    for m in &assets.maps {
                        ui.selectable_value(&mut f.palette_map, m.name.clone(), format!("{} · {}", m.title, m.planet));
                    }
                });
            });
            ui.add_space(6.0);
            button_row(ui, 36.0, |ui| {
                if ui.add(ActionButton::primary("CREATE").size(140.0, 36.0)).clicked() {
                    create = true;
                }
                if ui.add(ActionButton::secondary("CANCEL").size(110.0, 36.0)).clicked() {
                    self.dialog = Dialog::None;
                }
                ui.label(RichText::new(format!("{} × {} px", self.new_form.width_steps * SIZE_STEP, self.new_form.height_steps * SIZE_STEP)).font(strong(13.0)).color(STEEL));
            });
        });
        if response.exit {
            self.dialog = Dialog::None;
        }
        if create {
            self.dialog = Dialog::None;
            self.requests.switch = Some(Pending::New(self.new_form.clone()));
        }
    }

    fn open_dialog(&mut self, ctx: &egui::Context, assets: &Assets, room: ERect) {
        let page = Panel::new("OPEN A MAP").subtitle("EDIT A COPY: SAVING MAKES A NEW MAP").width(820.0).exit(Exit::Close);
        let maps = self.open_maps.get_or_insert_with(|| OpenEntry::list(assets)).clone();
        if self.open_choice.as_ref().is_none_or(|c| !maps.iter().any(|m| m.name == *c)) {
            self.open_choice = maps.first().map(|m| m.name.clone());
        }
        let mut open = None;
        let mut cancel = false;
        // (The dialog round the list, as laid out last frame: the list takes the rest of the
        // room.)
        let chrome_id = egui::Id::new("editor_open_chrome");
        let chrome = ctx.data(|d| d.get_temp::<f32>(chrome_id)).unwrap_or(230.0);
        let mut list = ERect::NOTHING;
        let response = panel::modal_within(ctx, "editor_open", page, true, room, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 20.0;
                // The maps, by their names.
                ui.vertical(|ui| {
                    ui.set_width(360.0);
                    ui.add(egui::TextEdit::singleline(&mut self.open_filter).hint_text(t("Search maps and planets…")).desired_width(ui.available_width()).margin(egui::Margin::symmetric(8, 6)));
                    ui.add_space(4.0);
                    let filter = self.open_filter.to_ascii_lowercase();
                    let height = scroll::whole_rows((room.height() - chrome).clamp(240.0, 460.0), panel::LIST_ROW + 1.0, 1.0);
                    let area = egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, false]);
                    list = scroll::well(ui, area, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for m in maps.iter().filter(|m| filter.is_empty() || m.title.to_ascii_lowercase().contains(&filter) || m.planet.to_ascii_lowercase().contains(&filter)) {
                            let chosen = self.open_choice.as_deref() == Some(m.name.as_str());
                            let row = panel::list_row(ui, chosen, true, RichText::new(&m.title), Some(RichText::new(m.planet.to_uppercase()).font(strong(11.0))));
                            if row.double_clicked() {
                                open = Some(m.name.clone());
                            } else if row.clicked() {
                                self.open_choice = Some(m.name.clone());
                            }
                        }
                    })
                    .inner_rect;
                });
                // The one picked: its picture and what it is.
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    let Some(m) = maps.iter().find(|m| Some(m.name.as_str()) == self.open_choice.as_deref()) else { return };
                    let (frame, _) = ui.allocate_exact_size(vec2(ui.available_width(), 250.0), Sense::hover());
                    if let Some(picture) = crate::map_preview::picture(ctx, assets, &m.name, frame.width()) {
                        crate::map_preview::show(ui.painter(), &picture, frame);
                    }
                    ui.add_space(8.0);
                    w::fitted_title(ui, &m.title, 24.0, TEXT_BRIGHT);
                    let line = if m.players >= 2 { tf("{planet} · {size} · UP TO {n} PLAYERS", &[("planet", &m.planet.to_uppercase()), ("size", &m.size), ("n", &m.players)]) } else { format!("{} · {}", m.planet.to_uppercase(), m.size) };
                    ui.label(RichText::new(line).font(strong(11.5)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
                    if !m.description.is_empty() {
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(RichText::new(&m.description).font(body(13.0)).color(TEXT)).wrap());
                    }
                });
            });
            ui.add_space(8.0);
            panel::rule(ui);
            button_row(ui, 36.0, |ui| {
                if ui.add_enabled(self.open_choice.is_some(), ActionButton::primary("OPEN").size(130.0, 36.0)).clicked() {
                    open = self.open_choice.clone();
                }
                if ui.add(ActionButton::secondary("CANCEL").size(110.0, 36.0)).clicked() {
                    cancel = true;
                }
            });
        });
        if list.is_positive() {
            ctx.data_mut(|d| d.insert_temp(chrome_id, response.rect.height() - list.height()));
        }
        if response.exit || cancel || open.is_some() {
            self.dialog = Dialog::None;
            // (Read again next time: a map saved meanwhile shows.)
            self.open_maps = None;
        }
        if let Some(name) = open {
            self.requests.switch = Some(Pending::Open(name));
        }
    }

    fn properties_dialog(&mut self, ctx: &egui::Context, assets: &Assets, room: ERect) {
        let page = Panel::new("MAP PROPERTIES").subtitle("WHAT THE SKIRMISH PAGE SHOWS AND THE GAME PLAYS BY").width(620.0).exit(Exit::Close);
        // The changes are made as they are typed: it closes by its cross, Escape or a click
        // outside, as Options does (a DONE under it made it too tall for the room).
        let response = panel::modal_within(ctx, "editor_properties", page, true, room, |ui| {
            let p = &mut self.doc.props;
            // (One value box width for the form, its widest value's.)
            let value_box = w::value_box_width(ui, &["5000", "100.0", "255"]);
            panel::setting(ui, "NAME", Some("Also the file name; saving replaces your own map of this name"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(ui.available_width()).margin(egui::Margin::symmetric(10, 6)).min_size(egui::vec2(0.0, w::CONTROL_HEIGHT)).vertical_align(egui::Align::Center));
            });
            panel::setting(ui, "DESCRIPTION", None, |ui| {
                ui.add(egui::TextEdit::singleline(&mut p.description).desired_width(ui.available_width()).margin(egui::Margin::symmetric(10, 6)).min_size(egui::vec2(0.0, w::CONTROL_HEIGHT)).vertical_align(egui::Align::Center));
            });
            let mut planets: Vec<String> = assets.maps.iter().map(|m| m.planet.trim().to_string()).filter(|p| !p.is_empty()).collect();
            planets.sort_by_key(|p| p.to_ascii_lowercase());
            planets.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            let sea = &mut self.doc.tnt.sea_level;
            pair(
                ui,
                |ui| {
                    panel::setting(ui, "PLANET", Some("Decides the features offered"), |ui| {
                        egui::ComboBox::from_id_salt("editor_planet").selected_text(p.planet.clone()).width(ui.available_width()).show_ui(ui, |ui| {
                            for planet in &planets {
                                ui.selectable_value(&mut p.planet, planet.clone(), planet);
                            }
                        });
                    })
                },
                |ui| {
                    panel::setting(ui, "SEA LEVEL", Some("Ground below it is under water"), |ui| {
                        w::value_slider(ui, egui::Slider::new(sea, 0..=255).clamping(egui::SliderClamping::Edits), value_box);
                    })
                },
            );
            pair(
                ui,
                |ui| {
                    panel::setting(ui, "MIN WIND", Some("Wind generators' least"), |ui| {
                        w::value_slider(ui, egui::Slider::new(&mut p.min_wind, 0.0..=10000.0).step_by(50.0).clamping(egui::SliderClamping::Edits), value_box);
                    })
                },
                |ui| {
                    panel::setting(ui, "MAX WIND", Some("...and most"), |ui| {
                        w::value_slider(ui, egui::Slider::new(&mut p.max_wind, 0.0..=10000.0).step_by(50.0).clamping(egui::SliderClamping::Edits), value_box);
                    })
                },
            );
            pair(
                ui,
                |ui| {
                    panel::setting(ui, "TIDAL STRENGTH", Some("Tidal generators' output"), |ui| {
                        w::value_slider(ui, egui::Slider::new(&mut p.tidal, 0.0..=100.0).step_by(1.0).clamping(egui::SliderClamping::Edits), value_box);
                    })
                },
                |ui| {
                    panel::setting(ui, "GRAVITY", Some("TA's usual is 112"), |ui| {
                        w::value_slider(ui, egui::Slider::new(&mut p.gravity, 1.0..=400.0).step_by(1.0).clamping(egui::SliderClamping::Edits), value_box);
                    })
                },
            );
            pair(
                ui,
                |ui| {
                    panel::setting(ui, "SURFACE METAL", Some("Metal in every cell of bare ground"), |ui| {
                        w::value_slider(ui, egui::Slider::new(&mut p.surface_metal, 0.0..=255.0).logarithmic(true).smallest_positive(1.0).step_by(1.0).clamping(egui::SliderClamping::Edits), value_box);
                    })
                },
                |ui| {
                    panel::setting(ui, "MOHO METAL", Some("TA's MohoMetal setting"), |ui| {
                        w::value_slider(ui, egui::Slider::new(&mut p.moho_metal, 0.0..=255.0).logarithmic(true).smallest_positive(1.0).step_by(1.0).clamping(egui::SliderClamping::Edits), value_box);
                    })
                },
            );
            if p.max_wind < p.min_wind {
                p.max_wind = p.min_wind;
            }
        });
        if response.exit {
            self.dialog = Dialog::None;
            self.requests.props_done = self.props_before.take().or_else(|| Some(Snapshot::of(&self.doc)));
        }
    }

    pub(super) fn open_picker(&mut self, source: Option<String>) {
        if self.picker.source != source || self.picker.map.is_none() && source.is_some() {
            self.picker.map = None;
            self.picker.selection = None;
        }
        self.picker.source = source;
        self.picker.error = None;
        self.picker.drag_from = None;
        self.picker.texture = None;
        self.dialog = Dialog::TilePicker;
    }

    fn picker_dialog(&mut self, ctx: &egui::Context, assets: &Assets, room: ERect) {
        // (Its frame, header and the rows round the picture, as laid out last frame: the
        // picture takes the rest of the room.)
        let chrome_id = egui::Id::new("editor_picker_chrome");
        let chrome = ctx.data(|d| d.get_temp::<egui::Vec2>(chrome_id)).unwrap_or(vec2(40.0, 240.0));
        let width = (room.width() - chrome.x).clamp(480.0, 1100.0);
        let page = Panel::new("CHOOSE TILES").subtitle("CLICK A TILE OR DRAG OVER A BLOCK OF THEM").width(width).exit(Exit::Close);
        let mut chosen = None;
        let mut switch_to: Option<Option<String>> = None;
        // (The room the picture was given, its bars' lanes in it: the same whichever is shown.)
        let mut picture = egui::Vec2::ZERO;
        let response = panel::modal_within(ctx, "editor_picker", page, false, room, |ui| {
            ui.horizontal(|ui| {
                ui.label(w::caption("FROM"));
                let current = match &self.picker.source {
                    None => t("THIS MAP").to_string(),
                    Some(name) => assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(name)).map_or(name.clone(), |m| m.title.clone()),
                };
                egui::ComboBox::from_id_salt("editor_picker_source").selected_text(current).width(300.0).height(400.0).show_ui(ui, |ui| {
                    if ui.selectable_label(self.picker.source.is_none(), t("THIS MAP")).clicked() {
                        switch_to = Some(None);
                    }
                    for m in &assets.maps {
                        if ui.selectable_label(self.picker.source.as_deref() == Some(m.name.as_str()), format!("{} · {}", m.title, m.planet)).clicked() {
                            switch_to = Some(Some(m.name.clone()));
                        }
                    }
                });
                ui.add_space(12.0);
                if w::segmented(ui, &mut self.picker.tile_set, &[(false, "ART"), (true, "TILE SET")]).changed() {
                    self.picker.selection = None;
                }
                ui.add_space(12.0);
                ui.label(w::caption("ZOOM"));
                w::segmented(ui, &mut self.picker.zoom, &[(4.0, "12%"), (8.0, "25%"), (16.0, "50%"), (32.0, "100%")]);
            });
            ui.add_space(6.0);
            // The source map and its picture: the art, or every tile of its set in rows.
            let map = match self.picker.source_map(&assets.vfs) {
                Ok(map) => map,
                Err(error) => {
                    self.picker.selection = None;
                    self.picker.texture = None;
                    ui.add(egui::Label::new(RichText::new(tf("Could not open the map: {error}", &[("error", &error)])).color(RED)).wrap());
                    if ui.add(ActionButton::secondary("TRY AGAIN")).clicked() { self.picker.error = None; }
                    return;
                }
            };
            let src: &TntMap = map.as_deref().unwrap_or(&self.doc.tnt);
            let src_tiles = src.tiles.len();
            let tile = self.picker.zoom;
            let room_height = (room.height() - chrome.y).clamp(240.0, 620.0);
            // The tile set in rows as wide as the picture (a lane kept beside them for the bar
            // when they run past its foot); a selection in other rows means other tiles.
            let set = self.picker.tile_set.then(|| set_columns(src_tiles, width, room_height, tile));
            if set.is_some() && self.picker.texture.as_ref().is_some_and(|(_, _, s)| s.is_some() && *s != set) {
                self.picker.selection = None;
            }
            let (tw, th) = match set {
                Some(columns) => (columns, (src_tiles as u32).div_ceil(columns).max(1)),
                None => (src.width / 2, src.height / 2),
            };
            let k = picture_scale(tw, th);
            if self.picker.texture.as_ref().is_none_or(|(_, kk, s)| (*kk, *s) != (k, set)) {
                let image = match set {
                    Some(columns) => set_picture(src, &assets.data.palette, k, columns),
                    None => map_picture(src, &assets.data.palette, k),
                };
                self.picker.texture = Some((ctx.load_texture("editor-picker", image, egui::TextureOptions::LINEAR), k, set));
            }
            let Some((tex, _, _)) = self.picker.texture.clone() else { return };
            let size = vec2(tw as f32 * tile, th as f32 * tile);
            // (Cut at a whole row of tiles, a bar's lane under them when they run past the side,
            // and its room kept when they do not: the dialog is one size whichever is shown.)
            let rows = ((room_height - scroll::LANE) / tile).floor().max(1.0) * tile;
            let lane = if size.x > width { scroll::LANE } else { 0.0 };
            let area = egui::ScrollArea::both().max_height(rows + lane).min_scrolled_height(rows).max_width(width).auto_shrink([false, false]);
            picture = vec2(width, rows + scroll::LANE);
            scroll::show(ui, area, Behind::Picture, |ui| {
                let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
                let painter = ui.painter_at(rect);
                painter.image(tex.id(), rect, ERect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                let to_tile = |p: Pos2| (((p.x - rect.min.x) / tile).clamp(0.0, tw as f32 - 1.0) as u32, ((p.y - rect.min.y) / tile).clamp(0.0, th as f32 - 1.0) as u32);
                if let Some(pos) = response.interact_pointer_pos() {
                    let t = to_tile(pos);
                    if response.drag_started() || response.clicked() {
                        self.picker.drag_from = Some(t);
                    }
                    if let Some(from) = self.picker.drag_from {
                        let r = super::tools::tile_rect((from.0 as i32, from.1 as i32), (t.0 as i32, t.1 as i32), tw, th, 64);
                        self.picker.selection = Some(r);
                    }
                }
                if response.drag_stopped() || response.clicked() {
                    self.picker.drag_from = None;
                }
                if let Some(r) = self.picker.selection.filter(|r| r.x1 <= tw && r.z1 <= th) {
                    let sel = ERect::from_min_max(rect.min + vec2(r.x0 as f32, r.z0 as f32) * tile, rect.min + vec2(r.x1 as f32, r.z1 as f32) * tile);
                    painter.rect_filled(sel, 0.0, Color32::from_rgba_unmultiplied(242, 166, 58, 40));
                    painter.rect_stroke(sel, 0.0, Stroke::new(2.0, AMBER), StrokeKind::Outside);
                }
                // A grid of tiles when zoomed in far enough to see them.
                if tile >= 16.0 {
                    let visible = ui.clip_rect();
                    let grid = Stroke::new(1.0, Color32::from_white_alpha(18));
                    for x in 0..=tw {
                        let px = rect.min.x + x as f32 * tile;
                        if px >= visible.min.x && px <= visible.max.x {
                            painter.vline(px, rect.y_range(), grid);
                        }
                    }
                    for z in 0..=th {
                        let py = rect.min.y + z as f32 * tile;
                        if py >= visible.min.y && py <= visible.max.y {
                            painter.hline(rect.x_range(), py, grid);
                        }
                    }
                }
            });
            ui.add_space(8.0 + scroll::LANE - lane);
            ui.horizontal(|ui| {
                let sel = self.picker.selection;
                let text = sel.map_or(tf("Nothing selected: click a tile or drag over a block (at most 64 × 64). {n} tiles in the set.", &[("n", &src_tiles)]), |r| tf("{width} × {height} tiles selected", &[("width", &r.width()), ("height", &r.height())]));
                ui.label(RichText::new(text).font(strong(13.0)).color(STEEL));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(sel.is_some(), ActionButton::primary("USE THESE TILES").size(170.0, 36.0)).clicked() {
                        chosen = sel;
                    }
                    if ui.add(ActionButton::secondary("CANCEL").size(110.0, 36.0)).clicked() {
                        self.dialog = Dialog::None;
                    }
                });
            });
        });
        if picture.x > 0.0 && picture.y > 0.0 {
            ctx.data_mut(|d| d.insert_temp(chrome_id, response.rect.size() - picture));
        }
        if let Some(source) = switch_to {
            self.picker.chosen = true;
            self.open_picker(source);
        }
        if response.exit {
            self.dialog = Dialog::None;
        }
        if let Some(r) = chosen {
            self.dialog = Dialog::None;
            self.requests.block = Some((self.picker.source.clone(), r, self.picker.texture.as_ref().and_then(|(_, _, set)| *set)));
        }
    }
}

/// How many of a tile set's `tiles` a row fill a picture `width` wide at `tile` a tile (fewer
/// by a bar's lane when its rows run past `height`).
fn set_columns(tiles: usize, width: f32, height: f32, tile: f32) -> u32 {
    let columns = |width: f32| ((width / tile).floor() as u32).max(1);
    let all = columns(width);
    if (tiles as u32).div_ceil(all) as f32 * tile > height { columns(width - scroll::LANE) } else { all }
}

/// A map's tile set at `k` pixels per tile, `columns` tiles a row.
fn set_picture(map: &TntMap, palette: &Palette, k: u32, columns: u32) -> egui::ColorImage {
    let (cols, rows) = (columns.max(1) as usize, map.tiles.len().div_ceil(columns.max(1) as usize).max(1));
    let k = k as usize;
    let (w, h) = (cols * k, rows * k);
    let mut rgba = vec![0u8; w * h * 4];
    for (i, tile) in map.tiles.iter().enumerate() {
        let thumb = tile_thumb(tile, palette, k as u32);
        let (tx, tz) = (i % cols, i / cols);
        for y in 0..k {
            for x in 0..k {
                let o = ((tz * k + y) * w + tx * k + x) * 4;
                rgba[o..o + 4].copy_from_slice(&thumb[y * k + x]);
            }
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([w.max(1), h.max(1)], &rgba)
}

/// A dialog's buttons, right to left on a row as tall as they are. (Laid out right to left in
/// what was left, the row took all the height there was, and the dialog ran down the screen.)
pub(super) fn button_row(ui: &mut egui::Ui, height: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), height), egui::Layout::right_to_left(egui::Align::Center), add);
}

/// Two settings side by side, row by row, a gutter between them.
pub(super) fn pair(ui: &mut egui::Ui, left: impl FnOnce(&mut egui::Ui), right: impl FnOnce(&mut egui::Ui)) {
    ui.spacing_mut().item_spacing.x = space::XL;
    ui.columns(2, |cols| {
        for col in cols.iter_mut() {
            col.spacing_mut().item_spacing.x = 8.0;
        }
        let (a, b) = cols.split_at_mut(1);
        left(&mut a[0]);
        right(&mut b[0]);
    });
}

/// Hints for the tool palette.
fn tool_hint(tool: Tool) -> &'static str {
    match tool {
        Tool::Raise => n("Hold the left button to build up the ground; Shift lowers it."),
        Tool::Lower => n("Hold the left button to dig; Shift raises instead."),
        Tool::Smooth => n("Evens out bumps and hard edges under the brush."),
        Tool::Flatten => n("Levels the ground to the height where the stroke began."),
        Tool::Level => n("Sets the ground under the brush to one height."),
        Tool::Tiles => n("Click or drag to stamp the tiles; Ctrl+drag takes a block from the map. Tiles go where the art shows them."),
        Tool::Copy => n("Drag to copy the tiles (and heights) of a region, then click to paste it."),
        Tool::Features => n("Click or drag to place the chosen feature; Ctrl+drag removes features."),
        Tool::Starts => n("Click to add a start position, drag one to move it, Ctrl+click to remove it."),
    }
}

/// A gunmetal tool tile `width` wide (brass when it is the tool in hand): its painted glyph,
/// its name under it and its key in the corner.
fn tool_button(ui: &mut egui::Ui, tool: Tool, active: bool, key: usize, width: f32) -> egui::Response {
    use ta_ui::metal::{self, Slab};
    let (rect, response) = ui.allocate_exact_size(vec2(width, 52.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, active, t(tool.label())));
    if ui.is_rect_visible(rect) {
        let hot = if response.hovered() { 1.0 } else { 0.0 };
        let down = response.is_pointer_button_down_on();
        let kind = if active { Slab::Brass } else { Slab::Steel };
        let painter = ui.painter();
        painter.add(metal::slab(ui.ctx(), rect, kind, hot, down));
        let drop = if down { 1.0 } else { 0.0 };
        let c = pos2(rect.center().x, rect.min.y + 19.0 + drop);
        let edge = if active { Color32::from_white_alpha(80) } else { Color32::from_black_alpha(190) };
        paint_glyph(painter, ERect::from_center_size(c + vec2(if active { 0.0 } else { 1.0 }, 1.0), vec2(20.0, 20.0)), tool, edge);
        paint_glyph(painter, ERect::from_center_size(c, vec2(20.0, 20.0)), tool, kind.text(hot));
        // Its name, as large as fits the tile.
        let name = t(tool.short());
        let fit = |size: f32| w::galley(ui, RichText::new(name).font(strong(size)).extra_letter_spacing(ta_ui::i18n::tracking(0.4)));
        let mut text = fit(10.5);
        for size in [9.5, 8.5] {
            if text.size().x <= width - 6.0 { break; }
            text = fit(size);
        }
        let at = pos2(rect.center().x - text.size().x.min(width - 6.0) / 2.0, rect.max.y - 7.0 - text.size().y + drop);
        painter.with_clip_rect(rect.shrink(3.0)).galley(at, text, kind.text(hot));
        painter.text(rect.min + vec2(5.0, 3.0), Align2::LEFT_TOP, key.to_string(), strong(9.0), kind.text(hot).gamma_multiply(0.6));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The tools' glyphs, drawn in a 22-point square.
fn paint_glyph(p: &egui::Painter, r: ERect, tool: Tool, color: Color32) {
    let s = Stroke::new(1.8, color);
    let at = |x: f32, y: f32| pos2(r.min.x + x * r.width(), r.min.y + y * r.height());
    let mound = |p: &egui::Painter, top: f32| {
        let pts: Vec<Pos2> = (0..=16).map(|i| {
            let t = i as f32 / 16.0;
            at(t, 0.95 - (1.0 - (2.0 * t - 1.0).powi(2)) * top)
        }).collect();
        p.add(Shape::line(pts, s));
    };
    let arrow = |p: &egui::Painter, up: bool| {
        let (a, b) = if up { (0.55, 0.05) } else { (0.05, 0.55) };
        p.line_segment([at(0.5, a), at(0.5, b)], s);
        let tip = at(0.5, b);
        let d = if up { 1.0 } else { -1.0 };
        p.add(Shape::convex_polygon(vec![tip, tip + vec2(-4.5, 5.0 * d), tip + vec2(4.5, 5.0 * d)], color, Stroke::NONE));
    };
    match tool {
        Tool::Raise => {
            mound(p, 0.35);
            arrow(p, true);
        }
        Tool::Lower => {
            let pts: Vec<Pos2> = (0..=16).map(|i| {
                let t = i as f32 / 16.0;
                at(t, 0.6 + (1.0 - (2.0 * t - 1.0).powi(2)) * 0.35)
            }).collect();
            p.add(Shape::line(pts, s));
            arrow(p, false);
        }
        Tool::Smooth => {
            let wave = |y: f32, amp: f32| -> Vec<Pos2> { (0..=20).map(|i| { let t = i as f32 / 20.0; at(t, y + (t * 12.0).sin() * amp) }).collect() };
            p.add(Shape::line(wave(0.3, 0.12), Stroke::new(1.4, color.gamma_multiply(0.6))));
            p.add(Shape::line(wave(0.72, 0.03), s));
        }
        Tool::Flatten => {
            p.add(Shape::line(vec![at(0.0, 0.95), at(0.25, 0.45), at(0.75, 0.45), at(1.0, 0.95)], s));
            p.line_segment([at(0.0, 0.2), at(1.0, 0.2)], Stroke::new(1.2, color.gamma_multiply(0.6)));
            p.line_segment([at(0.5, 0.2), at(0.5, 0.42)], Stroke::new(1.2, color.gamma_multiply(0.6)));
        }
        Tool::Level => {
            p.line_segment([at(0.0, 0.5), at(1.0, 0.5)], s);
            p.line_segment([at(0.0, 0.85), at(1.0, 0.85)], Stroke::new(1.2, color.gamma_multiply(0.6)));
            p.add(Shape::convex_polygon(vec![at(0.5, 0.45), at(0.35, 0.15), at(0.65, 0.15)], color, Stroke::NONE));
        }
        Tool::Tiles => {
            for (x, y) in [(0.0, 0.0), (0.52, 0.0), (0.0, 0.52), (0.52, 0.52)] {
                let q = ERect::from_min_size(at(x, y), vec2(r.width() * 0.46, r.height() * 0.46));
                if (x, y) == (0.52, 0.52) {
                    p.rect_filled(q, 1.0, color);
                } else {
                    p.rect_stroke(q, 1.0, Stroke::new(1.4, color), StrokeKind::Inside);
                }
            }
        }
        Tool::Copy => {
            p.rect_stroke(ERect::from_min_max(at(0.0, 0.0), at(0.62, 0.62)), 1.0, Stroke::new(1.4, color.gamma_multiply(0.7)), StrokeKind::Inside);
            p.rect_stroke(ERect::from_min_max(at(0.38, 0.38), at(1.0, 1.0)), 1.0, s, StrokeKind::Inside);
        }
        Tool::Features => {
            p.add(Shape::convex_polygon(vec![at(0.5, 0.0), at(0.9, 0.62), at(0.1, 0.62)], color, Stroke::NONE));
            p.rect_filled(ERect::from_min_max(at(0.42, 0.62), at(0.58, 0.98)), 0.0, color);
        }
        Tool::Starts => {
            p.line_segment([at(0.2, 0.0), at(0.2, 1.0)], s);
            p.add(Shape::convex_polygon(vec![at(0.22, 0.02), at(0.95, 0.24), at(0.22, 0.46)], color, Stroke::NONE));
        }
    }
}

/// A small toggle chip (feature groups), `width` wide (wider if its label needs it).
fn chip(ui: &mut egui::Ui, label: &str, width: f32, on: bool) -> egui::Response {
    let text = w::galley(ui, RichText::new(t(label)).font(strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.8)));
    let (rect, response) = ui.allocate_exact_size(vec2(width.max(text.size().x + 16.0), 24.0), Sense::click());
    let (fill, border, color) = if on {
        (AMBER_DEEP, AMBER, AMBER_BRIGHT)
    } else if response.hovered() {
        (PANEL_HI, AMBER_DIM, TEXT_BRIGHT)
    } else {
        (INSET, LINE_HI, STEEL)
    };
    ui.painter().rect(rect, CornerRadius::same(2), fill, Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().galley(rect.center() - text.size() / 2.0, text, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A feature in the grid, in the cell `response` was made for: its picture (a glyph where it has
/// none) and its footprint in the corner (its name for screen readers and in its tooltip);
/// brass when it is the one chosen.
fn feature_tile(ui: &egui::Ui, response: egui::Response, picture: Option<&egui::TextureHandle>, name: &str, footprint: &str, selected: bool) -> egui::Response {
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, name));
    let rect = response.rect;
    let hot = response.hovered();
    let painter = ui.painter();
    let (fill, border) = if selected { (AMBER_DEEP, AMBER) } else if hot { (PANEL_HI, AMBER_DIM) } else { (PANEL, LINE) };
    painter.rect(rect, CornerRadius::same(3), fill, Stroke::new(if selected { 1.5 } else { 1.0 }, border), StrokeKind::Inside);
    // The picture fitted in (small ones at most two and a half times their size).
    let slot = rect.shrink(4.0);
    match picture {
        Some(tex) => {
            let size = tex.size_vec2();
            let k = (slot.width() / size.x.max(1.0)).min(slot.height() / size.y.max(1.0)).min(2.5);
            painter.image(tex.id(), ERect::from_center_size(slot.center(), size * k), ERect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => paint_glyph(painter, ERect::from_center_size(slot.center(), vec2(24.0, 24.0)), Tool::Features, STEEL_DIM),
    }
    let detail = w::galley(ui, RichText::new(footprint).font(strong(9.0)));
    let at = pos2(rect.max.x - 4.0 - detail.size().x, rect.max.y - 3.0 - detail.size().y);
    painter.rect_filled(ERect::from_min_size(at - vec2(3.0, 1.0), detail.size() + vec2(6.0, 2.0)), 2.0, Color32::from_black_alpha(150));
    painter.galley(at, detail, STEEL);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A feature's picture made small for its tile: cut to what is drawn, then box-filtered to at
/// most `side` pixels (the colours are premultiplied, so its alpha weights them), its faint
/// strokes strengthened a little (thin lines all but vanish at that size).
fn thumbnail(image: &egui::ColorImage, side: usize) -> egui::ColorImage {
    let [w, h] = image.size;
    let drawn = |x: usize, y: usize| image.pixels[y * w + x].a() > 8;
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if drawn(x, y) {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    if x0 >= x1 || y0 >= y1 {
        return image.clone();
    }
    let (cw, ch) = (x1 - x0, y1 - y0);
    let scale = (side.max(1) as f32 / cw.max(ch) as f32).min(1.0);
    let (tw, th) = (((cw as f32 * scale).round() as usize).max(1), ((ch as f32 * scale).round() as usize).max(1));
    let mut pixels = Vec::with_capacity(tw * th);
    for ty in 0..th {
        for tx in 0..tw {
            let (sx0, sx1) = (x0 + tx * cw / tw, (x0 + (tx + 1) * cw / tw).max(x0 + tx * cw / tw + 1));
            let (sy0, sy1) = (y0 + ty * ch / th, (y0 + (ty + 1) * ch / th).max(y0 + ty * ch / th + 1));
            let mut sum = [0u32; 4];
            for y in sy0..sy1.min(y1) {
                for x in sx0..sx1.min(x1) {
                    let p = image.pixels[y * w + x];
                    for (s, v) in sum.iter_mut().zip([p.r(), p.g(), p.b(), p.a()]) {
                        *s += v as u32;
                    }
                }
            }
            let n = ((sx1.min(x1) - sx0) * (sy1.min(y1) - sy0)).max(1) as f32;
            let k = if scale < 1.0 { 1.6 } else { 1.0 };
            let c = |v: u32| (v as f32 / n * k).min(255.0) as u8;
            let a = c(sum[3]);
            pixels.push(Color32::from_rgba_premultiplied(c(sum[0]).min(a), c(sum[1]).min(a), c(sum[2]).min(a), a));
        }
    }
    egui::ColorImage::new([tw, th], pixels)
}

/// A feature's name as a title: "MarsPlant01Crisp" → "Mars Plant 01 Crisp".
pub(super) fn feature_title(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut last: Option<char> = None;
    for c in name.chars() {
        if c == '_' || c == '-' || c == ' ' {
            if !out.ends_with(' ') && !out.is_empty() {
                out.push(' ');
            }
            last = Some(' ');
            continue;
        }
        if let Some(p) = last && p != ' ' && ((p.is_lowercase() && c.is_uppercase()) || (p.is_alphabetic() && c.is_ascii_digit()) || (p.is_ascii_digit() && c.is_uppercase())) {
            out.push(' ');
        }
        if out.is_empty() {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        last = Some(c);
    }
    out.trim_end().to_string()
}

/// Where a tile block's picture will be, empty: a well ruled in tiles, what to do in it.
fn empty_frame(ui: &mut egui::Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.add(ta_ui::metal::well(rect, INSET));
    let grid = Stroke::new(1.0, Color32::from_white_alpha(7));
    let cell = 24.0;
    let mut x = rect.min.x + (rect.width() % cell) / 2.0;
    while x < rect.max.x {
        painter.vline(x, rect.y_range(), grid);
        x += cell;
    }
    let mut y = rect.min.y + (rect.height() % cell) / 2.0;
    while y < rect.max.y {
        painter.hline(rect.x_range(), y, grid);
        y += cell;
    }
    // (Centred line by line.)
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat { font_id: body(12.5), color: AMBER, ..Default::default() });
    job.wrap.max_width = rect.width() - 32.0;
    job.halign = egui::Align::Center;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    painter.galley(pos2(rect.center().x, rect.center().y - galley.size().y / 2.0), galley, AMBER);
}

/// A tile block's picture, as big as the panel allows, framed.
fn block_frame(ui: &mut egui::Ui, tex: &egui::TextureHandle, most_height: f32) {
    let size = tex.size_vec2();
    let scale = (ui.available_width() / size.x).min(most_height / size.y).min(4.0);
    let (rect, _) = ui.allocate_exact_size(size * scale, Sense::hover());
    ui.painter().image(tex.id(), rect, ERect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, LINE_HI), StrokeKind::Outside);
}

/// The brush from the side, in a well: the ground across it before (a thin line) and as the
/// tool leaves it, its width as the radius on the slider's scale and its height the strength
/// (Level: the level, over the whole brush).
fn brush_profile(ui: &mut egui::Ui, tool: Tool, radius: f32, strength: f32, level: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.add(ta_ui::metal::well(rect, INSET));
    let inner = rect.shrink2(vec2(10.0, 7.0));
    let span = ((radius / 16.0).ln() / (1024.0f32 / 16.0).ln()).clamp(0.0, 1.0);
    let half = inner.width() * (0.1 + 0.36 * span);
    let (cx, h) = (inner.center().x, inner.height());
    // Heights above the bottom of the well, in points, across the well (x from -1 to 1).
    let ground = |x: f32| -> f32 {
        match tool {
            Tool::Raise => h * 0.25,
            Tool::Lower => h * 0.75,
            Tool::Smooth => h * 0.45 + (x * 19.0).sin() * h * 0.12 + (x * 47.0).sin() * h * 0.07,
            Tool::Flatten => h * (0.2 + 0.6 * (x * 0.5 + 0.5)),
            _ => h * 0.5 + (x * 9.0).sin() * h * 0.12 + x * h * 0.2,
        }
    };
    let after = |x: f32| -> f32 {
        let d = (x * inner.width() * 0.5).abs() / half;
        let g = ground(x);
        if d >= 1.0 {
            return g;
        }
        let falloff = (1.0 - d * d).powi(2) * strength;
        match tool {
            Tool::Raise => g + falloff * h * 0.7,
            Tool::Lower => g - falloff * h * 0.7,
            Tool::Smooth => g + (h * 0.45 - g) * falloff,
            Tool::Flatten => g + (ground(0.0) - g) * falloff,
            _ => h * (0.1 + 0.8 * level / 255.0),
        }
    };
    let n = 96;
    let at = |x: f32, y: f32| pos2(cx + x * inner.width() * 0.5, inner.max.y - y.clamp(0.0, h));
    let before: Vec<Pos2> = (0..=n).map(|i| -1.0 + 2.0 * i as f32 / n as f32).map(|x| at(x, ground(x))).collect();
    let shaped: Vec<Pos2> = (0..=n).map(|i| -1.0 + 2.0 * i as f32 / n as f32).map(|x| at(x, after(x))).collect();
    // The ground as the tool leaves it, filled down to the well's floor.
    let floor = rect.max.y - 2.0;
    let fill = Color32::from_rgba_unmultiplied(216, 180, 86, 34);
    for pair in shaped.windows(2) {
        painter.add(Shape::convex_polygon(vec![pair[0], pair[1], pos2(pair[1].x, floor), pos2(pair[0].x, floor)], fill, Stroke::NONE));
    }
    painter.add(Shape::line(before, Stroke::new(1.0, STEEL_DIM.gamma_multiply(0.7))));
    painter.add(Shape::line(shaped, Stroke::new(1.8, AMBER)));
    // The brush's edges.
    for x in [cx - half, cx + half] {
        painter.vline(x, inner.y_range(), Stroke::new(1.0, AMBER_DIM.gamma_multiply(0.5)));
    }
}

/// Pixels per tile of the picker's picture: at most 16, and at most 4096 pixels a side.
fn picture_scale(tw: u32, th: u32) -> u32 {
    (4096 / tw.max(th).max(1)).clamp(1, 16)
}

/// A map's art at `k` pixels per tile (each tile box-filtered once, then laid out).
fn map_picture(map: &TntMap, palette: &Palette, k: u32) -> egui::ColorImage {
    let thumbs: Vec<Vec<[u8; 4]>> = map.tiles.iter().map(|t| tile_thumb(t, palette, k)).collect();
    let (tw, th) = ((map.width / 2) as usize, (map.height / 2) as usize);
    let k = k as usize;
    let (w, h) = (tw * k, th * k);
    let mut rgba = vec![0u8; w * h * 4];
    for tz in 0..th {
        for tx in 0..tw {
            let t = map.tile_map[tz * tw + tx] as usize;
            let Some(thumb) = thumbs.get(t) else { continue };
            for y in 0..k {
                for x in 0..k {
                    let o = ((tz * k + y) * w + tx * k + x) * 4;
                    rgba[o..o + 4].copy_from_slice(&thumb[y * k + x]);
                }
            }
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([w.max(1), h.max(1)], &rgba)
}

/// One tile scaled to `k` × `k` pixels (the mean of each block).
fn tile_thumb(tile: &[u8; TILE_SIZE * TILE_SIZE], palette: &Palette, k: u32) -> Vec<[u8; 4]> {
    let k = k.clamp(1, 32) as usize;
    let step = TILE_SIZE / k;
    let mut out = Vec::with_capacity(k * k);
    for y in 0..k {
        for x in 0..k {
            let mut sum = [0u32; 3];
            for yy in 0..step {
                for xx in 0..step {
                    let [r, g, b] = palette.rgb(tile[(y * step + yy) * TILE_SIZE + x * step + xx]);
                    sum[0] += r as u32;
                    sum[1] += g as u32;
                    sum[2] += b as u32;
                }
            }
            let n = (step * step).max(1) as u32;
            out.push([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]);
        }
    }
    out
}

/// The brush block's picture (up to 256 pixels a side).
fn block_picture(doc: &MapDoc, block: &super::tools::TileBlock, palette: &Palette) -> egui::ColorImage {
    let k = (256 / block.w.max(block.h).max(1)).clamp(2, 32).next_power_of_two().min(32);
    let k = if 32 % k == 0 { k } else { 8 };
    let (w, h) = ((block.w * k) as usize, (block.h * k) as usize);
    let mut rgba = vec![0u8; w * h * 4];
    let k = k as usize;
    for bz in 0..block.h as usize {
        for bx in 0..block.w as usize {
            let t = block.tiles[bz * block.w as usize + bx] as usize;
            let Some(tile) = doc.tnt.tiles.get(t) else { continue };
            let thumb = tile_thumb(tile, palette, k as u32);
            for y in 0..k {
                for x in 0..k {
                    let o = ((bz * k + y) * w + bx * k + x) * 4;
                    rgba[o..o + 4].copy_from_slice(&thumb[y * k + x]);
                }
            }
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([w.max(1), h.max(1)], &rgba)
}

/// A feature's first sprite frame, for the palette's preview (`None` for 3D wrecks).
fn feature_picture(vfs: &ta_vfs::Vfs, def: &ta_data::FeatureDef, palette: &Palette) -> Option<egui::ColorImage> {
    if def.filename.is_empty() {
        return None;
    }
    let gaf = ta_formats::gaf::Gaf::parse(&vfs.read_opt(&format!("anims/{}.gaf", def.filename.to_ascii_lowercase()))?).ok()?;
    let frame = gaf.entry(&def.seq_name)?.frames.first()?;
    let img = &frame.image;
    // (A mod's feature may have an empty first frame: no picture rather than a crash.)
    if img.width == 0 || img.height == 0 {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_rgba(palette)))
}

/// The tool panel's value boxes: as wide as its widest value.
fn brush_value_box(ui: &egui::Ui) -> f32 {
    w::value_box_width(ui, &["1024 px", "100%", "255"])
}
