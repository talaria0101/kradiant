//! Recursive‑descent parser for idTech‑style `.map` source files.
//!
//! - Handles the full nested `{}` structure for entities and brushes.
//! - Supports both terrain and Bezier patches (`patchTerrainDef3`, `patchDef5`).
//! - Works on a line‑based token stream to keep allocations low.
//! - Produces detailed error messages with exact line numbers.

use crate::map::{
    Brush, BrushContent, BrushId, Entity, EntityId, Face, Map, Patch, PatchParams, PatchType,
    PatchVertex, SurfaceFlags, TextureParams,
};
use crate::{IVec2, Vec2, Vec3};
use log::debug;
use std::collections::HashMap;
use std::fmt::Write;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Read Float error: {0}")]
    ReadFloat(#[from] std::num::ParseFloatError),

    #[error("Syntax error on line {line}: {msg}")]
    Syntax { line: usize, msg: String },

    #[error("Invalid plane definition on line {line}")]
    InvalidPlane { line: usize },

    #[error("Unsupported primitive `{kind}` on line {line}")]
    UnsupportedPrimitive { line: usize, kind: String },
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    pos: usize,
    current_line: usize,
}

impl<'a> Parser<'a> {
    fn new(content: &'a str) -> Self {
        Self {
            lines: content.lines().collect(),
            pos: 0,
            current_line: 0,
        }
    }

