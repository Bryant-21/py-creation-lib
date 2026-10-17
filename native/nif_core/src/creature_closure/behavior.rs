use super::*;
use havok_native::hkx::{HkxFile, types::HkxValue};

pub(super) fn stage(
    nif: &mut NifFile,
    source_root: &Path,
    namespace: &str,
    input: &PreparedInput,
    artifacts: &mut BTreeMap<String, PendingArtifact>,
) -> Result<bool, CreatureClosureError> {
    let pending = stage_attached_behavior_closure(nif, source_root, namespace)
        .map_err(|message| input_error(CreatureTerminalKind::ConversionFailed, input, message))?;
    let changed = !pending.is_empty();
    for (target, bytes) in pending {
        add_artifact(
            artifacts,
            CreatureArtifactKind::Hkx,
            &target,
            bytes,
            &input.input_key,
            input,
        )?;
    }
    Ok(changed)
}

pub fn stage_attached_behavior_closure(
    nif: &mut NifFile,
    source_root: &Path,
    namespace: &str,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut artifacts = BTreeMap::new();
    let mut queue = VecDeque::new();
    for block in &mut nif.blocks {
        if block.type_name != "BSBehaviorGraphExtraData" {
            continue;
        }
        let Some(NifValue::String(path)) = block.get_field("Behaviour Graph File") else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let source = canonical_runtime_path(path, "meshes")
            .and_then(|path| normalize_data_relative(&path, "hkx"))?;
        let project_root = source.rsplit_once('/').unwrap().0.to_string();
        queue.push_back((source.clone(), project_root));
        let target = add_namespace(&source, "meshes", namespace)?;
        block.set_field(
            "Behaviour Graph File",
            NifValue::String(target.trim_start_matches("meshes/").replace('/', "\\")),
        );
    }
    let mut visited = BTreeSet::new();
    while let Some((source, project_root)) = queue.pop_front() {
        if !visited.insert(source.clone()) {
            continue;
        }
        let path = resolve_case_insensitive(source_root, &source)
            .ok_or_else(|| format!("attached behavior dependency is missing: {source}"))?;
        let bytes = fs::read(path)
            .map_err(|failure| format!("read attached behavior {source}: {failure}"))?;
        let file = HkxFile::read(&bytes).map_err(|failure| format!("{source}: {failure}"))?;
        for dependency in dependencies(&file, &source, &project_root, source_root)? {
            queue.push_back((dependency, project_root.clone()));
        }
        let converted = if file.contents_version() == "hk_2010.2.0-r1" {
            havok_native::convert::skyrim_fx::reemit_skyrim_fx_asset_to_fo4(&bytes)
        } else {
            havok_native::api::havok_convert_bytes(&bytes, "hk_2014.1.0-r1")
        }
        .map_err(|failure| format!("attached behavior {source}: {failure}"))?;
        validate_hkx(&converted)?;
        let target = add_namespace(&source, "meshes", namespace)?;
        artifacts.insert(target, converted);
    }
    Ok(artifacts)
}

pub(super) fn validate_hkx(bytes: &[u8]) -> Result<(), String> {
    let file =
        HkxFile::read(bytes).map_err(|failure| format!("attached behavior reread: {failure}"))?;
    if file.contents_version() != "hk_2014.1.0-r1" {
        return Err(format!(
            "attached behavior has unsupported Havok version {}",
            file.contents_version()
        ));
    }
    Ok(())
}

