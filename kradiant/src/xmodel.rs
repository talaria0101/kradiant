//! XMODEL geometry and material loading for CoD1 assets.
//!
//! This module reads `xmodel/`, `xmodelparts/`, and `xmodelsurfs/` data from the existing
//! [`crate::assets::AssetDb`] virtual filesystem, producing CPU-side geometry plus resolved
//! texture references suitable for editor preview.

use std::path::Path;

use rayon::iter::{
    IndexedParallelIterator, IntoParallelRefIterator, IntoParallelRefMutIterator, ParallelIterator,
};
use thiserror::Error;

use crate::assets::{
    AssetDb, AssetDbError, ResolvedAsset, normalize_asset_path, normalize_material_name,
};
use crate::shader::ShaderDb;
use crate::{Quat, Vec2, Vec3};

const COD1_XMODEL_VERSION: u16 = 0x0E;
const COD1_RIGGED: u16 = 65535;
const XMODEL_MAX_BONES: usize = 256;
const XMODEL_MAX_LODS: usize = 3;

#[derive(Debug, Clone)]
pub struct XModelVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
}

#[derive(Debug, Clone)]
pub struct XModelSurface {
    pub vertices: Vec<XModelVertex>,
    pub indices: Vec<u32>,
    pub material_name: String,
    pub texture_name: Option<String>,
    pub texture_asset: Option<ResolvedAsset>,
}

#[derive(Debug, Clone)]
pub struct XModel {
    pub name: String,
    pub surfaces: Vec<XModelSurface>,
    pub mins: Vec3,
    pub maxs: Vec3,
    pub origin: Vec3,
    pub radius: f32,
}

#[derive(Debug, Error)]
pub enum XModelError {
    #[error("asset error: {0}")]
    Asset(#[from] AssetDbError),

    #[error("xmodel parse error in {path}: {msg}")]
    Parse { path: String, msg: String },
}

#[derive(Debug, Clone)]
struct Bone {
    parent: i8,
    local_translation: Vec3,
    local_rotation: Quat,
    world_translation: Vec3,
    world_rotation: Quat,
}

#[derive(Debug, Clone)]
struct LodInfo {
    name: String,
    material_names: Vec<String>,
}

#[derive(Debug)]
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn skip(&mut self, count: usize) {
        self.pos = self.pos.saturating_add(count).min(self.data.len());
    }

    fn u8(&mut self) -> u8 {
        if self.pos < self.data.len() {
            let v = self.data[self.pos];
            self.pos += 1;
            v
        } else {
            0
        }
    }

    fn s8(&mut self) -> i8 {
        self.u8() as i8
    }

    fn u16(&mut self) -> u16 {
        if self.remaining() < 2 {
            self.pos = self.data.len();
            return 0;
        }
        let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        v
    }

    fn s16(&mut self) -> i16 {
        self.u16() as i16
    }

    fn u32(&mut self) -> u32 {
        if self.remaining() < 4 {
            self.pos = self.data.len();
            return 0;
        }
        let v = u32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        v
    }

    fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }

    fn strz(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != 0 {
            self.pos += 1;
        }
        let s = String::from_utf8_lossy(&self.data[start..self.pos]).to_string();
        if self.pos < self.data.len() {
            self.pos += 1;
        }
        s
    }

    fn vec3(&mut self) -> Vec3 {
        Vec3::new(self.f32(), self.f32(), self.f32())
    }

    fn compact_quat(&mut self) -> Quat {
        let x = self.s16() as f32 / 32768.0;
        let y = self.s16() as f32 / 32768.0;
        let z = self.s16() as f32 / 32768.0;
        let ww = 1.0 - x * x - y * y - z * z;
        let w = if ww > 0.0 { ww.sqrt() } else { 0.0 };
        Quat::from_xyzw(x, y, z, w)
    }
}

