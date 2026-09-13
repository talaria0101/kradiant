//! 3D View

use dear_imgui_rs::{Condition, Key, StyleColor, TextureId, Ui, WindowFlags};
use glam::{Mat4, Vec2, Vec3, Vec4};
use kradiant::editing::Aabb;
use kradiant::editor::config::EntityDrawAnchor;
use kradiant::editor::viewport::types::Ortho;
use kradiant::editor::viewport::DragMode;
use kradiant::editor::viewport::state::{RotateDrag, SideStretchDrag, View3DState};
use kradiant::editor::{EditorConfig, EditorPalette, FaceSelection, PatchVertexSelection};
use kradiant::map_utils::format_vec3;
use std::ops::{Deref, DerefMut};

use crate::ui::editing::PickMask;
use crate::ui::view2d::{self, apply_affine_rotate_to_selected_faces, selection_aabb_active, selection_aabb_faces_from_map, selection_aabb_from_map};
use crate::util::{self, adjust_color_brightness, imgui_color_to_u32, text_height, text_width, world_to_screen_3d};
use kradiant::{core_util, editing};
use kradiant::editor::selection::EdgeSelection;
use std::collections::HashSet;

pub struct View3D {
    pub core: View3DState,
    pub tex_id: Option<TextureId>,
    pub wants_cursor_grab: bool,
    pub right_dragging: bool,
    pub last_cursor_pos: Option<[f32; 2]>,
    pub accumulated_mouse_delta: [f32; 2],
    /// Brushes touched during current selection drag (to avoid toggling multiple times)
    pub selection_drag_touched: Option<HashSet<(usize, usize)>>,
    /// Faces touched during current selection drag
    pub selection_drag_touched_faces: Option<HashSet<FaceSelection>>,
    /// Edges touched during current selection drag
    pub selection_drag_touched_edges: Option<HashSet<EdgeSelection>>,
    /// Patch vertices touched during current selection drag
    pub selection_drag_touched_verts: Option<HashSet<usize>>,
    pub selection_drag_touched_entities: Option<HashSet<usize>>,
    // Current drag mode for transformations
    // pub drag_mode: DragMode,
    /// Drag start position in screen space
    pub drag_start: Option<Vec2>,
    /// Drag current position in screen space
    pub drag_current: Option<Vec2>,
    pub drag_plane_normal: Vec3, // camera forward at drag start
    pub drag_plane_origin: Vec3, // selection center
    pub drag_world_anchor: Vec3, // world point clicked at drag start
    // Accumulated drag offset during drag (applied to geometry only on release)
    // pub move_offset: Vec3,
    work_pos: Vec3,
    pub last_aabb: Option<Aabb>,
}

impl Default for View3D {
    fn default() -> Self {
        Self {
            core: View3DState::default(),
            tex_id: None,
            wants_cursor_grab: false,
            right_dragging: false,
            last_cursor_pos: None,
            accumulated_mouse_delta: [0.0, 0.0],
            selection_drag_touched: None,
            selection_drag_touched_faces: None,
            selection_drag_touched_edges: None,
            selection_drag_touched_verts: None,
            selection_drag_touched_entities: None,
            // drag_mode: DragMode::NewBrush,
            drag_start: None,
            drag_current: None,
            drag_plane_normal: Vec3::ZERO,
            drag_plane_origin: Vec3::ZERO,
            drag_world_anchor: Vec3::ZERO,
            // move_offset: Vec3::ZERO,
            work_pos: Vec3::ZERO,
            last_aabb: None,
        }
    }
}

