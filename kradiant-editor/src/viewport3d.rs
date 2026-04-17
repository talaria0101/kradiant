//! 3D Viewport

use glam::Vec3;
use kradiant::map::BrushContent;
use kradiant::geometry::tessellate_patch;
use crate::RenderBackend;
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View3dCache {
    map_present: bool,
    map_ptr: usize,
    map_generation: u64,
    map_revision: u64,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LitVertex {
    pub pos:    [f32; 3],
    pub normal: [f32; 3],
}

pub struct Viewport3D {
    line_vertices: Vec<Vec3>,
    tri_vertices: Vec<LitVertex>,
    tri_vertices_selected: Vec<LitVertex>,
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    rbo: glow::Renderbuffer, // depth
    fbo_size: [u32; 2],
    cache: Option<View3dCache>,
    last_selection_hash: u64,
}

impl Viewport3D {
    pub fn new(line_vertices: Vec<Vec3>, tri_vertices: Vec<LitVertex>, tri_vertices_selected: Vec<LitVertex>, fbo: glow::Framebuffer, fbo_size: [u32; 2], tex: glow::Texture, rbo: glow::Renderbuffer, cache: Option<View3dCache>) -> Self
    {
        Self {
            line_vertices,
            tri_vertices,
            tri_vertices_selected,
            fbo,
            fbo_size,
            tex,
            rbo,
            cache,
            last_selection_hash: 0,
        }
    }
    pub fn render(&mut self, backend: &mut RenderBackend<'_>, editor: &mut ui::EditorState) {
        unsafe {
            use glow::HasContext;

            let r3 = editor.view3d.rect;
            if r3[2] <= 10.0 || r3[3] <= 10.0 {
                return;
            }

            let fbo_w = r3[2] as u32;
            let fbo_h = r3[3] as u32;

            if [fbo_w, fbo_h] != self.fbo_size {
                self.fbo_size = [fbo_w, fbo_h];
                backend.gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
                backend.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    fbo_w as i32,
                    fbo_h as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                backend
                .gl
                .bind_renderbuffer(glow::RENDERBUFFER, Some(self.rbo));
                backend.gl.renderbuffer_storage(
                    glow::RENDERBUFFER,
                    glow::DEPTH_COMPONENT16,
                    fbo_w as i32,
                    fbo_h as i32,
                );
            }

            // Geometry cache
            let map_present = editor.map.is_some();
            let map_ptr = editor
            .map
            .as_ref()
            .map(|m| m as *const _ as usize)
            .unwrap_or(0);
            let map_generation = editor.map.as_ref().map(|m| m.generation).unwrap_or(0u64);
            let map_revision = editor.map_revision;

            let need_rebuild = match self.cache {
                Some(c) => {
                    c.map_present != map_present
                    || c.map_ptr != map_ptr
                    || c.map_generation != map_generation
                    || c.map_revision != map_revision
                }
                None => true,
            };

            let mut bounds_min = Vec3::splat(f32::INFINITY);
            let mut bounds_max = Vec3::splat(f32::NEG_INFINITY);

            if need_rebuild {
                let prev_revision = self.cache.map(|c| c.map_revision).unwrap_or(0);
                self.line_vertices.clear();
                self.tri_vertices.clear();

                if let Some(map) = editor.map.as_mut() {
                    // Cheap reserve to reduce reallocs on medium maps
                    self.line_vertices.reserve(32_768);

                    for ent in &mut map.entities {
                        for brush in &mut ent.brushes {
                            match &mut brush.content {
                                BrushContent::Convex(_) => {
                                    let Some((aabb, polys)) = brush.get_polygons_and_aabb() else {
                                        continue;
                                    };
                                    bounds_min = bounds_min.min(aabb.min);
                                    bounds_max = bounds_max.max(aabb.max);
                                    for (positions, _) in polys {
                                        if positions.len() < 2 {
                                            continue;
                                        }
                                        for i in 0..positions.len() {
                                            let a = positions[i];
                                            let b = positions[(i + 1) % positions.len()];
                                            self.line_vertices.push(a);
                                            self.line_vertices.push(b);
                                        }
                                    }

                                    // replace the existing tri_vertices push block:
                                    for (positions, _) in polys {
                                        if positions.len() < 3 { continue; }
                                        // face normal from first triangle of this polygon
                                        let e1 = positions[1] - positions[0];
                                        let e2 = positions[2] - positions[0];
                                        let n = e1.cross(e2).normalize_or_zero();
                                        let nf = [n.x, n.y, n.z];

                                        for i in 1..positions.len() - 1 {
                                            let v0 = positions[0];
                                            let v1 = positions[i];
                                            let v2 = positions[i + 1];
                                            self.tri_vertices.push(LitVertex { pos: v0.into(), normal: nf });
                                            self.tri_vertices.push(LitVertex { pos: v1.into(), normal: nf });
                                            self.tri_vertices.push(LitVertex { pos: v2.into(), normal: nf });
                                        }
                                    }
                                }
                                BrushContent::Patch(patch) => {
                                    let Some((mesh, patch_aabb, edges)) =
                                    patch.get_mesh_aabb_wire()
                                    else {
                                        continue;
                                    };
                                    bounds_min = bounds_min.min(patch_aabb.min);
                                    bounds_max = bounds_max.max(patch_aabb.max);
                                    let positions = mesh.positions.as_slice();
                                    for &(a, b) in edges {
                                        let ia = a as usize;
                                        let ib = b as usize;
                                        if ia >= positions.len() || ib >= positions.len() {
                                            continue;
                                        }
                                        self.line_vertices.push(positions[ia]);
                                        self.line_vertices.push(positions[ib]);
                                    }

                                    if let Ok(tess) = tessellate_patch(patch) {
                                        let pos = &tess.positions;
                                        let indices = &tess.indices;

                                        for chunk in indices.chunks_exact(3) {
                                            let i0 = chunk[0] as usize;
                                            let i1 = chunk[1] as usize;
                                            let i2 = chunk[2] as usize;

                                            if i0 >= pos.len() || i1 >= pos.len() || i2 >= pos.len() {
                                                continue;
                                            }

                                            let v0 = pos[i0];
                                            let v1 = pos[i1];
                                            let v2 = pos[i2];

                                            // Flat face normal (same as convex brushes)
                                            let n = (v1 - v0).cross(v2 - v0).normalize_or_zero();
                                            let nf = [n.x, n.y, n.z];

                                            self.tri_vertices.push(LitVertex { pos: v0.into(), normal: nf });
                                            self.tri_vertices.push(LitVertex { pos: v1.into(), normal: nf });
                                            self.tri_vertices.push(LitVertex { pos: v2.into(), normal: nf });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                self.cache = Some(View3dCache {
                    map_present,
                    map_ptr,
                    map_generation,
                    map_revision,
                });
                self.last_selection_hash = 5382;

                // Auto-frame camera on new map load (pointer change)
                if map_ptr != 0
                    && map_revision != prev_revision
                    && bounds_min.x.is_finite()
                    && bounds_max.x.is_finite()
                {
                    let center = (bounds_min + bounds_max) * 0.5;
                    let ext = bounds_max - bounds_min;
                    let radius = (ext.length() * 0.5).max(64.0);

                    let offset = Vec3::new(-1.0, -1.0, 0.65).normalize() * (radius * 2.5);
                    let eye = center + offset;
                    let dir = (center - eye).normalize();
                    let yaw = dir.y.atan2(dir.x);
                    let pitch = dir.z.asin();

                    editor.view3d.cam.pos = eye;
                    editor.view3d.cam.angles = Vec3::new(yaw, pitch, 0.0);
                    if editor.view3d.cam.zoom <= 1.0 {
                        editor.view3d.cam.zoom = (radius * 0.08).clamp(16.0, 512.0);
                    }
                }
            }

            let current_selection_hash = {
                let mut h: u64 = 5381; // Non-zero initial value to avoid collision with empty selection
                let mut items: Vec<_> = editor.selected_brushes.iter().collect();
                items.sort_by_key(|&&(e, b)| (e, b));
                for &&(e, b) in &items {
                    h = h.wrapping_mul(31).wrapping_add((e as u64) * 1000003 + (b as u64));
                }
                h
            };
            println!("current_selection_hash: {}\nlast_selection_hash: {}", current_selection_hash, self.last_selection_hash);

            if current_selection_hash != self.last_selection_hash {
                self.tri_vertices_selected.clear();

                if let Some(map) = editor.map.as_mut() {
                    for (entity_idx, ent) in map.entities.iter_mut().enumerate() {
                        for (brush_idx, brush) in ent.brushes.iter_mut().enumerate() {
                            let is_selected = editor.selected_brushes.contains(&(entity_idx, brush_idx));

                            if !is_selected {
                                continue;
                            }

                            match &mut brush.content {
                                BrushContent::Convex(_) => {
                                    let Some(polys) = brush.get_polygons() else {
                                        continue;
                                    };

                                    // replace the existing tri_vertices push block:
                                    for (positions, _) in polys {
                                        if positions.len() < 3 { continue; }
                                        // face normal from first triangle of this polygon
                                        let e1 = positions[1] - positions[0];
                                        let e2 = positions[2] - positions[0];
                                        let n = e1.cross(e2).normalize_or_zero();
                                        let nf = [n.x, n.y, n.z];

                                        for i in 1..positions.len() - 1 {
                                            let v0 = positions[0];
                                            let v1 = positions[i];
                                            let v2 = positions[i + 1];
                                            self.tri_vertices_selected.push(LitVertex { pos: v0.into(), normal: nf });
                                            self.tri_vertices_selected.push(LitVertex { pos: v1.into(), normal: nf });
                                            self.tri_vertices_selected.push(LitVertex { pos: v2.into(), normal: nf });
                                        }
                                    }
                                }
                                BrushContent::Patch(patch) => {
                                    //let positions = mesh.positions.as_slice();

                                    if let Ok(tess) = tessellate_patch(patch) {
                                        let pos = &tess.positions;
                                        let indices = &tess.indices;

                                        for chunk in indices.chunks_exact(3) {
                                            let i0 = chunk[0] as usize;
                                            let i1 = chunk[1] as usize;
                                            let i2 = chunk[2] as usize;

                                            if i0 >= pos.len() || i1 >= pos.len() || i2 >= pos.len() {
                                                continue;
                                            }

                                            let v0 = pos[i0];
                                            let v1 = pos[i1];
                                            let v2 = pos[i2];

                                            // Flat face normal (same as convex brushes)
                                            let n = (v1 - v0).cross(v2 - v0).normalize_or_zero();
                                            let nf = [n.x, n.y, n.z];

                                            self.tri_vertices_selected.push(LitVertex { pos: v0.into(), normal: nf });
                                            self.tri_vertices_selected.push(LitVertex { pos: v1.into(), normal: nf });
                                            self.tri_vertices_selected.push(LitVertex { pos: v2.into(), normal: nf });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                self.last_selection_hash = current_selection_hash;
            }

            // Draw into FBO
            backend
            .gl
            .bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            backend.gl.viewport(0, 0, fbo_w as i32, fbo_h as i32);

            let view_bg = editor.palette.view2d_bg;
            backend
            .gl
            .clear_color(view_bg[0], view_bg[1], view_bg[2], view_bg[3]);
            backend
            .gl
            .clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            backend.gl.enable(glow::DEPTH_TEST);
            backend.gl.depth_func(glow::LEQUAL);

            let cam = &editor.view3d.cam;
            let (yaw, pitch) = (cam.angles.x, cam.angles.y);
            let forward = Vec3::new(
                yaw.cos() * pitch.cos(),
                                    yaw.sin() * pitch.cos(),
                                    pitch.sin(),
            )
            .normalize();
            let eye = cam.pos;
            let target = eye + forward;
            let world_up = Vec3::Z;
            let mut right = forward.cross(world_up);
            if right.length_squared() <= 1e-8 {
                right = Vec3::X;
            }
            right = right.normalize();
            let up = right.cross(forward).normalize_or_zero();

            let view = glam::Mat4::look_at_rh(eye, target, up);
            let aspect = fbo_w as f32 / (fbo_h as f32).max(1.0);
            let fov = editor.config.view3d_fov.to_radians().clamp(0.1, 3.0);
            let proj = glam::Mat4::perspective_rh_gl(fov, aspect, 4.0, 100_000.0);
            let mvp = proj * view;

            let geom_col = editor.palette.view2d_geometry;
            let light_dir = Vec3::new(0.5, 0.25, 1.0).normalize();
            let ambient = 0.4f32;

            backend.draw_triangles_lit(&self.tri_vertices, geom_col, mvp, ambient, light_dir);
            backend.draw_lines(&self.line_vertices, geom_col, mvp);

            if !self.tri_vertices_selected.is_empty() {
                backend.gl.enable(glow::BLEND);
                backend.gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                // Disable depth write so the tint doesn't occlude the wire on top
                backend.gl.depth_mask(false);

                let red_tint = [1.0, 0.25, 0.25, 0.65];
                backend.draw_triangles_lit(
                    &self.tri_vertices_selected,
                    red_tint,
                    mvp,
                    ambient * 0.7,
                    light_dir,
                );

                backend.gl.depth_mask(true);
                backend.gl.disable(glow::BLEND);
            }

            backend.gl.disable(glow::DEPTH_TEST);
            backend.gl.use_program(None);
            backend.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }
}
