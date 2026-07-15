use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use indexmap::IndexMap;
use thiserror::Error;

use crate::fo76_collision::{
    CollisionRoute, ExtractedCollisionBody, FO4_CLUTTER_LAYER, FO4_STATIC_LAYER,
    PlannedCollisionBody, RouteCounts, SourceBodyMetadata, classify_source_body,
    collision_summary_is_invalid, extract_source_collision_body, is_dynamic_from_nif_signals,
    motion_type_label, nif_vertices_to_havok, source_body_metadata,
    summary_has_degenerate_collision_shape,
};
use crate::model::{NifBlock, NifFile, NifValue};
use havok_native::collision::multi_body::{
    BodyMeta, BodyMotionType, build_fo4_multi_body_collision_with_constraints,
};
use havok_native::collision::{
    BuildOptions, CompoundChildKind, GraftCinfo, GraftedConstraints, MultiBodyShape,
    with_collision_diagnostic_context,
};

const VF_VERTEX: i64 = 0x0001;
const VF_UVS: i64 = 0x0002;
const VF_NORMALS: i64 = 0x0008;
const VF_TANGENTS: i64 = 0x0010;
const VF_VERTEX_COLORS: i64 = 0x0020;
const VF_SKINNED: i64 = 0x0040;
const SLSF1_SPECULAR: u64 = 1 << 0;
const SLSF1_SKINNED: u64 = 1 << 1;
const SLSF1_ENVIRONMENT_MAPPING: u64 = 1 << 7;
const SLSF1_HAIR: u64 = 1 << 18;
const SLSF1_OWN_EMIT: u64 = 1 << 22;
const SLSF1_DECAL: u64 = 1 << 26;
const SLSF2_TRANSFORM_CHANGED: u64 = 1 << 7;

/// FO4 hair color-gradient palette bound in GreyscaleToPalette slot 3.
const FO4_HAIR_PALETTE: &str = r"textures\Actors\Character\Hair\HairColor_LGrad_d.dds";
/// Vanilla FO4 baked-hair NiAlphaProperty: AlphaBlend+AlphaTest, threshold 90.
const FO4_HAIR_ALPHA_FLAGS: u64 = 4844;
const FO4_HAIR_ALPHA_THRESHOLD: u64 = 90;
const BSLSP_SHADER_TYPE_DEFAULT: u64 = 0;
const BSLSP_SHADER_TYPE_ENVIRONMENT_MAP: u64 = 1;
const BSLSP_SHADER_TYPE_GLOW: u64 = 2;
const BSLSP_SHADER_TYPE_SKIN_TINT: u64 = 5;
const BSLSP_SHADER_TYPE_HAIR_TINT: u64 = 6;
const TEX_CLAMP_MODE_CLAMP_S_CLAMP_T: u64 = 0;
const SLSF2_ZBUFFER_WRITE: u32 = 1 << 0;
const SLSF2_DOUBLE_SIDED: u32 = 1 << 4;
const SLSF2_VERTEX_COLORS: u32 = 1 << 5;
const SLSF2_GLOW_MAP: u64 = 1 << 6;
const SLSF2_TREE_ANIM: u32 = 1 << 29;
const BSX_DYNAMIC_FLAG: u64 = 0x40;
const BSX_COMPLEX_FLAG: u64 = 0x08;
const BSX_ARTICULATED_FLAG: u64 = 0x80;
const FO4_TEXTURE_SLOT_COUNT: usize = 10;
const HAVOK_SCALE: f32 = 69.99125;
const DEFAULT_COLLISION_RADIUS: f64 = 0.01;
const FO76_PBR_SHADER_FLAG_CRC: u64 = 731263983;
const FO76_TEMP_GROUND_DECAL_MATERIAL: &str =
    "materials\\landscape\\ground\\temp_groundtexture01decal.bgsm";
const FO4_DIRT_PATH_MATERIAL: &str = "Materials\\Landscape\\Ground\\DirtPath01.bgsm";
const FO76_VISIBLE_AABB_FALLBACK_MAX_EXTENT: f32 = 256.0;
const FO76_VISIBLE_AABB_FALLBACK_MIN_EXTENT: f32 = 1.0;
const FO4_NINODE_ROOT_FLAGS: u64 = 0x000E;
const FO76_STATIC_ROOT_FLAG: u64 = 0x4000;
const NIF_NODE_EDITOR_MARKER_FLAG: u64 = 0x2000_0000;
const NIF_NODE_PRESERVE_HIGH_FLAG_COMPANION: u64 = 0x0008_0000;
const FO4_WATER_SHADER_FLAGS_1: u64 = 0x8000_0000;
const FO4_WATER_SHADER_FLAGS_2: u64 = 0x0000_0001;
const FO4_WATER_SHADER_FLAGS: u64 = 0x0000_00C4;

#[derive(Debug, Clone)]
pub struct ConvertFileOptions {
    pub asset_prefix: Option<String>,
    pub material_namespace: Option<String>,
    pub asset_namespace_paths: HashSet<String>,
    pub material_namespace_paths: HashSet<String>,
    pub addon_index_map: HashMap<i64, i64>,
    pub translation_maps_dir: Option<PathBuf>,
    pub auto_skin_reference_body: Option<PathBuf>,
    pub emit_first_person: bool,
    pub first_person_reference: Option<PathBuf>,
    pub morph_weight_cap: f32,
    pub weapon_role: Option<String>,
    /// Source-game data root (e.g. the FO76 `extracted/fo76` dir). When set, the
    /// FO76→FO4 external-BGSM normalizer reads each referenced source material to
    /// decide whether the FO4 shader needs the `Glow_Map` flag. None disables it.
    pub source_material_dir: Option<PathBuf>,
    /// Source material substitutions keyed by canonical `materials/...` paths.
    /// The NIF keeps the original material reference, but shader texture data is
    /// read from the replacement source material.
    pub material_source_overrides: HashMap<String, String>,
}

impl Default for ConvertFileOptions {
    fn default() -> Self {
        Self {
            asset_prefix: None,
            material_namespace: None,
            asset_namespace_paths: HashSet::new(),
            material_namespace_paths: HashSet::new(),
            addon_index_map: HashMap::new(),
            translation_maps_dir: None,
            auto_skin_reference_body: None,
            emit_first_person: false,
            first_person_reference: None,
            morph_weight_cap: 0.5,
            weapon_role: None,
            source_material_dir: None,
            material_source_overrides: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConvertFileReport {
    pub supported: bool,
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub emitted_bgsms: Vec<String>,
    pub emitted_first_person: Option<String>,
    pub shapes_skinned: usize,
    pub vertices_repacked: usize,
    pub bones_remapped: usize,
    pub bones_dropped_unmapped: usize,
    pub weights_redistributed: usize,
    pub vertices_morph_weighted: usize,
    pub timings_ms: Vec<(String, u64)>,
}

impl ConvertFileReport {
    fn record_timing_ms(&mut self, step: &str, started: Instant) {
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.timings_ms.push((step.to_string(), elapsed));
    }
}

#[derive(Debug, Error)]
pub enum ConvertFileError {
    #[error("read: {0}")]
    Read(#[from] crate::io::ReadError),
    #[error("write: {0}")]
    Write(#[from] crate::io::WriteError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("legacy skin conversion: {0}")]
    LegacySkin(#[from] crate::skin::ConvertLegacySkinError),
}

pub fn convert_nif_file(
    src: &Path,
    dst: &Path,
    source_game: &str,
    target_game: &str,
    bgsm_output_dir: Option<&Path>,
    options: &ConvertFileOptions,
) -> Result<ConvertFileReport, ConvertFileError> {
    let total_started = Instant::now();
    let source_game = normalize_game(source_game);
    let target_game = normalize_game(target_game);
    let weapon_role = options
        .weapon_role
        .as_deref()
        .map(normalize_weapon_role)
        .filter(|role| matches!(*role, "gun" | "melee"));

    let mut report = ConvertFileReport::default();
    if source_game == target_game
        && options.addon_index_map.is_empty()
        && weapon_role.is_none()
        && !options.emit_first_person
    {
        let copy_started = Instant::now();
        copy_file(src, dst)?;
        report.record_timing_ms("copy_file", copy_started);
        report.supported = true;
        report
            .changes
            .push("Copied NIF without retargeting".to_string());
        report.record_timing_ms("total", total_started);
        return Ok(report);
    }

    if source_game != target_game
        && !matches!(
            (source_game.as_str(), target_game.as_str()),
            ("fnv", "fo4") | ("fo3", "fo4") | ("fo76", "fo4") | ("skyrimse", "fo4")
        )
    {
        report.errors.push(format!(
            "native NIF conversion does not support {source_game} -> {target_game}"
        ));
        report.record_timing_ms("total", total_started);
        return Ok(report);
    }

    let preflight_started = Instant::now();
    if !has_nif_header(src)? {
        report.record_timing_ms("preflight", preflight_started);
        report
            .errors
            .push("source is not a recognized NIF file".to_string());
        report.record_timing_ms("total", total_started);
        return Ok(report);
    }
    report.record_timing_ms("preflight", preflight_started);

    let load_started = Instant::now();
    let mut nif = NifFile::load(src.to_path_buf())?;
    report.record_timing_ms("load", load_started);
    let skyrim_static = source_game == "skyrimse" && target_game == "fo4";
    if skyrim_static {
        if let Err(error) = crate::skyrim::validate_static_only(&nif) {
            report.errors.push(error);
            report.record_timing_ms("total", total_started);
            return Ok(report);
        }
    }
    if source_game != target_game {
        let step_started = Instant::now();
        retarget_header(&mut nif, &target_game, &mut report);
        report.record_timing_ms("retarget_header", step_started);
        if skyrim_static {
            let step_started = Instant::now();
            let normalized = crate::skyrim::normalize_static_geometry(&mut nif);
            if normalized > 0 {
                report.changes.push(format!(
                    "Skyrim static geometry: normalized {normalized} shape(s) to the FO4 vertex stream"
                ));
            }
            report.record_timing_ms("skyrim_static_geometry", step_started);

            let step_started = Instant::now();
            normalize_texture_sets(
                &mut nif,
                &source_game,
                &target_game,
                options.asset_prefix.as_deref(),
                &options.asset_namespace_paths,
                &mut report,
            );
            report.record_timing_ms("normalize_texture_sets", step_started);

            let step_started = Instant::now();
            if let Some(output_dir) = bgsm_output_dir {
                let material_report =
                    crate::skyrim::synthesize_fo4_materials(&mut nif, src, output_dir)?;
                report.emitted_bgsms.extend(
                    material_report
                        .emitted
                        .into_iter()
                        .map(|path| path.to_string_lossy().into_owned()),
                );
                report.warnings.extend(material_report.warnings);
                if !report.emitted_bgsms.is_empty() {
                    report.changes.push(format!(
                        "Skyrim inline shaders: emitted {} FO4 BGSM/BGEM material(s)",
                        report.emitted_bgsms.len()
                    ));
                }
            } else if nif.blocks.iter().any(|block| {
                matches!(
                    block.type_name.as_str(),
                    "BSLightingShaderProperty" | "BSEffectShaderProperty"
                )
            }) {
                report.warnings.push(
                    "Skyrim inline shaders require bgsm_output_dir; no external FO4 material was emitted"
                        .to_string(),
                );
            }
            report.record_timing_ms("skyrim_materials", step_started);

            let step_started = Instant::now();
            let collision_report = crate::skyrim_collision::bridge_static_collision(&mut nif);
            if collision_report.converted > 0 || collision_report.stripped > 0 {
                report.changes.push(format!(
                    "Skyrim static collision: converted {} chain(s), stripped {} unsupported chain(s)",
                    collision_report.converted, collision_report.stripped
                ));
            }
            report.warnings.extend(collision_report.warnings);
            report.record_timing_ms("skyrim_static_collision", step_started);
        } else if source_game == "fo76" && target_game == "fo4" {
            let step_started = Instant::now();
            fix_fo76_float_controllers(&mut nif, &mut report);
            report.record_timing_ms("fo76_float_controllers", step_started);
            let step_started = Instant::now();
            flatten_fo76_effect_shader(&mut nif, &mut report);
            report.record_timing_ms("fo76_effect_shader", step_started);
            let step_started = Instant::now();
            ensure_fo4_effect_shader_defaults(&mut nif, &mut report);
            report.record_timing_ms("fo4_effect_shader_defaults", step_started);
            let step_started = Instant::now();
            flatten_fo76_water_shader(&mut nif, &mut report);
            report.record_timing_ms("fo76_water_shader", step_started);
            let step_started = Instant::now();
            flatten_fo76_lighting_shader(&mut nif, &mut report);
            report.record_timing_ms("fo76_lighting_shader", step_started);
            let step_started = Instant::now();
            clear_static_shape_skinned_shader_flags(&mut nif, &mut report);
            report.record_timing_ms("fo76_shader_skin_flags", step_started);
            let step_started = Instant::now();
            rewire_orphan_texture_sets(&mut nif, &mut report);
            report.record_timing_ms("fo76_orphan_texture_sets", step_started);
            let step_started = Instant::now();
            propagate_texture_sets_by_material(&mut nif, &mut report);
            report.record_timing_ms("fo76_material_texture_sets", step_started);
            let step_started = Instant::now();
            strip_fo76_position_data(&mut nif, &mut report);
            report.record_timing_ms("fo76_position_data", step_started);
            let step_started = Instant::now();
            ensure_fo4_lighting_shader_defaults(&mut nif, &mut report);
            report.record_timing_ms("fo4_lighting_shader_defaults", step_started);
            let step_started = Instant::now();
            remap_fo76_texture_slots(&mut nif, &mut report);
            report.record_timing_ms("fo76_texture_slots", step_started);
            let step_started = Instant::now();
            normalize_external_bgsm_shader_data_with_overrides(
                &mut nif,
                options.source_material_dir.as_deref(),
                &options.material_source_overrides,
                &mut report,
            );
            report.record_timing_ms("fo4_external_bgsm_shader_data", step_started);
            let step_started = Instant::now();
            clear_fo76_invalid_environment_mapping(&mut nif, &mut report);
            report.record_timing_ms("fo76_environment_mapping", step_started);
            let step_started = Instant::now();
            rebuild_fo76_np_collision(&mut nif, &mut report);
            report.record_timing_ms("fo76_np_collision", step_started);
            let step_started = Instant::now();
            synthesize_fo76_ground_object_collision(&mut nif, &mut report);
            report.record_timing_ms("fo76_ground_object_collision", step_started);
            let step_started = Instant::now();
            convert_fo76_havok_blobs(&mut nif, &mut report);
            report.record_timing_ms("fo76_havok_blobs", step_started);
            let step_started = Instant::now();
            convert_fo76_cloth_blobs(&mut nif, &mut report);
            report.record_timing_ms("fo76_cloth_blobs", step_started);
            let step_started = Instant::now();
            normalize_fo76_headwear_segments(&mut nif, &mut report);
            report.record_timing_ms("fo76_headwear_segments", step_started);
        } else {
            let step_started = Instant::now();
            strips_to_tri_shape(&mut nif, &mut report);
            report.record_timing_ms("strips_to_tri_shape", step_started);
            let step_started = Instant::now();
            run_legacy_skin_conversion(&mut nif, &source_game, &target_game, options, &mut report)?;
            report.record_timing_ms("legacy_skin_conversion", step_started);
            let step_started = Instant::now();
            legacy_shader_to_lighting(&mut nif, &mut report);
            report.record_timing_ms("legacy_shader_to_lighting", step_started);
            let step_started = Instant::now();
            mark_skinned_shape_shaders(&mut nif);
            report.record_timing_ms("mark_skinned_shape_shaders", step_started);
            let step_started = Instant::now();
            regenerate_fo4_collision(&mut nif, &mut report);
            report.record_timing_ms("regenerate_fo4_collision", step_started);
        }
        if !skyrim_static {
            let step_started = Instant::now();
            normalize_external_material_names(
                &mut nif,
                options.material_namespace.as_deref(),
                &options.material_namespace_paths,
                &mut report,
            );
            report.record_timing_ms("normalize_external_material_names", step_started);
        }
        if source_game == "fo76" && target_game == "fo4" {
            let step_started = Instant::now();
            prune_fo76_temp_ground_decal_shapes(&mut nif, &mut report);
            report.record_timing_ms("fo76_temp_ground_decal_shapes", step_started);
            let step_started = Instant::now();
            prune_fo76_flatwoods_skeleton_hand_helpers(&mut nif, &mut report);
            report.record_timing_ms("fo76_flatwoods_skeleton_hand_helpers", step_started);
        }
        if !skyrim_static {
            let step_started = Instant::now();
            normalize_texture_sets(
                &mut nif,
                &source_game,
                &target_game,
                options.asset_prefix.as_deref(),
                &options.asset_namespace_paths,
                &mut report,
            );
            report.record_timing_ms("normalize_texture_sets", step_started);
        }
        if source_game == "fo76" && target_game == "fo4" {
            let step_started = Instant::now();
            normalize_facegen_hair_shaders(&mut nif, &mut report);
            report.record_timing_ms("facegen_hair_shaders", step_started);
        }
    }
    let step_started = Instant::now();
    let preserve_scol_root_flags =
        source_game == "fo76" && target_game == "fo4" && is_scol_aggregate_nif(src, &nif);
    normalize_fo4_root_node(
        &mut nif,
        weapon_role,
        source_game == "fo76" && target_game == "fo4",
        preserve_scol_root_flags,
        &mut report,
    );
    report.record_timing_ms("normalize_fo4_root_node", step_started);
    if source_game == "fo76" && target_game == "fo4" {
        let step_started = Instant::now();
        normalize_fo76_fo4_scene_node_flags(&mut nif, &mut report);
        report.record_timing_ms("fo76_fo4_scene_node_flags", step_started);
    }
    let step_started = Instant::now();
    patch_addon_node_indices(&mut nif, &options.addon_index_map, &mut report);
    report.record_timing_ms("patch_addon_node_indices", step_started);
    if target_game == "fo4" {
        let step_started = Instant::now();
        mark_skinned_shape_shaders(&mut nif);
        report.record_timing_ms("mark_skinned_shape_shaders_final", step_started);
        let step_started = Instant::now();
        reconcile_havok_bsx_flags(&mut nif, &mut report);
        report.record_timing_ms("reconcile_havok_bsx_flags", step_started);
    }

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let save_started = Instant::now();
    let encode_started = Instant::now();
    let bytes = nif.to_bytes()?;
    report.record_timing_ms("save_encode", encode_started);
    let write_started = Instant::now();
    std::fs::write(dst, &bytes)?;
    report.record_timing_ms("save_write", write_started);
    nif.path = Some(PathBuf::from(dst));
    report.record_timing_ms("save", save_started);
    let first_person_started = Instant::now();
    emit_first_person_sibling(&nif, dst, &target_game, options, &mut report);
    report.record_timing_ms("emit_first_person_sibling", first_person_started);
    report.supported = true;
    report.record_timing_ms("total", total_started);
    Ok(report)
}

fn run_legacy_skin_conversion(
    nif: &mut NifFile,
    source_game: &str,
    target_game: &str,
    options: &ConvertFileOptions,
    report: &mut ConvertFileReport,
) -> Result<(), ConvertFileError> {
    if target_game != "fo4" || !matches!(source_game, "fnv" | "fo3") {
        return Ok(());
    }
    let Some(maps_dir) = options.translation_maps_dir.as_deref() else {
        return Ok(());
    };

    let skin_report = crate::skin::convert_legacy_skin_for_games(
        nif,
        maps_dir,
        source_game,
        target_game,
        options.auto_skin_reference_body.as_deref(),
        options.morph_weight_cap,
    )?;
    if skin_report.shapes_skinned > 0 {
        report.changes.push(format!(
            "Legacy skin -> FO4 skin: skinned {} shape(s), repacked {} vertex/vertices",
            skin_report.shapes_skinned, skin_report.vertices_repacked
        ));
    }
    report.shapes_skinned += skin_report.shapes_skinned;
    report.vertices_repacked += skin_report.vertices_repacked;
    report.bones_remapped += skin_report.bones_remapped;
    report.bones_dropped_unmapped += skin_report.bones_dropped_unmapped;
    report.weights_redistributed += skin_report.weights_redistributed;
    report.vertices_morph_weighted += skin_report.vertices_morph_weighted;
    report.warnings.extend(skin_report.warnings);
    Ok(())
}

fn emit_first_person_sibling(
    nif: &NifFile,
    dst: &Path,
    target_game: &str,
    options: &ConvertFileOptions,
    report: &mut ConvertFileReport,
) {
    if !options.emit_first_person || target_game != "fo4" {
        return;
    }

    match crate::skin::first_person::emit(nif, dst, options.first_person_reference.as_deref()) {
        Ok(Some(path)) => {
            report.emitted_first_person = Some(path);
            report
                .changes
                .push("Emitted first-person NIF sibling".to_string());
        }
        Ok(None) => report
            .warnings
            .push("emit_first_person requested but no arm-weighted geometry was found".to_string()),
        Err(error) => report
            .warnings
            .push(format!("first-person NIF emission failed: {error}")),
    }
}

/// Reconcile every `BSValueNode` addon-node block.
///
/// Runs unconditionally (not gated on a non-empty `index_map`) because FO76
/// names carry a `@#N` suffix that FO4 rejects — even a node with no index
/// remap must have its name normalized to plain `AddOnNode<digits>`.
///
/// * mapped (`index_map[old]` present) → `Value = new`, `Name = AddOnNode<new>`;
/// * unmapped → `Value = old`, `Name = AddOnNode<original_digits>` (strips the
///   `@…` suffix, preserves zero-padding). Already-clean FO4 nodes with matching
///   values are a no-op.
fn patch_addon_node_indices(
    nif: &mut NifFile,
    index_map: &HashMap<i64, i64>,
    report: &mut ConvertFileReport,
) {
    for block in &mut nif.blocks {
        if block.type_name != "BSValueNode" {
            continue;
        }
        let Some(name) = string_field(block, "Name") else {
            continue;
        };
        let Some((old_index, digits)) = addon_node_index(&name) else {
            continue;
        };
        match index_map.get(&old_index).copied() {
            Some(new_index) => {
                block.set_field("Value", NifValue::Int(new_index));
                block.set_field("Name", NifValue::String(format!("AddOnNode{new_index}")));
                report.changes.push(format!(
                    "BSValueNode AddOnNode{old_index} -> AddOnNode{new_index} (Value {old_index} -> {new_index})"
                ));
            }
            None => {
                let normalized = format!("AddOnNode{digits}");
                let current_value = int_field(block, "Value");
                if current_value != Some(old_index) {
                    block.set_field("Value", NifValue::Int(old_index));
                    report.changes.push(format!(
                        "BSValueNode {name} Value {} -> {old_index}",
                        current_value
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "<missing>".to_string())
                    ));
                }
                if normalized != name {
                    report.changes.push(format!(
                        "BSValueNode {name} -> {normalized} (@ suffix stripped)"
                    ));
                    block.set_field("Name", NifValue::String(normalized));
                }
            }
        }
    }
}

/// Parse a `BSValueNode` name of the form `AddOnNode<digits>[@…]`.
///
/// Returns `(index, digit_substring)` where the digit substring preserves the
/// original zero-padding and excludes any FO76-only `@…` suffix (e.g.
/// `"AddOnNode078@#0"` → `(78, "078")`). Returns `None` when the name isn't an
/// addon node or carries no leading digits.
fn addon_node_index(name: &str) -> Option<(i64, &str)> {
    let prefix = "addonnode";
    if !name.get(..prefix.len())?.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = name.get(prefix.len()..)?;
    let digit_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digit_len == 0 {
        return None;
    }
    let digits = &rest[..digit_len];
    let index = digits.parse::<i64>().ok()?;
    Some((index, digits))
}

fn copy_file(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)?;
    Ok(())
}

fn has_nif_header(path: &Path) -> Result<bool, std::io::Error> {
    let bytes = std::fs::read(path)?;
    Ok(bytes.starts_with(b"Gamebryo File Format") || bytes.starts_with(b"NetImmerse File Format"))
}

fn normalize_game(game: &str) -> String {
    game.trim().to_ascii_lowercase().replace('-', "")
}

fn normalize_weapon_role(role: &str) -> &str {
    match role.trim().to_ascii_lowercase().as_str() {
        "melee" => "melee",
        "gun" => "gun",
        _ => "",
    }
}

fn retarget_header(nif: &mut NifFile, target_game: &str, report: &mut ConvertFileReport) {
    let target_header = NifFile::new(target_game).header;
    if nif.header.bs_version != target_header.bs_version {
        report.changes.push(format!(
            "BS version: {} -> {}",
            nif.header.bs_version, target_header.bs_version
        ));
        nif.header.bs_version = target_header.bs_version;
    }
    if nif.header.user_version != target_header.user_version {
        report.changes.push(format!(
            "User version: {} -> {}",
            nif.header.user_version, target_header.user_version
        ));
        nif.header.user_version = target_header.user_version;
    }
    if nif.header.version != target_header.version {
        report.changes.push(format!(
            "NIF version: {}.{}.{}.{} -> {}.{}.{}.{}",
            nif.header.version.0,
            nif.header.version.1,
            nif.header.version.2,
            nif.header.version.3,
            target_header.version.0,
            target_header.version.1,
            target_header.version.2,
            target_header.version.3
        ));
        nif.header.version = target_header.version;
    }
    nif.header.version_packed = target_header.version_packed;
    nif.header.header_string = target_header.header_string;
}

fn strip_fo76_position_data(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let all_position_ids: HashSet<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSPositionData")
        .map(|block| block.block_id)
        .collect();
    let preserve_shape_ids = particle_emitter_shape_ids(nif);
    let preserve_ids = position_data_refs_for_shapes(nif, &preserve_shape_ids, &all_position_ids);
    let remove_ids: HashSet<usize> = all_position_ids
        .difference(&preserve_ids)
        .copied()
        .collect();
    if remove_ids.is_empty() {
        if !preserve_ids.is_empty() {
            report.changes.push(format!(
                "Preserved {} FO76 BSPositionData block(s) referenced by particle emitter mesh(es)",
                preserve_ids.len()
            ));
        }
        return;
    }

    let mut detached_refs = 0usize;
    let mut detached_shape_ids: HashSet<usize> = HashSet::new();
    for block in nif.blocks.iter_mut() {
        let Some(NifValue::Array(extra_refs)) = block.get_field("Extra Data List") else {
            continue;
        };
        let mut filtered: Vec<NifValue> = Vec::with_capacity(extra_refs.len());
        for value in extra_refs {
            match value {
                NifValue::Ref(id) if *id >= 0 && remove_ids.contains(&(*id as usize)) => {
                    detached_refs += 1;
                }
                _ => filtered.push(value.clone()),
            }
        }
        if filtered.len() != extra_refs.len() {
            if is_bs_geometry_shape(block) {
                detached_shape_ids.insert(block.block_id);
            }
            block.set_field("Num Extra Data List", NifValue::UInt(filtered.len() as u64));
            block.set_field("Extra Data List", NifValue::Array(filtered));
        }
    }

    let mut cleared_shapes = 0usize;
    for block in nif.blocks.iter_mut() {
        if !detached_shape_ids.contains(&block.block_id) || shape_has_inline_geometry(block) {
            continue;
        }
        clear_shape_geometry_counts(block);
        cleared_shapes += 1;
    }

    let mut sorted_remove_ids: Vec<usize> = remove_ids.iter().copied().collect();
    sorted_remove_ids.sort_unstable();
    nif.remove_blocks(&sorted_remove_ids);
    report.changes.push(format!(
        "Removed {} FO76 BSPositionData block(s) from FO4 NIF output",
        sorted_remove_ids.len()
    ));
    if detached_refs > sorted_remove_ids.len() {
        report.warnings.push(format!(
            "Removed {detached_refs} BSPositionData extra-data reference(s)"
        ));
    }
    if cleared_shapes > 0 {
        report.changes.push(format!(
            "Cleared geometry counts on {cleared_shapes} FO76 position-data shape(s)"
        ));
    }
    if !preserve_ids.is_empty() {
        report.changes.push(format!(
            "Preserved {} FO76 BSPositionData block(s) referenced by particle emitter mesh(es)",
            preserve_ids.len()
        ));
    }
}

fn particle_emitter_shape_ids(nif: &NifFile) -> HashSet<usize> {
    let mut ids = HashSet::new();
    for block in &nif.blocks {
        if block.type_name != "NiPSysMeshEmitter" {
            continue;
        }
        let Some(NifValue::Array(meshes)) = block.get_field("Emitter Meshes") else {
            continue;
        };
        for value in meshes {
            if let NifValue::Ref(id) = value {
                if *id >= 0 {
                    ids.insert(*id as usize);
                }
            }
        }
    }
    ids
}

fn position_data_refs_for_shapes(
    nif: &NifFile,
    shape_ids: &HashSet<usize>,
    position_ids: &HashSet<usize>,
) -> HashSet<usize> {
    let mut ids = HashSet::new();
    for shape_id in shape_ids {
        let Some(shape) = nif.get_block(*shape_id) else {
            continue;
        };
        let Some(NifValue::Array(extra_refs)) = shape.get_field("Extra Data List") else {
            continue;
        };
        for value in extra_refs {
            if let NifValue::Ref(id) = value {
                if *id >= 0 && position_ids.contains(&(*id as usize)) {
                    ids.insert(*id as usize);
                }
            }
        }
    }
    ids
}

fn is_bs_geometry_shape(block: &NifBlock) -> bool {
    matches!(
        block.type_name.as_str(),
        "BSTriShape" | "BSSubIndexTriShape" | "BSDynamicTriShape" | "BSMeshLODTriShape"
    )
}

fn prune_fo76_temp_ground_decal_shapes(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let shape_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| is_bs_geometry_shape(block))
        .filter(|shape| shape_uses_material(nif, shape, is_fo76_temp_ground_decal_material))
        .map(|shape| shape.block_id)
        .collect();
    if shape_ids.is_empty() {
        return;
    }

    let ref_counts = block_ref_counts(nif);
    let mut remove_ids: HashSet<usize> = shape_ids.iter().copied().collect();
    let mut shape_names = Vec::new();
    for shape_id in &shape_ids {
        let Some(shape) = nif.get_block(*shape_id) else {
            continue;
        };
        if let Some(name) = string_field(shape, "Name") {
            shape_names.push(name);
        }
        for property_id in shape_property_refs(shape) {
            if ref_counts.get(&property_id).copied().unwrap_or_default() <= 1 {
                remove_ids.insert(property_id);
                if let Some(texture_set_id) = shader_texture_set_ref(nif, property_id) {
                    if ref_counts.get(&texture_set_id).copied().unwrap_or_default() <= 1 {
                        remove_ids.insert(texture_set_id);
                    }
                }
            }
        }
    }

    detach_child_refs(nif, &remove_ids);
    let removed_shapes = shape_ids.len();
    remove_blocks(nif, remove_ids);
    if shape_names.is_empty() {
        report.changes.push(format!(
            "Pruned {removed_shapes} FO76 temp ground decal BSTriShape(s)"
        ));
    } else {
        report.changes.push(format!(
            "Pruned {removed_shapes} FO76 temp ground decal BSTriShape(s): {}",
            shape_names.join(", ")
        ));
    }
}

fn prune_fo76_flatwoods_skeleton_hand_helpers(nif: &mut NifFile, report: &mut ConvertFileReport) {
    if !matches!(
        nif.blocks.first().and_then(|block| string_field(block, "Name")),
        Some(name) if name == "FlatwoodsMonsterExportRoot"
    ) {
        return;
    }

    let shape_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| is_bs_geometry_shape(block))
        .filter(|shape| !shape_has_skin(shape))
        .filter(|shape| field_ref(shape, "Collision Object").map_or(true, |id| id < 0))
        .filter(|shape| {
            matches!(
                string_field(shape, "Name").as_deref(),
                Some("R_Hand:0" | "R_Hand:1")
            )
        })
        .map(|shape| shape.block_id)
        .collect();
    if shape_ids.is_empty() {
        return;
    }

    let ref_counts = block_ref_counts(nif);
    let mut remove_ids = HashSet::new();
    let mut shape_names = Vec::new();
    for shape_id in &shape_ids {
        if let Some(shape) = nif.get_block(*shape_id) {
            if let Some(name) = string_field(shape, "Name") {
                shape_names.push(name);
            }
            for property_id in shape_property_refs(shape) {
                if ref_counts.get(&property_id).copied().unwrap_or_default() <= 2 {
                    collect_exclusive_ref_subtree(nif, property_id, &ref_counts, &mut remove_ids);
                }
            }
        }
        remove_ids.insert(*shape_id);
    }

    detach_child_refs(nif, &remove_ids);
    remove_blocks(nif, remove_ids);
    report.changes.push(format!(
        "Pruned {} Flatwoods skeleton hand helper shape(s): {}",
        shape_ids.len(),
        shape_names.join(", ")
    ));
}

fn collect_exclusive_ref_subtree(
    nif: &NifFile,
    block_id: usize,
    ref_counts: &HashMap<usize, usize>,
    out: &mut HashSet<usize>,
) {
    if !out.insert(block_id) {
        return;
    }

    let Some(block) = nif.get_block(block_id) else {
        return;
    };
    let mut refs = Vec::new();
    for value in block.fields.values() {
        collect_value_refs(value, &mut refs);
    }
    for ref_id in refs {
        if ref_counts.get(&ref_id).copied().unwrap_or_default() <= 1 {
            collect_exclusive_ref_subtree(nif, ref_id, ref_counts, out);
        }
    }
}

fn collect_value_refs(value: &NifValue, out: &mut Vec<usize>) {
    match value {
        NifValue::Ref(id) if *id >= 0 => out.push(*id as usize),
        NifValue::Array(values) => {
            for value in values {
                collect_value_refs(value, out);
            }
        }
        NifValue::Struct(fields) => {
            for value in fields.values() {
                collect_value_refs(value, out);
            }
        }
        _ => {}
    }
}

fn shape_uses_material(nif: &NifFile, shape: &NifBlock, predicate: impl Fn(&str) -> bool) -> bool {
    shape_property_refs(shape).into_iter().any(|property_id| {
        let Some(shader) = nif.get_block(property_id) else {
            return false;
        };
        if !matches!(
            shader.type_name.as_str(),
            "BSLightingShaderProperty" | "BSEffectShaderProperty"
        ) {
            return false;
        }
        string_field(shader, "Name").is_some_and(|name| predicate(&name))
    })
}

fn shape_property_refs(shape: &NifBlock) -> Vec<usize> {
    let mut ids = Vec::new();
    for field in ["Shader Property", "Alpha Property"] {
        if let Some(id) = field_ref(shape, field).filter(|id| *id >= 0) {
            ids.push(id as usize);
        }
    }
    for id in ref_array(shape.get_field("Properties")) {
        if id >= 0 {
            ids.push(id as usize);
        }
    }
    ids
}

fn shader_texture_set_ref(nif: &NifFile, shader_id: usize) -> Option<usize> {
    let shader = nif.get_block(shader_id)?;
    if shader.type_name != "BSLightingShaderProperty" {
        return None;
    }
    field_ref(shader, "Texture Set")
        .filter(|id| *id >= 0)
        .map(|id| id as usize)
}

fn is_fo76_temp_ground_decal_material(path: &str) -> bool {
    canonical_material_path(path)
        .to_ascii_lowercase()
        .replace('/', "\\")
        == FO76_TEMP_GROUND_DECAL_MATERIAL
}

fn block_ref_counts(nif: &NifFile) -> HashMap<usize, usize> {
    let schema = &*crate::schema::SCHEMA;
    let mut counts = HashMap::new();
    for block in &nif.blocks {
        for id in block.get_refs(schema) {
            if id >= 0 {
                *counts.entry(id as usize).or_insert(0) += 1;
            }
        }
    }
    counts
}

fn detach_child_refs(nif: &mut NifFile, remove_ids: &HashSet<usize>) {
    for block in nif.blocks.iter_mut() {
        let Some(NifValue::Array(children)) = block.get_field("Children").cloned() else {
            continue;
        };
        let original_len = children.len();
        let filtered: Vec<NifValue> = children
            .into_iter()
            .filter(|value| {
                match value_ref(Some(value)).and_then(|id| (id >= 0).then_some(id as usize)) {
                    Some(id) => !remove_ids.contains(&id),
                    None => true,
                }
            })
            .collect();
        if filtered.len() != original_len {
            block.set_field("Num Children", NifValue::UInt(filtered.len() as u64));
            block.set_field("Children", NifValue::Array(filtered));
        }
    }
}

fn shape_has_inline_geometry(block: &NifBlock) -> bool {
    [
        "Vertex Data",
        "Triangles",
        "Particle Vertices",
        "Particle Normals",
        "Particle Triangles",
    ]
    .iter()
    .any(|field| matches!(block.get_field(field), Some(NifValue::Array(values)) if !values.is_empty()))
}

fn clear_shape_geometry_counts(block: &mut NifBlock) {
    block.set_field("Vertex Desc", NifValue::UInt(0));
    block.set_field("Num Triangles", NifValue::UInt(0));
    block.set_field("Num Vertices", NifValue::UInt(0));
    block.set_field("Data Size", NifValue::UInt(0));
}

fn strips_to_tri_shape(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let strip_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiTriStrips")
        .map(|block| block.block_id)
        .collect();
    if strip_ids.is_empty() {
        return;
    }

    let mut remove: HashSet<usize> = HashSet::new();
    let mut converted = 0usize;
    for strip_id in strip_ids {
        let Some(strip) = nif.get_block(strip_id).cloned() else {
            continue;
        };
        let Some(data_ref) = field_ref(&strip, "Data").filter(|id| *id >= 0) else {
            report
                .warnings
                .push(format!("NiTriStrips block {strip_id}: missing Data ref"));
            continue;
        };
        let Some(data) = nif.get_block(data_ref as usize).cloned() else {
            report.warnings.push(format!(
                "NiTriStrips block {strip_id}: invalid Data ref {data_ref}"
            ));
            continue;
        };
        if data.type_name != "NiTriStripsData" {
            report.warnings.push(format!(
                "NiTriStrips block {strip_id}: Data ref {data_ref} is {}",
                data.type_name
            ));
            continue;
        }

        let strip_lengths = int_array(data.get_field("Strip Lengths"));
        let strip_points = normalize_strip_points(data.get_field("Points"), &strip_lengths);
        let triangles = decode_strips(&strip_points);
        let vertex_data = build_vertex_data(&data);
        let has_vertex_colors = vertex_data.iter().any(|value| match value {
            NifValue::Struct(fields) => fields.contains_key("Vertex Colors"),
            _ => false,
        });

        let mut shader_ref = -1;
        let mut alpha_ref = -1;
        for prop_ref in ref_array(strip.get_field("Properties")) {
            let Some(prop) = nif.get_block(prop_ref as usize) else {
                continue;
            };
            match prop.type_name.as_str() {
                "NiAlphaProperty" => alpha_ref = prop_ref,
                "TallGrassShaderProperty"
                | "BSShaderPPLightingProperty"
                | "BSLightingShaderProperty"
                | "BSEffectShaderProperty"
                | "Lighting30ShaderProperty" => shader_ref = prop_ref,
                "NiStencilProperty" => {
                    remove.insert(prop.block_id);
                }
                _ => {}
            }
        }

        let mut fields = IndexMap::new();
        fields.insert("Name".to_string(), cloned_field(&strip, "Name"));
        fields.insert("Controller".to_string(), cloned_field(&strip, "Controller"));
        fields.insert("Flags".to_string(), NifValue::UInt(14));
        fields.insert(
            "Translation".to_string(),
            cloned_field(&strip, "Translation"),
        );
        fields.insert("Rotation".to_string(), cloned_field(&strip, "Rotation"));
        fields.insert(
            "Scale".to_string(),
            strip
                .get_field("Scale")
                .cloned()
                .unwrap_or(NifValue::Float(1.0)),
        );
        fields.insert(
            "Collision Object".to_string(),
            strip
                .get_field("Collision Object")
                .cloned()
                .unwrap_or(NifValue::Ref(-1)),
        );
        fields.insert(
            "Bounding Sphere".to_string(),
            cloned_field(&data, "Bounding Sphere"),
        );
        fields.insert(
            "Skin".to_string(),
            strip
                .get_field("Skin Instance")
                .or_else(|| strip.get_field("Skin"))
                .cloned()
                .unwrap_or(NifValue::Ref(-1)),
        );
        fields.insert("Shader Property".to_string(), NifValue::Ref(shader_ref));
        fields.insert("Alpha Property".to_string(), NifValue::Ref(alpha_ref));
        fields.insert(
            "Vertex Desc".to_string(),
            NifValue::Int(vertex_desc(has_vertex_colors)),
        );
        fields.insert(
            "Num Triangles".to_string(),
            NifValue::UInt(triangles.len() as u64),
        );
        fields.insert(
            "Num Vertices".to_string(),
            NifValue::UInt(vertex_data.len() as u64),
        );
        fields.insert("Vertex Data".to_string(), NifValue::Array(vertex_data));
        fields.insert("Triangles".to_string(), NifValue::Array(triangles));

        let new_shape_id = nif.add_block("BSTriShape", Some(fields));
        replace_child_ref(nif, strip_id as i32, new_shape_id as i32);
        remove.insert(strip_id);
        remove.insert(data_ref as usize);
        converted += 1;
    }

    remove_blocks(nif, remove);
    if converted > 0 {
        report.changes.push(format!(
            "NiTriStrips -> BSTriShape: converted {converted} shape(s)"
        ));
    }
}

fn vertex_desc(has_vertex_colors: bool) -> i64 {
    let stride = if has_vertex_colors { 6 } else { 5 };
    let mut flags = VF_VERTEX | VF_UVS | VF_NORMALS | VF_TANGENTS;
    let mut color_offset = 0;
    if has_vertex_colors {
        flags |= VF_VERTEX_COLORS;
        color_offset = 5;
    }
    stride | (2 << 8) | (3 << 16) | (4 << 20) | (color_offset << 24) | (flags << 44)
}

fn normalize_strip_points(raw: Option<&NifValue>, strip_lengths: &[usize]) -> Vec<Vec<usize>> {
    let Some(NifValue::Array(items)) = raw else {
        return Vec::new();
    };
    if items.iter().all(|item| matches!(item, NifValue::Array(_))) {
        return items
            .iter()
            .map(|item| match item {
                NifValue::Array(points) => points.iter().filter_map(value_usize).collect(),
                _ => Vec::new(),
            })
            .collect();
    }
    let flat: Vec<usize> = items.iter().filter_map(value_usize).collect();
    let mut strips = Vec::new();
    let mut offset = 0usize;
    for length in strip_lengths {
        let end = offset.saturating_add(*length).min(flat.len());
        strips.push(flat[offset..end].to_vec());
        offset = end;
    }
    strips
}

fn decode_strips(strip_points: &[Vec<usize>]) -> Vec<NifValue> {
    let mut triangles = Vec::new();
    for points in strip_points {
        for index in 0..points.len().saturating_sub(2) {
            let a = points[index];
            let b = points[index + 1];
            let c = points[index + 2];
            if a == b || b == c || a == c {
                continue;
            }
            if index % 2 == 0 {
                triangles.push(triangle(a as i64, b as i64, c as i64));
            } else {
                triangles.push(triangle(b as i64, a as i64, c as i64));
            }
        }
    }
    triangles
}

fn build_vertex_data(data: &NifBlock) -> Vec<NifValue> {
    let vertices = value_array(data.get_field("Vertices"));
    let normals = value_array(data.get_field("Normals"));
    let tangents = value_array(data.get_field("Tangents"));
    let bitangents = value_array(data.get_field("Bitangents"));
    let uv_sets = value_array(data.get_field("UV Sets"));
    let uvs = uv_sets
        .first()
        .and_then(|value| match value {
            NifValue::Array(items) => Some(items.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let vertex_colors = value_array(data.get_field("Vertex Colors"));

    let mut packed = Vec::new();
    for index in 0..vertices.len() {
        let bitangent = vec3_value(bitangents.get(index)).unwrap_or([0.0, 0.0, 0.0]);
        let mut entry = IndexMap::new();
        entry.insert(
            "Vertex".to_string(),
            NifValue::Vec3(vec3_value(vertices.get(index)).unwrap_or([0.0, 0.0, 0.0])),
        );
        entry.insert(
            "Bitangent X".to_string(),
            NifValue::Float(bitangent[0] as f64),
        );
        entry.insert(
            "UV".to_string(),
            tex_coord_value(uvs.get(index)).unwrap_or_else(|| tex_coord([0.0, 0.0])),
        );
        entry.insert(
            "Normal".to_string(),
            NifValue::Vec3(vec3_value(normals.get(index)).unwrap_or([0.0, 0.0, 1.0])),
        );
        entry.insert(
            "Bitangent Y".to_string(),
            NifValue::Float(bitangent[1] as f64),
        );
        entry.insert(
            "Tangent".to_string(),
            NifValue::Vec3(vec3_value(tangents.get(index)).unwrap_or([1.0, 0.0, 0.0])),
        );
        entry.insert(
            "Bitangent Z".to_string(),
            NifValue::Float(bitangent[2] as f64),
        );
        if let Some(color) = vertex_colors.get(index).and_then(color4_value) {
            entry.insert("Vertex Colors".to_string(), color);
        }
        packed.push(NifValue::Struct(entry));
    }
    packed
}

fn legacy_shader_to_lighting(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let direct_materials = pair_direct_materials(nif);
    let mut remap: HashMap<usize, usize> = HashMap::new();
    let mut remove: HashSet<usize> = HashSet::new();
    let mut converted_grass = 0usize;
    let mut converted_pp = 0usize;

    let shader_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "TallGrassShaderProperty" | "BSShaderPPLightingProperty"
            )
        })
        .map(|block| block.block_id)
        .collect();

    for shader_id in shader_ids {
        let Some(shader) = nif.get_block(shader_id).cloned() else {
            continue;
        };
        let new_id = if shader.type_name == "TallGrassShaderProperty" {
            converted_grass += 1;
            convert_tall_grass(nif, &shader)
        } else {
            converted_pp += 1;
            let lighting_id = convert_pp_lighting(nif, &shader);
            if let Some(material_id) = direct_materials.get(&shader.block_id) {
                if let Some(material) = nif.get_block(*material_id).cloned() {
                    if let Some(lighting) = nif.blocks.get_mut(lighting_id) {
                        apply_material(lighting, &material);
                    }
                }
            }
            lighting_id
        };
        remap.insert(shader_id, new_id);
        remove.insert(shader_id);
    }

    for block in nif.blocks.iter_mut() {
        if !matches!(
            block.type_name.as_str(),
            "BSTriShape" | "BSSubIndexTriShape"
        ) {
            continue;
        }
        let Some(shader_ref) = field_ref(block, "Shader Property") else {
            continue;
        };
        if let Some(new_ref) = remap.get(&(shader_ref as usize)) {
            block.set_field("Shader Property", NifValue::Ref(*new_ref as i32));
        }
    }

    for block in nif.blocks.iter() {
        if block.type_name == "NiMaterialProperty" {
            remove.insert(block.block_id);
        }
    }
    remove_blocks(nif, remove);

    if converted_grass > 0 || converted_pp > 0 {
        report.changes.push(format!(
            "Legacy shader properties -> BSLightingShaderProperty: {converted_grass} grass + {converted_pp} pp-lighting"
        ));
    }
}

fn mark_skinned_shape_shaders(nif: &mut NifFile) {
    let shader_ids = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "BSTriShape" | "BSSubIndexTriShape"
            ) && shape_has_skin(block)
        })
        .filter_map(|block| field_ref(block, "Shader Property").filter(|id| *id >= 0))
        .map(|id| id as usize)
        .collect::<HashSet<_>>();

    for shader_id in shader_ids {
        let Some(shader) = nif.blocks.get_mut(shader_id) else {
            continue;
        };
        if !matches!(
            shader.type_name.as_str(),
            "BSLightingShaderProperty" | "BSEffectShaderProperty"
        ) {
            continue;
        }
        let flags = flag_names_to_bits(
            shader
                .fields
                .get("Shader Flags 1:FO4")
                .or_else(|| shader.get_field("Shader Flags 1")),
            true,
        ) | 0x02;
        shader.set_field("Shader Flags 1", NifValue::UInt(flags));
        shader
            .fields
            .insert("Shader Flags 1:FO4".to_string(), NifValue::UInt(flags));
    }
}

