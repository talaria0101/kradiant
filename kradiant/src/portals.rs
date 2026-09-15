//! Smart portal generation for CoD 1/UO maps.
//!
//! Implements the workflow from the classic CoD portalling tutorials
//! (indoor door/window portals and outdoor cell walls):
//!
//! * A **portal brush** is a thin box whose faces are all
//!   `common/portalnodraw` except exactly ONE face textured
//!   `common/portal` — the game treats the portal as two dimensional and
//!   only recognises that "blue plane".
//! * **Indoor portals** fill doorways and windows. Given the brushes that
//!   frame an opening (jambs, lintel, threshold, surrounding wall pieces),
//!   [`detect_openings`] finds the empty rectangular gaps on the wall plane
//!   and [`generate_opening_portals`] creates a fitted portal brush for
//!   each one.
//! * **Outdoor portals** are walls between mapper-defined cells. Given the
//!   cell brushes, [`generate_cell_portal_walls`] builds one wall per pair
//!   of cells with coplanar opposing faces, spanning exactly the overlap
//!   of those faces — so a portal wall can never run along a second cell
//!   (the tutorials' T-junction rule) by construction. Collinear walls on
//!   the same plane are staggered to opposite sides of their plane
//!   (the tutorials' 4-way junction fix) so their active portal faces can
//!   never end up coplanar across a crossing wall.
//!
//! Known limitation: corners where walls of *different* cell pairs meet
//! can overlap by half a thickness each (the tutorials resolve these with
//! 45° bevels by hand). Overlaps are reported back to the caller so the
//! console can point the mapper at them.

use crate::editing::{Aabb, convex_brush_from_aabb};
use crate::map::{Brush, BrushContent, BrushId, Map};
use crate::texmap::face_plane_normal;
use crate::Vec3;

/// Textures written onto generated portal brushes.
#[derive(Debug, Clone)]
pub struct PortalTextures {
    /// The active portal plane ("blue face").
    pub portal: String,
    /// Every inactive face of a portal brush.
    pub nodraw: String,
}

impl Default for PortalTextures {
    fn default() -> Self {
        Self {
            portal: "common/portal".to_string(),
            nodraw: "common/portalnodraw".to_string(),
        }
    }
}

/// Smallest doorway/window opening (world units) that still gets a portal.
pub const MIN_OPENING_EXTENT: f32 = 24.0;
/// Smallest cell-face overlap (world units) that still gets a portal wall.
pub const MIN_WALL_OVERLAP: f32 = 24.0;
/// Default thickness of generated portal brushes (world units).
pub const DEFAULT_PORTAL_THICKNESS: f32 = 8.0;

/// Which side of an opening the active portal face points towards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalSide {
    /// Face whose normal points towards the negative axis end.
    Negative,
    /// Face whose normal points towards the positive axis end.
    Positive,
}

impl Default for PortalSide {
    fn default() -> Self {
        // The indoor tutorial textures the face facing "indoors"; there is
        // no way to detect indoors automatically, so negative is the
        // deterministic default and the caller can flip it.
        Self::Negative
    }
}

