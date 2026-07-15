use std::collections::HashMap;
use std::path::{Component, Path};

use serde::Deserialize;

const SCHEMA_VERSION: u32 = 1;
const VIRTUAL_BASE_SIGNATURES: [&str; 5] = ["ACTI", "MSTT", "SCOL", "TREE", "FLOR"];

#[derive(Clone, Debug, Deserialize)]
struct OverlayDocument {
    schema_version: u32,
    plugin_name: String,
    plugin_size: u64,
    plugin_mtime_ns: u64,
    entries: Vec<OverlayEntry>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct OverlayEntry {
    pub reference_form_id: u32,
    pub placed_base_form_id: u32,
    pub component_index: Option<u32>,
    pub component_base_form_id: Option<u32>,
    pub base_signature: String,
    pub lod_models: [Option<String>; 4],
    pub force_visible: bool,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct OverlayKey {
    reference_form_id: u32,
    placed_base_form_id: u32,
    component_index: Option<u32>,
    component_base_form_id: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ObjectLodOverlay {
    entries: HashMap<OverlayKey, OverlayEntry>,
}

impl ObjectLodOverlay {
    pub(super) fn load(path: &Path, plugin_path: &Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path).map_err(|error| {
            anyhow::anyhow!("read object LOD overlay {}: {error}", path.display())
        })?;
        let document: OverlayDocument = serde_json::from_slice(&bytes).map_err(|error| {
            anyhow::anyhow!("parse object LOD overlay {}: {error}", path.display())
        })?;
        if document.schema_version != SCHEMA_VERSION {
            anyhow::bail!(
                "object LOD overlay {} schema_version={} expected {}",
                path.display(),
                document.schema_version,
                SCHEMA_VERSION
            );
        }
        let expected_plugin = plugin_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("working plugin has no UTF-8 filename"))?;
        if !document.plugin_name.eq_ignore_ascii_case(expected_plugin) {
            anyhow::bail!(
                "object LOD overlay plugin {:?} does not match working plugin {:?}",
                document.plugin_name,
                expected_plugin
            );
        }
        let metadata = std::fs::metadata(plugin_path).map_err(|error| {
            anyhow::anyhow!(
                "read working plugin metadata {}: {error}",
                plugin_path.display()
            )
        })?;
        let plugin_mtime_ns = metadata
            .modified()
            .and_then(|modified| {
                modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(std::io::Error::other)
            })
            .and_then(|duration| u64::try_from(duration.as_nanos()).map_err(std::io::Error::other))
            .map_err(|error| {
                anyhow::anyhow!(
                    "read working plugin mtime {}: {error}",
                    plugin_path.display()
                )
            })?;
        if document.plugin_size != metadata.len() || document.plugin_mtime_ns != plugin_mtime_ns {
            anyhow::bail!(
                "object LOD overlay plugin fingerprint size={}/mtime_ns={} does not match working plugin size={}/mtime_ns={}",
                document.plugin_size,
                document.plugin_mtime_ns,
                metadata.len(),
                plugin_mtime_ns
            );
        }

        let mut entries = HashMap::with_capacity(document.entries.len());
        for mut entry in document.entries {
            validate_entry(&mut entry)?;
            let key = OverlayKey {
                reference_form_id: entry.reference_form_id,
                placed_base_form_id: entry.placed_base_form_id,
                component_index: entry.component_index,
                component_base_form_id: entry.component_base_form_id,
            };
            if entries.insert(key, entry).is_some() {
                anyhow::bail!(
                    "object LOD overlay has duplicate key ref={:08X} base={:08X} component={:?}/{:?}",
                    key.reference_form_id,
                    key.placed_base_form_id,
                    key.component_index,
                    key.component_base_form_id
                );
            }
        }
        Ok(Self { entries })
    }

    pub(super) fn parent(
        &self,
        reference_form_id: u32,
        placed_base_form_id: u32,
    ) -> Option<&OverlayEntry> {
        self.entries.get(&OverlayKey {
            reference_form_id,
            placed_base_form_id,
            component_index: None,
            component_base_form_id: None,
        })
    }

    pub(super) fn component(
        &self,
        reference_form_id: u32,
        placed_base_form_id: u32,
        component_index: u32,
        component_base_form_id: u32,
    ) -> Option<&OverlayEntry> {
        self.entries.get(&OverlayKey {
            reference_form_id,
            placed_base_form_id,
            component_index: Some(component_index),
            component_base_form_id: Some(component_base_form_id),
        })
    }

    #[cfg(test)]
    pub(super) fn from_entries_for_test(entries: Vec<OverlayEntry>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|entry| {
                    (
                        OverlayKey {
                            reference_form_id: entry.reference_form_id,
                            placed_base_form_id: entry.placed_base_form_id,
                            component_index: entry.component_index,
                            component_base_form_id: entry.component_base_form_id,
                        },
                        entry,
                    )
                })
                .collect(),
        }
    }
}

