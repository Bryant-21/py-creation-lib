use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Instant;

use indexmap::IndexMap;
use thiserror::Error;

use crate::fo76_collision::{
    CollisionRoute, ExtractedCollisionBody, FO4_CLUTTER_LAYER, FO4_STATIC_LAYER,
    PlannedCollisionBody, RouteCounts, SourceBodyMetadata, SourceCollisionContext,
    classify_source_body, collision_summary_is_invalid, is_dynamic_from_nif_signals,
    motion_type_label, nif_vertices_to_havok, summary_has_degenerate_collision_shape,
};
use crate::model::{NifBlock, NifFile, NifValue};
use havok_native::collision::multi_body::{
    BodyMeta, BodyMotionType, build_fo4_multi_body_collision_with_constraints,
};
use havok_native::collision::{
    BuildOptions, CompoundChildKind, GraftCinfo, GraftedConstraints, MultiBodyShape,
    convert_fo76_embedded_static_collision_direct, with_collision_diagnostic_context,
};

const VF_VERTEX: i64 = 0x0001;
const VF_UVS: i64 = 0x0002;
const VF_NORMALS: i64 = 0x0008;
const VF_TANGENTS: i64 = 0x0010;
const VF_VERTEX_COLORS: i64 = 0x0020;
const VF_SKINNED: i64 = 0x0040;
const SLSF1_SPECULAR: u64 = 1 << 0;
const SLSF1_SKINNED: u64 = 1 << 1;
const SLSF1_VERTEX_ALPHA: u64 = 1 << 3;
const SLSF1_USE_FALLOFF: u64 = 1 << 6;
const SLSF1_ENVIRONMENT_MAPPING: u64 = 1 << 7;
const SLSF1_CAST_SHADOWS: u64 = 1 << 9;
const SLSF1_HAIR: u64 = 1 << 18;
const SLSF1_SCREENDOOR_ALPHA_FADE: u64 = 1 << 19;
const SLSF1_OWN_EMIT: u64 = 1 << 22;
const SLSF1_EXTERNAL_EMITTANCE: u64 = 1 << 29;
const SLSF1_DECAL: u64 = 1 << 26;
const SLSF1_DYNAMIC_DECAL: u64 = 1 << 27;
const SLSF1_SOFT_EFFECT: u64 = 1 << 30;
const SLSF1_ZBUFFER_TEST: u64 = 1 << 31;
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
const SLSF2_EFFECT_LIGHTING: u64 = 1 << 30;
const FO4_PARTICLE_VERTEX_DESC: u64 = 594_510_335_218_548_785;
const FO4_LEGACY_PP_SHADER_FLAGS_1_MASK: u64 = (1 << 0)
    | (1 << 1)
    | (1 << 3)
    | (1 << 7)
    | (1 << 10)
    | (1 << 13)
    | (1 << 15)
    | (1 << 16)
    | (1 << 17)
    | (1 << 18)
    | (1 << 20)
    | (1 << 24)
    | (1 << 26)
    | (1 << 27)
    | (1 << 29)
    | (1 << 31);
const FO4_LEGACY_PP_SHADER_FLAGS_2_MASK: u64 = (1 << 0)
    | (1 << 1)
    | (1 << 2)
    | (1 << 3)
    | (1 << 5)
    | (1 << 10)
    | (1 << 11)
    | (1 << 12)
    | (1 << 13)
    | (1 << 14)
    | (1 << 16)
    | (1 << 19);
const FO4_TALL_GRASS_SHADER_FLAGS_1: u64 = SLSF1_SPECULAR
    | SLSF1_VERTEX_ALPHA
    | SLSF1_CAST_SHADOWS
    | SLSF1_SCREENDOOR_ALPHA_FADE
    | SLSF1_OWN_EMIT
    | SLSF1_ZBUFFER_TEST;
const FO4_TALL_GRASS_SHADER_FLAGS_2: u64 =
    (SLSF2_ZBUFFER_WRITE | SLSF2_DOUBLE_SIDED | SLSF2_VERTEX_COLORS | SLSF2_TREE_ANIM) as u64;
const BSX_DYNAMIC_FLAG: u64 = 0x40;
const BSX_COMPLEX_FLAG: u64 = 0x08;
const BSX_ARTICULATED_FLAG: u64 = 0x80;
const BSX_ANIMATED_FLAG: u64 = 0x01;
const BSX_RAGDOLL_FLAG: u64 = 0x04;
const BSX_ADDON_FLAG: u64 = 0x10;
const BSX_EDITOR_MARKER_FLAG: u64 = 0x20;
const BSX_EXTERNAL_EMIT_FLAG: u64 = 0x200;
const FO4_TEXTURE_SLOT_COUNT: usize = 10;
const HAVOK_SCALE: f32 = 69.99125;
const LEGACY_HAVOK_SCALE: f32 = 10.0;
/// Gamebryo-era (FO3/FNV) Havok units per FO4 Havok unit. Matches pynifly's
/// `game_collision_sf["FONV"]` and is confirmed by source assets whose collision
/// hull registers exactly against the visible mesh at this factor.
const LEGACY_HAVOK_UNIT_SCALE: f32 = 1.0 / LEGACY_HAVOK_SCALE;
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
    pub skin_policy: crate::skin::LegacySkinPolicy,
    /// Target-game skeleton the translated skin must bind to (FO4's shared
    /// humanoid `skeleton.nif`). Set for `TranslateSkeleton` conversions:
    /// remapping bone names leaves the source game's bind matrices in place,
    /// which explodes the mesh, so the binds are recomputed against this rest
    /// pose. `PreserveSourceRig` (creatures) ships its own skeleton and
    /// ignores this.
    pub target_skeleton: Option<PathBuf>,
    pub auto_skin_reference_body: Option<PathBuf>,
    pub emit_first_person: bool,
    pub first_person_reference: Option<PathBuf>,
    pub morph_weight_cap: f32,
    pub weapon_role: Option<String>,
    pub strip_cloth: bool,
    /// Source-game data root (e.g. the FO76 `extracted/fo76` dir). When set, the
    /// FO76→FO4 external-material normalizers read each referenced source BGSM/BGEM to
    /// restore FO4 shader flags and texture fallbacks. None disables it.
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
            skin_policy: crate::skin::LegacySkinPolicy::default(),
            target_skeleton: None,
            auto_skin_reference_body: None,
            emit_first_person: false,
            first_person_reference: None,
            morph_weight_cap: 0.5,
            weapon_role: None,
            strip_cloth: false,
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
    pub emitted_textures: Vec<String>,
    pub emitted_first_person: Option<String>,
    pub final_dependencies: Option<FinalNifDependencies>,
    pub shapes_skinned: usize,
    pub vertices_repacked: usize,
    pub bones_remapped: usize,
    pub bones_dropped_unmapped: usize,
    pub weights_redistributed: usize,
    pub vertices_morph_weighted: usize,
    pub timings_ms: Vec<(String, u64)>,
}

#[derive(Debug, Clone)]
pub struct FinalNifDependencies {
    pub digest: [u8; 32],
    pub materials: Vec<String>,
}

impl FinalNifDependencies {
    pub fn capture(nif: &NifFile, bytes: &[u8]) -> Self {
        Self {
            digest: *blake3::hash(bytes).as_bytes(),
            materials: nif.referenced_asset_paths().materials,
        }
    }
}

#[derive(Debug)]
struct PlannedTextureEmission {
    target_path: PathBuf,
    target_relative_path: String,
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
    #[error("preserve-source-rig conversion rejected {path}: {reason}")]
    PreserveSourceRig { path: PathBuf, reason: String },
    #[error("invalid source-rig target-relative path {path:?}: {reason}")]
    InvalidSourceRigTargetPath { path: String, reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRigNifKind {
    Body,
    Skeleton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRigNifOutputProvenance {
    PreserveSourceRigConversion,
}

#[derive(Debug, Clone)]
pub struct SourceRigNifStageReceipt {
    pub kind: SourceRigNifKind,
    pub output_provenance: SourceRigNifOutputProvenance,
    pub source_path: PathBuf,
    pub staged_target_path: PathBuf,
    pub target_relative_path: String,
    pub output_len: u64,
    pub output_fingerprint: u64,
    pub report: ConvertFileReport,
}

pub fn stage_preserve_source_rig_nif(
    kind: SourceRigNifKind,
    source_path: &Path,
    staged_target_path: &Path,
    target_relative_path: &str,
    source_game: &str,
    bgsm_output_dir: Option<&Path>,
    base_options: &ConvertFileOptions,
) -> Result<SourceRigNifStageReceipt, ConvertFileError> {
    let target_relative_path = normalize_source_rig_target_path(target_relative_path)?;
    let mut options = base_options.clone();
    options.skin_policy = crate::skin::LegacySkinPolicy::PreserveSourceRig;
    let report = convert_nif_file(
        source_path,
        staged_target_path,
        source_game,
        "fo4",
        bgsm_output_dir,
        &options,
    )?;
    if !report.supported {
        return Err(ConvertFileError::PreserveSourceRig {
            path: source_path.to_path_buf(),
            reason: if report.errors.is_empty() {
                "conversion did not produce a supported FO4 NIF".to_string()
            } else {
                report.errors.join("; ")
            },
        });
    }
    let output = std::fs::read(staged_target_path)?;
    Ok(SourceRigNifStageReceipt {
        kind,
        output_provenance: SourceRigNifOutputProvenance::PreserveSourceRigConversion,
        source_path: source_path.to_path_buf(),
        staged_target_path: staged_target_path.to_path_buf(),
        target_relative_path,
        output_len: output.len() as u64,
        output_fingerprint: output.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
        }),
        report,
    })
}

fn normalize_source_rig_target_path(path: &str) -> Result<String, ConvertFileError> {
    if Path::new(path).is_absolute()
        || path.trim_start().starts_with('/')
        || path.trim_start().starts_with('\\')
    {
        return Err(ConvertFileError::InvalidSourceRigTargetPath {
            path: path.to_string(),
            reason: "path must stay relative to the staging root".to_string(),
        });
    }
    let mut parts = Vec::new();
    for part in path.trim().replace('\\', "/").split('/') {
        let part = part.trim();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return Err(ConvertFileError::InvalidSourceRigTargetPath {
                path: path.to_string(),
                reason: "path must stay relative to the staging root".to_string(),
            });
        }
        parts.push(part.to_ascii_lowercase());
    }
    if parts.is_empty() {
        return Err(ConvertFileError::InvalidSourceRigTargetPath {
            path: path.to_string(),
            reason: "path is empty".to_string(),
        });
    }
    Ok(parts.join("/"))
}

pub fn load_skyrim_material_source_nif(
    source_path: &Path,
) -> Result<NifFile, crate::io::ReadError> {
    let source = NifFile::load(source_path.to_path_buf())?;
    if crate::skyrim::validate_unskinned_geometry(&source).is_err()
        && let Some((fallback, _)) = crate::skyrim::load_static_tree_fallback(source_path, &source)?
    {
        return Ok(fallback);
    }
    Ok(source)
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
            ("fnv", "fo4")
                | ("fo3", "fo4")
                | ("fo76", "fo4")
                | ("skyrimse", "fo4")
                | ("starfield", "fo4")
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

    if source_game == "starfield" && target_game == "fo4" {
        let sf_started = Instant::now();
        let sf_opts = crate::sf_convert::SfConvertOptions {
            geometries_root: crate::sf_convert::default_geometries_root(src),
            material_path_rewriter: None,
        };
        match crate::sf_convert::convert_starfield_nif(src, dst, &sf_opts) {
            Ok(sf_report) => {
                report.supported = true;
                report.changes.push(format!(
                    "Starfield BSGeometry -> FO4 BSTriShape: {} shape(s); collision decoded={} fallback={} none={}",
                    sf_report.shapes,
                    sf_report.collision_decoded,
                    sf_report.collision_fallback,
                    sf_report.collision_none
                ));
            }
            Err(error) => report.errors.push(error),
        }
        report.record_timing_ms("sf_convert", sf_started);
        report.record_timing_ms("total", total_started);
        return Ok(report);
    }

    let load_started = Instant::now();
    let mut nif = if source_game == "fo76" && target_game == "fo4" {
        NifFile::load_for_header_retarget(src.to_path_buf(), &target_game)?
    } else {
        NifFile::load(src.to_path_buf())?
    };
    report.record_timing_ms("load", load_started);
    let required_legacy_glow_textures =
        required_legacy_glow_texture_paths(&nif, &source_game, &target_game);
    let mut planned_texture_emissions = Vec::new();
    let skyrim_to_fo4 = source_game == "skyrimse" && target_game == "fo4";
    let defer_legacy_source_rig_ragdoll = should_defer_legacy_source_rig_ragdoll(
        &nif,
        &source_game,
        &target_game,
        options.skin_policy,
    );
    if skyrim_to_fo4
        && nif
            .blocks
            .iter()
            .any(|block| block.type_name == "BSTreeNode")
    {
        if let Err(error) = crate::skyrim::validate_unskinned_geometry(&nif) {
            match crate::skyrim::load_static_tree_fallback(src, &nif)? {
                Some((fallback, fallback_path)) => {
                    nif = fallback;
                    report.changes.push(format!(
                        "Skyrim tree skinning: used static switch child {}",
                        fallback_path.display()
                    ));
                }
                None => {
                    report.errors.push(error);
                    report.record_timing_ms("total", total_started);
                    return Ok(report);
                }
            }
        }
    }
    if skyrim_to_fo4 {
        if let Err(error) = crate::skyrim::validate_supported_geometry(&nif) {
            if matches!(
                options.skin_policy,
                crate::skin::LegacySkinPolicy::PreserveSourceRig
            ) {
                return Err(ConvertFileError::PreserveSourceRig {
                    path: src.to_path_buf(),
                    reason: error,
                });
            }
            report.errors.push(error);
            report.record_timing_ms("total", total_started);
            return Ok(report);
        }
        if crate::skyrim::contains_skinned_geometry(&nif)
            && matches!(
                options.skin_policy,
                crate::skin::LegacySkinPolicy::TranslateSkeleton
            )
            && options.translation_maps_dir.is_none()
        {
            report.errors.push(
                "Skyrim skinned NIF conversion requires translation_maps_dir for skeleton remapping"
                    .to_string(),
            );
            report.record_timing_ms("total", total_started);
            return Ok(report);
        }
    }
    if source_game != target_game {
        let step_started = Instant::now();
        retarget_header(&mut nif, &target_game, &mut report);
        report.record_timing_ms("retarget_header", step_started);
        if skyrim_to_fo4 {
            let step_started = Instant::now();
            run_legacy_skin_conversion(
                &mut nif,
                src,
                &source_game,
                &target_game,
                options,
                &mut report,
            )?;
            mark_skinned_shape_shaders(&mut nif);
            report.record_timing_ms("skyrim_skin_conversion", step_started);

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
            ensure_fo4_effect_shader_defaults(&mut nif, &mut report);
            ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, true);
            report.record_timing_ms("skyrim_fo4_shader_defaults", step_started);

            let step_started = Instant::now();
            let preserve_source_rig = matches!(
                options.skin_policy,
                crate::skin::LegacySkinPolicy::PreserveSourceRig
            );
            let had_active_legacy_collision = !preserve_source_rig
                && matches!(weapon_role, Some("melee"))
                && nif.blocks.iter().any(|block| {
                    block.type_name == "bhkCollisionObject"
                        && value_u64(block.get_field("Flags"))
                            .is_some_and(|flags| flags & 0x01 != 0)
                });
            let collision_report = if preserve_source_rig {
                strip_source_rig_collision(&mut nif)
            } else {
                crate::skyrim_collision::bridge_static_collision(&mut nif)
            };
            if collision_report.converted > 0 || collision_report.stripped > 0 {
                report.changes.push(format!(
                    "Skyrim static collision: converted {} chain(s), stripped {} unsupported chain(s)",
                    collision_report.converted, collision_report.stripped
                ));
            }
            if had_active_legacy_collision && collision_report.stripped > 0 {
                report.warnings.push(
                    "Skyrim melee weapon active collision is not FO4-compatible and was stripped; the Havok BSX flag will be cleared when no converted collision remains"
                        .to_string(),
                );
            }
            report.warnings.extend(collision_report.warnings);
            report.record_timing_ms("skyrim_static_collision", step_started);
        } else if source_game == "fo76" && target_game == "fo4" {
            let step_started = Instant::now();
            repair_fo76_held_prop_transform(&mut nif, src, &mut report);
            report.record_timing_ms("fo76_held_prop_transform", step_started);
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
            ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
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
            normalize_external_bgem_shader_data_with_overrides(
                &mut nif,
                options.source_material_dir.as_deref(),
                &options.material_source_overrides,
                &mut report,
            );
            report.record_timing_ms("fo4_external_bgem_shader_data", step_started);
            let step_started = Instant::now();
            normalize_fo76_environment_mapping(&mut nif, &mut report);
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
            if options.strip_cloth {
                strip_fo76_cloth_blobs(&mut nif, &mut report);
            } else {
                convert_fo76_cloth_blobs(&mut nif, &mut report);
            }
            report.record_timing_ms("fo76_cloth_blobs", step_started);
            let step_started = Instant::now();
            normalize_fo76_headwear_segments(&mut nif, &mut report);
            report.record_timing_ms("fo76_headwear_segments", step_started);
        } else {
            let step_started = Instant::now();
            strips_to_tri_shape(&mut nif, &mut report);
            report.record_timing_ms("strips_to_tri_shape", step_started);
            let step_started = Instant::now();
            run_legacy_skin_conversion(
                &mut nif,
                src,
                &source_game,
                &target_game,
                options,
                &mut report,
            )?;
            report.record_timing_ms("legacy_skin_conversion", step_started);
            let step_started = Instant::now();
            let static_shapes = crate::skin::convert_unskinned_legacy_shapes(&mut nif);
            if static_shapes > 0 {
                report.changes.push(format!(
                    "Legacy static geometry -> FO4 geometry: converted {static_shapes} shape(s)"
                ));
            }
            report.record_timing_ms("legacy_static_geometry", step_started);
            let step_started = Instant::now();
            legacy_shader_to_lighting(&mut nif, &mut report);
            report.record_timing_ms("legacy_shader_to_lighting", step_started);
            let step_started = Instant::now();
            normalize_legacy_particle_systems(&mut nif, &mut report);
            report.record_timing_ms("legacy_particle_systems", step_started);
            if matches!(source_game.as_str(), "fnv" | "fo3") && target_game == "fo4" {
                let step_started = Instant::now();
                normalize_legacy_furniture_markers(&mut nif, &mut report);
                report.record_timing_ms("legacy_furniture_markers", step_started);
            }
            let step_started = Instant::now();
            mark_skinned_shape_shaders(&mut nif);
            report.record_timing_ms("mark_skinned_shape_shaders", step_started);
            let step_started = Instant::now();
            if defer_legacy_source_rig_ragdoll {
                let collision_report = strip_source_rig_collision(&mut nif);
                if collision_report.stripped > 0 {
                    report.changes.push(format!(
                        "Legacy source-rig collision: stripped {} deferred ragdoll chain(s)",
                        collision_report.stripped
                    ));
                }
                report.warnings.extend(collision_report.warnings);
            } else {
                regenerate_fo4_collision(&mut nif, &source_game, &mut report);
            }
            report.record_timing_ms("regenerate_fo4_collision", step_started);
        }
        if !skyrim_to_fo4 {
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
        if !skyrim_to_fo4 {
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
            if source_game == "fo76" && target_game == "fo4" {
                let step_started = Instant::now();
                normalize_fo76_inline_texture_paths(&mut nif, &mut report);
                report.record_timing_ms("fo76_inline_texture_paths", step_started);
            }
            if matches!(source_game.as_str(), "fnv" | "fo3")
                && target_game == "fo4"
                && matches!(weapon_role, Some("melee"))
            {
                let step_started = Instant::now();
                planned_texture_emissions = close_legacy_melee_texture_gaps(
                    &mut nif,
                    src,
                    dst,
                    bgsm_output_dir,
                    &required_legacy_glow_textures,
                    &mut report,
                )?;
                report.record_timing_ms("legacy_melee_texture_closure", step_started);
            }
        }
        if source_game == "fo76" && target_game == "fo4" {
            let step_started = Instant::now();
            normalize_facegen_hair_shaders(&mut nif, &mut report);
            report.record_timing_ms("facegen_hair_shaders", step_started);
        }
    }
    let step_started = Instant::now();
    let preserve_fo76_static_root_flag = source_game == "fo76"
        && target_game == "fo4"
        && (is_scol_aggregate_nif(src, &nif) || nif_is_facegen(&nif));
    normalize_fo4_root_node(
        &mut nif,
        weapon_role,
        source_game == "fo76" && target_game == "fo4",
        preserve_fo76_static_root_flag,
        &mut report,
    );
    report.record_timing_ms("normalize_fo4_root_node", step_started);
    if source_game == "fo76" && target_game == "fo4" {
        let step_started = Instant::now();
        normalize_fo76_fo4_scene_node_flags(&mut nif, &mut report);
        report.record_timing_ms("fo76_fo4_scene_node_flags", step_started);
    } else if matches!(source_game.as_str(), "fnv" | "fo3") && target_game == "fo4" {
        let step_started = Instant::now();
        normalize_legacy_fo4_av_flags(&mut nif, &mut report);
        report.record_timing_ms("legacy_fo4_av_flags", step_started);
    }
    let step_started = Instant::now();
    patch_addon_node_indices(&mut nif, &options.addon_index_map, &mut report);
    report.record_timing_ms("patch_addon_node_indices", step_started);
    if source_game == "fo76" && target_game == "fo4" {
        let step_started = Instant::now();
        normalize_fo76_animation_contract(&mut nif, &mut report);
        report.record_timing_ms("fo76_animation_contract", step_started);
    }
    if target_game == "fo4" {
        let step_started = Instant::now();
        mark_skinned_shape_shaders(&mut nif);
        report.record_timing_ms("mark_skinned_shape_shaders_final", step_started);
        if source_game == "fo76" {
            let step_started = Instant::now();
            deduplicate_fo76_exact_vertices(&mut nif, &mut report);
            report.record_timing_ms("fo76_exact_vertex_dedup", step_started);
            let step_started = Instant::now();
            normalize_fo76_vertex_color_shader_flags(&mut nif, &mut report);
            report.record_timing_ms("fo76_vertex_color_shader_flags", step_started);
        }
        let step_started = Instant::now();
        reconcile_havok_bsx_flags(&mut nif, &mut report);
        report.record_timing_ms("reconcile_havok_bsx_flags", step_started);
        if source_game == "fo76" {
            let step_started = Instant::now();
            normalize_fo76_bsx_contract(&mut nif, &mut report);
            report.record_timing_ms("fo76_bsx_contract", step_started);
        }
    }
    if matches!(source_game.as_str(), "fnv" | "fo3") && target_game == "fo4" {
        let step_started = Instant::now();
        prune_legacy_material_controller_links(&mut nif, &mut report);
        detach_legacy_decal_placement_vector_nodes(&mut nif, &mut report);
        detach_legacy_blocks_without_fo4_equivalent(&mut nif, &mut report);
        prune_unreachable_legacy_blocks(&mut nif, &mut report);
        report.record_timing_ms("prune_legacy_orphans", step_started);
    }

    if target_game == "fo4" {
        let step_started = Instant::now();
        audit_fo4_block_types(&nif, &mut report);
        report.record_timing_ms("audit_fo4_block_types", step_started);
        if !report.errors.is_empty() {
            if matches!(
                options.skin_policy,
                crate::skin::LegacySkinPolicy::PreserveSourceRig
            ) {
                return Err(ConvertFileError::PreserveSourceRig {
                    path: src.to_path_buf(),
                    reason: report.errors.join("; "),
                });
            }
            // Bail before writing: a NIF the runtime cannot load is worse than
            // no NIF, because it ships as a silent red "!" instead of a failure.
            report.record_timing_ms("total", total_started);
            return Ok(report);
        }
    }

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    emit_planned_textures(&planned_texture_emissions, &mut report)?;
    let save_started = Instant::now();
    let encode_started = Instant::now();
    let bytes = nif.to_bytes()?;
    report.record_timing_ms("save_encode", encode_started);
    let write_started = Instant::now();
    std::fs::write(dst, &bytes)?;
    report.record_timing_ms("save_write", write_started);
    if target_game == "fo4" {
        let dependencies_started = Instant::now();
        report.final_dependencies = Some(FinalNifDependencies::capture(&nif, &bytes));
        report.record_timing_ms("capture_dependencies", dependencies_started);
    }
    nif.path = Some(PathBuf::from(dst));
    report.record_timing_ms("save", save_started);
    let first_person_started = Instant::now();
    emit_first_person_sibling(&nif, dst, &target_game, options, &mut report);
    report.record_timing_ms("emit_first_person_sibling", first_person_started);
    report.supported = true;
    let cleanup_started = Instant::now();
    drop(nif);
    drop(bytes);
    report.record_timing_ms("cleanup", cleanup_started);
    report.record_timing_ms("total", total_started);
    Ok(report)
}

pub fn prepare_legacy_face_part_for_fo4(nif: &mut NifFile) -> usize {
    let converted = crate::skin::convert_unskinned_legacy_shapes(nif);
    if converted == 0 {
        return 0;
    }

    let mut report = ConvertFileReport::default();
    legacy_shader_to_lighting(nif, &mut report);
    normalize_texture_sets(nif, "fnv", "fo4", None, &HashSet::new(), &mut report);
    normalize_legacy_fo4_av_flags(nif, &mut report);
    normalize_facegen_hair_shaders(nif, &mut report);
    nif.rebuild_header();
    converted
}

pub fn finalize_assembled_facegeom_for_fo4(nif: &mut NifFile) {
    let mut report = ConvertFileReport::default();
    normalize_legacy_fo4_av_flags(nif, &mut report);
    normalize_facegen_hair_shaders(nif, &mut report);
    nif.rebuild_header();
}

fn run_legacy_skin_conversion(
    nif: &mut NifFile,
    source_path: &Path,
    source_game: &str,
    target_game: &str,
    options: &ConvertFileOptions,
    report: &mut ConvertFileReport,
) -> Result<(), ConvertFileError> {
    if target_game != "fo4" || !matches!(source_game, "fnv" | "fo3" | "skyrimse") {
        return Ok(());
    }
    if matches!(
        options.skin_policy,
        crate::skin::LegacySkinPolicy::TranslateSkeleton
    ) && options.translation_maps_dir.is_none()
    {
        return Ok(());
    }

    let skin_report = crate::skin::convert_legacy_skin_for_games_with_policy(
        nif,
        options.translation_maps_dir.as_deref(),
        source_game,
        target_game,
        options.auto_skin_reference_body.as_deref(),
        options.morph_weight_cap,
        options.skin_policy,
    )
    .map_err(|error| match error {
        crate::skin::ConvertLegacySkinError::PreserveSourceRig(reason) => {
            ConvertFileError::PreserveSourceRig {
                path: source_path.to_path_buf(),
                reason,
            }
        }
        other => ConvertFileError::LegacySkin(other),
    })?;
    if skin_report.shapes_skinned > 0 {
        let source_label = if matches!(
            options.skin_policy,
            crate::skin::LegacySkinPolicy::PreserveSourceRig
        ) {
            "Source-rig skin"
        } else if source_game == "skyrimse" {
            "Skyrim skin"
        } else {
            "Legacy skin"
        };
        report.changes.push(format!(
            "{source_label} -> FO4 skin: skinned {} shape(s), repacked {} vertex/vertices",
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

    if skin_report.shapes_skinned > 0 {
        rebind_translated_skin(nif, options, report);
    }
    Ok(())
}

/// Recompute translated bind matrices against the target skeleton's rest pose.
///
/// Only for `TranslateSkeleton`: the bones were just renamed to target-game
/// names, but their binds still describe the source rest pose. Creatures
/// (`PreserveSourceRig`) keep their own rig and are reposed from the other
/// side by `skeleton_repose`, so they must not be touched here.
fn rebind_translated_skin(
    nif: &mut NifFile,
    options: &ConvertFileOptions,
    report: &mut ConvertFileReport,
) {
    if !matches!(
        options.skin_policy,
        crate::skin::LegacySkinPolicy::TranslateSkeleton
    ) {
        return;
    }
    let Some(skeleton_path) = options.target_skeleton.as_deref() else {
        report.warnings.push(
            "Translated skin kept its source-game bind matrices: no target_skeleton supplied"
                .to_string(),
        );
        return;
    };
    let skeleton = match NifFile::load(skeleton_path) {
        Ok(skeleton) => skeleton,
        Err(error) => {
            report.warnings.push(format!(
                "Translated skin kept its source-game bind matrices: load {} failed: {error}",
                skeleton_path.display()
            ));
            return;
        }
    };

    let rebind = crate::skeleton_repose::rebind_skin_to_skeleton(nif, &skeleton);
    if rebind.rebound > 0 {
        report.changes.push(format!(
            "Skin rebind -> target skeleton rest pose: {} bind matrix/matrices",
            rebind.rebound
        ));
    }
    if !rebind.unmatched.is_empty() {
        report.warnings.push(format!(
            "Skin rebind: {} bone(s) absent from {} kept their source bind: {}",
            rebind.unmatched.len(),
            skeleton_path.display(),
            rebind.unmatched.join(", ")
        ));
    }
}

fn strip_source_rig_collision(nif: &mut NifFile) -> crate::skyrim_collision::SkyrimCollisionReport {
    let remove = nif
        .blocks
        .iter()
        .filter(|block| block.type_name.starts_with("bhk"))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    if remove.is_empty() {
        return crate::skyrim_collision::SkyrimCollisionReport::default();
    }
    let stripped = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "bhkCollisionObject"
                    | "bhkBlendCollisionObject"
                    | "bhkPCollisionObject"
                    | "bhkSPCollisionObject"
            )
        })
        .count();
    nif.remove_blocks(&remove);
    crate::skyrim_collision::SkyrimCollisionReport {
        converted: 0,
        stripped,
        warnings: vec![
            "Preserve-source-rig skin policy stripped legacy creature collision; ragdoll conversion is a separate gate"
                .to_string(),
        ],
    }
}

fn should_defer_legacy_source_rig_ragdoll(
    nif: &NifFile,
    source_game: &str,
    target_game: &str,
    skin_policy: crate::skin::LegacySkinPolicy,
) -> bool {
    matches!(source_game, "fnv" | "fo3")
        && target_game == "fo4"
        && matches!(
            skin_policy,
            crate::skin::LegacySkinPolicy::PreserveSourceRig
        )
        && (crate::skyrim::contains_skinned_geometry(nif)
            || nif.blocks.iter().any(|block| {
                matches!(
                    block.type_name.as_str(),
                    "bhkBlendCollisionObject" | "bhkRagdollConstraint"
                )
            }))
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
/// names can carry whitespace before the index. The `@#N` suffix is identity:
/// several distinct nodes can share one numeric add-on index, so it must survive
/// normalization and remapping.
///
/// * mapped (`index_map[old]` present) → `Value = new`, numeric name portion =
///   `AddOnNode<new>`, original suffix preserved;
/// * unmapped → `Value = old`, canonical prefix + original digits and suffix.
///   Already-clean nodes with matching values are a no-op.
fn patch_addon_node_indices(
    nif: &mut NifFile,
    index_map: &HashMap<i64, i64>,
    report: &mut ConvertFileReport,
) {
    let mut renamed_nodes = HashMap::new();
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
                let normalized = normalized_addon_node_name(&name, &new_index.to_string());
                block.set_field("Value", NifValue::Int(new_index));
                block.set_field("Name", NifValue::String(normalized.clone()));
                if normalized != name {
                    renamed_nodes.insert(name.clone(), normalized);
                }
                report.changes.push(format!(
                    "BSValueNode AddOnNode{old_index} -> AddOnNode{new_index} (Value {old_index} -> {new_index})"
                ));
            }
            None => {
                let normalized = normalized_addon_node_name(&name, digits);
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
                    report
                        .changes
                        .push(format!("BSValueNode {name} -> {normalized}"));
                    block.set_field("Name", NifValue::String(normalized.clone()));
                    renamed_nodes.insert(name, normalized);
                }
            }
        }
    }

    if renamed_nodes.is_empty() {
        return;
    }

    let mut references = 0usize;
    for block in &mut nif.blocks {
        if block.type_name == "NiControllerSequence" {
            if let Some(NifValue::String(name)) = block.get_field("Accum Root Name").cloned()
                && let Some(normalized) = renamed_nodes.get(&name)
            {
                block.set_field("Accum Root Name", NifValue::String(normalized.clone()));
                references += 1;
            }
            if let Some(NifValue::Array(mut entries)) =
                block.get_field("Controlled Blocks").cloned()
            {
                let mut changed = false;
                for entry in &mut entries {
                    let NifValue::Struct(fields) = entry else {
                        continue;
                    };
                    let Some(NifValue::String(name)) = fields.get("Node Name") else {
                        continue;
                    };
                    let Some(normalized) = renamed_nodes.get(name) else {
                        continue;
                    };
                    fields.insert(
                        "Node Name".to_string(),
                        NifValue::String(normalized.clone()),
                    );
                    changed = true;
                    references += 1;
                }
                if changed {
                    block.set_field("Controlled Blocks", NifValue::Array(entries));
                }
            }
        } else if block.type_name == "NiDefaultAVObjectPalette"
            && let Some(NifValue::Array(mut entries)) = block.get_field("Objs").cloned()
        {
            let mut changed = false;
            for entry in &mut entries {
                let NifValue::Struct(fields) = entry else {
                    continue;
                };
                let Some(NifValue::String(name)) = fields.get("Name") else {
                    continue;
                };
                let Some(normalized) = renamed_nodes.get(name) else {
                    continue;
                };
                fields.insert("Name".to_string(), NifValue::String(normalized.clone()));
                changed = true;
                references += 1;
            }
            if changed {
                block.set_field("Objs", NifValue::Array(entries));
            }
        }
    }
    if references > 0 {
        report.changes.push(format!(
            "BSValueNode: propagated add-on node renames to {references} animation name reference(s)"
        ));
    }
}

