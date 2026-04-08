//! 3D View

use dear_imgui_rs::{Condition, TextureId, Ui, WindowFlags};
use glam::Vec3;

use crate::{theme::EditorPalette, util::imgui_color_to_u32};

#[derive(Default, Clone, Debug)]
pub struct Camera {
    pub pos: Vec3,
    /// Orbit camera (Z-up): angles.x = yaw, angles.y = pitch (radians)
    pub angles: Vec3,
    /// Interpreted as movement speed/sensitivity (not distance)
    pub zoom: f32,
}

pub struct View3D {
    pub rect: [f32; 4],
    pub tex_id: Option<TextureId>,
    pub cam: Camera,
    pub warp_request: Option<[f32; 2]>,
    pub(crate) warp_pending_reset: bool,
}

impl Default for View3D {
    fn default() -> Self {
        let cam = Camera {
            pos: Vec3::ZERO,
            angles: Vec3::new(0.8, -0.35, 0.0),
            zoom: 64.0,
        };

        Self {
            rect: [0.0; 4],
            tex_id: None,
            cam,
            warp_request: None,
            warp_pending_reset: false,
        }
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
        config: &mut crate::config::EditorConfig,
        palette: &EditorPalette,
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
                    .build(&mut config.view3d_fov);

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
                    if self.warp_pending_reset && ui.is_mouse_down(MouseButton::Right) {
                        ui.reset_mouse_drag_delta(MouseButton::Right);
                        self.warp_pending_reset = false;
                    }

                    let wheel = ui.io().mouse_wheel();
                    if wheel != 0.0 {
                        let f = if wheel > 0.0 { 1.15f32 } else { 1.0 / 1.15 };
                        self.cam.zoom = (self.cam.zoom * f).clamp(1.0, 65_536.0);
                    }

                    const TURN_SENS: f32 = 0.010;
                    const PITCH_SENS: f32 = 0.010;
                    const MOVE_SENS: f32 = 0.030;
                    if ui.is_mouse_dragging(MouseButton::Right) {
                        let [dx, dy] = ui.mouse_drag_delta(MouseButton::Right);
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

                        // Blender-like wrapping: when the cursor reaches an edge while dragging,
                        // request a warp to the opposite edge so deltas keep flowing.
                        let mouse = ui.io().mouse_pos();
                        let r = self.rect;
                        let margin = 2.0;
                        let left = r[0];
                        let top = r[1];
                        let right_edge = r[0] + r[2];
                        let bottom_edge = r[1] + r[3];

                        let mut warp: Option<[f32; 2]> = None;
                        if mouse[0] <= left + margin {
                            warp = Some([right_edge - margin - 1.0, mouse[1]]);
                        } else if mouse[0] >= right_edge - margin {
                            warp = Some([left + margin + 1.0, mouse[1]]);
                        } else if mouse[1] <= top + margin {
                            warp = Some([mouse[0], bottom_edge - margin - 1.0]);
                        } else if mouse[1] >= bottom_edge - margin {
                            warp = Some([mouse[0], top + margin + 1.0]);
                        }
                        if warp.is_some() {
                            self.warp_request = warp;
                        }

                        ui.reset_mouse_drag_delta(MouseButton::Right);
                    }

                    if ui.is_key_pressed(dear_imgui_rs::Key::D) {
                        self.cam.pos.z += self.cam.zoom / 2.0;
                    }
                    if ui.is_key_pressed(dear_imgui_rs::Key::C) {
                        self.cam.pos.z -= self.cam.zoom / 2.0;
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
