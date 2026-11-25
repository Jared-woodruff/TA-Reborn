//! Forged Alliance's weapon effects made into TA Reborn's (docs/SUPCOM_UNITS.md, "Weapons'
//! effects"; the keys and the effect files are docs/MODDING.md's): each weapon's muzzle flash,
//! its projectile's trail and its impacts (on land, on the water) as particle effects of the mod's
//! own (`effects/supcom_<side>.tdf`, named by the weapon's `fireeffect`, `traileffect`,
//! `hiteffect` and `waterhiteffect`), its shot's and its beam's true colours and width
//! (`rgbcolor`, `rgbcolor2`, `thickness`), and its projectile's own model where TA Reborn draws it
//! as one (a missile, a torpedo, a bomb: `model=`), as [`crate::fx`] finds them named.
//!
//! Each of Forged Alliance's particle emitters becomes a burst of TA Reborn's (ta_data's
//! `BurstDef`), approximated from its blueprint: what it is (a glow, sparks, smoke, grit, a ring
//! on the ground...) by its picture's name and how it blends; its particles' sizes, lives, speeds,
//! directions, fall and how many it makes from its curves, at TA Reborn's scale (an ogrid 32
//! pixels, a tick a tenth of a second); its colour, and the colour its particles turn to, from its
//! picture's colour on average and its ramp's colours at a particle's birth and near its death.
//! So each faction's weapons look as its own do: the UEF's shells, the Cybran's red lasers, the
//! Aeon's green and white, the Seraphim's yellow and violet. Only arithmetic is used, in a fixed
//! order: the same bytes on every computer.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::units::{self, Kind};
use super::{model, textures, UNIT_SCALE};
use crate::files::Files;
use crate::fx::{self, Emitter, Library, Particles, ShotFx, WeaponFx};
use crate::unit::{Faction, UnitBlueprint, WeaponBlueprint};
use crate::Result;

/// Ticks a second: Forged Alliance's emitters count their time in ticks.
const TICKS: f64 = 10.0;
/// Pixels an ogrid a tick, a second (an emitter's speeds), and an ogrid a tick a tick, a second a
/// second (its accelerations).
const SPEED: f64 = UNIT_SCALE * TICKS;
const ACCELERATION: f64 = UNIT_SCALE * TICKS * TICKS;
/// What Forged Alliance's gravity pulls an emitter's particles down at, where it asks for it
/// (pixels a second a second).
const GRAVITY: f64 = 160.0;
/// The most particles a burst makes at once, and a trail's a second; the most bursts an effect
/// keeps (ta_data's `effect::MAX_BURSTS`), and the most particles an effect makes at once.
const MOST_AT_ONCE: f64 = 48.0;
const MOST_A_SECOND: f64 = 90.0;
const MOST_BURSTS: usize = 12;
const MOST_IN_AN_EFFECT: f64 = 160.0;
/// How bright the glowing kinds are drawn, at their ramp's brightest.
const GLOW: f64 = 2.6;
const SPARKS: f64 = 3.2;
const FLAMES: f64 = 1.8;

/// What a weapon looks like in TA Reborn's keys: its projectile's model (where TA Reborn draws it
/// as a model and the game gives it a mesh) and the keys of its section.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub model: Option<String>,
    pub keys: Vec<(&'static str, String)>,
}

/// What the factions' effects come to: their files, the projectiles' models to make, and how each
/// weapon looks (by its unit's id and its place among its unit's weapons).
#[derive(Clone, Debug, Default)]
pub struct Made {
    /// `effects/supcom_<side>.tdf` and its text, a faction each, in the factions' order.
    pub files: Vec<(String, String)>,
    /// The projectiles whose meshes are made models (`projectiles/<name>/<name>_proj.bp`'s
    /// paths), each once, in order.
    pub models: Vec<String>,
    looks: BTreeMap<(String, usize), Look>,
    /// Each faction's adjacency beam: its colour (linear, 0 to 1) and thickness (pixels).
    adjacency: BTreeMap<String, ([f64; 3], f64)>,
}

impl Made {
    /// The unit with each of its weapons' looks.
    pub fn dressed(&self, u: &UnitBlueprint) -> UnitBlueprint {
        let mut u = u.clone();
        for (i, w) in u.weapons.iter_mut().enumerate() {
            if let Some(look) = self.looks.get(&(u.id.clone(), i)) {
                w.look = Some(look.clone());
            }
        }
        u
    }

    /// The keys of the unit's own file for how it looks: a structure giving the structures
    /// beside it bonuses, its faction's adjacency beam (`AdjacencyBeamColor`,
    /// `AdjacencyBeamThickness`).
    pub fn unit_keys(&self, u: &UnitBlueprint) -> Vec<(&'static str, String)> {
        if u.adjacency.is_none() || u.motion != crate::unit::Motion::None {
            return Vec::new();
        }
        match self.adjacency.get(units::side(&u.faction)) {
            Some((colour, thickness)) => vec![("AdjacencyBeamColor", rgb(*colour)), ("AdjacencyBeamThickness", number(*thickness))],
            None => Vec::new(),
        }
    }

    /// How many weapons have a look of their own.
    pub fn weapons(&self) -> usize {
        self.looks.len()
    }
}

/// A number as the effect files write it: to three places, without trailing zeros.
fn number(v: f64) -> String {
    let v = if v.abs() < 0.0005 { 0.0 } else { v };
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

fn rgb(c: [f64; 3]) -> String {
    format!("{} {} {}", number(c[0]), number(c[1]), number(c[2]))
}

/// A colour's linear value from its painted one (0 to 1): its square (near enough the sRGB curve
/// for effects' colours, and the same arithmetic on every computer).
fn linear(c: [f64; 3]) -> [f64; 3] {
    c.map(|v| v.clamp(0.0, 1.0) * v.clamp(0.0, 1.0))
}

/// A picture's look: its colour on average (painted, 0 to 1; the bright and opaque texels
/// counting most), and how much of it is covered (its alpha on average).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Picture {
    colour: [f64; 3],
    cover: f64,
}

