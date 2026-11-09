//! The unit preview and the skins' thumbnails, made on a worker with a graphics device of its
//! own. One replaceable request (the details' preview, or the editor's) comes first, and stale
//! work never replaces a newer one's pictures; thumbnails queue behind it, each read from the
//! disk cache when it was made before.

use ta_ui::tooltip::TooltipExt as _;
use std::{collections::{HashMap, HashSet, VecDeque}, path::PathBuf, sync::{Arc, Condvar, Mutex, atomic::{AtomicBool, Ordering}}, thread::JoinHandle, time::Duration};
use egui::{ColorImage, TextureHandle, TextureOptions, Ui};
use ta_ui::i18n::{t, tf};
use crate::theme::*;
use crate::widgets;
use super::Reference;

#[path = "preview_pose.rs"]
mod pose;

/// The angle thumbnails are seen from (radians round from the front).
const THUMB_YAW: f32 = 0.6;
/// A thumbnail: two units side by side, each cut to this share of its square picture.
const THUMB_SIDE: u32 = 256;
const THUMB_KEEP: u32 = 192;
/// Under a thumbnail (its picture has no alpha on disk): the card's well.
const WELL: [u8; 3] = [9, 10, 8];

/// What the preview paints the units with.
pub enum Paint {
    /// An installed or built-in skin, by id.
    Saved(String),
    /// The editor's skin: its models made again when `revision` changes, its livery painted live.
    Editing { pack: Arc<ta_skins::Pack>, serial: u64, revision: u64, livery: Option<ta_skins::Livery> },
}

/// A unit the preview can show.
#[derive(Clone, Debug)]
pub struct UnitChoice {
    pub name: String,
    pub display: String,
    pub arm: bool,
}

/// A unit's textures a skin may repaint, each with a small picture of its own paint.
pub struct Slot {
    pub unit: String,
    pub display: String,
    pub textures: Vec<(String, TextureHandle)>,
}

/// What a preview shows: the game's data (`source`, as of the library's `revision`), the skin
/// (an id, or the editor's at its `textures` revision) and the units.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key { source: String, root: PathBuf, skin: String, revision: u64, textures: u64, units: [String; 2] }

impl Key {
    /// The game's data it is made from.
    fn data(&self) -> String { data_key(&self.source, self.revision) }
}

/// The game's data a picture is made from, as of the library's revision.
fn data_key(source: &str, revision: u64) -> String { format!("{source}#{revision}") }

enum Painting {
    Saved,
    Editing { pack: Arc<ta_skins::Pack>, livery: Option<ta_skins::Livery> },
}

struct Request { key: Key, reference: Reference, yaw: f32, ctx: egui::Context, paint: Painting }

/// A unit's slots as the worker finds them: its name, its display name, its textures' names
/// and pictures.
type SlotImages = Vec<(String, String, Vec<(String, ColorImage)>)>;

struct Pictures { images: Vec<ta_render::preview::PreviewImage>, names: Vec<String>, slots: Arc<SlotImages>, units: Arc<Vec<UnitChoice>> }
struct Reply { key: Key, pictures: Result<Pictures, String> }

struct Thumb { key: String, skin: String, revision: u64, root: PathBuf, reference: Reference, cache: PathBuf }

#[derive(Default)]
struct Shared {
    request: Mutex<Option<Request>>,
    reply: Mutex<Option<Reply>>,
    thumbs: Mutex<VecDeque<Thumb>>,
    made: Mutex<Vec<(String, Result<ColorImage, String>)>>,
    /// Where to ask for a repaint when a thumbnail is made.
    ctx: Mutex<Option<egui::Context>>,
    ready: Condvar,
    stop: AtomicBool,
}

#[derive(Default)]
pub struct Preview {
    shared: Option<Arc<Shared>>,
    worker: Option<JoinHandle<()>>,
    key: Option<Key>,
    textures: Vec<TextureHandle>,
    names: Vec<String>,
    error: Option<String>,
    waiting: bool,
    yaw: f32,
    previous: Option<f64>,
    last_request: f64,
    paused: bool,
    redraw: bool,
    visible: bool,
    /// The livery last painted (the editor's): another is a picture to make.
    livery: Option<ta_skins::Livery>,
    slots: Vec<Slot>,
    slots_from: Option<Arc<SlotImages>>,
    units: Arc<Vec<UnitChoice>>,
    thumbs: HashMap<String, TextureHandle>,
    asked: HashSet<String>,
    failed: HashSet<String>,
}

impl Preview {
    /// Visibility belongs to this frame. A wide details pane can disappear when
    /// the library reflows, without another call to `show` to clear the flag.
    pub fn begin_frame(&mut self) { self.visible = false; }

    pub fn ready(&self) -> bool { (!self.visible || self.error.is_some() || !self.textures.is_empty()) && self.thumbnails_idle() }

    /// Whether every thumbnail asked for is made (or could not be).
    pub fn thumbnails_idle(&self) -> bool {
        self.asked.iter().all(|key| self.thumbs.contains_key(key) || self.failed.contains(key))
    }

    /// The units the preview can show (known once it has shown one).
    pub fn units(&self) -> &[UnitChoice] { &self.units }

    /// The shown units' textures a skin may repaint.
    pub fn slots(&self) -> &[Slot] { &self.slots }