const PLANE_EPS: f32 = 0.1;
const POINT_EPS: f32 = 0.05;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Thin axis of an AABB (the wall-normal direction of a door frame, or the
/// thickness direction of a portal wall).
fn thin_axis(aabb: &Aabb) -> usize {
    let e = [aabb.max.x - aabb.min.x, aabb.max.y - aabb.min.y, aabb.max.z - aabb.min.z];
    let mut axis = 0;
    if e[1] < e[axis] {
        axis = 1;
    }
    if e[2] < e[axis] {
        axis = 2;
    }
    axis
}

/// Public wrapper for crate-internal callers (the bsp module).
pub(crate) fn aabb_of_brush_pub(brush: &Brush) -> Option<Aabb> {
    aabb_of_brush(brush)
}

fn aabb_of_brush(brush: &Brush) -> Option<Aabb> {
    match &brush.content {
        BrushContent::Patch(_) => return None,
        BrushContent::Convex(_) => {}
    }
    // Never trust `brush.aabb`: freshly parsed brushes have it zeroed until
    // geometry is first requested. Derive it from the polygons instead.
    let polys = crate::geometry::brush_to_polygons(brush).ok()?;
    Some(crate::editing::aabb_from_polys(&polys))
}

/// The two axes that span the plane with normal along `axis`.
fn plane_axes(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

fn rect_of_aabb(aabb: &Aabb, u: usize, v: usize) -> (f32, f32, f32, f32) {
    (aabb.min[u], aabb.min[v], aabb.max[u], aabb.max[v])
}

fn intervals_overlap(a0: f32, a1: f32, b0: f32, b1: f32, eps: f32) -> bool {
    a0 < b1 - eps && b0 < a1 - eps
}

// ---------------------------------------------------------------------------
// Indoor portals: doorway / window openings
// ---------------------------------------------------------------------------

/// Detect the wall-normal axis of a frame selection: the thinnest axis of
/// the selection's bounding box. Returns `None` for empty selections.
pub fn detect_wall_axis(aabbs: &[Aabb]) -> Option<usize> {
    let first = aabbs.first()?;
    let mut union = first.clone();
    for a in &aabbs[1..] {
        union.min = union.min.min(a.min);
        union.max = union.max.max(a.max);
    }
    Some(thin_axis(&union))
}

/// Find empty rectangular openings on a wall plane.
///
/// `rects` are the frame brushes projected onto the plane (`u`, `v` are the
/// two in-plane axes). Returns the empty rectangles (as `(u0, v0, u1, v1)`)
/// that are fully bounded by the frame, largest first. Rectilinear
/// components that are not rectangular (L/T shaped holes) are skipped —
/// portals are boxes, so only rectangular openings can be filled.
fn find_opening_rects(rects: &[(f32, f32, f32, f32)]) -> Vec<(f32, f32, f32, f32)> {
    if rects.is_empty() {
        return Vec::new();
    }

    // Coordinate compression.
    let mut us: Vec<f32> = Vec::new();
    let mut vs: Vec<f32> = Vec::new();
    for &(u0, v0, u1, v1) in rects {
        us.push(u0);
        us.push(u1);
        vs.push(v0);
        vs.push(v1);
    }
    us.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    us.dedup_by(|a, b| (*a - *b).abs() <= POINT_EPS);
    vs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    vs.dedup_by(|a, b| (*a - *b).abs() <= POINT_EPS);
    if us.len() < 2 || vs.len() < 2 {
        return Vec::new();
    }

    let nu = us.len() - 1;
    let nv = vs.len() - 1;
    let mut covered = vec![false; nu * nv];
    for &(u0, v0, u1, v1) in rects {
        for i in 0..nu {
            if !intervals_overlap(us[i], us[i + 1], u0, u1, POINT_EPS) {
                continue;
            }
            for j in 0..nv {
                if intervals_overlap(vs[j], vs[j + 1], v0, v1, POINT_EPS) {
                    covered[j * nu + i] = true;
                }
            }
        }
    }

    // Connected components of empty grid cells (4-connected).
    let mut component = vec![usize::MAX; nu * nv];
    let mut components: Vec<Vec<usize>> = Vec::new();
    for start in 0..nu * nv {
        if covered[start] || component[start] != usize::MAX {
            continue;
        }
        let id = components.len();
        let mut stack = vec![start];
        component[start] = id;
        let mut cells = Vec::new();
        while let Some(cell) = stack.pop() {
            cells.push(cell);
            let ci = cell % nu;
            let cj = cell / nu;
            for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let ni = ci as i32 + di;
                let nj = cj as i32 + dj;
                if ni < 0 || nj < 0 || ni as usize >= nu || nj as usize >= nv {
                    continue;
                }
                let n = nj as usize * nu + ni as usize;
                if !covered[n] && component[n] == usize::MAX {
                    component[n] = id;
                    stack.push(n);
                }
            }
        }
        components.push(cells);
    }

    // Each component must be rectangular (its bounding box fully empty).
    let mut out = Vec::new();
    for cells in &components {
        let mut i0 = usize::MAX;
        let mut i1 = 0usize;
        let mut j0 = usize::MAX;
        let mut j1 = 0usize;
        for &c in cells {
            let ci = c % nu;
            let cj = c / nu;
            i0 = i0.min(ci);
            i1 = i1.max(ci);
            j0 = j0.min(cj);
            j1 = j1.max(cj);
        }
        let rectangular = (i0..=i1).all(|i| (j0..=j1).all(|j| !covered[j * nu + i]));
        if !rectangular {
            continue;
        }
        let (u0, u1) = (us[i0], us[i1 + 1]);
        let (v0, v1) = (vs[j0], vs[j1 + 1]);
        if u1 - u0 >= MIN_OPENING_EXTENT && v1 - v0 >= MIN_OPENING_EXTENT {
            out.push((u0, v0, u1, v1));
        }
    }
    // Largest first for deterministic ordering.
    out.sort_by(|a, b| {
        let area_a = (a.2 - a.0) * (a.3 - a.1);
        let area_b = (b.2 - b.0) * (b.3 - b.1);
        area_b.partial_cmp(&area_a).unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Detect doorway/window openings framed by the given brushes.
///
/// Returns one AABB per opening, spanning the full wall thickness of the
/// frame (the thickness of the frame brushes along the wall-normal axis).
pub fn detect_openings(frame_brushes: &[&Brush]) -> Vec<Aabb> {
    let aabbs: Vec<Aabb> = frame_brushes.iter().filter_map(|b| aabb_of_brush(b)).collect();
    if aabbs.len() < 2 {
        return Vec::new();
    }
    let Some(axis) = detect_wall_axis(&aabbs) else {
        return Vec::new();
    };
    let (u, v) = plane_axes(axis);

    // The portal spans the frame's extent along the wall normal.
    let mut n_min = f32::INFINITY;
    let mut n_max = f32::NEG_INFINITY;
    for a in &aabbs {
        n_min = n_min.min(a.min[axis]);
        n_max = n_max.max(a.max[axis]);
    }

    let rects: Vec<(f32, f32, f32, f32)> =
        aabbs.iter().map(|a| rect_of_aabb(a, u, v)).collect();

    let mut out = Vec::new();
    for (u0, v0, u1, v1) in find_opening_rects(&rects) {
        let mut min = [0.0f32; 3];
        let mut max = [0.0f32; 3];
        min[axis] = n_min;
        max[axis] = n_max;
        min[u] = u0;
        max[u] = u1;
        min[v] = v0;
        max[v] = v1;
        out.push(Aabb {
            min: Vec3::new(min[0], min[1], min[2]),
            max: Vec3::new(max[0], max[1], max[2]),
        });
    }
    out
}

/// Build a portal brush filling `opening`: every face portal-nodraw except
/// the face on `side` of the wall-normal axis, which gets the portal
/// texture. The wall-normal axis is the opening's thinnest axis.
pub fn generate_opening_portal_brush(
    id: BrushId,
    opening: &Aabb,
    side: PortalSide,
    textures: &PortalTextures,
) -> Brush {
    let axis = thin_axis(opening);
    let mut brush = convex_brush_from_aabb(id, opening.clone(), &textures.nodraw);
    if let BrushContent::Convex(faces) = &mut brush.content {
        let want = match side {
            PortalSide::Negative => -1.0f32,
            PortalSide::Positive => 1.0f32,
        };
        for face in faces {
            let n = face_plane_normal(face);
            let component = n[axis];
            if component * want > 0.9 {
                face.texture = textures.portal.clone();
            }
        }
    }
    brush
}

/// Map-level indoor portals: detect openings framed by the selected brushes
/// and insert one portal brush per opening into the entity that owns the
/// first frame brush. Returns the number of portal brushes created.
pub fn generate_opening_portals(
    map: &mut Map,
    selection: &[(usize, usize)],
    side: PortalSide,
    textures: &PortalTextures,
) -> Result<usize, String> {
    if selection.is_empty() {
        return Err("select the brushes framing the opening first".to_string());
    }

    // Collect frame brushes (all must live in one entity: a doorway lives
    // inside a single entity's geometry).
    let entity_idx = selection[0].0;
    if selection.iter().any(|s| s.0 != entity_idx) {
        return Err(
            "opening frames must belong to the same entity (split the selection)"
                .to_string(),
        );
    }
    let mut frame_brushes = Vec::new();
    let mut brush_indices = Vec::new();
    for &(e, b) in selection {
        let Some(entity) = map.entities.get(e) else {
            return Err(format!("entity {e} does not exist"));
        };
        let Some(brush) = entity.brushes.get(b) else {
            return Err(format!("brush {b} does not exist on entity {e}"));
        };
        brush_indices.push(b);
        frame_brushes.push(brush);
    }

    let openings = detect_openings(&frame_brushes);
    if openings.is_empty() {
        return Err(
            "no rectangular openings found in the selection; select the brushes \
             surrounding the doorway or window (jambs, lintel, threshold)"
                .to_string(),
        );
    }

    // Insert into the owning entity with fresh brush ids.
    let entity = map
        .entities
        .get_mut(entity_idx)
        .ok_or_else(|| format!("entity {entity_idx} does not exist"))?;
    let mut next_id = entity
        .brushes
        .iter()
        .map(|b| b.id.0)
        .max()
        .map(|id| id.wrapping_add(1))
        .unwrap_or(0);

    for opening in &openings {
        let brush = generate_opening_portal_brush(BrushId(next_id), opening, side, textures);
        next_id += 1;
        entity.brushes.push(brush);
    }
    map.generation = map.generation.wrapping_add(1);

    Ok(openings.len())
}

// ---------------------------------------------------------------------------
// Outdoor portals: cell walls
// ---------------------------------------------------------------------------

/// One generated portal wall: its slab (AABB) and its wall-normal axis.
#[derive(Debug, Clone)]
pub struct PortalWall {
    pub aabb: Aabb,
    pub axis: usize,
}

/// Generate portal walls between every pair of selected cell brushes that
/// have coplanar opposing faces. Each wall spans exactly the overlap of the
/// two faces (the T-junction rule), is `thickness` thick and centred on the
/// shared plane. Collinear walls sharing a plane are staggered to opposite
/// sides of the plane so their active portal faces can never be coplanar
/// across a crossing wall (the 4-way junction rule).
///
/// Returns the walls (as slabs) plus the number of corner overlaps that
/// would need manual bevels.
pub fn plan_cell_walls(
    cells: &[&Brush],
    thickness: f32,
) -> Result<(Vec<PortalWall>, usize), String> {
    if cells.len() < 2 {
        return Err("select at least two cell brushes".to_string());
    }

    let polys: Vec<Vec<(Vec<Vec3>, Vec<u32>)>> = cells
        .iter()
        .map(|b| {
            crate::geometry::brush_to_polygons(b)
                .map_err(|e| format!("cell brush geometry is invalid: {e:?}"))
        })
        .collect::<Result<_, _>>()?;

    let mut walls: Vec<PortalWall> = Vec::new();

    // Cheap pair pre-filter: a coplanar opposing face pair implies both
    // AABBs contain the shared plane on the normal axis and overlap on the
    // plane axes - so plain AABB triple-overlap never skips a valid pair,
    // while real maps skip the vast majority of the O(n^2) pairs.
    let aabbs: Vec<Option<Aabb>> = cells
        .iter()
        .map(|b| {
            crate::geometry::brush_to_polygons(b)
                .ok()
                .map(|polys| crate::editing::aabb_from_polys(&polys))
        })
        .collect();

    for a in 0..cells.len() {
        let Some(aabb_a) = &aabbs[a] else {
            continue;
        };
        for b in (a + 1)..cells.len() {
            let Some(aabb_b) = &aabbs[b] else {
                continue;
            };
            if aabb_a.min.x > aabb_b.max.x
                || aabb_b.min.x > aabb_a.max.x
                || aabb_a.min.y > aabb_b.max.y
                || aabb_b.min.y > aabb_a.max.y
                || aabb_a.min.z > aabb_b.max.z
                || aabb_b.min.z > aabb_a.max.z
            {
                continue;
            }
            // Every coplanar opposing face pair contributes one wall.
            for (fa, poly_a) in polys[a].iter().enumerate() {
                let na = face_plane_normal(&cells[a].get_face(fa).ok_or("bad face")?);
                for (fb, poly_b) in polys[b].iter().enumerate() {
                    let nb = face_plane_normal(&cells[b].get_face(fb).ok_or("bad face")?);
                    // Opposing normals, same plane.
                    if na.dot(nb) > -0.999 {
                        continue;
                    }
                    let da = na.dot(poly_a.0.first().copied().unwrap_or(Vec3::ZERO));
                    let db = na.dot(poly_b.0.first().copied().unwrap_or(Vec3::ZERO));
                    if (da - db).abs() > PLANE_EPS {
                        continue;
                    }

                    // Overlap of the two faces on the plane.
                    let axis = if na.x.abs() >= na.y.abs() && na.x.abs() >= na.z.abs() {
                        0
                    } else if na.y.abs() >= na.z.abs() {
                        1
                    } else {
                        2
                    };
                    let (u, v) = plane_axes(axis);
                    let ra = face_rect(poly_a.0.as_slice(), u, v);
                    let rb = face_rect(poly_b.0.as_slice(), u, v);
                    let Some((u0, v0, u1, v1)) = rect_intersection(ra, rb, MIN_WALL_OVERLAP)
                    else {
                        continue;
                    };

                    // Plane position along the axis: the offset da is signed
                    // along the (inward) normal, which may point towards
                    // either end of the axis — divide it out.
                    let plane_pos = da / na[axis];
                    let half = thickness * 0.5;

                    let mut min = [0.0f32; 3];
                    let mut max = [0.0f32; 3];
                    min[axis] = plane_pos - half;
                    max[axis] = plane_pos + half;
                    min[u] = u0;
                    max[u] = u1;
                    min[v] = v0;
                    max[v] = v1;
                    walls.push(PortalWall {
                        aabb: Aabb {
                            min: Vec3::new(min[0], min[1], min[2]),
                            max: Vec3::new(max[0], max[1], max[2]),
                        },
                        axis,
                    });
                }
            }
        }
    }

    // Stagger collinear walls (same plane, same normal): alternate walls get
    // displaced +/- half thickness along the normal so the active portal
    // faces never end up coplanar across a crossing wall.
    // Deterministic order: axis, plane offset, span start. Keeps collinear
    // walls adjacent so the stagger pass can group them.
    walls.sort_by(|w1, w2| {
        w1.axis
            .cmp(&w2.axis)
            .then_with(|| w1.aabb.min[w1.axis].total_cmp(&w2.aabb.min[w2.axis]))
            .then_with(|| {
                let (u, v) = plane_axes(w1.axis);
                w1.aabb.min[u]
                    .total_cmp(&w2.aabb.min[u])
                    .then_with(|| w1.aabb.min[v].total_cmp(&w2.aabb.min[v]))
            })
            .then_with(|| w1.aabb.min.z.total_cmp(&w2.aabb.min.z))
    });
    let mut i = 0usize;
    while i < walls.len() {
        let mut j = i + 1;
        while j < walls.len() && walls_collinear(&walls[i], &walls[j], thickness) {
            j += 1;
        }
        let group_len = j - i;
        if group_len > 1 {
            let half = thickness * 0.5;
            for (k, wall) in walls[i..j].iter_mut().enumerate() {
                let offset = if k % 2 == 0 { -half } else { half };
                wall.aabb.min[wall.axis] += offset;
                wall.aabb.max[wall.axis] += offset;
            }
        }
        i = j;
    }

    // Count corner overlaps between walls of different planes (the mapper
    // has to bevel these by hand, as in the tutorial).
    let mut overlaps = 0usize;
    for a in 0..walls.len() {
        for b in (a + 1)..walls.len() {
            if walls[a].axis == walls[b].axis {
                continue;
            }
            if aabb_overlap_volume(&walls[a].aabb, &walls[b].aabb) > 1.0 {
                overlaps += 1;
            }
        }
    }

    Ok((walls, overlaps))
}

fn face_rect(verts: &[Vec3], u: usize, v: usize) -> (f32, f32, f32, f32) {
    let mut u0 = f32::INFINITY;
    let mut v0 = f32::INFINITY;
    let mut u1 = f32::NEG_INFINITY;
    let mut v1 = f32::NEG_INFINITY;
    for p in verts {
        u0 = u0.min(p[u]);
        v0 = v0.min(p[v]);
        u1 = u1.max(p[u]);
        v1 = v1.max(p[v]);
    }
    (u0, v0, u1, v1)
}

fn rect_intersection(
    a: (f32, f32, f32, f32),
    b: (f32, f32, f32, f32),
    min_extent: f32,
) -> Option<(f32, f32, f32, f32)> {
    let u0 = a.0.max(b.0);
    let v0 = a.1.max(b.1);
    let u1 = a.2.min(b.2);
    let v1 = a.3.min(b.3);
    if u1 - u0 >= min_extent && v1 - v0 >= min_extent {
        Some((u0, v0, u1, v1))
    } else {
        None
    }
}

/// Collinear = same wall-normal axis, same plane offset, and the slabs'
/// spans along the plane actually could interact (their bounding ranges on
/// at least one in-plane axis overlap).
fn walls_collinear(a: &PortalWall, b: &PortalWall, thickness: f32) -> bool {
    if a.axis != b.axis {
        return false;
    }
    if (a.aabb.min[a.axis] - b.aabb.min[a.axis]).abs() > thickness {
        return false;
    }
    let (u, v) = plane_axes(a.axis);
    intervals_overlap(
        a.aabb.min[u],
        a.aabb.max[u],
        b.aabb.min[u],
        b.aabb.max[u],
        POINT_EPS,
    ) || intervals_overlap(
        a.aabb.min[v],
        a.aabb.max[v],
        b.aabb.min[v],
        b.aabb.max[v],
        POINT_EPS,
    )
}

fn aabb_overlap_volume(a: &Aabb, b: &Aabb) -> f32 {
    let dx = (a.max.x.min(b.max.x) - a.min.x.max(b.min.x)).max(0.0);
    let dy = (a.max.y.min(b.max.y) - a.min.y.max(b.min.y)).max(0.0);
    let dz = (a.max.z.min(b.max.z) - a.min.z.max(b.min.z)).max(0.0);
    dx * dy * dz
}

/// Union coplanar placed portals that overlap or come within `gap` world
/// units of each other on their shared plane, replacing the old brushes in
/// `map` with one unioned brush per merged group. Returns how many portals
/// were consolidated away.
pub(crate) fn merge_placed_portals_public(
    map: &mut Map,
    placed: &mut Vec<PlacedPortal>,
    gap: f32,
    side: PortalSide,
    textures: &PortalTextures,
) -> usize {
    merge_placed_portals(map, placed, gap, side, textures)
}

fn merge_placed_portals(
    map: &mut Map,
    placed: &mut Vec<PlacedPortal>,
    gap: f32,
    side: PortalSide,
    textures: &PortalTextures,
) -> usize {
    if placed.len() < 2 {
        return 0;
    }

    // Group by (thin axis, quantized plane position).
    let axis_of = |p: &PlacedPortal| thin_axis(&p.aabb);
    let plane_of = |p: &PlacedPortal| -> (usize, i64) {
        let axis = axis_of(p);
        let mid = (p.aabb.min[axis] + p.aabb.max[axis]) * 0.5;
        (axis, (mid / PLANE_QUANT).round() as i64)
    };
    let mut groups: std::collections::HashMap<(usize, i64), Vec<usize>> =
        std::collections::HashMap::new();
    for (i, p) in placed.iter().enumerate() {
        groups.entry(plane_of(p)).or_default().push(i);
    }

    // Fixpoint union inside each coplanar group.
    let mut merged: Vec<Vec<usize>> = Vec::new();
    for (_, members) in groups {
        let mut sets: Vec<Vec<usize>> = members.iter().map(|&i| vec![i]).collect();
        loop {
            let mut changed = false;
            'outer: for a in 0..sets.len() {
                for b in (a + 1)..sets.len() {
                    let (au0, av0, au1, av1) = {
                        let (u, v) = plane_axes(axis_of(&placed[sets[a][0]]));
                        let mut r = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                        for &i in &sets[a] {
                            r.0 = r.0.min(placed[i].aabb.min[u]);
                            r.1 = r.1.min(placed[i].aabb.min[v]);
                            r.2 = r.2.max(placed[i].aabb.max[u]);
                            r.3 = r.3.max(placed[i].aabb.max[v]);
                        }
                        r
                    };
                    let (bu0, bv0, bu1, bv1) = {
                        let (u, v) = plane_axes(axis_of(&placed[sets[b][0]]));
                        let mut r = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                        for &i in &sets[b] {
                            r.0 = r.0.min(placed[i].aabb.min[u]);
                            r.1 = r.1.min(placed[i].aabb.min[v]);
                            r.2 = r.2.max(placed[i].aabb.max[u]);
                            r.3 = r.3.max(placed[i].aabb.max[v]);
                        }
                        r
                    };
                    let near = au0 - gap <= bu1
                        && bu0 - gap <= au1
                        && av0 - gap <= bv1
                        && bv0 - gap <= av1;
                    if near {
                        let mut union = sets[a].clone();
                        union.extend(sets[b].iter().copied());
                        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                        sets[lo] = union;
                        sets.swap_remove(hi);
                        changed = true;
                        break 'outer;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        merged.extend(sets);
    }

    let removed = placed.len() - merged.len();
    if removed == 0 {
        return 0;
    }

    // Replace brushes: drop every placed portal, insert the unions.
    for p in placed.iter() {
        if let Some(entity) = map.entities.get_mut(p.entity) {
            entity.brushes.retain(|b| b.id != p.id);
        }
    }
    let mut next: Vec<PlacedPortal> = Vec::with_capacity(merged.len());
    for group in &merged {
        let first = group[0];
        let mut aabb = placed[first].aabb;
        for &i in group.iter().skip(1) {
            aabb.min = aabb.min.min(placed[i].aabb.min);
            aabb.max = aabb.max.max(placed[i].aabb.max);
        }
        let entity_idx = placed[first].entity;
        let Some(entity) = map.entities.get_mut(entity_idx) else {
            continue;
        };
        let next_id = entity
            .brushes
            .iter()
            .map(|b| b.id.0)
            .max()
            .map(|id| id.wrapping_add(1))
            .unwrap_or(0);
        let id = BrushId(next_id);
        let brush = generate_opening_portal_brush(id, &aabb, side, textures);
        entity.brushes.push(brush);
        next.push(PlacedPortal {
            entity: entity_idx,
            id,
            aabb,
        });
    }
    *placed = next;
    removed
}

/// Build the actual portal brush for a wall slab: every face portal-nodraw
/// except the face pointing towards `side` along the wall-normal axis.
pub fn generate_cell_wall_brush(
    id: BrushId,
    wall: &PortalWall,
    side: PortalSide,
    textures: &PortalTextures,
) -> Brush {
    generate_opening_portal_brush(id, &wall.aabb, side, textures)
}

/// Map-level outdoor portals: generate portal walls between the selected
/// cell brushes and insert them into worldspawn (cell walls are world
/// geometry). Returns the number of walls created and the number of corner
/// overlaps that need manual bevels.
pub fn generate_cell_portal_walls(
    map: &mut Map,
    selection: &[(usize, usize)],
    side: PortalSide,
    textures: &PortalTextures,
    thickness: f32,
) -> Result<(usize, usize), String> {
    let mut cells = Vec::new();
    for &(e, b) in selection {
        let Some(entity) = map.entities.get(e) else {
            return Err(format!("entity {e} does not exist"));
        };
        let Some(brush) = entity.brushes.get(b) else {
            return Err(format!("brush {b} does not exist on entity {e}"));
        };
        if matches!(brush.content, BrushContent::Patch(_)) {
            continue;
        }
        cells.push(brush);
    }

    let (walls, overlaps) = plan_cell_walls(&cells, thickness)?;
    if walls.is_empty() {
        return Err(
            "no coplanar opposing faces found between the selected cell brushes"
                .to_string(),
        );
    }

    // Insert into worldspawn (cell walls separate world cells).
    let entity = map
        .entities
        .first_mut()
        .ok_or_else(|| "map has no worldspawn".to_string())?;
    let mut next_id = entity
        .brushes
        .iter()
        .map(|b| b.id.0)
        .max()
        .map(|id| id.wrapping_add(1))
        .unwrap_or(0);

    for wall in &walls {
        let brush = generate_cell_wall_brush(BrushId(next_id), wall, side, textures);
        next_id += 1;
        entity.brushes.push(brush);
    }
    map.generation = map.generation.wrapping_add(1);

    Ok((walls.len(), overlaps))
}

// ---------------------------------------------------------------------------
// Whole-map automatic opening detection
// ---------------------------------------------------------------------------

/// How many distinct grid coordinates a plane cluster may span before it is
/// skipped (keeps the compressed grid bounded on huge open planes).
const CLUSTER_MAX_COORDS: usize = 512;
/// Plane positions are quantized to this grid before scanning.
const PLANE_QUANT: f32 = 0.25;
/// A brush must span the plane with at least this margin on both sides to
/// count as crossing it (walls thinner than 2 units are not scan targets).
const CROSS_MARGIN: f32 = 1.0;
/// Coplanar generated portals whose plane projections come within this gap
/// are unioned into one brush (the originals over-cover through trim and
/// let the compiler trim the portal to the enclosed volume).
pub(crate) const PORTAL_MERGE_GAP: f32 = 16.0;

/// A generated portal brush, tracked so the merge pass can replace it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlacedPortal {
    pub(crate) entity: usize,
    pub(crate) id: BrushId,
    pub(crate) aabb: Aabb,
}

/// Snap ascending, deduplicated grid coordinates DOWN onto a lattice of
/// `snap` world units (`snap <= 0` returns them unchanged).
fn snap_coords(coords: &[f32], snap: f32) -> Vec<f32> {
    if snap <= 0.0 {
        return coords.to_vec();
    }
    let mut out: Vec<f32> = coords.iter().map(|c| (c / snap).floor() * snap).collect();
    out.dedup_by(|a, b| (*a - *b).abs() <= POINT_EPS);
    out
}

/// Result of [`generate_auto_opening_portals`].
#[derive(Debug, Clone, Copy, Default)]
pub struct AutoOpeningReport {
    pub planes_scanned: usize,
    pub clusters_scanned: usize,
    pub clusters_skipped_oversize: usize,
    pub portals_created: usize,
    /// Portals consolidated away by the coplanar merge pass.
    pub portals_merged: usize,
}

/// Scan every wall plane of the map for rectangular holes (doorways,
/// windows, courtyard openings) and insert a portal brush into each one.
///
/// This is the fully automatic counterpart to
/// [`generate_opening_portals`]: candidate planes come from thin-brush
/// mid-planes and quantized hull extents; brushes spanning a plane are
/// clustered by on-plane face-rectangle overlap; each cluster's empty
/// rectangular regions become portal brushes spanning the wall's median
/// thickness, centred on the plane.
///
/// Coverage uses the brush faces lying in the plane band (not brush AABBs),
/// so sloped/arched geometry does not falsely cover openings.
///
/// Skipped: clusters whose compressed grid exceeds [`CLUSTER_MAX_COORDS`]
/// (huge open planes), clusters with fewer than two crossing brushes,
/// openings smaller than [`MIN_OPENING_EXTENT`], and openings overlapping
/// an already created portal.
pub fn generate_auto_opening_portals(
    map: &mut Map,
    side: PortalSide,
    textures: &PortalTextures,
) -> Result<AutoOpeningReport, String> {
    struct BrushEntry {
        entity_idx: usize,
        brush: Brush,
        aabb: Aabb,
    }
    let brushes: Vec<BrushEntry> = map
        .entities
        .iter()
        .enumerate()
        .flat_map(|(e, entity)| {
            entity.brushes.iter().filter_map(move |b| {
                if matches!(b.content, BrushContent::Patch(_)) {
                    return None;
                }
                let mut owned = b.clone();
                owned.invalidate_geometry();
                let polys = crate::geometry::brush_to_polygons(&owned).ok()?;
                let aabb = crate::editing::aabb_from_polys(&polys);
                Some(BrushEntry {
                    entity_idx: e,
                    brush: owned,
                    aabb,
                })
            })
        })
        .collect();

    let mut report = AutoOpeningReport::default();
    let mut created: Vec<PlacedPortal> = Vec::new();

    for axis in 0..3usize {
        let (u, v) = plane_axes(axis);

        // Candidate planes: mid-planes of wall-like thin brushes (so every
        // wall is scanned through its middle) plus quantized hull extents.
        let mut planes: Vec<i64> = Vec::new();
        for b in &brushes {
            let ext = b.aabb.max[axis] - b.aabb.min[axis];
            if ext <= 64.0 {
                planes.push(
                    ((b.aabb.min[axis] + b.aabb.max[axis]) * 0.5 / PLANE_QUANT).round() as i64,
                );
            }
            for p in [b.aabb.min[axis], b.aabb.max[axis]] {
                planes.push((p / PLANE_QUANT).round() as i64);
            }
        }
        planes.sort_unstable();
        planes.dedup();
        report.planes_scanned += planes.len();

        for plane_q in planes {
            let p = plane_q as f32 * PLANE_QUANT;

            // Brushes spanning the plane, with the rects of the faces lying
            // in the plane band (exact coverage, no AABB overreach).
            struct Crosser<'a> {
                entity_idx: usize,
                thickness: f32,
                face_rects: Vec<(f32, f32, f32, f32)>,
                _brush: &'a Brush,
            }
            let mut crossers: Vec<Crosser> = Vec::new();
            for b in &brushes {
                if b.aabb.min[axis] > p - CROSS_MARGIN || b.aabb.max[axis] < p + CROSS_MARGIN {
                    continue;
                }
                let Ok(polys) = crate::geometry::brush_to_polygons(&b.brush) else {
                    continue;
                };
                let mut face_rects: Vec<(f32, f32, f32, f32)> = Vec::new();
                for (verts, _indices) in &polys {
                    if verts.len() < 3 {
                        continue;
                    }
                    let e1 = verts[1] - verts[0];
                    let e2 = verts[2] - verts[0];
                    let n = e1.cross(e2);
                    if n.length_squared() < 1.0e-12 {
                        continue;
                    }
                    let n = n.normalize();
                    if n[axis].abs() < 0.999 {
                        continue; // not parallel to the scan plane
                    }
                    // The brush already crosses the plane, so every face of
                    // it that is parallel to the plane is one of the wall's
                    // two surfaces - no matter how thick the wall is. (A
                    // fixed band around the plane silently dropped every
                    // face of walls thicker than twice the band, which made
                    // whole buildings invisible to the scanner.)
                    face_rects.push(face_rect(verts, u, v));
                }
                if face_rects.is_empty() {
                    continue;
                }
                crossers.push(Crosser {
                    entity_idx: b.entity_idx,
                    thickness: b.aabb.max[axis] - b.aabb.min[axis],
                    face_rects,
                    _brush: &b.brush,
                });
            }
            if crossers.len() < 2 {
                continue;
            }

            // Cluster crossers by face-rect overlap (union-find).
            let n = crossers.len();
            let mut parent: Vec<usize> = (0..n).collect();
            fn find(parent: &mut Vec<usize>, mut i: usize) -> usize {
                while parent[i] != i {
                    parent[i] = parent[parent[i]];
                    i = parent[i];
                }
                i
            }
            for i in 0..n {
                for j in (i + 1)..n {
                    let overlap = crossers[i].face_rects.iter().any(|r1| {
                        crossers[j].face_rects.iter().any(|r2| {
                            intervals_overlap(r1.0, r1.2, r2.0, r2.2, POINT_EPS)
                                && intervals_overlap(r1.1, r1.3, r2.1, r2.3, POINT_EPS)
                        })
                    });
                    if overlap {
                        let ri = find(&mut parent, i);
                        let rj = find(&mut parent, j);
                        if ri != rj {
                            parent[ri] = rj;
                        }
                    }
                }
            }
            let mut groups: std::collections::HashMap<usize, Vec<usize>> =
                std::collections::HashMap::new();
            for i in 0..n {
                groups.entry(find(&mut parent, i)).or_default().push(i);
            }
            let mut groups: Vec<Vec<usize>> = groups.into_values().collect();
            groups.sort_by_key(|g| g.first().copied().unwrap_or(0));

            for group in &groups {
                report.clusters_scanned += 1;
                if group.len() < 2 {
                    continue;
                }

                // Compressed grid over all cluster face rects.
                let mut us: Vec<f32> = Vec::new();
                let mut vs: Vec<f32> = Vec::new();
                for &i in group {
                    for r in &crossers[i].face_rects {
                        us.push(r.0);
                        us.push(r.2);
                        vs.push(r.1);
                        vs.push(r.3);
                    }
                }
                us.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                us.dedup_by(|a, b| (*a - *b).abs() <= POINT_EPS);
                vs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                vs.dedup_by(|a, b| (*a - *b).abs() <= POINT_EPS);
                if us.len() < 2 || vs.len() < 2 {
                    continue;
                }

                // Long walls (whole rowhouses, city blocks) gather hundreds
                // of distinct face coordinates. Rather than skipping the
                // cluster wholesale, snap the grid lines to a coarser lattice
                // until it fits. Snapping rounds coordinates DOWN, so grid
                // cells grow and coverage shrinks slightly: openings get a
                // little larger, never closed off. Trim narrower than the
                // snap vanishes into the opening, which matches how the
                // originals over-cover and let the compiler trim.
                let mut snap = 0.0f32;
                loop {
                    if us.len() <= CLUSTER_MAX_COORDS && vs.len() <= CLUSTER_MAX_COORDS {
                        break;
                    }
                    snap = if snap == 0.0 { 1.0 } else { snap * 2.0 };
                    if snap > 64.0 {
                        break;
                    }
                    us = snap_coords(&us, snap);
                    vs = snap_coords(&vs, snap);
                }
                if us.len() > CLUSTER_MAX_COORDS || vs.len() > CLUSTER_MAX_COORDS {
                    report.clusters_skipped_oversize += 1;
                    continue;
                }

                let nu = us.len() - 1;
                let nv = vs.len() - 1;
                let mut covered = vec![false; nu * nv];
                for &i in group {
                    for r in &crossers[i].face_rects {
                        for gi in 0..nu {
                            if !intervals_overlap(us[gi], us[gi + 1], r.0, r.2, POINT_EPS) {
                                continue;
                            }
                            for gj in 0..nv {
                                if intervals_overlap(vs[gj], vs[gj + 1], r.1, r.3, POINT_EPS) {
                                    covered[gj * nu + gi] = true;
                                }
                            }
                        }
                    }
                }

                // Empty components become openings. Real openings are
                // rarely rectangular (stepped arches, trim, abutting
                // geometry), so each component is decomposed greedily into
                // maximal rectangles and every rectangle big enough becomes
                // a portal brush.
                let mut component = covered.clone();
                loop {
                    // Largest rectangle of full rows: histogram method.
                    let mut heights = vec![0usize; nu];
                    let mut best: Option<(usize, usize, usize, usize, u64)> = None;
                    for j in 0..nv {
                        for i in 0..nu {
                            heights[i] = if component[j * nu + i] {
                                heights[i] + 1
                            } else {
                                0
                            };
                        }
                        let mut stack: Vec<usize> = Vec::new();
                        let mut i = 0usize;
                        while i <= nu {
                            let h = if i < nu { heights[i] } else { 0 };
                            if stack.is_empty() || h >= heights[*stack.last().unwrap()] {
                                stack.push(i);
                                i += 1;
                            } else {
                                let top = stack.pop().unwrap();
                                let left = stack.last().copied().unwrap_or(0);
                                let width = i - left;
                                let height = heights[top];
                                if height > 0 {
                                    let area = width as u64 * height as u64;
                                    if best.as_ref().map_or(true, |b| area > b.4) {
                                        best = Some((left, j + 1 - height, i - 1, j, area));
                                    }
                                }
                            }
                        }
                    }
                    let Some((bi0, bj0, bi1, bj1, area)) = best else {
                        break;
                    };
                    if area == 0 {
                        break;
                    }
                    let ru0 = us[bi0];
                    let ru1 = us[bi1 + 1];
                    let rv0 = vs[bj0];
                    let rv1 = vs[bj1 + 1];
                    if ru1 - ru0 < MIN_OPENING_EXTENT || rv1 - rv0 < MIN_OPENING_EXTENT {
                        // Everything left is smaller than the minimum.
                        break;
                    }
                    for j in bj0..=bj1 {
                        for i in bi0..=bi1 {
                            component[j * nu + i] = false;
                        }
                    }

                    // Wall thickness: median extent of the cluster's crossing
                    // brushes along the wall axis.
                    let mut extents: Vec<f32> =
                        group.iter().map(|&i| crossers[i].thickness).collect();
                    extents.sort_by(|a, b| a.total_cmp(b));
                    let thickness = extents[extents.len() / 2].clamp(2.0, 16.0);
                    let half = thickness * 0.5;

                    let mut min = [0.0f32; 3];
                    let mut max = [0.0f32; 3];
                    min[axis] = p - half;
                    max[axis] = p + half;
                    min[u] = ru0;
                    max[u] = ru1;
                    min[v] = rv0;
                    max[v] = rv1;
                    let aabb = Aabb {
                        min: Vec3::new(min[0], min[1], min[2]),
                        max: Vec3::new(max[0], max[1], max[2]),
                    };

                    if created
                        .iter()
                        .any(|c| aabb_overlap_volume(&aabb, &c.aabb) > 1.0)
                    {
                        continue;
                    }

                    let entity_idx = crossers[group[0]].entity_idx;
                    let Some(entity) = map.entities.get_mut(entity_idx) else {
                        continue;
                    };
                    let next_id = entity
                        .brushes
                        .iter()
                        .map(|b| b.id.0)
                        .max()
                        .map(|id| id.wrapping_add(1))
                        .unwrap_or(0);
                    let id = BrushId(next_id);
                    let brush = generate_opening_portal_brush(id, &aabb, side, textures);
                    entity.brushes.push(brush);
                    created.push(PlacedPortal {
                        entity: entity_idx,
                        id,
                        aabb,
                    });
                    report.portals_created += 1;
                }
            }
        }
    }

    // The originals over-cover on purpose: one box through transoms, arch
    // steps and trim, letting the compiler trim the portal to the enclosed
    // volume. Union coplanar portals that touch or sit within a small gap
    // of each other, so trim between windows does not fragment the result.
    report.portals_merged =
        merge_placed_portals(map, &mut created, PORTAL_MERGE_GAP, side, textures);
    report.portals_created = created.len();

    Ok(report)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::editing::convex_brush_from_aabb;
    use crate::map::BrushId;

    fn box_brush(min: (f32, f32, f32), max: (f32, f32, f32)) -> Brush {
        convex_brush_from_aabb(
            BrushId(0),
            Aabb::from_points(
                Vec3::new(min.0, min.1, min.2),
                Vec3::new(max.0, max.1, max.2),
            ),
            "common/caulk",
        )
    }

    fn count_faces_with_texture(brush: &Brush, texture: &str) -> usize {
        match &brush.content {
            BrushContent::Convex(faces) => faces
                .iter()
                .filter(|f| f.texture == texture)
                .count(),
            _ => 0,
        }
    }

    fn face_normals_with_texture(brush: &Brush, texture: &str) -> Vec<Vec3> {
        match &brush.content {
            BrushContent::Convex(faces) => faces
                .iter()
                .filter(|f| f.texture == texture)
                .map(face_plane_normal)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Classic doorway: two jambs (y 0..8 and 32..40), a lintel (z 80..88)
    /// spanning between them, all thin in X (wall along the YZ plane).
    /// Brush ids are unique per entity, like the real map loader produces.
    fn door_frame() -> Vec<Brush> {
        let mut brushes = vec![
            box_brush((0.0, 0.0, 0.0), (8.0, 8.0, 88.0)),   // left jamb
            box_brush((0.0, 32.0, 0.0), (8.0, 40.0, 88.0)), // right jamb
            box_brush((0.0, 0.0, 80.0), (8.0, 40.0, 88.0)), // lintel
        ];
        for (id, brush) in brushes.iter_mut().enumerate() {
            brush.id = BrushId(id as u32);
        }
        brushes
    }

    #[test]
    fn wall_axis_is_the_thinnest_selection_axis() {
        let frame = door_frame();
        let refs: Vec<&Brush> = frame.iter().collect();
        let aabbs: Vec<Aabb> = refs.iter().map(|b| aabb_of_brush(b).unwrap()).collect();
        assert_eq!(detect_wall_axis(&aabbs), Some(0), "frame is thin in X");
    }

    #[test]
    fn detects_a_single_doorway_opening() {
        let frame = door_frame();
        let refs: Vec<&Brush> = frame.iter().collect();
        let openings = detect_openings(&refs);

        assert_eq!(openings.len(), 1, "one doorway");
        let o = &openings[0];
        // Opening spans between the jambs, floor to lintel, wall thickness.
        assert!((o.min.y - 8.0).abs() < POINT_EPS);
        assert!((o.max.y - 32.0).abs() < POINT_EPS);
        assert!((o.min.z - 0.0).abs() < POINT_EPS);
        assert!((o.max.z - 80.0).abs() < POINT_EPS);
        assert!((o.min.x - 0.0).abs() < POINT_EPS);
        assert!((o.max.x - 8.0).abs() < POINT_EPS);
    }

    #[test]
    fn detects_multiple_openings_in_one_wall() {
        // Wall strip with two windows: sill, lintel and three piers.
        let frame = vec![
            box_brush((0.0, 0.0, 0.0), (8.0, 16.0, 128.0)),    // pier 1
            box_brush((0.0, 48.0, 0.0), (8.0, 64.0, 128.0)),   // pier 2
            box_brush((0.0, 96.0, 0.0), (8.0, 112.0, 128.0)),  // pier 3
            box_brush((0.0, 0.0, 0.0), (8.0, 112.0, 16.0)),    // sill strip
            box_brush((0.0, 0.0, 96.0), (8.0, 112.0, 112.0)),  // lintel strip
        ];
        let refs: Vec<&Brush> = frame.iter().collect();
        let openings = detect_openings(&refs);

        assert_eq!(openings.len(), 2, "two window openings");
        for o in &openings {
            assert!((o.min.z - 16.0).abs() < POINT_EPS);
            assert!((o.max.z - 96.0).abs() < POINT_EPS);
            assert!((o.max.x - o.min.x - 8.0).abs() < POINT_EPS);
        }
        // Windows span pier1..pier2 and pier2..pier3.
        let mut spans: Vec<(f32, f32)> = openings
            .iter()
            .map(|o| (o.min.y, o.max.y))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert!((spans[0].0 - 16.0).abs() < POINT_EPS && (spans[0].1 - 48.0).abs() < POINT_EPS);
        assert!((spans[1].0 - 64.0).abs() < POINT_EPS && (spans[1].1 - 96.0).abs() < POINT_EPS);
    }

    #[test]
    fn solid_wall_selection_has_no_openings() {
        let frame = vec![
            box_brush((0.0, 0.0, 0.0), (8.0, 32.0, 64.0)),
            box_brush((0.0, 32.0, 0.0), (8.0, 64.0, 64.0)),
        ];
        let refs: Vec<&Brush> = frame.iter().collect();
        assert!(detect_openings(&refs).is_empty(), "no gap -> no openings");
    }

    #[test]
    fn opening_portal_brush_has_one_portal_face_on_the_chosen_side() {
        let opening = Aabb::from_points(Vec3::new(0.0, 8.0, 0.0), Vec3::new(8.0, 32.0, 80.0));
        let textures = PortalTextures::default();

        let brush = generate_opening_portal_brush(BrushId(0), &opening, PortalSide::Negative, &textures);
        assert_eq!(count_faces_with_texture(&brush, "common/portal"), 1);
        assert_eq!(count_faces_with_texture(&brush, "common/portalnodraw"), 5);
        let normals = face_normals_with_texture(&brush, "common/portal");
        assert!(
            normals[0].x < -0.9,
            "negative side face gets the portal texture: {normals:?}"
        );

        let brush = generate_opening_portal_brush(BrushId(0), &opening, PortalSide::Positive, &textures);
        let normals = face_normals_with_texture(&brush, "common/portal");
        assert!(normals[0].x > 0.9, "positive side: {normals:?}");
    }

    #[test]
    fn opening_portals_are_inserted_into_the_owning_entity() {
        let mut map = Map::default();
        map.entities.push(crate::map::Entity {
            id: crate::map::EntityId(0),
            classname: "worldspawn".to_string(),
            properties: Default::default(),
            brushes: door_frame(),
            model: None,
        });

        let selection = vec![(0usize, 0usize), (0, 1), (0, 2)];
        let created =
            generate_opening_portals(&mut map, &selection, PortalSide::Negative, &PortalTextures::default())
                .expect("doorway must generate");
        assert_eq!(created, 1);

        let entity = &map.entities[0];
        assert_eq!(entity.brushes.len(), 4, "3 frame brushes + 1 portal");
        let portal = &entity.brushes[3];
        assert_eq!(portal.id.0, 3, "fresh brush id");
        assert_eq!(count_faces_with_texture(portal, "common/portal"), 1);
        assert!(map.generation > 0);
    }

    #[test]
    fn opening_generation_rejects_cross_entity_and_empty_selections() {
        let mut map = Map::default();
        map.entities.push(crate::map::Entity {
            id: crate::map::EntityId(0),
            classname: "worldspawn".to_string(),
            properties: Default::default(),
            brushes: door_frame(),
            model: None,
        });
        map.entities.push(crate::map::Entity {
            id: crate::map::EntityId(1),
            classname: "script_brushmodel".to_string(),
            properties: Default::default(),
            brushes: vec![box_brush((0.0, 0.0, 0.0), (8.0, 8.0, 8.0))],
            model: None,
        });

        assert!(generate_opening_portals(&mut map, &[], PortalSide::Negative, &PortalTextures::default()).is_err());
        assert!(generate_opening_portals(
            &mut map,
            &[(0, 0), (1, 0)],
            PortalSide::Negative,
            &PortalTextures::default()
        )
        .is_err());
    }

    // --- Outdoor cell walls ----------------------------------------------

    #[test]
    fn wall_between_two_flush_cells_spans_the_shared_face() {
        // Two cubes side by side sharing the plane x=64.
        let a = box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0));
        let b = box_brush((64.0, 0.0, 0.0), (128.0, 64.0, 64.0));
        let cells = vec![&a, &b];

        let (walls, overlaps) = plan_cell_walls(&cells, 8.0).unwrap();
        assert_eq!(walls.len(), 1);
        assert_eq!(overlaps, 0);

        let w = &walls[0];
        assert_eq!(w.axis, 0, "wall is thin along X");
        assert!((w.aabb.min.x - 60.0).abs() < POINT_EPS, "centred on the plane");
        assert!((w.aabb.max.x - 68.0).abs() < POINT_EPS);
        assert!((w.aabb.min.y - 0.0).abs() < POINT_EPS);
        assert!((w.aabb.max.y - 64.0).abs() < POINT_EPS);
        assert!((w.aabb.min.z - 0.0).abs() < POINT_EPS);
        assert!((w.aabb.max.z - 64.0).abs() < POINT_EPS);
    }

    #[test]
    fn wall_spans_only_the_face_overlap() {
        // Cells offset in Y: the wall only covers the shared face region.
        let a = box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0));
        let b = box_brush((64.0, 32.0, 0.0), (128.0, 96.0, 64.0));
        let cells = vec![&a, &b];

        let (walls, _) = plan_cell_walls(&cells, 8.0).unwrap();
        assert_eq!(walls.len(), 1);
        assert!((walls[0].aabb.min.y - 32.0).abs() < POINT_EPS);
        assert!((walls[0].aabb.max.y - 64.0).abs() < POINT_EPS);
    }

    #[test]
    fn t_junction_by_construction_wall_never_runs_along_a_second_cell() {
        // A (x 0..64) touches B (x 64..128) and B also touches C further
        // along the same plane. The A-B wall must stop where B ends.
        let a = box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0));
        let b = box_brush((64.0, 0.0, 0.0), (128.0, 64.0, 64.0));
        let c = box_brush((128.0, 0.0, 0.0), (192.0, 64.0, 64.0));
        let cells = vec![&a, &b, &c];

        let (walls, _) = plan_cell_walls(&cells, 8.0).unwrap();
        assert_eq!(walls.len(), 2);
        for w in &walls {
            assert!((w.aabb.max.x - w.aabb.min.x - 8.0).abs() < POINT_EPS);
            assert!(
                w.aabb.max.y - w.aabb.min.y <= 64.0 + POINT_EPS,
                "wall never extends past its own cell pair"
            );
        }
    }

    #[test]
    fn collinear_walls_are_staggered_to_opposite_sides() {
        // The 4-way: NW, NE, SW, SE cells around the origin. The two walls
        // on the plane x=0 (NW-NE and SW-SE) are collinear and must be
        // staggered to opposite sides.
        let nw = box_brush((-64.0, 0.0, 0.0), (0.0, 64.0, 64.0));
        let ne = box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0));
        let sw = box_brush((-64.0, -64.0, 0.0), (0.0, 0.0, 64.0));
        let se = box_brush((0.0, -64.0, 0.0), (64.0, 0.0, 64.0));
        let cells = vec![&nw, &ne, &sw, &se];

        let (walls, _) = plan_cell_walls(&cells, 8.0).unwrap();
        assert_eq!(walls.len(), 4);

        let x_walls: Vec<&PortalWall> = walls.iter().filter(|w| w.axis == 0).collect();
        assert_eq!(x_walls.len(), 2, "NW-NE and SW-SE walls");
        let offsets: Vec<f32> = x_walls
            .iter()
            .map(|w| w.aabb.min.x + 4.0)
            .collect(); // plane offset = min + thickness/2
        assert!(
            (offsets[0] - offsets[1]).abs() >= 4.0,
            "staggered: offsets {offsets:?}"
        );

        // The y-normal walls must be staggered independently and also end up
        // on opposite sides.
        let y_walls: Vec<&PortalWall> = walls.iter().filter(|w| w.axis == 1).collect();
        assert_eq!(y_walls.len(), 2);
        let offsets: Vec<f32> = y_walls
            .iter()
            .map(|w| w.aabb.min.y + 4.0)
            .collect();
        assert!((offsets[0] - offsets[1]).abs() >= 4.0, "offsets {offsets:?}");
    }

    #[test]
    fn partial_face_overlap_creates_a_partial_wall() {
        // Cells offset in Z still share the x=64 plane; the wall only
        // covers the shared face region (z 32..64).
        let a = box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0));
        let b = box_brush((64.0, 0.0, 32.0), (128.0, 64.0, 96.0));
        let cells = vec![&a, &b];
        let (walls, _) = plan_cell_walls(&cells, 8.0).unwrap();
        assert_eq!(walls.len(), 1);
        assert_eq!(walls[0].axis, 0);
        assert!((walls[0].aabb.min.x - 60.0).abs() < POINT_EPS);
        assert!((walls[0].aabb.max.x - 68.0).abs() < POINT_EPS);
        assert!((walls[0].aabb.min.z - 32.0).abs() < POINT_EPS);
        assert!((walls[0].aabb.max.z - 64.0).abs() < POINT_EPS);
    }

    #[test]
    fn non_coplanar_cells_produce_no_walls() {
        let a = box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0));
        let b = box_brush((96.0, 32.0, 32.0), (160.0, 96.0, 96.0)); // no shared planes
        let cells = vec![&a, &b];
        let (walls, _) = plan_cell_walls(&cells, 8.0).unwrap();
        assert!(walls.is_empty(), "no coplanar opposing faces");
    }

    #[test]
    fn cell_wall_brush_textures_follow_the_chosen_side() {
        let wall = PortalWall {
            aabb: Aabb::from_points(Vec3::new(60.0, 0.0, 0.0), Vec3::new(68.0, 64.0, 64.0)),
            axis: 0,
        };
        let textures = PortalTextures::default();
        let brush = generate_cell_wall_brush(BrushId(7), &wall, PortalSide::Positive, &textures);
        assert_eq!(brush.id.0, 7);
        assert_eq!(count_faces_with_texture(&brush, "common/portal"), 1);
        assert_eq!(count_faces_with_texture(&brush, "common/portalnodraw"), 5);
        let normals = face_normals_with_texture(&brush, "common/portal");
        assert!(normals[0].x > 0.9);
    }

    #[test]
    fn cell_walls_are_inserted_into_worldspawn() {
        let mut map = Map::default();
        map.entities.push(crate::map::Entity {
            id: crate::map::EntityId(0),
            classname: "worldspawn".to_string(),
            properties: Default::default(),
            brushes: vec![
                box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0)),
                box_brush((64.0, 0.0, 0.0), (128.0, 64.0, 64.0)),
            ],
            model: None,
        });

        let selection = vec![(0usize, 0usize), (0, 1)];
        let (created, overlaps) = generate_cell_portal_walls(
            &mut map,
            &selection,
            PortalSide::Negative,
            &PortalTextures::default(),
            8.0,
        )
        .unwrap();
        assert_eq!(created, 1);
        assert_eq!(overlaps, 0);
        assert_eq!(map.entities[0].brushes.len(), 3);
        let portal = &map.entities[0].brushes[2];
        assert!(portal.is_portal(), "portal texture must be present");
        assert!(map.generation > 0);
    }

    #[test]
    fn cell_wall_generation_needs_two_cells() {
        let mut map = Map::default();
        map.entities.push(crate::map::Entity {
            id: crate::map::EntityId(0),
            classname: "worldspawn".to_string(),
            properties: Default::default(),
            brushes: vec![box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0))],
            model: None,
        });
        assert!(generate_cell_portal_walls(
            &mut map,
            &[(0, 0)],
            PortalSide::Negative,
            &PortalTextures::default(),
            8.0
        )
        .is_err());
    }

    /// Demo on the real training_outside map: the mapper splits the arena
    /// into four quadrant cells (as in the outdoor tutorial) and the
    /// generator produces the four interior portal walls, staggered at the
    /// 4-way junction.
    #[test]
    fn training_outside_quadrant_cells_generate_four_staggered_walls() {
        let map_text = include_str!("../test/training_outside.map");
        let mut map = crate::parser::parse_map_string(map_text).unwrap();

        // Mapper-designated cells: the arena split at x=0 / y=320.
        let (x0, x1, y0, y1) = (-1384.0f32, 1248.0f32, -1196.0f32, 1836.0f32);
        let (z0, z1) = (256.0f32, 512.0f32);
        let cells = vec![
            box_brush((x0, 320.0, z0), (0.0, y1, z1)),   // NW
            box_brush((0.0, 320.0, z0), (x1, y1, z1)),   // NE
            box_brush((x0, y0, z0), (0.0, 320.0, z1)),   // SW
            box_brush((0.0, y0, z0), (x1, 320.0, z1)),   // SE
        ];
        let world_brush_count = map.entities[0].brushes.len();
        let first_id = map.entities[0]
            .brushes
            .iter()
            .map(|b| b.id.0)
            .max()
            .unwrap_or(0)
            + 1;
        for (n, cell) in cells.into_iter().enumerate() {
            let mut cell = cell;
            cell.id = BrushId(first_id + n as u32);
            map.entities[0].brushes.push(cell);
        }
        let cell_count = 4usize;

        let selection: Vec<(usize, usize)> = ((world_brush_count)..(world_brush_count
            + cell_count))
            .map(|b| (0, b))
            .collect();

        let (created, overlaps) = generate_cell_portal_walls(
            &mut map,
            &selection,
            PortalSide::Negative,
            &PortalTextures::default(),
            8.0,
        )
        .expect("quadrant cells must generate walls");

        assert_eq!(created, 4, "one wall per neighbouring cell pair");
        // The stagger pass already resolves the junctions on the staggered
        // planes; the remaining 2 perpendicular corners would need manual
        // 45-degree bevels (as in the tutorial), reported to the caller.
        assert_eq!(overlaps, 2);

        // The walls sit at the map centre lines, staggered in pairs.
        let added = &map.entities[0].brushes[world_brush_count + cell_count..];
        assert_eq!(added.len(), 4);
        let x_walls: Vec<&Brush> = added
            .iter()
            .filter(|b| (b.aabb.max.x - b.aabb.min.x) < (b.aabb.max.y - b.aabb.min.y))
            .collect();
        assert_eq!(x_walls.len(), 2, "two walls on the y-split plane");
        for w in &x_walls {
            // One spans the north half (y 320..1836), one the south half
            // (y -1196..320) — the T-junction containment.
            let north = (w.aabb.min.y - 320.0).abs() < POINT_EPS
                && (w.aabb.max.y - y1).abs() < POINT_EPS;
            let south = (w.aabb.min.y - y0).abs() < POINT_EPS
                && (w.aabb.max.y - 320.0).abs() < POINT_EPS;
            assert!(
                north || south,
                "wall must span exactly one cell pair's face: {:?}",
                (w.aabb.min.y, w.aabb.max.y)
            );
        }
        let offsets: Vec<f32> = x_walls
            .iter()
            .map(|w| w.aabb.min.x + 4.0)
            .collect();
        assert!(
            (offsets[0] - offsets[1]).abs() >= 4.0,
            "4-way stagger: plane offsets {offsets:?}"
        );

        let y_walls: Vec<&Brush> = added
            .iter()
            .filter(|b| (b.aabb.max.y - b.aabb.min.y) <= (b.aabb.max.x - b.aabb.min.x))
            .collect();
        assert_eq!(y_walls.len(), 2, "two walls on the x-split plane");
        let offsets: Vec<f32> = y_walls
            .iter()
            .map(|w| w.aabb.min.y + 4.0)
            .collect();
        assert!(
            (offsets[0] - offsets[1]).abs() >= 4.0,
            "4-way stagger: plane offsets {offsets:?}"
        );

        // Every wall is a proper portal brush: 1 portal face + 5 nodraw.
        for w in added {
            let faces = match &w.content {
                BrushContent::Convex(f) => f,
                _ => panic!("expected convex portal brush"),
            };
            assert_eq!(faces.iter().filter(|f| f.texture == "common/portal").count(), 1);
            assert_eq!(
                faces
                    .iter()
                    .filter(|f| f.texture == "common/portalnodraw")
                    .count(),
                5
            );
        }
    }

    // -------------------------------------------------------------------
    // Whole-map dbg harness (ignored by default; needs dawnville on disk)
    // -------------------------------------------------------------------

    /// A brush is an IW portal brush when it has a `common/portal` face
    /// (exact texture, not `portalnodraw`).
    fn is_portal_brush(brush: &Brush) -> bool {
        match &brush.content {
            BrushContent::Convex(faces) => faces.iter().any(|f| f.texture == "common/portal"),
            BrushContent::Patch(_) => false,
        }
    }

    #[test]
    #[ignore = "dbg harness: needs the official IW dawnville.map on disk"]
    fn dbg_dawnville_pipeline() {
        let path = std::env::var("DAWNVILLE_MAP")
            .unwrap_or_else(|_| "/workspace/attachments/dawnville/dawnville.map".to_string());
        let text = std::fs::read_to_string(&path).expect("dawnville.map on disk");
        let t0 = std::time::Instant::now();
        let mut map = crate::parser::parse_map_string(&text).expect("parse");
        println!(
            "parsed dawnville: {} entities, {} brushes in {:?}",
            map.entities.len(),
            map.entities.iter().map(|e| e.brushes.len()).sum::<usize>(),
            t0.elapsed()
        );

        // Ground truth: record original portal AABBs, then strip them.
        let mut originals: Vec<Aabb> = Vec::new();
        let mut removed = 0usize;
        for entity in &mut map.entities {
            for brush in &entity.brushes {
                if is_portal_brush(brush) {
                    if let Some(aabb) = aabb_of_brush(brush) {
                        originals.push(aabb);
                    }
                }
            }
            let before = entity.brushes.len();
            entity.brushes.retain(|b| !is_portal_brush(b));
            removed += before - entity.brushes.len();
        }
        println!(
            "ground truth: {removed} IW portal brushes removed, {} with valid AABBs",
            originals.len()
        );
        let brush_count = map.entities.iter().map(|e| e.brushes.len()).sum::<usize>();

        let t1 = std::time::Instant::now();
        let report = generate_auto_opening_portals(
            &mut map,
            PortalSide::Negative,
            &PortalTextures::default(),
        )
        .expect("auto pass");
        println!("auto pass in {:?}: {report:?}", t1.elapsed());
        let total_after = map.entities.iter().map(|e| e.brushes.len()).sum::<usize>();
        println!("brushes: {brush_count} (cleaned) -> {total_after} (with portals)");

        // Coverage: for each original portal, take its two largest extents
        // as the face rect and grid-sample how much of it sits inside any
        // generated portal (16-unit tolerance bridges thickness gaps).
        let generated: Vec<Aabb> = map
            .entities
            .iter()
            .flat_map(|e| e.brushes.iter())
            .filter(|b| is_portal_brush(b))
            .filter_map(|b| aabb_of_brush(b))
            .collect();
        println!("generated portal brushes: {}", generated.len());

        const TOL: f32 = 16.0;
        const N: usize = 32;
        let mut fracs = Vec::new();
        let mut center_hits = 0usize;
        for o in &originals {
            let ext = [o.max[0] - o.min[0], o.max[1] - o.min[1], o.max[2] - o.min[2]];
            let skip = if ext[0] <= ext[1] && ext[0] <= ext[2] {
                0
            } else if ext[1] <= ext[2] {
                1
            } else {
                2
            };
            let axes: Vec<usize> = (0..3).filter(|&a| a != skip).collect();
            let (u0, v0, u1, v1) = (o.min[axes[0]], o.min[axes[1]], o.max[axes[0]], o.max[axes[1]]);
            let area = (u1 - u0) * (v1 - v0);
            if area <= 0.0 {
                continue;
            }
            let mid = (o.min[skip] + o.max[skip]) * 0.5;
            let mut hits = 0usize;
            for j in 0..N {
                for i in 0..N {
                    let p = [
                        if axes[0] == 0 { u0 + (u1 - u0) * (i as f32 + 0.5) / N as f32 } else if axes[1] == 0 { v0 + (v1 - v0) * (j as f32 + 0.5) / N as f32 } else { mid },
                        if axes[0] == 1 { u0 + (u1 - u0) * (i as f32 + 0.5) / N as f32 } else if axes[1] == 1 { v0 + (v1 - v0) * (j as f32 + 0.5) / N as f32 } else { mid },
                        if axes[0] == 2 { u0 + (u1 - u0) * (i as f32 + 0.5) / N as f32 } else if axes[1] == 2 { v0 + (v1 - v0) * (j as f32 + 0.5) / N as f32 } else { mid },
                    ];
                    if generated.iter().any(|g| {
                        (0..3).all(|a| p[a] >= g.min[a] - TOL && p[a] <= g.max[a] + TOL)
                    }) {
                        hits += 1;
                    }
                }
            }
            fracs.push(hits as f32 / (N * N) as f32);
            let c = [
                (o.min[0] + o.max[0]) * 0.5,
                (o.min[1] + o.max[1]) * 0.5,
                (o.min[2] + o.max[2]) * 0.5,
            ];
            if generated.iter().any(|g| {
                (0..3).all(|a| c[a] >= g.min[a] - TOL && c[a] <= g.max[a] + TOL)
            }) {
                center_hits += 1;
            }
        }
        let n = fracs.len() as f32;
        let full = fracs.iter().filter(|&&f| f >= 0.99).count();
        let partial = fracs.iter().filter(|&&f| f >= 0.05 && f < 0.99).count();
        let zero = fracs.iter().filter(|&&f| f < 0.05).count();
        println!(
            "coverage of {} originals: {full} full (>=99%), {partial} partial (5-99%), {zero} untouched; mean frac {:.2}; center-hit {center_hits}/{}",
            fracs.len(),
            fracs.iter().sum::<f32>() / n.max(1.0),
            fracs.len()
        );

        // Breakdown of the misses: size class x best guess why.
        println!("--- untouched originals (frac < 0.05) ---");
        for (o, &f) in originals.iter().zip(fracs.iter()) {
            if f >= 0.05 {
                continue;
            }
            let ext = [o.max[0] - o.min[0], o.max[1] - o.min[1], o.max[2] - o.min[2]];
            let mut e = ext;
            e.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!(
                "  center ({:.0},{:.0},{:.0}) ext {:.0}x{:.0}x{:.0} (sorted {:.0}/{:.0}/{:.0}) frac {:.2}",
                (o.min[0] + o.max[0]) * 0.5,
                (o.min[1] + o.max[1]) * 0.5,
                (o.min[2] + o.max[2]) * 0.5,
                ext[0], ext[1], ext[2], e[0], e[1], e[2], f
            );
        }

        // Round-trip: save and re-parse.
        let out = "/workspace/dawnville_portals_generated.map";
        crate::parser::save_map(&map, out).expect("save");
        let text2 = std::fs::read_to_string(out).expect("read back");
        let map2 = crate::parser::parse_map_string(&text2).expect("re-parse");
        let total2 = map2.entities.iter().map(|e| e.brushes.len()).sum::<usize>();
        println!(
            "round-trip: {} entities, {total2} brushes (expected {total_after})",
            map2.entities.len()
        );
        assert_eq!(total2, total_after, "round-trip brush count");
    }


        
    
}

