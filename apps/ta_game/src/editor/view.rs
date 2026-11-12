//! Showing the map being edited: edits go to the renderer's terrain in place (the changed
//! heights and tiles only; the whole terrain is loaded again when the sea level changes or new
//! tiles join the atlas), features are drawn as TA's sprites and wreck models, and the tools'
//! marks (brush, tile outline, start positions) are painted over the 3D view.

use std::collections::HashMap;

use glam::{Mat4, Vec2, Vec3, Vec4Swizzles};
use ta_data::GameData;
use ta_formats::gaf::Gaf;
use ta_formats::image::Palette;
use ta_render::{Camera, DrawInstance, ModelId, Renderer, SpriteId, SpriteInstance};
use ta_vfs::Vfs;

use super::doc::{CELL_PX, MapDoc};
use super::history::Changes;

/// How one feature type is drawn (as the game's feature view does).
#[derive(Clone, Default)]
struct Visual {
    frames: Vec<SpriteId>,
    shadow: Option<SpriteId>,
    /// Upright billboard (trees, spires) vs flat decal (metal patches, craters).
    upright: bool,
    /// Upright, but painted lying (cars, pipes): laid on the ground in the perspective camera.
    lies: bool,
    translucent: bool,
    model: Option<ModelId>,
}

/// A feature on the map: its type and where it stands.
struct Placed {
    def: usize,
    pos: Vec3,
}

#[derive(Default)]
pub struct View {
    visuals: HashMap<usize, Visual>,
    gafs: HashMap<String, Option<Gaf>>,
    placed: Vec<Placed>,
    /// Features named on the map that the game data doesn't know (drawn nowhere).
    pub unknown_features: usize,
    decals: Vec<SpriteInstance>,
    billboards: Vec<SpriteInstance>,
    lying: Vec<SpriteInstance>,
    matrices: Vec<Vec<Mat4>>,
    models: Vec<ModelId>,
    /// Which terrain chunks were under water when the water surface was last built.
    wet: Option<Vec<bool>>,
    planet: String,
}

impl View {
    /// Loads the map into the renderer (terrain, water) with a fresh sprite atlas.
    pub fn new(renderer: &mut Renderer, doc: &MapDoc, palette: &Palette) -> View {
        renderer.sprites.reset_atlas();
        renderer.clear_particles();
        renderer.set_fog_of_war(None);
        let mut view = View {
            visuals: HashMap::new(),
            gafs: HashMap::new(),
            placed: Vec::new(),
            unknown_features: 0,
            decals: Vec::new(),
            billboards: Vec::new(),
            lying: Vec::new(),
            matrices: Vec::new(),
            models: Vec::new(),
            wet: None,
            planet: String::new(),
        };
        view.reload_terrain(renderer, doc, palette);
        view
    }

    /// Uploads the whole terrain again (and the water, the ground beyond the edges...).
    pub fn reload_terrain(&mut self, renderer: &mut Renderer, doc: &MapDoc, palette: &Palette) {
        renderer.load_map(&doc.tnt, palette);
        self.set_world(renderer, &doc.props.planet);
        self.wet = renderer.terrain.as_ref().and_then(|t| ta_render::water::wet_chunks(&t.field));
    }

    fn set_world(&mut self, renderer: &mut Renderer, planet: &str) {
        // (Its world's ground, look, grade and sky, as in a battle, not the last battle's; its
        // tiles exactly as they are painted with; the sea of dark clouds beyond its edges, as a
        // battle has, so the map's bounds are plain.)
        renderer.set_ground_world(planet, false);
        renderer.lighting.sky.beyond = ta_render::Beyond::Clouds;
        // Lava planets' seas are lava, acid planets' acid; elsewhere the painted art decides (as
        // in a battle).
        renderer.lighting.water.fluid = None;
        let painted = renderer.fluid();
        let planet_lower = planet.to_ascii_lowercase();
        renderer.lighting.water.fluid = match painted {
            Some(ta_render::Fluid::Water) if planet_lower.contains("lava") => Some(ta_render::Fluid::Lava),
            Some(ta_render::Fluid::Water) if planet_lower.contains("acid") => Some(ta_render::Fluid::Acid),
            _ => None,
        };
        self.planet = planet.to_owned();
    }

