use crate::{Quat, Vec3, editing::{AffineRotate, AffineScale}, editor::viewport::{DragMode, Ortho, StretchMode}, map_utils::format_float_trim};

pub fn project_to_2d(v: Vec3, axis: Ortho) -> [f32; 2] {
    match axis {
        // Flip Y so +Y is up on screen (ImGui Y+ is down).
        Ortho::XY => [v.x, -v.y],
        // Flip Z so +Z is up on screen (ImGui Y+ is down).
        Ortho::XZ => [v.x, -v.z],
        Ortho::YZ => [v.y, -v.z],
    }
}

pub fn all_corners_3d(a: Vec3, b: Vec3) -> [Vec3; 8] {
    let mut out = [Vec3::ZERO; 8];
    let mut idx = 0;

    for &x in &[a.x, b.x] {
        for &y in &[a.y, b.y] {
            for &z in &[a.z, b.z] {
                out[idx] = Vec3::new(x, y, z);
                idx += 1;
            }
        }
    }

    out
}

pub fn origin_to_vec3(pos: &str) -> Vec3 {
    let parts: Vec<&str> = pos.split(" ").collect();
    Vec3 {
        x: parts[0].parse().unwrap(),
        y: parts[1].parse().unwrap(),
        z: parts[2].parse().unwrap(),
    }
}

pub fn vec3_to_origin(pos: Vec3) -> String {
    format!(
        "{} {} {}",
        format_float_trim(pos.x, 0),
        format_float_trim(pos.y, 0),
        format_float_trim(pos.z, 0)
    )
}

/// Entity `angles` are stored in degrees; keep float precision so small
/// rotation deltas are not lost to integer rounding.
pub fn vec3_to_angles(angles: Vec3) -> String {
    format!(
        "{} {} {}",
        format_float_trim(angles.x, 2),
        format_float_trim(angles.y, 2),
        format_float_trim(angles.z, 2)
    )
}

pub fn vec3_from_whitespace_triplet(text: &str) -> Option<Vec3> {
    let mut it = text.split_whitespace();
    let x = it.next()?.parse().ok()?;
    let y = it.next()?.parse().ok()?;
    let z = it.next()?.parse().ok()?;
    Some(Vec3::new(x, y, z))
}

/// CoD entity `angles` are stored as `pitch yaw roll` in degrees.
/// The engine's rotation matrix is `Rz(yaw) * Ry(pitch) * Rx(roll)` for a
/// Z-up world, with pitch measured DOWNWARD-positive (forward[2] = -sin(pitch)).
/// This matches the game's `AngleVectors`/`AnglesToAxis` math (CoD1/UO engine:
/// forward = (cp*cy, cp*sy, -sp), and the engine's left-handed "right" is
/// negated to form the orthonormal matrix, giving exactly Rz(yaw)*Ry(pitch)*Rx(roll)).
pub fn entity_angles_to_quat(angles: Vec3) -> Quat {
    let pitch = angles.x.to_radians();
    let yaw = angles.y.to_radians();
    let roll = angles.z.to_radians();

    Quat::from_rotation_z(yaw) * Quat::from_rotation_y(pitch) * Quat::from_rotation_x(roll)
}

/// Inverse of `entity_angles_to_quat` (id Tech 3 `AnglesFromMatrix`):
/// from the basis columns f = q*X, r = q*Y, u = q*Z extract
/// `yaw = atan2(f.y, f.x)` (normalized to [0, 360)),
/// `pitch = -atan2(f.z, sqrt(f.x^2 + f.y^2))`,
/// `roll = atan2(r.z, u.z)`.
pub fn quat_to_entity_angles(q: Quat) -> Vec3 {
    let f = q * Vec3::X;
    let r = q * Vec3::Y;
    let u = q * Vec3::Z;

    let yaw = f.y.atan2(f.x).to_degrees().rem_euclid(360.0);
    let pitch = -f.z.atan2((f.x * f.x + f.y * f.y).sqrt()).to_degrees();
    let roll = r.z.atan2(u.z).to_degrees();

    Vec3::new(pitch, yaw, roll)
}

