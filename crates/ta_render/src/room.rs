//! A room to watch the intro in: a teenager's bedroom late at night in the autumn of 1997, lit
//! by the CRT on the desk that plays TA's movies.
//!
//! The room is a model made in Blender by `tools/room/build.py` (original art, `room/room.glb`)
//! and read by [`RoomModel::from_glb`]: its surfaces, their materials by role (see [`Role`]),
//! the light Cycles baked into their corners (the moonlight's bounce and the occlusion, what
//! each quarter of the tube throws round the room, the lava lamp's glow), and anchors (the
//! tube's glass, the eye, the moon, the lamps). The game paints the pictures on it (posters,
//! the game's boxes, the keys' legends: [`RoomScene::layers`]), hands the renderer each movie
//! frame ([`Renderer::room_picture`](crate::Renderer::room_picture)) and, every frame, the camera
//! and how the room is doing ([`RoomFrame`]).
//!
//! Besides the baked light, the moon and the tube light it directly, with shadows: the moon
//! through the blinds (an orthographic shadow map along its light, the stripes as soft as the
//! blinds are far), the tube as an area light of 3×3 patches, each the colour of its part of
//! the picture (a shadow map from its middle, its shadows as soft as the glass is wide), its
//! reflection in the desk, the keys and the mug showing the picture itself. A tree outside
//! sways in the moonlight, the lava lamp's wax is traced through its glass, dust drifts in the
//! beams, the coffee steams, and the air glows where the light crosses it (a march through the
//! shadow maps at half resolution). See `shaders/room.wgsl`.

use anyhow::{Result, anyhow, bail};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;
use crate::gpu::CheckedBufferInit;

use crate::glb::{Glb, material_factors};
use crate::util::{DEPTH_FORMAT, HDR_FORMAT, Target};

/// The room's model (`TA_ROOM_MODEL=<file.glb>` plays another in its place, to try
/// `tools/room/build.py`'s output without building the game again).
pub fn room_model() -> &'static [u8] {
    static OTHER: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    OTHER.get_or_init(|| std::env::var_os("TA_ROOM_MODEL").and_then(|path| std::fs::read(path).ok())).as_deref().unwrap_or(include_bytes!("../room/room.glb"))
}

/// Picture layers are this many pixels square (a picture of another shape is stretched to fill
/// one; its UVs span it).
pub const LAYER_SIZE: u32 = 1024;
const LAYER_MIPS: u32 = 11;
/// At most this many materials.
pub const MAX_MATERIALS: usize = 192;
/// The glowing parts the game drives, by their materials' names (`emit.<name>`): the order of
/// [`RoomFrame::glows`]. Any other glows always.
pub const GLOW_NAMES: [&str; 20] = [
    "led_power", "led_hdd", "led_monitor", "modem0", "modem1", "modem2", "modem3", "modem4", "modem5", "modem6", "modem7", "digits", "window_lit", "clock",
    // Christmas: the tree's lights in four colours, the strings of fairy lights, the star.
    "xmas_red", "xmas_green", "xmas_gold", "xmas_blue", "fairy", "star",
];
/// Pictures that glow (displays): the glow they glow with, how much of the room's light they
/// show as they are (LEDs behind a dark filter little) and how bright they glow.
const GLOWING_PICTURES: [(&str, &str, f32, f32); 2] = [("turbo", "digits", 0.2, 3.5), ("clock", "clock", 0.2, 3.5)];
pub const GLOWS: usize = 24;
/// How far each of the season's three lights reaches (metres): the tree's (or the party's), and
/// the two strings of lights.
const FESTIVE_REACH: [f32; 3] = [1.0, 1.5, 1.5];
/// The glass as a grid of this many cells across and down.
const GRID: (u32, u32) = (48, 36);
/// The shadow maps' sizes.
const MOON_MAP: u32 = 2048;
const TUBE_MAP: u32 = 2048;
/// The tube's shadow map: its field of view, and its depth range (metres).
const TUBE_FOV: f32 = 140.0;
const TUBE_NEAR: f32 = 0.01;
const TUBE_FAR: f32 = 5.0;
/// The moon's shadow map: how far before the window it starts, and how deep it reaches.
const MOON_BACK: f32 = 2.0;
const MOON_DEPTH: f32 = 8.0;
/// The moon's angular radius (radians: its stripes soften with distance from the blinds).
const MOON_RADIUS: f32 = 0.0055;
/// Dust motes drifting in the light, and puffs of the coffee's steam.
const MOTES: u32 = 1400;
const STEAM: u32 = 36;
/// How `tools/room/build.py` scales the baked light (each channel is the square root of the
/// light over its scale).
const BOUNCE_SCALE: f32 = 4.0;
const TUBE_SCALE: f32 = 1.0;
const LAVA_SCALE: f32 = 4.0;
/// The moonlight (irradiance, linear colour), as the bakes had it.
const MOON_IRRADIANCE: f32 = 3.0;
const MOON_COLOR: [f32; 3] = [0.55, 0.68, 1.0];
/// The lava lamp's light.
const LAVA_COLOR: [f32; 3] = [1.0, 0.42, 0.12];
/// How near white the tube's white shows (its glass shows the movies in their own colours,
/// whatever the room's exposure), and the radiance of its white as it lights the room.
const SCREEN_WHITE: f32 = 0.9;
const SCREEN_LIGHT: f32 = 4.0;
/// The light the tube throws back off whoever sits at it, as a soft fill from the seat.
const SEAT_FILL: f32 = 0.04;

/// The season the room is dressed for. `tools/room/build.py` tags what each season puts up in
/// the model's node names: `season.christmas.<material>` is there only at Christmas; anything
/// else always.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Season {
    #[default]
    Normal,
    Christmas,
    NewYear,
}

impl Season {
    /// The name the model's tags use.
    pub fn name(self) -> &'static str {
        match self {
            Season::Normal => "normal",
            Season::Christmas => "christmas",
            Season::NewYear => "newyear",
        }
    }

    /// A season by its name (`normal`, `christmas`, `newyear`; any case, spaces, `-` and `_` left out).
    pub fn parse(text: &str) -> Option<Season> {
        let text: String = text.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()).collect();
        match text.as_str() {
            "normal" | "none" | "off" => Some(Season::Normal),
            "christmas" | "xmas" => Some(Season::Christmas),
            "newyear" | "newyears" => Some(Season::NewYear),
            _ => None,
        }
    }
}

/// A node's name as the room of `season` has it, or None when the node is not put up then.
fn in_season(name: &str, season: Season) -> Option<&str> {
    match name.strip_prefix("season.") {
        Some(rest) => {
            let (which, rest) = rest.split_once('.')?;
            (which == season.name()).then_some(rest)
        }
        None => Some(name),
    }
}

