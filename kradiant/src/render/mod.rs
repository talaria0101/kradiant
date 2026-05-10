use glam::{Vec2, Vec3, Vec4};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba(pub Vec4);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawParams {
    pub color: Rgba,
    pub thickness: f32,
}

impl Default for DrawParams {
    fn default() -> Self {
        Self {
            color: Rgba(Vec4::ONE),
            thickness: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(pub u64);

pub trait Renderer {
    fn draw_line_2d(&mut self, a: Vec2, b: Vec2, params: DrawParams);
    fn draw_triangles_3d(&mut self, vertices: &[Vec3], indices: &[u32], params: DrawParams);
    fn draw_text_2d(&mut self, pos: Vec2, text: &str, params: DrawParams);
}

pub trait TextureManager {
    fn create_rgba8(&mut self, width: u32, height: u32, data: &[u8]) -> TextureHandle;
    fn destroy(&mut self, handle: TextureHandle);
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PointerState {
    pub pos: Vec2,
    pub delta: Vec2,
    pub wheel: Vec2,
    pub buttons: u32,
    pub modifiers: u32,
}

pub trait InputHandler {
    fn pointer(&self) -> PointerState;
    fn key_down(&self, key: Key) -> bool;
    fn key_pressed(&self, key: Key) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Escape,
    Tab,
    Backspace,
    Enter,
    Space,
    Left,
    Right,
    Up,
    Down,
    A,
    C,
    D,
    O,
    S,
    V,
    X,
    Y,
    Z,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    PageUp,
    PageDown,
    LCtrl,
    RCtrl,
    LShift,
    RShift,
    LAlt,
    RAlt,
}

pub trait UiBackend {
    fn set_clipboard_text(&mut self, text: &str);
    fn clipboard_text(&mut self) -> Option<String>;
}

