//! Data model mirroring CoD1 .map format
//! Brushes = convex polyhedra defined by 3-point planes
//! Entities = worldspawn + keys (CoD1-specific: targetname, origin, etc.)
//! Face format: 3-point plane + texture + 9 numbers (CoD Radiant style)

use crate::{IVec2, Vec2, Vec3};
use std::collections::HashMap;

/// Strict newtype IDs (prevents accidental mixing of entity/brush indices)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BrushId(pub u32);

/// Surface flags (stored per-face in .map but must be identical across all faces of a brush)
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
    /// Cached geometry
    cached_geometry: Option<Vec<(Vec<Vec3>, Vec<u32>)>>,
    /// Brush was modified
    dirty: bool,
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
    pub shader: String,
    pub params: PatchParams,
    /// Vertex grid stored as `[row][col]`.
    pub vertices: Vec<Vec<PatchVertex>>,
    cached_mesh: Option<PatchMesh>,
    /// Patch was modified
    dirty: bool,
}

impl Brush {
    /// Create a new brush (used by parser and editor)
    pub fn new(id: BrushId, content: BrushContent) -> Self {
        Self {
            id,
            content,
            cached_geometry: None,
            dirty: false,
        }
    }

    /// Returns cached polygons for a brush (recomputes only if the brush changed).
    /// Call this every frame from the UI — it's O(1) when nothing changed.
    pub fn get_polygons(&mut self) -> Option<&[(Vec<Vec3>, Vec<u32>)]> {
        if self.dirty || self.cached_geometry.is_none() {
            let polys = crate::geometry::brush_to_polygons(self).ok()?;
            self.cached_geometry = Some(polys);
            self.dirty = false;
        }

        self.cached_geometry.as_deref()
    }

    pub fn update_brush_plane(&mut self, generation: &mut u64, plane_index: usize, new_plane: [Vec3; 3]) {
        if let BrushContent::Convex(faces) = &mut self.content {
            if let Some(face) = faces.get_mut(plane_index) {
                face.plane_points = new_plane;
                *generation = generation.wrapping_add(1);
                self.cached_geometry = None; // force next get_ to recompute
                return;
            }
        }
    }
}

impl Patch {
    pub fn new(patch_type: PatchType, shader: String, params: PatchParams, vertices: Vec<Vec<PatchVertex>>) -> Self {
        Self { patch_type, shader, params, vertices, cached_mesh: None, dirty: false }
    }

    /// Returns cached tessellation, recomputing only when map.generation changed.
    pub fn get_mesh(&mut self) -> Option<&PatchMesh> {
        if self.dirty || self.cached_mesh.is_none() {
            let mesh = crate::geometry::tessellate_patch(self).ok()?;
            self.cached_mesh = Some(mesh);
            self.dirty = false;
        }
        self.cached_mesh.as_ref()
    }

    pub fn update_vertex(&mut self, generation: &mut u64, row: usize, col: usize, vtx: PatchVertex) {
        if let Some(r) = self.vertices.get_mut(row) {
            if let Some(v) = r.get_mut(col) {
                *v = vtx;
                *generation = generation.wrapping_add(1);
                self.cached_mesh = None;
            }
        }
    }
}

/// Entity (worldspawn or any other - supports CoD1 keys)
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: EntityId,
    /// The must have key for every entity
    pub classname: String,
    /// "origin", "targetname", etc.
    pub properties: HashMap<String, String>,
    pub brushes: Vec<Brush>,
}

/// Top-level map (mirrors entire .map file)
#[derive(Debug, Default, Clone)]
pub struct Map {
    pub entities: Vec<Entity>,
    /// Incremented every time ANY brush or entity changes.
    /// Used for dirty-flag caching.
    pub generation: u64,
}
