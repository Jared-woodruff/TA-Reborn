//! Give a flat skeleton joint the depth its attached pieces need. Some TA walkers (the
//! Hammer and Jethro) have a pelvis made of one horizontal polygon: its torso and legs are deliberately
//! separated above and below it, hidden by TA's high camera. Closing holes cannot connect
//! these pieces because the missing hip was never a shell. Extend that polygon to its nearby
//! children's pivots, including a thigh attached just beyond its rim. This remains on the pelvis, so all of the
//! original walk, aim and recoil transforms keep working.

use glam::{Vec2, Vec3};
use ta_formats::model3do::{FIXED_ONE, Model3do};

use super::{caps::Polygon, face_normal, is_effect_piece};

/// A support's faces, in its piece's local coordinates. Only a small untextured horizontal
/// joint with solid children attached both above and below qualifies; fins, selection plates,
/// gun covers, and already modelled joints retain their geometry.
pub(super) fn supports(model: &Model3do, shapes: &[Vec<Polygon>]) -> Vec<Vec<Polygon>> {
    let fixed = |v: [i32; 3]| Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / FIXED_ONE;
    model.pieces.iter().enumerate().map(|(pi, piece)| {
        let faces = &shapes[pi];
        if faces.len() != 1 || piece.parent.is_none() || is_effect_piece(&piece.name)
            || piece.primitives.iter().any(|p| p.texture.is_some()) || piece.children.len() < 3
        {
            return Vec::new();
        }
        let plate = &faces[0];
        if !(3..=8).contains(&plate.len()) || face_normal(plate).y.abs() < 0.999 {
            return Vec::new();
        }
        let low = plate.iter().copied().fold(Vec3::splat(f32::MAX), Vec3::min);
        let high = plate.iter().copied().fold(Vec3::splat(f32::MIN), Vec3::max);
        let width = (high.x - low.x).min(high.z - low.z);
        // A narrow local joint, with no attempt to turn a large platform into a solid block.
        if !(0.5..=8.0).contains(&width) || (high.x - low.x).max(high.z - low.z) > 8.0
            || high.y - low.y > 0.02 || !plate.iter().all(|&p| inside(plate, p))
        {
            return Vec::new();
        }
        let y = (low.y + high.y) * 0.5;
        let mut bottom = y;
        let mut top = y;
        let mut below = 0;
        let mut above = 0;
        let mut extensions = Vec::new();
        for &child in &piece.children {
            let part = &model.pieces[child];
            let pivot = fixed(part.offset);
            if is_effect_piece(&part.name) || shapes[child].len() < 4
                || (pivot.y - y).abs() > width
            {
                return Vec::new();
            }
            // The pivot must actually belong to the child's body, not be a remote animation
            // hinge for a door or a weapon. A small overlap keeps contact through rotation.
            let lo = shapes[child].iter().flatten().copied().fold(Vec3::splat(f32::MAX), Vec3::min);
            let hi = shapes[child].iter().flatten().copied().fold(Vec3::splat(f32::MIN), Vec3::max);
            if !Vec3::ZERO.cmpge(lo - Vec3::splat(0.1)).all() || !Vec3::ZERO.cmple(hi + Vec3::splat(0.1)).all() {
                return Vec::new();
            }
            if !inside(plate, pivot) {
                // Jethro's thighs overlap its narrow pelvis, but their pivots sit just
                // outside it. Require contact with the actual projected faces, not only
                // their bounds, and never reach out to a distant hinge or separate body.
                if distance_to_rim(plate, pivot) > width * 0.5 || !overlaps(plate, &shapes[child], pivot) {
                    return Vec::new();
                }
                for x in [-0.15, 0.15] {
                    for z in [-0.15, 0.15] {
                        extensions.push(Vec3::new(pivot.x + x, y, pivot.z + z));
                    }
                }
            }
            below += usize::from(pivot.y < y - 0.25);
            above += usize::from(pivot.y > y + 0.25);
            bottom = bottom.min(pivot.y - 0.15);
            top = top.max(pivot.y + 0.15);
        }
        if below < 2 || above == 0 {
            return Vec::new();
        }
        if extensions.is_empty() {
            return extrude(plate, bottom, top);
        }
        extensions.extend_from_slice(plate);
        let outline = convex_hull(extensions);
        let lo = outline.iter().copied().fold(Vec3::splat(f32::MAX), Vec3::min);
        let hi = outline.iter().copied().fold(Vec3::splat(f32::MIN), Vec3::max);
        if (hi.x - lo.x).max(hi.z - lo.z) > 8.0 {
            return Vec::new();
        }
        extrude(&outline, bottom, top)
    }).collect()
}

