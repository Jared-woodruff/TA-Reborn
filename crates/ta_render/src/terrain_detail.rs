//! The ground's detail materials (TA Reborn's own art, drawn by `tools/terrain/detail.py`): sand,
//! soil, grass, rock, gravel, snow, ice, volcanic rock, worn metal and lunar dust, each a tileable
//! map of height and brightness. Up close the terrain shader lays them over TA's painted ground:
//! at each place the two materials the painted colour, its texture and the slope look most like,
//! weighed by what the map's world has (`set_world`), their grain lightening and darkening the
//! painted colour and their relief catching the sun. The texture array holds per texel the
//! brightness (R, 0.5 none), the height's slope across and down (G and B, 0.5 level) and the
//! height (A), with mips that fade to level and plain.

use crate::terrain::byte_mips;

/// The materials, in the order of the texture array's layers (and of `tools/terrain/detail.py`).
pub const MATERIALS: [&str; 10] = ["sand", "soil", "grass", "rock", "gravel", "snow", "ice", "volcanic", "metal", "regolith"];
const SIZE: u32 = 512;
/// The height's slope per texel is stored times this (it is small; the shader scales it back
/// with each material's own relief).
const SLOPE_GAIN: f32 = 6.0;

fn pngs() -> [&'static [u8]; 10] {
    [
        include_bytes!("../terrain_detail/sand.png"),
        include_bytes!("../terrain_detail/soil.png"),
        include_bytes!("../terrain_detail/grass.png"),
        include_bytes!("../terrain_detail/rock.png"),
        include_bytes!("../terrain_detail/gravel.png"),
        include_bytes!("../terrain_detail/snow.png"),
        include_bytes!("../terrain_detail/ice.png"),
        include_bytes!("../terrain_detail/volcanic.png"),
        include_bytes!("../terrain_detail/metal.png"),
        include_bytes!("../terrain_detail/regolith.png"),
    ]
}

/// What a world's ground is made of: a weight per material (`MATERIALS`' order), and whether
/// its walls are metal rather than rock.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct WorldUniform {
    weights: [[f32; 4]; 3],
    /// x: walls are metal (1) or rock (0); y: show the materials (a check); z: how much of the
    /// ground's colour the walls keep ([`world_wall_colour`]); w: how light they are kept at the
    /// least ([`world_wall_lift`]).
    flags: [f32; 4],
    /// The world's broad look ([`WorldLook`]): dry, dark, shore, strength, grade.
    look: [[f32; 4]; 5],
}

/// A world's broad look (Reborn): over hundreds of units its open ground takes on patches of
/// drier ground and of darker ground (in hollows more), and a band by the water or the melt
/// (`dry`, `dark`, `shore`: rgb a tint at luminance 1, a its value, both multiplying the
/// ground's own colour, so sand stays sand and grass grass). Painted maps are mosaics of a few
/// plain tiles, flat at that scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldLook {
    pub dry: [f32; 4],
    pub dark: [f32; 4],
    pub shore: [f32; 4],
    /// How strongly each of `dry`, `dark` and `shore` shows (0..1); w = 1 for dark ground in
    /// winding flows (a lava world's cooled basalt) rather than in patches.
    pub strength: [f32; 4],
    /// x: how much the ground's colours are desaturated, y: how much its value swings over the
    /// regions and patches, z: the patches' size, w: the regions' (world units).
    pub grade: [f32; 4],
}

