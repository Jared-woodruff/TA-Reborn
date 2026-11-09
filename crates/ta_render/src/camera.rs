//! Cameras.
//!
//! World space: `x` east, `y` up, `z` south, 1 unit = 1 TA map pixel.
//!
//! TA drew its world with an oblique ("cabinet"-like) projection: the ground plane maps 1:1
//! to screen pixels and height lifts things up the screen by `y / 2`. The painted map art
//! was authored in that projection, which is why the terrain mesh samples its texture at
//! `(x, z - y/2)`: seen through [`CameraMode::Classic`] the 3D terrain reproduces the
//! original image exactly, while [`CameraMode::Perspective`] reveals it as real relief.

use glam::{Mat4, Vec2, Vec3, Vec4, Vec4Swizzles};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode {
    /// Exact TA projection (orthographic ground, height shifted by y/2).
    Classic,
    /// Perspective camera orbiting the focus point.
    Perspective,
}

/// Elevation of TA's implied view direction: moving 2 up and 1 south keeps the screen
/// position fixed, so the camera looks along -(0, 2, 1).
pub const TA_ELEVATION: f32 = 1.107_148_7; // atan(2)

/// How far the classic camera's eye sits from the focus (far enough that nothing is behind it).
const CLASSIC_EYE_DISTANCE: f32 = 4000.0;

/// How far (world units) the perspective camera's eye keeps above the ground around it (see
/// [`Camera::keep_above_ground`]).
pub const EYE_CLEARANCE: f32 = 14.0;

/// How far towards a light a caster may stand from its receiver. The 4000-unit horizontal
/// cap matches the extra caster reach reserved by the renderer's finite shadow depth.
pub fn shadow_caster_offset(light: Vec3, height: f32) -> Vec3 {
    let light = light.normalize_or(Vec3::Y);
    if light.y <= 0.0 {
        return Vec3::ZERO;
    }
    let flat = light.x.hypot(light.z);
    let distance = (height.max(0.0) / light.y.max(1e-4)).min(4000.0 / flat.max(1e-4));
    light * distance
}

/// The steepest the view looks down (radians): a lift never raises it past this.
const MAX_VIEW_PITCH: f32 = 1.55;

#[derive(Clone, Debug)]
pub struct Camera {
    pub mode: CameraMode,
    /// Ground point at the centre of the screen.
    pub focus: Vec3,
    /// Screen pixels per world unit at the focus point.
    pub zoom: f32,
    /// Rotation around the vertical axis (radians, 0 = looking north as in TA).
    pub yaw: f32,
    /// Elevation angle above the horizon (radians).
    pub pitch: f32,
    pub fov_y: f32,
    pub viewport: Vec2,
    /// Roll about the view (radians; positive turns the picture anticlockwise, as the world
    /// looks from a plane banking right). Perspective only; 0 elsewhere.
    pub roll: f32,
    /// How much the view is raised above `pitch` (radians) so the eye clears the ground (see
    /// [`Camera::keep_above_ground`]); the player's pitch stays as chosen. 0 when nothing is
    /// in the way.
    pub lift: f32,
    /// Where the focus sits on the screen: pixels from the window's centre (the middle of the
    /// part of the window an interface leaves to the world). The picture is shifted, not
    /// turned, so whatever the camera centres on lands there.
    pub view_offset: Vec2,
    /// The perspective camera's near plane (world units), when it is set: an eye placed low over
    /// the ground or close to things (see [`Camera::place`]) keeps them in view. `None`: from
    /// the distance and the pitch (see [`Camera::clip_range`]).
    pub near: Option<f32>,
}

impl Camera {
    pub fn new(viewport: Vec2) -> Self {
        Camera {
            mode: CameraMode::Perspective,
            focus: Vec3::ZERO,
            zoom: 1.5,
            yaw: 0.0,
            pitch: TA_ELEVATION,
            fov_y: 30f32.to_radians(),
            viewport,
            roll: 0.0,
            lift: 0.0,
            view_offset: Vec2::ZERO,
            near: None,
        }
    }

    /// Puts the perspective camera's eye at `eye`, looking towards `yaw` (radians, 0 north) and
    /// down at `pitch` (radians below the horizon: -π/2 looks straight up), with its focus
    /// `distance` ahead (where `zoom` is measured, the haze begins, and the depth of field would
    /// focus). Any direction will do: straight up and straight down too. The lift is taken off.
    pub fn place(&mut self, eye: Vec3, yaw: f32, pitch: f32, distance: f32) {
        let half = std::f32::consts::FRAC_PI_2;
        self.mode = CameraMode::Perspective;
        self.yaw = yaw;
        self.pitch = pitch.clamp(-half, half);
        self.lift = 0.0;
        let distance = distance.max(1.0);
        self.zoom = self.viewport.y / (2.0 * (self.fov_y * 0.5).tan() * distance);
        self.focus = eye - self.back_dir() * self.distance();
    }

