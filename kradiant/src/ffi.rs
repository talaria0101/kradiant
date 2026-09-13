use crate::KRADIANT_VERSION;
use crate::editor::EditorState;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

/// Create a new EditorState.
/// Must be freed with kr_editor_free.
#[unsafe(no_mangle)]
pub extern "C" fn kr_editor_new() -> *mut EditorState {
    Box::into_raw(Box::new(EditorState::default()))
}

/// Free an EditorState.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_free(ptr: *mut EditorState) {
    if !ptr.is_null() {
        unsafe {
            drop(Box::from_raw(ptr));
        }
    }
}

/// Load a map into the editor.
/// Returns true on success, false on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_load_map(ptr: *mut EditorState, path: *const c_char) -> bool {
    if ptr.is_null() || path.is_null() {
        return false;
    }
    let state = unsafe { &mut *ptr };
    let c_str = unsafe { CStr::from_ptr(path) };
    let path_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };

    match crate::parser::load_map(path_str) {
        Ok(map) => {
            state.map = Some(map);
            state.map_path = path_str.to_string();
            state.bump_revision();
            state.dirty = false;
            true
        }
        Err(_) => false,
    }
}

/// Save the current map.
/// Returns true on success, false on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_save_map(ptr: *mut EditorState, path: *const c_char) -> bool {
    if ptr.is_null() || path.is_null() {
        return false;
    }
    let state = unsafe { &*ptr };
    let c_str = unsafe { CStr::from_ptr(path) };
    let path_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };

    if let Some(map) = &state.map {
        match crate::parser::save_map(map, path_str) {
            Ok(_) => true,
            Err(_) => false,
        }
    } else {
        false
    }
}

/// Get the version of the library.
/// The returned string is statically allocated and should NOT be freed.
#[unsafe(no_mangle)]
pub extern "C" fn kr_get_version() -> *const c_char {
    static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| CString::new(KRADIANT_VERSION).unwrap())
        .as_ptr()
}

/// Clear the current selection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_clear_selection(ptr: *mut EditorState) {
    if let Some(state) = unsafe { ptr.as_mut() } {
        state.clear_selection();
    }
}

/// Toggle face editing mode.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_set_edit_faces(ptr: *mut EditorState, enabled: bool) {
    if let Some(state) = unsafe { ptr.as_mut() } {
        state.edit_faces = enabled;
        if enabled {
            state.edit_edges = false;
            state.edit_vertices = false;
        }
    }
}

/// Toggle vertex editing mode.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_set_edit_vertices(ptr: *mut EditorState, enabled: bool) {
    if let Some(state) = unsafe { ptr.as_mut() } {
        state.edit_vertices = enabled;
        if enabled {
            state.edit_faces = false;
            state.edit_edges = false;
        }
    }
}

/// Perform undo. Returns true if something was undone.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_undo(ptr: *mut EditorState) -> bool {
    if let Some(state) = unsafe { ptr.as_mut() } {
        state
            .undo
            .undo(
                &mut state.map,
                &mut state.selected_brushes,
                &mut state.selected_faces,
                &mut state.selected_edges,
                &mut state.selected_patch_vertices,
                &mut state.selected_entities,
                &mut state.map_revision,
            )
            .is_some()
    } else {
        false
    }
}

/// Perform redo. Returns true if something was redone.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_redo(ptr: *mut EditorState) -> bool {
    if let Some(state) = unsafe { ptr.as_mut() } {
        state
            .undo
            .redo(
                &mut state.map,
                &mut state.selected_brushes,
                &mut state.selected_faces,
                &mut state.selected_edges,
                &mut state.selected_patch_vertices,
                &mut state.selected_entities,
                &mut state.map_revision,
            )
            .is_some()
    } else {
        false
    }
}

/// Get the number of entities in the current map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_get_entity_count(ptr: *mut EditorState) -> i32 {
    if let Some(state) = unsafe { ptr.as_ref() } {
        state
            .map
            .as_ref()
            .map(|m| m.entities.len() as i32)
            .unwrap_or(0)
    } else {
        0
    }
}

