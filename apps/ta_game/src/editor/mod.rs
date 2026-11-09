//! The map editor: makes new maps and remixes of TA's, and saves them as mods.
//!
//! [`doc`] holds the map (TA's TNT grids plus the OTA's properties), [`tools`] the edits
//! (height brushes, tile stamps, copy and paste, features), [`history`] undo and redo,
//! [`view`] what the renderer shows (terrain updated in place, feature sprites, overlays),
//! [`save`] the mod written to the user's mods folder, [`ui`] the panels. The app calls
//! [`Editor::update`], [`Editor::ui`] and [`Editor::prepare`] once a frame.

mod checks;
pub mod doc;
mod history;
mod import;
mod import_ui;
mod navigator;
pub mod save;
mod status;
pub mod tools;
mod ui;
mod view;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use ta_formats::tnt::TntMap;
use ta_render::{Camera, Renderer};
use winit::keyboard::KeyCode;

use crate::input::{Input, MouseButtonId};
use crate::setup::Assets;
use doc::{CELL_PX, MAX_STARTS, MapDoc, NewMap, Rect};
use history::{Changes, History, Snapshot};
use tools::{Clip, HeightOp, HeightStroke, TileBlock};
use view::View;

/// How the editor opens.
#[derive(Clone, Debug)]
pub enum Start {
    New(NewMap),
    /// An existing map (file stem); saving writes a new map.
    Open(String),
}

/// What the app should do after an editor frame.
pub enum EditorAction {
    None,
    /// Back to the title menu.
    Exit,
    /// Close the application after confirming unsaved edits.
    Quit,
    /// A map was saved under this name (the menu's cached preview of it is stale).
    Saved(String),
    /// The map was saved under this name: play a battle on it, then come back.
    TestPlay(String),
}

/// What the view shows over the ground.
#[derive(Clone, Copy, Debug)]
pub struct Layers {
    /// The tiles' grid.
    pub grid: bool,
    pub features: bool,
    /// The start positions' markers (always shown with their tool in hand).
    pub starts: bool,
    /// Impassable ground (TA's void cells), shaded red.
    pub blocked: bool,
}

impl Default for Layers {
    fn default() -> Self {
        Layers { grid: false, features: true, starts: true, blocked: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Raise,
    Lower,
    Smooth,
    Flatten,
    Level,
    Tiles,
    Copy,
    Features,
    Starts,
}

impl Tool {
    pub const ALL: [Tool; 9] = [Tool::Raise, Tool::Lower, Tool::Smooth, Tool::Flatten, Tool::Level, Tool::Tiles, Tool::Copy, Tool::Features, Tool::Starts];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Raise => ta_ui::i18n::n("RAISE"),
            Tool::Lower => ta_ui::i18n::n("LOWER"),
            Tool::Smooth => ta_ui::i18n::n("SMOOTH"),
            Tool::Flatten => ta_ui::i18n::n("FLATTEN"),
            Tool::Level => ta_ui::i18n::n("SET LEVEL"),
            Tool::Tiles => ta_ui::i18n::n("PAINT TILES"),
            Tool::Copy => ta_ui::i18n::n("COPY & PASTE"),
            Tool::Features => ta_ui::i18n::n("FEATURES"),
            Tool::Starts => ta_ui::i18n::n("START POSITIONS"),
        }
    }

    /// Its name on its tile in the palette (short).
    pub fn short(self) -> &'static str {
        match self {
            Tool::Raise => ta_ui::i18n::n("RAISE"),
            Tool::Lower => ta_ui::i18n::n("LOWER"),
            Tool::Smooth => ta_ui::i18n::n("SMOOTH"),
            Tool::Flatten => ta_ui::i18n::n("FLATTEN"),
            Tool::Level => ta_ui::i18n::n("LEVEL"),
            Tool::Tiles => ta_ui::i18n::n("TILES"),
            Tool::Copy => ta_ui::i18n::n("COPY"),
            Tool::Features => ta_ui::i18n::n("FEATURES"),
            Tool::Starts => ta_ui::i18n::n("STARTS"),
        }
    }

    fn height_op(self) -> bool {
        matches!(self, Tool::Raise | Tool::Lower | Tool::Smooth | Tool::Flatten | Tool::Level)
    }
}

/// Feature groups the feature list is filtered to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureGroup {
    Trees,
    Rocks,
    Metal,
    Vents,
    Wrecks,
    Other,
}

/// Dialogs over the editor.
#[derive(Clone, Debug, PartialEq)]
enum Dialog {
    None,
    NewMap,
    Open,
    Properties,
    TilePicker,
    /// Save under another name.
    SaveAs,
    /// A picture brought in (as heights, the map's art or a tile brush).
    Import,
    /// Saving would replace another map of the user's of that name (file stem).
    Replace(String),
    /// Unsaved changes would be lost by this.
    Discard(Pending),
}

/// A map switch or exit waiting for confirmation or for the loading notice to show.
#[derive(Clone, Debug, PartialEq)]
enum Pending {
    New(NewMap),
    Open(String),
    Exit,
    Quit,
}

/// Button clicks from the panels, carried out in the next [`Editor::update`].
#[derive(Default)]
struct Requests {
    undo: bool,
    redo: bool,
    save: bool,
    exit: bool,
    /// A map to switch to (after the unsaved-changes check).
    switch: Option<Pending>,
    /// A block of tiles chosen in the picker: the source map (`None` for this one), the tiles,
    /// and for a rectangle of its tile set (else of its art) the set's tiles a row as shown.
    block: Option<(Option<String>, Rect, Option<u32>)>,
    /// Properties edited in the dialog (compared with this state when it closed).
    props_done: Option<Snapshot>,
    remove_start: Option<usize>,
    add_start: Option<[f32; 2]>,
    reorder_start: Option<(usize, usize)>,
    /// A point of the map for the camera to look at (the navigator, a problem).
    look_at: Option<[f32; 2]>,
    /// The whole map in view.
    frame_map: bool,
    /// Save, then play a battle on the map.
    test_play: bool,
    /// A picture to bring in, as the Import dialog asked.
    import: Option<import_ui::ImportJob>,
    /// The heights to write out as a picture.
    export_heights: bool,
}

/// An edit in progress (mouse held down).
struct Stroke {
    before: Snapshot,
    label: &'static str,
    tool: Tool,
    heights: Option<HeightStroke>,
    /// Flatten's target height (the ground under the cursor when the stroke began).
    target: f32,
    /// First and last art tile stamped (the stamps' grid), last place a feature was put
    /// (drag spacing).
    first_tile: Option<(i32, i32)>,
    last_tile: Option<(i32, i32)>,
    last_point: Option<Vec3>,
    /// A start position taken hold of: the ground under the pointer and the start where it
    /// was then, and whether the pointer has moved enough to drag it (a click leaves it be).
    grab: Option<(Vec3, [f32; 2])>,
    moved: bool,
    /// Scattering features: how far the pointer moves before the next one (the last one's size).
    spacing: f32,
}

impl Stroke {
    fn new(before: Snapshot, label: &'static str, tool: Tool) -> Stroke {
        Stroke { before, label, tool, heights: None, target: 0.0, first_tile: None, last_tile: None, last_point: None, grab: None, moved: false, spacing: 0.0 }
    }
}

/// How far (map pixels) the pointer moves before a start position held moves with it.
const DRAG_START: f32 = 6.0;

/// The tile picker: a map's art, or its whole tile set, to choose a block of tiles from.
struct Picker {
    /// `None`: the map being edited.
    source: Option<String>,
    map: Option<Arc<TntMap>>,
    error: Option<String>,
    /// The picture: its texture, pixels per tile, and for the tile set its tiles a row (`None`:
    /// the art).
    texture: Option<(egui::TextureHandle, u32, Option<u32>)>,
    /// Show the tile set (every tile, in rows as wide as the picker) rather than the art.
    tile_set: bool,
    /// A source was chosen (else a new map's picker opens on its palette map).
    chosen: bool,
    /// Tiles selected (dragging, or the last selection).
    selection: Option<Rect>,
    drag_from: Option<(u32, u32)>,
    /// Pixels per tile in the picture.
    zoom: f32,
}

impl Picker {
    fn source_map(&mut self, vfs: &ta_vfs::Vfs) -> Result<Option<Arc<TntMap>>, String> {
        let Some(name) = &self.source else { return Ok(None) };
        if let Some(error) = &self.error { return Err(error.clone()); }
        if self.map.is_none() {
            match vfs.read(&format!("maps/{name}.tnt")).map_err(|error| error.to_string())
                .and_then(|bytes| TntMap::parse(&bytes).map_err(|error| error.to_string()))
            {
                Ok(map) => self.map = Some(Arc::new(map)),
                Err(error) => {
                    let error = format!("{name}: {error}");
                    self.error = Some(error.clone());
                    return Err(error);
                }
            }
        }
        Ok(self.map.clone())
    }
}

