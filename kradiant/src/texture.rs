//! Texture decoding utilities for turning common image formats into RGBA8 buffers.
//!
//! This module focuses on loading images from disk or memory and converting them into a simple
//! CPU-side representation (`TextureImage`) that callers can upload to their rendering backend.

use std::path::Path;

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    /// RGBA8, row-major, top-to-bottom.
    pub rgba8: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TextureError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    //#[error("feature required: {0}")]
    //FeatureRequired(&'static str),
    #[error("unsupported texture format: {0}")]
    UnsupportedFormat(String),

    #[error("decode error: {0}")]
    Decode(String),
}

/// Load a texture from disk and decode it into RGBA8 suitable for uploading to a GPU.
///
/// Supported:
/// - `jpg`/`jpeg`
/// - `tga`
/// - `dds` (DXT1/DXT3/DXT5 and some 32bpp bitmask formats; top mip only)
pub fn load_texture_rgba8(path: impl AsRef<Path>) -> Result<TextureImage, TextureError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    decode_texture_rgba8(&bytes, &ext)
}

/// Decode a texture already loaded into memory into RGBA8.
///
/// `ext` should be a lowercase extension (e.g. `"tga"`, `"dds"`).
pub fn decode_texture_rgba8(bytes: &[u8], ext: &str) -> Result<TextureImage, TextureError> {
    let fmt = match ext {
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "tga" => image::ImageFormat::Tga,
        "dds" => image::ImageFormat::Dds,
        _ => return Err(TextureError::UnsupportedFormat(ext.to_string())),
    };
    let img = image::load_from_memory_with_format(bytes, fmt)
        .map_err(|e| TextureError::Decode(e.to_string()))?;
    let rgba = img.into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(TextureImage {
        width,
        height,
        rgba8: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {}