fn clear_static_shape_skinned_shader_flags(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut shape_shader_ids = HashSet::new();
    let mut skinned_shape_shader_ids = HashSet::new();
    for block in &nif.blocks {
        if !is_bs_geometry_shape(block) {
            continue;
        }
        let Some(shader_id) = field_ref(block, "Shader Property").filter(|id| *id >= 0) else {
            continue;
        };
        let shader_id = shader_id as usize;
        shape_shader_ids.insert(shader_id);
        if shape_has_skin(block) {
            skinned_shape_shader_ids.insert(shader_id);
        }
    }

    let mut cleared = 0usize;
    for shader_id in shape_shader_ids
        .difference(&skinned_shape_shader_ids)
        .copied()
    {
        let Some(shader) = nif.blocks.get_mut(shader_id) else {
            continue;
        };
        if !matches!(
            shader.type_name.as_str(),
            "BSLightingShaderProperty" | "BSEffectShaderProperty"
        ) {
            continue;
        }
        let flags = value_u64(shader.get_field("Shader Flags 1")).unwrap_or_default();
        if flags & SLSF1_SKINNED == 0 {
            continue;
        }
        let updated = flags & !SLSF1_SKINNED;
        shader.set_field("Shader Flags 1", NifValue::UInt(updated));
        if shader.fields.contains_key("Shader Flags 1:FO4") {
            shader
                .fields
                .insert("Shader Flags 1:FO4".to_string(), NifValue::UInt(updated));
        }
        cleared += 1;
    }

    if cleared > 0 {
        report.changes.push(format!(
            "Shader skin flags: cleared Skinned from {cleared} shader(s) used only by static shapes"
        ));
    }
}

fn shape_has_skin(block: &NifBlock) -> bool {
    if field_ref(block, "Skin").is_some_and(|id| id >= 0)
        || field_ref(block, "Skin Instance").is_some_and(|id| id >= 0)
    {
        return true;
    }
    if field_ref(block, "Skin").is_some_and(|id| id < 0)
        || field_ref(block, "Skin Instance").is_some_and(|id| id < 0)
    {
        return false;
    }
    block
        .get_field("Vertex Desc")
        .is_some_and(|value| ((value.as_i64() >> 44) & VF_SKINNED) != 0)
}

fn convert_tall_grass(nif: &mut NifFile, block: &NifBlock) -> usize {
    let texset_id =
        build_fo4_texture_set(nif, string_field(block, "File Name").unwrap_or_default());
    let mut fields = IndexMap::new();
    fields.insert(
        "Name".to_string(),
        block
            .get_field("Name")
            .cloned()
            .unwrap_or(NifValue::String(String::new())),
    );
    fields.insert("Shader Type".to_string(), NifValue::UInt(0));
    fields.insert("Texture Set".to_string(), NifValue::Ref(texset_id as i32));
    fields.insert(
        "Shader Flags 1".to_string(),
        NifValue::UInt(flag_names_to_bits(block.get_field("Shader Flags"), true)),
    );
    fields.insert(
        "Shader Flags 2".to_string(),
        NifValue::UInt(flag_names_to_bits(block.get_field("Shader Flags 2"), false)),
    );
    nif.add_block("BSLightingShaderProperty", Some(fields))
}

fn convert_pp_lighting(nif: &mut NifFile, block: &NifBlock) -> usize {
    let texset_ref = resolve_texset(nif, block);
    let mut fields = IndexMap::new();
    fields.insert(
        "Name".to_string(),
        block
            .get_field("Name")
            .cloned()
            .unwrap_or(NifValue::String(String::new())),
    );
    fields.insert("Shader Type".to_string(), NifValue::UInt(0));
    fields.insert("Texture Set".to_string(), NifValue::Ref(texset_ref));
    fields.insert(
        "Shader Flags 1".to_string(),
        NifValue::UInt(flag_names_to_bits(block.get_field("Shader Flags"), true)),
    );
    fields.insert(
        "Shader Flags 2".to_string(),
        NifValue::UInt(flag_names_to_bits(block.get_field("Shader Flags 2"), false)),
    );
    fields.insert(
        "Texture Clamp Mode".to_string(),
        block
            .get_field("Texture Clamp Mode")
            .cloned()
            .unwrap_or(NifValue::UInt(0)),
    );
    fields.insert(
        "Refraction Strength".to_string(),
        NifValue::Float(value_f64(block.get_field("Refraction Strength")).unwrap_or(0.0)),
    );
    nif.add_block("BSLightingShaderProperty", Some(fields))
}

fn build_fo4_texture_set(nif: &mut NifFile, slot_zero_path: String) -> usize {
    let mut textures = vec![NifValue::String(String::new()); FO4_TEXTURE_SLOT_COUNT];
    if !slot_zero_path.is_empty() {
        textures[0] = NifValue::String(slot_zero_path);
    }
    let mut fields = IndexMap::new();
    fields.insert(
        "Num Textures".to_string(),
        NifValue::UInt(FO4_TEXTURE_SLOT_COUNT as u64),
    );
    fields.insert("Textures".to_string(), NifValue::Array(textures));
    nif.add_block("BSShaderTextureSet", Some(fields))
}

fn resolve_texset(nif: &mut NifFile, block: &NifBlock) -> i32 {
    let Some(texset_ref) = field_ref(block, "Texture Set").filter(|id| *id >= 0) else {
        return -1;
    };
    let Some(texset) = nif.blocks.get_mut(texset_ref as usize) else {
        return -1;
    };
    if texset.type_name != "BSShaderTextureSet" {
        return -1;
    }
    resize_texture_set(texset);
    texset.block_id as i32
}

fn resize_texture_set(texset: &mut NifBlock) {
    let mut textures = value_array(texset.get_field("Textures"));
    textures.resize(FO4_TEXTURE_SLOT_COUNT, NifValue::String(String::new()));
    textures.truncate(FO4_TEXTURE_SLOT_COUNT);
    texset.set_field(
        "Num Textures",
        NifValue::UInt(FO4_TEXTURE_SLOT_COUNT as u64),
    );
    texset.set_field("Textures", NifValue::Array(textures));
}

fn pair_direct_materials(nif: &NifFile) -> HashMap<usize, usize> {
    let mut pairs = HashMap::new();
    let mut last_pp_id = None;
    let mut blocks = nif.blocks.clone();
    blocks.sort_by_key(|block| block.block_id);
    for block in blocks {
        if block.type_name == "BSShaderPPLightingProperty" {
            last_pp_id = Some(block.block_id);
            continue;
        }
        if block.type_name == "NiMaterialProperty" {
            if let Some(pp_id) = last_pp_id.take() {
                pairs.insert(pp_id, block.block_id);
            }
        }
    }
    pairs
}

fn apply_material(shader: &mut NifBlock, material: &NifBlock) {
    for (src, dst) in [
        ("Emissive Color", "Emissive Color"),
        ("Specular Color", "Specular Color"),
    ] {
        if let Some(value) = material.get_field(src).cloned() {
            shader.set_field(dst, value);
        }
    }
    if let Some(alpha) = value_f64(material.get_field("Alpha")) {
        shader.set_field("Alpha", NifValue::Float(alpha));
    }
    if let Some(glossiness) = value_f64(material.get_field("Glossiness")) {
        shader.set_field("Smoothness", NifValue::Float(glossiness / 100.0));
    }
    if let Some(emissive_mult) = value_f64(material.get_field("Emissive Mult")) {
        shader.set_field("Emissive Multiple", NifValue::Float(emissive_mult));
    }
}

fn fix_fo76_float_controllers(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut dropped_ids = HashSet::new();
    let mut effect_remapped = 0usize;
    let mut lighting_remapped = 0usize;

    for block in nif.blocks.iter_mut() {
        match block.type_name.as_str() {
            "BSEffectShaderPropertyFloatController" => {
                let Some(value) = value_u64(block.get_field("Controlled Variable")) else {
                    continue;
                };
                let Some(mapped) = (match value {
                    11 => Some(6),
                    12 => Some(7),
                    13 => Some(8),
                    14 => Some(9),
                    _ => None,
                }) else {
                    continue;
                };
                block.set_field("Controlled Variable", NifValue::UInt(mapped));
                effect_remapped += 1;
            }
            "BSLightingShaderPropertyFloatController" => {
                let Some(value) = value_u64(block.get_field("Controlled Variable")) else {
                    continue;
                };
                if value == 3 {
                    block.set_field("Controlled Variable", NifValue::UInt(11));
                    lighting_remapped += 1;
                } else if matches!(value, 4 | 13 | 14) {
                    dropped_ids.insert(block.block_id);
                }
            }
            _ => {}
        }
    }

    let mut rewired = 0usize;
    if !dropped_ids.is_empty() {
        let next_by_id: HashMap<usize, i32> = nif
            .blocks
            .iter()
            .filter(|block| dropped_ids.contains(&block.block_id))
            .map(|block| {
                (
                    block.block_id,
                    field_ref(block, "Next Controller").unwrap_or(-1),
                )
            })
            .collect();
        for block in nif.blocks.iter_mut() {
            for field_name in ["Controller", "Next Controller"] {
                let Some(cur) = field_ref(block, field_name) else {
                    continue;
                };
                if cur < 0 || !dropped_ids.contains(&(cur as usize)) {
                    continue;
                }
                let new_ref = next_alive_controller(cur, &dropped_ids, &next_by_id);
                if new_ref != cur {
                    block.set_field(field_name, NifValue::Ref(new_ref));
                    rewired += 1;
                }
            }
        }
        remove_blocks(nif, dropped_ids.clone());
    }

    if effect_remapped > 0 || lighting_remapped > 0 || !dropped_ids.is_empty() {
        report.changes.push(format!(
            "Float controllers: remapped {effect_remapped} effect + {lighting_remapped} lighting; removed {} orphan lighting controller(s) with unmapped FO76 Controlled Variable (rewired {rewired} chain link(s))",
            dropped_ids.len()
        ));
    }
}

fn next_alive_controller(
    mut reference: i32,
    dropped_ids: &HashSet<usize>,
    next_by_id: &HashMap<usize, i32>,
) -> i32 {
    let mut seen = HashSet::new();
    while reference >= 0
        && dropped_ids.contains(&(reference as usize))
        && seen.insert(reference as usize)
    {
        reference = *next_by_id.get(&(reference as usize)).unwrap_or(&-1);
    }
    reference
}

fn flatten_fo76_effect_shader(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut flattened = 0usize;
    let mut dropped_flags = 0usize;
    let mut cleared_remainders = 0usize;
    for block in nif.blocks.iter_mut() {
        if block.type_name != "BSEffectShaderProperty" {
            continue;
        }
        let Some(spd) = shader_property_data(block) else {
            continue;
        };
        for key in [
            "UV Offset",
            "UV Scale",
            "Source Texture",
            "Texture Clamp Mode",
            "Lighting Influence",
            "Env Map Min LOD",
            "Unused Byte",
            "Falloff Start Angle",
            "Falloff Stop Angle",
            "Falloff Start Opacity",
            "Falloff Stop Opacity",
            "Base Color",
            "Base Color Scale",
            "Soft Falloff Depth",
            "Greyscale Texture",
            "Env Map Texture",
            "Normal Texture",
            "Env Mask Texture",
            "Environment Map Scale",
        ] {
            if let Some(value) =
                struct_get(&spd, key).filter(|value| !matches!(value, NifValue::Null))
            {
                block.set_field(key, value.clone());
            }
        }
        let (flags1, flags2, dropped) = translate_fo76_crc_fields(&spd);
        if struct_contains(&spd, "SF1")
            || struct_contains(&spd, "Num SF1")
            || struct_contains(&spd, "SF2")
            || struct_contains(&spd, "Num SF2")
        {
            block.set_field("Shader Flags 1", NifValue::UInt(flags1));
            block.set_field("Shader Flags 2", NifValue::UInt(flags2));
            dropped_flags += dropped;
        }
        block.fields.shift_remove("Shader Property Data");
        if !block.remainder.is_empty() {
            block.remainder.clear();
            cleared_remainders += 1;
        }
        flattened += 1;
    }
    if flattened > 0 {
        let mut msg = format!(
            "BSEffectShaderProperty: flattened {flattened} FO76 'Shader Property Data' nested struct(s) to FO4 top-level fields"
        );
        if dropped_flags > 0 {
            msg.push_str(&format!(
                " (dropped {dropped_flags} FO76-only shader flag CRC(s))"
            ));
        }
        if cleared_remainders > 0 {
            msg.push_str(&format!(
                " (cleared {cleared_remainders} FO76 shader tail remainder(s))"
            ));
        }
        report.changes.push(msg);
    }
}

fn ensure_fo4_effect_shader_defaults(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut patched = 0usize;
    for block in nif.blocks.iter_mut() {
        if block.type_name != "BSEffectShaderProperty" {
            continue;
        }
        if block.get_field("Name").and_then(|value| match value {
            NifValue::String(name) => Some(name.trim_matches('\0').trim().is_empty()),
            _ => None,
        }) != Some(false)
        {
            continue;
        }

        let mut changed = false;
        changed |= set_missing_changed(block, "Shader Flags 1", NifValue::UInt(0x80000000));
        changed |= set_missing_changed(block, "Shader Flags 2", NifValue::UInt(0));
        changed |= set_missing_changed(block, "UV Offset", tex_coord([0.0, 0.0]));
        changed |= set_missing_changed(block, "UV Scale", tex_coord([1.0, 1.0]));
        changed |= set_missing_changed(block, "Source Texture", NifValue::String(String::new()));
        changed |= set_missing_changed(block, "Texture Clamp Mode", NifValue::UInt(3));
        changed |= set_missing_changed(block, "Lighting Influence", NifValue::UInt(255));
        changed |= set_missing_changed(block, "Env Map Min LOD", NifValue::UInt(0));
        changed |= set_missing_changed(block, "Unused Byte", NifValue::UInt(0));
        changed |= set_missing_changed(block, "Falloff Start Angle", NifValue::Float(1.0));
        changed |= set_missing_changed(block, "Falloff Stop Angle", NifValue::Float(1.0));
        changed |= set_missing_changed(block, "Falloff Start Opacity", NifValue::Float(1.0));
        changed |= set_missing_changed(block, "Falloff Stop Opacity", NifValue::Float(0.0));
        changed |= set_missing_changed(block, "Base Color", NifValue::Color4([1.0, 1.0, 1.0, 1.0]));
        changed |= set_missing_changed(block, "Base Color Scale", NifValue::Float(1.0));
        changed |= set_missing_changed(block, "Soft Falloff Depth", NifValue::Float(100.0));
        changed |= set_missing_changed(block, "Greyscale Texture", NifValue::String(String::new()));
        changed |= set_missing_changed(block, "Env Map Texture", NifValue::String(String::new()));
        changed |= set_missing_changed(block, "Normal Texture", NifValue::String(String::new()));
        changed |= set_missing_changed(block, "Env Mask Texture", NifValue::String(String::new()));
        changed |= set_missing_changed(block, "Environment Map Scale", NifValue::Float(1.0));
        if !block.remainder.is_empty() {
            block.remainder.clear();
            changed = true;
        }
        if changed {
            patched += 1;
        }
    }
    if patched > 0 {
        report.changes.push(format!(
            "BSEffectShaderProperty: filled FO4 defaults for {patched} external BGEM shader(s)"
        ));
    }
}

fn flatten_fo76_water_shader(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut flattened = 0usize;
    let mut dropped_flags = 0usize;
    let mut cleared_remainders = 0usize;
    for block in nif.blocks.iter_mut() {
        if block.type_name != "BSWaterShaderProperty" {
            continue;
        }

        dropped_flags += shader_crc_array(block.get_field("SF1")).len();
        dropped_flags += shader_crc_array(block.get_field("SF2")).len();
        let uv_offset =
            tex_coord_value(block.get_field("UV Offset")).unwrap_or_else(|| tex_coord([0.0, 0.0]));
        let uv_scale =
            tex_coord_value(block.get_field("UV Scale")).unwrap_or_else(|| tex_coord([1.0, 1.0]));
        let water_flags =
            value_u64(block.get_field("Water Shader Flags")).unwrap_or(FO4_WATER_SHADER_FLAGS);

        for key in ["Num SF1", "SF1", "Num SF2", "SF2", "Shader Property Data"] {
            block.fields.shift_remove(key);
        }
        block.set_field("Shader Flags 1", NifValue::UInt(FO4_WATER_SHADER_FLAGS_1));
        block.set_field("Shader Flags 2", NifValue::UInt(FO4_WATER_SHADER_FLAGS_2));
        block.set_field("UV Offset", uv_offset);
        block.set_field("UV Scale", uv_scale);
        block.set_field("Water Shader Flags", NifValue::UInt(water_flags));
        if !block.remainder.is_empty() {
            block.remainder.clear();
            cleared_remainders += 1;
        }
        flattened += 1;
    }
    if flattened > 0 {
        let mut msg = format!(
            "BSWaterShaderProperty: normalized {flattened} FO76 water shader block(s) to FO4 fields"
        );
        if dropped_flags > 0 {
            msg.push_str(&format!(
                " (dropped {dropped_flags} FO76-only water shader flag CRC(s))"
            ));
        }
        if cleared_remainders > 0 {
            msg.push_str(&format!(
                " (cleared {cleared_remainders} FO76 water shader tail remainder(s))"
            ));
        }
        report.changes.push(msg);
    }
}

// FO76's BSShaderType155 has no Parallax slot: Face/Skin/Hair Tint sit at
// 3/4/5 and Eye Envmap at 12, versus FO4's 4/5/6 and 16. Copying the raw
// value types skin-tint shapes as FO4 Face Tint, so the writer drops the
// type-5-conditional Skin Tint Color and facegen neck/body shapes render
// untinted, mismatching the face.
fn fo76_shader_type_to_fo4(shader_type: u64) -> u64 {
    match shader_type {
        3 => 4,   // Face Tint
        4 => 5,   // Skin Tint
        5 => 6,   // Hair Tint
        12 => 16, // Eye Envmap
        other => other,
    }
}

fn flatten_fo76_lighting_shader(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut flattened = 0usize;
    let mut dropped_flags = 0usize;
    let mut pbr_reweighted = 0usize;
    let mut cleared_remainders = 0usize;
    let mut types_renumbered = 0usize;
    for block in nif.blocks.iter_mut() {
        if block.type_name != "BSLightingShaderProperty" {
            continue;
        }
        let Some(spd) = shader_property_data(block) else {
            continue;
        };
        let is_pbr = shader_crc_array(struct_get(&spd, "SF1")).contains(&FO76_PBR_SHADER_FLAG_CRC)
            || shader_crc_array(struct_get(&spd, "SF2")).contains(&FO76_PBR_SHADER_FLAG_CRC);
        for key in [
            "Shader Type",
            "UV Offset",
            "UV Scale",
            "Texture Set",
            "Emissive Color",
            "Emissive Multiple",
            "Root Material",
            "Texture Clamp Mode",
            "Alpha",
            "Refraction Strength",
            "Smoothness",
            "Specular Color",
            "Specular Strength",
            "Subsurface Rolloff",
            "Rimlight Power",
            "Backlight Power",
            "Grayscale to Palette Scale",
            "Fresnel Power",
            "Wetness",
            "Skin Tint Color",
            "Skin Tint Alpha",
            "Hair Tint Color",
            "Max Passes",
            "Scale",
            "Parallax Inner Layer Thickness",
            "Parallax Refraction Scale",
            "Parallax Inner Layer Texture Scale",
            "Parallax Envmap Strength",
            "Sparkle Parameters",
            "Eye Cubemap Scale",
            "Left Eye Reflection Center",
            "Right Eye Reflection Center",
        ] {
            if let Some(value) =
                struct_get(&spd, key).filter(|value| !matches!(value, NifValue::Null))
            {
                block.set_field(key, value.clone());
            }
        }
        if is_pbr {
            if let Some(smoothness) = value_f64(struct_get(&spd, "Smoothness"))
                .or_else(|| value_f64(block.get_field("Smoothness")))
            {
                block.set_field("Smoothness", NifValue::Float(smoothness.clamp(0.0, 1.0)));
                pbr_reweighted += 1;
            }
        }
        if let Some(shader_type) = value_u64(block.get_field("Shader Type")) {
            let mapped = fo76_shader_type_to_fo4(shader_type);
            if mapped != shader_type {
                block.set_field("Shader Type", NifValue::UInt(mapped));
                types_renumbered += 1;
            }
        }
        // FO76 stores the skin tint as a single Color4; FO4 serializes
        // Color3 + a separate Skin Tint Alpha float for the same 16 bytes.
        let fo76_skin_tint = match block.get_field("Skin Tint Color") {
            Some(NifValue::Color4(rgba)) | Some(NifValue::Vec4(rgba)) => Some(*rgba),
            _ => None,
        };
        if let Some([r, g, b, a]) = fo76_skin_tint {
            block.set_field("Skin Tint Color", NifValue::Color3([r, g, b]));
            block.set_field("Skin Tint Alpha", NifValue::Float(a as f64));
        }
        let (flags1, flags2, dropped) = translate_fo76_crc_fields(&spd);
        if struct_contains(&spd, "SF1")
            || struct_contains(&spd, "Num SF1")
            || struct_contains(&spd, "SF2")
            || struct_contains(&spd, "Num SF2")
        {
            block.set_field("Shader Flags 1", NifValue::UInt(flags1));
            block.set_field("Shader Flags 2", NifValue::UInt(flags2));
            dropped_flags += dropped;
        }
        block.fields.shift_remove("Shader Property Data");
        if !block.remainder.is_empty() {
            block.remainder.clear();
            cleared_remainders += 1;
        }
        flattened += 1;
    }
    if flattened > 0 {
        let mut msg = format!(
            "BSLightingShaderProperty: flattened {flattened} FO76 'Shader Property Data' nested struct(s) to FO4 top-level fields"
        );
        if pbr_reweighted > 0 {
            msg.push_str(&format!(
                " ({pbr_reweighted} PBR material(s) kept in FO76 smoothness range)"
            ));
        }
        if types_renumbered > 0 {
            msg.push_str(&format!(
                " (renumbered {types_renumbered} FO76 shader type(s) to FO4 numbering)"
            ));
        }
        if dropped_flags > 0 {
            msg.push_str(&format!(
                " (dropped {dropped_flags} FO76-only shader flag CRC(s))"
            ));
        }
        if cleared_remainders > 0 {
            msg.push_str(&format!(
                " (cleared {cleared_remainders} FO76 shader tail remainder(s))"
            ));
        }
        report.changes.push(msg);
    }
}

fn shader_property_data(block: &NifBlock) -> Option<IndexMap<String, NifValue>> {
    match block.get_field("Shader Property Data")? {
        NifValue::Struct(fields) => Some(fields.clone()),
        _ => None,
    }
}

fn translate_fo76_crc_fields(spd: &IndexMap<String, NifValue>) -> (u64, u64, usize) {
    let (flags1_a, flags2_a, dropped_a) = translate_fo76_crc_flags(struct_get(spd, "SF1"));
    let (flags1_b, flags2_b, dropped_b) = translate_fo76_crc_flags(struct_get(spd, "SF2"));
    (
        flags1_a | flags1_b,
        flags2_a | flags2_b,
        dropped_a + dropped_b,
    )
}

fn struct_get<'a>(fields: &'a IndexMap<String, NifValue>, name: &str) -> Option<&'a NifValue> {
    fields.get(name).or_else(|| {
        fields
            .iter()
            .find(|(key, _)| key.split_once(':').map_or(key.as_str(), |(bare, _)| bare) == name)
            .map(|(_, value)| value)
    })
}

fn struct_contains(fields: &IndexMap<String, NifValue>, name: &str) -> bool {
    struct_get(fields, name).is_some()
}

fn translate_fo76_crc_flags(value: Option<&NifValue>) -> (u64, u64, usize) {
    let mut flags1 = 0u64;
    let mut flags2 = 0u64;
    let mut dropped = 0usize;
    for crc in shader_crc_array(value) {
        match fo76_crc_to_fo4_flag(crc) {
            Some((1, bit)) => flags1 |= 1u64 << bit,
            Some((2, bit)) => flags2 |= 1u64 << bit,
            _ => dropped += 1,
        }
    }
    (flags1, flags2, dropped)
}

fn shader_crc_array(value: Option<&NifValue>) -> Vec<u64> {
    match value {
        Some(NifValue::Array(items)) => items
            .iter()
            .filter_map(|item| value_u64(Some(item)))
            .collect(),
        Some(value) => value_u64(Some(value)).into_iter().collect(),
        None => Vec::new(),
    }
}

fn fo76_crc_to_fo4_flag(crc: u64) -> Option<(u8, u8)> {
    match crc & 0xFFFF_FFFF {
        3744563888 => Some((1, 1)),
        2333069810 => Some((1, 3)),
        442246519 => Some((1, 4)),
        2901038324 => Some((1, 5)),
        3980660124 => Some((1, 6)),
        2893749418 => Some((1, 7)),
        3448946507 => Some((1, 8)),
        1563274220 => Some((1, 9)),
        314919375 => Some((1, 10)),
        2548465567 => Some((1, 12)),
        1957349758 => Some((1, 15)),
        1264105798 => Some((1, 18)),
        1483897208 => Some((1, 21)),
        2262553490 => Some((1, 22)),
        3849131744 => Some((1, 26)),
        1576614759 => Some((1, 27)),
        2150459555 => Some((1, 29)),
        3503164976 => Some((1, 30)),
        1740048692 => Some((1, 31)),
        3166356979 => Some((2, 0)),
        2896726515 => Some((2, 2)),
        2994043788 => Some((2, 3)),
        759557230 => Some((2, 4)),
        348504749 => Some((2, 5)),
        2399422528 => Some((2, 6)),
        3196772338 => Some((2, 7)),
        2078326675 => Some((2, 17)),
        3473438218 => Some((2, 30)),
        _ => None,
    }
}

fn rewire_orphan_texture_sets(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let referenced: HashSet<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSLightingShaderProperty")
        .filter_map(|block| field_ref(block, "Texture Set"))
        .filter(|id| *id >= 0)
        .map(|id| id as usize)
        .collect();
    let mut paired = HashSet::new();
    let mut linked = 0usize;
    let block_ids: Vec<usize> = nif.blocks.iter().map(|block| block.block_id).collect();
    for (index, block_id) in block_ids.iter().copied().enumerate() {
        let Some(texset) = nif.get_block(block_id) else {
            continue;
        };
        if texset.type_name != "BSShaderTextureSet" || referenced.contains(&block_id) {
            continue;
        }
        let mut target_shader_id = None;
        for prior_id in block_ids[..index].iter().rev().copied() {
            let Some(candidate) = nif.get_block(prior_id) else {
                continue;
            };
            if candidate.type_name != "BSLightingShaderProperty"
                || paired.contains(&candidate.block_id)
                || field_ref(candidate, "Texture Set").is_some_and(|id| id >= 0)
            {
                continue;
            }
            target_shader_id = Some(candidate.block_id);
            break;
        }
        if let Some(shader_id) = target_shader_id {
            if let Some(shader) = nif.blocks.get_mut(shader_id) {
                shader.set_field("Texture Set", NifValue::Ref(block_id as i32));
                paired.insert(shader_id);
                linked += 1;
            }
        }
    }
    if linked > 0 {
        report.changes.push(format!(
            "Linked {linked} orphan BSShaderTextureSet block(s) to BSLightingShaderProperty"
        ));
    }
}

fn propagate_texture_sets_by_material(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut material_texture_sets: HashMap<String, i32> = HashMap::new();
    for block in &nif.blocks {
        if block.type_name != "BSLightingShaderProperty" {
            continue;
        }
        let Some(texset_id) = valid_shader_texture_set_ref(nif, block) else {
            continue;
        };
        let Some(material) = shader_material_key(block) else {
            continue;
        };
        material_texture_sets.entry(material).or_insert(texset_id);
    }

    let mut propagated = 0usize;
    for block in nif.blocks.iter_mut() {
        if block.type_name != "BSLightingShaderProperty" {
            continue;
        }
        if field_ref(block, "Texture Set").is_some_and(|id| id >= 0) {
            continue;
        }
        let Some(material) = shader_material_key(block) else {
            continue;
        };
        let Some(texset_id) = material_texture_sets.get(&material).copied() else {
            continue;
        };
        block.set_field("Texture Set", NifValue::Ref(texset_id));
        propagated += 1;
    }

    if propagated > 0 {
        report.changes.push(format!(
            "BSLightingShaderProperty: propagated material texture sets to {propagated} shader(s)"
        ));
    }
}

fn valid_shader_texture_set_ref(nif: &NifFile, shader: &NifBlock) -> Option<i32> {
    let texset_id = field_ref(shader, "Texture Set").filter(|id| *id >= 0)?;
    let texset = nif.get_block(texset_id as usize)?;
    (texset.type_name == "BSShaderTextureSet").then_some(texset_id)
}

fn shader_material_key(block: &NifBlock) -> Option<String> {
    if block.type_name != "BSLightingShaderProperty" {
        return None;
    }
    let name = string_field(block, "Name")?;
    let canonical = canonical_material_path(&name).to_ascii_lowercase();
    canonical.ends_with(".bgsm").then_some(canonical)
}

