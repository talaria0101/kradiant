//! 2D View

use crate::config::EditorConfig;
use crate::ui::console::ConsoleLogger;
use crate::util::{project_to_2d, text_height, text_width};
use crate::{log_error, log_info, log_warn, util};
use dear_imgui_rs::{Condition, StyleColor, TextureId, Ui, WindowFlags};
use glam::{Vec2, Vec3};
use kradiant::editing::{self, Aabb};
use kradiant::map::BrushId;
use kradiant::map_utils::format_float;

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
    StretchSelection,
    RotateSelection,
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
        }
    }
}

impl View2D {
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
        let axis = match self.ortho_axis {
            Ortho::XY => Vec3::Z,
            Ortho::XZ => Vec3::Y,
            Ortho::YZ => Vec3::X,
        };
        editing::rotate_selection_transform(&rotate.selection_aabb, axis, self.rotate_angle)
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
        selected_brushes: &mut Vec<(usize, usize)>,
        selected_entity: &mut Option<usize>,
        map: &mut Option<kradiant::map::Map>,
        selection_rgba: [f32; 4],
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

                if canvas_interacting {
                    let wheel = ui.io().mouse_wheel();
                    if wheel != 0.0 {
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
                    if ui.is_mouse_dragging(MouseButton::Right) {
                        let [dx, dy] = ui.mouse_drag_delta(MouseButton::Right);
                        self.pan[0] += dx;
                        self.pan[1] += dy;
                        ui.reset_mouse_drag_delta(MouseButton::Right);
                    }
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
                if canvas_interacting
                    && (ui.is_mouse_clicked(MouseButton::Left)
                        || ui.is_mouse_dragging(MouseButton::Left))
                    && ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                {
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

                    let selected_brush = map.as_mut().and_then(|m| {
                        editing::pick_brush_by_ray(m, ray_origin, ray_dir, editing::PickMask::ALL)
                    });
                    if let Some(sel) = selected_brush {
                        if !selected_brushes.contains(&sel) {
                            selected_brushes.push(sel);
                        }
                        *selected_entity = Some(sel.0);
                        if let Some(aabb) = selection_aabb(map, selected_brushes) {
                            self.last_aabb = Some(aabb.clone());
                            update_last_work_from_aabb(self, &aabb);
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
                    self.drag_mode = if !selected_brushes.is_empty() {
                        if util::click_in_selection_aabb(selected_brushes, map.as_ref().unwrap(), snapped_i, self.ortho_axis) {
                            if rotate_mode {
                                if let Some(aabb) = selection_aabb(map, selected_brushes) {
                                    let center = (aabb.min + aabb.max) * 0.5;
                                    let pivot_uv = project_to_2d(center, self.ortho_axis);
                                    self.rotate = Some(RotateDrag {
                                        selection_aabb: aabb,
                                        pivot_uv,
                                        start_uv: world,
                                    });
                                    DragMode::RotateSelection
                                } else {
                                    DragMode::MoveSelection
                                }
                            } else {
                                DragMode::MoveSelection
                            }
                        } else if let Some(aabb) = selection_aabb(map, selected_brushes) {
                            let faces = stretch_faces_from_start(self.ortho_axis, &aabb, snapped_i);
                            self.stretch = Some(StretchDrag {
                                selection_aabb: aabb,
                                faces: [faces.get(0).copied(), faces.get(1).copied()],
                            });
                            self.stretch_delta = Vec3::ZERO;
                            DragMode::StretchSelection
                        } else {
                            DragMode::NewBrush
                        }
                    } else {
                        DragMode::NewBrush
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

                        if self.drag_mode == DragMode::MoveSelection {
                            let d = snapped_i - self.drag_start.unwrap();
                            self.move_offset =
                                util::drag_delta_to_3d(d, self.ortho_axis, axis_lock);
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
                                let angle = if self.rotation_locked(axis_lock) {
                                    0.0
                                } else {
                                    let v0 = [rot.start_uv[0] - rot.pivot_uv[0], rot.start_uv[1] - rot.pivot_uv[1]];
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
                            DragMode::MoveSelection => {
                                let d = end - start;
                                if d != Vec2::ZERO {
                                    let delta = util::drag_delta_to_3d(d, self.ortho_axis, axis_lock);
                                    if let Some(map) = map.as_mut() {
                                        let generation = &mut map.generation;
                                        for (entity_idx, brush_idx) in selected_brushes.iter() {
                                            if let Some(entity) = map.entities.get_mut(*entity_idx) {
                                                if let Some(brush) = entity.brushes.get_mut(*brush_idx) {
                                                    brush.translate(generation, delta);
                                                }
                                            }
                                        }
                                    }
                                    if let Some(aabb) = selection_aabb(map, selected_brushes) {
                                        self.last_aabb = Some(aabb.clone());
                                        update_last_work_from_aabb(self, &aabb);
                                    }
                                }
                                self.move_offset = Vec3::ZERO;
                                log_info!(console, "Dragged selection");
                            }
                            DragMode::NewBrush => {
                                if selected_brushes.is_empty() {
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
                                        selected_brushes.push((0, created.0 as usize));
                                        if let Some(aabb) = selection_aabb(map, selected_brushes) {
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
                                        let mut new_sel_aabb: Option<Aabb> = None;
                                        if let Some(map) = map.as_mut() {
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
                                                            let Some(brush) = entity.brushes.get_mut(*brush_idx)
                                                                else {
                                                                continue;
                                                            };
                                                            if editing::apply_affine_scale_to_brush(
                                                                brush,
                                                                &mut map.generation,
                                                                xform,
                                                            ) { any = true; }
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
                                                                ) { any = true; }
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
                                                                    ) { any = true; }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            if any {
                                                new_sel_aabb = selection_aabb_from_map(
                                                    map,
                                                    &selected_brushes,
                                                );
                                                log_info!(console, "Stretched selection");
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
                                        let axis = match self.ortho_axis {
                                            Ortho::XY => Vec3::Z,
                                            Ortho::XZ => Vec3::Y,
                                            Ortho::YZ => Vec3::X,
                                        };
                                        if let Some((xform, _preview)) =
                                            editing::rotate_selection_transform(
                                                &rot.selection_aabb,
                                                axis,
                                                angle,
                                            )
                                        {
                                            let mut new_sel_aabb: Option<Aabb> = None;
                                            if let Some(map) = map.as_mut() {
                                                let mut any = false;
                                                for (entity_idx, brush_idx) in selected_brushes.iter() {
                                                    let Some(entity) = map.entities.get_mut(*entity_idx)
                                                        else {
                                                        continue;
                                                    };
                                                    let Some(brush) = entity.brushes.get_mut(*brush_idx)
                                                        else {
                                                        continue;
                                                    };
                                                    if editing::apply_affine_rotate_to_brush(
                                                        brush,
                                                        &mut map.generation,
                                                        xform,
                                                    ) { any = true; }
                                                }

                                                if any {
                                                    new_sel_aabb = selection_aabb_from_map(map, &selected_brushes);
                                                    log_info!(console, "Rotated selection");
                                                }
                                            }
                                            if let Some(aabb) = new_sel_aabb {
                                                self.last_aabb = Some(aabb.clone());
                                                update_last_work_from_aabb(self, &aabb);
                                            }
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
                }

                if ui.is_key_pressed(dear_imgui_rs::Key::Escape) {
                    if let Some(aabb) = selection_aabb(map, selected_brushes) {
                        self.last_aabb = Some(aabb.clone());
                        update_last_work_from_aabb(self, &aabb);
                    }
                    selected_brushes.clear();
                    self.stretch = None;
                    self.stretch_delta = Vec3::ZERO;
                    self.rotate = None;
                    self.rotate_angle = 0.0;
                    self.move_offset = Vec3::ZERO;
                }

                if ui.is_key_pressed(dear_imgui_rs::Key::Backspace) {
                    if let Some(aabb) = selection_aabb(map, selected_brushes) {
                        self.last_aabb = Some(aabb.clone());
                        update_last_work_from_aabb(self, &aabb);
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
                    *selected_entity = None;
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
                            DragMode::MoveSelection => {
                                let (a, b) = if let Some(sel) = selection_aabb(map, selected_brushes) {
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
                        }
                    }

                    if let Some(mut selection_aabb) = selection_aabb(map, selected_brushes) {
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
                                let axis = match self.ortho_axis {
                                    Ortho::XY => Vec3::Z,
                                    Ortho::XZ => Vec3::Y,
                                    Ortho::YZ => Vec3::X,
                                };
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
