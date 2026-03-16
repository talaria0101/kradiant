//! Loader modules (assets, textures, shaders).
//!
//! This is intended as the public, UI-agnostic surface area for "getting bytes into core types".
//! Internals still live in `assets`, `texture`, and `shader` for now, but are re-exported here to
//! provide a stable module hierarchy.

pub mod asset_loader;
pub mod map_loader;
pub mod shader_loader;
pub mod texture_loader;
