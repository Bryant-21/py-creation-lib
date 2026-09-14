use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use nif_core_native::creature_closure::{
    CreatureArtifactKind, CreatureClosureReceipt, CreatureClosureRequest,
    CreatureCollisionDisposition, CreatureInputDispositionKind, CreatureNifInput, CreatureNifRole,
    CreatureTerminalKind, compute_creature_closure_request_blake3,
    seal_embedded_creature_collision, stage_creature_nif_closure,
};
use nif_core_native::model::{NifFile, NifValue};

fn input(role: CreatureNifRole, path: &str, owner: &str) -> CreatureNifInput {
    CreatureNifInput {
        role,
        source_data_relative_path: path.to_string(),
        source_owner: owner.to_string(),
        body_variant: (role == CreatureNifRole::Body).then(|| "default".to_string()),
    }
}

fn request(
    source_game: &str,
    data_root: &Path,
    staging_root: &Path,
    inputs: Vec<CreatureNifInput>,
) -> CreatureClosureRequest {
    CreatureClosureRequest {
        source_game: source_game.to_string(),
        source_data_root: data_root.to_path_buf(),
        private_staging_root: staging_root.to_path_buf(),
        target_namespace: "TestCreature".to_string(),
        texture_fallbacks: std::collections::BTreeMap::new(),
        inputs,
    }
}

fn write_minimal_nif(data_root: &Path, relative: &str, game: &str) -> PathBuf {
    let path = data_root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    fs::create_dir_all(path.parent().expect("NIF parent")).expect("create NIF parent");
    let mut nif = NifFile::new(game);
    nif.rebuild_header();
    nif.save(Some(path.clone())).expect("write synthetic NIF");
    path
}

fn one_pixel_dds() -> Vec<u8> {
    let mut bytes = vec![0_u8; 132];
    bytes[..4].copy_from_slice(b"DDS ");
    bytes[4..8].copy_from_slice(&124_u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&0x1007_u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&1_u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&1_u32.to_le_bytes());
    bytes[76..80].copy_from_slice(&32_u32.to_le_bytes());
    bytes[80..84].copy_from_slice(&0x41_u32.to_le_bytes());
    bytes[88..92].copy_from_slice(&32_u32.to_le_bytes());
    bytes[92..96].copy_from_slice(&0x00ff_0000_u32.to_le_bytes());
    bytes[96..100].copy_from_slice(&0x0000_ff00_u32.to_le_bytes());
    bytes[100..104].copy_from_slice(&0x0000_00ff_u32.to_le_bytes());
    bytes[104..108].copy_from_slice(&0xff00_0000_u32.to_le_bytes());
    bytes[108..112].copy_from_slice(&0x1000_u32.to_le_bytes());
    bytes
}

fn dxt1_dds(width: u32, height: u32, mip_count: u32) -> Vec<u8> {
    let payload_len = (0..mip_count)
        .map(|mip| {
            let width = u64::from((width >> mip).max(1));
            let height = u64::from((height >> mip).max(1));
            width.div_ceil(4) * height.div_ceil(4) * 8
        })
        .sum::<u64>() as usize;
    let mut bytes = vec![0_u8; 128 + payload_len];
    bytes[..4].copy_from_slice(b"DDS ");
    bytes[4..8].copy_from_slice(&124_u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&0x2_1007_u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&height.to_le_bytes());
    bytes[16..20].copy_from_slice(&width.to_le_bytes());
    bytes[28..32].copy_from_slice(&mip_count.to_le_bytes());
    bytes[76..80].copy_from_slice(&32_u32.to_le_bytes());
    bytes[80..84].copy_from_slice(&0x4_u32.to_le_bytes());
    bytes[84..88].copy_from_slice(b"DXT1");
    bytes[108..112].copy_from_slice(&0x401008_u32.to_le_bytes());
    bytes
}

fn write_skyrim_material_nif(data_root: &Path, texture_exists: bool) {
    write_skyrim_material_nif_at(
        data_root,
        "meshes/actors/wolf/body.nif",
        r"textures\actors\wolf\wolf_d.dds",
        0,
        0,
    );
    if texture_exists {
        let texture = data_root.join("textures/actors/wolf/wolf_d.dds");
        fs::create_dir_all(texture.parent().unwrap()).unwrap();
        fs::write(texture, one_pixel_dds()).unwrap();
    }
}

