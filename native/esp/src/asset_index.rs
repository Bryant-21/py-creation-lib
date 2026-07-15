//! Per-record asset path extraction for native plugin indices.

use super::*;
use smol_str::SmolStr;

const ASSET_PATH_EXTENSIONS: &[&str] = &[
    ".nif", ".egt", ".dds", ".bgsm", ".bgem", ".mat", ".hkx", ".hkt", ".wav", ".xwm", ".fuz",
];

pub(crate) fn extract_asset_paths(record: &ParsedRecord) -> Vec<AssetPathEntry> {
    let subrecords = effective_subrecords_for_record(record);
    extract_asset_paths_from_subrecords(record.signature.as_str(), &subrecords)
}

pub(crate) fn extract_asset_paths_from_subrecords(
    record_sig: &str,
    subrecords: &[ParsedSubrecord],
) -> Vec<AssetPathEntry> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for subrecord in subrecords {
        let sub_sig = subrecord.signature.as_str();
        let Some(default_kind) = asset_kind_for_subrecord(record_sig, sub_sig) else {
            continue;
        };
        let Some(path) = subrecord_as_path(subrecord) else {
            continue;
        };
        let kind = asset_kind_for_path(default_kind, path.as_str());
        let path = normalize_asset_path(kind.as_str(), path);
        let dedup_key = (kind.clone(), path.to_ascii_lowercase());
        if !seen.insert(dedup_key) {
            continue;
        }
        out.push(AssetPathEntry {
            kind,
            path,
            source_subrecord_sig: SmolStr::new(sub_sig),
        });
    }
    out
}

pub(crate) fn asset_kind_for_subrecord(record_sig: &str, sub_sig: &str) -> Option<SmolStr> {
    match (record_sig, sub_sig) {
        (_, "MODL" | "MOD2" | "MOD3" | "MOD4" | "MOD5") => Some(SmolStr::new("nif")),
        (_, "ICON" | "MICO") => Some(SmolStr::new("texture")),
        ("TXST", "TX00" | "TX01" | "TX02" | "TX03" | "TX04" | "TX05" | "TX06" | "TX07") => {
            Some(SmolStr::new("texture"))
        }
        ("MSWP", "BNAM" | "MNAM") => Some(SmolStr::new("material")),
        ("IDLE", "BNAM") => Some(SmolStr::new("behavior")),
        ("RACE", "ANAM") => Some(SmolStr::new("behavior")),
        ("SNDR", "ANAM" | "FNAM") => Some(SmolStr::new("sound")),
        ("SOUN", "FNAM") => Some(SmolStr::new("sound")),
        ("MUSC" | "MUST", "ANAM" | "FNAM") => Some(SmolStr::new("sound")),
        _ => None,
    }
}

fn asset_kind_for_path(default_kind: SmolStr, path: &str) -> SmolStr {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".hkx") || lower.ends_with(".hkt") {
        return SmolStr::new("behavior");
    }
    if lower.ends_with(".nif") {
        return SmolStr::new("nif");
    }
    default_kind
}

fn subrecord_as_path(subrecord: &ParsedSubrecord) -> Option<String> {
    let bytes = subrecord.data.as_ref();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let value = std::str::from_utf8(&bytes[..end]).ok()?.trim();
    if value.is_empty() || !looks_like_asset_path(value) {
        return None;
    }
    Some(value.replace('\\', "/"))
}

fn looks_like_asset_path(value: &str) -> bool {
    if value.chars().any(char::is_control) {
        return false;
    }
    if value.contains('\\') || value.contains('/') {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    ASSET_PATH_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

fn normalize_asset_path(kind: &str, path: String) -> String {
    if kind == "sound" && path.to_ascii_lowercase().starts_with("data/") {
        path[5..].to_string()
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::{asset_kind_for_path, asset_kind_for_subrecord};
    use smol_str::SmolStr;

    #[test]
    fn sound_asset_kind_matches_all_supported_audio_records() {
        for (record_sig, sub_sig) in [
            ("SNDR", "ANAM"),
            ("SNDR", "FNAM"),
            ("SOUN", "FNAM"),
            ("MUSC", "ANAM"),
            ("MUSC", "FNAM"),
            ("MUST", "ANAM"),
            ("MUST", "FNAM"),
        ] {
            assert_eq!(
                asset_kind_for_subrecord(record_sig, sub_sig).as_deref(),
                Some("sound"),
                "{record_sig}.{sub_sig} should collect as sound",
            );
        }
    }

    #[test]
    fn asset_kind_uses_havok_extension_over_subrecord_default() {
        assert_eq!(
            asset_kind_for_path(
                SmolStr::new("nif"),
                "Actors/Snallygaster/SnallygasterProject.hkx"
            )
            .as_str(),
            "behavior"
        );
    }

    #[test]
    fn asset_kind_uses_nif_extension_over_subrecord_default() {
        assert_eq!(
            asset_kind_for_path(
                SmolStr::new("behavior"),
                "Actors/Snallygaster/CharacterAssets/Skeleton.nif"
            )
            .as_str(),
            "nif"
        );
    }
}