/// A corner of a surface.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct RoomVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// The baked light, each channel the square root of the light over its scale (0..65535): the
    /// moonlight's bounce (rgb) and the occlusion (a, 1 open); the tube's quarters (top left,
    /// top right, bottom left, bottom right); the lava lamp's light (rgb).
    pub light: [u16; 4],
    pub tube: [u16; 4],
    pub lava: [u16; 4],
    pub material: u32,
}

/// What a material is, as its name in the model says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Plain = 0,
    /// The desk's varnished wood.
    Wood = 1,
    /// The shelves' wood-grain laminate.
    Laminate = 2,
    /// Painted walls, ceiling and woodwork.
    Paint = 3,
    Carpet = 4,
    /// Moulded plastic: the computer's cases, the keys.
    Plastic = 5,
    /// Painted by the game: a layer of [`RoomScene::layers`].
    Picture = 6,
    /// See-through: the window's pane.
    Glass = 7,
    /// The lava lamp's glass, its wax traced inside.
    Lava = 8,
    /// Glows by itself (LEDs, the street's lit windows), as bright as the game says.
    Glow = 9,
    Sky = 10,
    Moon = 11,
    Fabric = 12,
    /// Paper and card: books, boxes, magazines.
    Paper = 13,
    /// Outside: the houses and the tree against the sky.
    Backdrop = 14,
    Coffee = 15,
}

impl Role {
    fn of(name: &str) -> Role {
        match name {
            "wood" | "pencil" => Role::Wood,
            "laminate" | "cork" => Role::Laminate,
            "paint" | "paint_white" | "ceiling" => Role::Paint,
            "carpet" => Role::Carpet,
            "plastic" | "plastic_dark" | "seam" | "keycap" | "keycap_dark" | "grille" | "rubber" => Role::Plastic,
            "glass" => Role::Glass,
            "lava" => Role::Lava,
            "sky" => Role::Sky,
            "moon" => Role::Moon,
            "silhouette" | "silhouette_snow" | "silhouette_snow_near" => Role::Backdrop,
            "coffee" => Role::Coffee,
            "paper" | "paper_yellow" | "cardboard" | "tape" => Role::Paper,
            n if n.starts_with("picture.") => Role::Picture,
            n if n.starts_with("emit.") => Role::Glow,
            n if n.starts_with("fabric") => Role::Fabric,
            n if n.starts_with("book_") || n.starts_with("box_") => Role::Paper,
            _ => Role::Plain,
        }
    }

    /// Whether it stands in the light's way (the world outside is a backdrop, the pane lets the
    /// light through).
    fn casts_shadow(self) -> bool {
        !matches!(self, Role::Glass | Role::Sky | Role::Moon | Role::Backdrop)
    }
}

#[derive(Clone, Debug)]
pub struct RoomMaterial {
    pub name: String,
    pub role: Role,
    /// Linear colour and alpha.
    pub base: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
    /// A picture's layer, a glow's slot in [`RoomFrame::glows`].
    pub index: u32,
}

/// A picture the game paints: its name (the material's, without `picture.`) and shape (width
/// over height).
#[derive(Clone, Debug)]
pub struct RoomPicture {
    pub name: String,
    pub aspect: f32,
}

/// A named place in the room (an empty in the model, `anchor.<name>`), and what it carries.
#[derive(Clone, Debug)]
pub struct RoomAnchor {
    pub name: String,
    pub transform: Mat4,
    pub extras: serde_json::Value,
}

/// The tube's glass: its middle, half its width and height as vectors (its face towards the
/// room is `right × up`), and how far it bulges at the middle.
#[derive(Clone, Copy, Debug)]
pub struct RoomScreen {
    pub center: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub bulge: f32,
}

impl RoomScreen {
    pub fn normal(&self) -> Vec3 {
        self.right.cross(self.up).normalize_or(Vec3::Z)
    }
}

/// The room as read from its GLB, in its own space (glTF's: metres, x to the right, y up, the
/// viewer looking along -z at the wall behind the desk).
#[derive(Clone, Debug, Default)]
pub struct RoomModel {
    pub vertices: Vec<RoomVertex>,
    pub indices: Vec<u32>,
    pub materials: Vec<RoomMaterial>,
    pub pictures: Vec<RoomPicture>,
    pub anchors: Vec<RoomAnchor>,
    /// What it is dressed for.
    pub season: Season,
}

impl RoomModel {
    /// The room as `season` has it: what the model tags for other seasons is left out.
    pub fn from_glb(bytes: &[u8], season: Season) -> Result<RoomModel> {
        let glb = Glb::parse(bytes)?;
        let doc = &glb.doc;
        let mut model = RoomModel { season, ..RoomModel::default() };
        for m in doc["materials"].as_array().into_iter().flatten() {
            let name = m["name"].as_str().unwrap_or("").to_string();
            let (base, metallic, roughness, emissive) = material_factors(m);
            let role = Role::of(&name);
            let index = match role {
                Role::Picture => {
                    model.pictures.push(RoomPicture { name: name["picture.".len()..].to_string(), aspect: m["extras"]["aspect"].as_f64().unwrap_or(1.0) as f32 });
                    model.pictures.len() as u32 - 1
                }
                Role::Glow => GLOW_NAMES.iter().position(|g| name.strip_prefix("emit.") == Some(*g)).unwrap_or(GLOWS - 1) as u32,
                _ => 0,
            };
            model.materials.push(RoomMaterial { name, role, base, metallic, roughness, emissive, index });
        }
        if model.materials.len() > MAX_MATERIALS {
            bail!("room GLB: {} materials (at most {MAX_MATERIALS})", model.materials.len());
        }
        for (index, world) in glb.walk() {
            let node = glb.node(index);
            let Some(name) = in_season(node["name"].as_str().unwrap_or(""), season) else { continue };
            if let Some(a) = name.strip_prefix("anchor.") {
                model.anchors.push(RoomAnchor { name: a.to_string(), transform: world, extras: node["extras"].clone() });
            }
            let Some(mesh) = node["mesh"].as_u64() else { continue };
            let normal_matrix = world.inverse().transpose();
            for prim in doc["meshes"][mesh as usize]["primitives"].as_array().into_iter().flatten() {
                if prim["mode"].as_u64().unwrap_or(4) != 4 {
                    continue;
                }
                let attrs = &prim["attributes"];
                let positions = glb.read_vec(attrs["POSITION"].as_u64().ok_or_else(|| anyhow!("room GLB: a primitive without positions"))? as usize, 3)?;
                let count = positions.len() / 3;
                let normals = match attrs["NORMAL"].as_u64() {
                    Some(a) => glb.read_vec(a as usize, 3)?,
                    None => vec![0.0; count * 3],
                };
                let uvs = match attrs["TEXCOORD_0"].as_u64() {
                    Some(a) => glb.read_vec(a as usize, 2)?,
                    None => vec![0.0; count * 2],
                };
                // A baked set as 16-bit channels (unbaked: no light, the occlusion open).
                let baked = |key: &str, default: [f32; 4]| -> Result<Vec<[u16; 4]>> {
                    let unorm = |v: f32| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                    Ok(match attrs[key].as_u64() {
                        Some(a) => {
                            let width = glb.width(a as usize);
                            glb.read_vec(a as usize, width)?.chunks(width).map(|c| std::array::from_fn(|k| unorm(c.get(k).copied().unwrap_or(default[k])))).collect()
                        }
                        None => vec![default.map(unorm); count],
                    })
                };
                let light = baked("COLOR_0", [0.0, 0.0, 0.0, 1.0])?;
                let tube = baked("COLOR_1", [0.0; 4])?;
                let lava = baked("COLOR_2", [0.0, 0.0, 0.0, 1.0])?;
                let material = prim["material"].as_u64().unwrap_or(0) as u32;
                if material as usize >= model.materials.len() {
                    bail!("room GLB: a primitive with no such material");
                }
                let base = model.vertices.len() as u32;
                for k in 0..count {
                    let p = world.transform_point3(Vec3::from_slice(&positions[k * 3..k * 3 + 3]));
                    let n = normal_matrix.transform_vector3(Vec3::from_slice(&normals[k * 3..k * 3 + 3])).normalize_or_zero();
                    model.vertices.push(RoomVertex {
                        pos: p.to_array(),
                        normal: n.to_array(),
                        uv: [uvs[k * 2], uvs[k * 2 + 1]],
                        light: light.get(k).copied().unwrap_or([0, 0, 0, 65535]),
                        tube: tube.get(k).copied().unwrap_or([0; 4]),
                        lava: lava.get(k).copied().unwrap_or([0, 0, 0, 65535]),
                        material,
                    });
                }
                match prim["indices"].as_u64() {
                    Some(a) => model.indices.extend(glb.read_indices(a as usize)?.into_iter().map(|i| base + i)),
                    None => model.indices.extend(base..model.vertices.len() as u32),
                }
            }
        }
        if model.vertices.is_empty() {
            bail!("room GLB: no triangles");
        }
        if model.anchor("screen").is_none() {
            bail!("room GLB: no anchor.screen");
        }
        Ok(model)
    }

