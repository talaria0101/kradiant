use rayon::iter::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};

use crate::editor::config::{EntityDef, EntityDrawAnchor, EntityDrawKind, EntityDrawStyle};
use crate::editor::selection::EdgeSelection;
use crate::map::{
    Brush, BrushContent, BrushId, Entity, EntityId, Face, Map, SurfaceFlags, TextureParams,
};
use crate::texmap::face_plane_normal;
use crate::{Quat, Vec3};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy)]
pub struct AffineScale {
    pub anchor: Vec3,
    pub scale: Vec3,
}

fn entity_pick_bounds(entity: &Entity, origin: Vec3, style: &EntityDrawStyle) -> (Vec3, Vec3) {
    if matches!(
        style.kind,
        EntityDrawKind::ModelBounds | EntityDrawKind::ModelWireframe
    ) {
        if let Some(model) = &entity.model {
            let model_origin = origin + model.origin;
            return (model_origin + model.mins, model_origin + model.maxs);
        }
    }

    // Match the drawn proxy box orientation: enclose the rotated corners so the
    // pick area tracks the box (Center spins about origin, Base about Z/2).
    let size = Vec3::from_array(style.size);
    let half_size = size * 0.5;
    let box_center = match style.anchor {
        EntityDrawAnchor::Center => origin,
        EntityDrawAnchor::Base => origin + Vec3::new(0.0, 0.0, half_size.z),
    };
    let angles = entity
        .properties
        .get("angles")
        .and_then(|s| crate::core_util::vec3_from_whitespace_triplet(s));
    let rot = angles.map(crate::core_util::entity_angles_to_quat);
    let corners = crate::core_util::oriented_box_corners(box_center, half_size, rot);
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for c in corners {
        min = min.min(c);
        max = max.max(c);
    }
    (min, max)
}

