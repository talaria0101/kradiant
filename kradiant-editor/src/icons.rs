use dear_imgui_rs::TextureId;
use kradiant::texture::{decode_texture_rgba8, TextureImage};

pub struct EditorIcons {
    pub view_cycle: Option<TextureId>,
}

impl EditorIcons {
    pub fn get_image(icon: &[u8]) -> TextureImage
    {
        decode_texture_rgba8(icon, "dds").expect("decode editor icon")
    }
}

impl Default for EditorIcons {
    fn default() -> Self
    {
        Self {
            view_cycle: None,
        }
    }
}