pub fn entity_angles_forward(angles: Vec3) -> Vec3 {
    (entity_angles_to_quat(angles) * Vec3::X).normalize_or_zero()
}

/// Center of a style proxy box (Base: box bottom at origin; Center: box centered on origin).
pub fn proxy_box_center(
    anchor: crate::editor::config::EntityDrawAnchor,
    origin: Vec3,
    size: Vec3,
) -> Vec3 {
    match anchor {
        crate::editor::config::EntityDrawAnchor::Base => {
            origin + Vec3::new(0.0, 0.0, size.z * 0.5)
        }
        crate::editor::config::EntityDrawAnchor::Center => origin,
    }
}

/// Center of the front side of a model's bounds: the box's vertical center
/// pushed toward the face the arrow points through.
pub fn model_front_center(model_origin: Vec3, mins: Vec3, maxs: Vec3, forward: Vec3) -> Vec3 {
    let center = model_origin + (mins + maxs) * 0.5;
    let f = forward.normalize_or_zero();
    let half_depth = 0.5 * (f.abs() * (maxs - mins)).dot(Vec3::ONE);
    center + f * half_depth
}

pub const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

pub fn oriented_box_corners(center: Vec3, half_size: Vec3, rot: Option<Quat>) -> [Vec3; 8] {
    let mut out = [Vec3::ZERO; 8];
    let mut idx = 0;
    for &x in &[-half_size.x, half_size.x] {
        for &y in &[-half_size.y, half_size.y] {
            for &z in &[-half_size.z, half_size.z] {
                let p = Vec3::new(x, y, z);
                out[idx] = center + rot.map(|r| r * p).unwrap_or(p);
                idx += 1;
            }
        }
    }
    out
}

pub fn box_line_vertices(center: Vec3, size: Vec3, rot: Option<Quat>) -> Vec<Vec3> {
    let corners = oriented_box_corners(center, size * 0.5, rot);
    let mut out = Vec::with_capacity(BOX_EDGES.len() * 2);
    for &(a, b) in &BOX_EDGES {
        out.push(corners[a]);
        out.push(corners[b]);
    }
    out
}

pub fn box_line_vertices_from_base(base: Vec3, size: Vec3, rot: Option<Quat>) -> Vec<Vec3> {
    let center = base + Vec3::new(0.0, 0.0, size.z * 0.5);
    box_line_vertices(center, size, rot)
}

fn model_bounds_corners(origin: Vec3, mins: Vec3, maxs: Vec3, rot: Option<Quat>) -> [Vec3; 8]
{
    let corners = [
        Vec3::new(mins.x, mins.y, mins.z),
        Vec3::new(maxs.x, mins.y, mins.z),
        Vec3::new(mins.x, maxs.y, mins.z),
        Vec3::new(maxs.x, maxs.y, mins.z),
        Vec3::new(mins.x, mins.y, maxs.z),
        Vec3::new(maxs.x, mins.y, maxs.z),
        Vec3::new(mins.x, maxs.y, maxs.z),
        Vec3::new(maxs.x, maxs.y, maxs.z),
    ];
    let corners: [Vec3; 8] = match rot {
        Some(r) => corners.map(|c| origin + r * c),
        None => corners.map(|c| origin + c),
    };

    corners
}

/// Wireframe box for a model's bounds (`mins`/`maxs` in XModel space, relative
/// to the root bone). The box spins about `origin` (the root bone) by `rot`,
/// matching how the model mesh rotates.
pub fn model_bounds_line_vertices(
    origin: Vec3,
    mins: Vec3,
    maxs: Vec3,
    rot: Option<Quat>,
) -> Vec<Vec3> {
    let corners = model_bounds_corners(origin, mins, maxs, rot);
    let mut out = Vec::with_capacity(BOX_EDGES.len() * 2);
    for &(a, b) in &BOX_EDGES {
        out.push(corners[a]);
        out.push(corners[b]);
    }
    out
}