/// A ramp's colours (painted, RGBA 0 to 1) at a particle's birth and near its death, and its
/// alpha at its most (a ramp often fades a particle in); and its colour where it is brightest (by
/// its brightest channel, the more opaque the brighter: what a glow faded in from black shows).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ramp {
    start: [f64; 4],
    end: [f64; 4],
    most: f64,
    brightest: [f64; 4],
}

/// The textures read, by path (none: not there, or not read).
#[derive(Default)]
struct Pictures {
    pictures: BTreeMap<String, Option<Picture>>,
    ramps: BTreeMap<String, Option<Ramp>>,
}

impl Pictures {
    fn picture(&mut self, files: &Files, path: &str) -> Result<Option<Picture>> {
        if let Some(p) = self.pictures.get(path) {
            return Ok(*p);
        }
        let found = match files.read(path)? {
            Some(bytes) => picture(&bytes),
            None => None,
        };
        self.pictures.insert(path.to_string(), found);
        Ok(found)
    }

    fn ramp(&mut self, files: &Files, path: &str) -> Result<Option<Ramp>> {
        if let Some(r) = self.ramps.get(path) {
            return Ok(*r);
        }
        let found = match files.read(path)? {
            Some(bytes) => ramp(&bytes),
            None => None,
        };
        self.ramps.insert(path.to_string(), found);
        Ok(found)
    }
}

/// A picture's look (see [`Picture`]), from its level of about 32 texels a side.
fn picture(bytes: &[u8]) -> Option<Picture> {
    let header = crate::dds::Header::parse(bytes).ok()?;
    let image = crate::dds::decode(bytes, header.level_for(32)).ok()?;
    if image.rgba.is_empty() {
        return None;
    }
    // (Each texel weighted by its alpha and its brightness, in whole numbers.)
    let (mut sum, mut weight, mut alpha) = ([0u64; 3], 0u64, 0u64);
    for t in &image.rgba {
        let a = t[3] as u64;
        let bright = (t[0] as u64 + t[1] as u64 + t[2] as u64) / 3 + 1;
        let w = a * bright;
        for k in 0..3 {
            sum[k] += t[k] as u64 * w;
        }
        weight += w;
        alpha += a;
    }
    let colour = if weight > 0 { sum.map(|s| s as f64 / weight as f64 / 255.0) } else { [1.0; 3] };
    Some(Picture { colour, cover: alpha as f64 / (image.rgba.len() as f64 * 255.0) })
}

/// A ramp's colours: its top row, a sixteenth of the way along (a particle's birth) and nine
/// tenths (near its death).
fn ramp(bytes: &[u8]) -> Option<Ramp> {
    let image = crate::dds::decode(bytes, 0).ok()?;
    if image.width == 0 || image.height == 0 {
        return None;
    }
    let at = |share: u32| -> [f64; 4] {
        let x = ((image.width - 1) * share / 16) as i64;
        image.clamped(x, 0).map(|c| c as f64 / 255.0)
    };
    let most = (0..image.width as i64).map(|x| image.clamped(x, 0)[3]).max().unwrap_or(255) as f64 / 255.0;
    // (The first of the brightest, in whole numbers.)
    let shine = |t: [u8; 4]| u32::from(t[0].max(t[1]).max(t[2])) * (u32::from(t[3]) + 1);
    let mut brightest = image.clamped(0, 0);
    for x in 1..image.width as i64 {
        let t = image.clamped(x, 0);
        if shine(t) > shine(brightest) {
            brightest = t;
        }
    }
    Some(Ramp { start: at(1), end: at(14), most, brightest: brightest.map(|c| c as f64 / 255.0) })
}

/// What one of Forged Alliance's emitters is drawn as in TA Reborn (ta_data's burst kinds), by
/// its picture's name and how it blends: none for what TA Reborn does not draw so (a distortion,
/// a ring standing up).
fn kind_of(e: &Particles, speed: f64) -> Option<&'static str> {
    let name = e.texture.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    // (0 painted over, 4 its alpha premultiplied; 5 a distortion; the others light, added.)
    let light = !matches!(e.blend, 0 | 4);
    if e.blend == 5 || has(&["distort"]) {
        return None;
    }
    if has(&["ring", "ripple", "shockwave", "wave"]) {
        return e.flat.then_some("ring");
    }
    if has(&["water", "splash", "plume", "spray", "foam"]) {
        return Some(if light { "glow" } else { "spray" });
    }
    if !light && has(&["dirt", "chunk", "rock", "debris", "dust"]) {
        return Some(if has(&["dust"]) { "dust" } else { "dirt" });
    }
    if has(&["fire", "flame", "napalm", "inferno"]) {
        return Some(if light { "fire" } else { "smoke" });
    }
    if has(&["smoke", "cloud", "gas", "miasma", "steam", "mist", "fog"]) {
        return Some(if light { "glow" } else { "smoke" });
    }
    // (Streaks and sparkles flying are sparks; standing, they glint as glows.)
    if light && has(&["spark", "line", "streak", "debris", "shard"]) && speed > 40.0 {
        return Some("spark");
    }
    Some(if light { "glow" } else { "smoke" })
}

/// A curve's value either side of its mean (its mean, less and more half its range).
fn either(curve: &fx::Curve, floor: f64) -> (f64, f64) {
    let (mean, spread) = curve.mean(floor);
    ((mean - spread * 0.5).max(floor), mean + spread * 0.5)
}

/// Where an effect plays: at a muzzle, along a trail (its counts a second), where a shot hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Fire,
    Trail,
    Hit,
}