/// A world's broad look ([`WorldLook`]) from its OTA's `planet`, as [`world_weights`] reads it.
/// Worlds it does not know take patches of value only, never a hue (a mod's alien world is
/// never tinted green).
pub fn world_look(planet: &str) -> WorldLook {
    let p = planet.to_ascii_lowercase();
    let has = |w: &str| p.contains(w);
    // A hue at luminance 1, and its value.
    let tone = |rgb: [f32; 3], value: f32| {
        let l = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        [rgb[0] / l, rgb[1] / l, rgb[2] / l, value]
    };
    let grey = |value: f32| [1.0, 1.0, 1.0, value];
    let look = |dry, dark, shore, strength, grade| WorldLook { dry, dark, shore, strength, grade };
    if has("metal") || has("urban") {
        // Grime.
        look(grey(1.0), grey(0.85), grey(1.0), [0.0, 0.3, 0.0, 0.0], [0.045, 0.04, 160.0, 900.0])
    } else if has("lava") || has("volcan") || has("basalt") {
        // Ash fields, cooled basalt in winding flows, rust scorched by the melt.
        look(tone([1.05, 1.0, 0.95], 1.12), grey(0.72), tone([1.5, 0.85, 0.6], 0.95), [0.3, 0.35, 0.4, 1.0], [0.05, 0.06, 150.0, 900.0])
    } else if has("ice") || has("snow") || has("arctic") || has("frozen") || has("tundra") {
        // Brighter drifts, blue-grey hollows.
        look(grey(1.06), tone([0.92, 0.98, 1.12], 0.9), grey(1.0), [0.3, 0.35, 0.0, 0.0], [0.045, 0.05, 200.0, 1200.0])
    } else if has("lunar") || has("moon") || has("regolith") || space_world(&p) {
        // Ejecta.
        look(grey(1.12), grey(0.85), grey(1.0), [0.35, 0.35, 0.0, 0.0], [0.045, 0.06, 140.0, 900.0])
    } else if has("acid") {
        // Sulphur crust, wet mud, a mineral rim by the pools.
        look(tone([1.12, 1.06, 0.75], 1.1), tone([1.0, 0.97, 0.92], 0.8), tone([0.97, 1.0, 1.05], 1.15), [0.3, 0.3, 0.35, 0.0], [0.0, 0.07, 170.0, 1000.0])
    } else if has("desert") || has("arid") || has("dune") || has("red") || has("mars") || has("martian") {
        // Pale crests, darker hollows, wet sand by the water.
        look(grey(1.1), grey(0.85), grey(0.8), [0.4, 0.4, 0.5, 0.0], [0.045, 0.06, 200.0, 1200.0])
    } else if p.trim().is_empty() || has("green") || has("lush") || has("archi") || has("water") || has("wet") || has("forest") || has("jungle") || has("earth") {
        // Green, lush, water and archipelago worlds, and maps that name none: dry straw,
        // lush hollows, wet earth by the water.
        look(tone([1.1, 1.0, 0.8], 1.1), tone([0.92, 1.03, 0.95], 0.82), tone([1.1, 1.0, 0.85], 0.88), [0.35, 0.35, 0.3, 0.0], [0.12, 0.08, 170.0, 1100.0])
    } else {
        // Slate, crystal, alien and barren worlds, and worlds unknown: value only.
        look(grey(1.08), grey(0.9), grey(1.0), [0.3, 0.3, 0.0, 0.0], [0.045, 0.05, 170.0, 1000.0])
    }
}

/// A world's colour grade over its ground (Reborn). TA painted some worlds in a narrow band of
/// dim values (Krocark's asphalt, Ashap Plateau's one red-brown), which in 3D read dull and flat
/// whatever the light: the grade draws the map's key (the log-average value of its land) towards
/// the world's own, spreads the values about it (darks deeper, lights brighter, easing off
/// towards black and white), makes dull colours richer and tints the shade and the light apart.
/// The art's hues and markings stay TA's; the units, the sky and the water are not graded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldGrade {
    /// The key (linear luminance) the map's own is drawn towards, and how far (0 not at all, 1
    /// all the way; the gain kept within [`WorldGrade::MAX_GAIN`] either way).
    pub key: f32,
    pub pull: f32,
    /// The colour balance (rgb at luminance 1) the map's land's is drawn towards, or none
    /// (`[0.0; 3]`), and how far: a map painted greyer or bluer than its world (TA Reborn's
    /// Cinder Basin) takes on some of the world's own warmth, its hues kept apart as painted.
    pub balance: [f32; 3],
    pub balance_pull: f32,
    /// The values' spread about the map's key (1 as painted).
    pub contrast: f32,
    /// The colours' saturation (1 as painted; dull colours gain more than vivid ones).
    pub saturation: f32,
    /// The tints (rgb at luminance 1) of the shade, below the key, and of the light, above it,
    /// and how strongly they show.
    pub shade: [f32; 3],
    pub light: [f32; 3],
    pub split: f32,
}