/// Convex outline in the horizontal plane; points on the rim are attached too.
fn inside(plate: &[Vec3], p: Vec3) -> bool {
    let mut positive = false;
    let mut negative = false;
    for k in 0..plate.len() {
        let a = plate[k];
        let b = plate[(k + 1) % plate.len()];
        let cross = (b.x - a.x) * (p.z - a.z) - (b.z - a.z) * (p.x - a.x);
        positive |= cross > 0.02;
        negative |= cross < -0.02;
    }
    !(positive && negative)
}

fn horizontal(p: Vec3) -> Vec2 { Vec2::new(p.x, p.z) }

fn distance_to_rim(plate: &[Vec3], p: Vec3) -> f32 {
    let p = horizontal(p);
    (0..plate.len()).map(|k| {
        let a = horizontal(plate[k]);
        let edge = horizontal(plate[(k + 1) % plate.len()]) - a;
        let t = ((p - a).dot(edge) / edge.length_squared().max(1e-8)).clamp(0.0, 1.0);
        p.distance(a + edge * t)
    }).fold(f32::MAX, f32::min)
}

/// Clip each projected face against the convex joint. This also catches a thigh face
/// crossing the whole joint when none of its corners lie inside it.
fn overlaps(plate: &[Vec3], faces: &[Polygon], offset: Vec3) -> bool {
    let orientation = (0..plate.len()).map(|k| horizontal(plate[k]).perp_dot(horizontal(plate[(k + 1) % plate.len()]))).sum::<f32>().signum();
    faces.iter().any(|face| {
        let mut clipped: Vec<Vec2> = face.iter().map(|&p| horizontal(p + offset)).collect();
        for k in 0..plate.len() {
            let Some(&last) = clipped.last() else { return false };
            let a = horizontal(plate[k]);
            let edge = horizontal(plate[(k + 1) % plate.len()]) - a;
            let side = |p| orientation * edge.perp_dot(p - a);
            let mut previous = last;
            let mut previous_side = side(previous);
            let mut next = Vec::new();
            for &current in &clipped {
                let current_side = side(current);
                if (previous_side >= 0.0) != (current_side >= 0.0) {
                    next.push(previous.lerp(current, previous_side / (previous_side - current_side)));
                }
                if current_side >= 0.0 { next.push(current); }
                previous = current;
                previous_side = current_side;
            }
            clipped = next;
        }
        !clipped.is_empty()
    })
}