    /// The preview of `paint` on two units (`units`: Arm's and Core's, empty for the usual
    /// examples) turning, with its controls under it.
    #[allow(clippy::too_many_arguments)]
    pub fn show(&mut self, ui: &mut Ui, reference: Reference, root: PathBuf, paint: Paint, revision: u64, units: &[String; 2], height: f32) {
        let source = source_key(&reference);
        let (skin, painting, textures) = match paint {
            Paint::Saved(id) => (id, Painting::Saved, 0),
            Paint::Editing { pack, serial, revision: textures, livery } => {
                if self.livery != livery { self.livery = livery.clone(); self.redraw = true; }
                (format!("<editing:{serial}>"), Painting::Editing { pack, livery }, textures)
            }
        };
        let key = Key { source, root, skin, revision, textures, units: units.clone() };
        if self.key.as_ref() != Some(&key) {
            // (The editor's models made again keep its pictures until the new ones come.)
            let editing = matches!(painting, Painting::Editing { .. }) && self.key.as_ref().is_some_and(|old| old.skin == key.skin);
            self.key = Some(key.clone());
            if !editing { self.textures.clear(); self.names.clear(); }
            self.error = None; self.waiting = false;
            self.last_request = f64::NEG_INFINITY;
            self.redraw = true;
        }
        self.poll(ui.ctx());
        let now = ui.input(|i| i.time);
        let still = self.paused || ta_ui::motion::still(ui.ctx());
        let width = ui.available_width().max(1.0);
        let side = preview_side(width, height);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, side + 34.0), egui::Sense::hover());
        let visible = ui.is_rect_visible(rect);
        self.visible = visible;
        self.yaw = advance_yaw(self.yaw, self.previous, now, still || !visible);
        self.previous = Some(now);
        if visible && self.error.is_none() && !self.waiting &&
            frame_due(!self.textures.is_empty(), self.redraw, still, now - self.last_request) {
            if reference.game_dir.is_none() {
                self.error = Some(t("Choose your Total Annihilation installation on PLAY to preview units.").into());
            } else {
                self.submit(Request { key, reference, yaw: self.yaw, ctx: ui.ctx().clone(), paint: painting });
                self.last_request = now;
                self.redraw = false;
            }
        }
        let painter = ui.painter_at(rect);
        // Set into the card as the launcher's wells are: shadowed along its top, outlined; a
        // pool of warm light under each unit, as a showroom's (the thumbnails' too).
        painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(WELL[0], WELL[1], WELL[2]));
        let stands = if self.textures.is_empty() { 2 } else { self.textures.len() };
        for i in 0..stands {
            let center = egui::pos2(rect.left() + width * (i as f32 + 0.5) / stands as f32, rect.top() + side * 0.47);
            let lit = egui::Color32::from_rgb(SPOT[0] as u8, SPOT[1] as u8, SPOT[2] as u8);
            widgets::soft_glow(&painter, center, 0.0, (side * 0.62).min(width / stands as f32 * 0.7), lit.gamma_multiply(0.9));
        }
        painter.add(ta_ui::metal::vgrad(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 10.0)), egui::Color32::from_black_alpha(120), egui::Color32::TRANSPARENT));
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, ta_ui::metal::STEEL_EDGE), egui::StrokeKind::Inside);
        for i in 0..self.textures.len() {
            let center = egui::pos2(rect.left() + width * (i as f32 + 0.5) / self.textures.len() as f32, rect.top() + side / 2.0);
            let tile = egui::Rect::from_center_size(center, egui::vec2(side, side));
            if let Some(texture) = self.textures.get(i) {
                painter.image(texture.id(), tile, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            if let Some(name) = self.names.get(i) {
                let mut label = egui::text::LayoutJob::simple(name.clone(), body(12.0), TEXT, side - 8.0);
                label.wrap.max_rows = 2;
                label.wrap.break_anywhere = true;
                let galley = painter.layout_job(label);
                painter.galley(egui::pos2(tile.center().x - galley.size().x / 2.0, tile.bottom() + 3.0), galley, TEXT);
            }
        }
        if self.textures.is_empty() {
            let message = self.error.as_deref().unwrap_or_else(|| t("Loading unit preview…"));
            let galley = painter.layout(message.to_string(), body(14.0), if self.error.is_some() { STEEL } else { CREAM }, (width - 24.0).max(1.0));
            painter.galley(rect.center() - galley.size() / 2.0, galley, CREAM);
        }
        // Named image: decorative rotation never floods the accessibility live region.
        let name = tf("Live unit preview: {skin}", &[("skin", &self.key.as_ref().map_or("", |key| key.skin.as_str()))]);
        let response = ui.interact(rect, ui.id().with("unit_skin_preview"), egui::Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, name.clone()));
        // Dragged across, the units turn with the pointer (and stay where they are left); a
        // double click brings them back to the front.
        if response.dragged() {
            self.yaw = (self.yaw + response.drag_delta().x * 0.012).rem_euclid(std::f32::consts::TAU);
            self.paused = true;
            self.redraw = true;
        }
        if response.double_clicked() { self.yaw = 0.0; self.redraw = true; }
        let response = if response.dragged() { response.on_hover_cursor(egui::CursorIcon::Grabbing) } else { response.on_hover_cursor(egui::CursorIcon::Grab) };
        response.hover_tip(t("Actual unit models and skin materials: drag to turn them"));
        if !self.textures.is_empty() && let Some(error) = &self.error {
            ui.add(egui::Label::new(egui::RichText::new(error).small().color(STEEL)).wrap());
        }
        // One row of controls: turning on or off, the view angle's slider and its figure, a link
        // to reset it; in a narrow panel the slider takes a row of its own.
        ui.add_space(4.0);
        let figure = widgets::value_box_width(ui, &["360°"]);
        let reset = widgets::galley(ui, egui::RichText::new(t("Reset view")).font(strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(1.0))).size().x + 8.0;
        let words = widgets::galley(ui, egui::RichText::new(t("View angle"))).size().x;
        let one_row = ui.available_width() - (26.0 + words + reset + 3.0 * 10.0) >= figure + 140.0;
        let mut degrees = self.yaw.to_degrees().rem_euclid(360.0);
        let mut angle = |ui: &mut Ui, room: f32| ui.allocate_ui(egui::vec2(room.max(1.0), widgets::CONTROL_HEIGHT), |ui| {
            widgets::value_slider(ui, egui::Slider::new(&mut degrees, 0.0..=360.0).suffix("°").fixed_decimals(0), figure)
        }).inner;
        let mut slider = None;
        let label = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (glyph, hint) = if self.paused { (widgets::Glyph::Play, t("Rotate preview")) } else { (widgets::Glyph::Stop, t("Pause preview")) };
            if widgets::icon_button(ui, glyph, true).hover_tip(hint).clicked() { self.paused = !self.paused; }
            let label = ui.label(egui::RichText::new(t("View angle")).color(STEEL)).id;
            if one_row {
                let room = (ui.available_width() - reset - 10.0).min(360.0);
                slider = Some(angle(ui, room));
            }
            if widgets::text_link(ui, "Reset view").clicked() { self.yaw = 0.0; self.redraw = true; }
            label
        }).inner;
        let slider = match slider { Some(slider) => slider, None => { let room = ui.available_width(); angle(ui, room) } }.labelled_by(label);
        if slider.changed() { self.yaw = degrees.to_radians(); self.redraw = true; self.paused = true; }
        if self.error.is_some() && widgets::text_link(ui, "Retry preview").clicked() { self.retry(); }
        if visible && !still && self.error.is_none() { ta_ui::motion::next_frame(ui.ctx(), Duration::from_millis(67)); }
    }

    /// The thumbnail of the skin `id` (`revision`: the library's, bumped when skins change),
    /// asked for once and made on the worker; none until it is made. Also whether it could not
    /// be.
    pub fn thumbnail(&mut self, ctx: &egui::Context, id: &str, revision: u64, root: PathBuf, reference: &Reference, cache: PathBuf) -> (Option<TextureHandle>, bool) {
        let key = format!("{id}#{revision}");
        if let Some(texture) = self.thumbs.get(&key) { return (Some(texture.clone()), false); }
        if self.failed.contains(&key) { return (None, true); }
        if reference.game_dir.is_none() { return (None, true); }
        if self.asked.insert(key.clone()) {
            if !self.start() { self.failed.insert(key); return (None, true); }
            let shared = self.shared.as_ref().expect("started above");
            *shared.ctx.lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx.clone());
            shared.thumbs.lock().unwrap_or_else(|e| e.into_inner()).push_back(Thumb { key, skin: id.to_string(), revision, root, reference: reference.clone(), cache });
            let _pending = shared.request.lock().unwrap_or_else(|e| e.into_inner());
            shared.ready.notify_one();
        }
        (None, false)
    }

    /// Takes the thumbnails made since, and forgets those of an older library.
    pub fn poll_thumbnails(&mut self, ctx: &egui::Context, revision: u64) {
        self.take_thumbnails(ctx);
        let current = format!("#{revision}");
        if self.thumbs.keys().chain(&self.failed).chain(&self.asked).any(|key| !key.ends_with(&current)) {
            self.thumbs.retain(|key, _| key.ends_with(&current));
            self.failed.retain(|key| key.ends_with(&current));
            self.asked.retain(|key| key.ends_with(&current));
            if let Some(shared) = &self.shared {
                shared.thumbs.lock().unwrap_or_else(|e| e.into_inner()).retain(|thumb| thumb.key.ends_with(&current));
            }
        }
    }

    /// The thumbnails made since (wherever the preview shows: the editor's too).
    fn take_thumbnails(&mut self, ctx: &egui::Context) {
        let made = self.shared.as_ref().map(|shared| std::mem::take(&mut *shared.made.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for (key, picture) in made {
            match picture {
                Ok(image) => { self.thumbs.insert(key.clone(), ctx.load_texture(format!("skin thumbnail {key}"), image, TextureOptions::LINEAR)); }
                Err(error) => { log::warn!("skin thumbnail {key}: {error}"); self.failed.insert(key); }
            }
        }
    }

    fn retry(&mut self) {
        self.error = None;
        // An old image can remain after a later GPU/readback failure. A paused or reduced-
        // motion preview still needs a fresh request when the player explicitly retries.
        self.redraw = true;
    }

    /// The worker, started when first wanted. False when it could not start.
    fn start(&mut self) -> bool {
        if self.shared.is_some() { return true; }
        let shared = Arc::new(Shared::default());
        let worker_shared = shared.clone();
        match std::thread::Builder::new().name("skin preview".into()).spawn(move || worker(worker_shared)) {
            Ok(worker) => { self.shared = Some(shared); self.worker = Some(worker); true }
            Err(error) => { self.error = Some(tf("Could not start the unit preview: {error}", &[("error", &error)])); false }
        }
    }

    fn submit(&mut self, request: Request) {
        if !self.start() { return; }
        let shared = self.shared.as_ref().expect("started above");
        *shared.request.lock().unwrap_or_else(|e| e.into_inner()) = Some(request);
        self.waiting = true;
        shared.ready.notify_one();
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.take_thumbnails(ctx);
        let reply = self.shared.as_ref().and_then(|s| s.reply.lock().unwrap_or_else(|e| e.into_inner()).take());
        let Some(reply) = reply else { return };
        self.waiting = false;
        if self.key.as_ref() != Some(&reply.key) { return; }
        match reply.pictures {
            Ok(pictures) => {
                self.names = pictures.names;
                self.textures.truncate(pictures.images.len());
                for (i, picture) in pictures.images.into_iter().enumerate() {
                    let image = ColorImage::from_rgba_premultiplied([picture.size as usize; 2], &picture.rgba);
                    if let Some(texture) = self.textures.get_mut(i) { texture.set(image, TextureOptions::LINEAR); }
                    else { self.textures.push(ctx.load_texture(format!("unit skin preview {i}"), image, TextureOptions::LINEAR)); }
                }
                if self.slots_from.as_ref().is_none_or(|from| !Arc::ptr_eq(from, &pictures.slots)) {
                    self.slots = pictures.slots.iter().map(|(unit, display, textures)| Slot {
                        unit: unit.clone(), display: display.clone(),
                        textures: textures.iter().map(|(name, image)| (name.clone(), ctx.load_texture(format!("unit texture {name}"), image.clone(), TextureOptions::NEAREST))).collect(),
                    }).collect();
                    self.slots_from = Some(pictures.slots);
                }
                self.units = pictures.units;
            }
            Err(error) => { log::warn!("skin preview: {error}"); self.error = Some(tf("Preview unavailable: {error}", &[("error", &error)])); }
        }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            // Hold the condition mutex while changing its predicate: otherwise shutdown
            // can notify between the worker's predicate check and its wait.
            let _pending = shared.request.lock().unwrap_or_else(|e| e.into_inner());
            shared.stop.store(true, Ordering::Relaxed);
            shared.ready.notify_one();
        }
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn preview_side(width: f32, height: f32) -> f32 {
    ((width - 8.0) / 2.0).min(height - 34.0).clamp(64.0, 256.0)
}

fn advance_yaw(yaw: f32, previous: Option<f64>, now: f64, still: bool) -> f32 {
    if still { yaw } else { (yaw + previous.map_or(0.0, |before| (now - before).clamp(0.0, 0.1) as f32 * 0.24)).rem_euclid(std::f32::consts::TAU) }
}

fn frame_due(has_image: bool, redraw: bool, still: bool, since: f64) -> bool {
    !has_image || redraw || (!still && since >= 1.0 / 15.0)
}

fn source_key(reference: &Reference) -> String {
    let mods: Vec<_> = reference.available.iter().filter(|m| reference.enabled.contains(&m.manifest.id))
        .map(|m| (&m.path, &m.manifest.id, &m.manifest.version, &m.manifest.rename)).collect();
    format!("{:?}|{:?}|{mods:?}", reference.game_dir, reference.enabled)
}

struct Source {
    vfs: ta_vfs::Vfs,
    palette: ta_formats::image::Palette,
    units: Vec<ta_data::UnitDef>,
    gafs: Vec<ta_formats::gaf::Gaf>,
    /// Each texture's place among the GAFs, by lower-case name: its file, its entry.
    textures: HashMap<String, (usize, usize)>,
    choices: Arc<Vec<UnitChoice>>,
}

impl Source {
    /// A texture's frame count (none for one the install lacks).
    fn frames(&self, name: &str) -> Option<u32> {
        let &(gaf, entry) = self.textures.get(&name.to_ascii_lowercase())?;
        Some(self.gafs[gaf].entries[entry].frames.len() as u32)
    }

    /// A small picture of a texture's paint (its first frame).
    fn picture(&self, name: &str) -> ColorImage {
        const SIDE: usize = 24;
        let mut picture = ColorImage::filled([SIDE, SIDE], egui::Color32::from_gray(40));
        let Some(&(gaf, entry)) = self.textures.get(&name.to_ascii_lowercase()) else { return picture };
        let Some(frame) = self.gafs[gaf].entries[entry].frames.first() else { return picture };
        let image = &frame.image;
        if image.width == 0 || image.height == 0 { return picture; }
        for y in 0..SIDE {
            for x in 0..SIDE {
                let (sx, sy) = (x * image.width as usize / SIDE, y * image.height as usize / SIDE);
                let [r, g, b] = self.palette.rgb(image.pixels[sy * image.width as usize + sx]);
                picture.pixels[y * SIDE + x] = egui::Color32::from_rgb(r, g, b);
            }
        }
        picture
    }
}

fn load_source(reference: &Reference, shared: &Shared) -> anyhow::Result<Source> {
    use anyhow::{Context, ensure};
    let game_dir = reference.game_dir.as_deref().context("no game installation selected")?;
    let mounted = ta_vfs::mods::mount_enabled(Some(game_dir), ta_vfs::builtin_dir().as_deref(), &reference.enabled, &reference.available);
    mounted.install?;
    mounted.mods?;
    let vfs = mounted.vfs;
    let palette = ta_formats::image::Palette::parse(&vfs.read("palettes/palette.pal")?)?;
    let mut units = Vec::new();
    for path in vfs.list("units", Some("fbi"), false) {
        ensure!(!shared.stop.load(Ordering::Relaxed), "preview closed");
        if let Some(data) = vfs.read_opt(&path) {
            let doc = ta_formats::tdf::parse_bytes(&data);
            if let Some(unit) = doc.section("UNITINFO").and_then(ta_data::UnitDef::from_tdf) { units.push(unit); }
        }
    }
    let mut gafs = Vec::new();
    let mut budget = 256usize << 20;
    for path in vfs.list("textures", Some("gaf"), false) {
        ensure!(!shared.stop.load(Ordering::Relaxed), "preview closed");
        if let Some(data) = vfs.read_opt(&path) && let Ok(gaf) = ta_formats::gaf::Gaf::parse(&data) {
            let size: usize = gaf.entries.iter().flat_map(|e| &e.frames).map(|f| f.image.pixels.len() * 2).sum();
            budget = budget.checked_sub(size).context("installed textures exceed the preview memory limit")?;
            gafs.push(gaf);
        }
    }
    let mut textures = HashMap::new();
    for (g, gaf) in gafs.iter().enumerate() {
        for (e, entry) in gaf.entries.iter().enumerate() {
            textures.entry(entry.name.to_ascii_lowercase()).or_insert((g, e));
        }
    }
    let mut choices: Vec<UnitChoice> = units.iter()
        .filter(|u| (u.side.eq_ignore_ascii_case("ARM") || u.side.eq_ignore_ascii_case("CORE")) && vfs.exists(&format!("objects3d/{}.3do", u.object)))
        .map(|u| UnitChoice { name: u.name.clone(), display: u.display_name.clone(), arm: u.side.eq_ignore_ascii_case("ARM") }).collect();
    choices.sort_by_key(|choice| (choice.display.to_lowercase(), choice.name.clone()));
    Ok(Source { vfs, palette, units, gafs, textures, choices: Arc::new(choices) })
}

/// The units a preview shows, one a side: those `wanted` (by name, when there are), else a
/// pack's own unit overrides first, then the usual examples. With their names and slots.
fn subjects(source: &Source, pack: &ta_skins::Pack, wanted: &[String; 2]) -> anyhow::Result<(Vec<ta_render::preview::PreviewModel>, Vec<String>, SlotImages)> {
    use anyhow::ensure;
    let mut models = Vec::new();
    let mut names = Vec::new();
    let mut slots = Vec::new();
    for ((side, preferred), wanted) in [("ARM", "ARMSTUMP"), ("CORE", "CORRAID")].into_iter().zip(wanted) {
        let mut candidates: Vec<_> = source.units.iter().filter(|u| u.side.eq_ignore_ascii_case(side)).collect();
        // A pack's unit overrides take precedence over the usual examples, so custom paint
        // is visible even when its author paints only one particular bot or vehicle.
        candidates.sort_by_key(|u| (!u.name.eq_ignore_ascii_case(wanted), !pack.manifest.units.contains_key(&u.name.to_ascii_lowercase()), !u.name.eq_ignore_ascii_case(preferred), &u.name));
        for unit in candidates {
            let path = format!("objects3d/{}.3do", unit.object);
            let Some(bytes) = source.vfs.read_opt(&path) else { continue };
            let Ok(model) = ta_formats::model3do::Model3do::parse(&bytes) else { continue };
            if model.pieces.iter().all(|p| p.primitives.is_empty()) { continue; }
            names.push(format!("{} · {}", side, unit.display_name));
            let textures = ta_render::model::paintable_textures(&model, |name| source.frames(name));
            slots.push((unit.name.clone(), unit.display_name.clone(), textures.into_iter().map(|name| { let picture = source.picture(&name); (name, picture) }).collect()));
            // (Its own script, else its model's, as the game runs it.)
            let script = unit.script_paths().iter().find_map(|p| source.vfs.read_opt(p)).and_then(|bytes| ta_formats::cob::CobScript::parse(&bytes).ok());
            let pieces = pose::standing(&model, script, unit.activate_when_built);
            models.push(ta_render::preview::PreviewModel { unit: unit.name.clone(), object: unit.object.clone(), model, pieces: Some(pieces) });
            break;
        }
    }
    ensure!(!models.is_empty(), "no ARM or CORE example models are available in the enabled game data");
    Ok((models, names, slots))
}

/// What the worker's graphics device holds.
#[derive(PartialEq)]
enum Loaded {
    Saved(Key),
    Editing(Key),
    /// Reborn's models made for liveries (`set_skin_for_editing`), for liveries' thumbnails.
    Liveries(String),
    /// A skin with images of its own, for its thumbnail.
    Thumb(String),
}

#[derive(Default)]
struct State {
    source: Option<(String, Source)>,
    studio: Option<ta_render::preview::UnitStudio>,
    loaded: Option<Loaded>,
    names: Vec<String>,
    slots: Arc<SlotImages>,
}

impl State {
    fn source(&mut self, key: &str, reference: &Reference, shared: &Shared) -> anyhow::Result<()> {
        if self.source.as_ref().is_none_or(|(loaded, _)| loaded != key) {
            self.source = Some((key.to_string(), load_source(reference, shared)?));
            self.loaded = None;
        }
        Ok(())
    }

    fn studio(&mut self) -> anyhow::Result<()> {
        if self.studio.is_none() { self.studio = Some(ta_render::preview::UnitStudio::new()?); }
        Ok(())
    }

    fn paint(&mut self, request: &Request, shared: &Shared) -> anyhow::Result<Pictures> {
        self.source(&request.key.data(), &request.reference, shared)?;
        anyhow::ensure!(!shared.stop.load(Ordering::Relaxed), "preview closed");
        self.studio()?;
        let (Some((_, source)), Some(studio)) = (&self.source, &mut self.studio) else { unreachable!("made above") };
        match &request.paint {
            Painting::Saved => if self.loaded.as_ref() != Some(&Loaded::Saved(request.key.clone())) {
                let pack = ta_skins::load(&request.key.skin, &request.key.root)?;
                let (models, names, slots) = subjects(source, &pack, &request.key.units)?;
                studio.set_skin(&source.gafs, &source.palette, &pack, &models)?;
                (self.names, self.slots, self.loaded) = (names, Arc::new(slots), Some(Loaded::Saved(request.key.clone())));
            },
            Painting::Editing { pack, livery } => {
                if self.loaded.as_ref() != Some(&Loaded::Editing(request.key.clone())) {
                    let (models, names, slots) = subjects(source, pack, &request.key.units)?;
                    studio.set_skin_for_editing(&source.gafs, &source.palette, pack, &models)?;
                    (self.names, self.slots, self.loaded) = (names, Arc::new(slots), Some(Loaded::Editing(request.key.clone())));
                }
                studio.set_livery(livery.as_ref())?;
            }
        }
        anyhow::ensure!(!shared.stop.load(Ordering::Relaxed), "preview closed");
        Ok(Pictures { images: studio.render(request.yaw, 320)?, names: self.names.clone(), slots: self.slots.clone(), units: source.choices.clone() })
    }

    /// A skin's thumbnail: from the disk cache, or made (and kept there).
    fn thumbnail(&mut self, thumb: &Thumb, shared: &Shared) -> anyhow::Result<ColorImage> {
        let pack = ta_skins::load(&thumb.skin, &thumb.root)?;
        let source_key = source_key(&thumb.reference);
        let file = crate::library::cache_file(&thumb.cache, &thumb_key(&source_key, &pack));
        if let Some(cached) = crate::library::read_cached(&file) { return Ok(cached); }
        // (The same game data as the previews': moving between them reads it once.)
        self.source(&data_key(&source_key, thumb.revision), &thumb.reference, shared)?;
        self.studio()?;
        let (Some((_, source)), Some(studio)) = (&self.source, &mut self.studio) else { unreachable!("made above") };
        let examples = [String::new(), String::new()];
        if pack.images.is_empty() {
            if self.loaded.as_ref() != Some(&Loaded::Liveries(source_key.clone())) {
                let reborn = ta_skins::load("reborn", &thumb.root)?;
                let (models, _, _) = subjects(source, &reborn, &examples)?;
                studio.set_skin_for_editing(&source.gafs, &source.palette, &reborn, &models)?;
                self.loaded = Some(Loaded::Liveries(source_key));
            }
            studio.set_livery(pack.manifest.livery.as_ref())?;
        } else if self.loaded.as_ref() != Some(&Loaded::Thumb(thumb.skin.clone())) {
            let (models, _, _) = subjects(source, &pack, &examples)?;
            studio.set_skin(&source.gafs, &source.palette, &pack, &models)?;
            self.loaded = Some(Loaded::Thumb(thumb.skin.clone()));
        }
        let picture = compose(&studio.render(THUMB_YAW, THUMB_SIDE)?);
        crate::library::write_cached(&file, &picture);
        Ok(picture)
    }
}

/// What a thumbnail is made of: the game's data, and the skin's manifest and images.
fn thumb_key(source: &str, pack: &ta_skins::Pack) -> String {
    use sha2::{Digest, Sha256};
    let mut images = Sha256::new();
    for (name, image) in &pack.images {
        images.update(name.as_bytes());
        images.update(image.width().to_le_bytes());
        images.update(image.as_raw());
    }
    format!("{source}|skin thumbnail 2|{}|{:x}", toml::to_string(&pack.manifest).unwrap_or_default(), images.finalize())
}

/// Under each unit of a thumbnail: a pool of warm light on the well, brightest just above its
/// middle, gone to the well's own colour before the corners.
const SPOT: [f32; 3] = [62.0, 58.0, 49.0];

/// The light under a thumbnail's unit at (`x`, `y`) of its `keep` × `side` cut: the well's
/// colour, lit towards [`SPOT`] in a soft pool round the unit.
fn backdrop(x: usize, y: usize, keep: usize, side: usize) -> [u8; 3] {
    let (dx, dy) = (x as f32 + 0.5 - keep as f32 * 0.5, y as f32 + 0.5 - side as f32 * 0.46);
    let s = ((dx * dx + dy * dy * 1.3).sqrt() / (keep as f32 * 0.62)).min(1.0);
    let light = (1.0 - s * s).powi(2);
    std::array::from_fn(|k| (WELL[k] as f32 + (SPOT[k] - WELL[k] as f32) * light).round() as u8)
}

/// The two units' pictures side by side, each cut to its middle, over the card's well lit
/// under each.
fn compose(images: &[ta_render::preview::PreviewImage]) -> ColorImage {
    let (side, keep) = (THUMB_SIDE as usize, THUMB_KEEP as usize);
    let width = keep * images.len().max(1);
    let mut picture = ColorImage::filled([width, side], egui::Color32::from_rgb(WELL[0], WELL[1], WELL[2]));
    for (i, image) in images.iter().enumerate() {
        let size = image.size as usize;
        if size != side { continue; }
        let left = (side - keep) / 2;
        for y in 0..side {
            for x in 0..keep {
                let at = (y * size + left + x) * 4;
                let [r, g, b, a] = [image.rgba[at], image.rgba[at + 1], image.rgba[at + 2], image.rgba[at + 3]];
                let well = backdrop(x, y, keep, side);
                // (Premultiplied: the light shows through as much as the unit lets it.)
                let under = |c: u8, w: u8| (u16::from(c) + u16::from(w) * (255 - u16::from(a)) / 255).min(255) as u8;
                picture.pixels[y * width + i * keep + x] = egui::Color32::from_rgb(under(r, well[0]), under(g, well[1]), under(b, well[2]));
            }
        }
    }
    picture
}

fn worker(shared: Arc<Shared>) {
    let mut state = State::default();
    loop {
        let mut pending = shared.request.lock().unwrap_or_else(|e| e.into_inner());
        while pending.is_none() && shared.thumbs.lock().unwrap_or_else(|e| e.into_inner()).is_empty() && !shared.stop.load(Ordering::Relaxed) {
            let (guard, timeout) = shared.ready.wait_timeout(pending, Duration::from_secs(20)).unwrap_or_else(|e| e.into_inner());
            pending = guard;
            if timeout.timed_out() && pending.is_none() {
                // Resource teardown can wait for the driver. Never hold the UI mailbox
                // while releasing GPU objects or dropping the parsed installation.
                drop(pending);
                state = State::default();
                pending = shared.request.lock().unwrap_or_else(|e| e.into_inner());
            }
        }
        if shared.stop.load(Ordering::Relaxed) { return; }
        let request = pending.take();
        drop(pending);
        if let Some(request) = request {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.paint(&request, &shared)))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the graphics driver could not render this preview")));
            if result.is_err() { state.studio = None; state.loaded = None; }
            *shared.reply.lock().unwrap_or_else(|e| e.into_inner()) = Some(Reply { key: request.key, pictures: result.map_err(|e| e.to_string()) });
            request.ctx.request_repaint();
            continue;
        }
        // No preview waiting: every thumbnail asked for, in one go (the liveries' on one set of
        // models), before the next preview makes its own again.
        let stopped = take_each(&shared.thumbs, |thumb| {
            if shared.stop.load(Ordering::Relaxed) { return std::ops::ControlFlow::Break(()); }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.thumbnail(&thumb, &shared)))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the graphics driver could not render this thumbnail")));
            if result.is_err() { state.studio = None; state.loaded = None; }
            shared.made.lock().unwrap_or_else(|e| e.into_inner()).push((thumb.key, result.map_err(|e| e.to_string())));
            if let Some(ctx) = shared.ctx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() { ctx.request_repaint(); }
            std::ops::ControlFlow::Continue(())
        });
        if stopped.is_break() { return; }
    }
}

