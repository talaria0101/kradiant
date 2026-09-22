//! Render

pub mod texture_registry;
mod viewport2d;
mod viewport3d;

use crate::Vec3;

pub use texture_registry::{RenderTextureInfo, TextureRegistry};
pub use viewport2d::Viewport2D;
pub use viewport3d::{LitVertex, TexVertex, Viewport3D};

// ---------------------------------------------------------------------------
// Batch collector types — owned data, no lifetime headaches
// ---------------------------------------------------------------------------

/// A single line draw batch — owns vertex data for the frame.
pub struct LineBatch {
    pub vertices: Vec<Vec3>,
    pub color: [f32; 4],
    pub mvp: glam::Mat4,
}

/// A single lit triangle draw batch.
pub struct LitBatch {
    pub vertices: Vec<LitVertex>,
    pub color: [f32; 4],
    pub mvp: glam::Mat4,
    pub ambient: f32,
    pub light_dir: glam::Vec3,
}

/// A single textured triangle draw batch.
pub struct TexBatch {
    pub vertices: Vec<TexVertex>,
    pub texture: glow::Texture,
    pub color: [f32; 4],
    pub mvp: glam::Mat4,
    pub ambient: f32,
    pub light_dir: glam::Vec3,
}

/// Long-lived state for line batches. Persists across frames; owns the scratch
/// buffer used for concatenation.
pub struct LineBatchState {
    pub scratch: Vec<Vec3>,
}

impl LineBatchState {
    pub fn new() -> Self {
        Self {
            scratch: Vec::new(),
        }
    }
}

/// Long-lived state for lit triangle batches.
pub struct LitBatchState {
    pub scratch: Vec<LitVertex>,
}

impl LitBatchState {
    pub fn new() -> Self {
        Self {
            scratch: Vec::new(),
        }
    }
}

/// Long-lived state for textured triangle batches.
pub struct TexBatchState {
    pub scratch: Vec<TexVertex>,
}

