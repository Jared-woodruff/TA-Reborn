//! Local cosmetic paint: a skin's livery (`shaders/livery.wgsl`, its numbers in a small uniform
//! buffer that can change from frame to frame), and its texture mappings in a separate
//! mipmapped bank, so HQ skins do not enlarge every original texture. Unit-only resolution keeps
//! wrecks and projectiles unchanged.

use std::collections::BTreeMap;

use bytemuck::{Pod, Zeroable};

use super::{TextureRef, linear_to_srgb, srgb_to_linear_lut};
use crate::util::srgb_to_linear;

pub(super) const CUSTOM: u32 = 1 << 29;
/// A face its bank's livery paints (`livery.wgsl`): armour of one frame, not a light, marking or
/// track, and not a face the skin maps a texture to.
pub(super) const LIVERY: u32 = 1 << 30;
pub(super) const ENEMY: u32 = 1 << 31;

pub(super) struct Library {
    pub view: wgpu::TextureView,
    manifest: Option<ta_skins::Manifest>,
    images: BTreeMap<String, (u32, [u8; 3])>,
}

impl Library {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, pack: Option<&ta_skins::Pack>) -> Self {
        let limits = device.limits();
        let accepted = pack.filter(|p| {
            let side = p.images.values().map(|image| image.width()).max().unwrap_or(1);
            let layers = p.images.len() as u64 + 1;
            let fits = p.images.len() <= ta_skins::MAX_IMAGES
                && p.images.values().all(|image| image.width() == image.height() && image.width().is_power_of_two() && (64..=ta_skins::MAX_IMAGE_SIDE).contains(&image.width()))
                && side <= limits.max_texture_dimension_2d
                && layers <= u64::from(limits.max_texture_array_layers)
                && layers * u64::from(side).pow(2) <= ta_skins::MAX_BANK_TEXELS;
            if !fits {
                log::warn!("skin {} exceeds this graphics card's texture limits; using Reborn", p.manifest.id);
            }
            fits
        });
        let side = accepted.and_then(|p| p.images.values().map(|i| i.width()).max()).unwrap_or(1);
        let layers = accepted.map_or(1, |p| p.images.len() as u32 + 1);
        let mips = side.ilog2() + 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("unit skin paint"),
            size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: layers },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lut = srgb_to_linear_lut();
        let mut images = BTreeMap::new();
        if let Some(pack) = accepted {
            for (layer, (name, image)) in pack.images.iter().enumerate() {
                let linear = image::Rgb32FImage::from_fn(image.width(), image.height(), |x, y| {
                    let pixel = image.get_pixel(x, y);
                    image::Rgb([lut[pixel[0] as usize], lut[pixel[1] as usize], lut[pixel[2] as usize]])
                });
                let mut pixels = image::imageops::resize(&linear, side, side, image::imageops::FilterType::Triangle).into_raw();
                let mut mean = [0.0f32; 3];
                for pixel in pixels.chunks_exact(3) {
                    for c in 0..3 {
                        mean[c] += pixel[c] / (side * side) as f32;
                    }
                }
                images.insert(name.clone(), (layer as u32, mean.map(linear_to_srgb)));
                let mut size = side;
                for mip in 0..mips {
                    let rgba: Vec<u8> = pixels.chunks_exact(3).flat_map(|p| [linear_to_srgb(p[0]), linear_to_srgb(p[1]), linear_to_srgb(p[2]), 255]).collect();
                    crate::util::write_texture_layer(queue, &texture, mip, layer as u32, size, size, &rgba, 4);
                    if size > 1 {
                        pixels = half(&pixels, size);
                        size /= 2;
                    }
                }
            }
        }
        // A small valid texture is still bound for Reborn. No additional draw or upload is
        // needed in a frame; only vertices whose material opts in sample this bank.
        for mip in 0..mips {
            let size = (side >> mip).max(1);
            crate::util::write_texture_layer(queue, &texture, mip, layers - 1, size, size, &vec![255; (size * size * 4) as usize], 4);
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Self { view, manifest: accepted.map(|p| p.manifest.clone()), images }
    }

    pub fn id(&self) -> &str {
        self.manifest.as_ref().map_or("reborn", |m| m.id.as_str())
    }

    pub fn livery(&self) -> Option<&ta_skins::Livery> {
        self.manifest.as_ref().and_then(|m| m.livery.as_ref())
    }

    pub fn applies_to(&self, unit: &str) -> bool {
        self.manifest.as_ref().is_some_and(|m| m.livery.is_some() || !m.textures.is_empty() || m.units.get(unit).is_some_and(|u| !u.textures.is_empty()))
    }

    pub fn untextured(&self, unit: &str) -> Option<TextureRef> {
        let original = TextureRef { layer: 0xFFFF, frames: 1, emissive: false, roughness: 0.6, metallic: 0.1, mean: [128; 3] };
        let texture = self.get(unit, "_untextured", original);
        (texture.layer & CUSTOM != 0).then_some(texture)
    }

    pub fn get(&self, unit: &str, texture: &str, original: TextureRef) -> TextureRef {
        let Some(manifest) = &self.manifest else { return original };
        let Some(&(layer, mean)) = manifest.texture_for(unit, texture).and_then(|path| self.images.get(path)) else { return original };
        TextureRef { layer: layer | CUSTOM, mean, ..original }
    }
}