pub struct Editor {
    doc: MapDoc,
    history: History,
    saved_version: u64,
    view: View,
    pub tool: Tool,
    radius: f32,
    strength: f32,
    level: f32,
    stroke: Option<Stroke>,
    /// The ground point under the mouse.
    cursor: Option<Vec3>,
    block: Option<TileBlock>,
    block_texture: Option<(egui::TextureHandle, u32, u32)>,
    clip: Option<Clip>,
    paste_heights: bool,
    /// Pasted heights laid on the ground there (see [`tools::PasteWith::relative`]).
    paste_relative: bool,
    /// Pastes bring the copy's features and impassable cells.
    paste_features: bool,
    /// Tiles being selected with a drag (art tiles, from and to).
    selecting: Option<((i32, i32), (i32, i32))>,
    selecting_tool: Tool,
    feature_group: FeatureGroup,
    feature_search: String,
    feature_random: bool,
    feature_choice: Option<usize>,
    /// Features of every world on offer, not only the map's planet's.
    feature_all_worlds: bool,
    /// Scatter: features dropped at random in the brush's circle while the button is held, as
    /// thick as `density` (1-10), on land or under water as where the stroke began.
    scatter: bool,
    density: f32,
    /// The feature about to be placed, where it would stand and whether it fits there (drawn
    /// see-through), and the type whose art was last loaded for it.
    ghost: Option<(usize, Vec3, bool)>,
    ghost_loaded: Option<usize>,
    /// The features' pictures in the list, by feature, small for its row and whole (`None`: one
    /// with no sprite).
    feature_pictures: HashMap<usize, Option<(egui::TextureHandle, egui::TextureHandle)>>,
    /// The copied region's picture.
    clip_texture: Option<egui::TextureHandle>,
    /// Start position being dragged.
    dragging_start: Option<usize>,
    /// The interface's pixels per point as the overlays were last drawn (the start badges'
    /// size on the screen).
    ui_scale: f32,
    dialog: Dialog,
    new_form: NewMap,
    open_filter: String,
    /// The Open dialog's maps (read when it opens) and the one picked.
    open_maps: Option<Vec<ui::OpenEntry>>,
    open_choice: Option<String>,
    picker: Picker,
    /// The properties dialog's state when it opened (for its undo entry).
    props_before: Option<Snapshot>,
    /// Status messages: text, seconds left, is an error.
    messages: Vec<(String, f32, bool)>,
    /// A map switch shown as "loading" this frame and done next frame.
    loading: Option<Pending>,
    requests: Requests,
    /// For the random feature choice.
    seed: u64,
    changes: Changes,
    /// Camera framing wanted (a map was loaded).
    frame_camera: bool,
    /// Seconds since the editor opened (animated features).
    time: f32,
    /// The ground coloured by height with contour lines (H).
    height_map: bool,
    /// Scripted mouse actions still to play (`--editor-open demo`), and where the scripted
    /// mouse is.
    script: std::collections::VecDeque<Step>,
    script_mouse: glam::Vec2,
    /// What the view shows over the ground (the grid, features, start positions).
    pub layers: Layers,
    /// The page of keys and mouse buttons (F1).
    help: bool,
    /// The map in miniature in a corner of the view.
    navigator: navigator::Navigator,
    /// What is wrong with the map (see [`checks`]), worked out for `problems_version` of the
    /// history `problems_age` seconds ago.
    problems: Vec<checks::Problem>,
    problems_age: f32,
    problems_version: Option<u64>,
    /// The Save As dialog's name.
    save_as_name: String,
    /// The file stem this map was saved as or opened from (one of the user's own): saving it
    /// again replaces that map without asking; any other map of the user's asks first.
    saved_stem: Option<String>,
    /// Another map of the user's that saving may replace (answered yes).
    replace_ok: Option<String>,
    /// A test battle waits for the replace question's answer (then it is saved and played).
    test_after_replace: bool,
    /// The Import dialog's picture and choices.
    import: import_ui::Import,
    /// TA's palette as a table of nearest colours (made the first time a picture comes in).
    quantizer: Option<Arc<import::Quantizer>>,
    /// The sea level and planet the view shows while Properties is open (they show as they
    /// are changed, a few times a second), and seconds since they were last shown.
    shown_world: (i32, String),
    shown_age: f32,
}

impl Editor {
    /// Opens the editor on a new or an existing map. A map that can't be opened falls back to
    /// a new one (and says why).
    pub fn start(assets: &Assets, renderer: &mut Renderer, start: Start, panel: Option<&str>) -> anyhow::Result<Editor> {
        let load = |start: &Start| {
            let doc = load_doc(assets, start)?;
            renderer.map_fits(&doc.tnt).map_err(anyhow::Error::msg)?;
            Ok::<_, anyhow::Error>(doc)
        };
        let (doc, problem) = match load(&start) {
            Ok(doc) => (doc, None),
            Err(e) => (load(&Start::New(default_new(assets)))?, Some(ta_ui::i18n::tf("Could not open the map: {error}", &[("error", &format!("{e:#}"))]))),
        };
        let palette = &assets.data.palette;
        let mut view = View::new(renderer, &doc, palette);
        view.place_features(renderer, &doc, &assets.data, &assets.vfs, palette);
        let new_form = match &start {
            Start::New(n) => n.clone(),
            Start::Open(_) => default_new(assets),
        };
        let mut editor = Self::from_document(doc, view, new_form);
        if let Some(p) = problem {
            editor.message(p, true);
        } else {
            editor.saved_stem = own_stem(assets, &start);
        }
        editor.feature_choice = editor.feature_list(&assets.data).first().copied();
        match panel {
            Some("new") => editor.dialog = Dialog::NewMap,
            Some("open") => editor.dialog = Dialog::Open,
            Some("properties") => editor.open_properties(),
            Some("picker") => {
                let source = editor.doc.tiles_from.clone();
                editor.open_picker(source);
            }
            Some("tileset") => {
                editor.picker.tile_set = true;
                editor.open_picker(None);
            }
            Some("heights") => editor.height_map = true,
            Some("help") => editor.help = true,
            Some("import") => {
                if let Some(path) = std::env::var_os("TA_EDITOR_PICTURE") {
                    editor.import.load(std::path::Path::new(&path));
                }
                editor.open_import(import_ui::ImportAs::Art);
            }
            Some("grid") => editor.layers.grid = true,
            Some("saveas") => editor.open_save_as(),
            Some("tiles") => editor.tool = Tool::Tiles,
            Some("copy") => editor.tool = Tool::Copy,
            Some("copied") => {
                // A copy of the ground round the first start position, 8 × 6 tiles.
                editor.tool = Tool::Copy;
                let [x, z] = editor.doc.starts.first().copied().unwrap_or([0.0, 0.0]);
                let (tx, tz) = (((x / doc::TILE_PX) as u32).saturating_sub(4), ((z / doc::TILE_PX) as u32).saturating_sub(3));
                let rect = tools::tile_rect((tx as i32, tz as i32), (tx as i32 + 7, tz as i32 + 5), editor.doc.tiles_wide(), editor.doc.tiles_high(), 64);
                editor.clip = Some(Clip::copy(&editor.doc, rect));
            }
            Some("features") => editor.tool = Tool::Features,
            Some("starts") => editor.tool = Tool::Starts,
            Some("demo") => editor.demo(assets),
            Some("demo-save") => {
                // The demo, then saved as "Editor Demo" (to the mods folder: point APPDATA
                // elsewhere for tests).
                editor.demo(assets);
                editor.doc.props.name = "Editor Demo".into();
                editor.script.push_back(Step::Save);
            }
            Some(t) => {
                if let Some(tool) = Tool::ALL.iter().find(|x| x.label().eq_ignore_ascii_case(t)) {
                    editor.tool = *tool;
                } else {
                    log::warn!("--editor-open {t}: no such panel or tool");
                }
            }
            None => {}
        }
        Ok(editor)
    }

    /// Back from a test battle on the map: the map in the renderer again, as it was left (its
    /// undo history and all), framed.
    pub fn resume(&mut self, assets: &Assets, renderer: &mut Renderer) {
        let palette = &assets.data.palette;
        self.view = View::new(renderer, &self.doc, palette);
        self.view.place_features(renderer, &self.doc, &assets.data, &assets.vfs, palette);
        self.stroke = None;
        self.selecting = None;
        self.cursor = None;
        self.ghost = None;
        self.ghost_loaded = None;
        self.frame_camera = true;
        self.navigator.dirty = true;
    }

