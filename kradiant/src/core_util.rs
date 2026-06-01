use glam::IVec3;

use crate::{Quat, Vec3, editor::viewport::Ortho};

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
        z: parts[2].parse().unwrap()
    }
}

pub fn vec3_from_whitespace_triplet(text: &str) -> Option<Vec3> {
    let mut it = text.split_whitespace();
    let x = it.next()?.parse().ok()?;
    let y = it.next()?.parse().ok()?;
    let z = it.next()?.parse().ok()?;
    Some(Vec3::new(x, y, z))
}

/// CoD entity `angles` are stored as `pitch yaw roll` in degrees.
/// We apply them in Z (yaw), Y (pitch), X (roll) order for a Z-up world.
pub fn entity_angles_to_quat(angles: Vec3) -> Quat {
    let pitch = angles.x.to_radians();
    let yaw = angles.y.to_radians();
    let roll = angles.z.to_radians();

    Quat::from_rotation_z(yaw)
        * Quat::from_rotation_y(-pitch)
        * Quat::from_rotation_x(roll)
}

pub fn entity_angles_forward(angles: Vec3) -> Vec3 {
    (entity_angles_to_quat(angles) * Vec3::X).normalize_or_zero()
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
