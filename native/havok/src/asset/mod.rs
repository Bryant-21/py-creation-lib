pub mod classxml;
pub mod discovery;
pub mod manifest;

pub use classxml::parse_patches;
pub use discovery::{FileEntry, classify_category, classify_role, walk_meshes_dir};
pub use manifest::{ManifestData, ManifestDep, ManifestFileEntry, build_manifests};
