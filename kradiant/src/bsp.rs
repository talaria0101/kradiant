//! BSP-based portal discovery, following the CoD1 compiler (a q3map fork).
//!
//! How the real compiler thinks (from the cod-q3map-decomp reconstruction and
//! id's q3map/qbsp3 sources):
//!
//! 1. The BSP tree is built from the structural brush faces. Every leaf of
//!    that tree is a convex chunk of space; leaves still holding brushes are
//!    solid, empty ones are air.
//! 2. During the portal pass each internal node creates a portal winding on
//!    its split plane: the full plane winding, clipped against the portals
//!    that already bound the node. What survives is the *exact* empty
//!    cross-section between the two children - arches, transoms and stepped
//!    trim included, no rectangles, no grids.
//! 3. Mapper portal brushes never cut space; they are detail brushes whose
//!    faces *mark* existing leaf portals as vis borders (LinkPortalToFace),
//!    and cells are flooded around them.
//!
//! So the tool's job is steps 1-2 on the map without its portal brushes, and
//! then to emit a portal brush on the leaf portals where a mapper would have
//! placed one: wall-thickness passages (doorways, windows, arches) and the
//! wall-plane cross-sections that separate open areas.

use crate::editing::Aabb;
use crate::map::{Brush, BrushContent, BrushId, Map};
use crate::texmap::face_plane_normal;
use crate::Vec3;

/// Half extent of the base windings (the compiler's world is +-131072).
const WORLD_HALF: f32 = 131072.0;
/// Portal clip epsilon (decomp: 0.1 in MakeNodePortal).
const EPS_PORTAL: f64 = 0.1;
/// Split epsilon for portal partitioning (decomp: 0.001).
const EPS_SPLIT: f64 = 0.001;
/// Brush split epsilon (qbsp3 SplitBrush: PLANESIDE_EPSILON 0.1).
const EPS_BRUSH: f32 = 0.1;
/// Plane merge tolerances for the canonical plane pool.
const PLANE_NORMAL_EPS: f64 = 0.00001;
const PLANE_DIST_EPS: f64 = 0.01;

/// Splitter selection strategy for the BSP build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplitterSelection {
    /// CoD sub_404CE0: grid-forced axial splits on x/y, then face-based
    /// scoring (5*(facing - splits) + 5*axial). The compiler's own
    /// algorithm.
    #[default]
    CodFaces,
    /// qbsp3-style brush-side scoring (5*facing - 5*splits - |front-back|
    /// + 5*axial, splits preferred). Better measured coverage on
    /// dawnville with the current candidate framing.
    BrushScoring,
}

/// Parameters for [`generate_bsp_portals`].
#[derive(Debug, Clone)]
pub struct BspPortalParams {
    /// Shortest portal winding edge that still gets a brush (world units).
    pub min_edge: f32,
    /// Smallest portal winding area that still gets a brush.
    pub min_area: f32,
    /// Thickness of the emitted portal brush, centred on the leaf portal.
    pub thickness: f32,
    /// Also emit brushes on void-void cross-sections (open-area separators).
    /// When false only wall-passage portals (a leaf spanning wall thickness)
    /// are emitted.
    pub area_separators: bool,
    /// Largest allowed winding extent along any axis (world units). The
    /// originals top out around 5200; anything near the 131072 world bound
    /// is an uncarved void cross-section, not a mappable portal.
    pub max_extent: f32,
    /// Largest allowed union extent (merge + absorption) along any axis.
    /// Chained fragments tile void without bound; this keeps unions at
    /// district scale while still covering multi-window facades.
    pub max_union: f32,
    /// Coplanar portals within this gap (world units) are unioned into one
    /// brush after emission (the originals over-cover through trim and let
    /// the compiler trim the portal to the enclosed volume).
    pub merge_gap: f32,
    /// Deepest BSP level built; deeper nodes become leaves. Dawnville's
    /// 77k-unit y extent needs ~110 levels of 1024 grid peeling alone,
    /// so this must clear that plus face depth.
    pub max_depth: usize,
    /// Orient each portal's active face toward the larger adjacent open
    /// leaf (IW orients portals toward the larger cell). When false the
    /// `side` argument to [`generate_bsp_portals`] applies to all portals.
    pub auto_side: bool,
    /// Splitter selection strategy.
    pub selection: SplitterSelection,
}

impl Default for BspPortalParams {
    fn default() -> Self {
        // Leaf-portal pieces of one opening can be narrow (the carving splits
        // an 88-tall doorway into 64x16 + 64x72 chunks); the coplanar merge
        // pass rebuilds the full opening after emission, so the piece floor
        // only has to keep trim slivers out.
        Self {
            min_edge: 8.0,
            min_area: 64.0,
            thickness: 8.0,
            area_separators: true,
            max_extent: 8192.0,
            merge_gap: 32.0,
            max_union: 16384.0,
            max_depth: 200,
            auto_side: true,
            // BrushScoring is the default: its brush-face-aligned splits
            // place portals on the architecture (wall faces, openings),
            // while CodFaces grid peeling optimizes for compilation
            // balance and spends most of its depth budget slicing skybox
            // void on town maps. CodFaces stays available for
            // compiler-coupled placement once cells exist.
            selection: SplitterSelection::BrushScoring,
        }
    }
}

/// Report of [`generate_bsp_portals`].
#[derive(Debug, Clone, Copy, Default)]
pub struct BspPortalReport {
    pub structural_brushes: usize,
    pub leaves: usize,
    pub solid_leaves: usize,
    pub internal_portals: usize,
    pub candidates: usize,
    pub portals_created: usize,
    pub portals_merged: usize,
    /// Portals dropped by the prune pass (buried in solid, or twin of a
    /// larger portal through the same wall).
    pub portals_pruned: usize,
    pub max_depth: usize,
}

// ---------------------------------------------------------------------------
// Winding primitives (ports of polylib.c)
// ---------------------------------------------------------------------------

/// A convex polygon: points wound counter-clockwise around `outward_normal`.
type Winding = Vec<Vec3>;

/// Classification of a point against a plane.
const SIDE_FRONT: i32 = 0;
const SIDE_BACK: i32 = 1;
const SIDE_ON: i32 = 2;

fn classify(pt: Vec3, n: Vec3, d: f32, eps: f64) -> (f64, i32) {
    // The decompiler notes the original classifies in x87 double; float
    // arithmetic flips decisions at dawnville-scale coordinates.
    let dot = pt.x as f64 * n.x as f64 + pt.y as f64 * n.y as f64 + pt.z as f64 * n.z as f64
        - d as f64;
    let side = if dot > eps {
        SIDE_FRONT
    } else if dot < -eps {
        SIDE_BACK
    } else {
        SIDE_ON
    };
    (dot, side)
}

/// The full winding of a plane: a huge quad centred at the plane origin.
fn base_winding_for_plane(n: Vec3, d: f32) -> Winding {
    // Dominant axis (first max wins), up on another axis, Gram-Schmidt,
    // right = up x normal - the decomp sub_413420 order.
    let mut dom = 0usize;
    let mut best = -WORLD_HALF;
    for i in 0..3 {
        let a = n[i].abs();
        if a > best {
            best = a;
            dom = i;
        }
    }
    let mut up = Vec3::ZERO;
    if dom <= 1 {
        up.z = 1.0;
    } else {
        up.x = 1.0;
    }
    let dot = up.dot(n);
    up = (up - n * dot).normalize();
    let right = up.cross(n);
    let org = n * d;

    let big = WORLD_HALF;
    let u = up * big;
    let r = right * big;
    vec![org - r + u, org + r + u, org + r - u, org - r - u]
}

/// Keep only the front part of `w` against the plane. Returns `None` when
/// everything is clipped away. All-on-plane windings survive (q3
/// ClipWindingEpsilon behaviour: no front AND no back points -> front = copy).
fn clip_winding_front(w: &Winding, n: Vec3, d: f32, eps: f64) -> Option<Winding> {
    let (dots, sides, counts) = classify_winding(w, n, d, eps);
    if counts[SIDE_FRONT as usize] == 0 {
        if counts[SIDE_BACK as usize] == 0 {
            return Some(w.clone());
        }
        return None;
    }
    if counts[SIDE_BACK as usize] == 0 {
        return Some(w.clone());
    }
    let mut out = Winding::with_capacity(w.len() + 4);
    for i in 0..w.len() {
        let j = (i + 1) % w.len();
        if sides[i] != SIDE_BACK {
            out.push(w[i]);
        }
        if sides[i] == SIDE_ON || sides[j] == SIDE_ON || sides[i] == sides[j] {
            continue;
        }
        out.push(interpolate(w[i], w[j], dots[i], dots[j], n, d));
    }
    if out.len() < 3 {
        None
    } else {
        Some(out)
    }
}

/// Split `w` against the plane into (front, back).
fn split_winding(w: &Winding, n: Vec3, d: f32, eps: f64) -> (Option<Winding>, Option<Winding>) {
    let (dots, sides, counts) = classify_winding(w, n, d, eps);
    if counts[SIDE_ON as usize] == w.len() {
        return (Some(w.clone()), None);
    }
    if counts[SIDE_FRONT as usize] == 0 {
        return (None, Some(w.clone()));
    }
    if counts[SIDE_BACK as usize] == 0 {
        return (Some(w.clone()), None);
    }
    let mut front = Winding::with_capacity(w.len() + 4);
    let mut back = Winding::with_capacity(w.len() + 4);
    for i in 0..w.len() {
        let j = (i + 1) % w.len();
        match sides[i] {
            SIDE_FRONT => front.push(w[i]),
            SIDE_BACK => back.push(w[i]),
            _ => {
                front.push(w[i]);
                back.push(w[i]);
            }
        }
        if sides[i] == SIDE_ON || sides[j] == SIDE_ON || sides[i] == sides[j] {
            continue;
        }
        // The split point belongs to BOTH pieces (q3 ClipWindingEpsilon).
        let mid = interpolate(w[i], w[j], dots[i], dots[j], n, d);
        front.push(mid.clone());
        back.push(mid);
    }
    let f = if front.len() >= 3 { Some(front) } else { None };
    let b = if back.len() >= 3 { Some(back) } else { None };
    (f, b)
}

fn classify_winding(w: &Winding, n: Vec3, d: f32, eps: f64) -> (Vec<f64>, Vec<i32>, [usize; 3]) {
    let mut dots = Vec::with_capacity(w.len());
    let mut sides = Vec::with_capacity(w.len());
    let mut counts = [0usize; 3];
    for p in w {
        let (dot, side) = classify(*p, n, d, eps);
        dots.push(dot);
        sides.push(side);
        counts[side as usize] += 1;
    }
    (dots, sides, counts)
}

/// Edge interpolation with axial snapping (the decomp keeps exact coordinates
/// when the clip plane is axis aligned).
fn interpolate(p1: Vec3, p2: Vec3, dot1: f64, dot2: f64, n: Vec3, d: f32) -> Vec3 {
    let frac = (dot1 / (dot1 - dot2)) as f32;
    let mut mid = p1 + (p2 - p1) * frac;
    for k in 0..3 {
        if n[k] == 1.0 {
            mid[k] = d;
        } else if n[k] == -1.0 {
            mid[k] = -d;
        }
    }
    mid
}

fn winding_area(w: &Winding) -> f32 {
    let mut total = 0.0f32;
    for i in 2..w.len() {
        let v1 = w[i - 1] - w[0];
        let v2 = w[i] - w[0];
        total += v1.cross(v2).length() * 0.5;
    }
    total
}

/// qbsp3 WindingIsTiny: tiny edges dominate the winding.
fn winding_is_tiny(w: &Winding) -> bool {
    if w.len() < 3 {
        return true;
    }
    let mut edges = 0usize;
    for i in 0..w.len() {
        let j = (i + 1) % w.len();
        let d = w[j] - w[i];
        if d.length_squared() > 0.0400 {
            // (0.2)^2
            edges += 1;
        }
    }
    edges == 0 || winding_area(w) <= 1.0
}

