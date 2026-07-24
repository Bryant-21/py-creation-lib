//! Filtered `_OC.nif` writer for one baked precombine model group (v0 spike).
//!
//! Reads a source STAT model's single eligible inline render shape, packs its
//! geometry plus one combined-transform row per REFR instance into a
//! `BSPackedCombinedGeomDataExtra`, and writes the output NIF. Read-only on
//! the source ESP; no ESP mutation happens here (see `precombine::stamp`).
//!
//! Plan: `docs/superpowers/plans/2026-07-12-precombine-generation-v0.md` Task 3.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use bsarchive_native::{Reader as _, fo4};
use indexmap::IndexMap;
use nif_core_native::model::{NifBlock, NifFile, NifValue};

use crate::precombine::plan::{CellPlan, InstanceRef, ModelGroup, Params};

/// CK Filtered precombine root (`BSFadeNode`) flags. Real CK samples vary in
/// bit 0x1000 (one sample carries 0x400E, another 0x500E); 0x400E (16398) is
/// the common-denominator choice. Filtered roots carry no BSXFlags/Havok
/// extra data; that belongs to the separate `<cell>_Physics.NIF`, never the
/// render `_OC.nif` (a prior version of this bake wrongly emitted a BSXFlags
/// block here, copied from a Clean-variant oracle sample).
const ROOT_FLAGS: u64 = 0x400E;
/// CK Filtered precombine inner `NiNode` flags — invariant across samples.
const INNER_FLAGS: u64 = 14;
/// CK Filtered shapes set this on the shape's (single, plain) `BSTriShape` —
/// v0 only ever emits plain `BSTriShape`, never
/// `BSDynamicTriShape`/`BSSubIndexTriShape`.
const SHAPE_FLAGS: u64 = 526;

pub struct BakedMesh {
    pub mesh_id: u32,
    pub refs: Vec<u32>,
    pub rel_path: String,
}

pub struct BakedCell {
    pub cell_form_id: u32,
    pub meshes: Vec<BakedMesh>,
}

/// A possibly-successful bake. A per-group failure is isolated to a warning;
/// a cell with zero successfully baked meshes is not stampable (`baked` is
/// `None`), matching the plan's "never stamp a zero-mesh cell" invariant.
pub struct BakeReport {
    pub baked: Option<BakedCell>,
    pub warnings: Vec<String>,
}

pub fn bake_cell(plan: &CellPlan, params: &Params) -> BakeReport {
    let mut warnings = Vec::new();
    let mut meshes = Vec::new();
    let mut missing_mesh_paths: Vec<PathBuf> = Vec::new();
    let mut used_mesh_ids: HashSet<u32> = HashSet::new();
    let archives = open_mesh_archives(&params.mesh_archives);

    for group in &plan.groups {
        match bake_group(plan.cell_form_id, group, params, &mut used_mesh_ids, &archives) {
            Ok(mesh) => meshes.push(mesh),
            Err(reason) => {
                let source_path = resolve_data_relative_path(&params.data_root, &group.model_path);
                if !source_path.is_file() {
                    missing_mesh_paths.push(source_path);
                }
                warnings.push(format!(
                    "group {:?} ({} refs) skipped: {reason}",
                    group.model_path,
                    group.instances.len()
                ));
            }
        }
    }

    // A zero-mesh cell caused entirely (or partly) by unresolved loose-file
    // meshes is easy to mistake for "the cell had nothing eligible" — make
    // the BA2/wrong-path cause unambiguous instead of leaving a silent no-op.
    if meshes.is_empty() && !missing_mesh_paths.is_empty() {
        warnings.push(zero_mesh_missing_files_summary(
            plan.cell_form_id,
            &missing_mesh_paths,
        ));
    }

    let baked = if meshes.is_empty() {
        None
    } else {
        Some(BakedCell {
            cell_form_id: plan.cell_form_id,
            meshes,
        })
    };
    BakeReport { baked, warnings }
}

/// Caps the listed paths so one badly-configured cell with hundreds of
/// missing meshes can't flood the report; the count still names the total.
const MISSING_MESH_PATHS_SHOWN: usize = 10;

fn zero_mesh_missing_files_summary(cell_form_id: u32, missing: &[PathBuf]) -> String {
    let shown: Vec<String> = missing
        .iter()
        .take(MISSING_MESH_PATHS_SHOWN)
        .map(|path| path.display().to_string())
        .collect();
    let mut message = format!(
        "cell {cell_form_id:08X} produced zero meshes: {} of its group(s) failed because the \
         source mesh was not found on disk as a loose file, in any configured \
         mesh_extract_roots, or in any configured mesh_archives. Missing: {}",
        missing.len(),
        shown.join(", "),
    );
    if missing.len() > MISSING_MESH_PATHS_SHOWN {
        message.push_str(&format!(", and {} more", missing.len() - MISSING_MESH_PATHS_SHOWN));
    }
    message
}

fn bake_group(
    cell_form_id: u32,
    group: &ModelGroup,
    params: &Params,
    used_mesh_ids: &mut HashSet<u32>,
    archives: &[MeshArchive],
) -> Result<BakedMesh, String> {
    let source_nif = resolve_source_nif(group, params, archives)?;

    let shape_block_ids = eligible_shape_block_ids(&source_nif)?;

    let mesh_id = mint_mesh_id(cell_form_id, &group.instances, used_mesh_ids);
    let cell8 = cell_form_id & 0x00FF_FFFF;
    let file_name = format!("{cell8:08X}_{mesh_id:08X}_OC.nif");
    let rel_path = format!("meshes\\precombined\\{}\\{file_name}", params.plugin_name);
    let dest_path = {
        let mut p = params.data_root.clone();
        p.push("meshes");
        p.push("precombined");
        p.push(&params.plugin_name);
        p.push(&file_name);
        p
    };

    let mut out = NifFile::new("fo4");
    // `NifFile::new` already builds a BSFadeNode root at block 0 (see
    // `ROOT_FLAGS` above) — reuse it instead of dropping it and building a
    // fresh NiNode, which was the previous (wrong) behavior.
    let root_id = 0;
    let inner_id = out.add_block("NiNode", None);

    // One shape+PCD(+shader+texset+alpha) group per eligible source shape
    // (CK Filtered convention: one pair per material), all packed into this
    // same `_OC.nif`. `bake_shape` runs the exact per-shape body v0 ran once,
    // now called per shape via `?` — any single unsupported shape (skin,
    // controller, unsupported shader/texset/alpha, u16 overflow) still sinks
    // the WHOLE group, preserving the "no partial geometry per reference"
    // invariant.
    let mut shape_ids = Vec::with_capacity(shape_block_ids.len());
    for shape_block_id in &shape_block_ids {
        let shape_id = bake_shape(&source_nif, *shape_block_id, group, &mut out)?;
        shape_ids.push(shape_id);
    }

    out.blocks[inner_id].set_field("Flags", NifValue::UInt(INNER_FLAGS));
    out.blocks[inner_id].set_field("Num Children", NifValue::UInt(shape_ids.len() as u64));
    out.blocks[inner_id].set_field(
        "Children",
        NifValue::Array(shape_ids.iter().map(|&id| NifValue::Ref(id as i32)).collect()),
    );

    let root_name = format!("{cell8:08X}_{mesh_id:08X}_OC");
    out.blocks[root_id].set_field("Name", NifValue::String(root_name));
    out.blocks[root_id].set_field("Flags", NifValue::UInt(ROOT_FLAGS));
    out.blocks[root_id].set_field("Num Children", NifValue::UInt(1));
    out.blocks[root_id].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(inner_id as i32)]),
    );
    out.header.footer_roots = vec![root_id as i32];

    write_atomic(&mut out, &dest_path)?;

    if let Err(error) = NifFile::load(&dest_path) {
        let _ = std::fs::remove_file(&dest_path);
        return Err(format!(
            "baked nif {} failed to reload: {error}",
            dest_path.display()
        ));
    }

    Ok(BakedMesh {
        mesh_id,
        refs: group.instances.iter().map(|i| i.refr_form_id).collect(),
        rel_path,
    })
}

