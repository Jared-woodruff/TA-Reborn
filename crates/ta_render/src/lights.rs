//! Dynamic point lights: explosions, muzzle flashes, laser glow, burning wrecks.
//!
//! The caller submits this frame's lights with [`Renderer::set_lights`](crate::Renderer::set_lights).
//! At render time the lights whose influence circle (in xz) touches the visible ground are
//! binned into a coarse uniform world-space grid of at most [`GRID_MAX_DIM`]² cells that
//! covers only the union of those circles. Every shader that includes `common.wgsl` can then
//! call `point_lighting(world_pos, normal)`: it looks up the fragment's cell and loops over
//! the few lights listed there, so the per-fragment cost is proportional to the number of
//! lights that can actually reach it, and fragments outside every light's reach return after
//! a single bounds check.
//!
//! GPU layout (frame bind group, group 0):
//! * binding 3, `point_lights: array<PointLightGpu>` — [`MAX_LIGHTS`] × 32 bytes.
//! * binding 4, `light_grid: array<u32>` — `dims.x × dims.y` packed cell ranges
//!   (`first | count << 16`, `first` indexes this same array) followed by light indices.
//!
//! Grid origin, cell size, dimensions and the light count travel in the frame uniform.

use bytemuck::{Pod, Zeroable};
use glam::{UVec2, Vec2, Vec3};

/// Maximum number of lights uploaded per frame. When more are submitted, the most
/// significant ones (brightness × radius) are kept.
pub const MAX_LIGHTS: usize = 256;
/// Maximum grid cells per axis.
pub const GRID_MAX_DIM: u32 = 32;
/// Capacity of the light index list; the grid is coarsened until it fits.
pub const GRID_MAX_INDICES: usize = 16384;
/// Smallest grid cell edge (world units): finer cells cost more than they save.
const GRID_MIN_CELL: f32 = 32.0;
/// Number of `u32` words in the grid storage buffer.
pub(crate) const GRID_WORDS: usize = (GRID_MAX_DIM * GRID_MAX_DIM) as usize + GRID_MAX_INDICES;

/// A dynamic point light.
///
/// Lights add to the sun and ambient lighting of terrain, units, sprites and water. Their
/// contribution falls off smoothly and reaches exactly zero at `radius`, so a light never
/// affects anything farther away (which is also what makes culling exact).
///
/// Placement tip: lights for ground explosions work best a little above the ground
/// (10–30 units), so flat terrain around them receives light at a useful angle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
    /// World position (x east, y up, z south; 1 unit = 1 map pixel).
    pub position: [f32; 3],
    /// Linear HDR colour. Values above 1 are expected for bright sources, e.g.
    /// `[6.0, 3.2, 1.2]` for a fireball or `[0.5, 2.5, 0.6]` for a green laser glow.
    pub color: [f32; 3],
    /// Influence radius in world units.
    pub radius: f32,
    /// Whether what stands in its way shadows it (ray tracing, see raytrace.rs): not for lights
    /// inside what they light up, such as a nano stream's glow on the building it builds.
    pub shadows: bool,
}

impl PointLight {
    pub fn new(position: [f32; 3], color: [f32; 3], radius: f32) -> Self {
        PointLight { position, color, radius, shadows: true }
    }

    /// The light casts no shadows (see [`PointLight::shadows`]).
    pub fn without_shadows(mut self) -> Self {
        self.shadows = false;
        self
    }

    /// Rough visual significance, used to choose which lights to keep when there are too many.
    fn importance(&self) -> f32 {
        Vec3::from(self.color).max(Vec3::ZERO).dot(Vec3::new(0.2126, 0.7152, 0.0722)) * self.radius
    }
}

/// Keep the same lights, in the same order, as a stable descending importance sort. An
/// army's shells can submit thousands of lights, but only the best 256 need sorting.
fn keep_important(lights: &mut Vec<&PointLight>) {
    if lights.len() <= MAX_LIGHTS {
        return;
    }
    // Equal shell lights (and input already ordered by importance) need no selection. Keep
    // the old stable sort's linear best case, without computing either key twice per pair.
    let mut previous = lights[0].importance();
    if lights[1..].iter().all(|light| {
        let importance = light.importance();
        let ordered = !previous.total_cmp(&importance).is_lt();
        previous = importance;
        ordered
    }) {
        lights.truncate(MAX_LIGHTS);
        return;
    }
    // Cache the dot product once; the input index breaks ties exactly as the stable sort
    // did (including equal infinities from very large, but finite, input colours/radii).
    let mut ranked: Vec<_> = lights.iter().enumerate().map(|(index, &light)| (light.importance(), index, light)).collect();
    let compare = |a: &(f32, usize, &PointLight), b: &(f32, usize, &PointLight)| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1));
    ranked.select_nth_unstable_by(MAX_LIGHTS, compare);
    ranked.truncate(MAX_LIGHTS);
    ranked.sort_unstable_by(compare);
    for (to, (_, _, light)) in lights.iter_mut().zip(ranked) {
        *to = light;
    }
    lights.truncate(MAX_LIGHTS);
}

