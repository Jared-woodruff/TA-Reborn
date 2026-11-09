//! What a sensor reaches: the ground a radar, a sonar or a jammer covers, drawn beside what its
//! unit sees, when the unit is selected, pointed at, or being placed. A radar's is a disc shading
//! up to its rim with a beam sweeping round it; a sonar's the same with pings spreading from the
//! middle; a jammer's a dashed rim; the unit's own sight a thin dashed ring, so that what it
//! sees, and what it senses beyond, are told apart (the colours are the sight overlay's).

use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2 as EVec2};
use glam::Vec3;
use ta_sim::UnitId;
use ta_ui::i18n::{n, t, tf, upper};

use crate::game::Game;

/// What a range is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Line of sight.
    Sight,
    Radar,
    Sonar,
    RadarJam,
    SonarJam,
    Interceptor,
}

/// A range of a unit type: what it is of, and how far (world units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Range {
    pub reach: Reach,
    pub radius: f32,
}

impl Reach {
    pub(crate) fn colour(self) -> Color32 {
        match self {
            Reach::Sight => Color32::from_rgb(236, 236, 224),
            // (The sight overlay's blue and teal.)
            Reach::Radar => Color32::from_rgb(88, 160, 255),
            Reach::Sonar => Color32::from_rgb(50, 214, 198),
            Reach::RadarJam | Reach::SonarJam => Color32::from_rgb(255, 150, 66),
            Reach::Interceptor => Color32::from_rgb(255, 220, 70),
        }
    }

    /// Its caption on the map, translated: "RADAR 1500".
    fn caption(self, radius: f32) -> String {
        let r = format!("{radius:.0}");
        match self {
            Reach::Sight => format!("{} {r}", t(n("SIGHT"))),
            Reach::Radar => format!("{} {r}", t(n("RADAR"))),
            Reach::Sonar => format!("{} {r}", t(n("SONAR"))),
            Reach::RadarJam | Reach::SonarJam => upper(&tf("Jams {range}", &[("range", &r)])),
            Reach::Interceptor => upper(&tf("Missile defence {range}", &[("range", &r)])),
        }
    }
}

/// The sensor ranges of a unit type with its sight to set them against: radar, sonar and jammers
/// with their reach, the widest first (so that it is drawn under the others). None for a type with
/// no sensors.
pub(crate) fn sensor_ranges(t: &ta_sim::UnitType) -> Vec<Range> {
    let d = &t.def;
    let mut ranges: Vec<Range> = [(Reach::Radar, t.radar.to_f32()), (Reach::Sonar, d.sonar_distance as f32), (Reach::RadarJam, d.radar_jam as f32), (Reach::SonarJam, d.sonar_jam as f32)]
        .into_iter()
        .filter(|&(_, radius)| radius > 0.0)
        .map(|(reach, radius)| Range { reach, radius })
        .collect();
    if ranges.is_empty() {
        return ranges;
    }
    ranges.push(Range { reach: Reach::Sight, radius: t.sight.to_f32() });
    ranges.sort_by(|a, b| b.radius.total_cmp(&a.radius));
    ranges
}

/// Sensor and anti-missile ranges share the same terrain and minimap overlays. The
/// protection umbrella uses `coverage`, never the missile's much larger travel range.
pub(crate) fn display_ranges(t: &ta_sim::UnitType, weapons: &ta_sim::WeaponTypes) -> Vec<Range> {
    let mut ranges = sensor_ranges(t);
    let radius = t.weapons.iter().flatten().map(|&w| weapons.get(w).interceptor_coverage().to_f32()).fold(0.0, f32::max);
    if radius > 0.0 {
        if ranges.is_empty() {
            ranges.push(Range { reach: Reach::Sight, radius: t.sight.to_f32() });
        }
        ranges.push(Range { reach: Reach::Interceptor, radius });
        ranges.sort_by(|a, b| b.radius.total_cmp(&a.radius));
    }
    ranges
}

