//! In‑memory data model mirroring an idTech‑style `.map` file.
//!
//! - Brushes are convex polyhedra defined by three‑point planes.
//! - Entities are key/value dictionaries plus zero or more brushes.
//! - Faces carry texture and classic "9‑number" surface parameters as written by level editors.

use crate::core_util::vec3_to_origin;
use crate::editing::{Aabb, aabb_from_polys, aabb_from_positions};
use crate::editor::SurfInspector;
use crate::editor::viewport::Ortho;
use crate::texmap::{
    face_plane_normal, q3_texture_axes_from_normal, rotate_texture_axes, translation_offset_shift,
};
use crate::xmodel::XModel;
use crate::{IVec2, Vec2, Vec3, core_util};
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

/// Per-brush cached GPU data (vertices ready for upload).
///
/// Stored alongside the brush so that unchanged brushes skip UV mapping,
/// normal computation, and batch assembly during the rebuild loop.
#[derive(Debug, Clone)]
pub struct CachedBrushGpu {
    /// Wireframe line segment vertices.
    pub line_verts: Vec<Vec3>,
    /// Textured triangle vertices keyed by material name.
    pub tex_batches: HashMap<String, Vec<crate::render::TexVertex>>,
    /// Lit (flat-shaded) triangle vertices.
    pub lit_verts: Vec<crate::render::LitVertex>,
    /// Texture sizes (w, h) each material was UV-baked with. Used to detect
    /// stale UVs when a texture finishes loading asynchronously.
    pub tex_sizes: HashMap<String, [f32; 2]>,
}

/// Per-brush cached 2D projected wireframe lines for the ortho viewport.
///
/// Stores line segment endpoints projected to 2D for a specific axis, before
/// view-frustum culling. Regenerated when geometry changes or the view axis
/// changes; zoom/pan rebuilds reuse the cache and only re-cull.
#[derive(Debug, Clone)]
pub struct CachedBrushLines2d {
    /// Ortho axis these endpoints were projected for.
    pub axis: Ortho,
    /// Line segment endpoints as projected 2D points (z=0), stored in
    /// consecutive pairs `(a, b)` forming each segment.
    pub verts: Vec<Vec3>,
}

#[derive(Debug, Clone)]
pub struct Brush {
    pub id: BrushId,
    pub content: BrushContent,
    pub aabb: Aabb,
    /// Cached per‑face polygon data computed on demand.
    cached_geometry: Option<Vec<(Vec<Vec3>, Vec<u32>)>>,
    /// Cached GPU-facing data from last rebuild. None if never rebuilt or if
    /// cached_geometry was invalidated.
    pub(crate) cached_gpu: Option<CachedBrushGpu>,
    /// Cached 2D projected wireframe lines for the ortho viewport. Cleared on
    /// geometry changes; axis mismatches are detected at use time.
    pub(crate) cached_lines_2d: Option<CachedBrushLines2d>,
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
    pub fn apply_params(&mut self, si: &SurfInspector) {
        self.texture = si.tex_in.to_owned();
        self.params.shift = IVec2::new(si.hshift_in, si.vshift_in);
        self.params.scale = Vec2::new(si.hstretch_in, si.vstretch_in);
        self.params.rotate = si.rotate_in;
    }

