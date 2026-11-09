//! Around the view: the status bar under it (where the pointer is and what is there, what the
//! tool's buttons and keys do), the switches over it (what the view shows), the map's problems,
//! and the page of keys and mouse buttons (F1).

use egui::{Align2, Color32, CornerRadius, Rect as ERect, RichText, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use super::checks::Problem;
use super::{Editor, Tool};
use crate::setup::Assets;
use crate::theme::*;

/// Height of the status bar.
const STATUS_HEIGHT: f32 = 28.0;

/// What the left button and the keys do with each tool, for the status bar.
fn tool_keys(tool: Tool) -> &'static str {
    match tool {
        Tool::Raise => n("Left: raise · Shift: lower · [ ]: brush size"),
        Tool::Lower => n("Left: lower · Shift: raise · [ ]: brush size"),
        Tool::Smooth => n("Left: smooth · [ ]: brush size"),
        Tool::Flatten => n("Left: flatten to the height where you start · [ ]: brush size"),
        Tool::Level => n("Left: set the ground to the level · Ctrl+click: take the level · [ ]: brush size"),
        Tool::Tiles => n("Left: paint · Ctrl+drag: take tiles from the map"),
        Tool::Copy => n("Drag: copy a region · Left: paste it · Esc: let go of the copy"),
        Tool::Features => n("Left: place · Ctrl+drag: remove · Alt+click: take one from the map · [ ]: brush size"),
        Tool::Starts => n("Left: add or drag one · Ctrl+click: remove one"),
    }
}