impl TexBatchState {
    pub fn new() -> Self {
        Self {
            scratch: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Cached batch metadata — stored on viewport, used for fast draw without
// re-uploading vertex data.
// ---------------------------------------------------------------------------

/// Metadata for a cached textured triangle batch. Material is resolved to a
/// GPU texture at draw time (handles async texture loading).
pub struct CachedTexBatch {
    pub material: String,
    pub offset: i32,
    pub count: i32,
    pub alpha: f32,
    pub color: [f32; 4],
    /// Centroid of the batch geometry — used to sort transparent batches
    /// back-to-front relative to the camera each frame.
    pub centroid: Vec3,
}

/// Metadata for a cached lit triangle batch.
pub struct CachedLitBatch {
    pub offset: i32,
    pub count: i32,
    pub color: [f32; 4],
    pub ambient: f32,
    pub light_dir: Vec3,
}

/// Metadata for a cached line batch.
pub struct CachedLineBatch {
    pub offset: i32,
    pub count: i32,
    pub color: [f32; 4],
}

pub struct RenderBackend<'a> {
    pub gl: &'a glow::Context,
    pub wire_program: glow::Program,
    pub mvp_loc: glow::UniformLocation,
    pub color_loc: glow::UniformLocation,
    pub vao: glow::NativeVertexArray,
    pub vbo_line: glow::Buffer,
    pub vbo_lit: glow::Buffer,
    pub vbo_tex: glow::Buffer,
    pub vbo_dynamic: glow::Buffer,
    pub vbo_lit_dynamic: glow::Buffer,
    pub lit_program: glow::NativeProgram,
    pub lit_mvp_loc: glow::NativeUniformLocation,
    pub lit_color_loc: glow::NativeUniformLocation,
    pub lit_ldir_loc: glow::NativeUniformLocation,
    pub lit_amb_loc: glow::NativeUniformLocation,
    pub tex_program: glow::NativeProgram,
    pub tex_mvp_loc: glow::NativeUniformLocation,
    pub tex_color_loc: glow::NativeUniformLocation,
    pub tex_ldir_loc: glow::NativeUniformLocation,
    pub tex_amb_loc: glow::NativeUniformLocation,
    pub tex_sampler_loc: glow::NativeUniformLocation,
    pub missing_tex: glow::Texture,
    // Per-frame batch accumulators — cleared each viewport render
    pub line_batches: Vec<LineBatch>,
    pub lit_batches: Vec<LitBatch>,
    pub tex_batches: Vec<TexBatch>,
}

impl<'a> RenderBackend<'a> {
    pub fn enqueue_lines(&mut self, vertices: &[Vec3], color: [f32; 4], mvp: glam::Mat4) {
        if vertices.is_empty() {
            return;
        }
        self.line_batches.push(LineBatch {
            vertices: vertices.to_vec(),
            color,
            mvp,
        });
    }

    pub fn enqueue_triangles_lit(
        &mut self,
        vertices: &[LitVertex],
        color: [f32; 4],
        mvp: glam::Mat4,
        ambient: f32,
        light_dir: glam::Vec3,
    ) {
        if vertices.is_empty() {
            return;
        }
        self.lit_batches.push(LitBatch {
            vertices: vertices.to_vec(),
            color,
            mvp,
            ambient,
            light_dir,
        });
    }

    pub fn enqueue_triangles_tex(
        &mut self,
        vertices: &[TexVertex],
        texture: glow::Texture,
        color: [f32; 4],
        mvp: glam::Mat4,
        ambient: f32,
        light_dir: glam::Vec3,
    ) {
        if vertices.is_empty() {
            return;
        }
        self.tex_batches.push(TexBatch {
            vertices: vertices.to_vec(),
            texture,
            color,
            mvp,
            ambient,
            light_dir,
        });
    }
}

// ---------------------------------------------------------------------------
// Flush functions — free functions taking explicit disjoint field references.
//
// Why free functions instead of `&mut self` methods on Viewport3D/Viewport2D?
// The enqueue call borrows `&self.line_vertices` (etc.) into the batch list.
// That borrow must stay alive until flush reads it. A `&mut self` method
// would require exclusive access to the entire viewport struct, conflicting
// with the outstanding shared borrow. By passing `&mut self.line_batch_state`
// explicitly, the compiler can verify the fields are disjoint.
// ---------------------------------------------------------------------------

/// Flush line batches: concatenate, upload, replay draw calls.
///
/// # Safety
/// Caller must ensure GL context is valid and all handles are correct.
pub unsafe fn flush_lines(
    gl: &glow::Context,
    batches: &mut Vec<LineBatch>,
    state: &mut LineBatchState,
    vbo: glow::Buffer,
    wire_program: glow::Program,
    mvp_loc: &glow::UniformLocation,
    color_loc: &glow::UniformLocation,
    vao: glow::NativeVertexArray,
) {
    use glow::HasContext;

    if batches.is_empty() {
        return;
    }

    state.scratch.clear();
    for b in batches.iter() {
        state.scratch.extend_from_slice(&b.vertices);
    }

    unsafe {
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(&state.scratch),
            glow::STREAM_DRAW,
        );

        gl.use_program(Some(wire_program));
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
        gl.enable_vertex_attrib_array(0);

        let mut offset = 0i32;
        for b in batches.iter() {
            let count = b.vertices.len() as i32;
            gl.uniform_matrix_4_f32_slice(Some(mvp_loc), false, &b.mvp.to_cols_array());
            gl.uniform_4_f32_slice(Some(color_loc), &b.color);
            gl.draw_arrays(glow::LINES, offset, count);
            offset += count;
        }
    }

    batches.clear();
}

/// Flush lit triangle batches: concatenate, upload, replay draw calls.
///
/// # Safety
/// Caller must ensure GL context is valid and all handles are correct.
pub unsafe fn flush_lit(
    gl: &glow::Context,
    batches: &mut Vec<LitBatch>,
    state: &mut LitBatchState,
    vbo: glow::Buffer,
    lit_program: glow::NativeProgram,
    lit_mvp_loc: &glow::NativeUniformLocation,
    lit_color_loc: &glow::NativeUniformLocation,
    lit_ldir_loc: &glow::NativeUniformLocation,
    lit_amb_loc: &glow::NativeUniformLocation,
    vao: glow::NativeVertexArray,
) {
    use glow::HasContext;

    if batches.is_empty() {
        return;
    }

    state.scratch.clear();
    for b in batches.iter() {
        state.scratch.extend_from_slice(&b.vertices);
    }

    unsafe {
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(&state.scratch),
            glow::STREAM_DRAW,
        );

        gl.use_program(Some(lit_program));
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 24, 0);
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 24, 12);
        gl.enable_vertex_attrib_array(1);
        gl.disable_vertex_attrib_array(2);

        let mut offset = 0i32;
        for b in batches.iter() {
            let count = b.vertices.len() as i32;
            gl.uniform_matrix_4_f32_slice(Some(lit_mvp_loc), false, &b.mvp.to_cols_array());
            gl.uniform_4_f32_slice(Some(lit_color_loc), &b.color);
            gl.uniform_3_f32(
                Some(lit_ldir_loc),
                b.light_dir.x,
                b.light_dir.y,
                b.light_dir.z,
            );
            gl.uniform_1_f32(Some(lit_amb_loc), b.ambient);
            gl.draw_arrays(glow::TRIANGLES, offset, count);
            offset += count;
        }
    }

    batches.clear();
}