    /// Shows what an edit changed: heights and tiles in place, features again.
    pub fn push(&mut self, renderer: &mut Renderer, doc: &MapDoc, data: &GameData, vfs: &Vfs, palette: &Palette, changes: &Changes) {
        if changes.props && self.planet != doc.props.planet {
            self.set_world(renderer, &doc.props.planet);
        }
        let atlas_short = renderer.terrain.as_ref().is_none_or(|t| t.tile_count() < doc.tnt.tiles.len());
        if changes.sea_level || (changes.tiles.is_some() && atlas_short) {
            self.reload_terrain(renderer, doc, palette);
        } else if let Some(terrain) = renderer.terrain.as_mut() {
            let queue = &renderer.gpu.queue;
            if let Some(r) = changes.heights {
                terrain.update_heights(queue, &doc.tnt.heights, r.x0, r.z0, r.x1, r.z1);
            }
            if let Some(r) = changes.tiles {
                terrain.update_tiles(queue, &doc.tnt.tile_map, r.x0, r.z0, r.x1, r.z1);
            }
            if changes.heights.is_some() {
                renderer.heights_changed();
            }
        }
        if changes.features || changes.sea_level {
            self.place_features(renderer, doc, data, vfs, palette);
        } else if let Some(r) = changes.heights {
            // Features on the changed ground stand at its new height.
            let (x0, z0, x1, z1) = ((r.x0 as f32 - 1.0) * CELL_PX, (r.z0 as f32 - 1.0) * CELL_PX, (r.x1 as f32 + 1.0) * CELL_PX, (r.z1 as f32 + 1.0) * CELL_PX);
            for p in self.placed.iter_mut().filter(|p| p.pos.x >= x0 && p.pos.x <= x1 && p.pos.z >= z0 && p.pos.z <= z1) {
                p.pos.y = doc.ground(p.pos.x, p.pos.z);
            }
        }
    }

    /// Rebuilds the water surface when the chunks under the sea changed (after a stroke).
    pub fn refresh_water(&mut self, renderer: &mut Renderer, doc: &MapDoc, palette: &Palette) {
        let Some(terrain) = renderer.terrain.as_ref() else { return };
        let wet = ta_render::water::wet_chunks(&terrain.field);
        if wet != self.wet {
            renderer.water = ta_render::WaterSurface::new(&renderer.gpu.device, &terrain.field, &doc.tnt, palette);
            self.wet = wet;
        }
    }

    /// Lists every feature on the map with its position, loading the art of new types.
    pub fn place_features(&mut self, renderer: &mut Renderer, doc: &MapDoc, data: &GameData, vfs: &Vfs, palette: &Palette) {
        self.placed.clear();
        self.unknown_features = 0;
        let found: Vec<((u32, u32), usize)> = doc
            .features()
            .filter_map(|(origin, name)| match data.feature_id(name) {
                Some(id) => Some((origin, id)),
                None => {
                    self.unknown_features += 1;
                    None
                }
            })
            .collect();
        for (origin, def) in found {
            self.load_visual(renderer, data, vfs, palette, def);
            let d = &data.features[def];
            let x = (origin.0 as f32 + d.footprint_x.max(1) as f32 / 2.0) * CELL_PX;
            let z = (origin.1 as f32 + d.footprint_z.max(1) as f32 / 2.0) * CELL_PX;
            self.placed.push(Placed { def, pos: Vec3::new(x, doc.ground(x, z), z) });
        }
    }

