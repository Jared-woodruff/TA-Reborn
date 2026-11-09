//! The player's picture, beside their lines on the chat and in the players panel: an image
//! of their choosing, its middle square kept 256 pixels across as `portrait.png` in the
//! settings folder. (The computer players show their side's Commander.)

use std::path::{Path, PathBuf};

use egui::{Context, Id, TextureHandle};

/// The picture's size, square.
pub const SIZE: u32 = 256;

pub fn path() -> PathBuf {
    ta_config::player_data_dir().join("portrait.png")
}

/// Read and normalize a bounded PNG/JPEG, ready for a profile store.
pub fn read_from(file: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(file).map_err(|e| e.to_string())?
        .take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    ta_config::profiles::normalize_avatar(&bytes).map_err(|e| e.to_string())
}

/// Takes `file` (PNG or JPEG) as the current player's picture.
pub fn set_from(file: &Path) -> Result<(), String> {
    let bytes = read_from(file)?;
    if let Some(id) = ta_config::profiles::current_profile_id() {
        ta_config::profiles::ProfileStore::open(ta_config::config_dir())
            .and_then(|store| store.set_avatar(&id, &bytes)).map_err(|e| e.to_string())?;
    } else {
        let target = path();
        if let Some(dir) = target.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
        std::fs::write(target, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// No picture of one's own any more.
pub fn clear() {
    let result = if let Some(id) = ta_config::profiles::current_profile_id() {
        ta_config::profiles::ProfileStore::open(ta_config::config_dir())
            .and_then(|store| store.remove_avatar(&id)).map(|_| ())
    } else {
        match std::fs::remove_file(path()) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    };
    if let Err(e) = result { log::warn!("could not remove player picture: {e}"); }
}

/// The picture as a texture, or none; read again when the file changes (looked at once a
/// second).
pub fn texture(ctx: &Context) -> Option<TextureHandle> {
    if crate::profiles::current(ctx).is_some_and(|profile| !profile.has_avatar) { return None; }
    texture_at(ctx, &path())
}

/// Cache identity includes the profile path, so switching users never reuses a portrait.
pub fn texture_at(ctx: &Context, path: &Path) -> Option<TextureHandle> {
    let id = Id::new(("ta_ui::portrait", path));
    let now = ctx.input(|i| i.time);
    type Cached = (f64, Option<std::time::SystemTime>, Option<TextureHandle>);
    let cached: Option<Cached> = ctx.data(|d| d.get_temp(id));
    if let Some((checked, _, tex)) = &cached
        && now - checked < 1.0
    {
        return tex.clone();
    }
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let tex = match &cached {
        Some((_, was, tex)) if *was == modified => tex.clone(),
        _ => modified.and_then(|_| {
            let bytes = read_from(path).ok()?;
            let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
            let (w, h) = img.dimensions();
            let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
            Some(ctx.load_texture("portrait", color, egui::TextureOptions::LINEAR))
        }),
    };
    ctx.data_mut(|d| d.insert_temp(id, (now, modified, tex.clone())));
    tex
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_with_same_timestamp_cannot_share_cached_portraits() {
        let root = std::env::temp_dir().join(format!("ta-portrait-{}", ta_config::profiles::new_id()));
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("first.png");
        let second = root.join("second.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([220, 20, 20, 255])).save(&first).unwrap();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([20, 20, 220, 255])).save(&second).unwrap();
        let time = std::fs::metadata(&first).unwrap().modified().unwrap();
        std::fs::File::options().write(true).open(&second).unwrap().set_modified(time).unwrap();
        let ctx = Context::default();
        let a = texture_at(&ctx, &first).unwrap();
        let b = texture_at(&ctx, &second).unwrap();
        assert_ne!(a.id(), b.id(), "a user switch must not reuse the previous avatar");
        assert_eq!(texture_at(&ctx, &first).unwrap().id(), a.id());
        assert_eq!(a.size(), [256, 256]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