/// Flush textured triangle batches: concatenate, upload, replay draw calls.
///
/// # Safety
/// Caller must ensure GL context is valid and all handles are correct.
pub unsafe fn flush_tex(
    gl: &glow::Context,
    batches: &mut Vec<TexBatch>,
    state: &mut TexBatchState,
    vbo: glow::Buffer,
    tex_program: glow::NativeProgram,
    tex_mvp_loc: &glow::NativeUniformLocation,
    tex_color_loc: &glow::NativeUniformLocation,
    tex_ldir_loc: &glow::NativeUniformLocation,
    tex_amb_loc: &glow::NativeUniformLocation,
    tex_sampler_loc: &glow::NativeUniformLocation,
    vao: glow::NativeVertexArray,
) {
    use glow::HasContext;

    if batches.is_empty() {
        return;
    }

    state.scratch.clear();
    for b in batches.iter() {
        state.scratch.extend_from_slice(&b.vertices);
    }

    unsafe {
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(&state.scratch),
            glow::STREAM_DRAW,
        );

        gl.use_program(Some(tex_program));
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 32, 0);
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 32, 12);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, 32, 24);
        gl.enable_vertex_attrib_array(2);

        let mut offset = 0i32;
        for b in batches.iter() {
            let count = b.vertices.len() as i32;
            gl.uniform_matrix_4_f32_slice(Some(tex_mvp_loc), false, &b.mvp.to_cols_array());
            gl.uniform_4_f32_slice(Some(tex_color_loc), &b.color);
            gl.uniform_3_f32(
                Some(tex_ldir_loc),
                b.light_dir.x,
                b.light_dir.y,
                b.light_dir.z,
            );
            gl.uniform_1_f32(Some(tex_amb_loc), b.ambient);
            gl.uniform_1_i32(Some(tex_sampler_loc), 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(b.texture));
            gl.draw_arrays(glow::TRIANGLES, offset, count);
            offset += count;
        }

        gl.disable_vertex_attrib_array(2);
        gl.disable_vertex_attrib_array(1);
        gl.bind_texture(glow::TEXTURE_2D, None);
    }

    batches.clear();
}

// ---------------------------------------------------------------------------
// Cached draw functions — replay draw calls from a pre-built VBO without
// re-uploading vertex data. Used by the 3D viewport's draw phase.
// ---------------------------------------------------------------------------

/// Draw lit triangles from a cached VBO.
///
/// # Safety
/// Caller must ensure GL context is valid and all handles are correct.
pub unsafe fn draw_lit_cached(
    gl: &glow::Context,
    cached_batches: &[CachedLitBatch],
    vbo: glow::Buffer,
    lit_program: glow::NativeProgram,
    lit_mvp_loc: &glow::NativeUniformLocation,
    lit_color_loc: &glow::NativeUniformLocation,
    lit_ldir_loc: &glow::NativeUniformLocation,
    lit_amb_loc: &glow::NativeUniformLocation,
    mvp: glam::Mat4,
    vao: glow::NativeVertexArray,
) {
    use glow::HasContext;

    if cached_batches.is_empty() {
        return;
    }

    unsafe {
        gl.use_program(Some(lit_program));
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 24, 0);
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 24, 12);
        gl.enable_vertex_attrib_array(1);
        gl.disable_vertex_attrib_array(2);

        for b in cached_batches {
            gl.uniform_matrix_4_f32_slice(Some(lit_mvp_loc), false, &mvp.to_cols_array());
            gl.uniform_4_f32_slice(Some(lit_color_loc), &b.color);
            gl.uniform_3_f32(
                Some(lit_ldir_loc),
                b.light_dir.x,
                b.light_dir.y,
                b.light_dir.z,
            );
            gl.uniform_1_f32(Some(lit_amb_loc), b.ambient);
            gl.draw_arrays(glow::TRIANGLES, b.offset, b.count);
        }
    }
}

