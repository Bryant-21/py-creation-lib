use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use havok_native::api;
use havok_native::error::HavokError;
use havok_native::hkx::model::HkxFile;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

const RACE_PROJECTS: &[&str] = &[
    "ambient/chicken/chickenproject.hkx",
    "ambient/hare/hareproject.hkx",
    "atronachflame/atronachflame.hkx",
    "atronachfrost/atronachfrostproject.hkx",
    "atronachstorm/atronachstormproject.hkx",
    "bear/bearproject.hkx",
    "canine/dogproject.hkx",
    "canine/wolfproject.hkx",
    "chaurus/chaurusproject.hkx",
    "cow/highlandcowproject.hkx",
    "deer/deerproject.hkx",
    "dlc01/vampirebrute/vampirebruteproject.hkx",
    "dlc02/benthiclurker/benthiclurkerproject.hkx",
    "dlc02/boarriekling/boarproject.hkx",
    "dlc02/dwarvenballistacenturion/ballistacenturion.hkx",
    "dlc02/hmdaedra/hmdaedra.hkx",
    "dlc02/netch/netchproject.hkx",
    "dlc02/riekling/rieklingproject.hkx",
    "dlc02/scrib/scribproject.hkx",
    "dragon/dragonproject.hkx",
    "dragonpriest/dragon_priest.hkx",
    "draugr/draugrproject.hkx",
    "draugr/draugrskeletonproject.hkx",
    "dwarvenspherecenturion/spherecenturion.hkx",
    "dwarvenspider/dwarvenspidercenturionproject.hkx",
    "dwarvensteamcenturion/steamproject.hkx",
    "falmer/falmerproject.hkx",
    "frostbitespider/frostbitespiderproject.hkx",
    "giant/giantproject.hkx",
    "goat/goatproject.hkx",
    "hagraven/hagravenproject.hkx",
    "horker/horkerproject.hkx",
    "horse/horseproject.hkx",
    "icewraith/icewraithproject.hkx",
    "mammoth/mammothproject.hkx",
    "mudcrab/mudcrabproject.hkx",
    "sabrecat/sabrecatproject.hkx",
    "skeever/skeeverproject.hkx",
    "slaughterfish/slaughterfishproject.hkx",
    "spriggan/spriggan.hkx",
    "troll/trollproject.hkx",
    "werewolfbeast/werewolfbeastproject.hkx",
    "wisp/wispproject.hkx",
    "witchlight/witchlightproject.hkx",
];

fn ascii_hkx_references(bytes: &[u8]) -> Vec<String> {
    let mut references = Vec::new();
    let mut start = 0usize;
    while start < bytes.len() {
        while start < bytes.len() && !(0x20..=0x7e).contains(&bytes[start]) {
            start += 1;
        }
        let mut end = start;
        while end < bytes.len() && (0x20..=0x7e).contains(&bytes[end]) {
            end += 1;
        }
        if end > start {
            let text = String::from_utf8_lossy(&bytes[start..end]);
            if text.to_ascii_lowercase().ends_with(".hkx") {
                references.push(text.into_owned());
            }
        }
        start = end.saturating_add(1);
    }
    references
}

fn resolve_reference(
    actors: &Path,
    family_root: &Path,
    current: &Path,
    value: &str,
) -> Option<PathBuf> {
    let relative = value.replace('\\', "/");
    let relative = Path::new(relative.trim_start_matches('/'));
    let mut candidates = vec![family_root.join(relative)];
    let mut ancestor = current.parent();
    while let Some(parent) = ancestor {
        candidates.push(parent.join(relative));
        if parent == actors {
            break;
        }
        ancestor = parent.parent();
    }
    candidates.push(actors.join(relative));
    if relative.components().next().is_some_and(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("actors")
    }) {
        if let Some(meshes) = actors.parent() {
            candidates.push(meshes.join(relative));
        }
    }
    candidates.into_iter().find_map(|candidate| {
        let canonical = candidate.canonicalize().ok()?;
        canonical.starts_with(actors).then_some(canonical)
    })
}

fn race_reachable_hkx(actors: &Path) -> Vec<PathBuf> {
    let actors = actors.canonicalize().expect("canonical actors corpus");
    let mut queue = VecDeque::new();
    let mut seen = HashSet::new();
    for relative in RACE_PROJECTS {
        let project = actors.join(relative);
        assert!(
            project.exists(),
            "missing RACE project {}",
            project.display()
        );
        let family_root = project.parent().expect("project parent").to_path_buf();
        queue.push_back((project, family_root));
    }
    let mut files = Vec::new();
    while let Some((path, family_root)) = queue.pop_front() {
        let key = path.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let bytes = std::fs::read(&path).expect("read reachable HKX");
        for reference in ascii_hkx_references(&bytes) {
            if let Some(resolved) = resolve_reference(&actors, &family_root, &path, &reference) {
                queue.push_back((resolved, family_root.clone()));
            }
        }
        files.push(path);
    }
    files.sort();
    files
}

