//! Asset resolution and virtual filesystem helpers for `.map` content.
//!
//! This module is UI‑agnostic: it resolves asset paths, locates files on disk or in `.pk3`
//! archives, and decodes image bytes, but it does not create GPU resources. Callers are expected
//! to upload the returned RGBA8 data to their rendering backend of choice.

use crate::shader::{ShaderDb, parse_shader_source_into_db};
use crate::texture::{TextureError, TextureImage, decode_texture_rgba8, load_texture_rgba8};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use thiserror::Error;
use zip::ZipArchive;

/// Root folders backing a typical game "main" directory.
///
/// Convention:
/// - textures live in `(main)/textures/...`
/// - shader scripts live in `(main)/scripts/*.shader`
#[derive(Debug, Clone)]
pub struct AssetRoots {
    maindir: PathBuf,
    textures_dir: PathBuf,
    scripts_dir: PathBuf,
}

impl AssetRoots {
    pub fn new(maindir: impl Into<PathBuf>) -> Self {
        let maindir = maindir.into();
        Self {
            textures_dir: maindir.join("textures"),
            scripts_dir: maindir.join("scripts"),
            maindir,
        }
    }

    pub fn maindir(&self) -> &Path {
        &self.maindir
    }

    pub fn textures_dir(&self) -> &Path {
        &self.textures_dir
    }

    pub fn scripts_dir(&self) -> &Path {
        &self.scripts_dir
    }

    /// Resolve a `.map`/shader material name to an on-disk texture path.
    ///
    /// Notes:
    /// - Map material names usually omit the file extension.
    /// - We try a preferred extension order (`dds`, `tga`, `jpg`, `jpeg`).
    /// - We look under `(main)/textures/<name>` first, then fall back to `(main)/<name>` in case
    ///   the material already includes a `textures/...` prefix.
    pub fn resolve_texture_path(&self, material: &str) -> Option<PathBuf> {
        let rel = normalize_material_name(material);
        resolve_texture_path_under(&self.textures_dir, &rel)
            .or_else(|| resolve_texture_path_under(&self.maindir, &rel))
    }

    /// Resolve and decode a texture into RGBA8 for uploading to OpenGL.
    pub fn load_texture_rgba8(&self, material: &str) -> Result<TextureImage, TextureError> {
        let path = self
            .resolve_texture_path(material)
            .ok_or_else(|| TextureError::Decode(format!("texture not found: {material}")))?;
        load_texture_rgba8(path)
    }
}

/// Normalize a material name for filesystem lookup: forward slashes, no surrounding whitespace.
pub fn normalize_material_name(name: &str) -> String {
    name.trim().replace('\\', "/")
}

/// Canonicalize an asset path for zip/by-name lookup:
/// - trims whitespace
/// - forward slashes
/// - strips leading `/` and leading `./`
///
/// Note: this intentionally does *not* lowercase. Lowercasing is used for keys/indexing, but zip
/// entry names are case-sensitive.
fn canonical_virtual_path(path: &str) -> String {
    let mut p = path.trim().replace('\\', "/");
    while p.starts_with("./") {
        p = p[2..].to_string();
    }
    while p.starts_with('/') {
        p = p[1..].to_string();
    }
    p
}

/// Radiant shows shader surfaces using `qer_editorimage` when present.
/// If `shader_db` is missing or does not contain `material`, returns `material` (normalized).
pub fn resolve_editor_image_name(material: &str, shader_db: Option<&ShaderDb>) -> String {
    let name = normalize_material_name(material);
    let Some(db) = shader_db else {
        return name;
    };
    let Some(sh) = db.get(&name) else {
        return name;
    };
    sh.qer
        .editor_image
        .as_deref()
        .map(normalize_material_name)
        .unwrap_or(name)
}

/// Where an asset came from (loose file or inside a `.pk3`).
#[derive(Debug, Clone)]
pub enum ResolvedAsset {
    Loose(PathBuf),
    Pk3 {
        pk3_path: PathBuf,
        entry_name: String,
        virtual_path: String,
    },
}

