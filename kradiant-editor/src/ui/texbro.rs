//! Texture Browser — filesystem navigation and asset loading from pak files

use dear_imgui_rs::{Condition, StyleColor, TextureId, Ui};
use kradiant::loader::asset_loader::{AssetDb, AssetDbOptions};
use kradiant::map::Map;
use kradiant::shader::{QerParams, ShaderDb};
use kradiant::texture::TextureImage;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::util;

/// GPU texture handle + metadata needed for 3D viewport rendering.
#[derive(Debug, Clone)]
pub struct RenderTextureInfo {
    pub tex: glow::Texture,
    pub size: [f32; 2],
    pub qer: QerParams,
}

/// Manages asset loading and filesystem navigation for texture browsing.
#[derive(Debug)]
pub struct TextureBrowser {
    asset_db: Option<AssetDb>,
    /// Cached shader database for qer_* parameter lookups.
    pub shader_db: Option<ShaderDb>,
    current_path: String,
    /// Cached list of subdirectories under current path
    subdirs: Vec<String>,
    /// Cached list of textures under current path
    textures: Vec<TextureEntry>,
    /// Whether to show an expandable directory tree on the left
    show_dir_tree: bool,
    /// Current selected texture
    pub selected: Option<String>,
    /// CPU loaded pixel data for textures currently displayed
    pub tex_cache: HashMap<String, TextureImage>,
    /// GPU IDs for textures
    pub tex_gpu_cache: HashMap<String, (TextureId, [f32; 2])>,
    /// GPU textures for 3D rendering (OpenGL texture object + size + shader params).
    pub tex_render_cache: HashMap<String, RenderTextureInfo>,
    /// Textures waiting for upload to GPU
    pub pending_uploads: Vec<(String, TextureImage)>,
    /// Textures waiting for upload to GPU for 3D rendering (mipmapped GL textures).
    pub pending_render_uploads: Vec<(String, TextureImage)>,
}

#[derive(Debug, Clone)]
pub struct TextureEntry {
    /// Full virtual path (e.g., "textures/common/caulk")
    pub path: String,
    /// Display name (filename without extension)
    pub display: String,
}

fn texture_entry_from_str(s: &str) -> io::Result<TextureEntry> {
    //use std::path::Path;
    //let tmp_path = Path::new(s);
    let mut tok: Vec<&str> = s.split(".").collect();
    if let Some(ext) = tok.last() {
        println!("ext: {ext}");
        if AssetDb::file_useful_for_radiant(*ext) {
            tok.pop();
        } else {
            let t = format!("Unknown file: {}", s);
            return Err(io::Error::new(io::ErrorKind::InvalidInput, t));
        }
    }

    let path = tok.join("");
    println!("t entry path: {path}");

    let display = path.rsplit('/').nth(0).unwrap_or(&path).to_string();

    Ok(TextureEntry { path, display })
}

/// Convert a ResolvedAsset into a TextureEntry with a normalised material path.
///
/// The resulting `path` is always in the form `category/name` —
/// no `textures/` prefix, no file extension. This matches what the map
/// stores in face.texture and what AssetDb::resolve_texture() expects.
fn texture_entry_from_resolved(
    resolved: &kradiant::assets::ResolvedAsset,
    textures_dir: &std::path::Path,
) -> io::Result<TextureEntry> {
    use kradiant::assets::ResolvedAsset;

    let virtual_path: String = match resolved {
        // Pk3: virtual_path is already lowercase with forward slashes.
        // e.g. "textures/common/caulk.tga"
        ResolvedAsset::Pk3 { virtual_path, .. } => virtual_path.clone(),

        // Loose: absolute path on disk.
        // e.g. "/home/kazam/.local/share/orbix/main/textures/kazam/village/foliage@hay.jpg"
        // Strip the textures_dir prefix to get the relative part,
        // then rebuild it as a virtual path.
        ResolvedAsset::Loose(abs_path) => {
            let rel = abs_path.strip_prefix(textures_dir).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "loose path {} is not under textures_dir {}",
                        abs_path.display(),
                        textures_dir.display()
                    ),
                )
            })?;
            // Convert OS separators to forward slashes and prepend "textures/"
            format!("textures/{}", rel.to_string_lossy().replace('\\', "/"))
        }
    };

    // Strip the "textures/" prefix (9 chars) to get the material name.
    let material = virtual_path
        .strip_prefix("textures/")
        .unwrap_or(&virtual_path);

    // Strip the extension — find last dot after last slash.
    let material_no_ext = if let Some(dot) = material.rfind('.') {
        // Make sure the dot is in the filename, not a directory name.
        let last_slash = material.rfind('/').unwrap_or(0);
        if dot > last_slash {
            &material[..dot]
        } else {
            material
        }
    } else {
        material
    };

    if material_no_ext.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty material path",
        ));
    }

    let display = material_no_ext
        .rsplit('/')
        .next()
        .unwrap_or(material_no_ext)
        .to_string();

    Ok(TextureEntry {
        path: material_no_ext.to_string(),
        display,
    })
}