    pub fn anchor(&self, name: &str) -> Option<&RoomAnchor> {
        self.anchors.iter().find(|a| a.name == name)
    }

    /// Where an anchor is.
    pub fn point(&self, name: &str) -> Option<Vec3> {
        self.anchor(name).map(|a| a.transform.w_axis.truncate())
    }

    /// The tube's glass (`anchor.screen`: its scale is half the glass's width and height, and
    /// its bulge).
    pub fn screen(&self) -> Option<RoomScreen> {
        let a = self.anchor("screen")?;
        let (scale, rotation, center) = a.transform.to_scale_rotation_translation();
        Some(RoomScreen { center, right: rotation * Vec3::X * scale.x, up: rotation * Vec3::Y * scale.y, bulge: scale.z })
    }

    /// Towards the moon (`anchor.moon`'s place).
    pub fn moon_dir(&self) -> Vec3 {
        self.point("moon").unwrap_or(Vec3::new(-0.8, 0.4, 0.45)).normalize_or(Vec3::Y)
    }

    /// The bounds of a role's vertices.
    fn bounds(&self, role: Role) -> Option<(Vec3, Vec3)> {
        let mut out: Option<(Vec3, Vec3)> = None;
        for v in &self.vertices {
            if self.materials[v.material as usize].role == role {
                let p = Vec3::from(v.pos);
                out = Some(out.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
            }
        }
        out
    }
}

/// A key's legend on `picture.keys`: what it says (a line break puts the shifted sign above;
/// `<up>`, `<down>`, `<left>`, `<right>` and `<back>` are arrows), where the key's top is on the
/// picture (u0, v0, u1, v1, from its top left), and the key's colour (linear).
#[derive(Clone, Debug, PartialEq)]
pub struct KeyLegend {
    pub label: String,
    pub cell: [f32; 4],
    pub colour: [f32; 3],
}

impl RoomModel {
    /// The keyboard's legends (`anchor.keys`'s extras).
    pub fn key_legends(&self) -> Vec<KeyLegend> {
        let Some(text) = self.anchor("keys").and_then(|a| a.extras["keys"].as_str()) else { return Vec::new() };
        let Ok(serde_json::Value::Array(keys)) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
        let num = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0) as f32;
        keys.iter()
            .filter_map(|k| {
                let k = k.as_array().filter(|k| k.len() >= 8)?;
                Some(KeyLegend { label: k[0].as_str()?.to_string(), cell: [num(&k[1]), num(&k[2]), num(&k[3]), num(&k[4])], colour: [num(&k[5]), num(&k[6]), num(&k[7])] })
            })
            .collect()
    }
}

/// Everything the room is made of: its model, and the pictures the game painted for it (one
/// layer each, [`LAYER_SIZE`]² RGBA, sRGB, in the order of [`RoomModel::pictures`]).
pub struct RoomScene {
    pub model: RoomModel,
    pub layers: Vec<Vec<u8>>,
}

/// The room as this frame shows it.
#[derive(Clone, Copy, Debug)]
pub struct RoomFrame {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov_y: f32,
    /// Seconds since the room came up.
    pub time: f32,
    /// The tube: 0 off, 1 on (in between: warming up or going off).
    pub power: f32,
    /// A picture is on the tube (between movies it is dark).
    pub picture: bool,
    /// The picture's 3×3 regional colours (linear), row by row from the top left.
    pub glow: [[f32; 3]; 9],
    /// How bright each glowing part is (0..1; in the order of [`GLOW_NAMES`]).
    pub glows: [f32; GLOWS],
    /// The moonlight: 0 behind a cloud .. 1 clear.
    pub moon: f32,
    /// The lava lamp's light, 0..1 (its wax's warmth flickers it).
    pub lava: f32,
    /// The tube's degaussing as it comes on (the picture's colours swirl), 1 .. 0.
    pub degauss: f32,
    /// The lens's depth of field, focused on the tube: its largest blur's radius as a fraction
    /// of the picture's height (0: all sharp).
    pub dof: f32,
    /// The season's three lights, at the places of the model's `anchor.light0..2`: the tree's
    /// (or the party's) and two strings of lights.
    pub lights: [FestiveLight; 3],
    /// Fireworks over the street: along the sky (z) and up it (y) in metres, the burst's age
    /// (0 .. 1; below 0: none), its hue (0 .. 1).
    pub bursts: [[f32; 4]; 4],
    /// The fireworks' light through the window (linear rgb).
    pub flash: [f32; 3],
}