/// Resolve the pick bounds for a ray test against an entity: start from the
/// style-resolved bounds, then union the loaded model's world-space bounds so
/// the pick box covers the model regardless of the style's anchor/kind.
fn entity_ray_pick_bounds(entity: &Entity, origin: Vec3, style: &EntityDrawStyle) -> (Vec3, Vec3) {
    // Entities with a loaded model are pickable exactly within their rotated
    // bounds (the visible wireframe box), not the unrotated proxy/box position.
    if let Some(model) = &entity.model {
        let model_origin = origin + model.origin;
        let angles = entity
            .properties
            .get("angles")
            .and_then(|s| crate::core_util::vec3_from_whitespace_triplet(s));
        let rot = angles.map(crate::core_util::entity_angles_to_quat);
        return crate::core_util::model_bounds_aabb(model_origin, model.mins, model.maxs, rot);
    }
    entity_pick_bounds(entity, origin, style)
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

fn apply_stretch_face_delta(aabb: &mut Aabb, face: StretchFace, delta: f32) {
    if delta == 0.0 {
        return;
    }
    match face {
        StretchFace::XMin => aabb.min.x = (aabb.min.x + delta).min(aabb.max.x - 1.0),
        StretchFace::XMax => aabb.max.x = (aabb.max.x + delta).max(aabb.min.x + 1.0),
        StretchFace::YMin => aabb.min.y = (aabb.min.y + delta).min(aabb.max.y - 1.0),
        StretchFace::YMax => aabb.max.y = (aabb.max.y + delta).max(aabb.min.y + 1.0),
        StretchFace::ZMin => aabb.min.z = (aabb.min.z + delta).min(aabb.max.z - 1.0),
        StretchFace::ZMax => aabb.max.z = (aabb.max.z + delta).max(aabb.min.z + 1.0),
    }
}

pub fn preview_stretched_aabb(
    selection_aabb: &Aabb,
    faces: [Option<StretchFace>; 2],
    delta: Vec3,
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
    delta: Vec3,
) -> Option<(AffineScale, Aabb)> {
    let preview = preview_stretched_aabb(selection_aabb, faces, delta);

    let mut anchor = Vec3::ZERO;
    let mut scale = Vec3::ONE;
    let mut any = false;

    for face in faces.iter().flatten() {
        match face {
            StretchFace::XMin => {
                let old = selection_aabb.max.x - selection_aabb.min.x;
                if old == 0.0 {
                    return None;
                }
                let new_ = selection_aabb.max.x - preview.min.x;
                anchor.x = selection_aabb.max.x as f32;
                scale.x = new_ as f32 / old as f32;
                any = true;
            }
            StretchFace::XMax => {
                let old = selection_aabb.max.x - selection_aabb.min.x;
                if old == 0.0 {
                    return None;
                }
                let new_ = preview.max.x - selection_aabb.min.x;
                anchor.x = selection_aabb.min.x as f32;
                scale.x = new_ as f32 / old as f32;
                any = true;
            }
            StretchFace::YMin => {
                let old = selection_aabb.max.y - selection_aabb.min.y;
                if old == 0.0 {
                    return None;
                }
                let new_ = selection_aabb.max.y - preview.min.y;
                anchor.y = selection_aabb.max.y as f32;
                scale.y = new_ as f32 / old as f32;
                any = true;
            }
            StretchFace::YMax => {
                let old = selection_aabb.max.y - selection_aabb.min.y;
                if old == 0.0 {
                    return None;
                }
                let new_ = preview.max.y - selection_aabb.min.y;
                anchor.y = selection_aabb.min.y as f32;
                scale.y = new_ as f32 / old as f32;
                any = true;
            }
            StretchFace::ZMin => {
                let old = selection_aabb.max.z - selection_aabb.min.z;
                if old == 0.0 {
                    return None;
                }
                let new_ = selection_aabb.max.z - preview.min.z;
                anchor.z = selection_aabb.max.z as f32;
                scale.z = new_ as f32 / old as f32;
                any = true;
            }
            StretchFace::ZMax => {
                let old = selection_aabb.max.z - selection_aabb.min.z;
                if old == 0.0 {
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

fn transform_interval(lo: f32, hi: f32, anchor: f32, scale: f32) -> (f32, f32) {
    let f = |x: f32| anchor + (x - anchor) * scale;
    let a = f(lo);
    let b = f(hi);
    //let min = aabb_floor_eps(a.min(b));
    let min = a.min(b);
    //let max = aabb_ceil_eps(a.max(b));
    let max = a.max(b);
    (min, max)
}

// When brushes are grid-aligned, plane intersection can still yield slightly-off values like
// `88.00001`. Treat values within a tiny epsilon as on-grid to avoid 1-unit AABB drift.
// const AABB_EPS: f32 = 1.0e-3;

// fn aabb_floor_eps(v: f32) -> i32 {
//     (v + AABB_EPS).floor() as i32
// }

// fn aabb_ceil_eps(v: f32) -> i32 {
//     (v - AABB_EPS).ceil() as i32
// }

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
    brush.aabb.min = Vec3::new(min_x, min_y, min_z);
    brush.aabb.max = Vec3::new(max_x, max_y, max_z);

    // Bump generation and clear caches (via no-op translate).
    brush.translate(generation, Vec3::ZERO);
    true
}

pub fn rotate_selection_transform(
    selection_aabb: &Aabb,
    axis: Vec3,
    angle_rad: f32,
    pivot_override: Option<Vec3>,
) -> Option<(AffineRotate, Aabb)> {
    let aabb_center = (selection_aabb.min + selection_aabb.max) * 0.5;
    let pivot = pivot_override.unwrap_or(aabb_center);
    let xform = AffineRotate::from_axis_angle(pivot, axis, angle_rad)?;
    // Preview AABB always rotates around the AABB center (for selection highlight),
    // regardless of entity-specific pivot overrides.
    let preview = if pivot_override.is_some() {
        let aabb_xform = AffineRotate::from_axis_angle(aabb_center, axis, angle_rad)?;
        preview_rotated_aabb(selection_aabb, aabb_xform)
    } else {
        preview_rotated_aabb(selection_aabb, xform)
    };
    Some((xform, preview))
}

pub fn preview_rotated_aabb(selection_aabb: &Aabb, xform: AffineRotate) -> Aabb {
    let min = selection_aabb.min;
    let max = selection_aabb.max;
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
        /*min: Vec3::new(
            aabb_floor_eps(out_min.x),
            aabb_floor_eps(out_min.y),
            aabb_floor_eps(out_min.z),
        ),
        max: Vec3::new(
            aabb_ceil_eps(out_max.x),
            aabb_ceil_eps(out_max.y),
            aabb_ceil_eps(out_max.z),
        ),*/
        min: Vec3::new(out_min.x, out_min.y, out_min.z),
        max: Vec3::new(out_max.x, out_max.y, out_max.z),
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
    brush.translate(generation, Vec3::ZERO);
    true
}

fn axis_of_face(face: StretchFace) -> usize {
    match face {
        StretchFace::XMin | StretchFace::XMax => 0,
        StretchFace::YMin | StretchFace::YMax => 1,
        StretchFace::ZMin | StretchFace::ZMax => 2,
    }
}

fn div_ceil_f32(a: f32, b: f32) -> f32 {
    debug_assert!(b > 0.0);
    -((-a).div_euclid(b))
}

fn clamp_face_delta_snapped(aabb: &Aabb, face: StretchFace, delta: f32, grid_step: i32) -> f32 {
    let step = grid_step.abs().max(1) as f32;

    // Ensure grid-step alignment even if upstream input was slightly off.
    // This matches the editor's f32 snapping (`round()`).
    let delta = if step == 1.0 {
        delta
    } else {
        ((delta / step as f32).round()) * step as f32
    };

    if delta == 0.0 {
        return 0.0;
    }

    match face {
        StretchFace::XMin => {
            if delta <= 0.0 {
                return delta;
            }
            let max_delta = (aabb.max.x - 1.0) - aabb.min.x;
            if delta <= max_delta {
                return delta;
            }
            let q = max_delta.div_euclid(step) * step;
            q.max(0.0)
        }
        StretchFace::XMax => {
            if delta >= 0.0 {
                return delta;
            }
            let min_delta = (aabb.min.x + 1.0) - aabb.max.x;
            if delta >= min_delta {
                return delta;
            }
            div_ceil_f32(min_delta, step) * step
        }
        StretchFace::YMin => {
            if delta <= 0.0 {
                return delta;
            }
            let max_delta = (aabb.max.y - 1.0) - aabb.min.y;
            if delta <= max_delta {
                return delta;
            }
            let q = max_delta.div_euclid(step) * step;
            q.max(0.0)
        }
        StretchFace::YMax => {
            if delta >= 0.0 {
                return delta;
            }
            let min_delta = (aabb.min.y + 1.0) - aabb.max.y;
            if delta >= min_delta {
                return delta;
            }
            div_ceil_f32(min_delta, step) * step
        }
        StretchFace::ZMin => {
            if delta <= 0.0 {
                return delta;
            }
            let max_delta = (aabb.max.z - 1.0) - aabb.min.z;
            if delta <= max_delta {
                return delta;
            }
            let q = max_delta.div_euclid(step) * step;
            q.max(0.0)
        }
        StretchFace::ZMax => {
            if delta >= 0.0 {
                return delta;
            }
            let min_delta = (aabb.min.z + 1.0) - aabb.max.z;
            if delta >= min_delta {
                return delta;
            }
            div_ceil_f32(min_delta, step) * step
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
    delta: Vec3,
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
        if amt == 0.0 {
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
    delta: Vec3,
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
        if amt == 0.0 {
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
        brush.translate(generation, Vec3::ZERO);
        false
    }
}

/// q3radiant `Brush_SideSelect` (ported from CoD1Radiant core/brush_edit.py):
/// the mouse ray MISSED the brush — grab every face on whose OUTER side
/// the ray passes while it stays inside all the other planes (clicking
/// beside an edge grabs both adjacent faces). Returns face indices into
/// `BrushContent::Convex`.
///
/// Face planes use INWARD normals (`n·p >= dist` is the inside half-space),
/// matching `is_point_inside_convex_brush`.
pub fn side_select_faces(brush: &Brush, origin: Vec3, direction: Vec3) -> Vec<usize> {
    const RAY_LEN: f32 = 16384.0;

    let BrushContent::Convex(faces) = &brush.content else {
        return Vec::new();
    };
    let planes: Vec<(Vec3, f32)> = faces
        .iter()
        .map(|f| {
            let n = face_plane_normal(f);
            (n, n.dot(f.plane_points[0]))
        })
        .collect();

    let start = origin;
    let end = origin + direction * RAY_LEN;

    let mut selected = Vec::new();
    for (index, (normal, dist)) in planes.iter().enumerate() {
        let (mut p1, mut p2) = (start, end);
        let mut gone = false;
        for (other_index, (other_normal, other_dist)) in planes.iter().enumerate() {
            if other_index == index {
                continue;
            }
            match clip_segment_inside(p1, p2, *other_normal, *other_dist) {
                ClipResult::Inside(a, b) => {
                    p1 = a;
                    p2 = b;
                }
                ClipResult::Outside => {
                    gone = true;
                    break;
                }
            }
        }
        if gone {
            continue;
        }
        // q3: the ray start never entered the other planes
        if (p1 - start).length_squared() < 1e-12 {
            continue;
        }
        if matches!(
            clip_segment_inside(p1, p2, *normal, *dist),
            ClipResult::Outside
        ) {
            selected.push(index);
        }
    }
    selected
}

enum ClipResult {
    Inside(Vec3, Vec3),
    Outside,
}

/// Port of CoD1Radiant `brush_is_valid` (core/csg.py): a convex brush is
/// valid when its vertices — the 3-plane intersection points that lie
/// inside every half-space — number at least 4 and give it real extent
/// along all three axes. Face planes use INWARD normals
/// (`n·p >= dist` is inside).
pub fn is_convex_brush_valid(brush: &Brush) -> bool {
    let BrushContent::Convex(faces) = &brush.content else {
        return false;
    };
    if faces.len() < 4 {
        return false;
    }
    let planes: Vec<(Vec3, f32)> = faces
        .iter()
        .map(|f| {
            let n = face_plane_normal(f);
            (n, n.dot(f.plane_points[0]))
        })
        .collect();

    let mut verts: Vec<Vec3> = Vec::new();
    for i in 0..planes.len() {
        for j in (i + 1)..planes.len() {
            for k in (j + 1)..planes.len() {
                let (a, da) = planes[i];
                let (b, db) = planes[j];
                let (c, dc) = planes[k];
                let det = a.dot(b.cross(c));
                if det.abs() < 1e-9 {
                    continue;
                }
                let p = (da * b.cross(c) + db * c.cross(a) + dc * a.cross(b)) / det;
                if planes.iter().all(|(n, d)| n.dot(p) - d >= -0.05) {
                    verts.push(p);
                }
            }
        }
    }
    if verts.len() < 4 {
        return false;
    }
    for axis in 0..3 {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for v in &verts {
            min = min.min(v[axis]);
            max = max.max(v[axis]);
        }
        if max - min < 1.0 {
            return false;
        }
    }
    true
}

/// q3radiant `ClipLineToFace` ported to INWARD normals: trims the segment
/// to the plane's INNER half-space. `Outside` means the segment is fully
/// on the outside (gone).
fn clip_segment_inside(p1: Vec3, p2: Vec3, normal: Vec3, dist: f32) -> ClipResult {
    let d1 = normal.dot(p1) - dist;
    let d2 = normal.dot(p2) - dist;
    if d1 <= 0.0 && d2 <= 0.0 {
        ClipResult::Outside
    } else if d1 >= 0.0 && d2 >= 0.0 {
        ClipResult::Inside(p1, p2)
    } else {
        let fraction = d1 / (d1 - d2);
        let clipped = p1 + (p2 - p1) * fraction;
        if d1 < 0.0 {
            ClipResult::Inside(clipped, p2)
        } else {
            ClipResult::Inside(p1, clipped)
        }
    }
}

/// Move the given face planes of a convex brush by `move_v` (q3radiant
/// camera side stretch: the FULL drag vector, not just the normal
/// component). The result is validated first — false when the brush
/// would degenerate, so the caller can refuse the whole drag.
pub fn stretch_brush_side_faces(
    brush: &mut Brush,
    generation: &mut u64,
    indices: &[usize],
    move_v: Vec3,
) -> bool {
    let face_count = match &brush.content {
        BrushContent::Convex(faces) => faces.len(),
        _ => return false,
    };
    if indices.iter().any(|i| *i >= face_count) || move_v == Vec3::ZERO {
        return false;
    }

    let mut probe = brush.clone();
    let BrushContent::Convex(probe_faces) = &mut probe.content else {
        return false;
    };
    for idx in indices {
        for p in &mut probe_faces[*idx].plane_points {
            *p += move_v;
        }
    }
    if !is_convex_brush_valid(&probe) {
        return false;
    }

    for idx in indices {
        let old = {
            let BrushContent::Convex(faces) = &brush.content else {
                return false;
            };
            faces[*idx].plane_points
        };
        let new_plane = [old[0] + move_v, old[1] + move_v, old[2] + move_v];
        brush.update_brush_plane(generation, *idx, new_plane);
    }
    // Recompute the AABB from the new geometry.
    let _ = brush.get_polygons_and_aabb();
    true
}

#[derive(Debug, Clone, Copy)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

pub fn aabb_from_polys(polys: &[(Vec<Vec3>, Vec<u32>)]) -> Aabb {
    let (min, max) = polys
        .par_iter()
        .with_min_len(64)
        .map(|(verts, _)| {
            let mut mn = Vec3::splat(f32::INFINITY);
            let mut mx = Vec3::splat(f32::NEG_INFINITY);
            for v in verts {
                mn = mn.min(*v);
                mx = mx.max(*v);
            }
            (mn, mx)
        })
        .reduce(
            || (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(a_min, a_max), (b_min, b_max)| (a_min.min(b_min), a_max.max(b_max)),
        );

    Aabb {
        min: Vec3::new(min.x, min.y, min.z),
        max: Vec3::new(max.x, max.y, max.z),
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
        min: Vec3::new(min.x, min.y, min.z),
        max: Vec3::new(max.x, max.y, max.z),
    }
}

impl Default for Aabb {
    fn default() -> Self {
        Self {
            min: Vec3::default(),
            max: Vec3::default(),
        }
    }
}

impl Aabb {
    pub fn from_points(min: Vec3, max: Vec3) -> Self {
        Self {
            min: min.min(max),
            max: min.max(max),
        }
    }

    pub fn intersected_by_ray(&self, ray_origin: Vec3, ray_dir: Vec3) -> bool {
        let inv_dir = 1.0 / ray_dir;
        let min = self.min;
        let max = self.max;

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
    let min = aabb.min;
    let max = aabb.max;
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
            model: None,
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
        brush.translate(generation, Vec3::ZERO);
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
    mask: PickMask,
) -> Option<(usize, usize)> {
    pick_brush_by_ray(map, ray_origin, ray_dir, mask, None)
}

pub fn pick_convex_face_by_ray(
    map: &mut Map,
    ray_origin: Vec3,
    ray_dir: Vec3,
    mask: PickMask,
) -> Option<(usize, usize, usize)> {
    let mut best: Option<(usize, usize, usize, f32)> = None;

    let sel_clip = mask.contains(PickMask::CLIP);
    let sel_convex = mask.contains(PickMask::CONVEX);

    for (entity_index, entity) in map.entities.iter_mut().enumerate() {
        for (brush_index, brush) in entity.brushes.iter_mut().enumerate() {
            if !sel_convex {
                continue;
            }
            if brush.is_clip() && !sel_clip {
                continue;
            }
            let BrushContent::Convex(_) = &brush.content else {
                continue;
            };

            // Skip brushes that contain the camera.
            if is_point_inside_convex_brush(brush, ray_origin) {
                continue;
            }

            let Some((aabb, polys)) = brush.get_polygons_and_aabb() else {
                continue;
            };
            let Some((t_enter, t_exit)) =
                ray_aabb_intersection(aabb.min, aabb.max, ray_origin, ray_dir)
            else {
                continue;
            };
            if t_exit < 0.0 {
                continue;
            }

            if let Some((_, _, _, best_t)) = best {
                if t_enter > best_t {
                    continue;
                }
            }

            let Some((face_index, t)) = ray_polys_first_hit_with_index(polys, ray_origin, ray_dir)
            else {
                continue;
            };

            if t >= 0.0 {
                match best {
                    None => best = Some((entity_index, brush_index, face_index, t)),
                    Some((_, _, _, best_t)) if t < best_t => {
                        best = Some((entity_index, brush_index, face_index, t))
                    }
                    _ => {}
                }
            }
        }
    }

    best.map(|(e, b, f, _)| (e, b, f))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickMask(u16);

impl PickMask {
    pub const NONE: PickMask = PickMask(0);
    pub const CONVEX: PickMask = PickMask(1 << 0);
    pub const PATCH: PickMask = PickMask(1 << 1);
    pub const CLIP: PickMask = PickMask(1 << 2);
    pub const PORTAL: PickMask = PickMask(1 << 3);
    pub const HINT: PickMask = PickMask(1 << 4);
    pub const ALL: PickMask =
        PickMask(Self::CONVEX.0 | Self::PATCH.0 | Self::CLIP.0 | Self::PORTAL.0 | Self::HINT.0);

    pub fn contains(self, other: PickMask) -> bool {
        (self.0 & other.0) != 0
    }

    pub fn add(&mut self, other: PickMask) {
        self.0 |= other.0
    }
}

pub fn pick_brush_by_ray(
    map: &mut Map,
    ray_origin: Vec3,
    ray_dir: Vec3,
    mask: PickMask,
    exclude: Option<&HashSet<(usize, usize)>>,
) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize, f32)> = None;

    for (entity_index, entity) in map.entities.iter_mut().enumerate() {
        for (brush_index, brush) in entity.brushes.iter_mut().enumerate() {
            // Skip excluded brushes
            if let Some(exclude_set) = exclude {
                if exclude_set.contains(&(entity_index, brush_index)) {
                    continue;
                }
            }

            // Skip brushes that contain the camera.
            if let BrushContent::Convex(_) = &brush.content {
                if is_point_inside_convex_brush(brush, ray_origin) {
                    continue;
                }
            }

            match &mut brush.content {
                BrushContent::Convex(_) => {
                    if !mask.contains(PickMask::CONVEX) {
                        continue;
                    }
                    if brush.is_clip() && !mask.contains(PickMask::CLIP) {
                        continue;
                    }
                    if brush.is_portal() && !mask.contains(PickMask::PORTAL) {
                        continue;
                    }
                    if brush.is_hint() && !mask.contains(PickMask::HINT) {
                        continue;
                    }

                    let Some((aabb, polys)) = brush.get_polygons_and_aabb() else {
                        continue;
                    };
                    let Some((t_enter, t_exit)) =
                        ray_aabb_intersection(aabb.min, aabb.max, ray_origin, ray_dir)
                    else {
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

                    let Some((t_enter, t_exit)) =
                        ray_aabb_intersection(patch_aabb.min, patch_aabb.max, ray_origin, ray_dir)
                    else {
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

/// Pick an edge by ray casting in 3D space.
/// Returns the closest edge (as face pair indices) that the ray intersects.
pub fn pick_edge_by_ray(
    map: &mut Map,
    ray_origin: Vec3,
    ray_dir: Vec3,
    mask: PickMask,
    exclude: Option<&HashSet<(usize, usize)>>,
) -> Option<(usize, usize, usize, usize)> {
    let mut best: Option<(usize, usize, usize, usize, f32)> = None;
    const EDGE_PICK_THRESHOLD: f32 = 4.0; // Distance threshold for edge picking in world units

    for (entity_index, entity) in map.entities.iter_mut().enumerate() {
        for (brush_index, brush) in entity.brushes.iter_mut().enumerate() {
            // Skip excluded brushes
            if let Some(exclude_set) = exclude {
                if exclude_set.contains(&(entity_index, brush_index)) {
                    continue;
                }
            }

            // Skip brushes that contain the camera.
            if let BrushContent::Convex(_) = &brush.content {
                if is_point_inside_convex_brush(brush, ray_origin) {
                    continue;
                }
            }

            match &mut brush.content {
                BrushContent::Convex(_) => {
                    if !mask.contains(PickMask::CONVEX) {
                        continue;
                    }
                    if brush.is_clip() && !mask.contains(PickMask::CLIP) {
                        continue;
                    }

                    let Some((aabb, polys)) = brush.get_polygons_and_aabb() else {
                        continue;
                    };

                    // First check AABB intersection
                    let Some((_t_enter, t_exit)) =
                        ray_aabb_intersection(aabb.min, aabb.max, ray_origin, ray_dir)
                    else {
                        continue;
                    };
                    if t_exit < 0.0 {
                        continue;
                    }

                    // Find all edges in this brush
                    for face_a_idx in 0..polys.len() {
                        for face_b_idx in face_a_idx + 1..polys.len() {
                            if let Some((edge_start, edge_end)) =
                                crate::core_util::shared_edge_points(polys, face_a_idx, face_b_idx)
                            {
                                let edge_dir = edge_end - edge_start;
                                let edge_len = edge_dir.length();
                                if edge_len < 1e-6 {
                                    continue;
                                }
                                let edge_dir = edge_dir / edge_len;

                                // Closest distance between ray and finite segment
                                // Ray: R(t) = ray_origin + t * ray_dir, t >= 0
                                // Segment: S(s) = edge_start + s * edge_dir, 0 <= s <= edge_len
                                let w0 = edge_start - ray_origin;
                                let b = ray_dir.dot(edge_dir); // cos(theta)
                                let d = ray_dir.dot(w0);
                                let e = edge_dir.dot(w0);
                                let denom = 1.0 - b * b; // 1 - cos^2 = sin^2

                                let (mut t_ray, mut s_edge) = if denom > 1e-12 {
                                    // Non-parallel lines
                                    let t = (b * e - d) / denom;
                                    let s = (e - b * d) / denom;
                                    (t, s)
                                } else {
                                    // Parallel lines: closest point on segment to ray
                                    let s = (-e).clamp(0.0, edge_len);
                                    let closest_on_seg = edge_start + edge_dir * s;
                                    let t = ray_dir.dot(closest_on_seg - ray_origin);
                                    (t, s)
                                };

                                // Clamp parameters
                                t_ray = t_ray.max(0.0);
                                s_edge = s_edge.clamp(0.0, edge_len);

                                // If segment parameter was clamped to endpoint, recompute optimal ray parameter
                                let clamped_at_start = s_edge == 0.0 && e < 0.0;
                                let clamped_at_end = s_edge == edge_len && e > edge_len;
                                if clamped_at_start || clamped_at_end {
                                    let closest_on_seg = edge_start + edge_dir * s_edge;
                                    t_ray = ray_dir.dot(closest_on_seg - ray_origin).max(0.0);
                                }

                                let closest_on_ray = ray_origin + ray_dir * t_ray;
                                let closest_on_seg = edge_start + edge_dir * s_edge;
                                let dist = (closest_on_ray - closest_on_seg).length();

                                if dist > EDGE_PICK_THRESHOLD {
                                    continue;
                                }

                                // Check if this is the closest edge so far
                                match best {
                                    None => {
                                        best = Some((
                                            entity_index,
                                            brush_index,
                                            face_a_idx,
                                            face_b_idx,
                                            dist,
                                        ))
                                    }
                                    Some((_, _, _, _, best_dist)) if dist < best_dist => {
                                        best = Some((
                                            entity_index,
                                            brush_index,
                                            face_a_idx,
                                            face_b_idx,
                                            dist,
                                        ))
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                BrushContent::Patch(_) => {
                    // Patches don't have traditional edges, skip for now
                    continue;
                }
            }
        }
    }

    best.map(|(e, b, fa, fb, _)| (e, b, fa, fb))
}

pub fn pick_ent_by_ray(
    map: &mut Map,
    ray_origin: Vec3,
    ray_dir: Vec3,
    config: &crate::editor::config::EntityDrawingConfig,
    exclude: Option<&HashSet<usize>>,
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;

    for (entity_index, entity) in map.entities.iter_mut().enumerate() {
        if entity_index == 0 {
            continue; // Skip worldspawn
        }
        if let Some(exclude_set) = exclude {
            if exclude_set.contains(&entity_index) {
                continue;
            }
        }

        let origin_str = entity.properties.get("origin");
        let origin = if let Some(s) = origin_str {
            crate::core_util::origin_to_vec3(s)
        } else {
            continue; // Need an origin to be picked this way
        };

        let classname = &entity.classname;
        let has_model = entity.model.is_some();
        let style = config.resolve(classname, has_model);

        if matches!(style.kind, EntityDrawKind::Hidden) {
            continue;
        }

        let (min, max) = entity_ray_pick_bounds(entity, origin, &style);

        if let Some((t_enter, t_exit)) = ray_aabb_intersection(min, max, ray_origin, ray_dir) {
            if t_exit >= 0.0 {
                let t = if t_enter >= 0.0 { t_enter } else { 0.0 };
                match best {
                    None => best = Some((entity_index, t)),
                    Some((_, best_t)) if t < best_t => best = Some((entity_index, t)),
                    _ => {}
                }
            }
        }
    }

    best.map(|(e, _)| e)
}

/// Unified picking function that picks either a brush or entity, whichever is closer to the ray origin.
/// Returns Some(Ok((entity_idx, brush_idx))) for brush hit, Some(Err(entity_idx)) for entity hit, or None for no hit.
pub fn pick_brush_or_ent_by_ray(
    map: &mut Map,
    ray_origin: Vec3,
    ray_dir: Vec3,
    mask: PickMask,
    config: &crate::editor::config::EntityDrawingConfig,
    exclude_brushes: Option<&HashSet<(usize, usize)>>,
    exclude_entities: Option<&HashSet<usize>>,
) -> Option<Result<(usize, usize), usize>> {
    let mut best_brush: Option<(usize, usize, f32)> = None;
    let mut best_entity: Option<(usize, f32)> = None;

    // Pick brushes
    for (entity_index, entity) in map.entities.iter_mut().enumerate() {
        for (brush_index, brush) in entity.brushes.iter_mut().enumerate() {
            if let Some(exclude_set) = exclude_brushes {
                if exclude_set.contains(&(entity_index, brush_index)) {
                    continue;
                }
            }

            // Skip brushes that contain the camera — they would block
            // picking of brushes behind them (the ray hits their backfaces
            // first, making them the closest hit).
            if let BrushContent::Convex(_) = &brush.content {
                if is_point_inside_convex_brush(brush, ray_origin) {
                    continue;
                }
            }

            match &mut brush.content {
                BrushContent::Convex(_) => {
                    if !mask.contains(PickMask::CONVEX) {
                        continue;
                    }
                    if brush.is_clip() && !mask.contains(PickMask::CLIP) {
                        continue;
                    }

                    let Some((aabb, polys)) = brush.get_polygons_and_aabb() else {
                        continue;
                    };
                    let Some((t_enter, t_exit)) =
                        ray_aabb_intersection(aabb.min, aabb.max, ray_origin, ray_dir)
                    else {
                        continue;
                    };
                    if t_exit < 0.0 {
                        continue;
                    }

                    if let Some((_, _, best_t)) = best_brush {
                        if t_enter > best_t {
                            continue;
                        }
                    }

                    let Some(t) = ray_polys_first_hit(polys, ray_origin, ray_dir) else {
                        continue;
                    };
                    if t >= 0.0 {
                        match best_brush {
                            None => best_brush = Some((entity_index, brush_index, t)),
                            Some((_, _, best_t)) if t < best_t => {
                                best_brush = Some((entity_index, brush_index, t))
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

                    let Some((t_enter, t_exit)) =
                        ray_aabb_intersection(patch_aabb.min, patch_aabb.max, ray_origin, ray_dir)
                    else {
                        continue;
                    };
                    if t_exit < 0.0 {
                        continue;
                    }
                    if let Some((_, _, best_t)) = best_brush {
                        if t_enter > best_t {
                            continue;
                        }
                    }

                    let Some(t) = ray_patch_mesh_first_hit(mesh, ray_origin, ray_dir) else {
                        continue;
                    };
                    if t >= 0.0 {
                        match best_brush {
                            None => best_brush = Some((entity_index, brush_index, t)),
                            Some((_, _, best_t)) if t < best_t => {
                                best_brush = Some((entity_index, brush_index, t))
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // Pick entities
    for (entity_index, entity) in map.entities.iter_mut().enumerate() {
        if entity_index == 0 {
            continue; // Skip worldspawn
        }
        if let Some(exclude_set) = exclude_entities {
            if exclude_set.contains(&entity_index) {
                continue;
            }
        }

        let origin_str = entity.properties.get("origin");
        let origin = if let Some(s) = origin_str {
            crate::core_util::origin_to_vec3(s)
        } else {
            continue; // Need an origin to be picked this way
        };

        let classname = &entity.classname;
        let has_model = entity.model.is_some();
        let style = config.resolve(classname, has_model);

        if matches!(style.kind, EntityDrawKind::Hidden) {
            continue;
        }

        let (min, max) = entity_ray_pick_bounds(entity, origin, &style);

        if let Some((t_enter, t_exit)) = ray_aabb_intersection(min, max, ray_origin, ray_dir) {
            if t_exit >= 0.0 {
                let t = if t_enter >= 0.0 { t_enter } else { 0.0 };
                match best_entity {
                    None => best_entity = Some((entity_index, t)),
                    Some((_, best_t)) if t < best_t => best_entity = Some((entity_index, t)),
                    _ => {}
                }
            }
        }
    }

    // Return the closest hit
    match (best_brush, best_entity) {
        (Some((ei, bi, t_brush)), Some((ei_ent, t_ent))) => {
            if t_brush < t_ent {
                Some(Ok((ei, bi)))
            } else {
                Some(Err(ei_ent))
            }
        }
        (Some((ei, bi, _)), None) => Some(Ok((ei, bi))),
        (None, Some((ei, _))) => Some(Err(ei)),
        (None, None) => None,
    }
}

/// Normalized (face_a < face_b) face pair for an edge selection.
fn normalized_edge_pair(sel: &EdgeSelection) -> (usize, usize) {
    (
        sel.face_a_idx.min(sel.face_b_idx),
        sel.face_a_idx.max(sel.face_b_idx),
    )
}

/// Group edge selections per brush: (entity, brush) -> deduped normalized
/// face pairs.
fn group_edges_by_brush(
    selected_edges: &[EdgeSelection],
) -> std::collections::BTreeMap<(usize, usize), Vec<(usize, usize)>> {
    let mut grouped: std::collections::BTreeMap<(usize, usize), Vec<(usize, usize)>> =
        std::collections::BTreeMap::new();
    for sel in selected_edges {
        let pair = normalized_edge_pair(sel);
        let pairs = grouped.entry((sel.entity_idx, sel.brush_idx)).or_default();
        if !pairs.contains(&pair) {
            pairs.push(pair);
        }
    }
    grouped
}

/// Compute the plane updates needed to move the edges shared by the given
/// face pairs (indices into `polys`/`faces`, which correspond 1:1) by
/// `delta` — Blender-style edge dragging for plane-defined convex brushes.
///
/// Every OTHER face of the brush keeps its plane; the two faces adjacent to
/// a moved edge are re-fit through the moved edge plus one of their
/// stationary polygon vertices, so their planes tilt to follow the edge
/// while the rest of the geometry stays glued in place.
///
/// Returns `(face index, new plane points)` per updated face, or `None` when
/// any touched face cannot be re-fit (degenerate/collinear result) — the
/// caller must then refuse the whole move.
pub fn edge_move_plane_updates(
    faces: &[Face],
    polys: &[(Vec<Vec3>, Vec<u32>)],
    edge_face_pairs: &[(usize, usize)],
    delta: Vec3,
) -> Option<Vec<(usize, [Vec3; 3])>> {
    if delta == Vec3::ZERO || edge_face_pairs.is_empty() {
        return None;
    }

    // Pre-compute all UN-shifted edge endpoints per face.  When multiple
    // selected edges touch the same face, the reference vertex must avoid
    // every endpoint that will be translated—not just the current edge's—so
    // the anchor is truly stationary across every moved edge.
    let mut edge_endpoints_per_face: Vec<(usize, Vec<Vec3>)> = Vec::new();
    for &(face_a_idx, face_b_idx) in edge_face_pairs {
        let Some((edge_start, edge_end)) =
            crate::core_util::shared_edge_points(polys, face_a_idx, face_b_idx)
        else {
            return None;
        };
        for face_idx in [face_a_idx, face_b_idx] {
            if let Some(entry) = edge_endpoints_per_face
                .iter_mut()
                .find(|(i, _)| *i == face_idx)
            {
                if !entry
                    .1
                    .iter()
                    .any(|p| crate::core_util::same_point(*p, edge_start))
                {
                    entry.1.push(edge_start);
                }
                if !entry
                    .1
                    .iter()
                    .any(|p| crate::core_util::same_point(*p, edge_end))
                {
                    entry.1.push(edge_end);
                }
            } else {
                edge_endpoints_per_face.push((face_idx, vec![edge_start, edge_end]));
            }
        }
    }

    let mut updates: Vec<(usize, [Vec3; 3])> = Vec::new();
    let mut updated_faces: Vec<usize> = Vec::new();

    for &(face_a_idx, face_b_idx) in edge_face_pairs {
        let Some((edge_start, edge_end)) =
            crate::core_util::shared_edge_points(polys, face_a_idx, face_b_idx)
        else {
            return None;
        };

        for face_idx in [face_a_idx, face_b_idx] {
            if updated_faces.contains(&face_idx) {
                continue;
            }
            let Some(face) = faces.get(face_idx) else {
                return None;
            };
            let verts = &polys.get(face_idx)?.0;

            let face_endpoints = edge_endpoints_per_face
                .iter()
                .find(|(i, _)| *i == face_idx)
                .map(|(_, v)| v.as_slice())
                .unwrap_or(&[]);

            let Some(&reference) = verts.iter().find(|&&v| {
                !face_endpoints
                    .iter()
                    .any(|ep| crate::core_util::same_point(v, *ep))
            }) else {
                return None;
            };

            let old_normal = face_plane_normal(face);
            if old_normal.length_squared() < 1.0e-12 {
                return None;
            }

            let (mut p0, mut p1) = (edge_start + delta, edge_end + delta);
            let new_normal = (p1 - p0).cross(reference - p0);
            if new_normal.length_squared() < 1.0e-12 {
                return None;
            }
            if old_normal.dot(new_normal) < 0.0 {
                std::mem::swap(&mut p0, &mut p1);
            }

            // Coplanarity: every other moved endpoint on this face must land
            // on the plane we just fitted through (p0, p1, reference).  When
            // two selected edges share a face, the second edge's translated
            // endpoints may fall off the plane – the move is degenerate.
            let plane_n = (p1 - p0).cross(reference - p0);
            let plane_d = plane_n.dot(p0);
            let plane_len = plane_n.length();
            for &ep in face_endpoints {
                let shifted = ep + delta;
                let dist = (plane_n.dot(shifted) - plane_d).abs() / plane_len;
                // Reject if the endpoint is off the plane by more than a tiny
                // fraction of the delta magnitude.  Using an absolute floor
                // catches degenerate near-zero deltas too.
                let tol = delta.length().max(1.0) * 1.0e-6;
                if dist > tol {
                    return None;
                }
            }

            updates.push((face_idx, [p0, p1, reference]));
            updated_faces.push(face_idx);
        }
    }

    if updates.is_empty() {
        None
    } else {
        Some(updates)
    }
}

/// Recompute the brush polygons as they would look after moving the given
/// edges (face pairs within this brush) by `delta`, without mutating the
/// brush. Returns `None` when the move would degenerate the brush (faces
/// clipped away, fewer than 4 vertices, no extent on some axis) so callers
/// can refuse the drag.
pub fn preview_edge_moved_polys(
    brush: &Brush,
    edge_face_pairs: &[(usize, usize)],
    delta: Vec3,
) -> Option<Vec<(Vec<Vec3>, Vec<u32>)>> {
    let faces = match &brush.content {
        BrushContent::Convex(faces) => faces,
        BrushContent::Patch(_) => return None,
    };

    let polys = crate::geometry::brush_to_polygons(brush).ok()?;
    let updates = edge_move_plane_updates(faces, &polys, edge_face_pairs, delta)?;

    let mut probe = brush.clone();
    if let BrushContent::Convex(probe_faces) = &mut probe.content {
        for (face_idx, plane_points) in updates {
            probe_faces[face_idx].plane_points = plane_points;
        }
    }
    probe.invalidate_geometry();

    if !is_convex_brush_valid(&probe) {
        return None;
    }
    let new_polys = crate::geometry::brush_to_polygons(&probe).ok()?;
    // Every plane of a valid convex brush still contributes a face.
    if new_polys.len() != faces.len() || new_polys.iter().any(|(w, _)| w.len() < 3) {
        return None;
    }
    Some(new_polys)
}

/// Largest fraction `s` (0..=1) of `delta` with which every affected brush
/// can be edge-moved while staying valid AND non-explosive: no resulting
/// vertex may escape the brush's original AABB dilated by the move distance
/// plus a small margin. Without this, dragging an edge until an adjacent
/// face tilts through near-parallelism with another plane shoots the brush
/// vertices off towards infinity.
///
/// Uses bisection, assuming validity degrades monotonically along the drag
/// (true for the practical degeneration modes).
pub fn edge_move_clamp_factor(map: &Map, selected_edges: &[EdgeSelection], delta: Vec3) -> f32 {
    if selected_edges.is_empty() || delta == Vec3::ZERO {
        return 1.0;
    }

    fn brush_can_move(brush: &Brush, pairs: &[(usize, usize)], delta: Vec3) -> bool {
        if delta == Vec3::ZERO {
            return true;
        }
        let Some(polys) = preview_edge_moved_polys(brush, pairs, delta) else {
            return false;
        };
        // Explosion guard: moving an edge by `delta` may never push geometry
        // further than `|delta|` (+ margin) outside the original AABB.
        let min = brush.aabb.min - delta.abs() - Vec3::splat(1.0);
        let max = brush.aabb.max + delta.abs() + Vec3::splat(1.0);
        polys.iter().all(|(verts, _)| {
            verts.iter().all(|v| {
                v.x >= min.x
                    && v.y >= min.y
                    && v.z >= min.z
                    && v.x <= max.x
                    && v.y <= max.y
                    && v.z <= max.z
            })
        })
    }

    let grouped = group_edges_by_brush(selected_edges);
    let mut factor = 1.0f32;
    for ((entity_idx, brush_idx), pairs) in &grouped {
        let Some(entity) = map.entities.get(*entity_idx) else {
            return 0.0;
        };
        let Some(brush) = entity.brushes.get(*brush_idx) else {
            return 0.0;
        };
        if !brush_can_move(brush, pairs, delta) {
            // Bisect the largest valid fraction in [0, 1).
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..12 {
                let mid = (lo + hi) * 0.5;
                if brush_can_move(brush, pairs, delta * mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            factor = factor.min(lo);
            if factor <= 1.0e-4 {
                return 0.0;
            }
        }
    }
    factor
}

/// Move the selected edges (Blender-style edge drag) by `delta`.
///
/// Only the faces adjacent to a moved edge change: their planes are re-fit
/// through the moved edge and one stationary vertex, tilting to follow the
/// edge. All other faces keep their exact planes. The whole move is atomic —
/// if any affected brush cannot move at all without degenerating, nothing is
/// modified and `false` is returned. Otherwise the move is clamped to the
/// largest fraction of `delta` every affected brush tolerates (see
/// [`edge_move_clamp_factor`]), so a drag can squash a brush but never
/// stretch it out towards infinity.
pub fn translate_selected_edges(
    map: &mut Map,
    selected_edges: &[EdgeSelection],
    delta: Vec3,
) -> bool {
    if selected_edges.is_empty() || delta == Vec3::ZERO {
        return false;
    }

    let factor = edge_move_clamp_factor(map, selected_edges, delta);
    if factor <= 1.0e-4 {
        return false;
    }
    let delta = delta * factor;

    let grouped = group_edges_by_brush(selected_edges);

    let mut any = false;
    // grouped.into_par_iter().for_each(|((entity_idx, brush_idx), pairs)| {});
    for ((entity_idx, brush_idx), pairs) in grouped {
        let Some(entity) = map.entities.get_mut(entity_idx) else {
            continue;
        };
        let Some(brush) = entity.brushes.get_mut(brush_idx) else {
            continue;
        };

        // Plane updates come from the PRE-move polygons (the delta is added
        // internally); the clamp factor already proved this converges, but
        // apply to a clone so a failure cannot leave the brush half-updated.
        let updates = {
            let BrushContent::Convex(faces) = &brush.content else {
                continue;
            };
            let Ok(polys) = crate::geometry::brush_to_polygons(brush) else {
                continue;
            };
            edge_move_plane_updates(faces, &polys, &pairs, delta)
        };
        let Some(updates) = updates else {
            continue;
        };

        let mut tmp = brush.clone();
        for (face_idx, plane_points) in updates {
            tmp.update_brush_plane(&mut 0u64, face_idx, plane_points);
        }
        // Recompute the AABB (and the polygon cache) from the new geometry.
        let _ = tmp.get_polygons_and_aabb();
        *brush = tmp;

        map.generation = map.generation.wrapping_add(1);
        any = true;
    }

    any
}

/// Find the shared edge points between two faces in a convex brush.
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

fn ray_polys_first_hit_with_index(
    polys: &[(Vec<Vec3>, Vec<u32>)],
    origin: Vec3,
    dir: Vec3,
) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;

    for (poly_index, (positions, indices)) in polys.iter().enumerate() {
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
                    None => best = Some((poly_index, t)),
                    Some((_, best_t)) if t < best_t => best = Some((poly_index, t)),
                    _ => {}
                }
            }
        }
    }

    best
}

fn ray_polys_first_hit(polys: &[(Vec<Vec3>, Vec<u32>)], origin: Vec3, dir: Vec3) -> Option<f32> {
    ray_polys_first_hit_with_index(polys, origin, dir).map(|(_, t)| t)
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

/// Find brushes that touch (adjacent or overlapping) the target brush.
/// Uses epsilon to handle floating point precision.
pub fn find_touching_brushes(
    map: &mut Map,
    target_entity: usize,
    target_brush: usize,
    eps: f32,
) -> Vec<(usize, usize)> {
    let mut touching = Vec::new();

    // Get target brush AABB and polygons
    let (target_aabb, target_polys) = {
        let brush = &mut map.entities[target_entity].brushes[target_brush];
        match brush.get_polygons_and_aabb() {
            Some((aabb, polys)) => (aabb.clone(), polys.to_vec()),
            None => return touching,
        }
    };

    for (e_idx, entity) in map.entities.iter_mut().enumerate() {
        for (b_idx, brush) in entity.brushes.iter_mut().enumerate() {
            if e_idx == target_entity && b_idx == target_brush {
                continue;
            }

            // Get AABB and polygons (computes if not cached)
            let Some((other_aabb, other_polys)) = brush.get_polygons_and_aabb() else {
                continue;
            };

            // AABB proximity first
            if !aabb_within_distance(&target_aabb, other_aabb, eps) {
                continue;
            }

            // Detailed geometry check
            if brush_surfaces_within_distance(&target_polys, other_polys, eps) {
                touching.push((e_idx, b_idx));
            }
        }
    }
    touching
}

/// Check if AABBs are within distance eps of each other (touch or nearly touch)
fn aabb_within_distance(a: &Aabb, b: &Aabb, eps: f32) -> bool {
    a.min.x <= b.max.x + eps
        && a.max.x + eps >= b.min.x
        && a.min.y <= b.max.y + eps
        && a.max.y + eps >= b.min.y
        && a.min.z <= b.max.z + eps
        && a.max.z + eps >= b.min.z
}

/// Check if two brush polygon surfaces are within epsilon distance of each other.
/// This detects touching/adjacent brushes by checking vertex-to-triangle distances.
fn brush_surfaces_within_distance(
    polys_a: &[(Vec<Vec3>, Vec<u32>)],
    polys_b: &[(Vec<Vec3>, Vec<u32>)],
    eps: f32,
) -> bool {
    let eps_sq = eps * eps;

    // Collect all vertices and triangles from both brushes
    let mut verts_a: Vec<Vec3> = Vec::new();
    let mut tris_a: Vec<(Vec3, Vec3, Vec3)> = Vec::new();
    for (verts, indices) in polys_a {
        verts_a.extend(verts);
        for tri in indices.chunks_exact(3) {
            tris_a.push((
                verts[tri[0] as usize],
                verts[tri[1] as usize],
                verts[tri[2] as usize],
            ));
        }
    }

    let mut verts_b: Vec<Vec3> = Vec::new();
    let mut tris_b: Vec<(Vec3, Vec3, Vec3)> = Vec::new();
    for (verts, indices) in polys_b {
        verts_b.extend(verts);
        for tri in indices.chunks_exact(3) {
            tris_b.push((
                verts[tri[0] as usize],
                verts[tri[1] as usize],
                verts[tri[2] as usize],
            ));
        }
    }

    // Check if any vertex of A is close to any triangle of B
    for va in &verts_a {
        for (v0, v1, v2) in &tris_b {
            if point_triangle_distance_sq(*va, *v0, *v1, *v2) <= eps_sq {
                return true;
            }
        }
    }

    // Check if any vertex of B is close to any triangle of A
    for vb in &verts_b {
        for (v0, v1, v2) in &tris_a {
            if point_triangle_distance_sq(*vb, *v0, *v1, *v2) <= eps_sq {
                return true;
            }
        }
    }

    false
}

/// Compute squared distance from point p to triangle (v0, v1, v2).
fn point_triangle_distance_sq(p: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> f32 {
    // Compute triangle normal and plane distance
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let n = e1.cross(e2);
    let n_len_sq = n.length_squared();

    if n_len_sq < 1e-12 {
        // Degenerate triangle
        return f32::MAX;
    }

    // Project point onto triangle plane
    let n = n / n_len_sq.sqrt();
    let plane_dist = (p - v0).dot(n);
    let p_proj = p - n * plane_dist;

    // Check if projected point is inside triangle using barycentric coordinates
    let w = p_proj - v0;

    let dot00 = e2.dot(e2);
    let dot01 = e2.dot(e1);
    let dot02 = e2.dot(w);
    let dot11 = e1.dot(e1);
    let dot12 = e1.dot(w);

    let inv_denom = 1.0 / (dot00 * dot11 - dot01 * dot01);
    let u = (dot11 * dot02 - dot01 * dot12) * inv_denom;
    let v = (dot00 * dot12 - dot01 * dot02) * inv_denom;

    if u >= 0.0 && v >= 0.0 && (u + v) <= 1.0 {
        // Point projects inside triangle - distance is just plane distance
        return plane_dist * plane_dist;
    }

    // Point is outside triangle - find closest point on triangle edges
    let d0 = point_segment_distance_sq(p, v0, v1);
    let d1 = point_segment_distance_sq(p, v1, v2);
    let d2 = point_segment_distance_sq(p, v2, v0);

    d0.min(d1).min(d2)
}

/// Compute squared distance from point p to line segment (a, b).
fn point_segment_distance_sq(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let ab_len_sq = ab.length_squared();

    if ab_len_sq < 1e-12 {
        // Degenerate segment
        return ap.length_squared();
    }

    let t = ap.dot(ab) / ab_len_sq;
    let t = t.clamp(0.0, 1.0);
    let closest = a + ab * t;
    (p - closest).length_squared()
}

/// Find brushes that are completely inside the target brush's volume.
/// Uses AABB for early-out, then checks if all vertices are inside all target faces.
pub fn find_inside_brushes(
    map: &mut Map,
    target_entity: usize,
    target_brush: usize,
) -> Vec<(usize, usize)> {
    // Get target brush data
    let (target_aabb, target_planes) = {
        let brush = &map.entities[target_entity].brushes[target_brush];

        // Compute plane equations from faces (for Convex brushes)
        let planes: Vec<(Vec3, f32)> = match &brush.content {
            BrushContent::Convex(faces) => faces
                .iter()
                .map(|f| {
                    let n = face_plane_normal(f);
                    let d = -n.dot(f.plane_points[0]);
                    (n, d)
                })
                .collect(),
            BrushContent::Patch(_) => return Vec::new(), // Skip patches
        };

        // Now get AABB (needs mutable borrow for caching)
        let brush_mut = &mut map.entities[target_entity].brushes[target_brush];
        let Some((aabb, _)) = brush_mut.get_polygons_and_aabb() else {
            return Vec::new();
        };

        (aabb.clone(), planes)
    };

    let mut inside = Vec::new();

    for (entity_idx, entity) in map.entities.iter_mut().enumerate() {
        for (brush_idx, brush) in entity.brushes.iter_mut().enumerate() {
            if entity_idx == target_entity && brush_idx == target_brush {
                continue;
            }

            // AABB early-out: must be inside target AABB
            if !(brush.aabb.min.x >= target_aabb.min.x
                && brush.aabb.max.x <= target_aabb.max.x
                && brush.aabb.min.y >= target_aabb.min.y
                && brush.aabb.max.y <= target_aabb.max.y
                && brush.aabb.min.z >= target_aabb.min.z
                && brush.aabb.max.z <= target_aabb.max.z)
            {
                continue;
            }

            // Get candidate brush vertices
            let Some((_, other_polys)) = brush.get_polygons_and_aabb() else {
                continue;
            };

            // Collect all vertices from this brush
            let mut verts = Vec::new();
            for (v, _) in other_polys {
                verts.extend(v.iter().copied());
            }

            // Check if ALL vertices are inside ALL target planes
            let all_inside = verts.iter().all(|v| {
                target_planes.iter().all(|(n, d)| {
                    n.dot(*v) + d >= -0.001 // epsilon for floating point
                })
            });

            if all_inside {
                inside.push((entity_idx, brush_idx));
            }
        }
    }

    inside
}

/// Check if a point is inside a convex brush.
/// Returns true if the point is on the inside side of all face planes.
pub fn is_point_inside_convex_brush(brush: &Brush, point: Vec3) -> bool {
    let planes: Vec<(Vec3, f32)> = match &brush.content {
        BrushContent::Convex(faces) => faces
            .iter()
            .map(|f| {
                let n = face_plane_normal(f);
                let d = -n.dot(f.plane_points[0]);
                (n, d)
            })
            .collect(),
        BrushContent::Patch(_) => return false,
    };

    planes.iter().all(|(n, d)| {
        n.dot(point) + d >= -0.001 // epsilon for floating point
    })
}

pub fn add_entity(def: &EntityDef, loc: Vec3, map: &mut Map) {
    let mut properties = HashMap::new();
    properties.insert(
        "origin".to_string(),
        format!("{} {} {}", loc[0], loc[1], loc[2]),
    );
    // properties.insert("classname".to_string(), def.class.clone());
    for prop in &def.props {
        properties.insert(prop.0.clone(), prop.1.clone());
    }
    let ent = Entity {
        id: EntityId(map.entities.len() as u32),
        classname: def.class.clone(),
        properties,
        brushes: vec![],
        model: None,
    };

    map.entities.push(ent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_map_string;

    #[test]
    fn convex_brush_from_aabb_is_inward_by_default() {
        let aabb = Aabb::from_points(Vec3::new(-16.0, -32.0, 0.0), Vec3::new(48.0, 64.0, 128.0));
        let mut brush = convex_brush_from_aabb(BrushId(0), aabb.clone(), "common/caulk");

        let center = (aabb.min + aabb.max) * 0.5;
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

    fn box_brush() -> Brush {
        convex_brush_from_aabb(
            BrushId(0),
            Aabb::from_points(Vec3::splat(-16.0), Vec3::splat(16.0)),
            "common/caulk",
        )
    }

    /// Find the face index whose inward normal is `normal`.
    fn face_index_with_normal(brush: &Brush, normal: Vec3) -> usize {
        let BrushContent::Convex(faces) = &brush.content else {
            panic!("expected convex brush");
        };
        faces
            .iter()
            .position(|f| {
                let n = face_plane_normal(f);
                (n - normal).length_squared() < 1.0e-4
            })
            .unwrap_or_else(|| panic!("no face with normal {normal:?}"))
    }

    #[test]
    fn side_select_faces_grabs_facing_plane_only() {
        let brush = box_brush();
        // Ray passes ABOVE the box (z=20, well past the z=16 top): the ray
        // runs through the outer half-space of the top face while staying
        // inside every other plane -> only the top face is grabbed.
        // Face normals point INWARD, so the top face has normal -Z.
        let faces = side_select_faces(&brush, Vec3::new(500.0, 0.0, 20.0), Vec3::NEG_X);
        assert_eq!(faces, vec![face_index_with_normal(&brush, Vec3::NEG_Z)]);
    }

    #[test]
    fn side_select_faces_grabs_both_faces_beside_an_edge() {
        let brush = box_brush();
        // Ray passing BESIDE the top-right corner (y=20, z=20) stays outside
        // BOTH the top and right inner half-spaces — faithful to q3radiant
        // Brush_SideSelect (verified against CoD1Radiant): no face is grabbed.
        let faces = side_select_faces(&brush, Vec3::new(500.0, 20.0, 20.0), Vec3::NEG_X);
        assert!(faces.is_empty());
    }

    #[test]
    fn side_select_faces_grabs_the_side_the_ray_misses_toward() {
        let brush = box_brush();
        // Ray passing to the right (y=20) grabs the +Y face (inward normal -Y).
        let faces = side_select_faces(&brush, Vec3::new(500.0, 20.0, 0.0), Vec3::NEG_X);
        assert_eq!(faces, vec![face_index_with_normal(&brush, Vec3::NEG_Y)]);
    }

    #[test]
    fn side_select_faces_misses_never_grab_the_far_side() {
        let brush = box_brush();
        // Ray passing BELOW the box (z=-20) must NOT grab the top face
        // (its outer half-space is z > 16) — the bottom face is grabbed
        // instead (inward normal +Z).
        let faces = side_select_faces(&brush, Vec3::new(500.0, 0.0, -20.0), Vec3::NEG_X);
        assert_eq!(faces, vec![face_index_with_normal(&brush, Vec3::Z)]);
    }

    #[test]
    fn stretch_brush_side_faces_moves_only_the_grabbed_planes() {
        let mut brush = box_brush();
        let top = face_index_with_normal(&brush, Vec3::NEG_Z);
        let mut generation = 0u64;

        let before = brush.aabb.clone();
        assert!(stretch_brush_side_faces(
            &mut brush,
            &mut generation,
            &[top],
            Vec3::new(0.0, 0.0, 8.0),
        ));
        assert_eq!(brush.aabb.max.z, before.max.z + 8.0);
        assert_eq!(brush.aabb.min.z, before.min.z);
        assert_eq!(brush.aabb.min.x, before.min.x);

        // A delta that would push the top face THROUGH the bottom must be
        // refused (brush would degenerate).
        assert!(!stretch_brush_side_faces(
            &mut brush,
            &mut generation,
            &[top],
            Vec3::new(0.0, 0.0, -1000.0),
        ));
        // And the brush is left untouched after a refusal.
        assert_eq!(brush.aabb.max.z, before.max.z + 8.0);
    }

    #[test]
    fn find_touching_brushes_detects_touching_in_multi_entity_map() {
        let map_content = include_str!("../test/multi_entity.map");
        let mut map = parse_map_string(map_content).expect("failed to parse map");

        // Brush 0 sits on top of brush 1 (face 1 at z=0)
        let touching = find_touching_brushes(&mut map, 0, 0, 1.0);
        assert!(
            touching.contains(&(0, 2)),
            "brush 0 should touch brush 1 (sits on top). Found: {:?}",
            touching
        );

        // Same from brush 1's perspective
        let touching = find_touching_brushes(&mut map, 0, 2, 0.1);
        assert!(
            touching.contains(&(0, 0)),
            "brush 1 should detect brush 0 touching it"
        );

        // Entity 0 (worldspawn) and Entity 3 (trigger) don't touch (z ranges differ)
        let touching = find_touching_brushes(&mut map, 0, 0, 0.1);
        let touches_entity3 = touching.iter().any(|(e, b)| *e == 3 && *b == 0);
        assert!(
            !touches_entity3,
            "worldspawn brush should not touch entity 3 trigger"
        );

        // Entity 3 and Entity 4 are far apart in X (558-576 vs -50 to -32)
        let touching = find_touching_brushes(&mut map, 3, 0, 0.1);
        let touches_entity4 = touching.iter().any(|(e, b)| *e == 4 && *b == 0);
        assert!(
            !touches_entity4,
            "entity 3 trigger should not touch entity 4 trigger"
        );
    }

    #[test]
    fn find_touching_brushes_detects_adjacent_brushes() {
        // Create two adjacent brushes that share a face
        let brush1 = convex_brush_from_aabb(
            BrushId(0),
            Aabb::from_points(Vec3::new(0.0, 0.0, 0.0), Vec3::new(64.0, 64.0, 64.0)),
            "common/caulk",
        );
        let brush2 = convex_brush_from_aabb(
            BrushId(1),
            Aabb::from_points(Vec3::new(64.0, 0.0, 0.0), Vec3::new(128.0, 64.0, 64.0)),
            "common/caulk",
        );

        let mut map = Map::default();
        map.entities.push(Entity {
            id: EntityId(0),
            classname: "worldspawn".to_string(),
            properties: std::collections::HashMap::new(),
            brushes: vec![brush1],
            model: None,
        });
        map.entities.push(Entity {
            id: EntityId(1),
            classname: "func_group".to_string(),
            properties: std::collections::HashMap::new(),
            brushes: vec![brush2],
            model: None,
        });

        // Brush at entity 0, brush 0 should touch brush at entity 1, brush 0
        let touching = find_touching_brushes(&mut map, 0, 0, 0.1);
        assert!(touching.contains(&(1, 0)), "adjacent brushes should touch");

        // Same test from the other direction
        let touching = find_touching_brushes(&mut map, 1, 0, 0.1);
        assert!(
            touching.contains(&(0, 0)),
            "adjacent brushes should touch (reverse)"
        );
    }

    #[test]
    fn find_touching_brushes_with_epsilon_gap() {
        // Create two brushes with a small gap between them
        let brush1 = convex_brush_from_aabb(
            BrushId(0),
            Aabb::from_points(Vec3::new(0.0, 0.0, 0.0), Vec3::new(64.0, 64.0, 64.0)),
            "common/caulk",
        );
        let brush2 = convex_brush_from_aabb(
            BrushId(1),
            Aabb::from_points(Vec3::new(65.0, 0.0, 0.0), Vec3::new(129.0, 64.0, 64.0)),
            "common/caulk",
        );

        let mut map = Map::default();
        map.entities.push(Entity {
            id: EntityId(0),
            classname: "worldspawn".to_string(),
            properties: std::collections::HashMap::new(),
            brushes: vec![brush1],
            model: None,
        });
        map.entities.push(Entity {
            id: EntityId(1),
            classname: "func_group".to_string(),
            properties: std::collections::HashMap::new(),
            brushes: vec![brush2],
            model: None,
        });

        // With small epsilon, they don't touch (gap of 1 unit)
        let touching = find_touching_brushes(&mut map, 0, 0, 0.5);
        assert!(
            !touching.contains(&(1, 0)),
            "brushes with 1-unit gap should not touch with eps=0.5"
        );

        // With larger epsilon, they should touch
        let touching = find_touching_brushes(&mut map, 0, 0, 2.0);
        assert!(
            touching.contains(&(1, 0)),
            "brushes with 1-unit gap should touch with eps=2.0"
        );
    }

    #[test]
    fn pick_base_anchored_entity_hits_all_heights() {
        let mut map = Map::default();
        map.entities.push(Entity {
            id: EntityId(0),
            classname: "worldspawn".to_string(),
            properties: std::collections::HashMap::new(),
            brushes: vec![],
            model: None,
        });
        // Simulate an entity created by add_entity before classname was stored in
        // properties: the struct field is set, the property key is missing.
        let mut props = std::collections::HashMap::new();
        props.insert("origin".to_string(), "0 0 0".to_string());
        map.entities.push(Entity {
            id: EntityId(1),
            classname: "mp_deathmatch_spawn".to_string(),
            properties: props,
            brushes: vec![],
            model: None,
        });

        let config = crate::editor::config::EntityDrawingConfig::cod_default();
        let mask = PickMask::ALL;

        // 3D view style: camera above, ray aimed at the upper half (z=60 of a 0..72 box)
        let ray_origin = Vec3::new(0.0, 0.0, 120.0);
        let ray_dir = (Vec3::new(0.5, 0.5, 60.0) - ray_origin).normalize();
        let hit =
            pick_brush_or_ent_by_ray(&mut map, ray_origin, ray_dir, mask, &config, None, None);
        assert_eq!(
            hit,
            Some(Err(1)),
            "ray through upper half (z=60) must hit the entity"
        );

        // Same camera, ray aimed at the bottom of the box (z=8)
        let ray_dir = (Vec3::new(0.5, 0.5, 8.0) - ray_origin).normalize();
        let hit =
            pick_brush_or_ent_by_ray(&mut map, ray_origin, ray_dir, mask, &config, None, None);
        assert_eq!(
            hit,
            Some(Err(1)),
            "ray through bottom (z=8) must hit the entity"
        );

        // 2D XZ view style: vertical ray at click height z=60 (upper half)
        let hit = pick_brush_or_ent_by_ray(
            &mut map,
            Vec3::new(0.5, 1.0e6, 60.0),
            Vec3::new(0.0, -1.0, 0.0),
            mask,
            &config,
            None,
            None,
        );
        assert_eq!(hit, Some(Err(1)), "2D XZ ray at z=60 must hit the entity");

        // 2D XZ view style: click at z=8 (bottom region)
        let hit = pick_brush_or_ent_by_ray(
            &mut map,
            Vec3::new(0.5, 1.0e6, 8.0),
            Vec3::new(0.0, -1.0, 0.0),
            mask,
            &config,
            None,
            None,
        );
        assert_eq!(hit, Some(Err(1)), "2D XZ ray at z=8 must hit the entity");
    }

    #[test]
    fn find_touching_brushes_excludes_self() {
        // Create a single brush
        let brush = convex_brush_from_aabb(
            BrushId(0),
            Aabb::from_points(Vec3::new(0.0, 0.0, 0.0), Vec3::new(64.0, 64.0, 64.0)),
            "common/caulk",
        );

        let mut map = Map::default();
        map.entities.push(Entity {
            id: EntityId(0),
            classname: "worldspawn".to_string(),
            properties: std::collections::HashMap::new(),
            brushes: vec![brush],
            model: None,
        });

        let touching = find_touching_brushes(&mut map, 0, 0, 0.1);
        assert!(touching.is_empty(), "brush should not touch itself");
    }

    // --- Edge editing (Blender-style edge drag) ---------------------------

    fn cube_map() -> (Map, usize, usize) {
        let brush = convex_brush_from_aabb(
            BrushId(0),
            Aabb::from_points(Vec3::splat(0.0), Vec3::splat(64.0)),
            "common/caulk",
        );
        let mut map = Map::default();
        map.entities.push(Entity {
            id: EntityId(0),
            classname: "worldspawn".to_string(),
            properties: std::collections::HashMap::new(),
            brushes: vec![brush],
            model: None,
        });
        (map, 0, 0)
    }

    fn face_idx_with_normal(brush: &Brush, normal: Vec3) -> usize {
        let BrushContent::Convex(faces) = &brush.content else {
            panic!("expected convex brush");
        };
        faces
            .iter()
            .position(|f| {
                let n = face_plane_normal(f);
                (n - normal).length_squared() < 1.0e-4
            })
            .unwrap_or_else(|| panic!("no face with normal {normal:?}"))
    }

    fn find_shared_edge(map: &Map, na: Vec3, nb: Vec3) -> (usize, usize, Vec3, Vec3) {
        let brush = &map.entities[0].brushes[0];
        let polys = crate::geometry::brush_to_polygons(brush).unwrap();
        let fa = face_idx_with_normal(brush, na);
        let fb = face_idx_with_normal(brush, nb);
        let (a, b) = crate::core_util::shared_edge_points(&polys, fa, fb)
            .unwrap_or_else(|| panic!("faces {na:?}/{nb:?} share no edge"));
        (fa, fb, a, b)
    }

    #[test]
    fn edge_move_tilts_only_adjacent_faces() {
        let (mut map, e, b) = cube_map();
        // Edge between the top face (inward normal -Z) and the +X wall
        // (inward normal -X).
        let (fa, fb, a, bpt) = find_shared_edge(&map, Vec3::NEG_Z, Vec3::NEG_X);
        let sel = EdgeSelection {
            entity_idx: e,
            brush_idx: b,
            face_a_idx: fa,
            face_b_idx: fb,
        };

        let brush_before = map.entities[0].brushes[0].clone();
        let delta = Vec3::new(0.0, 0.0, 8.0);
        assert!(translate_selected_edges(&mut map, &[sel], delta));

        let brush_after = &map.entities[0].brushes[0];
        let BrushContent::Convex(faces_after) = &brush_after.content else {
            panic!("expected convex brush");
        };
        let BrushContent::Convex(faces_before) = &brush_before.content else {
            panic!("expected convex brush");
        };

        // Untouched faces keep their exact planes.
        for (i, (before, after)) in faces_before.iter().zip(faces_after.iter()).enumerate() {
            if i == fa || i == fb {
                assert_ne!(
                    before.plane_points, after.plane_points,
                    "adjacent faces must tilt"
                );
            } else {
                assert_eq!(
                    before.plane_points, after.plane_points,
                    "face {i} must keep its plane"
                );
            }
        }

        // The moved edge sits at start+delta / end+delta, and both refit
        // planes contain it.
        let polys = crate::geometry::brush_to_polygons(brush_after).unwrap();
        let (na, nb) = crate::core_util::shared_edge_points(&polys, fa, fb)
            .expect("edge still shared after move");
        assert!((na - (a + delta)).length() < 1.0e-3);
        assert!((nb - (bpt + delta)).length() < 1.0e-3);
        for &face_idx in &[fa, fb] {
            let n = face_plane_normal(&faces_after[face_idx]);
            let d = n.dot(faces_after[face_idx].plane_points[0]);
            assert!((n.dot(na) - d).abs() < 1.0e-3, "refit plane misses edge");
            assert!((n.dot(nb) - d).abs() < 1.0e-3, "refit plane misses edge");
        }

        // Brush stays valid and the rest of the geometry is untouched.
        assert!(is_convex_brush_valid(brush_after));
        assert_eq!(brush_after.aabb.max.z, 72.0);
        assert_eq!(brush_after.aabb.min.z, 0.0);
        assert_eq!(brush_after.aabb.max.x, 64.0);
    }

    #[test]
    fn edge_move_refuses_when_faces_share_no_edge() {
        let (mut map, e, b) = cube_map();
        // Top (inward -Z) and bottom (inward +Z) faces share no edge.
        let fa = face_idx_with_normal(&map.entities[0].brushes[0], Vec3::NEG_Z);
        let fb = face_idx_with_normal(&map.entities[0].brushes[0], Vec3::Z);
        let sel = EdgeSelection {
            entity_idx: e,
            brush_idx: b,
            face_a_idx: fa,
            face_b_idx: fb,
        };

        let before = map.entities[0].brushes[0].clone();
        assert!(!translate_selected_edges(
            &mut map,
            &[sel],
            Vec3::new(0.0, 0.0, -8.0)
        ));
        // Nothing was modified.
        let mut before = before;
        before.invalidate_geometry();
        assert_eq!(
            before
                .get_polygons()
                .map(|p| p.to_vec())
                .unwrap_or_default(),
            crate::geometry::brush_to_polygons(&map.entities[0].brushes[0]).unwrap()
        );
    }

    #[test]
    fn edge_move_clamps_instead_of_stretching_to_infinity() {
        let (mut map, e, b) = cube_map();
        let (fa, fb, a, _bpt) = find_shared_edge(&map, Vec3::NEG_Z, Vec3::NEG_X);
        let sel = EdgeSelection {
            entity_idx: e,
            brush_idx: b,
            face_a_idx: fa,
            face_b_idx: fb,
        };
        // Drag the edge down 1000 units — far past collapse.
        let delta = Vec3::new(0.0, 0.0, -1000.0);
        let factor = edge_move_clamp_factor(&map, &[sel], delta);
        assert!(factor > 1.0e-4 && factor < 0.1, "factor = {factor}");

        assert!(translate_selected_edges(&mut map, &[sel], delta));
        let brush = &map.entities[0].brushes[0];
        assert!(is_convex_brush_valid(brush), "clamped move must stay valid");
        // The brush got squashed, not stretched towards infinity: the whole
        // brush is within the original AABB dilated by the FULL drag.
        assert!(brush.aabb.min.z > -1000.0 - 1.0);
        assert!((brush.aabb.max.z - 64.0).abs() < 0.01);
        // The applied move matches the clamped delta (preview parity).
        let polys = crate::geometry::brush_to_polygons(brush).unwrap();
        let (na, _nb) = crate::core_util::shared_edge_points(&polys, fa, fb).unwrap();
        let expected_z = a.z + delta.z * factor;
        assert!(
            (na.z - expected_z).abs() < 1.0,
            "moved to {} want {expected_z}",
            na.z
        );
    }

    #[test]
    fn edge_move_preview_matches_applied_result() {
        let (mut map, e, b) = cube_map();
        let (fa, fb, ..) = find_shared_edge(&map, Vec3::NEG_Z, Vec3::NEG_Y);
        let sel = EdgeSelection {
            entity_idx: e,
            brush_idx: b,
            face_a_idx: fa,
            face_b_idx: fb,
        };
        let delta = Vec3::new(16.0, 0.0, 8.0);

        let brush = map.entities[0].brushes[0].clone();
        let preview =
            preview_edge_moved_polys(&brush, &[(fa, fb)], delta).expect("preview should succeed");

        assert!(translate_selected_edges(&mut map, &[sel], delta));
        let applied = crate::geometry::brush_to_polygons(&map.entities[0].brushes[0]).unwrap();
        assert_eq!(preview, applied, "preview and apply must agree exactly");
    }

    #[test]
    fn edge_move_with_two_selected_edges_sharing_face_rejected() {
        // Two edges that share a face (NEG_Z) → coplanarity check rejects
        // because the second edge's translated endpoint falls off the plane
        // fitted through the first edge's endpoints + reference.
        let (mut map, e, b) = cube_map();
        let (fa1, fb1, ..) = find_shared_edge(&map, Vec3::NEG_Z, Vec3::NEG_X);
        let (fa2, fb2, ..) = find_shared_edge(&map, Vec3::NEG_Z, Vec3::NEG_Y);
        let sels = [
            EdgeSelection {
                entity_idx: e,
                brush_idx: b,
                face_a_idx: fa1,
                face_b_idx: fb1,
            },
            EdgeSelection {
                entity_idx: e,
                brush_idx: b,
                face_a_idx: fa2,
                face_b_idx: fb2,
            },
        ];
        let delta = Vec3::new(0.0, 0.0, 4.0);
        assert!(
            !translate_selected_edges(&mut map, &sels, delta),
            "two edges sharing a face must be rejected (coplanarity)"
        );
        assert!(is_convex_brush_valid(&map.entities[e].brushes[b]));
    }

    #[test]
    fn edge_move_ignores_patch_brushes_and_zero_delta() {
        let (mut map, e, b) = cube_map();
        let sel = EdgeSelection {
            entity_idx: e,
            brush_idx: b,
            face_a_idx: 0,
            face_b_idx: 1,
        };
        assert!(!translate_selected_edges(&mut map, &[sel], Vec3::ZERO));
        assert!(!translate_selected_edges(&mut map, &[], Vec3::ONE));
    }

    #[test]
    fn edge_move_two_opposite_edges_sharing_face_rejected() {
        // Two opposite edges on the bottom face (-Z) cover all 4 vertices.
        // There is no stationary reference → must fail.
        let (mut map, e, b) = cube_map();
        let (fa1, fb1, ..) = find_shared_edge(&map, Vec3::NEG_Z, Vec3::NEG_X);
        let (fa2, fb2, ..) = find_shared_edge(&map, Vec3::NEG_Z, Vec3::X);
        let sels = [
            EdgeSelection {
                entity_idx: e,
                brush_idx: b,
                face_a_idx: fa1,
                face_b_idx: fb1,
            },
            EdgeSelection {
                entity_idx: e,
                brush_idx: b,
                face_a_idx: fa2,
                face_b_idx: fb2,
            },
        ];
        let delta = Vec3::new(0.0, 0.0, 4.0);
        assert!(
            !translate_selected_edges(&mut map, &sels, delta),
            "moving two opposite edges on one face must fail (no stationary reference)"
        );
        assert!(is_convex_brush_valid(&map.entities[e].brushes[b]));
    }

    #[test]
    fn edge_move_two_non_sharing_edges_succeeds() {
        // Two edges that don't share a face with each other: each face has
        // its own stationary reference and each edge is translated by delta.
        let (mut map, e, b) = cube_map();
        // Bottom-left edge (NEG_Z & NEG_X) and top-right edge (+Z & +X).
        let (fa1, fb1, a1_start, a1_end) = find_shared_edge(&map, Vec3::NEG_Z, Vec3::NEG_X);
        let (fa2, fb2, a2_start, a2_end) = find_shared_edge(&map, Vec3::X, Vec3::Z);
        let sels = [
            EdgeSelection {
                entity_idx: e,
                brush_idx: b,
                face_a_idx: fa1,
                face_b_idx: fb1,
            },
            EdgeSelection {
                entity_idx: e,
                brush_idx: b,
                face_a_idx: fa2,
                face_b_idx: fb2,
            },
        ];
        let delta = Vec3::new(8.0, 0.0, 0.0);
        assert!(
            translate_selected_edges(&mut map, &sels, delta),
            "moving two non-sharing edges must succeed"
        );
        let brush = &map.entities[e].brushes[b];
        assert!(is_convex_brush_valid(brush));
        let polys_after = crate::geometry::brush_to_polygons(brush).unwrap();
        let (na1, nb1) = crate::core_util::shared_edge_points(&polys_after, fa1, fb1).unwrap();
        let (na2, nb2) = crate::core_util::shared_edge_points(&polys_after, fa2, fb2).unwrap();
        assert!(
            (na1 - (a1_start + delta)).length() < 0.01,
            "edge 1 start not translated"
        );
        assert!(
            (nb1 - (a1_end + delta)).length() < 0.01,
            "edge 1 end not translated"
        );
        assert!(
            (na2 - (a2_start + delta)).length() < 0.01,
            "edge 2 start not translated"
        );
        assert!(
            (nb2 - (a2_end + delta)).length() < 0.01,
            "edge 2 end not translated"
        );
    }

    #[test]
    fn edge_move_two_adjacent_edges_same_face_rejected() {
        // Two adjacent edges on the top face (+Z) share one vertex.  Moving
        // both by a face-normal delta still makes the translated endpoints
        // non-coplanar with the fitted plane (the3rd translated vertex falls
        // off the plane through the first edge + reference).  The coplanarity
        // check in edge_move_plane_updates must reject this.
        let (mut map, e, b) = cube_map();
        let (fa1, fb1, ..) = find_shared_edge(&map, Vec3::Z, Vec3::NEG_X);
        let (fa2, fb2, ..) = find_shared_edge(&map, Vec3::Z, Vec3::NEG_Y);
        let sels = [
            EdgeSelection {
                entity_idx: e,
                brush_idx: b,
                face_a_idx: fa1,
                face_b_idx: fb1,
            },
            EdgeSelection {
                entity_idx: e,
                brush_idx: b,
                face_a_idx: fa2,
                face_b_idx: fb2,
            },
        ];
        // Even a face-normal delta makes the3 translated endpoints non-coplanar.
        let delta = Vec3::new(0.0, 0.0, 4.0);
        assert!(
            !translate_selected_edges(&mut map, &sels, delta),
            "two adjacent edges on one face must be rejected (coplanarity)"
        );
        assert!(is_convex_brush_valid(&map.entities[e].brushes[b]));
    }

    #[test]
    fn edge_move_two_adjacent_edges_oblique_delta_rejected() {
        // Same two adjacent edges, but an oblique delta makes the translated
        // endpoints non-coplanar → must be rejected by the coplanarity check.
        let (mut map, e, b) = cube_map();
        let (fa1, fb1, ..) = find_shared_edge(&map, Vec3::Z, Vec3::NEG_X);
        let (fa2, fb2, ..) = find_shared_edge(&map, Vec3::Z, Vec3::NEG_Y);
        let sels = [
            EdgeSelection {
                entity_idx: e,
                brush_idx: b,
                face_a_idx: fa1,
                face_b_idx: fb1,
            },
            EdgeSelection {
                entity_idx: e,
                brush_idx: b,
                face_a_idx: fa2,
                face_b_idx: fb2,
            },
        ];
        // Oblique delta: translated endpoints won't be coplanar.
        let delta = Vec3::new(8.0, 8.0, 4.0);
        assert!(
            !translate_selected_edges(&mut map, &sels, delta),
            "oblique delta on adjacent edges must fail coplanarity check"
        );
        assert!(is_convex_brush_valid(&map.entities[e].brushes[b]));
    }
}