/// One of Forged Alliance's particle emitters as a burst's keys (its section's body) and how many
/// particles it makes, played at `scale` (the class's `FxMuzzleFlashScale`...); none where TA
/// Reborn draws nothing of it. Its particles fly along its direction curves (not of unit length:
/// their length scales the speed) at its velocity curve's speed (ogrids a tick); its curves' ranges
/// are a particle's spread either side of their values.
fn burst(files: &Files, e: &Particles, role: Role, scale: f64, pictures: &mut Pictures) -> Result<Option<(String, f64)>> {
    if !e.high_fidelity {
        return Ok(None);
    }
    let c = |name: &str| e.curve(name);
    // (Forged Alliance's x is to the left: a unit faces +z, its right at -x.)
    let axis = |name: &str| c(name).mean(f64::MIN);
    let ((dx, sx), (dy, sy), (dz, sz)) = (axis("XDirectionCurve"), axis("YDirectionCurve"), axis("ZDirectionCurve"));
    let (dx, dy, dz) = (-dx, dy, dz);
    let mean = (dx * dx + dy * dy + dz * dz).sqrt();
    let spread = (sx * sx + sy * sy + sz * sz).sqrt() * 0.5;
    let (v_lo, v_hi) = either(c("VelocityCurve"), 0.0);
    let speed_lo = v_lo * (mean - spread).max(0.0) * SPEED * scale;
    let speed_hi = v_hi * (mean + spread) * SPEED * scale;
    let Some(kind) = kind_of(e, speed_hi) else { return Ok(None) };
    // How many: its rate (a tick) over its life (a tick at least), or a trail's a second.
    let (rate, _) = c("EmitRateCurve").mean(0.0);
    let count = match role {
        Role::Trail => (rate * TICKS).min(MOST_A_SECOND),
        _ => (rate * e.lifetime.max(1.0)).min(MOST_AT_ONCE),
    };
    if count < 0.5 && kind != "ring" {
        return Ok(None);
    }
    let (life_lo, life_hi) = either(c("LifetimeCurve"), 0.0);
    let (life_lo, life_hi) = ((life_lo / TICKS).clamp(0.03, 10.0), (life_hi / TICKS).clamp(0.03, 10.0));
    let size = |name: &str| -> f64 { c(name).mean(0.0).0 * UNIT_SCALE * scale };
    let start = size("StartSizeCurve");
    let end = if c("EndSizeCurve").given() { size("EndSizeCurve") } else { start };
    let (direction, cone) = if mean < 1e-6 || speed_hi < 1e-3 {
        ("out", 180.0)
    } else {
        let cone = (crate::convert::trig::atan2(spread, mean).to_degrees()).clamp(3.0, 180.0);
        let way = if dy.abs() >= dz.abs() && dy.abs() >= dx.abs() {
            if dy > 0.0 { "up" } else { "down" }
        } else if dz.abs() >= dx.abs() {
            if dz > 0.0 { "forward" } else { "back" }
        } else {
            "out"
        };
        (way, if way == "out" { 180.0 } else { cone })
    };
    let pos = |name: &str| c(name).mean(f64::MIN);
    let ((px, rx), (py, ry), (pz, rz)) = (pos("XPosCurve"), pos("YPosCurve"), pos("ZPosCurve"));
    let offset = [-px, py, pz].map(|v| v * UNIT_SCALE * scale);
    let radius = rx.max(ry).max(rz) * 0.5 * UNIT_SCALE * scale;
    let (ay, _) = c("YAccelCurve").mean(f64::MIN);
    let gravity = -ay * ACCELERATION * scale + if e.gravity { GRAVITY } else { 0.0 };
    let drag = if e.resistance { (c("ResistanceCurve").mean(0.0).0 * TICKS).min(20.0) } else { 0.0 };
    // Its colours: its picture's on average through its ramp's, at birth and near death.
    let picture = if e.texture.is_empty() { None } else { pictures.picture(files, &e.texture)? };
    let ramp = if e.ramp.is_empty() { None } else { pictures.ramp(files, &e.ramp)? };
    let tex = picture.map_or([1.0; 3], |p| p.colour);
    let cover = picture.map_or(0.5, |p| p.cover);
    let (r0, r1) = ramp.map_or(([1.0; 4], [1.0, 1.0, 1.0, 0.0]), |r| (r.start, r.end));
    // (A glowing kind's ramp fading it in from black: its colour where the ramp is brightest,
    // which is what shows, as at its birth it would add nothing; black throughout, it is none.)
    let dark = |c: [f64; 4]| c[0].max(c[1]).max(c[2]) < 0.02;
    let glowing = matches!(kind, "glow" | "spark" | "fire" | "ring");
    let r0 = match ramp {
        Some(r) if glowing && dark(r0) => r.brightest,
        _ => r0,
    };
    if glowing && dark(r0) {
        return Ok(None);
    }
    let most = ramp.map_or(1.0, |r| r.most);
    let paint = |r: [f64; 4]| linear([tex[0] * r[0], tex[1] * r[1], tex[2] * r[2]]);
    let (first, last) = (paint(r0), paint(r1));
    let peak = first[0].max(first[1]).max(first[2]).max(1e-3);
    let mut s = String::new();
    let mut put = |key: &str, value: String| {
        let _ = writeln!(s, "\t\t{key}={value};");
    };
    put("kind", kind.into());
    if kind == "ring" {
        let across = end.max(start).max(4.0);
        put("size", format!("{} {}", number((start * 0.25).max(across * 0.06).max(1.0)), number(across)));
        put("life", number((life_lo + life_hi) * 0.5));
        put("color", rgb(first.map(|v| v / peak)));
        put("intensity", number((0.25 + 0.5 * r0[3]).clamp(0.15, 0.75)));
        return Ok(Some((s, 1.0)));
    }
    let n = count.round().max(1.0);
    put("count", number(n));
    put("size", format!("{} {}", number(start.max(0.5)), number(end.max(0.25))));
    put("life", format!("{} {}", number(life_lo), number(life_hi.max(life_lo))));
    if speed_hi > 0.5 {
        put("speed", format!("{} {}", number(speed_lo), number(speed_hi)));
        put("direction", direction.into());
        put("cone", number(cone));
    } else {
        put("speed", "0".into());
    }
    put("radius", if radius > 0.5 { number(radius) } else { "0".into() });
    if offset.iter().any(|v| v.abs() >= 0.5) {
        put("offset", format!("{} {} {}", number(offset[0]), number(offset[1]), number(offset[2])));
    }
    put("gravity", number(gravity));
    put("drag", number(drag));
    // (How bright it is drawn at its ramp's brightest, the ramp's own at a particle's birth.)
    let bright = |base: f64| number((base * (0.4 + 0.6 * r0[3]) * (0.6 + 0.4 * peak.sqrt())).clamp(0.3, 6.0));
    match kind {
        "glow" | "spark" => {
            // (Its hue at its brightest; its end as dim as the ramp is there.)
            put("color", rgb(first.map(|v| v / peak)));
            // (Sparks keep their colour unless they burn as hot metal does, reds over blues: those
            // cool as hot metal does, by themselves.)
            let hot = kind == "spark" && first[0] >= first[2] * 1.5 && first[0] >= first[1] * 0.9;
            if !hot {
                put("endcolor", rgb(last.map(|v| (v / peak).min(1.0))));
            }
            if kind == "spark" && !hot {
                put("cold", "1".into());
            }
            put("intensity", bright(if kind == "spark" { SPARKS } else { GLOW }));
            put("collide", "0".into());
        }
        "fire" => {
            // (Flames cool from yellow to red by themselves: their tint alone.)
            put("color", rgb(first.map(|v| v / peak)));
            put("intensity", bright(FLAMES));
            put("collide", "0".into());
        }
        "dirt" | "dust" => {
            // (In the ground's colour, darkened as its own ramp darkens it.)
            put("ground", "1".into());
            let shade = (0.35 + 0.65 * peak.sqrt()).min(1.0);
            put("color", rgb([0.55, 0.5, 0.45].map(|v| v * shade)));
            put("intensity", number(if kind == "dirt" { 0.95 } else { (0.25 + 0.4 * r0[3]).min(0.6) }));
        }
        _ => {
            put("color", rgb(first));
            put("endcolor", rgb(last));
            // (As opaque as its ramp is at its most: smoke fades in.)
            put("intensity", number((most * (0.25 + 0.75 * (cover * 2.0).min(1.0)) * 0.7).clamp(0.08, 0.8)));
        }
    }
    if role == Role::Trail {
        put("inherit", "0.15".into());
    } else {
        // (Its births spread over its own life, a second at most.)
        let over = (e.lifetime.max(0.0) / TICKS).min(1.0);
        if over > 0.05 {
            put("over", number(over));
        }
    }
    Ok(Some((s, n)))
}