fn dependencies(
    file: &HkxFile,
    source: &str,
    project_root: &str,
    source_root: &Path,
) -> Result<Vec<String>, String> {
    let mut result = BTreeSet::new();
    for object in file.objects() {
        for member in &object.members {
            let path_prefix = match (object.class_name.as_str(), member.name.as_str()) {
                ("hkbProjectStringData", "characterFilenames") => Some("characterPath"),
                ("hkbProjectStringData", "behaviorFilenames") => Some("behaviorPath"),
                ("hkbProjectStringData", "animationFilenames") => Some("animationPath"),
                (
                    "hkbCharacterStringData",
                    "rigName"
                    | "ragdollName"
                    | "behaviorFilename"
                    | "animationNames"
                    | "animationBundleNameData"
                    | "retargetingSkeletonMapperFilenames",
                )
                | ("hkbBehaviorReferenceGenerator", "behaviorName")
                | ("hkbClipGenerator", "animationName") => None,
                _ => continue,
            };
            let prefix = path_prefix
                .and_then(|name| object.members.iter().find(|entry| entry.name == name))
                .and_then(|entry| match &entry.value {
                    HkxValue::String {
                        value,
                        is_null: false,
                    } => Some(value.as_str()),
                    _ => None,
                })
                .unwrap_or("");
            for path in hkx_strings(&member.value) {
                if path.is_empty() || path == "null" {
                    continue;
                }
                if !path.to_ascii_lowercase().ends_with(".hkx") {
                    return Err(format!(
                        "attached behavior {source} has unsupported {} dependency {path:?}",
                        member.name
                    ));
                }
                let raw = if prefix.is_empty() {
                    path.to_string()
                } else {
                    format!("{prefix}/{path}")
                };
                let owner_root = source.rsplit_once('/').unwrap().0;
                let behavior_root = format!("{project_root}/behaviors");
                let bases: &[&str] = if object.class_name == "hkbBehaviorReferenceGenerator" {
                    &[owner_root, &behavior_root, project_root, "meshes"]
                } else {
                    &[project_root, owner_root, "meshes"]
                };
                let resolved = bases
                    .iter()
                    .find_map(|base| {
                        let candidate = resolve_relative(base, &raw).ok()?;
                        resolve_case_insensitive(source_root, &candidate).map(|_| candidate)
                    })
                    .ok_or_else(|| {
                        format!(
                            "attached behavior {source} cannot resolve {} dependency {raw:?}",
                            member.name
                        )
                    })?;
                result.insert(resolved);
            }
        }
    }
    Ok(result.into_iter().collect())
}

fn hkx_strings(value: &HkxValue) -> Vec<&str> {
    match value {
        HkxValue::String {
            value,
            is_null: false,
        } => vec![value],
        HkxValue::Array(values) => values.iter().flat_map(hkx_strings).collect(),
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => members
            .iter()
            .filter(|member| member.name == "bundleName")
            .flat_map(|member| hkx_strings(&member.value))
            .collect(),
        _ => Vec::new(),
    }
}

fn resolve_relative(base: &str, raw: &str) -> Result<String, String> {
    let raw = raw.replace('\\', "/");
    if raw.starts_with('/') || raw.contains(':') {
        return Err(format!("behavior dependency is not relative: {raw:?}"));
    }
    let mut parts = if raw.to_ascii_lowercase().starts_with("meshes/") {
        Vec::new()
    } else {
        base.split('/').map(str::to_string).collect::<Vec<_>>()
    };
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.len() > 1 => {
                parts.pop();
            }
            ".." => return Err(format!("behavior dependency escapes meshes: {raw:?}")),
            value => parts.push(value.to_ascii_lowercase()),
        }
    }
    let path = parts.join("/");
    if !path.starts_with("meshes/") {
        return Err(format!("behavior dependency escapes meshes: {raw:?}"));
    }
    normalize_data_relative(&path, "hkx")
}

pub(super) fn validate_sequences(source: &NifFile, converted: &NifFile) -> Result<(), String> {
    for sequence in source
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiControllerSequence")
    {
        let Some(NifValue::String(name)) = sequence.get_field("Name") else {
            continue;
        };
        let target = converted
            .blocks
            .iter()
            .find(|block| {
                block.type_name == "NiControllerSequence"
                    && block.get_field("Name") == Some(&NifValue::String(name.clone()))
            })
            .ok_or_else(|| format!("source controller sequence {name:?} was lost"))?;
        let count = |block: &NifBlock| match block.get_field("Controlled Blocks") {
            Some(NifValue::Array(values)) => values.len(),
            _ => 0,
        };
        if count(sequence) != count(target) {
            return Err(format!(
                "source controller sequence {name:?} lost controlled blocks ({} -> {})",
                count(sequence),
                count(target)
            ));
        }
        for field in [
            "Weight",
            "Cycle Type",
            "Frequency",
            "Start Time",
            "Stop Time",
            "Accum Root Name",
        ] {
            if sequence.get_field(field) != target.get_field(field) {
                return Err(format!(
                    "source controller sequence {name:?} changed {field}"
                ));
            }
        }
        for field in ["Controlled Blocks", "Text Keys"] {
            if let Some(value) = sequence.get_field(field) {
                let target_value = target
                    .get_field(field)
                    .ok_or_else(|| format!("source controller sequence {name:?} lost {field}"))?;
                validate_controller_value(
                    source,
                    converted,
                    value,
                    target_value,
                    &mut BTreeSet::new(),
                )
                .map_err(|error| format!("source controller sequence {name:?} {field}: {error}"))?;
            }
        }
    }
    Ok(())
}