    fn from_document(doc: MapDoc, view: View, new_form: NewMap) -> Self {
        Self {
            doc,
            history: History::default(),
            saved_version: 0,
            view,
            tool: Tool::Raise,
            radius: 96.0,
            strength: 0.5,
            level: 60.0,
            stroke: None,
            cursor: None,
            block: None,
            block_texture: None,
            clip: None,
            paste_heights: true,
            paste_relative: false,
            paste_features: true,
            selecting: None,
            selecting_tool: Tool::Tiles,
            feature_group: FeatureGroup::Trees,
            feature_search: String::new(),
            feature_random: false,
            feature_choice: None,
            feature_all_worlds: false,
            scatter: false,
            density: 5.0,
            ghost: None,
            ghost_loaded: None,
            feature_pictures: HashMap::new(),
            clip_texture: None,
            dragging_start: None,
            ui_scale: 1.0,
            dialog: Dialog::None,
            new_form,
            open_filter: String::new(),
            open_maps: None,
            open_choice: None,
            picker: Picker { source: None, map: None, error: None, texture: None, tile_set: false, chosen: false, selection: None, drag_from: None, zoom: 16.0 },
            props_before: None,
            messages: Vec::new(),
            loading: None,
            requests: Requests::default(),
            seed: 0x5EED_ED17,
            changes: Changes::default(),
            frame_camera: true,
            time: 0.0,
            height_map: false,
            script: Default::default(),
            script_mouse: glam::Vec2::ZERO,
            layers: Layers::default(),
            help: false,
            navigator: navigator::Navigator::default(),
            problems: Vec::new(),
            problems_age: 0.0,
            problems_version: None,
            save_as_name: String::new(),
            saved_stem: None,
            replace_ok: None,
            test_after_replace: false,
            import: import_ui::Import::default(),
            quantizer: None,
            shown_world: (0, String::new()),
            shown_age: 0.0,
        }
    }

    /// A message for a few seconds (translated here: `tf` for one with values).
    fn message(&mut self, text: impl Into<String>, error: bool) {
        let text = ta_ui::i18n::t(&text.into()).to_string();
        if error {
            log::warn!("editor: {text}");
        }
        self.messages.push((text, if error { 8.0 } else { 4.0 }, error));
        if self.messages.len() > 4 {
            self.messages.remove(0);
        }
    }

    fn unsaved(&self) -> bool {
        self.history.version != self.saved_version
    }

    /// The quit key: true when nothing would be lost (the program may close); otherwise the
    /// unsaved-changes question is asked, as leaving the editor asks it.
    pub(crate) fn may_quit(&mut self) -> bool {
        // A held brush and the live Properties form already changed the document,
        // even though their undo entries normally wait for release/closing the form.
        self.finish_stroke();
        self.finish_properties();
        self.selecting = None;
        if self.unsaved() {
            self.dialog = Dialog::Discard(Pending::Quit);
            return false;
        }
        true
    }

    /// Alt+click with the features tool: the feature under the pointer becomes the one to
    /// place (its group and, if it is another world's, every world shown).
    fn pick_feature(&mut self, p: Vec3, data: &ta_data::GameData) {
        let (cx, cz) = ((p.x / CELL_PX).max(0.0) as u32, (p.z / CELL_PX).max(0.0) as u32);
        let Some(name) = self.doc.feature_covering(data, cx, cz).map(|(_, name)| name.to_string()) else {
            return self.message("Alt+click on a feature to take it", false);
        };
        let Some(def) = data.feature_id(&name) else {
            return self.message(ta_ui::i18n::tf("{name} is not in the game's data", &[("name", &name)]), true);
        };
        let f = &data.features[def];
        self.feature_group = feature_group(f);
        self.feature_search.clear();
        if !world_matches(&self.doc.props.planet, &f.world) && feature_group(f) != FeatureGroup::Wrecks {
            self.feature_all_worlds = true;
        }
        self.feature_choice = Some(def);
        self.feature_random = false;
        self.message(ta_ui::i18n::tf("Placing {name}", &[("name", &ui::feature_title(&name))]), false);
    }

    /// A point kept inside the map.
    fn inside(&self, p: [f32; 2]) -> [f32; 2] {
        [p[0].clamp(0.0, self.doc.width_px() - 1.0), p[1].clamp(0.0, self.doc.height_px() - 1.0)]
    }

    fn open_save_as(&mut self) {
        self.finish_stroke();
        self.selecting = None;
        self.save_as_name = self.doc.props.name.clone();
        self.dialog = Dialog::SaveAs;
    }

    fn open_properties(&mut self) {
        self.finish_stroke();
        self.selecting = None;
        self.props_before = Some(Snapshot::of(&self.doc));
        self.dialog = Dialog::Properties;
    }

    fn discard_for(&mut self, pending: Pending) {
        // Do not advance saved_version: the replacement may fail to load, leaving
        // this same unsaved document on screen with its undo history intact.
        self.loading = Some(pending);
        self.dialog = Dialog::None;
    }

    /// Frames the whole map (after loading one).
    fn frame(&self, camera: &mut Camera) {
        let (w, h) = (self.doc.width_px(), self.doc.height_px());
        camera.focus = Vec3::new(w * 0.5, self.doc.ground(w * 0.5, h * 0.5).max(self.doc.tnt.sea_level as f32), h * 0.5);
        camera.yaw = 0.0;
        if camera.mode == ta_render::CameraMode::Perspective {
            camera.pitch = 0.95;
        }
        // The map fits the view right of the tool palette (about 300 px) and under the bar.
        let free = Vec3::new(camera.viewport.x - 300.0, camera.viewport.y - 40.0, 0.0);
        let fit = (free.x / w).min(free.y / h) * 0.85;
        camera.zoom = fit.clamp(0.08, 1.6);
        camera.pan_pixels(glam::Vec2::new(-150.0, -20.0));
    }

    /// The feature types offered for the map's planet and the chosen group, by name.
    fn feature_list(&self, data: &ta_data::GameData) -> Vec<usize> {
        let planet = self.doc.props.planet.to_ascii_lowercase();
        let search = self.feature_search.trim().to_ascii_lowercase();
        let mut list: Vec<usize> = (0..data.features.len())
            .filter(|&i| {
                let f = &data.features[i];
                feature_group(f) == self.feature_group
                    && (self.feature_all_worlds || feature_group(f) == FeatureGroup::Wrecks || world_matches(&planet, &f.world))
                    && (f.is_sprite() || !f.object.is_empty())
                    && (search.is_empty() || f.name.to_ascii_lowercase().contains(&search) || f.description.to_ascii_lowercase().contains(&search))
            })
            .collect();
        list.sort_by_key(|&i| data.features[i].name.to_ascii_lowercase());
        list
    }