/// One of the season's lights: its colour (linear) and how bright it is (0 .. 1).
#[derive(Clone, Copy, Debug, Default)]
pub struct FestiveLight {
    pub color: [f32; 3],
    pub strength: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RoomUniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    moon_view_proj: [[f32; 4]; 4],
    tube_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    screen_center: [f32; 4],
    screen_right: [f32; 4],
    screen_up: [f32; 4],
    screen_normal: [f32; 4],
    glow: [[f32; 4]; 9],
    quarters: [[f32; 4]; 4],
    screen_light: [f32; 4],
    look: [f32; 4],
    seat: [f32; 4],
    moon_dir: [f32; 4],
    moon_color: [f32; 4],
    moon_map: [f32; 4],
    tube_map: [f32; 4],
    lava: [f32; 4],
    lava_color: [f32; 4],
    window: [f32; 4],
    window_half: [f32; 4],
    scales: [f32; 4],
    glows: [[f32; 4]; 6],
    steam: [f32; 4],
    pixel: [f32; 4],
    season: [f32; 4],
    festive: [[f32; 4]; 3],
    festive_color: [[f32; 4]; 3],
    bursts: [[f32; 4]; 4],
    flash: [f32; 4],
}

/// The uniforms' sizes, as the shader must lay them out.
#[cfg(test)]
pub(crate) const UNIFORM_SIZE: usize = std::mem::size_of::<RoomUniform>();
#[cfg(test)]
pub(crate) const MATERIAL_SIZE: usize = std::mem::size_of::<GpuMaterial>();

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMaterial {
    base: [f32; 4],
    /// Roughness, metallic, role, index.
    params: [f32; 4],
    emissive: [f32; 4],
}

/// What the room renderer keeps per size of the picture.
struct Sized {
    width: u32,
    height: u32,
    /// The scene's depth the groups read (made again when it is).
    depth: wgpu::TextureView,
    /// The air's light at half resolution (rgb) and its depth (a).
    fog: Target,
    fog_group: wgpu::BindGroup,
    composite_group: wgpu::BindGroup,
}

pub(crate) struct RoomRenderer {
    surface: wgpu::RenderPipeline,
    lava: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
    screen_pipeline: wgpu::RenderPipeline,
    motes: wgpu::RenderPipeline,
    fog: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    fog_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    materials: wgpu::Buffer,
    vertices: wgpu::Buffer,
    /// Triangles by how they are drawn: opaque, the lava lamp's glass, the window's pane.
    opaque: (wgpu::Buffer, u32),
    lava_glass: (wgpu::Buffer, u32),
    pane: (wgpu::Buffer, u32),
    grid: wgpu::Buffer,
    grid_indices: wgpu::Buffer,
    grid_count: u32,
    layers: wgpu::TextureView,
    layer_sampler: wgpu::Sampler,
    moon_map: Target,
    tube_map: Target,
    shadow_sampler: wgpu::Sampler,
    depth_sampler: wgpu::Sampler,
    picture: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    blank: wgpu::TextureView,
    picture_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    sized: Option<Sized>,
    /// The light in the air is drawn (`TA_ROOM_FOG=0` leaves it out, to compare).
    fog_on: bool,
    screen: RoomScreen,
    moon_dir: Vec3,
    moon_view_proj: Mat4,
    tube_view_proj: Mat4,
    moon_texel: f32,
    lava_at: Vec3,
    steam_at: Vec3,
    /// The season, and where its lights are (none where the model has no such anchor).
    season: Season,
    festive_at: [Option<Vec3>; 3],
    /// Where the person at the desk sits (their chest, under the eye).
    seat: Vec3,
    window: (Vec3, Vec3),
}

