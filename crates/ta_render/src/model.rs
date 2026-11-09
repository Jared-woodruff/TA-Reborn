//! 3DO unit/feature models: texture array, mesh building and instanced drawing.
//!
//! All models share one vertex/index buffer. Each vertex carries its piece index; the
//! vertex shader fetches the piece's world matrix from a storage buffer filled per frame,
//! so every unit's animation (COB `turn`/`move`/`hide`) costs one matrix per piece.
//!
//! 3DO data is left-handed relative to our world (barrels point along -z while right arms
//! sit at -x), so z is negated on load. Units therefore face +z (south, towards the classic
//! camera) at heading 0.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use ta_formats::gaf::Gaf;
use ta_formats::image::Palette;
use ta_formats::model3do::{FIXED_ONE, Model3do};

use crate::camera::{Camera, CameraMode};
use crate::occlusion::Footprint;
use crate::util::{linear_to_srgb, srgb_to_linear_lut};

mod batching;
mod caps;
mod connectors;
mod halves;
mod joints;
mod kit;
mod pads;
mod remaster;
mod rounded;
mod skin;
mod sight;
mod smooth;
#[cfg(test)]
mod xray;

pub(crate) use remaster::calm_sprite;

pub use pads::lift_ground_pads;

pub const TEXTURE_SIZE: u32 = 64;
const TEXTURE_MIPS: u32 = 7;

/// Player colours for primitives that use TA's 10-frame team textures.
pub const MAX_TEAMS: u32 = 10;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct ModelVertex {
    pub pos: [f32; 3],
    /// Normal, snorm8x4, with the surface's roughness in w.
    pub normal: [i8; 4],
    /// Texture coordinates within the layer, unorm16x2.
    pub uv: [u16; 2],
    /// Low 16 bits: texture layer (0xFFFF = untextured); bits 16-23: frame count; bit 24: a
    /// light (see `emissive_texture`); bit 25: a road wheel's hub, bit 26: a player's colour
    /// (Reborn's kit, see `kit.rs`); bit 27: one of TA's faces painted in a palette colour;
    /// bit 28: a physical finish (roughness in `normal.w`, metallic in `color.a`);
    /// bit 29: custom skin texture bank; bit 30: the skin's livery paints it; bit 31: the
    /// enemy skin bank (its texture, its livery).
    pub layer: u32,
    /// Base colour (sRGB) for untextured faces; white for textured. Metallic in alpha.
    pub color: [u8; 4],
    pub piece: u32,
    /// Edge wear: this corner's distance (model units, half floats) to up to four of its face's
    /// edges (see `worn_edges`); across the face they interpolate to the distance of every point.
    pub edge_dist: [u16; 4],
    /// Those edges' sharpness, snorm8x4: positive on convex edges, negative in creases.
    pub edge_weight: [i8; 4],
    /// Treads (faces painted with a track): how fast the texture runs along its u as the
    /// face moves forward (u per model unit, signed; 0 on every other face), and the face's
    /// distance to the side (model x), which sets how it runs when the unit turns. Reborn's
    /// kit runs its tracks' bands so (u round the band), and turns its road wheels' hubs a
    /// radian for each of the first per model unit.
    pub tread: [f32; 2],
}

/// Per-instance data (vertex step = instance).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct InstanceData {
    /// Index of this instance's first piece matrix.
    pub matrix_base: u32,
    pub team: u32,
    /// Bit 0: selected, bit 1: under construction, bit 2: cloaked.
    pub flags: u32,
    /// Build progress 0..1 (for the nano-lathe effect).
    pub build: f32,
    /// How damaged the unit is, 0 (whole) to 1 (about to go): soot spreads over it.
    pub damage: f32,
    /// How far it has driven (model units) and turned (radians), for its treads.
    pub travel: [f32; 2],
    /// Which pieces' animated textures play: bits 0-31 of [`DrawInstance::playing`] in x, 32-63
    /// in y.
    pub playing: [u32; 2],
}

pub const FLAG_SELECTED: u32 = 1;
pub const FLAG_BUILDING: u32 = 2;
pub const FLAG_CLOAKED: u32 = 4;
/// Placement ghost at an invalid position (drawn red).
pub const FLAG_INVALID: u32 = 8;
/// A submarine: the whole model looks under the water.
pub const FLAG_SUBMERGED: u32 = 16;
/// Paralysed (an EMP's): electric blue crawls over it.
pub const FLAG_STUNNED: u32 = 32;
/// A structure the viewer saw and no longer sees, remembered where it stood (the fog of war's
/// ghosts): a cool grey, a little translucent, casting no shadow.
pub const FLAG_REMEMBERED: u32 = 64;
/// Bits 8-15 of the flags: how bright the flash of a hit is (0..255).
pub const FLAG_HIT_SHIFT: u32 = 8;
/// Bits 16-23 of the flags: how fast its treads run (world units a second, 0..255): marks on
/// Reborn's tracks running past faster than the eye can follow blur away.
pub const FLAG_RUN_SHIFT: u32 = 16;
/// Bits 24-31 of the flags: how far its lights are out (0 lit, 255 dark): its lamps and glows
/// shine by themselves only while it works (switched on, powered, not stunned).
pub const FLAG_LIGHTS_OUT_SHIFT: u32 = 24;

/// [`DrawInstance::playing`] for things whose animated textures all play (features, previews:
/// nothing of theirs holds them).
pub const PLAYING_ALL: u64 = u64::MAX;

/// Whether piece `piece`'s animated textures play by a [`DrawInstance::playing`] mask (the pieces
/// past 63 share bit 63), as the shaders read it.
pub fn piece_plays(playing: u64, piece: u32) -> bool {
    playing >> piece.min(63) & 1 != 0
}

/// Texture name → first layer and frame count.
#[derive(Clone, Copy, Debug)]
pub struct TextureRef {
    pub layer: u32,
    pub frames: u32,
    /// A light (flashing lamps, beacons, glows): its bright texels glow with HD surfaces.
    pub emissive: bool,
    /// Material finish inferred once from the texture's name and broad painted colour.
    pub roughness: f32,
    pub metallic: f32,
    /// Its first frame's average colour (sRGB): Reborn's plates darken it for their seams.
    pub mean: [u8; 3],
}

fn material_finish(name: &str, rgb: [f32; 3], team: bool) -> (f32, f32) {
    let name = name.to_ascii_lowercase();
    if is_tread_texture(&name) || ["rubber", "tyre", "tire"].iter().any(|s| name.contains(s)) {
        return (0.88, 0.04);
    }
    if emissive_texture(&name) {
        return (0.28, 0.0);
    }
    if team {
        return (0.52, 0.05);
    }
    let high = rgb.into_iter().fold(0.0f32, f32::max);
    let low = rgb.into_iter().fold(1.0f32, f32::min);
    if high < 0.055 {
        return (0.82, 0.08);
    }
    if ["metal", "chrome", "silver", "steel"]
        .iter()
        .any(|s| name.contains(s))
    {
        return (0.42, 0.72);
    }
    if (high - low) / high.max(0.001) < 0.12 && high > 0.2 {
        (0.54, 0.42)
    } else {
        (0.61, 0.08)
    }
}

/// Textures with this many frames are team colours (one frame per player colour); textures
/// with any other number of frames are animations (flashing lamps, glows) that cycle.
pub const TEAM_FRAMES: u32 = 10;

/// Frames per second of animated textures.
pub const TEXTURE_FPS: f32 = 8.0;

/// Whether a texture is a light, by its name: TA's lamps and glows are `flashing01/02`,
/// `glow`, `Beacon1..7` and the alien glows `AlienGlo*`.
fn emissive_texture(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["flashing", "glow", "beacon", "lamp"]
        .iter()
        .any(|k| n.contains(k))
}

/// The unit shader's source: its liveries (`livery.wgsl`, shared with the ray-traced
/// reflections) before it.
pub(crate) fn unit_source() -> String {
    format!("{}\n{}", include_str!("shaders/livery.wgsl"), include_str!("shaders/unit.wgsl"))
}

/// Functional paint and team identity remain legible in every cosmetic pack. Restricting
/// both wildcard and named replacements also protects modded animations and lights.
fn skin_eligible(name: &str, texture: TextureRef) -> bool {
    let name = name.to_ascii_lowercase();
    texture.frames == 1
        && !texture.emissive
        && !is_tread_texture(&name)
        && !name.contains("logo")
        && !MARKINGS.iter().any(|prefix| name.starts_with(prefix))
}

/// The textures of `model` a skin may repaint, each once, in the order the model first paints
/// with it: of one frame (`frames` gives a texture's count, none for one the install lacks), not
/// a light, track, marking or logo, and not on an effect piece (a muzzle flash, a fire point).
pub fn paintable_textures(model: &Model3do, frames: impl Fn(&str) -> Option<u32>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for piece in model.pieces.iter().filter(|piece| !is_effect_piece(&piece.name)) {
        for name in piece.primitives.iter().filter_map(|primitive| primitive.texture.as_deref()) {
            let Some(frames) = frames(name) else { continue };
            let texture = TextureRef { layer: 0, frames, emissive: emissive_texture(name), roughness: 0.0, metallic: 0.0, mean: [0; 3] };
            if skin_eligible(name, texture) && !out.iter().any(|seen| seen.eq_ignore_ascii_case(name)) {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// An entry must fit whole: team frames and animation frames are addressed contiguously.
/// Keep one layer for the fallback, and respect the vertex's eight-bit frame count.
fn texture_entry_fits(used: usize, frames: usize, limit: u32) -> bool {
    (1..=255).contains(&frames)
        && used
            .checked_add(frames)
            .is_some_and(|end| end < limit as usize)
}

/// How many texture arrays (banks) the models' paint may take, each as many layers as the card
/// allows in one (2,048 on D3D12): a mod's models can name more textures than one holds (TA:
/// Enhanced's 1,923 textures take 2,475 layers). Layers are numbered on through the banks (a
/// vertex's 16 bits hold them all), and the shaders look in the second from its first layer on
/// (`SECOND_BANK` in unit.wgsl; raytrace.wgsl compares with the first's count).
pub const TEXTURE_BANKS: u32 = 2;

/// The layers of each bank, `layers` in all and each bank `bank_layers` at most: the first's
/// alone when it holds them all (see [`TEXTURE_BANKS`]).
fn bank_sizes(layers: u32, bank_layers: u32) -> Vec<u32> {
    if layers > bank_layers { vec![bank_layers, layers - bank_layers] } else { vec![layers] }
}

/// A layer's bank and its place in it, the second bank beginning at `second_bank` (0: none).
fn bank_of(layer: u32, second_bank: u32) -> (usize, u32) {
    if second_bank > 0 && layer >= second_bank { (1, layer - second_bank) } else { (0, layer) }
}

/// The textures a model's drawn faces name, in lower case, as [`ModelLibrary::add`] draws
/// them: faces of three corners or more, never a piece's selection plate. What a texture library
/// for the installed models needs of the GAFs (see [`TextureLibrary::new_with_skins`]).
pub fn drawn_textures(model: &Model3do) -> impl Iterator<Item = String> + '_ {
    model.pieces.iter().flat_map(|piece| {
        piece
            .primitives
            .iter()
            .enumerate()
            .filter(move |&(i, prim)| Some(i) != piece.selection_primitive && prim.indices.len() >= 3)
            .filter_map(|(_, prim)| prim.texture.as_deref().map(str::to_ascii_lowercase))
    })
}

/// All 3DO textures (`textures/*.gaf`), or those the models name, packed into mipmapped texture
/// arrays: a bank, or two when one cannot hold them all (see [`TEXTURE_BANKS`]).
pub struct TextureLibrary {
    pub names: HashMap<String, TextureRef>,
    /// Enhanced Classic's paint (the first bank's layers).
    pub view: wgpu::TextureView,
    /// Reborn's paint: every layer remastered (see remaster.rs).
    pub reborn_view: wgpu::TextureView,
    /// Reborn's finish: normal XY, roughness and metallic for every layer.
    pub reborn_physical: wgpu::TextureView,
    /// The same three of the second bank, for the layers from `second_bank` on; with one bank, a
    /// small stand-in nothing samples.
    pub second: [wgpu::TextureView; 3],
    /// The first layer in the second bank; 0 when one bank holds them all.
    pub second_bank: u32,
    skins: skin::Library,
    /// Matching selections alias the first bank, avoiding a second upload and model variant.
    enemy_skins: Option<skin::Library>,
    /// The banks' liveries, as the shaders read them (`skin::Liveries`).
    liveries: wgpu::Buffer,
    /// Every unit takes a livery, Reborn's own until one is set (`new_for_liveries`).
    livery_ready: bool,
    pub layers: u32,
    /// Every layer's paint as its lightness, for Reborn's kit to place its plates by (see
    /// `kit::PaintMap`).
    paint_maps: Vec<Option<kit::PaintMap>>,
}

impl TextureLibrary {
    /// Builds the texture arrays from parsed GAF files: every texture in them.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gafs: &[Gaf],
        palette: &Palette,
    ) -> Self {
        Self::new_with_skin(device, queue, gafs, palette, None, None)
    }

    pub fn new_with_skin(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gafs: &[Gaf],
        palette: &Palette,
        skin: Option<&ta_skins::Pack>,
        used: Option<&std::collections::HashSet<String>>,
    ) -> Self {
        Self::new_with_skins(device, queue, gafs, palette, skin, skin, used)
    }

    /// With `used`, only the textures named there (lower case: [`drawn_textures`] of every model
    /// that may be drawn) are loaded: most of a mod's GAF entries are never painted with (TA:
    /// Enhanced's models name 1,923 of its 4,663), and each costs a layer of every bank's arrays.
    pub fn new_with_skins(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gafs: &[Gaf],
        palette: &Palette,
        skin: Option<&ta_skins::Pack>,
        enemy_skin: Option<&ta_skins::Pack>,
        used: Option<&std::collections::HashSet<String>>,
    ) -> Self {
        Self::build(device, queue, gafs, palette, skin, enemy_skin, false, used)
    }

    /// For an editor's preview: every unit's eligible faces take the livery, Reborn's own until
    /// [`TextureLibrary::set_liveries`] sets another, so it can change from one frame to the next
    /// without the models being made again.
    pub fn new_for_liveries(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gafs: &[Gaf],
        palette: &Palette,
        skin: Option<&ta_skins::Pack>,
        used: Option<&std::collections::HashSet<String>>,
    ) -> Self {
        Self::build(device, queue, gafs, palette, skin, skin, true, used)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gafs: &[Gaf],
        palette: &Palette,
        skin: Option<&ta_skins::Pack>,
        enemy_skin: Option<&ta_skins::Pack>,
        livery_ready: bool,
        used: Option<&std::collections::HashSet<String>>,
    ) -> Self {
        let started = std::time::Instant::now();
        let lut = srgb_to_linear_lut();
        let mut names = HashMap::new();
        let mut images: Vec<Vec<[f32; 3]>> = Vec::new();
        // Keep borrowed native frames for Reborn; Classic's 64px preparation must
        // not discard finer source rows before the detailed material maps are built.
        // (Frames with a stray pink edge are mended first, for both: see `mend_edges`.)
        let mut native_frames: Vec<Option<std::borrow::Cow<ta_formats::image::IndexedImage>>> = Vec::new();
        let mut mended = 0;
        let mut finishes = Vec::new();
        // (Family, first layer, frames, brightness) of every entry: Reborn evens out a family's
        // lit and shaded variants.
        let mut levels: Vec<(String, u32, usize, f32)> = Vec::new();
        // (Each bank as many layers as one array may have; the layers numbered on through them.
        // `TA_TEXTURE_BANK=n` holds a bank to n layers, to see the second at work on TA's own.)
        static BANK: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
        let bank_cap = *BANK.get_or_init(|| std::env::var("TA_TEXTURE_BANK").ok().and_then(|v| v.parse().ok()).filter(|&n: &u32| n >= 16));
        let bank_layers = device.limits().max_texture_array_layers.min(bank_cap.unwrap_or(u32::MAX));
        let limit = bank_layers.saturating_mul(TEXTURE_BANKS);
        let mut omitted = 0;
        let mut unused = 0;
        for gaf in gafs {
            for entry in &gaf.entries {
                let key = entry.name.to_ascii_lowercase();
                if names.contains_key(&key) || entry.frames.is_empty() {
                    continue;
                }
                // (A texture no model paints with takes no layer.)
                if used.is_some_and(|used| !used.contains(&key)) {
                    unused += 1;
                    continue;
                }
                if !texture_entry_fits(images.len(), entry.frames.len(), limit) {
                    // Resolve the sentinel to the final fallback layer below. Registering the
                    // name now preserves precedence if another GAF contains the same entry.
                    names.insert(
                        key,
                        TextureRef {
                            layer: u32::MAX,
                            frames: 1,
                            emissive: false,
                            roughness: 0.6,
                            metallic: 0.05,
                            mean: [255; 3],
                        },
                    );
                    omitted += 1;
                    log::warn!(
                        "model texture {} ({} frames) exceeds the {limit}-layer/frame budget; using fallback",
                        entry.name,
                        entry.frames.len()
                    );
                    continue;
                }
                let layer = images.len() as u32;
                for frame in &entry.frames {
                    let image = match mend_edges(&frame.image, palette) {
                        Some(m) => {
                            mended += 1;
                            std::borrow::Cow::Owned(m)
                        }
                        None => std::borrow::Cow::Borrowed(&frame.image),
                    };
                    images.push(resample(&image, palette, &lut));
                    native_frames.push(Some(image));
                }
                let mean = images[layer as usize]
                    .iter()
                    .fold([0.0f32; 3], |mut sum, rgb| {
                        for c in 0..3 {
                            sum[c] += rgb[c] / (TEXTURE_SIZE * TEXTURE_SIZE) as f32;
                        }
                        sum
                    });
                let (roughness, metallic) = material_finish(
                    &entry.name,
                    mean,
                    entry.frames.len() == TEAM_FRAMES as usize,
                );
                finishes.extend(std::iter::repeat_n(
                    (
                        roughness,
                        metallic,
                        entry.frames.len() == TEAM_FRAMES as usize,
                        emissive_texture(&entry.name),
                        entry.frames.len() > 1,
                        is_tread_texture(&entry.name),
                        remaster::mottled(&entry.name),
                    ),
                    entry.frames.len(),
                ));
                levels.push((remaster::family(&entry.name), layer, entry.frames.len(), mean[0] * 0.2126 + mean[1] * 0.7152 + mean[2] * 0.0722));
                names.insert(
                    key,
                    TextureRef {
                        layer,
                        frames: entry.frames.len() as u32,
                        emissive: emissive_texture(&entry.name),
                        roughness,
                        metallic,
                        mean: mean.map(linear_to_srgb),
                    },
                );
            }
        }
        // Layer 0 must exist even with no data; add a white fallback texture last.
        let fallback = images.len() as u32;
        images.push(vec![[1.0; 3]; (TEXTURE_SIZE * TEXTURE_SIZE) as usize]);
        native_frames.push(None);
        finishes.push((0.6, 0.05, false, false, false, false, false));
        let layers = images.len() as u32;
        for texture in names.values_mut() {
            if texture.layer == u32::MAX {
                texture.layer = fallback;
            }
        }
        if omitted > 0 {
            log::warn!(
                "model textures: {omitted} entries use fallback; {layers}/{limit} layers allocated"
            );
        }

        // The banks: the first as many layers as one array may hold, the second the rest.
        let bank_sizes = bank_sizes(layers, bank_layers);
        let second_bank = if bank_sizes.len() > 1 { bank_sizes[0] } else { 0 };
        let make_texture = |label: &str, size, mips, format| -> Vec<wgpu::Texture> {
            bank_sizes
                .iter()
                .enumerate()
                .map(|(bank, &depth)| {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some(&format!("{label} (bank {})", bank + 1)),
                        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: depth },
                        mip_level_count: mips,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    })
                })
                .collect()
        };
        let texture = make_texture(
            "classic model paint",
            TEXTURE_SIZE,
            TEXTURE_MIPS,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let reborn_texture = make_texture(
            "Reborn model paint",
            remaster::SIZE,
            remaster::MIP_COUNT,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let reborn_physical = make_texture(
            "Reborn model finish",
            remaster::SIZE,
            remaster::MIP_COUNT,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        // Each layer's gain towards its family's lit variants.
        let mut gains = vec![1.0f32; images.len()];
        {
            let mut families: HashMap<&str, Vec<usize>> = HashMap::new();
            for (i, (family, ..)) in levels.iter().enumerate() {
                families.entry(family.as_str()).or_default().push(i);
            }
            for members in families.values() {
                let brightness: Vec<f32> = members.iter().map(|&i| levels[i].3).collect();
                for (&i, gain) in members.iter().zip(remaster::family_gains(&brightness)) {
                    let (_, first, frames, _) = levels[i];
                    gains[first as usize..first as usize + frames].fill(gain);
                }
            }
        }
        // Every layer's lightness, for the kit (see `kit::PaintMap`).
        let paint_maps: Vec<Option<kit::PaintMap>> = native_frames
            .iter()
            .map(|frame| {
                let frame = frame.as_deref()?;
                let light: Vec<f32> = frame
                    .pixels
                    .iter()
                    .map(|&i| {
                        let [r, g, b] = palette.rgb(i).map(|c| lut[c as usize]);
                        (0.2126 * r + 0.7152 * g + 0.0722 * b).cbrt()
                    })
                    .collect();
                kit::PaintMap::new(frame.width as usize, frame.height as usize, &light)
            })
            .collect();
        // Reborn's paint and finish for every layer, made in parallel.
        let empty = ta_formats::image::IndexedImage::default();
        let remastered: Vec<remaster::SurfaceMaps> = {
            use rayon::prelude::*;
            (0..images.len())
                .into_par_iter()
                .map(|layer| {
                    let (roughness, metallic, team, emissive, animated, tread, mottled) = finishes[layer];
                    remaster::build(
                        remaster::Source {
                            image: native_frames[layer].as_deref().unwrap_or(&empty),
                            gain: gains[layer],
                            roughness,
                            metallic,
                            protected: team || emissive || animated,
                            team,
                            tread,
                            mottled,
                        },
                        palette,
                        &lut,
                    )
                })
                .collect()
        };
        for (layer, img) in images.iter().enumerate() {
            let (bank, place) = bank_of(layer as u32, second_bank);
            {
                let target = &texture[bank];
                let mut level = img.clone();
                let mut size = TEXTURE_SIZE;
                for mip in 0..TEXTURE_MIPS {
                    let rgba: Vec<u8> = level
                        .iter()
                        .flat_map(|c| {
                            [
                                linear_to_srgb(c[0]),
                                linear_to_srgb(c[1]),
                                linear_to_srgb(c[2]),
                                255,
                            ]
                        })
                        .collect();
                    crate::util::write_texture_layer(
                        queue,
                        target,
                        mip,
                        place,
                        size,
                        size,
                        &rgba,
                        4,
                    );
                    if size > 1 {
                        let half = size / 2;
                        let mut next = vec![[0f32; 3]; (half * half) as usize];
                        for y in 0..half {
                            for x in 0..half {
                                let mut acc = [0f32; 3];
                                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                                    let c = level[((y * 2 + dy) * size + x * 2 + dx) as usize];
                                    for k in 0..3 {
                                        acc[k] += c[k] * 0.25;
                                    }
                                }
                                next[(y * half + x) as usize] = acc;
                            }
                        }
                        level = next;
                        size = half;
                    }
                }
            }
            let maps = &remastered[layer];
            for (target, levels) in [
                (&reborn_texture[bank], &maps.color),
                (&reborn_physical[bank], &maps.physical),
            ] {
                for (mip, rgba) in levels.iter().enumerate() {
                    let size = (remaster::SIZE >> mip).max(1);
                    crate::util::write_texture_layer(
                        queue,
                        target,
                        mip as u32,
                        place,
                        size,
                        size,
                        rgba,
                        4,
                    );
                }
            }
        }
        let array = |texture: &wgpu::Texture| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        // (With one bank, the second's bindings take a texel nothing samples.)
        let second = if second_bank > 0 {
            [array(&texture[1]), array(&reborn_texture[1]), array(&reborn_physical[1])]
        } else {
            let stand_in = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("model paint (no second bank)"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            crate::util::write_texture_layer(queue, &stand_in, 0, 0, 1, 1, &[255; 4], 4);
            [array(&stand_in), array(&stand_in), array(&stand_in)]
        };
        let (view, reborn_view, reborn_physical) = (array(&texture[0]), array(&reborn_texture[0]), array(&reborn_physical[0]));
        log::info!(
            "model textures: {} names{}, {layers} layers in {} bank{} ({mended} with a pink edge mended); Reborn's paint {}x{} ({:.1} MiB with its finish); built in {:.0} ms",
            names.len(),
            if unused > 0 { format!(" ({unused} more no model paints with left out)") } else { String::new() },
            bank_sizes.len(),
            if bank_sizes.len() == 1 { "" } else { "s" },
            remaster::SIZE,
            remaster::SIZE,
            (0..remaster::MIP_COUNT)
                .map(|mip| u64::from((remaster::SIZE >> mip).max(1)).pow(2) * 8 * u64::from(layers))
                .sum::<u64>() as f64
                / 1048576.0,
            started.elapsed().as_secs_f64() * 1000.0
        );
        let skins = skin::Library::new(device, queue, skin);
        let enemy_skins = (skin.map(|p| &p.manifest.id) != enemy_skin.map(|p| &p.manifest.id))
            .then(|| skin::Library::new(device, queue, enemy_skin));
        let liveries = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("unit liveries"),
            size: std::mem::size_of::<skin::Liveries>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let library = TextureLibrary {
            names,
            view,
            reborn_view,
            reborn_physical,
            second,
            second_bank,
            skins,
            enemy_skins,
            liveries,
            livery_ready,
            layers,
            paint_maps,
        };
        library.set_liveries(queue, library.skins.livery(), library.enemy_skins.as_ref().unwrap_or(&library.skins).livery());
        library
    }

    /// The liveries the banks paint with (none: Reborn's own colours, finish and wear). Only
    /// faces of models made with a livery take one: a skin's, or every unit's with
    /// [`TextureLibrary::new_for_liveries`].
    pub fn set_liveries(&self, queue: &wgpu::Queue, own: Option<&ta_skins::Livery>, enemy: Option<&ta_skins::Livery>) {
        let liveries: skin::Liveries = [skin::LiveryUniform::of(own), skin::LiveryUniform::of(enemy)];
        queue.write_buffer(&self.liveries, 0, bytemuck::cast_slice(&liveries));
    }

    pub(crate) fn livery_buffer(&self) -> &wgpu::Buffer {
        &self.liveries
    }

    pub fn get(&self, name: &str) -> Option<TextureRef> {
        self.names.get(&name.to_ascii_lowercase()).copied()
    }

    pub(crate) fn skin_view(&self) -> &wgpu::TextureView {
        &self.skins.view
    }

    pub(crate) fn enemy_skin_view(&self) -> &wgpu::TextureView {
        &self.enemy_skins.as_ref().unwrap_or(&self.skins).view
    }

    /// The skin accepted for enemies, including Reborn after a GPU-limit fallback.
    pub fn enemy_unit_skin_id(&self) -> &str {
        self.enemy_skins.as_ref().unwrap_or(&self.skins).id()
    }

    /// The skin accepted by this device, including Reborn after a GPU-limit fallback.
    pub fn unit_skin_id(&self) -> &str {
        self.skins.id()
    }
}

/// Doubles an indexed image with Scale2x (EPX): each pixel becomes four, and where two
/// neighbours of the same colour meet at a corner that corner takes their colour, so
/// staircase diagonals come out as finer steps while flat areas and hard edges stay
/// exactly as painted (palette indices are compared, never blended).
fn scale2x(pixels: &[u8], w: usize, h: usize) -> Vec<u8> {
    let at = |x: isize, y: isize| {
        pixels[y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize]
    };
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let (b, d, e, f, hh) = (
                at(x, y - 1),
                at(x - 1, y),
                at(x, y),
                at(x + 1, y),
                at(x, y + 1),
            );
            let (mut e0, mut e1, mut e2, mut e3) = (e, e, e, e);
            if b != hh && d != f {
                if d == b {
                    e0 = d;
                }
                if b == f {
                    e1 = f;
                }
                if d == hh {
                    e2 = d;
                }
                if hh == f {
                    e3 = f;
                }
            }
            let (ox, oy) = (x as usize * 2, y as usize * 2);
            out[oy * w * 2 + ox] = e0;
            out[oy * w * 2 + ox + 1] = e1;
            out[(oy + 1) * w * 2 + ox] = e2;
            out[(oy + 1) * w * 2 + ox + 1] = e3;
        }
    }
    out
}