impl TryFrom<String> for TextureEntry {
    type Error = std::io::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        texture_entry_from_str(&value)
    }
}
impl TryFrom<&str> for TextureEntry {
    type Error = std::io::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        texture_entry_from_str(value)
    }
}

impl Default for TextureBrowser {
    fn default() -> Self {
        Self {
            asset_db: None,
            shader_db: None,
            current_path: String::new(),
            subdirs: Vec::new(),
            textures: Vec::new(),
            show_dir_tree: true,
            selected: None,
            tex_cache: HashMap::new(),
            tex_gpu_cache: HashMap::new(),
            tex_render_cache: HashMap::new(),
            pending_uploads: Vec::new(),
            pending_render_uploads: Vec::new(),
        }
    }
}

impl TextureBrowser {
    /// Initialize if maindir is available
    pub fn init(&mut self, maindir: &PathBuf) {
        match AssetDb::from_maindir_with_options(
            maindir.clone(),
            AssetDbOptions {
                full_index: false,
                index_textures: true,
                index_shaders: true,
            },
        ) {
            Ok(db) => {
                self.shader_db = db.load_shader_db_cached().ok();
                self.asset_db = Some(db);
                self.refresh_contents();
            }
            Err(e) => {
                eprintln!("Failed to load asset database: {}", e);
            }
        }
    }