#[cfg(test)]
mod dbg_probe {
    use super::*;
    use crate::map::{BrushContent, BrushId};

    /// Dump the scan-plane composition at one missed IW portal:
    /// center (-1384,-18868,110) ext 64x8x92 -> wall plane y=-18868.
    #[test]
    #[ignore = "dbg probe: needs dawnville on disk"]
    fn dbg_dawnville_probe_missed_window() {
        let path = std::env::var("DAWNVILLE_MAP")
            .unwrap_or_else(|_| "/workspace/attachments/dawnville/dawnville.map".to_string());
        let text = std::fs::read_to_string(&path).expect("dawnville.map on disk");
        let mut map = crate::parser::parse_map_string(&text).expect("parse");
        for entity in &mut map.entities {
            entity.brushes.retain(|b| {
                !matches!(&b.content, BrushContent::Convex(faces) if faces.iter().any(|f| f.texture == "common/portal"))
            });
        }

        let axis = 1usize; // y
        let p = -18812.0f32;
        let (u, v) = plane_axes(axis);

        struct C {
            aabb: Aabb,
            rects: Vec<(f32, f32, f32, f32)>,
        }
        let mut crossers: Vec<C> = Vec::new();
        for (ei, entity) in map.entities.iter().enumerate() {
            for b in &entity.brushes {
                if matches!(b.content, BrushContent::Patch(_)) {
                    continue;
                }
                let mut owned = b.clone();
                owned.invalidate_geometry();
                let Ok(polys) = crate::geometry::brush_to_polygons(&owned) else { continue };
                let aabb = crate::editing::aabb_from_polys(&polys);
                if aabb.min[axis] > p - CROSS_MARGIN || aabb.max[axis] < p + CROSS_MARGIN {
                    continue;
                }
                if aabb.min[0] > 250.0 || aabb.max[0] < -150.0 || aabb.min[2] > 300.0 {
                    continue; // probe window neighbourhood only
                }
                let mut plane_info = Vec::new();
                for (verts, _idx) in &polys {
                    if verts.len() < 3 { continue; }
                    let e1 = verts[1] - verts[0];
                    let e2 = verts[2] - verts[0];
                    let n = e1.cross(e2);
                    if n.length_squared() < 1.0e-12 { continue; }
                    let n = n.normalize();
                    let dom = if n[0].abs() >= n[1].abs() && n[0].abs() >= n[2].abs() { 0 } else if n[1].abs() >= n[2].abs() { 1 } else { 2 };
                    plane_info.push(format!("{}@{:.0}", "xyz"[dom..].chars().next().unwrap(), verts[0][dom]));
                }
                println!(
                    "nearbrush ent{ei} aabb x[{:.0},{:.0}] y[{:.0},{:.0}] z[{:.0},{:.0}] faces {:?}",
                    aabb.min[0], aabb.max[0], aabb.min[1], aabb.max[1], aabb.min[2], aabb.max[2], plane_info
                );
                let mut rects = Vec::new();
                for (verts, _idx) in &polys {
                    if verts.len() < 3 { continue; }
                    let e1 = verts[1] - verts[0];
                    let e2 = verts[2] - verts[0];
                    let n = e1.cross(e2);
                    if n.length_squared() < 1.0e-12 { continue; }
                    let n = n.normalize();
                    if n[axis].abs() < 0.999 { continue; }
                    rects.push(face_rect(verts, u, v));
                }
                if !rects.is_empty() {
                    println!(
                        "crosser ent{ei} aabb x[{:.0},{:.0}] y[{:.0},{:.0}] z[{:.0},{:.0}] rects {:?}",
                        aabb.min[0], aabb.max[0], aabb.min[1], aabb.max[1], aabb.min[2], aabb.max[2], rects
                    );
                    crossers.push(C { aabb, rects });
                }
            }
        }
        println!("crossers: {}", crossers.len());
    }
}