impl WorldGrade {
    /// The ground as painted.
    pub const NONE: WorldGrade = WorldGrade { key: 0.05, pull: 0.0, balance: [0.0; 3], balance_pull: 0.0, contrast: 1.0, saturation: 1.0, shade: [1.0; 3], light: [1.0; 3], split: 0.0 };
    /// The most a grade brightens or darkens a map's key, and a channel of its balance.
    pub const MAX_GAIN: f32 = 3.0;
    pub const MAX_BALANCE: f32 = 1.6;

    /// Whether it changes anything.
    pub fn is_none(&self) -> bool {
        (self.pull == 0.0 || self.key <= 0.0) && self.balance_pull == 0.0 && self.contrast == 1.0 && self.saturation == 1.0 && self.split == 0.0
    }

    /// The gain on a map whose land's key is `key`.
    pub fn gain(&self, key: f32) -> f32 {
        if self.pull == 0.0 || self.key <= 0.0 || key <= 0.0 {
            return 1.0;
        }
        (self.key / key).powf(self.pull).clamp(1.0 / Self::MAX_GAIN, Self::MAX_GAIN)
    }

    /// The gains (rgb) on a map whose land's colour balance is `balance` (rgb at luminance 1),
    /// each within [`WorldGrade::MAX_BALANCE`] and together keeping that colour's luminance.
    pub fn balance_gains(&self, balance: [f32; 3]) -> [f32; 3] {
        if self.balance_pull == 0.0 || self.balance.iter().any(|&b| b <= 0.0) || balance.iter().any(|&b| b <= 0.0) {
            return [1.0; 3];
        }
        let g = [0, 1, 2].map(|i| (self.balance[i] / balance[i]).powf(self.balance_pull).clamp(1.0 / Self::MAX_BALANCE, Self::MAX_BALANCE));
        let l = 0.2126 * balance[0] * g[0] + 0.7152 * balance[1] * g[1] + 0.0722 * balance[2] * g[2];
        g.map(|v| v / l.max(1e-4))
    }
}

/// A world's colour grade ([`WorldGrade`]) from its OTA's `planet`, as [`world_weights`] reads
/// it; none for the worlds TA painted rich enough. (Lava worlds' basalt darker, so the melt
/// stands out of it; TA Reborn's Cinder Basin, painted a lighter grey-brown than TA's lava
/// worlds, the most.)
pub fn world_grade(planet: &str) -> WorldGrade {
    let p = planet.to_ascii_lowercase();
    let has = |w: &str| p.contains(w);
    let grade = |key, pull, contrast, saturation, shade, light, split| WorldGrade { key, pull, balance: [0.0; 3], balance_pull: 0.0, contrast, saturation, shade, light, split };
    if has("urban") {
        // Asphalt and concrete in a dim grey band: lit like a city by day, the concrete warm
        // against cool asphalt.
        grade(0.07, 0.9, 1.5, 1.35, [0.9, 0.97, 1.15], [1.06, 1.0, 0.91], 0.6)
    } else if has("red") || has("mars") || has("martian") {
        // One muddy red-brown: rust-orange in the light, violet in the shade.
        grade(0.075, 0.8, 1.4, 1.1, [0.88, 0.9, 1.2], [1.1, 0.98, 0.8], 0.6)
    } else if has("lava") || has("volcan") || has("basalt") {
        // Basalt and ash under a pall of smoke: darker, the darks deepest, the shade cool and the
        // warm ground warmer (the two meeting in a neutral, not a mauve, midtone); half-way to a
        // rust-warm balance, a little redder than TA painted its lava worlds (TA Reborn's Cinder
        // Basin is painted a cooler, mauve grey-brown).
        WorldGrade { balance: [1.5, 0.9, 0.5], balance_pull: 0.5, ..grade(0.04, 0.8, 1.45, 1.3, [0.86, 0.95, 1.18], [1.14, 1.0, 0.8], 0.6) }
    } else {
        WorldGrade::NONE
    }
}

