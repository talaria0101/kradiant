use dear_imgui_rs::TextureId;
use kradiant::texture::{TextureImage, decode_texture_rgba8};

pub struct EditorIcons {
    pub open: Option<TextureId>,
    pub save: Option<TextureId>,
    pub view_cycle: Option<TextureId>,
    pub free_rotate: Option<TextureId>,
    pub free_scale: Option<TextureId>,
    pub resize: Option<TextureId>,
    pub lock_x: Option<TextureId>,
    pub lock_y: Option<TextureId>,
    pub lock_z: Option<TextureId>,
    pub grid_snap: Option<TextureId>,
    pub edit_face: Option<TextureId>,
    pub edit_edge: Option<TextureId>,
    pub edit_vertex: Option<TextureId>,
    pub donate: Option<TextureId>,
}

impl EditorIcons {
    pub fn get_image(icon: &[u8]) -> TextureImage {
        decode_texture_rgba8(icon, "dds").expect("decode editor icon")
    }
}

impl Default for EditorIcons {
    fn default() -> Self {
        Self {
            open: None,
            save: None,
            view_cycle: None,
            free_rotate: None,
            free_scale: None,
            resize: None,
            lock_x: None,
            lock_y: None,
            lock_z: None,
            grid_snap: None,
            edit_face: None,
            edit_edge: None,
            edit_vertex: None,
            donate: None,
        }
    }
}
