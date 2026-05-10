use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EditorPalette {
    pub window_clear: [f32; 4],
    pub view2d_bg: [f32; 4],
    pub view2d_grid_major: [f32; 4],
    pub view2d_grid_minor: [f32; 4],
    pub view2d_axis_x: [f32; 4],
    pub view2d_axis_y: [f32; 4],
    pub view2d_geometry: [f32; 4],
    pub console_info: [f32; 4],
    pub console_warn: [f32; 4],
    pub console_error: [f32; 4],
    pub hud_text: [f32; 4],
    pub hud_text_dim: [f32; 4],
}

impl Default for EditorPalette {
    fn default() -> Self {
        Self {
            window_clear: [0.1, 0.1, 0.1, 1.0],
            view2d_bg: [0.15, 0.15, 0.15, 1.0],
            view2d_grid_major: [0.3, 0.3, 0.3, 1.0],
            view2d_grid_minor: [0.2, 0.2, 0.2, 1.0],
            view2d_axis_x: [0.5, 0.2, 0.2, 1.0],
            view2d_axis_y: [0.2, 0.5, 0.2, 1.0],
            view2d_geometry: [0.8, 0.8, 0.8, 1.0],
            console_info: [0.9, 0.9, 0.9, 1.0],
            console_warn: [0.9, 0.7, 0.1, 1.0],
            console_error: [0.9, 0.1, 0.1, 1.0],
            hud_text: [1.0, 1.0, 1.0, 1.0],
            hud_text_dim: [0.6, 0.6, 0.6, 1.0],
        }
    }
}