/// Bakes one source shape into `out`: validates its property chain, packs
/// its geometry plus one Combined row per instance, and emits the
/// shape+PCD(+shader+texset+alpha) block group — the per-shape body v0 ran
/// once, now called per eligible shape from `bake_group`'s loop. Block
/// emission order per shape (CK Filtered convention): shape, then PCD, then
/// shader, then texset, then alpha.
fn bake_shape(
    source_nif: &NifFile,
    shape_block_id: usize,
    group: &ModelGroup,
    out: &mut NifFile,
) -> Result<usize, String> {
    let shape = &source_nif.blocks[shape_block_id];

    if !ref_is_none(shape.get_field("Skin")) {
        return Err("skinned shapes are unsupported in v0".to_string());
    }
    if !ref_is_none(shape.get_field("Controller")) {
        return Err("controller-animated shapes are unsupported in v0".to_string());
    }

    let shader_block_id = resolve_ref_block(source_nif, shape, "Shader Property")
        .ok_or_else(|| "shape has no resolvable Shader Property".to_string())?;
    let shader = &source_nif.blocks[shader_block_id];
    if shader.type_name != "BSLightingShaderProperty" {
        return Err(format!("unsupported shader chain: {}", shader.type_name));
    }
    let texset_block_id = resolve_ref_block(source_nif, shader, "Texture Set")
        .ok_or_else(|| "shader has no resolvable Texture Set".to_string())?;
    if source_nif.blocks[texset_block_id].type_name != "BSShaderTextureSet" {
        return Err(format!(
            "unsupported texture set type: {}",
            source_nif.blocks[texset_block_id].type_name
        ));
    }

    let alpha_block_id = match resolve_ref_block(source_nif, shape, "Alpha Property") {
        Some(id) if source_nif.blocks[id].type_name == "NiAlphaProperty" => Some(id),
        Some(id) => {
            return Err(format!(
                "unsupported alpha property: {}",
                source_nif.blocks[id].type_name
            ));
        }
        None => None,
    };

    // CK Filtered `_OC` shapes and the top-level PCD `Vertex Desc` ALWAYS
    // declare Full_Precision, regardless of the source's actual precision —
    // the engine unconditionally up-converts half-precision storage into a
    // runtime buffer sized from the SHAPE desc's stride, so a half shape
    // desc under-allocates 8 bytes/vertex and heap-overruns. The inner
    // Object Data desc (and its inline `Vertex Data` bytes) stays exactly
    // as read from the source — v0 never repacks vertex storage — so the
    // shape/top-level descs and the inner desc intentionally diverge; see
    // `promote_to_full_precision` below.
    let vertex_desc = shape
        .get_field("Vertex Desc")
        .cloned()
        .ok_or_else(|| "shape has no Vertex Desc".to_string())?;
    let full_precision_vertex_desc =
        NifValue::Int(promote_to_full_precision(vertex_desc.as_i64().max(0) as u64) as i64);
    let vertex_data = match shape.get_field("Vertex Data") {
        Some(NifValue::Array(items)) if !items.is_empty() => items.clone(),
        _ => return Err("shape has no Vertex Data".to_string()),
    };
    let triangles = match shape.get_field("Triangles") {
        Some(NifValue::Array(items)) if !items.is_empty() => items.clone(),
        _ => return Err("shape has no Triangles".to_string()),
    };
    let num_verts = vertex_data.len() as u64;
    let num_triangles = triangles.len() as u64;

    // CK stores instance-EXPANDED counts (per-instance count x number of
    // instances) in the shape block and the PCD top level; only the inner
    // Object Data keeps the deduplicated per-instance counts (v0 bakes one
    // source mesh placed N times, PCD NumData=1). The engine sizes its
    // combined runtime buffers directly from the stored top-level counts,
    // then writes instance-expanded geometry into them, so deduplicated
    // counts there under-allocate and heap-overrun in-game.
    let num_instances = group.instances.len() as u64;
    let expanded_verts = num_verts * num_instances;
    let expanded_triangles = num_triangles * num_instances;
    // The shape's Num Vertices/Num Triangles fields are u16; CK's observed
    // per-file max is 65,444. Reject the group loudly rather than silently
    // truncating or overflowing on a hotter cell.
    if expanded_verts > u64::from(u16::MAX) || expanded_triangles > u64::from(u16::MAX) {
        return Err(format!(
            "instance-expanded counts exceed the shape's u16 capacity: {expanded_verts} \
             vertices / {expanded_triangles} triangles from {num_verts} verts x \
             {num_triangles} triangles x {num_instances} instances (max 65535)"
        ));
    }

    let source_bound = read_bound(shape.get_field("Bounding Sphere"))
        .ok_or_else(|| "shape has no Bounding Sphere".to_string())?;
    let grayscale = source_grayscale_to_palette_scale(shader);

    // `group.instances` is already sorted by `refr_form_id` (see plan::build_plan),
    // so `Combined` rows come out deterministically ordered for free.
    let mut combined_rows = Vec::with_capacity(group.instances.len());
    let mut transformed_bounds = Vec::with_capacity(group.instances.len());
    for instance in &group.instances {
        let rotation = euler_to_matrix33(instance.rotation);
        let (bound_center, bound_radius) =
            transform_bound(source_bound, rotation, instance.position, instance.scale);
        transformed_bounds.push((bound_center, bound_radius));
        // NIF Matrix33 wire order is column-contiguous, but nif_core's
        // writer serializes the `[[f32;3];3]` array in [row][col]
        // (row-contiguous) memory order — so the Combined row's stored
        // rotation must be the TRANSPOSE of the true rotation, or the
        // engine reconstructs the inverse rotation on load. `transform_bound`
        // above (and `aggregate_cull_sphere`) must keep using the true
        // `rotation` for world-space bound math — only this Combined-row
        // write needs the transpose.
        combined_rows.push(packed_geom_data_combined(
            grayscale,
            transpose3(rotation),
            instance.position,
            instance.scale,
            bound_center,
            bound_radius,
        ));
    }
    let (aggregate_center, aggregate_radius) = aggregate_cull_sphere(&transformed_bounds);

    let shape_id = out.add_block("BSTriShape", None);

    let object_data = NifValue::Struct(struct_fields([
        ("Num Verts", NifValue::UInt(num_verts)),
        ("LOD Levels", NifValue::UInt(3)),
        ("Tri Count LOD0", NifValue::UInt(num_triangles)),
        ("Tri Offset LOD0", NifValue::UInt(0)),
        ("Tri Count LOD1", NifValue::UInt(0)),
        ("Tri Offset LOD1", NifValue::UInt(0)),
        ("Tri Count LOD2", NifValue::UInt(0)),
        ("Tri Offset LOD2", NifValue::UInt(0)),
        ("Num Combined", NifValue::UInt(combined_rows.len() as u64)),
        ("Combined", NifValue::Array(combined_rows)),
        ("Vertex Desc", vertex_desc.clone()),
        ("Vertex Data", NifValue::Array(vertex_data)),
        ("Triangles", NifValue::Array(triangles)),
    ]));
    let pcd_id = out.add_block(
        "BSPackedCombinedGeomDataExtra",
        Some(struct_fields([
            ("Name", NifValue::String("PCD".to_string())),
            ("Vertex Desc", full_precision_vertex_desc.clone()),
            ("Num Vertices", NifValue::UInt(expanded_verts)),
            ("Num Triangles", NifValue::UInt(expanded_triangles)),
            ("Unknown Flags 1", NifValue::UInt(0)),
            ("Unknown Flags 2", NifValue::UInt(0)),
            ("Num Data", NifValue::UInt(1)),
            ("Object Data", NifValue::Array(vec![object_data])),
        ])),
    );

    let shader_clone_id = out.add_block("BSLightingShaderProperty", None);
    out.blocks[shader_clone_id].fields = source_nif.blocks[shader_block_id].fields.clone();
    strip_controller(&mut out.blocks[shader_clone_id]);

    let texset_clone_id = out.add_block("BSShaderTextureSet", None);
    out.blocks[texset_clone_id].fields = source_nif.blocks[texset_block_id].fields.clone();
    strip_controller(&mut out.blocks[texset_clone_id]);

    out.blocks[shader_clone_id].set_field("Texture Set", NifValue::Ref(texset_clone_id as i32));

    let alpha_clone_id = alpha_block_id.map(|id| {
        let new_id = out.add_block("NiAlphaProperty", None);
        out.blocks[new_id].fields = source_nif.blocks[id].fields.clone();
        strip_controller(&mut out.blocks[new_id]);
        new_id
    });

    out.blocks[shape_id].set_field("Name", NifValue::String(String::new()));
    out.blocks[shape_id].set_field("Translation", NifValue::Vec3(aggregate_center));
    out.blocks[shape_id].set_field("Skin", NifValue::Ref(-1));
    out.blocks[shape_id].set_field("Shader Property", NifValue::Ref(shader_clone_id as i32));
    out.blocks[shape_id].set_field(
        "Alpha Property",
        NifValue::Ref(alpha_clone_id.map(|id| id as i32).unwrap_or(-1)),
    );
    out.blocks[shape_id].set_field("Vertex Desc", full_precision_vertex_desc);
    // CK Filtered shapes carry the instance-EXPANDED vertex/triangle counts
    // (see `expanded_verts`/`expanded_triangles` above) even though Data
    // Size stays 0 — the counts are independent of dataSize, and the
    // engine's PCD unpacker sizes its runtime buffers from these counts, so
    // deduplicated (or 0/0) counts here under-allocate into a buffer-overrun
    // crash (nif_core's writer still forces DataSize=0 for a shape with no
    // inline vertex/triangle arrays, so this is byte-safe: the block stays
    // 122 bytes).
    out.blocks[shape_id].set_field("Num Vertices", NifValue::UInt(expanded_verts));
    out.blocks[shape_id].set_field("Num Triangles", NifValue::UInt(expanded_triangles));
    out.blocks[shape_id].set_field("Data Size", NifValue::UInt(0));
    out.blocks[shape_id].set_field("Flags", NifValue::UInt(SHAPE_FLAGS));
    out.blocks[shape_id].set_field(
        "Bounding Sphere",
        NifValue::Struct(struct_fields([
            ("Center", NifValue::Vec3([0.0, 0.0, 0.0])),
            ("Radius", NifValue::Float(f64::from(aggregate_radius))),
        ])),
    );
    out.blocks[shape_id].set_field("Num Extra Data List", NifValue::UInt(1));
    out.blocks[shape_id].set_field(
        "Extra Data List",
        NifValue::Array(vec![NifValue::Ref(pcd_id as i32)]),
    );

    Ok(shape_id)
}