    /// The art of a feature type: GAF sprite frames and shadow, or its 3DO model.
    pub fn load_visual(&mut self, renderer: &mut Renderer, data: &GameData, vfs: &Vfs, palette: &Palette, def_id: usize) {
        if self.visuals.contains_key(&def_id) {
            return;
        }
        let def = &data.features[def_id];
        let mut v = Visual { upright: def.height > 4.0 && !def.is_metal_deposit(), translucent: def.anim_trans, ..Default::default() };
        if !def.object.is_empty() {
            if let Some(models) = renderer.models.as_mut() {
                let name = def.object.to_ascii_lowercase();
                v.model = vfs
                    .read_opt(&format!("objects3d/{name}.3do"))
                    .and_then(|b| ta_formats::model3do::Model3do::parse(&b).ok())
                    .map(|m| models.library.add(&name, &m, &models.textures, palette));
            }
        } else if !def.filename.is_empty() {
            let key = def.filename.to_ascii_lowercase();
            let gaf = self.gafs.entry(key.clone()).or_insert_with(|| vfs.read_opt(&format!("anims/{key}.gaf")).and_then(|b| Gaf::parse(&b).ok()));
            if let Some(gaf) = gaf {
                let (outlined, translucent) = (v.upright && !v.translucent, v.translucent);
                let atlas = &mut renderer.sprites.atlas;
                if let Some(entry) = gaf.entry(&def.seq_name) {
                    v.frames = entry
                        .frames
                        .iter()
                        .filter_map(|f| match (outlined, translucent) {
                            (true, _) => atlas.add_gaf_outlined(f, palette),
                            (false, false) => atlas.add_gaf_decal(f, palette, def.flamable),
                            (false, true) => atlas.add_gaf(f, palette, false),
                        })
                        .collect();
                    if let Some(p) = entry.frames.first().and_then(|f| crate::features::Painting::of(f, palette)) {
                        v.lies = v.upright && crate::features::painted_lying(def, &p);
                    }
                }
                if !def.seq_name_shadow.is_empty() && let Some(entry) = gaf.entry(&def.seq_name_shadow) {
                    v.shadow = entry.frames.first().and_then(|f| atlas.add_gaf(f, palette, true));
                }
            }
        }
        self.visuals.insert(def_id, v);
    }

    /// Submits this frame's features in `bounds` (the ground in view, its least and greatest
    /// corners) to the renderer (and no units, effects or lights); with `modern` (the
    /// perspective camera) those painted lying are laid on the ground. Without
    /// `features` (the view's switch) none are drawn. `ghost`: a feature type, where it would
    /// stand and whether it fits there, drawn see-through and tinted green or red (the one about
    /// to be placed, under the pointer).
    pub fn prepare(&mut self, renderer: &mut Renderer, (view_min, view_max): (Vec2, Vec2), time: f32, modern: bool, features: bool, ghost: Option<(usize, Vec3, bool)>) {
        self.decals.clear();
        self.billboards.clear();
        self.lying.clear();
        self.matrices.clear();
        self.models.clear();
        let margin = 128.0;
        let tint = if ghost.is_some_and(|g| g.2) { [0.8, 1.15, 0.8] } else { [1.3, 0.6, 0.55] };
        let ghost = ghost.map(|(def, pos, _)| Placed { def, pos });
        let shown = self.placed.iter().filter(|_| features).chain(ghost.as_ref());
        for (k, p) in shown.enumerate() {
            let see_through = ghost.as_ref().is_some_and(|g| std::ptr::eq(g, p));
            if p.pos.x < view_min.x - margin || p.pos.x > view_max.x + margin || p.pos.z < view_min.y - margin || p.pos.z > view_max.y + margin {
                continue;
            }
            let Some(v) = self.visuals.get(&p.def) else { continue };
            if let Some(model) = v.model {
                if let Some(models) = renderer.models.as_ref() {
                    let root = Mat4::from_translation(p.pos);
                    self.matrices.push(models.library.info(model).rest_pose().into_iter().map(|m| root * m).collect());
                    self.models.push(model);
                }
                continue;
            }
            if let Some(shadow) = v.shadow {
                self.decals.push(SpriteInstance::new([p.pos.x, p.pos.y + 0.3, p.pos.z], shadow).alpha(if see_through { 0.25 } else { 0.5 }));
            }
            if v.frames.is_empty() {
                continue;
            }
            let frame = v.frames[((time * 10.0) as usize + k) % v.frames.len()];
            let alpha = if see_through { 0.55 } else if v.translucent { 0.6 } else { 1.0 };
            let mut inst = SpriteInstance::new([p.pos.x, p.pos.y + 0.2, p.pos.z], frame).alpha(alpha);
            if see_through {
                inst = inst.tint(tint);
            }
            if v.lies && modern {
                self.lying.push(inst);
            } else if v.upright {
                self.billboards.push(inst);
            } else {
                self.decals.push(inst);
            }
        }
        let instances: Vec<DrawInstance<'_>> =
            self.models.iter().zip(&self.matrices).map(|(&model, m)| DrawInstance { model, pieces: m, team: 0, flags: 0, build: 1.0, damage: 0.0, travel: [0.0; 2], piece: None, playing: ta_render::model::PLAYING_ALL }).collect();
        renderer.prepare_models(instances);
        renderer.prepare_sprites(&self.decals, &self.billboards, &[], &[], &self.lying);
        renderer.prepare_beams(&[]);
        renderer.prepare_rings(&[]);
        renderer.prepare_wakes(&[], &[]);
        renderer.set_lights(&[]);
    }
}

