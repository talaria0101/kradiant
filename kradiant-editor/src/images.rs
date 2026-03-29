use dear_imgui_rs::TextureId;
use kradiant::texture::{TextureImage, decode_texture_rgba8};

pub struct EditorImages {
    pub splash: Option<TextureId>,
}

impl EditorImages {
    pub fn get_image(icon: &[u8]) -> TextureImage {
        decode_texture_rgba8(icon, "dds").expect("decode editor icon")
    }
}

impl Default for EditorImages {
    fn default() -> Self {
        Self { splash: None }
    }
}
