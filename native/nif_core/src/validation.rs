use std::collections::{BTreeSet, HashMap, HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::Path;

use indexmap::IndexMap;
use thiserror::Error;

use crate::model::{NifBlock, NifFile, NifValue};
use crate::schema::{FieldDef, SCHEMA};

const VF_VERTEX_COLORS: u64 = 0x20;
const SLSF1_SPECULAR: u64 = 1 << 0;
const SLSF1_SKINNED: u64 = 1 << 1;
const SLSF1_VERTEX_ALPHA: u64 = 1 << 3;
const SLSF1_GRAYSCALE_COLOR: u64 = 1 << 4;
const SLSF1_GRAYSCALE_ALPHA: u64 = 1 << 5;
const SLSF1_ENVIRONMENT_MAPPING: u64 = 1 << 7;
const SLSF1_FACEGEN: u64 = 1 << 10;
const SLSF1_PARALLAX: u64 = 1 << 11;
const SLSF1_MODEL_SPACE_NORMALS: u64 = 1 << 12;
const SLSF1_EYE_ENVIRONMENT_MAPPING: u64 = 1 << 17;
const SLSF1_HAIR_TINT: u64 = 1 << 18;
const SLSF1_SKIN_TINT: u64 = 1 << 21;
const SLSF1_OWN_EMIT: u64 = 1 << 22;
const SLSF1_DECAL: u64 = 1 << 26;
const SLSF1_DYNAMIC_DECAL: u64 = 1 << 27;
const SLSF1_EXTERNAL_EMITTANCE: u64 = 1 << 29;
const SLSF1_WINDOW_ENVIRONMENT_MAPPING: u64 = 1 << 21;
const SLSF2_VERTEX_COLORS: u64 = 1 << 5;
const SLSF2_GLOW_MAP: u64 = 1 << 6;
const SLSF2_ASSUME_SHADOWMASK: u64 = 1 << 7;
const SLSF2_ENV_MAP_LIGHT_FADE: u64 = 1 << 15;
const SLSF2_ANISOTROPIC_LIGHTING: u64 = 1 << 21;
const SLSF2_MULTI_LAYER_PARALLAX: u64 = 1 << 24;
const SLSF2_SOFT_LIGHTING: u64 = 1 << 25;
const SLSF2_RIM_LIGHTING: u64 = 1 << 26;
const SLSF2_BACK_LIGHTING: u64 = 1 << 27;
const SLSF2_CHARACTER_LIGHTING: u64 = 1 << 8;
const SLSF2_TREE_ANIM: u64 = 1 << 29;

const SHADER_ENVIRONMENT_MAP: u64 = 1;
const SHADER_GLOW: u64 = 2;
const SHADER_PARALLAX: u64 = 3;
const SHADER_FACEGEN: u64 = 4;
const SHADER_SKIN_TINT: u64 = 5;
const SHADER_HAIR_TINT: u64 = 6;
const SHADER_MULTI_LAYER_PARALLAX: u64 = 11;
const SHADER_TREE_ANIM: u64 = 12;
const SHADER_EYE_ENVMAP: u64 = 16;
const FO3_SHADER_SKIN: u64 = 14;
const FO3_SHADER_NOLIGHTING: u64 = 33;

const BSX_ANIMATED: u64 = 1 << 0;
const BSX_HAVOK: u64 = 1 << 1;
const BSX_RAGDOLL: u64 = 1 << 2;
const BSX_COMPLEX: u64 = 1 << 3;
const BSX_ADDON: u64 = 1 << 4;
const BSX_EDITOR_MARKER: u64 = 1 << 5;
const BSX_DYNAMIC: u64 = 1 << 6;
const BSX_ARTICULATED: u64 = 1 << 7;
const BSX_EXTERNAL_EMIT: u64 = 1 << 9;
const BHKCO_SET_LOCAL: u64 = 1 << 3;
const BHKCO_USE_VEL: u64 = 1 << 5;
const OBLIVION_TANGENT_DATA_NAME: &str = "Tangent space (binormal & tangent vectors)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NifGame {
    Morrowind,
    Oblivion,
    Fo3,
    Skyrim,
    SkyrimSe,
    Fo4,
    Fo76,
    Starfield,
    Unknown,
}

impl NifGame {
    fn label(self) -> &'static str {
        match self {
            Self::Morrowind => "morrowind",
            Self::Oblivion => "oblivion",
            Self::Fo3 => "fo3/fnv",
            Self::Skyrim => "skyrim",
            Self::SkyrimSe => "skyrimse",
            Self::Fo4 => "fo4",
            Self::Fo76 => "fo76",
            Self::Starfield => "starfield",
            Self::Unknown => "unknown",
        }
    }

    fn uses_bsx(self) -> bool {
        matches!(
            self,
            Self::Fo3 | Self::Skyrim | Self::SkyrimSe | Self::Fo4 | Self::Fo76 | Self::Starfield
        )
    }

    fn uses_runtime_vertex_color_flags(self) -> bool {
        !matches!(
            self,
            Self::Morrowind | Self::Oblivion | Self::Fo3 | Self::Unknown
        )
    }

    fn uses_skyrim_shader_rules(self) -> bool {
        matches!(self, Self::Skyrim | Self::SkyrimSe)
    }

    fn is_fo4_family(self) -> bool {
        matches!(self, Self::Fo4 | Self::Fo76 | Self::Starfield)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SanitizeReport {
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationFinding {
    pub severity: String,
    pub rule: String,
    pub block_id: Option<usize>,
    pub block_type: Option<String>,
    pub field: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub game: String,
    pub changed: bool,
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
    pub findings: Vec<ValidationFinding>,
}

#[derive(Debug, Error)]
pub enum ValidationFileError {
    #[error("read: {0}")]
    Read(#[from] crate::io::ReadError),
    #[error("write: {0}")]
    Write(#[from] crate::io::WriteError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub fn validate_nif_file(
    input: &Path,
    output: Option<&Path>,
    fix: bool,
) -> Result<ValidationReport, ValidationFileError> {
    validate_nif_file_with_options(input, output, fix, false)
}

pub fn validate_nif_file_with_options(
    input: &Path,
    output: Option<&Path>,
    fix: bool,
    include_optional: bool,
) -> Result<ValidationReport, ValidationFileError> {
    let mut nif = NifFile::load(input.to_path_buf())?;
    let mut report = validate_nif_with_options(&mut nif, fix, include_optional);
    audit_morph_sibling(input, &nif, &mut report.findings);
    if fix {
        let target = output.unwrap_or(input);
        if report.changed || target != input {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            nif.save(Some(target.to_path_buf()))?;
        }
    }
    Ok(report)
}

fn audit_morph_sibling(path: &Path, nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    if !matches!(nif_game(nif), NifGame::Skyrim | NifGame::SkyrimSe)
        || !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().ends_with("_0.nif"))
        || !nif
            .blocks
            .iter()
            .any(|block| SCHEMA.is_subtype_of(&block.type_name, "NiSkinInstance"))
    {
        return;
    }
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let sibling_name = format!("{}_1.nif", &file_name[..file_name.len() - 6]);
    let sibling_path = path.with_file_name(sibling_name);
    let Ok(sibling) = NifFile::load(sibling_path) else {
        return;
    };
    let owner = root_ids(nif)
        .into_iter()
        .next()
        .and_then(|root| nif.get_block(root))
        .or_else(|| nif.blocks.first());
    let Some(owner) = owner else {
        return;
    };
    let bone_names = |file: &NifFile| {
        let mut names = file
            .blocks
            .iter()
            .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiNode"))
            .filter_map(|block| string_field(block, "Name"))
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
    };
    let bones_0 = bone_names(nif);
    let bones_1 = bone_names(&sibling);
    if bones_0.len() != bones_1.len() {
        findings.push(finding(
            "error",
            "morph-model-parity",
            owner,
            None,
            format!(
                "Bone counts differ between _0 and _1 morph models ({} vs {})",
                bones_0.len(),
                bones_1.len()
            ),
        ));
    } else if bones_0 != bones_1 {
        findings.push(finding(
            "error",
            "morph-model-parity",
            owner,
            None,
            "Bone names differ between _0 and _1 morph models".to_string(),
        ));
    }

    let shapes_0 = morph_shapes(nif);
    let shapes_1 = morph_shapes(&sibling);
    if shapes_0.len() != shapes_1.len() {
        findings.push(finding(
            "error",
            "morph-model-parity",
            owner,
            None,
            format!(
                "Shape counts differ between _0 and _1 morph models ({} vs {})",
                shapes_0.len(),
                shapes_1.len()
            ),
        ));
        return;
    }
    for ((name_0, partitions_0), (name_1, partitions_1)) in shapes_0.into_iter().zip(shapes_1) {
        if name_0 != name_1 {
            findings.push(finding(
                "error",
                "morph-model-parity",
                owner,
                None,
                format!("Shape names differ between morph models: {name_0:?} vs {name_1:?}"),
            ));
        } else if partitions_0 != partitions_1 {
            findings.push(finding(
                "error",
                "morph-model-parity",
                owner,
                None,
                format!("Skin partition vertex counts differ for shape {name_0:?}"),
            ));
        }
    }
}

fn morph_shapes(nif: &NifFile) -> Vec<(String, Vec<usize>)> {
    let mut shapes = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "BSTriShape"))
        .collect::<Vec<_>>();
    if shapes.is_empty() {
        shapes = nif
            .blocks
            .iter()
            .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom"))
            .collect();
    }
    shapes
        .into_iter()
        .map(|shape| {
            let partitions = skin_ref(shape)
                .and_then(|skin| nif.get_block(skin as usize))
                .and_then(|skin| value_ref(skin.get_field("Skin Partition")))
                .filter(|partition| *partition >= 0)
                .and_then(|partition| nif.get_block(partition as usize))
                .map(|partition| {
                    value_array(partition.get_field("Partitions"))
                        .iter()
                        .map(|entry| nested_u64(Some(entry), "Num Vertices").unwrap_or(0) as usize)
                        .collect()
                })
                .unwrap_or_default();
            (string_field(shape, "Name").unwrap_or_default(), partitions)
        })
        .collect()
}

pub fn validate_nif(nif: &mut NifFile, fix: bool) -> ValidationReport {
    validate_nif_with_options(nif, fix, false)
}

pub fn validate_nif_with_options(
    nif: &mut NifFile,
    fix: bool,
    include_optional: bool,
) -> ValidationReport {
    let sanitize = if fix {
        sanitize_nif(nif)
    } else {
        SanitizeReport::default()
    };
    ValidationReport {
        game: nif_game(nif).label().to_string(),
        changed: !sanitize.changes.is_empty(),
        changes: sanitize.changes,
        warnings: sanitize.warnings,
        findings: audit_nif_with_options(nif, include_optional),
    }
}

pub fn sanitize_nif(nif: &mut NifFile) -> SanitizeReport {
    let mut report = SanitizeReport::default();
    normalize_invalid_string_indices(nif, &mut report);
    collapse_ref_arrays(nif, &mut report);
    remove_empty_shapes(nif, &mut report);
    remove_redundant_properties(nif, &mut report);
    collapse_ref_arrays(nif, &mut report);
    normalize_asset_paths(nif, &mut report);
    normalize_shader_flags(nif, &mut report);
    normalize_bsx_flags(nif, &mut report);
    normalize_hardcoded_names(nif, &mut report);
    normalize_value_node_names(nif, &mut report);
    normalize_animation_metadata(nif, &mut report);
    normalize_collision_targets(nif, &mut report);
    normalize_collision_settings(nif, &mut report);
    normalize_particle_data(nif, &mut report);
    normalize_consistency_flags(nif, &mut report);
    nif.rebuild_header();
    report
}

pub fn sanitize_fo4_nif(nif: &mut NifFile) -> SanitizeReport {
    sanitize_nif(nif)
}

pub fn audit_nif(nif: &NifFile) -> Vec<ValidationFinding> {
    audit_nif_with_options(nif, false)
}

pub fn audit_nif_with_options(nif: &NifFile, include_optional: bool) -> Vec<ValidationFinding> {
    let mut findings = Vec::new();
    audit_invalid_string_indices(nif, &mut findings);
    audit_ref_fields(nif, &mut findings);
    audit_block_order(nif, &mut findings);
    audit_unreachable_blocks(nif, &mut findings);
    audit_duplicate_names(nif, &mut findings);
    audit_hardcoded_names(nif, &mut findings);
    audit_value_node_names(nif, &mut findings);
    audit_animation(nif, &mut findings);
    audit_particle_lifetimes(nif, &mut findings);
    audit_particle_systems(nif, &mut findings);
    audit_collision(nif, &mut findings);
    audit_collision_mopp(nif, &mut findings);
    audit_consistency_flags(nif, &mut findings);
    audit_skinning(nif, &mut findings);
    audit_geometry(nif, &mut findings);
    audit_asset_paths(nif, &mut findings);
    audit_texture_set_slots(nif, &mut findings);
    audit_shader_flags(nif, &mut findings);
    audit_bsx_flags(nif, &mut findings);
    audit_miscellaneous(nif, &mut findings);
    if include_optional {
        audit_optional_nif(nif, &mut findings);
    }
    findings
}

fn audit_invalid_string_indices(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    for block in &nif.blocks {
        let definitions = SCHEMA.get_all_fields(&block.type_name);
        for (name, value) in &block.fields {
            let Some(definition) = definitions
                .iter()
                .find(|definition| definition.name == bare_name(name))
            else {
                continue;
            };
            audit_string_value(block, name, value, definition, findings);
        }
    }
}

fn audit_string_value(
    owner: &NifBlock,
    path: &str,
    value: &NifValue,
    definition: &FieldDef,
    findings: &mut Vec<ValidationFinding>,
) {
    let type_name = if definition.type_name == "#T#" {
        definition.template.unwrap_or(definition.type_name)
    } else {
        definition.type_name
    };
    if matches!(type_name, "string" | "NiFixedString") {
        let values = match value {
            NifValue::Array(values) => values.as_slice(),
            _ => std::slice::from_ref(value),
        };
        for (index, value) in values.iter().enumerate() {
            if let NifValue::Int(string_index) = value
                && *string_index >= 0
            {
                findings.push(finding(
                    "error",
                    "invalid-string-index",
                    owner,
                    Some(if values.len() > 1 {
                        format!("{path}[{index}]")
                    } else {
                        path.to_string()
                    }),
                    format!("String index {string_index} is outside the string table"),
                ));
            }
        }
        return;
    }
    let Some(structure) = SCHEMA.get_struct(type_name) else {
        return;
    };
    let values = match value {
        NifValue::Struct(_) => std::slice::from_ref(value),
        NifValue::Array(values) => values.as_slice(),
        _ => return,
    };
    for (index, value) in values.iter().enumerate() {
        let NifValue::Struct(fields) = value else {
            continue;
        };
        let prefix = if values.len() > 1 {
            format!("{path}[{index}]")
        } else {
            path.to_string()
        };
        for (name, value) in fields {
            if let Some(definition) = structure
                .fields
                .iter()
                .find(|definition| definition.name == bare_name(name))
            {
                audit_string_value(
                    owner,
                    &format!("{prefix}.{name}"),
                    value,
                    definition,
                    findings,
                );
            }
        }
    }
}

fn nif_game(nif: &NifFile) -> NifGame {
    match (
        nif.header.version,
        nif.header.user_version,
        nif.header.bs_version,
    ) {
        ((4, 0, 0, 2), _, _) => NifGame::Morrowind,
        ((20, 0, 0, 5), 0 | 11, 0 | 11)
        | ((20, 0, 0, 4), 10 | 11, 11)
        | ((10, 1, 0, 106), 10, 5)
        | ((10, 2, 0, 0), 10, 6 | 7 | 8 | 9 | 11) => NifGame::Oblivion,
        ((20, 2, 0, 7), 11, _) => NifGame::Fo3,
        ((20, 2, 0, 7), 12, 83) => NifGame::Skyrim,
        ((20, 2, 0, 7), 12, 100) => NifGame::SkyrimSe,
        ((20, 2, 0, 7), 12, 130 | 132) => NifGame::Fo4,
        ((20, 2, 0, 7), 12, 155) => NifGame::Fo76,
        ((20, 2, 0, 7), 12, version) if version >= 170 => NifGame::Starfield,
        _ => NifGame::Unknown,
    }
}

pub fn nif_game_label(nif: &NifFile) -> &'static str {
    nif_game(nif).label()
}

fn normalize_invalid_string_indices(nif: &mut NifFile, report: &mut SanitizeReport) {
    let mut changed = 0usize;
    for block in &mut nif.blocks {
        let definitions = SCHEMA.get_all_fields(&block.type_name);
        for (name, value) in &mut block.fields {
            let Some(definition) = definitions
                .iter()
                .find(|definition| definition.name == bare_name(name))
            else {
                continue;
            };
            normalize_string_value(value, definition, &mut changed);
        }
    }
    if changed > 0 {
        report.changes.push(format!(
            "Strings: replaced {changed} invalid string-table index(es) with None"
        ));
    }
}

fn normalize_string_value(value: &mut NifValue, definition: &FieldDef, changed: &mut usize) {
    let type_name = if definition.type_name == "#T#" {
        definition.template.unwrap_or(definition.type_name)
    } else {
        definition.type_name
    };
    if matches!(type_name, "string" | "NiFixedString") {
        match value {
            NifValue::Int(index) if *index >= 0 => {
                *value = NifValue::Null;
                *changed += 1;
            }
            NifValue::Array(values) => {
                for value in values {
                    if matches!(value, NifValue::Int(index) if *index >= 0) {
                        *value = NifValue::Null;
                        *changed += 1;
                    }
                }
            }
            _ => {}
        }
        return;
    }
    let Some(structure) = SCHEMA.get_struct(type_name) else {
        return;
    };
    let values = match value {
        NifValue::Struct(_) => std::slice::from_mut(value),
        NifValue::Array(values) => values.as_mut_slice(),
        _ => return,
    };
    for value in values {
        let NifValue::Struct(fields) = value else {
            continue;
        };
        for (name, value) in fields {
            if let Some(definition) = structure
                .fields
                .iter()
                .find(|definition| definition.name == bare_name(name))
            {
                normalize_string_value(value, definition, changed);
            }
        }
    }
}

fn collapse_ref_arrays(nif: &mut NifFile, report: &mut SanitizeReport) {
    let block_count = nif.blocks.len() as i32;
    let eye_center_extra_data = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSEyeCenterExtraData")
        .map(|block| block.block_id as i32)
        .collect::<HashSet<_>>();
    let mut null_links = 0usize;
    let mut broken_links = 0usize;
    let mut repeated_links = 0usize;

    for block in &mut nif.blocks {
        let field_defs = SCHEMA.get_all_fields(&block.type_name);
        let keys = block.fields.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let bare = bare_name(&key);
            let Some(field_def) = field_defs.iter().find(|field| field.name == bare) else {
                continue;
            };
            if !is_direct_ref_field(field_def) || field_def.length.is_none() {
                continue;
            }
            let owner_type = block.type_name.clone();
            let preserve_nulls = allows_sparse_null_links(block, &key);
            let Some(NifValue::Array(values)) = block.fields.get_mut(&key) else {
                continue;
            };
            let old_len = values.len();
            let mut seen = HashSet::new();
            values.retain(|value| {
                let Some(reference) = value_ref(Some(value)) else {
                    return true;
                };
                if reference < 0 {
                    if preserve_nulls {
                        return true;
                    }
                    null_links += 1;
                    return false;
                }
                if reference >= block_count {
                    broken_links += 1;
                    return false;
                }
                if !seen.insert(reference) {
                    if allows_repeated_eye_center_link(
                        &owner_type,
                        &key,
                        eye_center_extra_data.contains(&reference),
                    ) {
                        return true;
                    }
                    repeated_links += 1;
                    return false;
                }
                true
            });
            let new_len = values.len();
            if new_len != old_len
                && let Some(length_field) = simple_length_field(field_def.length)
            {
                block.set_field(length_field, NifValue::UInt(new_len as u64));
            }
        }
    }

    if null_links + broken_links + repeated_links > 0 {
        report.changes.push(format!(
            "Link arrays: removed {null_links} null, {broken_links} broken, and {repeated_links} repeated link(s)"
        ));
    }
}

fn remove_empty_shapes(nif: &mut NifFile, report: &mut SanitizeReport) {
    let shape_ids = nif
        .blocks
        .iter()
        .filter(|block| {
            SCHEMA.is_subtype_of(&block.type_name, "BSTriShape")
                || SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom")
        })
        .filter(|block| skin_ref(block).is_none())
        .filter(|block| {
            ref_values(block.get_field("Extra Data List")).is_empty()
                && value_ref(block.get_field("Extra Data")).unwrap_or(-1) < 0
        })
        .filter(|block| {
            if SCHEMA.is_subtype_of(&block.type_name, "BSTriShape") {
                value_u64(block.get_field("Num Vertices"))
                    .unwrap_or_else(|| value_array(block.get_field("Vertex Data")).len() as u64)
                    == 0
            } else {
                value_ref(block.get_field("Data"))
                    .filter(|data| *data >= 0)
                    .and_then(|data| nif.get_block(data as usize))
                    .and_then(|data| value_u64(data.get_field("Num Vertices")))
                    .unwrap_or(0)
                    == 0
            }
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    if shape_ids.is_empty() {
        return;
    }
    let remove = exclusive_branch_ids(nif, &shape_ids);
    nif.remove_blocks(&remove);
    report.changes.push(format!(
        "Geometry: removed {} empty unskinned shape(s)",
        shape_ids.len()
    ));
}

fn exclusive_branch_ids(nif: &NifFile, roots: &[usize]) -> Vec<usize> {
    let root_set = roots.iter().copied().collect::<HashSet<_>>();
    let mut candidates = HashSet::new();
    let mut stack = roots.to_vec();
    while let Some(block_id) = stack.pop() {
        if !candidates.insert(block_id) {
            continue;
        }
        if let Some(block) = nif.get_block(block_id) {
            stack.extend(
                block
                    .get_refs(&SCHEMA)
                    .into_iter()
                    .filter(|reference| *reference >= 0)
                    .map(|reference| reference as usize)
                    .filter(|reference| !root_set.contains(reference)),
            );
        }
    }

    let protected = candidates
        .iter()
        .copied()
        .filter(|candidate| !root_set.contains(candidate))
        .filter(|candidate| {
            nif.header.footer_roots.contains(&(*candidate as i32))
                || nif.blocks.iter().any(|block| {
                    !candidates.contains(&block.block_id)
                        && block.get_refs(&SCHEMA).contains(&(*candidate as i32))
                })
        })
        .collect::<Vec<_>>();
    let mut keep = HashSet::new();
    let mut stack = protected;
    while let Some(block_id) = stack.pop() {
        if !keep.insert(block_id) {
            continue;
        }
        if let Some(block) = nif.get_block(block_id) {
            stack.extend(
                block
                    .get_refs(&SCHEMA)
                    .into_iter()
                    .filter(|reference| *reference >= 0)
                    .map(|reference| reference as usize)
                    .filter(|reference| !root_set.contains(reference)),
            );
        }
    }
    let mut remove = candidates.difference(&keep).copied().collect::<Vec<_>>();
    remove.sort_unstable();
    remove
}

fn remove_redundant_properties(nif: &mut NifFile, report: &mut SanitizeReport) {
    if matches!(nif_game(nif), NifGame::Unknown) {
        return;
    }
    let mut remove = if nif_game(nif) == NifGame::Morrowind {
        Vec::new()
    } else {
        nif.blocks
            .iter()
            .filter(|block| block.type_name == "NiSpecularProperty")
            .map(|block| block.block_id)
            .collect::<Vec<_>>()
    };
    remove.extend(
        nif.blocks
            .iter()
            .filter(|block| {
                block.type_name == "NiStringExtraData"
                    && string_field(block, "Name").as_deref() == Some("UPB")
                    && !nif.blocks.iter().any(|parent| {
                        string_field(parent, "Name").as_deref() == Some("Backpack")
                            && parent.get_refs(&SCHEMA).contains(&(block.block_id as i32))
                    })
            })
            .map(|block| block.block_id),
    );
    if remove.is_empty() {
        return;
    }
    let specular = remove
        .iter()
        .filter(|id| {
            nif.get_block(**id)
                .is_some_and(|block| block.type_name == "NiSpecularProperty")
        })
        .count();
    let upb = remove.len() - specular;
    nif.remove_blocks(&remove);
    report.changes.push(format!(
        "Redundant blocks: removed {specular} NiSpecularProperty and {upb} unused UPB block(s)"
    ));
}

fn hardcoded_block_name(type_name: &str) -> Option<&'static str> {
    match type_name {
        "BSBehaviorGraphExtraData" => Some("BGED"),
        "BSBoneLODExtraData" => Some("BSBoneLOD"),
        "BSBound" => Some("BBX"),
        "BSClothExtraData" => Some("CED"),
        "BSConnectPoint::Children" => Some("CPT"),
        "BSConnectPoint::Parents" => Some("CPA"),
        "BSDecalPlacementVectorExtraData" => Some("DVPG"),
        "BSDistantObjectLargeRefExtraData" => Some("DOLRED"),
        "BSEyeCenterExtraData" => Some("ECED"),
        "BSFurnitureMarker" | "BSFurnitureMarkerNode" => Some("FRN"),
        "BSInvMarker" => Some("INV"),
        "BSPositionData" => Some("BSPosData"),
        "BSWArray" => Some("BSW"),
        "BSXFlags" => Some("BSX"),
        _ => None,
    }
}

fn normalize_hardcoded_names(nif: &mut NifFile, report: &mut SanitizeReport) {
    let mut renamed = 0usize;
    for block in &mut nif.blocks {
        let Some(expected) = hardcoded_block_name(&block.type_name) else {
            continue;
        };
        if block.get_field("Name").is_some()
            && string_field(block, "Name").as_deref() != Some(expected)
        {
            block.set_field("Name", NifValue::String(expected.to_string()));
            renamed += 1;
        }
    }
    if renamed > 0 {
        report.changes.push(format!(
            "Hardcoded names: normalized {renamed} block name(s)"
        ));
    }
}

fn normalize_asset_paths(nif: &mut NifFile, report: &mut SanitizeReport) {
    let mut changed = 0usize;
    let fo4_materials = matches!(nif_game(nif), NifGame::Fo4 | NifGame::Fo76);
    let prefix_dds = nif_game(nif) != NifGame::Morrowind;
    for block in &mut nif.blocks {
        match block.type_name.as_str() {
            "BSShaderTextureSet" => {
                normalize_asset_field(block, "Textures", prefix_dds, &mut changed);
            }
            "BSLightingShaderProperty" if fo4_materials => {
                if block
                    .get_field("Name")
                    .and_then(value_string)
                    .is_some_and(is_material_path)
                {
                    normalize_asset_field(block, "Name", prefix_dds, &mut changed);
                }
            }
            "BSEffectShaderProperty" => {
                for field in [
                    "Source Texture",
                    "Grayscale Texture",
                    "Greyscale Texture",
                    "Env Map Texture",
                    "Normal Texture",
                    "Env Mask Texture",
                ] {
                    normalize_asset_field(block, field, prefix_dds, &mut changed);
                }
                if fo4_materials
                    && block
                        .get_field("Name")
                        .and_then(value_string)
                        .is_some_and(is_material_path)
                {
                    normalize_asset_field(block, "Name", prefix_dds, &mut changed);
                }
            }
            "BSShaderNoLightingProperty" | "TallGrassShaderProperty" | "TileShaderProperty" => {
                normalize_asset_field(block, "File Name", prefix_dds, &mut changed);
            }
            "BSSkyShaderProperty" => {
                normalize_asset_field(block, "Source Texture", prefix_dds, &mut changed);
            }
            "BSBehaviorGraphExtraData" => {
                normalize_asset_field(block, "Behavior Graph File", prefix_dds, &mut changed);
            }
            "BSSubIndexTriShape" => {
                if let Some(NifValue::Struct(fields)) = block.get_field_mut("Segment Data") {
                    if let Some((_, value)) = fields
                        .iter_mut()
                        .find(|(field, _)| bare_name(field) == "SSF File")
                    {
                        normalize_asset_value(value, prefix_dds, &mut changed);
                    }
                }
            }
            _ if SCHEMA.is_subtype_of(&block.type_name, "NiTexture") => {
                normalize_asset_field(block, "File Name", prefix_dds, &mut changed);
            }
            _ => {}
        }
    }
    if changed > 0 {
        report
            .changes
            .push(format!("Asset paths: normalized {changed} path field(s)"));
    }
}

fn audit_asset_paths(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    let fo4_materials = matches!(nif_game(nif), NifGame::Fo4 | NifGame::Fo76);
    let prefix_dds = nif_game(nif) != NifGame::Morrowind;
    for block in &nif.blocks {
        match block.type_name.as_str() {
            "BSShaderTextureSet" => {
                audit_asset_field(block, "Textures", prefix_dds, findings);
            }
            "BSLightingShaderProperty" if fo4_materials => {
                if block
                    .get_field("Name")
                    .and_then(value_string)
                    .is_some_and(is_material_path)
                {
                    audit_asset_field(block, "Name", prefix_dds, findings);
                }
            }
            "BSEffectShaderProperty" => {
                for field in [
                    "Source Texture",
                    "Grayscale Texture",
                    "Greyscale Texture",
                    "Env Map Texture",
                    "Normal Texture",
                    "Env Mask Texture",
                ] {
                    audit_asset_field(block, field, prefix_dds, findings);
                }
                if fo4_materials
                    && block
                        .get_field("Name")
                        .and_then(value_string)
                        .is_some_and(is_material_path)
                {
                    audit_asset_field(block, "Name", prefix_dds, findings);
                }
            }
            "BSShaderNoLightingProperty" | "TallGrassShaderProperty" | "TileShaderProperty" => {
                audit_asset_field(block, "File Name", prefix_dds, findings);
            }
            "BSSkyShaderProperty" => {
                audit_asset_field(block, "Source Texture", prefix_dds, findings);
            }
            "BSBehaviorGraphExtraData" => {
                audit_asset_field(block, "Behavior Graph File", prefix_dds, findings);
            }
            "BSSubIndexTriShape" => {
                if let Some(NifValue::Struct(fields)) = block.get_field("Segment Data")
                    && let Some((field, value)) = fields
                        .iter()
                        .find(|(field, _)| bare_name(field) == "SSF File")
                {
                    audit_normalizable_asset_value(
                        block,
                        value,
                        &format!("Segment Data.{field}"),
                        prefix_dds,
                        findings,
                    );
                }
            }
            _ if SCHEMA.is_subtype_of(&block.type_name, "NiTexture") => {
                audit_asset_field(block, "File Name", prefix_dds, findings);
            }
            _ => {}
        }
    }
}

fn audit_asset_field(
    block: &NifBlock,
    field: &str,
    prefix_dds: bool,
    findings: &mut Vec<ValidationFinding>,
) {
    if let Some(value) = block.get_field(field) {
        audit_normalizable_asset_value(block, value, field, prefix_dds, findings);
    }
}

fn audit_normalizable_asset_value(
    block: &NifBlock,
    value: &NifValue,
    path: &str,
    prefix_dds: bool,
    findings: &mut Vec<ValidationFinding>,
) {
    match value {
        NifValue::String(asset) => {
            let normalized = normalized_asset_path(asset, prefix_dds);
            if normalized != *asset {
                findings.push(finding(
                    "error",
                    "asset-path",
                    block,
                    Some(path.to_string()),
                    format!("Path should be normalized from {asset:?} to {normalized:?}"),
                ));
            } else if texture_path_is_invalid(asset) {
                findings.push(finding(
                    "error",
                    "asset-path",
                    block,
                    Some(path.to_string()),
                    format!("Invalid or absolute asset path {asset:?}"),
                ));
            }
        }
        NifValue::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                audit_normalizable_asset_value(
                    block,
                    value,
                    &format!("{path}[{index}]"),
                    prefix_dds,
                    findings,
                );
            }
        }
        NifValue::Struct(fields) => {
            for (field, value) in fields {
                audit_normalizable_asset_value(
                    block,
                    value,
                    &format!("{path}.{field}"),
                    prefix_dds,
                    findings,
                );
            }
        }
        _ => {}
    }
}