    /// Once a frame before the UI: shortcuts, the tools under the mouse, the buttons clicked
    /// last frame, map switches; the edits reach the renderer.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        input: &Input,
        camera: &mut Camera,
        renderer: &mut Renderer,
        assets: &mut Assets,
        settings: &mut ta_config::Config,
        ui_pointer: bool,
        dt: f32,
    ) -> EditorAction {
        for m in &mut self.messages {
            m.1 -= dt;
        }
        self.messages.retain(|m| m.1 > 0.0);
        let mut action = EditorAction::None;
        // A map switch shown as "loading" last frame.
        if let Some(pending) = self.loading.take() {
            match pending {
                Pending::Exit => return EditorAction::Exit,
                Pending::Quit => return EditorAction::Quit,
                start => {
                    let start = match start {
                        Pending::New(n) => Start::New(n),
                        Pending::Open(name) => Start::Open(name),
                        Pending::Exit | Pending::Quit => unreachable!(),
                    };
                    self.switch_map(assets, renderer, start);
                }
            }
        }
        if std::mem::take(&mut self.frame_camera) || std::mem::take(&mut self.requests.frame_map) {
            self.frame(camera);
        }
        if let Some([x, z]) = self.requests.look_at.take() {
            camera.focus.x = x.clamp(0.0, self.doc.width_px());
            camera.focus.z = z.clamp(0.0, self.doc.height_px());
        }
        self.update_problems(assets, dt);
        // A scripted demo (captures) plays its mouse actions in place of the mouse.
        let scripted = self.scripted_input(camera);
        let (input, ui_pointer) = match &scripted {
            Some(i) => (i, false),
            None => (input, ui_pointer),
        };
        let ctrl = input.modifiers.control_key();
        let shift = input.modifiers.shift_key();
        let dialog_open = self.dialog != Dialog::None;
        // Shortcuts (text fields in focus take the keys first: see `Input`).
        if !dialog_open {
            if ctrl && input.pressed(KeyCode::KeyZ) {
                if shift { self.requests.redo = true } else { self.requests.undo = true }
            }
            if ctrl && input.pressed(KeyCode::KeyY) {
                self.requests.redo = true;
            }
            if ctrl && input.pressed(KeyCode::KeyS) {
                if shift { self.open_save_as() } else { self.requests.save = true }
            }
            if input.pressed(KeyCode::F1) {
                self.help = !self.help;
            }
            if input.pressed(KeyCode::F5) {
                self.requests.test_play = true;
            }
            if input.pressed(KeyCode::Home) {
                self.requests.frame_map = true;
            }
            if ctrl && input.pressed(KeyCode::KeyN) {
                self.dialog = Dialog::NewMap;
            }
            if ctrl && input.pressed(KeyCode::KeyO) {
                self.dialog = Dialog::Open;
            }
            if !ctrl {
                let digits = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9];
                for (k, tool) in digits.iter().zip(Tool::ALL) {
                    if input.pressed(*k) {
                        self.tool = tool;
                    }
                }
                if input.pressed(KeyCode::BracketLeft) {
                    self.radius = (self.radius / 1.25).max(16.0);
                }
                if input.pressed(KeyCode::BracketRight) {
                    self.radius = (self.radius * 1.25).min(1024.0);
                }
                if input.pressed(KeyCode::KeyH) {
                    self.height_map = !self.height_map;
                }
                if input.pressed(KeyCode::KeyG) {
                    self.layers.grid = !self.layers.grid;
                }
            }
            if input.pressed(KeyCode::Escape) {
                self.help = false;
                self.selecting = None;
                if self.tool == Tool::Copy {
                    self.clip = None;
                }
            }
        }
        if dialog_open && input.pressed(KeyCode::Escape) && matches!(self.dialog, Dialog::TilePicker | Dialog::Open | Dialog::NewMap | Dialog::SaveAs | Dialog::Import) {
            self.dialog = Dialog::None;
        }
        if self.dialog != Dialog::None || self.stroke.as_ref().is_some_and(|s| s.tool != self.tool)
            || self.selecting.is_some() && self.selecting_tool != self.tool
        {
            self.finish_stroke();
            self.selecting = None;
        }
        self.run_requests(assets, renderer, settings, &mut action);
        self.preview_properties(renderer, assets, dt);

        // Shortcuts and queued button actions may have opened a dialog or begun
        // leaving this document during this very frame.
        let dialog_open = self.dialog != Dialog::None || self.loading.is_some() || matches!(action, EditorAction::Exit | EditorAction::Quit);

        // The ground under the mouse.
        self.cursor = if ui_pointer || dialog_open {
            None
        } else {
            renderer.terrain.as_ref().and_then(|t| {
                let (o, d) = camera.screen_ray(input.mouse);
                t.field.raycast(o, d)
            })
        };
        let held = input.button_held(MouseButtonId::Left);
        let pressed = input.button_pressed(MouseButtonId::Left) && !ui_pointer && !dialog_open;
        let alt = input.modifiers.alt_key();
        if pressed && alt && self.tool == Tool::Features && let Some(p) = self.cursor {
            self.pick_feature(p, &assets.data);
        } else if pressed && let Some(p) = self.cursor {
            self.begin_stroke(p, ctrl, input, camera);
            // (A click that came and went within the frame still does its tool's work once.)
            if !held && self.stroke.is_some() {
                self.continue_stroke(p, ctrl, shift, dt, &assets.data);
            }
        }
        if self.stroke.is_some() {
            if held {
                if let Some(p) = self.cursor {
                    self.continue_stroke(p, ctrl, shift, dt, &assets.data);
                }
            } else {
                self.finish_stroke();
            }
        }
        self.track_selection(input);
        self.ghost = match (self.tool, self.feature_choice, self.cursor) {
            (Tool::Features, Some(def), Some(p)) if !ctrl && !alt && !self.feature_random && !self.scatter => {
                let d = &assets.data.features[def];
                let (fx, fz) = (d.footprint_x.max(1), d.footprint_z.max(1));
                tools::footprint_origin(&self.doc, p.x, p.z, fx, fz).map(|(ox, oz)| {
                    let (x, z) = ((ox as f32 + fx as f32 / 2.0) * CELL_PX, (oz as f32 + fz as f32 / 2.0) * CELL_PX);
                    (def, Vec3::new(x, self.doc.ground(x, z), z), !self.doc.taken(&assets.data, (ox, oz), (fx, fz)))
                })
            }
            _ => None,
        };
        if let Some((def, ..)) = self.ghost && self.ghost_loaded != Some(def) {
            self.view.load_visual(renderer, &assets.data, &assets.vfs, &assets.data.palette, def);
            self.ghost_loaded = Some(def);
        }
        if self.changes.any() {
            let changes = std::mem::take(&mut self.changes);
            if changes.tiles.is_some() {
                self.navigator.dirty = true;
            }
            self.view.push(renderer, &self.doc, &assets.data, &assets.vfs, &assets.data.palette, &changes);
            if self.stroke.is_none() && (changes.heights.is_some() || changes.sea_level) {
                self.view.refresh_water(renderer, &self.doc, &assets.data.palette);
            }
        }
        action
    }

    /// Carries out the panels' clicks from the last frame.
    fn run_requests(&mut self, assets: &mut Assets, renderer: &mut Renderer, settings: &mut ta_config::Config, action: &mut EditorAction) {
        self.finish_requested_interactions();
        let mut r = std::mem::take(&mut self.requests);
        // Closing Properties and pressing Undo in the same frame must undo the
        // property edit, not the edit that preceded it.
        if let Some(before) = r.props_done.take() && let Some(c) = self.history.record(before, &self.doc, ta_ui::i18n::n("Properties")) {
            self.changed(c, renderer, assets);
        }
        self.apply_requests(r, assets, renderer, settings, action);
    }

    fn finish_requested_interactions(&mut self) {
        // A request acts on the finished edit, never underneath a brush's stale
        // snapshot/full-precision height buffer. A held mouse needs a new press
        // before another stroke can begin after one of these commands.
        if self.requests.undo || self.requests.redo || self.requests.save || self.requests.exit
            || self.requests.switch.is_some() || self.requests.block.is_some()
            || self.requests.remove_start.is_some() || self.requests.add_start.is_some() || self.requests.reorder_start.is_some()
        {
            self.finish_stroke();
            self.selecting = None;
        }
    }

    fn apply_requests(&mut self, r: Requests, assets: &mut Assets, renderer: &mut Renderer, settings: &mut ta_config::Config, action: &mut EditorAction) {
        if r.undo {
            match self.history.undo(&mut self.doc) {
                Some(c) => {
                    log::info!("map editor: undo {}", self.history.next_redo().unwrap_or_default());
                    self.changed(c, renderer, assets);
                }
                None => self.message("Nothing to undo", false),
            }
        }
        if r.redo {
            match self.history.redo(&mut self.doc) {
                Some(c) => {
                    log::info!("map editor: redo {}", self.history.next_undo().unwrap_or_default());
                    self.changed(c, renderer, assets);
                }
                None => self.message("Nothing to redo", false),
            }
        }
        if let Some(i) = r.remove_start {
            let before = Snapshot::of(&self.doc);
            if i < self.doc.starts.len() {
                self.doc.starts.remove(i);
            }
            self.record(before, ta_ui::i18n::n("Remove start position"), renderer, assets);
        }
        if let Some(p) = r.add_start {
            let before = Snapshot::of(&self.doc);
            if self.doc.starts.len() < MAX_STARTS {
                self.doc.starts.push(p);
            }
            self.record(before, ta_ui::i18n::n("Add start position"), renderer, assets);
        }
        if let Some((a, b)) = r.reorder_start {
            let before = Snapshot::of(&self.doc);
            if a < self.doc.starts.len() && b < self.doc.starts.len() {
                self.doc.starts.swap(a, b);
            }
            self.record(before, ta_ui::i18n::n("Renumber start positions"), renderer, assets);
        }
        if let Some((source, rect, set)) = r.block {
            let tile_count = self.doc.tnt.tiles.len();
            let other = source.is_some().then(|| self.picker.map.clone()).flatten();
            let block = if source.is_some() && other.is_none() {
                Err(anyhow::anyhow!(ta_ui::i18n::t("No tiles there")))
            } else { match (other, set) {
                (None, None) => Ok(TileBlock::from_map(&self.doc, rect)),
                (Some(map), None) => TileBlock::import(&mut self.doc, &map, rect),
                (None, Some(columns)) => TileBlock::from_set(&mut self.doc, None, rect, columns),
                (Some(map), Some(columns)) => TileBlock::from_set(&mut self.doc, Some(&map), rect, columns),
            } };
            match block {
                Ok(block) if block.tiles.is_empty() => self.message("No tiles there", true),
                Ok(block) => {
                    if let Err(error) = renderer.map_fits(&self.doc.tnt) {
                        self.doc.truncate_tiles(tile_count);
                        self.message(ta_ui::i18n::tf("Could not open the map: {error}", &[("error", &error)]), true);
                    } else {
                        self.set_block(block);
                        self.tool = Tool::Tiles;
                    }
                }
                Err(error) => self.message(error.to_string(), true),
            }
        }
        if r.save {
            self.save(assets, settings, action);
        }
        if let Some(job) = r.import {
            self.run_import(job, renderer, assets);
        }
        if r.export_heights {
            self.export_heights();
        }
        if r.test_play {
            if self.doc.starts.len() < 2 {
                self.message("A battle needs two start positions: add them with the start positions tool (9)", true);
            } else {
                self.save(assets, settings, action);
                if let EditorAction::Saved(stem) = action {
                    *action = EditorAction::TestPlay(stem.clone());
                }
                self.test_after_replace = matches!(self.dialog, Dialog::Replace(_));
            }
        }
        if let Some(p) = r.switch {
            if self.unsaved() {
                self.dialog = Dialog::Discard(p);
            } else {
                self.loading = Some(p);
            }
        }
        if r.exit {
            if self.unsaved() {
                self.dialog = Dialog::Discard(Pending::Exit);
            } else {
                *action = EditorAction::Exit;
            }
        }
    }

    /// Brings a picture in, as the Import dialog asked: the heights, the whole map's art (one
    /// edit to undo), or a brush of tiles for the Tiles tool.
    fn run_import(&mut self, job: import_ui::ImportJob, renderer: &mut Renderer, assets: &Assets) {
        use import_ui::ImportAs;
        match job.what {
            ImportAs::Heights => {
                let before = Snapshot::of(&self.doc);
                self.doc.tnt.heights = import::heights_from(&job.picture, self.doc.tnt.width, self.doc.tnt.height, job.lowest, job.highest);
                self.record(before, ta_ui::i18n::n("Import heights"), renderer, assets);
                self.message("The picture is the ground's heights now (Ctrl+Z takes it back)", false);
            }
            ImportAs::Art | ImportAs::Brush => {
                let quantizer = self.quantizer.get_or_insert_with(|| Arc::new(import::Quantizer::new(&assets.data.palette))).clone();
                let count = self.doc.tnt.tiles.len();
                let block = match import::art_tiles(&mut self.doc, &job.picture, job.tiles.0, job.tiles.1, &quantizer, job.dither) {
                    Ok(block) => block,
                    Err(error) => return self.message(error.to_string(), true),
                };
                if let Err(error) = renderer.map_fits(&self.doc.tnt) {
                    self.doc.truncate_tiles(count);
                    return self.message(ta_ui::i18n::tf("The map can't show that many tiles: {error}", &[("error", &error)]), true);
                }
                log::info!("map editor: a picture made {}×{} tiles ({} new)", block.w, block.h, self.doc.tnt.tiles.len() - count);
                if job.what == ImportAs::Brush {
                    self.message(ta_ui::i18n::tf("Painting with the picture: {w} × {h} tiles", &[("w", &block.w), ("h", &block.h)]), false);
                    self.set_block(block);
                    self.tool = Tool::Tiles;
                } else {
                    let before = Snapshot::of(&self.doc);
                    block.stamp(&mut self.doc, 0, 0);
                    self.record(before, ta_ui::i18n::n("Import picture"), renderer, assets);
                    self.message("The picture is the map's art now (Ctrl+Z takes it back)", false);
                }
            }
        }
    }

    fn set_block(&mut self, block: TileBlock) {
        self.block = Some(block);
        self.block_texture = None;
    }

    /// Records an edit made outside a stroke and shows it.
    fn record(&mut self, before: Snapshot, label: &str, renderer: &mut Renderer, assets: &Assets) {
        if let Some(c) = self.history.record(before, &self.doc, label) {
            self.changed(c, renderer, assets);
        }
    }

    fn changed(&mut self, c: Changes, renderer: &mut Renderer, assets: &Assets) {
        if c.tiles.is_some() || c.sea_level {
            self.navigator.dirty = true;
        }
        if c.props {
            let choices = self.feature_list(&assets.data);
            if self.feature_choice.is_none_or(|choice| !choices.contains(&choice)) {
                self.feature_choice = choices.first().copied();
            }
        }
        self.view.push(renderer, &self.doc, &assets.data, &assets.vfs, &assets.data.palette, &c);
        if c.heights.is_some() || c.sea_level {
            self.view.refresh_water(renderer, &self.doc, &assets.data.palette);
        }
    }

    fn begin_stroke(&mut self, p: Vec3, ctrl: bool, input: &Input, camera: &Camera) {
        // (Two presses in one frame: the first one's edit is recorded, not lost.)
        self.finish_stroke();
        // (The undo buttons name the edit, translated: `n`.)
        let n = ta_ui::i18n::n;
        let label = match self.tool {
            Tool::Raise => n("Raise"),
            Tool::Lower => n("Lower"),
            Tool::Smooth => n("Smooth"),
            Tool::Flatten => n("Flatten"),
            Tool::Level => n("Set level"),
            Tool::Tiles => n("Paint tiles"),
            Tool::Copy => n("Paste"),
            Tool::Features => if ctrl { n("Remove features") } else { n("Place features") },
            Tool::Starts => n("Move start position"),
        };
        if self.tool == Tool::Level && ctrl {
            // Ctrl+click takes the level from the ground.
            self.level = p.y.round();
            return;
        }
        let art = self.doc.art_tile_at(p.x, p.y, p.z);
        if matches!(self.tool, Tool::Tiles | Tool::Copy) && (ctrl || self.tool == Tool::Copy && self.clip.is_none()) {
            // Select a region: tiles to paint with, or to copy.
            self.selecting = Some((art, art));
            self.selecting_tool = self.tool;
            return;
        }
        if self.tool == Tool::Tiles && self.block.is_none() {
            self.message("Choose tiles first (CHOOSE TILES, or Ctrl+drag over the map)", false);
            return;
        }
        if self.tool == Tool::Features && self.feature_choice.is_none() && !ctrl {
            self.message("Choose a feature from the list first", false);
            return;
        }
        if self.tool == Tool::Starts {
            // The badge under the pointer, as they are drawn (moved apart where they would
            // cover one another); the nearest, should two be.
            let projector = view::Projector::new(camera, 1.0);
            let mouse = egui::pos2(input.mouse.x, input.mouse.y);
            let heads: Vec<Option<egui::Pos2>> = self
                .doc
                .starts
                .iter()
                .map(|s| projector.project(Vec3::new(s[0], self.doc.ground(s[0], s[1]).max(self.doc.tnt.sea_level as f32), s[1]) + Vec3::Y * 24.0))
                .collect();
            let badge = ui::START_BADGE * self.ui_scale;
            let hit = view::badge_layout(&heads, badge)
                .iter()
                .enumerate()
                .filter_map(|(i, b)| b.map(|b| (i, b.distance(mouse))))
                .filter(|&(_, d)| d < badge + 4.0 * self.ui_scale)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            let before = Snapshot::of(&self.doc);
            match (hit, ctrl) {
                (Some(i), true) => {
                    self.doc.starts.remove(i);
                    self.changes.starts = true;
                    self.stroke = Some(Stroke::new(before, "Remove start position", self.tool));
                }
                (Some(i), false) => {
                    self.dragging_start = Some(i);
                    let mut stroke = Stroke::new(before, label, self.tool);
                    stroke.grab = Some((p, self.doc.starts[i]));
                    self.stroke = Some(stroke);
                }
                (None, true) => self.message("Ctrl+click on a start position's marker to remove it", false),
                (None, false) if self.doc.starts.len() < MAX_STARTS => {
                    let at = self.inside([p.x, p.z]);
                    self.doc.starts.push(at);
                    self.dragging_start = Some(self.doc.starts.len() - 1);
                    self.changes.starts = true;
                    let mut stroke = Stroke::new(before, "Add start position", self.tool);
                    stroke.grab = Some((p, at));
                    stroke.moved = true;
                    self.stroke = Some(stroke);
                }
                (None, false) => self.message(ta_ui::i18n::tf("A map has at most {n} start positions", &[("n", &MAX_STARTS)]), true),
            }
            return;
        }
        let mut stroke = Stroke::new(Snapshot::of(&self.doc), label, self.tool);
        stroke.heights = self.tool.height_op().then(|| HeightStroke::new(&self.doc));
        // (Flatten's height, and whether a scatter keeps to the land or the sea.)
        stroke.target = p.y;
        self.stroke = Some(stroke);
    }

    fn continue_stroke(&mut self, p: Vec3, ctrl: bool, shift: bool, dt: f32, data: &ta_data::GameData) {
        // (Any feature of the list as it is filtered, when placing at random.)
        let random = (self.tool == Tool::Features && self.feature_random && !ctrl).then(|| self.feature_list(data));
        let Some(stroke) = self.stroke.as_mut() else { return };
        let doc = &mut self.doc;
        match self.tool {
            Tool::Raise | Tool::Lower | Tool::Smooth | Tool::Flatten | Tool::Level => {
                let op = match (self.tool, shift) {
                    (Tool::Raise, false) | (Tool::Lower, true) => HeightOp::Raise,
                    (Tool::Raise, true) | (Tool::Lower, false) => HeightOp::Lower,
                    (Tool::Smooth, _) => HeightOp::Smooth,
                    (Tool::Flatten, _) => HeightOp::Flatten(stroke.target),
                    _ => HeightOp::Level(self.level),
                };
                if let Some(hs) = stroke.heights.as_mut() {
                    let changed = hs.apply(doc, p.x, p.z, self.radius, self.strength, op, dt);
                    self.changes.heights = Rect::merge(self.changes.heights, changed);
                }
            }
            Tool::Tiles => {
                let Some(block) = &self.block else { return };
                let art = doc.art_tile_at(p.x, p.y, p.z);
                let at = (art.0 - block.w as i32 / 2, art.1 - block.h as i32 / 2);
                // Dragging lays the block out on a grid from the first stamp, so the
                // stamps join without gaps or overlaps.
                let (w, h) = (block.w.max(1) as i32, block.h.max(1) as i32);
                let at = match stroke.first_tile {
                    None => {
                        stroke.first_tile = Some(at);
                        at
                    }
                    Some(o) => (o.0 + ((at.0 - o.0) as f32 / w as f32).round() as i32 * w, o.1 + ((at.1 - o.1) as f32 / h as f32).round() as i32 * h),
                };
                if stroke.last_tile != Some(at) {
                    stroke.last_tile = Some(at);
                    let changed = block.stamp(doc, at.0, at.1);
                    self.changes.tiles = Rect::merge(self.changes.tiles, changed);
                }
            }
            Tool::Copy => {
                let Some(clip) = &self.clip else { return };
                let art = doc.art_tile_at(p.x, p.y, p.z);
                let at = (art.0 - clip.tiles.w as i32 / 2, art.1 - clip.tiles.h as i32 / 2);
                // One paste per click.
                if stroke.last_tile.is_none() {
                    stroke.last_tile = Some(at);
                    let with = tools::PasteWith { heights: self.paste_heights, relative: self.paste_relative, features: self.paste_features };
                    let pasted = clip.paste(doc, data, at.0, at.1, with);
                    self.changes.tiles = Rect::merge(self.changes.tiles, pasted.tiles);
                    self.changes.heights = Rect::merge(self.changes.heights, pasted.cells);
                    self.changes.features |= pasted.features;
                }
            }
            Tool::Features if ctrl => {
                if tools::erase_features(doc, data, p.x, p.z, self.radius) > 0 {
                    self.changes.features = true;
                }
            }
            Tool::Features if self.scatter => {
                let Some(chosen) = self.feature_choice else { return };
                // A burst for the click that starts it, then a steady spray while it is held.
                let tries = if stroke.last_point.is_none() { 12 } else { ((40.0 * dt).round() as usize).max(1) };
                stroke.last_point = Some(p);
                let ground = if stroke.target < doc.tnt.sea_level as f32 { tools::Ground::Water } else { tools::Ground::Land };
                let brush = tools::Scatter { radius: self.radius, density: self.density, ground };
                let pick = |k: f32| match &random {
                    Some(list) if !list.is_empty() => list[((k * list.len() as f32) as usize).min(list.len() - 1)],
                    _ => chosen,
                };
                if brush.spray(doc, data, &mut self.seed, (p.x, p.z), tries, pick) > 0 {
                    self.changes.features = true;
                }
            }
            Tool::Features => {
                let Some(chosen) = self.feature_choice else { return };
                // Scatter: another feature once the cursor has moved the last one's size away.
                if stroke.last_point.is_some_and(|q| q.distance(p) < stroke.spacing) {
                    return;
                }
                let def = match &random {
                    Some(list) if !list.is_empty() => {
                        self.seed ^= self.seed << 13;
                        self.seed ^= self.seed >> 7;
                        self.seed ^= self.seed << 17;
                        list[(self.seed % list.len() as u64) as usize]
                    }
                    _ => chosen,
                };
                let d = &data.features[def];
                // (Where another feature or impassable ground already is, nothing is placed.)
                if let Some(origin) = tools::footprint_origin(doc, p.x, p.z, d.footprint_x.max(1), d.footprint_z.max(1)) && doc.place_feature(data, &d.name, origin) {
                    self.changes.features = true;
                    stroke.last_point = Some(p);
                    stroke.spacing = (d.footprint_x.max(d.footprint_z) as f32 * CELL_PX * 1.5).max(24.0);
                }
            }
            Tool::Starts => {
                // Moved by the pointer's move since it took hold (the marker is drawn above
                // its spot): a click on it leaves it where it is.
                let Some((from, start)) = stroke.grab else { return };
                let delta = [p.x - from.x, p.z - from.z];
                if !stroke.moved && delta[0].hypot(delta[1]) < DRAG_START {
                    return;
                }
                stroke.moved = true;
                let (w, h) = (doc.width_px() - 1.0, doc.height_px() - 1.0);
                if let Some(i) = self.dragging_start && let Some(s) = doc.starts.get_mut(i) {
                    *s = [(start[0] + delta[0]).clamp(0.0, w), (start[1] + delta[1]).clamp(0.0, h)];
                    self.changes.starts = true;
                }
            }
        }
    }

    fn finish_stroke(&mut self) {
        self.dragging_start = None;
        if let Some(stroke) = self.stroke.take() && let Some(c) = self.history.record(stroke.before, &self.doc, stroke.label) {
            log::info!("map editor: {} ({c:?})", stroke.label);
            self.queue_changes(c);
        }
    }

    /// While Properties is open, the sea level and planet as they are being changed show in
    /// the view (at most a few times a second: a new sea level loads the terrain again).
    fn preview_properties(&mut self, renderer: &mut Renderer, assets: &Assets, dt: f32) {
        let now = (self.doc.tnt.sea_level, self.doc.props.planet.clone());
        if self.dialog != Dialog::Properties {
            self.shown_world = now;
            return;
        }
        self.shown_age += dt;
        if now == self.shown_world || self.shown_age < 0.25 {
            return;
        }
        let changes = Changes { sea_level: now.0 != self.shown_world.0, props: true, ..Default::default() };
        self.shown_world = now;
        self.shown_age = 0.0;
        self.view.push(renderer, &self.doc, &assets.data, &assets.vfs, &assets.data.palette, &changes);
        if changes.sea_level {
            self.view.refresh_water(renderer, &self.doc, &assets.data.palette);
        }
    }

    fn finish_properties(&mut self) {
        if let Some(before) = self.requests.props_done.take().or_else(|| self.props_before.take())
            && let Some(c) = self.history.record(before, &self.doc, ta_ui::i18n::n("Properties"))
        {
            self.queue_changes(c);
        }
    }

    fn queue_changes(&mut self, c: Changes) {
        self.changes.heights = Rect::merge(self.changes.heights, c.heights);
        self.changes.tiles = Rect::merge(self.changes.tiles, c.tiles);
        self.changes.features |= c.features;
        self.changes.starts |= c.starts;
        self.changes.props |= c.props;
        self.changes.sea_level |= c.sea_level;
    }

    /// The region selection (drag) finished: tiles to paint with, or a copy.
    fn finish_selection(&mut self, to: (i32, i32)) {
        let Some((from, _)) = self.selecting.take() else { return };
        let rect = tools::tile_rect(from, to, self.doc.tiles_wide(), self.doc.tiles_high(), 64);
        log::info!("map editor: {} tiles {rect:?}", if self.tool == Tool::Copy { "copied" } else { "picked" });
        match self.tool {
            Tool::Copy => {
                self.clip = Some(Clip::copy(&self.doc, rect));
                self.clip_texture = None;
                self.message(ta_ui::i18n::tf("Copied {width} × {height} tiles: click to paste", &[("width", &rect.width()), ("height", &rect.height())]), false);
            }
            _ => {
                self.set_block(TileBlock::from_map(&self.doc, rect));
                self.message(ta_ui::i18n::tf("Painting with {width} × {height} tiles from the map", &[("width", &rect.width()), ("height", &rect.height())]), false);
            }
        }
    }

    /// Selection drags (Tiles with Ctrl, Copy) follow the mouse and end on release.
    pub fn track_selection(&mut self, input: &Input) {
        let Some((from, _)) = self.selecting else { return };
        if let Some(p) = self.cursor {
            let to = self.doc.art_tile_at(p.x, p.y, p.z);
            self.selecting = Some((from, to));
        }
        if !input.button_held(MouseButtonId::Left) {
            let to = self.selecting.map_or(from, |s| s.1);
            self.finish_selection(to);
        }
    }

    fn switch_map(&mut self, assets: &Assets, renderer: &mut Renderer, start: Start) {
        // Validate before replacing the document or touching GPU terrain. Keeping the old
        // editor intact also preserves its unsaved protection when the new map is too large.
        match load_doc(assets, &start).and_then(|doc| {
            renderer.map_fits(&doc.tnt).map_err(anyhow::Error::msg)?;
            Ok(doc)
        }) {
            Ok(doc) => {
                let palette = &assets.data.palette;
                self.view = View::new(renderer, &doc, palette);
                self.view.place_features(renderer, &doc, &assets.data, &assets.vfs, palette);
                self.doc = doc;
                self.history = History::default();
                self.saved_stem = own_stem(assets, &start);
                self.replace_ok = None;
                self.saved_version = 0;
                self.stroke = None;
                self.selecting = None;
                self.dragging_start = None;
                self.cursor = None;
                self.props_before = None;
                self.requests = Requests::default();
                self.changes = Changes::default();
                self.clip = None;
                self.block = None;
                self.block_texture = None;
                self.picker.map = None;
                self.picker.error = None;
                self.picker.texture = None;
                self.picker.selection = None;
                self.picker.source = None;
                self.picker.chosen = false;
                self.picker.drag_from = None;
                self.clip_texture = None;
                self.ghost = None;
                self.ghost_loaded = None;
                self.feature_choice = self.feature_list(&assets.data).first().copied();
                self.frame_camera = true;
                self.message(format!("{} — {} × {}", self.doc.props.name, self.doc.width_px(), self.doc.height_px()), false);
            }
            Err(e) => self.message(ta_ui::i18n::tf("Could not open the map: {error}", &[("error", &format!("{e:#}"))]), true),
        }
    }

    fn save(&mut self, assets: &mut Assets, settings: &mut ta_config::Config, action: &mut EditorAction) {
        let mods = save::user_mods_dir();
        if let Some(problem) = save::name_problem(&assets.vfs, &self.doc.props.name, &mods) {
            self.message(problem, true);
            self.open_properties();
            return;
        }
        // Another map of the user's of this name: replaced only once they say so.
        let stem = save::file_stem(&self.doc.props.name);
        let same = |a: &Option<String>| a.as_ref().is_some_and(|s| s.eq_ignore_ascii_case(&stem));
        if mods.join(&stem).exists() && !same(&self.saved_stem) && !same(&self.replace_ok) {
            self.dialog = Dialog::Replace(stem);
            return;
        }
        self.replace_ok = None;
        let author = settings.gameplay.player_name.clone();
        match save::write_mod(&self.doc, &assets.data.palette, &mods, &author).and_then(|saved| save::publish(&saved, assets, settings).map(|_| saved)) {
            Ok(saved) => {
                self.saved_version = self.history.version;
                self.saved_stem = Some(saved.stem.clone());
                log::info!("map editor: saved {} as the mod {}", saved.dir.display(), saved.id);
                let playable = if self.doc.starts.len() >= 2 { ta_ui::i18n::t("listed under Skirmish") } else { ta_ui::i18n::t("Skirmish needs two start positions") };
                self.message(ta_ui::i18n::tf("Saved \"{name}\" as a mod in your mods folder: {playable}", &[("name", &saved.stem), ("playable", &playable)]), false);
                *action = EditorAction::Saved(saved.stem);
            }
            Err(e) => self.message(ta_ui::i18n::tf("Could not save: {error}", &[("error", &format!("{e:#}"))]), true),
        }
    }

    /// Submits the features to the renderer (nothing else moves in the editor), and the height
    /// map when it is on.
    pub fn prepare(&mut self, renderer: &mut Renderer, camera: &Camera, dt: f32) {
        self.time += dt;
        let (min, max) = camera.ground_bounds(260.0);
        let ghost = self.ghost.filter(|_| self.layers.features && self.dialog == Dialog::None);
        self.view.prepare(renderer, (min, max), self.time, camera.mode == ta_render::CameraMode::Perspective, self.layers.features, ghost);
        renderer.set_overlay(if self.height_map { ta_render::MapOverlay::Height } else { ta_render::MapOverlay::None });
    }

    /// `--editor-open demo`: every tool used once through the mouse path (scripted mouse
    /// actions, one a frame), for captures and to check the tools end to end.
    fn demo(&mut self, assets: &Assets) {
        use Step::*;
        let data = &assets.data;
        // Tiles from another map (they join this map's set and the terrain's atlas).
        let other = ["Sherwood", "Lava Run", "Coast To Coast"].into_iter().chain(assets.maps.iter().map(|m| m.name.as_str())).find(|n| !n.eq_ignore_ascii_case(&self.doc.origin) && assets.vfs.exists(&format!("maps/{n}.tnt")));
        if let Some(name) = other {
            self.picker.map = assets.vfs.read_opt(&format!("maps/{name}.tnt")).and_then(|b| TntMap::parse(&b).ok()).map(Arc::new);
            self.picker.source = Some(name.to_string());
        }
        let hold = |from: (f32, f32), to: (f32, f32), n: usize, ctrl: bool| -> Vec<Step> {
            let mut steps = vec![Down(from.0, from.1, ctrl)];
            steps.extend((1..=n).map(|k| {
                let t = k as f32 / n as f32;
                Drag(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t, ctrl)
            }));
            steps.push(Up);
            steps
        };
        // Trees where the planet has them, else rocks.
        for group in [FeatureGroup::Trees, FeatureGroup::Rocks] {
            self.feature_group = group;
            if let Some(&first) = self.feature_list(data).first() {
                self.feature_choice = Some(first);
                break;
            }
        }
        let (w, h) = (self.doc.width_px(), self.doc.height_px());
        let first_start = self.doc.starts.first().map_or((0.2, 0.2), |s| (s[0] / w, s[1] / h));
        (self.radius, self.strength) = (160.0, 1.0);
        let mut s = vec![Wait, Wait, Wait, Tool(self::Tool::Raise)];
        s.extend(hold((0.35, 0.45), (0.37, 0.46), 30, false));
        s.push(Tool(self::Tool::Lower));
        s.extend(hold((0.66, 0.62), (0.66, 0.62), 20, false));
        s.extend([Tool(self::Tool::Level), Level(110.0)]);
        s.extend(hold((0.62, 0.3), (0.66, 0.32), 4, false));
        s.push(Tool(self::Tool::Smooth));
        s.extend(hold((0.36, 0.45), (0.36, 0.45), 8, false));
        // A block of the map's tiles (Ctrl+drag), stamped along a line; then a block of
        // another map's.
        s.push(Tool(self::Tool::Tiles));
        s.extend(hold((0.5, 0.5), (0.54, 0.53), 3, true));
        s.extend(hold((0.12, 0.86), (0.55, 0.86), 16, false));
        if let Some(map) = &self.picker.map {
            let (tw, th) = (map.width / 2, map.height / 2);
            s.push(Import(Rect::new(tw / 3, th / 3, tw / 3 + 5, th / 3 + 5)));
            s.push(Wait);
            s.extend(hold((0.6, 0.1), (0.95, 0.1), 16, false));
        }
        // A scattered grove.
        s.push(Tool(self::Tool::Features));
        s.extend(hold((0.22, 0.62), (0.46, 0.74), 16, false));
        // Start position 1 dragged, one added.
        s.push(Tool(self::Tool::Starts));
        s.extend(hold(first_start, (0.12, 0.18), 6, false));
        s.extend(hold((0.86, 0.2), (0.86, 0.2), 1, false));
        // The hill copied and pasted, then undone and done again.
        s.push(Tool(self::Tool::Copy));
        s.extend(hold((0.3, 0.4), (0.4, 0.5), 3, false));
        s.extend(hold((0.8, 0.75), (0.8, 0.75), 1, false));
        s.extend([Undo, Wait, Redo, Wait, Tool(self::Tool::Raise)]);
        self.script = s.into();
    }

    /// The next scripted mouse action as input (positions are fractions of the map).
    fn scripted_input(&mut self, camera: &Camera) -> Option<Input> {
        let step = self.script.pop_front()?;
        let mut input = Input::default();
        let at = |fx: f32, fz: f32| {
            let (x, z) = (fx * self.doc.width_px(), fz * self.doc.height_px());
            let p = Vec3::new(x, self.doc.ground(x, z).max(self.doc.tnt.sea_level as f32), z);
            let c = camera.view_proj() * p.extend(1.0);
            let ndc = glam::Vec2::new(c.x, c.y) / c.w.max(1e-4);
            glam::Vec2::new((ndc.x + 1.0) * 0.5 * camera.viewport.x, (1.0 - ndc.y) * 0.5 * camera.viewport.y)
        };
        input.mouse = self.script_mouse;
        match step {
            Step::Tool(t) => self.tool = t,
            Step::Level(v) => self.level = v,
            Step::Undo => self.requests.undo = true,
            Step::Redo => self.requests.redo = true,
            Step::Import(rect) => self.requests.block = Some((self.picker.source.clone(), rect, None)),
            Step::Save => self.requests.save = true,
            Step::Wait => {}
            Step::Down(x, z, ctrl) | Step::Drag(x, z, ctrl) => {
                input.mouse = at(x, z);
                input.buttons_held.insert(MouseButtonId::Left);
                if matches!(step, Step::Down(..)) {
                    input.buttons_pressed.insert(MouseButtonId::Left);
                }
                if ctrl {
                    input.modifiers = winit::keyboard::ModifiersState::CONTROL;
                }
            }
            Step::Up => {
                input.buttons_released.insert(MouseButtonId::Left);
            }
        }
        self.script_mouse = input.mouse;
        Some(input)
    }
}