/// Axis-aligned bounds enclosing the rotated model-bounds corners.
///
/// The mesh spins about its root bone (`origin`) by `rot`; returns the
/// world-space AABB that contains the rotated model (matches the wireframe
/// box used for clickable/selectable area).
pub fn model_bounds_aabb(
    origin: Vec3,
    mins: Vec3,
    maxs: Vec3,
    rot: Option<Quat>,
) -> (Vec3, Vec3) {
    let corners = model_bounds_corners(origin, mins, maxs, rot);
    let mut min_out = Vec3::splat(f32::MAX);
    let mut max_out = Vec3::splat(f32::MIN);
    for c in corners {
        min_out = min_out.min(c);
        max_out = max_out.max(c);
    }
    (min_out, max_out)
}

/// 36 triangle vertices (12 tris) forming a solid box centered on `center`,
/// optionally rotated by `rot`. Winding matches `solid_box_lit_vertices_from_base`.
pub fn solid_box_tri_vertices(center: Vec3, size: Vec3, rot: Option<Quat>) -> Vec<Vec3> {
    let half = size * 0.5;
    let corners = [
        Vec3::new(-half.x, -half.y, -half.z),
        Vec3::new(half.x, -half.y, -half.z),
        Vec3::new(-half.x, half.y, -half.z),
        Vec3::new(half.x, half.y, -half.z),
        Vec3::new(-half.x, -half.y, half.z),
        Vec3::new(half.x, -half.y, half.z),
        Vec3::new(-half.x, half.y, half.z),
        Vec3::new(half.x, half.y, half.z),
    ];
    let corners: [Vec3; 8] = corners.map(|c| center + rot.map(|r| r * c).unwrap_or(c));
    const FACES: [[usize; 6]; 6] = [
        [0, 2, 3, 0, 3, 1],
        [4, 5, 7, 4, 7, 6],
        [0, 1, 5, 0, 5, 4],
        [2, 6, 7, 2, 7, 3],
        [0, 4, 6, 0, 6, 2],
        [1, 3, 7, 1, 7, 5],
    ];
    let mut out = Vec::with_capacity(36);
    for idxs in FACES {
        for i in idxs {
            out.push(corners[i]);
        }
    }
    out
}

pub fn arrow_line_vertices(
    center: Vec3,
    forward: Vec3,
    scale: f32,
    rot_up_hint: Vec3,
) -> Vec<Vec3> {
    let forward = forward.normalize_or_zero();
    if forward.length_squared() <= 1.0e-8 {
        return Vec::new();
    }
    let mut right = forward.cross(rot_up_hint);
    if right.length_squared() <= 1.0e-8 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }
    let up = right.cross(forward).normalize_or_zero();
    let shaft_len = (scale * 0.75).max(16.0);
    let head_len = (shaft_len * 0.28).max(8.0);
    let head_w = (shaft_len * 0.12).max(4.0);
    let tip = center + forward * shaft_len;
    let back = tip - forward * head_len;
    let wing_l = back + right * head_w + up * (head_w * 0.15);
    let wing_r = back - right * head_w + up * (head_w * 0.15);
    vec![center, tip, tip, wing_l, tip, wing_r]
}

/// Tolerance used when comparing positions for shared-edge matching.
pub const EDGE_POINT_EPSILON: f32 = 0.05;

/// Returns true if two points are within the shared-edge matching tolerance.
pub fn same_point(a: Vec3, b: Vec3) -> bool {
    (a - b).length_squared() <= EDGE_POINT_EPSILON * EDGE_POINT_EPSILON
}

/// Find the shared edge points between two faces in a convex brush.
pub fn shared_edge_points(
    polys: &[(Vec<Vec3>, Vec<u32>)],
    face_a: usize,
    face_b: usize,
) -> Option<(Vec3, Vec3)> {
    let (a_verts, _) = polys.get(face_a)?;
    let (b_verts, _) = polys.get(face_b)?;
    if a_verts.len() < 2 || b_verts.len() < 2 {
        return None;
    }

    for i in 0..a_verts.len() {
        let a0 = a_verts[i];
        let a1 = a_verts[(i + 1) % a_verts.len()];
        for j in 0..b_verts.len() {
            let b0 = b_verts[j];
            let b1 = b_verts[(j + 1) % b_verts.len()];
            if (same_point(a0, b1) && same_point(a1, b0))
                || (same_point(a0, b0) && same_point(a1, b1))
            {
                return Some((a0, a1));
            }
        }
    }

    None
}