fn normalize_asset_field(block: &mut NifBlock, field: &str, prefix_dds: bool, changed: &mut usize) {
    if let Some(value) = block.get_field_mut(field) {
        normalize_asset_value(value, prefix_dds, changed);
    }
}

fn is_material_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".bgsm") || lower.ends_with(".bgem")
}

fn normalize_asset_value(value: &mut NifValue, prefix_dds: bool, changed: &mut usize) {
    match value {
        NifValue::String(path) => {
            let normalized = normalized_asset_path(path, prefix_dds);
            if normalized != *path {
                *path = normalized;
                *changed += 1;
            }
        }
        NifValue::Array(values) => {
            for value in values {
                normalize_asset_value(value, prefix_dds, changed);
            }
        }
        NifValue::Struct(fields) => {
            for value in fields.values_mut() {
                normalize_asset_value(value, prefix_dds, changed);
            }
        }
        _ => {}
    }
}

fn normalized_asset_path(value: &str, prefix_dds: bool) -> String {
    if value.contains('\u{8}') && value.contains("NOR") {
        return String::new();
    }
    let mut normalized = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let delimiter = if normalized.contains('\\') { '\\' } else { '/' };
    let other = if delimiter == '\\' { '/' } else { '\\' };
    normalized = normalized.replace(other, &delimiter.to_string());
    let doubled = format!("{delimiter}{delimiter}");
    while normalized.contains(&doubled) {
        normalized = normalized.replace(&doubled, &delimiter.to_string());
    }

    let lower = normalized.to_ascii_lowercase();
    let parts = normalized
        .split(delimiter)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let absolute = normalized.starts_with(delimiter) || normalized.as_bytes().get(1) == Some(&b':');
    if absolute {
        if let Some(index) = parts.iter().rposition(|part| {
            matches!(
                part.to_ascii_lowercase().as_str(),
                "meshes" | "textures" | "materials" | "data" | "data files"
            )
        }) {
            normalized = parts[index..].join(&delimiter.to_string());
        } else {
            return value.to_string();
        }
    } else if prefix_dds && lower.ends_with(".dds") {
        let lower = normalized.to_ascii_lowercase();
        let texture_prefix = format!("textures{delimiter}");
        let data_texture_prefix = format!("data{delimiter}textures{delimiter}");
        if !lower.starts_with(&texture_prefix) && !lower.starts_with(&data_texture_prefix) {
            normalized = format!(
                "textures{delimiter}{}",
                normalized.trim_start_matches(delimiter)
            );
        }
    }
    normalized
}

fn normalize_value_node_names(nif: &mut NifFile, report: &mut SanitizeReport) {
    let mut changed = 0usize;
    let mut renamed = HashMap::new();
    for block in &mut nif.blocks {
        if block.type_name != "BSValueNode" {
            continue;
        }
        let Some(name) = string_field(block, "Name") else {
            continue;
        };
        let Some((index, digits)) = parse_addon_node_index(&name) else {
            continue;
        };
        let rest = name
            .get("addonnode".len()..)
            .unwrap_or_default()
            .trim_start();
        let suffix = rest.get(digits.len()..).unwrap_or_default();
        let normalized = format!("AddOnNode{digits}{suffix}");
        if name != normalized {
            block.set_field("Name", NifValue::String(normalized.clone()));
            renamed.insert(name, normalized);
            changed += 1;
        }
        if value_i64(block.get_field("Value")) != Some(index) {
            block.set_field("Value", NifValue::Int(index));
            changed += 1;
        }
    }
    let target_names = normalize_animation_target_names(nif, &renamed);
    if changed > 0 {
        report.changes.push(format!(
            "BSValueNode: normalized {changed} addon name/value field(s) and {target_names} animation target name(s)"
        ));
    }
}

fn normalize_animation_target_names(nif: &mut NifFile, renamed: &HashMap<String, String>) -> usize {
    if renamed.is_empty() {
        return 0;
    }
    let mut changed = 0usize;
    for block in &mut nif.blocks {
        if block.type_name != "NiControllerSequence" {
            continue;
        }
        if let Some(NifValue::String(name)) = block.fields.get_mut("Accum Root Name")
            && let Some(replacement) = renamed.get(name)
        {
            *name = replacement.clone();
            changed += 1;
        }
        let Some(NifValue::Array(entries)) = block.fields.get_mut("Controlled Blocks") else {
            continue;
        };
        for entry in entries {
            let NifValue::Struct(fields) = entry else {
                continue;
            };
            if let Some(NifValue::String(name)) = fields.get_mut("Node Name")
                && let Some(replacement) = renamed.get(name)
            {
                *name = replacement.clone();
                changed += 1;
            }
        }
    }
    changed
}

pub(crate) fn parse_addon_node_index(name: &str) -> Option<(i64, &str)> {
    let prefix = "addonnode";
    if !name.get(..prefix.len())?.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = name.get(prefix.len()..)?.trim_start();
    let digit_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digit_len == 0 {
        return None;
    }
    let digits = &rest[..digit_len];
    Some((digits.parse().ok()?, digits))
}

fn normalize_animation_metadata(nif: &mut NifFile, report: &mut SanitizeReport) {
    let reachable_before = reachable_block_ids(nif);
    let names = named_av_objects(nif);
    let mut target_repairs = Vec::new();
    for block in &nif.blocks {
        if !SCHEMA.is_subtype_of(&block.type_name, "NiObjectNET") {
            continue;
        }
        let mut controller = value_ref(block.get_field("Controller")).unwrap_or(-1);
        let mut seen = HashSet::new();
        while controller >= 0 && seen.insert(controller) {
            let Some(controller_block) = nif.get_block(controller as usize) else {
                break;
            };
            if !SCHEMA.is_subtype_of(&controller_block.type_name, "NiTimeController") {
                break;
            }
            if value_ref(controller_block.get_field("Target")).unwrap_or(-1) < 0 {
                target_repairs.push((controller as usize, block.block_id));
            }
            controller = value_ref(controller_block.get_field("Next Controller")).unwrap_or(-1);
        }
    }
    for (controller, target) in &target_repairs {
        if let Some(block) = nif.blocks.get_mut(*controller) {
            block.set_field("Target", NifValue::Ref(*target as i32));
        }
    }

    let manager_ids = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiControllerManager")
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut removed = 0usize;
    let mut sorted = 0usize;
    let mut accum_roots = 0usize;
    let mut palettes = 0usize;
    let mut extra_targets = 0usize;

    for manager_id in manager_ids {
        let Some(manager) = nif.get_block(manager_id) else {
            continue;
        };
        let manager_target = value_ref(manager.get_field("Target"))
            .filter(|target| *target >= 0)
            .map(|target| target as usize);
        let manager_target_name = manager_target
            .and_then(|target| nif.get_block(target))
            .and_then(|target| string_field(target, "Name"));
        let sequence_ids = ref_values(manager.get_field("Controller Sequences"));
        let multitarget_id = value_ref(manager.get_field("Next Controller"))
            .filter(|target| *target >= 0)
            .map(|target| target as usize);
        let mut controlled_targets = BTreeSet::new();

        for sequence_id in sequence_ids {
            let sequence_id = sequence_id as usize;
            let Some(sequence) = nif.get_block(sequence_id) else {
                continue;
            };
            if sequence.type_name != "NiControllerSequence" {
                continue;
            }
            let update_accum_root = manager_target_name.as_ref().is_some_and(|target_name| {
                string_field(sequence, "Accum Root Name").as_deref() != Some(target_name)
            });
            let controlled_blocks = sequence.get_field("Controlled Blocks").cloned();
            if update_accum_root {
                nif.blocks[sequence_id].set_field(
                    "Accum Root Name",
                    NifValue::String(manager_target_name.clone().unwrap_or_default()),
                );
                accum_roots += 1;
            }
            let Some(NifValue::Array(entries)) = controlled_blocks else {
                continue;
            };
            let old_len = entries.len();
            let mut valid = entries
                .into_iter()
                .filter_map(|entry| {
                    let target = controlled_block_target(nif, &entry, &names)?;
                    controlled_targets.insert(target);
                    Some((target, entry))
                })
                .collect::<Vec<_>>();
            removed += old_len - valid.len();
            let old_order = valid.iter().map(|(target, _)| *target).collect::<Vec<_>>();
            valid.sort_by_key(|(target, _)| *target);
            if old_order != valid.iter().map(|(target, _)| *target).collect::<Vec<_>>() {
                sorted += 1;
            }
            let entries = valid
                .into_iter()
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>();
            let sequence = &mut nif.blocks[sequence_id];
            sequence.set_field(
                "Num Controlled Blocks",
                NifValue::UInt(entries.len() as u64),
            );
            sequence.set_field("Controlled Blocks", NifValue::Array(entries));
        }

        let target_ids = controlled_targets.into_iter().collect::<Vec<_>>();
        if let Some(palette_id) = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "NiDefaultAVObjectPalette")
            .map(|block| block.block_id)
        {
            let objects = target_ids
                .iter()
                .filter_map(|target| {
                    let target_block = nif.get_block(*target)?;
                    let name = string_field(target_block, "Name")?;
                    Some(NifValue::Struct(IndexMap::from([
                        ("Name".to_string(), NifValue::String(name)),
                        ("AV Object".to_string(), NifValue::Ref(*target as i32)),
                    ])))
                })
                .collect::<Vec<_>>();
            let differs = nif
                .get_block(palette_id)
                .and_then(|palette| palette.get_field("Objs"))
                != Some(&NifValue::Array(objects.clone()));
            if differs && let Some(palette) = nif.blocks.get_mut(palette_id) {
                palette.set_field("Num Objs", NifValue::UInt(objects.len() as u64));
                palette.set_field("Objs", NifValue::Array(objects));
                palettes += 1;
            }
        }

        if let Some(multitarget_id) = multitarget_id
            && nif
                .get_block(multitarget_id)
                .is_some_and(|block| block.type_name == "NiMultiTargetTransformController")
        {
            let current = nif
                .get_block(multitarget_id)
                .and_then(|block| block.get_field("Extra Targets"))
                .map(ref_scalars)
                .unwrap_or_default();
            if !sparse_extra_targets_cover_expected(&current, &target_ids) {
                let targets = target_ids
                    .iter()
                    .map(|target| NifValue::Ref(*target as i32))
                    .collect::<Vec<_>>();
                let differs = nif
                    .get_block(multitarget_id)
                    .and_then(|block| block.get_field("Extra Targets"))
                    != Some(&NifValue::Array(targets.clone()));
                if differs && let Some(multitarget) = nif.blocks.get_mut(multitarget_id) {
                    multitarget
                        .set_field("Num Extra Targets", NifValue::UInt(targets.len() as u64));
                    multitarget.set_field("Extra Targets", NifValue::Array(targets));
                    extra_targets += 1;
                }
            }
        }
    }

    let total = target_repairs.len() + removed + sorted + accum_roots + palettes + extra_targets;
    if total > 0 {
        report.changes.push(format!(
            "Animation metadata: repaired {} controller target(s), removed {removed} invalid controlled block(s), sorted {sorted} sequence(s), set {accum_roots} accumulation root(s), updated {palettes} palette(s) and {extra_targets} extra-target list(s)",
            target_repairs.len()
        ));
    }
    if removed > 0 {
        let reachable_after = reachable_block_ids(nif);
        let orphaned = reachable_before
            .difference(&reachable_after)
            .copied()
            .collect::<Vec<_>>();
        if !orphaned.is_empty() {
            nif.remove_blocks(&orphaned);
        }
    }
}

fn controlled_block_target(
    nif: &NifFile,
    entry: &NifValue,
    names: &HashMap<String, usize>,
) -> Option<usize> {
    let NifValue::Struct(fields) = entry else {
        return None;
    };
    if let Some(name) = fields
        .get("Node Name")
        .and_then(value_string)
        .filter(|name| !name.is_empty())
    {
        return names.get(name).copied();
    }
    let palette_id = value_ref(fields.get("String Palette"))?;
    let offset = value_u64(fields.get("Node Name Offset"))? as usize;
    let palette = nif.get_block(usize::try_from(palette_id).ok()?)?;
    let text = nested_value(palette.get_field("Palette"), "Palette")
        .or_else(|| palette.get_field("Palette"))
        .and_then(value_string)?;
    let bytes = text.as_bytes().get(offset..)?;
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let name = std::str::from_utf8(&bytes[..length]).ok()?;
    names.get(name).copied()
}

fn sparse_extra_targets_cover_expected(current: &[i32], expected: &[usize]) -> bool {
    current.iter().any(|target| *target < 0)
        && current
            .iter()
            .filter_map(|target| usize::try_from(*target).ok())
            .collect::<BTreeSet<_>>()
            == expected.iter().copied().collect::<BTreeSet<_>>()
}

fn named_av_objects(nif: &NifFile) -> HashMap<String, usize> {
    let mut names = HashMap::new();
    for block in &nif.blocks {
        if !SCHEMA.is_subtype_of(&block.type_name, "NiAVObject") {
            continue;
        }
        if let Some(name) = string_field(block, "Name").filter(|name| !name.is_empty()) {
            names.entry(name).or_insert(block.block_id);
        }
    }
    names
}

fn normalize_collision_targets(nif: &mut NifFile, report: &mut SanitizeReport) {
    let game = nif_game(nif);
    if matches!(game, NifGame::Unknown) {
        return;
    }
    let parents = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiAVObject"))
        .filter_map(|parent| {
            value_ref(parent.get_field("Collision Object"))
                .filter(|collision| *collision >= 0)
                .map(|collision| (collision as usize, parent.block_id))
        })
        .collect::<Vec<_>>();
    let mut targets = 0usize;
    let mut flags = 0usize;

    for (collision_id, parent_id) in parents {
        let body_id = nif
            .get_block(collision_id)
            .and_then(|collision| value_ref(collision.get_field("Body")))
            .filter(|body| *body >= 0)
            .map(|body| body as usize);
        let layer = body_id
            .and_then(|body_id| nif.get_block(body_id))
            .and_then(|body| rigid_body_nested_u64(body, "Havok Filter", "Layer"));
        if let Some(collision) = nif.blocks.get_mut(collision_id) {
            if collision.get_field("Target").is_some()
                && value_ref(collision.get_field("Target")) != Some(parent_id as i32)
            {
                collision.set_field("Target", NifValue::Ref(parent_id as i32));
                targets += 1;
            }

            if body_id.is_some() {
                if let Some(old_flags) = value_u64(collision.get_field("Flags")) {
                    let mut new_flags = old_flags;
                    if matches!(game, NifGame::Skyrim | NifGame::SkyrimSe | NifGame::Fo4) {
                        new_flags |= 1 << 7;
                    }
                    if layer == Some(2) {
                        new_flags |= 1 << 3;
                    } else {
                        new_flags &= !(1 << 3);
                    }
                    if new_flags != old_flags {
                        collision.set_field("Flags", NifValue::UInt(new_flags));
                        flags += 1;
                    }
                }
            }
        }

        let compressed_id = body_id
            .and_then(|body| nif.get_block(body))
            .and_then(|body| value_ref(body.get_field("Shape")))
            .filter(|shape| *shape >= 0)
            .map(|shape| shape as usize)
            .and_then(|shape_id| {
                let shape = nif.get_block(shape_id)?;
                if SCHEMA.is_subtype_of(&shape.type_name, "bhkMoppBvTreeShape") {
                    value_ref(shape.get_field("Shape"))
                        .filter(|inner| *inner >= 0)
                        .map(|inner| inner as usize)
                } else {
                    None
                }
            })
            .filter(|shape_id| {
                nif.get_block(*shape_id)
                    .is_some_and(|shape| shape.type_name == "bhkCompressedMeshShape")
            });
        if let Some(compressed_id) = compressed_id
            && let Some(compressed) = nif.blocks.get_mut(compressed_id)
            && compressed.get_field("Target").is_some()
            && value_ref(compressed.get_field("Target")) != Some(parent_id as i32)
        {
            compressed.set_field("Target", NifValue::Ref(parent_id as i32));
            targets += 1;
        }
    }

    if targets + flags > 0 {
        report.changes.push(format!(
            "Collision: repaired {targets} target field(s) and {flags} collision flag field(s)"
        ));
    }
}

fn normalize_collision_settings(nif: &mut NifFile, report: &mut SanitizeReport) {
    let game = nif_game(nif);
    if !matches!(game, NifGame::Fo3 | NifGame::Skyrim | NifGame::SkyrimSe) {
        return;
    }
    let body_ids = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "bhkRigidBody"))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut changed = 0usize;
    let mut list_shapes = Vec::new();

    for body_id in body_ids {
        let Some(body) = nif.get_block(body_id) else {
            continue;
        };
        let Some(layer) = rigid_body_nested_u64(body, "Havok Filter", "Layer") else {
            continue;
        };
        let motion_system = value_u64(rigid_body_value(body, "Motion System")).unwrap_or(0);
        let static_layer = matches!(layer, 1 | 9 | 15);
        let animstatic_layer =
            matches!(game, NifGame::Skyrim | NifGame::SkyrimSe) && layer == 2 && motion_system != 6;
        let moveable_layer = matches!(layer, 4 | 5 | 10);
        if let Some(shape_id) = value_ref(body.get_field("Shape"))
            .filter(|shape| *shape >= 0)
            .map(|shape| shape as usize)
            .filter(|shape| {
                nif.get_block(*shape)
                    .is_some_and(|shape| shape.type_name == "bhkListShape")
            })
        {
            list_shapes.push((shape_id, layer));
        }

        let Some(body) = nif.blocks.get_mut(body_id) else {
            continue;
        };
        changed += usize::from(set_rigid_body_nested_u64_if_present(
            body,
            "Havok Filter Copy",
            "Layer",
            layer,
        ));

        if static_layer || animstatic_layer {
            let desired_motion = if static_layer { 7 } else { 4 };
            changed += usize::from(set_rigid_body_u64_if_present(
                body,
                "Motion System",
                desired_motion,
            ));
            changed += usize::from(set_rigid_body_u64_if_present(body, "Motion Quality", 1));
            changed += usize::from(set_rigid_body_u64_if_present(body, "Deactivator Type", 0));
            changed += usize::from(set_rigid_body_bool_if_present(
                body,
                "Enable Deactivation",
                false,
            ));
            changed += usize::from(set_rigid_body_u64_if_present(
                body,
                "Solver Deactivation",
                0,
            ));
            changed += usize::from(set_rigid_body_f64_if_present(body, "Mass", 0.0));
            changed += usize::from(set_inertia_diagonal_if_present(body, 0.0));
        } else if moveable_layer {
            let motion = value_u64(rigid_body_value(body, "Motion System")).unwrap_or(0);
            let desired_motion = if matches!(game, NifGame::Skyrim | NifGame::SkyrimSe) {
                if matches!(motion, 2 | 3) { motion } else { 3 }
            } else if matches!(motion, 2 | 4) {
                motion
            } else {
                4
            };
            changed += usize::from(set_rigid_body_u64_if_present(
                body,
                "Motion System",
                desired_motion,
            ));
            let quality = value_u64(rigid_body_value(body, "Motion Quality")).unwrap_or(0);
            let desired_quality = if matches!(game, NifGame::Skyrim | NifGame::SkyrimSe) {
                4
            } else if matches!(quality, 3 | 4) {
                quality
            } else {
                3
            };
            changed += usize::from(set_rigid_body_u64_if_present(
                body,
                "Motion Quality",
                desired_quality,
            ));
            changed += usize::from(set_rigid_body_u64_if_present(body, "Deactivator Type", 1));
            changed += usize::from(set_rigid_body_bool_if_present(
                body,
                "Enable Deactivation",
                true,
            ));
            let solver = value_u64(rigid_body_value(body, "Solver Deactivation")).unwrap_or(0);
            if !matches!(solver, 1..=4) {
                changed += usize::from(set_rigid_body_u64_if_present(
                    body,
                    "Solver Deactivation",
                    1,
                ));
            }
            if value_f64(rigid_body_value(body, "Mass")) == Some(0.0) {
                changed += usize::from(set_rigid_body_f64_if_present(body, "Mass", 1.0));
            }
            if inertia_tensor_is_bad(rigid_body_value(body, "Inertia Tensor")) {
                changed += usize::from(set_inertia_diagonal_if_present(body, 1.0));
            }
        }

        if value_f64(rigid_body_value(body, "Time Factor")) == Some(0.0) {
            changed += usize::from(set_rigid_body_f64_if_present(body, "Time Factor", 1.0));
        }
        if value_f64(rigid_body_value(body, "Gravity Factor")) == Some(0.0) {
            changed += usize::from(set_rigid_body_f64_if_present(body, "Gravity Factor", 1.0));
        }
    }

    for (shape_id, layer) in list_shapes {
        let Some(shape) = nif.blocks.get_mut(shape_id) else {
            continue;
        };
        let subshape_count = ref_values(shape.get_field("Sub Shapes")).len();
        let Some(NifValue::Array(mut filters)) = shape.get_field("Filters").cloned() else {
            continue;
        };
        let old_len = filters.len();
        filters.resize_with(subshape_count, || NifValue::Struct(IndexMap::new()));
        filters.truncate(subshape_count);
        changed += usize::from(old_len != filters.len());
        for filter in &mut filters {
            if nested_u64(Some(filter), "Layer").unwrap_or(0) == 0 {
                changed += usize::from(set_struct_field(filter, "Layer", NifValue::UInt(layer)));
            }
        }
        shape.set_field("Filters", NifValue::Array(filters));
    }

    if changed > 0 {
        report.changes.push(format!(
            "Collision: normalized {changed} rigid-body setting field(s)"
        ));
    }
}

fn normalize_particle_data(nif: &mut NifFile, report: &mut SanitizeReport) {
    if nif_game(nif) != NifGame::SkyrimSe {
        return;
    }
    let mesh_ids = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiPSysMeshEmitter"))
        .flat_map(|block| ref_values(block.get_field("Emitter Meshes")))
        .filter(|mesh| *mesh >= 0)
        .map(|mesh| mesh as usize)
        .collect::<BTreeSet<_>>();
    let mut changed = 0usize;
    for mesh_id in mesh_ids {
        let Some(mesh) = nif.blocks.get_mut(mesh_id) else {
            continue;
        };
        if mesh.type_name != "BSTriShape"
            || value_u64(mesh.get_field("Particle Data Size")).unwrap_or(0) != 0
        {
            continue;
        }
        let vertices = value_array(mesh.get_field("Vertex Data"));
        if vertices.is_empty() {
            continue;
        }
        let particle_vertices = vertices
            .iter()
            .filter_map(|vertex| nested_value(Some(vertex), "Vertex").cloned())
            .collect::<Vec<_>>();
        let particle_normals = vertices
            .iter()
            .filter_map(|vertex| nested_value(Some(vertex), "Normal").cloned())
            .collect::<Vec<_>>();
        if particle_vertices.len() != vertices.len() || particle_normals.len() != vertices.len() {
            continue;
        }
        let triangles = mesh
            .get_field("Triangles")
            .cloned()
            .unwrap_or_else(|| NifValue::Array(Vec::new()));
        mesh.set_field("Particle Data Size", NifValue::UInt(1));
        mesh.set_field("Particle Vertices", NifValue::Array(particle_vertices));
        mesh.set_field("Particle Normals", NifValue::Array(particle_normals));
        mesh.set_field("Particle Triangles", triangles);
        changed += 1;
    }
    if changed > 0 {
        report.changes.push(format!(
            "Particles: added missing SSE mesh-emitter data to {changed} shape(s)"
        ));
    }
}

fn normalize_consistency_flags(nif: &mut NifFile, report: &mut SanitizeReport) {
    if !matches!(
        nif_game(nif),
        NifGame::Oblivion | NifGame::Fo3 | NifGame::Skyrim
    ) {
        return;
    }
    let updates = nif
        .blocks
        .iter()
        .filter(|shape| SCHEMA.is_subtype_of(&shape.type_name, "NiGeometry"))
        .filter_map(|shape| {
            let data_id = value_ref(shape.get_field("Data")).filter(|id| *id >= 0)? as usize;
            let data = nif.get_block(data_id)?;
            data.get_field("Consistency Flags")?;
            let controller = value_ref(shape.get_field("Controller"))
                .filter(|id| *id >= 0)
                .and_then(|id| nif.get_block(id as usize));
            let mutable = SCHEMA.is_subtype_of(&shape.type_name, "NiParticles")
                || controller.is_some_and(|controller| {
                    SCHEMA.is_subtype_of(&controller.type_name, "NiGeomMorpherController")
                        || SCHEMA.is_subtype_of(&controller.type_name, "NiUVController")
                });
            Some((data_id, if mutable { 0 } else { 0x4000 }))
        })
        .collect::<Vec<_>>();
    let mut changed = 0usize;
    for (data_id, desired) in updates {
        let Some(data) = nif.blocks.get_mut(data_id) else {
            continue;
        };
        if value_u64(data.get_field("Consistency Flags")) != Some(desired) {
            data.set_field("Consistency Flags", NifValue::UInt(desired));
            changed += 1;
        }
    }
    if changed > 0 {
        report.changes.push(format!(
            "Geometry: normalized {changed} consistency flag field(s)"
        ));
    }
}

