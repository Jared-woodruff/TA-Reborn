//! The cockpit round a Perspective view (the game's `perspective.rs`): a model of the unit's crew
//! station, made in Blender by `tools/cockpits/build.py` (original art), drawn after the scene
//! and before the picture's post-processing (so its lamps bloom like the world's) with its own
//! depth, in the eye's own space: it moves with the view. The scene's sun and sky light it
//! through its openings; a cabin lamp, its screens and its lamps light it from inside.
//!
//! The models are GLB files read by the small reader here: triangles with positions, normals,
//! the masks baked into the first vertex colour (occlusion; the distance to the nearest edge;
//! occlusion close in; where hands go), decals' UVs, and materials whose names give their role
//! (see the builder) and whose extras how they wear: painted steel takes the side's paint
//! (`paint`) and wears to primer and bare steel (the shader, with the shared wear texture), `trim`
//! takes the side's colour, `screen.<name>` is an instrument's glass, `lamp.<colour>.<name>`
//! glows, `glass` is see-through, `decal` is paint from the shared atlas of stencils and plates.
//! Objects named `anim.<part>` move (levers, sticks, a helm): their vertices stay in the object's
//! own space and the game gives each a transform every frame.
//!
//! The instruments are pictures on their glass: each screen has a region of an atlas
//! ([`InstrumentAtlas`]) that the game paints every frame, and the glass shows it as a lit
//! display (phosphor glow and scan lines, under the glass's reflections), seen at its angle
//! and moving with the cockpit; and its glow lights what is round it.

use anyhow::{Result, anyhow, bail};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use crate::gpu::CheckedBufferInit;

use crate::glb::{Glb, material_factors, node_matrix};

use crate::util::{DEPTH_FORMAT, HDR_FORMAT, Target};

/// At most this many moving parts and materials.
pub const MAX_PARTS: usize = 16;
pub const MAX_MATERIALS: usize = 64;
/// At most this many instruments light the cockpit (a wide one counts thrice).
pub const MAX_LIGHTS: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CockpitVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// The masks baked into the model: ambient occlusion (1 open, 0 shut in); how far the
    /// nearest edge is (0..1 for 0..3 cm: the paint chips there); occlusion close in (1 open:
    /// grime gathers in seams and crevices); where hands go (1: scuffed, polished).
    pub masks: [f32; 4],
    /// Its place on the decal atlas (decals only).
    pub uv: [f32; 2],
    pub material: u32,
    /// The moving part it belongs to (0: the still body).
    pub part: u32,
}

impl Default for CockpitVertex {
    fn default() -> CockpitVertex {
        CockpitVertex { pos: [0.0; 3], normal: [0.0, 0.0, 1.0], masks: [1.0, 1.0, 1.0, 0.0], uv: [0.0; 2], material: 0, part: 0 }
    }
}

/// What a material is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Painted steel (the hull's paint, black enamel, hazard stripes).
    Surface = 0,
    /// Painted in the side's colour.
    Trim = 1,
    /// An instrument's glass.
    Screen = 2,
    /// A lamp's lens; `Alarm` ones turn red and blink when the hull is in danger.
    Lamp = 3,
    Alarm = 4,
    Glass = 5,
    /// Glows in the side's colour.
    TrimGlow = 6,
    /// Bare metal: steel, brass, a weld.
    Metal = 7,
    /// Rubber and padding.
    Rubber = 8,
    /// Canvas.
    Fabric = 9,
    /// Paint from the decal atlas (stencils, plates, labels): drawn after the rest, over it.
    Decal = 10,
    Concrete = 11,
}

impl Role {
    /// The role a material's name gives it (see the builder).
    pub fn of(name: &str) -> Role {
        let key = name.split('.').next().unwrap_or("");
        match key {
            "trim" => Role::Trim,
            "screen" => Role::Screen,
            "lamp" if name.contains("alarm") || name.contains("hull") => Role::Alarm,
            "lamp" => Role::Lamp,
            "glass" => Role::Glass,
            "glow_trim" => Role::TrimGlow,
            "steel" | "brass" | "weld" | "metal" => Role::Metal,
            "rubber" | "vinyl" => Role::Rubber,
            "canvas" => Role::Fabric,
            "decal" => Role::Decal,
            "concrete" => Role::Concrete,
            _ => Role::Surface,
        }
    }

