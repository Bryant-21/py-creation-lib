//! Behavior index for AnimTextData generation (CK-free).
//!
//! Collects a behavior graph's `hkbClipGenerator` clip leaves and resolves each through
//! the subgraph's `SAPT` directory chain by filesystem existence: the first directory
//! holding `<leaf>.hkx` wins, else the base dir. This is CK's rule, confirmed against the
//! Snallygaster oracle (an injured-variant subgraph redirects only clips whose animation
//! exists under the injured `SAPT` directory).
//!
//! Output is sorted: content-list order has no runtime effect and CK's arena order is not
//! reproducible offline. See `docs/re/animtextdata_generation.md`.

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

/// Collapse `a\b\..\c` to `a\c` textually (no filesystem access).
fn normalize_rel(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("\\")
}

/// `Actors\<Project>` for the project that owns `core_behavior_file`, derived from
/// its path under `meshes_root` (`…\Actors\Character\Behaviors\MTBehavior.hkx` ->
/// `Actors\Character`).
pub fn core_project_dir(meshes_root: &Path, core_behavior_file: &Path) -> Option<String> {
    let rel = core_behavior_file.strip_prefix(meshes_root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if parts.len() >= 3 && parts[0].eq_ignore_ascii_case("actors") {
        Some(parts[..parts.len() - 2].join("\\"))
    } else {
        None
    }
}

/// Resolve one clip leaf against the SAPT chain (self-first) by disk existence.
/// Returns `<sapt>\<leaf>.hkx` for the first chain directory holding the file, else
/// the base (last) dir.
///
/// `core_project` is `Actors\<Project>` for the graph that owns the clip. A subgraph
/// grafted from the shared `Actors\Character` tree names its clips relative to that
/// project, so a leaf no SAPT directory satisfies (`1HM\TurnInPlaceLeft180Loop`, or an
/// escape like `..\PowerArmor\Animations\1HM\SprintPainTrain`) is resolved there, after
/// the SAPT chain misses. Joined onto a SAPT dir, such a leaf normalizes into the
/// mounting creature's own tree, where the animation does not exist.
pub fn resolve_leaf(
    meshes_root: &Path,
    sapt_chain: &[String],
    leaf: &str,
    core_project: Option<&str>,
) -> String {
    let rel = format!("{leaf}.hkx");
    let exists = |candidate: &str| -> bool {
        !candidate.is_empty() && meshes_root.join(candidate.replace('\\', "/")).is_file()
    };

    for sapt in sapt_chain {
        let on_disk = meshes_root.join(sapt.replace('\\', "/")).join(&rel);
        if on_disk.is_file() {
            return format!("{sapt}\\{rel}");
        }
    }

    if let Some(project) = core_project {
        // An escaping leaf is relative to the project ROOT.
        if leaf.starts_with("..\\") || leaf.starts_with("../") {
            let candidate = normalize_rel(&format!("{project}\\{rel}"));
            if exists(&candidate) {
                return candidate;
            }
        }
        // Otherwise it is relative to the project's own `Animations` dir.
        let candidate = normalize_rel(&format!("{project}\\Animations\\{rel}"));
        if exists(&candidate) {
            return candidate;
        }
    }

    let base = sapt_chain.last().map(String::as_str).unwrap_or("");
    format!("{base}\\{rel}")
}

/// Resolve a subgraph's animation files as sorted full FO4 anim paths.
///
/// Only `core_behavior_file`'s clips count; a sibling root behavior's clips (e.g. death
/// anims) belong to that behavior's own subgraph. `sapt_chain` is self-first, e.g.
/// `[r"Actors\X\Animations\Injured\RightLeg", r"Actors\X\Animations"]`.
pub fn resolve_subgraph_files(
    core_behavior_file: &Path,
    meshes_root: &Path,
    sapt_chain: &[String],
) -> Vec<String> {
    let core_project = core_project_dir(meshes_root, core_behavior_file);
    let mut files = collect_behavior_clip_names_from_file(core_behavior_file)
        .iter()
        .map(|an| {
            resolve_leaf(
                meshes_root,
                sapt_chain,
                &clip_leaf(an),
                core_project.as_deref(),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();

    if behavior_uses_dynamic_animation_tags(core_behavior_file)
        && let Some(sapt) = sapt_chain.first()
    {
        add_direct_sapt_hkx_files(&mut files, meshes_root, sapt);
    }

    files.into_iter().collect()
}

pub(super) fn behavior_uses_dynamic_animation_tags(core_behavior_file: &Path) -> bool {
    super::hkx_cache::behavior_packfile(core_behavior_file).is_some_and(|hkx| {
        hkx.objects()
            .iter()
            .any(|object| object.class_name == "DynamicAnimationTaggingGenerator")
    })
}

pub(super) fn add_direct_sapt_hkx_files(
    files: &mut std::collections::BTreeSet<String>,
    meshes_root: &Path,
    sapt: &str,
) {
    let Ok(entries) = std::fs::read_dir(meshes_root.join(sapt.replace('\\', "/"))) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file()
            || !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("hkx"))
        {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        files.insert(format!(
            "{}\\{file_name}",
            sapt.trim_end_matches(['\\', '/'])
        ));
    }
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

    fn touch(root: &Path, rel: &str) {
        let full = root.join(rel.replace('\\', "/"));
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, b"x").unwrap();
    }

    #[test]
    fn sapt_chain_still_wins_when_the_creature_overrides_a_clip() {
        let root = tempfile::tempdir().unwrap();
        touch(
            root.path(),
            r"Actors\MoleMiner\Animations\Shared\DeathChest01.hkx",
        );
        let chain = vec![
            r"Actors\MoleMiner\Animations\Shared".to_string(),
            r"Actors\MoleMiner\Animations".to_string(),
        ];

        let got = resolve_leaf(
            root.path(),
            &chain,
            "DeathChest01",
            Some(r"Actors\Character"),
        );

        assert_eq!(got, r"Actors\MoleMiner\Animations\Shared\DeathChest01.hkx");
    }

    #[test]
    fn leaf_the_creature_lacks_falls_back_to_the_core_behaviors_project() {
        let root = tempfile::tempdir().unwrap();
        // Only the shared Character tree has it — the mounting creature does not.
        touch(
            root.path(),
            r"Actors\Character\Animations\1HM\TurnInPlaceLeft180Loop.hkx",
        );
        let chain = vec![
            r"Actors\MoleMiner\Animations\Shared".to_string(),
            r"Actors\MoleMiner\Animations".to_string(),
        ];

        let got = resolve_leaf(
            root.path(),
            &chain,
            r"1HM\TurnInPlaceLeft180Loop",
            Some(r"Actors\Character"),
        );

        assert_eq!(
            got,
            r"Actors\Character\Animations\1HM\TurnInPlaceLeft180Loop.hkx"
        );
    }

    #[test]
    fn escaping_leaf_resolves_against_the_project_root_not_the_sapt_dir() {
        let root = tempfile::tempdir().unwrap();
        touch(
            root.path(),
            r"Actors\PowerArmor\Animations\1HM\SprintPainTrain.hkx",
        );
        let chain = vec![
            r"Actors\MoleMiner\Animations\Shared".to_string(),
            r"Actors\MoleMiner\Animations".to_string(),
        ];

        let got = resolve_leaf(
            root.path(),
            &chain,
            r"..\PowerArmor\Animations\1HM\SprintPainTrain",
            Some(r"Actors\Character"),
        );

        // NOT Actors\MoleMiner\Animations\Shared\..\PowerArmor\...
        assert_eq!(got, r"Actors\PowerArmor\Animations\1HM\SprintPainTrain.hkx");
    }

    #[test]
    fn unresolvable_leaf_keeps_the_historical_base_fallback() {
        let root = tempfile::tempdir().unwrap();
        let chain = vec![r"Actors\MoleMiner\Animations".to_string()];

        let got = resolve_leaf(root.path(), &chain, "NoSuchAnim", Some(r"Actors\Character"));

        assert_eq!(got, r"Actors\MoleMiner\Animations\NoSuchAnim.hkx");
    }

    #[test]
    fn core_project_dir_comes_from_the_behavior_path() {
        let root = Path::new(r"C:\meshes");
        assert_eq!(
            core_project_dir(
                root,
                &root.join(r"Actors\Character\Behaviors\MTBehavior.hkx")
            ),
            Some(r"Actors\Character".to_string())
        );
        assert_eq!(
            core_project_dir(
                root,
                &root.join(r"Actors\Fixture\MoleMiner\Behaviors\MTBehavior.hkx")
            ),
            Some(r"Actors\Fixture\MoleMiner".to_string())
        );
    }

    #[test]
    fn dynamic_animation_subgraphs_include_unreferenced_direct_sapt_clips() {
        use havok_native::hkx::descriptors::DescriptorRegistry;
        use havok_native::hkx::types::HkxValue;
        use havok_native::hkx::{HkxFile, HkxMember, HkxObject, write_hkx};

        let root = tempfile::tempdir().unwrap();
        let behavior = root
            .path()
            .join(r"Actors/Character/Behaviors/WorkbenchFurnitureBehavior.hkx");
        std::fs::create_dir_all(behavior.parent().unwrap()).unwrap();
        let hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                HkxObject {
                    name: Some("#0001".to_string()),
                    offset: 0,
                    signature: 0,
                    class_name: "hkbClipGenerator".to_string(),
                    members: vec![HkxMember {
                        name: "animationName".to_string(),
                        value: HkxValue::String {
                            value: r"Animations\PoseA_IdleFlavor2.hkt".to_string(),
                            is_null: false,
                        },
                    }],
                },
                HkxObject {
                    name: Some("#0002".to_string()),
                    offset: 0,
                    signature: 0,
                    class_name: "DynamicAnimationTaggingGenerator".to_string(),
                    members: Vec::new(),
                },
            ],
        );
        let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
        std::fs::write(&behavior, write_hkx(&hkx, &mut registry)).unwrap();

        let sapt = r"Actors\Character\Animations\Furniture\WorkbenchTinkers";
        touch(root.path(), &format!(r"{sapt}\PoseA_IdleFlavor1.hkx"));
        touch(root.path(), &format!(r"{sapt}\PoseA_IdleFlavor2.hkx"));
        touch(root.path(), &format!(r"{sapt}\PoseA_IdleFlavor3.hkx"));

        let files = resolve_subgraph_files(&behavior, root.path(), &[sapt.to_string()]);

        assert_eq!(
            files,
            vec![
                format!(r"{sapt}\PoseA_IdleFlavor1.hkx"),
                format!(r"{sapt}\PoseA_IdleFlavor2.hkx"),
                format!(r"{sapt}\PoseA_IdleFlavor3.hkx"),
            ]
        );
    }
}