    pub fn fit_texture(&mut self, poly: &[Vec3], tex_w: f32, tex_h: f32, _sample_size: i32) {
        if poly.is_empty() {
            return;
        }

        let tex_w = tex_w.max(1.0);
        let tex_h = tex_h.max(1.0);

        let n = face_plane_normal(self);
        let (s_axis, t_axis) = q3_texture_axes_from_normal(n);
        let (s_axis, t_axis) =
            rotate_texture_axes(s_axis, t_axis, (self.params.rotate as f32).to_radians());

        let mut s_min = f32::INFINITY;
        let mut s_max = f32::NEG_INFINITY;
        let mut t_min = f32::INFINITY;
        let mut t_max = f32::NEG_INFINITY;
        for &p in poly {
            // World coordinates, matching Q3Radiant Face_FitTexture (texture
            // alignment is relative to the world grid, not the face origin).
            let s = p.dot(s_axis);
            let t = p.dot(t_axis);
            s_min = s_min.min(s);
            s_max = s_max.max(s);
            t_min = t_min.min(t);
            t_max = t_max.max(t);
        }

        if !s_min.is_finite() || !t_min.is_finite() || !s_max.is_finite() || !t_max.is_finite() {
            return;
        }

        let s_extent = (s_max - s_min).max(1.0);
        let t_extent = (t_max - t_min).max(1.0);

        self.params.scale = Vec2::new(s_extent / tex_w, t_extent / tex_h);
        // Shift in texels, wrapped into [0, tex_w)x[0, tex_h) like Q3Radiant's
        // Face_FitTexture (texture repeats, so only the remainder matters).
        let wrap = |shift: f32, tex_size: f32| -> i32 { shift.rem_euclid(tex_size) as i32 };
        self.params.shift = IVec2::new(
            wrap(-s_min / self.params.scale.x, tex_w),
            wrap(-t_min / self.params.scale.y, tex_h),
        );
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
            cached_gpu: None,
            cached_lines_2d: None,
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

    /// Clone for temporary preview/probe edits. Omits GPU and 2D line caches
    /// (they are invalid after preview geometry edits and only add clone cost).
    pub fn clone_for_preview(&self) -> Self {
        Self {
            id: self.id,
            content: self.content.clone(),
            aabb: self.aabb,
            cached_geometry: None,
            cached_gpu: None,
            cached_lines_2d: None,
        }
    }

    /// Invalidate cached geometry so the next `get_polygons` call recomputes.
    pub fn invalidate_geometry(&mut self) {
        self.cached_geometry = None;
        self.cached_gpu = None;
        self.cached_lines_2d = None;
    }

    /// Invalidate only the cached GPU data (UVs, normals, batch data) without
    /// re-tessellating. Use when texture params change but geometry is unchanged.
    pub fn invalidate_gpu(&mut self) {
        self.cached_gpu = None;
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
                self.cached_geometry = None;
                self.cached_gpu = None;
                self.cached_lines_2d = None;
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
                for face in faces.iter_mut() {
                    for p in &mut face.plane_points {
                        *p += dv;
                    }
                    // Texture Lock: adjust shift so the texture stays visually
                    // glued when the brush moves (matching CoDRadiant/GTKRadiant).
                    let shift_delta = translation_offset_shift(face, dv);
                    face.params.shift += shift_delta;
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
        self.cached_gpu = None;
        self.cached_lines_2d = None;
        *generation = generation.wrapping_add(1);
        self.aabb.min += delta;
        self.aabb.max += delta;
    }

    pub fn get_texture_last(&self) -> String {
        match &self.content {
            BrushContent::Convex(faces) => faces.last().unwrap().texture.clone(),
            BrushContent::Patch(patch) => patch.texture.clone(),
        }
    }

    pub fn get_face(&self, idx: usize) -> Option<Face> {
        if let BrushContent::Convex(faces) = &self.content {
            Some(faces[idx].clone())
        } else {
            None
        }
    }

    pub fn get_last_face(&self) -> Option<Face> {
        if let BrushContent::Convex(faces) = &self.content {
            faces.last().cloned()
        } else {
            None
        }
    }

    /*pub fn get_textures(&self) -> Vec<String>
    {
        match &self.content {
            BrushContent::Convex(faces) => {
                faces.iter().map(|f| f.texture.clone()).collect()
            }
            BrushContent::Patch(patch) => {
                vec![patch.texture.clone()]
            }
        }
    }*/

    pub fn set_texture(&mut self, texture: &str) {
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
        self.cached_gpu = None;
    }
    pub fn set_texture_params(&mut self, si: SurfInspector) {
        match &mut self.content {
            BrushContent::Convex(faces) => {
                for f in faces {
                    f.apply_params(&si);
                }
            }
            BrushContent::Patch(patch) => {
                patch.texture = si.tex_in;
                // TODO: other params
            }
        }
        self.cached_gpu = None;
    }

    pub fn is_clip(&self) -> bool {
        match &self.content {
            BrushContent::Convex(faces) => {
                for f in faces {
                    if f.texture.contains("clip") {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    pub fn is_portal(&self) -> bool {
        match &self.content {
            BrushContent::Convex(faces) => {
                for f in faces {
                    if f.texture.contains("portal") {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    pub fn is_hint(&self) -> bool {
        match &self.content {
            BrushContent::Convex(faces) => {
                for f in faces {
                    if f.texture.contains("common/hint") {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
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
    pub model: Option<XModel>,
}

impl Entity {
    pub fn aabb_for_selection(&self) -> Option<&Aabb> {
        if self.brushes.len() != 0 {
            println!("entity not selected because it contains brush(es)");
        }
        None
    }

    pub fn translate(&mut self, generation: &mut u64, delta: Vec3) {
        if self.classname == "worldspawn" {
            eprintln!("translate should not be called on worldspawn, check all call sites");
            return; // just in case
        }
        let mut origin = self
            .properties
            .get("origin")
            .map(|s| core_util::origin_to_vec3(s))
            .unwrap_or(Vec3::ZERO);

        origin += delta;

        let origin_str = vec3_to_origin(origin);
        println!("origin_str: {origin_str:?}");
        self.properties.insert("origin".to_string(), origin_str);
        *generation = generation.wrapping_add(1);
    }
}

/// Top‑level map structure, mirroring a complete `.map` file.
#[derive(Debug, Default, Clone)]
pub struct Map {
    pub entities: Vec<Entity>,
    /// Incremented every time ANY brush or entity changes.
    /// Used for dirty-flag caching.
    pub generation: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editing::default_texture_params;
    use crate::texmap::FaceUvMapper;

    #[test]
    fn fit_texture_maps_min_to_zero() {
        // Floor at z=0 (Z-dominant normal -> s=X, t=-Y).
        let face = Face {
            plane_points: [
                Vec3::new(-10.0, -10.0, 0.0),
                Vec3::new(10.0, -10.0, 0.0),
                Vec3::new(-10.0, 10.0, 0.0),
            ],
            texture: "test".into(),
            params: default_texture_params(),
        };

        let poly = [
            Vec3::new(-10.0, -5.0, 0.0),
            Vec3::new(10.0, -5.0, 0.0),
            Vec3::new(10.0, 5.0, 0.0),
            Vec3::new(-10.0, 5.0, 0.0),
        ];

        let mut face = face;
        face.fit_texture(&poly, 64.0, 64.0, 1);

        // World-relative fit (matching CoDRadiant): s_min=-10, extent=20,
        // scale.x=20/64, shift.x=wrap(-(-10)/(20/64),64)=32.
        assert!(
            (face.params.scale.x - 20.0 / 64.0).abs() < 1.0e-3,
            "expected scale.x ~= {}, got {}",
            20.0 / 64.0,
            face.params.scale.x
        );
        assert_eq!(face.params.shift.x, 32, "shift.x must match CoDRadiant fit");
        assert_eq!(face.params.shift.y, 32, "shift.y must match CoDRadiant fit");

        // World-relative UV mapper: u(x) = x/(64*scale.x) + shift.x/64
        let mapper = FaceUvMapper::new(&face, 64.0, 64.0);
        let u_min = mapper.uv(Vec3::new(-10.0, 0.0, 0.0)).x;
        let u_max = mapper.uv(Vec3::new(10.0, 0.0, 0.0)).x;
        assert!(u_min.abs() < 1.0e-3, "expected u(-10) ~= 0, got {u_min}");
        assert!(
            (u_max - 1.0).abs() < 1.0e-3,
            "expected u(10) ~= 1, got {u_max}"
        );
    }

    #[test]
    fn translate_adjusts_shift_to_glue_texture() {
        let mut brush = Brush {
            id: BrushId(0),
            content: BrushContent::Convex(vec![Face {
                plane_points: [
                    Vec3::new(-10.0, -10.0, 0.0),
                    Vec3::new(10.0, -10.0, 0.0),
                    Vec3::new(-10.0, 10.0, 0.0),
                ],
                texture: "test".into(),
                params: default_texture_params(),
            }]),
            aabb: Aabb::default(),
            cached_geometry: None,
            cached_gpu: None,
            cached_lines_2d: None,
        };

        let mut gen_id = 0u64;
        let before_shift = brush.get_face(0).unwrap().params.shift;
        // A point on the face before the move.
        let p = Vec3::new(0.0, 0.0, 0.0);
        let before = FaceUvMapper::new(&brush.get_face(0).unwrap(), 64.0, 64.0).uv(p);

        // Move the brush; texture lock adjusts shift so the UV at the moved
        // face location stays identical (texture stays glued to the face).
        let dv = Vec3::new(128.0, -64.0, 0.0);
        brush.translate(&mut gen_id, dv);
        let after = FaceUvMapper::new(&brush.get_face(0).unwrap(), 64.0, 64.0).uv(p + dv);
        let after_shift = brush.get_face(0).unwrap().params.shift;

        assert_ne!(
            before_shift, after_shift,
            "texture lock must modify shift on translate"
        );
        assert!(
            (before.x - after.x).abs() < 1.0e-3 && (before.y - after.y).abs() < 1.0e-3,
            "expected UVs glued across translation, got {before:?} -> {after:?}"
        );
    }
}