fn normalize_shader_flags(nif: &mut NifFile, report: &mut SanitizeReport) {
    let game = nif_game(nif);
    if matches!(game, NifGame::Unknown) {
        return;
    }
    let usage = shader_usage(nif);
    let facegen = nif.blocks.iter().any(|block| {
        SCHEMA.is_subtype_of(&block.type_name, "NiNode")
            && string_field(block, "Name").as_deref() == Some("BSFaceGenNiNodeSkinned")
    });
    let root_type = root_ids(nif)
        .into_iter()
        .next()
        .and_then(|root| nif.get_block(root))
        .map(|root| root.type_name.clone())
        .unwrap_or_default();
    let mut vertex_added = 0usize;
    let mut vertex_removed = 0usize;
    let mut skin_added = 0usize;
    let mut skin_removed = 0usize;
    let mut game_rules = 0usize;

    for (shader_id, usage) in usage {
        let texture_features = skyrim_shader_texture_features(nif, shader_id);
        let Some(shader) = nif.blocks.get_mut(shader_id) else {
            continue;
        };
        if !SCHEMA.is_subtype_of(&shader.type_name, "BSShaderProperty") {
            continue;
        }
        let mut flags1 = shader_flags1(shader);
        let mut flags2 = value_u64(shader.get_field("Shader Flags 2")).unwrap_or(0);
        let old_flags1 = flags1;
        let old_flags2 = flags2;

        if game.uses_runtime_vertex_color_flags() {
            if usage.with_vertex_colors > 0 {
                flags2 |= SLSF2_VERTEX_COLORS;
            } else if usage.total > 0 {
                flags2 &= !SLSF2_VERTEX_COLORS;
            }
        }
        if usage.with_vertex_colors == 0 && usage.total > 0 {
            flags1 &= !SLSF1_VERTEX_ALPHA;
        }
        if usage.skinned > 0 {
            flags1 |= SLSF1_SKINNED;
        } else if usage.total > 0 {
            flags1 &= !SLSF1_SKINNED;
        }

        if old_flags2 & SLSF2_VERTEX_COLORS == 0 && flags2 & SLSF2_VERTEX_COLORS != 0 {
            vertex_added += 1;
        }
        if old_flags2 & SLSF2_VERTEX_COLORS != 0 && flags2 & SLSF2_VERTEX_COLORS == 0 {
            vertex_removed += 1;
        }
        if old_flags1 & SLSF1_SKINNED == 0 && flags1 & SLSF1_SKINNED != 0 {
            skin_added += 1;
        }
        if old_flags1 & SLSF1_SKINNED != 0 && flags1 & SLSF1_SKINNED == 0 {
            skin_removed += 1;
        }
        if game.uses_skyrim_shader_rules() {
            game_rules += usize::from(normalize_skyrim_shader_rules(
                shader,
                &mut flags1,
                &mut flags2,
                facegen,
                &root_type,
                usage.with_vertex_colors > 0,
                texture_features,
            ));
        }
        if flags1 != old_flags1 {
            set_shader_flags1(shader, flags1);
        }
        if flags2 != old_flags2 {
            shader.set_field("Shader Flags 2", NifValue::UInt(flags2));
        }
        if usage.with_vertex_colors > 0 && usage.with_vertex_colors < usage.total {
            report.warnings.push(format!(
                "Shader block {shader_id} is shared by geometry with and without vertex colors; Vertex_Colors was enabled"
            ));
        }
    }

    if vertex_added + vertex_removed + skin_added + skin_removed + game_rules > 0 {
        report.changes.push(format!(
            "Shader flags: Vertex_Colors added={vertex_added} removed={vertex_removed}; Skinned added={skin_added} removed={skin_removed}; game rules={game_rules}"
        ));
    }
}

fn normalize_skyrim_shader_rules(
    shader: &mut NifBlock,
    flags1: &mut u64,
    flags2: &mut u64,
    facegen: bool,
    root_type: &str,
    has_vertex_colors: bool,
    textures: SkyrimShaderTextureFeatures,
) -> bool {
    let old_flags1 = *flags1;
    let old_flags2 = *flags2;
    let old_glossiness = value_f64(shader.get_field("Glossiness"));
    if *flags1 & SLSF1_DYNAMIC_DECAL != 0 {
        *flags1 |= SLSF1_DECAL;
        *flags2 |= SLSF2_ASSUME_SHADOWMASK;
    }
    if shader.type_name != "BSLightingShaderProperty" {
        return *flags1 != old_flags1 || *flags2 != old_flags2;
    }
    let mut shader_type = value_u64(shader.get_field("Shader Type")).unwrap_or(0);
    if let Some(inferred) = inferred_skyrim_shader_type(shader_type, *flags1, *flags2, textures)
        && inferred != shader_type
    {
        shader.set_field("Shader Type", NifValue::UInt(inferred));
        shader_type = inferred;
    }
    if shader_type == SHADER_ENVIRONMENT_MAP {
        *flags1 |= SLSF1_ENVIRONMENT_MAPPING;
    } else {
        *flags1 &= !SLSF1_ENVIRONMENT_MAPPING;
    }
    if shader_type == SHADER_GLOW {
        *flags1 |= SLSF1_OWN_EMIT;
        *flags2 |= SLSF2_GLOW_MAP;
    } else {
        *flags2 &= !SLSF2_GLOW_MAP;
        if color_is_black(shader.get_field("Emissive Color")) {
            *flags1 &= !SLSF1_OWN_EMIT;
        }
        *flags1 &= !SLSF1_EXTERNAL_EMITTANCE;
    }
    if shader_type == SHADER_PARALLAX {
        *flags1 |= SLSF1_PARALLAX;
    } else {
        *flags1 &= !SLSF1_PARALLAX;
    }
    if shader_type == SHADER_FACEGEN {
        *flags1 |= SLSF1_FACEGEN;
        *flags2 |= SLSF2_SOFT_LIGHTING;
        *flags2 &= !SLSF2_ANISOTROPIC_LIGHTING;
    } else {
        *flags1 &= !SLSF1_FACEGEN;
    }
    if shader_type == SHADER_SKIN_TINT {
        *flags1 |= SLSF1_SKIN_TINT;
        *flags2 |= SLSF2_SOFT_LIGHTING;
    } else {
        *flags1 &= !SLSF1_SKIN_TINT;
    }
    if shader_type == SHADER_HAIR_TINT {
        *flags1 |= SLSF1_HAIR_TINT;
    } else {
        *flags1 &= !SLSF1_HAIR_TINT;
    }
    if shader_type == SHADER_MULTI_LAYER_PARALLAX {
        *flags2 |= SLSF2_MULTI_LAYER_PARALLAX;
    } else {
        *flags2 &= !SLSF2_MULTI_LAYER_PARALLAX;
    }
    if shader_type == SHADER_EYE_ENVMAP {
        *flags1 |= SLSF1_EYE_ENVIRONMENT_MAPPING;
    } else {
        *flags1 &= !SLSF1_EYE_ENVIRONMENT_MAPPING;
    }
    if facegen {
        *flags2 |= SLSF2_CHARACTER_LIGHTING;
    } else {
        *flags2 &= !SLSF2_CHARACTER_LIGHTING;
    }
    if matches!(
        shader_type,
        SHADER_ENVIRONMENT_MAP | SHADER_MULTI_LAYER_PARALLAX | SHADER_EYE_ENVMAP
    ) {
        *flags2 |= SLSF2_ENV_MAP_LIGHT_FADE;
    } else {
        *flags2 &= !SLSF2_ENV_MAP_LIGHT_FADE;
    }
    if *flags1 & SLSF1_SPECULAR != 0 && color_is_black(shader.get_field("Specular Color")) {
        *flags1 &= !SLSF1_SPECULAR;
    }
    if *flags2 & SLSF2_TREE_ANIM != 0 {
        if !matches!(root_type, "BSLeafAnimNode" | "BSTreeNode")
            || !matches!(shader_type, 0 | SHADER_TREE_ANIM)
            || !has_vertex_colors
        {
            *flags2 &= !SLSF2_TREE_ANIM;
            *flags1 &= !SLSF1_VERTEX_ALPHA;
        } else {
            *flags1 |= SLSF1_VERTEX_ALPHA;
        }
    }
    if value_f64(shader.get_field("Glossiness")) == Some(0.0) {
        shader.set_field("Glossiness", NifValue::Float(1.0));
    }
    *flags1 != old_flags1
        || *flags2 != old_flags2
        || old_glossiness != value_f64(shader.get_field("Glossiness"))
}

#[derive(Debug, Clone, Copy, Default)]
struct SkyrimShaderTextureFeatures {
    emissive: bool,
    parallax: bool,
    environment: bool,
    subsurface: bool,
}

fn skyrim_shader_texture_features(nif: &NifFile, shader_id: usize) -> SkyrimShaderTextureFeatures {
    let Some(shader) = nif.get_block(shader_id) else {
        return SkyrimShaderTextureFeatures::default();
    };
    let Some(texture_set) = value_ref(shader.get_field("Texture Set"))
        .filter(|reference| *reference >= 0)
        .and_then(|reference| nif.get_block(reference as usize))
    else {
        return SkyrimShaderTextureFeatures::default();
    };
    let textures = value_array(texture_set.get_field("Textures"));
    let populated = |index: usize| {
        textures
            .get(index)
            .and_then(value_string)
            .is_some_and(|texture| !texture.is_empty())
    };
    SkyrimShaderTextureFeatures {
        emissive: populated(2),
        parallax: populated(3),
        environment: populated(4) || populated(5),
        subsurface: populated(6),
    }
}

fn inferred_skyrim_shader_type(
    current: u64,
    flags1: u64,
    flags2: u64,
    textures: SkyrimShaderTextureFeatures,
) -> Option<u64> {
    let facegen_textures = textures.emissive && textures.parallax && textures.subsurface;
    let multilayer_textures = textures.environment && textures.subsurface;
    let current_has_textures = match current {
        SHADER_FACEGEN => facegen_textures,
        SHADER_MULTI_LAYER_PARALLAX => multilayer_textures,
        SHADER_ENVIRONMENT_MAP | SHADER_EYE_ENVMAP => textures.environment,
        SHADER_GLOW | SHADER_SKIN_TINT => textures.emissive,
        SHADER_PARALLAX => textures.parallax,
        _ => false,
    };
    let mut inferred = current_has_textures.then_some(current);

    inferred = if facegen_textures && flags1 & SLSF1_FACEGEN != 0 {
        Some(SHADER_FACEGEN)
    } else if multilayer_textures && flags2 & SLSF2_MULTI_LAYER_PARALLAX != 0 {
        Some(SHADER_MULTI_LAYER_PARALLAX)
    } else if textures.environment && flags1 & SLSF1_ENVIRONMENT_MAPPING != 0 {
        Some(SHADER_ENVIRONMENT_MAP)
    } else if textures.environment && flags1 & SLSF1_EYE_ENVIRONMENT_MAPPING != 0 {
        Some(SHADER_EYE_ENVMAP)
    } else if textures.emissive && flags2 & SLSF2_GLOW_MAP != 0 {
        Some(SHADER_GLOW)
    } else if textures.emissive && flags1 & SLSF1_SKIN_TINT != 0 {
        Some(SHADER_SKIN_TINT)
    } else if textures.parallax && flags1 & SLSF1_PARALLAX != 0 {
        Some(SHADER_PARALLAX)
    } else {
        inferred
    };
    inferred
}

#[derive(Debug, Clone, Copy, Default)]
struct ShaderUsage {
    total: usize,
    with_vertex_colors: usize,
    skinned: usize,
}

fn shader_usage(nif: &NifFile) -> HashMap<usize, ShaderUsage> {
    let mut usage = HashMap::new();
    for block in &nif.blocks {
        if !SCHEMA.is_subtype_of(&block.type_name, "NiGeometry")
            && !SCHEMA.is_subtype_of(&block.type_name, "BSTriShape")
            && block.get_field("Vertex Desc").is_none()
        {
            continue;
        }
        let Some(shader_id) = geometry_shader_id(nif, block) else {
            continue;
        };
        let entry = usage.entry(shader_id).or_insert_with(ShaderUsage::default);
        entry.total += 1;
        if shape_has_vertex_colors(nif, block) {
            entry.with_vertex_colors += 1;
        }
        if skin_ref(block).is_some() {
            entry.skinned += 1;
        }
    }
    usage
}

fn geometry_shader_id(nif: &NifFile, block: &NifBlock) -> Option<usize> {
    if let Some(shader) =
        value_ref(block.get_field("Shader Property")).filter(|shader| *shader >= 0)
    {
        return Some(shader as usize);
    }
    ref_values(block.get_field("Properties"))
        .into_iter()
        .map(|property| property as usize)
        .find(|property| {
            nif.get_block(*property).is_some_and(|property| {
                SCHEMA.is_subtype_of(&property.type_name, "BSShaderProperty")
            })
        })
}

fn shape_has_vertex_colors(nif: &NifFile, block: &NifBlock) -> bool {
    if value_u64(block.get_field("Vertex Desc"))
        .is_some_and(|descriptor| ((descriptor >> 44) & VF_VERTEX_COLORS) != 0)
    {
        return true;
    }
    if value_array(block.get_field("Vertex Data")).iter().any(
        |value| matches!(value, NifValue::Struct(fields) if fields.contains_key("Vertex Colors")),
    ) {
        return true;
    }
    value_ref(block.get_field("Data"))
        .filter(|data| *data >= 0)
        .and_then(|data| nif.get_block(data as usize))
        .is_some_and(|data| value_bool(data.get_field("Has Vertex Colors")).unwrap_or(false))
}

fn normalize_bsx_flags(nif: &mut NifFile, report: &mut SanitizeReport) {
    if !nif_game(nif).uses_bsx() {
        return;
    }
    let existing = nif
        .blocks
        .iter()
        .find(|block| block.type_name == "BSXFlags")
        .map(|block| {
            (
                block.block_id,
                value_u64(block.get_field("Integer Data")).unwrap_or(0),
            )
        });
    let old_flags = existing.map(|(_, flags)| flags).unwrap_or(0);
    let desired = detected_bsx_flags(nif, old_flags);

    match existing {
        Some((block_id, _)) if desired == 0 => {
            nif.remove_blocks(&[block_id]);
            report
                .changes
                .push("BSXFlags: removed empty block".to_string());
        }
        Some((block_id, flags)) => {
            let Some(block) = nif.blocks.get_mut(block_id) else {
                return;
            };
            let mut changed = false;
            if string_field(block, "Name").as_deref() != Some("BSX") {
                block.set_field("Name", NifValue::String("BSX".to_string()));
                changed = true;
            }
            if flags != desired {
                block.set_field("Integer Data", NifValue::UInt(desired));
                changed = true;
            }
            if changed {
                report.changes.push(format!(
                    "BSXFlags: normalized name and flags {flags} -> {desired}"
                ));
            }
        }
        None if desired != 0 => {
            let root_id = root_ids(nif).into_iter().next().unwrap_or(0);
            let block_id = nif.add_block(
                "BSXFlags",
                Some(IndexMap::from([
                    ("Name".to_string(), NifValue::String("BSX".to_string())),
                    ("Integer Data".to_string(), NifValue::UInt(desired)),
                ])),
            );
            let mut extras = nif
                .get_block(root_id)
                .map(|root| ref_values(root.get_field("Extra Data List")))
                .unwrap_or_default();
            extras.push(block_id as i32);
            if let Some(root) = nif.blocks.get_mut(root_id) {
                root.set_field("Num Extra Data List", NifValue::UInt(extras.len() as u64));
                root.set_field(
                    "Extra Data List",
                    NifValue::Array(extras.into_iter().map(NifValue::Ref).collect()),
                );
            }
            report.changes.push(format!(
                "BSXFlags: added missing block with flags {desired}"
            ));
        }
        _ => {}
    }
}

fn detected_bsx_flags(nif: &NifFile, old_flags: u64) -> u64 {
    let game = nif_game(nif);
    let mut collisions = 0usize;
    let mut constraints = 0usize;
    let mut controllers = 0usize;
    let mut bounds = 0usize;
    let mut addons = 0usize;
    let mut markers = 0usize;
    let mut emitters = 0usize;
    let mut dynamic_bodies = 0usize;

    for block in &nif.blocks {
        if SCHEMA.is_subtype_of(&block.type_name, "NiCollisionObject") {
            collisions += 1;
        } else if SCHEMA.is_subtype_of(&block.type_name, "bhkConstraint")
            || SCHEMA.is_subtype_of(&block.type_name, "bhkBallSocketConstraintChain")
        {
            constraints += 1;
        } else if SCHEMA.is_subtype_of(&block.type_name, "NiTimeController") {
            controllers += 1;
        } else if SCHEMA.is_subtype_of(&block.type_name, "BSBound") {
            bounds += 1;
        } else if block.type_name == "BSValueNode" {
            addons += 1;
        } else if matches!(game, NifGame::Fo3)
            && SCHEMA.is_subtype_of(&block.type_name, "NiNode")
            && string_field(block, "Name").is_some_and(|name| {
                name.starts_with("FlameNode") || name.starts_with("AttachLight")
            })
        {
            addons += 1;
        }
        if string_field(block, "Name")
            .is_some_and(|name| name.to_ascii_lowercase().contains("editormarker"))
        {
            markers += 1;
        }
        if SCHEMA.is_subtype_of(&block.type_name, "BSShaderProperty")
            && shader_flags1(block) & SLSF1_EXTERNAL_EMITTANCE != 0
        {
            emitters += 1;
        }
        if is_dynamic_rigid_body(nif, block) {
            dynamic_bodies += 1;
        }
    }

    let mut flags = 0u64;
    if (controllers > 0 || addons > 0) && bounds == 0 {
        flags |= BSX_ANIMATED;
    }
    if collisions > 0 {
        flags |= BSX_HAVOK;
    }
    if constraints > 0 {
        flags |= BSX_RAGDOLL;
    }
    if addons > 0 {
        flags |= BSX_ADDON;
    }
    if markers > 0 {
        flags |= BSX_EDITOR_MARKER;
    }
    if dynamic_bodies > 1 && bounds == 0 {
        flags |= BSX_COMPLEX;
    }
    if dynamic_bodies > 0 {
        flags |= BSX_DYNAMIC;
    }
    if dynamic_bodies > usize::from(matches!(game, NifGame::Fo3)) {
        flags |= BSX_ARTICULATED;
    }
    if emitters > 0 {
        flags |= BSX_EXTERNAL_EMIT;
    }
    if game.is_fo4_family() {
        flags |= old_flags & (BSX_COMPLEX | BSX_DYNAMIC);
    }
    flags | (old_flags & BSX_ARTICULATED)
}

fn is_dynamic_rigid_body(nif: &NifFile, block: &NifBlock) -> bool {
    if !SCHEMA.is_subtype_of(&block.type_name, "bhkRigidBody") {
        return false;
    }
    let motion_system = value_u64(rigid_body_value(block, "Motion System")).unwrap_or(0);
    let layer = rigid_body_nested_u64(block, "Havok Filter", "Layer").unwrap_or(0);
    if matches!(motion_system, 0 | 7) || (motion_system == 6 && layer != 8) {
        return false;
    }
    if matches!(nif_game(nif), NifGame::Skyrim | NifGame::SkyrimSe) || nif_game(nif).is_fo4_family()
    {
        let quality = value_u64(rigid_body_value(block, "Motion Quality")).unwrap_or(0);
        return layer > 2 && !matches!(quality, 0 | 1);
    }
    true
}

fn audit_ref_fields(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    for block in &nif.blocks {
        let field_defs = SCHEMA.get_all_fields(&block.type_name);
        for (key, value) in &block.fields {
            let bare = bare_name(key);
            let Some(field_def) = field_defs.iter().find(|field| field.name == bare) else {
                continue;
            };
            audit_value_refs(
                nif,
                block,
                key,
                value,
                field_def,
                findings,
                field_def.length.is_some() || field_def.width.is_some(),
            );
        }
    }
}

fn audit_value_refs(
    nif: &NifFile,
    owner: &NifBlock,
    path: &str,
    value: &NifValue,
    field_def: &FieldDef,
    findings: &mut Vec<ValidationFinding>,
    array_field: bool,
) {
    if is_direct_ref_field(field_def) {
        let references = ref_scalars(value);
        let mut seen = HashSet::new();
        for (index, reference) in references.into_iter().enumerate() {
            let field_path = if array_field {
                format!("{path}[{index}]")
            } else {
                path.to_string()
            };
            if reference < 0 {
                if array_field && !allows_sparse_null_links(owner, path) {
                    findings.push(finding(
                        "error",
                        "invalid-array-link",
                        owner,
                        Some(field_path),
                        "Array contains a null link".to_string(),
                    ));
                }
                continue;
            }
            let Some(target) = nif.get_block(reference as usize) else {
                findings.push(finding(
                    "error",
                    "broken-link",
                    owner,
                    Some(field_path),
                    format!("Link {reference} is outside the block table"),
                ));
                continue;
            };
            if array_field
                && !seen.insert(reference)
                && !allows_repeated_eye_center_link(
                    &owner.type_name,
                    path,
                    target.type_name == "BSEyeCenterExtraData",
                )
            {
                findings.push(finding(
                    "error",
                    "repeated-array-link",
                    owner,
                    Some(field_path.clone()),
                    format!("Link {reference} is repeated"),
                ));
            }
            if let Some(expected) = field_def.template
                && !SCHEMA.is_subtype_of(&target.type_name, expected)
            {
                findings.push(finding(
                    "error",
                    "wrong-link-type",
                    owner,
                    Some(field_path),
                    format!(
                        "Links to {} {}, expected {expected}",
                        target.block_id, target.type_name
                    ),
                ));
            }
        }
        return;
    }

    let Some(struct_def) = SCHEMA.get_struct(field_def.type_name) else {
        return;
    };
    let values = match value {
        NifValue::Struct(_) => std::slice::from_ref(value),
        NifValue::Array(values) => values.as_slice(),
        _ => return,
    };
    for (index, value) in values.iter().enumerate() {
        let NifValue::Struct(fields) = value else {
            continue;
        };
        for (key, nested) in fields {
            let bare = bare_name(key);
            let Some(nested_def) = struct_def.fields.iter().find(|field| field.name == bare) else {
                continue;
            };
            audit_value_refs(
                nif,
                owner,
                &format!("{path}[{index}].{key}"),
                nested,
                nested_def,
                findings,
                nested_def.length.is_some() || nested_def.width.is_some(),
            );
        }
    }
}

fn allows_sparse_null_links(owner: &NifBlock, path: &str) -> bool {
    owner.type_name == "NiMultiTargetTransformController" && bare_name(path) == "Extra Targets"
}

fn allows_repeated_eye_center_link(
    owner_type: &str,
    path: &str,
    target_is_eye_center_extra_data: bool,
) -> bool {
    owner_type == "BSSubIndexTriShape"
        && bare_name(path) == "Extra Data List"
        && target_is_eye_center_extra_data
}

fn audit_block_order(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    if nif_game(nif).is_fo4_family() || matches!(nif_game(nif), NifGame::Unknown) {
        return;
    }
    for collision in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "bhkCollisionObject"))
    {
        let mut touched = HashSet::new();
        audit_owned_block_order(nif, collision.block_id, &mut touched, findings);
    }
}

fn audit_owned_block_order(
    nif: &NifFile,
    parent_id: usize,
    touched: &mut HashSet<usize>,
    findings: &mut Vec<ValidationFinding>,
) {
    if !touched.insert(parent_id) {
        return;
    }
    let Some(parent) = nif.get_block(parent_id) else {
        return;
    };
    for child_id in owned_block_refs(parent) {
        let Some(child) = nif.get_block(child_id) else {
            continue;
        };
        if SCHEMA.is_subtype_of(&child.type_name, "bhkConstraint")
            || SCHEMA.is_subtype_of(&child.type_name, "bhkBallSocketConstraintChain")
        {
            continue;
        }
        if SCHEMA.is_subtype_of(&child.type_name, "bhkAction") && child_id < parent_id {
            findings.push(finding(
                "error",
                "block-order",
                child,
                None,
                format!("Must have greater index than parent block {parent_id}"),
            ));
        } else if SCHEMA.is_subtype_of(&child.type_name, "bhkRefObject") && child_id > parent_id {
            findings.push(finding(
                "error",
                "block-order",
                child,
                None,
                format!("Must have lesser index than parent block {parent_id}"),
            ));
        }
        audit_owned_block_order(nif, child_id, touched, findings);
    }
}

fn owned_block_refs(block: &NifBlock) -> Vec<usize> {
    let field_defs = SCHEMA.get_all_fields(&block.type_name);
    let mut output = Vec::new();
    for (key, value) in &block.fields {
        let bare = bare_name(key);
        if let Some(field_def) = field_defs.iter().find(|field| field.name == bare) {
            collect_owned_refs(value, field_def, &mut output);
        }
    }
    output
}

fn collect_owned_refs(value: &NifValue, field_def: &FieldDef, output: &mut Vec<usize>) {
    if field_def.type_name == "Ref" {
        output.extend(
            ref_scalars(value)
                .into_iter()
                .filter(|reference| *reference >= 0)
                .map(|reference| reference as usize),
        );
        return;
    }
    if field_def.type_name == "Ptr" {
        return;
    }
    let Some(struct_def) = SCHEMA.get_struct(field_def.type_name) else {
        return;
    };
    let values = match value {
        NifValue::Struct(_) => std::slice::from_ref(value),
        NifValue::Array(values) => values,
        _ => return,
    };
    for value in values {
        let NifValue::Struct(fields) = value else {
            continue;
        };
        for (key, nested) in fields {
            if let Some(nested_def) = struct_def
                .fields
                .iter()
                .find(|field| field.name == bare_name(key))
            {
                collect_owned_refs(nested, nested_def, output);
            }
        }
    }
}

fn audit_unreachable_blocks(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    let roots = root_ids(nif);
    if roots.is_empty() {
        findings.push(ValidationFinding {
            severity: "error".to_string(),
            rule: "missing-root".to_string(),
            block_id: None,
            block_type: None,
            field: None,
            message: "NIF has no root block".to_string(),
        });
        return;
    }
    if roots.len() > 1 {
        findings.push(ValidationFinding {
            severity: "warning".to_string(),
            rule: "multiple-roots".to_string(),
            block_id: None,
            block_type: None,
            field: None,
            message: format!("NIF has {} root blocks", roots.len()),
        });
    }

    let mut reachable = HashSet::new();
    let mut stack = roots;
    while let Some(block_id) = stack.pop() {
        if !reachable.insert(block_id) {
            continue;
        }
        if let Some(block) = nif.get_block(block_id) {
            for reference in block.get_refs(&SCHEMA) {
                if reference >= 0 {
                    stack.push(reference as usize);
                }
            }
        }
    }
    for block in &nif.blocks {
        if !reachable.contains(&block.block_id) {
            findings.push(finding(
                "warning",
                "unused-block",
                block,
                None,
                "Unused block not referenced from a root".to_string(),
            ));
        }
    }
}

fn root_ids(nif: &NifFile) -> Vec<usize> {
    nif.header
        .footer_roots
        .iter()
        .copied()
        .filter(|root| *root >= 0 && (*root as usize) < nif.blocks.len())
        .map(|root| root as usize)
        .collect::<Vec<_>>()
}

fn reachable_block_ids(nif: &NifFile) -> HashSet<usize> {
    let mut reachable = HashSet::new();
    let mut stack = root_ids(nif);
    while let Some(block_id) = stack.pop() {
        if !reachable.insert(block_id) {
            continue;
        }
        if let Some(block) = nif.get_block(block_id) {
            stack.extend(
                block
                    .get_refs(&SCHEMA)
                    .into_iter()
                    .filter(|reference| *reference >= 0)
                    .map(|reference| reference as usize),
            );
        }
    }
    reachable
}

fn audit_duplicate_names(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    if !matches!(
        nif_game(nif),
        NifGame::Fo3
            | NifGame::Skyrim
            | NifGame::SkyrimSe
            | NifGame::Fo4
            | NifGame::Fo76
            | NifGame::Starfield
    ) {
        return;
    }
    let mut names: HashMap<String, usize> = HashMap::new();
    for block in &nif.blocks {
        if !SCHEMA.is_subtype_of(&block.type_name, "NiObjectNET")
            || block.type_name == "BSValueNode"
            || (nif_game(nif).is_fo4_family()
                && SCHEMA.is_subtype_of(&block.type_name, "BSShaderProperty"))
        {
            continue;
        }
        let Some(name) = string_field(block, "Name") else {
            continue;
        };
        if name.is_empty()
            || matches!(name.as_str(), "InvMarker" | "FurnitureMarker")
            || name.to_ascii_lowercase().contains("editormarker")
        {
            continue;
        }
        if let Some(first) = names.get(&name) {
            findings.push(finding(
                "warning",
                "duplicate-name",
                block,
                Some("Name".to_string()),
                format!("The same name {name:?} is also used by block {first}"),
            ));
        } else {
            names.insert(name, block.block_id);
        }
    }
}