impl XModel {
    pub fn load(
        asset_db: &mut AssetDb,
        name: &str,
        shader_db: Option<&ShaderDb>,
    ) -> Result<Self, XModelError> {
        let virtual_path = normalize_xmodel_path(name);
        let root = read_virtual(asset_db, &virtual_path)?;
        let mut reader = Reader::new(&root);
        ensure_version(&mut reader, &virtual_path)?;

        let header_mins = reader.vec3();
        let header_maxs = reader.vec3();

        let mut lods = Vec::<LodInfo>::new();
        for _ in 0..XMODEL_MAX_LODS {
            let _lod_distance = reader.f32();
            let lod_name = reader.strz();
            if !lod_name.is_empty() {
                lods.push(LodInfo {
                    name: lod_name,
                    material_names: Vec::new(),
                });
            }
        }
        if lods.is_empty() {
            return Err(parse_err(&virtual_path, "xmodel has no lods"));
        }

        reader.skip(4);
        let collision_surfaces = reader.u32();
        for _ in 0..collision_surfaces.min(4096) {
            let tri_count = reader.u32() as usize;
            reader.skip(tri_count.saturating_mul(48).saturating_add(24));
            reader.skip(12);
        }

        for lod in &mut lods {
            let material_count = reader.u16() as usize;
            for _ in 0..material_count {
                lod.material_names
                    .push(normalize_material_name(&reader.strz()));
            }
        }

        let lod0 = &lods[0];
        let bones = load_parts(asset_db, &lod0.name)?;
        let origin = bones
            .first()
            .map(|bone| bone.world_translation)
            .unwrap_or(Vec3::ZERO);
        let mut surfaces = load_surfaces(asset_db, &lod0.name, &bones, lod0)?;

        // Phase 1: parallel texture name resolution (pure string work, no &mut needed).
        surfaces.par_iter_mut().for_each(|surface| {
            if surface.material_name.is_empty() {
                return;
            }
            surface.texture_name =
                resolve_xmodel_texture_name(&surface.material_name, shader_db, asset_db);
        });

        // Phase 2: sequential texture asset resolution (needs &mut AssetDb for case-insensitive fallback).
        for surface in &mut surfaces {
            if let Some(texture_name) = &surface.texture_name {
                surface.texture_asset = asset_db.resolve_texture(texture_name);
            }
        }

        let (mins, maxs, radius) = compute_bounds(&surfaces, origin, header_mins, header_maxs);

        Ok(Self {
            name: virtual_path,
            surfaces,
            mins,
            maxs,
            origin,
            radius,
        })
    }
}

pub fn model_wireframe_lines(model: &XModel, origin: Vec3, rot: Option<Quat>) -> Vec<Vec3> {
    model
        .surfaces
        .par_iter()
        .flat_map_iter(|surf| {
            let verts = &surf.vertices;
            surf.indices
                .chunks_exact(3)
                .filter_map(move |tri| {
                    let i0 = tri[0] as usize;
                    let i1 = tri[1] as usize;
                    let i2 = tri[2] as usize;
                    if i0 >= verts.len() || i1 >= verts.len() || i2 >= verts.len() {
                        return None;
                    }
                    let transform = |p: Vec3| origin + rot.map_or(p, |r| r * p);
                    let p0 = transform(verts[i0].position);
                    let p1 = transform(verts[i1].position);
                    let p2 = transform(verts[i2].position);
                    Some([p0, p1, p1, p2, p2, p0])
                })
                .flatten()
        })
        .collect()
}

pub fn resolve_xmodel_texture_name(
    material_name: &str,
    shader_db: Option<&ShaderDb>,
    asset_db: &AssetDb,
) -> Option<String> {
    let key = normalize_material_name(material_name).to_ascii_lowercase();

    if let Some(db) = shader_db {
        if let Some(shader) = db.get(&key) {
            if let Some(mapped) = shader.diffuse_map.as_deref() {
                if let Some(path) = resolve_texture_alias(asset_db, mapped) {
                    return Some(path);
                }
            }
        }
    }

    for candidate in texture_candidates_for_material(&key) {
        if let Some(path) = resolve_texture_alias(asset_db, &candidate) {
            return Some(path);
        }
    }

    None
}

fn resolve_texture_alias(asset_db: &AssetDb, name: &str) -> Option<String> {
    let normalized = normalize_material_name(name);
    let resolved = asset_db.resolve_texture_fast(&normalized)?;
    Some(match &resolved {
        ResolvedAsset::Loose(path) => path
            .strip_prefix(asset_db.roots().maindir())
            .ok()
            .and_then(|p| p.to_str())
            .map(normalize_asset_path)
            .unwrap_or_else(|| normalize_asset_path(&normalized)),
        ResolvedAsset::Pk3 { virtual_path, .. } => normalize_asset_path(virtual_path),
    })
}

