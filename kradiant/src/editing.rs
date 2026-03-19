use crate::{IVec3, Vec3};
use crate::map::Brush;

#[derive(Debug, Clone)]
pub struct Aabb {
    pub min: IVec3,
    pub max: IVec3
}

pub fn aabb_from_polys(polys: &[(Vec<Vec3>, Vec<u32>)]) -> Aabb {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for (verts, _) in polys {
        for v in verts {
            min = min.min(*v);
            max = max.max(*v);
        }
    }

    Aabb {
        min: IVec3::new(min.x.floor() as i32, min.y.floor() as i32, min.z.floor() as i32),
        max: IVec3::new(max.x.ceil() as i32,  max.y.ceil() as i32,  max.z.ceil() as i32),
    }
}

impl Default for Aabb {
    fn default() -> Self
    {
        Self { min: IVec3::default(), max: IVec3::default() }
    }
}

impl Aabb {
    pub fn from_points(min: IVec3, max: IVec3) -> Self
    {
        Self {
            min: min.min(max),
            max: min.max(max),
        }
    }

    pub fn intersected_by_ray(&self, ray_origin: Vec3, ray_dir: Vec3) -> bool
    {
        let inv_dir = 1.0 / ray_dir;
        let min = self.min.as_vec3();
        let max = self.max.as_vec3();

        let t1 = (min - ray_origin) * inv_dir;
        let t2 = (max - ray_origin) * inv_dir;

        let tmin = t1.min(t2);
        let tmax = t1.max(t2);

        let t_enter = tmin.max_element();
        let t_exit = tmax.min_element();

        t_exit >= t_enter && t_exit >= 0.0
    }
}

impl From<Aabb> for Brush {
    fn from(aabb: Aabb) -> Self
    {
        let min = aabb.min.as_vec3();
        let max = aabb.max.as_vec3();

        todo!()
    }
}

pub struct BrushEditor;

impl BrushEditor {
    pub fn stretch_x(brush: &mut Brush, length: i32) // brush sizes are always integers
    {
        //brush.update_brush_plane(, , );
    }
}