/// The effect of Forged Alliance's emitters `list` (each read once into `cache`) played as `role`
/// at `scale`: its sections' text, none where it draws nothing.
fn effect(files: &Files, list: &[String], role: Role, scale: f64, cache: &mut BTreeMap<String, Option<Emitter>>, pictures: &mut Pictures) -> Result<Option<String>> {
    let mut bursts: Vec<(String, String, f64)> = Vec::new();
    for path in list {
        if bursts.len() >= MOST_BURSTS {
            break;
        }
        if !cache.contains_key(path) {
            let read = fx::emitter(files, path)?;
            cache.insert(path.clone(), read);
        }
        let Some(Emitter::Particles(e)) = cache.get(path).cloned().flatten() else { continue };
        if let Some((body, count)) = burst(files, &e, role, scale, pictures)? {
            let stem = path.rsplit('/').next().unwrap_or(path).trim_end_matches(".bp").trim_end_matches("_emit");
            let mut name: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
            if bursts.iter().any(|(n, ..)| *n == name) {
                name = format!("{name}_{}", bursts.len() + 1);
            }
            bursts.push((name, body, count));
        }
    }
    if bursts.is_empty() {
        return Ok(None);
    }
    // (Held to so many particles at once: each burst thinned alike.)
    let total: f64 = bursts.iter().map(|b| b.2).sum();
    let mut s = String::new();
    for (name, body, count) in &bursts {
        let body = if role != Role::Trail && total > MOST_IN_AN_EFFECT && *count > 1.0 {
            let thinned = (count * MOST_IN_AN_EFFECT / total).round().max(1.0);
            body.replace(&format!("\t\tcount={};\n", number(*count)), &format!("\t\tcount={};\n", number(thinned)))
        } else {
            body.clone()
        };
        let _ = write!(s, "\t[{name}]\n\t\t{{\n{body}\t\t}}\n");
    }
    Ok(Some(s))
}

/// A beam's look (`BeamBlueprint`): its colour (its picture's through its start colour, linear,
/// its brightest at 1), a core whiter than it, and how thick it is (pixels across).
fn beam_look(files: &Files, beam: &fx::Beam, scale: f64, pictures: &mut Pictures) -> Result<([f64; 3], [f64; 3], f64)> {
    let tex = if beam.texture.is_empty() { None } else { pictures.picture(files, &beam.texture)? };
    let tex = tex.map_or([1.0; 3], |p| p.colour);
    let c = linear([tex[0] * beam.start[0], tex[1] * beam.start[1], tex[2] * beam.start[2]]);
    let peak = c[0].max(c[1]).max(c[2]).max(1e-3);
    let colour = c.map(|v| v / peak);
    let core = colour.map(|v| v + (1.0 - v) * 0.6);
    Ok((colour, core, (beam.thickness * UNIT_SCALE * 2.0 * scale).clamp(1.0, 48.0)))
}

/// A ribbon's look (`TrailEmitterBlueprint`): its colour (its picture's through its ramp's start,
/// linear, its brightest at 1) and how wide it is (pixels).
fn ribbon_look(files: &Files, ribbon: &fx::Ribbon, pictures: &mut Pictures) -> Result<([f64; 3], f64)> {
    let tex = if ribbon.texture.is_empty() { None } else { pictures.picture(files, &ribbon.texture)? };
    let tex = tex.map_or([1.0; 3], |p| p.colour);
    let ramp = if ribbon.ramp.is_empty() { None } else { pictures.ramp(files, &ribbon.ramp)? };
    let r = ramp.map_or([1.0; 4], |r| r.start);
    let c = linear([tex[0] * r[0], tex[1] * r[1], tex[2] * r[2]]);
    let peak = c[0].max(c[1]).max(c[2]).max(1e-3);
    Ok((c.map(|v| v / peak), (ribbon.size * UNIT_SCALE).clamp(0.5, 24.0)))
}

/// Whether TA Reborn draws the weapon's projectile as a model (`units::weapon_section`'s render
/// types 1 and 6: missiles, torpedoes, bombs, nukes, anti-missiles, anti-air missiles).
pub fn drawn_as_a_model(w: &WeaponBlueprint) -> bool {
    match units::kind(w) {
        Kind::Missile | Kind::Torpedo | Kind::Bomb | Kind::Nuke | Kind::AntiMissile => true,
        Kind::AntiAir => w.shot.as_ref().is_some_and(|s| s.is("MISSILE")),
        _ => false,
    }
}

