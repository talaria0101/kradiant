//! Texture decoding utilities for turning common image formats into RGBA8 buffers.
//!
//! This module focuses on loading images from disk or memory and converting them into a simple
//! CPU-side representation (`TextureImage`) that callers can upload to their rendering backend.

use std::path::Path;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
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

/// Load and decode multiple textures from disk in parallel.
///
/// Returns one `Result` per path, preserving order. File reads and decoding are both parallelized.
pub fn load_textures_batch(paths: &[&Path]) -> Vec<Result<TextureImage, TextureError>> {
    paths.par_iter().map(|p| load_texture_rgba8(p)).collect()
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

/// Decode multiple textures in parallel.
///
/// Each item is `(bytes, ext)` where `ext` is a lowercase extension (e.g. `"dds"`, `"tga"`).
/// Returns one `Result` per input, preserving order.
pub fn decode_textures_batch(items: &[(&[u8], &str)]) -> Vec<Result<TextureImage, TextureError>> {
    items
        .par_iter()
        .map(|(bytes, ext)| decode_texture_rgba8(bytes, ext))
        .collect()
}

// ---------------------------------------------------------------------------
// DDS GPU-compressed path: parse header, upload compressed blocks directly.
// ---------------------------------------------------------------------------

/// OpenGL internal format for a compressed DDS surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsGpuFormat {
    Dxt1,
    Dxt3,
    Dxt5,
}

impl DdsGpuFormat {
    /// `gl::COMPRESSED_*_S3TC_*_EXT` constant.
    pub fn gl_format(self) -> u32 {
        match self {
            DdsGpuFormat::Dxt1 => glow::COMPRESSED_RGB_S3TC_DXT1_EXT,
            DdsGpuFormat::Dxt3 => glow::COMPRESSED_RGBA_S3TC_DXT3_EXT,
            DdsGpuFormat::Dxt5 => glow::COMPRESSED_RGBA_S3TC_DXT5_EXT,
        }
    }

    /// Bytes per 4×4 block.
    pub fn block_bytes(self) -> usize {
        match self {
            DdsGpuFormat::Dxt1 => 8,
            DdsGpuFormat::Dxt3 | DdsGpuFormat::Dxt5 => 16,
        }
    }
}

/// A DDS surface ready for direct GPU upload via `glCompressedTexImage2D`.
///
/// Contains pre-computed compressed data and per-mip metadata so the caller
/// never needs to touch the DDS header again.
#[derive(Debug, Clone)]
pub struct CompressedTexture {
    pub width: u32,
    pub height: u32,
    pub gpu_format: DdsGpuFormat,
    /// Per-mip compressed data.  `mips[0]` is the base level.
    pub mips: Vec<Vec<u8>>,
}

/// Parse a DDS file (header + compressed pixel data) into a [`CompressedTexture`].
///
/// Only DXT1, DXT3, and DXT5 are supported.  Returns an error for other fourCC
/// values (uncompressed formats, BPTC, etc.) — those should fall through to the
/// CPU `image` crate path.
pub fn parse_dds(data: &[u8]) -> Result<CompressedTexture, TextureError> {
    if data.len() < 128 {
        return Err(TextureError::Decode("DDS data too short".into()));
    }
    if &data[..4] != b"DDS " {
        return Err(TextureError::Decode("missing DDS magic".into()));
    }

    // ---------- header (124 bytes starting at offset 4) ----------
    let h = &data[4..128];
    let dw_height = u32::from_le_bytes(h[8..12].try_into().unwrap());
    let dw_width = u32::from_le_bytes(h[12..16].try_into().unwrap());
    let dw_mip_count = {
        let v = u32::from_le_bytes(h[24..28].try_into().unwrap());
        v.max(1)
    };

    // ---------- pixel format (at file offset 76, 32 bytes) ----------
    // h starts at file offset 4, so pixel format is at h[72..104]
    let pf = &h[72..104];
    let fourcc = &pf[8..12]; // pf.dwFourCC at file offset 84
    let gpu_format = match fourcc {
        b"DXT1" => DdsGpuFormat::Dxt1,
        b"DXT3" => DdsGpuFormat::Dxt3,
        b"DXT5" => DdsGpuFormat::Dxt5,
        other => {
            return Err(TextureError::UnsupportedFormat(format!(
                "DDS fourCC {:?} (only DXT1/3/5 supported for GPU path)",
                other
            )));
        }
    };

    let block_bytes = gpu_format.block_bytes();
    let mut mips = Vec::with_capacity(dw_mip_count as usize);
    let mut offset = 128usize; // skip header

    for level in 0..dw_mip_count {
        let mw = (dw_width >> level).max(1);
        let mh = (dw_height >> level).max(1);
        let blocks_x = ((mw + 3) / 4) as usize;
        let blocks_y = ((mh + 3) / 4) as usize;
        let size = blocks_x * blocks_y * block_bytes;

        if offset + size > data.len() {
            return Err(TextureError::Decode(format!(
                "DDS truncated at mip {level}: need {size} bytes, have {}",
                data.len() - offset,
            )));
        }
        mips.push(data[offset..offset + size].to_vec());
        offset += size;
    }

    Ok(CompressedTexture {
        width: dw_width,
        height: dw_height,
        gpu_format,
        mips,
    })
}