/// How much of the colour of the ground round them a world's walls keep (Reborn; 0 grey rock, 1
/// as painted): rock is less coloured than the ground it rises from, grass, sand and soil
/// (0.4); on the red planets the ground's red is the rock's own, its iron (0.85), or the walls
/// of a crater or a canyon stand grey in a red world; on slate worlds the ridges are the rock,
/// blue-grey slate, and on ice worlds the walls are the ice, blue (0.85).
pub fn world_wall_colour(planet: &str) -> f32 {
    let p = planet.to_ascii_lowercase();
    if p.contains("red") || p.contains("mars") || p.contains("martian") || p.contains("slate") || ice_world(&p) { 0.85 } else { 0.4 }
}

/// How light a world's walls are kept at the least, as a share of the ground of the region round
/// them (Reborn): three quarters, as TA painted the faces towards its camera dark, their shade,
/// not their rock; none on slate worlds, whose ridges TA painted dark slate on their tops and
/// faces alike (lifted to the light snow-plains round them, the faces were pale panels), nor on
/// ice worlds, whose crevasses are dark down their walls and floors alike (a crevasse's steep
/// part, lifted to the snow round it, was a pale slab lying in it).
pub fn world_wall_lift(planet: &str) -> f32 {
    let p = planet.to_ascii_lowercase();
    if p.contains("slate") || ice_world(&p) { 0.0 } else { 0.75 }
}

/// Whether a world (its `planet`, lower case) is one of ice and snow, as [`world_weights`] reads it.
fn ice_world(p: &str) -> bool {
    ["ice", "snow", "arctic", "frozen", "tundra"].iter().any(|w| p.contains(w))
}

/// Whether TA's hair-like strands over a world's hills are painted over (Reborn; see
/// terrain_strands.rs): on worlds of earth, grass and sand, where they are the painter's
/// hatching of the slopes; not where lines are what the ground is made of (metal and urban
/// worlds' seams, the moon's crater rims, crystal's and ice's cracks, slate's strata, lava's
/// and acid's veins).
pub fn world_strands(planet: &str) -> bool {
    let p = planet.to_ascii_lowercase();
    let has = |w: &str| p.contains(w);
    !["metal", "urban", "lunar", "moon", "regolith", "crystal", "ice", "snow", "arctic", "frozen", "tundra", "slate", "lava", "volcan", "basalt", "acid", "alien", "barren"].iter().any(|w| has(w)) && !space_world(&p)
}

/// Whether a world (its `planet`, lower case) is one in space: rocks floating in the void (TA
/// Reborn's asteroid maps). Its void cells show the sky in the perspective view (see
/// [`crate::Renderer::set_ground_world`]), and its ground is the moon's.
pub fn space_world(p: &str) -> bool {
    ["asteroid", "space", "orbit"].iter().any(|w| p.contains(w))
}

/// The materials' weights for a world (the OTA's `planet`), and whether its walls are metal.
pub fn world_weights(planet: &str) -> ([f32; 10], bool) {
    let p = planet.to_ascii_lowercase();
    let has = |w: &str| p.contains(w);
    //                       sand soil grass rock gravel snow ice  volc metal rego
    let (w, metal) = if has("metal") || has("urban") {
        ([0.0, 0.0, 0.0, 0.5, 0.6, 0.0, 0.0, 0.0, 1.6, 0.0], true)
    } else if has("lava") || has("volcan") || has("basalt") {
        ([0.2, 0.6, 0.0, 1.0, 0.6, 0.0, 0.0, 1.5, 0.0, 0.0], false)
    } else if has("ice") || has("snow") || has("arctic") || has("frozen") || has("tundra") {
        ([0.3, 0.2, 0.0, 1.0, 0.4, 1.5, 1.1, 0.0, 0.0, 0.0], false)
    } else if has("lunar") || has("moon") || has("regolith") || space_world(&p) {
        ([0.0, 0.0, 0.0, 0.8, 0.5, 0.0, 0.0, 0.0, 0.0, 1.6], false)
    } else if has("slate") {
        ([0.3, 0.5, 0.3, 1.4, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0], false)
    } else if has("crystal") {
        ([0.3, 0.3, 0.0, 1.2, 0.6, 0.0, 0.6, 0.0, 0.0, 0.6], false)
    } else if has("acid") {
        ([0.4, 1.2, 0.3, 1.0, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0], false)
    } else if has("red") || has("mars") || has("martian") {
        ([1.0, 1.3, 0.0, 1.0, 0.6, 0.0, 0.0, 0.0, 0.0, 0.0], false)
    } else if has("desert") || has("arid") || has("dune") {
        ([1.4, 0.8, 0.15, 1.0, 0.6, 0.0, 0.0, 0.0, 0.0, 0.0], false)
    } else if has("earth") {
        // TA Reborn's maps of real places: every ground there is, from the deserts' sand to the
        // ice caps' snow.
        ([1.1, 1.0, 1.25, 1.0, 0.6, 0.9, 0.5, 0.0, 0.0, 0.0], false)
    } else if has("alien") || has("barren") {
        // Unknown alien pigments must not turn into terrestrial turf merely because green.
        ([0.5, 0.8, 0.0, 1.2, 0.8, 0.0, 0.0, 0.3, 0.0, 0.4], false)
    } else {
        // Green, lush, archipelago and water worlds, and maps that name none.
        ([1.0, 1.0, 1.3, 1.0, 0.6, 0.0, 0.0, 0.0, 0.0, 0.0], false)
    };
    (w, metal)
}