/// A livery as `livery.wgsl` reads it (its `Livery`, the same size: a test checks): colours in
/// linear light, the pattern's numbers, its finish and weather.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub(super) struct LiveryUniform {
    /// w: 1 for each colour it has.
    colours: [[f32; 4]; 3],
    /// w: 1 with a trim.
    trim: [f32; 4],
    /// The colours as they blend seen from afar.
    average: [f32; 4],
    /// The pattern (`ta_skins::Pattern::index`), its frequency (1 / scale), sharpness and
    /// number of colours.
    pattern: [f32; 4],
    /// Where in the noise the seed puts it.
    offset: [f32; 4],
    /// Coverage, detail, roughness and metallic.
    finish: [f32; 4],
    /// Wear, grime, TA's own paint under it (1) and fade.
    weather: [f32; 4],
}

/// The own bank's, then the enemy bank's (the vertex's `ENEMY` bit picks).
pub(super) type Liveries = [LiveryUniform; 2];

/// How much of each pattern its colours cover, about (the second and third take their share
/// from the first when there are fewer): what it blends to, seen from afar.
fn shares(pattern: ta_skins::Pattern) -> [f32; 3] {
    use ta_skins::Pattern::*;
    match pattern {
        Solid => [1.0, 0.0, 0.0],
        Blotch | Digital => [0.42, 0.36, 0.22],
        Splinter => [0.45, 0.33, 0.22],
        Stripes => [0.62, 0.3, 0.08],
        Hex => [0.6, 0.3, 0.1],
        Festive => [0.47, 0.47, 0.06],
        Tricolour => [0.34, 0.33, 0.33],
        Union => [0.6, 0.25, 0.15],
        // (Two colours: stars on a field; three: every other row a stripe.)
        Stars => [0.6, 0.2, 0.2],
    }
}

impl LiveryUniform {
    pub fn of(livery: Option<&ta_skins::Livery>) -> Self {
        let none = ta_skins::Livery::none();
        let livery = livery.unwrap_or(&none);
        let linear = |c: ta_skins::Colour| c.0.map(|v| srgb_to_linear(f32::from(v) / 255.0));
        let count = livery.colours.len().clamp(1, ta_skins::MAX_COLOURS);
        let first = livery.colours.first().copied().unwrap_or(ta_skins::Colour::rgb(0x808080));
        let mut colours = [[0.0; 4]; 3];
        for (i, colour) in colours.iter_mut().enumerate() {
            let [r, g, b] = linear(livery.colours.get(i).copied().unwrap_or(first));
            *colour = [r, g, b, if i < count { 1.0 } else { 0.0 }];
        }
        let mut weights = shares(livery.pattern);
        for i in (count..3).rev() {
            weights[0] += weights[i];
            weights[i] = 0.0;
        }
        let mut average = [0.0; 4];
        for (colour, weight) in colours.iter().zip(weights) {
            for c in 0..3 {
                average[c] += colour[c] * weight;
            }
        }
        let trim = livery.trim.map_or([0.0; 4], |c| {
            let [r, g, b] = linear(c);
            [r, g, b, 1.0]
        });
        // (Far enough apart in the noise that two seeds share nothing; small enough for floats.)
        let place = |k: u32| {
            let h = (livery.seed ^ k.wrapping_mul(0x9e37_79b9)).wrapping_mul(0x85eb_ca6b).rotate_left(13).wrapping_mul(0xc2b2_ae35);
            (h >> 8) as f32 / (1 << 24) as f32 * 256.0
        };
        Self {
            colours,
            trim,
            average,
            pattern: [livery.pattern.index() as f32, 1.0 / livery.scale.clamp(*ta_skins::SCALE.start(), *ta_skins::SCALE.end()), livery.sharpness, count as f32],
            offset: [place(1), place(2), place(3), 0.0],
            finish: [livery.coverage, livery.detail, 0.82 + (0.22 - 0.82) * livery.gloss, livery.metal],
            weather: [livery.wear, livery.grime, if livery.original { 1.0 } else { 0.0 }, livery.fade],
        }
    }
}

