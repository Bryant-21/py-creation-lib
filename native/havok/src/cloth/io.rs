use std::collections::BTreeMap;

use crate::error::HavokResult;
use crate::hkx::{self, HkxFile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClothClassInventoryEntry {
    pub class_name: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClothMetadata {
    pub object_count: usize,
    pub class_inventory: Vec<ClothClassInventoryEntry>,
    pub has_cloth_data: bool,
    pub has_setup_data: bool,
    pub has_runtime_data: bool,
}

pub fn cloth_metadata_from_blob(blob: &[u8]) -> HavokResult<ClothMetadata> {
    let hkx = hkx::read_packfile(blob)?;
    let mut counts = BTreeMap::<String, usize>::new();
    for object in hkx.objects() {
        *counts.entry(object.class_name.clone()).or_insert(0) += 1;
    }
    let class_inventory = counts
        .into_iter()
        .map(|(class_name, count)| ClothClassInventoryEntry { class_name, count })
        .collect::<Vec<_>>();
    let has_class = |name: &str| class_inventory.iter().any(|entry| entry.class_name == name);
    let has_setup_data = class_inventory
        .iter()
        .any(|entry| entry.class_name.contains("Setup"));
    let has_runtime_data = class_inventory
        .iter()
        .any(|entry| entry.class_name.starts_with("hcl"));

    let has_cloth_data = has_class("hclClothData");

    Ok(ClothMetadata {
        object_count: hkx.objects().len(),
        class_inventory,
        has_cloth_data,
        has_setup_data,
        has_runtime_data,
    })
}

pub fn validate_cloth_blob(blob: &[u8]) -> HavokResult<serde_json::Value> {
    let hkx = hkx::read_packfile(blob)?;
    let cloth_data = super::runtime::ClothData::from_hkx_file(&hkx);
    let result = super::validate::validate_cloth_data(cloth_data.as_ref());
    Ok(result.to_summary())
}

pub fn load_cloth_hkx(blob: &[u8]) -> HavokResult<HkxFile> {
    hkx::read_packfile(blob)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    #[test]
    fn load_cloth_hkx_from_blob_returns_parsed_object_graph() {
        let path = repo_path("native/havok/tests/fixtures/skeleton.hkx");
        let blob = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));

        let hkx = load_cloth_hkx(&blob).expect("load HKX from raw blob");

        assert!(!hkx.objects().is_empty());
    }
}