#[derive(Debug, Error)]
pub enum AssetDbError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("utf-8 decoding error in {path}: {source}")]
    Utf8 {
        path: String,
        #[source]
        source: std::string::FromUtf8Error,
    },

    #[error("asset not found: {0}")]
    NotFound(String),

    #[error("texture error: {0}")]
    Texture(#[from] TextureError),

    #[error("shader error: {0}")]
    Shader(#[from] crate::shader::ShaderError),
}

#[derive(Debug, Clone)]
struct Pk3EntryRef {
    pk3_path: PathBuf,
    entry_name: String,
}

/// Asset database for a game content directory.
///
/// - Prefers loose files on disk.
/// - Falls back to `.pk3` archives found directly under `<maindir>`.
/// - Indexes `.pk3` contents once at startup for fast existence checks and resolution.
#[derive(Debug)]
pub struct AssetDb {
    roots: AssetRoots,
    pk3_paths: Vec<PathBuf>,
    // key is normalized (lowercase, forward slashes, no leading slash)
    pk3_index: HashMap<String, Pk3EntryRef>,
    // Open pk3 archives, so repeated reads don't re-parse the central directory.
    pk3_open: HashMap<PathBuf, ZipArchive<File>>,
}

#[derive(Debug, Clone)]
pub struct AssetDbOptions {
    /// If false (default), only index useful filetypes (textures + shader scripts) for faster startup.
    pub full_index: bool,
    /// Index texture images under `textures/` inside `.pk3` archives.
    ///
    /// If false, texture resolution will fall back to a slower zip lookup on-demand.
    pub index_textures: bool,
    /// Index shader scripts under `scripts/` inside `.pk3` archives.
    pub index_shaders: bool,
}

impl Default for AssetDbOptions {
    fn default() -> Self {
        // Default tuned for fast startup:
        // - do not pre-index pk3 contents (can be extremely large)
        // - resolve assets on-demand (and allow frontends to prefetch only what they need)
        Self {
            full_index: false,
            index_textures: false,
            index_shaders: false,
        }
    }
}

impl AssetDb {
    /// Build a `.pk3` index by scanning `<maindir>` for archives.
    pub fn from_maindir(maindir: impl Into<PathBuf>) -> Result<Self, AssetDbError> {
        Self::from_maindir_with_options(maindir, AssetDbOptions::default())
    }

    /// Same as [`AssetDb::from_maindir`], but configurable.
    pub fn from_maindir_with_options(
        maindir: impl Into<PathBuf>,
        opts: AssetDbOptions,
    ) -> Result<Self, AssetDbError> {
        let roots = AssetRoots::new(maindir);
        let mut pk3_paths = find_pk3_files(roots.maindir())?;
        pk3_paths.sort();

        // Build index in ascending order so later filenames (e.g. pak1.pk3) override earlier (pak0.pk3).
        let mut pk3_index: HashMap<String, Pk3EntryRef> = HashMap::new();
        if opts.full_index || opts.index_shaders || opts.index_textures {
            for pk3_path in &pk3_paths {
                let f = File::open(pk3_path)?;
                let zip = ZipArchive::new(f)?;

                // Fast path: avoid indexing everything. For editor preview we only need:
                // - textures: textures/**.(dds|tga|jpg|jpeg)
                // - shader scripts: scripts/*.shader
                //
                // Full indexing is available for future expansion (models, sounds, etc.).
                for name in zip.file_names() {
                    if !opts.full_index && !is_useful_index_entry_raw(name, &opts) {
                        continue;
                    }

                    let key = normalize_asset_path(name);

                    // Keep the last one (highest priority) when duplicates exist.
                    pk3_index.insert(
                        key,
                        Pk3EntryRef {
                            pk3_path: pk3_path.clone(),
                            entry_name: name.to_string(),
                        },
                    );
                }
            }
        }

        Ok(Self {
            roots,
            pk3_paths,
            pk3_index,
            pk3_open: HashMap::new(),
        })
    }

    pub fn file_useful_for_radiant(ext: &str) -> bool {
        matches!(
            ext.trim()
                .trim_start_matches('.')
                .to_ascii_lowercase()
                .as_str(),
            "jpg" | "tga" | "dds" | "shader" | "shadertype"
        )
    }

    pub fn roots(&self) -> &AssetRoots {
        &self.roots
    }

    pub fn pk3_paths(&self) -> &[PathBuf] {
        &self.pk3_paths
    }

    fn open_pk3_cached(&mut self, pk3_path: &Path) -> Result<&mut ZipArchive<File>, AssetDbError> {
        use std::collections::hash_map::Entry;

        match self.pk3_open.entry(pk3_path.to_path_buf()) {
            Entry::Occupied(o) => Ok(o.into_mut()),
            Entry::Vacant(v) => {
                let f = File::open(pk3_path)?;
                let zip = ZipArchive::new(f)?;
                Ok(v.insert(zip))
            }
        }
    }

    /// Iterate normalized virtual paths provided by indexed `.pk3` files.
    pub fn iter_pk3_virtual_paths(&self) -> impl Iterator<Item = &str> {
        self.pk3_index.keys().map(|s| s.as_str())
    }

    /// Resolve an arbitrary virtual path (e.g. `scripts/common.shader`, `textures/common/caulk.tga`)
    /// to either a loose file or a `.pk3` entry.
    ///
    /// Loose files have priority.
    pub fn resolve_virtual_path(&mut self, virtual_path: &str) -> Option<ResolvedAsset> {
        let v_zip = canonical_virtual_path(virtual_path);
        let v_key = normalize_asset_path(&v_zip);
        let loose = self.roots.maindir.join(&v_zip);
        if loose.is_file() {
            return Some(ResolvedAsset::Loose(loose));
        }
        if v_key != v_zip {
            let loose2 = self.roots.maindir.join(&v_key);
            if loose2.is_file() {
                return Some(ResolvedAsset::Loose(loose2));
            }
        }

        if let Some(pk3) = self.pk3_index.get(&v_key) {
            return Some(ResolvedAsset::Pk3 {
                pk3_path: pk3.pk3_path.clone(),
                entry_name: pk3.entry_name.clone(),
                virtual_path: v_key,
            });
        }

        // Slow path (on-demand): search `.pk3` archives from highest priority to lowest.
        // When found, memoize into the index so subsequent lookups are fast.
        let pk3s: Vec<PathBuf> = self.pk3_paths.iter().cloned().collect();
        for pk3_path in pk3s.iter().rev() {
            // Try exact/canonical name first (fast).
            let has_exact = {
                let zip = self.open_pk3_cached(pk3_path).ok()?;
                zip.by_name(&v_zip).is_ok()
            };
            if has_exact {
                self.pk3_index.insert(
                    v_key.clone(),
                    Pk3EntryRef {
                        pk3_path: pk3_path.clone(),
                        entry_name: v_zip.clone(),
                    },
                );
                return Some(ResolvedAsset::Pk3 {
                    pk3_path: pk3_path.clone(),
                    entry_name: v_zip,
                    virtual_path: v_key,
                });
            }

            // Try lowercase variant (common in pk3).
            if v_key != v_zip {
                let has_lower = {
                    let zip = self.open_pk3_cached(pk3_path).ok()?;
                    zip.by_name(&v_key).is_ok()
                };
                if has_lower {
                    self.pk3_index.insert(
                        v_key.clone(),
                        Pk3EntryRef {
                            pk3_path: pk3_path.clone(),
                            entry_name: v_key.clone(),
                        },
                    );
                    return Some(ResolvedAsset::Pk3 {
                        pk3_path: pk3_path.clone(),
                        entry_name: v_key.clone(),
                        virtual_path: v_key,
                    });
                }
            }
        }

        None
    }

    /// Read the bytes of a resolved asset.
    pub fn read(&mut self, asset: &ResolvedAsset) -> Result<Vec<u8>, AssetDbError> {
        match asset {
            ResolvedAsset::Loose(path) => Ok(std::fs::read(path)?),
            ResolvedAsset::Pk3 {
                pk3_path,
                entry_name,
                ..
            } => {
                let zip = self.open_pk3_cached(pk3_path)?;
                let mut file = zip.by_name(entry_name)?;
                let mut buf = Vec::with_capacity(file.size() as usize);
                use std::io::Read;
                file.read_to_end(&mut buf)?;
                Ok(buf)
            }
        }
    }

    /// Resolve a material name (as used in `.map`/shader references) to a texture asset.
    ///
    /// Notes:
    /// - Tries `dds`, `tga`, `jpg`, `jpeg` when no extension is present.
    /// - Tries under `textures/` first, then falls back to the raw material string in case it
    ///   already contains a `textures/...` prefix.
    pub fn resolve_texture(&mut self, material: &str) -> Option<ResolvedAsset> {
        let rel = normalize_material_name(material);
        let rel = normalize_asset_path(&rel);

        // If material includes an extension, try it directly (both with and without textures/ prefix).
        if Path::new(&rel).extension().is_some() {
            if !rel.starts_with("textures/") {
                let cand = format!("textures/{rel}");
                if let Some(a) = self.resolve_virtual_path(&cand) {
                    return Some(a);
                }
            }
            if let Some(a) = self.resolve_virtual_path(&rel) {
                return Some(a);
            }
            return None;
        }

        let preferred = ["dds", "tga", "jpg", "jpeg"];
        for ext in preferred {
            if !rel.starts_with("textures/") {
                let cand = format!("textures/{rel}.{ext}");
                if let Some(a) = self.resolve_virtual_path(&cand) {
                    return Some(a);
                }
            }
            let cand = format!("{rel}.{ext}");
            if let Some(a) = self.resolve_virtual_path(&cand) {
                return Some(a);
            }
        }

        // Loose-file-only fallback: scan directory for case variants.
        self.find_first_supported_loose(&rel)
    }

    /// Resolve + decode a texture into RGBA8 suitable for uploading to OpenGL.
    pub fn load_texture_rgba8(&mut self, material: &str) -> Result<TextureImage, AssetDbError> {
        let asset = self
            .resolve_texture(material)
            .ok_or_else(|| AssetDbError::NotFound(format!("texture not found: {material}")))?;

        let bytes = self.read(&asset)?;

        let ext = match &asset {
            ResolvedAsset::Loose(p) => p
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase(),
            ResolvedAsset::Pk3 { virtual_path, .. } => Path::new(virtual_path)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase(),
        };

        Ok(decode_texture_rgba8(&bytes, &ext)?)
    }

    /// Resolve and memoize textures for a set of materials.
    ///
    /// This is intended for frontends that want to avoid opening many `.pk3` archives repeatedly
    /// during initial map load. It mirrors `resolve_texture()` logic (including extension priority)
    /// and fills the internal pk3 index for any found textures.
    pub fn prefetch_textures_for_materials<'a, I>(&mut self, materials: I)
    where
        I: IntoIterator<Item = &'a str>,
    {
        // Build candidate virtual paths for each material in resolve order.
        #[derive(Debug)]
        struct Mat {
            // Candidate virtual paths to check, in priority order.
            candidates: Vec<String>,
        }

        use std::collections::HashSet;

        let mut seen = HashSet::<String>::new();
        let mut mats: Vec<Mat> = Vec::new();

        for m in materials {
            let m_zip = canonical_virtual_path(&normalize_material_name(m));
            let m_key = normalize_asset_path(&m_zip);
            if !seen.insert(m_key.clone()) {
                continue;
            }

            let mut candidates = Vec::<String>::new();
            let rel = m_zip;

            if Path::new(&rel).extension().is_some() {
                if !rel.starts_with("textures/") {
                    candidates.push(format!("textures/{rel}"));
                }
                candidates.push(rel);
            } else {
                let preferred = ["dds", "tga", "jpg", "jpeg"];
                for ext in preferred {
                    if !rel.starts_with("textures/") {
                        candidates.push(format!("textures/{rel}.{ext}"));
                    }
                    candidates.push(format!("{rel}.{ext}"));
                }
            }

            mats.push(Mat { candidates });
        }

        if mats.is_empty() || self.pk3_paths.is_empty() {
            return;
        }

        // Search pk3s from highest priority to lowest. Once a material is resolved, stop searching it.
        let mut unresolved = vec![true; mats.len()];
        let mut remaining = mats.len();

        let pk3s: Vec<PathBuf> = self.pk3_paths.iter().cloned().collect();
        for pk3_path in pk3s.iter().rev() {
            if remaining == 0 {
                break;
            }

            let mut updates: Vec<(String, String)> = Vec::new(); // (key, entry_name)

            {
                let zip = match self.open_pk3_cached(pk3_path) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                for (idx, mat) in mats.iter().enumerate() {
                    if !unresolved[idx] {
                        continue;
                    }

                    // Try candidates in priority order; first hit wins for this pk3 priority level.
                    let mut found: Option<(String, String)> = None; // (key, entry_name)
                    for cand_zip in &mat.candidates {
                        let cand_key = normalize_asset_path(cand_zip);

                        if zip.by_name(cand_zip).is_ok() {
                            found = Some((cand_key, cand_zip.clone()));
                            break;
                        }

                        // Try lowercase variant.
                        if cand_key != *cand_zip && zip.by_name(&cand_key).is_ok() {
                            found = Some((cand_key.clone(), cand_key));
                            break;
                        }
                    }

                    if let Some((key, entry_name)) = found {
                        updates.push((key, entry_name));
                        unresolved[idx] = false;
                        remaining -= 1;
                    }
                }
            }

            for (key, entry_name) in updates {
                self.pk3_index.insert(
                    key,
                    Pk3EntryRef {
                        pk3_path: pk3_path.clone(),
                        entry_name,
                    },
                );
            }
        }
    }

    /// Load all `.shader` files from `scripts/` (loose + `.pk3`) and extract `qer_*` params.
    pub fn load_shader_db(&self) -> Result<ShaderDb, AssetDbError> {
        let mut db = ShaderDb::default();

        // 1) pk3 scripts first (lower priority). Prefer the pk3 index if it contains scripts entries
        // to avoid scanning every zip entry again.
        let mut scripts_by_pk3: HashMap<PathBuf, Vec<(String, String)>> = HashMap::new();
        for (vpath, pref) in &self.pk3_index {
            if vpath.starts_with("scripts/") && vpath.ends_with(".shader") {
                scripts_by_pk3
                    .entry(pref.pk3_path.clone())
                    .or_default()
                    .push((vpath.clone(), pref.entry_name.clone()));
            }
        }

        if !scripts_by_pk3.is_empty() {
            for pk3_path in &self.pk3_paths {
                let Some(list) = scripts_by_pk3.get(pk3_path) else {
                    continue;
                };
                let f = File::open(pk3_path)?;
                let mut zip = ZipArchive::new(f)?;
                for (vpath, entry_name) in list {
                    let mut file = zip.by_name(entry_name)?;
                    use std::io::Read;
                    let mut bytes = Vec::with_capacity(file.size() as usize);
                    file.read_to_end(&mut bytes)?;
                    let src = String::from_utf8(bytes).map_err(|e| AssetDbError::Utf8 {
                        path: vpath.clone(),
                        source: e,
                    })?;
                    parse_shader_source_into_db(&src, Path::new(vpath), &mut db)?;
                }
            }
        } else {
            // Fallback: scan pk3s directly.
            for pk3_path in &self.pk3_paths {
                let f = File::open(pk3_path)?;
                let mut zip = ZipArchive::new(f)?;
                for i in 0..zip.len() {
                    let mut file = zip.by_index(i)?;
                    if file.is_dir() {
                        continue;
                    }
                    let name = file.name().to_string();
                    let vpath = normalize_asset_path(&name);
                    if !vpath.starts_with("scripts/") || !vpath.ends_with(".shader") {
                        continue;
                    }
                    use std::io::Read;
                    let mut bytes = Vec::with_capacity(file.size() as usize);
                    file.read_to_end(&mut bytes)?;
                    let src = String::from_utf8(bytes).map_err(|e| AssetDbError::Utf8 {
                        path: vpath.clone(),
                        source: e,
                    })?;
                    parse_shader_source_into_db(&src, Path::new(&vpath), &mut db)?;
                }
            }
        }

        // 2) loose scripts override.
        let scripts_dir = self.roots.scripts_dir();
        if scripts_dir.exists() {
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
                let src = String::from_utf8(bytes).map_err(|e| AssetDbError::Utf8 {
                    path: path.display().to_string(),
                    source: e,
                })?;
                parse_shader_source_into_db(&src, &path, &mut db)?;
            }
        }

        Ok(db)
    }

    /// Like [`AssetDb::load_shader_db`], but caches the extracted `qer_*` params on disk to
    /// avoid re-parsing every `.shader` file on every startup.
    ///
    /// Cache location: `(main)/.radiant_core/shader_qer_cache.tsv`
    pub fn load_shader_db_cached(&self) -> Result<ShaderDb, AssetDbError> {
        let fingerprint = shader_cache_fingerprint(self.roots.maindir(), &self.pk3_paths)?;
        let cache_path = shader_cache_path(self.roots.maindir());

        if let Ok(file) = File::open(&cache_path) {
            use std::io::{BufRead, BufReader};
            let mut reader = BufReader::new(file);
            let mut header = String::new();
            if reader.read_line(&mut header).is_ok() {
                if shader_cache_header_matches(&header, fingerprint) {
                    let mut db = ShaderDb::default();
                    let mut line = String::new();
                    while reader
                        .read_line(&mut line)
                        .ok()
                        .filter(|&n| n > 0)
                        .is_some()
                    {
                        if let Some(def) = parse_shader_cache_line(line.trim_end()) {
                            db.insert(def);
                        }
                        line.clear();
                    }
                    return Ok(db);
                }
            }
        }

        let db = self.load_shader_db()?;

        // Best-effort write: failing to cache should not break startup.
        let _ = write_shader_cache(&cache_path, fingerprint, &db);

        Ok(db)
    }

    fn find_first_supported_loose(&self, rel: &str) -> Option<ResolvedAsset> {
        // Try under `(main)/textures/<rel>` first, then `(main)/<rel>`.
        let mut stems: Vec<PathBuf> = Vec::new();
        if !rel.starts_with("textures/") {
            stems.push(self.roots.maindir.join("textures").join(rel));
        }
        stems.push(self.roots.maindir.join(rel));

        let supported = ["dds", "tga", "jpg", "jpeg"];
        for stem in stems {
            let stem = stem.with_extension("");
            if let Some(found) = find_first_supported_in_dir(&stem, &supported) {
                return Some(ResolvedAsset::Loose(found));
            }
        }
        None
    }
}

