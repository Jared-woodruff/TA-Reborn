//! Map feature visuals: GAF sprites (trees, rocks, metal deposits), 3DO features (wrecks), and
//! up close TA Reborn's own 3D trees and rocks in place of the paintings (`ta_render::foliage`).

use std::collections::HashMap;

use glam::{Mat4, Vec2, Vec3};
use ta_data::feature::FeatureDef;
use ta_formats::gaf::{Gaf, GafFrame};
use ta_formats::image::Palette;
use ta_render::foliage::{FoliageInstance, FoliageLibrary, FoliageModelId};

use crate::feature_strokes::{Junction, Stroke, Strokes, dist};
use ta_render::{DrawInstance, ModelId, Renderer, SpriteId, SpriteInstance};
use ta_sim::Sim;
use ta_vfs::Vfs;

/// How one feature type is drawn.
#[derive(Clone, Default)]
struct FeatureVisual {
    frames: Vec<SpriteId>,
    /// While on fire (TA's `seqnameburn`: the tree in flames).
    burn_frames: Vec<SpriteId>,
    shadow: Option<SpriteId>,
    /// Upright billboard (trees, spires) vs flat decal (metal deposits, craters).
    upright: bool,
    /// Upright to TA, but painted lying (cars, pipes, barriers, ruins; see
    /// `ta_render::sprites::painted_lying`): laid on the ground under the perspective camera.
    lies: bool,
    /// Sways in the wind (trees and bushes: upright and burnable).
    sways: bool,
    /// A metal deposit: how rich (0..1), its ore glinting in the sun.
    ore: f32,
    translucent: bool,
    model: Option<ModelId>,
    /// Positive gap under the rest mesh (some wrecks' root offsets leave them above zero).
    /// Removed only for a feature anchored on the ground; buried or elevated art stays so.
    ground_sink: f32,
    /// The 3D tree or rock standing in for the painting up close.
    stand_in: Option<StandIn>,
    /// The light it casts by night (TA Reborn's own lanterns, lit windows: `rebornlight`).
    light: Option<([f32; 3], f32, f32)>,
    /// It sends up fireworks by night (`rebornfireworks`).
    fireworks: bool,
}

/// How far a model whose geometry sits wholly above its origin must come down to meet the
/// ground. Follow the complete rest hierarchy and ignore selection plates: the Reaper wreck
/// has a low selection plate but its visible hull starts nine units above the origin.
fn model_ground_sink(rest: &[Mat4], surfaces: &[Option<std::sync::Arc<crate::scene::PieceSurface>>]) -> f32 {
    let mut bottom = f32::INFINITY;
    for (matrix, surface) in rest.iter().zip(surfaces) {
        if let Some(surface) = surface {
            for &point in &surface.points {
                bottom = bottom.min(matrix.transform_point3(point).y);
            }
        }
    }
    if bottom.is_finite() { bottom.max(0.0) } else { 0.0 }
}

fn grounded_model_y(y: f32, simulated: f32, drawn: f32, sink: f32) -> f32 {
    // A deliberately elevated feature (including anything afloat) keeps its anchor.
    if (y - simulated).abs() <= 1.0 { y + drawn - simulated - sink } else { y }
}

/// A feature's 3D stand-in: the models it may be (one per instance, by its place), each
/// stretched to the painting's size, and the painting's colours (linear).
#[derive(Clone, Debug)]
struct StandIn {
    models: Vec<(FoliageModelId, [f32; 3])>,
    leaf: [f32; 3],
    bark: [f32; 3],
    /// Each model's height in the world (for the wind), and its greater size, height or width
    /// (for its size on the screen).
    heights: Vec<f32>,
    sizes: Vec<f32>,
    /// A plant on the ground (a bush, a fern): taken over only as close as with medium effects
    /// (`FoliageView::close_flat`): from further its few leaves would be flat green squares.
    flat: bool,
    /// Lying on the ground, following its slope (a Mars plant's tendrils).
    lies: bool,
    /// Rock of some kind: set on the slope it stands on (each model's reach round its foot, see
    /// `rock_seat`).
    seated: bool,
    reaches: Vec<f32>,
    /// A plant: each one a little different (see `Variety`).
    varied: bool,
    /// A crystal's light within (linear), see `FoliageInstance::light`.
    light: [f32; 3],
    /// Its rock as dark as painted, not lifted towards the ground's light as dark rock otherwise
    /// is (an energy node's mound: TA painted it darker than the ground round it).
    dark: bool,
    /// A vehicle's: turned as painted (its front, the end its glass is at), and its foot this far
    /// (east, south) from the feature's place.
    yaw: Option<f32>,
    offset: [f32; 2],
    /// Laid along its painting's lines (pipework, a row of barriers): its models, in place of
    /// `models`' one.
    parts: Vec<Part>,
    /// How much snow lies on its leaves (TA Reborn's own snowy firs, see `own_stand_in`).
    snow: f32,
}

/// One model of a stand-in laid along its painting's lines (a length of pipe, a joint, a
/// barrier): where its middle lies from the feature's place (east, south), the way it runs (a
/// unit vector: its model's x is turned to it), how far it reaches either way of its middle along
/// that way (it is set on the ground at both ends: on a slope it follows it), and how it is
/// stretched.
#[derive(Clone, Debug)]
struct Part {
    model: FoliageModelId,
    at: [f32; 2],
    way: [f32; 2],
    reach: f32,
    scale: [f32; 3],
}

/// What kind of 3D model stands in for a feature's painting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Broadleaf,
    Conifer,
    /// A lush world's wide jungle giant, and its tall narrow trees.
    JungleCanopy,
    JungleTall,
    /// A swollen root mass with stems and pads of leaves (TA's Oolok trees).
    Tuber,
    /// An archipelago's branching trees with rosettes of big leaves.
    Tropical,
    DeadBroadleaf,
    DeadConifer,
    DeadJungle,
    DeadTall,
    DeadTuber,
    DeadTropical,
    Bush,
    /// Ground plants: a rosette of big leaves.
    Fern,
    Boulder,
    /// Tall standing rocks: lava worlds' spires, and narrower pillars.
    Spire,
    Pillar,
    /// Wet deserts' clusters of banded spires, rounded at their tops.
    Hoodoos,
    /// Slate worlds' fog strata: clusters of tall blades of rock.
    Shards,
    /// Crystal worlds' spires: crystals over a plate of crystal.
    Crystal,
    /// Ice worlds' chunks lying about.
    Ice,
    /// Acid worlds' gasbags, lava worlds' energy nodes, Mars plants.
    Gasbag,
    Node,
    Tendrils,
    /// Urban worlds' cars and trucks (painted from above, lying until their models take over).
    Car,
    Truck,
    /// Metal worlds' pipework and urban worlds' road barriers (painted from above, lying until
    /// the pieces laid along their lines take over).
    Pipe,
    Barrier,
    /// Urban worlds' transit nodes: a squat dome (painted from above, lying until it takes over).
    Dome,
    /// Their microwave relays: cabinets and a dish on a pad, turned as painted.
    Relay,
    /// Their climatic optimizers: stacked drums under a round head, a lamp in its face.
    Optimizer,
    /// What lies scattered (painted from above, lying until its pieces take over): desert ruins'
    /// masonry, urban ruins' broken pillars, rubble.
    Ruin,
    Stumps,
    Rubble,
}

impl Kind {
    /// The library's models of this kind are named `<prefix>...`.
    fn prefix(self) -> &'static str {
        match self {
            Kind::Broadleaf => "broadleaf_",
            Kind::Conifer => "conifer_",
            Kind::JungleCanopy => "jungle_canopy_",
            Kind::JungleTall => "jungle_tall_",
            Kind::Tuber => "tuber_",
            Kind::Tropical => "tropical_",
            Kind::DeadBroadleaf => "dead_broadleaf_",
            Kind::DeadConifer => "dead_conifer_",
            Kind::DeadJungle => "dead_jungle_",
            Kind::DeadTall => "dead_tall_",
            Kind::DeadTuber => "dead_tuber_",
            Kind::DeadTropical => "dead_tropical_",
            Kind::Bush => "bush_",
            Kind::Fern => "fern_",
            Kind::Boulder => "boulder_",
            Kind::Spire => "spire_",
            Kind::Pillar => "pillar_",
            Kind::Hoodoos => "hoodoos_",
            Kind::Shards => "shards_",
            Kind::Crystal => "crystal_",
            Kind::Ice => "ice_",
            Kind::Gasbag => "gasbag_",
            Kind::Node => "node_",
            Kind::Tendrils => "tendrils_",
            Kind::Car => "car_",
            Kind::Truck => "truck_",
            Kind::Pipe => "pipe_",
            Kind::Barrier => "barrier_",
            Kind::Dome => "transit_",
            Kind::Relay => "relay_",
            Kind::Optimizer => "optimizer_",
            Kind::Ruin => "block_",
            Kind::Stumps => "stump_",
            Kind::Rubble => "boulder_",
        }
    }

    /// Painted from above, lying in the perspective camera until its 3D stand-in takes over.
    pub(crate) fn painted_lying(self) -> bool {
        self.vehicle() || self.laid() || matches!(self, Kind::Dome | Kind::Relay)
    }

    /// Laid along its painting's lines, or set where its patches lie (see `Part`).
    fn laid(self) -> bool {
        matches!(self, Kind::Pipe | Kind::Barrier | Kind::Ruin | Kind::Stumps | Kind::Rubble)
    }

    /// A vehicle: stretched to its painting's length and breadth, turned as painted.
    fn vehicle(self) -> bool {
        matches!(self, Kind::Car | Kind::Truck)
    }

    /// Rock of some kind (its colour is all of its painting's; it stands as tall as painted).
    fn rock(self) -> bool {
        matches!(self, Kind::Boulder | Kind::Spire | Kind::Pillar | Kind::Hoodoos | Kind::Shards | Kind::Crystal | Kind::Ice | Kind::Node)
    }

    /// Stands as tall as painted (not lowered as trees are) and takes its colour from all of its
    /// painting.
    fn whole(self) -> bool {
        self.rock() || self.vehicle() || self.laid() || matches!(self, Kind::Gasbag | Kind::Tendrils | Kind::Dome | Kind::Relay | Kind::Optimizer)
    }

    /// What a living tree of this kind is once burnt.
    fn burnt(self) -> Option<Kind> {
        Some(match self {
            Kind::Broadleaf => Kind::DeadBroadleaf,
            Kind::Conifer => Kind::DeadConifer,
            Kind::JungleCanopy => Kind::DeadJungle,
            Kind::JungleTall => Kind::DeadTall,
            Kind::Tuber => Kind::DeadTuber,
            Kind::Tropical => Kind::DeadTropical,
            // (A burnt Mars plant keeps its shape, charred: its painting's colours.)
            Kind::Tendrils => Kind::Tendrils,
            _ => return None,
        })
    }

    /// Painted flat on the ground (bushes, ground plants): such a model keeps its own
    /// proportions, as wide as the painting.
    fn flat(self) -> bool {
        matches!(self, Kind::Bush | Kind::Fern)
    }

    /// Its shape must stay round (a gasbag's bag): it is scaled the same every way, and of its
    /// variants those whose height fits its painting are taken.
    fn round(self) -> bool {
        matches!(self, Kind::Gasbag)
    }

    fn dead(self) -> bool {
        matches!(self, Kind::DeadBroadleaf | Kind::DeadConifer | Kind::DeadJungle | Kind::DeadTall | Kind::DeadTuber | Kind::DeadTropical)
    }
}

/// What a feature's painting (its first frame) shows: how wide it is and how far it reaches above
/// its anchor (its foot on the ground), how narrow its top is, and its colours.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Painting {
    pub width: f32,
    pub above: f32,
    /// How far it reaches below its anchor.
    pub below: f32,
    /// The painted width a sixth of the way down from its top, over the widest.
    pub top_width: f32,
    /// Mean colours (linear): of the brighter part of its upper three quarters (a tree's crown:
    /// the three fifths of its pixels most lit, its lit side without the shade TA painted in), of
    /// its lowest rows (the trunk at its foot) and of all of it.
    pub crown_lit: [f32; 3],
    pub foot: [f32; 3],
    pub all: [f32; 3],
    /// And of the brighter part of all of it (as `crown_lit`).
    pub all_lit: [f32; 3],
    /// How many pixels its lowest rows have.
    pub foot_pixels: usize,
    /// Of its top third (a crystal spire's crystals over their plate).
    pub top: [f32; 3],
    /// Of its blue pixels, if it has a few (a crystal's light: an energy node's glowing tips).
    pub accent: Option<[f32; 3]>,
    /// Of the warm pixels of its upper three fifths, if it has enough (an energy node's tubes).
    pub warm: Option<[f32; 3]>,
    /// Of its pixels neither warm nor blue, if it has enough (the rock an energy node's tubes
    /// stand out of).
    pub rock: Option<[f32; 3]>,
    /// Of the brighter three fifths of its pixels not blue (a vehicle's paint, not its glass).
    pub body: [f32; 3],
    /// Its shape (painting pixels, x east and y south): the middle of its pixels from its anchor,
    /// the way its longest runs (a unit vector), how far it reaches along that way and across,
    /// and the middle of its blue pixels from its anchor (a vehicle's glass), if it has a few.
    pub centre: [f32; 2],
    pub axis: [f32; 2],
    pub length: f32,
    pub breadth: f32,
    pub glass_at: Option<[f32; 2]>,
    /// Of its warm pixels anywhere, if it has a few: where their middle is from its anchor and
    /// their colour (linear; a relay's roof).
    pub roof: Option<([f32; 2], [f32; 3])>,
}