/// Whether selecting a unit of this type shows its sensors: a building, or one that does not
/// fight (a radar vehicle, a jammer). Not the Commander and other fighters that happen to sense a
/// little (pointing at one of those shows them).
pub(crate) fn is_sensor_unit(t: &ta_sim::UnitType) -> bool {
    !sensor_ranges(t).is_empty() && (t.locomotion == ta_sim::Locomotion::Static || t.weapons.iter().all(|w| w.is_none()))
}

/// A unit type held somewhere, whose ranges show (see [`collect`]).
pub(crate) struct Ranged {
    pub type_id: usize,
    pub at: Vec3,
    /// Its weapons' reach shows too (a building's).
    pub weapons: bool,
    pub power: Power,
}

/// What shows its ranges now: the building being placed; the selected finished buildings, and
/// units that only sense (a radar vehicle, a jammer); and `hovered`, a unit the pointer rests on,
/// when it has sensors (the Commander's, an enemy's tower: how far they reach). The Commander
/// and the fighters that sense a little show nothing selected: they are selected all the time.
pub(crate) fn collect(game: &Game, alpha: f32, hovered: Option<UnitId>) -> Vec<Ranged> {
    let mut ranged: Vec<Ranged> = game
        .placement
        .as_ref()
        .and_then(|pl| pl.pos.map(|p| (pl.type_id, p)))
        .into_iter()
        .map(|(type_id, at)| Ranged { type_id, at, weapons: true, power: Power::On })
        .collect();
    let at_of = |u: &ta_sim::Unit| game.scene.unit_position(u.id, alpha).unwrap_or(Vec3::from_array(u.pos.to_f32()));
    // (A unit that is switched off covers nothing, and says so.)
    let power_of = |u: &ta_sim::Unit| if game.sim.sensors_active(u) { Power::On } else { Power::Off };
    let mut shown = Vec::new();
    for &id in game.selection.iter().take(16) {
        let Some(u) = game.sim.unit(id).filter(|u| u.is_complete()) else { continue };
        let t = game.sim.unit_type(u);
        let building = t.locomotion == ta_sim::Locomotion::Static;
        let protects = t.weapons.iter().flatten().any(|&w| game.sim.weapon_types.get(w).def.interceptor);
        if building || is_sensor_unit(t) || protects {
            ranged.push(Ranged { type_id: u.type_id, at: at_of(u), weapons: building, power: power_of(u) });
            shown.push(id);
        }
    }
    if let Some(u) = hovered
        .and_then(|h| game.sim.unit(h))
        .filter(|u| u.is_complete() && hover_adds(&shown, u.id) && game.seen(u) == ta_sim::Seen::Visible)
        && !display_ranges(game.sim.unit_type(u), &game.sim.weapon_types).is_empty()
    {
        ranged.push(Ranged { type_id: u.type_id, at: at_of(u), weapons: false, power: power_of(u) });
    }
    ranged
}

/// Whether the unit pointed at shows its ranges besides the selection's: unless they show already.
/// (A selected Commander's, a fighter's, do not: pointing at one shows them, selected or not; so
/// do the selected sensors past the sixteen shown.)
fn hover_adds(shown: &[UnitId], hovered: UnitId) -> bool {
    !shown.contains(&hovered)
}

/// Where a range is drawn: the picture's painter, how the ground projects to the screen, how high
/// it is, where a caption may stand.
pub(super) struct Canvas<'a> {
    pub painter: &'a Painter,
    pub project: &'a dyn Fn(Vec3) -> Option<Pos2>,
    pub ground: &'a dyn Fn(f32, f32) -> f32,
    pub free: Rect,
    /// The interface clock (seconds), and whether motion is held still (nothing moves).
    pub clock: f32,
    pub still: bool,
    /// Where captions have been put this frame (and what they keep clear of: the minimap's
    /// plate).
    pub placed: std::cell::RefCell<Vec<Rect>>,
}

/// Whether the unit whose range it is covers anything: one that is switched off does not, and
/// says so.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Power {
    On,
    Off,
}