/// The name of a projectile's model (its blueprint's folder: `tifmissilecruise01`).
pub fn model_name(projectile: &str) -> String {
    projectile.trim_end_matches("_proj.bp").rsplit('/').next().unwrap_or(projectile).to_ascii_lowercase()
}

/// The mesh of a projectile's blueprint (`projectiles/<name>/<name>_lod0.scm`).
fn mesh_path(projectile: &str) -> String {
    format!("{}_lod0.scm", projectile.trim_end_matches("_proj.bp"))
}

/// An effect of `body` for `side`, named after `stem` and `what` (`UEF_TDFGAUSS01_HIT`): the name
/// of one of the same text there already, else a new one.
fn named(effects: &mut BTreeMap<String, String>, names: &mut BTreeSet<String>, side: &str, stem: &str, what: &str, body: String) -> String {
    if let Some(name) = effects.get(&body) {
        return name.clone();
    }
    let clean: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
    let mut name = format!("{side}_{clean}_{what}");
    let mut n = 2;
    while names.contains(&name) {
        name = format!("{side}_{clean}_{what}{n}");
        n += 1;
    }
    names.insert(name.clone());
    effects.insert(body, name.clone());
    name
}

/// The factions' weapons' effects, from the player's copy of the game (`units`: those made).
pub fn make(files: &Files, units: &[UnitBlueprint]) -> Result<Made> {
    let lib = Library::read(files)?;
    let mut made = Made::default();
    let mut cache: BTreeMap<String, Option<Emitter>> = BTreeMap::new();
    let mut pictures = Pictures::default();
    let mut shots: BTreeMap<String, ShotFx> = BTreeMap::new();
    let mut models: BTreeSet<String> = BTreeSet::new();
    // Each faction's effects (by their text: one of the same text named once) and its names.
    let mut by_side: BTreeMap<&'static str, (BTreeMap<String, String>, BTreeSet<String>)> = BTreeMap::new();
    for u in units {
        let side = units::side(&u.faction);
        let id = u.id.to_ascii_lowercase();
        let classes: BTreeMap<String, WeaponFx> = match files.read(&format!("units/{id}/{id}_script.lua"))? {
            Some(bytes) => lib.weapons(&String::from_utf8_lossy(&bytes)),
            None => BTreeMap::new(),
        };
        for (i, w) in u.weapons.iter().enumerate() {
            let weapon_fx = classes.get(&w.label).cloned().unwrap_or_default();
            let shot = match w.projectile.as_deref().map(crate::projectile::normalise) {
                Some(path) => {
                    if !shots.contains_key(&path) {
                        let script = format!("{}_script.lua", path.trim_end_matches("_proj.bp"));
                        let fx = match files.read(&script)? {
                            Some(bytes) => lib.shot(&String::from_utf8_lossy(&bytes)),
                            None => ShotFx::default(),
                        };
                        shots.insert(path.clone(), fx);
                    }
                    Some((path.clone(), shots[&path].clone()))
                }
                None => None,
            };
            let (effects, names) = by_side.entry(side).or_default();
            let look = weapon_look(files, w, &weapon_fx, shot.as_ref(), side, effects, names, &mut cache, &mut pictures)?;
            // (Its projectile's own model, where TA Reborn draws it as one and the game has its mesh.)
            let mut look = look;
            if let Some((path, _)) = &shot
                && drawn_as_a_model(w)
                && files.read(&mesh_path(path))?.is_some()
            {
                look.model = Some(model_name(path));
                models.insert(path.clone());
            }
            if look != Look::default() {
                made.looks.insert((u.id.clone(), i), look);
            }
        }
    }
    for f in Faction::PLAYABLE {
        let side = units::side(&f);
        let Some((effects, _)) = by_side.get(side) else { continue };
        if effects.is_empty() {
            continue;
        }
        // (In their names' order.)
        let mut sections: Vec<(&String, &String)> = effects.iter().map(|(body, name)| (name, body)).collect();
        sections.sort();
        let mut text = String::new();
        for (name, body) in sections {
            let _ = write!(text, "[{name}]\n\t{{\n{body}\t}}\n");
        }
        made.files.push((format!("effects/supcom_{}.tdf", side.to_ascii_lowercase()), text));
    }
    made.models = models.into_iter().collect();
    // The factions' adjacency beams.
    for (side, path) in ADJACENCY_BEAMS {
        if let Some(Emitter::Beam(beam)) = fx::emitter(files, path)? {
            let (colour, _, thickness) = beam_look(files, &beam, 0.5, &mut pictures)?;
            made.adjacency.insert(side.to_string(), (colour, thickness.clamp(1.5, 6.0)));
        }
    }
    Ok(made)
}

/// The beams Forged Alliance draws between a faction's structures that help each other (its
/// `CreateAdjacencyBeams`), by the side they are drawn for (the Seraphim's draw none).
const ADJACENCY_BEAMS: [(&str, &str); 3] = [("UEF", "effects/emitters/adjacency_uef_beam_01_emit.bp"), ("CYBRAN", "effects/emitters/adjacency_cybran_beam_01_emit.bp"), ("AEON", "effects/emitters/adjacency_aeon_beam_01_emit.bp")];

