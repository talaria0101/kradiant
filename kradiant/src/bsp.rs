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
}

impl Default for BspPortalParams {
    fn default() -> Self {
        Self {
            min_edge: crate::portals::MIN_OPENING_EXTENT,
            min_area: crate::portals::MIN_OPENING_EXTENT * crate::portals::MIN_OPENING_EXTENT,
            thickness: 8.0,
            area_separators: true,
            max_extent: 8192.0,
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
        let mid = interpolate(w[i], w[j], dots[i], dots[j], n, d);
        if sides[i] == SIDE_FRONT {
            back.push(mid);
        } else {
            front.push(mid);
        }
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

#[derive(Default)]
struct PlanePool {
    planes: Vec<Plane>,
}

impl PlanePool {
    /// Canonical, positive-facing plane index (normalised like q3map).
    fn find(&mut self, n: Vec3, d: f32) -> usize {
        let mut n = n;
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
    leaf: bool,
    /// index into `Vec<NodeData>`, or usize::MAX for the outside node.
    portals: Vec<usize>,
}

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
    fn build(pool: PlanePool, brushes: Vec<BspBrush>) -> PortalTree {
        let mut bounds: Option<Aabb> = None;
        for b in &brushes {
            bounds = Some(match bounds {
                None => b.bounds,
                Some(acc) => aabb_union(acc, b.bounds),
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
        };

        // Headnode + outside node with 6 border portals (decomp MakeOutsideNode).
        let head = tree.push_node(TreeNode {
            planenum: PLANENUM_LEAF,
            children: None,
            brushes,
            leaf: false,
            portals: Vec::new(),
        });
        let outside = tree.push_node(TreeNode {
            planenum: PLANENUM_LEAF,
            children: None,
            brushes: Vec::new(),
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

    /// Pick the plane that splits fewest brushes while balancing them
    /// (qbsp3 SelectSplitPlane scoring, simplified). `parent_planes` are
    /// the ancestor split planes (q3 CheckPlaneAgainstParents: never split
    /// on the same plane twice on one path).
    fn select_split_plane(
        &self,
        node: &TreeNode,
        parent_planes: &[usize],
    ) -> Option<usize> {
        const MAX_CANDIDATES: usize = 256;
        let brushes = &node.brushes;
        let mut best: Option<(usize, i64)> = None;

        // Candidate planes: canonical side planes of the node's brushes.
        let mut candidates: Vec<usize> = Vec::new();
        for b in brushes {
            for (idx, _) in &b.sides {
                if !candidates.contains(idx) {
                    candidates.push(*idx);
                }
            }
        }
        // Deterministic order, cap the work.
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
                // Quick AABB reject.
                let side = box_on_plane_side(&b.bounds, &plane);
                const FRONT: u8 = 1;
                const BACK: u8 = 2;
                const BOTH: u8 = 3;
                match side {
                    FRONT => {
                        front += 1;
                        continue;
                    }
                    BACK => {
                        back += 1;
                        continue;
                    }
                    BOTH => {}
                    _ => {
                        facing += 1;
                        continue;
                    }
                }
                // Exact test: any winding point crossing?
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
                // db = max(-dot) >= 0 here; a brush is entirely front when
                // nothing sits behind (db ~ 0), entirely back when nothing
                // sits in front (df ~ 0).
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
            let axial = plane.n.x.abs() > 0.999 || plane.n.y.abs() > 0.999 || plane.n.z.abs() > 0.999;
            let mut value: i64 = 5 * facing - 5 * splits - (front - back).abs();
            if axial {
                value += 5;
            }
            // Strongly prefer planes that actually separate brushes; a
            // zero-split plane is only chosen when nothing better exists
            // (routing/balance, like q3).
            if splits == 0 {
                value -= 1000;
            }
            if best.map_or(true, |(_, bv)| value > bv) {
                best = Some((cand, value));
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

        if depth >= 128 {
            self.nodes[node_idx].leaf = true;
            return;
        }

        let split = {
            let node = &self.nodes[node_idx];
            let planes: Vec<usize> = parent_planes.iter().map(|(p, _)| *p).collect();
            self.select_split_plane(node, &planes)
        };
        let Some(split) = split else {
            self.nodes[node_idx].leaf = true;
            return;
        };

        // Partition the brushes.
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

        let planenum = split as i32;
        self.nodes[node_idx].planenum = planenum;
        let front_idx = self.push_node(TreeNode {
            planenum: PLANENUM_LEAF,
            children: None,
            brushes: front_list,
            leaf: false,
            portals: Vec::new(),
        });
        let back_idx = self.push_node(TreeNode {
            planenum: PLANENUM_LEAF,
            children: None,
            brushes: back_list,
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

fn box_on_plane_side(b: &Aabb, plane: &Plane) -> u8 {
    const FRONT: u8 = 1;
    const BACK: u8 = 2;
    // Fast axial reject.
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
    // Non-axial plane: classify both corners (conservative).
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

/// Fraction of the winding's edges whose immediate outside is solid world
/// (a jamb, sill, lintel or wall face). A mapper-portal-worthy opening is
/// framed by solid geometry; void-shell cross-sections are not.
fn solid_perimeter_fraction(tree: &PortalTree, winding: &Winding) -> f32 {
    let Some(plane) = winding_plane(winding) else {
        return 0.0;
    };
    let n = winding.len();
    let mut solid = 0usize;
    let mut total = 0usize;
    for i in 0..n {
        let a = winding[i];
        let b = winding[(i + 1) % n];
        let edge = b - a;
        let len = edge.length();
        if len < 0.5 {
            continue;
        }
        total += 1;
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
            solid += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        solid as f32 / total as f32
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
        if (n - plane.n * -want).length() < 0.1 && n.dot(plane.n * -want) > 0.95 {
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
    let tree = PortalTree::build(pool, structural);
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
        let brush =
            portal_brush_from_winding(BrushId(next_id), &winding, plane, params.thickness, textures, side);
        next_id += 1;
        let Some(brush) = brush else { continue };
        let id = brush.id;
        let aabb = crate::portals::aabb_of_brush_pub(&brush);
        if let Some(world) = map.entities.first_mut() {
            world.brushes.push(brush);
        }
        if let Some(aabb) = aabb {
            placed.push(crate::portals::PlacedPortal {
                entity: 0,
                id,
                aabb,
            });
        }
    }

    // Over-cover like the originals: union coplanar near-touching portals.
    let merged = crate::portals::merge_placed_portals_public(map, &mut placed, crate::portals::PORTAL_MERGE_GAP, side, textures);
    report.portals_merged = merged;
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

        let tree = PortalTree::build(pool, structural);
        let candidates = collect_candidates(&tree);
        // Framed openings on the doorway plane(s).
        let doorway = candidates
            .iter()
            .filter(|(_, plane)| plane.n.x.abs() > 0.9 && (plane.d - 64.0).abs() < 0.5 || plane.n.x.abs() > 0.9 && (plane.d - 72.0).abs() < 0.5)
            .filter(|(w, _)| {
                let e = winding_bounds(w);
                (e.max.x - e.min.x) < 12.0 && winding_area(w) < 20000.0
            })
            .max_by_key(|(w, _)| winding_area(w) as u64)
            .expect("a framed portal on the doorway plane");
        let w = &doorway.0;
        let area = winding_area(w);
        assert!(area > 48.0 * 64.0 * 0.5, "doorway-sized winding, got {area}");
        // The winding lies within the doorway bounds (y,z), x == 64..72.
        let b = winding_bounds(w);
        eprintln!("doorway winding bounds: min=({}, {}, {}) max=({}, {}, {}) area={:.0}",
            b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z, area);
        assert!(b.min.x >= 63.5 && b.max.x <= 72.5);
        assert!(b.min.y >= 47.0 && b.max.y <= 113.0);
        assert!(b.min.z >= -1.0 && b.max.z <= 89.0);
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
    let tree = PortalTree::build(pool, structural);
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
    let tree = PortalTree::build(pool, structural);
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
    let tree = PortalTree::build(pool, structural);
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
