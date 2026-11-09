//! The skin editor's state: the skin being made (its manifest, and every image it may use), what
//! was last saved, undo and redo, and the change each control makes. `pages/skin_editor.rs`
//! draws it; the preview paints it as it changes (`skin_preview.rs`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use image::RgbaImage;
use ta_skins::{Colour, Livery, Manifest, Pack, Pattern, UnitSkin};

/// A texture slot for every armour surface the skin may repaint.
pub const EVERY: &str = "*";
/// A texture slot for armour painted in a plain colour (no texture).
pub const PLAIN: &str = "_untextured";
/// Changes to one control this close together are one step to undo (a slider dragged).
const MERGE_SECONDS: f64 = 1.5;
/// How many steps can be undone.
const UNDO_DEPTH: usize = 200;

pub struct Editor {
    /// The installed skin it edits; none for one not saved yet.
    pub id: Option<String>,
    pub manifest: Manifest,
    /// Every image the skin may use: those it came with and those assigned since (an undo can
    /// map one again).
    images: BTreeMap<String, RgbaImage>,
    /// The images already in its folder.
    on_disk: BTreeSet<String>,
    /// The manifest as last saved (or as opened).
    saved: Manifest,
    undo: Vec<Manifest>,
    redo: Vec<Manifest>,
    /// The control changed last, and when.
    last: Option<(&'static str, f64)>,
    /// A livery switched off, to switch back on.
    parked: Option<Livery>,
    /// Bumped whenever the mappings or the images change: the preview's models are made again.
    pub textures: u64,
    /// The preview's units, Arm's and Core's (empty: the usual examples).
    pub units: [String; 2],
    /// The unit texture slots map for, or none for every unit.
    pub scope: Option<String>,
    preview: Option<(u64, Arc<Pack>)>,
    /// Which opening of the editor this is (pictures made of its images are its own: another
    /// skin's may have the same names).
    pub serial: u64,
}

/// A new skin's first livery: plain olive drab, as Reborn wears it.
pub fn starting_livery() -> Livery {
    Livery::new(Pattern::Solid, &[Colour::rgb(0x6b705c)])
}

impl Editor {
    /// Edits `pack`: the installed skin `id`, or (none) a new one to be saved as new.
    pub fn open(pack: Pack, id: Option<String>) -> Self {
        static OPENED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let on_disk = if id.is_some() { pack.images.keys().cloned().collect() } else { BTreeSet::new() };
        Self {
            id,
            saved: pack.manifest.clone(),
            manifest: pack.manifest,
            images: pack.images,
            on_disk,
            undo: Vec::new(),
            redo: Vec::new(),
            last: None,
            parked: None,
            textures: 0,
            units: [String::new(), String::new()],
            scope: None,
            preview: None,
            serial: OPENED.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Whether there is anything to save: changes since it was saved, or a skin never saved.
    pub fn dirty(&self) -> bool {
        self.id.is_none() || self.manifest != self.saved
    }

    /// Changes made since it was opened or last saved (what closing would lose).
    pub fn changed(&self) -> bool {
        self.manifest != self.saved
    }

    pub fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }

    /// Changes the manifest by `change`, for the control `control` at `now` (seconds): one
    /// step to undo, unless the same control changed a moment ago (a slider being dragged).
    pub fn change(&mut self, control: &'static str, now: f64, change: impl FnOnce(&mut Manifest)) {
        let before = self.manifest.clone();
        change(&mut self.manifest);
        if self.manifest == before { return; }
        let merge = self.last.is_some_and(|(last, at)| last == control && now - at < MERGE_SECONDS);
        if !merge {
            self.undo.push(before.clone());
            if self.undo.len() > UNDO_DEPTH { self.undo.remove(0); }
        }
        self.redo.clear();
        self.last = Some((control, now));
        self.after(&before);
    }

    /// Changes the livery, if it has one.
    pub fn livery(&mut self, control: &'static str, now: f64, change: impl FnOnce(&mut Livery)) {
        self.change(control, now, |manifest| if let Some(livery) = manifest.livery.as_mut() { change(livery) });
    }

    pub fn undo(&mut self) {
        let Some(previous) = self.undo.pop() else { return };
        let before = std::mem::replace(&mut self.manifest, previous);
        self.redo.push(before.clone());
        self.last = None;
        self.after(&before);
    }

    pub fn redo(&mut self) {
        let Some(next) = self.redo.pop() else { return };
        let before = std::mem::replace(&mut self.manifest, next);
        self.undo.push(before.clone());
        self.last = None;
        self.after(&before);
    }

    /// Back to the skin as last saved (itself a step to undo).
    pub fn revert(&mut self, now: f64) {
        let saved = self.saved.clone();
        self.last = None;
        self.change("revert", now, |manifest| *manifest = saved);
        self.last = None;
    }

    /// Paints a livery, or not (the one switched off kept, to come back).
    pub fn set_livery_on(&mut self, on: bool, now: f64) {
        if on == self.manifest.livery.is_some() { return; }
        let parked = self.parked.take();
        let mut taken = None;
        self.change("livery", now, |manifest| {
            if on { manifest.livery = Some(parked.unwrap_or_else(starting_livery)); } else { taken = manifest.livery.take(); }
        });
        self.parked = taken;
    }

    /// The image path a slot maps for `scope` (one unit's, or every unit's), exactly there.
    pub fn mapping(&self, scope: Option<&str>, texture: &str) -> Option<&str> {
        let texture = texture.to_ascii_lowercase();
        match scope {
            None => self.manifest.textures.get(&texture),
            Some(unit) => self.manifest.units.get(&unit.to_ascii_lowercase()).and_then(|skin| skin.textures.get(&texture)),
        }.map(String::as_str)
    }

    /// Maps `texture` (for `scope`) to `image`, kept in the skin under a name of its own;
    /// returns that name.
    pub fn assign(&mut self, scope: Option<&str>, texture: &str, image: RgbaImage, now: f64) -> String {
        let stem = match texture {
            EVERY => "armour".to_string(),
            PLAIN => "plain".to_string(),
            name => name.to_ascii_lowercase(),
        };
        let stem: String = scope.map_or(stem.clone(), |unit| format!("{}-{stem}", unit.to_ascii_lowercase()))
            .chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
        let path = (1..).map(|n| if n == 1 { format!("textures/{stem}.png") } else { format!("textures/{stem}-{n}.png") })
            .find(|path| !self.images.keys().chain(&self.on_disk).any(|taken| taken.eq_ignore_ascii_case(path)))
            .expect("some name is free");
        self.images.insert(path.clone(), image);
        let (key, value) = (texture.to_ascii_lowercase(), path.clone());
        let unit = scope.map(str::to_ascii_lowercase);
        self.last = None;
        self.change("assign", now, |manifest| match unit {
            None => { manifest.textures.insert(key, value); }
            Some(unit) => { manifest.units.entry(unit).or_insert_with(UnitSkin::default).textures.insert(key, value); }
        });
        self.last = None;
        path
    }

    /// Takes `texture`'s image away (for `scope`): the livery, or the unit's own paint, shows there again.
    pub fn clear(&mut self, scope: Option<&str>, texture: &str, now: f64) {
        let key = texture.to_ascii_lowercase();
        let unit = scope.map(str::to_ascii_lowercase);
        self.last = None;
        self.change("clear", now, |manifest| match unit {
            None => { manifest.textures.remove(&key); }
            Some(unit) => {
                if let Some(skin) = manifest.units.get_mut(&unit) { skin.textures.remove(&key); }
                if manifest.units.get(&unit).is_some_and(|skin| skin.textures.is_empty()) { manifest.units.remove(&unit); }
            }
        });
        self.last = None;
    }

    /// An image the skin may use, by name.
    pub fn image(&self, path: &str) -> Option<&RgbaImage> {
        self.images.get(path)
    }

    /// The skin as it stands: its manifest and the images it maps.
    pub fn pack(&self) -> Pack {
        let names = self.manifest.image_names();
        Pack { manifest: self.manifest.clone(), images: self.images.iter().filter(|(name, _)| names.contains(name.as_str())).map(|(n, i)| (n.clone(), i.clone())).collect() }
    }

    /// The skin for the preview, made again only when its mappings or images changed.
    pub fn preview_pack(&mut self) -> Arc<Pack> {
        if let Some((revision, pack)) = &self.preview && *revision == self.textures {
            return pack.clone();
        }
        let pack = Arc::new(self.pack());
        self.preview = Some((self.textures, pack.clone()));
        pack
    }

    /// The images it maps that its folder does not have yet (what saving it writes).
    pub fn added(&self) -> BTreeMap<String, RgbaImage> {
        let names = self.manifest.image_names();
        self.images.iter().filter(|(name, _)| names.contains(name.as_str()) && !self.on_disk.contains(*name)).map(|(n, i)| (n.clone(), i.clone())).collect()
    }

    /// It was saved as the installed skin `id` (with the manifest `manifest`, its id and
    /// name as saved).
    pub fn saved(&mut self, id: &str, manifest: &Manifest) {
        let names = manifest.image_names();
        // (Saved as another skin, its folder has only the images mapped then: one mapped again
        // by an undo is written at the next save. Saved again, a folder keeps those it had.)
        if self.id.as_deref() != Some(id) {
            self.on_disk.clear();
        }
        self.on_disk.extend(names.iter().map(|name| name.to_string()));
        self.id = Some(id.to_string());
        self.manifest.id = id.to_string();
        self.manifest.name = manifest.name.clone();
        // The worker saved its snapshot, not edits or undo actions made while it
        // was running. Those changes must remain dirty and available to save.
        self.saved = manifest.clone();
        self.saved.id = id.to_string();
        // (The steps to undo stay, under the id it has now.)
        for step in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            step.id = id.to_string();
        }
    }

