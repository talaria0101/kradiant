//! Geometry utilities for tessellating `.map` brushes and patches into renderable meshes.
//!

use std::collections::HashMap;

use crate::map::{Brush, BrushContent, Face, Patch, PatchType};
use glam::{Vec2, Vec3, Vec4};
use rayon::iter::{
    IndexedParallelIterator, IntoParallelRefIterator, IntoParallelRefMutIterator, ParallelIterator,
};
use rayon::slice::ParallelSlice;
use smallvec::{SmallVec, smallvec};
use thiserror::Error;

/// Errors produced while generating renderable geometry from map data.
#[derive(Error, Debug)]
pub enum GeometryError {
    #[error("degenerate brush (fewer than 4 faces)")]
    DegenerateBrush,

    #[error("plane intersection/tessellation failed")]
    IntersectionFailed,

    #[error("invalid patch: {0}")]
    InvalidPatch(String),
}

#[derive(Debug, Clone, Copy)]
struct PlaneEq {
    n: Vec3, // unit normal
    d: f32,  // plane offset: n·x = d
}

/// A simple triangle mesh suitable for OpenGL-style rendering.
#[derive(Debug, Clone)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub indices: Vec<u32>,
}

/// Tessellated patch mesh (positions + UVs + colors).
#[derive(Debug, Clone)]
pub struct PatchMesh {
    pub positions: Vec<Vec3>,
    /// UVs in "repeat space" (not clamped to 0..1).
    pub uvs: Vec<Vec2>,
    /// Vertex color as RGBA8 (as stored in the map for terrain/patches).
    pub colors: Vec<[u8; 4]>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

/// Convert a convex brush into renderable per‑face polygons (vertex + local index buffers).
///
/// Output is aligned with the source face list: `out[i]` corresponds to `faces[i]`.
pub fn brush_to_polygons(brush: &Brush) -> Result<Vec<(Vec<Vec3>, Vec<u32>)>, GeometryError> {
    let faces = match &brush.content {
        BrushContent::Convex(faces) => faces.as_slice(),
        BrushContent::Patch(_) => return Err(GeometryError::IntersectionFailed),
    };

    if faces.len() < 4 {
        return Err(GeometryError::DegenerateBrush);
    }

    // Epsilon tuning:
    // - CoD1 maps are authored at grid-aligned scales, but plane intersection math introduces
    // small errors. Clipping with a modest epsilon is typically more robust than strict tests.
    const ORIENT_EPS: f32 = 1e-3;
    const CLIP_EPS: f32 = 1e-3;
    const DEDUP_EPS: f32 = 1e-4;
    const COLINEAR_EPS: f32 = 1e-5;

    let planes_raw = faces
        .iter()
        .map(face_to_plane)
        .collect::<Result<Vec<_>, GeometryError>>()?;

    let extent = brush_extent_hint(faces).max(128.0);
    let base_size = extent * 4.0 + 1024.0;

    // Fast path: trust Radiant plane point ordering (common for .map) and avoid the expensive
    // "enumerate all 3-plane intersections" orientation step.
    let out = build_polys_for_planes(&planes_raw, base_size, CLIP_EPS, DEDUP_EPS, COLINEAR_EPS);
    if out.par_iter().any(|(w, _)| w.len() >= 3) {
        return Ok(out);
    }

    // Slow path: orient planes using a bounded set of candidate intersection points.
    const MAX_CANDIDATES: usize = 8192;
    let candidates = enumerate_candidates_capped(&planes_raw, MAX_CANDIDATES);
    if candidates.is_empty() {
        return Err(GeometryError::IntersectionFailed);
    }

    let (planes, _interior) = orient_planes(&planes_raw, &candidates, ORIENT_EPS)?;
    let out = build_polys_for_planes(&planes, base_size, CLIP_EPS, DEDUP_EPS, COLINEAR_EPS);
    if out.par_iter().any(|(w, _)| w.len() >= 3) {
        Ok(out)
    } else {
        Err(GeometryError::IntersectionFailed)
    }
}

/// Flatten a brush into a single triangle‑list mesh (positions + indices).
///
/// This is a convenience wrapper over [`brush_to_polygons`]. Each face is triangulated
/// into a fan, and all triangles are concatenated into one index buffer.
pub fn brush_to_mesh(brush: &Brush) -> Result<Mesh, GeometryError> {
    let polys = brush_to_polygons(brush)?;
    let mut positions = Vec::new();
    let mut indices = Vec::new();

    for (face_positions, face_indices) in polys {
        if face_positions.len() < 3 || face_indices.is_empty() {
            continue;
        }
        let base = positions.len() as u32;
        positions.extend(face_positions);
        indices.extend(face_indices.into_iter().map(|i| i + base));
    }

    Ok(Mesh { positions, indices })
}

/// Tessellate a patch brush (terrain or curve) into a triangle mesh.
pub fn tessellate_patch(patch: &Patch) -> Result<PatchMesh, GeometryError> {
    match patch.patch_type {
        PatchType::Terrain => tessellate_terrain_patch(patch),
        PatchType::Curve => tessellate_curve_patch(patch),
    }
}

/// Mutate a single brush plane in‑place, marking cached geometry dirty.
pub fn update_brush_plane(brush: &mut Brush, plane_index: usize, new_plane: [Vec3; 3]) {
    if let BrushContent::Convex(faces) = &mut brush.content {
        if let Some(face) = faces.get_mut(plane_index) {
            face.plane_points = new_plane;
        }
    }
}

fn face_to_plane(face: &Face) -> Result<PlaneEq, GeometryError> {
    let a = face.plane_points[0];
    let b = face.plane_points[1];
    let c = face.plane_points[2];
    let n_raw = (b - a).cross(c - a);
    let len = n_raw.length();
    if !len.is_finite() || len < 1e-8 {
        return Err(GeometryError::IntersectionFailed);
    }
    let n = n_raw / len;
    let d = n.dot(a);
    Ok(PlaneEq { n, d })
}

fn intersect_three(p1: PlaneEq, p2: PlaneEq, p3: PlaneEq) -> Option<Vec3> {
    // Closed-form intersection of three planes:
    // x = (d1*(n2×n3) + d2*(n3×n1) + d3*(n1×n2)) / (n1·(n2×n3))
    let n1 = p1.n;
    let n2 = p2.n;
    let n3 = p3.n;
    let n2xn3 = n2.cross(n3);
    let denom = n1.dot(n2xn3);
    if denom.abs() < 1e-8 || !denom.is_finite() {
        return None;
    }
    let num = (n2xn3 * p1.d) + (n3.cross(n1) * p2.d) + (n1.cross(n2) * p3.d);
    let x = num / denom;
    if x.x.is_finite() && x.y.is_finite() && x.z.is_finite() {
        Some(x)
    } else {
        None
    }
}

fn enumerate_candidates_capped(planes: &[PlaneEq], cap: usize) -> Vec<Vec3> {
    let mut candidates = Vec::<Vec3>::new();
    let n = planes.len();
    if n < 3 || cap == 0 {
        return candidates;
    }
    candidates.reserve(cap.min(1024));
    for i in 0..n {
        for j in (i + 1)..n {
            for k in (j + 1)..n {
                if candidates.len() >= cap {
                    return candidates;
                }
                if let Some(p) = intersect_three(planes[i], planes[j], planes[k]) {
                    candidates.push(p);
                }
            }
        }
    }
    candidates
}

fn build_polys_for_planes(
    planes: &[PlaneEq],
    base_size: f32,
    clip_eps: f32,
    dedup_eps: f32,
    colinear_eps: f32,
) -> Vec<(Vec<Vec3>, Vec<u32>)> {
    planes
        .par_iter()
        .with_min_len(1)
        .enumerate()
        .map(|(i, face_plane)| {
            let mut winding = base_winding_for_plane(*face_plane, base_size);

            for (j, clip_plane) in planes.iter().enumerate() {
                if i == j {
                    continue;
                }
                if is_same_plane(*face_plane, *clip_plane, 1e-4) {
                    continue;
                }
                winding = clip_winding_epsilon(&winding, *clip_plane, clip_eps);
                if winding.len() < 3 {
                    return (Vec::new(), Vec::new());
                }
            }

            if winding.len() >= 3 {
                dedup_consecutive(&mut winding, dedup_eps);
                remove_colinear(&mut winding, colinear_eps);
            }

            if winding.len() < 3 {
                return (Vec::new(), Vec::new());
            }

            let indices = triangulate_fan(winding.len());
            (winding.to_vec(), indices)
        })
        .collect()
}

fn orient_planes(
    planes_raw: &[PlaneEq],
    candidates: &[Vec3],
    orient_eps: f32,
) -> Result<(Vec<PlaneEq>, Vec3), GeometryError> {
    // Choose a candidate intersection point that is most "inside-like" for either global
    // orientation, then use it to orient each plane individually.
    let mut best_score = f32::INFINITY;
    let mut best_p = None;
    let mut best_global_flip = false;

    for &p in candidates {
        let mut max_a = f32::NEG_INFINITY;
        let mut max_b = f32::NEG_INFINITY;
        for pl in planes_raw {
            let dist = pl.n.dot(p) - pl.d;
            max_a = max_a.max(dist);
            max_b = max_b.max(-dist);
        }
        let (score, flip) = if max_a <= max_b {
            (max_a, false)
        } else {
            (max_b, true)
        };
        if score < best_score {
            best_score = score;
            best_p = Some(p);
            best_global_flip = flip;
        }
    }

    let Some(interior) = best_p else {
        return Err(GeometryError::IntersectionFailed);
    };

    let mut planes: Vec<PlaneEq> = if best_global_flip {
        planes_raw
            .iter()
            .map(|p| PlaneEq { n: -p.n, d: -p.d })
            .collect()
    } else {
        planes_raw.to_vec()
    };

    for pl in &mut planes {
        // Ensure `interior` lies on the "inside" side: n·x <= d (+eps).
        if pl.n.dot(interior) > pl.d + orient_eps {
            pl.n = -pl.n;
            pl.d = -pl.d;
        }
    }

    Ok((planes, interior))
}

fn brush_extent_hint(faces: &[Face]) -> f32 {
    let mut max_abs = 0.0f32;
    for f in faces {
        for p in &f.plane_points {
            let ax = p.x.abs();
            let ay = p.y.abs();
            let az = p.z.abs();
            max_abs = max_abs.max(ax.max(ay).max(az));
        }
    }
    max_abs
}

fn is_same_plane(a: PlaneEq, b: PlaneEq, eps: f32) -> bool {
    let nd = a.n.dot(b.n);
    nd.abs() > 0.9999 && (a.d - b.d).abs() <= eps
}

// fn base_winding_for_plane(plane: PlaneEq, size: f32) -> [Vec3; 4] {
//     // Construct a large quad on the plane. The returned winding is CCW when viewed from the
//     // side the plane normal points towards.
//     let n = plane.n;
//     let org = n * plane.d;
//
//     // Pick an "up" that is not parallel to the normal.
//     let up = if n.z.abs() < 0.999 { Vec3::Z } else { Vec3::Y };
//     let vright = up.cross(n).normalize();
//     let vup = n.cross(vright).normalize();
//
//     let vright = vright * size;
//     let vup = vup * size;
//
//     [org - vright + vup, org - vright - vup, org + vright - vup, org + vright + vup]
// }

fn base_winding_for_plane(plane: PlaneEq, size: f32) -> WindBuf {
    // Construct a large quad on the plane. The returned winding is CCW when viewed from the
    // side the plane normal points towards.
    let n = plane.n;
    let org = n * plane.d;

    // Pick an "up" that is not parallel to the normal.
    let up = if n.z.abs() < 0.999 { Vec3::Z } else { Vec3::Y };
    let vright = up.cross(n).normalize();
    let vup = n.cross(vright).normalize();

    let vright = vright * size;
    let vup = vup * size;

    // CCW winding (for n=+Z: TL -> BL -> BR -> TR).
    smallvec![
        org - vright + vup,
        org - vright - vup,
        org + vright - vup,
        org + vright + vup,
    ]
}

type DistBuf = SmallVec<[f32; 20]>;
type WindBuf = SmallVec<[Vec3; 24]>;

fn clip_winding_epsilon(winding: &[Vec3], plane: PlaneEq, eps: f32) -> WindBuf {
    if winding.is_empty() {
        return SmallVec::new();
    }

    // Keep points on the inside (back) side: dist <= eps.
    // let mut dists = Vec::with_capacity(winding.len());
    let mut dists: DistBuf = SmallVec::new();
    let mut any_front = false;
    let mut any_back = false;

    for &p in winding {
        let dist = plane.n.dot(p) - plane.d;
        dists.push(dist);
        if dist > eps {
            any_front = true;
        } else {
            any_back = true;
        }
    }

    if !any_back {
        return SmallVec::new();
    }
    if !any_front {
        return winding.into();
    }

    let mut out = SmallVec::with_capacity(winding.len() + 4);
    let n = winding.len();
    for i in 0..n {
        let p1 = winding[i];
        let p2 = winding[(i + 1) % n];
        let d1 = dists[i];
        let d2 = dists[(i + 1) % n];

        if d1 <= eps {
            out.push(p1);
        }

        let crosses = (d1 <= eps && d2 > eps) || (d1 > eps && d2 <= eps);
        if crosses {
            // Intersect segment with plane at dist = 0.
            let t = d1 / (d1 - d2);
            out.push(p1 + (p2 - p1) * t);
        }
    }

    out
}

fn dedup_consecutive(w: &mut WindBuf, eps: f32) {
    if w.len() < 2 {
        return;
    }
    let eps2 = eps * eps;
    let mut out: WindBuf = SmallVec::new();
    for &p in w.iter() {
        if out.last().map(|q| (*q - p).length_squared() <= eps2) == Some(true) {
            continue;
        }
        out.push(p);
    }
    // Close the loop: drop last if it matches first.
    if out.len() >= 2 {
        let first = out[0];
        if (out[out.len() - 1] - first).length_squared() <= eps2 {
            out.pop();
        }
    }
    *w = out;
}

fn remove_colinear(w: &mut WindBuf, eps: f32) {
    if w.len() < 3 { return; }
    let eps2 = eps * eps;
    let mut out: WindBuf = SmallVec::new();
    let n = w.len();
    for i in 0..n {
        let prev = w[(i + n - 1) % n];
        let cur = w[i];
        let next = w[(i + 1) % n];
        let d1 = cur - prev;
        let d2 = next - cur;
        // Check colinearity without normalizing: cross product magnitude
        // relative to edge lengths. If cross² < eps² * |d1|² * |d2|², colinear.
        let cross = d1.cross(d2);
        let len1sq = d1.length_squared();
        let len2sq = d2.length_squared();
        if len1sq < 1e-12 || len2sq < 1e-12 || cross.length_squared() <= eps2 * len1sq * len2sq {
            continue;
        }
        out.push(cur);
    }
    // Close the loop: drop last if it matches first (same as dedup_consecutive).
    if out.len() >= 2 {
        let first = out[0];
        if (out[out.len() - 1] - first).length_squared() <= eps2 {
            out.pop();
        }
    }
    *w = out;
}

fn triangulate_fan(vertex_count: usize) -> Vec<u32> {
    if vertex_count < 3 {
        return Vec::new();
    }
    let mut indices = Vec::with_capacity((vertex_count - 2) * 3);
    for i in 1..(vertex_count - 1) {
        indices.push(0);
        indices.push(i as u32);
        indices.push((i + 1) as u32);
    }
    indices
}

// ========================= Patch Tessellation =========================

fn tessellate_terrain_patch(patch: &Patch) -> Result<PatchMesh, GeometryError> {
    let rows = patch.vertices.len();
    if rows < 2 {
        return Err(GeometryError::InvalidPatch(
            "terrain patch must have at least 2 rows".into(),
        ));
    }
    let cols = patch.vertices[0].len();
    if cols < 2 {
        return Err(GeometryError::InvalidPatch(
            "terrain patch must have at least 2 cols".into(),
        ));
    }
    for (ri, r) in patch.vertices.iter().enumerate() {
        if r.len() != cols {
            return Err(GeometryError::InvalidPatch(format!(
                "terrain patch is ragged at row {ri}"
            )));
        }
    }

    let mut positions = Vec::with_capacity(rows * cols);
    let mut uvs = Vec::with_capacity(rows * cols);
    let mut colors = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            let v = patch.vertices[r][c];
            positions.push(v.position);
            uvs.push(v.uv);
            colors.push(v.color);
        }
    }

    let mut indices: Vec<u32> = Vec::with_capacity((rows - 1) * (cols - 1) * 6);
    for r in 0..(rows - 1) {
        for c in 0..(cols - 1) {
            let idx00 = (r * cols + c) as u32;
            let idx01 = (r * cols + (c + 1)) as u32;
            let idx10 = ((r + 1) * cols + c) as u32;
            let idx11 = ((r + 1) * cols + (c + 1)) as u32;

            let turned = patch.vertices[r][c].turned_edge;
            if turned {
                // Diagonal v01-v10.
                indices.extend_from_slice(&[idx00, idx10, idx01, idx01, idx10, idx11]);
            } else {
                // Diagonal v00-v11.
                indices.extend_from_slice(&[idx00, idx10, idx11, idx00, idx11, idx01]);
            }
        }
    }

    let normals = compute_vertex_normals(&positions, &indices);
    Ok(PatchMesh {
        positions,
        uvs,
        colors,
        normals,
        indices,
    })
}