/// Many of TA's textures have a row or a column at their edge (most often the top row) painted
/// in a pinkish purple that the row inside it is not, a slip of the tools they were made with.
/// Faces map their texture edge to edge, so it shows as a pink line along the face's edge,
/// crisp in Enhanced Classic. Where an edge is markedly pinker than the row inside it, its pink
/// pixels take the colours of the pixels inside them. Returns the mended frame, if it needed it.
fn mend_edges(image: &ta_formats::image::IndexedImage, palette: &Palette) -> Option<ta_formats::image::IndexedImage> {
    let (w, h) = (image.width as usize, image.height as usize);
    if w < 4 || h < 4 || image.pixels.len() != w * h {
        return None;
    }
    let pink = |i: u8| {
        let [r, g, b] = palette.rgb(i).map(i32::from);
        r > g + 25 && b > g + 25
    };
    // Each edge's pixels, each with the one inside it: the top row, the bottom, the left
    // column and the right.
    let sides: [Vec<(usize, usize)>; 4] = [
        (0..w).map(|x| (x, w + x)).collect(),
        (0..w).map(|x| ((h - 1) * w + x, (h - 2) * w + x)).collect(),
        (0..h).map(|y| (y * w, y * w + 1)).collect(),
        (0..h).map(|y| (y * w + w - 1, y * w + w - 2)).collect(),
    ];
    let mut out: Option<ta_formats::image::IndexedImage> = None;
    for side in &sides {
        let share = |inner: bool| side.iter().filter(|&&(e, i)| pink(image.pixels[if inner { i } else { e }])).count() as f32 / side.len() as f32;
        if share(false) - share(true) < 0.15 {
            continue;
        }
        let mended = out.get_or_insert_with(|| image.clone());
        for &(e, i) in side {
            if pink(image.pixels[e]) && !pink(image.pixels[i]) {
                mended.pixels[e] = image.pixels[i];
            }
        }
    }
    out
}

fn resample(
    img: &ta_formats::image::IndexedImage,
    palette: &Palette,
    lut: &[f32; 256],
) -> Vec<[f32; 3]> {
    resample_to(img, palette, lut, TEXTURE_SIZE)
}

/// Resolve the native indexed artwork directly at the requested layer size.
/// Scale2x refines existing palette edges; shrinking uses linear-light box filtering.
fn resample_to(
    img: &ta_formats::image::IndexedImage,
    palette: &Palette,
    lut: &[f32; 256],
    size: u32,
) -> Vec<[f32; 3]> {
    let n = size as usize;
    let mut out = vec![[0f32; 3]; n * n];
    if img.pixels.is_empty() {
        return vec![[1.0; 3]; n * n];
    }
    // Small textures (most of TA's are 32 × 32) are doubled with Scale2x until they fill a
    // layer, rather than blown up into blocks; the box filter below takes any excess.
    let (mut w, mut h) = (img.width.max(1) as usize, img.height.max(1) as usize);
    let mut scaled = std::borrow::Cow::Borrowed(img.pixels.as_slice());
    while (w < n || h < n) && w.max(h) <= n {
        scaled = std::borrow::Cow::Owned(scale2x(&scaled, w, h));
        w *= 2;
        h *= 2;
    }
    let pixels = scaled.as_ref();
    for y in 0..n {
        for x in 0..n {
            // Source footprint of this destination texel.
            let x0 = x * w / n;
            let x1 = ((x + 1) * w).div_ceil(n).max(x0 + 1).min(w);
            let y0 = y * h / n;
            let y1 = ((y + 1) * h).div_ceil(n).max(y0 + 1).min(h);
            let mut acc = [0f32; 3];
            let mut count = 0.0;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let [r, g, b] = palette.rgb(pixels[sy * w + sx]);
                    acc[0] += lut[r as usize];
                    acc[1] += lut[g as usize];
                    acc[2] += lut[b as usize];
                    count += 1.0;
                }
            }
            out[y * n + x] = [acc[0] / count, acc[1] / count, acc[2] / count];
        }
    }
    out
}

/// GPU-side description of one loaded model.
#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub name: String,
    pub first_index: u32,
    pub index_count: u32,
    pub base_vertex: i32,
    /// Piece hierarchy (parent indices, rest offsets in world orientation).
    pub pieces: Vec<PieceInfo>,
    pub radius: f32,
    pub height: f32,
    /// Where the model stands on the ground (contact occlusion, see occlusion.rs).
    pub(crate) footprint: Footprint,
}

#[derive(Clone, Debug)]
pub struct PieceInfo {
    pub name: String,
    pub parent: Option<usize>,
    /// Offset from parent in world orientation (z already negated).
    pub offset: Vec3,
    /// This piece's triangles in the shared index buffer (pieces are contiguous).
    pub first_index: u32,
    pub index_count: u32,
}

impl ModelInfo {
    /// Rest-pose piece matrices (no animation) relative to the model origin.
    pub fn rest_pose(&self) -> Vec<Mat4> {
        let mut out: Vec<Mat4> = Vec::with_capacity(self.pieces.len());
        for p in &self.pieces {
            let local = Mat4::from_translation(p.offset);
            out.push(match p.parent {
                Some(parent) => out[parent] * local,
                None => local,
            });
        }
        out
    }
}

pub type ModelId = u32;

/// What Reborn's kit made of a model.
#[derive(Clone, Copy, Debug, Default)]
pub struct DetailStats {
    pub triangles: usize,
    pub detailed_triangles: usize,
    pub plates: usize,
    pub bolts: usize,
    pub wheels: usize,
    /// Triangles added rounding TA's curves (see rounded.rs), and how far that moved a point the
    /// most (model units).
    pub rounded: usize,
    pub bowed: f32,
}

/// How many pixels across a model of `radius` at `centre` is on screen.
fn projected_pixels(camera: &Camera, centre: Vec3, radius: f32) -> f32 {
    if camera.mode == CameraMode::Classic {
        return 2.0 * radius * camera.zoom;
    }
    let distance = camera.view_distance(centre);
    let pixels = radius * camera.viewport.y / ((camera.fov_y * 0.5).tan() * distance.max(1.0));
    // A model surrounding the eye is still close: its centre can pass behind the camera while
    // its barrel or hull is in view.
    if distance + radius > 0.0 { pixels } else { 0.0 }
}

/// Reborn's detail shows from this many pixels across: smaller, its plates and bolts would be
/// finer than a pixel.
const DETAIL_PIXELS: f32 = 56.0;

/// A piece's faces as drawn, ready for Reborn's kit: what each is to it (its kind, its paint's
/// colour and whether that is a player's colour), and which of each one's edges are open.
struct Prepared {
    faces: Vec<smooth::SourceFace>,
    /// (Its kind, its paint's colour, whether a player's colour, its texture's layer.)
    kinds: Vec<FaceRole>,
    open: Vec<Vec<bool>>,
    /// The model's face each is, or mirrors (see `FaceSource::of`).
    origins: Vec<Option<usize>>,
}

/// CPU-side accumulation of all model meshes; uploaded when changed.
#[derive(Default)]
pub struct ModelLibrary {
    pub models: Vec<ModelInfo>,
    by_name: HashMap<String, ModelId>,
    /// Original model -> Reborn's detailed version (the kit fitted, see kit.rs): the same
    /// pieces in the same order, drawn when the model is big enough on screen.
    detail: HashMap<ModelId, ModelId>,
    /// And back: a detailed version -> its original.
    original: HashMap<ModelId, ModelId>,
    detail_stats: HashMap<ModelId, DetailStats>,
    /// How long building the models (the kit included) has taken, all told.
    build_time: std::time::Duration,
    /// The holes closed and the faces turned round in them (see caps.rs).
    mended: MendStats,
    /// Whether forms are made whole (halves, sheets' backs, shallow shells' rims: see halves.rs
    /// and caps.rs); None as `TA_FORMS` says.
    forms: Option<bool>,
    /// Whether Reborn's detail rounds TA's curves (see rounded.rs); None as `TA_ROUNDED` says
    /// (`TA_ROUNDED=0`: as drawn, for comparisons).
    rounded: Option<bool>,
    vertices: Vec<ModelVertex>,
    indices: Vec<u32>,
    dirty: bool,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    /// The buffers are also what ray tracing builds its acceleration structures from.
    pub(crate) blas_input: bool,
}

impl ModelLibrary {
    pub fn id(&self, name: &str) -> Option<ModelId> {
        self.by_name.get(&name.to_ascii_lowercase()).copied()
    }

    pub fn info(&self, id: ModelId) -> &ModelInfo {
        &self.models[id as usize]
    }

