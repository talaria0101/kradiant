pub mod config;
pub mod selection;
pub mod state;
pub mod theme;
pub mod undo;
pub mod viewport;

pub use config::EditorConfig;
pub use selection::{FaceSelection, PatchVertexSelection};
pub use state::EditorState;
pub use theme::EditorPalette;
pub use undo::UndoRedo;

#[derive(Default, PartialEq, Clone)]
pub struct SurfInspector {
    pub tex_in: String,
    pub vshift_in: i32,
    pub hshift_in: i32,
    pub vstretch_in: f32,
    pub hstretch_in: f32,
    pub rotate_in: i32,
    pub sampsize_in: i32,
    pub sampsize_only: bool,
    pub value_in: i32,
}