/// A scripted mouse action of the demo: positions are fractions of the map's width and height.
#[derive(Clone, Copy, Debug)]
enum Step {
    Tool(Tool),
    Level(f32),
    /// Press the left button there (with Ctrl or not).
    Down(f32, f32, bool),
    /// The button held, the mouse there.
    Drag(f32, f32, bool),
    Up,
    Undo,
    Redo,
    /// Tiles of the picker's map, as if chosen in the picker.
    Import(Rect),
    /// The SAVE button.
    Save,
    Wait,
}

/// The map a [`Start`] names.
fn load_doc(assets: &Assets, start: &Start) -> anyhow::Result<MapDoc> {
    match start {
        Start::Open(name) => {
            let mut doc = MapDoc::open(&assets.vfs, name)?;
            // Saved as a new map, "<name> (Edited)", unless it is one of the user's own.
            let ours = own_map(assets, name);
            if !ours {
                doc.props.name = format!("{} (Edited)", doc.props.name.trim());
            }
            Ok(doc)
        }
        Start::New(new) => {
            let palette_name = if assets.vfs.exists(&format!("maps/{}.tnt", new.palette_map)) {
                new.palette_map.clone()
            } else {
                assets.maps.first().map(|m| m.name.clone()).ok_or_else(|| anyhow::anyhow!("no maps to take tiles from"))?
            };
            let source = MapDoc::open(&assets.vfs, &palette_name)?;
            let info = source.ota.as_ref().map(ta_data::MapInfo::from_ota);
            let mut doc = MapDoc::new_blank(new, &source.tnt, info.as_ref());
            doc.origin = source.props.name.clone();
            doc.tiles_from = Some(palette_name);
            Ok(doc)
        }
    }
}