    /// A model's triangles' corners as built (piece space), each with its piece: for framing
    /// portraits and seating wrecks/debris against the visible geometry, including caps/forms.
    pub fn corners(&self, id: ModelId) -> impl Iterator<Item = (u32, Vec3)> + '_ {
        let info = &self.models[id as usize];
        let base = info.base_vertex as usize;
        let range = info.first_index as usize..(info.first_index + info.index_count) as usize;
        self.indices.get(range).unwrap_or(&[]).iter().filter_map(move |&i| self.vertices.get(base + i as usize)).map(|v| (connectors::own_piece(v.piece), Vec3::from_array(v.pos)))
    }

    /// Converts a 3DO into renderable triangles and registers it under `name`.
    pub fn add(
        &mut self,
        name: &str,
        model: &Model3do,
        textures: &TextureLibrary,
        palette: &Palette,
    ) -> ModelId {
        self.add_cpu(name, model, |name| textures.get(name), |layer| textures.paint_maps.get(layer as usize).and_then(Option::as_ref), palette)
    }

    fn add_cpu<'p>(
        &mut self,
        name: &str,
        model: &Model3do,
        texture: impl Fn(&str) -> Option<TextureRef>,
        paint_map: impl Fn(u32) -> Option<&'p kit::PaintMap>,
        palette: &Palette,
    ) -> ModelId {
        self.add_cpu_scoped(name, name, model, |name, _| texture(name), paint_map, palette, None, None, true)
    }

    /// A model's shapes as the game builds them (TA's faces made whole and capped, and Reborn's
    /// detail), with no paint and no graphics device: for tests and tools that measure what is
    /// drawn (`corners`).
    pub fn add_shape(&mut self, name: &str, model: &Model3do, palette: &Palette) -> ModelId {
        let plain = |name: &str| Some(TextureRef { layer: 0, frames: 1, emissive: emissive_texture(name), roughness: 0.5, metallic: 0.1, mean: [128; 3] });
        self.add_cpu(name, model, plain, |_| None, palette)
    }

    /// A unit's cosmetic paint is keyed by its FBI name, separately from its shared object.
    /// Two units using the same 3DO may have different skins; features and projectiles never
    /// resolve through a unit's overrides.
    pub fn add_unit(
        &mut self,
        unit: &str,
        object: &str,
        model: &Model3do,
        textures: &TextureLibrary,
        palette: &Palette,
    ) -> ModelId {
        self.add_unit_for_side(unit, object, model, textures, palette, false)
    }

    /// Registers the viewer-relative cosmetic variant. The material bank and accepted skin
    /// identity are part of the model cache; matching choices share all mesh buffers.
    pub fn add_unit_for_side(
        &mut self,
        unit: &str,
        object: &str,
        model: &Model3do,
        textures: &TextureLibrary,
        palette: &Palette,
        enemy: bool,
    ) -> ModelId {
        let unit = unit.to_ascii_lowercase();
        let skins = if enemy { textures.enemy_skins.as_ref().unwrap_or(&textures.skins) } else { &textures.skins };
        let bank = if enemy && textures.enemy_skins.is_some() { skin::ENEMY } else { 0 };
        let bank_texture = |mut texture: TextureRef| {
            if texture.layer & skin::CUSTOM != 0 { texture.layer |= bank; }
            texture
        };
        if !skins.applies_to(&unit) && !textures.livery_ready {
            return self.add(object, model, textures, palette);
        }
        let livery = (skins.livery().is_some() || textures.livery_ready).then_some(bank);
        // (A livery over TA's own paint is on TA's own shapes: no kit, at any size.)
        let kit = !skins.livery().is_some_and(|l| l.original);
        self.add_cpu_scoped(
            &format!("(unit:{unit}/{object}/skin:{})", skins.id()),
            object,
            model,
            |name, eligible_piece| {
                let original = textures.get(name)?;
                Some(if eligible_piece && skin_eligible(name, original) { bank_texture(skins.get(&unit, &name.to_ascii_lowercase(), original)) } else { original })
            },
            |layer| textures.paint_maps.get(layer as usize).and_then(Option::as_ref),
            palette,
            livery,
            skins.untextured(&unit).map(bank_texture),
            kit,
        )
    }

    fn add_cpu_scoped<'p>(
        &mut self,
        key: &str,
        name: &str,
        model: &Model3do,
        texture: impl Fn(&str, bool) -> Option<TextureRef>,
        paint_map: impl Fn(u32) -> Option<&'p kit::PaintMap>,
        palette: &Palette,
        livery: Option<u32>,
        untextured: Option<TextureRef>,
        kit: bool,
    ) -> ModelId {
        let key = key.to_ascii_lowercase();
        if let Some(&id) = self.by_name.get(&key) {
            return id;
        }
        let started = std::time::Instant::now();
        let base_vertex = self.vertices.len() as i32;
        let first_index = self.indices.len() as u32;
        let fixed = |v: [i32; 3]| Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / FIXED_ONE;
        let lut = srgb_to_linear_lut();
        // Reborn's detailed pieces, and what went into them. (A very dense mod model keeps its
        // own surface: the kit is for TA's sparse ones.)
        let mut detailed: Vec<kit::Kit> = Vec::with_capacity(model.pieces.len());
        let mut stats = DetailStats::default();
        let mut budget = kit::Budget::default();
        static ROUNDED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let rounding = self.rounded.unwrap_or_else(|| *ROUNDED.get_or_init(|| std::env::var("TA_ROUNDED").map_or(true, |v| v != "0")));
        let fit_kit = kit && model
            .pieces
            .iter()
            .flat_map(|piece| &piece.primitives)
            .map(|primitive| primitive.indices.len())
            .sum::<usize>()
            <= 80_000;
        let mut max_r2 = 0f32;
        let mut max_h = 0f32;
        let mut polygons: Vec<Vec<Vec3>> = Vec::new();
        let rest: Vec<Vec3> = {
            let mut acc: Vec<Vec3> = Vec::with_capacity(model.pieces.len());
            for p in &model.pieces {
                let off = fixed(p.offset);
                acc.push(match p.parent {
                    Some(parent) => acc[parent] + off,
                    None => off,
                });
            }
            acc
        };

        // Each piece's faces as TA drew them (the selection plate and stray lines left out), the
        // halves of barrels and booms TA's camera saw made whole (see halves.rs), and the holes
        // it never showed closed (see caps.rs).
        let drawn: Vec<Vec<&ta_formats::model3do::Primitive>> = model
            .pieces
            .iter()
            .zip(&rest)
            .map(|(piece, at)| {
                let plate = piece.selection_primitive.or_else(|| unflagged_ground_plate(piece, at.y));
                piece.primitives.iter().enumerate().filter(|&(prim_i, prim)| Some(prim_i) != plate && prim.indices.len() >= 3).map(|(_, prim)| prim).collect()
            })
            .collect();
        let mut shapes: Vec<Vec<Vec<Vec3>>> = model
            .pieces
            .iter()
            .zip(&drawn)
            .map(|(piece, prims)| prims.iter().map(|prim| prim.indices.iter().map(|&i| fixed(piece.vertices[i as usize])).collect()).collect())
            .collect();
        fn to_mend<'a>(model: &Model3do, shapes: &'a [Vec<Vec<Vec3>>], rest: &[Vec3]) -> Vec<caps::PieceFaces<'a>> {
            model
                .pieces
                .iter()
                .zip(shapes)
                .zip(rest)
                .map(|((piece, faces), &offset)| caps::PieceFaces { faces: faces.as_slice(), offset, mend: mending() && !is_effect_piece(&piece.name) })
                .collect()
        }
        let forms = mending() && self.forms.unwrap_or_else(whole_forms);
        let joints = if forms { joints::supports(model, &shapes) } else { vec![Vec::new(); shapes.len()] };
        let mut halves = if forms { halves::complete(&to_mend(model, &shapes, &rest)) } else { vec![halves::Halves::default(); shapes.len()] };
        for (faces, h) in shapes.iter_mut().zip(&halves) {
            faces.extend(h.mirrored.iter().map(|m| m.corners.clone()));
        }
        let mut mendings: Vec<caps::Mending> = caps::mend(&to_mend(model, &shapes, &rest), forms);
        // (And the faces a 3D camera sees from behind given their backs: see halves.rs.)
        if forms {
            let before: Vec<usize> = halves.iter().map(|h| h.mirrored.len()).collect();
            halves::give_backs(&to_mend(model, &shapes, &rest), &mut mendings, &mut halves);
            for ((faces, h), &k) in shapes.iter_mut().zip(&halves).zip(&before) {
                faces.extend(h.mirrored[k..].iter().map(|m| m.corners.clone()));
            }
        }
        self.mended.add(&mendings);
        self.mended.add_halves(&halves);
        // (And TA Reborn's columns between pieces TA left apart: see connectors.rs.)
        let names: Vec<&str> = model.pieces.iter().map(|p| p.name.as_str()).collect();
        let connectors = if forms { connectors::build(name, &names, &shapes, &rest) } else { connectors::Connectors::default() };

        let mut piece_ranges = Vec::with_capacity(model.pieces.len());
        let mut prepared: Vec<Prepared> = Vec::with_capacity(model.pieces.len());
        for (pi, piece) in model.pieces.iter().enumerate() {
            let piece_first = self.indices.len() as u32;
            // The piece's faces first, for the edges they share (see `worn_edges`): its own (those
            // wound against their neighbours turned round, their paint where it was), the faces
            // mirrored from them making halves whole (their paint mirrored with them), then the
            // caps closing its holes.
            let mended = &mendings[pi];
            let own = drawn[pi].len();
            let faces: Vec<(FaceSource, Face)> = std::mem::take(&mut shapes[pi])
                .into_iter()
                .enumerate()
                .map(|(f, mut corners)| {
                    let mirrored = f.checked_sub(own).map(|m| &halves[pi].mirrored[m]);
                    let prim = drawn[pi][mirrored.map_or(f, |m| m.face)];
                    // (A mirrored face is painted as the face it mirrors, laid as on that face.)
                    let mirror: Option<Vec<Vec3>> = mirrored.map(|_| prim.indices.iter().map(|&i| fixed(piece.vertices[i as usize])).collect());
                    let back = mirrored.is_some_and(|m| m.back);
                    let late = mirrored.is_some();
                    let of = Some(mirrored.map_or(f, |m| m.face));
                    let source = |uvs| FaceSource { texture: prim.texture.as_deref(), color_index: prim.color_index, cap: false, uvs, mirror: mirror.clone(), back, late, of };
                    if mended.flipped.contains(&f) {
                        let mut uvs = polygon_uvs(&corners, face_normal(&corners));
                        uvs.reverse();
                        corners.reverse();
                        let normal = face_normal(&corners);
                        (source(Some(uvs)), Face { corners, normal })
                    } else {
                        let normal = face_normal(&corners);
                        (source(None), Face { corners, normal })
                    }
                })
                .collect();
            // (A logo on a quad that is no parallelogram laid on a grid: see `even_grid`.)
            let mut faces: Vec<(FaceSource, Face)> = faces.into_iter().flat_map(|(source, face)| even_grid(source, face)).collect();
            // (A cap closing a rim, or a hole round faces mirrored to make a half whole, a
            // muzzle's end, is late too.)
            let metal = |late| FaceSource { texture: None, color_index: 0, cap: true, uvs: None, mirror: None, back: false, late, of: None };
            for cap in &mended.caps {
                let late = cap.rim || cap.late || cap.faces.iter().any(|&f| f as usize >= own);
                if cap.convex {
                    faces.push((metal(late), Face { corners: cap.corners.clone(), normal: cap.normal }));
                    continue;
                }
                for t in &cap.triangles {
                    let corners: Vec<Vec3> = t.iter().map(|&i| cap.corners[i as usize]).collect();
                    let normal = face_normal(&corners);
                    faces.push((metal(late), Face { corners, normal }));
                }
            }
            for corners in &joints[pi] {
                faces.push((metal(true), Face { corners: corners.clone(), normal: face_normal(corners) }));
            }
            if mending() && !is_effect_piece(&piece.name) {
                thicken_lone_sheets(&mut faces);
            }
            // (TA's own faces, and the caps of before, first and in their order; then what makes
            // forms whole.)
            faces.sort_by_key(|(s, _)| s.late);
            let late: Vec<bool> = faces.iter().map(|(s, _)| s.late).collect();
            let origins: Vec<Option<usize>> = faces.iter().map(|(s, _)| s.of).collect();
            let worn = worn_edges(&faces.iter().map(|(_, f)| f).collect::<Vec<_>>(), &late);
            let open = open_edges(&faces.iter().map(|(_, f)| f).collect::<Vec<_>>(), &faces.iter().map(|(s, _)| s.back).collect::<Vec<_>>(), &late);
            let mut source_faces = Vec::with_capacity(faces.len());
            let effect_piece = is_effect_piece(&piece.name);
            // What each face is to the kit, and its paint's colour.
            let mut kinds = Vec::with_capacity(faces.len());
            for ((source, face), edges) in faces.iter().zip(&worn) {
                let poly = &face.corners;
                let n = face.normal;
                let mut edge_weight = [0i8; 4];
                for (k, e) in edges.iter().enumerate() {
                    edge_weight[k] = (e.weight.clamp(-1.0, 1.0) * 127.0) as i8;
                }

                let texture_name = source.texture;
                let tex = texture_name.and_then(|name| texture(name, !effect_piece && !source.cap)).or_else(|| {
                    // Named textures that are missing from the install retain their fallback;
                    // only actual palette-painted armour opts into the reserved material.
                    (texture_name.is_none() && !effect_piece && !source.cap
                        && palette.rgb(source.color_index as u8).into_iter().max().unwrap_or(0) >= 48)
                        .then_some(untextured).flatten()
                });
                let name = texture_name.unwrap_or("").to_ascii_lowercase();
                let kind = match tex {
                    // TA's effect pieces (a muzzle's flash, a fire point) and its untextured faces
                    // (flashes, glows, dark holes) stay as they are, and so do the caps closing
                    // holes (undersides, mostly).
                    _ if effect_piece || source.cap => kit::FaceKind::Keep,
                    // (A flat face in a palette colour: one plate, unbolted, as a picture's.)
                    None => kit::FaceKind::Picture,
                    Some(t) if t.layer & skin::CUSTOM != 0 => kit::FaceKind::Picture,
                    _ if name.contains("tredside") => kit::FaceKind::TrackSide,
                    Some(t) if t.emissive || t.frames > 1 && t.frames != TEAM_FRAMES || name.contains("logo") || is_tread_texture(&name) => kit::FaceKind::Keep,
                    _ if MARKINGS.iter().any(|m| name.starts_with(m)) => kit::FaceKind::Keep,
                    _ if PICTURES.iter().any(|m| name.starts_with(m)) => kit::FaceKind::Picture,
                    _ => kit::FaceKind::Plating,
                };
                let tread = if texture_name.is_some_and(is_tread_texture) {
                    tread_of(poly, rest[pi])
                } else {
                    [0.0; 2]
                };
                let (layer, mut color) = match tex {
                    // Low 16 bits: the layer; then 8 bits of frame count; bit 24: a light.
                    Some(t) => (t.layer | (t.frames.min(0xFF) << 16) | ((t.emissive as u32) << 24), [255; 4]),
                    None => {
                        let [r, g, b] = if source.cap { CAP_METAL.0 } else { palette.rgb((source.color_index & 0xFF) as u8) };
                        (0xFFFF | (1 << 16) | if source.cap { 0 } else { PALETTE_FACE }, [r, g, b, 255])
                    }
                };
                // (A player's colour: its first frame's paint as the grey that each player's
                // colour multiplies, so its seams come out in a dark shade of that colour.)
                let team = tex.is_some_and(|t| t.frames == TEAM_FRAMES);
                let paint = match tex {
                    Some(t) if team => team_grey(t.mean, &lut),
                    Some(t) => t.mean,
                    None => [color[0], color[1], color[2]],
                };
                let (roughness, metallic) = tex.map_or_else(
                    || {
                        if source.cap {
                            return (CAP_METAL.2, CAP_METAL.1);
                        }
                        material_finish(
                            texture_name.unwrap_or(""),
                            [
                                lut[color[0] as usize],
                                lut[color[1] as usize],
                                lut[color[2] as usize],
                            ],
                            false,
                        )
                    },
                    |t| (t.roughness, t.metallic),
                );
                let normal = [
                    (n.x * 127.0) as i8,
                    (n.y * 127.0) as i8,
                    (n.z * 127.0) as i8,
                    (roughness * 127.0).round() as i8,
                ];
                color[3] = (metallic * 255.0).round() as u8;
                // (A livery paints the armour a skin's textures leave: of one frame, not a light,
                // marking or track; a palette colour not too dark to be a hole or a shadow.)
                let painted = livery.filter(|_| !effect_piece && !source.cap && match tex {
                    Some(t) => t.layer & skin::CUSTOM == 0 && skin_eligible(&name, t),
                    None => color[..3].iter().copied().max().unwrap_or(0) >= 48,
                });
                let layer = layer | (1 << 28) | painted.map_or(0, |bank| skin::LIVERY | bank);
                // (The ground under the unit is shaded by TA's own faces, as it was.)
                if !source.late {
                    polygons.push(poly.iter().map(|p| rest[pi] + *p).collect());
                }
                let laid = |poly: &[Vec3], n: Vec3| {
                    match (kind, tex) {
                        (kit::FaceKind::Plating | kit::FaceKind::Picture, Some(t)) if BRUSHED.iter().any(|b| name.starts_with(b)) => brushed_uvs(poly, paint_map(t.layer)),
                        _ => None,
                    }
                    .unwrap_or_else(|| polygon_uvs(poly, n))
                };
                let uvs = source.uvs.clone().unwrap_or_else(|| match &source.mirror {
                    // (Its corners are the mirrored face's the other way round.)
                    Some(of) => {
                        let mut uvs = laid(of, face_normal(of));
                        uvs.reverse();
                        uvs
                    }
                    None => laid(poly, n),
                });
                let mut vertices = Vec::with_capacity(poly.len());
                for (k, p) in poly.iter().enumerate() {
                    let uv = uvs[k];
                    let mut edge_dist = [f16_bits(FAR_FROM_EDGES); 4];
                    for (j, e) in edges.iter().enumerate() {
                        edge_dist[j] = f16_bits((*p - e.at).dot(e.inward).max(0.0));
                    }
                    vertices.push(ModelVertex {
                        pos: p.to_array(),
                        normal,
                        uv: [
                            (uv[0].clamp(0.0, 1.0) * 65535.0) as u16,
                            (uv[1].clamp(0.0, 1.0) * 65535.0) as u16,
                        ],
                        layer,
                        color,
                        piece: pi as u32,
                        edge_dist,
                        edge_weight,
                        tread,
                    });
                    let world = rest[pi] + *p;
                    max_r2 = max_r2.max(world.x * world.x + world.z * world.z);
                    max_h = max_h.max(world.y);
                }
                source_faces.push(smooth::SourceFace {
                    vertices,
                    normal: n,
                    late: source.late,
                });
                kinds.push((kind, paint, team, tex.map(|t| t.layer)));
            }
            smooth::smooth_normals(&mut source_faces);
            for face in &source_faces {
                let start = (self.vertices.len() as i32 - base_vertex) as u32;
                self.vertices.extend_from_slice(&face.vertices);
                for k in 1..face.vertices.len() as u32 - 1 {
                    self.indices
                        .extend_from_slice(&[start, start + k + 1, start + k]);
                }
            }
            piece_ranges.push((piece_first, self.indices.len() as u32 - piece_first));
            prepared.push(Prepared { faces: source_faces, kinds, open, origins });
        }
        // The connectors after every piece's own triangles: drawn with the whole model, not
        // with one piece alone (a piece flung off as debris), nor traced (rays follow pieces).
        let start = (self.vertices.len() as i32 - base_vertex) as u32;
        self.vertices.extend_from_slice(&connectors.vertices);
        self.indices.extend(connectors.indices.iter().map(|i| i + start));
        // (Reborn lights the hand-shaded parts itself: see `smooth::even_hand_shading`.)
        {
            let mut pieces: Vec<(&mut Vec<smooth::SourceFace>, Vec3)> = prepared.iter_mut().zip(&rest).map(|(p, &offset)| (&mut p.faces, offset)).collect();
            smooth::even_hand_shading(&mut pieces, |v| v.layer & PALETTE_FACE != 0);
            // (A face mirrored from a face TA shaded by hand takes that face's colour as evened.)
            for p in prepared.iter_mut() {
                for f in 0..p.faces.len() {
                    let Some(of) = p.origins[f].filter(|_| p.faces[f].late && p.faces[f].vertices.first().is_some_and(|v| v.layer & PALETTE_FACE != 0)) else { continue };
                    let Some(src) = (0..p.faces.len()).find(|&g| !p.faces[g].late && p.origins[g] == Some(of)) else { continue };
                    let colour = p.faces[src].vertices[0].color;
                    for v in &mut p.faces[f].vertices {
                        v.color[..3].copy_from_slice(&colour[..3]);
                    }
                }
            }
        }
        // Reborn's detail, piece by piece, each seeing the whole model at rest (a face of one
        // piece lying on another's, which side of the model a track is on).
        let neighbours: Vec<kit::Neighbour> = if fit_kit {
            prepared
                .iter()
                .enumerate()
                .flat_map(|(pi, p)| p.faces.iter().filter(|f| !f.late).map(move |f| (pi, f)))
                .filter_map(|(pi, f)| kit::Neighbour::new(pi, f.vertices.iter().map(|v| Vec3::from_array(v.pos) + rest[pi]).collect(), f.normal))
                .collect()
        } else {
            Vec::new()
        };
        for (pi, piece) in prepared.iter().enumerate() {
            let pieces_kit = if fit_kit {
                let greys = paint_plain_greys(&piece.faces, &piece.kinds);
                let faces: Vec<kit::KitFace> = piece
                    .faces
                    .iter()
                    .zip(&piece.kinds)
                    .zip(&piece.open)
                    .zip(&greys)
                    .map(|(((face, &kind), open), grey)| {
                        let (vertices, (kind, paint, team, layer)) = grey.as_ref().map_or((&face.vertices[..], kind), |(v, k)| (&v[..], *k));
                        kit::KitFace { vertices, normal: face.normal, kind, paint, team, open, joined: &[], painted: layer.and_then(&paint_map), late: face.late }
                    })
                    .collect();
                let mut kit = kit::build(&faces, &kit::Around { piece: pi, offset: rest[pi], faces: &neighbours }, &mut budget);
                if rounding {
                    let (added, bowed) = rounded::round(&mut kit);
                    stats.rounded += added;
                    stats.bowed = stats.bowed.max(bowed);
                }
                kit
            } else {
                let mut whole = kit::Kit::default();
                for face in &piece.faces {
                    let start = whole.vertices.len() as u32;
                    whole.vertices.extend_from_slice(&face.vertices);
                    for k in 1..face.vertices.len() as u32 - 1 {
                        whole.indices.extend_from_slice(&[start, start + k + 1, start + k]);
                    }
                }
                whole
            };
            stats.plates += pieces_kit.plates;
            stats.bolts += pieces_kit.bolts;
            stats.wheels += pieces_kit.wheels;
            detailed.push(pieces_kit);
        }
        let pieces = model
            .pieces
            .iter()
            .zip(&piece_ranges)
            .map(|(p, &(first_index, index_count))| PieceInfo {
                name: p.name.clone(),
                parent: p.parent,
                offset: fixed(p.offset),
                first_index,
                index_count,
            })
            .collect();
        let id = self.models.len() as ModelId;
        self.models.push(ModelInfo {
            name: name.to_string(),
            first_index,
            index_count: self.indices.len() as u32 - first_index,
            base_vertex,
            pieces,
            radius: max_r2.sqrt(),
            height: max_h,
            footprint: Footprint::of(&polygons, rest.first().copied().unwrap_or(Vec3::ZERO)),
        });
        self.by_name.insert(key, id);
        stats.triangles = self.models[id as usize].index_count as usize / 3;
        stats.detailed_triangles = detailed.iter().map(|m| m.indices.len() / 3).sum();
        if stats.plates + stats.bolts + stats.wheels + stats.rounded > 0 {
            let variant = self.append_surface_variant(id, detailed, &connectors, "Reborn detail");
            self.detail.insert(id, variant);
            self.original.insert(variant, id);
        } else {
            stats.detailed_triangles = stats.triangles;
        }
        self.detail_stats.insert(id, stats);
        self.dirty = true;
        self.build_time += started.elapsed();
        id
    }

    /// The version of a model to draw: with Reborn's art, its detailed one where it is big
    /// enough on screen to show (always, with no camera: wallpapers, the showroom); the
    /// original otherwise. Every pass (colour, shadows, ray tracing, debris) draws the same
    /// choice but the water's reflection, which draws the original (see `draw_reflection`);
    /// the pieces and their order are the original's either way.
    pub(super) fn detail_for(&self, original: ModelId, root: Mat4, camera: Option<&Camera>, reborn: bool) -> ModelId {
        // (`TA_REBORN_DETAIL=0`: Reborn's paint on the original shapes, for comparisons.)
        static DETAIL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let wanted = *DETAIL.get_or_init(|| std::env::var("TA_REBORN_DETAIL").map_or(true, |v| v != "0"));
        let Some(&detail) = self.detail.get(&original).filter(|_| reborn && wanted) else {
            return original;
        };
        let info = &self.models[original as usize];
        let close = camera.is_none_or(|camera| projected_pixels(camera, root.transform_point3(Vec3::Y * info.height * 0.5), info.radius.max(info.height * 0.5)) >= DETAIL_PIXELS);
        if close { detail } else { original }
    }

    fn append_surface_variant(&mut self, original: ModelId, meshes: Vec<kit::Kit>, connectors: &connectors::Connectors, label: &str) -> ModelId {
        let mut info = self.models[original as usize].clone();
        for (mesh, pose) in meshes.iter().zip(info.rest_pose()) {
            for vertex in &mesh.vertices {
                let p = pose.transform_point3(Vec3::from_array(vertex.pos));
                info.radius = info.radius.max(p.x.hypot(p.z));
                info.height = info.height.max(p.y);
            }
        }
        info.name = format!("{} ({label})", info.name);
        info.base_vertex = self.vertices.len() as i32;
        info.first_index = self.indices.len() as u32;
        let mut offset = 0;
        for (piece, mesh) in info.pieces.iter_mut().zip(meshes) {
            piece.first_index = self.indices.len() as u32;
            piece.index_count = mesh.indices.len() as u32;
            self.indices.extend(mesh.indices.iter().map(|i| i + offset));
            offset += mesh.vertices.len() as u32;
            self.vertices.extend(mesh.vertices);
        }
        // (The connectors after the pieces, as in the original.)
        self.indices.extend(connectors.indices.iter().map(|i| i + offset));
        self.vertices.extend_from_slice(&connectors.vertices);
        info.index_count = self.indices.len() as u32 - info.first_index;
        let id = self.models.len() as ModelId;
        self.models.push(info);
        id
    }

    /// Logs what Reborn's kit made of a unit's model.
    pub fn register_unit(&self, name: &str, original: ModelId) {
        if let Some(s) = self.detail_stats.get(&original) {
            log::debug!(
                "unit model {}: {} pieces, {} triangles ({} with Reborn's detail: {} plates, {} bolts, {} wheels)",
                name.to_ascii_lowercase(),
                self.models[original as usize].pieces.len(),
                s.triangles,
                s.detailed_triangles,
                s.plates,
                s.bolts,
                s.wheels
            );
        }
    }

    /// What Reborn's kit made of a model.
    pub fn detail_stats(&self, original: ModelId) -> Option<DetailStats> {
        self.detail_stats.get(&original).copied()
    }

    fn upload(&mut self, device: &wgpu::Device) {
        if !self.dirty || self.vertices.is_empty() {
            return;
        }
        use crate::gpu::CheckedBufferInit;
        // (Ray tracing builds from them, and shades what reflections meet from them.)
        let traced = if self.blas_input {
            wgpu::BufferUsages::BLAS_INPUT | wgpu::BufferUsages::STORAGE
        } else {
            wgpu::BufferUsages::empty()
        };
        self.vertex_buffer = Some(
            device.create_buffer_checked(&wgpu::util::BufferInitDescriptor {
                label: Some("model vertices"),
                contents: bytemuck::cast_slice(&self.vertices),
                usage: wgpu::BufferUsages::VERTEX | traced,
            }),
        );
        self.index_buffer = Some(
            device.create_buffer_checked(&wgpu::util::BufferInitDescriptor {
                label: Some("model indices"),
                contents: bytemuck::cast_slice(&self.indices),
                usage: wgpu::BufferUsages::INDEX | traced,
            }),
        );
        self.dirty = false;
        log::info!(
            "model library: {} models, {} vertices, {} triangles; {} halves made whole and {} sheets given backs ({} faces mirrored), {} holes closed with {} caps ({} triangles), {} faces turned round; built in {:.0} ms with Reborn's detail",
            self.models.len(),
            self.vertices.len(),
            self.indices.len() / 3,
            self.mended.halves,
            self.mended.backed,
            self.mended.mirrored,
            self.mended.holes,
            self.mended.caps,
            self.mended.triangles,
            self.mended.flipped,
            self.build_time.as_secs_f64() * 1000.0
        );
    }

    /// The shared vertex and index buffers, once uploaded.
    pub(crate) fn buffers(&self) -> Option<(&wgpu::Buffer, &wgpu::Buffer)> {
        Some((self.vertex_buffer.as_ref()?, self.index_buffer.as_ref()?))
    }

    /// How many of its model's vertices a piece's triangles reach (from the model's first).
    pub(crate) fn piece_vertex_count(&self, model: ModelId, piece: u32) -> u32 {
        let Some(p) = self
            .models
            .get(model as usize)
            .and_then(|m| m.pieces.get(piece as usize))
        else {
            return 0;
        };
        let range = p.first_index as usize..(p.first_index + p.index_count) as usize;
        self.indices
            .get(range)
            .and_then(|i| i.iter().copied().max())
            .map_or(0, |m| m + 1)
    }
}