#[test]
fn all_extracted_skyrim_creature_hkx_have_a_panic_free_terminal_disposition() {
    let actors = repo_root().join("extracted/skyrimse/meshes/actors");
    if !actors.exists() {
        return;
    }
    let files = race_reachable_hkx(&actors);
    let inventory_only = std::env::var_os("SKYRIM_CREATURE_CORPUS_INVENTORY_ONLY").is_some();

    let mut projects = 0usize;
    let mut characters = 0usize;
    let mut skeletons = 0usize;
    let mut behaviors = 0usize;
    let mut clips = 0usize;
    let mut emitted_skeletons = 0usize;
    let mut emitted_clips = 0usize;
    let mut paired_root = 0usize;
    let mut source_paired_root = 0usize;
    let mut failures = Vec::new();

    for path in files {
        let relative = path
            .strip_prefix(&actors)
            .unwrap_or(&path)
            .display()
            .to_string();
        let bytes = std::fs::read(&path).expect("read Skyrim corpus HKX");
        let read_result = std::panic::catch_unwind(|| HkxFile::read(&bytes));
        let hkx = match read_result {
            Ok(Ok(hkx)) => hkx,
            Ok(Err(error)) => {
                failures.push(format!("{relative}: read: {error}"));
                continue;
            }
            Err(_) => {
                failures.push(format!("{relative}: reader panic"));
                continue;
            }
        };
        if hkx.contents_version() != "hk_2010.2.0-r1" {
            continue;
        }
        let has = |class_name: &str| {
            hkx.objects()
                .iter()
                .any(|object| object.class_name == class_name)
        };
        projects += usize::from(has("hkbProjectData"));
        characters += usize::from(has("hkbCharacterData"));
        behaviors += usize::from(has("hkbBehaviorGraph"));
        let is_skeleton = has("hkaSkeleton") && !has("hkaSplineCompressedAnimation");
        let is_clip = has("hkaSplineCompressedAnimation");
        let is_source_paired_root = is_clip
            && hkx.objects().iter().any(|object| {
                object.class_name == "hkaAnimationBinding"
                    && object.members.iter().any(|member| {
                        member.name == "originalSkeletonName"
                            && matches!(
                                &member.value,
                                havok_native::hkx::types::HkxValue::String {
                                    value,
                                    is_null: false
                                } if value == "PairedRoot"
                            )
                    })
            });
        skeletons += usize::from(is_skeleton);
        clips += usize::from(is_clip);
        source_paired_root += usize::from(is_source_paired_root);
        if inventory_only {
            continue;
        }
        if std::panic::catch_unwind(|| api::havok_hkx_to_xml(&bytes))
            .map_or(true, |result| result.is_err())
        {
            failures.push(format!("{relative}: XML export failed or panicked"));
            continue;
        }
        if hkx.save() != bytes {
            failures.push(format!(
                "{relative}: unchanged packfile roundtrip changed bytes"
            ));
            continue;
        }
        if !is_skeleton && !is_clip {
            continue;
        }

        let emit_result = std::panic::catch_unwind(|| {
            api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&bytes)
        });
        match emit_result {
            Ok(Ok(output)) => {
                if is_source_paired_root {
                    failures.push(format!("{relative}: PairedRoot clip was admitted"));
                    continue;
                }
                let target = match HkxFile::read(&output) {
                    Ok(target) => target,
                    Err(error) => {
                        failures.push(format!("{relative}: emitted reread: {error}"));
                        continue;
                    }
                };
                if target.class_version() != 11
                    || target.contents_version() != "hk_2014.1.0-r1"
                    || target.packfile().header.pointer_size != 8
                    || target.save() != output
                {
                    failures.push(format!("{relative}: invalid emitted FO4 packfile"));
                    continue;
                }
                emitted_skeletons += usize::from(is_skeleton);
                emitted_clips += usize::from(is_clip);
            }
            Ok(Err(HavokError::UnportedEdgeCase { edge_case, .. }))
                if is_source_paired_root && edge_case == "paired_root_binding" =>
            {
                paired_root += 1;
            }
            Ok(Err(error)) => failures.push(format!("{relative}: reemit: {error}")),
            Err(_) => failures.push(format!("{relative}: reemitter panic")),
        }
    }

    eprintln!(
        "projects={projects} characters={characters} skeletons={skeletons} behaviors={behaviors} clips={clips} emitted_skeletons={emitted_skeletons} emitted_clips={emitted_clips} source_paired_root={source_paired_root} paired_root={paired_root}"
    );
    assert!(
        failures.is_empty(),
        "Skyrim creature corpus failures ({}):\n{}",
        failures.len(),
        failures.join("\n")
    );
    if inventory_only {
        return;
    }
    assert_eq!(projects, 44, "expected the 44-family project closure");
    assert_eq!(characters, 44, "expected the 44-family character closure");
    assert_eq!(skeletons, 44, "expected the 44-family skeleton closure");
    assert_eq!(behaviors, 73, "expected the 44-family behavior closure");
    assert_eq!(clips, 2_327, "expected the 44-family clip closure");
    assert_eq!(emitted_skeletons, skeletons);
    assert_eq!(emitted_clips + paired_root, clips);
    assert_eq!(paired_root, source_paired_root);
}