    fn next_line(&mut self) -> Option<&'a str> {
        while self.pos < self.lines.len() {
            let line = self.lines[self.pos].trim();
            self.pos += 1;
            self.current_line += 1;
            if !line.is_empty() && !line.starts_with("//") {
                return Some(line);
            }
        }
        None
    }

    fn tokenize(line: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            if bytes[i] == b'"' {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += 1;
                }
                tokens.push(
                    std::str::from_utf8(&bytes[start..i])
                        .unwrap_or("")
                        .to_string(),
                );
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                tokens.push(
                    std::str::from_utf8(&bytes[start..i])
                        .unwrap_or("")
                        .to_string(),
                );
            }
        }
        tokens
    }

    fn parse_vec3(tokens: &[String], start: usize, line: usize) -> Result<Vec3, ParseError> {
        if start + 2 >= tokens.len() {
            return Err(ParseError::InvalidPlane { line });
        }
        Ok(Vec3::new(
            tokens[start]
                .parse()
                .map_err(|_| ParseError::InvalidPlane { line })?,
            tokens[start + 1]
                .parse()
                .map_err(|_| ParseError::InvalidPlane { line })?,
            tokens[start + 2]
                .parse()
                .map_err(|_| ParseError::InvalidPlane { line })?,
        ))
    }

    /*fn parse_vec4(tokens: &[String], start: usize, line: usize) -> Result<Vec4, ParseError> {
        if start + 3 >= tokens.len() {
            return Err(ParseError::InvalidPlane { line });
        }
        Ok(Vec4::new(
            tokens[start].parse().map_err(|_| ParseError::InvalidPlane { line })?,
                     tokens[start + 1].parse().map_err(|_| ParseError::InvalidPlane { line })?,
                     tokens[start + 2].parse().map_err(|_| ParseError::InvalidPlane { line })?,
                     tokens[start + 3].parse().map_err(|_| ParseError::InvalidPlane { line })?,
        ))
    }*/

    fn parse_map(&mut self) -> Result<Map, ParseError> {
        let mut map = Map::default();
        let mut entity_id = 0u32;

        while let Some(line) = self.next_line() {
            if line == "{" {
                let mut entity = self.parse_entity_body()?;
                entity.id = EntityId(entity_id);
                map.entities.push(entity);
                entity_id += 1;
            }
        }
        debug!("Parsed {} entities (CoD1 .map)", map.entities.len());
        Ok(map)
    }

    fn parse_entity_body(&mut self) -> Result<Entity, ParseError> {
        let mut entity = Entity {
            id: EntityId(0),
            classname: String::new(),
            properties: HashMap::new(),
            brushes: vec![],
        };
        let mut brush_id = 0u32;

        while let Some(line) = self.next_line() {
            let tokens = Self::tokenize(line);
            if tokens.is_empty() {
                continue;
            }

            match tokens[0].as_str() {
                "{" => {
                    let content = self.parse_brush_or_primitive()?;
                    entity.brushes.push(Brush::new(BrushId(brush_id), content));
                    brush_id += 1;
                }
                "}" => break,
                _ if tokens.len() >= 2 => {
                    let key = tokens[0].clone();
                    let value = tokens[1].clone();
                    if key == "classname" {
                        entity.classname = value.clone();
                    }
                    entity.properties.insert(key, value);
                }
                _ => {}
            }
        }
        Ok(entity)
    }

    fn parse_brush_or_primitive(&mut self) -> Result<BrushContent, ParseError> {
        let mut faces = vec![];

        while let Some(line) = self.next_line() {
            let tokens = Self::tokenize(line);
            if tokens.is_empty() {
                continue;
            }

            if tokens[0] == "}" {
                return Ok(BrushContent::Convex(faces));
            }
            if tokens[0] == "patchTerrainDef3" || tokens[0] == "patchDef5" {
                let patch_type = if tokens[0] == "patchTerrainDef3" {
                    PatchType::Terrain
                } else {
                    PatchType::Curve
                };
                let patch = self.parse_patch(patch_type)?;

                // Patch brushes are nested like:
                // { patchDef5 { ... } }
                // After parsing the inner patch block, we should see the outer brush closing brace.
                let Some(next) = self.next_line() else {
                    return Err(ParseError::Syntax {
                        line: self.current_line,
                        msg: "unclosed brush after patch".into(),
                    });
                };
                if next != "}" {
                    return Err(ParseError::Syntax {
                        line: self.current_line,
                        msg: format!("expected '}}' after patch, got: {next}"),
                    });
                }

                return Ok(BrushContent::Patch(patch));
            }
            if tokens[0].starts_with("patch") {
                return Err(ParseError::UnsupportedPrimitive {
                    line: self.current_line,
                    kind: tokens[0].clone(),
                });
            }
            if tokens[0] == "(" {
                let face = self.parse_face_from_tokens(&tokens)?;
                faces.push(face);
            }
        }
        Err(ParseError::Syntax {
            line: self.current_line,
            msg: "unclosed brush".into(),
        })
    }

    fn parse_face_from_tokens(&self, tokens: &[String]) -> Result<Face, ParseError> {
        let p1 = Self::parse_vec3(tokens, 1, self.current_line)?;
        let p2 = Self::parse_vec3(tokens, 6, self.current_line)?;
        let p3 = Self::parse_vec3(tokens, 11, self.current_line)?;

        let texture = tokens
            .get(15)
            .cloned()
            .ok_or_else(|| ParseError::InvalidPlane {
                line: self.current_line,
            })?;

        // CoD1 Radiant: 9 numbers after texture (exactly what Surface Inspector writes)
        let mut nums = [0.0f32; 9];
        for i in 0..9 {
            if let Some(s) = tokens.get(16 + i) {
                nums[i] = s.parse().unwrap_or(0.0);
            }
        }

        let params = TextureParams {
            shift: IVec2 {
                x: nums[0] as i32,
                y: nums[1] as i32,
            },
            rotate: nums[2] as i32,
            scale: Vec2 {
                x: nums[3],
                y: nums[4],
            },
            surface_flags: SurfaceFlags::from_u32(nums[5] as u32),
            idk: nums[6],
            value: nums[7] as i32,
            sample_size: nums[8] as i32,
        };

        Ok(Face {
            plane_points: [p1, p2, p3],
            texture,
            params,
        })
    }

    fn parse_patch(&mut self, patch_type: PatchType) -> Result<Patch, ParseError> {
        // Expected structure (inner block):
        // {
        // shader
        // ( rows cols contents 0 0 0 subdiv )
        // (
        // ( ( x y z u v r g b a turned ) ... )
        // ...
        // )
        // }

        let Some(line) = self.next_line() else {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: "unexpected EOF after patch keyword".into(),
            });
        };
        if line != "{" {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: format!("expected '{{' after patch keyword, got: {line}"),
            });
        }

        let Some(shader_line) = self.next_line() else {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: "unexpected EOF while reading patch shader".into(),
            });
        };
        let shader = shader_line.trim().trim_matches('"').to_string();

        let Some(params_line) = self.next_line() else {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: "unexpected EOF while reading patch params".into(),
            });
        };
        let params = self.parse_patch_params(params_line)?;

        let Some(open) = self.next_line() else {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: "unexpected EOF while reading patch vertices".into(),
            });
        };
        if open != "(" {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: format!("expected '(' to begin patch vertices, got: {open}"),
            });
        }

        let mut rows: Vec<Vec<PatchVertex>> = Vec::new();
        while let Some(vline) = self.next_line() {
            if vline == ")" {
                break;
            }
            if vline.starts_with('(') {
                let row = self.parse_patch_vertex_row(vline)?;
                rows.push(row);
            } else {
                return Err(ParseError::Syntax {
                    line: self.current_line,
                    msg: format!("unexpected token in patch vertices: {vline}"),
                });
            }
        }

        let Some(close) = self.next_line() else {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: "unexpected EOF while closing patch".into(),
            });
        };
        if close != "}" {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: format!("expected '}}' to close patch, got: {close}"),
            });
        }

        // Validate dimensions when present.
        if params.rows != 0 && rows.len() != params.rows as usize {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: format!(
                    "patch row count mismatch: params.rows={} but parsed {} rows",
                    params.rows,
                    rows.len()
                ),
            });
        }
        if params.cols != 0 {
            for (ri, r) in rows.iter().enumerate() {
                if r.len() != params.cols as usize {
                    return Err(ParseError::Syntax {
                        line: self.current_line,
                        msg: format!(
                            "patch col count mismatch at row {ri}: params.cols={} but parsed {} cols",
                            params.cols,
                            r.len()
                        ),
                    });
                }
            }
        }

        Ok(Patch::new(patch_type, shader, params, rows))
    }

    fn parse_patch_params(&self, line: &str) -> Result<PatchParams, ParseError> {
        let tokens = Self::tokenize(line);
        let mut nums: Vec<i32> = Vec::new();
        for t in tokens {
            if t == "(" || t == ")" {
                continue;
            }
            nums.push(t.parse().map_err(|_| ParseError::Syntax {
                line: self.current_line,
                msg: format!("invalid patch param int: {t}"),
            })?);
        }

        let rows = *nums.get(0).unwrap_or(&3);
        let cols = *nums.get(1).unwrap_or(&3);
        if rows <= 0 || cols <= 0 {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: format!("invalid patch size: {rows}x{cols}"),
            });
        }

        Ok(PatchParams {
            rows: rows as u32,
            cols: cols as u32,
            contents: *nums.get(2).unwrap_or(&0),
            reserved: [
                *nums.get(3).unwrap_or(&0),
                *nums.get(4).unwrap_or(&0),
                *nums.get(5).unwrap_or(&0),
            ],
            subdivision: (*nums.get(6).unwrap_or(&8)).max(0) as u32,
        })
    }

    fn parse_patch_vertex_row(&self, line: &str) -> Result<Vec<PatchVertex>, ParseError> {
        let tokens = Self::tokenize(line);
        if tokens.len() < 2 || tokens.first().map(|s| s.as_str()) != Some("(") {
            return Err(ParseError::Syntax {
                line: self.current_line,
                msg: "invalid patch vertex row".into(),
            });
        }

        let mut out: Vec<PatchVertex> = Vec::new();
        let mut i = 1usize; // skip outer '('
        while i < tokens.len() {
            let t = tokens[i].as_str();
            if t == ")" {
                break; // end of row
            }
            if t != "(" {
                i += 1;
                continue;
            }
            // vertex: ( x y z u v r g b a turned )
            if i + 11 >= tokens.len() {
                return Err(ParseError::Syntax {
                    line: self.current_line,
                    msg: "truncated patch vertex".into(),
                });
            }
            let x: f32 = tokens[i + 1].parse().map_err(|_| ParseError::Syntax {
                line: self.current_line,
                msg: format!("invalid patch vertex x: {}", tokens[i + 1]),
            })?;
            let y: f32 = tokens[i + 2].parse().map_err(|_| ParseError::Syntax {
                line: self.current_line,
                msg: format!("invalid patch vertex y: {}", tokens[i + 2]),
            })?;
            let z: f32 = tokens[i + 3].parse().map_err(|_| ParseError::Syntax {
                line: self.current_line,
                msg: format!("invalid patch vertex z: {}", tokens[i + 3]),
            })?;
            let u: f32 = tokens[i + 4].parse().map_err(|_| ParseError::Syntax {
                line: self.current_line,
                msg: format!("invalid patch vertex u: {}", tokens[i + 4]),
            })?;
            let v: f32 = tokens[i + 5].parse().map_err(|_| ParseError::Syntax {
                line: self.current_line,
                msg: format!("invalid patch vertex v: {}", tokens[i + 5]),
            })?;

            let r: u8 = tokens[i + 6]
                .parse::<i32>()
                .map_err(|_| ParseError::Syntax {
                    line: self.current_line,
                    msg: format!("invalid patch vertex r: {}", tokens[i + 6]),
                })?
                .clamp(0, 255) as u8;
            let g: u8 = tokens[i + 7]
                .parse::<i32>()
                .map_err(|_| ParseError::Syntax {
                    line: self.current_line,
                    msg: format!("invalid patch vertex g: {}", tokens[i + 7]),
                })?
                .clamp(0, 255) as u8;
            let b: u8 = tokens[i + 8]
                .parse::<i32>()
                .map_err(|_| ParseError::Syntax {
                    line: self.current_line,
                    msg: format!("invalid patch vertex b: {}", tokens[i + 8]),
                })?
                .clamp(0, 255) as u8;
            let a: u8 = tokens[i + 9]
                .parse::<i32>()
                .map_err(|_| ParseError::Syntax {
                    line: self.current_line,
                    msg: format!("invalid patch vertex a: {}", tokens[i + 9]),
                })?
                .clamp(0, 255) as u8;

            let turned: u8 = tokens[i + 10]
                .parse::<i32>()
                .map_err(|_| ParseError::Syntax {
                    line: self.current_line,
                    msg: format!("invalid patch vertex turned_edge: {}", tokens[i + 10]),
                })?
                .clamp(0, 255) as u8;

            if tokens[i + 11] != ")" {
                return Err(ParseError::Syntax {
                    line: self.current_line,
                    msg: "expected ')' after patch vertex".into(),
                });
            }

            out.push(PatchVertex {
                position: Vec3::new(x, y, z),
                uv: Vec2::new(u, v),
                color: [r, g, b, a],
                turned_edge: turned != 0,
            });

            i += 12;
        }

        Ok(out)
    }
}

