#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FaceSelection {
    pub entity_idx: usize,
    pub brush_idx: usize,
    pub face_idx: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PatchVertexSelection {
    pub entity_idx: usize,
    pub brush_idx: usize,
    pub row: usize,
    pub col: usize,
}