// Returns true when the NIF is a tree / plant / grass mesh that needs FO4's
// vegetation shader path. Triggers on a BSLeafAnimNode root (trees / plants)
// or any BSShaderTextureSet entry pointing into landscape/grass|plants|trees.
fn nif_looks_like_vegetation(nif: &NifFile) -> bool {
    if nif
        .blocks
        .iter()
        .any(|b| b.type_name == "BSLeafAnimNode" || b.type_name == "BSTreeNode")
    {
        return true;
    }
    for block in &nif.blocks {
        if block.type_name != "BSShaderTextureSet" {
            continue;
        }
        if let Some(NifValue::Array(textures)) = block.get_field("Textures") {
            for tex in textures {
                if let NifValue::String(s) = tex {
                    let lower = s.to_ascii_lowercase().replace('\\', "/");
                    if lower.contains("landscape/grass/")
                        || lower.contains("landscape/plants/")
                        || lower.contains("landscape/trees/")
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn ensure_fo4_lighting_shader_defaults(nif: &mut NifFile, report: &mut ConvertFileReport) {
    // Detect "vegetation" NIFs once per file: the FO4 vegetation shader path
    // uses Double_Sided on BSLightingShaderProperty. Tree_Anim and
    // Vertex_Colors are only valid when all shapes using the shader carry
    // vertex colors; otherwise the CK disables flutter animation and warns.
    // Signals: root node is BSLeafAnimNode (trees / plants), or any texture-set
    // entry references a `landscape/(grass|plants|trees)` path.
    let is_vegetation = nif_looks_like_vegetation(nif);
    let shader_vertex_color_counts = if is_vegetation {
        shader_vertex_color_counts(nif)
    } else {
        HashMap::new()
    };
    // Specular | Cast_Shadows | Own_Emit | ZBuffer_Test. External BGSMs use
    // FO4's default shader path; vanilla meshes still keep linked texture slots
    // populated as fallback data beside the BGSM material name.
    let default_sf1: u32 = 0x80400201;

    let mut patched = 0usize;
    let mut tail_patched = 0usize;
    let mut hair_type_patched = 0usize;
    for block in nif.blocks.iter_mut() {
        if block.type_name != "BSLightingShaderProperty" {
            continue;
        }
        if normalize_fo76_hair_tint_shader_type(block) {
            hair_type_patched += 1;
        }
        if ensure_fo4_lighting_shader_tail_fields(block) {
            tail_patched += 1;
        }
        if block.get_field("Name").and_then(|value| match value {
            NifValue::String(name) => Some(name.trim_matches('\0').trim().is_empty()),
            _ => None,
        }) != Some(false)
        {
            continue;
        }
        if block.get_field("Shader Flags 1").is_some()
            || block.get_field("Shader Flags 1:FO4").is_some()
        {
            continue;
        }
        let default_sf2: u32 = if is_vegetation {
            let mut flags = SLSF2_ZBUFFER_WRITE | SLSF2_DOUBLE_SIDED;
            if shader_all_referenced_shapes_have_vertex_colors(
                &shader_vertex_color_counts,
                block.block_id,
            ) {
                flags |= SLSF2_VERTEX_COLORS | SLSF2_TREE_ANIM;
            }
            flags
        } else {
            SLSF2_ZBUFFER_WRITE
        };

        set_missing(
            block,
            "Shader Type",
            NifValue::UInt(BSLSP_SHADER_TYPE_DEFAULT),
        );
        set_missing(block, "Shader Flags 1", NifValue::UInt(default_sf1 as u64));
        set_missing(block, "Shader Flags 2", NifValue::UInt(default_sf2 as u64));
        set_missing(block, "UV Offset", tex_coord([0.0, 0.0]));
        set_missing(block, "UV Scale", tex_coord([1.0, 1.0]));
        set_missing(block, "Texture Set", NifValue::Ref(-1));
        set_missing(block, "Emissive Color", NifValue::Color3([0.0, 0.0, 0.0]));
        set_missing(block, "Emissive Multiple", NifValue::Float(1.0));
        set_missing(block, "Root Material", NifValue::String(String::new()));
        set_missing(block, "Texture Clamp Mode", NifValue::UInt(3));
        set_missing(block, "Alpha", NifValue::Float(1.0));
        set_missing(block, "Refraction Strength", NifValue::Float(0.0));
        set_missing(block, "Smoothness", NifValue::Float(1.0));
        set_missing(block, "Specular Color", NifValue::Color3([1.0, 1.0, 1.0]));
        set_missing(block, "Specular Strength", NifValue::Float(1.0));
        set_missing(block, "Subsurface Rolloff", NifValue::Float(0.0));
        set_missing(block, "Rimlight Power", NifValue::Float(f32::MAX as f64));
        set_missing(block, "Backlight Power", NifValue::Float(0.0));
        set_missing(block, "Grayscale to Palette Scale", NifValue::Float(1.0));
        set_missing(block, "Fresnel Power", NifValue::Float(5.0));
        set_missing(block, "Wetness", default_fo4_wetness());
        patched += 1;
    }
    if patched > 0 {
        report.changes.push(format!(
            "BSLightingShaderProperty: filled FO4 defaults for {patched} external BGSM shader(s)"
        ));
    }
    if tail_patched > 0 {
        report.changes.push(format!(
            "BSLightingShaderProperty: normalized FO4 tail fields for {tail_patched} shader(s)"
        ));
    }
    if hair_type_patched > 0 {
        report.changes.push(format!(
            "BSLightingShaderProperty: normalized FO76 hair tint shader type for {hair_type_patched} shader(s)"
        ));
    }
}

fn shader_vertex_color_counts(nif: &NifFile) -> HashMap<usize, (usize, usize)> {
    let mut counts = HashMap::new();
    for block in &nif.blocks {
        if !is_bs_geometry_shape(block) {
            continue;
        }
        let Some(shader_id) = field_ref(block, "Shader Property").filter(|id| *id >= 0) else {
            continue;
        };
        let entry = counts.entry(shader_id as usize).or_insert((0usize, 0usize));
        entry.0 += 1;
        if shape_has_vertex_colors(block) {
            entry.1 += 1;
        }
    }
    counts
}

fn shader_all_referenced_shapes_have_vertex_colors(
    counts: &HashMap<usize, (usize, usize)>,
    shader_id: usize,
) -> bool {
    matches!(counts.get(&shader_id), Some((total, with_colors)) if *total > 0 && total == with_colors)
}

fn shape_has_vertex_colors(block: &NifBlock) -> bool {
    if value_u64(block.get_field("Vertex Desc"))
        .is_some_and(|desc| ((desc >> 44) & VF_VERTEX_COLORS as u64) != 0)
    {
        return true;
    }
    value_array(block.get_field("Vertex Data")).iter().any(
        |value| matches!(value, NifValue::Struct(fields) if fields.contains_key("Vertex Colors")),
    )
}

fn set_missing(block: &mut NifBlock, name: &str, value: NifValue) {
    if block.get_field(name).is_none() {
        block.set_field(name, value);
    }
}

fn set_missing_changed(block: &mut NifBlock, name: &str, value: NifValue) -> bool {
    if block.get_field(name).is_none() {
        block.set_field(name, value);
        true
    } else {
        false
    }
}

fn ensure_fo4_lighting_shader_tail_fields(block: &mut NifBlock) -> bool {
    let mut changed = false;
    changed |= set_missing_changed(block, "Subsurface Rolloff", NifValue::Float(0.0));
    changed |= set_missing_changed(block, "Rimlight Power", NifValue::Float(f32::MAX as f64));
    changed |= set_missing_changed(block, "Backlight Power", NifValue::Float(0.0));
    changed |= set_missing_changed(block, "Grayscale to Palette Scale", NifValue::Float(1.0));
    changed |= set_missing_changed(block, "Fresnel Power", NifValue::Float(5.0));
    changed |= ensure_fo4_wetness_fields(block);
    changed |= ensure_fo4_lighting_shader_conditional_fields(block);
    if !block.remainder.is_empty() {
        block.remainder.clear();
        changed = true;
    }
    changed
}

fn normalize_fo76_hair_tint_shader_type(block: &mut NifBlock) -> bool {
    if value_u64(block.get_field("Shader Type")) != Some(BSLSP_SHADER_TYPE_SKIN_TINT)
        || !fo76_lighting_shader_looks_like_hair(block)
    {
        return false;
    }

    block.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_HAIR_TINT));
    let mut changed = true;
    changed |= block.fields.shift_remove("Skin Tint Color").is_some();
    changed |= block.fields.shift_remove("Skin Tint Alpha").is_some();
    changed
}

fn fo76_lighting_shader_looks_like_hair(block: &NifBlock) -> bool {
    if block.get_field("Hair Tint Color").is_some() {
        return true;
    }

    ["Shader Flags 1", "Shader Flags 1:FO4"]
        .into_iter()
        .any(|field| flag_names_to_bits(block.fields.get(field), true) & SLSF1_HAIR != 0)
}

// FO4 BSLightingShaderProperty appends type-conditional fields after Wetness
// (nif.xml: cond="Shader Type == N"). The schema-driven writer only emits a
// field when it is present in the block map, so a flattened FO76 shader that
// lacks them serializes short and the FO4 engine over-reads into the next
// block -> string-pool corruption / CTD. Stamp the defaults the engine expects
// for whichever type is set.
fn ensure_fo4_lighting_shader_conditional_fields(block: &mut NifBlock) -> bool {
    let Some(shader_type) = value_u64(block.get_field("Shader Type")) else {
        return false;
    };
    let mut changed = false;
    match shader_type {
        1 => {
            // Environment Map
            changed |= set_missing_changed(block, "Environment Map Scale", NifValue::Float(1.0));
            changed |=
                set_missing_changed(block, "Use Screen Space Reflections", NifValue::Bool(false));
            changed |=
                set_missing_changed(block, "Wetness Control: Use SSR", NifValue::Bool(false));
        }
        BSLSP_SHADER_TYPE_SKIN_TINT => {
            // Skin Tint
            changed |=
                set_missing_changed(block, "Skin Tint Color", NifValue::Color3([1.0, 1.0, 1.0]));
            changed |= set_missing_changed(block, "Skin Tint Alpha", NifValue::Float(1.0));
        }
        BSLSP_SHADER_TYPE_HAIR_TINT => {
            // Hair Tint
            changed |=
                set_missing_changed(block, "Hair Tint Color", NifValue::Color3([1.0, 1.0, 1.0]));
        }
        7 => {
            // Parallax Occ
            changed |= set_missing_changed(block, "Max Passes", NifValue::Float(4.0));
            changed |= set_missing_changed(block, "Scale", NifValue::Float(1.0));
        }
        11 => {
            // MultiLayer Parallax
            changed |= set_missing_changed(
                block,
                "Parallax Inner Layer Thickness",
                NifValue::Float(5.0),
            );
            changed |=
                set_missing_changed(block, "Parallax Refraction Scale", NifValue::Float(0.25));
            changed |= set_missing_changed(
                block,
                "Parallax Inner Layer Texture Scale",
                tex_coord([1.0, 1.0]),
            );
            changed |= set_missing_changed(block, "Parallax Envmap Strength", NifValue::Float(1.0));
        }
        14 => {
            // Sparkle Snow
            changed |= set_missing_changed(
                block,
                "Sparkle Parameters",
                NifValue::Vec4([0.0, 0.0, 0.0, 0.0]),
            );
        }
        16 => {
            // Eye Envmap
            changed |= set_missing_changed(block, "Eye Cubemap Scale", NifValue::Float(1.3));
            changed |= set_missing_changed(
                block,
                "Left Eye Reflection Center",
                NifValue::Vec3([0.0, 0.0, 0.0]),
            );
            changed |= set_missing_changed(
                block,
                "Right Eye Reflection Center",
                NifValue::Vec3([0.0, 0.0, 0.0]),
            );
        }
        _ => {}
    }
    changed
}

fn ensure_fo4_wetness_fields(block: &mut NifBlock) -> bool {
    let defaults = [
        "Spec Scale",
        "Spec Power",
        "Min Var",
        "Env Map Scale",
        "Fresnel Power",
        "Metalness",
    ];
    if let Some(NifValue::Struct(data)) = block.fields.get_mut("Wetness") {
        let mut changed = false;
        for key in defaults {
            if !data.contains_key(key) {
                data.insert(key.to_string(), NifValue::Float(-1.0));
                changed = true;
            }
        }
        return changed;
    }
    block.set_field("Wetness", default_fo4_wetness());
    true
}

fn default_fo4_wetness() -> NifValue {
    let mut data = IndexMap::new();
    for key in [
        "Spec Scale",
        "Spec Power",
        "Min Var",
        "Env Map Scale",
        "Fresnel Power",
        "Metalness",
    ] {
        data.insert(key.to_string(), NifValue::Float(-1.0));
    }
    NifValue::Struct(data)
}

fn remap_fo76_texture_slots(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let emissive_texture_sets = emissive_lighting_texture_sets(nif);
    let mut remapped_count = 0usize;
    let mut glow_remap = 0usize;
    let mut non_emissive_lighting_dropped = 0usize;
    for block in nif.blocks.iter_mut() {
        if block.type_name != "BSShaderTextureSet" {
            continue;
        }
        let textures = value_array(block.get_field("Textures"));
        if textures.len() <= FO4_TEXTURE_SLOT_COUNT {
            continue;
        }
        let mut remapped = vec![NifValue::String(String::new()); FO4_TEXTURE_SLOT_COUNT];
        for index in 0..FO4_TEXTURE_SLOT_COUNT.min(textures.len()) {
            if index == 9 {
                continue;
            }
            remapped[index] = textures[index].clone();
        }
        if textures.get(9).is_some_and(non_empty_texture)
            && !remapped.get(7).is_some_and(non_empty_texture)
        {
            remapped[7] = textures[9].clone();
        }
        if textures.get(10).is_some_and(non_empty_texture) {
            if emissive_texture_sets.contains(&block.block_id)
                && !remapped.get(2).is_some_and(non_empty_texture)
            {
                remapped[2] = textures[10].clone();
                glow_remap += 1;
            } else {
                non_emissive_lighting_dropped += 1;
            }
        }
        block.set_field("Textures", NifValue::Array(remapped));
        block.set_field(
            "Num Textures",
            NifValue::UInt(FO4_TEXTURE_SLOT_COUNT as u64),
        );
        remapped_count += 1;
    }

    if remapped_count > 0 {
        report.changes.push(format!(
            "BSShaderTextureSet: FO76 texture slot remap on {remapped_count} set(s) (slot 9 reflectivity -> slot 7 specular)"
        ));
    }
    if glow_remap > 0 {
        report.changes.push(format!(
            "BSShaderTextureSet: FO76 emissive lighting slot 10 -> FO4 glow slot 2 on {glow_remap} set(s)"
        ));
    }
    if non_emissive_lighting_dropped > 0 {
        report.changes.push(format!(
            "BSShaderTextureSet: dropped non-emissive FO76 lighting slot 10 on {non_emissive_lighting_dropped} set(s)"
        ));
    }
}

fn normalize_external_bgsm_shader_data_with_overrides(
    nif: &mut NifFile,
    source_material_dir: Option<&Path>,
    material_source_overrides: &HashMap<String, String>,
    report: &mut ConvertFileReport,
) {
    let shader_texture_sets: Vec<(usize, String, Option<usize>)> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSLightingShaderProperty")
        .filter(|block| shader_uses_external_bgsm(block))
        .filter_map(|block| {
            let material_path = string_field(block, "Name")?;
            Some((
                block.block_id,
                material_path,
                valid_shader_texture_set_ref(nif, block).map(|texset_id| texset_id as usize),
            ))
        })
        .collect();

    let mut shaders_normalized = 0usize;
    let mut texture_sets_normalized = 0usize;
    let mut texture_sets_created = 0usize;
    let mut glow_flags_set = 0usize;
    for (shader_id, material_path, texset_id) in shader_texture_sets {
        let (source_material_path, source_overridden) =
            material_source_override_path(&material_path, material_source_overrides);
        // FO76 NIFs carry no shader flags. Read the source material to learn
        // whether the converted FO4 BGSM keeps an explicit glow map, and set the
        // matching shader flag only for that case. Synthesized FO76 `_l`
        // emission is disabled in material conversion because FO4 applies it
        // across the whole object surface.
        let wants_glow_map =
            material_yields_fo4_glow_map(&source_material_path, source_material_dir);
        let material_texture_paths =
            converted_source_bgsm_texture_paths(&source_material_path, source_material_dir);
        {
            let Some(shader) = nif.blocks.get_mut(shader_id) else {
                continue;
            };
            let mut shader_changed = false;
            // A glow-emitting material needs the Glow Shader type (2) so FO4
            // selects the glow-map technique and masks the emittance by the glow
            // texture. With Default (0) the engine ignores the glow map and
            // applies Own_Emit flat across the whole mesh (perkboard glowed solid
            // white; AutoDispenser solid red). Vanilla FO4 glow meshes
            // (EmergencyLightOn01, SecurityCamera01) confirm Glow Shader is the
            // expected type for material-backed glow shapes.
            let desired_shader_type = if wants_glow_map {
                BSLSP_SHADER_TYPE_GLOW
            } else {
                BSLSP_SHADER_TYPE_DEFAULT
            };
            if value_u64(shader.get_field("Shader Type")) != Some(desired_shader_type) {
                shader.set_field("Shader Type", NifValue::UInt(desired_shader_type));
                shader_changed = true;
            }
            if clear_shader_flag(shader, "Shader Flags 1", SLSF1_ENVIRONMENT_MAPPING) {
                shader_changed = true;
            }
            if clear_shader_flag(shader, "Shader Flags 1:FO4", SLSF1_ENVIRONMENT_MAPPING) {
                shader_changed = true;
            }
            for field in [
                "Environment Map Scale",
                "Use Screen Space Reflections",
                "Wetness Control",
                "Wetness Control: Use SSR",
            ] {
                if shader.fields.shift_remove(field).is_some() {
                    shader_changed = true;
                }
            }
            if let Some(clamp_mode) = external_bgsm_texture_clamp_mode(&source_material_path) {
                if value_u64(shader.get_field("Texture Clamp Mode")) != Some(clamp_mode) {
                    shader.set_field("Texture Clamp Mode", NifValue::UInt(clamp_mode));
                    shader_changed = true;
                }
            }
            if wants_glow_map {
                let mut set = set_shader_flag(shader, "Shader Flags 1", SLSF1_OWN_EMIT);
                set |= set_shader_flag(shader, "Shader Flags 1:FO4", SLSF1_OWN_EMIT);
                if set {
                    shader_changed = true;
                }
                let mut glow_set = set_shader_flag(shader, "Shader Flags 2", SLSF2_GLOW_MAP);
                glow_set |= set_shader_flag(shader, "Shader Flags 2:FO4", SLSF2_GLOW_MAP);
                if glow_set {
                    glow_flags_set += 1;
                    shader_changed = true;
                }
            } else {
                let mut cleared = clear_shader_flag(shader, "Shader Flags 1", SLSF1_OWN_EMIT);
                cleared |= clear_shader_flag(shader, "Shader Flags 1:FO4", SLSF1_OWN_EMIT);
                cleared |= clear_shader_flag(shader, "Shader Flags 2", SLSF2_GLOW_MAP);
                cleared |= clear_shader_flag(shader, "Shader Flags 2:FO4", SLSF2_GLOW_MAP);
                if cleared {
                    shader_changed = true;
                }
            }
            if shader_changed {
                shaders_normalized += 1;
            }
        }

        if let Some(texset_id) = texset_id {
            if let Some(texset) = nif.blocks.get_mut(texset_id) {
                if normalize_external_bgsm_texture_set(
                    texset,
                    &source_material_path,
                    wants_glow_map,
                    material_texture_paths.as_deref(),
                    source_overridden,
                ) {
                    texture_sets_normalized += 1;
                }
            }
            continue;
        }

        let Some(textures) = external_bgsm_fallback_textures(
            &source_material_path,
            wants_glow_map,
            material_texture_paths.as_deref(),
        ) else {
            continue;
        };
        let mut fields = IndexMap::new();
        fields.insert(
            "Num Textures".to_string(),
            NifValue::UInt(FO4_TEXTURE_SLOT_COUNT as u64),
        );
        fields.insert("Textures".to_string(), NifValue::Array(textures));
        let texset_id = nif.add_block("BSShaderTextureSet", Some(fields));
        if let Some(shader) = nif.blocks.get_mut(shader_id) {
            shader.set_field("Texture Set", NifValue::Ref(texset_id as i32));
            texture_sets_created += 1;
        }
    }

    if shaders_normalized > 0 {
        report.changes.push(format!(
            "BSLightingShaderProperty: normalized {shaders_normalized} external BGSM shader(s) to FO4 default shader path"
        ));
    }
    if texture_sets_normalized > 0 {
        report.changes.push(format!(
            "BSShaderTextureSet: normalized fallback texture slots for {texture_sets_normalized} external BGSM shader set(s)"
        ));
    }
    if texture_sets_created > 0 {
        report.changes.push(format!(
            "BSShaderTextureSet: created fallback texture slots for {texture_sets_created} external BGSM shader(s)"
        ));
    }
    if glow_flags_set > 0 {
        report.changes.push(format!(
            "BSLightingShaderProperty: set Glow_Map flag for {glow_flags_set} external BGSM shader(s) whose material emits a glow map"
        ));
    }
}

fn normalize_external_bgsm_texture_set(
    texset: &mut NifBlock,
    material_path: &str,
    wants_glow: bool,
    material_texture_paths: Option<&[(usize, String)]>,
    force_material_texture_paths: bool,
) -> bool {
    let original_len = value_array(texset.get_field("Textures")).len();
    let mut textures = value_array(texset.get_field("Textures"));
    let mut changed = original_len != FO4_TEXTURE_SLOT_COUNT
        || value_u64(texset.get_field("Num Textures")) != Some(FO4_TEXTURE_SLOT_COUNT as u64);
    textures.resize(FO4_TEXTURE_SLOT_COUNT, NifValue::String(String::new()));
    textures.truncate(FO4_TEXTURE_SLOT_COUNT);
    for texture in textures.iter_mut() {
        if !matches!(texture, NifValue::String(_)) {
            *texture = NifValue::String(String::new());
            changed = true;
        }
    }
    if let Some(fallbacks) = material_texture_paths {
        if textures.get(6).is_some_and(non_empty_texture) {
            textures[6] = NifValue::String(String::new());
            changed = true;
        }
        for (index, path) in fallbacks {
            if force_material_texture_paths || !textures.get(*index).is_some_and(non_empty_texture)
            {
                textures[*index] = NifValue::String(path.clone());
                changed = true;
            }
        }
    }
    if let Some(fallbacks) = external_bgsm_fallback_texture_paths(material_path) {
        for (index, path) in fallbacks {
            if !textures.get(index).is_some_and(non_empty_texture) {
                textures[index] = NifValue::String(path);
                changed = true;
            }
        }
    }
    if wants_glow {
        // Vanilla FO4 glow meshes bind the glow texture in slot 2 (named after
        // the diffuse base, X_d -> X_g, which may differ from the material
        // name). Populate it so the glow map renders alongside the Glow_Map flag.
        if !textures.get(2).is_some_and(non_empty_texture) {
            if let Some(glow) = glow_slot_from_diffuse(&textures) {
                textures[2] = NifValue::String(glow);
                changed = true;
            }
        }
    } else if textures.get(2).is_some_and(non_empty_texture) {
        textures[2] = NifValue::String(String::new());
        changed = true;
    }
    texset.set_field(
        "Num Textures",
        NifValue::UInt(FO4_TEXTURE_SLOT_COUNT as u64),
    );
    texset.set_field("Textures", NifValue::Array(textures));
    changed
}

fn material_source_override_path(
    material_path: &str,
    material_source_overrides: &HashMap<String, String>,
) -> (String, bool) {
    let canonical = canonical_material_path(material_path);
    let key = canonical.replace('\\', "/").to_ascii_lowercase();
    match material_source_overrides.get(&key) {
        Some(source) => (canonical_material_path(source), true),
        None => (canonical, false),
    }
}

fn external_bgsm_fallback_textures(
    material_path: &str,
    wants_glow: bool,
    material_texture_paths: Option<&[(usize, String)]>,
) -> Option<Vec<NifValue>> {
    let mut textures = vec![NifValue::String(String::new()); FO4_TEXTURE_SLOT_COUNT];
    let mut populated = false;
    if let Some(fallbacks) = material_texture_paths {
        for (index, path) in fallbacks {
            textures[*index] = NifValue::String(path.clone());
            populated = true;
        }
    }
    for (index, path) in external_bgsm_fallback_texture_paths(material_path)? {
        if !textures.get(index).is_some_and(non_empty_texture) {
            textures[index] = NifValue::String(path);
            populated = true;
        }
    }
    if wants_glow {
        if let Some(glow) = glow_slot_from_diffuse(&textures) {
            textures[2] = NifValue::String(glow);
            populated = true;
        }
    }
    populated.then_some(textures)
}

/// Derive the FO4 glow texture slot path (`X_g.dds`) from the diffuse slot
/// (`X_d.dds`) of a texture set. FO4 names the glow map after the diffuse base,
/// which can differ from the material name, so derive it from the actual
/// diffuse path rather than the BGSM name.
fn glow_slot_from_diffuse(textures: &[NifValue]) -> Option<String> {
    let diffuse = match textures.first() {
        Some(NifValue::String(path)) => path.trim_end_matches('\0').trim(),
        _ => return None,
    };
    let stem_len = diffuse.to_ascii_lowercase().strip_suffix("_d.dds")?.len();
    Some(format!("{}_g.dds", &diffuse[..stem_len]))
}

fn external_bgsm_fallback_texture_paths(material_path: &str) -> Option<Vec<(usize, String)>> {
    let base = external_bgsm_texture_base_path(material_path)?;
    Some(vec![
        (0, format!("{base}_d.dds")),
        (1, format!("{base}_n.dds")),
        (7, format!("{base}_s.dds")),
    ])
}

fn external_bgsm_texture_base_path(material_path: &str) -> Option<String> {
    let material_path = canonical_material_path(material_path);
    let lower = material_path.to_ascii_lowercase();
    if !lower.ends_with(".bgsm") || lower.starts_with("materials\\template\\") {
        return None;
    }
    let without_extension = &material_path[..material_path.len() - ".bgsm".len()];
    let relative = without_extension.strip_prefix("Materials\\")?;
    Some(format!("textures\\{relative}"))
}

fn external_bgsm_texture_clamp_mode(material_path: &str) -> Option<u64> {
    let lower = canonical_material_path(material_path).to_ascii_lowercase();
    if lower.starts_with("materials\\landscape\\rocks\\") {
        return Some(TEX_CLAMP_MODE_CLAMP_S_CLAMP_T);
    }
    None
}

fn shader_uses_external_bgsm(block: &NifBlock) -> bool {
    if block.type_name != "BSLightingShaderProperty" {
        return false;
    }
    string_field(block, "Name").is_some_and(|name| {
        canonical_material_path(&name)
            .to_ascii_lowercase()
            .ends_with(".bgsm")
    })
}

fn clear_shader_flag(block: &mut NifBlock, field: &str, flag: u64) -> bool {
    let Some(flags) = value_u64(block.get_field(field)) else {
        return false;
    };
    let updated = flags & !flag;
    if updated == flags {
        return false;
    }
    block.set_field(field, NifValue::UInt(updated));
    true
}

fn set_shader_flag(block: &mut NifBlock, field: &str, flag: u64) -> bool {
    let Some(flags) = value_u64(block.get_field(field)) else {
        return false;
    };
    let updated = flags | flag;
    if updated == flags {
        return false;
    }
    block.set_field(field, NifValue::UInt(updated));
    true
}

/// Reads the FO76 source BGSM referenced by `material_path` (resolved under the
/// source data root) and reports whether its FO4 conversion enables a glow map.
/// Returns false when no source dir is given, the file is missing, or it fails
/// to parse — the shader simply keeps the FO4 default (Glow_Map off).
fn material_yields_fo4_glow_map(material_path: &str, source_material_dir: Option<&Path>) -> bool {
    let Some((bytes, relative, _resolved)) = read_source_bgsm(material_path, source_material_dir)
    else {
        return false;
    };
    materials_native::bgsm::parse(&bytes)
        .map(|bgsm| materials_native::convert::source_bgsm_enables_fo4_glowmap(&bgsm, &relative))
        .unwrap_or(false)
}

fn converted_source_bgsm_texture_paths(
    material_path: &str,
    source_material_dir: Option<&Path>,
) -> Option<Vec<(usize, String)>> {
    let (bytes, relative, resolved) = read_source_bgsm(material_path, source_material_dir)?;
    let mut bgsm = materials_native::bgsm::parse(&bytes).ok()?;
    materials_native::convert::repair_missing_fo76_smoothspec_from_specular(
        &mut bgsm,
        &resolved,
        materials_native::convert::Game::Fo76,
        materials_native::convert::Game::Fo4,
    );
    let converted = materials_native::convert::downgrade_bgsm(
        bgsm,
        &relative,
        materials_native::convert::Game::Fo76,
        materials_native::convert::Game::Fo4,
    );
    let mut paths = Vec::new();
    push_bgsm_texture_slot(&mut paths, 0, &converted.DiffuseTexture);
    push_bgsm_texture_slot(&mut paths, 1, &converted.NormalTexture);
    push_bgsm_texture_slot(
        &mut paths,
        2,
        converted.GlowTexture.as_deref().unwrap_or_default(),
    );
    push_bgsm_texture_slot(&mut paths, 7, &converted.SmoothSpecTexture);
    (!paths.is_empty()).then_some(paths)
}

fn push_bgsm_texture_slot(paths: &mut Vec<(usize, String)>, slot: usize, path: &str) {
    let clean = path.trim_end_matches('\0').trim();
    if clean.is_empty() {
        return;
    }
    paths.push((slot, canonical_texture_path(clean, "", "")));
}

fn read_source_bgsm(
    material_path: &str,
    source_material_dir: Option<&Path>,
) -> Option<(Vec<u8>, String, PathBuf)> {
    let Some(dir) = source_material_dir else {
        return None;
    };
    let relative = canonical_material_path(material_path)
        .replace('\\', "/")
        .to_ascii_lowercase();
    let resolved = dir.join(&relative);
    let bytes = std::fs::read(&resolved).ok()?;
    Some((bytes, relative, resolved))
}

fn clear_fo76_invalid_environment_mapping(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut shader_ids = Vec::new();
    for block in &nif.blocks {
        if block.type_name != "BSLightingShaderProperty" {
            continue;
        }
        let flags = value_u64(block.get_field("Shader Flags 1")).unwrap_or(0);
        if flags & SLSF1_ENVIRONMENT_MAPPING == 0 {
            continue;
        }
        let Some(texset_id) = field_ref(block, "Texture Set").filter(|id| *id >= 0) else {
            continue;
        };
        let has_cubemap = nif.get_block(texset_id as usize).is_some_and(|texset| {
            texset.type_name == "BSShaderTextureSet"
                && value_array(texset.get_field("Textures"))
                    .get(4)
                    .is_some_and(non_empty_texture)
        });
        if !has_cubemap {
            shader_ids.push(block.block_id);
        }
    }

    let mut cleared = 0usize;
    for shader_id in shader_ids {
        let Some(shader) = nif.blocks.get_mut(shader_id) else {
            continue;
        };
        let flags = value_u64(shader.get_field("Shader Flags 1")).unwrap_or(0);
        let updated = flags & !SLSF1_ENVIRONMENT_MAPPING;
        if updated == flags {
            continue;
        }
        shader.set_field("Shader Flags 1", NifValue::UInt(updated));
        if shader.fields.contains_key("Shader Flags 1:FO4") {
            shader
                .fields
                .insert("Shader Flags 1:FO4".to_string(), NifValue::UInt(updated));
        }
        if value_u64(shader.get_field("Shader Type")) == Some(BSLSP_SHADER_TYPE_ENVIRONMENT_MAP) {
            shader.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_DEFAULT));
        }
        cleared += 1;
    }

    if cleared > 0 {
        report.changes.push(format!(
            "BSLightingShaderProperty: cleared Environment_Mapping on {cleared} FO76 shader(s) without FO4 cubemap texture"
        ));
    }
}

fn emissive_lighting_texture_sets(nif: &NifFile) -> HashSet<usize> {
    let mut texture_sets = HashSet::new();
    for block in &nif.blocks {
        if block.type_name != "BSLightingShaderProperty" || !lighting_shader_uses_emissive(block) {
            continue;
        }
        if let Some(texset_id) = field_ref(block, "Texture Set").filter(|id| *id >= 0) {
            texture_sets.insert(texset_id as usize);
        }
    }
    texture_sets
}

fn lighting_shader_uses_emissive(block: &NifBlock) -> bool {
    let flags1 = value_u64(block.get_field("Shader Flags 1")).unwrap_or(0);
    let flags2 = value_u64(block.get_field("Shader Flags 2")).unwrap_or(0);
    if flags1 & (1u64 << 22) != 0 || flags1 & (1u64 << 29) != 0 || flags2 & (1u64 << 6) != 0 {
        return true;
    }
    let multiple = value_f64(block.get_field("Emissive Multiple")).unwrap_or(0.0);
    if multiple <= 0.0 {
        return false;
    }
    vec3_value(block.get_field("Emissive Color"))
        .is_some_and(|color| color.iter().any(|channel| *channel > 0.0))
}

fn non_empty_texture(value: &NifValue) -> bool {
    matches!(value, NifValue::String(path) if !path.trim_matches('\0').trim().is_empty())
}

fn convert_fo76_havok_blobs(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut converted = 0usize;
    let mut failed = 0usize;
    let mut skipped_np_collision = 0usize;
    let np_collision_physics = np_collision_physics_block_ids(nif);
    for block in nif.blocks.iter_mut() {
        if !matches!(
            block.type_name.as_str(),
            "bhkPhysicsSystem" | "bhkRagdollSystem"
        ) {
            continue;
        }
        if block.type_name == "bhkPhysicsSystem" && np_collision_physics.contains(&block.block_id) {
            skipped_np_collision += 1;
            continue;
        }
        let Some(binary_data) = block.get_field("Binary Data").cloned() else {
            continue;
        };
        let bytes = match crate::cloth::byte_array_to_bytes(&binary_data) {
            Ok(bytes) if !bytes.is_empty() => bytes,
            Ok(_) => continue,
            Err(error) => {
                failed += 1;
                report.warnings.push(format!(
                    "Havok blobs: skipped {} block {} Binary Data ({error})",
                    block.type_name, block.block_id
                ));
                continue;
            }
        };
        let outcome =
            havok_native::api::havok_convert_bytes(&bytes, "fo4").map_err(|e| e.to_string());
        match outcome {
            Ok(out) => {
                block.set_field("Binary Data", crate::cloth::bytes_to_byte_array(&out));
                converted += 1;
            }
            Err(error) => {
                failed += 1;
                report.warnings.push(format!(
                    "Havok blobs: failed to convert {} block {} Binary Data ({error})",
                    block.type_name, block.block_id
                ));
            }
        }
    }
    if converted > 0 {
        report.changes.push(format!(
            "Havok blobs: converted {converted} embedded FO76 blob(s) to FO4"
        ));
    }
    if skipped_np_collision > 0 {
        report.changes.push(format!(
            "Havok blobs: skipped {skipped_np_collision} regenerated NP collision blob(s)"
        ));
    }
    if converted == 0 && failed > 0 {
        report.warnings.push(format!(
            "Havok blobs: {failed} embedded FO76 blob(s) could not be converted"
        ));
    }
}

/// Vanilla FO4 baked FaceGeom *main* hair uses the Glow Shader convention
/// (Glow Shader type + Own_Emit/Glow_Map, GreyscaleToPalette color gradient in
/// slot 3, flow + specular). FO76 hair converts as a loose-hair "Hair Tint"
/// shader with a glow-synthesized `_g` palette slot and a missing/`HairDefault`
/// specular, so it renders untextured / wrong-colored as baked hair. Detect
/// FaceGeom NIFs (root node `BSFaceGenNiNodeSkinned`) and rewrite each non-decal
/// hair shape to match vanilla. The hairline (a Decal) correctly stays Hair
/// Tint and is left alone. Cloth-bone folding is handled in
/// `convert_fo76_cloth_blobs`.
fn normalize_facegen_hair_shaders(nif: &mut NifFile, report: &mut ConvertFileReport) {
    if !nif_is_facegen(nif) {
        return;
    }
    let shape_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| is_bs_geometry_shape(block))
        .map(|block| block.block_id)
        .collect();

    let mut targets: Vec<(usize, Option<usize>, Option<usize>)> = Vec::new();
    for shape_id in shape_ids {
        let Some(shape) = nif.get_block(shape_id) else {
            continue;
        };
        let Some(shader_id) = field_ref(shape, "Shader Property")
            .filter(|id| *id >= 0)
            .map(|id| id as usize)
        else {
            continue;
        };
        let alpha_id = field_ref(shape, "Alpha Property")
            .filter(|id| *id >= 0)
            .map(|id| id as usize);
        let Some(shader) = nif.get_block(shader_id) else {
            continue;
        };
        if shader.type_name != "BSLightingShaderProperty" {
            continue;
        }
        let flags1 = value_u64(shader.get_field("Shader Flags 1")).unwrap_or(0);
        // Main hair only: Hair flag set and NOT a decal. The hairline is a decal
        // that renders correctly as Hair Tint and must stay that way.
        if flags1 & SLSF1_HAIR == 0 || flags1 & SLSF1_DECAL != 0 {
            continue;
        }
        let texset_id = field_ref(shader, "Texture Set")
            .filter(|id| *id >= 0)
            .map(|id| id as usize);
        targets.push((shader_id, texset_id, alpha_id));
    }
    if targets.is_empty() {
        return;
    }

    let count = targets.len();
    for (shader_id, texset_id, alpha_id) in targets {
        if let Some(shader) = nif.blocks.get_mut(shader_id) {
            shader.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_GLOW));
            let flags1 = value_u64(shader.get_field("Shader Flags 1")).unwrap_or(0)
                | SLSF1_SPECULAR
                | SLSF1_OWN_EMIT;
            shader.set_field("Shader Flags 1", NifValue::UInt(flags1));
            let flags2 = value_u64(shader.get_field("Shader Flags 2")).unwrap_or(0)
                | SLSF2_DOUBLE_SIDED as u64
                | SLSF2_VERTEX_COLORS as u64
                | SLSF2_GLOW_MAP
                | SLSF2_TRANSFORM_CHANGED;
            shader.set_field("Shader Flags 2", NifValue::UInt(flags2));
        }
        if let Some(texset) = texset_id.and_then(|id| nif.blocks.get_mut(id)) {
            normalize_facegen_hair_texture_set(texset);
        }
        if let Some(alpha) = alpha_id.and_then(|id| nif.blocks.get_mut(id)) {
            alpha.set_field("Flags", NifValue::UInt(FO4_HAIR_ALPHA_FLAGS));
            alpha.set_field("Threshold", NifValue::UInt(FO4_HAIR_ALPHA_THRESHOLD));
        }
    }
    report.changes.push(format!(
        "FaceGeom hair: normalized {count} baked-hair shape(s) to the FO4 Glow Shader convention"
    ));
}