/// Plane of a CCW winding (normal pointing against the winding's front).
/// AABB side test against a plane: 1 front, 2 back, 3 both, 0 facing.
fn box_on_plane_side(b: &Aabb, plane: &Plane) -> u8 {
    const FRONT: u8 = 1;
    const BACK: u8 = 2;
    for k in 0..3 {
        let a = plane.n[k].abs();
        if a < 0.999 {
            continue;
        }
        if plane.n[k] > 0.0 {
            let front = ((b.max[k] as f64) - (plane.d as f64)) > EPS_BRUSH as f64;
            let back = ((b.min[k] as f64) - (plane.d as f64)) < -(EPS_BRUSH as f64);
            return match (front, back) {
                (true, true) => FRONT | BACK,
                (true, false) => FRONT,
                _ => BACK,
            };
        } else {
            let front = ((plane.d as f64) - (b.min[k] as f64)) > EPS_BRUSH as f64;
            let back = ((plane.d as f64) - (b.max[k] as f64)) < -(EPS_BRUSH as f64);
            return match (front, back) {
                (true, true) => FRONT | BACK,
                (true, false) => FRONT,
                _ => BACK,
            };
        }
    }
    let mut out = 0u8;
    for p in [b.min, b.max] {
        let (dot, _) = classify(p, plane.n, plane.d, EPS_BRUSH as f64);
        if dot > 0.0 {
            out |= FRONT;
        } else {
            out |= BACK;
        }
    }
    out
}

fn winding_plane(w: &Winding) -> Option<Plane> {
    if w.len() < 3 {
        return None;
    }
    let n = (w[1] - w[0]).cross(w[2] - w[0]);
    if n.length_squared() < 1.0e-12 {
        return None;
    }
    let n = n.normalize();
    Some(Plane {
        n,
        d: n.dot(w[0]),
    })
}

fn winding_bounds(w: &Winding) -> Aabb {
    let mut min = w[0];
    let mut max = w[0];
    for p in w.iter().skip(1) {
        min = min.min(*p);
        max = max.max(*p);
    }
    Aabb::from_points(min, max)
}

/// Component-wise union of two AABBs.
fn aabb_union(a: Aabb, b: Aabb) -> Aabb {
    Aabb::from_points(a.min.min(b.min), a.max.max(b.max))
}

// ---------------------------------------------------------------------------
// Canonical plane pool
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
struct Plane {
    n: Vec3,
    d: f32,
}

/// Largest normal deviation from axial that still snaps to the axis.
/// 1e-4 (~0.006 degrees) catches float-error rotations from plane-point
/// math while leaving mapper-rotated walls (dawnville's diagonals deviate
/// by 0.01+) alone.
const SNAP_NORMAL_EPS: f32 = 1.0e-4;

/// Snap a near-axial normal to the exact axis (q3map FindFloatPlane
/// behaviour). Returns the input unchanged when no component dominates.
fn snap_normal(n: Vec3) -> Vec3 {
    let mut dom = 0usize;
    let mut best = 0.0f32;
    for k in 0..3 {
        let a = n[k].abs();
        if a > best {
            best = a;
            dom = k;
        }
    }
    if best > 1.0 - SNAP_NORMAL_EPS {
        let mut s = Vec3::ZERO;
        s[dom] = if n[dom] >= 0.0 { 1.0 } else { -1.0 };
        s
    } else {
        n
    }
}

#[derive(Default)]
struct PlanePool {
    planes: Vec<Plane>,
}

impl PlanePool {
    /// Canonical, positive-facing plane index (normalised like q3map).
    /// Near-axial normals snap to exact axial planes first: downstream
    /// code compares pool normals with `== 1.0` (axial split bounds
    /// seeding, axial scoring bonuses), and the grid force merges its
    /// exact-axial planes into near-axial face planes without snapping.
    /// Without the snap a merged grid plane is near-axial, never seeds
    /// bounds, partitions nothing, and is re-selected forever (depth-200
    /// sliver recursion on dawnville: x=-2048 re-split 140 levels deep).
    fn find(&mut self, n: Vec3, d: f32) -> usize {
        let mut n = snap_normal(n);
        let mut d = d;
        const N_EPS: f32 = PLANE_NORMAL_EPS as f32;
        if n.x < -N_EPS
            || (n.x.abs() <= N_EPS && (n.y < -N_EPS
                || (n.y.abs() <= N_EPS && n.z < -N_EPS)))
        {
            n = -n;
            d = -d;
        }
        for (i, p) in self.planes.iter().enumerate() {
            let dn = (p.n.x - n.x) as f64 * (p.n.x - n.x) as f64
                + (p.n.y - n.y) as f64 * (p.n.y - n.y) as f64
                + (p.n.z - n.z) as f64 * (p.n.z - n.z) as f64;
            if dn < PLANE_NORMAL_EPS * PLANE_NORMAL_EPS
                && ((p.d - d).abs() as f64) < PLANE_DIST_EPS
            {
                return i;
            }
        }
        self.planes.push(Plane { n, d });
        self.planes.len() - 1
    }
}

// ---------------------------------------------------------------------------
// BSP brushes
// ---------------------------------------------------------------------------

/// A convex brush in split form: one outward-facing plane + CCW winding per
/// side.
#[derive(Clone)]
struct BspBrush {
    sides: Vec<(usize, Winding)>,
    bounds: Aabb,
    entity: usize,
}

fn bounds_of_sides(sides: &[(usize, Winding)]) -> Aabb {
    let mut b: Option<Aabb> = None;
    for (_, w) in sides {
        for p in w {
            b = Some(match b {
                None => Aabb::from_points(*p, *p),
                Some(acc) => aabb_union(acc, Aabb::from_points(*p, *p)),
            });
        }
    }
    b.unwrap_or(Aabb::from_points(Vec3::ZERO, Vec3::ZERO))
}

/// Build split brushes from a map brush. Polygons are re-oriented outward
/// regardless of the parser's face convention.
fn bsp_brush_from_map_brush(
    brush: &Brush,
    entity: usize,
    pool: &mut PlanePool,
) -> Option<BspBrush> {
    if matches!(brush.content, BrushContent::Patch(_)) {
        return None;
    }
    let mut owned = brush.clone();
    owned.invalidate_geometry();
    let polys = crate::geometry::brush_to_polygons(&owned).ok()?;
    if polys.len() < 4 {
        return None;
    }

    // Interior reference point (average of face centroids).
    let mut interior = Vec3::ZERO;
    for (verts, _) in &polys {
        for v in verts {
            interior += *v;
        }
    }
    let nverts: f32 = polys.iter().map(|(v, _)| v.len()).sum::<usize>() as f32;
    interior *= 1.0 / nverts;

    let mut sides = Vec::with_capacity(polys.len());
    for (verts, _) in &polys {
        if verts.len() < 3 {
            continue;
        }
        let mut n = (verts[1] - verts[0]).cross(verts[2] - verts[0]);
        if n.length_squared() < 1.0e-12 {
            continue;
        }
        n = n.normalize();
        let d = n.dot(verts[0]);
        let mut pts = verts.clone();
        // Outward: interior must be behind the plane.
        if (interior.dot(n) - d) > 0.0 {
            n = -n;
            let dd = n.dot(verts[0]);
            pts.reverse();
            let _ = dd;
        }
        let idx = pool.find(n, n.dot(pts[0]));
        sides.push((idx, pts));
    }
    if sides.len() < 4 {
        return None;
    }
    let bounds = bounds_of_sides(&sides);
    if (bounds.max.x - bounds.min.x).max((bounds.max.y - bounds.min.y).max(bounds.max.z - bounds.min.z)) <= 0.0 {
        return None;
    }
    Some(BspBrush {
        sides,
        bounds,
        entity,
    })
}

/// qbsp3 SplitBrush: split a convex brush by a plane into two brushes.
/// `plane_idx` is the plane's canonical index in the pool (the mid winding
/// side reuses it).
fn split_brush(
    brush: &BspBrush,
    plane: &Plane,
    plane_idx: usize,
    pool: &PlanePool,
) -> (Option<BspBrush>, Option<BspBrush>) {
    // q3 convention: d_front = largest dot, d_back = smallest dot (<= 0).
    let mut d_front = 0.0f64;
    let mut d_back = 0.0f64;
    for (_, w) in &brush.sides {
        for p in w {
            let (dot, _) = classify(*p, plane.n, plane.d, 0.0);
            if dot > d_front {
                d_front = dot;
            }
            if dot < d_back {
                d_back = dot;
            }
        }
    }
    if d_front < EPS_BRUSH as f64 {
        return (None, Some(brush.clone()));
    }
    if d_back > -(EPS_BRUSH as f64) {
        return (Some(brush.clone()), None);
    }

    // Mid winding: full plane winding clipped into the brush volume. The
    // side windings are CCW around their OUTWARD normal, so derive each
    // side's true plane from its points (the pool only stores canonical
    // planes without facing).
    let mut mid = base_winding_for_plane(plane.n, plane.d);
    for (idx, w) in &brush.sides {
        let sp = winding_plane(w).unwrap_or(pool.planes[*idx]);
        let _ = (idx, sp);
        match clip_winding_front(&mid, -sp.n, -sp.d, 0.0) {
            Some(w) => mid = w,
            None => {
                mid = Vec::new();
                break;
            }
        }
    }
    if mid.len() < 3 || winding_is_tiny(&mid) {
        // Not really split: brush goes where most of it is.
        return mostly_on_side(brush, plane);
    }

    let mut front_sides: Vec<(usize, Winding)> = Vec::new();
    let mut back_sides: Vec<(usize, Winding)> = Vec::new();
    for (idx, w) in &brush.sides {
        let (f, b) = split_winding(w, plane.n, plane.d, 0.0);
        if let Some(f) = f {
            front_sides.push((*idx, f));
        }
        if let Some(b) = b {
            back_sides.push((*idx, b));
        }
    }
    if front_sides.len() < 4 || back_sides.len() < 4 {
        return mostly_on_side(brush, plane);
    }

    let mid_rev: Winding = mid.iter().rev().copied().collect();
    front_sides.push((plane_idx, mid.clone()));
    back_sides.push((plane_idx, mid_rev));

    let fb = bounds_of_sides(&front_sides);
    let bb = bounds_of_sides(&back_sides);
    if !bounds_valid(&fb) || !bounds_valid(&bb) {
        return mostly_on_side(brush, plane);
    }

    (
        Some(BspBrush {
            sides: front_sides,
            bounds: fb,
            entity: brush.entity,
        }),
        Some(BspBrush {
            sides: back_sides,
            bounds: bb,
            entity: brush.entity,
        }),
    )
}

fn bounds_valid(b: &Aabb) -> bool {
    b.max.x - b.min.x > -EPS_BRUSH && b.max.y - b.min.y > -EPS_BRUSH && b.max.z - b.min.z > -EPS_BRUSH
}

fn mostly_on_side(brush: &BspBrush, plane: &Plane) -> (Option<BspBrush>, Option<BspBrush>) {
    // qbsp3 BrushMostlyOnSide.
    let mut max = 0.0f64;
    let mut front = true;
    for (_, w) in &brush.sides {
        for p in w {
            let (dot, _) = classify(*p, plane.n, plane.d, 0.0);
            if dot > max {
                max = dot;
                front = true;
            }
            if -dot > max {
                max = -dot;
                front = false;
            }
        }
    }
    let clone = brush.clone();
    if front {
        (Some(clone), None)
    } else {
        // keep move checker happy
        (None, Some(clone))
    }
}

// ---------------------------------------------------------------------------
// Tree + portals
// ---------------------------------------------------------------------------

const PLANENUM_LEAF: i32 = -1;

struct TreeNode {
    planenum: i32,
    children: Option<[usize; 2]>,
    brushes: Vec<BspBrush>,
    /// Structural faces driving the splitter selection (CoD FaceBSP).
    faces: Vec<BspFace>,
    /// Node volume bounds. Children inherit the parent bounds, then an
    /// axial split plane seeds the split axis on each side (facebsp.c).
    mins: Vec3,
    maxs: Vec3,
    leaf: bool,
    /// index into `Vec<NodeData>`, or usize::MAX for the outside node.
    portals: Vec<usize>,
}

/// One structural face: canonical plane index + a winding. Splitters are
/// chosen from these, exactly like CoD's FaceBSP (sub_404CE0).
#[derive(Clone)]
struct BspFace {
    plane_idx: usize,
    winding: Winding,
}

/// CoD WindingAgainstPlane (facebsp.c): -2 crosses, 0 front, 1 back,
/// 2 coplanar. Epsilon 0.1 on both sides.
fn winding_against_plane(w: &Winding, n: Vec3, d: f32) -> i32 {
    if w.is_empty() {
        return 2;
    }
    let mut front = false;
    let mut back = false;
    for p in w {
        let dot = (p.x as f64 * n.x as f64 + p.y as f64 * n.y as f64 + p.z as f64 * n.z as f64)
            - d as f64;
        if dot >= -0.1 {
            if dot > 0.1 {
                if back {
                    return -2;
                }
                front = true;
            }
        } else {
            if front {
                return -2;
            }
            back = true;
        }
    }
    if back {
        1
    } else if front {
        0
    } else {
        2
    }
}