impl RoomRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene: &RoomScene, samples: u32) -> RoomRenderer {
        let model = &scene.model;
        let shader = crate::util::shader_standalone(device, "room", &shader_source(samples));
        let tex = |binding: u32, dimension: wgpu::TextureViewDimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: dimension, multisampled: false },
            count: None,
        };
        let depth_tex = |binding: u32, multisampled: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled },
            count: None,
        };
        let sampler = |binding: u32, kind: wgpu::SamplerBindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Sampler(kind), count: None };
        let buffer = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("room"),
            entries: &[
                buffer(0),
                buffer(1),
                tex(2, wgpu::TextureViewDimension::D2Array),
                sampler(3, wgpu::SamplerBindingType::Filtering),
                tex(4, wgpu::TextureViewDimension::D2),
                sampler(5, wgpu::SamplerBindingType::Filtering),
                depth_tex(6, false),
                depth_tex(7, false),
                sampler(8, wgpu::SamplerBindingType::Comparison),
                sampler(9, wgpu::SamplerBindingType::NonFiltering),
            ],
        });
        let fog_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("room fog"), entries: &[depth_tex(0, samples > 1)] });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("room fog composite"),
            entries: &[depth_tex(0, samples > 1), tex(1, wgpu::TextureViewDimension::D2), sampler(2, wgpu::SamplerBindingType::Filtering)],
        });
        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("room"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let fog_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("room fog"), bind_group_layouts: &[Some(&layout), Some(&fog_layout)], immediate_size: 0 });
        let composite_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("room fog composite"), bind_group_layouts: &[Some(&layout), Some(&composite_layout)], immediate_size: 0 });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<RoomVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Unorm16x4, 4 => Unorm16x4, 5 => Unorm16x4, 6 => Uint32],
        };
        let opaque_target = [Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let blended_target = [Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::COLOR })];
        let additive = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let additive_target = [Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: Some(wgpu::BlendState { color: additive, alpha: additive }), write_mask: wgpu::ColorWrites::COLOR })];
        let multisample = wgpu::MultisampleState { count: samples, ..Default::default() };
        let pipeline = |label: &str, vs: &str, fs: &str, buffers: &[Option<wgpu::VertexBufferLayout>], targets: &[Option<wgpu::ColorTargetState>], depth_write: bool, layout: &wgpu::PipelineLayout, depth: bool, multisample: wgpu::MultisampleState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), buffers, compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: depth.then(|| crate::util::depth_state(depth_write)),
                multisample,
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), targets, compilation_options: Default::default() }),
                multiview_mask: None,
                cache: None,
            })
        };
        let surface = pipeline("room surfaces", "vs_surface", "fs_surface", &[Some(vertex_layout.clone())], &opaque_target, true, &main_layout, true, multisample);
        let lava = pipeline("room lava lamp", "vs_surface", "fs_lava", &[Some(vertex_layout.clone())], &opaque_target, true, &main_layout, true, multisample);
        let glass = pipeline("room window glass", "vs_surface", "fs_glass", &[Some(vertex_layout.clone())], &blended_target, false, &main_layout, true, multisample);
        let grid_layout = wgpu::VertexBufferLayout { array_stride: 8, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x2] };
        let screen_pipeline = pipeline("room tube", "vs_screen", "fs_screen", &[Some(grid_layout)], &opaque_target, true, &main_layout, true, multisample);
        let motes = pipeline("room motes", "vs_mote", "fs_mote", &[], &additive_target, false, &main_layout, true, multisample);
        let single = wgpu::MultisampleState::default();
        let fog_target = [Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let fog = pipeline("room fog", "vs_fullscreen", "fs_fog", &[], &fog_target, false, &fog_pipeline_layout, false, single);
        let composite = pipeline("room fog composite", "vs_fullscreen", "fs_fog_composite", &[], &additive_target, false, &composite_pipeline_layout, false, single);

        // The surfaces, sorted by how they are drawn.
        let vertices = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("room vertices"), contents: bytemuck::cast_slice(&model.vertices), usage: wgpu::BufferUsages::VERTEX });
        let role_of = |t: &[u32]| model.materials[model.vertices[t[0] as usize].material as usize].role;
        let (mut opaque, mut lava_glass, mut pane, mut casters) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for t in model.indices.as_chunks::<3>().0 {
            let role = role_of(t);
            match role {
                Role::Lava => lava_glass.extend_from_slice(t),
                Role::Glass => pane.extend_from_slice(t),
                _ => opaque.extend_from_slice(t),
            }
            let m = &model.materials[model.vertices[t[0] as usize].material as usize];
            if role.casts_shadow() && m.name != "emit.window_lit" {
                casters.extend_from_slice(t);
            }
        }
        let index_buffer = |label: &str, indices: &[u32]| -> (wgpu::Buffer, u32) {
            // (An empty buffer cannot be made: one degenerate triangle stands in, drawn as none.)
            let contents: &[u32] = if indices.is_empty() { &[0, 0, 0] } else { indices };
            (device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytemuck::cast_slice(contents), usage: wgpu::BufferUsages::INDEX }), indices.len() as u32)
        };
        let opaque = index_buffer("room opaque", &opaque);
        let lava_glass = index_buffer("room lava glass", &lava_glass);
        let pane = index_buffer("room pane", &pane);
        let casters = index_buffer("room casters", &casters);

        // The glass's grid.
        let (gw, gh) = GRID;
        let grid_points: Vec<[f32; 2]> = (0..=gh).flat_map(|y| (0..=gw).map(move |x| [x as f32 / gw as f32, y as f32 / gh as f32])).collect();
        let mut grid_index: Vec<u32> = Vec::with_capacity((gw * gh * 6) as usize);
        for y in 0..gh {
            for x in 0..gw {
                let i = y * (gw + 1) + x;
                grid_index.extend_from_slice(&[i, i + 1, i + gw + 1, i + 1, i + gw + 2, i + gw + 1]);
            }
        }
        let grid = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("room glass"), contents: bytemuck::cast_slice(&grid_points), usage: wgpu::BufferUsages::VERTEX });
        let grid_indices = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("room glass indices"), contents: bytemuck::cast_slice(&grid_index), usage: wgpu::BufferUsages::INDEX });

        // The materials.
        let mut gpu_materials = [GpuMaterial::zeroed(); MAX_MATERIALS];
        for (g, m) in gpu_materials.iter_mut().zip(&model.materials) {
            // (A display's picture glows as its glow does: what it shows of the room's light,
            // how bright it glows, and the glow's slot plus one.)
            let mut emissive = [m.emissive.x, m.emissive.y, m.emissive.z, 0.0];
            if let Some((_, glow, shown, bright)) = GLOWING_PICTURES.iter().find(|(picture, ..)| m.name.strip_prefix("picture.") == Some(*picture))
                && let Some(slot) = GLOW_NAMES.iter().position(|g| g == glow)
            {
                emissive = [*shown, *bright, 0.0, slot as f32 + 1.0];
            }
            *g = GpuMaterial { base: m.base, params: [m.roughness, m.metallic, m.role as u32 as f32, m.index as f32], emissive };
        }
        let materials = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("room materials"), contents: bytemuck::cast_slice(&gpu_materials), usage: wgpu::BufferUsages::UNIFORM });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("room"), size: std::mem::size_of::<RoomUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        // The pictures, with their mipmaps.
        let count = scene.layers.len().max(1) as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("room pictures"),
            size: wgpu::Extent3d { width: LAYER_SIZE, height: LAYER_SIZE, depth_or_array_layers: count },
            mip_level_count: LAYER_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, image) in scene.layers.iter().enumerate() {
            if image.len() != (LAYER_SIZE * LAYER_SIZE * 4) as usize {
                continue;
            }
            let mut level = image.clone();
            let mut size = LAYER_SIZE;
            for mip in 0..LAYER_MIPS {
                crate::util::write_texture_layer(queue, &texture, mip, layer as u32, size, size, &level, 4);
                if size == 1 {
                    break;
                }
                level = half_size(&level, size, size, true);
                size /= 2;
            }
        }
        let layers = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let layer_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("room pictures"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let picture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("room picture"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("room linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("room shadows"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("room depth"), ..Default::default() });
        let blank = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("room no picture"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[0, 0, 0, 255],
            )
            .create_view(&Default::default());

        // The lights' places: the tube, the moon through the window, the lava lamp.
        let screen = model.screen().unwrap_or(RoomScreen { center: Vec3::new(0.0, 1.0, -0.2), right: Vec3::X * 0.16, up: Vec3::Y * 0.12, bulge: 0.012 });
        let moon_dir = model.moon_dir();
        let window = model.bounds(Role::Glass).unwrap_or((Vec3::new(-1.65, 0.98, -0.52), Vec3::new(-1.65, 2.06, 0.42)));
        let (moon_view_proj, moon_texel) = moon_projection(moon_dir, window);
        let tube_eye = screen.center + screen.normal() * screen.bulge;
        let tube_view = glam::camera::rh::view::look_to_mat4(tube_eye, screen.normal(), screen.up.normalize_or(Vec3::Y));
        let tube_view_proj = glam::camera::rh::proj::directx::perspective(TUBE_FOV.to_radians(), 1.0, TUBE_NEAR, TUBE_FAR) * tube_view;
        let lava_at = model.point("lava").unwrap_or(Vec3::new(0.66, 0.98, -0.46));
        let steam_at = model.point("steam").unwrap_or(Vec3::new(-0.42, 0.82, -0.02));
        let seat = model.point("eye").unwrap_or(Vec3::new(0.0, 1.2, 0.86)) - Vec3::Y * 0.25;
        let festive_at = [0, 1, 2].map(|i| model.point(&format!("light{i}")));

        // The shadow maps, once: nothing in the room moves.
        let map = |label: &str, size: u32| Target::new(device, label, size, size, DEPTH_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let moon_map = map("room moon shadows", MOON_MAP);
        let tube_map = map("room tube shadows", TUBE_MAP);
        render_shadows(device, queue, &vertices, &casters, &[(&moon_map, moon_view_proj), (&tube_map, tube_view_proj)]);

        let bind_group = Self::bind(device, &layout, &uniform, &materials, &layers, &layer_sampler, &blank, &picture_sampler, &moon_map, &tube_map, &shadow_sampler, &depth_sampler);
        RoomRenderer {
            surface,
            lava,
            glass,
            screen_pipeline,
            motes,
            fog,
            composite,
            layout,
            fog_layout,
            composite_layout,
            bind_group,
            uniform,
            materials,
            vertices,
            opaque,
            lava_glass,
            pane,
            grid,
            grid_indices,
            grid_count: grid_index.len() as u32,
            layers,
            layer_sampler,
            moon_map,
            tube_map,
            shadow_sampler,
            depth_sampler,
            picture: None,
            blank,
            picture_sampler,
            linear_sampler,
            sized: None,
            fog_on: std::env::var("TA_ROOM_FOG").map_or(true, |v| v != "0"),
            screen,
            moon_dir,
            moon_view_proj,
            tube_view_proj,
            moon_texel,
            lava_at,
            steam_at,
            season: model.season,
            festive_at,
            seat,
            window,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
        materials: &wgpu::Buffer,
        layers: &wgpu::TextureView,
        layer_sampler: &wgpu::Sampler,
        picture: &wgpu::TextureView,
        picture_sampler: &wgpu::Sampler,
        moon_map: &Target,
        tube_map: &Target,
        shadow_sampler: &wgpu::Sampler,
        depth_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("room"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: materials.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(layers) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(layer_sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(picture) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(picture_sampler) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&moon_map.view) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&tube_map.view) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(shadow_sampler) },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(depth_sampler) },
            ],
        })
    }

    fn rebind(&mut self, device: &wgpu::Device) {
        let picture = self.picture.as_ref().map_or(&self.blank, |p| &p.1);
        self.bind_group = Self::bind(device, &self.layout, &self.uniform, &self.materials, &self.layers, &self.layer_sampler, picture, &self.picture_sampler, &self.moon_map, &self.tube_map, &self.shadow_sampler, &self.depth_sampler);
    }

    /// The movie's newest frame, RGBA (sRGB), with its mipmaps (its reflections blur by them).
    pub fn set_picture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, rgba: &[u8]) {
        if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
            return;
        }
        let mips = 32 - width.max(height).leading_zeros();
        if self.picture.as_ref().is_none_or(|p| p.2 != width || p.3 != height) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("room picture"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.picture = Some((texture, view, width, height));
            self.rebind(device);
        }
        if let Some((texture, ..)) = &self.picture {
            let mut level = rgba[..(width * height * 4) as usize].to_vec();
            let (mut w, mut h) = (width, height);
            for mip in 0..mips {
                crate::util::write_texture_level(queue, texture, mip, w, h, &level, 4);
                if w == 1 && h == 1 {
                    break;
                }
                level = half_size(&level, w, h, false);
                w = (w / 2).max(1);
                h = (h / 2).max(1);
            }
        }
    }

    /// The targets for a picture `width` × `height` and the scene's depth (`depth`).
    fn size(&mut self, device: &wgpu::Device, width: u32, height: u32, depth: &wgpu::TextureView) {
        if self.sized.as_ref().is_some_and(|s| s.width == width && s.height == height && s.depth == *depth) {
            return;
        }
        let fog = Target::new(device, "room fog", width.div_ceil(2), height.div_ceil(2), HDR_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let fog_group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("room fog"), layout: &self.fog_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) }] });
        let composite_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("room fog composite"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&fog.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.linear_sampler) },
            ],
        });
        self.sized = Some(Sized { width, height, depth: depth.clone(), fog, fog_group, composite_group });
    }

    /// Writes this frame's camera and lights; `size` = the picture's size in pixels, `exposure`
    /// the room look's. Returns the camera's near and far planes.
    pub fn prepare(&self, queue: &wgpu::Queue, frame: &RoomFrame, size: (u32, u32), exposure: f32) -> (f32, f32) {
        let (near, far) = (0.02, 20.0);
        let aspect = size.0 as f32 / size.1.max(1) as f32;
        let view = glam::camera::rh::view::look_at_mat4(frame.eye, frame.target, Vec3::Y);
        let proj = glam::camera::rh::proj::directx::perspective(frame.fov_y, aspect.max(0.1), near, far);
        let view_proj = proj * view;
        let s = &self.screen;
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let power = frame.power.clamp(0.0, 1.0);
        let on = if frame.picture { 1.0 } else { 0.0 };
        let mut glow = [[0.0f32; 4]; 9];
        for (g, c) in glow.iter_mut().zip(frame.glow) {
            *g = [c[0], c[1], c[2], 0.0];
        }
        // Each quarter's colour: the mean of the patches it overlaps (a patch on its edge counts
        // half, the middle one a quarter), as bright as the tube is.
        let mut quarters = [[0.0f32; 4]; 4];
        for (q, out) in quarters.iter_mut().enumerate() {
            let (qx, qy) = (q % 2, q / 2);
            let mut sum = [0.0f32; 3];
            let mut weight = 0.0;
            for (i, c) in frame.glow.iter().enumerate() {
                let (px, py) = (i % 3, i / 3);
                let wx = match (qx, px) {
                    (0, 0) | (1, 2) => 1.0,
                    (_, 1) => 0.5,
                    _ => 0.0,
                };
                let wy = match (qy, py) {
                    (0, 0) | (1, 2) => 1.0,
                    (_, 1) => 0.5,
                    _ => 0.0,
                };
                let w = wx * wy;
                for k in 0..3 {
                    sum[k] += c[k] * w;
                }
                weight += w;
            }
            let k = SCREEN_LIGHT * power * on / weight;
            *out = [sum[0] * k, sum[1] * k, sum[2] * k, 0.0];
        }
        let mut glows = [[0.0f32; 4]; 6];
        for (i, g) in frame.glows.iter().enumerate().take(GLOWS - 1) {
            glows[i / 4][i % 4] = *g;
        }
        glows[(GLOWS - 1) / 4][(GLOWS - 1) % 4] = 1.0;
        let mut festive = [[0.0f32; 4]; 3];
        let mut festive_color = [[0.0f32; 4]; 3];
        for (i, light) in frame.lights.iter().enumerate() {
            if let Some(at) = self.festive_at[i] {
                festive[i] = [at.x, at.y, at.z, FESTIVE_REACH[i]];
                festive_color[i] = [light.color[0] * light.strength, light.color[1] * light.strength, light.color[2] * light.strength, 0.0];
            }
        }
        let (lo, hi) = self.window;
        let moon = MOON_IRRADIANCE * frame.moon.clamp(0.0, 1.0);
        let u = RoomUniform {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            moon_view_proj: self.moon_view_proj.to_cols_array_2d(),
            tube_view_proj: self.tube_view_proj.to_cols_array_2d(),
            eye: v4(frame.eye, frame.time),
            screen_center: v4(s.center, power),
            screen_right: v4(s.right, s.bulge),
            screen_up: v4(s.up, on),
            screen_normal: v4(s.normal(), SCREEN_WHITE),
            glow,
            quarters,
            screen_light: [SCREEN_LIGHT, SEAT_FILL, 0.0, 0.0],
            look: [exposure.max(1e-3), crate::post::ROOM_TOE, crate::post::ROOM_SHOULDER, 0.0],
            seat: v4(self.seat, 0.0),
            moon_dir: v4(self.moon_dir, moon),
            moon_color: [MOON_COLOR[0], MOON_COLOR[1], MOON_COLOR[2], frame.degauss],
            moon_map: [MOON_DEPTH, MOON_RADIUS.tan(), self.moon_texel, MOON_MAP as f32],
            tube_map: [TUBE_NEAR, TUBE_FAR, (TUBE_FOV.to_radians() * 0.5).tan(), TUBE_MAP as f32],
            lava: v4(self.lava_at, frame.lava),
            lava_color: [LAVA_COLOR[0], LAVA_COLOR[1], LAVA_COLOR[2], 0.0],
            window: v4((lo + hi) * 0.5, 0.0),
            window_half: v4((hi - lo) * 0.5, 0.0),
            scales: [BOUNCE_SCALE, TUBE_SCALE, LAVA_SCALE, 0.0012],
            glows,
            steam: v4(self.steam_at, MOTES as f32),
            pixel: [1.0 / size.0.max(1) as f32, 1.0 / size.1.max(1) as f32, near, far],
            season: [self.season as u32 as f32, 0.0, 0.0, 0.0],
            festive,
            festive_color,
            bursts: frame.bursts,
            flash: [frame.flash[0], frame.flash[1], frame.flash[2], 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        (near, far)
    }

    /// How far the tube is from the eye along the view (the lens's focus).
    pub fn focus(&self, frame: &RoomFrame) -> f32 {
        (self.screen.center - frame.eye).dot((frame.target - frame.eye).normalize_or(Vec3::NEG_Z)).max(0.05)
    }

    /// Draws the room into the scene's picture (`color`, multisampled and resolved into
    /// `resolve` when the picture is), with `depth` (the scene's), then the light in the air
    /// over it (into `resolved`, the picture the post-processing reads).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        (color, resolve): (&wgpu::TextureView, Option<&wgpu::TextureView>),
        resolved: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        self.size(device, size.0, size.1, depth);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("room"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color, resolve_target: resolve, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store }, depth_slice: None })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_pipeline(&self.surface);
            pass.set_index_buffer(self.opaque.0.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.opaque.1, 0, 0..1);
            if self.lava_glass.1 > 0 {
                pass.set_pipeline(&self.lava);
                pass.set_index_buffer(self.lava_glass.0.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.lava_glass.1, 0, 0..1);
            }
            pass.set_pipeline(&self.screen_pipeline);
            pass.set_vertex_buffer(0, self.grid.slice(..));
            pass.set_index_buffer(self.grid_indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.grid_count, 0, 0..1);
            if self.pane.1 > 0 {
                pass.set_pipeline(&self.glass);
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.set_index_buffer(self.pane.0.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.pane.1, 0, 0..1);
            }
            pass.set_pipeline(&self.motes);
            pass.draw(0..6, 0..MOTES + STEAM);
        }
        let Some(sized) = self.sized.as_ref().filter(|_| self.fog_on) else { return };
        // The light in the air: marched at half resolution, then laid over the picture.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("room fog"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &sized.fog.view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store }, depth_slice: None })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.fog);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &sized.fog_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("room fog composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: resolved, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }, depth_slice: None })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &sized.composite_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The moon's shadow map: an orthographic view along its light, just wide enough for the light