impl Map {
    /// Serialize the entire map back to exact CoD1 .map text format
    pub fn to_map_string(&self) -> String {
        let mut out = String::new();

        for (i, entity) in self.entities.iter().enumerate() {
            if i != 0 {
                writeln!(out, "// entity {}", i).unwrap();
            }
            out.push_str(&entity.to_map_string());
            //out.push('\n');
        }
        out
    }
}

impl Entity {
    pub fn to_map_string(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");

        // classname first
        writeln!(out, "\"classname\" \"{}\"", self.classname).unwrap();
        for (k, v) in &self.properties {
            if k == "classname" {
                continue;
            }
            writeln!(out, "\"{}\" \"{}\"", k, v).unwrap();
        }

        // brushes
        for (i, brush) in self.brushes.iter().enumerate() {
            writeln!(out, "// brush {}", i).unwrap();
            out.push_str(&brush.to_map_string());
        }

        out.push_str("}\n");
        out
    }
}

impl Brush {
    pub fn to_map_string(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");

        match &self.content {
            BrushContent::Convex(faces) => {
                for face in faces {
                    out.push_str(&face.to_map_string());
                    out.push('\n');
                }
            }
            BrushContent::Patch(patch) => {
                out.push_str(&patch.to_map_string());
            }
        }

        out.push_str("}\n");
        out
    }
}

