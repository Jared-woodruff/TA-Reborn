//! Reborn's detail kit: new geometry fitted at load to the installed model's own faces, so
//! the shapes, the paint and the rig stay the original's. Large flat faces become raised
//! armour plates with a dark seam round each and, on the big ones, a bolt at every corner;
//! the outer sides of tracks, painted with their wheels, get road wheels.
//!
//! It reads nothing but the faces, so it works on every model, mods' included, and it leaves
//! alone what it can't place well: small, curved, thin or bent faces, lights, decals (a face
//! painted over another, in its piece or another) and faces no plate fits. Nothing here is
//! drawn from TA's art; the plates take their paint from the face they stand on.

use glam::{Vec2, Vec3};

use super::{FAR_FROM_EDGES, ModelVertex, f16_bits};

/// What a face is, to the kit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FaceKind {
    /// Painted plating: a plate where it is big enough, a grid of them on a big face.
    Plating,
    /// Painted with a picture of its own that a grid of plates would cut (a vent panel, an
    /// emblem): one plate at most, and no bolts on it.
    Picture,
    /// A track's side, painted with its wheels (TA's `Tredside` textures).
    TrackSide,
    /// Left as it is: lights, the treads' other faces, logos and markings.
    Keep,
}

/// A face of a piece as the kit sees it.
pub(super) struct KitFace<'a> {
    /// Its corners as the model has them (positions, paint, edges), in order.
    pub vertices: &'a [ModelVertex],
    /// Outwards.
    pub normal: Vec3,
    pub kind: FaceKind,
    /// The average colour of its paint (sRGB): its seams are a dark shade of it. For a
    /// player's colour, the grey that the colour multiplies (see `team`).
    pub paint: [u8; 3],
    /// Painted in the player's colour: its seams are drawn in a dark shade of it too.
    pub team: bool,
    /// Which of its edges (from each corner to the next) no other face meets.
    pub open: &'a [bool],
    /// Which of its edges are where it meets another plate of the same face (a big face laid
    /// with a grid of plates, see `parts`): a groove between the two, not a chamfer down to the
    /// face's own edge.
    pub joined: &'a [bool],
    /// Its texture's paint, where the cuts across a big face may run (see `parts`).
    pub painted: Option<&'a PaintMap>,
    /// Added to make a form whole (see `FaceSource::late` in model.rs): TA's own faces are fitted
    /// as if it weren't there (it lays no ceiling on them, is no track side, gets no bolts), so
    /// they look as they did.
    pub late: bool,
}

/// A texture's paint at its own size, as its lightness a little smoothed (TA's dither evened
/// out): where the cuts laying a big face with several plates may run without crossing what is
/// painted on it, its lamps, emblems, stripes and doors (see `parts`).
pub(super) struct PaintMap {
    w: usize,
    h: usize,
    light: Vec<f32>,
}

impl PaintMap {
    /// From a texture's lightness (0..1), `w` by `h`, row by row.
    pub fn new(w: usize, h: usize, light: &[f32]) -> Option<PaintMap> {
        if w == 0 || h == 0 || light.len() != w * h {
            return None;
        }
        let at = |x: isize, y: isize| light[y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize];
        let light = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as isize, (i / w) as isize);
                (at(x, y) * 4.0 + (at(x - 1, y) + at(x + 1, y) + at(x, y - 1) + at(x, y + 1)) * 2.0 + at(x - 1, y - 1) + at(x + 1, y - 1) + at(x - 1, y + 1) + at(x + 1, y + 1)) / 16.0
            })
            .collect();
        Some(PaintMap { w, h, light })
    }

    /// Its texels across (u) and down (v).
    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// The lightness at texture coordinates `uv` (bilinear).
    fn at(&self, uv: Vec2) -> f32 {
        let p = (uv.clamp(Vec2::ZERO, Vec2::ONE) * Vec2::new(self.w as f32, self.h as f32) - 0.5).max(Vec2::ZERO);
        let (x0, y0) = (p.x.floor() as usize, p.y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (p.x - x0 as f32, p.y - y0 as f32);
        let l = |x: usize, y: usize| self.light[y * self.w + x];
        let top = l(x0.min(self.w - 1), y0.min(self.h - 1)) * (1.0 - fx) + l(x1, y0.min(self.h - 1)) * fx;
        let bottom = l(x0.min(self.w - 1), y1) * (1.0 - fx) + l(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// How much the paint changes along v (`along_v`) or along u, from texel to texel on
    /// average, but for each line's change from end to end: what drawing the paint out that way
    /// turns into streaks (a ramp across it, or stripes along it, nothing).
    pub fn streaky(&self, along_v: bool) -> f32 {
        let (lines, n) = if along_v { (self.w, self.h) } else { (self.h, self.w) };
        let l = |line: usize, k: usize| if along_v { self.light[k * self.w + line] } else { self.light[line * self.w + k] };
        let mut total = 0.0;
        for line in 0..lines {
            let changes: f32 = (1..n).map(|k| (l(line, k) - l(line, k - 1)).abs()).sum();
            total += (changes - (l(line, n - 1) - l(line, 0)).abs()) / (n - 1).max(1) as f32;
        }
        total / lines.max(1) as f32
    }

    /// The few lines of the paint (`lines` of them, side by side) running along u (`along_u`)
    /// or along v that draw out best into stripes: the most painted along them (lines, panel
    /// edges, the paint's tones), the least changing from one to the next, and nearest the
    /// paint's own lightness on the whole. Their start (0..1).
    pub fn stripe_lines(&self, along_u: bool, lines: usize) -> f32 {
        let (count, n) = if along_u { (self.h, self.w) } else { (self.w, self.h) };
        let l = |line: usize, k: usize| if along_u { self.light[line * self.w + k] } else { self.light[k * self.w + line] };
        let mean = self.light.iter().sum::<f32>() / self.light.len().max(1) as f32;
        let lines = lines.clamp(1, count);
        let score = |first: usize| {
            let band = first..first + lines;
            let rich: f32 = band.clone().map(|a| (1..n).map(|k| (l(a, k) - l(a, k - 1)).abs()).sum::<f32>()).sum::<f32>() / lines as f32;
            let unlike: f32 = band.clone().skip(1).map(|a| (0..n).map(|k| (l(a, k) - l(a - 1, k)).abs()).sum::<f32>()).sum::<f32>() / (lines - 1).max(1) as f32;
            let light = band.map(|a| (0..n).map(|k| l(a, k)).sum::<f32>()).sum::<f32>() / (lines * n) as f32;
            rich - 2.0 * unlike - (light - mean).abs() * n as f32
        };
        let best = (0..=count - lines).max_by(|&a, &b| score(a).total_cmp(&score(b))).unwrap_or(0);
        best as f32 / count as f32
    }

    /// How much is painted along the line from `a` to `b` (texture coordinates): the mean change
    /// of its lightness from one sample to the next, about a texel apart, but for the change from
    /// end to end (a gradient along it paints nothing in its way).
    fn busy(&self, a: Vec2, b: Vec2) -> f32 {
        let span = ((b - a) * Vec2::new(self.w as f32, self.h as f32)).length();
        let n = (span.ceil() as usize).clamp(2, 128);
        let first = self.at(a);
        let mut previous = first;
        let mut total = 0.0;
        for k in 1..=n {
            let l = self.at(a.lerp(b, k as f32 / n as f32));
            total += (l - previous).abs();
            previous = l;
        }
        (total - (previous - first).abs()) / n as f32
    }
}

/// A face of the model at rest (model space), for the faces of its other pieces lying on one
/// of a piece's: see `Around`.
pub(super) struct Neighbour {
    pub piece: usize,
    outline: Outline,
    normal: Vec3,
    /// How far its corners are from its middle at most.
    reach: f32,
}

impl Neighbour {
    /// None for a face that isn't flat and convex.
    pub fn new(piece: usize, corners: Vec<Vec3>, normal: Vec3) -> Option<Neighbour> {
        let outline = convex(corners, normal)?;
        let reach = outline.corners.iter().map(|c| c.distance(outline.centre)).fold(0.0, f32::max);
        Some(Neighbour { piece, outline, normal, reach })
    }
}

/// What the kit knows of the model around the piece it details.
#[derive(Default)]
pub(super) struct Around<'a> {
    /// The piece's place in the model at rest.
    pub piece: usize,
    pub offset: Vec3,
    /// Every face of the model at rest (model space).
    pub faces: &'a [Neighbour],
}

/// A piece's detailed surface: its faces (some made plates) and what the kit added.
#[derive(Default)]
pub(super) struct Kit {
    pub vertices: Vec<ModelVertex>,
    pub indices: Vec<u32>,
    pub plates: usize,
    pub bolts: usize,
    pub wheels: usize,
}

/// How much the kit may add to one model (bolts and wheels are many triangles each).
#[derive(Clone, Copy)]
pub(super) struct Budget {
    pub bolts: usize,
    pub wheels: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { bolts: 48, wheels: 32 }
    }
}

