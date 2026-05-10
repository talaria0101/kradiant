#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ortho {
    #[default]
    XY,
    XZ,
    YZ,
}

impl Ortho {
    pub fn label(self) -> &'static str {
        match self {
            Ortho::XY => "XY (top)",
            Ortho::XZ => "XZ (front)",
            Ortho::YZ => "YZ (side)",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::XY => Self::XZ,
            Self::XZ => Self::YZ,
            Self::YZ => Self::XY,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum DragMode {
    #[default]
    NewBrush,
    MoveSelection,
    MoveVertices,
    StretchSelection,
    RotateSelection,
    RectangularSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StretchMode {
    Scale,
    Resize,
}

impl Default for StretchMode {
    fn default() -> Self {
        Self::Scale
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AxisLock {
    pub x: bool,
    pub y: bool,
    pub z: bool,
}
