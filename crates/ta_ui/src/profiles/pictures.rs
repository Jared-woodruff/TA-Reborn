//! Pictures read from disk off the interface's thread (an achievement's moment): `None` while
//! one is read, its texture after. Each is read once a session, unless its file changes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui::{Context, Id, TextureHandle};

/// Pictures are brought down to this many pixels across at most.
const MOST: u32 = 1280;

#[derive(Clone)]
enum Slot {
    Reading,
    Read(Arc<egui::ColorImage>),
    Ready(TextureHandle),
    Failed,
}

type Shared = Arc<Mutex<BTreeMap<(PathBuf, Option<std::time::SystemTime>), Slot>>>;

fn shared(ctx: &Context) -> Shared {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Shared>(Id::new("ta_ui::profiles::pictures")).clone())
}

/// The picture at `path`, or `None` while it is read (or when it cannot be).
pub fn texture(ctx: &Context, path: &Path) -> Option<TextureHandle> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let key = (path.to_path_buf(), modified);
    let shared = shared(ctx);
    let mut slots = shared.lock().unwrap_or_else(|e| e.into_inner());
    match slots.get(&key).cloned() {
        Some(Slot::Ready(texture)) => Some(texture),
        Some(Slot::Read(image)) => {
            let texture = ctx.load_texture(format!("picture {}", path.display()), Arc::unwrap_or_clone(image), egui::TextureOptions::LINEAR);
            slots.insert(key, Slot::Ready(texture.clone()));
            Some(texture)
        }
        Some(Slot::Reading | Slot::Failed) => None,
        None => {
            slots.insert(key.clone(), Slot::Reading);
            let (shared, ctx) = (shared.clone(), ctx.clone());
            std::thread::spawn(move || {
                let slot = match read(&key.0) {
                    Ok(image) => Slot::Read(Arc::new(image)),
                    Err(error) => {
                        log::warn!("could not read {}: {error}", key.0.display());
                        Slot::Failed
                    }
                };
                shared.lock().unwrap_or_else(|e| e.into_inner()).insert(key, slot);
                ctx.request_repaint();
            });
            None
        }
    }
}

fn read(path: &Path) -> Result<egui::ColorImage, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(|e| e.to_string())?.take(16 * 1024 * 1024).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let mut image = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    if image.width() > MOST {
        image = image.resize(MOST, MOST, image::imageops::FilterType::Triangle);
    }
    let rgba = image.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}