fn tessellate_curve_patch(patch: &Patch) -> Result<PatchMesh, GeometryError> {
    let rows = patch.vertices.len();
    let cols = patch.vertices.first().map(|r| r.len()).unwrap_or(0);
    if rows < 3 || cols < 3 {
        return Err(GeometryError::InvalidPatch(
            "curve patch must be at least 3x3".into(),
        ));
    }
    for (ri, r) in patch.vertices.iter().enumerate() {
        if r.len() != cols {
            return Err(GeometryError::InvalidPatch(format!(
                "curve patch is ragged at row {ri}"
            )));
        }
    }
    if (rows - 1) % 2 != 0 || (cols - 1) % 2 != 0 {
        return Err(GeometryError::InvalidPatch(format!(
            "curve patch dimensions must be odd; got {rows}x{cols}"
        )));
    }

    let seg_r = (rows - 1) / 2;
    let seg_c = (cols - 1) / 2;

    let subdiv = patch.params.subdivision.max(1) as usize;
    let tess_rows = seg_r * subdiv + 1;
    let tess_cols = seg_c * subdiv + 1;

    // Parallel evaluation: each (tr, tc) vertex is independent.
    let total = tess_rows * tess_cols;
    let mut positions = vec![Vec3::ZERO; total];
    let mut uvs = vec![Vec2::ZERO; total];
    let mut colors = vec![[0u8; 4]; total];

    let mut color_cache: HashMap<(usize, usize), [[Vec4; 3]; 3]> = HashMap::new();
    for sr in 0..seg_r {
        for sc in 0..seg_c {
            let base_r = sr * 2;
            let base_c = sc * 2;
            let mut c_ctrl = [[Vec4::ZERO; 3]; 3];
            for rr in 0..3 {
                for cc in 0..3 {
                    let vtx = patch.vertices[base_r + rr][base_c + cc];
                    c_ctrl[rr][cc] = Vec4::new(
                        vtx.color[0] as f32 / 255.0,
                        vtx.color[1] as f32 / 255.0,
                        vtx.color[2] as f32 / 255.0,
                        vtx.color[3] as f32 / 255.0,
                    );
                }
            }
            color_cache.insert((base_r, base_c), c_ctrl);
        }
    }

    positions.par_iter_mut().zip(uvs.par_iter_mut()).zip(colors.par_iter_mut()).enumerate().with_min_len(64)
    .for_each(|(idx, ((pos, uv), col))| {
        let tr = idx / tess_cols;
        let tc = idx % tess_cols;

        let (sr, tv) = if tr + 1 == tess_rows {
            (seg_r - 1, 1.0f32)
        } else {
            (tr / subdiv, (tr % subdiv) as f32 / subdiv as f32)
        };
        let base_r = sr * 2;

        let (sc, tu) = if tc + 1 == tess_cols {
            (seg_c - 1, 1.0f32)
        } else {
            (tc / subdiv, (tc % subdiv) as f32 / subdiv as f32)
        };
        let base_c = sc * 2;

        let c_ctrl = color_cache.get(&(base_r, base_c)).unwrap();

        if let Ok((p, u, c)) = eval_quadratic_patch_attributes(patch, base_r, base_c, tu, tv, c_ctrl) {
            *pos = p;
            *uv = u;
            *col = c;
        }
        else {
            log::debug!("Invalid patch");
        }
    });

    // let flat: Vec<(usize, usize)> = (0..tess_rows)
    //     .flat_map(|tr| (0..tess_cols).map(move |tc| (tr, tc)))
    //     .collect();
    //
    // let results: Vec<(Vec3, Vec2, [u8; 4])> = flat
    //     .par_iter()
    //     .with_min_len(64)
    //     .map(|&(tr, tc)| {
    //         let (sr, tv) = if tr + 1 == tess_rows {
    //             (seg_r - 1, 1.0f32)
    //         } else {
    //             (tr / subdiv, (tr % subdiv) as f32 / subdiv as f32)
    //         };
    //         let base_r = sr * 2;
    //
    //         let (sc, tu) = if tc + 1 == tess_cols {
    //             (seg_c - 1, 1.0f32)
    //         } else {
    //             (tc / subdiv, (tc % subdiv) as f32 / subdiv as f32)
    //         };
    //         let base_c = sc * 2;
    //
    //         eval_quadratic_patch_attributes(patch, base_r, base_c, tu, tv)
    //     })
    //     .collect::<Result<Vec<_>, _>>()?;
    //
    // for (pos, uv, col) in results {
    //     positions.push(pos);
    //     uvs.push(uv);
    //     colors.push(col);
    // }

    let mut indices: Vec<u32> = Vec::with_capacity((tess_rows - 1) * (tess_cols - 1) * 6);
    for r in 0..(tess_rows - 1) {
        for c in 0..(tess_cols - 1) {
            let idx00 = (r * tess_cols + c) as u32;
            let idx01 = (r * tess_cols + (c + 1)) as u32;
            let idx10 = ((r + 1) * tess_cols + c) as u32;
            let idx11 = ((r + 1) * tess_cols + (c + 1)) as u32;
            indices.extend_from_slice(&[idx00, idx10, idx11, idx00, idx11, idx01]);
        }
    }

    // If we can compute a reference normal from the first segment, use it to pick winding.
    if let Some(ref_n) = eval_quadratic_patch_reference_normal(patch, 0, 0) {
        if let Some(first_tri_n) = first_triangle_normal(&positions, &indices) {
            if first_tri_n.dot(ref_n) < 0.0 {
                flip_triangle_winding(&mut indices);
            }
        }
    }

    // `patchDef5` surfaces are rendered double-sided in editors/game tools, while
    // `patchTerrainDef3` remains single-sided. The 3D renderer enables backface culling
    // globally, so duplicate the curve triangles with reversed winding here.
    // Compute normals from front-face indices only (before reversal) to avoid
    // processing 2x triangles — the back-face contributes the same normals.
    let normals = compute_vertex_normals(&positions, &indices);

    append_reverse_winding(&mut indices);

    Ok(PatchMesh {
        positions,
        uvs,
        colors,
        normals,
        indices,
    })
}