/// A piece whose only face is a flat, untextured quad facing straight down, the model's root or
/// lying on the ground it stands on (`at`: the piece's height at rest over the model's origin):
/// the selection plate the model's file forgot to flag as one (the Core Mine's `ground`, where
/// its sister mines flag theirs), or a plate under a mod's unit (TA: Escalation's Commanders'
/// `flip`, 108 units square, the point the sparks of their arrival burst from). Never seen from
/// above, it would still cast its square shadow (the shadow pass draws both sides of a face) and
/// darken the ground round the unit as what stands there does (its `Footprint`): a dark square on
/// the ground round the mine, and black wedges and a dark square 108 units across round TA:
/// Escalation's Commanders.
fn unflagged_ground_plate(piece: &ta_formats::model3do::Piece, at: f32) -> Option<usize> {
    let [prim] = piece.primitives.as_slice() else { return None };
    if prim.texture.is_some() || prim.indices.len() != 4 {
        return None;
    }
    // (As `ModelLibrary::add` reads the vertices: z turned round.)
    let corners: Vec<Vec3> = prim.indices.iter().map(|&i| piece.vertices.get(i as usize).map(|v| Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / FIXED_ONE)).collect::<Option<_>>()?;
    let flat = corners.iter().all(|c| (c.y - corners[0].y).abs() < 0.01);
    // (Higher up, a plate under a hull or a wing can be seen from a low camera; on the ground,
    // never.)
    let on_ground = piece.parent.is_none() || (at + corners[0].y).abs() <= GROUND_PLATE_HEIGHT;
    (flat && on_ground && face_normal(&corners).y < -0.99).then_some(0)
}

/// How far from the model's origin (which stands on the ground) a plate under a piece other
/// than the root may lie and still be a ground plate (see [`unflagged_ground_plate`]).
const GROUND_PLATE_HEIGHT: f32 = 0.5;

/// Newell's method: a robust normal for possibly non-planar polygons. The z flip (see
/// `ModelLibrary::add`) mirrors the winding, so it is negated to point outwards.
fn face_normal(poly: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        n += Vec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    -n.normalize_or(Vec3::Y)
}

/// A polygon of a piece (piece space), for the edge wear.
struct Face {
    corners: Vec<Vec3>,
    normal: Vec3,
}

/// The grid a logo is laid on (see `even_grid`): this many small quads each way.
const EVEN_GRID: usize = 6;

/// A logo on a four-cornered face that is no parallelogram (a trapezoid, the Krogoth's back
/// pack's): TA stretched the picture evenly across the quad, its corners the picture's, line by
/// line; drawn as two triangles it bends along their diagonal, sheared. Such a face becomes a
/// grid of small quads, each corner given its place in the picture as on the evenly stretched
/// quad (bilinearly), so what bend is left is too small to see. (Logos only: Reborn's kit keeps
/// them as they are, where it fits plates to the faces of others.) Other faces stay as they are.
fn even_grid<'a>(source: FaceSource<'a>, face: Face) -> Vec<(FaceSource<'a>, Face)> {
    let logo = source.texture.is_some_and(|t| t.to_ascii_lowercase().contains("logo"));
    let c = &face.corners;
    if !logo || c.len() != 4 || source.mirror.is_some() {
        return vec![(source, face)];
    }
    // (A parallelogram's diagonals halve each other: two triangles lay it evenly already.)
    let skew = ((c[0] + c[2]) - (c[1] + c[3])).length();
    if skew < 0.05 * (c[2] - c[0]).length().max((c[3] - c[1]).length()) {
        return vec![(source, face)];
    }
    let uvs = source.uvs.clone().unwrap_or_else(|| polygon_uvs(c, face.normal));
    let weights = |s: f32, t: f32| [(1.0 - s) * (1.0 - t), s * (1.0 - t), s * t, (1.0 - s) * t];
    let at = |s: f32, t: f32| weights(s, t).iter().zip(c).fold(Vec3::ZERO, |acc, (w, p)| acc + *p * *w);
    let uv = |s: f32, t: f32| {
        let w = weights(s, t);
        [0, 1].map(|k| (0..4).map(|i| uvs[i][k] * w[i]).sum::<f32>())
    };
    let step = 1.0 / EVEN_GRID as f32;
    let mut out = Vec::with_capacity(EVEN_GRID * EVEN_GRID);
    for i in 0..EVEN_GRID {
        for j in 0..EVEN_GRID {
            let (s0, s1, t0, t1) = (i as f32 * step, (i + 1) as f32 * step, j as f32 * step, (j + 1) as f32 * step);
            let corners = vec![at(s0, t0), at(s1, t0), at(s1, t1), at(s0, t1)];
            let laid = vec![uv(s0, t0), uv(s1, t0), uv(s1, t1), uv(s0, t1)];
            out.push((FaceSource { uvs: Some(laid), ..source.clone() }, Face { corners, normal: face.normal }));
        }
    }
    out
}

/// One of a face's edges that the edge wear shows: a point on it, the direction into the face
/// across it, and how sharp it is (positive: a convex edge; negative: a crease).
#[derive(Clone, Copy, Debug)]
struct WornEdge {
    at: Vec3,
    inward: Vec3,
    weight: f32,
}

/// Distance that stands for "no edge near" in `ModelVertex::edge_dist`.
const FAR_FROM_EDGES: f32 = 4096.0;

/// The edges of each face that the edge wear shows (at most four, the sharpest). An edge's
/// sharpness comes from the face across it in the same piece: none where the two are nearly
/// flat, the more the sharper the fold, negative where the surface folds inwards (a crease);
/// an edge with no face across it (a plate's rim, where a piece meets another) is mildly
/// convex. The face across is the one sharing the edge's corners (to an eighth of a unit:
/// TA's models repeat vertices rather than share them), or else one whose plane the edge lies
/// in (a face meeting another mid-way: a turret's side on a hull's top). Works on any model,
/// so on every mod's too. A face of TA's own sees no `late` face (one added to make a form
/// whole, see `FaceSource::late`): its edges wear as they did.
/// Faces in a piece past which an edge with no face sharing it is not looked for along every
/// other face (as smoothing, mending and making forms whole leave such pieces be).
const DENSE_FACES: usize = 4096;

fn worn_edges(faces: &[&Face], late: &[bool]) -> Vec<Vec<WornEdge>> {
    // Joints flatter than this (the cosine between the faces' normals) show no edge.
    const FLAT: f32 = 0.96;
    const RIM: f32 = 0.45;
    let key = |p: Vec3| {
        [
            (p.x * 8.0).round() as i32,
            (p.y * 8.0).round() as i32,
            (p.z * 8.0).round() as i32,
        ]
    };
    let edge_key = |a: Vec3, b: Vec3| {
        let (ka, kb) = (key(a), key(b));
        if ka <= kb { (ka, kb) } else { (kb, ka) }
    };
    let centres: Vec<Vec3> = faces
        .iter()
        .map(|f| f.corners.iter().copied().sum::<Vec3>() / f.corners.len() as f32)
        .collect();
    // A face's own back (some models have the same polygon facing the other way).
    let twin = |i: usize, j: usize| {
        faces[i].normal.dot(faces[j].normal) < -0.95 && centres[i].distance(centres[j]) < 0.5
    };
    let mut by_edge: HashMap<([i32; 3], [i32; 3]), Vec<usize>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        let n = f.corners.len();
        for k in 0..n {
            by_edge
                .entry(edge_key(f.corners[k], f.corners[(k + 1) % n]))
                .or_default()
                .push(i);
        }
    }
    // Whether `p` lies on face `g` (in its plane, inside its outline), to a tolerance.
    let on_face = |g: &Face, p: Vec3| {
        if (p - g.corners[0]).dot(g.normal).abs() > 0.2 {
            return false;
        }
        let n = g.corners.len();
        let centre = g.corners.iter().copied().sum::<Vec3>() / n as f32;
        (0..n).all(|k| {
            let (a, b) = (g.corners[k], g.corners[(k + 1) % n]);
            let mut inward = g.normal.cross(b - a).normalize_or_zero();
            if (centre - a).dot(inward) < 0.0 {
                inward = -inward;
            }
            (p - a).dot(inward) > -0.25
        })
    };
    faces
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let n = f.corners.len();
            let mut edges: Vec<WornEdge> = (0..n)
                .filter_map(|k| {
                    let (a, b) = (f.corners[k], f.corners[(k + 1) % n]);
                    let along = b - a;
                    if along.length_squared() < 1e-6 {
                        return None;
                    }
                    let mut inward = f.normal.cross(along).normalize_or_zero();
                    if (centres[i] - a).dot(inward) < 0.0 {
                        inward = -inward;
                    }
                    let shared: Vec<usize> = by_edge[&edge_key(a, b)]
                        .iter()
                        .copied()
                        .filter(|&j| j != i && !twin(i, j) && !(late[j] && !late[i]))
                        .collect();
                    // (A joint the smoothing rounds, two faces sharing it at a shallow fold, is
                    // part of a curved surface, a barrel's or a dome's: no edge to wear.)
                    let curved = shared.len() == 1 && f.normal.dot(faces[shared[0]].normal) >= smooth::SHALLOW;
                    // (A face lying along it is looked for among them all: not in a dense mod
                    // model, where that took minutes.)
                    let across: Vec<usize> = if shared.is_empty() && faces.len() <= DENSE_FACES {
                        let mid = (a + b) * 0.5;
                        (0..faces.len())
                            .filter(|&j| {
                                j != i && !twin(i, j) && !(late[j] && !late[i]) && faces[j].normal.dot(f.normal) < FLAT
                            })
                            .filter(|&j| {
                                on_face(faces[j], a)
                                    && on_face(faces[j], b)
                                    && on_face(faces[j], mid)
                            })
                            .collect()
                    } else {
                        shared
                    };
                    // The sharpest joint. It folds outwards (a convex edge) when the face across
                    // turns away from this face's inside, inwards (a crease) when it faces it.
                    let weight = match across.iter().filter(|_| !curved).min_by(|&&x, &&y| {
                        f.normal
                            .dot(faces[x].normal)
                            .total_cmp(&f.normal.dot(faces[y].normal))
                    }) {
                        None if curved => 0.0,
                        None => RIM,
                        Some(&j) => {
                            let cos = f.normal.dot(faces[j].normal);
                            if cos > FLAT {
                                0.0
                            } else {
                                let sharp = ((1.0 - cos) * 1.6).clamp(0.3, 1.0);
                                if inward.dot(faces[j].normal) < 0.0 {
                                    sharp
                                } else {
                                    -0.8 * sharp
                                }
                            }
                        }
                    };
                    (weight != 0.0).then_some(WornEdge {
                        at: a,
                        inward,
                        weight,
                    })
                })
                .collect();
            edges.sort_by(|x, y| y.weight.abs().total_cmp(&x.weight.abs()));
            edges.truncate(4);
            edges
        })
        .collect()
}

/// For each face, which of its edges are open: met by no other face of the piece (none sharing
/// it, none it lies on, as a turret's side stands on a hull's top, none running along it).
/// Open edges are common in TA's models (a track pod with no inner side, a fin, a muzzle
/// brake's sides): Reborn's plates raise nothing there, or it would show past them. (A sheet
/// given its back, see halves.rs, the faces `back`, keeps its plates as they were: its back
/// meets none of its edges; and a face of TA's own sees no `late` face, see
/// `FaceSource::late`.)
fn open_edges(faces: &[&Face], back: &[bool], late: &[bool]) -> Vec<Vec<bool>> {
    let key = |p: Vec3| [(p.x * 8.0).round() as i32, (p.y * 8.0).round() as i32, (p.z * 8.0).round() as i32];
    let edge_key = |a: Vec3, b: Vec3| {
        let (ka, kb) = (key(a), key(b));
        if ka <= kb { (ka, kb) } else { (kb, ka) }
    };
    let mut by_edge: HashMap<([i32; 3], [i32; 3]), Vec<usize>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        let n = f.corners.len();
        for k in 0..n {
            by_edge.entry(edge_key(f.corners[k], f.corners[(k + 1) % n])).or_default().push(i);
        }
    }
    // Whether `p` is on face `g`'s outline or inside it, in its plane.
    let on_face = |g: &Face, p: Vec3| {
        if (p - g.corners[0]).dot(g.normal).abs() > 0.2 {
            return false;
        }
        let n = g.corners.len();
        let centre = g.corners.iter().copied().sum::<Vec3>() / n as f32;
        (0..n).all(|k| {
            let (a, b) = (g.corners[k], g.corners[(k + 1) % n]);
            let mut inward = g.normal.cross(b - a).normalize_or_zero();
            if (centre - a).dot(inward) < 0.0 {
                inward = -inward;
            }
            (p - a).dot(inward) > -0.25
        })
    };
    // Whether `p` lies on one of face `g`'s edges.
    let on_edge = |g: &Face, p: Vec3| {
        let n = g.corners.len();
        (0..n).any(|k| {
            let (a, b) = (g.corners[k], g.corners[(k + 1) % n]);
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
            (a + ab * t).distance(p) < 0.15
        })
    };
    // (A sheet's back given it: the same corners facing the other way.)
    let centres: Vec<Vec3> = faces.iter().map(|f| f.corners.iter().copied().sum::<Vec3>() / f.corners.len().max(1) as f32).collect();
    let twin = |i: usize, j: usize| (back[i] || back[j]) && faces[i].normal.dot(faces[j].normal) < -0.95 && centres[i].distance(centres[j]) < 0.05 && faces[i].corners.len() == faces[j].corners.len();
    faces
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let n = f.corners.len();
            (0..n)
                .map(|k| {
                    let (a, b) = (f.corners[k], f.corners[(k + 1) % n]);
                    if by_edge[&edge_key(a, b)].iter().any(|&j| j != i && !twin(i, j) && !(late[j] && !late[i])) {
                        return false;
                    }
                    // (A dense mod model's: taken for open, no face looked for along it.)
                    if faces.len() > DENSE_FACES {
                        return true;
                    }
                    let mid = (a + b) * 0.5;
                    !faces.iter().enumerate().any(|(j, g)| {
                        j != i
                            && !twin(i, j)
                            && !(late[j] && !late[i])
                            && (g.normal.dot(f.normal) < 0.96 && on_face(g, a) && on_face(g, b) && on_face(g, mid)
                                || on_edge(g, mid) && on_edge(g, a.lerp(b, 0.25)) && on_edge(g, a.lerp(b, 0.75)))
                    })
                })
                .collect()
        })
        .collect()
}

