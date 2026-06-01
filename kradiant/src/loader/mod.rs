//! High‑level loader modules for assets, maps, shaders and textures.
//!
//! This is the public, UI‑agnostic surface area for turning on‑disk data into in‑memory core
//! types. Internals live in lower‑level modules such as `assets`, `texture`, and `shader`, but
//! are re‑exported here to provide a stable module hierarchy.

pub mod asset_loader;
pub mod map_loader;
pub mod shader_loader;
pub mod texture_loader;
pub mod xmodel_loader;
