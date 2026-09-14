//! Make each clip generator share a name with the animation it plays.
//!
//! FO4 AnimationOffsets keys on name identity: all 511064 section-1 rows across the 3156
//! shipped files have `clip_name == basename(anim_path)`. The runtime (`0x1313BE0`) finds the
//! clip by generator name, then probes a per-subgraph table keyed by clip name with the
//! record's animation basename, comparing interned pointers. The two coincide only under the
//! invariant.
//!
//! FO76 creature graphs break it: RadHog drives `TuskSwipe_Front.hkx` from a generator called
//! `AttackMelee_TuskSwipe_Front`. In FO4 the animation probe misses, the entry is dropped from
//! the actor's attack array, `attackTime` stays 0.0, and combat's weighting filter rejects
//! every candidate, so the creature charges but never swings.
//!
//! This pass copies the animation to a sibling named after the generator and repoints
//! `hkbClipGenerator::animationName` at the copy, so the emitter's identity guard keeps the
//! clip. Copy, not rename: one animation may back several generators (RadHog drives
//! `ChargeStrike.hkx` from both `AttackMelee01` and `ChargeStrike`).
//!
//! Every aliased generator is covered, not only combat ones: the weapon builder emits all
//! generators of the shared `Weapon`/`Melee`/`MTBehavior` cores into section 1, so an aliased
//! locomotion generator is a dropped row. The section-2 `TurnLeft<deg>`/`TurnRight<deg>` disk
//! scan is unaffected: the original file stays in place, and on the live tree no aliased
//! generator is named `turn(left|right)<digits>`.
//!
//! # FixupReport mapping
//! `records_changed` = number of clip generators realigned.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use havok_native::hkx::read_packfile;
use havok_native::hkx::types::HkxValue;

use super::behavior_index::{clip_leaf, core_project_dir, resolve_leaf};
use super::emit::{AnimTextDataInputs, SubgraphInput};

/// A clip whose generator name and animation basename disagree.
struct Aliased {
    generator: String,
    /// Full FO4 path of the animation as it exists today, e.g.
    /// `Actors\RadHog\Animations\TuskSwipe_Front.hkx`.
    source_rel: String,
}

/// Realign every aliased attack clip generator reachable from `inputs`.
///
/// Runs before AnimTextData generation so the emitter observes the repaired graphs.
pub fn align_clip_generator_names(
    inputs: &AnimTextDataInputs,
    src_meshes_root: &Path,
) -> Result<u32, String> {
    let mut realigned = 0u32;
    let mut seen_core: BTreeSet<String> = BTreeSet::new();
    for subgraph in &inputs.subgraphs {
        if !seen_core.insert(subgraph.core_behavior.to_ascii_lowercase()) {
            continue;
        }
        let core_file = src_meshes_root.join(subgraph.core_behavior.replace('\\', "/"));
        if !core_file.is_file() {
            continue;
        }
        realigned += align_one_core(&core_file, subgraph, src_meshes_root)?;
    }
    if realigned > 0 {
        // Required. `resolve_anim_events` reads through `hkx_cache::behavior_packfile`, so the
        // process-wide memo can hold the pre-rewrite parse of every touched graph. Left there,
        // the clip generator, AnimEventInfo and offsets emitters read stale objects and emit
        // the old aliased names even though the graphs on disk are correct.
        super::hkx_cache::clear_all();
    }
    Ok(realigned)
}

fn align_one_core(
    core_file: &Path,
    subgraph: &SubgraphInput,
    src_meshes_root: &Path,
) -> Result<u32, String> {
    let aliased = collect_aliased(core_file, subgraph, src_meshes_root);
    if aliased.is_empty() {
        return Ok(0);
    }

    // Copy first: a graph repointed at an animation that failed to materialize would be
    // worse than leaving the clip aliased, so only generators whose copy exists get
    // rewritten.
    let mut landed: BTreeSet<String> = BTreeSet::new();
    for clip in &aliased {
        if copy_animation_beside(src_meshes_root, &clip.source_rel, &clip.generator)? {
            landed.insert(clip.generator.to_ascii_lowercase());
        }
    }
    if landed.is_empty() {
        return Ok(0);
    }

    rewrite_animation_names(core_file, &landed)
}