    /// Whether what has it is drawn over what is opaque (blended), and in which turn: decals on
    /// the paint first, then the windows' glass.
    fn layer(self) -> u8 {
        match self {
            Role::Decal => 1,
            Role::Glass => 2,
            _ => 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CockpitMaterial {
    pub name: String,
    pub base: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
    pub role: Role,
    /// How much it wears (chips, scratches) and gathers grime: 0 none, 1 as the paint does.
    pub wear: f32,
    pub grime: f32,
    /// Painted in the side's own paint (the hull's), not its own colour.
    pub side_paint: bool,
}

impl CockpitMaterial {
    /// A material of that role, its other values plain (for tests and tools).
    pub fn plain(role: Role) -> CockpitMaterial {
        CockpitMaterial { name: String::new(), base: Vec3::ONE, metallic: 0.0, roughness: 0.5, emissive: Vec3::ZERO, role, wear: 0.0, grime: 0.0, side_paint: false }
    }
}

/// An instrument's glass: its middle and its half extents across and up (cockpit space), and
/// its material.
#[derive(Clone, Debug)]
pub struct Screen {
    pub name: String,
    pub centre: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub material: u32,
}

impl Screen {
    /// Its corners: bottom left, bottom right, top right, top left.
    pub fn corners(&self) -> [Vec3; 4] {
        let (c, r, u) = (self.centre, self.right, self.up);
        [c - r - u, c + r - u, c + r + u, c - r + u]
    }
}

/// A cockpit as read from its GLB.
#[derive(Clone, Debug, Default)]
pub struct CockpitModel {
    pub vertices: Vec<CockpitVertex>,
    pub indices: Vec<u32>,
    pub materials: Vec<CockpitMaterial>,
    /// Moving parts: name (without `anim.`) and where it rests; part 0 is the still body.
    pub parts: Vec<(String, Mat4)>,
    pub screens: Vec<Screen>,
}

impl CockpitModel {
    /// The index of a moving part by name.
    pub fn part(&self, name: &str) -> Option<usize> {
        self.parts.iter().position(|p| p.0 == name)
    }

    pub fn screen(&self, name: &str) -> Option<&Screen> {
        self.screens.iter().find(|s| s.name == name)
    }

    /// Reads a GLB (glTF 2.0 binary): every mesh of the default scene, in cockpit space (glTF's:
    /// x right, y up, the eye looking along -z).
    pub fn from_glb(bytes: &[u8]) -> Result<CockpitModel> {
        let glb = Glb::parse(bytes)?;
        let doc = &glb.doc;
        let materials: Vec<CockpitMaterial> = doc["materials"].as_array().map(|ms| ms.iter().map(read_material).collect()).unwrap_or_default();
        if materials.len() > MAX_MATERIALS {
            bail!("cockpit GLB: {} materials (at most {MAX_MATERIALS})", materials.len());
        }
        let mut model = CockpitModel { materials, parts: vec![(String::new(), Mat4::IDENTITY)], ..Default::default() };
        let roots = glb.roots();
        // (Depth first, in the file's order.) A node named `anim.<part>` and everything under it
        // is that moving part: its vertices go into the part's own space (the node's), wherever
        // the meshes hang under it (an optimiser may hang them from children that scale them).
        let mut stack: Vec<Visit> = roots.into_iter().rev().map(|n| (n, Mat4::IDENTITY, None)).collect();
        let mut screen_points: Vec<Vec<(Vec3, Vec3)>> = vec![Vec::new(); model.materials.len()];
        while let Some((index, parent, mut part_of)) = stack.pop() {
            let node = &doc["nodes"][index];
            let world = parent * node_matrix(node);
            if let Some(p) = node["name"].as_str().unwrap_or("").strip_prefix("anim.") {
                if model.parts.len() >= MAX_PARTS {
                    bail!("cockpit GLB: more than {MAX_PARTS} moving parts");
                }
                model.parts.push((p.to_string(), world));
                part_of = Some((model.parts.len() as u32 - 1, world));
            }
            if let Some(children) = node["children"].as_array() {
                stack.extend(children.iter().rev().filter_map(|c| c.as_u64()).map(|c| (c as usize, world, part_of)));
            }
            let Some(mesh) = node["mesh"].as_u64() else { continue };
            // A moving part keeps its own space; the rest is placed in the cockpit's.
            let (part, placed) = match part_of {
                Some((part, rest)) => (part, rest.inverse() * world),
                None => (0, world),
            };
            let normal_matrix = placed.inverse().transpose();
            for prim in doc["meshes"][mesh as usize]["primitives"].as_array().into_iter().flatten() {
                if prim["mode"].as_u64().unwrap_or(4) != 4 {
                    continue;
                }
                let attrs = &prim["attributes"];
                let positions = glb.read_vec(attrs["POSITION"].as_u64().ok_or_else(|| anyhow!("cockpit GLB: a primitive without positions"))? as usize, 3)?;
                let normals = match attrs["NORMAL"].as_u64() {
                    Some(a) => glb.read_vec(a as usize, 3)?,
                    None => vec![0.0; positions.len()],
                };
                // The masks (open, far from any edge, no hands where the model has none).
                let open = CockpitVertex::default().masks;
                let masks: Vec<[f32; 4]> = match attrs["COLOR_0"].as_u64() {
                    Some(a) => {
                        let width = glb.width(a as usize);
                        glb.read_vec(a as usize, width)?.chunks(width).map(|c| std::array::from_fn(|i| c.get(i).copied().unwrap_or(open[i]))).collect()
                    }
                    None => vec![open; positions.len() / 3],
                };
                let uvs = match attrs["TEXCOORD_0"].as_u64() {
                    Some(a) => glb.read_vec(a as usize, 2)?,
                    None => vec![0.0; positions.len() / 3 * 2],
                };
                let material = prim["material"].as_u64().unwrap_or(0) as u32;
                let base = model.vertices.len() as u32;
                for k in 0..positions.len() / 3 {
                    let p = placed.transform_point3(Vec3::new(positions[k * 3], positions[k * 3 + 1], positions[k * 3 + 2]));
                    let n = normal_matrix.transform_vector3(Vec3::new(normals[k * 3], normals[k * 3 + 1], normals[k * 3 + 2])).normalize_or_zero();
                    let uv = [uvs.get(k * 2).copied().unwrap_or(0.0), uvs.get(k * 2 + 1).copied().unwrap_or(0.0)];
                    model.vertices.push(CockpitVertex { pos: p.to_array(), normal: n.to_array(), masks: masks.get(k).copied().unwrap_or(open), uv, material, part });
                    if let Some(points) = screen_points.get_mut(material as usize).filter(|_| model.materials[material as usize].role == Role::Screen) {
                        points.push((p, n));
                    }
                }
                match prim["indices"].as_u64() {
                    Some(a) => model.indices.extend(glb.read_indices(a as usize)?.into_iter().map(|i| base + i)),
                    None => model.indices.extend(base..model.vertices.len() as u32),
                }
            }
        }
        // Each instrument's glass: the rectangle round it in its own plane (its middle, and its
        // extent across and up). (Not round the vertices' average: the bake subdivides the glass
        // unevenly, which pulls that to one side.)
        for (m, points) in screen_points.iter().enumerate() {
            if points.is_empty() {
                continue;
            }
            let mean = points.iter().map(|p| p.0).sum::<Vec3>() / points.len() as f32;
            let normal = points.iter().map(|p| p.1).sum::<Vec3>().normalize_or(Vec3::Z);
            let up = (Vec3::Y - normal * Vec3::Y.dot(normal)).normalize_or(Vec3::Y);
            let right = up.cross(normal).normalize_or(Vec3::X);
            let (mut lo, mut hi) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
            for (p, _) in points {
                let q = glam::Vec2::new((*p - mean).dot(right), (*p - mean).dot(up));
                lo = lo.min(q);
                hi = hi.max(q);
            }
            let (mid, half) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
            let centre = mean + right * mid.x + up * mid.y;
            let name = model.materials[m].name.trim_start_matches("screen.").to_string();
            model.screens.push(Screen { name, centre, right: right * half.x, up: up * half.y, material: m as u32 });
        }
        model.screens.sort_by(|a, b| a.name.cmp(&b.name));
        if model.vertices.is_empty() {
            bail!("cockpit GLB: no triangles");
        }
        // Drawn in one go, its triangles in turn: the decals blend over the opaque paint under
        // them, and the windows' glass over everything, so each comes after what it covers.
        let layer = |t: &[u32; 3]| model.materials.get(model.vertices[t[0] as usize].material as usize).map_or(0, |m| m.role.layer());
        let mut triangles: Vec<[u32; 3]> = model.indices.as_chunks::<3>().0.to_vec();
        triangles.sort_by_key(layer);
        model.indices = triangles.into_iter().flatten().collect();
        Ok(model)
    }
}

/// A node to visit: its index, its parent's world transform, and the moving part it belongs to
/// (its index and where it rests), if any.
type Visit = (usize, Mat4, Option<(u32, Mat4)>);

fn read_material(m: &serde_json::Value) -> CockpitMaterial {
    let name = m["name"].as_str().unwrap_or("").to_string();
    let (base, metallic, roughness, emissive) = material_factors(m);
    let base = Vec3::new(base[0], base[1], base[2]);
    let role = Role::of(&name);
    // How it wears, from its extras (none where it has none: glass, lamps, screens).
    let extra = |key: &str| m["extras"][key].as_f64().map_or(0.0, |v| v as f32).clamp(0.0, 2.0);
    CockpitMaterial { side_paint: name == "paint", wear: extra("wear"), grime: extra("grime"), name, base, metallic, roughness, emissive, role }
}

/// How the cockpit is this frame.
#[derive(Clone, Debug)]
pub struct CockpitFrame {
    /// Cockpit space to the eye's: the give of its mounting as the unit jolts.
    pub mount: Mat4,
    /// The vertical field of view it is seen with (radians): its kind's own, whatever the
    /// camera's is meanwhile (diving in, the world's widens to it; through the magnified sight
    /// it is narrower).
    pub fov_y: f32,
    /// Each moving part's transform from its rest (as `CockpitModel::parts`; the first is the
    /// still body and is ignored).
    pub parts: Vec<Mat4>,
    /// The side's colour on the trim (linear).
    pub trim: Vec3,
    /// The side's paint on the hull's steel (linear).
    pub paint: Vec3,
    /// 0..1: the lamps are on; the alarm (the hull in danger: its lamps red, blinking, and a red
    /// wash); a hit's flash.
    pub lamps: f32,
    pub alarm: f32,
    pub flash: f32,
    /// The cabin lamp's colour and strength (linear), and the screens' glow on what is round
    /// them.
    pub cabin: Vec3,
    pub glow: Vec3,
    /// How much of the sun gets in (0..1).
    pub sun: f32,
    /// Seconds, for blinking.
    pub time: f32,
    /// How hard it rains (0..1, drops on the glass), and whether it flies (blown back over a
    /// canopy rather than running down).
    pub rain: f32,
    pub flies: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    proj: [[f32; 4]; 4],
    mount: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky: [f32; 4],
    ground: [f32; 4],
    trim: [f32; 4],
    cabin: [f32; 4],
    glow: [f32; 4],
    params: [f32; 4],
    weather: [f32; 4],
    paint: [f32; 4],
    /// The instruments' glow as lights, two each (see the shader).
    lights: [[f32; 4]; MAX_LIGHTS * 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMaterial {
    base_rough: [f32; 4],
    emit_metal: [f32; 4],
    /// Its role, wear, grime, and 1 where it takes the side's paint.
    role: [f32; 4],
}

/// The buffers' sizes, as the shader must lay them out.
#[cfg(test)]
pub(crate) const UNIFORM_SIZE: usize = std::mem::size_of::<Uniform>();
#[cfg(test)]
pub(crate) const MATERIAL_SIZE: usize = std::mem::size_of::<GpuMaterial>();
#[cfg(test)]
pub(crate) const SCREEN_SIZE: usize = std::mem::size_of::<GpuScreen>();

/// An instrument's glow, lighting what is round it (cockpit space): where it shines from, the
/// way it faces, how far its light reaches and how bright it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenLight {
    pub at: Vec3,
    pub facing: Vec3,
    pub reach: f32,
    pub strength: f32,
}

/// The instruments' glow as lights: one at each screen's middle, as bright as it is large; a wide
/// one (a panoramic display) as three along it. Facing the eye; at most [`MAX_LIGHTS`].
pub fn screen_lights(screens: &[Screen]) -> Vec<ScreenLight> {
    let mut out = Vec::new();
    for s in screens {
        let (w, h) = (s.right.length() * 2.0, s.up.length() * 2.0);
        let mut facing = s.right.normalize_or_zero().cross(s.up.normalize_or_zero()).normalize_or(Vec3::Z);
        if facing.dot(-s.centre) < 0.0 {
            facing = -facing;
        }
        let pieces = if w > 0.3 { 3 } else { 1 };
        let area = w * h / pieces as f32;
        for k in 0..pieces {
            // (The middles of its thirds, or of the whole: -2/3, 0, 2/3 or 0 of its half width.)
            let along = (k as f32 + 0.5) / pieces as f32 * 2.0 - 1.0;
            let at = s.centre + s.right * along + facing * 0.01;
            out.push(ScreenLight { at, facing, reach: 0.06 + area.sqrt() * 0.9, strength: (area / 0.02).sqrt() * 1.1 });
        }
    }
    out.truncate(MAX_LIGHTS);
    out
}

/// A screen's glass for the shader, by material: its middle; its half extents across and up,
/// each with the inverse of its length squared; its region of the atlas (corner and size, in
/// the atlas's 0..1) and that region's height in pixels.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct GpuScreen {
    centre: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    region: [f32; 4],
    info: [f32; 4],
}

/// Atlas pixels to a metre of glass: about one and a half to every pixel the glass covers on a
/// 1080-line screen, which keeps the lettering sharp seen at an angle.
pub const ATLAS_DENSITY: f32 = 3200.0;
const ATLAS_WIDTH: u32 = 2048;
/// Pixels between regions (the glass samples between pixels).
const ATLAS_GAP: u32 = 6;

/// The pictures of a cockpit's instruments: one region of a texture per screen, which the game
/// paints (sRGB, premultiplied alpha, cleared every frame) and the glass shows.
pub struct InstrumentAtlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
    /// Each screen's region, by name: x, y, width, height in pixels.
    pub regions: Vec<(String, [u32; 4])>,
}

impl InstrumentAtlas {
    pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    /// Regions for these screens, as big as their glass (shelves, tallest first).
    fn layout(screens: &[Screen]) -> (Vec<[u32; 4]>, (u32, u32)) {
        let sizes: Vec<(u32, u32)> = screens
            .iter()
            .map(|s| {
                let px = |half: Vec3, most: u32| ((half.length() * 2.0 * ATLAS_DENSITY).round() as u32).clamp(32, most);
                (px(s.right, ATLAS_WIDTH), px(s.up, 1024))
            })
            .collect();
        let mut order: Vec<usize> = (0..sizes.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(sizes[i].1));
        let mut regions = vec![[0u32; 4]; sizes.len()];
        let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
        for i in order {
            let (w, h) = sizes[i];
            if x > 0 && x + w > ATLAS_WIDTH {
                x = 0;
                y += shelf + ATLAS_GAP;
                shelf = 0;
            }
            regions[i] = [x, y, w, h];
            x += w + ATLAS_GAP;
            shelf = shelf.max(h);
        }
        (regions, (ATLAS_WIDTH, (y + shelf).next_power_of_two().max(64)))
    }

    fn new(device: &wgpu::Device, screens: &[Screen]) -> InstrumentAtlas {
        let (rects, size) = Self::layout(screens);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cockpit instruments"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        InstrumentAtlas { texture, view, size, regions: screens.iter().map(|s| s.name.clone()).zip(rects).collect() }
    }
}

/// A PNG as RGBA8 with its mipmaps (each a quarter of the one before, its colours averaged as
/// light where `srgb`), down to one pixel.
pub fn mipmapped(png: &[u8], srgb: bool) -> Result<(u32, u32, Vec<Vec<u8>>)> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.to_rgba8();
    let (w, h) = image.dimensions();
    let mut levels = vec![image.into_raw()];
    let (mut lw, mut lh) = (w, h);
    let lut = srgb.then(crate::util::srgb_to_linear_lut);
    while lw > 1 || lh > 1 {
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
        let src = levels.last().expect("a level");
        let mut out = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let i = (((y * 2 + dy).min(lh - 1) * lw + (x * 2 + dx).min(lw - 1)) * 4) as usize;
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a += match (&lut, k) {
                            (Some(l), 0..=2) => l[src[i + k] as usize],
                            _ => src[i + k] as f32 / 255.0,
                        } * 0.25;
                    }
                }
                let o = ((y * nw + x) * 4) as usize;
                for (k, a) in acc.iter().enumerate() {
                    out[o + k] = match (&lut, k) {
                        (Some(_), 0..=2) => crate::util::linear_to_srgb(*a),
                        _ => (a * 255.0 + 0.5).clamp(0.0, 255.0) as u8,
                    };
                }
            }
        }
        levels.push(out);
        (lw, lh) = (nw, nh);
    }
    Ok((w, h, levels))
}

