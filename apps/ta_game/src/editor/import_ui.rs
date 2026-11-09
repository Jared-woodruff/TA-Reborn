//! The Import dialog: a picture (chosen, or dropped on the window) brought into the map as its
//! heights or as its ground's art (the whole map, or a brush of tiles to paint with), and the
//! heights written out as a grey picture. The work itself is [`super::import`]'s.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Rect as ERect, RichText, vec2};
use image::RgbaImage;
use ta_ui::i18n::{grouped, n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use super::ui::{button_row, pair};
use super::{Dialog, Editor};
use crate::theme::*;

/// What the picture becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportAs {
    Heights,
    /// The whole map's art.
    Art,
    /// A brush of tiles for the Tiles tool.
    Brush,
}

/// The picture being brought in and how.
pub struct Import {
    pub path: Option<PathBuf>,
    pub picture: Option<Arc<RgbaImage>>,
    pub error: Option<String>,
    preview: Option<egui::TextureHandle>,
    pub what: ImportAs,
    pub lowest: u8,
    pub highest: u8,
    /// The brush's longer side, in tiles.
    pub brush: u32,
    pub dither: bool,
}

impl Default for Import {
    fn default() -> Self {
        Import { path: None, picture: None, error: None, preview: None, what: ImportAs::Art, lowest: 0, highest: 160, brush: 8, dither: true }
    }
}

/// A picture to bring in, as the dialog asked (carried out by the next update).
pub struct ImportJob {
    pub picture: Arc<RgbaImage>,
    pub what: ImportAs,
    pub lowest: u8,
    pub highest: u8,
    /// Tiles across and down (the whole map's, or the brush's).
    pub tiles: (u32, u32),
    pub dither: bool,
}

impl Import {
    /// Reads `path`'s picture (its preview made when next shown).
    pub fn load(&mut self, path: &Path) {
        self.path = Some(path.to_path_buf());
        self.preview = None;
        match super::import::read(path) {
            Ok(picture) => {
                self.picture = Some(Arc::new(picture));
                self.error = None;
            }
            Err(error) => {
                self.picture = None;
                self.error = Some(format!("{error:#}"));
            }
        }
    }

    /// The brush's size in tiles: `brush` on its longer side, the other in proportion.
    fn brush_tiles(&self) -> (u32, u32) {
        let Some(p) = &self.picture else { return (self.brush, self.brush) };
        let (pw, ph) = (p.width().max(1) as f32, p.height().max(1) as f32);
        let long = self.brush.max(1) as f32;
        if pw >= ph { (long as u32, ((long * ph / pw).round() as u32).max(1)) } else { (((long * pw / ph).round() as u32).max(1), long as u32) }
    }
}

impl Editor {
    /// Opens the Import dialog (as `what`), asking for a picture when there is none yet.
    pub(super) fn open_import(&mut self, what: ImportAs) {
        self.finish_stroke();
        self.selecting = None;
        self.import.what = what;
        self.dialog = Dialog::Import;
    }

    /// A file dropped on the window: a picture, into the Import dialog.
    pub fn dropped(&mut self, path: &Path) {
        if !matches!(self.dialog, Dialog::None | Dialog::Import) {
            return;
        }
        self.import.load(path);
        if self.import.picture.is_some() && self.dialog == Dialog::None {
            self.import.what = if self.tool.height_op() { ImportAs::Heights } else { ImportAs::Art };
        }
        self.open_import(self.import.what);
    }

    /// The system's file chooser for a picture.
    fn choose_picture(&mut self) {
        if let Some(path) = rfd::FileDialog::new().set_title(t("A picture to bring into the map")).add_filter(t("Pictures"), &["png", "jpg", "jpeg"]).pick_file() {
            self.import.load(&path);
        }
    }

