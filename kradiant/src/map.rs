//! In‑memory data model mirroring an idTech‑style `.map` file.
//!
//! - Brushes are convex polyhedra defined by three‑point planes.
//! - Entities are key/value dictionaries plus zero or more brushes.
//! - Faces carry texture and classic "9‑number" surface parameters as written by level editors.

use crate::editing::{Aabb, aabb_from_polys, aabb_from_positions};
use crate::map_utils::rotate_vector;
use crate::{IVec2, Vec2, Vec3};
use std::collections::{HashMap, HashSet};

/// Strongly‑typed entity identifiers (prevents mixing entity and brush indices).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BrushId(pub u32);

/// Surface flags stored per face in a `.map` file.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SurfaceFlags {
    #[default]
    Structural = 0, // 0x00000000

    Detail = 1 << 27,               // 0x8000000  = 134217728
    WeaponClip = (1 << 27) | 0x200, // 0x8000200 = 134226048
    NonColliding = (1 << 27) | 0x4, // 0x8000004 = 134217732

    // Unknown
    Unknown(u32),
}

impl SurfaceFlags {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Structural,
            134217728 => Self::Detail,
            134226048 => Self::WeaponClip,
            134217732 => Self::NonColliding,
            _ => Self::Unknown(v),
        }
    }

    pub fn as_u32(self) -> u32 {
        match self {
            Self::Structural => 0,
            Self::Detail => 1 << 27,
            Self::WeaponClip => (1 << 27) | 0x200,
            Self::NonColliding => (1 << 27) | 0x4,
            Self::Unknown(v) => v,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TextureParams {
    pub shift: IVec2, // Horizontal / Vertical
    pub rotate: i32,  // degrees
    pub scale: Vec2,  // Horizontal / Vertical stretch
    pub surface_flags: SurfaceFlags,
    pub idk: f32,
    pub value: i32,
    pub sample_size: i32,
}

#[derive(Debug, Clone)]
pub struct Face {
    pub plane_points: [Vec3; 3],
    pub texture: String,
    pub params: TextureParams, // CoD1 Radiant 9-number format
}

#[derive(Debug, Clone)]
pub enum BrushContent {
    Convex(Vec<Face>),
    Patch(Patch),
}

#[derive(Debug, Clone)]
pub struct Brush {
    pub id: BrushId,
    pub content: BrushContent,
    pub aabb: Aabb,
    /// Cached per‑face polygon data computed on demand.
    cached_geometry: Option<Vec<(Vec<Vec3>, Vec<u32>)>>,
    // Indicates whether the brush has been modified since geometry was cached.
    //dirty: bool,
    //generation: u64,
    //last_generation: u64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchType {
    /// `patchTerrainDef3` (terrain mesh)
    Terrain,
    /// `patchDef5` (bezier curve/surface)
    Curve,
}

#[derive(Debug, Clone, Copy)]
pub struct PatchParams {
    /// Number of vertex rows.
    pub rows: u32,
    /// Number of vertex columns.
    pub cols: u32,
    /// Content flags (collision/detail/etc).
    pub contents: i32,
    pub reserved: [i32; 3],
    /// Tessellation level (commonly 8).
    pub subdivision: u32,
}

impl Default for PatchParams {
    fn default() -> Self {
        Self {
            rows: 3,
            cols: 3,
            contents: 0,
            reserved: [0; 3],
            subdivision: 8,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PatchVertex {
    pub position: Vec3,
    pub uv: Vec2,
    /// Vertex color as RGBA8.
    pub color: [u8; 4],
    /// Edge direction flag (0 or 1) used by terrain triangulation.
    pub turned_edge: bool,
}

use crate::geometry::PatchMesh;
#[derive(Debug, Clone)]
pub struct Patch {
    pub patch_type: PatchType,
    pub texture: String,
    pub params: PatchParams,
    /// Vertex grid stored as `[row][col]` in map space.
    pub vertices: Vec<Vec<PatchVertex>>,
    cached_mesh: Option<PatchMesh>,
    cached_aabb: Option<Aabb>,
    cached_wire_edges: Option<Vec<(u32, u32)>>,
    // Indicates whether the patch has been modified since tessellation was cached.
    //dirty: bool,
    //generation: u64,
    //last_generation: u64
}

impl Face {
    pub fn calculate_uv(&self, pos: Vec3) -> [f32; 2] {
        let p0 = self.plane_points[0];
        let p1 = self.plane_points[1];
        let p2 = self.plane_points[2];
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let normal = e1.cross(e2).normalize();

        let (mut u_axis, mut v_axis) = Self::best_fit_axes(normal);

        println!("texture params: {:#?}", self.params);

        if self.params.rotate != 0 {
            let angle = self.params.rotate as f32;
            u_axis = rotate_vector(u_axis, normal, angle);
            v_axis = rotate_vector(v_axis, normal, angle);
        }

        let delta = pos - p0;
        let u = delta.dot(u_axis);
        let v = delta.dot(v_axis);

        let u = u / self.params.scale.x;
        let v = v / self.params.scale.y;

        let u = u + self.params.shift.x as f32;
        let v = v + self.params.shift.y as f32;

        [u, v]
    }

    fn best_fit_axes(normal: Vec3) -> (Vec3, Vec3) {
        let abs_n = normal.abs();

        if abs_n.z > abs_n.x && abs_n.z > abs_n.y {
            (Vec3::X, Vec3::Y)
        } else if abs_n.x > abs_n.y {
            (Vec3::Y, Vec3::Z)
        } else {
            (Vec3::X, Vec3::Z)
        }
    }
}

impl Brush {
    /// Create a new brush with the given identifier and content.
    pub fn new(id: BrushId, content: BrushContent) -> Self {
        Self {
            id,
            content,
            aabb: Aabb::default(),
            cached_geometry: None,
            //dirty: false,
            //generation: 0,
            //last_generation: 1,
        }
    }

    /// Return cached polygons for a brush, recomputing only if the brush has changed.
    ///
    /// Callers are expected to reuse the returned slice between frames; when nothing changed this
    /// is effectively O(1).
    pub fn get_polygons_and_aabb(&mut self) -> Option<(&Aabb, &[(Vec<Vec3>, Vec<u32>)])> {
        /*if self.dirty || self.cached_geometry.is_none() {
            let polys = crate::geometry::brush_to_polygons(self).ok()?;
            self.cached_geometry = Some(polys);
            self.dirty = false;
        }*/
        if
        /*self.generation != self.last_generation ||*/
        self.cached_geometry.is_none() {
            let polys = crate::geometry::brush_to_polygons(self).ok()?;
            self.aabb = aabb_from_polys(&polys);
            self.cached_geometry = Some(polys);
            //self.generation = self.generation.wrapping_add(1);
            //self.last_generation = self.generation;
        }

        let aabb = &self.aabb;
        let polys = self.cached_geometry.as_deref()?;
        Some((aabb, polys))
    }

    pub fn get_polygons(&mut self) -> Option<&[(Vec<Vec3>, Vec<u32>)]> {
        self.get_polygons_and_aabb().map(|(_, polys)| polys)
    }

    /// Update a single brush plane and bump the map generation counter if it changed.
    pub fn update_brush_plane(
        &mut self,
        generation: &mut u64,
        plane_index: usize,
        new_plane: [Vec3; 3],
    ) {
        if let BrushContent::Convex(faces) = &mut self.content {
            if let Some(face) = faces.get_mut(plane_index) {
                face.plane_points = new_plane;
                *generation = generation.wrapping_add(1);
                self.cached_geometry = None; // force next get_ to recompute
                return;
            }
        }
    }

    pub fn polygons_for_drawing(&self) -> Option<Vec<(Vec<Vec3>, Vec<u32>)>> {
        match &self.content {
            BrushContent::Convex(_) => crate::geometry::brush_to_polygons(self).ok(),
            BrushContent::Patch(_) => None, // skip patches here
        }
    }

    pub fn translate(&mut self, generation: &mut u64, delta: Vec3) {
        let dv = delta;
        match &mut self.content {
            BrushContent::Convex(faces) => {
                for face in faces {
                    for p in &mut face.plane_points {
                        *p += dv;
                    }
                }
                self.cached_geometry = None;
            }
            BrushContent::Patch(patch) => {
                for row in &mut patch.vertices {
                    for v in row {
                        v.position += dv;
                    }
                }
                patch.cached_mesh = None;
            }
        }
        *generation = generation.wrapping_add(1);
        self.aabb.min += delta;
        self.aabb.max += delta;
    }

    pub fn apply_texture(&mut self, texture: &str) {
        match &mut self.content {
            BrushContent::Convex(faces) => {
                for face in faces.iter_mut() {
                    face.texture = texture.to_owned();
                }
            }
            BrushContent::Patch(patch) => {
                patch.texture = texture.to_owned();
            }
        }
    }

    /*pub fn recompute_aabb(&mut self)
    {
        if let Some(polys) = self.get_polygons() {
            self.aabb = aabb_from_polys(polys);
        }
    }*/
}

impl Patch {
    fn ensure_cached(&mut self) -> Option<()> {
        if
        /*self.generation != self.last_generation ||*/
        self.cached_mesh.is_none() {
            let mesh = crate::geometry::tessellate_patch(self).ok()?;
            self.cached_aabb = Some(aabb_from_positions(mesh.positions.as_slice()));

            let mut seen: HashSet<(u32, u32)> = HashSet::new();
            for tri in mesh.indices.chunks_exact(3) {
                let edges = [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])];
                for (a, b) in edges {
                    let key = if a < b { (a, b) } else { (b, a) };
                    seen.insert(key);
                }
            }
            let mut edges: Vec<(u32, u32)> = seen.into_iter().collect();
            edges.sort_unstable();
            self.cached_wire_edges = Some(edges);

            self.cached_mesh = Some(mesh);
            //self.generation = self.generation.wrapping_add(1);
            //self.last_generation = self.generation;
        }
        Some(())
    }

    /// Create a new patch with the given shader, parameters and vertex grid.
    pub fn new(
        patch_type: PatchType,
        texture: String,
        params: PatchParams,
        vertices: Vec<Vec<PatchVertex>>,
    ) -> Self {
        Self {
            patch_type,
            texture,
            params,
            vertices,
            cached_mesh: None,
            cached_aabb: None,
            cached_wire_edges: None,
            //generation: 0,
            //last_generation: 1,
        }
    }

    /// Return cached tessellation, recomputing only when the patch was modified.
    pub fn get_mesh(&mut self) -> Option<&PatchMesh> {
        let _ = self.ensure_cached()?;
        self.cached_mesh.as_ref()
    }

    pub fn get_mesh_aabb_wire(&mut self) -> Option<(&PatchMesh, &Aabb, &[(u32, u32)])> {
        let _ = self.ensure_cached()?;
        let mesh = self.cached_mesh.as_ref()?;
        let aabb = self.cached_aabb.as_ref()?;
        let edges = self.cached_wire_edges.as_deref()?;
        Some((mesh, aabb, edges))
    }

    /// Update a single vertex in the patch grid and bump the map generation counter if it changed.
    pub fn update_vertex(
        &mut self,
        generation: &mut u64,
        row: usize,
        col: usize,
        vtx: PatchVertex,
    ) {
        if let Some(r) = self.vertices.get_mut(row) {
            if let Some(v) = r.get_mut(col) {
                *v = vtx;
                *generation = generation.wrapping_add(1);
                self.cached_mesh = None;
                self.cached_aabb = None;
                self.cached_wire_edges = None;
            }
        }
    }
}

/// Entity (worldspawn or any other brush collection with key/value properties).
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: EntityId,
    /// Mandatory classification key as written in the source `.map`.
    pub classname: String,
    /// Arbitrary key/value pairs such as `"origin"`, `"targetname"`, etc.
    pub properties: HashMap<String, String>,
    pub brushes: Vec<Brush>,
}

/// Top‑level map structure, mirroring a complete `.map` file.
#[derive(Debug, Default, Clone)]
pub struct Map {
    pub entities: Vec<Entity>,
    /// Incremented every time ANY brush or entity changes.
    /// Used for dirty-flag caching.
    pub generation: u64,
}