/// Normalize an asset path for virtual lookup:
/// - trims whitespace
/// - forward slashes
/// - strips leading `/` and leading `./`
/// - lowercases ASCII (pk3 lookups are effectively case-insensitive)
pub fn normalize_asset_path(path: &str) -> String {
    let mut p = canonical_virtual_path(path);
    p.make_ascii_lowercase();
    p
}

fn find_pk3_files(maindir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut out = Vec::<PathBuf>::new();
    for entry in std::fs::read_dir(maindir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if ext.eq_ignore_ascii_case("pk3") {
            out.push(path);
        }
    }
    Ok(out)
}

fn is_useful_index_entry_raw(name: &str, opts: &AssetDbOptions) -> bool {
    if opts.full_index {
        return true;
    }
    // Most game paks use lowercase names already; treat this as an ASCII-fast filter and only
    // normalize when we keep the entry.
    if opts.index_shaders && name.starts_with("scripts/") && name.ends_with(".shader") {
        return true;
    }
    if opts.index_textures && name.starts_with("textures/") {
        return name.ends_with(".dds")
            || name.ends_with(".tga")
            || name.ends_with(".jpg")
            || name.ends_with(".jpeg");
    }
    false
}

fn resolve_texture_path_under(root: &Path, rel: &str) -> Option<PathBuf> {
    let path = root.join(rel);

    if path.extension().is_some() && path.exists() {
        return Some(path);
    }

    let stem = path.with_extension("");
    let preferred = ["dds", "tga", "jpg", "jpeg"];
    for ext in preferred {
        let cand = stem.with_extension(ext);
        if cand.exists() {
            return Some(cand);
        }
    }

    find_first_supported_in_dir(&stem, &preferred)
}