fn audit_hardcoded_names(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    let has_bound = nif.blocks.iter().any(|block| block.type_name == "BSBound");
    for block in &nif.blocks {
        if let Some(expected) = hardcoded_block_name(&block.type_name)
            && block.get_field("Name").is_some()
            && string_field(block, "Name").as_deref() != Some(expected)
        {
            findings.push(finding(
                "error",
                "hardcoded-name",
                block,
                Some("Name".to_string()),
                format!("Block must be named {expected:?}"),
            ));
        }
        if !has_bound
            && string_field(block, "Name").as_deref() == Some("Weapon")
            && SCHEMA.is_subtype_of(&block.type_name, "NiAVObject")
        {
            findings.push(finding(
                "warning",
                "hardcoded-weapon-name",
                block,
                Some("Name".to_string()),
                "The hardcoded Weapon name is only valid in skeleton NIFs".to_string(),
            ));
        }
        if nif_game(nif) == NifGame::Oblivion
            && SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom")
            && geometry_property_by_type(nif, block, "NiTexturingProperty").is_some()
            && let Some(material) = geometry_property_by_type(nif, block, "NiMaterialProperty")
            && string_field(material, "Name").is_none_or(|name| name.is_empty())
        {
            findings.push(finding(
                "error",
                "oblivion-material-name",
                material,
                Some("Name".to_string()),
                "Rendered Oblivion geometry requires a named NiMaterialProperty".to_string(),
            ));
        }
    }
}

fn audit_value_node_names(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    for block in &nif.blocks {
        if block.type_name != "BSValueNode" {
            continue;
        }
        let Some(value) = value_i64(block.get_field("Value")) else {
            continue;
        };
        let expected = format!("AddOnNode{value}");
        if !string_field(block, "Name").is_some_and(|name| name.starts_with(&expected)) {
            findings.push(finding(
                "error",
                "addon-node-name",
                block,
                Some("Name".to_string()),
                format!("Name must start with {expected:?}"),
            ));
        }
    }
}

fn audit_animation(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    let names = named_av_objects(nif);
    for owner in &nif.blocks {
        if !SCHEMA.is_subtype_of(&owner.type_name, "NiObjectNET") {
            continue;
        }
        let mut controller_id = value_ref(owner.get_field("Controller")).unwrap_or(-1);
        let mut seen = HashSet::new();
        while controller_id >= 0 && seen.insert(controller_id) {
            let Some(controller) = nif.get_block(controller_id as usize) else {
                break;
            };
            if !SCHEMA.is_subtype_of(&controller.type_name, "NiTimeController") {
                findings.push(finding(
                    "error",
                    "controller-type",
                    owner,
                    Some("Controller".to_string()),
                    format!(
                        "Uses block {} {}, which is not a NiTimeController",
                        controller.block_id, controller.type_name
                    ),
                ));
                break;
            }
            let target_id = value_ref(controller.get_field("Target")).unwrap_or(-1);
            if target_id < 0 || nif.get_block(target_id as usize).is_none() {
                findings.push(finding(
                    "error",
                    "controller-target",
                    controller,
                    Some("Target".to_string()),
                    "Invalid Target field".to_string(),
                ));
            } else if target_id as usize != owner.block_id {
                findings.push(finding(
                    "error",
                    "controller-target",
                    controller,
                    Some("Target".to_string()),
                    format!(
                        "Used by block {} but targets block {target_id}",
                        owner.block_id
                    ),
                ));
            }
            controller_id = value_ref(controller.get_field("Next Controller")).unwrap_or(-1);
        }
    }

    for sequence in &nif.blocks {
        if sequence.type_name != "NiControllerSequence" {
            continue;
        }
        let manager_id = manager_for_sequence(nif, sequence.block_id);
        if let Some(manager_id) = manager_id
            && sequence.get_field("Manager").is_some()
            && value_ref(sequence.get_field("Manager")) != Some(manager_id as i32)
        {
            findings.push(finding(
                "error",
                "animation-manager",
                sequence,
                Some("Manager".to_string()),
                format!("Manager must link to block {manager_id}"),
            ));
        }
        if sequence.get_field("Accum Root Name").is_some() {
            let root_name = string_field(sequence, "Accum Root Name").unwrap_or_default();
            let expected = manager_id
                .and_then(|manager_id| nif.get_block(manager_id))
                .and_then(|manager| value_ref(manager.get_field("Target")))
                .filter(|target| *target >= 0)
                .and_then(|target| nif.get_block(target as usize))
                .and_then(|target| string_field(target, "Name"));
            if let Some(expected) = expected.as_deref()
                && root_name != expected
            {
                findings.push(finding(
                    "error",
                    "animation-accum-root",
                    sequence,
                    Some("Accum Root Name".to_string()),
                    format!(
                        "Accumulation root must match the controller manager target {expected:?}"
                    ),
                ));
            } else if expected.is_none() && !names.contains_key(&root_name) {
                findings.push(finding(
                    "error",
                    "animation-accum-root",
                    sequence,
                    Some("Accum Root Name".to_string()),
                    format!("Accumulation root {root_name:?} is not an existing NiAVObject"),
                ));
            }
        }
        let stop_time = value_f64(sequence.get_field("Stop Time"));
        let entries = value_array(sequence.get_field("Controlled Blocks"));
        for (index, entry) in entries.iter().enumerate() {
            let NifValue::Struct(fields) = entry else {
                continue;
            };
            let node_name = fields
                .get("Node Name")
                .and_then(value_string)
                .unwrap_or_default();
            match names.get(node_name).and_then(|id| nif.get_block(*id)) {
                None => findings.push(finding(
                    "error",
                    "animation-target",
                    sequence,
                    Some(format!("Controlled Blocks[{index}].Node Name")),
                    format!("Invalid Node Name {node_name:?}; expected an existing NiAVObject"),
                )),
                Some(target) if value_u64(target.get_field("Flags")).unwrap_or(0) & 1 != 0 => {
                    findings.push(finding(
                        "warning",
                        "hidden-animation-target",
                        sequence,
                        Some(format!("Controlled Blocks[{index}].Node Name")),
                        format!("Target node {node_name:?} is hidden"),
                    ));
                }
                _ => {}
            }
            if let Some(target) = names.get(node_name).and_then(|id| nif.get_block(*id)) {
                let property_type = fields
                    .get("Property Type")
                    .and_then(value_string)
                    .unwrap_or_default();
                if !property_type.is_empty()
                    && geometry_property_by_type(nif, target, property_type).is_none()
                {
                    findings.push(finding(
                        "error",
                        "animation-property",
                        sequence,
                        Some(format!("Controlled Blocks[{index}].Property Type")),
                        format!("Property {property_type:?} was not found on target {node_name:?}"),
                    ));
                }
            }

            let Some(stop_time) = stop_time else {
                continue;
            };
            let Some(interpolator_id) = fields
                .get("Interpolator")
                .and_then(|value| value_ref(Some(value)))
                .filter(|id| *id >= 0)
            else {
                continue;
            };
            let Some(data_id) = nif
                .get_block(interpolator_id as usize)
                .and_then(|interpolator| value_ref(interpolator.get_field("Data")))
                .filter(|id| *id >= 0)
            else {
                continue;
            };
            let Some(data) = nif.get_block(data_id as usize) else {
                continue;
            };
            let mut key_times = Vec::new();
            for (field, value) in &data.fields {
                collect_last_key_times(value, field, &mut key_times);
            }
            for (path, time) in key_times {
                if (time - stop_time).abs() > 1.0e-5 {
                    findings.push(finding(
                        "warning",
                        "animation-stop-time",
                        data,
                        Some(path),
                        format!("Last key time {time:.6} does not match Stop Time {stop_time:.6}"),
                    ));
                }
            }
        }

        let Some(stop_time) = stop_time else {
            continue;
        };
        let Some(text_keys) = value_ref(sequence.get_field("Text Keys"))
            .filter(|id| *id >= 0)
            .and_then(|id| nif.get_block(id as usize))
        else {
            continue;
        };
        let end_key = value_array(text_keys.get_field("Text Keys"))
            .iter()
            .rev()
            .find(|key| nested_value(Some(key), "Value").and_then(value_string) == Some("end"));
        if let Some(end_key) = end_key
            && nested_value(Some(end_key), "Time")
                .or_else(|| nested_value(Some(end_key), "Float"))
                .and_then(|value| value_f64(Some(value)))
                .is_some_and(|time| (time - stop_time).abs() > 1.0e-5)
        {
            let time = nested_value(Some(end_key), "Time")
                .or_else(|| nested_value(Some(end_key), "Float"))
                .and_then(|value| value_f64(Some(value)))
                .unwrap();
            findings.push(finding(
                "warning",
                "animation-stop-time",
                text_keys,
                Some("Text Keys".to_string()),
                format!("The end text-key time {time:.6} does not match Stop Time {stop_time:.6}"),
            ));
        }
    }

    audit_animation_metadata_alignment(nif, &names, findings);
}

fn audit_animation_metadata_alignment(
    nif: &NifFile,
    names: &HashMap<String, usize>,
    findings: &mut Vec<ValidationFinding>,
) {
    for manager in nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiControllerManager")
    {
        let mut controlled_targets = BTreeSet::new();
        for sequence_id in ref_values(manager.get_field("Controller Sequences")) {
            let Some(sequence) = usize::try_from(sequence_id)
                .ok()
                .and_then(|sequence_id| nif.get_block(sequence_id))
                .filter(|sequence| sequence.type_name == "NiControllerSequence")
            else {
                continue;
            };
            let target_order = value_array(sequence.get_field("Controlled Blocks"))
                .iter()
                .filter_map(|entry| controlled_block_target(nif, entry, names))
                .inspect(|target| {
                    controlled_targets.insert(*target);
                })
                .collect::<Vec<_>>();
            let mut expected_order = target_order.clone();
            expected_order.sort_unstable();
            if target_order != expected_order {
                findings.push(finding(
                    "error",
                    "animation-controlled-block-order",
                    sequence,
                    Some("Controlled Blocks".to_string()),
                    "Controlled blocks must be sorted by target block index".to_string(),
                ));
            }
        }

        let target_ids = controlled_targets.into_iter().collect::<Vec<_>>();
        if let Some(palette) = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "NiDefaultAVObjectPalette")
        {
            let expected = target_ids
                .iter()
                .filter_map(|target| {
                    let target_block = nif.get_block(*target)?;
                    let name = string_field(target_block, "Name")?;
                    Some(NifValue::Struct(IndexMap::from([
                        ("Name".to_string(), NifValue::String(name)),
                        ("AV Object".to_string(), NifValue::Ref(*target as i32)),
                    ])))
                })
                .collect::<Vec<_>>();
            if palette.get_field("Objs") != Some(&NifValue::Array(expected)) {
                findings.push(finding(
                    "error",
                    "animation-object-palette",
                    palette,
                    Some("Objs".to_string()),
                    "Object palette must contain the controlled target nodes in block order"
                        .to_string(),
                ));
            }
        }

        let Some(multitarget) = value_ref(manager.get_field("Next Controller"))
            .filter(|target| *target >= 0)
            .and_then(|target| nif.get_block(target as usize))
            .filter(|block| block.type_name == "NiMultiTargetTransformController")
        else {
            continue;
        };
        let current = multitarget
            .get_field("Extra Targets")
            .map(ref_scalars)
            .unwrap_or_default();
        if sparse_extra_targets_cover_expected(&current, &target_ids) {
            continue;
        }
        let expected = target_ids
            .iter()
            .map(|target| NifValue::Ref(*target as i32))
            .collect::<Vec<_>>();
        if multitarget.get_field("Extra Targets") != Some(&NifValue::Array(expected)) {
            findings.push(finding(
                "error",
                "animation-extra-targets",
                multitarget,
                Some("Extra Targets".to_string()),
                "Extra targets must match the controlled target nodes in block order".to_string(),
            ));
        }
    }
}

fn manager_for_sequence(nif: &NifFile, sequence_id: usize) -> Option<usize> {
    nif.blocks
        .iter()
        .filter(|block| block.type_name == "NiControllerManager")
        .find(|manager| {
            ref_values(manager.get_field("Controller Sequences")).contains(&(sequence_id as i32))
        })
        .map(|manager| manager.block_id)
}

fn collect_last_key_times(value: &NifValue, path: &str, output: &mut Vec<(String, f64)>) {
    match value {
        NifValue::Struct(fields) => {
            if let Some(NifValue::Array(keys)) = fields.get("Keys")
                && let Some(NifValue::Struct(last)) = keys.last()
                && let Some(time) = value_f64(last.get("Time"))
            {
                output.push((format!("{path}.Keys"), time));
            }
            for (field, value) in fields {
                if field != "Keys" {
                    collect_last_key_times(value, &format!("{path}.{field}"), output);
                }
            }
        }
        NifValue::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_last_key_times(value, &format!("{path}[{index}]"), output);
            }
        }
        _ => {}
    }
}

fn audit_particle_lifetimes(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    for block in &nif.blocks {
        if SCHEMA.is_subtype_of(&block.type_name, "NiPSysEmitter")
            && value_f64(block.get_field("Life Span")).is_some_and(|life_span| life_span > 12.0)
        {
            let life_span = value_f64(block.get_field("Life Span")).unwrap_or(0.0);
            findings.push(finding(
                "warning",
                "particle-lifetime",
                block,
                Some("Life Span".to_string()),
                format!("Life Span of {life_span} might negatively affect performance"),
            ));
        }
    }
}

fn audit_particle_systems(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    for modifier in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiPSysModifier"))
    {
        if string_field(modifier, "Name").is_none_or(|name| name.is_empty()) {
            findings.push(finding(
                "error",
                "particle-modifier-name",
                modifier,
                Some("Name".to_string()),
                "Particle modifier name is not set".to_string(),
            ));
        }
    }

    for controller in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiPSysModifierCtlr"))
    {
        let modifier_name = string_field(controller, "Modifier Name").unwrap_or_default();
        let valid = nif.blocks.iter().any(|block| {
            SCHEMA.is_subtype_of(&block.type_name, "NiPSysModifier")
                && string_field(block, "Name").as_deref() == Some(&modifier_name)
        });
        if !valid {
            findings.push(finding(
                "error",
                "particle-modifier-target",
                controller,
                Some("Modifier Name".to_string()),
                format!("Modifier Name {modifier_name:?} does not identify a NiPSysModifier"),
            ));
        }
    }

    for emitter_node in nif.blocks.iter().filter(|block| {
        SCHEMA.is_subtype_of(&block.type_name, "NiAVObject")
            && string_field(block, "Name")
                .is_some_and(|name| name.to_ascii_lowercase().ends_with("-emitter"))
    }) {
        let name = string_field(emitter_node, "Name").unwrap_or_default();
        let particle_name = &name[..name.len().saturating_sub(8)];
        let referenced_by_emitter = nif.blocks.iter().any(|block| {
            SCHEMA.is_subtype_of(&block.type_name, "NiPSysEmitter")
                && block
                    .get_refs(&SCHEMA)
                    .contains(&(emitter_node.block_id as i32))
        });
        let matching_system = nif.blocks.iter().any(|block| {
            SCHEMA.is_subtype_of(&block.type_name, "NiParticleSystem")
                && string_field(block, "Name").as_deref() == Some(particle_name)
        });
        if !referenced_by_emitter && !matching_system {
            findings.push(finding(
                "warning",
                "orphan-particle-emitter",
                emitter_node,
                Some("Name".to_string()),
                format!("Emitter {name:?} has no matching particle system or emitter reference"),
            ));
        }
    }

    let max_offsets = if matches!(
        nif_game(nif),
        NifGame::Morrowind | NifGame::Oblivion | NifGame::Fo3
    ) {
        16
    } else {
        256
    };
    for data in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiParticlesData"))
    {
        if value_u64(data.get_field("Num Subtexture Offsets"))
            .is_some_and(|count| count > max_offsets)
        {
            findings.push(finding(
                "error",
                "particle-subtexture-offsets",
                data,
                Some("Num Subtexture Offsets".to_string()),
                format!("Num Subtexture Offsets cannot exceed {max_offsets}"),
            ));
        }
    }

    if !matches!(
        nif_game(nif),
        NifGame::SkyrimSe | NifGame::Fo4 | NifGame::Fo76 | NifGame::Starfield
    ) {
        return;
    }
    for emitter in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiPSysMeshEmitter"))
    {
        for mesh_id in ref_values(emitter.get_field("Emitter Meshes")) {
            let Some(mesh) = nif.get_block(mesh_id as usize) else {
                continue;
            };
            if mesh.type_name != "BSTriShape" {
                continue;
            }
            let has_position_data = ref_values(mesh.get_field("Extra Data List"))
                .into_iter()
                .filter_map(|extra| nif.get_block(extra as usize))
                .any(|extra| extra.type_name == "BSPositionData");
            if value_u64(mesh.get_field("Particle Data Size")).unwrap_or(0) == 0
                && !has_position_data
            {
                findings.push(finding(
                    "error",
                    "particle-mesh-data",
                    mesh,
                    Some("Particle Data Size".to_string()),
                    format!(
                        "Missing particle data while used by emitter block {}",
                        emitter.block_id
                    ),
                ));
            }
        }
    }
}

fn audit_collision(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    let game = nif_game(nif);
    let parent_by_collision = nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiAVObject"))
        .filter_map(|parent| {
            value_ref(parent.get_field("Collision Object"))
                .filter(|collision| *collision >= 0)
                .map(|collision| (collision as usize, parent.block_id))
        })
        .collect::<HashMap<_, _>>();

    for collision in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiCollisionObject"))
    {
        let body_id = value_ref(collision.get_field("Body")).filter(|body| *body >= 0);
        if collision.get_field("Body").is_some() && body_id.is_none() {
            findings.push(finding(
                "error",
                "collision-body",
                collision,
                Some("Body".to_string()),
                "Missing collision body".to_string(),
            ));
        }
        let target_id = value_ref(collision.get_field("Target")).filter(|target| *target >= 0);
        if matches!(game, NifGame::Skyrim | NifGame::SkyrimSe)
            && target_id
                .and_then(|target| nif.get_block(target as usize))
                .is_some_and(|target| {
                    string_field(target, "Name").is_none_or(|name| name.is_empty())
                })
        {
            findings.push(finding(
                "error",
                "collision-target-name",
                collision,
                Some("Target".to_string()),
                "Collision target node must have a name".to_string(),
            ));
        }
        if let Some(parent_id) = parent_by_collision.get(&collision.block_id)
            && target_id != Some(*parent_id as i32)
        {
            findings.push(finding(
                "error",
                "collision-target",
                collision,
                Some("Target".to_string()),
                format!("Target must link to parent node block {parent_id}"),
            ));
        }

        let Some(body) = body_id.and_then(|body| nif.get_block(body as usize)) else {
            continue;
        };
        if body.get_field("Shape").is_some()
            && value_ref(body.get_field("Shape"))
                .filter(|shape| *shape >= 0)
                .and_then(|shape| nif.get_block(shape as usize))
                .is_none()
            && body.type_name != "bhkAabbPhantom"
        {
            findings.push(finding(
                "error",
                "collision-shape",
                body,
                Some("Shape".to_string()),
                "Missing rigid body shape".to_string(),
            ));
        }
        let Some(parent_id) = parent_by_collision.get(&collision.block_id).copied() else {
            continue;
        };
        let Some(compressed) = value_ref(body.get_field("Shape"))
            .filter(|shape| *shape >= 0)
            .and_then(|shape| nif.get_block(shape as usize))
            .filter(|shape| SCHEMA.is_subtype_of(&shape.type_name, "bhkMoppBvTreeShape"))
            .and_then(|mopp| value_ref(mopp.get_field("Shape")))
            .filter(|shape| *shape >= 0)
            .and_then(|shape| nif.get_block(shape as usize))
            .filter(|shape| SCHEMA.is_subtype_of(&shape.type_name, "bhkCompressedMeshShape"))
        else {
            continue;
        };
        let compressed_target = value_ref(compressed.get_field("Target"))
            .filter(|target| *target >= 0)
            .and_then(|target| nif.get_block(target as usize));
        if compressed_target
            .is_some_and(|target| string_field(target, "Name").is_none_or(|name| name.is_empty()))
        {
            findings.push(finding(
                "error",
                "collision-target-name",
                compressed,
                Some("Target".to_string()),
                "Compressed-mesh target node must have a name".to_string(),
            ));
        }
        let root_id = root_ids(nif).into_iter().next();
        if compressed_target.map(|target| target.block_id) != Some(parent_id)
            && compressed_target.map(|target| target.block_id) != root_id
        {
            findings.push(finding(
                "error",
                "collision-target",
                compressed,
                Some("Target".to_string()),
                "Compressed-mesh Target must link to its collision parent or the root".to_string(),
            ));
        }
    }

    if !matches!(
        game,
        NifGame::Oblivion | NifGame::Fo3 | NifGame::Skyrim | NifGame::SkyrimSe
    ) {
        return;
    }
    let animated_collisions = animated_collision_ids(nif);
    let animated = nif
        .blocks
        .iter()
        .any(|block| block.type_name == "NiControllerManager");
    for body in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "bhkRigidBody"))
    {
        let motion_system = value_u64(rigid_body_value(body, "Motion System")).unwrap_or(0);
        let motion_quality = value_u64(rigid_body_value(body, "Motion Quality")).unwrap_or(0);
        let shape = value_ref(body.get_field("Shape"))
            .filter(|shape| *shape >= 0)
            .and_then(|shape| nif.get_block(shape as usize));
        let dynamic = is_dynamic_rigid_body(nif, body);
        if motion_system == 7 && motion_quality != 1 {
            findings.push(finding(
                "error",
                "collision-motion-quality",
                body,
                Some("Motion Quality".to_string()),
                "Fixed motion system requires fixed motion quality".to_string(),
            ));
        }
        if matches!(game, NifGame::Oblivion | NifGame::Fo3)
            && motion_system != 7
            && motion_quality == 1
        {
            findings.push(finding(
                "error",
                "collision-motion-quality",
                body,
                Some("Motion Quality".to_string()),
                "Non-fixed motion system cannot use fixed motion quality".to_string(),
            ));
        }
        if matches!(game, NifGame::Oblivion | NifGame::Fo3) && matches!(motion_system, 3 | 5) {
            findings.push(finding(
                "error",
                "collision-motion-system",
                body,
                Some("Motion System".to_string()),
                "Stabilized motion systems are not supported before Skyrim".to_string(),
            ));
        }
        if (dynamic || motion_system == 6)
            && shape.is_some_and(|shape| shape.type_name == "bhkMoppBvTreeShape")
        {
            findings.push(finding(
                "error",
                "dynamic-mopp",
                body,
                Some("Shape".to_string()),
                "Dynamic or keyframed collision must use simple shapes instead of MOPP".to_string(),
            ));
        }
        if !dynamic && value_u64(rigid_body_value(body, "Body Flags")).unwrap_or(0) > 0 {
            findings.push(finding(
                "warning",
                "static-body-flags",
                body,
                Some("Body Flags".to_string()),
                "Wind-simulation body flags on static collision hurt performance".to_string(),
            ));
        }
        let minimum_penetration = if matches!(game, NifGame::Oblivion | NifGame::Fo3) {
            0.01
        } else {
            0.002
        };
        if value_f64(rigid_body_value(body, "Penetration Depth"))
            .is_some_and(|depth| depth > 0.0 && depth < minimum_penetration)
        {
            findings.push(finding(
                "warning",
                "collision-penetration-depth",
                body,
                Some("Penetration Depth".to_string()),
                format!("Penetration depth below {minimum_penetration} loses Havok precision"),
            ));
        }
        if dynamic && !animated {
            let mass = value_f64(rigid_body_value(body, "Mass")).unwrap_or(0.0);
            if mass == 0.0 {
                findings.push(finding(
                    "error",
                    "collision-mass",
                    body,
                    Some("Mass".to_string()),
                    "Moveable collision has zero mass".to_string(),
                ));
            } else if mass < 0.95 {
                findings.push(finding(
                    "warning",
                    "collision-mass",
                    body,
                    Some("Mass".to_string()),
                    "Low moveable mass can cause physics precision issues".to_string(),
                ));
            }
            if mass > 0.0 && inertia_tensor_is_bad(rigid_body_value(body, "Inertia Tensor")) {
                findings.push(finding(
                    "error",
                    "collision-inertia",
                    body,
                    Some("Inertia Tensor".to_string()),
                    "Moveable collision has a zero or invalid inertia tensor".to_string(),
                ));
            }
            if value_f64(rigid_body_value(body, "Max Angular Velocity")).unwrap_or(0.0) < 1.0 {
                findings.push(finding(
                    "warning",
                    "collision-angular-velocity",
                    body,
                    Some("Max Angular Velocity".to_string()),
                    "Max Angular Velocity below 1.0 can cause terrain sinking".to_string(),
                ));
            }
            if matches!(game, NifGame::Skyrim | NifGame::SkyrimSe)
                && !value_bool(rigid_body_value(body, "Enable Deactivation")).unwrap_or(false)
            {
                findings.push(finding(
                    "warning",
                    "collision-deactivation",
                    body,
                    Some("Enable Deactivation".to_string()),
                    "Dynamic collision with deactivation disabled hurts performance".to_string(),
                ));
            }
            if matches!(game, NifGame::Oblivion | NifGame::Fo3)
                && value_u64(rigid_body_value(body, "Deactivator Type")) == Some(0)
            {
                findings.push(finding(
                    "warning",
                    "collision-deactivation",
                    body,
                    Some("Deactivator Type".to_string()),
                    "Dynamic collision uses DEACTIVATOR_NEVER".to_string(),
                ));
            }
            if value_u64(rigid_body_value(body, "Solver Deactivation")) == Some(0) {
                findings.push(finding(
                    "warning",
                    "collision-deactivation",
                    body,
                    Some("Solver Deactivation".to_string()),
                    "Dynamic collision has solver deactivation disabled".to_string(),
                ));
            }
        }
    }

    for collision in nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkCollisionObject")
    {
        let Some(body) = value_ref(collision.get_field("Body"))
            .filter(|id| *id >= 0)
            .and_then(|id| nif.get_block(id as usize))
        else {
            continue;
        };
        let Some(layer) = rigid_body_nested_u64(body, "Havok Filter", "Layer") else {
            continue;
        };
        let is_animated = animated_collisions.contains(&collision.block_id);
        if matches!(layer, 2 | 28) && !is_animated {
            findings.push(finding(
                "warning",
                "collision-animated-layer",
                collision,
                Some("Body.Havok Filter.Layer".to_string()),
                "Animated layer is used on non-animated collision".to_string(),
            ));
        }
        if !is_animated {
            continue;
        }
        if !matches!(layer, 2 | 4 | 5 | 6 | 14 | 15 | 16 | 28) {
            findings.push(finding(
                "error",
                "collision-animated-layer",
                body,
                Some("Havok Filter.Layer".to_string()),
                "Animated collision must use an animated collision layer".to_string(),
            ));
        }
        let flags = value_u64(collision.get_field("Flags")).unwrap_or(0);
        if matches!(game, NifGame::Skyrim | NifGame::SkyrimSe)
            && matches!(layer, 2 | 28)
            && flags & BHKCO_SET_LOCAL == 0
        {
            findings.push(finding(
                "error",
                "collision-set-local",
                collision,
                Some("Flags".to_string()),
                "Animated layer collision is missing SET_LOCAL".to_string(),
            ));
        }
        if matches!(game, NifGame::Oblivion | NifGame::Fo3)
            && value_u64(rigid_body_value(body, "Motion System")) == Some(6)
            && flags & BHKCO_USE_VEL == 0
        {
            findings.push(finding(
                "error",
                "collision-use-velocity",
                collision,
                Some("Flags".to_string()),
                "Transformed keyframed animated collision is missing USE_VEL".to_string(),
            ));
        }
    }

    for constraint in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "bhkConstraint"))
    {
        if fields_contain_named_zero(&constraint.fields, "Cone Max Angle") {
            findings.push(finding(
                "warning",
                "constraint-cone-angle",
                constraint,
                Some("Cone Max Angle".to_string()),
                "Zero ragdoll cone maximum angle can cause jittering".to_string(),
            ));
        }
    }
}