/// Subtract a convex brush volume from a winding: return the pieces of `w`
/// lying outside `brush`. Points within `eps` of a brush plane count as
/// inside, so abutting coincident faces are consumed. Pieces are wound
/// like the input and carry no plane association (callers keep the side's
/// canonical plane index).
fn subtract_brush(w: &Winding, brush_planes: &[Plane], eps: f64) -> Vec<Winding> {
    let mut outside: Vec<Winding> = Vec::new();
    // Stack of (piece, next brush plane to test): a piece surviving all
    // planes is fully buried and dropped.
    let mut stack: Vec<(Winding, usize)> = vec![(w.clone(), 0)];
    while let Some((piece, pi)) = stack.pop() {
        if piece.len() < 3 {
            continue;
        }
        if pi >= brush_planes.len() {
            continue; // inside every plane -> buried
        }
        let pl = &brush_planes[pi];
        let (_, _, counts) = classify_winding(&piece, pl.n, pl.d, eps);
        if counts[SIDE_FRONT as usize] == 0 {
            // All back or on-plane (abutting counts as buried): still
            // inside with respect to this plane, test the rest.
            stack.push((piece, pi + 1));
            continue;
        }
        if counts[SIDE_BACK as usize] == 0 {
            // All front or on-plane: outside, keep the whole piece.
            outside.push(piece);
            continue;
        }
        // Crossing: the front part is outside; the back part may still be
        // outside through the remaining planes.
        let (front, back) = split_winding(&piece, pl.n, pl.d, eps);
        if let Some(f) = front {
            outside.push(f);
        }
        if let Some(b) = back {
            stack.push((b, pi + 1));
        }
    }
    outside
        .into_iter()
        .filter(|p| p.len() >= 3 && !winding_is_tiny(p))
        .collect()
}

/// Build the structural face list from clipped hull windings (CoD
/// MakeVisibleBspFaceList / visibleHull): every brush side winding with
/// the parts buried inside other structural brushes removed. Fully buried
/// sides yield no faces, which is what keeps coincident interior planes
/// out of the splitter selection. Falls back to raw sides when nothing
/// survives (degenerate input).
fn make_visible_bsp_face_list(brushes: &[BspBrush]) -> Vec<BspFace> {
    // Outward planes per brush (the pool only stores canonical
    // positive-facing planes, so derive the true planes from windings).
    let outward: Vec<Vec<Plane>> = brushes
        .iter()
        .map(|b| {
            b.sides
                .iter()
                .filter_map(|(_, w)| winding_plane(w))
                .collect()
        })
        .collect();
    let mut faces = Vec::new();
    for (bi, b) in brushes.iter().enumerate() {
        for (idx, w) in &b.sides {
            let wb = winding_bounds(w);
            let mut pieces = vec![w.clone()];
            for (oi, o_planes) in outward.iter().enumerate() {
                if oi == bi {
                    continue;
                }
                // Quick reject: the winding must touch the other brush.
                let ob = &brushes[oi].bounds;
                const PAD: f32 = 0.5;
                if wb.max.x < ob.min.x - PAD
                    || wb.min.x > ob.max.x + PAD
                    || wb.max.y < ob.min.y - PAD
                    || wb.min.y > ob.max.y + PAD
                    || wb.max.z < ob.min.z - PAD
                    || wb.min.z > ob.max.z + PAD
                {
                    continue;
                }
                let mut next = Vec::new();
                for p in &pieces {
                    next.extend(subtract_brush(p, o_planes, 0.1));
                }
                pieces = next;
                if pieces.is_empty() {
                    break;
                }
            }
            for p in pieces {
                faces.push(BspFace {
                    plane_idx: *idx,
                    winding: p,
                });
            }
        }
    }
    if faces.is_empty() {
        // Degenerate: every side buried (or no brushes); raw sides still
        // partition space.
        for b in brushes {
            for (idx, w) in &b.sides {
                faces.push(BspFace {
                    plane_idx: *idx,
                    winding: w.clone(),
                });
            }
        }
    }
    faces
}

/// blocksize for the grid-forced axial splits (q3map default, -blocksize).
const BLOCKSIZE: f32 = 1024.0;
/// Face clip epsilon for splitter partitioning (facebsp.c FACE_EPSILON).
const FACE_EPSILON: f64 = 0.2;

struct TreePortal {
    winding: Winding,
    plane: Plane,
    /// node indices: [0] is on the front side.
    nodes: [usize; 2],
}


struct PortalTree {
    pool: PlanePool,
    nodes: Vec<TreeNode>,
    portals: Vec<TreePortal>,
    outside: usize,
    max_depth: usize,
    depth_limit: usize,
    selection: SplitterSelection,
}

impl PortalTree {
    fn node_is_outside(&self, i: usize) -> bool {
        i == self.outside
    }
}

/// Detail textures never become structural faces (CoD builtin shader table).
fn is_detail_texture(tex: &str) -> bool {
    let t = tex.rsplit('/').next().unwrap_or(tex);
    const NAMES: &[&str] = &[
        "portal",
        "portalnodraw",
        "nodraw",
        "nonsolid",
        "ladder",
        "lightclip",
        "lightclipfolkage",
        "clip",
        "clipfoliage",
        "clipmonster",
        "clipai",
        "clipshot",
        "clipweapon",
        "clipglass",
        "trigger",
        "trigger_use",
        "trigger_damage",
        "trigger_use_touch",
        "trigger_multiple",
        "origin",
        "skip",
        "hint",
        "caulktrans",
        "foliage",
    ];
    if tex.starts_with("common/") && NAMES.contains(&t) {
        return true;
    }
    false
}

impl PortalTree {
    fn build(
        pool: PlanePool,
        brushes: Vec<BspBrush>,
        selection: SplitterSelection,
        depth_limit: usize,
    ) -> PortalTree {
        // CoD FaceBSP: the structural face list drives the splitter
        // selection; faces are the clipped hull windings
        // (MakeVisibleBspFaceList): side parts buried inside other
        // structural brushes are removed, so coincident interior planes
        // never reach the splitter.
        let faces: Vec<BspFace> = make_visible_bsp_face_list(&brushes);
        // Tree bounds from the face windings (FaceBSP).
        let mut bounds: Option<Aabb> = None;
        for f in &faces {
            let wb = winding_bounds(&f.winding);
            bounds = Some(match bounds {
                None => wb,
                Some(acc) => aabb_union(acc, wb),
            });
        }
        let mut mins = Vec3::new(-4096.0, -4096.0, -4096.0);
        let mut maxs = Vec3::new(4096.0, 4096.0, 4096.0);
        if let Some(b) = bounds {
            mins = b.min;
            maxs = b.max;
        }
        let mut tree = PortalTree {
            pool,
            nodes: Vec::new(),
            portals: Vec::new(),
            outside: usize::MAX,
            max_depth: 0,
            depth_limit,
            selection,
        };

        // Headnode + outside node with 6 border portals (decomp MakeOutsideNode).
        let head = tree.push_node(TreeNode {
            planenum: PLANENUM_LEAF,
            children: None,
            brushes,
            faces,
            mins,
            maxs,
            leaf: false,
            portals: Vec::new(),
        });
        let outside = tree.push_node(TreeNode {
            planenum: PLANENUM_LEAF,
            children: None,
            brushes: Vec::new(),
            faces: Vec::new(),
            mins,
            maxs,
            leaf: true,
            portals: Vec::new(),
        });
        tree.outside = outside;

        for axis in 0..3usize {
            for neg in [false, true] {
                let mut n = Vec3::ZERO;
                let d;
                if neg {
                    n[axis] = -1.0;
                    d = -maxs[axis] - 8.0;
                } else {
                    n[axis] = 1.0;
                    d = mins[axis] - 8.0;
                }
                let mut w = base_winding_for_plane(n, d);
                for a2 in 0..3usize {
                    for neg2 in [false, true] {
                        if a2 == axis && neg == neg2 {
                            continue;
                        }
                        let mut n2 = Vec3::ZERO;
                        let d2;
                        if neg2 {
                            n2[a2] = -1.0;
                            d2 = -maxs[a2] - 8.0;
                        } else {
                            n2[a2] = 1.0;
                            d2 = mins[a2] - 8.0;
                        }
                        if let Some(clipped) = clip_winding_front(&w, n2, d2, 0.1) {
                            w = clipped;
                        } else {
                            w = Vec::new();
                            break;
                        }
                    }
                    if w.is_empty() {
                        break;
                    }
                }
                if w.len() >= 3 {
                    let pi = tree.portals.len();
                    tree.portals.push(TreePortal {
                        winding: w,
                        plane: Plane { n, d },
                        nodes: [head, outside],
                    });
                    tree.nodes[head].portals.push(pi);
                    tree.nodes[outside].portals.push(pi);
                }
            }
        }

        tree.make_tree_portals_r_inner(head, 0, &[]);
        tree
    }

    fn push_node(&mut self, n: TreeNode) -> usize {
        self.nodes.push(n);
        self.nodes.len() - 1
    }

    /// qbsp3-style brush-side scoring with ancestor-plane blocking. Kept
    /// because it measures better on dawnville with the current framing
    /// classifier; see SplitterSelection.
    fn select_split_plane_brush(
        &self,
        node: &TreeNode,
        parent_planes: &[usize],
    ) -> Option<usize> {
        const MAX_CANDIDATES: usize = 256;
        let brushes = &node.brushes;
        if brushes.is_empty() {
            return None;
        }
        let mut best: Option<(usize, i64)> = None;

        let mut candidates: Vec<usize> = Vec::new();
        for b in brushes {
            for (idx, _) in &b.sides {
                if !candidates.contains(idx) {
                    candidates.push(*idx);
                }
            }
        }
        candidates.sort_unstable();
        if candidates.len() > MAX_CANDIDATES {
            let step = candidates.len() / MAX_CANDIDATES;
            candidates = candidates.into_iter().step_by(step).collect();
        }

        for &cand in &candidates {
            if parent_planes.contains(&cand) {
                continue;
            }
            let plane = self.pool.planes[cand];
            let mut front = 0i64;
            let mut back = 0i64;
            let mut splits = 0i64;
            let mut facing = 0i64;
            for b in brushes {
                let side = box_on_plane_side(&b.bounds, &plane);
                const FRONT: u8 = 1;
                const BACK: u8 = 2;
                match side {
                    FRONT => {
                        front += 1;
                        continue;
                    }
                    BACK => {
                        back += 1;
                        continue;
                    }
                    3 => {}
                    _ => {
                        facing += 1;
                        continue;
                    }
                }
                let mut df = 0.0f64;
                let mut db = 0.0f64;
                for (sidx, w) in &b.sides {
                    if *sidx == cand {
                        facing += 1;
                    }
                    for p in w {
                        let (dot, _) = classify(*p, plane.n, plane.d, EPS_BRUSH as f64);
                        if dot > df {
                            df = dot;
                        }
                        if -dot > db {
                            db = -dot;
                        }
                    }
                }
                if df < EPS_BRUSH as f64 {
                    back += 1;
                } else if db < EPS_BRUSH as f64 {
                    front += 1;
                } else {
                    splits += 1;
                    front += 1;
                    back += 1;
                }
            }
            let axial = plane.n.x == 1.0 || plane.n.y == 1.0 || plane.n.z == 1.0;
            let mut value: i64 = 5 * facing - 5 * splits - (front - back).abs();
            if axial {
                value += 5;
            }
            if splits == 0 {
                value -= 1000;
            }
            if best.map_or(true, |(_, bv)| value > bv) {
                best = Some((cand, value));
            }
        }
        best.map(|(idx, _)| idx)
    }