fn normalize_fo76_animation_contract(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let manager_ids = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiControllerManager")
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let reachable_before =
        (!manager_ids.is_empty()).then(|| reachable_block_ids_excluding(nif, &HashSet::new()));
    let mut names = HashMap::new();
    for block in nif
        .blocks
        .iter()
        .filter(|block| crate::schema::SCHEMA.is_subtype_of(&block.type_name, "NiAVObject"))
    {
        if let Some(name) = string_field(block, "Name").filter(|name| !name.is_empty()) {
            names.entry(name).or_insert(block.block_id);
        }
    }

    let mut target_repairs = Vec::new();
    for owner in &nif.blocks {
        if !crate::schema::SCHEMA.is_subtype_of(&owner.type_name, "NiObjectNET") {
            continue;
        }
        let mut controller = field_ref(owner, "Controller").unwrap_or(-1);
        let mut seen = HashSet::new();
        while controller >= 0 && seen.insert(controller) {
            let Some(controller_block) = nif.get_block(controller as usize) else {
                break;
            };
            if !crate::schema::SCHEMA.is_subtype_of(&controller_block.type_name, "NiTimeController")
            {
                break;
            }
            if field_ref(controller_block, "Target") != Some(owner.block_id as i32) {
                target_repairs.push((controller as usize, owner.block_id));
            }
            controller = field_ref(controller_block, "Next Controller").unwrap_or(-1);
        }
    }
    for (controller_id, target_id) in &target_repairs {
        if let Some(controller) = nif.blocks.get_mut(*controller_id) {
            controller.set_field("Target", NifValue::Ref(*target_id as i32));
        }
    }

    let mut manager_links = 0usize;
    let mut invalid_entries = 0usize;
    let mut sorted_sequences = 0usize;
    let mut accum_roots = 0usize;
    let mut palettes = 0usize;
    let mut extra_targets = 0usize;

    for manager_id in manager_ids {
        let Some(manager) = nif.get_block(manager_id) else {
            continue;
        };
        let manager_target = field_ref(manager, "Target")
            .filter(|target| *target >= 0)
            .map(|target| target as usize);
        let manager_target_name = manager_target
            .and_then(|target| nif.get_block(target))
            .and_then(|target| string_field(target, "Name"));
        let sequence_ids = ref_array(manager.get_field("Controller Sequences"));
        let palette_id = field_ref(manager, "Object Palette")
            .filter(|target| *target >= 0)
            .map(|target| target as usize);
        let multitarget_id = field_ref(manager, "Next Controller")
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
            let update_manager = sequence.get_field("Manager").is_some()
                && field_ref(sequence, "Manager") != Some(manager_id as i32);
            let update_accum_root = manager_target_name.as_ref().is_some_and(|target_name| {
                string_field(sequence, "Accum Root Name").as_deref() != Some(target_name)
            });
            let entries = value_array(sequence.get_field("Controlled Blocks"));
            let old_len = entries.len();
            let mut valid = entries
                .into_iter()
                .filter_map(|entry| {
                    let target = fo76_controlled_block_target(&entry, &names)?;
                    controlled_targets.insert(target);
                    Some((target, entry))
                })
                .collect::<Vec<_>>();
            invalid_entries += old_len - valid.len();
            let old_order = valid.iter().map(|(target, _)| *target).collect::<Vec<_>>();
            valid.sort_by_key(|(target, _)| *target);
            if old_order != valid.iter().map(|(target, _)| *target).collect::<Vec<_>>() {
                sorted_sequences += 1;
            }
            let entries = valid
                .into_iter()
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>();
            let sequence = &mut nif.blocks[sequence_id];
            if update_manager {
                sequence.set_field("Manager", NifValue::Ref(manager_id as i32));
                manager_links += 1;
            }
            if update_accum_root {
                sequence.set_field(
                    "Accum Root Name",
                    NifValue::String(manager_target_name.clone().unwrap_or_default()),
                );
                accum_roots += 1;
            }
            sequence.set_field(
                "Num Controlled Blocks",
                NifValue::UInt(entries.len() as u64),
            );
            sequence.set_field("Controlled Blocks", NifValue::Array(entries));
        }

        let target_ids = controlled_targets.into_iter().collect::<Vec<_>>();
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
        let palette_id = palette_id
            .filter(|palette_id| {
                nif.get_block(*palette_id)
                    .is_some_and(|palette| palette.type_name == "NiDefaultAVObjectPalette")
            })
            .or_else(|| {
                (!target_ids.is_empty()).then(|| {
                    let palette_id = nif.add_block("NiDefaultAVObjectPalette", None);
                    nif.blocks[manager_id]
                        .set_field("Object Palette", NifValue::Ref(palette_id as i32));
                    palette_id
                })
            });
        if let Some(palette_id) = palette_id {
            let palette_differs = nif
                .get_block(palette_id)
                .and_then(|palette| palette.get_field("Objs"))
                != Some(&NifValue::Array(objects.clone()));
            if palette_differs && let Some(palette) = nif.blocks.get_mut(palette_id) {
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
                .map(|value| ref_array(Some(value)))
                .unwrap_or_default();
            if current.iter().any(|target| *target < 0)
                && current
                    .iter()
                    .filter_map(|target| usize::try_from(*target).ok())
                    .collect::<BTreeSet<_>>()
                    == target_ids.iter().copied().collect::<BTreeSet<_>>()
            {
                continue;
            }
            let targets = target_ids
                .iter()
                .map(|target| NifValue::Ref(*target as i32))
                .collect::<Vec<_>>();
            let differs = nif
                .get_block(multitarget_id)
                .and_then(|block| block.get_field("Extra Targets"))
                != Some(&NifValue::Array(targets.clone()));
            if differs && let Some(multitarget) = nif.blocks.get_mut(multitarget_id) {
                multitarget.set_field("Num Extra Targets", NifValue::UInt(targets.len() as u64));
                multitarget.set_field("Extra Targets", NifValue::Array(targets));
                extra_targets += 1;
            }
        }
    }

    if invalid_entries > 0 {
        let reachable_after = reachable_block_ids_excluding(nif, &HashSet::new());
        let newly_unreachable = reachable_before
            .expect("invalid controller entries require a controller manager")
            .difference(&reachable_after)
            .copied()
            .collect::<HashSet<_>>();
        remove_blocks(nif, newly_unreachable);
    }

    let total = target_repairs.len()
        + manager_links
        + invalid_entries
        + sorted_sequences
        + accum_roots
        + palettes
        + extra_targets;
    if total > 0 {
        report.changes.push(format!(
            "Animation contract: targets={} manager-links={manager_links} invalid-blocks={invalid_entries} sorted={sorted_sequences} accumulation-roots={accum_roots} palettes={palettes} extra-targets={extra_targets}",
            target_repairs.len()
        ));
    }
}

fn fo76_controlled_block_target(entry: &NifValue, names: &HashMap<String, usize>) -> Option<usize> {
    let NifValue::Struct(fields) = entry else {
        return None;
    };
    fields
        .get("Node Name")
        .and_then(nif_value_string)
        .filter(|name| !name.is_empty())
        .and_then(|name| names.get(name).copied())
}

/// Parse a `BSValueNode` name of the form `AddOnNode[whitespace]<digits>[@…]`.
///
/// Returns `(index, digit_substring)` where the digit substring preserves the
/// original zero-padding and excludes any FO76-only `@…` suffix (e.g.
/// `"AddOnNode078@#0"` → `(78, "078")`). Returns `None` when the name isn't an
/// addon node or carries no leading digits.
fn addon_node_index(name: &str) -> Option<(i64, &str)> {
    crate::validation::parse_addon_node_index(name)
}

fn normalized_addon_node_name(name: &str, digits: &str) -> String {
    let rest = name
        .get("addonnode".len()..)
        .unwrap_or_default()
        .trim_start();
    let digit_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    format!("AddOnNode{digits}{}", &rest[digit_len..])
}

fn copy_file(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)?;
    Ok(())
}

fn has_nif_header(path: &Path) -> Result<bool, std::io::Error> {
    use std::io::Read;

    const PREFIX_BYTES: u64 = b"NetImmerse File Format".len() as u64;
    let mut bytes = Vec::with_capacity(PREFIX_BYTES as usize);
    std::fs::File::open(path)?
        .take(PREFIX_BYTES)
        .read_to_end(&mut bytes)?;
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
    remove_blocks(nif, remove_ids);
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
                | "BSShaderNoLightingProperty"
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
        replace_block_ref(nif, strip_id as i32, new_shape_id as i32);
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
    let mut converted_unlit = 0usize;

    let shader_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "TallGrassShaderProperty"
                    | "BSShaderPPLightingProperty"
                    | "BSShaderNoLightingProperty"
            )
        })
        .map(|block| block.block_id)
        .collect();

    for shader_id in shader_ids {
        let Some(shader) = nif.get_block(shader_id).cloned() else {
            continue;
        };
        let material = direct_materials
            .get(&shader.block_id)
            .and_then(|material_id| nif.get_block(*material_id))
            .cloned();
        let new_id = match shader.type_name.as_str() {
            "TallGrassShaderProperty" => {
                converted_grass += 1;
                convert_tall_grass(nif, &shader)
            }
            "BSShaderPPLightingProperty" => {
                converted_pp += 1;
                let lighting_id = convert_pp_lighting(nif, &shader);
                if let (Some(material), Some(lighting)) =
                    (material.as_ref(), nif.blocks.get_mut(lighting_id))
                {
                    apply_material(lighting, material);
                }
                lighting_id
            }
            "BSShaderNoLightingProperty" => {
                converted_unlit += 1;
                convert_no_lighting_effect(
                    nif,
                    &shader,
                    material.as_ref(),
                    legacy_shader_uses_vertex_colors(nif, shader.block_id),
                )
            }
            _ => continue,
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

    let particle_bindings = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "NiParticleSystem" | "NiMeshParticleSystem"
            )
        })
        .map(|block| {
            let properties = ref_array(block.get_field("Properties"));
            let shader = properties
                .iter()
                .find_map(|property| remap.get(&(*property as usize)).copied())
                .map(|id| id as i32)
                .unwrap_or(-1);
            let alpha = properties
                .iter()
                .find(|property| {
                    nif.get_block(**property as usize)
                        .is_some_and(|block| block.type_name == "NiAlphaProperty")
                })
                .copied()
                .unwrap_or(-1);
            (block.block_id, shader, alpha)
        })
        .collect::<Vec<_>>();
    for (block_id, shader, alpha) in particle_bindings {
        if let Some(block) = nif.blocks.get_mut(block_id) {
            block.set_field("Shader Property", NifValue::Ref(shader));
            block.set_field("Alpha Property", NifValue::Ref(alpha));
        }
    }

    for block in nif.blocks.iter() {
        if block.type_name == "NiMaterialProperty" {
            remove.insert(block.block_id);
        }
    }
    remove_blocks(nif, remove);

    if converted_grass > 0 || converted_pp > 0 || converted_unlit > 0 {
        report.changes.push(format!(
            "Legacy shader properties -> FO4 shaders: {converted_grass} grass + {converted_pp} pp-lighting + {converted_unlit} unlit-effect"
        ));
    }
}

fn legacy_shader_uses_vertex_colors(nif: &NifFile, shader_id: usize) -> bool {
    nif.blocks
        .iter()
        .any(|block| match block.type_name.as_str() {
            "BSTriShape" | "BSSubIndexTriShape" => {
                field_ref(block, "Shader Property") == Some(shader_id as i32)
                    && value_u64(block.get_field("Vertex Desc"))
                        .is_some_and(|desc| desc & ((VF_VERTEX_COLORS as u64) << 44) != 0)
            }
            "NiParticleSystem" | "NiMeshParticleSystem" => {
                if !ref_array(block.get_field("Properties")).contains(&(shader_id as i32)) {
                    return false;
                }
                field_ref(block, "Data")
                    .filter(|id| *id >= 0)
                    .and_then(|id| nif.get_block(id as usize))
                    .and_then(|data| data.get_field("Has Vertex Colors"))
                    .is_some_and(|value| match value {
                        NifValue::Bool(value) => *value,
                        value => value.as_i64() != 0,
                    })
            }
            _ => false,
        })
}

fn convert_no_lighting_effect(
    nif: &mut NifFile,
    block: &NifBlock,
    material: Option<&NifBlock>,
    uses_vertex_colors: bool,
) -> usize {
    let source_flags_1 = value_u64(block.get_field("Shader Flags"))
        .unwrap_or_else(|| flag_names_to_bits(block.get_field("Shader Flags"), true));
    let source_flags_2 = value_u64(block.get_field("Shader Flags 2"))
        .unwrap_or_else(|| flag_names_to_bits(block.get_field("Shader Flags 2"), false));
    let mut flags_1 = source_flags_1
        & (SLSF1_SPECULAR
            | SLSF1_SKINNED
            | SLSF1_VERTEX_ALPHA
            | SLSF1_ENVIRONMENT_MAPPING
            | (1 << 15)
            | SLSF1_HAIR
            | SLSF1_DECAL
            | SLSF1_DYNAMIC_DECAL
            | (1 << 29)
            | SLSF1_ZBUFFER_TEST);
    if source_flags_1 & (1 << 8) != 0 || legacy_flag_name_present(block, "Alpha_Texture") {
        flags_1 |= SLSF1_VERTEX_ALPHA;
    }
    let mut flags_2 =
        source_flags_2 & (SLSF2_ZBUFFER_WRITE as u64 | (1 << 3) | SLSF2_VERTEX_COLORS as u64);
    if uses_vertex_colors {
        flags_2 |= SLSF2_VERTEX_COLORS as u64;
    }

    let alpha = material
        .and_then(|material| value_f64(material.get_field("Alpha")))
        .unwrap_or(1.0) as f32;
    let mut fields = IndexMap::new();
    fields.insert(
        "Name".to_string(),
        NifValue::String(string_field(block, "Name").unwrap_or_default()),
    );
    fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
    fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
    fields.insert(
        "Controller".to_string(),
        block
            .get_field("Controller")
            .cloned()
            .unwrap_or(NifValue::Ref(-1)),
    );
    fields.insert("Shader Flags 1".to_string(), NifValue::UInt(flags_1));
    fields.insert("Shader Flags 1:FO4".to_string(), NifValue::UInt(flags_1));
    fields.insert("Shader Flags 2".to_string(), NifValue::UInt(flags_2));
    fields.insert("Shader Flags 2:FO4".to_string(), NifValue::UInt(flags_2));
    fields.insert("UV Offset".to_string(), tex_coord([0.0, 0.0]));
    fields.insert("UV Scale".to_string(), tex_coord([1.0, 1.0]));
    fields.insert(
        "Source Texture".to_string(),
        NifValue::String(string_field(block, "File Name").unwrap_or_default()),
    );
    fields.insert(
        "Texture Clamp Mode".to_string(),
        NifValue::UInt(value_u64(block.get_field("Texture Clamp Mode")).unwrap_or(3)),
    );
    fields.insert("Lighting Influence".to_string(), NifValue::UInt(255));
    fields.insert("Env Map Min LOD".to_string(), NifValue::UInt(0));
    fields.insert("Unused Byte".to_string(), NifValue::UInt(0));
    fields.insert(
        "Falloff Start Angle".to_string(),
        NifValue::Float(value_f64(block.get_field("Falloff Start Angle")).unwrap_or(1.0)),
    );
    fields.insert(
        "Falloff Stop Angle".to_string(),
        NifValue::Float(value_f64(block.get_field("Falloff Stop Angle")).unwrap_or(1.0)),
    );
    fields.insert(
        "Falloff Start Opacity".to_string(),
        NifValue::Float(value_f64(block.get_field("Falloff Start Opacity")).unwrap_or(1.0)),
    );
    fields.insert(
        "Falloff Stop Opacity".to_string(),
        NifValue::Float(value_f64(block.get_field("Falloff Stop Opacity")).unwrap_or(0.0)),
    );
    fields.insert(
        "Base Color".to_string(),
        NifValue::Color4([1.0, 1.0, 1.0, alpha]),
    );
    fields.insert("Base Color Scale".to_string(), NifValue::Float(1.0));
    fields.insert("Soft Falloff Depth".to_string(), NifValue::Float(100.0));
    fields.insert(
        "Greyscale Texture".to_string(),
        NifValue::String(String::new()),
    );
    fields.insert(
        "Env Map Texture".to_string(),
        NifValue::String(String::new()),
    );
    fields.insert(
        "Normal Texture".to_string(),
        NifValue::String(String::new()),
    );
    fields.insert(
        "Env Mask Texture".to_string(),
        NifValue::String(String::new()),
    );
    fields.insert("Environment Map Scale".to_string(), NifValue::Float(1.0));
    nif.add_block("BSEffectShaderProperty", Some(fields))
}

fn legacy_flag_name_present(block: &NifBlock, expected: &str) -> bool {
    let expected = expected.replace('_', "").to_ascii_lowercase();
    value_array(block.get_field("Shader Flags"))
        .iter()
        .filter_map(|value| match value {
            NifValue::String(value) => Some(value),
            _ => None,
        })
        .any(|value| value.replace('_', "").to_ascii_lowercase() == expected)
}

fn normalize_legacy_furniture_markers(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut converted = 0usize;
    for block in &mut nif.blocks {
        if block.type_name != "BSFurnitureMarker" {
            continue;
        }

        let positions = value_array(block.get_field("Positions"))
            .into_iter()
            .map(|position| match position {
                NifValue::Struct(fields) => {
                    let orientation = value_u64(fields.get("Orientation")).unwrap_or_default();
                    NifValue::Struct(IndexMap::from([
                        (
                            "Offset".to_string(),
                            fields
                                .get("Offset")
                                .cloned()
                                .unwrap_or(NifValue::Vec3([0.0, 0.0, 0.0])),
                        ),
                        (
                            "Heading".to_string(),
                            NifValue::Float(orientation as f64 / 1000.0),
                        ),
                        ("Animation Type".to_string(), NifValue::UInt(0)),
                        ("Entry Properties".to_string(), NifValue::UInt(0)),
                    ]))
                }
                value => value,
            })
            .collect::<Vec<_>>();

        block.type_name = "BSFurnitureMarkerNode".to_string();
        block.set_field("Num Positions", NifValue::UInt(positions.len() as u64));
        block.set_field("Positions", NifValue::Array(positions));
        block.remainder.clear();
        converted += 1;
    }

    if converted > 0 {
        report.changes.push(format!(
            "Legacy furniture markers: converted {converted} BSFurnitureMarker block(s) to FO4 layout"
        ));
    }
}

fn normalize_legacy_particle_systems(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let particle_ids = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "NiParticleSystem" | "NiMeshParticleSystem"
            )
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();

    for particle_id in &particle_ids {
        let Some(source) = nif.get_block(*particle_id).cloned() else {
            continue;
        };
        let data_ref = field_ref(&source, "Data").unwrap_or(-1);
        let bounding_sphere = (data_ref >= 0)
            .then(|| nif.get_block(data_ref as usize))
            .flatten()
            .map(|data| cloned_field(data, "Bounding Sphere"))
            .unwrap_or_else(zero_bounding_sphere);
        let modifiers = ref_array(source.get_field("Modifiers"));
        let extra_data = ref_array(source.get_field("Extra Data List"));
        let mut fields = IndexMap::new();
        fields.insert(
            "Name".to_string(),
            NifValue::String(string_field(&source, "Name").unwrap_or_default()),
        );
        fields.insert(
            "Num Extra Data List".to_string(),
            NifValue::UInt(extra_data.len() as u64),
        );
        fields.insert(
            "Extra Data List".to_string(),
            NifValue::Array(extra_data.into_iter().map(NifValue::Ref).collect()),
        );
        fields.insert(
            "Controller".to_string(),
            NifValue::Ref(field_ref(&source, "Controller").unwrap_or(-1)),
        );
        fields.insert("Flags".to_string(), NifValue::UInt(14));
        fields.insert(
            "Translation".to_string(),
            source
                .get_field("Translation")
                .cloned()
                .unwrap_or(NifValue::Vec3([0.0, 0.0, 0.0])),
        );
        fields.insert(
            "Rotation".to_string(),
            source.get_field("Rotation").cloned().unwrap_or_else(|| {
                NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])
            }),
        );
        fields.insert(
            "Scale".to_string(),
            NifValue::Float(value_f64(source.get_field("Scale")).unwrap_or(1.0)),
        );
        fields.insert(
            "Collision Object".to_string(),
            NifValue::Ref(field_ref(&source, "Collision Object").unwrap_or(-1)),
        );
        fields.insert("Bounding Sphere".to_string(), bounding_sphere);
        fields.insert("Skin".to_string(), NifValue::Ref(-1));
        fields.insert(
            "Shader Property".to_string(),
            NifValue::Ref(field_ref(&source, "Shader Property").unwrap_or(-1)),
        );
        fields.insert(
            "Alpha Property".to_string(),
            NifValue::Ref(field_ref(&source, "Alpha Property").unwrap_or(-1)),
        );
        fields.insert(
            "Vertex Desc".to_string(),
            NifValue::UInt(FO4_PARTICLE_VERTEX_DESC),
        );
        fields.insert("Far Begin".to_string(), NifValue::UInt(0));
        fields.insert("Far End".to_string(), NifValue::UInt(0));
        fields.insert("Near Begin".to_string(), NifValue::UInt(0));
        fields.insert("Near End".to_string(), NifValue::UInt(0));
        fields.insert("Data".to_string(), NifValue::Ref(data_ref));
        fields.insert(
            "World Space".to_string(),
            NifValue::UInt(value_u64(source.get_field("World Space")).unwrap_or(1)),
        );
        fields.insert(
            "Num Modifiers".to_string(),
            NifValue::UInt(modifiers.len() as u64),
        );
        fields.insert(
            "Modifiers".to_string(),
            NifValue::Array(modifiers.into_iter().map(NifValue::Ref).collect()),
        );
        if let Some(block) = nif.blocks.get_mut(*particle_id) {
            block.fields = fields;
            block.remainder.clear();
        }
    }

    let mut data_count = 0usize;
    for block in nif
        .blocks
        .iter_mut()
        .filter(|block| matches!(block.type_name.as_str(), "NiPSysData" | "NiMeshPSysData"))
    {
        let subtexture_count = value_u64(block.get_field("Num Subtexture Offsets"))
            .unwrap_or_else(|| value_array(block.get_field("Subtexture Offsets")).len() as u64);
        set_missing(block, "Material CRC", NifValue::UInt(0));
        set_missing(
            block,
            "Has Texture Indices",
            NifValue::Bool(subtexture_count > 0),
        );
        set_missing(block, "Aspect Ratio", NifValue::Float(1.0));
        set_missing(block, "Aspect Flags", NifValue::UInt(0));
        set_missing(block, "Speed to Aspect Aspect 2", NifValue::Float(0.0));
        set_missing(block, "Speed to Aspect Speed 1", NifValue::Float(0.0));
        set_missing(block, "Speed to Aspect Speed 2", NifValue::Float(0.0));
        block.remainder.clear();
        data_count += 1;
    }

    if !particle_ids.is_empty() || data_count > 0 {
        report.changes.push(format!(
            "Legacy particles -> FO4 layout: {} system(s) + {data_count} data block(s)",
            particle_ids.len()
        ));
    }
}

fn zero_bounding_sphere() -> NifValue {
    NifValue::Struct(IndexMap::from([
        ("Center".to_string(), NifValue::Vec3([0.0, 0.0, 0.0])),
        ("Radius".to_string(), NifValue::Float(0.0)),
    ]))
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
        NifValue::UInt(FO4_TALL_GRASS_SHADER_FLAGS_1),
    );
    fields.insert(
        "Shader Flags 1:FO4".to_string(),
        NifValue::UInt(FO4_TALL_GRASS_SHADER_FLAGS_1),
    );
    fields.insert(
        "Shader Flags 2".to_string(),
        NifValue::UInt(FO4_TALL_GRASS_SHADER_FLAGS_2),
    );
    fields.insert(
        "Shader Flags 2:FO4".to_string(),
        NifValue::UInt(FO4_TALL_GRASS_SHADER_FLAGS_2),
    );
    fields.insert("UV Offset".to_string(), tex_coord([0.0, 0.0]));
    fields.insert("UV Scale".to_string(), tex_coord([1.0, 1.0]));
    fields.insert("Emissive Color".to_string(), NifValue::Color3([0.0; 3]));
    fields.insert("Emissive Multiple".to_string(), NifValue::Float(1.0));
    fields.insert("Root Material".to_string(), NifValue::String(String::new()));
    fields.insert("Texture Clamp Mode".to_string(), NifValue::UInt(3));
    fields.insert("Alpha".to_string(), NifValue::Float(1.0));
    fields.insert("Refraction Strength".to_string(), NifValue::Float(0.0));
    fields.insert("Smoothness".to_string(), NifValue::Float(0.282));
    fields.insert("Specular Color".to_string(), NifValue::Color3([1.0; 3]));
    fields.insert("Specular Strength".to_string(), NifValue::Float(1.0));
    fields.insert("Subsurface Rolloff".to_string(), NifValue::Float(10.0));
    fields.insert(
        "Rimlight Power".to_string(),
        NifValue::Float(f32::MAX as f64),
    );
    fields.insert("Backlight Power".to_string(), NifValue::Float(0.0));
    fields.insert(
        "Grayscale to Palette Scale".to_string(),
        NifValue::Float(1.0),
    );
    fields.insert("Fresnel Power".to_string(), NifValue::Float(5.0));
    fields.insert("Wetness".to_string(), default_fo4_wetness());
    nif.add_block("BSLightingShaderProperty", Some(fields))
}

