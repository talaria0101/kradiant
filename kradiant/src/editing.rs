use crate::map::{
    Brush, BrushContent, BrushId, Entity, EntityId, Face, Map, SurfaceFlags, TextureParams,
};
use crate::{IVec3, Vec3};

#[derive(Debug, Clone)]
pub struct Aabb {
    pub min: IVec3,
    pub max: IVec3,
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
        min: IVec3::new(
            min.x.floor() as i32,
            min.y.floor() as i32,
            min.z.floor() as i32,
        ),
        max: IVec3::new(
            max.x.ceil() as i32,
            max.y.ceil() as i32,
            max.z.ceil() as i32,
        ),
    }
}

pub fn aabb_from_positions(positions: &[Vec3]) -> Aabb {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for &p in positions {
        min = min.min(p);
        max = max.max(p);
    }

    Aabb {
        min: IVec3::new(
            min.x.floor() as i32,
            min.y.floor() as i32,
            min.z.floor() as i32,
        ),
        max: IVec3::new(
            max.x.ceil() as i32,
            max.y.ceil() as i32,
            max.z.ceil() as i32,
        ),
    }
}

impl Default for Aabb {
    fn default() -> Self {
        Self {
            min: IVec3::default(),
            max: IVec3::default(),
        }
    }
}

impl Aabb {
    pub fn from_points(min: IVec3, max: IVec3) -> Self {
        Self {
            min: min.min(max),
            max: min.max(max),
        }
    }

