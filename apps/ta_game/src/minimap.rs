//! Minimap: the whole map drawn from its own art at the size shown (`ta_render::map_preview`,
//! on a worker; TA's stored minimap stands in for the moment it takes), with unit dots, the
//! camera footprint, and click-to-look / right-click-to-order interaction.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2 as EVec2};
use glam::Vec2;
use ta_render::Camera;
use ta_sim::Fx;

use crate::game::Game;
use crate::palette::team_color;

/// Most texels a side of the minimap's fog (TA's maps have a few hundred sight cells a side).
const FOG_MAX_SIDE: usize = 1024;

#[derive(Default)]
pub struct Minimap {
    picture: Option<Picture>,
    /// Fog overlay and the vision version it was built from.
    fog: Option<(u64, egui::TextureHandle)>,
    nuclear_tracks: NuclearTracks,
}

/// How long an observed nuclear flight's trail lasts on the minimap: older stretches fade out
/// over this (game seconds; the whole of it is kept while the missile is tracked).
const TRAIL_SECONDS: f32 = 10.0;
/// Ticks between a trail's points.
const SAMPLE_TICKS: u64 = 3;
/// How long the place a tracked missile ended (went off, or was shot down) stays marked (s).
const ENDING_SECONDS: f32 = 2.5;

/// Only observed positions are kept: radar must not disclose the launch point or intended
/// destination of an enemy missile. Lost contacts lose their trails; a missile tracked to its
/// end leaves a brief mark where it ended. Where a missile is going is told to its own side only
/// ([`Bound`]).
#[derive(Default)]
struct NuclearTracks {
    paths: rustc_hash::FxHashMap<u32, NuclearTrack>,
    /// Where tracked missiles ended, and the tick.
    endings: Vec<([f32; 2], u64)>,
    tick: u64,
    /// Effective player vision and whether radar reveal overrides its detection limits.
    view: Option<(Option<u8>, bool)>,
}

struct NuclearTrack {
    /// The places (x, z) it was seen at, and the tick of each, oldest first.
    points: std::collections::VecDeque<([f32; 2], u64)>,
    current: [f32; 2],
    bound: Option<Bound>,
}

/// Where a missile is going, for its own side (`nuclear::destination_shown`): the place (x, z),
/// its blast's radius and the seconds until it comes down.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bound {
    at: [f32; 2],
    radius: f32,
    seconds: f32,
}

/// A missile in sight or on radar now: which, where (x, z) and, for its own side, where to.
struct Sighting {
    id: u32,
    at: [f32; 2],
    bound: Option<Bound>,
}

impl NuclearTracks {
    /// `gone`: whether a missile is no longer in flight at all (it went off, or was shot down),
    /// not only out of sight and radar.
    fn update(&mut self, tick: u64, view: (Option<u8>, bool), visible: impl Iterator<Item = Sighting>, gone: impl Fn(u32) -> bool) {
        if tick < self.tick || self.view != Some(view) {
            // A different perspective must not inherit positions it never observed,
            // even if the missile remains detectable and the simulation is paused.
            self.paths.clear();
            self.endings.clear();
        }
        self.tick = tick;
        self.view = Some(view);
        let keep = (TRAIL_SECONDS * 30.0) as u64;
        let mut active = rustc_hash::FxHashSet::default();
        for s in visible {
            active.insert(s.id);
            let path = self.paths.entry(s.id).or_insert_with(|| NuclearTrack { points: Default::default(), current: s.at, bound: None });
            if path.points.back().is_none_or(|&(at, t)| tick >= t + SAMPLE_TICKS && at != s.at) {
                path.points.push_back((s.at, tick));
            }
            while path.points.front().is_some_and(|&(_, t)| tick > t + keep) {
                path.points.pop_front();
            }
            path.current = s.at;
            path.bound = s.bound;
        }
        let ended: Vec<[f32; 2]> = self.paths.iter().filter(|(id, _)| !active.contains(*id) && gone(**id)).map(|(_, p)| p.current).collect();
        self.endings.extend(ended.into_iter().map(|at| (at, tick)));
        self.paths.retain(|id, _| active.contains(id));
        let shown = (ENDING_SECONDS * 30.0) as u64;
        self.endings.retain(|&(_, t)| tick <= t + shown);
    }
}