fn write_skyrim_material_nif_at(
    data_root: &Path,
    relative: &str,
    texture_path: &str,
    texture_slot: usize,
    shader_flags: u32,
) {
    let path = data_root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut nif = NifFile::new("skyrimse");
    let mut textures = vec![NifValue::String(String::new()); 9];
    textures[texture_slot] = NifValue::String(texture_path.to_string());
    let texture_set = nif.add_block(
        "BSShaderTextureSet",
        Some(IndexMap::from([
            ("Num Textures".to_string(), NifValue::UInt(9)),
            ("Textures".to_string(), NifValue::Array(textures)),
        ])),
    );
    nif.add_block(
        "BSLightingShaderProperty",
        Some(IndexMap::from([
            ("Name".to_string(), NifValue::String(String::new())),
            ("Texture Set".to_string(), NifValue::Ref(texture_set as i32)),
            (
                "Shader Flags 1:SK".to_string(),
                NifValue::UInt(shader_flags.into()),
            ),
            ("Shader Flags 2:SK".to_string(), NifValue::UInt(0)),
        ])),
    );
    nif.rebuild_header();
    nif.save(Some(path)).unwrap();
}

#[test]
fn stages_synthetic_skyrim_fnv_and_fo3_nifs_with_conversion_provenance() {
    for game in ["skyrimse", "fnv", "fo3"] {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("source-data");
        write_minimal_nif(&data, "meshes/actors/test/body.nif", game);
        write_minimal_nif(&data, "meshes/actors/test/skeleton.nif", game);
        let stage = temp.path().join("stage");
        let receipt = stage_creature_nif_closure(&request(
            game,
            &data,
            &stage,
            vec![
                input(
                    CreatureNifRole::Body,
                    "meshes/actors/test/body.nif",
                    "base.esm",
                ),
                input(
                    CreatureNifRole::Skeleton,
                    "meshes/actors/test/skeleton.nif",
                    "base.esm",
                ),
            ],
        ))
        .unwrap_or_else(|failure| panic!("{game}: {failure}"));

        assert_eq!(receipt.receipt_version, 2);
        assert_eq!(receipt.source_game, game);
        assert!(receipt.inputs.iter().all(|entry| {
            entry.disposition == CreatureInputDispositionKind::ConvertedPreserveSourceRig
        }));
        assert_eq!(
            receipt
                .artifacts
                .iter()
                .filter(|artifact| artifact.kind == CreatureArtifactKind::Nif)
                .count(),
            2
        );
        for artifact in &receipt.artifacts {
            assert!(
                stage
                    .join("data")
                    .join(
                        artifact
                            .target_data_relative_path
                            .replace('/', std::path::MAIN_SEPARATOR_STR)
                    )
                    .is_file()
            );
        }
    }
}

#[test]
fn recursively_stages_synthesized_material_and_texture() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    write_skyrim_material_nif(&data, true);
    let receipt = stage_creature_nif_closure(&request(
        "skyrimse",
        &data,
        &temp.path().join("stage"),
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/wolf/body.nif",
            "skyrim.esm",
        )],
    ))
    .expect("stage material closure");

    assert!(
        receipt
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == CreatureArtifactKind::Bgsm)
    );
    assert!(receipt.artifacts.iter().any(|artifact| {
        artifact.kind == CreatureArtifactKind::Dds
            && artifact.target_data_relative_path == "textures/testcreature/actors/wolf/wolf_d.dds"
    }));
}

#[test]
fn skyrim_normal_generates_fo4_normal_and_specgloss_textures() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    write_skyrim_material_nif_at(
        &data,
        "meshes/actors/wolf/body.nif",
        r"textures\actors\wolf\wolf_n.dds",
        1,
        0,
    );
    let normal = data.join("textures/actors/wolf/wolf_n.dds");
    fs::create_dir_all(normal.parent().unwrap()).unwrap();
    directxtex_native::write_dds_float_rgba_image(
        &normal,
        2,
        2,
        &[
            0.5, 0.5, 1.0, 0.25, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 1.0, 0.75, 0.5, 0.5, 1.0, 1.0,
        ],
        "BC3_UNORM",
        false,
    )
    .unwrap();
    let stage = temp.path().join("stage");
    let receipt = stage_creature_nif_closure(&request(
        "skyrimse",
        &data,
        &stage,
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/wolf/body.nif",
            "skyrim.esm",
        )],
    ))
    .expect("stage generated Skyrim material textures");

    for relative in [
        "textures/testcreature/actors/wolf/wolf_n.dds",
        "textures/testcreature/actors/wolf/wolf_s.dds",
    ] {
        assert!(receipt.artifacts.iter().any(|artifact| {
            artifact.kind == CreatureArtifactKind::Dds
                && artifact.target_data_relative_path == relative
        }));
        let bytes = fs::read(stage.join("data").join(relative)).unwrap();
        assert_eq!(&bytes[84..88], b"ATI2");
    }
}