    /// CoD sub_404CE0 SelectSplitPlaneNum: grid-forced axial splits on x/y
    /// first, then the plane whose face has the most coplanar supporters and
    /// the fewest splits: score = 5*(facing - splits) + 5*axial.
    fn select_split_plane_cod(node: &TreeNode, pool: &mut PlanePool) -> Option<usize> {
        // Grid force (x/y only): the next 1024-line boundary the node spans.
        for axis in 0..2usize {
            let grid = (node.mins[axis] / BLOCKSIZE).floor() * BLOCKSIZE + BLOCKSIZE;
            if node.maxs[axis] > grid {
                let mut n = Vec3::ZERO;
                n[axis] = 1.0;
                return Some(pool.find(n, grid));
            }
        }

        // Face scoring. `checked` mirrors the decomp: reset once, outer loop
        // skips coplanar faces already counted, inner loop counts all faces.
        let faces = &node.faces;
        let mut checked = vec![false; faces.len()];
        let mut best: Option<(usize, f32)> = None;
        for i in 0..faces.len() {
            if checked[i] {
                continue;
            }
            let plane = pool.planes[faces[i].plane_idx];
            let mut facing = 0i32;
            let mut splits = 0i32;
            for (k, f) in faces.iter().enumerate() {
                if f.plane_idx == faces[i].plane_idx {
                    facing += 1;
                    checked[k] = true;
                } else if winding_against_plane(&f.winding, plane.n, plane.d) == -2 {
                    splits += 1;
                }
            }
            // `plane->type < 3` = the three axial plane types.
            let axial = plane.n.x == 1.0 || plane.n.y == 1.0 || plane.n.z == 1.0;
            let score = 5.0 * (facing - splits) as f32 + if axial { 5.0 } else { 0.0 };
            if best.map_or(true, |(_, bs)| score > bs) {
                best = Some((faces[i].plane_idx, score));
            }
        }
        best.map(|(idx, _)| idx)
    }

    fn make_tree_portals_r_inner(
        &mut self,
        node_idx: usize,
        depth: usize,
        parent_planes: &[(usize, bool)],
    ) {
        if depth > self.max_depth {
            self.max_depth = depth;
        }
        if self.nodes[node_idx].leaf {
            return;
        }

        if depth >= self.depth_limit || self.nodes[node_idx].faces.is_empty() {
            // CoD BuildTree_r: no faces left -> leaf. Brushes still here
            // make it solid.
            self.nodes[node_idx].leaf = true;
            return;
        }

        let split = {
            let node = &self.nodes[node_idx];
            match self.selection {
                SplitterSelection::CodFaces => {
                    Self::select_split_plane_cod(node, &mut self.pool)
                }
                SplitterSelection::BrushScoring => {
                    let planes: Vec<usize> =
                        parent_planes.iter().map(|(p, _)| *p).collect();
                    self.select_split_plane_brush(node, &planes)
                }
            }
        };
        let Some(split) = split else {
            self.nodes[node_idx].leaf = true;
            return;
        };

        // Partition the brushes (solid/open classification).
        let plane = self.pool.planes[split];
        let node = &mut self.nodes[node_idx];
        let mut front_list: Vec<BspBrush> = Vec::new();
        let mut back_list: Vec<BspBrush> = Vec::new();
        let brushes = std::mem::take(&mut node.brushes);
        for b in &brushes {
            let (f, b2) = split_brush(b, &plane, split, &self.pool);
            if let Some(f) = f {
                front_list.push(f);
            }
            if let Some(b2) = b2 {
                back_list.push(b2);
            }
        }

        // Partition the faces (facebsp.c): crossing faces are clipped,
        // coplanar faces are dropped, which is what keeps a plane from being
        // selected twice on one path.
        let mut front_faces: Vec<BspFace> = Vec::new();
        let mut back_faces: Vec<BspFace> = Vec::new();
        let faces = std::mem::take(&mut node.faces);
        for f in faces {
            let side = winding_against_plane(&f.winding, plane.n, plane.d);
            match side {
                -2 => {
                    let (wf, wb) = split_winding(&f.winding, plane.n, plane.d, FACE_EPSILON);
                    if let Some(wf) = wf {
                        front_faces.push(BspFace {
                            plane_idx: f.plane_idx,
                            winding: wf,
                        });
                    }
                    if let Some(wb) = wb {
                        back_faces.push(BspFace {
                            plane_idx: f.plane_idx,
                            winding: wb,
                        });
                    }
                }
                0 => front_faces.push(f),
                1 => back_faces.push(f),
                _ => {} // coplanar: dropped (decomp @404FAD)
            }
        }

        let planenum = split as i32;
        self.nodes[node_idx].planenum = planenum;
        // Children inherit the full node bounds, then an axial split plane
        // seeds the split axis on each side (facebsp.c).
        let parent_mins = self.nodes[node_idx].mins;
        let parent_maxs = self.nodes[node_idx].maxs;
        let mut front_mins = parent_mins;
        let front_maxs = parent_maxs;
        let back_mins = parent_mins;
        let mut back_maxs = parent_maxs;
        for axis in 0..3usize {
            if plane.n[axis] == 1.0 {
                front_mins[axis] = plane.d;
                back_maxs[axis] = plane.d;
                break;
            }
        }
        let front_idx = self.push_node(TreeNode {
            planenum: PLANENUM_LEAF,
            children: None,
            brushes: front_list,
            faces: front_faces,
            mins: front_mins,
            maxs: front_maxs,
            leaf: false,
            portals: Vec::new(),
        });
        let back_idx = self.push_node(TreeNode {
            planenum: PLANENUM_LEAF,
            children: None,
            brushes: back_list,
            faces: back_faces,
            mins: back_mins,
            maxs: back_maxs,
            leaf: false,
            portals: Vec::new(),
        });
        self.nodes[node_idx].children = Some([front_idx, back_idx]);

        // CoD MakeNodePortal clips the seam against every portal attached to
        // the node; we additionally clip against the full ancestor plane
        // chain, which is the node volume (decomp MakeNodeSeamA).
        self.make_node_portal(node_idx, front_idx, back_idx, split, parent_planes);
        self.split_node_portals(node_idx, front_idx, back_idx, split);

        let mut front_chain = parent_planes.to_vec();
        front_chain.push((split, true));
        let mut back_chain = parent_planes.to_vec();
        back_chain.push((split, false));
        self.make_tree_portals_r_inner(front_idx, depth + 1, &front_chain);
        self.make_tree_portals_r_inner(back_idx, depth + 1, &back_chain);
    }


    /// decomp MakeNodePortal: seam winding on the node plane clipped against
    /// every portal currently attached to the node.
    fn make_node_portal(
        &mut self,
        node_idx: usize,
        front: usize,
        back: usize,
        split: usize,
        ancestors: &[(usize, bool)],
    ) {
        let plane = self.pool.planes[split];
        let mut w = base_winding_for_plane(plane.n, plane.d);
        // Ancestor chain = exact node volume: keep the side this node is on.
        for &(pi, went_front) in ancestors {
            let ap = self.pool.planes[pi];
            let (n, d) = if went_front {
                (ap.n, ap.d)
            } else {
                (-ap.n, -ap.d)
            };
            match clip_winding_front(&w, n, d, 0.001) {
                Some(clipped) => w = clipped,
                None => return, // node has no volume on this plane
            }
        }
        let attached: Vec<usize> = self.nodes[node_idx].portals.clone();
        for pi in attached {
            let portal = &self.portals[pi];
            let (n, d) = if portal.nodes[0] == node_idx {
                (portal.plane.n, portal.plane.d)
            } else {
                (-portal.plane.n, -portal.plane.d)
            };
            match clip_winding_front(&w, n, d, EPS_PORTAL) {
                Some(clipped) => w = clipped,
                None => return, // eaten
            }
        }
        if w.len() < 3 || winding_is_tiny(&w) {
            return;
        }
        let pi = self.portals.len();
        self.portals.push(TreePortal {
            winding: w,
            plane,
            nodes: [front, back],
        });
        self.nodes[front].portals.push(pi);
        self.nodes[back].portals.push(pi);
    }

    /// decomp SplitNodePortals: partition the node's attached portals down to
    /// the children, splitting windings on the node plane.
    fn split_node_portals(&mut self, node_idx: usize, front: usize, back: usize, split: usize) {
        let plane = self.pool.planes[split];
        let attached = std::mem::take(&mut self.nodes[node_idx].portals);
        for pi in attached {
            let (other, on_front) = {
                let p = &self.portals[pi];
                if p.nodes[0] == node_idx {
                    (p.nodes[1], true)
                } else if p.nodes[1] == node_idx {
                    (p.nodes[0], false)
                } else {
                    // mislinked
                    continue;
                }
            };
            // Detach from `node_idx` on the other side record.
            let w = self.portals[pi].winding.clone();
            let (wf, wb) = split_winding(&w, plane.n, plane.d, EPS_SPLIT);

            // Tiny pieces are dropped (accounting like the decomp: each counts
            // on both endpoints; we just drop them).
            let wf = wf.filter(|w| !winding_is_tiny(w));
            let wb = wb.filter(|w| !winding_is_tiny(w));

            match (wf, wb) {
                (Some(f), Some(b)) => {
                    self.portals[pi].winding = f;
                    self.portals[pi].nodes = if on_front {
                        [front, other]
                    } else {
                        [other, front]
                    };
                    self.nodes[front].portals.push(pi);
                    let np = self.portals.len();
                    self.portals.push(TreePortal {
                        winding: b,
                        plane: self.portals[pi].plane,
                        nodes: if on_front {
                            [back, other]
                        } else {
                            [other, back]
                        },
                    });
                    self.nodes[back].portals.push(np);
                    self.nodes[other].portals.push(np);
                }
                (Some(f), None) => {
                    self.portals[pi].winding = f;
                    self.portals[pi].nodes = if on_front {
                        [front, other]
                    } else {
                        [other, front]
                    };
                    self.nodes[front].portals.push(pi);
                }
                (None, Some(b)) => {
                    self.portals[pi].winding = b;
                    self.portals[pi].nodes = if on_front {
                        [back, other]
                    } else {
                        [other, back]
                    };
                    self.nodes[back].portals.push(pi);
                }
                (None, None) => {
                    // Portal vanished: unlink everywhere. The `other` node may
                    // still reference it; prune lazily during collection.
                    self.portals[pi].nodes = [usize::MAX, usize::MAX];
                }
            }
        }
    }
}



// ---------------------------------------------------------------------------
// Candidate filtering + brush emission
// ---------------------------------------------------------------------------

/// Which kind of volume a leaf holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeafKind {
    Solid,
    Open,
    Outside,
    Internal,
}

fn leaf_kind(tree: &PortalTree, i: usize) -> LeafKind {
    let n = &tree.nodes[i];
    if !n.leaf {
        return LeafKind::Internal;
    }
    if tree.node_is_outside(i) {
        return LeafKind::Outside;
    }
    if !n.brushes.is_empty() {
        LeafKind::Solid
    } else {
        LeafKind::Open
    }
}

/// Locate the leaf containing a point (downward walk by plane tests).
fn locate_leaf(tree: &PortalTree, pt: Vec3) -> usize {
    let mut i = 0usize; // headnode
    loop {
        let node = &tree.nodes[i];
        if node.leaf {
            return i;
        }
        let plane = &tree.pool.planes[node.planenum as usize];
        let (dot, _) = classify(pt, plane.n, plane.d, 0.0);
        let children = node.children.expect("non-leaf has children");
        i = if dot >= 0.0 { children[0] } else { children[1] };
    }
}

/// Fraction of the winding's perimeter whose immediate outside is solid
/// world (a jamb, sill, lintel or wall face), weighted by edge length.
/// A mapper-portal-worthy opening is framed by solid geometry;
/// void-shell cross-sections are not. Length weighting (rather than
/// per-edge counts) keeps long wall-span edges from being outvoted by
/// clusters of short trim-sliver edges on fragmented leaf portals.
fn solid_perimeter_fraction(tree: &PortalTree, winding: &Winding) -> f32 {
    let Some(plane) = winding_plane(winding) else {
        return 0.0;
    };
    let n = winding.len();
    let mut solid_len = 0.0f32;
    let mut total_len = 0.0f32;
    for i in 0..n {
        let a = winding[i];
        let b = winding[(i + 1) % n];
        let edge = b - a;
        let len = edge.length();
        if len < 0.5 {
            continue;
        }
        total_len += len;
        // Outward in-plane edge normal: edge_dir x plane_normal points away
        // from the winding interior for CCW windings.
        let dir = edge * (1.0 / len);
        let mut out_n = dir.cross(plane.n);
        if out_n.length_squared() < 1.0e-12 {
            continue;
        }
        out_n = out_n.normalize();
        let mid = (a + b) * 0.5 + out_n * 2.0;
        if leaf_kind(tree, locate_leaf(tree, mid)) == LeafKind::Solid {
            solid_len += len;
        }
    }
    if total_len <= 0.0 {
        0.0
    } else {
        solid_len / total_len
    }
}