    /// The heights written out as a grey picture, a pixel a cell.
    pub(super) fn export_heights(&mut self) {
        let name = format!("{} heights.png", super::save::file_stem(&self.doc.props.name));
        let Some(path) = rfd::FileDialog::new().set_title(t("Save the heights as a picture")).add_filter(t("Pictures"), &["png"]).set_file_name(name).save_file() else { return };
        match super::import::heights_picture(&self.doc).save_with_format(&path, image::ImageFormat::Png) {
            Ok(()) => self.message(tf("The heights are saved as a picture: {path}", &[("path", &path.display())]), false),
            Err(error) => self.message(tf("Could not save the heights: {error}", &[("error", &error)]), true),
        }
    }

    pub(super) fn import_dialog(&mut self, ctx: &egui::Context, room: ERect) {
        let page = Panel::new("IMPORT A PICTURE").subtitle("AS THE GROUND'S HEIGHTS, OR AS ITS ART").width(620.0).exit(Exit::Close);
        let mut import = false;
        let mut export = false;
        let (tw, th) = (self.doc.tiles_wide(), self.doc.tiles_high());
        let sea = self.doc.tnt.sea_level;
        let tiles_now = self.doc.tnt.tiles.len();
        let response = panel::modal_within(ctx, "editor_import", page, true, room, |ui| {
            let im = &mut self.import;
            // The picture: its preview, its name and size, and the way to choose another.
            ui.horizontal_top(|ui| {
                let side = 150.0;
                if im.preview.is_none() && let Some(p) = &im.picture {
                    im.preview = Some(ui.ctx().load_texture("editor-import-preview", preview(p, 300), egui::TextureOptions::LINEAR));
                }
                let (rect, _) = ui.allocate_exact_size(vec2(side, side), egui::Sense::hover());
                ui.painter().rect_filled(rect, 3.0, egui::Color32::from_black_alpha(120));
                match &im.preview {
                    Some(tex) => {
                        let size = tex.size_vec2();
                        let scale = (side / size.x.max(size.y).max(1.0)).min(4.0);
                        ui.painter().image(tex.id(), ERect::from_center_size(rect.center(), size * scale), ERect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                    }
                    None => {
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, t("No picture"), body(12.0), STEEL_DIM);
                    }
                }
                ui.vertical(|ui| {
                    match (&im.path, &im.picture) {
                        (Some(path), Some(p)) => {
                            ui.add(egui::Label::new(RichText::new(path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())).font(strong(14.0)).color(TEXT_BRIGHT)).truncate());
                            ui.label(RichText::new(format!("{} × {} px", p.width(), p.height())).color(STEEL));
                        }
                        _ => {
                            ui.add(egui::Label::new(RichText::new(t("Choose a picture, or drop one on the window: PNG or JPEG.")).color(STEEL)).wrap());
                        }
                    }
                    if let Some(error) = &im.error {
                        ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap());
                    }
                    ui.add_space(6.0);
                    // (Not the profile's CHOOSE PICTURE…, a portrait in some languages.)
                    if ui.add(ActionButton::secondary("CHOOSE A PICTURE…").height(30.0)).clicked() {
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("editor_import_choose"), true));
                    }
                });
            });
            ui.add_space(8.0);
            ui.label(w::caption("USE IT AS"));
            let mut what = im.what;
            w::segmented_fill(ui, &mut what, &[(ImportAs::Heights, n("HEIGHTS")), (ImportAs::Art, n("THE WHOLE MAP'S ART")), (ImportAs::Brush, n("A TILE BRUSH"))]);
            im.what = what;
            ui.add_space(4.0);
            let value_box = w::value_box_width(ui, &["255"]);
            match im.what {
                ImportAs::Heights => {
                    pair(
                        ui,
                        |ui| panel::setting(ui, "LOWEST", Some("Where the picture is black"), |ui| {
                            w::value_slider(ui, egui::Slider::new(&mut im.lowest, 0..=255), value_box);
                        }),
                        |ui| panel::setting(ui, "HIGHEST", Some("Where it is white"), |ui| {
                            w::value_slider(ui, egui::Slider::new(&mut im.highest, 0..=255), value_box);
                        }),
                    );
                    ui.add(egui::Label::new(RichText::new(tf("The map's sea level is {sea}: lower ground is under water. The picture is stretched to the map's {w} × {h} cells.",
                        &[("sea", &sea), ("w", &(tw * 2)), ("h", &(th * 2))])).font(body(12.0)).color(STEEL)).wrap());
                    // The way round: the heights out to a paint program, and back.
                    ui.add_space(4.0);
                    if ui.add(ActionButton::secondary("EXPORT HEIGHTS…").height(30.0)).on_hover_text(t("The ground's heights as a grey picture, a pixel a cell")).clicked() {
                        export = true;
                    }
                }
                ImportAs::Art | ImportAs::Brush => {
                    if im.what == ImportAs::Brush {
                        let (bw, bh) = im.brush_tiles();
                        // (The hint says what the number counts.)
                        panel::setting(ui, "BRUSH SIZE", Some("Tiles on its longer side"), |ui| {
                            w::value_slider(ui, egui::Slider::new(&mut im.brush, 1..=64), value_box);
                        });
                        ui.label(RichText::new(tf("A brush of {w} × {h} tiles for the Tiles tool", &[("w", &bw), ("h", &bh)])).font(body(12.0)).color(STEEL));
                    } else {
                        ui.add(egui::Label::new(RichText::new(tf("The picture is stretched over the map's {w} × {h} tiles and painted in TA's colours. Its heights stay as they are.", &[("w", &tw), ("h", &th)]))
                            .font(body(12.0)).color(STEEL)).wrap());
                    }
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut im.dither, RichText::new(t("Dither")).color(TEXT));
                        ui.label(RichText::new(t("mixes TA's colours for smoother shades")).font(body(12.0)).color(STEEL_DIM));
                    });
                    let (aw, ah) = if im.what == ImportAs::Brush { im.brush_tiles() } else { (tw, th) };
                    ui.label(RichText::new(tf("At most {new} new tiles; the map has {now} of {max}.", &[("new", &grouped((aw * ah) as u64)), ("now", &grouped(tiles_now as u64)),
                        ("max", &grouped(ta_formats::tnt::MAX_TILES as u64))])).font(body(12.0)).color(STEEL_DIM));
                }
            }
            ui.add_space(8.0);
            button_row(ui, 36.0, |ui| {
                if ui.add_enabled(im.picture.is_some(), ActionButton::primary("IMPORT").height(36.0).min_width(140.0)).clicked() {
                    import = true;
                }
                if ui.add(ActionButton::secondary("CANCEL").height(36.0).min_width(110.0)).clicked() {
                    self.dialog = Dialog::None;
                }
            });
        });
        // (The file chooser blocks: opened outside the dialog's frame.)
        if ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("editor_import_choose"))).unwrap_or(false) {
            self.choose_picture();
        }
        if response.exit {
            self.dialog = Dialog::None;
        }
        if export {
            self.requests.export_heights = true;
        }
        if import && let Some(picture) = self.import.picture.clone() {
            let im = &self.import;
            let tiles = if im.what == ImportAs::Brush { im.brush_tiles() } else { (tw, th) };
            self.requests.import = Some(ImportJob { picture, what: im.what, lowest: im.lowest.min(im.highest), highest: im.highest.max(im.lowest), tiles, dither: im.dither });
            self.dialog = Dialog::None;
        }
    }
}

/// The picture at most `side` pixels a side, for the dialog.
fn preview(picture: &RgbaImage, side: u32) -> egui::ColorImage {
    let (w, h) = (picture.width().max(1), picture.height().max(1));
    let scale = (side as f32 / w.max(h) as f32).min(1.0);
    let small = image::imageops::thumbnail(picture, ((w as f32 * scale) as u32).max(1), ((h as f32 * scale) as u32).max(1));
    egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw())
}
