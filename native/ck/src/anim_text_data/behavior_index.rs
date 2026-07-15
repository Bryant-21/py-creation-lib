//! Behavior index for AnimTextData generation (CK-free).
//!
//! Resolves a subgraph's referenced animation files: parse the behavior graph for
//! its `hkbClipGenerator` clip leaves, then resolve each leaf through the subgraph's
//! `SAPT` directory chain by **filesystem existence** (first directory in the chain
//! that actually contains `<leaf>.hkx` wins; falls back to the base dir). This is the
//! exact rule CK uses — confirmed against the Snallygaster oracle: an injured-variant
//! subgraph keeps un-overridden clips at the base path and only redirects clips whose
//! animation exists under the injured `SAPT` directory.
//!
//! Order is intentionally deterministic-but-ours (sorted): the content-list order has
//! no runtime effect and CK's internal arena order is not reproducible offline. See
//! `docs/re/animtextdata_generation.md`.

use std::path::Path;

use havok_native::behavior_clip_names::collect_behavior_clip_names_from_file;

/// Strip a leading `Animations\` component (case-insensitive) and the `.hk[tx]`
/// extension from an `animationName`, yielding the leaf relative to a `SAPT` dir.
/// `r"Animations\Attack3.hkt"` -> `r"Attack3"`.
pub fn clip_leaf(animation_name: &str) -> String {
    let norm = animation_name.replace('/', "\\");
    let no_prefix = norm
        .strip_prefix("Animations\\")
        .or_else(|| norm.strip_prefix("animations\\"))
        .unwrap_or(&norm);
    match no_prefix.rfind('.') {
        Some(dot) => no_prefix[..dot].to_string(),
        None => no_prefix.to_string(),
    }
}

/// Resolve one clip leaf against the SAPT chain (self-first) by disk existence.
/// Returns the full FO4 animation path (`<sapt>\<leaf>.hkx`) for the first chain
/// directory that contains the file on disk; falls back to the base (last) dir.
pub fn resolve_leaf(meshes_root: &Path, sapt_chain: &[String], leaf: &str) -> String {
    let rel = format!("{leaf}.hkx");
    for sapt in sapt_chain {
        let on_disk = meshes_root.join(sapt.replace('\\', "/")).join(&rel);
        if on_disk.is_file() {
            return format!("{sapt}\\{rel}");
        }
    }
    let base = sapt_chain.last().map(String::as_str).unwrap_or("");
    format!("{base}\\{rel}")
}

/// Resolve the full animation file list for a subgraph.
///
/// * `core_behavior_file` — full path to the subgraph's core behavior `.hkx` (clip
///   source). Only this graph's clips count — a sibling root behavior's clips
///   (e.g. death anims) belong to that behavior's own subgraph, not this one.
/// * `meshes_root`  — the mod's `Meshes` root (for disk-existence override resolution).
/// * `sapt_chain`   — the subgraph's SAPT chain, self-first (e.g.
///   `[r"Actors\X\Animations\Injured\RightLeg", r"Actors\X\Animations"]`).
///
/// Returns full FO4 anim paths, sorted (deterministic; order has no runtime effect).
pub fn resolve_subgraph_files(
    core_behavior_file: &Path,
    meshes_root: &Path,
    sapt_chain: &[String],
) -> Vec<String> {
    let mut files: Vec<String> = collect_behavior_clip_names_from_file(core_behavior_file)
        .iter()
        .map(|an| resolve_leaf(meshes_root, sapt_chain, &clip_leaf(an)))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_leaf_strips_prefix_and_ext() {
        assert_eq!(clip_leaf(r"Animations\Attack3.hkt"), "Attack3");
        assert_eq!(clip_leaf(r"animations\Idle_Flavor1.hkx"), "Idle_Flavor1");
        assert_eq!(clip_leaf("Bare"), "Bare");
    }
}