    /// The preview's models are made again after a change to the mappings.
    fn after(&mut self, before: &Manifest) {
        // (The preview's models are made again for new mappings, or for TA's own shapes in
        // place of Reborn's kit or back.)
        let original = |m: &Manifest| m.livery.as_ref().is_some_and(|l| l.original);
        if before.textures != self.manifest.textures || before.units != self.manifest.units || original(before) != original(&self.manifest) {
            self.textures += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        let pack = ta_skins::load("builtin:desert", std::path::Path::new("unused")).unwrap();
        let mut manifest = pack.manifest;
        manifest.id = "desert-copy".into();
        Editor::open(Pack { manifest, images: BTreeMap::new() }, None)
    }

    fn solid(side: u32) -> RgbaImage { RgbaImage::from_pixel(side, side, image::Rgba([90, 80, 70, 255])) }

    #[test]
    fn edits_made_while_a_save_runs_remain_unsaved_and_revert_to_the_written_snapshot() {
        let mut editor = editor();
        let mut written = editor.manifest.clone();
        written.id = "written-skin".into();
        editor.livery("wear", 0.0, |livery| livery.wear = 0.9);
        editor.saved("written-skin", &written);
        assert!(editor.dirty() && editor.changed(), "the worker did not write the later edit");
        assert_eq!(editor.manifest.livery.as_ref().unwrap().wear, 0.9);
        editor.revert(1.0);
        assert_eq!(editor.manifest, written);
        assert!(!editor.dirty());
    }

    #[test]
    fn saved_as_another_skin_a_picture_mapped_again_is_written_into_its_folder() {
        let mut editor = editor();
        editor.assign(None, EVERY, solid(64), 0.0);
        let manifest = editor.manifest.clone();
        editor.saved("first", &manifest);
        assert!(editor.added().is_empty(), "saved: the picture is in the first skin's folder");
        // Its picture taken away, then saved as another skin: that one's folder has none.
        editor.clear(None, EVERY, 5.0);
        let mut copy = editor.manifest.clone();
        copy.id = "second".into();
        editor.saved("second", &copy);
        assert!(editor.added().is_empty());
        // An undo maps the picture again: the next save writes it into the second folder.
        editor.undo();
        assert_eq!(editor.mapping(None, EVERY), Some("textures/armour.png"));
        assert_eq!(editor.added().len(), 1, "the second skin's folder does not have the picture");
        // Saved again under the same id, a folder keeps what it had.
        let again = editor.manifest.clone();
        editor.saved("second", &again);
        assert!(editor.added().is_empty());
    }

    #[test]
    fn every_opening_of_the_editor_is_told_apart() {
        assert_ne!(editor().serial, editor().serial, "pictures cached for one skin are not shown for another");
    }

    #[test]
    fn a_dragged_slider_is_one_step_to_undo_and_redo_brings_it_back() {
        let mut editor = editor();
        assert!(editor.dirty(), "a skin never saved has something to save");
        let scale = editor.manifest.livery.as_ref().unwrap().scale;
        for (i, at) in [0.0, 0.1, 0.2, 0.3].into_iter().enumerate() {
            editor.livery("scale", at, |livery| livery.scale = 1.5 + i as f32 * 0.1);
        }
        editor.livery("wear", 0.4, |livery| livery.wear = 0.9);
        assert_eq!(editor.undo.len(), 2, "the drag, then the wear");
        editor.undo();
        assert_eq!(editor.manifest.livery.as_ref().unwrap().wear, 0.6);
        editor.undo();
        assert_eq!(editor.manifest.livery.as_ref().unwrap().scale, scale);
        assert!(!editor.can_undo());
        editor.redo();
        assert!((editor.manifest.livery.as_ref().unwrap().scale - 1.8).abs() < 1e-6);
        // (After a pause the same slider is a step of its own.)
        editor.livery("scale", 10.0, |livery| livery.scale = 2.0);
        editor.livery("scale", 20.0, |livery| livery.scale = 3.0);
        editor.undo();
        assert_eq!(editor.manifest.livery.as_ref().unwrap().scale, 2.0);
        assert!(!editor.can_redo() || editor.redo.len() == 1);
    }

    #[test]
    fn switching_the_livery_off_and_on_keeps_it() {
        let mut editor = editor();
        let livery = editor.manifest.livery.clone();
        editor.set_livery_on(false, 0.0);
        assert!(editor.manifest.livery.is_none());
        editor.set_livery_on(true, 5.0);
        assert_eq!(editor.manifest.livery, livery);
        editor.undo();
        editor.undo();
        assert_eq!(editor.manifest.livery, livery);
    }

    /// Painting TA's originals in place of TA Reborn's units (or back) makes the preview's models
    /// again (no kit, or the kit); the livery's other values only paint them.
    #[test]
    fn tas_originals_make_the_preview_models_again() {
        let mut editor = editor();
        let revision = editor.textures;
        editor.livery("wear", 0.0, |livery| livery.wear = 0.9);
        editor.livery("fade", 1.0, |livery| livery.fade = 0.3);
        assert_eq!(editor.textures, revision, "paint alone is one write");
        editor.livery("original", 2.0, |livery| livery.original = true);
        assert!(editor.textures > revision, "TA's own shapes: the models made again");
        let after = editor.textures;
        editor.undo();
        assert!(editor.textures > after && !editor.manifest.livery.as_ref().unwrap().original, "and back");
    }

    #[test]
    fn texture_slots_map_for_every_unit_or_one_and_new_images_are_what_saving_writes() {
        let mut editor = editor();
        let revision = editor.textures;
        let every = editor.assign(None, EVERY, solid(64), 0.0);
        assert_eq!(every, "textures/armour.png");
        let commander = editor.assign(Some("ARMCOM"), "ArmMetal1", solid(128), 1.0);
        assert_eq!(commander, "textures/armcom-armmetal1.png");
        let again = editor.assign(None, EVERY, solid(256), 2.0);
        assert_eq!(again, "textures/armour-2.png", "a new image never takes an old one's name");
        assert!(editor.textures > revision, "the preview's models are made again");
        assert_eq!(editor.mapping(None, "*"), Some("textures/armour-2.png"));
        assert_eq!(editor.mapping(Some("armcom"), "armmetal1"), Some("textures/armcom-armmetal1.png"));
        assert_eq!(editor.manifest.texture_for("armcom", "armmetal1"), Some("textures/armcom-armmetal1.png"));
        assert_eq!(editor.manifest.texture_for("corak", "cormetal"), Some("textures/armour-2.png"));
        let pack = editor.pack();
        assert_eq!(pack.images.len(), 2, "only the images it maps");
        assert_eq!(editor.added().len(), 2);
        editor.undo();
        assert_eq!(editor.mapping(None, EVERY), Some("textures/armour.png"), "undone, the image before maps again");
        editor.clear(Some("armcom"), "ArmMetal1", 3.0);
        assert!(!editor.manifest.units.contains_key("armcom"), "an emptied unit goes");
        let mut manifest = editor.manifest.clone();
        manifest.id = "my-desert".into();
        editor.saved("my-desert", &manifest);
        assert!(!editor.dirty());
        assert!(editor.added().is_empty(), "saved images are in the folder");
        editor.assign(None, PLAIN, solid(64), 9.0);
        assert_eq!(editor.added().len(), 1);
        assert!(editor.dirty());
        editor.revert(10.0);
        assert!(!editor.dirty());
        assert_eq!(editor.mapping(None, PLAIN), None);
    }

    #[test]
    fn the_preview_gets_the_same_skin_until_its_textures_change() {
        let mut editor = editor();
        let first = editor.preview_pack();
        editor.livery("gloss", 0.0, |livery| livery.gloss = 0.9);
        assert!(Arc::ptr_eq(&first, &editor.preview_pack()), "a livery change paints live, with the same models");
        editor.assign(None, EVERY, solid(64), 1.0);
        assert!(!Arc::ptr_eq(&first, &editor.preview_pack()));
    }
}
