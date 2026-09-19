//! The kradiant shared library for working with CoD maps.
//!
//! Currently this library handles:
//! - parsing `.map` source files into a structured [`map`] model
//! - tessellating brushes and patches into renderable geometry ([`geometry`])
//! - resolving textures and shader metadata needed for editing and preview
//! - providing loader facades ([`loader`]) that are independent of any UI or renderer

pub const KRADIANT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod assets;
pub mod core_util;
pub mod dirs;
pub mod editing;
pub mod editor;
pub mod ffi;
pub mod geometry;
pub mod loader;
pub mod map;
pub mod map_utils;
pub mod parser;
pub mod render;
pub mod shader;
pub mod texmap;
pub mod texture;
pub mod xmodel;

// Re-export common types for UI layer
pub use glam::{IVec2, IVec3, Mat4, Quat, Vec2, Vec3, Vec4};

// TODO: get_polygons_for_brush(id) -> &[Polygon] (cached vertex/index buffers)
// TODO: FFI layer (extern "C" + cbindgen) - low priority
