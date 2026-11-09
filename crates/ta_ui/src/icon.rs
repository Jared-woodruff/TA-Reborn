//! The emblem as a bitmap: the window icons, the executables' icon and the installer's art (see
//! `ta-tools brand`), the launcher's emblem (`widgets::paint_emblem`). The pictures are
//! `tools/branding/emblem.py`'s (branding/emblem-*.png): a hexagonal brass plate with two
//! stacked chevrons, machined, each size drawn for itself (from 32 pixels down, a cut of its own
//! with the rim and chevrons a couple of pixels thick).

/// The pictures, smallest first: each size `tools/branding/emblem.py` draws.
const PICTURES: [(u32, &[u8]); 11] = [
    (16, include_bytes!("../../../branding/emblem-16.png")),
    (20, include_bytes!("../../../branding/emblem-20.png")),
    (24, include_bytes!("../../../branding/emblem-24.png")),
    (32, include_bytes!("../../../branding/emblem-32.png")),
    (40, include_bytes!("../../../branding/emblem-40.png")),
    (48, include_bytes!("../../../branding/emblem-48.png")),
    (64, include_bytes!("../../../branding/emblem-64.png")),
    (96, include_bytes!("../../../branding/emblem-96.png")),
    (128, include_bytes!("../../../branding/emblem-128.png")),
    (256, include_bytes!("../../../branding/emblem-256.png")),
    (512, include_bytes!("../../../branding/emblem-512.png")),
];

/// The emblem, `size` × `size` pixels of straight-alpha RGBA: the picture drawn at that size, or
/// else the next larger one brought down to it (the largest for anything larger).
pub fn emblem_rgba(size: u32) -> Vec<u8> {
    let size = size.max(1);
    let (drawn, png) = PICTURES.iter().copied().find(|&(s, _)| s >= size).unwrap_or(PICTURES[PICTURES.len() - 1]);
    let picture = image::load_from_memory_with_format(png, image::ImageFormat::Png).expect("the emblem's pictures are PNGs").to_rgba8();
    if drawn == size {
        return picture.into_raw();
    }
    image::imageops::resize(&picture, size, size, image::imageops::FilterType::Lanczos3).into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_picture_is_its_size() {
        for (size, png) in PICTURES {
            let picture = image::load_from_memory(png).unwrap();
            assert_eq!((picture.width(), picture.height()), (size, size), "emblem-{size}.png");
        }
    }

    #[test]
    fn emblem_fills_the_middle_and_leaves_the_corners_clear() {
        for size in [16u32, 32, 45, 64, 108, 256] {
            let rgba = emblem_rgba(size);
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            assert_eq!(rgba[3], 0, "{size}: top-left corner is outside the hexagon");
            let centre = ((size / 2 * size + size / 2) * 4) as usize;
            assert_eq!(rgba[centre + 3], 255, "{size}: centre is opaque");
        }
    }
}