/// that comes in by the window (`window`, the pane's bounds, widened for its frame); and the
/// size of a texel (metres).
fn moon_projection(moon_dir: Vec3, window: (Vec3, Vec3)) -> (Mat4, f32) {
    let (lo, hi) = (window.0 - Vec3::splat(0.12), window.1 + Vec3::splat(0.12));
    let centre = (lo + hi) * 0.5;
    let up = if moon_dir.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
    let view = glam::camera::rh::view::look_to_mat4(centre + moon_dir * MOON_BACK, -moon_dir, up);
    let (mut min, mut max) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    for i in 0..8 {
        let corner = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
        let p = view.transform_point3(corner);
        min = min.min(p.truncate());
        max = max.max(p.truncate());
    }
    let proj = glam::camera::rh::proj::directx::orthographic(min.x, max.x, min.y, max.y, 0.0, MOON_DEPTH);
    let texel = (max - min).max_element() / MOON_MAP as f32;
    (proj * view, texel)
}

/// Draws the shadow casters' depth into each map from its light.
fn render_shadows(device: &wgpu::Device, queue: &wgpu::Queue, vertices: &wgpu::Buffer, casters: &(wgpu::Buffer, u32), maps: &[(&Target, Mat4)]) {
    let shader = crate::util::shader_standalone(device, "room shadows", SHADOW_SHADER);
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("room shadows"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("room shadows"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("room shadows"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_shadow"),
            buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<RoomVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3] })],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState { constant: 2, slope_scale: 1.5, clamp: 0.0 },
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: None,
        multiview_mask: None,
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("room shadows") });
    for (target, view_proj) in maps {
        let uniform = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("room shadow"), contents: bytemuck::cast_slice(&view_proj.to_cols_array()), usage: wgpu::BufferUsages::UNIFORM });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("room shadow"), layout: &layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }] });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("room shadow map"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &target.view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(casters.0.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..casters.1, 0, 0..1);
    }
    queue.submit(Some(encoder.finish()));
}