impl Deref for View3D {
    type Target = View3DState;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl DerefMut for View3D {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl View3D {
    fn forward_from_angles(angles: Vec3) -> Vec3 {
        let (yaw, pitch) = (angles.x, angles.y);
        // Z-up variant of the common yaw/pitch forward calculation.
        Vec3::new(
            yaw.cos() * pitch.cos(),
            yaw.sin() * pitch.cos(),
            pitch.sin(),
        )
        .normalize()
    }

    fn right_from_angles(angles: Vec3) -> Vec3 {
        let yaw = angles.x;
        Vec3::new(yaw.sin(), -yaw.cos(), 0.0).normalize()
    }

    pub fn update_work_from_aabb(&mut self, aabb: Aabb) {
        self.work_pos = (aabb.min + aabb.max) / 2.0;
    }

    pub fn center_to_work(&mut self) {
        let forward = Self::forward_from_angles(self.cam.angles);
        self.cam.pos = self.work_pos - forward * self.cam.zoom;
    }

    pub fn draw_impl(
        &mut self,
        ui: &Ui,
        config: &mut EditorConfig,
        palette: &EditorPalette,
        console: &mut crate::ui::console::ConsoleLogger,
        undo: &mut kradiant::editor::undo::UndoRedo,
        selected_brushes: &mut Vec<(usize, usize)>,
        selected_faces: &mut Vec<FaceSelection>,
        selected_edges: &mut Vec<EdgeSelection>,
        selected_patch_vertices: &mut Vec<PatchVertexSelection>,
        selected_entities: &mut Vec<usize>,
        edit_faces: bool,
        edit_vertices: bool,
        edit_edges: bool,
        map: &mut Option<kradiant::map::Map>,
        ent_draw_config: &kradiant::editor::config::EntityDrawingConfig,
        selection_rgba: [f32; 4],
        rotate_mode: bool,
        ortho_axis: Ortho,
    ) {
        use dear_imgui_rs::MouseButton;

        ui.window("3D View")
            .size([640.0, 480.0], Condition::FirstUseEver)
            .flags(WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE)
            .build(|| {
                ui.text("FOV");
                ui.same_line();
                ui.set_next_item_width(80.0);
                ui.slider_config("##fov", 40.0f32, 120.0f32)
                    .display_format("%.0f°")
                    .build(&mut config.view.fov);

                ui.same_line();
                if ui.button("Reset") {
                    self.cam.pos = Vec3::ZERO;
                    self.cam.angles = Vec3::new(0.8, -0.35, 0.0);
                    self.cam.zoom = 64.0;
                }

                ui.separator();

                let [w, h] = ui.content_region_avail();
                let (w, h) = (w.max(1.0), h.max(1.0));
                let p = ui.cursor_screen_pos();
                let draw = ui.get_window_draw_list();

                self.rect = [p[0], p[1], w, h];

                ui.set_cursor_screen_pos(p);
                ui.invisible_button("##3d_canvas", [w, h]);
                let canvas_interacting = ui.is_item_hovered() || ui.is_item_active();

                if canvas_interacting && ui.is_mouse_clicked(MouseButton::Right) {
                    self.right_dragging = true;
                    self.last_cursor_pos = Some(ui.io().mouse_pos());
                    self.accumulated_mouse_delta = [0.0, 0.0];
                }
                if !ui.is_mouse_down(MouseButton::Right) {
                    self.right_dragging = false;
                    self.last_cursor_pos = None;
                }
                self.wants_cursor_grab = self.right_dragging;

                let mut common_vp = Mat4::ZERO;

                if canvas_interacting {
                    if !self.right_dragging {
                        self.accumulated_mouse_delta = [0.0, 0.0];
                    }

                    let wheel = ui.io().mouse_wheel();
                    if wheel != 0.0 {
                        let f = if wheel > 0.0 { 1.15f32 } else { 1.0 / 1.15 };
                        self.cam.zoom = (self.cam.zoom * f).clamp(1.0, 65_536.0);
                    }

                    let ctrl = ui.is_key_down(dear_imgui_rs::Key::LeftCtrl) || ui.is_key_down(dear_imgui_rs::Key::RightCtrl);

                    {
                        let up_pressed = ui.is_key_down(Key::UpArrow);
                        let dn_pressed = ui.is_key_down(Key::DownArrow);
                        let lf_pressed = ui.is_key_down(Key::LeftArrow);
                        let rg_pressed = ui.is_key_down(Key::RightArrow);

                        let forward = Self::forward_from_angles(self.cam.angles);
                        let right = Self::right_from_angles(self.cam.angles);

                        let mut move_dir = Vec3::ZERO;
                        if up_pressed {
                            move_dir += forward;
                        }
                        if dn_pressed {
                            move_dir -= forward;
                        }
                        if lf_pressed {
                            move_dir -= right;
                        }
                        if rg_pressed {
                            move_dir += right;
                        }

                        // Normalize only if there's input
                        if move_dir.length_squared() > 1e-6 {
                            move_dir = move_dir.normalize();
                        }

                        let mut move_amt = self.cam.zoom * MOVE_SENS;
                        if ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                            || ui.is_key_down(dear_imgui_rs::Key::RightShift)
                        {
                            move_amt *= 3.0;
                        }
                        if ctrl {
                            let mut move_dir = Vec3::ZERO;
                            let forward = Vec3::new(0.0, 1.0, 0.0);
                            let right = Vec3::new(-1.0, 0.0, 0.0);
                            if up_pressed {
                                move_dir += forward;
                            }
                            if dn_pressed {
                                move_dir -= forward;
                            }
                            if lf_pressed {
                                move_dir -= right;
                            }
                            if rg_pressed {
                                move_dir += right;
                            }
                            // println!("move dir: {move_dir}");
                            self.cam.angles += move_dir * move_amt * 0.05;
                        }
                        else {
                            self.cam.pos += move_dir * move_amt * 1.3; // felt too slow
                        }
                    }

                    const TURN_SENS: f32 = 0.010;
                    const PITCH_SENS: f32 = 0.010;
                    const MOVE_SENS: f32 = 0.030;
                    if self.right_dragging {
                        let dx = self.accumulated_mouse_delta[0];
                        let dy = self.accumulated_mouse_delta[1];
                        self.accumulated_mouse_delta = [0.0, 0.0];

                        self.cam.angles.x -= dx * TURN_SENS;

                        if ctrl {
                            self.cam.angles.y -= dy * PITCH_SENS;
                            self.cam.angles.y = self.cam.angles.y.clamp(-1.55, 1.55);
                        }

                        let forward = Self::forward_from_angles(self.cam.angles);
                        let flat = Vec3::new(forward.x, forward.y, 0.0);
                        let move_dir = if flat.length_squared() > 1e-6 {
                            flat.normalize()
                        } else {
                            forward
                        };

                        if !ctrl {
                            let mut move_amt = (-dy) * self.cam.zoom * MOVE_SENS;
                            if ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                                || ui.is_key_down(dear_imgui_rs::Key::RightShift)
                            {
                                move_amt *= 3.0;
                            }
                            self.cam.pos += move_dir * move_amt;
                        }
                    }

                    if ui.is_key_pressed(dear_imgui_rs::Key::D) {
                        self.cam.pos.z += self.cam.zoom / 2.0;
                    }
                    if ui.is_key_pressed(dear_imgui_rs::Key::C) {
                        self.cam.pos.z -= self.cam.zoom / 2.0;
                    }

                    let forward = Self::forward_from_angles(self.cam.angles);
                    let view = Mat4::look_at_rh(self.cam.pos, self.cam.pos + forward, Vec3::Z);
                    let proj = Mat4::perspective_rh_gl(
                        config.view.fov.to_radians(),
                        w / h,
                        4.0,
                        100_000.0,
                    );
                    common_vp = proj * view;

                    if canvas_interacting && ui.is_key_pressed(dear_imgui_rs::Key::Tab) {
                        // TAB key handling - could be used for other purposes if needed
                    }

                    let mouse = ui.io().mouse_pos();
                    let mut mask = PickMask::NONE;
                    if config.view.show.convex {
                        mask.add(PickMask::CONVEX);
                    }
                    if config.view.show.patches {
                        mask.add(PickMask::PATCH);
                    }
                    if config.view.show.clip_brushes {
                        mask.add(PickMask::CLIP);
                    }
                    if config.view.show.portal_brushes {
                        mask.add(PickMask::PORTAL);
                    }
                    if config.view.show.hint_brushes {
                        mask.add(PickMask::HINT);
                    }

                    let shift_selecting = canvas_interacting
                        && (ui.is_mouse_clicked(MouseButton::Left)
                            || ui.is_mouse_dragging(MouseButton::Left))
                        && ui.is_key_down(dear_imgui_rs::Key::LeftShift);

                    if shift_selecting && ui.is_mouse_clicked(MouseButton::Left) {
                        // Start new selection drag - clear touched sets
                        self.selection_drag_touched = Some(HashSet::new());
                        self.selection_drag_touched_faces = Some(HashSet::new());
                        self.selection_drag_touched_edges = Some(HashSet::new());
                        self.selection_drag_touched_verts = Some(HashSet::new());
                        self.selection_drag_touched_entities = Some(HashSet::new());
                    }
                    if shift_selecting {
                        let ray_far = 1.0e6;
                        let [rx, ry, rw, rh] = self.rect;
                        let nx = (mouse[0] - rx) / rw * 2.0 - 1.0;
                        let ny = 1.0 - (mouse[1] - ry) / rh * 2.0;

                        let aspect = rw / rh;
                        let proj = Mat4::perspective_rh(
                            config.view.fov.to_radians(),
                            aspect,
                            1.0,
                            ray_far,
                        );

                        let forward = Self::forward_from_angles(self.cam.angles);
                        let up = Vec3::Z;
                        let view = Mat4::look_at_rh(self.cam.pos, self.cam.pos + forward, up);

                        let inv_vp = (proj * view).inverse();

                        let target_world = inv_vp.project_point3(Vec3::new(nx, ny, 1.0));

                        let ray_origin = self.cam.pos;
                        let ray_dir = (target_world - ray_origin).normalize();

                        if edit_faces {
                            let selected_face = map.as_mut().and_then(|m| {
                                editing::pick_convex_face_by_ray(m, ray_origin, ray_dir, mask)
                            });

                            if let Some((entity_idx, brush_idx, face_idx)) = selected_face {
                                let sel = FaceSelection {
                                    entity_idx,
                                    brush_idx,
                                    face_idx,
                                };
                                let touched = self
                                    .selection_drag_touched_faces
                                    .get_or_insert_with(HashSet::new);
                                if touched.insert(sel) {
                                    if !selected_faces.contains(&sel) {
                                        selected_faces.push(sel);
                                    } else if let Some(i) =
                                        selected_faces.iter().position(|f| *f == sel)
                                    {
                                        selected_faces.remove(i);
                                    }
                                    view2d::sync_selected_brushes_from_faces(
                                        selected_faces,
                                        selected_brushes,
                                    );
                                    if let Some(aabb) = view2d::selection_aabb_active(map, selected_brushes, selected_faces, selected_edges, selected_entities, edit_faces, edit_edges, ent_draw_config) {
                                        self.last_aabb = Some(aabb.clone());
                                        self.update_work_from_aabb(aabb);
                                    }
                                }
                            }
                        } else if edit_edges {
                            let vp = proj * view;
                            let prefer: Vec<(usize, usize)> = {
                                let mut set = std::collections::BTreeSet::new();
                                for sel in selected_edges.iter() {
                                    set.insert((sel.entity_idx, sel.brush_idx));
                                }
                                set.into_iter().collect()
                            };
                            let selected_edge = map.as_mut().and_then(|m| {
                                pick_convex_edge_by_screen_3d(
                                    m,
                                    (!prefer.is_empty()).then_some(&prefer[..]),
                                    mask,
                                    vp,
                                    ray_origin,
                                    ray_dir,
                                    mouse,
                                    self.rect,
                                    4.0,
                                )
                            });

                            if let Some(sel) = selected_edge {
                                let touched = self
                                    .selection_drag_touched_edges
                                    .get_or_insert_with(HashSet::new);
                                if touched.insert(sel) {
                                    let z_held = ui.is_key_down(dear_imgui_rs::Key::Z);
                                    if z_held {
                                        // Z: subtract only (like brush
                                        // rect-select in the 2D view).
                                        if let Some(i) =
                                            selected_edges.iter().position(|e| *e == sel)
                                        {
                                            selected_edges.remove(i);
                                        }
                                    } else if !selected_edges.contains(&sel) {
                                        selected_edges.push(sel);
                                    } else if let Some(i) =
                                        selected_edges.iter().position(|e| *e == sel)
                                    {
                                        selected_edges.remove(i);
                                    }
                                    view2d::sync_selected_brushes_from_edges(
                                        selected_edges,
                                        selected_brushes,
                                    );
                                    if let Some(aabb) = view2d::selection_aabb_active(map, selected_brushes, selected_faces, selected_edges, selected_entities, edit_faces, edit_edges, ent_draw_config) {
                                        self.last_aabb = Some(aabb.clone());
                                        self.update_work_from_aabb(aabb);
                                    }
                                }
                            }
                        } else if edit_vertices {
                            let vp = proj * view;
                            let prefer =
                                (!selected_brushes.is_empty()).then_some(&selected_brushes[..]);
                            let selected_vert = map.as_mut().and_then(|m| {
                                pick_patch_control_vertex_by_screen_3d(
                                    m, prefer, vp, mouse, self.rect, 10.0,
                                )
                            });

                            if let Some(vsel) = selected_vert {
                                let touched = self
                                    .selection_drag_touched_verts
                                    .get_or_insert_with(HashSet::new);
                                if touched.insert(
                                    vsel.entity_idx
                                        ^ (vsel.brush_idx << 8)
                                        ^ (vsel.row << 16)
                                        ^ (vsel.col << 24),
                                ) {
                                    let z_held = ui.is_key_down(dear_imgui_rs::Key::Z);
                                    if z_held {
                                        if let Some(i) =
                                            selected_patch_vertices.iter().position(|s| *s == vsel)
                                        {
                                            selected_patch_vertices.remove(i);
                                        } else {
                                            selected_patch_vertices.push(vsel);
                                        }
                                    } else if !selected_patch_vertices.contains(&vsel) {
                                        selected_patch_vertices.push(vsel);
                                    }
                                    if let Some(aabb) = view2d::selection_aabb_active(map, selected_brushes, selected_faces, selected_edges, selected_entities, edit_faces, edit_edges, ent_draw_config) {
                                        self.last_aabb = Some(aabb.clone());
                                        self.update_work_from_aabb(aabb);
                                    }
                                }
                            }
                        } else {
                            // Use unified picking to get the closest brush or entity
                            let selected = map.as_mut().and_then(|m| {
                                editing::pick_brush_or_ent_by_ray(
                                    m,
                                    ray_origin,
                                    ray_dir,
                                    mask,
                                    ent_draw_config,
                                    None,
                                    None,
                                )
                            });

                            if let Some(sel) = selected {
                                match sel {
                                    Ok((ent_idx, brush_idx)) => {
                                        // Brush hit
                                        let touched = self
                                            .selection_drag_touched
                                            .get_or_insert_with(HashSet::new);
                                        if touched.insert((ent_idx, brush_idx)) {
                                            if !selected_brushes.contains(&(ent_idx, brush_idx)) {
                                                selected_brushes.push((ent_idx, brush_idx));
                                            } else if let Some(i) = selected_brushes
                                                .iter()
                                                .position(|b| b == &(ent_idx, brush_idx))
                                            {
                                                selected_brushes.remove(i);
                                            }
                                        }
                                        selected_faces.clear();
                                        if let Some(aabb) = view2d::selection_aabb_active(map, selected_brushes, selected_faces, selected_edges, selected_entities, edit_faces, edit_edges, ent_draw_config) {
                                            self.last_aabb = Some(aabb.clone());
                                            self.update_work_from_aabb(aabb);
                                        }
                                    }
                                    Err(ent_idx) => {
                                        // Entity hit
                                        let touched = self
                                            .selection_drag_touched_entities
                                            .get_or_insert_with(HashSet::new);
                                        if touched.insert(ent_idx) {
                                            if !selected_entities.contains(&ent_idx) {
                                                selected_entities.push(ent_idx);
                                            } else if let Some(i) = selected_entities
                                                .iter()
                                                .position(|e| *e == ent_idx)
                                            {
                                                selected_entities.remove(i);
                                            }
                                        }
                                        if let Some(aabb) = view2d::selection_aabb_active(map, selected_brushes, selected_faces, selected_edges, selected_entities, edit_faces, edit_edges, ent_draw_config) {
                                            self.last_aabb = Some(aabb.clone());
                                            self.update_work_from_aabb(aabb);
                                        }
                                    }
                                }
                            }
                        } // end outer else (not edit_faces/edges/vertices)
                    } // end if shift_selecting
                    if ui.is_mouse_clicked(MouseButton::Left)
                        && !ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                        && ui.is_key_down(dear_imgui_rs::Key::LeftAlt)
                    {
                        self.drag_mode = DragMode::RectangularSelection;
                    }

                    // Handle left-click drag for transformations
                    if ui.is_mouse_clicked(MouseButton::Left)
                        && !ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                        && !ui.is_key_down(dear_imgui_rs::Key::LeftAlt)
                    {
                        if edit_edges {
                            // Edge edit mode, brush-like: the selection itself
                            // is shift+click (Z subtracts); a plain drag moves
                            // the current edge selection from wherever the
                            // user grabs it.
                            if !selected_edges.is_empty() {
                                let forward = Self::forward_from_angles(self.cam.angles);
                                let ray =
                                    self.screen_to_ray(Vec2::new(mouse[0], mouse[1]), config);
                                // Anchor at the frontmost brush surface under
                                // the cursor (or through the selection center
                                // when pointing at the sky).
                                let anchor = map
                                    .as_mut()
                                    .and_then(|m| {
                                        ray_front_brush_hit_point(m, self.cam.pos, ray, mask)
                                    })
                                    .unwrap_or_else(|| {
                                        let t = view2d::selection_aabb_active(
                                            map,
                                            selected_brushes,
                                            selected_faces,
                                            selected_edges,
                                            selected_entities,
                                            edit_faces,
                                            edit_edges,
                                            ent_draw_config,
                                        )
                                        .map(|aabb| {
                                            ray.dot((aabb.min + aabb.max) * 0.5 - self.cam.pos)
                                                .max(64.0)
                                        })
                                        .unwrap_or(256.0);
                                        self.cam.pos + ray * t
                                    });
                                self.drag_mode = DragMode::MoveEdges;
                                self.drag_start = Some(Vec2::new(mouse[0], mouse[1]));
                                self.drag_current = Some(Vec2::new(mouse[0], mouse[1]));
                                self.move_offset = Vec3::ZERO;
                                self.stretch = None;
                                self.stretch_delta = Vec3::ZERO;
                                self.rotate = None;
                                self.rotate_angle = 0.0;
                                self.drag_world_anchor = anchor;
                                self.drag_plane_normal = forward;
                                self.drag_plane_origin = anchor;
                            }
                        } else if !selected_brushes.is_empty() || !selected_entities.is_empty() {
                            self.drag_start = Some(Vec2::new(mouse[0], mouse[1]));
                            self.drag_current = Some(Vec2::new(mouse[0], mouse[1]));
                            self.move_offset = Vec3::ZERO;
                            self.stretch = None;
                            self.stretch_delta = Vec3::ZERO;

                            let forward = Self::forward_from_angles(self.cam.angles);
                            let ray = self.screen_to_ray(Vec2::new(mouse[0], mouse[1]), config);

                            let selection_aabb = if let Some(map) = map {
                                view2d::selection_aabb(
                                    map,
                                    selected_brushes,
                                    selected_entities,
                                    ent_draw_config,
                                )
                            } else {
                                None
                            };

                            // Does the click ray hit any selected brush/entity?
                            // Hit -> drag the whole selection (q3 Drag_Setup).
                            let brush_hit_t =
                                selected_brushes
                                    .iter()
                                    .filter_map(|&(entity_idx, brush_idx)| {
                                        let entity = map.as_mut()?.entities.get_mut(entity_idx)?;
                                        let brush = entity.brushes.get_mut(brush_idx)?;
                                        let (_aabb, polys) = brush.get_polygons_and_aabb()?;
                                        let mut best: Option<f32> = None;
                                        for (positions, indices) in polys {
                                            for tri in indices.chunks_exact(3) {
                                                let (i0, i1, i2) = (
                                                    tri[0] as usize,
                                                    tri[1] as usize,
                                                    tri[2] as usize,
                                                );
                                                if let Some(t) = Self::ray_triangle_t(
                                                    self.cam.pos,
                                                    ray,
                                                    positions[i0],
                                                    positions[i1],
                                                    positions[i2],
                                                ) {
                                                    if t > 0.0 {
                                                        best = Some(
                                                            best.map_or(t, |b: f32| b.min(t)),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        best
                                    })
                                    .min_by(|a, b| a.total_cmp(b));
                            let entity_hit_t = map.as_ref().and_then(|map| {
                                selected_entities
                                    .iter()
                                    .filter_map(|&ent_idx| {
                                        let entity = map.entities.get(ent_idx)?;
                                        let aabb =
                                            view2d::entity_selection_aabb(entity, ent_draw_config)?;
                                        let (t_enter, t_exit) = Self::ray_aabb_intersection(
                                            aabb.min,
                                            aabb.max,
                                            self.cam.pos,
                                            ray,
                                        )?;
                                        (t_exit >= 0.0).then_some(if t_enter >= 0.0 {
                                            t_enter
                                        } else {
                                            0.0
                                        })
                                    })
                                    .min_by(|a, b| a.total_cmp(b))
                            });

                            let hit_t = if brush_hit_t.is_some() || entity_hit_t.is_some() {
                                if rotate_mode && !edit_edges {
                                    // Rotation: use the same axis as the 2D view
                                    let axis = match ortho_axis {
                                        Ortho::XY => Vec3::Z,
                                        Ortho::XZ => Vec3::Y,
                                        Ortho::YZ => Vec3::X,
                                    };
                                    // For entities with models, rotate around entity origin
                                    let pivot = if selected_brushes.is_empty()
                                        && !selected_entities.is_empty()
                                    {
                                        let mut origin_sum = Vec3::ZERO;
                                        let mut count = 0u32;
                                        if let Some(map) = map.as_ref() {
                                            for &ent_idx in selected_entities.iter() {
                                                if ent_idx == 0 {
                                                    continue;
                                                }
                                                if let Some(entity) =
                                                    map.entities.get(ent_idx)
                                                {
                                                    if entity.model.is_some() {
                                                        let origin = entity
                                                            .properties
                                                            .get("origin")
                                                            .map(|s| {
                                                                core_util::origin_to_vec3(s)
                                                            })
                                                            .unwrap_or(Vec3::ZERO);
                                                        origin_sum += origin;
                                                        count += 1;
                                                    }
                                                }
                                            }
                                        }
                                        if count > 0 {
                                            Some(origin_sum / count as f32)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    };
                                    self.rotate = Some(RotateDrag {
                                        selection_aabb: selection_aabb.unwrap_or(Aabb {
                                            min: Vec3::ZERO,
                                            max: Vec3::ZERO,
                                        }),
                                        pivot_uv: [0.0, 0.0], // Not used in 3D
                                        start_uv: [0.0, 0.0], // Not used in 3D
                                        axis,
                                        pivot,
                                    });
                                    self.drag_mode = DragMode::RotateSelection;
                                    match (brush_hit_t, entity_hit_t) {
                                        (Some(a), Some(b)) => a.min(b),
                                        (Some(t), None) | (None, Some(t)) => t,
                                        (None, None) => 1.0,
                                    }
                                } else {
                                    self.drag_mode = DragMode::MoveSelection;
                                    match (brush_hit_t, entity_hit_t) {
                                        (Some(a), Some(b)) => a.min(b),
                                        (Some(t), None) | (None, Some(t)) => t,
                                        (None, None) => 1.0,
                                    }
                                }
                            } else {
                                // q3radiant Brush_SideSelect: the ray missed
                                // the selection — grab every face of every
                                // selected brush whose OUTER side the ray
                                // passes while staying inside all the other
                                // planes.
                                let mut side_faces: Vec<(usize, usize, Vec<usize>)> = Vec::new();
                                if let Some(map) = map.as_ref() {
                                    for &(entity_idx, brush_idx) in selected_brushes.iter() {
                                        let Some(entity) = map.entities.get(entity_idx) else {
                                            continue;
                                        };
                                        let Some(brush) = entity.brushes.get(brush_idx) else {
                                            continue;
                                        };
                                        if !matches!(
                                            brush.content,
                                            kradiant::map::BrushContent::Convex(_)
                                        ) {
                                            continue;
                                        }
                                        let faces =
                                            editing::side_select_faces(brush, self.cam.pos, ray);
                                        if !faces.is_empty() {
                                            side_faces.push((entity_idx, brush_idx, faces));
                                        }
                                    }
                                }
                                if !side_faces.is_empty() {
                                    let aabb = selection_aabb.unwrap_or_else(|| Aabb {
                                        min: Vec3::ZERO,
                                        max: Vec3::ZERO,
                                    });
                                    // Compute cached avg_normal and face_center from side_faces
                                    let (avg_normal, face_center) = if let Some(map_mut) = map.as_mut() {
                                        let mut normal_sum = Vec3::ZERO;
                                        let mut normal_count = 0u32;
                                        let mut center_sum = Vec3::ZERO;
                                        let mut center_count = 0u32;
                                        for (ent_idx, br_idx, face_indices) in &side_faces {
                                            if let Some(entity) = map_mut.entities.get_mut(*ent_idx) {
                                                if let Some(brush) = entity.brushes.get_mut(*br_idx) {
                                                    if let Some(polys) = brush.get_polygons() {
                                                        for &face_idx in face_indices {
                                                            if face_idx < polys.len() {
                                                                let (positions, _) = &polys[face_idx];
                                                                // Compute normal
                                                                if positions.len() >= 3 {
                                                                    let e1 = positions[1] - positions[0];
                                                                    let e2 = positions[2] - positions[0];
                                                                    let n = e1.cross(e2).normalize_or_zero();
                                                                    normal_sum += n;
                                                                    normal_count += 1;
                                                                }
                                                                // Compute center
                                                                if !positions.is_empty() {
                                                                    for pos in positions {
                                                                        center_sum += *pos;
                                                                        center_count += 1;
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        (
                                            if normal_count > 0 {
                                                normal_sum.normalize_or_zero()
                                            } else {
                                                Vec3::ZERO
                                            },
                                            if center_count > 0 {
                                                center_sum / center_count as f32
                                            } else {
                                                Vec3::ZERO
                                            },
                                        )
                                    } else {
                                        (Vec3::ZERO, Vec3::ZERO)
                                    };
                                    self.stretch = Some(SideStretchDrag {
                                        selection_aabb: aabb,
                                        side_faces,
                                        avg_normal,
                                        face_center,
                                    });
                                    self.stretch_delta = Vec3::ZERO;
                                    self.drag_mode = DragMode::StretchSelection;
                                } else {
                                    // No facing plane grabbed: drag the whole
                                    // selection like a normal move.
                                    self.drag_mode = DragMode::MoveSelection;
                                }
                                let center = selection_aabb
                                    .map(|aabb| (aabb.min + aabb.max) * 0.5)
                                    .unwrap_or(Vec3::ZERO);
                                let denom = forward.dot(ray);
                                if denom.abs() > 1e-6 {
                                    forward.dot(center - self.cam.pos) / denom
                                } else {
                                    1.0
                                }
                            };

                            self.drag_world_anchor = self.cam.pos + ray * hit_t;
                            self.drag_plane_normal = forward;
                            self.drag_plane_origin = self.drag_world_anchor;
                        }
                    }

                    if self.drag_start.is_some()
                        && ui.is_mouse_down(MouseButton::Left)
                        && !ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                    {
                        if self.drag_mode == DragMode::MoveEdges
                            || !selected_brushes.is_empty()
                            || !selected_entities.is_empty()
                        {

                            let current = Vec2::new(mouse[0], mouse[1]);

                            let ray = self.screen_to_ray(current, config);
                            let denom = self.drag_plane_normal.dot(ray);
                            if denom.abs() > 1e-6 {
                                let t = self
                                    .drag_plane_normal
                                    .dot(self.drag_plane_origin - self.cam.pos)
                                    / denom;
                                let world_current = self.cam.pos + ray * t;
                                let raw_offset = world_current - self.drag_world_anchor;
                                self.drag_current = Some(current);

                                let my_snapping = if ctrl {
                                    !config.view.grid_snap
                                } else {
                                    config.view.grid_snap
                                };
                                match self.drag_mode {
                                    DragMode::MoveSelection | DragMode::MoveEdges => {
                                        let mut offset = raw_offset;
                                        if my_snapping {
                                            let grid =
                                                config.view.grid_minor_step.max(1) as f32;
                                            offset.x = (offset.x / grid).round() * grid;
                                            offset.y = (offset.y / grid).round() * grid;
                                            offset.z = (offset.z / grid).round() * grid;
                                        }
                                        self.move_offset = offset;
                                    }
                                    DragMode::StretchSelection => {
                                        let mut delta = raw_offset;

                                        if my_snapping {
                                            let grid =
                                                config.view.grid_minor_step.max(1) as f32;
                                            delta.x = (delta.x / grid).round() * grid;
                                            delta.y = (delta.y / grid).round() * grid;
                                            delta.z = (delta.z / grid).round() * grid;
                                        }
                                        // Constrain delta to face normals using cached avg_normal
                                        if let Some(stretch) = self.stretch.as_ref() {
                                            let avg_normal = stretch.avg_normal;
                                            if avg_normal != Vec3::ZERO {
                                                // Project delta onto average normal
                                                let projected = avg_normal * delta.dot(avg_normal);
                                                delta = projected;
                                            }
                                        }
                                        self.stretch_delta = delta;
                                    }
                                    DragMode::RotateSelection => {
                                        if let Some(_rot) = self.rotate.as_ref() {
                                            // Calculate rotation angle from mouse drag
                                            // Use horizontal mouse movement as the rotation input
                                            let [rx, _ry, rw, _rh] = self.rect;
                                            let _center_x = rx + rw * 0.5;
                                            let start = self.drag_start.unwrap_or(current);
                                            let dx = current.x - start.x;
                                            // Convert pixels to radians (similar to 2D view)
                                            let angle = -dx * 0.01;
                                            let mut angle = angle;
                                            if my_snapping {
                                                angle = angle.to_degrees().round().to_radians();
                                            }
                                            self.rotate_angle = angle;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    } else {
                        // Mouse released or not dragging - apply accumulated changes
                        if self.drag_start.is_some() {
                            match self.drag_mode {
                                DragMode::MoveSelection => {
                                    if self.move_offset != Vec3::ZERO {
                                        if map.is_some() {
                                            undo.push(
                                                "Move selection",
                                                map,
                                                selected_brushes,
                                                selected_faces,
                                                selected_edges,
                                                selected_patch_vertices,
                                                selected_entities,
                                            );

                                            if let Some(map) = map.as_mut() {
                                                let delta = self.move_offset;
                                                let generation = &mut map.generation;
                                                for (entity_idx, brush_idx) in
                                                    selected_brushes.clone()
                                                {
                                                    if let Some(entity) =
                                                        map.entities.get_mut(entity_idx)
                                                    {
                                                        if let Some(brush) =
                                                            entity.brushes.get_mut(brush_idx)
                                                        {
                                                            brush.translate(generation, delta);
                                                        }
                                                    }
                                                }
                                                for ent_idx in selected_entities.iter().copied() {
                                                    if ent_idx == 0 {
                                                        continue;
                                                    }
                                                    if selected_brushes
                                                        .iter()
                                                        .any(|(entity_idx, _)| {
                                                            *entity_idx == ent_idx
                                                        })
                                                    {
                                                        continue;
                                                    }
                                                    if let Some(entity) =
                                                        map.entities.get_mut(ent_idx)
                                                    {
                                                        entity.translate(generation, delta);
                                                    }
                                                }
                                            }

                                            crate::log_info!(console, "Dragged selection in 3D View");
                                            if let Some(aabb) = view2d::selection_aabb_active(map, selected_brushes, selected_faces, selected_edges, selected_entities, edit_faces, edit_edges, ent_draw_config) {
                                                self.update_work_from_aabb(aabb);
                                            }
                                        }
                                    }
                                }
                                DragMode::StretchSelection => {
                                    if let Some(stretch) = self.stretch.take() {
                                        let delta = self.stretch_delta;
                                        if delta != Vec3::ZERO && map.is_some() {
                                            // Validate EVERY brush first: q3 refuses
                                            // the whole drag if any would degenerate
                                            // ("Brush dragged backwards, move canceled").
                                            let mut valid = true;
                                            if let Some(map_ref) = map.as_ref() {
                                                for (entity_idx, brush_idx, indices) in
                                                    &stretch.side_faces
                                                {
                                                    let Some(entity) =
                                                        map_ref.entities.get(*entity_idx)
                                                    else {
                                                        valid = false;
                                                        break;
                                                    };
                                                    let Some(brush) =
                                                        entity.brushes.get(*brush_idx)
                                                    else {
                                                        valid = false;
                                                        break;
                                                    };
                                                    let mut probe = brush.clone();
                                                    let kradiant::map::BrushContent::Convex(
                                                        probe_faces,
                                                    ) = &mut probe.content
                                                    else {
                                                        valid = false;
                                                        break;
                                                    };
                                                    for idx in indices {
                                                        let Some(face) =
                                                            probe_faces.get_mut(*idx)
                                                        else {
                                                            valid = false;
                                                            break;
                                                        };
                                                        for p in &mut face.plane_points {
                                                            *p += delta;
                                                        }
                                                    }
                                                    if valid
                                                        && !editing::is_convex_brush_valid(&probe)
                                                    {
                                                        valid = false;
                                                    }
                                                    if !valid {
                                                        break;
                                                    }
                                                }
                                            }
                                            if !valid {
                                                crate::log_info!(
                                                    console,
                                                    "Side stretch refused — a brush would degenerate"
                                                );
                                            } else {
                                                undo.push(
                                                    "Side stretch",
                                                    map,
                                                    selected_brushes,
                                                    selected_faces,
                                                    selected_edges,
                                                    selected_patch_vertices,
                                                    selected_entities,
                                                );

                                                if let Some(map_ref) = map.as_mut() {
                                                    let mut any = false;
                                                    for (entity_idx, brush_idx, indices) in
                                                        &stretch.side_faces
                                                    {
                                                        let Some(entity) =
                                                            map_ref.entities.get_mut(*entity_idx)
                                                        else {
                                                            continue;
                                                        };
                                                        let Some(brush) =
                                                            entity.brushes.get_mut(*brush_idx)
                                                        else {
                                                            continue;
                                                        };
                                                        if editing::stretch_brush_side_faces(
                                                            brush,
                                                            &mut map_ref.generation,
                                                            indices,
                                                            delta,
                                                        ) {
                                                            any = true;
                                                        }
                                                    }

                                                    if any {
                                                        crate::log_info!(console, "Side stretch in 3D View");
                                                        if let Some(aabb) = view2d::selection_aabb_active(map, selected_brushes, selected_faces, selected_edges, selected_entities, edit_faces, edit_edges, ent_draw_config) {
                                                            self.update_work_from_aabb(aabb);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    self.stretch_delta = Vec3::ZERO;
                                }
                                DragMode::RotateSelection => {
                                    if let Some(rot) = self.rotate.take() {
                                        let angle = self.rotate_angle;
                                        if angle.abs() > 1.0e-6 {
                                            let has_selection = if edit_faces {
                                                !selected_faces.is_empty()
                                            } else if edit_edges {
                                                !selected_edges.is_empty()
                                            } else {
                                                !selected_brushes.is_empty() || !selected_entities.is_empty()
                                            };
                                            if map.is_some() && has_selection {
                                                let label = if edit_faces { "Rotate faces" } else { "Rotate selection" };
                                                undo.push(
                                                    label,
                                                    map,
                                                    selected_brushes,
                                                    selected_faces,
                                                    selected_edges,
                                                    selected_patch_vertices,
                                                    selected_entities,
                                                );
                                            }
                                            let axis = rot.axis;
                                            let mut new_sel_aabb: Option<Aabb> = None;
                                            if let Some(map) = map.as_mut() {
                                                if edit_faces {
                                                    let any = apply_affine_rotate_to_selected_faces(
                                                        map,
                                                        selected_faces.as_mut_slice(),
                                                        axis,
                                                        angle,
                                                    );
                                                    if any {
                                                        new_sel_aabb =
                                                            selection_aabb_faces_from_map(map, selected_faces);
                                                        crate::log_info!(console, "Rotated selected faces");
                                                    }
                                                } else if let Some((xform, _preview)) =
                                                    editing::rotate_selection_transform(
                                                        &rot.selection_aabb,
                                                        axis,
                                                        angle,
                                                        rot.pivot,
                                                    )
                                                {
                                                    let mut any = false;
                                                    for (entity_idx, brush_idx) in selected_brushes.iter() {
                                                        let Some(entity) = map.entities.get_mut(*entity_idx) else {
                                                            continue;
                                                        };
                                                        let Some(brush) = entity.brushes.get_mut(*brush_idx) else {
                                                            continue;
                                                        };
                                                        if editing::apply_affine_rotate_to_brush(
                                                            brush,
                                                            &mut map.generation,
                                                            xform,
                                                        ) {
                                                            any = true;
                                                        }
                                                    }

                                                    let mut any_entity = false;
                                                    for ent_idx in selected_entities.iter().copied() {
                                                        if ent_idx == 0 {
                                                            continue;
                                                        }
                                                        let Some(entity) = map.entities.get_mut(ent_idx) else {
                                                            continue;
                                                        };
                                                        let angles = entity
                                                            .properties
                                                            .get("angles")
                                                            .and_then(|s| core_util::vec3_from_whitespace_triplet(s))
                                                            .unwrap_or(Vec3::ZERO);
                                                        let q_new = glam::Quat::from_axis_angle(axis, angle)
                                                            * core_util::entity_angles_to_quat(angles);
                                                        let new_angles = core_util::quat_to_entity_angles(q_new);
                                                        entity.properties.insert(
                                                            "angles".to_string(),
                                                            core_util::vec3_to_angles(new_angles),
                                                        );
                                                        map.generation = map.generation.wrapping_add(1);
                                                        any_entity = true;
                                                    }

                                                    if any || any_entity {
                                                        new_sel_aabb = selection_aabb_from_map(map, &selected_brushes, &selected_entities, ent_draw_config);
                                                        crate::log_info!(console, "Rotated selection");
                                                    }
                                                }
                                            }
                                            if let Some(aabb) = new_sel_aabb {
                                                self.last_aabb = Some(aabb.clone());
                                                self.update_work_from_aabb(aabb);
                                            }
                                        }
                                    }
                                    self.rotate_angle = 0.0;
                                }
                                DragMode::MoveEdges => {
                                    let delta = self.move_offset;
                                    if delta != Vec3::ZERO && !selected_edges.is_empty() {
                                        // The move gets clamped to the largest
                                        // fraction every affected brush
                                        // tolerates; refuse only when nothing
                                        // can move at all.
                                        let factor = map
                                            .as_ref()
                                            .map(|m| editing::edge_move_clamp_factor(
                                                m,
                                                selected_edges,
                                                delta,
                                            ))
                                            .unwrap_or(0.0);
                                        if factor <= 1.0e-4 {
                                            crate::log_info!(
                                                console,
                                                "Edge move refused — a brush would degenerate"
                                            );
                                        } else {
                                            undo.push(
                                                "Move edges",
                                                map,
                                                selected_brushes,
                                                selected_faces,
                                                selected_edges,
                                                selected_patch_vertices,
                                                selected_entities,
                                            );

                                            if let Some(map_ref) = map.as_mut() {
                                                if editing::translate_selected_edges(
                                                    map_ref,
                                                    selected_edges,
                                                    delta,
                                                ) {
                                                    crate::log_info!(
                                                        console,
                                                        "Moved selected edges in 3D View"
                                                    );
                                                }
                                            }

                                            if let Some(aabb) = view2d::selection_aabb_active(
                                                map,
                                                selected_brushes,
                                                selected_faces,
                                                selected_edges,
                                                selected_entities,
                                                edit_faces,
                                                edit_edges,
                                                ent_draw_config,
                                            ) {
                                                self.last_aabb = Some(aabb.clone());
                                                self.update_work_from_aabb(aabb);
                                            }
                                        }
                                    }
                                    self.move_offset = Vec3::ZERO;
                                }
                                _ => {}
                            }
                        }
                        self.drag_start = None;
                        self.drag_current = None;
                        self.move_offset = Vec3::ZERO;
                    }
                    if let Some(aabb) = view2d::selection_aabb_active(map, selected_brushes, selected_faces, selected_edges, selected_entities, edit_faces, edit_edges, ent_draw_config) {
                        self.update_work_from_aabb(aabb);
                    }
                }

                if !ui.is_item_hovered() {
                    // if mouse no longer hovers on 3d view, reset drag state
                    if self.drag_start.is_some() {
                        self.drag_start = None;
                        self.drag_current = None;
                        self.move_offset = Vec3::ZERO;
                        self.stretch = None;
                        self.stretch_delta = Vec3::ZERO;
                        self.rotate = None;
                        self.rotate_angle = 0.0;
                    }
                }

                if ui.is_window_hovered() {
                    if ui.is_key_pressed(dear_imgui_rs::Key::Escape) {
                        if self.drag_start.is_some() {
                            self.drag_start = None;
                            self.drag_current = None;
                            self.move_offset = Vec3::ZERO;
                            self.stretch = None;
                            self.stretch_delta = Vec3::ZERO;
                            self.rotate = None;
                            self.rotate_angle = 0.0;
                        }
                        if edit_faces {
                            selected_faces.clear();
                            view2d::sync_selected_brushes_from_faces(
                                selected_faces,
                                selected_brushes,
                            );
                        } else if edit_edges {
                            selected_edges.clear();
                            selected_brushes.clear();
                        } else {
                            selected_brushes.clear();
                        }
                        if edit_vertices {
                            selected_patch_vertices.clear();
                        }
                        selected_entities.clear();
                    }
                }

                draw.with_clip_rect(p, [p[0] + w, p[1] + h], || {
                    draw.add_rect(
                        p,
                        [p[0] + w, p[1] + h],
                        imgui_color_to_u32(palette.view2d_bg),
                    )
                    .filled(true)
                    .build();

                    if let Some(tid) = self.tex_id {
                        draw.add_image(
                            tid,
                            p,
                            [p[0] + w, p[1] + h],
                            [0.0, 1.0],
                            [1.0, 0.0],
                            0xFFFFFFFFu32,
                        );
                    }
                });

                let hud_pos = [p[0] + 8.0, p[1] + 8.0];
                let hud_col = imgui_color_to_u32(palette.hud_text_dim);
                draw.add_text(
                    hud_pos,
                    hud_col,
                    &format!("Move Speed ({:.0})", self.cam.zoom),
                );

                // Snapped drag marker
                if self.drag_start.is_some() && self.move_offset != Vec3::ZERO {
                    let snapped_world = self.drag_world_anchor + self.move_offset;
                    if let Some(screen_pos) = world_to_screen_3d(common_vp, self.rect, snapped_world) {
                        let col = imgui_color_to_u32(palette.theme_primary);
                        draw.add_circle(screen_pos, 4.0, col).filled(true).build();
                    }
                }

                // Drag info
                if let (Some(start), Some(end)) = (self.drag_start, self.drag_current) {
                    let col = ui.style_color(StyleColor::TabSelectedOverline);
                    let delta_info_col = util::imgui_color_to_u32(col);

                    match self.drag_mode {
                        DragMode::MoveEdges => {
                            if self.move_offset != Vec3::ZERO {
                                let a =
                                    world_to_screen_3d(common_vp, self.rect, self.drag_world_anchor);
                                let b = world_to_screen_3d(
                                    common_vp,
                                    self.rect,
                                    self.drag_world_anchor + self.move_offset,
                                );
                                if let (Some(a), Some(b)) = (a, b) {
                                    let delta_info = format_vec3(self.move_offset, 2);
                                    let tw = text_width(ui, &delta_info);
                                    let text_h = text_height(ui, "1") + 2.0;
                                    let text_pos = [b[0] - tw / 2.0, b[1] - text_h];

                                    draw.add_line(
                                        a,
                                        b,
                                        util::adjust_color_brightness(delta_info_col, 1.5),
                                    )
                                    .thickness(2.0)
                                    .build();
                                    draw.add_text(
                                        text_pos,
                                        util::adjust_color_brightness(delta_info_col, 2.0),
                                        delta_info,
                                    );
                                }
                            }
                        }
                        DragMode::MoveSelection => {
                            let draw_coords = if let Some(sel) = selection_aabb_active(map, selected_brushes, selected_faces, selected_edges, selected_entities, edit_faces, edit_edges, ent_draw_config) {
                                let center = Vec3::new(
                                    (sel.min.x + sel.max.x) * 0.5,
                                    (sel.min.y + sel.max.y) * 0.5,
                                    (sel.min.z + sel.max.z) * 0.5,
                                );
                                let a = world_to_screen_3d(common_vp, self.rect, center);
                                let b = world_to_screen_3d(common_vp, self.rect, center + self.move_offset);
                                (a, b)
                            }
                            else {
                                (Some(start.into()), Some(end.into()))
                            };
                            if let (Some(a), Some(b)) = draw_coords {
                                let delta_info = format_vec3(self.move_offset, 2);
                                let tw = text_width(ui, &delta_info);
                                let text_h = text_height(ui, "1") + 2.0;
                                let text_pos = [b[0] - tw / 2.0, b[1] - text_h];

                                draw.add_line(a, b, util::adjust_color_brightness(delta_info_col, 1.5))
                                    .thickness(2.0)
                                    .build();
                                draw.add_text(text_pos, util::adjust_color_brightness(delta_info_col, 2.0), delta_info);
                            }
                        }
                        DragMode::StretchSelection => {
                            if self.stretch_delta != Vec3::ZERO {
                                // Use cached face_center from SideStretchDrag
                                let face_center = if let Some(stretch) = self.stretch.as_ref() {
                                    Some(stretch.face_center)
                                } else {
                                    None
                                };

                                // Line from original face center to stretched face center
                                let draw_coords = face_center.and_then(|center| {
                                    let a = world_to_screen_3d(common_vp, self.rect, center);
                                    let b = world_to_screen_3d(common_vp, self.rect, center + self.stretch_delta);
                                    Some((a?, b?))
                                });

                                if let Some((a, b)) = draw_coords {
                                    let delta_info = format_vec3(self.stretch_delta, 2);
                                    let text_h = text_height(ui, "1") + 2.0;
                                    let text_pos = [b[0] + 8.0, b[1] - text_h];

                                    draw.add_line(a, b, util::adjust_color_brightness(delta_info_col, 1.5))
                                        .thickness(2.0)
                                        .build();
                                    draw.add_text(text_pos, util::adjust_color_brightness(delta_info_col, 2.0), delta_info);
                                }
                            }
                        }
                        DragMode::RotateSelection => {
                            if self.rotate_angle.abs() > 1.0e-6 {
                                let angle_deg = self.rotate_angle.to_degrees();
                                let delta_info = format!("{:.1}°", angle_deg);
                                let tw = text_width(ui, &delta_info);
                                let text_h = text_height(ui, "1") + 2.0;
                                let text_pos = [end.x - tw / 2.0, end.y - text_h];

                                // Draw a line from start to current mouse position
                                let a = [start.x, start.y];
                                let b = [end.x, end.y];
                                draw.add_line(a, b, util::adjust_color_brightness(delta_info_col, 1.5))
                                    .thickness(2.0)
                                    .build();
                                draw.add_text(text_pos, util::adjust_color_brightness(delta_info_col, 2.0), delta_info);
                            }
                        }
                        _ => {}
                    }
                }

                // selected entity labels
                for ent_id in selected_entities {
                    if let Some(map) = map && let Some(ent) = map.entities.get(*ent_id) {
                        // skip entities with brushes, e.g. triggers, worldspawn
                        if !ent.brushes.is_empty() {
                            continue;
                        }
                        let has_model = ent.model.is_some();
                        let origin = if let Some(o) = ent.properties.get("origin") {
                            core_util::origin_to_vec3(o)
                        }
                        else { Vec3::ZERO };
                        let style = ent_draw_config.resolve(&ent.classname, has_model);
                        let world_pos = match style.anchor {
                            EntityDrawAnchor::Center => {
                                if has_model {
                                    let model_height = if let Some(ref m) = ent.model {
                                        (m.maxs.z - m.mins.z).abs()
                                    } else { 0.0 };
                                    Vec3::new(origin.x, origin.y, origin.z + model_height + 4.0)
                                }
                                else {
                                    Vec3::new(origin.x, origin.y, origin.z + (style.size[2] / 2.0) + 4.0)
                                }
                            },
                            EntityDrawAnchor::Base => {
                                if has_model {
                                    let model_height = if let Some(ref m) = ent.model {
                                        (m.maxs.z - m.mins.z).abs()
                                    } else { 0.0 };
                                    Vec3::new(origin.x, origin.y, origin.z + model_height + 4.0)
                                }
                                else {
                                    Vec3::new(origin.x, origin.y, origin.z + style.size[2] + 4.0)
                                }
                            }
                        };
                        if let Some(screen_pos) = world_to_screen_3d(common_vp, self.rect, world_pos) {
                            let label = format!("{ent_id}: {} ({}, {}, {})", &ent.classname, origin.x, origin.y, origin.z);
                            let tw = crate::util::text_width(ui, &label);
                            draw.add_text(
                                [screen_pos[0] - tw * 0.5, screen_pos[1]],
                                adjust_color_brightness(imgui_color_to_u32(selection_rgba), 1.5),
                                label,
                            );
                        }
                    }
                }
            });
    }

    fn screen_to_ray(&self, screen_pos: Vec2, config: &EditorConfig) -> Vec3 {
        let [rx, ry, rw, rh] = self.rect;
        let nx = (screen_pos.x - rx) / rw * 2.0 - 1.0;
        let ny = 1.0 - (screen_pos.y - ry) / rh * 2.0;

        let forward = Self::forward_from_angles(self.cam.angles);
        // perspective_rh (not _gl) + near=1.0, matches the selection pick code
        let proj = Mat4::perspective_rh(config.view.fov.to_radians(), rw / rh, 1.0, 1.0e6);
        let view = Mat4::look_at_rh(self.cam.pos, self.cam.pos + forward, Vec3::Z);
        let inv_vp = (proj * view).inverse();

        let target = inv_vp.project_point3(Vec3::new(nx, ny, 1.0));
        (target - self.cam.pos).normalize()
    }

    pub fn ray_triangle_t(origin: Vec3, dir: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
        let eps = 1e-7;
        let e1 = v1 - v0;
        let e2 = v2 - v0;
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < eps {
            return None;
        }
        let inv = 1.0 / det;
        let t_vec = origin - v0;
        let u = t_vec.dot(p) * inv;
        if u < 0.0 || u > 1.0 {
            return None;
        }
        let q = t_vec.cross(e1);
        let v = dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = e2.dot(q) * inv;
        (t > eps).then_some(t)
    }

    fn ray_aabb_intersection(min: Vec3, max: Vec3, origin: Vec3, dir: Vec3) -> Option<(f32, f32)> {
        let inv_dir = Vec3::new(
            if dir.x.abs() > 1e-8 {
                1.0 / dir.x
            } else {
                f32::INFINITY
            },
            if dir.y.abs() > 1e-8 {
                1.0 / dir.y
            } else {
                f32::INFINITY
            },
            if dir.z.abs() > 1e-8 {
                1.0 / dir.z
            } else {
                f32::INFINITY
            },
        );

        let t1 = (min - origin) * inv_dir;
        let t2 = (max - origin) * inv_dir;

        let t_min = t1.min(t2);
        let t_max = t1.max(t2);

        let t_enter = t_min.x.max(t_min.y).max(t_min.z);
        let t_exit = t_max.x.min(t_max.y).min(t_max.z);

        (t_enter <= t_exit).then_some((t_enter, t_exit))
    }
}

fn pick_patch_control_vertex_by_screen_3d(
    map: &kradiant::map::Map,
    prefer: Option<&[(usize, usize)]>,
    vp: glam::Mat4,
    mouse_screen: [f32; 2],
    rect: [f32; 4], // [rx, ry, rw, rh]
    radius_px: f32,
) -> Option<PatchVertexSelection> {
    let r2 = radius_px.max(1.0) * radius_px.max(1.0);
    let mut best: Option<(PatchVertexSelection, f32, f32)> = None; // sel, dist2, depth

    let mut visit_patch = |entity_idx: usize, brush_idx: usize, patch: &kradiant::map::Patch| {
        for (row_idx, row) in patch.vertices.iter().enumerate() {
            for (col_idx, v) in row.iter().enumerate() {
                let clip = vp * glam::Vec4::new(v.position.x, v.position.y, v.position.z, 1.0);
                if clip.w < 0.1 {
                    continue; // Behind or too close to camera
                }

                let ndc = glam::Vec3::new(clip.x, clip.y, clip.z) / clip.w;
                let [rx, ry, rw, rh] = rect;
                let sx = rx + (ndc.x * 0.5 + 0.5) * rw;
                let sy = ry + (1.0 - (ndc.y * 0.5 + 0.5)) * rh;

                let dx = sx - mouse_screen[0];
                let dy = sy - mouse_screen[1];
                let d2 = dx * dx + dy * dy;

                if d2 > r2 {
                    continue;
                }

                let sel = PatchVertexSelection {
                    entity_idx,
                    brush_idx,
                    row: row_idx,
                    col: col_idx,
                };

                let depth = ndc.z; // -1 to 1 depth

                match best {
                    None => best = Some((sel, d2, depth)),
                    Some((_, _, best_depth)) => {
                        // Prefer closer depth.
                        if depth < best_depth {
                            best = Some((sel, d2, depth));
                        }
                    }
                }
            }
        }
    };

    if let Some(prefer) = prefer {
        for &(entity_idx, brush_idx) in prefer {
            let Some(entity) = map.entities.get(entity_idx) else {
                continue;
            };
            let Some(brush) = entity.brushes.get(brush_idx) else {
                continue;
            };
            let kradiant::map::BrushContent::Patch(patch) = &brush.content else {
                continue;
            };
            visit_patch(entity_idx, brush_idx, patch);
        }
    } else {
        for (entity_idx, entity) in map.entities.iter().enumerate() {
            for (brush_idx, brush) in entity.brushes.iter().enumerate() {
                let kradiant::map::BrushContent::Patch(patch) = &brush.content else {
                    continue;
                };
                visit_patch(entity_idx, brush_idx, patch);
            }
        }
    }

    best.map(|(sel, _, _)| sel)
}

/// Project an edge segment for screen-space picking, clipping it to the
/// camera's near plane so long edges with off-screen endpoints (or endpoints
/// behind the camera) stay pickable along their visible part. Screen
/// coordinates may lie outside the viewport (distance math still works).
fn project_edge_for_pick(
    vp: Mat4,
    rect: [f32; 4],
    a: Vec3,
    b: Vec3,
) -> Option<([f32; 2], [f32; 2])> {
    const W_MIN: f32 = 0.1;
    let mut pa = vp * Vec4::new(a.x, a.y, a.z, 1.0);
    let mut pb = vp * Vec4::new(b.x, b.y, b.z, 1.0);
    if pa.w < W_MIN && pb.w < W_MIN {
        return None;
    }
    if pa.w < W_MIN {
        let t = (W_MIN - pa.w) / (pb.w - pa.w);
        pa = pa.lerp(pb, t);
    } else if pb.w < W_MIN {
        let t = (W_MIN - pb.w) / (pa.w - pb.w);
        pb = pb.lerp(pa, t);
    }
    let to_screen = |c: Vec4| -> [f32; 2] {
        let ndc = Vec3::new(c.x, c.y, c.z) / c.w;
        let [rx, ry, rw, rh] = rect;
        [
            rx + (ndc.x * 0.5 + 0.5) * rw,
            ry + (1.0 - (ndc.y * 0.5 + 0.5)) * rh,
        ]
    };
    Some((to_screen(pa), to_screen(pb)))
}

/// Screen-space edge picking for the 3D view. Occlusion rules:
/// - When the mouse ray hits a brush surface, only edges ON or IN FRONT of
///   that surface (its own edges, flush seams of neighbours, geometry
///   nearer than it) are pickable — never an edge strictly behind it, so
///   no edges get grabbed through other brushes. Edges are compared by
///   camera depth against the front hit, so this does not depend on which
///   brush happens to be hit first.
/// - With no brush under the ray, all edges within `radius_px` compete.
/// Among candidates the CLOSEST TO THE CAMERA wins (depth-buffer semantics);
/// ties within a small depth band prefer brushes that already have selected
/// edges, then the smaller pixel distance. Visibility mask applies to both
/// the blocking test and the candidate edges.
fn pick_convex_edge_by_screen_3d(
    map: &mut kradiant::map::Map,
    prefer: Option<&[(usize, usize)]>,
    mask: PickMask,
    vp: Mat4,
    ray_origin: Vec3,
    ray_dir: Vec3,
    mouse_screen: [f32; 2],
    rect: [f32; 4],
    radius_px: f32,
) -> Option<EdgeSelection> {
    let r2 = radius_px.max(1.0) * radius_px.max(1.0);
    // (selection, pixel distance², camera-space depth of the closest approach
    // of the mouse ray to the edge)
    let mut candidates: Vec<(EdgeSelection, f32, f32)> = Vec::new();

    let visit_brush = |entity_idx: usize,
                           brush_idx: usize,
                           brush: &mut kradiant::map::Brush,
                           out: &mut Vec<(EdgeSelection, f32, f32)>| {
        if !matches!(brush.content, kradiant::map::BrushContent::Convex(_)) {
            return;
        }
        // Same visibility rules as brush picking.
        if !mask.contains(PickMask::CONVEX) {
            return;
        }
        if brush.is_clip() && !mask.contains(PickMask::CLIP) {
            return;
        }
        if brush.is_portal() && !mask.contains(PickMask::PORTAL) {
            return;
        }
        if brush.is_hint() && !mask.contains(PickMask::HINT) {
            return;
        }
        let Some((_aabb, polys)) = brush.get_polygons_and_aabb() else {
            return;
        };
        for (sel, a, b) in view2d::convex_edges_for_brush(entity_idx, brush_idx, polys) {
            // Near-plane-clipped projection: an edge stays pickable along
            // its visible part even when an endpoint is off-screen or behind
            // the camera.
            let Some((sa, sb)) = project_edge_for_pick(vp, rect, a, b) else {
                continue;
            };
            let ab = [sb[0] - sa[0], sb[1] - sa[1]];
            let ap = [mouse_screen[0] - sa[0], mouse_screen[1] - sa[1]];
            let len2 = ab[0] * ab[0] + ab[1] * ab[1];
            let d2 = if len2 <= 1.0e-6 {
                ap[0] * ap[0] + ap[1] * ap[1]
            } else {
                let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / len2).clamp(0.0, 1.0);
                let q = [sa[0] + ab[0] * t, sa[1] + ab[1] * t];
                let dx = mouse_screen[0] - q[0];
                let dy = mouse_screen[1] - q[1];
                dx * dx + dy * dy
            };
            if d2 > r2 {
                continue;
            }

            // Camera-space depth: ray parameter at the closest approach of
            // the mouse ray to this edge.
            let depth = ray_closest_point_on_segment(ray_origin, ray_dir, a, b)
                .map(|p| ray_dir.dot(p - ray_origin).max(0.0))
                .unwrap_or_else(|| (a - ray_origin).dot(ray_dir).max(0.0));

            out.push((sel, d2, depth));
        }
    };

    for (entity_idx, entity) in map.entities.iter_mut().enumerate() {
        for (brush_idx, brush) in entity.brushes.iter_mut().enumerate() {
            visit_brush(entity_idx, brush_idx, brush, &mut candidates);
        }
    }

    // Occlusion: nothing strictly BEHIND the frontmost brush surface under
    // the cursor may be picked. Comparing depths (not brush identity) keeps
    // edges lying ON that surface pickable — the front brush's own edges and
    // flush seams of neighbouring brushes — while everything behind it is
    // blocked. When the ray misses every brush, all candidates compete.
    let front_t = ray_front_brush_hit_point(map, ray_origin, ray_dir, mask)
        .map(|p| ray_dir.dot(p - ray_origin));

    let mut best: Option<(EdgeSelection, f32, f32)> = None;
    for (sel, d2, depth) in candidates {
        if let Some(front_t) = front_t {
            let band = (front_t * 1.0e-3).max(0.5);
            if depth > front_t + band {
                continue;
            }
        }
        match best {
            None => best = Some((sel, d2, depth)),
            Some((best_sel, best_d2, best_depth)) => {
                // Depth band: flush/coincident edges count as tied so
                // pixel distance can still decide between them.
                let band = (best_depth * 1.0e-3).max(0.5);
                let take = if depth < best_depth - band {
                    // Strictly in front of the current best: always wins.
                    true
                } else if depth > best_depth + band {
                    false
                } else {
                    let best_preferred = prefer
                        .map(|p| p.contains(&(best_sel.entity_idx, best_sel.brush_idx)))
                        .unwrap_or(false);
                    let this_preferred = prefer
                        .map(|p| p.contains(&(sel.entity_idx, sel.brush_idx)))
                        .unwrap_or(false);
                    if this_preferred != best_preferred {
                        this_preferred
                    } else {
                        d2 < best_d2
                    }
                };
                if take {
                    best = Some((sel, d2, depth));
                }
            }
        }
    }

    best.map(|(sel, _, _)| sel)
}

/// World-space hit point on the frontmost brush surface along the ray.
fn ray_front_brush_hit_point(
    map: &mut kradiant::map::Map,
    origin: Vec3,
    dir: Vec3,
    mask: PickMask,
) -> Option<Vec3> {
    let (entity_idx, brush_idx) = editing::pick_brush_by_ray(map, origin, dir, mask, None)?;
    let entity = map.entities.get_mut(entity_idx)?;
    let brush = entity.brushes.get_mut(brush_idx)?;
    let polys = brush.get_polygons()?;
    let mut best: Option<f32> = None;
    for (positions, indices) in polys {
        for tri in indices.chunks_exact(3) {
            let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
                continue;
            }
            if let Some(t) = View3D::ray_triangle_t(origin, dir, positions[i0], positions[i1], positions[i2]) {
                best = Some(best.map_or(t, |b: f32| b.min(t)));
            }
        }
    }
    best.map(|t| origin + dir * t)
}

/// Closest point on segment `a..b` to the ray `origin + t * dir` (t >= 0).
fn ray_closest_point_on_segment(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3) -> Option<Vec3> {
    let edge = b - a;
    let len = edge.length();
    if len < 1.0e-6 {
        return None;
    }
    let edge_dir = edge / len;

    let w0 = a - origin;
    let bb = dir.dot(edge_dir);
    let dd = dir.dot(w0);
    let ee = edge_dir.dot(w0);
    let denom = 1.0 - bb * bb;

    let mut s = if denom > 1.0e-12 {
        (bb * dd - ee) / denom
    } else {
        -ee
    };
    s = s.clamp(0.0, len);

    let t = (dd + bb * s).max(0.0);
    if t == 0.0 {
        s = (-ee).clamp(0.0, len);
    }
    Some(a + edge_dir * s)
}