/// Extract leaf-portal candidates: portals between two open leaves, neither
/// of which is the outside void, with their perimeter framed by solid world.
fn collect_candidates(tree: &PortalTree) -> Vec<(Winding, Plane)> {
    let mut out = Vec::new();
    for p in tree.portals.iter() {
        let [a, b] = p.nodes;
        if a == usize::MAX || b == usize::MAX {
            continue;
        }
        if !tree.nodes[a].leaf || !tree.nodes[b].leaf {
            continue;
        }
        if tree.node_is_outside(a) || tree.node_is_outside(b) {
            continue;
        }
        if leaf_kind(&tree, a) != LeafKind::Open || leaf_kind(&tree, b) != LeafKind::Open {
            continue;
        }
        if p.winding.len() < 3 {
            continue;
        }
        out.push((p.winding.clone(), p.plane));
    }
    out
}

/// March from `start` along `dir` through open leaves and return how many
/// steps stay in open space (stops at solid leaves, the outside void and
/// after 2048 units). Measures how deep the open cell behind a portal
/// runs, for portal-side selection.
fn open_run_length(tree: &PortalTree, start: Vec3, dir: Vec3) -> usize {
    const STEP: f32 = 8.0;
    const MAX_STEPS: usize = 256; // 2048 world units
    let mut run = 0usize;
    let mut p = start;
    for _ in 0..MAX_STEPS {
        if leaf_kind(tree, locate_leaf(tree, p)) != LeafKind::Open {
            break;
        }
        run += 1;
        p += dir * STEP;
    }
    run
}

/// Emit one convex portal brush for a winding: the winding extruded
/// +-thickness/2 along the plane normal.
fn portal_brush_from_winding(
    id: BrushId,
    winding: &Winding,
    plane: Plane,
    thickness: f32,
    textures: &crate::portals::PortalTextures,
    side: crate::portals::PortalSide,
) -> Option<Brush> {
    if winding.len() < 3 {
        return None;
    }
    let half = thickness * 0.5;
    let offset = plane.n * half;
    let front: Vec<Vec3> = winding.iter().map(|p| *p + offset).collect();
    let back: Vec<Vec3> = winding.iter().map(|p| *p - offset).collect();
    let params = crate::editing::default_texture_params();

    // Faces carry INWARD-pointing plane points in this codebase
    // (see convex_brush_from_aabb): wound so cross(p1-p0, p2-p0) points
    // into the prism.
    let mut faces: Vec<crate::map::Face> = Vec::with_capacity(winding.len() + 2);

    // Cap faces: one plane each, three points off the cap (a Face is a
    // plane + texture; the mesh is rebuilt from planes later). Front cap at
    // +half, inward normal -n; back cap at -half, inward normal +n.
    // Skip windings too small to give 3 distinct cap points.
    if front.len() >= 3 {
        faces.push(crate::map::Face {
            plane_points: [front[0], front[1], front[2]],
            texture: textures.nodraw.clone(),
            params,
        });
        faces.push(crate::map::Face {
            plane_points: [back[0], back[1], back[2]],
            texture: textures.nodraw.clone(),
            params,
        });
    }
    // Side quads: one per winding edge.
    for k in 0..winding.len() {
        let j = (k + 1) % winding.len();
        let f0 = front[k];
        let f1 = front[j];
        let b0 = back[k];
        if is_flat_degenerate(f0, f1, b0) {
            continue;
        }
        faces.push(crate::map::Face {
            plane_points: [f1, f0, b0],
            texture: textures.nodraw.clone(),
            params,
        });
    }

    if faces.len() < 4 {
        return None;
    }

    // Sanity: every face normal must point towards the prism interior
    // (the winding centroid at mid-thickness).
    let centroid = winding.iter().copied().sum::<Vec3>() * (1.0 / winding.len() as f32);
    let interior = centroid;
    for f in &mut faces {
        let n = face_plane_normal(f);
        let a = f.plane_points[0];
        if (interior - a).dot(n) < 0.0 {
            f.plane_points.swap(1, 2);
        }
    }

    // The active portal plane: the cap face on the requested side. With the
    // inward-normal convention this mirrors generate_opening_portal_brush:
    // Negative -> the face whose inward normal points along -n (the +half
    // cap), Positive -> the -half cap.
    let want = match side {
        crate::portals::PortalSide::Negative => -1.0f32,
        crate::portals::PortalSide::Positive => 1.0f32,
    };
    for f in &mut faces {
        let n = face_plane_normal(f);
        if (n - plane.n * want).length() < 0.1 && n.dot(plane.n * want) > 0.95 {
            f.texture = textures.portal.clone();
        }
    }

    let mut brush = Brush::new(id, BrushContent::Convex(faces));
    brush.aabb = bounds_of_sides(&[(0, front), (0, back)]);
    Some(brush)
}

fn is_flat_degenerate(a: Vec3, b: Vec3, c: Vec3) -> bool {
    (b - a).cross(c - a).length_squared() < 1.0e-8
}

/// Prune-pass thresholds (tuned on dawnville; see PORTALS-STATUS.md).
/// Drop a portal only when EVERY face sample sits inside solid world on
/// both sides: partially buried slabs still cover a real opening on their
/// open samples, and the tutorials over-cover on purpose (the compiler
/// trims the portal to the enclosed volume).
const PRUNE_MAX_BLOCKED: f32 = 1.0;
/// Absorption: a portal folds into a larger nearby one at most this far
/// away plane-wise (opposite wall faces are 8-16 apart; stacked floors
/// are 100+ apart and never match).
const ABSORB_MAX_PLANE_DIST: f32 = 24.0;
/// ...whose dilated face rect (this margin) must contain the small
/// portal's rect. Dilation is merge-gap scale: coplanar neighbours were
/// already unioned by the merge, so this reaches stepped trim and the
/// opposite wall face, not distant openings.
const ABSORB_DILATION: f32 = 32.0;
/// ...and whose volumes must touch within this tolerance (stacked
/// openings on different floors have disjoint volumes and stay).
const ABSORB_VOLUME_TOL: f32 = 1.0;

fn extents_of(aabb: &Aabb) -> [f32; 3] {
    [
        aabb.max.x - aabb.min.x,
        aabb.max.y - aabb.min.y,
        aabb.max.z - aabb.min.z,
    ]
}

fn thin_axis_of(e: &[f32; 3]) -> usize {
    let mut axis = 0usize;
    if e[1] < e[axis] {
        axis = 1;
    }
    if e[2] < e[axis] {
        axis = 2;
    }
    axis
}