    /// The screen's right on the ground (unit, horizontal): square to the yaw, so it stays
    /// defined looking straight up or down.
    fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, sy)
    }

    /// Distance from focus for the perspective camera so that the ground scale at the
    /// focus matches `zoom` pixels per unit.
    pub fn distance(&self) -> f32 {
        self.viewport.y / (2.0 * (self.fov_y * 0.5).tan() * self.zoom)
    }

    /// The view's elevation above the horizon (radians): the pitch, raised by the lift.
    pub fn view_pitch(&self) -> f32 {
        self.pitch + self.lift
    }

    /// Unit vector from the focus towards the camera.
    pub fn back_dir(&self) -> Vec3 {
        let (sp, cp) = self.view_pitch().sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(-sy * cp, sp, cy * cp)
    }

    pub fn eye(&self) -> Vec3 {
        match self.mode {
            CameraMode::Perspective => self.focus + self.back_dir() * self.distance(),
            CameraMode::Classic => self.focus + Vec3::new(0.0, 2.0, 1.0).normalize() * CLASSIC_EYE_DISTANCE,
        }
    }

    /// Distance from the eye to the focus, along the view.
    pub fn focus_distance(&self) -> f32 {
        match self.mode {
            CameraMode::Perspective => self.distance(),
            CameraMode::Classic => CLASSIC_EYE_DISTANCE,
        }
    }

    /// Distance from the eye to `p`, along the view (what the depth of field measures).
    pub fn view_distance(&self, p: Vec3) -> f32 {
        (p - self.eye()).dot(self.forward())
    }

    /// How depth as the projection writes it (0..1) maps back to [`Camera::view_distance`]:
    /// `[near, far, 0]` for the perspective camera, `[offset, scale, 1]` (distance = offset +
    /// scale × depth) for the classic one.
    pub fn depth_mapping(&self) -> [f32; 3] {
        match self.mode {
            CameraMode::Perspective => {
                let (near, far) = self.clip_range();
                [near, far, 0.0]
            }
            CameraMode::Classic => {
                // Depth is 0.5 at the focus and falls by 1 / range per unit of 2y + z, which is
                // √5 units along the view.
                let scale = self.classic_depth_range() / 5f32.sqrt();
                [CLASSIC_EYE_DISTANCE - 0.5 * scale, scale, 1.0]
            }
        }
    }

    /// The view (world to eye) of the perspective camera, roll and all. (Its up is square to the
    /// screen's right on the ground and to the view: what a look at the focus with the world's
    /// up gives, and defined looking straight up or down too.)
    pub fn view(&self) -> Mat4 {
        let forward = self.forward();
        let up = self.right().cross(forward).normalize_or(Vec3::Y);
        Mat4::from_rotation_z(self.roll) * glam::camera::rh::view::look_to_mat4(self.eye(), forward, up)
    }

    pub fn view_proj(&self) -> Mat4 {
        let m = match self.mode {
            CameraMode::Perspective => {
                let view = self.view();
                let (near, far) = self.clip_range();
                let aspect = self.viewport.x / self.viewport.y.max(1.0);
                glam::camera::rh::proj::directx::perspective(self.fov_y, aspect, near, far) * view
            }
            CameraMode::Classic => self.classic_matrix(),
        };
        if self.view_offset == Vec2::ZERO {
            return m;
        }
        // (The picture moved after the projection, by `view_offset` in clip space.)
        let shift = Vec3::new(2.0 * self.view_offset.x / self.viewport.x.max(1.0), -2.0 * self.view_offset.y / self.viewport.y.max(1.0), 0.0);
        Mat4::from_translation(shift) * m
    }

    /// Near and far clip distances of the perspective camera. The near plane sits at 5% of the
    /// focus distance, or at half the camera's height above the focus when that is closer (a
    /// camera tilted towards the horizon), so the ground at the bottom of the screen, a few
    /// camera heights away, is never clipped. Unchanged for pitches above ~5.7°.
    pub fn clip_range(&self) -> (f32, f32) {
        let dist = self.distance();
        let height = dist * self.view_pitch().sin().max(0.0);
        let near = self.near.unwrap_or_else(|| (dist * 0.05).min(height * 0.5)).max(1.0);
        (near, dist * 6.0 + 20000.0)
    }

    /// The oblique TA projection: screen = (x, z - y/2) around the focus, scaled by zoom.
    /// Depth runs along -(0, 2, 1) so higher and more southern points are nearer.
    fn classic_matrix(&self) -> Mat4 {
        let sx = 2.0 * self.zoom / self.viewport.x;
        let sy = 2.0 * self.zoom / self.viewport.y;
        let f = self.focus;
        let screen_focus_y = f.z - f.y * 0.5;
        // depth d = k - (2y + z); map a generous range around the focus into [0, 1].
        let range = self.classic_depth_range();
        let center = 2.0 * f.y + f.z;
        let dz = -1.0 / range;
        let dz0 = 0.5 + center / range;
        // Column-major: each Vec4 is a column (coefficient of x, y, z, w).
        Mat4::from_cols(
            Vec4::new(sx, 0.0, 0.0, 0.0),
            Vec4::new(0.0, sy * 0.5, dz * 2.0, 0.0),
            Vec4::new(0.0, -sy, dz, 0.0),
            Vec4::new(-sx * f.x, sy * screen_focus_y, dz0, 1.0),
        )
    }

    /// The span of 2y + z the classic projection maps into depth 0..1, centred on the focus.
    fn classic_depth_range(&self) -> f32 {
        8000.0 + 4.0 * self.viewport.y / self.zoom
    }

    /// World vectors for one sprite pixel right / up on screen. In the classic projection
    /// these make a billboard reproduce TA's flat sprite exactly: +1 px up is (0, 0.4, -0.8),
    /// perpendicular to the view direction (0, 2, 1).
    pub fn billboard_basis(&self) -> (Vec3, Vec3) {
        match self.mode {
            CameraMode::Classic => (Vec3::X, Vec3::new(0.0, 0.4, -0.8)),
            CameraMode::Perspective => {
                let back = self.back_dir();
                let right = self.right();
                let up = back.cross(right).normalize_or(Vec3::Y);
                // Same on-screen size as the classic basis at TA's default elevation.
                (right, up * 0.894)
            }
        }
    }

    /// World-space ray through a screen pixel (origin, normalised direction).
    pub fn screen_ray(&self, screen: Vec2) -> (Vec3, Vec3) {
        let ndc = Vec2::new(screen.x / self.viewport.x * 2.0 - 1.0, 1.0 - screen.y / self.viewport.y * 2.0);
        let inv = self.view_proj().inverse();
        let p0 = inv * Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
        let p1 = inv * Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
        let a = p0.xyz() / p0.w;
        let b = p1.xyz() / p1.w;
        (a, (b - a).normalize())
    }

    /// Moves the focus by a screen-space delta in pixels, keeping the camera height.
    pub fn pan_pixels(&mut self, delta: Vec2) {
        // (The classic view never turns: the top of its screen is always north.)
        let yaw = if self.mode == CameraMode::Classic { 0.0 } else { self.yaw };
        let (sy, cy) = yaw.sin_cos();
        let right = Vec3::new(cy, 0.0, sy);
        let forward = Vec3::new(sy, 0.0, -cy); // screen-up on the ground
        let scale = 1.0 / self.zoom;
        self.focus += (right * delta.x - forward * delta.y) * scale;
    }

    /// Axis-aligned bounds of the visible ground (at heights 0..max_height), used for culling.
    /// At low pitch, where the top of the screen shows the sky, the ground is visible up to
    /// the far plane.
    pub fn ground_bounds(&self, max_height: f32) -> (Vec2, Vec2) {
        self.ground_bounds_with(max_height, true)
    }

    /// Bounds used to select scene objects, including objects outside the picture whose
    /// reflection or shadow can enter it. `max_height` includes terrain and object height;
    /// ordinary effects and HUD culling should continue to use `ground_bounds`.
    pub fn scene_bounds(&self, max_height: f32, mirror: Option<f32>, light: Option<Vec3>) -> (Vec2, Vec2) {
        let (mut min, mut max) = self.ground_bounds(max_height);
        if let Some(sea) = mirror {
            let (lo, hi) = self.slab_bounds([2.0 * sea - max_height, sea], true);
            min = min.min(lo);
            max = max.max(hi);
        }
        if let Some(light) = light {
            let offset = shadow_caster_offset(light, max_height);
            let offset = Vec2::new(offset.x, offset.z);
            min += offset.min(Vec2::ZERO);
            max += offset.max(Vec2::ZERO);
        }
        (min, max)
    }

    /// [`Camera::ground_bounds`], optionally ignoring view rays that point above the horizon
    /// (`sky_to_far` = false: they only contribute their origin, which keeps a shadow map
    /// fitted to the ground near the camera).
    pub(crate) fn ground_bounds_with(&self, max_height: f32, sky_to_far: bool) -> (Vec2, Vec2) {
        self.slab_bounds([0.0, max_height], sky_to_far)
    }

    /// Axis-aligned bounds (xz) of what the view sees between the horizontal planes at
    /// `heights` (below the camera): where the rays through the screen corners cross them, as
    /// in [`Camera::ground_bounds_with`]. A ray rising from an eye within them adds the eye's
    /// place (what stands or flies round an eye looking up is in view).
    pub(crate) fn slab_bounds(&self, heights: [f32; 2], sky_to_far: bool) -> (Vec2, Vec2) {
        let points = self.slab_points(heights, sky_to_far);
        let min = points.iter().fold(Vec2::splat(f32::MAX), |m, &p| m.min(p));
        let max = points.iter().fold(Vec2::splat(f32::MIN), |m, &p| m.max(p));
        (min, max)
    }

    /// The points (xz) [`Camera::slab_bounds`] bounds: where the rays through the screen corners
    /// cross the planes at `heights` (and the eye's place, for a ray rising from within them).
    /// Where the rays through the screen's corners and the middles of its edges meet the planes at
    /// `heights`, on the ground plane (x, z): the view's outline on the ground, as `slab_points`
    /// has it without the far plane (a ray into the sky stops at the eye; one running towards
    /// the horizon meets the ground far off), with the edges' middles (cut short, a view to the
    /// horizon bulges there).
    pub(crate) fn outline_points(&self, heights: [f32; 2]) -> Vec<Vec2> {
        let (w, h) = (self.viewport.x, self.viewport.y);
        let mut points = Vec::with_capacity(16);
        for screen in [Vec2::ZERO, Vec2::new(w * 0.5, 0.0), Vec2::new(w, 0.0), Vec2::new(w, h * 0.5), Vec2::new(w, h), Vec2::new(w * 0.5, h), Vec2::new(0.0, h), Vec2::new(0.0, h * 0.5)] {
            let (o, d) = self.screen_ray(screen);
            for y in heights {
                let t = if d.y.abs() > 1e-4 { (y - o.y) / d.y } else { 1e6 };
                let p = o + d * t.clamp(0.0, 1e6);
                points.push(Vec2::new(p.x, p.z));
            }
        }
        points
    }

    pub(crate) fn slab_points(&self, heights: [f32; 2], sky_to_far: bool) -> Vec<Vec2> {
        let mut points = Vec::with_capacity(36);
        let mut near_corners = [Vec3::ZERO; 4];
        let mut far_corners = [Vec3::ZERO; 4];
        for (i, corner) in [Vec2::ZERO, Vec2::new(self.viewport.x, 0.0), self.viewport, Vec2::new(0.0, self.viewport.y)].into_iter().enumerate() {
            let (o, d) = self.screen_ray(corner);
            near_corners[i] = o;
            far_corners[i] = match self.mode {
                // Work in view distance: unprojecting depth 1 with a very small near plane
                // loses precision, and the corners are farther from the eye than the centre.
                CameraMode::Perspective => self.eye() + d * (self.clip_range().1 / d.dot(self.forward()).max(1e-3)),
                CameraMode::Classic => o + d * (self.classic_depth_range() / 5f32.sqrt()),
            };
            if sky_to_far && d.y > -1e-4 && o.y < heights[0].max(heights[1]) {
                points.push(Vec2::new(o.x, o.z));
            }
            for h in heights {
                // Intersect with plane y = h; rays nearly parallel get clamped far away.
                let mut t = if d.y.abs() > 1e-4 { (h - o.y) / d.y } else { 1e6 };
                if sky_to_far && self.mode == CameraMode::Perspective && d.y > -1e-4 {
                    // The ray never comes down to the plane: ground is visible below it up to
                    // the far plane.
                    let (_, far) = self.clip_range();
                    t = far / d.dot(self.forward()).max(1e-3);
                }
                let p = o + d * t.clamp(0.0, 1e6);
                points.push(Vec2::new(p.x, p.z));
            }
        }
        if sky_to_far {
            // The slab can meet the near/far plane between screen corners, particularly
            // across a horizon or a rolled view. Corner rays alone miss these vertices and
            // can cull visible terrain and features. Keep the existing conservative ray
            // bounds, and include the two plane outlines clipped to the height slab too.
            let (low, high) = (heights[0].min(heights[1]), heights[0].max(heights[1]));
            for corners in [near_corners, far_corners] {
                for i in 0..4 {
                    let a = corners[i];
                    let b = corners[(i + 1) % 4];
                    if a.y >= low && a.y <= high {
                        points.push(Vec2::new(a.x, a.z));
                    }
                    if (b.y - a.y).abs() > 1e-6 {
                        for height in heights {
                            let t = (height - a.y) / (b.y - a.y);
                            if (0.0..=1.0).contains(&t) {
                                let p = a.lerp(b, t);
                                points.push(Vec2::new(p.x, p.z));
                            }
                        }
                    }
                }
            }
        }
        points
    }

    /// Keeps the perspective camera's eye above the ground (`ground(x, z)`: its height there,
    /// water counted as the ground it covers): the view is raised (`lift`) just enough that the
    /// ground under the eye and around it stays [`EYE_CLEARANCE`] below it, and when the ground
    /// falls away again the lift eases off over a second or so, back to the pitch the player
    /// chose. A camera low over a plateau or a pit looks down a little more instead of from
    /// inside the rock. (The classic camera has nothing to clear.)
    pub fn keep_above_ground(&mut self, ground: impl Fn(f32, f32) -> f32, dt: f32) {
        if self.mode != CameraMode::Perspective {
            self.lift = 0.0;
            return;
        }
        let clear = |lift: f32| {
            let mut c = self.clone();
            c.lift = lift;
            let eye = c.eye();
            // Around the eye as far as its near plane reaches, and a little way ahead.
            let r = (c.distance() * 0.05).max(24.0);
            let ahead = -c.back_dir();
            let flat = Vec2::new(ahead.x, ahead.z).normalize_or_zero();
            let mut highest = ground(eye.x, eye.z).max(ground(eye.x + flat.x * r * 2.0, eye.z + flat.y * r * 2.0));
            for k in 0..8 {
                let (s, c) = (k as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
                highest = highest.max(ground(eye.x + c * r, eye.z + s * r));
            }
            eye.y >= highest + EYE_CLEARANCE
        };
        let most = (MAX_VIEW_PITCH - self.pitch).max(0.0);
        let needed = if clear(0.0) {
            0.0
        } else if !clear(most) {
            most
        } else {
            // The smallest lift that clears (the eye only rises as the lift grows).
            let (mut lo, mut hi) = (0.0f32, most);
            for _ in 0..14 {
                let mid = 0.5 * (lo + hi);
                if clear(mid) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            hi
        };
        self.lift = if needed >= self.lift { needed } else { self.lift + (needed - self.lift) * (1.0 - (-dt * 2.5).exp()) };
    }

    /// Unit view direction (from the eye towards the focus).
    fn forward(&self) -> Vec3 {
        match self.mode {
            CameraMode::Perspective => -self.back_dir(),
            CameraMode::Classic => -Vec3::new(0.0, 2.0, 1.0).normalize(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_screen(cam: &Camera, p: Vec3) -> Vec2 {
        let c = cam.view_proj() * p.extend(1.0);
        let ndc = c.xy() / c.w;
        Vec2::new((ndc.x + 1.0) * 0.5 * cam.viewport.x, (1.0 - ndc.y) * 0.5 * cam.viewport.y)
    }

    #[test]
    fn depth_maps_back_to_the_distance_along_the_view() {
        for mode in [CameraMode::Perspective, CameraMode::Classic] {
            let mut cam = Camera::new(Vec2::new(1280.0, 720.0));
            cam.mode = mode;
            cam.focus = Vec3::new(900.0, 40.0, 700.0);
            cam.zoom = 0.8;
            cam.yaw = 0.4;
            let [a, b, linear] = cam.depth_mapping();
            for p in [cam.focus, Vec3::new(700.0, 0.0, 400.0), Vec3::new(1200.0, 120.0, 1000.0)] {
                let c = cam.view_proj() * p.extend(1.0);
                let depth = c.z / c.w;
                let mapped = if linear > 0.5 { a + b * depth } else { a * b / (b - depth * (b - a)) };
                let expected = cam.view_distance(p);
                assert!((mapped - expected).abs() < expected * 1e-3 + 0.5, "{mode:?} {p}: {mapped} vs {expected}");
            }
            assert!((cam.view_distance(cam.focus) - cam.focus_distance()).abs() < 0.5, "{mode:?}");
        }
    }

    #[test]
    fn classic_projection_matches_ta() {
        let mut cam = Camera::new(Vec2::new(800.0, 600.0));
        cam.mode = CameraMode::Classic;
        cam.zoom = 1.0;
        cam.focus = Vec3::new(1000.0, 0.0, 1000.0);
        // The focus is at the screen centre; a point 100 units up appears 50 px higher.
        let c = to_screen(&cam, cam.focus);
        assert!((c - Vec2::new(400.0, 300.0)).length() < 1e-3);
        let up = to_screen(&cam, cam.focus + Vec3::new(0.0, 100.0, 0.0));
        assert!((up - Vec2::new(400.0, 250.0)).length() < 1e-3);
        let east = to_screen(&cam, cam.focus + Vec3::new(10.0, 0.0, 20.0));
        assert!((east - Vec2::new(410.0, 320.0)).length() < 1e-3);
    }

    #[test]
    fn classic_scrolling_ignores_a_camera_turned_before() {
        // Turned with the modern camera, then switched to the classic one: the arrows still
        // scroll the classic screen's way (right is east, up is north).
        let mut cam = Camera::new(Vec2::new(800.0, 600.0));
        cam.yaw = 1.2;
        cam.mode = CameraMode::Classic;
        cam.zoom = 2.0;
        cam.focus = Vec3::ZERO;
        cam.pan_pixels(Vec2::new(10.0, 0.0));
        assert!((cam.focus - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-4, "{:?}", cam.focus);
        cam.pan_pixels(Vec2::new(0.0, -10.0));
        assert!((cam.focus - Vec3::new(5.0, 0.0, -5.0)).length() < 1e-4, "{:?}", cam.focus);
    }

    #[test]
    fn classic_depth_orders_higher_points_nearer() {
        let mut cam = Camera::new(Vec2::new(800.0, 600.0));
        cam.mode = CameraMode::Classic;
        let m = cam.view_proj();
        let low = m * Vec4::new(0.0, 0.0, 0.0, 1.0);
        let high = m * Vec4::new(0.0, 50.0, 0.0, 1.0);
        let south = m * Vec4::new(0.0, 0.0, 50.0, 1.0);
        assert!(high.z < low.z && south.z < low.z);
        assert!(low.z > 0.0 && low.z < 1.0);
    }

    #[test]
    fn perspective_focus_is_centred() {
        let cam = Camera::new(Vec2::new(1280.0, 720.0));
        let c = to_screen(&cam, cam.focus);
        assert!((c - Vec2::new(640.0, 360.0)).length() < 0.01);
    }

    #[test]
    fn the_focus_sits_where_the_view_offset_puts_it() {
        for mode in [CameraMode::Perspective, CameraMode::Classic] {
            let mut cam = Camera::new(Vec2::new(1280.0, 720.0));
            cam.mode = mode;
            cam.focus = Vec3::new(900.0, 40.0, 700.0);
            cam.view_offset = Vec2::new(141.0, 17.0);
            let c = to_screen(&cam, cam.focus);
            assert!((c - Vec2::new(781.0, 377.0)).length() < 0.05, "{mode:?}: {c}");
            // Picking agrees: the ray through that pixel passes through the focus.
            let (o, d) = cam.screen_ray(c);
            let to = cam.focus - o;
            assert!((to - d * to.dot(d)).length() < 0.05, "{mode:?}");
        }
    }

    #[test]
    fn ground_below_the_view_is_never_behind_the_near_plane() {
        for pitch in [0.3f32, 0.55, 2.0, 5.7, 12.0, 25.0, 63.4, 85.0] {
            for zoom in [0.08f32, 0.6, 1.6, 8.0] {
                let mut cam = Camera::new(Vec2::new(1600.0, 900.0));
                cam.focus = Vec3::new(3000.0, 86.0, 300.0);
                cam.pitch = pitch.to_radians();
                cam.zoom = zoom;
                let (near, _) = cam.clip_range();
                let forward = (cam.focus - cam.eye()).normalize();
                // Where the bottom row of the screen meets flat ground at the focus height.
                for x in [0.0, 800.0, 1600.0] {
                    let (o, d) = cam.screen_ray(Vec2::new(x, 900.0));
                    let hit = o + d * ((cam.focus.y - o.y) / d.y);
                    let depth = (hit - cam.eye()).dot(forward);
                    assert!(depth > near * 1.5, "pitch {pitch} zoom {zoom}: ground at depth {depth}, near plane {near}");
                }
            }
        }
        // Normal pitches keep the old near plane.
        let cam = Camera::new(Vec2::new(1600.0, 900.0));
        assert_eq!(cam.clip_range().0, cam.distance() * 0.05);
    }

    #[test]
    fn the_eye_stays_above_the_ground() {
        // Low over a pit: the focus on its floor (height 20), ringed by a plateau at 150 that
        // starts 100 units out; the player's 8° pitch would put the eye inside the rock.
        let ground = |x: f32, z: f32| if Vec2::new(x - 1000.0, z - 1000.0).length() < 100.0 { 20.0 } else { 150.0 };
        let mut cam = Camera::new(Vec2::new(2560.0, 1440.0));
        cam.focus = Vec3::new(1000.0, 20.0, 1000.0);
        cam.zoom = 7.0;
        cam.pitch = 8f32.to_radians();
        let eye = cam.eye();
        assert!(eye.y < ground(eye.x, eye.z), "the eye starts in the rock");
        cam.keep_above_ground(ground, 1.0 / 60.0);
        let eye = cam.eye();
        assert!(eye.y >= ground(eye.x, eye.z) + EYE_CLEARANCE - 0.5, "lifted clear: {eye}");
        assert!(cam.lift > 0.0 && (cam.pitch - 8f32.to_radians()).abs() < 1e-6, "the player's pitch is kept");
        // Out over open ground the lift eases off, back to the pitch chosen.
        let open = |_: f32, _: f32| 0.0;
        for _ in 0..600 {
            cam.keep_above_ground(open, 1.0 / 60.0);
        }
        assert!(cam.lift < 1e-3, "{}", cam.lift);
        // The classic camera has nothing to clear.
        cam.mode = CameraMode::Classic;
        cam.lift = 0.3;
        cam.keep_above_ground(ground, 1.0 / 60.0);
        assert_eq!(cam.lift, 0.0);
    }

    #[test]
    fn a_camera_placed_anywhere_looks_where_it_is_told() {
        // Level, looking up (a worm's-eye view), straight up, down, straight down; turned and
        // rolled; at a lens of 30° or 70°.
        for pitch in [0.0f32, -35.0, -90.0, 40.0, 90.0] {
            for yaw in [0.0f32, 133.0] {
                for fov in [30.0f32, 70.0] {
                    let mut cam = Camera::new(Vec2::new(1920.0, 1080.0));
                    cam.fov_y = fov.to_radians();
                    cam.roll = 0.1;
                    let eye = Vec3::new(1200.0, 6.0, 800.0);
                    cam.place(eye, yaw.to_radians(), pitch.to_radians(), 250.0);
                    assert!((cam.eye() - eye).length() < 0.01, "pitch {pitch} yaw {yaw}: the eye at {}", cam.eye());
                    assert!((cam.distance() - 250.0).abs() < 0.01);
                    let m = cam.view_proj();
                    assert!(m.to_cols_array().iter().all(|v| v.is_finite()), "pitch {pitch}: a finite view");
                    // What lies straight ahead is at the middle of the picture (rolled or not).
                    let ahead = eye + (cam.focus - eye).normalize() * 600.0;
                    let c = to_screen(&cam, ahead);
                    assert!((c - Vec2::new(960.0, 540.0)).length() < 0.05, "pitch {pitch} yaw {yaw}: {c}");
                    // Billboards face the eye.
                    let (r, u) = cam.billboard_basis();
                    let f = cam.focus - eye;
                    assert!(r.dot(f).abs() < 1e-3 * f.length() && u.dot(f).abs() < 1e-3 * f.length(), "pitch {pitch}");
                }
            }
        }
        // Looking straight up facing north (yaw 0, unrolled), the top of the picture is south:
        // the head tipped back.
        let mut cam = Camera::new(Vec2::new(1920.0, 1080.0));
        cam.place(Vec3::new(0.0, 5.0, 0.0), 0.0, -std::f32::consts::FRAC_PI_2, 100.0);
        let top = to_screen(&cam, Vec3::new(0.0, 300.0, 50.0));
        assert!(top.y < 540.0 && (top.x - 960.0).abs() < 1.0, "south of overhead is up the picture: {top}");
        // A near plane set keeps what is close to the lens.
        cam.near = Some(2.0);
        assert_eq!(cam.clip_range().0, 2.0);
    }

    #[test]
    fn bounds_cover_what_a_camera_looking_up_sees() {
        // An eye 8 units over the ground looking up 40° to the north-east, at a lens of 70°: an
        // aircraft overhead in view, and a tower 150 units ahead.
        let mut cam = Camera::new(Vec2::new(2560.0, 1440.0));
        cam.fov_y = 70f32.to_radians();
        let eye = Vec3::new(3000.0, 8.0, 3000.0);
        cam.place(eye, 45f32.to_radians(), -40f32.to_radians(), 300.0);
        let (min, max) = cam.ground_bounds(260.0);
        let inside = |p: Vec3| Vec2::new(p.x, p.z).cmpge(min - 1.0).all() && Vec2::new(p.x, p.z).cmple(max + 1.0).all();
        let forward = (cam.focus - eye).normalize();
        for d in [20.0f32, 120.0, 400.0, 3000.0] {
            assert!(inside(eye + forward * d), "{d} along the view");
        }
        assert!(inside(eye), "round the eye");
        // And the shadows are fitted round what it sees close to it, not round nothing.
        let (smin, smax) = cam.ground_bounds_with(200.0, false);
        assert!((smax - smin).length() > 50.0 && Vec2::new(eye.x, eye.z).cmpge(smin - 1.0).all() && Vec2::new(eye.x, eye.z).cmple(smax + 1.0).all());
    }

    #[test]
    fn ground_bounds_cover_the_view_at_any_pitch() {
        for pitch in [5.0f32, 12.0, 25.0, 63.4, 85.0] {
            let mut cam = Camera::new(Vec2::new(1600.0, 900.0));
            cam.focus = Vec3::new(5000.0, 40.0, 5000.0);
            cam.pitch = pitch.to_radians();
            cam.yaw = 0.7;
            let (min, max) = cam.ground_bounds(260.0);
            let inside = |p: Vec2| p.cmpge(min - 1.0).all() && p.cmple(max + 1.0).all();
            // The focus, and every visible ground point along the top and bottom screen edges.
            assert!(inside(Vec2::new(cam.focus.x, cam.focus.z)), "focus at pitch {pitch}");
            for x in [0.0, 400.0, 800.0, 1200.0, 1600.0] {
                for y in [0.0, 450.0, 900.0] {
                    let (o, d) = cam.screen_ray(Vec2::new(x, y));
                    if d.y < -1e-3 {
                        let hit = o + d * ((40.0 - o.y) / d.y);
                        // Only ground in front of the far plane can be visible.
                        if hit.distance(o) < cam.clip_range().1 * 0.99 {
                            assert!(inside(Vec2::new(hit.x, hit.z)), "pixel ({x}, {y}) at pitch {pitch}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn slab_bounds_cover_rolled_and_offset_views() {
        for pitch in [-70.0f32, -35.0, -5.0, 0.0, 5.0, 25.0, 70.0] {
            for roll in [0.0f32, 20.0, 65.0, 110.0] {
                for yaw in [0.0f32, 30.0, 110.0] {
                    let mut cam = Camera::new(Vec2::new(1920.0, 1080.0));
                    cam.fov_y = 70f32.to_radians();
                    cam.roll = roll.to_radians();
                    cam.view_offset = Vec2::new(180.0, -70.0);
                    cam.near = Some(1.0);
                    cam.place(Vec3::new(3000.0, 120.0, 3000.0), yaw.to_radians(), pitch.to_radians(), 500.0);
                    let (min, max) = cam.slab_bounds([0.0, 260.0], true);
                    for ix in 0..=32 {
                        for iy in 0..=18 {
                            let (o, d) = cam.screen_ray(Vec2::new(ix as f32 * 60.0, iy as f32 * 60.0));
                            for height in [0.0, 130.0, 260.0] {
                                if d.y.abs() < 1e-5 { continue; }
                                let t = (height - o.y) / d.y;
                                if t < 0.0 { continue; }
                                let p = o + d * t;
                                if cam.view_distance(p) > cam.clip_range().1 { continue; }
                                assert!(p.x >= min.x - 1.0 && p.x <= max.x + 1.0 && p.z >= min.y - 1.0 && p.z <= max.y + 1.0,
                                    "pitch {pitch}, roll {roll}, yaw {yaw}, pixel ({ix}, {iy}), height {height}: {p} outside {min}..{max}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn scene_bounds_include_offscreen_reflection_sources() {
        let mut cam = Camera::new(Vec2::new(1920.0, 1080.0));
        cam.focus = Vec3::new(3000.0, 0.0, 3000.0);
        cam.zoom = 4.0;
        let (old_min, old_max) = cam.ground_bounds(260.0);
        let (min, max) = cam.scene_bounds(500.0, Some(0.0), None);
        let mut outside_old = 0;
        for ix in 0..=16 {
            for iy in 0..=9 {
                let (o, d) = cam.screen_ray(Vec2::new(ix as f32 * 120.0, iy as f32 * 120.0));
                let reflection = o + d * ((-400.0 - o.y) / d.y);
                let source = Vec2::new(reflection.x, reflection.z);
                if !source.cmpge(old_min - 96.0).all() || !source.cmple(old_max + 96.0).all() {
                    outside_old += 1;
                }
                assert!(source.cmpge(min - 1.0).all() && source.cmple(max + 1.0).all(), "reflected source {source}");
            }
        }
        assert!(outside_old > 0, "the fixture must cover reflected objects missed by ground-only selection");
    }

    #[test]
    fn scene_bounds_include_low_sun_casters_and_bound_horizon_reach() {
        let mut cam = Camera::new(Vec2::new(1920.0, 1080.0));
        cam.focus = Vec3::new(3000.0, 0.0, 3000.0);
        cam.zoom = 4.0;
        let light = Vec3::new(1.0, 0.1, 0.0).normalize();
        let (old_min, old_max) = cam.ground_bounds(500.0);
        let (min, max) = cam.scene_bounds(500.0, None, Some(light));
        let caster = cam.focus + shadow_caster_offset(light, 150.0);
        assert!(caster.x > old_max.x + 96.0, "caster must be outside the old feature selection");
        assert!(caster.x <= max.x && caster.z >= min.y && caster.z <= max.y);
        assert_eq!(old_min, min, "only the direction towards the light is extended");
        for y in [0.0, 1e-8, 0.001, 0.1, 1.0] {
            let offset = shadow_caster_offset(Vec3::new(1.0, y, 0.0), 500.0);
            assert!(offset.is_finite() && offset.x <= 4000.01);
        }
    }
}