    pub fn intersected_by_ray(&self, ray_origin: Vec3, ray_dir: Vec3) -> bool {
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

pub fn default_texture_params() -> TextureParams {
    TextureParams {
        shift: crate::IVec2::new(0, 0),
        rotate: 0,
        scale: crate::Vec2::new(0.25, 0.25),
        surface_flags: SurfaceFlags::Structural,
        idk: 0.0,
        value: 0,
        sample_size: 0,
    }
}

pub fn convex_brush_from_aabb(id: BrushId, aabb: Aabb, texture: impl Into<String>) -> Brush {
    let min = aabb.min.as_vec3();
    let max = aabb.max.as_vec3();
    let texture = texture.into();
    let params = default_texture_params();

    let faces = vec![
        Face {
            plane_points: [
                Vec3::new(max.x, min.y, min.z),
                Vec3::new(max.x, max.y, max.z),
                Vec3::new(max.x, min.y, max.z),
            ],
            texture: texture.clone(),
            params,
        },
        Face {
            plane_points: [
                Vec3::new(min.x, min.y, min.z),
                Vec3::new(min.x, min.y, max.z),
                Vec3::new(min.x, max.y, max.z),
            ],
            texture: texture.clone(),
            params,
        },
        Face {
            plane_points: [
                Vec3::new(min.x, max.y, min.z),
                Vec3::new(min.x, max.y, max.z),
                Vec3::new(max.x, max.y, max.z),
            ],
            texture: texture.clone(),
            params,
        },
        Face {
            plane_points: [
                Vec3::new(min.x, min.y, min.z),
                Vec3::new(max.x, min.y, max.z),
                Vec3::new(min.x, min.y, max.z),
            ],
            texture: texture.clone(),
            params,
        },
        Face {
            plane_points: [
                Vec3::new(min.x, min.y, max.z),
                Vec3::new(max.x, min.y, max.z),
                Vec3::new(max.x, max.y, max.z),
            ],
            texture: texture.clone(),
            params,
        },
        Face {
            plane_points: [
                Vec3::new(min.x, min.y, min.z),
                Vec3::new(max.x, max.y, min.z),
                Vec3::new(max.x, min.y, min.z),
            ],
            texture,
            params,
        },
    ];

    let mut brush = Brush::new(id, BrushContent::Convex(faces));
    brush.aabb = aabb;
    brush
}

pub fn add_convex_brush_from_aabb(
    map: &mut Map,
    entity_index: usize,
    aabb: Aabb,
    texture: impl Into<String>,
) -> Result<BrushId, String> {
    if aabb.min == aabb.max {
        return Err("AABB has zero size".to_string());
    }

    if map.entities.is_empty() {
        map.entities.push(Entity {
            id: EntityId(0),
            classname: "worldspawn".to_string(),
            properties: Default::default(),
            brushes: vec![],
        });
    }

    let idx = entity_index.min(map.entities.len() - 1);
    let entity = &mut map.entities[idx];

    let next_id = entity
        .brushes
        .iter()
        .map(|b| b.id.0)
        .max()
        .map(|id| id.wrapping_add(1))
        .unwrap_or(0);
    let brush_id = BrushId(next_id);

    entity
        .brushes
        .push(convex_brush_from_aabb(brush_id, aabb, texture));

    map.generation = map.generation.wrapping_add(1);
    Ok(brush_id)
}

pub fn pick_convex_brush_by_ray(
    map: &mut Map,
    ray_origin: Vec3,
    ray_dir: Vec3,
) -> Option<(usize, usize)> {
    pick_brush_by_ray(map, ray_origin, ray_dir, PickMask::CONVEX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickMask(u8);

impl PickMask {
    pub const CONVEX: PickMask = PickMask(1 << 0);
    pub const PATCH: PickMask = PickMask(1 << 1);
    pub const ALL: PickMask = PickMask(Self::CONVEX.0 | Self::PATCH.0);

    pub fn contains(self, other: PickMask) -> bool {
        (self.0 & other.0) != 0
    }
}

pub fn pick_brush_by_ray(
    map: &mut Map,
    ray_origin: Vec3,
    ray_dir: Vec3,
    mask: PickMask,
) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize, f32)> = None;

    for (entity_index, entity) in map.entities.iter_mut().enumerate() {
        for (brush_index, brush) in entity.brushes.iter_mut().enumerate() {
            match &mut brush.content {
                BrushContent::Convex(_) => {
                    if !mask.contains(PickMask::CONVEX) {
                        continue;
                    }

                    let Some((aabb, polys)) = brush.get_polygons_and_aabb() else {
                        continue;
                    };
                    let Some((t_enter, t_exit)) = ray_aabb_intersection(
                        aabb.min.as_vec3(),
                        aabb.max.as_vec3(),
                        ray_origin,
                        ray_dir,
                    ) else {
                        continue;
                    };
                    if t_exit < 0.0 {
                        continue;
                    }

                    if let Some((_, _, best_t)) = best {
                        if t_enter > best_t {
                            continue;
                        }
                    }

                    let Some(t) = ray_polys_first_hit(polys, ray_origin, ray_dir) else {
                        continue;
                    };
                    if t >= 0.0 {
                        match best {
                            None => best = Some((entity_index, brush_index, t)),
                            Some((_, _, best_t)) if t < best_t => {
                                best = Some((entity_index, brush_index, t))
                            }
                            _ => {}
                        }
                    }
                }
                BrushContent::Patch(patch) => {
                    if !mask.contains(PickMask::PATCH) {
                        continue;
                    }

                    let Some((mesh, patch_aabb, _edges)) = patch.get_mesh_aabb_wire() else {
                        continue;
                    };
                    brush.aabb = patch_aabb.clone();

                    let Some((t_enter, t_exit)) = ray_aabb_intersection(
                        patch_aabb.min.as_vec3(),
                        patch_aabb.max.as_vec3(),
                        ray_origin,
                        ray_dir,
                    ) else {
                        continue;
                    };
                    if t_exit < 0.0 {
                        continue;
                    }
                    if let Some((_, _, best_t)) = best {
                        if t_enter > best_t {
                            continue;
                        }
                    }

                    let Some(t) = ray_patch_mesh_first_hit(mesh, ray_origin, ray_dir) else {
                        continue;
                    };
                    if t >= 0.0 {
                        match best {
                            None => best = Some((entity_index, brush_index, t)),
                            Some((_, _, best_t)) if t < best_t => {
                                best = Some((entity_index, brush_index, t))
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    best.map(|(e, b, _)| (e, b))
}

fn ray_aabb_intersection(min: Vec3, max: Vec3, origin: Vec3, dir: Vec3) -> Option<(f32, f32)> {
    let eps = 1.0e-8;
    let mut tmin = f32::NEG_INFINITY;
    let mut tmax = f32::INFINITY;

    for i in 0..3 {
        let o = origin[i];
        let d = dir[i];
        let lo = min[i];
        let hi = max[i];

        if d.abs() < eps {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }

        let inv = 1.0 / d;
        let mut t1 = (lo - o) * inv;
        let mut t2 = (hi - o) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmax < tmin {
            return None;
        }
    }

    Some((tmin, tmax))
}

fn ray_polys_first_hit(polys: &[(Vec<Vec3>, Vec<u32>)], origin: Vec3, dir: Vec3) -> Option<f32> {
    let mut best = None;
    for (positions, indices) in polys {
        if positions.len() < 3 || indices.len() < 3 {
            continue;
        }
        for tri in indices.chunks_exact(3) {
            let i0 = tri[0] as usize;
            let i1 = tri[1] as usize;
            let i2 = tri[2] as usize;
            if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
                continue;
            }
            let v0 = positions[i0];
            let v1 = positions[i1];
            let v2 = positions[i2];
            let Some(t) = ray_triangle_intersection(origin, dir, v0, v1, v2) else {
                continue;
            };
            if t >= 0.0 {
                match best {
                    None => best = Some(t),
                    Some(best_t) if t < best_t => best = Some(t),
                    _ => {}
                }
            }
        }
    }
    best
}

fn ray_patch_mesh_first_hit(
    mesh: &crate::geometry::PatchMesh,
    origin: Vec3,
    dir: Vec3,
) -> Option<f32> {
    let positions = mesh.positions.as_slice();
    let indices = mesh.indices.as_slice();
    if positions.len() < 3 || indices.len() < 3 {
        return None;
    }

    let mut best = None;
    for tri in indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }
        let v0 = positions[i0];
        let v1 = positions[i1];
        let v2 = positions[i2];
        let Some(t) = ray_triangle_intersection(origin, dir, v0, v1, v2) else {
            continue;
        };
        if t >= 0.0 {
            match best {
                None => best = Some(t),
                Some(best_t) if t < best_t => best = Some(t),
                _ => {}
            }
        }
    }
    best
}

fn ray_triangle_intersection(origin: Vec3, dir: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
    let eps = 1.0e-7;
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < eps {
        return None;
    }
    let inv_det = 1.0 / det;
    let tvec = origin - v0;
    let u = tvec.dot(p) * inv_det;
    if u < 0.0 || u > 1.0 {
        return None;
    }
    let q = tvec.cross(e1);
    let v = dir.dot(q) * inv_det;
    if v < 0.0 || (u + v) > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv_det;
    Some(t)
}

pub struct BrushEditor;

impl BrushEditor {
    pub fn stretch_x(brush: &mut Brush, length: i32) // brush sizes are always integers
    {
        //brush.update_brush_plane(, , );
    }
}