#[test]
fn duplicate_source_preserves_each_owner_and_deduplicates_output() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    write_minimal_nif(&data, "meshes/actors/gecko/body.nif", "fnv");
    let receipt = stage_creature_nif_closure(&request(
        "fnv",
        &data,
        &temp.path().join("stage"),
        vec![
            input(
                CreatureNifRole::Body,
                "meshes/actors/gecko/body.nif",
                "falloutnv.esm",
            ),
            input(
                CreatureNifRole::Body,
                "meshes/actors/gecko/body.nif",
                "gecko-variant.esp",
            ),
        ],
    ))
    .expect("deduplicated closure");

    assert_eq!(receipt.inputs.len(), 2);
    assert!(receipt.inputs.iter().any(|entry| {
        entry.disposition == CreatureInputDispositionKind::DeduplicatedPreserveSourceRig
    }));
    let nif = receipt
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == CreatureArtifactKind::Nif)
        .unwrap();
    assert_eq!(nif.source_inputs.len(), 2);
}

#[test]
fn missing_texture_is_typed_and_failure_publishes_no_data_tree() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    write_skyrim_material_nif(&data, false);
    let stage = temp.path().join("stage");
    let failure = stage_creature_nif_closure(&request(
        "skyrimse",
        &data,
        &stage,
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/wolf/body.nif",
            "skyrim.esm",
        )],
    ))
    .expect_err("missing texture");

    assert_eq!(failure.kind, CreatureTerminalKind::MissingTexture);
    assert!(failure.input_key.is_some());
    assert!(!stage.join("data").exists());
    assert!(
        fs::read_dir(&stage)
            .expect("private staging root")
            .next()
            .is_none()
    );
}

#[test]
fn explicit_texture_fallback_stages_bytes_at_the_declared_target_path() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    write_skyrim_material_nif(&data, false);
    let fallback = data.join("textures/actors/wolf/wolf_base_d.dds");
    fs::create_dir_all(fallback.parent().unwrap()).unwrap();
    let fallback_bytes = one_pixel_dds();
    fs::write(&fallback, &fallback_bytes).unwrap();
    let stage = temp.path().join("stage");
    let mut request = request(
        "skyrimse",
        &data,
        &stage,
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/wolf/body.nif",
            "skyrim.esm",
        )],
    );
    request.texture_fallbacks.insert(
        "textures/actors/wolf/wolf_d.dds".to_string(),
        "textures/actors/wolf/wolf_base_d.dds".to_string(),
    );

    let receipt = stage_creature_nif_closure(&request).expect("fallback texture closure");
    let artifact = receipt
        .artifacts
        .iter()
        .find(|artifact| {
            artifact.kind == CreatureArtifactKind::Dds
                && artifact.target_data_relative_path
                    == "textures/testcreature/actors/wolf/wolf_d.dds"
        })
        .expect("declared target texture artifact");
    assert_eq!(
        fs::read(stage.join("data").join(&artifact.target_data_relative_path)).unwrap(),
        fallback_bytes
    );
}

