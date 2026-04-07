//! 3D View

use dear_imgui_rs::TextureId;
use glam::Vec3;

#[derive(Default, Clone, Debug)]
pub struct Camera {
    pub pos: Vec3,
    pub angles: Vec3,
    pub zoom: f32,
}

pub struct View3D {
    pub rect: [f32; 4],
    pub tex_id: Option<TextureId>,
    pub cam: Camera,
}

impl Default for View3D {
    fn default() -> Self {
        let cam = Camera {
            pos: Vec3::ZERO,
            angles: Vec3::ZERO,
            zoom: 1.0,
        };

        Self {
            rect: [0.0; 4],
            tex_id: None,
            cam,
        }
    }
}