/// `x` as IEEE half-precision bits (the vertex format `Float16`); small numbers flush to zero,
/// large ones to the largest half.
fn f16_bits(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32 - 127 + 15;
    if exp <= 0 {
        return sign;
    }
    if exp >= 31 {
        return sign | 0x7BFF;
    }
    sign | ((exp as u16) << 10) | ((bits & 0x7F_FFFF) >> 13) as u16
}

/// A piece that is an effect rather than a part: a muzzle's flash, where a weapon fires from,
/// where a beam or a nanolathe's spray comes from (TA's scripts show and hide these, or only
/// use their place). Pieces named for a nanolathe are parts (the factories' nanolathe towers,
/// a Commander's arm): its spray points have no faces.
fn is_effect_piece(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["flare", "fire", "muzzle", "flash", "emit", "beam", "spray"].iter().any(|k| n.starts_with(k)) || n.contains("flare") || n.contains("flash")
}

/// The layer word's bit for one of TA's faces painted in a palette colour (see `ModelVertex`).
const PALETTE_FACE: u32 = 1 << 27;

/// What a face is to the kit: its kind, its paint's colour, whether a player's colour, its
/// texture's layer.
type FaceRole = (kit::FaceKind, [u8; 3], bool, Option<u32>);

/// TA's plain grey faces among its painted ones (a Krogoth's claws: each a painted face and two
/// grey sides): TA shaded them by hand, each the grey its light would give it, and among Reborn's
/// plating they showed as flat, unfinished cream. In the detailed version such a face (a neutral
/// grey, not dark, a recess, nor coloured, a glow or a lamp; no bigger than the face it meets;
/// meeting no other plain face, as the facets of a part shaded by hand do, see
/// `smooth::even_hand_shading`) takes the paint of a painted face of its piece it meets along an
/// edge, laid across it (one plate, unbolted, as before): Reborn's light shades it. Each face's
/// corners and role so, or None.
fn paint_plain_greys(faces: &[smooth::SourceFace], roles: &[FaceRole]) -> Vec<Option<(Vec<ModelVertex>, FaceRole)>> {
    let key = |v: &ModelVertex| v.pos.map(|c| (c * 64.0).round() as i32);
    let area = |f: &smooth::SourceFace| {
        let p: Vec<Vec3> = f.vertices.iter().map(|v| Vec3::from_array(v.pos)).collect();
        (2..p.len()).map(|k| (p[k - 1] - p[0]).cross(p[k] - p[0]).length() * 0.5).sum::<f32>()
    };
    let grey = |f: &smooth::SourceFace, role: &FaceRole| {
        let v = &f.vertices[0];
        let [r, g, b, _] = v.color;
        let (lo, hi) = (r.min(g).min(b), r.max(g).max(b));
        role.0 == kit::FaceKind::Picture && role.3.is_none() && v.layer & PALETTE_FACE != 0 && hi - lo <= 12 && (64..=215).contains(&hi)
    };
    let meets = |a: &smooth::SourceFace, corners: &[[i32; 3]]| a.vertices.iter().filter(|v| corners.contains(&key(v))).count() >= 2;
    faces
        .iter()
        .zip(roles)
        .map(|(face, role)| {
            if !grey(face, role) {
                return None;
            }
            let corners: Vec<[i32; 3]> = face.vertices.iter().map(key).collect();
            // (A late face is nothing to TA's own: see `FaceSource::late`.)
            let seen = |other: &smooth::SourceFace| face.late || !other.late;
            if faces.iter().any(|other| !std::ptr::eq(other, face) && seen(other) && other.vertices[0].layer & PALETTE_FACE != 0 && meets(other, &corners)) {
                return None;
            }
            let size = area(face);
            let (painted, painted_role) = faces
                .iter()
                .zip(roles)
                .find(|&(other, other_role)| seen(other) && other_role.0 == kit::FaceKind::Plating && !other_role.2 && other_role.3.is_some() && size <= area(other) * 1.5 && meets(other, &corners))?;
            let like = painted.vertices[0];
            let positions: Vec<Vec3> = face.vertices.iter().map(|v| Vec3::from_array(v.pos)).collect();
            let uvs = polygon_uvs(&positions, face.normal);
            let vertices = face
                .vertices
                .iter()
                .zip(uvs)
                .map(|(v, uv)| ModelVertex {
                    uv: uv.map(|c| (c.clamp(0.0, 1.0) * 65535.0) as u16),
                    layer: like.layer,
                    color: like.color,
                    normal: [v.normal[0], v.normal[1], v.normal[2], like.normal[3]],
                    ..*v
                })
                .collect();
            Some((vertices, (kit::FaceKind::Picture, painted_role.1, painted_role.2, painted_role.3)))
        })
        .collect()
}

/// TA's functional markings, painted as one picture over several faces (landing circles,
/// runways, factory and air pads, the Core emblem pads): plates would cut through them, so they
/// keep TA's paint whole. By the textures' names, lower case, as prefixes.
const MARKINGS: [&str; 6] = ["helipad", "corse11", "armcam1b", "corcl03", "armplat02", "corebridge"];
/// Faces painted with a picture of their own that a grid of plates would cut (Core's air plant
/// panels, its emblem panels): one plate each at most.
const PICTURES: [&str; 2] = ["coreap0", "corv05"];

/// The player colour TA paints a team texture's first frame in (its blue).
const FIRST_TEAM: [u8; 3] = [48, 92, 220];

/// A team texture's first frame's mean colour as the grey that each player's colour multiplies
/// to look as bright: the frame's brightness next to the colour it was painted in.
fn team_grey(mean: [u8; 3], lut: &[f32; 256]) -> [u8; 3] {
    let lum = |c: [u8; 3]| 0.2126 * lut[c[0] as usize] + 0.7152 * lut[c[1] as usize] + 0.0722 * lut[c[2] as usize];
    [linear_to_srgb(lum(mean) / lum(FIRST_TEAM)); 3]
}

/// Whether a texture paints a track (TA's `Tredside1` and `Tredside2`; mods' treads and
/// tracks by their names), whose links run along the texture's u.
fn is_tread_texture(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("tredside") || n.contains("tread") || (n.contains("track") && !n.contains("tracker"))
}

/// A track face's tread (see `ModelVertex::tread`): how fast u runs as the face moves forward,
/// from the face's u axis (its first edge, as `polygon_uvs` lays the texture), and how far to
/// the side it is (`offset`: the piece's place in the model).
fn tread_of(poly: &[Vec3], offset: Vec3) -> [f32; 2] {
    if poly.len() < 2 {
        return [0.0; 2];
    }
    let (along, length) = if poly.len() <= 4 {
        let e = poly[1] - poly[0];
        (e.normalize_or_zero(), e.length())
    } else {
        let axis = (poly[1] - poly[0]).normalize_or(Vec3::X);
        let (lo, hi) = poly
            .iter()
            .map(|p| (*p - poly[0]).dot(axis))
            .fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(d), hi.max(d)));
        (axis, hi - lo)
    };
    if length < 0.5 {
        return [0.0; 2];
    }
    // u gains 1 over the edge: moving forward (+z) by one unit moves it along by z / length.
    let rate = along.z / length;
    let centre = poly.iter().copied().sum::<Vec3>() / poly.len() as f32 + offset;
    [rate, centre.x]
}

/// TA's chrome, painted with the sky's reflections in it (the Core's sheet metal: its tanks'
/// track covers), by the textures' names, lower case, as prefixes: see `brushed_uvs`.
const BRUSHED: [&str; 7] = ["corsea1a", "corsea1b", "corsea1c", "corsea1d", "corsea2", "corsea3", "corsea12"];

/// TA maps a texture's corners onto a quad's corners whatever its shape: on a long, narrow face
/// (the Goliath's track covers, 31 units by 6) the paint is drawn out five times along it, and
/// chrome's painted reflections smear into streaks like a blurred photograph. Laid evenly they
/// repeat, laid as a band of the paint they are its texels blown up. Such a face (its texels
/// drawn out three times or more, its paint changing along the way it is drawn out) is laid
/// instead with the few lines of its paint across it that draw out best (see
/// `PaintMap::stripe_lines`), running its whole length: TA's own colours as brushed metal,
/// polished along the cover.
fn brushed_uvs(poly: &[Vec3], paint: Option<&kit::PaintMap>) -> Option<Vec<[f32; 2]>> {
    const STRETCH: f32 = 3.0;
    const STREAKY: f32 = 0.012;
    let paint = paint?;
    if poly.len() != 4 {
        return None;
    }
    // (u runs along corners 0 to 1 and 3 to 2, v along 1 to 2 and 0 to 3: texels a unit each
    // way, the texture's own shape counted.)
    let lu = (poly[1].distance(poly[0]) + poly[2].distance(poly[3])) * 0.5;
    let lv = (poly[2].distance(poly[1]) + poly[3].distance(poly[0])) * 0.5;
    let (w, h) = paint.size();
    let (du, dv) = (w as f32 / lu.max(1e-3), h as f32 / lv.max(1e-3));
    let along_v = dv < du;
    if du.max(dv) / du.min(dv).max(1e-6) < STRETCH || paint.streaky(along_v) < STREAKY {
        return None;
    }
    let lines = 2;
    let (start, span) = if along_v { (paint.stripe_lines(true, lines), lines as f32 / h as f32) } else { (paint.stripe_lines(false, lines), lines as f32 / w as f32) };
    Some(if along_v {
        vec![[0.0, start], [1.0, start], [1.0, start + span], [0.0, start + span]]
    } else {
        vec![[start, 0.0], [start + span, 0.0], [start + span, 1.0], [start, 1.0]]
    })
}

/// TA maps a texture's corners onto a quad's vertices in order; triangles use the first
/// three corners. Larger polygons get a planar projection onto their bounding rectangle.
fn polygon_uvs(poly: &[Vec3], normal: Vec3) -> Vec<[f32; 2]> {
    const CORNERS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    match poly.len() {
        3 | 4 => (0..poly.len()).map(|i| CORNERS[i]).collect(),
        _ => {
            let u_axis = (poly[1] - poly[0]).normalize_or(Vec3::X);
            let v_axis = normal.cross(u_axis).normalize_or(Vec3::Z);
            let coords: Vec<[f32; 2]> = poly
                .iter()
                .map(|p| [(*p - poly[0]).dot(u_axis), (*p - poly[0]).dot(v_axis)])
                .collect();
            let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
            for c in &coords {
                for k in 0..2 {
                    min[k] = min[k].min(c[k]);
                    max[k] = max[k].max(c[k]);
                }
            }
            coords
                .iter()
                .map(|c| {
                    [
                        (c[0] - min[0]) / (max[0] - min[0]).max(1e-3),
                        (c[1] - min[1]) / (max[1] - min[1]).max(1e-3),
                    ]
                })
                .collect()
        }
    }
}

/// Where a face of a piece comes from: one of the model's own (turned round where it was wound
/// against its neighbours), one mirrored from one of them (see halves.rs) or a cap closing a
/// hole (see caps.rs).
#[derive(Clone)]
struct FaceSource<'a> {
    /// Its texture's name, and its colour where it has none (or the texture is missing): the
    /// palette's, or for a cap dark metal.
    texture: Option<&'a str>,
    color_index: u32,
    cap: bool,
    /// Its corners' texture coordinates where they aren't TA's (see `polygon_uvs`): a face turned
    /// round keeps its paint where it was.
    uvs: Option<Vec<[f32; 2]>>,
    /// For a face mirrored from one of the model's (see halves.rs), that face's corners: its
    /// paint is laid as on that face, mirrored.
    mirror: Option<Vec<Vec3>>,
    /// The back given a sheet TA built as one side (see halves.rs): the sheet's edges stay open
    /// to Reborn's plates.
    back: bool,
    /// Added to make a form whole (a face mirrored, a sheet's back, a rim's cap or a muzzle's,
    /// see halves.rs and caps.rs): drawn after TA's own faces, which are shaded and fitted as if
    /// it weren't there (their edges' wear, their plates and rims, their smoothed corners, their
    /// hand shading), so that they look exactly as they did, TA's view from above with them.
    late: bool,
    /// The model's face it is, or mirrors (by its place among the piece's faces as TA drew them).
    of: Option<usize>,
}

/// Long flat sheets standing alone in a palette colour (a sea serpent's frills: no face meets any
/// of their edges but their own back): TA drew each as a face, or a face and its back, and seen
/// edge-on, as a low camera sees them, such a sheet is a hairline broken into dashes running out
/// of the model. Each becomes a thin lens of the same outline, its two sides rising to a point
/// over its middle (`LENS` of its length thick there): edge-on a slim blade, from above as TA
/// drew it.
fn thicken_lone_sheets(faces: &mut Vec<(FaceSource, Face)>) {
    // (Shorter sheets, a missile's fins, are never seen so close.)
    const LONG: f32 = 14.0;
    const LENS: f32 = 0.05;
    let key = |p: Vec3| [(p.x * 8.0).round() as i32, (p.y * 8.0).round() as i32, (p.z * 8.0).round() as i32];
    let edge_key = |a: Vec3, b: Vec3| {
        let (ka, kb) = (key(a), key(b));
        if ka <= kb { (ka, kb) } else { (kb, ka) }
    };
    let corners_of = |f: &Face| {
        let mut k: Vec<[i32; 3]> = f.corners.iter().map(|&p| key(p)).collect();
        k.sort();
        k
    };
    let mut by_edge: HashMap<([i32; 3], [i32; 3]), Vec<usize>> = HashMap::new();
    for (i, (_, f)) in faces.iter().enumerate() {
        let n = f.corners.len();
        for k in 0..n {
            by_edge.entry(edge_key(f.corners[k], f.corners[(k + 1) % n])).or_default().push(i);
        }
    }
    let mut gone = vec![false; faces.len()];
    let mut lenses: Vec<(u32, Vec<Vec3>)> = Vec::new();
    for i in 0..faces.len() {
        let (source, f) = &faces[i];
        let n = f.corners.len();
        if gone[i] || source.texture.is_some() || source.cap || source.late || n < 3 {
            continue;
        }
        let long = (0..n).map(|k| f.corners[k].distance(f.corners[(k + 1) % n])).fold(0.0f32, f32::max);
        let area = (1..n - 1).map(|k| (f.corners[k] - f.corners[0]).cross(f.corners[k + 1] - f.corners[0])).sum::<Vec3>().length() * 0.5;
        // (A sliver: no broader than four tenths of its length.)
        if long < LONG || 2.0 * area / long > long * 0.4 {
            continue;
        }
        let mine = corners_of(f);
        let mut twins = Vec::new();
        let mut lone = true;
        for k in 0..n {
            for &o in &by_edge[&edge_key(f.corners[k], f.corners[(k + 1) % n])] {
                if o == i {
                    continue;
                }
                // (Its back, TA's or given it, goes with it; a late face is nothing to it.)
                if corners_of(&faces[o].1) == mine {
                    twins.push(o);
                } else if !faces[o].0.late {
                    lone = false;
                }
            }
        }
        if !lone {
            continue;
        }
        gone[i] = true;
        for o in twins {
            gone[o] = true;
        }
        let middle = f.corners.iter().copied().sum::<Vec3>() / n as f32;
        let rise = f.normal * long * LENS * 0.5;
        for k in 0..n {
            let (a, b) = (f.corners[k], f.corners[(k + 1) % n]);
            lenses.push((source.color_index, vec![a, b, middle + rise]));
            lenses.push((source.color_index, vec![b, a, middle - rise]));
        }
    }
    if lenses.is_empty() {
        return;
    }
    let mut k = 0;
    faces.retain(|_| {
        k += 1;
        !gone[k - 1]
    });
    faces.extend(lenses.into_iter().map(|(color_index, corners)| {
        let normal = face_normal(&corners);
        (FaceSource { texture: None, color_index, cap: false, uvs: None, mirror: None, back: false, late: false, of: None }, Face { corners, normal })
    }));
}

/// What caps are made of: dark metal (sRGB colour, metallic, roughness), an underside in shade.
/// (The paint of the faces round a hole, stretched over it, reads as a stray picture.)
const CAP_METAL: ([u8; 3], f32, f32) = ([58, 58, 55], 0.55, 0.62);

/// `TA_MEND=0`: TA's models as they are, holes and all (for comparisons).
fn mending() -> bool {
    static MEND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *MEND.get_or_init(|| std::env::var("TA_MEND").map_or(true, |v| v != "0"))
}

/// `TA_FORMS=0`: TA's forms as it built them, for comparisons: barrels and booms left half,
/// sheets one-sided, shallow shells' rims open (their other holes capped as before; `TA_MEND=0`
/// leaves them open too). See halves.rs and caps.rs.
fn whole_forms() -> bool {
    static WHOLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WHOLE.get_or_init(|| std::env::var("TA_FORMS").map_or(true, |v| v != "0"))
}

/// `TA_SHOW_BACKS=1`: the models' back faces drawn in magenta where nothing of the model is in
/// front of them: the holes a 3D camera looks through into a shell, or past a sheet seen from
/// behind (with `--demo gallery` and `TA_GALLERY_TOUR`, every model's; see unit.wgsl).
fn show_backs() -> bool {
    static SHOW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SHOW.get_or_init(|| std::env::var("TA_SHOW_BACKS").is_ok_and(|v| v == "1"))
}

/// What the mending did to the models loaded (see halves.rs and caps.rs), for the log.
#[derive(Clone, Copy, Debug, Default)]
struct MendStats {
    halves: usize,
    backed: usize,
    mirrored: usize,
    holes: usize,
    caps: usize,
    triangles: usize,
    flipped: usize,
}

impl MendStats {
    fn add(&mut self, mendings: &[caps::Mending]) {
        for m in mendings {
            self.holes += m.holes.iter().filter(|h| h.verdict == caps::Verdict::Capped).count();
            self.caps += m.caps.len();
            self.triangles += m.caps.iter().map(|c| c.triangles.len()).sum::<usize>();
            self.flipped += m.flipped.len();
        }
    }

    fn add_halves(&mut self, halves: &[halves::Halves]) {
        for h in halves {
            self.halves += h.shells.iter().filter(|s| matches!(s.verdict, halves::Verdict::Boxed | halves::Verdict::Rounded)).count();
            self.backed += h.shells.iter().filter(|s| s.verdict == halves::Verdict::Backed).count();
            self.mirrored += h.mirrored.len();
        }
    }
}

/// A building under construction is a hologram until about this far built (see `shade_model` in
/// unit.wgsl): it casts no shadow before then (placement ghosts and plans never do).
const HOLOGRAM_SHADOWLESS: f32 = 0.5;

/// Instances from which their level of detail is chosen on all cores.
const PARALLEL_INSTANCES: usize = 512;
/// Piece matrices from which they are written on all cores.
const PARALLEL_MATRICES: usize = 8192;

/// One model instance to draw this frame.
pub struct DrawInstance<'a> {
    pub model: ModelId,
    /// World matrices of every piece (already including the unit transform). Hidden pieces
    /// should use a zero matrix. With `piece` set, just that piece's matrix.
    pub pieces: &'a [Mat4],
    pub team: u32,
    pub flags: u32,
    pub build: f32,
    /// How damaged it is, 0 (whole) to 1 (about to go), for the battle damage (HD surfaces).
    pub damage: f32,
    /// Draw only this piece of the model (flying debris): one matrix, one piece's triangles.
    pub piece: Option<u32>,
    /// How far it has driven (model units, forwards positive) and turned (radians, towards
    /// +x positive) all told: its treads run by these (zero for things without).
    pub travel: [f32; 2],
    /// Which pieces' animated textures (TA's flashing lamps and glows) play, bit n for piece n
    /// (the pieces past 63 sharing bit 63): a unit's are those its script has `dont-cache`d
    /// (`ta_sim::Unit::textures_playing`); the others hold their first frame, as TA drew them.
    /// [`PLAYING_ALL`] for things without a script.
    pub playing: u64,
}

