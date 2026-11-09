//! The people's pictures in a lobby: ours sent to the host as a small PNG, everyone's shared
//! by the host, each read back strictly (exactly [`ta_net::PORTRAIT_SIZE`] square, a PNG of at
//! most [`ta_net::PORTRAIT_BYTES`]) before it becomes a texture.

use std::collections::BTreeMap;
use std::io::Cursor;

use egui::{Context, TextureHandle};

/// Our own picture for the network: the player's picture, made [`ta_net::PORTRAIT_SIZE`]
/// square; none without one.
pub fn own(ctx: &Context) -> Option<Vec<u8>> {
    if ta_ui::profiles::current(ctx).is_some_and(|profile| !profile.has_avatar) {
        return None;
    }
    let bytes = std::fs::read(ta_ui::portrait::path()).ok()?;
    let image = image::load_from_memory(&bytes).ok()?;
    let size = ta_net::PORTRAIT_SIZE;
    let small = image.resize_to_fill(size, size, image::imageops::FilterType::Lanczos3).to_rgba8();
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(small).write_to(&mut out, image::ImageFormat::Png).ok()?;
    let png = out.into_inner();
    (png.len() <= ta_net::PORTRAIT_BYTES).then_some(png)
}

/// A picture from the network, read back strictly: a PNG exactly [`ta_net::PORTRAIT_SIZE`]
/// square, decoded within that size's memory.
pub fn decode(png: &[u8]) -> Option<egui::ColorImage> {
    if png.is_empty() || png.len() > ta_net::PORTRAIT_BYTES {
        return None;
    }
    let mut reader = image::ImageReader::new(Cursor::new(png));
    reader.set_format(image::ImageFormat::Png);
    let size = ta_net::PORTRAIT_SIZE;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(size);
    limits.max_image_height = Some(size);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.to_rgba8();
    if image.dimensions() != (size, size) {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([size as usize, size as usize], image.as_raw()))
}

/// The pictures as textures, made once for each picture a person sends.
#[derive(Default)]
pub struct Textures {
    made: BTreeMap<u8, (u64, Option<TextureHandle>)>,
}

impl Textures {
    /// The picture of `peer`, from `pictures` (peer → PNG), as a texture; none without one, or
    /// with one that does not read.
    pub fn get(&mut self, ctx: &Context, pictures: &BTreeMap<u8, Vec<u8>>, peer: u8) -> Option<TextureHandle> {
        let Some(png) = pictures.get(&peer) else {
            self.made.remove(&peer);
            return None;
        };
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            png.hash(&mut h);
            h.finish()
        };
        if let Some((made, texture)) = self.made.get(&peer)
            && *made == hash
        {
            return texture.clone();
        }
        let texture = decode(png).map(|image| ctx.load_texture(format!("lobby_portrait_{peer}"), image, egui::TextureOptions::LINEAR));
        self.made.insert(peer, (hash, texture.clone()));
        texture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 120, 40, 255]));
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn only_a_small_square_png_reads_back() {
        let size = ta_net::PORTRAIT_SIZE;
        assert!(decode(&png(size, size)).is_some());
        assert!(decode(&png(size + 1, size)).is_none(), "another size");
        assert!(decode(&png(4096, 4096)).is_none(), "a large picture is refused before it is decoded");
        assert!(decode(b"not a picture").is_none());
        assert!(decode(&[]).is_none());
    }
}
