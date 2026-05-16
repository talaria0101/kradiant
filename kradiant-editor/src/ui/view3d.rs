//! 3D View

use dear_imgui_rs::{Condition, TextureId, Ui, WindowFlags};
use glam::Vec3;
use kradiant::editor::viewport::state::View3DState;
use kradiant::editor::{EditorConfig, EditorPalette};
use std::ops::{Deref, DerefMut};

use crate::util::imgui_color_to_u32;

pub struct View3D {
    pub core: View3DState,
    pub tex_id: Option<TextureId>,
    pub wants_cursor_grab: bool,
    pub accumulated_mouse_delta: [f32; 2],
}

impl Default for View3D {
    fn default() -> Self {
        Self {
            core: View3DState::default(),
            tex_id: None,
            wants_cursor_grab: false,
            accumulated_mouse_delta: [0.0, 0.0],
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

    pub fn draw_impl(&mut self, ui: &Ui, config: &mut EditorConfig, palette: &EditorPalette) {
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