    /// Refresh the list of subdirectories and textures in the current path
    fn refresh_contents(&mut self) {
        self.subdirs.clear();
        self.textures.clear();
        self.tex_cache.clear();
        self.tex_gpu_cache.clear();
        self.pending_uploads.clear();
        // Note: tex_render_cache is NOT cleared here because textures used by the map
        // for 3D rendering should persist when navigating directories.

        let Some(db) = &self.asset_db else {
            return;
        };

        // Collect all virtual paths that start with current_path
        let mut dir_set = BTreeSet::new();
        let mut tex_vec: Vec<TextureEntry> = Vec::new();

        // Prepend "textures/" to current_path to match virtual paths from asset DB
        let prefix = format!("textures/{}", self.current_path);

        // Scan pk3 files
        for vpath in db.iter_pk3_virtual_paths() {
            if !vpath.starts_with(&prefix) {
                continue;
            }

            let remainder = &vpath[prefix.len()..];

            // If there's a slash in the remainder, it's a subdirectory
            if let Some(slash_pos) = remainder.find('/') {
                let subdir = remainder[..slash_pos].to_string();
                if !subdir.is_empty() {
                    dir_set.insert(subdir);
                }
            } else if !remainder.is_empty() {
                // It's a file in the current directory, add if it looks like a texture
                let s = &vpath[9..];
                match s.try_into() {
                    Ok(e) => tex_vec.push(e),
                    Err(e) => eprintln!("{e}"),
                }
            }
        }

        // Scan loose files in textures directory
        let textures_dir = db.roots().textures_dir();
        let current_loose_dir = if self.current_path.is_empty() {
            textures_dir.to_path_buf()
        } else {
            textures_dir.join(self.current_path.trim_end_matches('/'))
        };

        if current_loose_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&current_loose_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            dir_set.insert(name.to_string());
                        }
                    } else if path.is_file() {
                        if let Ok(rel) = path.strip_prefix(textures_dir) {
                            if let Some(rel_str) = rel.to_str() {
                                let vpath = format!("textures/{}", rel_str.replace('\\', "/"));
                                if let Ok(entry) = (&vpath[9..]).try_into() {
                                    println!("path: {}", path.display());
                                    println!("entry: {:#?}", &entry);
                                    tex_vec.push(entry);
                                }
                            }
                        }
                    }
                }
            }
        }

        self.subdirs = dir_set.into_iter().collect();
        self.textures = tex_vec;
    }

    /// Navigate to a subdirectory or parent with ".."
    pub fn navigate(&mut self, dir: &str) {
        if dir == ".." {
            let trimmed = self.current_path.trim_end_matches('/');
            if let Some(last_slash) = trimmed.rfind('/') {
                self.current_path = trimmed[..=last_slash].to_string();
            } else {
                self.current_path = String::new();
            }
        } else {
            // only add separator if we already have a path
            if !self.current_path.is_empty() && !self.current_path.ends_with('/') {
                self.current_path.push('/');
            }
            self.current_path.push_str(dir);
            self.current_path.push('/');
        }
        self.refresh_contents();
        self.selected = None;
    }

    /// Get the display name of the current directory
    pub fn current_dir_name(&self) -> String {
        let trimmed = self.current_path.trim_end_matches('/');
        trimmed.rsplit('/').next().unwrap_or("textures").to_string()
    }

    /// Get the breadcrumb path (e.g., "common > caulk")
    pub fn breadcrumb_path(&self) -> Vec<String> {
        if self.current_path.is_empty() {
            return vec![];
        }
        let trimmed = self.current_path.trim_end_matches('/');
        trimmed.split('/').map(|s| s.to_string()).collect()
    }

    /// Load a texture by virtual path
    pub fn load_texture(&mut self, material: &str) -> Result<TextureImage, String> {
        let Some(db) = &mut self.asset_db else {
            return Err("Asset database not initialized".to_string());
        };
        db.load_texture_rgba8(material).map_err(|e| e.to_string())
    }

    /// Get list of subdirectories in current path
    pub fn get_subdirs(&self) -> &[String] {
        &self.subdirs
    }

    /// Get list of textures in current path
    pub fn get_textures(&self) -> &[TextureEntry] {
        &self.textures
    }

    pub fn is_initialized(&self) -> bool {
        self.asset_db.is_some()
    }

    /// Public method for clearing texture caches
    pub fn clear_texture_caches(&mut self) {
        self.tex_cache.clear();
        self.tex_gpu_cache.clear();
        self.tex_render_cache.clear();
        self.pending_uploads.clear();
        self.pending_render_uploads.clear();
    }

    /// Enqueue a texture for GPU upload if not already cached or pending.
    /// Note: browser textures (tex_gpu_cache) are loaded independently of 3D render textures
    /// (tex_render_cache) so that navigating directories doesn't affect 3D viewport rendering.
    pub fn request_texture_load(&mut self, material: &str) {
        // Only check browser-specific caches - 3D render cache is independent
        if self.tex_gpu_cache.contains_key(material)
            || self.tex_cache.contains_key(material)
            || self.pending_uploads.iter().any(|(k, _)| k == material)
        {
            return;
        }
        match self.load_texture(material) {
            Ok(img) => {
                self.tex_cache.insert(material.to_string(), img.clone());
                self.pending_uploads
                    .push((material.to_string(), img.clone()));
                // Also enqueue for 3D render cache in case it's needed there too
                if !self.tex_render_cache.contains_key(material)
                    && !self.pending_render_uploads.iter().any(|(k, _)| k == material)
                {
                    self.pending_render_uploads
                        .push((material.to_string(), img));
                }
            }
            Err(e) => eprintln!("tex load failed {material}: {e}"),
        }
    }

    /// Upload pending render textures to OpenGL with mipmaps.
    ///
    /// This is separate from ImGui texture registration; it builds real GL texture objects
    /// suitable for sampling in the 3D viewport.
    pub fn process_pending_render_uploads(
        &mut self,
        gl: &glow::Context,
        uploads_per_frame: usize,
        upload_texture_mipmaps: unsafe fn(&glow::Context, [u32; 2], &[u8]) -> glow::Texture,
    ) {
        let batch: Vec<_> = self
            .pending_render_uploads
            .drain(..self.pending_render_uploads.len().min(uploads_per_frame))
            .collect();

        for (material, img) in batch {
            let tex = unsafe { upload_texture_mipmaps(gl, [img.width, img.height], &img.rgba8) };
            let qer = self
                .shader_db
                .as_ref()
                .and_then(|db| db.get(&material))
                .map(|sh| sh.qer.clone())
                .unwrap_or_default();
            //println!("{material} qer: {:#?}", &qer);
            self.tex_render_cache.insert(
                material,
                RenderTextureInfo {
                    tex,
                    size: [img.width as f32, img.height as f32],
                    qer,
                },
            );
        }
    }
}

