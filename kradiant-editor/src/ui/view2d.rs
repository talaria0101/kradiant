//! 2D View

use crate::config::EditorConfig;
use crate::ui::FaceSelection;
use crate::ui::console::ConsoleLogger;
use crate::util::{project_to_2d, text_height, text_width};
use crate::{log_error, log_info, log_warn, util};
use dear_imgui_rs::{Condition, StyleColor, TextureId, Ui, WindowFlags};
use glam::{Quat, Vec2, Vec3};
use kradiant::editing::{self, Aabb};
use kradiant::map::{BrushContent, BrushId, Face};
use kradiant::map_utils::format_float;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ortho {
    #[default]
    XY,
    XZ,
    YZ,
}

impl Ortho {
    pub fn label(self) -> &'static str {
        match self {
            Ortho::XY => "XY (top)",
            Ortho::XZ => "XZ (front)",
            Ortho::YZ => "YZ (side)",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::XY => Self::XZ,
            Self::XZ => Self::YZ,
            Self::YZ => Self::XY,
        }
    }
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum DragMode {
    #[default]
    NewBrush,
    MoveSelection,
    MoveVertices,
    StretchSelection,
    RotateSelection,
    RectangularSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchMode {
    Scale,
    Resize,
}

impl Default for StretchMode {
    fn default() -> Self {
        Self::Scale
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AxisLock {
    pub x: bool,
    pub y: bool,
    pub z: bool,
}

#[derive(Clone)]
pub struct StretchDrag {
    pub selection_aabb: Aabb,
    pub faces: [Option<editing::StretchFace>; 2],
}

#[derive(Clone)]
pub struct RotateDrag {
    pub selection_aabb: Aabb,
    pub pivot_uv: [f32; 2],
    pub start_uv: [f32; 2],
    pub axis: Vec3,
}

pub struct View2D {
    pub ortho_axis: Ortho,
    pub rect: [f32; 4],
    pub zoom: f32,
    pub pan: [f32; 2],
    pub drag_start: Option<Vec2>,
    pub drag_current: Option<Vec2>,
    pub drag_mode: DragMode,
    pub tex_id: Option<TextureId>,
    /// Offset for rendering when we are moving something
    pub move_offset: Vec3,
    pub work_pos: Vec3,
    pub work_depth: Vec3,
    pub stretch: Option<StretchDrag>,
    pub stretch_delta: Vec3,
    pub rotate: Option<RotateDrag>,
    pub rotate_angle: f32,
    pub last_aabb: Option<Aabb>,
    // Selection rectangle for rectangular selection mode
    //pub selection_rect: Option<[Vec2; 2]>,
    /// Brushes touched during current selection drag (to avoid toggling multiple times)
    pub selection_drag_touched: Option<HashSet<(usize, usize)>>,
    /// Faces touched during current selection drag
    pub selection_drag_touched_faces: Option<HashSet<FaceSelection>>,
    /// Patch vertices touched during current selection drag
    pub selection_drag_touched_verts: Option<HashSet<usize>>, // stores hashed selection
}

impl Default for View2D {
    fn default() -> Self {
        Self {
            ortho_axis: Ortho::default(),
            rect: [0.0; 4],
            zoom: 1.0,
            pan: [0.0, 0.0],
            drag_start: None,
            drag_current: None,
            drag_mode: DragMode::NewBrush,
            tex_id: None,
            move_offset: Vec3::ZERO,
            stretch: None,
            stretch_delta: Vec3::ZERO,
            rotate: None,
            rotate_angle: 0.0,
            work_pos: Vec3::ZERO,
            work_depth: Vec3::ZERO,
            last_aabb: None,
            selection_drag_touched: None,
            selection_drag_touched_faces: None,
            selection_drag_touched_verts: None,
        }
    }
}

impl View2D {
    /// Check if a face is facing the current orthographic view direction.
    /// Returns true if the face normal points toward the camera.
    fn is_face_facing_view(&self, face: &Face) -> bool {
        // Compute face normal from plane points
        let v0 = face.plane_points[1] - face.plane_points[0];
        let v1 = face.plane_points[2] - face.plane_points[0];
        let normal = v0.cross(v1).normalize();

        // View direction for each ortho axis (camera looks along negative axis)
        let view_dir = match self.ortho_axis {
            Ortho::XY => Vec3::NEG_Z, // Top-down view, looking down -Z
            Ortho::XZ => Vec3::NEG_Y, // Front view, looking down -Y
            Ortho::YZ => Vec3::NEG_X, // Side view, looking down -X
        };

        // Face is visible if normal points toward camera (dot product > 0)
        normal.dot(view_dir) > 0.0
    }

    pub fn stretch_preview_xform(&self) -> Option<editing::AffineScale> {
        let stretch = self.stretch.as_ref()?;
        editing::stretch_selection_transform(
            &stretch.selection_aabb,
            stretch.faces,
            self.stretch_delta,
        )
        .map(|(xform, _)| xform)
    }

    pub fn face_stretch_preview(&self) -> Option<([Option<editing::StretchFace>; 2], Vec3)> {
        let stretch = self.stretch.as_ref()?;
        Some((stretch.faces, self.stretch_delta))
    }

    pub fn rotate_preview_xform(&self) -> Option<editing::AffineRotate> {
        let rotate = self.rotate.as_ref()?;
        editing::rotate_selection_transform(&rotate.selection_aabb, rotate.axis, self.rotate_angle)
            .map(|(xform, _)| xform)
    }

    pub fn get_center(&self) -> Vec3 {
        let zoom = self.zoom.max(0.001);
        let world_x = -self.pan[0] / zoom;
        let world_y = -self.pan[1] / zoom;

        match self.ortho_axis {
            Ortho::XY => Vec3::new(world_x, world_y, self.work_pos.z),
            Ortho::XZ => Vec3::new(world_x, self.work_pos.y, -world_y),
            Ortho::YZ => Vec3::new(self.work_pos.x, world_x, -world_y),
        }
    }

    pub fn set_center(&mut self, world_center: Vec3) {
        let zoom = self.zoom.max(0.001);

        let (target_x, target_y) = match self.ortho_axis {
            Ortho::XY => (world_center.x, world_center.y),
            Ortho::XZ => (world_center.x, -world_center.z),
            Ortho::YZ => (world_center.y, -world_center.z),
        };

        self.pan[0] = -target_x * zoom;
        self.pan[1] = -target_y * zoom;
    }

    pub fn center_to_work(&mut self) {
        self.set_center(self.work_pos);
    }

    pub fn rotation_locked(&self, axis_lock: &AxisLock) -> bool {
        match self.ortho_axis {
            Ortho::XY => axis_lock.y,
            Ortho::XZ | Ortho::YZ => axis_lock.z,
        }
    }

    pub fn draw_impl(
        &mut self,
        ui: &Ui,
        mut config: &mut crate::config::EditorConfig,
        axis_lock: &AxisLock,
        rotate_mode: bool,
        stretch_mode: StretchMode,
        palette: &crate::theme::EditorPalette,
        console: &mut crate::ui::console::ConsoleLogger,
        undo: &mut crate::ui::undo::UndoRedo,
        selected_brushes: &mut Vec<(usize, usize)>,
        selected_faces: &mut Vec<FaceSelection>,
        selected_patch_vertices: &mut Vec<crate::ui::PatchVertexSelection>,
        selected_entity: &mut Option<usize>,
        edit_faces: bool,
        edit_vertices: bool,
        map: &mut Option<kradiant::map::Map>,
        selection_rgba: [f32; 4],
        selection_rect_rgba: [f32; 4],
        dt: f32,
    ) {
        use dear_imgui_rs::MouseButton;

        ui.window("2D View")
            .size([640.0, 480.0], Condition::FirstUseEver)
            .flags(WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE)
            .build(|| {
                let [w, h] = ui.content_region_avail();
                let (w, h) = (w.max(1.0), h.max(1.0));
                let p = ui.cursor_screen_pos();
                let draw = ui.get_window_draw_list();

                self.rect = [p[0], p[1], w, h];

                ui.set_cursor_screen_pos(p);
                ui.invisible_button("##2d_canvas", [w, h]);
                let canvas_interacting = ui.is_item_hovered() || ui.is_item_active();

                if edit_faces {
                    sync_selected_brushes_from_faces(selected_faces, selected_brushes);
                    if !selected_patch_vertices.is_empty() {
                        selected_patch_vertices.clear();
                    }
                } else {
                    if !selected_faces.is_empty() {
                        selected_faces.clear();
                    }
                    if !edit_vertices && !selected_patch_vertices.is_empty() {
                        selected_patch_vertices.clear();
                    }
                }

                if canvas_interacting {
                    if ui.is_key_pressed(dear_imgui_rs::Key::Tab)
                    {
                        let old_center = self.get_center();
                        self.ortho_axis = self.ortho_axis.next();
                        self.set_center(old_center);

                        if let Some(aabb) = self.last_aabb.clone() {
                            update_last_work_from_aabb(self, &aabb);
                        }
                    }

                    let wheel = ui.io().mouse_wheel();
                    if wheel != 0.0 {
                        if edit_faces
                            && ui.is_key_down(dear_imgui_rs::Key::LeftAlt)
                            && map.is_some()
                            && !selected_faces.is_empty()
                        {
                            let ticks = wheel.round() as i32;
                            if ticks != 0 {
                                let step = (config.grid_minor_step as f32).max(1.0);
                                let mut delta = match self.ortho_axis {
                                    Ortho::XY => Vec3::new(0.0, 0.0, step * ticks as f32),
                                    Ortho::XZ => Vec3::new(0.0, step * ticks as f32, 0.0),
                                    Ortho::YZ => Vec3::new(step * ticks as f32, 0.0, 0.0),
                                };

                                if axis_lock.x {
                                    delta.x = 0.0;
                                }
                                if axis_lock.y {
                                    delta.y = 0.0;
                                }
                                if axis_lock.z {
                                    delta.z = 0.0;
                                }

                                if delta != Vec3::ZERO {
                                    undo.push(
                                        "Move faces",
                                        map,
                                        selected_brushes,
                                        selected_faces,
                                        selected_patch_vertices,
                                        selected_entity,
                                    );
                                    if let Some(map) = map.as_mut() {
                                        if translate_selected_faces(map, selected_faces, delta) {
                                            log_info!(console, "Moved selected faces");
                                            if let Some(aabb) =
                                                selection_aabb_faces_from_map(map, selected_faces)
                                            {
                                                self.last_aabb = Some(aabb.clone());
                                                update_last_work_from_aabb(self, &aabb);
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                        let mouse = ui.io().mouse_pos();
                        let old_zoom = self.zoom;
                        let f = if wheel > 0.0 { 1.15f32 } else { 1.0 / 1.15 };
                        let new_zoom = (old_zoom * f).clamp(0.025, 64.0);
                        if (new_zoom - old_zoom).abs() > f32::EPSILON {
                            let world = crate::util::screen_to_world(mouse, p, [w, h], old_zoom, self.pan);
                            self.zoom = new_zoom;
                            self.pan[0] = (mouse[0] - (p[0] + w * 0.5)) - world[0] * new_zoom;
                            self.pan[1] = (mouse[1] - (p[1] + h * 0.5)) - world[1] * new_zoom;
                        }
                        }
                    }
                    if ui.is_mouse_dragging(MouseButton::Right) {
                        let [dx, dy] = ui.mouse_drag_delta(MouseButton::Right);
                        self.pan[0] += dx;
                        self.pan[1] += dy;
                        ui.reset_mouse_drag_delta(MouseButton::Right);
                    }
                    if ui.is_mouse_clicked(MouseButton::Right) {}
                }

                let mouse = ui.io().mouse_pos();
                let world = crate::util::screen_to_world(mouse, p, [w, h], self.zoom, self.pan);
                let world_axis = match self.ortho_axis {
                    Ortho::XY => world,
                    Ortho::XZ | Ortho::YZ => [world[0], -world[1]],
                };

                let step = config.grid_minor_step as f32;
                let my_snapping = if ui.is_key_down(dear_imgui_rs::Key::LeftCtrl) {
                    !config.grid_snap
                } else {
                    config.grid_snap
                };

                let (snapped, snapped_i) = if my_snapping {
                    let snapped = [crate::util::snap(world[0], step), crate::util::snap(world[1], step)];
                    let snapped_i = Vec2::new(snapped[0].round(), snapped[1].round());
                    (snapped, snapped_i)
                } else {
                    (world, world.into())
                };

                // Shift+LMouse for selection
                let shift_selecting = canvas_interacting
                    && (ui.is_mouse_clicked(MouseButton::Left)
                        || ui.is_mouse_dragging(MouseButton::Left))
                    && ui.is_key_down(dear_imgui_rs::Key::LeftShift);

                if shift_selecting && ui.is_mouse_clicked(MouseButton::Left) {
                    // Start new selection drag - clear touched sets
                    self.selection_drag_touched = Some(HashSet::new());
                    self.selection_drag_touched_faces = Some(HashSet::new());
                    self.selection_drag_touched_verts = Some(HashSet::new());
                }

                if shift_selecting {
                    let ray_far = 1.0e6;
                    let (ray_origin, ray_dir) = match self.ortho_axis {
                        Ortho::XY => (
                            Vec3::new(world[0], world[1], ray_far),
                            Vec3::new(0.0, 0.0, -1.0),
                        ),
                        Ortho::XZ => (
                            Vec3::new(world[0], ray_far, -world[1]),
                            Vec3::new(0.0, -1.0, 0.0),
                        ),
                        Ortho::YZ => (
                            Vec3::new(ray_far, world[0], -world[1]),
                            Vec3::new(-1.0, 0.0, 0.0),
                        ),
                    };

                    if edit_faces {
                        let selected_face = map
                            .as_mut()
                            .and_then(|m| editing::pick_convex_face_by_ray(m, ray_origin, ray_dir));
                        if let Some((entity_idx, brush_idx, face_idx)) = selected_face {
                            let sel = FaceSelection {
                                entity_idx,
                                brush_idx,
                                face_idx,
                            };
                            // Only toggle if not already touched this drag
                            let touched = self.selection_drag_touched_faces.get_or_insert_with(HashSet::new);
                            if touched.insert(sel) {
                                if !selected_faces.contains(&sel) {
                                    selected_faces.push(sel);
                                } else if let Some(i) = selected_faces.iter().position(|f| *f == sel) {
                                    selected_faces.remove(i);
                                }
                            }
                            sync_selected_brushes_from_faces(selected_faces, selected_brushes);
                            *selected_entity = Some(entity_idx);

                            if let Some(aabb) = selection_aabb_active(map, selected_brushes, selected_faces, edit_faces)
                            {
                                self.last_aabb = Some(aabb.clone());
                                update_last_work_from_aabb(self, &aabb);
                            }
                        }
                    } else if edit_vertices {
                        let Some(map) = map.as_mut() else {
                            return;
                        };

                        // Prefer picking within the current brush selection
                        let prefer = (!selected_brushes.is_empty()).then_some(&selected_brushes[..]);
                        if let Some(vsel) = pick_patch_control_vertex_by_screen(
                            map,
                            prefer,
                            self.ortho_axis,
                            mouse,
                            p,
                            [w, h],
                            self.zoom,
                            self.pan,
                            10.0,
                        ) {
                            //log_info!(console, "{}", z_held);
                            let z_held = ui.is_key_down(dear_imgui_rs::Key::Z);
                            if z_held {
                                if let Some(i) = selected_patch_vertices.iter().position(|s| *s == vsel) {
                                    selected_patch_vertices.remove(i);
                                } else {
                                    selected_patch_vertices.push(vsel);
                                }
                            } else if !selected_patch_vertices.contains(&vsel) {
                                selected_patch_vertices.push(vsel);
                            }

                            let brush_sel = (vsel.entity_idx, vsel.brush_idx);
                            if !selected_brushes.contains(&brush_sel) {
                                selected_brushes.push(brush_sel);
                            }
                            *selected_entity = Some(vsel.entity_idx);

                            if let Some(aabb) =
                                selection_aabb_patch_vertices_from_map(map, selected_patch_vertices)
                            {
                                self.last_aabb = Some(aabb.clone());
                                update_last_work_from_aabb(self, &aabb);
                            }
                        }
                    } else {
                        let selected_brush = map.as_mut().and_then(|m| {
                            editing::pick_brush_by_ray(m, ray_origin, ray_dir, editing::PickMask::ALL)
                        });
                        if let Some(sel) = selected_brush {
                            // Only toggle if not already touched this drag
                            let touched = self.selection_drag_touched.get_or_insert_with(HashSet::new);
                            if touched.insert(sel) {
                                if !selected_brushes.contains(&sel) {
                                    selected_brushes.push(sel);
                                } else if let Some(i) = selected_brushes.iter().position(|b| b == &sel) {
                                    selected_brushes.remove(i);
                                }
                            }
                            selected_faces.clear();
                            *selected_entity = Some(sel.0);
                            if let Some(aabb) =
                                selection_aabb_active(map, selected_brushes, selected_faces, edit_faces)
                            {
                                self.last_aabb = Some(aabb.clone());
                                update_last_work_from_aabb(self, &aabb);
                            }
                        }
                    }
                }

                let mut snapped_marker: Option<([f32; 2], u32)> = None;
                if canvas_interacting && ui.is_mouse_down(MouseButton::Left) {
                    let sx = p[0] + w * 0.5 + self.pan[0] + snapped[0] * self.zoom;
                    let sy = p[1] + h * 0.5 + self.pan[1] + snapped[1] * self.zoom;
                    let col = util::imgui_color_to_u32(ui.style_color(StyleColor::TabSelectedOverline));
                    snapped_marker = Some(([sx, sy], col));
                }

                // START drag
                if canvas_interacting
                    && ui.is_mouse_clicked(MouseButton::Left)
                    && !ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                {
                    self.move_offset = Vec3::ZERO;
                    self.stretch = None;
                    self.stretch_delta = Vec3::ZERO;
                    self.rotate = None;
                    self.rotate_angle = 0.0;
                    let has_vertex_selection = edit_vertices && !selected_patch_vertices.is_empty();
                    let has_selection = if edit_faces {
                        !selected_faces.is_empty()
                    } else {
                        !selected_brushes.is_empty()
                    };

                    self.drag_mode = if ui.is_key_down(dear_imgui_rs::Key::LeftAlt) {
                        DragMode::RectangularSelection
                    } else if has_vertex_selection {
                        DragMode::MoveVertices
                    } else if has_selection {
                        if let Some(aabb) =
                            selection_aabb_active(map, selected_brushes, selected_faces, edit_faces)
                        {
                            if click_in_aabb_2d(&aabb, snapped_i, self.ortho_axis) {
                                if rotate_mode {
                                    let center = (aabb.min + aabb.max) * 0.5;
                                    let pivot_uv = project_to_2d(center, self.ortho_axis);
                                    let axis = match self.ortho_axis {
                                        Ortho::XY => Vec3::Z,
                                        Ortho::XZ => Vec3::Y,
                                        Ortho::YZ => Vec3::X,
                                    };
                                    self.rotate = Some(RotateDrag {
                                        selection_aabb: aabb,
                                        pivot_uv,
                                        start_uv: world,
                                        axis,
                                    });
                                    DragMode::RotateSelection
                                } else {
                                    DragMode::MoveSelection
                                }
                            } else {
                                let faces = stretch_faces_from_start(self.ortho_axis, &aabb, snapped_i);
                                self.stretch = Some(StretchDrag {
                                    selection_aabb: aabb,
                                    faces: [faces.get(0).copied(), faces.get(1).copied()],
                                });
                                self.stretch_delta = Vec3::ZERO;
                                DragMode::StretchSelection
                            }
                        } else {
                            if edit_vertices {
                                DragMode::MoveSelection
                            } else {
                                DragMode::NewBrush
                            }
                        }
                    } else {
                        if edit_vertices {
                            DragMode::MoveSelection
                        } else {
                            DragMode::NewBrush
                        }
                    };
                    self.drag_start = Some(snapped_i);
                    self.drag_current = Some(snapped_i);
                }

                // UPDATE drag
                if canvas_interacting
                    && ui.is_mouse_down(MouseButton::Left)
                    && !ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                {
                    if self.drag_start.is_some() {
                        self.drag_current = Some(snapped_i);

                            if self.drag_mode == DragMode::MoveSelection
                                || self.drag_mode == DragMode::MoveVertices
                            {
                                let d = snapped_i - self.drag_start.unwrap();
                                let delta = util::drag_delta_to_3d(d, self.ortho_axis, axis_lock);
                                self.move_offset = delta;
                            } else if self.drag_mode == DragMode::StretchSelection {
                                let d = snapped_i - self.drag_start.unwrap();
                                let mut delta = util::drag_delta_to_3d(d, self.ortho_axis, axis_lock);
                            if let Some(stretch) = self.stretch.as_ref() {
                                delta = util::clamp_stretch_delta(
                                    &stretch.selection_aabb,
                                    stretch.faces,
                                    delta,
                                    config.grid_minor_step as i32,
                                    my_snapping,
                                );
                            }
                            self.stretch_delta = delta;
                            } else if self.drag_mode == DragMode::RotateSelection {
                                if let Some(rot) = self.rotate.as_ref() {
                                    let locked = (rot.axis == Vec3::X && axis_lock.x)
                                        || (rot.axis == Vec3::Y && axis_lock.y)
                                        || (rot.axis == Vec3::Z && axis_lock.z);
                                    let angle = if locked {
                                        0.0
                                    } else {
                                        let v0 = [
                                            rot.start_uv[0] - rot.pivot_uv[0],
                                            rot.start_uv[1] - rot.pivot_uv[1],
                                        ];
                                        let v1 = [world[0] - rot.pivot_uv[0], world[1] - rot.pivot_uv[1]];
                                        let dot = v0[0] * v1[0] + v0[1] * v1[1];
                                        let cross = v0[0] * v1[1] - v0[1] * v1[0];
                                        let mut angle = cross.atan2(dot);
                                        if matches!(self.ortho_axis, Ortho::XY | Ortho::YZ) {
                                            angle = -angle;
                                        }
                                        if my_snapping {
                                            angle = angle.to_degrees().round().to_radians();
                                        }
                                        angle
                                    };
                                    self.rotate_angle = angle;
                                }
                            }

                        let edge_zone = 20.0;
                        let pan_speed = 100.0 * dt;
                        let mouse = ui.mouse_pos();
                        let [rx, ry, rw, rh] = self.rect;

                        if mouse[0] < rx + edge_zone {
                            self.pan[0] += pan_speed;
                        }
                        if mouse[0] > rx + rw - edge_zone {
                            self.pan[0] -= pan_speed;
                        }
                        if mouse[1] < ry + edge_zone {
                            self.pan[1] += pan_speed;
                        }
                        if mouse[1] > ry + rh - edge_zone {
                            self.pan[1] -= pan_speed;
                        }
                    }
                }

                // FINISH drag
                if canvas_interacting && ui.is_mouse_released(MouseButton::Left) {
                    if let (Some(start), Some(end)) = (self.drag_start, self.drag_current) {
                            match self.drag_mode {
                                    DragMode::MoveVertices => {
                                        let d = end - start;
                                        if d != Vec2::ZERO {
                                            let delta = util::drag_delta_to_3d(d, self.ortho_axis, axis_lock);
                                            let can_apply = delta != Vec3::ZERO && !selected_patch_vertices.is_empty();

                                            if can_apply {
                                                undo.push(
                                                    "Move vertices",
                                                    map,
                                                    selected_brushes,
                                                    selected_faces,
                                                    selected_patch_vertices,
                                                    selected_entity,
                                                );
                                            }

                                            if let Some(map) = map.as_mut() {
                                                if can_apply
                                                    && translate_selected_patch_vertices(
                                                        map,
                                                        selected_patch_vertices,
                                                        delta,
                                                    )
                                                {
                                                    log_info!(console, "Moved selected vertices");
                                                }

                                                if can_apply {
                                                    if let Some(aabb) = selection_aabb_patch_vertices_from_map(
                                                        map,
                                                        selected_patch_vertices,
                                                    ) {
                                                        self.last_aabb = Some(aabb.clone());
                                                        update_last_work_from_aabb(self, &aabb);
                                                    }
                                                }
                                            }
                                        }

                                        self.move_offset = Vec3::ZERO;
                                    }
                                    DragMode::MoveSelection => {
                                        let d = end - start;
                                        if d != Vec2::ZERO {
                                            let delta = util::drag_delta_to_3d(d, self.ortho_axis, axis_lock);
                                            let can_apply = if edit_faces {
                                                delta != Vec3::ZERO && !selected_faces.is_empty()
                                            } else {
                                                delta != Vec3::ZERO && !selected_brushes.is_empty()
                                            };

                                            if can_apply {
                                                let label = if edit_faces { "Move faces" } else { "Move selection" };
                                                undo.push(
                                                    label,
                                                    map,
                                                    selected_brushes,
                                                    selected_faces,
                                                    selected_patch_vertices,
                                                    selected_entity,
                                                );
                                            }

                                            if let Some(map) = map.as_mut() {
                                                if edit_faces {
                                                    let any = can_apply
                                                        && translate_selected_faces(map, selected_faces, delta);
                                                    if any {
                                                        log_info!(console, "Moved selected faces");
                                                    }
                                                } else {
                                                let generation = &mut map.generation;
                                                for (entity_idx, brush_idx) in selected_brushes.iter() {
                                                    if let Some(entity) = map.entities.get_mut(*entity_idx) {
                                                        if let Some(brush) = entity.brushes.get_mut(*brush_idx) {
                                                            brush.translate(generation, delta);
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                            if can_apply {
                                                if let Some(aabb) = selection_aabb_active(
                                                    map,
                                                    selected_brushes,
                                                    selected_faces,
                                                    edit_faces,
                                                ) {
                                                    self.last_aabb = Some(aabb.clone());
                                                    update_last_work_from_aabb(self, &aabb);
                                                }
                                            }
                                        }
                                        self.move_offset = Vec3::ZERO;
                                        log_info!(console, "Dragged selection");
                                    }
                                    DragMode::RectangularSelection => {
                                        let min_x = start.x.min(end.x);
                                        let max_x = start.x.max(end.x);
                                        let min_y = start.y.min(end.y);
                                        let max_y = start.y.max(end.y);

                                        if let Some(map) = map.as_mut() {
                                            let toggle = ui.is_key_down(dear_imgui_rs::Key::Z);

                                            if edit_vertices {
                                                // Vertex mode: select patch vertices
                                                if toggle {
                                                    // Toggle mode: remove vertices from selection if they're in the rect
                                                    selected_patch_vertices.retain(|sel| {
                                                        let Some(entity) = map.entities.get(sel.entity_idx) else {
                                                            return true;
                                                        };
                                                        let Some(brush) = entity.brushes.get(sel.brush_idx) else {
                                                            return true;
                                                        };
                                                        let kradiant::map::BrushContent::Patch(patch) = &brush.content else {
                                                            return true;
                                                        };
                                                        let Some(row) = patch.vertices.get(sel.row) else {
                                                            return true;
                                                        };
                                                        let Some(vtx) = row.get(sel.col) else {
                                                            return true;
                                                        };

                                                        let p2 = util::project_to_2d(vtx.position, self.ortho_axis);

                                                        !(p2[0] >= min_x && p2[0] <= max_x && p2[1] >= min_y && p2[1] <= max_y)
                                                    });
                                                } else {
                                                    // Normal mode: clear selection and add vertices in the rect
                                                    selected_patch_vertices.clear();
                                                    selected_brushes.clear();

                                                    for (entity_idx, entity) in map.entities.iter().enumerate() {
                                                        for (brush_idx, brush) in entity.brushes.iter().enumerate() {
                                                            if let kradiant::map::BrushContent::Patch(patch) = &brush.content {
                                                                for (row_idx, row) in patch.vertices.iter().enumerate() {
                                                                    for (col_idx, vtx) in row.iter().enumerate() {
                                                                        let p2 = util::project_to_2d(vtx.position, self.ortho_axis);

                                                                        if p2[0] >= min_x && p2[0] <= max_x && p2[1] >= min_y && p2[1] <= max_y {
                                                                            let sel = crate::ui::PatchVertexSelection {
                                                                                entity_idx,
                                                                                brush_idx,
                                                                                row: row_idx,
                                                                                col: col_idx,
                                                                            };
                                                                            selected_patch_vertices.push(sel);

                                                                            let brush_sel = (entity_idx, brush_idx);
                                                                            if !selected_brushes.contains(&brush_sel) {
                                                                                selected_brushes.push(brush_sel);
                                                                            }
                                                                            *selected_entity = Some(entity_idx);
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }

                                                if let Some(aabb) = selection_aabb_patch_vertices_from_map(
                                                    map,
                                                    selected_patch_vertices,
                                                ) {
                                                    self.last_aabb = Some(aabb.clone());
                                                    update_last_work_from_aabb(self, &aabb);
                                                }
                                            } else if edit_faces {
                                                // Face mode: select faces within the rectangle
                                                if toggle {
                                                    // Toggle mode: remove faces from selection if they're in the rect
                                                    selected_faces.retain(|sel| {
                                                        let Some(entity) = map.entities.get(sel.entity_idx) else {
                                                            return true;
                                                        };
                                                        let Some(brush) = entity.brushes.get(sel.brush_idx) else {
                                                            return true;
                                                        };
                                                        let faces: Option<&Vec<Face>> = match &brush.content {
                                                            BrushContent::Convex(convex) => Some(convex),
                                                            _ => None,
                                                        };
                                                        let Some(faces) = faces else {
                                                            return true;
                                                        };
                                                        if sel.face_idx >= faces.len() {
                                                            return true;
                                                        }
                                                        let face = &faces[sel.face_idx];

                                                        // Only consider faces facing the view
                                                        if !self.is_face_facing_view(face) {
                                                            return true; // Keep the face in selection (not in rect)
                                                        }

                                                        // Check if any vertex of the face is in the rectangle
                                                        let mut any_in_rect = false;
                                                        for pos in &face.plane_points {
                                                            let p2 = util::project_to_2d(*pos, self.ortho_axis);
                                                            if p2[0] >= min_x && p2[0] <= max_x && p2[1] >= min_y && p2[1] <= max_y {
                                                                any_in_rect = true;
                                                                break;
                                                            }
                                                        }

                                                        !any_in_rect
                                                    });
                                                } else {
                                                    // Normal mode: clear selection and add faces in the rect
                                                    selected_faces.clear();
                                                    selected_brushes.clear();

                                                    for (entity_idx, entity) in map.entities.iter().enumerate() {
                                                        for (brush_idx, brush) in entity.brushes.iter().enumerate() {
                                                            let faces: Option<&Vec<Face>> = match &brush.content {
                                                                BrushContent::Convex(convex) => Some(convex),
                                                                _ => None,
                                                            };
                                                            let Some(faces) = faces else {
                                                                continue;
                                                            };
                                                            for (face_idx, face) in faces.iter().enumerate() {
                                                                // Only select faces facing the view (not back faces)
                                                                if !self.is_face_facing_view(face) {
                                                                    continue;
                                                                }

                                                                // Check if any vertex of the face is in the rectangle
                                                                let mut any_in_rect = false;
                                                                for pos in &face.plane_points {
                                                                    let p2 = util::project_to_2d(*pos, self.ortho_axis);
                                                                    if p2[0] >= min_x && p2[0] <= max_x && p2[1] >= min_y && p2[1] <= max_y {
                                                                        any_in_rect = true;
                                                                        break;
                                                                    }
                                                                }

                                                                if any_in_rect {
                                                                    let sel = FaceSelection {
                                                                        entity_idx,
                                                                        brush_idx,
                                                                        face_idx,
                                                                    };
                                                                    selected_faces.push(sel);

                                                                    let brush_sel = (entity_idx, brush_idx);
                                                                    if !selected_brushes.contains(&brush_sel) {
                                                                        selected_brushes.push(brush_sel);
                                                                    }
                                                                    *selected_entity = Some(entity_idx);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }

                                                sync_selected_brushes_from_faces(selected_faces, selected_brushes);
                                                if let Some(aabb) = selection_aabb_active(&mut Some(map.clone()), selected_brushes, selected_faces, edit_faces) {
                                                    self.last_aabb = Some(aabb.clone());
                                                    update_last_work_from_aabb(self, &aabb);
                                                }
                                            } else {
                                                // Normal mode: select brushes within the rectangle
                                                if toggle {
                                                    // Toggle mode: remove brushes from selection if they're in the rect
                                                    selected_brushes.retain(|&(entity_idx, brush_idx)| {
                                                        let Some(entity) = map.entities.get(entity_idx) else {
                                                            return true;
                                                        };
                                                        let Some(brush) = entity.brushes.get(brush_idx) else {
                                                            return true;
                                                        };
                                                        let aabb = &brush.aabb;

                                                        // Check if AABB is in the rectangle
                                                        let (min2, max2) = crate::util::project_aabb_to_2d(&aabb, self.ortho_axis);
                                                        let aabb_min_x = min2.x as f32;
                                                        let aabb_max_x = max2.x as f32;
                                                        let aabb_min_y = min2.y as f32;
                                                        let aabb_max_y = max2.y as f32;

                                                        // Check if AABB overlaps with selection rectangle
                                                        !(aabb_max_x >= min_x && aabb_min_x <= max_x && aabb_max_y >= min_y && aabb_min_y <= max_y)
                                                    });
                                                } else {
                                                    // Normal mode: clear selection and add brushes in the rect
                                                    selected_brushes.clear();
                                                    selected_faces.clear();

                                                    for (entity_idx, entity) in map.entities.iter().enumerate() {
                                                        for (brush_idx, brush) in entity.brushes.iter().enumerate() {
                                                            let aabb = &brush.aabb;
                                                            // Check if AABB is in the rectangle
                                                            let (min2, max2) = crate::util::project_aabb_to_2d(&aabb, self.ortho_axis);
                                                            let aabb_min_x = min2.x as f32;
                                                            let aabb_max_x = max2.x as f32;
                                                            let aabb_min_y = min2.y as f32;
                                                            let aabb_max_y = max2.y as f32;

                                                            // Check if AABB overlaps with selection rectangle
                                                            if aabb_max_x >= min_x && aabb_min_x <= max_x && aabb_max_y >= min_y && aabb_min_y <= max_y {
                                                                selected_brushes.push((entity_idx, brush_idx));
                                                                *selected_entity = Some(entity_idx);
                                                            }
                                                        }
                                                    }
                                                }

                                                if let Some(aabb) = selection_aabb_from_map(map, selected_brushes) {
                                                    self.last_aabb = Some(aabb.clone());
                                                    update_last_work_from_aabb(self, &aabb);
                                                }
                                            }
                                        }
                                    }
                                DragMode::NewBrush => {
                                    let can_create = if edit_faces {
                                        selected_faces.is_empty()
                                    } else {
                                        selected_brushes.is_empty()
                                    };
                                    if can_create {
                                        if map.is_some() {
                                            undo.push(
                                                "Create brush",
                                                map,
                                                selected_brushes,
                                                selected_faces,
                                                selected_patch_vertices,
                                                selected_entity,
                                            );
                                        }
                                        if let Some(created) = create_brush_from_drag(
                                            self,
                                            start,
                                            end,
                                            &mut config,
                                            map,
                                            console
                                        ) {
                                            let diff = (start - end).abs();
                                            if diff.x < 1.0 || diff.y < 1.0 {
                                                log_warn!(console, "Brush planes smaller than `1` not recommended");
                                            }
                                            selected_faces.clear();
                                            selected_brushes.push((0, created.0 as usize));
                                            if let Some(aabb) =
                                                selection_aabb_active(map, selected_brushes, selected_faces, edit_faces)
                                            {
                                                self.last_aabb = Some(aabb.clone());
                                                update_last_work_from_aabb(self, &aabb);
                                            }
                                        }
                                    }
                                }
                            DragMode::StretchSelection => {
                                if let Some(stretch) = self.stretch.take() {
                                    let delta = self.stretch_delta;
                                    if delta != Vec3::ZERO {
                                        let has_selection = if edit_faces {
                                            !selected_faces.is_empty()
                                        } else {
                                            !selected_brushes.is_empty()
                                        };

                                        if map.is_some() && has_selection {
                                            let label = if edit_faces {
                                                match stretch_mode {
                                                    StretchMode::Scale => "Scale faces",
                                                    StretchMode::Resize => "Resize faces",
                                                }
                                            } else {
                                                match stretch_mode {
                                                    StretchMode::Scale => "Scale selection",
                                                    StretchMode::Resize => "Resize selection",
                                                }
                                            };
                                            undo.push(
                                                label,
                                                map,
                                                selected_brushes,
                                                selected_faces,
                                                selected_patch_vertices,
                                                selected_entity,
                                            );
                                        }

                                        if let Some(map) = map.as_mut() {
                                            let mut new_sel_aabb: Option<Aabb> = None;

                                            if edit_faces {
                                                let mut any = false;
                                                match stretch_mode {
                                                    StretchMode::Scale => {
                                                        if let Some((xform, _preview)) =
                                                            editing::stretch_selection_transform(
                                                                &stretch.selection_aabb,
                                                                stretch.faces,
                                                                delta,
                                                            )
                                                        {
                                                            any = apply_affine_scale_to_selected_faces(
                                                                map,
                                                                selected_faces,
                                                                xform,
                                                            );
                                                        }
                                                    }
                                                    StretchMode::Resize => {
                                                        for (entity_idx, brush_idx) in selected_brushes.iter() {
                                                            let Some(entity) =
                                                                map.entities.get_mut(*entity_idx)
                                                            else {
                                                                continue;
                                                            };
                                                            let Some(brush) =
                                                                entity.brushes.get_mut(*brush_idx)
                                                            else {
                                                                continue;
                                                            };
                                                            match &brush.content {
                                                                kradiant::map::BrushContent::Convex(_) => {
                                                                    if editing::stretch_convex_brush_faces(
                                                                        brush,
                                                                        &mut map.generation,
                                                                        stretch.faces,
                                                                        delta,
                                                                        config.grid_minor_step as i32,
                                                                    ) {
                                                                        any = true;
                                                                    }
                                                                }
                                                                kradiant::map::BrushContent::Patch(_patch) => {
                                                                    if let Some((xform, _)) =
                                                                        editing::stretch_selection_transform(
                                                                            &stretch.selection_aabb,
                                                                            stretch.faces,
                                                                            delta,
                                                                        )
                                                                    {
                                                                        if editing::apply_affine_scale_to_brush(
                                                                            brush,
                                                                            &mut map.generation,
                                                                            xform,
                                                                        ) {
                                                                            any = true;
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }

                                                if any {
                                                    new_sel_aabb =
                                                        selection_aabb_faces_from_map(map, selected_faces);
                                                    log_info!(console, "Stretched selected faces");
                                                }
                                            } else {
                                                let mut any = false;
                                                match stretch_mode {
                                                    StretchMode::Scale => {
                                                        if let Some((xform, _preview)) =
                                                            editing::stretch_selection_transform(
                                                                &stretch.selection_aabb,
                                                                stretch.faces,
                                                                delta,
                                                            )
                                                        {
                                                            for (entity_idx, brush_idx) in selected_brushes.iter() {
                                                                let Some(entity) = map.entities.get_mut(*entity_idx)
                                                                    else {
                                                                    continue;
                                                                };
                                                                let Some(brush) =
                                                                    entity.brushes.get_mut(*brush_idx)
                                                                else {
                                                                    continue;
                                                                };
                                                                if editing::apply_affine_scale_to_brush(
                                                                    brush,
                                                                    &mut map.generation,
                                                                    xform,
                                                                ) {
                                                                    any = true;
                                                                }
                                                            }
                                                        }
                                                    }
                                                    StretchMode::Resize => {
                                                        for (entity_idx, brush_idx) in &mut *selected_brushes {
                                                            let Some(entity) = map.entities.get_mut(*entity_idx)
                                                                else {
                                                                continue;
                                                            };
                                                            let Some(brush) = entity.brushes.get_mut(*brush_idx)
                                                                else {
                                                                continue;
                                                            };
                                                            match &brush.content {
                                                                kradiant::map::BrushContent::Convex(_) => {
                                                                    if editing::stretch_convex_brush_faces(
                                                                        brush,
                                                                        &mut map.generation,
                                                                        stretch.faces,
                                                                        delta,
                                                                        config.grid_minor_step as i32,
                                                                    ) {
                                                                        any = true;
                                                                    }
                                                                }
                                                                kradiant::map::BrushContent::Patch(_patch) => {
                                                                    if let Some((xform, _)) =
                                                                        editing::stretch_selection_transform(
                                                                            &stretch.selection_aabb,
                                                                            stretch.faces,
                                                                            delta,
                                                                        )
                                                                    {
                                                                        if editing::apply_affine_scale_to_brush(
                                                                            brush,
                                                                            &mut map.generation,
                                                                            xform,
                                                                        ) {
                                                                            any = true;
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }

                                                if any {
                                                    new_sel_aabb =
                                                        selection_aabb_from_map(map, &selected_brushes);
                                                    log_info!(console, "Stretched selection");
                                                }
                                            }

                                            if let Some(aabb) = new_sel_aabb {
                                                self.last_aabb = Some(aabb.clone());
                                                update_last_work_from_aabb(self, &aabb);
                                            }
                                        }
                                    }
                                }
                                self.stretch_delta = Vec3::ZERO;
                            }
                            DragMode::RotateSelection => {
                                if let Some(rot) = self.rotate.take() {
                                    let angle = if self.rotation_locked(axis_lock) {
                                        0.0
                                    } else {
                                        self.rotate_angle
                                    };

                                    if angle.abs() > 1.0e-6 {
                                        let has_selection = if edit_faces {
                                            !selected_faces.is_empty()
                                        } else {
                                            !selected_brushes.is_empty()
                                        };
                                        if map.is_some() && has_selection {
                                            let label = if edit_faces { "Rotate faces" } else { "Rotate selection" };
                                            undo.push(
                                                label,
                                                map,
                                                selected_brushes,
                                                selected_faces,
                                                selected_patch_vertices,
                                                selected_entity,
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
                                                    log_info!(console, "Rotated selected faces");
                                                }
                                            } else if let Some((xform, _preview)) =
                                                editing::rotate_selection_transform(
                                                    &rot.selection_aabb,
                                                    axis,
                                                    angle,
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

                                                if any {
                                                    new_sel_aabb = selection_aabb_from_map(map, &selected_brushes);
                                                    log_info!(console, "Rotated selection");
                                                }
                                            }
                                        }
                                        if let Some(aabb) = new_sel_aabb {
                                            self.last_aabb = Some(aabb.clone());
                                            update_last_work_from_aabb(self, &aabb);
                                        }
                                    }
                                }
                                self.rotate_angle = 0.0;
                            }
                        }
                    }
                    self.drag_start = None;
                    self.drag_current = None;
                    self.stretch = None;
                    self.stretch_delta = Vec3::ZERO;
                    self.rotate = None;
                    self.rotate_angle = 0.0;
                    self.selection_drag_touched = None;
                    self.selection_drag_touched_faces = None;
                    self.selection_drag_touched_verts = None;
                }

                if ui.is_window_hovered()
                    && edit_faces
                    && map.is_some()
                    && !selected_faces.is_empty()
                    && self.drag_start.is_none()
                {
                    let step = (config.grid_minor_step as f32).max(1.0);
                    let depth_axis = match self.ortho_axis {
                        Ortho::XY => Vec3::Z,
                        Ortho::XZ => Vec3::Y,
                        Ortho::YZ => Vec3::X,
                    };
                    let mut delta = Vec3::ZERO;
                    if ui.is_key_pressed(dear_imgui_rs::Key::PageUp) {
                        delta += depth_axis * step;
                    }
                    if ui.is_key_pressed(dear_imgui_rs::Key::PageDown) {
                        delta -= depth_axis * step;
                    }
                    if axis_lock.x {
                        delta.x = 0.0;
                    }
                    if axis_lock.y {
                        delta.y = 0.0;
                    }
                    if axis_lock.z {
                        delta.z = 0.0;
                    }

                    if delta != Vec3::ZERO {
                        undo.push(
                            "Move faces",
                            map,
                            selected_brushes,
                            selected_faces,
                            selected_patch_vertices,
                            selected_entity,
                        );
                        if let Some(map) = map.as_mut() {
                            if translate_selected_faces(map, selected_faces, delta) {
                                log_info!(console, "Moved selected faces");
                                if let Some(aabb) =
                                    selection_aabb_faces_from_map(map, selected_faces)
                                {
                                    self.last_aabb = Some(aabb.clone());
                                    update_last_work_from_aabb(self, &aabb);
                                }
                            }
                        }
                    }
                }

                if ui.is_key_pressed(dear_imgui_rs::Key::Escape) {
                    if let Some(aabb) =
                        selection_aabb_active(map, selected_brushes, selected_faces, edit_faces)
                    {
                        self.last_aabb = Some(aabb.clone());
                        update_last_work_from_aabb(self, &aabb);
                    }
                    if edit_faces {
                        selected_faces.clear();
                        sync_selected_brushes_from_faces(selected_faces, selected_brushes);
                    } else {
                        selected_brushes.clear();
                    }
                    if edit_vertices {
                        selected_patch_vertices.clear();
                    }
                    self.stretch = None;
                    self.stretch_delta = Vec3::ZERO;
                    self.rotate = None;
                    self.rotate_angle = 0.0;
                    self.move_offset = Vec3::ZERO;
                }

                if ui.is_key_pressed(dear_imgui_rs::Key::Backspace) {
                    if edit_faces {
                        selected_faces.clear();
                        sync_selected_brushes_from_faces(selected_faces, selected_brushes);
                    } else {
                        if let Some(aabb) =
                            selection_aabb_active(map, selected_brushes, selected_faces, edit_faces)
                        {
                            self.last_aabb = Some(aabb.clone());
                            update_last_work_from_aabb(self, &aabb);
                        }

                        if map.is_some() && !selected_brushes.is_empty() {
                            undo.push(
                                "Delete selection",
                                map,
                                selected_brushes,
                                selected_faces,
                                selected_patch_vertices,
                                selected_entity,
                            );
                        }
                        if let Some(map) = map.as_mut() {
                            let mut any = false;
                            use std::collections::BTreeMap;
                            let mut by_entity: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
                            for (entity_idx, brush_idx) in &*selected_brushes {
                                by_entity.entry(*entity_idx).or_default().push(*brush_idx);
                            }

                            for brush_indices in by_entity.values_mut() {
                                brush_indices.sort_unstable();
                                brush_indices.dedup();
                                brush_indices.sort_unstable_by(|a, b| b.cmp(a));
                            }

                            for (entity_idx, brush_indices) in by_entity {
                                let Some(entity) = map.entities.get_mut(entity_idx) else {
                                    continue;
                                };
                                for brush_idx in brush_indices {
                                    if brush_idx < entity.brushes.len() {
                                        entity.brushes.swap_remove(brush_idx);
                                        if !any {
                                            any = true; // we actually deleted something
                                        }
                                    }
                                }
                            }

                            if any {
                                map.generation = map.generation.wrapping_add(1);
                                log_info!(console, "Deleted selected brushes");
                            }
                        }

                        selected_brushes.clear();
                        selected_patch_vertices.clear();
                        *selected_entity = None;
                    }
                }

                // Render canvas
                draw.with_clip_rect(p, [p[0] + w, p[1] + h], || {
                    draw.add_rect(p, [p[0] + w, p[1] + h], util::imgui_color_to_u32(palette.view2d_bg))
                        .filled(true)
                        .build();

                    if let Some(tid) = self.tex_id {
                        draw.add_image(tid, p, [p[0] + w, p[1] + h], [0.0, 1.0], [1.0, 0.0], 0xFFFFFFFFu32);
                    }

                    draw.add_text(
                        [p[0] + 8.0, p[1] + 6.0],
                        util::imgui_color_to_u32(palette.hud_text),
                        self.ortho_axis.label(),
                    );
                    if canvas_interacting {
                        draw.add_text(
                            [p[0] + 8.0, p[1] + 24.0],
                            util::imgui_color_to_u32(palette.hud_text_dim),
                            format!("{:.1}, {:.1}", world_axis[0], world_axis[1]),
                        );
                    }

                    draw.add_text(
                        [p[0] + 100.0, p[1] + 6.0],
                        util::imgui_color_to_u32(palette.hud_text_dim),
                        format!("{:.2} FPS (average)", ui.io().framerate()),
                    );

                    if ui.is_window_hovered() {
                        if ui.is_key_pressed(dear_imgui_rs::Key::Key1) {
                            config.update("grid_minor_step", 1u8, console);
                        }
                        if ui.is_key_pressed(dear_imgui_rs::Key::Key2) {
                            config.update("grid_minor_step", 2u8, console);
                        }
                        if ui.is_key_pressed(dear_imgui_rs::Key::Key3) {
                            config.update("grid_minor_step", 4u8, console);
                        }
                        if ui.is_key_pressed(dear_imgui_rs::Key::Key4) {
                            config.update("grid_minor_step", 8u8, console);
                        }
                        if ui.is_key_pressed(dear_imgui_rs::Key::Key5) {
                            config.update("grid_minor_step", 16u8, console);
                        }
                        if ui.is_key_pressed(dear_imgui_rs::Key::Key6) {
                            config.update("grid_minor_step", 32u8, console);
                        }
                        if ui.is_key_pressed(dear_imgui_rs::Key::Key7) {
                            config.update("grid_minor_step", 64u8, console);
                        }
                        if ui.is_key_pressed(dear_imgui_rs::Key::Key8) {
                            config.update("grid_minor_step", 128u8, console);
                        }
                    }

                    if let Some((pos, col)) = snapped_marker {
                        draw.add_circle(pos, 4.0, col).filled(true).build();
                    }

                    // Render drag preview
                    if let (Some(start), Some(end)) = (self.drag_start, self.drag_current) {
                        let col = ui.style_color(StyleColor::TabSelectedOverline);
                        let to_screen = |v: Vec2| -> [f32; 2] {
                            [
                                p[0] + w * 0.5 + self.pan[0] + v.x * self.zoom,
                                p[1] + h * 0.5 + self.pan[1] + v.y * self.zoom,
                            ]
                        };
                        let to_screen_f = |v: [f32; 2]| -> [f32; 2] {
                            [
                                p[0] + w * 0.5 + self.pan[0] + v[0] * self.zoom,
                                p[1] + h * 0.5 + self.pan[1] + v[1] * self.zoom,
                            ]
                        };

                        match self.drag_mode {
                                DragMode::MoveSelection | DragMode::MoveVertices => {
                                    let (a, b) = if let Some(sel) =
                                        selection_aabb_active(map, selected_brushes, selected_faces, edit_faces)
                                    {
                                        let (min2, max2) = crate::util::project_aabb_to_2d(&sel, self.ortho_axis);
                                        let center = [
                                            (min2.x as f32 + max2.x as f32) * 0.5,
                                            (min2.y as f32 + max2.y as f32) * 0.5,
                                        ];
                                    let off2 = project_to_2d(self.move_offset, self.ortho_axis);
                                    let a = to_screen_f(center);
                                    let b = to_screen_f([center[0] + off2[0], center[1] + off2[1]]);
                                    (a, b)
                                } else {
                                    (to_screen(start), to_screen(end))
                                };

                                let off = self.move_offset;
                                let (dx, dy) = match self.ortho_axis {
                                    Ortho::XY => (off.x, off.y),
                                    Ortho::XZ => (off.x, -off.z),
                                    Ortho::YZ => (off.y, -off.z),
                                };
                                let delta_info = format!("({}, {})", format_float(dx, 2), format_float(dy, 2));

                                let tw = text_width(ui, &delta_info);
                                let text_h = text_height(ui, "1") + 2.0;
                                let text_pos = [b[0] - tw / 2.0, b[1] - text_h];

                                let delta_info_col = util::imgui_color_to_u32(col);
                                draw.add_line(a, b, util::adjust_color_brightness(delta_info_col, 1.5))
                                    .thickness(2.0)
                                    .build();
                                draw.add_text(text_pos, util::adjust_color_brightness(delta_info_col, 2.0), delta_info);
                            }
                            DragMode::NewBrush => {
                                let min = start.min(end);
                                let max = start.max(end);
                                let a = to_screen(min);
                                let b = to_screen(max);
                                draw.add_rect(a, b, util::imgui_color_to_u32(col))
                                    .thickness(2.0)
                                    .build();
                            }
                            DragMode::StretchSelection => {
                                if let Some(stretch) = self.stretch.as_ref() {
                                    let preview = editing::preview_stretched_aabb(
                                        &stretch.selection_aabb,
                                        stretch.faces,
                                        self.stretch_delta,
                                    );
                                    let (min2, max2) = crate::util::project_aabb_to_2d(&preview, self.ortho_axis);
                                    let a = to_screen(min2);
                                    let b = to_screen(max2);

                                    if let (Some(p0), Some(p1)) = (
                                        util::stretch_handle_point_2d(
                                            &stretch.selection_aabb,
                                            self.ortho_axis,
                                            stretch.faces,
                                        ),
                                        util::stretch_handle_point_2d(&preview, self.ortho_axis, stretch.faces),
                                    ) {
                                        let a = to_screen_f(p0);
                                        let b = to_screen_f(p1);

                                        let du = format_float(p1[0] - p0[0], 2);
                                        let dv = format_float(p1[1] - p0[1], 2);
                                        let delta_info = format!("({}, {})", du, dv);

                                        let tw = text_width(ui, &delta_info);
                                        let d_info_pos = [b[0] - tw - 12.0, b[1] + 4.0];

                                        let delta_info_col = util::imgui_color_to_u32(col);
                                        draw.add_line(a, b, util::adjust_color_brightness(delta_info_col, 1.5))
                                            .thickness(2.0)
                                            .build();
                                        draw.add_text(
                                            d_info_pos,
                                            util::adjust_color_brightness(delta_info_col, 2.0),
                                            delta_info,
                                        );
                                    }

                                    draw.add_rect(a, b, util::imgui_color_to_u32(col))
                                        .thickness(2.0)
                                        .build();
                                } else {
                                    let min = start.min(end);
                                    let max = start.max(end);
                                    let a = to_screen(min);
                                    let b = to_screen(max);
                                    draw.add_rect(a, b, util::imgui_color_to_u32(col))
                                        .thickness(2.0)
                                        .build();
                                }
                            }
                            DragMode::RotateSelection => {
                                if let Some(rot) = self.rotate.as_ref() {
                                    let a = to_screen_f(rot.pivot_uv);
                                    let b = to_screen(end);

                                    let deg = self.rotate_angle.to_degrees();
                                    let delta_info = format!("{deg:.1}°");
                                    let tw = text_width(ui, &delta_info);
                                    let text_h = text_height(ui, "1") + 2.0;
                                    let d_info_pos = [b[0] - tw / 2.0, b[1] - text_h];

                                    let delta_info_col = util::imgui_color_to_u32(col);
                                    draw.add_line(a, b, util::adjust_color_brightness(delta_info_col, 1.5))
                                        .thickness(2.0)
                                        .build();
                                    draw.add_text(
                                        d_info_pos,
                                        util::adjust_color_brightness(delta_info_col, 2.0),
                                        delta_info,
                                    );
                                }
                            }
                            DragMode::RectangularSelection => {
                                let min = start.min(end);
                                let max = start.max(end);
                                let tl = to_screen(min);
                                let br = to_screen(max);
                                let (tr, bl) = util::other_corners(tl, br);
                                let rect_col = util::adjust_color_opacity(util::imgui_color_to_u32(selection_rect_rgba), 0.4);
                                draw.add_quad_filled(tl, tr, br, bl, rect_col);
                            }
                        }
                    }

                        if let Some(mut selection_aabb) =
                            selection_aabb_active(map, selected_brushes, selected_faces, edit_faces)
                        {
                        if self.drag_mode == DragMode::StretchSelection {
                            if let Some(stretch) = self.stretch.as_ref() {
                                selection_aabb = editing::preview_stretched_aabb(
                                    &stretch.selection_aabb,
                                    stretch.faces,
                                    self.stretch_delta,
                                );
                            }
                            } else if self.drag_mode == DragMode::RotateSelection {
                                if let Some(rot) = self.rotate.as_ref() {
                                    let axis = rot.axis;
                                    if let Some((_xform, preview)) = editing::rotate_selection_transform(
                                        &rot.selection_aabb,
                                        axis,
                                        self.rotate_angle,
                                ) {
                                    selection_aabb = preview;
                                }
                            }
                        }

                        let off = if self.drag_mode == DragMode::MoveSelection {
                            project_to_2d(self.move_offset, self.ortho_axis)
                        } else {
                            [0.0, 0.0]
                        };

                        let (min_x, max_x, min_y, max_y) = match self.ortho_axis {
                            Ortho::XY => (
                                selection_aabb.min.x + off[0],
                                selection_aabb.max.x + off[0],
                                selection_aabb.min.y + off[1],
                                selection_aabb.max.y + off[1],
                            ),
                            Ortho::XZ => (
                                selection_aabb.min.x + off[0],
                                selection_aabb.max.x + off[0],
                                -selection_aabb.max.z + off[1],
                                -selection_aabb.min.z + off[1],
                            ),
                            Ortho::YZ => (
                                selection_aabb.min.y + off[0],
                                selection_aabb.max.y + off[0],
                                -selection_aabb.max.z + off[1],
                                -selection_aabb.min.z + off[1],
                            ),
                        };

                        let width = (max_x - min_x).abs();
                        let height = (max_y - min_y).abs();

                        let col_u32 = util::imgui_color_to_u32(selection_rgba);
                        let to_screen_f = |v: [f32; 2]| -> [f32; 2] {
                            [
                                p[0] + w * 0.5 + self.pan[0] + v[0] * self.zoom,
                                p[1] + h * 0.5 + self.pan[1] + v[1] * self.zoom,
                            ]
                        };

                        let bottom = to_screen_f([(min_x + max_x) * 0.5, max_y]);
                        let right = to_screen_f([max_x, (min_y + max_y) * 0.5]);

                        let w_text = format_float(width, 2);
                        let h_text = format_float(height, 2);
                        let w_tw = text_width(ui, &w_text);
                        //let h_tw = text_width(ui, &h_text);

                        draw.add_text([bottom[0] - w_tw * 0.5, bottom[1] + 8.0], col_u32, w_text);
                        draw.add_text([right[0] + 12.0 - 4.0, right[1] - 7.0], col_u32, h_text);

                        // the handle bars (c) raph
                        let v0 = to_screen_f([max_x, min_y]);
                        let v1 = to_screen_f([max_x, max_y]);
                        draw.add_line([v0[0] + 3.0, v0[1]], [v1[0] + 3.0, v1[1]], col_u32)
                        .build(); //.thickness(1.0).build();

                        let h0 = to_screen_f([min_x, max_y]);
                        let h1 = to_screen_f([max_x, max_y]);
                        draw.add_line([h0[0], h0[1] + 3.5], [h1[0], h1[1] + 3.5], col_u32)
                        .build();
                    }
                });
            });
    }
}

fn sync_selected_brushes_from_faces(
    selected_faces: &[FaceSelection],
    selected_brushes: &mut Vec<(usize, usize)>,
) {
    use std::collections::BTreeSet;

    let mut set: BTreeSet<(usize, usize)> = BTreeSet::new();
    for sel in selected_faces {
        set.insert((sel.entity_idx, sel.brush_idx));
    }
    selected_brushes.clear();
    selected_brushes.extend(set.into_iter());
}

fn selection_aabb_faces_from_map(
    map: &mut kradiant::map::Map,
    selected_faces: &[FaceSelection],
) -> Option<Aabb> {
    if selected_faces.is_empty() {
        return None;
    }

    use std::collections::BTreeMap;
    let mut by_brush: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for sel in selected_faces {
        by_brush
            .entry((sel.entity_idx, sel.brush_idx))
            .or_default()
            .push(sel.face_idx);
    }

    let mut out = Aabb {
        min: Vec3::new(f32::MAX, f32::MAX, f32::MAX),
        max: Vec3::new(f32::MIN, f32::MIN, f32::MIN),
    };

    let mut any = false;
    for ((entity_idx, brush_idx), mut face_indices) in by_brush {
        face_indices.sort_unstable();
        face_indices.dedup();

        let Some(entity) = map.entities.get_mut(entity_idx) else {
            continue;
        };
        let Some(brush) = entity.brushes.get_mut(brush_idx) else {
            continue;
        };
        let Some((_aabb, polys)) = brush.get_polygons_and_aabb() else {
            continue;
        };

        for face_idx in face_indices {
            let Some((verts, _indices)) = polys.get(face_idx) else {
                continue;
            };
            for v in verts {
                out.min = out.min.min(*v);
                out.max = out.max.max(*v);
                any = true;
            }
        }
    }

    any.then_some(out)
}

fn selection_aabb_faces(
    map: &mut Option<kradiant::map::Map>,
    selected_faces: &[FaceSelection],
) -> Option<Aabb> {
    let map = map.as_mut()?;
    selection_aabb_faces_from_map(map, selected_faces)
}

fn selection_aabb_active(
    map: &mut Option<kradiant::map::Map>,
    selected_brushes: &[(usize, usize)],
    selected_faces: &[FaceSelection],
    edit_faces: bool,
) -> Option<Aabb> {
    if edit_faces {
        selection_aabb_faces(map, selected_faces)
    } else {
        selection_aabb(&*map, selected_brushes)
    }
}

fn selection_aabb_patch_vertices_from_map(
    map: &kradiant::map::Map,
    selected: &[crate::ui::PatchVertexSelection],
) -> Option<Aabb> {
    if selected.is_empty() {
        return None;
    }

    let mut out = Aabb {
        min: Vec3::splat(f32::MAX),
        max: Vec3::splat(f32::MIN),
    };
    let mut any = false;

    for sel in selected {
        let Some(entity) = map.entities.get(sel.entity_idx) else {
            continue;
        };
        let Some(brush) = entity.brushes.get(sel.brush_idx) else {
            continue;
        };
        let kradiant::map::BrushContent::Patch(patch) = &brush.content else {
            continue;
        };
        let Some(row) = patch.vertices.get(sel.row) else {
            continue;
        };
        let Some(vtx) = row.get(sel.col) else {
            continue;
        };
        out.min = out.min.min(vtx.position);
        out.max = out.max.max(vtx.position);
        any = true;
    }

    any.then_some(out)
}

fn translate_selected_patch_vertices(
    map: &mut kradiant::map::Map,
    selected: &[crate::ui::PatchVertexSelection],
    delta: Vec3,
) -> bool {
    if selected.is_empty() || delta == Vec3::ZERO {
        return false;
    }

    use std::collections::BTreeMap;
    let mut by_brush: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
    for sel in selected {
        by_brush
            .entry((sel.entity_idx, sel.brush_idx))
            .or_default()
            .push((sel.row, sel.col));
    }

    let mut any = false;
    for ((entity_idx, brush_idx), mut verts) in by_brush {
        verts.sort_unstable();
        verts.dedup();

        let Some(entity) = map.entities.get_mut(entity_idx) else {
            continue;
        };
        let Some(brush) = entity.brushes.get_mut(brush_idx) else {
            continue;
        };
        let kradiant::map::BrushContent::Patch(patch) = &mut brush.content else {
            continue;
        };

        let mut brush_any = false;
        for (row, col) in verts {
            let Some(mut vtx) = patch.vertices.get(row).and_then(|r| r.get(col)).copied() else {
                continue;
            };
            vtx.position += delta;
            patch.update_vertex(&mut map.generation, row, col, vtx);
            brush_any = true;
        }

        if brush_any {
            if let Some((_mesh, aabb, _edges)) = patch.get_mesh_aabb_wire() {
                brush.aabb = aabb.clone();
            }
            any = true;
        }
    }

    any
}

fn pick_patch_control_vertex_by_screen(
    map: &kradiant::map::Map,
    prefer: Option<&[(usize, usize)]>,
    axis: Ortho,
    mouse_screen: [f32; 2],
    origin: [f32; 2],
    size: [f32; 2],
    zoom: f32,
    pan: [f32; 2],
    radius_px: f32,
) -> Option<crate::ui::PatchVertexSelection> {
    let r2 = radius_px.max(1.0) * radius_px.max(1.0);
    let mut best: Option<(crate::ui::PatchVertexSelection, f32)> = None;

    let mut visit_patch = |entity_idx: usize, brush_idx: usize, patch: &kradiant::map::Patch| {
        for (row_idx, row) in patch.vertices.iter().enumerate() {
            for (col_idx, v) in row.iter().enumerate() {
                let p2 = crate::util::project_to_2d(v.position, axis);
                let s = crate::util::world_to_screen(p2, origin, size[0], size[1], zoom, pan);
                let dx = s[0] - mouse_screen[0];
                let dy = s[1] - mouse_screen[1];
                let d2 = dx * dx + dy * dy;
                if d2 > r2 {
                    continue;
                }

                let sel = crate::ui::PatchVertexSelection {
                    entity_idx,
                    brush_idx,
                    row: row_idx,
                    col: col_idx,
                };
                match best {
                    None => best = Some((sel, d2)),
                    Some((_, best_d2)) if d2 < best_d2 => best = Some((sel, d2)),
                    _ => {}
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

    best.map(|(sel, _)| sel)
}

fn click_in_aabb_2d(aabb: &Aabb, pt: Vec2, ortho: Ortho) -> bool {
    let (min2, max2) = crate::util::project_aabb_to_2d(aabb, ortho);
    pt.x >= min2.x && pt.x <= max2.x && pt.y >= min2.y && pt.y <= max2.y
}

fn face_plane_normal(plane_points: [Vec3; 3]) -> Option<Vec3> {
    let n = (plane_points[1] - plane_points[0]).cross(plane_points[2] - plane_points[0]);
    let len2 = n.length_squared();
    if len2 <= 1.0e-10 || !len2.is_finite() {
        return None;
    }
    Some(n / len2.sqrt())
}

fn apply_face_weighted_point_transform_to_convex_brush<F>(
    brush: &mut kradiant::map::Brush,
    face_idx: usize,
    transform_point: F,
) -> bool
where
    F: Fn(Vec3) -> Vec3,
{
    let (face_plane, face_n) = match &brush.content {
        kradiant::map::BrushContent::Convex(faces) => {
            let Some(face) = faces.get(face_idx) else {
                return false;
            };
            let Some(n) = face_plane_normal(face.plane_points) else {
                return false;
            };
            (face.plane_points, n)
        }
        kradiant::map::BrushContent::Patch(_) => return false,
    };

    let mut tmp = brush.clone();

    let Some((_aabb, polys)) = tmp.get_polygons_and_aabb() else {
        return false;
    };
    let p0 = face_plane[0];

    let mut min_d = f32::INFINITY;
    let mut max_d = f32::NEG_INFINITY;
    for (verts, _) in polys {
        for v in verts {
            let d = v.dot(face_n);
            min_d = min_d.min(d);
            max_d = max_d.max(d);
        }
    }
    let span = max_d - min_d;
    if !span.is_finite() || span.abs() < 1.0e-6 {
        return false;
    }

    let face_d = p0.dot(face_n);
    let selected_is_min = (face_d - min_d).abs() <= (face_d - max_d).abs();

    let old_planes: Vec<[Vec3; 3]> = match &tmp.content {
        kradiant::map::BrushContent::Convex(faces) => {
            faces.iter().map(|f| f.plane_points).collect()
        }
        kradiant::map::BrushContent::Patch(_) => return false,
    };

    let xform = |pt: Vec3| -> Vec3 {
        let alpha = ((pt.dot(face_n) - min_d) / span).clamp(0.0, 1.0);
        let w = if selected_is_min { 1.0 - alpha } else { alpha };
        let target = transform_point(pt);
        pt + (target - pt) * w
    };

    let mut dummy_gen = 0u64;
    for (i, p) in old_planes.into_iter().enumerate() {
        let new_plane = [xform(p[0]), xform(p[1]), xform(p[2])];
        tmp.update_brush_plane(&mut dummy_gen, i, new_plane);
    }

    if tmp.get_polygons_and_aabb().is_some() {
        *brush = tmp;
        true
    } else {
        false
    }
}

fn apply_face_weighted_twist_rotate_to_convex_brush(
    brush: &mut kradiant::map::Brush,
    face_idx: usize,
    pivot: Vec3,
    axis: Vec3,
    angle_rad: f32,
) -> bool {
    if angle_rad.abs() <= 1.0e-8 {
        return false;
    }

    let axis_len2 = axis.length_squared();
    if axis_len2 <= 1.0e-10 || !axis_len2.is_finite() {
        return false;
    }
    let axis_n = axis / axis_len2.sqrt();

    let (face_plane, face_n) = match &brush.content {
        kradiant::map::BrushContent::Convex(faces) => {
            let Some(face) = faces.get(face_idx) else {
                return false;
            };
            let Some(n) = face_plane_normal(face.plane_points) else {
                return false;
            };
            (face.plane_points, n)
        }
        kradiant::map::BrushContent::Patch(_) => return false,
    };

    let mut tmp = brush.clone();
    let Some((_aabb, polys)) = tmp.get_polygons_and_aabb() else {
        return false;
    };

    let p0 = face_plane[0];

    let mut min_d = f32::INFINITY;
    let mut max_d = f32::NEG_INFINITY;
    for (verts, _) in polys {
        for v in verts {
            let d = v.dot(face_n);
            min_d = min_d.min(d);
            max_d = max_d.max(d);
        }
    }
    let span = max_d - min_d;
    if !span.is_finite() || span.abs() < 1.0e-6 {
        return false;
    }

    let face_d = p0.dot(face_n);
    let selected_is_min = (face_d - min_d).abs() <= (face_d - max_d).abs();

    let old_planes: Vec<[Vec3; 3]> = match &tmp.content {
        kradiant::map::BrushContent::Convex(faces) => {
            faces.iter().map(|f| f.plane_points).collect()
        }
        kradiant::map::BrushContent::Patch(_) => return false,
    };

    let xform = |pt: Vec3| -> Vec3 {
        let alpha = ((pt.dot(face_n) - min_d) / span).clamp(0.0, 1.0);
        let w = if selected_is_min { 1.0 - alpha } else { alpha };
        if w <= 0.0 {
            return pt;
        }
        let phi = angle_rad * w;
        let q = Quat::from_axis_angle(axis_n, phi);
        pivot + q * (pt - pivot)
    };

    let mut dummy_gen = 0u64;
    for (i, p) in old_planes.into_iter().enumerate() {
        let new_plane = [xform(p[0]), xform(p[1]), xform(p[2])];
        tmp.update_brush_plane(&mut dummy_gen, i, new_plane);
    }

    if tmp.get_polygons_and_aabb().is_some() {
        *brush = tmp;
        true
    } else {
        false
    }
}

fn centroid(verts: &[Vec3]) -> Option<Vec3> {
    if verts.is_empty() {
        return None;
    }
    let mut sum = Vec3::ZERO;
    for v in verts {
        sum += *v;
    }
    Some(sum / verts.len() as f32)
}

fn outward_plane_points(mut pts: [Vec3; 3], center: Vec3) -> [Vec3; 3] {
    let n = (pts[1] - pts[0]).cross(pts[2] - pts[0]);
    if n.dot(center - pts[0]) > 0.0 {
        pts.swap(1, 2);
    }
    pts
}

fn uv_basis_from_axis(axis_n: Vec3) -> (Vec3, Vec3) {
    let ref_axis = if axis_n.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let u = axis_n.cross(ref_axis).normalize_or_zero();
    let v = axis_n.cross(u).normalize_or_zero();
    (u, v)
}

fn twist_rotate_face_antiprism(
    brush: &mut kradiant::map::Brush,
    face_idx: usize,
    axis: Vec3,
    angle_rad: f32,
) -> Option<usize> {
    if angle_rad.abs() <= 1.0e-8 {
        return Some(face_idx);
    }

    let axis_len2 = axis.length_squared();
    if axis_len2 <= 1.0e-10 || !axis_len2.is_finite() {
        return None;
    }
    let axis_n = axis / axis_len2.sqrt();

    let (orig_faces, selected_face, selected_face_normal) = match &brush.content {
        kradiant::map::BrushContent::Convex(faces) => {
            let selected_face = faces.get(face_idx)?.clone();
            let n = face_plane_normal(selected_face.plane_points)?;
            (faces.clone(), selected_face, n)
        }
        kradiant::map::BrushContent::Patch(_) => return None,
    };

    // Only handle the "twist" case: rotating a face around (roughly) its own normal.
    if selected_face_normal.dot(axis_n).abs() < 0.95 {
        return None;
    }

    let (top_verts, bottom_verts, bottom_idx) = {
        let Some((_aabb, polys)) = brush.get_polygons_and_aabb() else {
            return None;
        };
        let (top_verts, _) = polys.get(face_idx)?;
        if top_verts.len() < 3 {
            return None;
        }
        let top_verts = top_verts.clone();
        let top_c = centroid(&top_verts)?;

        let top_t = top_c.dot(axis_n);
        let mut best: Option<(usize, f32, Vec<Vec3>)> = None;
        for (i, (verts, _)) in polys.iter().enumerate() {
            if i == face_idx || verts.len() < 3 {
                continue;
            }
            let c = match centroid(verts) {
                Some(v) => v,
                None => continue,
            };
            let sep = (c.dot(axis_n) - top_t).abs();
            if sep < 1.0e-4 {
                continue;
            }
            let score = sep;
            match &best {
                None => best = Some((i, score, verts.clone())),
                Some((_, best_score, _)) if score > *best_score => {
                    best = Some((i, score, verts.clone()))
                }
                _ => {}
            }
        }
        let (bottom_idx, _score, bottom_verts) = best?;
        (top_verts, bottom_verts, bottom_idx)
    };

    if top_verts.len() != bottom_verts.len() || top_verts.len() < 3 {
        return None;
    }
    let n = top_verts.len();

    let top_c = centroid(&top_verts)?;
    let bottom_c = centroid(&bottom_verts)?;
    let pivot = (top_c + bottom_c) * 0.5;

    let (u, v) = uv_basis_from_axis(axis_n);
    if u == Vec3::ZERO || v == Vec3::ZERO {
        return None;
    }
    let angle_uv = |p: Vec3| -> f32 {
        let d = p - pivot;
        d.dot(v).atan2(d.dot(u))
    };

    let mut top_sorted: Vec<Vec3> = top_verts.clone();
    top_sorted.sort_by(|a, b| {
        angle_uv(*a)
            .partial_cmp(&angle_uv(*b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut bottom_sorted: Vec<Vec3> = bottom_verts.clone();
    bottom_sorted.sort_by(|a, b| {
        angle_uv(*a)
            .partial_cmp(&angle_uv(*b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let q = Quat::from_axis_angle(axis_n, angle_rad);

    // Rotate top vertices around pivot.
    let mut top_rot: Vec<Vec3> = top_sorted
        .iter()
        .map(|p| pivot + q * (*p - pivot))
        .collect();

    // Keep the rotated top face within the original 2D extents in (u,v) by scaling the
    // perpendicular components (this matches the “keep 128x128 size” expectation in 2D view).
    let extents_uv = |verts: &[Vec3]| -> Option<(f32, f32, f32, f32)> {
        let mut min_u = f32::INFINITY;
        let mut max_u = f32::NEG_INFINITY;
        let mut min_v = f32::INFINITY;
        let mut max_v = f32::NEG_INFINITY;
        for p in verts {
            let d = *p - pivot;
            let cu = d.dot(u);
            let cv = d.dot(v);
            if !cu.is_finite() || !cv.is_finite() {
                return None;
            }
            min_u = min_u.min(cu);
            max_u = max_u.max(cu);
            min_v = min_v.min(cv);
            max_v = max_v.max(cv);
        }
        Some((min_u, max_u, min_v, max_v))
    };
    let (old_min_u, old_max_u, old_min_v, old_max_v) = extents_uv(&top_sorted)?;
    let (new_min_u, new_max_u, new_min_v, new_max_v) = extents_uv(&top_rot)?;
    let old_du = (old_max_u - old_min_u).abs().max(1.0e-6);
    let old_dv = (old_max_v - old_min_v).abs().max(1.0e-6);
    let new_du = (new_max_u - new_min_u).abs().max(1.0e-6);
    let new_dv = (new_max_v - new_min_v).abs().max(1.0e-6);
    let su = old_du / new_du;
    let sv = old_dv / new_dv;
    top_rot = top_rot
        .into_iter()
        .map(|p| {
            let d = p - pivot;
            let du = d.dot(u);
            let dv = d.dot(v);
            let da = d.dot(axis_n);
            pivot + u * (du * su) + v * (dv * sv) + axis_n * da
        })
        .collect();

    // Align bottom so top[0] sits between bottom[0] and bottom[1] in angle-space.
    let top0 = angle_uv(top_rot[0]);
    let mut shift = 0usize;
    for i in 0..n {
        let a0 = angle_uv(bottom_sorted[i]);
        let a1 = angle_uv(bottom_sorted[(i + 1) % n]);
        let in_range = if a0 <= a1 {
            top0 >= a0 && top0 < a1
        } else {
            // wrapped interval
            top0 >= a0 || top0 < a1
        };
        if in_range {
            shift = i;
            break;
        }
    }
    let mut bottom_aligned = Vec::with_capacity(n);
    for k in 0..n {
        bottom_aligned.push(bottom_sorted[(shift + k) % n]);
    }

    // Build new face list: top + bottom + 2n triangle sides.
    let mut all_points = Vec::with_capacity(n * 2);
    all_points.extend_from_slice(&top_rot);
    all_points.extend_from_slice(&bottom_aligned);
    let center = centroid(&all_points)?;

    let bottom_face = orig_faces.get(bottom_idx)?.clone();
    let mut side_candidates: Vec<(Vec3, kradiant::map::Face)> = Vec::new();
    for (i, f) in orig_faces.iter().enumerate() {
        if i == face_idx || i == bottom_idx {
            continue;
        }
        if let Some(n) = face_plane_normal(f.plane_points) {
            side_candidates.push((n, f.clone()));
        }
    }

    let mut new_faces: Vec<kradiant::map::Face> = Vec::with_capacity(2 + 2 * n);

    // Top face (keep selected face material).
    let top_plane = outward_plane_points([top_rot[0], top_rot[1], top_rot[2]], center);
    new_faces.push(kradiant::map::Face {
        plane_points: top_plane,
        texture: selected_face.texture.clone(),
        params: selected_face.params,
    });

    // Bottom face.
    let bottom_plane = outward_plane_points(
        [bottom_aligned[0], bottom_aligned[1], bottom_aligned[2]],
        center,
    );
    new_faces.push(kradiant::map::Face {
        plane_points: bottom_plane,
        texture: bottom_face.texture.clone(),
        params: bottom_face.params,
    });

    // Side triangles.
    let pick_side_mat = |tri: [Vec3; 3]| -> (String, kradiant::map::TextureParams) {
        let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
        let len2 = n.length_squared();
        if len2 <= 1.0e-10 || !len2.is_finite() || side_candidates.is_empty() {
            return (selected_face.texture.clone(), selected_face.params);
        }
        let n = n / len2.sqrt();
        let mut best = 0.0f32;
        let mut best_face: Option<&kradiant::map::Face> = None;
        for (cn, f) in &side_candidates {
            let d = cn.dot(n).abs();
            if d > best {
                best = d;
                best_face = Some(f);
            }
        }
        if let Some(f) = best_face {
            (f.texture.clone(), f.params)
        } else {
            (selected_face.texture.clone(), selected_face.params)
        }
    };

    for i in 0..n {
        let a = top_rot[i];
        let b = bottom_aligned[i];
        let c = bottom_aligned[(i + 1) % n];
        let tri = outward_plane_points([a, b, c], center);
        let (tex, params) = pick_side_mat(tri);
        new_faces.push(kradiant::map::Face {
            plane_points: tri,
            texture: tex,
            params,
        });

        let a = bottom_aligned[i];
        let b = top_rot[i];
        let c = top_rot[(i + n - 1) % n];
        let tri = outward_plane_points([a, b, c], center);
        let (tex, params) = pick_side_mat(tri);
        new_faces.push(kradiant::map::Face {
            plane_points: tri,
            texture: tex,
            params,
        });
    }

    let mut tmp =
        kradiant::map::Brush::new(brush.id, kradiant::map::BrushContent::Convex(new_faces));
    if tmp.get_polygons_and_aabb().is_some() {
        *brush = tmp;
        Some(0) // top face is always index 0 in the rebuilt brush
    } else {
        None
    }
}

fn translate_selected_faces(
    map: &mut kradiant::map::Map,
    selected_faces: &[FaceSelection],
    delta: Vec3,
) -> bool {
    if selected_faces.is_empty() || delta == Vec3::ZERO {
        return false;
    }

    use std::collections::BTreeMap;
    let mut by_brush: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for sel in selected_faces {
        by_brush
            .entry((sel.entity_idx, sel.brush_idx))
            .or_default()
            .push(sel.face_idx);
    }

    let mut any = false;

    for ((entity_idx, brush_idx), mut face_indices) in by_brush {
        face_indices.sort_unstable();
        face_indices.dedup();

        let Some(entity) = map.entities.get_mut(entity_idx) else {
            continue;
        };
        let Some(brush) = entity.brushes.get_mut(brush_idx) else {
            continue;
        };

        let mut changed = false;
        for face_idx in face_indices {
            if apply_face_weighted_point_transform_to_convex_brush(brush, face_idx, |p| p + delta) {
                changed = true;
            }
        }

        if changed {
            map.generation = map.generation.wrapping_add(1);
            any = true;
        }
    }

    any
}

fn apply_affine_scale_to_selected_faces(
    map: &mut kradiant::map::Map,
    selected_faces: &[FaceSelection],
    xform: editing::AffineScale,
) -> bool {
    if selected_faces.is_empty() || xform.scale == Vec3::ONE {
        return false;
    }

    use std::collections::BTreeMap;
    let mut by_brush: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for sel in selected_faces {
        by_brush
            .entry((sel.entity_idx, sel.brush_idx))
            .or_default()
            .push(sel.face_idx);
    }

    let mut any = false;
    for ((entity_idx, brush_idx), mut face_indices) in by_brush {
        face_indices.sort_unstable();
        face_indices.dedup();

        let Some(entity) = map.entities.get_mut(entity_idx) else {
            continue;
        };
        let Some(brush) = entity.brushes.get_mut(brush_idx) else {
            continue;
        };

        let mut changed = false;
        for face_idx in face_indices {
            if apply_face_weighted_point_transform_to_convex_brush(brush, face_idx, |p| {
                xform.apply_point(p)
            }) {
                changed = true;
            }
        }

        if changed {
            map.generation = map.generation.wrapping_add(1);
            any = true;
        }
    }

    any
}

fn apply_affine_rotate_to_selected_faces(
    map: &mut kradiant::map::Map,
    selected_faces: &mut [FaceSelection],
    axis: Vec3,
    angle_rad: f32,
) -> bool {
    if selected_faces.is_empty() {
        return false;
    }

    use std::collections::BTreeMap;
    let mut by_brush: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for sel in &*selected_faces {
        by_brush
            .entry((sel.entity_idx, sel.brush_idx))
            .or_default()
            .push(sel.face_idx);
    }

    let mut any = false;
    for ((entity_idx, brush_idx), mut face_indices) in by_brush {
        face_indices.sort_unstable();
        face_indices.dedup();

        let Some(entity) = map.entities.get_mut(entity_idx) else {
            continue;
        };
        let Some(brush) = entity.brushes.get_mut(brush_idx) else {
            continue;
        };

        let mut changed = false;
        for face_idx in face_indices {
            if let Some(_new_top_idx) =
                twist_rotate_face_antiprism(brush, face_idx, axis, angle_rad)
            {
                changed = true;
            } else if apply_face_weighted_twist_rotate_to_convex_brush(
                brush,
                face_idx,
                (brush.aabb.min + brush.aabb.max) * 0.5,
                axis,
                angle_rad,
            ) {
                changed = true;
            }
        }

        if changed {
            map.generation = map.generation.wrapping_add(1);
            any = true;

            // Face indices may have changed after rebuild; re-target selection to the new "top"
            // face (we put it at index 0).
            for sel in selected_faces.iter_mut() {
                if sel.entity_idx == entity_idx && sel.brush_idx == brush_idx {
                    sel.face_idx = 0;
                }
            }
        }
    }

    any
}

pub fn selection_aabb(
    map: &Option<kradiant::map::Map>,
    selected_brushes: &[(usize, usize)],
) -> Option<Aabb> {
    if selected_brushes.is_empty() {
        return None;
    }
    let map = map.as_ref()?;
    selection_aabb_from_map(map, selected_brushes)
}

pub fn selection_aabb_from_map(
    map: &kradiant::map::Map,
    selected: &[(usize, usize)],
) -> Option<Aabb> {
    if selected.is_empty() {
        return None;
    }

    let mut out = Aabb {
        min: Vec3::new(f32::MAX, f32::MAX, f32::MAX),
        max: Vec3::new(f32::MIN, f32::MIN, f32::MIN),
    };

    let mut any = false;
    for &(entity_idx, brush_idx) in selected {
        let Some(entity) = map.entities.get(entity_idx) else {
            continue;
        };
        let Some(brush) = entity.brushes.get(brush_idx) else {
            continue;
        };
        out.min = out.min.min(brush.aabb.min);
        out.max = out.max.max(brush.aabb.max);
        any = true;
    }
    any.then_some(out)
}

pub fn stretch_faces_from_start(
    axis: Ortho,
    aabb: &Aabb,
    start: Vec2,
) -> Vec<editing::StretchFace> {
    let (min_u, max_u, min_v, max_v) = match axis {
        Ortho::XY => (aabb.min.x, aabb.max.x, aabb.min.y, aabb.max.y),
        Ortho::XZ => (aabb.min.x, aabb.max.x, -aabb.max.z, -aabb.min.z),
        Ortho::YZ => (aabb.min.y, aabb.max.y, -aabb.max.z, -aabb.min.z),
    };

    let mut out = Vec::with_capacity(2);

    if start.x < min_u {
        out.push(match axis {
            Ortho::XY | Ortho::XZ => editing::StretchFace::XMin,
            Ortho::YZ => editing::StretchFace::YMin,
        });
    } else if start.x > max_u {
        out.push(match axis {
            Ortho::XY | Ortho::XZ => editing::StretchFace::XMax,
            Ortho::YZ => editing::StretchFace::YMax,
        });
    }

    if start.y < min_v {
        out.push(match axis {
            Ortho::XY => editing::StretchFace::YMin,
            Ortho::XZ | Ortho::YZ => editing::StretchFace::ZMax,
        });
    } else if start.y > max_v {
        out.push(match axis {
            Ortho::XY => editing::StretchFace::YMax,
            Ortho::XZ | Ortho::YZ => editing::StretchFace::ZMin,
        });
    }

    out
}

pub fn update_last_work_from_aabb(view: &mut View2D, aabb: &Aabb) {
    view.work_pos = (aabb.min + aabb.max) / 2.0;
    let d = aabb.max - aabb.min;
    let fallback = 1.0;
    view.work_depth = Vec3::new(
        crate::util::normalize_depth(d.x, fallback),
        crate::util::normalize_depth(d.y, fallback),
        crate::util::normalize_depth(d.z, fallback),
    );
}

fn create_brush_from_drag(
    view: &View2D,
    start: Vec2,
    end: Vec2,
    config: &mut EditorConfig,
    map: &mut Option<kradiant::map::Map>,
    console: &mut ConsoleLogger,
) -> Option<BrushId> {
    let min2 = start.min(end);
    let max2 = start.max(end);
    if min2.x == max2.x || min2.y == max2.y {
        return None;
    }

    let fallback = (config.grid_minor_step as f32).max(1.0);
    let last = view.last_aabb.as_ref();
    let (min3, max3) = match view.ortho_axis {
        Ortho::XY => {
            let (z0, z1) = last.map(|a| (a.min.z, a.max.z)).unwrap_or((0.0, fallback));
            (Vec3::new(min2.x, min2.y, z0), Vec3::new(max2.x, max2.y, z1))
        }
        Ortho::XZ => {
            let (y0, y1) = last.map(|a| (a.min.y, a.max.y)).unwrap_or((0.0, fallback));
            (
                Vec3::new(min2.x, y0, -max2.y),
                Vec3::new(max2.x, y1, -min2.y),
            )
        }
        Ortho::YZ => {
            let (x0, x1) = last.map(|a| (a.min.x, a.max.x)).unwrap_or((0.0, fallback));
            (
                Vec3::new(x0, min2.x, -max2.y),
                Vec3::new(x1, max2.x, -min2.y),
            )
        }
    };

    let Some(map) = map.as_mut() else {
        log_error!(console, "can't get mutable ref to map");
        return None;
    };

    let aabb = Aabb::from_points(min3, max3);
    match editing::add_convex_brush_from_aabb(map, 0, aabb, "common/caulk") {
        Ok(id) => {
            log_info!(console, "Created brush {:?}", id);
            Some(id)
        }
        Err(e) => {
            log_error!(console, "Failed to create brush: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twist_rotate_antiprism_cube_top_face() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("kradiant")
            .join("test")
            .join("cube.map");
        let mut map =
            kradiant::loader::map_loader::load_map(path.to_str().unwrap()).expect("load cube");
        let brush = map.entities[0].brushes.get_mut(0).expect("brush 0");

        // In `cube.map`, face 4 is the Z+ face (top).
        let out = twist_rotate_face_antiprism(brush, 4, Vec3::Z, 45.0f32.to_radians());
        assert!(out.is_some(), "antiprism rotate should succeed");

        let face_count = match &brush.content {
            kradiant::map::BrushContent::Convex(faces) => faces.len(),
            kradiant::map::BrushContent::Patch(_) => 0,
        };
        assert!(
            face_count >= 10,
            "antiprism rotate should increase face count (got {face_count})"
        );

        let (_aabb, polys) = brush.get_polygons_and_aabb().expect("polys");
        let mut min_z = f32::INFINITY;
        let mut max_z = f32::NEG_INFINITY;
        for (verts, _) in polys {
            for v in verts {
                min_z = min_z.min(v.z);
                max_z = max_z.max(v.z);
            }
        }
        assert!((min_z + 64.0).abs() < 1.0e-2, "bottom should stay at z=-64");
        assert!((max_z - 64.0).abs() < 1.0e-2, "top should stay at z=+64");
    }
}