fn nif_is_facegen(nif: &NifFile) -> bool {
    nif.blocks.iter().any(|block| {
        block.type_name == "NiNode"
            && string_field(block, "Name")
                .is_some_and(|name| name.trim_end_matches('\0') == "BSFaceGenNiNodeSkinned")
    })
}

/// Rewrite a baked-hair texture set to the vanilla slot convention: slot 3 =
/// `HairColor_LGrad_d` color gradient (FO76 converts it to a bogus `_g` glow
/// map), slot 2 = flow, slot 7 = specular (FO76 leaves a missing `HairDefault`).
/// slots 0/1 (grayscale diffuse + normal) already convert correctly.
fn normalize_facegen_hair_texture_set(texset: &mut NifBlock) {
    let mut slots = value_array(texset.get_field("Textures"));
    if slots.len() < FO4_TEXTURE_SLOT_COUNT {
        slots.resize(FO4_TEXTURE_SLOT_COUNT, NifValue::String(String::new()));
    }
    let slot_str = |slots: &[NifValue], index: usize| -> String {
        match slots.get(index) {
            Some(NifValue::String(value)) => value.clone(),
            _ => String::new(),
        }
    };

    // slot 3: the GreyscaleToPalette color gradient must be `_d`, not the
    // synthesized `_g` glow map (which does not exist for hair palettes).
    let palette = slot_str(&slots, 3);
    slots[3] = NifValue::String(if palette.is_empty() {
        FO4_HAIR_PALETTE.to_string()
    } else {
        retarget_texture_suffix(&palette, "_d")
    });

    // slots 2/7: derive flow + specular from the normal-map base "<dir>X_n".
    if let Some(base) = hair_texture_base(&slot_str(&slots, 1)) {
        if slot_str(&slots, 2).is_empty() {
            slots[2] = NifValue::String(format!("{base}_f.dds"));
        }
        let specular = slot_str(&slots, 7);
        if specular.is_empty() || specular.to_ascii_lowercase().contains("default") {
            slots[7] = NifValue::String(format!("{base}_s.dds"));
        }
    }

    texset.set_field(
        "Num Textures",
        NifValue::UInt(FO4_TEXTURE_SLOT_COUNT as u64),
    );
    texset.set_field("Textures", NifValue::Array(slots));
}

/// Replace a texture path's trailing `_<token>.dds` with `suffix`, preserving
/// directory + stem (e.g. `…HairColor_LGrad_g.DDS` -> `…HairColor_LGrad_d.DDS`).
fn retarget_texture_suffix(path: &str, suffix: &str) -> String {
    let (stem, ext) = match path.rfind('.') {
        Some(dot) => (&path[..dot], &path[dot..]),
        None => (path, ""),
    };
    match stem.rfind('_') {
        Some(underscore) => format!("{}{}{}", &stem[..underscore], suffix, ext),
        None => format!("{stem}{suffix}{ext}"),
    }
}

/// Return the "<dir>X" base of a `<dir>X_n.dds` normal-map path, or None.
fn hair_texture_base(normal_path: &str) -> Option<String> {
    let dot = normal_path.to_ascii_lowercase().rfind(".dds")?;
    let stem = &normal_path[..dot];
    let underscore = stem.rfind('_')?;
    stem[underscore..]
        .eq_ignore_ascii_case("_n")
        .then(|| stem[..underscore].to_string())
}

fn convert_fo76_cloth_blobs(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let cloth_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSClothExtraData")
        .map(|block| block.block_id)
        .collect();
    if cloth_ids.is_empty() {
        return;
    }

    let facegen = nif_is_facegen(nif);
    let mut converted = 0usize;
    let mut remove_ids = HashSet::new();
    for block_id in cloth_ids {
        if facegen {
            remove_ids.insert(block_id);
            continue;
        }
        let Some(binary_data) = nif
            .get_block(block_id)
            .and_then(|block| block.get_field("Binary Data"))
            .cloned()
        else {
            remove_ids.insert(block_id);
            report.warnings.push(format!(
                "Havok cloth: BSClothExtraData block {block_id} has no Binary Data"
            ));
            continue;
        };
        let source = match crate::cloth::byte_array_to_bytes(&binary_data) {
            Ok(bytes) if !bytes.is_empty() => bytes,
            Ok(_) => {
                remove_ids.insert(block_id);
                report.warnings.push(format!(
                    "Havok cloth: BSClothExtraData block {block_id} has empty Binary Data"
                ));
                continue;
            }
            Err(error) => {
                remove_ids.insert(block_id);
                report.warnings.push(format!(
                    "Havok cloth: BSClothExtraData block {block_id} Binary Data is invalid ({error})"
                ));
                continue;
            }
        };
        let conversion = match havok_native::api::havok_convert_bytes_report(&source, "fo4") {
            Ok(conversion) => conversion,
            Err(error) => {
                remove_ids.insert(block_id);
                report.warnings.push(format!(
                    "Havok cloth: BSClothExtraData block {block_id} could not be converted to FO4 ({error})"
                ));
                continue;
            }
        };
        if let Err(error) = validate_fo4_cloth_blob(&conversion.bytes) {
            remove_ids.insert(block_id);
            report.warnings.push(format!(
                "Havok cloth: BSClothExtraData block {block_id} produced invalid FO4 cloth ({error})"
            ));
            continue;
        }
        for warning in conversion.warnings {
            report.warnings.push(format!(
                "Havok cloth: BSClothExtraData block {block_id}: {warning}"
            ));
        }
        if let Some(block) = nif.blocks.get_mut(block_id) {
            block.set_field(
                "Binary Data",
                crate::cloth::bytes_to_byte_array(&conversion.bytes),
            );
            converted += 1;
        }
    }

    if converted > 0 {
        report.changes.push(format!(
            "Havok cloth: converted {converted} embedded FO76 cloth blob(s) to validated FO4 packfiles"
        ));
    }
    if !remove_ids.is_empty() {
        let removed_count = remove_ids.len();
        let detached_refs = detach_extra_data_refs(nif, &remove_ids);
        remove_blocks(nif, remove_ids);
        report.warnings.push(format!(
            "Havok cloth: stripped {removed_count} FO76 BSClothExtraData block(s) that could not be emitted safely for FO4"
        ));
        if detached_refs > 0 {
            report.changes.push(format!(
                "Havok cloth: detached {detached_refs} extra-data reference(s)"
            ));
        }
    }
    if !nif
        .blocks
        .iter()
        .any(|block| block.type_name == "BSClothExtraData")
    {
        fold_fo76_cloth_skin_bones(nif, report);
    }
}

fn validate_fo4_cloth_blob(blob: &[u8]) -> Result<(), String> {
    let format = havok_native::api::hkx_detect_format_full(blob).map_err(|error| error.to_string())?;
    if format.kind != "packfile" || format.version != "hk_2014.1.0-r1" {
        return Err(format!(
            "expected hk_2014.1.0-r1 packfile, got {} {}",
            format.kind, format.version
        ));
    }
    let summary = havok_native::api::hkx_class_summary(blob).map_err(|error| error.to_string())?;
    if !summary.has_cloth_data {
        return Err("converted packfile has no hclClothData".to_string());
    }
    let validation = havok_native::api::cloth_validate(blob).map_err(|error| error.to_string())?;
    let validation: serde_json::Value =
        serde_json::from_str(&validation).map_err(|error| error.to_string())?;
    if validation.get("valid").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err("cloth validation reported errors".to_string());
    }
    Ok(())
}

fn normalize_fo76_headwear_segments(nif: &mut NifFile, report: &mut ConvertFileReport) {
    if nif_is_facegen(nif) {
        return;
    }

    let shape_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSSubIndexTriShape")
        .filter(|block| shape_has_skin(block))
        .filter(|block| shape_is_head_only_skinned(nif, block))
        .filter(|block| segment_data_has_user_index(block, 32))
        .map(|block| block.block_id)
        .collect();
    if shape_ids.is_empty() {
        return;
    }

    let mut changed_shapes = 0usize;
    let mut changed_entries = 0usize;
    for shape_id in shape_ids {
        let Some(shape) = nif.blocks.get_mut(shape_id) else {
            continue;
        };
        let changed = remap_segment_data_user_index(shape, 32, 30);
        if changed == 0 {
            continue;
        }
        changed_shapes += 1;
        changed_entries += changed;
    }

    if changed_entries > 0 {
        report.changes.push(format!(
            "FO76 headwear segments: remapped {changed_entries} segment user-index entry/entries from 32 to 30 across {changed_shapes} shape(s)"
        ));
    }
}

fn shape_is_head_only_skinned(nif: &NifFile, shape: &NifBlock) -> bool {
    let Some(skin_id) = shape_skin_id(shape) else {
        return false;
    };
    let Some(skin) = nif.get_block(skin_id) else {
        return false;
    };
    let bone_refs = ref_array(skin.get_field("Bones"));
    if bone_refs.is_empty() {
        return false;
    }

    let mut has_head_bone = false;
    for bone_ref in bone_refs {
        let Some(bone) = (bone_ref >= 0)
            .then_some(bone_ref as usize)
            .and_then(|id| nif.get_block(id))
        else {
            return false;
        };
        let Some(name) = string_field(bone, "Name") else {
            return false;
        };
        if !is_headwear_skin_bone_name(&name) {
            return false;
        }
        has_head_bone |= is_headwear_anchor_bone_name(&name);
    }
    has_head_bone
}

fn shape_skin_id(shape: &NifBlock) -> Option<usize> {
    field_ref(shape, "Skin")
        .or_else(|| field_ref(shape, "Skin Instance"))
        .filter(|id| *id >= 0)
        .map(|id| id as usize)
}

fn is_headwear_skin_bone_name(name: &str) -> bool {
    let name = name.trim_end_matches('\0').to_ascii_lowercase();
    is_headwear_anchor_bone_name(&name)
        || name.starts_with("neck")
        || matches!(
            name.as_str(),
            "chest"
                | "chest_skin"
                | "chest_rear_skin"
                | "larm_collarbone"
                | "larm_collarbone_skin"
                | "rarm_collarbone"
                | "rarm_collarbone_skin"
        )
}

fn is_headwear_anchor_bone_name(name: &str) -> bool {
    let name = name.trim_end_matches('\0').to_ascii_lowercase();
    matches!(name.as_str(), "head" | "head_skin")
}

fn segment_data_has_user_index(shape: &NifBlock, user_index: u64) -> bool {
    let Some(NifValue::Struct(fields)) = shape.get_field("Segment Data") else {
        return false;
    };
    let Some(NifValue::Array(entries)) = fields.get("Per Segment Data") else {
        return false;
    };
    entries.iter().any(|entry| match entry {
        NifValue::Struct(fields) => value_u64(fields.get("User Index")) == Some(user_index),
        _ => false,
    })
}

fn remap_segment_data_user_index(shape: &mut NifBlock, from: u64, to: u64) -> usize {
    let Some(mut segment_data) = shape.get_field("Segment Data").cloned() else {
        return 0;
    };
    let changed = remap_user_index_in_segment_data(&mut segment_data, from, to);
    if changed > 0 {
        shape.set_field("Segment Data", segment_data);
    }
    changed
}

fn remap_user_index_in_segment_data(segment_data: &mut NifValue, from: u64, to: u64) -> usize {
    let NifValue::Struct(fields) = segment_data else {
        return 0;
    };
    let Some(NifValue::Array(entries)) = fields.get_mut("Per Segment Data") else {
        return 0;
    };

    let mut changed = 0usize;
    for entry in entries {
        let NifValue::Struct(fields) = entry else {
            continue;
        };
        if value_u64(fields.get("User Index")) != Some(from) {
            continue;
        }
        fields.insert("User Index".to_string(), NifValue::UInt(to));
        changed += 1;
    }
    changed
}

struct ClothBoneFoldPlan {
    skin_id: usize,
    data_id: Option<usize>,
    old_to_new: Vec<usize>,
    kept_bones: Vec<NifValue>,
    kept_bone_list: Option<Vec<NifValue>>,
    dropped_count: usize,
}

fn fold_fo76_cloth_skin_bones(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let cloth_node_ids = collect_fo76_cloth_node_ids(nif);
    if cloth_node_ids.is_empty() {
        return;
    }

    let skin_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSSkin::Instance")
        .map(|block| block.block_id)
        .collect();
    let mut plans = Vec::new();
    let mut skipped_all_cloth = 0usize;
    for skin_id in skin_ids {
        match build_cloth_bone_fold_plan(nif, skin_id, &cloth_node_ids) {
            Some(plan) => plans.push(plan),
            None => {
                if skin_has_only_cloth_bones(nif, skin_id, &cloth_node_ids) {
                    skipped_all_cloth += 1;
                }
            }
        }
    }

    let mut changed_instances = 0usize;
    let mut dropped_bones = 0usize;
    let mut changed_vertices = 0usize;
    for plan in plans {
        changed_vertices += fold_shape_vertex_bones_for_skin(nif, plan.skin_id, &plan.old_to_new);
        apply_cloth_bone_fold_plan(nif, &plan);
        changed_instances += 1;
        dropped_bones += plan.dropped_count;
    }

    if changed_instances > 0 {
        report.changes.push(format!(
            "Havok cloth: folded {dropped_bones} cloth skin bone(s) across {changed_instances} skin instance(s), remapped {changed_vertices} weighted vertex/vertices"
        ));
    }
    if skipped_all_cloth > 0 {
        report.warnings.push(format!(
            "Havok cloth: left {skipped_all_cloth} all-cloth skin instance(s) unchanged; no non-cloth bone was available"
        ));
    }

    let remaining_skin_refs = collect_skin_bone_ref_ids(nif);
    let remove_ids: HashSet<usize> = cloth_node_ids
        .into_iter()
        .filter(|id| !remaining_skin_refs.contains(id))
        .collect();
    if remove_ids.is_empty() {
        return;
    }
    let removed_count = remove_ids.len();
    detach_child_refs(nif, &remove_ids);
    remove_blocks(nif, remove_ids);
    report.changes.push(format!(
        "Havok cloth: removed {removed_count} unsupported cloth bone node(s)"
    ));
}

fn collect_fo76_cloth_node_ids(nif: &NifFile) -> HashSet<usize> {
    nif.blocks
        .iter()
        .filter(|block| is_fo76_cloth_bone_block(block))
        .map(|block| block.block_id)
        .collect()
}

fn is_fo76_cloth_bone_block(block: &NifBlock) -> bool {
    block.type_name == "NiNode"
        && string_field(block, "Name").is_some_and(|name| is_fo76_cloth_bone_name(&name))
}

fn is_fo76_cloth_bone_name(name: &str) -> bool {
    let n = name.trim_end_matches('\0').to_ascii_lowercase();
    if n.starts_with("cloth_bone") {
        return true;
    }
    // FO76 cloth-sim rigs also use "<part>_Cloth<NN>" and "Cloth_<part><NN>".
    // Match indexed sim bones while leaving nodes like "Clothing" alone.
    n.find("cloth").is_some_and(|pos| {
        let suffix = &n[pos + "cloth".len()..];
        if suffix
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        {
            return true;
        }
        let mut chars = suffix.chars();
        chars.next() == Some('_')
            && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.next().is_some_and(|c| c.is_ascii_digit())
    })
}

fn build_cloth_bone_fold_plan(
    nif: &NifFile,
    skin_id: usize,
    cloth_node_ids: &HashSet<usize>,
) -> Option<ClothBoneFoldPlan> {
    let skin = nif.get_block(skin_id)?;
    let bones = value_array(skin.get_field("Bones"));
    if bones.is_empty() {
        return None;
    }

    let bone_refs: Vec<Option<usize>> = bones
        .iter()
        .map(|value| value_ref(Some(value)).and_then(|id| (id >= 0).then_some(id as usize)))
        .collect();
    let mut keep_old = Vec::with_capacity(bone_refs.len());
    let mut kept_old_indices = Vec::new();
    let mut dropped_count = 0usize;
    for (index, bone_ref) in bone_refs.iter().enumerate() {
        let drop = bone_ref.is_some_and(|id| cloth_node_ids.contains(&id));
        keep_old.push(!drop);
        if drop {
            dropped_count += 1;
        } else {
            kept_old_indices.push(index);
        }
    }
    if dropped_count == 0 || kept_old_indices.is_empty() {
        return None;
    }

    let mut old_to_new = vec![0usize; bones.len()];
    let mut kept_bones = Vec::with_capacity(kept_old_indices.len());
    for (new_index, old_index) in kept_old_indices.iter().copied().enumerate() {
        old_to_new[old_index] = new_index;
        kept_bones.push(bones[old_index].clone());
    }

    let data_id = field_ref(skin, "Data")
        .filter(|id| *id >= 0)
        .map(|id| id as usize);
    let translations = data_id
        .and_then(|id| bone_data_translations(nif, id, bones.len()))
        .unwrap_or_else(|| vec![None; bones.len()]);
    for old_index in 0..bones.len() {
        if keep_old[old_index] {
            continue;
        }
        let replacement_old = nearest_kept_bone(old_index, &kept_old_indices, &translations)
            .unwrap_or(kept_old_indices[0]);
        old_to_new[old_index] = old_to_new[replacement_old];
    }

    let kept_bone_list = data_id.and_then(|id| {
        let bone_list = nif.get_block(id)?.get_field("Bone List")?;
        let NifValue::Array(items) = bone_list else {
            return None;
        };
        if items.len() != bones.len() {
            return None;
        }
        Some(
            kept_old_indices
                .iter()
                .map(|index| items[*index].clone())
                .collect(),
        )
    });

    Some(ClothBoneFoldPlan {
        skin_id,
        data_id,
        old_to_new,
        kept_bones,
        kept_bone_list,
        dropped_count,
    })
}

fn skin_has_only_cloth_bones(
    nif: &NifFile,
    skin_id: usize,
    cloth_node_ids: &HashSet<usize>,
) -> bool {
    let Some(skin) = nif.get_block(skin_id) else {
        return false;
    };
    let bones = value_array(skin.get_field("Bones"));
    !bones.is_empty()
        && bones.iter().all(|value| {
            value_ref(Some(value))
                .and_then(|id| (id >= 0).then_some(id as usize))
                .is_some_and(|id| cloth_node_ids.contains(&id))
        })
}

fn bone_data_translations(
    nif: &NifFile,
    data_id: usize,
    expected_count: usize,
) -> Option<Vec<Option<[f32; 3]>>> {
    let data = nif.get_block(data_id)?;
    let NifValue::Array(bone_list) = data.get_field("Bone List")? else {
        return None;
    };
    if bone_list.len() != expected_count {
        return None;
    }
    Some(
        bone_list
            .iter()
            .map(|value| match value {
                NifValue::Struct(fields) => vec3_value(fields.get("Translation")),
                _ => None,
            })
            .collect(),
    )
}

fn nearest_kept_bone(
    old_index: usize,
    kept_old_indices: &[usize],
    translations: &[Option<[f32; 3]>],
) -> Option<usize> {
    let source = translations.get(old_index).copied().flatten()?;
    kept_old_indices
        .iter()
        .copied()
        .filter_map(|index| {
            translations
                .get(index)
                .copied()
                .flatten()
                .map(|target| (index, distance_squared(source, target)))
        })
        .min_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
}

fn distance_squared(left: [f32; 3], right: [f32; 3]) -> f32 {
    let dx = left[0] - right[0];
    let dy = left[1] - right[1];
    let dz = left[2] - right[2];
    dx * dx + dy * dy + dz * dz
}

fn fold_shape_vertex_bones_for_skin(
    nif: &mut NifFile,
    skin_id: usize,
    old_to_new: &[usize],
) -> usize {
    let shape_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| {
            field_ref(block, "Skin") == Some(skin_id as i32)
                || field_ref(block, "Skin Instance") == Some(skin_id as i32)
        })
        .map(|block| block.block_id)
        .collect();
    let mut changed_vertices = 0usize;
    for shape_id in shape_ids {
        if let Some(shape) = nif.blocks.get_mut(shape_id) {
            changed_vertices += fold_shape_vertex_bones(shape, old_to_new);
        }
    }
    changed_vertices
}

fn fold_shape_vertex_bones(shape: &mut NifBlock, old_to_new: &[usize]) -> usize {
    let Some(NifValue::Array(vertices)) = shape.get_field("Vertex Data").cloned() else {
        return 0;
    };

    let mut changed_vertices = 0usize;
    let mut updated_vertices = Vec::with_capacity(vertices.len());
    for vertex in vertices {
        let mut fields = match vertex {
            NifValue::Struct(fields) => fields,
            other => {
                updated_vertices.push(other);
                continue;
            }
        };
        let indices = value_array(fields.get("Bone Indices"));
        let weights = value_array(fields.get("Bone Weights"));
        if indices.is_empty() || weights.is_empty() {
            updated_vertices.push(NifValue::Struct(fields));
            continue;
        }

        let slot_count = indices.len().max(weights.len());
        let mut merged: BTreeMap<usize, f64> = BTreeMap::new();
        let mut changed = false;
        for slot in 0..slot_count {
            let old_index = indices.get(slot).and_then(value_usize).unwrap_or(0);
            let weight = value_f64(weights.get(slot)).unwrap_or(0.0);
            if weight <= 0.0 {
                continue;
            }
            let new_index = old_to_new.get(old_index).copied().unwrap_or(old_index);
            if new_index != old_index {
                changed = true;
            }
            *merged.entry(new_index).or_insert(0.0) += weight;
        }

        if changed {
            let (new_indices, new_weights) = compact_weight_slots(merged, slot_count);
            fields.insert("Bone Indices".to_string(), NifValue::Array(new_indices));
            fields.insert("Bone Weights".to_string(), NifValue::Array(new_weights));
            changed_vertices += 1;
        }
        updated_vertices.push(NifValue::Struct(fields));
    }

    if changed_vertices > 0 {
        shape.set_field("Vertex Data", NifValue::Array(updated_vertices));
    }
    changed_vertices
}

fn compact_weight_slots(
    merged: BTreeMap<usize, f64>,
    slot_count: usize,
) -> (Vec<NifValue>, Vec<NifValue>) {
    let mut pairs: Vec<(usize, f64)> = merged.into_iter().filter(|(_, w)| *w > 0.0).collect();
    pairs.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    pairs.truncate(slot_count);

    let sum: f64 = pairs.iter().map(|(_, weight)| *weight).sum();
    let mut indices = Vec::with_capacity(slot_count);
    let mut weights = Vec::with_capacity(slot_count);
    for (index, weight) in pairs {
        indices.push(NifValue::UInt(index as u64));
        weights.push(NifValue::Float(if sum > 0.0 { weight / sum } else { 0.0 }));
    }
    while indices.len() < slot_count {
        indices.push(NifValue::UInt(0));
        weights.push(NifValue::Float(0.0));
    }
    (indices, weights)
}

fn apply_cloth_bone_fold_plan(nif: &mut NifFile, plan: &ClothBoneFoldPlan) {
    if let Some(skin) = nif.blocks.get_mut(plan.skin_id) {
        skin.set_field("Num Bones", NifValue::UInt(plan.kept_bones.len() as u64));
        skin.set_field("Bones", NifValue::Array(plan.kept_bones.clone()));
    }
    let Some(data_id) = plan.data_id else {
        return;
    };
    let Some(kept_bone_list) = plan.kept_bone_list.clone() else {
        return;
    };
    if let Some(data) = nif.blocks.get_mut(data_id) {
        data.set_field("Num Bones", NifValue::UInt(kept_bone_list.len() as u64));
        data.set_field("Bone List", NifValue::Array(kept_bone_list));
    }
}

fn collect_skin_bone_ref_ids(nif: &NifFile) -> HashSet<usize> {
    let mut refs = HashSet::new();
    for block in &nif.blocks {
        if block.type_name != "BSSkin::Instance" {
            continue;
        }
        for value in value_array(block.get_field("Bones")) {
            if let Some(id) =
                value_ref(Some(&value)).and_then(|id| (id >= 0).then_some(id as usize))
            {
                refs.insert(id);
            }
        }
    }
    refs
}

fn detach_extra_data_refs(nif: &mut NifFile, remove_ids: &HashSet<usize>) -> usize {
    let mut detached = 0usize;
    for block in nif.blocks.iter_mut() {
        let Some(NifValue::Array(extra_refs)) = block.get_field("Extra Data List").cloned() else {
            continue;
        };
        let mut filtered = Vec::with_capacity(extra_refs.len());
        let before = extra_refs.len();
        for value in extra_refs {
            if value_ref(Some(&value))
                .filter(|id| *id >= 0)
                .is_some_and(|id| remove_ids.contains(&(id as usize)))
            {
                detached += 1;
                continue;
            }
            filtered.push(value);
        }
        if filtered.len() != before {
            block.set_field("Num Extra Data List", NifValue::UInt(filtered.len() as u64));
            block.set_field("Extra Data List", NifValue::Array(filtered));
        }
    }
    detached
}

fn np_collision_physics_block_ids(nif: &NifFile) -> HashSet<usize> {
    nif.blocks
        .iter()
        .filter(|block| block.type_name == "bhkNPCollisionObject")
        .filter_map(|block| field_ref(block, "Data"))
        .filter(|id| *id >= 0)
        .map(|id| id as usize)
        .collect()
}

#[derive(Debug, Clone, Copy, Default)]
struct NifCollisionIntent {
    bsx_flags: u64,
    has_dynamic_bsx: bool,
    has_complex_bsx: bool,
}

fn nif_collision_intent(nif: &NifFile) -> NifCollisionIntent {
    let bsx_flags = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSXFlags")
        .filter_map(|block| value_u64(block.get_field("Integer Data")))
        .fold(0, |acc, flags| acc | flags);
    NifCollisionIntent {
        bsx_flags,
        has_dynamic_bsx: bsx_flags & BSX_DYNAMIC_FLAG != 0,
        has_complex_bsx: bsx_flags & BSX_COMPLEX_FLAG != 0,
    }
}

fn source_metadata_for_nif_intent(
    mut metadata: SourceBodyMetadata,
    intent: NifCollisionIntent,
) -> SourceBodyMetadata {
    metadata.is_dynamic = source_body_is_dynamic_for_nif(metadata, intent, None);
    metadata
}

fn source_body_is_dynamic_for_nif(
    metadata: SourceBodyMetadata,
    intent: NifCollisionIntent,
    body: Option<&ExtractedCollisionBody>,
) -> bool {
    // KEYFRAMED (hknpMotionType==1) bodies are never loose clutter — keep them
    // out of the refmass fallback below so an animated door with an inertia
    // distribution is not promoted to a dynamic body.
    if metadata.motion_type == Some(1) {
        return false;
    }
    let is_single_convex = body.is_some_and(source_body_is_single_convex);
    // FO76 set dressing can carry Dynamic motion and BSX Dynamic without being
    // loose clutter. Non-complex compounds must remain static for FO4.
    if metadata.motion_type == Some(2)
        && intent.has_dynamic_bsx
        && !intent.has_complex_bsx
        && !is_single_convex
    {
        return false;
    }
    if is_dynamic_from_nif_signals(
        metadata.has_ref_mass_distribution,
        metadata.motion_type,
        intent.has_dynamic_bsx,
        intent.has_complex_bsx,
    ) {
        return true;
    }
    metadata.has_ref_mass_distribution && intent.has_dynamic_bsx && is_single_convex
}

fn source_body_is_single_convex(body: &ExtractedCollisionBody) -> bool {
    body.source_polytopes.len() == 1
        || (body.source_polytopes.is_empty()
            && body.meshes.len() == 1
            && body.meshes[0].shape_type == "convex_hull")
}

struct CollisionPlanEntry {
    source_collision_id: usize,
    source_parent_id: usize,
    source_parent_name: String,
    parent_id: usize,
    parent_name: String,
    planned: PlannedCollisionBody,
    source: Option<SourceCollisionSummary>,
    source_metadata: SourceBodyMetadata,
    nif_collision_intent: NifCollisionIntent,
    in_multi_body_assembly: bool,
    /// Explicit source `hknpBodyCinfo.mass`, decoded from the NIF's Havok body.
    body_mass: Option<f32>,
    /// The source body's true mass distribution (FO76 `hknpRefMassDistribution`),
    /// decoded from the source blob. Applied to dynamic (clutter) bodies' mass
    /// properties; ignored for statics by the builder's `mass > 0` guard.
    mass_distribution: Option<havok_native::collision::SourceMassDistribution>,
}

#[derive(Debug, Clone)]
struct SourceCollisionSummary {
    shape_kind: String,
    shape_summary: String,
    layer: Option<u8>,
    material_crc: Option<u32>,
}

#[derive(Debug, Default)]
struct CollisionChangeSummary {
    shape_changes: usize,
    layer_changes: usize,
    motion_info_changes: usize,
    details: Vec<String>,
}

fn synthesize_fo76_ground_object_collision(
    nif: &mut NifFile,
    report: &mut ConvertFileReport,
) {
    if has_live_collision_object(nif) || !is_fo76_ground_object_nif(nif) {
        return;
    }
    let root_id = nif
        .header
        .footer_roots
        .first()
        .copied()
        .filter(|id| *id >= 0)
        .map(|id| id as usize)
        .filter(|id| *id < nif.blocks.len())
        .unwrap_or(0);
    let metadata = SourceBodyMetadata {
        layer: Some(FO4_CLUTTER_LAYER),
        motion_type: Some(2),
        is_dynamic: true,
        ..SourceBodyMetadata::default()
    };
    let Some((parent_id, planned)) =
        plan_visible_aabb_collision_fallback(nif, root_id, 0, metadata, false)
    else {
        report.warnings.push(
            "FO76 ground object collision: no bounded visible geometry was available"
                .to_string(),
        );
        return;
    };
    let parent_name = nif
        .get_block(parent_id)
        .and_then(|block| string_field(block, "Name"))
        .unwrap_or_default();
    let entry = CollisionPlanEntry {
        source_collision_id: root_id,
        source_parent_id: root_id,
        source_parent_name: parent_name.clone(),
        parent_id,
        parent_name,
        planned,
        source: None,
        source_metadata: metadata,
        nif_collision_intent: NifCollisionIntent {
            bsx_flags: BSX_HAVOK_FLAG | BSX_DYNAMIC_FLAG | BSX_ARTICULATED_FLAG,
            has_dynamic_bsx: true,
            has_complex_bsx: false,
        },
        in_multi_body_assembly: false,
        body_mass: None,
        mass_distribution: None,
    };
    match install_fo4_np_collision_system_separate(nif, &[entry]) {
        Ok((count, failures)) if count > 0 => {
            ensure_root_bsx_flags(
                nif,
                parent_id,
                BSX_HAVOK_FLAG | BSX_DYNAMIC_FLAG | BSX_ARTICULATED_FLAG,
            );
            report.changes.push(format!(
                "FO76 ground object collision: synthesized {count} dynamic FO4 clutter AABB collision object(s)"
            ));
            for failure in failures {
                report.warnings.push(format!(
                    "FO76 ground object collision: additional collision build failed ({failure})"
                ));
            }
        }
        Ok((_, failures)) => report.warnings.push(format!(
            "FO76 ground object collision: collision synthesis produced no bodies ({})",
            failures.join("; ")
        )),
        Err(error) => report.warnings.push(format!(
            "FO76 ground object collision: collision synthesis failed ({error})"
        )),
    }
}

fn is_fo76_ground_object_nif(nif: &NifFile) -> bool {
    let is_ground_object_name = |name: &str| {
        let name = name.trim_end_matches('\0').to_ascii_lowercase();
        name.ends_with("_go") || name.starts_with("go_")
    };
    if nif
        .path
        .as_deref()
        .and_then(Path::file_stem)
        .and_then(|stem| stem.to_str())
        .is_some_and(is_ground_object_name)
    {
        return true;
    }
    nif.header
        .footer_roots
        .iter()
        .copied()
        .filter(|id| *id >= 0)
        .filter_map(|id| nif.get_block(id as usize))
        .filter_map(|block| string_field(block, "Name"))
        .any(|name| is_ground_object_name(&name))
}

fn rebuild_fo76_np_collision(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let collision_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkNPCollisionObject")
        .filter(|block| collision_data_is_physics_system(nif, block))
        .map(|block| block.block_id)
        .collect();
    if collision_ids.is_empty() {
        return;
    }
    // Several physics-system collision objects usually means a static assembly
    // (SCOL combined mesh, multi-part static, ...). Loose multi-part clutter can
    // also have several collision objects; keep those dynamic when BSX says the
    // NIF itself is dynamic.
    let nif_intent = nif_collision_intent(nif);
    let in_multi_body_assembly = collision_ids.len() > 1 && !nif_intent.has_dynamic_bsx;

    let mut remove = HashSet::new();
    let mut pending: Vec<CollisionPlanEntry> = Vec::new();
    let mut route_counts = RouteCounts::default();
    let mut regenerated = 0usize;
    let mut degenerate = 0usize;
    for collision_id in collision_ids.iter().copied() {
        let Some(collision) = nif.get_block(collision_id).cloned() else {
            continue;
        };
        let body_id = collision
            .get_field("Body ID")
            .and_then(value_usize)
            .unwrap_or(0);
        if collision_data_has_degenerate_shapes(nif, &collision) {
            degenerate += 1;
        }
        let Some(parent_ref) = field_ref(&collision, "Target").filter(|id| *id >= 0) else {
            report.warnings.push(format!(
                "FO76 bhkNPCollisionObject route: nif={}; src_block={collision_id}; body={body_id}; route={}; reason=missing parent target; stripping only",
                nif_diagnostic_path(nif),
                collision_route_name(CollisionRoute::StrippedUnrecoverable)
            ));
            route_counts.bump(CollisionRoute::StrippedUnrecoverable);
            collect_collision_subtree(nif, collision_id, &mut remove);
            continue;
        };
        if nif.get_block(parent_ref as usize).is_none() {
            report.warnings.push(format!(
                "FO76 bhkNPCollisionObject route: nif={}; src_block={collision_id}; parent_block={parent_ref}; body={body_id}; route={}; reason=parent block missing; stripping only",
                nif_diagnostic_path(nif),
                collision_route_name(CollisionRoute::StrippedUnrecoverable)
            ));
            route_counts.bump(CollisionRoute::StrippedUnrecoverable);
            collect_collision_subtree(nif, collision_id, &mut remove);
            continue;
        }
        let source_parent_name = nif
            .get_block(parent_ref as usize)
            .and_then(|block| string_field(block, "Name"))
            .unwrap_or_default();
        let source_blob = collision_physics_blob(nif, &collision);
        let mut source_metadata = source_blob
            .as_deref()
            .ok()
            .map(|blob| source_body_metadata(blob, body_id))
            .unwrap_or_default();
        source_metadata = source_metadata_for_nif_intent(source_metadata, nif_intent);
        // Decode this body's true mass distribution from the source blob so the
        // builder can use the real COM / volume / inertia instead of the AABB
        // approximation. `None` for statics / undecodable; the builder's
        // `mass > 0` guard keeps it from touching static bodies.
        let mass_distribution = source_blob.as_deref().ok().and_then(|blob| {
            havok_native::collision::decode_source_mass_distributions(blob)
                .get(body_id)
                .copied()
                .flatten()
        });
        let mut planned_parent_ref = parent_ref as usize;
        let mut source_summary = None;
        let planned = match source_blob
            .as_ref()
            .map_err(|error| error.clone())
            .and_then(|blob| extract_source_collision_body(blob, body_id))
            .and_then(|mut body| {
                source_metadata.is_dynamic =
                    source_body_is_dynamic_for_nif(source_metadata, nif_intent, Some(&body));
                body.is_dynamic = source_metadata.is_dynamic;
                source_summary = Some(SourceCollisionSummary::from_body(&body));
                classify_source_body(body, in_multi_body_assembly)
            }) {
            Ok(planned) => planned,
            Err(source_error) => {
                if let Some((fallback_parent_id, fallback)) = plan_visible_aabb_collision_fallback(
                    nif,
                    parent_ref as usize,
                    body_id,
                    source_metadata,
                    in_multi_body_assembly,
                ) {
                    let fallback_parent_name = nif
                        .get_block(fallback_parent_id)
                        .and_then(|block| string_field(block, "Name"))
                        .unwrap_or_default();
                    report.warnings.push(format!(
                        "FO76 bhkNPCollisionObject block {collision_id}: source collision unavailable ({source_error}); using visible mesh AABB fallback on block {fallback_parent_id} {fallback_parent_name:?}"
                    ));
                    planned_parent_ref = fallback_parent_id;
                    fallback
                } else if let Some((fallback_parent_id, fallback)) =
                    plan_minimal_aabb_collision_fallback(
                        nif,
                        parent_ref as usize,
                        body_id,
                        source_metadata,
                        in_multi_body_assembly,
                    )
                {
                    let fallback_parent_name = nif
                        .get_block(fallback_parent_id)
                        .and_then(|block| string_field(block, "Name"))
                        .unwrap_or_default();
                    report.warnings.push(format!(
                        "FO76 bhkNPCollisionObject block {collision_id}: source collision unavailable ({source_error}); using minimal AABB fallback on block {fallback_parent_id} {fallback_parent_name:?}"
                    ));
                    planned_parent_ref = fallback_parent_id;
                    fallback
                } else {
                    report.warnings.push(format!(
                        "FO76 bhkNPCollisionObject route: nif={}; src_block={collision_id}; parent_block={parent_ref}; parent_name={source_parent_name:?}; body={body_id}; route={}; reason=source collision unavailable ({source_error}); stripping collision",
                        nif_diagnostic_path(nif),
                        collision_route_name(CollisionRoute::StrippedUnrecoverable)
                    ));
                    if let Some(parent_mut) = nif.blocks.get_mut(parent_ref as usize) {
                        parent_mut.set_field("Collision Object", NifValue::Ref(-1));
                    }
                    route_counts.bump(CollisionRoute::StrippedUnrecoverable);
                    collect_collision_subtree(nif, collision_id, &mut remove);
                    continue;
                }
            }
        };
        route_counts.bump(planned.route);
        let planned_parent_name = nif
            .get_block(planned_parent_ref)
            .and_then(|block| string_field(block, "Name"))
            .unwrap_or_default();
        pending.push(CollisionPlanEntry {
            source_collision_id: collision_id,
            source_parent_id: parent_ref as usize,
            source_parent_name,
            parent_id: planned_parent_ref,
            parent_name: planned_parent_name,
            planned,
            source: source_summary,
            source_metadata,
            nif_collision_intent: nif_intent,
            in_multi_body_assembly,
            body_mass: source_metadata.body_mass,
            mass_distribution,
        });
        collect_collision_subtree(nif, collision_id, &mut remove);
    }

    pending.sort_by_key(|entry| {
        (
            collision_build_sort_rank(&entry.planned.shape),
            entry.planned.source_body_id,
        )
    });

    // Lift any ragdoll/articulation constraints out of the shared source physics
    // system and remap their body handles to the rebuilt output body order, so an
    // articulated trap (hanging chime, swinging sign) keeps its joints instead of
    // falling apart into loose clutter. Static assemblies (SCOL) carry no
    // constraintCinfos, so this is a no-op for them.
    let grafted_constraints = grafted_constraints_for_pending(nif, &pending);

    if !pending.is_empty() {
        match install_fo4_np_collision_system(nif, &pending, grafted_constraints.as_ref()) {
            Ok(count) => {
                regenerated = count;
                for entry in &pending {
                    ensure_root_havok_bsx_flag(nif, entry.parent_id);
                }
            }
            Err(error) => match install_fo4_np_collision_system_separate(nif, &pending) {
                Ok((count, failures)) => {
                    regenerated = count;
                    report.warnings.push(format!(
                            "FO76 bhkNPCollisionObject combined rebuild failed ({error}); regenerated {count}/{} collision object(s) as separate FO4 physics system(s)",
                            pending.len()
                        ));
                    for failure in failures {
                        report.warnings.push(format!(
                            "FO76 bhkNPCollisionObject separate rebuild skipped: {failure}"
                        ));
                    }
                    for entry in &pending {
                        ensure_root_havok_bsx_flag(nif, entry.parent_id);
                    }
                }
                Err(separate_error) => {
                    report.warnings.push(format!(
                            "FO76 bhkNPCollisionObject rebuild failed; stripping old collision only: {error}; separate fallback failed: {separate_error}"
                ));
                    for entry in &pending {
                        if let Some(parent_mut) = nif.blocks.get_mut(entry.parent_id) {
                            parent_mut.set_field("Collision Object", NifValue::Ref(-1));
                        }
                    }
                }
            },
        }
    }

    remove_blocks(nif, remove);

    let change_summary = summarize_collision_changes(&pending);
    report.changes.push(format!(
        "FO76 hknp collision: replaced {} bhkNPCollisionObject chain(s) ({degenerate} degenerate); regenerated {regenerated} FO4 collision object(s); routes: {}; shape_changes={}; layer_changes={}; motion_info_changes={}",
        collision_ids.len(),
        route_counts.report_fragment(),
        change_summary.shape_changes,
        change_summary.layer_changes,
        change_summary.motion_info_changes,
    ));
    if !change_summary.details.is_empty() {
        report.changes.push(format!(
            "FO76 hknp collision detail: {}",
            change_summary.details.join("; ")
        ));
    }
    for detail in summarize_collision_routes(nif, &pending) {
        report.changes.push(detail);
    }
}