fn half(pixels: &[f32], size: u32) -> Vec<f32> {
    let half = size / 2;
    let mut next = vec![0.0; (half * half * 3) as usize];
    for y in 0..half {
        for x in 0..half {
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let from = (((y * 2 + dy) * size + x * 2 + dx) * 3) as usize;
                let to = ((y * half + x) * 3) as usize;
                for c in 0..3 {
                    next[to + c] += pixels[from + c] * 0.25;
                }
            }
        }
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_mips_average_linear_light_without_transparency() {
        let pixels = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        assert_eq!(super::half(&pixels, 2), [0.5, 0.5, 0.5]);
    }

    #[test]
    fn a_livery_reaches_the_shader_in_linear_light_with_its_unused_colours_off() {
        let tiger = ta_skins::builtin_livery("builtin:tiger").unwrap();
        let uniform = LiveryUniform::of(Some(&tiger));
        assert_eq!(uniform.pattern[3], 2.0);
        assert_eq!(uniform.colours.map(|c| c[3]), [1.0, 1.0, 0.0]);
        // (#c86a1e: its red channel 200 is 0.578 in linear light.)
        assert!((uniform.colours[0][0] - 0.578).abs() < 0.002);
        // The third colour's share went to the first.
        let expect = |c: usize| uniform.colours[0][c] * 0.7 + uniform.colours[1][c] * 0.3;
        assert!((0..3).all(|c| (uniform.average[c] - expect(c)).abs() < 1e-5));
        assert_eq!(uniform.trim[3], 0.0);
        assert_eq!(uniform.pattern[0], ta_skins::Pattern::Stripes.index() as f32);
        let other = LiveryUniform::of(Some(&ta_skins::Livery { seed: 3, ..tiger.clone() }));
        assert_ne!(uniform.offset, other.offset, "a seed moves the pattern");
        assert!(uniform.offset.iter().all(|v| (0.0..=256.0).contains(v)));
        let ceremonial = LiveryUniform::of(ta_skins::builtin_livery("builtin:ceremonial").as_ref());
        assert_eq!(ceremonial.trim[3], 1.0);
        assert!(ceremonial.finish[2] < 0.4, "a glossy livery is smooth");
        let heritage = LiveryUniform::of(ta_skins::builtin_livery("builtin:heritage").as_ref());
        assert_eq!(heritage.weather[2], 1.0, "Heritage is laid over TA's own paint");
        assert!(heritage.weather[3] > 0.0, "and faded");
        assert_eq!(uniform.weather[2..], [0.0, 0.0], "a livery over Reborn's paint, unfaded");
    }

    /// The buffer is laid out as both shaders read it.
    #[test]
    fn the_shaders_liveries_are_the_buffers_size() {
        use wgpu::naga;
        for (label, source) in [("unit", crate::util::with_common_for(&super::super::unit_source(), false)), ("ray tracing", crate::raytrace::shader_source())] {
            let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
            let mut layouter = naga::proc::Layouter::default();
            layouter.update(module.to_ctx()).unwrap();
            let size = |name: &str| {
                let (handle, _) = module.types.iter().find(|(_, t)| t.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("{label}: no {name}"));
                layouter[handle].size as usize
            };
            assert_eq!(size("Livery"), std::mem::size_of::<LiveryUniform>(), "{label}");
            assert_eq!(size("Liveries"), std::mem::size_of::<Liveries>(), "{label}");
        }
    }

    #[test]
    fn no_livery_paints_nothing_and_keeps_reborns_wear() {
        let none = LiveryUniform::of(None);
        assert_eq!(none.finish[0], 0.0, "it covers nothing");
        assert_eq!(none.weather, [0.5, 0.0, 0.0, 0.0], "Reborn's own wear, and no grime");
        assert_eq!(none.trim[3], 0.0);
        assert_eq!(none, LiveryUniform::of(Some(&ta_skins::Livery::none())));
    }
}