/// A texture of `format` from a PNG, with its mipmaps (a pixel of grey where it does not read).
fn texture_from_png(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, png: &[u8], format: wgpu::TextureFormat) -> wgpu::TextureView {
    let (w, h, levels) = mipmapped(png, format.is_srgb()).unwrap_or_else(|e| {
        log::warn!("{label}: {e}");
        (1, 1, vec![vec![128, 128, 128, 0]])
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, data) in levels.iter().enumerate() {
        crate::util::write_texture_level(queue, &texture, level as u32, (w >> level).max(1), (h >> level).max(1), data, 4);
    }
    texture.create_view(&Default::default())
}

/// The GPU side of a cockpit.
pub struct CockpitRenderer {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    uniform: wgpu::Buffer,
    parts: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    samples: u32,
    depth: Option<(u32, u32, Target)>,
    /// Where each moving part rests.
    rests: Vec<Mat4>,
    /// The instruments' glow (cockpit space).
    lights: Vec<ScreenLight>,
    pub instruments: InstrumentAtlas,
}

impl CockpitRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, model: &CockpitModel, samples: u32) -> CockpitRenderer {
        // (`TA_COCKPITS=<folder>` with a `cockpit.wgsl` in it: that one, to try it unbuilt.)
        let source = std::env::var_os("TA_COCKPITS").and_then(|dir| std::fs::read_to_string(std::path::Path::new(&dir).join("cockpit.wgsl")).ok());
        let module = crate::util::shader_standalone(device, "cockpit", source.as_deref().unwrap_or(include_str!("shaders/cockpit.wgsl")));
        let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let texture = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cockpit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                storage(1),
                storage(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                storage(5),
                texture(6),
                texture(7),
                wgpu::BindGroupLayoutEntry { binding: 8, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 9, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("cockpit"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2, 4 => Uint32, 5 => Uint32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cockpit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<CockpitVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            multiview_mask: None,
            cache: None,
        });
        let vertices = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("cockpit vertices"), contents: bytemuck::cast_slice(&model.vertices), usage: wgpu::BufferUsages::VERTEX });
        let indices = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("cockpit indices"), contents: bytemuck::cast_slice(&model.indices), usage: wgpu::BufferUsages::INDEX });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("cockpit uniform"), size: std::mem::size_of::<Uniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let parts = device.create_buffer(&wgpu::BufferDescriptor { label: Some("cockpit parts"), size: (MAX_PARTS * 64) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mats: Vec<GpuMaterial> = (0..MAX_MATERIALS)
            .map(|i| {
                let m = model.materials.get(i);
                let plain = CockpitMaterial::plain(Role::Surface);
                let m = m.unwrap_or(&plain);
                let (base, emit) = (m.base, m.emissive);
                GpuMaterial {
                    base_rough: [base.x, base.y, base.z, m.roughness],
                    emit_metal: [emit.x, emit.y, emit.z, m.metallic],
                    role: [m.role as u32 as f32, m.wear, m.grime, if m.side_paint { 1.0 } else { 0.0 }],
                }
            })
            .collect();
        let materials = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("cockpit materials"), contents: bytemuck::cast_slice(&mats), usage: wgpu::BufferUsages::STORAGE });
        // The instruments: the atlas, and each screen's glass with its region, by material.
        let instruments = InstrumentAtlas::new(device, &model.screens);
        let mut screens = vec![GpuScreen::default(); MAX_MATERIALS];
        let (aw, ah) = (instruments.size.0 as f32, instruments.size.1 as f32);
        for (s, (_, r)) in model.screens.iter().zip(&instruments.regions) {
            let Some(g) = screens.get_mut(s.material as usize) else { continue };
            let inv = |v: Vec3| 1.0 / v.length_squared().max(1e-12);
            *g = GpuScreen {
                centre: [s.centre.x, s.centre.y, s.centre.z, 1.0],
                right: [s.right.x, s.right.y, s.right.z, inv(s.right)],
                up: [s.up.x, s.up.y, s.up.z, inv(s.up)],
                region: [r[0] as f32 / aw, r[1] as f32 / ah, r[2] as f32 / aw, r[3] as f32 / ah],
                info: [r[3] as f32, r[2] as f32, 0.0, 0.0],
            };
        }
        let screens = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("cockpit screens"), contents: bytemuck::cast_slice(&screens), usage: wgpu::BufferUsages::STORAGE });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cockpit instruments"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        // The wear (repeating across every face, filtered steeply: the console is seen at a
        // slant) and the decals.
        let (wear_png, decals_png) = crate::cockpit_textures();
        let wear = texture_from_png(device, queue, "cockpit wear", wear_png, wgpu::TextureFormat::Rgba8Unorm);
        let decals = texture_from_png(device, queue, "cockpit decals", decals_png, wgpu::TextureFormat::Rgba8UnormSrgb);
        let mipped = |label: &'static str, address: wgpu::AddressMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: address,
                address_mode_v: address,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                anisotropy_clamp: 8,
                ..Default::default()
            })
        };
        let wrap = mipped("cockpit wear", wgpu::AddressMode::Repeat);
        let clamp = mipped("cockpit decals", wgpu::AddressMode::ClampToEdge);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cockpit"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: parts.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: materials.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&instruments.view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 5, resource: screens.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&wear) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&decals) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&wrap) },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(&clamp) },
            ],
        });
        CockpitRenderer {
            pipeline,
            vertices,
            indices,
            count: model.indices.len() as u32,
            uniform,
            parts,
            bind_group,
            samples,
            depth: None,
            rests: model.parts.iter().map(|p| p.1).collect(),
            lights: screen_lights(&model.screens),
            instruments,
        }
    }

    /// Draws the cockpit over the scene's picture (`color`, resolved into `resolve` when
    /// multisampled), `size` pixels, with its own depth (its time on the GPU taken with
    /// `timestamps`, when they are).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        (color, resolve): (&wgpu::TextureView, Option<&wgpu::TextureView>),
        size: (u32, u32),
        camera: &crate::Camera,
        lighting: &crate::Lighting,
        frame: &CockpitFrame,
        timestamps: Option<&crate::timing::PassTimestamps>,
    ) {
        if self.depth.as_ref().is_none_or(|d| (d.0, d.1) != size) {
            self.depth = Some((size.0, size.1, Target::new(device, "cockpit depth", size.0, size.1, DEPTH_FORMAT, self.samples, wgpu::TextureUsages::RENDER_ATTACHMENT)));
        }
        let aspect = camera.viewport.x / camera.viewport.y.max(1.0);
        let proj = glam::camera::rh::proj::directx::perspective(frame.fov_y, aspect, 0.01, 30.0);
        // The world's light, turned into the eye's space.
        let view = camera.view();
        let sun = view.transform_vector3(lighting.sun_dir.normalize_or(Vec3::Y)).normalize_or(Vec3::Y);
        let sun_color = lighting.sun_color * lighting.sun_intensity;
        let sky = lighting.fog_color.lerp(lighting.ambient, 0.5);
        // The instruments' glow, where they are this frame (moving with the cockpit).
        let mut lights = [[0.0f32; 4]; MAX_LIGHTS * 2];
        for (k, l) in self.lights.iter().enumerate().take(MAX_LIGHTS) {
            let at = frame.mount.transform_point3(l.at);
            let facing = frame.mount.transform_vector3(l.facing).normalize_or(Vec3::Z);
            lights[k * 2] = [at.x, at.y, at.z, l.reach];
            lights[k * 2 + 1] = [facing.x, facing.y, facing.z, l.strength];
        }
        let u = Uniform {
            proj: proj.to_cols_array_2d(),
            mount: frame.mount.to_cols_array_2d(),
            sun_dir: [sun.x, sun.y, sun.z, frame.sun],
            sun_color: [sun_color.x, sun_color.y, sun_color.z, 0.0],
            sky: [sky.x, sky.y, sky.z, 0.0],
            ground: [lighting.ambient.x * 0.5, lighting.ambient.y * 0.45, lighting.ambient.z * 0.4, 0.0],
            trim: [frame.trim.x, frame.trim.y, frame.trim.z, 0.0],
            cabin: [frame.cabin.x, frame.cabin.y, frame.cabin.z, 0.0],
            glow: [frame.glow.x, frame.glow.y, frame.glow.z, 0.0],
            params: [frame.time, frame.alarm, frame.flash, frame.lamps],
            weather: [frame.rain, if frame.flies { 1.0 } else { 0.0 }, 0.0, 0.0],
            paint: [frame.paint.x, frame.paint.y, frame.paint.z, self.lights.len().min(MAX_LIGHTS) as f32],
            lights,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        let parts: Vec<[[f32; 4]; 4]> = (0..MAX_PARTS)
            .map(|i| match self.rests.get(i).filter(|_| i > 0) {
                Some(rest) => *rest * frame.parts.get(i).copied().unwrap_or(Mat4::IDENTITY),
                None => Mat4::IDENTITY,
            })
            .map(|m| m.to_cols_array_2d())
            .collect();
        queue.write_buffer(&self.parts, 0, bytemuck::cast_slice(&parts));
        let Some((_, _, depth)) = &self.depth else { return };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cockpit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color, resolve_target: resolve, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }, depth_slice: None })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth.view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: timestamps.map(|t| t.writes()),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.count, 0, 0..1);
    }
}