/// A piece's world matrix as the shaders read it: the three rows of a 3×4 affine matrix.
type Rows = [[f32; 4]; 3];

/// `m` as the shaders read it: 3 rows of a 3x4 affine matrix (the transpose of its top 3x4).
fn rows(m: &Mat4) -> Rows {
    let t = m.transpose();
    [t.x_axis.to_array(), t.y_axis.to_array(), t.z_axis.to_array()]
}

/// What the ray tracer knows of an instance's `piece` beside its paint (raytrace.rs
/// `TracedInstance::state`): bit 0, its animated textures play; bits 8-15, how far its unit's
/// lights are out.
fn traced_state(inst: &InstanceData, piece: u32) -> u32 {
    let playing = inst.playing[0] as u64 | (inst.playing[1] as u64) << 32;
    piece_plays(playing, piece) as u32 | (inst.flags >> FLAG_LIGHTS_OUT_SHIFT) << 8
}

/// Writes a frame's model matrices into `out` (each a 3×4 matrix's rows, as bytes): the whole
/// models' first, in `order`, each its run of `list[k].pieces` (on all cores for an army), then
/// the single pieces' at their places past `total`, the gaps zero.
fn write_matrices(out: wgpu::WriteOnly<'_, [[u8; 48]]>, order: &[u32], list: &[DrawInstance<'_>], single: &[(usize, &Mat4)], total: usize) {
    use rayon::prelude::*;
    let matrix = |m: &Mat4| bytemuck::cast::<Rows, [u8; 48]>(rows(m));
    let (mut rest, mut tail) = out.split_at(total);
    if total >= PARALLEL_MATRICES {
        // A write-only slice of the upload is not `Send`: each thread is given the place of its
        // run instead, the runs one after another in `rest`.
        struct Run(std::ptr::NonNull<[u8; 48]>, usize);
        // SAFETY: the runs are disjoint parts of `rest`, which lives on (untouched) until all
        // are written, each run by one thread alone.
        unsafe impl Send for Run {}
        let base = rest.as_raw_element_ptr();
        let mut at = 0;
        let runs: Vec<(&[Mat4], Run)> = order
            .iter()
            .map(|&k| {
                let pieces = list[k as usize].pieces;
                // SAFETY: `at + pieces.len()` stays within `rest`, `total` long (the runs fill it).
                let run = Run(unsafe { base.add(at) }, pieces.len());
                at += pieces.len();
                (pieces, run)
            })
            .collect();
        debug_assert_eq!(at, total);
        runs.into_par_iter().with_min_len(64).for_each(|(pieces, Run(start, len))| {
            // SAFETY: as above: valid for writes, aligned (bytes), no one else's while written.
            let out = unsafe { wgpu::WriteOnly::new(std::ptr::NonNull::slice_from_raw_parts(start, len)) };
            out.write_iter(pieces.iter().map(matrix));
        });
    } else {
        for &k in order {
            let pieces = list[k as usize].pieces;
            let (here, after) = rest.split_at(pieces.len());
            here.write_iter(pieces.iter().map(matrix));
            rest = after;
        }
    }
    tail.fill([0; 48]);
    for &(at, m) in single {
        tail.slice(at - total..=at - total).copy_from_slice(&[matrix(m)]);
    }
}

/// Per-frame instance batching and the unit pipeline.
pub struct ModelRenderer {
    pub library: ModelLibrary,
    pub textures: TextureLibrary,
    art_style_reborn: bool,
    pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    /// Models mirrored in the water (reflection.rs).
    reflection_pipeline: wgpu::RenderPipeline,
    /// `TA_SHOW_BACKS=1`'s (see `show_backs`).
    backs_pipeline: Option<wgpu::RenderPipeline>,
    /// Ray tracing's depth prepass (raytrace.rs): depth only, single-sampled.
    prepass_pipeline: Option<wgpu::RenderPipeline>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    matrices: Vec<Rows>,
    instances: Vec<InstanceData>,
    /// (model, single piece or the whole model, first instance, instance count).
    batches: Vec<(ModelId, Option<u32>, u32, u32)>,
    whole_groups: batching::InstanceGroups,
    matrix_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// This frame's instances that occlude the ground around them (contact occlusion): model,
    /// root piece matrix and how built it is (0..1).
    pub(crate) occluder_roots: Vec<(ModelId, Mat4, f32)>,
}

