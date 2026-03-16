//! Texture coordinate mapping helpers for idTech3 / Radiant style brush faces.
//!
//! This is the classic "texture axis from dominant normal" projection with per-face
//! shift/rotate/scale applied, matching how Radiant's surface inspector parameters are used.

use crate::map::Face;
use crate::{IVec2, Vec2, Vec3};

/// Precomputed UV projection for a single brush face.
///
/// This avoids doing sin/cos and basis selection per vertex when tessellating large meshes.
#[derive(Debug, Clone, Copy)]
pub struct FaceUvMapper {
    s_axis: Vec3,
    t_axis: Vec3,
    inv_scale_x: f32,
    inv_scale_y: f32,
    shift: IVec2,
    inv_tex_w: f32,
    inv_tex_h: f32,
}

impl FaceUvMapper {
    pub fn new(face: &Face, tex_w: f32, tex_h: f32) -> Self {
        let tex_w = tex_w.max(1.0);
        let tex_h = tex_h.max(1.0);

        let n = face_plane_normal(face);
        let (s_axis, t_axis) = q3_texture_axes_from_normal(n);
        let (s_axis, t_axis) =
            rotate_texture_axes(s_axis, t_axis, (face.params.rotate as f32).to_radians());

        let scale_x = if face.params.scale.x.abs() < 1e-6 {
            1.0
        } else {
            face.params.scale.x
        };
        let scale_y = if face.params.scale.y.abs() < 1e-6 {
            1.0
        } else {
            face.params.scale.y
        };

        Self {
            s_axis,
            t_axis,
            inv_scale_x: 1.0 / scale_x,
            inv_scale_y: 1.0 / scale_y,
            shift: face.params.shift,
            inv_tex_w: 1.0 / tex_w,
            inv_tex_h: 1.0 / tex_h,
        }
    }

    /// Compute UVs in "repeat" space (not clamped to 0..1).
    pub fn uv(&self, point: Vec3) -> Vec2 {
        let u = (point.dot(self.s_axis) * self.inv_scale_x + self.shift.x as f32) * self.inv_tex_w;
        let v = (point.dot(self.t_axis) * self.inv_scale_y + self.shift.y as f32) * self.inv_tex_h;
        Vec2::new(u, v)
    }
}

/// Compute a face plane normal from its three plane points.
///
/// Note: this is in map space (CoD is Z-up). Downstream renderers may transform coordinates.
pub fn face_plane_normal(face: &Face) -> Vec3 {
    let a = face.plane_points[0];
    let b = face.plane_points[1];
    let c = face.plane_points[2];
    let n = (b - a).cross(c - a);
    if n.length_squared() < 1e-12 {
        Vec3::Z
    } else {
        n.normalize()
    }
}

/// Quake3-style axis selection: pick a stable basis for texture projection from the dominant normal axis.
///
/// Returns (S axis, T axis) in map space, forming a right-handed basis w.r.t `n`.
pub fn q3_texture_axes_from_normal(n: Vec3) -> (Vec3, Vec3) {
    let ax = n.x.abs();
    let ay = n.y.abs();
    let az = n.z.abs();

    let (s, mut t) = if az >= ax && az >= ay {
        (Vec3::X, -Vec3::Y)
    } else if ax >= ay {
        (Vec3::Y, -Vec3::Z)
    } else {
        (Vec3::X, -Vec3::Z)
    };

    // Ensure the basis is right-handed with respect to the face normal.
    if s.cross(t).dot(n) < 0.0 {
        t = -t;
    }
    (s, t)
}

/// Rotate the (S,T) texture axes in their plane by `angle_rad`.
pub fn rotate_texture_axes(s: Vec3, t: Vec3, angle_rad: f32) -> (Vec3, Vec3) {
    if angle_rad.abs() < 1e-6 {
        return (s, t);
    }
    let c = angle_rad.cos();
    let sn = angle_rad.sin();
    let s2 = s * c + t * sn;
    let t2 = t * c - s * sn;
    (s2, t2)
}

/// Compute Radiant-style UVs for a face at a point, normalized by texture dimensions.
///
/// `tex_w/tex_h` are in pixels. UVs are returned in "repeat" space (not clamped to 0..1),
/// so renderers should use `GL_REPEAT`/wrap repeat for correct tiling.
pub fn face_uv(face: &Face, point: Vec3, tex_w: f32, tex_h: f32) -> Vec2 {
    FaceUvMapper::new(face, tex_w, tex_h).uv(point)
}