fn in_plane_axes(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

/// Fraction of a portal slab's face-rect samples that sit inside solid
/// world on BOTH sides (merges can swallow piers; fragments are clean by
/// construction, so this runs on the merged set). Sampled on a 16-unit
/// grid through the slab centre plane.
fn portal_blocked_fraction(tree: &PortalTree, aabb: &Aabb) -> f32 {
    let e = extents_of(aabb);
    let axis = thin_axis_of(&e);
    let (u, v) = in_plane_axes(axis);
    let mid = (aabb.min[axis] + aabb.max[axis]) * 0.5;
    let half = e[axis] * 0.5;
    let nu = ((e[u] / 16.0).ceil() as usize).clamp(1, 32);
    let nv = ((e[v] / 16.0).ceil() as usize).clamp(1, 32);
    let mut blocked = 0usize;
    let mut total = 0usize;
    for iu in 0..nu {
        for iv in 0..nv {
            let mut c = Vec3::ZERO;
            c[u] = aabb.min[u] + e[u] * (iu as f32 + 0.5) / nu as f32;
            c[v] = aabb.min[v] + e[v] * (iv as f32 + 0.5) / nv as f32;
            c[axis] = mid;
            let mut pa = c;
            pa[axis] += half + 1.0;
            let mut pb = c;
            pb[axis] -= half + 1.0;
            total += 1;
            if leaf_kind(tree, locate_leaf(tree, pa)) == LeafKind::Solid
                && leaf_kind(tree, locate_leaf(tree, pb)) == LeafKind::Solid
            {
                blocked += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        blocked as f32 / total as f32
    }
}

/// Whether portal `small` folds into portal `large`: same thin axis,
/// planes close, volumes touching, and the small face rect contained in
/// the large rect dilated by the absorb margin. Same-wall twins (both
/// faces of one opening) always match; stacked floors never do.
fn can_absorb(large: &Aabb, small: &Aabb) -> bool {
    let el = extents_of(large);
    let es = extents_of(small);
    if thin_axis_of(&el) != thin_axis_of(&es) {
        return false;
    }
    let axis = thin_axis_of(&el);
    let (u, v) = in_plane_axes(axis);
    let midl = (large.min[axis] + large.max[axis]) * 0.5;
    let mids = (small.min[axis] + small.max[axis]) * 0.5;
    if (midl - mids).abs() > ABSORB_MAX_PLANE_DIST {
        return false;
    }
    if large.min.x > small.max.x + ABSORB_VOLUME_TOL
        || small.min.x > large.max.x + ABSORB_VOLUME_TOL
        || large.min.y > small.max.y + ABSORB_VOLUME_TOL
        || small.min.y > large.max.y + ABSORB_VOLUME_TOL
        || large.min.z > small.max.z + ABSORB_VOLUME_TOL
        || small.min.z > large.max.z + ABSORB_VOLUME_TOL
    {
        return false;
    }
    small.min[u] >= large.min[u] - ABSORB_DILATION
        && small.max[u] <= large.max[u] + ABSORB_DILATION
        && small.min[v] >= large.min[v] - ABSORB_DILATION
        && small.max[v] <= large.max[v] + ABSORB_DILATION
}

fn rect_area(aabb: &Aabb) -> f32 {
    let e = extents_of(aabb);
    let (u, v) = in_plane_axes(thin_axis_of(&e));
    e[u] * e[v]
}

/// Second pass over the merged portals: fold small slabs into larger
/// nearby ones (which expand to cover them), then drop slabs buried in
/// solid. Coverage only grows through absorption (unions), so unlike
/// dropping it cannot strand an opening. Removes the dropped brushes
/// from the map. Returns how many portals were pruned.
fn prune_placed_portals(
    map: &mut Map,
    placed: &mut Vec<crate::portals::PlacedPortal>,
    tree: &PortalTree,
    textures: &crate::portals::PortalTextures,
    // Largest allowed absorption union extent: chains propagate through
    // neighboring fragments, so cap them like the merge does.
    max_extent: f32,
) -> usize {
    let before = placed.len();
    let mut dropped: std::collections::HashSet<BrushId> = std::collections::HashSet::new();
    // Buried slabs first, on the merged set: fully inside solid on both
    // sides at every sample. Absorption runs second so expanded keepers
    // (supersets, still carrying their open samples) never face this
    // test; sampling an expanded rect could otherwise dodge its notch
    // and kill real coverage. (Partition manually: Vec::retain would
    // drop the tracking entries without recording their brush ids.)
    let mut visible = Vec::with_capacity(placed.len());
    for p in placed.drain(..) {
        let blocked = portal_blocked_fraction(tree, &p.aabb);
        if blocked < PRUNE_MAX_BLOCKED {
            visible.push(p);
        } else {
            if std::env::var("PRUNE_DEBUG").is_ok() {
                let b = &p.aabb;
                eprintln!(
                    "prune buried frac={:.2} x[{:.0},{:.0}] y[{:.0},{:.0}] z[{:.0},{:.0}]",
                    blocked, b.min.x, b.max.x, b.min.y, b.max.y, b.min.z, b.max.z
                );
            }
            dropped.insert(p.id);
        }
    }
    *placed = visible;
    // Absorption, smallest rect first: each portal folds into the
    // smallest larger-or-equal portal (by rect area, then index) that
    // contains it, which expands to the union. Equal-area twins fold in
    // index order; emission order is deterministic. Each portal takes
    // part in at most one event (as either side): keepers freeze after
    // expanding, so folds stay local and cannot chain across districts.
    let mut order: Vec<usize> = (0..placed.len()).collect();
    order.sort_by(|&i, &j| {
        rect_area(&placed[i].aabb)
            .partial_cmp(&rect_area(&placed[j].aabb))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| i.cmp(&j))
    });
    let mut gone = vec![false; placed.len()];
    let mut done = vec![false; placed.len()];
    for pos in 0..order.len() {
        let i = order[pos];
        if gone[i] || done[i] {
            continue;
        }
        // First fit scanning upward = smallest sufficient absorber.
        let mut absorber: Option<usize> = None;
        for &j in order.iter().skip(pos + 1) {
            if gone[j] || done[j] {
                continue;
            }
            if can_absorb(&placed[j].aabb, &placed[i].aabb) {
                absorber = Some(j);
                break;
            }
        }
        let Some(j) = absorber else {
            continue;
        };
        // Expand the absorber to the union and rebuild its brush (same
        // id keeps tracking stable); the small portal's brush goes.
        // Unions stay capped like the merge step: no continent chains.
        let mut aabb = placed[j].aabb;
        aabb.min = aabb.min.min(placed[i].aabb.min);
        aabb.max = aabb.max.max(placed[i].aabb.max);
        let ue = [
            aabb.max.x - aabb.min.x,
            aabb.max.y - aabb.min.y,
            aabb.max.z - aabb.min.z,
        ];
        if ue[0].max(ue[1].max(ue[2])) > max_extent {
            continue;
        }
        let id = placed[j].id;
        let entity_idx = placed[j].entity;
        let brush = crate::portals::generate_opening_portal_brush(
            id,
            &aabb,
            placed[j].side,
            textures,
        );
        if let Some(entity) = map.entities.get_mut(entity_idx) {
            entity.brushes.retain(|b| b.id != id && b.id != placed[i].id);
            entity.brushes.push(brush);
        }
        if std::env::var("PRUNE_DEBUG").is_ok() {
            let s = &placed[i].aabb;
            eprintln!(
                "prune absorb x[{:.0},{:.0}] y[{:.0},{:.0}] z[{:.0},{:.0}] into id={}",
                s.min.x, s.max.x, s.min.y, s.max.y, s.min.z, s.max.z, id.0
            );
        }
        placed[j].aabb = aabb;
        gone[i] = true;
        done[i] = true;
        done[j] = true;
        dropped.insert(placed[i].id);
    }
    // Survivors, preserving order.
    let mut survivors = Vec::with_capacity(placed.len());
    for (idx, p) in placed.drain(..).enumerate() {
        if !gone[idx] {
            survivors.push(p);
        }
    }
    *placed = survivors;
    for entity in map.entities.iter_mut() {
        entity.brushes.retain(|b| !dropped.contains(&b.id));
    }
    before - placed.len()
}

// ---------------------------------------------------------------------------

/// Build the BSP for the map's worldspawn structural brushes and emit a
/// portal brush for every leaf portal a mapper would mark. Returns the
/// number of portal brushes created.
pub fn generate_bsp_portals(
    map: &mut Map,
    side: crate::portals::PortalSide,
    textures: &crate::portals::PortalTextures,
    params: &BspPortalParams,
) -> Result<BspPortalReport, String> {
    let mut report = BspPortalReport::default();
    let mut pool = PlanePool::default();

    // Structural brushes: worldspawn, non-patch, non-detail textures.
    let mut structural: Vec<BspBrush> = Vec::new();
    if let Some(world) = map.entities.first() {
        for brush in &world.brushes {
            if matches!(brush.content, BrushContent::Patch(_)) {
                continue;
            }
            let detail = match &brush.content {
                BrushContent::Convex(faces) => {
                    faces.iter().any(|f| is_detail_texture(&f.texture))
                }
                BrushContent::Patch(_) => true,
            };
            if detail {
                continue;
            }
            if let Some(b) = bsp_brush_from_map_brush(brush, 0, &mut pool) {
                structural.push(b);
            }
        }
    }
    report.structural_brushes = structural.len();
    if structural.is_empty() {
        return Err("no structural brushes in worldspawn".to_string());
    }

    // Build tree + portals.
    let tree = PortalTree::build(pool, structural, params.selection, params.max_depth);
    report.max_depth = tree.max_depth;
    for node in &tree.nodes {
        if node.leaf {
            report.leaves += 1;
            if !node.brushes.is_empty() {
                report.solid_leaves += 1;
            }
        }
    }
    report.internal_portals = tree.portals.len();

    // Candidates: void-void leaf portals.
    let candidates = collect_candidates(&tree);
    report.candidates = candidates.len();

    // Filter + emit: keep openings whose perimeter is mostly solid-framed
    // (doorways, windows, arches). With `area_separators` also keep the
    // larger open-area cross-sections, which need only partial framing.
    let world_brush_count = map.entities.first().map(|e| e.brushes.len()).unwrap_or(0);
    let mut next_id = map
        .entities
        .iter()
        .flat_map(|e| e.brushes.iter().map(|b| b.id.0))
        .max()
        .map_or(0, |id| id.wrapping_add(1));
    let mut placed: Vec<crate::portals::PlacedPortal> = Vec::new();
    for (winding, plane) in candidates {
        // Reject uncarved void cross-sections that reach the world bound.
        let wb = winding_bounds(&winding);
        let max_ext = (wb.max.x - wb.min.x)
            .max((wb.max.y - wb.min.y).max(wb.max.z - wb.min.z));
        if max_ext > params.max_extent {
            continue;
        }
        let framed = solid_perimeter_fraction(&tree, &winding);
        let keep = if params.area_separators {
            framed >= 0.3
        } else {
            framed >= 0.8
        };
        if std::env::var("BSP_DEBUG").is_ok() {
            let wb2 = winding_bounds(&winding);
            eprintln!(
                "cand: n=({},{},{}) d={:.0} x[{:.0},{:.0}] y[{:.0},{:.0}] z[{:.0},{:.0}] framed={:.2} keep={}",
                plane.n.x, plane.n.y, plane.n.z, plane.d,
                wb2.min.x, wb2.max.x, wb2.min.y, wb2.max.y, wb2.min.z, wb2.max.z, framed, keep
            );
        }
        if !keep {
            continue;
        }
        // Winding sanity: min edge + area.
        let mut min_edge = f32::MAX;
        for i in 0..winding.len() {
            let j = (i + 1) % winding.len();
            min_edge = min_edge.min((winding[j] - winding[i]).length());
        }
        if min_edge < params.min_edge {
            continue;
        }
        if winding_area(&winding) < params.min_area {
            continue;
        }
        // PortalSide: IW orients the active face toward the larger cell.
        // March open space behind both caps; the deeper run wins. With
        // the inward-normal convention Negative faces the +normal (front)
        // leaf, Positive the back leaf; ties keep Negative.
        let face_side = if params.auto_side {
            let centroid = winding.iter().copied().sum::<Vec3>() * (1.0 / winding.len() as f32);
            let off = params.thickness * 0.5 + 2.0;
            let front_open = open_run_length(&tree, centroid + plane.n * off, plane.n);
            let back_open = open_run_length(&tree, centroid - plane.n * off, -plane.n);
            let picked = if front_open > back_open {
                crate::portals::PortalSide::Negative
            } else {
                crate::portals::PortalSide::Positive
            };
            if std::env::var("BSP_DEBUG").is_ok() {
                eprintln!(
                    "side: n=({},{},{}) d={:.0} centroid=({:.0},{:.0},{:.0}) front_run={} back_run={} -> {:?}",
                    plane.n.x, plane.n.y, plane.n.z, plane.d,
                    centroid.x, centroid.y, centroid.z, front_open, back_open, picked
                );
            }
            picked
        } else {
            side
        };
        let brush =
            portal_brush_from_winding(BrushId(next_id), &winding, plane, params.thickness, textures, face_side);
        next_id += 1;
        let Some(brush) = brush else { continue };
        // Track the analytic prism bounds (exact), not a kernel-derived
        // AABB: the constructor above already rejected brushes whose
        // kernel geometry disagrees with them.
        let id = brush.id;
        let aabb = brush.aabb;
        if let Some(world) = map.entities.first_mut() {
            world.brushes.push(brush);
            placed.push(crate::portals::PlacedPortal {
                entity: 0,
                id,
                aabb,
                side: face_side,
            });
        }
    }

    // Over-cover like the originals: union coplanar near-touching portals.
    let merged = crate::portals::merge_placed_portals_public(map, &mut placed, params.merge_gap, side, textures, params.max_union);
    report.portals_merged = merged;
    // Second pass: drop slabs buried in solid and collapse twin slabs
    // covering one opening from parallel planes.
    report.portals_pruned = prune_placed_portals(map, &mut placed, &tree, textures, params.max_union);
    report.portals_created = placed.len();
    let _ = world_brush_count;

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

    #[test]
    fn base_winding_lies_on_the_plane() {
        let n = Vec3::new(0.0, 1.0, 0.0);
        let w = base_winding_for_plane(n, -5.0);
        assert_eq!(w.len(), 4);
        for p in &w {
            assert!((p.y + 5.0).abs() < 0.01);
        }
        assert!(winding_area(&w) > 0.0);
    }

    #[test]
    fn clip_keeps_front_only() {
        let w = base_winding_for_plane(Vec3::Z, 0.0);
        // Keep z < 16 (front of plane z=16 with normal -Z? front = +n side).
        let clipped = clip_winding_front(&w, Vec3::new(0.0, 0.0, -1.0), -16.0, 0.1)
            .expect("some winding survives");
        for p in &clipped {
            assert!(p.z <= 16.0 + 0.1);
        }
    }

    #[test]
    fn split_brush_cuts_a_box_in_two() {
        let mut pool = PlanePool::default();
        let b = bsp_brush_from_map_brush(
            &box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0)),
            0,
            &mut pool,
        )
        .expect("box");
        let (front, back) = split_brush(&b, &Plane { n: Vec3::X, d: 32.0 }, pool.find(Vec3::X, 32.0), &pool);
        let f = front.expect("front half");
        let bk = back.expect("back half");
        // Front = the +plane side (x 32..64), back = x 0..32.
        assert!((f.bounds.min.x - 32.0).abs() < 0.2 && (f.bounds.max.x - 64.0).abs() < 0.2);
        assert!((bk.bounds.min.x - 0.0).abs() < 0.2 && (bk.bounds.max.x - 32.0).abs() < 0.2);
    }

    #[test]
    fn plane_pool_snaps_near_axial_normals() {
        let mut pool = PlanePool::default();
        // Float-error axial (the dawnville x=-2048 sliver-loop plane).
        let a = pool.find(Vec3::new(0.99999994, 0.0, 0.0), -2048.0);
        assert_eq!(pool.planes[a].n, Vec3::X);
        // The exact grid plane merges into it instead of coexisting.
        let b = pool.find(Vec3::X, -2048.0);
        assert_eq!(a, b);
        // Genuinely rotated walls are untouched.
        let c = pool.find(Vec3::new(0.116835214, -0.9931513, 0.0), 16871.91);
        assert!((pool.planes[c].n.x - 0.116835214).abs() < 1.0e-6);
    }

    #[test]
    fn visible_hull_consumes_abutting_faces() {
        // Two boxes sharing the x=64 plane with identical footprints:
        // the touching faces are interior and must vanish, while the far
        // faces survive whole.
        let mut pool = PlanePool::default();
        let a = bsp_brush_from_map_brush(&box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0)), 0, &mut pool)
            .expect("box a");
        let b = bsp_brush_from_map_brush(&box_brush((64.0, 0.0, 0.0), (128.0, 64.0, 64.0)), 0, &mut pool)
            .expect("box b");
        let b_planes: Vec<Plane> = b.sides.iter().filter_map(|(_, w)| winding_plane(w)).collect();
        assert_eq!(b_planes.len(), 6);
        let side_at = |brush: &BspBrush, n: Vec3| -> Winding {
            brush
                .sides
                .iter()
                .find(|(_, w)| {
                    winding_plane(w).map_or(false, |p| (p.n - n).length() < 0.01)
                })
                .map(|(_, w)| w.clone())
                .expect("side")
        };
        // Touching face (a's +x at x=64): buried inside b -> nothing.
        let touching = side_at(&a, Vec3::X);
        assert!((winding_area(&touching) - 64.0 * 64.0).abs() < 1.0);
        assert!(subtract_brush(&touching, &b_planes, 0.1).is_empty());
        // Far face (a's -x at x=0): fully outside -> one identical piece.
        let far = side_at(&a, -Vec3::X);
        let kept = subtract_brush(&far, &b_planes, 0.1);
        assert_eq!(kept.len(), 1);
        assert!((winding_area(&kept[0]) - 64.0 * 64.0).abs() < 1.0);
    }

    #[test]
    fn visible_hull_keeps_exposed_overhang() {
        // Box b covers only half of a's +x face in y: the exposed half
        // must survive as one piece of half the area.
        let mut pool = PlanePool::default();
        let a = bsp_brush_from_map_brush(&box_brush((0.0, 0.0, 0.0), (64.0, 64.0, 64.0)), 0, &mut pool)
            .expect("box a");
        let b = bsp_brush_from_map_brush(&box_brush((64.0, 32.0, 0.0), (128.0, 96.0, 64.0)), 0, &mut pool)
            .expect("box b");
        let b_planes: Vec<Plane> = b.sides.iter().filter_map(|(_, w)| winding_plane(w)).collect();
        let face = a
            .sides
            .iter()
            .find(|(_, w)| winding_plane(w).map_or(false, |p| (p.n - Vec3::X).length() < 0.01))
            .map(|(_, w)| w.clone())
            .expect("+x side");
        let kept = subtract_brush(&face, &b_planes, 0.1);
        assert_eq!(kept.len(), 1);
        assert!((winding_area(&kept[0]) - 64.0 * 32.0).abs() < 2.0);
    }

    /// A sealed room with an interior wall that has a doorway. The BSP must
    /// find a void-void leaf portal exactly in the doorway.
    #[test]
    fn doorway_between_two_rooms_yields_a_leaf_portal() {
        let mut pool = PlanePool::default();
        let mut brushes = Vec::new();
        // Floor + ceiling.
        brushes.push(box_brush((-64.0, -64.0, -16.0), (192.0, 192.0, 0.0)));
        brushes.push(box_brush((-64.0, -64.0, 128.0), (192.0, 192.0, 144.0)));
        // Outer walls.
        brushes.push(box_brush((-80.0, -80.0, 0.0), (-64.0, 208.0, 128.0)));
        brushes.push(box_brush((192.0, -80.0, 0.0), (208.0, 208.0, 128.0)));
        brushes.push(box_brush((-80.0, -80.0, 0.0), (208.0, -64.0, 128.0)));
        brushes.push(box_brush((-80.0, 192.0, 0.0), (208.0, 208.0, 128.0)));
        // Interior wall across x=64, thickness 8, with a doorway
        // y 48..112, z 0..88 (lintel above).
        brushes.push(box_brush((64.0, -64.0, 0.0), (72.0, 48.0, 128.0)));
        brushes.push(box_brush((64.0, 112.0, 0.0), (72.0, 192.0, 128.0)));
        brushes.push(box_brush((64.0, 48.0, 88.0), (72.0, 112.0, 128.0)));

        let mut structural = Vec::new();
        for b in &brushes {
            if let Some(b) = bsp_brush_from_map_brush(b, 0, &mut pool) {
                structural.push(b);
            }
        }
        assert_eq!(structural.len(), 9);

        // End-to-end: emit portal brushes through the full pipeline and
        // require the doorway cross-section to be covered.
        let mut map = crate::map::Map::default();
        map.entities.push(crate::map::Entity {
            id: crate::map::EntityId(0),
            classname: "worldspawn".to_string(),
            properties: Default::default(),
            brushes,
            model: None,
        });
        let textures = crate::portals::PortalTextures::default();
        let _ = &textures;
        let report = generate_bsp_portals(
            &mut map,
            crate::portals::PortalSide::Negative,
            &textures,
            &BspPortalParams::default(),
        )
        .expect("bsp portals");
        eprintln!("doorway e2e report: {report:?}");

        // The doorway void spans x 64..72, y 48..112, z 0..88. Some emitted
        // portal brush must sit in there, thin along x, covering a decent
        // chunk of the 64x88 cross-section.
        let doorway = Aabb::from_points(
            Vec3::new(63.5, 47.5, -0.5),
            Vec3::new(72.5, 112.5, 88.5),
        );

        let mut best = 0.0f32;
        for b in &map.entities[0].brushes {
            let Some(aabb) = crate::portals::aabb_of_brush_pub(b) else {
                continue;
            };
            // Overlap of the brush with the doorway column.
            let dx = (aabb.max.x.min(doorway.max.x) - aabb.min.x.max(doorway.min.x)).max(0.0);
            let dy = (aabb.max.y.min(doorway.max.y) - aabb.min.y.max(doorway.min.y)).max(0.0);
            let dz = (aabb.max.z.min(doorway.max.z) - aabb.min.z.max(doorway.min.z)).max(0.0);
            if dx <= 0.0 || aabb.max.x - aabb.min.x > 16.0 {
                continue; // must be thin along x, inside the wall
            }
            best = best.max(dy * dz);
        }
        assert!(
            best >= 48.0 * 64.0 * 0.5,
            "doorway coverage too small: {best}"
        );
    }

    #[test]
    fn kernel_holds_thin_sliver_at_dawnville_scale() {
        // Repro for a 75k-unit bogus AABB seen on dawnville: a thin flat
        // winding at large coordinates must round-trip through the
        // geometry kernel with its true bounds, not base-winding
        // remnants or far intersections.
        let winding = vec![
            Vec3::new(-3398.0, -17397.0, 88.0),
            Vec3::new(-3296.0, -17397.0, 88.0),
            Vec3::new(-3296.0, -17376.0, 88.0),
            Vec3::new(-3398.0, -17376.0, 88.0),
        ];
        let textures = crate::portals::PortalTextures::default();
        let brush = portal_brush_from_winding(
            BrushId(99),
            &winding,
            Plane { n: Vec3::Z, d: 88.0 },
            8.0,
            &textures,
            crate::portals::PortalSide::Negative,
        )
        .expect("brush");
        let aabb = crate::portals::aabb_of_brush_pub(&brush).expect("aabb");
        let e = [aabb.max.x - aabb.min.x, aabb.max.y - aabb.min.y, aabb.max.z - aabb.min.z];
        eprintln!("sliver aabb ext={e:?}");
        assert!(e[0] < 1000.0 && e[1] < 1000.0 && e[2] < 100.0, "no blowup: {e:?}");
    }

    /// Scale limits of the geometry kernel at dawnville coordinates:
    /// axis boxes must round-trip exactly. Documents how far union
    /// extents can be trusted (coverage reads kernel AABBs).
    #[test]
    fn kernel_box_scale_limits() {
        for (name, min, max) in [
            ("8k box", (-3398.0, -17397.0, 88.0), (4794.0, -9205.0, 96.0)),
            ("16k box", (-8000.0, -17000.0, 80.0), (8000.0, -1000.0, 96.0)),
        ] {
            let b = box_brush(min, max);
            let mut owned = b.clone();
            owned.invalidate_geometry();
            let polys = crate::geometry::brush_to_polygons(&owned).expect("polys");
            let aabb = crate::editing::aabb_from_polys(&polys);
            let e = [aabb.max.x - aabb.min.x, aabb.max.y - aabb.min.y, aabb.max.z - aabb.min.z];
            let want = [max.0 - min.0, max.1 - min.1, max.2 - min.2];
            eprintln!("{name}: got {e:?} want {want:?}");
            for k in 0..3 {
                assert!((e[k] - want[k]).abs() < 2.0, "{name} axis {k}: {e:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn prune_helpers_burial_and_absorb() {
        // One solid wall box: slabs inside it read fully blocked, slabs
        // in open air read clean.
        let mut pool = PlanePool::default();
        let wall = bsp_brush_from_map_brush(&box_brush((0.0, 0.0, 0.0), (8.0, 64.0, 64.0)), 0, &mut pool)
            .expect("wall");
        let tree = PortalTree::build(pool, vec![wall], SplitterSelection::CodFaces, 200);
        let buried = Aabb::from_points(Vec3::new(2.0, 8.0, 8.0), Vec3::new(6.0, 56.0, 56.0));
        assert!((portal_blocked_fraction(&tree, &buried) - 1.0).abs() < 1.0e-6);
        let open = Aabb::from_points(Vec3::new(16.0, 8.0, 8.0), Vec3::new(24.0, 56.0, 56.0));
        assert!((portal_blocked_fraction(&tree, &open)).abs() < 1.0e-6);

        // Absorption predicate: same-wall neighbours fold (either
        // direction by size), stacked floors and far planes do not.
        let big = Aabb::from_points(Vec3::new(0.0, 0.0, 0.0), Vec3::new(8.0, 64.0, 64.0));
        let face = Aabb::from_points(Vec3::new(8.0, 0.0, 0.0), Vec3::new(16.0, 64.0, 64.0));
        assert!(can_absorb(&big, &face));
        assert!(can_absorb(&face, &big));
        let cube = Aabb::from_points(Vec3::new(8.0, 0.0, 0.0), Vec3::new(16.0, 8.0, 8.0));
        assert!(can_absorb(&big, &cube));
        assert!(!can_absorb(&cube, &big));
        // Same plane but disjoint rect: separate openings.
        let c = Aabb::from_points(Vec3::new(0.0, 100.0, 0.0), Vec3::new(8.0, 164.0, 64.0));
        assert!(!can_absorb(&big, &c));
        assert!(!can_absorb(&c, &big));
        // Stacked floors: disjoint volumes.
        let d = Aabb::from_points(Vec3::new(0.0, 0.0, 200.0), Vec3::new(8.0, 64.0, 264.0));
        assert!(!can_absorb(&big, &d));
        // Different axis.
        let e = Aabb::from_points(Vec3::new(0.0, 0.0, 0.0), Vec3::new(64.0, 8.0, 64.0));
        assert!(!can_absorb(&big, &e));
        // Parallel but a wall apart.
        let f = Aabb::from_points(Vec3::new(64.0, 0.0, 0.0), Vec3::new(72.0, 64.0, 64.0));
        assert!(!can_absorb(&big, &f));
        assert!(!can_absorb(&f, &big));
    }

    /// End to end through prune: a trim cube beside a wall slab folds
    /// into it (one portal left, expanded to cover the cube); a
    /// stacked-slab pair on another floor survives untouched.
    #[test]
    fn prune_absorb_expands_keeper() {
        use crate::map::{BrushId, Entity, EntityId};
        let mut pool = PlanePool::default();
        // Distant solid box so the tree has solid leaves for the burial
        // sampler; test slabs sit in open air near the origin.
        let far = bsp_brush_from_map_brush(
            &box_brush((1000.0, 1000.0, 1000.0), (1064.0, 1064.0, 1064.0)),
            0,
            &mut pool,
        )
        .expect("far box");
        let tree = PortalTree::build(pool, vec![far], SplitterSelection::CodFaces, 200);
        let textures = crate::portals::PortalTextures::default();
        let mut map = crate::map::Map::default();
        map.entities.push(Entity {
            id: EntityId(0),
            classname: "worldspawn".to_string(),
            properties: Default::default(),
            brushes: Vec::new(),
            model: None,
        });
        let mut next_id = 0u32;
        let mut placed = Vec::new();
        // Wall slab (thin x) + trim cube touching its +x face + a
        // stacked pair far above (disjoint volumes).
        let slabs = vec![
            Aabb::from_points(Vec3::new(0.0, 0.0, 0.0), Vec3::new(8.0, 64.0, 64.0)),
            Aabb::from_points(Vec3::new(8.0, 0.0, 0.0), Vec3::new(16.0, 8.0, 8.0)),
            Aabb::from_points(Vec3::new(0.0, 0.0, 200.0), Vec3::new(8.0, 64.0, 264.0)),
            Aabb::from_points(Vec3::new(0.0, 0.0, 300.0), Vec3::new(8.0, 64.0, 364.0)),
        ];
        for aabb in &slabs {
            let brush = crate::portals::generate_opening_portal_brush(
                BrushId(next_id),
                aabb,
                crate::portals::PortalSide::Negative,
                &textures,
            );
            map.entities[0].brushes.push(brush);
            placed.push(crate::portals::PlacedPortal {
                entity: 0,
                id: BrushId(next_id),
                aabb: *aabb,
                side: crate::portals::PortalSide::Negative,
            });
            next_id += 1;
        }
        let pruned = prune_placed_portals(&mut map, &mut placed, &tree, &textures, 8192.0);
        // Only the cube folds (into the wall slab); the stacked pair has
        // disjoint volumes and survives.
        assert_eq!(pruned, 1, "cube absorbed");
        assert_eq!(placed.len(), 3);
        assert_eq!(map.entities[0].brushes.len(), 3);
        // The keeper expanded over the cube's bounds.
        let keeper = placed
            .iter()
            .find(|p| p.aabb.min.y == 0.0 && p.aabb.max.y == 64.0 && p.aabb.max.z == 64.0)
            .expect("expanded wall slab");
        assert!(keeper.aabb.max.x >= 16.0 && keeper.aabb.min.x <= 0.0);
    }

    /// Two rooms of different sizes sharing a doorway wall: with
    /// `auto_side` the emitted portal's active face must point toward the
    /// larger room (IW orients portals toward the larger cell).
    #[test]
    fn auto_side_faces_the_larger_room() {
        // Small room x -28..0, big room x 8..108, doorway wall x 0..8
        // with a doorway y 48..112, z 0..88.
        let rooms = || {
            vec![
                box_brush((-28.0, -64.0, -16.0), (108.0, 192.0, 0.0)),
                box_brush((-28.0, -64.0, 128.0), (108.0, 192.0, 144.0)),
                box_brush((-36.0, -80.0, 0.0), (-28.0, 208.0, 128.0)),
                box_brush((108.0, -80.0, 0.0), (116.0, 208.0, 128.0)),
                box_brush((-36.0, -80.0, 0.0), (116.0, -64.0, 128.0)),
                box_brush((-36.0, 192.0, 0.0), (116.0, 208.0, 128.0)),
                box_brush((0.0, -64.0, 0.0), (8.0, 48.0, 128.0)),
                box_brush((0.0, 112.0, 0.0), (8.0, 192.0, 128.0)),
                box_brush((0.0, 48.0, 88.0), (8.0, 112.0, 128.0)),
            ]
        };
        let run = |auto_side: bool, side: crate::portals::PortalSide| {
            let mut map = crate::map::Map::default();
            map.entities.push(crate::map::Entity {
                id: crate::map::EntityId(0),
                classname: "worldspawn".to_string(),
                properties: Default::default(),
                brushes: rooms(),
                model: None,
            });
            let mut params = BspPortalParams::default();
            params.auto_side = auto_side;
            generate_bsp_portals(
                &mut map,
                side,
                &crate::portals::PortalTextures::default(),
                &params,
            )
            .expect("bsp portals");
            // Portal-textured face inward normals of brushes sitting thin
            // along x inside the doorway column.
            let mut normals = Vec::new();
            for b in &map.entities[0].brushes {
                let Some(aabb) = crate::portals::aabb_of_brush_pub(b) else {
                    continue;
                };
                // Overlap with the doorway column, thin along x.
                if aabb.max.x < -1.0
                    || aabb.min.x > 9.0
                    || aabb.max.x - aabb.min.x > 16.0
                {
                    continue;
                }
                if aabb.max.y < 48.0 || aabb.min.y > 112.0 {
                    continue;
                }
                if aabb.max.z < 0.0 || aabb.min.z > 88.0 {
                    continue;
                }
                if let BrushContent::Convex(faces) = &b.content {
                    for f in faces {
                        if f.texture == "common/portal" {
                            normals.push(crate::texmap::face_plane_normal(f));
                        }
                    }
                }
            }
            normals
        };
        // Auto: active face points toward the big room (+x). With the
        // inward-normal convention that cap's inward normal points -x.
        let auto_normals = run(true, crate::portals::PortalSide::Negative);
        assert!(!auto_normals.is_empty(), "doorway portal emitted");
        for n in &auto_normals {
            assert!(
                (n + Vec3::X).length() < 0.1,
                "auto side faces +x, inward normal {n:?}"
            );
        }
        // Fixed Positive is the opposite cap (inward +x).
        let fixed_normals = run(false, crate::portals::PortalSide::Positive);
        assert!(!fixed_normals.is_empty(), "doorway portal emitted");
        for n in &fixed_normals {
            assert!(
                (n - Vec3::X).length() < 0.1,
                "fixed Positive keeps inward normal {n:?}"
            );
        }
    }

    /// Regression baseline on the in-repo training_outside map: pins the
    /// BSP pass against silent collapse (exact numbers live in
    /// PORTALS-STATUS.md). training_outside is an open arena, so most
    /// portals are area separators.
    #[test]
    fn training_outside_bsp_baseline() {
        let text = include_str!("../test/training_outside.map");
        let mut map = crate::parser::parse_map_string(text).expect("parse");
        let brushes_before = map.entities.iter().map(|e| e.brushes.len()).sum::<usize>();
        let textures = crate::portals::PortalTextures::default();
        let report = generate_bsp_portals(
            &mut map,
            crate::portals::PortalSide::Negative,
            &textures,
            &BspPortalParams::default(),
        )
        .expect("bsp portals");
        eprintln!("training_outside bsp ({brushes_before} brushes): {report:?}");
        // Loose guardrails: catch structural collapse (empty trees, zero
        // emission, runaway depth), not exact tuning. As of the prune
        // pass: 1118 leaves, 1808 candidates, 168 created, depth 18.
        assert!((300..1500).contains(&report.leaves), "leaves {}", report.leaves);
        assert!((50..300).contains(&report.portals_created), "created {}", report.portals_created);
        assert!(report.max_depth <= 100, "depth {}", report.max_depth);
        // Every emitted brush must survive a geometry round-trip.
        for b in &map.entities[0].brushes[brushes_before..] {
            let mut owned = b.clone();
            owned.invalidate_geometry();
            assert!(crate::geometry::brush_to_polygons(&owned).is_ok());
        }
    }

    #[test]
    fn emitted_portal_brush_is_convex_and_portal_textured() {
        let winding = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(64.0, 0.0, 0.0),
            Vec3::new(64.0, 64.0, 0.0),
            Vec3::new(0.0, 64.0, 0.0),
        ];
        let textures = crate::portals::PortalTextures::default();
        let brush = portal_brush_from_winding(
            BrushId(0),
            &winding,
            Plane {
                n: Vec3::Z,
                d: 0.0,
            },
            8.0,
            &textures,
            crate::portals::PortalSide::Negative,
        )
        .expect("brush");
        if let BrushContent::Convex(faces) = &brush.content {
            assert_eq!(
                faces.iter().filter(|f| f.texture == "common/portal").count(),
                1,
                "exactly one active portal face"
            );
            assert!(faces.len() >= 6, "prism needs caps + sides");
            // Round-trip through the geometry code: it must survive as a
            // convex brush.
            let mut b2 = brush.clone();
            b2.invalidate_geometry();
            assert!(crate::geometry::brush_to_polygons(&b2).is_ok());
        } else {
            panic!("expected convex");
        }
    }
}


/// Debug: distribution of perimeter framing over all valid void-void leaf
/// portals (extent-capped). Only for tuning; not part of the public API.
pub fn dbg_framing_histogram(map: &Map) -> Vec<(usize, usize)> {
    let mut pool = PlanePool::default();
    let mut structural: Vec<BspBrush> = Vec::new();
    if let Some(world) = map.entities.first() {
        for brush in &world.brushes {
            if matches!(brush.content, BrushContent::Patch(_)) {
                continue;
            }
            let detail = match &brush.content {
                BrushContent::Convex(faces) => faces.iter().any(|f| is_detail_texture(&f.texture)),
                BrushContent::Patch(_) => true,
            };
            if detail {
                continue;
            }
            if let Some(b) = bsp_brush_from_map_brush(brush, 0, &mut pool) {
                structural.push(b);
            }
        }
    }
    let tree = PortalTree::build(pool, structural, SplitterSelection::default(), BspPortalParams::default().max_depth);
    let cands = collect_candidates(&tree);
    let mut buckets = [0usize; 11];
    for (winding, _plane) in &cands {
        let wb = winding_bounds(winding);
        let max_ext = (wb.max.x - wb.min.x)
            .max((wb.max.y - wb.min.y).max(wb.max.z - wb.min.z));
        if max_ext > 8192.0 {
            continue;
        }
        let f = solid_perimeter_fraction(&tree, winding);
        buckets[(f * 10.0).round() as usize % 11] += 1;
    }
    buckets.iter().enumerate().map(|(i, c)| (i, *c)).collect()
}

/// Debug helper: build the BSP for `map` and dump all leaf portals lying on
/// the given (dist, axis) planes within the given (u, v) window, with their
/// areas and perimeter framing.
pub fn dbg_probe_planes(map: &mut Map, planes: &[(f32, usize)], u_range: (f32, f32), v_range: (f32, f32)) {
    let mut pool = PlanePool::default();
    let mut structural: Vec<BspBrush> = Vec::new();
    if let Some(world) = map.entities.first() {
        for brush in &world.brushes {
            if matches!(brush.content, BrushContent::Patch(_)) {
                continue;
            }
            let detail = match &brush.content {
                BrushContent::Convex(faces) => faces.iter().any(|f| is_detail_texture(&f.texture)),
                BrushContent::Patch(_) => true,
            };
            if detail {
                continue;
            }
            if let Some(b) = bsp_brush_from_map_brush(brush, 0, &mut pool) {
                structural.push(b);
            }
        }
    }
    let tree = PortalTree::build(pool, structural, SplitterSelection::default(), BspPortalParams::default().max_depth);
    eprintln!("bsp: {} leaves, {} portals", tree.nodes.iter().filter(|n| n.leaf).count(), tree.portals.len());
    for p in &tree.portals {
        let [a, b] = p.nodes;
        if a == usize::MAX || b == usize::MAX {
            continue;
        }
        if !tree.nodes[a].leaf || !tree.nodes[b].leaf {
            continue;
        }
        if tree.node_is_outside(a) || tree.node_is_outside(b) {
            continue;
        }
        if leaf_kind(&tree, a) != LeafKind::Open || leaf_kind(&tree, b) != LeafKind::Open {
            continue;
        }
        let axis = p
            .plane
            .n
            .x
            .abs()
            .max(p.plane.n.y.abs())
            .max(p.plane.n.z.abs());
        let axis = if p.plane.n.x == axis { 0 } else if p.plane.n.y == axis { 1 } else { 2 };
        for (dist, paxis) in planes {
            if *paxis != axis || (p.plane.d - dist).abs() > 0.5 {
                continue;
            }
            let w = &p.winding;
            let wb = winding_bounds(w);
            let (u, v) = match axis {
                1 => (0usize, 2usize),
                0 => (1usize, 2usize),
                _ => (0usize, 1usize),
            };
            if wb.max[u] < u_range.0 || wb.min[u] > u_range.1 {
                continue;
            }
            if wb.max[v] < v_range.0 || wb.min[v] > v_range.1 {
                continue;
            }
            let framed = solid_perimeter_fraction(&tree, w);
            eprintln!(
                "plane {} d={:.0}: pts={} bounds u[{:.0},{:.0}] v[{:.0},{:.0}] area={:.0} framed={:.2}",
                axis, p.plane.d, w.len(), wb.min[u], wb.max[u], wb.min[v], wb.max[v],
                winding_area(w), framed
            );
        }
    }
}

/// Debug: walk the BSP from the headnode to the leaf containing `pt` and
/// print the ancestor plane chain plus the leaf's portal list.
pub fn dbg_trace_point(map: &mut Map, pt: Vec3) {
    let mut pool = PlanePool::default();
    let mut structural: Vec<BspBrush> = Vec::new();
    if let Some(world) = map.entities.first() {
        for brush in &world.brushes {
            if matches!(brush.content, BrushContent::Patch(_)) {
                continue;
            }
            let detail = match &brush.content {
                BrushContent::Convex(faces) => faces.iter().any(|f| is_detail_texture(&f.texture)),
                BrushContent::Patch(_) => true,
            };
            if detail {
                continue;
            }
            if let Some(b) = bsp_brush_from_map_brush(brush, 0, &mut pool) {
                structural.push(b);
            }
        }
    }
    let tree = PortalTree::build(pool, structural, SplitterSelection::default(), BspPortalParams::default().max_depth);
    eprintln!(
        "trace point ({:.0},{:.0},{:.0}): leaves {} portals {}",
        pt.x, pt.y, pt.z,
        tree.nodes.iter().filter(|n| n.leaf).count(),
        tree.portals.len()
    );
    let mut i = 0usize;
    let mut depth = 0;
    loop {
        let node = &tree.nodes[i];
        if node.leaf {
            eprintln!(
                "leaf {}: brushes={} portals={}",
                i,
                node.brushes.len(),
                node.portals.len()
            );
            for &pi in &node.portals {
                let p = &tree.portals[pi];
                if p.nodes[0] == usize::MAX {
                    continue;
                }
                eprintln!(
                    "  portal {}: n=({},{},{}) d={:.0} area={:.0} otherleaf={} otherbrushes={}",
                    pi,
                    p.plane.n.x, p.plane.n.y, p.plane.n.z, p.plane.d,
                    winding_area(&p.winding),
                    if p.nodes[0] == i { p.nodes[1] } else { p.nodes[0] },
                    tree.nodes[if p.nodes[0] == i { p.nodes[1] } else { p.nodes[0] }].brushes.len()
                );
            }
            break;
        }
        let plane = &tree.pool.planes[node.planenum as usize];
        let (dot, _) = classify(pt, plane.n, plane.d, 0.0);
        let children = node.children.expect("children");
        let side = if dot >= 0.0 { 0 } else { 1 };
        eprintln!(
            "node {}: depth {} n=({},{},{}) d={:.0} dot={:.0} portals={} -> child {}",
            i, depth, plane.n.x, plane.n.y, plane.n.z, plane.d, dot,
            node.portals.len(), children[side]
        );
        i = children[side];
        depth += 1;
        if depth > 200 {
            eprintln!("depth runaway");
            break;
        }
    }
}

