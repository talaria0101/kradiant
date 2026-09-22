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
use crate::map_utils::format_float;
use crate::{IVec2, Vec2, Vec3};
use log::debug;
use rayon::iter::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};
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
    range: Range,
}

#[derive(Clone, Copy)]
struct Range {
    start: usize,
    end: usize,
}

impl<'a> Parser<'a> {
    fn new(content: &'a str) -> Self {
        let lines: Vec<&'a str> = content.lines().collect();
        let end = lines.len();
        Self {
            lines,
            pos: 0,
            current_line: 0,
            range: Range { start: 0, end },
        }
    }

    fn new_from_range(lines: Vec<&'a str>, range: Range) -> Self {
        Self {
            lines,
            pos: range.start,
            current_line: range.start,
            range,
        }
    }

    fn next_line(&mut self) -> Option<&'a str> {
        while self.pos < self.lines.len() && self.current_line < self.range.end {
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

    fn parse_map(&mut self) -> Result<Map, ParseError> {
        let mut map = Map::default();

        let entity_ranges = self.collect_entity_ranges()?;

        let entites: Vec<_> = entity_ranges
            .par_iter()
            .with_min_len(12)
            .map(|(idx, r)| {
                let mut sub_parser = Self::new_from_range(self.lines.clone(), *r);
                let mut entity = sub_parser.parse_entity_body()?;
                entity.id = EntityId(*idx);
                Ok(entity)
            })
            .collect::<Result<Vec<_>, ParseError>>()?;

        map.entities = entites;

        debug!("Parsed {} entities (CoD1 .map)", map.entities.len());
        Ok(map)
    }

    fn collect_entity_ranges(&self) -> Result<Vec<(u32, Range)>, ParseError> {
        let mut r = Vec::new();
        let mut idx = 0;
        let mut depth = 0i32;
        let mut cur_start = 0;
        for (i, l) in self.lines.iter().enumerate() {
            let trimmed = l.trim_start();
            if trimmed.starts_with("{") {
                if depth == 0 {
                    cur_start = i;
                }
                depth += 1;
            } else if trimmed.starts_with("}") {
                depth -= 1;
                if depth == 0 {
                    r.push((
                        idx,
                        Range {
                            start: cur_start + 1,
                            end: i + 1,
                        },
                    ));
                    idx += 1;
                }
            }
        }

        if r.is_empty() {
            Err(ParseError::Syntax {
                line: 0,
                msg: "No Entities to in map".to_string(),
            })
        } else {
            Ok(r)
        }
    }

    fn parse_entity_body(&mut self) -> Result<Entity, ParseError> {
        let mut entity = Entity {
            id: EntityId(0),
            classname: String::new(),
            properties: HashMap::new(),
            brushes: vec![],
            model: None,
        };

        // Phase 1: sequential property parsing until first brush or entity end.
        while let Some(line) = self.next_line() {
            let tokens = Self::tokenize(line);
            if tokens.is_empty() {
                continue;
            }
            match tokens[0].as_str() {
                "{" => break,
                "}" => return Ok(entity),
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

        // Phase 2: collect brush/primitive ranges within the entity range.
        let brush_ranges = self.collect_brush_ranges();

        // Phase 3: parse each brush in parallel.
        let brushes: Vec<_> = brush_ranges
            .par_iter()
            .with_min_len(1)
            .enumerate()
            .map(|(i, r)| {
                let mut sub = Self::new_from_range(self.lines.clone(), *r);
                let content = sub.parse_brush_or_primitive()?;
                Ok(Brush::new(BrushId(i as u32), content))
            })
            .collect::<Result<Vec<_>, ParseError>>()?;

        entity.brushes = brushes;
        Ok(entity)
    }

    fn collect_brush_ranges(&self) -> Vec<Range> {
        let mut ranges = Vec::new();
        let mut depth = 0i32;
        let mut cur_start = 0;
        for i in self.range.start..self.range.end {
            let trimmed = self.lines[i].trim_start();
            if trimmed.starts_with("{") {
                if depth == 0 {
                    cur_start = i + 1;
                }
                depth += 1;
            } else if trimmed.starts_with("}") {
                depth -= 1;
                if depth == 0 {
                    ranges.push(Range {
                        start: cur_start,
                        end: i + 1,
                    });
                }
            }
        }
        ranges
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

        let mut row_lines = Vec::new();

        // let mut rows: Vec<Vec<PatchVertex>> = Vec::new();
        while let Some(vline) = self.next_line() {
            if vline == ")" {
                break;
            }
            row_lines.push(vline);
        }
        let rows: Vec<Vec<PatchVertex>> = row_lines
            .par_iter()
            .with_min_len(128)
            .map(|line| self.parse_patch_vertex_row(line))
            .collect::<Result<_, _>>()?;

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
            "( {} {} {} ) ( {} {} {} ) ( {} {} {} ) {} {shift_x} {shift_y} {rot} {scale_x} {scale_y} {s_flags} {idk} {value} {sample_size}",
            format_float(p[0].x, 4),
            format_float(p[0].y, 4),
            format_float(p[0].z, 4),
            format_float(p[1].x, 4),
            format_float(p[1].y, 4),
            format_float(p[1].z, 4),
            format_float(p[2].x, 4),
            format_float(p[2].y, 4),
            format_float(p[2].z, 4),
            self.texture,
            shift_x = f.shift.x,
            shift_y = f.shift.y,
            rot = f.rotate,
            scale_x = format_float(f.scale.x, 2),
            scale_y = format_float(f.scale.y, 2),
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

        let mut out = format!("{}\n{{\n{}\n", type_name, self.texture);

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
                    "( {} {} {} {} {} {} {} {} {} {} ) ",
                    format_float(v.position.x, 4),
                    format_float(v.position.y, 4),
                    format_float(v.position.z, 4),
                    format_float(v.uv.x, 4),
                    format_float(v.uv.y, 4),
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
    use glam::Vec3;

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
                assert_eq!(p.texture, "common/caulk");
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

    // Real patch data extracted from zh_frenzy_br.map

    const PATCH_DEF5_9X3: &str = r#"{
"classname" "worldspawn"
{
patchDef5
{
 battleship/metal@grey
 ( 9 3 0 0 0 0 8 )
(
( ( 3408 -3216 79.999985 0 0 255 255 255 255 0 ) ( 3408 -3216 268 0 1.468750 255 255 255 255 0 ) ( 3408 -3216 456 0 2.937500 255 255 255 255 0 ) )
( ( 3408 -3472 79.999985 2 0 255 255 255 255 0 ) ( 3408 -3472 268 2 1.468750 255 255 255 255 0 ) ( 3408 -3472 456 2 2.937500 255 255 255 255 0 ) )
( ( 3672 -3472 79.999985 4.062500 0 255 255 255 255 0 ) ( 3672 -3472 268 4.062500 1.468750 255 255 255 255 0 ) ( 3672 -3472 456 4.062500 2.937500 255 255 255 255 0 ) )
( ( 3936 -3472 79.999985 6.125000 0 255 255 255 255 0 ) ( 3936 -3472 268 6.125000 1.468750 255 255 255 255 0 ) ( 3936 -3472 456 6.125000 2.937500 255 255 255 255 0 ) )
( ( 3936 -3216 79.999985 8.125000 0 255 255 255 255 0 ) ( 3936 -3216 268 8.125000 1.468750 255 255 255 255 0 ) ( 3936 -3216 456 8.125000 2.937500 255 255 255 255 0 ) )
( ( 3936 -2960 79.999985 10.125000 0 255 255 255 255 0 ) ( 3936 -2960 268 10.125000 1.468750 255 255 255 255 0 ) ( 3936 -2960 456 10.125000 2.937500 255 255 255 255 0 ) )
( ( 3672 -2960 79.999985 12.187500 0 255 255 255 255 0 ) ( 3672 -2960 268 12.187500 1.468750 255 255 255 255 0 ) ( 3672 -2960 456 12.187500 2.937500 255 255 255 255 0 ) )
( ( 3408 -2960 79.999985 14.250000 0 255 255 255 255 0 ) ( 3408 -2960 268 14.250000 1.468750 255 255 255 255 0 ) ( 3408 -2960 456 14.250000 2.937500 255 255 255 255 0 ) )
( ( 3408 -3216 79.999985 16.250000 0 255 255 255 255 0 ) ( 3408 -3216 268 16.250000 1.468750 255 255 255 255 0 ) ( 3408 -3216 456 16.250000 2.937500 255 255 255 255 0 ) )
)
}
}
}"#;

    const PATCH_DEF5_9X3_SLOPED: &str = r#"{
"classname" "worldspawn"
{
patchDef5
{
 battleship/metal@grey
 ( 9 3 0 0 0 0 8 )
(
( ( 2641.999756 -5850 356 0 0 255 255 255 255 0 ) ( 2641.999756 -5850 445 0 0.679688 255 255 255 255 0 ) ( 2641.999756 -5850 534 0 1.359375 255 255 255 255 0 ) )
( ( 2641.999756 -5926 356 0.593750 0 255 255 255 255 0 ) ( 2641.999756 -5926 445 0.593750 0.679688 255 255 255 255 0 ) ( 2641.999756 -5926 534 0.593750 1.359375 255 255 255 255 0 ) )
( ( 2724 -5926 356 1.234377 0 255 255 255 255 0 ) ( 2724 -5926 445 1.234377 0.679688 255 255 255 255 0 ) ( 2724 -5926 534 1.234377 1.359375 255 255 255 255 0 ) )
( ( 2806 -5926 356 1.875002 0 255 255 255 255 0 ) ( 2806 -5926 445 1.875002 0.679688 255 255 255 255 0 ) ( 2806 -5926 534 1.875002 1.359375 255 255 255 255 0 ) )
( ( 2806 -5850 356 2.468752 0 255 255 255 255 0 ) ( 2806 -5850 445 2.468752 0.679688 255 255 255 255 0 ) ( 2806 -5850 534 2.468752 1.359375 255 255 255 255 0 ) )
( ( 2806 -5774 356 3.062502 0 255 255 255 255 0 ) ( 2806 -5774 445 3.062502 0.679688 255 255 255 255 0 ) ( 2806 -5774 534 3.062502 1.359375 255 255 255 255 0 ) )
( ( 2724 -5774 356 3.703127 0 255 255 255 255 0 ) ( 2724 -5774 445 3.703127 0.679688 255 255 255 255 0 ) ( 2724 -5774 534 3.703127 1.359375 255 255 255 255 0 ) )
( ( 2641.999756 -5774 356 4.343754 0 255 255 255 255 0 ) ( 2641.999756 -5774 445 4.343754 0.679688 255 255 255 255 0 ) ( 2641.999756 -5774 534 4.343754 1.359375 255 255 255 255 0 ) )
( ( 2641.999756 -5850 356 4.937504 0 255 255 255 255 0 ) ( 2641.999756 -5850 445 4.937504 0.679688 255 255 255 255 0 ) ( 2641.999756 -5850 534 4.937504 1.359375 255 255 255 255 0 ) )
)
}
}
}"#;

    const PATCH_TERRAIN_15X15: &str = r#"{
"classname" "worldspawn"
{
patchTerrainDef3
{
 belgium/ground/rock@rubble_2asnow
 ( 15 15 134217728 0 0 0 8 )
(
( ( 4584 -424 70.118874 0 0 255 255 255 255 0 ) ( 4584 -406 56.920891 0 0.174376 255 255 255 255 0 ) ( 4584 -388 49.486500 0 0.326523 255 255 255 255 0 ) ( 4584 -370 52.082932 0 0.468604 255 255 255 255 0 ) ( 4584 -352 59.483105 0 0.620649 255 255 255 255 0 ) ( 4584 -334 67.515610 0 0.774641 255 255 255 255 0 ) ( 4584 -316 67.709694 0 0.915274 255 255 255 255 0 ) ( 4584 -298 51.943775 0 1.102214 255 255 255 255 0 ) ( 4584 -280 36.525829 0 1.287374 255 255 255 255 0 ) ( 4584 -262 37.414574 0 1.428170 255 255 255 255 0 ) ( 4584 -244 43.037491 0 1.575497 255 255 255 255 0 ) ( 4584 -226 44.057770 0 1.716348 255 255 255 255 0 ) ( 4584 -208 51.586636 0 1.868778 255 255 255 255 0 ) ( 4584 -190 58.534760 0 2.019516 255 255 255 255 0 ) ( 4584 -172 67.205544 0 2.175607 255 255 255 255 0 ) )
( ( 4603 -424 56.036259 0.184765 0 255 255 255 255 0 ) ( 4603 -406 34.085236 0.184765 0.174376 255 255 255 255 0 ) ( 4603 -388 15.757007 0.184765 0.326523 255 255 255 255 0 ) ( 4603 -370 9.189809 0.184765 0.468604 255 255 255 255 0 ) ( 4603 -352 10.272140 0.184765 0.620649 255 255 255 255 0 ) ( 4603 -334 13.086275 0.184765 0.774641 255 255 255 255 0 ) ( 4603 -316 11.509486 0.184765 0.915274 255 255 255 255 0 ) ( 4603 -298 4.157465 0.184765 1.102214 255 255 255 255 0 ) ( 4603 -280 3.693922 0.184765 1.287374 255 255 255 255 0 ) ( 4603 -262 17.539291 0.184765 1.428170 255 255 255 255 0 ) ( 4603 -244 26.352638 0.184765 1.575497 255 255 255 255 0 ) ( 4603 -226 26.258898 0.184765 1.716348 255 255 255 255 0 ) ( 4603 -208 35.060123 0.184765 1.868778 255 255 255 255 0 ) ( 4603 -190 43.934212 0.184765 2.019516 255 255 255 255 0 ) ( 4603 -172 55.286213 0.184765 2.175607 255 255 255 255 0 ) )
( ( 4622 -424 53.915836 0.334124 0 255 255 255 255 0 ) ( 4622 -406 35.992481 0.334124 0.174376 255 255 255 255 0 ) ( 4622 -388 18.123352 0.334124 0.326523 255 255 255 255 0 ) ( 4622 -370 4.228700 0.334124 0.468604 255 255 255 255 0 ) ( 4622 -352 -4.176507 0.334124 0.620649 255 255 255 255 0 ) ( 4622 -334 -7.904556 0.334124 0.774641 255 255 255 255 0 ) ( 4622 -316 -9.849315 0.334124 0.915274 255 255 255 255 0 ) ( 4622 -298 -10.715615 0.334124 1.102214 255 255 255 255 0 ) ( 4622 -280 -4.391700 0.334124 1.287374 255 255 255 255 0 ) ( 4622 -262 11.922306 0.334124 1.428170 255 255 255 255 0 ) ( 4622 -244 24.018579 0.334124 1.575497 255 255 255 255 0 ) ( 4622 -226 23.560001 0.334124 1.716348 255 255 255 255 0 ) ( 4622 -208 26.528625 0.334124 1.868778 255 255 255 255 0 ) ( 4622 -190 31.275501 0.334124 2.019516 255 255 255 255 0 ) ( 4622 -172 46.176506 0.334124 2.175607 255 255 255 255 0 ) )
( ( 4641 -424 71.544624 0.536613 0 255 255 255 255 0 ) ( 4641 -406 60.384964 0.536613 0.174376 255 255 255 255 0 ) ( 4641 -388 42.751778 0.536613 0.326523 255 255 255 255 0 ) ( 4641 -370 28.691113 0.536613 0.468604 255 255 255 255 0 ) ( 4641 -352 22.670166 0.536613 0.620649 255 255 255 255 0 ) ( 4641 -334 19.209177 0.536613 0.774641 255 255 255 255 0 ) ( 4641 -316 16.739542 0.536613 0.915274 255 255 255 255 0 ) ( 4641 -298 13.936602 0.536613 1.102214 255 255 255 255 0 ) ( 4641 -280 17.903004 0.536613 1.287374 255 255 255 255 0 ) ( 4641 -262 35.587353 0.536613 1.428170 255 255 255 255 0 ) ( 4641 -244 47.241219 0.536613 1.575497 255 255 255 255 0 ) ( 4641 -226 36.806225 0.536613 1.716348 255 255 255 255 0 ) ( 4641 -208 25.390171 0.536613 1.868778 255 255 255 255 0 ) ( 4641 -190 23.889011 0.536613 2.019516 255 255 255 255 0 ) ( 4641 -172 41.100063 0.536613 2.175607 255 255 255 255 0 ) )
( ( 4660 -424 104.304977 0.832483 0 255 255 255 255 0 ) ( 4660 -406 103.874023 0.832483 0.174376 255 255 255 255 0 ) ( 4660 -388 90.192787 0.832483 0.326523 255 255 255 255 0 ) ( 4660 -370 78.211029 0.832483 0.468604 255 255 255 255 0 ) ( 4660 -352 68.022606 0.832483 0.620649 255 255 255 255 0 ) ( 4660 -334 57.429909 0.832483 0.774641 255 255 255 255 0 ) ( 4660 -316 56.389309 0.832483 0.915274 255 255 255 255 0 ) ( 4660 -298 61.261532 0.832483 1.102214 255 255 255 255 0 ) ( 4660 -280 67.024178 0.832483 1.287374 255 255 255 255 0 ) ( 4660 -262 80.737610 0.832483 1.428170 255 255 255 255 0 ) ( 4660 -244 78.376236 0.832483 1.575497 255 255 255 255 0 ) ( 4660 -226 47.702309 0.832483 1.716348 255 255 255 255 0 ) ( 4660 -208 24.451889 0.832483 1.868778 255 255 255 255 0 ) ( 4660 -190 21.009628 0.832483 2.019516 255 255 255 255 0 ) ( 4660 -172 37.647449 0.832483 2.175607 255 255 255 255 0 ) )
( ( 4679 -424 121.216965 1.031206 0 255 255 255 255 0 ) ( 4679 -406 132.395645 1.031206 0.174376 255 255 255 255 0 ) ( 4679 -388 134.476822 1.031206 0.326523 255 255 255 255 0 ) ( 4679 -370 132.820358 1.031206 0.468604 255 255 255 255 0 ) ( 4679 -352 123.889099 1.031206 0.620649 255 255 255 255 0 ) ( 4679 -334 112.199890 1.031206 0.774641 255 255 255 255 0 ) ( 4679 -316 111.838280 1.031206 0.915274 255 255 255 255 0 ) ( 4679 -298 113.573761 1.031206 1.102214 255 255 255 255 0 ) ( 4679 -280 109.333633 1.031206 1.287374 255 255 255 255 0 ) ( 4679 -262 107.006607 1.031206 1.428170 255 255 255 255 0 ) ( 4679 -244 92.949257 1.031206 1.575497 255 255 255 255 0 ) ( 4679 -226 55.496510 1.031206 1.716348 255 255 255 255 0 ) ( 4679 -208 28.079382 1.031206 1.868778 255 255 255 255 0 ) ( 4679 -190 23.056753 1.031206 2.019516 255 255 255 255 0 ) ( 4679 -172 37.069592 1.031206 2.175607 255 255 255 255 0 ) )
( ( 4698 -424 122.168457 1.179829 0 255 255 255 255 0 ) ( 4698 -406 134.456802 1.179829 0.174376 255 255 255 255 0 ) ( 4698 -388 140.327347 1.179829 0.326523 255 255 255 255 0 ) ( 4698 -370 140.179718 1.179829 0.468604 255 255 255 255 0 ) ( 4698 -352 138.087326 1.179829 0.620649 255 255 255 255 0 ) ( 4698 -334 140.642944 1.179829 0.774641 255 255 255 255 0 ) ( 4698 -316 144.253220 1.179829 0.915274 255 255 255 255 0 ) ( 4698 -298 138.172440 1.179829 1.102214 255 255 255 255 0 ) ( 4698 -280 126.320206 1.179829 1.287374 255 255 255 255 0 ) ( 4698 -262 117.816429 1.179829 1.428170 255 255 255 255 0 ) ( 4698 -244 102.723732 1.179829 1.575497 255 255 255 255 0 ) ( 4698 -226 67.296669 1.179829 1.716348 255 255 255 255 0 ) ( 4698 -208 37.221786 1.179829 1.868778 255 255 255 255 0 ) ( 4698 -190 27.374844 1.179829 2.019516 255 255 255 255 0 ) ( 4698 -172 36.051704 1.179829 2.175607 255 255 255 255 0 ) )
( ( 4717 -424 120.396370 1.328911 0 255 255 255 255 0 ) ( 4717 -406 131.850220 1.328911 0.174376 255 255 255 255 0 ) ( 4717 -388 137.259033 1.328911 0.326523 255 255 255 255 0 ) ( 4717 -370 134.567902 1.328911 0.468604 255 255 255 255 0 ) ( 4717 -352 133.028778 1.328911 0.620649 255 255 255 255 0 ) ( 4717 -334 141.299469 1.328911 0.774641 255 255 255 255 0 ) ( 4717 -316 145.977707 1.328911 0.915274 255 255 255 255 0 ) ( 4717 -298 140.951981 1.328911 1.102214 255 255 255 255 0 ) ( 4717 -280 134.748108 1.328911 1.287374 255 255 255 255 0 ) ( 4717 -262 128.206146 1.328911 1.428170 255 255 255 255 0 ) ( 4717 -244 111.813065 1.328911 1.575497 255 255 255 255 0 ) ( 4717 -226 79.662216 1.328911 1.716348 255 255 255 255 0 ) ( 4717 -208 50.321110 1.328911 1.868778 255 255 255 255 0 ) ( 4717 -190 35.635342 1.328911 2.019516 255 255 255 255 0 ) ( 4717 -172 37.874733 1.328911 2.175607 255 255 255 255 0 ) )
( ( 4736 -424 115.330383 1.482534 0 255 255 255 255 0 ) ( 4736 -406 127.559982 1.482534 0.174376 255 255 255 255 0 ) ( 4736 -388 139.065216 1.482534 0.326523 255 255 255 255 0 ) ( 4736 -370 146.958603 1.482534 0.468604 255 255 255 255 0 ) ( 4736 -352 150.696808 1.482534 0.620649 255 255 255 255 0 ) ( 4736 -334 152.392181 1.482534 0.774641 255 255 255 255 0 ) ( 4736 -316 150.780457 1.482534 0.915274 255 255 255 255 0 ) ( 4736 -298 141.503510 1.482534 1.102214 255 255 255 255 0 ) ( 4736 -280 135.541458 1.482534 1.287374 255 255 255 255 0 ) ( 4736 -262 131.798767 1.482534 1.428170 255 255 255 255 0 ) ( 4736 -244 117.426964 1.482534 1.575497 255 255 255 255 0 ) ( 4736 -226 90.658150 1.482534 1.716348 255 255 255 255 0 ) ( 4736 -208 64.380257 1.482534 1.868778 255 255 255 255 0 ) ( 4736 -190 49.624584 1.482534 2.019516 255 255 255 255 0 ) ( 4736 -172 49.190121 1.482534 2.175607 255 255 255 255 0 ) )
( ( 4755 -424 109.142357 1.638646 0 255 255 255 255 0 ) ( 4755 -406 123.559052 1.638646 0.174376 255 255 255 255 0 ) ( 4755 -388 141.161896 1.638646 0.326523 255 255 255 255 0 ) ( 4755 -370 157.568726 1.638646 0.468604 255 255 255 255 0 ) ( 4755 -352 163.929871 1.638646 0.620649 255 255 255 255 0 ) ( 4755 -334 163.072266 1.638646 0.774641 255 255 255 255 0 ) ( 4755 -316 157.628143 1.638646 0.915274 255 255 255 255 0 ) ( 4755 -298 147.204529 1.638646 1.102214 255 255 255 255 0 ) ( 4755 -280 141.173019 1.638646 1.287374 255 255 255 255 0 ) ( 4755 -262 137.550888 1.638646 1.428170 255 255 255 255 0 ) ( 4755 -244 125.024590 1.638646 1.575497 255 255 255 255 0 ) ( 4755 -226 101.986847 1.638646 1.716348 255 255 255 255 0 ) ( 4755 -208 76.165215 1.638646 1.868778 255 255 255 255 0 ) ( 4755 -190 61.882046 1.638646 2.019516 255 255 255 255 0 ) ( 4755 -172 61.595734 1.638646 2.175607 255 255 255 255 0 ) )
( ( 4774 -424 105.353874 1.790006 0 255 255 255 255 0 ) ( 4774 -406 122.937622 1.790006 0.174376 255 255 255 255 0 ) ( 4774 -388 142.626709 1.790006 0.326523 255 255 255 255 0 ) ( 4774 -370 155.101807 1.790006 0.468604 255 255 255 255 0 ) ( 4774 -352 159.687790 1.790006 0.620649 255 255 255 255 0 ) ( 4774 -334 158.596161 1.790006 0.774641 255 255 255 255 0 ) ( 4774 -316 153.106064 1.790006 0.915274 255 255 255 255 0 ) ( 4774 -298 145.563858 1.790006 1.102214 255 255 255 255 0 ) ( 4774 -280 141.332306 1.790006 1.287374 255 255 255 255 0 ) ( 4774 -262 132.806229 1.790006 1.428170 255 255 255 255 0 ) ( 4774 -244 117.864693 1.790006 1.575497 255 255 255 255 0 ) ( 4774 -226 98.387260 1.790006 1.716348 255 255 255 255 0 ) ( 4774 -208 73.999802 1.790006 1.868778 255 255 255 255 0 ) ( 4774 -190 63.400288 1.790006 2.019516 255 255 255 255 0 ) ( 4774 -172 68.959145 1.790006 2.175607 255 255 255 255 0 ) )
( ( 4793 -424 93.317009 1.965724 0 255 255 255 255 0 ) ( 4793 -406 109.012848 1.965724 0.174376 255 255 255 255 0 ) ( 4793 -388 129.990005 1.965724 0.326523 255 255 255 255 0 ) ( 4793 -370 142.368118 1.965724 0.468604 255 255 255 255 0 ) ( 4793 -352 144.028580 1.965724 0.620649 255 255 255 255 0 ) ( 4793 -334 137.196487 1.965724 0.774641 255 255 255 255 0 ) ( 4793 -316 126.530586 1.965724 0.915274 255 255 255 255 0 ) ( 4793 -298 123.831123 1.965724 1.102214 255 255 255 255 0 ) ( 4793 -280 124.034027 1.965724 1.287374 255 255 255 255 0 ) ( 4793 -262 110.856636 1.965724 1.428170 255 255 255 255 0 ) ( 4793 -244 90.371719 1.965724 1.575497 255 255 255 255 0 ) ( 4793 -226 71.074921 1.965724 1.716348 255 255 255 255 0 ) ( 4793 -208 55.410763 1.965724 1.868778 255 255 255 255 0 ) ( 4793 -190 53.487888 1.965724 2.019516 255 255 255 255 0 ) ( 4793 -172 67.118896 1.965724 2.175607 255 255 255 255 0 ) )
( ( 4812 -424 75.210594 2.170769 0 255 255 255 255 0 ) ( 4812 -406 83.446625 2.170769 0.174376 255 255 255 255 0 ) ( 4812 -388 99.991814 2.170769 0.326523 255 255 255 255 0 ) ( 4812 -370 112.313553 2.170769 0.468604 255 255 255 255 0 ) ( 4812 -352 112.795860 2.170769 0.620649 255 255 255 255 0 ) ( 4812 -334 106.398575 2.170769 0.774641 255 255 255 255 0 ) ( 4812 -316 96.208214 2.170769 0.915274 255 255 255 255 0 ) ( 4812 -298 98.012459 2.170769 1.102214 255 255 255 255 0 ) ( 4812 -280 103.584747 2.170769 1.287374 255 255 255 255 0 ) ( 4812 -262 90.870499 2.170769 1.428170 255 255 255 255 0 ) ( 4812 -244 66.895454 2.170769 1.575497 255 255 255 255 0 ) ( 4812 -226 46.207001 2.170769 1.716348 255 255 255 255 0 ) ( 4812 -208 38.238770 2.170769 1.868778 255 255 255 255 0 ) ( 4812 -190 45.319973 2.170769 2.019516 255 255 255 255 0 ) ( 4812 -172 62.848015 2.170769 2.175607 255 255 255 255 0 ) )
( ( 4831 -424 68.025970 2.329464 0 255 255 255 255 0 ) ( 4831 -406 68.766739 2.329464 0.174376 255 255 255 255 0 ) ( 4831 -388 76.310966 2.329464 0.326523 255 255 255 255 0 ) ( 4831 -370 79.509148 2.329464 0.468604 255 255 255 255 0 ) ( 4831 -352 77.788811 2.329464 0.620649 255 255 255 255 0 ) ( 4831 -334 78.246788 2.329464 0.774641 255 255 255 255 0 ) ( 4831 -316 75.915405 2.329464 0.915274 255 255 255 255 0 ) ( 4831 -298 78.564720 2.329464 1.102214 255 255 255 255 0 ) ( 4831 -280 82.236641 2.329464 1.287374 255 255 255 255 0 ) ( 4831 -262 72.945313 2.329464 1.428170 255 255 255 255 0 ) ( 4831 -244 52.995037 2.329464 1.575497 255 255 255 255 0 ) ( 4831 -226 35.263130 2.329464 1.716348 255 255 255 255 0 ) ( 4831 -208 32.870571 2.329464 1.868778 255 255 255 255 0 ) ( 4831 -190 46.195911 2.329464 2.019516 255 255 255 255 0 ) ( 4831 -172 64.933739 2.329464 2.175607 255 255 255 255 0 ) )
( ( 4850 -424 66.268280 2.478536 0 255 255 255 255 0 ) ( 4850 -406 64.713951 2.478536 0.174376 255 255 255 255 0 ) ( 4850 -388 70.254967 2.478536 0.326523 255 255 255 255 0 ) ( 4850 -370 71.229332 2.478536 0.468604 255 255 255 255 0 ) ( 4850 -352 66.550385 2.478536 0.620649 255 255 255 255 0 ) ( 4850 -334 66.570229 2.478536 0.774641 255 255 255 255 0 ) ( 4850 -316 68.236603 2.478536 0.915274 255 255 255 255 0 ) ( 4850 -298 72.170837 2.478536 1.102214 255 255 255 255 0 ) ( 4850 -280 74.278900 2.478536 1.287374 255 255 255 255 0 ) ( 4850 -262 68.074120 2.478536 1.428170 255 255 255 255 0 ) ( 4850 -244 58.547871 2.478536 1.575497 255 255 255 255 0 ) ( 4850 -226 48.638390 2.478536 1.716348 255 255 255 255 0 ) ( 4850 -208 48.272411 2.478536 1.868778 255 255 255 255 0 ) ( 4850 -190 60.021736 2.478536 2.019516 255 255 255 255 0 ) ( 4850 -172 75.063469 2.478536 2.175607 255 255 255 255 0 ) )
)
}
}
}"#;

    // --- Parser tests for real patch data ---

    #[test]
    fn parses_real_patch_def5_9x3_flat() {
        let map = parse_map_string(PATCH_DEF5_9X3).unwrap();
        let brush = &map.entities[0].brushes[0];
        match &brush.content {
            BrushContent::Patch(p) => {
                assert_eq!(p.patch_type, PatchType::Curve);
                assert_eq!(p.texture, "battleship/metal@grey");
                assert_eq!(p.params.rows, 9);
                assert_eq!(p.params.cols, 3);
                assert_eq!(p.params.subdivision, 8);
                assert_eq!(p.vertices.len(), 9);
                assert_eq!(p.vertices[0].len(), 3);
                // First vertex position: (3408, -3216, ~80)
                let v00 = p.vertices[0][0].position;
                assert!((v00.x - 3408.0).abs() < 0.01);
                assert!((v00.y - (-3216.0)).abs() < 0.01);
                assert!((v00.z - 80.0).abs() < 1.0);
                // UVs: row 0, col 0 -> (0, 0), row 0, col 2 -> (0, 2.9375)
                assert!((p.vertices[0][0].uv.x - 0.0).abs() < 0.01);
                assert!((p.vertices[0][2].uv.y - 2.9375).abs() < 0.01);
                // Corner vertex: (3408, -3216, 456)
                let v02 = p.vertices[0][2].position;
                assert!((v02.z - 456.0).abs() < 1.0);
            }
            _ => panic!("Expected Patch brush"),
        }
    }

    #[test]
    fn parses_real_patch_def5_9x3_sloped() {
        let map = parse_map_string(PATCH_DEF5_9X3_SLOPED).unwrap();
        let brush = &map.entities[0].brushes[0];
        match &brush.content {
            BrushContent::Patch(p) => {
                assert_eq!(p.patch_type, PatchType::Curve);
                assert_eq!(p.texture, "battleship/metal@grey");
                assert_eq!(p.params.rows, 9);
                assert_eq!(p.params.cols, 3);
                assert_eq!(p.vertices.len(), 9);
                assert_eq!(p.vertices[0].len(), 3);
                // Verify fractional coordinates parsed correctly
                let v00 = p.vertices[0][0].position;
                assert!((v00.x - 2641.999756).abs() < 0.01);
                // Verify Z varies across rows (sloped)
                let z_row0 = p.vertices[0][1].position.z;
                let z_row4 = p.vertices[4][1].position.z;
                assert!((z_row0 - 445.0).abs() < 1.0);
                assert!((z_row4 - 445.0).abs() < 1.0);
                // Col 2 varies (sloped Z)
                let z_col2_r0 = p.vertices[0][2].position.z;
                let z_col2_r8 = p.vertices[8][2].position.z;
                assert!((z_col2_r0 - 534.0).abs() < 1.0);
                assert!((z_col2_r8 - 534.0).abs() < 1.0);
            }
            _ => panic!("Expected Patch brush"),
        }
    }

    #[test]
    fn parses_real_terrain_15x15() {
        let map = parse_map_string(PATCH_TERRAIN_15X15).unwrap();
        let brush = &map.entities[0].brushes[0];
        match &brush.content {
            BrushContent::Patch(p) => {
                assert_eq!(p.patch_type, PatchType::Terrain);
                assert_eq!(p.texture, "belgium/ground/rock@rubble_2asnow");
                assert_eq!(p.params.rows, 15);
                assert_eq!(p.params.cols, 15);
                assert_eq!(p.params.contents, 134217728);
                assert_eq!(p.params.subdivision, 8);
                assert_eq!(p.vertices.len(), 15);
                for (ri, row) in p.vertices.iter().enumerate() {
                    assert_eq!(row.len(), 15, "row {ri} should have 15 cols");
                }
                // First vertex: (4584, -424, ~70.12)
                let v00 = p.vertices[0][0].position;
                assert!((v00.x - 4584.0).abs() < 0.1);
                assert!((v00.y - (-424.0)).abs() < 0.1);
                assert!((v00.z - 70.118874).abs() < 0.1);
                // Row 2, col 4: Z ~ -4.18 (negative elevation)
                let v24 = p.vertices[2][4].position;
                assert!((v24.z - (-4.176507)).abs() < 0.1);
                // Last vertex: (4850, -172, ~75.06)
                let v1414 = p.vertices[14][14].position;
                assert!((v1414.x - 4850.0).abs() < 0.1);
                assert!((v1414.z - 75.063469).abs() < 0.1);
            }
            _ => panic!("Expected Patch brush"),
        }
    }

    #[test]
    fn roundtrip_real_patch_def5_9x3() {
        let map = parse_map_string(PATCH_DEF5_9X3).unwrap();
        let serialized = map.to_map_string();
        let map2 = parse_map_string(&serialized).unwrap();
        let brush1 = &map.entities[0].brushes[0];
        let brush2 = &map2.entities[0].brushes[0];
        match (&brush1.content, &brush2.content) {
            (BrushContent::Patch(p1), BrushContent::Patch(p2)) => {
                assert_eq!(p1.patch_type, p2.patch_type);
                assert_eq!(p1.texture, p2.texture);
                assert_eq!(p1.params.rows, p2.params.rows);
                assert_eq!(p1.params.cols, p2.params.cols);
                assert_eq!(p1.vertices.len(), p2.vertices.len());
                for (r, (row1, row2)) in p1.vertices.iter().zip(p2.vertices.iter()).enumerate() {
                    for (c, (v1, v2)) in row1.iter().zip(row2.iter()).enumerate() {
                        assert!(
                            (v1.position - v2.position).length() < 0.01,
                            "vertex[{r}][{c}] position mismatch"
                        );
                        assert!(
                            (v1.uv - v2.uv).length() < 0.01,
                            "vertex[{r}][{c}] uv mismatch"
                        );
                    }
                }
            }
            _ => panic!("Expected Patch brushes on both sides"),
        }
    }

    #[test]
    fn roundtrip_real_terrain_15x15() {
        let map = parse_map_string(PATCH_TERRAIN_15X15).unwrap();
        let serialized = map.to_map_string();
        let map2 = parse_map_string(&serialized).unwrap();
        let brush1 = &map.entities[0].brushes[0];
        let brush2 = &map2.entities[0].brushes[0];
        match (&brush1.content, &brush2.content) {
            (BrushContent::Patch(p1), BrushContent::Patch(p2)) => {
                assert_eq!(p1.patch_type, PatchType::Terrain);
                assert_eq!(p2.patch_type, PatchType::Terrain);
                assert_eq!(p1.vertices.len(), 15);
                assert_eq!(p2.vertices.len(), 15);
                for r in 0..15 {
                    for c in 0..15 {
                        let dist =
                            (p1.vertices[r][c].position - p2.vertices[r][c].position).length();
                        assert!(
                            dist < 0.01,
                            "terrain vertex[{r}][{c}] roundtrip mismatch: {dist}"
                        );
                    }
                }
            }
            _ => panic!("Expected Patch brushes"),
        }
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

        assert_eq!(aabb.min, Vec3::new(16.0, -76.0, 0.0));
        assert_eq!(aabb.max, Vec3::new(68.0, 0.0, 128.0));

        assert_eq!(aabb.max.x - aabb.min.x, 52.0);
        assert_eq!(aabb.max.y - aabb.min.y, 76.0);
        assert_eq!(aabb.max.z - aabb.min.z, 128.0);
    }
}