fn validate_entry(entry: &mut OverlayEntry) -> anyhow::Result<()> {
    if entry.reference_form_id == 0 || entry.placed_base_form_id == 0 {
        anyhow::bail!("object LOD overlay FormIDs must be nonzero");
    }
    if entry.component_index.is_some() != entry.component_base_form_id.is_some() {
        anyhow::bail!(
            "object LOD overlay component_index and component_base_form_id must appear together"
        );
    }
    entry.base_signature.make_ascii_uppercase();
    if !VIRTUAL_BASE_SIGNATURES.contains(&entry.base_signature.as_str()) {
        anyhow::bail!(
            "object LOD overlay signature {:?} is not a non-STAT LOD base",
            entry.base_signature
        );
    }
    for model in entry.lod_models.iter_mut().flatten() {
        *model = normalize_model_path(model)?;
    }
    if entry.lod_models.iter().all(Option::is_none) {
        anyhow::bail!("object LOD overlay entry has no model slots");
    }
    Ok(())
}

fn normalize_model_path(path: &str) -> anyhow::Result<String> {
    let normalized = path.trim().replace('/', "\\");
    let portable = normalized.replace('\\', "/");
    let candidate = Path::new(&portable);
    if normalized.is_empty()
        || normalized.contains('\0')
        || candidate.is_absolute()
        || candidate.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || !matches!(
            candidate.extension().and_then(|extension| extension.to_str()),
            Some(extension)
                if extension.eq_ignore_ascii_case("nif")
                    || extension.eq_ignore_ascii_case("dds")
        )
    {
        anyhow::bail!("unsafe object LOD overlay model path {path:?}");
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_overlay(
        temp: &tempfile::TempDir,
        mut body: serde_json::Value,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let plugin_path = temp.path().join("Output.esm");
        std::fs::write(&plugin_path, b"TES4").unwrap();
        let metadata = std::fs::metadata(&plugin_path).unwrap();
        let mtime_ns = u64::try_from(
            metadata
                .modified()
                .unwrap()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        )
        .unwrap();
        let object = body.as_object_mut().unwrap();
        object.insert("plugin_size".to_string(), metadata.len().into());
        object.insert("plugin_mtime_ns".to_string(), mtime_ns.into());
        let path = temp.path().join("overlay.json");
        std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
        (path, plugin_path)
    }

    #[test]
    fn loads_normalized_exact_entries() {
        let temp = tempfile::tempdir().unwrap();
        let (overlay_path, plugin_path) = write_overlay(
            &temp,
            serde_json::json!({
                "schema_version": 1,
                "plugin_name": "Output.esm",
                "entries": [{
                    "reference_form_id": 0x100,
                    "placed_base_form_id": 0x200,
                    "component_index": null,
                    "component_base_form_id": null,
                    "base_signature": "acti",
                    "lod_models": ["LOD/Test.nif", null, null, null],
                    "force_visible": true
                }]
            }),
        );
        let overlay = ObjectLodOverlay::load(&overlay_path, &plugin_path).unwrap();
        let entry = overlay.parent(0x100, 0x200).unwrap();
        assert_eq!(entry.base_signature, "ACTI");
        assert_eq!(entry.lod_models[0].as_deref(), Some("LOD\\Test.nif"));
        assert!(overlay.parent(0x101, 0x200).is_none());
    }

    #[test]
    fn rejects_stale_plugin_duplicate_and_unsafe_paths() {
        for (body, expected) in [
            (
                serde_json::json!({
                    "schema_version": 1,
                    "plugin_name": "Old.esm",
                    "entries": []
                }),
                "does not match",
            ),
            (
                serde_json::json!({
                    "schema_version": 1,
                    "plugin_name": "Output.esm",
                    "entries": [
                        {"reference_form_id":1,"placed_base_form_id":2,"component_index":null,"component_base_form_id":null,"base_signature":"ACTI","lod_models":["LOD\\A.nif",null,null,null],"force_visible":false},
                        {"reference_form_id":1,"placed_base_form_id":2,"component_index":null,"component_base_form_id":null,"base_signature":"ACTI","lod_models":["LOD\\B.nif",null,null,null],"force_visible":false}
                    ]
                }),
                "duplicate key",
            ),
            (
                serde_json::json!({
                    "schema_version": 1,
                    "plugin_name": "Output.esm",
                    "entries": [{"reference_form_id":1,"placed_base_form_id":2,"component_index":null,"component_base_form_id":null,"base_signature":"ACTI","lod_models":["..\\escape.nif",null,null,null],"force_visible":false}]
                }),
                "unsafe",
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let (path, plugin_path) = write_overlay(&temp, body);
            let error = ObjectLodOverlay::load(&path, &plugin_path).unwrap_err();
            assert!(error.to_string().contains(expected), "{error:#}");
        }
    }

    #[test]
    fn rejects_same_name_overlay_with_stale_plugin_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let (path, plugin_path) = write_overlay(
            &temp,
            serde_json::json!({
                "schema_version": 1,
                "plugin_name": "Output.esm",
                "entries": []
            }),
        );
        std::fs::write(&plugin_path, b"TES4 changed after overlay binding").unwrap();

        let error = ObjectLodOverlay::load(&path, &plugin_path).unwrap_err();
        assert!(error.to_string().contains("fingerprint"), "{error:#}");
    }
}
