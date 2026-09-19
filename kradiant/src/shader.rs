//! Minimal shader database and `.shader` file parser focused on editor-facing metadata.
//!
//! It extracts `qer_*` properties (such as editor and light images) into a lightweight
//! [`ShaderDb`] that can be queried by material name, without interpreting full rendering stages.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::iter::{ParallelBridge, ParallelIterator};
use thiserror::Error;

#[derive(Debug, Clone, Default)]
pub struct QerParams {
    pub editor_image: Option<String>,
    pub light_image: Option<String>,
    pub trans: Option<f32>,
    pub no_draw: bool,
    pub no_carve: bool,
    /// Any other `qer_*` directives we don't explicitly model yet.
    pub extra: HashMap<String, Option<String>>,
}

#[derive(Debug, Clone)]
pub struct ShaderDef {
    pub name: String,
    pub qer: QerParams,
    /// First editor-relevant diffuse image referenced by the shader, if any.
    pub diffuse_map: Option<String>,
}

#[derive(Debug, Default)]
pub struct ShaderDb {
    // keys are normalized to lowercase with forward slashes.
    by_name: HashMap<String, ShaderDef>,
}

impl ShaderDb {
    pub fn get(&self, name: &str) -> Option<&ShaderDef> {
        let key = normalize_name(name);
        if let Some(def) = self.by_name.get(&key) {
            return Some(def);
        }
        if !key.starts_with("textures/") {
            let prefixed = format!("textures/{key}");
            if let Some(def) = self.by_name.get(&prefixed) {
                return Some(def);
            }
        }
        None
    }

    pub fn iter(&self) -> impl Iterator<Item = &ShaderDef> {
        self.by_name.values()
    }

    pub fn insert(&mut self, shader: ShaderDef) {
        self.by_name.insert(normalize_name(&shader.name), shader);
    }