#[cfg(test)]
mod dbg_bsp_tests {
    use super::*;
    use crate::bsp::{generate_bsp_portals, BspPortalParams};
    use crate::editing::Aabb;

    /// A brush is an IW portal brush when it has a `common/portal` face.
    fn is_portal_brush(brush: &Brush) -> bool {
        match &brush.content {
            BrushContent::Convex(faces) => faces.iter().any(|f| f.texture == "common/portal"),
            BrushContent::Patch(_) => false,
        }
    }

    #[test]
    #[ignore = "dbg harness: needs the official IW dawnville.map on disk"]
    fn dbg_dawnville_bsp() {
        let path = std::env::var("DAWNVILLE_MAP")
            .unwrap_or_else(|_| "/workspace/attachments/dawnville/dawnville.map".to_string());
        let text = std::fs::read_to_string(&path).expect("dawnville.map on disk");
        let mut map = crate::parser::parse_map_string(&text).expect("parse");
        println!(
            "parsed dawnville: {} entities, {} brushes",
            map.entities.len(),
            map.entities.iter().map(|e| e.brushes.len()).sum::<usize>()
        );

        // Strip the IW portals, keep their AABBs as ground truth.
        let mut originals: Vec<Aabb> = Vec::new();
        let mut removed = 0usize;
        for entity in &mut map.entities {
            for brush in &entity.brushes {
                if is_portal_brush(brush) {
                    if let Some(aabb) = aabb_of_brush(brush) {
                        originals.push(aabb);
                    }
                }
            }
            let before = entity.brushes.len();
            entity.brushes.retain(|b| !is_portal_brush(b));
            removed += before - entity.brushes.len();
        }
        println!("ground truth: {removed} IW portal brushes, {} valid", originals.len());

        let mut params = BspPortalParams::default();
        if std::env::var("BSP_COD").is_ok() {
            params.selection = crate::bsp::SplitterSelection::CodFaces;
        }
        println!("selection: {:?}", params.selection);
        let hist = crate::bsp::dbg_framing_histogram(&map);
        println!("framing histogram (0.0..1.0 by 0.1): {hist:?}");
        let t1 = std::time::Instant::now();
        let report = generate_bsp_portals(&mut map, PortalSide::Negative, &PortalTextures::default(), &params)
            .expect("bsp portals");
        println!("bsp pass in {:?}: {report:?}", t1.elapsed());

        // Coverage metric (same as the plane-scan harness).
        let generated: Vec<Aabb> = map
            .entities
            .iter()
            .flat_map(|e| e.brushes.iter())
            .filter(|b| is_portal_brush(b))
            .filter_map(|b| aabb_of_brush(b))
            .collect();
        println!("generated portal brushes: {}", generated.len());

        const TOL: f32 = 16.0;
        const N: usize = 32;
        let mut fracs = Vec::new();
        let mut center_hits = 0usize;
        for o in &originals {
            let ext = [o.max[0] - o.min[0], o.max[1] - o.min[1], o.max[2] - o.min[2]];
            let skip = if ext[0] <= ext[1] && ext[0] <= ext[2] {
                0
            } else if ext[1] <= ext[2] {
                1
            } else {
                2
            };
            let axes: Vec<usize> = (0..3).filter(|&a| a != skip).collect();
            let (u0, v0, u1, v1) = (o.min[axes[0]], o.min[axes[1]], o.max[axes[0]], o.max[axes[1]]);
            if (u1 - u0) * (v1 - v0) <= 0.0 {
                continue;
            }
            let mid = (o.min[skip] + o.max[skip]) * 0.5;
            let mut hits = 0usize;
            for j in 0..N {
                for i in 0..N {
                    let fu = u0 + (u1 - u0) * (i as f32 + 0.5) / N as f32;
                    let fv = v0 + (v1 - v0) * (j as f32 + 0.5) / N as f32;
                    let p = [
                        if axes[0] == 0 { fu } else if axes[1] == 0 { fv } else { mid },
                        if axes[0] == 1 { fu } else if axes[1] == 1 { fv } else { mid },
                        if axes[0] == 2 { fu } else if axes[1] == 2 { fv } else { mid },
                    ];
                    if generated.iter().any(|g| {
                        (0..3).all(|a| p[a] >= g.min[a] - TOL && p[a] <= g.max[a] + TOL)
                    }) {
                        hits += 1;
                    }
                }
            }
            fracs.push(hits as f32 / (N * N) as f32);
            let c = [(o.min[0] + o.max[0]) * 0.5, (o.min[1] + o.max[1]) * 0.5, (o.min[2] + o.max[2]) * 0.5];
            if generated.iter().any(|g| {
                (0..3).all(|a| c[a] >= g.min[a] - TOL && c[a] <= g.max[a] + TOL)
            }) {
                center_hits += 1;
            }
        }
        let n = fracs.len() as f32;
        let full = fracs.iter().filter(|&&f| f >= 0.99).count();
        let partial = fracs.iter().filter(|&&f| f >= 0.05 && f < 0.99).count();
        let zero = fracs.iter().filter(|&&f| f < 0.05).count();
        println!(
            "coverage of {} originals: {full} full (>=99%), {partial} partial (5-99%), {zero} untouched; mean frac {:.2}; center-hit {center_hits}/{}",
            fracs.len(),
            fracs.iter().sum::<f32>() / n.max(1.0),
            fracs.len()
        );

        crate::parser::save_map(&map, "/workspace/dawnville_portals_bsp.map").expect("save");
        let text2 = std::fs::read_to_string("/workspace/dawnville_portals_bsp.map").expect("read back");
        let map2 = crate::parser::parse_map_string(&text2).expect("re-parse");
        let total2 = map2.entities.iter().map(|e| e.brushes.len()).sum::<usize>();
        let total = map.entities.iter().map(|e| e.brushes.len()).sum::<usize>();
        println!("round-trip: {} entities, {total2} brushes (expected {total})", map2.entities.len());
        assert_eq!(total2, total, "round-trip brush count");
    }
}