/// GPU representation (matches `PointLightGpu` in `common.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default, PartialEq)]
pub(crate) struct GpuLight {
    /// xyz = position, w = radius (negative: the light casts no shadows).
    pub pos_radius: [f32; 4],
    /// rgb = colour, w = 1 / radius².
    pub color: [f32; 4],
}

/// Lights binned into a world-space grid, ready for upload.
#[derive(Clone, Debug, Default)]
pub(crate) struct LightGrid {
    /// World xz of the grid's minimum corner.
    pub origin: Vec2,
    /// Cell edge length in world units.
    pub cell: f32,
    /// Cells per axis (zero when there are no lights).
    pub dims: UVec2,
    pub lights: Vec<GpuLight>,
    /// Packed cell ranges followed by light indices (see the module docs).
    pub words: Vec<u32>,
}

impl LightGrid {
    /// Bins `input` for the visible ground rectangle `view_min..view_max` (world xz).
    /// Lights that cannot reach the rectangle are dropped.
    pub fn build(input: &[PointLight], view_min: Vec2, view_max: Vec2) -> LightGrid {
        let mut kept: Vec<&PointLight> = input
            .iter()
            .filter(|l| {
                let p = Vec3::from(l.position);
                let c = Vec3::from(l.color);
                if !(p.is_finite() && c.is_finite() && l.radius.is_finite() && l.radius > 0.0 && c.max_element() > 0.0) {
                    return false;
                }
                let r = l.radius;
                p.x + r >= view_min.x && p.x - r <= view_max.x && p.z + r >= view_min.y && p.z - r <= view_max.y
            })
            .collect();
        keep_important(&mut kept);
        if kept.is_empty() {
            return LightGrid::default();
        }

        let footprint = |l: &PointLight| {
            let r = l.radius;
            (Vec2::new(l.position[0] - r, l.position[2] - r), Vec2::new(l.position[0] + r, l.position[2] + r))
        };
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for l in &kept {
            let (a, b) = footprint(l);
            lo = lo.min(a);
            hi = hi.max(b);
        }
        // Only the visible part needs cells.
        lo = lo.max(view_min);
        hi = hi.min(view_max).max(lo + Vec2::ONE);
        let size = hi - lo;

        // Pick the finest cell size whose index list fits the budget.
        let mut cell = (size.max_element() / GRID_MAX_DIM as f32).max(GRID_MIN_CELL);
        let range = |l: &PointLight, cell: f32, dims: UVec2| {
            let (a, b) = footprint(l);
            let max = dims.as_ivec2() - 1;
            let c0 = ((a - lo) / cell).floor().as_ivec2().clamp(glam::IVec2::ZERO, max);
            let c1 = ((b - lo) / cell).floor().as_ivec2().clamp(glam::IVec2::ZERO, max);
            (c0.as_uvec2(), c1.as_uvec2())
        };
        let dims = loop {
            let dims = (size / cell).ceil().as_uvec2().clamp(UVec2::ONE, UVec2::splat(GRID_MAX_DIM));
            let total: usize = kept
                .iter()
                .map(|l| {
                    let (c0, c1) = range(l, cell, dims);
                    ((c1.x - c0.x + 1) * (c1.y - c0.y + 1)) as usize
                })
                .sum();
            if total <= GRID_MAX_INDICES || dims == UVec2::ONE {
                break dims;
            }
            cell *= 2.0;
        };

        // Counting sort of (cell, light) pairs into per-cell lists.
        let cells = (dims.x * dims.y) as usize;
        let mut counts = vec![0u32; cells];
        for l in &kept {
            let (c0, c1) = range(l, cell, dims);
            for z in c0.y..=c1.y {
                for x in c0.x..=c1.x {
                    counts[(z * dims.x + x) as usize] += 1;
                }
            }
        }
        let mut words = vec![0u32; cells];
        let mut next = Vec::with_capacity(cells);
        let mut offset = cells as u32;
        for (i, &n) in counts.iter().enumerate() {
            words[i] = offset | (n << 16);
            next.push(offset);
            offset += n;
        }
        words.resize(offset as usize, 0);
        for (index, l) in kept.iter().enumerate() {
            let (c0, c1) = range(l, cell, dims);
            for z in c0.y..=c1.y {
                for x in c0.x..=c1.x {
                    let slot = &mut next[(z * dims.x + x) as usize];
                    words[*slot as usize] = index as u32;
                    *slot += 1;
                }
            }
        }

        let lights = kept
            .iter()
            .map(|l| {
                let r = l.radius;
                let c = Vec3::from(l.color).max(Vec3::ZERO);
                GpuLight {
                    pos_radius: [l.position[0], l.position[1], l.position[2], if l.shadows { r } else { -r }],
                    color: [c.x, c.y, c.z, 1.0 / (r * r)],
                }
            })
            .collect();
        LightGrid { origin: lo, cell, dims, lights, words }
    }