/// The smallest face (in square model units) that becomes a plate, and the smallest that
/// also gets bolts.
const PLATE_AREA: f32 = 5.0;
const BOLT_AREA: f32 = 20.0;
/// A plate is about this long at most: a bigger four-sided face is laid with a grid of them.
const PLATE_SPAN: f32 = 12.0;
/// How much the paint may change along a cut between plates (its mean change of lightness from
/// one texel to the next): busier, the cut would cross painted detail, and is left out. And how
/// much a cut prefers its own share of the face to a calmer place further off.
const CALM: f32 = 0.015;
const CUT_DRIFT: f32 = 0.01;
/// Corners whose normals were smoothed (a curved surface) further than this from the face's
/// own normal keep the face as it is.
const FLAT: f32 = 0.995;
const WEIGHT_RIM: i8 = 96;
/// The untextured layer word: no texture, one frame, a physical finish (see `ModelVertex`).
const PLAIN: u32 = 0xFFFF | (1 << 16) | (1 << 28);
/// The layer word's bit for a colour the shader multiplies by the player's (`vs_main`).
const TEAM_PAINT: u32 = 1 << 26;
/// The layer word's bit for a road wheel's hub: its uv runs across the hub, and the shader turns
/// it as the wheel rolls and draws lug nuts on it (`kit_tread`). The track's band, running, needs
/// none: its uv runs round it (see `TrackLoop`).
const WHEEL: u32 = 1 << 25;
/// How far a track's back lies in front of its face (behind its relief), so that where TA put
/// another face in the same plane facing the other way, that face is the nearer from behind and
/// shows.
const BACK_LIFT: f32 = 0.02;
/// Faces facing the same way closer than this, one over the other, are layered (see `ceilings`).
const LAYER_GAP: f32 = 0.4;
/// How far a plate keeps under another piece's face over it (see `ceilings`).
const CLEARANCE: f32 = 0.03;
/// How steeply a plate's seams and a bolt head's sides rise (height over width): gently enough
/// that from behind a face they face away from the eye but within about 24 degrees of its plane
/// (see `plate`).
const SEAM_SLOPE: f32 = 0.45;

/// What the kit adds, by material: (sRGB colour, metallic, roughness). A plate's chamfer down to
/// its face's edge is its paint a little darker (it is lit as it slopes, catching the light on
/// one side and shading on the other, as a machined chamfer does); the groove between two
/// plates of a face darker still, its depth in shade. Neither is black: an outline in ink would
/// flatten the shapes into a cartoon.
const CHAMFER_SHADE: f32 = 0.66;
const GROOVE_SHADE: f32 = 0.42;
/// Bolt heads: steel, darker than bright paint, so they read as rivets and not as white specks.
const BOLT: ([u8; 3], f32, f32) = ([116, 116, 110], 0.7, 0.42);
const TIRE: ([u8; 3], f32, f32) = ([38, 37, 35], 0.04, 0.82);
const HUB: ([u8; 3], f32, f32) = ([128, 130, 127], 0.8, 0.38);
const BELT: ([u8; 3], f32, f32) = ([44, 43, 40], 0.1, 0.78);
const TRACK: ([u8; 3], f32, f32) = ([62, 60, 56], 0.45, 0.66);
/// A face's back, behind what the kit put in front of it.
const BACK: ([u8; 3], f32, f32) = ([24, 24, 22], 0.1, 0.9);

fn pos(v: &ModelVertex) -> Vec3 {
    Vec3::from_array(v.pos)
}

fn uv(v: &ModelVertex) -> Vec2 {
    Vec2::new(v.uv[0] as f32, v.uv[1] as f32) / 65535.0
}

fn packed_normal(n: Vec3, roughness: f32) -> [i8; 4] {
    let n = n.normalize_or(Vec3::Y);
    [(n.x * 127.0) as i8, (n.y * 127.0) as i8, (n.z * 127.0) as i8, (roughness.clamp(0.0, 1.0) * 127.0).round() as i8]
}

/// A plain (untextured) vertex of the kit's own material.
fn plain(template: &ModelVertex, p: Vec3, n: Vec3, (color, metallic, roughness): ([u8; 3], f32, f32)) -> ModelVertex {
    ModelVertex {
        pos: p.to_array(),
        normal: packed_normal(n, roughness),
        uv: [0, 0],
        layer: PLAIN,
        color: [color[0], color[1], color[2], (metallic * 255.0).round() as u8],
        piece: template.piece,
        edge_dist: [f16_bits(FAR_FROM_EDGES); 4],
        edge_weight: [0; 4],
        tread: [0.0; 2],
    }
}

impl Kit {
    fn start(&self) -> u32 {
        self.vertices.len() as u32
    }

    /// A triangle of the last vertices, wound to face `outward` (as the model's own faces are,
    /// see `ModelLibrary::add`: the cross product of its two edges points out).
    fn tri(&mut self, a: u32, b: u32, c: u32, outward: Vec3) {
        let (pa, pb, pc) = (pos(&self.vertices[a as usize]), pos(&self.vertices[b as usize]), pos(&self.vertices[c as usize]));
        if (pb - pa).cross(pc - pa).dot(outward) >= 0.0 {
            self.indices.extend_from_slice(&[a, b, c]);
        } else {
            self.indices.extend_from_slice(&[a, c, b]);
        }
    }

    /// A convex polygon of vertices already pushed, `first..first + n`, as a fan.
    fn fan(&mut self, first: u32, n: u32, outward: Vec3) {
        for k in 1..n.saturating_sub(1) {
            self.tri(first, first + k, first + k + 1, outward);
        }
    }

    /// A face as the model has it, its triangles wound exactly as the original's (TA's corner
    /// order, which odd polygons' normals can't be trusted to reproduce).
    fn keep(&mut self, face: &KitFace) {
        let first = self.start();
        self.vertices.extend_from_slice(face.vertices);
        for k in 1..face.vertices.len().saturating_sub(1) as u32 {
            self.indices.extend_from_slice(&[first, first + k + 1, first + k]);
        }
    }
}

/// The face again, facing the other way: TA's faces are seen from their front alone, so from
/// behind a track's side (through a pod's open inner side) its road wheels, standing out of it,
/// would show through. Its back hides them, a dark inside. It lies a hair in front of the face
/// (`BACK_LIFT`), so another face of the model in the same plane facing the other way stays in
/// front of it. (Plates need none: see `plate`.)
fn back(kit: &mut Kit, face: &KitFace, front: Vec3) {
    let first = kit.start();
    for v in face.vertices {
        kit.vertices.push(plain(v, pos(v) + front * BACK_LIFT, -front, BACK));
    }
    kit.fan(first, face.vertices.len() as u32, -front);
}

/// A face's corners, its centre and its area, if it is a convex polygon that is flat.
struct Outline {
    corners: Vec<Vec3>,
    centre: Vec3,
    area: f32,
}

/// A plain face (in a palette colour, or a picture of its own) folding shallowly into smaller
/// faces all round, twice the biggest of them at least: a flat panel let into a rounded part
/// (the Intimidator's turret's sides, between its rounded walls and roof), plated as any flat
/// face; not a facet of a barrel or a dome, each like its neighbours, which stay smooth.
fn panel_in_curve(i: usize, faces: &[KitFace], outlines: &[Option<Outline>]) -> bool {
    let Some(o) = &outlines[i] else { return false };
    let key = |p: Vec3| [(p.x * 64.0).round() as i32, (p.y * 64.0).round() as i32, (p.z * 64.0).round() as i32];
    let mine: Vec<[i32; 3]> = faces[i].vertices.iter().map(|v| key(pos(v))).collect();
    let mut neighbours = 0;
    for (j, f) in faces.iter().enumerate() {
        // (The faces it folds shallowly into, as the smoothing has it: not a cap under it, nor
        // a late face for one of TA's own.)
        if j == i || (f.late && !faces[i].late) || f.normal.dot(faces[i].normal) < super::smooth::SHALLOW || f.vertices.iter().filter(|v| mine.contains(&key(pos(v)))).count() < 2 {
            continue;
        }
        match &outlines[j] {
            Some(n) if o.area >= n.area * 2.0 => neighbours += 1,
            _ => return false,
        }
    }
    neighbours > 0
}

fn outline(face: &KitFace) -> Option<Outline> {
    convex(face.vertices.iter().map(pos).collect(), face.normal)
}

fn convex(corners: Vec<Vec3>, normal: Vec3) -> Option<Outline> {
    let n = corners.len();
    if n < 3 {
        return None;
    }
    let centre = corners.iter().copied().sum::<Vec3>() / n as f32;
    let mut area = 0.0;
    for k in 0..n {
        let (a, b) = (corners[k], corners[(k + 1) % n]);
        // Flat: every corner in the face's plane.
        if (a - centre).dot(normal).abs() > 0.05 {
            return None;
        }
        // Convex: every turn the same way.
        let c = corners[(k + 2) % n];
        if (b - a).cross(c - b).dot(normal) * (corners[1] - corners[0]).cross(corners[2] - corners[1]).dot(normal) < -1e-6 {
            return None;
        }
        area += (a - centre).cross(b - centre).length() * 0.5;
    }
    Some(Outline { corners, centre, area })
}

/// Whether `p` (in the outline's plane) lies on it, to a twentieth of a unit.
fn inside(o: &Outline, normal: Vec3, p: Vec3) -> bool {
    let n = o.corners.len();
    (0..n).all(|k| {
        let (a, b) = (o.corners[k], o.corners[(k + 1) % n]);
        let mut m = normal.cross(b - a).normalize_or_zero();
        if (o.centre - a).dot(m) < 0.0 {
            m = -m;
        }
        (p - a).dot(m) > -0.05
    })
}

/// Whether two faces in nearly the same plane overlap: the smaller's middle, or one of its
/// corners drawn a little in, lies on the bigger (faces that only meet at an edge don't).
fn overlapping(a: &Outline, b: &Outline, normal: Vec3) -> bool {
    let (big, small) = if b.area <= a.area { (a, b) } else { (b, a) };
    let flat = |p: Vec3| p - normal * (p - big.centre).dot(normal);
    std::iter::once(small.centre)
        .chain(small.corners.iter().map(|&c| c + (small.centre - c) * 0.15))
        .any(|p| inside(big, normal, flat(p)))
}

