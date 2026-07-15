/// Manifest assembly — groups Havok files into conversion-ready bundles.
///
/// Port of `py_creation_lib/python/creation_lib/havok/manifest.py`.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::animation::parsers::CharacterRecord;
use crate::asset::discovery::FileEntry;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A file in a manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFileEntry {
    pub file_path: String,
    pub file_type: String,
    pub role: String,
    /// "owned" or "shared_ref"
    pub ref_type: String,
    pub file_size: u64,
}

/// A dependency on another manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestDep {
    /// Manifest ID this depends on.
    pub depends_on: String,
    /// skeleton, shared_behavior, base_actor
    pub dep_type: String,
}

/// A complete asset manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestData {
    pub id: String,
    pub name: String,
    /// actor, weapon_fx, generic_fx, dlc_fx, furniture, effect
    pub manifest_type: String,
    pub source: String,
    pub project_id: String,
    pub files: Vec<ManifestFileEntry>,
    pub dependencies: Vec<ManifestDep>,
    pub file_count: usize,
    pub total_size: u64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the manifest root directory from a file's relative path.
///
/// Returns `None` for paths that don't map to a known manifest root.
fn infer_manifest_root(rel_path: &str) -> Option<String> {
    let norm = rel_path.replace('\\', "/");
    let parts: Vec<&str> = norm.split('/').collect();

    if parts.len() < 2 {
        return None;
    }

    // UniqueBehaviors/X/... or GenericBehaviors/X/...
    for prefix in &["UniqueBehaviors", "GenericBehaviors"] {
        if parts[0].eq_ignore_ascii_case(prefix) && parts.len() >= 2 {
            return Some(format!("{}/{}", parts[0], parts[1]));
        }
    }

    // DLCn*/BehaviorsUnique/X/...
    let p0_lc = parts[0].to_lowercase();
    if p0_lc.starts_with("dlc")
        && parts.len() >= 3
        && parts[1].eq_ignore_ascii_case("behaviorsunique")
    {
        return Some(format!("{}/{}/{}", parts[0], parts[1], parts[2]));
    }

    // Actors/X/...
    if parts[0].eq_ignore_ascii_case("actors") && parts.len() >= 2 {
        return Some(format!("{}/{}", parts[0], parts[1]));
    }

    // Furniture/X/..., SetDressing/X/..., Architecture/X/..., Interface/X/...
    for prefix in &["Furniture", "SetDressing", "Architecture", "Interface"] {
        if parts[0].eq_ignore_ascii_case(prefix) && parts.len() >= 2 {
            return Some(format!("{}/{}", parts[0], parts[1]));
        }
    }

    // Effects/EffectBehaviors/X/...
    if parts[0].eq_ignore_ascii_case("effects")
        && parts.len() >= 3
        && parts[1].eq_ignore_ascii_case("effectbehaviors")
    {
        return Some(format!("{}/{}/{}", parts[0], parts[1], parts[2]));
    }

    None
}

fn classify_manifest_type(root: &str, category: &str) -> &'static str {
    let lower = root.to_lowercase();
    if lower.starts_with("actors/") {
        return "actor";
    }
    if lower.starts_with("uniquebehaviors/") {
        return "weapon_fx";
    }
    if lower.starts_with("genericbehaviors/") {
        return "generic_fx";
    }
    if lower.contains("behaviorsunique") {
        return "dlc_fx";
    }
    if category == "Furniture" {
        return "furniture";
    }
    if category == "Effect" {
        return "effect";
    }
    "generic_fx"
}

fn display_name(root: &str) -> String {
    let norm = root.replace('\\', "/");
    norm.rsplit('/').next().unwrap_or(root).to_string()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build manifests by grouping discovered files by their root directory.
///
/// `source` is the game source identifier (fo4, fo76, starfield).
/// `character_data` maps a character file's `rel_path` to its parsed `CharacterRecord`;
/// used to emit `ManifestDep(dep_type="skeleton")` entries for cross-manifest skeleton refs.
pub fn build_manifests(
    entries: &[FileEntry],
    character_data: &HashMap<String, CharacterRecord>,
    source: &str,
) -> Vec<ManifestData> {
    // Group entries by manifest root
    let mut groups: HashMap<String, Vec<&FileEntry>> = HashMap::new();
    for e in entries {
        if let Some(root) = infer_manifest_root(&e.rel_path) {
            groups.entry(root).or_default().push(e);
        }
    }

    let mut manifests: Vec<ManifestData> = Vec::new();
    let mut sorted_roots: Vec<String> = groups.keys().cloned().collect();
    sorted_roots.sort();

    for root in sorted_roots {
        let group_entries = &groups[&root];
        let category = group_entries
            .first()
            .map(|e| e.category.as_str())
            .unwrap_or("Misc");

        let manifest_id = format!("{}/{}", source, root);
        let mut m = ManifestData {
            id: manifest_id,
            name: display_name(&root),
            manifest_type: classify_manifest_type(&root, category).to_string(),
            source: source.to_string(),
            project_id: String::new(),
            files: Vec::new(),
            dependencies: Vec::new(),
            file_count: 0,
            total_size: 0,
        };

        // Find project entry
        for e in group_entries.iter() {
            if e.role == "project" {
                let stem = e
                    .rel_path
                    .rsplit('.')
                    .nth(1)
                    .map_or(e.rel_path.as_str(), |_| {
                        let dot = e.rel_path.rfind('.').unwrap_or(e.rel_path.len());
                        &e.rel_path[..dot]
                    });
                m.project_id = format!("{}/{}", source, stem);
                break;
            }
        }

        // Add files
        let mut total_size: u64 = 0;
        for e in group_entries.iter() {
            let size = if e.abs_path.exists() {
                e.abs_path.metadata().map(|md| md.len()).unwrap_or(0)
            } else {
                0
            };
            m.files.push(ManifestFileEntry {
                file_path: e.rel_path.clone(),
                file_type: e.file_type.clone(),
                role: e.role.clone(),
                ref_type: "owned".to_string(),
                file_size: size,
            });
            total_size += size;
        }
        m.file_count = m.files.len();
        m.total_size = total_size;

        // Emit skeleton dependencies from character data (port of manifest.py:165-182)
        for e in group_entries.iter() {
            if e.role == "character" {
                if let Some(cdata) = character_data.get(&e.rel_path) {
                    let rig = &cdata.rig_name;
                    if !rig.is_empty() && rig.contains("..") {
                        let mut rig_norm = rig.replace('\\', "/");
                        while rig_norm.starts_with("../") {
                            rig_norm = rig_norm[3..].to_string();
                        }
                        if let Some(rig_root) = infer_manifest_root(&rig_norm) {
                            m.dependencies.push(ManifestDep {
                                depends_on: format!("{}/{}", source, rig_root),
                                dep_type: "skeleton".to_string(),
                            });
                        }
                    }
                }
            }
        }

        manifests.push(m);
    }

    manifests
}