/// Get a property value of an entity by key.
/// The returned string MUST be freed with kr_free_string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_get_entity_property(
    ptr: *mut EditorState,
    entity_idx: i32,
    key: *const c_char,
) -> *mut c_char {
    if ptr.is_null() || key.is_null() {
        return std::ptr::null_mut();
    }
    let state = unsafe { &*ptr };
    let key_str = match unsafe { CStr::from_ptr(key) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let val = state.map.as_ref().and_then(|m| {
        m.entities
            .get(entity_idx as usize)
            .and_then(|e| e.properties.get(key_str))
    });

    match val {
        Some(s) => CString::new(s.as_str()).unwrap().into_raw(),
        None => std::ptr::null_mut(),
    }
}

/// Free a string returned by the library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            drop(CString::from_raw(ptr));
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IVec2 {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl From<glam::Vec3> for Vec3 {
    fn from(v: glam::Vec3) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
        }
    }
}

impl From<Vec3> for glam::Vec3 {
    fn from(v: Vec3) -> Self {
        Self::new(v.x, v.y, v.z)
    }
}

impl From<glam::Vec2> for Vec2 {
    fn from(v: glam::Vec2) -> Self {
        Self { x: v.x, y: v.y }
    }
}

impl From<glam::IVec2> for IVec2 {
    fn from(v: glam::IVec2) -> Self {
        Self { x: v.x, y: v.y }
    }
}

impl From<glam::Quat> for Quat {
    fn from(q: glam::Quat) -> Self {
        Self {
            x: q.x,
            y: q.y,
            z: q.z,
            w: q.w,
        }
    }
}

/// Select a brush by entity and brush index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_select_brush(
    ptr: *mut EditorState,
    entity_idx: i32,
    brush_idx: i32,
) {
    if let Some(state) = unsafe { ptr.as_mut() } {
        state
            .selected_brushes
            .push((entity_idx as usize, brush_idx as usize));
    }
}

/// Translate the current selection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_translate_selection(ptr: *mut EditorState, delta: Vec3) -> bool {
    if let Some(state) = unsafe { ptr.as_mut() } {
        let dv = glam::Vec3::from(delta);
        if dv == glam::Vec3::ZERO {
            return false;
        }

        let label = if state.selected_brushes.len() == 1 {
            "Translate Brush"
        } else {
            "Translate Selection"
        };

        state.undo.push(
            label,
            &state.map,
            &state.selected_brushes,
            &state.selected_faces,
            &state.selected_edges,
            &state.selected_patch_vertices,
            &state.selected_entities,
        );

        if let Some(map) = state.map.as_mut() {
            for &(ei, bi) in &state.selected_brushes {
                if let Some(e) = map.entities.get_mut(ei) {
                    if let Some(b) = e.brushes.get_mut(bi) {
                        b.translate(&mut state.map_revision, dv);
                    }
                }
            }
            state.bump_revision();
            true
        } else {
            false
        }
    } else {
        false
    }
}

// --- Map Utils ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_map_count_brushes(map_ptr: *const crate::map::Map) -> i32 {
    if let Some(map) = unsafe { map_ptr.as_ref() } {
        map.entities.iter().map(|e| e.brushes.len()).sum::<usize>() as i32
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_compute_normal(p1: Vec3, p2: Vec3, p3: Vec3) -> Vec3 {
    let n = (glam::Vec3::from(p2) - glam::Vec3::from(p1))
        .cross(glam::Vec3::from(p3) - glam::Vec3::from(p1))
        .normalize();
    Vec3::from(n)
}

// --- Editor Access ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_get_map_ptr(ptr: *mut EditorState) -> *mut crate::map::Map {
    if let Some(state) = unsafe { ptr.as_mut() } {
        state
            .map
            .as_mut()
            .map(|m| m as *mut _)
            .unwrap_or(std::ptr::null_mut())
    } else {
        std::ptr::null_mut()
    }
}