impl Editor {
    /// The bar under the view: the ground under the pointer, then what the tool's buttons do,
    /// and the way to the keys page.
    pub(super) fn status_bar(&mut self, root: &mut egui::Ui, assets: &Assets) {
        let frame = egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 10, top: 4, bottom: 4 });
        let bar = egui::Panel::bottom("editor_status").exact_size(STATUS_HEIGHT).frame(frame).show(root, |ui| {
            let background = ui.painter().add(Shape::Noop);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                match self.cursor {
                    Some(p) => {
                        let mut parts = vec![format!("X {:.0} · Z {:.0}", p.x, p.z), tf("height {h}", &[("h", &format!("{:.0}", p.y))])];
                        let sea = self.doc.tnt.sea_level as f32;
                        if p.y < sea {
                            parts.push(tf("{d} under water", &[("d", &format!("{:.0}", sea - p.y))]));
                        }
                        let art = self.doc.art_tile_at(p.x, p.y, p.z);
                        if art.0 >= 0 && art.1 >= 0 && (art.0 as u32) < self.doc.tiles_wide() && (art.1 as u32) < self.doc.tiles_high() {
                            parts.push(tf("tile {x}, {z}", &[("x", &art.0), ("z", &art.1)]));
                        }
                        let cell = ((p.x / super::doc::CELL_PX) as u32, (p.z / super::doc::CELL_PX) as u32);
                        if let Some((_, name)) = self.doc.feature_covering(&assets.data, cell.0, cell.1) {
                            parts.push(super::ui::feature_title(name));
                        }
                        ui.label(RichText::new(parts.join("  ·  ")).font(strong(12.0)).color(STEEL));
                    }
                    None => {
                        ui.label(RichText::new(t("Point at the map")).font(body(12.0)).color(STEEL_DIM));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(ActionButton::secondary("KEYS & MOUSE").height(20.0).min_width(96.0)).on_hover_text(t("Every key and mouse button of the editor (F1)")).clicked() {
                        self.help = !self.help;
                    }
                    ui.add(egui::Label::new(RichText::new(t(tool_keys(self.tool))).font(body(12.0)).color(STEEL_DIM)).truncate());
                });
            });
            background
        });
        let r = bar.response.rect;
        let plate = Shape::Vec(vec![
            ta_ui::metal::plate(root.ctx(), r, BG_DEEP),
            Shape::hline(r.x_range(), r.min.y + 0.5, Stroke::new(1.0, Color32::from_black_alpha(220))),
            Shape::hline(r.x_range(), r.min.y + 1.5, Stroke::new(1.0, Color32::from_white_alpha(28))),
        ]);
        root.painter().set(bar.inner, plate);
    }

    /// The view's switches, at the top right of `view`: the tile grid, the height map, the
    /// features and the start positions shown or not, and the whole map framed.
    pub(super) fn view_bar(&mut self, ctx: &egui::Context, view: ERect) {
        egui::Area::new(egui::Id::new("editor_view_bar")).anchor(Align2::RIGHT_TOP, [-(ctx.content_rect().max.x - view.max.x) - 12.0, view.min.y + 10.0])
            .order(egui::Order::Middle).show(ctx, |ui| {
            egui::Frame::new().fill(Color32::from_black_alpha(170)).stroke(Stroke::new(1.0, LINE_HI)).corner_radius(3).inner_margin(egui::Margin::same(5)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let toggle = |ui: &mut egui::Ui, label: &str, key: &str, hint: &str, on: &mut bool| {
                        let response = switch_chip(ui, label, *on).on_hover_text(if key.is_empty() { t(hint).to_string() } else { format!("{} ({key})", t(hint)) });
                        if response.clicked() {
                            *on = !*on;
                        }
                    };
                    toggle(ui, n("GRID"), "G", n("The tiles' grid over the ground"), &mut self.layers.grid);
                    toggle(ui, n("HEIGHTS"), "H", n("The ground coloured by height, a contour every 16"), &mut self.height_map);
                    toggle(ui, n("FEATURES"), "", n("Trees, rocks, wrecks and the other features"), &mut self.layers.features);
                    toggle(ui, n("STARTS"), "", n("The start positions' numbered markers"), &mut self.layers.starts);
                    toggle(ui, n("BLOCKED"), "", n("Impassable ground, in red: no unit can cross it"), &mut self.layers.blocked);
                    ui.add_space(4.0);
                    if ui.add(ActionButton::secondary("FRAME MAP").height(22.0).min_width(80.0)).on_hover_text(t("The whole map in view (Home)")).clicked() {
                        self.requests.frame_map = true;
                    }
                });
            });
        });
    }

    /// The problems' chip in the top bar: how many there are, and the list on hover; a click on
    /// one with a place takes the camera there.
    pub(super) fn problems_chip(&mut self, ui: &mut egui::Ui) {
        let problems = self.problems.clone();
        if problems.is_empty() {
            return;
        }
        let blocking = problems.iter().any(|p| p.blocking);
        let color = if blocking { RED } else { AMBER };
        let label = tf("PROBLEMS: {n}", &[("n", &problems.len())]);
        let text = w::galley(ui, RichText::new(&label).font(strong(11.5)).color(color));
        let (rect, response) = ui.allocate_exact_size(vec2(text.size().x + 26.0, 22.0), Sense::click());
        let p = ui.painter();
        p.rect(rect, CornerRadius::same(2), if response.hovered() { PANEL_HI } else { INSET }, Stroke::new(1.0, color.gamma_multiply(0.7)), StrokeKind::Inside);
        // (A warning triangle before the words.)
        let c = pos2(rect.min.x + 11.0, rect.center().y);
        p.add(Shape::convex_polygon(vec![c + vec2(0.0, -6.0), c + vec2(6.0, 5.0), c + vec2(-6.0, 5.0)], color, Stroke::NONE));
        p.text(c + vec2(0.0, 1.5), Align2::CENTER_CENTER, "!", strong(9.0), Color32::BLACK);
        p.galley(pos2(rect.min.x + 20.0, rect.center().y - text.size().y / 2.0), text, color);
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        egui::Popup::menu(&response).width(360.0).show(|ui| {
            ui.label(w::caption("WHAT TO PUT RIGHT"));
            for problem in &problems {
                let color = if problem.blocking { RED } else { AMBER };
                let row = ui.add(egui::Button::new(RichText::new(&problem.text).color(color)).wrap().frame(false));
                if problem.at.is_some() {
                    let row = row.on_hover_text(t("Show me"));
                    if row.clicked() {
                        self.requests.look_at = problem.at;
                        ui.close();
                    }
                }
            }
            ui.add_space(4.0);
            ui.label(RichText::new(t("Problems in red stop the map being saved; the others are only warnings.")).font(body(11.5)).color(STEEL_DIM));
        });
    }

    /// The problems, worked out again twice a second (the name's check reads the mods folder).
    pub(super) fn update_problems(&mut self, assets: &Assets, dt: f32) {
        self.problems_age += dt;
        if self.problems_age < 0.5 && self.problems_version == Some(self.history.version) {
            return;
        }
        self.problems_age = 0.0;
        self.problems_version = Some(self.history.version);
        let problems: Vec<Problem> = super::checks::problems(&self.doc, &assets.vfs, &super::save::user_mods_dir());
        self.problems = problems;
    }

    /// The keys and the mouse's buttons, all of them.
    pub(super) fn help_page(&mut self, ctx: &egui::Context, room: ERect) {
        let page = Panel::new("KEYS AND MOUSE").subtitle("EVERYTHING THE EDITOR ANSWERS TO").width(680.0).exit(Exit::Close);
        let groups: [(&str, &[(&str, &str)]); 4] = [
            (n("THE VIEW"), &[
                (n("Right-drag"), n("Turn the view round")),
                (n("Middle-drag"), n("Move the view")),
                (n("Arrows"), n("Move the view")),
                (n("Wheel"), n("Zoom in and out")),
                ("Home", n("The whole map in view")),
                ("G", n("The tiles' grid")),
                ("H", n("The height map")),
            ]),
            (n("THE TOOLS"), &[
                ("1-5", n("Raise, lower, smooth, flatten, level")),
                ("6-9", n("Tiles, copy, features, start positions")),
                ("[ ]", n("Brush or eraser smaller, larger")),
                ("Shift", n("Raise and lower swap")),
                ("Ctrl", n("Take tiles, take the level, remove features or a start position")),
                ("Alt+click", n("Take the feature under the pointer")),
                ("Esc", n("Let go of a selection or a copy")),
            ]),
            (n("THE MAP"), &[
                ("Ctrl+N", n("New map")),
                ("Ctrl+O", n("Open a map")),
                ("Ctrl+S", n("Save")),
                ("Ctrl+Shift+S", n("Save under another name")),
                ("F5", n("Save and play it")),
                (n("Drop a picture"), n("Bring it in: heights, art or a tile brush")),
            ]),
            (n("EDITS"), &[
                ("Ctrl+Z", n("Undo")),
                ("Ctrl+Y", n("Redo")),
                ("Ctrl+Shift+Z", n("Redo")),
                ("F1", n("This page")),
            ]),
        ];
        let response = panel::modal_within(ctx, "editor_help", page, true, room, |ui| {
            ui.columns(2, |cols| {
                for (k, (title, keys)) in groups.iter().enumerate() {
                    let ui = &mut cols[k % 2];
                    panel::section(ui, title);
                    for (key, what) in keys.iter() {
                        ui.horizontal(|ui| {
                            let cap = w::galley(ui, RichText::new(t(key)).font(strong(11.5)));
                            let (rect, _) = ui.allocate_exact_size(vec2(cap.size().x.max(18.0) + 12.0, 20.0), Sense::hover());
                            ui.painter().rect(rect, CornerRadius::same(2), INSET, Stroke::new(1.0, LINE_HI), StrokeKind::Inside);
                            ui.painter().galley(rect.center() - cap.size() / 2.0, cap, TEXT_BRIGHT);
                            ui.add(egui::Label::new(RichText::new(t(what)).font(body(12.5)).color(TEXT)).wrap());
                        });
                    }
                    ui.add_space(8.0);
                }
            });
        });
        if response.exit {
            self.help = false;
        }
    }
}

/// A switch in the view's bar: lit when on.
fn switch_chip(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
    let text = w::galley(ui, RichText::new(t(label)).font(strong(11.0)).extra_letter_spacing(ta_ui::i18n::tracking(0.6)));
    let (rect, response) = ui.allocate_exact_size(vec2(text.size().x + 30.0, 22.0), Sense::click());
    let (fill, border, color) = if on { (AMBER_DEEP, AMBER, AMBER_BRIGHT) } else if response.hovered() { (PANEL_HI, AMBER_DIM, TEXT_BRIGHT) } else { (INSET, LINE_HI, STEEL) };
    let p = ui.painter();
    p.rect(rect, CornerRadius::same(2), fill, Stroke::new(1.0, border), StrokeKind::Inside);
    // (A lamp: lit when on.)
    let lamp = pos2(rect.min.x + 10.0, rect.center().y);
    p.circle(lamp, 3.5, if on { AMBER_BRIGHT } else { Color32::from_black_alpha(160) }, Stroke::new(1.0, Color32::from_black_alpha(200)));
    p.galley(pos2(rect.min.x + 19.0, rect.center().y - text.size().y / 2.0), text, color);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, t(label)));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
