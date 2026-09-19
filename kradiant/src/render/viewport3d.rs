//! 3D Viewport

use std::collections::{HashMap, HashSet};

use crate::editing;
use crate::editor::EditorState;
use crate::editor::config::{EntityDrawAnchor, EntityDrawKind, RenderMode};
use crate::editor::viewport::DragMode;
use crate::render::RenderBackend;
//use crate::ui;
use crate::assets::normalize_material_name;
use crate::core_util;
use crate::geometry::tessellate_patch;
use crate::map::BrushContent;
use crate::texmap::FaceUvMapper;
use crate::xmodel::XModel;
use glam::{Quat, Vec3};

use crate::editor::selection::EdgeSelection;

/// Compute a stable hash of the edge selection for caching.
fn edge_selection_hash(edges: &[EdgeSelection]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    // Sort for stable ordering
    let mut sorted: Vec<_> = edges.iter().collect();
    sorted.sort_by_key(|e| (e.entity_idx, e.brush_idx, e.face_a_idx, e.face_b_idx));
    for e in sorted {
        e.entity_idx.hash(&mut hasher);
        e.brush_idx.hash(&mut hasher);
        e.face_a_idx.hash(&mut hasher);
        e.face_b_idx.hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View3dCache {
    map_present: bool,
    map_ptr: usize,
    map_generation: u64,
    map_revision: u64,
    map_load_count: u64,
    view_config_rev: u64,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LitVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TexVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

fn solid_box_lit_vertices_from_base<F: Fn(Vec3) -> Vec3>(
    base: Vec3,
    size: Vec3,
    rot: Option<Quat>,
    preview_fn: F,
) -> Vec<LitVertex> {
    let min = base;
    let max = base + size;
    let center = base + size * 0.5;
    let corners = [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ];
    let corners: [Vec3; 8] = rot
        .map(|r| {
            corners.map(|c| {
                let o = c - center;
                center + r * o
            })
        })
        .unwrap_or(corners);
    const FACES: [([usize; 6], Vec3); 6] = [
        ([0, 2, 3, 0, 3, 1], Vec3::NEG_Z),
        ([4, 5, 7, 4, 7, 6], Vec3::Z),
        ([0, 1, 5, 0, 5, 4], Vec3::NEG_Y),
        ([2, 6, 7, 2, 7, 3], Vec3::Y),
        ([0, 4, 6, 0, 6, 2], Vec3::NEG_X),
        ([1, 3, 7, 1, 7, 5], Vec3::X),
    ];

    let mut out = Vec::with_capacity(36);
    for (idxs, normal) in FACES {
        let normal = rot.map(|r| r * normal).unwrap_or(normal);
        for i in idxs {
            out.push(LitVertex {
                pos: preview_fn(corners[i]).into(),
                normal: normal.into(),
            });
        }
    }
    out
}

fn entity_solid_box_base(origin: Vec3, size: Vec3, anchor: EntityDrawAnchor) -> Vec3 {
    match anchor {
        EntityDrawAnchor::Base => origin - Vec3::new(size.x * 0.5, size.y * 0.5, 0.0),
        EntityDrawAnchor::Center => origin - size * 0.5,
    }
}

pub struct Viewport3D {
    line_vertices: Vec<Vec3>,
    entity_line_batches: Vec<([f32; 4], Vec<Vec3>)>,
    entity_solid_batches: Vec<([f32; 4], Vec<LitVertex>)>,
    tri_vertices: Vec<LitVertex>,
    tri_vertices_tex: Vec<(String, Vec<TexVertex>)>,
    tri_vertices_selected: Vec<LitVertex>,
    control_vertices: Vec<Vec3>,
    control_selected_vertices: Vec<Vec3>,
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    rbo: glow::Renderbuffer, // depth
    fbo_size: [u32; 2],
    cache: Option<View3dCache>,
    last_selection_hash: u64,
    last_preview_hash: u64,
    /// Cache for edge_move_clamp_factor: keyed by (map_ptr, map_revision, move_offset, selection_hash, map_generation)
    last_edge_clamp_offset: Vec3,
    last_edge_clamp_selection_hash: u64,
    last_edge_clamp_map_generation: u64,
    last_edge_clamp_map_ptr: usize,
    last_edge_clamp_map_revision: u64,
    last_edge_clamp_factor: f32,
    /// Cache for dimmed edges: keyed by (map_ptr, map_revision, map_generation, selection_hash, show_config_hash)
    last_dim_edges_map_generation: u64,
    last_dim_edges_selection_hash: u64,
    last_dim_edges_config_hash: u64,
    last_dim_edges_map_ptr: usize,
    last_dim_edges_map_revision: u64,
    cached_dim_edges: Vec<Vec3>,
}

impl Viewport3D {
    pub fn new(
        line_vertices: Vec<Vec3>,
        tri_vertices: Vec<LitVertex>,
        tri_vertices_tex: Vec<(String, Vec<TexVertex>)>,
        tri_vertices_selected: Vec<LitVertex>,
        control_vertices: Vec<Vec3>,
        control_selected_vertices: Vec<Vec3>,
        fbo: glow::Framebuffer,
        fbo_size: [u32; 2],
        tex: glow::Texture,
        rbo: glow::Renderbuffer,
        cache: Option<View3dCache>,
    ) -> Self {
        Self {
            line_vertices,
            entity_line_batches: Vec::new(),
            entity_solid_batches: Vec::new(),
            tri_vertices,
            tri_vertices_tex,
            tri_vertices_selected,
            control_vertices,
            control_selected_vertices,
            fbo,
            fbo_size,
            tex,
            rbo,
            cache,
            last_selection_hash: 0,
            last_preview_hash: 0,
            last_edge_clamp_offset: Vec3::ZERO,
            last_edge_clamp_selection_hash: 0,
            last_edge_clamp_map_generation: 0,
            last_edge_clamp_map_ptr: 0,
            last_edge_clamp_map_revision: 0,
            last_edge_clamp_factor: 1.0,
            last_dim_edges_map_generation: 0,
            last_dim_edges_selection_hash: 0,
            last_dim_edges_config_hash: 0,
            last_dim_edges_map_ptr: 0,
            last_dim_edges_map_revision: 0,
            cached_dim_edges: Vec::new(),
        }
    }
    pub fn render(&mut self, backend: &mut RenderBackend<'_>, editor: &mut EditorState) {
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
            let map_load_count = editor.map_load_count;
            let view_config_rev = editor.view_config_rev;

            let need_rebuild = match self.cache {
                Some(c) => {
                    c.map_present != map_present
                        || c.map_ptr != map_ptr
                        || c.map_generation != map_generation
                        || c.map_revision != map_revision
                        || c.view_config_rev != view_config_rev
                }
                None => true,
            };
            // Only reload textures when map changes, not when view config changes
            let tex_reload = match self.cache {
                Some(c) => c.map_ptr != map_ptr || c.map_load_count != map_load_count,
                None => true,
            };

            let mut bounds_min = Vec3::splat(f32::INFINITY);
            let mut bounds_max = Vec3::splat(f32::NEG_INFINITY);

            if need_rebuild {
                self.line_vertices.clear();
                self.entity_line_batches.clear();
                self.entity_solid_batches.clear();
                self.tri_vertices.clear();
                self.tri_vertices_tex.clear();

                if let Some(map) = editor.map.as_mut() {
                    if tex_reload {
                        editor.tex_registry.clear();
                    }
                    // Request all textures used by the map for 3D rendering. This leverages
                    // Map::collect_used_materials() which de-duplicates and normalizes names.
                    let shader_db = editor.shader_db.as_ref();
                    for mat in map.collect_used_materials(shader_db) {
                        editor.tex_registry.request(mat);
                    }

                    // Cheap reserve to reduce reallocs on medium maps
                    self.line_vertices.reserve(32_768);
                    let mut tex_batches: HashMap<String, Vec<TexVertex>> = HashMap::new();

                    for ent in &mut map.entities {
                        let origin = ent
                            .properties
                            .get("origin")
                            .map(|s| core_util::origin_to_vec3(s))
                            .unwrap_or({
                                // println!("entity {} has no origin: {:?}", ent.classname, &ent.properties);
                                Vec3::ZERO
                            });
                        let angles = ent
                            .properties
                            .get("angles")
                            .and_then(|s| core_util::vec3_from_whitespace_triplet(s));
                        let model_rot = angles.map(core_util::entity_angles_to_quat);
                        let pivot = origin;

                        // Resolve style early using actual loaded state so
                        // Hidden entities skip model loading entirely.
                        let has_model_prop = ent.properties.get("model").is_some();
                        let has_model = ent.model.is_some();
                        let style = editor.entity_drawing.resolve(&ent.classname, has_model);

                        let draw_entity_visual =
                            ent.brushes.is_empty() && !matches!(style.kind, EntityDrawKind::Hidden);

                        let mut model_ref: Option<&crate::xmodel::XModel> = None;
                        if draw_entity_visual && has_model_prop {
                            if let Some(model_name) = ent.properties.get("model") {
                                let expected_name = {
                                    let normalized = normalize_material_name(model_name);
                                    if normalized.starts_with("xmodel/") {
                                        normalized
                                    } else {
                                        format!("xmodel/{normalized}")
                                    }
                                };
                                let needs_load = ent
                                    .model
                                    .as_ref()
                                    .map(|model| model.name != expected_name)
                                    .unwrap_or(true);
                                if needs_load && !editor.failed_models.contains(&expected_name) {
                                    if let Some(model_asset_db) = editor.model_asset_db.as_mut() {
                                        match XModel::load(model_asset_db, model_name, shader_db) {
                                            Ok(m) => ent.model = Some(m),
                                            Err(e) => {
                                                eprintln!("Failed load model {model_name}: {e}");
                                                editor.failed_models.insert(expected_name);
                                            }
                                        }
                                    }
                                }
                            }
                            model_ref = ent.model.as_ref();
                        }

                        // Recompute style with post-load model state
                        let has_model = ent.model.is_some();
                        let style = editor.entity_drawing.resolve(&ent.classname, has_model);

                        if draw_entity_visual {
                            match style.kind {
                                EntityDrawKind::Box
                                | EntityDrawKind::SolidBox
                                | EntityDrawKind::ModelBounds
                                | EntityDrawKind::ModelWireframe => {
                                    if let Some(model) = model_ref {
                                        let model_origin = pivot + model.origin;
                                        self.entity_line_batches.push((
                                            style.color,
                                            crate::core_util::model_bounds_line_vertices(
                                                model_origin,
                                                model.mins,
                                                model.maxs,
                                                model_rot,
                                            ),
                                        ));
                                        // Vertical center guide line (top to bottom)
                                        let (min_b, max_b) = crate::core_util::model_bounds_aabb(
                                            model_origin,
                                            model.mins,
                                            model.maxs,
                                            model_rot,
                                        );
                                        let cx = (min_b.x + max_b.x) * 0.5;
                                        let cy = (min_b.y + max_b.y) * 0.5;
                                        self.entity_line_batches.push((
                                            style.color,
                                            vec![
                                                Vec3::new(cx, cy, min_b.z),
                                                Vec3::new(cx, cy, max_b.z),
                                            ],
                                        ));
                                        if style.show_origin_box {
                                            self.entity_line_batches.push((
                                                [1.0, 1.0, 1.0, 1.0],
                                                crate::core_util::box_line_vertices(
                                                    model_origin,
                                                    Vec3::from_array(style.origin_box_size),
                                                    None,
                                                ),
                                            ));
                                        }
                                        if style.show_arrow {
                                            if let Some(angles) =
                                                angles.filter(|a| a.length_squared() > 1.0e-6)
                                            {
                                                let forward =
                                                    crate::core_util::entity_angles_forward(angles);
                                                let arrow_len = core_util::resolved_arrow_length(
                                                    &style,
                                                    model.radius,
                                                );
                                                self.entity_line_batches.push((
                                                    style.color,
                                                    crate::core_util::arrow_line_vertices(
                                                        crate::core_util::model_front_center(
                                                            model_origin,
                                                            model.mins,
                                                            model.maxs,
                                                            forward,
                                                        ),
                                                        forward,
                                                        arrow_len,
                                                        Vec3::Z,
                                                    ),
                                                ));
                                            }
                                        }
                                    } else {
                                        let size = Vec3::from_array(style.size);
                                        let dumb_fn = |p: Vec3| -> Vec3 { p };
                                        let tris = solid_box_lit_vertices_from_base(
                                            entity_solid_box_base(pivot, size, style.anchor),
                                            size,
                                            model_rot,
                                            dumb_fn,
                                        );
                                        self.entity_solid_batches.push((style.color, tris));
                                        if style.show_arrow {
                                            if let Some(angles) =
                                                angles.filter(|a| a.length_squared() > 1.0e-6)
                                            {
                                                let arrow_len = core_util::resolved_arrow_length(
                                                    &style,
                                                    size.length(),
                                                );
                                                self.entity_line_batches.push((
                                                    style.color,
                                                    crate::core_util::arrow_line_vertices(
                                                        crate::core_util::proxy_box_center(
                                                            style.anchor,
                                                            pivot,
                                                            size,
                                                        ),
                                                        crate::core_util::entity_angles_forward(
                                                            angles,
                                                        ),
                                                        arrow_len,
                                                        Vec3::Z,
                                                    ),
                                                ));
                                            }
                                        }
                                    }
                                }
                                EntityDrawKind::Hidden => {}
                            }
                        }

                        if editor.config.view.show.models {
                            if let Some(model) = model_ref {
                                let model_origin = pivot + model.origin;
                                for surface in &model.surfaces {
                                    if surface.indices.len() < 3 || surface.vertices.is_empty() {
                                        continue;
                                    }

                                    let texture_key =
                                        surface.texture_name.clone().unwrap_or_default();
                                    if !texture_key.is_empty() {
                                        editor.tex_registry.request(texture_key.clone());
                                    }

                                    let batch = tex_batches.entry(texture_key).or_default();
                                    for tri in surface.indices.chunks_exact(3) {
                                        let i0 = tri[0] as usize;
                                        let i1 = tri[1] as usize;
                                        let i2 = tri[2] as usize;
                                        if i0 >= surface.vertices.len()
                                            || i1 >= surface.vertices.len()
                                            || i2 >= surface.vertices.len()
                                        {
                                            continue;
                                        }

                                        let v0 = &surface.vertices[i0];
                                        let v1 = &surface.vertices[i1];
                                        let v2 = &surface.vertices[i2];
                                        let p0 = model_rot
                                            .map(|r| model_origin + r * v0.position)
                                            .unwrap_or(model_origin + v0.position);
                                        let p1 = model_rot
                                            .map(|r| model_origin + r * v1.position)
                                            .unwrap_or(model_origin + v1.position);
                                        let p2 = model_rot
                                            .map(|r| model_origin + r * v2.position)
                                            .unwrap_or(model_origin + v2.position);
                                        let n0 =
                                            model_rot.map(|r| r * v0.normal).unwrap_or(v0.normal);
                                        let n1 =
                                            model_rot.map(|r| r * v1.normal).unwrap_or(v1.normal);
                                        let n2 =
                                            model_rot.map(|r| r * v2.normal).unwrap_or(v2.normal);

                                        self.tri_vertices.push(LitVertex {
                                            pos: p0.into(),
                                            normal: n0.into(),
                                        });
                                        self.tri_vertices.push(LitVertex {
                                            pos: p1.into(),
                                            normal: n1.into(),
                                        });
                                        self.tri_vertices.push(LitVertex {
                                            pos: p2.into(),
                                            normal: n2.into(),
                                        });

                                        batch.push(TexVertex {
                                            pos: p0.into(),
                                            normal: n0.into(),
                                            uv: v0.uv.into(),
                                        });
                                        batch.push(TexVertex {
                                            pos: p1.into(),
                                            normal: n1.into(),
                                            uv: v1.uv.into(),
                                        });
                                        batch.push(TexVertex {
                                            pos: p2.into(),
                                            normal: n2.into(),
                                            uv: v2.uv.into(),
                                        });
                                    }
                                }
                            }
                        }

                        for brush in &mut ent.brushes {
                            if brush.is_clip() && !editor.config.view.show.clip_brushes {
                                continue;
                            }
                            if brush.is_portal() && !editor.config.view.show.portal_brushes {
                                continue;
                            }
                            if brush.is_hint() && !editor.config.view.show.hint_brushes {
                                continue;
                            }
                            if let BrushContent::Convex(faces_src) = &brush.content {
                                if !editor.config.view.show.convex {
                                    continue;
                                }
                                // Avoid borrow conflicts with `get_polygons_and_aabb()` by cloning
                                // the small face metadata we need (texture + params + plane points).
                                let faces = faces_src.clone();
                                let Some((aabb, polys)) = brush.get_polygons_and_aabb() else {
                                    continue;
                                };

                                debug_assert_eq!(
                                    polys.len(),
                                    faces.len(),
                                    "brush polys not aligned with faces"
                                );
                                bounds_min = bounds_min.min(aabb.min);
                                bounds_max = bounds_max.max(aabb.max);

                                if editor.config.view.wireframe {
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
                                }

                                for (face, (positions, indices)) in faces.iter().zip(polys.iter()) {
                                    if positions.len() < 3 || indices.len() < 3 {
                                        continue;
                                    }

                                    let nf = crate::texmap::face_plane_normal(face).to_array();

                                    // Fallback size if the texture isn't loaded yet (keeps UV math stable).
                                    let (tex_w, tex_h) = editor
                                        .tex_registry
                                        .get(&face.texture)
                                        .map(|rt| (rt.size[0], rt.size[1]))
                                        .unwrap_or((256.0, 256.0));
                                    let mapper = FaceUvMapper::new(face, tex_w, tex_h);

                                    for tri in indices.chunks_exact(3) {
                                        let i0 = tri[0] as usize;
                                        let i1 = tri[1] as usize;
                                        let i2 = tri[2] as usize;
                                        if i0 >= positions.len()
                                            || i1 >= positions.len()
                                            || i2 >= positions.len()
                                        {
                                            continue;
                                        }

                                        let v0 = positions[i0];
                                        let v1 = positions[i1];
                                        let v2 = positions[i2];

                                        self.tri_vertices.push(LitVertex {
                                            pos: v0.into(),
                                            normal: nf,
                                        });
                                        self.tri_vertices.push(LitVertex {
                                            pos: v1.into(),
                                            normal: nf,
                                        });
                                        self.tri_vertices.push(LitVertex {
                                            pos: v2.into(),
                                            normal: nf,
                                        });

                                        let batch =
                                            tex_batches.entry(face.texture.clone()).or_default();
                                        batch.push(TexVertex {
                                            pos: v0.into(),
                                            normal: nf,
                                            uv: mapper.uv(v0).to_array(),
                                        });
                                        batch.push(TexVertex {
                                            pos: v1.into(),
                                            normal: nf,
                                            uv: mapper.uv(v1).to_array(),
                                        });
                                        batch.push(TexVertex {
                                            pos: v2.into(),
                                            normal: nf,
                                            uv: mapper.uv(v2).to_array(),
                                        });
                                    }
                                }
                            } else if let BrushContent::Patch(patch) = &mut brush.content {
                                if !editor.config.view.show.patches {
                                    continue;
                                }
                                if editor.config.view.wireframe {
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
                                }

                                if let Ok(tess) = tessellate_patch(patch) {
                                    let pos = &tess.positions;
                                    let indices = &tess.indices;
                                    let uvs = &tess.uvs;
                                    let normals = &tess.normals;

                                    for chunk in indices.chunks_exact(3) {
                                        let i0 = chunk[0] as usize;
                                        let i1 = chunk[1] as usize;
                                        let i2 = chunk[2] as usize;

                                        if i0 >= pos.len()
                                            || i1 >= pos.len()
                                            || i2 >= pos.len()
                                            || i0 >= uvs.len()
                                            || i1 >= uvs.len()
                                            || i2 >= uvs.len()
                                            || i0 >= normals.len()
                                            || i1 >= normals.len()
                                            || i2 >= normals.len()
                                        {
                                            continue;
                                        }

                                        let v0 = pos[i0];
                                        let v1 = pos[i1];
                                        let v2 = pos[i2];

                                        // Flat face normal (same as convex brushes)
                                        let n = (v1 - v0).cross(v2 - v0).normalize_or_zero();
                                        let nf = [n.x, n.y, n.z];

                                        self.tri_vertices.push(LitVertex {
                                            pos: v0.into(),
                                            normal: nf,
                                        });
                                        self.tri_vertices.push(LitVertex {
                                            pos: v1.into(),
                                            normal: nf,
                                        });
                                        self.tri_vertices.push(LitVertex {
                                            pos: v2.into(),
                                            normal: nf,
                                        });

                                        let batch =
                                            tex_batches.entry(patch.texture.clone()).or_default();
                                        batch.push(TexVertex {
                                            pos: v0.into(),
                                            normal: normals[i0].to_array(),
                                            uv: uvs[i0].to_array(),
                                        });
                                        batch.push(TexVertex {
                                            pos: v1.into(),
                                            normal: normals[i1].to_array(),
                                            uv: uvs[i1].to_array(),
                                        });
                                        batch.push(TexVertex {
                                            pos: v2.into(),
                                            normal: normals[i2].to_array(),
                                            uv: uvs[i2].to_array(),
                                        });
                                    }
                                }
                            }
                        }
                    }

                    let mut tmp: Vec<_> = tex_batches.into_iter().collect();
                    tmp.sort_by(|a, b| a.0.cmp(&b.0));
                    self.tri_vertices_tex = tmp;
                }

                self.last_selection_hash = 5382;
                let prev_map_ptr = self.cache.map(|c| c.map_ptr).unwrap_or(0);

                self.cache = Some(View3dCache {
                    map_present,
                    map_ptr,
                    map_generation,
                    map_revision,
                    map_load_count,
                    view_config_rev,
                });

                // Auto-frame camera only when the actual map instance changes.
                if map_ptr != 0
                    && map_ptr != prev_map_ptr
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
                    h = h
                        .wrapping_mul(31)
                        .wrapping_add((e as u64) * 1000003 + (b as u64));
                }

                let mut faces: Vec<_> = editor.selected_faces.iter().collect();
                faces.sort_by_key(|f| (f.entity_idx, f.brush_idx, f.face_idx));
                for f in &faces {
                    h = h
                        .wrapping_mul(31)
                        .wrapping_add((f.entity_idx as u64) * 1000003)
                        .wrapping_add((f.brush_idx as u64) * 1000001)
                        .wrapping_add(f.face_idx as u64);
                }

                let mut ents: Vec<_> = editor.selected_entities.iter().collect();
                ents.sort();
                for e in ents {
                    h = h
                        .wrapping_mul(31)
                        .wrapping_add((*e as u64) * 1000003)
                        .wrapping_add((*e as u64) * 1000001)
                        .wrapping_add(*e as u64);
                }

                h
            };
            //println!("current_selection_hash: {}\nlast_selection_hash: {}", current_selection_hash, self.last_selection_hash);

            // Drag preview state: the highlight must follow the cursor while
            // dragging, but the selection hash doesn't change during a drag.
            // Include the preview transform in the rebuild condition.
            let preview_hash = {
                let mut h: u64 = 5381;
                h = h
                    .wrapping_mul(31)
                    .wrapping_add(editor.view3d.drag_mode as u64);
                let o = editor.view3d.move_offset;
                for b in [o.x.to_bits(), o.y.to_bits(), o.z.to_bits()] {
                    h = h.wrapping_mul(31).wrapping_add(b as u64);
                }
                let r = editor.view3d.rotate_angle;
                h = h.wrapping_mul(31).wrapping_add(r.to_bits() as u64);
                let s = editor.view3d.stretch_delta;
                for b in [s.x.to_bits(), s.y.to_bits(), s.z.to_bits()] {
                    h = h.wrapping_mul(31).wrapping_add(b as u64);
                }
                h
            };

            if current_selection_hash != self.last_selection_hash
                || preview_hash != self.last_preview_hash
            {
                self.tri_vertices_selected.clear();
                let preview_drag_mode = editor.view3d.drag_mode;
                let preview_move_offset = editor.view3d.move_offset;
                let preview_rotate = editor.view3d.rotate_preview_xform();
                let preview_point = |p: Vec3| -> Vec3 {
                    core_util::preview_point(
                        preview_drag_mode,
                        preview_move_offset,
                        None,
                        None,
                        preview_rotate,
                        p,
                    )
                };

                if let Some(map) = editor.map.as_mut() {
                    let _selected_face_set: HashSet<(usize, usize, usize)> = editor
                        .selected_faces
                        .iter()
                        .map(|f| (f.entity_idx, f.brush_idx, f.face_idx))
                        .collect();

                    for (entity_idx, ent) in map.entities.iter_mut().enumerate() {
                        if editor.selected_entities.contains(&entity_idx) && ent.brushes.is_empty()
                        {
                            let style = editor
                                .entity_drawing
                                .resolve(&ent.classname, ent.model.is_some());
                            let origin = ent
                                .properties
                                .get("origin")
                                .map(|s| core_util::origin_to_vec3(s))
                                .unwrap_or(Vec3::ZERO);

                            // If entity has a model, render model triangles for selection
                            if let Some(model) = &ent.model {
                                let model_origin = origin + model.origin;
                                let angles = ent
                                    .properties
                                    .get("angles")
                                    .and_then(|s| core_util::vec3_from_whitespace_triplet(s));
                                let model_rot = angles.map(core_util::entity_angles_to_quat);

                                for surface in &model.surfaces {
                                    if surface.indices.len() < 3 || surface.vertices.is_empty() {
                                        continue;
                                    }

                                    for tri in surface.indices.chunks_exact(3) {
                                        let i0 = tri[0] as usize;
                                        let i1 = tri[1] as usize;
                                        let i2 = tri[2] as usize;
                                        if i0 >= surface.vertices.len()
                                            || i1 >= surface.vertices.len()
                                            || i2 >= surface.vertices.len()
                                        {
                                            continue;
                                        }

                                        let v0 = &surface.vertices[i0];
                                        let v1 = &surface.vertices[i1];
                                        let v2 = &surface.vertices[i2];
                                        let p0 = model_rot
                                            .map(|r| model_origin + r * v0.position)
                                            .unwrap_or(model_origin + v0.position);
                                        let p1 = model_rot
                                            .map(|r| model_origin + r * v1.position)
                                            .unwrap_or(model_origin + v1.position);
                                        let p2 = model_rot
                                            .map(|r| model_origin + r * v2.position)
                                            .unwrap_or(model_origin + v2.position);
                                        let n0 =
                                            model_rot.map(|r| r * v0.normal).unwrap_or(v0.normal);
                                        let n1 =
                                            model_rot.map(|r| r * v1.normal).unwrap_or(v1.normal);
                                        let n2 =
                                            model_rot.map(|r| r * v2.normal).unwrap_or(v2.normal);

                                        self.tri_vertices_selected.push(LitVertex {
                                            pos: preview_point(p0).into(),
                                            normal: n0.into(),
                                        });
                                        self.tri_vertices_selected.push(LitVertex {
                                            pos: preview_point(p1).into(),
                                            normal: n1.into(),
                                        });
                                        self.tri_vertices_selected.push(LitVertex {
                                            pos: preview_point(p2).into(),
                                            normal: n2.into(),
                                        });
                                    }
                                }
                            } else {
                                // Fall back to proxy box for entities without models
                                let size = Vec3::from_array(style.size);
                                let base = entity_solid_box_base(origin, size, style.anchor);
                                let angles = ent
                                    .properties
                                    .get("angles")
                                    .and_then(|s| core_util::vec3_from_whitespace_triplet(s));
                                let rot = angles.map(core_util::entity_angles_to_quat);
                                let tris = solid_box_lit_vertices_from_base(
                                    base,
                                    size,
                                    rot,
                                    preview_point,
                                );
                                self.tri_vertices_selected.extend(tris);
                            }
                        }
                        for (brush_idx, brush) in ent.brushes.iter_mut().enumerate() {
                            let is_brush_selected =
                                editor.selected_brushes.contains(&(entity_idx, brush_idx));

                            let mut faces_to_highlight: Vec<usize> = Vec::new();

                            if editor.edit_faces {
                                // Face edit mode: ONLY highlight selected faces
                                for face_sel in &editor.selected_faces {
                                    if face_sel.entity_idx == entity_idx
                                        && face_sel.brush_idx == brush_idx
                                    {
                                        faces_to_highlight.push(face_sel.face_idx);
                                    }
                                }
                            } else if editor.edit_edges {
                                // Edge edit mode: edges are drawn separately;
                                // no whole-brush face tint.
                            } else if !editor.edit_vertices {
                                // Brush edit mode: highlight all faces of selected brushes
                                if is_brush_selected {
                                    match &brush.content {
                                        BrushContent::Convex(faces) => {
                                            faces_to_highlight.extend(0..faces.len());
                                        }
                                        BrushContent::Patch(_) => {
                                            faces_to_highlight.push(0); // whole patch
                                        }
                                    }
                                }
                            }

                            if faces_to_highlight.is_empty() || editor.edit_vertices {
                                continue;
                            }

                            match &mut brush.content {
                                BrushContent::Convex(_) => {
                                    // For stretch preview, compute polygons from a hypothetical stretched brush
                                    let stretched_polys;
                                    let polys = if editor.view3d.drag_mode
                                        == DragMode::StretchSelection
                                    {
                                        let mut probe = brush.clone();
                                        if let BrushContent::Convex(probe_faces) =
                                            &mut probe.content
                                        {
                                            if let Some(stretch) = editor.view3d.stretch.as_ref() {
                                                for (ent_idx, br_idx, face_indices) in
                                                    &stretch.side_faces
                                                {
                                                    if *ent_idx == entity_idx
                                                        && *br_idx == brush_idx
                                                    {
                                                        for &face_idx in face_indices {
                                                            if let Some(face) =
                                                                probe_faces.get_mut(face_idx)
                                                            {
                                                                for p in &mut face.plane_points {
                                                                    *p +=
                                                                        editor.view3d.stretch_delta;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        // Clear cached geometry so get_polygons recomputes from modified planes
                                        probe.invalidate_geometry();
                                        stretched_polys = probe.get_polygons().map(|p| p.to_vec());
                                        stretched_polys.as_deref()
                                    } else {
                                        brush.get_polygons()
                                    };
                                    let Some(polys) = polys else {
                                        continue;
                                    };

                                    for &face_idx in &faces_to_highlight {
                                        if face_idx >= polys.len() {
                                            continue;
                                        }
                                        let (positions, _) = &polys[face_idx];
                                        if positions.len() < 3 {
                                            continue;
                                        }

                                        let e1 = positions[1] - positions[0];
                                        let e2 = positions[2] - positions[0];
                                        let n = e1.cross(e2).normalize_or_zero();
                                        let nf = [n.x, n.y, n.z];

                                        for i in 1..positions.len() - 1 {
                                            let v0 = preview_point(positions[0]);
                                            let v1 = preview_point(positions[i]);
                                            let v2 = preview_point(positions[i + 1]);
                                            self.tri_vertices_selected.push(LitVertex {
                                                pos: v0.into(),
                                                normal: nf,
                                            });
                                            self.tri_vertices_selected.push(LitVertex {
                                                pos: v1.into(),
                                                normal: nf,
                                            });
                                            self.tri_vertices_selected.push(LitVertex {
                                                pos: v2.into(),
                                                normal: nf,
                                            });
                                        }
                                    }
                                }
                                BrushContent::Patch(patch) => {
                                    if let Ok(tess) = tessellate_patch(patch) {
                                        let pos = &tess.positions;
                                        let indices = &tess.indices;

                                        for chunk in indices.chunks_exact(3) {
                                            let i0 = chunk[0] as usize;
                                            let i1 = chunk[1] as usize;
                                            let i2 = chunk[2] as usize;

                                            if i0 >= pos.len() || i1 >= pos.len() || i2 >= pos.len()
                                            {
                                                continue;
                                            }

                                            let v0 = preview_point(pos[i0]);
                                            let v1 = preview_point(pos[i1]);
                                            let v2 = preview_point(pos[i2]);

                                            let n = (v1 - v0).cross(v2 - v0).normalize_or_zero();
                                            let nf = [n.x, n.y, n.z];

                                            self.tri_vertices_selected.push(LitVertex {
                                                pos: v0.into(),
                                                normal: nf,
                                            });
                                            self.tri_vertices_selected.push(LitVertex {
                                                pos: v1.into(),
                                                normal: nf,
                                            });
                                            self.tri_vertices_selected.push(LitVertex {
                                                pos: v2.into(),
                                                normal: nf,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                self.last_selection_hash = current_selection_hash;
                self.last_preview_hash = preview_hash;
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
            backend.gl.enable(glow::CULL_FACE);
            backend.gl.cull_face(glow::BACK);

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
            let fov = editor.config.view.fov.to_radians().clamp(0.1, 3.0);
            let proj = glam::Mat4::perspective_rh_gl(fov, aspect, 4.0, 100_000.0);
            let mvp = proj * view;

            let geom_col = editor.palette.view2d_geometry;
            let light_dir = Vec3::new(0.5, 0.25, 1.0).normalize();
            let ambient = 0.4f32;

            match editor.config.view.rendermode {
                RenderMode::None => {}
                RenderMode::Flat => {
                    if !self.tri_vertices.is_empty() {
                        backend.draw_triangles_lit(
                            &self.tri_vertices,
                            geom_col,
                            mvp,
                            ambient,
                            light_dir,
                        );
                    }
                }
                _ => {
                    let mut tmp_lit: Vec<LitVertex> = Vec::new();
                    // First pass: opaque textured faces (no blending).
                    for (material, batch) in &self.tri_vertices_tex {
                        if let Some(rt) = editor.tex_registry.get(material) {
                            let alpha = 1.0 - rt.qer.trans.unwrap_or(0.0).clamp(0.0, 1.0);
                            if alpha < 1.0 {
                                continue; // skip transparent for second pass
                            }
                            backend.draw_triangles_tex(
                                batch,
                                rt.tex,
                                [1.0, 1.0, 1.0, alpha],
                                mvp,
                                ambient,
                                light_dir,
                            );
                        } else {
                            // Fallback while texture is still loading.
                            tmp_lit.clear();
                            tmp_lit.reserve(batch.len());
                            for v in batch {
                                tmp_lit.push(LitVertex {
                                    pos: v.pos,
                                    normal: v.normal,
                                });
                            }
                            backend.draw_triangles_lit(&tmp_lit, geom_col, mvp, ambient, light_dir);
                        }
                    }
                    // Second pass: transparent textured faces (blending on, no depth write).
                    backend.gl.enable(glow::BLEND);
                    backend
                        .gl
                        .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                    backend.gl.depth_mask(false);
                    for (material, batch) in &self.tri_vertices_tex {
                        let Some(rt) = editor.tex_registry.get(material) else {
                            continue;
                        };
                        let alpha = 1.0 - rt.qer.trans.unwrap_or(0.0).clamp(0.0, 1.0);
                        if alpha >= 1.0 {
                            continue;
                        }
                        backend.draw_triangles_tex(
                            batch,
                            rt.tex,
                            [1.0, 1.0, 1.0, alpha],
                            mvp,
                            ambient,
                            light_dir,
                        );
                    }
                    backend.gl.depth_mask(true);
                    backend.gl.disable(glow::BLEND);
                }
            }
            if editor.config.view.wireframe && !self.line_vertices.is_empty() {
                backend.draw_lines(&self.line_vertices, geom_col, mvp);
            }

            // Edge edit mode: draw every convex brush edge, highlight the
            // selected ones (always on top), and preview the drag by re-fitting
            // the adjacent face planes exactly like the final apply will
            // (including the move clamp, so the preview never shows the brush
            // stretching out towards infinity).
            if editor.edit_edges {
                let raw_delta = if editor.view3d.drag_mode == DragMode::MoveEdges {
                    editor.view3d.move_offset
                } else {
                    Vec3::ZERO
                };
                let sel_hash = edge_selection_hash(&editor.selected_edges);
                let map_gen = editor.map.as_ref().map(|m| m.generation).unwrap_or(0);
                let map_rev = editor.map_revision;
                let clamp_factor = if raw_delta != Vec3::ZERO && !editor.selected_edges.is_empty() {
                    if raw_delta == self.last_edge_clamp_offset
                        && sel_hash == self.last_edge_clamp_selection_hash
                        && map_gen == self.last_edge_clamp_map_generation
                        && map_ptr == self.last_edge_clamp_map_ptr
                        && map_rev == self.last_edge_clamp_map_revision
                    {
                        self.last_edge_clamp_factor
                    } else {
                        let factor = editor
                            .map
                            .as_ref()
                            .map(|m| {
                                editing::edge_move_clamp_factor(
                                    m,
                                    &editor.selected_edges,
                                    raw_delta,
                                )
                            })
                            .unwrap_or(0.0);
                        self.last_edge_clamp_offset = raw_delta;
                        self.last_edge_clamp_selection_hash = sel_hash;
                        self.last_edge_clamp_map_generation = map_gen;
                        self.last_edge_clamp_map_ptr = map_ptr;
                        self.last_edge_clamp_map_revision = map_rev;
                        self.last_edge_clamp_factor = factor;
                        factor
                    }
                } else {
                    1.0
                };
                let drag_delta = raw_delta * clamp_factor;
                let previewing = drag_delta != Vec3::ZERO && !editor.selected_edges.is_empty();

                // Selected edges grouped per brush: normalized face pairs.
                let mut sel_by_brush: HashMap<(usize, usize), Vec<(usize, usize)>> = HashMap::new();
                for sel in &editor.selected_edges {
                    let pair = (
                        sel.face_a_idx.min(sel.face_b_idx),
                        sel.face_a_idx.max(sel.face_b_idx),
                    );
                    let pairs = sel_by_brush
                        .entry((sel.entity_idx, sel.brush_idx))
                        .or_default();
                    if !pairs.contains(&pair) {
                        pairs.push(pair);
                    }
                }
                let is_selected_edge = |entity_idx: usize, brush_idx: usize, a: usize, b: usize| {
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    sel_by_brush
                        .get(&(entity_idx, brush_idx))
                        .map(|pairs| pairs.contains(&(lo, hi)))
                        .unwrap_or(false)
                };

                let mut sel_handles: Vec<Vec3> = Vec::new();

                // Compute config hash for dimmed edges cache
                let config_hash = {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    editor.config.view.show.convex.hash(&mut hasher);
                    editor.config.view.show.clip_brushes.hash(&mut hasher);
                    editor.config.view.show.portal_brushes.hash(&mut hasher);
                    editor.config.view.show.hint_brushes.hash(&mut hasher);
                    hasher.finish()
                };

                // Check if we can reuse cached dimmed edges (no preview, same state)
                let use_cached_dim = !previewing
                    && map_gen == self.last_dim_edges_map_generation
                    && sel_hash == self.last_dim_edges_selection_hash
                    && config_hash == self.last_dim_edges_config_hash
                    && map_ptr == self.last_dim_edges_map_ptr
                    && map_rev == self.last_dim_edges_map_revision;

                // During a drag with a valid cache, only iterate brushes that
                // have selected edges (for sel_edges); dim_edges comes from cache.
                // When not previewing, iterate all brushes to rebuild the cache.
                let dim_edges: Vec<Vec3> = if use_cached_dim {
                    self.cached_dim_edges.clone()
                } else if previewing {
                    // During drag without a matching cache (e.g. selection changed),
                    // use stale cache for dim_edges and only compute sel_edges.
                    self.cached_dim_edges.clone()
                } else {
                    let mut edges = Vec::new();
                    if let Some(map) = editor.map.as_mut() {
                        for (entity_idx, entity) in map.entities.iter_mut().enumerate() {
                            for (brush_idx, brush) in entity.brushes.iter_mut().enumerate() {
                                if !matches!(&brush.content, BrushContent::Convex(_)) {
                                    continue;
                                }
                                if !editor.config.view.show.convex
                                    || (brush.is_clip() && !editor.config.view.show.clip_brushes)
                                    || (brush.is_portal()
                                        && !editor.config.view.show.portal_brushes)
                                    || (brush.is_hint() && !editor.config.view.show.hint_brushes)
                                {
                                    continue;
                                }

                                let polys = brush.get_polygons().unwrap_or(&[]);
                                for fa in 0..polys.len() {
                                    for fb in (fa + 1)..polys.len() {
                                        let Some((a, b)) =
                                            core_util::shared_edge_points(polys, fa, fb)
                                        else {
                                            continue;
                                        };
                                        if !is_selected_edge(entity_idx, brush_idx, fa, fb) {
                                            edges.push(a);
                                            edges.push(b);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    self.last_dim_edges_map_generation = map_gen;
                    self.last_dim_edges_selection_hash = sel_hash;
                    self.last_dim_edges_config_hash = config_hash;
                    self.last_dim_edges_map_ptr = map_ptr;
                    self.last_dim_edges_map_revision = map_rev;
                    self.cached_dim_edges = edges.clone();
                    edges
                };

                // Compute sel_edges: always from live data since it depends on
                // the current preview delta (moved polygons differ per frame).
                let mut sel_edges: Vec<Vec3> = Vec::new();
                if let Some(map) = editor.map.as_mut() {
                    // Only iterate brushes that have selected edges.
                    let selected_brushes: Vec<(usize, usize)> = editor
                        .selected_edges
                        .iter()
                        .map(|e| (e.entity_idx, e.brush_idx))
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect();
                    for (entity_idx, brush_idx) in selected_brushes {
                        let Some(entity) = map.entities.get_mut(entity_idx) else {
                            continue;
                        };
                        let Some(brush) = entity.brushes.get_mut(brush_idx) else {
                            continue;
                        };
                        if !matches!(&brush.content, BrushContent::Convex(_)) {
                            continue;
                        }

                        let owned_polys: Option<Vec<(Vec<Vec3>, Vec<u32>)>>;
                        let polys: &[(Vec<Vec3>, Vec<u32>)] = if previewing {
                            match sel_by_brush.get(&(entity_idx, brush_idx)) {
                                Some(pairs) => {
                                    match editing::preview_edge_moved_polys(
                                        brush, pairs, drag_delta,
                                    ) {
                                        Some(p) => {
                                            owned_polys = Some(p);
                                            owned_polys.as_deref().unwrap_or(&[])
                                        }
                                        None => brush.get_polygons().unwrap_or(&[]),
                                    }
                                }
                                None => brush.get_polygons().unwrap_or(&[]),
                            }
                        } else {
                            brush.get_polygons().unwrap_or(&[])
                        };

                        for fa in 0..polys.len() {
                            for fb in (fa + 1)..polys.len() {
                                let Some((a, b)) = core_util::shared_edge_points(polys, fa, fb)
                                else {
                                    continue;
                                };
                                if is_selected_edge(entity_idx, brush_idx, fa, fb) {
                                    sel_edges.push(a);
                                    sel_edges.push(b);
                                }
                            }
                        }
                    }
                }

                // Small cube handles at the selected edge endpoints.
                for chunk in sel_edges.chunks_exact(2) {
                    let push_cube = |out: &mut Vec<Vec3>, center: Vec3, size: f32| {
                        let corners = [
                            Vec3::new(-size, -size, -size),
                            Vec3::new(size, -size, -size),
                            Vec3::new(size, size, -size),
                            Vec3::new(-size, size, -size),
                            Vec3::new(-size, -size, size),
                            Vec3::new(size, -size, size),
                            Vec3::new(size, size, size),
                            Vec3::new(-size, size, size),
                        ];
                        let edge_ids = [
                            0, 1, 1, 2, 2, 3, 3, 0, // bottom
                            4, 5, 5, 6, 6, 7, 7, 4, // top
                            0, 4, 1, 5, 2, 6, 3, 7, // pillars
                        ];
                        for &idx in &edge_ids {
                            out.push(center + corners[idx]);
                        }
                    };
                    push_cube(&mut sel_handles, chunk[0], 1.5);
                    push_cube(&mut sel_handles, chunk[1], 1.5);
                }

                let dim_col = [
                    geom_col[0] * 0.65,
                    geom_col[1] * 0.65,
                    geom_col[2] * 0.65,
                    1.0,
                ];

                if !dim_edges.is_empty() {
                    backend.gl.enable(glow::DEPTH_TEST);
                    backend.draw_lines(&dim_edges, dim_col, mvp);
                }
                if !sel_edges.is_empty() {
                    // Selected edges render on top (depth test off), like a
                    // Blender edit-mode selection.
                    backend.gl.disable(glow::DEPTH_TEST);
                    backend.draw_lines(&sel_edges, editor.selection_rgba, mvp);
                    if !sel_handles.is_empty() {
                        backend.draw_lines(&sel_handles, editor.selection_rgba, mvp);
                    }
                    backend.gl.enable(glow::DEPTH_TEST);
                }
            }

            if !self.entity_solid_batches.is_empty() {
                backend.gl.depth_mask(true);
                for (color, tris) in &self.entity_solid_batches {
                    if tris.is_empty() {
                        continue;
                    }
                    backend.draw_triangles_lit(tris, *color, mvp, ambient, light_dir);
                }
            }

            for (color, lines) in &self.entity_line_batches {
                if lines.is_empty() {
                    continue;
                }
                backend.draw_lines(lines, *color, mvp);
            }

            if !self.tri_vertices_selected.is_empty() && !editor.edit_vertices {
                backend.gl.enable(glow::BLEND);
                backend
                    .gl
                    .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                // Disable depth test so the tint is visible through geometry
                if !editor.edit_faces {
                    backend.gl.disable(glow::DEPTH_TEST);
                }

                // let tint = [1.0, 0.25, 0.25, 0.65];
                let tint = {
                    let base = editor.selection_rgba;
                    [base[0], base[1], base[2], base[3] * 0.75]
                };
                backend.draw_triangles_lit(
                    &self.tri_vertices_selected,
                    tint,
                    mvp,
                    ambient * 0.7,
                    light_dir,
                );

                backend.gl.enable(glow::DEPTH_TEST);
                backend.gl.disable(glow::BLEND);
            }

            self.control_vertices.clear();
            self.control_selected_vertices.clear();
            let mut aabb_lines = Vec::new();

            if editor.edit_vertices {
                if let Some(map) = &editor.map {
                    for (entity_idx, entity) in map.entities.iter().enumerate() {
                        for (brush_idx, brush) in entity.brushes.iter().enumerate() {
                            if editor.selected_brushes.contains(&(entity_idx, brush_idx)) {
                                let min = brush.aabb.min;
                                let max = brush.aabb.max;
                                let corners = [
                                    Vec3::new(min.x, min.y, min.z),
                                    Vec3::new(max.x, min.y, min.z),
                                    Vec3::new(max.x, max.y, min.z),
                                    Vec3::new(min.x, max.y, min.z),
                                    Vec3::new(min.x, min.y, max.z),
                                    Vec3::new(max.x, min.y, max.z),
                                    Vec3::new(max.x, max.y, max.z),
                                    Vec3::new(min.x, max.y, max.z),
                                ];
                                let edges = [
                                    0, 1, 1, 2, 2, 3, 3, 0, // bottom
                                    4, 5, 5, 6, 6, 7, 7, 4, // top
                                    0, 4, 1, 5, 2, 6, 3, 7, // pillars
                                ];
                                for &idx in &edges {
                                    aabb_lines.push(corners[idx]);
                                }
                            }

                            if let crate::map::BrushContent::Patch(patch) = &brush.content {
                                for (row_idx, row) in patch.vertices.iter().enumerate() {
                                    for (col_idx, vtx) in row.iter().enumerate() {
                                        let is_selected =
                                            editor.selected_patch_vertices.iter().any(|sel| {
                                                sel.entity_idx == entity_idx
                                                    && sel.brush_idx == brush_idx
                                                    && sel.row == row_idx
                                                    && sel.col == col_idx
                                            });

                                        if is_selected {
                                            self.control_selected_vertices.push(vtx.position);
                                        } else {
                                            self.control_vertices.push(vtx.position);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !aabb_lines.is_empty() {
                backend.gl.disable(glow::DEPTH_TEST);
                backend.draw_lines(&aabb_lines, editor.selection_rgba, mvp);
                backend.gl.enable(glow::DEPTH_TEST);
            }

            if !self.control_vertices.is_empty() || !self.control_selected_vertices.is_empty() {
                backend.gl.disable(glow::DEPTH_TEST);

                let mut lines = Vec::new();
                let push_cube = |out: &mut Vec<Vec3>, center: Vec3, size: f32| {
                    let s = size;
                    let corners = [
                        Vec3::new(-s, -s, -s),
                        Vec3::new(s, -s, -s),
                        Vec3::new(s, s, -s),
                        Vec3::new(-s, s, -s),
                        Vec3::new(-s, -s, s),
                        Vec3::new(s, -s, s),
                        Vec3::new(s, s, s),
                        Vec3::new(-s, s, s),
                    ];
                    let edges = [
                        0, 1, 1, 2, 2, 3, 3, 0, // bottom
                        4, 5, 5, 6, 6, 7, 7, 4, // top
                        0, 4, 1, 5, 2, 6, 3, 7, // pillars
                    ];
                    for &idx in &edges {
                        out.push(center + corners[idx]);
                    }
                };

                if !self.control_vertices.is_empty() {
                    for &v in &self.control_vertices {
                        push_cube(&mut lines, v, 2.0);
                    }
                    backend.draw_lines(&lines, [0.8, 0.8, 0.8, 1.0], mvp);
                    lines.clear();
                }

                if !self.control_selected_vertices.is_empty() {
                    for &v in &self.control_selected_vertices {
                        push_cube(&mut lines, v, 3.0);
                    }
                    backend.draw_lines(&lines, editor.selection_rgba, mvp);
                }

                backend.gl.enable(glow::DEPTH_TEST);
            }

            backend.gl.disable(glow::CULL_FACE);
            backend.gl.disable(glow::DEPTH_TEST);
            backend.gl.use_program(None);
            backend.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }
}
