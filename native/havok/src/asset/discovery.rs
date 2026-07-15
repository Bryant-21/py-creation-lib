/// File walking, role classification, and category assignment for Havok assets.
///
/// Port of `py_creation_lib/python/creation_lib/havok/discovery.py`.
use serde::Serialize;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Category patterns — checked in order, first match wins.
// Static string-based matching (no regex dep needed for these static prefixes).
// ---------------------------------------------------------------------------

/// Classify a file's category based on its relative path.
///
/// Normalises backslashes to forward slashes before matching.
pub fn classify_category(rel_path: &str) -> &'static str {
    let norm = rel_path.replace('\\', "/");
    let lc = norm.to_lowercase();

    // UniqueBehaviors/ at start or after /
    if lc.starts_with("uniquebehaviors/") || lc.contains("/uniquebehaviors/") {
        return "Weapon";
    }
    // BehaviorsUnique/ (DLC variant)
    if lc.contains("/behaviorsunique/") || lc.starts_with("behaviorsunique/") {
        return "Weapon";
    }
    // GenericBehaviors/
    if lc.starts_with("genericbehaviors/") || lc.contains("/genericbehaviors/") {
        return "Generic";
    }
    // Actors/Character/
    if lc.starts_with("actors/character/") || lc.contains("/actors/character/") {
        return "Character";
    }
    // Actors/Shared/
    if lc.starts_with("actors/shared/") || lc.contains("/actors/shared/") {
        return "ActorShared";
    }
    // Actors/Turret/
    if lc.starts_with("actors/turret/") || lc.contains("/actors/turret/") {
        return "Turret";
    }
    // Actors/PowerArmor/
    if lc.starts_with("actors/powerarmor/") || lc.contains("/actors/powerarmor/") {
        return "PowerArmor";
    }
    // Actors/ (catch-all for other actors)
    if lc.starts_with("actors/") || lc.contains("/actors/") {
        return "Creature";
    }
    // SetDressing/
    if lc.starts_with("setdressing/") || lc.contains("/setdressing/") {
        return "SetDressing";
    }
    // Effects/EffectBehaviors/
    if lc.contains("effects/effectbehaviors/") {
        return "Effect";
    }
    // Furniture/
    if lc.starts_with("furniture/") || lc.contains("/furniture/") {
        return "Furniture";
    }
    // Interface / Pipboy / Note01
    if lc.starts_with("interface/")
        || lc.contains("/interface/")
        || lc.starts_with("pipboy/")
        || lc.contains("/pipboy/")
        || lc.starts_with("note01/")
        || lc.contains("/note01/")
    {
        return "Interface";
    }
    // Architecture / Interiors
    if lc.starts_with("architecture/")
        || lc.contains("/architecture/")
        || lc.starts_with("interiors/")
        || lc.contains("/interiors/")
    {
        return "Architecture";
    }
    // AnimTextData/
    if lc.starts_with("animtextdata/") || lc.contains("/animtextdata/") {
        return "AnimGraph";
    }
    // Markers / Lights / Landscape
    if lc.starts_with("markers/")
        || lc.contains("/markers/")
        || lc.starts_with("lights/")
        || lc.contains("/lights/")
        || lc.starts_with("landscape/")
        || lc.contains("/landscape/")
    {
        return "Misc";
    }

    "Misc"
}

// ---------------------------------------------------------------------------
// Role classification
// ---------------------------------------------------------------------------