/// How weapon `w` looks (its muzzle, its shot's trail, look and impacts, its beam), its effects
/// named among `side`'s.
#[allow(clippy::too_many_arguments)]
fn weapon_look(files: &Files, w: &WeaponBlueprint, weapon: &WeaponFx, shot: Option<&(String, ShotFx)>, side: &str, effects: &mut BTreeMap<String, String>, names: &mut BTreeSet<String>, cache: &mut BTreeMap<String, Option<Emitter>>, pictures: &mut Pictures) -> Result<Look> {
    let mut look = Look::default();
    let kind = units::kind(w);
    // (The death weapons and blasts are no shots.)
    if w.category.eq_ignore_ascii_case("death") || w.category.eq_ignore_ascii_case("kamikaze") {
        return Ok(look);
    }
    let class = if weapon.class.is_empty() { w.label.clone() } else { weapon.class.clone() };
    // At its muzzle: its flash, or its beam's start.
    let mut fire: Vec<String> = weapon.muzzle.clone();
    if let Some(beam) = &weapon.beam {
        fire.extend(beam.start.iter().cloned());
    }
    if let Some(body) = effect(files, &fire, Role::Fire, weapon.muzzle_scale, cache, pictures)? {
        look.keys.push(("fireeffect", named(effects, names, side, &class, "FIRE", body)));
    }
    let stem = shot.map_or(class.clone(), |(path, _)| model_name(path));
    // A beam's colours and width, and what shows where it ends.
    if let Some(beam) = &weapon.beam {
        let first = match beam.beam.first() {
            Some(path) => cache_emitter(files, path, cache)?,
            None => None,
        };
        if let Some(Emitter::Beam(b)) = first {
            let (colour, core, thickness) = beam_look(files, &b, 1.0, pictures)?;
            look.keys.push(("rgbcolor", rgb(colour)));
            look.keys.push(("rgbcolor2", rgb(core)));
            look.keys.push(("thickness", number(thickness)));
        }
        if let Some(body) = effect(files, &beam.end, Role::Hit, 1.0, cache, pictures)? {
            look.keys.push(("hiteffect", named(effects, names, side, &format!("{class}_beam"), "HIT", body)));
        }
        return Ok(look);
    }
    let Some((_, fx)) = shot else { return Ok(look) };
    // Its shot in flight: a bolt's or a shell's colours and size, from its beam (a bolt's look)
    // or its ribbon (a shell's tail) and its brightest glow.
    if matches!(kind, Kind::Laser | Kind::Shell | Kind::Artillery | Kind::AntiAir) && !drawn_as_a_model(w) {
        let mut colour: Option<[f64; 3]> = None;
        let mut tail: Option<[f64; 3]> = None;
        let mut thickness: Option<f64> = None;
        if let Some(path) = fx.beams.first()
            && let Some(Emitter::Beam(b)) = cache_emitter(files, path, cache)?
        {
            let (c, core, t) = beam_look(files, &b, 1.0, pictures)?;
            colour = Some(c);
            tail = Some(core);
            thickness = Some(t);
        }
        // (A ribbon's colour: a shell's tail, a bolt's own where nothing else gives it; its width
        // in Forged Alliance's terms is no bolt's, which keeps TA Reborn's for its damage.)
        if let Some(path) = fx.ribbons.first()
            && let Some(Emitter::Ribbon(r)) = cache_emitter(files, path, cache)?
        {
            let (c, _) = ribbon_look(files, &r, pictures)?;
            tail = tail.or(Some(c));
            colour = colour.or(Some(c));
        }
        if let Some((c, size)) = brightest_glow(files, &fx.trails, cache, pictures)? {
            colour = Some(c);
            if kind != Kind::Laser {
                thickness = Some(size.clamp(3.0, 48.0));
            }
        } else if kind != Kind::Laser
            && let Some(c) = glowing_mesh(files, w, pictures)?
        {
            // (A shell Forged Alliance draws as a glowing mesh: its paint's colour.)
            colour = Some(c);
        }
        // (A bolt's core: its colour, whiter, where no beam gives it.)
        if kind == Kind::Laser && fx.beams.is_empty() {
            tail = colour.map(|c| c.map(|v| v + (1.0 - v) * 0.55));
        }
        if let Some(c) = colour {
            look.keys.push(("rgbcolor", rgb(c)));
        }
        if let Some(c) = tail {
            look.keys.push(("rgbcolor2", rgb(c)));
        }
        if let Some(t) = thickness {
            look.keys.push(("thickness", number(t)));
        }
    }
    // Along its flight, and where it lands.
    if let Some(body) = effect(files, &fx.trails, Role::Trail, fx.trail_scale, cache, pictures)? {
        look.keys.push(("traileffect", named(effects, names, side, &stem, "TRAIL", body)));
    }
    let (land, scale) = if fx.land.is_empty() { (&fx.unit, fx.unit_scale) } else { (&fx.land, fx.land_scale) };
    let hit = effect(files, land, Role::Hit, scale, cache, pictures)?;
    if let Some(body) = hit.clone() {
        look.keys.push(("hiteffect", named(effects, names, side, &stem, "HIT", body)));
    }
    if !fx.water.is_empty()
        && let Some(body) = effect(files, &fx.water, Role::Hit, fx.water_scale, cache, pictures)?
        && Some(&body) != hit.as_ref()
    {
        look.keys.push(("waterhiteffect", named(effects, names, side, &stem, "WATER", body)));
    }
    Ok(look)
}

/// The colour of a shot's glowing mesh (its blueprint's shader one that glows, `TMeshGlow`...):
/// its paint's on average (linear, its brightest at 1); none where it has none.
fn glowing_mesh(files: &Files, w: &WeaponBlueprint, pictures: &mut Pictures) -> Result<Option<[f64; 3]>> {
    let Some(shot) = w.shot.as_ref() else { return Ok(None) };
    if !shader(shot).to_ascii_lowercase().contains("glow") {
        return Ok(None);
    }
    let albedo = format!("{}_albedo.dds", shot.path.trim_end_matches("_proj.bp"));
    let Some(p) = pictures.picture(files, &albedo)? else { return Ok(None) };
    let c = linear(p.colour);
    let peak = c[0].max(c[1]).max(c[2]);
    Ok((peak > 1e-3).then(|| c.map(|v| v / peak)))
}

/// The shader a projectile's mesh is drawn with (its first level of detail's `ShaderName`).
fn shader(shot: &crate::projectile::ProjectileBlueprint) -> String {
    shot.table
        .path(&["Display", "Mesh", "LODs"])
        .and_then(|l| l.array.first())
        .and_then(crate::lua::Value::table)
        .and_then(|lod| lod.get_ci("ShaderName"))
        .and_then(crate::lua::Value::str)
        .unwrap_or("")
        .to_string()
}