impl Face {
    pub fn to_map_string(&self) -> String {
        let p = self.plane_points;
        let f = self.params;

        format!(
            "( {:.0} {:.0} {:.0} ) ( {:.0} {:.0} {:.0} ) ( {:.0} {:.0} {:.0} ) {} {shift_x} {shift_y} {rot} {scale_x} {scale_y} {s_flags} {idk} {value} {sample_size}",
            p[0].x,
            p[0].y,
            p[0].z,
            p[1].x,
            p[1].y,
            p[1].z,
            p[2].x,
            p[2].y,
            p[2].z,
            self.texture,
            shift_x = f.shift.x,
            shift_y = f.shift.y,
            rot = f.rotate,
            scale_x = format!("{:.6}", f.scale.x)
                .trim_end_matches("0")
                .trim_end_matches("."),
            scale_y = format!("{:.6}", f.scale.y)
                .trim_end_matches("0")
                .trim_end_matches("."),
            s_flags = f.surface_flags.as_u32(),
            idk = f.idk,
            value = f.value,
            sample_size = f.sample_size
        )
    }
}

impl Patch {
    pub fn to_map_string(&self) -> String {
        let type_name = match self.patch_type {
            PatchType::Terrain => "patchTerrainDef3",
            PatchType::Curve => "patchDef5",
        };

        let mut out = format!("{}\n{{\n{}\n", type_name, self.shader);

        // params line
        writeln!(
            out,
            "( {} {} {} 0 0 0 {} )",
            self.params.rows, self.params.cols, self.params.contents, self.params.subdivision
        )
        .unwrap();

        // vertices
        out.push_str("(\n");
        for row in &self.vertices {
            out.push_str("  ( ");
            for v in row {
                let turned = if v.turned_edge { 1 } else { 0 };
                write!(
                    out,
                    "( {:.0} {:.0} {:.0} {:.6} {:.6} {} {} {} {} {} ) ",
                    v.position.x,
                    v.position.y,
                    v.position.z,
                    v.uv.x,
                    v.uv.y,
                    v.color[0],
                    v.color[1],
                    v.color[2],
                    v.color[3],
                    turned
                )
                .unwrap();
            }
            out.push_str(")\n");
        }

        out.push_str(")\n}\n");
        out
    }
}