fn animated_collision_ids(nif: &NifFile) -> HashSet<usize> {
    let names = named_av_objects(nif);
    let mut collisions = HashSet::new();
    for sequence in nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiControllerSequence")
    {
        for entry in value_array(sequence.get_field("Controlled Blocks")) {
            if nested_value(Some(entry), "Controller Type").and_then(value_string)
                != Some("NiTransformController")
            {
                continue;
            }
            let Some(interpolator) = nested_value(Some(entry), "Interpolator")
                .and_then(|value| value_ref(Some(value)))
                .filter(|id| *id >= 0)
                .and_then(|id| nif.get_block(id as usize))
                .filter(|block| block.type_name == "NiTransformInterpolator")
            else {
                continue;
            };
            if value_ref(interpolator.get_field("Data")).unwrap_or(-1) < 0 {
                continue;
            }
            let Some(node_id) = nested_value(Some(entry), "Node Name")
                .and_then(value_string)
                .and_then(|name| names.get(name))
                .copied()
            else {
                continue;
            };
            collect_node_collision_ids(nif, node_id, &mut HashSet::new(), &mut collisions);
        }
    }
    for node in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiNode"))
    {
        if node_has_data_transform_controller(nif, node) {
            collect_node_collision_ids(nif, node.block_id, &mut HashSet::new(), &mut collisions);
        }
    }
    collisions
}

fn node_has_data_transform_controller(nif: &NifFile, node: &NifBlock) -> bool {
    let mut controller_id = value_ref(node.get_field("Controller")).unwrap_or(-1);
    let mut seen = HashSet::new();
    while controller_id >= 0 && seen.insert(controller_id) {
        let Some(controller) = nif.get_block(controller_id as usize) else {
            break;
        };
        if controller.type_name == "NiTransformController"
            && value_ref(controller.get_field("Interpolator"))
                .filter(|id| *id >= 0)
                .and_then(|id| nif.get_block(id as usize))
                .filter(|interpolator| interpolator.type_name == "NiTransformInterpolator")
                .and_then(|interpolator| value_ref(interpolator.get_field("Data")))
                .is_some_and(|id| id >= 0)
        {
            return true;
        }
        controller_id = value_ref(controller.get_field("Next Controller")).unwrap_or(-1);
    }
    false
}

fn collect_node_collision_ids(
    nif: &NifFile,
    node_id: usize,
    visited: &mut HashSet<usize>,
    collisions: &mut HashSet<usize>,
) {
    if !visited.insert(node_id) {
        return;
    }
    let Some(node) = nif.get_block(node_id) else {
        return;
    };
    if let Some(collision) = value_ref(node.get_field("Collision Object"))
        .filter(|id| *id >= 0)
        .map(|id| id as usize)
    {
        collisions.insert(collision);
    }
    for child_id in ref_values(node.get_field("Children")) {
        let child_id = child_id as usize;
        if nif
            .get_block(child_id)
            .is_some_and(|child| SCHEMA.is_subtype_of(&child.type_name, "NiNode"))
        {
            collect_node_collision_ids(nif, child_id, visited, collisions);
        }
    }
}

fn audit_collision_mopp(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    let game = nif_game(nif);
    if !matches!(
        game,
        NifGame::Oblivion | NifGame::Fo3 | NifGame::Skyrim | NifGame::SkyrimSe
    ) || !nif
        .blocks
        .iter()
        .any(|block| block.type_name == "bhkMoppBvTreeShape")
    {
        return;
    }
    let mut rendered_triangles = 0usize;
    let mut collision_triangles = 0usize;
    for block in &nif.blocks {
        if SCHEMA.is_subtype_of(&block.type_name, "BSTriShape") {
            rendered_triangles += value_u64(block.get_field("Num Triangles")).unwrap_or(0) as usize;
        } else if SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom") {
            rendered_triangles += value_ref(block.get_field("Data"))
                .filter(|data| *data >= 0)
                .and_then(|data| nif.get_block(data as usize))
                .and_then(|data| value_u64(data.get_field("Num Triangles")))
                .unwrap_or(0) as usize;
        } else if block.type_name == "bhkNiTriStripsShape" {
            for data_id in ref_values(block.get_field("Strips Data")) {
                collision_triangles += nif
                    .get_block(data_id as usize)
                    .filter(|data| data.type_name == "NiTriStripsData")
                    .and_then(|data| value_u64(data.get_field("Num Triangles")))
                    .unwrap_or(0) as usize;
            }
        } else if block.type_name == "hkPackedNiTriStripsData" {
            collision_triangles += value_array(block.get_field("Triangles")).len();
        } else if block.type_name == "bhkCompressedMeshShapeData" {
            collision_triangles += value_array(block.get_field("Big Tris")).len();
            for chunk in value_array(block.get_field("Chunks")) {
                let strip_lengths = nested_value(Some(chunk), "Strip Lengths")
                    .map(numeric_scalars)
                    .unwrap_or_default();
                let strip_indices = strip_lengths.iter().sum::<usize>();
                collision_triangles += strip_lengths
                    .into_iter()
                    .map(|length| length.saturating_sub(2))
                    .sum::<usize>();
                let indices = nested_value(Some(chunk), "Indices")
                    .map(numeric_scalars)
                    .unwrap_or_default()
                    .len();
                collision_triangles += indices.saturating_sub(strip_indices) / 3;
            }
        }
    }
    if rendered_triangles <= 10 || collision_triangles <= 10 {
        return;
    }
    let ratio = collision_triangles.saturating_mul(100) / rendered_triangles;
    if ratio > 50 {
        let owner = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "bhkMoppBvTreeShape")
            .unwrap();
        findings.push(finding(
            "warning",
            "collision-mopp-complexity",
            owner,
            None,
            format!(
                "MOPP collision-to-rendered triangle ratio is {ratio}% ({collision_triangles}/{rendered_triangles})"
            ),
        ));
    }
}

fn audit_consistency_flags(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    if !matches!(
        nif_game(nif),
        NifGame::Oblivion | NifGame::Fo3 | NifGame::Skyrim
    ) {
        return;
    }
    for shape in nif
        .blocks
        .iter()
        .filter(|shape| SCHEMA.is_subtype_of(&shape.type_name, "NiGeometry"))
    {
        let Some(data) = value_ref(shape.get_field("Data"))
            .filter(|data| *data >= 0)
            .and_then(|data| nif.get_block(data as usize))
        else {
            continue;
        };
        let Some(actual) = value_u64(data.get_field("Consistency Flags")) else {
            continue;
        };
        let controller = value_ref(shape.get_field("Controller"))
            .filter(|controller| *controller >= 0)
            .and_then(|controller| nif.get_block(controller as usize));
        let mutable = SCHEMA.is_subtype_of(&shape.type_name, "NiParticles")
            || controller.is_some_and(|controller| {
                SCHEMA.is_subtype_of(&controller.type_name, "NiGeomMorpherController")
                    || SCHEMA.is_subtype_of(&controller.type_name, "NiUVController")
            });
        let expected = if mutable { 0 } else { 0x4000 };
        if actual != expected {
            findings.push(finding(
                "warning",
                "consistency-flags",
                data,
                Some("Consistency Flags".to_string()),
                format!("Consistency Flags are {actual:#06x}; expected {expected:#06x}"),
            ));
        }
    }
}

fn audit_skinning(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    let game = nif_game(nif);
    let root_is_node = root_ids(nif)
        .into_iter()
        .next()
        .and_then(|root| nif.get_block(root))
        .is_some_and(|root| root.type_name == "NiNode");
    if matches!(game, NifGame::Fo3 | NifGame::Skyrim | NifGame::SkyrimSe) && root_is_node {
        for skin in nif
            .blocks
            .iter()
            .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiSkinInstance"))
        {
            if skin.type_name != "BSDismemberSkinInstance" {
                findings.push(finding(
                    "error",
                    "skin-instance-type",
                    skin,
                    None,
                    "Skin instance must be BSDismemberSkinInstance".to_string(),
                ));
                continue;
            }
            if value_ref(skin.get_field("Skeleton Root")) != Some(0) {
                findings.push(finding(
                    "error",
                    "skin-skeleton-root",
                    skin,
                    Some("Skeleton Root".to_string()),
                    "Skeleton Root must link to block 0".to_string(),
                ));
            }
            let mut body_parts = HashSet::new();
            for (index, partition) in value_array(skin.get_field("Partitions")).iter().enumerate() {
                let NifValue::Struct(fields) = partition else {
                    continue;
                };
                let Some(body_part) = value_u64(fields.get("Body Part")) else {
                    continue;
                };
                if matches!(game, NifGame::Skyrim | NifGame::SkyrimSe)
                    && !(30..=62).contains(&body_part)
                {
                    findings.push(finding(
                        "error",
                        "skin-body-part",
                        skin,
                        Some(format!("Partitions[{index}].Body Part")),
                        format!("Invalid Skyrim body part {body_part}"),
                    ));
                }
                if !body_parts.insert(body_part) {
                    findings.push(finding(
                        "error",
                        "skin-body-part",
                        skin,
                        Some(format!("Partitions[{index}].Body Part")),
                        format!("Repeated body part {body_part}"),
                    ));
                }
            }
            if let Some(partition) = value_ref(skin.get_field("Skin Partition"))
                .filter(|partition| *partition >= 0)
                .and_then(|partition| nif.get_block(partition as usize))
            {
                let dismember_count = value_array(skin.get_field("Partitions")).len();
                let skin_count =
                    value_u64(partition.get_field("Num Partitions")).unwrap_or(0) as usize;
                if dismember_count < skin_count {
                    findings.push(finding(
                        "error",
                        "skin-partition-count",
                        skin,
                        Some("Partitions".to_string()),
                        format!(
                            "Dismember partition count {dismember_count} is lower than skin partition count {skin_count}"
                        ),
                    ));
                }
            }
        }
    }
    if matches!(
        game,
        NifGame::Skyrim | NifGame::SkyrimSe | NifGame::Fo4 | NifGame::Fo76
    ) {
        for shape in nif
            .blocks
            .iter()
            .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "BSDynamicTriShape"))
        {
            if value_ref(shape.get_field("Skin")).unwrap_or(-1) < 0 {
                findings.push(finding(
                    "warning",
                    "missing-skin-instance",
                    shape,
                    Some("Skin".to_string()),
                    "BSDynamicTriShape is missing its skin instance".to_string(),
                ));
            }
        }
    }
}

fn audit_geometry(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    for shape in &nif.blocks {
        let packed = SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape");
        let legacy = SCHEMA.is_subtype_of(&shape.type_name, "NiTriBasedGeom");
        if !packed && !legacy {
            continue;
        }
        let geometry = if packed {
            shape
        } else {
            let Some(data) = value_ref(shape.get_field("Data"))
                .filter(|data| *data >= 0)
                .and_then(|data| nif.get_block(data as usize))
            else {
                continue;
            };
            data
        };
        let num_vertices = value_u64(geometry.get_field("Num Vertices")).unwrap_or(0) as usize;
        let vertices =
            value_array(geometry.get_field(if packed { "Vertex Data" } else { "Vertices" }));
        let triangles = value_array(geometry.get_field("Triangles"));
        if packed && num_vertices > 0 {
            let duplicate_count = duplicate_value_count(vertices);
            if duplicate_count > 0 {
                findings.push(finding(
                    "warning",
                    "duplicate-vertices",
                    geometry,
                    Some("Vertex Data".to_string()),
                    format!(
                        "Duplicate vertices (Num Vertices: {num_vertices}, Dup Vertices: {duplicate_count})"
                    ),
                ));
            }
        }
        if num_vertices > 0 {
            let mut used = vec![false; num_vertices];
            let mut invalid_reported = false;
            for triangle in triangles {
                for index in triangle_indices(triangle) {
                    if index < num_vertices {
                        used[index] = true;
                    } else if !invalid_reported {
                        findings.push(finding(
                            "error",
                            "triangle-index",
                            geometry,
                            Some("Triangles".to_string()),
                            format!(
                                "Triangle index {index} exceeds the number of vertices {num_vertices}"
                            ),
                        ));
                        invalid_reported = true;
                    }
                }
            }
            let used_count = used.into_iter().filter(|used| *used).count();
            if used_count != num_vertices {
                findings.push(finding(
                    "warning",
                    "unused-vertices",
                    geometry,
                    Some(if packed {
                        "Vertex Data".to_string()
                    } else {
                        "Vertices".to_string()
                    }),
                    format!(
                        "Unused vertices (Num Vertices: {num_vertices}, Used vertices: {used_count})"
                    ),
                ));
            }
        }
        if geometry.type_name == "NiTriStripsData" {
            let strip_indices = geometry
                .get_field("Strips")
                .map(numeric_scalars)
                .unwrap_or_default();
            let mut used = vec![false; num_vertices];
            let mut invalid = None;
            for index in strip_indices {
                if index < num_vertices {
                    used[index] = true;
                } else if invalid.is_none() {
                    invalid = Some(index);
                }
            }
            if let Some(index) = invalid {
                findings.push(finding(
                    "error",
                    "strip-index",
                    geometry,
                    Some("Strips".to_string()),
                    format!("Strip point {index} exceeds the number of vertices {num_vertices}"),
                ));
            }
            let used_count = used.into_iter().filter(|used| *used).count();
            if num_vertices > 0 && used_count != num_vertices {
                findings.push(finding(
                    "warning",
                    "unused-vertices",
                    geometry,
                    Some("Strips".to_string()),
                    format!(
                        "Unused vertices (Num Vertices: {num_vertices}, Used vertices: {used_count})"
                    ),
                ));
            }
            let strips = value_u64(geometry.get_field("Num Strips")).unwrap_or(0);
            if strips > 1 {
                findings.push(finding(
                    "warning",
                    "multiple-triangle-strips",
                    geometry,
                    Some("Num Strips".to_string()),
                    format!("Num Strips is {strips}; one strip reduces draw calls"),
                ));
            }
        }
        let colors = geometry_vertex_colors(nif, shape);
        let all_white = !colors.is_empty()
            && colors
                .iter()
                .all(|color| color.iter().all(|component| *component == 1.0));
        let shader = geometry_shader_id(nif, shape).and_then(|shader| nif.get_block(shader));
        let colors_required = shader.is_some_and(|shader| {
            value_u64(shader.get_field("Shader Flags 2"))
                .is_some_and(|flags| flags & SLSF2_TREE_ANIM != 0)
                || value_u64(shader.get_field("Shader Type")) == Some(SHADER_PARALLAX)
        });
        if all_white && !colors_required {
            findings.push(finding(
                "info",
                "all-white-vertex-colors",
                geometry,
                Some(if packed {
                    "Vertex Data".to_string()
                } else {
                    "Vertex Colors".to_string()
                }),
                "All white #FFFFFFFF vertex colors".to_string(),
            ));
        }
        if !packed
            && colors.iter().any(|color| {
                color
                    .iter()
                    .any(|component| *component < 0.0 || *component > 1.0)
            })
        {
            findings.push(finding(
                "warning",
                "hdr-vertex-colors",
                geometry,
                Some("Vertex Colors".to_string()),
                "Vertex colors contain values outside the 0..1 range".to_string(),
            ));
        }
        if colors.iter().any(|color| color[3] < 1.0)
            && matches!(
                nif_game(nif),
                NifGame::Fo3 | NifGame::Skyrim | NifGame::SkyrimSe
            )
            && geometry_has_property(nif, shape, "NiAlphaProperty")
            && shader.is_some_and(|shader| shader_flags1(shader) & SLSF1_VERTEX_ALPHA == 0)
        {
            findings.push(finding(
                "warning",
                "vertex-alpha-flag",
                geometry,
                None,
                "Vertex alpha and NiAlphaProperty are present but Vertex_Alpha is not set"
                    .to_string(),
            ));
        }
    }

    for data in nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "hkPackedNiTriStripsData")
    {
        let num_vertices = value_u64(data.get_field("Num Vertices")).unwrap_or(0) as usize;
        let triangles = value_array(data.get_field("Triangles"));
        let mut used = vec![false; num_vertices];
        let mut invalid = None;
        for triangle in triangles {
            for index in triangle_indices(triangle) {
                if index < num_vertices {
                    used[index] = true;
                } else if invalid.is_none() {
                    invalid = Some(index);
                }
            }
        }
        if let Some(index) = invalid {
            findings.push(finding(
                "error",
                "triangle-index",
                data,
                Some("Triangles".to_string()),
                format!("Triangle index {index} exceeds the number of vertices {num_vertices}"),
            ));
        }
        let used_count = used.into_iter().filter(|used| *used).count();
        if num_vertices > 0 && used_count != num_vertices {
            findings.push(finding(
                "warning",
                "unused-vertices",
                data,
                Some("Triangles".to_string()),
                format!(
                    "Unused vertices (Num Vertices: {num_vertices}, Used vertices: {used_count})"
                ),
            ));
        }
    }

    for partition in nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiSkinPartition")
    {
        for (index, entry) in value_array(partition.get_field("Partitions"))
            .iter()
            .enumerate()
        {
            if nested_u64(Some(entry), "Num Strips").unwrap_or(0) > 1 {
                findings.push(finding(
                    "warning",
                    "multiple-triangle-strips",
                    partition,
                    Some(format!("Partitions[{index}].Num Strips")),
                    "Skin partition uses more than one strip, increasing draw calls".to_string(),
                ));
            }
        }
    }
}

fn geometry_vertex_colors(nif: &NifFile, shape: &NifBlock) -> Vec<[f64; 4]> {
    if SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape") {
        return value_array(shape.get_field("Vertex Data"))
            .iter()
            .filter_map(|vertex| {
                let NifValue::Struct(fields) = vertex else {
                    return None;
                };
                byte_color(fields.get("Vertex Colors"))
            })
            .collect();
    }
    value_ref(shape.get_field("Data"))
        .filter(|data| *data >= 0)
        .and_then(|data| nif.get_block(data as usize))
        .map(|data| {
            value_array(data.get_field("Vertex Colors"))
                .iter()
                .filter_map(float_color)
                .collect()
        })
        .unwrap_or_default()
}

fn geometry_has_property(nif: &NifFile, shape: &NifBlock, property_type: &str) -> bool {
    if property_type == "NiAlphaProperty"
        && value_ref(shape.get_field("Alpha Property")).is_some_and(|property| property >= 0)
    {
        return true;
    }
    ref_values(shape.get_field("Properties"))
        .into_iter()
        .filter_map(|property| nif.get_block(property as usize))
        .any(|property| SCHEMA.is_subtype_of(&property.type_name, property_type))
}

fn duplicate_value_count(values: &[NifValue]) -> usize {
    let mut buckets: HashMap<u64, Vec<&NifValue>> = HashMap::new();
    let mut duplicates = 0usize;
    for value in values {
        let hash = nif_value_hash(value);
        let bucket = buckets.entry(hash).or_default();
        if bucket.iter().any(|candidate| *candidate == value) {
            duplicates += 1;
        } else {
            bucket.push(value);
        }
    }
    duplicates
}

fn nif_value_hash(value: &NifValue) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_value(value, &mut hasher);
    hasher.finish()
}

fn hash_value(value: &NifValue, hasher: &mut DefaultHasher) {
    std::mem::discriminant(value).hash(hasher);
    match value {
        NifValue::Null => {}
        NifValue::Bool(value) => value.hash(hasher),
        NifValue::Int(value) => value.hash(hasher),
        NifValue::UInt(value) => value.hash(hasher),
        NifValue::Float(value) => value.to_bits().hash(hasher),
        NifValue::FloatNan(value) => value.hash(hasher),
        NifValue::String(value) | NifValue::Char(value) => value.hash(hasher),
        NifValue::Ref(value) => value.hash(hasher),
        NifValue::Vec3(values) | NifValue::Color3(values) => {
            for value in values {
                value.to_bits().hash(hasher);
            }
        }
        NifValue::Vec4(values) | NifValue::Color4(values) | NifValue::Quaternion(values) => {
            for value in values {
                value.to_bits().hash(hasher);
            }
        }
        NifValue::Matrix33(values) => {
            for row in values {
                for value in row {
                    value.to_bits().hash(hasher);
                }
            }
        }
        NifValue::Matrix44(values) => {
            for row in values {
                for value in row {
                    value.to_bits().hash(hasher);
                }
            }
        }
        NifValue::Array(values) => {
            for value in values {
                hash_value(value, hasher);
            }
        }
        NifValue::Struct(fields) => {
            for (key, value) in fields {
                key.hash(hasher);
                hash_value(value, hasher);
            }
        }
        NifValue::Bytes(values) => values.hash(hasher),
    }
}

fn triangle_indices(value: &NifValue) -> Vec<usize> {
    let NifValue::Struct(fields) = value else {
        return Vec::new();
    };
    let direct = ["v1", "v2", "v3"]
        .iter()
        .filter_map(|field| {
            fields
                .iter()
                .find(|(name, _)| bare_name(name) == *field)
                .and_then(|(_, value)| value_u64(Some(value)))
                .map(|value| value as usize)
        })
        .collect::<Vec<_>>();
    if !direct.is_empty() {
        return direct;
    }
    fields
        .values()
        .flat_map(triangle_indices)
        .collect::<Vec<_>>()
}

fn audit_texture_set_slots(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    let game = nif_game(nif);
    for shader in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "BSShaderProperty"))
    {
        let Some(texture_set) = value_ref(shader.get_field("Texture Set"))
            .filter(|id| *id >= 0)
            .and_then(|id| nif.get_block(id as usize))
        else {
            continue;
        };
        let textures = value_array(texture_set.get_field("Textures"));
        let flags1 = shader_flags1(shader);
        let flags2 = value_u64(shader.get_field("Shader Flags 2")).unwrap_or(0);
        let shader_type = value_u64(shader.get_field("Shader Type")).unwrap_or(0);
        if game == NifGame::Fo3
            && !texture_slot(textures, 4).is_empty()
            && flags1
                & (SLSF1_ENVIRONMENT_MAPPING
                    | SLSF1_EYE_ENVIRONMENT_MAPPING
                    | SLSF1_WINDOW_ENVIRONMENT_MAPPING)
                == 0
        {
            findings.push(finding(
                "error",
                "shader-texture-slot",
                shader,
                Some("Texture Set.Textures[4]".to_string()),
                "Environment-map texture is assigned without an environment-map flag".to_string(),
            ));
        }
        if !matches!(game, NifGame::Skyrim | NifGame::SkyrimSe) {
            continue;
        }
        let unused = [
            (
                2,
                !matches!(shader_type, SHADER_GLOW | SHADER_FACEGEN | SHADER_SKIN_TINT)
                    && flags2 & (SLSF2_SOFT_LIGHTING | SLSF2_RIM_LIGHTING) == 0,
            ),
            (3, !matches!(shader_type, SHADER_PARALLAX | SHADER_FACEGEN)),
            (
                4,
                !matches!(
                    shader_type,
                    SHADER_ENVIRONMENT_MAP | SHADER_MULTI_LAYER_PARALLAX | SHADER_EYE_ENVMAP
                ),
            ),
            (
                5,
                !matches!(
                    shader_type,
                    SHADER_ENVIRONMENT_MAP | SHADER_MULTI_LAYER_PARALLAX | SHADER_EYE_ENVMAP
                ),
            ),
            (
                6,
                !matches!(shader_type, SHADER_FACEGEN | SHADER_MULTI_LAYER_PARALLAX),
            ),
            (
                7,
                flags2 & SLSF2_BACK_LIGHTING == 0 && flags1 & SLSF1_MODEL_SPACE_NORMALS == 0,
            ),
        ];
        for (slot, is_unused) in unused {
            if is_unused && !texture_slot(textures, slot).is_empty() {
                findings.push(finding(
                    "warning",
                    "shader-texture-slot",
                    texture_set,
                    Some(format!("Textures[{slot}]")),
                    format!("Texture slot {slot} is assigned but unused by the shader"),
                ));
            }
        }
    }
}

fn texture_path_is_invalid(path: &str) -> bool {
    path.chars().any(|character| {
        character.is_control() || matches!(character, '<' | '>' | '"' | '|' | '?' | '*')
    }) || path.starts_with('/')
        || path.starts_with('\\')
        || path.as_bytes().get(1) == Some(&b':')
}