impl ModelRenderer {
    pub fn new(
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        hdr_format: wgpu::TextureFormat,
        samples: u32,
        textures: TextureLibrary,
    ) -> Self {
        use wgpu::{BindingType as B, ShaderStages as S};
        let paint_array = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: S::FRAGMENT,
            ty: B::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: S::VERTEX,
                    ty: B::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: S::FRAGMENT,
                    ty: B::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: S::FRAGMENT,
                    ty: B::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: S::FRAGMENT,
                    ty: B::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: S::FRAGMENT,
                    ty: B::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: S::FRAGMENT,
                    ty: B::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: S::FRAGMENT,
                    ty: B::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: S::FRAGMENT,
                    ty: B::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The second bank's paint, Reborn's paint and finish (see `TEXTURE_BANKS`).
                paint_array(8),
                paint_array(9),
                paint_array(10),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("model sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let shader = crate::util::shader(device, "unit", &unit_source());
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("unit"),
            bind_group_layouts: &[Some(frame_layout), Some(&layout)],
            immediate_size: 0,
        });
        let vertex_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<ModelVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Snorm8x4, 2 => Unorm16x2, 3 => Uint32, 4 => Unorm8x4, 5 => Uint32, 10 => Float16x4, 11 => Snorm8x4, 13 => Float32x2],
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<InstanceData>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![6 => Uint32, 7 => Uint32, 8 => Uint32, 9 => Float32, 12 => Float32, 14 => Float32x2, 15 => Uint32x2],
            }),
        ];
        // The scene pass's pipeline, or with `mirrored` the reflection pass's.
        let make = |label: &str, mirrored: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &vertex_buffers,
                    compilation_options: Default::default(),
                },
                // Back faces are culled, as TA did: its models are wound consistently, and
                // drawn from both sides the undersides of flat parts (ground plates under
                // scorpions, labs and storage tanks) and the insides of shells show through
                // as black or wrongly textured faces. (The mirror pass sees them clockwise.)
                primitive: wgpu::PrimitiveState {
                    front_face: if mirrored {
                        wgpu::FrontFace::Cw
                    } else {
                        wgpu::FrontFace::Ccw
                    },
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(crate::util::depth_state(true)),
                multisample: wgpu::MultisampleState {
                    count: if mirrored { 1 } else { samples },
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if mirrored { "fs_reflection" } else { "fs_main" }),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: hdr_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    // (How many samples a cloaked unit's share of them is taken from; where the
                    // paint's second bank begins.)
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("SCENE_SAMPLES", if mirrored { 1.0 } else { samples as f64 }), ("SECOND_BANK", f64::from(textures.second_bank))],
                        ..Default::default()
                    },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make("unit", false);
        let reflection_pipeline = make("unit reflection", true);
        // `TA_SHOW_BACKS=1`: the back faces again, in magenta, where they lie in front of what
        // was drawn (see `BACKS_PASS` in unit.wgsl).
        let backs_pipeline = show_backs().then(|| {
            let constants = [("BACKS_PASS", 1.0)];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("unit backs"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &vertex_buffers,
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                },
                primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Front), ..Default::default() },
                depth_stencil: Some(crate::util::depth_state(false)),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_backs"),
                    targets: &[Some(wgpu::ColorTargetState { format: hdr_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("unit shadow"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                buffers: &vertex_buffers,
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
                ..crate::util::depth_state(true)
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let prepass_pipeline = crate::util::ray_traced().then(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("unit depth prepass"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &vertex_buffers,
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(crate::util::depth_state(true)),
                multisample: wgpu::MultisampleState::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        });
        let matrix_buffer = Self::make_matrix_buffer(device, 1024);
        let instance_buffer = Self::make_instance_buffer(device, 256);
        let bind_group = Self::make_bind_group(device, &layout, &matrix_buffer, &textures, &sampler);
        ModelRenderer {
            library: ModelLibrary {
                blas_input: crate::util::ray_traced(),
                ..Default::default()
            },
            textures,
            art_style_reborn: true,
            pipeline,
            shadow_pipeline,
            reflection_pipeline,
            backs_pipeline,
            prepass_pipeline,
            layout,
            sampler,
            matrices: Vec::new(),
            instances: Vec::new(),
            batches: Vec::new(),
            whole_groups: batching::InstanceGroups::default(),
            matrix_buffer,
            instance_buffer,
            bind_group,
            occluder_roots: Vec::new(),
        }
    }

    fn make_matrix_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("piece matrices"),
            size: (count * 48) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_instance_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("model instances"),
            size: (count * std::mem::size_of::<InstanceData>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        matrices: &wgpu::Buffer,
        textures: &TextureLibrary,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("model"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: matrices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&textures.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&textures.reborn_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&textures.reborn_physical),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(textures.skin_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(textures.enemy_skin_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: textures.livery_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&textures.second[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&textures.second[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&textures.second[2]),
                },
            ],
        })
    }

    /// Style switches rebuild the next frame's batches; source rigs remain untouched.
    pub fn set_art_style(&mut self, reborn: bool) {
        self.art_style_reborn = reborn;
    }

    /// Collects this frame's instances (grouped by model) and uploads them; with `traced`, a copy
    /// of their matrices is kept for ray tracing ([`ModelRenderer::traced_pieces`]).
    pub fn prepare<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: impl IntoIterator<Item = DrawInstance<'a>>,
        view: Option<(&crate::camera::Camera, bool)>,
        traced: bool,
    ) {
        use rayon::prelude::*;
        self.library.upload(device);
        let mut list: Vec<DrawInstance<'a>> = instances.into_iter().collect();
        // Each instance's model by its size on screen (Reborn's detail up close): an army's on
        // all cores.
        let (library, reborn, camera) = (&self.library, self.art_style_reborn, view.map(|v| v.0));
        let detail = |inst: &mut DrawInstance<'a>| {
            if let Some(root) = inst.pieces.first() {
                inst.model = library.detail_for(inst.model, *root, camera, reborn);
            }
        };
        if list.len() >= PARALLEL_INSTANCES {
            list.par_iter_mut().with_min_len(128).for_each(detail);
        } else {
            list.iter_mut().for_each(detail);
        }
        // Instances by model, each model's in the order given (indices into `list`).
        let models = self.library.models.len();
        self.whole_groups.build(models, list.iter().map(|inst| inst.piece.is_none().then_some(inst.model as usize)));
        let mut by_piece: std::collections::BTreeMap<(ModelId, u32), Vec<u32>> =
            Default::default();
        self.occluder_roots.clear();
        for (k, inst) in list.iter().enumerate() {
            // Whole, visible models occlude the ground around them; nanoframes as far as they
            // are built (placement ghosts, at zero, not at all); debris and cloaked units never.
            if inst.piece.is_none() && inst.flags & (FLAG_CLOAKED | FLAG_REMEMBERED) == 0 && (inst.model as usize) < models {
                let built = if inst.flags & FLAG_BUILDING != 0 {
                    inst.build.clamp(0.0, 1.0)
                } else {
                    1.0
                };
                if let (Some(root), true) = (inst.pieces.first(), built > 0.0) {
                    self.occluder_roots.push((inst.model, *root, built));
                }
            }
            match inst.piece {
                Some(p) if (inst.model as usize) < models => {
                    by_piece.entry((inst.model, p)).or_default().push(k as u32)
                }
                Some(_) => {}
                None => {}
            }
        }
        self.instances.clear();
        self.batches.clear();
        // Whole models, model by model: where each instance's matrices go.
        let mut total = 0usize;
        for (model, group) in self.whole_groups.groups().enumerate() {
            if group.is_empty() {
                continue;
            }
            let first = self.instances.len() as u32;
            for &k in group {
                let inst = &list[k as usize];
                self.instances.push(InstanceData {
                    matrix_base: total as u32,
                    team: inst.team,
                    flags: inst.flags,
                    build: inst.build,
                    damage: inst.damage,
                    travel: inst.travel,
                    playing: [inst.playing as u32, (inst.playing >> 32) as u32],
                });
                total += inst.pieces.len();
            }
            self.batches
                .push((model as ModelId, None, first, group.len() as u32));
        }
        // Single pieces: one matrix each. The vertex shader indexes matrices by the vertex's
        // piece number, so the base is set that far before the matrix (the gap left blank).
        let mut single: Vec<(usize, &Mat4)> = Vec::new();
        let mut len = total;
        for (&(model, piece), group) in &by_piece {
            let first = self.instances.len() as u32;
            for inst in group.iter().map(|&k| &list[k as usize]) {
                let Some(m) = inst.pieces.first() else {
                    continue;
                };
                len = len.max(piece as usize);
                single.push((len, m));
                self.instances.push(InstanceData {
                    matrix_base: len as u32 - piece,
                    team: inst.team,
                    flags: inst.flags,
                    build: inst.build,
                    damage: inst.damage,
                    travel: inst.travel,
                    playing: [inst.playing as u32, (inst.playing >> 32) as u32],
                });
                len += 1;
            }
            let count = self.instances.len() as u32 - first;
            if count > 0 {
                self.batches.push((model, Some(piece), first, count));
            }
        }
        if self.instances.is_empty() {
            self.matrices.clear();
            return;
        }
        if (self.matrix_buffer.size() as usize) < len * 48 {
            self.matrix_buffer = Self::make_matrix_buffer(device, len.next_power_of_two());
            self.bind_group = Self::make_bind_group(device, &self.layout, &self.matrix_buffer, &self.textures, &self.sampler);
        }
        let need_i = self.instances.len() * std::mem::size_of::<InstanceData>();
        if (self.instance_buffer.size() as usize) < need_i {
            self.instance_buffer =
                Self::make_instance_buffer(device, self.instances.len().next_power_of_two());
        }
        // The matrices (hundreds of thousands with an army in view): written straight into the
        // upload's memory, unless ray tracing reads them back this frame (`traced`), which keeps
        // a copy. (An army's 11 MB were written twice a frame: into the copy, then the upload.)
        let order = &self.whole_groups.order;
        let write = |out: wgpu::WriteOnly<'_, [[u8; 48]]>| write_matrices(out, order, &list, &single, total);
        if traced {
            self.matrices.resize(len, [[0.0; 4]; 3]);
            write(wgpu::WriteOnly::from(bytemuck::cast_slice_mut::<Rows, [u8; 48]>(&mut self.matrices)));
            queue.write_buffer(&self.matrix_buffer, 0, bytemuck::cast_slice(&self.matrices));
        } else {
            self.matrices.clear();
            if let Some(mut upload) = wgpu::BufferSize::new((len * 48) as u64).and_then(|size| queue.write_buffer_with(&self.matrix_buffer, 0, size)) {
                write(upload.slice(..).into_chunks::<48>().0);
            }
        }
        queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&self.instances),
        );
    }

    /// Draws this frame's batches; with `originals`, each model's original in place of Reborn's
    /// detailed version (the same pieces, so the same instances and matrices).
    fn draw_with<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        pipeline: &'a wgpu::RenderPipeline,
        originals: bool,
    ) {
        let (Some(vb), Some(ib)) = (&self.library.vertex_buffer, &self.library.index_buffer) else {
            return;
        };
        if self.batches.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, vb.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
        for &(model, piece, first, count) in &self.batches {
            let model = if originals { self.library.original.get(&model).copied().unwrap_or(model) } else { model };
            let info = &self.library.models[model as usize];
            let range = match piece.and_then(|p| info.pieces.get(p as usize)) {
                Some(p) => p.first_index..p.first_index + p.index_count,
                None if piece.is_some() => continue,
                None => info.first_index..info.first_index + info.index_count,
            };
            if !range.is_empty() {
                pass.draw_indexed(range, info.base_vertex, first..first + count);
            }
        }
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.draw_with(pass, &self.pipeline, false);
        if let Some(backs) = &self.backs_pipeline {
            self.draw_with(pass, backs, false);
        }
    }

    pub fn draw_shadow<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.draw_with(pass, &self.shadow_pipeline, false);
    }

    /// Draws the models mirrored in the water (inside the reflection pass): TA's shapes with
    /// Reborn's paint. The mirror sees them from below, where what Reborn's kit adds was never
    /// meant to be seen from (its tracks' dark backs, its plates' rims at a grazing angle under
    /// decks and floating floors), and the rippled reflection shows none of its detail anyway.
    pub(crate) fn draw_reflection<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.draw_with(pass, &self.reflection_pipeline, true);
    }

    /// Draws the models' depth for ray tracing (raytrace.rs).
    pub(crate) fn draw_prepass<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if let Some(p) = &self.prepass_pipeline {
            self.draw_with(pass, p, false);
        }
    }

    /// This frame's pieces as ray tracing sees them: model, piece, player colour and its 3×4 world
    /// matrix (rows), its unit's travel, and its state ([`traced_state`]), for every visible
    /// piece of every instance but cloaked units and placement ghosts (hidden pieces have zero
    /// matrices).
    pub(crate) fn traced_pieces(
        &self,
    ) -> impl Iterator<Item = (ModelId, u32, u32, [f32; 12], [f32; 2], u32)> + '_ {
        self.batches
            .iter()
            .flat_map(move |&(model, piece, first, count)| {
                let pieces = match piece {
                    Some(p) => p..p + 1,
                    None => {
                        0..self
                            .library
                            .models
                            .get(model as usize)
                            .map_or(0, |m| m.pieces.len() as u32)
                    }
                };
                self.instances[first as usize..(first + count) as usize]
                    .iter()
                    .filter(|i| {
                        i.flags & (FLAG_CLOAKED | FLAG_REMEMBERED) == 0
                            && !(i.flags & FLAG_BUILDING != 0 && i.build < HOLOGRAM_SHADOWLESS)
                    })
                    .flat_map(move |inst| {
                        pieces.clone().filter_map(move |p| {
                            let m = self.matrices.get((inst.matrix_base + p) as usize)?;
                            if m.iter().all(|row| row.iter().all(|&v| v == 0.0)) {
                                return None;
                            }
                            Some((
                                model,
                                p,
                                inst.team,
                                [
                                    m[0][0], m[0][1], m[0][2], m[0][3], m[1][0], m[1][1], m[1][2],
                                    m[1][3], m[2][0], m[2][1], m[2][2], m[2][3],
                                ],
                                inst.travel,
                                traced_state(inst, p),
                            ))
                        })
                    })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which pieces' animated textures play, and how far the lights are out, reach the shaders
    /// (the raster's instance and the ray tracer's piece) as the scene gave them.
    #[test]
    fn played_textures_and_lights_out_reach_each_piece() {
        let playing: u64 = 1 | 1 << 31 | 1 << 32 | 1 << 63;
        for (piece, plays) in [(0, true), (1, false), (31, true), (32, true), (33, false), (63, true), (64, true), (300, true)] {
            assert_eq!(piece_plays(playing, piece), plays, "piece {piece}");
        }
        assert!(!piece_plays(1, 64), "the pieces past 63 share bit 63");
        assert!(!piece_plays(0, 0) && piece_plays(PLAYING_ALL, 7));
        let inst = InstanceData { matrix_base: 0, team: 0, flags: FLAG_SELECTED | 200 << FLAG_LIGHTS_OUT_SHIFT, build: 1.0, damage: 0.0, travel: [0.0; 2], playing: [playing as u32, (playing >> 32) as u32] };
        assert_eq!(traced_state(&inst, 0), 1 | 200 << 8);
        assert_eq!(traced_state(&inst, 1), 200 << 8);
        assert_eq!(traced_state(&inst, 32), 1 | 200 << 8);
        let lit = InstanceData { flags: 0, ..inst };
        assert_eq!(traced_state(&lit, 33), 0);
    }

    /// A square quad twice `half` across lying flat on its piece's origin, its one face down (as
    /// `ModelLibrary::add` turns z round) or up, painted or not.
    fn plate(name: &str, parent: Option<usize>, half: f32, down: bool, texture: Option<&str>) -> ta_formats::model3do::Piece {
        let s = (half * FIXED_ONE) as i32;
        ta_formats::model3do::Piece {
            name: name.into(),
            parent,
            offset: [0; 3],
            children: vec![],
            selection_primitive: None,
            vertices: vec![[-s, 0, -s], [-s, 0, s], [s, 0, s], [s, 0, -s]],
            primitives: vec![ta_formats::model3do::Primitive { color_index: 1, indices: if down { vec![3, 2, 1, 0] } else { vec![0, 1, 2, 3] }, texture: texture.map(Into::into) }],
        }
    }

    #[test]
    fn plates_on_the_ground_are_left_undrawn_wherever_they_hang() {
        // The Core Mine's root plate; TA: Escalation's Commanders' `flip` (108 units square),
        // hung from the root and lying on the ground.
        assert_eq!(unflagged_ground_plate(&plate("ground", None, 16.0, true, None), 0.0), Some(0));
        assert_eq!(unflagged_ground_plate(&plate("flip", Some(0), 54.0, true, None), 0.0), Some(0));
        // Off the ground (a hull's belly, which a low camera sees), facing up (seen from above),
        // or painted: drawn.
        assert_eq!(unflagged_ground_plate(&plate("belly", Some(0), 10.0, true, None), 12.0), None);
        assert_eq!(unflagged_ground_plate(&plate("flip", Some(0), 54.0, false, None), 0.0), None);
        assert_eq!(unflagged_ground_plate(&plate("flip", Some(0), 54.0, true, Some("ArmCam3a")), 0.0), None);
        // A Commander standing over one: the ground round it is darkened as under its body (16
        // units across), not under the plate (108).
        let s = 8 * FIXED_ONE as i32;
        let body = ta_formats::model3do::Piece {
            name: "body".into(),
            parent: None,
            offset: [0; 3],
            children: vec![1],
            selection_primitive: None,
            vertices: vec![[-s, 0, -s], [s, 0, -s], [s, 0, s], [-s, 0, s], [-s, 2 * s, -s], [s, 2 * s, -s], [s, 2 * s, s], [-s, 2 * s, s]],
            primitives: [[4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]].into_iter().map(|indices| ta_formats::model3do::Primitive { color_index: 128, indices: indices.to_vec(), texture: None }).collect(),
        };
        let model = Model3do { pieces: vec![body, plate("flip", Some(0), 54.0, true, None)] };
        let mut library = ModelLibrary::default();
        let id = library.add_cpu("commander", &model, |_| None, |_| None, &Palette::default());
        let b = library.info(id).footprint.bounds;
        assert!(b.iter().all(|v| v.abs() <= 8.01), "{b:?}");
    }

    /// The plates every model of the install (and of the mod `TA_TEST_MOD=<mod folder>` mounted
    /// over it) has lying on the ground under a piece other than its root, left undrawn (`cargo
    /// test --release -p ta_render --lib ground_plates_of_every_model -- --ignored --nocapture`).
    #[test]
    #[ignore = "requires the locally installed original game"]
    fn ground_plates_of_every_model() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        // (As the game mounts it: what the mod replaces of the install left out.)
        let module = std::env::var("TA_TEST_MOD").ok().map(|dir| ta_vfs::mods::read_mod(std::path::Path::new(&dir)).unwrap().expect("a mod's folder"));
        let mounted = ta_vfs::mods::mount(Some(&install.root), None, &module.iter().collect::<Vec<_>>(), &|m| m.manifest.id.clone());
        mounted.install.unwrap();
        mounted.mods.unwrap();
        let vfs = mounted.vfs;
        let (mut models, mut found) = (0, 0);
        for path in vfs.list("objects3d", Some("3do"), true) {
            let Ok(model) = Model3do::parse(&vfs.read(&path).unwrap()) else { continue };
            models += 1;
            let mut at: Vec<f32> = Vec::with_capacity(model.pieces.len());
            for p in &model.pieces {
                at.push(p.parent.and_then(|q| at.get(q).copied()).unwrap_or(0.0) + p.offset[1] as f32 / FIXED_ONE);
            }
            for (p, &y) in model.pieces.iter().zip(&at) {
                if p.parent.is_some() && p.selection_primitive.is_none() && unflagged_ground_plate(p, y).is_some() {
                    let v = &p.vertices;
                    let span = |k: usize| (v.iter().map(|v| v[k]).max().unwrap_or(0) - v.iter().map(|v| v[k]).min().unwrap_or(0)) as f32 / FIXED_ONE;
                    println!("{path}: {} ({:.1} x {:.1})", p.name, span(0), span(2));
                    found += 1;
                }
            }
        }
        println!("{found} plates in {models} models");
    }

    #[test]
    fn skins_preserve_team_identity_animation_and_functional_marks() {
        let armour = TextureRef { layer: 0, frames: 1, emissive: false, roughness: 0.6, metallic: 0.1, mean: [128; 3] };
        assert!(skin_eligible("ArmCam3A", armour));
        assert!(!skin_eligible("armour", TextureRef { frames: TEAM_FRAMES, ..armour }));
        assert!(!skin_eligible("armour", TextureRef { frames: 3, ..armour }));
        assert!(!skin_eligible("lamp", TextureRef { emissive: true, ..armour }));
        for name in ["Tredside", "arm_logo", "helipad", "corebridge"] {
            assert!(!skin_eligible(name, armour), "{name} must retain its functional paint");
        }
    }

    #[test]
    fn unit_skin_scopes_keep_shared_objects_and_unskinned_features_separate() {
        use ta_formats::model3do::{Piece, Primitive};
        let model = Model3do { pieces: vec![Piece {
            name: "body".into(), parent: None, offset: [0; 3], children: vec![], selection_primitive: None,
            vertices: vec![[0, 0, 0], [655360, 0, 0], [0, 655360, 0], [0, 0, 655360]],
            primitives: [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]].into_iter().map(|indices| Primitive { color_index: 128, indices: indices.to_vec(), texture: Some("armour".into()) }).collect(),
        }] };
        let original = TextureRef { layer: 7, frames: 1, emissive: false, roughness: 0.6, metallic: 0.1, mean: [128; 3] };
        let custom = TextureRef { layer: skin::CUSTOM | 2, ..original };
        let mut library = ModelLibrary::default();
        let palette = Palette::default();
        let a = library.add_cpu_scoped("unit:a/shared", "shared", &model, |_, eligible| Some(if eligible { custom } else { original }), |_| None, &palette, None, None, true);
        let b = library.add_cpu_scoped("unit:b/shared", "shared", &model, |_, _| Some(original), |_| None, &palette, Some(0), None, true);
        let enemy = TextureRef { layer: custom.layer | skin::ENEMY, ..custom };
        let opposing = library.add_cpu_scoped("unit:a/shared/skin:enemy", "shared", &model, |_, eligible| Some(if eligible { enemy } else { original }), |_| None, &palette, None, None, true);
        let again = library.add_cpu_scoped("unit:a/shared", "shared", &model, |_, _| panic!("matching selections must reuse the mesh"), |_| None, &palette, None, None, true);
        assert_eq!(again, a);
        assert_ne!(opposing, a, "the same unit and object can have opposing paint");
        let c = library.add_cpu("shared", &model, |_| Some(original), |_| None, &palette);
        let mut effect = model.clone();
        effect.pieces[0].name = "muzzle_flash".into();
        let d = library.add_cpu_scoped("unit:d/effect", "effect", &effect, |_, eligible| Some(if eligible { custom } else { original }), |_| None, &palette, Some(0), Some(custom), true);
        let mut plain = model.clone();
        for primitive in &mut plain.pieces[0].primitives {
            primitive.texture = None;
        }
        let e = library.add_cpu_scoped("unit:e/plain", "plain", &plain, |_, _| None, |_| None, &palette, None, Some(custom), true);
        // The enemy bank's livery; and a livery with a texture mapped over it.
        let theirs = library.add_cpu_scoped("unit:f/shared/skin:enemy", "shared", &model, |_, _| Some(original), |_| None, &palette, Some(skin::ENEMY), None, true);
        let mapped = library.add_cpu_scoped("unit:g/shared", "shared", &model, |_, eligible| Some(if eligible { custom } else { original }), |_| None, &palette, Some(0), None, true);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
        let flags = |id| {
            let info = library.info(id);
            library.vertices[info.base_vertex as usize].layer
        };
        assert_eq!(flags(a) & skin::CUSTOM, skin::CUSTOM);
        assert_eq!(flags(opposing) & (skin::CUSTOM | skin::ENEMY), skin::CUSTOM | skin::ENEMY);
        assert_eq!(flags(a) & skin::ENEMY, 0);
        assert_eq!(library.info(a).rest_pose(), library.info(opposing).rest_pose());
        assert_eq!(library.info(a).index_count, library.info(opposing).index_count);
        assert_eq!(flags(b) & (skin::LIVERY | skin::ENEMY), skin::LIVERY);
        assert_eq!(flags(c) & (skin::CUSTOM | skin::LIVERY), 0);
        assert_eq!(flags(d) & (skin::CUSTOM | skin::LIVERY), 0, "textured effects must keep their original paint");
        assert_eq!(flags(e) & skin::CUSTOM, skin::CUSTOM, "plain armour supports the reserved _untextured material");
        assert_eq!(flags(mapped) & (skin::CUSTOM | skin::LIVERY), skin::CUSTOM, "a face a skin maps a texture to takes no livery");
        assert_eq!(library.info(a).rest_pose(), library.info(c).rest_pose());
        assert_eq!(library.info(a).index_count, library.info(c).index_count);
        assert_eq!(library.vertices[library.info(a).base_vertex as usize].color[..3], [255; 3]);
        assert_eq!(flags(theirs) & (skin::LIVERY | skin::ENEMY), skin::LIVERY | skin::ENEMY);
    }

    /// What a skin's editor offers to repaint: the unit's armour, once each, but neither its
    /// team colours, lights, tracks and markings nor its effect pieces' paint.
    #[test]
    fn paintable_textures_are_the_armour_a_skin_may_repaint() {
        use ta_formats::model3do::{Piece, Primitive};
        let piece = |name: &str, textures: &[&str]| Piece {
            name: name.into(), parent: None, offset: [0; 3], children: vec![], selection_primitive: None,
            vertices: vec![[0, 0, 0], [65536, 0, 0], [0, 65536, 0]],
            primitives: textures.iter().map(|t| Primitive { color_index: 0, indices: vec![0, 1, 2], texture: Some((*t).into()) }).collect(),
        };
        let model = Model3do { pieces: vec![
            piece("base", &["ArmMetal1", "Team", "Tredside1", "armmetal1", "Lamp", "Helipad", "Missing"]),
            piece("turret", &["ArmPlate", "ArmMetal1"]),
            piece("flare", &["FlareMetal"]),
        ] };
        let frames = |name: &str| match name.to_ascii_lowercase().as_str() {
            "missing" => None,
            "team" => Some(TEAM_FRAMES),
            _ => Some(1),
        };
        assert_eq!(paintable_textures(&model, frames), ["ArmMetal1", "ArmPlate"]);
    }

    /// The kit's seams round a plate are shades of its paint: a livery paints them too, from
    /// the same bank; its bolts, wheels and tracks keep their steel and rubber.
    #[test]
    fn a_livery_paints_the_kits_seams_but_not_its_hardware() {
        use ta_formats::model3do::{Piece, Primitive};
        // (A wide flat box, plated by the kit.)
        let s = 655360 * 4;
        let model = Model3do { pieces: vec![Piece {
            name: "base".into(), parent: None, offset: [0; 3], children: vec![], selection_primitive: None,
            vertices: vec![[-s, 0, -s], [s, 0, -s], [s, 0, s], [-s, 0, s], [-s, s / 4, -s], [s, s / 4, -s], [s, s / 4, s], [-s, s / 4, s]],
            primitives: [[4, 7, 6, 5], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]].into_iter()
                .map(|indices| Primitive { color_index: 128, indices: indices.to_vec(), texture: Some("armour".into()) }).collect(),
        }] };
        let armour = TextureRef { layer: 3, frames: 1, emissive: false, roughness: 0.6, metallic: 0.1, mean: [128; 3] };
        let mut library = ModelLibrary::default();
        let palette = Palette::default();
        let id = library.add_cpu_scoped("unit:box/box/skin:test", "box", &model, |_, _| Some(armour), |_| None, &palette, Some(skin::ENEMY), None, true);
        let Some(&detailed) = library.detail.get(&id) else { panic!("the kit plates a wide box") };
        let info = library.info(detailed);
        let used: std::collections::BTreeSet<u32> = library.indices[info.first_index as usize..][..info.index_count as usize].iter().copied().collect();
        let plain: Vec<&ModelVertex> = used.iter().map(|&i| &library.vertices[(info.base_vertex + i as i32) as usize]).filter(|v| v.layer & 0xFFFF == 0xFFFF).collect();
        assert!(plain.iter().any(|v| v.layer & (skin::LIVERY | skin::ENEMY) == skin::LIVERY | skin::ENEMY), "seams take the livery");
        assert!(plain.iter().filter(|v| v.layer & skin::LIVERY != 0).all(|v| v.layer & skin::ENEMY != 0), "from the face's bank");
        // TA's originals (a livery over TA's own paint, Heritage's): the same box, no kit at any size.
        let original = library.add_cpu_scoped("unit:box/box/skin:originals", "box", &model, |_, _| Some(armour), |_| None, &palette, Some(0), None, false);
        assert!(!library.detail.contains_key(&original), "TA's own shapes take no kit");
        assert_eq!(library.detail_for(original, Mat4::IDENTITY, None, true), original);
    }


    /// A long, narrow face of TA's painted chrome is laid with a few lines of its paint running
    /// its length (brushed metal) instead of the whole drawn out into streaks; a square one, or
    /// paint that is a ramp along the way it is drawn out, keeps TA's mapping.
    #[test]
    fn drawn_out_chrome_is_brushed() {
        // (A quad 6 across u and 31 along v.)
        let quad = [Vec3::ZERO, Vec3::new(6.0, 0.0, 0.0), Vec3::new(6.0, 0.0, 31.0), Vec3::new(0.0, 0.0, 31.0)];
        let noise: Vec<f32> = (0..64 * 64u32).map(|i| ((i.wrapping_mul(2_654_435_761) >> 7) % 97) as f32 / 97.0).collect();
        let paint = kit::PaintMap::new(64, 64, &noise).unwrap();
        let uvs = brushed_uvs(&quad, Some(&paint)).expect("brushed");
        assert_eq!((uvs[0][0], uvs[1][0]), (0.0, 1.0));
        assert!((uvs[2][1] - uvs[1][1] - 2.0 / 64.0).abs() < 1e-6, "{uvs:?}");
        let square = [Vec3::ZERO, Vec3::new(20.0, 0.0, 0.0), Vec3::new(20.0, 0.0, 20.0), Vec3::new(0.0, 0.0, 20.0)];
        assert!(brushed_uvs(&square, Some(&paint)).is_none());
        // A ramp along v draws out as TA meant it.
        let ramp: Vec<f32> = (0..64 * 64).map(|i| (i / 64) as f32 / 63.0).collect();
        assert!(brushed_uvs(&quad, Some(&kit::PaintMap::new(64, 64, &ramp).unwrap())).is_none());
    }

    /// A long sliver standing alone (a frill, drawn with its back or not) becomes a thin lens of
    /// its outline; one meeting another face, or a broad one, stays as TA drew it.
    #[test]
    fn lone_slivers_become_lenses() {
        let face = |corners: Vec<Vec3>| {
            let normal = face_normal(&corners);
            (FaceSource { texture: None, color_index: 7, cap: false, uvs: None, mirror: None, back: false, late: false, of: None }, Face { corners, normal })
        };
        let (a, b, c) = (Vec3::ZERO, Vec3::new(20.0, 0.0, 2.0), Vec3::new(20.0, 0.0, -2.0));
        let mut faces = vec![face(vec![a, b, c]), face(vec![a, c, b])];
        thicken_lone_sheets(&mut faces);
        assert_eq!(faces.len(), 6);
        let thick = faces.iter().flat_map(|(_, f)| f.corners.iter().map(|p| p.y)).fold(0.0f32, |m, y| m.max(y.abs()));
        assert!(thick > 0.3 && thick < 1.0, "{thick}");
        // (Its two sides face away from each other.)
        assert!(faces.iter().any(|(_, f)| f.normal.y > 0.9) && faces.iter().any(|(_, f)| f.normal.y < -0.9));
        // Meeting another face along an edge: kept.
        let mut faces = vec![face(vec![a, b, c]), face(vec![b, a, Vec3::new(10.0, 8.0, 0.0)])];
        thicken_lone_sheets(&mut faces);
        assert_eq!(faces.len(), 2);
        // Broad: kept.
        let mut faces = vec![face(vec![a, Vec3::new(20.0, 0.0, 10.0), Vec3::new(20.0, 0.0, -10.0)])];
        thicken_lone_sheets(&mut faces);
        assert_eq!(faces.len(), 1);
    }

    /// Each piece's extent (model units, piece space) in a model of the library.
    fn piece_bounds(library: &ModelLibrary, id: ModelId) -> Vec<Option<(Vec3, Vec3)>> {
        let info = &library.models[id as usize];
        info.pieces
            .iter()
            .map(|p| {
                let range = p.first_index as usize..(p.first_index + p.index_count) as usize;
                library.indices[range].iter().map(|&i| Vec3::from_array(library.vertices[(info.base_vertex as u32 + i) as usize].pos)).fold(None, |b: Option<(Vec3, Vec3)>, v| Some(b.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v)))))
            })
            .collect()
    }

    /// Reborn's kit on every model of the install: each detailed piece keeps to its original's
    /// extent (out of it by a plate's height, or a road wheel's hub standing out of a track's
    /// side, never past its ends), and its triangles are a modest multiple of the original's.
    /// (Skipped without an install.)
    #[test]
    fn the_kit_keeps_to_every_installed_models_shape() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        // What the kit asks of a texture (a light, a team colour) from its name alone.
        let texture = |name: &str| {
            Some(TextureRef { layer: 0, frames: 1, emissive: emissive_texture(name), roughness: 0.5, metallic: 0.1, mean: [128; 3] })
        };
        let mut library = ModelLibrary::default();
        let (mut models, mut detailed, mut worst) = (0, 0, Vec::new());
        let (mut triangles, mut detailed_triangles) = (0usize, 0usize);
        for path in vfs.list("objects3d", Some("3do"), true) {
            let Ok(model) = Model3do::parse(&vfs.read(&path).unwrap()) else { continue };
            let id = library.add_cpu(&path, &model, texture, |_| None, &palette);
            models += 1;
            let Some(&detail) = library.detail.get(&id) else { continue };
            detailed += 1;
            let stats = library.detail_stats[&id];
            triangles += stats.triangles;
            detailed_triangles += stats.detailed_triangles;
            for (k, (before, after)) in piece_bounds(&library, id).into_iter().zip(piece_bounds(&library, detail)).enumerate() {
                let (Some((lo, hi)), Some((dlo, dhi))) = (before, after) else { continue };
                let grown = (lo - dlo).max(dhi - hi).max_element();
                // A road wheel stands out of its track by its hub, a unit at most.
                // (And as far as rounding drew a curve's facets out towards it.)
                let allowed = if stats.wheels > 0 { 1.05 } else { 0.45 } + stats.bowed;
                if grown > allowed {
                    worst.push((grown, format!("{path} piece {k} ({}): out by {grown:.2}", model.pieces[k].name)));
                }
            }
        }
        worst.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, line) in worst.iter().take(20) {
            eprintln!("{line}");
        }
        eprintln!("kit: {detailed} of {models} models detailed, {triangles} -> {detailed_triangles} triangles");
        assert!(worst.is_empty(), "{} pieces out of shape", worst.len());
        // (Drawn only where a unit is big on screen: a tenfold bound keeps it in reason.)
        assert!(detailed_triangles < triangles * 10, "{triangles} -> {detailed_triangles}");
    }

    /// How much of the installed models is drawn curved (smooth-shaded: a triangle whose
    /// corners' normals stray from its face's by more than 8 degrees), and the models most so.
    #[test]
    #[ignore = "requires the locally installed original game"]
    fn scan_curves() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let texture = |name: &str| Some(TextureRef { layer: 0, frames: 1, emissive: emissive_texture(name), roughness: 0.5, metallic: 0.1, mean: [128; 3] });
        let mut library = ModelLibrary::default();
        let (mut all, mut curved, mut by_model) = (0usize, 0usize, Vec::new());
        for path in vfs.list("objects3d", Some("3do"), true) {
            let Ok(model) = Model3do::parse(&vfs.read(&path).unwrap()) else { continue };
            let id = library.add_cpu(&path, &model, texture, |_| None, &palette);
            let id = if std::env::var("TA_SCAN_DETAIL").is_ok_and(|v| v == "1") { library.detail.get(&id).copied().unwrap_or(id) } else { id };
            let info = library.models[id as usize].clone();
            let mut here = (0usize, 0usize);
            for tri in library.indices[info.first_index as usize..][..info.index_count as usize].chunks_exact(3) {
                let v = [tri[0], tri[1], tri[2]].map(|i| &library.vertices[(info.base_vertex + i as i32) as usize]);
                let p = v.map(|v| Vec3::from_array(v.pos));
                let face = (p[1] - p[0]).cross(p[2] - p[0]);
                if face.length_squared() < 1e-8 { continue; }
                let face = face.normalize();
                let bent = v.iter().map(|v| Vec3::new(v.normal[0] as f32, v.normal[1] as f32, v.normal[2] as f32).normalize_or_zero().dot(face)).fold(1.0f32, f32::min);
                here.0 += 1;
                if bent < 8.0f32.to_radians().cos() { here.1 += 1; }
            }
            all += here.0;
            curved += here.1;
            by_model.push((here.1, here.0, path));
        }
        by_model.sort_by(|a, b| b.0.cmp(&a.0));
        for (c, n, path) in by_model.iter().take(25) {
            eprintln!("{path}: {c} of {n} triangles curved");
        }
        eprintln!("curved: {curved} of {all} triangles ({:.1}%) in {} models with any", curved as f32 * 100.0 / all.max(1) as f32, by_model.iter().filter(|m| m.0 > 0).count());
    }

    /// Each piece's vertices in a model of the library (they are contiguous, piece by piece).
    fn piece_vertices(library: &ModelLibrary, id: ModelId) -> Vec<&[ModelVertex]> {
        let info = &library.models[id as usize];
        info.pieces
            .iter()
            .map(|p| {
                let range = p.first_index as usize..(p.first_index + p.index_count) as usize;
                let indices = &library.indices[range];
                let (Some(&lo), Some(&hi)) = (indices.iter().min(), indices.iter().max()) else { return &[][..] };
                let base = info.base_vertex as usize;
                &library.vertices[base + lo as usize..=base + hi as usize]
            })
            .collect()
    }

    /// TA's view is as it was: making forms whole (see halves.rs, and the rims in caps.rs) adds
    /// faces after TA's own and changes nothing of them. For every installed model, each piece's
    /// vertices with forms made whole begin with those without, the same to the bit (positions,
    /// normals, paint, edges' wear), in the original and in Reborn's detailed version alike, where
    /// the piece's caps of before are as they were; and the model's extent and its footprint (the
    /// shade it lays on the ground) are as they were. (Skipped without an install.)
    #[test]
    fn making_forms_whole_leaves_tas_own_faces_as_they_were() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let texture = |name: &str| Some(TextureRef { layer: 0, frames: 1, emissive: emissive_texture(name), roughness: 0.5, metallic: 0.1, mean: [128; 3] });
        let (mut before, mut after) = (ModelLibrary { forms: Some(false), rounded: Some(false), ..Default::default() }, ModelLibrary { forms: Some(true), rounded: Some(false), ..Default::default() });
        let (mut pieces, mut grown, mut recapped, mut differ) = (0, 0, 0, Vec::new());
        let mut shapes = Vec::new();
        // Each piece's caps of before, and the caps it has with forms made whole but for those
        // they added (a rim's, a muzzle's): where a half was capped before (the Stumpy's middle
        // barrel), its faces met those caps and now meet the faces making it whole.
        let old_caps = |model: &Model3do| -> Vec<bool> {
            use caps::tests::model_faces;
            let (faces, rest, mendable) = model_faces(model);
            let off = caps::mend(&faces.iter().zip(&rest).zip(&mendable).map(|((f, &offset), &mend)| caps::PieceFaces { faces: f, offset, mend }).collect::<Vec<_>>(), false);
            let own: Vec<usize> = faces.iter().map(Vec::len).collect();
            let (_, _, on) = halves::tests::make_whole_as_the_game(faces, &rest, &mendable);
            let corners = |caps: Vec<&caps::Cap>| {
                let mut out: Vec<Vec<[u32; 3]>> = caps.iter().map(|c| c.corners.iter().map(|p| p.to_array().map(f32::to_bits)).collect()).collect();
                out.sort();
                out
            };
            off.iter()
                .zip(&on)
                .zip(&own)
                .map(|((a, b), &own)| corners(a.caps.iter().collect()) == corners(b.caps.iter().filter(|c| !c.rim && !c.late && c.faces.iter().all(|&f| (f as usize) < own)).collect()))
                .collect()
        };
        for path in vfs.list("objects3d", Some("3do"), true) {
            let Ok(model) = Model3do::parse(&vfs.read(&path).unwrap()) else { continue };
            let (a, b) = (before.add_cpu(&path, &model, texture, |_| None, &palette), after.add_cpu(&path, &model, texture, |_| None, &palette));
            let same_caps = old_caps(&model);
            // (Its extent and the shade it lays on the ground.)
            let (x, y) = (&before.models[a as usize], &after.models[b as usize]);
            if x.radius != y.radius || x.height != y.height || x.footprint != y.footprint {
                shapes.push(format!("{path}: extent or footprint not as it was"));
            }
            let versions = [(a, b), (before.detail.get(&a).copied().unwrap_or(a), after.detail.get(&b).copied().unwrap_or(b))];
            for (k, (x, y)) in versions.into_iter().enumerate() {
                for (pi, (was, now)) in piece_vertices(&before, x).into_iter().zip(piece_vertices(&after, y)).enumerate() {
                    pieces += 1;
                    grown += usize::from(now.len() > was.len());
                    if !same_caps[pi] {
                        recapped += 1;
                        continue;
                    }
                    if now.len() < was.len() || bytemuck::cast_slice::<ModelVertex, u8>(was) != bytemuck::cast_slice::<ModelVertex, u8>(&now[..was.len()]) {
                        let first = (0..was.len().min(now.len())).find(|&v| bytemuck::bytes_of(&was[v]) != bytemuck::bytes_of(&now[v])).unwrap_or(was.len().min(now.len()));
                        let own: usize = model.pieces[pi].primitives.iter().enumerate().filter(|&(q, prim)| Some(q) != model.pieces[pi].selection_primitive && prim.indices.len() >= 3).map(|(_, prim)| prim.indices.len()).sum();
                        differ.push(format!("{path} piece {pi} ({}){}: first vertex {first} of {} (TA's {own}), now {}", model.pieces[pi].name, if k == 1 { ", detailed" } else { "" }, was.len(), now.len()));
                    }
                }
            }
        }
        for line in differ.iter().chain(&shapes) {
            eprintln!("{line}");
        }
        eprintln!("{pieces} pieces, {grown} with faces added, {recapped} whose caps of before gave way to them, {} not as they were", differ.len());
        assert!(differ.is_empty() && shapes.is_empty());
    }

    /// How long building every installed model takes with forms made whole and without, the
    /// slowest models: `cargo test --release -p ta_render --lib time_forms -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn time_forms() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let texture = |name: &str| Some(TextureRef { layer: 0, frames: 1, emissive: emissive_texture(name), roughness: 0.5, metallic: 0.1, mean: [128; 3] });
        let models: Vec<(String, Model3do)> = vfs.list("objects3d", Some("3do"), true).into_iter().filter_map(|p| Some((p.clone(), Model3do::parse(&vfs.read(&p).ok()?).ok()?))).collect();
        for forms in [false, true, false, true] {
            let mut library = ModelLibrary { forms: Some(forms), ..Default::default() };
            let started = std::time::Instant::now();
            let mut worst: Vec<(f64, String)> = Vec::new();
            for (path, model) in &models {
                let t = std::time::Instant::now();
                library.add_cpu(path, model, texture, |_| None, &palette);
                worst.push((t.elapsed().as_secs_f64() * 1000.0, path.clone()));
            }
            worst.sort_by(|a, b| b.0.total_cmp(&a.0));
            eprintln!("forms {forms}: {:.0} ms; slowest {:?}", started.elapsed().as_secs_f64() * 1000.0, &worst[..5]);
        }
    }

    /// `TA_KIT_DUMP=armstump,corraid TA_KIT_OUT=<dir>`: writes those models' Reborn detail
    /// (pieces at rest) as OBJ, to look at in a 3D program. On this computer only.
    #[test]
    #[ignore]
    fn dump_kit_models() {
        let (Ok(names), Ok(out)) = (std::env::var("TA_KIT_DUMP"), std::env::var("TA_KIT_OUT")) else { return };
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let texture = |name: &str| Some(TextureRef { layer: 0, frames: 1, emissive: emissive_texture(name), roughness: 0.5, metallic: 0.1, mean: [128; 3] });
        std::fs::create_dir_all(&out).unwrap();
        for name in names.split(',') {
            let model = Model3do::parse(&vfs.read(&format!("objects3d/{name}.3do")).unwrap()).unwrap();
            let mut library = ModelLibrary::default();
            let id = library.add_cpu(name, &model, texture, |_| None, &palette);
            let detail = library.detail.get(&id).copied().unwrap_or(id);
            let info = library.models[detail as usize].clone();
            let pose = info.rest_pose();
            let mut obj = String::new();
            for (k, piece) in info.pieces.iter().enumerate() {
                obj += &format!("o {}_{}\n", k, piece.name);
                let range = piece.first_index as usize..(piece.first_index + piece.index_count) as usize;
                for tri in library.indices[range].chunks(3) {
                    for &i in tri {
                        let p = pose[k].transform_point3(Vec3::from_array(library.vertices[(info.base_vertex as u32 + i) as usize].pos));
                        obj += &format!("v {} {} {}\n", p.x, p.y, p.z);
                    }
                    obj += "f -3 -2 -1\n";
                }
            }
            std::fs::write(std::path::Path::new(&out).join(format!("{name}_kit.obj")), obj).unwrap();
        }
    }

    #[test]
    fn a_textures_stray_pink_edge_takes_the_colours_inside_it() {
        let mut palette = Palette::default();
        palette.colors[200] = [187, 147, 195];
        palette.colors[10] = [120, 120, 120];
        palette.colors[11] = [60, 60, 60];
        // A grey texture, a darker line down column 3, its top row pink.
        let (w, h) = (16, 16);
        let mut pixels: Vec<u8> = (0..w * h).map(|i| if i % w == 3 { 11 } else { 10 }).collect();
        pixels[..w].fill(200);
        let image = ta_formats::image::IndexedImage { width: w as u32, height: h as u32, pixels, opaque: vec![true; w * h] };
        let mended = mend_edges(&image, &palette).expect("a pink top row");
        assert_eq!(&mended.pixels[..w], &image.pixels[w..2 * w], "the row inside it, line and all");
        assert_eq!(&mended.pixels[w..], &image.pixels[w..]);
        // Pink painted on purpose (the row inside it pink too) stays.
        let mut pink = image.clone();
        pink.pixels[w..2 * w].fill(200);
        assert!(mend_edges(&pink, &palette).is_none());
        assert!(mend_edges(&mended, &palette).is_none());
    }

    #[test]
    fn texture_budget_keeps_complete_animations_and_a_fallback() {
        assert!(texture_entry_fits(0, 10, 11));
        assert!(
            !texture_entry_fits(0, 10, 10),
            "the fallback needs one layer"
        );
        assert!(texture_entry_fits(2037, 10, 2048));
        assert!(
            !texture_entry_fits(2038, 10, 2048),
            "never truncate team frames"
        );
        assert!(texture_entry_fits(0, 255, 2048));
        assert!(
            !texture_entry_fits(0, 256, 2048),
            "the packed frame count is eight bits"
        );
        assert!(!texture_entry_fits(0, 0, 2048));
        assert!(!texture_entry_fits(usize::MAX, 1, 2048));
        assert!(
            !texture_entry_fits(0, 1, 1),
            "a minimal array only holds the fallback"
        );
    }

    /// TA's own textures fit one bank; a mod's that do not (TA: Enhanced's 2,476 layers with the
    /// fallback) take a second, numbered on from the first's last layer, a team texture's
    /// frames running across the line as they come.
    #[test]
    fn textures_past_one_array_take_the_second_bank() {
        assert_eq!(bank_sizes(705, 2048), [705]);
        assert_eq!(bank_sizes(2048, 2048), [2048]);
        assert_eq!(bank_sizes(2476, 2048), [2048, 428]);
        assert_eq!(bank_of(704, 0), (0, 704), "one bank: every layer in the first");
        assert_eq!(bank_of(2047, 2048), (0, 2047));
        assert_eq!(bank_of(2048, 2048), (1, 0));
        assert_eq!(bank_of(2475, 2048), (1, 427));
        // (The banks hold twice what one did: an entry fits up to the second's last layer but one.)
        let capacity = 2048 * TEXTURE_BANKS;
        assert!(texture_entry_fits(2045, 10, capacity), "frames running across into the second bank");
        assert!(texture_entry_fits(4085, 10, capacity));
        assert!(!texture_entry_fits(4086, 10, capacity), "the fallback keeps the last layer");
        // (Every layer's number fits the vertex's 16 bits, 0xFFFF meaning untextured.)
        assert!(capacity < 0xFFFF);
    }

    /// A texture library for the installed models takes what their drawn faces name: neither a
    /// selection plate's paint nor a line's, each name once, lower case.
    #[test]
    fn drawn_textures_are_the_faces_paint() {
        use ta_formats::model3do::{Piece, Primitive};
        let face = |texture: &str, corners: usize| Primitive { color_index: 0, indices: (0..corners as u16).collect(), texture: Some(texture.into()) };
        let model = Model3do { pieces: vec![Piece {
            name: "base".into(), parent: None, offset: [0; 3], children: vec![], selection_primitive: Some(0),
            vertices: vec![[0, 0, 0], [65536, 0, 0], [0, 65536, 0], [65536, 65536, 0]],
            primitives: vec![face("Plate", 4), face("ArmMetal1", 3), face("Wire", 2), face("ARMMETAL1", 4), face("CorV04b", 4)],
        }] };
        let mut names: Vec<String> = drawn_textures(&model).collect();
        names.sort();
        names.dedup();
        assert_eq!(names, ["armmetal1", "corv04b"]);
    }

    /// A box's faces (outward normals, as `ModelLibrary::add` makes them) from its corners.
    fn boxed(min: Vec3, max: Vec3) -> Vec<Face> {
        let c = |x: bool, y: bool, z: bool| {
            Vec3::new(
                if x { max.x } else { min.x },
                if y { max.y } else { min.y },
                if z { max.z } else { min.z },
            )
        };
        let quads = [
            [
                c(false, true, false),
                c(true, true, false),
                c(true, true, true),
                c(false, true, true),
            ],
            [
                c(false, false, false),
                c(false, false, true),
                c(true, false, true),
                c(true, false, false),
            ],
            [
                c(true, false, false),
                c(true, false, true),
                c(true, true, true),
                c(true, true, false),
            ],
            [
                c(false, false, false),
                c(false, true, false),
                c(false, true, true),
                c(false, false, true),
            ],
            [
                c(false, false, true),
                c(false, true, true),
                c(true, true, true),
                c(true, false, true),
            ],
            [
                c(false, false, false),
                c(true, false, false),
                c(true, true, false),
                c(false, true, false),
            ],
        ];
        quads
            .into_iter()
            .map(|corners| {
                let centre = corners.iter().copied().sum::<Vec3>() / 4.0;
                let mut normal = (corners[1] - corners[0])
                    .cross(corners[2] - corners[0])
                    .normalize();
                if normal.dot(centre - (min + max) / 2.0) < 0.0 {
                    normal = -normal;
                }
                Face {
                    corners: corners.to_vec(),
                    normal,
                }
            })
            .collect()
    }

    #[test]
    fn a_boxs_edges_are_all_worn() {
        let faces = boxed(Vec3::ZERO, Vec3::splat(10.0));
        let edges = worn_edges(&faces.iter().collect::<Vec<_>>(), &vec![false; faces.len()]);
        for (face, worn) in faces.iter().zip(&edges) {
            assert_eq!(worn.len(), 4);
            for e in worn {
                assert!(
                    e.weight > 0.9,
                    "a right angle is a sharp convex edge: {e:?}"
                );
                // Corners on the edge are at 0; the face's far side a whole side away.
                let d: Vec<f32> = face
                    .corners
                    .iter()
                    .map(|p| (*p - e.at).dot(e.inward))
                    .collect();
                assert!(
                    d.iter()
                        .all(|&d| d > -1e-4 && (d.abs() < 1e-4 || (d - 10.0).abs() < 1e-3)),
                    "{d:?}"
                );
            }
        }
    }

    #[test]
    fn a_dense_piece_is_not_searched_face_by_face() {
        // 5,000 lone triangles (a dense mod model's piece): every edge unshared, taken for open,
        // without looking along every other face for one (minutes, in pieces this dense).
        let faces: Vec<Face> = (0..5000)
            .map(|i| {
                let x = i as f32 * 3.0;
                Face { corners: vec![Vec3::new(x, 0.0, 0.0), Vec3::new(x + 1.0, 0.0, 0.0), Vec3::new(x, 1.0, 0.0)], normal: Vec3::Z }
            })
            .collect();
        let refs: Vec<&Face> = faces.iter().collect();
        let started = std::time::Instant::now();
        let open = open_edges(&refs, &vec![false; refs.len()], &vec![false; refs.len()]);
        let worn = worn_edges(&refs, &vec![false; refs.len()]);
        assert!(open.iter().all(|edges| edges.len() == 3 && edges.iter().all(|&o| o)));
        assert_eq!(worn.len(), faces.len());
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    }

    #[test]
    fn a_step_has_a_crease() {
        // A low block and a tall one side by side: where the low one's top meets the tall one's
        // side, the surface folds inwards.
        let mut faces = boxed(Vec3::ZERO, Vec3::new(10.0, 5.0, 10.0));
        faces.extend(boxed(
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(20.0, 15.0, 10.0),
        ));
        // (The two boxes' touching sides are inside the model: drop them.)
        faces.retain(|f| {
            !(f.corners.iter().all(|p| (p.x - 10.0).abs() < 1e-3)
                && f.corners.iter().all(|p| p.y <= 5.0 + 1e-3))
        });
        let refs: Vec<&Face> = faces.iter().collect();
        let edges = worn_edges(&refs, &vec![false; refs.len()]);
        let top = faces
            .iter()
            .position(|f| f.normal.y > 0.9 && f.corners.iter().all(|p| (p.y - 5.0).abs() < 1e-3))
            .expect("the low top");
        let crease = edges[top]
            .iter()
            .find(|e| (e.at.x - 10.0).abs() < 1e-3 && e.inward.x < -0.9)
            .expect("the edge against the wall");
        assert!(crease.weight < -0.5, "{crease:?}");
    }

    #[test]
    fn half_floats() {
        let back = |bits: u16| {
            let exp = ((bits >> 10) & 0x1F) as i32;
            let frac = (bits & 0x3FF) as f32 / 1024.0;
            if exp == 0 {
                0.0
            } else {
                (1.0 + frac) * 2f32.powi(exp - 15)
            }
        };
        for x in [0.0f32, 0.5, 1.0, 1.1, 3.75, 10.0, 250.0, 4096.0] {
            assert!((back(f16_bits(x)) - x).abs() <= x * 1e-3, "{x}");
        }
        assert_eq!(f16_bits(1e9), 0x7BFF);
    }
}