#[test]
fn missing_external_material_is_typed() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    let path = write_minimal_nif(&data, "meshes/actors/gecko/body.nif", "fnv");
    let mut nif = NifFile::load(path.clone()).unwrap();
    let shader = nif.add_block(
        "BSLightingShaderProperty",
        Some(IndexMap::from([(
            "Name".to_string(),
            NifValue::String(r"materials\actors\gecko\missing.bgsm".to_string()),
        )])),
    );
    let shape = nif.add_block(
        "BSTriShape",
        Some(IndexMap::from([
            ("Name".to_string(), NifValue::String("Gecko:0".to_string())),
            ("Skin".to_string(), NifValue::Ref(-1)),
            ("Shader Property".to_string(), NifValue::Ref(shader as i32)),
            ("Alpha Property".to_string(), NifValue::Ref(-1)),
        ])),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(shape as i32)]),
    );
    nif.rebuild_header();
    nif.save(Some(path)).unwrap();

    let failure = stage_creature_nif_closure(&request(
        "fnv",
        &data,
        &temp.path().join("stage"),
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/gecko/body.nif",
            "falloutnv.esm",
        )],
    ))
    .expect_err("missing material");
    assert_eq!(failure.kind, CreatureTerminalKind::MissingMaterial);
}

#[test]
fn static_collision_is_typed_and_one_nif_can_fill_body_and_skeleton_roles() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    let path = write_minimal_nif(&data, "meshes/actors/test/body.nif", "fnv");
    let mut nif = NifFile::load(path.clone()).unwrap();
    nif.add_block("bhkCollisionObject", None);
    nif.rebuild_header();
    nif.save(Some(path.clone())).unwrap();
    let failure = stage_creature_nif_closure(&request(
        "fnv",
        &data,
        &temp.path().join("collision-stage"),
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/test/body.nif",
            "base.esm",
        )],
    ))
    .expect_err("static collision");
    assert_eq!(
        failure.kind,
        CreatureTerminalKind::UnsupportedCollisionClass
    );

    write_minimal_nif(&data, "meshes/actors/test/body.nif", "fnv");
    let receipt = stage_creature_nif_closure(&request(
        "fnv",
        &data,
        &temp.path().join("path-stage"),
        vec![
            input(
                CreatureNifRole::Body,
                "meshes/actors/test/body.nif",
                "base.esm",
            ),
            input(
                CreatureNifRole::Skeleton,
                "meshes/actors/test/body.nif",
                "base.esm",
            ),
        ],
    ))
    .expect("dual-role NIF closure");
    assert_eq!(receipt.inputs.len(), 2);
    assert_eq!(
        receipt
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == CreatureArtifactKind::Nif)
            .count(),
        1
    );
    assert!(
        receipt
            .inputs
            .iter()
            .any(|entry| entry.role == CreatureNifRole::Body)
    );
    assert!(
        receipt
            .inputs
            .iter()
            .any(|entry| entry.role == CreatureNifRole::Skeleton)
    );
    assert_eq!(
        receipt
            .inputs
            .iter()
            .filter(|entry| {
                entry.disposition == CreatureInputDispositionKind::ConvertedPreserveSourceRig
            })
            .count(),
        1
    );
    assert_eq!(
        receipt
            .inputs
            .iter()
            .filter(|entry| {
                entry.disposition == CreatureInputDispositionKind::DeduplicatedPreserveSourceRig
            })
            .count(),
        1
    );
}

#[test]
fn articulated_collision_is_deferred_with_an_explicit_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    let path = write_minimal_nif(&data, "meshes/actors/gecko/body.nif", "fnv");
    let mut nif = NifFile::load(path.clone()).unwrap();
    nif.add_block("bhkRagdollConstraint", None);
    nif.rebuild_header();
    nif.save(Some(path)).unwrap();

    let receipt = stage_creature_nif_closure(&request(
        "fnv",
        &data,
        &temp.path().join("stage"),
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/gecko/body.nif",
            "falloutnv.esm",
        )],
    ))
    .expect("deferred articulated collision");
    assert!(matches!(
        &receipt.inputs[0].collision,
        CreatureCollisionDisposition::ArticulatedDeferredForHkx { source_block_types }
            if source_block_types == &["bhkRagdollConstraint"]
    ));
}