impl Painting {
    pub fn of(frame: &GafFrame, palette: &Palette) -> Option<Painting> {
        let img = &frame.image;
        let (w, h) = (img.width as usize, img.height as usize);
        let opaque = |x: usize, y: usize| img.opaque[y * w + x];
        let rows: Vec<(usize, usize, usize)> = (0..h)
            .filter_map(|y| {
                let xs: Vec<usize> = (0..w).filter(|&x| opaque(x, y)).collect();
                Some((y, *xs.first()?, *xs.last()?))
            })
            .collect();
        let (&(top, _, _), &(bottom, _, _)) = (rows.first()?, rows.last()?);
        let left = rows.iter().map(|r| r.1).min()?;
        let right = rows.iter().map(|r| r.2).max()?;
        let widest = rows.iter().map(|r| r.2 - r.1 + 1).max()? as f32;
        let sixth = top + (bottom - top) / 6;
        let top_width = rows.iter().find(|r| r.0 >= sixth).map_or(widest, |r| (r.2 - r.1 + 1) as f32) / widest;
        let lut = ta_render::util::srgb_to_linear_lut();
        let mut sums = [[0.0f32; 4]; 3];
        // (Of the top third, the blue pixels and the warm ones.)
        let mut more = [[0.0f32; 4]; 3];
        let mut crown_pixels: Vec<[f32; 3]> = Vec::new();
        // (Every pixel's place, and the blue ones' and the rest's colours.)
        let mut places: Vec<[f32; 2]> = Vec::new();
        let mut blue_places: Vec<[f32; 2]> = Vec::new();
        let mut warm_places: Vec<([f32; 2], [f32; 3])> = Vec::new();
        let mut body_pixels: Vec<[f32; 3]> = Vec::new();
        let mut all_pixels: Vec<[f32; 3]> = Vec::new();
        let mut rock_pixels: Vec<[f32; 3]> = Vec::new();
        let span = (bottom - top + 1) as f32;
        for y in top..=bottom {
            let t = (y - top) as f32 / span;
            for x in 0..w {
                if !opaque(x, y) {
                    continue;
                }
                let c = palette.rgba(img.pixels[y * w + x]);
                let lin = [lut[c[0] as usize], lut[c[1] as usize], lut[c[2] as usize], 1.0];
                let [r, g, b] = [c[0] as i32, c[1] as i32, c[2] as i32];
                let blue = b > r + 25 && b > g + 10 && r + g + b > 180;
                let warm = t < 0.6 && r >= b + 15 && g >= b + 15;
                for (k, take) in [t < 0.35, blue, warm].into_iter().enumerate() {
                    if take {
                        for i in 0..4 {
                            more[k][i] += lin[i];
                        }
                    }
                }
                places.push([x as f32 + 0.5, y as f32 + 0.5]);
                if r >= b + 15 && g >= b + 15 {
                    warm_places.push(([x as f32 + 0.5, y as f32 + 0.5], [lin[0], lin[1], lin[2]]));
                } else if !blue {
                    rock_pixels.push([lin[0], lin[1], lin[2]]);
                }
                if blue {
                    blue_places.push([x as f32 + 0.5, y as f32 + 0.5]);
                } else {
                    body_pixels.push([lin[0], lin[1], lin[2]]);
                }
                if t < 0.75 {
                    crown_pixels.push([lin[0], lin[1], lin[2]]);
                }
                all_pixels.push([lin[0], lin[1], lin[2]]);
                for (k, take) in [t < 0.75, t > 0.88, true].into_iter().enumerate() {
                    if take {
                        for i in 0..4 {
                            sums[k][i] += lin[i];
                        }
                    }
                }
            }
        }
        let mean = |s: [f32; 4]| [s[0], s[1], s[2]].map(|v| v / s[3].max(1.0));
        let lum = |c: &[f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
        crown_pixels.sort_by(|a, b| lum(b).total_cmp(&lum(a)));
        let lit = &crown_pixels[..(crown_pixels.len() * 3 / 5).max(1).min(crown_pixels.len())];
        let crown_lit = if lit.is_empty() { mean(sums[0]) } else { [0, 1, 2].map(|i| lit.iter().map(|p| p[i]).sum::<f32>() / lit.len() as f32) };
        all_pixels.sort_by(|a, b| lum(b).total_cmp(&lum(a)));
        let lit_all = &all_pixels[..(all_pixels.len() * 3 / 5).max(1).min(all_pixels.len())];
        let all_lit = if lit_all.is_empty() { mean(sums[2]) } else { [0, 1, 2].map(|i| lit_all.iter().map(|p| p[i]).sum::<f32>() / lit_all.len() as f32) };
        let oy = frame.origin_y as f32;
        let ox = frame.origin_x as f32;
        // Its shape: the middle, the way its pixels spread most (the covariance's main
        // eigenvector), and how far they reach along it and across (all but the farthest
        // twentieth either way).
        let n = places.len().max(1) as f32;
        let (mx, my) = (places.iter().map(|p| p[0]).sum::<f32>() / n, places.iter().map(|p| p[1]).sum::<f32>() / n);
        let (mut sxx, mut sxy, mut syy) = (0.0f32, 0.0f32, 0.0f32);
        for p in &places {
            let (dx, dy) = (p[0] - mx, p[1] - my);
            sxx += dx * dx;
            sxy += dx * dy;
            syy += dy * dy;
        }
        let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
        let axis = [theta.cos(), theta.sin()];
        let reach = |dir: [f32; 2]| {
            let mut d: Vec<f32> = places.iter().map(|p| (p[0] - mx) * dir[0] + (p[1] - my) * dir[1]).collect();
            d.sort_by(f32::total_cmp);
            if d.is_empty() { 0.0 } else { d[d.len() * 19 / 20] - d[d.len() / 20] + 1.0 }
        };
        let glass_at = (blue_places.len() >= 6).then(|| {
            let k = blue_places.len() as f32;
            [blue_places.iter().map(|p| p[0]).sum::<f32>() / k - ox, blue_places.iter().map(|p| p[1]).sum::<f32>() / k - oy]
        });
        body_pixels.sort_by(|a, b| lum(b).total_cmp(&lum(a)));
        let body_lit = &body_pixels[..(body_pixels.len() * 3 / 5).max(1).min(body_pixels.len())];
        let body = if body_lit.is_empty() { all_lit } else { [0, 1, 2].map(|i| body_lit.iter().map(|p| p[i]).sum::<f32>() / body_lit.len() as f32) };
        let rock = (rock_pixels.len() >= 50).then(|| [0, 1, 2].map(|i| rock_pixels.iter().map(|p| p[i]).sum::<f32>() / rock_pixels.len() as f32));
        Some(Painting {
            width: (right - left + 1) as f32,
            above: oy - top as f32,
            below: (bottom as f32 + 1.0 - oy).max(0.0),
            top_width,
            crown_lit,
            foot: mean(sums[1]),
            all: mean(sums[2]),
            all_lit,
            foot_pixels: sums[1][3] as usize,
            top: mean(more[0]),
            accent: (more[1][3] >= 12.0).then(|| mean(more[1])),
            warm: (more[2][3] >= 50.0).then(|| mean(more[2])),
            rock,
            body,
            centre: [mx - ox, my - oy],
            axis,
            length: reach(axis),
            breadth: reach([-axis[1], axis[0]]),
            glass_at,
            roof: (warm_places.len() >= 12).then(|| {
                let k = warm_places.len() as f32;
                let sum = warm_places.iter().fold(([0.0f32; 2], [0.0f32; 3]), |(p, c), (q, d)| ([p[0] + q[0], p[1] + q[1]], [c[0] + d[0], c[1] + d[1], c[2] + d[2]]));
                ([sum.0[0] / k - ox, sum.0[1] / k - oy], sum.1.map(|v| v / k))
            }),
        })
    }
}

/// The kind of 3D model that stands in for `def`'s painting, if any. Trees by their world and
/// shape (on green worlds the narrow ones are conifers; on lush ones the wide ones are jungle
/// giants); a burnt tree is the burnt form of the tree that burns into it (`burns_from`), or of
/// its own world and shape; shrubs are bushes, a lush or tropical world's foliage ground plants
/// (not its creeping vines); standing rocks are boulders. The rest stays painted.
pub(crate) fn kind_of(def: &FeatureDef, p: &Painting, burns_from: Option<Kind>) -> Option<Kind> {
    if def.is_metal_deposit() || !def.object.is_empty()
        || def.category.eq_ignore_ascii_case("scars") || def.description.eq_ignore_ascii_case("Damaged Terrain")
    {
        return None;
    }
    // Urban worlds' cars and trucks (not the scars they leave, nor anything else of theirs).
    if def.height > 4.0 && def.category.eq_ignore_ascii_case("cars") && def.description.eq_ignore_ascii_case("car") {
        return Some(Kind::Car);
    }
    if def.height > 4.0 && def.category.eq_ignore_ascii_case("trucks") && def.description.eq_ignore_ascii_case("truck") {
        return Some(Kind::Truck);
    }
    // Metal worlds' pipes and urban worlds' barriers (not the holes and scars they leave).
    if def.height > 4.0 && def.category.eq_ignore_ascii_case("pipes") && def.description.eq_ignore_ascii_case("pipe") {
        return Some(Kind::Pipe);
    }
    if def.height > 4.0 && def.category.eq_ignore_ascii_case("barriers") && def.description.eq_ignore_ascii_case("barrier") {
        return Some(Kind::Barrier);
    }
    if def.height > 4.0 && def.category.eq_ignore_ascii_case("machines") && def.description.eq_ignore_ascii_case("transit node") {
        return Some(Kind::Dome);
    }
    if def.height > 4.0 && def.category.eq_ignore_ascii_case("machines") && def.description.eq_ignore_ascii_case("microwave relay") {
        return Some(Kind::Relay);
    }
    if def.height > 4.0 && def.category.eq_ignore_ascii_case("machines") && def.description.eq_ignore_ascii_case("climatic optimizer") {
        return Some(Kind::Optimizer);
    }
    // What TA scattered and painted from above (a ruin's masonry, an urban ruin's broken
    // pillars, rubble), lying (not a standing wreck of a building).
    let lying = || ta_render::sprites::painted_lying(def.height as f32, def.footprint_x.max(def.footprint_z), p.above, p.below);
    if def.height > 4.0 && def.category.eq_ignore_ascii_case("ruin") && def.description.eq_ignore_ascii_case("ruin") && lying() {
        return Some(Kind::Ruin);
    }
    if def.height > 4.0 && def.category.eq_ignore_ascii_case("buildings") && def.seq_name.to_ascii_lowercase().starts_with("u-gird") && lying() {
        return Some(Kind::Stumps);
    }
    if def.height > 4.0 && def.category.eq_ignore_ascii_case("rocks") && def.description.eq_ignore_ascii_case("rubble") && lying() {
        return Some(Kind::Rubble);
    }
    if let Some(k) = burns_from {
        return k.burnt();
    }
    let text = format!("{} {} {}", def.description, def.category, def.name).to_ascii_lowercase();
    let world = def.world.to_ascii_lowercase();
    let has = |w: &str| text.contains(w);
    let burnt = ["burnt", "burned", "dead", "crisp", "charred"].iter().any(|w| has(w));
    if has("shrub") {
        return (!burnt).then_some(Kind::Bush);
    }
    if has("tree") || has("palm") {
        if def.height <= 4.0 {
            return None;
        }
        let aspect = p.width / p.above.max(1.0);
        let kind = if has("oolok") || has("tuber") {
            Kind::Tuber
        } else if world.contains("archipelago") || has("palm") {
            Kind::Tropical
        } else if world.contains("lush") {
            if aspect > 0.9 { Kind::JungleCanopy } else { Kind::JungleTall }
        } else if aspect < 0.66 && p.top_width < 0.7 {
            Kind::Conifer
        } else {
            Kind::Broadleaf
        };
        return if burnt { kind.burnt() } else { Some(kind) };
    }
    if has("foliage") && !has("vine") && !burnt && (world.contains("lush") || world.contains("archipelago")) {
        return Some(Kind::Fern);
    }
    if has("spire") || has("strata") {
        return (def.height > 4.0).then_some(if world.contains("crystal") || has("crystal") {
            Kind::Crystal
        } else if world.contains("slate") || has("strata") {
            Kind::Shards
        } else {
            Kind::Spire
        });
    }
    if has("ice") && (has("chunk") || has("rock")) {
        return Some(Kind::Ice);
    }
    if has("gasbag") || has("gasplant") {
        return (def.height > 4.0).then_some(Kind::Gasbag);
    }
    if world.contains("lava") && has("node") {
        return (def.height > 4.0).then_some(Kind::Node);
    }
    // (Not the reclaim smudge a Mars plant leaves: "damaged terrain", its name the plant's.)
    if world.contains("mars") && has("plant") && !has("gone") && !has("smudge") {
        return Some(Kind::Tendrils);
    }
    // Rocks, standing or painted flat; not the rubble left where one was destroyed. A wet
    // desert's standing rocks are clusters of spires (painted side by side, as wide as tall:
    // not boulders), but for its low ones.
    if ["rock", "boulder", "stone"].iter().any(|w| has(w)) && !["dead", "gone", "rubble"].iter().any(|w| has(w)) {
        return Some(if def.height > 4.0 && world.contains("wetdesert") && p.above > p.width * 0.6 {
            Kind::Hoodoos
        } else if def.height > 4.0 && p.above > p.width * 1.3 {
            Kind::Pillar
        } else {
            Kind::Boulder
        });
    }
    None
}

/// Whether a standing feature was painted lying, from TA's camera high above (a car, a pipe, a
/// barrier, a ruin, a low machine): the perspective camera lays its painting on the ground (see
/// `ta_render::sprites::painted_lying`). Whatever a vehicle or pieces laid along its lines stand
/// in for always was (one of TA's barriers is 80 high); trees, rocks and the like never.
pub(crate) fn painted_lying(def: &FeatureDef, p: &Painting) -> bool {
    match kind_of(def, p, None) {
        Some(k) => k.painted_lying(),
        None => ta_render::sprites::painted_lying(def.height as f32, def.footprint_x.max(def.footprint_z), p.above, p.below),
    }
}

/// How much lower the trees stand than their paintings imply. Seen from TA's height a crown
/// hides most of its trunk, so a painted tree is some 110 units tall (three Commanders) with a
/// long bare trunk; from a unit's eyes that dwarfs the battle. At this they stand about twice a
/// Commander's height.
const TREE_HEIGHT: f32 = 0.72;

/// How far a model must be stretched (across, up, across) to stand where a painting `p` is drawn:
/// as wide as it, and, as TA's view would show it, as tall above its anchor.
fn fit(library: &FoliageLibrary, model: FoliageModelId, p: &Painting) -> [f32; 3] {
    let m = library.get(model);
    let across = (p.width / m.width.max(1.0)).clamp(0.3, 4.0);
    [across, fit_up(library, model, p, across), across]
}

/// How far a model `across` as wide must be stretched up to stand as tall above its anchor as
/// the painting `p`, as TA's view would show it.
fn fit_up(library: &FoliageLibrary, model: FoliageModelId, p: &Painting, across: f32) -> f32 {
    let m = library.get(model);
    let (mut lo, mut hi) = (across * 0.55, across * 1.9);
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if m.ta_height(across, mid) < p.above {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

/// How the 3D trees and rocks are seen this frame: from where, how close the camera is to show
/// them at all (0 only TA's paintings, 1 the models wherever they are big enough), and how many
/// pixels on the screen a world unit is one unit away (at `d` units, `scale / d`).
#[derive(Clone, Copy, Debug, Default)]
pub struct FoliageView {
    pub eye: Vec3,
    pub close: f32,
    /// As `close`, for the plants on the ground (see `StandIn::flat`).
    pub close_flat: f32,
    pub scale: f32,
    /// The view's clip planes (x, y, z, w: inside where `xyz . p + w >= 0`), to skip the models
    /// out of sight.
    pub planes: [glam::Vec4; 6],
    /// The sea level, when the water mirrors the world (a tree out of sight may show in it).
    pub mirror: Option<f32>,
    /// Direction towards this frame's sun or moon, when shadows are enabled.
    pub light: Option<Vec3>,
}

impl FoliageView {
    /// Whether a sphere (`centre`, `radius`) may be in sight.
    fn sees(&self, centre: Vec3, radius: f32) -> bool {
        self.planes.iter().all(|p| p.truncate().dot(centre) + p.w >= -radius)
    }

    /// Whether a model (`centre`, `radius`, `height` tall) may show: itself, its shadow, or its
    /// reflection in the water.
    fn shows(&self, centre: Vec3, radius: f32, height: f32) -> bool {
        if self.sees(centre, radius) || self.mirror.is_some_and(|sea| self.sees(Vec3::new(centre.x, 2.0 * sea - centre.y, centre.z), radius)) {
            return true;
        }
        self.light.is_some_and(|light| {
            // A long dawn/dusk shadow can enter the view while its tree is far outside it.
            // Test the swept sphere along the shadow, conservatively down to height zero.
            let end = centre - ta_render::camera::shadow_caster_offset(light, centre.y + height * 0.5);
            self.planes.iter().all(|p| p.truncate().dot(centre).max(p.truncate().dot(end)) + p.w >= -radius)
        })
    }
}

/// The clip planes of a view-projection matrix (depth 0..1), normalised.
pub fn clip_planes(m: glam::Mat4) -> [glam::Vec4; 6] {
    let r = [m.row(0), m.row(1), m.row(2), m.row(3)];
    [r[3] + r[0], r[3] - r[0], r[3] + r[1], r[3] - r[1], r[2], r[3] - r[2]].map(|p| p / p.truncate().length().max(1e-9))
}

/// On-screen heights (pixels) above which a model is drawn at its finer levels of detail, and
/// between which it takes over from the painting.
const LOD0_PIXELS: f32 = 240.0;
const LOD1_PIXELS: f32 = 70.0;
/// Blend adjacent meshes across a band of screen sizes rather than changing a tree's whole
/// crown on one frame as the camera passes it. Outside the bands only one mesh is drawn.
fn foliage_detail(pixels: f32) -> f32 {
    2.0 - smoothstep(LOD1_PIXELS * 0.9, LOD1_PIXELS * 1.1, pixels)
        - smoothstep(LOD0_PIXELS * 0.9, LOD0_PIXELS * 1.1, pixels)
}
const MODEL_PIXELS: (f32, f32) = (10.0, 16.0);
/// Each model takes over from its painting at its own point of the camera's way down
/// (`FoliageView::close`) and of its size on the screen, over this much of the way: the forest
/// turns over tree by tree, never all of it half drawn at once.
const HANDOVER_SPAN: f32 = 0.06;

pub struct FeatureView {
    visuals: HashMap<usize, FeatureVisual>,
    matrices: Vec<Vec<Mat4>>,
    decals: Vec<SpriteInstance>,
    billboards: Vec<SpriteInstance>,
    /// The features painted lying, laid on the ground (perspective camera).
    lying: Vec<SpriteInstance>,
    /// Crystal clusters for the metal deposits (see ore.rs).
    ore_models: Vec<ModelId>,
    /// The modern camera is on: the deposits get their crystals.
    pub modern: bool,
    /// Features made during the battle (wrecks, heaps, burnt trees) and when (game seconds):
    /// they come down glowing and cool to soot.
    born: rustc_hash::FxHashMap<usize, f32>,
    /// This frame's 3D trees and rocks: model, level of detail, instance.
    foliage: Vec<(FoliageModelId, f32, FoliageInstance)>,
    /// This frame's burning trees drawn in 3D: feature index and the middle of the crown.
    flames: Vec<(usize, Vec3)>,
    /// Each model's crown (centre and radius, in its own units).
    crowns: Vec<[f32; 4]>,
    /// Where the parts of each feature laid along its painting's lines lie on the ground (see
    /// `seat_part`), by feature (and its type), once it has been seen.
    seats: rustc_hash::FxHashMap<usize, Seats>,
    /// Highest loaded feature/model above the maximum 255-unit terrain, including plant
    /// variety. Used for reflection and shadow source selection, not just visible ground.
    source_height: f32,
}

/// A feature's type and where each of its parts lies on the ground (its foot's height and the
/// slope it follows, see `seat_part`).
type Seats = (usize, Vec<(f32, [f32; 2])>);

/// Seconds a fresh wreck takes to cool from glowing embers to soot.
const COOLING: f32 = 24.0;

impl FeatureView {
    /// Loads sprites/models for every feature type present on the map.
    pub fn new(vfs: &Vfs, renderer: &mut Renderer, sim: &Sim) -> FeatureView {
        let started = std::time::Instant::now();
        let palette = &sim.data.palette;
        let mut gaf_cache: HashMap<String, Option<Gaf>> = HashMap::new();
        let mut visuals: HashMap<usize, FeatureVisual> = HashMap::new();
        let mut used: Vec<usize> = sim.features.iter().map(|f| f.def).collect();
        // Wrecks and heaps of every unit type, so deaths never need to load assets.
        for t in &sim.types.types {
            if let Some(c) = sim.data.feature_id(&t.def.corpse) {
                used.push(c);
                if let Some(h) = sim.data.feature_id(&sim.data.features[c].feature_dead) {
                    used.push(h);
                }
            }
        }
        // Everything those can turn into when destroyed or reclaimed (burnt trees, heaps,
        // reclaim smudges), so runtime-created features always have visuals.
        used.sort_unstable();
        used.dedup();
        let mut i = 0;
        while i < used.len() {
            let def = &sim.data.features[used[i]];
            for next in [&def.feature_dead, &def.feature_reclamate, &def.feature_burnt] {
                if let Some(n) = sim.data.feature_id(next) && !used.contains(&n) {
                    used.push(n);
                }
            }
            i += 1;
        }
        used.sort_unstable();
        used.dedup();
        let off = std::env::var("TA_FOLIAGE").is_ok_and(|v| v == "0");
        let mut paintings: HashMap<usize, Painting> = HashMap::new();
        let mut strokes: HashMap<usize, crate::feature_strokes::Strokes> = HashMap::new();
        let mut patches: HashMap<usize, Vec<crate::feature_strokes::Blob>> = HashMap::new();
        // (With HD surfaces the standing paintings' palette static is calmed, as the ground's.)
        renderer.sprites.atlas.calm_paintings = renderer.lighting.hd_surfaces > 0.0;
        for def_id in used {
            let def = &sim.data.features[def_id];
            let mut v = FeatureVisual {
                upright: def.height > 4.0 && !def.is_metal_deposit(),
                sways: def.height > 4.0 && !def.is_metal_deposit() && def.flamable,
                ore: if def.is_metal_deposit() { (def.metal / 40.0).sqrt().clamp(0.35, 1.0) as f32 } else { 0.0 },
                translucent: def.anim_trans,
                light: def.reborn_light,
                fireworks: def.reborn_fireworks,
                ..Default::default()
            };
            if !def.object.is_empty() {
                if let Some(models) = renderer.models.as_mut() {
                    let name = def.object.to_ascii_lowercase();
                    v.model = vfs
                        .read_opt(&format!("objects3d/{name}.3do"))
                        .and_then(|b| ta_formats::model3do::Model3do::parse(&b).ok())
                        .map(|mut m| {
                            // A wreck's pads (an air plant's runway) clear of the ground.
                            ta_render::model::lift_ground_pads(&mut m);
                            let id = models.library.add(&name, &m, &models.textures, palette);
                            let surfaces = crate::scene::PieceSurface::of_render(&models.library, id);
                            v.ground_sink = model_ground_sink(&models.library.info(id).rest_pose(), &surfaces);
                            id
                        });
                }
            } else if !def.filename.is_empty() {
                let key = def.filename.to_ascii_lowercase();
                let gaf = gaf_cache
                    .entry(key.clone())
                    .or_insert_with(|| vfs.read_opt(&format!("anims/{key}.gaf")).and_then(|b| Gaf::parse(&b).ok()));
                if let Some(gaf) = gaf {
                    // Upright ones are alpha tested: outlined for smooth edges up close (and so
                    // are those laid down). Flat ones fade at their edges into the ground, but
                    // for the plants (those that burn: fronds, Mars plants), painted leaf by leaf.
                    let outlined = v.upright && !v.translucent;
                    let atlas = &mut renderer.sprites.atlas;
                    // (Calmed as the atlas is told: the paintings, not their flames, which burn
                    // for a moment.)
                    let calm = atlas.calm_paintings;
                    let mut add = |f: &ta_formats::gaf::GafFrame, calmed: bool| {
                        atlas.calm_paintings = calmed;
                        match (outlined, v.translucent) {
                            (true, _) => atlas.add_gaf_outlined(f, palette),
                            (false, false) => atlas.add_gaf_decal(f, palette, def.flamable),
                            (false, true) => atlas.add_gaf(f, palette, false),
                        }
                    };
                    if let Some(entry) = gaf.entry(&def.seq_name) {
                        v.frames = entry.frames.iter().filter_map(|f| add(f, calm)).collect();
                    }
                    if !def.seq_name_burn.is_empty() && let Some(entry) = gaf.entry(&def.seq_name_burn) {
                        v.burn_frames = entry.frames.iter().filter_map(|f| add(f, false)).collect();
                    }
                    atlas.calm_paintings = calm;
                    if !def.seq_name_shadow.is_empty() && let Some(entry) = gaf.entry(&def.seq_name_shadow) {
                        v.shadow = entry.frames.first().and_then(|f| renderer.sprites.atlas.add_gaf(f, palette, true));
                    }
                    if let Some(p) = gaf.entry(&def.seq_name).and_then(|e| e.frames.first()).and_then(|f| Painting::of(f, palette)) {
                        // (Painted lying, if no 3D stand-in takes its place up close.)
                        v.lies = v.upright && painted_lying(def, &p);
                        if !off {
                            // (The lines of what is laid along them.)
                            if kind_of(def, &p, None).is_some_and(Kind::laid)
                                && let Some(f) = gaf.entry(&def.seq_name).and_then(|e| e.frames.first())
                            {
                                let (w, h, origin) = (f.image.width as usize, f.image.height as usize, [f.origin_x as f32, f.origin_y as f32]);
                                strokes.insert(def_id, crate::feature_strokes::trace(&f.image.opaque, w, h, origin));
                                // (A ruin's blocks and rubble's stones touch each other in the
                                // painting, their shaded sides between them: their lit tops,
                                // lighter than the painting's middling pixel, are each one.)
                                let lit = if kind_of(def, &p, None).is_some_and(|k| matches!(k, Kind::Ruin | Kind::Rubble)) {
                                    let light = |i: usize| {
                                        let c = palette.rgba(f.image.pixels[i]);
                                        c[0] as u32 * 54 + c[1] as u32 * 183 + c[2] as u32 * 19
                                    };
                                    let mut lights: Vec<u32> = (0..w * h).filter(|&i| f.image.opaque[i]).map(light).collect();
                                    lights.sort_unstable();
                                    let middle = lights.get(lights.len() / 2).copied().unwrap_or(0);
                                    (0..w * h).map(|i| f.image.opaque[i] && light(i) > middle).collect()
                                } else {
                                    f.image.opaque.clone()
                                };
                                patches.insert(def_id, crate::feature_strokes::blobs(&lit, w, h, origin, 5));
                            }
                            paintings.insert(def_id, p);
                        }
                    }
                }
            }
            visuals.insert(def_id, v);
        }
        // The 3D stand-ins: each kind by its painting, a burnt tree's by the tree that burns into
        // it.
        let mut stand_ins = 0;
        if let Some(library) = renderer.foliage.as_ref().map(|f| &f.library) {
            let mut burns_from: HashMap<usize, Kind> = HashMap::new();
            for (&id, p) in &paintings {
                let def = &sim.data.features[id];
                if let (Some(k), Some(burnt)) = (kind_of(def, p, None), sim.data.feature_id(&def.feature_burnt)) {
                    burns_from.insert(burnt, k);
                }
            }
            for (&id, p) in &paintings {
                // TA Reborn's own: its own model, as it was made.
                let def = &sim.data.features[id];
                if !def.reborn_model.is_empty() {
                    if let Some(v) = visuals.get_mut(&id) {
                        v.stand_in = own_stand_in(library, &def.reborn_model);
                        stand_ins += v.stand_in.is_some() as usize;
                    }
                    continue;
                }
                let kind = kind_of(&sim.data.features[id], p, burns_from.get(&id).copied());
                if let (Some(kind), Some(v)) = (kind, visuals.get_mut(&id)) {
                    v.stand_in = match kind {
                        Kind::Relay => relay(library, p),
                        Kind::Ruin | Kind::Stumps | Kind::Rubble => patches.get(&id).and_then(|b| scattered(library, kind, p, b)),
                        Kind::Pipe => strokes.get(&id).and_then(|s| pipework(library, p, s)),
                        Kind::Barrier => strokes.get(&id).and_then(|s| barriers(library, p, s)),
                        k if k.vehicle() => vehicle(library, kind, p),
                        _ => stand_in(library, kind, p, v.upright),
                    };
                    stand_ins += v.stand_in.is_some() as usize;
                }
            }
        }
        log::info!("features: {} instances of {} types ({stand_ins} types with 3D stand-ins) in {:.0} ms", sim.features.len(), visuals.len(), started.elapsed().as_secs_f64() * 1000.0);
        let ore_models = match renderer.models.as_mut() {
            Some(models) if sim.features.iter().any(|f| sim.data.features[f.def].is_metal_deposit()) => {
                (0..crate::ore::VARIANTS).map(|i| models.library.add(&format!("(ore {i})"), &crate::ore::cluster(i, palette), &models.textures, palette)).collect()
            }
            _ => Vec::new(),
        };
        let crowns = renderer.foliage.as_ref().map_or_else(Vec::new, |f| f.library.models.iter().map(|m| m.crown).collect());
        let foliage_height = visuals.values().filter_map(|v| v.stand_in.as_ref()).flat_map(|s| s.heights.iter()).copied().fold(0.0f32, f32::max) * 1.2;
        let model_height = renderer.models.as_ref().map_or(0.0, |models| models.library.models.iter().map(|m| m.height).fold(0.0f32, f32::max));
        let source_height = 260.0 + foliage_height.max(model_height);
        FeatureView { visuals, matrices: Vec::new(), decals: Vec::new(), billboards: Vec::new(), lying: Vec::new(), ore_models, modern: false, born: Default::default(), foliage: Vec::new(), flames: Vec::new(), crowns, seats: Default::default(), source_height }
    }

    pub fn source_height(&self) -> f32 {
        self.source_height
    }

    /// A feature was made during the battle (`now` in game seconds).
    pub fn created(&mut self, index: usize, now: f32) {
        self.born.insert(index, now);
        if self.born.len() > 4096 {
            self.born.retain(|_, t| now - *t < COOLING);
        }
    }

    /// How a feature made during the battle looks now: embers and soot as the model shader's
    /// battle damage draws them (above 0.6 the embers glow), cooling to soot over [`COOLING`].
    fn heat(&self, index: usize, now: f32) -> f32 {
        self.born.get(&index).map_or(0.0, |&t| {
            let k = ((now - t) / COOLING).clamp(0.0, 1.0);
            0.5 + 0.5 * (1.0 - k).powf(1.5)
        })
    }

    /// Builds sprite instances for features inside the visible ground rectangle, and the 3D
    /// stand-ins of those near enough to `view` (handing over from the paintings as they grow).
    /// The wrecks in `waiting` are left out: what is left of their units is still coming to rest
    /// over them (a hull sinking, an aircraft falling; see `Effects::unsettled_wrecks`).
    pub fn update(&mut self, sim: &Sim, time: f32, view_min: Vec2, view_max: Vec2, view: &FoliageView, waiting: &[usize]) {
        self.decals.clear();
        self.billboards.clear();
        self.lying.clear();
        self.matrices.clear();
        self.foliage.clear();
        self.flames.clear();
        let margin = 96.0;
        let tick = sim.tick;
        // (Those in the squares round the view, in index order: a big map has tens of
        // thousands.)
        let near = features_in_view(sim, view_min, view_max, margin);
        for index in near {
            let index = index as usize;
            let f = &sim.features[index];
            if !f.alive || waiting.contains(&index) {
                continue;
            }
            let mut p = f.pos.to_f32();
            if p[0] < view_min.x - margin || p[0] > view_max.x + margin || p[2] < view_min.y - margin || p[2] > view_max.y + margin {
                continue;
            }
            p[1] += crate::scene::ground_offset(&sim.world, Vec3::from_array(p));
            let Some(v) = self.visuals.get(&f.def) else { continue };
            // The 3D stand-in, as far as the camera is close and it is big enough on the screen.
            let mut shown3d = 0.0;
            let h = (f.origin.0 as u32).wrapping_mul(73_856_093) ^ (f.origin.1 as u32).wrapping_mul(19_349_663);
            // A plant as it grows at its place (see `Variety`), its painting too.
            let variety = v.stand_in.as_ref().filter(|si| si.varied).map(|_| Variety::at(h));
            if let Some((si, close)) = v.stand_in.as_ref().map(|si| (si, if si.flat { view.close_flat } else { view.close })).filter(|&(_, close)| close > 0.0) {
                let pick = (h >> 4) as usize % si.models.len();
                let (model, scale) = si.models[pick];
                let height = si.heights[pick];
                let varied_height = height * variety.map_or(1.0, |v| v.up);
                let centre = Vec3::new(p[0] + si.offset[0], p[1] + varied_height * 0.5, p[2] + si.offset[1]);
                let d = centre.distance(view.eye).max(1.0);
                let pixels = si.sizes[pick] * view.scale / d;
                let at = 0.05 + 0.9 * ((h >> 16) & 255) as f32 / 255.0;
                let size_at = MODEL_PIXELS.0 + (MODEL_PIXELS.1 - MODEL_PIXELS.0) * ((h >> 24) & 255) as f32 / 255.0;
                let t = smoothstep(at - HANDOVER_SPAN * 0.5, at + HANDOVER_SPAN * 0.5, close) * smoothstep(size_at - 1.0, size_at + 1.0, pixels);
                // (In 256ths: the painting's share, 1 - this, then gives back exactly this, and
                // the two take complementary samples, see `handover_mask` in common.wgsl.)
                shown3d = (t * 256.0).round() / 256.0;
                // Include plant variety and wind at the edge of the view as well as the
                // static shape. The model's nominal width omits its few farthest twigs.
                let radius = si.sizes[pick] * variety.map_or(0.9, |v| 0.9 * v.across.max(v.up)) + varied_height * 0.1;
                if shown3d > 0.0 && view.shows(centre, radius, varied_height) {
                    let lod = foliage_detail(pixels);
                    let def = &sim.data.features[f.def];
                    // On fire: charring as it burns, embers glowing.
                    let yaw = si.yaw.unwrap_or((h % 6283) as f32 / 1000.0);
                    let (scale, leaf, height) = match variety {
                        Some(v) => ([scale[0] * v.across, scale[1] * v.up, scale[2] * v.across], [0, 1, 2].map(|i| si.leaf[i] * v.tint[i]), height * v.up),
                        None => (scale, si.leaf, height),
                    };
                    let (mut foot, mut slope) = ([p[0] + si.offset[0], p[1], p[2] + si.offset[1]], [0.0; 2]);
                    if si.seated {
                        (foot[1], slope) = rock_seat(&sim.world, foot, si.reaches[pick]);
                    }
                    let (char, glow) = if f.burn_until > tick {
                        let total = (((def.burn_min + def.burn_max) * 0.5).max(1.0) * 30.0) as f32;
                        let gone = (1.0 - (f.burn_until - tick) as f32 / total).clamp(0.0, 1.0);
                        if let Some(c) = self.crowns.get(model as usize) {
                            let (sin, cos) = yaw.sin_cos();
                            let l = Vec3::new(c[0] * scale[0], c[1] * scale[1], c[2] * scale[2]);
                            self.flames.push((index, Vec3::from_array(p) + Vec3::new(l.x * cos + l.z * sin, l.y, -l.x * sin + l.z * cos)));
                        }
                        (gone * 0.85, 0.5 + 0.5 * gone)
                    } else {
                        (0.0, 0.0)
                    };
                    if si.parts.is_empty() {
                        self.foliage.push((
                            model,
                            lod,
                            FoliageInstance {
                                pos: foot,
                                yaw,
                                scale,
                                fade: shown3d,
                                leaf: [leaf[0], leaf[1], leaf[2], char],
                                bark: [si.bark[0], si.bark[1], si.bark[2], glow],
                                extra: [height, if si.lies { 1.0 } else { 0.0 }, slope[0], slope[1]],
                                light: [si.light[0], si.light[1], si.light[2], if si.dark { 1.0 } else { 0.0 }],
                                snow: si.snow,
                            },
                        ));
                    } else {
                        // Laid along its painting's lines: each part on the ground where it lies.
                        let seats = self.seats.entry(index).or_default();
                        if seats.0 != f.def || seats.1.len() != si.parts.len() {
                            *seats = (f.def, si.parts.iter().map(|part| seat_part(&sim.world, [p[0], p[2]], part)).collect());
                        }
                        for (part, &(foot, slope)) in si.parts.iter().zip(seats.1.iter()) {
                            self.foliage.push((
                                part.model,
                                lod,
                                FoliageInstance {
                                    pos: [p[0] + part.at[0], foot, p[2] + part.at[1]],
                                    yaw: (-part.way[1]).atan2(part.way[0]),
                                    scale: part.scale,
                                    fade: shown3d,
                                    leaf: [leaf[0], leaf[1], leaf[2], char],
                                    bark: [si.bark[0], si.bark[1], si.bark[2], glow],
                                    extra: [height, 0.0, slope[0], slope[1]],
                                    light: [0.0; 4],
                                    snow: 0.0,
                                },
                            ));
                        }
                    }
                }
            }
            if let Some(shadow) = v.shadow
                && shown3d < 1.0
            {
                self.decals.push(SpriteInstance::new([p[0], p[1] + 0.3, p[2]], shadow).alpha(0.5 * (1.0 - shown3d)));
            }
            if v.frames.is_empty() {
                if v.model.is_some() {
                    self.matrices.push(vec![Mat4::from_translation(Vec3::from_array(p))]);
                }
                continue;
            }
            let frame = if f.burn_until != 0 && !v.burn_frames.is_empty() {
                v.burn_frames[((time * 12.0) as usize + f.origin.0 as usize) % v.burn_frames.len()]
            } else {
                v.frames[((time * 10.0) as usize + f.origin.0 as usize) % v.frames.len()]
            };
            let mut inst = SpriteInstance::new([p[0], p[1] + 0.2, p[2]], frame).alpha(if v.translucent { 0.6 } else { 1.0 });
            if let Some(variety) = variety.filter(|_| self.modern) {
                inst = inst.tint(variety.tint);
            }
            if v.lies && self.modern {
                // (TA's camera showed it from high above: stood on end before a low camera, its
                // painting would be a card twice its height. A vehicle's model takes over from it.)
                if shown3d < 1.0 {
                    self.lying.push(inst.alpha(inst.params[0] * (1.0 - shown3d)));
                }
            } else if v.upright {
                // (Still there when the model has taken over: it shades the ground at its foot
                // and is mirrored in the water.)
                let inst = inst.shown(1.0 - shown3d);
                self.billboards.push(if v.sways { inst.sway(1.0) } else { inst });
            } else if shown3d < 1.0 {
                // (Perspective camera: a deposit richer than the plain ground is a bed in the
                // ground itself, see terrain.wgsl; TA's painted patch, its pastel dither and the
                // ground it was painted on round it, is the classic camera's.)
                let def = &sim.data.features[f.def];
                if v.ore > 0.0 && self.modern && sim.world.ore_under(f.origin, (def.footprint_x, def.footprint_z)) {
                    continue;
                }
                let inst = if v.stand_in.is_some() { inst.alpha(inst.params[0] * (1.0 - shown3d)) } else { inst };
                self.decals.push(if v.ore > 0.0 { inst.ore(v.ore) } else { inst });
            }
        }
    }

    /// The lights of the features in view by night (`lamps`, 0..1, how far they are on; `time`
    /// game seconds): TA Reborn's own lanterns, lit windows and decorated firs (`rebornlight`),
    /// each flickering a little in its own rhythm.
    pub fn night_lights(&self, sim: &Sim, lamps: f32, time: f32, view: (Vec2, Vec2), out: &mut Vec<ta_render::PointLight>) {
        if lamps <= 0.0 || !self.visuals.values().any(|v| v.light.is_some()) {
            return;
        }
        for index in features_in_view(sim, view.0, view.1, 120.0) {
            let f = &sim.features[index as usize];
            let Some((colour, reach, up)) = self.visuals.get(&f.def).and_then(|v| v.light).filter(|_| f.alive) else { continue };
            let mut p = f.pos.to_f32();
            p[1] += crate::scene::ground_offset(&sim.world, Vec3::from_array(p)) + up;
            let phase = (f.origin.0 as f32 * 0.37 + f.origin.1 as f32 * 0.61).fract() * std::f32::consts::TAU;
            let flicker = 0.92 + 0.08 * (time * 6.3 + phase).sin() * (time * 2.9 + phase * 1.7).sin();
            let k = lamps * flicker;
            out.push(ta_render::PointLight::new(p, colour.map(|c| c * k), reach).without_shadows());
        }
    }

    /// The racks of fireworks in view (`rebornfireworks`): each one's feature index and where its
    /// shells go up from.
    pub fn fireworks_in_view(&self, sim: &Sim, view: (Vec2, Vec2)) -> Vec<(usize, Vec3)> {
        if !self.visuals.values().any(|v| v.fireworks) {
            return Vec::new();
        }
        features_in_view(sim, view.0, view.1, 200.0)
            .into_iter()
            .filter_map(|index| {
                let f = &sim.features[index as usize];
                self.visuals.get(&f.def).filter(|v| v.fireworks && f.alive)?;
                let p = Vec3::from_array(f.pos.to_f32());
                Some((index as usize, p + Vec3::Y * (crate::scene::ground_offset(&sim.world, p) + 10.0)))
            })
            .collect()
    }

    /// This frame's 3D trees and rocks.
    pub fn foliage(&self) -> &[(FoliageModelId, f32, FoliageInstance)] {
        &self.foliage
    }

    /// This frame's burning trees drawn in 3D: feature index and the middle of its crown, where
    /// its flames are.
    pub fn flames(&self) -> &Vec<(usize, Vec3)> {
        &self.flames
    }

    /// This frame's sprites: decals, billboards, and the features painted lying, laid on the
    /// ground (perspective camera).
    pub fn sprites(&self) -> (&[SpriteInstance], &[SpriteInstance], &[SpriteInstance]) {
        (&self.decals, &self.billboards, &self.lying)
    }

    /// 3DO feature instances (rest pose; single-piece models get one matrix, others the
    /// root transform applied to each piece's rest offset). The wrecks in `waiting` are left out,
    /// as from the sprites (`update`): most wrecks are models.
    pub fn model_instances<'a>(&'a mut self, sim: &Sim, renderer: &Renderer, view_min: Vec2, view_max: Vec2, waiting: &[usize]) -> Vec<DrawInstance<'a>> {
        let Some(models) = renderer.models.as_ref() else { return Vec::new() };
        let margin = 96.0;
        let now = sim.tick as f32 / 30.0;
        let mut ids = Vec::new();
        let mut heats = Vec::new();
        self.matrices.clear();
        let near = features_in_view(sim, view_min, view_max, margin);
        // Crystals on the metal deposits no building stands on (modern camera).
        if self.modern && !self.ore_models.is_empty() {
            for f in near.iter().map(|&i| &sim.features[i as usize]).filter(|f| f.alive) {
                let def = &sim.data.features[f.def];
                if !def.is_metal_deposit() {
                    continue;
                }
                let p = f.pos.to_f32();
                if p[0] < view_min.x - margin || p[0] > view_max.x + margin || p[2] < view_min.y - margin || p[2] > view_max.y + margin {
                    continue;
                }
                let centre = (f.origin.0 + def.footprint_x as i32 / 2, f.origin.1 + def.footprint_z as i32 / 2);
                if matches!(sim.world.occupant(centre.0, centre.1), ta_sim::world::Occupant::Unit(_)) {
                    continue;
                }
                let h = (f.origin.0 as u32).wrapping_mul(73_856_093) ^ (f.origin.1 as u32).wrapping_mul(19_349_663);
                let yaw = (h % 628) as f32 / 100.0;
                let ground = crate::scene::drawn_ground(&sim.world, p[0], p[2]);
                self.matrices.push(vec![Mat4::from_translation(Vec3::new(p[0], ground, p[2])) * Mat4::from_rotation_y(yaw)]);
                ids.push(self.ore_models[(h >> 8) as usize % self.ore_models.len()]);
                heats.push(0.0);
            }
        }
        for i in near {
            let (i, f) = (i as usize, &sim.features[i as usize]);
            if !f.alive || waiting.contains(&i) {
                continue;
            }
            let Some(visual) = self.visuals.get(&f.def) else { continue };
            let Some(model) = visual.model else { continue };
            let mut p = Vec3::from_array(f.pos.to_f32());
            if p.x < view_min.x - margin || p.x > view_max.x + margin || p.z < view_min.y - margin || p.z > view_max.y + margin {
                continue;
            }
            let ground = sim.world.height_at(f.pos.x, f.pos.z).to_f32();
            p.y = grounded_model_y(p.y, ground, crate::scene::drawn_ground(&sim.world, p.x, p.z), visual.ground_sink);
            let root = Mat4::from_translation(p);
            let rest = models.library.info(model).rest_pose();
            self.matrices.push(rest.into_iter().map(|m| root * m).collect());
            ids.push(model);
            heats.push(self.heat(i, now));
        }
        ids.into_iter()
            .zip(heats)
            .zip(self.matrices.iter())
            .map(|((model, damage), m)| DrawInstance { model, pieces: m, team: 0, flags: 0, build: 1.0, damage, travel: [0.0; 2], piece: None, playing: ta_render::model::PLAYING_ALL })
            .collect()
    }
}

/// A climatic optimizer's lamp (linear): the pale yellow TA paints it.
const OPTIMIZER_LAMP: [f32; 3] = [0.79, 0.66, 0.15];

/// The 3D stand-in of a kind for a painting `p`, if the kind has models. A painting drawn flat
/// on the ground (not `upright`) sizes its model by its width alone.
fn stand_in(library: &FoliageLibrary, kind: Kind, p: &Painting, upright: bool) -> Option<StandIn> {
    let variants = library.variants(kind.prefix());
    if variants.is_empty() {
        return None;
    }
    let mut models: Vec<(FoliageModelId, [f32; 3])> = variants
        .iter()
        .map(|&m| {
            // (A dome keeps its shape too: TA painted it as wide as it is from above.)
            if kind.flat() || !upright || kind == Kind::Dome {
                let s = (p.width / library.get(m).width.max(1.0)).clamp(0.2, 3.0);
                return (m, [s, s, s]);
            }
            if kind.round() {
                // As wide as the painting's bag, the height taken from the variant's tether (up
                // to 8% off rather than the bag stretched into an egg).
                let model = library.get(m);
                let across = (p.width / (2.0 * model.crown[3]).max(1.0)).clamp(0.3, 4.0);
                let up = fit_up(library, m, p, across);
                return (m, [across, across * (up / across).clamp(0.92, 1.08), across]);
            }
            let s = fit(library, m, p);
            let tall = if kind.whole() { 1.0 } else { TREE_HEIGHT };
            (m, [s[0], s[1] * tall, s[2]])
        })
        .collect();
    if kind.round() && upright {
        // (The variants that fit best: their height off by little more than the best's.)
        let misfit = |&(m, s): &(FoliageModelId, [f32; 3])| (library.get(m).ta_height(s[0], s[1]) / p.above.max(1.0)).ln().abs();
        let best = models.iter().map(misfit).fold(f32::MAX, f32::min);
        models.retain(|v| misfit(v) <= best + 0.08);
    }
    if kind == Kind::Hoodoos {
        // (The clusters stretched least out of their shape: a wide painting's spires in a row,
        // not a few stretched fat.)
        let stretch = |&(_, s): &(FoliageModelId, [f32; 3])| (s[1] / s[0]).ln().abs();
        let best = models.iter().map(stretch).fold(f32::MAX, f32::min);
        models.retain(|v| stretch(v) <= best + 0.25);
    }
    let heights = models.iter().map(|&(m, s)| library.get(m).height * s[1]).collect();
    let sizes = models.iter().map(|&(m, s)| (library.get(m).height * s[1]).max(library.get(m).width * s[0])).collect();
    let reaches = models.iter().map(|&(m, s)| library.get(m).width * s[0] * 0.42).collect();
    // A tree's bark takes the colour of its trunk where the painting shows it, lightened towards a
    // brown: painted in the crown's shade, it would be near black in the open. A burnt tree is
    // charred through (but not black), a tuber's root mass the painting's own colour. A rock (and
    // the like) takes all of the painting's, halfway to its lit part (the shade TA painted in
    // is Reborn's light's to give: a dark lava spire would be black), a burnt Mars plant charred
    // but not black.
    const BROWN: [f32; 3] = [0.15, 0.115, 0.085];
    // Charcoal (linear): TA's burnt trees are near black with ash-grey highlights; the floor
    // keeps them wood, lit, rather than black cut-outs.
    const CHAR: [f32; 3] = [0.095, 0.088, 0.080];
    let bark = match kind {
        Kind::Tendrils => [0, 1, 2].map(|i| (0.5 * (p.all[i] + p.all_lit[i])).max(0.05)),
        k if k.whole() => [0, 1, 2].map(|i| 0.5 * (p.all[i] + p.all_lit[i])),
        k if k.dead() => [0, 1, 2].map(|i| p.all[i] * 0.3 + CHAR[i] * 0.7),
        Kind::Tuber if p.foot_pixels >= 6 => p.foot,
        _ if p.foot_pixels >= 6 => [0, 1, 2].map(|i| p.foot[i] * 0.3 + BROWN[i] * 0.7),
        _ => BROWN,
    };
    // The leaves take the colour of the painting's lit side (the renderer shades them), a little
    // less saturated: TA's dither mixes greys and browns into its greens, which the eye averages
    // away.
    let grey = p.crown_lit[0] * 0.2126 + p.crown_lit[1] * 0.7152 + p.crown_lit[2] * 0.0722;
    let mut leaf = if kind.dead() { bark.map(|c| c * 0.9) } else { p.crown_lit.map(|c| c * 0.85 + grey * 0.15) };
    // Crystal: its own colour (`leaf`, the crystals' as painted, dark violet on a crystal world,
    // olive tubes on a lava world's node; a little light for Reborn's light to show their facets),
    // the rock under it (`bark`: a spire's plate; a node's mound as dark as painted, its painting's
    // rock alone, see `StandIn::dark`: lifted, and with its tubes' colour in it, it was a pale
    // patch on the basalt from afar), and the light within it, as painted (the blue of a node's
    // tips, the violet of the crystals, brighter).
    let lum = |c: [f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
    let lifted = |c: [f32; 3], floor: f32| c.map(|v| v * (floor / lum(c).max(1e-4)).max(1.0));
    let mut bark = bark;
    let mut light = [0.0; 3];
    if kind == Kind::Crystal {
        leaf = lifted(p.top, 0.012);
        bark = p.foot;
        light = p.accent.unwrap_or_else(|| lifted(p.top, 0.18));
    } else if kind == Kind::Node {
        leaf = p.warm.unwrap_or(p.crown_lit);
        bark = p.rock.unwrap_or(bark);
        light = p.accent.unwrap_or([0.19, 0.2, 0.43]);
    } else if kind == Kind::Optimizer {
        // (Its lamp TA's pale yellow.)
        light = OPTIMIZER_LAMP;
    }
    let light = lifted(light, 0.2);
    // (Standing as painted: an optimizer's lamp to the south, where TA's camera saw it; a wet
    // desert's spires in their row east to west.)
    let yaw = matches!(kind, Kind::Optimizer | Kind::Hoodoos).then_some(0.0);
    let seated = kind.rock() || matches!(kind, Kind::Dome | Kind::Optimizer);
    Some(StandIn { models, leaf, bark, heights, sizes, flat: kind.flat(), lies: kind == Kind::Tendrils, seated, reaches, varied: !kind.whole(), light, dark: kind == Kind::Node, yaw, offset: [0.0; 2], parts: Vec::new(), snow: 0.0 })
}

/// A vehicle standing in for its painting `p`: as long and as broad as painted, as tall as its
/// kind is for its breadth (TA's paintings are smaller than their features' footprints and
/// heights; TA lifted the top by half its height, north: what of the painting's reach that
/// adds is taken off), turned as painted, its front the end its glass is at, its foot under the
/// painting's middle (half its lift south of it); its paint the painting's (not its glass), its
/// glass dark with the painting's blue in it.
fn vehicle(library: &FoliageLibrary, kind: Kind, p: &Painting) -> Option<StandIn> {
    let variants = library.variants(kind.prefix());
    if variants.is_empty() {
        return None;
    }
    // (Height over breadth: a car is a low pod, a truck a tall tank.)
    let stature = if kind == Kind::Truck { 0.85 } else { 0.55 };
    let breadth = p.breadth / (1.0 + stature * 0.5 * p.axis[0].abs());
    let tall = stature * breadth;
    let lift = tall * 0.5;
    let length = (p.length - lift * p.axis[1].abs()).max(breadth);
    let mut axis = p.axis;
    if let Some(g) = p.glass_at {
        let along = (g[0] - p.centre[0]) * axis[0] + (g[1] - p.centre[1]) * axis[1];
        if along < 0.0 {
            axis = [-axis[0], -axis[1]];
        }
    }
    // (The model's front, +x, turned to (cos, -sin) by its yaw: the painting's y runs south, +z.)
    let yaw = (-axis[1]).atan2(axis[0]);
    let models: Vec<(FoliageModelId, [f32; 3])> = variants
        .iter()
        .map(|&m| {
            let model = library.get(m);
            (m, [length / model.length.max(1.0), tall / model.height.max(1.0), breadth / model.breadth.max(1.0)])
        })
        .collect();
    let heights = models.iter().map(|_| tall).collect();
    let sizes = models.iter().map(|_| length.max(tall)).collect();
    let reaches = models.iter().map(|_| breadth * 0.5).collect();
    let glass = p.accent.unwrap_or([0.2, 0.35, 0.45]);
    let lum = |c: [f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
    let light = glass.map(|c| c * (0.2 / lum(glass).max(1e-4)).max(1.0));
    Some(StandIn {
        models,
        leaf: glass.map(|c| c * 0.3),
        bark: p.body,
        heights,
        sizes,
        flat: false,
        lies: false,
        seated: true,
        reaches,
        varied: false,
        light,
        dark: false,
        yaw: Some(yaw),
        offset: [p.centre[0], p.centre[1] + lift * 0.5],
        parts: Vec::new(),
        snow: 0.0,
    })
}

/// What a painting `p` scatters (its patches, `blobs`), each set where it lies as a piece of its
/// kind: a ruin's masonry blocks, as long and as broad as painted, low (TA's lift of their tops
/// taken off); an urban ruin's broken pillars standing on their painted feet, as wide as painted
/// and twice as tall as their painted lift; rubble's stones as long as painted, squat. In the
/// painting's colour (its shade and light, as a rock's).
fn scattered(library: &FoliageLibrary, kind: Kind, p: &Painting, blobs: &[crate::feature_strokes::Blob]) -> Option<StandIn> {
    let models = library.variants(kind.prefix());
    if models.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    let mut tallest = 0.0f32;
    for (i, b) in blobs.iter().enumerate() {
        let model = models[i % models.len()];
        // (Each turned its own way: a hash of its place in the painting.)
        let spin = ((i as u32).wrapping_mul(2_654_435_761) >> 16) as f32 / 65536.0 * std::f32::consts::TAU;
        let part = match kind {
            Kind::Ruin => {
                // (A lit top, its shaded sides round it: the block a little bigger.)
                let (length, breadth) = (b.length * 1.15 + 0.5, b.breadth * 1.15 + 0.5);
                let tall = (breadth * 0.55).clamp(1.5, 6.0);
                let at = [b.centre[0], b.centre[1] + tall * 0.25];
                Part { model, at, way: b.axis, reach: length * 0.5, scale: [length, tall, breadth] }
            }
            Kind::Stumps => {
                if b.pixels < 12 {
                    continue;
                }
                let wide = (b.across * 0.75).clamp(3.0, 14.0);
                let tall = ((b.down - wide) * 2.0).clamp(wide, 45.0);
                Part { model, at: [b.foot[0], b.foot[1] - wide * 0.5], way: [spin.cos(), spin.sin()], reach: 0.0, scale: [wide, tall, wide] }
            }
            _ => {
                let m = library.get(model);
                let s = ((b.length * 1.15 + 0.5) / m.width.max(1.0)).clamp(0.05, 2.0);
                let tall = m.height * s * 0.7;
                Part { model, at: [b.centre[0], b.centre[1] + tall * 0.25], way: b.axis, reach: 0.0, scale: [s, s * 0.7, s * (b.breadth / b.length.max(1.0)).clamp(0.5, 1.0)] }
            }
        };
        tallest = tallest.max(library.get(part.model).height * part.scale[1]);
        parts.push(part);
    }
    laid(models[0], parts, p, [0, 1, 2].map(|i| 0.5 * (p.all[i] + p.all_lit[i])), tallest)
}

/// A microwave relay standing in for its painting `p`: as wide as painted (its shape kept),
/// turned so its main cabinet's roof is where the painting's warm roof is, its middle under the
/// painting's (TA lifted its tops north: a quarter of its height south of it); its body the
/// painting's colour (shade and light, as a rock's), its roofs the warm roof's.
fn relay(library: &FoliageLibrary, p: &Painting) -> Option<StandIn> {
    let &m = library.variants(Kind::Relay.prefix()).first()?;
    let model = library.get(m);
    let s = (p.width / model.width.max(1.0)).clamp(0.4, 2.5);
    let tall = model.height * s;
    let centre = [p.centre[0], p.centre[1] + tall * 0.25];
    // (The model's main cabinet is at +x, turned to (cos, -sin) by its yaw.)
    let yaw = p.roof.map(|(at, _)| (-(at[1] - p.centre[1])).atan2(at[0] - p.centre[0]));
    let bark = [0, 1, 2].map(|i| 0.5 * (p.all[i] + p.all_lit[i]));
    let roof = p.roof.map_or([0.16, 0.06, 0.035], |(_, c)| c);
    Some(StandIn {
        models: vec![(m, [s; 3])],
        leaf: roof,
        bark,
        heights: vec![tall],
        sizes: vec![model.width * s],
        flat: false,
        lies: false,
        seated: true,
        reaches: vec![model.width * s * 0.42],
        varied: false,
        light: [0.0; 3],
        dark: false,
        yaw,
        offset: centre,
        parts: Vec::new(),
        snow: 0.0,
    })
}

/// Draws a stroke's free ends in along their last stretch by `by(the way out)`, no more than
/// nearly half that stretch.
fn drawn_in(free: [bool; 2], pts: &mut [[f32; 2]], by: impl Fn([f32; 2]) -> f32) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    for (k, &f) in free.iter().enumerate() {
        let (end, next) = if k == 0 { (0, 1) } else { (n - 1, n - 2) };
        let d = dist(pts[end], pts[next]);
        if !f || d < 1e-3 {
            continue;
        }
        let out = [(pts[end][0] - pts[next][0]) / d, (pts[end][1] - pts[next][1]) / d];
        let pull = by(out).min(d * 0.45);
        pts[end] = [pts[end][0] - out[0] * pull, pts[end][1] - out[1] * pull];
    }
}

/// The unit vector from `a` to `b` (east if they meet).
fn way(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let d = dist(a, b);
    if d < 1e-6 { [1.0, 0.0] } else { [(b[0] - a[0]) / d, (b[1] - a[1]) / d] }
}

/// Pipework standing in for its painting `p`, laid along its lines (`strokes`): lengths of pipe
/// along each stretch of each, as wide as painted, a ball where one bends, a bolted collar every
/// so often, a domed cap at each free end and a ball where lines meet, as wide as the widest
/// pipe there; in the painting's colour (its shade and light, as a rock's). TA drew a pipe's top lifted north by its radius
/// (its centre line by half that): its lines are taken that far south, and its ends drawn in by
/// what of their reach the lift adds.
fn pipework(library: &FoliageLibrary, p: &Painting, strokes: &Strokes) -> Option<StandIn> {
    let [run, joint, collar, cap] = ["pipe_run", "pipe_joint", "pipe_collar", "pipe_cap"].map(|n| library.id(n));
    let (run, joint, collar, cap) = (run?, joint?, collar?, cap?);
    // (Lengths of pipe no longer than this, each set on the ground at its ends; how far the cap
    // reaches past a pipe's end, in radii.)
    const LONGEST: f32 = 32.0;
    const CAP: f32 = 0.42;
    // (TA painted a pipe running east and west a tenth wider than one running north and south:
    // its top lifted over its side.)
    let radius = |s: &Stroke| {
        let w = way(s.points[0], *s.points.last().unwrap_or(&s.points[0]));
        s.radius / (1.0 + 0.118 * w[0].abs())
    };
    // Each junction's widest pipe.
    let mut hubs: Vec<(Junction, f32)> = strokes.junctions.iter().map(|&j| (j, 0.0)).collect();
    let at_hub = |hubs: &[(Junction, f32)], q: [f32; 2]| hubs.iter().position(|(j, _)| dist(j.at, q) < 0.75);
    for s in &strokes.strokes {
        let r = radius(s);
        for k in 0..2 {
            if !s.free[k] && let Some(h) = at_hub(&hubs, if k == 0 { s.points[0] } else { s.points[s.points.len() - 1] }) {
                hubs[h].1 = hubs[h].1.max(r);
            }
        }
    }
    let mut parts = Vec::new();
    for s in &strokes.strokes {
        let r = radius(s);
        let n = s.points.len();
        let pts: Vec<[f32; 2]> = s.points.iter().enumerate().map(|(i, &q)| {
            // (A joined end is its junction's: taken south as far as the junction's widest pipe.)
            let joined = (i == 0 && !s.free[0]) || (i == n - 1 && !s.free[1]);
            let lift = if joined { at_hub(&hubs, q).map_or(r, |h| hubs[h].1) } else { r };
            [q[0], q[1] + lift * 0.5]
        }).collect();
        let mut pts = pts;
        drawn_in(s.free, &mut pts, |out| r * 0.5 * out[1].abs() + r * CAP);
        // Its caps.
        for k in 0..2 {
            if s.free[k] {
                let (end, next) = if k == 0 { (pts[0], pts[1]) } else { (pts[n - 1], pts[n - 2]) };
                parts.push(Part { model: cap, at: end, way: way(next, end), reach: r, scale: [r; 3] });
            }
        }
        // Its lengths, and a ball where it bends.
        for (i, w) in pts.windows(2).enumerate() {
            let d = dist(w[0], w[1]);
            if d < 0.05 {
                continue;
            }
            let along = way(w[0], w[1]);
            let pieces = (d / LONGEST).ceil().max(1.0);
            for k in 0..pieces as usize {
                let t = (k as f32 + 0.5) / pieces;
                parts.push(Part { model: run, at: [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t], way: along, reach: d / pieces * 0.5, scale: [d / pieces, r, r] });
            }
            if i + 2 < n {
                let next = way(w[1], pts[i + 2]);
                if along[0] * next[0] + along[1] * next[1] < 0.998 {
                    parts.push(Part { model: joint, at: w[1], way: along, reach: 0.0, scale: [r; 3] });
                }
            }
        }
        // A collar every so often, clear of its bends and ends.
        let line = Stroke { points: pts.clone(), radius: r, free: s.free };
        let total = line.length();
        let count = (total / (r * 7.5).clamp(18.0, 40.0)).floor();
        let mut bends = vec![0.0];
        let mut run_to = 0.0;
        for w in pts.windows(2) {
            run_to += dist(w[0], w[1]);
            bends.push(run_to);
        }
        for k in 0..count as usize {
            let s_at = (k as f32 + 0.5) * total / count;
            if bends.iter().any(|&b| (b - s_at).abs() < r * 1.6) {
                continue;
            }
            let (a, b) = (line.at(s_at - 0.1), line.at(s_at + 0.1));
            parts.push(Part { model: collar, at: line.at(s_at), way: way(a, b), reach: r, scale: [r; 3] });
        }
    }
    for (j, r) in &hubs {
        if *r > 0.0 {
            parts.push(Part { model: joint, at: [j.at[0], j.at[1] + r * 0.5], way: [1.0, 0.0], reach: 0.0, scale: [*r; 3] });
        }
    }
    let widest = parts.iter().map(|q| q.scale[1]).fold(0.0, f32::max);
    laid(run, parts, p, [0, 1, 2].map(|i| 0.5 * (p.all[i] + p.all_lit[i])), widest * 2.6)
}

/// A row of concrete road barriers standing in for its painting `p`, laid end to end along its
/// lines (`strokes`): as many as fit at about their own length, each as broad as painted; their
/// concrete and stripes their own colours, as light as the painting. TA drew a barrier's top
/// lifted north by half its height: a row's north end is drawn in by what of its reach that adds.
fn barriers(library: &FoliageLibrary, p: &Painting, strokes: &Strokes) -> Option<StandIn> {
    let model = library.id("barrier_a")?;
    let m = library.get(model);
    // (A hair's breadth between barriers.)
    const GAP: f32 = 0.6;
    let mut parts = Vec::new();
    for s in &strokes.strokes {
        // (Its painted half-width: a barrier's foot and its top lifted over it, as TA shows it.)
        let k = (s.radius / 7.3).clamp(0.85, 1.1);
        let tall = m.height * k;
        let mut pts = s.points.clone();
        drawn_in(s.free, &mut pts, |out| tall * 0.5 * (-out[1]).max(0.0));
        let line = Stroke { points: pts, radius: s.radius, free: s.free };
        let total = line.length();
        let count = (total / m.length).round().max(1.0);
        let each = total / count;
        for i in 0..count as usize {
            let (a, b) = (line.at(i as f32 * each + GAP * 0.5), line.at((i + 1) as f32 * each - GAP * 0.5));
            let d = dist(a, b).max(1e-3);
            parts.push(Part { model, at: [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5], way: way(a, b), reach: d * 0.5, scale: [d / m.length, k, k] });
        }
    }
    let layer = ta_render::foliage::LAYERS.iter().position(|&l| l == "barrier")?;
    let mean = library.layer_mean[layer];
    let lum = |c: [f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
    let paint = mean.map(|c| c * lum(p.all_lit) / lum(mean).max(1e-4));
    laid(model, parts, p, paint, m.height * 1.1)
}

/// A stand-in made of `parts` (see `Part`; `first` its first model), `tall` at most, in the
/// colour `paint` (linear, its bark's: the parts are opaque).
fn laid(first: FoliageModelId, parts: Vec<Part>, p: &Painting, paint: [f32; 3], tall: f32) -> Option<StandIn> {
    if parts.is_empty() {
        return None;
    }
    let size = p.length.max(p.breadth);
    Some(StandIn {
        models: vec![(first, [1.0; 3])],
        leaf: paint,
        bark: paint,
        heights: vec![tall],
        sizes: vec![size],
        flat: false,
        lies: false,
        seated: false,
        reaches: vec![size * 0.5],
        varied: false,
        light: [0.0; 3],
        dark: false,
        yaw: None,
        offset: [0.0; 2],
        parts,
        snow: 0.0,
    })
}

/// TA Reborn's own feature's stand-in (its `rebornmodel`, see tools/features/festive.py): the
/// model as it was made, its painting drawn from it (so neither stretched nor lowered), turned as
/// painted (to the south), in its own colours (`FoliageLibrary::own_colours`) and snow.
fn own_stand_in(library: &FoliageLibrary, name: &str) -> Option<StandIn> {
    let m = library.id(name)?;
    let model = library.get(m);
    let own = model.own.unwrap_or_default();
    let (leaf, bark) = library.own_colours(&own);
    Some(StandIn {
        models: vec![(m, [1.0; 3])],
        leaf,
        bark,
        heights: vec![model.height],
        sizes: vec![model.height.max(model.width)],
        flat: false,
        lies: false,
        // (A boulder set on the slope it stands on.)
        seated: own.bark.is_some(),
        reaches: vec![model.width * 0.42],
        varied: false,
        light: own.light,
        dark: true,
        yaw: Some(0.0),
        offset: [0.0; 2],
        parts: Vec::new(),
        snow: own.snow,
    })
}

/// Where a part of a stand-in laid along its painting's lines (see `Part`) lies, at the feature's
/// place `at` (east, south): its foot's height (between the ground's at its ends, or the ground's
/// at its middle if it has no reach) and the slope it follows (its rise for a unit east and
/// south).
fn seat_part(world: &ta_sim::world::World, at: [f32; 2], part: &Part) -> (f32, [f32; 2]) {
    let (x, z) = (at[0] + part.at[0], at[1] + part.at[1]);
    if part.reach <= 0.0 {
        return (crate::scene::drawn_ground(world, x, z), [0.0; 2]);
    }
    let (dx, dz) = (part.way[0] * part.reach, part.way[1] * part.reach);
    let (h0, h1) = (crate::scene::drawn_ground(world, x - dx, z - dz), crate::scene::drawn_ground(world, x + dx, z + dz));
    let rise = (h1 - h0) / (2.0 * part.reach);
    ((h0 + h1) * 0.5, [part.way[0] * rise, part.way[1] * rise])
}

/// How one plant of a kind grows at its place: TA's forests were the same few paintings over
/// and over, and their models' copies stood in rows of identical balls. Each is a little bigger
/// or smaller, broader or taller (its model), its leaves lighter or darker, more olive or bluer
/// than its painting, a little more often olive (its model's leaves, and its painting in the
/// perspective camera, so the two still hand over as one).
#[derive(Clone, Copy, Debug)]
struct Variety {
    across: f32,
    up: f32,
    tint: [f32; 3],
}

impl Variety {
    /// The plant at a place (`h`, its place's hash).
    fn at(h: u32) -> Variety {
        let g = h.wrapping_mul(0x9e37_79b9) ^ (h >> 13);
        let unit = |k: u32| ((g >> (8 * k)) & 255) as f32 / 255.0;
        let across = 0.9 + 0.2 * unit(0);
        let tone = 0.86 + 0.24 * unit(2);
        let warm = unit(3) - 0.35;
        Variety { across, up: across * (0.93 + 0.14 * unit(1)), tint: [tone * (1.0 + 0.3 * warm), tone, tone * (1.0 - 0.35 * warm)] }
    }
}

/// How a rock at `p` stands on a slope or a plateau's edge: its foot following the slope's plane
/// (most of the way: it stays upright, see `place` in foliage.wgsl), and sunk by what of the
/// ground within `reach` of its foot falls away below that plane, so none of it hangs out over
/// the drop with its underside in view above its shadow. Its foot's height, and the slope (the
/// ground's rise for a unit east and south).
fn rock_seat(world: &ta_sim::world::World, p: [f32; 3], reach: f32) -> (f32, [f32; 2]) {
    const FOLLOW: f32 = 0.75;
    const STEEPEST: f32 = 0.7;
    let ground: [(f32, f32, f32); 8] = std::array::from_fn(|k| {
        let (s, c) = (k as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
        (c, s, crate::scene::drawn_ground(world, p[0] + c * reach, p[2] + s * reach) - p[1])
    });
    // (The plane through the eight: over eight even directions the sum of cos² is 4.)
    let mut slope = ground.iter().fold([0.0f32; 2], |g, &(c, s, h)| [g[0] + c * h, g[1] + s * h]).map(|g| g / (4.0 * reach.max(1.0)) * FOLLOW);
    let steep = (slope[0] * slope[0] + slope[1] * slope[1]).sqrt();
    if steep > STEEPEST {
        slope = slope.map(|g| g * STEEPEST / steep);
    }
    let below = ground.iter().map(|&(c, s, h)| h - (slope[0] * c + slope[1] * s) * reach).fold(0.0f32, f32::min);
    (p[1] + below, slope)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The features (standing or not), in index order, that may lie in the ground rectangle
/// `view_min`..`view_max` grown by `margin`: every one that does among them.
fn features_in_view(sim: &Sim, view_min: Vec2, view_max: Vec2, margin: f32) -> Vec<u32> {
    let (centre, half) = ((view_min + view_max) * 0.5, (view_max - view_min) * 0.5 + Vec2::splat(margin));
    let at = ta_sim::FxVec3::new(ta_sim::Fx::from_f32(centre.x), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(centre.y));
    let mut near = Vec::new();
    sim.features_near(at, ta_sim::Fx::from_f32(half.x.max(half.y).max(0.0) + 1.0), &mut near);
    near
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_ground_sink(model: &ta_formats::model3do::Model3do) -> f32 {
        let mut rest = Vec::<Mat4>::new();
        for piece in &model.pieces {
            let [x, y, z] = piece.offset;
            let local = Mat4::from_translation(Vec3::new(x as f32, y as f32, -(z as f32)) / ta_formats::model3do::FIXED_ONE);
            rest.push(piece.parent.map_or(local, |p| rest[p] * local));
        }
        model_ground_sink(&rest, &crate::scene::PieceSurface::of(model))
    }

    #[test]
    fn wreck_ground_contact_follows_rest_hierarchy_and_ignores_selection_plate() {
        use ta_formats::model3do::{Model3do, Piece, Primitive};
        let fixed = |y: i32| [0, y * 65536, 0];
        let model = Model3do { pieces: vec![
            Piece { name: "root".into(), parent: None, offset: fixed(20), vertices: Vec::new(), primitives: Vec::new(), selection_primitive: None, children: vec![1] },
            Piece {
                name: "hull".into(), parent: Some(0), offset: fixed(3),
                vertices: vec![fixed(-14), fixed(-10), fixed(-8), fixed(-100), fixed(-101), fixed(-102)],
                primitives: vec![
                    Primitive { color_index: 0, indices: vec![3, 4, 5], texture: None },
                    Primitive { color_index: 0, indices: vec![0, 1, 2], texture: None },
                ],
                selection_primitive: Some(0), children: Vec::new(),
            },
        ] };
        assert_eq!(source_ground_sink(&model), 9.0);
        assert_eq!(grounded_model_y(30.0, 30.0, 35.0, 9.0), 26.0, "the hull bottom meets the drawn ground, including raised pads");
        assert_eq!(grounded_model_y(80.0, 30.0, 35.0, 9.0), 80.0, "an elevated or floating feature keeps its altitude");
        let mut buried = model;
        buried.pieces[0].offset = fixed(0);
        assert_eq!(source_ground_sink(&buried), 0.0, "do not expose deliberately buried geometry");
        assert_eq!(source_ground_sink(&Model3do::default()), 0.0);
    }

    #[test]
    fn installed_reaper_wreck_has_ground_contact() {
        let Some(vfs) = install() else { return };
        let model = ta_formats::model3do::Model3do::parse(&vfs.read("objects3d/correap_dead.3do").unwrap()).unwrap();
        let sink = source_ground_sink(&model);
        assert!((sink - 9.0207).abs() < 0.01, "the Reaper's visible hull used to float nine units above its anchor: {sink}");
        let seated = grounded_model_y(40.0, 40.0, 40.0, sink);
        assert!((seated + sink - 40.0).abs() < 1e-4);
    }

    #[test]
    fn foliage_detail_changes_continuously_with_camera_distance() {
        assert_eq!(foliage_detail(10.0), 2.0);
        assert_eq!(foliage_detail(120.0), 1.0);
        assert_eq!(foliage_detail(400.0), 0.0);
        for boundary in [LOD0_PIXELS, LOD1_PIXELS] {
            assert!((foliage_detail(boundary - 0.01) - foliage_detail(boundary + 0.01)).abs() < 0.003,
                "a small camera movement must not replace a whole mesh");
        }
        let mut previous = 2.0;
        for tenths in 0..4000 {
            let detail = foliage_detail(tenths as f32 * 0.1);
            assert!(detail <= previous && previous - detail < 0.012);
            previous = detail;
        }
    }

    #[test]
    fn offscreen_foliage_is_kept_for_reflections_and_long_shadows() {
        let mut cam = ta_render::Camera::new(Vec2::new(1920.0, 1080.0));
        cam.focus = Vec3::new(3000.0, 0.0, 3000.0);
        cam.zoom = 4.0;
        let light = Vec3::new(1.0, 0.1, 0.0).normalize();
        let view = FoliageView { eye: cam.eye(), planes: clip_planes(cam.view_proj()), light: Some(light), ..Default::default() };
        let caster = cam.focus + ta_render::camera::shadow_caster_offset(light, 150.0);
        assert!(!view.sees(caster, 30.0), "the caster must be outside the picture");
        assert!(view.shows(caster, 30.0, 100.0), "its shadow reaches the view");
        assert!(!FoliageView { light: None, ..view }.shows(caster, 30.0, 100.0));

        let (o, d) = cam.screen_ray(Vec2::new(960.0, 900.0));
        let reflection = o + d * ((-400.0 - o.y) / d.y);
        let source = Vec3::new(reflection.x, 400.0, reflection.z);
        let view = FoliageView { mirror: Some(0.0), light: None, ..view };
        assert!(!view.sees(source, 5.0));
        assert!(view.shows(source, 5.0, 100.0), "its mirror image is visible");
    }

    fn install() -> Option<Vfs> {
        let install = ta_vfs::install::discover(None).ok()?;
        let mut vfs = Vfs::new();
        vfs.mount_game_dir(&install.root, "base").ok()?;
        Some(vfs)
    }

    /// A feature's definition and its painting (its first frame).
    fn painted(vfs: &Vfs, data: &ta_data::GameData, palette: &Palette, name: &str) -> (FeatureDef, Painting) {
        let def = data.feature(name).unwrap_or_else(|| panic!("no feature {name}")).clone();
        let gaf = Gaf::parse(&vfs.read(&format!("anims/{}.gaf", def.filename.to_ascii_lowercase())).unwrap()).unwrap();
        let frame = gaf.entry(&def.seq_name).and_then(|e| e.frames.first()).unwrap_or_else(|| panic!("{name}: no frame"));
        (def.clone(), Painting::of(frame, palette).unwrap())
    }

    /// TA's own trees and rocks get the 3D stand-ins they look like; flat features, burnt trees
    /// and metal deposits stay painted. (Skipped without a TA install.)
    #[test]
    fn ta_trees_and_rocks_get_their_kinds() {
        let Some(vfs) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let data = ta_data::GameData::load(&vfs);
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        use Kind::*;
        let kind = |name: &str| {
            let (def, p) = painted(&vfs, &data, &palette, name);
            // As the game does: a burnt tree by the tree that burns into it.
            let from = data.features.iter().find(|d| d.feature_burnt.eq_ignore_ascii_case(name) && !d.filename.is_empty()).map(|d| {
                let (def, p) = painted(&vfs, &data, &palette, &d.name);
                kind_of(&def, &p, None).unwrap()
            });
            kind_of(&def, &p, from)
        };
        let expect = [
            ("Tree1", Some(Broadleaf)),
            ("Tree2", Some(Broadleaf)),
            ("Tree3", Some(Broadleaf)),
            ("Tree4", Some(Conifer)),
            ("Tree5", Some(Conifer)),
            ("Tree6", Some(Conifer)),
            ("Tree1Dead", Some(DeadBroadleaf)),
            ("Tree4Dead", Some(DeadConifer)),
            ("Tree5Dead", Some(DeadConifer)),
            ("Lush04", Some(JungleCanopy)),
            ("Lush06", Some(JungleCanopy)),
            ("Lush02", Some(JungleTall)),
            ("LushTree01", Some(JungleTall)),
            ("Lush07", Some(Tuber)),
            ("Lush04crisp", Some(DeadJungle)),
            ("Lush02crisp", Some(DeadTall)),
            ("Lush08crisp", Some(DeadTuber)),
            ("Palm01", Some(Tropical)),
            ("ArchiTree04", Some(Tropical)),
            ("Palm04crisp", Some(DeadTropical)),
            ("Shrub3", Some(Bush)),
            ("lushfoliage01", Some(Fern)),
            ("Frond02", Some(Fern)),
            ("lushvine01", None),
            ("Frond02crisp", None),
            ("RockMetal1", None),
            ("Rockgone06", None),
            ("DryRock04", Some(Boulder)),
            ("MarsRock17", Some(Boulder)),
            ("LushRock05", Some(Boulder)),
            ("MoonRock09", Some(Boulder)),
            ("WetRock14", Some(Hoodoos)),
            ("WetRock04", Some(Hoodoos)),
            ("WetRock12", Some(Boulder)),
            ("LavaSpire002", Some(Spire)),
            ("FogStrata02", Some(Shards)),
            ("CrystalSpire11", Some(Crystal)),
            ("IceChunk05", Some(Ice)),
            ("AcidPlant01", Some(Gasbag)),
            ("LavaNode006", Some(Node)),
            ("MarsPlant01", Some(Tendrils)),
            ("MarsPlant02Crisp", Some(Tendrils)),
            ("MarsPlant01Gone", None),
            ("DryRock02Scar", None),
            ("DryRock10Scar", None),
            ("DryRock14Scar", None),
            ("Car06", Some(Car)),
            ("Car13", Some(Car)),
            ("Truck03", Some(Truck)),
            ("CarScar01", None),
            ("Pipe01", Some(Pipe)),
            ("Pipe13", Some(Pipe)),
            ("Pipe01Hole", None),
            ("Barrier09", Some(Barrier)),
            ("Barrier17", Some(Barrier)),
            ("widget01", Some(Dome)),
            ("widget06", Some(Dome)),
            ("widget07", Some(Relay)),
            ("widget10", Some(Relay)),
            ("widget11", Some(Optimizer)),
            ("widget16", Some(Optimizer)),
            ("DryRuin01", Some(Ruin)),
            ("DryRuin18", Some(Ruin)),
            ("Building14c", Some(Stumps)),
            ("Building12c", None),
            ("RockTeeth", Some(Rubble)),
        ];
        for (name, want) in expect {
            assert_eq!(kind(name), want, "{name}");
        }
    }

    /// TA's cars, pipes, barriers, ruins and low machines were painted lying (from TA's camera
    /// high above): the perspective camera lays them on the ground; towers, buildings, statues
    /// and the trees (3D stand-ins up close) stand. (Skipped without a TA install.)
    #[test]
    fn low_features_painted_from_above_lie_down() {
        let Some(vfs) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let data = ta_data::GameData::load(&vfs);
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let lies = |name: &str| {
            let (def, p) = painted(&vfs, &data, &palette, name);
            def.height > 4.0 && painted_lying(&def, &p)
        };
        for name in ["Car08", "Car12", "Car13", "Car15", "Truck03", "Truck07", "Pipe01", "Pipe04", "Barrier09", "Barrier16", "Barrier04", "DryRuin01", "widget01", "widget08", "MoonCrater08", "MetalTower03"] {
            assert!(lies(name), "{name} stands");
        }
        for name in ["Tree1", "Tree4", "Building01", "Building05", "widget11", "MetalTower01", "Rail01", "comstat01", "Kelp-WithShadow"] {
            assert!(!lies(name), "{name} lies down");
        }
    }

    /// A stand-in is as wide as its painting and, seen as TA shows things, as tall above its
    /// anchor (before trees are lowered, see `TREE_HEIGHT`); its colours are the painting's.
    #[test]
    fn stand_ins_fit_their_paintings() {
        let Some(vfs) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let data = ta_data::GameData::load(&vfs);
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let library = FoliageLibrary::load().unwrap();
        for name in ["Tree1", "Tree4", "Lush04", "Palm02", "MarsRock17"] {
            let (def, p) = painted(&vfs, &data, &palette, name);
            let kind = kind_of(&def, &p, None).unwrap();
            for m in library.variants(kind.prefix()) {
                let s = fit(&library, m, &p);
                let model = library.get(m);
                assert!((model.width * s[0] - p.width).abs() < 1.0, "{name} as {}: {} wide, painted {}", model.name, model.width * s[0], p.width);
                let shown = model.ta_height(s[0], s[1]);
                assert!((shown - p.above).abs() < 2.0 || s[1] / s[0] < 0.56 || s[1] / s[0] > 1.89, "{name} as {}: {shown} tall as TA shows it, painted {}", model.name, p.above);
            }
            // The leaves as bright as the painting's lit side (if a little less saturated), which
            // is brighter than the crown's mean.
            let si = stand_in(&library, kind, &p, true).unwrap();
            let lum = |c: [f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
            assert!((lum(si.leaf) - lum(p.crown_lit)).abs() < 1e-4, "{name}: leaves {:?}, painted {:?}", si.leaf, p.crown_lit);
        }
    }

    /// Lists the features still painted lying (no 3D stand-in) on the install's maps, the most
    /// often placed first: what a low camera sees lying flat most. (Ignored; skipped without a
    /// TA install.)
    #[test]
    #[ignore]
    fn list_features_painted_lying() {
        let Some(mut vfs) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        vfs.mount_builtin();
        let data = ta_data::GameData::load(&vfs);
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        // Each feature's placements: (count, maps).
        let mut placed: std::collections::BTreeMap<String, (usize, std::collections::BTreeSet<String>)> = Default::default();
        for path in vfs.list("maps", Some("tnt"), false) {
            let Some(map) = vfs.read_opt(&path).and_then(|b| ta_formats::tnt::TntMap::parse(&b).ok()) else { continue };
            let name = path.trim_start_matches("maps/").trim_end_matches(".tnt").to_string();
            let wanted = std::env::var("TA_LYING_MAP").ok().filter(|m| m.eq_ignore_ascii_case(&name));
            for (cell, &slot) in map.features.iter().enumerate() {
                if let Some(f) = map.feature_names.get(slot as usize) {
                    if wanted.is_some() {
                        eprintln!("at {} {} {f}", (cell as u32 % map.width) * 16, (cell as u32 / map.width) * 16);
                    }
                    let e = placed.entry(f.to_ascii_lowercase()).or_default();
                    e.0 += 1;
                    e.1.insert(name.clone());
                }
            }
        }
        let mut rows = Vec::new();
        for (fname, (count, maps)) in &placed {
            let Some(def) = data.feature(fname) else { continue };
            if def.filename.is_empty() || def.height <= 4.0 || def.is_metal_deposit() {
                continue;
            }
            let Some(gaf) = vfs.read_opt(&format!("anims/{}.gaf", def.filename.to_ascii_lowercase())).and_then(|b| Gaf::parse(&b).ok()) else { continue };
            let Some(p) = gaf.entry(&def.seq_name).and_then(|e| e.frames.first()).and_then(|f| Painting::of(f, &palette)) else { continue };
            let kind = kind_of(def, &p, None);
            let lies = painted_lying(def, &p);
            if lies && kind.is_none() {
                rows.push((*count, def.name.clone(), def.world.clone(), def.category.clone(), def.description.clone(), format!("{}/{}", def.filename, def.seq_name), def.footprint_x, def.footprint_z, def.height, p.width, p.above, p.below, p.length, p.breadth, maps.iter().take(6).cloned().collect::<Vec<_>>().join(", ")));
            }
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        for r in &rows {
            eprintln!("{:6} {:18} world={:10} cat={:14} desc={:22} gaf={:24} fp={}x{} h={} w={} above={} below={} len={:.0} br={:.0} [{}]", r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10, r.11, r.12, r.13, r.14);
        }
        eprintln!("{} lying types, {} placements", rows.len(), rows.iter().map(|r| r.0).sum::<usize>());
    }

    /// TA's pipes and barriers are laid along their paintings' lines: every one of them gets its
    /// pieces, a row of barriers as many as it shows, a straight pipe a length of pipe capped at
    /// both ends. (Skipped without a TA install.)
    #[test]
    fn pipes_and_barriers_are_laid_along_their_paintings() {
        let Some(vfs) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let data = ta_data::GameData::load(&vfs);
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let library = FoliageLibrary::load().unwrap();
        let laid = |name: &str| {
            let (def, p) = painted(&vfs, &data, &palette, name);
            let gaf = Gaf::parse(&vfs.read(&format!("anims/{}.gaf", def.filename.to_ascii_lowercase())).unwrap()).unwrap();
            let f = gaf.entry(&def.seq_name).and_then(|e| e.frames.first()).unwrap();
            let strokes = crate::feature_strokes::trace(&f.image.opaque, f.image.width as usize, f.image.height as usize, [f.origin_x as f32, f.origin_y as f32]);
            match kind_of(&def, &p, None) {
                Some(Kind::Pipe) => pipework(&library, &p, &strokes),
                Some(Kind::Barrier) => barriers(&library, &p, &strokes),
                k => panic!("{name}: {k:?}"),
            }
            .unwrap_or_else(|| panic!("{name}: no stand-in"))
        };
        let count = |si: &StandIn, model: &str| si.parts.iter().filter(|q| q.model == library.id(model).unwrap()).count();
        for name in data.features.iter().filter(|d| d.height > 4.0 && ["pipes", "barriers"].iter().any(|c| d.category.eq_ignore_ascii_case(c))).map(|d| d.name.clone()) {
            let si = laid(&name);
            // Within the painting's reach of its place.
            assert!(si.parts.iter().all(|q| q.at[0].abs() < 90.0 && q.at[1].abs() < 90.0), "{name}: {:?}", si.parts);
        }
        let row = laid("Barrier10");
        assert_eq!(count(&row, "barrier_a"), 3);
        assert!(row.parts.iter().all(|q| (q.scale[0] - 1.0).abs() < 0.12 && q.way[0].abs() > 0.99), "{:?}", row.parts);
        let pipe = laid("Pipe04");
        assert_eq!(count(&pipe, "pipe_cap"), 2);
        assert!(count(&pipe, "pipe_run") >= 4 && count(&pipe, "pipe_collar") >= 2);
    }

    /// Draws the strokes traced through TA's pipes' and barriers' paintings over them, into
    /// `TA_STROKES_OUT` (ignored; skipped without a TA install).
    #[test]
    #[ignore]
    fn dump_painted_strokes() {
        let (Some(vfs), Ok(out)) = (install(), std::env::var("TA_STROKES_OUT")) else {
            eprintln!("TA install or TA_STROKES_OUT not found; skipping");
            return;
        };
        let data = ta_data::GameData::load(&vfs);
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let names: Vec<String> = data.features.iter().filter(|d| d.height > 4.0 && ["pipes", "barriers"].iter().any(|c| d.category.eq_ignore_ascii_case(c))).map(|d| d.name.clone()).collect();
        for name in names {
            let def = data.feature(&name).unwrap();
            let Some(gaf) = vfs.read_opt(&format!("anims/{}.gaf", def.filename.to_ascii_lowercase())).and_then(|b| Gaf::parse(&b).ok()) else { continue };
            let Some(frame) = gaf.entry(&def.seq_name).and_then(|e| e.frames.first()) else { continue };
            let img = &frame.image;
            let (w, h) = (img.width as usize, img.height as usize);
            let s = crate::feature_strokes::trace(&img.opaque, w, h, [frame.origin_x as f32, frame.origin_y as f32]);
            const K: u32 = 6;
            let mut pic = image::RgbaImage::new(w as u32 * K, h as u32 * K);
            for y in 0..h {
                for x in 0..w {
                    let c = if img.opaque[y * w + x] { let c = palette.rgba(img.pixels[y * w + x]); [c[0] / 2, c[1] / 2, c[2] / 2, 255] } else { [40, 0, 40, 255] };
                    for dy in 0..K {
                        for dx in 0..K {
                            pic.put_pixel(x as u32 * K + dx, y as u32 * K + dy, image::Rgba(c));
                        }
                    }
                }
            }
            let to_px = |p: [f32; 2]| [(p[0] + frame.origin_x as f32) * K as f32, (p[1] + frame.origin_y as f32) * K as f32];
            let mut dot = |p: [f32; 2], r: f32, c: [u8; 4]| {
                let q = to_px(p);
                let r = r * K as f32;
                for y in (q[1] - r).floor() as i32..=(q[1] + r).ceil() as i32 {
                    for x in (q[0] - r).floor() as i32..=(q[0] + r).ceil() as i32 {
                        if x >= 0 && y >= 0 && (x as u32) < pic.width() && (y as u32) < pic.height() && ((x as f32 - q[0]).powi(2) + (y as f32 - q[1]).powi(2)) <= r * r {
                            pic.put_pixel(x as u32, y as u32, image::Rgba(c));
                        }
                    }
                }
            };
            for k in &s.strokes {
                for wnd in k.points.windows(2) {
                    let n = (crate::feature_strokes::dist(wnd[0], wnd[1]) * 4.0).ceil().max(1.0) as usize;
                    for i in 0..=n {
                        let t = i as f32 / n as f32;
                        dot([wnd[0][0] + (wnd[1][0] - wnd[0][0]) * t, wnd[0][1] + (wnd[1][1] - wnd[0][1]) * t], 0.18, [0, 255, 0, 255]);
                    }
                }
                for p in &k.points {
                    dot(*p, 0.4, [255, 255, 0, 255]);
                }
                // Its half-width, at its middle.
                let mid = k.at(k.length() * 0.5);
                dot(mid, k.radius, [0, 160, 255, 90]);
            }
            for j in &s.junctions {
                dot(j.at, 0.6, [255, 0, 0, 255]);
            }
            eprintln!("{name}: {} strokes ({}), {} junctions", s.strokes.len(), s.strokes.iter().map(|k| format!("r {:.1} len {:.0} free {:?}", k.radius, k.length(), k.free)).collect::<Vec<_>>().join("; "), s.junctions.len());
            pic.save(format!("{out}/{name}.png")).unwrap();
        }
    }
}
