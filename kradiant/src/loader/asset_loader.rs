//! Game-asset virtual filesystem (loose + pk3) and editor-oriented resolving helpers.

pub use crate::assets::{
    AssetDb, AssetDbError, AssetDbOptions, AssetRoots, ResolvedAsset, normalize_asset_path,
    normalize_material_name, resolve_editor_image_name,
};