fn collect_aliased(
    core_file: &Path,
    subgraph: &SubgraphInput,
    src_meshes_root: &Path,
) -> Vec<Aliased> {
    let Ok(data) = std::fs::read(core_file) else {
        return Vec::new();
    };
    let Ok(hkx) = read_packfile(&data) else {
        return Vec::new();
    };
    let core_project = core_project_dir(src_meshes_root, core_file);

    let mut out = Vec::new();
    for obj in hkx.objects() {
        if obj.class_name != "hkbClipGenerator" {
            continue;
        }
        let (Some(name), Some(animation)) = (
            string_member(obj.members.as_slice(), "name"),
            string_member(obj.members.as_slice(), "animationName"),
        ) else {
            continue;
        };
        if animation.is_empty() {
            continue; // nothing to align against
        }
        let leaf = clip_leaf(&animation);
        let base = leaf.rsplit(['\\', '/']).next().unwrap_or(&leaf);
        if base.eq_ignore_ascii_case(&name) {
            continue; // already satisfies the invariant
        }
        if base.eq_ignore_ascii_case("idle") {
            continue; // dynamic placeholder — its animation is injected at runtime
        }
        let source_rel = resolve_leaf(
            src_meshes_root,
            &subgraph.sapt_chain,
            &leaf,
            core_project.as_deref(),
        );
        if !src_meshes_root
            .join(source_rel.replace('\\', "/"))
            .is_file()
        {
            continue; // unresolvable on disk; not this pass's problem to invent
        }
        out.push(Aliased {
            generator: name,
            source_rel,
        });
    }
    out
}

/// Copy `source_rel` to a sibling named `generator`. Returns whether the target now holds
/// this animation — i.e. whether the caller may repoint the generator at it.
fn copy_animation_beside(
    src_meshes_root: &Path,
    source_rel: &str,
    generator: &str,
) -> Result<bool, String> {
    let source = src_meshes_root.join(source_rel.replace('\\', "/"));
    let Some(dir) = source.parent() else {
        return Ok(false);
    };
    let target: PathBuf = dir.join(format!("{generator}.hkx"));
    if target.is_file() {
        // An existing target counts as aligned only if it holds the same animation. Otherwise
        // repointing would swap the generator's motion (5 generators named `Idle` play
        // `posea_idle1` beside an unrelated `Idle.hkx`); a dropped row is the lesser harm.
        let same = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0)
            == std::fs::metadata(&source)
                .map(|m| m.len())
                .unwrap_or(u64::MAX)
            && std::fs::read(&target).ok() == std::fs::read(&source).ok();
        return Ok(same);
    }
    std::fs::copy(&source, &target).map_err(|error| {
        format!(
            "failed to align attack clip {} to {}: {error}",
            source.display(),
            target.display()
        )
    })?;
    Ok(true)
}

/// Point each named generator's `animationName` at its own copy, preserving the
/// directory prefix and extension spelling already in the graph.
fn rewrite_animation_names(core_file: &Path, generators: &BTreeSet<String>) -> Result<u32, String> {
    let data = std::fs::read(core_file).map_err(|e| format!("{}: {e}", core_file.display()))?;
    let mut hkx = read_packfile(&data).map_err(|e| format!("{}: {e}", core_file.display()))?;

    let mut changed = 0u32;
    for obj in hkx.objects_mut() {
        if obj.class_name != "hkbClipGenerator" {
            continue;
        }
        let Some(name) = string_member(obj.members.as_slice(), "name") else {
            continue;
        };
        if !generators.contains(&name.to_ascii_lowercase()) {
            continue;
        }
        for member in &mut obj.members {
            if member.name != "animationName" {
                continue;
            }
            if let HkxValue::String { value, .. } = &mut member.value {
                let replaced = rename_leaf(value, &name);
                if replaced != *value {
                    *value = replaced;
                    changed += 1;
                }
            }
        }
    }

    if changed > 0 {
        let out = hkx.save();
        std::fs::write(core_file, out).map_err(|e| format!("{}: {e}", core_file.display()))?;
    }
    Ok(changed)
}

/// `Animations\TuskSwipe_Front.hkt` + `AttackMelee_TuskSwipe_Front`
/// -> `Animations\AttackMelee_TuskSwipe_Front.hkt`.
fn rename_leaf(animation_name: &str, generator: &str) -> String {
    let norm = animation_name.replace('/', "\\");
    let (dir, file) = match norm.rfind('\\') {
        Some(slash) => (&norm[..=slash], &norm[slash + 1..]),
        None => ("", norm.as_str()),
    };
    let ext = file.rfind('.').map(|dot| &file[dot..]).unwrap_or("");
    format!("{dir}{generator}{ext}")
}

