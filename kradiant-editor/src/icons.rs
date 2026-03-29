use dear_imgui_rs::TextureId;
use kradiant::texture::{TextureImage, decode_texture_rgba8};

pub struct EditorIcons {
    pub open: Option<TextureId>,
    pub save: Option<TextureId>,
    pub view_cycle: Option<TextureId>,
    pub mouse_rotate: Option<TextureId>
}

impl EditorIcons {
    pub fn get_image(icon: &[u8]) -> TextureImage {
        decode_texture_rgba8(icon, "dds").expect("decode editor icon")
    }
}

impl Default for EditorIcons {
    fn default() -> Self {
        Self { open: None, save: None, view_cycle: None, mouse_rotate: None }
    }
}