const SHADOW_SHADER: &str = "
@group(0) @binding(0) var<uniform> light_view_proj: mat4x4<f32>;

@vertex
fn vs_shadow(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return light_view_proj * vec4<f32>(pos, 1.0);
}
";

/// The room's shader (the full-screen triangle and the room), reading a depth buffer of
/// `samples` samples.
pub(crate) fn shader_source(samples: u32) -> String {
    let source = format!("{}\n{}", include_str!("shaders/fullscreen.wgsl"), include_str!("shaders/room.wgsl"));
    if samples > 1 { source } else { source.replace("texture_depth_multisampled_2d", "texture_depth_2d") }
}

/// An RGBA image `w` × `h` at half the size (each side at least 1). `linear`: averaged in linear
/// light, as an sRGB picture should be (the movie's frames, mipmapped every frame, are averaged
/// as they are, which is quicker).
fn half_size(image: &[u8], w: u32, h: u32, linear: bool) -> Vec<u8> {
    let (hw, hh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; (hw * hh * 4) as usize];
    let lut = linear.then(crate::util::srgb_to_linear_lut);
    for y in 0..hh {
        for x in 0..hw {
            let mut acc = [0f32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let i = (((y * 2 + dy).min(h - 1) * w + (x * 2 + dx).min(w - 1)) * 4) as usize;
                for k in 0..4 {
                    acc[k] += match (&lut, k) {
                        (Some(l), 0..=2) => l[image[i + k] as usize],
                        _ => image[i + k] as f32,
                    } * 0.25;
                }
            }
            let o = ((y * hw + x) * 4) as usize;
            for k in 0..4 {
                out[o + k] = match k {
                    0..=2 if lut.is_some() => crate::util::linear_to_srgb(acc[k]),
                    _ => (acc[k] + 0.5) as u8,
                };
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_room_shipped_loads() {
        let model = RoomModel::from_glb(room_model(), Season::Normal).unwrap();
        assert!(model.vertices.len() > 10_000, "{} vertices", model.vertices.len());
        let screen = model.screen().unwrap();
        // The tube faces the room (towards the viewer, +z), about 32 by 24 cm.
        assert!(screen.normal().z > 0.9, "{:?}", screen.normal());
        assert!((0.12..0.2).contains(&screen.right.length()) && (0.09..0.15).contains(&screen.up.length()), "{screen:?}");
        for name in ["eye", "target", "moon", "lava", "steam", "keys"] {
            assert!(model.anchor(name).is_some(), "anchor.{name}");
        }
        for name in ["poster_main", "poster_space", "poster_outrun", "poster_band", "box0", "keys", "turbo"] {
            assert!(model.pictures.iter().any(|p| p.name == name), "picture.{name}");
        }
        // The can of cola and the mobile phone (it passed for a remote control) are gone, the
        // Cavedog stone's poster too.
        for gone in ["phone", "poster_cavedog"] {
            assert!(!model.pictures.iter().any(|p| p.name == gone), "picture.{gone} is back");
        }
        assert!(!model.materials.iter().any(|m| m.name.starts_with("can_") || m.name.starts_with("phone_")), "the can or the phone is back");
        // So are the headphones, whose cushions and shells were the only things in the black
        // plastic and the leatherette (in any season's room).
        for season in [Season::Normal, Season::Christmas, Season::NewYear] {
            let room = RoomModel::from_glb(room_model(), season).unwrap();
            let used: std::collections::HashSet<&str> = room.vertices.iter().map(|v| room.materials[v.material as usize].name.as_str()).collect();
            assert!(!used.contains("pleather") && !used.contains("plastic_black"), "the headphones are back in the {} room", season.name());
        }
        // The baked light is in: the walls see the moon's bounce somewhere, and are open.
        let paint = model.materials.iter().position(|m| m.name == "paint").unwrap() as u32;
        let walls: Vec<&RoomVertex> = model.vertices.iter().filter(|v| v.material == paint).collect();
        assert!(walls.iter().any(|v| v.light[0] > 1000), "no bounce on the walls");
        assert!(walls.iter().map(|v| v.light[3] as f32 / 65535.0).sum::<f32>() / walls.len() as f32 > 0.5);
    }

    #[test]
    fn the_seasons_put_up_their_own_things() {
        let normal = RoomModel::from_glb(room_model(), Season::Normal).unwrap();
        let christmas = RoomModel::from_glb(room_model(), Season::Christmas).unwrap();
        let new_year = RoomModel::from_glb(room_model(), Season::NewYear).unwrap();
        // Each season is a room of its own (it puts things up), with its lights; the ordinary
        // room has none.
        assert!(christmas.vertices.len() > normal.vertices.len() + 2000, "{} against {}", christmas.vertices.len(), normal.vertices.len());
        assert_ne!(new_year.vertices.len(), normal.vertices.len());
        for (season, model) in [("christmas", &christmas), ("new year", &new_year)] {
            for i in 0..2 {
                assert!(model.anchor(&format!("light{i}")).is_some(), "{season}: anchor.light{i}");
            }
        }
        assert!(normal.anchor("light0").is_none(), "the ordinary room has no festive light");
        assert!(christmas.anchor("light0").unwrap().transform != new_year.anchor("light0").unwrap().transform);
        let named = |model: &RoomModel, material: &str| model.vertices.iter().any(|v| model.materials[v.material as usize].name == material);
        assert!(named(&christmas, "emit.xmas_red") && !named(&new_year, "emit.xmas_red") && !named(&normal, "emit.xmas_red"));
        assert!(named(&new_year, "balloon_gold") && !named(&christmas, "balloon_gold") && !named(&normal, "balloon_gold"));
    }

    #[test]
    fn seasons_are_named_as_the_model_tags_them() {
        assert_eq!(Season::parse("Christmas"), Some(Season::Christmas));
        assert_eq!(Season::parse("new-year"), Some(Season::NewYear));
        assert_eq!(Season::parse(" NEW_YEAR "), Some(Season::NewYear));
        assert_eq!(Season::parse("normal"), Some(Season::Normal));
        assert_eq!(Season::parse("halloween"), None);
        assert_eq!(in_season("static.plastic", Season::Normal), Some("static.plastic"));
        assert_eq!(in_season("season.christmas.fabric_fir", Season::Christmas), Some("fabric_fir"));
        assert_eq!(in_season("season.christmas.fabric_fir", Season::NewYear), None);
        assert_eq!(in_season("season.christmas.fabric_fir", Season::Normal), None);
        assert_eq!(in_season("season.newyear.balloon_gold", Season::NewYear), Some("balloon_gold"));
    }

    #[test]
    fn the_keys_legends_are_in_the_model() {
        let model = RoomModel::from_glb(room_model(), Season::Normal).unwrap();
        let keys = model.key_legends();
        assert_eq!(keys.len(), 101);
        assert_eq!(keys[0].label, "Esc");
        // Each on its own part of the picture.
        for (i, a) in keys.iter().enumerate() {
            assert!(a.cell[0] < a.cell[2] && a.cell[1] < a.cell[3] && a.cell[2] <= 1.0 && a.cell[3] <= 1.0, "{a:?}");
            for b in &keys[i + 1..] {
                assert!(a.cell[2] <= b.cell[0] + 1e-4 || b.cell[2] <= a.cell[0] + 1e-4 || a.cell[3] <= b.cell[1] + 1e-4 || b.cell[3] <= a.cell[1] + 1e-4, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn the_moon_map_covers_the_window() {
        let window = (Vec3::new(-1.65, 0.98, -0.52), Vec3::new(-1.65, 2.06, 0.42));
        let dir = Vec3::new(-0.8, 0.4, 0.45).normalize();
        let (m, texel) = moon_projection(dir, window);
        // The window's middle is in view, a little inside the depth range.
        let c = m.project_point3((window.0 + window.1) * 0.5);
        assert!(c.x.abs() < 0.2 && c.y.abs() < 0.2 && (0.0..1.0).contains(&c.z), "{c:?}");
        // A point of the room the moon shines on through the window is in view, deeper.
        let lit = (window.0 + window.1) * 0.5 - dir * 1.5;
        let p = m.project_point3(lit);
        assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0 && p.z > c.z, "{p:?}");
        assert!(texel < 0.001, "{texel}");
    }
}