#[test]
fn embedded_articulated_collision_updates_and_validates_the_closure_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    let source_path = write_minimal_nif(&data, "meshes/actors/gecko/skeleton.nif", "fnv");
    let mut source = NifFile::load(source_path.clone()).unwrap();
    source.add_block("bhkRagdollConstraint", None);
    source.rebuild_header();
    source.save(Some(source_path)).unwrap();
    let stage = temp.path().join("stage");
    let mut receipt = stage_creature_nif_closure(&request(
        "fnv",
        &data,
        &stage,
        vec![
            input(
                CreatureNifRole::Body,
                "meshes/actors/gecko/skeleton.nif",
                "falloutnv.esm",
            ),
            input(
                CreatureNifRole::Skeleton,
                "meshes/actors/gecko/skeleton.nif",
                "falloutnv.esm",
            ),
        ],
    ))
    .expect("deferred articulated skeleton");
    let skeleton_input = receipt
        .inputs
        .iter()
        .find(|input| input.role == CreatureNifRole::Skeleton)
        .expect("skeleton receipt input");
    let target = skeleton_input.target_data_relative_path.clone();
    let staged_path = stage
        .join("data")
        .join(target.replace('/', std::path::MAIN_SEPARATOR_STR));
    let mut staged = NifFile::load(&staged_path).unwrap();
    staged.add_block("bhkRagdollSystem", None);
    staged.add_block("bhkNPCollisionObject", None);
    staged.add_block("bhkNPCollisionObject", None);
    staged.rebuild_header();
    staged.save(Some(staged_path)).unwrap();

    seal_embedded_creature_collision(&mut receipt, &stage.join("data"), &target, 2)
        .expect("seal embedded articulated collision");

    assert!(matches!(
        &receipt
            .inputs
            .iter()
            .find(|input| input.role == CreatureNifRole::Skeleton)
            .unwrap()
            .collision,
        CreatureCollisionDisposition::ArticulatedEmbeddedFo4 {
            source_block_types,
            body_count: 2,
        } if source_block_types == &["bhkRagdollConstraint"]
    ));
    receipt.validate().expect("updated receipt validates");
}

#[test]
fn unsupported_block_and_material_classes_are_typed_per_input() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    let path = write_minimal_nif(&data, "meshes/actors/wolf/body.nif", "skyrimse");
    let mut nif = NifFile::load(path.clone()).unwrap();
    nif.add_block("BSDynamicTriShape", None);
    nif.rebuild_header();
    nif.save(Some(path)).unwrap();
    let failure = stage_creature_nif_closure(&request(
        "skyrimse",
        &data,
        &temp.path().join("block-stage"),
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/wolf/body.nif",
            "skyrim.esm",
        )],
    ))
    .expect_err("unsupported block");
    assert_eq!(failure.kind, CreatureTerminalKind::UnsupportedBlockClass);
    assert!(failure.input_key.is_some());

    let path = write_minimal_nif(&data, "meshes/actors/gecko/body.nif", "fnv");
    let mut nif = NifFile::load(path.clone()).unwrap();
    nif.add_block(
        "BSLightingShaderProperty",
        Some(IndexMap::from([(
            "Name".to_string(),
            NifValue::String(r"materials\actors\gecko\unsupported.mat".to_string()),
        )])),
    );
    nif.rebuild_header();
    nif.save(Some(path)).unwrap();
    let failure = stage_creature_nif_closure(&request(
        "fnv",
        &data,
        &temp.path().join("material-stage"),
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/gecko/body.nif",
            "falloutnv.esm",
        )],
    ))
    .expect_err("unsupported material");
    assert_eq!(failure.kind, CreatureTerminalKind::UnsupportedMaterialClass);
    assert!(failure.input_key.is_some());
}

#[test]
fn canonical_receipt_and_hash_do_not_depend_on_request_order_or_private_root() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    write_minimal_nif(&data, "meshes/actors/test/a.nif", "fo3");
    write_minimal_nif(&data, "meshes/actors/test/b.nif", "fo3");
    let first = input(
        CreatureNifRole::Body,
        "meshes/actors/test/a.nif",
        "fallout3.esm",
    );
    let second = input(
        CreatureNifRole::Body,
        "meshes/actors/test/b.nif",
        "fallout3.esm",
    );

    let left = stage_creature_nif_closure(&request(
        "fo3",
        &data,
        &temp.path().join("left"),
        vec![first.clone(), second.clone()],
    ))
    .unwrap();
    let right = stage_creature_nif_closure(&request(
        "fo3",
        &data,
        &temp.path().join("right"),
        vec![second, first],
    ))
    .unwrap();

    assert_eq!(left.receipt_hash, right.receipt_hash);
    assert_eq!(
        left.canonical_json().unwrap(),
        right.canonical_json().unwrap()
    );
}