/// The face's outline with each edge moved inwards by its own margin (mitred corners), or None
/// where it would turn inside out or leave too little.
fn inset(outline: &Outline, normal: Vec3, margins: &[f32]) -> Option<Vec<Vec3>> {
    let c = &outline.corners;
    let n = c.len();
    let inward = |a: Vec3, b: Vec3| {
        let m = normal.cross(b - a).normalize_or_zero();
        if (outline.centre - a).dot(m) < 0.0 { -m } else { m }
    };
    // Each edge's line moved in, and where consecutive ones cross.
    let lines: Vec<(Vec3, Vec3)> = (0..n).map(|k| (c[k] + inward(c[k], c[(k + 1) % n]) * margins[k], (c[(k + 1) % n] - c[k]).normalize_or_zero())).collect();
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let ((p0, d0), (p1, d1)) = (lines[(k + n - 1) % n], lines[k]);
        // p0 + d0 s = p1 + d1 t, in the face's plane.
        let cross = d0.cross(d1).dot(normal);
        if cross.abs() < 0.05 {
            // (Nearly straight on: the corner moves with the edge after it.)
            out.push(c[k] + inward(c[k], c[(k + 1) % n]) * margins[k]);
            continue;
        }
        let s = (p1 - p0).cross(d1).dot(normal) / cross;
        out.push(p0 + d0 * s);
    }
    // Still the same way round, every edge still there.
    for k in 0..n {
        let (a, b) = (out[k], out[(k + 1) % n]);
        let (oa, ob) = (c[k], c[(k + 1) % n]);
        if (b - a).dot(ob - oa) <= 0.0 || (b - a).length() < 0.15 {
            return None;
        }
    }
    // Every new corner inside the face, at least each edge's margin in from it: a thin wedge's
    // corners would otherwise cross over and stick out.
    for &p in &out {
        for k in 0..n {
            if (p - c[k]).dot(inward(c[k], c[(k + 1) % n])) < margins[k] * 0.9 - 1e-4 {
                return None;
            }
        }
    }
    Some(out)
}

/// Where `p` falls in the face's paint (0..1).
fn paint_uv(face: &KitFace, p: Vec3) -> Vec2 {
    let [u, v] = paint_at(face, p);
    Vec2::new(u as f32, v as f32) / 65535.0
}

/// Where `p` falls in the face's paint: its texture coordinates, from the corners' through the
/// fan of triangles the face is drawn with.
fn paint_at(face: &KitFace, p: Vec3) -> [u16; 2] {
    let v = face.vertices;
    let mut best = (f32::MAX, uv(&v[0]));
    for k in 1..v.len().saturating_sub(1) {
        let (a, b, c) = (pos(&v[0]), pos(&v[k]), pos(&v[k + 1]));
        let (e0, e1, e2) = (b - a, c - a, p - a);
        let (d00, d01, d11, d20, d21) = (e0.dot(e0), e0.dot(e1), e1.dot(e1), e2.dot(e0), e2.dot(e1));
        let det = d00 * d11 - d01 * d01;
        if det.abs() < 1e-9 {
            continue;
        }
        let s = (d11 * d20 - d01 * d21) / det;
        let t = (d00 * d21 - d01 * d20) / det;
        let r = 1.0 - s - t;
        // How far outside this triangle (0 inside): the best fit wins.
        let outside = (-s).max(0.0) + (-t).max(0.0) + (-r).max(0.0);
        if outside < best.0 {
            best = (outside, uv(&v[0]) * r + uv(&v[k]) * s + uv(&v[k + 1]) * t);
        }
    }
    let at = best.1.clamp(Vec2::ZERO, Vec2::ONE) * 65535.0;
    [at.x.round() as u16, at.y.round() as u16]
}

/// A plate: the face raised on its outline moved in, a sloping seam all round down to where the
/// face was. The plate's own rim is its new edges, which the shader rounds and the edge wear
/// marks.
///
/// TA's faces are one-sided: from behind, TA shows what lies beyond one (into a factory's bay
/// with its doors open, through a fin), and so does the plate. Its top faces out as the face did
/// and its seams slope gently (rising less than half their width), so from behind every part of
/// it faces away but at angles within about 24 degrees of the face's plane.
///
/// It rises no higher than `ceiling` (another piece's face lying over this one, see `ceilings`).
/// Returns the plate's top outline and its height.
fn plate(kit: &mut Kit, face: &KitFace, outline: &Outline, ceiling: f32) -> Option<(Vec<Vec3>, f32)> {
    let normal = face.normal;
    // (A machined chamfer and the gap between plates: narrow, a fine line of light or shade
    // rather than a band.)
    let margin = (0.04 * outline.area.sqrt()).clamp(0.12, 0.3);
    // Where an edge is open the plate reaches it, and a plate with one lies flush: its seams are
    // drawn on the face rather than cut into it (raised, it would show past the open edge).
    let n = outline.corners.len();
    let open = |k: usize| face.open.get(k).copied().unwrap_or(true);
    let joined = |k: usize| face.joined.get(k).copied().unwrap_or(false);
    // (Between two plates each keeps back a little less, so the groove is about as wide as a
    // chamfer.)
    let margins: Vec<f32> = (0..n).map(|k| if open(k) { 0.0 } else if joined(k) { margin * 0.6 } else { margin }).collect();
    if margins.iter().all(|&m| m == 0.0) {
        return None;
    }
    let height = if (0..n).any(open) { 0.0 } else { (SEAM_SLOPE * margin).min(ceiling) };
    let top: Vec<Vec3> = inset(outline, normal, &margins)?.into_iter().map(|p| p + normal * height).collect();
    // The top: the face's own paint on the smaller outline.
    let first = kit.start();
    let template = &face.vertices[0];
    // Its edges for the shader (the four longest at most): distances in from each.
    let mut edges: Vec<(Vec3, Vec3, f32)> = (0..n)
        .map(|k| {
            let (a, b) = (top[k], top[(k + 1) % n]);
            let mut m = normal.cross(b - a).normalize_or_zero();
            let centre = top.iter().copied().sum::<Vec3>() / n as f32;
            if (centre - a).dot(m) < 0.0 {
                m = -m;
            }
            (a, m, (b - a).length())
        })
        .collect();
    edges.sort_by(|x, y| y.2.total_cmp(&x.2));
    edges.truncate(4);
    for (k, &p) in top.iter().enumerate() {
        let source = &face.vertices[k.min(face.vertices.len() - 1)];
        let mut vertex = *template;
        vertex.pos = p.to_array();
        vertex.normal = source.normal;
        vertex.uv = paint_at(face, p - normal * height);
        vertex.edge_dist = [f16_bits(FAR_FROM_EDGES); 4];
        vertex.edge_weight = [0; 4];
        for (j, &(a, m, _)) in edges.iter().enumerate() {
            vertex.edge_dist[j] = f16_bits((p - a).dot(m).max(0.0));
            vertex.edge_weight[j] = WEIGHT_RIM;
        }
        kit.vertices.push(vertex);
    }
    kit.fan(first, n as u32, normal);
    // The seams: from the face's outline up to the plate's, a shade of the paint (of the player's
    // colour, on a team colour's plate): a chamfer at the face's edge, a groove between plates.
    let shaded = |k: f32| {
        let shade = |c: u8| (c as f32 * k).round() as u8;
        ([shade(face.paint[0]), shade(face.paint[1]), shade(face.paint[2])], 0.15, 0.72)
    };
    let (chamfer, groove) = (shaded(CHAMFER_SHADE), shaded(GROOVE_SHADE));
    let base = &outline.corners;
    for k in 0..n {
        if margins[k] == 0.0 && height == 0.0 {
            continue;
        }
        let seam = if joined(k) { groove } else { chamfer };
        let (a, b, c, d) = (base[k], base[(k + 1) % n], top[(k + 1) % n], top[k]);
        let wall = if height == 0.0 {
            normal
        } else {
            let wall = (b - a).cross(d - a).normalize_or_zero();
            if wall.dot((a + b) * 0.5 - outline.centre) + wall.dot(normal) < 0.0 { -wall } else { wall }
        };
        let first = kit.start();
        for p in [a, b, c, d] {
            let mut v = plain(template, p, wall, seam);
            if face.team {
                v.layer |= TEAM_PAINT;
            }
            // (Shades of the face's paint: a livery paints them as it does the face, from its bank.)
            if template.layer & super::skin::LIVERY != 0 {
                v.layer |= template.layer & (super::skin::LIVERY | super::skin::ENEMY);
            }
            kit.vertices.push(v);
        }
        kit.tri(first, first + 1, first + 2, wall);
        kit.tri(first, first + 2, first + 3, wall);
    }
    Some((top, height))
}