fn eval_quadratic_patch_attributes(
    patch: &Patch,
    base_r: usize,
    base_c: usize,
    u: f32,
    v: f32,
    c_ctrl: &[[Vec4; 3]; 3],
) -> Result<(Vec3, Vec2, [u8; 4]), GeometryError> {
    if base_r + 2 >= patch.vertices.len() || base_c + 2 >= patch.vertices[0].len() {
        return Err(GeometryError::InvalidPatch(
            "bezier control point access out of bounds".into(),
        ));
    }

    let mut p_ctrl = [[Vec3::ZERO; 3]; 3];
    let mut uv_ctrl = [[Vec2::ZERO; 3]; 3];
    // let mut c_ctrl = [[Vec4::ZERO; 3]; 3];
    for rr in 0..3 {
        for cc in 0..3 {
            let vtx = patch.vertices[base_r + rr][base_c + cc];
            p_ctrl[rr][cc] = vtx.position;
            uv_ctrl[rr][cc] = vtx.uv;
            // c_ctrl[rr][cc] = Vec4::new(
            //     vtx.color[0] as f32 / 255.0,
            //     vtx.color[1] as f32 / 255.0,
            //     vtx.color[2] as f32 / 255.0,
            //     vtx.color[3] as f32 / 255.0,
            // );
        }
    }

    let pos = bezier2_surface_vec3(&p_ctrl, u, v);
    let uv = bezier2_surface_vec2(&uv_ctrl, u, v);
    let col = bezier2_surface_vec4(&c_ctrl, u, v);
    let rgba = [
        (col.x * 255.0).round().clamp(0.0, 255.0) as u8,
        (col.y * 255.0).round().clamp(0.0, 255.0) as u8,
        (col.z * 255.0).round().clamp(0.0, 255.0) as u8,
        (col.w * 255.0).round().clamp(0.0, 255.0) as u8,
    ];
    Ok((pos, uv, rgba))
}

