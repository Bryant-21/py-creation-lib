//! AnimTextData `SubgraphIdentifier` generation (CK-free).
//!
//! FO4's `Data\Meshes\AnimTextData\` cache is normally produced by
//! `CreationKit.exe -GenerateAnimInfo`. FO76 source has none, so toolkit users
//! converting content would otherwise need CK. This module reproduces the
//! filename ids the engine computes at runtime, so we can name generated bucket
//! files such that the engine finds them.
//!
//! # Reverse-engineered recipe (FO4 1.10.155, see `docs/re/animtextdata_generation.md`)
//!
//! Each bucket file is named `"<id>.txt"` where `id` is a `SubgraphIdentifier`
//! (u64) the engine formats with `%llu` and opens directly (load-bearing — no
//! directory enumeration). It is built by `BSSubBehaviorUtils::CreateID`:
//!
//! ```text
//! id_u64 = crc32(name) | (crc32(JoinAnimationPaths(sapt_chain, '|')) << 32)
//! ```
//!
//! - `name` = the **core behavior** `.hkx` path, **lowercased** → low 32 bits.
//!   Name-only buckets (`AnimEventInfo`, `ClipGeneratorData`) use just this with
//!   the high word = 0, so their filenames look "32-bit".
//! - `sapt_chain` = the subgraph's animation-path chain `[self, parent, ...]`
//!   (`SAPT` subrecords from the RACE record). A lone path (no inheritance) is
//!   hashed **verbatim**; an inherited chain is **lowercased**, self-first,
//!   joined with `'|'` → high 32 bits.
//!
//! The CRC is `BSCRC32` = standard reflected CRC32 (poly `0xEDB88320`, init 0,
//! no final XOR) — identical to `bsarchive`'s `crc32` (kept local here to avoid
//! a cross-crate private-fn dependency).

/// Reflected CRC32 lookup table (poly 0xEDB88320).
const CRC32_LUT: [u32; 256] = build_lut();

const fn build_lut() -> [u32; 256] {
    let mut lut = [0u32; 256];
    let mut n = 0usize;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        lut[n] = c;
        n += 1;
    }
    lut
}

/// `BSCRC32::GenerateCRC` with `seed = 0` over raw bytes (caller controls case).
fn bs_crc32(s: &str) -> u32 {
    let mut crc: u32 = 0;
    for &b in s.as_bytes() {
        crc = (crc >> 8) ^ CRC32_LUT[((crc ^ b as u32) & 0xFF) as usize];
    }
    crc
}

/// Name-only id (32-bit, high word 0): `AnimEventInfo` / `ClipGeneratorData`.
/// `behavior_path` e.g. `Actors\Snallygaster\Behaviors\SnallygasterCoreBehavior.hkx`.
pub fn name_id(behavior_path: &str) -> u32 {
    bs_crc32(&behavior_path.to_ascii_lowercase())
}

/// High word: `crc32(JoinAnimationPaths(sapt_chain, '|'))`.
/// A single path is hashed verbatim; an inherited chain is lowercased, self-first.
fn sapt_chain_crc(sapt_chain: &[&str]) -> u32 {
    match sapt_chain {
        [] => 0,
        [only] => bs_crc32(only),
        chain => {
            let joined = chain
                .iter()
                .map(|p| p.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("|");
            bs_crc32(&joined)
        }
    }
}

/// Full 64-bit `SubgraphIdentifier` for the asset buckets
/// (`AnimationFileData` / `AnimationOffsets` / `AnimationSpeedInfo` /
/// `AnimationStanceData` / `DynamicIdleData`).
///
/// `core_behavior_path` is the subgraph's core behavior `.hkx`; `sapt_chain` is
/// the `SAPT` path chain (self first, then inherited parents).
pub fn subgraph_id(core_behavior_path: &str, sapt_chain: &[&str]) -> u64 {
    let low = bs_crc32(&core_behavior_path.to_ascii_lowercase()) as u64;
    let high = sapt_chain_crc(sapt_chain) as u64;
    low | (high << 32)
}

/// `"<id>.txt"` for a name-only bucket.
pub fn name_filename(behavior_path: &str) -> String {
    format!("{}.txt", name_id(behavior_path))
}

/// `"<id>.txt"` for an asset bucket.
pub fn subgraph_filename(core_behavior_path: &str, sapt_chain: &[&str]) -> String {
    format!("{}.txt", subgraph_id(core_behavior_path, sapt_chain))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ground truth: CK-generated AnimTextData for the Snallygaster fan mod
    // (refs/fanmods/Snallygaster). These filenames are what the engine opens.
    const CORE: &str = r"Actors\Snallygaster\Behaviors\SnallygasterCoreBehavior.hkx";
    const ROOT: &str = r"Actors\Snallygaster\Behaviors\SnallygasterRootBehavior.hkx";
    const BASE: &str = r"Actors\Snallygaster\Animations";

    #[test]
    fn name_only_ids_match_ck() {
        // AnimEventInfo/<id>.txt
        assert_eq!(name_id(ROOT), 133025100);
        assert_eq!(name_id(CORE), 4152054059);
    }

    #[test]
    fn base_subgraph_id_matches_ck() {
        // AnimationFileData/16837539554263781675.txt — lone SAPT, verbatim.
        assert_eq!(subgraph_id(CORE, &[BASE]), 16837539554263781675);
    }

    #[test]
    fn inherited_subgraph_ids_match_ck() {
        // Injured variants: chain = [self, base], lowercased.
        assert_eq!(
            subgraph_id(CORE, &[&format!(r"{BASE}\Injured\RightLeg"), BASE]),
            3419542945344999723
        );
        assert_eq!(
            subgraph_id(CORE, &[&format!(r"{BASE}\Injured\LeftLeg"), BASE]),
            9947826212300345643
        );
        // Note: this subgraph's SAPT is authored lowercase in the ESP.
        assert_eq!(
            subgraph_id(CORE, &[&format!(r"{BASE}\injured\boothlegs"), BASE]),
            3632382008203366699
        );
    }

    #[test]
    fn name_id_is_case_insensitive() {
        assert_eq!(name_id(CORE), name_id(&CORE.to_uppercase()));
    }

    #[test]
    fn filename_formatting() {
        assert_eq!(name_filename(ROOT), "133025100.txt");
        assert_eq!(subgraph_filename(CORE, &[BASE]), "16837539554263781675.txt");
    }
}
