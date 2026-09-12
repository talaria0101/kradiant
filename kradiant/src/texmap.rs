//! Texture coordinate mapping helpers for classic brush faces.
//!
//! This implements the familiar "texture axis from dominant normal" projection with per‑face
//! shift/rotate/scale applied, matching how typical level‑editor surface inspector parameters
//! are interpreted.

use crate::map::Face;
use crate::{IVec2, Vec2, Vec3};

/// Precomputed UV projection for a single brush face.
///
/// This avoids doing sin/cos and basis selection per vertex when tessellating large meshes.
#[derive(Debug, Clone, Copy)]
pub struct FaceUvMapper {
    s_axis: Vec3,
    t_axis: Vec3,
    scale_x: f32,
    scale_y: f32,
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
            scale_x,
            scale_y,
            shift: face.params.shift,
            inv_tex_w: 1.0 / tex_w,
            inv_tex_h: 1.0 / tex_h,
        }
    }

    /// Compute UVs in "repeat" space (not clamped to 0..1).
    ///
    /// Matches Q3Radiant `Face_TextureVectors`: world coordinates are projected
    /// onto the texture axes, so texture alignment is relative to the world grid
    /// (not the face origin), exactly like Radiant/CoDRadiant.
    pub fn uv(&self, point: Vec3) -> Vec2 {
        // Scale makes texture appear smaller/larger on surface:
        // scale=0.25 means texture is 1/4 size, so 4× more repeats
        // Formula: (world_units / (tex_size * scale)) + (shift / tex_size)
        let world_scale_x = self.inv_tex_w / self.scale_x; // = 1 / (tex_w * scale_x)
        let world_scale_y = self.inv_tex_h / self.scale_y; // = 1 / (tex_h * scale_y)
        let u = point.dot(self.s_axis) * world_scale_x + self.shift.x as f32 * self.inv_tex_w;
        let v = point.dot(self.t_axis) * world_scale_y + self.shift.y as f32 * self.inv_tex_h;
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
/// Returns (S axis, T axis) in map space. Uses consistent axes regardless of normal direction
/// to prevent texture mirroring on opposite faces of a brush (matching CoDRadiant behavior).
pub fn q3_texture_axes_from_normal(n: Vec3) -> (Vec3, Vec3) {
    let ax = n.x.abs();
    let ay = n.y.abs();
    let az = n.z.abs();

    // Use consistent axes based only on which component is dominant (absolute value).
    // Do NOT flip based on normal sign - this ensures opposite faces have the same
    // texture orientation and don't appear mirrored.
    // T-axis is negated to flip V coordinate (textures were upside down).
    if az >= ax && az >= ay {
        // Z-dominant (floor/ceiling): project onto XY plane
        (Vec3::X, -Vec3::Y)
    } else if ax >= ay {
        // X-dominant (east/west walls): project onto YZ plane
        (Vec3::Y, -Vec3::Z)
    } else {
        // Y-dominant (north/south walls): project onto XZ plane
        (Vec3::X, -Vec3::Z)
    }
}

/// Rotate the (S,T) texture axes in their plane by `angle_rad`.
///
/// Matches Q3Radiant `Face_TextureVectors` rotation:
/// `S' = cos*S - sin*T`, `T' = sin*S + cos*T`.
pub fn rotate_texture_axes(s: Vec3, t: Vec3, angle_rad: f32) -> (Vec3, Vec3) {
    if angle_rad.abs() < 1e-6 {
        return (s, t);
    }
    let c = angle_rad.cos();
    let sn = angle_rad.sin();
    let s2 = s * c - t * sn;
    let t2 = s * sn + t * c;
    (s2, t2)
}

/// Compute Radiant-style UVs for a face at a point, normalized by texture dimensions.
///
/// `tex_w/tex_h` are in pixels. UVs are returned in "repeat" space (not clamped to 0..1),
/// so renderers should use `GL_REPEAT`/wrap repeat for correct tiling.
pub fn face_uv(face: &Face, point: Vec3, tex_w: f32, tex_h: f32) -> Vec2 {
    FaceUvMapper::new(face, tex_w, tex_h).uv(point)
}

/// Texture Lock: the shift delta (in texels) that keeps a face's texture
/// visually attached under a translation by `delta`.
///
/// The world-projected UV term shifts by `delta.dot(s_axis) / (tex_w * scale)`.
/// To cancel this, the shift (stored in texels) must change by
/// `-delta.dot(axis) / scale` (the texture size drops out).
pub fn translation_offset_shift(face: &Face, delta: Vec3) -> IVec2 {
    let n = face_plane_normal(face);
    let (s_axis, t_axis) = q3_texture_axes_from_normal(n);
    let (s_axis, t_axis) =
        rotate_texture_axes(s_axis, t_axis, (face.params.rotate as f32).to_radians());
    let scale_u = if face.params.scale.x.abs() < 1e-6 {
        1.0
    } else {
        face.params.scale.x
    };
    let scale_v = if face.params.scale.y.abs() < 1e-6 {
        1.0
    } else {
        face.params.scale.y
    };
    let raw_u = delta.dot(s_axis);
    let raw_v = delta.dot(t_axis);
    IVec2::new(
        (-raw_u / scale_u).round() as i32,
        (-raw_v / scale_v).round() as i32,
    )
}