fn string_member(members: &[havok_native::hkx::HkxMember], name: &str) -> Option<String> {
    members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::String { value, .. } => Some(value.clone()),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_leaf_preserves_directory_and_extension() {
        assert_eq!(
            rename_leaf(
                r"Animations\TuskSwipe_Front.hkt",
                "AttackMelee_TuskSwipe_Front"
            ),
            r"Animations\AttackMelee_TuskSwipe_Front.hkt"
        );
        assert_eq!(
            rename_leaf("ChargeStrike.hkx", "AttackMelee01"),
            "AttackMelee01.hkx"
        );
        // Forward slashes normalize; a leaf with no extension stays extensionless.
        assert_eq!(rename_leaf("Animations/Foo", "Bar"), r"Animations\Bar");
    }

    /// A target that already holds this same animation — an earlier run, or a generator that
    /// already owned its copy — is aligned and must be reused without being rewritten.
    #[test]
    fn copy_is_skipped_when_the_generator_already_owns_the_same_animation() {
        let dir = tempfile::tempdir().unwrap();
        let anims = dir.path().join("Actors/X/Animations");
        std::fs::create_dir_all(&anims).unwrap();
        std::fs::write(anims.join("TuskSwipe_Front.hkx"), b"source").unwrap();
        std::fs::write(anims.join("AttackMelee_TuskSwipe_Front.hkx"), b"source").unwrap();

        let made = copy_animation_beside(
            dir.path(),
            r"Actors\X\Animations\TuskSwipe_Front.hkx",
            "AttackMelee_TuskSwipe_Front",
        )
        .unwrap();

        assert!(made);
        assert_eq!(
            std::fs::read(anims.join("AttackMelee_TuskSwipe_Front.hkx")).unwrap(),
            b"source"
        );
    }

    /// The real case that made widening dangerous: a generator named `Idle` plays
    /// `posea_idle1` while an UNRELATED `Idle.hkx` already sits beside it. Repointing would
    /// swap the generator's motion, so the clip must stay aliased (and be dropped) instead.
    /// Five generators on the live tree are in exactly this shape.
    #[test]
    fn a_different_animation_already_holding_the_name_blocks_the_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let anims = dir.path().join("Actors/X/Animations");
        std::fs::create_dir_all(&anims).unwrap();
        std::fs::write(
            anims.join("posea_idle1.hkx"),
            b"the generator's real animation",
        )
        .unwrap();
        std::fs::write(anims.join("Idle.hkx"), b"a completely different animation").unwrap();

        let made =
            copy_animation_beside(dir.path(), r"Actors\X\Animations\posea_idle1.hkx", "Idle")
                .unwrap();

        assert!(
            !made,
            "must not claim alignment against an unrelated animation"
        );
        // And the pre-existing file must not be clobbered.
        assert_eq!(
            std::fs::read(anims.join("Idle.hkx")).unwrap(),
            b"a completely different animation"
        );
    }

    /// Same length but different bytes must not pass as "already aligned" either.
    #[test]
    fn same_size_but_different_bytes_is_not_treated_as_aligned() {
        let dir = tempfile::tempdir().unwrap();
        let anims = dir.path().join("Actors/X/Animations");
        std::fs::create_dir_all(&anims).unwrap();
        std::fs::write(anims.join("walkfwd.hkx"), b"AAAA").unwrap();
        std::fs::write(anims.join("WalkForward.hkx"), b"BBBB").unwrap();

        let made = copy_animation_beside(
            dir.path(),
            r"Actors\X\Animations\walkfwd.hkx",
            "WalkForward",
        )
        .unwrap();

        assert!(!made);
    }

    #[test]
    fn copy_materializes_the_generator_named_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let anims = dir.path().join("Actors/X/Animations");
        std::fs::create_dir_all(&anims).unwrap();
        std::fs::write(anims.join("TuskSwipe_Front.hkx"), b"source").unwrap();

        let made = copy_animation_beside(
            dir.path(),
            r"Actors\X\Animations\TuskSwipe_Front.hkx",
            "AttackMelee_TuskSwipe_Front",
        )
        .unwrap();

        assert!(made);
        assert_eq!(
            std::fs::read(anims.join("AttackMelee_TuskSwipe_Front.hkx")).unwrap(),
            b"source"
        );
    }
}