/// Six-sided bolt heads near the plate's corners (big four-cornered plates only), each a low
/// hexagon with chamfered sides that, as a plate's seams, face away from an eye behind the face.
/// None where they would rise above `room`, over the plate's top.
fn bolts(kit: &mut Kit, face: &KitFace, top: &[Vec3], area: f32, room: f32, budget: &mut Budget) {
    if top.len() != 4 || area < BOLT_AREA || budget.bolts < 4 || face.late {
        return;
    }
    let shortest = (0..4).map(|k| top[k].distance(top[(k + 1) % 4])).fold(f32::MAX, f32::min);
    if shortest < 3.0 {
        return;
    }
    let normal = face.normal;
    let centre = top.iter().copied().sum::<Vec3>() / 4.0;
    let radius = (0.032 * area.sqrt()).clamp(0.11, 0.26);
    // A rivet's domed head: its top a third as wide as its foot, its sides rising as gently as
    // the seams (over the hexagon's flats, 0.866 of its radius).
    let crown = radius * 0.35;
    let height = (radius - crown) * 0.866 * SEAM_SLOPE;
    if height > room {
        return;
    }
    let across = (top[1] - top[0]).normalize_or(Vec3::X);
    let side = normal.cross(across).normalize_or(Vec3::Z);
    for &corner in top {
        let at = corner + (centre - corner).normalize_or_zero() * (0.22 * shortest).min(1.1);
        let ring = |r: f32, up: f32| -> Vec<Vec3> {
            (0..6)
                .map(|i| {
                    let a = std::f32::consts::TAU * (i as f32 + 0.5) / 6.0;
                    at + (across * a.cos() + side * a.sin()) * r + normal * up
                })
                .collect()
        };
        let (foot, head) = (ring(radius, 0.0), ring(crown, height));
        for i in 0..6 {
            let j = (i + 1) % 6;
            // (Its slope: out from the middle, and up by as much as the side rises.)
            let mid = ((foot[i] + foot[j]) * 0.5 - at).normalize_or(across);
            let out = (mid * height + normal * (radius - crown) * 0.866).normalize_or(normal);
            let first = kit.start();
            for p in [foot[i], foot[j], head[j], head[i]] {
                kit.vertices.push(plain(&face.vertices[0], p, out, BOLT));
            }
            kit.tri(first, first + 1, first + 2, out);
            kit.tri(first, first + 2, first + 3, out);
        }
        let first = kit.start();
        for &p in &head {
            kit.vertices.push(plain(&face.vertices[0], p, normal, BOLT));
        }
        kit.fan(first, 6, normal);
        kit.bolts += 1;
        budget.bolts -= 1;
    }
}

/// The sides of a road wheel's tyre and of its hub: round, not polygons, in the Archives' and
/// the wallpapers' close-ups (the hub is the brightest part of the running gear).
const TIRE_SIDES: usize = 20;
const HUB_SIDES: usize = 16;

/// A short cylinder: its middle on a face, its axis out of it.
struct Cylinder {
    centre: Vec3,
    axis: Vec3,
    radius: f32,
    length: f32,
    sides: usize,
}

/// A short cylinder standing out of a face (its sides and its outer end).
fn cylinder(kit: &mut Kit, template: &ModelVertex, shape: Cylinder, material: ([u8; 3], f32, f32)) {
    let Cylinder { centre, axis, radius, length, sides } = shape;
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let ring = |r: f32| -> Vec<Vec3> {
        (0..sides)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / sides as f32;
                (u * a.cos() + v * a.sin()) * r
            })
            .collect()
    };
    let spokes = ring(1.0);
    let first = kit.start();
    for s in &spokes {
        kit.vertices.push(plain(template, centre + *s * radius, *s, material));
        kit.vertices.push(plain(template, centre + *s * radius + axis * length, *s, material));
    }
    for i in 0..sides as u32 {
        let j = (i + 1) % sides as u32;
        let (a0, a1, b0, b1) = (first + i * 2, first + i * 2 + 1, first + j * 2, first + j * 2 + 1);
        let out = spokes[i as usize];
        kit.tri(a0, b0, b1, out);
        kit.tri(a0, b1, a1, out);
    }
    // The outer face, its edge a little darker with a smaller ring, like a rim.
    let first = kit.start();
    for s in &spokes {
        kit.vertices.push(plain(template, centre + *s * radius + axis * length, axis, material));
    }
    kit.fan(first, sides as u32, axis);
}

/// A track's side in relief, fitted inside its outline (the side's faces taken together: some
/// of TA's are two faces): the faces a dark belt, a row of road wheels and the track round them
/// standing a little out of it, as a bas-relief does, deep enough to catch the light and cast
/// shade, shallow enough that nothing sticks out past the model's outline. The band round the
/// wheels is as tall as the side allows, a little smaller where its ends slope in too far for
/// two wheels or more; on a short wedge of a pod even that won't go, and the side keeps TA's
/// paint (a tiny pair of wheels in a big pod would look like eyes). As the unit drives the band's
/// links run round the wheels with the ground under them and the wheels' hubs turn, as TA's
/// painted treads run (`side`: how far to the side the track is in the model, which sets how it
/// runs as the unit turns).
fn track(kit: &mut Kit, members: &[&KitFace], outline: &[Vec3], normal: Vec3, side: f32, budget: &mut Budget) -> bool {
    if budget.wheels < 2 || outline.len() < 3 {
        return false;
    }
    // The side's frame: along the track (front to back), up and out.
    let along = (Vec3::Z - normal * normal.z).normalize_or_zero();
    if along == Vec3::ZERO {
        return false;
    }
    let mut up = normal.cross(along).normalize_or(Vec3::Y);
    if up.y < 0.0 {
        up = -up;
    }
    let origin = outline[0];
    let flat: Vec<Vec2> = outline.iter().map(|&p| Vec2::new((p - origin).dot(along), (p - origin).dot(up))).collect();
    let (lo, hi) = flat.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    let (length, height) = (hi.x - lo.x, hi.y - lo.y);
    if height < 1.6 || length < height {
        return false;
    }
    // The outline's edges: a point on each and the direction into the side across it.
    let middle = flat.iter().copied().sum::<Vec2>() / flat.len() as f32;
    let edges: Vec<(Vec2, Vec2)> = (0..flat.len())
        .filter_map(|k| {
            let (a, b) = (flat[k], flat[(k + 1) % flat.len()]);
            let d = b - a;
            if d.length_squared() < 1e-8 {
                return None;
            }
            let mut m = Vec2::new(-d.y, d.x).normalize();
            if (middle - a).dot(m) < 0.0 {
                m = -m;
            }
            Some((a, m))
        })
        .collect();
    // Along the line at height `v`, where a disc of radius `r` can be centred and stay inside
    // the side: the first and last places, if there is room at all.
    let span = |v: f32, r: f32| -> Option<(f32, f32)> {
        let (mut first, mut last) = (f32::MIN, f32::MAX);
        for &(a, m) in &edges {
            // (p - a) . m >= r, for p = (s, v).
            let need = r - m.y * (v - a.y);
            if m.x.abs() < 1e-6 {
                if need > 1e-4 {
                    return None;
                }
            } else if m.x > 0.0 {
                first = first.max(a.x + need / m.x);
            } else {
                last = last.min(a.x + need / m.x);
            }
        }
        (last >= first).then_some((first, last))
    };
    // The band's outer radius, from the most the side's height allows down to three quarters of
    // that, and its height on the side (the middle unless another leaves room for more wheels):
    // the first with three wheels or more, else the first with two.
    const MARGIN: f32 = 0.06;
    let most = (height * 0.5 - MARGIN).min(height * 0.44);
    let wheels = |radius: f32, (first, last): (f32, f32)| ((last - first) / (radius * 2.25)).floor() as usize + 1;
    let (mut fit, mut fallback) = (None, None);
    for step in 0..=4 {
        let outer = most * (1.0 - 0.0625 * step as f32);
        let thickness = (outer * 0.17).max(0.18);
        let inner = outer - thickness;
        let radius = inner - 0.05;
        if radius < 0.4 {
            break;
        }
        let (v0, v1) = (lo.y + outer + MARGIN, hi.y - outer - MARGIN);
        if v1 < v0 {
            continue;
        }
        let centre = ((lo.y + hi.y) * 0.5).clamp(v0, v1);
        let mut best = span(centre, outer + MARGIN).map(|s| (centre, s));
        for i in 0..=8 {
            let v = v0 + (v1 - v0) * i as f32 / 8.0;
            if let Some(s) = span(v, outer + MARGIN)
                && best.is_none_or(|(_, b)| wheels(radius, s) > wheels(radius, b))
            {
                best = Some((v, s));
            }
        }
        let Some((v, s)) = best else { continue };
        let count = wheels(radius, s);
        if count >= 3 {
            fit = Some((outer, inner, radius, v, s));
            break;
        }
        if count >= 2 && fallback.is_none() {
            fallback = Some((outer, inner, radius, v, s));
        }
    }
    let Some((outer, inner, radius, centre_u, (first, last))) = fit.or(fallback) else { return false };
    let tire = (radius * 0.16).clamp(0.12, 0.6);
    let hub = tire + (radius * 0.1).clamp(0.06, 0.4);
    let count = wheels(radius, (first, last)).clamp(2, 12).min(budget.wheels);
    let spacing = (last - first) / (count - 1) as f32;
    // The belt: the faces themselves, a dark rubber (wound as the model has them).
    for face in members {
        let start = kit.start();
        for v in face.vertices {
            kit.vertices.push(plain(v, pos(v), face.normal, BELT));
        }
        for k in 1..face.vertices.len().saturating_sub(1) as u32 {
            kit.indices.extend_from_slice(&[start, start + k + 1, start + k]);
        }
    }
    let template = &members[0].vertices[0];
    for i in 0..count {
        let at = origin + along * (first + spacing * i as f32) + up * centre_u;
        cylinder(kit, template, Cylinder { centre: at, axis: normal, radius, length: tire, sides: TIRE_SIDES }, TIRE);
        let hub_radius = radius * 0.55;
        cylinder(kit, template, Cylinder { centre: at, axis: normal, radius: hub_radius, length: hub, sides: HUB_SIDES }, HUB);
        // The hub's face, across it (along the track and up), turning a radian for each radius
        // of track the wheel's rim rolls over.
        let face = at + normal * hub;
        let start = kit.vertices.len() - HUB_SIDES;
        for v in &mut kit.vertices[start..] {
            let d = (pos(v) - face) / hub_radius;
            v.uv = [unorm16(d.dot(along) * 0.5 + 0.5), unorm16(d.dot(up) * 0.5 + 0.5)];
            v.tread = [1.0 / radius, side];
            v.layer |= WHEEL;
        }
        kit.wheels += 1;
        budget.wheels -= 1;
    }
    // The track round them: over the wheels, under them and round both ends.
    let band = TrackLoop { origin, along, up, normal, ends: (first, last), centre_u, inner, outer, depth: tire + 0.03, side };
    band.build(kit, template);
    true
}