fn find_first_supported_in_dir(stem: &Path, supported: &[&str]) -> Option<PathBuf> {
    let parent = stem.parent()?;
    let base = stem.file_name()?.to_string_lossy().to_string();

    let rd = std::fs::read_dir(parent).ok()?;
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let file_stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if !file_stem.eq_ignore_ascii_case(&base) {
            continue;
        }
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        if supported.iter().any(|e| ext.eq_ignore_ascii_case(e)) {
            return Some(p);
        }
    }
    None
}

fn shader_cache_path(maindir: &Path) -> PathBuf {
    maindir.join(".radiant_core").join("shader_qer_cache.tsv")
}

fn shader_cache_fingerprint(maindir: &Path, pk3_paths: &[PathBuf]) -> Result<u64, std::io::Error> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    1u32.hash(&mut hasher); // cache version salt

    // pk3 archives (sorted by caller).
    for p in pk3_paths {
        p.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .hash(&mut hasher);
        let meta = std::fs::metadata(p)?;
        meta.len().hash(&mut hasher);
        let m = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        m.hash(&mut hasher);
    }

    // loose scripts directory (top-level .shader files).
    let scripts_dir = maindir.join("scripts");
    if scripts_dir.exists() {
        let mut entries: Vec<PathBuf> = Vec::new();
        for e in std::fs::read_dir(&scripts_dir)? {
            let e = e?;
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !ext.eq_ignore_ascii_case("shader") {
                continue;
            }
            entries.push(p);
        }
        entries.sort();

        for p in entries {
            p.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .hash(&mut hasher);
            let meta = std::fs::metadata(&p)?;
            meta.len().hash(&mut hasher);
            let m = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            m.hash(&mut hasher);
        }
    }

    Ok(hasher.finish())
}