fn audit_shader_flags(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    let game = nif_game(nif);
    let mut external_emittance_shader = None;
    for (shader_id, usage) in shader_usage(nif) {
        let Some(shader) = nif.get_block(shader_id) else {
            continue;
        };
        let flags1 = shader_flags1(shader);
        let flags2 = value_u64(shader.get_field("Shader Flags 2")).unwrap_or(0);
        if flags1 & SLSF1_EXTERNAL_EMITTANCE != 0 && external_emittance_shader.is_none() {
            external_emittance_shader = Some(shader);
        }
        if usage.skinned > 0 && flags1 & SLSF1_SKINNED == 0 {
            findings.push(finding(
                "error",
                "skinned-shader-flag",
                shader,
                Some("Shader Flags 1".to_string()),
                "Skinned geometry is present but Skinned is not set".to_string(),
            ));
        } else if usage.skinned == 0 && usage.total > 0 && flags1 & SLSF1_SKINNED != 0 {
            findings.push(finding(
                "error",
                "skinned-shader-flag",
                shader,
                Some("Shader Flags 1".to_string()),
                "Shader is not used by skinned geometry but Skinned is set".to_string(),
            ));
        }
        if game.uses_runtime_vertex_color_flags()
            && usage.with_vertex_colors > 0
            && flags2 & SLSF2_VERTEX_COLORS == 0
        {
            findings.push(finding(
                "error",
                "vertex-color-shader-flag",
                shader,
                Some("Shader Flags 2".to_string()),
                "Vertex colors are present but Vertex_Colors is not set".to_string(),
            ));
        } else if game.uses_runtime_vertex_color_flags()
            && usage.with_vertex_colors == 0
            && usage.total > 0
            && flags2 & SLSF2_VERTEX_COLORS != 0
        {
            findings.push(finding(
                "error",
                "vertex-color-shader-flag",
                shader,
                Some("Shader Flags 2".to_string()),
                "Vertex colors are missing but Vertex_Colors is set".to_string(),
            ));
        }
        if usage.with_vertex_colors == 0 && usage.total > 0 && flags1 & SLSF1_VERTEX_ALPHA != 0 {
            findings.push(finding(
                "error",
                "vertex-alpha-shader-flag",
                shader,
                Some("Shader Flags 1".to_string()),
                "Vertex colors are missing but Vertex_Alpha is set".to_string(),
            ));
        }
        if game == NifGame::Fo3 {
            let shader_type = value_u64(shader.get_field("Shader Type")).unwrap_or(0);
            if (shader_type == FO3_SHADER_SKIN) != (flags1 & SLSF1_FACEGEN != 0) {
                shader_rule_finding(
                    shader,
                    "SHADER_SKIN and FaceGen must be set together",
                    "Shader Type",
                    findings,
                );
            }
            if shader_type == FO3_SHADER_NOLIGHTING
                && shader.type_name == "BSShaderPPLightingProperty"
            {
                shader_rule_finding(
                    shader,
                    "BSShaderPPLightingProperty cannot use SHADER_NOLIGHTING",
                    "Shader Type",
                    findings,
                );
            }
        }
        if game.uses_skyrim_shader_rules() {
            audit_skyrim_shader_rules(
                nif,
                shader,
                flags1,
                flags2,
                usage.with_vertex_colors > 0,
                findings,
            );
        }
    }
    if let Some(shader) = external_emittance_shader
        && nif
            .blocks
            .iter()
            .find(|block| string_field(block, "Name").as_deref() == Some("BSX"))
            .is_some_and(|bsx| {
                value_u64(bsx.get_field("Integer Data")).unwrap_or(0) & BSX_EXTERNAL_EMIT == 0
            })
    {
        shader_rule_finding(
            shader,
            "External_Emittance is set but BSX lacks External Emit",
            "Shader Flags 1",
            findings,
        );
    }
    if game.uses_skyrim_shader_rules() {
        for shape in nif
            .blocks
            .iter()
            .filter(|shape| SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape"))
        {
            let Some(shader) = geometry_shader_id(nif, shape).and_then(|id| nif.get_block(id))
            else {
                continue;
            };
            let has_tangents = value_u64(shape.get_field("Vertex Desc"))
                .is_some_and(|descriptor| ((descriptor >> 44) & 0x10) != 0);
            let model_space = shader_flags1(shader) & SLSF1_MODEL_SPACE_NORMALS != 0;
            if !has_tangents && !model_space {
                findings.push(finding(
                    "error",
                    "missing-tangent-space",
                    shape,
                    Some("Vertex Desc".to_string()),
                    "Shape has no tangent space and does not use model-space normals".to_string(),
                ));
            }
        }
    }
}

fn audit_skyrim_shader_rules(
    nif: &NifFile,
    shader: &NifBlock,
    flags1: u64,
    flags2: u64,
    has_vertex_colors: bool,
    findings: &mut Vec<ValidationFinding>,
) {
    if flags1 & SLSF1_DYNAMIC_DECAL != 0 {
        if flags1 & SLSF1_DECAL == 0 {
            shader_rule_finding(
                shader,
                "Dynamic_Decal requires Decal",
                "Shader Flags 1",
                findings,
            );
        }
        if flags2 & SLSF2_ASSUME_SHADOWMASK == 0 {
            shader_rule_finding(
                shader,
                "Dynamic_Decal requires Assume_Shadowmask",
                "Shader Flags 2",
                findings,
            );
        }
    }
    if shader.type_name == "BSEffectShaderProperty" {
        if flags1 & (SLSF1_GRAYSCALE_COLOR | SLSF1_GRAYSCALE_ALPHA) != 0
            && string_field(shader, "Grayscale Texture").is_none_or(|texture| texture.is_empty())
        {
            shader_rule_finding(
                shader,
                "Grayscale flags require a Grayscale Texture",
                "Grayscale Texture",
                findings,
            );
        }
        return;
    }
    if shader.type_name != "BSLightingShaderProperty" {
        return;
    }
    let Some(texture_set) = value_ref(shader.get_field("Texture Set"))
        .filter(|texture_set| *texture_set >= 0)
        .and_then(|texture_set| nif.get_block(texture_set as usize))
    else {
        shader_rule_finding(
            shader,
            "Missing BSShaderTextureSet",
            "Texture Set",
            findings,
        );
        return;
    };
    let textures = value_array(texture_set.get_field("Textures"));
    for (slot, label) in [(0, "Diffuse"), (1, "Normal")] {
        if texture_slot(textures, slot).is_empty() {
            findings.push(finding(
                "error",
                "shader-texture-slot",
                texture_set,
                Some(format!("Textures[{slot}]")),
                format!("{label} texture slot {slot} must be set"),
            ));
        }
    }
    let shader_type = value_u64(shader.get_field("Shader Type")).unwrap_or(0);
    match shader_type {
        SHADER_ENVIRONMENT_MAP => {
            if flags1 & SLSF1_ENVIRONMENT_MAPPING == 0 {
                shader_rule_finding(
                    shader,
                    "Environment Map requires Environment_Mapping",
                    "Shader Flags 1",
                    findings,
                );
            }
            require_texture_slot(
                texture_set,
                textures,
                4,
                "Environment",
                shader_type,
                findings,
            );
            require_texture_slot(
                texture_set,
                textures,
                5,
                "Environment mask",
                shader_type,
                findings,
            );
        }
        SHADER_GLOW => {
            if flags2 & SLSF2_GLOW_MAP == 0 || flags1 & SLSF1_OWN_EMIT == 0 {
                shader_rule_finding(
                    shader,
                    "Glow Shader requires Glow_Map and Own_Emit",
                    "Shader Flags 1",
                    findings,
                );
            }
            if color_is_black(shader.get_field("Emissive Color")) {
                shader_rule_finding(
                    shader,
                    "Glow Shader requires a non-black Emissive Color",
                    "Emissive Color",
                    findings,
                );
            }
            require_texture_slot(texture_set, textures, 2, "Glow", shader_type, findings);
        }
        SHADER_PARALLAX => {
            if flags1 & SLSF1_PARALLAX == 0 {
                shader_rule_finding(
                    shader,
                    "Parallax shader requires Parallax",
                    "Shader Flags 1",
                    findings,
                );
            }
            if !has_vertex_colors {
                shader_rule_finding(
                    shader,
                    "Parallax shader requires vertex colors on its shape",
                    "Shader Type",
                    findings,
                );
            }
            if flags2 & SLSF2_MULTI_LAYER_PARALLAX != 0 {
                shader_rule_finding(
                    shader,
                    "Multi_Layer_Parallax cannot be used with Parallax shader type",
                    "Shader Flags 2",
                    findings,
                );
            }
            require_texture_slot(texture_set, textures, 3, "Parallax", shader_type, findings);
        }
        SHADER_FACEGEN => {
            if flags1 & SLSF1_FACEGEN == 0 || flags2 & SLSF2_SOFT_LIGHTING == 0 {
                shader_rule_finding(
                    shader,
                    "Facegen requires Facegen and Soft_Lighting",
                    "Shader Flags 1",
                    findings,
                );
            }
            if flags2 & SLSF2_ANISOTROPIC_LIGHTING != 0 {
                shader_rule_finding(
                    shader,
                    "Anisotropic_Lighting cannot be used with Facegen",
                    "Shader Flags 2",
                    findings,
                );
            }
            require_texture_slot(texture_set, textures, 2, "Skin tint", shader_type, findings);
            require_texture_slot(
                texture_set,
                textures,
                3,
                "Facegen detail",
                shader_type,
                findings,
            );
            require_texture_slot(
                texture_set,
                textures,
                6,
                "Facegen tint",
                shader_type,
                findings,
            );
        }
        SHADER_SKIN_TINT => {
            if flags1 & SLSF1_SKIN_TINT == 0 || flags2 & SLSF2_SOFT_LIGHTING == 0 {
                shader_rule_finding(
                    shader,
                    "Skin Tint requires Skin_Tint and Soft_Lighting",
                    "Shader Flags 1",
                    findings,
                );
            }
            require_texture_slot(texture_set, textures, 2, "Skin tint", shader_type, findings);
        }
        SHADER_HAIR_TINT if flags1 & SLSF1_HAIR_TINT == 0 => shader_rule_finding(
            shader,
            "Hair Tint requires Hair_Tint",
            "Shader Flags 1",
            findings,
        ),
        SHADER_MULTI_LAYER_PARALLAX => {
            if flags2 & SLSF2_MULTI_LAYER_PARALLAX == 0 {
                shader_rule_finding(
                    shader,
                    "MultiLayer Parallax requires Multi_Layer_Parallax",
                    "Shader Flags 2",
                    findings,
                );
            }
            if flags1 & SLSF1_PARALLAX != 0 {
                shader_rule_finding(
                    shader,
                    "Parallax cannot be used with MultiLayer Parallax shader type",
                    "Shader Flags 1",
                    findings,
                );
            }
            require_texture_slot(
                texture_set,
                textures,
                4,
                "Environment",
                shader_type,
                findings,
            );
            require_texture_slot(
                texture_set,
                textures,
                5,
                "Environment mask",
                shader_type,
                findings,
            );
            require_texture_slot(
                texture_set,
                textures,
                6,
                "Inner layer",
                shader_type,
                findings,
            );
        }
        SHADER_EYE_ENVMAP => {
            if flags1 & SLSF1_EYE_ENVIRONMENT_MAPPING == 0 {
                shader_rule_finding(
                    shader,
                    "Eye Envmap requires Eye_Environment_Mapping",
                    "Shader Flags 1",
                    findings,
                );
            }
            require_texture_slot(
                texture_set,
                textures,
                4,
                "Environment",
                shader_type,
                findings,
            );
            require_texture_slot(
                texture_set,
                textures,
                5,
                "Environment mask",
                shader_type,
                findings,
            );
        }
        _ => {}
    }
    for (flag_is_set, expected_type, label, field) in [
        (
            flags1 & SLSF1_ENVIRONMENT_MAPPING != 0,
            SHADER_ENVIRONMENT_MAP,
            "Environment_Mapping",
            "Shader Flags 1",
        ),
        (
            flags2 & SLSF2_GLOW_MAP != 0,
            SHADER_GLOW,
            "Glow_Map",
            "Shader Flags 2",
        ),
        (
            flags1 & SLSF1_PARALLAX != 0,
            SHADER_PARALLAX,
            "Parallax",
            "Shader Flags 1",
        ),
        (
            flags1 & SLSF1_FACEGEN != 0,
            SHADER_FACEGEN,
            "Facegen",
            "Shader Flags 1",
        ),
        (
            flags1 & SLSF1_SKIN_TINT != 0,
            SHADER_SKIN_TINT,
            "Skin_Tint",
            "Shader Flags 1",
        ),
        (
            flags1 & SLSF1_HAIR_TINT != 0,
            SHADER_HAIR_TINT,
            "Hair_Tint",
            "Shader Flags 1",
        ),
        (
            flags2 & SLSF2_MULTI_LAYER_PARALLAX != 0,
            SHADER_MULTI_LAYER_PARALLAX,
            "Multi_Layer_Parallax",
            "Shader Flags 2",
        ),
        (
            flags1 & SLSF1_EYE_ENVIRONMENT_MAPPING != 0,
            SHADER_EYE_ENVMAP,
            "Eye_Environment_Mapping",
            "Shader Flags 1",
        ),
    ] {
        if flag_is_set && shader_type != expected_type {
            shader_rule_finding(
                shader,
                &format!("{label} is set but shader type is {shader_type}"),
                field,
                findings,
            );
        }
    }
    if matches!(
        shader_type,
        SHADER_ENVIRONMENT_MAP | SHADER_MULTI_LAYER_PARALLAX | SHADER_EYE_ENVMAP
    ) && flags2 & SLSF2_ENV_MAP_LIGHT_FADE == 0
    {
        shader_rule_finding(
            shader,
            "Environment shader types require EnvMap_Light_Fade",
            "Shader Flags 2",
            findings,
        );
    }
    if flags1 & SLSF1_SPECULAR != 0 && color_is_black(shader.get_field("Specular Color")) {
        shader_rule_finding(
            shader,
            "Specular is set but Specular Color is black",
            "Specular Color",
            findings,
        );
    }
    if flags2 & SLSF2_BACK_LIGHTING != 0 && texture_slot(textures, 7).is_empty() {
        require_texture_slot(
            texture_set,
            textures,
            7,
            "Back lighting",
            shader_type,
            findings,
        );
    }
    if flags2 & SLSF2_RIM_LIGHTING != 0 && texture_slot(textures, 2).is_empty() {
        require_texture_slot(
            texture_set,
            textures,
            2,
            "Rim lighting",
            shader_type,
            findings,
        );
    }
    if flags2 & SLSF2_SOFT_LIGHTING != 0
        && !matches!(shader_type, SHADER_SKIN_TINT | SHADER_FACEGEN)
        && texture_slot(textures, 2).is_empty()
    {
        require_texture_slot(
            texture_set,
            textures,
            2,
            "Soft lighting",
            shader_type,
            findings,
        );
    }
    if flags2 & SLSF2_RIM_LIGHTING != 0 && flags2 & SLSF2_SOFT_LIGHTING != 0 {
        shader_rule_finding(
            shader,
            "Rim_Lighting and Soft_Lighting cannot be used together",
            "Shader Flags 2",
            findings,
        );
    }
    if flags1 & (SLSF1_SPECULAR | SLSF1_MODEL_SPACE_NORMALS)
        == (SLSF1_SPECULAR | SLSF1_MODEL_SPACE_NORMALS)
    {
        if texture_slot(textures, 7).is_empty() {
            require_texture_slot(
                texture_set,
                textures,
                7,
                "Model-space specular",
                shader_type,
                findings,
            );
        }
        if flags2 & SLSF2_BACK_LIGHTING != 0 {
            shader_rule_finding(
                shader,
                "Back_Lighting cannot be used with Model_Space_Normals and Specular",
                "Shader Flags 2",
                findings,
            );
        }
    }
    let facegen = nif.blocks.iter().any(|block| {
        SCHEMA.is_subtype_of(&block.type_name, "NiNode")
            && string_field(block, "Name").as_deref() == Some("BSFaceGenNiNodeSkinned")
    });
    if facegen != (flags2 & SLSF2_CHARACTER_LIGHTING != 0) {
        shader_rule_finding(
            shader,
            "Character_Lighting must match whether the NIF is facegen",
            "Shader Flags 2",
            findings,
        );
    }
    if flags2 & SLSF2_TREE_ANIM != 0 && flags2 & SLSF2_GLOW_MAP != 0 {
        shader_rule_finding(
            shader,
            "Tree_Anim and Glow_Map together can crash the Creation Kit",
            "Shader Flags 2",
            findings,
        );
    }
    if flags2 & SLSF2_TREE_ANIM != 0 {
        let root_type = root_ids(nif)
            .into_iter()
            .next()
            .and_then(|root| nif.get_block(root))
            .map(|root| root.type_name.as_str())
            .unwrap_or_default();
        if !matches!(root_type, "BSLeafAnimNode" | "BSTreeNode") {
            shader_rule_finding(
                shader,
                "Tree_Anim requires a BSLeafAnimNode or BSTreeNode root",
                "Shader Flags 2",
                findings,
            );
        }
        if flags2 & SLSF2_VERTEX_COLORS == 0 {
            shader_rule_finding(
                shader,
                "Tree_Anim requires Vertex_Colors",
                "Shader Flags 2",
                findings,
            );
        }
        if flags1 & SLSF1_VERTEX_ALPHA == 0 {
            shader_rule_finding(
                shader,
                "Tree_Anim requires Vertex_Alpha",
                "Shader Flags 1",
                findings,
            );
        }
    }
    if value_f64(shader.get_field("Glossiness")) == Some(0.0) {
        shader_rule_finding(
            shader,
            "Zero Glossiness causes lighting issues",
            "Glossiness",
            findings,
        );
    }
}

fn require_texture_slot(
    texture_set: &NifBlock,
    textures: &[NifValue],
    slot: usize,
    label: &str,
    shader_type: u64,
    findings: &mut Vec<ValidationFinding>,
) {
    if texture_slot(textures, slot).is_empty() {
        findings.push(finding(
            "error",
            "shader-texture-slot",
            texture_set,
            Some(format!("Textures[{slot}]")),
            format!("{label} texture slot {slot} is required by shader type {shader_type}"),
        ));
    }
}

fn shader_rule_finding(
    shader: &NifBlock,
    message: &str,
    field: &str,
    findings: &mut Vec<ValidationFinding>,
) {
    findings.push(finding(
        "error",
        "shader-type-flags",
        shader,
        Some(field.to_string()),
        message.to_string(),
    ));
}

fn audit_bsx_flags(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    if !nif_game(nif).uses_bsx() {
        return;
    }
    let block = nif
        .blocks
        .iter()
        .find(|block| block.type_name == "BSXFlags");
    let actual = block
        .and_then(|block| value_u64(block.get_field("Integer Data")))
        .unwrap_or(0);
    let expected = detected_bsx_flags(nif, actual);
    let mut checked_mask =
        BSX_ANIMATED | BSX_HAVOK | BSX_RAGDOLL | BSX_ADDON | BSX_EDITOR_MARKER | BSX_EXTERNAL_EMIT;
    if !nif_game(nif).is_fo4_family() {
        checked_mask |= BSX_COMPLEX | BSX_DYNAMIC;
    }
    if actual & checked_mask != expected & checked_mask || (block.is_some() && expected == 0) {
        let message = format!(
            "Flags are {actual}; detected structure requires {} for checked bits {checked_mask:#x}",
            expected & checked_mask
        );
        if let Some(block) = block {
            findings.push(finding(
                "error",
                "bsx-flags",
                block,
                Some("Integer Data".to_string()),
                message,
            ));
        } else {
            findings.push(ValidationFinding {
                severity: "error".to_string(),
                rule: "bsx-flags".to_string(),
                block_id: None,
                block_type: None,
                field: None,
                message,
            });
        }
    }
}

fn audit_miscellaneous(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    let roots = root_ids(nif);
    if let Some(root_id) = roots.first().copied()
        && let Some(root) = nif.get_block(root_id)
    {
        if root_id != 0 {
            findings.push(finding(
                "error",
                "root-index",
                root,
                None,
                "Root node must be block 0".to_string(),
            ));
        }
        if !SCHEMA.is_subtype_of(&root.type_name, "NiAVObject")
            && !SCHEMA.is_subtype_of(&root.type_name, "NiSequence")
            && root.type_name != "NiSequenceStreamHelper"
        {
            findings.push(finding(
                "error",
                "root-type",
                root,
                None,
                "Root must be a NiAVObject or NiSequence descendant".to_string(),
            ));
        }
    }

    if nif_game(nif) == NifGame::Oblivion {
        for shape in nif
            .blocks
            .iter()
            .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "NiTriBasedGeom"))
        {
            let Some(tangents) = ref_values(shape.get_field("Extra Data List"))
                .into_iter()
                .filter_map(|id| nif.get_block(id as usize))
                .find(|extra| {
                    extra.type_name == "NiBinaryExtraData"
                        && string_field(extra, "Name").as_deref()
                            == Some(OBLIVION_TANGENT_DATA_NAME)
                })
            else {
                continue;
            };
            let Some(data) = value_ref(shape.get_field("Data"))
                .filter(|id| *id >= 0)
                .and_then(|id| nif.get_block(id as usize))
            else {
                continue;
            };
            let byte_count = nested_value(tangents.get_field("Binary Data"), "Data")
                .and_then(nif_byte_count)
                .unwrap_or(0);
            let tangent_vertices = byte_count / 24;
            let vertex_count = value_u64(data.get_field("Num Vertices")).unwrap_or(0) as usize;
            if tangent_vertices != 0 && tangent_vertices != vertex_count {
                findings.push(finding(
                    "error",
                    "oblivion-tangent-count",
                    tangents,
                    Some("Binary Data".to_string()),
                    format!(
                        "Tangents and binormals contain {tangent_vertices} vertices; geometry has {vertex_count}"
                    ),
                ));
            }
        }
    }

    for block in &nif.blocks {
        if block.type_name == "NiSpecularProperty"
            && matches!(
                nif_game(nif),
                NifGame::Oblivion
                    | NifGame::Fo3
                    | NifGame::Skyrim
                    | NifGame::SkyrimSe
                    | NifGame::Fo4
                    | NifGame::Fo76
                    | NifGame::Starfield
            )
        {
            findings.push(finding(
                "warning",
                "unsupported-specular-property",
                block,
                None,
                "NiSpecularProperty is unsupported and does nothing".to_string(),
            ));
        }
        if block.type_name == "bhkListShape"
            && matches!(
                nif_game(nif),
                NifGame::Skyrim
                    | NifGame::SkyrimSe
                    | NifGame::Fo4
                    | NifGame::Fo76
                    | NifGame::Starfield
            )
        {
            for child_id in ref_values(block.get_field("Sub Shapes")) {
                if let Some(child) = nif.get_block(child_id as usize)
                    && !SCHEMA.is_subtype_of(&child.type_name, "bhkConvexShape")
                {
                    findings.push(finding(
                        "error",
                        "collision-list-shape",
                        block,
                        Some("Sub Shapes".to_string()),
                        format!(
                            "Invalid child container shape {} {}",
                            child.block_id, child.type_name
                        ),
                    ));
                }
            }
        }
        if matches!(
            block.type_name.as_str(),
            "BSEffectShaderProperty" | "BSLightingShaderProperty"
        ) && let Some(controller) = value_ref(block.get_field("Controller"))
            .filter(|controller| *controller >= 0)
            .and_then(|controller| nif.get_block(controller as usize))
        {
            let wrong = (block.type_name == "BSEffectShaderProperty"
                && controller.type_name.starts_with("BSLighting"))
                || (block.type_name == "BSLightingShaderProperty"
                    && controller.type_name.starts_with("BSEffect"));
            if wrong {
                findings.push(finding(
                    "error",
                    "shader-controller-type",
                    block,
                    Some("Controller".to_string()),
                    format!(
                        "Controller type {} is invalid for {}",
                        controller.type_name, block.type_name
                    ),
                ));
            }
        }
    }

    if matches!(nif_game(nif), NifGame::SkyrimSe) {
        let facegen = nif.blocks.iter().any(|block| {
            SCHEMA.is_subtype_of(&block.type_name, "NiNode")
                && string_field(block, "Name").as_deref() == Some("BSFaceGenNiNodeSkinned")
        });
        for block in &nif.blocks {
            if facegen && block.type_name == "BSTriShape" {
                findings.push(finding(
                    "error",
                    "sse-facegen-shape",
                    block,
                    None,
                    "Facegen shapes must be BSDynamicTriShape".to_string(),
                ));
            }
        }
    }

    if !matches!(
        nif_game(nif),
        NifGame::Fo3
            | NifGame::Skyrim
            | NifGame::SkyrimSe
            | NifGame::Fo4
            | NifGame::Fo76
            | NifGame::Starfield
    ) {
        return;
    }
    for shape in &nif.blocks {
        if !SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape")
            && !SCHEMA.is_subtype_of(&shape.type_name, "NiGeometry")
        {
            continue;
        }
        if geometry_shader_id(nif, shape).is_some() || block_is_editor_marker(shape) {
            continue;
        }
        if SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape")
            && value_u64(shape.get_field("Vertex Desc")).is_some_and(|descriptor| {
                let attributes = (descriptor >> 44) & 0xFFF;
                attributes & 0x10 == 0
            })
        {
            continue;
        }
        let used_as_emitter = nif.blocks.iter().any(|block| {
            SCHEMA.is_subtype_of(&block.type_name, "NiPSysEmitter")
                && block.get_refs(&SCHEMA).contains(&(shape.block_id as i32))
        });
        if !used_as_emitter {
            findings.push(finding(
                "error",
                "missing-shader-property",
                shape,
                None,
                "Rendered geometry is missing a shader property".to_string(),
            ));
        }
    }
}

fn audit_optional_nif(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    audit_alpha_properties(nif, findings);
    audit_optional_shader_rules(nif, findings);
    audit_clamped_uvs(nif, findings);
    audit_repeated_degenerate_strips(nif, findings);
    audit_sse_unsupported_formats(nif, findings);
}

fn audit_alpha_properties(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    if matches!(
        nif_game(nif),
        NifGame::Fo4 | NifGame::Fo76 | NifGame::Starfield
    ) {
        return;
    }
    for shape in &nif.blocks {
        if !SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape")
            && !SCHEMA.is_subtype_of(&shape.type_name, "NiGeometry")
        {
            continue;
        }
        let Some(alpha) = geometry_property_by_type(nif, shape, "NiAlphaProperty") else {
            continue;
        };
        if geometry_property_by_type(nif, shape, "BSShaderNoLightingProperty").is_some()
            || value_u64(alpha.get_field("Flags")).unwrap_or(0) & 1 == 0
        {
            continue;
        }
        let Some(shader) = geometry_shader_id(nif, shape).and_then(|id| nif.get_block(id)) else {
            continue;
        };
        if value_u64(shader.get_field("Shader Flags 2"))
            .is_some_and(|flags| flags & SLSF2_ASSUME_SHADOWMASK != 0)
        {
            continue;
        }
        findings.push(finding(
            "warning",
            "alpha-property-single-pass",
            alpha,
            Some("Flags".to_string()),
            "Blend alpha forces single-pass rendering and can cause lighting issues with multiple lights"
                .to_string(),
        ));
    }
}

fn audit_optional_shader_rules(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    for shader in nif
        .blocks
        .iter()
        .filter(|block| SCHEMA.is_subtype_of(&block.type_name, "BSShaderProperty"))
    {
        let flags1 = shader_flags1(shader);
        if flags1 == 0 {
            findings.push(finding(
                "warning",
                "optional-shader-flags",
                shader,
                Some("Shader Flags 1".to_string()),
                "Empty shader flags".to_string(),
            ));
        }
        if shader.type_name != "BSShaderPPLightingProperty" {
            continue;
        }
        let flags2 = value_u64(shader.get_field("Shader Flags 2")).unwrap_or(0);
        if flags1 & SLSF1_ENVIRONMENT_MAPPING != 0 && flags2 & SLSF2_ENV_MAP_LIGHT_FADE == 0 {
            findings.push(finding(
                "warning",
                "optional-shader-flags",
                shader,
                Some("Shader Flags 2".to_string()),
                "Environment_Mapping is set without Envmap_Light_Fade".to_string(),
            ));
        }
        let Some(texture_set) = value_ref(shader.get_field("Texture Set"))
            .filter(|id| *id >= 0)
            .and_then(|id| nif.get_block(id as usize))
        else {
            continue;
        };
        let textures = value_array(texture_set.get_field("Textures"));
        if textures.len() > 5
            && (texture_slot(textures, 4).is_empty() != texture_slot(textures, 5).is_empty())
        {
            findings.push(finding(
                "warning",
                "optional-shader-flags",
                texture_set,
                Some("Textures".to_string()),
                "FO3/FNV BSShaderPPLightingProperty must use texture slots 5 and 6 together"
                    .to_string(),
            ));
        }
    }
}

fn audit_clamped_uvs(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    for shape in &nif.blocks {
        if !SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape")
            && !SCHEMA.is_subtype_of(&shape.type_name, "NiTriBasedGeom")
        {
            continue;
        }
        let Some((property, clamp_mode)) = geometry_clamp_mode(nif, shape) else {
            continue;
        };
        if clamp_mode == 3 {
            continue;
        }
        let geometry = if SCHEMA.is_subtype_of(&shape.type_name, "BSTriShape") {
            shape
        } else {
            let Some(data) = value_ref(shape.get_field("Data"))
                .filter(|id| *id >= 0)
                .and_then(|id| nif.get_block(id as usize))
            else {
                continue;
            };
            data
        };
        let mut uvs = Vec::new();
        collect_uvs(&geometry.fields, &mut uvs);
        if uvs.iter().any(|[u, v]| {
            ((clamp_mode == 0 || clamp_mode == 1) && !(-0.001..=1.001).contains(u))
                || ((clamp_mode == 0 || clamp_mode == 2) && !(-0.001..=1.001).contains(v))
        }) {
            findings.push(finding(
                "warning",
                "clamped-tiling-uvs",
                geometry,
                None,
                format!(
                    "UVs outside 0..1 use clamp mode {clamp_mode} in block {} {}",
                    property.block_id, property.type_name
                ),
            ));
        }
    }
}

fn audit_repeated_degenerate_strips(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    for block in &nif.blocks {
        let repeated = if block.type_name == "NiTriStripsData" {
            value_array(block.get_field("Strips"))
                .iter()
                .any(strip_has_repeated_degenerates)
        } else if block.type_name == "NiSkinPartition" {
            value_array(block.get_field("Partitions"))
                .iter()
                .any(|partition| {
                    nested_value(Some(partition), "Strips")
                        .map(value_array_from_value)
                        .is_some_and(|strips| strips.iter().any(strip_has_repeated_degenerates))
                })
        } else {
            false
        };
        if repeated {
            findings.push(finding(
                "warning",
                "repeated-degenerate-strips",
                block,
                Some("Strips".to_string()),
                "Repeated degenerate triangles in strip".to_string(),
            ));
        }
    }
}

fn audit_sse_unsupported_formats(nif: &NifFile, findings: &mut Vec<ValidationFinding>) {
    if !matches!(nif_game(nif), NifGame::SkyrimSe) {
        return;
    }
    for block in &nif.blocks {
        let unsupported = matches!(
            block.type_name.as_str(),
            "NiTriStrips" | "bhkMultiSphereShape"
        ) || (block.type_name == "NiSkinPartition"
            && value_array(block.get_field("Partitions"))
                .iter()
                .any(|partition| nested_u64(Some(partition), "Num Strips").unwrap_or(0) != 0));
        if unsupported {
            findings.push(finding(
                "error",
                "sse-unsupported-block",
                block,
                None,
                format!("{} can crash Skyrim SE", block.type_name),
            ));
        }
    }
}

fn geometry_property_by_type<'a>(
    nif: &'a NifFile,
    shape: &NifBlock,
    property_type: &str,
) -> Option<&'a NifBlock> {
    let direct_field = match property_type {
        "NiAlphaProperty" => Some("Alpha Property"),
        _ if SCHEMA.is_subtype_of(property_type, "BSShaderProperty") => Some("Shader Property"),
        _ => None,
    };
    direct_field
        .and_then(|field| value_ref(shape.get_field(field)))
        .filter(|id| *id >= 0)
        .and_then(|id| nif.get_block(id as usize))
        .filter(|property| SCHEMA.is_subtype_of(&property.type_name, property_type))
        .or_else(|| {
            ref_values(shape.get_field("Properties"))
                .into_iter()
                .filter_map(|id| nif.get_block(id as usize))
                .find(|property| SCHEMA.is_subtype_of(&property.type_name, property_type))
        })
}

fn geometry_clamp_mode<'a>(nif: &'a NifFile, shape: &NifBlock) -> Option<(&'a NifBlock, u64)> {
    if let Some(shader) = geometry_shader_id(nif, shape).and_then(|id| nif.get_block(id)) {
        if shader.type_name == "BSEffectShaderProperty"
            || (nif_game(nif).is_fo4_family()
                && string_field(shader, "Name").is_some_and(|name| !name.is_empty()))
        {
            return None;
        }
        return value_u64(shader.get_field("Texture Clamp Mode")).map(|mode| (shader, mode));
    }
    let property = geometry_property_by_type(nif, shape, "NiTexturingProperty")?;
    let mode = nested_u64(property.get_field("Base Texture"), "Clamp Mode")?;
    Some((property, mode))
}

fn collect_uvs(fields: &IndexMap<String, NifValue>, out: &mut Vec<[f64; 2]>) {
    for value in fields.values() {
        collect_uv_value(value, out);
    }
}