    pub fn extend<D: IntoIterator<Item = ShaderDef>>(&mut self, defs: D) {
        defs.into_iter().for_each(|d| {
            self.by_name.insert(normalize_name(&d.name), d);
        });
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

impl From<Vec<ShaderDef>> for ShaderDb {
    fn from(value: Vec<ShaderDef>) -> Self {
        let hmap: HashMap<String, ShaderDef> = value
            .into_iter()
            .map(|d| (normalize_name(&d.name), d))
            .collect();
        Self { by_name: hmap }
    }
}

impl From<ShaderDef> for ShaderDb {
    fn from(value: ShaderDef) -> Self {
        Self {
            by_name: [(normalize_name(&value.name), value)].into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ShaderError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("syntax error in {path}:{line}: {msg}")]
    Syntax {
        path: PathBuf,
        line: usize,
        msg: String,
    },

    #[error("invalid number in {path}:{line}: {value}")]
    InvalidNumber {
        path: PathBuf,
        line: usize,
        value: String,
    },
}

/// Load all `.shader` files from a `scripts` directory and extract `qer_*` parameters.
pub fn load_shader_db_from_scripts_dir(
    scripts_dir: impl AsRef<Path>,
) -> Result<ShaderDb, ShaderError> {
    let scripts_dir = scripts_dir.as_ref();

    if !scripts_dir.exists() {
        return Ok(ShaderDb::default());
    }

    let defs: Vec<ShaderDef> = std::fs::read_dir(scripts_dir)?
        .par_bridge()
        .filter_map(|e| {
            let entry = match e {
                Ok(e) => e,
                Err(e) => {
                    log::warn!("read_dir error: {e}");
                    return None;
                }
            };
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !ext.eq_ignore_ascii_case("shader") {
                return None;
            }

            let src = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("failed to read {}: {e}", path.display());
                    return None;
                }
            };

            match parse_shader_file(&src, &path) {
                Ok(v) => Some(v),
                Err(e) => {
                    log::warn!("failed to parse {}: {e}", path.display());
                    None
                }
            }
        })
        .flatten_iter()
        .collect();

    Ok(defs.into())
}

/// Convenience wrapper around [`load_shader_db_from_scripts_dir`] for a game root directory.
pub fn load_shader_db_from_main_dir(main_dir: impl AsRef<Path>) -> Result<ShaderDb, ShaderError> {
    load_shader_db_from_scripts_dir(main_dir.as_ref().join("scripts"))
}

#[derive(Debug, Clone)]
struct Tok {
    text: String,
    line: usize,
}

pub fn parse_shader_file(src: &str, path: &Path) -> Result<Vec<ShaderDef>, ShaderError> {
    let tokens = tokenize(src);
    let mut i = 0usize;
    let mut defs = Vec::new();

    while i < tokens.len() {
        // Skip stray braces/empties.
        if tokens[i].text == "{" || tokens[i].text == "}" {
            i += 1;
            continue;
        }

        let name_tok = &tokens[i];
        let shader_name = name_tok.text.clone();
        i += 1;

        let Some(open) = tokens.get(i) else {
            return Err(ShaderError::Syntax {
                path: path.to_path_buf(),
                line: name_tok.line,
                msg: "expected '{' after shader name".into(),
            });
        };
        if open.text != "{" {
            return Err(ShaderError::Syntax {
                path: path.to_path_buf(),
                line: open.line,
                msg: "expected '{' after shader name".into(),
            });
        }
        i += 1;

        let mut qer = QerParams::default();
        let mut diffuse_map: Option<String> = None;
        let mut depth: i32 = 1;

        while i < tokens.len() && depth > 0 {
            let t = &tokens[i];
            i += 1;

            match t.text.as_str() {
                "{" => {
                    depth += 1;
                    continue;
                }
                "}" => {
                    depth -= 1;
                    continue;
                }
                _ => {}
            }

            if diffuse_map.is_none() && depth > 0 {
                match t.text.to_ascii_lowercase().as_str() {
                    "map" | "clampmap" => {
                        if let Some(nxt) = tokens.get(i) {
                            if let Some(mapped) = normalize_diffuse_map_token(&nxt.text) {
                                diffuse_map = Some(mapped);
                            }
                            i += 1;
                        }
                    }
                    "animmap" => {
                        if tokens.get(i).is_some() {
                            i += 1;
                        }
                        if let Some(nxt) = tokens.get(i) {
                            if let Some(mapped) = normalize_diffuse_map_token(&nxt.text) {
                                diffuse_map = Some(mapped);
                            }
                            i += 1;
                        }
                    }
                    _ => {}
                }
            }

            if depth != 1 {
                continue;
            }

            if !t.text.starts_with("qer_") {
                continue;
            }

            let key = t.text.clone();
            let value = match tokens.get(i) {
                Some(nxt)
                    if nxt.text != "{" && nxt.text != "}" && !nxt.text.starts_with("qer_") =>
                {
                    i += 1;
                    Some(nxt.text.clone())
                }
                _ => None,
            };

            apply_qer(&mut qer, &key, value.clone(), path, t.line)?;
        }

        if depth != 0 {
            return Err(ShaderError::Syntax {
                path: path.to_path_buf(),
                line: name_tok.line,
                msg: "unterminated shader block".into(),
            });
        }

        defs.push(ShaderDef {
            name: shader_name,
            qer,
            diffuse_map,
        });
    }

    Ok(defs)
}

/// Parse a `.shader` source string and merge its `qer_*` parameters into an existing database.
///
/// This is useful when shader sources come from a virtual filesystem (for example a `.pk3`
/// archive) rather than the OS filesystem.
pub fn parse_shader_source_into_db(
    src: &str,
    path: &Path,
    db: &mut ShaderDb,
) -> Result<(), ShaderError> {
    db.extend(parse_shader_file(src, path)?);
    Ok(())
}

fn apply_qer(
    qer: &mut QerParams,
    key: &str,
    value: Option<String>,
    path: &Path,
    line: usize,
) -> Result<(), ShaderError> {
    match key {
        "qer_editorimage" => {
            qer.editor_image = value;
        }
        "qer_lightimage" => {
            qer.light_image = value;
        }
        "qer_trans" => {
            let Some(v) = value else {
                return Ok(());
            };
            let f: f32 = v.parse().map_err(|_| ShaderError::InvalidNumber {
                path: path.to_path_buf(),
                line,
                value: v,
            })?;
            qer.trans = Some(f);
        }
        "qer_nodraw" => qer.no_draw = true,
        "qer_nocarve" => qer.no_carve = true,
        _ => {
            qer.extra.insert(key.to_string(), value);
        }
    }
    Ok(())
}

fn normalize_name(s: &str) -> String {
    s.replace('\\', "/").to_ascii_lowercase()
}

fn normalize_diffuse_map_token(token: &str) -> Option<String> {
    let normalized = normalize_name(token);
    match normalized.as_str() {
        "$lightmap" | "$whiteimage" | "$nodraw" => None,
        _ => Some(normalized),
    }
}

fn tokenize(src: &str) -> Vec<Tok> {
    let bytes = src.as_bytes();
    let mut out = Vec::<Tok>::new();
    let mut i = 0usize;
    let mut line = 1usize;

    while i < bytes.len() {
        let b = bytes[i];

        // Newline tracking.
        if b == b'\n' {
            line += 1;
            i += 1;
            continue;
        }

        // Whitespace.
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Block comment.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() {
                if bytes[i] == b'\n' {
                    line += 1;
                }
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // Braces as single-character tokens.
        if b == b'{' || b == b'}' {
            out.push(Tok {
                text: (b as char).to_string(),
                line,
            });
            i += 1;
            continue;
        }

        // Quoted token.
        if b == b'"' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            let text = String::from_utf8_lossy(&bytes[start..i]).to_string();
            if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
            }
            out.push(Tok { text, line });
            continue;
        }

        // Normal token: read until whitespace or brace.
        let start = i;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'\n' || c.is_ascii_whitespace() || c == b'{' || c == b'}' {
                break;
            }
            // comment start terminates token
            if c == b'/' && i + 1 < bytes.len() && (bytes[i + 1] == b'/' || bytes[i + 1] == b'*') {
                break;
            }
            i += 1;
        }
        if i > start {
            let text = String::from_utf8_lossy(&bytes[start..i]).to_string();
            out.push(Tok { text, line });
        } else {
            i += 1;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qer_params() {
        let src = r#"
// comment
textures/common/caulk
{
    qer_editorimage textures/common/caulk.tga
    qer_trans 0.35
    qer_nodraw
    {
        map textures/common/caulk.tga
    }
}
"#;
        let db: ShaderDb = parse_shader_file(src, Path::new("test.shader"))
            .unwrap()
            .into();
        let sh = db.get("textures/common/caulk").unwrap();
        assert_eq!(
            sh.qer.editor_image.as_deref(),
            Some("textures/common/caulk.tga")
        );
        assert_eq!(sh.qer.trans, Some(0.35));
        assert!(sh.qer.no_draw);
        assert_eq!(sh.diffuse_map.as_deref(), Some("textures/common/caulk.tga"));
    }

    #[test]
    fn lookup_falls_back_to_textures_prefix() {
        let db: ShaderDb = parse_shader_file(
            "textures/common/trigger\n{\nqer_trans 0.5\n}",
            Path::new("test.shader"),
        )
        .unwrap()
        .into();
        // map faces store material without "textures/" prefix
        let sh = db.get("common/trigger").expect("lookup should fall back");
        assert_eq!(sh.qer.trans, Some(0.5));
        // exact key still works
        assert!(db.get("textures/common/trigger").is_some());
    }

    #[test]
    fn parses_stage_diffuse_map() {
        let src = r#"
skins/test/example
{
    {
        map $lightmap
    }
    {
        clampMap "Textures/Characters/Test_D"
    }
}
"#;
        let db: ShaderDb = parse_shader_file(src, Path::new("test.shader"))
            .unwrap()
            .into();
        let sh = db.get("skins/test/example").unwrap();
        assert_eq!(
            sh.diffuse_map.as_deref(),
            Some("textures/characters/test_d")
        );
    }
}