/// The emitter at `path`, read once into `cache`.
fn cache_emitter(files: &Files, path: &str, cache: &mut BTreeMap<String, Option<Emitter>>) -> Result<Option<Emitter>> {
    if !cache.contains_key(path) {
        let read = fx::emitter(files, path)?;
        cache.insert(path.to_string(), read);
    }
    Ok(cache[path].clone())
}

/// The brightest glowing emitter a shot carries (its colour, linear, its brightest at 1; its size
/// across, pixels): the shot's own look in flight, where its trails draw it.
fn brightest_glow(files: &Files, trails: &[String], cache: &mut BTreeMap<String, Option<Emitter>>, pictures: &mut Pictures) -> Result<Option<([f64; 3], f64)>> {
    let mut best: Option<([f64; 3], f64, f64)> = None;
    for path in trails {
        let Some(Emitter::Particles(e)) = cache_emitter(files, path, cache)? else { continue };
        if matches!(e.blend, 0 | 4 | 5) {
            continue;
        }
        let picture = if e.texture.is_empty() { None } else { pictures.picture(files, &e.texture)? };
        let ramp = if e.ramp.is_empty() { None } else { pictures.ramp(files, &e.ramp)? };
        let tex = picture.map_or([1.0; 3], |p| p.colour);
        let r = ramp.map_or([1.0; 4], |r| r.start);
        let c = linear([tex[0] * r[0], tex[1] * r[1], tex[2] * r[2]]);
        let peak = c[0].max(c[1]).max(c[2]);
        let size = e.curve("StartSizeCurve").mean(0.0).0 * UNIT_SCALE;
        // (The brightest, then the largest.)
        let score = peak * picture.map_or(0.5, |p| p.cover).max(0.05);
        if peak > 1e-3 && best.is_none_or(|b| score > b.2 + 1e-9) {
            best = Some((c.map(|v| v / peak), size, score));
        }
    }
    Ok(best.map(|(c, size, _)| (c, size)))
}

/// Files made, each its path in the mod and its bytes.
pub type ModFiles = Vec<(String, Vec<u8>)>;