fn collect_uv_value(value: &NifValue, out: &mut Vec<[f64; 2]>) {
    match value {
        NifValue::Struct(fields) => {
            let u = fields
                .iter()
                .find(|(name, _)| bare_name(name).eq_ignore_ascii_case("u"))
                .and_then(|(_, value)| value_f64(Some(value)));
            let v = fields
                .iter()
                .find(|(name, _)| bare_name(name).eq_ignore_ascii_case("v"))
                .and_then(|(_, value)| value_f64(Some(value)));
            if let (Some(u), Some(v)) = (u, v) {
                out.push([u, v]);
            } else {
                collect_uvs(fields, out);
            }
        }
        NifValue::Array(values) => {
            for value in values {
                collect_uv_value(value, out);
            }
        }
        _ => {}
    }
}

fn value_array_from_value(value: &NifValue) -> &[NifValue] {
    match value {
        NifValue::Array(values) => values,
        _ => &[],
    }
}

fn strip_has_repeated_degenerates(strip: &NifValue) -> bool {
    let values = value_array_from_value(strip);
    let mut repeated = 0;
    for pair in values.windows(2) {
        if value_u64(pair.first()) == value_u64(pair.get(1)) {
            repeated += 1;
            if repeated == 3 {
                return true;
            }
        } else {
            repeated = 0;
        }
    }
    false
}

fn block_is_editor_marker(block: &NifBlock) -> bool {
    string_field(block, "Name")
        .is_some_and(|name| name.to_ascii_lowercase().contains("editormarker"))
}

fn nested_value<'a>(value: Option<&'a NifValue>, name: &str) -> Option<&'a NifValue> {
    let NifValue::Struct(fields) = value? else {
        return None;
    };
    fields.get(name).or_else(|| {
        fields
            .iter()
            .find(|(field, _)| bare_name(field) == name)
            .map(|(_, value)| value)
    })
}

fn nested_u64(value: Option<&NifValue>, name: &str) -> Option<u64> {
    value_u64(nested_value(value, name))
}

fn rigid_body_value<'a>(block: &'a NifBlock, name: &str) -> Option<&'a NifValue> {
    block
        .get_field(name)
        .or_else(|| nested_value(block.get_field("Rigid Body Info"), name))
}

fn rigid_body_nested_u64(block: &NifBlock, group: &str, name: &str) -> Option<u64> {
    nested_u64(block.get_field(group), name).or_else(|| {
        nested_value(block.get_field("Rigid Body Info"), group)
            .and_then(|value| nested_u64(Some(value), name))
    })
}

fn set_struct_field_if_present(value: &mut NifValue, name: &str, replacement: NifValue) -> bool {
    let NifValue::Struct(fields) = value else {
        return false;
    };
    let Some(key) = fields.keys().find(|key| bare_name(key) == name).cloned() else {
        return false;
    };
    if fields.get(&key) == Some(&replacement) {
        return false;
    }
    fields.insert(key, replacement);
    true
}

fn set_struct_field(value: &mut NifValue, name: &str, replacement: NifValue) -> bool {
    let NifValue::Struct(fields) = value else {
        return false;
    };
    let key = fields
        .keys()
        .find(|key| bare_name(key) == name)
        .cloned()
        .unwrap_or_else(|| name.to_string());
    if fields.get(&key) == Some(&replacement) {
        return false;
    }
    fields.insert(key, replacement);
    true
}

fn rigid_body_info_mut(block: &mut NifBlock) -> Option<&mut NifValue> {
    let key = block
        .fields
        .keys()
        .find(|key| bare_name(key) == "Rigid Body Info")
        .cloned()?;
    block.fields.get_mut(&key)
}

fn set_rigid_body_field_if_present(
    block: &mut NifBlock,
    name: &str,
    replacement: NifValue,
) -> bool {
    if block.get_field(name).is_some() {
        if block.get_field(name) == Some(&replacement) {
            return false;
        }
        block.set_field(name, replacement);
        return true;
    }
    rigid_body_info_mut(block)
        .is_some_and(|info| set_struct_field_if_present(info, name, replacement))
}

fn set_rigid_body_u64_if_present(block: &mut NifBlock, name: &str, value: u64) -> bool {
    set_rigid_body_field_if_present(block, name, NifValue::UInt(value))
}

fn set_rigid_body_bool_if_present(block: &mut NifBlock, name: &str, value: bool) -> bool {
    set_rigid_body_field_if_present(block, name, NifValue::Bool(value))
}

fn set_rigid_body_f64_if_present(block: &mut NifBlock, name: &str, value: f64) -> bool {
    set_rigid_body_field_if_present(block, name, NifValue::Float(value))
}

fn set_rigid_body_nested_u64_if_present(
    block: &mut NifBlock,
    group: &str,
    name: &str,
    value: u64,
) -> bool {
    let replacement = NifValue::UInt(value);
    let direct_key = block
        .fields
        .keys()
        .find(|key| bare_name(key) == group)
        .cloned();
    if let Some(key) = direct_key {
        return block
            .fields
            .get_mut(&key)
            .is_some_and(|group| set_struct_field_if_present(group, name, replacement));
    }
    let Some(NifValue::Struct(info)) = rigid_body_info_mut(block) else {
        return false;
    };
    let Some(key) = info.keys().find(|key| bare_name(key) == group).cloned() else {
        return false;
    };
    info.get_mut(&key)
        .is_some_and(|group| set_struct_field_if_present(group, name, replacement))
}

fn set_inertia_diagonal_if_present(block: &mut NifBlock, value: f32) -> bool {
    let Some(inertia) = rigid_body_value(block, "Inertia Tensor").cloned() else {
        return false;
    };
    let replacement = match inertia {
        NifValue::Matrix33(mut matrix) => {
            matrix[0][0] = value;
            matrix[1][1] = value;
            matrix[2][2] = value;
            NifValue::Matrix33(matrix)
        }
        NifValue::Struct(mut fields) => {
            let mut found = false;
            for (name, item) in [("m11", value), ("m22", value), ("m33", value)] {
                if let Some(key) = fields.keys().find(|key| bare_name(key) == name).cloned() {
                    fields.insert(key, NifValue::Float(item.into()));
                    found = true;
                }
            }
            if !found {
                return false;
            }
            NifValue::Struct(fields)
        }
        _ => return false,
    };
    set_rigid_body_field_if_present(block, "Inertia Tensor", replacement)
}

fn texture_slot(textures: &[NifValue], slot: usize) -> &str {
    textures
        .get(slot)
        .and_then(value_string)
        .unwrap_or_default()
}

fn byte_color(value: Option<&NifValue>) -> Option<[f64; 4]> {
    let NifValue::Struct(fields) = value? else {
        return None;
    };
    Some([
        value_u64(fields.get("r"))? as f64 / 255.0,
        value_u64(fields.get("g"))? as f64 / 255.0,
        value_u64(fields.get("b"))? as f64 / 255.0,
        value_u64(fields.get("a"))? as f64 / 255.0,
    ])
}

fn float_color(value: &NifValue) -> Option<[f64; 4]> {
    match value {
        NifValue::Color4(color) | NifValue::Vec4(color) => Some([
            color[0] as f64,
            color[1] as f64,
            color[2] as f64,
            color[3] as f64,
        ]),
        NifValue::Struct(fields) => Some([
            value_f64(fields.get("r"))?,
            value_f64(fields.get("g"))?,
            value_f64(fields.get("b"))?,
            value_f64(fields.get("a"))?,
        ]),
        _ => None,
    }
}

fn color_is_black(value: Option<&NifValue>) -> bool {
    match value {
        Some(NifValue::Color3(color)) | Some(NifValue::Vec3(color)) => {
            color.iter().all(|component| *component == 0.0)
        }
        Some(NifValue::Struct(fields)) => ["r", "g", "b"]
            .iter()
            .all(|field| value_f64(fields.get(*field)) == Some(0.0)),
        _ => false,
    }
}

fn inertia_tensor_is_bad(value: Option<&NifValue>) -> bool {
    let diagonal = match value {
        Some(NifValue::Matrix33(matrix)) => [
            Some(matrix[0][0] as f64),
            Some(matrix[1][1] as f64),
            Some(matrix[2][2] as f64),
        ],
        Some(NifValue::Struct(fields)) => [
            value_f64(fields.get("m11")),
            value_f64(fields.get("m22")),
            value_f64(fields.get("m33")),
        ],
        _ => return false,
    };
    diagonal
        .into_iter()
        .flatten()
        .any(|component| !component.is_finite() || component <= 0.0)
}

fn finding(
    severity: &str,
    rule: &str,
    block: &NifBlock,
    field: Option<String>,
    message: String,
) -> ValidationFinding {
    ValidationFinding {
        severity: severity.to_string(),
        rule: rule.to_string(),
        block_id: Some(block.block_id),
        block_type: Some(block.type_name.clone()),
        field,
        message,
    }
}

fn is_direct_ref_field(field: &FieldDef) -> bool {
    matches!(field.type_name, "Ref" | "Ptr")
}

fn simple_length_field(length: Option<&str>) -> Option<&str> {
    let length = length?;
    length
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == ' ')
        .then_some(length)
}

fn bare_name(key: &str) -> &str {
    key.split_once(':').map(|(bare, _)| bare).unwrap_or(key)
}

fn string_field(block: &NifBlock, name: &str) -> Option<String> {
    block
        .get_field(name)
        .and_then(value_string)
        .map(str::to_string)
}

fn shader_flags1(block: &NifBlock) -> u64 {
    value_u64(
        block
            .get_field("Shader Flags 1")
            .or_else(|| block.get_field("Shader Flags")),
    )
    .unwrap_or(0)
}

fn skin_ref(block: &NifBlock) -> Option<i32> {
    ["Skin", "Skin Instance"]
        .into_iter()
        .filter_map(|field| value_ref(block.get_field(field)))
        .find(|reference| *reference >= 0)
}

fn set_shader_flags1(block: &mut NifBlock, value: u64) {
    let field = if block.get_field("Shader Flags 1").is_some() {
        "Shader Flags 1"
    } else {
        "Shader Flags"
    };
    block.set_field(field, NifValue::UInt(value));
}

fn value_string(value: &NifValue) -> Option<&str> {
    match value {
        NifValue::String(value) | NifValue::Char(value) => Some(value),
        _ => None,
    }
}

fn value_i64(value: Option<&NifValue>) -> Option<i64> {
    match value? {
        NifValue::Int(value) => Some(*value),
        NifValue::UInt(value) => Some(*value as i64),
        NifValue::Ref(value) => Some(*value as i64),
        _ => None,
    }
}

fn value_u64(value: Option<&NifValue>) -> Option<u64> {
    value_i64(value).and_then(|value| (value >= 0).then_some(value as u64))
}

fn value_f64(value: Option<&NifValue>) -> Option<f64> {
    match value? {
        NifValue::Float(value) => Some(*value),
        NifValue::Int(value) => Some(*value as f64),
        NifValue::UInt(value) => Some(*value as f64),
        _ => None,
    }
}

fn value_bool(value: Option<&NifValue>) -> Option<bool> {
    match value? {
        NifValue::Bool(value) => Some(*value),
        NifValue::Int(value) => Some(*value != 0),
        NifValue::UInt(value) => Some(*value != 0),
        _ => None,
    }
}

fn value_ref(value: Option<&NifValue>) -> Option<i32> {
    match value? {
        NifValue::Ref(value) => Some(*value),
        NifValue::Int(value) => i32::try_from(*value).ok(),
        NifValue::UInt(value) => i32::try_from(*value).ok(),
        _ => None,
    }
}

fn ref_values(value: Option<&NifValue>) -> Vec<i32> {
    match value {
        Some(NifValue::Array(values)) => values
            .iter()
            .filter_map(|value| value_ref(Some(value)))
            .filter(|value| *value >= 0)
            .collect(),
        _ => Vec::new(),
    }
}

fn ref_scalars(value: &NifValue) -> Vec<i32> {
    match value {
        NifValue::Array(values) => values
            .iter()
            .filter_map(|value| value_ref(Some(value)))
            .collect(),
        _ => value_ref(Some(value)).into_iter().collect(),
    }
}

fn value_array(value: Option<&NifValue>) -> &[NifValue] {
    match value {
        Some(NifValue::Array(values)) => values,
        _ => &[],
    }
}

fn nif_byte_count(value: &NifValue) -> Option<usize> {
    match value {
        NifValue::Bytes(values) => Some(values.len()),
        NifValue::Array(values) => Some(values.len()),
        _ => None,
    }
}

fn numeric_scalars(value: &NifValue) -> Vec<usize> {
    match value {
        NifValue::UInt(value) => vec![*value as usize],
        NifValue::Int(value) if *value >= 0 => vec![*value as usize],
        NifValue::Array(values) => values.iter().flat_map(numeric_scalars).collect(),
        NifValue::Struct(fields) => fields.values().flat_map(numeric_scalars).collect(),
        _ => Vec::new(),
    }
}