fn bezier2_vec3(p0: Vec3, p1: Vec3, p2: Vec3, t: f32) -> Vec3 {
    let it = 1.0 - t;
    (p0 * (it * it)) + (p1 * (2.0 * it * t)) + (p2 * (t * t))
}

fn bezier2_vec2(p0: Vec2, p1: Vec2, p2: Vec2, t: f32) -> Vec2 {
    let it = 1.0 - t;
    (p0 * (it * it)) + (p1 * (2.0 * it * t)) + (p2 * (t * t))
}

fn bezier2_vec4(p0: Vec4, p1: Vec4, p2: Vec4, t: f32) -> Vec4 {
    let it = 1.0 - t;
    (p0 * (it * it)) + (p1 * (2.0 * it * t)) + (p2 * (t * t))
}

fn bezier2_derivative_vec3(p0: Vec3, p1: Vec3, p2: Vec3, t: f32) -> Vec3 {
    // d/dt of quadratic bezier.
    let it = 1.0 - t;
    ((p1 - p0) * (2.0 * it)) + ((p2 - p1) * (2.0 * t))
}

fn bezier2_surface_vec3(ctrl: &[[Vec3; 3]; 3], u: f32, v: f32) -> Vec3 {
    let c0 = bezier2_vec3(ctrl[0][0], ctrl[0][1], ctrl[0][2], u);
    let c1 = bezier2_vec3(ctrl[1][0], ctrl[1][1], ctrl[1][2], u);
    let c2 = bezier2_vec3(ctrl[2][0], ctrl[2][1], ctrl[2][2], u);
    bezier2_vec3(c0, c1, c2, v)
}

