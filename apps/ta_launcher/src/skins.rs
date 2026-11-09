//! Skin library work stays off the UI thread. Only one operation can touch the library at
//! a time, and closing the launcher waits for its atomic file operation to finish.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;

use image::RgbaImage;
use ta_skins::{Catalog, Entry, Manifest, Pack};

use crate::skin_editor::Editor;

#[path = "skin_preview.rs"]
pub mod preview;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter { #[default] All, Builtin, Custom }

/// How the library shows its skins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View { #[default] Grid, List }

/// The units the details' preview shows a skin on, an Arm one and a Core one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Subjects { #[default] Tanks, Kbots, Commanders, Aircraft, Buildings }

impl Subjects {
    pub const ALL: [Subjects; 5] = [Self::Tanks, Self::Kbots, Self::Commanders, Self::Aircraft, Self::Buildings];

    /// Their caption, to translate.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tanks => n("TANKS"),
            Self::Kbots => n("KBOTS"),
            Self::Commanders => n("COMMANDERS"),
            Self::Aircraft => n("AIRCRAFT"),
            Self::Buildings => n("BUILDINGS"),
        }
    }

    /// The Arm unit and the Core one (TA's own, in every install).
    pub fn units(self) -> [String; 2] {
        let [arm, core] = match self {
            Self::Tanks => ["ARMSTUMP", "CORRAID"],
            Self::Kbots => ["ARMPW", "CORAK"],
            Self::Commanders => ["ARMCOM", "CORCOM"],
            Self::Aircraft => ["ARMTHUND", "CORSHAD"],
            Self::Buildings => ["ARMSOLAR", "CORSOLAR"],
        };
        [arm.to_string(), core.to_string()]
    }
}

#[derive(Default)]
pub struct Browser {
    pub query: String,
    pub filter: Filter,
    pub view: View,
    /// The skin whose details show.
    pub inspecting: String,
    /// On a narrow page the details stand in the library's place.
    pub details_open: bool,
    /// The card with the keyboard's focus (egui moves it between frames).
    pub focused: Option<String>,
    /// A card to bring into view.
    pub reveal: Option<String>,
    pub preview: preview::Preview,
    /// The units the details' preview shows.
    pub subjects: Subjects,
}

/// Search metadata only; the virtualized list clones no textures or offscreen rows.
pub fn matches(entry: &Entry, filter: Filter, query: &str) -> bool {
    if matches!(filter, Filter::Builtin) && !entry.builtin || matches!(filter, Filter::Custom) && entry.builtin { return false; }
    let needle = query.trim().to_lowercase();
    needle.is_empty() || [&entry.id, &entry.name, &entry.author, &entry.description].iter().any(|text| text.to_lowercase().contains(&needle))
}
use ta_ui::i18n::{n, t, tf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side { Friendly, Enemy }

pub enum Operation {
    Scan([String; 2]),
    Select(String),
    SelectEnemy(String),
    Import(PathBuf),
    Export { id: String, to: PathBuf },
    /// A skin read whole for the editor.
    Open(String),
    /// The editor's skin installed as a new one, under an id free for its name.
    Add { pack: Pack, reference: Reference },
    /// The editor's changes to an installed skin saved.
    Update { manifest: Manifest, added: BTreeMap<String, RgbaImage> },
    /// A picture read (and fitted) for a texture slot.
    ReadImage { path: PathBuf, scope: Option<String>, texture: String },
}

/// Installation metadata is copied to the worker. The generated reference contains names,
/// never the player's game art.
#[derive(Clone)]
pub struct Reference {
    pub game_dir: Option<PathBuf>,
    pub enabled: Vec<String>,
    pub available: Vec<ta_vfs::mods::ModInfo>,
}

impl Operation {
    fn label(&self) -> &'static str {
        match self {
            Self::Scan(_) => n("Looking for skins…"),
            Self::Select(_) | Self::SelectEnemy(_) => n("Checking skin before use…"),
            Self::Import(_) => n("Checking and importing skin…"),
            Self::Export { .. } => n("Checking and exporting skin…"),
            Self::Open(_) => n("Opening the skin…"),
            Self::Add { .. } | Self::Update { .. } => n("Saving the skin…"),
            Self::ReadImage { .. } => n("Reading the picture…"),
        }
    }
}

