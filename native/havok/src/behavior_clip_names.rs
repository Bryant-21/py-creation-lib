//! Collect hkbClipGenerator animationName values from behavior packfiles.
use std::collections::HashSet;
use std::path::Path;

use crate::hkx::read_packfile;
use crate::hkx::types::HkxValue;

/// Extract unique animation names from `hkbClipGenerator.animationName` in a single
/// behavior `.hkx` file.
pub fn collect_behavior_clip_names_from_file(path: &Path) -> HashSet<String> {
    let mut clip_names = HashSet::new();
    let Ok(data) = std::fs::read(path) else {
        return clip_names;
    };
    let Ok(hkx) = read_packfile(&data) else {
        return clip_names;
    };
    for obj in hkx.objects() {
        if obj.class_name != "hkbClipGenerator" {
            continue;
        }
        for m in &obj.members {
            if m.name != "animationName" {
                continue;
            }
            if let HkxValue::String { value, .. } = &m.value {
                if !value.is_empty() {
                    clip_names.insert(value.clone());
                }
            }
        }
    }
    clip_names
}

/// Extract unique animation names from `hkbClipGenerator.animationName` across
/// all `.hkx` files in `behavior_dir`.
pub fn collect_behavior_clip_names_from_dir(behavior_dir: &Path) -> HashSet<String> {
    let mut clip_names = HashSet::new();
    let Ok(entries) = std::fs::read_dir(behavior_dir) else {
        return clip_names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("hkx"))
            .unwrap_or(false)
        {
            continue;
        }
        clip_names.extend(collect_behavior_clip_names_from_file(&path));
    }
    clip_names
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::hkx::descriptors::DescriptorRegistry;
    use crate::hkx::{HkxFile, HkxMember, HkxObject, write_hkx};

    use super::*;

    fn behavior_clip_test_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "havok-behavior-clip-names-{}-{nonce}",
            std::process::id()
        ))
    }

    fn write_behavior(path: &Path, animation_name: &str) {
        let hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![HkxObject {
                name: Some("#0001".to_string()),
                offset: 0,
                signature: 0,
                class_name: "hkbClipGenerator".to_string(),
                members: vec![HkxMember {
                    name: "animationName".to_string(),
                    value: HkxValue::String {
                        value: animation_name.to_string(),
                        is_null: false,
                    },
                }],
            }],
        );
        let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
        std::fs::write(path, write_hkx(&hkx, &mut registry)).unwrap();
    }

    #[test]
    fn behavior_clip_names_collects_unique_names_from_directory() {
        let dir = behavior_clip_test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        write_behavior(&dir.join("first.hkx"), "Animations\\Idle.hkt");
        write_behavior(&dir.join("second.HKX"), "Animations\\Attack.hkt");
        std::fs::write(dir.join("ignored.txt"), b"not hkx").unwrap();

        let names = collect_behavior_clip_names_from_dir(&dir);

        assert_eq!(
            names,
            HashSet::from([
                "Animations\\Attack.hkt".to_string(),
                "Animations\\Idle.hkt".to_string(),
            ])
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