/// Projects world points to egui points on the screen.
pub struct Projector {
    view_proj: Mat4,
    viewport: Vec2,
    ppp: f32,
}

impl Projector {
    pub fn new(camera: &Camera, pixels_per_point: f32) -> Projector {
        Projector { view_proj: camera.view_proj(), viewport: camera.viewport, ppp: pixels_per_point }
    }

    pub fn project(&self, p: Vec3) -> Option<egui::Pos2> {
        let c = self.view_proj * p.extend(1.0);
        if c.w <= 1e-4 {
            return None;
        }
        let ndc = c.xy() / c.w;
        let s = Vec2::new((ndc.x + 1.0) * 0.5 * self.viewport.x, (1.0 - ndc.y) * 0.5 * self.viewport.y) / self.ppp;
        Some(egui::pos2(s.x, s.y))
    }
}

/// Where the badges of start positions whose flags stand at `heads` (on the screen; none
/// for one out of sight) go: where they stand, or moved apart until none covers another
/// (`radius` each, in the heads' units), the closest pairs first. The same for the same
/// heads, so what is drawn is what is clicked.
pub fn badge_layout(heads: &[Option<egui::Pos2>], radius: f32) -> Vec<Option<egui::Pos2>> {
    let mut at = heads.to_vec();
    let apart = radius * 2.0 + radius * 0.3;
    for _ in 0..16 {
        let mut moved = false;
        for i in 0..at.len() {
            for j in i + 1..at.len() {
                let (Some(a), Some(b)) = (at[i], at[j]) else { continue };
                let d = b - a;
                let distance = d.length();
                if distance >= apart - 0.01 {
                    continue;
                }
                // (Two on one spot part along a direction of their own.)
                let along = if distance > 0.01 { d / distance } else { egui::Vec2::angled((i * 7 + j * 3) as f32 * 2.399) };
                let push = along * (apart - distance) / 2.0;
                at[i] = Some(a - push);
                at[j] = Some(b + push);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    at
}

/// A ring on the ground (points lifted a little so the terrain doesn't hide them).
pub fn ground_ring(doc: &MapDoc, x: f32, z: f32, radius: f32, steps: usize) -> Vec<Vec3> {
    (0..=steps)
        .map(|i| {
            let a = i as f32 / steps as f32 * std::f32::consts::TAU;
            let (px, pz) = (x + a.cos() * radius, z + a.sin() * radius);
            Vec3::new(px, doc.ground(px, pz).max(doc.tnt.sea_level as f32) + 2.0, pz)
        })
        .collect()
}
