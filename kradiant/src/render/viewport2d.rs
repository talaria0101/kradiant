//! 2D Viewport

use crate::assets::normalize_material_name;
use crate::core_util::{self, project_to_2d};
use crate::core_util::{arrow_line_vertices, box_line_vertices, entity_angles_forward};
use crate::editing::AffineScale;
use crate::editor::EditorState;
use crate::editor::config::{EntityDrawAnchor, EntityDrawKind};
use crate::editor::viewport::{DragMode, Ortho, StretchMode};
use crate::map::BrushContent;
use crate::render::RenderBackend;
use crate::xmodel::XModel;
use glam::Vec3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View2dCache {
    axis: Ortho,
    map_present: bool,
    map_ptr: usize,
    map_generation: u64,
    map_revision: u64,
    zoom: f32,
    // Expanded cull bounds (world units in the 2D plane coordinates).
    cull_left: f32,
    cull_right: f32,
    cull_top: f32,
    cull_bottom: f32,
    view_config_rev: u64,
}

pub struct Viewport2D {
    line_vertices: Vec<Vec3>,
    entity_line_batches: Vec<([f32; 4], Vec<Vec3>)>,
    selected_vertices: Vec<Vec3>,
    control_vertices: Vec<Vec3>,
    control_selected_vertices: Vec<Vec3>,
    grid_vertices: Vec<Vec3>,
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    rbo: glow::Renderbuffer, // depth
    fbo_size: [u32; 2],
    cache: Option<View2dCache>,
}

impl Viewport2D {
    pub fn new(
        fbo: glow::Framebuffer,
        fbo_size: [u32; 2],
        rbo: glow::Renderbuffer,
        tex: glow::Texture,
    ) -> Self {
        Self {
            line_vertices: vec![],
            entity_line_batches: Vec::new(),
            selected_vertices: vec![],
            control_vertices: vec![],
            control_selected_vertices: vec![],
            grid_vertices: vec![],
            fbo,
            fbo_size,
            tex,
            rbo,
            cache: None,
        }
    }