pub struct TerrainDetail {
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    world: wgpu::Buffer,
}

impl TerrainDetail {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> TerrainDetail {
        let levels = SIZE.ilog2() + 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("terrain detail"),
            size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: MATERIALS.len() as u32 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, png) in pngs().iter().enumerate() {
            let texels = match texels(png) {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("terrain detail {}: {e}", MATERIALS[layer]);
                    vec![128u8; (SIZE * SIZE * 4) as usize]
                }
            };
            for (level, (w, h, data)) in detail_mips(&texels).into_iter().enumerate().take(levels as usize) {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: layer as u32 }, aspect: wgpu::TextureAspect::All },
                    &data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
            }
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain detail"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let world = device.create_buffer(&wgpu::BufferDescriptor { label: Some("terrain detail world"), size: std::mem::size_of::<WorldUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain detail"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain detail"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: world.as_entire_binding() },
            ],
        });
        let detail = TerrainDetail { layout, bind_group, world };
        detail.set_world(queue, "", WorldGrade::NONE.key, [1.0; 3]);
        detail
    }

    /// The map's world (its OTA's `planet`): which materials its ground may be, and its look;
    /// and its grade (`key` and `balance`: the key of the map's land and its colour balance, see
    /// [`WorldGrade`]): the gains the grade balances its colours by, and the grade as the shaders
    /// take it (the frame's `world_grade`, see `world_grade` in common.wgsl: x the gain, y the
    /// contrast, z the saturation, w the key after the gain; the shade's tint, w how strongly
    /// the tints show; the light's tint, w 1 when there is a grade at all). (`TA_GROUND_DETAIL=0`
    /// leaves the materials out, to compare.)
    pub fn set_world(&self, queue: &wgpu::Queue, planet: &str, key: f32, balance: [f32; 3]) -> ([f32; 3], [[f32; 4]; 3]) {
        let (mut w, mut metal) = world_weights(planet);
        if std::env::var("TA_GROUND_DETAIL").is_ok_and(|v| v == "0") {
            (w, metal) = ([0.0; 10], false);
        }
        // (`TA_GROUND_DEBUG=1` paints each place in its material's colour, to check the guesses.)
        let debug = std::env::var("TA_GROUND_DEBUG").is_ok_and(|v| v == "1");
        let mut u = WorldUniform { flags: [if metal { 1.0 } else { 0.0 }, if debug { 1.0 } else { 0.0 }, world_wall_colour(planet), world_wall_lift(planet)], ..Default::default() };
        for (i, v) in w.iter().enumerate() {
            u.weights[i / 4][i % 4] = *v;
        }
        // (`TA_GROUND_MACRO=0` leaves the broad look out, to compare.)
        let mut look = world_look(planet);
        if std::env::var("TA_GROUND_MACRO").is_ok_and(|v| v == "0") {
            look.strength = [0.0; 4];
            look.grade[1] = 0.0;
        }
        u.look = [look.dry, look.dark, look.shore, look.strength, look.grade];
        // (`TA_WORLD_GRADE=0` leaves the grade out, to compare.)
        let mut grade = world_grade(planet);
        if std::env::var("TA_WORLD_GRADE").is_ok_and(|v| v == "0") {
            grade = WorldGrade::NONE;
        }
        let gain = grade.gain(key);
        // (The balance's gains multiply both tints, shown in full: the shader's tint, mixed
        // between them, is then the balance times the tint as the split shows it.)
        let b = grade.balance_gains(balance);
        log::info!("world {planet:?}: land key {key:.4}, balance {balance:.3?}, grade gain {gain:.2}, balance gains {b:.3?}");
        let tint = |t: [f32; 3]| [0, 1, 2].map(|i| b[i] * (1.0 + (t[i] - 1.0) * grade.split));
        let (shade, light) = (tint(grade.shade), tint(grade.light));
        let shown = [
            [gain, grade.contrast, grade.saturation, key.max(1e-4) * gain],
            [shade[0], shade[1], shade[2], 1.0],
            [light[0], light[1], light[2], if grade.is_none() { 0.0 } else { 1.0 }],
        ];
        queue.write_buffer(&self.world, 0, bytemuck::bytes_of(&u));
        (b, shown)
    }
}