/// Draw the texture browser UI
pub fn draw_texture_browser(
    ui: &Ui,
    browser: &mut TextureBrowser,
    tex_filter: &mut String,
    tex_tile_size: &mut f32,
    on_select: &mut dyn FnMut(String, &mut Option<Map>),
    map: &mut Option<Map>,
) {
    ui.window("Textures")
        .size([1280.0, 400.0], Condition::FirstUseEver)
        .build(|| {
            // Top toolbar
            ui.text("Filter:");
            ui.same_line();
            ui.set_next_item_width(150.0);
            ui.input_text("##tex_filter", tex_filter)
                .hint("e.g. caulk")
                .build();

            ui.same_line();
            ui.text("Size:");
            ui.same_line();
            ui.set_next_item_width(100.0);
            ui.slider_config("##tile_size", 32.0f32, 256.0f32)
                .display_format("%.0f px")
                .build(tex_tile_size);
            ui.same_line();
            if ui.button("Show Used") {
                if let Some(map) = map {
                    browser.clear_texture_caches();
                    browser.textures.clear();
                    let used = map.collect_used_materials(None);
                    if let Some(db) = &mut browser.asset_db {
                        let textures_dir = db.roots().textures_dir().to_path_buf();
                        for material in used {
                            if let Some(resolved) = db.resolve_texture(&material) {
                                match texture_entry_from_resolved(&resolved, &textures_dir) {
                                    Ok(entry) => browser.textures.push(entry),
                                    Err(e) => eprintln!("skip {material}: {e}"),
                                }
                            }
                        }
                    }
                    // Don't call refresh_contents() here — it would overwrite what we just built
                    //browser.subdirs.clear();
                }
            }

            if let Some(sel) = &browser.selected {
                ui.same_line();
                ui.text_disabled(format!("selected: {sel}"));
            }

            ui.separator();

            if !browser.is_initialized() {
                ui.text_disabled("Asset database not initialized");
                return;
            }

            // Breadcrumb navigation
            draw_breadcrumb(ui, browser, on_select);

            ui.separator();

            // Main layout: left panel (directory tree) + right panel (textures)
            let [panel_w, panel_h] = ui.content_region_avail();
            let tree_w = (panel_w * 0.25).max(150.0);
            let tiles_w = panel_w - tree_w - 4.0;

            // Left panel: directory tree
            ui.child_window("##dir_tree")
                .size([tree_w, panel_h])
                .border(true)
                .build(ui, || {
                    draw_directory_tree(ui, browser);
                });

            ui.same_line_with_spacing(0.0, 4.0);

            // Right panel: texture tiles
            ui.child_window("##tex_tiles")
                .size([tiles_w, panel_h])
                .build(ui, || {
                    draw_texture_tiles(ui, browser, tex_filter, *tex_tile_size, map, on_select);
                });
        });
}

/// Draw breadcrumb navigation bar
fn draw_breadcrumb(
    ui: &Ui,
    browser: &mut TextureBrowser,
    _on_select: &mut dyn FnMut(String, &mut Option<Map>),
) {
    let breadcrumbs = browser.breadcrumb_path();

    if breadcrumbs.is_empty() {
        ui.text("Textures /");
        return;
    }

    for (i, crumb) in breadcrumbs.iter().enumerate() {
        if i > 0 {
            ui.same_line_with_spacing(0.0, 4.0);
            ui.text("/");
            ui.same_line_with_spacing(0.0, 4.0);
        }

        if ui.button(crumb) {
            // Navigate to this level by reconstructing the path
            let mut target_path = breadcrumbs[..=i].join("/");
            if !target_path.is_empty() {
                target_path.push('/');
            }
            if target_path != browser.current_path {
                browser.current_path = target_path;
                browser.refresh_contents();
                browser.selected = None;
            }
        }
    }
}

