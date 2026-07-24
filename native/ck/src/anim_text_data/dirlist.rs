//! CK-parity `dirlist.txt` emission. The FO4 runtime treats dirlist.txt purely as a
//! skip-sentinel during category-dir enumeration (it is never read as a manifest), but
//! the CK ships one per category dir and we match its output byte-for-byte.

use std::path::Path;

/// The 7 category dirs FO4 ships dirlists for. ClipGeneratorData deliberately has none.
const DIRLIST_BUCKETS: &[&str] = &[
    "AnimationFileData",
    "AnimationOffsets",
    "AnimationSpeedInfo",
    "AnimationStanceData",
    "AnimEventInfo",
    "DynamicIdleData",
    "SyncAnimData",
];

/// CK dirlist body: entries sorted by uppercase-ASCII ordinal, CRLF-terminated
/// (including the final line).
pub fn build_dirlist(names: &[String]) -> Vec<u8> {
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort_by(|a, b| a.to_ascii_uppercase().cmp(&b.to_ascii_uppercase()));
    let mut out = Vec::with_capacity(names.iter().map(|n| n.len() + 2).sum());
    for name in sorted {
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Write `dirlist.txt` into each shipped category dir under
/// `<out_meshes_root>/AnimTextData/`, listing that dir's data files (never itself).
/// Dirs that are absent or empty get no dirlist. Returns the number written.
pub fn emit_dirlists(out_meshes_root: &Path) -> Result<u32, String> {
    let atd = out_meshes_root.join("AnimTextData");
    let mut written = 0u32;
    for bucket in DIRLIST_BUCKETS {
        let dir = atd.join(bucket);
        if !dir.is_dir() {
            continue;
        }
        let mut names = Vec::new();
        let entries = std::fs::read_dir(&dir)
            .map_err(|error| format!("failed to list {}: {error}", dir.display()))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("failed to list {}: {error}", dir.display()))?;
            if !entry.path().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case("dirlist.txt") {
                continue;
            }
            names.push(name);
        }
        if names.is_empty() {
            continue;
        }
        let path = dir.join("dirlist.txt");
        std::fs::write(&path, build_dirlist(&names))
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn oracle(path: &str) -> Option<Vec<u8>> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../extracted/fo4/meshes/animtextdata")
            .join(path);
        if !path.is_file() {
            eprintln!("skipping: oracle missing {}", path.display());
            return None;
        }
        Some(std::fs::read(path).unwrap())
    }

    #[test]
    fn dirlist_rebuilds_all_seven_vanilla_dirlists_byte_exact() {
        // Covers every shipped dirlist, including the ordering stress case
        // (animationfiledata: 2777 numeric + 217 mixed-case named entries with the
        // uppercase-collation edge pairs, e.g. yaoguai.txt before _sharedproject.txt).
        let buckets = [
            r"animationfiledata\dirlist.txt",
            r"animationoffsets\dirlist.txt",
            r"animationspeedinfo\dirlist.txt",
            r"animationstancedata\dirlist.txt",
            r"animeventinfo\dirlist.txt",
            r"dynamicidledata\dirlist.txt",
            r"syncanimdata\dirlist.txt",
        ];
        for bucket in buckets {
            let Some(raw) = oracle(bucket) else { return };
            let mut names: Vec<String> = String::from_utf8(raw.clone())
                .unwrap()
                .split("\r\n")
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
            names.reverse(); // destroy the order; the builder must restore it
            assert_eq!(build_dirlist(&names), raw, "mismatch for {bucket}");
        }
    }

    #[test]
    fn emit_dirlists_writes_seven_buckets_and_skips_clipgeneratordata_and_self() {
        let out = tempfile::tempdir().unwrap();
        let atd = out.path().join("AnimTextData");
        for (bucket, file) in [
            ("AnimationFileData", "123.txt"),
            ("ClipGeneratorData", "42.txt"),
            ("SyncAnimData", "ResolvedSyncAnimDataFoo.txt"),
        ] {
            let dir = atd.join(bucket);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(file), b"x").unwrap();
        }
        // pre-existing dirlist must be excluded from its own listing on re-run
        std::fs::write(atd.join("SyncAnimData").join("dirlist.txt"), b"stale").unwrap();

        let written = emit_dirlists(out.path()).unwrap();
        assert_eq!(written, 2); // AnimationFileData + SyncAnimData; ClipGeneratorData skipped
        assert_eq!(
            std::fs::read(atd.join("AnimationFileData").join("dirlist.txt")).unwrap(),
            b"123.txt\r\n"
        );
        assert_eq!(
            std::fs::read(atd.join("SyncAnimData").join("dirlist.txt")).unwrap(),
            b"ResolvedSyncAnimDataFoo.txt\r\n"
        );
        assert!(!atd.join("ClipGeneratorData").join("dirlist.txt").exists());
    }
}