/// A material's texels from its PNG (height in L, brightness in A): brightness, the height's
/// slopes (wrapping round, as the map does) and the height.
fn texels(png: &[u8]) -> anyhow::Result<Vec<u8>> {
    let img = image::load_from_memory(png)?.to_luma_alpha8();
    anyhow::ensure!(img.width() == SIZE && img.height() == SIZE, "{}x{}, not {SIZE}x{SIZE}", img.width(), img.height());
    let n = SIZE as usize;
    let h = |x: usize, y: usize| img.get_pixel((x % n) as u32, (y % n) as u32)[0] as f32 / 255.0;
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let p = img.get_pixel(x as u32, y as u32);
            let sx = (h(x + 1, y) - h(x + n - 1, y)) * 0.5 * SLOPE_GAIN;
            let sy = (h(x, y + 1) - h(x, y + n - 1)) * 0.5 * SLOPE_GAIN;
            // Exactly neutral at byte 128. Decoding 128 as 128/255*2-1 used to add a
            // persistent positive slope even to perfectly flat/coarsely filtered ground.
            let byte = |v: f32| (v.clamp(-1.0, 1.0) * 127.0 + 128.0).round() as u8;
            let i = (y * n + x) * 4;
            out[i] = p[1];
            out[i + 1] = byte(sx);
            out[i + 2] = byte(sy);
            out[i + 3] = p[0];
        }
    }
    Ok(out)
}

