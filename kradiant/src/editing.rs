use crate::map::{
    Brush, BrushContent, BrushId, Entity, EntityId, Face, Map, SurfaceFlags, TextureParams,
};
use crate::{IVec3, Quat, Vec3};

#[derive(Debug, Clone, Copy)]
pub struct AffineScale {
    pub anchor: Vec3,
    pub scale: Vec3,
}

impl AffineScale {
    pub fn apply_point(self, p: Vec3) -> Vec3 {
        self.anchor + (p - self.anchor) * self.scale
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AffineRotate {
    pub pivot: Vec3,
    pub rot: Quat,
}

impl AffineRotate {
    pub fn from_axis_angle(pivot: Vec3, axis: Vec3, angle_rad: f32) -> Option<Self> {
        let len2 = axis.length_squared();
        if len2 <= f32::EPSILON {
            return None;
        }
        let axis_n = axis / len2.sqrt();
        Some(Self {
            pivot,
            rot: Quat::from_axis_angle(axis_n, angle_rad),
        })
    }

    pub fn apply_point(self, p: Vec3) -> Vec3 {
        self.pivot + self.rot * (p - self.pivot)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchFace {
    XMin,
    XMax,
    YMin,
    YMax,
    ZMin,
    ZMax,
}

fn apply_stretch_face_delta(aabb: &mut Aabb, face: StretchFace, delta: i32) {
    if delta == 0 {
        return;
    }
    match face {
        StretchFace::XMin => aabb.min.x = (aabb.min.x + delta).min(aabb.max.x - 1),
        StretchFace::XMax => aabb.max.x = (aabb.max.x + delta).max(aabb.min.x + 1),
        StretchFace::YMin => aabb.min.y = (aabb.min.y + delta).min(aabb.max.y - 1),
        StretchFace::YMax => aabb.max.y = (aabb.max.y + delta).max(aabb.min.y + 1),
        StretchFace::ZMin => aabb.min.z = (aabb.min.z + delta).min(aabb.max.z - 1),
        StretchFace::ZMax => aabb.max.z = (aabb.max.z + delta).max(aabb.min.z + 1),
    }
}

pub fn preview_stretched_aabb(
    selection_aabb: &Aabb,
    faces: [Option<StretchFace>; 2],
    delta: IVec3,
) -> Aabb {
    let mut out = selection_aabb.clone();
    for face in faces.iter().flatten() {
        let amt = match face {
            StretchFace::XMin | StretchFace::XMax => delta.x,
            StretchFace::YMin | StretchFace::YMax => delta.y,
            StretchFace::ZMin | StretchFace::ZMax => delta.z,
        };
        apply_stretch_face_delta(&mut out, *face, amt);
    }
    out
}

pub fn stretch_selection_transform(
    selection_aabb: &Aabb,
    faces: [Option<StretchFace>; 2],
    delta: IVec3,
) -> Option<(AffineScale, Aabb)> {
    let preview = preview_stretched_aabb(selection_aabb, faces, delta);

    let mut anchor = Vec3::ZERO;
    let mut scale = Vec3::ONE;
    let mut any = false;

    for face in faces.iter().flatten() {
        match face {
            StretchFace::XMin => {
                let old = selection_aabb.max.x - selection_aabb.min.x;
                if old == 0 {
                    return None;
                }
                let new_ = selection_aabb.max.x - preview.min.x;
                anchor.x = selection_aabb.max.x as f32;
                scale.x = new_ as f32 / old as f32;
                any = true;
            }
            StretchFace::XMax => {
                let old = selection_aabb.max.x - selection_aabb.min.x;
                if old == 0 {
                    return None;
                }
                let new_ = preview.max.x - selection_aabb.min.x;
                anchor.x = selection_aabb.min.x as f32;
                scale.x = new_ as f32 / old as f32;
                any = true;
            }
            StretchFace::YMin => {
                let old = selection_aabb.max.y - selection_aabb.min.y;
                if old == 0 {
                    return None;
                }
                let new_ = selection_aabb.max.y - preview.min.y;
                anchor.y = selection_aabb.max.y as f32;
                scale.y = new_ as f32 / old as f32;
                any = true;
            }
            StretchFace::YMax => {
                let old = selection_aabb.max.y - selection_aabb.min.y;
                if old == 0 {
                    return None;
                }
                let new_ = preview.max.y - selection_aabb.min.y;
                anchor.y = selection_aabb.min.y as f32;
                scale.y = new_ as f32 / old as f32;
                any = true;
            }
            StretchFace::ZMin => {
                let old = selection_aabb.max.z - selection_aabb.min.z;
                if old == 0 {
                    return None;
                }
                let new_ = selection_aabb.max.z - preview.min.z;
                anchor.z = selection_aabb.max.z as f32;
                scale.z = new_ as f32 / old as f32;
                any = true;
            }
            StretchFace::ZMax => {
                let old = selection_aabb.max.z - selection_aabb.min.z;
                if old == 0 {
                    return None;
                }
                let new_ = preview.max.z - selection_aabb.min.z;
                anchor.z = selection_aabb.min.z as f32;
                scale.z = new_ as f32 / old as f32;
                any = true;
            }
        }
    }

    any.then_some((AffineScale { anchor, scale }, preview))
}

fn transform_interval(lo: i32, hi: i32, anchor: f32, scale: f32) -> (i32, i32) {
    let f = |x: i32| anchor + (x as f32 - anchor) * scale;
    let a = f(lo);
    let b = f(hi);
    let min = aabb_floor_eps(a.min(b));
    let max = aabb_ceil_eps(a.max(b));
    (min, max)
}

// When brushes are grid-aligned, plane intersection can still yield slightly-off values like
// `88.00001`. Treat values within a tiny epsilon as on-grid to avoid 1-unit AABB drift.
const AABB_EPS: f32 = 1.0e-3;

fn aabb_floor_eps(v: f32) -> i32 {
    (v + AABB_EPS).floor() as i32
}

fn aabb_ceil_eps(v: f32) -> i32 {
    (v - AABB_EPS).ceil() as i32
}

pub fn apply_affine_scale_to_brush(
    brush: &mut Brush,
    generation: &mut u64,
    xform: AffineScale,
) -> bool {
    if xform.scale == Vec3::ONE {
        return false;
    }

    let old_aabb = brush.aabb.clone();

    match &mut brush.content {
        BrushContent::Convex(faces) => {
            for face in faces {
                for p in &mut face.plane_points {
                    *p = xform.apply_point(*p);
                }
            }
        }
        BrushContent::Patch(patch) => {
            for row in &mut patch.vertices {
                for v in row {
                    v.position = xform.apply_point(v.position);
                }
            }
        }
    }

    let (min_x, max_x) = transform_interval(
        old_aabb.min.x,
        old_aabb.max.x,
        xform.anchor.x,
        xform.scale.x,
    );
    let (min_y, max_y) = transform_interval(
        old_aabb.min.y,
        old_aabb.max.y,
        xform.anchor.y,
        xform.scale.y,
    );
    let (min_z, max_z) = transform_interval(
        old_aabb.min.z,
        old_aabb.max.z,
        xform.anchor.z,
        xform.scale.z,
    );
    brush.aabb.min = IVec3::new(min_x, min_y, min_z);
    brush.aabb.max = IVec3::new(max_x, max_y, max_z);

    // Bump generation and clear caches (via no-op translate).
    brush.translate(generation, IVec3::ZERO);
    true
}

pub fn rotate_selection_transform(
    selection_aabb: &Aabb,
    axis: Vec3,
    angle_rad: f32,
) -> Option<(AffineRotate, Aabb)> {
    let pivot = (selection_aabb.min.as_vec3() + selection_aabb.max.as_vec3()) * 0.5;
    let xform = AffineRotate::from_axis_angle(pivot, axis, angle_rad)?;
    let preview = preview_rotated_aabb(selection_aabb, xform);
    Some((xform, preview))
}

pub fn preview_rotated_aabb(selection_aabb: &Aabb, xform: AffineRotate) -> Aabb {
    let min = selection_aabb.min.as_vec3();
    let max = selection_aabb.max.as_vec3();
    let corners = [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ];

    let mut out_min = Vec3::splat(f32::INFINITY);
    let mut out_max = Vec3::splat(f32::NEG_INFINITY);
    for &c in &corners {
        let p = xform.apply_point(c);
        out_min = out_min.min(p);
        out_max = out_max.max(p);
    }

    Aabb {
        min: IVec3::new(
            aabb_floor_eps(out_min.x),
            aabb_floor_eps(out_min.y),
            aabb_floor_eps(out_min.z),
        ),
        max: IVec3::new(
            aabb_ceil_eps(out_max.x),
            aabb_ceil_eps(out_max.y),
            aabb_ceil_eps(out_max.z),
        ),
    }
}

pub fn apply_affine_rotate_to_brush(
    brush: &mut Brush,
    generation: &mut u64,
    xform: AffineRotate,
) -> bool {
    if xform.rot == Quat::IDENTITY {
        return false;
    }

    match &mut brush.content {
        BrushContent::Convex(faces) => {
            for face in faces {
                for p in &mut face.plane_points {
                    *p = xform.apply_point(*p);
                }
            }

            if let Ok(polys) = crate::geometry::brush_to_polygons(&*brush) {
                brush.aabb = aabb_from_polys(&polys);
            }
        }
        BrushContent::Patch(patch) => {
            let mut positions = Vec::new();
            for row in &mut patch.vertices {
                for v in row {
                    v.position = xform.apply_point(v.position);
                    positions.push(v.position);
                }
            }
            brush.aabb = aabb_from_positions(positions.as_slice());
        }
    }

    // Bump generation and clear caches (via no-op translate).
    brush.translate(generation, IVec3::ZERO);
    true
}

fn axis_of_face(face: StretchFace) -> usize {
    match face {
        StretchFace::XMin | StretchFace::XMax => 0,
        StretchFace::YMin | StretchFace::YMax => 1,
        StretchFace::ZMin | StretchFace::ZMax => 2,
    }
}

fn div_ceil_i32(a: i32, b: i32) -> i32 {
    debug_assert!(b > 0);
    -((-a).div_euclid(b))
}

fn clamp_face_delta_snapped(aabb: &Aabb, face: StretchFace, delta: i32, grid_step: i32) -> i32 {
    let step = grid_step.abs().max(1);

    // Ensure grid-step alignment even if upstream input was slightly off.
    // This matches the editor's f32 snapping (`round()`).
    let delta = if step == 1 {
        delta
    } else {
        ((delta as f32 / step as f32).round() as i32) * step
    };

    if delta == 0 {
        return 0;
    }

    match face {
        StretchFace::XMin => {
            if delta <= 0 {
                return delta;
            }
            let max_delta = (aabb.max.x - 1) - aabb.min.x;
            if delta <= max_delta {
                return delta;
            }
            let q = max_delta.div_euclid(step) * step;
            q.max(0)
        }
        StretchFace::XMax => {
            if delta >= 0 {
                return delta;
            }
            let min_delta = (aabb.min.x + 1) - aabb.max.x;
            if delta >= min_delta {
                return delta;
            }
            div_ceil_i32(min_delta, step) * step
        }
        StretchFace::YMin => {
            if delta <= 0 {
                return delta;
            }
            let max_delta = (aabb.max.y - 1) - aabb.min.y;
            if delta <= max_delta {
                return delta;
            }
            let q = max_delta.div_euclid(step) * step;
            q.max(0)
        }
        StretchFace::YMax => {
            if delta >= 0 {
                return delta;
            }
            let min_delta = (aabb.min.y + 1) - aabb.max.y;
            if delta >= min_delta {
                return delta;
            }
            div_ceil_i32(min_delta, step) * step
        }
        StretchFace::ZMin => {
            if delta <= 0 {
                return delta;
            }
            let max_delta = (aabb.max.z - 1) - aabb.min.z;
            if delta <= max_delta {
                return delta;
            }
            let q = max_delta.div_euclid(step) * step;
            q.max(0)
        }
        StretchFace::ZMax => {
            if delta >= 0 {
                return delta;
            }
            let min_delta = (aabb.min.z + 1) - aabb.max.z;
            if delta >= min_delta {
                return delta;
            }
            div_ceil_i32(min_delta, step) * step
        }
    }
}

fn faces_touching_extreme(polys: &[(Vec<Vec3>, Vec<u32>)], face: StretchFace) -> Vec<usize> {
    let axis = axis_of_face(face);
    let mut any = false;
    let mut global_min = f32::INFINITY;
    let mut global_max = f32::NEG_INFINITY;
    for (positions, _) in polys {
        for p in positions {
            any = true;
            global_min = global_min.min(p[axis]);
            global_max = global_max.max(p[axis]);
        }
    }
    if !any {
        return Vec::new();
    }

    let target = match face {
        StretchFace::XMin | StretchFace::YMin | StretchFace::ZMin => global_min,
        StretchFace::XMax | StretchFace::YMax | StretchFace::ZMax => global_max,
    };
    let eps = 1.0e-3f32;

    let mut out = Vec::new();
    for (i, (positions, _)) in polys.iter().enumerate() {
        if positions.is_empty() {
            continue;
        }
        let mut face_min = f32::INFINITY;
        let mut face_max = f32::NEG_INFINITY;
        for p in positions {
            face_min = face_min.min(p[axis]);
            face_max = face_max.max(p[axis]);
        }
        let hit = match face {
            StretchFace::XMin | StretchFace::YMin | StretchFace::ZMin => face_min <= target + eps,
            StretchFace::XMax | StretchFace::YMax | StretchFace::ZMax => face_max >= target - eps,
        };
        if hit {
            out.push(i);
        }
    }
    out
}

fn apply_plane_translation(faces: &mut [Face], indices: &[usize], dv: Vec3) {
    for &idx in indices {
        if let Some(face) = faces.get_mut(idx) {
            for p in &mut face.plane_points {
                *p += dv;
            }
        }
    }
}

pub fn preview_convex_face_stretch_polys(
    brush: &Brush,
    faces: [Option<StretchFace>; 2],
    delta: IVec3,
    grid_step: i32,
) -> Option<Vec<(Vec<Vec3>, Vec<u32>)>> {
    let BrushContent::Convex(_) = &brush.content else {
        return None;
    };

    let mut tmp = brush.clone();
    let base_polys = crate::geometry::brush_to_polygons(&tmp).ok()?;
    let mut aabb = tmp.aabb.clone();

    let BrushContent::Convex(tmp_faces) = &mut tmp.content else {
        return None;
    };

    for face in faces.iter().flatten() {
        let amt = match face {
            StretchFace::XMin | StretchFace::XMax => delta.x,
            StretchFace::YMin | StretchFace::YMax => delta.y,
            StretchFace::ZMin | StretchFace::ZMax => delta.z,
        };
        let amt = clamp_face_delta_snapped(&aabb, *face, amt, grid_step);
        if amt == 0 {
            continue;
        }

        let indices = faces_touching_extreme(&base_polys, *face);
        if indices.is_empty() {
            continue;
        }
        let dv = match face {
            StretchFace::XMin | StretchFace::XMax => Vec3::new(amt as f32, 0.0, 0.0),
            StretchFace::YMin | StretchFace::YMax => Vec3::new(0.0, amt as f32, 0.0),
            StretchFace::ZMin | StretchFace::ZMax => Vec3::new(0.0, 0.0, amt as f32),
        };
        apply_plane_translation(tmp_faces.as_mut_slice(), &indices, dv);

        apply_stretch_face_delta(&mut aabb, *face, amt);
    }

    crate::geometry::brush_to_polygons(&tmp).ok()
}

pub fn stretch_convex_brush_faces(
    brush: &mut Brush,
    generation: &mut u64,
    faces: [Option<StretchFace>; 2],
    delta: IVec3,
    grid_step: i32,
) -> bool {
    let BrushContent::Convex(_) = &brush.content else {
        return false;
    };

    let base_polys = match crate::geometry::brush_to_polygons(&*brush) {
        Ok(p) => p,
        Err(_) => return false,
    };

    let old_aabb = brush.aabb.clone();
    let old_faces = match &brush.content {
        BrushContent::Convex(f) => f.clone(),
        _ => return false,
    };

    let mut any = false;
    let mut aabb = brush.aabb.clone();

    for face in faces.iter().flatten() {
        let amt = match face {
            StretchFace::XMin | StretchFace::XMax => delta.x,
            StretchFace::YMin | StretchFace::YMax => delta.y,
            StretchFace::ZMin | StretchFace::ZMax => delta.z,
        };
        let amt = clamp_face_delta_snapped(&aabb, *face, amt, grid_step);
        if amt == 0 {
            continue;
        }

        let indices = faces_touching_extreme(&base_polys, *face);
        if indices.is_empty() {
            continue;
        }

        for idx in indices {
            let dv = match face {
                StretchFace::XMin | StretchFace::XMax => Vec3::new(amt as f32, 0.0, 0.0),
                StretchFace::YMin | StretchFace::YMax => Vec3::new(0.0, amt as f32, 0.0),
                StretchFace::ZMin | StretchFace::ZMax => Vec3::new(0.0, 0.0, amt as f32),
            };
            let old = {
                let BrushContent::Convex(faces_vec) = &brush.content else {
                    continue;
                };
                let Some(face_) = faces_vec.get(idx) else {
                    continue;
                };
                face_.plane_points
            };
            let new_plane = [old[0] + dv, old[1] + dv, old[2] + dv];
            brush.update_brush_plane(generation, idx, new_plane);
            any = true;
        }

        apply_stretch_face_delta(&mut aabb, *face, amt);
    }

    if !any {
        return false;
    }
    brush.aabb = aabb;

    if crate::geometry::brush_to_polygons(&*brush).is_ok() {
        true
    } else {
        brush.content = BrushContent::Convex(old_faces);
        brush.aabb = old_aabb;
        brush.translate(generation, IVec3::ZERO);
        false
    }
}

#[derive(Debug, Clone)]
pub struct Aabb {
    pub min: IVec3,
    pub max: IVec3,
}

pub fn aabb_from_polys(polys: &[(Vec<Vec3>, Vec<u32>)]) -> Aabb {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for (verts, _) in polys {
        for v in verts {
            min = min.min(*v);
            max = max.max(*v);
        }
    }

    Aabb {
        min: IVec3::new(
            aabb_floor_eps(min.x),
            aabb_floor_eps(min.y),
            aabb_floor_eps(min.z),
        ),
        max: IVec3::new(
            aabb_ceil_eps(max.x),
            aabb_ceil_eps(max.y),
            aabb_ceil_eps(max.z),
        ),
    }
}

pub fn aabb_from_positions(positions: &[Vec3]) -> Aabb {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for &p in positions {
        min = min.min(p);
        max = max.max(p);
    }

    Aabb {
        min: IVec3::new(
            aabb_floor_eps(min.x),
            aabb_floor_eps(min.y),
            aabb_floor_eps(min.z),
        ),
        max: IVec3::new(
            aabb_ceil_eps(max.x),
            aabb_ceil_eps(max.y),
            aabb_ceil_eps(max.z),
        ),
    }
}

impl Default for Aabb {
    fn default() -> Self {
        Self {
            min: IVec3::default(),
            max: IVec3::default(),
        }
    }
}

impl Aabb {
    pub fn from_points(min: IVec3, max: IVec3) -> Self {
        Self {
            min: min.min(max),
            max: min.max(max),
        }
    }

    pub fn intersected_by_ray(&self, ray_origin: Vec3, ray_dir: Vec3) -> bool {
        let inv_dir = 1.0 / ray_dir;
        let min = self.min.as_vec3();
        let max = self.max.as_vec3();

        let t1 = (min - ray_origin) * inv_dir;
        let t2 = (max - ray_origin) * inv_dir;

        let tmin = t1.min(t2);
        let tmax = t1.max(t2);

        let t_enter = tmin.max_element();
        let t_exit = tmax.min_element();

        t_exit >= t_enter && t_exit >= 0.0
    }
}

pub fn default_texture_params() -> TextureParams {
    TextureParams {
        shift: crate::IVec2::new(0, 0),
        rotate: 0,
        scale: crate::Vec2::new(0.25, 0.25),
        surface_flags: SurfaceFlags::Structural,
        idk: 0.0,
        value: 0,
        sample_size: 0,
    }
}

fn orient_faces_inward_toward_point(faces: &mut [Face], interior: Vec3) -> bool {
    let mut any = false;
    for face in faces {
        let p = face.plane_points;
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        if n.length_squared() < 1.0e-10 {
            continue;
        }
        // CoD Radiant convention: face normals point toward the brush interior.
        if n.dot(interior - p[0]) < 0.0 {
            face.plane_points.swap(1, 2);
            any = true;
        }
    }
    any
}

pub fn convex_brush_from_aabb(id: BrushId, aabb: Aabb, texture: impl Into<String>) -> Brush {
    let min = aabb.min.as_vec3();
    let max = aabb.max.as_vec3();
    let texture = texture.into();
    let params = default_texture_params();

    let mut faces = vec![
        Face {
            plane_points: [
                Vec3::new(max.x, min.y, min.z),
                Vec3::new(max.x, min.y, max.z),
                Vec3::new(max.x, max.y, max.z),
            ],
            texture: texture.clone(),
            params,
        },
        Face {
            plane_points: [
                Vec3::new(min.x, min.y, min.z),
                Vec3::new(min.x, max.y, max.z),
                Vec3::new(min.x, min.y, max.z),
            ],
            texture: texture.clone(),
            params,
        },
        Face {
            plane_points: [
                Vec3::new(min.x, max.y, min.z),
                Vec3::new(max.x, max.y, max.z),
                Vec3::new(min.x, max.y, max.z),
            ],
            texture: texture.clone(),
            params,
        },
        Face {
            plane_points: [
                Vec3::new(min.x, min.y, min.z),
                Vec3::new(min.x, min.y, max.z),
                Vec3::new(max.x, min.y, max.z),
            ],
            texture: texture.clone(),
            params,
        },
        Face {
            plane_points: [
                Vec3::new(min.x, min.y, max.z),
                Vec3::new(max.x, max.y, max.z),
                Vec3::new(max.x, min.y, max.z),
            ],
            texture: texture.clone(),
            params,
        },
        Face {
            plane_points: [
                Vec3::new(min.x, min.y, min.z),
                Vec3::new(max.x, min.y, min.z),
                Vec3::new(max.x, max.y, min.z),
            ],
            texture,
            params,
        },
    ];

    // Ensure the plane-point winding produces inward-pointing face normals.
    // This avoids a save-time "fixup" (and texture axis flips) for newly created brushes.
    let interior = (min + max) * 0.5;
    orient_faces_inward_toward_point(&mut faces, interior);

    let mut brush = Brush::new(id, BrushContent::Convex(faces));
    brush.aabb = aabb;
    brush
}

pub fn add_convex_brush_from_aabb(
    map: &mut Map,
    entity_index: usize,
    aabb: Aabb,
    texture: impl Into<String>,
) -> Result<BrushId, String> {
    if aabb.min == aabb.max {
        return Err("AABB has zero size".to_string());
    }

    if map.entities.is_empty() {
        map.entities.push(Entity {
            id: EntityId(0),
            classname: "worldspawn".to_string(),
            properties: Default::default(),
            brushes: vec![],
        });
    }

    let idx = entity_index.min(map.entities.len() - 1);
    let entity = &mut map.entities[idx];

    let next_id = entity
        .brushes
        .iter()
        .map(|b| b.id.0)
        .max()
        .map(|id| id.wrapping_add(1))
        .unwrap_or(0);
    let brush_id = BrushId(next_id);

    entity
        .brushes
        .push(convex_brush_from_aabb(brush_id, aabb, texture));

    map.generation = map.generation.wrapping_add(1);
    Ok(brush_id)
}

pub fn orient_convex_brush_faces_inward(brush: &mut Brush, generation: &mut u64) -> bool {
    let Some((_aabb, polys)) = brush.get_polygons_and_aabb() else {
        return false;
    };

    let mut sum = Vec3::ZERO;
    let mut count = 0usize;
    for (positions, _) in polys {
        for &p in positions {
            sum += p;
            count += 1;
        }
    }
    if count == 0 {
        return false;
    }
    let interior = sum / count as f32;

    let BrushContent::Convex(faces) = &mut brush.content else {
        return false;
    };

    let mut any = false;
    for face in faces {
        let p = face.plane_points;
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        if n.length_squared() < 1.0e-10 {
            continue;
        }
        // CoD Radiant convention: face normals point toward the brush interior.
        if n.dot(interior - p[0]) < 0.0 {
            face.plane_points.swap(1, 2);
            any = true;
        }
    }

    if any {
        // Clear caches (and bump generation) via no-op translate.
        brush.translate(generation, IVec3::ZERO);
    }

    any
}

pub fn orient_map_convex_brushes_inward(map: &mut Map) -> usize {
    let mut changed = 0usize;
    for entity in &mut map.entities {
        for brush in &mut entity.brushes {
            if orient_convex_brush_faces_inward(brush, &mut map.generation) {
                changed += 1;
            }
        }
    }
    changed
}

pub fn pick_convex_brush_by_ray(
    map: &mut Map,
    ray_origin: Vec3,
    ray_dir: Vec3,
) -> Option<(usize, usize)> {
    pick_brush_by_ray(map, ray_origin, ray_dir, PickMask::CONVEX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickMask(u8);

impl PickMask {
    pub const CONVEX: PickMask = PickMask(1 << 0);
    pub const PATCH: PickMask = PickMask(1 << 1);
    pub const ALL: PickMask = PickMask(Self::CONVEX.0 | Self::PATCH.0);

    pub fn contains(self, other: PickMask) -> bool {
        (self.0 & other.0) != 0
    }
}

pub fn pick_brush_by_ray(
    map: &mut Map,
    ray_origin: Vec3,
    ray_dir: Vec3,
    mask: PickMask,
) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize, f32)> = None;

    for (entity_index, entity) in map.entities.iter_mut().enumerate() {
        for (brush_index, brush) in entity.brushes.iter_mut().enumerate() {
            match &mut brush.content {
                BrushContent::Convex(_) => {
                    if !mask.contains(PickMask::CONVEX) {
                        continue;
                    }

                    let Some((aabb, polys)) = brush.get_polygons_and_aabb() else {
                        continue;
                    };
                    let Some((t_enter, t_exit)) = ray_aabb_intersection(
                        aabb.min.as_vec3(),
                        aabb.max.as_vec3(),
                        ray_origin,
                        ray_dir,
                    ) else {
                        continue;
                    };
                    if t_exit < 0.0 {
                        continue;
                    }

                    if let Some((_, _, best_t)) = best {
                        if t_enter > best_t {
                            continue;
                        }
                    }

                    let Some(t) = ray_polys_first_hit(polys, ray_origin, ray_dir) else {
                        continue;
                    };
                    if t >= 0.0 {
                        match best {
                            None => best = Some((entity_index, brush_index, t)),
                            Some((_, _, best_t)) if t < best_t => {
                                best = Some((entity_index, brush_index, t))
                            }
                            _ => {}
                        }
                    }
                }
                BrushContent::Patch(patch) => {
                    if !mask.contains(PickMask::PATCH) {
                        continue;
                    }

                    let Some((mesh, patch_aabb, _edges)) = patch.get_mesh_aabb_wire() else {
                        continue;
                    };
                    brush.aabb = patch_aabb.clone();

                    let Some((t_enter, t_exit)) = ray_aabb_intersection(
                        patch_aabb.min.as_vec3(),
                        patch_aabb.max.as_vec3(),
                        ray_origin,
                        ray_dir,
                    ) else {
                        continue;
                    };
                    if t_exit < 0.0 {
                        continue;
                    }
                    if let Some((_, _, best_t)) = best {
                        if t_enter > best_t {
                            continue;
                        }
                    }

                    let Some(t) = ray_patch_mesh_first_hit(mesh, ray_origin, ray_dir) else {
                        continue;
                    };
                    if t >= 0.0 {
                        match best {
                            None => best = Some((entity_index, brush_index, t)),
                            Some((_, _, best_t)) if t < best_t => {
                                best = Some((entity_index, brush_index, t))
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    best.map(|(e, b, _)| (e, b))
}

fn ray_aabb_intersection(min: Vec3, max: Vec3, origin: Vec3, dir: Vec3) -> Option<(f32, f32)> {
    let eps = 1.0e-8;
    let mut tmin = f32::NEG_INFINITY;
    let mut tmax = f32::INFINITY;

    for i in 0..3 {
        let o = origin[i];
        let d = dir[i];
        let lo = min[i];
        let hi = max[i];

        if d.abs() < eps {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }

        let inv = 1.0 / d;
        let mut t1 = (lo - o) * inv;
        let mut t2 = (hi - o) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmax < tmin {
            return None;
        }
    }

    Some((tmin, tmax))
}

fn ray_polys_first_hit(polys: &[(Vec<Vec3>, Vec<u32>)], origin: Vec3, dir: Vec3) -> Option<f32> {
    let mut best = None;
    for (positions, indices) in polys {
        if positions.len() < 3 || indices.len() < 3 {
            continue;
        }
        for tri in indices.chunks_exact(3) {
            let i0 = tri[0] as usize;
            let i1 = tri[1] as usize;
            let i2 = tri[2] as usize;
            if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
                continue;
            }
            let v0 = positions[i0];
            let v1 = positions[i1];
            let v2 = positions[i2];
            let Some(t) = ray_triangle_intersection(origin, dir, v0, v1, v2) else {
                continue;
            };
            if t >= 0.0 {
                match best {
                    None => best = Some(t),
                    Some(best_t) if t < best_t => best = Some(t),
                    _ => {}
                }
            }
        }
    }
    best
}

fn ray_patch_mesh_first_hit(
    mesh: &crate::geometry::PatchMesh,
    origin: Vec3,
    dir: Vec3,
) -> Option<f32> {
    let positions = mesh.positions.as_slice();
    let indices = mesh.indices.as_slice();
    if positions.len() < 3 || indices.len() < 3 {
        return None;
    }

    let mut best = None;
    for tri in indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }
        let v0 = positions[i0];
        let v1 = positions[i1];
        let v2 = positions[i2];
        let Some(t) = ray_triangle_intersection(origin, dir, v0, v1, v2) else {
            continue;
        };
        if t >= 0.0 {
            match best {
                None => best = Some(t),
                Some(best_t) if t < best_t => best = Some(t),
                _ => {}
            }
        }
    }
    best
}

fn ray_triangle_intersection(origin: Vec3, dir: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
    let eps = 1.0e-7;
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < eps {
        return None;
    }
    let inv_det = 1.0 / det;
    let tvec = origin - v0;
    let u = tvec.dot(p) * inv_det;
    if u < 0.0 || u > 1.0 {
        return None;
    }
    let q = tvec.cross(e1);
    let v = dir.dot(q) * inv_det;
    if v < 0.0 || (u + v) > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv_det;
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convex_brush_from_aabb_is_inward_by_default() {
        let aabb = Aabb::from_points(IVec3::new(-16, -32, 0), IVec3::new(48, 64, 128));
        let mut brush = convex_brush_from_aabb(BrushId(0), aabb.clone(), "common/caulk");

        let center = (aabb.min.as_vec3() + aabb.max.as_vec3()) * 0.5;
        let BrushContent::Convex(faces) = &brush.content else {
            panic!("expected convex brush");
        };
        for face in faces {
            let p = face.plane_points;
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            assert!(n.length_squared() > 1.0e-6);
            assert!(n.dot(center - p[0]) > 0.0);
        }

        let mut generation = 0u64;
        assert!(!orient_convex_brush_faces_inward(
            &mut brush,
            &mut generation
        ));
    }
}