fn write_atomic(nif: &mut NifFile, dest_path: &Path) -> Result<(), String> {
    let parent = dest_path
        .parent()
        .ok_or_else(|| format!("output path has no parent: {}", dest_path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create output dir {}: {error}", parent.display()))?;
    let bytes = nif
        .to_bytes()
        .map_err(|error| format!("serialize baked nif: {error}"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("create temp file in {}: {error}", parent.display()))?;
    temp.write_all(&bytes)
        .map_err(|error| format!("write temp file: {error}"))?;
    temp.flush()
        .map_err(|error| format!("flush temp file: {error}"))?;
    temp.persist(dest_path)
        .map_err(|error| format!("persist {}: {}", dest_path.display(), error.error))?;
    Ok(())
}

/// Resolve a MODL-style path against a data root, splitting on either
/// separator so callers don't depend on the host OS's path convention.
/// Mirrors the resolution pattern used by object LOD's `mnam_abs_path`.
///
/// By game convention, MODL paths are relative to `Data\Meshes\` and do NOT
/// carry a leading "meshes" component (e.g. `architecture\cabin\wall01.nif`,
/// verified against the WhitespringMall01 real run) — a leading "meshes" is
/// prepended unless the path already starts with one (case-insensitively;
/// some MODL strings do carry it), so it's never doubled.
fn resolve_data_relative_path(data_root: &Path, model_path: &str) -> PathBuf {
    let mut out = data_root.to_path_buf();
    let normalized = model_path.replace('\\', "/");
    let components: Vec<&str> = normalized.split('/').filter(|c| !c.is_empty()).collect();
    let already_prefixed = components
        .first()
        .is_some_and(|c| c.eq_ignore_ascii_case("meshes"));
    if !already_prefixed {
        out.push("meshes");
    }
    for component in components {
        out.push(component);
    }
    out
}

/// One `mesh_archives` entry, opened (mmapped + name-table indexed) once per
/// `bake_cell` call and shared read-only across every group's resolution
/// attempt — extraction only decompresses the one member a group actually
/// needs, never the whole archive.
struct MeshArchive {
    path: PathBuf,
    archive: fo4::Archive<'static>,
    write_options: fo4::FileWriteOptions,
}

fn open_mesh_archives(archive_paths: &[PathBuf]) -> Vec<MeshArchive> {
    archive_paths
        .iter()
        .filter_map(|path| {
            let (archive, options) = fo4::Archive::read(path.as_path()).ok()?;
            let write_options: fo4::FileWriteOptions = (&options).into();
            Some(MeshArchive {
                path: path.clone(),
                archive,
                write_options,
            })
        })
        .collect()
}

/// Mirrors `resolve_data_relative_path`'s meshes-prefix convention to
/// produce the Data-relative member path BA2 archives are packed with.
/// Separator/case normalization happens inside `fo4::ArchiveKey::from`, so
/// this only needs to get the "meshes\" prefix right.
fn archive_member_path(model_path: &str) -> String {
    let already_prefixed = model_path
        .replace('\\', "/")
        .split('/')
        .find(|c| !c.is_empty())
        .is_some_and(|c| c.eq_ignore_ascii_case("meshes"));
    if already_prefixed {
        model_path.to_string()
    } else {
        format!("meshes\\{model_path}")
    }
}

fn archives_consulted_suffix(archives: &[MeshArchive]) -> String {
    if archives.is_empty() {
        return String::new();
    }
    let names: Vec<String> = archives.iter().map(|a| a.path.display().to_string()).collect();
    format!(", tried archive(s): {}", names.join(", "))
}

/// Names the resolved path tried in each `mesh_extract_roots` entry, mirroring
/// `archives_consulted_suffix` for the loud-miss message.
fn extract_roots_tried_suffix(mesh_extract_roots: &[PathBuf], model_path: &str) -> String {
    if mesh_extract_roots.is_empty() {
        return String::new();
    }
    let tried: Vec<String> = mesh_extract_roots
        .iter()
        .map(|root| resolve_data_relative_path(root, model_path).display().to_string())
        .collect();
    format!(", tried extract root(s): {}", tried.join(", "))
}

/// Resolves and loads a group's source NIF. Resolution order: the loose path
/// under `data_root` (unchanged v0 behavior), then each of `mesh_extract_roots`
/// in list order (a pre-extracted asset directory mirroring `Data\` layout —
/// same resolution convention as `data_root`), then each of `archives` in
/// list order — first hit wins. A member found in an extract root or archive
/// but corrupt/unparseable is a hard error (mirrors a loose file that fails
/// to load) — resolution does not fall through to later candidates once a
/// member is found. Archive hits are read entirely in memory
/// (`file.write` into a `Vec<u8>` + `NifFile::from_bytes`); no temp file is
/// ever written for an archive-sourced mesh.
fn resolve_source_nif(
    group: &ModelGroup,
    params: &Params,
    archives: &[MeshArchive],
) -> Result<NifFile, String> {
    let source_path = resolve_data_relative_path(&params.data_root, &group.model_path);
    if source_path.is_file() {
        return NifFile::load(&source_path)
            .map_err(|error| format!("load source nif {}: {error}", source_path.display()));
    }

    for extract_root in &params.mesh_extract_roots {
        let candidate = resolve_data_relative_path(extract_root, &group.model_path);
        if candidate.is_file() {
            return NifFile::load(&candidate)
                .map_err(|error| format!("load source nif {}: {error}", candidate.display()));
        }
    }

    let member_path = archive_member_path(&group.model_path);
    for mesh_archive in archives {
        let key: fo4::ArchiveKey = member_path.as_str().into();
        let Some(file) = mesh_archive.archive.get(&key) else {
            continue;
        };
        let mut bytes = Vec::new();
        file.write(&mut bytes, &mesh_archive.write_options)
            .map_err(|error| {
                format!(
                    "extract {member_path} from archive {}: {error}",
                    mesh_archive.path.display()
                )
            })?;
        return NifFile::from_bytes(&bytes, Some(source_path.clone())).map_err(|error| {
            format!(
                "load source nif {member_path} from archive {}: {error}",
                mesh_archive.path.display()
            )
        });
    }

    Err(format!(
        "source mesh not found on disk as a loose file: {}{}{} (mesh must be present loose \
         under data_root, one of mesh_extract_roots, or inside a configured mesh_archives entry)",
        source_path.display(),
        extract_roots_tried_suffix(&params.mesh_extract_roots, &group.model_path),
        archives_consulted_suffix(archives),
    ))
}

fn is_eligible_shape_block(block: &NifBlock) -> bool {
    matches!(
        block.type_name.as_str(),
        "BSTriShape" | "BSDynamicTriShape" | "BSSubIndexTriShape"
    ) && matches!(block.get_field("Vertex Data"), Some(NifValue::Array(a)) if !a.is_empty())
        && matches!(block.get_field("Triangles"), Some(NifValue::Array(a)) if !a.is_empty())
}

/// v0 requires at least one eligible inline render shape and no particle
/// systems in the whole source NIF; anything else rejects the whole group.
/// v1.b lifts the exactly-one-shape limit — CK Filtered emits one
/// shape+PCD(+shader+texset) pair per material within a single `_OC.nif`
/// (block index order gives a deterministic, source-order emission order).
fn eligible_shape_block_ids(nif: &NifFile) -> Result<Vec<usize>, String> {
    if nif
        .blocks
        .iter()
        .any(|b| b.type_name.contains("ParticleSystem"))
    {
        return Err("particle systems are unsupported in v0".to_string());
    }
    let candidates: Vec<usize> = nif
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| is_eligible_shape_block(b))
        .map(|(i, _)| i)
        .collect();
    if candidates.is_empty() {
        return Err("no eligible inline shape found".to_string());
    }
    Ok(candidates)
}

fn resolve_ref_block(nif: &NifFile, block: &NifBlock, field_name: &str) -> Option<usize> {
    match block.get_field(field_name) {
        Some(NifValue::Ref(r)) if *r >= 0 => {
            let id = *r as usize;
            if id < nif.blocks.len() { Some(id) } else { None }
        }
        _ => None,
    }
}

fn ref_is_none(value: Option<&NifValue>) -> bool {
    match value {
        None => true,
        Some(NifValue::Ref(r)) => *r < 0,
        Some(_) => false,
    }
}

fn strip_controller(block: &mut NifBlock) {
    if block.get_field("Controller").is_some() {
        block.set_field("Controller", NifValue::Ref(-1));
    }
}

/// The NIF reader represents a `Vector3` sub-field of an already-loaded
/// struct generically as `NifValue::Struct({"x","y","z"})`, not the
/// `NifValue::Vec3` convenience variant (that variant is write-only: the
/// writer special-cases it, but the reader never produces it). Accept both
/// so this helper works on freshly-constructed and reloaded values alike.
fn as_vec3(value: Option<&NifValue>) -> Option<[f32; 3]> {
    match value {
        Some(NifValue::Vec3(v)) => Some(*v),
        Some(NifValue::Struct(fields)) => {
            let x = as_f32(fields.get("x"))?;
            let y = as_f32(fields.get("y"))?;
            let z = as_f32(fields.get("z"))?;
            Some([x, y, z])
        }
        _ => None,
    }
}

fn as_f32(value: Option<&NifValue>) -> Option<f32> {
    match value {
        Some(NifValue::Float(f)) => Some(*f as f32),
        _ => None,
    }
}

fn read_bound(value: Option<&NifValue>) -> Option<([f32; 3], f32)> {
    let NifValue::Struct(fields) = value? else {
        return None;
    };
    let center = as_vec3(fields.get("Center"))?;
    let radius = as_f32(fields.get("Radius"))?;
    Some((center, radius))
}

/// Prefers the source shader's own grayscale-to-palette value when present
/// (checking both the flattened top-level slot and the nested inline
/// `Shader Property Data` struct, since the schema branch that applies
/// depends on whether the shader references a material file), else 1.0.
fn source_grayscale_to_palette_scale(shader: &NifBlock) -> f32 {
    if let Some(NifValue::Float(f)) = shader.get_field("Grayscale to Palette Scale") {
        return *f as f32;
    }
    if let Some(NifValue::Struct(inner)) = shader.get_field("Shader Property Data") {
        if let Some(NifValue::Float(f)) = inner.get("Grayscale to Palette Scale") {
            return *f as f32;
        }
    }
    1.0
}

/// Bethesda REFR `DATA` rotations are radians, applied Z*Y*X: rotate about X,
/// then Y, then Z. Verified by `euler_z_rotation_maps_plus_x_to_plus_y` below.
fn euler_to_matrix33(rotation: [f32; 3]) -> [[f32; 3]; 3] {
    let (sx, cx) = rotation[0].sin_cos();
    let (sy, cy) = rotation[1].sin_cos();
    let (sz, cz) = rotation[2].sin_cos();
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mat3_mul(&mat3_mul(&rz, &ry), &rx)
}

fn mat3_mul(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

fn transpose3(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

fn mat3_apply(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Transform a source bound by one REFR's rotation/translation/uniform
/// scale. Scale is applied before rotation, but since it's a uniform scalar
/// the two commute; only rotation needs to be orthonormal for the radius to
/// scale correctly.
fn transform_bound(
    source: ([f32; 3], f32),
    rotation: [[f32; 3]; 3],
    translation: [f32; 3],
    scale: f32,
) -> ([f32; 3], f32) {
    let (center, radius) = source;
    let rotated = mat3_apply(&rotation, center);
    let world_center = [
        rotated[0] * scale + translation[0],
        rotated[1] * scale + translation[1],
        rotated[2] * scale + translation[2],
    ];
    (world_center, radius * scale.abs())
}

/// Aggregate culling sphere: center is the mean of every transformed
/// instance bound center; radius covers every transformed instance bound
/// (distance from the aggregate center to each instance's farthest point).
fn aggregate_cull_sphere(bounds: &[([f32; 3], f32)]) -> ([f32; 3], f32) {
    debug_assert!(!bounds.is_empty());
    let n = bounds.len() as f32;
    let mut center = [0.0f32; 3];
    for (c, _) in bounds {
        center[0] += c[0];
        center[1] += c[1];
        center[2] += c[2];
    }
    center[0] /= n;
    center[1] /= n;
    center[2] /= n;

    let mut radius = 0.0f32;
    for (c, r) in bounds {
        let dx = c[0] - center[0];
        let dy = c[1] - center[1];
        let dz = c[2] - center[2];
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        radius = radius.max(distance + r);
    }
    (center, radius)
}

fn packed_geom_data_combined(
    grayscale: f32,
    rotation: [[f32; 3]; 3],
    translation: [f32; 3],
    scale: f32,
    bound_center: [f32; 3],
    bound_radius: f32,
) -> NifValue {
    NifValue::Struct(struct_fields([
        (
            "Grayscale to Palette Scale",
            NifValue::Float(f64::from(grayscale)),
        ),
        (
            "Transform",
            NifValue::Struct(struct_fields([
                ("Rotation", NifValue::Matrix33(rotation)),
                ("Translation", NifValue::Vec3(translation)),
                ("Scale", NifValue::Float(f64::from(scale))),
            ])),
        ),
        (
            "Bounding Sphere",
            NifValue::Struct(struct_fields([
                ("Center", NifValue::Vec3(bound_center)),
                ("Radius", NifValue::Float(f64::from(bound_radius))),
            ])),
        ),
    ]))
}

/// Promotes a half-precision `Vertex Desc` (as read from a source shape) to
/// the full-precision desc CK Filtered `_OC` shapes and top-level PCD blocks
/// always carry: +2 words (8 bytes) to the stride and every populated
/// attribute-offset field (position grows from half3+halfBitangentX, 8B, to
/// float3+floatBitangentX, 16B), plus the Full_Precision flag bit. A zero
/// attribute-offset means "not present" and is left at 0, not shifted.
/// Verified against two real CK descs:
/// `0x1B00000430205` (20B half, no color) -> `0x41B00000650407` (28B full)
/// `0x3B00005430206` (24B half, +color) -> `0x43B00007650408` (32B full)
fn promote_to_full_precision(desc: u64) -> u64 {
    const VF_FULLPREC: u64 = 0x0400;
    const WORD_DELTA: u64 = 2;

    let mut out = shift_word_field(desc, 0, 8, WORD_DELTA, false); // stride
    out = shift_word_field(out, 8, 8, WORD_DELTA, false); // UV offset
    out = shift_word_field(out, 16, 4, WORD_DELTA, false); // Normal offset
    out = shift_word_field(out, 20, 4, WORD_DELTA, false); // Tangent offset
    out = shift_word_field(out, 24, 8, WORD_DELTA, true); // Color offset (0 = absent)
    out | (VF_FULLPREC << 44)
}

/// Adds `delta` to the `width`-bit field at bit `shift` within `desc`,
/// leaving every other bit untouched. When `skip_if_zero` is set, a
/// zero-valued field (meaning "attribute not present") is left at 0.
fn shift_word_field(desc: u64, shift: u32, width: u32, delta: u64, skip_if_zero: bool) -> u64 {
    let mask = ((1u64 << width) - 1) << shift;
    let value = (desc & mask) >> shift;
    if skip_if_zero && value == 0 {
        return desc;
    }
    (desc & !mask) | (((value + delta) << shift) & mask)
}

/// crc32 over the cell's own object id plus the sorted member REFR object
/// ids; 0 and `u32::MAX` are reserved sentinels, and collisions within one
/// `bake_cell` call are resolved deterministically by salting the hash.
fn mint_mesh_id(cell_form_id: u32, instances: &[InstanceRef], used: &mut HashSet<u32>) -> u32 {
    let mut member_ids: Vec<u32> = instances
        .iter()
        .map(|i| i.refr_form_id & 0x00FF_FFFF)
        .collect();
    member_ids.sort_unstable();

    let mut salt: u32 = 0;
    loop {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&(cell_form_id & 0x00FF_FFFF).to_le_bytes());
        for id in &member_ids {
            hasher.update(&id.to_le_bytes());
        }
        if salt > 0 {
            hasher.update(&salt.to_le_bytes());
        }
        let candidate = hasher.finalize();
        if candidate != 0 && candidate != u32::MAX && used.insert(candidate) {
            return candidate;
        }
        salt += 1;
    }
}

fn struct_fields<const N: usize>(entries: [(&str, NifValue); N]) -> IndexMap<String, NifValue> {
    entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ck_native_precombine_bake_{name}_{}_{suffix}",
            std::process::id()
        ))
    }

    fn basic_vertex(position: [f32; 3]) -> NifValue {
        NifValue::Struct(struct_fields([
            ("Vertex", NifValue::Vec3(position)),
            ("Bitangent X", NifValue::Float(0.0)),
            (
                "UV",
                NifValue::Struct(struct_fields([
                    ("u", NifValue::Float(0.0)),
                    ("v", NifValue::Float(0.0)),
                ])),
            ),
            ("Normal", NifValue::Vec3([0.0, 0.0, 1.0])),
            ("Bitangent Y", NifValue::Float(1.0)),
            ("Tangent", NifValue::Vec3([1.0, 0.0, 0.0])),
            ("Bitangent Z", NifValue::Float(0.0)),
        ]))
    }

    fn triangle(v1: u64, v2: u64, v3: u64) -> NifValue {
        NifValue::Struct(struct_fields([
            ("v1", NifValue::UInt(v1)),
            ("v2", NifValue::UInt(v2)),
            ("v3", NifValue::UInt(v3)),
        ]))
    }

    /// Proven-good stride/attribute bit pattern (float3 position, UV, normal,
    /// tangent/bitangent, no vertex colors); mirrors
    /// `nif_core/tests/convert_file.rs::basic_vertex_desc(false)`.
    fn basic_vertex_desc() -> i64 {
        let stride = 5i64;
        let flags = 0x0001 | 0x0002 | 0x0008 | 0x0010;
        stride | (2 << 8) | (3 << 16) | (4 << 20) | (flags << 44)
    }

    fn simple_bound() -> NifValue {
        NifValue::Struct(struct_fields([
            ("Center", NifValue::Vec3([0.0, 0.0, 0.0])),
            ("Radius", NifValue::Float(1.0)),
        ]))
    }

    /// Mirrors `as_vec3`'s write-only-variant-vs-reloaded-struct handling,
    /// but for `Matrix33`. On the reloaded `Struct` form, field name `mRC`
    /// maps to row R-1, column C-1 (matches `nif_core::skeleton_repose`'s
    /// `read_rotation`), so this decodes to exactly the `[[f32;3];3]` array
    /// the writer was given — a faithful round-trip, letting the test assert
    /// on what `bake_group` actually stored.
    fn read_matrix33(value: Option<&NifValue>) -> Option<[[f32; 3]; 3]> {
        match value {
            Some(NifValue::Matrix33(m)) => Some(*m),
            Some(NifValue::Struct(fields)) => {
                let g = |key: &str| as_f32(fields.get(key));
                Some([
                    [g("m11")?, g("m21")?, g("m31")?],
                    [g("m12")?, g("m22")?, g("m32")?],
                    [g("m13")?, g("m23")?, g("m33")?],
                ])
            }
            _ => None,
        }
    }

    fn simple_geometry() -> (Vec<NifValue>, Vec<NifValue>) {
        let vertex_data = vec![
            basic_vertex([0.0, 0.0, 0.0]),
            basic_vertex([1.0, 0.0, 0.0]),
            basic_vertex([0.0, 1.0, 0.0]),
            basic_vertex([1.0, 1.0, 0.0]),
        ];
        let triangles = vec![triangle(0, 1, 2), triangle(1, 3, 2)];
        (vertex_data, triangles)
    }

    /// Writes a single-shape FO4 source NIF (inline vertices/triangles,
    /// lighting shader, texture set, optional alpha property) to `path`.
    /// Returns (vertex_count, triangle_count) for the caller's assertions.
    fn write_source_fixture(path: &Path, with_alpha: bool) -> (usize, usize) {
        let mut nif = NifFile::new("fo4");
        let texset_id = nif.add_block(
            "BSShaderTextureSet",
            Some(struct_fields([
                ("Num Textures", NifValue::UInt(2)),
                (
                    "Textures",
                    NifValue::Array(vec![
                        NifValue::String(r"textures\test\wall_d.dds".to_string()),
                        NifValue::String(r"textures\test\wall_n.dds".to_string()),
                    ]),
                ),
            ])),
        );
        let shader_id = nif.add_block(
            "BSLightingShaderProperty",
            Some(struct_fields([
                ("Name", NifValue::String(String::new())),
                ("Texture Set", NifValue::Ref(texset_id as i32)),
            ])),
        );
        let alpha_id = if with_alpha {
            Some(nif.add_block("NiAlphaProperty", None))
        } else {
            None
        };

        let (vertex_data, triangles) = simple_geometry();
        let vertex_count = vertex_data.len();
        let triangle_count = triangles.len();
        let shape_id = nif.add_block(
            "BSTriShape",
            Some(struct_fields([
                ("Name", NifValue::String("Shape:0".to_string())),
                ("Bounding Sphere", simple_bound()),
                ("Skin", NifValue::Ref(-1)),
                ("Shader Property", NifValue::Ref(shader_id as i32)),
                (
                    "Alpha Property",
                    NifValue::Ref(alpha_id.map(|id| id as i32).unwrap_or(-1)),
                ),
                ("Vertex Desc", NifValue::Int(basic_vertex_desc())),
                ("Num Triangles", NifValue::UInt(triangle_count as u64)),
                ("Num Vertices", NifValue::UInt(vertex_count as u64)),
                ("Vertex Data", NifValue::Array(vertex_data)),
                ("Triangles", NifValue::Array(triangles)),
            ])),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );

        std::fs::create_dir_all(path.parent().expect("fixture path has a parent"))
            .expect("create fixture dir");
        nif.save(Some(path.to_path_buf())).expect("write source nif");
        (vertex_count, triangle_count)
    }

    /// A geometry distinguishable in vertex count from `simple_geometry`'s
    /// (4 verts), so a test can tell which of two candidate sources actually
    /// got baked.
    fn small_geometry() -> (Vec<NifValue>, Vec<NifValue>) {
        let vertex_data = vec![
            basic_vertex([0.0, 0.0, 0.0]),
            basic_vertex([1.0, 0.0, 0.0]),
            basic_vertex([0.0, 1.0, 0.0]),
        ];
        let triangles = vec![triangle(0, 1, 2)];
        (vertex_data, triangles)
    }

    /// Same shape as `write_source_fixture` but returns serialized bytes
    /// instead of writing to a loose path, for embedding into a synthetic
    /// BA2 fixture via `write_ba2_fixture`.
    fn build_source_fixture_bytes(vertex_data: Vec<NifValue>, triangles: Vec<NifValue>) -> Vec<u8> {
        let mut nif = NifFile::new("fo4");
        let texset_id = nif.add_block(
            "BSShaderTextureSet",
            Some(struct_fields([
                ("Num Textures", NifValue::UInt(1)),
                (
                    "Textures",
                    NifValue::Array(vec![NifValue::String(r"textures\test\a.dds".to_string())]),
                ),
            ])),
        );
        let shader_id = nif.add_block(
            "BSLightingShaderProperty",
            Some(struct_fields([
                ("Name", NifValue::String(String::new())),
                ("Texture Set", NifValue::Ref(texset_id as i32)),
            ])),
        );
        let vertex_count = vertex_data.len();
        let triangle_count = triangles.len();
        let shape_id = nif.add_block(
            "BSTriShape",
            Some(struct_fields([
                ("Name", NifValue::String("Shape:0".to_string())),
                ("Bounding Sphere", simple_bound()),
                ("Skin", NifValue::Ref(-1)),
                ("Shader Property", NifValue::Ref(shader_id as i32)),
                ("Alpha Property", NifValue::Ref(-1)),
                ("Vertex Desc", NifValue::Int(basic_vertex_desc())),
                ("Num Triangles", NifValue::UInt(triangle_count as u64)),
                ("Num Vertices", NifValue::UInt(vertex_count as u64)),
                ("Vertex Data", NifValue::Array(vertex_data)),
                ("Triangles", NifValue::Array(triangles)),
            ])),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );
        nif.to_bytes().expect("serialize fixture nif")
    }

    /// Like `write_source_fixture` but accepts custom geometry (so two
    /// fixtures can be distinguished by vertex count) and writes it to a
    /// loose path — for `mesh_extract_roots` fixtures, which (unlike
    /// `mesh_archives`) are ordinary loose files under a different root.
    fn write_fixture_with_geometry(path: &Path, vertex_data: Vec<NifValue>, triangles: Vec<NifValue>) {
        let bytes = build_source_fixture_bytes(vertex_data, triangles);
        std::fs::create_dir_all(path.parent().expect("fixture path has a parent"))
            .expect("create fixture dir");
        std::fs::write(path, bytes).expect("write fixture nif");
    }

    /// Writes a synthetic GNRL-format FO4 BA2 at `path` containing `entries`
    /// (member path -> uncompressed bytes), using bsarchive's write API.
    fn write_ba2_fixture(path: &Path, entries: &[(&str, &[u8])]) {
        use bsarchive_native::fo4::{Archive, ArchiveKey, ArchiveOptions, Chunk, File as BsaFile};
        use bsarchive_native::prelude::*;

        let archive: Archive = entries
            .iter()
            .map(|(member_path, bytes)| {
                let chunk = Chunk::from_decompressed(*bytes);
                let file: BsaFile = [chunk].into_iter().collect();
                let key: ArchiveKey = (*member_path).into();
                (key, file)
            })
            .collect();
        std::fs::create_dir_all(path.parent().expect("archive path has a parent"))
            .expect("create archive dir");
        let mut dst = std::fs::File::create(path).expect("create ba2 fixture file");
        archive
            .write(&mut dst, &ArchiveOptions::default())
            .expect("write ba2 fixture");
    }

    #[test]
    fn euler_z_rotation_maps_plus_x_to_plus_y() {
        // Documents the chosen convention: Z*Y*X applied as a column-vector
        // matrix-vector product, matrix rows stored in NifValue::Matrix33.
        let m = euler_to_matrix33([0.0, 0.0, std::f32::consts::FRAC_PI_2]);
        let rotated = mat3_apply(&m, [1.0, 0.0, 0.0]);
        assert!(rotated[0].abs() < 1e-5, "x should vanish: {rotated:?}");
        assert!((rotated[1] - 1.0).abs() < 1e-5, "y should be 1: {rotated:?}");
        assert!(rotated[2].abs() < 1e-5, "z should vanish: {rotated:?}");
    }

    #[test]
    fn aggregate_cull_sphere_covers_every_transformed_instance() {
        let bounds = vec![([0.0, 0.0, 0.0], 1.0), ([10.0, 0.0, 0.0], 1.0)];
        let (center, radius) = aggregate_cull_sphere(&bounds);
        assert_eq!(center, [5.0, 0.0, 0.0]);
        assert_eq!(radius, 6.0);
    }

    #[test]
    fn grayscale_to_palette_scale_prefers_nested_source_value_over_default() {
        let mut block = NifBlock::new(0, "BSLightingShaderProperty");
        let inner = struct_fields([("Grayscale to Palette Scale", NifValue::Float(0.42))]);
        block
            .fields
            .insert("Shader Property Data".to_string(), NifValue::Struct(inner));
        assert!((source_grayscale_to_palette_scale(&block) - 0.42).abs() < 1e-6);
    }

    #[test]
    fn grayscale_to_palette_scale_defaults_to_one_when_absent() {
        let block = NifBlock::new(0, "BSLightingShaderProperty");
        assert_eq!(source_grayscale_to_palette_scale(&block), 1.0);
    }

    #[test]
    fn promote_to_full_precision_matches_ck_ground_truth_no_color() {
        assert_eq!(promote_to_full_precision(0x1B00000430205), 0x41B00000650407);
    }

    #[test]
    fn promote_to_full_precision_matches_ck_ground_truth_with_color() {
        assert_eq!(promote_to_full_precision(0x3B00005430206), 0x43B00007650408);
    }

    #[test]
    fn bakes_two_instance_group_into_filtered_oc_nif() {
        let dir = temp_dir("roundtrip");
        let source_path = dir
            .join("data")
            .join("meshes")
            .join("test")
            .join("chair01.nif");
        let (vertex_count, triangle_count) = write_source_fixture(&source_path, true);

        let group = ModelGroup {
            model_path: "meshes\\test\\chair01.nif".to_string(),
            instances: vec![
                InstanceRef {
                    refr_form_id: 0x0100_0600,
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: 1.0,
                },
                InstanceRef {
                    refr_form_id: 0x0100_0601,
                    position: [10.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, std::f32::consts::FRAC_PI_2],
                    scale: 1.0,
                },
            ],
        };
        let plan = CellPlan {
            cell_form_id: 0x0062_781C,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x0062_781C],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: Vec::new(),
        };

        let report = bake_cell(&plan, &params);
        assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
        let baked = report.baked.expect("bake should succeed");
        assert_eq!(baked.cell_form_id, 0x0062_781C);
        assert_eq!(baked.meshes.len(), 1);

        let mesh = &baked.meshes[0];
        assert_eq!(mesh.refs, vec![0x0100_0600, 0x0100_0601]);
        assert_ne!(mesh.mesh_id, 0);
        assert_ne!(mesh.mesh_id, u32::MAX);
        let expected_rel = format!("meshes\\precombined\\Test.esm\\0062781C_{:08X}_OC.nif", mesh.mesh_id);
        assert_eq!(mesh.rel_path, expected_rel);

        let dest = {
            let mut p = params.data_root.clone();
            p.push("meshes");
            p.push("precombined");
            p.push("Test.esm");
            p.push(format!("0062781C_{:08X}_OC.nif", mesh.mesh_id));
            p
        };
        let out = NifFile::load(&dest).expect("reload baked nif");

        assert!(
            !out.blocks.iter().any(|b| b.type_name == "BSXFlags"),
            "Filtered _OC.nif must carry no BSXFlags/Havok block (that belongs to <cell>_Physics.NIF)"
        );

        let root = out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSFadeNode")
            .expect("root BSFadeNode");
        assert_eq!(root.get_field("Flags").map(|v| v.as_i64()), Some(0x400E));
        match root.get_field("Name") {
            Some(NifValue::String(s)) => {
                assert_eq!(s, &format!("0062781C_{:08X}_OC", mesh.mesh_id));
            }
            other => panic!("expected root Name string, got {other:?}"),
        }

        let root_children = match root.get_field("Children") {
            Some(NifValue::Array(items)) => items.clone(),
            other => panic!("expected root children, got {other:?}"),
        };
        assert_eq!(root_children.len(), 1);
        let inner_ref = match &root_children[0] {
            NifValue::Ref(r) => *r,
            other => panic!("expected ref, got {other:?}"),
        };
        let inner = &out.blocks[inner_ref as usize];
        assert_eq!(inner.type_name, "NiNode");
        assert_eq!(inner.get_field("Flags").map(|v| v.as_i64()), Some(14));

        let inner_children = match inner.get_field("Children") {
            Some(NifValue::Array(items)) => items.clone(),
            other => panic!("expected inner children, got {other:?}"),
        };
        assert_eq!(inner_children.len(), 1);
        let shape_ref = match &inner_children[0] {
            NifValue::Ref(r) => *r,
            other => panic!("expected ref, got {other:?}"),
        };
        let shape = &out.blocks[shape_ref as usize];
        assert_eq!(shape.type_name, "BSTriShape");
        // TIER 1: instance-EXPANDED counts (per-instance count x number of
        // instances) alongside Data Size=0, or the engine's PCD unpacker
        // sizes its combined runtime buffers from the deduplicated count and
        // heap-overruns once it writes instance-expanded geometry into them.
        // This fixture has 2 instances of a 4-vert/2-tri mesh.
        assert_eq!(
            shape.get_field("Num Vertices").map(|v| v.as_i64()),
            Some((vertex_count * 2) as i64)
        );
        assert_eq!(
            shape.get_field("Num Triangles").map(|v| v.as_i64()),
            Some((triangle_count * 2) as i64)
        );
        assert_eq!(shape.get_field("Data Size").map(|v| v.as_i64()), Some(0));
        assert_eq!(shape.get_field("Flags").map(|v| v.as_i64()), Some(526));
        // Round 2: shape Vertex Desc must be promoted to Full_Precision, not
        // left at the source's half-precision desc.
        assert_eq!(
            shape.get_field("Vertex Desc").map(|v| v.as_i64()),
            Some(promote_to_full_precision(basic_vertex_desc() as u64) as i64)
        );

        let translation = as_vec3(shape.get_field("Translation")).expect("shape translation vec3");
        // Aggregate center of two instances at x=0 and x=10 is x=5.
        assert!((translation[0] - 5.0).abs() < 1e-3, "translation: {translation:?}");

        let bound = match shape.get_field("Bounding Sphere") {
            Some(NifValue::Struct(m)) => m.clone(),
            other => panic!("expected bounding sphere struct, got {other:?}"),
        };
        let bound_center = as_vec3(bound.get("Center")).expect("bound center vec3");
        assert_eq!(bound_center, [0.0, 0.0, 0.0]);
        match bound.get("Radius") {
            Some(NifValue::Float(r)) => assert!((*r - 6.0).abs() < 1e-2, "radius: {r}"),
            other => panic!("expected bound radius, got {other:?}"),
        }

        let shape_extra = match shape.get_field("Extra Data List") {
            Some(NifValue::Array(items)) => items.clone(),
            other => panic!("expected shape extra data list, got {other:?}"),
        };
        assert_eq!(shape_extra.len(), 1);
        let pcd_ref = match &shape_extra[0] {
            NifValue::Ref(r) => *r,
            other => panic!("expected ref, got {other:?}"),
        };
        let pcd = &out.blocks[pcd_ref as usize];
        assert_eq!(pcd.type_name, "BSPackedCombinedGeomDataExtra");
        match pcd.get_field("Name") {
            Some(NifValue::String(s)) => assert_eq!(s, "PCD"),
            other => panic!("expected PCD name, got {other:?}"),
        }
        assert_eq!(pcd.get_field("Unknown Flags 1").map(|v| v.as_i64()), Some(0));
        assert_eq!(pcd.get_field("Unknown Flags 2").map(|v| v.as_i64()), Some(0));
        // Top-level PCD counts must equal the shape's instance-expanded
        // counts, which diverge from the inner Object Data's deduplicated
        // per-instance counts below (this fixture: 2 instances of 4/2).
        assert_eq!(
            pcd.get_field("Num Vertices").map(|v| v.as_i64()),
            Some((vertex_count * 2) as i64)
        );
        assert_eq!(
            pcd.get_field("Num Triangles").map(|v| v.as_i64()),
            Some((triangle_count * 2) as i64)
        );
        // Top-level PCD desc must also be promoted, matching the shape.
        assert_eq!(
            pcd.get_field("Vertex Desc").map(|v| v.as_i64()),
            Some(promote_to_full_precision(basic_vertex_desc() as u64) as i64)
        );

        let object_data = match pcd.get_field("Object Data") {
            Some(NifValue::Array(items)) => items.clone(),
            other => panic!("expected object data array, got {other:?}"),
        };
        assert_eq!(object_data.len(), 1);
        let geom = match &object_data[0] {
            NifValue::Struct(m) => m.clone(),
            other => panic!("expected geom struct, got {other:?}"),
        };
        assert_eq!(geom.get("Num Verts").map(|v| v.as_i64()), Some(vertex_count as i64));
        assert_eq!(geom.get("LOD Levels").map(|v| v.as_i64()), Some(3));
        assert_eq!(
            geom.get("Tri Count LOD0").map(|v| v.as_i64()),
            Some(triangle_count as i64)
        );
        assert_eq!(geom.get("Tri Offset LOD0").map(|v| v.as_i64()), Some(0));
        assert_eq!(geom.get("Tri Offset LOD1").map(|v| v.as_i64()), Some(0));
        assert_eq!(geom.get("Tri Offset LOD2").map(|v| v.as_i64()), Some(0));
        // Round 2: the inner Object Data desc must stay half-precision
        // (unchanged from the source) — only the shape and top-level PCD
        // descs are promoted. The two must intentionally diverge.
        assert_eq!(geom.get("Vertex Desc").map(|v| v.as_i64()), Some(basic_vertex_desc()));
        let combined = match geom.get("Combined") {
            Some(NifValue::Array(items)) => items.clone(),
            other => panic!("expected combined array, got {other:?}"),
        };
        assert_eq!(combined.len(), 2, "one combined row per REFR instance");
        for row in &combined {
            let NifValue::Struct(fields) = row else {
                panic!("expected combined row struct, got {row:?}");
            };
            match fields.get("Grayscale to Palette Scale") {
                Some(NifValue::Float(f)) => assert!((*f - 1.0).abs() < 1e-6, "grayscale default: {f}"),
                other => panic!("expected grayscale float, got {other:?}"),
            }
        }
        let second_translation = match &combined[1] {
            NifValue::Struct(fields) => match fields.get("Transform") {
                Some(NifValue::Struct(t)) => as_vec3(t.get("Translation")).expect("combined translation vec3"),
                other => panic!("expected transform struct, got {other:?}"),
            },
            other => panic!("expected combined row struct, got {other:?}"),
        };
        assert!((second_translation[0] - 10.0).abs() < 1e-3);

        // Round 4: the Combined row's stored rotation must be the TRANSPOSE
        // of the instance's true rotation (decoder mirrors encoder, so the
        // decoded array shows exactly what bake_group stored). The second
        // instance has rot Z=+90deg; true Rz(90) = [[0,-1,0],[1,0,0],[0,0,1]],
        // so the stored/decoded matrix must be [[0,1,0],[-1,0,0],[0,0,1]].
        let second_rotation = match &combined[1] {
            NifValue::Struct(fields) => match fields.get("Transform") {
                Some(NifValue::Struct(t)) => {
                    read_matrix33(t.get("Rotation")).expect("combined rotation matrix33")
                }
                other => panic!("expected transform struct, got {other:?}"),
            },
            other => panic!("expected combined row struct, got {other:?}"),
        };
        let expected_transpose = [[0.0f32, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (second_rotation[i][j] - expected_transpose[i][j]).abs() < 1e-4,
                    "combined rotation must be stored transposed: got {second_rotation:?}, \
                     expected {expected_transpose:?}"
                );
            }
        }

        let geom_vertex_len = match geom.get("Vertex Data") {
            Some(NifValue::Array(items)) => items.len(),
            other => panic!("expected vertex data array, got {other:?}"),
        };
        assert_eq!(geom_vertex_len, vertex_count);
        let geom_triangle_len = match geom.get("Triangles") {
            Some(NifValue::Array(items)) => items.len(),
            other => panic!("expected triangles array, got {other:?}"),
        };
        assert_eq!(geom_triangle_len, triangle_count);

        let shader_ref = match shape.get_field("Shader Property") {
            Some(NifValue::Ref(r)) => *r,
            other => panic!("expected shader ref, got {other:?}"),
        };
        assert!(shader_ref >= 0, "shader property must be remapped");
        let shader = &out.blocks[shader_ref as usize];
        assert_eq!(shader.type_name, "BSLightingShaderProperty");

        let texset_ref = match shader.get_field("Texture Set") {
            Some(NifValue::Ref(r)) => *r,
            other => panic!("expected texset ref, got {other:?}"),
        };
        assert!(texset_ref >= 0 && texset_ref != shader_ref, "texset must be remapped");
        assert_eq!(out.blocks[texset_ref as usize].type_name, "BSShaderTextureSet");

        let alpha_ref = match shape.get_field("Alpha Property") {
            Some(NifValue::Ref(r)) => *r,
            other => panic!("expected alpha ref, got {other:?}"),
        };
        assert!(alpha_ref >= 0, "alpha property must be cloned and remapped");
        assert_eq!(out.blocks[alpha_ref as usize].type_name, "NiAlphaProperty");

        // TIER 3c: emission order per shape is shape, PCD, shader, texset, alpha.
        assert!(shape_ref < pcd_ref, "shape must be emitted before PCD");
        assert!(pcd_ref < shader_ref, "PCD must be emitted before shader");
        assert!(shader_ref < texset_ref, "shader must be emitted before texset");
        assert!(texset_ref < alpha_ref, "texset must be emitted before alpha");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_material_source_bakes_both_shapes_into_one_oc_file_with_all_invariants() {
        let dir = temp_dir("multishape");
        let source_path = dir.join("data").join("meshes").join("test").join("multi.nif");

        let mut nif = NifFile::new("fo4");
        // Two DISTINCT materials (different texset/shader blocks) so the
        // test proves each output shape clones its OWN property chain, not
        // a shared/cross-wired one.
        let texset_a = nif.add_block(
            "BSShaderTextureSet",
            Some(struct_fields([
                ("Num Textures", NifValue::UInt(1)),
                (
                    "Textures",
                    NifValue::Array(vec![NifValue::String(r"textures\test\a.dds".to_string())]),
                ),
            ])),
        );
        let shader_a = nif.add_block(
            "BSLightingShaderProperty",
            Some(struct_fields([
                ("Name", NifValue::String(String::new())),
                ("Texture Set", NifValue::Ref(texset_a as i32)),
            ])),
        );
        let texset_b = nif.add_block(
            "BSShaderTextureSet",
            Some(struct_fields([
                ("Num Textures", NifValue::UInt(1)),
                (
                    "Textures",
                    NifValue::Array(vec![NifValue::String(r"textures\test\b.dds".to_string())]),
                ),
            ])),
        );
        let shader_b = nif.add_block(
            "BSLightingShaderProperty",
            Some(struct_fields([
                ("Name", NifValue::String(String::new())),
                ("Texture Set", NifValue::Ref(texset_b as i32)),
            ])),
        );

        // Distinguishable geometry (4 verts vs 3 verts) so the test can tell
        // which output shape corresponds to which source shape.
        let (vertex_data_a, triangles_a) = simple_geometry();
        let vertex_count_a = vertex_data_a.len();
        let triangle_count_a = triangles_a.len();
        let (vertex_data_b, triangles_b) = small_geometry();
        let vertex_count_b = vertex_data_b.len();
        let triangle_count_b = triangles_b.len();
        assert_ne!(vertex_count_a, vertex_count_b, "fixtures must be distinguishable");

        let shape_a = nif.add_block(
            "BSTriShape",
            Some(struct_fields([
                ("Bounding Sphere", simple_bound()),
                ("Skin", NifValue::Ref(-1)),
                ("Shader Property", NifValue::Ref(shader_a as i32)),
                ("Alpha Property", NifValue::Ref(-1)),
                ("Vertex Desc", NifValue::Int(basic_vertex_desc())),
                ("Num Triangles", NifValue::UInt(triangle_count_a as u64)),
                ("Num Vertices", NifValue::UInt(vertex_count_a as u64)),
                ("Vertex Data", NifValue::Array(vertex_data_a)),
                ("Triangles", NifValue::Array(triangles_a)),
            ])),
        );
        let shape_b = nif.add_block(
            "BSTriShape",
            Some(struct_fields([
                ("Bounding Sphere", simple_bound()),
                ("Skin", NifValue::Ref(-1)),
                ("Shader Property", NifValue::Ref(shader_b as i32)),
                ("Alpha Property", NifValue::Ref(-1)),
                ("Vertex Desc", NifValue::Int(basic_vertex_desc())),
                ("Num Triangles", NifValue::UInt(triangle_count_b as u64)),
                ("Num Vertices", NifValue::UInt(vertex_count_b as u64)),
                ("Vertex Data", NifValue::Array(vertex_data_b)),
                ("Triangles", NifValue::Array(triangles_b)),
            ])),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(shape_a as i32),
                NifValue::Ref(shape_b as i32),
            ]),
        );
        std::fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        nif.save(Some(source_path.clone())).expect("write multi-shape source");

        // 2 instances, second rotated, so expanded-count and
        // transposed-rotation invariants are meaningfully exercised per shape.
        let group = ModelGroup {
            model_path: "meshes\\test\\multi.nif".to_string(),
            instances: vec![
                InstanceRef {
                    refr_form_id: 0x0100_0700,
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: 1.0,
                },
                InstanceRef {
                    refr_form_id: 0x0100_0701,
                    position: [10.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, std::f32::consts::FRAC_PI_2],
                    scale: 1.0,
                },
            ],
        };
        let plan = CellPlan {
            cell_form_id: 0x0010_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x0010_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: Vec::new(),
        };

        let report = bake_cell(&plan, &params);
        assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
        let baked = report.baked.expect("two-material group should bake");
        assert_eq!(baked.meshes.len(), 1, "both materials pack into ONE _OC.nif");
        let mesh = &baked.meshes[0];

        let dest = {
            let mut p = params.data_root.clone();
            p.push("meshes");
            p.push("precombined");
            p.push("Test.esm");
            p.push(format!("00100000_{:08X}_OC.nif", mesh.mesh_id));
            p
        };
        let out = NifFile::load(&dest).expect("reload baked nif");

        let root = out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSFadeNode")
            .expect("root BSFadeNode");
        let inner_ref = match root.get_field("Children") {
            Some(NifValue::Array(items)) if items.len() == 1 => match &items[0] {
                NifValue::Ref(r) => *r,
                other => panic!("expected ref, got {other:?}"),
            },
            other => panic!("expected exactly one root child, got {other:?}"),
        };
        let inner = &out.blocks[inner_ref as usize];
        assert_eq!(inner.type_name, "NiNode");
        assert_eq!(
            inner.get_field("Num Children").map(|v| v.as_i64()),
            Some(2),
            "inner NiNode must carry one child per material"
        );
        let inner_children = match inner.get_field("Children") {
            Some(NifValue::Array(items)) => items.clone(),
            other => panic!("expected inner children, got {other:?}"),
        };
        assert_eq!(inner_children.len(), 2);

        // Locate each output shape by its distinguishing vertex count and
        // verify all four engine invariants independently per shape.
        let shape_refs: Vec<i32> = inner_children
            .iter()
            .map(|v| match v {
                NifValue::Ref(r) => *r,
                other => panic!("expected ref, got {other:?}"),
            })
            .collect();
        assert!(
            shape_refs.windows(2).all(|w| w[0] < w[1]),
            "shapes must be emitted in source order: {shape_refs:?}"
        );

        let mut found_a = false;
        let mut found_b = false;
        for &shape_ref in &shape_refs {
            let shape = &out.blocks[shape_ref as usize];
            assert_eq!(shape.type_name, "BSTriShape");
            let expanded_verts = shape.get_field("Num Vertices").map(|v| v.as_i64());
            let expanded_triangles = shape.get_field("Num Triangles").map(|v| v.as_i64());
            // Invariant: full-prec desc promotion (same for both shapes here
            // since both source shapes share `basic_vertex_desc`).
            assert_eq!(
                shape.get_field("Vertex Desc").map(|v| v.as_i64()),
                Some(promote_to_full_precision(basic_vertex_desc() as u64) as i64)
            );

            let pcd_ref = match shape.get_field("Extra Data List") {
                Some(NifValue::Array(items)) => match &items[0] {
                    NifValue::Ref(r) => *r,
                    other => panic!("expected ref, got {other:?}"),
                },
                other => panic!("expected extra data list, got {other:?}"),
            };
            let pcd = &out.blocks[pcd_ref as usize];
            assert_eq!(pcd.type_name, "BSPackedCombinedGeomDataExtra");
            let object_data = match pcd.get_field("Object Data") {
                Some(NifValue::Array(items)) => items.clone(),
                other => panic!("expected object data array, got {other:?}"),
            };
            let geom = match &object_data[0] {
                NifValue::Struct(m) => m.clone(),
                other => panic!("expected geom struct, got {other:?}"),
            };
            let combined = match geom.get("Combined") {
                Some(NifValue::Array(items)) => items.clone(),
                other => panic!("expected combined array, got {other:?}"),
            };
            assert_eq!(combined.len(), 2, "one combined row per instance");
            let second_rotation = match &combined[1] {
                NifValue::Struct(fields) => match fields.get("Transform") {
                    Some(NifValue::Struct(t)) => {
                        read_matrix33(t.get("Rotation")).expect("combined rotation matrix33")
                    }
                    other => panic!("expected transform struct, got {other:?}"),
                },
                other => panic!("expected combined row struct, got {other:?}"),
            };
            let expected_transpose = [[0.0f32, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
            for i in 0..3 {
                for j in 0..3 {
                    assert!(
                        (second_rotation[i][j] - expected_transpose[i][j]).abs() < 1e-4,
                        "shape at block {shape_ref}: combined rotation must be stored \
                         transposed: got {second_rotation:?}"
                    );
                }
            }

            // Which source shape is this? Distinguish by dedup'd inner count
            // (== source per-instance count) vs the expanded (x2) top-level
            // count.
            let inner_verts = geom.get("Num Verts").map(|v| v.as_i64());
            if inner_verts == Some(vertex_count_a as i64) {
                found_a = true;
                assert_eq!(expanded_verts, Some((vertex_count_a * 2) as i64));
                assert_eq!(expanded_triangles, Some((triangle_count_a * 2) as i64));
                let shader_ref = match shape.get_field("Shader Property") {
                    Some(NifValue::Ref(r)) => *r,
                    other => panic!("expected shader ref, got {other:?}"),
                };
                let texset_ref = match out.blocks[shader_ref as usize].get_field("Texture Set") {
                    Some(NifValue::Ref(r)) => *r,
                    other => panic!("expected texset ref, got {other:?}"),
                };
                match &out.blocks[texset_ref as usize].get_field("Textures") {
                    Some(NifValue::Array(items)) => match &items[0] {
                        NifValue::String(s) => assert_eq!(s, r"textures\test\a.dds"),
                        other => panic!("expected string, got {other:?}"),
                    },
                    other => panic!("expected textures array, got {other:?}"),
                }
            } else if inner_verts == Some(vertex_count_b as i64) {
                found_b = true;
                assert_eq!(expanded_verts, Some((vertex_count_b * 2) as i64));
                assert_eq!(expanded_triangles, Some((triangle_count_b * 2) as i64));
                let shader_ref = match shape.get_field("Shader Property") {
                    Some(NifValue::Ref(r)) => *r,
                    other => panic!("expected shader ref, got {other:?}"),
                };
                let texset_ref = match out.blocks[shader_ref as usize].get_field("Texture Set") {
                    Some(NifValue::Ref(r)) => *r,
                    other => panic!("expected texset ref, got {other:?}"),
                };
                match &out.blocks[texset_ref as usize].get_field("Textures") {
                    Some(NifValue::Array(items)) => match &items[0] {
                        NifValue::String(s) => assert_eq!(s, r"textures\test\b.dds"),
                        other => panic!("expected string, got {other:?}"),
                    },
                    other => panic!("expected textures array, got {other:?}"),
                }
            } else {
                panic!("output shape's inner vertex count {inner_verts:?} matches neither source shape");
            }
        }
        assert!(found_a && found_b, "both source shapes must appear in the output");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_unsupported_shape_rejects_the_whole_multi_shape_group() {
        let dir = temp_dir("multishape_partial_invalid");
        let source_path = dir.join("data").join("meshes").join("test").join("mixed.nif");

        let mut nif = NifFile::new("fo4");
        let texset_id = nif.add_block(
            "BSShaderTextureSet",
            Some(struct_fields([
                ("Num Textures", NifValue::UInt(1)),
                (
                    "Textures",
                    NifValue::Array(vec![NifValue::String(r"textures\test\a.dds".to_string())]),
                ),
            ])),
        );
        let shader_id = nif.add_block(
            "BSLightingShaderProperty",
            Some(struct_fields([
                ("Name", NifValue::String(String::new())),
                ("Texture Set", NifValue::Ref(texset_id as i32)),
            ])),
        );
        let (vertex_data, triangles) = simple_geometry();
        let shape_fields = |skin: NifValue| {
            struct_fields([
                ("Bounding Sphere", simple_bound()),
                ("Skin", skin),
                ("Shader Property", NifValue::Ref(shader_id as i32)),
                ("Alpha Property", NifValue::Ref(-1)),
                ("Vertex Desc", NifValue::Int(basic_vertex_desc())),
                ("Num Triangles", NifValue::UInt(triangles.len() as u64)),
                ("Num Vertices", NifValue::UInt(vertex_data.len() as u64)),
                ("Vertex Data", NifValue::Array(vertex_data.clone())),
                ("Triangles", NifValue::Array(triangles.clone())),
            ])
        };
        // Valid shape.
        let shape_ok = nif.add_block("BSTriShape", Some(shape_fields(NifValue::Ref(-1))));
        // Skinned shape — unsupported; must sink the WHOLE group, not just
        // itself (v0's "no partial geometry per reference" invariant).
        let shape_skinned = nif.add_block("BSTriShape", Some(shape_fields(NifValue::Ref(0))));
        nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![
                NifValue::Ref(shape_ok as i32),
                NifValue::Ref(shape_skinned as i32),
            ]),
        );
        std::fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        nif.save(Some(source_path.clone())).expect("write mixed-validity source");

        let group = ModelGroup {
            model_path: "meshes\\test\\mixed.nif".to_string(),
            instances: vec![InstanceRef {
                refr_form_id: 0x0100_0800,
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
            }],
        };
        let plan = CellPlan {
            cell_form_id: 0x0011_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x0011_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: Vec::new(),
        };

        let report = bake_cell(&plan, &params);
        assert!(
            report.baked.is_none(),
            "one unsupported shape must reject the whole group, not partially bake"
        );
        assert_eq!(report.warnings.len(), 1);
        assert!(
            report.warnings[0].contains("skinned shapes are unsupported"),
            "{:?}",
            report.warnings
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn group_with_missing_mesh(model_path: &str, refr_form_id: u32) -> ModelGroup {
        ModelGroup {
            model_path: model_path.to_string(),
            instances: vec![InstanceRef {
                refr_form_id,
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
            }],
        }
    }

    #[test]
    fn missing_source_mesh_names_the_resolved_path_and_archives_tried() {
        let dir = temp_dir("missing_mesh");
        // Deliberately never write anything at this path, and configure no
        // mesh_archives — simulates a bad MODL path (or an unconfigured
        // archive) that MINOR-2 covers.
        let group = group_with_missing_mesh("meshes\\test\\ba2_only.nif", 0x0100_0800);
        let plan = CellPlan {
            cell_form_id: 0x0020_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x0020_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: Vec::new(),
        };

        let report = bake_cell(&plan, &params);
        assert!(report.baked.is_none(), "zero-mesh cell must not be stampable");

        let expected_path = dir.join("data").join("meshes").join("test").join("ba2_only.nif");
        let expected_path_str = expected_path.display().to_string();

        let per_group_warning = report
            .warnings
            .iter()
            .find(|w| w.starts_with("group "))
            .expect("per-group warning present");
        assert!(
            per_group_warning.contains("not found on disk as a loose file"),
            "{per_group_warning}"
        );
        assert!(
            per_group_warning.contains("mesh_archives"),
            "{per_group_warning}"
        );
        assert!(
            per_group_warning.contains(&expected_path_str),
            "warning should name the resolved path: {per_group_warning}"
        );

        let summary = report
            .warnings
            .iter()
            .find(|w| w.starts_with("cell "))
            .expect("zero-mesh summary warning present");
        assert!(summary.contains("produced zero meshes"), "{summary}");
        assert!(
            summary.contains("not found on disk as a loose file"),
            "{summary}"
        );
        assert!(summary.contains(&expected_path_str), "{summary}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zero_mesh_summary_caps_listed_paths_and_counts_the_rest() {
        let dir = temp_dir("cap_test");
        let groups: Vec<ModelGroup> = (0..12)
            .map(|i| {
                group_with_missing_mesh(
                    &format!("meshes\\test\\missing_{i:02}.nif"),
                    0x0100_0900 + i as u32,
                )
            })
            .collect();
        let plan = CellPlan {
            cell_form_id: 0x0030_0000,
            groups,
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x0030_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: Vec::new(),
        };

        let report = bake_cell(&plan, &params);
        assert!(report.baked.is_none());
        assert_eq!(report.warnings.len(), 13, "12 per-group + 1 summary");

        let summary = report
            .warnings
            .iter()
            .find(|w| w.starts_with("cell "))
            .expect("zero-mesh summary warning present");
        assert!(summary.contains("12 of its group(s)"), "{summary}");
        assert!(summary.contains("and 2 more"), "{summary}");
        // Exactly MISSING_MESH_PATHS_SHOWN (10) distinct paths listed by name.
        assert_eq!(summary.matches("missing_").count(), MISSING_MESH_PATHS_SHOWN);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_data_relative_path_prepends_meshes_when_missing() {
        // Real MODL paths are relative to `Data\Meshes\` by game convention
        // and do NOT carry a leading "meshes" component (verified against
        // the WhitespringMall01 T6b real run, e.g.
        // `architecture\cabin\cabinmodernwalla01.nif`).
        let data_root = PathBuf::from("C:\\mod\\data");
        let resolved = resolve_data_relative_path(&data_root, "architecture\\cabin\\wall01.nif");
        assert_eq!(
            resolved,
            data_root.join("meshes").join("architecture").join("cabin").join("wall01.nif")
        );
    }

    #[test]
    fn resolve_data_relative_path_does_not_double_prefix_meshes() {
        // Some MODL strings already carry the "meshes" component; the
        // resolver must detect that (case-insensitively) and not join it
        // twice.
        let data_root = PathBuf::from("C:\\mod\\data");
        let resolved = resolve_data_relative_path(&data_root, "Meshes\\test\\chair01.nif");
        assert_eq!(resolved, data_root.join("Meshes").join("test").join("chair01.nif"));
    }

    #[test]
    fn bakes_group_whose_model_path_lacks_the_meshes_prefix() {
        // Regression test for the T6b real-run bug: every group in the
        // WhitespringMall01 run skipped because source meshes were resolved
        // at `<data_root>\<model_path>` instead of `<data_root>\meshes\<model_path>`.
        let dir = temp_dir("no_meshes_prefix");
        let source_path = dir
            .join("data")
            .join("meshes")
            .join("architecture")
            .join("wall01.nif");
        write_source_fixture(&source_path, false);

        let group = ModelGroup {
            model_path: "architecture\\wall01.nif".to_string(),
            instances: vec![InstanceRef {
                refr_form_id: 0x0100_0A00,
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
            }],
        };
        let plan = CellPlan {
            cell_form_id: 0x0040_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x0040_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: Vec::new(),
        };

        let report = bake_cell(&plan, &params);
        assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
        assert!(
            report.baked.is_some(),
            "group must bake once the meshes/ prefix is applied to an unprefixed model path"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_instance_expansion_rejects_group_without_truncating() {
        // 2 instances of a 40,000-vertex mesh expand to 80,000 — past the
        // shape's u16 Num Vertices capacity (65,535) — and must reject the
        // whole group loudly rather than truncate or overflow.
        let dir = temp_dir("guard_oversize");
        let source_path = dir.join("data").join("meshes").join("test").join("huge.nif");

        let vertex_count = 40_000usize;
        let mut nif = NifFile::new("fo4");
        let texset_id = nif.add_block(
            "BSShaderTextureSet",
            Some(struct_fields([
                ("Num Textures", NifValue::UInt(1)),
                (
                    "Textures",
                    NifValue::Array(vec![NifValue::String(r"textures\test\a.dds".to_string())]),
                ),
            ])),
        );
        let shader_id = nif.add_block(
            "BSLightingShaderProperty",
            Some(struct_fields([
                ("Name", NifValue::String(String::new())),
                ("Texture Set", NifValue::Ref(texset_id as i32)),
            ])),
        );
        let vertex_data: Vec<NifValue> = (0..vertex_count)
            .map(|i| basic_vertex([i as f32, 0.0, 0.0]))
            .collect();
        let triangles = vec![triangle(0, 1, 2)];
        let shape_id = nif.add_block(
            "BSTriShape",
            Some(struct_fields([
                ("Name", NifValue::String("Shape:0".to_string())),
                ("Bounding Sphere", simple_bound()),
                ("Skin", NifValue::Ref(-1)),
                ("Shader Property", NifValue::Ref(shader_id as i32)),
                ("Alpha Property", NifValue::Ref(-1)),
                ("Vertex Desc", NifValue::Int(basic_vertex_desc())),
                ("Num Triangles", NifValue::UInt(triangles.len() as u64)),
                ("Num Vertices", NifValue::UInt(vertex_count as u64)),
                ("Vertex Data", NifValue::Array(vertex_data)),
                ("Triangles", NifValue::Array(triangles)),
            ])),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );
        std::fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        nif.save(Some(source_path.clone())).expect("write oversized source nif");

        let group = ModelGroup {
            model_path: "meshes\\test\\huge.nif".to_string(),
            instances: vec![
                InstanceRef {
                    refr_form_id: 0x0100_0B00,
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: 1.0,
                },
                InstanceRef {
                    refr_form_id: 0x0100_0B01,
                    position: [10.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: 1.0,
                },
            ],
        };
        let plan = CellPlan {
            cell_form_id: 0x0050_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x0050_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: Vec::new(),
        };

        let report = bake_cell(&plan, &params);
        assert!(report.baked.is_none(), "oversized expansion must not be stampable");
        let warning = report
            .warnings
            .iter()
            .find(|w| w.starts_with("group "))
            .expect("per-group rejection warning present");
        assert!(warning.contains("65535"), "{warning}");
        assert!(warning.contains("80000"), "{warning}");

        let out_dir = dir.join("data").join("meshes").join("precombined").join("Test.esm");
        assert!(
            !out_dir.exists() || std::fs::read_dir(&out_dir).unwrap().next().is_none(),
            "rejected group must not leave a truncated output file behind"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_only_mesh_bakes_successfully_with_all_invariants() {
        let dir = temp_dir("archive_only");
        let (vertex_data, triangles) = simple_geometry();
        let vertex_count = vertex_data.len();
        let triangle_count = triangles.len();
        let bytes = build_source_fixture_bytes(vertex_data, triangles);

        let archive_path = dir.join("archives").join("Fallout4 - Meshes.ba2");
        write_ba2_fixture(&archive_path, &[("meshes\\test\\archived01.nif", &bytes)]);

        let group = ModelGroup {
            model_path: "meshes\\test\\archived01.nif".to_string(),
            instances: vec![
                InstanceRef {
                    refr_form_id: 0x0100_0C00,
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: 1.0,
                },
                InstanceRef {
                    refr_form_id: 0x0100_0C01,
                    position: [10.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, std::f32::consts::FRAC_PI_2],
                    scale: 1.0,
                },
            ],
        };
        let plan = CellPlan {
            cell_form_id: 0x0060_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"), // no loose mesh written here — archive-only
            include_cells: vec![0x0060_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: vec![archive_path],
        };

        let report = bake_cell(&plan, &params);
        assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
        let baked = report.baked.expect("archive-sourced group should bake");
        let mesh = &baked.meshes[0];

        let dest = {
            let mut p = params.data_root.clone();
            p.push("meshes");
            p.push("precombined");
            p.push("Test.esm");
            p.push(format!("00600000_{:08X}_OC.nif", mesh.mesh_id));
            p
        };
        let out = NifFile::load(&dest).expect("reload baked nif");
        let shape = out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSTriShape")
            .expect("shape block");

        // Invariant 1+3: real, instance-EXPANDED counts (2 instances of the
        // archive-sourced 4-vert/2-tri mesh).
        assert_eq!(
            shape.get_field("Num Vertices").map(|v| v.as_i64()),
            Some((vertex_count * 2) as i64)
        );
        assert_eq!(
            shape.get_field("Num Triangles").map(|v| v.as_i64()),
            Some((triangle_count * 2) as i64)
        );
        // Invariant 2: shape Vertex Desc promoted to Full_Precision.
        assert_eq!(
            shape.get_field("Vertex Desc").map(|v| v.as_i64()),
            Some(promote_to_full_precision(basic_vertex_desc() as u64) as i64)
        );

        let pcd_ref = match shape.get_field("Extra Data List") {
            Some(NifValue::Array(items)) => match &items[0] {
                NifValue::Ref(r) => *r,
                other => panic!("expected ref, got {other:?}"),
            },
            other => panic!("expected extra data list, got {other:?}"),
        };
        let pcd = &out.blocks[pcd_ref as usize];
        let object_data = match pcd.get_field("Object Data") {
            Some(NifValue::Array(items)) => items.clone(),
            other => panic!("expected object data array, got {other:?}"),
        };
        let geom = match &object_data[0] {
            NifValue::Struct(m) => m.clone(),
            other => panic!("expected geom struct, got {other:?}"),
        };
        let combined = match geom.get("Combined") {
            Some(NifValue::Array(items)) => items.clone(),
            other => panic!("expected combined array, got {other:?}"),
        };
        // Invariant 4: Combined-row rotation stored transposed. Second
        // instance has rot Z=+90deg; true Rz(90)=[[0,-1,0],[1,0,0],[0,0,1]],
        // so the stored/decoded matrix must be its transpose.
        let second_rotation = match &combined[1] {
            NifValue::Struct(fields) => match fields.get("Transform") {
                Some(NifValue::Struct(t)) => {
                    read_matrix33(t.get("Rotation")).expect("combined rotation matrix33")
                }
                other => panic!("expected transform struct, got {other:?}"),
            },
            other => panic!("expected combined row struct, got {other:?}"),
        };
        let expected_transpose = [[0.0f32, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (second_rotation[i][j] - expected_transpose[i][j]).abs() < 1e-4,
                    "archive-sourced combined rotation must be stored transposed: got \
                     {second_rotation:?}, expected {expected_transpose:?}"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loose_file_wins_over_archive_copy() {
        let dir = temp_dir("loose_wins");
        let source_path = dir.join("data").join("meshes").join("test").join("dual01.nif");
        let (loose_vertex_count, _) = write_source_fixture(&source_path, false);

        let (archive_vertex_data, archive_triangles) = small_geometry();
        let archive_vertex_count = archive_vertex_data.len();
        assert_ne!(
            loose_vertex_count, archive_vertex_count,
            "fixtures must be distinguishable by vertex count"
        );
        let archive_bytes = build_source_fixture_bytes(archive_vertex_data, archive_triangles);
        let archive_path = dir.join("archives").join("Fallout4 - Meshes.ba2");
        write_ba2_fixture(&archive_path, &[("meshes\\test\\dual01.nif", &archive_bytes)]);

        let group = ModelGroup {
            model_path: "meshes\\test\\dual01.nif".to_string(),
            instances: vec![InstanceRef {
                refr_form_id: 0x0100_0D00,
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
            }],
        };
        let plan = CellPlan {
            cell_form_id: 0x0070_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x0070_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: vec![archive_path],
        };

        let report = bake_cell(&plan, &params);
        assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
        let baked = report.baked.expect("group should bake from the loose file");
        let mesh = &baked.meshes[0];

        let dest = {
            let mut p = params.data_root.clone();
            p.push("meshes");
            p.push("precombined");
            p.push("Test.esm");
            p.push(format!("00700000_{:08X}_OC.nif", mesh.mesh_id));
            p
        };
        let out = NifFile::load(&dest).expect("reload baked nif");
        let shape = out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSTriShape")
            .expect("shape block");
        // 1 instance, so expanded count == the per-instance count of
        // whichever source actually got baked.
        assert_eq!(
            shape.get_field("Num Vertices").map(|v| v.as_i64()),
            Some(loose_vertex_count as i64),
            "loose file must win over the archive copy of the same model_path"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_order_respected_first_configured_archive_wins() {
        let dir = temp_dir("archive_order");
        let (first_vertex_data, first_triangles) = simple_geometry();
        let first_vertex_count = first_vertex_data.len();
        let (second_vertex_data, second_triangles) = small_geometry();
        let second_vertex_count = second_vertex_data.len();
        assert_ne!(
            first_vertex_count, second_vertex_count,
            "fixtures must be distinguishable by vertex count"
        );

        let first_bytes = build_source_fixture_bytes(first_vertex_data, first_triangles);
        let second_bytes = build_source_fixture_bytes(second_vertex_data, second_triangles);
        let archive_a = dir.join("archives").join("a_first.ba2");
        let archive_b = dir.join("archives").join("b_second.ba2");
        write_ba2_fixture(&archive_a, &[("meshes\\test\\ordered01.nif", &first_bytes)]);
        write_ba2_fixture(&archive_b, &[("meshes\\test\\ordered01.nif", &second_bytes)]);

        let group = ModelGroup {
            model_path: "meshes\\test\\ordered01.nif".to_string(),
            instances: vec![InstanceRef {
                refr_form_id: 0x0100_0E00,
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
            }],
        };
        let plan = CellPlan {
            cell_form_id: 0x0080_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x0080_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: vec![archive_a, archive_b],
        };

        let report = bake_cell(&plan, &params);
        assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
        let baked = report.baked.expect("group should bake from the first archive");
        let mesh = &baked.meshes[0];

        let dest = {
            let mut p = params.data_root.clone();
            p.push("meshes");
            p.push("precombined");
            p.push("Test.esm");
            p.push(format!("00800000_{:08X}_OC.nif", mesh.mesh_id));
            p
        };
        let out = NifFile::load(&dest).expect("reload baked nif");
        let shape = out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSTriShape")
            .expect("shape block");
        assert_eq!(
            shape.get_field("Num Vertices").map(|v| v.as_i64()),
            Some(first_vertex_count as i64),
            "first archive in mesh_archives order must win over a later one"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn miss_in_loose_extract_roots_and_archives_warns_naming_both_consulted() {
        let dir = temp_dir("archive_miss");
        // An extract root and an archive that both open/resolve fine but
        // neither contains the requested member — distinct from the "path
        // doesn't even exist/open" case.
        let extract_root = dir.join("extract");
        let (vertex_data, triangles) = simple_geometry();
        let bytes = build_source_fixture_bytes(vertex_data, triangles);
        let archive_path = dir.join("archives").join("Fallout4 - Meshes.ba2");
        write_ba2_fixture(&archive_path, &[("meshes\\test\\unrelated.nif", &bytes)]);

        let group = group_with_missing_mesh("meshes\\test\\nowhere.nif", 0x0100_0F00);
        let plan = CellPlan {
            cell_form_id: 0x0090_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x0090_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: vec![extract_root.clone()],
            mesh_archives: vec![archive_path.clone()],
        };

        let report = bake_cell(&plan, &params);
        assert!(
            report.baked.is_none(),
            "miss in loose, extract roots, and archives must not be stampable"
        );

        let per_group_warning = report
            .warnings
            .iter()
            .find(|w| w.starts_with("group "))
            .expect("per-group warning present");
        assert!(
            per_group_warning.contains("not found on disk as a loose file"),
            "{per_group_warning}"
        );
        let expected_extract_path = extract_root.join("meshes").join("test").join("nowhere.nif");
        assert!(
            per_group_warning.contains(&expected_extract_path.display().to_string()),
            "warning should name the extract root(s) consulted: {per_group_warning}"
        );
        assert!(
            per_group_warning.contains(&archive_path.display().to_string()),
            "warning should name the archive(s) consulted: {per_group_warning}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mesh_extract_root_wins_over_archive_copy() {
        let dir = temp_dir("extract_root_wins");
        let extract_root = dir.join("extract");
        let extract_path = extract_root.join("meshes").join("test").join("extracted01.nif");
        let (extract_vertex_count, _) = write_source_fixture(&extract_path, false);

        let (archive_vertex_data, archive_triangles) = small_geometry();
        let archive_vertex_count = archive_vertex_data.len();
        assert_ne!(
            extract_vertex_count, archive_vertex_count,
            "fixtures must be distinguishable by vertex count"
        );
        let archive_bytes = build_source_fixture_bytes(archive_vertex_data, archive_triangles);
        let archive_path = dir.join("archives").join("Fallout4 - Meshes.ba2");
        write_ba2_fixture(&archive_path, &[("meshes\\test\\extracted01.nif", &archive_bytes)]);

        let group = ModelGroup {
            model_path: "meshes\\test\\extracted01.nif".to_string(),
            instances: vec![InstanceRef {
                refr_form_id: 0x0100_1000,
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
            }],
        };
        let plan = CellPlan {
            cell_form_id: 0x00A0_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"), // no loose mesh here — must fall to extract root
            include_cells: vec![0x00A0_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: vec![extract_root],
            mesh_archives: vec![archive_path],
        };

        let report = bake_cell(&plan, &params);
        assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
        let baked = report.baked.expect("group should bake from the extract root");
        let mesh = &baked.meshes[0];

        let dest = {
            let mut p = params.data_root.clone();
            p.push("meshes");
            p.push("precombined");
            p.push("Test.esm");
            p.push(format!("00A00000_{:08X}_OC.nif", mesh.mesh_id));
            p
        };
        let out = NifFile::load(&dest).expect("reload baked nif");
        let shape = out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSTriShape")
            .expect("shape block");
        assert_eq!(
            shape.get_field("Num Vertices").map(|v| v.as_i64()),
            Some(extract_vertex_count as i64),
            "mesh_extract_roots must win over a mesh_archives copy of the same model_path"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_root_only_mesh_bakes_successfully_with_all_invariants() {
        let dir = temp_dir("extract_only");
        let extract_root = dir.join("extract");
        let extract_path = extract_root.join("meshes").join("test").join("exonly01.nif");
        let (vertex_count, triangle_count) = write_source_fixture(&extract_path, false);

        let group = ModelGroup {
            model_path: "meshes\\test\\exonly01.nif".to_string(),
            instances: vec![
                InstanceRef {
                    refr_form_id: 0x0100_1100,
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0],
                    scale: 1.0,
                },
                InstanceRef {
                    refr_form_id: 0x0100_1101,
                    position: [10.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, std::f32::consts::FRAC_PI_2],
                    scale: 1.0,
                },
            ],
        };
        let plan = CellPlan {
            cell_form_id: 0x00B0_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"), // no loose mesh here, and no archives at all
            include_cells: vec![0x00B0_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: vec![extract_root],
            mesh_archives: Vec::new(),
        };

        let report = bake_cell(&plan, &params);
        assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
        let baked = report.baked.expect("extract-root-only group should bake");
        let mesh = &baked.meshes[0];

        let dest = {
            let mut p = params.data_root.clone();
            p.push("meshes");
            p.push("precombined");
            p.push("Test.esm");
            p.push(format!("00B00000_{:08X}_OC.nif", mesh.mesh_id));
            p
        };
        let out = NifFile::load(&dest).expect("reload baked nif");
        let shape = out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSTriShape")
            .expect("shape block");

        // Instance-expanded counts (2 instances of the 4-vert/2-tri mesh).
        assert_eq!(
            shape.get_field("Num Vertices").map(|v| v.as_i64()),
            Some((vertex_count * 2) as i64)
        );
        assert_eq!(
            shape.get_field("Num Triangles").map(|v| v.as_i64()),
            Some((triangle_count * 2) as i64)
        );
        // Full_Precision desc promotion.
        assert_eq!(
            shape.get_field("Vertex Desc").map(|v| v.as_i64()),
            Some(promote_to_full_precision(basic_vertex_desc() as u64) as i64)
        );

        let pcd_ref = match shape.get_field("Extra Data List") {
            Some(NifValue::Array(items)) => match &items[0] {
                NifValue::Ref(r) => *r,
                other => panic!("expected ref, got {other:?}"),
            },
            other => panic!("expected extra data list, got {other:?}"),
        };
        let pcd = &out.blocks[pcd_ref as usize];
        let object_data = match pcd.get_field("Object Data") {
            Some(NifValue::Array(items)) => items.clone(),
            other => panic!("expected object data array, got {other:?}"),
        };
        let geom = match &object_data[0] {
            NifValue::Struct(m) => m.clone(),
            other => panic!("expected geom struct, got {other:?}"),
        };
        let combined = match geom.get("Combined") {
            Some(NifValue::Array(items)) => items.clone(),
            other => panic!("expected combined array, got {other:?}"),
        };
        // Transposed Combined-row rotation.
        let second_rotation = match &combined[1] {
            NifValue::Struct(fields) => match fields.get("Transform") {
                Some(NifValue::Struct(t)) => {
                    read_matrix33(t.get("Rotation")).expect("combined rotation matrix33")
                }
                other => panic!("expected transform struct, got {other:?}"),
            },
            other => panic!("expected combined row struct, got {other:?}"),
        };
        let expected_transpose = [[0.0f32, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (second_rotation[i][j] - expected_transpose[i][j]).abs() < 1e-4,
                    "extract-root-sourced combined rotation must be stored transposed: got \
                     {second_rotation:?}, expected {expected_transpose:?}"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loose_file_wins_over_extract_root_copy() {
        let dir = temp_dir("loose_beats_extract");
        let source_path = dir.join("data").join("meshes").join("test").join("dual02.nif");
        let (loose_vertex_count, _) = write_source_fixture(&source_path, false);

        let extract_root = dir.join("extract");
        let extract_path = extract_root.join("meshes").join("test").join("dual02.nif");
        let (extract_vertex_data, extract_triangles) = small_geometry();
        let extract_vertex_count = extract_vertex_data.len();
        assert_ne!(
            loose_vertex_count, extract_vertex_count,
            "fixtures must be distinguishable by vertex count"
        );
        write_fixture_with_geometry(&extract_path, extract_vertex_data, extract_triangles);

        let group = ModelGroup {
            model_path: "meshes\\test\\dual02.nif".to_string(),
            instances: vec![InstanceRef {
                refr_form_id: 0x0100_1200,
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
            }],
        };
        let plan = CellPlan {
            cell_form_id: 0x00C0_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x00C0_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: vec![extract_root],
            mesh_archives: Vec::new(),
        };

        let report = bake_cell(&plan, &params);
        assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
        let baked = report.baked.expect("group should bake from the loose file");
        let mesh = &baked.meshes[0];

        let dest = {
            let mut p = params.data_root.clone();
            p.push("meshes");
            p.push("precombined");
            p.push("Test.esm");
            p.push(format!("00C00000_{:08X}_OC.nif", mesh.mesh_id));
            p
        };
        let out = NifFile::load(&dest).expect("reload baked nif");
        let shape = out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSTriShape")
            .expect("shape block");
        assert_eq!(
            shape.get_field("Num Vertices").map(|v| v.as_i64()),
            Some(loose_vertex_count as i64),
            "the mod loose root must win over a mesh_extract_roots copy of the same model_path"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_root_order_respected_first_configured_root_wins() {
        let dir = temp_dir("extract_order");
        let (first_vertex_data, first_triangles) = simple_geometry();
        let first_vertex_count = first_vertex_data.len();
        let (second_vertex_data, second_triangles) = small_geometry();
        let second_vertex_count = second_vertex_data.len();
        assert_ne!(
            first_vertex_count, second_vertex_count,
            "fixtures must be distinguishable by vertex count"
        );

        let root_a = dir.join("extract_a");
        let root_b = dir.join("extract_b");
        write_fixture_with_geometry(
            &root_a.join("meshes").join("test").join("ordered02.nif"),
            first_vertex_data,
            first_triangles,
        );
        write_fixture_with_geometry(
            &root_b.join("meshes").join("test").join("ordered02.nif"),
            second_vertex_data,
            second_triangles,
        );

        let group = ModelGroup {
            model_path: "meshes\\test\\ordered02.nif".to_string(),
            instances: vec![InstanceRef {
                refr_form_id: 0x0100_1300,
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
            }],
        };
        let plan = CellPlan {
            cell_form_id: 0x00D0_0000,
            groups: vec![group],
        };
        let params = Params {
            target_handle_id: 0,
            plugin_name: "Test.esm".to_string(),
            data_root: dir.join("data"),
            include_cells: vec![0x00D0_0000],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: vec![root_a, root_b],
            mesh_archives: Vec::new(),
        };

        let report = bake_cell(&plan, &params);
        assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);
        let baked = report.baked.expect("group should bake from the first extract root");
        let mesh = &baked.meshes[0];

        let dest = {
            let mut p = params.data_root.clone();
            p.push("meshes");
            p.push("precombined");
            p.push("Test.esm");
            p.push(format!("00D00000_{:08X}_OC.nif", mesh.mesh_id));
            p
        };
        let out = NifFile::load(&dest).expect("reload baked nif");
        let shape = out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSTriShape")
            .expect("shape block");
        assert_eq!(
            shape.get_field("Num Vertices").map(|v| v.as_i64()),
            Some(first_vertex_count as i64),
            "first mesh_extract_roots entry in order must win over a later one"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