    /// Indices of the lights listed for world position `xz` (CPU mirror of the shader lookup).
    #[cfg(test)]
    pub fn lookup(&self, xz: Vec2) -> &[u32] {
        if self.lights.is_empty() {
            return &[];
        }
        let g = (xz - self.origin) / self.cell;
        if g.x < 0.0 || g.y < 0.0 || g.x >= self.dims.x as f32 || g.y >= self.dims.y as f32 {
            return &[];
        }
        let c = g.as_uvec2();
        let packed = self.words[(c.y * self.dims.x + c.x) as usize];
        let (first, count) = ((packed & 0xFFFF) as usize, (packed >> 16) as usize);
        &self.words[first..first + count]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_lights_match_the_previous_stable_sort() {
        for count in [0, 1, MAX_LIGHTS, MAX_LIGHTS + 1, 1000, 4096] {
            let lights: Vec<_> = (0..count).map(|i| {
                let color = match i % 11 {
                    0 => [f32::MAX; 3],
                    1 => [f32::NAN, 1.0, 0.0],
                    2 => [-0.0; 3],
                    _ => [(i * 7919 % 23) as f32, 2.0, 1.0],
                };
                PointLight::new([i as f32, 20.0, 0.0], color, (i % 7 + 1) as f32 * 40.0)
            }).collect();
            let mut expected: Vec<_> = lights.iter().collect();
            if expected.len() > MAX_LIGHTS {
                expected.sort_by(|a, b| b.importance().total_cmp(&a.importance()));
                expected.truncate(MAX_LIGHTS);
            }
            let mut actual: Vec<_> = lights.iter().collect();
            keep_important(&mut actual);
            // Compare identity, since some adversarial values above contain NaN.
            assert_eq!(actual.iter().map(|l| l.position[0]).collect::<Vec<_>>(), expected.iter().map(|l| l.position[0]).collect::<Vec<_>>());
        }
    }

    fn light(x: f32, z: f32, r: f32) -> PointLight {
        PointLight::new([x, 20.0, z], [2.0, 1.0, 0.5], r)
    }

    /// Every (position, light) pair within reach must be found through the grid.
    fn assert_complete(grid: &LightGrid, lights: &[PointLight], view_min: Vec2, view_max: Vec2) {
        let mut probe = 0u32;
        let mut z = view_min.y;
        while z <= view_max.y {
            let mut x = view_min.x;
            while x <= view_max.x {
                let listed = grid.lookup(Vec2::new(x, z));
                for (index, gl) in grid.lights.iter().enumerate() {
                    let d = Vec2::new(gl.pos_radius[0] - x, gl.pos_radius[2] - z).length();
                    if d < gl.pos_radius[3].abs() {
                        assert!(listed.contains(&(index as u32)), "light {index} missing at ({x}, {z})");
                    }
                }
                probe += 1;
                x += 7.3;
            }
            z += 7.3;
        }
        assert!(probe > 100);
        // Every submitted light that reaches the view is present (when under the cap).
        if lights.len() <= MAX_LIGHTS {
            let reaching = lights
                .iter()
                .filter(|l| {
                    l.position[0] + l.radius >= view_min.x
                        && l.position[0] - l.radius <= view_max.x
                        && l.position[2] + l.radius >= view_min.y
                        && l.position[2] - l.radius <= view_max.y
                })
                .count();
            assert_eq!(grid.lights.len(), reaching);
        }
    }

    #[test]
    fn empty_and_invalid_lights_produce_no_grid() {
        let grid = LightGrid::build(&[], Vec2::ZERO, Vec2::splat(1000.0));
        assert!(grid.lights.is_empty() && grid.lookup(Vec2::splat(10.0)).is_empty());
        let bad = [
            PointLight::new([0.0, 0.0, 0.0], [1.0; 3], 0.0),
            PointLight::new([0.0, 0.0, 0.0], [0.0; 3], 50.0),
            PointLight::new([f32::NAN, 0.0, 0.0], [1.0; 3], 50.0),
        ];
        assert!(LightGrid::build(&bad, Vec2::ZERO, Vec2::splat(1000.0)).lights.is_empty());
    }

    #[test]
    fn lights_outside_the_view_are_culled() {
        let lights = [light(100.0, 100.0, 50.0), light(5000.0, 5000.0, 100.0)];
        let grid = LightGrid::build(&lights, Vec2::ZERO, Vec2::splat(1000.0));
        assert_eq!(grid.lights.len(), 1);
        assert_eq!(grid.lookup(Vec2::new(110.0, 90.0)), &[0]);
        // Outside the light's footprint (and thus outside the grid): nothing.
        assert!(grid.lookup(Vec2::new(600.0, 600.0)).is_empty());
    }

    #[test]
    fn grid_lists_every_light_in_reach() {
        let mut lights = Vec::new();
        for i in 0..120 {
            let f = i as f32;
            lights.push(light((f * 173.0) % 2900.0 + 30.0, (f * 97.0) % 1900.0 + 40.0, 20.0 + (f * 13.0) % 180.0));
        }
        let (vmin, vmax) = (Vec2::new(-100.0, -50.0), Vec2::new(3100.0, 2050.0));
        let grid = LightGrid::build(&lights, vmin, vmax);
        assert!(grid.dims.x > 4 && grid.dims.y > 4, "expected a real grid, got {:?}", grid.dims);
        assert!(grid.words.len() <= GRID_WORDS);
        assert_complete(&grid, &lights, vmin, vmax);
    }

    #[test]
    fn huge_lights_coarsen_the_grid_to_fit_the_budget() {
        // 256 lights that each cover the whole view would need 256 × 1024 indices at full
        // resolution; the grid must coarsen instead of overflowing.
        let lights: Vec<PointLight> = (0..300).map(|i| light(500.0 + i as f32, 500.0, 5000.0)).collect();
        let (vmin, vmax) = (Vec2::ZERO, Vec2::splat(1000.0));
        let grid = LightGrid::build(&lights, vmin, vmax);
        assert_eq!(grid.lights.len(), MAX_LIGHTS);
        assert!(grid.words.len() <= GRID_WORDS, "{} words", grid.words.len());
        assert_complete(&grid, &lights, vmin, vmax);
    }

    #[test]
    fn brightest_lights_win_when_over_the_cap() {
        let mut lights: Vec<PointLight> = (0..MAX_LIGHTS).map(|i| light(10.0 * i as f32, 100.0, 30.0)).collect();
        let bright = PointLight::new([50.0, 10.0, 60.0], [50.0, 40.0, 30.0], 200.0);
        lights.push(bright);
        let grid = LightGrid::build(&lights, Vec2::ZERO, Vec2::splat(4000.0));
        assert_eq!(grid.lights.len(), MAX_LIGHTS);
        assert!(grid.lights.iter().any(|l| l.pos_radius == [50.0, 10.0, 60.0, 200.0]));
    }

    /// Over the cap, of lights as bright as each other (a battle's shells) the same ones are kept
    /// frame after frame, those submitted first, whatever else changes: a light kept one frame
    /// and dropped the next blinked on the ground and the hulls round it.
    #[test]
    fn equal_lights_over_the_cap_are_kept_in_their_order() {
        let shells = |extra: usize| -> Vec<PointLight> { (0..MAX_LIGHTS + 40 + extra).map(|i| light(5.0 * i as f32, 300.0, 30.0)).collect() };
        let kept = |lights: &[PointLight]| -> Vec<[f32; 4]> {
            let mut kept: Vec<[f32; 4]> = LightGrid::build(lights, Vec2::ZERO, Vec2::splat(4000.0)).lights.iter().map(|l| l.pos_radius).collect();
            kept.sort_by(|a, b| a[0].total_cmp(&b[0]));
            kept
        };
        let first = kept(&shells(0));
        assert_eq!(first.len(), MAX_LIGHTS);
        assert!(first.iter().all(|p| p[0] < 5.0 * MAX_LIGHTS as f32), "the first submitted are kept");
        for extra in [1, 7, 33] {
            assert_eq!(kept(&shells(extra)), first, "with {extra} more after them");
        }
    }
}