fn bezier2_surface_vec2(ctrl: &[[Vec2; 3]; 3], u: f32, v: f32) -> Vec2 {
    let c0 = bezier2_vec2(ctrl[0][0], ctrl[0][1], ctrl[0][2], u);
    let c1 = bezier2_vec2(ctrl[1][0], ctrl[1][1], ctrl[1][2], u);
    let c2 = bezier2_vec2(ctrl[2][0], ctrl[2][1], ctrl[2][2], u);
    bezier2_vec2(c0, c1, c2, v)
}

fn bezier2_surface_vec4(ctrl: &[[Vec4; 3]; 3], u: f32, v: f32) -> Vec4 {
    let c0 = bezier2_vec4(ctrl[0][0], ctrl[0][1], ctrl[0][2], u);
    let c1 = bezier2_vec4(ctrl[1][0], ctrl[1][1], ctrl[1][2], u);
    let c2 = bezier2_vec4(ctrl[2][0], ctrl[2][1], ctrl[2][2], u);
    bezier2_vec4(c0, c1, c2, v)
}

fn eval_quadratic_patch_reference_normal(
    patch: &Patch,
    base_r: usize,
    base_c: usize,
) -> Option<Vec3> {
    if base_r + 2 >= patch.vertices.len() || base_c + 2 >= patch.vertices[0].len() {
        return None;
    }

    let mut p_ctrl = [[Vec3::ZERO; 3]; 3];
    for rr in 0..3 {
        for cc in 0..3 {
            p_ctrl[rr][cc] = patch.vertices[base_r + rr][base_c + cc].position;
        }
    }

    let u = 0.5;
    let v = 0.5;

    // du: bezier over v of per-row du.
    let d0 = bezier2_derivative_vec3(p_ctrl[0][0], p_ctrl[0][1], p_ctrl[0][2], u);
    let d1 = bezier2_derivative_vec3(p_ctrl[1][0], p_ctrl[1][1], p_ctrl[1][2], u);
    let d2 = bezier2_derivative_vec3(p_ctrl[2][0], p_ctrl[2][1], p_ctrl[2][2], u);
    let du = bezier2_vec3(d0, d1, d2, v);

    // dv: derivative over v of per-row points at u.
    let c0 = bezier2_vec3(p_ctrl[0][0], p_ctrl[0][1], p_ctrl[0][2], u);
    let c1 = bezier2_vec3(p_ctrl[1][0], p_ctrl[1][1], p_ctrl[1][2], u);
    let c2 = bezier2_vec3(p_ctrl[2][0], p_ctrl[2][1], p_ctrl[2][2], u);
    let dv = bezier2_derivative_vec3(c0, c1, c2, v);

    let n = du.cross(dv);
    if n.length_squared() < 1e-12 {
        None
    } else {
        Some(n.normalize())
    }
}