/// A projectile's model, its surface and its paint (`objects3d/<name>.3do`, `.surface`,
/// `textures/supcom/<name>_*.dds`), made of its mesh as a unit's is (`model::convert`), at its
/// blueprint's scale; its paint glowing where Forged Alliance's shader lights it so (a glowing
/// mesh's). None where its mesh does not read.
pub fn projectile_model(files: &Files, path: &str, shot: &crate::projectile::ProjectileBlueprint) -> Result<Option<ModFiles>> {
    let Some(bytes) = files.read(&mesh_path(path))? else { return Ok(None) };
    let Ok(mesh) = crate::scm::parse(&bytes) else { return Ok(None) };
    let name = model_name(path);
    let stem = path.trim_end_matches("_proj.bp");
    let scale = shot.table.path(&["Display"]).and_then(|d| d.get_ci("UniformScale")).and_then(crate::lua::Value::num).filter(|s| *s > 0.0).unwrap_or(0.1);
    let glows = shader(shot).to_ascii_lowercase().contains("glow");
    let mut paint: Vec<(String, Vec<u8>)> = Vec::new();
    for (kind, file, plain) in [("albedo", "albedo", textures::PLAIN_ALBEDO), ("specteam", "specteam", textures::PLAIN_SPECULAR), ("normals", "normalsts", textures::PLAIN_NORMALS)] {
        let source = files.read(&format!("{stem}_{file}.dds"))?;
        let bytes = match (&source, kind) {
            // (A glowing mesh's paint glows all over: its specular map's blue.)
            (_, "specteam") if glows => textures::plain([0, 64, 255, 0]),
            (Some(b), _) => textures::texture(b).unwrap_or_else(|_| textures::plain(plain)),
            (None, _) => textures::plain(plain),
        };
        paint.push((format!("textures/supcom/{name}_{kind}.dds"), bytes));
    }
    let material = ta_formats::surface::Material { name: name.clone(), albedo: paint[0].0.clone(), specular: paint[1].0.clone(), normals: paint[2].0.clone(), mean: textures::mean(&paint[0].1) };
    paint[0].1 = textures::opaque(&paint[0].1);
    let Ok(converted) = model::convert(&mesh, scale, material) else { return Ok(None) };
    let mut out = paint;
    out.push((format!("objects3d/{name}.3do"), converted.model.to_bytes()));
    out.push((format!("objects3d/{name}.surface"), converted.surface.to_bytes()));
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn particles(texture: &str, blend: i64, flat: bool, curves: &[(&str, &[fx::Key])]) -> Particles {
        Particles {
            path: "effects/emitters/test_emit.bp".into(),
            lifetime: 1.0,
            blend,
            texture: texture.into(),
            ramp: String::new(),
            flat,
            gravity: false,
            resistance: false,
            high_fidelity: true,
            curves: curves.iter().map(|(n, keys)| (n.to_string(), fx::Curve { range: 1.0, keys: keys.to_vec() })).collect(),
        }
    }

    /// What an emitter is drawn as, by its picture and its blend: a flare glows, smoke painted
    /// over is smoke, a flat ring runs out over the ground (one standing up is not drawn), a
    /// distortion is not drawn, streaks flying are sparks and standing they glint.
    #[test]
    fn an_emitter_is_drawn_as_its_picture_says() {
        let e = |t: &str, blend: i64, flat: bool| particles(&format!("textures/particles/{t}.dds"), blend, flat, &[]);
        assert_eq!(kind_of(&e("flare_lens_add_02", 3, false), 0.0), Some("glow"));
        assert_eq!(kind_of(&e("cloud_smoke_alpha_01", 0, false), 0.0), Some("smoke"));
        assert_eq!(kind_of(&e("ring_white_01", 0, true), 0.0), Some("ring"));
        assert_eq!(kind_of(&e("ring_white_01", 3, false), 0.0), None);
        assert_eq!(kind_of(&e("ring_distort_01", 5, true), 0.0), None);
        assert_eq!(kind_of(&e("sparkle_white_add_01", 3, false), 200.0), Some("spark"));
        assert_eq!(kind_of(&e("sparkle_white_add_01", 3, false), 0.0), Some("glow"));
        assert_eq!(kind_of(&e("dirt_chunks_04", 0, false), 0.0), Some("dirt"));
        assert_eq!(kind_of(&e("fire_flame_add_03", 3, false), 0.0), Some("fire"));
        assert_eq!(kind_of(&e("water_splash_alpha_01", 0, false), 0.0), Some("spray"));
    }

    /// An emitter's curves at TA Reborn's scale: its rate over its life, its particles' lives in
    /// seconds (ticks a tenth of one), their sizes and speeds in pixels (an ogrid 32, a tick a
    /// tenth of a second), up and forward kept, its left made right.
    #[test]
    fn an_emitter_is_made_a_burst_at_reborns_scale() {
        let files = Files::of(Vec::new(), Vec::new());
        let mut e = particles(
            "",
            3,
            false,
            &[
                ("EmitRateCurve", &[[0.0, 6.0, 0.0]]),
                ("LifetimeCurve", &[[0.0, 4.0, 2.0]]),
                ("VelocityCurve", &[[0.0, 0.5, 0.0]]),
                ("StartSizeCurve", &[[0.0, 0.25, 0.0]]),
                ("EndSizeCurve", &[[0.0, 0.5, 0.0]]),
                ("YDirectionCurve", &[[0.0, 1.0, 0.2]]),
                ("XPosCurve", &[[0.0, 0.5, 0.0]]),
                ("YAccelCurve", &[[0.0, -0.01, 0.0]]),
            ],
        );
        e.lifetime = 2.0;
        let (body, count) = burst(&files, &e, Role::Hit, 1.0, &mut Pictures::default()).unwrap().unwrap();
        let get = |key: &str| body.lines().find_map(|l| l.trim().strip_prefix(&format!("{key}=")).map(|v| v.trim_end_matches(';').to_string()));
        assert_eq!((get("kind").as_deref(), count), (Some("glow"), 12.0), "six a tick for two ticks");
        assert_eq!(get("life").as_deref(), Some("0.3 0.5"), "four ticks, less and more one");
        assert_eq!(get("size").as_deref(), Some("8 16"));
        assert_eq!(get("speed").as_deref(), Some("144 176"), "half an ogrid a tick along a way of length 1, its range a tenth either side");
        assert_eq!(get("direction").as_deref(), Some("up"));
        assert_eq!(get("offset").as_deref(), Some("-16 0 0"), "its left, our right");
        assert_eq!(get("gravity").as_deref(), Some("32"));
        // A trail's count is a second's.
        let (_, a_second) = burst(&files, &e, Role::Trail, 1.0, &mut Pictures::default()).unwrap().unwrap();
        assert_eq!(a_second, 60.0);
        // Scaled, it is bigger and faster, as many.
        let (big, n) = burst(&files, &e, Role::Hit, 2.0, &mut Pictures::default()).unwrap().unwrap();
        assert!(big.contains("size=16 32;") && n == 12.0, "{big}");
    }

    /// A glow whose ramp fades it in from black takes the ramp's colour where it is brightest (at
    /// its birth it would add nothing); one black throughout is no burst at all; smoke keeps the
    /// black it is born (dark smoke turning grey).
    #[test]
    fn a_glow_faded_in_from_black_takes_its_brightest_colour() {
        // (Ramps of 16 texels, blue, green, red and alpha: two black and clear, then green, growing
        // opaque; and black throughout.)
        let ramp = |texel: &dyn Fn(u32) -> [u8; 4]| {
            let mut file = ta_formats::dds::header(16, 1, 1, None);
            for i in 0..16 {
                file.extend_from_slice(&texel(i));
            }
            file
        };
        let fading = ramp(&|i| if i < 2 { [0; 4] } else { [47, 123, 24, (i * 16) as u8] });
        let black = ramp(&|_| [0, 0, 0, 255]);
        let dir = crate::tests::Temp::new("effects_ramps");
        let entries: [(&str, &[u8], bool); 2] = [("textures/particles/test_fading.dds", &fading, true), ("textures/particles/test_black.dds", &black, true)];
        std::fs::write(dir.0.join("effects.scd"), crate::scd::tests::zip(&entries)).unwrap();
        let files = Files::of(Vec::new(), vec![crate::scd::Scd::open(&dir.0.join("effects.scd")).unwrap()]);
        let curves: &[(&str, &[fx::Key])] = &[("EmitRateCurve", &[[0.0, 4.0, 0.0]]), ("LifetimeCurve", &[[0.0, 5.0, 0.0]]), ("StartSizeCurve", &[[0.0, 1.0, 0.0]])];
        let made = |texture: &str, blend: i64, ramp: &str| {
            let mut e = particles(texture, blend, false, curves);
            e.ramp = format!("/textures/particles/{ramp}.dds");
            burst(&files, &e, Role::Hit, 1.0, &mut Pictures::default()).unwrap().map(|(body, _)| body)
        };
        let get = |body: &str, key: &str| body.lines().find_map(|l| l.trim().strip_prefix(&format!("{key}=")).map(|v| v.trim_end_matches(';').to_string()));
        let glow = made("", 3, "test_fading").expect("a glow");
        let colour: Vec<f64> = get(&glow, "color").unwrap().split(' ').map(|v| v.parse().unwrap()).collect();
        assert!(colour[1] == 1.0 && colour[0] < 0.1 && colour[2] < 0.2, "green, as the ramp is where it shows: {glow}");
        assert!(made("", 3, "test_black").is_none(), "black throughout: nothing to draw");
        let smoke = made("/textures/particles/cloud_smoke_alpha_test.dds", 0, "test_fading").expect("smoke");
        assert_eq!(get(&smoke, "color").as_deref(), Some("0 0 0"), "{smoke}");
    }

    /// Numbers as the files write them.
    #[test]
    fn numbers_are_written_short() {
        assert_eq!((number(1.0), number(0.5), number(0.12345), number(-0.0001), number(160.0)), ("1".into(), "0.5".into(), "0.123".into(), "0".into(), "160".into()));
        assert_eq!(rgb([1.0, 0.25, 0.0]), "1 0.25 0");
        assert_eq!(model_name("projectiles/tifmissilecruise01/tifmissilecruise01_proj.bp"), "tifmissilecruise01");
    }
}