/// Where a point of the cockpit (cockpit space, moved by `mount`) lands on the screen, in
/// pixels, seen with the field of view `fov_y` (radians, as [`CockpitFrame::fov_y`]) in the
/// camera's viewport; none behind the eye.
pub fn project(camera: &crate::Camera, fov_y: f32, mount: Mat4, p: Vec3) -> Option<glam::Vec2> {
    let aspect = camera.viewport.x / camera.viewport.y.max(1.0);
    let proj = glam::camera::rh::proj::directx::perspective(fov_y, aspect, 0.01, 30.0);
    let c = proj * mount * p.extend(1.0);
    if c.w <= 1e-4 {
        return None;
    }
    let ndc = glam::Vec2::new(c.x, c.y) / c.w;
    Some(glam::Vec2::new((ndc.x + 1.0) * 0.5 * camera.viewport.x, (1.0 - ndc.y) * 0.5 * camera.viewport.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GLB of one triangle with a named material and a moving node.
    fn tiny_glb() -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        for v in [0.0f32, 0.0, -1.0, 1.0, 0.0, -1.0, 0.0, 1.0, -1.0] {
            bin.extend(v.to_le_bytes());
        }
        for v in [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0] {
            bin.extend(v.to_le_bytes());
        }
        for i in [0u16, 1, 2, 0] {
            bin.extend(i.to_le_bytes());
        }
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0, 1]}],
            "nodes": [{"name": "static.screen.radar", "mesh": 0}, {"name": "anim.lever_l", "translation": [0.5, 0.0, 0.0], "children": [2]}, {"mesh": 0, "scale": [2.0, 2.0, 2.0]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "material": 0}]}],
            "materials": [{"name": "screen.radar", "pbrMetallicRoughness": {"baseColorFactor": [0.1, 0.2, 0.3, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.2}}],
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 36}, {"buffer": 0, "byteOffset": 72, "byteLength": 6}],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ]
        });
        let mut json = serde_json::to_vec(&json).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut out = Vec::new();
        out.extend(b"glTF");
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(0x4E4F_534Au32.to_le_bytes());
        out.extend(&json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(0x004E_4942u32.to_le_bytes());
        out.extend(&bin);
        out
    }

    #[test]
    fn reads_a_glb_with_its_parts_and_screens() {
        let m = CockpitModel::from_glb(&tiny_glb()).unwrap();
        assert_eq!(m.vertices.len(), 6);
        assert_eq!(m.indices.len(), 6);
        assert_eq!(m.parts.len(), 2);
        assert_eq!(m.part("lever_l"), Some(1));
        // The moving part keeps its own space (the child's scale applied); its rest is where it
        // sits.
        assert_eq!(m.vertices[3].part, 1);
        assert_eq!(m.vertices[4].pos, [2.0, 0.0, -2.0]);
        assert!((m.parts[1].1.w_axis.x - 0.5).abs() < 1e-6);
        assert_eq!(m.materials[0].role, Role::Screen);
        let s = m.screen("radar").unwrap();
        assert!(s.right.x > 0.0 && s.up.y > 0.0, "{s:?}");
        assert_eq!(s.material, 0);
    }

    #[test]
    fn every_screen_gets_its_own_region_of_the_atlas() {
        for (name, bytes) in crate::cockpit_models() {
            let m = CockpitModel::from_glb(bytes).unwrap();
            let (regions, (w, h)) = InstrumentAtlas::layout(&m.screens);
            assert!(w <= 2048 && h <= 2048, "{name}: atlas {w}x{h}");
            for (i, a) in regions.iter().enumerate() {
                assert!(a[0] + a[2] <= w && a[1] + a[3] <= h, "{name}: {a:?} outside {w}x{h}");
                // As big as the glass (a disc's square, a screen's oblong).
                let s = &m.screens[i];
                assert!((a[2] as f32 / a[3] as f32 - s.right.length() / s.up.length()).abs() < 0.05, "{name} {}: {a:?}", s.name);
                for b in &regions[i + 1..] {
                    let apart = a[0] + a[2] <= b[0] || b[0] + b[2] <= a[0] || a[1] + a[3] <= b[1] || b[1] + b[3] <= a[1];
                    assert!(apart, "{name}: {a:?} and {b:?} overlap");
                }
            }
        }
    }

    #[test]
    fn the_cockpits_shipped_load() {
        for (name, bytes) in crate::cockpit_models() {
            let m = CockpitModel::from_glb(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(m.indices.len() >= 300, "{name}: {} indices", m.indices.len());
            assert!(!m.screens.is_empty(), "{name}: no instruments");
            // Every moving part is one the game knows by its name, and has its vertices (the
            // nodes that dequantise a part's mesh are not parts of their own).
            for (i, (part, _)) in m.parts.iter().enumerate().skip(1) {
                assert!(!part.contains('.'), "{name}: a part named {part}");
                assert!(m.vertices.iter().any(|v| v.part == i as u32), "{name}: {part} has nothing");
            }
        }
    }

    #[test]
    fn the_names_give_the_roles() {
        for (name, role) in [
            ("paint", Role::Surface),
            ("paint_dark", Role::Surface),
            ("hazard_black", Role::Surface),
            ("trim", Role::Trim),
            ("steel", Role::Metal),
            ("brass", Role::Metal),
            ("weld", Role::Metal),
            ("rubber", Role::Rubber),
            ("vinyl", Role::Rubber),
            ("canvas", Role::Fabric),
            ("decal", Role::Decal),
            ("concrete", Role::Concrete),
            ("glass", Role::Glass),
            ("glow_trim", Role::TrimGlow),
            ("screen.radar", Role::Screen),
            ("lamp.green.status0", Role::Lamp),
            ("lamp.alarm.hull.status2", Role::Alarm),
        ] {
            assert_eq!(Role::of(name), role, "{name}");
        }
    }

    /// The shipped cockpits are aged: their paint takes the side's and wears, its edges are
    /// found (some of it on them, more of it clear of them), the occlusion is baked, and hands
    /// go somewhere; every decal shows a piece of the atlas; and the one draw goes opaque, then
    /// decals, then glass.
    #[test]
    fn the_cockpits_carry_their_wear() {
        for (name, bytes) in crate::cockpit_models() {
            let m = CockpitModel::from_glb(bytes).unwrap();
            let paint = m.materials.iter().position(|x| x.name == "paint").unwrap_or_else(|| panic!("{name}: no paint")) as u32;
            assert!(m.materials[paint as usize].side_paint && m.materials[paint as usize].wear > 0.5, "{name}: {:?}", m.materials[paint as usize]);
            let painted: Vec<&CockpitVertex> = m.vertices.iter().filter(|v| v.material == paint).collect();
            let share = |f: &dyn Fn(&CockpitVertex) -> bool| painted.iter().filter(|v| f(v)).count() as f32 / painted.len() as f32;
            let on_edges = share(&|v| v.masks[1] < 0.2);
            let clear = share(&|v| v.masks[1] > 0.99);
            assert!((0.05..0.85).contains(&on_edges) && clear > 0.03, "{name}: {on_edges} of the paint on its edges, {clear} clear of them");
            assert!(share(&|v| v.masks[0] < 0.5) > 0.02 && share(&|v| v.masks[0] > 0.75) > 0.02, "{name}: the occlusion is not baked");
            assert!(share(&|v| v.masks[2] < 0.7) > 0.02, "{name}: no crevices");
            assert!(m.vertices.iter().any(|v| v.masks[3] > 0.5), "{name}: no hands");
            let decal = m.materials.iter().position(|x| x.role == Role::Decal).unwrap_or_else(|| panic!("{name}: no decals")) as u32;
            for v in m.vertices.iter().filter(|v| v.material == decal) {
                assert!(v.uv.iter().all(|c| (0.0..=1.0).contains(c)), "{name}: a decal off the atlas at {:?}", v.uv);
            }
            let layers: Vec<u8> = m.indices.as_chunks::<3>().0.iter().map(|t| m.materials[m.vertices[t[0] as usize].material as usize].role.layer()).collect();
            assert!(layers.windows(2).all(|w| w[0] <= w[1]), "{name}: decals or glass drawn before what they cover");
        }
    }

    #[test]
    fn the_shared_textures_read_with_their_mipmaps() {
        let (wear, decals) = crate::cockpit_textures();
        let (w, h, levels) = mipmapped(wear, false).unwrap();
        assert_eq!((w, h), (512, 512));
        assert_eq!(levels.len(), 10);
        assert_eq!(levels.last().unwrap().len(), 4);
        // The chip field and the grain are spread over their whole range (a threshold keeps a
        // share of the picture as large as it says).
        let mean = |k: usize| levels[0].iter().skip(k).step_by(4).map(|&b| b as f32 / 255.0).sum::<f32>() / (w * h) as f32;
        for k in [0, 2, 3] {
            assert!((mean(k) - 0.5).abs() < 0.02, "channel {k}: mean {}", mean(k));
        }
        let (dw, dh, dl) = mipmapped(decals, true).unwrap();
        assert!(dw == 1024 && dh >= 512 && dl.len() >= 10, "{dw}x{dh}, {} levels", dl.len());
    }

    #[test]
    fn the_instruments_light_what_is_round_them() {
        for (name, bytes) in crate::cockpit_models() {
            let m = CockpitModel::from_glb(bytes).unwrap();
            let lights = screen_lights(&m.screens);
            assert!(!lights.is_empty() && lights.len() <= MAX_LIGHTS, "{name}: {} lights", lights.len());
            for l in &lights {
                // Out of the glass, towards the eye.
                assert!(l.facing.dot(-l.at) > 0.0 && l.strength > 0.0 && l.reach > 0.05, "{name}: {l:?}");
            }
        }
        // A wide display lights as three.
        let wide = Screen { name: "pcd".into(), centre: Vec3::new(0.0, -0.2, -0.5), right: Vec3::new(0.3, 0.0, 0.0), up: Vec3::new(0.0, 0.06, 0.03), material: 0 };
        let three = screen_lights(std::slice::from_ref(&wide));
        assert_eq!(three.len(), 3);
        assert!((three[0].at.x + 0.2).abs() < 1e-4 && three[1].at.x.abs() < 1e-4 && (three[2].at.x - 0.2).abs() < 1e-4, "{three:?}");
    }
}