// --- Assets ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_asset_db_new(maindir: *const c_char) -> *mut crate::assets::AssetDb {
    if maindir.is_null() {
        return std::ptr::null_mut();
    }
    let c_str = unsafe { CStr::from_ptr(maindir) };
    let path = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    match crate::assets::AssetDb::from_maindir(path) {
        Ok(db) => Box::into_raw(Box::new(db.0)),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_asset_db_free(ptr: *mut crate::assets::AssetDb) {
    if !ptr.is_null() {
        unsafe {
            drop(Box::from_raw(ptr));
        }
    }
}

// --- Texture ---

#[repr(C)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    pub data: *mut u8,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_asset_db_load_texture(
    ptr: *mut crate::assets::AssetDb,
    material: *const c_char,
) -> *mut TextureImage {
    if ptr.is_null() || material.is_null() {
        return std::ptr::null_mut();
    }
    let db = unsafe { &mut *ptr };
    let mat_str = match unsafe { CStr::from_ptr(material) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    match db.load_texture_rgba8(mat_str) {
        Ok(img) => {
            let mut data = img.rgba8.into_boxed_slice();
            let raw_data = data.as_mut_ptr();
            std::mem::forget(data); // C side will handle or we provide a free function

            Box::into_raw(Box::new(TextureImage {
                width: img.width,
                height: img.height,
                data: raw_data,
            }))
        }
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_texture_image_free(ptr: *mut TextureImage) {
    if !ptr.is_null() {
        unsafe {
            let img = Box::from_raw(ptr);
            // Re-construct the slice to free the data
            let _ = Box::from_raw(std::slice::from_raw_parts_mut(
                img.data,
                (img.width * img.height * 4) as usize,
            ));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_brush_get_face_count(brush_ptr: *const crate::map::Brush) -> i32 {
    if let Some(brush) = unsafe { brush_ptr.as_ref() } {
        match &brush.content {
            crate::map::BrushContent::Convex(faces) => faces.len() as i32,
            crate::map::BrushContent::Patch(_) => 0,
        }
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_brush_get_face_texture(
    brush_ptr: *const crate::map::Brush,
    face_idx: i32,
) -> *mut c_char {
    if let Some(brush) = unsafe { brush_ptr.as_ref() } {
        match &brush.content {
            crate::map::BrushContent::Convex(faces) => {
                if let Some(face) = faces.get(face_idx as usize) {
                    return CString::new(face.texture.as_str()).unwrap().into_raw();
                }
            }
            crate::map::BrushContent::Patch(_) => {}
        }
    }
    std::ptr::null_mut()
}

// --- Config & Viewport ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_get_grid_snap(ptr: *const EditorState) -> bool {
    if let Some(state) = unsafe { ptr.as_ref() } {
        state.config.view.grid_snap
    } else {
        false
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_set_grid_snap(ptr: *mut EditorState, enabled: bool) {
    if let Some(state) = unsafe { ptr.as_mut() } {
        state.config.view.grid_snap = enabled;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_get_view2d_pan(
    ptr: *const EditorState,
    x: *mut f32,
    y: *mut f32,
) {
    if let Some(state) = unsafe { ptr.as_ref() } {
        if !x.is_null() {
            unsafe { *x = state.view2d.pan[0] };
        }
        if !y.is_null() {
            unsafe { *y = state.view2d.pan[1] };
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_set_view2d_pan(ptr: *mut EditorState, x: f32, y: f32) {
    if let Some(state) = unsafe { ptr.as_mut() } {
        state.view2d.pan = [x, y];
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_brush_get_face_plane_point(
    brush_ptr: *const crate::map::Brush,
    face_idx: i32,
    point_idx: i32,
) -> Vec3 {
    if let Some(brush) = unsafe { brush_ptr.as_ref() } {
        match &brush.content {
            crate::map::BrushContent::Convex(faces) => {
                if let Some(face) = faces.get(face_idx as usize) {
                    if let Some(p) = face.plane_points.get(point_idx as usize) {
                        return Vec3::from(*p);
                    }
                }
            }
            crate::map::BrushContent::Patch(_) => {}
        }
    }
    Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_map_get_entity(
    map_ptr: *mut crate::map::Map,
    idx: i32,
) -> *mut crate::map::Entity {
    if let Some(map) = unsafe { map_ptr.as_mut() } {
        map.entities
            .get_mut(idx as usize)
            .map(|e| e as *mut _)
            .unwrap_or(std::ptr::null_mut())
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_entity_get_brush_count(ent_ptr: *const crate::map::Entity) -> i32 {
    if let Some(ent) = unsafe { ent_ptr.as_ref() } {
        ent.brushes.len() as i32
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_entity_get_brush(
    ent_ptr: *mut crate::map::Entity,
    idx: i32,
) -> *mut crate::map::Brush {
    if let Some(ent) = unsafe { ent_ptr.as_mut() } {
        ent.brushes
            .get_mut(idx as usize)
            .map(|b| b as *mut _)
            .unwrap_or(std::ptr::null_mut())
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_shader_db_load(
    main_dir: *const c_char,
) -> *mut crate::shader::ShaderDb {
    if main_dir.is_null() {
        return std::ptr::null_mut();
    }
    let c_str = unsafe { CStr::from_ptr(main_dir) };
    let path = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    match crate::shader::load_shader_db_from_main_dir(path) {
        Ok(db) => Box::into_raw(Box::new(db)),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_shader_db_free(ptr: *mut crate::shader::ShaderDb) {
    if !ptr.is_null() {
        unsafe {
            drop(Box::from_raw(ptr));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_geom_update_brush_plane(
    brush_ptr: *mut crate::map::Brush,
    plane_idx: i32,
    p1: Vec3,
    p2: Vec3,
    p3: Vec3,
) {
    if let Some(brush) = unsafe { brush_ptr.as_mut() } {
        // Note: we need a way to pass generation. For simplicity in FFI, we might want a version that handles it.
        // But for now, we'll just use a dummy generation or expose it.
        let mut dummy_gen = 0u64;
        brush.update_brush_plane(
            &mut dummy_gen,
            plane_idx as usize,
            [
                glam::Vec3::from(p1),
                glam::Vec3::from(p2),
                glam::Vec3::from(p3),
            ],
        );
    }
}

// TODO: update to take mask as argument
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_edit_pick_brush_by_ray(
    map_ptr: *mut crate::map::Map,
    ray_origin: Vec3,
    ray_dir: Vec3,
    out_ent: *mut i32,
    out_brush: *mut i32,
) -> bool {
    if let Some(map) = unsafe { map_ptr.as_mut() } {
        if let Some(res) = crate::editing::pick_brush_by_ray(
            map,
            glam::Vec3::from(ray_origin),
            glam::Vec3::from(ray_dir),
            crate::editing::PickMask::ALL,
            None,
        ) {
            if !out_ent.is_null() {
                unsafe { *out_ent = res.0 as i32 };
            }
            if !out_brush.is_null() {
                unsafe { *out_brush = res.1 as i32 };
            }
            return true;
        }
    }
    false
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_tex_q3_axes_from_normal(normal: Vec3, s: *mut Vec3, t: *mut Vec3) {
    let (rs, rt) = crate::texmap::q3_texture_axes_from_normal(glam::Vec3::from(normal));
    if !s.is_null() {
        unsafe { *s = Vec3::from(rs) };
    }
    if !t.is_null() {
        unsafe { *t = Vec3::from(rt) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_tex_rotate_axes(
    s: Vec3,
    t: Vec3,
    angle_rad: f32,
    rs: *mut Vec3,
    rt: *mut Vec3,
) {
    let (os, ot) =
        crate::texmap::rotate_texture_axes(glam::Vec3::from(s), glam::Vec3::from(t), angle_rad);
    if !rs.is_null() {
        unsafe { *rs = Vec3::from(os) };
    }
    if !rt.is_null() {
        unsafe { *rt = Vec3::from(ot) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_brush_get_polygon_count(brush_ptr: *mut crate::map::Brush) -> i32 {
    if let Some(brush) = unsafe { brush_ptr.as_mut() } {
        brush.get_polygons().map(|p| p.len() as i32).unwrap_or(0)
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_brush_get_polygon_data(
    brush_ptr: *mut crate::map::Brush,
    poly_idx: i32,
    out_vertex_count: *mut i32,
    out_vertices: *mut Vec3,
    out_index_count: *mut i32,
    out_indices: *mut u32,
) -> bool {
    if let Some(brush) = unsafe { brush_ptr.as_mut() } {
        if let Some(polys) = brush.get_polygons() {
            if let Some((verts, indices)) = polys.get(poly_idx as usize) {
                if !out_vertex_count.is_null() {
                    unsafe { *out_vertex_count = verts.len() as i32 };
                }
                if !out_index_count.is_null() {
                    unsafe { *out_index_count = indices.len() as i32 };
                }

                if !out_vertices.is_null() {
                    let out_slice =
                        unsafe { std::slice::from_raw_parts_mut(out_vertices, verts.len()) };
                    for (i, v) in verts.iter().enumerate() {
                        out_slice[i] = Vec3::from(*v);
                    }
                }

                if !out_indices.is_null() {
                    let out_slice =
                        unsafe { std::slice::from_raw_parts_mut(out_indices, indices.len()) };
                    out_slice.copy_from_slice(indices);
                }
                return true;
            }
        }
    }
    false
}

// --- View3D & Camera ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_get_view3d_cam(
    ptr: *const EditorState,
    pos: *mut Vec3,
    angles: *mut Vec3,
    zoom: *mut f32,
) {
    if let Some(state) = unsafe { ptr.as_ref() } {
        if !pos.is_null() {
            unsafe { *pos = Vec3::from(state.view3d.cam.pos) };
        }
        if !angles.is_null() {
            unsafe { *angles = Vec3::from(state.view3d.cam.angles) };
        }
        if !zoom.is_null() {
            unsafe { *zoom = state.view3d.cam.zoom };
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_set_view3d_cam(
    ptr: *mut EditorState,
    pos: Vec3,
    angles: Vec3,
    zoom: f32,
) {
    if let Some(state) = unsafe { ptr.as_mut() } {
        state.view3d.cam.pos = glam::Vec3::from(pos);
        state.view3d.cam.angles = glam::Vec3::from(angles);
        state.view3d.cam.zoom = zoom;
    }
}

// --- Property Strings ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_get_new_prop_key(ptr: *const EditorState) -> *mut c_char {
    if let Some(state) = unsafe { ptr.as_ref() } {
        CString::new(state.new_prop_key.as_str())
            .unwrap()
            .into_raw()
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_set_new_prop_key(ptr: *mut EditorState, val: *const c_char) {
    if let Some(state) = unsafe { ptr.as_mut() } {
        if !val.is_null() {
            if let Ok(s) = unsafe { CStr::from_ptr(val) }.to_str() {
                state.new_prop_key = s.to_string();
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_get_new_prop_val(ptr: *const EditorState) -> *mut c_char {
    if let Some(state) = unsafe { ptr.as_ref() } {
        CString::new(state.new_prop_val.as_str())
            .unwrap()
            .into_raw()
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_editor_set_new_prop_val(ptr: *mut EditorState, val: *const c_char) {
    if let Some(state) = unsafe { ptr.as_mut() } {
        if !val.is_null() {
            if let Ok(s) = unsafe { CStr::from_ptr(val) }.to_str() {
                state.new_prop_val = s.to_string();
            }
        }
    }
}

// --- Rendering Helpers ---

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TexVertex {
    pub pos: Vec3,
    pub normal: Vec3,
    pub uv: [f32; 2],
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_render_gen_grid(
    pan_x: f32,
    pan_y: f32,
    zoom: f32,
    width: f32,
    height: f32,
    minor_step: f32,
    out_vertex_count: *mut i32,
    out_vertices: *mut Vec3,
    is_major: bool,
) -> bool {
    let zoom = zoom.max(0.001);
    let half_w = width / (2.0 * zoom);
    let half_h = height / (2.0 * zoom);
    let view_left = -half_w - (pan_x / zoom);
    let view_right = half_w - (pan_x / zoom);
    let view_top = -half_h - (pan_y / zoom);
    let view_bottom = half_h - (pan_y / zoom);

    let step = if is_major {
        64.0f32.max(minor_step)
    } else {
        minor_step
    };

    let mut verts = Vec::new();
    let i0 = (view_left / step).floor() as i32 - 1;
    let i1 = (view_right / step).ceil() as i32 + 1;
    for i in i0..=i1 {
        let x = i as f32 * step;
        verts.push(Vec3::new(x, view_top, 0.0));
        verts.push(Vec3::new(x, view_bottom, 0.0));
    }

    let j0 = (view_top / step).floor() as i32 - 1;
    let j1 = (view_bottom / step).ceil() as i32 + 1;
    for j in j0..=j1 {
        let y = j as f32 * step;
        verts.push(Vec3::new(view_left, y, 0.0));
        verts.push(Vec3::new(view_right, y, 0.0));
    }

    if !out_vertex_count.is_null() {
        unsafe { *out_vertex_count = verts.len() as i32 };
    }

    if !out_vertices.is_null() {
        let out_slice = unsafe { std::slice::from_raw_parts_mut(out_vertices, verts.len()) };
        out_slice.copy_from_slice(&verts);
    }

    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_brush_get_wireframe_count(brush_ptr: *mut crate::map::Brush) -> i32 {
    if let Some(brush) = unsafe { brush_ptr.as_mut() } {
        match &mut brush.content {
            crate::map::BrushContent::Convex(_) => {
                if let Some(polys) = brush.get_polygons() {
                    let mut count = 0;
                    for (verts, _) in polys {
                        count += (verts.len() * 2) as i32;
                    }
                    count
                } else {
                    0
                }
            }
            crate::map::BrushContent::Patch(patch) => {
                if let Some((_, _, edges)) = patch.get_mesh_aabb_wire() {
                    (edges.len() * 2) as i32
                } else {
                    0
                }
            }
        }
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_brush_get_wireframe_data(
    brush_ptr: *mut crate::map::Brush,
    out_vertices: *mut Vec3,
) -> bool {
    if brush_ptr.is_null() || out_vertices.is_null() {
        return false;
    }
    let brush = unsafe { &mut *brush_ptr };
    let mut verts = Vec::new();

    match &mut brush.content {
        crate::map::BrushContent::Convex(_) => {
            if let Some(polys) = brush.get_polygons() {
                for (positions, _) in polys {
                    for i in 0..positions.len() {
                        verts.push(Vec3::from(positions[i]));
                        verts.push(Vec3::from(positions[(i + 1) % positions.len()]));
                    }
                }
            }
        }
        crate::map::BrushContent::Patch(patch) => {
            if let Some((mesh, _, edges)) = patch.get_mesh_aabb_wire() {
                let positions = mesh.positions.as_slice();
                for &(a, b) in edges {
                    verts.push(Vec3::from(positions[a as usize]));
                    verts.push(Vec3::from(positions[b as usize]));
                }
            }
        }
    }

    let out_slice = unsafe { std::slice::from_raw_parts_mut(out_vertices, verts.len()) };
    out_slice.copy_from_slice(&verts);
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_patch_get_tessellation_counts(
    brush_ptr: *mut crate::map::Brush,
    out_vertex_count: *mut i32,
    out_index_count: *mut i32,
) -> bool {
    if let Some(brush) = unsafe { brush_ptr.as_mut() } {
        if let crate::map::BrushContent::Patch(patch) = &brush.content {
            if let Ok(tess) = crate::geometry::tessellate_patch(patch) {
                if !out_vertex_count.is_null() {
                    unsafe { *out_vertex_count = tess.positions.len() as i32 };
                }
                if !out_index_count.is_null() {
                    unsafe { *out_index_count = tess.indices.len() as i32 };
                }
                return true;
            }
        }
    }
    false
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_patch_get_tessellation_data(
    brush_ptr: *mut crate::map::Brush,
    out_vertices: *mut TexVertex,
    out_indices: *mut u32,
) -> bool {
    if let Some(brush) = unsafe { brush_ptr.as_mut() } {
        if let crate::map::BrushContent::Patch(patch) = &brush.content {
            if let Ok(tess) = crate::geometry::tessellate_patch(patch) {
                if !out_vertices.is_null() {
                    let out_slice = unsafe {
                        std::slice::from_raw_parts_mut(out_vertices, tess.positions.len())
                    };
                    for i in 0..tess.positions.len() {
                        out_slice[i] = TexVertex {
                            pos: Vec3::from(tess.positions[i]),
                            normal: Vec3::from(tess.normals[i]),
                            uv: tess.uvs[i].to_array(),
                        };
                    }
                }
                if !out_indices.is_null() {
                    let out_slice =
                        unsafe { std::slice::from_raw_parts_mut(out_indices, tess.indices.len()) };
                    out_slice.copy_from_slice(&tess.indices);
                }
                return true;
            }
        }
    }
    false
}

// --- Texture Mapping (Texmap) ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_face_uv(
    brush_ptr: *const crate::map::Brush,
    face_idx: i32,
    point: Vec3,
    tex_w: f32,
    tex_h: f32,
    out_uv: *mut f32,
) {
    if let Some(brush) = unsafe { brush_ptr.as_ref() } {
        if let crate::map::BrushContent::Convex(faces) = &brush.content {
            if let Some(face) = faces.get(face_idx as usize) {
                let uv = crate::texmap::face_uv(face, glam::Vec3::from(point), tex_w, tex_h);
                if !out_uv.is_null() {
                    let out_slice = unsafe { std::slice::from_raw_parts_mut(out_uv, 2) };
                    out_slice[0] = uv.x;
                    out_slice[1] = uv.y;
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kr_face_plane_normal(
    brush_ptr: *const crate::map::Brush,
    face_idx: i32,
) -> Vec3 {
    if let Some(brush) = unsafe { brush_ptr.as_ref() } {
        if let crate::map::BrushContent::Convex(faces) = &brush.content {
            if let Some(face) = faces.get(face_idx as usize) {
                return Vec3::from(crate::texmap::face_plane_normal(face));
            }
        }
    }
    Vec3::ZERO
}