    pub fn render(&mut self, backend: &mut RenderBackend<'_>, editor: &mut EditorState) {
        unsafe {
            use glow::HasContext;

            let r2 = editor.view2d.rect;
            if r2[2] <= 10.0 || r2[3] <= 10.0 {
                return;
            }

            let fbo_w = r2[2] as u32;
            let fbo_h = r2[3] as u32;

            // Resize FBO texture if the panel size changed.
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

            // Draw into FBO.
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

            backend.gl.use_program(Some(backend.wire_program));

            let zoom = editor.view2d.zoom.max(0.001);
            // `view2d_pan` is in pixels (screen space). Convert to world units here.
            // Note: view_top/view_bottom are swapped in ortho matrix to render flipped Y
            // so the texture matches ImGui's Y-down coordinate system.
            let half_w = fbo_w as f32 / (2.0 * zoom);
            let half_h = fbo_h as f32 / (2.0 * zoom);
            let pan_x = editor.view2d.pan[0] / zoom;
            let pan_y = editor.view2d.pan[1] / zoom;
            let view_left = -half_w - pan_x;
            let view_right = half_w - pan_x;
            let view_top = -half_h - pan_y;
            let view_bottom = half_h - pan_y;
            let ortho = glam::Mat4::orthographic_rh_gl(
                view_left,
                view_right,
                view_bottom,
                view_top,
                -1024.0,
                1024.0,
            );
            backend.gl.uniform_matrix_4_f32_slice(
                Some(&backend.mvp_loc),
                false,
                &ortho.to_cols_array(),
            );

            backend.gl.bind_vertex_array(Some(backend.vao));
            backend
                .gl
                .bind_buffer(glow::ARRAY_BUFFER, Some(backend.vbo));
            backend
                .gl
                .vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
            backend.gl.enable_vertex_attrib_array(0);

            #[inline(always)]
            unsafe fn draw_lines(backend: &RenderBackend<'_>, vertices: &[Vec3], color: [f32; 4]) {
                if vertices.is_empty() {
                    return;
                }
                unsafe {
                    backend
                        .gl
                        .uniform_4_f32_slice(Some(&backend.color_loc), &color);
                    backend.gl.buffer_data_u8_slice(
                        glow::ARRAY_BUFFER,
                        bytemuck::cast_slice(vertices),
                        glow::STREAM_DRAW,
                    );
                    backend
                        .gl
                        .draw_arrays(glow::LINES, 0, vertices.len() as i32);
                }
            }

            let axis = editor.view2d.ortho_axis;

            // Draw grid under map lines.
            const MIN_MINOR_STEP_PX: f32 = 1.0;
            const MIN_MAJOR_STEP_PX: f32 = 8.0;
            const MAJOR_PROMOTE_FACTOR: f32 = 64.0;

            let base_minor_world = editor.config.view.grid_minor_step.max(1) as f32;
            let mut minor_world = base_minor_world;
            let mut major_world = 64.0f32.max(minor_world);

            // Promote major grid levels while it would draw "minor dense".
            for _ in 0..16 {
                if major_world * zoom >= MIN_MAJOR_STEP_PX {
                    break;
                }
                minor_world = major_world;
                major_world *= MAJOR_PROMOTE_FACTOR;
            }

            let major_step_px = major_world * zoom;
            let minor_step_px = minor_world * zoom;

            if major_step_px >= MIN_MINOR_STEP_PX {
                // Major grid
                self.grid_vertices.clear();

                let i0 = (view_left / major_world).floor() as i32 - 1;
                let i1 = (view_right / major_world).ceil() as i32 + 1;
                for i in i0..=i1 {
                    let x = i as f32 * major_world;
                    self.grid_vertices.push(Vec3::new(x, view_top, 0.0));
                    self.grid_vertices.push(Vec3::new(x, view_bottom, 0.0));
                }

                let j0 = (view_top / major_world).floor() as i32 - 1;
                let j1 = (view_bottom / major_world).ceil() as i32 + 1;
                for j in j0..=j1 {
                    let y = j as f32 * major_world;
                    self.grid_vertices.push(Vec3::new(view_left, y, 0.0));
                    self.grid_vertices.push(Vec3::new(view_right, y, 0.0));
                }

                draw_lines(
                    backend,
                    &self.grid_vertices,
                    editor.palette.view2d_grid_major,
                );
            }

            if major_step_px >= MIN_MINOR_STEP_PX
                && minor_step_px >= MIN_MINOR_STEP_PX
                && (minor_world < major_world)
            {
                // Minor grid (skip where major grid will draw, when aligned)
                self.grid_vertices.clear();

                let ratio = major_world / minor_world;
                let major_factor = if (ratio - ratio.round()).abs() < 1.0e-4 && ratio >= 1.0 {
                    Some(ratio.round() as i32)
                } else {
                    None
                };

                let i0 = (view_left / minor_world).floor() as i32 - 1;
                let i1 = (view_right / minor_world).ceil() as i32 + 1;
                for i in i0..=i1 {
                    if let Some(f) = major_factor {
                        if i.rem_euclid(f) == 0 {
                            continue;
                        }
                    }
                    let x = i as f32 * minor_world;
                    self.grid_vertices.push(Vec3::new(x, view_top, 0.0));
                    self.grid_vertices.push(Vec3::new(x, view_bottom, 0.0));
                }

                let j0 = (view_top / minor_world).floor() as i32 - 1;
                let j1 = (view_bottom / minor_world).ceil() as i32 + 1;
                for j in j0..=j1 {
                    if let Some(f) = major_factor {
                        if j.rem_euclid(f) == 0 {
                            continue;
                        }
                    }
                    let y = j as f32 * minor_world;
                    self.grid_vertices.push(Vec3::new(view_left, y, 0.0));
                    self.grid_vertices.push(Vec3::new(view_right, y, 0.0));
                }

                draw_lines(
                    backend,
                    &self.grid_vertices,
                    editor.palette.view2d_grid_minor,
                );
            }

            // World origin axes under map lines.
            self.grid_vertices.clear();
            self.grid_vertices.push(Vec3::new(0.0, view_top, 0.0));
            self.grid_vertices.push(Vec3::new(0.0, view_bottom, 0.0));
            draw_lines(backend, &self.grid_vertices, editor.palette.view2d_axis_y);

            self.grid_vertices.clear();
            self.grid_vertices.push(Vec3::new(view_left, 0.0, 0.0));
            self.grid_vertices.push(Vec3::new(view_right, 0.0, 0.0));
            draw_lines(backend, &self.grid_vertices, editor.palette.view2d_axis_x);

            // brush geometry
            let map_ptr = editor
                .map
                .as_ref()
                .map(|m| (m as *const crate::map::Map) as usize)
                .unwrap_or(0);
            let map_generation = editor.map.as_ref().map(|m| m.generation).unwrap_or(0u64);
            let map_revision = editor.map_revision;
            let map_present = editor.map.is_some();
            let view_config_rev = editor.view_config_rev;

            if !map_present {
                self.grid_vertices.clear();
                self.cache = None;
            }

            let mut rebuild_view2d = false;
            match self.cache {
                None => rebuild_view2d = map_present,
                Some(cache) => {
                    if !map_present {
                        rebuild_view2d = true;
                    } else if cache.axis != axis
                        || cache.map_present != map_present
                        || cache.map_ptr != map_ptr
                        || cache.map_generation != map_generation
                        || cache.map_revision != map_revision
                        || (cache.zoom - zoom).abs() > 1.0e-6
                        || cache.view_config_rev != view_config_rev
                    {
                        rebuild_view2d = true;
                    } else {
                        // Reuse cached cull set while the current view stays within it.
                        if view_left < cache.cull_left
                            || view_right > cache.cull_right
                            || view_top < cache.cull_top
                            || view_bottom > cache.cull_bottom
                        {
                            rebuild_view2d = true;
                        }
                    }
                }
            }

            if rebuild_view2d {
                self.line_vertices.clear();
                self.entity_line_batches.clear();

                let margin_x = (view_right - view_left).abs() * 0.50;
                let margin_y = (view_bottom - view_top).abs() * 0.50;
                let cull_left = view_left - margin_x;
                let cull_right = view_right + margin_x;
                let cull_top = view_top - margin_y;
                let cull_bottom = view_bottom + margin_y;

                self.cache = Some(View2dCache {
                    axis,
                    map_present,
                    map_ptr,
                    map_generation,
                    map_revision,
                    zoom,
                    cull_left,
                    cull_right,
                    cull_top,
                    cull_bottom,
                    view_config_rev,
                });
            }

            if rebuild_view2d {
                if let Some(cache) = self.cache {
                    let view_min_x = cache.cull_left.min(cache.cull_right);
                    let view_max_x = cache.cull_left.max(cache.cull_right);
                    let view_min_y = cache.cull_top.min(cache.cull_bottom);
                    let view_max_y = cache.cull_top.max(cache.cull_bottom);

                    if let Some(map) = &mut editor.map {
                        let project_line_batch = |lines: Vec<Vec3>| -> Vec<Vec3> {
                            lines
                                .into_iter()
                                .map(|p| {
                                    let p2 = project_to_2d(p, axis);
                                    Vec3::new(p2[0], p2[1], 0.0)
                                })
                                .collect()
                        };

                        for entity in &mut map.entities {
                            let draw_entity_visual = entity.classname != "worldspawn";

                            let origin = entity
                                .properties
                                .get("origin")
                                .map(|s| core_util::origin_to_vec3(s))
                                .unwrap_or(Vec3::ZERO);
                            let angles = entity
                                .properties
                                .get("angles")
                                .and_then(|s| core_util::vec3_from_whitespace_triplet(s));
                            let model_rot = angles.map(core_util::entity_angles_to_quat);

                            // Resolve style early using the actual loaded model
                            // state so Hidden entities are skipped before any
                            // model loading is attempted.
                            let has_model_prop = entity.properties.get("model").is_some();
                            let has_model = entity.model.is_some();
                            let style =
                                editor.entity_drawing.resolve(&entity.classname, has_model);

                            if draw_entity_visual
                                && matches!(style.kind, EntityDrawKind::Hidden)
                            {
                                continue;
                            }

                            // Load model (skip for previously-failed names)
                            if has_model_prop {
                                if let Some(model_name) = entity.properties.get("model") {
                                    let expected_name = {
                                        let normalized = normalize_material_name(model_name);
                                        if normalized.starts_with("xmodel/") {
                                            normalized
                                        } else {
                                            format!("xmodel/{normalized}")
                                        }
                                    };
                                    let needs_load = entity
                                        .model
                                        .as_ref()
                                        .map(|model| model.name != expected_name)
                                        .unwrap_or(true);
                                    if needs_load
                                        && !editor.failed_models.contains(&expected_name)
                                    {
                                        if let Some(model_asset_db) =
                                            editor.model_asset_db.as_mut()
                                        {
                                            match XModel::load(
                                                model_asset_db,
                                                model_name,
                                                editor.shader_db.as_ref(),
                                            ) {
                                                Ok(m) => entity.model = Some(m),
                                                Err(e) => {
                                                    eprintln!(
                                                        "Failed load model {model_name}: {e}"
                                                    );
                                                    editor.failed_models.insert(expected_name);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Recompute has_model after load attempt
                            let has_model = entity.model.is_some();
                            let style =
                                editor.entity_drawing.resolve(&entity.classname, has_model);

                            if draw_entity_visual {
                                let pivot = origin;

                                if let Some(model) = entity.model.as_ref() {
                                    let model_origin = pivot + model.origin;
                                    let mut lines = project_line_batch(
                                        crate::core_util::model_bounds_line_vertices(
                                            model_origin,
                                            model.mins,
                                            model.maxs,
                                            model_rot,
                                        ),
                                    );
                                    // Vertical center guide line (top to bottom)
                                    let (min_b, max_b) = crate::core_util::
                                        model_bounds_aabb(
                                            model_origin,
                                            model.mins,
                                            model.maxs,
                                            model_rot,
                                        );
                                    let cx = (min_b.x + max_b.x) * 0.5;
                                    let cy = (min_b.y + max_b.y) * 0.5;
                                    lines.extend(project_line_batch(vec![
                                        Vec3::new(cx, cy, min_b.z),
                                        Vec3::new(cx, cy, max_b.z),
                                    ]));
                                    if style.show_arrow {
                                        if let Some(angles) = angles {
                                            let forward =
                                                entity_angles_forward(angles);
                                            lines.extend(project_line_batch(
                                                arrow_line_vertices(
                                                    core_util::model_front_center(
                                                        model_origin,
                                                        model.mins,
                                                        model.maxs,
                                                        forward,
                                                    ),
                                                    forward,
                                                    core_util::resolved_arrow_length(
                                                        &style,
                                                        model.radius,
                                                    ),
                                                    Vec3::Z,
                                                ),
                                            ));
                                        }
                                    }
                                    self.entity_line_batches
                                        .push((style.color, lines));
                                    if style.show_origin_box {
                                        let origin_center =
                                            model_origin;
                                        let origin_lines = project_line_batch(
                                            box_line_vertices(
                                                origin_center,
                                                Vec3::from_array(
                                                    style.origin_box_size,
                                                ),
                                                None,
                                            ),
                                        );
                                        self.entity_line_batches
                                            .push(([1.0, 1.0, 1.0, 1.0], origin_lines));
                                    }
                                    if editor.config.view.show.models {
                                        let wire_lines = project_line_batch(
                                            crate::xmodel::model_wireframe_lines(
                                                model,
                                                model_origin,
                                                model_rot,
                                            ),
                                        );
                                        self.entity_line_batches
                                            .push((style.color, wire_lines));
                                    }
                                } else {
                                    let size = Vec3::from_array(style.size);
                                    let mut lines = project_line_batch(
                                        match style.anchor {
                                            EntityDrawAnchor::Base => {
                                                crate::core_util::
                                                    box_line_vertices_from_base(
                                                        pivot,
                                                        size,
                                                        model_rot,
                                                    )
                                            }
                                            EntityDrawAnchor::Center => {
                                                crate::core_util::box_line_vertices(
                                                    pivot,
                                                    size,
                                                    model_rot,
                                                )
                                            }
                                        },
                                    );
                                    if style.show_arrow {
                                        if let Some(angles) = angles {
                                            lines.extend(project_line_batch(
                                                arrow_line_vertices(
                                                    core_util::proxy_box_center(
                                                        style.anchor.clone(),
                                                        pivot,
                                                        size,
                                                    ),
                                                    entity_angles_forward(angles),
                                                    core_util::resolved_arrow_length(
                                                        &style,
                                                        size.length(),
                                                    ),
                                                    Vec3::Z,
                                                ),
                                            ));
                                        }
                                    }
                                    self.entity_line_batches
                                        .push((style.color, lines));
                                }
                            }

                            for brush in &mut entity.brushes {
                                if brush.is_clip() && !editor.config.view.show.clip_brushes {
                                    continue;
                                }
                                if brush.is_portal() && !editor.config.view.show.portal_brushes {
                                    continue;
                                }
                                if brush.is_hint() && !editor.config.view.show.hint_brushes {
                                    continue;
                                }
                                match &mut brush.content {
                                    BrushContent::Convex(_) => {
                                        if !editor.config.view.show.convex {
                                            continue;
                                        }
                                        let Some((aabb, polys)) = brush.get_polygons_and_aabb()
                                        else {
                                            continue;
                                        };

                                        // Coarse frustum cull by brush AABB before iterating faces/edges.
                                        let (a_min_x, a_max_x, a_min_y, a_max_y) = match axis {
                                            Ortho::XY => (
                                                aabb.min.x as f32,
                                                aabb.max.x as f32,
                                                -(aabb.max.y as f32),
                                                -(aabb.min.y as f32),
                                            ),
                                            Ortho::XZ => (
                                                aabb.min.x as f32,
                                                aabb.max.x as f32,
                                                -(aabb.max.z as f32),
                                                -(aabb.min.z as f32),
                                            ),
                                            Ortho::YZ => (
                                                aabb.min.y as f32,
                                                aabb.max.y as f32,
                                                -(aabb.max.z as f32),
                                                -(aabb.min.z as f32),
                                            ),
                                        };
                                        if a_max_x < view_min_x
                                            || a_min_x > view_max_x
                                            || a_max_y < view_min_y
                                            || a_min_y > view_max_y
                                        {
                                            continue;
                                        }

                                        for (verts, _) in polys {
                                            if verts.len() < 2 {
                                                continue;
                                            }

                                            for i in 0..verts.len() {
                                                let a = verts[i];
                                                let b = verts[(i + 1) % verts.len()];

                                                // Frustum cull in projected 2D space.
                                                let pa = project_to_2d(a, axis);
                                                let pb = project_to_2d(b, axis);
                                                let seg_min_x = pa[0].min(pb[0]);
                                                let seg_max_x = pa[0].max(pb[0]);
                                                let seg_min_y = pa[1].min(pb[1]);
                                                let seg_max_y = pa[1].max(pb[1]);
                                                if seg_max_x < view_min_x
                                                    || seg_min_x > view_max_x
                                                    || seg_max_y < view_min_y
                                                    || seg_min_y > view_max_y
                                                {
                                                    continue;
                                                }

                                                self.line_vertices
                                                    .push(Vec3::new(pa[0], pa[1], 0.0));
                                                self.line_vertices
                                                    .push(Vec3::new(pb[0], pb[1], 0.0));
                                            }
                                        }
                                    }
                                    BrushContent::Patch(patch) => {
                                        if !editor.config.view.show.patches {
                                            continue;
                                        }
                                        let Some((mesh, patch_aabb, edges)) =
                                            patch.get_mesh_aabb_wire()
                                        else {
                                            continue;
                                        };
                                        brush.aabb = patch_aabb.clone();
                                        let positions = mesh.positions.as_slice();

                                        let (a_min_x, a_max_x, a_min_y, a_max_y) = match axis {
                                            Ortho::XY => (
                                                patch_aabb.min.x as f32,
                                                patch_aabb.max.x as f32,
                                                -(patch_aabb.max.y as f32),
                                                -(patch_aabb.min.y as f32),
                                            ),
                                            Ortho::XZ => (
                                                patch_aabb.min.x as f32,
                                                patch_aabb.max.x as f32,
                                                -(patch_aabb.max.z as f32),
                                                -(patch_aabb.min.z as f32),
                                            ),
                                            Ortho::YZ => (
                                                patch_aabb.min.y as f32,
                                                patch_aabb.max.y as f32,
                                                -(patch_aabb.max.z as f32),
                                                -(patch_aabb.min.z as f32),
                                            ),
                                        };
                                        if a_max_x < view_min_x
                                            || a_min_x > view_max_x
                                            || a_max_y < view_min_y
                                            || a_min_y > view_max_y
                                        {
                                            continue;
                                        }

                                        if positions.len() < 2 || edges.is_empty() {
                                            continue;
                                        }

                                        for &(a, b) in edges {
                                            let ia = a as usize;
                                            let ib = b as usize;
                                            if ia >= positions.len() || ib >= positions.len() {
                                                continue;
                                            }
                                            let pa = core_util::project_to_2d(positions[ia], axis);
                                            let pb = core_util::project_to_2d(positions[ib], axis);

                                            let seg_min_x = pa[0].min(pb[0]);
                                            let seg_max_x = pa[0].max(pb[0]);
                                            let seg_min_y = pa[1].min(pb[1]);
                                            let seg_max_y = pa[1].max(pb[1]);
                                            if seg_max_x < view_min_x
                                                || seg_min_x > view_max_x
                                                || seg_max_y < view_min_y
                                                || seg_min_y > view_max_y
                                            {
                                                continue;
                                            }

                                            self.line_vertices.push(Vec3::new(pa[0], pa[1], 0.0));
                                            self.line_vertices.push(Vec3::new(pb[0], pb[1], 0.0));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !self.line_vertices.is_empty() {
                draw_lines(backend, &self.line_vertices, editor.palette.view2d_geometry);
            }

            for (color, lines) in &self.entity_line_batches {
                if lines.is_empty() {
                    continue;
                }
                draw_lines(backend, lines, *color);
            }

            self.selected_vertices.clear();
            if !editor.edit_faces
                && !editor.edit_edges
                && !editor.edit_vertices
                && (!editor.selected_brushes.is_empty() || !editor.selected_entities.is_empty())
            {
                let view_min_x = view_left.min(view_right);
                let view_max_x = view_left.max(view_right);
                let view_min_y = view_top.min(view_bottom);
                let view_max_y = view_top.max(view_bottom);
                let view_dir = match axis {
                    Ortho::XY => glam::Vec3::new(0.0, 0.0, -1.0),
                    Ortho::XZ => glam::Vec3::new(0.0, -1.0, 0.0),
                    Ortho::YZ => glam::Vec3::new(-1.0, 0.0, 0.0),
                };

                let preview_drag_mode = editor.view2d.drag_mode;
                let preview_stretch_mode = editor.stretch_mode;
                let preview_move_offset = editor.view2d.move_offset;
                let preview_stretch = editor.view2d.stretch_preview_xform();
                let preview_rotate = editor.view2d.rotate_preview_xform();
                let preview_face = editor.view2d.face_stretch_preview();
                let preview_point = |p: Vec3| -> Vec3 {
                    match preview_drag_mode {
                        DragMode::MoveSelection | DragMode::MoveVertices | DragMode::MoveEdges => {
                            p + preview_move_offset
                        }
                        DragMode::StretchSelection => {
                            if preview_stretch_mode == StretchMode::Scale {
                                preview_stretch
                                    .map(|x: AffineScale| x.apply_point(p))
                                    .unwrap_or(p)
                            } else {
                                p
                            }
                        }
                        DragMode::NewBrush | DragMode::RectangularSelection => p,
                        DragMode::RotateSelection => {
                            preview_rotate.map(|r| r.apply_point(p)).unwrap_or(p)
                        }
                    }
                };

                for (entity_index, brush_index) in &editor.selected_brushes {
                    if let Some(map) = &mut editor.map {
                        if let Some(entity) = map.entities.get_mut(*entity_index) {
                            if let Some(brush) = entity.brushes.get_mut(*brush_index) {
                                if matches!(&brush.content, BrushContent::Convex(_)) {
                                    if preview_drag_mode == DragMode::StretchSelection
                                        && preview_stretch_mode == StretchMode::Resize
                                    {
                                        if let Some((faces, delta)) = preview_face {
                                            if let Some(polys) =
                                                crate::editing::preview_convex_face_stretch_polys(
                                                    &*brush,
                                                    faces,
                                                    delta,
                                                    editor.config.view.grid_minor_step as i32,
                                                )
                                            {
                                                for (positions, _) in polys {
                                                    if positions.len() < 2 {
                                                        continue;
                                                    }
                                                    for i in 0..positions.len() {
                                                        let a = positions[i];
                                                        let b =
                                                            positions[(i + 1) % positions.len()];
                                                        let pa = core_util::project_to_2d(a, axis);
                                                        let pb = core_util::project_to_2d(b, axis);

                                                        let seg_min_x = pa[0].min(pb[0]);
                                                        let seg_max_x = pa[0].max(pb[0]);
                                                        let seg_min_y = pa[1].min(pb[1]);
                                                        let seg_max_y = pa[1].max(pb[1]);
                                                        if seg_max_x < view_min_x
                                                            || seg_min_x > view_max_x
                                                            || seg_max_y < view_min_y
                                                            || seg_min_y > view_max_y
                                                        {
                                                            continue;
                                                        }

                                                        self.selected_vertices
                                                            .push(Vec3::new(pa[0], pa[1], 0.0));
                                                        self.selected_vertices
                                                            .push(Vec3::new(pb[0], pb[1], 0.0));
                                                    }
                                                }
                                                continue;
                                            }
                                        }
                                    }

                                    if let Some((_aabb, polys)) = brush.get_polygons_and_aabb() {
                                        for (positions, _) in polys {
                                            if positions.len() < 2 {
                                                continue;
                                            }
                                            if positions.len() >= 3 {
                                                let p0 = preview_point(positions[0]);
                                                let p1 = preview_point(positions[1]);
                                                let p2 = preview_point(positions[2]);
                                                let n = (p1 - p0).cross(p2 - p0);
                                                let n_len = n.length();
                                                if n_len.is_finite() && n_len > 1e-6 {
                                                    let dot = (n / n_len).dot(view_dir);
                                                    if dot > 1e-4 {
                                                        continue;
                                                    }
                                                }
                                            }
                                            for i in 0..positions.len() {
                                                let a = positions[i];
                                                let b = positions[(i + 1) % positions.len()];
                                                let pa = core_util::project_to_2d(
                                                    preview_point(a),
                                                    axis,
                                                );
                                                let pb = core_util::project_to_2d(
                                                    preview_point(b),
                                                    axis,
                                                );

                                                let seg_min_x = pa[0].min(pb[0]);
                                                let seg_max_x = pa[0].max(pb[0]);
                                                let seg_min_y = pa[1].min(pb[1]);
                                                let seg_max_y = pa[1].max(pb[1]);
                                                if seg_max_x < view_min_x
                                                    || seg_min_x > view_max_x
                                                    || seg_max_y < view_min_y
                                                    || seg_min_y > view_max_y
                                                {
                                                    continue;
                                                }

                                                self.selected_vertices
                                                    .push(Vec3::new(pa[0], pa[1], 0.0));
                                                self.selected_vertices
                                                    .push(Vec3::new(pb[0], pb[1], 0.0));
                                            }
                                        }
                                    }
                                } else if let BrushContent::Patch(patch) = &mut brush.content {
                                    let Some((mesh, patch_aabb, edges)) =
                                        patch.get_mesh_aabb_wire()
                                    else {
                                        continue;
                                    };
                                    brush.aabb = patch_aabb.clone();

                                    let (a_min_x, a_max_x, a_min_y, a_max_y) = match axis {
                                        Ortho::XY => (
                                            patch_aabb.min.x as f32,
                                            patch_aabb.max.x as f32,
                                            -(patch_aabb.max.y as f32),
                                            -(patch_aabb.min.y as f32),
                                        ),
                                        Ortho::XZ => (
                                            patch_aabb.min.x as f32,
                                            patch_aabb.max.x as f32,
                                            -(patch_aabb.max.z as f32),
                                            -(patch_aabb.min.z as f32),
                                        ),
                                        Ortho::YZ => (
                                            patch_aabb.min.y as f32,
                                            patch_aabb.max.y as f32,
                                            -(patch_aabb.max.z as f32),
                                            -(patch_aabb.min.z as f32),
                                        ),
                                    };
                                    if a_max_x < view_min_x
                                        || a_min_x > view_max_x
                                        || a_max_y < view_min_y
                                        || a_min_y > view_max_y
                                    {
                                        continue;
                                    }

                                    let positions = mesh.positions.as_slice();
                                    for &(a, b) in edges {
                                        let ia = a as usize;
                                        let ib = b as usize;
                                        if ia >= positions.len() || ib >= positions.len() {
                                            continue;
                                        }
                                        let pa = core_util::project_to_2d(
                                            preview_point(positions[ia]),
                                            axis,
                                        );
                                        let pb = core_util::project_to_2d(
                                            preview_point(positions[ib]),
                                            axis,
                                        );

                                        let seg_min_x = pa[0].min(pb[0]);
                                        let seg_max_x = pa[0].max(pb[0]);
                                        let seg_min_y = pa[1].min(pb[1]);
                                        let seg_max_y = pa[1].max(pb[1]);
                                        if seg_max_x < view_min_x
                                            || seg_min_x > view_max_x
                                            || seg_max_y < view_min_y
                                            || seg_min_y > view_max_y
                                        {
                                            continue;
                                        }

                                        self.selected_vertices.push(Vec3::new(pa[0], pa[1], 0.0));
                                        self.selected_vertices.push(Vec3::new(pb[0], pb[1], 0.0));
                                    }
                                }
                            }
                        }
                    }
                }

                for entity_index in &editor.selected_entities {
                    if let Some(map) = &editor.map {
                        if let Some(entity) = map.entities.get(*entity_index) {
                            if entity.classname == "worldspawn" {
                                continue;
                            }
                            let style = editor
                                .entity_drawing
                                .resolve(&entity.classname, entity.model.is_some());
let origin = entity
                                .properties
                                .get("origin")
                                .map(|s| core_util::origin_to_vec3(s))
                                .unwrap_or(Vec3::ZERO);

                            // If entity has a model, render model bounds box for selection (cheaper
                            // than full wireframe). Full wireframe is rendered in the main pass
                            // when view.models is enabled.
                            if let Some(model) = &entity.model {
                                let model_origin = origin + model.origin;
                                let angles = entity
                                    .properties
                                    .get("angles")
                                    .and_then(|s| core_util::vec3_from_whitespace_triplet(s));
                                let model_rot = angles.map(core_util::entity_angles_to_quat);

                                let lines = crate::core_util::model_bounds_line_vertices(
                                    model_origin,
                                    model.mins,
                                    model.maxs,
                                    model_rot,
                                );
                                for chunk in lines.chunks_exact(2) {
                                    let a = chunk[0];
                                    let b = chunk[1];
                                    let pa = core_util::project_to_2d(preview_point(a), axis);
                                    let pb = core_util::project_to_2d(preview_point(b), axis);

                                    let seg_min_x = pa[0].min(pb[0]);
                                    let seg_max_x = pa[0].max(pb[0]);
                                    let seg_min_y = pa[1].min(pb[1]);
                                    let seg_max_y = pa[1].max(pb[1]);
                                    if seg_max_x < view_min_x
                                        || seg_min_x > view_max_x
                                        || seg_max_y < view_min_y
                                        || seg_min_y > view_max_y
                                    {
                                        continue;
                                    }

                                    self.selected_vertices.push(Vec3::new(pa[0], pa[1], 0.0));
                                    self.selected_vertices.push(Vec3::new(pb[0], pb[1], 0.0));
                                }
                            } else {
                                // Fall back to proxy box for entities without models
                                let size = Vec3::from_array(style.size);
                                let angles = entity
                                    .properties
                                    .get("angles")
                                    .and_then(|s| core_util::vec3_from_whitespace_triplet(s));
                                let rot = angles.map(core_util::entity_angles_to_quat);
                                let lines = match style.anchor {
                                    crate::editor::config::EntityDrawAnchor::Base => {
                                        // base = origin (box sits on top, offset Z internally)
                                        crate::core_util::box_line_vertices_from_base(
                                            origin, size, rot,
                                        )
                                    }
                                    crate::editor::config::EntityDrawAnchor::Center => {
                                        // center = origin
                                        crate::core_util::box_line_vertices(origin, size, rot)
                                    }
                                };
                                for chunk in lines.chunks_exact(2) {
                                    let a = chunk[0];
                                    let b = chunk[1];
                                    let pa = core_util::project_to_2d(preview_point(a), axis);
                                    let pb = core_util::project_to_2d(preview_point(b), axis);

                                    let seg_min_x = pa[0].min(pb[0]);
                                    let seg_max_x = pa[0].max(pb[0]);
                                    let seg_min_y = pa[1].min(pb[1]);
                                    let seg_max_y = pa[1].max(pb[1]);
                                    if seg_max_x < view_min_x
                                        || seg_min_x > view_max_x
                                        || seg_max_y < view_min_y
                                        || seg_min_y > view_max_y
                                    {
                                        continue;
                                    }

                                    self.selected_vertices.push(Vec3::new(pa[0], pa[1], 0.0));
                                    self.selected_vertices.push(Vec3::new(pb[0], pb[1], 0.0));
                                }
                            }
                        }
                    }
                }
            }

            if !self.selected_vertices.is_empty() {
                draw_lines(backend, &self.selected_vertices, editor.selection_rgba);
            }

            if editor.edit_edges && !editor.selected_edges.is_empty() {
                self.selected_vertices.clear();
                let move_offset = if editor.view2d.drag_mode == DragMode::MoveSelection {
                    editor.view2d.move_offset
                } else {
                    Vec3::ZERO
                };

                if let Some(map) = &mut editor.map {
                    for sel in &editor.selected_edges {
                        let Some(entity) = map.entities.get_mut(sel.entity_idx) else {
                            continue;
                        };
                        let Some(brush) = entity.brushes.get_mut(sel.brush_idx) else {
                            continue;
                        };
                        if !matches!(brush.content, BrushContent::Convex(_)) {
                            continue;
                        }
                        let Some((_aabb, polys)) = brush.get_polygons_and_aabb() else {
                            continue;
                        };
                        let Some((a, b)) =
                            crate::core_util::shared_edge_points(polys, sel.face_a_idx, sel.face_b_idx)
                        else {
                            continue;
                        };
                        let pa = core_util::project_to_2d(a + move_offset, axis);
                        let pb = core_util::project_to_2d(b + move_offset, axis);
                        self.selected_vertices.push(Vec3::new(pa[0], pa[1], 0.0));
                        self.selected_vertices.push(Vec3::new(pb[0], pb[1], 0.0));
                    }
                }

                if !self.selected_vertices.is_empty() {
                    draw_lines(backend, &self.selected_vertices, editor.selection_rgba);
                }
            }
            // Draw patch control vertices when in vertex editing mode
            self.control_vertices.clear();
            self.control_selected_vertices.clear();
            if editor.edit_vertices {
                let move_offset = if editor.view2d.drag_mode == DragMode::MoveVertices {
                    editor.view2d.move_offset
                } else {
                    Vec3::ZERO
                };

                if let Some(map) = &editor.map {
                    for (entity_idx, entity) in map.entities.iter().enumerate() {
                        for (brush_idx, brush) in entity.brushes.iter().enumerate() {
                            if let BrushContent::Patch(patch) = &brush.content {
                                for (row_idx, row) in patch.vertices.iter().enumerate() {
                                    for (col_idx, vtx) in row.iter().enumerate() {
                                        let is_selected =
                                            editor.selected_patch_vertices.iter().any(|sel| {
                                                sel.entity_idx == entity_idx
                                                    && sel.brush_idx == brush_idx
                                                    && sel.row == row_idx
                                                    && sel.col == col_idx
                                            });

                                        let pos_3d = if is_selected {
                                            vtx.position + move_offset
                                        } else {
                                            vtx.position
                                        };

                                        let p2 = core_util::project_to_2d(pos_3d, axis);
                                        let pos = Vec3::new(p2[0], p2[1], 0.0);

                                        if is_selected {
                                            self.control_selected_vertices.push(pos);
                                        } else {
                                            self.control_vertices.push(pos);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !self.control_vertices.is_empty() {
                let size = 3.0;
                let mut square_vertices = Vec::with_capacity(self.control_vertices.len() * 8);
                for v in &self.control_vertices {
                    let x = v.x;
                    let y = v.y;
                    square_vertices.push(Vec3::new(x - size, y - size, 0.0));
                    square_vertices.push(Vec3::new(x + size, y - size, 0.0));
                    square_vertices.push(Vec3::new(x + size, y - size, 0.0));
                    square_vertices.push(Vec3::new(x + size, y + size, 0.0));
                    square_vertices.push(Vec3::new(x + size, y + size, 0.0));
                    square_vertices.push(Vec3::new(x - size, y + size, 0.0));
                    square_vertices.push(Vec3::new(x - size, y + size, 0.0));
                    square_vertices.push(Vec3::new(x - size, y - size, 0.0));
                }
                draw_lines(backend, &square_vertices, [0.8, 0.8, 0.8, 1.0]);
            }

            if !self.control_selected_vertices.is_empty() {
                let size = 4.0;
                let mut square_vertices =
                    Vec::with_capacity(self.control_selected_vertices.len() * 8);
                for v in &self.control_selected_vertices {
                    let x = v.x;
                    let y = v.y;
                    square_vertices.push(Vec3::new(x - size, y - size, 0.0));
                    square_vertices.push(Vec3::new(x + size, y - size, 0.0));
                    square_vertices.push(Vec3::new(x + size, y - size, 0.0));
                    square_vertices.push(Vec3::new(x + size, y + size, 0.0));
                    square_vertices.push(Vec3::new(x + size, y + size, 0.0));
                    square_vertices.push(Vec3::new(x - size, y + size, 0.0));
                    square_vertices.push(Vec3::new(x - size, y + size, 0.0));
                    square_vertices.push(Vec3::new(x - size, y - size, 0.0));
                }
                draw_lines(backend, &square_vertices, editor.selection_rgba);
            }

            backend.gl.use_program(None);
            backend.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }
}