/// Lift the constraint sub-graph from the shared source physics system and remap
/// its `constraintCinfos` body handles (source physics-system body indices) onto
/// the rebuilt output body order (position in `pending`). Returns `None` when the
/// source carries no constraints or none survive the remap. The articulated trap
/// case is a single physics system, so the constraints ride on every body's blob;
/// reading the first pending entry's blob is sufficient.
fn grafted_constraints_for_pending(
    nif: &NifFile,
    pending: &[CollisionPlanEntry],
) -> Option<GraftedConstraints> {
    let first = pending.first()?;
    let collision = nif.get_block(first.source_collision_id)?.clone();
    let blob = collision_physics_blob(nif, &collision).ok()?;
    let grafted = havok_native::collision::extract_grafted_constraints(&blob).ok()??;

    let src_to_out: HashMap<u32, u32> = pending
        .iter()
        .enumerate()
        .map(|(out, entry)| (entry.planned.source_body_id as u32, out as u32))
        .collect();

    let cinfos: Vec<GraftCinfo> = grafted
        .cinfos
        .iter()
        .filter_map(|cinfo| {
            Some(GraftCinfo {
                body_a: *src_to_out.get(&cinfo.body_a)?,
                body_b: *src_to_out.get(&cinfo.body_b)?,
                data_object: cinfo.data_object,
                flags: cinfo.flags,
            })
        })
        .collect();
    if cinfos.is_empty() {
        return None;
    }
    Some(GraftedConstraints {
        objects: grafted.objects,
        cinfos,
    })
}

/// FO4 ANIMSTATIC collision layer (value 2). A body on this layer is an
/// animated door/shutter/container part: it MUST be emitted as a keyframed
/// body (motionId → motionCinfo). Emitting it as Static yields motionId =
/// HK_INVALID, which FO4 treats as a static collision object — the engine then
/// never plays the NIF's Open/Close NiControllerSequence. Multi-body systems
/// masked this (the `bodies.len() > 1 && cm_count > 0` rule in
/// `build_fo4_multi_body_collision` grants a motionCinfo anyway), so only
/// single-body doors (e.g. CivWarDoor01/02) regressed. Mirrors the Python
/// `_motion_type_for_layer` in `nif/operations/collision.py`.
const FO4_ANIMSTATIC_LAYER: u8 = 2;

fn body_motion_type_for_layer(layer: u8) -> BodyMotionType {
    if layer == FO4_ANIMSTATIC_LAYER {
        BodyMotionType::Keyframed
    } else {
        BodyMotionType::Static
    }
}

fn body_motion_type_for_source(metadata: SourceBodyMetadata, output_layer: u8) -> BodyMotionType {
    if metadata.motion_type == Some(1) {
        // hknpMotionType::KEYFRAMED
        BodyMotionType::Keyframed
    } else {
        body_motion_type_for_layer(output_layer)
    }
}

fn body_motion_type_for_entry(entry: &CollisionPlanEntry) -> BodyMotionType {
    body_motion_type_for_source(entry.source_metadata, entry.planned.layer)
}

impl SourceCollisionSummary {
    fn from_body(body: &ExtractedCollisionBody) -> Self {
        Self {
            shape_kind: source_collision_shape_kind(body),
            shape_summary: source_collision_shape_summary(body),
            layer: body.layer,
            material_crc: body.material_crc,
        }
    }
}

fn source_collision_shape_kind(body: &ExtractedCollisionBody) -> String {
    if body.meshes.len() != 1 {
        return "compound".to_string();
    }
    match body.meshes[0].shape_type.as_str() {
        "convex_hull" => "polytope".to_string(),
        other => other.to_string(),
    }
}

fn source_collision_shape_summary(body: &ExtractedCollisionBody) -> String {
    if body.meshes.len() == 1 {
        let mesh = &body.meshes[0];
        return format!(
            "{}({}v/{}t)",
            mesh.shape_type,
            mesh.vertices.len(),
            mesh.triangles.len()
        );
    }

    let mut counts = BTreeMap::<&str, usize>::new();
    for mesh in &body.meshes {
        *counts.entry(mesh.shape_type.as_str()).or_default() += 1;
    }
    let kinds = counts
        .iter()
        .map(|(kind, count)| format!("{kind}x{count}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("compound({} children:{kinds})", body.meshes.len())
}

fn output_collision_shape_kind(shape: &MultiBodyShape) -> &'static str {
    match shape {
        MultiBodyShape::Polytope { .. } | MultiBodyShape::SourcePolytope { .. } => "polytope",
        MultiBodyShape::CompressedMesh { .. } => "compressed_mesh",
        MultiBodyShape::RawCompressedMesh { .. } => "raw_compressed_mesh",
        MultiBodyShape::Compound { .. } => "compound",
        MultiBodyShape::Sphere { .. } => "sphere",
        MultiBodyShape::Capsule { .. } => "capsule",
    }
}

fn output_collision_shape_summary(shape: &MultiBodyShape) -> String {
    match shape {
        MultiBodyShape::Polytope { vertices } => format!("polytope({}v)", vertices.len()),
        MultiBodyShape::SourcePolytope { shape } => format!(
            "polytope({}v/{}p/{}f/{}i source)",
            shape.vertices.len(),
            shape.planes.len(),
            shape.faces.len(),
            shape.indices.len()
        ),
        MultiBodyShape::CompressedMesh {
            vertices,
            triangles,
        } => format!("compressed_mesh({}v/{}t)", vertices.len(), triangles.len()),
        MultiBodyShape::RawCompressedMesh { .. } => "raw_compressed_mesh".to_string(),
        MultiBodyShape::Compound { children } => {
            format!(
                "compound({} children:{})",
                children.len(),
                output_compound_child_counts(children)
            )
        }
        MultiBodyShape::Sphere { radius, .. } => format!("sphere(r={radius:.3})"),
        MultiBodyShape::Capsule { convex_radius, .. } => {
            format!("capsule(cr={convex_radius:.3})")
        }
    }
}

fn output_compound_child_counts(children: &[havok_native::collision::CompoundChild]) -> String {
    let mut counts = BTreeMap::<&str, usize>::new();
    for child in children {
        *counts
            .entry(output_compound_child_kind(&child.kind))
            .or_default() += 1;
    }
    counts
        .iter()
        .map(|(kind, count)| format!("{kind}x{count}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn output_compound_child_kind(kind: &CompoundChildKind) -> &'static str {
    match kind {
        CompoundChildKind::Polytope { .. } => "polytope",
        CompoundChildKind::SourcePolytope { .. } => "source_polytope",
        CompoundChildKind::CompressedMesh { .. } => "compressed_mesh",
    }
}

fn output_motion_policy(entries: &[CollisionPlanEntry], index: usize) -> String {
    let has_keyframed = entries
        .iter()
        .any(|entry| body_motion_type_for_entry(entry) == BodyMotionType::Keyframed);
    let cm_count = entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.planned.shape,
                MultiBodyShape::CompressedMesh { .. } | MultiBodyShape::RawCompressedMesh { .. }
            )
        })
        .count();
    let all_bodies_compressed = cm_count == entries.len();
    let entry = &entries[index];
    let motion_type = body_motion_type_for_entry(entry);
    let needs_motion_cinfo = if motion_type == BodyMotionType::Keyframed {
        true
    } else if entry.planned.layer == FO4_CLUTTER_LAYER {
        true
    } else {
        match (&entry.planned.shape, motion_type) {
            _ if has_keyframed => false,
            (
                MultiBodyShape::CompressedMesh { .. } | MultiBodyShape::RawCompressedMesh { .. },
                _,
            ) if entries.len() > 1 && all_bodies_compressed => true,
            _ => false,
        }
    };
    let motion_label = if motion_type == BodyMotionType::Keyframed {
        "keyframed"
    } else if entry.planned.layer == FO4_CLUTTER_LAYER {
        "dynamic-clutter"
    } else {
        "static"
    };
    let cinfo_label = if needs_motion_cinfo {
        "motionCinfo"
    } else {
        "no-motionCinfo"
    };
    let mass_label =
        if entry.planned.layer == FO4_CLUTTER_LAYER && motion_type != BodyMotionType::Keyframed {
            "+clutter-mass"
        } else {
            ""
        };
    format!("{motion_label}+{cinfo_label}{mass_label}")
}

fn summarize_collision_changes(entries: &[CollisionPlanEntry]) -> CollisionChangeSummary {
    let mut summary = CollisionChangeSummary::default();
    let mut changed_entries = 0usize;
    for (idx, entry) in entries.iter().enumerate() {
        let Some(source) = &entry.source else {
            continue;
        };
        let output_shape_kind = output_collision_shape_kind(&entry.planned.shape);
        let source_motion = source_motion_policy_from_metadata(entry.source_metadata);
        let output_motion = output_motion_policy(entries, idx);
        let shape_changed = source.shape_kind != output_shape_kind;
        let layer_changed = source.layer != Some(entry.planned.layer);
        let motion_changed = source_motion != output_motion;
        if shape_changed {
            summary.shape_changes += 1;
        }
        if layer_changed {
            summary.layer_changes += 1;
        }
        if motion_changed {
            summary.motion_info_changes += 1;
        }
        if shape_changed || layer_changed || motion_changed {
            changed_entries += 1;
            if summary.details.len() < 8 {
                summary.details.push(format!(
                    "{}: body={} shape {} -> {}; layer {} -> {}; motion {} -> {}; route={}",
                    collision_parent_label(entry),
                    entry.planned.source_body_id,
                    source.shape_summary,
                    output_collision_shape_summary(&entry.planned.shape),
                    source
                        .layer
                        .map(|layer| layer.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    entry.planned.layer,
                    source_motion,
                    output_motion,
                    collision_route_name(entry.planned.route)
                ));
            }
        }
    }
    if changed_entries > summary.details.len() {
        let omitted = changed_entries - summary.details.len();
        summary
            .details
            .push(format!("+{omitted} more change marker(s)"));
    }
    summary
}

fn summarize_collision_routes(nif: &NifFile, entries: &[CollisionPlanEntry]) -> Vec<String> {
    entries
        .iter()
        .enumerate()
        .map(|(idx, _)| {
            format!(
                "FO76 hknp collision route: nif={}; {}",
                nif_diagnostic_path(nif),
                collision_route_detail(entries, idx)
            )
        })
        .collect()
}

fn collision_route_detail(entries: &[CollisionPlanEntry], index: usize) -> String {
    let entry = &entries[index];
    let source = entry
        .source
        .as_ref()
        .map(|source| source.shape_summary.as_str())
        .unwrap_or("unavailable");
    let source_layer = entry
        .source
        .as_ref()
        .and_then(|source| source.layer)
        .or(entry.source_metadata.layer);
    let source_material = entry
        .source
        .as_ref()
        .and_then(|source| source.material_crc)
        .or(entry.source_metadata.material_crc);
    let source_motion = source_motion_policy_from_metadata(entry.source_metadata);
    let mut detail = format!(
        "src_block={}; source_parent={}; body={}; route={}; source_shape={source}; output_shape={}; filter=layer {}->{}; material {}->{}; motion {} -> {}; meta={}",
        entry.source_collision_id,
        source_parent_label(entry),
        entry.planned.source_body_id,
        collision_route_name(entry.planned.route),
        output_collision_shape_summary(&entry.planned.shape),
        format_layer(source_layer),
        entry.planned.layer,
        format_material_crc(source_material),
        format_material_crc(entry.planned.material_crc),
        source_motion,
        output_motion_policy(entries, index),
        source_metadata_fragment(entry),
    );
    if entry.parent_id != entry.source_parent_id {
        detail.push_str(&format!(
            "; output_parent={}",
            collision_parent_label(entry)
        ));
    }
    if let Some(notes) = collision_decision_notes(entry) {
        detail.push_str("; notes=");
        detail.push_str(&notes);
    }
    detail
}

fn source_parent_label(entry: &CollisionPlanEntry) -> String {
    if entry.source_parent_name.is_empty() {
        format!("block={}", entry.source_parent_id)
    } else {
        format!(
            "block={} name={:?}",
            entry.source_parent_id, entry.source_parent_name
        )
    }
}

fn source_motion_policy_from_metadata(metadata: SourceBodyMetadata) -> &'static str {
    if metadata.motion_type == Some(1) {
        // hknpMotionType::KEYFRAMED
        if metadata.has_ref_mass_distribution {
            "keyframed-refmass"
        } else {
            "keyframed-motionType"
        }
    } else if metadata.is_dynamic {
        if metadata.has_ref_mass_distribution {
            "dynamic-refmass"
        } else {
            "dynamic-motionType"
        }
    } else if metadata.has_ref_mass_distribution {
        "static/refmass-nonmovable"
    } else if metadata.layer == Some(FO4_ANIMSTATIC_LAYER) {
        "keyframed-layer2"
    } else {
        "static/no-refmass"
    }
}

fn source_metadata_fragment(entry: &CollisionPlanEntry) -> String {
    format!(
        "motion_type:{},flags:{},mass:{},refmass:{},dynamic:{},bsx:0x{:X}(dynamic:{},complex:{}),assembly:{}",
        format_motion_type(entry.source_metadata.motion_type),
        format_body_flags(entry.source_metadata.body_flags),
        format_mass(entry.body_mass),
        yes_no(entry.mass_distribution.is_some()),
        yes_no(entry.source_metadata.is_dynamic),
        entry.nif_collision_intent.bsx_flags,
        yes_no(entry.nif_collision_intent.has_dynamic_bsx),
        yes_no(entry.nif_collision_intent.has_complex_bsx),
        if entry.in_multi_body_assembly {
            "multi-static"
        } else {
            "single"
        },
    )
}

fn collision_decision_notes(entry: &CollisionPlanEntry) -> Option<String> {
    let mut notes = Vec::new();
    if entry.source.is_none() {
        notes.push("source-shape-undecodable");
    }
    if entry.source_metadata.is_dynamic && entry.in_multi_body_assembly {
        notes.push("dynamic-source-in-static-assembly");
    }
    if entry.mass_distribution.is_some()
        && entry.source_metadata.motion_type == Some(1)
        && !entry.source_metadata.is_dynamic
    {
        notes.push("keyframed-refmass-kept-keyframed");
    } else if entry.mass_distribution.is_some() && !entry.source_metadata.is_dynamic {
        notes.push("refmass-without-dynamic-complex-bsx-kept-static");
    }
    if entry.source_metadata.motion_type == Some(2)
        && !entry.source_metadata.is_dynamic
        && !entry.nif_collision_intent.has_dynamic_bsx
    {
        notes.push("dynamic-motion-without-bsx-kept-static");
    }
    if entry.source_metadata.layer == Some(FO4_CLUTTER_LAYER)
        && entry.planned.layer == FO4_STATIC_LAYER
    {
        notes.push("clutter-layer-demoted-to-static");
    }
    if notes.is_empty() {
        None
    } else {
        Some(notes.join(","))
    }
}