/// The file stem of the map `start` opens, when it is one of the user's own.
fn own_stem(assets: &Assets, start: &Start) -> Option<String> {
    match start {
        Start::Open(name) if own_map(assets, name) => Some(name.clone()),
        _ => None,
    }
}

/// Whether `name` is a map the editor saved (one of the user's own, saved over in place).
fn own_map(assets: &Assets, name: &str) -> bool {
    assets.vfs.origin(&format!("maps/{name}.tnt")).is_some_and(|o| o.path.starts_with(save::user_mods_dir()) && save::editor_map(&o.path, name))
}

/// The new map form's defaults: TA's Painted Desert tiles when the install has them.
pub fn default_new(assets: &Assets) -> NewMap {
    let mut n = NewMap::default();
    if !assets.maps.iter().any(|m| m.name.eq_ignore_ascii_case(&n.palette_map)) && let Some(m) = assets.maps.first() {
        n.palette_map = m.name.clone();
    }
    n
}

/// Which group of the feature list a feature belongs to (by its category and properties).
pub fn feature_group(f: &ta_data::FeatureDef) -> FeatureGroup {
    let c = f.category.to_ascii_lowercase();
    if f.is_metal_deposit() {
        FeatureGroup::Metal
    } else if f.geothermal || c.contains("vent") {
        FeatureGroup::Vents
    } else if c.contains("corpse") || c == "heaps" {
        FeatureGroup::Wrecks
    } else if ["trees", "foliage", "shrubs", "plants", "gasplants", "kelp", "anemones", "corals", "aquacorals", "weeds"].contains(&c.as_str()) {
        FeatureGroup::Trees
    } else if ["rocks", "spires", "nodes", "craters", "ruin", "glyph", "chunks", "dragonteeth"].contains(&c.as_str()) {
        FeatureGroup::Rocks
    } else {
        FeatureGroup::Other
    }
}