fn first_triangle_normal(positions: &[Vec3], indices: &[u32]) -> Option<Vec3> {
    for tri in indices.chunks(3) {
        if tri.len() != 3 {
            break;
        }
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }
        let p0 = positions[i0];
        let p1 = positions[i1];
        let p2 = positions[i2];
        let n = (p1 - p0).cross(p2 - p0);
        if n.length_squared() > 1e-12 {
            return Some(n.normalize());
        }
    }
    None
}

fn flip_triangle_winding(indices: &mut [u32]) {
    for tri in indices.chunks_mut(3) {
        if tri.len() == 3 {
            tri.swap(1, 2);
        }
    }
}

fn append_reverse_winding(indices: &mut Vec<u32>) {
    let len = indices.len();
    // Safety: we read indices[0..len] and append to the end.
    // reserve() ensures no reallocation, so the original data is stable.
    indices.reserve(len);
    for i in (0..len).step_by(3) {
        let a = indices[i];
        let b = indices[i + 1];
        let c = indices[i + 2];
        indices.push(a);
        indices.push(c);
        indices.push(b);
    }
}

fn compute_vertex_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let n_verts = positions.len();
    if n_verts == 0 || indices.len() < 3 {
        return vec![Vec3::Z; n_verts];
    }

    // Parallel face computation
    let face_normals: Vec<Vec3> = indices.par_chunks(3).with_min_len(64).filter_map(|tri| {
        if tri.len() != 3 { return None; }
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= n_verts || i1 >= n_verts || i2 >= n_verts { return None; }
        let n = (positions[i1] - positions[i0]).cross(positions[i2] - positions[i0]);
        if n.length_squared() < 1e-12 { None } else { Some(n) }
    }).collect();

    // Sequential accumulation into vertex buffer.
    // This is cache-friendly because each triangle's 3 vertices are adjacent in memory
    // for typical mesh layouts, and we avoid the atomic/lock overhead of parallel scatter.
    let mut normals = vec![Vec3::ZERO; n_verts];
    for (tri, &fn_) in indices.chunks(3).zip(&face_normals) {
        if tri.len() != 3 { continue; }
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= n_verts || i1 >= n_verts || i2 >= n_verts { continue; }
        normals[i0] += fn_;
        normals[i1] += fn_;
        normals[i2] += fn_;
    }

    // parallel normalization
    normals.par_iter_mut().with_min_len(64).for_each(|n| {
        if n.length_squared() < 1e-12 {
            *n = Vec3::Z;
        } else {
            *n = n.normalize();
        }
    });

    normals
}

