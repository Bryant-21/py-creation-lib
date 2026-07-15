use crate::model::WorldSession;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AssetCacheKey {
    pub normalized_path: String,
    pub source_fingerprint: String,
}

impl AssetCacheKey {
    pub fn new(path: &str, source_fingerprint: &str) -> Self {
        Self {
            normalized_path: normalize_asset_path(path),
            source_fingerprint: source_fingerprint.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetResolution {
    pub requested_path: String,
    pub resolved_path: Option<String>,
    pub source_fingerprint: String,
    pub missing: bool,
}

pub fn normalize_asset_path(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

pub fn resolve_model_path(session: &WorldSession, model_path: &str) -> AssetResolution {
    resolve_asset_path(session, model_path, "model")
}

pub fn resolve_material_path(session: &WorldSession, material_path: &str) -> AssetResolution {
    resolve_asset_path(session, material_path, "material")
}

pub fn resolve_texture_path(session: &WorldSession, texture_path: &str) -> AssetResolution {
    resolve_asset_path(session, texture_path, "texture")
}

fn resolve_asset_path(session: &WorldSession, asset_path: &str, kind: &str) -> AssetResolution {
    let normalized = normalize_asset_path(asset_path);
    for data_path in &session.data_paths {
        let candidate = PathBuf::from(data_path).join(Path::new(&normalized));
        if candidate.is_file() {
            return AssetResolution {
                requested_path: asset_path.to_string(),
                resolved_path: Some(candidate.to_string_lossy().to_string()),
                source_fingerprint: format!("loose:{data_path}"),
                missing: false,
            };
        }
    }

    for archive_path in &session.archive_paths {
        if Path::new(archive_path).is_file() {
            return AssetResolution {
                requested_path: asset_path.to_string(),
                resolved_path: None,
                source_fingerprint: format!("archive:{archive_path}:{kind}"),
                missing: false,
            };
        }
    }

    AssetResolution {
        requested_path: asset_path.to_string(),
        resolved_path: None,
        source_fingerprint: "missing".to_string(),
        missing: true,
    }
}