/// Whether a feature `world` belongs on a map of `planet` (both as the data spells them:
/// "Green Planet" maps use "greenworld" features, "Red Planet" "mars", "Lunar" "moon").
/// Features of every world ("All Worlds") fit anywhere.
pub fn world_matches(planet: &str, world: &str) -> bool {
    let norm = |s: &str| s.to_ascii_lowercase().replace("planet", "").replace("world", "").replace(' ', "");
    let (p, w) = (norm(planet), norm(world));
    if w.is_empty() || w == "all" || w == "alls" {
        return true;
    }
    let p = match p.as_str() {
        "red" => "mars".to_string(),
        "lunar" => "moon".to_string(),
        "" => "green".to_string(),
        // (TA Reborn's real places hold the green, lush, island, desert and ice worlds' features.)
        "earth" => return ["green", "lush", "archipelago", "desert", "ice"].iter().any(|k| world_matches(k, world)),
        _ => p,
    };
    p == w || p.starts_with(&w) || w.starts_with(&p)
}

#[cfg(test)]
mod interaction_tests {
    use super::*;

    fn editor() -> Editor {
        let palette = TntMap {
            width: 2, height: 2, sea_level: 0,
            tiles: vec![Box::new([1; ta_formats::tnt::TILE_SIZE * ta_formats::tnt::TILE_SIZE])],
            tile_map: vec![0], heights: vec![40; 4], features: vec![ta_formats::tnt::FEATURE_NONE; 4],
            feature_names: Vec::new(), minimap: Default::default(),
        };
        let form = NewMap { width_steps: 1, height_steps: 1, height: 40, ..Default::default() };
        let doc = MapDoc::new_blank(&form, &palette, Some(&ta_data::MapInfo::default()));
        Editor::from_document(doc, View::default(), form)
    }