const NUCLEAR_YELLOW: Color32 = Color32::from_rgb(255, 225, 50);
const IMPACT_RED: Color32 = Color32::from_rgb(255, 80, 55);

/// A dashed line from `a` to `b`: 3 points drawn, 3 left out.
fn dashed(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    let length = a.distance(b);
    let dir = (b - a) / length.max(1e-3);
    let mut s = 0.0;
    while s < length {
        painter.line_segment([a + dir * s, a + dir * (s + 3.0).min(length)], stroke);
        s += 6.0;
    }
}

/// A small radiation trefoil built from geometry, independent of font glyph support.
fn nuclear_symbol(painter: &egui::Painter, centre: Pos2, radius: f32) {
    painter.circle_filled(centre, radius + 1.2, Color32::from_black_alpha(225));
    let mut mesh = egui::Mesh::default();
    for lobe in 0..3 {
        let base = mesh.vertices.len() as u32;
        for step in 0..=10 {
            let angle = (-90.0 + lobe as f32 * 120.0 + step as f32 * 6.0).to_radians();
            let direction = EVec2::angled(angle);
            mesh.colored_vertex(centre + direction * radius * 0.38, NUCLEAR_YELLOW);
            mesh.colored_vertex(centre + direction * radius, NUCLEAR_YELLOW);
            if step > 0 {
                let j = base + step * 2;
                mesh.add_triangle(j - 2, j - 1, j + 1);
                mesh.add_triangle(j - 2, j + 1, j);
            }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.circle_filled(centre, radius * 0.22, NUCLEAR_YELLOW);
}

/// The map's picture: which map (its name and size) and at how many pixels along its longer
/// side, the ground it shows (world pixels from the map's top left: TA's minimap leaves out the
/// last rows and columns), and the drawing of it on its way.
struct Picture {
    map: (String, u32, u32, usize),
    pixels: u32,
    texture: egui::TextureHandle,
    area: (f32, f32),
    drawing: Option<std::sync::mpsc::Receiver<ta_render::map_preview::MapPicture>>,
}

impl Minimap {
    /// The map's picture for showing `pixels` along its longer side, and the ground it shows.
    fn texture(&mut self, ctx: &egui::Context, game: &Game, pixels: u32) -> (egui::TextureId, (f32, f32)) {
        let map = (game.setup.map.clone(), game.map.width, game.map.height, game.map.tiles.len());
        if self.picture.as_ref().is_none_or(|p| p.map != map) {
            self.nuclear_tracks = NuclearTracks::default();
            // A new map: TA's minimap until the drawing comes.
            let mm = &game.map.minimap;
            let rgba = mm.to_rgba(&game.sim.data.palette);
            let img = egui::ColorImage::from_rgba_unmultiplied([mm.width.max(1) as usize, mm.height.max(1) as usize], &rgba);
            let (aw, ah) = game.map.minimap_area_px();
            self.picture = Some(Picture { map, pixels: 0, texture: ctx.load_texture("minimap", img, egui::TextureOptions::LINEAR), area: (aw as f32, ah as f32), drawing: None });
        }
        let picture = self.picture.as_mut().unwrap();
        // Shown at another size (or not drawn yet): drawn afresh, the picture as it is meanwhile.
        if picture.pixels != pixels {
            picture.pixels = pixels;
            let (send, drawing) = std::sync::mpsc::channel();
            let (art, palette, repaint) = (game.map.clone(), game.sim.data.palette.clone(), ctx.clone());
            rayon::spawn(move || {
                if send.send(ta_render::map_preview::render_whole(&art, &palette, pixels)).is_ok() {
                    repaint.request_repaint();
                }
            });
            picture.drawing = Some(drawing);
        }
        if let Some(p) = picture.drawing.as_ref().and_then(|d| d.try_recv().ok()) {
            picture.texture.set(egui::ColorImage::from_rgba_unmultiplied([p.width, p.height], &p.rgba), egui::TextureOptions::LINEAR);
            picture.area = p.area;
            picture.drawing = None;
        }
        (picture.texture.id(), picture.area)
    }

    /// Darkens ground outside the viewer's sight (and more so where never explored).
    fn fog(&mut self, ctx: &egui::Context, game: &Game) -> Option<egui::TextureId> {
        let player = game.viewer()?;
        if !game.sim.los_enabled {
            return None;
        }
        let v = &game.sim.vision;
        let version = v.version ^ ((player as u64) << 56);
        if self.fog.as_ref().is_none_or(|(ver, _)| *ver != version) {
            let cells = v.player_cells(game.sim.vision_slot(player));
            // (On a huge map a texel stands for a block of cells: the minimap is a few hundred
            // points across, and the picture goes up again whenever the sight changes.)
            let step = v.width.max(v.height).div_ceil(FOG_MAX_SIDE).max(1);
            let (fw, fh) = (v.width.div_ceil(step), v.height.div_ceil(step));
            let mut rgba = Vec::with_capacity(fw * fh * 4);
            for z in 0..fh {
                for x in 0..fw {
                    let c = cells.get((z * step).min(v.height - 1) * v.width + (x * step).min(v.width - 1)).copied().unwrap_or(0);
                    let a = if c & ta_sim::vision::LOS != 0 {
                        0
                    } else if c & ta_sim::vision::EXPLORED != 0 {
                        110
                    } else {
                        185
                    };
                    // (Out of sight but in range of a radar or sonar: a little lighter, as the
                    // battlefield shows it, and no tint.)
                    let a = if a > 0 && c & (ta_sim::vision::RADAR | ta_sim::vision::SONAR) != 0 { a - a / 5 } else { a };
                    rgba.extend_from_slice(&[0, 0, 0, a]);
                }
            }
            let img = egui::ColorImage::from_rgba_unmultiplied([fw, fh], &rgba);
            match &mut self.fog {
                Some((ver, tex)) => {
                    tex.set(img, egui::TextureOptions::LINEAR);
                    *ver = version;
                }
                None => self.fog = Some((version, ctx.load_texture("minimap-fog", img, egui::TextureOptions::LINEAR))),
            }
        }
        self.fog.as_ref().map(|(_, t)| t.id())
    }

    /// Draws the minimap inside `ui` at `size` points wide and handles interaction.
    /// Draws the minimap `width` wide, or narrower to keep within `max_height` (a tall map, a
    /// short window), centred in the width. Returns where it is.
    pub fn draw(&mut self, ui: &mut egui::Ui, game: &mut Game, camera: &mut Camera, width: f32, max_height: f32) -> egui::Rect {
        let map_px = Vec2::new(game.map.width_px() as f32, game.map.height_px() as f32);
        let aspect = map_px.y / map_px.x.max(1.0);
        let fit = (max_height / (width * aspect)).min(1.0);
        let size = EVec2::new(width * fit, width * aspect * fit).round();
        let (outer, _) = ui.allocate_exact_size(EVec2::new(width, size.y), Sense::hover());
        let rect = egui::Rect::from_center_size(outer.center(), size);
        let response = ui.interact(rect, ui.id().with("minimap"), Sense::click_and_drag());
        // (Drawn at the size shown, to a few pixels.)
        let pixels = ((rect.width().max(rect.height()) * ui.ctx().pixels_per_point() / 4.0).round() as u32 * 4).max(16);
        let (tex, (aw, ah)) = self.texture(ui.ctx(), game, pixels);
        let painter = ui.painter_at(rect);
        // (TA's picture, while it stands in, stops short of the right and bottom edges: its last
        // pixels, the texture clamping, carry on over the rest.)
        painter.image(tex, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(map_px.x / aw, map_px.y / ah)), Color32::WHITE);
        if let Some(fog) = self.fog(ui.ctx(), game) {
            // The vision grid covers whole 32 px cells, slightly past the map edge.
            let v = &game.sim.vision;
            let uv = Pos2::new(map_px.x / (v.width as f32 * 32.0), map_px.y / (v.height as f32 * 32.0));
            painter.image(fog, rect, Rect::from_min_max(Pos2::ZERO, uv), Color32::WHITE);
        }
        let to_screen = |x: f32, z: f32| Pos2::new(rect.min.x + x / map_px.x * rect.width(), rect.min.y + z / map_px.y * rect.height());
        let to_world = |p: Pos2| Vec2::new((p.x - rect.min.x) / rect.width() * map_px.x, (p.y - rect.min.y) / rect.height() * map_px.y);

        // Units: buildings as squares, mobile units as dots; selected units outlined. One mark of
        // a kind and colour to a square point of the map (in an army the rest would only cover
        // it: twenty thousand shapes a frame otherwise).
        let selected: rustc_hash::FxHashSet<ta_sim::UnitId> = game.selection.iter().copied().collect();
        let (cols, rows) = (rect.width().ceil().max(1.0) as usize + 1, rect.height().ceil().max(1.0) as usize + 1);
        // Per square point: the colour (and one) of the last radar mark, dot and selection ring.
        let mut marked = vec![[0u8; 3]; cols * rows];
        let mut ownership = crate::team_markers::Clusters::new(EVec2::new(36.0, 26.0));
        let mut first = |s: Pos2, kind: usize, colour: u8| {
            let (x, y) = ((s.x - rect.min.x).max(0.0) as usize, (s.y - rect.min.y).max(0.0) as usize);
            let m = &mut marked[y.min(rows - 1) * cols + x.min(cols - 1)][kind];
            let new = *m != colour.wrapping_add(1);
            *m = colour.wrapping_add(1);
            new
        };
        for u in game.sim.units() {
            if u.dead || u.carried_by.is_some() {
                continue;
            }
            let seen = game.seen(u);
            if seen == ta_sim::Seen::Hidden {
                continue;
            }
            let p = u.pos.to_f32();
            let colour = game.sim.players.get(u.owner as usize).map_or(0, |pl| pl.color);
            let c = team_color(colour);
            let s = to_screen(p[0], p[2]);
            if seen == ta_sim::Seen::Radar {
                if first(s, 0, if game.team_markers { 0 } else { colour }) {
                    if game.team_markers {
                        // Radar reports a contact, not an identified player or unit type.
                        painter.circle_stroke(s, 2.5, Stroke::new(3.0, Color32::BLACK));
                        painter.circle_stroke(s, 2.5, Stroke::new(1.0, Color32::WHITE));
                    } else {
                        painter.circle_stroke(s, 1.8, Stroke::new(1.0, c));
                    }
                }
                continue;
            }
            if game.team_markers && let Some(marker) = crate::team_markers::identified(game, u.owner, seen) {
                ownership.push(s, marker);
            }
            let t = game.sim.unit_type(u);
            let r = (t.radius.to_f32() / map_px.x * rect.width()).clamp(1.2, 4.0);
            let building = t.locomotion == ta_sim::Locomotion::Static;
            // (Buildings, few and big, each drawn.)
            if building {
                painter.rect_filled(Rect::from_center_size(s, EVec2::splat(r * 2.0)), 0.0, c);
            } else if first(s, 1, colour) {
                painter.circle_filled(s, r, c);
            }
            if selected.contains(&u.id) && first(s, 2, 0) {
                painter.circle_stroke(s, r + 1.5, Stroke::new(1.0, Color32::WHITE));
            }
        }
        ownership.paint(&painter, 10.0, EVec2::new(0.0, -9.0));
        // Recent attacks on our units (red) and an ally's (their colour): expanding rings for a
        // few seconds.
        let now = game.sim.tick as f32 / 30.0;
        for &(p, t, who) in &game.attacks {
            let age = now - t;
            if !(0.0..4.0).contains(&age) {
                continue;
            }
            let s = to_screen(p.x, p.z);
            let pulse = (age * 2.0).fract();
            let a = ((1.0 - age / 4.0) * 230.0) as u8;
            let [r, g, b, _] = who.and_then(|w| game.sim.players.get(w as usize)).map_or([255, 70, 50, 255], |q| crate::palette::team_color(q.color).to_array());
            painter.circle_stroke(s, 3.0 + pulse * 9.0, Stroke::new(1.5, Color32::from_rgba_unmultiplied(r, g, b, a)));
        }
        // Team signals: bright expanding rings in the meaning's colour.
        for ping in &game.pings {
            let age = ping.age(now);
            if !(0.0..1.0).contains(&age) {
                continue;
            }
            let s = to_screen(ping.at.x, ping.at.z);
            let pulse = ((now - ping.time) * 1.5).fract();
            let a = ((1.0 - age) * 255.0) as u8;
            painter.circle_stroke(s, 3.0 + pulse * 12.0, Stroke::new(2.0, crate::signals::colour(ping.kind, a)));
            painter.circle_filled(s, 2.5, crate::signals::colour(ping.kind, a));
        }
        let primary_modifiers = ta_ui::widgets::click_modifiers(&response, egui::PointerButton::Primary);
        let targeted = game.command_mode.is_some() && game.can_order();
        // Alt+click pings when no armed order owns the click, as on the battlefield.
        if response.clicked() && primary_modifiers.alt && !targeted && let Some(pos) = response.interact_pointer_pos() {
            let w = to_world(pos);
            let y = game.sim.world.height_at(ta_sim::Fx::from_f32(w.x), ta_sim::Fx::from_f32(w.y)).to_f32();
            game.ping(glam::Vec3::new(w.x, y, w.y));
        }
        // Mission objective areas.
        for (x, z, r) in game.objective_areas() {
            let s = to_screen(x, z);
            let rr = (r / map_px.x * rect.width()).max(3.0);
            painter.circle_stroke(s, rr, Stroke::new(1.5, Color32::from_rgb(120, 255, 140)));
            painter.circle_stroke(s, rr + 3.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 255, 140, 120)));
        }
        // What the selected sensors, and one being placed, reach: their discs and rims in the sight
        // overlay's colours, so that a radar's whole coverage shows at a glance (the battlefield
        // shows only the part in view).
        for held in crate::hud::ranges::collect(game, game.alpha(), None) {
            let centre = to_screen(held.at.x, held.at.z);
            let dim = if held.power == crate::hud::ranges::Power::On { 1.0 } else { 0.4 };
            for range in crate::hud::ranges::display_ranges(game.sim.types.get(held.type_id), &game.sim.weapon_types) {
                if range.reach == crate::hud::ranges::Reach::Sight {
                    continue;
                }
                let (colour, radius) = (range.reach.colour(), range.radius / map_px.x * rect.width());
                painter.circle_filled(centre, radius, crate::hud::ranges::tint(colour, 0.13 * dim));
                painter.circle_stroke(centre, radius, Stroke::new(1.2, crate::hud::ranges::tint(colour, 0.9 * dim)));
            }
        }
        // Nuclear radar contacts and their observed flight paths, above coverage shading: the
        // trail fading out behind, the head pulsing; for their own side, the way ahead dashed
        // to where they come down, the blast's reach there and the time left. Interceptor
        // missiles and ordinary rockets do not receive nuclear markers.
        let alpha = game.alpha();
        let viewer = game.viewer();
        let radar_reveal = game.view_cheats.reveal == crate::chat::Reveal::Radar && !game.spectator;
        let sim = &game.sim;
        let sightings = sim.projectiles.iter().filter(|p| {
            let w = sim.weapon_types.get(p.weapon);
            !p.dead && w.def.stockpile && w.def.targetable && !w.def.interceptor
                && viewer.is_none_or(|viewer| sim.projectile_seen_by(p, viewer) != ta_sim::Seen::Hidden || radar_reveal)
        }).map(|p| {
            let before = p.prev_pos.to_f32();
            let now = p.pos.to_f32();
            let bound = crate::game::nuclear::destination_shown(sim, viewer, p).then(|| {
                let to = p.target_pos.to_f32();
                Bound { at: [to[0], to[2]], radius: sim.weapon_types.get(p.weapon).aoe_radius.to_f32(), seconds: crate::game::nuclear::time_to_impact(sim, p) }
            });
            Sighting { id: p.id, at: [before[0] + (now[0] - before[0]) * alpha, before[2] + (now[2] - before[2]) * alpha], bound }
        });
        let flying = |id: u32| sim.projectiles.iter().any(|p| p.id == id && !p.dead);
        self.nuclear_tracks.update(sim.tick, (viewer, radar_reveal), sightings, |id| !flying(id));
        let scale = rect.width() / map_px.x;
        let pulse = ((sim.tick as f32 + alpha) / 30.0 * 1.5).fract();
        for path in self.nuclear_tracks.paths.values() {
            let head = to_screen(path.current[0], path.current[1]);
            if let Some(b) = path.bound {
                let at = to_screen(b.at[0], b.at[1]);
                dashed(&painter, head, at, Stroke::new(1.0, NUCLEAR_YELLOW.gamma_multiply(0.6)));
                let r = (b.radius * scale).max(3.5);
                painter.circle_stroke(at, r, Stroke::new(2.8, Color32::from_black_alpha(160)));
                painter.circle_stroke(at, r, Stroke::new(1.3, IMPACT_RED));
                for d in [EVec2::X, EVec2::Y] {
                    painter.line_segment([at - d * 2.5, at + d * 2.5], Stroke::new(1.2, IMPACT_RED));
                }
                let text = crate::hud::duration(b.seconds.max(0.0));
                let font = egui::FontId::proportional(11.0);
                let place = at - EVec2::new(0.0, r + 2.0);
                painter.text(place + EVec2::new(1.0, 1.0), egui::Align2::CENTER_BOTTOM, &text, font.clone(), Color32::BLACK);
                painter.text(place, egui::Align2::CENTER_BOTTOM, &text, font, IMPACT_RED);
            }
            // The trail seen, each stretch fading as it ages, and on to where it is now.
            let mut points: Vec<(Pos2, f32)> = path.points.iter().map(|&(p, t)| (to_screen(p[0], p[1]), sim.tick.saturating_sub(t) as f32 / 30.0)).collect();
            points.push((head, 0.0));
            // (Its dark edge under the whole of it first: drawn stretch by stretch, each stretch's
            // edge notched the line before it.)
            let fades: Vec<f32> = points.iter().map(|&(_, age)| (1.0 - age / TRAIL_SECONDS).clamp(0.0, 1.0)).collect();
            for (pair, fade) in points.windows(2).zip(&fades) {
                painter.line_segment([pair[0].0, pair[1].0], Stroke::new(3.4, Color32::from_black_alpha((170.0 * fade) as u8)));
            }
            for (pair, fade) in points.windows(2).zip(&fades) {
                painter.line_segment([pair[0].0, pair[1].0], Stroke::new(1.6, NUCLEAR_YELLOW.gamma_multiply(*fade)));
            }
            // Its head: a ring running out from it, and the trefoil.
            painter.circle_stroke(head, 5.5 + pulse * 6.0, Stroke::new(1.3, NUCLEAR_YELLOW.gamma_multiply(1.0 - pulse)));
            nuclear_symbol(&painter, head, 5.0);
        }
        // Where a missile tracked to its end ended: a flash running out.
        for &(at, t) in &self.nuclear_tracks.endings {
            let age = (sim.tick.saturating_sub(t) as f32 / 30.0 / ENDING_SECONDS).clamp(0.0, 1.0);
            let s = to_screen(at[0], at[1]);
            painter.circle_stroke(s, 3.0 + age * 11.0, Stroke::new(2.0, Color32::WHITE.gamma_multiply(1.0 - age)));
            painter.circle_filled(s, 3.5 * (1.0 - age), NUCLEAR_YELLOW.gamma_multiply(1.0 - age));
        }
        // Camera footprint: where the screen corners hit the ground.
        let corners = [Vec2::ZERO, Vec2::new(camera.viewport.x, 0.0), camera.viewport, Vec2::new(0.0, camera.viewport.y)];
        let pts: Vec<Pos2> = corners
            .iter()
            .map(|&c| {
                let (o, d) = camera.screen_ray(c);
                let t = if d.y.abs() > 1e-4 { ((camera.focus.y - o.y) / d.y).clamp(0.0, 1e6) } else { 1e6 };
                let g = o + d * t;
                to_screen(g.x.clamp(-map_px.x, map_px.x * 2.0), g.z.clamp(-map_px.y, map_px.y * 2.0))
            })
            .collect();
        painter.add(egui::Shape::closed_line(pts, Stroke::new(1.2, Color32::from_rgba_unmultiplied(255, 255, 255, 200))));
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_rgb(214, 170, 64)), egui::StrokeKind::Outside);

        // An armed command targets the minimap with either button. Otherwise primary
        // click/drag looks there and secondary click moves the selection as before.
        let primary_order = targeted && (response.clicked() || response.drag_stopped_by(egui::PointerButton::Primary));
        if !targeted && (response.clicked() || response.dragged()) && let Some(p) = response.interact_pointer_pos() {
            let w = to_world(p);
            camera.focus.x = w.x.clamp(0.0, map_px.x);
            camera.focus.z = w.y.clamp(0.0, map_px.y);
            // (Looking elsewhere lets go of a unit followed and of the action camera.)
            game.follow = None;
            game.action_camera = false;
        }
        if (primary_order || response.secondary_clicked()) && !game.selection.is_empty() && let Some(p) = response.interact_pointer_pos() {
            // A drag can finish beyond the minimap; target its nearest map edge.
            let w = to_world(p).clamp(Vec2::ZERO, (map_px - Vec2::ONE).max(Vec2::ZERO));
            let ground = game.sim.world.height_at(Fx::from_f32(w.x), Fx::from_f32(w.y)).to_f32();
            let modifiers = if primary_order { primary_modifiers } else { ta_ui::widgets::click_modifiers(&response, egui::PointerButton::Secondary) };
            // (A unit's mark within a few of the minimap's pixels is the one clicked.)
            let reach = 5.0 / rect.width().max(1.0) * map_px.x;
            game.order_selection_from_map(glam::Vec3::new(w.x, ground, w.y), reach, modifiers.shift, ui.input(|i| i.modifiers.shift));
        }
        rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(id: u32, x: f32) -> std::iter::Once<Sighting> {
        std::iter::once(Sighting { id, at: [x, 500.0], bound: None })
    }

    #[test]
    fn nuclear_trails_only_join_observations_of_the_same_visible_flight() {
        let mut tracks = NuclearTracks::default();
        let view = (None, false);
        let flying = |_| false;
        for (tick, x) in [(10, 100.0), (16, 200.0), (22, 300.0)] {
            tracks.update(tick, view, seen(7, x), flying);
        }
        assert_eq!(tracks.paths[&7].points.len(), 3);
        tracks.update(22, view, std::iter::empty(), flying);
        assert!(tracks.paths.is_empty(), "losing detection, including while paused, hides the flight");
        assert!(tracks.endings.is_empty(), "a missile only out of sight is not marked as ended");
        tracks.update(28, view, seen(7, 900.0), flying);
        assert_eq!(tracks.paths[&7].points.len(), 1, "reacquisition must not trace an unseen path");
        tracks.update(5, view, seen(7, 50.0), flying);
        assert_eq!(tracks.paths[&7].points.len(), 1, "rewinding cannot join different flights");
        for tick in 6..2000 { tracks.update(tick, view, seen(7, tick as f32), flying); }
        let kept = tracks.paths[&7].points.len();
        assert!((95..=101).contains(&kept), "long flights keep the last few seconds' points: {kept}");
        assert!(tracks.paths[&7].points.iter().all(|&(_, t)| t + (TRAIL_SECONDS * 30.0) as u64 >= 1999));
    }

    #[test]
    fn a_tracked_missile_that_goes_off_leaves_a_mark_for_a_moment() {
        let mut tracks = NuclearTracks::default();
        let view = (Some(0), false);
        tracks.update(10, view, seen(7, 100.0), |_| false);
        tracks.update(11, view, seen(7, 110.0), |_| false);
        // Gone from the battle (it went off, or was shot down) while tracked.
        tracks.update(12, view, std::iter::empty(), |_| true);
        assert!(tracks.paths.is_empty());
        assert_eq!(tracks.endings, [([110.0, 500.0], 12)]);
        tracks.update(12 + (ENDING_SECONDS * 30.0) as u64, view, std::iter::empty(), |_| true);
        assert_eq!(tracks.endings.len(), 1);
        tracks.update(13 + (ENDING_SECONDS * 30.0) as u64, view, std::iter::empty(), |_| true);
        assert!(tracks.endings.is_empty(), "the mark fades");
        // Another view does not inherit it.
        tracks.update(20, view, seen(8, 100.0), |_| false);
        tracks.update(21, view, std::iter::empty(), |_| true);
        tracks.update(21, (Some(1), false), std::iter::empty(), |_| true);
        assert!(tracks.endings.is_empty());
    }

    #[test]
    fn where_a_missile_is_going_is_kept_as_it_is_told() {
        let mut tracks = NuclearTracks::default();
        let bound = Bound { at: [900.0, 900.0], radius: 256.0, seconds: 12.0 };
        tracks.update(10, (Some(0), false), std::iter::once(Sighting { id: 7, at: [100.0, 100.0], bound: Some(bound) }), |_| false);
        assert_eq!(tracks.paths[&7].bound, Some(bound));
        // No longer told (the unit it homes on out of sight): none.
        tracks.update(11, (Some(0), false), std::iter::once(Sighting { id: 7, at: [110.0, 110.0], bound: None }), |_| false);
        assert_eq!(tracks.paths[&7].bound, None);
    }

    #[test]
    fn nuclear_trails_reset_when_the_viewer_or_radar_reveal_changes() {
        for (before, after) in [
            ((None, false), (Some(0), false)),
            ((Some(0), false), (Some(1), false)),
            ((Some(1), false), (None, false)),
            ((Some(0), true), (Some(0), false)),
            ((Some(0), false), (Some(0), true)),
        ] {
            let mut tracks = NuclearTracks::default();
            let flying = |_| false;
            for (tick, x) in [(10, 100.0), (16, 200.0), (22, 300.0)] {
                tracks.update(tick, before, seen(7, x), flying);
            }
            assert_eq!(tracks.paths[&7].points.len(), 3);
            // Still detectable in the new perspective, with the simulation paused.
            tracks.update(22, after, seen(7, 400.0), flying);
            assert_eq!(tracks.paths[&7].points.iter().map(|&(at, _)| at).collect::<Vec<_>>(), [[400.0, 500.0]], "a new perspective must not inherit the old trail: {before:?} -> {after:?}");
            tracks.update(28, after, seen(7, 500.0), flying);
            assert_eq!(tracks.paths[&7].points.len(), 2, "the unchanged perspective keeps its own observations");
            tracks.update(28, before, seen(7, 500.0), flying);
            assert_eq!(tracks.paths[&7].points.len(), 1, "switching back must not restore an earlier perspective's trail");
        }
    }
}