/// `c` at an opacity `a` (0 to 1).
pub(crate) fn tint(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Draws `range` round `centre`: the ground it covers, shaded up to its rim, the rim, what moves in
/// it, and its caption at the rim's upper right.
pub(super) fn draw(c: &Canvas, centre: Vec3, range: Range, power: Power) {
    let colour = range.reach.colour();
    // (A unit that is off still sees: only what it senses is dimmed.)
    let off = power == Power::Off && range.reach != Reach::Sight;
    let dim = if off { 0.4 } else { 1.0 };
    let at = |a: f32, r: f32| {
        let (x, z) = (centre.x + a.cos() * r, centre.z + a.sin() * r);
        (c.project)(Vec3::new(x, (c.ground)(x, z) + 2.0, z))
    };
    let Some(middle) = at(0.0, 0.0) else { return };
    // Sides by how big it is on the screen (one every few points).
    let across = [0.0f32, std::f32::consts::FRAC_PI_2].iter().filter_map(|&a| Some(at(a, range.radius)?.distance(middle))).fold(0.0f32, f32::max);
    let sides = (std::f32::consts::TAU * across / 5.0).clamp(64.0, 360.0) as usize;
    let rim: Vec<Option<Pos2>> = (0..=sides).map(|k| at(k as f32 / sides as f32 * std::f32::consts::TAU, range.radius)).collect();
    // The ground it covers: clear in the middle, shaded towards the rim.
    if range.reach != Reach::Sight {
        let mut mesh = Mesh::default();
        mesh.colored_vertex(middle, tint(colour, 0.12 * dim));
        for p in &rim {
            match p {
                Some(p) => mesh.colored_vertex(*p, tint(colour, 0.28 * dim)),
                None => mesh.colored_vertex(middle, Color32::TRANSPARENT),
            }
        }
        for k in 1..=sides as u32 {
            if rim[k as usize - 1].is_some() && rim[k as usize].is_some() {
                mesh.add_triangle(0, k, k + 1);
            }
        }
        c.painter.add(Shape::mesh(mesh));
    }
    // What moves in it (not with motion held still, nor when the unit is off): a radar's beam
    // sweeping round, a sonar's pings spreading out.
    if !off && !c.still {
        match range.reach {
            Reach::Radar => {
                // The beam's trailing wedge, brightest at its leading edge, as bright all along its
                // length (a pair of vertices a step, the middle's and the rim's).
                let head = (c.clock * 0.8) % std::f32::consts::TAU;
                let (width, steps) = (1.1, 18);
                let mut mesh = Mesh::default();
                let mut last: Option<(u32, u32)> = None;
                for k in 0..=steps {
                    let f = k as f32 / steps as f32;
                    let shade = tint(colour, 0.46 * f * f);
                    match at(head - width * (1.0 - f), range.radius) {
                        Some(p) => {
                            let index = mesh.vertices.len() as u32;
                            mesh.colored_vertex(middle, shade);
                            mesh.colored_vertex(p, shade);
                            if let Some((c0, p0)) = last {
                                mesh.add_triangle(c0, p0, index + 1);
                                mesh.add_triangle(c0, index + 1, index);
                            }
                            last = Some((index, index + 1));
                        }
                        None => last = None,
                    }
                }
                c.painter.add(Shape::mesh(mesh));
                if let Some(tip) = at(head, range.radius) {
                    c.painter.line_segment([middle, tip], Stroke::new(1.4, tint(colour, 0.5)));
                }
            }
            Reach::Sonar => {
                for k in 0..3 {
                    let phase = (c.clock * 0.3 + k as f32 / 3.0) % 1.0;
                    let ring: Vec<Pos2> = (0..=sides / 2).filter_map(|j| at(j as f32 / (sides / 2) as f32 * std::f32::consts::TAU, range.radius * phase)).collect();
                    c.painter.add(Shape::line(ring, Stroke::new(1.6, tint(colour, 0.55 * (1.0 - phase)))));
                }
            }
            _ => {}
        }
    }
    // The rim, edged dark so that it reads on pale ground: sight thin and dashed, a jammer's dashed,
    // the sensors' solid.
    let points: Vec<Pos2> = rim.iter().flatten().copied().collect();
    let sight = range.reach == Reach::Sight;
    c.painter.add(Shape::line(points.clone(), Stroke::new(if sight { 2.6 } else { 3.6 }, Color32::from_black_alpha((70.0 * dim) as u8))));
    let stroke = Stroke::new(if sight { 1.2 } else { 2.0 }, tint(colour, 0.95 * dim));
    if matches!(range.reach, Reach::Sight | Reach::RadarJam | Reach::SonarJam) {
        let (dash, gap) = if sight { (4.0, 5.0) } else { (10.0, 6.0) };
        let crawl = if c.still { 0.0 } else { (c.clock * 14.0) % (dash + gap) };
        c.painter.extend(Shape::dashed_line_with_offset(&points, stroke, &[dash], &[gap], crawl));
    } else {
        c.painter.add(Shape::line(points, stroke));
    }
    // Its caption at the rim's upper right, or, where the rim is out of the view (a radar reaches
    // further than the screen shows), beside the unit; and OFF where it covers nothing.
    let room = c.free.shrink(30.0);
    let on_screen = rim.iter().flatten().copied().filter(|p| room.contains(*p));
    let anchor = on_screen.max_by(|a, b| (a.x - a.y).total_cmp(&(b.x - b.y))).unwrap_or(middle + EVec2::new(0.0, -34.0));
    let mut text = range.reach.caption(range.radius);
    if off {
        text = format!("{text} · {}", t(n("OFF")));
    }
    caption(c, anchor, &text, colour, dim);
}

/// A small plate of lettering in `colour` standing on `at`, kept in the free view.
fn caption(c: &Canvas, at: Pos2, text: &str, colour: Color32, dim: f32) {
    let galley = c.painter.layout_no_wrap(text.to_string(), crate::theme::strong(10.5), tint(colour, dim.max(0.7)));
    let size = galley.size() + EVec2::new(10.0, 4.0);
    let mut min = at + EVec2::new(-size.x / 2.0, -size.y - 3.0);
    let room = c.free.shrink(4.0);
    min.x = min.x.clamp(room.left(), (room.right() - size.x).max(room.left()));
    min.y = min.y.clamp(room.top(), (room.bottom() - size.y).max(room.top()));
    // (Not over another's: those of one unit's ranges can meet where they leave the view.)
    let mut placed = c.placed.borrow_mut();
    for _ in 0..8 {
        let plate = Rect::from_min_size(min, size);
        if !placed.iter().any(|other| other.intersects(plate)) {
            break;
        }
        min.y = (min.y + size.y + 3.0).min((room.bottom() - size.y).max(room.top()));
    }
    let plate = Rect::from_min_size(min, size);
    placed.push(plate);
    c.painter.rect_filled(plate, 3.0, Color32::from_black_alpha(175));
    c.painter.rect_stroke(plate, 3.0, Stroke::new(1.0, tint(colour, 0.55 * dim)), egui::StrokeKind::Inside);
    c.painter.galley(plate.min + EVec2::new(5.0, 2.0), galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn types(units: &[(&str, &str)]) -> ta_sim::UnitTypes {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("ta-ranges-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let files: Vec<(String, String)> = units
            .iter()
            .map(|(name, keys)| (format!("units/{}.fbi", name.to_ascii_lowercase()), format!("[UNITINFO]\n{{\nUnitName={name};\nObjectname={name};\nMaxDamage=100;\nSightDistance=300;\n{keys}\n}}\n")))
            .chain([("weapons/gun.tdf".to_string(), "[GUN]\n{\nname=Gun;\nrange=300;\nreloadtime=1;\n[DAMAGE]\n{\ndefault=10;\n}\n}\n".to_string())])
            .collect();
        for (path, text) in &files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let data = Arc::new(ta_data::GameData::load(&vfs));
        std::fs::remove_dir_all(&dir).ok();
        ta_sim::UnitTypes::compile(&data, &|_| None)
    }

    /// A radar tower, a radar vehicle, a fighter that senses a little, a sonar station, a jammer
    /// and a tank.
    fn yard() -> ta_sim::UnitTypes {
        types(&[
            ("TOWER", "RadarDistance=1500;"),
            ("SEER", "RadarDistance=1200; CanMove=1; MaxVelocity=2;"),
            ("COM", "RadarDistance=400; CanMove=1; MaxVelocity=1; Weapon1=GUN;"),
            ("SONAR", "SonarDistance=800;"),
            ("JAMMER", "RadarDistanceJam=450; CanMove=1; MaxVelocity=2;"),
            ("TANK", "CanMove=1; MaxVelocity=2; Weapon1=GUN;"),
        ])
    }

    #[test]
    fn a_sensor_unit_lists_its_ranges_widest_first_with_its_sight() {
        let yard = yard();
        let ranges = |name: &str| sensor_ranges(yard.by_name(name).unwrap()).into_iter().map(|r| (r.reach, r.radius)).collect::<Vec<_>>();
        assert_eq!(ranges("TOWER"), [(Reach::Radar, 1500.0), (Reach::Sight, 300.0)]);
        assert_eq!(ranges("SONAR"), [(Reach::Sonar, 800.0), (Reach::Sight, 300.0)]);
        assert_eq!(ranges("JAMMER"), [(Reach::RadarJam, 450.0), (Reach::Sight, 300.0)]);
        assert!(ranges("TANK").is_empty(), "no sensors, nothing to set against its sight");
    }

    #[test]
    fn selecting_shows_the_sensors_of_buildings_and_unarmed_units_not_of_fighters() {
        let yard = yard();
        let shown = |name: &str| is_sensor_unit(yard.by_name(name).unwrap());
        assert!(shown("TOWER") && shown("SONAR"), "buildings");
        assert!(shown("SEER") && shown("JAMMER"), "a radar vehicle and a jammer do nothing else");
        assert!(!shown("COM"), "the Commander senses a little: pointing at it shows it, selecting does not");
        assert!(!shown("TANK"));
    }

    /// Pointing at a unit shows its ranges unless the selection shows them already: the Commander
    /// selected (whose ranges selecting does not show) shows them when pointed at.
    #[test]
    fn pointing_at_a_selected_fighter_shows_its_sensors() {
        let id = |slot| UnitId { slot, generation: 0 };
        let shown = [id(3)];
        assert!(!hover_adds(&shown, id(3)), "a selected radar tower shows them already");
        assert!(hover_adds(&shown, id(7)), "the selected Commander, not among those shown, shows them pointed at");
        assert!(hover_adds(&[], id(7)));
    }

    #[test]
    fn captions_name_what_they_reach() {
        assert_eq!(Reach::Radar.caption(1500.0), "RADAR 1500");
        assert_eq!(Reach::Sight.caption(290.0), "SIGHT 290");
        assert_eq!(Reach::Sonar.caption(800.0), "SONAR 800");
        assert_eq!(Reach::RadarJam.caption(450.0), "JAMS 450");
    }

    #[test]
    fn anti_missile_overlays_show_protection_instead_of_flight_range() {
        let mut unit = types(&[("PROTECTOR", "")]).types.remove(0);
        unit.weapons = [Some(0), None, None];
        for (keys, expected) in [("interceptor=1; coverage=2000; range=32000;", 2000.0), ("interceptor=1; coverage=500; range=32000;", 500.0), ("interceptor=1; range=900;", 900.0)] {
            let mut data = ta_data::GameData::load(&ta_vfs::Vfs::new());
            data.weapons.push(ta_data::WeaponDef::from_tdf(&ta_formats::tdf::parse(&format!("[ANTI]{{{keys}}}")).sections[0]));
            let weapons = ta_sim::WeaponTypes::compile(&data);
            let shown = display_ranges(&unit, &weapons);
            assert!(shown.contains(&Range { reach: Reach::Interceptor, radius: expected }));
            assert!(!shown.iter().any(|r| r.radius == 32000.0));
            assert!(shown.contains(&Range { reach: Reach::Sight, radius: 300.0 }));
        }
        assert_eq!(Reach::Interceptor.caption(2000.0), "MISSILE DEFENCE 2000");
    }
}
