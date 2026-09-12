//! Small convenience methods on [`crate::map::Map`] used by editor frontends.

use std::fmt::Display;

use num_traits::{Float, NumCast};

use crate::Vec3;
use crate::map::{BrushContent, Map};
use crate::shader::ShaderDb;

impl Map {
    /// Count total brushes across all entities.
    pub fn count_brushes(&self) -> usize {
        self.entities.iter().map(|e| e.brushes.len()).sum()
    }

    /// Return the number of entities contained in the map.
    pub fn get_entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Collect a de-duplicated list of materials/shaders used by the map.
    /// Returns original material names (not resolved qer_editorimage paths) so that
    /// viewport texture lookups work correctly. The qer_editorimage resolution happens
    /// during texture loading in request_texture_load().
    pub fn collect_used_materials(&self, _shader_db: Option<&ShaderDb>) -> Vec<String> {
        let mut out = Vec::<String>::new();
        for ent in &self.entities {
            for b in &ent.brushes {
                match &b.content {
                    BrushContent::Convex(faces) => {
                        for f in faces {
                            out.push(f.texture.clone());
                        }
                    }
                    BrushContent::Patch(p) => {
                        out.push(p.texture.clone());
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Increment the map generation counter to signal that something changed.
    pub fn mark_map_dirty(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

pub fn round_float<T: Float + NumCast>(value: T, points: u8) -> T {
    let multiplier = T::from(10).unwrap().powi(points as i32);
    (value * multiplier).round() / multiplier
}

pub fn format_float<T: Float + Display>(value: T, points: u8) -> String {
    let rounded = round_float(value, points);
    format!("{rounded}")
}

pub fn format_float_trim<T: Float + Display>(value: T, points: u8) -> String {
    let rounded = round_float(value, points);
    let formatted = format!("{rounded}");
    // Only trim trailing zeros after a decimal point. Whole numbers like
    // "270" must not lose their trailing digit ("27").
    if !formatted.contains('.') {
        return formatted;
    }
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');

    if trimmed.is_empty() { "0" } else { trimmed }.to_string()
}

pub fn format_vec2<T: From<[f32; 2]> + Into<[f32; 2]>>(value: T, points: u8) -> String {
    let v: [f32; 2] = value.into();
    let x_r = round_float(v[0], points);
    let y_r = round_float(v[1], points);
    format!("({x_r}, {y_r})")
}

pub fn format_vec3<T: From<[f32; 3]> + Into<[f32; 3]>>(value: T, points: u8) -> String {
    let v: [f32; 3] = value.into();
    let x_r = round_float(v[0], points);
    let y_r = round_float(v[1], points);
    let z_r = round_float(v[2], points);
    format!("({x_r}, {y_r}, {z_r})")
}

pub fn rotate_vector(v: Vec3, axis: Vec3, degree: f32) -> Vec3 {
    let radians = degree.to_radians();
    let sin = radians.sin();
    let cos = radians.cos();
    let one_minus_cos = 1.0 - cos;

    // Rodrigues' rotation formula
    let k = axis.normalize();
    let rotated = v * cos + k.cross(v) * sin + k * k.dot(v) * one_minus_cos;
    rotated
}

pub fn compute_normal(plane_points: [Vec3; 3]) -> Vec3 {
    let e1 = plane_points[1] - plane_points[0];
    let e2 = plane_points[2] - plane_points[0];

    e1.cross(e2).normalize_or_zero()
}