    #[test]
    fn quitting_records_held_strokes_and_live_properties_before_the_discard_check() {
        let mut editor = editor();
        let before = Snapshot::of(&editor.doc);
        editor.stroke = Some(Stroke::new(Snapshot::of(&editor.doc), "Raise", Tool::Raise));
        editor.doc.tnt.heights[0] += 20;
        assert!(!editor.may_quit(), "an unreleased stroke already changed the map");
        assert_eq!(editor.dialog, Dialog::Discard(Pending::Quit));
        assert!(editor.stroke.is_none());
        assert_eq!(editor.history.next_undo(), Some("Raise"));
        editor.history.undo(&mut editor.doc).unwrap();
        assert!(before.matches(&editor.doc));
        assert!(editor.may_quit(), "undo back to the saved revision permits closing");

        editor.dialog = Dialog::None;
        editor.open_properties();
        editor.doc.props.name = "Unsaved name".into();
        editor.doc.tnt.sea_level += 1;
        assert!(!editor.may_quit(), "the still-open form is also a pending edit");
        assert_eq!(editor.history.next_undo(), Some("Properties"));
        assert!(editor.props_before.is_none());
        editor.history.undo(&mut editor.doc).unwrap();
        assert!(before.matches(&editor.doc));
        assert!(editor.may_quit());
    }

    #[test]
    fn opening_properties_finishes_the_stroke_as_a_separate_undo_step() {
        let mut editor = editor();
        let initial = Snapshot::of(&editor.doc);
        editor.stroke = Some(Stroke::new(Snapshot::of(&editor.doc), "Raise", Tool::Raise));
        editor.doc.tnt.heights[0] += 20;
        editor.open_properties();
        let terrain = Snapshot::of(&editor.doc);
        editor.doc.props.description = "New description".into();
        assert!(!editor.may_quit());
        editor.history.undo(&mut editor.doc).unwrap();
        assert!(terrain.matches(&editor.doc), "Properties must not absorb the preceding stroke");
        editor.history.undo(&mut editor.doc).unwrap();
        assert!(initial.matches(&editor.doc));
    }

    #[test]
    fn undo_requests_retire_the_held_stroke_before_changing_the_document() {
        let mut editor = editor();
        let initial = Snapshot::of(&editor.doc);
        editor.stroke = Some(Stroke::new(Snapshot::of(&editor.doc), "Raise", Tool::Raise));
        editor.doc.tnt.heights[0] += 20;
        editor.requests.undo = true;
        editor.finish_requested_interactions();
        assert!(editor.stroke.is_none(), "no stale brush can write over the undo next frame");
        editor.history.undo(&mut editor.doc).unwrap();
        assert!(initial.matches(&editor.doc));
    }

    #[test]
    fn unchanged_interactions_do_not_prompt_to_discard() {
        let mut editor = editor();
        editor.stroke = Some(Stroke::new(Snapshot::of(&editor.doc), "Raise", Tool::Raise));
        assert!(editor.may_quit());
        editor.open_properties();
        assert!(editor.may_quit());
        assert_eq!(editor.history.next_undo(), None);
    }

    #[test]
    fn discard_permission_does_not_mark_the_current_document_as_saved() {
        let mut editor = editor();
        editor.open_properties();
        editor.doc.props.description = "Keep me if the next map fails to load".into();
        editor.finish_properties();
        editor.discard_for(Pending::Open("missing-map".into()));
        assert!(editor.unsaved(), "permission to replace a map is not a successful save");
        assert_eq!(editor.loading.take(), Some(Pending::Open("missing-map".into())));
        assert!(!editor.may_quit(), "a failed replacement must still protect the retained map");
        assert_eq!(editor.doc.props.description, "Keep me if the next map fails to load");
    }

    #[test]
    fn a_bad_picker_source_reports_its_error_without_falling_back_or_repeated_reads() {
        let mut editor = editor();
        let root = std::env::temp_dir().join(format!("ta_editor_picker_error_{}", std::process::id()));
        std::fs::create_dir_all(root.join("maps")).unwrap();
        std::fs::write(root.join("maps/source.tnt"), b"broken map").unwrap();
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&root, "picker fixture").unwrap();
        editor.open_picker(Some("source".into()));
        let first = editor.picker.source_map(&vfs).unwrap_err();
        assert!(first.contains("source"));
        assert!(editor.picker.map.is_none(), "never show the current document as the requested source");
        std::fs::write(root.join("maps/source.tnt"), editor.doc.tnt.write().unwrap()).unwrap();
        assert_eq!(editor.picker.source_map(&vfs).unwrap_err(), first, "failure is cached until Retry/reopen");
        editor.open_picker(Some("source".into()));
        assert!(editor.picker.source_map(&vfs).unwrap().is_some(), "reopening retries a repaired map");
        editor.open_picker(None);
        assert!(editor.picker.source_map(&vfs).unwrap().is_none(), "THIS MAP remains an explicit selection");
        std::fs::remove_dir_all(root).unwrap();
    }
}