fn format_layer(layer: Option<u8>) -> String {
    layer
        .map(|layer| layer.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn format_material_crc(material_crc: Option<u32>) -> String {
    material_crc
        .map(|crc| format!("0x{crc:08X}"))
        .unwrap_or_else(|| "none".to_string())
}

fn format_motion_type(motion_type: Option<u8>) -> String {
    motion_type
        .map(|motion_type| format!("{}({motion_type})", motion_type_label(motion_type as i64)))
        .unwrap_or_else(|| "unknown".to_string())
}

fn format_mass(mass: Option<f32>) -> String {
    mass.map(|mass| format!("{mass:.3}"))
        .unwrap_or_else(|| "none".to_string())
}

fn format_body_flags(flags: Option<i64>) -> String {
    flags
        .map(|flags| format!("0x{flags:X}"))
        .unwrap_or_else(|| "unknown".to_string())
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn collision_parent_label(entry: &CollisionPlanEntry) -> String {
    if entry.parent_name.is_empty() {
        format!("block={}", entry.parent_id)
    } else {
        format!("block={} name={:?}", entry.parent_id, entry.parent_name)
    }
}

fn collision_route_name(route: CollisionRoute) -> &'static str {
    match route {
        CollisionRoute::SourcePolytope => "source-polytope",
        CollisionRoute::SourceCompound => "source-compound",
        CollisionRoute::SourceCompressedMesh => "source-compressed-mesh",
        CollisionRoute::ClutterConvex => "clutter-convex",
        CollisionRoute::VisibleMeshAabbFallback => "visible-mesh-aabb-fallback",
        CollisionRoute::StrippedUnrecoverable => "stripped-unrecoverable",
    }
}

fn build_validated_np_blob(
    bodies: &[MultiBodyShape],
    material_crcs: &[Option<u32>],
    body_metas: &[BodyMeta],
    opts: &BuildOptions,
    constraints: Option<&GraftedConstraints>,
    diagnostic_context: &str,
) -> Result<std::sync::Arc<Vec<u8>>, String> {
    let blob = with_collision_diagnostic_context(diagnostic_context, || {
        build_fo4_multi_body_collision_with_constraints(
            bodies,
            opts,
            Some(material_crcs),
            Some(body_metas),
            constraints,
        )
    })
    .map_err(|error| error.to_string())?;
    let summary = havok_native::api::havok_collision_summary(&blob)
        .map_err(|error| format!("rebuilt FO4 collision summary failed: {error}"))?;
    if collision_summary_is_invalid(&summary) {
        return Err(
            "rebuilt FO4 collision summary is invalid (degenerate / NaN inertia / unresolved dynamic motion)"
                .to_string(),
        );
    }
    Ok(std::sync::Arc::new(blob))
}

fn np_collision_diagnostic_context(nif: &NifFile, entries: &[CollisionPlanEntry]) -> String {
    let mut parents = entries
        .iter()
        .take(8)
        .map(np_collision_parent_diagnostic)
        .collect::<Vec<_>>();
    if entries.len() > parents.len() {
        parents.push(format!("+{} more", entries.len() - parents.len()));
    }
    let parents = if parents.is_empty() {
        "none".to_string()
    } else {
        parents.join(", ")
    };
    format!("nif={}; parents={parents}", nif_diagnostic_path(nif))
}

fn np_collision_entry_diagnostic_context(
    nif: &NifFile,
    parent_id: usize,
    parent_name: &str,
    planned: &PlannedCollisionBody,
) -> String {
    format!(
        "nif={}; parent={}",
        nif_diagnostic_path(nif),
        np_collision_parent_diagnostic_parts(parent_id, parent_name, planned.source_body_id)
    )
}

fn nif_diagnostic_path(nif: &NifFile) -> String {
    nif.path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<in-memory NIF>".to_string())
}

fn np_collision_parent_diagnostic(entry: &CollisionPlanEntry) -> String {
    np_collision_parent_diagnostic_parts(
        entry.parent_id,
        &entry.parent_name,
        entry.planned.source_body_id,
    )
}

fn np_collision_parent_diagnostic_parts(
    parent_id: usize,
    parent_name: &str,
    source_body_id: usize,
) -> String {
    if parent_name.is_empty() {
        format!("block={parent_id}; body={source_body_id}")
    } else {
        format!(
            "block={parent_id}; name={parent_name:?}; body={}",
            source_body_id
        )
    }
}

fn install_fo4_np_collision_system(
    nif: &mut NifFile,
    entries: &[CollisionPlanEntry],
    constraints: Option<&GraftedConstraints>,
) -> Result<usize, String> {
    let bodies = entries
        .iter()
        .map(|entry| entry.planned.shape.clone())
        .collect::<Vec<_>>();
    let material_crcs = entries
        .iter()
        .map(|entry| entry.planned.material_crc)
        .collect::<Vec<_>>();
    let body_metas = entries
        .iter()
        .map(|entry| BodyMeta {
            // In a constrained assembly the source group/system filter bits are
            // load-bearing (they keep the linked bodies from self-colliding), so
            // carry the full filter; otherwise the builder writes just the layer.
            collision_filter_info: constraints.and(entry.source_metadata.collision_filter_info),
            layer: entry.planned.layer,
            body_flags: entry.source_metadata.body_flags,
            // Position is INTENTIONALLY origin (orientation identity). The shape
            // vertices reaching the builder are already world/NIF-baked: the FO76
            // decode path (`extract_preview_meshes_from_blob` →
            // `shape_targets_for_body`) starts from an identity transform and only
            // composes compound-instance transforms — it never applies the source
            // `bodyCinfo.position`, so each body's geometry already sits at its
            // authored world offset (verified on FO76 ammo / cryo-debris multi-body
            // NIFs: per-body geometry AABB centers equal the source body positions).
            // FO4's narrowphase places a body's shape at `bodyCinfo.position`, so
            // copying the source body transform here would translate the
            // already-offset geometry a SECOND time and mis-place collision across
            // every multi-body static. Keep the body frame at origin; placement
            // comes from the baked geometry.
            position: [0.0, 0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
            motion_type: body_motion_type_for_entry(entry),
            body_mass: entry.body_mass,
            mass_distribution: entry.mass_distribution,
        })
        .collect::<Vec<_>>();
    let opts = BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer: 1,
        mass: 0.0,
        convex_radius: DEFAULT_COLLISION_RADIUS as f32,
        materials: Vec::new(),
        user_data: None,
        body_props_raw: None,
        mass_distribution: None,
    };
    let diagnostic_context = np_collision_diagnostic_context(nif, entries);
    let blob = build_validated_np_blob(
        &bodies,
        &material_crcs,
        &body_metas,
        &opts,
        constraints,
        &diagnostic_context,
    )?;

    let mut collision_ids = Vec::with_capacity(entries.len());
    for (body_id, entry) in entries.iter().enumerate() {
        let mut collision_fields = IndexMap::new();
        collision_fields.insert("Flags".to_string(), NifValue::UInt(0x80));
        collision_fields.insert("Target".to_string(), NifValue::Ref(entry.parent_id as i32));
        collision_fields.insert("Body ID".to_string(), NifValue::UInt(body_id as u64));
        collision_fields.insert("Data".to_string(), NifValue::Ref(-1));
        let collision_id = nif.add_block("bhkNPCollisionObject", Some(collision_fields));
        let parent = nif
            .blocks
            .get_mut(entry.parent_id)
            .ok_or_else(|| format!("collision parent block {} missing", entry.parent_id))?;
        parent.set_field("Collision Object", NifValue::Ref(collision_id as i32));
        collision_ids.push(collision_id);
    }

    let mut physics_fields = IndexMap::new();
    physics_fields.insert(
        "Binary Data".to_string(),
        crate::cloth::bytes_to_byte_array(&blob),
    );
    let physics_id = nif.add_block("bhkPhysicsSystem", Some(physics_fields));

    for collision_id in collision_ids {
        let collision = nif
            .blocks
            .get_mut(collision_id)
            .ok_or_else(|| format!("collision block {collision_id} missing"))?;
        collision.set_field("Data", NifValue::Ref(physics_id as i32));
    }

    Ok(entries.len())
}

fn install_fo4_np_collision_system_separate(
    nif: &mut NifFile,
    entries: &[CollisionPlanEntry],
) -> Result<(usize, Vec<String>), String> {
    let mut built = Vec::new();
    let mut failures = Vec::new();
    for entry in entries {
        let bodies = [entry.planned.shape.clone()];
        let material_crcs = [entry.planned.material_crc];
        let body_metas = [BodyMeta {
            collision_filter_info: None,
            layer: entry.planned.layer,
            body_flags: entry.source_metadata.body_flags,
            // Origin/identity is intentional — the shape geometry is already
            // world-baked, so a body transform would double-apply the offset. See
            // `install_fo4_np_collision_system` for the full rationale.
            position: [0.0, 0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
            motion_type: body_motion_type_for_entry(entry),
            body_mass: entry.body_mass,
            mass_distribution: entry.mass_distribution,
        }];
        let opts = BuildOptions {
            friction: 0.5,
            restitution: 0.4,
            layer: 1,
            mass: 0.0,
            convex_radius: DEFAULT_COLLISION_RADIUS as f32,
            materials: Vec::new(),
            user_data: None,
            body_props_raw: None,
            mass_distribution: None,
        };
        let diagnostic_context = np_collision_entry_diagnostic_context(
            nif,
            entry.parent_id,
            &entry.parent_name,
            &entry.planned,
        );
        // The separate/fallback path emits one single-body blob per entry, so
        // there is no second body for a constraint to link — constraints ride
        // only on the combined multi-body blob.
        match build_validated_np_blob(
            &bodies,
            &material_crcs,
            &body_metas,
            &opts,
            None,
            &diagnostic_context,
        ) {
            Ok(blob) => built.push((entry.parent_id, blob)),
            Err(error) => failures.push(format!(
                "parent block {} ({}) body {}: {error}",
                entry.parent_id, entry.parent_name, entry.planned.source_body_id
            )),
        }
    }

    if built.is_empty() {
        return Err(if failures.is_empty() {
            "no collision bodies were built".to_string()
        } else {
            failures.join("; ")
        });
    }

    for entry in entries {
        if let Some(parent) = nif.blocks.get_mut(entry.parent_id) {
            parent.set_field("Collision Object", NifValue::Ref(-1));
        }
    }

    for (parent_id, blob) in &built {
        let mut collision_fields = IndexMap::new();
        collision_fields.insert("Flags".to_string(), NifValue::UInt(0x80));
        collision_fields.insert("Target".to_string(), NifValue::Ref(*parent_id as i32));
        collision_fields.insert("Body ID".to_string(), NifValue::UInt(0));
        collision_fields.insert("Data".to_string(), NifValue::Ref(-1));
        let collision_id = nif.add_block("bhkNPCollisionObject", Some(collision_fields));

        let mut physics_fields = IndexMap::new();
        physics_fields.insert(
            "Binary Data".to_string(),
            crate::cloth::bytes_to_byte_array(blob),
        );
        let physics_id = nif.add_block("bhkPhysicsSystem", Some(physics_fields));

        let collision = nif
            .blocks
            .get_mut(collision_id)
            .ok_or_else(|| format!("collision block {collision_id} missing"))?;
        collision.set_field("Data", NifValue::Ref(physics_id as i32));
        let parent = nif
            .blocks
            .get_mut(*parent_id)
            .ok_or_else(|| format!("collision parent block {parent_id} missing"))?;
        parent.set_field("Collision Object", NifValue::Ref(collision_id as i32));
    }

    Ok((built.len(), failures))
}

fn plan_visible_aabb_collision_fallback(
    nif: &NifFile,
    parent_id: usize,
    body_id: usize,
    metadata: SourceBodyMetadata,
    in_multi_body_assembly: bool,
) -> Option<(usize, PlannedCollisionBody)> {
    let (target_parent_id, aabb_vertices) = visible_aabb_fallback_vertices(nif, parent_id)?;
    let resolved_layer =
        resolve_aabb_fallback_layer(metadata.layer, metadata.is_dynamic, in_multi_body_assembly);
    Some((
        target_parent_id,
        PlannedCollisionBody {
            source_body_id: body_id,
            route: CollisionRoute::VisibleMeshAabbFallback,
            layer: resolved_layer,
            material_crc: metadata.material_crc,
            shape: MultiBodyShape::Polytope {
                vertices: nif_vertices_to_havok(&aabb_vertices),
            },
        },
    ))
}

fn plan_minimal_aabb_collision_fallback(
    nif: &NifFile,
    parent_id: usize,
    body_id: usize,
    metadata: SourceBodyMetadata,
    in_multi_body_assembly: bool,
) -> Option<(usize, PlannedCollisionBody)> {
    nif.get_block(parent_id)?;
    let resolved_layer =
        resolve_aabb_fallback_layer(metadata.layer, metadata.is_dynamic, in_multi_body_assembly);
    Some((
        parent_id,
        PlannedCollisionBody {
            source_body_id: body_id,
            route: CollisionRoute::VisibleMeshAabbFallback,
            layer: resolved_layer,
            material_crc: metadata.material_crc,
            shape: MultiBodyShape::Polytope {
                vertices: nif_vertices_to_havok(&minimal_aabb_vertices()),
            },
        },
    ))
}

fn resolve_aabb_fallback_layer(
    source_layer: Option<u8>,
    source_is_dynamic: bool,
    in_multi_body_assembly: bool,
) -> u8 {
    if source_is_dynamic && !in_multi_body_assembly {
        return FO4_CLUTTER_LAYER;
    }
    if source_layer == Some(FO4_CLUTTER_LAYER) {
        return FO4_STATIC_LAYER;
    }
    source_layer.unwrap_or(FO4_STATIC_LAYER)
}

fn visible_aabb_fallback_vertices(
    nif: &NifFile,
    parent_id: usize,
) -> Option<(usize, Vec<[f32; 3]>)> {
    let mut candidates = vec![parent_id, scene_root_id(nif, parent_id), 0];
    candidates.dedup();
    for candidate_id in candidates {
        let vertices = collect_geometry_vertices(nif, candidate_id);
        if vertices.len() < 3 {
            continue;
        }
        if let Some(aabb) = bounded_aabb_vertices(&vertices) {
            return Some((candidate_id, aabb));
        }
    }
    None
}

fn minimal_aabb_vertices() -> Vec<[f32; 3]> {
    let half = FO76_VISIBLE_AABB_FALLBACK_MIN_EXTENT * 0.5;
    vec![
        [-half, -half, -half],
        [half, -half, -half],
        [-half, half, -half],
        [half, half, -half],
        [-half, -half, half],
        [half, -half, half],
        [-half, half, half],
        [half, half, half],
    ]
}

fn bounded_aabb_vertices(vertices: &[[f32; 3]]) -> Option<Vec<[f32; 3]>> {
    let mut mins = vertices.first().copied()?;
    let mut maxs = mins;
    for vertex in vertices.iter().skip(1) {
        for axis in 0..3 {
            mins[axis] = mins[axis].min(vertex[axis]);
            maxs[axis] = maxs[axis].max(vertex[axis]);
        }
    }

    for axis in 0..3 {
        let extent = maxs[axis] - mins[axis];
        if extent > FO76_VISIBLE_AABB_FALLBACK_MAX_EXTENT {
            return None;
        }
        if extent < FO76_VISIBLE_AABB_FALLBACK_MIN_EXTENT {
            let center = (mins[axis] + maxs[axis]) * 0.5;
            let half = FO76_VISIBLE_AABB_FALLBACK_MIN_EXTENT * 0.5;
            mins[axis] = center - half;
            maxs[axis] = center + half;
        }
    }

    Some(vec![
        [mins[0], mins[1], mins[2]],
        [maxs[0], mins[1], mins[2]],
        [mins[0], maxs[1], mins[2]],
        [maxs[0], maxs[1], mins[2]],
        [mins[0], mins[1], maxs[2]],
        [maxs[0], mins[1], maxs[2]],
        [mins[0], maxs[1], maxs[2]],
        [maxs[0], maxs[1], maxs[2]],
    ])
}

fn collision_build_sort_rank(shape: &MultiBodyShape) -> usize {
    match shape {
        MultiBodyShape::CompressedMesh { .. } => 0,
        _ => 1,
    }
}

fn collision_data_is_physics_system(nif: &NifFile, collision: &NifBlock) -> bool {
    let Some(data_ref) = field_ref(collision, "Data").filter(|id| *id >= 0) else {
        return false;
    };
    let Some(data_block) = nif.get_block(data_ref as usize) else {
        return false;
    };
    data_block.type_name == "bhkPhysicsSystem"
}

fn collision_physics_blob(nif: &NifFile, collision: &NifBlock) -> Result<Vec<u8>, String> {
    let data_ref = field_ref(collision, "Data")
        .filter(|id| *id >= 0)
        .ok_or_else(|| "collision Data ref is missing".to_string())?;
    let data_block = nif
        .get_block(data_ref as usize)
        .ok_or_else(|| format!("collision Data block {data_ref} is missing"))?;
    let binary_data = data_block
        .get_field("Binary Data")
        .ok_or_else(|| format!("bhkPhysicsSystem block {data_ref} has no Binary Data"))?;
    crate::cloth::byte_array_to_bytes(binary_data).map_err(|error| error.to_string())
}

fn collision_data_has_degenerate_shapes(nif: &NifFile, collision: &NifBlock) -> bool {
    let Some(data_ref) = field_ref(collision, "Data").filter(|id| *id >= 0) else {
        return false;
    };
    let Some(data_block) = nif.get_block(data_ref as usize) else {
        return false;
    };
    let Some(binary_data) = data_block.get_field("Binary Data").cloned() else {
        return false;
    };
    let Ok(bytes) = crate::cloth::byte_array_to_bytes(&binary_data) else {
        return true;
    };
    if bytes.is_empty() {
        return false;
    }
    fo4_havok_blob_needs_collision_rebuild(&bytes)
}

fn fo4_havok_blob_needs_collision_rebuild(bytes: &[u8]) -> bool {
    match havok_native::api::havok_collision_summary(bytes) {
        Ok(summary) => summary_has_degenerate_collision_shape(&summary),
        Err(_) => true,
    }
}

fn regenerate_fo4_collision(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let collision_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkCollisionObject")
        .map(|block| block.block_id)
        .collect();
    if collision_ids.is_empty() {
        return;
    }

    let mut pending = Vec::new();
    let mut remove = HashSet::new();
    for collision_id in collision_ids.iter().copied() {
        let Some(collision) = nif.get_block(collision_id).cloned() else {
            continue;
        };
        let Some(parent_ref) = field_ref(&collision, "Target").filter(|id| *id >= 0) else {
            report.warnings.push(format!(
                "bhkCollisionObject block {collision_id}: missing parent target; skipping"
            ));
            continue;
        };
        let Some(parent) = nif.get_block(parent_ref as usize).cloned() else {
            continue;
        };
        pending.push((
            string_field(&parent, "Name").unwrap_or_default(),
            parent.type_name.clone(),
        ));
        if let Some(parent_mut) = nif.blocks.get_mut(parent_ref as usize) {
            parent_mut.set_field("Collision Object", NifValue::Ref(-1));
        }
        collect_collision_subtree(nif, collision_id, &mut remove);
    }

    remove_blocks(nif, remove);

    let mut regenerated = 0usize;
    for (parent_name, parent_type) in pending {
        let Some(parent_id) = find_node_by_name_and_type(nif, &parent_name, &parent_type) else {
            report.warnings.push(format!(
                "Legacy collision parent {parent_name:?} not found after strip; skipping"
            ));
            continue;
        };
        let vertices = collect_geometry_vertices(nif, parent_id);
        if vertices.len() < 3 {
            report.warnings.push(format!(
                "Node {parent_name:?} has no FO4 triangle geometry after strip; skipping FO4 collision regeneration"
            ));
            continue;
        }
        if build_box_collision(nif, parent_id, &vertices).is_some() {
            ensure_root_havok_bsx_flag(nif, parent_id);
            regenerated += 1;
        }
    }

    report.changes.push(format!(
        "Legacy collision: stripped {} chain(s); regenerated {regenerated} FO4 collision object(s)",
        collision_ids.len()
    ));
}

fn collect_collision_subtree(nif: &NifFile, root_id: usize, out: &mut HashSet<usize>) {
    if !out.insert(root_id) {
        return;
    }
    let Some(block) = nif.get_block(root_id) else {
        return;
    };
    for field_name in ["Body", "Data", "Shape", "Sub Shapes"] {
        let Some(value) = block.get_field(field_name) else {
            continue;
        };
        match value {
            NifValue::Ref(id) if *id >= 0 => collect_collision_subtree(nif, *id as usize, out),
            NifValue::Int(id) if *id >= 0 => collect_collision_subtree(nif, *id as usize, out),
            NifValue::UInt(id) => collect_collision_subtree(nif, *id as usize, out),
            NifValue::Array(items) => {
                for item in items {
                    if let Some(id) = value_ref(Some(item)).filter(|id| *id >= 0) {
                        collect_collision_subtree(nif, id as usize, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn build_box_collision(nif: &mut NifFile, parent_id: usize, vertices: &[[f32; 3]]) -> Option<()> {
    let mut mins = vertices[0];
    let mut maxs = vertices[0];
    for vertex in vertices.iter().skip(1) {
        for axis in 0..3 {
            mins[axis] = mins[axis].min(vertex[axis]);
            maxs[axis] = maxs[axis].max(vertex[axis]);
        }
    }
    let half_extents = [
        (maxs[0] - mins[0]) * 0.5 / HAVOK_SCALE,
        (maxs[1] - mins[1]) * 0.5 / HAVOK_SCALE,
        (maxs[2] - mins[2]) * 0.5 / HAVOK_SCALE,
    ];

    let mut box_fields = IndexMap::new();
    box_fields.insert("Dimensions".to_string(), NifValue::Vec3(half_extents));
    box_fields.insert(
        "Radius".to_string(),
        NifValue::Float(DEFAULT_COLLISION_RADIUS),
    );
    let box_id = nif.add_block("bhkBoxShape", Some(box_fields));

    let mut rigid_fields = IndexMap::new();
    rigid_fields.insert("Shape".to_string(), NifValue::Ref(box_id as i32));
    rigid_fields.insert("Havok Filter".to_string(), havok_filter_struct(1));
    let rigid_id = nif.add_block("bhkRigidBody", Some(rigid_fields));
    if let Some(rigid) = nif.blocks.get_mut(rigid_id) {
        let mut info = match rigid.get_field("Rigid Body Info").cloned() {
            Some(NifValue::Struct(fields)) => fields,
            _ => IndexMap::new(),
        };
        info.insert("Havok Filter".to_string(), havok_filter_struct(1));
        info.insert("Mass".to_string(), NifValue::Float(0.0));
        info.insert("Friction".to_string(), NifValue::Float(0.5));
        info.insert("Restitution".to_string(), NifValue::Float(0.4));
        info.insert("Motion System".to_string(), NifValue::UInt(7));
        info.insert("Quality Type".to_string(), NifValue::UInt(1));
        rigid.set_field("Rigid Body Info", NifValue::Struct(info));
    }

    let mut collision_fields = IndexMap::new();
    collision_fields.insert("Flags".to_string(), NifValue::UInt(0x81));
    collision_fields.insert("Target".to_string(), NifValue::Ref(parent_id as i32));
    collision_fields.insert("Body".to_string(), NifValue::Ref(rigid_id as i32));
    let collision_id = nif.add_block("bhkCollisionObject", Some(collision_fields));
    let parent = nif.blocks.get_mut(parent_id)?;
    parent.set_field("Collision Object", NifValue::Ref(collision_id as i32));
    Some(())
}

fn havok_filter_struct(layer: u64) -> NifValue {
    let mut filter = IndexMap::new();
    filter.insert("Layer".to_string(), NifValue::UInt(layer));
    filter.insert("Flags".to_string(), NifValue::UInt(0));
    filter.insert("Group".to_string(), NifValue::UInt(0));
    NifValue::Struct(filter)
}

fn collect_geometry_vertices(nif: &NifFile, parent_id: usize) -> Vec<[f32; 3]> {
    let mut vertices = Vec::new();
    let mut visited = HashSet::new();
    collect_geometry_vertices_inner(nif, parent_id, &mut visited, &mut vertices);
    vertices
}

fn collect_geometry_vertices_inner(
    nif: &NifFile,
    block_id: usize,
    visited: &mut HashSet<usize>,
    out: &mut Vec<[f32; 3]>,
) {
    if !visited.insert(block_id) {
        return;
    }
    let Some(block) = nif.get_block(block_id) else {
        return;
    };
    if matches!(
        block.type_name.as_str(),
        "BSTriShape" | "BSSubIndexTriShape"
    ) {
        for vertex in value_array(block.get_field("Vertex Data")) {
            let Some(NifValue::Struct(fields)) = Some(vertex) else {
                continue;
            };
            if let Some(position) = fields
                .get("Vertex")
                .and_then(|value| vec3_value(Some(value)))
            {
                out.push(position);
            }
        }
        return;
    }
    if !is_node_type(&block.type_name) {
        return;
    }
    for child_id in ref_array(block.get_field("Children")) {
        if child_id >= 0 {
            collect_geometry_vertices_inner(nif, child_id as usize, visited, out);
        }
    }
}

fn is_node_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "NiNode"
            | "BSFadeNode"
            | "BSLeafAnimNode"
            | "BSOrderedNode"
            | "NiBillboardNode"
            | "NiSwitchNode"
    )
}

fn find_node_by_name_and_type(nif: &NifFile, name: &str, type_name: &str) -> Option<usize> {
    nif.blocks
        .iter()
        .find(|block| {
            block.type_name == type_name && string_field(block, "Name").unwrap_or_default() == name
        })
        .map(|block| block.block_id)
}

fn ensure_root_havok_bsx_flag(nif: &mut NifFile, node_id: usize) {
    ensure_root_bsx_flags(nif, node_id, BSX_HAVOK_FLAG);
}

fn ensure_root_bsx_flags(nif: &mut NifFile, node_id: usize, flags: u64) {
    let root_id = scene_root_id(nif, node_id);
    let mut extra_ids = nif
        .get_block(root_id)
        .and_then(|root| root.get_field("Extra Data List"))
        .map(|value| ref_array(Some(value)))
        .unwrap_or_default();
    for extra_id in extra_ids.iter().copied().filter(|id| *id >= 0) {
        let Some(extra) = nif.blocks.get_mut(extra_id as usize) else {
            continue;
        };
        if extra.type_name == "BSXFlags" {
            let current = value_u64(extra.get_field("Integer Data")).unwrap_or(0);
            extra.set_field("Integer Data", NifValue::UInt(current | flags));
            return;
        }
    }

    let mut fields = IndexMap::new();
    fields.insert("Name".to_string(), NifValue::String("BSX".to_string()));
    fields.insert("Integer Data".to_string(), NifValue::UInt(flags));
    let bsx_id = nif.add_block("BSXFlags", Some(fields));
    extra_ids.push(bsx_id as i32);
    let extra_count = extra_ids.len();
    if let Some(root) = nif.blocks.get_mut(root_id) {
        root.set_field(
            "Extra Data List",
            NifValue::Array(extra_ids.into_iter().map(NifValue::Ref).collect()),
        );
        root.set_field(
            "Num Extra Data List",
            NifValue::UInt(extra_count as u64),
        );
    }
}

const BSX_HAVOK_FLAG: u64 = 0x02;

fn reconcile_havok_bsx_flags(nif: &mut NifFile, report: &mut ConvertFileReport) {
    if has_live_collision_object(nif) {
        return;
    }

    let mut cleared = 0usize;
    for block in nif.blocks.iter_mut() {
        if block.type_name != "BSXFlags" {
            continue;
        }
        let current = value_u64(block.get_field("Integer Data")).unwrap_or(0);
        if current & BSX_HAVOK_FLAG == 0 {
            continue;
        }
        block.set_field("Integer Data", NifValue::UInt(current & !BSX_HAVOK_FLAG));
        cleared += 1;
    }

    if cleared > 0 {
        report.warnings.push(format!(
            "Havok/BSX: cleared stale Havok flag from {cleared} BSXFlags block(s); no live collision object remains after conversion"
        ));
    }
}

fn has_live_collision_object(nif: &NifFile) -> bool {
    nif.blocks.iter().any(|block| {
        field_ref(block, "Collision Object")
            .filter(|id| *id >= 0)
            .and_then(|id| nif.get_block(id as usize))
            .is_some_and(|collision| is_collision_object_type(&collision.type_name))
    })
}

fn is_collision_object_type(type_name: &str) -> bool {
    matches!(type_name, "bhkCollisionObject" | "bhkNPCollisionObject")
}

fn scene_root_id(nif: &NifFile, node_id: usize) -> usize {
    let mut current = node_id;
    let mut seen = HashSet::new();
    while seen.insert(current) {
        let Some(parent) = find_parent_node_id(nif, current) else {
            return current;
        };
        current = parent;
    }
    0
}

fn find_parent_node_id(nif: &NifFile, child_id: usize) -> Option<usize> {
    for block in nif.blocks.iter() {
        if !is_node_type(&block.type_name) {
            continue;
        }
        if ref_array(block.get_field("Children"))
            .iter()
            .any(|id| *id == child_id as i32)
        {
            return Some(block.block_id);
        }
    }
    None
}

fn normalize_fo4_root_node(
    nif: &mut NifFile,
    weapon_role: Option<&str>,
    normalize_fo76_static_flags: bool,
    preserve_scol_root_flags: bool,
    report: &mut ConvertFileReport,
) {
    let mut roots: Vec<usize> = nif
        .header
        .footer_roots
        .iter()
        .filter_map(|id| (*id >= 0).then_some(*id as usize))
        .collect();
    if roots.is_empty() && !nif.blocks.is_empty() {
        roots.push(0);
    }
    let mut converted = 0usize;
    for root_id in roots {
        let add_melee_marker = matches!(weapon_role, Some("melee"));
        let root_controller_is_manager = nif
            .blocks
            .get(root_id)
            .and_then(|root| root.get_field("Controller"))
            .map(NifValue::as_i64)
            .and_then(|controller| (controller >= 0).then_some(controller as usize))
            .and_then(|controller_id| nif.blocks.get(controller_id))
            .is_some_and(|controller| controller.type_name == "NiControllerManager");
        {
            let Some(root) = nif.blocks.get_mut(root_id) else {
                continue;
            };
            if root.type_name == "BSFadeNode" {
                root.type_name = "NiNode".to_string();
                converted += 1;
            }
            if root_controller_is_manager
                && root.type_name == "NiNode"
                && value_u64(root.get_field("Flags")).is_some_and(|flags| {
                    flags != FO4_NINODE_ROOT_FLAGS && (flags & !FO4_NINODE_ROOT_FLAGS) != 0
                })
            {
                root.set_field("Flags", NifValue::UInt(FO4_NINODE_ROOT_FLAGS));
                converted += 1;
            } else if normalize_fo76_static_flags
                && !preserve_scol_root_flags
                && root.type_name == "NiNode"
                && value_u64(root.get_field("Flags"))
                    .is_some_and(|flags| flags & FO76_STATIC_ROOT_FLAG != 0)
            {
                let flags = value_u64(root.get_field("Flags")).unwrap_or_default();
                root.set_field("Flags", NifValue::UInt(flags & !FO76_STATIC_ROOT_FLAG));
                converted += 1;
            }
            if root.get_field("Collision Object").is_none() {
                root.set_field("Collision Object", NifValue::Ref(-1));
                converted += 1;
            }
            if root.get_field("Children").is_none() {
                root.set_field("Children", NifValue::Array(Vec::new()));
                converted += 1;
            }
            let child_count = match root.get_field("Children") {
                Some(NifValue::Array(children)) => children.len() as u64,
                _ => 0,
            };
            if root.get_field("Num Children").map(NifValue::as_i64) != Some(child_count as i64) {
                root.set_field("Num Children", NifValue::UInt(child_count));
                converted += 1;
            }
            if matches!(weapon_role, Some("gun")) {
                let name = string_field(root, "Name").unwrap_or_default();
                if name != "Weapon" {
                    root.set_field("Name", NifValue::String("Weapon".to_string()));
                    converted += 1;
                }
            }
        }
        if add_melee_marker && ensure_root_weapon_marker(nif, root_id) {
            converted += 1;
        }
    }
    if converted > 0 {
        nif.rebuild_header();
        report
            .changes
            .push(format!("Normalized FO4 root data: {converted} change(s)"));
    }
}

fn is_scol_aggregate_nif(path: &Path, nif: &NifFile) -> bool {
    if path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("scol")
    }) {
        return true;
    }

    nif.blocks.first().is_some_and(|root| {
        let name = string_field(root, "Name")
            .unwrap_or_default()
            .to_ascii_lowercase();
        name.ends_with("_scol") || name.contains("statcoll")
    })
}

fn normalize_fo76_fo4_scene_node_flags(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut normalized = 0usize;
    for block in nif.blocks.iter_mut() {
        if !is_node_type(&block.type_name) {
            continue;
        }
        let Some(flags) = value_u64(block.get_field("Flags")) else {
            continue;
        };
        if flags & NIF_NODE_EDITOR_MARKER_FLAG == 0 {
            continue;
        }
        if string_field(block, "Name").as_deref() == Some("EditorMarker") {
            continue;
        }
        if flags & NIF_NODE_PRESERVE_HIGH_FLAG_COMPANION != 0 {
            continue;
        }
        block.set_field(
            "Flags",
            NifValue::UInt(flags & !NIF_NODE_EDITOR_MARKER_FLAG),
        );
        normalized += 1;
    }
    if normalized > 0 {
        report.changes.push(format!(
            "Normalized FO76 scene node flags for FO4 on {normalized} node(s)"
        ));
    }
}

fn ensure_root_weapon_marker(nif: &mut NifFile, root_id: usize) -> bool {
    let mut extra_ids = nif
        .get_block(root_id)
        .and_then(|root| root.get_field("Extra Data List"))
        .and_then(|value| match value {
            NifValue::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|item| match item {
                        NifValue::Ref(id) if *id >= 0 => Some(*id),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    if extra_ids.iter().copied().any(|extra_id| {
        nif.get_block(extra_id as usize).is_some_and(|extra| {
            extra.type_name == "NiStringExtraData"
                && string_field(extra, "Name").unwrap_or_default() == "WEAPON"
        })
    }) {
        return false;
    }

    let marker_id = nif.blocks.len();
    let mut marker = NifBlock::new(marker_id, "NiStringExtraData");
    marker.set_field("Name", NifValue::String("WEAPON".to_string()));
    marker.set_field("String Data", NifValue::String("WEAPON".to_string()));
    nif.blocks.push(marker);
    extra_ids.push(marker_id as i32);
    if let Some(root) = nif.blocks.get_mut(root_id) {
        root.set_field(
            "Extra Data List",
            NifValue::Array(
                extra_ids
                    .iter()
                    .copied()
                    .map(|id| NifValue::Int(id as i64))
                    .collect(),
            ),
        );
        root.set_field(
            "Num Extra Data List",
            NifValue::UInt(extra_ids.len() as u64),
        );
    }
    true
}

fn normalize_texture_sets(
    nif: &mut NifFile,
    source_game: &str,
    target_game: &str,
    texture_namespace: Option<&str>,
    texture_namespace_paths: &HashSet<String>,
    report: &mut ConvertFileReport,
) {
    let mut normalized = 0usize;
    for block in nif.blocks.iter_mut() {
        if block.type_name != "BSShaderTextureSet" {
            continue;
        }
        let mut textures = value_array(block.get_field("Textures"));
        textures.resize(FO4_TEXTURE_SLOT_COUNT, NifValue::String(String::new()));
        textures.truncate(FO4_TEXTURE_SLOT_COUNT);
        let mut changed = false;
        for texture in textures.iter_mut() {
            let NifValue::String(path) = texture else {
                continue;
            };
            if path.trim_end_matches('\0').trim().is_empty() {
                if !path.is_empty() {
                    *path = String::new();
                    changed = true;
                }
                continue;
            }
            let source_key = canonical_texture_path(path, "", "");
            let mut updated = canonical_texture_path(path, source_game, target_game);
            updated = namespace_texture_set_path(
                &updated,
                texture_namespace,
                texture_namespace_paths,
                Some(&source_key),
            );
            if updated != *path {
                *path = updated;
                changed = true;
            }
        }
        block.set_field(
            "Num Textures",
            NifValue::UInt(FO4_TEXTURE_SLOT_COUNT as u64),
        );
        block.set_field("Textures", NifValue::Array(textures));
        if changed {
            normalized += 1;
        }
    }
    if normalized > 0 {
        report.changes.push(format!(
            "BSShaderTextureSet: normalized {normalized} block's texture paths"
        ));
    }
}

fn normalize_external_material_names(
    nif: &mut NifFile,
    material_namespace: Option<&str>,
    material_namespace_paths: &HashSet<String>,
    report: &mut ConvertFileReport,
) {
    let mut normalized = 0usize;
    for block in nif.blocks.iter_mut() {
        if !is_shader_block_with_external_material(block) {
            continue;
        }
        let Some(name) = string_field(block, "Name") else {
            continue;
        };
        let mut updated = canonical_material_path(&name);
        updated = namespace_external_material_path(
            &updated,
            material_namespace,
            material_namespace_paths,
        );
        if updated == name {
            continue;
        }
        block.set_field("Name", NifValue::String(updated));
        normalized += 1;
    }
    if normalized > 0 {
        report.changes.push(format!(
            "Shader material paths: normalized {normalized} external material reference(s)"
        ));
    }
}

fn namespace_external_material_path(
    path: &str,
    material_namespace: Option<&str>,
    material_namespace_paths: &HashSet<String>,
) -> String {
    let Some(namespace) = material_namespace
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return path.to_owned();
    };
    let canonical = canonical_material_path(path);
    let lower = canonical.to_ascii_lowercase();
    if lower == FO4_DIRT_PATH_MATERIAL.to_ascii_lowercase()
        || lower.starts_with("materials\\template\\")
    {
        return canonical;
    }

    let namespace = namespace.trim_matches(|c| c == '/' || c == '\\');
    if namespace.is_empty() {
        return canonical;
    }
    if !material_namespace_paths.is_empty()
        && !material_namespace_paths.contains(&namespace_match_key(&canonical, namespace))
    {
        return canonical;
    }
    let mut parts: Vec<&str> = canonical
        .split('\\')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 2 || !parts[0].eq_ignore_ascii_case("materials") {
        return canonical;
    }
    if parts[1].eq_ignore_ascii_case(namespace) {
        parts[0] = "Materials";
        parts[1] = namespace;
        return parts.join("\\");
    }
    parts.insert(1, namespace);
    parts[0] = "Materials";
    parts.join("\\")
}

fn namespace_texture_set_path(
    path: &str,
    texture_namespace: Option<&str>,
    texture_namespace_paths: &HashSet<String>,
    source_path: Option<&str>,
) -> String {
    if path.trim_end_matches('\0').trim().is_empty() {
        return String::new();
    }
    let Some(namespace) = texture_namespace
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return path.to_owned();
    };
    let canonical = canonical_texture_path(path, "", "");
    let namespace = namespace.trim_matches(|c| c == '/' || c == '\\');
    if namespace.is_empty() {
        return canonical;
    }
    if !texture_namespace_paths.is_empty() {
        let output_key = namespace_match_key(&canonical, namespace);
        let source_key = source_path.map(|path| namespace_match_key(path, namespace));
        if !texture_namespace_paths.contains(&output_key)
            && !source_key
                .as_ref()
                .is_some_and(|key| texture_namespace_paths.contains(key))
        {
            return canonical;
        }
    }
    let mut parts: Vec<&str> = canonical
        .split('\\')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 2 || !parts[0].eq_ignore_ascii_case("textures") {
        return canonical;
    }
    if parts[1].eq_ignore_ascii_case(namespace) {
        parts[0] = "textures";
        parts[1] = namespace;
        return parts.join("\\");
    }
    parts.insert(1, namespace);
    parts[0] = "textures";
    parts.join("\\")
}

fn namespace_match_key(path: &str, namespace: &str) -> String {
    let mut parts: Vec<String> = path
        .replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect();
    if parts.len() >= 2 && parts[1].eq_ignore_ascii_case(namespace) {
        parts.remove(1);
    }
    parts.join("/")
}

fn is_shader_block_with_external_material(block: &NifBlock) -> bool {
    if !matches!(
        block.type_name.as_str(),
        "BSLightingShaderProperty" | "BSEffectShaderProperty"
    ) {
        return false;
    }
    let Some(name) = string_field(block, "Name") else {
        return false;
    };
    let lower = name.trim_end_matches('\0').trim().to_ascii_lowercase();
    lower.ends_with(".bgsm") || lower.ends_with(".bgem")
}

fn canonical_material_path(path: &str) -> String {
    canonical_asset_path(path, "materials", "Materials")
}

fn canonical_texture_path(path: &str, source_game: &str, target_game: &str) -> String {
    let mut normalized = path.trim_end_matches('\0').trim().replace('/', "\\");
    if source_game == "fo76" && target_game == "fo4" {
        normalized = rewrite_fo76_texture_path_to_fo4(&normalized).replace('/', "\\");
    }
    canonical_asset_path(&normalized, "textures", "textures")
}

fn canonical_asset_path(path: &str, root: &str, canonical_root: &str) -> String {
    let mut normalized = path.trim_end_matches('\0').trim().replace('/', "\\");
    while normalized.contains("\\\\") {
        normalized = normalized.replace("\\\\", "\\");
    }
    let parts_without_empty: Vec<&str> = normalized
        .split('\\')
        .filter(|part| !part.is_empty())
        .collect();
    if let Some(root_index) = parts_without_empty
        .iter()
        .rposition(|part| part.eq_ignore_ascii_case(root))
    {
        let mut rooted_parts = parts_without_empty[root_index..].to_vec();
        rooted_parts[0] = canonical_root;
        normalized = rooted_parts.join("\\");
    }
    let root_prefix = format!("{canonical_root}\\");
    if !normalized
        .to_ascii_lowercase()
        .starts_with(&root_prefix.to_ascii_lowercase())
    {
        normalized = format!("{canonical_root}\\{}", normalized.trim_start_matches('\\'));
    }
    let mut parts: Vec<&str> = normalized.split('\\').collect();
    if parts.len() > 1 && is_known_asset_prefix(parts[1]) {
        parts.remove(1);
    }
    parts.join("\\")
}

fn rewrite_fo76_texture_path_to_fo4(path: &str) -> String {
    let clean = path.trim_end_matches('\0');
    if clean.is_empty() {
        return path.to_owned();
    }
    let normalized = clean.replace('\\', "/");
    let (dir, basename) = normalized
        .rsplit_once('/')
        .map(|(dir, basename)| (format!("{dir}/"), basename))
        .unwrap_or_else(|| (String::new(), normalized.as_str()));
    if let Some(updated) = rename_fo76_character_eye_texture_to_fo4(&dir, basename) {
        return format!("{dir}{updated}");
    }
    match rename_fo76_texture_basename_to_fo4(basename) {
        Some(updated) => format!("{dir}{updated}"),
        None => normalized,
    }
}

fn rename_fo76_character_eye_texture_to_fo4(dir: &str, basename: &str) -> Option<String> {
    if !dir
        .trim_matches('/')
        .eq_ignore_ascii_case("actors/character/eyes")
    {
        return None;
    }
    let (stem, ext) = basename
        .rfind('.')
        .map(|dot| (&basename[..dot], &basename[dot..]))
        .unwrap_or((basename, ""));
    match stem.to_ascii_lowercase().as_str() {
        // The _d diffuse carries FO76's lash-strand alpha where the FO76 lash
        // geometry UVs sample; FO4's vanilla EyeBrown.dds has a different
        // layout there, so it must stay on the shipped FO76 texture. The name
        // has no FO4 collision. Bare iris diffuses map to the vanilla texture.
        "eyebro_d" | "eyebrown_d" => Some(format!("eyebrown_d{ext}")),
        "eyebro" | "eyebrown" => Some(format!("EyeBrown{ext}")),
        "eyebro_n" | "eyebrown_n" => Some(format!("EyeBrown_n{ext}")),
        "eyebro_r" | "eyebro_s" | "eyebrown_r" | "eyebrown_s" => Some(format!("Eye_s{ext}")),
        "eyebro_l" | "eyebro_g" | "eyebrown_l" | "eyebrown_g" => Some(format!("EyeBrown_sk{ext}")),
        "eyebrownbloodshot" => Some(format!("EyeBrownBloodshot{ext}")),
        _ => None,
    }
}

fn rename_fo76_texture_basename_to_fo4(basename: &str) -> Option<String> {
    let (stem, ext) = basename
        .rfind('.')
        .map(|dot| (&basename[..dot], &basename[dot..]))
        .unwrap_or((basename, ""));
    let lower = stem.to_ascii_lowercase();
    if lower.ends_with("_r") {
        return Some(format!("{}_s{}", &stem[..stem.len() - 2], ext));
    }
    if lower.ends_with("_l") {
        return Some(format!("{}_g{}", &stem[..stem.len() - 2], ext));
    }
    None
}

fn is_known_asset_prefix(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "fo4" | "fo76" | "fnv" | "fo3" | "skyrim" | "skyrimse" | "starfield" | "oblivion"
    )
}

fn remove_blocks(nif: &mut NifFile, remove: HashSet<usize>) {
    if remove.is_empty() {
        return;
    }
    let mut ids: Vec<usize> = remove.into_iter().collect();
    ids.sort_unstable();
    nif.remove_blocks(&ids);
}

fn replace_child_ref(nif: &mut NifFile, old_ref: i32, new_ref: i32) {
    for block in nif.blocks.iter_mut() {
        let Some(NifValue::Array(children)) = block.get_field("Children").cloned() else {
            continue;
        };
        let mut changed = false;
        let updated = children
            .into_iter()
            .map(|value| {
                if value_ref(Some(&value)) == Some(old_ref) {
                    changed = true;
                    NifValue::Ref(new_ref)
                } else {
                    value
                }
            })
            .collect();
        if changed {
            block.set_field("Children", NifValue::Array(updated));
        }
    }
}

fn field_ref(block: &NifBlock, name: &str) -> Option<i32> {
    value_ref(block.get_field(name))
}

fn value_ref(value: Option<&NifValue>) -> Option<i32> {
    match value? {
        NifValue::Ref(id) => Some(*id),
        NifValue::Int(id) => Some(*id as i32),
        NifValue::UInt(id) => Some(*id as i32),
        _ => None,
    }
}

fn value_usize(value: &NifValue) -> Option<usize> {
    match value {
        NifValue::Int(id) if *id >= 0 => Some(*id as usize),
        NifValue::UInt(id) => Some(*id as usize),
        NifValue::Ref(id) if *id >= 0 => Some(*id as usize),
        _ => None,
    }
}

fn value_u64(value: Option<&NifValue>) -> Option<u64> {
    match value? {
        NifValue::UInt(value) => Some(*value),
        NifValue::Int(value) if *value >= 0 => Some(*value as u64),
        NifValue::Ref(value) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn value_f64(value: Option<&NifValue>) -> Option<f64> {
    match value? {
        NifValue::Float(value) => Some(*value),
        NifValue::Int(value) => Some(*value as f64),
        NifValue::UInt(value) => Some(*value as f64),
        _ => None,
    }
}

fn value_array(value: Option<&NifValue>) -> Vec<NifValue> {
    match value {
        Some(NifValue::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

fn int_array(value: Option<&NifValue>) -> Vec<usize> {
    value_array(value).iter().filter_map(value_usize).collect()
}

fn ref_array(value: Option<&NifValue>) -> Vec<i32> {
    value_array(value)
        .iter()
        .filter_map(|value| value_ref(Some(value)))
        .collect()
}

fn cloned_field(block: &NifBlock, name: &str) -> NifValue {
    block.get_field(name).cloned().unwrap_or(NifValue::Null)
}

fn string_field(block: &NifBlock, name: &str) -> Option<String> {
    match block.get_field(name)? {
        NifValue::String(value) => Some(value.trim_end_matches('\0').to_string()),
        _ => None,
    }
}

fn int_field(block: &NifBlock, name: &str) -> Option<i64> {
    block.get_field(name).map(NifValue::as_i64)
}

fn vec3_value(value: Option<&NifValue>) -> Option<[f32; 3]> {
    match value? {
        NifValue::Vec3(value) => Some(*value),
        NifValue::Struct(fields) => Some([
            value_f64(fields.get("x")).unwrap_or(0.0) as f32,
            value_f64(fields.get("y")).unwrap_or(0.0) as f32,
            value_f64(fields.get("z")).unwrap_or(0.0) as f32,
        ]),
        _ => None,
    }
}

fn color4_value(value: &NifValue) -> Option<NifValue> {
    match value {
        NifValue::Color4(value) => Some(NifValue::Color4(*value)),
        NifValue::Struct(fields) => {
            let mut out = IndexMap::new();
            for key in ["r", "g", "b", "a"] {
                out.insert(
                    key.to_string(),
                    fields
                        .get(key)
                        .cloned()
                        .unwrap_or_else(|| NifValue::Float(1.0)),
                );
            }
            Some(NifValue::Struct(out))
        }
        _ => None,
    }
}

fn tex_coord_value(value: Option<&NifValue>) -> Option<NifValue> {
    match value? {
        NifValue::Struct(fields) => {
            let u = value_f64(fields.get("u")).unwrap_or(0.0) as f32;
            let v = value_f64(fields.get("v")).unwrap_or(0.0) as f32;
            Some(tex_coord([u, v]))
        }
        NifValue::Vec3(value) => Some(tex_coord([value[0], value[1]])),
        _ => None,
    }
}

fn tex_coord(uv: [f32; 2]) -> NifValue {
    let mut data = IndexMap::new();
    data.insert("u".to_string(), NifValue::Float(uv[0] as f64));
    data.insert("v".to_string(), NifValue::Float(uv[1] as f64));
    NifValue::Struct(data)
}

fn triangle(v1: i64, v2: i64, v3: i64) -> NifValue {
    let mut data = IndexMap::new();
    data.insert("v1".to_string(), NifValue::Int(v1));
    data.insert("v2".to_string(), NifValue::Int(v2));
    data.insert("v3".to_string(), NifValue::Int(v3));
    NifValue::Struct(data)
}

fn flag_names_to_bits(value: Option<&NifValue>, flags1: bool) -> u64 {
    match value {
        Some(NifValue::UInt(bits)) => *bits,
        Some(NifValue::Int(bits)) if *bits >= 0 => *bits as u64,
        Some(NifValue::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                NifValue::String(name) => flag_name_bit(name, flags1),
                _ => None,
            })
            .fold(0, |acc, bit| acc | (1u64 << bit)),
        Some(NifValue::String(name)) => flag_name_bit(name, flags1)
            .map(|bit| 1u64 << bit)
            .unwrap_or(0),
        _ => 0,
    }
}

fn flag_name_bit(name: &str, flags1: bool) -> Option<u64> {
    let key = name
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect::<String>();
    if flags1 {
        match key.as_str() {
            "specular" => Some(0),
            "skinned" => Some(1),
            "vertexalpha" => Some(3),
            "greyscaletopalettecolor" => Some(4),
            "greyscaletopalettealpha" => Some(5),
            "usefalloff" => Some(6),
            "environmentmapping" => Some(7),
            "rgbfalloff" => Some(8),
            "castshadows" => Some(9),
            "face" => Some(10),
            "modelspacenormals" => Some(12),
            "refraction" => Some(15),
            "hair" => Some(18),
            "skintint" => Some(21),
            "ownemit" => Some(22),
            "decal" => Some(26),
            "dynamicdecal" => Some(27),
            "externalemittance" => Some(29),
            "softeffect" => Some(30),
            "zbuffertest" => Some(31),
            _ => None,
        }
    } else {
        match key.as_str() {
            "zbufferwrite" => Some(0),
            "lodobjects" => Some(2),
            "nofade" => Some(3),
            "doublesided" => Some(4),
            "vertexcolors" => Some(5),
            "glowmap" => Some(6),
            "transformchanged" => Some(7),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::NifWriter;
    use crate::schema::NifSchema;

    #[test]
    fn animstatic_layer_bodies_are_keyframed() {
        // FO76→FO4 regression: an ANIMSTATIC (layer 2) collision body — an
        // animated door/shutter — must convert to a keyframed body so FO4 plays
        // the NIF's Open/Close NiControllerSequence. CivWarDoor01/02 (single
        // compressed-mesh body) silently became static (motionId=HK_INVALID)
        // because the install path hardcoded Static.
        assert_eq!(
            body_motion_type_for_layer(2),
            BodyMotionType::Keyframed,
            "ANIMSTATIC door body must be keyframed"
        );
        // STATIC (1) and other layers stay static set-dressing.
        assert_eq!(body_motion_type_for_layer(1), BodyMotionType::Static);
        assert_eq!(body_motion_type_for_layer(4), BodyMotionType::Static);
    }

    #[test]
    fn source_keyframed_motion_wins_over_layer() {
        // TireSwing01-style FO76 collision uses layer 4 plus motionType=KEYFRAMED.
        // Layer 4 alone is not enough to call it dynamic clutter; the source
        // motion type must stay keyframed.
        let metadata = SourceBodyMetadata {
            collision_filter_info: None,
            motion_type: Some(1), // hknpMotionType::KEYFRAMED
            has_ref_mass_distribution: true,
            ..SourceBodyMetadata::default()
        };

        assert_eq!(
            body_motion_type_for_source(metadata, FO4_CLUTTER_LAYER),
            BodyMotionType::Keyframed
        );
    }

    #[test]
    fn source_keyframed_motion_blocks_single_convex_dynamic_fallback() {
        let metadata = SourceBodyMetadata {
            collision_filter_info: None,
            motion_type: Some(1), // hknpMotionType::KEYFRAMED
            has_ref_mass_distribution: true,
            ..SourceBodyMetadata::default()
        };
        let intent = NifCollisionIntent {
            bsx_flags: BSX_DYNAMIC_FLAG | BSX_COMPLEX_FLAG,
            has_dynamic_bsx: true,
            has_complex_bsx: true,
        };
        let body = ExtractedCollisionBody {
            body_id: 1,
            source_polytopes: Vec::new(),
            meshes: vec![havok_native::collision::PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                triangles: vec![[0, 1, 2]],
            }],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: None,
            is_dynamic: true,
        };

        assert!(
            !source_body_is_dynamic_for_nif(metadata, intent, Some(&body)),
            "keyframed refmass bodies must not be re-promoted by the single-convex fallback"
        );
    }

    #[test]
    fn dynamic_noncomplex_compound_is_not_loose_clutter() {
        let metadata = SourceBodyMetadata {
            collision_filter_info: None,
            motion_type: Some(2), // hknpMotionType::DYNAMIC
            has_ref_mass_distribution: true,
            ..SourceBodyMetadata::default()
        };
        let intent = NifCollisionIntent {
            bsx_flags: BSX_DYNAMIC_FLAG,
            has_dynamic_bsx: true,
            has_complex_bsx: false,
        };
        let body = ExtractedCollisionBody {
            body_id: 0,
            source_polytopes: Vec::new(),
            meshes: vec![
                havok_native::collision::PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    triangles: vec![[0, 1, 2]],
                },
                havok_native::collision::PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: vec![[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]],
                    triangles: vec![[0, 1, 2]],
                },
            ],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: None,
            is_dynamic: true,
        };

        assert!(
            !source_body_is_dynamic_for_nif(metadata, intent, Some(&body)),
            "WhitespringLamp03Off-style non-complex compounds must not become FO4 DynamicCompound shapes"
        );
    }

    #[test]
    fn dynamic_noncomplex_single_convex_remains_loose_clutter() {
        let metadata = SourceBodyMetadata {
            collision_filter_info: None,
            motion_type: Some(2), // hknpMotionType::DYNAMIC
            has_ref_mass_distribution: true,
            ..SourceBodyMetadata::default()
        };
        let intent = NifCollisionIntent {
            bsx_flags: BSX_DYNAMIC_FLAG,
            has_dynamic_bsx: true,
            has_complex_bsx: false,
        };
        let body = ExtractedCollisionBody {
            body_id: 0,
            source_polytopes: Vec::new(),
            meshes: vec![havok_native::collision::PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                triangles: vec![[0, 1, 2]],
            }],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: None,
            is_dynamic: true,
        };

        assert!(
            source_body_is_dynamic_for_nif(metadata, intent, Some(&body)),
            "GaussPistolReceiverDummy-style single convex bodies must remain dynamic"
        );
    }

    #[test]
    fn aabb_fallback_keeps_dynamic_assembly_children_static() {
        assert_eq!(resolve_aabb_fallback_layer(Some(19), true, true), 19);
        assert_eq!(
            resolve_aabb_fallback_layer(Some(FO4_CLUTTER_LAYER), false, true),
            FO4_STATIC_LAYER
        );
        assert_eq!(
            resolve_aabb_fallback_layer(Some(FO4_CLUTTER_LAYER), false, false),
            FO4_STATIC_LAYER
        );
        assert_eq!(
            resolve_aabb_fallback_layer(Some(19), true, false),
            FO4_CLUTTER_LAYER
        );
    }

    #[test]
    fn stale_havok_bsx_without_collision_is_cleared_and_warned() {
        let mut nif = NifFile::default();
        let mut bsx = NifBlock::new(0, "BSXFlags");
        bsx.set_field("Integer Data", NifValue::UInt(202));
        nif.blocks.push(bsx);

        let mut report = ConvertFileReport::default();
        reconcile_havok_bsx_flags(&mut nif, &mut report);

        assert_eq!(
            value_u64(nif.blocks[0].get_field("Integer Data")),
            Some(200)
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("Havok/BSX: cleared stale Havok flag"))
        );
    }

    #[test]
    fn havok_bsx_with_live_collision_is_preserved() {
        let mut nif = NifFile::default();
        let mut root = NifBlock::new(0, "NiNode");
        root.set_field("Collision Object", NifValue::Ref(2));
        let mut bsx = NifBlock::new(1, "BSXFlags");
        bsx.set_field("Integer Data", NifValue::UInt(202));
        let collision = NifBlock::new(2, "bhkNPCollisionObject");
        nif.blocks.push(root);
        nif.blocks.push(bsx);
        nif.blocks.push(collision);

        let mut report = ConvertFileReport::default();
        reconcile_havok_bsx_flags(&mut nif, &mut report);

        assert_eq!(
            value_u64(nif.blocks[1].get_field("Integer Data")),
            Some(202)
        );
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn fo76_marker_flags_are_cleared_from_non_marker_scene_nodes() {
        let mut nif = NifFile::default();
        let mut root = NifBlock::new(0, "NiNode");
        root.set_field("Name", NifValue::String("ToxicStone01".to_string()));
        root.set_field("Flags", NifValue::UInt(0x2000_500E));
        root.set_field("Children", NifValue::Array(vec![NifValue::Ref(1)]));

        let mut child = NifBlock::new(1, "NiNode");
        child.set_field("Name", NifValue::String("L1_ToxicStone01".to_string()));
        child.set_field("Flags", NifValue::UInt(0x2000_000E));

        let mut marker = NifBlock::new(2, "NiNode");
        marker.set_field("Name", NifValue::String("EditorMarker".to_string()));
        marker.set_field("Flags", NifValue::UInt(0x2000_000E));

        let mut bridge = NifBlock::new(3, "NiNode");
        bridge.set_field("Name", NifValue::String("TempBridge01".to_string()));
        bridge.set_field("Flags", NifValue::UInt(0x2008_000E));

        nif.blocks.extend([root, child, marker, bridge]);
        let mut report = ConvertFileReport::default();

        normalize_fo76_fo4_scene_node_flags(&mut nif, &mut report);

        assert_eq!(value_u64(nif.blocks[0].get_field("Flags")), Some(0x500E));
        assert_eq!(value_u64(nif.blocks[1].get_field("Flags")), Some(0x000E));
        assert_eq!(
            value_u64(nif.blocks[2].get_field("Flags")),
            Some(0x2000_000E)
        );
        assert_eq!(
            value_u64(nif.blocks[3].get_field("Flags")),
            Some(0x2008_000E)
        );
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("Normalized FO76 scene node flags for FO4"))
        );
    }

    #[test]
    fn fo76_cloth_bone_name_matches_hair_and_generic_sim_bones() {
        // FaceGeom hair cloth-sim bones.
        assert!(is_fo76_cloth_bone_name("Hair_C_Cloth00"));
        assert!(is_fo76_cloth_bone_name("Hair_L_Cloth01"));
        assert!(is_fo76_cloth_bone_name("Hair_R_Cloth02"));
        // Existing generic cloth rig naming.
        assert!(is_fo76_cloth_bone_name("Cloth_BoneA00"));
        assert!(is_fo76_cloth_bone_name("Cloth00\0"));
        // Ranger outfit cloth rig naming.
        assert!(is_fo76_cloth_bone_name("Cloth_A00"));
        // Non-sim nodes that merely contain "cloth" must NOT fold.
        assert!(!is_fo76_cloth_bone_name("Clothing"));
        assert!(!is_fo76_cloth_bone_name("Hair_Cloth_Root"));
        assert!(!is_fo76_cloth_bone_name("DefaultClothPose"));
        assert!(!is_fo76_cloth_bone_name("HEAD"));
    }

    #[test]
    fn fo76_headwear_segment_32_remaps_to_hairtop_30() {
        let mut nif = segmented_skin_nif(&["HEAD", "Head_skin"], false);
        let mut report = ConvertFileReport::default();

        normalize_fo76_headwear_segments(&mut nif, &mut report);

        let shape = segmented_test_shape(&nif);
        assert_eq!(segment_user_indices(shape), vec![0, 1, 30]);
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("FO76 headwear segments"))
        );
    }

    #[test]
    fn fo76_hood_chest_bones_remap_segment_32() {
        let mut nif = segmented_skin_nif(
            &["Chest_skin", "Chest_Rear_Skin", "HEAD", "Head_skin"],
            false,
        );
        let mut report = ConvertFileReport::default();

        normalize_fo76_headwear_segments(&mut nif, &mut report);

        let shape = segmented_test_shape(&nif);
        assert_eq!(segment_user_indices(shape), vec![0, 1, 30]);
    }

    #[test]
    fn fo76_hazmat_mask_support_bones_remap_segment_32() {
        let mut nif = segmented_skin_nif(
            &[
                "Chest",
                "Chest_skin",
                "LArm_Collarbone",
                "LArm_Collarbone_skin",
                "Neck_Low_skin",
                "Neck_skin",
                "RArm_Collarbone",
                "RArm_Collarbone_skin",
                "HEAD",
                "Head_skin",
                "Neck",
                "Neck1_skin",
            ],
            false,
        );
        let mut report = ConvertFileReport::default();

        normalize_fo76_headwear_segments(&mut nif, &mut report);

        let shape = segmented_test_shape(&nif);
        assert_eq!(segment_user_indices(shape), vec![0, 1, 30]);
    }

    #[test]
    fn fo76_body_segment_32_is_not_remapped() {
        let mut nif = segmented_skin_nif(&["Pelvis"], false);
        let mut report = ConvertFileReport::default();

        normalize_fo76_headwear_segments(&mut nif, &mut report);

        let shape = segmented_test_shape(&nif);
        assert_eq!(segment_user_indices(shape), vec![0, 1, 32]);
        assert!(report.changes.is_empty());
    }

    #[test]
    fn fo76_facegen_head_segment_32_is_not_remapped() {
        let mut nif = segmented_skin_nif(&["HEAD", "Head_skin"], true);
        let mut report = ConvertFileReport::default();

        normalize_fo76_headwear_segments(&mut nif, &mut report);

        let shape = segmented_test_shape(&nif);
        assert_eq!(segment_user_indices(shape), vec![0, 1, 32]);
        assert!(report.changes.is_empty());
    }

    fn segmented_skin_nif(bone_names: &[&str], facegen: bool) -> NifFile {
        let mut nif = NifFile::default();
        let mut root = NifBlock::new(0, "NiNode");
        root.set_field(
            "Name",
            NifValue::String(
                if facegen {
                    "BSFaceGenNiNodeSkinned"
                } else {
                    "Root"
                }
                .to_string(),
            ),
        );

        let mut blocks = vec![root];
        let mut bone_refs = Vec::new();
        for (index, name) in bone_names.iter().enumerate() {
            let block_id = index + 1;
            let mut bone = NifBlock::new(block_id, "NiNode");
            bone.set_field("Name", NifValue::String((*name).to_string()));
            bone_refs.push(NifValue::Ref(block_id as i32));
            blocks.push(bone);
        }

        let skin_id = bone_names.len() + 1;
        let shape_id = skin_id + 1;
        let mut skin = NifBlock::new(skin_id, "BSSkin::Instance");
        skin.set_field("Bones", NifValue::Array(bone_refs));
        let mut shape = NifBlock::new(shape_id, "BSSubIndexTriShape");
        shape.set_field("Skin", NifValue::Ref(skin_id as i32));
        shape.set_field(
            "Segment Data",
            NifValue::Struct(IndexMap::from([(
                "Per Segment Data".to_string(),
                NifValue::Array(
                    [0, 1, 32]
                        .into_iter()
                        .map(segment_user_index_entry)
                        .collect(),
                ),
            )])),
        );
        blocks.push(skin);
        blocks.push(shape);
        nif.blocks = blocks;
        nif
    }

    fn segmented_test_shape(nif: &NifFile) -> &NifBlock {
        nif.blocks
            .iter()
            .find(|block| block.type_name == "BSSubIndexTriShape")
            .expect("shape")
    }

    fn segment_user_index_entry(user_index: u64) -> NifValue {
        NifValue::Struct(IndexMap::from([(
            "User Index".to_string(),
            NifValue::UInt(user_index),
        )]))
    }

    fn segment_user_indices(shape: &NifBlock) -> Vec<u64> {
        let Some(NifValue::Struct(fields)) = shape.get_field("Segment Data") else {
            return Vec::new();
        };
        let Some(NifValue::Array(entries)) = fields.get("Per Segment Data") else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| match entry {
                NifValue::Struct(fields) => value_u64(fields.get("User Index")),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn facegen_main_hair_gets_glow_shader_textures_and_alpha() {
        fn shader_block(id: usize, flags1: u64) -> NifBlock {
            let mut b = NifBlock::new(id, "BSLightingShaderProperty");
            b.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_HAIR_TINT));
            b.set_field("Shader Flags 1", NifValue::UInt(flags1));
            b.set_field("Shader Flags 2", NifValue::UInt(SLSF2_VERTEX_COLORS as u64));
            b
        }
        let mut nif = NifFile::default();
        let mut root = NifBlock::new(0, "NiNode");
        root.set_field("Name", NifValue::String("Root".to_string()));
        let mut facegen = NifBlock::new(1, "NiNode");
        facegen.set_field(
            "Name",
            NifValue::String("BSFaceGenNiNodeSkinned".to_string()),
        );

        // Main hair shape -> shader 4, texset 5, alpha 6.
        let mut hair = NifBlock::new(2, "BSSubIndexTriShape");
        hair.set_field("Shader Property", NifValue::Ref(4));
        hair.set_field("Alpha Property", NifValue::Ref(6));
        // Hairline shape (decal) -> shader 7.
        let mut hairline = NifBlock::new(3, "BSSubIndexTriShape");
        hairline.set_field("Shader Property", NifValue::Ref(7));

        let mut hair_shader = shader_block(4, SLSF1_HAIR | SLSF1_SKINNED);
        hair_shader.set_field("Texture Set", NifValue::Ref(5));

        let mut texset = NifBlock::new(5, "BSShaderTextureSet");
        texset.set_field(
            "Textures",
            NifValue::Array(vec![
                NifValue::String(
                    r"textures\Actors\Character\Hair\HairLong01Grayscale_d.dds".into(),
                ),
                NifValue::String(r"textures\Actors\Character\Hair\HairLong01_n.dds".into()),
                NifValue::String(String::new()),
                NifValue::String(r"textures\Actors\Character\Hair\HairColor_LGrad_g.dds".into()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
                NifValue::String(r"textures\Actors\Character\Hair\HairDefault_s.dds".into()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
            ]),
        );

        let mut alpha = NifBlock::new(6, "NiAlphaProperty");
        alpha.set_field("Threshold", NifValue::UInt(168));

        // Decal hairline shader must be left untouched.
        let hairline_shader = shader_block(7, SLSF1_HAIR | SLSF1_DECAL);

        nif.blocks.extend([
            root,
            facegen,
            hair,
            hairline,
            hair_shader,
            texset,
            alpha,
            hairline_shader,
        ]);

        let mut report = ConvertFileReport::default();
        normalize_facegen_hair_shaders(&mut nif, &mut report);

        // Main hair shader -> Glow Shader + flags.
        let sh = nif.get_block(4).unwrap();
        assert_eq!(
            value_u64(sh.get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_GLOW)
        );
        let f1 = value_u64(sh.get_field("Shader Flags 1")).unwrap();
        assert!(f1 & SLSF1_SPECULAR != 0 && f1 & SLSF1_OWN_EMIT != 0 && f1 & SLSF1_HAIR != 0);
        let f2 = value_u64(sh.get_field("Shader Flags 2")).unwrap();
        assert!(f2 & SLSF2_GLOW_MAP != 0 && f2 & SLSF2_TRANSFORM_CHANGED != 0);

        // Texture slots: palette -> _d, flow + specular derived from the normal.
        let tex = value_array(nif.get_block(5).unwrap().get_field("Textures"));
        let slot = |i: usize| match &tex[i] {
            NifValue::String(s) => s.clone(),
            _ => String::new(),
        };
        assert_eq!(
            slot(3),
            r"textures\Actors\Character\Hair\HairColor_LGrad_d.dds"
        );
        assert_eq!(slot(2), r"textures\Actors\Character\Hair\HairLong01_f.dds");
        assert_eq!(slot(7), r"textures\Actors\Character\Hair\HairLong01_s.dds");

        // Alpha threshold normalized.
        assert_eq!(
            value_u64(nif.get_block(6).unwrap().get_field("Threshold")),
            Some(FO4_HAIR_ALPHA_THRESHOLD)
        );

        // Hairline decal shader untouched (stays Hair Tint).
        assert_eq!(
            value_u64(nif.get_block(7).unwrap().get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_HAIR_TINT)
        );
    }

    #[test]
    fn fo76_cloth_skin_bones_fold_to_supported_bone() {
        let mut nif = NifFile::default();
        let mut root = NifBlock::new(0, "NiNode");
        root.set_field("Name", NifValue::String("Root".to_string()));
        root.set_field("Num Children", NifValue::UInt(2));
        root.set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(1), NifValue::Ref(2)]),
        );
        let mut supported_bone = NifBlock::new(1, "NiNode");
        supported_bone.set_field("Name", NifValue::String("Pelvis".to_string()));
        let mut cloth_bone = NifBlock::new(2, "NiNode");
        cloth_bone.set_field("Name", NifValue::String("Cloth_BoneA00".to_string()));

        let mut skin_data = NifBlock::new(3, "BSSkin::BoneData");
        skin_data.set_field("Num Bones", NifValue::UInt(2));
        skin_data.set_field(
            "Bone List",
            NifValue::Array(vec![
                NifValue::Struct(IndexMap::from([(
                    "Translation".to_string(),
                    NifValue::Vec3([0.0, 0.0, 0.0]),
                )])),
                NifValue::Struct(IndexMap::from([(
                    "Translation".to_string(),
                    NifValue::Vec3([1.0, 0.0, 0.0]),
                )])),
            ]),
        );

        let mut skin = NifBlock::new(4, "BSSkin::Instance");
        skin.set_field("Data", NifValue::Ref(3));
        skin.set_field("Num Bones", NifValue::UInt(2));
        skin.set_field(
            "Bones",
            NifValue::Array(vec![NifValue::Ref(1), NifValue::Ref(2)]),
        );

        let mut shape = NifBlock::new(5, "BSSubIndexTriShape");
        shape.set_field("Skin", NifValue::Ref(4));
        shape.set_field(
            "Vertex Data",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                (
                    "Bone Indices".to_string(),
                    NifValue::Array(vec![
                        NifValue::UInt(0),
                        NifValue::UInt(1),
                        NifValue::UInt(0),
                        NifValue::UInt(0),
                    ]),
                ),
                (
                    "Bone Weights".to_string(),
                    NifValue::Array(vec![
                        NifValue::Float(0.25),
                        NifValue::Float(0.75),
                        NifValue::Float(0.0),
                        NifValue::Float(0.0),
                    ]),
                ),
            ]))]),
        );

        nif.blocks
            .extend([root, supported_bone, cloth_bone, skin_data, skin, shape]);

        let mut report = ConvertFileReport::default();
        fold_fo76_cloth_skin_bones(&mut nif, &mut report);

        assert!(
            nif.blocks
                .iter()
                .all(|block| !is_fo76_cloth_bone_block(block))
        );
        let root = nif
            .blocks
            .iter()
            .find(|block| string_field(block, "Name").as_deref() == Some("Root"))
            .unwrap();
        let children = ref_array(root.get_field("Children"));
        assert_eq!(value_u64(root.get_field("Num Children")), Some(1));
        assert_eq!(children.len(), 1);
        assert_eq!(
            string_field(nif.get_block(children[0] as usize).unwrap(), "Name").as_deref(),
            Some("Pelvis")
        );

        let skin = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSSkin::Instance")
            .unwrap();
        let bones = ref_array(skin.get_field("Bones"));
        assert_eq!(value_u64(skin.get_field("Num Bones")), Some(1));
        assert_eq!(bones.len(), 1);
        assert_eq!(
            string_field(nif.get_block(bones[0] as usize).unwrap(), "Name").as_deref(),
            Some("Pelvis")
        );

        let data_id = field_ref(skin, "Data").unwrap() as usize;
        let skin_data = nif.get_block(data_id).unwrap();
        assert_eq!(value_u64(skin_data.get_field("Num Bones")), Some(1));
        assert_eq!(value_array(skin_data.get_field("Bone List")).len(), 1);

        let shape = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSSubIndexTriShape")
            .unwrap();
        let vertices = value_array(shape.get_field("Vertex Data"));
        let NifValue::Struct(vertex) = &vertices[0] else {
            panic!("expected vertex struct");
        };
        let indices: Vec<_> = value_array(vertex.get("Bone Indices"))
            .iter()
            .filter_map(value_usize)
            .collect();
        let weights = value_array(vertex.get("Bone Weights"));
        assert_eq!(indices, vec![0, 0, 0, 0]);
        assert_eq!(value_f64(weights.first()), Some(1.0));
        assert!(
            weights
                .iter()
                .skip(1)
                .all(|value| value_f64(Some(value)) == Some(0.0))
        );
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("folded 1 cloth skin bone"))
        );
    }

    #[test]
    fn fo76_water_shader_flattens_to_fo4_fields() {
        let mut nif = NifFile::default();
        let mut water = NifBlock::new(0, "BSWaterShaderProperty");
        water.set_field("Num SF1", NifValue::UInt(2));
        water.set_field(
            "SF1",
            NifValue::Array(vec![NifValue::UInt(1740048692), NifValue::UInt(3166356979)]),
        );
        water.set_field("Num SF2", NifValue::UInt(1));
        water.set_field("SF2", NifValue::Array(vec![NifValue::UInt(2893749418)]));
        water.set_field("UV Offset", tex_coord([0.0, 0.0]));
        water.set_field("UV Scale", tex_coord([1.0, 1.0]));
        water.set_field("Water Shader Flags", NifValue::UInt(FO4_WATER_SHADER_FLAGS));
        water.remainder = vec![1, 2, 3, 4];
        nif.blocks.push(water);

        let mut report = ConvertFileReport::default();
        flatten_fo76_water_shader(&mut nif, &mut report);

        let water = &nif.blocks[0];
        assert_eq!(
            value_u64(water.get_field("Shader Flags 1")),
            Some(FO4_WATER_SHADER_FLAGS_1)
        );
        assert_eq!(
            value_u64(water.get_field("Shader Flags 2")),
            Some(FO4_WATER_SHADER_FLAGS_2)
        );
        assert_eq!(
            value_u64(water.get_field("Water Shader Flags")),
            Some(FO4_WATER_SHADER_FLAGS)
        );
        assert!(water.get_field("SF1").is_none());
        assert!(water.get_field("SF2").is_none());
        assert!(water.remainder.is_empty());
        assert!(report.changes.iter().any(|change| {
            change.contains("BSWaterShaderProperty") && change.contains("dropped 3")
        }));
    }

    #[test]
    fn skin_tint_shader_gets_fo4_skin_tint_conditional_fields() {
        // FO76 skin/hair shaders flatten to Shader Type 5 without the FO4
        // Skin Tint Color/Alpha trailing fields. FO4 reads those 16 bytes for
        // type 5 regardless, so they MUST be present or the engine desyncs.
        let mut block = NifBlock::new(0, "BSLightingShaderProperty");
        block.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_SKIN_TINT));
        block.set_field("Wetness", default_fo4_wetness());

        ensure_fo4_lighting_shader_tail_fields(&mut block);

        match block.get_field("Skin Tint Color") {
            Some(NifValue::Color3(c)) => assert_eq!(*c, [1.0, 1.0, 1.0]),
            _ => panic!("type 5 shader missing Skin Tint Color"),
        }
        assert_eq!(value_f64(block.get_field("Skin Tint Alpha")), Some(1.0));
    }

    #[test]
    fn hair_tint_shader_gets_fo4_hair_tint_conditional_field() {
        let mut block = NifBlock::new(0, "BSLightingShaderProperty");
        block.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_HAIR_TINT));
        block.set_field("Wetness", default_fo4_wetness());

        ensure_fo4_lighting_shader_tail_fields(&mut block);

        match block.get_field("Hair Tint Color") {
            Some(NifValue::Color3(c)) => assert_eq!(*c, [1.0, 1.0, 1.0]),
            _ => panic!("type 6 shader missing Hair Tint Color"),
        }
    }

    #[test]
    fn fo76_hair_shader_type_5_becomes_fo4_hair_tint_before_tail_fields() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_SKIN_TINT));
        shader.set_field("Shader Flags 1", NifValue::UInt(SLSF1_HAIR));
        shader.set_field("Wetness", default_fo4_wetness());
        shader.set_field("Hair Tint Color", NifValue::Color3([0.25, 0.5, 0.75]));
        shader.set_field("Skin Tint Color", NifValue::Color3([1.0, 1.0, 1.0]));
        shader.set_field("Skin Tint Alpha", NifValue::Float(1.0));
        nif.blocks.push(shader);

        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report);

        let shader = &nif.blocks[0];
        assert_eq!(
            value_u64(shader.get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_HAIR_TINT)
        );
        match shader.get_field("Hair Tint Color") {
            Some(NifValue::Color3(color)) => assert_eq!(*color, [0.25, 0.5, 0.75]),
            other => panic!("expected preserved Hair Tint Color, got {other:?}"),
        }
        assert!(shader.get_field("Skin Tint Color").is_none());
        assert!(shader.get_field("Skin Tint Alpha").is_none());
        assert!(
            report
                .changes
                .iter()
                .any(|change| { change.contains("normalized FO76 hair tint shader type") })
        );
    }

    #[test]
    fn fo76_skin_shader_type_5_stays_skin_tint_when_not_hair() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_SKIN_TINT));
        shader.set_field("Wetness", default_fo4_wetness());
        nif.blocks.push(shader);

        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report);

        let shader = &nif.blocks[0];
        assert_eq!(
            value_u64(shader.get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_SKIN_TINT)
        );
        assert!(shader.get_field("Hair Tint Color").is_none());
        assert!(shader.get_field("Skin Tint Color").is_some());
        assert!(shader.get_field("Skin Tint Alpha").is_some());
    }

    #[test]
    fn flatten_renumbers_fo76_tint_shader_types_and_splits_skin_tint_rgba() {
        // Live repro: FO76 facegen shapes use BSShaderType155 (no Parallax
        // slot) — face = 3, skin tint = 4 with a Color4 tint. FO4 needs
        // 4/5 plus Color3 + Skin Tint Alpha or the writer drops the tint.
        let mut nif = NifFile::default();

        let mut face = NifBlock::new(0, "BSLightingShaderProperty");
        let mut face_spd: IndexMap<String, NifValue> = IndexMap::new();
        face_spd.insert("Shader Type".into(), NifValue::UInt(3));
        face.set_field("Shader Property Data", NifValue::Struct(face_spd));
        nif.blocks.push(face);

        let mut skin = NifBlock::new(1, "BSLightingShaderProperty");
        let mut skin_spd: IndexMap<String, NifValue> = IndexMap::new();
        skin_spd.insert("Shader Type".into(), NifValue::UInt(4));
        skin_spd.insert(
            "Skin Tint Color".into(),
            NifValue::Color4([0.9, 0.8, 0.7, 1.0]),
        );
        skin.set_field("Shader Property Data", NifValue::Struct(skin_spd));
        nif.blocks.push(skin);

        let mut report = ConvertFileReport::default();
        flatten_fo76_lighting_shader(&mut nif, &mut report);

        assert_eq!(value_u64(nif.blocks[0].get_field("Shader Type")), Some(4));
        assert!(nif.blocks[0].get_field("Skin Tint Color").is_none());
        assert_eq!(
            value_u64(nif.blocks[1].get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_SKIN_TINT)
        );
        match nif.blocks[1].get_field("Skin Tint Color") {
            Some(NifValue::Color3(color)) => assert_eq!(*color, [0.9, 0.8, 0.7]),
            other => panic!("expected Color3 skin tint, got {other:?}"),
        }
        assert_eq!(
            value_f64(nif.blocks[1].get_field("Skin Tint Alpha")),
            Some(1.0)
        );
    }

    #[test]
    fn flatten_renumbers_fo76_hair_and_eye_envmap_types() {
        let mut nif = NifFile::default();
        for (id, fo76_type) in [(0, 5u64), (1, 12u64)] {
            let mut shader = NifBlock::new(id, "BSLightingShaderProperty");
            let mut spd: IndexMap<String, NifValue> = IndexMap::new();
            spd.insert("Shader Type".into(), NifValue::UInt(fo76_type));
            shader.set_field("Shader Property Data", NifValue::Struct(spd));
            nif.blocks.push(shader);
        }

        let mut report = ConvertFileReport::default();
        flatten_fo76_lighting_shader(&mut nif, &mut report);

        assert_eq!(
            value_u64(nif.blocks[0].get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_HAIR_TINT)
        );
        assert_eq!(value_u64(nif.blocks[1].get_field("Shader Type")), Some(16));
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("renumbered 2 FO76 shader type(s)"))
        );
    }

    #[test]
    fn environment_map_shader_gets_full_fo4_conditional_fields() {
        let mut block = NifBlock::new(0, "BSLightingShaderProperty");
        block.set_field("Shader Type", NifValue::UInt(1));
        block.set_field("Wetness", default_fo4_wetness());

        ensure_fo4_lighting_shader_tail_fields(&mut block);

        assert_eq!(
            value_f64(block.get_field("Environment Map Scale")),
            Some(1.0)
        );
        assert!(block.get_field("Use Screen Space Reflections").is_some());
        assert!(block.get_field("Wetness Control: Use SSR").is_some());
    }

    #[test]
    fn default_shader_does_not_get_type_conditional_fields() {
        let mut block = NifBlock::new(0, "BSLightingShaderProperty");
        block.set_field("Shader Type", NifValue::UInt(0));
        block.set_field("Wetness", default_fo4_wetness());

        ensure_fo4_lighting_shader_tail_fields(&mut block);

        assert!(block.get_field("Skin Tint Color").is_none());
        assert!(block.get_field("Hair Tint Color").is_none());
        assert!(block.get_field("Environment Map Scale").is_none());
    }

    #[test]
    fn np_collision_diagnostic_context_includes_nif_and_parent() {
        let mut nif = NifFile::default();
        nif.path = Some(PathBuf::from(
            "X:\\extracted\\fo76\\meshes\\test\\example.nif",
        ));
        let planned = PlannedCollisionBody {
            source_body_id: 7,
            route: CollisionRoute::SourceCompound,
            layer: 1,
            material_crc: None,
            shape: MultiBodyShape::Polytope {
                vertices: Vec::new(),
            },
        };

        let context = np_collision_entry_diagnostic_context(&nif, 42, "CollisionParent", &planned);

        assert!(context.contains("example.nif"), "{context}");
        assert!(context.contains("block=42"), "{context}");
        assert!(context.contains("name=\"CollisionParent\""), "{context}");
        assert!(context.contains("body=7"), "{context}");
    }

    #[test]
    fn multi_body_collision_placement_comes_from_baked_geometry_not_body_transform() {
        // Regression guard for the "drop the source body transform" hypothesis.
        //
        // The shape vertices reaching `install_fo4_np_collision_system` are already
        // world/NIF-baked — the FO76 decode never applies `bodyCinfo.position`, so
        // each body's geometry carries its own world offset. We therefore keep
        // `BodyMeta.position` at origin; copying the source body transform here would
        // double-apply the offset.
        //
        // This builds two bodies whose convex shapes sit at DISTINCT, pre-offset
        // world positions (a "left" box at X≈0 and a "right" box at X≈+5 Havok
        // units) and asserts the rebuilt FO4 collision blob preserves both bodies at
        // their distinct geometry offsets — proving placement survives purely on the
        // baked geometry while the body frame stays at origin.
        use havok_native::collision::extract_preview_meshes_from_blob;

        fn box_at(center_x: f32) -> Vec<[f32; 3]> {
            // 8 corners of a unit Havok box, translated +center_x on X.
            let mut v = Vec::with_capacity(8);
            for &x in &[-0.5_f32, 0.5] {
                for &y in &[-0.5_f32, 0.5] {
                    for &z in &[-0.5_f32, 0.5] {
                        v.push([x + center_x, y, z]);
                    }
                }
            }
            v
        }

        let mut nif = NifFile::new("fo4");
        // Two distinct parent NiNodes, each at origin (the world offset lives in the
        // collision geometry, exactly like the FO76 ammo / cryo-debris multi-body
        // NIFs this models).
        let left_parent = nif.add_block("NiNode", None);
        let right_parent = nif.add_block("NiNode", None);

        let entries = vec![
            CollisionPlanEntry {
                source_collision_id: 10,
                source_parent_id: left_parent,
                source_parent_name: "LeftBox".to_string(),
                parent_id: left_parent,
                parent_name: "LeftBox".to_string(),
                planned: PlannedCollisionBody {
                    source_body_id: 0,
                    route: CollisionRoute::SourcePolytope,
                    layer: 1,
                    material_crc: None,
                    shape: MultiBodyShape::Polytope {
                        vertices: box_at(0.0),
                    },
                },
                source: None,
                source_metadata: SourceBodyMetadata::default(),
                nif_collision_intent: NifCollisionIntent::default(),
                in_multi_body_assembly: false,
                body_mass: None,
                mass_distribution: None,
            },
            CollisionPlanEntry {
                source_collision_id: 11,
                source_parent_id: right_parent,
                source_parent_name: "RightBox".to_string(),
                parent_id: right_parent,
                parent_name: "RightBox".to_string(),
                planned: PlannedCollisionBody {
                    source_body_id: 1,
                    route: CollisionRoute::SourcePolytope,
                    layer: 1,
                    material_crc: None,
                    shape: MultiBodyShape::Polytope {
                        vertices: box_at(5.0),
                    },
                },
                source: None,
                source_metadata: SourceBodyMetadata::default(),
                nif_collision_intent: NifCollisionIntent::default(),
                in_multi_body_assembly: false,
                body_mass: None,
                mass_distribution: None,
            },
        ];

        let installed =
            install_fo4_np_collision_system(&mut nif, &entries, None).expect("install collision");
        assert_eq!(installed, 2);

        let physics = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "bhkPhysicsSystem")
            .expect("physics system block");
        let blob = crate::cloth::byte_array_to_bytes(
            physics.get_field("Binary Data").expect("binary data"),
        )
        .expect("blob bytes");

        // Re-decode each body's geometry from the rebuilt blob. With the body frame
        // at origin, the X-center of each body must still equal its authored offset.
        let center_x = |body_id: usize| -> f32 {
            let meshes = extract_preview_meshes_from_blob(&blob, 1.0, Some(body_id))
                .expect("preview meshes");
            let xs: Vec<f32> = meshes
                .iter()
                .flat_map(|mesh| mesh.vertices.iter().map(|v| v[0]))
                .collect();
            assert!(!xs.is_empty(), "body {body_id} produced no geometry");
            let lo = xs.iter().cloned().fold(f32::INFINITY, f32::min);
            let hi = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            (lo + hi) / 2.0
        };

        let left_x = center_x(0);
        let right_x = center_x(1);
        assert!(
            left_x.abs() < 0.25,
            "left body geometry must stay at X≈0, got {left_x}"
        );
        assert!(
            (right_x - 5.0).abs() < 0.25,
            "right body geometry must stay at its baked X≈5 offset, got {right_x}"
        );
        assert!(
            (right_x - left_x) > 4.0,
            "the two bodies must remain distinctly placed by their baked geometry \
             alone (no body transform applied); separation was {}",
            right_x - left_x
        );
    }

    #[test]
    fn collision_change_summary_reports_shape_layer_and_motion_changes() {
        let entry = CollisionPlanEntry {
            source_collision_id: 5,
            source_parent_id: 42,
            source_parent_name: "CollisionParent".to_string(),
            parent_id: 42,
            parent_name: "CollisionParent".to_string(),
            planned: PlannedCollisionBody {
                source_body_id: 7,
                route: CollisionRoute::ClutterConvex,
                layer: FO4_CLUTTER_LAYER,
                material_crc: Some(0xC0EB_623D),
                shape: MultiBodyShape::Polytope {
                    vertices: vec![[0.0, 0.0, 0.0]; 8],
                },
            },
            source: Some(SourceCollisionSummary {
                shape_kind: "compressed_mesh".to_string(),
                shape_summary: "compressed_mesh(120v/240t)".to_string(),
                layer: Some(29),
                material_crc: Some(0x1234_5678),
            }),
            source_metadata: SourceBodyMetadata {
                collision_filter_info: None,
                layer: Some(29),
                body_flags: Some(128),
                material_crc: Some(0x1234_5678),
                body_mass: Some(2.0),
                motion_type: Some(2), // hknpMotionType::DYNAMIC
                has_ref_mass_distribution: true,
                is_dynamic: true,
            },
            nif_collision_intent: NifCollisionIntent {
                bsx_flags: BSX_DYNAMIC_FLAG | BSX_COMPLEX_FLAG,
                has_dynamic_bsx: true,
                has_complex_bsx: true,
            },
            in_multi_body_assembly: false,
            body_mass: Some(2.0),
            mass_distribution: None,
        };
        let entries = vec![entry];

        let summary = summarize_collision_changes(&entries);

        assert_eq!(summary.shape_changes, 1);
        assert_eq!(summary.layer_changes, 1);
        assert_eq!(summary.motion_info_changes, 1);
        assert_eq!(summary.details.len(), 1);
        assert!(
            summary.details[0].contains("compressed_mesh(120v/240t) -> polytope(8v)"),
            "{:?}",
            summary.details
        );
        assert!(
            summary.details[0]
                .contains("motion dynamic-refmass -> dynamic-clutter+motionCinfo+clutter-mass"),
            "{:?}",
            summary.details
        );

        let mut nif = NifFile::default();
        nif.path = Some(PathBuf::from(
            "X:\\extracted\\fo76\\meshes\\test\\example.nif",
        ));
        let routes = summarize_collision_routes(&nif, &entries);
        assert_eq!(routes.len(), 1);
        for expected in [
            "example.nif",
            "src_block=5",
            "source_parent=block=42 name=\"CollisionParent\"",
            "body=7",
            "route=clutter-convex",
            "source_shape=compressed_mesh(120v/240t)",
            "output_shape=polytope(8v)",
            "filter=layer 29->4",
            "material 0x12345678->0xC0EB623D",
            "motion dynamic-refmass -> dynamic-clutter+motionCinfo+clutter-mass",
            "meta=motion_type:dynamic(2),flags:0x80,mass:2.000,refmass:no,dynamic:yes,bsx:0x48(dynamic:yes,complex:yes),assembly:single",
        ] {
            assert!(routes[0].contains(expected), "{expected}: {:?}", routes);
        }
    }

    fn texture_set_block(block_id: usize) -> NifBlock {
        let mut block = NifBlock::new(block_id, "BSShaderTextureSet");
        let mut textures = vec![NifValue::String(String::new()); 15];
        textures[0] = NifValue::String("Landscape/Plants/Bramble01_d.dds".to_owned());
        textures[1] = NifValue::String("Landscape/Plants/Bramble01_n.dds".to_owned());
        textures[9] = NifValue::String("Landscape/Plants/Bramble01_r.dds".to_owned());
        textures[10] = NifValue::String("Landscape/Plants/Bramble01_l.dds".to_owned());
        block.set_field("Num Textures", NifValue::UInt(15));
        block.set_field("Textures", NifValue::Array(textures));
        block
    }

    fn vault_texture_set_block(block_id: usize) -> NifBlock {
        let mut block = NifBlock::new(block_id, "BSShaderTextureSet");
        let mut textures = vec![NifValue::String(String::new()); 15];
        textures[0] = NifValue::String("Interiors/Vault/Vault76Concrete01_d.dds".to_owned());
        textures[1] = NifValue::String("Interiors/Vault/Vault76Concrete01_n.dds".to_owned());
        textures[9] = NifValue::String("Interiors/Vault/Vault76Concrete01_r.dds".to_owned());
        block.set_field("Num Textures", NifValue::UInt(15));
        block.set_field("Textures", NifValue::Array(textures));
        block
    }

    fn lighting_shader_block(
        block_id: usize,
        texture_set_id: i32,
        flags1: u64,
        flags2: u64,
    ) -> NifBlock {
        let mut block = NifBlock::new(block_id, "BSLightingShaderProperty");
        block.set_field("Texture Set", NifValue::Ref(texture_set_id));
        block.set_field("Shader Flags 1", NifValue::UInt(flags1));
        block.set_field("Shader Flags 2", NifValue::UInt(flags2));
        block.set_field("Emissive Color", NifValue::Color3([0.0, 0.0, 0.0]));
        block.set_field("Emissive Multiple", NifValue::Float(1.0));
        block
    }

    fn texture_at(block: &NifBlock, index: usize) -> String {
        let textures = value_array(block.get_field("Textures"));
        match &textures[index] {
            NifValue::String(value) => value.clone(),
            _ => String::new(),
        }
    }

    fn bsvaluenode(block_id: usize, name: &str, value: i64) -> NifBlock {
        let mut block = NifBlock::new(block_id, "BSValueNode");
        block.set_field("Name", NifValue::String(name.to_string()));
        block.set_field("Value", NifValue::Int(value));
        block
    }

    fn block_name(block: &NifBlock) -> String {
        match block.get_field("Name") {
            Some(NifValue::String(value)) => value.clone(),
            _ => String::new(),
        }
    }

    fn block_value(block: &NifBlock) -> i64 {
        match block.get_field("Value") {
            Some(NifValue::Int(value)) => *value,
            _ => i64::MIN,
        }
    }

    #[test]
    fn addon_node_index_parses_fo76_suffix_and_digits() {
        assert_eq!(addon_node_index("AddOnNode078@#0"), Some((78, "078")));
        assert_eq!(addon_node_index("AddOnNode78"), Some((78, "78")));
        assert_eq!(
            addon_node_index("AddOnNode760001"),
            Some((760001, "760001"))
        );
        assert_eq!(addon_node_index("addonnode12@#3"), Some((12, "12")));
        assert_eq!(addon_node_index("NotANode"), None);
        assert_eq!(addon_node_index("AddOnNode"), None);
    }

    #[test]
    fn patch_addon_strips_suffix_with_empty_map() {
        let mut nif = NifFile::default();
        nif.blocks.push(bsvaluenode(0, "AddOnNode078@#0", 0));
        let mut report = ConvertFileReport::default();
        patch_addon_node_indices(&mut nif, &HashMap::new(), &mut report);
        assert_eq!(block_name(&nif.blocks[0]), "AddOnNode078");
        assert_eq!(
            block_value(&nif.blocks[0]),
            78,
            "Value is restored from the AddOnNode name for unmapped nodes"
        );
    }

    #[test]
    fn patch_addon_remaps_index_and_name() {
        let mut nif = NifFile::default();
        nif.blocks.push(bsvaluenode(0, "AddOnNode078@#0", 78));
        let mut map: HashMap<i64, i64> = HashMap::new();
        map.insert(78, 760_001);
        let mut report = ConvertFileReport::default();
        patch_addon_node_indices(&mut nif, &map, &mut report);
        assert_eq!(block_name(&nif.blocks[0]), "AddOnNode760001");
        assert_eq!(block_value(&nif.blocks[0]), 760_001);
    }

    #[test]
    fn patch_addon_clean_fo4_name_is_noop() {
        let mut nif = NifFile::default();
        nif.blocks.push(bsvaluenode(0, "AddOnNode78", 78));
        let mut report = ConvertFileReport::default();
        patch_addon_node_indices(&mut nif, &HashMap::new(), &mut report);
        assert_eq!(block_name(&nif.blocks[0]), "AddOnNode78");
        assert!(
            report.changes.is_empty(),
            "clean name must not be rewritten"
        );
    }

    #[test]
    fn remap_fo76_texture_slots_drops_non_emissive_lighting_slot() {
        let mut nif = NifFile::default();
        nif.blocks.push(lighting_shader_block(0, 1, 0, 0));
        nif.blocks.push(texture_set_block(1));

        let mut report = ConvertFileReport::default();
        remap_fo76_texture_slots(&mut nif, &mut report);

        let texset = &nif.blocks[1];
        assert_eq!(texture_at(texset, 2), "");
        assert_eq!(texture_at(texset, 7), "Landscape/Plants/Bramble01_r.dds");
        assert_eq!(texture_at(texset, 9), "");
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("dropped non-emissive"))
        );
    }

    #[test]
    fn remap_fo76_texture_slots_keeps_lighting_slot_for_emissive_shader() {
        let mut nif = NifFile::default();
        nif.blocks.push(lighting_shader_block(0, 1, 0, 1u64 << 6));
        nif.blocks.push(texture_set_block(1));

        let mut report = ConvertFileReport::default();
        remap_fo76_texture_slots(&mut nif, &mut report);

        let texset = &nif.blocks[1];
        assert_eq!(texture_at(texset, 2), "Landscape/Plants/Bramble01_l.dds");
        assert_eq!(texture_at(texset, 7), "Landscape/Plants/Bramble01_r.dds");
        assert_eq!(texture_at(texset, 9), "");
    }

    #[test]
    fn ensure_fo4_lighting_shader_tail_fields_completes_partial_wetness() {
        let mut block = NifBlock::new(0, "BSLightingShaderProperty");
        block.set_field("Rimlight Power", NifValue::Float(5.0));
        block.set_field(
            "Wetness",
            NifValue::Struct(IndexMap::from([
                ("Spec Scale".to_string(), NifValue::Float(-1.0)),
                ("Spec Power".to_string(), NifValue::Float(-1.0)),
                ("Min Var".to_string(), NifValue::Float(-1.0)),
                ("Fresnel Power".to_string(), NifValue::Float(-1.0)),
            ])),
        );

        assert!(ensure_fo4_lighting_shader_tail_fields(&mut block));
        assert!(block.get_field("Subsurface Rolloff").is_some());
        assert!(block.get_field("Rimlight Power").is_some());
        assert!(block.get_field("Backlight Power").is_some());
        let wetness = match block.get_field("Wetness") {
            Some(NifValue::Struct(fields)) => fields,
            other => panic!("expected wetness struct, got {other:?}"),
        };
        assert!(wetness.contains_key("Env Map Scale"));
        assert!(wetness.contains_key("Metalness"));
    }

    #[test]
    fn external_bgsm_shader_defaults_use_default_path() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String(
                "Materials\\Interiors\\Vault\\Vault76ExteriorGearDoor04.BGSM".to_string(),
            ),
        );
        nif.blocks.push(shader);

        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report);

        let shader = &nif.blocks[0];
        assert_eq!(
            value_u64(shader.get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_DEFAULT)
        );
        assert!(
            value_u64(shader.get_field("Shader Flags 1"))
                .is_some_and(|flags| flags & SLSF1_ENVIRONMENT_MAPPING == 0)
        );
        assert_eq!(value_f64(shader.get_field("Environment Map Scale")), None);
        assert!(shader.get_field("Use Screen Space Reflections").is_none());
        assert!(shader.fields.get("Wetness Control: Use SSR").is_none());
    }

    #[test]
    fn external_bgsm_default_shader_serializes_without_envmap_tail_booleans() {
        let schema = NifSchema::from_generated();
        let mut nif = NifFile::new("fo4");
        let shader_id = nif.blocks.len();
        let mut shader = NifBlock::new(shader_id, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String(
                "Materials\\Interiors\\Vault\\Vault76ExteriorGearDoor04.BGSM".to_string(),
            ),
        );
        shader.set_field("Num Extra Data List", NifValue::UInt(0));
        shader.set_field("Extra Data List", NifValue::Array(Vec::new()));
        shader.set_field("Controller", NifValue::Ref(-1));
        nif.blocks.push(shader);
        nif.rebuild_header();

        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report);
        assert_eq!(
            value_u64(nif.blocks[shader_id].get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_DEFAULT)
        );
        assert!(
            nif.blocks[shader_id]
                .get_field("Use Screen Space Reflections")
                .is_none()
        );
        assert!(
            nif.blocks[shader_id]
                .fields
                .get("Wetness Control: Use SSR")
                .is_none()
        );
        NifWriter::write_to_bytes(&mut nif, &schema).expect("serialize envmap shader");
    }

    #[test]
    fn grass_model_shader_defaults_do_not_use_environment_map_path() {
        let mut nif = NifFile::default();
        nif.path = Some(PathBuf::from(
            "meshes\\Landscape\\Grass\\RiverRockGrassObj01.nif",
        ));
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String("Materials\\Landscape\\Rocks\\RockRiverStones.BGSM".to_string()),
        );
        shader.set_field("Texture Set", NifValue::Ref(1));
        nif.blocks.push(shader);
        nif.blocks.push(vault_texture_set_block(1));

        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report);
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            None,
            &HashMap::new(),
            &mut report,
        );

        let shader = &nif.blocks[0];
        assert_eq!(
            value_u64(shader.get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_DEFAULT)
        );
        assert!(
            value_u64(shader.get_field("Shader Flags 1"))
                .is_some_and(|flags| flags & SLSF1_ENVIRONMENT_MAPPING == 0)
        );
        assert_eq!(texture_at(&nif.blocks[1], 4), "");
        assert_eq!(value_f64(shader.get_field("Environment Map Scale")), None);
    }

    fn write_glow_bgsm(dir: &Path, relative: &str, glowmap: bool) {
        let mut bgsm = materials_native::bgsm::BgsmData {
            EmitEnabled: glowmap,
            Glowmap: glowmap,
            ..Default::default()
        };
        bgsm.header.signature = materials_native::bgsm::BGSM_SIGNATURE;
        bgsm.header.version = 2;
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, materials_native::bgsm::write(&bgsm)).unwrap();
    }

    fn external_bgsm_shader_nif(material_name: &str) -> NifFile {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field("Name", NifValue::String(material_name.to_string()));
        shader.set_field("Texture Set", NifValue::Ref(1));
        nif.blocks.push(shader);
        // Texture set with a diffuse in slot 0; the glow slot derives X_d -> X_g.
        let mut texset = NifBlock::new(1, "BSShaderTextureSet");
        let mut textures = vec![NifValue::String(String::new()); FO4_TEXTURE_SLOT_COUNT];
        textures[0] = NifValue::String("textures\\TestGlow\\Board_d.dds".to_string());
        texset.set_field(
            "Num Textures",
            NifValue::UInt(FO4_TEXTURE_SLOT_COUNT as u64),
        );
        texset.set_field("Textures", NifValue::Array(textures));
        nif.blocks.push(texset);
        nif
    }

    #[test]
    fn external_bgsm_glow_material_sets_glow_map_flag_and_slot() {
        // Explicit glow material → Glow_Map flag set and slot 2 bound. FO76
        // LightingTexture-derived static-object emission is disabled earlier in
        // material downgrade, so this path is reserved for real glow materials.
        let dir = tempfile::tempdir().unwrap();
        write_glow_bgsm(&dir.path(), "materials/testglow/glowboard.bgsm", true);
        write_glow_bgsm(&dir.path(), "materials/testglow/plainboard.bgsm", false);

        // Glow material → Glow_Map flag set AND slot 2 bound to the glow texture.
        let mut nif = external_bgsm_shader_nif("Materials\\TestGlow\\GlowBoard.bgsm");
        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report);
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            Some(dir.path()),
            &HashMap::new(),
            &mut report,
        );
        assert!(
            value_u64(nif.blocks[0].get_field("Shader Flags 2"))
                .is_some_and(|flags| flags & SLSF2_GLOW_MAP != 0),
            "glow-emitting material must set the Glow_Map flag"
        );
        assert_eq!(
            value_u64(nif.blocks[0].get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_GLOW),
            "glow-emitting material must use the Glow Shader type so FO4 selects \
             the glow-map technique (Default type applies emittance unmasked)"
        );
        assert_eq!(
            texture_at(&nif.blocks[1], 2),
            "textures\\TestGlow\\Board_g.dds",
            "glow slot 2 must be bound to the diffuse-derived glow texture"
        );
        assert!(
            value_u64(nif.blocks[0].get_field("Shader Flags 1"))
                .is_some_and(|flags| flags & SLSF1_OWN_EMIT != 0),
            "glow-emitting material must keep Own_Emit"
        );

        // Non-glow material → Glow_Map off and slot 2 stays empty.
        let mut nif = external_bgsm_shader_nif("Materials\\TestGlow\\PlainBoard.bgsm");
        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report);
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            Some(dir.path()),
            &HashMap::new(),
            &mut report,
        );
        assert!(
            value_u64(nif.blocks[0].get_field("Shader Flags 2"))
                .is_some_and(|flags| flags & SLSF2_GLOW_MAP == 0),
            "non-glow material must not set the Glow_Map flag"
        );
        assert_eq!(
            value_u64(nif.blocks[0].get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_DEFAULT),
            "non-glow material must stay the Default shader type"
        );
        assert_eq!(
            texture_at(&nif.blocks[1], 2),
            "",
            "non-glow material must not bind a glow texture"
        );
        assert!(
            value_u64(nif.blocks[0].get_field("Shader Flags 1"))
                .is_some_and(|flags| flags & SLSF1_OWN_EMIT == 0),
            "non-glow material must not apply unmasked emittance"
        );

        // No source dir → behavior unchanged (flag off, slot 2 empty).
        let mut nif = external_bgsm_shader_nif("Materials\\TestGlow\\GlowBoard.bgsm");
        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report);
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            None,
            &HashMap::new(),
            &mut report,
        );
        assert!(
            value_u64(nif.blocks[0].get_field("Shader Flags 2"))
                .is_some_and(|flags| flags & SLSF2_GLOW_MAP == 0),
            "without a source dir the Glow_Map flag must stay off"
        );
        assert_eq!(
            value_u64(nif.blocks[0].get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_DEFAULT),
            "without a source dir the shader type must stay Default"
        );
        assert_eq!(texture_at(&nif.blocks[1], 2), "");
    }

    #[test]
    fn material_texture_set_propagates_to_matching_external_shaders() {
        let mut nif = NifFile::default();
        let mut source_shader = NifBlock::new(0, "BSLightingShaderProperty");
        source_shader.set_field(
            "Name",
            NifValue::String("Materials\\Interiors\\Vault\\Vault76Concrete01.BGSM".to_string()),
        );
        source_shader.set_field("Texture Set", NifValue::Ref(2));
        let mut matching_shader = NifBlock::new(1, "BSLightingShaderProperty");
        matching_shader.set_field(
            "Name",
            NifValue::String("Materials\\Interiors\\Vault\\Vault76Concrete01.BGSM".to_string()),
        );
        nif.blocks.push(source_shader);
        nif.blocks.push(matching_shader);
        nif.blocks.push(vault_texture_set_block(2));

        let mut report = ConvertFileReport::default();
        propagate_texture_sets_by_material(&mut nif, &mut report);

        assert_eq!(field_ref(&nif.blocks[1], "Texture Set"), Some(2));
    }

    #[test]
    fn external_material_names_use_forced_namespace() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String("Materials\\Landscape\\Trees\\TreeForestLimbs.BGSM".to_string()),
        );
        nif.blocks.push(shader);

        let mut report = ConvertFileReport::default();
        normalize_external_material_names(&mut nif, Some("FO76"), &HashSet::new(), &mut report);

        assert_eq!(
            string_field(&nif.blocks[0], "Name").as_deref(),
            Some("Materials\\FO76\\Landscape\\Trees\\TreeForestLimbs.BGSM")
        );
    }

    #[test]
    fn external_material_names_use_selective_namespace_paths() {
        let mut nif = NifFile::default();
        let mut relocated = NifBlock::new(0, "BSLightingShaderProperty");
        relocated.set_field(
            "Name",
            NifValue::String("Materials\\SetDressing\\Crates\\WoodenCrate01Dest.BGSM".to_string()),
        );
        let mut local = NifBlock::new(1, "BSLightingShaderProperty");
        local.set_field(
            "Name",
            NifValue::String(
                "Materials\\Furniture\\WorkstationDistillery\\WorkstationDistillery01_Pipes.bgsm"
                    .to_string(),
            ),
        );
        nif.blocks.push(relocated);
        nif.blocks.push(local);

        let namespace_paths =
            HashSet::from([("materials/setdressing/crates/woodencrate01dest.bgsm".to_string())]);
        let mut report = ConvertFileReport::default();
        normalize_external_material_names(&mut nif, Some("FO76"), &namespace_paths, &mut report);

        assert_eq!(
            string_field(&nif.blocks[0], "Name").as_deref(),
            Some("Materials\\FO76\\SetDressing\\Crates\\WoodenCrate01Dest.BGSM")
        );
        assert_eq!(
            string_field(&nif.blocks[1], "Name").as_deref(),
            Some("Materials\\Furniture\\WorkstationDistillery\\WorkstationDistillery01_Pipes.bgsm")
        );
    }

    #[test]
    fn external_material_namespace_preserves_fo4_fallback_materials() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String("Materials\\Landscape\\Ground\\DirtPath01.bgsm".to_string()),
        );
        nif.blocks.push(shader);

        let mut report = ConvertFileReport::default();
        normalize_external_material_names(&mut nif, Some("FO76"), &HashSet::new(), &mut report);

        assert_eq!(
            string_field(&nif.blocks[0], "Name").as_deref(),
            Some("Materials\\Landscape\\Ground\\DirtPath01.bgsm")
        );
    }

    #[test]
    fn texture_sets_use_forced_namespace_and_preserve_empty_slots() {
        let mut nif = NifFile::default();
        let mut texset = NifBlock::new(0, "BSShaderTextureSet");
        texset.set_field(
            "Textures",
            NifValue::Array(vec![
                NifValue::String("textures\\Landscape\\Trees\\TreeForestBareFrayed_d.dds".into()),
                NifValue::String("textures\\Landscape\\Trees\\TreeForestBareFrayed_n.dds".into()),
                NifValue::String(String::new()),
                NifValue::String("   ".into()),
                NifValue::String("\0\0".into()),
            ]),
        );
        nif.blocks.push(texset);

        let mut report = ConvertFileReport::default();
        normalize_texture_sets(
            &mut nif,
            "fo76",
            "fo4",
            Some("FO76"),
            &HashSet::new(),
            &mut report,
        );

        assert_eq!(
            texture_at(&nif.blocks[0], 0),
            "textures\\FO76\\Landscape\\Trees\\TreeForestBareFrayed_d.dds"
        );
        assert_eq!(
            texture_at(&nif.blocks[0], 1),
            "textures\\FO76\\Landscape\\Trees\\TreeForestBareFrayed_n.dds"
        );
        assert_eq!(texture_at(&nif.blocks[0], 2), "");
        assert_eq!(texture_at(&nif.blocks[0], 3), "");
        assert_eq!(texture_at(&nif.blocks[0], 4), "");
    }

    #[test]
    fn texture_sets_use_selective_namespace_paths() {
        let mut nif = NifFile::default();
        let mut texset = NifBlock::new(0, "BSShaderTextureSet");
        texset.set_field(
            "Textures",
            NifValue::Array(vec![
                NifValue::String("textures\\SetDressing\\Crates\\WoodenCrate01Dest_d.dds".into()),
                NifValue::String(
                    "textures\\Furniture\\WorkstationDistillery\\WorkstationDistillery01_Pipes_d.dds"
                        .into(),
                ),
            ]),
        );
        nif.blocks.push(texset);

        let namespace_paths =
            HashSet::from([("textures/setdressing/crates/woodencrate01dest_d.dds".to_string())]);
        let mut report = ConvertFileReport::default();
        normalize_texture_sets(
            &mut nif,
            "fo76",
            "fo4",
            Some("FO76"),
            &namespace_paths,
            &mut report,
        );

        assert_eq!(
            texture_at(&nif.blocks[0], 0),
            "textures\\FO76\\SetDressing\\Crates\\WoodenCrate01Dest_d.dds"
        );
        assert_eq!(
            texture_at(&nif.blocks[0], 1),
            "textures\\Furniture\\WorkstationDistillery\\WorkstationDistillery01_Pipes_d.dds"
        );
    }

    #[test]
    fn texture_sets_map_fo76_character_eye_reflectivity_to_fo4_generic_specular() {
        let mut nif = NifFile::default();
        let mut texset = NifBlock::new(0, "BSShaderTextureSet");
        texset.set_field(
            "Textures",
            NifValue::Array(vec![NifValue::String(
                "Actors/Character/Eyes/EyeBrown_r.dds".into(),
            )]),
        );
        nif.blocks.push(texset);

        let mut report = ConvertFileReport::default();
        normalize_texture_sets(&mut nif, "fo76", "fo4", None, &HashSet::new(), &mut report);

        assert_eq!(
            texture_at(&nif.blocks[0], 0),
            "textures\\Actors\\Character\\Eyes\\Eye_s.dds"
        );
    }

    #[test]
    fn texture_sets_map_fo76_eyebro_reflectivity_to_fo4_generic_specular() {
        let mut nif = NifFile::default();
        let mut texset = NifBlock::new(0, "BSShaderTextureSet");
        texset.set_field(
            "Textures",
            NifValue::Array(vec![NifValue::String(
                "Actors/Character/Eyes/EyeBro_r.dds".into(),
            )]),
        );
        nif.blocks.push(texset);

        let mut report = ConvertFileReport::default();
        normalize_texture_sets(&mut nif, "fo76", "fo4", None, &HashSet::new(), &mut report);

        assert_eq!(
            texture_at(&nif.blocks[0], 0),
            "textures\\Actors\\Character\\Eyes\\Eye_s.dds"
        );
    }

    #[test]
    fn texture_sets_map_fo76_eyebro_lash_bundle_to_fo4_base_eye_brown_bundle() {
        let mut nif = NifFile::default();
        let mut texset = NifBlock::new(0, "BSShaderTextureSet");
        texset.set_field(
            "Textures",
            NifValue::Array(vec![
                NifValue::String("actors/character/eyes/eyebrown_d.dds".into()),
                NifValue::String("Actors/Character/Eyes/EyeBro_n.DDS".into()),
                NifValue::String("Actors/Character/Eyes/EyeBro_r.dds".into()),
                NifValue::String("Actors/Character/Eyes/EyeBro_s.DDS".into()),
                NifValue::String("Actors/Character/Eyes/EyeBrown_l.dds".into()),
                NifValue::String("Actors/Character/Eyes/EyeBro_g.dds".into()),
            ]),
        );
        nif.blocks.push(texset);

        let mut report = ConvertFileReport::default();
        normalize_texture_sets(&mut nif, "fo76", "fo4", None, &HashSet::new(), &mut report);

        assert_eq!(
            texture_at(&nif.blocks[0], 0),
            "textures\\actors\\character\\eyes\\eyebrown_d.dds"
        );
        assert_eq!(
            texture_at(&nif.blocks[0], 1),
            "textures\\Actors\\Character\\Eyes\\EyeBrown_n.DDS"
        );
        assert_eq!(
            texture_at(&nif.blocks[0], 2),
            "textures\\Actors\\Character\\Eyes\\Eye_s.dds"
        );
        assert_eq!(
            texture_at(&nif.blocks[0], 3),
            "textures\\Actors\\Character\\Eyes\\Eye_s.DDS"
        );
        assert_eq!(
            texture_at(&nif.blocks[0], 4),
            "textures\\Actors\\Character\\Eyes\\EyeBrown_sk.dds"
        );
        assert_eq!(
            texture_at(&nif.blocks[0], 5),
            "textures\\Actors\\Character\\Eyes\\EyeBrown_sk.dds"
        );
    }

    #[test]
    fn external_bgsm_shader_data_uses_converted_source_bgsm_texture_slots() {
        let dir = tempfile::tempdir().unwrap();
        let material_path = dir
            .path()
            .join("materials")
            .join("landscape")
            .join("trees")
            .join("treeforestbarefrayed.bgsm");
        std::fs::create_dir_all(material_path.parent().unwrap()).unwrap();

        let mut bgsm = materials_native::bgsm::BgsmData::default();
        bgsm.header.signature = materials_native::bgsm::BGSM_SIGNATURE;
        bgsm.header.version = 20;
        bgsm.DiffuseTexture = "Landscape/Trees/TreeForestBareFrayed_d.dds".to_string();
        bgsm.NormalTexture = "Landscape/Trees/TreeForestBareFrayed_n.dds".to_string();
        bgsm.SpecularTexture = Some("Landscape/Trees/TreeForestBare_r.dds".to_string());
        bgsm.LightingTexture = Some("Landscape/Trees/TreeForestBareFrayed_l.dds".to_string());
        std::fs::write(&material_path, materials_native::bgsm::write(&bgsm)).unwrap();

        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String("Materials\\Landscape\\Trees\\TreeForestBareFrayed.BGSM".to_string()),
        );
        nif.blocks.push(shader);

        let mut report = ConvertFileReport::default();
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            Some(dir.path()),
            &HashMap::new(),
            &mut report,
        );
        normalize_external_material_names(&mut nif, Some("FO76"), &HashSet::new(), &mut report);
        normalize_texture_sets(
            &mut nif,
            "fo76",
            "fo4",
            Some("FO76"),
            &HashSet::new(),
            &mut report,
        );

        assert_eq!(
            string_field(&nif.blocks[0], "Name").as_deref(),
            Some("Materials\\FO76\\Landscape\\Trees\\TreeForestBareFrayed.BGSM")
        );
        let texset_id = field_ref(&nif.blocks[0], "Texture Set").expect("texture set ref");
        let texset = nif.get_block(texset_id as usize).expect("texture set");
        assert_eq!(
            texture_at(texset, 0),
            "textures\\FO76\\Landscape\\Trees\\TreeForestBareFrayed_d.dds"
        );
        assert_eq!(
            texture_at(texset, 1),
            "textures\\FO76\\Landscape\\Trees\\TreeForestBareFrayed_n.dds"
        );
        assert_eq!(texture_at(texset, 2), "");
        assert_eq!(
            texture_at(texset, 7),
            "textures\\FO76\\Landscape\\Trees\\TreeForestBare_s.dds"
        );
    }

    #[test]
    fn external_bgsm_shader_data_uses_material_source_override_texture_slots() {
        let dir = tempfile::tempdir().unwrap();
        let material_path = dir
            .path()
            .join("materials")
            .join("landscape")
            .join("ground")
            .join("forestrocks01.bgsm");
        std::fs::create_dir_all(material_path.parent().unwrap()).unwrap();

        let mut bgsm = materials_native::bgsm::BgsmData::default();
        bgsm.header.signature = materials_native::bgsm::BGSM_SIGNATURE;
        bgsm.header.version = 20;
        bgsm.DiffuseTexture = "Landscape/Ground/ForestRocks01_d.dds".to_string();
        bgsm.NormalTexture = "Landscape/Ground/ForestRocks01_n.dds".to_string();
        bgsm.SmoothSpecTexture = "Landscape/Ground/ForestRocks01_s.dds".to_string();
        std::fs::write(&material_path, materials_native::bgsm::write(&bgsm)).unwrap();

        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String("Materials\\Landscape\\Ground\\TEMP_GroundTexture01.bgsm".to_string()),
        );
        shader.set_field("Texture Set", NifValue::Ref(1));
        nif.blocks.push(shader);

        let mut textures = vec![NifValue::String(String::new()); FO4_TEXTURE_SLOT_COUNT];
        textures[0] = NifValue::String("Landscape/Ground/TEMP_GroundTexture01_d.dds".to_string());
        textures[1] = NifValue::String("Landscape/Ground/TEMP_GroundTexture01_n.dds".to_string());
        textures[7] = NifValue::String("Landscape/Ground/TEMP_GroundTexture01_r.dds".to_string());
        let mut texset = NifBlock::new(1, "BSShaderTextureSet");
        texset.set_field(
            "Num Textures",
            NifValue::UInt(FO4_TEXTURE_SLOT_COUNT as u64),
        );
        texset.set_field("Textures", NifValue::Array(textures));
        nif.blocks.push(texset);

        let overrides = HashMap::from([(
            "materials/landscape/ground/temp_groundtexture01.bgsm".to_string(),
            "materials/landscape/ground/forestrocks01.bgsm".to_string(),
        )]);
        let mut report = ConvertFileReport::default();
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            Some(dir.path()),
            &overrides,
            &mut report,
        );

        assert_eq!(
            string_field(&nif.blocks[0], "Name").as_deref(),
            Some("Materials\\Landscape\\Ground\\TEMP_GroundTexture01.bgsm")
        );
        assert_eq!(
            texture_at(&nif.blocks[1], 0),
            "textures\\Landscape\\Ground\\ForestRocks01_d.dds"
        );
        assert_eq!(
            texture_at(&nif.blocks[1], 1),
            "textures\\Landscape\\Ground\\ForestRocks01_n.dds"
        );
        assert_eq!(
            texture_at(&nif.blocks[1], 7),
            "textures\\Landscape\\Ground\\ForestRocks01_s.dds"
        );
    }

    #[test]
    fn external_bgsm_shader_data_uses_specular_bundle_when_smoothspec_missing() {
        let dir = tempfile::tempdir().unwrap();
        let material_path = dir
            .path()
            .join("materials")
            .join("setdressing")
            .join("playerhouse_ruin")
            .join("playerhouse_ruin_kitchenrefrigerator05.bgsm");
        let texture_path = dir
            .path()
            .join("textures")
            .join("SetDressing")
            .join("PlayerHouse_Ruin")
            .join("playerhouse_ruin_kitchenrefrigerator01_r.DDS");
        std::fs::create_dir_all(material_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(texture_path.parent().unwrap()).unwrap();
        std::fs::write(&texture_path, b"r").unwrap();

        let mut bgsm = materials_native::bgsm::BgsmData::default();
        bgsm.header.signature = materials_native::bgsm::BGSM_SIGNATURE;
        bgsm.header.version = 20;
        bgsm.DiffuseTexture =
            "SetDressing/PlayerHouse_Ruin/playerhouse_ruin_kitchenrefrigerator05_d.dds".to_string();
        bgsm.NormalTexture =
            "SetDressing/PlayerHouse_Ruin/playerhouse_ruin_kitchenrefrigerator05_n.dds".to_string();
        bgsm.SmoothSpecTexture =
            "SetDressing/PlayerHouse_Ruin/playerhouse_ruin_kitchenrefrigerator05_s.dds".to_string();
        bgsm.SpecularTexture = Some(
            "SetDressing/PlayerHouse_Ruin/playerhouse_ruin_kitchenrefrigerator01_r.DDS".to_string(),
        );
        std::fs::write(&material_path, materials_native::bgsm::write(&bgsm)).unwrap();

        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String(
                "Materials\\SetDressing\\PlayerHouse_Ruin\\PlayerHouse_Ruin_KitchenRefrigerator05.BGSM"
                    .to_string(),
            ),
        );
        shader.set_field("Texture Set", NifValue::Ref(1));
        nif.blocks.push(shader);
        let mut texset = NifBlock::new(1, "BSShaderTextureSet");
        let mut textures = vec![NifValue::String(String::new()); FO4_TEXTURE_SLOT_COUNT];
        textures[6] = NifValue::String(
            "textures\\SetDressing\\PlayerHouse_Ruin\\playerhouse_ruin_kitchenrefrigerator05_s.dds"
                .to_string(),
        );
        texset.set_field(
            "Num Textures",
            NifValue::UInt(FO4_TEXTURE_SLOT_COUNT as u64),
        );
        texset.set_field("Textures", NifValue::Array(textures));
        nif.blocks.push(texset);

        let mut report = ConvertFileReport::default();
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            Some(dir.path()),
            &HashMap::new(),
            &mut report,
        );
        normalize_external_material_names(&mut nif, Some("FO76"), &HashSet::new(), &mut report);
        normalize_texture_sets(
            &mut nif,
            "fo76",
            "fo4",
            Some("FO76"),
            &HashSet::new(),
            &mut report,
        );

        let texset_id = field_ref(&nif.blocks[0], "Texture Set").expect("texture set ref");
        let texset = nif.get_block(texset_id as usize).expect("texture set");
        assert_eq!(texture_at(texset, 6), "");
        assert_eq!(
            texture_at(texset, 7).to_ascii_lowercase(),
            "textures\\fo76\\setdressing\\playerhouse_ruin\\playerhouse_ruin_kitchenrefrigerator01_s.dds"
        );
    }

    #[test]
    fn external_bgsm_shader_data_fills_nif_texture_slots() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String("Materials\\Interiors\\Vault\\Vault76Concrete01.BGSM".to_string()),
        );
        shader.set_field(
            "Shader Type",
            NifValue::UInt(BSLSP_SHADER_TYPE_ENVIRONMENT_MAP),
        );
        shader.set_field("Shader Flags 1", NifValue::UInt(SLSF1_ENVIRONMENT_MAPPING));
        shader.set_field("Environment Map Scale", NifValue::Float(1.0));
        shader.set_field("Use Screen Space Reflections", NifValue::Bool(false));
        shader.set_field("Texture Set", NifValue::Ref(1));
        nif.blocks.push(shader);
        let mut texset = NifBlock::new(1, "BSShaderTextureSet");
        texset.set_field("Num Textures", NifValue::UInt(0));
        texset.set_field("Textures", NifValue::Array(Vec::new()));
        nif.blocks.push(texset);

        let mut report = ConvertFileReport::default();
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            None,
            &HashMap::new(),
            &mut report,
        );

        assert_eq!(
            value_u64(nif.blocks[0].get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_DEFAULT)
        );
        assert!(
            value_u64(nif.blocks[0].get_field("Shader Flags 1"))
                .is_some_and(|flags| flags & SLSF1_ENVIRONMENT_MAPPING == 0)
        );
        assert_eq!(
            texture_at(&nif.blocks[1], 0),
            "textures\\Interiors\\Vault\\Vault76Concrete01_d.dds"
        );
        assert_eq!(
            texture_at(&nif.blocks[1], 1),
            "textures\\Interiors\\Vault\\Vault76Concrete01_n.dds"
        );
        assert_eq!(
            texture_at(&nif.blocks[1], 7),
            "textures\\Interiors\\Vault\\Vault76Concrete01_s.dds"
        );
        assert_eq!(texture_at(&nif.blocks[1], 2), "");
        assert!(
            nif.blocks[0]
                .get_field("Use Screen Space Reflections")
                .is_none()
        );
        assert!(
            nif.blocks[0]
                .fields
                .get("Wetness Control: Use SSR")
                .is_none()
        );
    }

    #[test]
    fn external_bgsm_rock_shader_uses_fo4_clamp_mode() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String(
                "Materials\\Landscape\\Rocks\\RockBoulderForest_SingleDraw01.BGSM".to_string(),
            ),
        );
        shader.set_field("Texture Clamp Mode", NifValue::UInt(3));
        nif.blocks.push(shader);

        let mut report = ConvertFileReport::default();
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            None,
            &HashMap::new(),
            &mut report,
        );

        assert_eq!(
            value_u64(nif.blocks[0].get_field("Texture Clamp Mode")),
            Some(TEX_CLAMP_MODE_CLAMP_S_CLAMP_T)
        );
    }

    /// FO76 temp-ground placeholder material references survive fo76->fo4
    /// conversion unchanged (NOT retargeted to FO4 DirtPath01). The mesh then uses
    /// the converted FO76 forestrocks01 material that `material_source_overrides`
    /// emits at the temp_groundtexture01 output path — the FO76 look the override
    /// (mirroring Bethesda's runtime MSWP swap to ForestRocks01) is meant to provide.
    #[test]
    fn fo76_temp_ground_material_is_preserved_not_retargeted() {
        let tmp = std::env::temp_dir().join("fo76_temp_ground_material_is_preserved");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let src = tmp.join("src.nif");
        let dst = tmp.join("dst.nif");

        let mut nif = NifFile::new("fo76");
        let mut fields = IndexMap::new();
        fields.insert(
            "Name".to_string(),
            NifValue::String("Materials\\Landscape\\Ground\\temp_groundtexture01.bgsm".to_string()),
        );
        nif.add_block("BSLightingShaderProperty", Some(fields));
        nif.save(Some(src.clone())).unwrap();

        convert_nif_file(
            &src,
            &dst,
            "fo76",
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .unwrap();

        let refs = NifFile::load(&dst).unwrap().referenced_asset_paths();
        let joined = refs.materials.join(",");
        assert!(
            joined.contains("temp_groundtexture01.bgsm"),
            "temp ground material reference must be preserved, got {joined:?}"
        );
        assert!(
            !joined.contains("dirtpath01"),
            "temp ground material must NOT be retargeted to FO4 DirtPath01, got {joined:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn invalid_environment_mapping_resets_shader_type() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field("Texture Set", NifValue::Ref(1));
        shader.set_field(
            "Shader Type",
            NifValue::UInt(BSLSP_SHADER_TYPE_ENVIRONMENT_MAP),
        );
        shader.set_field("Shader Flags 1", NifValue::UInt(SLSF1_ENVIRONMENT_MAPPING));
        let mut texset = NifBlock::new(1, "BSShaderTextureSet");
        texset.set_field(
            "Textures",
            NifValue::Array(vec![NifValue::String(String::new()); 9]),
        );
        nif.blocks.push(shader);
        nif.blocks.push(texset);

        let mut report = ConvertFileReport::default();
        clear_fo76_invalid_environment_mapping(&mut nif, &mut report);

        let shader = &nif.blocks[0];
        assert_eq!(
            value_u64(shader.get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_DEFAULT)
        );
        assert_eq!(value_u64(shader.get_field("Shader Flags 1")), Some(0));
    }

    #[test]
    fn strip_fo76_position_data_removes_blocks_and_remaps_remaining_refs() {
        let mut nif = NifFile::default();
        nif.blocks.push(NifBlock::new(0, "NiNode"));

        let mut shape = NifBlock::new(1, "BSTriShape");
        shape.set_field("Num Extra Data List", NifValue::UInt(3));
        shape.set_field(
            "Extra Data List",
            NifValue::Array(vec![NifValue::Ref(2), NifValue::Ref(3), NifValue::Ref(4)]),
        );
        shape.set_field("Shader Property", NifValue::Ref(5));
        shape.set_field("Vertex Desc", NifValue::UInt(0));
        shape.set_field("Num Triangles", NifValue::UInt(36));
        shape.set_field("Num Vertices", NifValue::UInt(38));
        shape.set_field("Data Size", NifValue::UInt(0));
        nif.blocks.push(shape);

        nif.blocks.push(NifBlock::new(2, "BSPositionData"));
        nif.blocks.push(NifBlock::new(3, "BSPositionData"));
        nif.blocks.push(NifBlock::new(4, "NiStringExtraData"));
        nif.blocks.push(lighting_shader_block(5, -1, 0, 0));

        let mut report = ConvertFileReport::default();
        strip_fo76_position_data(&mut nif, &mut report);

        let block_types: Vec<&str> = nif
            .blocks
            .iter()
            .map(|block| block.type_name.as_str())
            .collect();
        assert_eq!(
            block_types,
            vec![
                "NiNode",
                "BSTriShape",
                "NiStringExtraData",
                "BSLightingShaderProperty"
            ]
        );
        let shape = &nif.blocks[1];
        assert_eq!(
            shape.get_field("Num Extra Data List").map(NifValue::as_i64),
            Some(1)
        );
        assert!(matches!(
            shape.get_field("Extra Data List"),
            Some(NifValue::Array(values)) if matches!(values.as_slice(), [NifValue::Ref(2)])
        ));
        assert!(matches!(
            shape.get_field("Shader Property"),
            Some(NifValue::Ref(3))
        ));
        assert_eq!(
            shape.get_field("Num Triangles").map(NifValue::as_i64),
            Some(0)
        );
        assert_eq!(
            shape.get_field("Num Vertices").map(NifValue::as_i64),
            Some(0)
        );
        assert_eq!(
            shape.get_field("Vertex Desc").map(NifValue::as_i64),
            Some(0)
        );
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("Removed 2 FO76 BSPositionData"))
        );
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("Cleared geometry counts on 1"))
        );
    }

    #[test]
    fn strip_fo76_position_data_preserves_particle_emitter_mesh_geometry() {
        let mut nif = NifFile::default();
        nif.blocks.push(NifBlock::new(0, "NiNode"));

        let mut shape = NifBlock::new(1, "BSTriShape");
        shape.set_field("Num Extra Data List", NifValue::UInt(2));
        shape.set_field(
            "Extra Data List",
            NifValue::Array(vec![NifValue::Ref(2), NifValue::Ref(3)]),
        );
        shape.set_field("Vertex Desc", NifValue::UInt(0));
        shape.set_field("Num Triangles", NifValue::UInt(99));
        shape.set_field("Num Vertices", NifValue::UInt(177));
        shape.set_field("Data Size", NifValue::UInt(0));
        nif.blocks.push(shape);

        nif.blocks.push(NifBlock::new(2, "BSPositionData"));
        nif.blocks.push(NifBlock::new(3, "BSPositionData"));

        let mut emitter = NifBlock::new(4, "NiPSysMeshEmitter");
        emitter.set_field("Num Emitter Meshes", NifValue::UInt(1));
        emitter.set_field("Emitter Meshes", NifValue::Array(vec![NifValue::Ref(1)]));
        nif.blocks.push(emitter);

        let mut report = ConvertFileReport::default();
        strip_fo76_position_data(&mut nif, &mut report);

        let block_types: Vec<&str> = nif
            .blocks
            .iter()
            .map(|block| block.type_name.as_str())
            .collect();
        assert_eq!(
            block_types,
            vec![
                "NiNode",
                "BSTriShape",
                "BSPositionData",
                "BSPositionData",
                "NiPSysMeshEmitter"
            ]
        );
        let shape = &nif.blocks[1];
        assert_eq!(
            shape.get_field("Num Extra Data List").map(NifValue::as_i64),
            Some(2)
        );
        assert!(matches!(
            shape.get_field("Extra Data List"),
            Some(NifValue::Array(values))
                if matches!(values.as_slice(), [NifValue::Ref(2), NifValue::Ref(3)])
        ));
        assert_eq!(
            shape.get_field("Num Triangles").map(NifValue::as_i64),
            Some(99)
        );
        assert_eq!(
            shape.get_field("Num Vertices").map(NifValue::as_i64),
            Some(177)
        );
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("Preserved 2 FO76 BSPositionData"))
        );
    }

    #[test]
    fn prune_fo76_temp_ground_decal_shapes_removes_only_decal_overlay() {
        let mut nif = NifFile::new("fo76");
        let base_shader = nif.add_block(
            "BSLightingShaderProperty",
            Some(IndexMap::from([(
                "Name".to_string(),
                NifValue::String(
                    "C:\\Projects\\76\\Build\\PC\\Data\\Materials\\Landscape\\Ground\\TEMP_GroundTexture01.bgsm"
                        .to_string(),
                ),
            )])),
        );
        let decal_shader = nif.add_block(
            "BSLightingShaderProperty",
            Some(IndexMap::from([(
                "Name".to_string(),
                NifValue::String(
                    "Materials\\Landscape\\Ground\\TEMP_GroundTexture01Decal.BGSM".to_string(),
                ),
            )])),
        );
        let alpha = nif.add_block("NiAlphaProperty", None);
        let base_shape = nif.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                (
                    "Name".to_string(),
                    NifValue::String("ECliffCurved02:0".to_string()),
                ),
                (
                    "Shader Property".to_string(),
                    NifValue::Ref(base_shader as i32),
                ),
                ("Alpha Property".to_string(), NifValue::Ref(-1)),
            ])),
        );
        let decal_shape = nif.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                (
                    "Name".to_string(),
                    NifValue::String("ECliffCurved02:1".to_string()),
                ),
                (
                    "Shader Property".to_string(),
                    NifValue::Ref(decal_shader as i32),
                ),
                ("Alpha Property".to_string(), NifValue::Ref(alpha as i32)),
            ])),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(base_shape as i32),
                NifValue::Ref(decal_shape as i32),
            ]),
        );
        let mut report = ConvertFileReport::default();

        prune_fo76_temp_ground_decal_shapes(&mut nif, &mut report);

        assert!(nif.blocks.iter().any(|block| {
            block.type_name == "BSTriShape"
                && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "ECliffCurved02:0")
        }));
        assert!(!nif.blocks.iter().any(|block| {
            block.type_name == "BSTriShape"
                && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "ECliffCurved02:1")
        }));
        assert!(!nif.blocks.iter().any(|block| {
            matches!(
                block.get_field("Name"),
                Some(NifValue::String(name))
                    if name.to_ascii_lowercase().contains("temp_groundtexture01decal")
            )
        }));
        assert!(
            nif.blocks
                .iter()
                .all(|block| block.type_name != "NiAlphaProperty")
        );
        assert!(matches!(
            nif.blocks[0].get_field("Children"),
            Some(NifValue::Array(children)) if children.len() == 1
        ));
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("ECliffCurved02:1"))
        );
    }
}
