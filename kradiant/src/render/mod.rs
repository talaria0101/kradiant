//! Render

pub mod texture_registry;
mod viewport2d;
mod viewport3d;

use crate::{Vec3, render::viewport3d::TexVertex};
use glow::HasContext;

use viewport3d::LitVertex;

pub use texture_registry::{RenderTextureInfo, TextureRegistry};
pub use viewport2d::Viewport2D;
pub use viewport3d::Viewport3D;

pub struct RenderBackend<'a> {
    pub gl: &'a glow::Context,
    pub wire_program: glow::Program,
    pub mvp_loc: glow::UniformLocation,
    pub color_loc: glow::UniformLocation,
    pub vao: glow::NativeVertexArray,
    pub vbo: glow::Buffer,
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
}

impl RenderBackend<'_> {
    pub unsafe fn draw_lines(&self, vertices: &[Vec3], color: [f32; 4], mvp: glam::Mat4) {
        unsafe {
            if vertices.is_empty() {
                return;
            }
            self.gl.use_program(Some(self.wire_program));
            self.gl
                .uniform_matrix_4_f32_slice(Some(&self.mvp_loc), false, &mvp.to_cols_array());
            self.gl.uniform_4_f32_slice(Some(&self.color_loc), &color);
            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(vertices),
                glow::STREAM_DRAW,
            );
            self.gl
                .vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
            self.gl.enable_vertex_attrib_array(0);
            self.gl.draw_arrays(glow::LINES, 0, vertices.len() as i32);
        }
    }

    pub unsafe fn draw_triangles(&self, vertices: &[Vec3], color: [f32; 4], mvp: glam::Mat4) {
        if vertices.is_empty() {
            return;
        }

        unsafe {
            self.gl.use_program(Some(self.wire_program)); // fine for now
            self.gl
                .uniform_matrix_4_f32_slice(Some(&self.mvp_loc), false, &mvp.to_cols_array());
            self.gl.uniform_4_f32_slice(Some(&self.color_loc), &color);

            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(vertices),
                glow::STREAM_DRAW,
            );

            self.gl
                .vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
            self.gl.enable_vertex_attrib_array(0);

            self.gl
                .draw_arrays(glow::TRIANGLES, 0, vertices.len() as i32);
        }
    }

    pub unsafe fn draw_triangles_lit(
        &self,
        vertices: &[LitVertex],
        color: [f32; 4],
        mvp: glam::Mat4,
        ambient: f32,
        light_dir: Vec3,
    ) {
        if vertices.is_empty() {
            return;
        }

        unsafe {
            self.gl.use_program(Some(self.lit_program));
            self.gl.uniform_matrix_4_f32_slice(
                Some(&self.lit_mvp_loc),
                false,
                &mvp.to_cols_array(),
            );
            self.gl
                .uniform_4_f32_slice(Some(&self.lit_color_loc), &color);
            self.gl.uniform_3_f32(
                Some(&self.lit_ldir_loc),
                light_dir.x,
                light_dir.y,
                light_dir.z,
            );
            self.gl.uniform_1_f32(Some(&self.lit_amb_loc), ambient);

            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(vertices),
                glow::STREAM_DRAW,
            );

            // stride = 24, pos at offset 0, normal at offset 12
            self.gl
                .vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 24, 0);
            self.gl.enable_vertex_attrib_array(0);
            self.gl
                .vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 24, 12);
            self.gl.enable_vertex_attrib_array(1);

            self.gl
                .draw_arrays(glow::TRIANGLES, 0, vertices.len() as i32);

            self.gl.disable_vertex_attrib_array(1); // don't leave attrib 1 enabled for wire draws
        }
    }

    pub unsafe fn draw_triangles_tex(
        &self,
        vertices: &[TexVertex],
        texture: glow::Texture,
        color: [f32; 4],
        mvp: glam::Mat4,
        ambient: f32,
        light_dir: Vec3,
    ) {
        if vertices.is_empty() {
            return;
        }

        unsafe {
            self.gl.use_program(Some(self.tex_program));

            self.gl.uniform_matrix_4_f32_slice(
                Some(&self.tex_mvp_loc),
                false,
                &mvp.to_cols_array(),
            );
            self.gl
                .uniform_4_f32_slice(Some(&self.tex_color_loc), &color);
            self.gl.uniform_3_f32(
                Some(&self.tex_ldir_loc),
                light_dir.x,
                light_dir.y,
                light_dir.z,
            );
            self.gl.uniform_1_f32(Some(&self.tex_amb_loc), ambient);
            self.gl.uniform_1_i32(Some(&self.tex_sampler_loc), 0);

            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(texture));

            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(vertices),
                glow::STREAM_DRAW,
            );

            self.gl
                .vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 32, 0);
            self.gl.enable_vertex_attrib_array(0);

            // Normal at offset 12
            self.gl
                .vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 32, 12);
            self.gl.enable_vertex_attrib_array(1);

            // UV at offset 24
            self.gl
                .vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, 32, 24);
            self.gl.enable_vertex_attrib_array(2);

            self.gl
                .draw_arrays(glow::TRIANGLES, 0, vertices.len() as i32);

            // Cleanup
            self.gl.disable_vertex_attrib_array(2);
            self.gl.disable_vertex_attrib_array(1);
            self.gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }
}
