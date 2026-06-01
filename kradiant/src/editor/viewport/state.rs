use super::types::{DragMode, Ortho};
use crate::editing::{self, Aabb, AffineRotate, AffineScale};
use glam::Vec3;

#[derive(Clone, Debug)]
pub struct StretchDrag {
    pub selection_aabb: Aabb,
    pub faces: [Option<editing::StretchFace>; 2],
}

#[derive(Clone, Debug)]
pub struct RotateDrag {
    pub selection_aabb: Aabb,
    pub pivot_uv: [f32; 2],
    pub start_uv: [f32; 2],
    pub axis: Vec3,
}

#[derive(Debug, Clone)]
pub struct View2DState {
    pub ortho_axis: Ortho,
    pub rect: [f32; 4],
    pub zoom: f32,
    pub pan: [f32; 2],
    pub drag_mode: DragMode,
    pub move_offset: Vec3,
    pub stretch: Option<StretchDrag>,
    pub stretch_delta: Vec3,
    pub rotate: Option<RotateDrag>,
    pub rotate_angle: f32,
    pub work_pos: Vec3,
    pub work_depth: Vec3,
}

impl Default for View2DState {
    fn default() -> Self {
        Self {
            ortho_axis: Ortho::default(),
            rect: [0.0; 4],
            zoom: 1.0,
            pan: [0.0, 0.0],
            drag_mode: DragMode::default(),
            move_offset: Vec3::ZERO,
            stretch: None,
            stretch_delta: Vec3::ZERO,
            rotate: None,
            rotate_angle: 0.0,
            work_pos: Vec3::ZERO,
            work_depth: Vec3::ZERO,
        }
    }
}

impl View2DState {
    pub fn stretch_preview_xform(&self) -> Option<AffineScale> {
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

    pub fn rotate_preview_xform(&self) -> Option<AffineRotate> {
        let rotate = self.rotate.as_ref()?;
        editing::rotate_selection_transform(&rotate.selection_aabb, rotate.axis, self.rotate_angle)
            .map(|(xform, _)| xform)
    }
}

#[derive(Default, Clone, Debug, Copy)]
pub struct Camera {
    pub pos: Vec3,
    /// Orbit camera (Z-up): angles.x = yaw, angles.y = pitch (radians)
    pub angles: Vec3,
    /// Interpreted as movement speed/sensitivity (not distance)
    pub zoom: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct View3DState {
    pub rect: [f32; 4],
    pub cam: Camera,
    pub drag_mode: DragMode,
}

impl Default for View3DState {
    fn default() -> Self {
        Self {
            rect: [0.0; 4],
            cam: Camera {
                pos: Vec3::ZERO,
                angles: Vec3::new(0.8, -0.35, 0.0),
                zoom: 64.0,
            },
            drag_mode: DragMode::RectangularSelection,
        }
    }
}
