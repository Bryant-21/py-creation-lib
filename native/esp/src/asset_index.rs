//! Per-record asset path extraction for native plugin indices.

use super::*;
use smol_str::SmolStr;

const ASSET_PATH_EXTENSIONS: &[&str] = &[
    ".nif", ".egt", ".dds", ".bgsm", ".bgem", ".mat", ".kf", ".hkx", ".hkt", ".wav", ".xwm", ".fuz",
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
        if record_sig == "IDLE" && sub_sig == "MODL" && !has_known_asset_extension(path.as_str()) {
            continue;
        }
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
        // Weather cloud layers: FO3/FNV name the first four, every later game
        // numbers all thirty-two `<n>0TX`.  Record-scoped so it cannot capture
        // RACE/IDLE ANAM-BNAM behaviors or MSWP BNAM materials.
        ("WTHR", "DNAM" | "CNAM" | "ANAM" | "BNAM") => Some(SmolStr::new("texture")),
        ("WTHR", sub) if sub.len() == 4 && sub.ends_with("0TX") => Some(SmolStr::new("texture")),
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
    if lower.ends_with(".kf") {
        return SmolStr::new("kf_animation");
    }
    if lower.ends_with(".hkx") || lower.ends_with(".hkt") {
        return SmolStr::new("behavior");
    }
    if lower.ends_with(".nif") {
        return SmolStr::new("nif");
    }
    default_kind
}

pub(crate) fn subrecord_as_path(subrecord: &ParsedSubrecord) -> Option<String> {
    let bytes = subrecord.data.as_ref();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let value = std::str::from_utf8(&bytes[..end]).ok()?.trim();
    if value.is_empty() || !looks_like_asset_path(value) {
        return None;
    }
    Some(value.replace('\\', "/"))
}

fn has_known_asset_extension(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    ASSET_PATH_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

fn looks_like_asset_path(value: &str) -> bool {
    if value.chars().any(char::is_control) {
        return false;
    }
    if value.contains('\\') || value.contains('/') {
        return true;
    }
    has_known_asset_extension(value)
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
    use super::ParsedSubrecord;
    use super::{
        asset_kind_for_path, asset_kind_for_subrecord, extract_asset_paths_from_subrecords,
    };
    use bytes::Bytes;
    use smol_str::SmolStr;

    fn path_subrecord(signature: &str, path: &str) -> ParsedSubrecord {
        let mut data = path.as_bytes().to_vec();
        data.push(0);
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

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
    fn weather_cloud_layers_collect_as_textures_in_both_naming_schemes() {
        for sub_sig in ["DNAM", "CNAM", "ANAM", "BNAM", "00TX", "30TX", "O0TX"] {
            assert_eq!(
                asset_kind_for_subrecord("WTHR", sub_sig).as_deref(),
                Some("texture"),
                "WTHR.{sub_sig} should collect as texture",
            );
        }
    }

    #[test]
    fn weather_cloud_layer_names_stay_scoped_to_weather_records() {
        // The same 4CCs mean behavior or material elsewhere.
        assert_eq!(
            asset_kind_for_subrecord("RACE", "ANAM").as_deref(),
            Some("behavior")
        );
        assert_eq!(
            asset_kind_for_subrecord("IDLE", "BNAM").as_deref(),
            Some("behavior")
        );
        assert_eq!(
            asset_kind_for_subrecord("MSWP", "BNAM").as_deref(),
            Some("material")
        );
        assert_eq!(asset_kind_for_subrecord("STAT", "DNAM"), None);
        assert_eq!(asset_kind_for_subrecord("STAT", "00TX"), None);
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
    fn fnv_idle_modl_indexes_human_kf_as_animation() {
        let assets = extract_asset_paths_from_subrecords(
            "IDLE",
            &[path_subrecord(
                "MODL",
                "Characters\\_Male\\IdleAnims\\dlcpittweldinghighidle.kf",
            )],
        );

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].kind.as_str(), "kf_animation");
        assert_eq!(
            assets[0].path,
            "Characters/_Male/IdleAnims/dlcpittweldinghighidle.kf"
        );
        assert_eq!(assets[0].source_subrecord_sig.as_str(), "MODL");
    }

    #[test]
    fn fnv_idle_modl_indexes_gecko_kf_as_animation() {
        let assets = extract_asset_paths_from_subrecords(
            "IDLE",
            &[path_subrecord(
                "MODL",
                "creatures\\NVGecko\\IdleAnims\\MT_SpecialIdle_EyeLickLeft.kf",
            )],
        );

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].kind.as_str(), "kf_animation");
        assert_eq!(
            assets[0].path,
            "creatures/NVGecko/IdleAnims/MT_SpecialIdle_EyeLickLeft.kf"
        );
    }

    #[test]
    fn fnv_idle_modl_directory_is_topology_not_an_asset() {
        let assets = extract_asset_paths_from_subrecords(
            "IDLE",
            &[path_subrecord("MODL", "creatures\\NVGecko\\IdleAnims")],
        );

        assert!(assets.is_empty());
    }

    #[test]
    fn idle_bnam_hkx_stays_behavior() {
        assert_eq!(
            asset_kind_for_path(
                asset_kind_for_subrecord("IDLE", "BNAM").unwrap(),
                "Actors/Example/Behaviors/Example.hkx",
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