/// Draw the directory tree
fn draw_directory_tree(ui: &Ui, browser: &mut TextureBrowser) {
    let mut has_items = false;

    if !browser.current_path.is_empty() {
        if ui.button("📁 ..") {
            browser.navigate("..");
        }
        has_items = true;
    }

    // collect first to avoid borrow issues
    let subdirs: Vec<String> = browser.get_subdirs().to_vec();
    for subdir in subdirs {
        if ui.button(format!("📁 {}", subdir)) {
            browser.navigate(&subdir);
        }
        has_items = true;
    }

    // Ensure window is never completely empty
    if !has_items {
        ui.text_disabled("(no subdirectories)");
    }
}

/// Draw texture tiles
fn draw_texture_tiles(
    ui: &Ui,
    browser: &mut TextureBrowser,
    filter: &str,
    tile_size: f32,
    map: &mut Option<Map>,
    on_select: &mut dyn FnMut(String, &mut Option<Map>),
) {
    let textures = browser.get_textures();

    if textures.is_empty() {
        ui.text_disabled("no textures in this directory");
        return;
    }

    let filter_lc = filter.to_ascii_lowercase();
    let visible: Vec<TextureEntry> = textures
        .iter()
        .filter(|t| filter_lc.is_empty() || t.display.to_ascii_lowercase().contains(&filter_lc))
        .cloned()
        .collect();

    if visible.is_empty() {
        ui.text_disabled("no textures match filter");
        ui.spacing();
        return;
    }

    let cell_width = tile_size + 8.0; // 8 = padding between columns
    let avail_w = ui.content_region_avail()[0].max(cell_width);
    let cols = ((avail_w) / cell_width).floor().max(1.0) as usize;

    for (i, entry) in visible.iter().enumerate() {
        let p = ui.cursor_screen_pos();

        let display_size = if let Some(&(_, orig_size)) = browser.tex_gpu_cache.get(&entry.path) {
            let aspect = (orig_size[0] / orig_size[1].max(1.0)).max(0.001);
            let height = tile_size / aspect;
            [tile_size, height]
        } else {
            // Placeholder square
            [tile_size, tile_size]
        };

        if let Some(&(tid, _)) = browser.tex_gpu_cache.get(&entry.path) {
            ui.image(tid, display_size);
        } else {
            ui.dummy(display_size); // Reserves the exact layout space

            // Draw color placeholder exactly over the dummy's space
            let hash = entry
                .display
                .bytes()
                .fold(5381u32, |a, b| a.wrapping_mul(33).wrapping_add(b as u32));
            let r = (((hash) & 0x7F) as f32 + 64.0) / 255.0;
            let g = (((hash >> 8) & 0x7F) as f32 + 64.0) / 255.0;
            let b = (((hash >> 16) & 0x7F) as f32 + 64.0) / 255.0;
            let col = crate::util::pack_abgr(r, g, b, 1.0);

            ui.get_window_draw_list()
                .add_rect(p, [p[0] + display_size[0], p[1] + display_size[1]], col)
                .filled(true)
                .build();

            let in_view = unsafe {
                dear_imgui_rs::sys::igIsRectVisible_Vec2(
                    dear_imgui_rs::sys::ImVec2 { x: p[0], y: p[1] },
                    dear_imgui_rs::sys::ImVec2 {
                        x: p[0] + display_size[0],
                        y: p[1] + display_size[1],
                    },
                )
            };
            if in_view {
                browser.request_texture_load(&entry.path);
            }
        }
        let selected = browser.selected.as_deref() == Some(&entry.path);
        if selected {
            let h_color = util::imgui_color_to_u32(ui.style_color(StyleColor::TabSelectedOverline));
            ui.get_window_draw_list()
                .add_rect(
                    p,
                    [p[0] + display_size[0], p[1] + display_size[1]],
                    util::adjust_color_opacity(h_color, 0.5),
                )
                .filled(true)
                .build();
        }

        if ui.is_item_hovered() {
            ui.tooltip_text(&entry.path);
        }

        if ui.is_item_clicked() {
            browser.selected = if selected {
                None
            } else {
                on_select(entry.path.clone(), map);
                Some(entry.path.clone())
            };
        }

        if (i + 1) % cols != 0 {
            ui.same_line_with_spacing(0.0, 4.0);
        }
    }
}