/// The original convex plate plus only the accepted pivots' small attachment margins.
fn convex_hull(mut points: Vec<Vec3>) -> Polygon {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.z.total_cmp(&b.z)));
    points.dedup_by(|a, b| a.x == b.x && a.z == b.z);
    let mut hull = Vec::new();
    for run in [points.clone(), points.into_iter().rev().collect()] {
        let start = hull.len();
        for p in run {
            while hull.len() >= start + 2 {
                let a = hull[hull.len() - 2];
                let b = hull[hull.len() - 1];
                if horizontal(b - a).perp_dot(horizontal(p - b)) > 1e-6 { break; }
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

fn extrude(plate: &[Vec3], bottom: f32, top: f32) -> Vec<Polygon> {
    let at = |y| plate.iter().map(|p| Vec3::new(p.x, y, p.z)).collect::<Polygon>();
    let mut lo = at(bottom);
    let mut hi = at(top);
    if face_normal(&lo).y > 0.0 { lo.reverse(); }
    if face_normal(&hi).y < 0.0 { hi.reverse(); }
    let center = plate.iter().copied().sum::<Vec3>() / plate.len() as f32;
    let mut faces = vec![lo, hi];
    for k in 0..plate.len() {
        let a = plate[k];
        let b = plate[(k + 1) % plate.len()];
        let mut wall = vec![Vec3::new(a.x, bottom, a.z), Vec3::new(b.x, bottom, b.z), Vec3::new(b.x, top, b.z), Vec3::new(a.x, top, a.z)];
        if face_normal(&wall).dot((a + b) * 0.5 - center) < 0.0 { wall.reverse(); }
        faces.push(wall);
    }
    faces
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::model3do::{Piece, Primitive};

    fn skeleton() -> (Model3do, Vec<Vec<Polygon>>) {
        let piece = |parent, children, offset: Vec3| Piece {
            name: "joint".into(), parent, children,
            offset: [(offset.x * FIXED_ONE) as i32, (offset.y * FIXED_ONE) as i32, (-offset.z * FIXED_ONE) as i32],
            vertices: Vec::new(), primitives: Vec::new(), selection_primitive: None,
        };
        let mut model = Model3do { pieces: vec![
            piece(None, vec![1], Vec3::ZERO),
            piece(Some(0), vec![2, 3, 4], Vec3::ZERO),
            piece(Some(1), vec![], Vec3::new(-1.5, -2.5, 0.0)),
            piece(Some(1), vec![], Vec3::new(1.5, -2.5, 0.0)),
            piece(Some(1), vec![], Vec3::new(0.0, 2.0, 0.0)),
        ] };
        model.pieces[1].primitives.push(Primitive { color_index: 0, indices: vec![0, 1, 2, 3], texture: None });
        let plate = vec![Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, 2.0), Vec3::new(-2.0, 0.0, 2.0)];
        let body = extrude(&plate.iter().map(|p| *p * 0.25).collect::<Vec<_>>(), -1.0, 1.0);
        (model, vec![Vec::new(), vec![plate], body.clone(), body.clone(), body])
    }

    #[test]
    fn flat_skeleton_joint_spans_the_attachments_without_changing_source_faces() {
        let (model, shapes) = skeleton();
        let original = shapes.clone();
        let added = supports(&model, &shapes);
        assert_eq!(shapes, original);
        assert_eq!(added.iter().map(Vec::len).collect::<Vec<_>>(), vec![0, 6, 0, 0, 0]);
        let low = added[1].iter().flatten().map(|p| p.y).fold(f32::MAX, f32::min);
        let high = added[1].iter().flatten().map(|p| p.y).fold(f32::MIN, f32::max);
        assert!((low + 2.65).abs() < 1e-5 && (high - 2.15).abs() < 1e-5);
        assert_eq!(added[1], extrude(&shapes[1][0], low, high), "inside pivots must retain the original outline");
    }

    #[test]
    fn nearby_thigh_pivots_outside_the_plate_get_solid_connections() {
        let (mut model, shapes) = skeleton();
        for (child, x) in [(2, -2.4), (3, 2.4)] {
            model.pieces[child].offset[0] = (x * FIXED_ONE) as i32;
        }
        let original = shapes.clone();
        let added = supports(&model, &shapes);
        assert_eq!(shapes, original);
        assert_eq!(added[1].len(), 10);
        let center = Vec3::new(0.0, -0.25, 0.0);
        for face in &added[1] {
            let middle = face.iter().copied().sum::<Vec3>() / face.len() as f32;
            assert!(face_normal(face).dot(middle - center) > 0.0);
            assert!(face.iter().all(|p| p.x.abs() <= 2.551 && p.z.abs() <= 2.0));
        }
        for &child in &model.pieces[1].children {
            let p = model.pieces[child].offset;
            let p = Vec3::new(p[0] as f32, p[1] as f32, -(p[2] as f32)) / FIXED_ONE;
            assert!(inside(&added[1][0], p));
            assert!(distance_to_rim(&added[1][0], p) >= 0.149);
        }
    }

    #[test]
    fn outside_pivots_need_nearby_overlapping_bodies() {
        let (mut model, shapes) = skeleton();
        model.pieces[2].offset[0] = -3 * FIXED_ONE as i32;
        assert!(supports(&model, &shapes)[1].is_empty(), "a nearby separate body must not be bridged");
        model.pieces[2].offset[0] = (-4.1 * FIXED_ONE) as i32;
        let mut wide = shapes.clone();
        wide[2].iter_mut().flatten().for_each(|p| p.x *= 6.0);
        assert!(supports(&model, &wide)[1].is_empty(), "overlap must not allow a distant hinge");
        model.pieces[2].offset[0] = (-2.4 * FIXED_ONE) as i32;
        let mut remote = shapes;
        remote[2].iter_mut().flatten().for_each(|p| p.x += 1.0);
        assert!(supports(&model, &remote)[1].is_empty(), "the pivot must remain inside its body");
    }

    #[test]
    fn joint_overlap_checks_projected_faces_instead_of_bounds() {
        let plate = vec![Vec3::new(-2.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -2.0), Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 2.0)];
        let outside = vec![Vec3::new(1.5, 0.0, 1.5), Vec3::new(2.0, 0.0, 1.5), Vec3::new(2.0, 1.0, 2.0), Vec3::new(1.5, 1.0, 2.0)];
        assert!(!overlaps(&plate, &[outside], Vec3::ZERO));
        let crossing = vec![Vec3::new(-3.0, 0.0, -0.2), Vec3::new(3.0, 0.0, -0.2), Vec3::new(3.0, 1.0, 0.2), Vec3::new(-3.0, 1.0, 0.2)];
        assert!(crossing.iter().all(|&p| !inside(&plate, p)));
        assert!(overlaps(&plate, std::slice::from_ref(&crossing), Vec3::ZERO));
        let reversed: Vec<Vec3> = plate.into_iter().rev().collect();
        assert!(overlaps(&reversed, &[crossing], Vec3::ZERO));
    }

    #[test]
    fn sheets_platforms_and_remote_hinges_are_not_filled() {
        let (model, shapes) = skeleton();
        let mut painted = model.clone();
        painted.pieces[1].primitives[0].texture = Some("paint".into());
        assert!(supports(&painted, &shapes)[1].is_empty());
        let mut no_torso = model.clone();
        no_torso.pieces[4].offset[1] = -2 * FIXED_ONE as i32;
        assert!(supports(&no_torso, &shapes)[1].is_empty());
        let mut platform = shapes.clone();
        platform[1][0].iter_mut().for_each(|p| p.x *= 10.0);
        assert!(supports(&model, &platform)[1].is_empty());
        let mut remote = shapes.clone();
        remote[4].iter_mut().flatten().for_each(|p| p.y += 3.0);
        assert!(supports(&model, &remote)[1].is_empty());
        let mut sheet = shapes;
        sheet[4].truncate(1);
        assert!(supports(&model, &sheet)[1].is_empty());
    }

    #[test]
    fn joint_support_keeps_its_outline_and_faces_outward() {
        let plate = vec![Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, 2.0), Vec3::new(-2.0, 0.0, 2.0)];
        assert!(inside(&plate, Vec3::new(2.0, -2.5, 0.0)));
        assert!(!inside(&plate, Vec3::new(2.5, 0.0, 0.0)));
        let faces = extrude(&plate, -2.65, 2.15);
        assert_eq!(faces.len(), 6);
        let center = Vec3::new(0.0, -0.25, 0.0);
        for face in faces {
            let middle = face.iter().copied().sum::<Vec3>() / face.len() as f32;
            assert!(face_normal(&face).dot(middle - center) > 0.0);
            assert!(face.iter().all(|p| p.x.abs() == 2.0 && p.z.abs() == 2.0));
        }
    }

    #[test]
    #[ignore = "requires an installed copy of Total Annihilation"]
    fn installed_flat_skeleton_joints_connect_their_children() {
        let vfs = super::super::caps::tests::install().expect("installed TA");
        let mut joined = Vec::new();
        for path in vfs.list("objects3d", Some("3do"), true) {
            let model = Model3do::parse(&vfs.read(&path).unwrap()).unwrap();
            let (shapes, _, _) = super::super::caps::tests::model_faces(&model);
            let additions = supports(&model, &shapes);
            for (pi, faces) in additions.iter().enumerate().filter(|(_, f)| !f.is_empty()) {
                let piece = &model.pieces[pi];
                eprintln!("{path}: {} gets {} support faces", piece.name, faces.len());
                joined.push((path.clone(), piece.name.clone()));
                let lo = faces.iter().flatten().copied().fold(Vec3::splat(f32::MAX), Vec3::min);
                let hi = faces.iter().flatten().copied().fold(Vec3::splat(f32::MIN), Vec3::max);
                for &child in &piece.children {
                    let p = model.pieces[child].offset;
                    let p = Vec3::new(p[0] as f32, p[1] as f32, -(p[2] as f32)) / FIXED_ONE;
                    assert!(p.cmpge(lo).all() && p.cmple(hi).all(), "{} misses {}", piece.name, model.pieces[child].name);
                }
            }
        }
        for unit in ["armham", "armjeth"] {
            assert!(joined.iter().any(|(path, piece)| path.ends_with(&format!("/{unit}.3do")) && piece == "pelvis"), "{unit}: {joined:?}");
        }
    }
}