fn validate_controller_value(
    source: &NifFile,
    target: &NifFile,
    left: &NifValue,
    right: &NifValue,
    visited: &mut BTreeSet<(i32, i32)>,
) -> Result<(), String> {
    match (left, right) {
        (NifValue::Ref(a), NifValue::Ref(b)) if *a >= 0 && *b >= 0 => {
            if !visited.insert((*a, *b)) {
                return Ok(());
            }
            let source_block = source
                .get_block(*a as usize)
                .ok_or("invalid source controller reference")?;
            let target_block = target
                .get_block(*b as usize)
                .ok_or("invalid target controller reference")?;
            if source_block.type_name != target_block.type_name {
                return Err(format!(
                    "{} reference became {}",
                    source_block.type_name, target_block.type_name
                ));
            }
            for (name, value) in &source_block.fields {
                // The shader/geometry graph has its own conversion. Sequence data
                // and controller links remain semantic, while these links rebind it.
                if matches!(name.as_str(), "Target" | "Next Controller") {
                    continue;
                }
                let target_value = target_block
                    .get_field(name)
                    .ok_or_else(|| format!("{} lost {name}", source_block.type_name))?;
                validate_controller_value(source, target, value, target_value, visited)?;
            }
            Ok(())
        }
        (NifValue::Array(a), NifValue::Array(b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b) {
                validate_controller_value(source, target, a, b, visited)?;
            }
            Ok(())
        }
        (NifValue::Struct(a), NifValue::Struct(b)) => {
            for (name, value) in a {
                let target_value = b
                    .get(name)
                    .ok_or_else(|| format!("controller data lost {name}"))?;
                validate_controller_value(source, target, value, target_value, visited)?;
            }
            Ok(())
        }
        _ if left == right => Ok(()),
        _ => Err("controller data or binding changed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_declared_animation_bundles_close_even_without_graph_clip_references() {
        use havok_native::hkx::model::{HkxMember, HkxObject};
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("meshes/weapons/bow/animations/declared_only.hkx");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"dependency fixture").unwrap();
        let file = HkxFile::from_tagxml(
            8,
            "hk_2010.2.0-r1",
            vec![HkxObject {
                name: Some("#0001".into()),
                offset: 0,
                signature: 0x655b42bc,
                class_name: "hkbCharacterStringData".into(),
                members: vec![HkxMember {
                    name: "animationBundleNameData".into(),
                    value: HkxValue::Array(vec![HkxValue::Object(vec![HkxMember {
                        name: "bundleName".into(),
                        value: HkxValue::String {
                            value: "Animations\\declared_only.hkx".into(),
                            is_null: false,
                        },
                    }])]),
                }],
            }],
        );
        assert_eq!(
            dependencies(
                &file,
                "meshes/weapons/bow/characters/bowcharacter.hkx",
                "meshes/weapons/bow",
                directory.path()
            )
            .unwrap(),
            vec!["meshes/weapons/bow/animations/declared_only.hkx"]
        );
    }

    #[test]
    fn behavior_relative_dependencies_preserve_parent_links_without_escaping_meshes() {
        assert_eq!(
            resolve_relative(
                "meshes/uniquebehaviors/sprigganfx",
                "CharacterAssets\\SingleBoneSkeleton.hkx"
            )
            .unwrap(),
            "meshes/uniquebehaviors/sprigganfx/characterassets/singleboneskeleton.hkx"
        );
        assert_eq!(
            resolve_relative(
                "meshes/uniquebehaviors/sprigganfx",
                "../../GenericBehaviors/Default.hkx"
            )
            .unwrap(),
            "meshes/genericbehaviors/default.hkx"
        );
        assert!(
            resolve_relative("meshes/uniquebehaviors/sprigganfx", "../../../outside.hkx").is_err()
        );
        assert!(
            resolve_relative("meshes/uniquebehaviors/sprigganfx", "C:/source/test.hkx").is_err()
        );
    }

    #[test]
    fn lost_controller_sequence_is_rejected() {
        let mut source = NifFile::new("fo4");
        let sequence = source.add_block("NiControllerSequence", None);
        source.blocks[sequence].set_field("Name", NifValue::String("KillFX".into()));
        assert!(
            validate_sequences(&source, &NifFile::new("fo4"))
                .unwrap_err()
                .contains("KillFX")
        );
        assert!(validate_sequences(&source, &source).is_ok());
    }

    #[test]
    fn changed_controller_binding_and_key_data_are_rejected() {
        let mut source = NifFile::new("fo4");
        let sequence = source.add_block("NiControllerSequence", None);
        let interpolator = source.add_block("NiFloatInterpolator", None);
        source.blocks[interpolator].set_field("Value", NifValue::Float(0.25));
        source.blocks[sequence].set_field("Name", NifValue::String("SkinFadeOut".into()));
        source.blocks[sequence].set_field(
            "Controlled Blocks",
            NifValue::Array(vec![NifValue::Struct(indexmap::IndexMap::from([
                ("Interpolator".into(), NifValue::Ref(interpolator as i32)),
                ("Node Name".into(), NifValue::String("Skin".into())),
            ]))]),
        );
        let mut converted = source.clone();
        converted.blocks[interpolator].set_field("Value", NifValue::Float(0.75));
        assert!(validate_sequences(&source, &converted).is_err());
        converted = source.clone();
        let Some(NifValue::Array(blocks)) =
            converted.blocks[sequence].get_field_mut("Controlled Blocks")
        else {
            unreachable!()
        };
        let NifValue::Struct(fields) = &mut blocks[0] else {
            unreachable!()
        };
        fields.insert("Node Name".into(), NifValue::String("WrongSkin".into()));
        assert!(validate_sequences(&source, &converted).is_err());
    }

    #[test]
    #[ignore = "requires extracted Skyrim creature FX NIFs and attached HKX projects"]
    fn source_creature_fx_projects_stage_all_attached_dependencies() {
        let source_root = std::env::var_os("MODKIT_SKYRIM_MESH_ROOT")
            .map(PathBuf::from)
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/skyrimse")
            });
        let temporary = tempfile::tempdir().unwrap();
        for relative in [
            "actors/icewraith/character assets/icewraith.nif",
            "actors/icewraith/character assets/fxicewraithmultiparticles.nif",
            "actors/atronachstorm/character assets/atronachstorm.nif",
            "actors/atronachstorm/character assets/fxatronachstormskinlightning.nif",
            "actors/dwarvensteamcenturion/character assets/steamcenturionfx.nif",
            "actors/dlc01/dragon/dragon_icelakebloodhead.nif",
            "actors/dlc01/dragon/dragon_icelakebloodtail.nif",
            "actors/dlc01/dragon/dragon_icelakebloodwingl.nif",
            "actors/dlc01/dragon/dragon_icelakebloodwingr.nif",
        ] {
            let request = CreatureClosureRequest {
                source_game: "skyrimse".into(),
                source_data_root: source_root.clone(),
                private_staging_root: temporary.path().join("staging"),
                target_namespace: "actors/fx_fixture".into(),
                texture_fallbacks: BTreeMap::new(),
                inputs: vec![CreatureNifInput {
                    role: CreatureNifRole::Body,
                    source_data_relative_path: format!("meshes/{relative}"),
                    source_owner: "fixture".into(),
                    body_variant: None,
                }],
            };
            let inputs = prepare_inputs(&request, "skyrimse", &request.target_namespace).unwrap();
            let input = &inputs[0];
            let path = resolve_case_insensitive(&source_root, &input.source_relative).unwrap();
            let mut nif = NifFile::load(path).unwrap();
            let mut artifacts = BTreeMap::new();
            assert!(
                stage(
                    &mut nif,
                    &source_root,
                    &request.target_namespace,
                    input,
                    &mut artifacts
                )
                .unwrap_or_else(|error| panic!("{relative}: {error}"))
            );
            assert!(artifacts.len() >= 3, "{relative}: incomplete project");
            for artifact in artifacts.values() {
                validate_hkx(&artifact.bytes).unwrap();
            }
            for block in nif
                .blocks
                .iter()
                .filter(|block| block.type_name == "BSBehaviorGraphExtraData")
            {
                let Some(NifValue::String(path)) = block.get_field("Behaviour Graph File") else {
                    panic!("missing attached project")
                };
                assert!(artifacts.contains_key(&canonical_runtime_path(path, "meshes").unwrap()));
            }
        }
    }

    #[test]
    #[ignore = "requires extracted Skyrim Spriggan body and attached FX assets"]
    fn source_spriggan_body_stages_complete_fx_closure_and_controller_sequences() {
        let source_root = std::env::var_os("MODKIT_SKYRIM_MESH_ROOT")
            .map(PathBuf::from)
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/skyrimse")
            });
        let temporary = tempfile::tempdir().unwrap();
        let request = CreatureClosureRequest {
            source_game: "skyrimse".into(),
            source_data_root: source_root.clone(),
            private_staging_root: temporary.path().join("staging"),
            target_namespace: "actors/fx_fixture".into(),
            texture_fallbacks: BTreeMap::new(),
            inputs: vec![CreatureNifInput {
                role: CreatureNifRole::Body,
                source_data_relative_path: "meshes/actors/spriggan/character assets/spriggan.nif"
                    .into(),
                source_owner: "fixture".into(),
                body_variant: None,
            }],
        };
        let prepared = prepare_inputs(&request, "skyrimse", &request.target_namespace).unwrap();
        let input = &prepared[0];
        let source_path = resolve_case_insensitive(&source_root, &input.source_relative).unwrap();
        let source = NifFile::load(source_path.clone()).unwrap();
        let target = temporary.path().join("body.nif");
        stage_preserve_source_rig_nif(
            SourceRigNifKind::Body,
            &source_path,
            &target,
            &input.target_relative,
            "skyrimse",
            Some(&temporary.path().join("materials")),
            &ConvertFileOptions {
                skin_policy: crate::skin::LegacySkinPolicy::PreserveSourceRig,
                ..Default::default()
            },
        )
        .unwrap();
        let mut converted = NifFile::load(target.clone()).unwrap();
        validate_sequences(&source, &converted).unwrap();
        let mut artifacts = BTreeMap::new();
        assert!(
            stage(
                &mut converted,
                &source_root,
                &request.target_namespace,
                input,
                &mut artifacts
            )
            .unwrap()
        );
        assert_eq!(artifacts.len(), 4);
        assert!(
            artifacts.keys().all(
                |path| path.starts_with("meshes/actors/fx_fixture/uniquebehaviors/sprigganfx/")
            )
        );
        converted.save(Some(target.clone())).unwrap();
        let reread = NifFile::load(target).unwrap();
        validate_sequences(&source, &reread).unwrap();
        let project = reread
            .blocks
            .iter()
            .find(|block| block.type_name == "BSBehaviorGraphExtraData")
            .unwrap();
        let Some(NifValue::String(path)) = project.get_field("Behaviour Graph File") else {
            panic!("missing attached project");
        };
        assert!(artifacts.contains_key(&canonical_runtime_path(path, "meshes").unwrap()));
        let graph = artifacts
            .values()
            .find(|artifact| {
                artifact
                    .target_relative
                    .ends_with("behaviors/sprigganfx01behavior.hkx")
            })
            .unwrap();
        let xml = havok_native::api::havok_hkx_to_xml(&graph.bytes).unwrap();
        for event in ["KillFX", "Revive", "LeavesScared", "LeavesLanding"] {
            assert!(xml.contains(event));
        }
    }
}