fn fields_contain_named_zero(fields: &IndexMap<String, NifValue>, name: &str) -> bool {
    fields.iter().any(|(field, value)| {
        (bare_name(field) == name && value_f64(Some(value)) == Some(0.0))
            || match value {
                NifValue::Struct(fields) => fields_contain_named_zero(fields, name),
                NifValue::Array(values) => values.iter().any(|value| match value {
                    NifValue::Struct(fields) => fields_contain_named_zero(fields, name),
                    _ => false,
                }),
                _ => false,
            }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(
        values: impl IntoIterator<Item = (&'static str, NifValue)>,
    ) -> IndexMap<String, NifValue> {
        values
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    #[test]
    fn audit_reports_preserved_invalid_string_indices() {
        let mut nif = NifFile::new("fo4");
        nif.blocks[0].set_field("Name", NifValue::Int(99));
        let findings = audit_nif(&nif);
        let finding = findings
            .iter()
            .find(|finding| finding.rule == "invalid-string-index")
            .unwrap();
        assert_eq!(finding.block_id, Some(0));
        assert_eq!(finding.field.as_deref(), Some("Name"));
        assert_eq!(
            crate::processor_catalog::check_id_for_finding_rule(&finding.rule),
            "invalid-string-index"
        );

        let report = sanitize_nif(&mut nif);
        assert_eq!(nif.blocks[0].get_field("Name"), Some(&NifValue::Null));
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("invalid string-table"))
        );
    }

    #[test]
    fn sanitizer_reconciles_vertex_colors_and_skeleton_bsx_flags() {
        let mut nif = NifFile::new("fo4");
        let shader = nif.add_block(
            "BSEffectShaderProperty",
            Some(fields([
                (
                    "Name",
                    NifValue::String("Materials\\Effects\\Test.bgem".to_string()),
                ),
                ("Shader Flags 1", NifValue::UInt(0)),
                ("Shader Flags 2", NifValue::UInt(0)),
                (
                    "Source Texture",
                    NifValue::String("Effects/Test.dds".to_string()),
                ),
            ])),
        );
        let mut vertex = IndexMap::new();
        vertex.insert(
            "Vertex Colors".to_string(),
            NifValue::Struct(IndexMap::from([
                ("r".to_string(), NifValue::UInt(255)),
                ("g".to_string(), NifValue::UInt(255)),
                ("b".to_string(), NifValue::UInt(255)),
                ("a".to_string(), NifValue::UInt(255)),
            ])),
        );
        let particle = nif.add_block(
            "NiParticleSystem",
            Some(fields([
                ("Vertex Desc", NifValue::UInt(VF_VERTEX_COLORS << 44)),
                ("Shader Property", NifValue::Ref(shader as i32)),
                (
                    "Vertex Data",
                    NifValue::Array(vec![NifValue::Struct(vertex)]),
                ),
            ])),
        );
        let bsx = nif.add_block(
            "BSXFlags",
            Some(fields([
                ("Name", NifValue::String("BSX".to_string())),
                ("Integer Data", NifValue::UInt(198)),
            ])),
        );
        let collision = nif.add_block("bhkNPCollisionObject", None);
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(particle as i32)]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
        nif.blocks[0].set_field(
            "Extra Data List",
            NifValue::Array(vec![NifValue::Ref(bsx as i32)]),
        );
        nif.blocks[0].set_field("Num Extra Data List", NifValue::UInt(1));
        nif.blocks[0].set_field("Collision Object", NifValue::Ref(collision as i32));

        let report = sanitize_fo4_nif(&mut nif);
        assert!(!report.changes.is_empty());
        let shader = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSEffectShaderProperty")
            .unwrap();
        assert_ne!(
            value_u64(shader.get_field("Shader Flags 2")).unwrap() & SLSF2_VERTEX_COLORS,
            0
        );
        assert_eq!(
            string_field(shader, "Source Texture").as_deref(),
            Some("textures/Effects/Test.dds")
        );
        let bsx = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSXFlags")
            .unwrap();
        assert_eq!(value_u64(bsx.get_field("Integer Data")), Some(194));
    }

    #[test]
    fn audit_preserves_fo4_bsx_bits_that_the_fixer_cannot_detect() {
        let mut nif = NifFile::new("fo4");
        nif.add_block(
            "BSXFlags",
            Some(fields([
                ("Name", NifValue::String("BSX".to_string())),
                (
                    "Integer Data",
                    NifValue::UInt(BSX_DYNAMIC | BSX_ARTICULATED),
                ),
            ])),
        );

        assert!(
            audit_nif(&nif)
                .iter()
                .all(|finding| finding.rule != "bsx-flags")
        );
    }

    #[test]
    fn universal_fixer_only_normalizes_registered_asset_fields() {
        let mut nif = NifFile::new("fo4");
        nif.blocks[0].set_field(
            "Name",
            NifValue::String("C:\\Nodes\\LooksLikeMesh.nif".to_string()),
        );
        let textures = nif.add_block(
            "BSShaderTextureSet",
            Some(fields([(
                "Textures",
                NifValue::Array(vec![NifValue::String(
                    "C:\\Game\\Data\\textures\\actors\\body.dds".to_string(),
                )]),
            )])),
        );
        normalize_asset_paths(&mut nif, &mut SanitizeReport::default());

        assert_eq!(
            string_field(&nif.blocks[0], "Name").as_deref(),
            Some("C:\\Nodes\\LooksLikeMesh.nif")
        );
        assert_eq!(
            value_array(nif.blocks[textures].get_field("Textures")).first(),
            Some(&NifValue::String("textures\\actors\\body.dds".to_string()))
        );
    }

    #[test]
    fn audit_reports_registered_asset_paths_that_the_fixer_would_normalize() {
        let mut nif = NifFile::new("fo4");
        nif.blocks[0].set_field(
            "Name",
            NifValue::String("C:\\Nodes\\LooksLikeMesh.nif".to_string()),
        );
        let shader = nif.add_block(
            "BSEffectShaderProperty",
            Some(fields([(
                "Source Texture",
                NifValue::String("Effects/Test.dds".to_string()),
            )])),
        );
        let behavior = nif.add_block(
            "BSBehaviorGraphExtraData",
            Some(fields([(
                "Behavior Graph File",
                NifValue::String("actors//test_behavior".to_string()),
            )])),
        );

        let findings = audit_nif(&nif);
        assert!(findings.iter().any(|finding| {
            finding.rule == "asset-path"
                && finding.block_id == Some(shader)
                && finding.field.as_deref() == Some("Source Texture")
                && finding.message.contains("textures/Effects/Test.dds")
        }));
        assert!(findings.iter().all(|finding| {
            finding.rule != "asset-path"
                || finding.block_id != Some(0)
                || finding.field.as_deref() != Some("Name")
        }));
        assert!(findings.iter().any(|finding| {
            finding.rule == "asset-path"
                && finding.block_id == Some(behavior)
                && finding.message.contains("actors/test_behavior")
        }));

        let mut fo76 = NifFile::new("fo76");
        let material = fo76.add_block(
            "BSLightingShaderProperty",
            Some(fields([(
                "Name",
                NifValue::String("materials//actors/test.bgsm".to_string()),
            )])),
        );
        assert!(audit_nif(&fo76).iter().any(|finding| {
            finding.rule == "asset-path"
                && finding.block_id == Some(material)
                && finding.message.contains("materials/actors/test.bgsm")
        }));
    }

    #[test]
    fn asset_path_rules_match_unfixable_and_tes3_reference_cases() {
        let mut fo4 = NifFile::new("fo4");
        let shader = fo4.add_block(
            "BSEffectShaderProperty",
            Some(fields([(
                "Source Texture",
                NifValue::String("C:\\Loose\\effect.dds".to_string()),
            )])),
        );
        let findings = audit_nif(&fo4);
        assert!(findings.iter().any(|finding| {
            finding.rule == "asset-path"
                && finding.block_id == Some(shader)
                && finding.message.contains("Invalid or absolute")
                && !finding.message.contains("textures\\C:")
        }));
        normalize_asset_paths(&mut fo4, &mut SanitizeReport::default());
        assert_eq!(
            string_field(&fo4.blocks[shader], "Source Texture").as_deref(),
            Some("C:\\Loose\\effect.dds")
        );

        let mut morrowind = NifFile::new("morrowind");
        let texture = morrowind.add_block(
            "NiSourceTexture",
            Some(fields([(
                "File Name",
                NifValue::String("effects/foo.dds".to_string()),
            )])),
        );
        assert!(
            audit_nif(&morrowind)
                .iter()
                .all(|finding| finding.rule != "asset-path")
        );
        normalize_asset_paths(&mut morrowind, &mut SanitizeReport::default());
        assert_eq!(
            string_field(&morrowind.blocks[texture], "File Name").as_deref(),
            Some("effects/foo.dds")
        );
    }

    #[test]
    fn sanitizer_removes_empty_legacy_shape_branch() {
        let mut nif = NifFile::new("fnv");
        let data_id = nif.add_block(
            "NiTriShapeData",
            Some(fields([("Num Vertices", NifValue::UInt(0))])),
        );
        let shape_id = nif.add_block(
            "NiTriShape",
            Some(fields([
                ("Data", NifValue::Ref(data_id as i32)),
                ("Extra Data List", NifValue::Array(Vec::new())),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );
        let mut report = SanitizeReport::default();

        remove_empty_shapes(&mut nif, &mut report);

        assert!(
            !nif.blocks
                .iter()
                .any(|block| block.type_name == "NiTriShape")
        );
        assert!(
            !nif.blocks
                .iter()
                .any(|block| block.type_name == "NiTriShapeData")
        );
        assert_eq!(report.changes.len(), 1);
    }

    #[test]
    fn sanitizer_normalizes_spaced_addon_names_and_ref_arrays() {
        let mut nif = NifFile::new("fo4");
        let addon = nif.add_block(
            "BSValueNode",
            Some(fields([
                ("Name", NifValue::String("AddOnNode 1078".to_string())),
                ("Value", NifValue::Int(1078)),
            ])),
        );
        let sequence = nif.add_block(
            "NiControllerSequence",
            Some(fields([(
                "Controlled Blocks",
                NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                    "Node Name".to_string(),
                    NifValue::String("AddOnNode 1078".to_string()),
                )]))]),
            )])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(addon as i32),
                NifValue::Ref(addon as i32),
                NifValue::Ref(-1),
            ]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(3));

        sanitize_fo4_nif(&mut nif);

        let addon = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSValueNode")
            .unwrap();
        assert_eq!(
            string_field(addon, "Name").as_deref(),
            Some("AddOnNode1078")
        );
        assert_eq!(
            ref_values(nif.blocks[0].get_field("Children")),
            vec![addon.block_id as i32]
        );
        assert_eq!(value_u64(nif.blocks[0].get_field("Num Children")), Some(1));
        assert_eq!(
            nested_value(
                value_array(nif.blocks[sequence].get_field("Controlled Blocks")).first(),
                "Node Name",
            )
            .and_then(value_string)
            .as_deref(),
            Some("AddOnNode1078")
        );
        assert!(
            audit_nif(&nif)
                .iter()
                .all(|finding| finding.rule != "animation-target")
        );
    }

    #[test]
    fn sanitizer_preserves_unique_addon_suffixes_and_animation_targets() {
        let mut nif = NifFile::new("fo4");
        let base = nif.add_block(
            "BSValueNode",
            Some(fields([
                ("Name", NifValue::String("AddOnNode298".to_string())),
                ("Value", NifValue::Int(298)),
            ])),
        );
        let suffixed = nif.add_block(
            "BSValueNode",
            Some(fields([
                ("Name", NifValue::String("AddOnNode298@#0".to_string())),
                ("Value", NifValue::Int(298)),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(base as i32),
                NifValue::Ref(suffixed as i32),
            ]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
        let sequence = nif.add_block(
            "NiControllerSequence",
            Some(fields([(
                "Controlled Blocks",
                NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                    "Node Name".to_string(),
                    NifValue::String("AddOnNode298@#0".to_string()),
                )]))]),
            )])),
        );

        sanitize_fo4_nif(&mut nif);

        assert_eq!(
            string_field(&nif.blocks[suffixed], "Name").as_deref(),
            Some("AddOnNode298@#0")
        );
        assert_eq!(
            nested_value(
                value_array(nif.blocks[sequence].get_field("Controlled Blocks")).first(),
                "Node Name",
            )
            .and_then(value_string)
            .as_deref(),
            Some("AddOnNode298@#0")
        );
        assert!(audit_nif(&nif).iter().all(|finding| {
            !matches!(
                finding.rule.as_str(),
                "addon-node-name" | "animation-target"
            )
        }));
    }

    #[test]
    fn sparse_multitarget_slots_are_preserved_but_required_null_links_are_not() {
        let mut nif = NifFile::new("fo4");
        nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
        let target = nif.add_block(
            "NiNode",
            Some(fields([("Name", NifValue::String("Target".to_string()))])),
        );
        let invalid_owner = nif.add_block(
            "NiNode",
            Some(fields([
                ("Num Children", NifValue::UInt(1)),
                ("Children", NifValue::Array(vec![NifValue::Ref(-1)])),
            ])),
        );
        let multitarget = nif.add_block(
            "NiMultiTargetTransformController",
            Some(fields([
                ("Num Extra Targets", NifValue::UInt(3)),
                (
                    "Extra Targets",
                    NifValue::Array(vec![
                        NifValue::Ref(-1),
                        NifValue::Ref(target as i32),
                        NifValue::Ref(-1),
                    ]),
                ),
            ])),
        );
        let sequence = nif.add_block(
            "NiControllerSequence",
            Some(fields([(
                "Controlled Blocks",
                NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                    "Node Name".to_string(),
                    NifValue::String("Target".to_string()),
                )]))]),
            )])),
        );
        let manager = nif.add_block(
            "NiControllerManager",
            Some(fields([
                ("Target", NifValue::Ref(0)),
                ("Next Controller", NifValue::Ref(multitarget as i32)),
                (
                    "Controller Sequences",
                    NifValue::Array(vec![NifValue::Ref(sequence as i32)]),
                ),
            ])),
        );
        nif.blocks[0].set_field("Controller", NifValue::Ref(manager as i32));
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(target as i32),
                NifValue::Ref(invalid_owner as i32),
            ]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(2));

        let findings = audit_nif(&nif);
        assert_eq!(
            findings
                .iter()
                .filter(|finding| finding.rule == "invalid-array-link")
                .count(),
            1
        );
        assert!(
            findings
                .iter()
                .all(|finding| finding.rule != "animation-extra-targets")
        );

        sanitize_fo4_nif(&mut nif);

        let multitarget = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "NiMultiTargetTransformController")
            .unwrap();
        assert_eq!(
            multitarget
                .get_field("Extra Targets")
                .map(ref_scalars)
                .unwrap_or_default(),
            [-1, target as i32, -1]
        );
        let invalid_owner = nif
            .blocks
            .iter()
            .find(|block| block.block_id == invalid_owner)
            .unwrap();
        assert!(
            invalid_owner
                .get_field("Children")
                .map(ref_scalars)
                .unwrap_or_default()
                .is_empty()
        );
        assert!(
            audit_nif(&nif)
                .iter()
                .all(|finding| finding.rule != "invalid-array-link")
        );
    }

    #[test]
    fn face_eye_center_extra_data_keeps_its_vanilla_duplicate_link() {
        let mut nif = NifFile::new("fo4");
        let eye_center = nif.add_block(
            "BSEyeCenterExtraData",
            Some(fields([("Name", NifValue::String("ECED".to_string()))])),
        );
        let shape = nif.add_block(
            "BSSubIndexTriShape",
            Some(fields([
                ("Num Extra Data List", NifValue::UInt(2)),
                (
                    "Extra Data List",
                    NifValue::Array(vec![
                        NifValue::Ref(eye_center as i32),
                        NifValue::Ref(eye_center as i32),
                    ]),
                ),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape as i32)]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));

        assert!(
            audit_nif(&nif)
                .iter()
                .all(|finding| finding.rule != "repeated-array-link")
        );
        sanitize_fo4_nif(&mut nif);

        let shape = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSSubIndexTriShape")
            .unwrap();
        assert_eq!(
            shape
                .get_field("Extra Data List")
                .map(ref_scalars)
                .unwrap_or_default(),
            [eye_center as i32, eye_center as i32]
        );
        assert_eq!(value_u64(shape.get_field("Num Extra Data List")), Some(2));
    }

    #[test]
    fn external_emittance_adds_the_matching_fo4_bsx_flag() {
        let mut nif = NifFile::new("fo4");
        let shader = nif.add_block(
            "BSLightingShaderProperty",
            Some(fields([
                ("Shader Flags 1", NifValue::UInt(SLSF1_EXTERNAL_EMITTANCE)),
                ("Shader Flags 2", NifValue::UInt(0)),
            ])),
        );
        let shape = nif.add_block(
            "BSTriShape",
            Some(fields([
                ("Vertex Desc", NifValue::UInt(0)),
                ("Num Vertices", NifValue::UInt(1)),
                ("Shader Property", NifValue::Ref(shader as i32)),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape as i32)]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));

        sanitize_fo4_nif(&mut nif);

        let bsx = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSXFlags")
            .unwrap();
        assert_ne!(
            value_u64(bsx.get_field("Integer Data")).unwrap() & BSX_EXTERNAL_EMIT,
            0
        );
        assert!(audit_nif(&nif).iter().all(|finding| {
            !matches!(finding.rule.as_str(), "bsx-flags" | "shader-type-flags")
        }));
    }

    #[test]
    fn sanitizer_matches_nif_animation_metadata_repairs() {
        let mut nif = NifFile::new("fo4");
        nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
        let node_a = nif.add_block(
            "NiNode",
            Some(fields([("Name", NifValue::String("A".to_string()))])),
        );
        let node_b = nif.add_block(
            "NiNode",
            Some(fields([("Name", NifValue::String("B".to_string()))])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(node_a as i32),
                NifValue::Ref(node_b as i32),
            ]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
        let palette = nif.add_block("NiDefaultAVObjectPalette", None);
        let multitarget = nif.add_block("NiMultiTargetTransformController", None);
        let sequence = nif.add_block("NiControllerSequence", None);
        let interpolator = nif.add_block("NiFloatInterpolator", None);
        let data = nif.add_block("NiFloatData", None);
        nif.blocks[interpolator].set_field("Data", NifValue::Ref(data as i32));
        let controlled = |name: &str, interpolator: i32| {
            NifValue::Struct(IndexMap::from([
                ("Node Name".to_string(), NifValue::String(name.to_string())),
                ("Interpolator".to_string(), NifValue::Ref(interpolator)),
                ("Controller".to_string(), NifValue::Ref(-1)),
            ]))
        };
        nif.blocks[sequence].set_field(
            "Controlled Blocks",
            NifValue::Array(vec![
                controlled("B", -1),
                controlled("Missing", interpolator as i32),
                controlled("A", -1),
            ]),
        );
        nif.blocks[sequence].set_field("Num Controlled Blocks", NifValue::UInt(3));
        nif.blocks[sequence].set_field("Accum Root Name", NifValue::String("Wrong".to_string()));
        let manager = nif.add_block("NiControllerManager", None);
        nif.blocks[manager].set_field("Target", NifValue::Ref(0));
        nif.blocks[manager].set_field("Next Controller", NifValue::Ref(multitarget as i32));
        nif.blocks[manager].set_field(
            "Controller Sequences",
            NifValue::Array(vec![NifValue::Ref(sequence as i32)]),
        );
        nif.blocks[manager].set_field("Num Controller Sequences", NifValue::UInt(1));
        nif.blocks[manager].set_field("Object Palette", NifValue::Ref(palette as i32));
        nif.blocks[0].set_field("Controller", NifValue::Ref(manager as i32));

        let report = sanitize_nif(&mut nif);
        assert!(report.changes.iter().any(|change| {
            change.contains("removed 1 invalid controlled block")
                && change.contains("sorted 1 sequence")
                && change.contains("updated 1 palette")
                && change.contains("1 extra-target")
        }));
        let sequence = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "NiControllerSequence")
            .unwrap();
        let names = value_array(sequence.get_field("Controlled Blocks"))
            .iter()
            .filter_map(|entry| nested_value(Some(entry), "Node Name"))
            .filter_map(value_string)
            .collect::<Vec<_>>();
        assert_eq!(names, ["A", "B"]);
        assert_eq!(
            string_field(sequence, "Accum Root Name").as_deref(),
            Some("Root")
        );
        assert_eq!(
            nif.blocks
                .iter()
                .filter(|block| matches!(
                    block.type_name.as_str(),
                    "NiFloatInterpolator" | "NiFloatData"
                ))
                .count(),
            0
        );
        let palette = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "NiDefaultAVObjectPalette")
            .unwrap();
        assert_eq!(value_array(palette.get_field("Objs")).len(), 2);
        let multitarget = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "NiMultiTargetTransformController")
            .unwrap();
        assert_eq!(value_array(multitarget.get_field("Extra Targets")).len(), 2);
    }

    #[test]
    fn audit_reports_animation_metadata_repairs_without_mutating() {
        let mut nif = NifFile::new("fo4");
        nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
        let node_a = nif.add_block(
            "NiNode",
            Some(fields([("Name", NifValue::String("A".to_string()))])),
        );
        let node_b = nif.add_block(
            "NiNode",
            Some(fields([("Name", NifValue::String("B".to_string()))])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(node_a as i32),
                NifValue::Ref(node_b as i32),
            ]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
        let palette = nif.add_block("NiDefaultAVObjectPalette", None);
        let multitarget = nif.add_block(
            "NiMultiTargetTransformController",
            Some(fields([(
                "Extra Targets",
                NifValue::Array(vec![NifValue::Ref(node_b as i32), NifValue::Ref(-1)]),
            )])),
        );
        let controlled = |name: &str| {
            NifValue::Struct(IndexMap::from([(
                "Node Name".to_string(),
                NifValue::String(name.to_string()),
            )]))
        };
        let sequence = nif.add_block(
            "NiControllerSequence",
            Some(fields([
                ("Accum Root Name", NifValue::String("A".to_string())),
                (
                    "Controlled Blocks",
                    NifValue::Array(vec![controlled("B"), controlled("A")]),
                ),
            ])),
        );
        let manager = nif.add_block(
            "NiControllerManager",
            Some(fields([
                ("Target", NifValue::Ref(0)),
                ("Next Controller", NifValue::Ref(multitarget as i32)),
                (
                    "Controller Sequences",
                    NifValue::Array(vec![NifValue::Ref(sequence as i32)]),
                ),
                ("Object Palette", NifValue::Ref(palette as i32)),
            ])),
        );
        nif.blocks[0].set_field("Controller", NifValue::Ref(manager as i32));

        let before = nif.to_bytes().unwrap();
        let findings = audit_nif(&nif);
        for rule in [
            "animation-accum-root",
            "animation-controlled-block-order",
            "animation-object-palette",
            "animation-extra-targets",
        ] {
            assert!(
                findings.iter().any(|finding| finding.rule == rule),
                "missing {rule}: {findings:#?}"
            );
        }
        assert_eq!(nif.to_bytes().unwrap(), before);

        sanitize_fo4_nif(&mut nif);
        let findings = audit_nif(&nif);
        for rule in [
            "animation-accum-root",
            "animation-controlled-block-order",
            "animation-object-palette",
            "animation-extra-targets",
        ] {
            assert!(
                findings.iter().all(|finding| finding.rule != rule),
                "{rule} remained after sanitation: {findings:#?}"
            );
        }
    }

    #[test]
    fn audit_reports_geometry_hygiene_without_mutating_it() {
        let mut nif = NifFile::new("fo4");
        let white = NifValue::Struct(IndexMap::from([
            ("r".to_string(), NifValue::UInt(255)),
            ("g".to_string(), NifValue::UInt(255)),
            ("b".to_string(), NifValue::UInt(255)),
            ("a".to_string(), NifValue::UInt(255)),
        ]));
        let vertex = NifValue::Struct(IndexMap::from([
            ("Vertex".to_string(), NifValue::Vec3([0.0, 0.0, 0.0])),
            ("Vertex Colors".to_string(), white),
        ]));
        let shape = nif.add_block(
            "BSTriShape",
            Some(fields([
                ("Vertex Desc", NifValue::UInt(VF_VERTEX_COLORS << 44)),
                ("Num Vertices", NifValue::UInt(2)),
                ("Vertex Data", NifValue::Array(vec![vertex.clone(), vertex])),
                (
                    "Triangles",
                    NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                        ("v1".to_string(), NifValue::UInt(0)),
                        ("v2".to_string(), NifValue::UInt(0)),
                        ("v3".to_string(), NifValue::UInt(0)),
                    ]))]),
                ),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape as i32)]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));

        let findings = audit_nif(&nif);
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "duplicate-vertices")
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "unused-vertices")
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "all-white-vertex-colors")
        );
    }

    #[test]
    fn game_specific_shader_rules_follow_the_nif_header() {
        let mut fnv = NifFile::new("fnv");
        let fnv_shader = fnv.add_block(
            "BSShaderPPLightingProperty",
            Some(fields([
                ("Shader Flags", NifValue::UInt(0)),
                ("Shader Flags 2", NifValue::UInt(0)),
            ])),
        );
        let fnv_shape = fnv.add_block(
            "NiTriShape",
            Some(fields([
                (
                    "Properties",
                    NifValue::Array(vec![NifValue::Ref(fnv_shader as i32)]),
                ),
                ("Skin Instance", NifValue::Ref(0)),
            ])),
        );
        fnv.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(fnv_shape as i32)]),
        );
        assert_eq!(shader_usage(&fnv).get(&fnv_shader).unwrap().skinned, 1);
        normalize_shader_flags(&mut fnv, &mut SanitizeReport::default());
        let fnv_shader = fnv.get_block(fnv_shader).unwrap();
        assert_eq!(
            value_u64(fnv_shader.get_field("Shader Flags 2")).unwrap() & SLSF2_VERTEX_COLORS,
            0
        );
        assert_ne!(
            value_u64(fnv_shader.get_field("Shader Flags")).unwrap() & SLSF1_SKINNED,
            0
        );

        let mut sse = NifFile::new("skyrimse");
        let sse_shader = sse.add_block(
            "BSLightingShaderProperty",
            Some(fields([
                ("Shader Type", NifValue::UInt(0)),
                ("Shader Flags 1", NifValue::UInt(SLSF1_DYNAMIC_DECAL)),
                ("Shader Flags 2", NifValue::UInt(0)),
                ("Glossiness", NifValue::Float(0.0)),
            ])),
        );
        let sse_shape = sse.add_block(
            "BSTriShape",
            Some(fields([
                ("Vertex Desc", NifValue::UInt(0)),
                ("Num Vertices", NifValue::UInt(1)),
                ("Shader Property", NifValue::Ref(sse_shader as i32)),
            ])),
        );
        sse.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(sse_shape as i32)]),
        );
        sanitize_nif(&mut sse);
        let sse_shader = sse.get_block(sse_shader).unwrap();
        assert_ne!(
            value_u64(sse_shader.get_field("Shader Flags 1")).unwrap() & SLSF1_DECAL,
            0
        );
        assert_ne!(
            value_u64(sse_shader.get_field("Shader Flags 2")).unwrap() & SLSF2_ASSUME_SHADOWMASK,
            0
        );
        assert_eq!(value_f64(sse_shader.get_field("Glossiness")), Some(1.0));

        let mut skyrim = NifFile::new("skyrim");
        assert_eq!(validate_nif(&mut skyrim, false).game, "skyrim");
        assert_eq!(
            validate_nif(&mut NifFile::new("morrowind"), false).game,
            "morrowind"
        );
        assert_eq!(
            validate_nif(&mut NifFile::new("oblivion"), false).game,
            "oblivion"
        );
        assert_eq!(validate_nif(&mut NifFile::new("fo76"), false).game, "fo76");
        assert_eq!(
            validate_nif(&mut NifFile::new("starfield"), false).game,
            "starfield"
        );
    }

    #[test]
    fn skyrim_universal_fixer_infers_shader_type_from_textures_and_flags() {
        let mut nif = NifFile::new("skyrim");
        let texture_set = nif.add_block(
            "BSShaderTextureSet",
            Some(fields([(
                "Textures",
                NifValue::Array(vec![
                    NifValue::String("textures\\diffuse.dds".to_string()),
                    NifValue::String("textures\\normal.dds".to_string()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String("textures\\cube.dds".to_string()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                ]),
            )])),
        );
        let shader = nif.add_block(
            "BSLightingShaderProperty",
            Some(fields([
                ("Shader Type", NifValue::UInt(0)),
                ("Shader Flags 1", NifValue::UInt(SLSF1_ENVIRONMENT_MAPPING)),
                ("Shader Flags 2", NifValue::UInt(0)),
                ("Texture Set", NifValue::Ref(texture_set as i32)),
            ])),
        );
        let shape = nif.add_block(
            "NiTriShape",
            Some(fields([(
                "Properties",
                NifValue::Array(vec![NifValue::Ref(shader as i32)]),
            )])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape as i32)]),
        );

        normalize_shader_flags(&mut nif, &mut SanitizeReport::default());

        assert_eq!(
            value_u64(nif.blocks[shader].get_field("Shader Type")),
            Some(SHADER_ENVIRONMENT_MAP)
        );
    }

    #[test]
    fn oblivion_universal_fixer_resolves_controlled_block_string_palettes() {
        let mut nif = NifFile::new("oblivion");
        nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
        let target = nif.add_block(
            "NiNode",
            Some(fields([(
                "Name",
                NifValue::String("Bip01 Head".to_string()),
            )])),
        );
        let palette = nif.add_block(
            "NiStringPalette",
            Some(fields([(
                "Palette",
                NifValue::Struct(IndexMap::from([(
                    "Palette".to_string(),
                    NifValue::String("Bip01 Head\0".to_string()),
                )])),
            )])),
        );
        let sequence = nif.add_block(
            "NiControllerSequence",
            Some(fields([
                ("Accum Root Name", NifValue::String(String::new())),
                ("Num Controlled Blocks", NifValue::UInt(1)),
                (
                    "Controlled Blocks",
                    NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                        ("String Palette".to_string(), NifValue::Ref(palette as i32)),
                        ("Node Name Offset".to_string(), NifValue::UInt(0)),
                        ("Interpolator".to_string(), NifValue::Ref(-1)),
                        ("Controller".to_string(), NifValue::Ref(-1)),
                    ]))]),
                ),
            ])),
        );
        let manager = nif.add_block(
            "NiControllerManager",
            Some(fields([
                ("Target", NifValue::Ref(0)),
                (
                    "Controller Sequences",
                    NifValue::Array(vec![NifValue::Ref(sequence as i32)]),
                ),
                ("Next Controller", NifValue::Ref(-1)),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(target as i32)]),
        );
        nif.blocks[0].set_field("Controller", NifValue::Ref(manager as i32));

        normalize_animation_metadata(&mut nif, &mut SanitizeReport::default());

        assert_eq!(
            value_array(nif.blocks[sequence].get_field("Controlled Blocks")).len(),
            1
        );
    }

    #[test]
    fn optional_sse_crash_format_checks_are_opt_in() {
        let mut nif = NifFile::new("skyrimse");
        let strips = nif.add_block("NiTriStrips", None);
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(strips as i32)]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));

        assert!(
            !audit_nif(&nif)
                .iter()
                .any(|finding| finding.rule == "sse-unsupported-block")
        );
        assert!(
            audit_nif_with_options(&nif, true)
                .iter()
                .any(|finding| finding.rule == "sse-unsupported-block")
        );
    }

    #[test]
    fn fixer_handles_nested_legacy_rigid_body_settings() {
        let mut nif = NifFile::new("fnv");
        let body_id = nif.add_block("bhkRigidBody", None);
        nif.blocks[body_id].fields.clear();
        nif.blocks[body_id].set_field(
            "Rigid Body Info:550_660",
            NifValue::Struct(IndexMap::from([
                (
                    "Havok Filter".to_string(),
                    NifValue::Struct(IndexMap::from([(
                        "Layer:FO".to_string(),
                        NifValue::UInt(1),
                    )])),
                ),
                (
                    "Havok Filter Copy".to_string(),
                    NifValue::Struct(IndexMap::from([(
                        "Layer:FO".to_string(),
                        NifValue::UInt(0),
                    )])),
                ),
                ("Motion System".to_string(), NifValue::UInt(1)),
                ("Motion Quality".to_string(), NifValue::UInt(4)),
                ("Deactivator Type".to_string(), NifValue::UInt(1)),
                ("Enable Deactivation".to_string(), NifValue::Bool(true)),
                ("Solver Deactivation".to_string(), NifValue::UInt(1)),
                ("Mass".to_string(), NifValue::Float(2.0)),
                (
                    "Inertia Tensor".to_string(),
                    NifValue::Matrix33([[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0]]),
                ),
                ("Time Factor".to_string(), NifValue::Float(0.0)),
                ("Gravity Factor".to_string(), NifValue::Float(0.0)),
            ])),
        );

        sanitize_nif(&mut nif);

        let body = nif.get_block(body_id).unwrap();
        assert_eq!(value_u64(rigid_body_value(body, "Motion System")), Some(7));
        assert_eq!(value_f64(rigid_body_value(body, "Mass")), Some(0.0));
        assert_eq!(
            rigid_body_nested_u64(body, "Havok Filter Copy", "Layer"),
            Some(1)
        );
        assert_eq!(value_f64(rigid_body_value(body, "Time Factor")), Some(1.0));
        assert_eq!(
            rigid_body_value(body, "Inertia Tensor"),
            Some(&NifValue::Matrix33([
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
            ]))
        );
    }

    #[test]
    fn audit_covers_strip_indices_and_mopp_complexity() {
        let mut nif = NifFile::new("fnv");
        nif.add_block("bhkMoppBvTreeShape", None);
        nif.add_block(
            "BSTriShape",
            Some(fields([("Num Triangles", NifValue::UInt(20))])),
        );
        let triangle = NifValue::Struct(IndexMap::from([
            ("v1".to_string(), NifValue::UInt(0)),
            ("v2".to_string(), NifValue::UInt(1)),
            ("v3".to_string(), NifValue::UInt(2)),
        ]));
        nif.add_block(
            "hkPackedNiTriStripsData",
            Some(fields([("Triangles", NifValue::Array(vec![triangle; 12]))])),
        );
        let strips = nif.add_block(
            "NiTriStripsData",
            Some(fields([
                ("Num Vertices", NifValue::UInt(4)),
                ("Num Strips", NifValue::UInt(2)),
                (
                    "Strips",
                    NifValue::Array(vec![NifValue::Array(vec![
                        NifValue::UInt(0),
                        NifValue::UInt(1),
                        NifValue::UInt(5),
                    ])]),
                ),
            ])),
        );
        nif.add_block(
            "NiTriStrips",
            Some(fields([("Data", NifValue::Ref(strips as i32))])),
        );

        let findings = audit_nif(&nif);
        for rule in [
            "strip-index",
            "multiple-triangle-strips",
            "collision-mopp-complexity",
        ] {
            assert!(
                findings.iter().any(|finding| finding.rule == rule),
                "missing {rule}: {findings:?}"
            );
        }
    }

    #[test]
    fn audit_matches_oblivion_material_tangent_and_consistency_checks() {
        let mut nif = NifFile::new("oblivion");
        let material = nif.add_block(
            "NiMaterialProperty",
            Some(fields([("Name", NifValue::String(String::new()))])),
        );
        let texturing = nif.add_block("NiTexturingProperty", None);
        let geometry = nif.add_block(
            "NiTriShapeData",
            Some(fields([
                ("Num Vertices", NifValue::UInt(2)),
                ("Consistency Flags", NifValue::UInt(0)),
            ])),
        );
        let tangents = nif.add_block(
            "NiBinaryExtraData",
            Some(fields([
                (
                    "Name",
                    NifValue::String(OBLIVION_TANGENT_DATA_NAME.to_string()),
                ),
                (
                    "Binary Data",
                    NifValue::Struct(IndexMap::from([
                        ("Data Size".to_string(), NifValue::UInt(24)),
                        ("Data".to_string(), NifValue::Bytes(vec![0; 24])),
                    ])),
                ),
            ])),
        );
        let shape = nif.add_block(
            "NiTriShape",
            Some(fields([
                ("Data", NifValue::Ref(geometry as i32)),
                (
                    "Properties",
                    NifValue::Array(vec![
                        NifValue::Ref(material as i32),
                        NifValue::Ref(texturing as i32),
                    ]),
                ),
                (
                    "Extra Data List",
                    NifValue::Array(vec![NifValue::Ref(tangents as i32)]),
                ),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape as i32)]),
        );

        let findings = audit_nif(&nif);
        for rule in [
            "oblivion-material-name",
            "oblivion-tangent-count",
            "consistency-flags",
        ] {
            assert!(
                findings.iter().any(|finding| finding.rule == rule),
                "missing {rule}: {findings:?}"
            );
        }

        sanitize_nif(&mut nif);
        let geometry = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "NiTriShapeData")
            .unwrap();
        assert_eq!(
            value_u64(geometry.get_field("Consistency Flags")),
            Some(0x4000)
        );
    }

    #[test]
    fn audit_checks_end_text_key_without_controlled_blocks() {
        let mut nif = NifFile::new("fo4");
        let text_keys = nif.add_block(
            "NiTextKeyExtraData",
            Some(fields([(
                "Text Keys",
                NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                    ("Time".to_string(), NifValue::Float(1.0)),
                    ("Value".to_string(), NifValue::String("end".to_string())),
                ]))]),
            )])),
        );
        nif.add_block(
            "NiControllerSequence",
            Some(fields([
                ("Stop Time", NifValue::Float(2.0)),
                ("Controlled Blocks", NifValue::Array(Vec::new())),
                ("Text Keys", NifValue::Ref(text_keys as i32)),
            ])),
        );

        assert!(
            audit_nif(&nif)
                .iter()
                .any(|finding| finding.rule == "animation-stop-time")
        );
    }

    #[test]
    fn audit_accepts_addon_node_suffixes_like_nif() {
        let mut nif = NifFile::new("fo4");
        nif.add_block(
            "BSValueNode",
            Some(fields([
                (
                    "Name",
                    NifValue::String("AddOnNode1078-OptionalSuffix".to_string()),
                ),
                ("Value", NifValue::Int(1078)),
            ])),
        );

        assert!(
            audit_nif(&nif)
                .iter()
                .all(|finding| finding.rule != "addon-node-name")
        );
    }

    #[test]
    fn audit_checks_animated_collision_layers_and_flags() {
        let mut nif = NifFile::new("skyrim");
        let transform_data = nif.add_block("NiTransformData", None);
        let interpolator = nif.add_block(
            "NiTransformInterpolator",
            Some(fields([("Data", NifValue::Ref(transform_data as i32))])),
        );
        let animated_body = nif.add_block(
            "bhkRigidBody",
            Some(fields([(
                "Havok Filter",
                NifValue::Struct(IndexMap::from([("Layer".to_string(), NifValue::UInt(2))])),
            )])),
        );
        let animated_collision = nif.add_block(
            "bhkCollisionObject",
            Some(fields([
                ("Body", NifValue::Ref(animated_body as i32)),
                ("Flags", NifValue::UInt(0)),
            ])),
        );
        let animated_node = nif.add_block(
            "NiNode",
            Some(fields([
                ("Name", NifValue::String("AnimatedNode".to_string())),
                ("Collision Object", NifValue::Ref(animated_collision as i32)),
            ])),
        );
        let static_body = nif.add_block(
            "bhkRigidBody",
            Some(fields([(
                "Havok Filter",
                NifValue::Struct(IndexMap::from([("Layer".to_string(), NifValue::UInt(28))])),
            )])),
        );
        let static_collision = nif.add_block(
            "bhkCollisionObject",
            Some(fields([("Body", NifValue::Ref(static_body as i32))])),
        );
        let static_node = nif.add_block(
            "NiNode",
            Some(fields([(
                "Collision Object",
                NifValue::Ref(static_collision as i32),
            )])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(animated_node as i32),
                NifValue::Ref(static_node as i32),
            ]),
        );
        nif.add_block(
            "NiControllerSequence",
            Some(fields([(
                "Controlled Blocks",
                NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                    (
                        "Controller Type".to_string(),
                        NifValue::String("NiTransformController".to_string()),
                    ),
                    (
                        "Node Name".to_string(),
                        NifValue::String("AnimatedNode".to_string()),
                    ),
                    (
                        "Interpolator".to_string(),
                        NifValue::Ref(interpolator as i32),
                    ),
                ]))]),
            )])),
        );

        let findings = audit_nif(&nif);
        assert!(
            findings
                .iter()
                .any(|finding| finding.rule == "collision-set-local")
        );
        assert!(findings.iter().any(|finding| {
            finding.rule == "collision-animated-layer" && finding.block_id == Some(static_collision)
        }));
    }

    #[test]
    fn audit_checks_pre_skyrim_animated_collision_use_velocity() {
        let mut nif = NifFile::new("oblivion");
        let transform_data = nif.add_block("NiTransformData", None);
        let interpolator = nif.add_block(
            "NiTransformInterpolator",
            Some(fields([("Data", NifValue::Ref(transform_data as i32))])),
        );
        let body = nif.add_block(
            "bhkRigidBody",
            Some(fields([
                (
                    "Havok Filter",
                    NifValue::Struct(IndexMap::from([("Layer".to_string(), NifValue::UInt(4))])),
                ),
                ("Motion System", NifValue::UInt(6)),
            ])),
        );
        let collision = nif.add_block(
            "bhkCollisionObject",
            Some(fields([
                ("Body", NifValue::Ref(body as i32)),
                ("Flags", NifValue::UInt(0)),
            ])),
        );
        let node = nif.add_block(
            "NiNode",
            Some(fields([
                ("Name", NifValue::String("AnimatedNode".to_string())),
                ("Collision Object", NifValue::Ref(collision as i32)),
            ])),
        );
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(node as i32)]),
        );
        nif.add_block(
            "NiControllerSequence",
            Some(fields([(
                "Controlled Blocks",
                NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                    (
                        "Controller Type".to_string(),
                        NifValue::String("NiTransformController".to_string()),
                    ),
                    (
                        "Node Name".to_string(),
                        NifValue::String("AnimatedNode".to_string()),
                    ),
                    (
                        "Interpolator".to_string(),
                        NifValue::Ref(interpolator as i32),
                    ),
                ]))]),
            )])),
        );

        assert!(
            audit_nif(&nif)
                .iter()
                .any(|finding| finding.rule == "collision-use-velocity")
        );
    }

    #[test]
    fn converted_fo76_regression_samples_are_normalized_in_memory() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../mods/SeventySix/data/Meshes");
        let hazard_path = root.join("actors/MPS_EmpyHazard.nif");
        let skeleton_path = root.join("actors/antiairturret/characterassets/skeleton.nif");
        let addon_path = root.join(
            "atx/SetDressing/ATX_Light_Ceiling_BranchChandelier/ATX_Light_Ceiling_BranchChandelier.nif",
        );
        if !hazard_path.is_file() || !skeleton_path.is_file() || !addon_path.is_file() {
            return;
        }

        let mut hazard = NifFile::load(hazard_path).unwrap();
        let hazard_report = sanitize_nif(&mut hazard);
        assert_eq!(nif_game(&hazard), NifGame::Fo4);
        assert!(shader_usage(&hazard).into_iter().all(|(shader_id, usage)| {
            usage.with_vertex_colors == 0
                || value_u64(hazard.blocks[shader_id].get_field("Shader Flags 2"))
                    .is_some_and(|flags| flags & SLSF2_VERTEX_COLORS != 0)
        }));
        assert!(
            hazard_report
                .changes
                .iter()
                .any(|change| change.starts_with("Shader flags:"))
                || audit_shader_vertex_color_parity(&hazard)
        );

        let mut skeleton = NifFile::load(skeleton_path).unwrap();
        sanitize_nif(&mut skeleton);
        let bsx = skeleton
            .blocks
            .iter()
            .find(|block| block.type_name == "BSXFlags")
            .unwrap();
        assert_eq!(value_u64(bsx.get_field("Integer Data")), Some(194));

        let mut addon = NifFile::load(addon_path).unwrap();
        sanitize_nif(&mut addon);
        assert!(addon.blocks.iter().all(|block| {
            block.type_name != "BSValueNode"
                || string_field(block, "Name").is_none_or(|name| {
                    parse_addon_node_index(&name)
                        .is_none_or(|(_, digits)| name == format!("AddOnNode{digits}"))
                })
        }));
    }

    fn audit_shader_vertex_color_parity(nif: &NifFile) -> bool {
        !audit_nif(nif)
            .iter()
            .any(|finding| finding.rule == "vertex-color-shader-flag")
    }
}