fn shader_cache_header_matches(line0: &str, fingerprint: u64) -> bool {
    // radiant_core_shader_cache\t1\t<fingerprint>\n
    let line0 = line0.trim_end();
    let mut it = line0.split('\t');
    let tag = it.next().unwrap_or("");
    let ver = it.next().unwrap_or("");
    let fp = it.next().unwrap_or("");
    if tag != "radiant_core_shader_cache" {
        return false;
    }
    if ver != "1" {
        return false;
    }
    fp.parse::<u64>().ok() == Some(fingerprint)
}

fn parse_shader_cache_line(line: &str) -> Option<crate::shader::ShaderDef> {
    if line.is_empty() {
        return None;
    }
    let mut it = line.split('\t');
    let name = it.next()?.to_string();
    let editor_image = it.next().map(|s| s.to_string()).filter(|s| !s.is_empty());
    let light_image = it.next().map(|s| s.to_string()).filter(|s| !s.is_empty());
    let trans = it.next().and_then(|s| {
        if s.is_empty() {
            None
        } else {
            s.parse::<f32>().ok()
        }
    });
    let no_draw = it.next().map(|s| s == "1").unwrap_or(false);
    let no_carve = it.next().map(|s| s == "1").unwrap_or(false);

    Some(crate::shader::ShaderDef {
        name,
        qer: crate::shader::QerParams {
            editor_image,
            light_image,
            trans,
            no_draw,
            no_carve,
            extra: HashMap::new(),
        },
    })
}

fn write_shader_cache(path: &Path, fingerprint: u64, db: &ShaderDb) -> Result<(), std::io::Error> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write directly; this is best-effort, not crash-critical.
    let mut f = File::create(path)?;
    writeln!(f, "radiant_core_shader_cache\t1\t{fingerprint}")?;
    for sh in db.iter() {
        let e = sh.qer.editor_image.as_deref().unwrap_or("");
        let l = sh.qer.light_image.as_deref().unwrap_or("");
        let t = sh
            .qer
            .trans
            .map(|v| v.to_string())
            .unwrap_or_else(String::new);
        let nd = if sh.qer.no_draw { "1" } else { "0" };
        let nc = if sh.qer.no_carve { "1" } else { "0" };
        writeln!(f, "{}\t{}\t{}\t{}\t{}\t{}", sh.name, e, l, t, nd, nc)?;
    }
    Ok(())
}