/// A texture coordinate 0..1 as the vertex has it.
fn unorm16(x: f32) -> u16 {
    (x.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// A track seen from the side: a band round the row of wheels (two half circles joined over
/// and under them), as wide as the wheels stand out of the face.
struct TrackLoop {
    /// The face's frame: a corner, its long axis, its upward axis and outwards.
    origin: Vec3,
    along: Vec3,
    up: Vec3,
    normal: Vec3,
    /// The first and last wheels' centres along the face, and their height on it.
    ends: (f32, f32),
    centre_u: f32,
    /// The band's inner and outer radius round the wheels' centres, and how far out it stands.
    inner: f32,
    outer: f32,
    depth: f32,
    /// How far to the side the track is in the model (see `track`).
    side: f32,
}

impl TrackLoop {
    fn build(&self, kit: &mut Kit, template: &ModelVertex) {
        const ARC: usize = 8;
        // Round the far end from below to above, then the near end from above to below.
        let ring: Vec<(Vec2, f32)> = (0..=ARC)
            .map(|i| (std::f32::consts::PI * (i as f32 / ARC as f32 - 0.5), self.ends.1))
            .chain((0..=ARC).map(|i| (std::f32::consts::PI * (i as f32 / ARC as f32 + 0.5), self.ends.0)))
            .map(|(angle, centre)| (Vec2::new(angle.cos(), angle.sin()), centre))
            .collect();
        let n = ring.len();
        let at = |k: usize, r: f32, out: f32| {
            let (dir, centre) = ring[k];
            self.origin + self.along * (centre + dir.x * r) + self.up * (self.centre_u + dir.y * r) + self.normal * out
        };
        let radial = |k: usize| (self.along * ring[k].0.x + self.up * ring[k].0.y).normalize_or(self.up);
        // How far round the band each point is (along its middle, from the front's foot: over
        // the front, back along the top, round the back and forward along the bottom). The
        // links run back along it as the unit drives, the bottom's staying on the ground.
        let middle = (self.inner + self.outer) * 0.5;
        let mut round = vec![0.0f32; n + 1];
        for k in 0..n {
            round[k + 1] = round[k] + at(k, middle, 0.0).distance(at((k + 1) % n, middle, 0.0));
        }
        let length = round[n].max(1e-3);
        // About a link a band's width and a half long, a whole number of them round it.
        let links = (length / ((self.outer - self.inner) * 3.2).max(0.6)).round().clamp(8.0, 255.0);
        let running = |v: &mut ModelVertex, along_band: f32| {
            v.uv = [unorm16(along_band / length), unorm16(links / 255.0)];
            v.tread = [1.0 / length, self.side];
        };
        for k in 0..n {
            let j = (k + 1) % n;
            let (uk, uj) = (round[k], round[k + 1]);
            // The band's outside, its inside (towards the wheels), and its flat side.
            for (r, sign) in [(self.outer, 1.0f32), (self.inner, -1.0)] {
                let first = kit.start();
                for (p, n, u) in [(at(k, r, 0.0), radial(k), uk), (at(j, r, 0.0), radial(j), uj), (at(j, r, self.depth), radial(j), uj), (at(k, r, self.depth), radial(k), uk)] {
                    let mut v = plain(template, p, n * sign, TRACK);
                    running(&mut v, u);
                    kit.vertices.push(v);
                }
                let out = (radial(k) + radial(j)) * sign;
                kit.tri(first, first + 1, first + 2, out);
                kit.tri(first, first + 2, first + 3, out);
            }
            let first = kit.start();
            for (p, u) in [(at(k, self.inner, self.depth), uk), (at(j, self.inner, self.depth), uj), (at(j, self.outer, self.depth), uj), (at(k, self.outer, self.depth), uk)] {
                let mut v = plain(template, p, self.normal, TRACK);
                running(&mut v, u);
                kit.vertices.push(v);
            }
            kit.tri(first, first + 1, first + 2, self.normal);
            kit.tri(first, first + 2, first + 3, self.normal);
        }
    }
}

/// The outer sides of the piece's tracks, each with its outline: a side's faces in one plane
/// that meet at an edge are taken together where they make one convex shape (ARMMART's sides are
/// two faces each). Only the sides facing out from the model's middle, left or right: the inner
/// sides face its hull, and a face painted with tracks that faces another way (a Kbot's, a
/// submarine's tail) is no track's side.
fn track_sides(faces: &[KitFace], outlines: &[Option<Outline>], around: &Around) -> Vec<(Vec<usize>, Vec<Vec3>)> {
    let middle = -around.offset;
    let sides: Vec<usize> = (0..faces.len())
        .filter(|&i| {
            let n = faces[i].normal;
            faces[i].kind == FaceKind::TrackSide && !faces[i].late && n.x.abs() >= 0.9 && outlines[i].as_ref().is_some_and(|o| n.x * (o.centre.x - middle.x) > 0.0)
        })
        .collect();
    // Faces of one side: in one plane, sharing two corners.
    let mut group: Vec<usize> = (0..sides.len()).collect();
    let root = |group: &Vec<usize>, mut k: usize| {
        while group[k] != k {
            k = group[k];
        }
        k
    };
    for a in 0..sides.len() {
        for b in a + 1..sides.len() {
            let (fa, fb) = (&faces[sides[a]], &faces[sides[b]]);
            let (Some(oa), Some(ob)) = (&outlines[sides[a]], &outlines[sides[b]]) else { continue };
            if fa.normal.dot(fb.normal) < 0.999 || (ob.centre - oa.centre).dot(fa.normal).abs() > 0.05 {
                continue;
            }
            let shared = oa.corners.iter().filter(|p| ob.corners.iter().any(|q| p.distance(*q) < 0.01)).count();
            if shared >= 2 {
                let (ra, rb) = (root(&group, a), root(&group, b));
                group[rb] = ra;
            }
        }
    }
    let mut out = Vec::new();
    for k in 0..sides.len() {
        if root(&group, k) != k {
            continue;
        }
        let members: Vec<usize> = (0..sides.len()).filter(|&j| root(&group, j) == k).map(|j| sides[j]).collect();
        if members.len() > 1
            && let Some(hull) = merged(faces[members[0]].normal, members.iter().filter_map(|&i| outlines[i].as_ref()))
        {
            out.push((members, hull));
            continue;
        }
        for i in members {
            if let Some(o) = &outlines[i] {
                out.push((vec![i], o.corners.clone()));
            }
        }
    }
    out
}

/// The outline of faces in one plane taken together, if they make one convex shape (their hull
/// no bigger than their areas added up).
fn merged<'a>(normal: Vec3, outlines: impl Iterator<Item = &'a Outline>) -> Option<Vec<Vec3>> {
    let outlines: Vec<&Outline> = outlines.collect();
    let origin = outlines.first()?.corners[0];
    let u = normal.any_orthonormal_vector();
    let v = normal.cross(u);
    let mut points: Vec<Vec2> = outlines.iter().flat_map(|o| o.corners.iter()).map(|&p| Vec2::new((p - origin).dot(u), (p - origin).dot(v))).collect();
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup_by(|a, b| a.distance(*b) < 0.01);
    // Andrew's monotone chain: the lower hull left to right, then the upper right to left.
    let turn = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<Vec2> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        for k in 0..points.len() {
            let p = if pass == 0 { points[k] } else { points[points.len() - 1 - k] };
            while hull.len() >= start + 2 && turn(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 1e-6 {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    if hull.len() < 3 {
        return None;
    }
    let area = (0..hull.len()).map(|k| hull[k].perp_dot(hull[(k + 1) % hull.len()])).sum::<f32>().abs() * 0.5;
    let sum: f32 = outlines.iter().map(|o| o.area).sum();
    ((area - sum).abs() <= sum * 0.02).then(|| hull.iter().map(|p| origin + u * p.x + v * p.y).collect())
}

/// How high a plate on each face may rise (0: it stays as it is; infinite: as high as plates go).
///
/// Faces painted over another in the same piece (a decal a little above a bigger face in the same
/// plane), and the faces under them, stay as they are: plates would bury the paint on top. A
/// face of another piece lying over a face at rest (facing the same way, overlapping it) is a
/// ceiling to it: lying on it or nearly (a wing's root on the hull, a landing pad on a runway
/// lifted clear of the ground), the face stays as it is; a little higher (the nested segments
/// of a telescoping arm), its plate rises no higher than a little under it.
fn ceilings(faces: &[KitFace], outlines: &[Option<Outline>], around: &Around) -> Vec<f32> {
    let mut out = vec![f32::INFINITY; faces.len()];
    for i in 0..faces.len() {
        let Some(a) = &outlines[i] else { continue };
        let normal = faces[i].normal;
        for j in i + 1..faces.len() {
            let Some(b) = &outlines[j] else { continue };
            if normal.dot(faces[j].normal) < 0.99 || (b.centre - a.centre).dot(normal).abs() > LAYER_GAP {
                continue;
            }
            if overlapping(a, b, normal) {
                // (A late face lays nothing on TA's own.)
                if !(faces[j].late && !faces[i].late) {
                    out[i] = 0.0;
                }
                if !(faces[i].late && !faces[j].late) {
                    out[j] = 0.0;
                }
            }
        }
        if out[i] == 0.0 {
            continue;
        }
        // The other pieces' faces over it, in the model's space.
        let centre = a.centre + around.offset;
        let reach = a.corners.iter().map(|c| c.distance(a.centre)).fold(0.0, f32::max);
        let moved = Outline { corners: a.corners.iter().map(|&c| c + around.offset).collect(), centre, area: a.area };
        for other in around.faces.iter().filter(|f| f.piece != around.piece) {
            let o = &other.outline;
            let above = (o.centre - centre).dot(normal);
            if normal.dot(other.normal) < 0.99 || !(-0.01..=LAYER_GAP).contains(&above) || o.centre.distance(centre) > reach + other.reach {
                continue;
            }
            if overlapping(&moved, o, normal) {
                out[i] = out[i].min(if above < CLEARANCE * 3.0 { 0.0 } else { above - CLEARANCE });
            }
        }
    }
    out
}

/// A big four-sided face as a grid of smaller ones (up to 3 x 3, about `PLATE_SPAN` each), so it
/// is laid with several plates as armour is: each part's corners where they fall on the face,
/// its paint where TA put it there, and which of its edges are the face's open ones and which
/// meet the next part (the plates either side meet there, a groove between them). A face is cut
/// only across sides that run parallel, so the cuts do too: a trapezoid along its parallel
/// sides, a kite or an odd quadrilateral not at all (cuts fanning across it would read as cracks
/// in it). None for a face small enough for one plate, or not four-sided.
#[allow(clippy::type_complexity)]
fn parts(face: &KitFace, outline: &Outline) -> Option<Vec<(Vec<ModelVertex>, Vec<bool>, Vec<bool>)>> {
    let c = &outline.corners;
    if c.len() != 4 {
        return None;
    }
    // (Within about 6 degrees.)
    let parallel = |a: Vec3, b: Vec3| a.normalize_or_zero().dot(b.normalize_or_zero()) > 0.995;
    let (across, down) = ((c[1].distance(c[0]) + c[2].distance(c[3])) * 0.5, (c[3].distance(c[0]) + c[2].distance(c[1])) * 0.5);
    // Cuts across the first edge (from it to the third) run along the sides from it: they only
    // run parallel where those sides do.
    let nu = if parallel(c[3] - c[0], c[2] - c[1]) { ((across / PLATE_SPAN).round() as usize).clamp(1, 3) } else { 1 };
    let nv = if parallel(c[1] - c[0], c[2] - c[3]) { ((down / PLATE_SPAN).round() as usize).clamp(1, 3) } else { 1 };
    if nu * nv == 1 {
        return None;
    }
    let at = |s: f32, t: f32| c[0] * ((1.0 - s) * (1.0 - t)) + c[1] * (s * (1.0 - t)) + c[2] * (s * t) + c[3] * ((1.0 - s) * t);
    // Where the cuts run: each near its share of the face, moved to where the paint is calmest
    // along it (along a painted panel line or a seam of TA's own, between its lamps and emblems),
    // and left out where it would cross what is painted wherever it went.
    let cuts = |n: usize, line: &dyn Fn(f32) -> (Vec3, Vec3)| -> Vec<f32> {
        let mut bounds = vec![0.0];
        for k in 1..n {
            let nominal = k as f32 / n as f32;
            let Some(map) = face.painted else {
                bounds.push(nominal);
                continue;
            };
            let cost = |s: f32| {
                let (a, b) = line(s);
                map.busy(paint_uv(face, a), paint_uv(face, b)) + CUT_DRIFT * (s - nominal).abs() * n as f32
            };
            let (best, at_best) = (0..=12)
                .map(|i| nominal + (i as f32 / 12.0 - 0.5) * 0.6 / n as f32)
                .map(|s| (cost(s), s))
                .fold((f32::MAX, nominal), |b, c| if c.0 < b.0 { c } else { b });
            if best <= CALM {
                bounds.push(at_best);
            }
        }
        bounds.push(1.0);
        bounds
    };
    let us = cuts(nu, &|s| (at(s, 0.04), at(s, 0.96)));
    let vs = cuts(nv, &|t| (at(0.04, t), at(0.96, t)));
    let (nu, nv) = (us.len() - 1, vs.len() - 1);
    if nu * nv == 1 {
        return None;
    }
    let open = |k: usize| face.open.get(k).copied().unwrap_or(true);
    let mut out = Vec::with_capacity(nu * nv);
    for j in 0..nv {
        for i in 0..nu {
            let (s0, s1, t0, t1) = (us[i], us[i + 1], vs[j], vs[j + 1]);
            let corners = [at(s0, t0), at(s1, t0), at(s1, t1), at(s0, t1)];
            let vertices = corners
                .iter()
                .map(|&p| {
                    let mut v = face.vertices[0];
                    v.pos = p.to_array();
                    v.uv = paint_at(face, p);
                    v
                })
                .collect();
            // Its edges in the face's order: the first runs along t0 (the face's first edge when
            // j = 0), the second along s1 (its second when i is the last), and so on.
            let edges = vec![j == 0 && open(0), i + 1 == nu && open(1), j + 1 == nv && open(2), i == 0 && open(3)];
            let joined = vec![j > 0, i + 1 < nu, j + 1 < nv, i > 0];
            out.push((vertices, edges, joined));
        }
    }
    Some(out)
}

/// A piece's surface with the kit fitted: every face, as it was or made a plate, and what was
/// added to them.
pub(super) fn build(faces: &[KitFace], around: &Around, budget: &mut Budget) -> Kit {
    let mut kit = Kit::default();
    let outlines: Vec<Option<Outline>> = faces.iter().map(outline).collect();
    let ceilings = if faces.len() <= 2048 { ceilings(faces, &outlines, around) } else { vec![0.0; faces.len()] };
    // The tracks' outer sides first (a side can be several faces).
    let mut done = vec![false; faces.len()];
    for (members, hull) in track_sides(faces, &outlines, around) {
        let normal = faces[members[0]].normal;
        let on: Vec<&KitFace> = members.iter().map(|&i| &faces[i]).collect();
        let side = around.offset.x + hull.iter().map(|p| p.x).sum::<f32>() / hull.len().max(1) as f32;
        if track(&mut kit, &on, &hull, normal, side, budget) {
            for &i in &members {
                back(&mut kit, &faces[i], normal);
                done[i] = true;
            }
        }
    }
    for (i, face) in faces.iter().enumerate() {
        if done[i] {
            continue;
        }
        let Some(o) = &outlines[i] else {
            kit.keep(face);
            continue;
        };
        // (Smoothed corners: part of a curved surface, but for a face whose smoothed corners
        // are the two ends of one edge alone, where it folds shallowly into its neighbour: the
        // Millennium's band in the player's colour, along its hull's side.)
        let bent: Vec<bool> = face
            .vertices
            .iter()
            .map(|v| {
                let n = Vec3::new(v.normal[0] as f32, v.normal[1] as f32, v.normal[2] as f32) / 127.0;
                n.normalize_or_zero().dot(face.normal) <= FLAT
            })
            .collect();
        let corners = bent.len();
        let flat = match bent.iter().filter(|&&b| b).count() {
            0 => true,
            2 => corners >= 4 && (0..corners).any(|k| bent[k] && bent[(k + 1) % corners]),
            _ => face.kind == FaceKind::Picture && panel_in_curve(i, faces, &outlines),
        };
        let ceiling = ceilings[i];
        let laid = match face.kind {
            FaceKind::Plating | FaceKind::Picture if flat && ceiling > 0.0 && o.area >= PLATE_AREA => {
                // A big face: a grid of plates (any part no plate fits stays as it was); a
                // picture, one plate, unbolted.
                let bolted = face.kind == FaceKind::Plating;
                match if bolted { parts(face, o) } else { None } {
                    Some(parts) => {
                        for (vertices, open, joined) in &parts {
                            let part = KitFace { vertices, open, joined, ..*face };
                            let Some(po) = outline(&part) else {
                                kit.keep(&part);
                                continue;
                            };
                            match plate(&mut kit, &part, &po, ceiling) {
                                Some((top, height)) => {
                                    kit.plates += 1;
                                    bolts(&mut kit, &part, &top, po.area, ceiling - height, budget);
                                }
                                None => kit.keep(&part),
                            }
                        }
                        true
                    }
                    None => match plate(&mut kit, face, o, ceiling) {
                        Some((top, height)) => {
                            kit.plates += 1;
                            if bolted {
                                bolts(&mut kit, face, &top, o.area, ceiling - height, budget);
                            }
                            true
                        }
                        None => false,
                    },
                }
            }
            _ => false,
        };
        if !laid {
            kit.keep(face);
        }
    }
    kit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(p: Vec3, n: Vec3, uv: [f32; 2]) -> ModelVertex {
        ModelVertex {
            pos: p.to_array(),
            normal: packed_normal(n, 0.5),
            uv: [(uv[0] * 65535.0) as u16, (uv[1] * 65535.0) as u16],
            layer: 3,
            color: [255; 4],
            piece: 2,
            edge_dist: [f16_bits(FAR_FROM_EDGES); 4],
            edge_weight: [0; 4],
            tread: [0.0; 2],
        }
    }

    fn face<'a>(vertices: &'a [ModelVertex], normal: Vec3, kind: FaceKind, open: &'a [bool]) -> KitFace<'a> {
        KitFace { vertices, normal, kind, paint: [200, 180, 120], team: false, open, joined: &[], painted: None, late: false }
    }

    /// A square face in the plane y = 0 facing up, `size` across, wound as the model's are.
    fn square(size: f32) -> Vec<ModelVertex> {
        let s = size / 2.0;
        // (Wound so that the fan's first triangle's edges cross to point down, as TA's do
        // once z is flipped; `Kit::tri` puts it right.)
        [(-s, -s, [0.0, 0.0]), (-s, s, [1.0, 0.0]), (s, s, [1.0, 1.0]), (s, -s, [0.0, 1.0])]
            .iter()
            .map(|&(x, z, uv)| vertex(Vec3::new(x, 0.0, z), Vec3::Y, uv))
            .collect()
    }

    /// Whether every triangle of the kit faces away from an eye behind the face (the plane y = 0,
    /// facing up) looking at it from at least `angle` degrees off its plane, as a one-sided face
    /// of TA's does: TA shows what lies beyond it from there.
    fn unseen_from_below(kit: &Kit, angle: f32) -> bool {
        let tilt = angle.to_radians();
        (0..16).all(|k| {
            let turn = std::f32::consts::TAU * k as f32 / 16.0;
            // From the eye towards the face: up, and across it.
            let look = Vec3::new(tilt.cos() * turn.cos(), tilt.sin(), tilt.cos() * turn.sin());
            kit.indices.chunks(3).all(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| pos(&kit.vertices[i as usize]));
                (b - a).cross(c - a).dot(look) >= -1e-6
            })
        })
    }

    #[test]
    fn a_big_face_becomes_a_raised_plate_with_a_seam_and_bolts() {
        let quad = square(8.0);
        let mut budget = Budget::default();
        let kit = build(&[face(&quad, Vec3::Y, FaceKind::Plating, &[false; 4])], &Around::default(), &mut budget);
        assert_eq!(kit.plates, 1);
        assert_eq!(kit.bolts, 4);
        // The top stands above where the face was, within its outline; the paint goes with it.
        let top: Vec<&ModelVertex> = kit.vertices.iter().filter(|v| v.layer == 3).collect();
        assert_eq!(top.len(), 4);
        for v in &top {
            assert!(v.pos[1] > 0.05 && v.pos[0].abs() < 4.0 && v.pos[2].abs() < 4.0, "{:?}", v.pos);
            assert!(v.edge_weight.iter().all(|&w| w == WEIGHT_RIM));
            assert_eq!(v.piece, 2);
        }
        // The seams (the plate's chamfer down to the face's edge) are a shade of the paint.
        let chamfer = (200.0 * CHAMFER_SHADE).round() as u8;
        assert!(kit.vertices.iter().any(|v| v.layer == PLAIN && v.color[0] == chamfer));
        // From behind, as TA's face, it can't be seen (but within 25 degrees of its plane): what
        // lies beyond shows, as in TA (a factory's bay through its open doors).
        assert!(unseen_from_below(&kit, 25.0));
        assert!(!unseen_from_below(&kit, 10.0), "(a raised plate's seams do show, grazing)");
        assert!(kit.vertices.iter().all(|v| v.color[..3] != BACK.0), "a dark back over what TA showed");
        assert!(kit.indices.iter().all(|&i| (i as usize) < kit.vertices.len()));
    }

    #[test]
    fn a_plate_with_an_open_edge_lies_flush_and_reaches_it() {
        let quad = square(8.0);
        let open = [false, true, false, false];
        let kit = build(&[face(&quad, Vec3::Y, FaceKind::Plating, &open)], &Around::default(), &mut Budget { bolts: 0, wheels: 0 });
        assert_eq!(kit.plates, 1);
        // Flush: nothing above or below the face's plane.
        assert!(kit.vertices.iter().all(|v| v.pos[1].abs() < 1e-4), "raised past an open edge");
        assert!(unseen_from_below(&kit, 1.0));
        // The open edge (from the second corner to the third, along z = 4) is the plate's own.
        let top: Vec<&ModelVertex> = kit.vertices.iter().filter(|v| v.layer == 3).collect();
        assert_eq!(top.iter().filter(|v| (v.pos[2] - 4.0).abs() < 1e-3).count(), 2, "{:?}", top.iter().map(|v| v.pos).collect::<Vec<_>>());
    }

    #[test]
    fn a_big_face_is_laid_with_a_grid_of_plates_and_a_picture_with_one() {
        let quad = square(36.0);
        let kit = build(&[face(&quad, Vec3::Y, FaceKind::Plating, &[false; 4])], &Around::default(), &mut Budget::default());
        assert_eq!(kit.plates, 9);
        assert!(kit.vertices.iter().all(|v| v.pos[0].abs() <= 18.0 + 1e-3 && v.pos[2].abs() <= 18.0 + 1e-3));
        // Each plate's paint is where it lies on the face: the paint runs on across them.
        let tops: Vec<&ModelVertex> = kit.vertices.iter().filter(|v| v.layer == 3).collect();
        for v in &tops {
            let (x, z) = (v.pos[0], v.pos[2]);
            let expect = paint_at(&face(&quad, Vec3::Y, FaceKind::Plating, &[false; 4]), Vec3::new(x, 0.0, z));
            assert!((v.uv[0] as i32 - expect[0] as i32).abs() <= 2 && (v.uv[1] as i32 - expect[1] as i32).abs() <= 2);
        }
        // A picture (an emblem) is one plate, without bolts to cover it.
        let kit = build(&[face(&quad, Vec3::Y, FaceKind::Picture, &[false; 4])], &Around::default(), &mut Budget::default());
        assert_eq!((kit.plates, kit.bolts), (1, 0));
    }

    #[test]
    fn small_curved_and_layered_faces_stay_as_they_are() {
        let small = square(1.5);
        let mut curved = square(8.0);
        curved[0].normal = packed_normal(Vec3::new(0.4, 1.0, 0.0), 0.5);
        let under = square(10.0);
        let mut over = square(3.0);
        for v in &mut over {
            v.pos[1] = 0.02;
        }
        let faces = [&small, &curved, &under, &over].map(|q| face(q, Vec3::Y, FaceKind::Plating, &[false; 4]));
        let kit = build(&faces, &Around::default(), &mut Budget::default());
        assert_eq!(kit.plates, 0);
        assert_eq!(kit.vertices.len(), 16);
    }

    #[test]
    fn plates_keep_under_another_pieces_face_over_them() {
        // A runway in the base, and a landing pad of another piece just over part of it (as the
        // Aircraft Plant's, its runway lifted clear of the ground): a plate on the runway would
        // come up through the pad, so it stays as it is.
        let runway = square(20.0);
        let pad = |y: f32| -> Vec<Vec3> { [(-4.0, -4.0), (-4.0, 4.0), (4.0, 4.0), (4.0, -4.0)].iter().map(|&(x, z)| Vec3::new(x + 6.0, y, z)).collect() };
        let neighbours = [Neighbour::new(1, pad(0.06), Vec3::Y).unwrap()];
        let around = Around { piece: 0, offset: Vec3::ZERO, faces: &neighbours };
        let kit = build(&[face(&runway, Vec3::Y, FaceKind::Plating, &[false; 4])], &around, &mut Budget::default());
        assert_eq!(kit.plates, 0);
        // A little higher (a telescoping arm's segment over the next), plates keep under it,
        // unbolted.
        let neighbours = [Neighbour::new(1, pad(0.16), Vec3::Y).unwrap()];
        let around = Around { piece: 0, offset: Vec3::ZERO, faces: &neighbours };
        let kit = build(&[face(&runway, Vec3::Y, FaceKind::Plating, &[false; 4])], &around, &mut Budget::default());
        assert!(kit.plates > 0 && kit.bolts == 0);
        assert!(kit.vertices.iter().all(|v| v.pos[1] <= 0.16 - CLEARANCE + 1e-4), "a plate through the face over it");
        // A face under it is no ceiling, and the piece's own faces count only once (a model's
        // faces include the piece's own).
        let neighbours = [Neighbour::new(1, pad(-0.1), Vec3::Y).unwrap(), Neighbour::new(0, runway.iter().map(pos).collect(), Vec3::Y).unwrap()];
        let around = Around { piece: 0, offset: Vec3::ZERO, faces: &neighbours };
        let kit = build(&[face(&runway, Vec3::Y, FaceKind::Plating, &[false; 4])], &around, &mut Budget::default());
        assert!(kit.plates > 0 && kit.bolts > 0);
    }

    #[test]
    fn a_team_colours_seams_take_the_players_colour() {
        let quad = square(8.0);
        let team = KitFace { team: true, paint: [180; 3], ..face(&quad, Vec3::Y, FaceKind::Plating, &[false; 4]) };
        let kit = build(&[team], &Around::default(), &mut Budget::default());
        let chamfer = (180.0 * CHAMFER_SHADE).round() as u8;
        let seams: Vec<&ModelVertex> = kit.vertices.iter().filter(|v| v.layer & 0xFFFF == 0xFFFF && v.color[0] == chamfer).collect();
        assert!(!seams.is_empty() && seams.iter().all(|v| v.layer & TEAM_PAINT != 0 && v.color[0] == v.color[2]));
        // (Its bolts are steel, whatever the player's colour.)
        let bolts: Vec<&ModelVertex> = kit.vertices.iter().filter(|v| v.color[..3] == BOLT.0).collect();
        assert!(!bolts.is_empty() && bolts.iter().all(|v| v.layer & TEAM_PAINT == 0));
    }

    /// A track's side facing -x at x = -12.5 (the left of a unit facing +z), from its corners
    /// given as (z, y).
    fn side(corners: &[(f32, f32)]) -> Vec<ModelVertex> {
        corners.iter().map(|&(z, y)| vertex(Vec3::new(-12.5, y, z), -Vec3::X, [0.0, 0.0])).collect()
    }

    /// Every vertex of the relief within the side's outline (seen from the side), and none
    /// into the hull.
    fn within(kit: &Kit, outline: &[(f32, f32)]) -> Result<(), String> {
        let n = outline.len();
        let centre = outline.iter().fold(Vec2::ZERO, |s, &(z, y)| s + Vec2::new(z, y)) / n as f32;
        for v in &kit.vertices {
            if v.pos[0] > -12.5 + 1e-3 || v.pos[0] < -13.6 {
                return Err(format!("out of the side's relief: {:?}", v.pos));
            }
            let p = Vec2::new(v.pos[2], v.pos[1]);
            for k in 0..n {
                let (a, b) = (Vec2::from(outline[k]), Vec2::from(outline[(k + 1) % n]));
                let d = b - a;
                let mut m = Vec2::new(-d.y, d.x).normalize();
                if (centre - a).dot(m) < 0.0 {
                    m = -m;
                }
                if (p - a).dot(m) < -1e-3 {
                    return Err(format!("{:?} past the edge from {a} to {b}", v.pos));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn a_tracks_side_gets_a_row_of_wheels() {
        // A side 23.5 long and 4 high, facing -x, as Stumpy's left track.
        let outline = [(-11.0, 0.5), (-11.0, 4.5), (12.5, 4.5), (12.5, 0.5)];
        let quad = side(&outline);
        let around = Around { offset: Vec3::ZERO, ..Default::default() };
        let kit = build(&[face(&quad, -Vec3::X, FaceKind::TrackSide, &[false; 4])], &around, &mut Budget::default());
        assert_eq!(kit.wheels, 7);
        // A relief: a little out of the side (under a unit here), never into the hull.
        within(&kit, &outline).unwrap();
        assert!(kit.vertices.iter().any(|v| v.pos[0] < -12.6));
    }

    /// As the unit drives the band's links run round it and the hubs turn, with the ground: the
    /// band's u runs once round it, a whole number of links to it, at a unit of the band a unit
    /// driven; a hub turns a radian for each radius of its wheel driven. The side's place sets
    /// how they run as the unit turns.
    #[test]
    fn a_tracks_band_and_wheels_run_with_the_ground() {
        let outline = [(-11.0, 0.5), (-11.0, 4.5), (12.5, 4.5), (12.5, 0.5)];
        let quad = side(&outline);
        let around = Around { offset: Vec3::new(-2.0, 0.0, 0.0), ..Default::default() };
        let kit = build(&[face(&quad, -Vec3::X, FaceKind::TrackSide, &[false; 4])], &around, &mut Budget::default());
        let u = |v: &ModelVertex| v.uv[0] as f32 / 65535.0;
        let band: Vec<&ModelVertex> = kit.vertices.iter().filter(|v| v.layer & WHEEL == 0 && v.tread[0] != 0.0).collect();
        let hubs: Vec<&ModelVertex> = kit.vertices.iter().filter(|v| v.layer & WHEEL != 0).collect();
        assert!(!band.is_empty() && hubs.len() == HUB_SIDES * 7, "{} band corners, {} hub corners", band.len(), hubs.len());
        // The band: round it once, its length the inverse of its rate.
        let (lo, hi) = band.iter().map(|v| u(v)).fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
        assert!(lo < 1e-4 && hi > 1.0 - 1e-4, "u from {lo} to {hi}");
        let length = 1.0 / band[0].tread[0];
        assert!((40.0..60.0).contains(&length), "a band {length} long round wheels 23.5 apart at most");
        let links = (band[0].uv[1] as f32 / 65535.0 * 255.0).round();
        assert!(band.iter().all(|v| v.tread == band[0].tread && v.uv[1] == band[0].uv[1]));
        assert!((8.0..=255.0).contains(&links) && (length / links - 0.95).abs() < 0.4, "{links} links of {}", length / links);
        // Its top run, going back as u rises, and its bottom run, going forward.
        let top = band.iter().filter(|v| v.pos[1] > 4.0).max_by(|a, b| a.pos[2].total_cmp(&b.pos[2])).unwrap();
        let top_back = band.iter().filter(|v| v.pos[1] > 4.0).min_by(|a, b| a.pos[2].total_cmp(&b.pos[2])).unwrap();
        assert!(u(top) < u(top_back), "u runs back along the top");
        // Hubs: a radian for each radius of the wheel (a little bigger than the hub).
        for v in &hubs {
            let hub_radius = (Vec2::new(u(v), v.uv[1] as f32 / 65535.0) - 0.5).length() * 2.0;
            assert!((hub_radius - 1.0).abs() < 0.02, "a hub's rim at {hub_radius} of its radius");
            assert!((0.8..2.5).contains(&(1.0 / v.tread[0])), "a wheel of radius {}", 1.0 / v.tread[0]);
        }
        // Both run as a track 14.5 to the side of the unit's middle (the piece's place and the
        // side's own).
        assert!(band.iter().chain(&hubs).all(|v| (v.tread[1] + 14.5).abs() < 0.01));
    }

    #[test]
    fn a_sloping_tracks_wheels_and_band_keep_inside_it() {
        // Sides whose ends slope in towards the ground, as most of TA's do (a Flash's, an
        // advanced Construction Vehicle's): the band and the end wheels stay inside them.
        for outline in [vec![(-10.2, 0.6), (8.2, 0.6), (11.2, 4.7), (-12.7, 4.7)], vec![(-11.25, 0.25), (6.25, 0.25), (13.75, 9.0), (-17.5, 9.0)]] {
            let quad = side(&outline);
            let kit = build(&[face(&quad, -Vec3::X, FaceKind::TrackSide, &[false; 4])], &Around::default(), &mut Budget::default());
            assert!(kit.wheels >= 3, "{outline:?}: {} wheels", kit.wheels);
            within(&kit, &outline).unwrap_or_else(|e| panic!("{outline:?}: {e}"));
        }
        // A short wedge of a pod (a Construction Vehicle's, a Construction Truck's) has no room
        // for a track: it keeps TA's paint.
        for outline in [vec![(-16.25, 0.0), (-12.5, 0.0), (-8.75, 8.75), (-20.0, 8.75)], vec![(10.0, 0.05), (2.5, 0.05), (0.0, 8.8), (12.5, 8.8)]] {
            let quad = side(&outline);
            let kit = build(&[face(&quad, -Vec3::X, FaceKind::TrackSide, &[false; 4])], &Around::default(), &mut Budget::default());
            assert_eq!((kit.wheels, kit.vertices.len()), (0, 4), "{outline:?}");
        }
    }

    #[test]
    fn a_side_of_two_faces_gets_one_row_of_wheels() {
        // ARMMART's sides: a lower and an upper face, together a long hexagon.
        let lower = [(-19.25, 0.5), (-3.5, 0.5), (1.0, 6.0), (-22.75, 6.0)];
        let upper = [(-3.5, 11.0), (-18.75, 11.0), (-22.75, 6.0), (1.0, 6.0)];
        let (a, b) = (side(&lower), side(&upper));
        let faces = [face(&a, -Vec3::X, FaceKind::TrackSide, &[false; 4]), face(&b, -Vec3::X, FaceKind::TrackSide, &[false; 4])];
        let kit = build(&faces, &Around::default(), &mut Budget::default());
        assert!((3..=8).contains(&kit.wheels), "{} wheels", kit.wheels);
        let hexagon = [(-19.25, 0.5), (-3.5, 0.5), (1.0, 6.0), (-3.5, 11.0), (-18.75, 11.0), (-22.75, 6.0)];
        within(&kit, &hexagon).unwrap();
        // One band: the wheels in a single row at one height.
        let hubs: Vec<f32> = kit.vertices.iter().filter(|v| v.color[0] == HUB.0[0]).map(|v| v.pos[1]).collect();
        let (lo, hi) = hubs.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &y| (lo.min(y), hi.max(y)));
        assert!(hi - lo < 6.0, "two rows: hubs from {lo} to {hi}");
    }

    #[test]
    fn only_the_outer_sides_of_tracks_get_wheels() {
        let outline = [(-11.0, 0.5), (-11.0, 4.5), (12.5, 4.5), (12.5, 0.5)];
        // The left pod's inner side (at x = -7.5, facing the hull, +x) stays as TA painted it.
        let inner: Vec<ModelVertex> = outline.iter().map(|&(z, y)| vertex(Vec3::new(-7.5, y, z), Vec3::X, [0.0, 0.0])).collect();
        let kit = build(&[face(&inner, Vec3::X, FaceKind::TrackSide, &[false; 4])], &Around::default(), &mut Budget::default());
        assert_eq!((kit.wheels, kit.vertices.len()), (0, 4));
        // So does a face painted with tracks facing forwards (a Kbot's, a submarine's tail).
        let front: Vec<ModelVertex> = [(-4.0, 1.0), (4.0, 1.0), (4.0, 6.0), (-4.0, 6.0)].iter().map(|&(x, y)| vertex(Vec3::new(x, y, 10.0), Vec3::Z, [0.0, 0.0])).collect();
        let kit = build(&[face(&front, Vec3::Z, FaceKind::TrackSide, &[false; 4])], &Around::default(), &mut Budget::default());
        assert_eq!(kit.wheels, 0);
        // A track in a piece of its own (a pod at the side, the piece's origin at x = -12): which
        // side faces out is the model's, not the piece's.
        let pod: Vec<ModelVertex> = outline.iter().map(|&(z, y)| vertex(Vec3::new(2.0, y, z), Vec3::X, [0.0, 0.0])).collect();
        let around = Around { offset: Vec3::new(-12.0, 0.0, 0.0), ..Default::default() };
        let kit = build(&[face(&pod, Vec3::X, FaceKind::TrackSide, &[false; 4])], &around, &mut Budget::default());
        assert_eq!(kit.wheels, 0, "the pod's inner side (x = -10 in the model, facing +x)");
    }

    #[test]
    fn the_budget_stops_the_bolts() {
        let quads: Vec<Vec<ModelVertex>> = (0..30)
            .map(|i| {
                let mut q = square(8.0);
                for v in &mut q {
                    v.pos[0] += i as f32 * 10.0;
                }
                q
            })
            .collect();
        let faces: Vec<KitFace> = quads.iter().map(|q| face(q, Vec3::Y, FaceKind::Plating, &[false; 4])).collect();
        let mut budget = Budget { bolts: 10, wheels: 0 };
        let kit = build(&faces, &Around::default(), &mut budget);
        assert_eq!(kit.plates, 30);
        assert_eq!(kit.bolts, 8);
    }
}