/// Load a DDS file from disk and parse it for direct GPU upload.
pub fn load_dds_compressed(path: impl AsRef<Path>) -> Result<CompressedTexture, TextureError> {
    let data = std::fs::read(path)?;
    parse_dds(&data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal DDS file with the given fourCC and dimensions.
    fn make_test_dds(fourcc: &[u8; 4], width: u32, height: u32) -> Vec<u8> {
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(b"DDS ");
        // dwSize = 124
        data[4..8].copy_from_slice(&124u32.to_le_bytes());
        // dwHeight
        data[12..16].copy_from_slice(&height.to_le_bytes());
        // dwWidth
        data[16..20].copy_from_slice(&width.to_le_bytes());
        // dwMipMapCount = 1
        data[28..32].copy_from_slice(&1u32.to_le_bytes());
        // pf.dwSize = 32
        data[76..80].copy_from_slice(&32u32.to_le_bytes());
        // pf.dwFourCC
        data[84..88].copy_from_slice(fourcc);

        let block_bytes: usize = match fourcc {
            b"DXT1" => 8,
            _ => 16,
        };
        let blocks_x = ((width + 3) / 4) as usize;
        let blocks_y = ((height + 3) / 4) as usize;
        let mip_size = blocks_x * blocks_y * block_bytes;
        data.resize(128 + mip_size, 0xAB);
        data
    }

    #[test]
    fn parse_dds_dxt5_basic() {
        let data = make_test_dds(b"DXT5", 64, 64);
        let ctex = parse_dds(&data).expect("parse");
        assert_eq!(ctex.width, 64);
        assert_eq!(ctex.height, 64);
        assert_eq!(ctex.gpu_format, DdsGpuFormat::Dxt5);
        assert_eq!(ctex.mips.len(), 1);
        // 64x64 = 16x16 blocks * 16 bytes = 4096
        assert_eq!(ctex.mips[0].len(), 16 * 16 * 16);
    }

    #[test]
    fn parse_dds_dxt1_basic() {
        let data = make_test_dds(b"DXT1", 32, 32);
        let ctex = parse_dds(&data).expect("parse");
        assert_eq!(ctex.gpu_format, DdsGpuFormat::Dxt1);
        // 32x32 = 8x8 blocks * 8 bytes = 512
        assert_eq!(ctex.mips[0].len(), 8 * 8 * 8);
    }

    #[test]
    fn parse_dds_too_short() {
        assert!(parse_dds(&[0u8; 10]).is_err());
    }

    #[test]
    fn parse_dds_bad_magic() {
        let mut data = make_test_dds(b"DXT5", 16, 16);
        data[0..4].copy_from_slice(b"BAD ");
        assert!(parse_dds(&data).is_err());
    }

    #[test]
    fn parse_dds_unsupported_fourcc() {
        let data = make_test_dds(b"BC1 ", 16, 16);
        assert!(parse_dds(&data).is_err());
    }
}
