use crate::{Vec3, editor::viewport::Ortho};

pub fn project_to_2d(v: Vec3, axis: Ortho) -> [f32; 2] {
    match axis {
        // Flip Y so +Y is up on screen (ImGui Y+ is down).
        Ortho::XY => [v.x, -v.y],
        // Flip Z so +Z is up on screen (ImGui Y+ is down).
        Ortho::XZ => [v.x, -v.z],
        Ortho::YZ => [v.y, -v.z],
    }
}
