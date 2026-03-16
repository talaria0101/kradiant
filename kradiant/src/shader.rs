//! Minimal shader database and `.shader` file parser focused on editor-facing metadata.
//!
//! It extracts `qer_*` properties (such as editor and light images) into a lightweight
//! [`ShaderDb`] that can be queried by material name, without interpreting full rendering stages.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
}

#[derive(Debug, Default)]
pub struct ShaderDb {
    // keys are normalized to lowercase with forward slashes.
    by_name: HashMap<String, ShaderDef>,
}

impl ShaderDb {
    pub fn get(&self, name: &str) -> Option<&ShaderDef> {
        self.by_name.get(&normalize_name(name))
    }

    pub fn iter(&self) -> impl Iterator<Item = &ShaderDef> {
        self.by_name.values()
    }

    pub fn insert(&mut self, shader: ShaderDef) {
        self.by_name.insert(normalize_name(&shader.name), shader);
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

#[derive(Debug, Error)]
pub enum ShaderError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("utf-8 decoding error in {path}: {source}")]
    Utf8 {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },

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
    let mut db = ShaderDb::default();

    if !scripts_dir.exists() {
        return Ok(db);
    }

    for entry in std::fs::read_dir(scripts_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("shader") {
            continue;
        }

        let bytes = std::fs::read(&path)?;
        let src = String::from_utf8(bytes).map_err(|e| ShaderError::Utf8 {
            path: path.clone(),
            source: e,
        })?;

        parse_shader_file_into_db(&src, &path, &mut db)?;
    }

    Ok(db)
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

fn parse_shader_file_into_db(src: &str, path: &Path, db: &mut ShaderDb) -> Result<(), ShaderError> {
    let tokens = tokenize(src);
    let mut i = 0usize;

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

        db.insert(ShaderDef {
            name: shader_name,
            qer,
        });
    }

    Ok(())
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
    parse_shader_file_into_db(src, path, db)
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
        let mut db = ShaderDb::default();
        parse_shader_file_into_db(src, Path::new("test.shader"), &mut db).unwrap();
        let sh = db.get("textures/common/caulk").unwrap();
        assert_eq!(
            sh.qer.editor_image.as_deref(),
            Some("textures/common/caulk.tga")
        );
        assert_eq!(sh.qer.trans, Some(0.35));
        assert!(sh.qer.no_draw);
    }
}
