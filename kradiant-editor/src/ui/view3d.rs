//! 3D View

use dear_imgui_rs::{Condition, TextureId, Ui, WindowFlags};
use glam::{Mat4, Vec3};
use kradiant::editor::viewport::DragMode;
use kradiant::editor::viewport::state::View3DState;
use kradiant::editor::{EditorConfig, EditorPalette, FaceSelection, PatchVertexSelection};
use std::ops::{Deref, DerefMut};

use crate::ui::editing::PickMask;
use crate::ui::view2d;
use crate::util::imgui_color_to_u32;
use kradiant::editing;
use std::collections::HashSet;

pub struct View3D {
    pub core: View3DState,
    pub tex_id: Option<TextureId>,
    pub wants_cursor_grab: bool,
    pub accumulated_mouse_delta: [f32; 2],
    /// Brushes touched during current selection drag (to avoid toggling multiple times)
    pub selection_drag_touched: Option<HashSet<(usize, usize)>>,
    /// Faces touched during current selection drag
    pub selection_drag_touched_faces: Option<HashSet<FaceSelection>>,
    /// Patch vertices touched during current selection drag
    pub selection_drag_touched_verts: Option<HashSet<usize>>,
}

impl Default for View3D {
    fn default() -> Self {
        Self {
            core: View3DState::default(),
            tex_id: None,
            wants_cursor_grab: false,
            accumulated_mouse_delta: [0.0, 0.0],
            selection_drag_touched: None,
            selection_drag_touched_faces: None,
            selection_drag_touched_verts: None,
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

    pub fn draw_impl(
        &mut self,
        ui: &Ui,
        config: &mut EditorConfig,
        palette: &EditorPalette,
        selected_brushes: &mut Vec<(usize, usize)>,
        selected_faces: &mut Vec<FaceSelection>,
        selected_patch_vertices: &mut Vec<PatchVertexSelection>,
        edit_faces: bool,
        edit_vertices: bool,
        map: &mut Option<kradiant::map::Map>,
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

                if canvas_interacting {
                    if !ui.is_mouse_down(MouseButton::Right) {
                        self.wants_cursor_grab = false;
                        self.accumulated_mouse_delta = [0.0, 0.0];
                    }

                    let wheel = ui.io().mouse_wheel();
                    if wheel != 0.0 {
                        let f = if wheel > 0.0 { 1.15f32 } else { 1.0 / 1.15 };
                        self.cam.zoom = (self.cam.zoom * f).clamp(1.0, 65_536.0);
                    }

                    const TURN_SENS: f32 = 0.010;
                    const PITCH_SENS: f32 = 0.010;
                    const MOVE_SENS: f32 = 0.030;
                    if ui.is_mouse_down(MouseButton::Right) {
                        self.wants_cursor_grab = true;

                        let dx = self.accumulated_mouse_delta[0];
                        let dy = self.accumulated_mouse_delta[1];
                        self.accumulated_mouse_delta = [0.0, 0.0];

                        self.cam.angles.x -= dx * TURN_SENS;

                        let ctrl = ui.is_key_down(dear_imgui_rs::Key::LeftCtrl)
                            || ui.is_key_down(dear_imgui_rs::Key::RightCtrl);
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
                        self.selection_drag_touched_verts = Some(HashSet::new());
                    }
                    if shift_selecting {
                        let ray_far = 1.0e6;

                        let mouse = ui.io().mouse_pos();
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
                            let selected_brush = map.as_mut().and_then(|m| {
                                editing::pick_brush_by_ray(
                                    m,
                                    ray_origin,
                                    ray_dir,
                                    mask,
                                )
                            });
                            if let Some(sel) = selected_brush {
                                // Only toggle if not already touched this drag
                                let touched =
                                    self.selection_drag_touched.get_or_insert_with(HashSet::new);
                                if touched.insert(sel) {
                                    if !selected_brushes.contains(&sel) {
                                        selected_brushes.push(sel);
                                    } else if let Some(i) =
                                        selected_brushes.iter().position(|b| b == &sel)
                                    {
                                        selected_brushes.remove(i);
                                    }
                                }
                                selected_faces.clear();
                            }
                        }
                    }

                    if ui.is_mouse_clicked(MouseButton::Left)
                        && !ui.is_key_down(dear_imgui_rs::Key::LeftShift)
                    {
                        if ui.is_key_down(dear_imgui_rs::Key::LeftAlt) {
                            self.drag_mode = DragMode::RectangularSelection;
                        }
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