/// Fine grain must converge to a plain, level surface as it becomes subpixel. A regular box
/// mip chain retains brightness bias and accumulates byte rounding in its normal channels.
fn detail_mips(texels: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = byte_mips(SIZE, SIZE, 4, texels);
    for (level, (_, _, bytes)) in levels.iter_mut().enumerate() {
        let fade = ((level as f32 - 3.0) / 6.0).clamp(0.0, 1.0);
        for pixel in bytes.as_chunks_mut::<4>().0 {
            for channel in pixel {
                *channel = (*channel as f32 * (1.0 - fade) + 128.0 * fade).round() as u8;
            }
        }
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_materials_shipped_load_and_tile() {
        for (name, png) in MATERIALS.iter().zip(pngs()) {
            let t = texels(png).unwrap_or_else(|e| panic!("{name}: {e}"));
            let n = SIZE as usize;
            // Brightness about neutral on average, and the edges continue each other.
            let mean = t.chunks(4).map(|p| p[0] as f32).sum::<f32>() / (n * n) as f32;
            assert!((mean - 128.0).abs() < 40.0, "{name}: mean brightness {mean}");
            let height = |x: usize, y: usize| t[(y * n + x) * 4 + 3] as f32;
            let seam: f32 = (0..n).map(|y| (height(0, y) - height(n - 1, y)).abs()).sum::<f32>() / n as f32;
            let inside: f32 = (0..n).map(|y| (height(n / 2, y) - height(n / 2 - 1, y)).abs()).sum::<f32>() / n as f32;
            assert!(seam < inside * 3.0 + 2.0, "{name}: seam {seam} against {inside} inside");
        }
    }

    #[test]
    fn worlds_in_space_are_the_moons_ground() {
        for planet in ["Asteroid Belt", "Deep Space", "Orbital Station"] {
            assert!(space_world(&planet.to_ascii_lowercase()), "{planet}");
            assert_eq!(world_weights(planet), world_weights("Lunar"), "{planet}");
            assert!(!world_strands(planet) && world_grade(planet).is_none(), "{planet}");
        }
        for planet in ["Lunar", "Green planet", "Metal", "Archipelago", ""] {
            assert!(!space_world(&planet.to_ascii_lowercase()), "{planet}");
        }
    }

    #[test]
    fn only_the_dull_worlds_are_graded() {
        for planet in ["Green planet", "Archipelago", "Desert", "Wet Desert", "Ice", "Lunar", "Metal", "Acid", "Crystal", "Slate", "Water World", "Lush", ""] {
            assert!(world_grade(planet).is_none(), "{planet} is graded");
        }
        for planet in ["Urban", "Red Planet", "Lava"] {
            let g = world_grade(planet);
            assert!(!g.is_none(), "{planet} is not graded");
            // A dim map is lifted more than a bright one of the same world, within the limit.
            assert!(g.gain(0.02) > g.gain(0.06), "{planet}");
            assert!(g.gain(1e-4) <= WorldGrade::MAX_GAIN && g.gain(1.0) >= 1.0 / WorldGrade::MAX_GAIN, "{planet}");
        }
        assert_eq!(WorldGrade::NONE.gain(0.01), 1.0);
        // Their walls: as red as the ground on the red planets, grey rock elsewhere.
        assert!(world_wall_colour("Red Planet") > 0.8 && world_wall_colour("Green planet") == 0.4 && world_wall_colour("Lava") == 0.4);
        // Lifted out of TA's painted shade, but not slate's dark rock nor ice's crevasses.
        assert!(world_wall_lift("Green planet") == 0.75 && world_wall_lift("Metal") == 0.75 && world_wall_lift("Slate") == 0.0 && world_wall_lift("Ice") == 0.0);
    }

    #[test]
    fn a_map_painted_cooler_than_its_world_is_warmed() {
        let lava = world_grade("Lava");
        // TA's lava maps are painted near the world's own balance: much as they were.
        let ta = lava.balance_gains([1.4, 0.93, 0.51]);
        assert!(ta.iter().all(|g| (g - 1.0).abs() < 0.05), "{ta:?}");
        // TA Reborn's Cinder Basin, a cooler grey-brown: less blue, as light.
        let cinder = [1.336, 0.926, 0.743];
        let g = lava.balance_gains(cinder);
        assert!(g[2] < 0.9 && g[0] > 1.0, "{g:?}");
        let l = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        assert!((l([0, 1, 2].map(|i| cinder[i] * g[i])) - l(cinder)).abs() < 1e-3);
        assert_eq!(WorldGrade::NONE.balance_gains(cinder), [1.0; 3]);
        assert_eq!(world_grade("Urban").balance_gains(cinder), [1.0; 3]);
    }

    #[test]
    fn worlds_weigh_their_own_ground() {
        let (desert, _) = world_weights("Desert");
        assert!(desert[0] > desert[2], "sand over grass in a desert");
        let (ice, _) = world_weights("Ice");
        assert!(ice[5] > 1.0 && ice[2] == 0.0, "snow, no grass on ice");
        let (_, metal) = world_weights("Metal");
        assert!(metal, "metal worlds have metal walls");
        let (green, metal) = world_weights("");
        assert!(green[2] > 1.0 && !metal);
    }

    #[test]
    fn every_world_family_has_appropriate_surfaces() {
        let cases: &[(&[&str], usize)] = &[
            (&["Green", "Lush", "Archipelago", "Water", "Oolok", ""], 2),
            (&["Desert", "arid dunes"], 0),
            (&["Mars", "Martian", "Red Planet"], 1),
            (&["Metal", "Urban"], 8),
            (&["Lava", "Volcanic", "Basalt"], 7),
            (&["Ice", "Snow", "Arctic", "Frozen", "Tundra"], 5),
            (&["Lunar", "Moon", "Regolith"], 9),
            (&["Slate", "Alien", "Barren", "Crystal"], 3),
            (&["Acid"], 1),
        ];
        for &(names, expected) in cases {
            for &name in names {
                let (weights, metal) = world_weights(name);
                assert!(weights[expected] >= 1.0, "{name}: expected {}", MATERIALS[expected]);
                assert!(weights.iter().all(|w| w.is_finite() && *w >= 0.0));
                assert_eq!(metal, expected == 8, "{name}: wall material");
                if matches!(expected, 5 | 7 | 8 | 9) || name == "Mars" || name == "Alien" {
                    assert_eq!(weights[2], 0.0, "{name}: no terrestrial grass");
                }
            }
        }
        assert_eq!(world_weights("Unspecified mod world"), world_weights(""), "unknown worlds retain a stable mixed-ground fallback");
    }

    #[test]
    fn every_world_family_has_a_sensible_look() {
        for name in ["Green", "Lush", "Archipelago", "Desert", "Mars", "Metal", "Urban", "Lava", "Ice", "Lunar", "Slate", "Crystal", "Acid", "Alien", "Unspecified mod world", ""] {
            let look = world_look(name);
            for tone in [look.dry, look.dark, look.shore] {
                assert!(tone.iter().all(|v| v.is_finite() && *v > 0.0), "{name}");
                let l = 0.2126 * tone[0] + 0.7152 * tone[1] + 0.0722 * tone[2];
                assert!((l - 1.0).abs() < 0.01, "{name}: a hue at luminance 1");
            }
            assert!(look.strength[..3].iter().all(|s| (0.0..=1.0).contains(s)), "{name}");
            assert!((0.0..=0.3).contains(&look.grade[0]) && look.grade[2] > 0.0 && look.grade[3] > look.grade[2], "{name}");
        }
        // Metal and unknown worlds take no hue; lava's flows stay light enough to see units on.
        for name in ["Metal", "Alien", "Unspecified mod world"] {
            let look = world_look(name);
            assert!([look.dry, look.dark, look.shore].iter().all(|t| t[0] == t[1] && t[1] == t[2]), "{name}");
        }
        assert!(world_look("Lava").dark[3] >= 0.6);
        assert_eq!(std::mem::size_of::<WorldUniform>(), 144);
    }

    #[test]
    fn every_surface_filters_to_plain_level_ground() {
        for (name, png) in MATERIALS.iter().zip(pngs()) {
            let base = texels(png).unwrap();
            assert!(base.as_chunks::<4>().0.iter().any(|p| p[3] < 96), "{name}: non-flat original height");
            assert!(base.as_chunks::<4>().0.iter().any(|p| p[3] > 160), "{name}: non-flat original height");
            let chain = detail_mips(&base);
            assert_eq!(chain.len(), 10);
            assert_eq!(chain.last(), Some(&(1, 1, vec![128; 4])), "{name}: neutral minified mip");
            let n = SIZE as usize;
            let h = |x: usize, y: usize| base[(y*n+x)*4+3] as f32;
            let seam = (0..n).map(|x| (h(x, 0)-h(x, n-1)).abs()).sum::<f32>() / n as f32;
            let inside = (0..n).map(|x| (h(x, n/2)-h(x, n/2-1)).abs()).sum::<f32>() / n as f32;
            assert!(seam < inside*3.0+2.0, "{name}: vertical seam {seam} against {inside}");
        }
    }

    #[test]
    fn installed_map_roster_has_finite_material_profiles() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "original").unwrap();
        let maps = vfs.list("maps", Some("ota"), true);
        assert!(!maps.is_empty(), "installed game has map metadata");
        let mut worlds = std::collections::BTreeSet::new();
        for path in &maps {
            let data = vfs.read(path).unwrap();
            let doc = ta_formats::tdf::parse_bytes(&data);
            let planet = doc.section("GlobalHeader").and_then(|s| s.get("planet")).unwrap_or("");
            let (weights, _) = world_weights(planet);
            assert!(weights.iter().all(|v| v.is_finite() && *v >= 0.0), "{path}: {planet}");
            assert!(weights.iter().any(|v| *v >= 1.0), "{path}: {planet} has no usable ground material");
            worlds.insert(planet.to_string());
        }
        eprintln!("World material coverage: {} installed map metadata files, planet labels {worlds:?}", maps.len());
    }
}
