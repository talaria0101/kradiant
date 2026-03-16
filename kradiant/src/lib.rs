//! radiant_core - CoD1 .map backend (idTech 3 style)
//! High-priority: .map parser (text format mirroring Q3/CoD1)
//! Low-priority: C FFI (cbindgen-ready stubs added later)

pub mod assets;
pub mod geometry;
pub mod loader;
pub mod map;
pub mod map_utils;
pub mod parser;
pub mod shader;
pub mod texmap;
pub mod texture;

/*
pub use geometry::{
    brush_to_mesh, brush_to_polygons, tessellate_patch, update_brush_plane, GeometryError, Mesh,
    PatchMesh,
};*/
/*pub use assets::{
    normalize_asset_path, resolve_editor_image_name, AssetDb, AssetDbError, AssetRoots,
    ResolvedAsset,
};
pub use map::{
    Brush, BrushId, Entity, EntityId, Face, Map, Patch, PatchParams, PatchType, PatchVertex,
};
pub use parser::{load_map, save_map, ParseError};
pub use shader::{
    load_shader_db_from_main_dir, load_shader_db_from_scripts_dir, QerParams, ShaderDb, ShaderDef,
    ShaderError, parse_shader_source_into_db,
};
pub use texture::{decode_texture_rgba8, load_texture_rgba8, TextureError, TextureImage};
pub use texmap::{
    face_plane_normal, face_uv, q3_texture_axes_from_normal, rotate_texture_axes, FaceUvMapper,
};
pub use map_utils::{collect_used_materials, count_brushes};*/

// Re-export common types for UI layer
pub use glam::{IVec2, Mat4, Quat, Vec2, Vec3, Vec4};

// TODO: get_polygons_for_brush(id) -> &[Polygon] (cached vertex/index buffers)
// TODO: FFI layer (extern "C" + cbindgen) - low priority