enum Completed {
    Scanned(Catalog),
    Selected(Side, String),
    Imported(Entry, Catalog),
    Exported(PathBuf),
    Opened(Pack),
    Saved { entry: Box<Entry>, manifest: Box<Manifest>, catalog: Catalog, warning: Option<String> },
    Image { scope: Option<String>, texture: String, image: RgbaImage, fitted: bool },
}

struct Job {
    label: &'static str,
    selecting: bool,
    editing: bool,
    receive: Receiver<Result<Completed, String>>,
    worker: JoinHandle<()>,
}

pub struct Feedback {
    pub message: String,
    pub error: bool,
    pub warning: Option<String>,
    /// Successful exports and saved folders remain a click away after the toast fades.
    pub path: Option<PathBuf>,
}

/// A question the Skins page is asking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Remove this skin (to the Recycle Bin)?
    Delete(String),
    /// Leave the editor with changes not saved?
    Leave,
    /// Save the editor's skin as a new one, under this name.
    SaveAs(String),
}

pub struct Library {
    pub catalog: Catalog,
    pub feedback: Option<Feedback>,
    pub browser: Browser,
    pub preview_revision: u64,
    /// The skin being edited, when the editor is open.
    pub editor: Option<Editor>,
    pub ask: Option<Ask>,
    /// The editor's pattern tiles, drawn for the livery they show.
    pub editor_tiles: Option<(String, Vec<egui::TextureHandle>)>,
    /// Pictures assigned to texture slots, small, by the editor's opening and image name.
    pub slot_pictures: std::collections::HashMap<(u64, String), egui::TextureHandle>,
    selected: Option<(Side, String)>,
    job: Option<Job>,
}

impl Library {
    pub fn new(root: PathBuf, selected: [String; 2]) -> Self {
        let mut library = Self {
            catalog: Catalog { entries: Vec::new(), errors: Vec::new() },
            feedback: None,
            browser: Browser { inspecting: selected[0].clone(), ..Default::default() },
            preview_revision: 0,
            editor: None,
            ask: None,
            editor_tiles: None,
            slot_pictures: Default::default(),
            selected: None,
            job: None,
        };
        library.start(root, Operation::Scan(selected));
        library
    }

    /// Rescanning mods or skins invalidates source data and custom PNGs, even when IDs stay unchanged.
    pub fn invalidate_preview(&mut self) {
        self.preview_revision = self.preview_revision.wrapping_add(1);
    }

    pub fn preview_ready(&self) -> bool { self.browser.preview.ready() }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Launching before an assignment is validated would silently use the old skin.
    /// Scans, imports and exports do not change assignments and need not delay Play.
    pub fn selection_pending(&self) -> bool {
        self.selected.is_some() || self.job.as_ref().is_some_and(|job| job.selecting)
    }