// ========================= Tests =========================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{PatchParams, PatchVertex};
    use crate::parser::parse_map_string;

    #[test]
    fn simple_box_tesselates_to_polygons() {
        let src = include_str!("../test/simple_box.map");
        let map = parse_map_string(src).expect("parse");
        let brush = &map.entities[0].brushes[0];
        let polys = brush_to_polygons(brush).expect("tesselate");
        assert_eq!(polys.len(), 6);
        for (verts, indices) in polys {
            assert!(verts.len() >= 3);
            assert_eq!(indices.len() % 3, 0);
            assert_eq!(indices.len(), (verts.len() - 2) * 3);
        }
    }

    #[test]
    fn tesselates_all_convex_brushes_in_training_outside() {
        let src = include_str!("../test/training_outside.map");
        let map = parse_map_string(src).expect("parse");
        for ent in &map.entities {
            for brush in &ent.brushes {
                if let BrushContent::Convex(faces) = &brush.content {
                    let polys = brush_to_polygons(brush).expect("tesselate brush");
                    assert_eq!(polys.len(), faces.len());
                }
            }
        }
    }

    #[test]
    fn tesselates_terrain_patch_grid() {
        let patch = Patch::new(
            PatchType::Terrain,
            "common/caulk".into(),
            PatchParams {
                rows: 2,
                cols: 2,
                ..Default::default()
            },
            vec![
                vec![
                    PatchVertex {
                        position: Vec3::new(0.0, 0.0, 0.0),
                        uv: Vec2::new(0.0, 0.0),
                        color: [255, 255, 255, 255],
                        turned_edge: false,
                    },
                    PatchVertex {
                        position: Vec3::new(64.0, 0.0, 0.0),
                        uv: Vec2::new(1.0, 0.0),
                        color: [255, 255, 255, 255],
                        turned_edge: false,
                    },
                ],
                vec![
                    PatchVertex {
                        position: Vec3::new(0.0, 64.0, 0.0),
                        uv: Vec2::new(0.0, 1.0),
                        color: [255, 255, 255, 255],
                        turned_edge: false,
                    },
                    PatchVertex {
                        position: Vec3::new(64.0, 64.0, 0.0),
                        uv: Vec2::new(1.0, 1.0),
                        color: [255, 255, 255, 255],
                        turned_edge: false,
                    },
                ],
            ],
        );
        let mesh = tessellate_patch(&patch).expect("tessellate");
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.indices.len(), 6);
        assert_eq!(mesh.uvs.len(), 4);
        assert_eq!(mesh.colors.len(), 4);
    }

    #[test]
    fn tesselates_curve_patch_bezier() {
        let v = |x, y, z, u, v| PatchVertex {
            position: Vec3::new(x, y, z),
            uv: Vec2::new(u, v),
            color: [255, 255, 255, 255],
            turned_edge: false,
        };

        let patch = Patch::new(
            PatchType::Curve,
            "common/caulk".into(),
            PatchParams {
                rows: 3,
                cols: 3,
                subdivision: 2,
                ..Default::default()
            },
            vec![
                vec![
                    v(0.0, 0.0, 0.0, 0.0, 0.0),
                    v(64.0, 0.0, 0.0, 1.0, 0.0),
                    v(128.0, 0.0, 0.0, 2.0, 0.0),
                ],
                vec![
                    v(0.0, 64.0, 16.0, 0.0, 1.0),
                    v(64.0, 64.0, 32.0, 1.0, 1.0),
                    v(128.0, 64.0, 16.0, 2.0, 1.0),
                ],
                vec![
                    v(0.0, 128.0, 0.0, 0.0, 2.0),
                    v(64.0, 128.0, 0.0, 1.0, 2.0),
                    v(128.0, 128.0, 0.0, 2.0, 2.0),
                ],
            ],
        );

        let mesh = tessellate_patch(&patch).expect("tessellate");
        assert_eq!(mesh.positions.len(), 9); // (subdiv=2) -> 3x3 grid
        assert_eq!(mesh.indices.len(), 48); // Front + back faces for 4 quads -> 16 tris
        assert_eq!(mesh.uvs.len(), 9);
        assert_eq!(mesh.colors.len(), 9);
    }
}
