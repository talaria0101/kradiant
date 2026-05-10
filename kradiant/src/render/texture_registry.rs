use crate::shader::QerParams;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct RenderTextureInfo {
    pub tex: glow::Texture,
    pub size: [f32; 2],
    pub qer: QerParams,
}

#[derive(Default, Debug)]
pub struct TextureRegistry {
    pub tex_render_cache: HashMap<String, RenderTextureInfo>,
    pub pending_requests: HashSet<String>,
}

impl TextureRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, material: String, info: RenderTextureInfo) {
        self.pending_requests.remove(&material);
        self.tex_render_cache.insert(material, info);
    }

    pub fn get(&self, material: &str) -> Option<&RenderTextureInfo> {
        self.tex_render_cache.get(material)
    }

    pub fn request(&mut self, material: String) {
        if !self.tex_render_cache.contains_key(&material) {
            self.pending_requests.insert(material);
        }
    }

    pub fn clear(&mut self) {
        self.tex_render_cache.clear();
        self.pending_requests.clear();
    }
}