fn convert_pp_lighting(nif: &mut NifFile, block: &NifBlock) -> usize {
    let texset_ref = resolve_texset(nif, block);
    let flags_1 = flag_names_to_bits(block.get_field("Shader Flags"), true)
        & FO4_LEGACY_PP_SHADER_FLAGS_1_MASK;
    let flags_2 = flag_names_to_bits(block.get_field("Shader Flags 2"), false)
        & FO4_LEGACY_PP_SHADER_FLAGS_2_MASK;
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
    fields.insert("Shader Flags 1".to_string(), NifValue::UInt(flags_1));
    fields.insert("Shader Flags 1:FO4".to_string(), NifValue::UInt(flags_1));
    fields.insert("Shader Flags 2".to_string(), NifValue::UInt(flags_2));
    fields.insert("Shader Flags 2:FO4".to_string(), NifValue::UInt(flags_2));
    fields.insert("UV Offset".to_string(), tex_coord([0.0, 0.0]));
    fields.insert("UV Scale".to_string(), tex_coord([1.0, 1.0]));
    fields.insert("Emissive Color".to_string(), NifValue::Color3([0.0; 3]));
    fields.insert("Emissive Multiple".to_string(), NifValue::Float(1.0));
    fields.insert("Root Material".to_string(), NifValue::String(String::new()));
    fields.insert(
        "Texture Clamp Mode".to_string(),
        block
            .get_field("Texture Clamp Mode")
            .cloned()
            .unwrap_or(NifValue::UInt(0)),
    );
    fields.insert("Alpha".to_string(), NifValue::Float(1.0));
    fields.insert(
        "Refraction Strength".to_string(),
        NifValue::Float(value_f64(block.get_field("Refraction Strength")).unwrap_or(0.0)),
    );
    fields.insert("Smoothness".to_string(), NifValue::Float(1.0));
    fields.insert("Specular Color".to_string(), NifValue::Color3([1.0; 3]));
    fields.insert("Specular Strength".to_string(), NifValue::Float(1.0));
    fields.insert("Subsurface Rolloff".to_string(), NifValue::Float(0.0));
    fields.insert(
        "Rimlight Power".to_string(),
        NifValue::Float(f32::MAX as f64),
    );
    fields.insert("Backlight Power".to_string(), NifValue::Float(0.0));
    fields.insert(
        "Grayscale to Palette Scale".to_string(),
        NifValue::Float(1.0),
    );
    fields.insert("Fresnel Power".to_string(), NifValue::Float(5.0));
    fields.insert("Wetness".to_string(), default_fo4_wetness());
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
    let mut last_shader_id = None;
    let mut blocks = nif.blocks.clone();
    blocks.sort_by_key(|block| block.block_id);
    for block in blocks {
        if matches!(
            block.type_name.as_str(),
            "BSShaderPPLightingProperty" | "BSShaderNoLightingProperty"
        ) {
            last_shader_id = Some(block.block_id);
            continue;
        }
        if block.type_name == "NiMaterialProperty" {
            if let Some(shader_id) = last_shader_id.take() {
                pairs.insert(shader_id, block.block_id);
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

fn repair_fo76_held_prop_transform(nif: &mut NifFile, src: &Path, report: &mut ConvertFileReport) {
    let filename = src
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let (attachment, replace_motion) = match filename.as_str() {
        "animobjectacousticguitar.nif" => ("AnimObjectL1", false),
        "animobject_atx_alienwhackamole_mallet.nif" => ("AnimObjectR1", true),
        _ => return,
    };
    let Some(root) = nif.get_block(0) else { return };
    if !ref_array(root.get_field("Extra Data List"))
        .iter()
        .any(|id| {
            usize::try_from(*id)
                .ok()
                .and_then(|id| nif.get_block(id))
                .is_some_and(|block| {
                    block.type_name == "NiStringExtraData"
                        && string_field(block, "Name")
                            .is_some_and(|name| name.eq_ignore_ascii_case("Prn"))
                        && string_field(block, "String Data").as_deref() == Some(attachment)
                })
        })
    {
        return;
    }
    let Some(controller_id) = field_ref(root, "Controller").filter(|id| *id >= 0) else {
        return;
    };
    let Some(controller) = nif.get_block(controller_id as usize) else {
        return;
    };
    if controller.type_name != "NiTransformController" || field_ref(controller, "Target") != Some(0)
    {
        return;
    }
    let Some(interpolator_id) = field_ref(controller, "Interpolator").filter(|id| *id >= 0) else {
        return;
    };
    let Some(interpolator) = nif.get_block(interpolator_id as usize) else {
        return;
    };
    if interpolator.type_name != "NiTransformInterpolator"
        || (!replace_motion && field_ref(interpolator, "Data") != Some(-1))
    {
        return;
    }
    let Some(NifValue::Struct(mut transform)) = interpolator.get_field("Transform").cloned() else {
        return;
    };
    // The actor's attachment bone already supplies placement. These exports add
    // a stale guitar offset or a second, furniture-space mallet motion on top.
    transform.insert("Translation".into(), NifValue::Vec3([0.0; 3]));
    if replace_motion {
        transform.insert(
            "Rotation".into(),
            NifValue::Quaternion([1.0, 0.0, 0.0, 0.0]),
        );
        transform.insert("Scale".into(), NifValue::Float(1.0));
    }
    let transform = NifValue::Struct(transform);
    if interpolator.get_field("Transform") == Some(&transform)
        && field_ref(interpolator, "Data") == Some(-1)
    {
        return;
    }
    let reachable_before = reachable_block_ids_excluding(nif, &HashSet::new());
    nif.blocks[interpolator_id as usize].set_field("Transform", transform);
    if replace_motion {
        nif.blocks[interpolator_id as usize].set_field("Data", NifValue::Ref(-1));
        let reachable_after = reachable_block_ids_excluding(nif, &HashSet::new());
        let orphaned = reachable_before
            .difference(&reachable_after)
            .copied()
            .collect::<Vec<_>>();
        nif.remove_blocks(&orphaned);
    }
    report.changes.push(format!(
        "Pinned FO76 held-prop root transform to {attachment}: {filename}"
    ));
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
        let reachable_before = reachable_block_ids_excluding(nif, &HashSet::new());
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
        let reachable_after = reachable_block_ids_excluding(nif, &dropped_ids);
        let mut newly_unreachable = reachable_before
            .difference(&reachable_after)
            .copied()
            .collect::<HashSet<_>>();
        newly_unreachable.extend(dropped_ids.iter().copied());
        remove_blocks(nif, newly_unreachable);
    }

    if effect_remapped > 0 || lighting_remapped > 0 || !dropped_ids.is_empty() {
        report.changes.push(format!(
            "Float controllers: remapped {effect_remapped} effect + {lighting_remapped} lighting; removed {} orphan lighting controller(s) with unmapped FO76 Controlled Variable (rewired {rewired} chain link(s))",
            dropped_ids.len()
        ));
    }
}

fn reachable_block_ids_excluding(nif: &NifFile, excluded: &HashSet<usize>) -> HashSet<usize> {
    let mut reachable = HashSet::new();
    let mut pending = nif
        .header
        .footer_roots
        .iter()
        .copied()
        .filter(|root| *root >= 0)
        .map(|root| root as usize)
        .filter(|root| !excluded.contains(root))
        .collect::<Vec<_>>();
    while let Some(block_id) = pending.pop() {
        if block_id >= nif.blocks.len()
            || excluded.contains(&block_id)
            || !reachable.insert(block_id)
        {
            continue;
        }
        pending.extend(
            conversion_block_refs(&nif.blocks[block_id])
                .into_iter()
                .filter(|reference| *reference >= 0)
                .map(|reference| reference as usize)
                .filter(|reference| !excluded.contains(reference)),
        );
    }
    reachable
}

fn conversion_block_refs(block: &NifBlock) -> Vec<i32> {
    let mut refs = block.get_refs(&crate::schema::SCHEMA);
    if block.type_name == "BSProceduralLightningController" {
        refs.extend(
            (1..=9)
                .filter_map(|index| field_ref(block, &format!("Interpolator {index}")))
                .filter(|reference| *reference >= 0),
        );
    }
    refs
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

fn ensure_fo4_lighting_shader_defaults(
    nif: &mut NifFile,
    report: &mut ConvertFileReport,
    complete_existing: bool,
) {
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
        if !complete_existing
            && (block.get_field("Shader Flags 1").is_some()
                || block.get_field("Shader Flags 1:FO4").is_some())
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
    match block.get_field("Vertex Data") {
        Some(NifValue::Array(vertices)) => vertices.iter().any(
            |value| matches!(value, NifValue::Struct(fields) if fields.contains_key("Vertex Colors")),
        ),
        _ => false,
    }
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

fn normalize_external_bgem_shader_data_with_overrides(
    nif: &mut NifFile,
    source_material_dir: Option<&Path>,
    material_source_overrides: &HashMap<String, String>,
    report: &mut ConvertFileReport,
) {
    let shaders: Vec<(usize, String)> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSEffectShaderProperty")
        .filter_map(|block| {
            let material_path = string_field(block, "Name")?;
            material_path
                .to_ascii_lowercase()
                .ends_with(".bgem")
                .then_some((block.block_id, material_path))
        })
        .collect();

    let mut normalized = 0usize;
    for (shader_id, material_path) in shaders {
        let (source_material_path, _) =
            material_source_override_path(&material_path, material_source_overrides);
        let Some(material) = converted_source_bgem(&source_material_path, source_material_dir)
        else {
            continue;
        };
        let Some(shader) = nif.blocks.get_mut(shader_id) else {
            continue;
        };

        for field in ["Shader Flags 1", "Shader Flags 1:FO4"] {
            sync_shader_flag(shader, field, SLSF1_USE_FALLOFF, material.FalloffEnabled);
            sync_shader_flag(
                shader,
                field,
                SLSF1_ENVIRONMENT_MAPPING,
                material.EnvironmentMapping == Some(true)
                    || material.header.env_mapping == Some(true)
                    || nonempty_material_texture(&material.EnvmapTexture),
            );
            sync_shader_flag(shader, field, SLSF1_SOFT_EFFECT, material.SoftEnabled);
            sync_shader_flag(
                shader,
                field,
                SLSF1_ZBUFFER_TEST,
                material.header.zbuffer_test,
            );
            sync_shader_flag(
                shader,
                field,
                SLSF1_DECAL,
                material.header.decal || material.header.decal_nofade,
            );
            sync_shader_flag(
                shader,
                field,
                SLSF1_DYNAMIC_DECAL,
                material.header.decal_nofade,
            );
        }
        for field in ["Shader Flags 2", "Shader Flags 2:FO4"] {
            sync_shader_flag(
                shader,
                field,
                SLSF2_ZBUFFER_WRITE as u64,
                material.header.zbuffer_write,
            );
            sync_shader_flag(
                shader,
                field,
                SLSF2_DOUBLE_SIDED as u64,
                material.header.two_sided,
            );
            sync_shader_flag(
                shader,
                field,
                SLSF2_EFFECT_LIGHTING,
                material.EffectLightingEnabled,
            );
        }

        shader.set_field(
            "UV Offset",
            tex_coord([material.header.u_offset, material.header.v_offset]),
        );
        shader.set_field(
            "UV Scale",
            tex_coord([material.header.u_scale, material.header.v_scale]),
        );
        shader.set_field(
            "Source Texture",
            NifValue::String(canonical_bgem_texture_path(&material.BaseTexture)),
        );
        shader.set_field(
            "Texture Clamp Mode",
            NifValue::UInt(texture_clamp_mode(
                material.header.tile_u,
                material.header.tile_v,
            )),
        );
        shader.set_field(
            "Lighting Influence",
            NifValue::UInt(
                (material.LightingInfluence.clamp(0.0, 1.0) * u8::MAX as f32).round() as u64,
            ),
        );
        shader.set_field(
            "Env Map Min LOD",
            NifValue::UInt(material.EnvmapMinLOD as u64),
        );
        shader.set_field(
            "Falloff Start Angle",
            NifValue::Float(material.FalloffStartAngle as f64),
        );
        shader.set_field(
            "Falloff Stop Angle",
            NifValue::Float(material.FalloffStopAngle as f64),
        );
        shader.set_field(
            "Falloff Start Opacity",
            NifValue::Float(material.FalloffStartOpacity as f64),
        );
        shader.set_field(
            "Falloff Stop Opacity",
            NifValue::Float(material.FalloffStopOpacity as f64),
        );
        shader.set_field(
            "Base Color",
            NifValue::Color4([
                material.BaseColor[0],
                material.BaseColor[1],
                material.BaseColor[2],
                material.header.alpha.clamp(0.0, 1.0),
            ]),
        );
        shader.set_field(
            "Base Color Scale",
            NifValue::Float(material.BaseColorScale as f64),
        );
        shader.set_field(
            "Soft Falloff Depth",
            NifValue::Float(material.SoftDepth as f64),
        );
        shader.set_field(
            "Greyscale Texture",
            NifValue::String(canonical_bgem_texture_path(&material.GrayscaleTexture)),
        );
        shader.set_field(
            "Env Map Texture",
            NifValue::String(canonical_bgem_texture_path(&material.EnvmapTexture)),
        );
        shader.set_field(
            "Normal Texture",
            NifValue::String(canonical_bgem_texture_path(&material.NormalTexture)),
        );
        shader.set_field(
            "Env Mask Texture",
            NifValue::String(canonical_bgem_texture_path(&material.EnvmapMaskTexture)),
        );
        shader.set_field(
            "Environment Map Scale",
            NifValue::Float(
                material
                    .EnvironmentMappingMaskScale
                    .or(material.header.env_mapping_mask_scale)
                    .unwrap_or(1.0) as f64,
            ),
        );
        normalized += 1;
    }

    if normalized > 0 {
        report.changes.push(format!(
            "BSEffectShaderProperty: normalized {normalized} external BGEM shader(s) from converted source material data"
        ));
    }
}

fn converted_source_bgem(
    material_path: &str,
    source_material_dir: Option<&Path>,
) -> Option<materials_native::bgem::BgemData> {
    let (bytes, relative, _) = read_source_material(material_path, source_material_dir)?;
    let bgem = materials_native::bgem::parse(&bytes).ok()?;
    Some(materials_native::convert::downgrade_bgem(
        bgem,
        &relative,
        materials_native::convert::Game::Fo76,
        materials_native::convert::Game::Fo4,
    ))
}

fn canonical_bgem_texture_path(path: &str) -> String {
    if !nonempty_material_texture(path) {
        return String::new();
    }
    canonical_texture_path(path, "", "")
}

fn nonempty_material_texture(path: &str) -> bool {
    !path.trim_end_matches('\0').trim().is_empty()
}

fn texture_clamp_mode(tile_u: bool, tile_v: bool) -> u64 {
    match (tile_u, tile_v) {
        (false, false) => 0,
        (false, true) => 1,
        (true, false) => 2,
        (true, true) => 3,
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
    let mut non_emitting_shaders: HashSet<usize> = HashSet::new();
    let mut emissive_capable_shaders: HashSet<usize> = HashSet::new();
    for (shader_id, material_path, texset_id) in shader_texture_sets {
        let (source_material_path, source_overridden) =
            material_source_override_path(&material_path, material_source_overrides);
        let source_shader_flags =
            source_bgsm_shader_flags(&source_material_path, source_material_dir);
        let wants_glow_map = source_shader_flags.is_some_and(|flags| flags.glow_map);
        // An unreadable material is treated as emissive-capable so a missing
        // source never silently darkens a shader.
        if source_shader_flags.is_some_and(|flags| !flags.emits) {
            non_emitting_shaders.insert(shader_id);
        } else {
            emissive_capable_shaders.insert(shader_id);
        }
        let material_texture_paths =
            converted_source_bgsm_texture_paths(&source_material_path, source_material_dir);
        {
            let Some(shader) = nif.blocks.get_mut(shader_id) else {
                continue;
            };
            let mut shader_changed = false;
            // Own_Emit is part of FO4's external-BGSM baseline, including
            // non-glowing vanilla rocks. A glow-emitting material additionally
            // needs the Glow Shader type (2) so FO4 selects the glow-map
            // technique and masks the emittance by the glow texture.
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
            if let Some(flags) = source_shader_flags {
                for field in ["Shader Flags 1", "Shader Flags 1:FO4"] {
                    shader_changed |= sync_shader_flag(shader, field, SLSF1_DECAL, flags.decal);
                    shader_changed |=
                        sync_shader_flag(shader, field, SLSF1_DYNAMIC_DECAL, flags.dynamic_decal);
                }
                for field in ["Shader Flags 2", "Shader Flags 2:FO4"] {
                    shader_changed |= sync_shader_flag(
                        shader,
                        field,
                        SLSF2_DOUBLE_SIDED as u64,
                        flags.double_sided,
                    );
                }
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
            let mut own_emit_set = set_shader_flag(shader, "Shader Flags 1", SLSF1_OWN_EMIT);
            own_emit_set |= set_shader_flag(shader, "Shader Flags 1:FO4", SLSF1_OWN_EMIT);
            if own_emit_set {
                shader_changed = true;
            }
            if wants_glow_map {
                let mut glow_set = set_shader_flag(shader, "Shader Flags 2", SLSF2_GLOW_MAP);
                glow_set |= set_shader_flag(shader, "Shader Flags 2:FO4", SLSF2_GLOW_MAP);
                if glow_set {
                    glow_flags_set += 1;
                    shader_changed = true;
                }
            } else {
                let mut cleared = clear_shader_flag(shader, "Shader Flags 2", SLSF2_GLOW_MAP);
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

    let (controllers_neutralized, sequence_driven) = neutralize_dormant_emissive_controllers(
        nif,
        &non_emitting_shaders,
        &emissive_capable_shaders,
    );
    if controllers_neutralized > 0 {
        report.changes.push(format!(
            "BSLightingShaderPropertyColorController: pinned {controllers_neutralized} emissive colour interpolator(s) to black for shaders whose source BGSM disables emittance"
        ));
    }
    if sequence_driven > 0 {
        report.changes.push(format!(
            "BSLightingShaderPropertyColorController: {sequence_driven} dormant emissive controller(s) are sequence-driven and were left alone (colour still comes from NiControllerSequence)"
        ));
    }
}

/// FO76 gates emittance on the BGSM (`EmitEnabled`), so a mesh can ship an
/// emissive colour controller that never fires. FO4 has no such gate once
/// `Own_Emit` is on — which is FO4's external-BGSM baseline — so those dormant
/// controllers wake up and wash the whole surface in their key colour. Pin the
/// interpolator to constant black instead of deleting blocks: renumbering would
/// desync `NiControllerSequence` controlled-block arrays.
fn neutralize_dormant_emissive_controllers(
    nif: &mut NifFile,
    non_emitting: &HashSet<usize>,
    emissive_capable: &HashSet<usize>,
) -> (usize, usize) {
    if non_emitting.is_empty() {
        return (0, 0);
    }
    const LSCC_EMISSIVE_COLOR: u64 = 1;

    let mut dormant: HashSet<usize> = HashSet::new();
    let mut in_use: HashSet<usize> = HashSet::new();
    for block in nif.blocks.iter() {
        if block.type_name != "BSLightingShaderPropertyColorController"
            || value_u64(block.get_field("Controlled Color")) != Some(LSCC_EMISSIVE_COLOR)
        {
            continue;
        }
        let Some(interpolator) = field_ref(block, "Interpolator").filter(|id| *id >= 0) else {
            continue;
        };
        let target = field_ref(block, "Target").unwrap_or(-1);
        // An interpolator shared with any shader that may legitimately emit
        // stays untouched, even if another controller says it is dormant.
        if target >= 0
            && non_emitting.contains(&(target as usize))
            && !emissive_capable.contains(&(target as usize))
        {
            dormant.insert(interpolator as usize);
        } else {
            in_use.insert(interpolator as usize);
        }
    }

    let mut neutralized = 0usize;
    let mut sequence_driven = 0usize;
    let candidates: Vec<usize> = dormant.difference(&in_use).copied().collect();
    for id in candidates {
        let Some(block) = nif.blocks.get_mut(id) else {
            continue;
        };
        if block.type_name != "NiPoint3Interpolator" {
            // NiBlendPoint3Interpolator gets its value from the owning
            // NiControllerSequence, not from this block.
            sequence_driven += 1;
            continue;
        }
        block.set_field("Value", NifValue::Vec3([0.0; 3]));
        block.set_field("Data", NifValue::Ref(-1));
        neutralized += 1;
    }
    (neutralized, sequence_driven)
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

fn sync_shader_flag(block: &mut NifBlock, field: &str, flag: u64, enabled: bool) -> bool {
    if enabled {
        set_shader_flag(block, field, flag)
    } else {
        clear_shader_flag(block, field, flag)
    }
}

#[derive(Clone, Copy, Default)]
struct SourceBgsmShaderFlags {
    glow_map: bool,
    emits: bool,
    decal: bool,
    dynamic_decal: bool,
    double_sided: bool,
}

fn source_bgsm_shader_flags(
    material_path: &str,
    source_material_dir: Option<&Path>,
) -> Option<SourceBgsmShaderFlags> {
    let Some((bytes, relative, _resolved)) =
        read_source_material(material_path, source_material_dir)
    else {
        return None;
    };
    let bgsm = materials_native::bgsm::parse(&bytes).ok()?;
    let glow_map = materials_native::convert::source_bgsm_enables_fo4_glowmap(&bgsm, &relative);
    Some(SourceBgsmShaderFlags {
        glow_map,
        emits: glow_map || bgsm.EmitEnabled,
        decal: bgsm.header.decal || bgsm.header.decal_nofade,
        dynamic_decal: bgsm.header.decal_nofade,
        double_sided: bgsm.header.two_sided,
    })
}

fn converted_source_bgsm_texture_paths(
    material_path: &str,
    source_material_dir: Option<&Path>,
) -> Option<Vec<(usize, String)>> {
    let (bytes, relative, resolved) = read_source_material(material_path, source_material_dir)?;
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

fn read_source_material(
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

// FO4 picks the BSLightingShaderMaterial subclass from Shader Type but gates
// render setup on the Environment_Mapping flag, so the two must agree: only
// type 1 allocates the envTexture member MakeValidForRendering reads. An
// env-flagged type-0 shader makes it read that pointer out of the material's
// float block -> IsTextureCubeMap derefs garbage -> CTD on cell load. FO76
// carries the flag as a name CRC independent of its own shader type, so a
// flattened shader can land on either side of the mismatch.
fn normalize_fo76_environment_mapping(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut clear_ids = Vec::new();
    let mut promote_ids = Vec::new();
    for block in &nif.blocks {
        if block.type_name != "BSLightingShaderProperty" {
            continue;
        }
        let flags = value_u64(block.get_field("Shader Flags 1")).unwrap_or(0);
        if flags & SLSF1_ENVIRONMENT_MAPPING == 0 {
            continue;
        }
        let has_cubemap = field_ref(block, "Texture Set")
            .filter(|id| *id >= 0)
            .and_then(|id| nif.get_block(id as usize))
            .is_some_and(|texset| {
                texset.type_name == "BSShaderTextureSet"
                    && value_array(texset.get_field("Textures"))
                        .get(4)
                        .is_some_and(non_empty_texture)
            });
        match value_u64(block.get_field("Shader Type")) {
            // Already consistent.
            Some(BSLSP_SHADER_TYPE_ENVIRONMENT_MAP) if has_cubemap => {}
            // A usable cubemap: keep the reflection and promote the type.
            Some(BSLSP_SHADER_TYPE_DEFAULT) if has_cubemap => promote_ids.push(block.block_id),
            // No cubemap to reflect, or a more specific type (glow, skin tint)
            // whose technique outranks the reflection -- drop the flag instead.
            _ => clear_ids.push(block.block_id),
        }
    }

    let mut cleared = 0usize;
    for shader_id in clear_ids {
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

    let mut promoted = 0usize;
    for shader_id in promote_ids {
        let Some(shader) = nif.blocks.get_mut(shader_id) else {
            continue;
        };
        shader.set_field(
            "Shader Type",
            NifValue::UInt(BSLSP_SHADER_TYPE_ENVIRONMENT_MAP),
        );
        // Type 1 turns on the schema's cond-gated tail; without the defaults
        // the block would serialize short.
        ensure_fo4_lighting_shader_conditional_fields(shader);
        promoted += 1;
    }

    if cleared > 0 {
        report.changes.push(format!(
            "BSLightingShaderProperty: cleared Environment_Mapping on {cleared} FO76 shader(s) without FO4 cubemap texture"
        ));
    }
    if promoted > 0 {
        report.changes.push(format!(
            "BSLightingShaderProperty: promoted {promoted} env-mapped FO76 shader(s) to FO4 Environment Map shader type"
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
            if shader.fields.contains_key("Shader Flags 1:FO4") {
                shader
                    .fields
                    .insert("Shader Flags 1:FO4".to_string(), NifValue::UInt(flags1));
            }
            let flags2 = value_u64(shader.get_field("Shader Flags 2")).unwrap_or(0)
                | SLSF2_DOUBLE_SIDED as u64
                | SLSF2_VERTEX_COLORS as u64
                | SLSF2_GLOW_MAP
                | SLSF2_TRANSFORM_CHANGED;
            shader.set_field("Shader Flags 2", NifValue::UInt(flags2));
            if shader.fields.contains_key("Shader Flags 2:FO4") {
                shader
                    .fields
                    .insert("Shader Flags 2:FO4".to_string(), NifValue::UInt(flags2));
            }
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

fn strip_fo76_cloth_blobs(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let cloth_ids: HashSet<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSClothExtraData")
        .map(|block| block.block_id)
        .collect();
    if cloth_ids.is_empty() {
        return;
    }

    let removed_count = cloth_ids.len();
    let detached_refs = detach_extra_data_refs(nif, &cloth_ids);
    remove_blocks(nif, cloth_ids);
    report.changes.push(format!(
        "Havok cloth: stripped {removed_count} BSClothExtraData block(s) for a static model variant"
    ));
    if detached_refs > 0 {
        report.changes.push(format!(
            "Havok cloth: detached {detached_refs} extra-data reference(s)"
        ));
    }
    fold_fo76_cloth_skin_bones(nif, report);
}

fn validate_fo4_cloth_blob(blob: &[u8]) -> Result<(), String> {
    let format =
        havok_native::api::hkx_detect_format_full(blob).map_err(|error| error.to_string())?;
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
        if suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
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
    /// Whether the NIF is an inventory ground-object world model (`GO_*` /
    /// `*_GO`). Set dressing and ground objects are indistinguishable by every
    /// in-mesh signal — both ship as dynamic-motion, dynamic-BSX, non-complex
    /// compounds on CLUTTER — so the role is the only thing that separates a
    /// lamp that must stay static from a dropped power armor piece that must
    /// fall. Vanilla FO4 ships its own ground objects (`go_t51_helmet.nif`)
    /// as dynamic compounds with the same BSX 194.
    is_ground_object: bool,
    /// `Prn=WEAPON` distinguishes held/dropped weapon models from static set
    /// dressing when both use the same dynamic, non-complex compound layout.
    is_weapon_model: bool,
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
        is_ground_object: is_fo76_ground_object_nif(nif),
        is_weapon_model: is_fo76_weapon_nif(nif),
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
    // FO76 keeps ground-object motion at runtime: none of its ground-object
    // NIFs carry motionCinfos, and most declare BSX without Dynamic plus
    // hknpMotionType::STATIC, so the BSX-gated routes below would ship them as
    // STATIC(1) bodies that read in-game as having no collision. Vanilla FO4
    // ships 189 of 192 `go*.nif` as dynamic clutter, so the ground-object role
    // decides. The source layer is inconsistent (Hellcat torso on CLUTTER,
    // Vulcan torso on STATIC), so accept exactly those two: 405 of the 410
    // affected meshes. Stragglers on other layers, and any volume layer reached
    // through the root-node-name fallback in `is_fo76_ground_object_nif`, keep
    // their source behaviour.
    if intent.is_ground_object
        && matches!(metadata.layer, Some(FO4_STATIC_LAYER | FO4_CLUTTER_LAYER))
    {
        return true;
    }
    let is_single_convex = body.is_some_and(source_body_is_single_convex);
    // FO76 set dressing can carry Dynamic motion and BSX Dynamic without being
    // loose clutter. Non-complex compounds must remain static for FO4 —
    // EXCEPT inventory ground objects, which are genuinely loose: a dropped
    // power armor piece (`GO_Ultra_Helmet`) is byte-identical to
    // `WhitespringLamp03Off` on every in-mesh signal (BSX 194, layer 4,
    // flags 128, motionType 2, compound_polytope), so only the ground-object
    // role separates them. Vanilla FO4 ships `go_t51_helmet.nif` as a dynamic
    // compound with that same BSX.
    if metadata.motion_type == Some(2)
        && intent.has_dynamic_bsx
        && !intent.has_complex_bsx
        && !is_single_convex
        && !intent.is_ground_object
        && !intent.is_weapon_model
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
    body.source_primitive.is_some()
        || body.source_polytopes.len() == 1
        || (body.source_polytopes.is_empty()
            && body.meshes.len() == 1
            && body.meshes[0].shape_type == "convex_hull")
}

struct CollisionPlanEntry {
    source_collision_id: usize,
    /// The source `bhkPhysicsSystem` block this entry's body came from (the
    /// collision object's Data ref). Non-constrained rebuilds emit one output
    /// physics system per distinct value, mirroring the source partitioning.
    source_system_id: usize,
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

fn synthesize_fo76_ground_object_collision(nif: &mut NifFile, report: &mut ConvertFileReport) {
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
            "FO76 ground object collision: no bounded visible geometry was available".to_string(),
        );
        return;
    };
    let parent_name = nif
        .get_block(parent_id)
        .and_then(|block| string_field(block, "Name"))
        .unwrap_or_default();
    let entry = CollisionPlanEntry {
        source_collision_id: root_id,
        source_system_id: root_id,
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
            is_ground_object: true,
            is_weapon_model: false,
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

fn is_fo76_weapon_nif(nif: &NifFile) -> bool {
    nif.blocks
        .iter()
        .filter(|block| block.type_name == "NiStringExtraData")
        .any(|block| {
            string_field(block, "Name")
                .is_some_and(|name| name.trim_end_matches('\0').eq_ignore_ascii_case("Prn"))
                && string_field(block, "String Data").is_some_and(|value| {
                    value.trim_end_matches('\0').eq_ignore_ascii_case("WEAPON")
                })
        })
}

fn rebuild_fo76_np_collision(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut collision_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkNPCollisionObject")
        .filter(|block| collision_data_is_physics_system(nif, block))
        .map(|block| block.block_id)
        .collect();
    if collision_ids.is_empty() {
        return;
    }
    let source_collision_count = collision_ids.len();
    let mut source_blobs = BTreeMap::new();
    for collision_id in collision_ids.iter().copied() {
        let Some(collision) = nif.get_block(collision_id) else {
            continue;
        };
        let system_id = field_ref(collision, "Data")
            .filter(|id| *id >= 0)
            .map(|id| id as usize)
            .unwrap_or(collision_id);
        source_blobs
            .entry(system_id)
            .or_insert_with(|| collision_physics_blob(nif, collision));
    }
    let source_contexts = source_blobs
        .iter()
        .filter_map(|(system_id, blob)| {
            blob.as_deref()
                .ok()
                .map(|blob| (*system_id, SourceCollisionContext::new(blob)))
        })
        .collect::<BTreeMap<_, _>>();
    let directly_converted = direct_convert_eligible_fo76_static_np_collision_systems(
        nif,
        &collision_ids,
        &source_blobs,
        &source_contexts,
        report,
    );
    collision_ids.retain(|collision_id| !directly_converted.contains(collision_id));
    if collision_ids.is_empty() {
        return;
    }
    // Several physics-system collision objects usually means a static assembly
    // (SCOL combined mesh, multi-part static, ...). Loose multi-part clutter can
    // also have several collision objects; keep those dynamic when BSX says the
    // NIF itself is dynamic.
    let nif_intent = nif_collision_intent(nif);
    let in_multi_body_assembly = source_collision_count > 1 && !nif_intent.has_dynamic_bsx;

    let mut remove = HashSet::new();
    let mut pending: Vec<CollisionPlanEntry> = Vec::new();
    let mut route_counts = RouteCounts::default();
    let mut regenerated = 0usize;
    let mut degenerate = 0usize;
    let mut degenerate_by_system = BTreeMap::new();
    for collision_id in collision_ids.iter().copied() {
        let Some(collision) = nif.get_block(collision_id).cloned() else {
            continue;
        };
        let body_id = collision
            .get_field("Body ID")
            .and_then(value_usize)
            .unwrap_or(0);
        let has_degenerate_shapes = field_ref(&collision, "Data")
            .filter(|id| *id >= 0)
            .map(|system_id| {
                *degenerate_by_system
                    .entry(system_id as usize)
                    .or_insert_with(|| collision_data_has_degenerate_shapes(nif, &collision))
            })
            .unwrap_or(false);
        if has_degenerate_shapes {
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
        let source_system_id = field_ref(&collision, "Data")
            .filter(|id| *id >= 0)
            .map(|id| id as usize)
            .unwrap_or(collision_id);
        let source_blob = source_blobs
            .get(&source_system_id)
            .expect("source collision blob entry");
        let source_context = source_contexts.get(&source_system_id);
        let mut source_metadata = source_context
            .map(|context| context.body_metadata(body_id))
            .unwrap_or_default();
        source_metadata = source_metadata_for_nif_intent(source_metadata, nif_intent);
        // Decode this body's true mass distribution from the source blob so the
        // builder can use the real COM / volume / inertia instead of the AABB
        // approximation. `None` for statics / undecodable; the builder's
        // `mass > 0` guard keeps it from touching static bodies.
        let mass_distribution =
            source_context.and_then(|context| context.mass_distribution(body_id));
        let mut planned_parent_ref = parent_ref as usize;
        let mut source_summary = None;
        let planned = match source_blob
            .as_ref()
            .map_err(|error| error.clone())
            .and_then(|_| {
                source_context
                    .expect("successful blob has source context")
                    .extract_body(body_id)
            })
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
            source_system_id,
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
        // Constrained (articulated) systems remap constraint body handles by
        // position across the whole pending set, so they keep the single
        // combined blob. Everything else mirrors the SOURCE bhkPhysicsSystem
        // partitioning: FO4 binds a layer-31 stair helper to the step body of
        // its OWN system (vanilla stairs pair them two-per-system; vanilla
        // SCOLs never share a system across members). Merging every SCOL
        // member into one 5-body system leaves the helpers unbound and the
        // stairs unclimbable (Point Pleasant SCOLs 00491AE1 / 00491B05).
        let install_result = if grafted_constraints.is_some() {
            install_fo4_np_collision_system(nif, &pending, grafted_constraints.as_ref())
        } else {
            install_fo4_np_collision_systems_grouped(nif, &mut pending)
        };
        match install_result {
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

fn direct_convert_eligible_fo76_static_np_collision_systems(
    nif: &mut NifFile,
    collision_ids: &[usize],
    source_blobs: &BTreeMap<usize, Result<Vec<u8>, String>>,
    source_contexts: &BTreeMap<usize, SourceCollisionContext<'_>>,
    report: &mut ConvertFileReport,
) -> HashSet<usize> {
    let nif_intent = nif_collision_intent(nif);
    let has_animation_blocks = nif.blocks.iter().any(|block| {
        block.type_name.contains("Controller")
            || block.type_name.contains("Interpolator")
            || block.type_name.contains("Sequence")
    });
    let mut collisions_by_system: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for collision_id in collision_ids {
        let Some(system_id) = nif
            .get_block(*collision_id)
            .and_then(|collision| field_ref(collision, "Data"))
            .filter(|id| *id >= 0)
            .map(|id| id as usize)
        else {
            continue;
        };
        collisions_by_system
            .entry(system_id)
            .or_default()
            .push(*collision_id);
    }

    let mut converted_collision_ids = HashSet::new();
    let mut converted_system_count = 0usize;
    for (system_id, system_collision_ids) in collisions_by_system {
        let mut body_ids = Vec::with_capacity(system_collision_ids.len());
        let mut target_ids = Vec::with_capacity(system_collision_ids.len());
        let bindings_are_complete = system_collision_ids.iter().all(|collision_id| {
            let Some(collision) = nif.get_block(*collision_id) else {
                return false;
            };
            let Some(body_id) = collision.get_field("Body ID").and_then(value_usize) else {
                return false;
            };
            let Some(target_id) = field_ref(collision, "Target")
                .filter(|id| *id >= 0)
                .map(|id| id as usize)
            else {
                return false;
            };
            body_ids.push(body_id);
            target_ids.push(target_id);
            true
        });
        if !bindings_are_complete {
            continue;
        }

        let Some(source_blob) = source_blobs.get(&system_id).and_then(|blob| blob.as_ref().ok())
        else {
            continue;
        };
        let Some(source_context) = source_contexts.get(&system_id) else {
            continue;
        };
        let source_body_count = source_context.body_count();
        let requires_rebuild = nif_intent.has_dynamic_bsx
            || body_ids.iter().copied().any(|body_id| {
                let metadata = source_metadata_for_nif_intent(
                    source_context.body_metadata(body_id),
                    nif_intent,
                );
                metadata.is_dynamic
                    || !metadata.layer.is_some_and(is_direct_static_collision_layer)
                    || (metadata
                        .motion_type
                        .is_some_and(|motion_type| motion_type != 0)
                        && (metadata.has_ref_mass_distribution || has_animation_blocks))
            })
            || target_ids.iter().copied().any(|target_id| {
                nif.get_block(target_id)
                    .and_then(|target| string_field(target, "Name"))
                    .is_some_and(|name| name.to_ascii_lowercase().contains("navcut"))
            });
        if requires_rebuild {
            continue;
        }
        body_ids.sort_unstable();
        if source_body_count == 0
            || source_body_count != body_ids.len()
            || !body_ids.iter().copied().eq(0..source_body_count)
        {
            continue;
        }

        let Ok(converted_blob) = convert_fo76_embedded_static_collision_direct(source_blob) else {
            continue;
        };
        let Some(system) = nif.blocks.get_mut(system_id) else {
            continue;
        };
        system.set_field(
            "Binary Data",
            crate::cloth::bytes_to_byte_array(&converted_blob),
        );
        converted_system_count += 1;
        converted_collision_ids.extend(system_collision_ids);
        for target_id in target_ids {
            ensure_root_havok_bsx_flag(nif, target_id);
        }
    }

    if converted_system_count > 0 {
        report.changes.push(format!(
            "FO76 hknp collision: direct-transcoded {converted_system_count} static physics system(s) in place; preserved {} bhkNPCollisionObject binding(s)",
            converted_collision_ids.len()
        ));
    }
    converted_collision_ids
}

fn is_direct_static_collision_layer(layer: u8) -> bool {
    matches!(
        layer,
        // STATIC, ANIM_STATIC, TRANSPARENT, TREES, PROPS, TERRAIN, GROUND, DEBRIS,
        // TRANSPARENT_SMALL, INVISIBLE_WALL, STAIRHELPER, COLLISIONBOX.
        1 | 2 | 3 | 9 | 10 | 13 | 17 | 19 | 20 | 26 | 27 | 31 | 35
    )
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
/// never plays the NIF's Open/Close NiControllerSequence. Multi-body systems get
/// a motionCinfo anyway (the `bodies.len() > 1 && cm_count > 0` rule in
/// `build_fo4_multi_body_collision`); single-body doors (e.g. CivWarDoor01/02)
/// depend on this. Mirrors the Python `_motion_type_for_layer` in
/// `nif/operations/collision.py`.
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
    if let Some((primitive, _)) = &body.source_primitive {
        return match primitive {
            havok_native::collision::SourcePrimitiveShape::Sphere { .. } => "sphere",
            havok_native::collision::SourcePrimitiveShape::Capsule(_) => "capsule",
            havok_native::collision::SourcePrimitiveShape::Convex(_) => "convex",
        }
        .to_string();
    }
    if body.source_polytopes.len() == 1 {
        return "polytope".to_string();
    }
    if body.meshes.len() != 1 {
        return "compound".to_string();
    }
    match body.meshes[0].shape_type.as_str() {
        "convex_hull" => "polytope".to_string(),
        other => other.to_string(),
    }
}

fn source_collision_shape_summary(body: &ExtractedCollisionBody) -> String {
    if let Some((primitive, _)) = &body.source_primitive {
        return match primitive {
            havok_native::collision::SourcePrimitiveShape::Sphere { radius, .. } => {
                format!("sphere(r={radius:.3})")
            }
            havok_native::collision::SourcePrimitiveShape::Capsule(shape) => format!(
                "capsule({}v/{}p/{}f/{}i,r={:.3},cr={:.3})",
                shape.hull.vertices.len(),
                shape.hull.planes.len(),
                shape.hull.faces.len(),
                shape.hull.indices.len(),
                shape.a[3],
                shape.convex_radius
            ),
            havok_native::collision::SourcePrimitiveShape::Convex(shape) => {
                format!(
                    "convex({}v,cr={:.3})",
                    shape.vertices.len(),
                    shape.convex_radius
                )
            }
        };
    }
    if body.source_polytopes.len() == 1 {
        let shape = &body.source_polytopes[0];
        return format!(
            "polytope({}v/{}p/{}f/{}i source)",
            shape.vertices.len(),
            shape.planes.len(),
            shape.faces.len(),
            shape.indices.len()
        );
    }
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
        MultiBodyShape::SourceConvex { .. } => "convex",
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
        MultiBodyShape::Capsule { shape } => {
            format!(
                "capsule({}v,r={:.3},cr={:.3})",
                shape.hull.vertices.len(),
                shape.a[3],
                shape.convex_radius
            )
        }
        MultiBodyShape::SourceConvex { shape } => {
            format!(
                "convex({}v,cr={:.3})",
                shape.vertices.len(),
                shape.convex_radius
            )
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
        CollisionRoute::SourceSphere => "source-sphere",
        CollisionRoute::SourceCapsule => "source-capsule",
        CollisionRoute::SourceConvex => "source-convex",
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

fn np_collision_body_frame(
    metadata: SourceBodyMetadata,
    is_articulated: bool,
) -> ([f32; 4], [f32; 4]) {
    if is_articulated {
        (
            metadata.position.unwrap_or([0.0; 4]),
            metadata.orientation.unwrap_or([0.0, 0.0, 0.0, 1.0]),
        )
    } else {
        ([0.0; 4], [0.0, 0.0, 0.0, 1.0])
    }
}

fn install_fo4_np_collision_system(
    nif: &mut NifFile,
    entries: &[CollisionPlanEntry],
    constraints: Option<&GraftedConstraints>,
) -> Result<usize, String> {
    let is_articulated = constraints.is_some_and(|constraints| !constraints.is_empty());
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
        .map(|entry| {
            let (position, orientation) =
                np_collision_body_frame(entry.source_metadata, is_articulated);
            BodyMeta {
                // In a constrained assembly the source group/system filter bits are
                // load-bearing (they keep the linked bodies from self-colliding), so
                // carry the full filter; otherwise the builder writes just the layer.
                collision_filter_info: constraints.and(entry.source_metadata.collision_filter_info),
                layer: entry.planned.layer,
                body_flags: entry.source_metadata.body_flags,
                material_flags: entry.source_metadata.material_flags,
                material_trigger_type: entry.source_metadata.material_trigger_type,
                // Static multi-body set dressing has world-baked shapes and must stay
                // at origin. Articulated systems are different: their constraint pivots
                // are relative to each source body frame, so dropping those frames
                // makes every segment spawn at one point and the solver explodes them.
                position,
                orientation,
                motion_type: body_motion_type_for_entry(entry),
                body_mass: entry.body_mass,
                mass_distribution: entry.mass_distribution,
            }
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

/// Emit one FO4 physics system per distinct SOURCE `bhkPhysicsSystem`,
/// preserving the source's body grouping (a stair-step body and its layer-31
/// helper stay paired in one system; separate SCOL members get separate
/// systems). Reorders `entries` so each group is a contiguous slice; the
/// pre-existing shape-rank order is kept within each group.
fn install_fo4_np_collision_systems_grouped(
    nif: &mut NifFile,
    entries: &mut [CollisionPlanEntry],
) -> Result<usize, String> {
    let initial_block_count = nif.blocks.len();
    let original_parent_collisions = entries
        .iter()
        .map(|entry| {
            (
                entry.parent_id,
                nif.get_block(entry.parent_id)
                    .and_then(|parent| parent.get_field("Collision Object"))
                    .cloned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut group_rank: Vec<usize> = Vec::new();
    for entry in entries.iter() {
        if !group_rank.contains(&entry.source_system_id) {
            group_rank.push(entry.source_system_id);
        }
    }
    entries.sort_by_key(|entry| {
        group_rank
            .iter()
            .position(|id| *id == entry.source_system_id)
            .unwrap_or(usize::MAX)
    });
    let mut regenerated = 0usize;
    let mut start = 0usize;
    while start < entries.len() {
        let key = entries[start].source_system_id;
        let mut end = start;
        while end < entries.len() && entries[end].source_system_id == key {
            end += 1;
        }
        match install_fo4_np_collision_system(nif, &entries[start..end], None) {
            Ok(count) => regenerated += count,
            Err(error) => {
                nif.blocks.truncate(initial_block_count);
                for (parent_id, collision) in original_parent_collisions {
                    let Some(parent) = nif.blocks.get_mut(parent_id) else {
                        continue;
                    };
                    match collision {
                        Some(value) => parent.set_field("Collision Object", value),
                        None => parent.fields.retain(|name, _| name != "Collision Object"),
                    }
                }
                nif.rebuild_header();
                return Err(error);
            }
        }
        start = end;
    }
    Ok(regenerated)
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
            material_flags: entry.source_metadata.material_flags,
            material_trigger_type: entry.source_metadata.material_trigger_type,
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
        MultiBodyShape::CompressedMesh { .. } | MultiBodyShape::RawCompressedMesh { .. } => 0,
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

#[derive(Debug, Clone)]
struct LegacyDynamicCollision {
    shape: Option<MultiBodyShape>,
    position: [f32; 4],
    orientation: [f32; 4],
    mass: f32,
    friction: f32,
    restitution: f32,
}

fn regenerate_fo4_collision(nif: &mut NifFile, source_game: &str, report: &mut ConvertFileReport) {
    let collision_ids: Vec<usize> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkCollisionObject")
        .map(|block| block.block_id)
        .collect();
    if collision_ids.is_empty() {
        return;
    }

    let collision_intent = nif_collision_intent(nif);
    // Built once per NIF: the render mesh decides which way collision faces.
    let visible = crate::skyrim_collision::VisibleFacets::new(collect_visible_facets(nif));
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
        let dynamic_collision =
            legacy_dynamic_collision(nif, &collision, source_game, collision_intent);
        // The source shape chain is about to be stripped, so decode it now. A
        // failure here falls back to the visible-mesh AABB rather than dropping
        // collision outright.
        let decoded = if dynamic_collision.is_none() {
            decode_legacy_source_shape(nif, &collision, source_game, &parent, &visible, report)
        } else {
            None
        };
        pending.push((
            string_field(&parent, "Name").unwrap_or_default(),
            parent.type_name.clone(),
            dynamic_collision,
            decoded,
        ));
        if let Some(parent_mut) = nif.blocks.get_mut(parent_ref as usize) {
            parent_mut.set_field("Collision Object", NifValue::Ref(-1));
        }
        collect_collision_subtree(nif, collision_id, &mut remove);
    }

    remove_blocks(nif, remove);

    let mut regenerated = 0usize;
    let mut dynamic = 0usize;
    let mut source_shapes = 0usize;
    for (parent_name, parent_type, dynamic_collision, decoded) in pending {
        let Some(parent_id) = find_node_by_name_and_type(nif, &parent_name, &parent_type) else {
            report.warnings.push(format!(
                "Legacy collision parent {parent_name:?} not found after strip; skipping"
            ));
            continue;
        };
        let is_dynamic = dynamic_collision.is_some();
        let is_source_shape = decoded.is_some();
        let built = match (dynamic_collision, decoded) {
            (Some(spec), _) => {
                let vertices = collect_geometry_vertices(nif, parent_id);
                if vertices.len() < 3 {
                    report.warnings.push(format!(
                        "Node {parent_name:?} has no FO4 triangle geometry after strip; skipping FO4 collision regeneration"
                    ));
                    continue;
                }
                build_dynamic_legacy_collision(nif, parent_id, &vertices, spec)
            }
            (None, Some(shape)) => {
                crate::skyrim_collision::install_static_collision(nif, parent_id, shape)
            }
            (None, None) => {
                let vertices = collect_geometry_vertices(nif, parent_id);
                if vertices.len() < 3 {
                    report.warnings.push(format!(
                        "Node {parent_name:?} has no FO4 triangle geometry after strip; skipping FO4 collision regeneration"
                    ));
                    continue;
                }
                build_box_collision(nif, parent_id, &vertices)
                    .ok_or_else(|| "visible geometry has no collision volume".to_string())
            }
        };
        if built.is_ok() {
            ensure_root_havok_bsx_flag(nif, parent_id);
            regenerated += 1;
            if is_dynamic {
                dynamic += 1;
            }
            if is_source_shape {
                source_shapes += 1;
            }
        } else if let Err(error) = built {
            report.warnings.push(format!(
                "Legacy collision parent {parent_name:?}: FO4 rebuild failed ({error})"
            ));
        }
    }

    report.changes.push(format!(
        "Legacy collision: stripped {} chain(s); regenerated {regenerated} FO4 collision object(s) ({dynamic} dynamic, {source_shapes} from source shapes, {} from visible-mesh AABB)",
        collision_ids.len(),
        regenerated.saturating_sub(dynamic + source_shapes),
    ));
}

/// Decode the source rigid body's real collision shape for FO3/FNV. Returns
/// `None` when the chain is unsupported so the caller can fall back to the
/// visible-mesh AABB.
fn decode_legacy_source_shape(
    nif: &NifFile,
    collision: &NifBlock,
    source_game: &str,
    parent: &NifBlock,
    visible: &crate::skyrim_collision::VisibleFacets,
    report: &mut ConvertFileReport,
) -> Option<MultiBodyShape> {
    if !matches!(source_game, "fnv" | "fo3") {
        return None;
    }
    let body_id = field_ref(collision, "Body").filter(|id| *id >= 0)? as usize;
    match crate::skyrim_collision::decode_legacy_static_shape(
        nif,
        body_id,
        LEGACY_HAVOK_UNIT_SCALE,
        visible,
    ) {
        Ok(shape) => Some(shape),
        Err(error) => {
            report.warnings.push(format!(
                "Legacy collision parent {:?}: source shape unsupported ({error}); using visible-mesh AABB",
                string_field(parent, "Name").unwrap_or_default()
            ));
            None
        }
    }
}

fn legacy_dynamic_collision(
    nif: &NifFile,
    collision: &NifBlock,
    source_game: &str,
    intent: NifCollisionIntent,
) -> Option<LegacyDynamicCollision> {
    if !matches!(source_game, "fnv" | "fo3") || !intent.has_dynamic_bsx {
        return None;
    }
    let body_id = field_ref(collision, "Body").filter(|id| *id >= 0)? as usize;
    let body = nif.get_block(body_id)?;
    let info = struct_fields(body.get_field("Rigid Body Info:550_660"))?;
    let layer = struct_fields(info.get("Havok Filter"))
        .and_then(|filter| value_u64(filter.get("Layer:FO")))?;
    let motion_system = value_u64(info.get("Motion System"))?;
    let mass = value_f64(info.get("Mass"))? as f32;
    if layer != u64::from(FO4_CLUTTER_LAYER)
        || motion_system != 2
        || !mass.is_finite()
        || mass <= 0.0
    {
        return None;
    }

    let scale = LEGACY_HAVOK_UNIT_SCALE;
    let position = vec4_value(info.get("Translation")).unwrap_or([0.0; 4]);
    let position = [
        position[0] * scale,
        position[1] * scale,
        position[2] * scale,
        position[3],
    ];
    let orientation = vec4_value(info.get("Rotation")).unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let shape = field_ref(body, "Shape")
        .filter(|id| *id >= 0)
        .and_then(|id| nif.get_block(id as usize))
        .and_then(|shape| match shape.type_name.as_str() {
            "bhkSphereShape" => value_f64(shape.get_field("Radius"))
                .map(|radius| dynamic_sphere_polytope(radius as f32 * scale)),
            _ => None,
        });

    Some(LegacyDynamicCollision {
        shape,
        position,
        orientation,
        mass,
        friction: value_f64(info.get("Friction")).unwrap_or(0.5) as f32,
        restitution: value_f64(info.get("Restitution")).unwrap_or(0.4) as f32,
    })
}

fn struct_fields(value: Option<&NifValue>) -> Option<&IndexMap<String, NifValue>> {
    match value? {
        NifValue::Struct(fields) => Some(fields),
        _ => None,
    }
}

fn vec4_value(value: Option<&NifValue>) -> Option<[f32; 4]> {
    match value? {
        NifValue::Vec4(value) | NifValue::Color4(value) => Some(*value),
        NifValue::Struct(fields) => Some([
            value_f64(fields.get("x"))? as f32,
            value_f64(fields.get("y"))? as f32,
            value_f64(fields.get("z"))? as f32,
            value_f64(fields.get("w"))? as f32,
        ]),
        _ => None,
    }
}

fn dynamic_sphere_polytope(radius: f32) -> MultiBodyShape {
    let mut vertices = Vec::with_capacity(26);
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if x == 0 && y == 0 && z == 0 {
                    continue;
                }
                let direction = [x as f32, y as f32, z as f32];
                let length = direction
                    .iter()
                    .map(|value| value * value)
                    .sum::<f32>()
                    .sqrt();
                vertices.push(direction.map(|value| value * radius / length));
            }
        }
    }
    MultiBodyShape::Polytope { vertices }
}

fn collect_collision_subtree(nif: &NifFile, root_id: usize, out: &mut HashSet<usize>) {
    if !out.insert(root_id) {
        return;
    }
    let Some(block) = nif.get_block(root_id) else {
        return;
    };
    for field_name in ["Body", "Data", "Shape", "Sub Shapes", "Strips Data"] {
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

fn build_dynamic_legacy_collision(
    nif: &mut NifFile,
    parent_id: usize,
    vertices: &[[f32; 3]],
    spec: LegacyDynamicCollision,
) -> Result<(), String> {
    let LegacyDynamicCollision {
        shape,
        position,
        orientation,
        mass,
        friction,
        restitution,
    } = spec;
    let (shape, position, orientation) = match shape {
        Some(shape) => (shape, position, orientation),
        None => (
            box_collision_shape(vertices)
                .ok_or_else(|| "visible geometry has no collision volume".to_string())?,
            [0.0; 4],
            [0.0, 0.0, 0.0, 1.0],
        ),
    };
    let bodies = [shape];
    let material_crcs = [None];
    let body_metas = [BodyMeta {
        collision_filter_info: None,
        layer: FO4_CLUTTER_LAYER,
        body_flags: None,
        material_flags: None,
        material_trigger_type: None,
        position,
        orientation,
        motion_type: BodyMotionType::Static,
        body_mass: Some(mass),
        mass_distribution: None,
    }];
    let options = BuildOptions {
        friction,
        restitution,
        layer: FO4_CLUTTER_LAYER,
        mass,
        convex_radius: DEFAULT_COLLISION_RADIUS as f32,
        materials: Vec::new(),
        user_data: None,
        body_props_raw: None,
        mass_distribution: None,
    };
    let diagnostic_context = format!("legacy dynamic collision parent block {parent_id}");
    let blob = build_validated_np_blob(
        &bodies,
        &material_crcs,
        &body_metas,
        &options,
        None,
        &diagnostic_context,
    )?;

    let mut physics_fields = IndexMap::new();
    physics_fields.insert(
        "Binary Data".to_string(),
        crate::cloth::bytes_to_byte_array(blob.as_slice()),
    );
    let physics_id = nif.add_block("bhkPhysicsSystem", Some(physics_fields));

    let mut collision_fields = IndexMap::new();
    collision_fields.insert("Flags".to_string(), NifValue::UInt(0x80));
    collision_fields.insert("Target".to_string(), NifValue::Ref(parent_id as i32));
    collision_fields.insert("Data".to_string(), NifValue::Ref(physics_id as i32));
    collision_fields.insert("Body ID".to_string(), NifValue::UInt(0));
    let collision_id = nif.add_block("bhkNPCollisionObject", Some(collision_fields));
    nif.blocks[parent_id].set_field("Collision Object", NifValue::Ref(collision_id as i32));
    Ok(())
}

fn build_box_collision(nif: &mut NifFile, parent_id: usize, vertices: &[[f32; 3]]) -> Option<()> {
    let shape = box_collision_shape(vertices)?;
    crate::skyrim_collision::install_static_collision(nif, parent_id, shape).ok()
}

fn box_collision_shape(vertices: &[[f32; 3]]) -> Option<MultiBodyShape> {
    if vertices.is_empty() {
        return None;
    }
    let mut mins = vertices[0];
    let mut maxs = vertices[0];
    for vertex in vertices.iter().skip(1) {
        for axis in 0..3 {
            mins[axis] = mins[axis].min(vertex[axis]);
            maxs[axis] = maxs[axis].max(vertex[axis]);
        }
    }
    let mins = mins.map(|value| value / HAVOK_SCALE);
    let maxs = maxs.map(|value| value / HAVOK_SCALE);
    let box_vertices = vec![
        [mins[0], mins[1], mins[2]],
        [maxs[0], mins[1], mins[2]],
        [mins[0], maxs[1], mins[2]],
        [maxs[0], maxs[1], mins[2]],
        [mins[0], mins[1], maxs[2]],
        [maxs[0], mins[1], maxs[2]],
        [mins[0], maxs[1], maxs[2]],
        [maxs[0], maxs[1], maxs[2]],
    ];
    Some(MultiBodyShape::Polytope {
        vertices: box_vertices,
    })
}

/// Render-mesh facets in FO4 Havok units, used to decide which way a converted
/// collision surface should face. Runs after `strips_to_tri_shape`, so the
/// geometry is already `BSTriShape`.
fn collect_visible_facets(nif: &NifFile) -> Vec<crate::skyrim_collision::VisibleFacet> {
    let mut facets = Vec::new();
    for block in &nif.blocks {
        if !matches!(
            block.type_name.as_str(),
            "BSTriShape" | "BSSubIndexTriShape"
        ) {
            continue;
        }
        let vertices: Vec<[f32; 3]> = value_array(block.get_field("Vertex Data"))
            .iter()
            .filter_map(|vertex| match vertex {
                NifValue::Struct(fields) => fields
                    .get("Vertex")
                    .and_then(|value| vec3_value(Some(value))),
                _ => None,
            })
            .map(|vertex| vertex.map(|value| value / HAVOK_SCALE))
            .collect();
        if vertices.len() < 3 {
            continue;
        }
        for triangle in value_array(block.get_field("Triangles")) {
            let NifValue::Struct(fields) = triangle else {
                continue;
            };
            let indices = ["v1", "v2", "v3"]
                .into_iter()
                .filter_map(|key| value_usize(fields.get(key)?))
                .collect::<Vec<_>>();
            if indices.len() != 3 || indices.iter().any(|index| *index >= vertices.len()) {
                continue;
            }
            let (a, b, c) = (
                vertices[indices[0]],
                vertices[indices[1]],
                vertices[indices[2]],
            );
            let edge1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let edge2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let normal = [
                edge1[1] * edge2[2] - edge1[2] * edge2[1],
                edge1[2] * edge2[0] - edge1[0] * edge2[2],
                edge1[0] * edge2[1] - edge1[1] * edge2[0],
            ];
            if !normal.iter().all(|value| value.is_finite())
                || normal.iter().all(|value| *value == 0.0)
            {
                continue;
            }
            facets.push(crate::skyrim_collision::VisibleFacet {
                centroid: [
                    (a[0] + b[0] + c[0]) / 3.0,
                    (a[1] + b[1] + c[1]) / 3.0,
                    (a[2] + b[2] + c[2]) / 3.0,
                ],
                normal,
            });
        }
    }
    facets
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
        root.set_field("Num Extra Data List", NifValue::UInt(extra_count as u64));
    }
}

const BSX_HAVOK_FLAG: u64 = 0x02;

fn deduplicate_fo76_exact_vertices(nif: &mut NifFile, report: &mut ConvertFileReport) {
    enum VertexBucket {
        One(usize),
        Collisions(Vec<usize>),
    }

    let skin_use_counts = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "BSTriShape" | "BSSubIndexTriShape" | "BSMeshLODTriShape"
            )
        })
        .filter_map(|shape| field_ref(shape, "Skin").filter(|skin| *skin >= 0))
        .fold(HashMap::<i32, usize>::new(), |mut counts, skin| {
            *counts.entry(skin).or_default() += 1;
            counts
        });
    let shape_ids = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "BSTriShape" | "BSSubIndexTriShape" | "BSMeshLODTriShape"
            )
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    let mut shapes = 0usize;
    let mut removed = 0usize;
    let mut skipped_shared_skin = 0usize;

    for shape_id in shape_ids {
        let shape = &nif.blocks[shape_id];
        let has_array_values = |field| matches!(shape.get_field(field), Some(NifValue::Array(values)) if !values.is_empty());
        if has_array_values("Particle Vertices")
            || has_array_values("Particle Normals")
            || has_array_values("Vertices")
        {
            continue;
        }
        let skin_id = field_ref(shape, "Skin").filter(|skin| *skin >= 0);
        if skin_id.is_some_and(|skin| skin_use_counts.get(&skin).copied().unwrap_or(0) > 1) {
            skipped_shared_skin += 1;
            continue;
        }
        let Some(NifValue::Array(vertices)) = shape.get_field("Vertex Data") else {
            continue;
        };
        if vertices.len() < 2 {
            continue;
        }
        let old_vertex_count = vertices.len();
        let mut unique_source_indices = Vec::with_capacity(vertices.len());
        let mut buckets = HashMap::<u64, VertexBucket>::with_capacity(vertices.len());
        let mut vertex_remap = Vec::with_capacity(vertices.len());
        for (source_id, vertex) in vertices.iter().enumerate() {
            let hash = conversion_value_hash(vertex);
            let unique_id = match buckets.entry(hash) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let unique_id = unique_source_indices.len();
                    unique_source_indices.push(source_id);
                    entry.insert(VertexBucket::One(unique_id));
                    unique_id
                }
                std::collections::hash_map::Entry::Occupied(entry) => {
                    let bucket = entry.into_mut();
                    match bucket {
                        VertexBucket::One(candidate) => {
                            let first = *candidate;
                            if &vertices[unique_source_indices[first]] == vertex {
                                first
                            } else {
                                let unique_id = unique_source_indices.len();
                                unique_source_indices.push(source_id);
                                *bucket = VertexBucket::Collisions(vec![first, unique_id]);
                                unique_id
                            }
                        }
                        VertexBucket::Collisions(candidates) => candidates
                            .iter()
                            .copied()
                            .find(|candidate| &vertices[unique_source_indices[*candidate]] == vertex)
                            .unwrap_or_else(|| {
                                let unique_id = unique_source_indices.len();
                                unique_source_indices.push(source_id);
                                candidates.push(unique_id);
                                unique_id
                            }),
                    }
                }
            };
            vertex_remap.push(unique_id);
        }
        let duplicate_count = old_vertex_count - unique_source_indices.len();
        if duplicate_count == 0 {
            continue;
        }
        let vertices = match nif.blocks[shape_id].get_field_mut("Vertex Data") {
            Some(NifValue::Array(vertices)) => std::mem::take(vertices),
            _ => unreachable!("vertex data was validated as an array"),
        };
        let mut retained = unique_source_indices.into_iter();
        let mut next_retained = retained.next();
        let unique = vertices
            .into_iter()
            .enumerate()
            .filter_map(|(source_id, vertex)| {
                (next_retained == Some(source_id)).then(|| {
                    next_retained = retained.next();
                    vertex
                })
            })
            .collect::<Vec<_>>();

        let mut triangles = match nif.blocks[shape_id].get_field_mut("Triangles") {
            Some(NifValue::Array(triangles)) => std::mem::take(triangles),
            _ => Vec::new(),
        };
        for triangle in &mut triangles {
            remap_triangle_vertex_indices(triangle, &vertex_remap);
        }
        let triangle_count = triangles.len();
        if let Some(skin_id) = skin_id {
            for partition_id in reachable_skin_partition_ids(nif, skin_id as usize) {
                if let Some(partition) = nif.blocks.get_mut(partition_id) {
                    for value in partition.fields.values_mut() {
                        remap_skin_vertex_maps(value, &vertex_remap);
                    }
                }
            }
        }

        let shape = &mut nif.blocks[shape_id];
        shape.set_field("Num Vertices", NifValue::UInt(unique.len() as u64));
        shape.set_field("Vertex Data", NifValue::Array(unique));
        shape.set_field("Triangles", NifValue::Array(triangles));
        if shape.get_field("Data Size").is_some() {
            let stride_words = value_u64(shape.get_field("Vertex Desc")).unwrap_or(0) & 0x0f;
            let data_size = stride_words * 4 * (old_vertex_count - duplicate_count) as u64
                + triangle_count as u64 * 6;
            shape.set_field("Data Size", NifValue::UInt(data_size));
        }
        shapes += 1;
        removed += duplicate_count;
    }

    if removed > 0 {
        report.changes.push(format!(
            "Exact vertex deduplication: collapsed {removed} full-payload duplicate(s) across {shapes} shape(s)"
        ));
    }
    if skipped_shared_skin > 0 {
        report.warnings.push(format!(
            "Exact vertex deduplication skipped {skipped_shared_skin} shape(s) sharing a skin object; independent vertex remaps would be ambiguous"
        ));
    }
}

fn conversion_value_hash(value: &NifValue) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_conversion_value(value, &mut hasher);
    hasher.finish()
}

fn hash_conversion_value(value: &NifValue, hasher: &mut DefaultHasher) {
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
            values.len().hash(hasher);
            for value in values {
                hash_conversion_value(value, hasher);
            }
        }
        NifValue::Struct(fields) => {
            fields.len().hash(hasher);
            for (key, value) in fields {
                key.hash(hasher);
                hash_conversion_value(value, hasher);
            }
        }
        NifValue::Bytes(values) => values.hash(hasher),
    }
}

fn remap_triangle_vertex_indices(value: &mut NifValue, vertex_remap: &[usize]) {
    let NifValue::Struct(fields) = value else {
        return;
    };
    for (field, index) in fields.iter_mut() {
        if matches!(field.split(':').next(), Some("v1" | "v2" | "v3")) {
            remap_numeric_vertex_index(index, vertex_remap);
        } else {
            remap_triangle_vertex_indices(index, vertex_remap);
        }
    }
}

fn remap_numeric_vertex_index(value: &mut NifValue, vertex_remap: &[usize]) {
    match value {
        NifValue::Int(index) if *index >= 0 && (*index as usize) < vertex_remap.len() => {
            *index = vertex_remap[*index as usize] as i64;
        }
        NifValue::UInt(index) if (*index as usize) < vertex_remap.len() => {
            *index = vertex_remap[*index as usize] as u64;
        }
        _ => {}
    }
}

fn reachable_skin_partition_ids(nif: &NifFile, skin_id: usize) -> Vec<usize> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = vec![skin_id];
    while let Some(block_id) = pending.pop() {
        if block_id >= nif.blocks.len() || !seen.insert(block_id) {
            continue;
        }
        let block = &nif.blocks[block_id];
        if block.type_name == "NiSkinPartition" {
            found.push(block_id);
        }
        pending.extend(
            conversion_block_refs(block)
                .into_iter()
                .filter_map(|reference| usize::try_from(reference).ok()),
        );
    }
    found
}

fn remap_skin_vertex_maps(value: &mut NifValue, vertex_remap: &[usize]) {
    match value {
        NifValue::Struct(fields) => {
            for (field, value) in fields {
                if field.split(':').next() == Some("Vertex Map") {
                    if let NifValue::Array(indices) = value {
                        for index in indices {
                            remap_numeric_vertex_index(index, vertex_remap);
                        }
                    }
                } else {
                    remap_skin_vertex_maps(value, vertex_remap);
                }
            }
        }
        NifValue::Array(values) => {
            for value in values {
                remap_skin_vertex_maps(value, vertex_remap);
            }
        }
        _ => {}
    }
}

fn normalize_fo76_vertex_color_shader_flags(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let counts = shader_vertex_color_counts(nif);
    let mut added = 0usize;
    let mut removed = 0usize;
    for (shader_id, (total, with_colors)) in counts {
        let Some(shader) = nif.blocks.get_mut(shader_id) else {
            continue;
        };
        let old_flags2 = value_u64(shader.get_field("Shader Flags 2")).unwrap_or(0);
        let new_flags2 = if with_colors > 0 {
            old_flags2 | u64::from(SLSF2_VERTEX_COLORS)
        } else {
            old_flags2 & !u64::from(SLSF2_VERTEX_COLORS)
        };
        if new_flags2 != old_flags2 {
            shader.set_field("Shader Flags 2", NifValue::UInt(new_flags2));
            if shader.fields.contains_key("Shader Flags 2:FO4") {
                shader
                    .fields
                    .insert("Shader Flags 2:FO4".to_string(), NifValue::UInt(new_flags2));
            }
            if new_flags2 & u64::from(SLSF2_VERTEX_COLORS) != 0 {
                added += 1;
            } else {
                removed += 1;
            }
        }
        if with_colors == 0 {
            let old_flags1 = value_u64(shader.get_field("Shader Flags 1")).unwrap_or(0);
            let new_flags1 = old_flags1 & !SLSF1_VERTEX_ALPHA;
            if new_flags1 != old_flags1 {
                shader.set_field("Shader Flags 1", NifValue::UInt(new_flags1));
                if shader.fields.contains_key("Shader Flags 1:FO4") {
                    shader
                        .fields
                        .insert("Shader Flags 1:FO4".to_string(), NifValue::UInt(new_flags1));
                }
            }
        }
        if with_colors > 0 && with_colors < total {
            report.warnings.push(format!(
                "Shader block {shader_id} is shared by FO4 geometry with and without vertex colors; Vertex_Colors was enabled"
            ));
        }
    }
    if added + removed > 0 {
        report.changes.push(format!(
            "Vertex-color shader contract: enabled {added}, disabled {removed} shader flag(s)"
        ));
    }
}

fn normalize_fo76_bsx_contract(nif: &mut NifFile, report: &mut ConvertFileReport) {
    // Switch-node flora authors External Emit only on BSX, so the shader graph
    // cannot reconstruct that runtime intent after the flag is stripped.
    let has_authored_external_emit = nif.blocks.iter().any(|block| {
        block.type_name == "BSXFlags"
            && value_u64(block.get_field("Integer Data")).unwrap_or(0) & BSX_EXTERNAL_EMIT_FLAG != 0
    });
    let has_bounds = nif
        .blocks
        .iter()
        .any(|block| crate::schema::SCHEMA.is_subtype_of(&block.type_name, "BSBound"));
    let has_controllers = nif
        .blocks
        .iter()
        .any(|block| crate::schema::SCHEMA.is_subtype_of(&block.type_name, "NiTimeController"));
    let has_addons = nif
        .blocks
        .iter()
        .any(|block| block.type_name == "BSValueNode");
    // FO4/FO76 express a ragdoll as one `bhkRagdollSystem` holding an embedded
    // Havok blob, not as the `bhkConstraint` blocks Skyrim-era NIFs use — 0 of
    // 29 vanilla FO4 actor skeletons carry a `bhkConstraint`, so the legacy
    // clauses alone can never fire on an FO4-format skeleton.
    let has_ragdoll = nif.blocks.iter().any(|block| {
        block.type_name == "bhkRagdollSystem"
            || crate::schema::SCHEMA.is_subtype_of(&block.type_name, "bhkConstraint")
            || crate::schema::SCHEMA.is_subtype_of(&block.type_name, "bhkBallSocketConstraintChain")
    });
    let has_marker = nif.blocks.iter().any(|block| {
        string_field(block, "Name")
            .is_some_and(|name| name.to_ascii_lowercase().contains("editormarker"))
    });
    let has_external_emittance = nif.blocks.iter().any(|block| {
        crate::schema::SCHEMA.is_subtype_of(&block.type_name, "BSShaderProperty")
            && value_u64(block.get_field("Shader Flags 1")).unwrap_or(0) & SLSF1_EXTERNAL_EMITTANCE
                != 0
    });

    let mut structural = 0u64;
    if (has_controllers || has_addons) && !has_bounds {
        structural |= BSX_ANIMATED_FLAG;
    }
    if has_live_collision_object(nif) {
        structural |= BSX_HAVOK_FLAG;
    }
    if has_ragdoll {
        structural |= BSX_RAGDOLL_FLAG;
    }
    if has_addons {
        structural |= BSX_ADDON_FLAG;
    }
    if has_marker {
        structural |= BSX_EDITOR_MARKER_FLAG;
    }
    if has_authored_external_emit || has_external_emittance {
        structural |= BSX_EXTERNAL_EMIT_FLAG;
    }

    // Ragdoll is added structurally but never cleared: vanilla FO4 ships actor
    // skeletons carrying the flag with no ragdoll blocks at all (`robot`,
    // `createabot`), so an authored bit is intent we cannot re-derive.
    let managed = BSX_ANIMATED_FLAG
        | BSX_HAVOK_FLAG
        | BSX_ADDON_FLAG
        | BSX_EDITOR_MARKER_FLAG
        | BSX_EXTERNAL_EMIT_FLAG;
    let bsx_ids = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSXFlags")
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    if bsx_ids.is_empty() {
        if structural != 0 {
            let root_id = nif
                .header
                .footer_roots
                .iter()
                .copied()
                .find(|root| *root >= 0)
                .map(|root| root as usize)
                .unwrap_or(0);
            ensure_root_bsx_flags(nif, root_id, structural);
            report.changes.push(format!(
                "BSX target contract: added missing flags {structural}"
            ));
        }
        return;
    }

    let mut remove = HashSet::new();
    let mut changes = Vec::new();
    for block_id in bsx_ids {
        let block = &mut nif.blocks[block_id];
        let current = value_u64(block.get_field("Integer Data")).unwrap_or(0);
        let desired = (current & !managed) | structural;
        if desired == 0 {
            remove.insert(block_id);
            changes.push(format!("{current}->removed"));
            continue;
        }
        block.set_field("Name", NifValue::String("BSX".to_string()));
        if desired != current {
            block.set_field("Integer Data", NifValue::UInt(desired));
            changes.push(format!("{current}->{desired}"));
        }
    }
    detach_extra_data_refs(nif, &remove);
    remove_blocks(nif, remove);
    if !changes.is_empty() {
        report.changes.push(format!(
            "BSX target contract: normalized {}",
            changes.join(", ")
        ));
    }
}

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
    preserve_fo76_static_root_flag: bool,
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
    let melee_root = matches!(weapon_role, Some("melee"))
        .then(|| roots.first().copied())
        .flatten();
    let mut converted = 0usize;
    for root_id in roots {
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
                if let Some(flags) = value_u64(root.get_field("Flags"))
                    && flags & NIF_NODE_PRESERVE_HIGH_FLAG_COMPANION != 0
                    && flags & NIF_NODE_EDITOR_MARKER_FLAG == 0
                {
                    root.set_field(
                        "Flags",
                        NifValue::UInt(flags & !NIF_NODE_PRESERVE_HIGH_FLAG_COMPANION),
                    );
                    converted += 1;
                }
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
                && !preserve_fo76_static_root_flag
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
    }
    if let Some(root_id) = melee_root
        && ensure_root_weapon_marker(nif, root_id)
    {
        converted += 1;
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

fn normalize_legacy_fo4_av_flags(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut normalized = 0usize;
    for block in &mut nif.blocks {
        if !crate::schema::SCHEMA.is_subtype_of(&block.type_name, "NiAVObject") {
            continue;
        }
        let Some(flags) = value_u64(block.get_field("Flags")) else {
            continue;
        };
        if flags & NIF_NODE_PRESERVE_HIGH_FLAG_COMPANION == 0 {
            continue;
        }
        block.set_field(
            "Flags",
            NifValue::UInt(flags & !NIF_NODE_PRESERVE_HIGH_FLAG_COMPANION),
        );
        normalized += 1;
    }
    if normalized > 0 {
        report.changes.push(format!(
            "Removed pre-FO4 0x80000 flag from {normalized} AV object(s)"
        ));
    }
}

fn ensure_root_weapon_marker(nif: &mut NifFile, root_id: usize) -> bool {
    let original_extra_ids = nif
        .get_block(root_id)
        .and_then(|root| root.get_field("Extra Data List"))
        .map(|value| ref_array(Some(value)))
        .unwrap_or_default();
    let candidate_ids = original_extra_ids
        .iter()
        .filter_map(|id| usize::try_from(*id).ok())
        .filter(|id| nif.get_block(*id).is_some_and(is_weapon_marker_candidate))
        .collect::<Vec<_>>();
    let marker_id = candidate_ids
        .iter()
        .copied()
        .find(|id| extra_data_parent_ids(nif, *id) == [root_id])
        .unwrap_or_else(|| nif.add_block("NiStringExtraData", None));
    let mut changed = candidate_ids.as_slice() != [marker_id];
    if let Some(marker) = nif.blocks.get_mut(marker_id) {
        if string_field(marker, "Name").unwrap_or_default() != "Prn" {
            marker.set_field("Name", NifValue::String("Prn".to_string()));
            changed = true;
        }
        if string_field(marker, "String Data").unwrap_or_default() != "WEAPON" {
            marker.set_field("String Data", NifValue::String("WEAPON".to_string()));
            changed = true;
        }
    }

    let candidate_set = candidate_ids.into_iter().collect::<HashSet<_>>();
    let mut extra_ids = Vec::with_capacity(original_extra_ids.len() + 1);
    for extra_id in original_extra_ids.iter().copied() {
        let Some(id) = usize::try_from(extra_id).ok() else {
            continue;
        };
        if candidate_set.contains(&id) {
            if id == marker_id && !extra_ids.contains(&extra_id) {
                extra_ids.push(extra_id);
            }
        } else if !extra_ids.contains(&extra_id) {
            extra_ids.push(extra_id);
        }
    }
    if !extra_ids.contains(&(marker_id as i32)) {
        extra_ids.push(marker_id as i32);
    }
    changed |= original_extra_ids != extra_ids;
    if let Some(root) = nif.blocks.get_mut(root_id) {
        root.set_field(
            "Extra Data List",
            NifValue::Array(extra_ids.iter().copied().map(NifValue::Ref).collect()),
        );
        root.set_field(
            "Num Extra Data List",
            NifValue::UInt(extra_ids.len() as u64),
        );
    }
    changed
}

fn is_weapon_marker_candidate(block: &NifBlock) -> bool {
    block.type_name == "NiStringExtraData"
        && matches!(
            string_field(block, "Name")
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
            "prn" | "weapon"
        )
}

fn extra_data_parent_ids(nif: &NifFile, extra_id: usize) -> Vec<usize> {
    nif.blocks
        .iter()
        .filter(|block| {
            block
                .get_field("Extra Data List")
                .map(|value| ref_array(Some(value)))
                .is_some_and(|ids| ids.contains(&(extra_id as i32)))
        })
        .map(|block| block.block_id)
        .collect()
}

fn required_legacy_glow_texture_paths(
    nif: &NifFile,
    source_game: &str,
    target_game: &str,
) -> HashSet<String> {
    if !matches!(source_game, "fnv" | "fo3") || target_game != "fo4" {
        return HashSet::new();
    }
    nif.blocks
        .iter()
        .filter(|shader| shader.type_name == "BSShaderPPLightingProperty")
        .filter(|shader| {
            value_u64(shader.get_field("Shader Type")) == Some(BSLSP_SHADER_TYPE_GLOW)
                || string_field(shader, "Shader Type")
                    .is_some_and(|value| value.to_ascii_lowercase().contains("glow"))
                || flag_names_to_bits(shader.get_field("Shader Flags 2"), false) & SLSF2_GLOW_MAP
                    != 0
        })
        .filter_map(|shader| field_ref(shader, "Texture Set"))
        .filter(|id| *id >= 0)
        .filter_map(|id| nif.get_block(id as usize))
        .filter_map(|texture_set| {
            value_array(texture_set.get_field("Textures"))
                .get(2)
                .cloned()
        })
        .filter_map(|value| match value {
            NifValue::String(path) if !path.trim_end_matches('\0').trim().is_empty() => {
                Some(canonical_texture_path(&path, "", "").to_ascii_lowercase())
            }
            _ => None,
        })
        .collect()
}

fn close_legacy_melee_texture_gaps(
    nif: &mut NifFile,
    source_nif: &Path,
    target_nif: &Path,
    bgsm_output_dir: Option<&Path>,
    required_glow_textures: &HashSet<String>,
    report: &mut ConvertFileReport,
) -> Result<Vec<PlannedTextureEmission>, std::io::Error> {
    let Some(source_data_root) = asset_data_root(source_nif, "meshes") else {
        report.warnings.push(format!(
            "Legacy melee textures: could not locate source Data root for {}",
            source_nif.display()
        ));
        return Ok(Vec::new());
    };
    let target_data_root = bgsm_output_dir
        .and_then(|path| asset_data_root(path, "materials"))
        .or_else(|| asset_data_root(target_nif, "meshes"));
    let mut cleared_glow_texture_sets = HashSet::new();
    let mut emissions = Vec::new();
    let mut planned_targets = HashSet::new();

    for texture_set in nif
        .blocks
        .iter_mut()
        .filter(|block| block.type_name == "BSShaderTextureSet")
    {
        let mut textures = value_array(texture_set.get_field("Textures"));
        textures.resize(FO4_TEXTURE_SLOT_COUNT, NifValue::String(String::new()));
        let mut changed = false;

        if let Some(NifValue::String(glow_path)) = textures.get(2) {
            let source_key = canonical_texture_path(glow_path, "", "");
            let source_path = asset_path(&source_data_root, &source_key);
            if source_key.to_ascii_lowercase().ends_with("_g.dds")
                && source_path.as_ref().is_some_and(|path| !path.is_file())
                && !required_glow_textures.contains(&source_key.to_ascii_lowercase())
            {
                textures[2] = NifValue::String(String::new());
                cleared_glow_texture_sets.insert(texture_set.block_id);
                changed = true;
                report.changes.push(format!(
                    "Legacy melee textures: cleared optional missing glow {source_key}"
                ));
            }
        }

        if let Some(NifValue::String(normal_path)) = textures.get(1) {
            let source_key = canonical_texture_path(normal_path, "", "");
            let source_path = asset_path(&source_data_root, &source_key);
            if source_key.to_ascii_lowercase().ends_with("_n.dds")
                && source_path.as_ref().is_some_and(|path| !path.is_file())
            {
                let Some(target_data_root) = target_data_root.as_ref() else {
                    report.warnings.push(format!(
                        "Legacy melee textures: cannot emit missing normal fallback without a target Data root: {normal_path}"
                    ));
                    continue;
                };
                let Some(target_path) = asset_path(target_data_root, normal_path) else {
                    report.warnings.push(format!(
                        "Legacy melee textures: rejected unsafe target normal path {normal_path}"
                    ));
                    continue;
                };
                if target_path.exists() && !target_path.is_file() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "legacy melee normal target is not a file: {}",
                            target_path.display()
                        ),
                    ));
                }
                if !target_path.is_file() && planned_targets.insert(target_path.clone()) {
                    emissions.push(PlannedTextureEmission {
                        target_path,
                        target_relative_path: normal_path.clone(),
                    });
                }
            }
        }

        if changed {
            texture_set.set_field("Textures", NifValue::Array(textures));
        }
    }

    if !cleared_glow_texture_sets.is_empty() {
        for shader in nif
            .blocks
            .iter_mut()
            .filter(|block| block.type_name == "BSLightingShaderProperty")
        {
            let Some(texture_set_id) = field_ref(shader, "Texture Set").filter(|id| *id >= 0)
            else {
                continue;
            };
            if !cleared_glow_texture_sets.contains(&(texture_set_id as usize)) {
                continue;
            }
            clear_shader_flag(shader, "Shader Flags 2", SLSF2_GLOW_MAP);
            clear_shader_flag(shader, "Shader Flags 2:FO4", SLSF2_GLOW_MAP);
            if value_u64(shader.get_field("Shader Type")) == Some(BSLSP_SHADER_TYPE_GLOW) {
                shader.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_DEFAULT));
            }
        }
    }

    Ok(emissions)
}

fn asset_data_root(path: &Path, asset_directory: &str) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| {
            ancestor
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(asset_directory))
        })
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn asset_path(data_root: &Path, asset_path: &str) -> Option<PathBuf> {
    let normalized = asset_path.trim_end_matches('\0').trim().replace('/', "\\");
    if normalized.is_empty()
        || Path::new(&normalized).is_absolute()
        || normalized
            .split('\\')
            .any(|component| component.is_empty() || component == ".." || component == ".")
    {
        return None;
    }
    let mut parts = normalized.split('\\');
    let root = parts.next()?;
    if !matches!(
        root.to_ascii_lowercase().as_str(),
        "textures" | "materials" | "meshes"
    ) {
        return None;
    }
    let mut output = data_root.join(root);
    for part in parts {
        output.push(part);
    }
    Some(output)
}

fn emit_planned_textures(
    emissions: &[PlannedTextureEmission],
    report: &mut ConvertFileReport,
) -> Result<(), std::io::Error> {
    static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let bytes = flat_fo4_normal_dds();
    for emission in emissions {
        if let Some(parent) = emission.target_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let sequence = TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let extension = emission
            .target_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("dds");
        let temporary = emission.target_path.with_extension(format!(
            "{extension}.tmp.{}.{}",
            std::process::id(),
            sequence
        ));
        let write_result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            use std::io::Write;
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        if let Err(error) = std::fs::hard_link(&temporary, &emission.target_path) {
            let _ = std::fs::remove_file(&temporary);
            if !emission.target_path.is_file() {
                return Err(error);
            }
        } else {
            std::fs::remove_file(&temporary)?;
        }
        report.changes.push(format!(
            "Legacy melee textures: emitted deterministic flat FO4 BC5 normal {}",
            emission.target_relative_path
        ));
        report
            .emitted_textures
            .push(emission.target_path.to_string_lossy().into_owned());
    }
    Ok(())
}

fn flat_fo4_normal_dds() -> Vec<u8> {
    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    let mut bytes = Vec::with_capacity(176);
    bytes.extend_from_slice(b"DDS ");
    push_u32(&mut bytes, 124);
    push_u32(&mut bytes, 0x000a_1007);
    push_u32(&mut bytes, 4);
    push_u32(&mut bytes, 4);
    push_u32(&mut bytes, 16);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 3);
    for _ in 0..11 {
        push_u32(&mut bytes, 0);
    }
    push_u32(&mut bytes, 32);
    push_u32(&mut bytes, 0x0000_0004);
    bytes.extend_from_slice(b"ATI2");
    for _ in 0..5 {
        push_u32(&mut bytes, 0);
    }
    push_u32(&mut bytes, 0x0040_1008);
    for _ in 0..4 {
        push_u32(&mut bytes, 0);
    }
    let flat_channel = [128, 128, 0, 0, 0, 0, 0, 0];
    for _ in 0..3 {
        bytes.extend_from_slice(&flat_channel);
        bytes.extend_from_slice(&flat_channel);
    }
    bytes
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
    let mut invalid = 0usize;
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
            if path.chars().any(char::is_control)
                || Path::new(path).is_absolute()
                || path.split(['/', '\\']).any(|component| component == "..")
                || !path
                    .trim_end_matches('\0')
                    .to_ascii_lowercase()
                    .ends_with(".dds")
            {
                *path = String::new();
                changed = true;
                invalid += 1;
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
    if invalid > 0 {
        report.warnings.push(format!(
            "BSShaderTextureSet: cleared {invalid} invalid source texture path(s)"
        ));
    }
}

fn normalize_fo76_inline_texture_paths(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut changed = 0usize;
    for block in &mut nif.blocks {
        let fields: &[&str] = match block.type_name.as_str() {
            "BSEffectShaderProperty" => &[
                "Source Texture",
                "Grayscale Texture",
                "Greyscale Texture",
                "Env Map Texture",
                "Normal Texture",
                "Env Mask Texture",
            ],
            "BSSkyShaderProperty" => &["Source Texture"],
            "BSShaderNoLightingProperty" | "TallGrassShaderProperty" | "TileShaderProperty" => {
                &["File Name"]
            }
            _ => &[],
        };
        for field in fields {
            let Some(NifValue::String(path)) = block.get_field(field).cloned() else {
                continue;
            };
            if path.trim_end_matches('\0').trim().is_empty() {
                continue;
            }
            let normalized = canonical_texture_path(&path, "fo76", "fo4");
            if normalized != path {
                block.set_field(field, NifValue::String(normalized));
                changed += 1;
            }
        }
    }
    if changed > 0 {
        report.changes.push(format!(
            "Inline shader textures: normalized {changed} FO76 texture path(s) for FO4"
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
    let lightning_refs = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSProceduralLightningController")
        .map(|block| {
            let refs = (1..=9)
                .filter_map(|index| {
                    let field = format!("Interpolator {index}");
                    field_ref(block, &field).map(|reference| (field, reference))
                })
                .collect::<Vec<_>>();
            (block.block_id, refs)
        })
        .collect::<Vec<_>>();
    let old_len = nif.blocks.len();
    let removed = ids.iter().copied().collect::<HashSet<_>>();
    let remap = (0..old_len)
        .map(|old_id| {
            let new_id = if removed.contains(&old_id) {
                -1
            } else {
                (old_id - ids.partition_point(|removed_id| *removed_id < old_id)) as i32
            };
            (old_id as i32, new_id)
        })
        .collect::<HashMap<_, _>>();
    nif.remove_blocks(&ids);
    for (old_controller, refs) in lightning_refs {
        let Some(new_controller) = remap
            .get(&(old_controller as i32))
            .copied()
            .filter(|controller| *controller >= 0)
            .map(|controller| controller as usize)
        else {
            continue;
        };
        let Some(block) = nif.blocks.get_mut(new_controller) else {
            continue;
        };
        for (field, old_reference) in refs {
            let new_reference = if old_reference < 0 {
                old_reference
            } else {
                remap.get(&old_reference).copied().unwrap_or(-1)
            };
            block.set_field(&field, NifValue::Ref(new_reference));
        }
    }
}

fn replace_block_ref(nif: &mut NifFile, old_ref: i32, new_ref: i32) {
    for block in nif.blocks.iter_mut() {
        for value in block.fields.values_mut() {
            replace_value_ref(value, old_ref, new_ref);
        }
        if block.type_name == "NiDefaultAVObjectPalette"
            && let Some(NifValue::Array(entries)) = block.get_field("Objs").cloned()
        {
            let entries = entries
                .into_iter()
                .map(|entry| match entry {
                    NifValue::Struct(mut fields)
                        if value_ref(fields.get("AV Object")) == Some(old_ref) =>
                    {
                        fields.insert("AV Object".to_string(), NifValue::Ref(new_ref));
                        NifValue::Struct(fields)
                    }
                    entry => entry,
                })
                .collect();
            block.set_field("Objs", NifValue::Array(entries));
        }
    }
}

fn replace_value_ref(value: &mut NifValue, old_ref: i32, new_ref: i32) {
    match value {
        NifValue::Ref(reference) if *reference == old_ref => *reference = new_ref,
        NifValue::Array(values) => {
            for value in values {
                replace_value_ref(value, old_ref, new_ref);
            }
        }
        NifValue::Struct(fields) => {
            for value in fields.values_mut() {
                replace_value_ref(value, old_ref, new_ref);
            }
        }
        _ => {}
    }
}

fn prune_legacy_material_controller_links(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut removed = 0usize;
    for sequence in nif
        .blocks
        .iter_mut()
        .filter(|block| block.type_name == "NiControllerSequence")
    {
        let Some(NifValue::Array(controlled_blocks)) =
            sequence.get_field("Controlled Blocks").cloned()
        else {
            continue;
        };
        let retained = controlled_blocks
            .into_iter()
            .filter(|controlled_block| {
                let NifValue::Struct(fields) = controlled_block else {
                    return true;
                };
                let property_type = fields
                    .get("Property Type")
                    .and_then(nif_value_string)
                    .unwrap_or_default();
                let controller_type = fields
                    .get("Controller Type")
                    .and_then(nif_value_string)
                    .unwrap_or_default();
                let incompatible =
                    matches!(property_type, "NiMaterialProperty" | "NiTexturingProperty")
                        || matches!(
                            controller_type,
                            "NiAlphaController"
                                | "NiMaterialColorController"
                                | "NiTextureTransformController"
                        );
                if incompatible {
                    removed += 1;
                }
                !incompatible
            })
            .collect::<Vec<_>>();
        sequence.set_field(
            "Num Controlled Blocks",
            NifValue::UInt(retained.len() as u64),
        );
        sequence.set_field("Controlled Blocks", NifValue::Array(retained));
    }
    if removed > 0 {
        report.changes.push(format!(
            "Legacy controllers: removed {removed} sequence link(s) targeting discarded material properties"
        ));
    }
}

fn nif_value_string(value: &NifValue) -> Option<&str> {
    match value {
        NifValue::String(value) => Some(value),
        _ => None,
    }
}

fn prune_unreachable_legacy_blocks(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let mut reachable = HashSet::new();
    let mut pending = nif
        .header
        .footer_roots
        .iter()
        .copied()
        .filter(|root| *root >= 0)
        .map(|root| root as usize)
        .collect::<Vec<_>>();
    if pending.is_empty() && !nif.blocks.is_empty() {
        pending.push(0);
    }
    while let Some(block_id) = pending.pop() {
        if block_id >= nif.blocks.len() || !reachable.insert(block_id) {
            continue;
        }
        let mut references = Vec::new();
        for value in nif.blocks[block_id].fields.values() {
            collect_value_refs(value, &mut references);
        }
        collect_legacy_nested_refs(&nif.blocks[block_id], &mut references);
        pending.extend(references);
    }

    let remove = nif
        .blocks
        .iter()
        .filter(|block| !reachable.contains(&block.block_id))
        .map(|block| block.block_id)
        .collect::<HashSet<_>>();
    if remove.is_empty() {
        return;
    }
    let removed = remove.len();
    remove_blocks(nif, remove);
    report.changes.push(format!(
        "Legacy cleanup: pruned {removed} unreachable property/controller block(s)"
    ));
}

fn detach_legacy_decal_placement_vector_nodes(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let decal_nodes = nif
        .blocks
        .iter()
        .filter(|block| {
            is_node_type(&block.type_name)
                && string_field(block, "Name")
                    .is_some_and(|name| name.starts_with("DecalPlacementVector"))
        })
        .map(|block| block.block_id as i32)
        .collect::<HashSet<_>>();
    if decal_nodes.is_empty() {
        return;
    }

    let mut detached = 0usize;
    for block in nif.blocks.iter_mut() {
        let Some(NifValue::Array(children)) = block.get_field("Children").cloned() else {
            continue;
        };
        let retained = children
            .into_iter()
            .filter(|child| {
                let remove = value_ref(Some(child)).is_some_and(|id| decal_nodes.contains(&id));
                detached += usize::from(remove);
                !remove
            })
            .collect::<Vec<_>>();
        block.set_field("Num Children", NifValue::UInt(retained.len() as u64));
        block.set_field("Children", NifValue::Array(retained));
    }
    if detached > 0 {
        report.changes.push(format!(
            "Legacy decals: detached {detached} placement-vector node reference(s)"
        ));
    }
}

/// Legacy Gamebryo block types the FO4 runtime has no RTTI entry for.
///
/// Every name here was measured absent from the shipped Fallout4.exe. FO4
/// resolves blocks by RTTI name, so a single survivor fails the *whole* file
/// load and the engine draws the red "!" marker instead of the mesh. These are
/// detached from their referrers and then garbage-collected by
/// [`prune_unreachable_legacy_blocks`].
///
/// This is a reviewed list rather than "everything [`is_fo4_block_type`]
/// rejects" on purpose: stripping is destructive, so a list harvested from a
/// binary must never drive it. The harvested list only drives the audit.
static LEGACY_BLOCK_TYPES_WITHOUT_FO4_EQUIVALENT: &[&str] = &[
    // Controllers with no FO4 counterpart. The geometry survives; only the
    // FNV/FO3-specific animated effect is lost.
    "BSRefractionFirePeriodController",
    "BSRefractionStrengthController",
    "NiBSBoneLODController",
    "NiGeomMorpherController",
    // FO4's BSLightingShaderPropertyFloatController can animate UV offset/scale,
    // so a mapping is conceivable -- but the FO4 "Controlled Variable" enum
    // values for U/V offset/scale are not pinned down anywhere in this repo, and
    // guessing wrong animates an unrelated channel (glossiness, emissive) which
    // is far harder to spot than a missing UV scroll. Dropped until the enum is
    // confirmed against a vanilla FO4 NIF that animates UVs.
    "NiTextureTransformController",
    "bhkBlendController",
    // Fixed-function properties superseded by BSLightingShaderProperty, which
    // `legacy_shader_to_lighting` has already synthesized by this point.
    "Lighting30ShaderProperty",
    "NiSourceTexture",
    "NiStencilProperty",
    "NiTexturingProperty",
    // FO3/FNV decal placement; FO4 drives decals from the record side.
    "BSDecalPlacementVectorExtraData",
    // Legacy Havok. Anything `regenerate_fo4_collision` can rebuild is already
    // gone by the time this runs, so these are the chains it does not handle.
    // Phantoms are trigger volumes -- rebuilding them as real FO4 collision
    // would turn every trigger into an invisible wall, so they are dropped.
    "bhkAabbPhantom",
    "bhkBlendCollisionObject",
    "bhkBoxShape",
    "bhkCapsuleShape",
    "bhkConvexTransformShape",
    "bhkConvexVerticesShape",
    "bhkLimitedHingeConstraint",
    "bhkListShape",
    "bhkMalleableConstraint",
    "bhkMoppBvTreeShape",
    "bhkPCollisionObject",
    "bhkPackedNiTriStripsShape",
    "bhkRagdollConstraint",
    "bhkRigidBody",
    "bhkRigidBodyT",
    "bhkSPCollisionObject",
    "bhkSimpleShapePhantom",
    "bhkSphereShape",
    "bhkTransformShape",
    "hkPackedNiTriStripsData",
];

/// Ref-bearing arrays that carry a paired count field which must stay in sync.
const COUNTED_REF_ARRAYS: &[(&str, &str)] = &[
    ("Children", "Num Children"),
    ("Controlled Blocks", "Num Controlled Blocks"),
    ("Effects", "Num Effects"),
    ("Extra Data List", "Num Extra Data List"),
    ("Objs", "Num Objs"),
    ("Properties", "Num Properties"),
];

fn detach_legacy_blocks_without_fo4_equivalent(nif: &mut NifFile, report: &mut ConvertFileReport) {
    let doomed: HashSet<i32> = nif
        .blocks
        .iter()
        .filter(|block| {
            LEGACY_BLOCK_TYPES_WITHOUT_FO4_EQUIVALENT.contains(&block.type_name.as_str())
        })
        .map(|block| block.block_id as i32)
        .collect();
    if doomed.is_empty() {
        return;
    }

    let mut dropped: BTreeMap<String, usize> = BTreeMap::new();
    for block in &nif.blocks {
        if doomed.contains(&(block.block_id as i32)) {
            *dropped.entry(block.type_name.clone()).or_default() += 1;
        }
    }

    for block in nif.blocks.iter_mut() {
        if doomed.contains(&(block.block_id as i32)) {
            continue;
        }
        for (array_field, count_field) in COUNTED_REF_ARRAYS {
            let Some(NifValue::Array(items)) = block.get_field(array_field).cloned() else {
                continue;
            };
            let original = items.len();
            // An entry goes if any ref inside it is doomed: plain child refs are
            // bare Refs, but a controlled block is a struct whose Interpolator /
            // Controller refs are what point at the dead controller.
            let retained: Vec<NifValue> = items
                .into_iter()
                .filter(|item| {
                    let mut refs = Vec::new();
                    collect_value_refs(item, &mut refs);
                    !refs.iter().any(|id| doomed.contains(&(*id as i32)))
                })
                .collect();
            if retained.len() == original {
                continue;
            }
            block.set_field(count_field, NifValue::UInt(retained.len() as u64));
            block.set_field(array_field, NifValue::Array(retained));
        }
        for value in block.fields.values_mut() {
            null_doomed_refs(value, &doomed);
        }
    }

    let detail = dropped
        .iter()
        .map(|(name, count)| format!("{name} x{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    report.changes.push(format!(
        "Legacy blocks without an FO4 equivalent: detached {detail}"
    ));
}

fn null_doomed_refs(value: &mut NifValue, doomed: &HashSet<i32>) {
    match value {
        NifValue::Ref(id) if doomed.contains(id) => *id = -1,
        NifValue::Array(values) => {
            for value in values {
                null_doomed_refs(value, doomed);
            }
        }
        NifValue::Struct(fields) => {
            for value in fields.values_mut() {
                null_doomed_refs(value, doomed);
            }
        }
        _ => {}
    }
}

/// Fail the file rather than shipping a NIF the FO4 runtime cannot load.
///
/// Without this the conversion phase reports `failed=0` while writing meshes
/// that render as the red "!" marker -- a silent failure with no log line.
fn audit_fo4_block_types(nif: &NifFile, report: &mut ConvertFileReport) {
    let mut unsupported: BTreeMap<&str, usize> = BTreeMap::new();
    for block in &nif.blocks {
        if !crate::fo4_block_types::is_fo4_block_type(&block.type_name) {
            *unsupported.entry(block.type_name.as_str()).or_default() += 1;
        }
    }
    if unsupported.is_empty() {
        return;
    }
    let detail = unsupported
        .iter()
        .map(|(name, count)| format!("{name} x{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    report.errors.push(format!(
        "block types absent from the FO4 runtime would fail the whole file load: {detail}"
    ));
}

fn collect_legacy_nested_refs(block: &NifBlock, references: &mut Vec<usize>) {
    let entries = match block.type_name.as_str() {
        "NiDefaultAVObjectPalette" => block.get_field("Objs"),
        "NiControllerSequence" => block.get_field("Controlled Blocks"),
        _ => None,
    };
    let Some(NifValue::Array(entries)) = entries else {
        return;
    };
    let field_names: &[&str] = if block.type_name == "NiDefaultAVObjectPalette" {
        &["AV Object"]
    } else {
        &["Interpolator", "Controller"]
    };
    for entry in entries {
        let NifValue::Struct(fields) = entry else {
            continue;
        };
        for field_name in field_names {
            if let Some(reference) = fields.get(*field_name).and_then(value_usize) {
                references.push(reference);
            }
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
        NifValue::Struct(fields) => Some(NifValue::Color4([
            value_f64(fields.get("r")).unwrap_or(1.0) as f32,
            value_f64(fields.get("g")).unwrap_or(1.0) as f32,
            value_f64(fields.get("b")).unwrap_or(1.0) as f32,
            value_f64(fields.get("a")).unwrap_or(1.0) as f32,
        ])),
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
    fn nif_header_probe_accepts_both_prefixes_and_short_non_nifs() {
        let temp = tempfile::tempdir().unwrap();
        for (name, bytes, expected) in [
            ("gamebryo", b"Gamebryo File Format trailing data".as_slice(), true),
            ("netimmerse", b"NetImmerse File Format trailing data".as_slice(), true),
            ("short", b"Gamebryo".as_slice(), false),
            ("other", b"not a nif".as_slice(), false),
        ] {
            let path = temp.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(has_nif_header(&path).unwrap(), expected, "{name}");
        }
        assert!(has_nif_header(&temp.path().join("missing")).is_err());
    }

    #[test]
    fn final_dependencies_match_written_outputs_for_every_fo4_source_pair() {
        let temp = tempfile::tempdir().unwrap();
        for game in ["fo76", "skyrimse", "fnv", "fo3"] {
            let mut nif = NifFile::new(game);
            let root = nif.add_block("NiNode", None);
            nif.blocks[root].set_field("Name", NifValue::String("root".into()));
            let source = temp.path().join(format!("{game}.nif"));
            let output = temp.path().join(format!("{game}-fo4.nif"));
            std::fs::write(&source, nif.to_bytes().unwrap()).unwrap();
            let report = convert_nif_file(&source, &output, game, "fo4", None, &ConvertFileOptions::default()).unwrap();
            assert!(report.supported, "{game}: {:?}", report.errors);
            let captured = report.final_dependencies.unwrap();
            let bytes = std::fs::read(&output).unwrap();
            let actual = NifFile::from_bytes(&bytes, Some(output)).unwrap();
            assert_eq!(captured.digest, *blake3::hash(&bytes).as_bytes());
            assert_eq!(captured.materials, actual.referenced_asset_paths().materials);
        }
    }

    fn test_vertex(uv: [f64; 2], normal: [f32; 3], color: [f32; 4], weights: [f64; 4]) -> NifValue {
        NifValue::Struct(IndexMap::from([
            ("Vertex".to_string(), NifValue::Vec3([1.0, 2.0, 3.0])),
            (
                "UV".to_string(),
                NifValue::Array(uv.into_iter().map(NifValue::Float).collect()),
            ),
            ("Normal".to_string(), NifValue::Vec3(normal)),
            ("Vertex Colors".to_string(), NifValue::Color4(color)),
            (
                "Bone Weights".to_string(),
                NifValue::Array(weights.into_iter().map(NifValue::Float).collect()),
            ),
            (
                "Bone Indices".to_string(),
                NifValue::Array([0, 1, 2, 3].into_iter().map(NifValue::UInt).collect()),
            ),
        ]))
    }

    fn test_triangle(v1: u64, v2: u64, v3: u64) -> NifValue {
        NifValue::Struct(IndexMap::from([
            ("v1".to_string(), NifValue::UInt(v1)),
            ("v2".to_string(), NifValue::UInt(v2)),
            ("v3".to_string(), NifValue::UInt(v3)),
        ]))
    }

    #[test]
    fn exact_vertex_dedup_remaps_triangles_and_preserves_semantic_seams() {
        let base = test_vertex(
            [0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 0.0],
        );
        let different_uv = test_vertex(
            [1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 0.0],
        );
        let different_normal = test_vertex(
            [0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 0.0],
        );
        let different_color = test_vertex(
            [0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.5, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 0.0],
        );
        let different_weights = test_vertex(
            [0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [0.5, 0.5, 0.0, 0.0],
        );
        let mut nif = NifFile::new("fo4");
        let shape_id = nif.add_block("BSTriShape", None);
        let vertices = vec![
            base.clone(),
            base,
            different_uv,
            different_normal,
            different_color,
            different_weights,
        ];
        nif.blocks[shape_id].set_field("Vertex Desc", NifValue::UInt(0x65));
        nif.blocks[shape_id].set_field("Num Vertices", NifValue::UInt(vertices.len() as u64));
        nif.blocks[shape_id].set_field("Vertex Data", NifValue::Array(vertices));
        nif.blocks[shape_id].set_field("Triangles", NifValue::Array(vec![test_triangle(0, 1, 5)]));
        nif.blocks[shape_id].set_field("Num Triangles", NifValue::UInt(1));
        nif.blocks[shape_id].set_field("Data Size", NifValue::UInt(0));

        let mut report = ConvertFileReport::default();
        deduplicate_fo76_exact_vertices(&mut nif, &mut report);

        assert_eq!(
            value_u64(nif.blocks[shape_id].get_field("Num Vertices")),
            Some(5)
        );
        assert_eq!(
            value_array(nif.blocks[shape_id].get_field("Vertex Data")).len(),
            5
        );
        assert_eq!(
            value_array(nif.blocks[shape_id].get_field("Triangles")),
            vec![test_triangle(0, 0, 4)]
        );
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("collapsed 1"))
        );
    }

    #[test]
    fn exact_vertex_dedup_remaps_skin_partitions() {
        let duplicate = test_vertex(
            [0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 0.0],
        );
        let unique = test_vertex(
            [1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 0.0],
        );
        let mut nif = NifFile::new("fo4");
        let shape_id = nif.add_block("BSTriShape", None);
        let skin_id = nif.add_block("NiSkinInstance", None);
        let partition_id = nif.add_block("NiSkinPartition", None);
        nif.blocks[shape_id].set_field("Skin", NifValue::Ref(skin_id as i32));
        nif.blocks[shape_id].set_field("Vertex Desc", NifValue::UInt(0x65));
        nif.blocks[shape_id].set_field("Num Vertices", NifValue::UInt(3));
        nif.blocks[shape_id].set_field(
            "Vertex Data",
            NifValue::Array(vec![duplicate.clone(), duplicate, unique]),
        );
        nif.blocks[shape_id]
            .set_field("Triangles", NifValue::Array(vec![test_triangle(0, 1, 2)]));
        nif.blocks[skin_id].set_field("Skin Partition", NifValue::Ref(partition_id as i32));
        nif.blocks[partition_id].set_field(
            "Partitions",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                "Vertex Map".to_string(),
                NifValue::Array(vec![
                    NifValue::UInt(0),
                    NifValue::UInt(1),
                    NifValue::UInt(2),
                ]),
            )]))]),
        );

        deduplicate_fo76_exact_vertices(&mut nif, &mut ConvertFileReport::default());

        let Some(NifValue::Array(partitions)) =
            nif.blocks[partition_id].get_field("Partitions")
        else {
            panic!("missing skin partitions");
        };
        let NifValue::Struct(partition) = &partitions[0] else {
            panic!("skin partition must be a struct");
        };
        assert_eq!(
            partition.get("Vertex Map"),
            Some(&NifValue::Array(vec![
                NifValue::UInt(0),
                NifValue::UInt(0),
                NifValue::UInt(1),
            ]))
        );
    }

    #[test]
    fn exact_vertex_dedup_keeps_particle_geometry_untouched() {
        let vertex = test_vertex(
            [0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 0.0],
        );
        let vertices = NifValue::Array(vec![vertex.clone(), vertex]);
        let mut nif = NifFile::new("fo4");
        let shape_id = nif.add_block("BSTriShape", None);
        nif.blocks[shape_id].set_field(
            "Particle Vertices",
            NifValue::Array(vec![NifValue::Vec3([0.0, 0.0, 0.0])]),
        );
        nif.blocks[shape_id].set_field("Vertex Data", vertices.clone());
        let mut report = ConvertFileReport::default();

        deduplicate_fo76_exact_vertices(&mut nif, &mut report);

        assert_eq!(nif.blocks[shape_id].get_field("Vertex Data"), Some(&vertices));
        assert!(report.changes.is_empty());
    }

    #[test]
    fn exact_vertex_dedup_keeps_same_hash_unequal_nan_payloads() {
        let nan_bits = 0x7ff8_0000_0000_0042;
        let first = NifValue::Float(f64::from_bits(nan_bits));
        let second = NifValue::Float(f64::from_bits(nan_bits));
        assert_eq!(conversion_value_hash(&first), conversion_value_hash(&second));
        assert_ne!(first, second);
        let mut nif = NifFile::new("fo4");
        let shape_id = nif.add_block("BSTriShape", None);
        nif.blocks[shape_id].set_field("Num Vertices", NifValue::UInt(2));
        nif.blocks[shape_id].set_field("Vertex Data", NifValue::Array(vec![first, second]));
        let mut report = ConvertFileReport::default();

        deduplicate_fo76_exact_vertices(&mut nif, &mut report);

        assert_eq!(
            value_u64(nif.blocks[shape_id].get_field("Num Vertices")),
            Some(2)
        );
        assert_eq!(
            value_array(nif.blocks[shape_id].get_field("Vertex Data")).len(),
            2
        );
        assert!(report.changes.is_empty());
    }

    #[test]
    fn exact_vertex_dedup_skips_shapes_that_share_a_skin() {
        let vertex = test_vertex(
            [0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 0.0],
        );
        let mut nif = NifFile::new("fo4");
        let skin_id = nif.add_block("NiSkinInstance", None);
        for _ in 0..2 {
            let shape_id = nif.add_block("BSTriShape", None);
            nif.blocks[shape_id].set_field("Skin", NifValue::Ref(skin_id as i32));
            nif.blocks[shape_id].set_field("Num Vertices", NifValue::UInt(2));
            nif.blocks[shape_id].set_field(
                "Vertex Data",
                NifValue::Array(vec![vertex.clone(), vertex.clone()]),
            );
        }
        let mut report = ConvertFileReport::default();

        deduplicate_fo76_exact_vertices(&mut nif, &mut report);

        for shape in nif
            .blocks
            .iter()
            .filter(|block| block.type_name == "BSTriShape")
        {
            assert_eq!(value_array(shape.get_field("Vertex Data")).len(), 2);
        }
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("skipped 2 shape(s) sharing a skin object"))
        );
    }

    #[test]
    fn vertex_colors_fall_back_to_borrowed_vertex_payload() {
        let mut shape = NifBlock::new(0, "BSTriShape");
        shape.set_field("Vertex Desc", NifValue::UInt(0));
        shape.set_field(
            "Vertex Data",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                "Vertex Colors".to_string(),
                NifValue::Color4([1.0; 4]),
            )]))]),
        );
        assert!(shape_has_vertex_colors(&shape));

        shape.set_field("Vertex Data", NifValue::UInt(1));
        assert!(!shape_has_vertex_colors(&shape));
    }

    #[test]
    fn animation_contract_without_manager_still_repairs_controller_target() {
        let mut nif = NifFile::new("fo4");
        let owner_id = nif.add_block("NiNode", None);
        let controller_id = nif.add_block("NiTimeController", None);
        nif.blocks[owner_id].set_field("Controller", NifValue::Ref(controller_id as i32));
        nif.blocks[controller_id].set_field("Target", NifValue::Ref(-1));
        nif.blocks[controller_id].set_field("Next Controller", NifValue::Ref(-1));
        let mut report = ConvertFileReport::default();

        normalize_fo76_animation_contract(&mut nif, &mut report);

        assert_eq!(
            field_ref(&nif.blocks[controller_id], "Target"),
            Some(owner_id as i32)
        );
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("targets=1"))
        );
    }

    #[test]
    fn procedural_lightning_refs_follow_conversion_block_removal() {
        let mut nif = NifFile::new("fo4");
        let dropped_id = nif.add_block("BSLightingShaderPropertyFloatController", None);
        let controller_id = nif.add_block("BSProceduralLightningController", None);
        let generation_id = nif.add_block("NiBlendBoolInterpolator", None);
        let arc_id = nif.add_block("NiBlendFloatInterpolator", None);
        nif.blocks[controller_id].set_field("Interpolator 1", NifValue::Ref(generation_id as i32));
        nif.blocks[controller_id].set_field("Interpolator 9", NifValue::Ref(arc_id as i32));

        remove_blocks(&mut nif, HashSet::from([dropped_id]));

        let controller = &nif.blocks[1];
        assert_eq!(field_ref(controller, "Interpolator 1"), Some(2));
        assert_eq!(field_ref(controller, "Interpolator 9"), Some(3));
        assert_eq!(nif.blocks[2].type_name, "NiBlendBoolInterpolator");
        assert_eq!(nif.blocks[3].type_name, "NiBlendFloatInterpolator");
    }

    #[test]
    fn float_controller_pruning_preserves_preexisting_detached_blocks() {
        let mut nif = NifFile::new("fo4");
        let root_id = nif.add_block("NiNode", None);
        let controller_id = nif.add_block("BSLightingShaderPropertyFloatController", None);
        let interpolator_id = nif.add_block("NiBlendFloatInterpolator", None);
        let detached_id = nif.add_block("NiStringExtraData", None);
        nif.header.footer_roots = vec![root_id as i32];
        nif.blocks[root_id].set_field("Controller", NifValue::Ref(controller_id as i32));
        nif.blocks[controller_id].set_field("Controlled Variable", NifValue::UInt(4));
        nif.blocks[controller_id].set_field("Interpolator", NifValue::Ref(interpolator_id as i32));
        nif.blocks[detached_id].set_field("Name", NifValue::String("detached".to_string()));

        let mut report = ConvertFileReport::default();
        fix_fo76_float_controllers(&mut nif, &mut report);

        assert!(nif.blocks.iter().all(|block| {
            block.type_name != "BSLightingShaderPropertyFloatController"
                && block.type_name != "NiBlendFloatInterpolator"
        }));
        assert!(nif.blocks.iter().any(|block| {
            block.type_name == "NiStringExtraData"
                && string_field(block, "Name").as_deref() == Some("detached")
        }));
    }

    #[test]
    fn inline_texture_paths_and_external_emit_bsx_are_fo4_normalized() {
        let mut nif = NifFile::new("fo4");
        let root_id = nif.add_block("NiNode", None);
        let bsx_id = nif.add_block("BSXFlags", None);
        let shader_id = nif.add_block("BSEffectShaderProperty", None);
        nif.header.footer_roots = vec![root_id as i32];
        nif.blocks[root_id].set_field(
            "Extra Data List",
            NifValue::Array(vec![NifValue::Ref(bsx_id as i32)]),
        );
        nif.blocks[bsx_id].set_field("Name", NifValue::String("BSX".to_string()));
        nif.blocks[bsx_id].set_field("Integer Data", NifValue::UInt(2049));
        nif.blocks[shader_id].set_field(
            "Name",
            NifValue::String("materials/effects/test.bgem".to_string()),
        );
        nif.blocks[shader_id].set_field(
            "Source Texture",
            NifValue::String("Shared/Black01_d.dds".to_string()),
        );
        nif.blocks[shader_id].set_field("Shader Flags 1", NifValue::UInt(SLSF1_EXTERNAL_EMITTANCE));
        nif.add_block("BSEffectShaderPropertyFloatController", None);

        let mut report = ConvertFileReport::default();
        normalize_fo76_inline_texture_paths(&mut nif, &mut report);
        normalize_fo76_bsx_contract(&mut nif, &mut report);

        assert_eq!(
            string_field(&nif.blocks[shader_id], "Source Texture").as_deref(),
            Some(r"textures\Shared\Black01_d.dds")
        );
        assert_eq!(
            string_field(&nif.blocks[shader_id], "Name").as_deref(),
            Some("materials/effects/test.bgem")
        );
        assert_eq!(
            value_u64(nif.blocks[bsx_id].get_field("Integer Data")),
            Some(2049 | BSX_EXTERNAL_EMIT_FLAG)
        );
    }

    #[test]
    fn removed_bsx_is_detached_from_extra_data_owner() {
        let mut nif = NifFile::new("fo4");
        let root_id = nif.add_block("NiNode", None);
        let bsx_id = nif.add_block("BSXFlags", None);
        nif.header.footer_roots = vec![root_id as i32];
        nif.blocks[root_id].set_field("Num Extra Data List", NifValue::UInt(1));
        nif.blocks[root_id].set_field(
            "Extra Data List",
            NifValue::Array(vec![NifValue::Ref(bsx_id as i32)]),
        );
        nif.blocks[bsx_id].set_field("Integer Data", NifValue::UInt(0));

        normalize_fo76_bsx_contract(&mut nif, &mut ConvertFileReport::default());

        assert!(nif.blocks.iter().all(|block| block.type_name != "BSXFlags"));
        assert_eq!(
            value_u64(nif.blocks[root_id].get_field("Num Extra Data List")),
            Some(0)
        );
        assert!(value_array(nif.blocks[root_id].get_field("Extra Data List")).is_empty());
    }

    #[test]
    fn vertex_color_flags_and_animation_metadata_are_target_derived() {
        let mut nif = NifFile::new("fo4");
        let root_id = nif.add_block("NiNode", None);
        let manager_id = nif.add_block("NiControllerManager", None);
        let multitarget_id = nif.add_block("NiMultiTargetTransformController", None);
        let sequence_id = nif.add_block("NiControllerSequence", None);
        let palette_id = nif.add_block("NiDefaultAVObjectPalette", None);
        let second_target_id = nif.add_block("NiNode", None);
        let first_target_id = nif.add_block("BSTriShape", None);
        let shader_id = nif.add_block("BSEffectShaderProperty", None);
        nif.header.footer_roots = vec![root_id as i32];
        nif.blocks[root_id].set_field("Name", NifValue::String("Root".to_string()));
        nif.blocks[root_id].set_field("Controller", NifValue::Ref(manager_id as i32));
        nif.blocks[root_id].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(second_target_id as i32),
                NifValue::Ref(first_target_id as i32),
            ]),
        );
        nif.blocks[manager_id].set_field("Target", NifValue::Ref(-1));
        nif.blocks[manager_id].set_field("Next Controller", NifValue::Ref(multitarget_id as i32));
        nif.blocks[manager_id].set_field(
            "Controller Sequences",
            NifValue::Array(vec![NifValue::Ref(sequence_id as i32)]),
        );
        nif.blocks[manager_id].set_field("Object Palette", NifValue::Ref(palette_id as i32));
        nif.blocks[second_target_id].set_field("Name", NifValue::String("Second".to_string()));
        nif.blocks[first_target_id].set_field("Name", NifValue::String("First".to_string()));
        nif.blocks[first_target_id].set_field("Shader Property", NifValue::Ref(shader_id as i32));
        nif.blocks[first_target_id].set_field(
            "Vertex Desc",
            NifValue::UInt((VF_VERTEX_COLORS as u64) << 44),
        );
        nif.blocks[shader_id].set_field("Shader Flags 2", NifValue::UInt(0));
        let controlled = |name: &str| {
            NifValue::Struct(IndexMap::from([(
                "Node Name".to_string(),
                NifValue::String(name.to_string()),
            )]))
        };
        nif.blocks[sequence_id].set_field(
            "Controlled Blocks",
            NifValue::Array(vec![
                controlled("First"),
                controlled("Second"),
                controlled("Missing"),
            ]),
        );
        nif.blocks[sequence_id].set_field("Accum Root Name", NifValue::String("Wrong".to_string()));

        let mut report = ConvertFileReport::default();
        normalize_fo76_animation_contract(&mut nif, &mut report);
        normalize_fo76_vertex_color_shader_flags(&mut nif, &mut report);

        assert_eq!(
            field_ref(&nif.blocks[manager_id], "Target"),
            Some(root_id as i32)
        );
        assert_eq!(
            string_field(&nif.blocks[sequence_id], "Accum Root Name").as_deref(),
            Some("Root")
        );
        let entries = value_array(nif.blocks[sequence_id].get_field("Controlled Blocks"));
        assert_eq!(entries.len(), 2);
        let names = HashMap::from([
            ("Second".to_string(), second_target_id),
            ("First".to_string(), first_target_id),
        ]);
        assert_eq!(
            fo76_controlled_block_target(&entries[0], &names),
            Some(second_target_id)
        );
        assert_eq!(
            fo76_controlled_block_target(&entries[1], &names),
            Some(first_target_id)
        );
        assert_eq!(
            field_ref(&nif.blocks[sequence_id], "Manager"),
            Some(manager_id as i32)
        );
        assert_eq!(
            ref_array(nif.blocks[multitarget_id].get_field("Extra Targets")),
            vec![second_target_id as i32, first_target_id as i32]
        );
        let palette_objects = value_array(nif.blocks[palette_id].get_field("Objs"));
        assert_eq!(palette_objects.len(), 2);
        let NifValue::Struct(first_palette_entry) = &palette_objects[0] else {
            panic!("palette entry must be a struct");
        };
        assert_eq!(
            value_ref(first_palette_entry.get("AV Object")),
            Some(second_target_id as i32)
        );
        assert_eq!(
            value_u64(nif.blocks[shader_id].get_field("Shader Flags 2")).unwrap_or_default()
                & u64::from(SLSF2_VERTEX_COLORS),
            u64::from(SLSF2_VERTEX_COLORS)
        );
    }

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
    fn prn_weapon_marks_weapon_collision_intent() {
        let mut nif = NifFile::new("fo76");
        let prn_id = nif.add_block("NiStringExtraData", None);
        nif.blocks[prn_id].set_field("Name", NifValue::String("Prn".to_string()));
        nif.blocks[prn_id].set_field("String Data", NifValue::String("WEAPON".to_string()));

        assert!(nif_collision_intent(&nif).is_weapon_model);
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
            is_ground_object: false,
            is_weapon_model: false,
        };
        let body = ExtractedCollisionBody {
            body_id: 1,
            source_polytopes: Vec::new(),
            source_compound_children: Vec::new(),
            source_compressed_mesh: None,
            source_primitive: None,
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
    fn dynamic_noncomplex_compound_requires_loose_item_role() {
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
            is_ground_object: false,
            is_weapon_model: false,
        };
        let body = ExtractedCollisionBody {
            body_id: 0,
            source_polytopes: Vec::new(),
            source_compound_children: Vec::new(),
            source_compressed_mesh: None,
            source_primitive: None,
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
        assert!(
            source_body_is_dynamic_for_nif(
                metadata,
                NifCollisionIntent {
                    is_weapon_model: true,
                    ..intent
                },
                Some(&body),
            ),
            "Prn=WEAPON compounds must remain dynamic loose clutter"
        );
    }

    /// A dropped power armor piece (`GO_Ultra_Helmet`) carries the SAME source
    /// signals as `WhitespringLamp03Off` — BSX 194 (dynamic, non-complex),
    /// layer 4, flags 128, motionType 2, compound_polytope — so the static
    /// compound rule above alone would ship power armor with no motionCinfo,
    /// no mass and a STATIC filter. Vanilla FO4 ships its own
    /// ground objects (`go_t51_helmet.nif`, BSX 194, dynamic compound with
    /// inverseMass 1/7.0) exactly this way, so the ground-object role must
    /// re-admit them.
    #[test]
    fn dynamic_noncomplex_compound_ground_object_is_loose_clutter() {
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
            is_ground_object: true,
            is_weapon_model: false,
        };
        let body = ExtractedCollisionBody {
            body_id: 0,
            source_polytopes: Vec::new(),
            source_compound_children: Vec::new(),
            source_compressed_mesh: None,
            source_primitive: None,
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
            source_body_is_dynamic_for_nif(metadata, intent, Some(&body)),
            "GO_Ultra_Helmet-style ground objects must stay dynamic loose clutter"
        );
    }

    /// FO76 authors ~a third of its ground objects with BSX 130
    /// (Havok|Articulated, no Dynamic) and `hknpMotionType::STATIC`, keeping the
    /// motion purely at runtime — `Headwear_Fasnacht_Mask_Bigfoot`
    /// (`BigfootFasnachtMaskHeadwear_GO.nif`, 7AC159) is one. Requiring
    /// `has_dynamic_bsx` would ship those as STATIC(1) bodies with no
    /// `motionCinfos` that read in-game as having no collision.
    /// Vanilla FO4 ships 189 of 192 `go*.nif` as dynamic clutter, so the
    /// ground-object role — not the source BSX — has to decide it.
    #[test]
    fn clutter_layer_ground_object_without_dynamic_bsx_is_loose_clutter() {
        let metadata = SourceBodyMetadata {
            layer: Some(FO4_CLUTTER_LAYER),
            motion_type: Some(0), // hknpMotionType::STATIC
            has_ref_mass_distribution: false,
            ..SourceBodyMetadata::default()
        };
        let intent = NifCollisionIntent {
            bsx_flags: BSX_HAVOK_FLAG | BSX_ARTICULATED_FLAG,
            has_dynamic_bsx: false,
            has_complex_bsx: false,
            is_ground_object: true,
            is_weapon_model: false,
        };
        let body = ExtractedCollisionBody {
            body_id: 0,
            source_polytopes: Vec::new(),
            source_compound_children: Vec::new(),
            source_compressed_mesh: None,
            source_primitive: None,
            meshes: vec![havok_native::collision::PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                triangles: vec![[0, 1, 2]],
            }],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: None,
            is_dynamic: false,
        };

        assert!(
            source_body_is_dynamic_for_nif(metadata, intent, Some(&body)),
            "clutter-layer ground objects must be loose clutter even when the source BSX omits Dynamic"
        );
    }

    /// FO76 authors the same class of ground object on either layer — the
    /// Hellcat torso (`GO_HellcatsMercenaryPA_Body.nif`, 60D5B8) lands on
    /// CLUTTER(4) while the Vulcan torso (`ATX_PA_Vulcan_Torso_GO.nif`, 788D0E)
    /// lands on STATIC(1), both with BSX 130 and no `motionCinfos`. Layer 1 is
    /// therefore inconsistent authoring, not an instruction to stay static: a
    /// dropped power armor torso must be loose clutter in FO4 either way.
    #[test]
    fn static_layer_ground_object_without_dynamic_bsx_is_loose_clutter() {
        let metadata = SourceBodyMetadata {
            layer: Some(FO4_STATIC_LAYER),
            motion_type: Some(0), // hknpMotionType::STATIC
            has_ref_mass_distribution: false,
            ..SourceBodyMetadata::default()
        };
        let intent = NifCollisionIntent {
            bsx_flags: BSX_HAVOK_FLAG | BSX_ARTICULATED_FLAG,
            has_dynamic_bsx: false,
            has_complex_bsx: false,
            is_ground_object: true,
            is_weapon_model: false,
        };
        let body = ExtractedCollisionBody {
            body_id: 0,
            source_polytopes: Vec::new(),
            source_compound_children: Vec::new(),
            source_compressed_mesh: None,
            source_primitive: None,
            meshes: vec![havok_native::collision::PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                triangles: vec![[0, 1, 2]],
            }],
            layer: Some(FO4_STATIC_LAYER),
            material_crc: None,
            is_dynamic: false,
        };

        assert!(
            source_body_is_dynamic_for_nif(metadata, intent, Some(&body)),
            "Vulcan-torso-style layer-1 ground objects must be loose clutter too"
        );
    }

    /// The promotion is confined to the two ordinary solid layers FO76 actually
    /// authors ground objects on (STATIC/CLUTTER, 405 of the 410 broken
    /// meshes). Anything else — the handful on layer 8, or a trigger/volume
    /// layer reached via the root-node-name fallback in
    /// `is_fo76_ground_object_nif` — keeps its source behaviour.
    #[test]
    fn ground_object_on_unusual_layer_is_not_promoted() {
        let metadata = SourceBodyMetadata {
            layer: Some(8),
            motion_type: Some(0), // hknpMotionType::STATIC
            has_ref_mass_distribution: false,
            ..SourceBodyMetadata::default()
        };
        let intent = NifCollisionIntent {
            bsx_flags: BSX_HAVOK_FLAG | BSX_ARTICULATED_FLAG,
            has_dynamic_bsx: false,
            has_complex_bsx: false,
            is_ground_object: true,
            is_weapon_model: false,
        };
        let body = ExtractedCollisionBody {
            body_id: 0,
            source_polytopes: Vec::new(),
            source_compound_children: Vec::new(),
            source_compressed_mesh: None,
            source_primitive: None,
            meshes: vec![havok_native::collision::PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                triangles: vec![[0, 1, 2]],
            }],
            layer: Some(8),
            material_crc: None,
            is_dynamic: false,
        };

        assert!(
            !source_body_is_dynamic_for_nif(metadata, intent, Some(&body)),
            "ground objects on layers outside STATIC/CLUTTER must keep source behaviour"
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
            is_ground_object: false,
            is_weapon_model: false,
        };
        let body = ExtractedCollisionBody {
            body_id: 0,
            source_polytopes: Vec::new(),
            source_compound_children: Vec::new(),
            source_compressed_mesh: None,
            source_primitive: None,
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

    /// Build the shape of an FO4/FO76 actor skeleton: root with BSBound, a
    /// `bhkNPCollisionObject` on a bone, and the ragdoll as one
    /// `bhkRagdollSystem` blob — no `bhkConstraint` blocks anywhere.
    fn fo4_actor_skeleton(bsx_flags: u64, with_ragdoll_system: bool) -> NifFile {
        let mut nif = NifFile::default();
        let mut root = NifBlock::new(0, "NiNode");
        root.set_field("Extra Data List", NifValue::Array(vec![NifValue::Ref(1)]));
        root.set_field("Num Extra Data List", NifValue::UInt(1));
        root.set_field("Children", NifValue::Array(vec![NifValue::Ref(3)]));
        let mut bsx = NifBlock::new(1, "BSXFlags");
        bsx.set_field("Integer Data", NifValue::UInt(bsx_flags));
        let bound = NifBlock::new(2, "BSBound");
        let mut bone = NifBlock::new(3, "NiNode");
        bone.set_field("Collision Object", NifValue::Ref(4));
        let collision = NifBlock::new(4, "bhkNPCollisionObject");
        nif.blocks.push(root);
        nif.blocks.push(bsx);
        nif.blocks.push(bound);
        nif.blocks.push(bone);
        nif.blocks.push(collision);
        if with_ragdoll_system {
            nif.blocks.push(NifBlock::new(5, "bhkRagdollSystem"));
        }
        nif
    }

    #[test]
    fn bsx_contract_keeps_the_ragdoll_flag_on_an_fo4_format_skeleton() {
        // 198 = Havok | Ragdoll | Dynamic | Articulated, what all 29 vanilla FO4
        // actor skeletons and all 65 FO76 source skeletons author.
        let mut nif = fo4_actor_skeleton(198, true);
        let mut report = ConvertFileReport::default();
        normalize_fo76_bsx_contract(&mut nif, &mut report);
        assert_eq!(
            value_u64(nif.blocks[1].get_field("Integer Data")),
            Some(198),
            "bhkRagdollSystem must satisfy the ragdoll contract; dropping to 194 \
             strips the flag from every converted creature skeleton"
        );
    }

    #[test]
    fn bsx_contract_adds_the_ragdoll_flag_when_the_source_omitted_it() {
        let mut nif = fo4_actor_skeleton(194, true);
        let mut report = ConvertFileReport::default();
        normalize_fo76_bsx_contract(&mut nif, &mut report);
        assert_eq!(
            value_u64(nif.blocks[1].get_field("Integer Data")),
            Some(198)
        );
    }

    #[test]
    fn bsx_contract_preserves_an_authored_ragdoll_flag_without_ragdoll_blocks() {
        // Vanilla FO4 `robot` / `createabot` ship BSX 70 with no ragdoll blocks,
        // so the flag is authored intent the structure cannot re-derive.
        let mut nif = fo4_actor_skeleton(70, false);
        let mut report = ConvertFileReport::default();
        normalize_fo76_bsx_contract(&mut nif, &mut report);
        assert_eq!(value_u64(nif.blocks[1].get_field("Integer Data")), Some(70));
    }

    #[test]
    fn legacy_aabb_collision_uses_fo4_np_blocks() {
        let mut nif = NifFile::new("fo4");
        let parent_id = nif.add_block("NiNode", None);
        let vertices = [
            [-69.99125, -139.9825, -209.97375],
            [69.99125, 139.9825, 209.97375],
        ];

        build_box_collision(&mut nif, parent_id, &vertices).expect("build collision");

        assert!(
            nif.blocks
                .iter()
                .any(|block| block.type_name == "bhkNPCollisionObject")
        );
        assert!(
            nif.blocks
                .iter()
                .any(|block| block.type_name == "bhkPhysicsSystem")
        );
        assert!(nif.blocks.iter().all(|block| !matches!(
            block.type_name.as_str(),
            "bhkCollisionObject" | "bhkRigidBody" | "bhkRigidBodyT" | "bhkBoxShape"
        )));
        let collision = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "bhkNPCollisionObject")
            .expect("collision object");
        let blob = collision_physics_blob(&nif, collision).expect("collision blob");
        let summary = havok_native::api::havok_collision_summary(&blob).expect("collision summary");
        assert!(!summary.contains("\"n_vertices\":0"));
    }

    #[test]
    fn legacy_dynamic_sphere_preserves_clutter_body_intent() {
        let mut nif = NifFile::default();
        let mut bsx = NifBlock::new(0, "BSXFlags");
        bsx.set_field("Integer Data", NifValue::UInt(0x42));
        let mut collision = NifBlock::new(1, "bhkCollisionObject");
        collision.set_field("Body", NifValue::Ref(2));
        let mut body = NifBlock::new(2, "bhkRigidBodyT");
        body.set_field("Shape", NifValue::Ref(3));
        body.set_field(
            "Rigid Body Info:550_660",
            NifValue::Struct(IndexMap::from([
                (
                    "Havok Filter".to_string(),
                    NifValue::Struct(IndexMap::from([(
                        "Layer:FO".to_string(),
                        NifValue::UInt(FO4_CLUTTER_LAYER.into()),
                    )])),
                ),
                ("Motion System".to_string(), NifValue::UInt(2)),
                ("Mass".to_string(), NifValue::Float(2.5)),
                ("Friction".to_string(), NifValue::Float(0.75)),
                ("Restitution".to_string(), NifValue::Float(0.9)),
                (
                    "Translation".to_string(),
                    NifValue::Vec4([1.0, 2.0, 3.0, 0.0]),
                ),
                ("Rotation".to_string(), NifValue::Vec4([0.0, 0.0, 0.0, 1.0])),
            ])),
        );
        let mut sphere = NifBlock::new(3, "bhkSphereShape");
        sphere.set_field("Radius", NifValue::Float(7.0));
        nif.blocks.extend([bsx, collision, body, sphere]);

        let spec =
            legacy_dynamic_collision(&nif, &nif.blocks[1], "fnv", nif_collision_intent(&nif))
                .expect("legacy dynamic collision");

        assert_eq!(spec.mass, 2.5);
        assert_eq!(spec.friction, 0.75);
        assert_eq!(spec.restitution, 0.9);
        assert!((spec.position[0] - LEGACY_HAVOK_UNIT_SCALE).abs() < 0.0001);
        let Some(MultiBodyShape::Polytope { vertices }) = spec.shape else {
            panic!("expected rounded dynamic polytope");
        };
        assert_eq!(vertices.len(), 26);
    }

    #[test]
    fn legacy_collision_subtree_includes_tri_strips_data() {
        let mut nif = NifFile::default();

        let root = NifBlock::new(0, "NiNode");
        let mut collision = NifBlock::new(1, "bhkCollisionObject");
        collision.set_field("Body", NifValue::Ref(2));
        let mut body = NifBlock::new(2, "bhkRigidBody");
        body.set_field("Shape", NifValue::Ref(3));
        let mut mopp = NifBlock::new(3, "bhkMoppBvTreeShape");
        mopp.set_field("Shape", NifValue::Ref(4));
        let mut strips = NifBlock::new(4, "bhkNiTriStripsShape");
        strips.set_field("Strips Data", NifValue::Array(vec![NifValue::Ref(5)]));
        let data = NifBlock::new(5, "NiTriStripsData");
        nif.blocks
            .extend([root, collision, body, mopp, strips, data]);

        let mut subtree = HashSet::new();
        collect_collision_subtree(&nif, 1, &mut subtree);

        assert_eq!(subtree, HashSet::from([1, 2, 3, 4, 5]));
    }

    /// Builds the FNV/FO3 shape a rock or building uses: a MOPP-wrapped packed
    /// tri-strips mesh under a `bhkRigidBodyT`, whose translation is baked into
    /// the shape. Geometry is a unit tetrahedron offset by the body translation.
    fn fnv_packed_strips_nif(body_type: &str, translation: [f32; 4]) -> NifFile {
        let mut nif = NifFile::new("fnv");
        let mut root = NifBlock::new(0, "NiNode");
        root.set_field("Name", NifValue::String("Rock01".to_string()));
        root.set_field("Collision Object", NifValue::Ref(1));

        let mut collision = NifBlock::new(1, "bhkCollisionObject");
        collision.set_field("Target", NifValue::Ref(0));
        collision.set_field("Body", NifValue::Ref(2));

        let mut body = NifBlock::new(2, body_type);
        body.set_field("Shape", NifValue::Ref(3));
        body.set_field(
            "Rigid Body Info:550_660",
            NifValue::Struct(IndexMap::from([
                (
                    "Havok Filter".to_string(),
                    NifValue::Struct(IndexMap::from([(
                        "Layer:FO".to_string(),
                        NifValue::UInt(1),
                    )])),
                ),
                ("Motion System".to_string(), NifValue::UInt(7)),
                ("Mass".to_string(), NifValue::Float(0.0)),
                ("Translation".to_string(), NifValue::Vec4(translation)),
                ("Rotation".to_string(), NifValue::Vec4([0.0, 0.0, 0.0, 1.0])),
            ])),
        );

        let mut mopp = NifBlock::new(3, "bhkMoppBvTreeShape");
        mopp.set_field("Shape", NifValue::Ref(4));

        let mut shape = NifBlock::new(4, "bhkPackedNiTriStripsShape");
        shape.set_field("Data", NifValue::Ref(5));
        shape.set_field("Scale", NifValue::Vec4([1.0, 1.0, 1.0, 0.0]));

        let mut data = NifBlock::new(5, "hkPackedNiTriStripsData");
        data.set_field("Compressed", NifValue::UInt(0));
        data.set_field(
            "Vertices",
            NifValue::Array(vec![
                NifValue::Vec3([0.0, 0.0, 0.0]),
                NifValue::Vec3([10.0, 0.0, 0.0]),
                NifValue::Vec3([0.0, 10.0, 0.0]),
                NifValue::Vec3([0.0, 0.0, 10.0]),
            ]),
        );
        let triangle = |v1: u64, v2: u64, v3: u64| {
            NifValue::Struct(IndexMap::from([(
                "Triangle".to_string(),
                NifValue::Struct(IndexMap::from([
                    ("v1".to_string(), NifValue::UInt(v1)),
                    ("v2".to_string(), NifValue::UInt(v2)),
                    ("v3".to_string(), NifValue::UInt(v3)),
                ])),
            )]))
        };
        data.set_field(
            "Triangles",
            NifValue::Array(vec![
                triangle(0, 1, 2),
                triangle(0, 1, 3),
                triangle(0, 2, 3),
                triangle(1, 2, 3),
            ]),
        );

        nif.blocks.clear();
        nif.blocks
            .extend([root, collision, body, mopp, shape, data]);
        nif
    }

    fn physics_blob(nif: &NifFile) -> Vec<u8> {
        let physics = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "bhkPhysicsSystem")
            .expect("physics system");
        crate::cloth::byte_array_to_bytes(physics.get_field("Binary Data").expect("binary data"))
            .expect("blob")
    }

    #[test]
    fn fnv_packed_tri_strips_collision_becomes_fo4_mesh_not_a_box() {
        let mut nif = fnv_packed_strips_nif("bhkRigidBody", [0.0, 0.0, 0.0, 0.0]);
        let mut report = ConvertFileReport::default();
        regenerate_fo4_collision(&mut nif, "fnv", &mut report);

        let summary =
            havok_native::api::havok_collision_summary(&physics_blob(&nif)).expect("summary");
        assert!(
            summary.contains("hknpCompressedMeshShape"),
            "packed tri strips must convert to an FO4 mesh, got {summary}"
        );
        assert!(
            !summary.contains("hknpConvexPolytopeShape"),
            "packed tri strips must not collapse to an AABB box, got {summary}"
        );
    }

    #[test]
    fn fnv_packed_tri_strips_collision_uses_legacy_havok_units() {
        let mut nif = fnv_packed_strips_nif("bhkRigidBody", [0.0, 0.0, 0.0, 0.0]);
        let mut report = ConvertFileReport::default();
        regenerate_fo4_collision(&mut nif, "fnv", &mut report);

        // The source tetrahedron spans 10 legacy Havok units per axis, which is
        // 1.0 FO4 Havok unit — roughly 70 game units.
        let preview = havok_native::api::havok_collision_preview(&physics_blob(&nif), 1.0, Some(0))
            .expect("preview");
        let mut max = f32::MIN;
        for axis in ["\"x\":", "\"y\":", "\"z\":"] {
            for chunk in preview.split(axis).skip(1) {
                let value = chunk
                    .trim_start()
                    .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e'))
                    .next()
                    .unwrap_or_default();
                if let Ok(value) = value.parse::<f32>() {
                    max = max.max(value);
                }
            }
        }
        assert!(
            (max - 1.0).abs() < 0.05,
            "expected a 1.0 FO4-Havok-unit extent from 10 legacy units, got {max}"
        );
    }

    #[test]
    fn fnv_rigid_body_t_translation_is_baked_into_the_shape() {
        let mut plain = fnv_packed_strips_nif("bhkRigidBody", [0.0, 0.0, 20.0, 0.0]);
        let mut transformed = fnv_packed_strips_nif("bhkRigidBodyT", [0.0, 0.0, 20.0, 0.0]);
        let mut report = ConvertFileReport::default();
        regenerate_fo4_collision(&mut plain, "fnv", &mut report);
        regenerate_fo4_collision(&mut transformed, "fnv", &mut report);

        assert_ne!(
            physics_blob(&plain),
            physics_blob(&transformed),
            "bhkRigidBodyT must bake its translation into the shape; bhkRigidBody must not"
        );
    }

    #[test]
    fn unsupported_legacy_shape_still_falls_back_to_the_visible_mesh_box() {
        let mut nif = fnv_packed_strips_nif("bhkRigidBody", [0.0, 0.0, 0.0, 0.0]);
        // Break the shape chain so the source decode fails.
        nif.blocks[4].set_field("Data", NifValue::Ref(-1));
        let mut geometry = NifBlock::new(6, "BSTriShape");
        geometry.set_field("Name", NifValue::String("Rock01:0".to_string()));
        geometry.set_field(
            "Vertex Data",
            NifValue::Array(vec![
                NifValue::Vec3([0.0, 0.0, 0.0]),
                NifValue::Vec3([70.0, 0.0, 0.0]),
                NifValue::Vec3([0.0, 70.0, 0.0]),
                NifValue::Vec3([0.0, 0.0, 70.0]),
            ]),
        );
        nif.blocks.push(geometry);
        nif.blocks[0].set_field("Children", NifValue::Array(vec![NifValue::Ref(6)]));

        let mut report = ConvertFileReport::default();
        regenerate_fo4_collision(&mut nif, "fnv", &mut report);

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("source shape unsupported")),
            "a failed source decode must be reported, got {:?}",
            report.warnings
        );
    }

    fn fnv_extracted_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fnv")
    }

    fn skyrimse_extracted_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/skyrimse")
    }

    fn conversion_translation_maps_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps")
    }

    fn fo4_extracted_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo4")
    }

    fn fo4_humanoid_skeleton() -> std::path::PathBuf {
        fo4_extracted_dir().join("meshes/actors/character/characterassets/skeleton.nif")
    }

    /// Largest per-bone deviation of `world_skeleton(bone) @ bind` from the
    /// field's own median. Zero means every bone agrees on one rigid offset,
    /// which is the only thing that renders undeformed; per-bone disagreement
    /// is exactly what explodes a mesh.
    fn bind_offset_spread(mesh: &NifFile, skeleton: &NifFile) -> f64 {
        let binds = crate::skeleton_repose::collect_bind_matrices_by_name(mesh);
        let residuals = crate::skeleton_repose::skeleton_bind_offsets(skeleton, &binds);
        // Guards against a vacuous pass: a spread measured over a subset of
        // the bones says nothing, and a bone the skeleton lacks is itself the
        // defect. Small garments bind few bones (iron boots bind 8), so the
        // invariant is "all of them resolved", not a fixed count.
        assert!(!residuals.is_empty(), "mesh bound no bones");
        assert_eq!(
            residuals.len(),
            binds.len(),
            "only {} of {} bound bones resolved against the skeleton",
            residuals.len(),
            binds.len()
        );
        let mut spread = 0.0_f64;
        for axis in 0..3 {
            let mut values: Vec<f64> = residuals.iter().map(|r| r[axis]).collect();
            values.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median = values[values.len() / 2];
            for value in values {
                spread = spread.max((value - median).abs());
            }
        }
        spread
    }

    /// FNV clothing renamed onto FO4 bones must have its Gamebryo bind matrices
    /// recomputed against the FO4 rest pose; otherwise every bone pulls its
    /// vertices somewhere different and the mesh explodes (arms ~150 units out).
    #[test]
    fn real_fnv_vault_suit_binds_cohere_with_the_fo4_skeleton() {
        let source = fnv_extracted_dir().join("meshes/armor/vaultsuit/m/outfit.nif");
        let skeleton_path = fo4_humanoid_skeleton();
        if !source.is_file() || !skeleton_path.is_file() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("outfit.nif");
        let options = ConvertFileOptions {
            translation_maps_dir: Some(conversion_translation_maps_dir()),
            target_skeleton: Some(skeleton_path.clone()),
            ..ConvertFileOptions::default()
        };

        let report = convert_nif_file(&source, &output, "fnv", "fo4", None, &options).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.shapes_skinned > 0, "{:?}", report.changes);

        let converted = NifFile::load(&output).unwrap();
        let skeleton = NifFile::load(&skeleton_path).unwrap();

        // Vanilla FO4 clothing measures ~2.8 on this metric; the unrebound
        // FNV conversion measured ~118.
        let spread = bind_offset_spread(&converted, &skeleton);
        assert!(
            spread <= 3.0,
            "converted binds disagree across bones by {spread:.2} units"
        );

        // Every remapped bone must exist in the skeleton it binds to.
        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.starts_with("Skin rebind:")),
            "{:?}",
            report.warnings
        );
    }

    /// The rebind is wired for every legacy pair, so Skyrim armor must land on
    /// the same coherent rest pose FNV clothing does.
    #[test]
    fn real_skyrim_armor_binds_cohere_with_the_fo4_skeleton() {
        let skeleton_path = fo4_humanoid_skeleton();
        if !skeleton_path.is_file() {
            return;
        }
        let skeleton = NifFile::load(&skeleton_path).unwrap();
        for relative in [
            "Meshes/Armor/Iron/Male/CuirassLight_1.nif",
            "Meshes/Armor/Iron/Male/Boots_1.nif",
        ] {
            let source = skyrimse_extracted_dir().join(relative);
            if !source.is_file() {
                continue;
            }
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("converted.nif");
            let options = ConvertFileOptions {
                translation_maps_dir: Some(conversion_translation_maps_dir()),
                target_skeleton: Some(skeleton_path.clone()),
                ..ConvertFileOptions::default()
            };

            let report =
                convert_nif_file(&source, &output, "skyrimse", "fo4", None, &options).unwrap();
            if report.shapes_skinned == 0 {
                continue;
            }
            let converted = NifFile::load(&output).unwrap();
            let spread = bind_offset_spread(&converted, &skeleton);
            assert!(
                spread <= 3.0,
                "{relative} binds disagree across bones by {spread:.2} units"
            );
        }
    }

    #[test]
    fn real_skyrim_tree_uses_static_switch_child() {
        let source = skyrimse_extracted_dir().join("Meshes/Landscape/Trees/TreePineForest01.nif");
        if !source.is_file() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("TreePineForest01.nif");
        let report = convert_nif_file(
            &source,
            &output,
            "skyrimse",
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .unwrap();

        assert!(report.supported, "conversion errors: {:?}", report.errors);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("static switch child")),
            "conversion did not use the tree fallback: {:?}",
            report.changes
        );

        let converted = NifFile::load(output).unwrap();
        assert_eq!(converted.header.version, (20, 2, 0, 7));
        assert_eq!(converted.header.user_version, 12);
        assert_eq!(converted.header.bs_version, 130);
        assert!(crate::skyrim::validate_unskinned_geometry(&converted).is_ok());
        assert!(
            converted
                .blocks
                .iter()
                .any(|block| block.type_name == "BSLeafAnimNode")
        );
        assert!(converted.blocks.iter().any(|block| {
            block.type_name == "BSTriShape"
                && matches!(block.get_field("Vertex Data"), Some(NifValue::Array(data)) if !data.is_empty())
        }));
    }

    #[test]
    fn real_skyrim_armor_corpus_is_repacked_as_fo4_skinned_geometry() {
        let source_root = skyrimse_extracted_dir();
        for relative in [
            "Meshes/Armor/Iron/Male/CuirassLight_1.nif",
            "Meshes/Armor/Iron/F/CuirassLight_1.nif",
            "Meshes/Armor/Iron/Male/Gauntlets_1.nif",
            "Meshes/Armor/Iron/Male/Boots_1.nif",
        ] {
            let source = source_root.join(relative);
            if !source.is_file() {
                continue;
            }
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("converted.nif");
            let material_dir = temp.path().join("Materials");
            let options = ConvertFileOptions {
                translation_maps_dir: Some(conversion_translation_maps_dir()),
                ..ConvertFileOptions::default()
            };

            let report = convert_nif_file(
                &source,
                &output,
                "skyrimse",
                "fo4",
                Some(&material_dir),
                &options,
            )
            .unwrap();

            assert!(
                report.supported,
                "{relative} conversion errors: {:?}",
                report.errors
            );
            assert!(report.errors.is_empty(), "{relative}: {:?}", report.errors);
            assert!(
                report.shapes_skinned > 0,
                "{relative}: {:?}",
                report.changes
            );
            assert_eq!(
                report.bones_dropped_unmapped, 0,
                "{relative}: {:?}",
                report.warnings
            );

            let converted = NifFile::load(output).unwrap();
            assert_eq!(converted.header.version, (20, 2, 0, 7), "{relative}");
            assert_eq!(converted.header.user_version, 12, "{relative}");
            assert_eq!(converted.header.bs_version, 130, "{relative}");
            assert!(
                converted
                    .blocks
                    .iter()
                    .any(|block| block.type_name == "BSSkin::Instance"),
                "{relative}"
            );
            assert!(
                converted.blocks.iter().any(|block| {
                    block.type_name == "BSSubIndexTriShape"
                        && matches!(block.get_field("Skin"), Some(NifValue::Ref(reference)) if *reference >= 0)
                }),
                "{relative}"
            );
            assert!(
                !converted.blocks.iter().any(|block| {
                    matches!(
                        block.type_name.as_str(),
                        "NiSkinInstance"
                            | "BSDismemberSkinInstance"
                            | "NiSkinData"
                            | "NiSkinPartition"
                    )
                }),
                "{relative}"
            );
        }
    }

    #[test]
    fn real_skyrim_argonian_facegeom_is_repacked_as_fo4_geometry() {
        let source = skyrimse_extracted_dir()
            .join("Meshes/Actors/Character/FaceGenData/FaceGeom/Skyrim.esm/00103512.nif");
        if !source.is_file() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("argonian-facegeom.nif");
        let materials = temp.path().join("Materials");
        let options = ConvertFileOptions {
            translation_maps_dir: Some(conversion_translation_maps_dir()),
            ..ConvertFileOptions::default()
        };

        let report = convert_nif_file(
            &source,
            &output,
            "skyrimse",
            "fo4",
            Some(&materials),
            &options,
        )
        .unwrap();

        assert!(report.supported, "conversion errors: {:?}", report.errors);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.shapes_skinned >= 3, "{:?}", report.changes);
        assert!(!report.emitted_bgsms.is_empty());

        let converted = NifFile::load(output).unwrap();
        assert!(
            !converted
                .blocks
                .iter()
                .any(|block| block.type_name == "BSDynamicTriShape")
        );
        let head = converted
            .blocks
            .iter()
            .find(|block| {
                block.type_name == "BSSubIndexTriShape"
                    && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "MaleHeadArgonian")
            })
            .expect("converted Argonian head");
        let vertex_count = match head.get_field("Vertex Data") {
            Some(NifValue::Array(vertices)) => vertices.len(),
            _ => 0,
        };
        assert_eq!(
            vertex_count,
            1219,
            "fields={:?}",
            head.fields.keys().collect::<Vec<_>>()
        );
        assert!(matches!(head.get_field("Skin"), Some(NifValue::Ref(id)) if *id >= 0));
    }

    #[test]
    fn real_fnv_republican_outfit_preserves_nonidentity_inverse_bind_rotations() {
        let source = fnv_extracted_dir().join("Meshes/armor/republicans/republican_02.nif");
        if !source.is_file() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("republican_02.nif");
        let options = ConvertFileOptions {
            translation_maps_dir: Some(conversion_translation_maps_dir()),
            ..ConvertFileOptions::default()
        };

        let report = convert_nif_file(&source, &output, "fnv", "fo4", None, &options).unwrap();
        assert!(report.supported, "conversion errors: {:?}", report.errors);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.shapes_skinned > 0, "{:?}", report.changes);

        let converted = NifFile::load(output).unwrap();
        let binds = crate::skeleton_repose::collect_bind_matrices_by_name(&converted);
        let consistency =
            crate::skeleton_repose::skeleton_bind_consistency(&converted, &binds, 0.05);
        assert!(!binds.is_empty());
        assert!(
            consistency.1 >= 20 && consistency.0 + 2 >= consistency.1,
            "converted FNV bone nodes disagree with their inverse binds: {consistency:?}"
        );
        assert!(
            binds.values().any(|matrix| {
                matrix[0][1].abs() > 0.01
                    || matrix[0][2].abs() > 0.01
                    || matrix[1][0].abs() > 0.01
                    || matrix[1][2].abs() > 0.01
                    || matrix[2][0].abs() > 0.01
                    || matrix[2][1].abs() > 0.01
            }),
            "FNV inverse-bind rotations were replaced by identity: {binds:?}"
        );
    }

    #[test]
    fn real_fnv_unskinned_hair_beard_and_hat_become_fo4_geometry() {
        let source_root = fnv_extracted_dir();
        for relative in [
            "Meshes/characters/hair/beardfullold.nif",
            "Meshes/characters/hair/hairbaseold.nif",
            "Meshes/armor/headgear/cowboyhat/cowboyhat2.nif",
        ] {
            let source = source_root.join(relative);
            if !source.is_file() {
                continue;
            }
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("converted.nif");
            convert_nif_file(
                &source,
                &output,
                "fnv",
                "fo4",
                None,
                &ConvertFileOptions::default(),
            )
            .unwrap();
            let converted = NifFile::load(&output).unwrap();

            assert!(
                converted.blocks.iter().any(|block| {
                    block.type_name == "BSSubIndexTriShape"
                        && matches!(block.get_field("Vertex Data"), Some(NifValue::Array(data)) if !data.is_empty())
                }),
                "{relative} must contain FO4 inline geometry"
            );
            assert!(
                !converted.blocks.iter().any(|block| matches!(
                    block.type_name.as_str(),
                    "NiTriShape" | "NiTriStrips" | "NiTriShapeData" | "NiTriStripsData"
                )),
                "{relative} retained legacy geometry blocks"
            );
        }
    }

    #[test]
    fn real_fnv_beard_full_old_prepares_as_facegen_hair_geometry() {
        let source = fnv_extracted_dir().join("Meshes/characters/hair/beardfullold.nif");
        if !source.is_file() {
            return;
        }
        let mut nif = NifFile::load(&source).unwrap();

        assert_eq!(prepare_legacy_face_part_for_fo4(&mut nif), 1);
        assert!(nif.blocks.iter().any(|block| {
            block.type_name == "BSSubIndexTriShape"
                && string_field(block, "Name").as_deref() == Some("BeardFullOld:0")
                && matches!(block.get_field("Vertex Data"), Some(NifValue::Array(data)) if data.len() == 562)
        }));
        assert!(nif.blocks.iter().any(|block| {
            block.type_name == "BSShaderTextureSet"
                && value_array(block.get_field("Textures")).iter().any(|texture| {
                    matches!(texture, NifValue::String(path) if path.eq_ignore_ascii_case("textures\\characters\\hair\\BeardFull.dds"))
                })
        }));
    }

    #[test]
    fn real_fnv_packed_mesh_collision_matches_the_source_hull() {
        let path = fnv_extracted_dir().join("meshes/landscape/rocks/nv_qj_limepile03.nif");
        if !path.is_file() {
            return;
        }
        let mut nif = NifFile::load(&path).expect("load FNV packed strips fixture");
        let mut report = ConvertFileReport::default();
        regenerate_fo4_collision(&mut nif, "fnv", &mut report);

        let decoded = havok_native::collision::parse_fo4_compressed_mesh(&physics_blob(&nif))
            .expect("converted collision must be an FO4 compressed mesh");
        assert_eq!(
            decoded
                .sections
                .iter()
                .map(|section| section.triangles.len())
                .sum::<usize>(),
            48,
            "every source triangle must survive conversion"
        );

        // The source body translation puts the hull's base exactly on the
        // visible mesh's lowest vertex; a unit or transform error breaks this.
        let min_z = decoded
            .sections
            .iter()
            .flat_map(|section| section.vertices.iter())
            .map(|vertex| vertex[2] * HAVOK_SCALE)
            .fold(f32::MAX, f32::min);
        assert!(
            (min_z - (-58.02)).abs() < 0.5,
            "collision hull base should register against the visible mesh at -58.02, got {min_z}"
        );
    }

    /// (consistent, shared) edge counts. A correctly wound mesh has every shared
    /// edge traversed in opposite directions by its two triangles.
    /// (consistent, shared) over a raw vertex/triangle pair.
    fn shared_edge_orientation(vertices: &[[f32; 3]], triangles: &[[u32; 3]]) -> (usize, usize) {
        use std::collections::HashMap;
        let mut welded: HashMap<[i64; 3], u32> = HashMap::new();
        let mut canonical = Vec::with_capacity(vertices.len());
        for vertex in vertices {
            let key = vertex.map(|value| (value * 2000.0).round() as i64);
            let next = welded.len() as u32;
            canonical.push(*welded.entry(key).or_insert(next));
        }
        let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
        for triangle in triangles {
            let [a, b, c] = [
                canonical[triangle[0] as usize],
                canonical[triangle[1] as usize],
                canonical[triangle[2] as usize],
            ];
            if a == b || b == c || a == c {
                continue;
            }
            for edge in [(a, b), (b, c), (c, a)] {
                *directed.entry(edge).or_default() += 1;
            }
        }
        let mut shared = 0usize;
        let mut consistent = 0usize;
        for (&(u, v), &count) in &directed {
            if u > v {
                continue;
            }
            let reverse = directed.get(&(v, u)).copied().unwrap_or(0);
            if count > 0 && reverse > 0 {
                shared += 1;
                consistent += 1;
            } else if count > 1 {
                shared += 1;
            }
        }
        (consistent, shared)
    }

    fn edge_orientation(
        mesh: &havok_native::collision::compressed_mesh::CompressedMeshData,
    ) -> (usize, usize) {
        use std::collections::HashMap;
        let mut welded: HashMap<[i64; 3], u32> = HashMap::new();
        let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
        for section in &mesh.sections {
            let mut canonical = Vec::with_capacity(section.vertices.len());
            for vertex in &section.vertices {
                let key = vertex.map(|value| (value * 2000.0).round() as i64);
                let next = welded.len() as u32;
                canonical.push(*welded.entry(key).or_insert(next));
            }
            for triangle in &section.triangles {
                let [a, b, c] = [
                    canonical[triangle[0] as usize],
                    canonical[triangle[1] as usize],
                    canonical[triangle[2] as usize],
                ];
                if a == b || b == c || a == c {
                    continue;
                }
                for edge in [(a, b), (b, c), (c, a)] {
                    *directed.entry(edge).or_default() += 1;
                }
            }
        }
        let mut shared = 0usize;
        let mut consistent = 0usize;
        for (&(u, v), &count) in &directed {
            if u > v {
                continue;
            }
            let reverse = directed.get(&(v, u)).copied().unwrap_or(0);
            if count > 0 && reverse > 0 {
                shared += 1;
                consistent += 1;
            } else if count > 1 {
                shared += 1;
            }
        }
        (consistent, shared)
    }

    #[test]
    fn fnv_mesh_collision_is_wound_consistently() {
        let mut nif = fnv_packed_strips_nif("bhkRigidBody", [0.0, 0.0, 0.0, 0.0]);
        let mut report = ConvertFileReport::default();
        regenerate_fo4_collision(&mut nif, "fnv", &mut report);

        let decoded = havok_native::collision::parse_fo4_compressed_mesh(&physics_blob(&nif))
            .expect("compressed mesh");
        let total: usize = decoded
            .sections
            .iter()
            .map(|section| section.triangles.len())
            .sum();
        // Orientation must not duplicate geometry: duplicated opposing faces make
        // the player sink and bounce on anything walkable.
        assert_eq!(total, 4, "source triangles must not be duplicated");

        // The fixture's source winding is deliberately inconsistent, as FNV's is.
        let (consistent, shared) = edge_orientation(&decoded);
        assert_eq!(
            consistent, shared,
            "every shared edge must be consistently oriented after conversion"
        );
    }

    #[test]
    fn real_fnv_collision_is_wound_consistently() {
        let path =
            fnv_extracted_dir().join("meshes/architecture/goodsprings/nv_prospectorsaloon.nif");
        if !path.is_file() {
            return;
        }
        let mut nif = NifFile::load(&path).expect("load saloon fixture");
        let mut report = ConvertFileReport::default();
        regenerate_fo4_collision(&mut nif, "fnv", &mut report);

        let decoded = havok_native::collision::parse_fo4_compressed_mesh(&physics_blob(&nif))
            .expect("compressed mesh");
        let total: usize = decoded
            .sections
            .iter()
            .map(|section| section.triangles.len())
            .sum();
        assert_eq!(total, 409, "no duplication; every source triangle once");

        // Source scores 41% here; the converted output must be essentially perfect.
        let (consistent, shared) = edge_orientation(&decoded);
        assert!(
            consistent * 100 / shared >= 99,
            "saloon collision winding still inconsistent: {consistent}/{shared}"
        );
    }

    #[test]
    fn largest_fnv_collision_mesh_survives_orientation() {
        // 7000 source triangles is the FNV corpus maximum — 59 components to
        // flood-fill and sign independently.
        let path = fnv_extracted_dir().join("meshes/architecture/primm/eldiablocurvenorth.nif");
        if !path.is_file() {
            return;
        }
        let mut nif = NifFile::load(&path).expect("load largest FNV collision fixture");
        let mut report = ConvertFileReport::default();
        regenerate_fo4_collision(&mut nif, "fnv", &mut report);
        assert!(
            report
                .warnings
                .iter()
                .all(|warning| !warning.contains("rebuild failed")),
            "{:?}",
            report.warnings
        );

        let blob = physics_blob(&nif);
        let decoded =
            havok_native::collision::parse_fo4_compressed_mesh(&blob).expect("compressed mesh");
        let total: usize = decoded
            .sections
            .iter()
            .map(|section| section.triangles.len())
            .sum();
        assert_eq!(total, 7000, "7000 source triangles, none duplicated");
        assert!(
            decoded
                .sections
                .iter()
                .all(|section| section.triangles.len() <= 128),
            "sections must stay within the FO4 per-section triangle limit"
        );
        let summary = havok_native::api::havok_collision_summary(&blob).expect("summary");
        assert!(summary.contains("\"geometry_status\":\"ok\""), "{summary}");
    }

    #[test]
    fn degenerate_source_triangle_does_not_drop_the_whole_collision() {
        // diner01 carries a zero-area sliver at triangle 317; rejecting the whole
        // build for it left the asset with no collision at all.
        let path = fnv_extracted_dir().join("meshes/architecture/diner/diner01.nif");
        if !path.is_file() {
            return;
        }
        let mut nif = NifFile::load(&path).expect("load diner fixture");
        let mut report = ConvertFileReport::default();
        regenerate_fo4_collision(&mut nif, "fnv", &mut report);

        assert!(
            nif.blocks
                .iter()
                .any(|block| block.type_name == "bhkNPCollisionObject"),
            "diner must keep its collision; warnings: {:?}",
            report.warnings
        );
        let decoded = havok_native::collision::parse_fo4_compressed_mesh(&physics_blob(&nif))
            .expect("compressed mesh");
        let total: usize = decoded
            .sections
            .iter()
            .map(|section| section.triangles.len())
            .sum();
        assert!(
            (1360..1374).contains(&total),
            "expected ~1374 source triangles minus a few slivers, got {total}"
        );
    }

    #[test]
    fn real_fnv_collision_corpus_decodes_source_shapes() {
        let root = fnv_extracted_dir().join("meshes");
        if !root.is_dir() {
            return;
        }
        let mut queue = vec![root.clone()];
        let mut paths = Vec::new();
        while let Some(dir) = queue.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    queue.push(path);
                } else if path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("nif"))
                {
                    paths.push(path);
                }
            }
            if paths.len() >= 1500 {
                break;
            }
        }
        paths.sort();

        let mut decoded = 0usize;
        let mut fell_back = 0usize;
        let mut meshes = 0usize;
        let mut well_wound = 0usize;
        let mut failures: std::collections::BTreeMap<String, usize> = Default::default();
        for path in &paths {
            let Ok(nif) = NifFile::load(path) else {
                continue;
            };
            let visible = crate::skyrim_collision::VisibleFacets::new(collect_visible_facets(&nif));
            for collision in nif
                .blocks
                .iter()
                .filter(|block| block.type_name == "bhkCollisionObject")
            {
                let Some(body_id) = field_ref(collision, "Body").filter(|id| *id >= 0) else {
                    continue;
                };
                match crate::skyrim_collision::decode_legacy_static_shape(
                    &nif,
                    body_id as usize,
                    LEGACY_HAVOK_UNIT_SCALE,
                    &visible,
                ) {
                    Ok(shape) => {
                        decoded += 1;
                        if let MultiBodyShape::CompressedMesh {
                            vertices,
                            triangles,
                        } = &shape
                        {
                            if triangles.len() >= 8 {
                                meshes += 1;
                                let (consistent, shared) =
                                    shared_edge_orientation(vertices, triangles);
                                if shared > 0 && consistent * 100 / shared >= 99 {
                                    well_wound += 1;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        fell_back += 1;
                        let key = error
                            .split(" at block ")
                            .next()
                            .unwrap_or(&error)
                            .to_string();
                        *failures.entry(key).or_default() += 1;
                    }
                }
            }
        }

        let total = decoded + fell_back;
        assert!(total > 0, "no FNV collision chains found under {root:?}");
        println!("FNV collision decode: {decoded}/{total} source shapes; fallbacks: {failures:?}");
        println!("FNV collision winding: {well_wound}/{meshes} meshes >=99% edge-consistent");
        // Source scores ~40%; orientation must make essentially all of them clean,
        // or FO4's back-face rejection leaves holes the player walks through.
        assert!(
            meshes > 0 && well_wound * 100 / meshes >= 99,
            "expected >=99% of converted FNV meshes to be consistently wound, got \
             {well_wound}/{meshes}"
        );
        // The sampled corpus decodes fully; the margin only absorbs assets a
        // differently-sliced extraction might surface.
        assert!(
            decoded * 100 / total >= 99,
            "expected >=99% of FNV collision chains to decode, got {decoded}/{total}; fallbacks: {failures:?}"
        );
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
    fn legacy_fo4_av_flags_are_cleared_from_all_scene_objects() {
        let mut nif = NifFile::default();
        for type_name in [
            "NiNode",
            "NiBillboardNode",
            "NiParticleSystem",
            "BSTriShape",
        ] {
            let mut block = NifBlock::new(nif.blocks.len(), type_name);
            block.set_field("Flags", NifValue::UInt(0x8000E));
            nif.blocks.push(block);
        }
        let mut alpha = NifBlock::new(nif.blocks.len(), "NiAlphaProperty");
        alpha.set_field("Flags", NifValue::UInt(237));
        nif.blocks.push(alpha);
        let mut report = ConvertFileReport::default();

        normalize_legacy_fo4_av_flags(&mut nif, &mut report);

        for block in &nif.blocks[..4] {
            assert_eq!(value_u64(block.get_field("Flags")), Some(0xE));
        }
        assert_eq!(value_u64(nif.blocks[4].get_field("Flags")), Some(237));
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("pre-FO4 0x80000 flag"))
        );
    }

    #[test]
    fn legacy_furniture_markers_use_fo4_position_layout() {
        let mut nif = NifFile::default();
        let mut marker = NifBlock::new(0, "BSFurnitureMarker");
        marker.set_field("Name", NifValue::String("FRN".to_string()));
        marker.set_field("Num Positions", NifValue::UInt(1));
        marker.set_field(
            "Positions",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                ("Offset".to_string(), NifValue::Vec3([1.0, 2.0, 3.0])),
                ("Orientation".to_string(), NifValue::UInt(1570)),
                ("Position Ref 1".to_string(), NifValue::UInt(11)),
                ("Position Ref 2".to_string(), NifValue::UInt(11)),
            ]))]),
        );
        nif.blocks.push(marker);
        let mut report = ConvertFileReport::default();

        normalize_legacy_furniture_markers(&mut nif, &mut report);

        let marker = &nif.blocks[0];
        assert_eq!(marker.type_name, "BSFurnitureMarkerNode");
        let NifValue::Array(positions) = marker.get_field("Positions").unwrap() else {
            panic!("expected furniture positions");
        };
        let NifValue::Struct(position) = &positions[0] else {
            panic!("expected furniture position");
        };
        assert_eq!(vec3_value(position.get("Offset")), Some([1.0, 2.0, 3.0]));
        assert_eq!(value_f64(position.get("Heading")), Some(1.57));
        assert_eq!(value_u64(position.get("Animation Type")), Some(0));
        assert_eq!(value_u64(position.get("Entry Properties")), Some(0));
        assert!(!position.contains_key("Position Ref 1"));
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("Legacy furniture markers"))
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
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);

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
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);

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
    fn unconstrained_multi_body_placement_comes_from_baked_geometry_not_body_transform() {
        // The shape vertices reaching `install_fo4_np_collision_system` are already
        // world/NIF-baked (the FO76 decode never applies `bodyCinfo.position`), so
        // an unconstrained assembly keeps `BodyMeta.position` at origin; copying
        // the source body transform would double-apply the offset.
        //
        // Two bodies at distinct pre-offset positions (a box at X≈0 and one at
        // X≈+5 Havok units) must keep those offsets in the rebuilt FO4 blob while
        // the body frame stays at origin.
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
                source_system_id: 100,
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
                source_system_id: 101,
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
    fn grouped_install_mirrors_source_physics_system_partitioning() {
        // A SCOL-style NIF: two source systems (a stairs piece with its layer-31
        // helper, and an unrelated member) must come out as TWO output physics
        // systems, with per-group Body IDs restarting at 0 — the vanilla stairs
        // pairing FO4's stair-helper binding requires. One merged system puts
        // helpers at body index 3+ and leaves the stairs unclimbable (Point
        // Pleasant SCOLs 00491AE1 / 00491B05).
        fn unit_box() -> Vec<[f32; 3]> {
            let mut v = Vec::with_capacity(8);
            for &x in &[-0.5_f32, 0.5] {
                for &y in &[-0.5_f32, 0.5] {
                    for &z in &[-0.5_f32, 0.5] {
                        v.push([x, y, z]);
                    }
                }
            }
            v
        }
        fn entry(
            source_system_id: usize,
            parent_id: usize,
            layer: u8,
            source_body_id: usize,
        ) -> CollisionPlanEntry {
            CollisionPlanEntry {
                source_collision_id: source_system_id,
                source_system_id,
                source_parent_id: parent_id,
                source_parent_name: String::new(),
                parent_id,
                parent_name: String::new(),
                planned: PlannedCollisionBody {
                    source_body_id,
                    route: CollisionRoute::SourcePolytope,
                    layer,
                    material_crc: None,
                    shape: MultiBodyShape::Polytope {
                        vertices: unit_box(),
                    },
                },
                source: None,
                source_metadata: SourceBodyMetadata::default(),
                nif_collision_intent: NifCollisionIntent::default(),
                in_multi_body_assembly: true,
                body_mass: None,
                mass_distribution: None,
            }
        }

        let mut nif = NifFile::new("fo4");
        let steps_node = nif.add_block("NiNode", None);
        let helper_node = nif.add_block("NiNode", None);
        let other_node = nif.add_block("NiNode", None);

        let mut entries = vec![
            entry(50, steps_node, 1, 0),
            entry(50, helper_node, 31, 1),
            entry(60, other_node, 1, 0),
        ];

        let installed = install_fo4_np_collision_systems_grouped(&mut nif, &mut entries)
            .expect("grouped install");
        assert_eq!(installed, 3);

        let systems: Vec<usize> = nif
            .blocks
            .iter()
            .filter(|block| block.type_name == "bhkPhysicsSystem")
            .map(|block| block.block_id)
            .collect();
        assert_eq!(systems.len(), 2, "one output system per source system");

        let mut wiring = Vec::new();
        for block in &nif.blocks {
            if block.type_name == "bhkNPCollisionObject" {
                let target = block.get_field("Target").and_then(value_usize).unwrap();
                let body_id = block.get_field("Body ID").and_then(value_usize).unwrap();
                let data = match block.get_field("Data") {
                    Some(NifValue::Ref(id)) => *id as usize,
                    other => panic!("unexpected Data field {other:?}"),
                };
                wiring.push((target, data, body_id));
            }
        }
        wiring.sort();
        assert_eq!(wiring.len(), 3);
        let (steps_sys, helper_sys, other_sys) = (wiring[0].1, wiring[1].1, wiring[2].1);
        assert_eq!(
            steps_sys, helper_sys,
            "steps and helper must share ONE system"
        );
        assert_ne!(
            steps_sys, other_sys,
            "the unrelated member must get its own system"
        );
        assert_eq!(wiring[0].2, 0, "steps body id");
        assert_eq!(wiring[1].2, 1, "helper body id");
        assert_eq!(wiring[2].2, 0, "other member restarts at body id 0");
    }

    #[test]
    fn grouped_install_rolls_back_partial_output_on_late_group_failure() {
        fn entry(
            source_system_id: usize,
            parent_id: usize,
            shape: MultiBodyShape,
        ) -> CollisionPlanEntry {
            CollisionPlanEntry {
                source_collision_id: source_system_id,
                source_system_id,
                source_parent_id: parent_id,
                source_parent_name: String::new(),
                parent_id,
                parent_name: String::new(),
                planned: PlannedCollisionBody {
                    source_body_id: 0,
                    route: CollisionRoute::SourcePolytope,
                    layer: 1,
                    material_crc: None,
                    shape,
                },
                source: None,
                source_metadata: SourceBodyMetadata::default(),
                nif_collision_intent: NifCollisionIntent::default(),
                in_multi_body_assembly: true,
                body_mass: None,
                mass_distribution: None,
            }
        }

        let cube = vec![
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [-0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
        ];
        let mut nif = NifFile::new("fo4");
        let first_parent = nif.add_block("NiNode", None);
        let second_parent = nif.add_block("NiNode", None);
        let third_parent = nif.add_block("NiNode", None);
        for parent_id in [first_parent, second_parent, third_parent] {
            nif.blocks[parent_id].set_field("Collision Object", NifValue::Ref(-1));
        }
        let initial_block_count = nif.blocks.len();
        let mut entries = vec![
            entry(
                10,
                first_parent,
                MultiBodyShape::Polytope {
                    vertices: cube.clone(),
                },
            ),
            entry(
                20,
                second_parent,
                MultiBodyShape::Polytope {
                    vertices: cube.clone(),
                },
            ),
            entry(
                20,
                third_parent,
                MultiBodyShape::CompressedMesh {
                    vertices: cube,
                    triangles: vec![[0, 1, 2]],
                },
            ),
        ];

        let error = install_fo4_np_collision_systems_grouped(&mut nif, &mut entries)
            .expect_err("the second group has invalid polytope-before-mesh order");

        assert!(error.contains("body order violation"), "{error}");
        assert_eq!(nif.blocks.len(), initial_block_count);
        assert!(nif.blocks.iter().all(|block| {
            !matches!(
                block.type_name.as_str(),
                "bhkNPCollisionObject" | "bhkPhysicsSystem"
            )
        }));
        for parent_id in [first_parent, second_parent, third_parent] {
            assert_eq!(
                field_ref(&nif.blocks[parent_id], "Collision Object"),
                Some(-1)
            );
        }
    }

    #[test]
    fn constrained_collision_preserves_source_body_frame_only_for_articulated_systems() {
        let metadata = SourceBodyMetadata {
            position: Some([1.0, 2.0, 3.0, 4.0]),
            orientation: Some([0.1, 0.2, 0.3, 0.9]),
            ..SourceBodyMetadata::default()
        };

        assert_eq!(
            np_collision_body_frame(metadata, true),
            ([1.0, 2.0, 3.0, 4.0], [0.1, 0.2, 0.3, 0.9])
        );
        assert_eq!(
            np_collision_body_frame(metadata, false),
            ([0.0; 4], [0.0, 0.0, 0.0, 1.0])
        );
    }

    #[test]
    fn collision_change_summary_reports_shape_layer_and_motion_changes() {
        let entry = CollisionPlanEntry {
            source_collision_id: 5,
            source_system_id: 6,
            source_parent_id: 42,
            source_parent_name: "CollisionParent".to_string(),
            parent_id: 42,
            parent_name: "CollisionParent".to_string(),
            planned: PlannedCollisionBody {
                source_body_id: 7,
                route: CollisionRoute::ClutterConvex,
                layer: FO4_CLUTTER_LAYER,
                material_crc: Some(0x0640_03D4),
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
                material_flags: None,
                material_trigger_type: None,
                material_crc: Some(0x1234_5678),
                body_mass: Some(2.0),
                motion_type: Some(2), // hknpMotionType::DYNAMIC
                position: None,
                orientation: None,
                has_ref_mass_distribution: true,
                is_dynamic: true,
            },
            nif_collision_intent: NifCollisionIntent {
                bsx_flags: BSX_DYNAMIC_FLAG | BSX_COMPLEX_FLAG,
                has_dynamic_bsx: true,
                has_complex_bsx: true,
                is_ground_object: false,
                is_weapon_model: false,
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
            "material 0x12345678->0x064003D4",
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
        assert_eq!(addon_node_index("AddOnNode 1078"), Some((1078, "1078")));
        assert_eq!(
            addon_node_index("AddOnNode760001"),
            Some((760001, "760001"))
        );
        assert_eq!(addon_node_index("addonnode12@#3"), Some((12, "12")));
        assert_eq!(addon_node_index("NotANode"), None);
        assert_eq!(addon_node_index("AddOnNode"), None);
    }

    #[test]
    fn patch_addon_preserves_suffix_with_empty_map() {
        let mut nif = NifFile::default();
        nif.blocks.push(bsvaluenode(0, "AddOnNode078@#0", 0));
        let mut report = ConvertFileReport::default();
        patch_addon_node_indices(&mut nif, &HashMap::new(), &mut report);
        assert_eq!(block_name(&nif.blocks[0]), "AddOnNode078@#0");
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
        assert_eq!(block_name(&nif.blocks[0]), "AddOnNode760001@#0");
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
    fn patch_addon_propagates_animation_name_references() {
        let old_name = "AddOnNode078@#0";
        let new_name = "AddOnNode760001@#0";
        let mut nif = NifFile::default();
        nif.blocks.push(bsvaluenode(0, old_name, 78));

        let mut sequence = NifBlock::new(1, "NiControllerSequence");
        sequence.set_field("Accum Root Name", NifValue::String(old_name.to_string()));
        sequence.set_field(
            "Controlled Blocks",
            NifValue::Array(vec![
                NifValue::Struct(IndexMap::from([(
                    "Node Name".to_string(),
                    NifValue::String(old_name.to_string()),
                )])),
                NifValue::Struct(IndexMap::from([(
                    "Node Name".to_string(),
                    NifValue::String("UnrelatedTarget".to_string()),
                )])),
            ]),
        );
        nif.blocks.push(sequence);

        let mut palette = NifBlock::new(2, "NiDefaultAVObjectPalette");
        palette.set_field(
            "Objs",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([
                ("Name".to_string(), NifValue::String(old_name.to_string())),
                ("AV Object".to_string(), NifValue::Ref(0)),
            ]))]),
        );
        nif.blocks.push(palette);

        let mut map = HashMap::new();
        map.insert(78, 760_001);
        let mut report = ConvertFileReport::default();
        patch_addon_node_indices(&mut nif, &map, &mut report);

        assert_eq!(block_name(&nif.blocks[0]), new_name);
        assert_eq!(
            string_field(&nif.blocks[1], "Accum Root Name").as_deref(),
            Some(new_name)
        );
        let Some(NifValue::Array(controlled)) = nif.blocks[1].get_field("Controlled Blocks") else {
            panic!("controlled blocks missing");
        };
        let controlled_names = controlled
            .iter()
            .map(|entry| match entry {
                NifValue::Struct(fields) => fields
                    .get("Node Name")
                    .and_then(nif_value_string)
                    .unwrap_or_default(),
                _ => "",
            })
            .collect::<Vec<_>>();
        assert_eq!(controlled_names, vec![new_name, "UnrelatedTarget"]);
        let Some(NifValue::Array(objects)) = nif.blocks[2].get_field("Objs") else {
            panic!("palette objects missing");
        };
        assert!(matches!(
            objects.first(),
            Some(NifValue::Struct(fields))
                if fields.get("Name").and_then(nif_value_string) == Some(new_name)
        ));
        assert!(report.changes.iter().any(|change| {
            change.contains("propagated add-on node renames to 3 animation name reference")
        }));
    }

    #[test]
    fn patch_addon_keeps_suffixed_siblings_distinct_and_retargets_each_reference() {
        let old_names = ["AddOnNode298", "AddOnNode298@#0", "AddOnNode298@#2"];
        let expected_names = [
            "AddOnNode760298",
            "AddOnNode760298@#0",
            "AddOnNode760298@#2",
        ];
        let mut nif = NifFile::default();
        for (block_id, name) in old_names.iter().enumerate() {
            nif.blocks.push(bsvaluenode(block_id, name, 298));
        }
        let mut sequence = NifBlock::new(3, "NiControllerSequence");
        sequence.set_field(
            "Controlled Blocks",
            NifValue::Array(
                old_names
                    .iter()
                    .map(|name| {
                        NifValue::Struct(IndexMap::from([(
                            "Node Name".to_string(),
                            NifValue::String((*name).to_string()),
                        )]))
                    })
                    .collect(),
            ),
        );
        nif.blocks.push(sequence);

        let mut map = HashMap::new();
        map.insert(298, 760_298);
        patch_addon_node_indices(&mut nif, &map, &mut ConvertFileReport::default());

        let names = nif.blocks[..3].iter().map(block_name).collect::<Vec<_>>();
        assert_eq!(names, expected_names);
        assert_eq!(names.iter().collect::<HashSet<_>>().len(), 3);
        let controlled_names = match nif.blocks[3].get_field("Controlled Blocks") {
            Some(NifValue::Array(entries)) => entries
                .iter()
                .map(|entry| match entry {
                    NifValue::Struct(fields) => fields
                        .get("Node Name")
                        .and_then(nif_value_string)
                        .unwrap_or_default(),
                    _ => "",
                })
                .collect::<Vec<_>>(),
            _ => panic!("controlled blocks missing"),
        };
        assert_eq!(controlled_names, expected_names);
    }

    #[test]
    fn same_game_keeps_byte_copy_behavior() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"same-game copy does not parse";
        for game in ["fo4", "fo76"] {
            let src = dir.path().join(format!("{game}-source.bin"));
            let dst = dir.path().join(format!("{game}-copy.bin"));
            std::fs::write(&src, bytes).unwrap();

            let report =
                convert_nif_file(&src, &dst, game, game, None, &ConvertFileOptions::default())
                    .unwrap();

            assert!(report.supported);
            assert_eq!(report.changes, vec!["Copied NIF without retargeting"]);
            assert_eq!(std::fs::read(dst).unwrap(), bytes);
        }
    }

    #[test]
    fn dropping_float_controller_prunes_only_its_newly_unreachable_chain() {
        let mut nif = NifFile::new("fo4");
        let shape_id = nif.add_block("BSTriShape", None);
        let shader_id = nif.add_block("BSLightingShaderProperty", None);
        let controller_id = nif.add_block("BSLightingShaderPropertyFloatController", None);
        let interpolator_id = nif.add_block("NiBlendFloatInterpolator", None);
        nif.add_block("BSShaderTextureSet", None);

        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );
        nif.blocks[shape_id].set_field("Shader Property", NifValue::Ref(shader_id as i32));
        nif.blocks[shader_id].set_field("Controller", NifValue::Ref(controller_id as i32));
        nif.blocks[controller_id].set_field("Next Controller", NifValue::Ref(-1));
        nif.blocks[controller_id].set_field("Interpolator", NifValue::Ref(interpolator_id as i32));
        nif.blocks[controller_id].set_field("Controlled Variable", NifValue::UInt(4));

        let mut report = ConvertFileReport::default();
        fix_fo76_float_controllers(&mut nif, &mut report);

        assert!(nif.blocks.iter().all(|block| {
            !matches!(
                block.type_name.as_str(),
                "BSLightingShaderPropertyFloatController" | "NiBlendFloatInterpolator"
            )
        }));
        assert!(
            nif.blocks
                .iter()
                .any(|block| block.type_name == "BSShaderTextureSet"),
            "a source-owned detached block must not be globally pruned"
        );
        let shader = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSLightingShaderProperty")
            .expect("shader");
        assert_eq!(field_ref(shader, "Controller"), Some(-1));
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
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);

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
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
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
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
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

    #[test]
    fn tall_grass_shader_matches_fo4_grass_render_state() {
        let mut nif = NifFile::new("fnv");
        let mut source = NifBlock::new(0, "TallGrassShaderProperty");
        source.set_field(
            "File Name",
            NifValue::String("textures\\landscape\\grass\\GrassWastelandComp01.dds".into()),
        );

        let shader_id = convert_tall_grass(&mut nif, &source);
        let shader = nif.get_block(shader_id).expect("converted grass shader");

        assert_eq!(
            value_u64(shader.get_field("Shader Flags 1")),
            Some(FO4_TALL_GRASS_SHADER_FLAGS_1)
        );
        assert_eq!(
            value_u64(shader.get_field("Shader Flags 2")),
            Some(FO4_TALL_GRASS_SHADER_FLAGS_2)
        );
        assert_eq!(
            value_u64(shader.fields.get("Shader Flags 1:FO4")),
            Some(FO4_TALL_GRASS_SHADER_FLAGS_1)
        );
        assert_eq!(
            value_u64(shader.fields.get("Shader Flags 2:FO4")),
            Some(FO4_TALL_GRASS_SHADER_FLAGS_2)
        );
        assert_eq!(value_u64(shader.get_field("Texture Clamp Mode")), Some(3));
        let Some(NifValue::Struct(uv_scale)) = shader.get_field("UV Scale") else {
            panic!("expected UV Scale");
        };
        assert_eq!(value_f64(uv_scale.get("u")), Some(1.0));
        assert_eq!(value_f64(uv_scale.get("v")), Some(1.0));
        assert_eq!(value_f64(shader.get_field("Smoothness")), Some(0.282));
    }

    #[test]
    fn pp_lighting_shader_serializes_fo4_flags_and_uv_scale() {
        let mut nif = NifFile::new("fo4");
        let mut source = NifBlock::new(99, "BSShaderPPLightingProperty");
        source.set_field(
            "Shader Flags",
            NifValue::Array(vec![
                NifValue::String("Specular".to_string()),
                NifValue::String("ZBuffer_Test".to_string()),
            ]),
        );
        source.set_field(
            "Shader Flags 2",
            NifValue::Array(vec![NifValue::String("ZBuffer_Write".to_string())]),
        );

        let shader_id = convert_pp_lighting(&mut nif, &source);
        nif.rebuild_header();
        let bytes = nif.to_bytes().expect("serialize pp-lighting shader");
        let reparsed = NifFile::from_bytes(&bytes, None).expect("reparse pp-lighting shader");
        let shader = reparsed.get_block(shader_id).expect("converted shader");

        assert_eq!(
            value_u64(shader.get_field("Shader Flags 1")),
            Some(SLSF1_SPECULAR | SLSF1_ZBUFFER_TEST)
        );
        assert_eq!(
            value_u64(shader.get_field("Shader Flags 2")),
            Some(SLSF2_ZBUFFER_WRITE as u64)
        );
        let Some(NifValue::Struct(uv_scale)) = shader.get_field("UV Scale") else {
            panic!("expected UV Scale");
        };
        assert_eq!(value_f64(uv_scale.get("u")), Some(1.0));
        assert_eq!(value_f64(uv_scale.get("v")), Some(1.0));
        assert_eq!(
            value_f64(shader.get_field("Rimlight Power")),
            Some(f32::MAX as f64)
        );
    }

    #[test]
    fn legacy_grass_root_drops_orphan_high_flag_companion() {
        let mut nif = NifFile::new("fnv");
        nif.blocks[0].type_name = "BSFadeNode".to_string();
        nif.blocks[0].set_field("Flags", NifValue::UInt(0x0008_000e));
        let mut report = ConvertFileReport::default();

        normalize_fo4_root_node(&mut nif, None, false, false, &mut report);

        assert_eq!(nif.blocks[0].type_name, "NiNode");
        assert_eq!(value_u64(nif.blocks[0].get_field("Flags")), Some(0x000e));
    }

    #[test]
    fn legacy_float_vertex_color_becomes_compact_color() {
        let source = NifValue::Struct(IndexMap::from([
            ("r".to_string(), NifValue::Float(0.5)),
            ("g".to_string(), NifValue::Float(0.25)),
            ("b".to_string(), NifValue::Float(0.75)),
            ("a".to_string(), NifValue::Float(0.125)),
        ]));

        let Some(NifValue::Color4(color)) = color4_value(&source) else {
            panic!("expected compact color");
        };
        assert_eq!(color, [0.5, 0.25, 0.75, 0.125]);
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

    fn write_fo76_named_glow_bgsm(dir: &Path, relative: &str) {
        let mut bgsm = materials_native::bgsm::BgsmData {
            DiffuseTexture: "SetDressing/AutoDispenser/AutoDispenserAmmo_d.dds".to_owned(),
            NormalTexture: "SetDressing/AutoDispenser/AutoDispenserAmmo_n.dds".to_owned(),
            LightingTexture: Some("SetDressing/AutoDispenser/AutoDispenserAmmo_l.dds".to_owned()),
            EmitEnabled: true,
            EmittanceColor: Some([1.0, 0.9568628, 0.43529415]),
            EmittanceMult: 10.0,
            ..Default::default()
        };
        bgsm.header.signature = materials_native::bgsm::BGSM_SIGNATURE;
        bgsm.header.version = 20;
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, materials_native::bgsm::write(&bgsm)).unwrap();
    }

    fn write_fo76_ultracite_bgsm(dir: &Path, relative: &str) {
        let mut bgsm = materials_native::bgsm::BgsmData {
            DiffuseTexture: "Landscape/Plants/Mineral_Ultracite01_d.dds".to_owned(),
            NormalTexture: "Landscape/Plants/Mineral_Ultracite01_n.dds".to_owned(),
            LightingTexture: Some("Landscape/Plants/Mineral_Ultracite01_l.dds".to_owned()),
            EmitEnabled: true,
            EmittanceColor: Some([0.8196079, 0.854902, 0.17254902]),
            EmittanceMult: 3.0,
            ..Default::default()
        };
        bgsm.header.signature = materials_native::bgsm::BGSM_SIGNATURE;
        bgsm.header.version = 20;
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, materials_native::bgsm::write(&bgsm)).unwrap();
    }

    fn write_decal_bgsm(dir: &Path, relative: &str) {
        let mut bgsm = materials_native::bgsm::BgsmData::default();
        bgsm.header.signature = materials_native::bgsm::BGSM_SIGNATURE;
        bgsm.header.version = 2;
        bgsm.header.decal = true;
        bgsm.header.decal_nofade = true;
        bgsm.header.two_sided = true;
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, materials_native::bgsm::write(&bgsm)).unwrap();
    }

    fn write_emittance_bgsm(dir: &Path, relative: &str, emit_enabled: bool) {
        let mut bgsm = materials_native::bgsm::BgsmData {
            DiffuseTexture: "TestGlow/Board_d.dds".to_owned(),
            LightingTexture: Some("TestGlow/Board_l.dds".to_owned()),
            EmitEnabled: emit_enabled,
            ..Default::default()
        };
        bgsm.header.signature = materials_native::bgsm::BGSM_SIGNATURE;
        bgsm.header.version = 20;
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, materials_native::bgsm::write(&bgsm)).unwrap();
    }

    fn external_bgsm_emissive_controller_nif(material_name: &str) -> NifFile {
        let mut nif = external_bgsm_shader_nif(material_name);
        nif.blocks[0].set_field("Controller", NifValue::Ref(2));
        let mut controller = NifBlock::new(2, "BSLightingShaderPropertyColorController");
        controller.set_field("Next Controller", NifValue::Ref(-1));
        controller.set_field("Flags", NifValue::UInt(76));
        controller.set_field("Target", NifValue::Ref(0));
        controller.set_field("Interpolator", NifValue::Ref(3));
        // LSCC_EMISSIVE_COLOR; Specular Color is 0.
        controller.set_field("Controlled Color", NifValue::UInt(1));
        nif.blocks.push(controller);
        let mut interpolator = NifBlock::new(3, "NiPoint3Interpolator");
        interpolator.set_field("Value", NifValue::Vec3([f32::MIN; 3]));
        interpolator.set_field("Data", NifValue::Ref(4));
        nif.blocks.push(interpolator);
        nif.blocks.push(NifBlock::new(4, "NiPosData"));
        nif
    }

    #[test]
    fn dormant_emissive_controller_is_pinned_black_for_non_emitting_material() {
        let dir = tempfile::tempdir().unwrap();
        write_emittance_bgsm(dir.path(), "materials/testglow/darkboard.bgsm", false);
        write_emittance_bgsm(dir.path(), "materials/testglow/litboard.bgsm", true);

        // FO76 gates emittance on the BGSM, so a white colour controller on an
        // EmitEnabled=false material is dead data there. FO4 has no such gate.
        let mut nif = external_bgsm_emissive_controller_nif("Materials\\TestGlow\\DarkBoard.bgsm");
        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            Some(dir.path()),
            &HashMap::new(),
            &mut report,
        );
        assert_eq!(
            nif.blocks[3].get_field("Value"),
            Some(&NifValue::Vec3([0.0; 3])),
            "dormant emissive interpolator must be pinned to black"
        );
        assert_eq!(
            field_ref(&nif.blocks[3], "Data"),
            Some(-1),
            "dormant emissive interpolator must stop reading its key data"
        );
        assert!(
            value_u64(nif.blocks[0].get_field("Shader Flags 1"))
                .is_some_and(|flags| flags & SLSF1_OWN_EMIT != 0),
            "neutralizing the controller must not disturb the Own_Emit baseline"
        );
        assert_eq!(
            nif.blocks.len(),
            5,
            "no blocks may be removed: renumbering desyncs NiControllerSequence arrays"
        );

        // EmitEnabled=true → the animated emissive colour is real, keep it.
        let mut nif = external_bgsm_emissive_controller_nif("Materials\\TestGlow\\LitBoard.bgsm");
        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            Some(dir.path()),
            &HashMap::new(),
            &mut report,
        );
        assert_eq!(
            field_ref(&nif.blocks[3], "Data"),
            Some(4),
            "an emissive material must keep its animated colour"
        );
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
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
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
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
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
                .is_some_and(|flags| flags & SLSF1_OWN_EMIT != 0),
            "non-glow external BGSM must retain FO4 Own_Emit"
        );

        // No source dir → behavior unchanged (flag off, slot 2 empty).
        let mut nif = external_bgsm_shader_nif("Materials\\TestGlow\\GlowBoard.bgsm");
        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
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
    fn external_fo76_named_glow_material_uses_masked_glow_shader() {
        let dir = tempfile::tempdir().unwrap();
        write_fo76_named_glow_bgsm(
            dir.path(),
            "materials/setdressing/autodispenser/autodispenserammo_glow.bgsm",
        );

        let mut nif = external_bgsm_shader_nif(
            "Materials\\SetDressing\\AutoDispenser\\AutoDispenserAmmo_Glow.bgsm",
        );
        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            Some(dir.path()),
            &HashMap::new(),
            &mut report,
        );

        assert!(
            value_u64(nif.blocks[0].get_field("Shader Flags 2"))
                .is_some_and(|flags| flags & SLSF2_GLOW_MAP != 0)
        );
        assert_eq!(
            value_u64(nif.blocks[0].get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_GLOW)
        );
        assert_eq!(
            texture_at(&nif.blocks[1], 2).to_ascii_lowercase(),
            "textures\\setdressing\\autodispenser\\autodispenserammo_g.dds"
        );
    }

    #[test]
    fn external_fo76_ultracite_material_uses_masked_glow_shader() {
        let dir = tempfile::tempdir().unwrap();
        write_fo76_ultracite_bgsm(
            dir.path(),
            "materials/landscape/plants/mineral_ultracite01.bgsm",
        );

        let mut nif =
            external_bgsm_shader_nif("Materials\\Landscape\\Plants\\Mineral_Ultracite01.bgsm");
        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            Some(dir.path()),
            &HashMap::new(),
            &mut report,
        );

        assert!(
            value_u64(nif.blocks[0].get_field("Shader Flags 2"))
                .is_some_and(|flags| flags & SLSF2_GLOW_MAP != 0)
        );
        assert_eq!(
            value_u64(nif.blocks[0].get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_GLOW)
        );
        assert_eq!(
            texture_at(&nif.blocks[1], 2).to_ascii_lowercase(),
            "textures\\landscape\\plants\\mineral_ultracite01_g.dds"
        );
    }

    #[test]
    fn external_fo76_mothman_eye_materials_use_masked_glow_shader() {
        let dir = tempfile::tempdir().unwrap();
        for (name, emits) in [
            ("Mothman", true),
            ("Mothman01", true),
            ("Mothman02", true),
            ("MothmanWise", true),
            ("MothmanUltracite", true),
            ("MothmanGlow", true),
            ("MothmanWingGlow", true),
            ("MothmanNoEmit", false),
            ("MothmanWing", false),
        ] {
            let path = format!("materials/actors/mothman/{name}.bgsm");
            write_emittance_bgsm(dir.path(), &path, emits);
            let mut nif = external_bgsm_shader_nif(&path);
            let mut report = ConvertFileReport::default();
            ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
            normalize_external_bgsm_shader_data_with_overrides(
                &mut nif,
                Some(dir.path()),
                &HashMap::new(),
                &mut report,
            );
            assert_eq!(
                value_u64(nif.blocks[0].get_field("Shader Flags 2"))
                    .is_some_and(|flags| flags & SLSF2_GLOW_MAP != 0),
                emits,
                "{name}"
            );
            assert_eq!(
                value_u64(nif.blocks[0].get_field("Shader Type")),
                Some(if emits {
                    BSLSP_SHADER_TYPE_GLOW
                } else {
                    BSLSP_SHADER_TYPE_DEFAULT
                }),
                "{name}"
            );
            assert_eq!(
                texture_at(&nif.blocks[1], 2),
                if emits {
                    "textures\\TestGlow\\Board_g.dds"
                } else {
                    ""
                },
                "{name}"
            );
        }
    }

    #[test]
    fn external_bgsm_decal_material_restores_fo4_shader_flags() {
        let dir = tempfile::tempdir().unwrap();
        write_decal_bgsm(
            dir.path(),
            "materials/setdressing/nukaworldprops/signdecals01.bgsm",
        );

        let mut nif =
            external_bgsm_shader_nif("Materials\\SetDressing\\NukaWorldProps\\SignDecals01.bgsm");
        let mut report = ConvertFileReport::default();
        ensure_fo4_lighting_shader_defaults(&mut nif, &mut report, false);
        normalize_external_bgsm_shader_data_with_overrides(
            &mut nif,
            Some(dir.path()),
            &HashMap::new(),
            &mut report,
        );

        let flags1 = value_u64(nif.blocks[0].get_field("Shader Flags 1")).unwrap();
        let flags2 = value_u64(nif.blocks[0].get_field("Shader Flags 2")).unwrap();
        assert_ne!(flags1 & SLSF1_DECAL, 0);
        assert_ne!(flags1 & SLSF1_DYNAMIC_DECAL, 0);
        assert_ne!(flags2 & SLSF2_DOUBLE_SIDED as u64, 0);
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
        normalize_fo76_environment_mapping(&mut nif, &mut report);

        let shader = &nif.blocks[0];
        assert_eq!(
            value_u64(shader.get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_DEFAULT)
        );
        assert_eq!(value_u64(shader.get_field("Shader Flags 1")), Some(0));
    }

    #[test]
    fn environment_mapping_with_cubemap_promotes_shader_type() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field("Texture Set", NifValue::Ref(1));
        shader.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_DEFAULT));
        shader.set_field("Shader Flags 1", NifValue::UInt(SLSF1_ENVIRONMENT_MAPPING));
        let mut texset = NifBlock::new(1, "BSShaderTextureSet");
        let mut textures = vec![NifValue::String(String::new()); 9];
        textures[4] = NifValue::String("textures\\Shared\\Cubemaps\\Cube01_e.dds".to_string());
        texset.set_field("Textures", NifValue::Array(textures));
        nif.blocks.push(shader);
        nif.blocks.push(texset);

        let mut report = ConvertFileReport::default();
        normalize_fo76_environment_mapping(&mut nif, &mut report);

        let shader = &nif.blocks[0];
        assert_eq!(
            value_u64(shader.get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_ENVIRONMENT_MAP)
        );
        assert_eq!(
            value_u64(shader.get_field("Shader Flags 1")),
            Some(SLSF1_ENVIRONMENT_MAPPING)
        );
        // Type 1 turns on the cond-gated tail; missing defaults serialize short.
        assert_eq!(
            value_f64(shader.get_field("Environment Map Scale")),
            Some(1.0)
        );
        assert!(shader.get_field("Use Screen Space Reflections").is_some());
        assert!(shader.fields.contains_key("Wetness Control: Use SSR"));
    }

    #[test]
    fn environment_mapping_on_specialized_shader_type_drops_the_flag() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field("Texture Set", NifValue::Ref(1));
        shader.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_GLOW));
        shader.set_field("Shader Flags 1", NifValue::UInt(SLSF1_ENVIRONMENT_MAPPING));
        let mut texset = NifBlock::new(1, "BSShaderTextureSet");
        let mut textures = vec![NifValue::String(String::new()); 9];
        textures[4] = NifValue::String("textures\\Shared\\Cubemaps\\Cube01_e.dds".to_string());
        texset.set_field("Textures", NifValue::Array(textures));
        nif.blocks.push(shader);
        nif.blocks.push(texset);

        let mut report = ConvertFileReport::default();
        normalize_fo76_environment_mapping(&mut nif, &mut report);

        let shader = &nif.blocks[0];
        assert_eq!(
            value_u64(shader.get_field("Shader Type")),
            Some(BSLSP_SHADER_TYPE_GLOW)
        );
        assert_eq!(value_u64(shader.get_field("Shader Flags 1")), Some(0));
    }

    #[test]
    fn environment_mapping_without_texture_set_clears_the_flag() {
        let mut nif = NifFile::default();
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field("Shader Type", NifValue::UInt(BSLSP_SHADER_TYPE_DEFAULT));
        shader.set_field("Shader Flags 1", NifValue::UInt(SLSF1_ENVIRONMENT_MAPPING));
        nif.blocks.push(shader);

        let mut report = ConvertFileReport::default();
        normalize_fo76_environment_mapping(&mut nif, &mut report);

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