pub fn resolved_arrow_length(style: &crate::editor::config::EntityDrawStyle, fallback: f32) -> f32 {
    if style.arrow_length > 0.0 {
        style.arrow_length
    } else {
        fallback
    }
}

pub(crate) fn preview_point(dmode: DragMode, offset: Vec3, stretch_mode: Option<StretchMode>, stretch: Option<AffineScale>, rotate: Option<AffineRotate>, p: Vec3) -> Vec3
{
    match dmode {
        DragMode::MoveSelection | DragMode::MoveVertices => p + offset,
        DragMode::StretchSelection if stretch_mode.is_some() => {
            let stretch_mode = stretch_mode.unwrap();
            if stretch_mode == StretchMode::Scale {
                stretch
                .map(|x: AffineScale| x.apply_point(p))
                .unwrap_or(p)
            } else {
                p
            }
        }
        DragMode::NewBrush | DragMode::RectangularSelection => p,
        DragMode::RotateSelection => {
            rotate.map(|r| r.apply_point(p)).unwrap_or(p)
        }
        _ => p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_quat_eq(a: Quat, b: Quat, tol: f32) {
        let dot = a.dot(b).abs();
        assert!(
            dot > 1.0 - tol,
            "quats differ: dot = {dot} ({a:?} vs {b:?})"
        );
    }

    #[test]
    fn forward_matches_engine_anglevectors() {
        // CoD1/UO engine AngleVectors: forward = (cp*cy, cp*sy, -sp).
        for (p, y, r) in [
            (30.0, 47.0, 12.0),
            (-15.0, -120.0, 45.0),
            (90.0, 10.0, 0.0),
            (0.0, -39.0, 0.0),
        ] {
            let p = p as f32;
            let y = y as f32;
            let (sp, cp) = p.to_radians().sin_cos();
            let (sy, cy) = y.to_radians().sin_cos();
            let expect = Vec3::new(cp * cy, cp * sy, -sp);
            let got = entity_angles_forward(Vec3::new(p, y, r));
            assert!(
                (got - expect).length() < 1.0e-4,
                "forward for ({p},{y},{r}): {got:?} != {expect:?}"
            );
        }
    }

    #[test]
    fn codradiant_rotation_example() {
        // CoDRadiant: angles 0 -39 0 rotated 90 deg in the XZ (front) view
        // yields ~51 270 -90. That is Ry(+90) pre-multiplied on Rz(-39).
        let q_old = entity_angles_to_quat(Vec3::new(0.0, -39.0, 0.0));
        let q_new = Quat::from_axis_angle(Vec3::Y, 90.0_f32.to_radians()) * q_old;
        let a = quat_to_entity_angles(q_new);
        assert!((a.x - 51.0).abs() < 0.01, "pitch = {}", a.x);
        assert!((a.y - 270.0).abs() < 0.01, "yaw = {}", a.y);
        assert!((a.z + 90.0).abs() < 0.01, "roll = {}", a.z);
    }

    #[test]
    fn angles_round_trip() {
        let cases = [
            (0.0, 0.0, 0.0),
            (0.0, -39.0, 0.0),
            (51.0, 270.0, -90.0),
            (15.0, 120.0, -20.0),
            (-80.0, 10.0, 170.0),
            (45.0, -120.0, 45.0),
            (30.0, 359.0, 10.0),
            (-0.5, 180.5, -0.5),
        ];
        for (p, y, r) in cases {
            let a = Vec3::new(p, y, r);
            let b = quat_to_entity_angles(entity_angles_to_quat(a));
            assert_quat_eq(entity_angles_to_quat(b), entity_angles_to_quat(a), 1.0e-6);
        }
    }
}