#[test]
fn strict_receipt_parser_rejects_tampering_and_noncanonical_json() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    write_minimal_nif(&data, "meshes/actors/test/body.nif", "fnv");
    let stage = temp.path().join("stage");
    let receipt = stage_creature_nif_closure(&request(
        "fnv",
        &data,
        &stage,
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/test/body.nif",
            "falloutnv.esm",
        )],
    ))
    .unwrap();
    let canonical = receipt.canonical_json().unwrap();
    let parsed = CreatureClosureReceipt::parse_and_validate(&canonical).unwrap();
    assert_eq!(parsed, receipt);
    assert_eq!(receipt.hash_algorithm, "blake3");
    assert_eq!(receipt.request_blake3.len(), 64);
    assert!(receipt.inputs.iter().all(|input| {
        input.source_byte_len > 0
            && input.source_blake3.len() == 64
            && input
                .source_blake3
                .chars()
                .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
    }));
    assert!(
        receipt
            .receipt_hash
            .chars()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
    );
    assert_eq!(receipt.receipt_hash.len(), 64);
    assert!(
        receipt
            .artifacts
            .iter()
            .all(|artifact| artifact.fingerprint.len() == 64)
    );
    let artifact = &receipt.artifacts[0];
    let artifact_bytes = fs::read(
        stage.join("data").join(
            artifact
                .target_data_relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        ),
    )
    .unwrap();
    receipt
        .verify_artifact_bytes(&artifact.target_data_relative_path, &artifact_bytes)
        .unwrap();
    let mut modified_bytes = artifact_bytes;
    modified_bytes[0] ^= 1;
    let failure = receipt
        .verify_artifact_bytes(&artifact.target_data_relative_path, &modified_bytes)
        .expect_err("tampered artifact bytes");
    assert_eq!(failure.kind, CreatureTerminalKind::InvalidReceipt);

    let mut tampered = receipt.clone();
    tampered.artifacts[0].byte_len += 1;
    let failure = CreatureClosureReceipt::parse_and_validate(&tampered.canonical_json().unwrap())
        .expect_err("tampered receipt");
    assert_eq!(failure.kind, CreatureTerminalKind::InvalidReceipt);

    let mut source_tampered = receipt.clone();
    let replacement = if source_tampered.inputs[0].source_blake3.starts_with('0') {
        "1"
    } else {
        "0"
    };
    source_tampered.inputs[0]
        .source_blake3
        .replace_range(0..1, replacement);
    let failure =
        CreatureClosureReceipt::parse_and_validate(&source_tampered.canonical_json().unwrap())
            .expect_err("tampered source fingerprint");
    assert_eq!(failure.kind, CreatureTerminalKind::InvalidReceipt);

    let failure = CreatureClosureReceipt::parse_and_validate(&(canonical + "\n"))
        .expect_err("noncanonical receipt JSON");
    assert_eq!(failure.kind, CreatureTerminalKind::InvalidReceipt);
}

#[test]
fn same_source_path_with_changed_bytes_changes_input_and_request_commitments() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    let relative = "meshes/actors/test/body.nif";
    let source_path = write_minimal_nif(&data, relative, "fnv");
    let first_request = request(
        "fnv",
        &data,
        &temp.path().join("first-stage"),
        vec![input(CreatureNifRole::Body, relative, "falloutnv.esm")],
    );
    let first_request_blake3 = compute_creature_closure_request_blake3(&first_request).unwrap();
    let first = stage_creature_nif_closure(&first_request).unwrap();
    assert_eq!(first.request_blake3, first_request_blake3);

    let mut changed = NifFile::load(source_path.clone()).unwrap();
    changed.blocks[0].set_field("Flags", NifValue::UInt(15));
    changed.rebuild_header();
    changed.save(Some(source_path)).unwrap();

    let second_request = request(
        "fnv",
        &data,
        &temp.path().join("second-stage"),
        vec![input(CreatureNifRole::Body, relative, "falloutnv.esm")],
    );
    let second_request_blake3 = compute_creature_closure_request_blake3(&second_request).unwrap();
    let second = stage_creature_nif_closure(&second_request).unwrap();

    assert_eq!(second.request_blake3, second_request_blake3);
    assert_ne!(
        first.inputs[0].source_blake3,
        second.inputs[0].source_blake3
    );
    assert_ne!(
        first.inputs[0].source_byte_len,
        second.inputs[0].source_byte_len
    );
    assert_ne!(first.inputs[0].input_key, second.inputs[0].input_key);
    assert_ne!(first.request_blake3, second.request_blake3);
    assert_ne!(first.receipt_hash, second.receipt_hash);
}