    /// Operations whose completion belongs to the current editor. A passive
    /// scan or selection can safely finish after a new editor is opened.
    pub fn editing_pending(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.editing)
    }

    pub fn status(&self) -> Option<&'static str> {
        self.job.as_ref().map(|job| job.label)
    }

    pub fn start(&mut self, root: PathBuf, operation: Operation) {
        if self.busy() {
            self.feedback = Some(Feedback { message: t("Wait for the current skin operation to finish, then try again.").into(), error: true, warning: None, path: None });
            return;
        }
        let label = operation.label();
        let selecting = matches!(&operation, Operation::Select(_) | Operation::SelectEnemy(_));
        let editing = matches!(&operation, Operation::Open(_) | Operation::Add { .. } | Operation::Update { .. } | Operation::ReadImage { .. });
        let quiet = matches!(&operation, Operation::Scan(_) | Operation::ReadImage { .. } | Operation::Open(_));
        let (send, receive) = mpsc::channel();
        match std::thread::Builder::new().name("skin library".into()).spawn(move || {
            let result = run(&root, operation);
            let _ = send.send(result);
        }) {
            Ok(worker) => {
                if !quiet { self.feedback = None; }
                self.job = Some(Job { label, selecting, editing, receive, worker });
            }
            Err(error) => self.feedback = Some(Feedback { message: tf("Could not start skin operation: {error}", &[("error", &error)]), error: true, warning: None, path: None }),
        }
    }

    /// Returns true once an operation completed. Scans retain earlier feedback so Refresh
    /// does not silently dismiss a failed import or the location of a successful export.
    pub fn poll(&mut self) -> bool {
        let Some(job) = &self.job else { return false };
        let result = match job.receive.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err(t("The skin operation stopped unexpectedly. Please try again.").into()),
        };
        if let Some(job) = self.job.take() {
            let _ = job.worker.join();
        }
        self.complete(result);
        true
    }

    fn complete(&mut self, result: Result<Completed, String>) {
        self.feedback = match result {
            Ok(Completed::Scanned(catalog)) => {
                self.invalidate_preview();
                self.catalog = catalog;
                return;
            }
            Ok(Completed::Selected(side, id)) => {
                self.selected = Some((side, id));
                Some(Feedback { message: t("Skin selected for the next game or benchmark launch").into(), error: false, warning: None, path: None })
            }
            Ok(Completed::Imported(entry, catalog)) => {
                self.invalidate_preview();
                self.browser.inspecting = entry.id.clone();
                self.browser.reveal = Some(entry.id.clone());
                self.catalog = catalog;
                Some(Feedback { message: tf("{name} is imported. Choose a side to apply it next time you launch.", &[("name", &entry.name)]), error: false, warning: None, path: entry.path })
            }
            Ok(Completed::Exported(path)) => Some(Feedback { message: t("Skin exported. Share the .taskin file with other players.").into(), error: false, warning: None, path: Some(path) }),
            Ok(Completed::Opened(pack)) => {
                let id = pack.manifest.id.clone();
                self.editor = Some(Editor::open(pack, Some(id)));
                return;
            }
            Ok(Completed::Saved { entry, manifest, catalog, warning }) => {
                self.invalidate_preview();
                self.browser.inspecting = entry.id.clone();
                self.browser.reveal = Some(entry.id.clone());
                self.catalog = catalog;
                if let Some(editor) = &mut self.editor { editor.saved(&entry.id, &manifest); }
                Some(Feedback { message: tf("{name} is saved.", &[("name", &entry.name)]), error: false, warning, path: entry.path })
            }
            Ok(Completed::Image { scope, texture, image, fitted }) => {
                let Some(editor) = &mut self.editor else { return };
                editor.assign(scope.as_deref(), &texture, image, 0.0);
                if !fitted { return; }
                Some(Feedback { message: t("The picture was made square, sized to a power of two and opaque to fit a skin.").into(), error: false, warning: None, path: None })
            }
            Err(error) => Some(Feedback { message: error, error: true, warning: None, path: None }),
        };
    }

    pub fn shutdown(&mut self) {
        if let Some(job) = self.job.take() {
            let _ = job.worker.join();
            // Selection validation can finish after the window's final frame. Preserve its
            // result so on_exit can save the chosen skin before flushing the profile.
            let result = job.receive.try_recv().unwrap_or_else(|_| Err(t("The skin operation stopped unexpectedly. Please try again.").into()));
            self.complete(result);
        }
    }

    pub fn take_selected(&mut self) -> Option<(Side, String)> {
        self.selected.take()
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(root: &std::path::Path, operation: Operation) -> Result<Completed, String> {
    match operation {
        Operation::Scan(selected) => {
            let mut catalog = ta_skins::list(root);
            // A previously selected pack may have been edited outside the launcher. Check
            // its images as well, so corrupt paint is not presented as an available choice.
            for id in &selected {
                if catalog.entries.iter().any(|entry| entry.id == *id && !entry.builtin)
                    && let Err(error) = ta_skins::load(id, root)
                {
                    catalog.entries.retain(|entry| entry.id != *id);
                    catalog.errors.push(tf("Selected skin could not be loaded. The game will use Reborn: {error}", &[("error", &error)]));
                }
            }
            Ok(Completed::Scanned(catalog))
        }
        Operation::Select(id) => {
            ta_skins::load(&id, root).map_err(|error| error.to_string())?;
            Ok(Completed::Selected(Side::Friendly, id))
        }
        Operation::SelectEnemy(id) => {
            ta_skins::load(&id, root).map_err(|error| error.to_string())?;
            Ok(Completed::Selected(Side::Enemy, id))
        }
        Operation::Import(file) => {
            let entry = ta_skins::import_file(&file, root).map_err(|error| error.to_string())?;
            Ok(Completed::Imported(entry, ta_skins::list(root)))
        }
        Operation::Export { id, to } => {
            ta_skins::export_replacing(&id, root, &to).map_err(|error| error.to_string())?;
            Ok(Completed::Exported(to))
        }
        Operation::Open(id) => Ok(Completed::Opened(ta_skins::load(&id, root).map_err(|error| error.to_string())?)),
        Operation::Add { mut pack, reference } => {
            pack.manifest.id = ta_skins::free_id(root, &pack.manifest.name);
            let entry = ta_skins::add(&pack, root).map_err(|error| error.to_string())?;
            let warning = entry.path.as_deref().and_then(|folder| write_reference(&reference, folder).err());
            Ok(Completed::Saved { entry: Box::new(entry), manifest: Box::new(pack.manifest), catalog: ta_skins::list(root), warning })
        }
        Operation::Update { manifest, added } => {
            let entry = ta_skins::update(root, &manifest, &added).map_err(|error| error.to_string())?;
            Ok(Completed::Saved { entry: Box::new(entry), manifest: Box::new(manifest), catalog: ta_skins::list(root), warning: None })
        }
        Operation::ReadImage { path, scope, texture } => {
            let picture = image::ImageReader::open(&path).and_then(|reader| reader.with_guessed_format())
                .map_err(|error| error.to_string())
                .and_then(|mut reader| {
                    let mut limits = image::Limits::default();
                    limits.max_image_width = Some(8192);
                    limits.max_image_height = Some(8192);
                    limits.max_alloc = Some(256 * 1024 * 1024);
                    reader.limits(limits);
                    reader.decode().map_err(|error| error.to_string())
                })
                .map_err(|error| tf("Could not read {file}: {error}", &[("file", &path.display()), ("error", &error)]))?;
            let (image, fitted) = ta_skins::fit_image(&picture);
            Ok(Completed::Image { scope, texture, image, fitted })
        }
    }
}

/// The reference of the unit textures a new skin's folder starts with (names only).
fn write_reference(reference: &Reference, folder: &std::path::Path) -> Result<(), String> {
    use std::io::Write;
    let Some(game_dir) = &reference.game_dir else { return Ok(()) };
    let result = || -> Result<(), String> {
        let mounted = ta_vfs::mods::mount_enabled(Some(game_dir), ta_vfs::builtin_dir().as_deref(), &reference.enabled, &reference.available);
        mounted.install.map_err(|error| error.to_string())?;
        mounted.mods.map_err(|error| error.to_string())?;
        let text = ta_data::skin_catalogue(&mounted.vfs);
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(folder.join("UNIT-TEXTURES.toml")).map_err(|error| error.to_string())?;
        file.write_all(text.as_bytes()).map_err(|error| error.to_string())
    };
    result().map_err(|error| tf("The skin was saved, but its unit texture reference could not be written: {error}", &[("error", &error)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Each choice of the preview's units is an Arm unit and a Core one of TA's own, every pair
    /// its own.
    #[test]
    fn the_previews_units_are_an_arm_and_a_core_pair_each() {
        let pairs: Vec<[String; 2]> = Subjects::ALL.iter().map(|s| s.units()).collect();
        for [arm, core] in &pairs {
            assert!(arm.starts_with("ARM") && core.starts_with("COR"), "{arm} {core}");
        }
        assert_eq!(pairs[0], ["ARMSTUMP", "CORRAID"], "tanks first, as on the cards");
        assert!(pairs.iter().enumerate().all(|(i, a)| pairs[i + 1..].iter().all(|b| a != b)));
        assert_eq!(Subjects::default(), Subjects::Tanks);
    }

    fn finish(library: &mut Library) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while library.busy() {
            library.poll();
            assert!(Instant::now() < deadline, "skin worker did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn skin_search_filters_names_authors_and_metadata_without_changing_selection() {
        let entry = Entry { id: "night-armour".into(), name: "Night Armour".into(), author: "Jared".into(),
            description: "Winter camouflage".into(), builtin: false, path: None, livery: None, texture_count: 1, unit_count: 0 };
        for word in ["night", "JARED", "  winter  ", ""] { assert!(matches(&entry, Filter::All, word)); }
        assert!(matches(&entry, Filter::Custom, "armour"));
        assert!(!matches(&entry, Filter::Builtin, ""));
        assert!(!matches(&entry, Filter::All, "missing"));
    }

    #[test]
    fn closing_the_launcher_keeps_a_validated_pending_selection() {
        let root = std::env::temp_dir().join(format!("ta_launcher_skin_close_{}", std::process::id()));
        let mut library = Library::new(root.clone(), ["reborn".into(), "reborn".into()]);
        library.shutdown();
        library.start(root.clone(), Operation::Select("christmas".into()));
        // No poll/frame occurs between the user's action and the OS closing the window.
        library.shutdown();
        assert_eq!(library.take_selected(), Some((Side::Friendly, "christmas".into())));
        library.start(root, Operation::SelectEnemy("builtin:navy".into()));
        library.shutdown();
        assert_eq!(library.take_selected(), Some((Side::Enemy, "builtin:navy".into())));
        assert!(!library.busy());
    }

    /// A skin made in the editor: saved as new (under an id free for its name), edited and
    /// saved again in place, exported and imported elsewhere; a selected skin whose files
    /// break is never chosen.
    #[test]
    fn edited_skins_save_as_new_then_in_place_and_failures_keep_files() {
        let dir = std::env::temp_dir().join(format!("ta_launcher_skin_worker_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source");
        let mut library = Library::new(source.clone(), ["reborn".into(), "reborn".into()]);
        finish(&mut library);
        assert_eq!(library.catalog.entries.len(), ta_skins::builtin_ids().count());
        // (A built-in customised: a new skin until saved.)
        let mut pack = ta_skins::load("builtin:desert", &source).unwrap();
        pack.manifest.name = "Winter Armour".into();
        pack.manifest.author = "Test artist".into();
        library.editor = Some(Editor::open(pack, None));
        let editor = library.editor.as_mut().unwrap();
        editor.assign(None, crate::skin_editor::EVERY, RgbaImage::from_pixel(64, 64, image::Rgba([200, 200, 210, 255])), 0.0);
        let pack = editor.pack();
        let reference = Reference { game_dir: None, enabled: Vec::new(), available: Vec::new() };
        library.start(source.clone(), Operation::Add { pack, reference });
        finish(&mut library);
        assert!(!library.feedback.as_ref().unwrap().error, "{}", library.feedback.as_ref().unwrap().message);
        assert!(library.take_selected().is_none(), "saving a skin is not selecting it");
        let editor = library.editor.as_mut().unwrap();
        assert_eq!(editor.id.as_deref(), Some("winter-armour"));
        assert!(!editor.dirty());
        assert_eq!(library.browser.inspecting, "winter-armour");
        // Edited again: saved in place.
        let editor = library.editor.as_mut().unwrap();
        editor.livery("wear", 0.0, |livery| livery.wear = 1.0);
        let (manifest, added) = (editor.manifest.clone(), editor.added());
        assert!(added.is_empty(), "the image is in the folder already");
        library.start(source.clone(), Operation::Update { manifest, added });
        finish(&mut library);
        assert!(!library.feedback.as_ref().unwrap().error, "{}", library.feedback.as_ref().unwrap().message);
        assert_eq!(ta_skins::load("winter-armour", &source).unwrap().manifest.livery.unwrap().wear, 1.0);
        // Exported once; again to the same file (the save dialog asked to replace it): replaced
        // whole.
        let package = dir.join("winter.taskin");
        library.start(source.clone(), Operation::Export { id: "winter-armour".into(), to: package.clone() });
        finish(&mut library);
        assert!(!library.feedback.as_ref().unwrap().error);
        let original = std::fs::read(&package).unwrap();
        library.start(source.clone(), Operation::Export { id: "winter-armour".into(), to: package.clone() });
        finish(&mut library);
        assert!(!library.feedback.as_ref().unwrap().error, "{}", library.feedback.as_ref().unwrap().message);
        assert_eq!(std::fs::read(&package).unwrap(), original);
        // Opened for editing: whole, as saved.
        library.editor = None;
        library.start(source.clone(), Operation::Open("winter-armour".into()));
        finish(&mut library);
        let editor = library.editor.as_ref().expect("opened in the editor");
        assert_eq!(editor.id.as_deref(), Some("winter-armour"));
        assert!(!editor.dirty());

        let installed = dir.join("installed");
        library.start(installed.clone(), Operation::Import(package));
        finish(&mut library);
        assert!(!library.feedback.as_ref().unwrap().error);
        assert!(library.take_selected().is_none(), "importing a skin is not selecting it");
        let image = installed.join("winter-armour/textures/armour.png");
        let pixels = std::fs::read(&image).unwrap();
        std::fs::write(&image, b"invalid PNG").unwrap();
        library.start(installed.clone(), Operation::Scan(["reborn".into(), "winter-armour".into()]));
        finish(&mut library);
        assert!(!library.catalog.errors.is_empty());
        assert!(!library.catalog.entries.iter().any(|entry| entry.id == "winter-armour"));
        library.start(installed.clone(), Operation::Select("winter-armour".into()));
        finish(&mut library);
        assert!(library.feedback.as_ref().unwrap().error);
        assert!(library.take_selected().is_none(), "invalid textures must never become selected");
        std::fs::write(image, pixels).unwrap();
        library.start(installed.clone(), Operation::Select("winter-armour".into()));
        finish(&mut library);
        assert_eq!(library.take_selected(), Some((Side::Friendly, "winter-armour".into())));
        library.start(installed, Operation::SelectEnemy("winter-armour".into()));
        finish(&mut library);
        assert_eq!(library.take_selected(), Some((Side::Enemy, "winter-armour".into())));
        drop(library);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_picture_for_a_slot_is_read_fitted_and_assigned() {
        let dir = std::env::temp_dir().join(format!("ta_launcher_skin_picture_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("photo.png");
        image::RgbaImage::from_pixel(300, 180, image::Rgba([10, 120, 40, 255])).save(&file).unwrap();
        let mut library = Library::new(dir.join("skins"), ["reborn".into(), "reborn".into()]);
        finish(&mut library);
        library.editor = Some(Editor::open(ta_skins::load("builtin:navy", &dir).unwrap(), None));
        library.start(dir.join("skins"), Operation::ReadImage { path: file, scope: Some("armcom".into()), texture: "ArmMetal1".into() });
        finish(&mut library);
        let editor = library.editor.as_ref().unwrap();
        let path = editor.mapping(Some("armcom"), "armmetal1").expect("assigned");
        assert_eq!(editor.image(path).unwrap().dimensions(), (256, 256));
        assert!(library.feedback.as_ref().is_some_and(|f| !f.error), "the player is told it was fitted");
        library.start(dir.join("skins"), Operation::ReadImage { path: dir.join("missing.png"), scope: None, texture: "*".into() });
        finish(&mut library);
        assert!(library.feedback.as_ref().unwrap().error);
        drop(library);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