/// Parse a `.map` source string into an in‑memory [`Map`].
pub fn parse_map_string(content: &str) -> Result<Map, ParseError> {
    let mut parser = Parser::new(content);
    parser.parse_map()
}

/// Read a `.map` file from disk and parse it into a [`Map`].
pub fn load_map(path: &str) -> Result<Map, ParseError> {
    let content = std::fs::read_to_string(path)?;
    parse_map_string(&content)
}

/// Serialize a [`Map`] back to `.map` text and write it to disk.
pub fn save_map(map: &Map, path: &str) -> Result<(), ParseError> {
    let content = map.to_map_string();
    std::fs::write(path, content).map_err(|e| ParseError::Io(e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{IVec3, Vec3};

    const SIMPLE_BOX_MAP: &str = include_str!("../test/simple_box.map");

    const MULTI_ENTITY_MAP: &str = include_str!("../test/multi_entity.map");

    const PATCH_DEF5_MAP: &str = r#"
{
"classname" "worldspawn"
{
patchDef5
{
common/caulk
( 3 3 0 0 0 0 8 )
(
( ( 0 0 0 0 0 255 255 255 255 0 ) ( 64 0 0 1 0 255 255 255 255 0 ) ( 128 0 0 2 0 255 255 255 255 0 ) )
( ( 0 64 0 0 1 255 255 255 255 0 ) ( 64 64 0 1 1 255 255 255 255 0 ) ( 128 64 0 2 1 255 255 255 255 0 ) )
( ( 0 128 0 0 2 255 255 255 255 0 ) ( 64 128 0 1 2 255 255 255 255 0 ) ( 128 128 0 2 2 255 255 255 255 0 ) )
)
}
}
}
"#;

    #[test]
    fn parses_simple_box_worldspawn() {
        let map = parse_map_string(SIMPLE_BOX_MAP).unwrap();

        assert_eq!(map.entities.len(), 1);

        let world = &map.entities[0];
        assert_eq!(world.classname, "worldspawn");
        assert_eq!(world.brushes.len(), 1);

        if let BrushContent::Convex(faces) = &world.brushes[0].content {
            assert_eq!(faces.len(), 6);
            assert_eq!(faces[0].plane_points[0], Vec3::new(288.0, 264.0, -146.0));
            assert_eq!(faces[0].texture, "common/caulk");
            assert_eq!(faces[0].params.shift, IVec2::new(0, 0));
            assert_eq!(faces[0].params.rotate, 0);
            assert_eq!(faces[0].params.scale, Vec2::new(0.25, 0.25));
            assert_eq!(faces[0].params.surface_flags, SurfaceFlags::Structural);
            assert_eq!(faces[0].params.idk, 0.0);
            assert_eq!(faces[0].params.value, 0);
            assert_eq!(faces[0].params.sample_size, 0);
        } else {
            panic!("Expected Convex brush");
        }
    }

    #[test]
    fn parses_multiple_entities() {
        let map = parse_map_string(MULTI_ENTITY_MAP).unwrap();

        assert_eq!(map.entities.len(), 5);

        assert_eq!(map.entities[0].classname, "worldspawn");
        /*assert_eq!(
            map.entities[0].properties.get("diffusefraction"),
            Some(&"0.6".to_string())
        );*/

        assert_eq!(map.entities[1].classname, "mp_deathmatch_intermission");
        assert_eq!(
            map.entities[1].properties.get("origin"),
            Some(&"-124 -16 16".to_string())
        );

        assert_eq!(map.entities[4].classname, "trigger_use");
        assert_eq!(
            map.entities[4].properties.get("targetname"),
            Some(&"start_button".to_string())
        );
    }

    #[test]
    fn parses_patch_def5_curve() {
        let map = parse_map_string(PATCH_DEF5_MAP).unwrap();
        assert_eq!(map.entities.len(), 1);
        let world = &map.entities[0];
        assert_eq!(world.classname, "worldspawn");
        assert_eq!(world.brushes.len(), 1);

        match &world.brushes[0].content {
            BrushContent::Patch(p) => {
                assert_eq!(p.patch_type, PatchType::Curve);
                assert_eq!(p.shader, "common/caulk");
                assert_eq!(p.params.rows, 3);
                assert_eq!(p.params.cols, 3);
                assert_eq!(p.vertices.len(), 3);
                assert_eq!(p.vertices[0].len(), 3);
                assert_eq!(p.vertices[0][0].position, Vec3::new(0.0, 0.0, 0.0));
                assert_eq!(p.vertices[2][2].uv, Vec2::new(2.0, 2.0));
            }
            _ => panic!("Expected Patch brush"),
        }
    }

    #[test]
    fn errors_on_malformed_plane() {
        let bad = r#"
{
"classname" "worldspawn"
{
( 0 0 0 ) ( 64 0 0 ) BAD
}
}
"#;
        let err = parse_map_string(bad).unwrap_err();
        assert!(matches!(err, ParseError::InvalidPlane { .. }));
    }

    #[test]
    fn errors_on_unclosed_brush() {
        let bad = r#"
{
"classname" "worldspawn"
{
"#; // missing }
        let err = parse_map_string(bad).unwrap_err();
        eprintln!("{err}");
        assert!(matches!(err, ParseError::Syntax { .. }));
    }

    #[test]
    fn parse_real_map() {
        let map = include_str!("../test/village.map");
        let res = parse_map_string(map);
        /*if res.is_err() {
            eprintln!("{}", res.as_ref().unwrap_err());
        }*/
        assert!(res.is_ok());
    }

    #[test]
    fn aabb_is_stable_for_grid_aligned_brushes() {
        // This map is authored with integer plane points; due to float intersection epsilon,
        // tessellated vertices can land at e.g. `88.00001`, which must not inflate the integer AABB.
        let map = include_str!("../test/kradiant_map.map");
        let mut map = parse_map_string(map).unwrap();

        let world = &mut map.entities[0];
        let brush = &mut world.brushes[0];
        let (aabb, _polys) = brush.get_polygons_and_aabb().expect("tessellate brush");

        assert_eq!(aabb.min, IVec3::new(16, -76, 0));
        assert_eq!(aabb.max, IVec3::new(68, 0, 128));

        assert_eq!(aabb.max.x - aabb.min.x, 52);
        assert_eq!(aabb.max.y - aabb.min.y, 76);
        assert_eq!(aabb.max.z - aabb.min.z, 128);
    }
}
