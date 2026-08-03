//! 3D View

use dear_imgui_rs::{Condition, Key, TextureId, Ui, WindowFlags};
use glam::{Mat4, Vec2, Vec3};
use kradiant::editor::viewport::DragMode;
use kradiant::editor::viewport::state::View3DState;
use kradiant::editor::{EditorConfig, EditorPalette, FaceSelection, PatchVertexSelection};
use std::ops::{Deref, DerefMut};

use crate::ui::editing::PickMask;
use crate::ui::view2d;
use crate::util::imgui_color_to_u32;
use kradiant::editing;
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
    /// Current drag mode for transformations
    pub drag_mode: DragMode,
    /// Drag start position in screen space
    pub drag_start: Option<Vec2>,
    /// Drag current position in screen space
    pub drag_current: Option<Vec2>,
    pub drag_plane_normal: Vec3, // camera forward at drag start
    pub drag_plane_origin: Vec3, // selection center
    pub drag_world_anchor: Vec3, // world point clicked at drag start
    /// Accumulated drag offset during drag (applied to geometry only on release)
    pub move_offset: Vec3,
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
            drag_mode: DragMode::NewBrush,
            drag_start: None,
            drag_current: None,
            drag_plane_normal: Vec3::ZERO,
            drag_plane_origin: Vec3::ZERO,
            drag_world_anchor: Vec3::ZERO,
            move_offset: Vec3::ZERO,
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
                                }
                            }
                        } else if edit_edges {
                            let selected_edge = map.as_mut().and_then(|m| {
                                let (entity_idx, brush_idx, face_a_idx, face_b_idx) =
                                    editing::pick_edge_by_ray(m, ray_origin, ray_dir, mask, None)?;
                                Some(EdgeSelection {
                                    entity_idx,
                                    brush_idx,
                                    face_a_idx,
                                    face_b_idx,
                                })
                            });

                            if let Some(sel) = selected_edge {
                                let touched = self
                                    .selection_drag_touched_edges
                                    .get_or_insert_with(HashSet::new);
                                if touched.insert(sel) {
                                    if !selected_edges.contains(&sel) {
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
                                    }
                                }
                            }
                        } // end outer else (not edit_faces/edges/vertices)
                    } // end if shift_selecting
                    if ui.is_mouse_clicked(MouseButton::Left)
                        && !ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                    {
                        if ui.is_key_down(dear_imgui_rs::Key::LeftAlt) {
                            self.drag_mode = DragMode::RectangularSelection;
                        }
                    }

                    // Handle left-click drag for transformations
                    if ui.is_mouse_clicked(MouseButton::Left)
                        && !ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                        && !ui.is_key_down(dear_imgui_rs::Key::LeftAlt)
                    {
                        if !selected_brushes.is_empty() || !selected_entities.is_empty() {
                            self.drag_start = Some(Vec2::new(mouse[0], mouse[1]));
                            self.drag_current = Some(Vec2::new(mouse[0], mouse[1]));
                            self.move_offset = Vec3::ZERO;
                            self.drag_mode = DragMode::MoveSelection;

                            // Calculate drag sensitivity vectors based on camera and selection depth
                            let forward = Self::forward_from_angles(self.cam.angles);

                            let selection_center = if let Some(map) = map {
                                view2d::selection_aabb(
                                    map,
                                    selected_brushes,
                                    selected_entities,
                                    ent_draw_config,
                                )
                                .map(|aabb| (aabb.min + aabb.max) * 0.5)
                                .unwrap_or(Vec3::ZERO)
                            }
                            else {
                                Vec3::ZERO
                            };

                            //let forward = Self::forward_from_angles(self.cam.angles);
                            // let up_vec = Vec3::Z;
                            // let view = Mat4::look_at_rh(self.cam.pos, self.cam.pos + forward, up_vec);
                            // Use perspective_rh_gl to match the renderer
                            // let proj = Mat4::perspective_rh_gl(config.view.fov.to_radians(), w / h, 4.0, 100_000.0);
                            // let inv_vp = (proj * view).inverse();

                            let ray = self.screen_to_ray(Vec2::new(mouse[0], mouse[1]), config);
                            // let denom = forward.dot(ray);
                            // if denom.abs() > 1e-6 {
                            //     let t = forward.dot(selection_center - self.cam.pos) / denom;
                            //     self.drag_world_anchor = self.cam.pos + ray * t;
                            //     self.drag_plane_normal = forward;
                            //     self.drag_plane_origin = selection_center;
                            // }
                            //
                            // crate::log_info!(console, "anchor={:.1?} sel_center={:.1?}", self.drag_world_anchor, selection_center);
                            let brush_hit_t =
                                selected_brushes
                                    .iter()
                                    .find_map(|&(entity_idx, brush_idx)| {
                                        let entity = map.as_mut()?.entities.get_mut(entity_idx)?;
                                        let brush = entity.brushes.get_mut(brush_idx)?;
                                        let (_aabb, polys) = brush.get_polygons_and_aabb()?;
                                        // ray_polys_first_hit is private to editing.rs, replicate inline:
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
                                                        best =
                                                            Some(best.map_or(t, |b: f32| b.min(t)));
                                                    }
                                                }
                                            }
                                        }
                                        best
                                    });
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
                            let hit_t = match (brush_hit_t, entity_hit_t) {
                                (Some(a), Some(b)) => a.min(b),
                                (Some(t), None) | (None, Some(t)) => t,
                                (None, None) => {
                                    // Fallback: distance to selection center
                                    let denom = forward.dot(ray);
                                    if denom.abs() > 1e-6 {
                                        forward.dot(selection_center - self.cam.pos) / denom
                                    } else {
                                        1.0
                                    }
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
                        if !selected_brushes.is_empty() || !selected_entities.is_empty() {
                            //                             let current = Vec2::new(mouse[0], mouse[1]);
                            //                             let start = self.drag_start.unwrap();
                            //                             self.drag_current = Some(current);
                            //
                            //                             // Calculate total movement from start position
                            //                             let dx = current.x - start.x;
                            //                             let dy = current.y - start.y;
                            //                             let mut total_move = self.drag_x_dir * dx + self.drag_y_dir * dy;
                            //
                            //                             // Snap to grid if enabled (snap the total movement, not frame delta)
                            //                             if config.view.grid_snap {
                            //                                 let grid = config.view.grid_minor_step.max(1) as f32;
                            //                                 total_move.x = (total_move.x / grid + 0.5).floor() * grid;
                            //                                 total_move.y = (total_move.y / grid + 0.5).floor() * grid;
                            //                                 total_move.z = (total_move.z / grid + 0.5).floor() * grid;
                            //                             }
                            //
                            //                             // Update the move offset (this will be applied on release)
                            //                             self.move_offset = total_move;
                            //
                            //                             crate::log_info!(console, "drag: current={:?} start={:?} dx={} dy={} total_move={:?}", current, start, dx, dy, total_move);

                            let current = Vec2::new(mouse[0], mouse[1]);

                            // let forward = Self::forward_from_angles(self.cam.angles);
                            // let up_vec = Vec3::Z;
                            // let view = Mat4::look_at_rh(self.cam.pos, self.cam.pos + forward, up_vec);
                            // let proj = Mat4::perspective_rh_gl(config.view.fov.to_radians(), w / h, 4.0, 100_000.0);
                            // let inv_vp = (proj * view).inverse();

                            let ray = self.screen_to_ray(current, config);
                            let denom = self.drag_plane_normal.dot(ray);
                            if denom.abs() > 1e-6 {
                                let t = self
                                    .drag_plane_normal
                                    .dot(self.drag_plane_origin - self.cam.pos)
                                    / denom;
                                let world_current = self.cam.pos + ray * t;
                                let mut offset = world_current - self.drag_world_anchor;

                                if config.view.grid_snap {
                                    let grid = config.view.grid_minor_step.max(1) as f32;
                                    offset.x = (offset.x / grid).round() * grid;
                                    offset.y = (offset.y / grid).round() * grid;
                                    offset.z = (offset.z / grid).round() * grid;
                                }
                                self.move_offset = offset;
                            }
                        }
                    } else {
                        // Mouse released or not dragging - apply accumulated movement to geometry and push to undo
                        if self.drag_start.is_some() && self.move_offset != Vec3::ZERO {
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

                                // Apply the accumulated movement to the selected brushes and entities.
                                if let Some(map) = map.as_mut() {
                                    let delta = self.move_offset;
                                    let generation = &mut map.generation;
                                    for (entity_idx, brush_idx) in selected_brushes.clone() {
                                        if let Some(entity) = map.entities.get_mut(entity_idx) {
                                            if let Some(brush) = entity.brushes.get_mut(brush_idx) {
                                                brush.translate(generation, delta);
                                            }
                                        }
                                    }
                                    for ent_idx in selected_entities.iter().copied() {
                                        if ent_idx == 0 {
                                            continue; // Never drag worldspawn
                                        }
                                        if selected_brushes
                                            .iter()
                                            .any(|(entity_idx, _)| *entity_idx == ent_idx)
                                        {
                                            continue;
                                        }
                                        if let Some(entity) = map.entities.get_mut(ent_idx) {
                                            entity.translate(generation, delta);
                                        }
                                    }
                                }

                                crate::log_info!(console, "Dragged selection in 3D View");
                            }
                        }
                        self.drag_start = None;
                        self.drag_current = None;
                        self.move_offset = Vec3::ZERO;
                    }
                }

                if ui.is_window_hovered() {
                    if ui.is_key_pressed(dear_imgui_rs::Key::Escape) {
                        if edit_faces {
                            selected_faces.clear();
                            view2d::sync_selected_brushes_from_faces(
                                selected_faces,
                                selected_brushes,
                            );
                        } else {
                            selected_brushes.clear();
                        }
                        if edit_vertices {
                            selected_patch_vertices.clear();
                        }
                        selected_entities.clear();
                        // self.stretch = None;
                        // self.stretch_delta = Vec3::ZERO;
                        // self.rotate = None;
                        // self.rotate_angle = 0.0;
                        // self.move_offset = Vec3::ZERO;
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

    fn ray_triangle_t(origin: Vec3, dir: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
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