/// Draw textured triangles from a cached VBO.
/// Resolves material names to textures at draw time.
/// If `transparent_only` is true, draws only batches with alpha < 1.0.
/// If `transparent_only` is false, draws only batches with alpha >= 1.0.
///
/// # Safety
/// Caller must ensure GL context is valid and all handles are correct.
pub unsafe fn draw_tex_cached(
    gl: &glow::Context,
    cached_batches: &[CachedTexBatch],
    tex_registry: &TextureRegistry,
    missing_tex: glow::Texture,
    vbo: glow::Buffer,
    tex_program: glow::NativeProgram,
    tex_mvp_loc: &glow::NativeUniformLocation,
    tex_color_loc: &glow::NativeUniformLocation,
    tex_ldir_loc: &glow::NativeUniformLocation,
    tex_amb_loc: &glow::NativeUniformLocation,
    tex_sampler_loc: &glow::NativeUniformLocation,
    mvp: glam::Mat4,
    ambient: f32,
    light_dir: glam::Vec3,
    vao: glow::NativeVertexArray,
    transparent_only: bool,
) {
    use glow::HasContext;

    if cached_batches.is_empty() {
        return;
    }

    let mut found = 0u32;
    let mut missing = 0u32;

    unsafe {
        gl.use_program(Some(tex_program));
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 32, 0);
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 32, 12);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, 32, 24);
        gl.enable_vertex_attrib_array(2);

        for b in cached_batches {
            let is_transparent = b.alpha < 1.0;
            if transparent_only != is_transparent {
                continue;
            }

            if let Some(rt) = tex_registry.get(&b.material) {
                gl.uniform_matrix_4_f32_slice(Some(tex_mvp_loc), false, &mvp.to_cols_array());
                gl.uniform_4_f32_slice(Some(tex_color_loc), &b.color);
                gl.uniform_3_f32(Some(tex_ldir_loc), light_dir.x, light_dir.y, light_dir.z);
                gl.uniform_1_f32(Some(tex_amb_loc), ambient);
                gl.uniform_1_i32(Some(tex_sampler_loc), 0);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(rt.tex));
                gl.draw_arrays(glow::TRIANGLES, b.offset, b.count);
                found += 1;
            } else {
                gl.uniform_matrix_4_f32_slice(Some(tex_mvp_loc), false, &mvp.to_cols_array());
                gl.uniform_4_f32_slice(Some(tex_color_loc), &b.color);
                gl.uniform_3_f32(Some(tex_ldir_loc), light_dir.x, light_dir.y, light_dir.z);
                gl.uniform_1_f32(Some(tex_amb_loc), ambient);
                gl.uniform_1_i32(Some(tex_sampler_loc), 0);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(missing_tex));
                gl.draw_arrays(glow::TRIANGLES, b.offset, b.count);
                missing += 1;
            }
        }

        gl.disable_vertex_attrib_array(2);
        gl.disable_vertex_attrib_array(1);
        gl.bind_texture(glow::TEXTURE_2D, None);
    }

    if missing > 0 {
        log::debug!(
            "draw_tex_cached: {} batches found, {} missing (transparent_only={})",
            found,
            missing,
            transparent_only,
        );
    }
}

/// Draw lines from a cached VBO.
///
/// # Safety
/// Caller must ensure GL context is valid and all handles are correct.
pub unsafe fn draw_line_cached(
    gl: &glow::Context,
    cached_batches: &[CachedLineBatch],
    vbo: glow::Buffer,
    wire_program: glow::Program,
    mvp_loc: &glow::UniformLocation,
    color_loc: &glow::UniformLocation,
    mvp: glam::Mat4,
    vao: glow::NativeVertexArray,
) {
    use glow::HasContext;

    if cached_batches.is_empty() {
        return;
    }

    unsafe {
        gl.use_program(Some(wire_program));
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
        gl.enable_vertex_attrib_array(0);

        for b in cached_batches {
            gl.uniform_matrix_4_f32_slice(Some(mvp_loc), false, &mvp.to_cols_array());
            gl.uniform_4_f32_slice(Some(color_loc), &b.color);
            gl.draw_arrays(glow::LINES, b.offset, b.count);
        }
    }
}