fn texture_candidates_for_material(material: &str) -> Vec<String> {
    let base = strip_extension(material);
    let mut out = Vec::<String>::new();
    for candidate in [
        format!("skins/{material}"),
        format!("skins/{base}"),
        format!("textures/{base}"),
        format!("textures/{material}"),
        material.to_string(),
        base.to_string(),
    ] {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    add_template_skin_fallbacks(material, &mut out);
    out
}

fn add_template_skin_fallbacks(material: &str, out: &mut Vec<String>) {
    let Some(at) = material.find("@default") else {
        return;
    };
    let prefix = &material[..at];
    for candidate in [
        format!("skins/{prefix}@hand"),
        format!("skins/{prefix}@characterhand"),
    ] {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
}

fn strip_extension(path: &str) -> &str {
    let ext = Path::new(path).extension().and_then(|s| s.to_str());
    match ext {
        Some(_) => path.rsplit_once('.').map(|(head, _)| head).unwrap_or(path),
        None => path,
    }
}

fn load_parts(asset_db: &mut AssetDb, lod_name: &str) -> Result<Vec<Bone>, XModelError> {
    let path = format!("xmodelparts/{lod_name}");
    let data = read_virtual(asset_db, &path)?;
    let mut reader = Reader::new(&data);
    ensure_version(&mut reader, &path)?;

    let bone_count = reader.u16() as usize;
    let root_count = reader.u16() as usize;
    let total_bones = bone_count + root_count;
    if total_bones == 0 || total_bones > XMODEL_MAX_BONES {
        return Err(parse_err(&path, "invalid bone count"));
    }

    let mut bones = vec![
        Bone {
            parent: -1,
            local_translation: Vec3::ZERO,
            local_rotation: Quat::IDENTITY,
            world_translation: Vec3::ZERO,
            world_rotation: Quat::IDENTITY,
        };
        total_bones
    ];

    for i in 0..bone_count {
        let idx = root_count + i;
        bones[idx].parent = reader.s8();
        bones[idx].local_translation = reader.vec3();
        bones[idx].local_rotation = reader.compact_quat();
    }

    for bone in &mut bones {
        let _name = reader.strz();
        reader.skip(24);
        bone.world_translation = bone.local_translation;
        bone.world_rotation = bone.local_rotation;
    }

    for i in 0..bones.len() {
        if bones[i].parent < 0 {
            continue;
        }
        let parent = bones[i].parent as usize;
        let parent_rot = bones[parent].world_rotation;
        let parent_pos = bones[parent].world_translation;
        bones[i].world_translation = parent_pos + parent_rot * bones[i].local_translation;
        bones[i].world_rotation = parent_rot * bones[i].local_rotation;
    }

    Ok(bones)
}

fn load_surfaces(
    asset_db: &mut AssetDb,
    lod_name: &str,
    bones: &[Bone],
    lod: &LodInfo,
) -> Result<Vec<XModelSurface>, XModelError> {
    let path = format!("xmodelsurfs/{lod_name}");
    let data = read_virtual(asset_db, &path)?;
    let mut reader = Reader::new(&data);
    ensure_version(&mut reader, &path)?;

    let surface_count = reader.u16() as usize;
    if surface_count == 0 {
        return Err(parse_err(&path, "xmodelsurfs has no surfaces"));
    }

    let mut surfaces = Vec::with_capacity(surface_count);

    for surface_index in 0..surface_count {
        reader.skip(1);
        let vert_count = reader.u16() as usize;
        let tri_count = reader.u16() as usize;
        reader.skip(2);
        let original_bone = reader.u16();
        let rigged = original_bone == COD1_RIGGED;
        if rigged {
            reader.skip(4);
        }

        let mut indices = Vec::with_capacity(tri_count.saturating_mul(3));
        let mut decoded = 0usize;
        while decoded < tri_count {
            let got = decode_fan_strip(&mut reader, &mut indices, tri_count);
            if got == 0 {
                break;
            }
            decoded += got;
        }

        let mut weight_counts = vec![0usize; vert_count];
        let mut raw_vertices = Vec::with_capacity(vert_count);

        for weight_count in &mut weight_counts {
            let local_normal = reader.vec3();
            let uv = Vec2::new(reader.f32(), reader.f32());

            let mut bone_index = original_bone;
            let mut extra_weights = 0usize;
            if rigged {
                extra_weights = reader.u16() as usize;
                bone_index = reader.u16();
            }

            let local_position = reader.vec3();
            if extra_weights > 0 {
                reader.skip(4);
            }
            *weight_count = extra_weights;

            raw_vertices.push((bone_index, local_position, local_normal, uv));
        }

        for extra_weights in weight_counts {
            for _ in 0..extra_weights {
                let _bone_index = reader.u16();
                reader.skip(12);
                let _weight = reader.f32();
            }
        }

        // Phase 2: parallel bone transformation (pure computation, no I/O).
        let vertices: Vec<XModelVertex> = raw_vertices
            .par_iter()
            .with_min_len(64)
            .map(|&(bone_index, local_position, local_normal, uv)| {
                let (position, normal) =
                    transform_vertex(bones, bone_index, local_position, local_normal);
                XModelVertex {
                    position,
                    normal,
                    uv,
                }
            })
            .collect();

        // CoD xmodel surfaces come in opposite winding to the renderer's front-face
        // convention, so flip each triangle here instead of disabling culling globally.
        for tri in indices.chunks_mut(3) {
            if tri.len() == 3 {
                tri.swap(1, 2);
            }
        }

        let material_name = if lod.material_names.is_empty() {
            String::new()
        } else {
            lod.material_names[surface_index.min(lod.material_names.len() - 1)].clone()
        };

        surfaces.push(XModelSurface {
            vertices,
            indices,
            material_name,
            texture_name: None,
            texture_asset: None,
        });
    }

    Ok(surfaces)
}

fn transform_vertex(
    bones: &[Bone],
    bone_index: u16,
    local_position: Vec3,
    local_normal: Vec3,
) -> (Vec3, Vec3) {
    let Some(bone) = bones.get(bone_index as usize) else {
        return (local_position, local_normal.normalize_or_zero());
    };
    let position = bone.world_translation + bone.world_rotation * local_position;
    let normal = (bone.world_rotation * local_normal).normalize_or_zero();
    (position, normal)
}

fn decode_fan_strip(reader: &mut Reader<'_>, out: &mut Vec<u32>, max_tris: usize) -> usize {
    let count = reader.u8();
    if count < 3 {
        return 0;
    }

    let mut written = 0usize;
    let i1 = reader.u16() as u32;
    let mut i2 = reader.u16() as u32;
    let mut i3 = reader.u16() as u32;
    if is_valid_triangle(i1, i2, i3) && out.len() / 3 < max_tris {
        out.extend_from_slice(&[i1, i2, i3]);
        written += 1;
    }

    let mut k = 3u8;
    while k < count {
        let i4 = i3;
        let i5 = reader.u16() as u32;
        k += 1;
        if is_valid_triangle(i4, i2, i5) && out.len() / 3 < max_tris {
            out.extend_from_slice(&[i4, i2, i5]);
            written += 1;
        }
        if k >= count {
            break;
        }
        i2 = i5;
        i3 = reader.u16() as u32;
        k += 1;
        if is_valid_triangle(i4, i2, i3) && out.len() / 3 < max_tris {
            out.extend_from_slice(&[i4, i2, i3]);
            written += 1;
        }
    }

    written
}

fn is_valid_triangle(a: u32, b: u32, c: u32) -> bool {
    a != b && a != c && b != c
}

fn compute_bounds(
    surfaces: &[XModelSurface],
    origin: Vec3,
    header_mins: Vec3,
    header_maxs: Vec3,
) -> (Vec3, Vec3, f32) {
    let (mins, maxs) = surfaces
        .par_iter()
        .flat_map_iter(|s| s.vertices.iter())
        .map(|v| v.position - origin)
        .fold(
            || (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(mut mn, mut mx), pos| {
                mn = mn.min(pos);
                mx = mx.max(pos);
                (mn, mx)
            },
        )
        .reduce(
            || (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(mn_a, mx_a), (mn_b, mx_b)| (mn_a.min(mn_b), mx_a.max(mx_b)),
        );

    if !mins.is_finite() || !maxs.is_finite() {
        return (header_mins - origin, header_maxs - origin, 10.0);
    }

    let radius = ((maxs - mins).length() * 0.5).max(0.1);
    (mins, maxs, radius)
}

fn normalize_xmodel_path(name: &str) -> String {
    let normalized = normalize_material_name(name);
    if normalized.starts_with("xmodel/") {
        normalized
    } else {
        format!("xmodel/{normalized}")
    }
}

fn read_virtual(asset_db: &mut AssetDb, path: &str) -> Result<Vec<u8>, XModelError> {
    let Some(asset) = asset_db.resolve_virtual_path(path) else {
        return Err(XModelError::Asset(AssetDbError::NotFound(path.to_string())));
    };
    Ok(asset_db.read(&asset)?)
}

fn ensure_version(reader: &mut Reader<'_>, path: &str) -> Result<(), XModelError> {
    let version = reader.u16();
    if version == COD1_XMODEL_VERSION {
        Ok(())
    } else {
        Err(parse_err(
            path,
            &format!("unsupported xmodel version {version:#x}"),
        ))
    }
}

fn parse_err(path: &str, msg: &str) -> XModelError {
    XModelError::Parse {
        path: path.to_string(),
        msg: msg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_extensions() {
        assert_eq!(strip_extension("textures/foo/bar.tga"), "textures/foo/bar");
        assert_eq!(strip_extension("skins/test@default"), "skins/test@default");
    }

    #[test]
    fn adds_skin_fallbacks_for_default_templates() {
        let mut out = vec![];
        add_template_skin_fallbacks("viewmodel@default", &mut out);
        assert_eq!(
            out,
            vec![
                "skins/viewmodel@hand".to_string(),
                "skins/viewmodel@characterhand".to_string()
            ]
        );
    }
}