#[test]
fn divergent_synthesized_materials_at_one_target_are_a_typed_collision() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    let left = "meshes/owner-a/meshes/actors/wolf/body.nif";
    let right = "meshes/owner-b/meshes/actors/wolf/body.nif";
    write_skyrim_material_nif_at(&data, left, r"textures\actors\wolf\wolf_d.dds", 0, 0);
    write_skyrim_material_nif_at(&data, right, r"textures\actors\wolf\wolf_alt_d.dds", 0, 0);
    let stage = temp.path().join("stage");
    let failure = stage_creature_nif_closure(&request(
        "skyrimse",
        &data,
        &stage,
        vec![
            input(CreatureNifRole::Body, left, "left.esp"),
            input(CreatureNifRole::Body, right, "right.esp"),
        ],
    ))
    .expect_err("divergent synthesized material target");

    assert_eq!(failure.kind, CreatureTerminalKind::TargetCollision);
    assert!(!stage.join("data").exists());
}

#[test]
fn identical_synthesized_material_dedupe_preserves_recursive_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    let left = "meshes/owner-a/meshes/actors/wolf/body.nif";
    let right = "meshes/owner-b/meshes/actors/wolf/body.nif";
    for relative in [left, right] {
        write_skyrim_material_nif_at(&data, relative, r"textures\actors\wolf\wolf_d.dds", 0, 0);
    }
    let texture = data.join("textures/actors/wolf/wolf_d.dds");
    fs::create_dir_all(texture.parent().unwrap()).unwrap();
    fs::write(texture, one_pixel_dds()).unwrap();
    let receipt = stage_creature_nif_closure(&request(
        "skyrimse",
        &data,
        &temp.path().join("stage"),
        vec![
            input(CreatureNifRole::Body, left, "left.esp"),
            input(CreatureNifRole::Body, right, "right.esp"),
        ],
    ))
    .expect("identical synthesized material dedupe");

    for kind in [CreatureArtifactKind::Bgsm, CreatureArtifactKind::Dds] {
        let dependencies = receipt
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == kind)
            .collect::<Vec<_>>();
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].source_inputs.len(), 2);
        assert!(
            receipt
                .inputs
                .iter()
                .all(|input| dependencies[0].source_inputs.contains(&input.input_key))
        );
    }
}

#[test]
fn truncated_declared_mip_payload_is_a_typed_texture_failure() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    write_skyrim_material_nif(&data, false);
    let texture = data.join("textures/actors/wolf/wolf_d.dds");
    fs::create_dir_all(texture.parent().unwrap()).unwrap();
    let mut truncated = dxt1_dds(4, 4, 3);
    truncated.pop();
    fs::write(texture, truncated).unwrap();
    let stage = temp.path().join("stage");
    let failure = stage_creature_nif_closure(&request(
        "skyrimse",
        &data,
        &stage,
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/wolf/body.nif",
            "skyrim.esm",
        )],
    ))
    .expect_err("truncated mip payload");

    assert_eq!(failure.kind, CreatureTerminalKind::InvalidTexture);
    assert!(failure.message.contains("truncated"));
    assert!(!stage.join("data").exists());
}

#[test]
fn complete_partial_mip_chain_is_valid() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("source-data");
    write_skyrim_material_nif(&data, false);
    let texture = data.join("textures/actors/wolf/wolf_d.dds");
    fs::create_dir_all(texture.parent().unwrap()).unwrap();
    fs::write(texture, dxt1_dds(4, 4, 2)).unwrap();

    let receipt = stage_creature_nif_closure(&request(
        "skyrimse",
        &data,
        &temp.path().join("stage"),
        vec![input(
            CreatureNifRole::Body,
            "meshes/actors/wolf/body.nif",
            "skyrim.esm",
        )],
    ))
    .expect("partial mip chains are valid when every declared level is present");

    assert!(
        receipt
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == CreatureArtifactKind::Dds)
    );
}