/// Classify a file's role in the Havok hierarchy based on path patterns.
///
/// `meshes_dir` is needed for the project-detection sibling check. Pass `None`
/// to skip that check (project will fall through to `"unknown"`).
pub fn classify_role(rel_path: &str, meshes_dir: Option<&Path>) -> &'static str {
    let norm = rel_path.replace('\\', "/");
    let lc = norm.to_lowercase();

    // Extension-based classification for Starfield native formats
    if lc.ends_with(".rig") {
        return "skeleton";
    }
    if lc.ends_with(".af") {
        return "animation";
    }
    if lc.ends_with(".agx") {
        return "behavior";
    }

    // Asset files
    if lc.ends_with(".nif")
        || lc.ends_with(".dds")
        || lc.ends_with(".bgsm")
        || lc.ends_with(".bgem")
    {
        return "asset";
    }

    // Skeleton HKT
    if lc.ends_with(".hkt") {
        return "skeleton";
    }

    // Skeleton HKX/XML in CharacterAssets — path must contain "characterassets/"
    // and basename must contain "skeleton"
    if lc.contains("/characterassets/") {
        let basename = norm.rsplit('/').next().unwrap_or("").to_lowercase();
        if basename.contains("skeleton") {
            return "skeleton";
        }
    }

    // Character definition in Characters/ directory
    if lc.contains("/characters/") {
        return "character";
    }

    // Behavior in Behaviors/ directory
    if lc.contains("/behaviors/") {
        return "behavior";
    }

    // Animation in Animations/ directory
    if lc.contains("/animations/") {
        return "animation";
    }

    // Project file: HKX/XML at same level as a Characters/ or Behaviors/ sibling dir
    let is_xml_or_hkx = lc.ends_with(".xml") || lc.ends_with(".hkx");
    if is_xml_or_hkx {
        if let Some(dir) = meshes_dir {
            let parent_rel = norm.rfind('/').map_or("", |i| &norm[..i]);
            let abs_parent = if parent_rel.is_empty() {
                dir.to_path_buf()
            } else {
                dir.join(parent_rel)
            };
            if abs_parent.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&abs_parent) {
                    for entry in entries.flatten() {
                        let name_lc = entry.file_name().to_string_lossy().to_lowercase();
                        if entry.path().is_dir()
                            && (name_lc == "characters" || name_lc == "behaviors")
                        {
                            return "project";
                        }
                    }
                }
            }
        }
    }

    // Fallback: txt and other asset extensions
    if lc.ends_with(".txt") {
        return "asset";
    }

    "unknown"
}

// ---------------------------------------------------------------------------
// File entry
// ---------------------------------------------------------------------------

/// A discovered file with its classification.
#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    #[serde(skip)]
    pub abs_path: PathBuf,
    /// Forward-slash relative path under meshes_dir.
    pub rel_path: String,
    /// project, character, skeleton, behavior, animation, asset
    pub role: String,
    /// Weapon, Generic, Character, Creature, etc.
    pub category: String,
    /// Extension without dot: xml, hkx, hkt, nif, dds, etc.
    pub file_type: String,
    /// True if .xml or .agx (no unpacking needed).
    pub is_xml: bool,
}

// ---------------------------------------------------------------------------
// Directory walker
// ---------------------------------------------------------------------------

/// The file extensions collected during discovery.
const HAVOK_EXTENSIONS: &[&str] = &[".xml", ".hkx", ".hkt", ".af", ".rig", ".agx"];
const ASSET_EXTENSIONS: &[&str] = &[".nif", ".dds", ".bgsm", ".bgem", ".txt"];

fn is_known_extension(lc_name: &str) -> bool {
    HAVOK_EXTENSIONS
        .iter()
        .chain(ASSET_EXTENSIONS.iter())
        .any(|ext| lc_name.ends_with(ext))
}

/// Walk `meshes_dir` and classify all Havok-related files.
///
/// Returns entries sorted by `rel_path`.
pub fn walk_meshes_dir(meshes_dir: &Path) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    walk_recursive(meshes_dir, meshes_dir, &mut entries);
    entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    entries
}

fn walk_recursive(meshes_dir: &Path, current: &Path, entries: &mut Vec<FileEntry>) {
    let Ok(read_dir) = std::fs::read_dir(current) else {
        return;
    };
    for item in read_dir.flatten() {
        let path = item.path();
        if path.is_dir() {
            walk_recursive(meshes_dir, &path, entries);
        } else if path.is_file() {
            let lc_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !is_known_extension(&lc_name) {
                continue;
            }
            let rel = path
                .strip_prefix(meshes_dir)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();

            let role = classify_role(&rel, Some(meshes_dir));
            if role == "unknown" {
                continue;
            }

            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            let category = classify_category(&rel).to_string();
            entries.push(FileEntry {
                abs_path: path,
                rel_path: rel.clone(),
                role: role.to_string(),
                category,
                file_type: ext.to_string(),
                is_xml: rel.ends_with(".xml") || rel.ends_with(".agx"),
            });
        }
    }
}