/// Hands `queue`'s items to `each` one at a time, the queue unlocked while each is handled: the
/// interface adds to it as cards come into view, and a thumbnail takes a while to make (a
/// `while let` on the lock would hold it through the loop's body, the interface waiting).
fn take_each<T>(queue: &Mutex<VecDeque<T>>, mut each: impl FnMut(T) -> std::ops::ControlFlow<()>) -> std::ops::ControlFlow<()> {
    loop {
        let Some(item) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front() else { return std::ops::ControlFlow::Continue(()) };
        each(item)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worker makes each thumbnail with the queue unlocked: the interface asking for more
    /// as cards come into view never waits for one being made.
    #[test]
    fn thumbnails_are_made_with_their_queue_unlocked() {
        let queue = Mutex::new(VecDeque::from([1, 2, 3]));
        let mut made = Vec::new();
        let flow = take_each(&queue, |item| {
            let mut waiting = queue.try_lock().expect("the queue is free while a thumbnail is made");
            if item == 1 { waiting.push_back(4); }
            drop(waiting);
            made.push(item);
            std::ops::ControlFlow::Continue(())
        });
        assert!(flow.is_continue());
        assert_eq!(made, [1, 2, 3, 4], "those asked for meanwhile are made too");
        queue.lock().unwrap().extend([5, 6]);
        assert!(take_each(&queue, |_| std::ops::ControlFlow::Break(())).is_break(), "a stop stops at once");
        assert_eq!(queue.lock().unwrap().len(), 1, "the rest stay queued");
    }

    #[test]
    fn a_hidden_pending_preview_does_not_block_readiness_after_the_library_reflows() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut preview = Preview::default();
        preview.shared = Some(Arc::new(Shared::default()));
        let reference = Reference { game_dir: Some("game".into()), enabled: Vec::new(), available: Vec::new() };
        preview.begin_frame();
        ctx.run_ui(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 400.0))), ..Default::default()
        }, |ui| preview.show(ui, reference.clone(), PathBuf::new(), Paint::Saved("reborn".into()), 0, &Default::default(), 200.0)).drop_without_applying_deltas();
        assert!(!preview.ready(), "the visible preview still needs its first rendered frame");
        // A narrow library renders cards only, so show is not called next frame.
        preview.begin_frame();
        assert!(preview.ready(), "a preview absent from this layout must not stall a capture or film");
    }
    #[test]
    fn retry_queues_a_fresh_frame_even_with_a_paused_cached_image() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let reference = Reference { game_dir: Some("game".into()), enabled: Vec::new(), available: Vec::new() };
        let key = Key { source: source_key(&reference), root: PathBuf::new(), skin: "reborn".into(), revision: 0, textures: 0, units: Default::default() };
        let shared = Arc::new(Shared::default());
        let texture = ctx.load_texture("cached preview", ColorImage::filled([1, 1], egui::Color32::WHITE), TextureOptions::LINEAR);
        let mut preview = Preview::default();
        preview.shared = Some(shared.clone());
        preview.key = Some(key);
        preview.textures = vec![texture];
        preview.paused = true;
        preview.error = Some("temporary readback failure".into());
        preview.retry();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 400.0))), ..Default::default() };
        ctx.run_ui(input, |ui| preview.show(ui, reference.clone(), PathBuf::new(), Paint::Saved("reborn".into()), 0, &Default::default(), 200.0)).drop_without_applying_deltas();
        assert!(shared.request.lock().unwrap().is_some(), "retry must reach the renderer even when the old image is still displayed");
        assert!(preview.waiting);
    }

    #[test]
    fn a_new_livery_is_a_new_picture_but_the_editors_models_stay() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let reference = Reference { game_dir: Some("game".into()), enabled: Vec::new(), available: Vec::new() };
        let shared = Arc::new(Shared::default());
        let mut preview = Preview::default();
        preview.shared = Some(shared.clone());
        preview.paused = true;
        let pack = Arc::new(ta_skins::load("builtin:desert", std::path::Path::new("unused")).unwrap());
        let show = |preview: &mut Preview, livery: Option<ta_skins::Livery>, revision: u64, serial: u64| {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 400.0))), ..Default::default() };
            ctx.run_ui(input, |ui| {
                let paint = Paint::Editing { pack: pack.clone(), serial, revision, livery: livery.clone() };
                preview.show(ui, reference.clone(), PathBuf::new(), paint, 0, &Default::default(), 200.0);
            }).drop_without_applying_deltas();
            shared.request.lock().unwrap().take().map(|request| request.key)
        };
        let first = show(&mut preview, pack.manifest.livery.clone(), 0, 1).expect("the first picture");
        preview.waiting = false;
        preview.textures = vec![ctx.load_texture("cached", ColorImage::filled([1, 1], egui::Color32::WHITE), TextureOptions::LINEAR)];
        assert!(show(&mut preview, pack.manifest.livery.clone(), 0, 1).is_none(), "paused, nothing new to paint");
        let tiger = ta_skins::builtin_livery("builtin:tiger");
        let second = show(&mut preview, tiger, 0, 1).expect("a new livery is a new picture");
        assert_eq!(first, second, "with the same models");
        preview.waiting = false;
        let third = show(&mut preview, None, 1, 1).expect("new textures");
        assert_ne!(first, third, "new textures make the models again");
        assert_eq!(first.data(), third.data(), "on the same game data");
        assert_eq!(preview.textures.len(), 1, "the editor's last picture stays until the new one comes");
        // A different editor can have the same units and texture revision. It
        // must still rebuild models, even with rotation paused and no new livery.
        preview.waiting = false;
        let fourth = show(&mut preview, None, 1, 2).expect("another skin opened");
        assert_ne!(third, fourth);
        assert!(preview.textures.is_empty(), "the previous skin's paint is not shown as the new one");
    }

    #[test]
    fn preview_size_follows_both_viewport_dimensions() {
        assert_eq!(preview_side(800.0, 290.0), 256.0);
        assert_eq!(preview_side(800.0, 138.0), 104.0);
        assert_eq!(preview_side(300.0, 290.0), 146.0);
        assert!(preview_side(800.0, 180.0) < preview_side(800.0, 260.0));
    }
    #[test]
    fn closing_an_idle_preview_wakes_the_worker_without_waiting_for_eviction() {
        for _ in 0..8 {
            let shared = Arc::new(Shared::default());
            let runner = shared.clone();
            let worker = std::thread::spawn(move || super::worker(runner));
            let mut preview = Preview::default();
            preview.shared = Some(shared);
            preview.worker = Some(worker);
            let started = std::time::Instant::now();
            drop(preview);
            assert!(started.elapsed() < Duration::from_secs(2));
        }
    }
    #[test]
    fn reduced_motion_keeps_one_frame_and_manual_views_still_work() {
        assert!(frame_due(false, false, true, 0.0), "even a still preview needs its first image");
        assert!(!frame_due(true, false, true, 600.0));
        assert!(frame_due(true, true, true, 0.0), "manual view controls are available with reduced motion");
        assert!(!frame_due(true, false, false, 0.01));
        assert!(frame_due(true, false, false, 0.07));
        assert_eq!(advance_yaw(1.0, Some(0.0), 30.0, true), 1.0);
        assert!((advance_yaw(1.0, Some(0.0), 30.0, false) - 1.0) < 0.025, "refocusing never jumps past unseen rotation");
    }
    #[test]
    fn preview_source_key_includes_install_mod_order_and_mod_path() {
        let mut r = Reference { game_dir: Some("one".into()), enabled: vec!["a".into(), "b".into()], available: Vec::new() };
        let first = source_key(&r);
        r.game_dir = Some("two".into()); assert_ne!(first, source_key(&r));
        let second = source_key(&r); r.enabled.reverse(); assert_ne!(second, source_key(&r));
    }
    #[test]
    fn new_selection_replaces_pending_preview_instead_of_growing_a_queue() {
        let shared = Shared::default();
        for i in 0..1000 {
            *shared.request.lock().unwrap() = Some(Request { key: Key { source: "same".into(), root: PathBuf::new(), skin: i.to_string(), revision: 0, textures: 0, units: Default::default() },
                reference: Reference { game_dir: None, enabled: Vec::new(), available: Vec::new() }, yaw: 0.0, ctx: egui::Context::default(), paint: Painting::Saved });
        }
        assert_eq!(shared.request.lock().unwrap().as_ref().unwrap().key.skin, "999");
    }
    #[test]
    fn a_thumbnail_is_two_units_cut_to_their_middles_over_the_well() {
        let side = THUMB_SIDE as usize;
        let mut unit = vec![0u8; side * side * 4];
        // (A white pixel in the middle of each picture, the rest transparent.)
        let middle = (side / 2 * side + side / 2) * 4;
        unit[middle..middle + 4].copy_from_slice(&[255, 255, 255, 255]);
        let images: Vec<_> = (0..2).map(|_| ta_render::preview::PreviewImage { size: THUMB_SIDE, rgba: unit.clone() }).collect();
        let picture = compose(&images);
        assert_eq!(picture.size, [THUMB_KEEP as usize * 2, side]);
        let well = egui::Color32::from_rgb(WELL[0], WELL[1], WELL[2]);
        assert_eq!(picture.pixels[0], well, "the corners are the well's own colour");
        let keep = THUMB_KEEP as usize;
        for i in 0..2 { assert_eq!(picture.pixels[side / 2 * keep * 2 + i * keep + keep / 2], egui::Color32::WHITE); }
        // (Round each unit, where it is transparent, a pool of warm light.)
        let near = picture.pixels[side / 2 * keep * 2 + keep / 2 + 12];
        assert!(near.r() > WELL[0] + 30 && near.r() > near.b(), "lit warm beside the unit: {near:?}");
        let pack = ta_skins::load("builtin:navy", std::path::Path::new("unused")).unwrap();
        let other = ta_skins::load("builtin:urban", std::path::Path::new("unused")).unwrap();
        assert_ne!(thumb_key("game", &pack), thumb_key("game", &other));
        assert_ne!(thumb_key("game", &pack), thumb_key("modded", &pack), "another game's data, another picture");
        assert_eq!(thumb_key("game", &pack), thumb_key("game", &pack));
    }
    #[test]
    #[ignore = "requires a GPU and the locally installed original game"]
    fn installed_preview_framing_keeps_rotating_units_inside_the_image() {
        let game_dir = std::env::var_os("TA_REBORN_GAME_DIR").map(PathBuf::from)
            .unwrap_or_else(|| "C:/GOG Games/Total Annihilation".into());
        let source = load_source(&Reference { game_dir: Some(game_dir), enabled: Vec::new(), available: Vec::new() }, &Shared::default()).unwrap();
        let mut studio = ta_render::preview::UnitStudio::new().unwrap();
        for id in ta_skins::builtin_ids() {
            let pack = ta_skins::load(id, std::path::Path::new(".")).unwrap();
            let (models, _, slots) = subjects(&source, &pack, &Default::default()).unwrap();
            assert!(slots.iter().all(|(_, _, textures)| !textures.is_empty()), "the examples have armour to repaint");
            studio.set_skin(&source.gafs, &source.palette, &pack, &models).unwrap();
            for yaw in [0.0, 0.65, std::f32::consts::FRAC_PI_2, std::f32::consts::PI, 3.0 * std::f32::consts::FRAC_PI_2, 5.5] {
                for (unit, picture) in studio.render(yaw, 320).unwrap().iter().enumerate() {
                    let size = picture.size as usize;
                    let visible = picture.rgba.chunks_exact(4).filter(|p| p[3] > 128).count();
                    assert!(visible > 1000, "{id} model {unit} at {yaw} must remain visible");
                    for y in 0..size { for x in 0..size {
                        if x < 2 || x >= size - 2 || y < 2 || y >= size - 2 {
                            assert!(picture.rgba[(y * size + x) * 4 + 3] <= 8,
                                "{id} model {unit} at {yaw} clips the image edge at {x},{y}");
                        }
                    }}
                }
            }
        }
        assert!(source.choices.iter().any(|choice| choice.name.eq_ignore_ascii_case("ARMCOM") && choice.arm));
        assert!(source.choices.iter().any(|choice| choice.name.eq_ignore_ascii_case("CORKROG") && !choice.arm));
    }
}
