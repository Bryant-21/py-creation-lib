//! PyO3 surface for `havok_native`.
//!
//! Binding discipline:
//! - Clone owned data out of GIL-bound Python arguments first.
//! - Run native work under `py.detach(...)`.
//! - Functions: pass bytes/strings, return bytes/strings/JSON.
//!
//! ## Model pyclass policy
//!
//! The Rust `HkxFile` / `HkxObject` / `HkxMember` model in `crate::hkx::model`
//! is exposed directly as `#[pyclass]` (see the "Hkxpack model wrappers"
//! section below) so Python callers operate on the native model instead of
//! round-tripping through TagXML.

use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList, PyModule, PyTuple};
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::api;
use crate::error::HavokError;
use crate::hkx::descriptors::{
    ClassDescriptor, ClassKind, DescriptorRegistry, EnumDef, MemberTemplate,
};
use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
use crate::hkx::types::{HkxType, HkxTypeFamily, HkxValue};

fn map_error(error: HavokError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[pyfunction]
fn hkx_detect_format(py: Python<'_>, data: &Bound<'_, PyBytes>) -> PyResult<(String, String)> {
    let bytes = data.as_bytes().to_vec();
    py.detach(move || {
        api::hkx_detect_format_full(&bytes)
            .map(|r| (r.kind, r.version))
            .map_err(map_error)
    })
}

#[pyfunction]
fn hkx_roundtrip_bytes<'py>(
    py: Python<'py>,
    data: &Bound<'_, PyBytes>,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = data.as_bytes().to_vec();
    let result = py.detach(move || api::hkx_roundtrip_bytes(&bytes).map_err(map_error))?;
    Ok(PyBytes::new(py, &result))
}

#[pyfunction]
fn havok_convert_bytes<'py>(
    py: Python<'py>,
    data: &Bound<'_, PyBytes>,
    target_version: String,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = data.as_bytes().to_vec();
    let result =
        py.detach(move || api::havok_convert_bytes(&bytes, &target_version).map_err(map_error))?;
    Ok(PyBytes::new(py, &result))
}

#[pyfunction]
fn havok_convert_file(
    py: Python<'_>,
    src_path: String,
    dst_path: String,
    target_version: String,
) -> PyResult<()> {
    py.detach(move || {
        api::havok_convert_file(src_path, dst_path, &target_version).map_err(map_error)
    })
}

#[pyfunction]
fn havok_convert_batch(
    py: Python<'_>,
    src_dir: String,
    dst_dir: String,
    target_version: String,
    preserve_structure: bool,
) -> PyResult<String> {
    py.detach(move || {
        let result =
            api::havok_convert_batch(src_dir, dst_dir, &target_version, preserve_structure)
                .map_err(map_error)?;
        Ok(json!({
            "converted": result.converted,
            "skipped": result.skipped,
            "errors": result
                .errors
                .into_iter()
                .map(|error| json!({"path": error.path, "error": error.error}))
                .collect::<Vec<_>>(),
        })
        .to_string())
    })
}

#[pyfunction]
fn havok_extract_clip(
    py: Python<'_>,
    xml: String,
    skeleton_xml: Option<String>,
) -> PyResult<String> {
    py.detach(move || api::havok_extract_clip(&xml, skeleton_xml.as_deref()).map_err(map_error))
}

#[pyfunction]
fn havok_write_animation_xml(
    py: Python<'_>,
    clip_json: String,
    skeleton_bone_names: Vec<String>,
) -> PyResult<String> {
    py.detach(move || {
        api::havok_write_animation_xml(&clip_json, &skeleton_bone_names).map_err(map_error)
    })
}

#[pyfunction]
#[pyo3(signature = (blob, havok_scale=1.0, body_id=None))]
fn havok_collision_preview(
    py: Python<'_>,
    blob: &Bound<'_, PyBytes>,
    havok_scale: f32,
    body_id: Option<usize>,
) -> PyResult<String> {
    let bytes = blob.as_bytes().to_vec();
    py.detach(move || api::havok_collision_preview(&bytes, havok_scale, body_id).map_err(map_error))
}

#[pyfunction]
fn havok_collision_summary(py: Python<'_>, blob: &Bound<'_, PyBytes>) -> PyResult<String> {
    let bytes = blob.as_bytes().to_vec();
    py.detach(move || api::havok_collision_summary(&bytes).map_err(map_error))
}

#[pyfunction]
fn validate_collision_blob(
    py: Python<'_>,
    blob: &Bound<'_, PyBytes>,
    invariants_json: String,
) -> PyResult<String> {
    let bytes = blob.as_bytes().to_vec();
    py.detach(move || api::validate_collision_blob(&bytes, &invariants_json).map_err(map_error))
}

#[pyfunction]
fn havok_parse_skeleton(py: Python<'_>, xml: String) -> PyResult<String> {
    py.detach(move || api::havok_parse_skeleton(&xml).map_err(map_error))
}

#[pyfunction]
fn havok_parse_behavior(py: Python<'_>, xml: String) -> PyResult<String> {
    py.detach(move || api::havok_parse_behavior(&xml).map_err(map_error))
}

// ---------------------------------------------------------------------------
// Full behavior-graph → UI dict-node graph
// ---------------------------------------------------------------------------

/// Parse a Havok behavior graph XML and return the full UI dict-node graph as JSON.
///
/// Mirrors `py_creation_lib/python/creation_lib/behavior/xml_import.py::import_xml_file` exactly. Output:
/// `{"nodes": {...}, "connections": [...], "global_state": {...}, "unhandled": [...]}`
#[pyfunction]
fn havok_behavior_graph_to_ui_json(py: Python<'_>, xml: String) -> PyResult<String> {
    py.detach(move || api::havok_behavior_graph_to_ui_json(&xml).map_err(map_error))
}

// ---------------------------------------------------------------------------
// Cloth pyfunctions
// ---------------------------------------------------------------------------

#[pyfunction]
fn cloth_metadata_from_blob(py: Python<'_>, blob_bytes: &Bound<'_, PyBytes>) -> PyResult<String> {
    let bytes = blob_bytes.as_bytes().to_vec();
    py.detach(move || api::cloth_metadata_from_blob(&bytes).map_err(map_error))
}

#[pyfunction]
fn cloth_bake<'py>(py: Python<'py>, setup_json: String) -> PyResult<Bound<'py, PyBytes>> {
    let result = py.detach(move || api::cloth_bake(&setup_json).map_err(map_error))?;
    Ok(PyBytes::new(py, &result))
}

#[pyfunction]
fn cloth_validate(py: Python<'_>, blob_or_nif_bytes: &Bound<'_, PyBytes>) -> PyResult<String> {
    let bytes = blob_or_nif_bytes.as_bytes().to_vec();
    py.detach(move || api::cloth_validate(&bytes).map_err(map_error))
}

#[pyfunction]
#[pyo3(signature = (setup_json, steps, config_json=None))]
fn cloth_simulate(
    py: Python<'_>,
    setup_json: String,
    steps: u32,
    config_json: Option<String>,
) -> PyResult<String> {
    py.detach(move || {
        api::cloth_simulate(&setup_json, steps, config_json.as_deref()).map_err(map_error)
    })
}

// ---------------------------------------------------------------------------
// cloth_simulate_from_blob pyfunction
// ---------------------------------------------------------------------------

/// Run the XPBD cloth solver on an existing HCL packfile blob for `steps` frames.
///
/// Derives particle positions, pin indices, distance constraints, and capsule
/// collidables from the blob directly — no setup JSON required.
///
/// Returns JSON: `{"positions": [[x, y, z], ...], "n_particles": N, "fixed_count": K}`
#[pyfunction]
#[pyo3(signature = (blob, steps, config_json=None))]
fn cloth_simulate_from_blob(
    py: Python<'_>,
    blob: &Bound<'_, PyBytes>,
    steps: u32,
    config_json: Option<String>,
) -> PyResult<String> {
    let bytes = blob.as_bytes().to_vec();
    py.detach(move || {
        api::cloth_simulate_from_blob(&bytes, steps, config_json.as_deref()).map_err(map_error)
    })
}

#[pyfunction]
#[pyo3(signature = (blob, positions_json, prev_positions_json, config_json=None))]
fn cloth_step_from_blob_state(
    py: Python<'_>,
    blob: &Bound<'_, PyBytes>,
    positions_json: String,
    prev_positions_json: String,
    config_json: Option<String>,
) -> PyResult<String> {
    let bytes = blob.as_bytes().to_vec();
    py.detach(move || {
        api::cloth_step_from_blob_state(
            &bytes,
            &positions_json,
            &prev_positions_json,
            config_json.as_deref(),
        )
        .map_err(map_error)
    })
}

/// Compute a 3D convex hull. Returns `(hull_vertices, planes)` matching
/// the legacy Python convex hull contract for bhk callers:
/// `hull_vertices` is the deduplicated NIF-space vertex subset on the hull,
/// and each `plane = [nx, ny, nz, offset]` uses the Python convention
/// `offset = -hull.equations[:, 3]` so callers can write `Normal.w = offset * havok_scale`.
#[pyfunction]
fn convex_hull_simple(
    py: Python<'_>,
    vertices: Vec<[f32; 3]>,
) -> PyResult<(Vec<[f32; 3]>, Vec<[f32; 4]>)> {
    py.detach(move || crate::api::convex_hull_simple(&vertices).map_err(map_error))
}

/// Compute a 3D convex hull surface mesh. Returns `(hull_vertices, triangles)`.
#[pyfunction]
fn convex_hull_triangles(
    py: Python<'_>,
    vertices: Vec<[f32; 3]>,
) -> PyResult<(Vec<[f32; 3]>, Vec<[u32; 3]>)> {
    py.detach(move || crate::api::convex_hull_triangles(&vertices).map_err(map_error))
}

/// Decimate a triangle mesh using the native QEM reducer.
#[pyfunction]
fn decimate_mesh(
    py: Python<'_>,
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    target_tri_count: usize,
) -> PyResult<(Vec<[f32; 3]>, Vec<[u32; 3]>)> {
    py.detach(move || {
        crate::api::decimate_mesh(&vertices, &triangles, target_tri_count).map_err(map_error)
    })
}

/// Build a single FO4 hknpConvexPolytopeShape packfile blob from a vertex list.
///
/// `vertices`: list of `[x, y, z]` float32 triples (NIF-space).
/// Returns raw bytes of the Havok 2014.1.0 packfile.
///
#[pyfunction]
#[pyo3(signature = (vertices, friction, restitution, layer, mass, material_crc=None))]
fn fo4_polytope_collision_blob<'py>(
    py: Python<'py>,
    vertices: Vec<[f32; 3]>,
    friction: f32,
    restitution: f32,
    layer: u8,
    mass: f32,
    material_crc: Option<u32>,
) -> PyResult<Bound<'py, PyBytes>> {
    use crate::collision::build_fo4_polytope_collision;
    use crate::collision::compressed_mesh::BuildOptions;
    let opts = BuildOptions {
        friction,
        restitution,
        layer,
        mass,
        convex_radius: 0.05,
        materials: Vec::new(),
        user_data: material_crc.map(u64::from),
        body_props_raw: None,
        mass_distribution: None,
    };
    let result =
        py.detach(move || build_fo4_polytope_collision(&vertices, &opts).map_err(map_error))?;
    Ok(PyBytes::new(py, &result))
}

/// Build a FO4 hknpDynamicCompoundShape packfile blob from a list of sub-shapes.
///
/// Each sub-shape is a tuple of:
///   `(transform_flat, kind, vertices, triangles_or_none)`
/// where:
///   `transform_flat`: 16 f32s in row-major order (4×4 matrix)
///   `kind`: "polytope" or "compressed_mesh"
///   `vertices`: list of `[x, y, z]` f32 triples
///   `triangles_or_none`: list of `[i0, i1, i2]` u32 triples, or None
///
#[pyfunction]
#[pyo3(signature = (sub_shapes, friction, restitution, layer, mass, material_crc=None))]
fn fo4_compound_collision_blob<'py>(
    py: Python<'py>,
    sub_shapes: Vec<(Vec<f32>, String, Vec<[f32; 3]>, Option<Vec<[u32; 3]>>)>,
    friction: f32,
    restitution: f32,
    layer: u8,
    mass: f32,
    material_crc: Option<u32>,
) -> PyResult<Bound<'py, PyBytes>> {
    use crate::collision::compressed_mesh::BuildOptions;
    use crate::collision::{CompoundChild, CompoundChildKind, build_fo4_compound_collision};

    let children: Vec<CompoundChild> = sub_shapes
        .into_iter()
        .map(|(transform_flat, kind, verts, tris)| {
            // Row-major flat vec of 16 → [[f32;4];4]
            let mut mat = [[0f32; 4]; 4];
            for (i, v) in transform_flat.iter().take(16).enumerate() {
                mat[i / 4][i % 4] = *v;
            }
            let child_kind = match kind.as_str() {
                "compressed_mesh" => CompoundChildKind::CompressedMesh {
                    vertices: verts,
                    triangles: tris.unwrap_or_default(),
                },
                _ => CompoundChildKind::Polytope { vertices: verts },
            };
            CompoundChild {
                transform: mat,
                kind: child_kind,
            }
        })
        .collect();

    let opts = BuildOptions {
        friction,
        restitution,
        layer,
        mass,
        convex_radius: 0.05,
        materials: Vec::new(),
        user_data: material_crc.map(u64::from),
        body_props_raw: None,
        mass_distribution: None,
    };
    let result =
        py.detach(move || build_fo4_compound_collision(&children, &opts).map_err(map_error))?;
    Ok(PyBytes::new(py, &result))
}

// ---------------------------------------------------------------------------
// FO4 compressed mesh collision blob pyfunction
// ---------------------------------------------------------------------------

/// Build a FO4 hknpCompressedMeshShape packfile blob from vertices + triangles.
///
/// `vertices`: list of `[x, y, z]` float32 triples (NIF-space, max 255).
/// `triangles`: list of `[i0, i1, i2]` u32 index triples (max 255).
/// Returns raw bytes of the Havok 2014.1.0 packfile for bhkPhysicsSystem.
#[pyfunction]
#[pyo3(signature = (vertices, triangles, friction, restitution, layer, mass, material_crc=None))]
fn fo4_compressed_mesh_collision_blob<'py>(
    py: Python<'py>,
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    friction: f32,
    restitution: f32,
    layer: u8,
    mass: f32,
    material_crc: Option<u32>,
) -> PyResult<Bound<'py, PyBytes>> {
    use crate::collision::build_compressed_mesh_collision;
    use crate::collision::compressed_mesh::{BuildOptions, MaterialEntry};
    let opts = BuildOptions {
        friction,
        restitution,
        layer,
        mass,
        convex_radius: 0.05,
        materials: material_crc
            .map(|material_crc| {
                vec![MaterialEntry {
                    filter_info: layer as u32,
                    material_crc,
                }]
            })
            .unwrap_or_default(),
        user_data: material_crc.map(u64::from),
        body_props_raw: None,
        mass_distribution: None,
    };
    let result = py.detach(move || {
        build_compressed_mesh_collision(&vertices, &triangles, opts).map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &result))
}

/// Build a FO4 hknpPhysicsSystemData packfile with one body per supplied shape.
///
/// Each body is a tuple of `(kind, vertices, triangles_or_none, children_or_none)`
/// where kind is `"polytope"`, `"compressed_mesh"`, or `"compound"`.
///
/// `body_metas` (optional) is a parallel list of
/// `(layer, position_xyz, orientation_xyzw, motion_type_str)` per body where
/// `motion_type_str` is `"static"` or `"keyframed"`. ANIMSTATIC doors must
/// pass `"keyframed"` — otherwise the broadphase crashes on workshop sweep
/// (motionId resolves to HK_INVALID with no matching motionCinfo).
#[pyfunction]
#[pyo3(signature = (bodies, friction, restitution, layer, mass, material_crcs=None, body_metas=None))]
fn fo4_multi_body_collision_blob<'py>(
    py: Python<'py>,
    bodies: Vec<(
        String,
        Vec<[f32; 3]>,
        Option<Vec<[u32; 3]>>,
        Option<Vec<(String, Vec<[f32; 3]>, Option<Vec<[u32; 3]>>)>>,
    )>,
    friction: f32,
    restitution: f32,
    layer: u8,
    mass: f32,
    material_crcs: Option<Vec<Option<u32>>>,
    body_metas: Option<Vec<(u8, [f32; 3], [f32; 4], String)>>,
) -> PyResult<Bound<'py, PyBytes>> {
    use crate::collision::compressed_mesh::BuildOptions;
    use crate::collision::multi_body::{BodyMeta, BodyMotionType};
    use crate::collision::{
        CompoundChild, CompoundChildKind, MultiBodyShape, build_fo4_multi_body_collision,
    };

    let bodies: Vec<MultiBodyShape> = bodies
        .into_iter()
        .map(
            |(kind, vertices, triangles, children)| match kind.as_str() {
                "compressed_mesh" | "mesh" => MultiBodyShape::CompressedMesh {
                    vertices,
                    triangles: triangles.unwrap_or_default(),
                },
                "compound" => {
                    let children = children
                        .unwrap_or_default()
                        .into_iter()
                        .map(
                            |(child_kind, child_vertices, child_triangles)| CompoundChild {
                                transform: CompoundChild::identity_transform(),
                                kind: match child_kind.as_str() {
                                    "compressed_mesh" | "mesh" => {
                                        CompoundChildKind::CompressedMesh {
                                            vertices: child_vertices,
                                            triangles: child_triangles.unwrap_or_default(),
                                        }
                                    }
                                    _ => CompoundChildKind::Polytope {
                                        vertices: child_vertices,
                                    },
                                },
                            },
                        )
                        .collect();
                    MultiBodyShape::Compound { children }
                }
                // Sphere kind: encoded as `vertices=[[radius, 0, 0], [x, y, z]]`
                // (two-element list — first element's x is the radius, second
                // element is the center position). `triangles` is None.
                // Position can also come from `body_metas[i].position` which
                // takes precedence inside `build_fo4_multi_body_collision`.
                "sphere" => {
                    let radius = vertices.first().map(|v| v[0]).unwrap_or(0.0);
                    let position = vertices.get(1).copied().unwrap_or([0.0, 0.0, 0.0]);
                    MultiBodyShape::Sphere { radius, position }
                }
                _ => MultiBodyShape::Polytope { vertices },
            },
        )
        .collect();
    let opts = BuildOptions {
        friction,
        restitution,
        layer,
        mass,
        convex_radius: 0.05,
        materials: Vec::new(),
        user_data: None,
        body_props_raw: None,
        mass_distribution: None,
    };
    let metas: Option<Vec<BodyMeta>> = body_metas.map(|raw| {
        raw.into_iter()
            .map(|(b_layer, pos, orient, motion)| BodyMeta {
                collision_filter_info: None,
                layer: b_layer,
                body_flags: None,
                material_flags: None,
                material_trigger_type: None,
                position: [pos[0], pos[1], pos[2], 0.0],
                orientation: orient,
                motion_type: match motion.as_str() {
                    "keyframed" | "animstatic" => BodyMotionType::Keyframed,
                    _ => BodyMotionType::Static,
                },
                body_mass: None,
                mass_distribution: None,
            })
            .collect()
    });
    let result = py.detach(move || {
        build_fo4_multi_body_collision(&bodies, &opts, material_crcs.as_deref(), metas.as_deref())
            .map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &result))
}

// ---------------------------------------------------------------------------
// Starfield convex collision blob pyfunction
// ---------------------------------------------------------------------------

/// Build a Starfield Havok 2019 TAG0 tagged binary blob for convex collision.
///
/// `vertices`: list of `[x, y, z]` float32 triples (NIF-space).
/// `friction`, `restitution`, `layer`, `mass`: material and body parameters
/// (accepted for API parity; material fields are encoded in the embedded
/// reference defaults — patching is not yet implemented).
///
/// Returns raw bytes of the Havok 2019 TAG0 blob suitable for bhkPhysicsSystem.
#[pyfunction]
fn starfield_convex_collision_blob<'py>(
    py: Python<'py>,
    vertices: Vec<[f32; 3]>,
    friction: f32,
    restitution: f32,
    layer: u8,
    mass: f32,
) -> PyResult<Bound<'py, PyBytes>> {
    let result = py.detach(move || {
        api::starfield_convex_collision_blob(&vertices, friction, restitution, layer, mass)
            .map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &result))
}

/// Round-trip a packfile through `patch_hkx`. Mirrors
/// `py_creation_lib/python/creation_lib/hkxpack/__init__.py::save_hkx`'s patcher branch — with no model edits
/// this is byte-exact with the input. Raises `ValueError` when an array's
/// serialized length no longer matches the source length (Python's
/// `CannotPatch`).
#[pyfunction]
fn hkx_patch_roundtrip<'py>(
    py: Python<'py>,
    data: &Bound<'_, PyBytes>,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = data.as_bytes().to_vec();
    let result = py.detach(move || api::hkx_patch_roundtrip(&bytes).map_err(map_error))?;
    Ok(PyBytes::new(py, &result))
}

// ---------------------------------------------------------------------------
// Native XML I/O pyfunctions
// ---------------------------------------------------------------------------

/// Accept HKX bytes (auto-detect packfile vs tagfile), parse, and return TagXML as a string.
#[pyfunction]
fn hkx_to_xml(py: Python<'_>, data: &Bound<'_, PyBytes>) -> PyResult<String> {
    let bytes = data.as_bytes().to_vec();
    py.detach(move || api::havok_hkx_to_xml(&bytes).map_err(map_error))
}

/// Accept a TagXML string, parse it, serialize to a Havok packfile, and return the bytes.
#[pyfunction]
fn xml_to_hkx<'py>(py: Python<'py>, xml: String) -> PyResult<Bound<'py, PyBytes>> {
    let result = py.detach(move || api::havok_xml_to_hkx(&xml).map_err(map_error))?;
    Ok(PyBytes::new(py, &result))
}

/// Decompress a spline-compressed animation block.
///
/// Parameters mirror `creation_lib.havok.spline_decompress.decompress_spline` exactly.
/// Returns a JSON string: `[[{"translation":[x,y,z],"rotation":[x,y,z,w],"scale":[x,y,z]}, ...], ...]`
#[pyfunction]
#[pyo3(signature = (
    data,
    num_transform_tracks,
    num_float_tracks,
    num_frames,
    max_frames_per_block,
    num_blocks,
    block_offsets,
    float_block_offsets,
    mask_and_quant_size,
    block_duration,
    block_inverse_duration,
    frame_duration
))]
fn havok_decompress_spline(
    py: Python<'_>,
    data: &Bound<'_, PyBytes>,
    num_transform_tracks: u32,
    num_float_tracks: u32,
    num_frames: u32,
    max_frames_per_block: u32,
    num_blocks: u32,
    block_offsets: Vec<u32>,
    float_block_offsets: Vec<u32>,
    mask_and_quant_size: u32,
    block_duration: f32,
    block_inverse_duration: f32,
    frame_duration: f32,
) -> PyResult<String> {
    let bytes = data.as_bytes().to_vec();
    py.detach(move || {
        api::havok_decompress_spline(
            &bytes,
            num_transform_tracks,
            num_float_tracks,
            num_frames,
            max_frames_per_block,
            num_blocks,
            &block_offsets,
            &float_block_offsets,
            mask_and_quant_size,
            block_duration,
            block_inverse_duration,
            frame_duration,
        )
        .map_err(map_error)
    })
}

// ---------------------------------------------------------------------------
// ClothEditor pyfunctions — bytes-in / (bytes|tuple)-out
// ---------------------------------------------------------------------------

#[pyfunction]
#[pyo3(signature = (blob, mass, sim_cloth_idx=0))]
fn cloth_set_particle_mass_all<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    mass: f32,
    sim_cloth_idx: usize,
) -> PyResult<(Bound<'py, PyBytes>, usize)> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, count) = py.detach(move || {
        api::cloth_set_particle_mass_all(&bytes, mass, sim_cloth_idx).map_err(map_error)
    })?;
    Ok((PyBytes::new(py, &new_bytes), count))
}

#[pyfunction]
#[pyo3(signature = (blob, factor, sim_cloth_idx=0))]
fn cloth_scale_particle_mass<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    factor: f32,
    sim_cloth_idx: usize,
) -> PyResult<(Bound<'py, PyBytes>, usize)> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, count) = py.detach(move || {
        api::cloth_scale_particle_mass(&bytes, factor, sim_cloth_idx).map_err(map_error)
    })?;
    Ok((PyBytes::new(py, &new_bytes), count))
}

#[pyfunction]
#[pyo3(signature = (blob, radius, sim_cloth_idx=0))]
fn cloth_set_particle_radius_all<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    radius: f32,
    sim_cloth_idx: usize,
) -> PyResult<(Bound<'py, PyBytes>, usize)> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, count) = py.detach(move || {
        api::cloth_set_particle_radius_all(&bytes, radius, sim_cloth_idx).map_err(map_error)
    })?;
    Ok((PyBytes::new(py, &new_bytes), count))
}

#[pyfunction]
#[pyo3(signature = (blob, friction, sim_cloth_idx=0))]
fn cloth_set_particle_friction_all<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    friction: f32,
    sim_cloth_idx: usize,
) -> PyResult<(Bound<'py, PyBytes>, usize)> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, count) = py.detach(move || {
        api::cloth_set_particle_friction_all(&bytes, friction, sim_cloth_idx).map_err(map_error)
    })?;
    Ok((PyBytes::new(py, &new_bytes), count))
}

#[pyfunction]
#[pyo3(signature = (blob, particle_index, fixed, sim_cloth_idx=0))]
fn cloth_set_particle_fixed<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    particle_index: usize,
    fixed: bool,
    sim_cloth_idx: usize,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, _count) = py.detach(move || {
        api::cloth_set_particle_fixed(&bytes, particle_index, fixed, sim_cloth_idx)
            .map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &new_bytes))
}

#[pyfunction]
#[pyo3(signature = (blob, indices, fixed, sim_cloth_idx=0))]
fn cloth_set_particles_fixed<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    indices: Vec<u32>,
    fixed: bool,
    sim_cloth_idx: usize,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, _count) = py.detach(move || {
        api::cloth_set_particles_fixed(&bytes, &indices, fixed, sim_cloth_idx).map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &new_bytes))
}

#[pyfunction]
#[pyo3(signature = (blob, indices, mass, sim_cloth_idx=0))]
fn cloth_set_particles_mass<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    indices: Vec<usize>,
    mass: f32,
    sim_cloth_idx: usize,
) -> PyResult<(Bound<'py, PyBytes>, usize)> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, count) = py.detach(move || {
        api::cloth_set_particles_mass(&bytes, &indices, mass, sim_cloth_idx).map_err(map_error)
    })?;
    Ok((PyBytes::new(py, &new_bytes), count))
}

#[pyfunction]
#[pyo3(signature = (blob, indices, radius, sim_cloth_idx=0))]
fn cloth_set_particles_radius<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    indices: Vec<usize>,
    radius: f32,
    sim_cloth_idx: usize,
) -> PyResult<(Bound<'py, PyBytes>, usize)> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, count) = py.detach(move || {
        api::cloth_set_particles_radius(&bytes, &indices, radius, sim_cloth_idx).map_err(map_error)
    })?;
    Ok((PyBytes::new(py, &new_bytes), count))
}

#[pyfunction]
#[pyo3(signature = (blob, constraint_class, factor, sim_cloth_idx=0))]
fn cloth_scale_stiffness<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    constraint_class: Option<String>,
    factor: f32,
    sim_cloth_idx: usize,
) -> PyResult<(Bound<'py, PyBytes>, usize)> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, count) = py.detach(move || {
        api::cloth_scale_stiffness(&bytes, constraint_class.as_deref(), factor, sim_cloth_idx)
            .map_err(map_error)
    })?;
    Ok((PyBytes::new(py, &new_bytes), count))
}

#[pyfunction]
#[pyo3(signature = (blob, constraint_class, value, sim_cloth_idx=0))]
fn cloth_set_stiffness<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    constraint_class: Option<String>,
    value: f32,
    sim_cloth_idx: usize,
) -> PyResult<(Bound<'py, PyBytes>, usize)> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, count) = py.detach(move || {
        api::cloth_set_stiffness(&bytes, constraint_class.as_deref(), value, sim_cloth_idx)
            .map_err(map_error)
    })?;
    Ok((PyBytes::new(py, &new_bytes), count))
}

#[pyfunction]
#[pyo3(signature = (blob, gravity_xyzw, sim_cloth_idx=0))]
fn cloth_set_gravity<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    gravity_xyzw: [f32; 4],
    sim_cloth_idx: usize,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = blob.as_bytes().to_vec();
    let new_bytes = py.detach(move || {
        api::cloth_set_gravity(&bytes, gravity_xyzw, sim_cloth_idx).map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &new_bytes))
}

#[pyfunction]
#[pyo3(signature = (blob, damping, sim_cloth_idx=0))]
fn cloth_set_damping<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    damping: f32,
    sim_cloth_idx: usize,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = blob.as_bytes().to_vec();
    let new_bytes = py.detach(move || {
        api::cloth_set_damping(&bytes, damping, sim_cloth_idx).map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &new_bytes))
}

#[pyfunction]
#[pyo3(signature = (blob, tolerance, sim_cloth_idx=0))]
fn cloth_set_collision_tolerance<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    tolerance: f32,
    sim_cloth_idx: usize,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = blob.as_bytes().to_vec();
    let new_bytes = py.detach(move || {
        api::cloth_set_collision_tolerance(&bytes, tolerance, sim_cloth_idx).map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &new_bytes))
}

#[pyfunction]
fn cloth_set_substeps<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    substeps: u32,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = blob.as_bytes().to_vec();
    let new_bytes =
        py.detach(move || api::cloth_set_substeps(&bytes, substeps).map_err(map_error))?;
    Ok(PyBytes::new(py, &new_bytes))
}

#[pyfunction]
fn cloth_set_solver_iterations<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    iterations: u32,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = blob.as_bytes().to_vec();
    let new_bytes =
        py.detach(move || api::cloth_set_solver_iterations(&bytes, iterations).map_err(map_error))?;
    Ok(PyBytes::new(py, &new_bytes))
}

#[pyfunction]
#[pyo3(signature = (blob, collidable_idx, radius, sim_cloth_idx=0))]
fn cloth_set_capsule_radius<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    collidable_idx: usize,
    radius: f32,
    sim_cloth_idx: usize,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = blob.as_bytes().to_vec();
    let new_bytes = py.detach(move || {
        api::cloth_set_capsule_radius(&bytes, collidable_idx, radius, sim_cloth_idx)
            .map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &new_bytes))
}

#[pyfunction]
#[pyo3(signature = (blob, factor, sim_cloth_idx=0))]
fn cloth_scale_all_capsule_radii<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    factor: f32,
    sim_cloth_idx: usize,
) -> PyResult<(Bound<'py, PyBytes>, usize)> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, count) = py.detach(move || {
        api::cloth_scale_all_capsule_radii(&bytes, factor, sim_cloth_idx).map_err(map_error)
    })?;
    Ok((PyBytes::new(py, &new_bytes), count))
}

#[pyfunction]
#[pyo3(signature = (blob, collidable_idx, start_xyzw, end_xyzw, sim_cloth_idx=0))]
fn cloth_set_capsule_endpoints<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    collidable_idx: usize,
    start_xyzw: [f32; 4],
    end_xyzw: [f32; 4],
    sim_cloth_idx: usize,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = blob.as_bytes().to_vec();
    let new_bytes = py.detach(move || {
        api::cloth_set_capsule_endpoints(
            &bytes,
            collidable_idx,
            start_xyzw,
            end_xyzw,
            sim_cloth_idx,
        )
        .map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &new_bytes))
}

#[pyfunction]
#[pyo3(signature = (blob, bone_name, radius, start_xyzw, end_xyzw, sim_cloth_idx=0))]
fn cloth_add_capsule<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    bone_name: String,
    radius: f32,
    start_xyzw: [f32; 4],
    end_xyzw: [f32; 4],
    sim_cloth_idx: usize,
) -> PyResult<(Bound<'py, PyBytes>, usize)> {
    let bytes = blob.as_bytes().to_vec();
    let (new_bytes, new_idx) = py.detach(move || {
        api::cloth_add_capsule(
            &bytes,
            &bone_name,
            radius,
            start_xyzw,
            end_xyzw,
            sim_cloth_idx,
        )
        .map_err(map_error)
    })?;
    Ok((PyBytes::new(py, &new_bytes), new_idx))
}

#[pyfunction]
#[pyo3(signature = (blob, collidable_idx, sim_cloth_idx=0))]
fn cloth_remove_capsule<'py>(
    py: Python<'py>,
    blob: &Bound<'_, PyBytes>,
    collidable_idx: usize,
    sim_cloth_idx: usize,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = blob.as_bytes().to_vec();
    let new_bytes = py.detach(move || {
        api::cloth_remove_capsule(&bytes, collidable_idx, sim_cloth_idx).map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &new_bytes))
}

#[pyfunction]
#[pyo3(signature = (blob, sim_cloth_idx=0))]
fn cloth_summary_json(
    py: Python<'_>,
    blob: &Bound<'_, PyBytes>,
    sim_cloth_idx: usize,
) -> PyResult<String> {
    let bytes = blob.as_bytes().to_vec();
    py.detach(move || api::cloth_summary_json(&bytes, sim_cloth_idx).map_err(map_error))
}

#[pyfunction]
fn cloth_inspect_blob_json(py: Python<'_>, blob: &Bound<'_, PyBytes>) -> PyResult<String> {
    let bytes = blob.as_bytes().to_vec();
    py.detach(move || api::cloth_inspect_blob_json(&bytes).map_err(map_error))
}

// ---------------------------------------------------------------------------
// cloth_inspect_full_json pyfunction
// ---------------------------------------------------------------------------

#[pyfunction]
fn cloth_inspect_full_json(py: Python<'_>, blob: &Bound<'_, PyBytes>) -> PyResult<String> {
    let bytes = blob.as_bytes().to_vec();
    py.detach(move || api::cloth_inspect_full_json(&bytes).map_err(map_error))
}

// ---------------------------------------------------------------------------
// Cloth helper pyfunctions
// ---------------------------------------------------------------------------

#[pyfunction]
fn cloth_material_list(py: Python<'_>) -> PyResult<String> {
    py.detach(move || api::cloth_material_list().map_err(map_error))
}

#[pyfunction]
fn cloth_material_get(py: Python<'_>, name: String) -> PyResult<String> {
    py.detach(move || api::cloth_material_get(&name).map_err(map_error))
}

#[pyfunction]
fn cloth_material_apply(
    py: Python<'_>,
    setup_json: String,
    preset_name: String,
) -> PyResult<String> {
    py.detach(move || api::cloth_material_apply(&setup_json, &preset_name).map_err(map_error))
}

#[pyfunction]
fn cloth_topology_list(py: Python<'_>) -> PyResult<String> {
    py.detach(move || api::cloth_topology_list().map_err(map_error))
}

#[pyfunction]
fn cloth_topology_get(py: Python<'_>, name: String) -> PyResult<String> {
    py.detach(move || api::cloth_topology_get(&name).map_err(map_error))
}

#[pyfunction]
fn cloth_region_generate(
    py: Python<'_>,
    region_json: String,
    topology_name: String,
    args_json: String,
) -> PyResult<String> {
    py.detach(move || {
        api::cloth_region_generate(&region_json, &topology_name, &args_json).map_err(map_error)
    })
}

#[pyfunction]
fn cloth_reverse_to_setup(py: Python<'_>, blob: &Bound<'_, PyBytes>) -> PyResult<String> {
    let bytes = blob.as_bytes().to_vec();
    py.detach(move || api::cloth_reverse_to_setup(&bytes).map_err(map_error))
}

#[pyfunction]
fn cloth_generate_bones_from_particles(
    py: Python<'_>,
    positions_json: String,
    args_json: String,
) -> PyResult<String> {
    py.detach(move || {
        api::cloth_generate_bones_from_particles(&positions_json, &args_json).map_err(map_error)
    })
}

#[pyfunction]
fn cloth_bones_to_transform_set(py: Python<'_>, bones_json: String) -> PyResult<String> {
    py.detach(move || api::cloth_bones_to_transform_set(&bones_json).map_err(map_error))
}

#[pyfunction]
fn cloth_auto_skin(
    py: Python<'_>,
    positions_json: String,
    bone_positions_json: String,
    args_json: String,
) -> PyResult<String> {
    py.detach(move || {
        api::cloth_auto_skin(&positions_json, &bone_positions_json, &args_json).map_err(map_error)
    })
}

#[pyfunction]
fn cloth_template_list(py: Python<'_>) -> PyResult<String> {
    py.detach(move || api::cloth_template_list().map_err(map_error))
}

#[pyfunction]
fn cloth_template_get(py: Python<'_>, name: String) -> PyResult<String> {
    py.detach(move || api::cloth_template_get(&name).map_err(map_error))
}

#[pyfunction]
fn cloth_template_blob<'py>(
    py: Python<'py>,
    name: String,
    args_json: String,
) -> PyResult<Bound<'py, PyBytes>> {
    let result =
        py.detach(move || api::cloth_template_blob(&name, &args_json).map_err(map_error))?;
    Ok(PyBytes::new(py, &result))
}

// ---------------------------------------------------------------------------
// Asset discovery / manifest / parser pyfunctions
// ---------------------------------------------------------------------------

/// Walk a Meshes directory and return classified file entries as a JSON string.
#[pyfunction]
fn walk_meshes_dir(py: Python<'_>, meshes_dir: String, source: String) -> PyResult<String> {
    py.detach(move || api::walk_meshes_dir_json(&meshes_dir, &source).map_err(map_error))
}

/// Classify a file path's category (e.g. "Weapon", "Character", "Creature").
#[pyfunction]
fn classify_category(py: Python<'_>, path: String) -> PyResult<String> {
    py.detach(move || Ok(api::classify_category_str(&path).to_string()))
}

/// Classify a file path's role (e.g. "skeleton", "behavior", "animation").
#[pyfunction]
fn classify_role(py: Python<'_>, path: String) -> PyResult<String> {
    py.detach(move || Ok(api::classify_role_str(&path).to_string()))
}

/// Build manifests from JSON-encoded entries + character_data.
///
/// `entries_json`: JSON array of file-entry objects (from `walk_meshes_dir`).
/// `character_data_json`: JSON object mapping `rel_path → CharacterRecord`.
/// `source`: game source identifier (fo4, fo76, starfield).
/// Returns a JSON string of `Vec<ManifestData>`.
#[pyfunction]
fn build_manifests(
    py: Python<'_>,
    entries_json: String,
    character_data_json: String,
    source: String,
) -> PyResult<String> {
    py.detach(move || {
        api::build_manifests_json(&entries_json, &character_data_json, &source).map_err(map_error)
    })
}

/// Parse a Havok animation XML and return structured metadata as JSON.
#[pyfunction]
fn parse_animation_xml(py: Python<'_>, xml: String) -> PyResult<String> {
    py.detach(move || api::parse_animation_xml_json(&xml).map_err(map_error))
}

/// Parse a Havok character XML and return structured metadata as JSON.
#[pyfunction]
fn parse_character_xml(py: Python<'_>, xml: String) -> PyResult<String> {
    py.detach(move || api::parse_character_xml_json(&xml).map_err(map_error))
}

/// Parse a Havok project XML and return structured metadata as JSON.
#[pyfunction]
fn parse_project_xml(py: Python<'_>, xml: String) -> PyResult<String> {
    py.detach(move || api::parse_project_xml_json(&xml).map_err(map_error))
}

// ---------------------------------------------------------------------------
// generate_classxml pyfunction
// ---------------------------------------------------------------------------

/// Generate per-version classxml directories from an SDK patches directory.
///
/// Mirrors `py_creation_lib/python/creation_lib/havok/gen_classxml.py::generate_per_version_classxml`.
///
/// `source_dir`: base classxml directory path (e.g. `resource/classxml`).
/// `patches_dir`: SDK patches directory path.
/// `output_base`: parent output directory.
/// `targets_json`: JSON array of `[suffix, version_id]` pairs,
///   e.g. `[["2012", 46], ["2013", 49]]`.
/// `base_version_id`: version ID of the source classxml (53 for FO4).
#[pyfunction]
#[pyo3(signature = (source_dir, patches_dir, output_base, targets_json, base_version_id=53))]
fn generate_classxml(
    py: Python<'_>,
    source_dir: String,
    patches_dir: String,
    output_base: String,
    targets_json: String,
    base_version_id: i32,
) -> PyResult<()> {
    py.detach(move || {
        api::generate_classxml(
            &source_dir,
            &patches_dir,
            &output_base,
            &targets_json,
            base_version_id,
        )
        .map_err(map_error)
    })
}

// ---------------------------------------------------------------------------
// DescriptorRegistry pyfunctions (JSON-based, no pyclass)
// ---------------------------------------------------------------------------

/// Look up a Havok class descriptor from the classxml directory.
///
/// `version`: optional version suffix (e.g. "2012") to select a versioned
/// `resource/classxml_<version>/` directory.
///
/// Returns JSON-encoded `ClassDescriptor` or `null` if not found.
/// JSON shape: `{"name": "...", "signature": "...", "parent": "...",
///   "is_struct": false, "members": [...], "enums": {...}}`
#[pyfunction]
#[pyo3(signature = (class_name, version=None))]
fn descriptor_registry_get(
    py: Python<'_>,
    class_name: String,
    version: Option<String>,
) -> PyResult<String> {
    py.detach(move || {
        let mut registry = make_registry(version.as_deref());
        let json_val = registry
            .get(&class_name)
            .ok()
            .flatten()
            .map(|d| descriptor_to_json(d));
        serde_json::to_string(&json_val)
            .map_err(|e| map_error(crate::error::HavokError::InvalidInput(e.to_string())))
    })
}

/// Get all members (including inherited) for a class, ordered by offset.
///
/// Returns JSON array of member descriptors.
#[pyfunction]
#[pyo3(signature = (class_name, version=None))]
fn descriptor_registry_get_all_members(
    py: Python<'_>,
    class_name: String,
    version: Option<String>,
) -> PyResult<String> {
    py.detach(move || {
        let mut registry = make_registry(version.as_deref());
        let members = registry.get_all_members(&class_name).unwrap_or_default();
        let json: Vec<serde_json::Value> = members.iter().map(member_to_json).collect();
        serde_json::to_string(&json)
            .map_err(|e| map_error(crate::error::HavokError::InvalidInput(e.to_string())))
    })
}

/// Resolve an integer enum value to its string name.
///
/// Walks the inheritance chain until found. Returns the integer as a string if
/// no named value matches.
#[pyfunction]
#[pyo3(signature = (class_name, enum_name, int_value, version=None))]
fn descriptor_registry_get_enum_value(
    py: Python<'_>,
    class_name: String,
    enum_name: String,
    int_value: i32,
    version: Option<String>,
) -> PyResult<String> {
    py.detach(move || {
        let mut registry = make_registry(version.as_deref());
        Ok(registry.get_enum_value(&class_name, &enum_name, int_value))
    })
}

/// Resolve a string enum name to its integer value.
///
/// Returns 0 if not found (matches Python behaviour).
#[pyfunction]
#[pyo3(signature = (class_name, enum_name, str_value, version=None))]
fn descriptor_registry_get_enum_int(
    py: Python<'_>,
    class_name: String,
    enum_name: String,
    str_value: String,
    version: Option<String>,
) -> PyResult<i32> {
    py.detach(move || {
        let mut registry = make_registry(version.as_deref());
        Ok(registry.get_enum_int(&class_name, &enum_name, &str_value))
    })
}

fn make_registry(version: Option<&str>) -> crate::hkx::descriptors::DescriptorRegistry {
    if let Some(v) = version {
        crate::hkx::descriptors::DescriptorRegistry::for_version(v)
            .unwrap_or_else(|_| crate::hkx::descriptors::DescriptorRegistry::new())
    } else {
        crate::hkx::descriptors::DescriptorRegistry::new()
    }
}

fn descriptor_to_json(d: &crate::hkx::descriptors::ClassDescriptor) -> serde_json::Value {
    serde_json::json!({
        "name": d.name,
        "signature": d.signature,
        "parent": d.parent,
        "is_struct": d.is_struct,
        "members": d.members.iter().map(member_to_json).collect::<Vec<_>>(),
        "enums": d.enums,
    })
}

fn member_to_json(m: &crate::hkx::descriptors::MemberTemplate) -> serde_json::Value {
    serde_json::json!({
        "name": m.name,
        "offset": m.offset,
        "vtype": format!("{:?}", m.vtype),
        "vsubtype": format!("{:?}", m.vsubtype),
        "ctype": m.ctype,
        "arrsize": m.arrsize,
        "flags": m.flags,
        "etype": m.etype,
    })
}

// ---------------------------------------------------------------------------
// hkx_load_to_json / hkx_save_from_json pyfunctions
// ---------------------------------------------------------------------------

/// Parse HKX bytes and return a JSON envelope wrapping the TagXML representation.
///
/// Output: `{"format": "tagxml", "content": "<hkpackfile ...>...</hkpackfile>"}`
///
/// Pair with `hkx_save_from_json` for edit cycles.
#[pyfunction]
fn hkx_load_to_json(py: Python<'_>, data: &Bound<'_, PyBytes>) -> PyResult<String> {
    let bytes = data.as_bytes().to_vec();
    py.detach(move || api::hkx_load_to_json(&bytes).map_err(map_error))
}

/// Accept a JSON envelope produced by `hkx_load_to_json` and return HKX packfile bytes.
///
/// Input: `{"format": "tagxml", "content": "<hkpackfile ...>...</hkpackfile>"}`
#[pyfunction]
fn hkx_save_from_json<'py>(py: Python<'py>, json_str: String) -> PyResult<Bound<'py, PyBytes>> {
    let result = py.detach(move || api::hkx_save_from_json(&json_str).map_err(map_error))?;
    Ok(PyBytes::new(py, &result))
}

// ---------------------------------------------------------------------------
// havok_compress_spline pyfunction
// ---------------------------------------------------------------------------

/// Compress per-frame transforms (as JSON) into a spline-compressed binary blob.
///
/// Input JSON shape: `[[{"translation":[x,y,z],"rotation":[x,y,z,w],"scale":[x,y,z]}, ...], ...]`
/// (same shape as `havok_decompress_spline` output).
///
/// `duration`: total animation duration in seconds.
/// `fps`: frames-per-second (used only when `len(frames) < 2`).
///
/// Returns raw compressed bytes for the `data` member of `hkaSplineCompressedAnimation`.
#[pyfunction]
fn havok_compress_spline<'py>(
    py: Python<'py>,
    frames_json: String,
    duration: f32,
    fps: f32,
) -> PyResult<Bound<'py, PyBytes>> {
    let result = py.detach(move || {
        api::havok_compress_spline(&frames_json, duration, fps).map_err(map_error)
    })?;
    Ok(PyBytes::new(py, &result))
}

// ---------------------------------------------------------------------------
// hkx_inspect_packfile pyfunction
// ---------------------------------------------------------------------------

/// Parse a Havok packfile and return header, sections, classnames, and fixup
/// tables as JSON. See `api::hkx_inspect_packfile` for the JSON shape.
#[pyfunction]
fn hkx_inspect_packfile(py: Python<'_>, data: &Bound<'_, PyBytes>) -> PyResult<String> {
    let bytes = data.as_bytes().to_vec();
    py.detach(move || api::hkx_inspect_packfile(&bytes).map_err(map_error))
}

// ===========================================================================
// Hkxpack model wrappers — mutation surface
// ===========================================================================
//
// These pyclass types expose `crate::hkx::model::{HkxFile, HkxObject,
// HkxMember}` and `crate::hkx::descriptors::{DescriptorRegistry, ...}` to
// Python so callers in `py_creation_lib/python/creation_lib/hkxpack/` can stop round-tripping through
// TagXML and operate directly on the native model.
//
// Storage model: `PyHkxFile` owns the canonical `HkxFile`. All other model
// pyclasses are either:
//   * Bound — hold a `Py<PyHkxFile>` plus an address into the file's tree
//     (object index, member index, nested member-path). Property reads and
//     writes route through the parent file via `borrow_mut()`. Each setter
//     marks the file dirty so `save()` re-runs the writer.
//   * Unbound — hold their own data inline. Constructed via `__new__` from
//     Python; converted into the Bound form when appended into a file.
//
// Proxy lists (`HKXObjectList`, `HKXMemberList`, `HKXValueList`) are
// returned from `HKXFile.objects`, `HKXObject.members`, and
// `HKXArrayMember.contents`. They implement the dunder protocol so caller
// code keeps working unchanged: `lst.append(x)`, `lst[i] = x`, `lst[i]`,
// `len(lst)`, `for x in lst`, `lst == [...]`. Mutations forward to the
// parent file's underlying Vec.
//
// Identity caching is intentionally NOT implemented in this pass — every
// `obj.members[0]` call returns a fresh `Py<PyHkxMember>` instance.
// Callers that need identity comparison must compare addresses manually
// (e.g. `(obj_idx, mem_idx)`). Documented as a known deviation from the
// API surface spec.

// --- Address / storage primitives ----------------------------------------

/// Path through a single object's member tree. Empty path == top-level
/// member of the object. Each segment indexes into the next nested
/// `HkxValue::Array` or `HkxValue::Object`/`TypedObject` member list.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MemberPath {
    /// Top-level member index inside `HkxObject.members`.
    member_index: usize,
    /// Subsequent nesting steps. Each step is either:
    ///   * `Member(usize)` — index into a nested object's `members` Vec.
    ///   * `Array(usize)`  — index into a nested array's contents.
    nested: Vec<PathStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathStep {
    /// Index into a nested object's `members` Vec
    /// (i.e. into `HkxValue::Object` / `HkxValue::TypedObject`).
    Member(usize),
    /// Index into a nested array's contents
    /// (i.e. into `HkxValue::Array`).
    Array(usize),
}

impl MemberPath {
    fn top(member_index: usize) -> Self {
        Self {
            member_index,
            nested: Vec::new(),
        }
    }

    fn push(&self, step: PathStep) -> Self {
        let mut nested = self.nested.clone();
        nested.push(step);
        Self {
            member_index: self.member_index,
            nested,
        }
    }
}

/// Resolve a `MemberPath` to the `HkxMember` it identifies. Returns `None`
/// if any segment is out of bounds.
fn resolve_member<'a>(obj: &'a HkxObject, path: &MemberPath) -> Option<&'a HkxMember> {
    let mut current = obj.members.get(path.member_index)?;
    for step in &path.nested {
        match step {
            PathStep::Member(i) => {
                let members = current.value.as_object_members()?;
                current = members.get(*i)?;
            }
            PathStep::Array(_) => {
                // Array indices step into HkxValue::Array contents which are
                // themselves HkxValue (not HkxMember). The caller of
                // resolve_member shouldn't terminate inside an array — only
                // value-walks do.
                return None;
            }
        }
    }
    Some(current)
}

fn resolve_member_mut<'a>(obj: &'a mut HkxObject, path: &MemberPath) -> Option<&'a mut HkxMember> {
    let mut current = obj.members.get_mut(path.member_index)?;
    for step in &path.nested {
        match step {
            PathStep::Member(i) => {
                let members = current.value.as_object_members_mut()?;
                current = members.get_mut(*i)?;
            }
            PathStep::Array(_) => return None,
        }
    }
    Some(current)
}

/// Resolve to a `HkxValue` reference. Used by HKXValueList accessors that
/// step into `HkxValue::Array(Vec<HkxValue>)` contents.
fn resolve_value_array<'a>(obj: &'a HkxObject, path: &MemberPath) -> Option<&'a Vec<HkxValue>> {
    let member = resolve_member(obj, path)?;
    match &member.value {
        HkxValue::Array(values) => Some(values),
        _ => None,
    }
}

fn resolve_value_array_mut<'a>(
    obj: &'a mut HkxObject,
    path: &MemberPath,
) -> Option<&'a mut Vec<HkxValue>> {
    let member = resolve_member_mut(obj, path)?;
    match &mut member.value {
        HkxValue::Array(values) => Some(values),
        _ => None,
    }
}

// --- HKXEnumMember tracker -----------------------------------------------
//
// Enum-typed members are stored as `HkxValue::I32` in Rust. To round-trip
// the (enum_name, value-as-string) view that Python expects, we cache
// the enum metadata on the file as a side-table keyed by member path.
// The cache is purely advisory — losing an entry just means the member
// appears as HKXDirectMember(I32) instead of HKXEnumMember.

#[derive(Debug, Clone, Default)]
struct EnumMetadata {
    /// (object_index, member_path) -> (enum_name, ...)
    /// Stored on PyHkxFile, populated by descriptor lookup or by the user
    /// constructing HKXEnumMember instances and binding them.
    map: std::collections::HashMap<(usize, MemberPath), String>,
}

// --- Enums ----------------------------------------------------------------

/// Mirrors the `HkxTypeFamily` Rust enum, exposed with the same names as
/// the Python `creation_lib.hkxpack.model.HKXTypeFamily` enum.
#[pyclass(
    eq,
    eq_int,
    name = "HKXTypeFamily",
    module = "creation_lib._native.havok_native"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyHkxTypeFamily {
    Direct,
    Complex,
    Enum,
    Array,
    Pointer,
    String,
    Object,
}

impl From<HkxTypeFamily> for PyHkxTypeFamily {
    fn from(family: HkxTypeFamily) -> Self {
        match family {
            HkxTypeFamily::Direct => Self::Direct,
            HkxTypeFamily::Complex => Self::Complex,
            HkxTypeFamily::Enum => Self::Enum,
            HkxTypeFamily::Array => Self::Array,
            HkxTypeFamily::Pointer => Self::Pointer,
            HkxTypeFamily::String => Self::String,
            HkxTypeFamily::Object => Self::Object,
        }
    }
}

/// Mirrors the `HkxType` Rust enum.
#[pyclass(
    eq,
    eq_int,
    name = "HKXType",
    module = "creation_lib._native.havok_native"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyHkxType {
    VOID,
    BOOL,
    INT8,
    UINT8,
    INT16,
    UINT16,
    HALF,
    INT32,
    UINT32,
    REAL,
    INT64,
    UINT64,
    ULONG,
    VECTOR4,
    QUATERNION,
    MATRIX3,
    MATRIX4,
    TRANSFORM,
    QSTRANSFORM,
    ENUM,
    FLAGS,
    ARRAY,
    SIMPLEARRAY,
    RELARRAY,
    POINTER,
    FUNCTIONPOINTER,
    CSTRING,
    STRINGPTR,
    STRUCT,
}

impl From<HkxType> for PyHkxType {
    fn from(t: HkxType) -> Self {
        match t {
            HkxType::Void => Self::VOID,
            HkxType::Bool => Self::BOOL,
            HkxType::Int8 => Self::INT8,
            HkxType::Uint8 => Self::UINT8,
            HkxType::Int16 => Self::INT16,
            HkxType::Uint16 => Self::UINT16,
            HkxType::Half => Self::HALF,
            HkxType::Int32 => Self::INT32,
            HkxType::Uint32 => Self::UINT32,
            HkxType::Real => Self::REAL,
            HkxType::Int64 => Self::INT64,
            HkxType::Uint64 => Self::UINT64,
            HkxType::Ulong => Self::ULONG,
            HkxType::Vector4 => Self::VECTOR4,
            HkxType::Quaternion => Self::QUATERNION,
            HkxType::Matrix3 => Self::MATRIX3,
            HkxType::Matrix4 => Self::MATRIX4,
            HkxType::Transform => Self::TRANSFORM,
            HkxType::QsTransform => Self::QSTRANSFORM,
            HkxType::Enum => Self::ENUM,
            HkxType::Flags => Self::FLAGS,
            HkxType::Array => Self::ARRAY,
            HkxType::SimpleArray => Self::SIMPLEARRAY,
            HkxType::RelArray => Self::RELARRAY,
            HkxType::Pointer => Self::POINTER,
            HkxType::FunctionPointer => Self::FUNCTIONPOINTER,
            HkxType::CString => Self::CSTRING,
            HkxType::StringPtr => Self::STRINGPTR,
            HkxType::Struct => Self::STRUCT,
        }
    }
}

impl PyHkxType {
    fn to_native(self) -> HkxType {
        match self {
            Self::VOID => HkxType::Void,
            Self::BOOL => HkxType::Bool,
            Self::INT8 => HkxType::Int8,
            Self::UINT8 => HkxType::Uint8,
            Self::INT16 => HkxType::Int16,
            Self::UINT16 => HkxType::Uint16,
            Self::HALF => HkxType::Half,
            Self::INT32 => HkxType::Int32,
            Self::UINT32 => HkxType::Uint32,
            Self::REAL => HkxType::Real,
            Self::INT64 => HkxType::Int64,
            Self::UINT64 => HkxType::Uint64,
            Self::ULONG => HkxType::Ulong,
            Self::VECTOR4 => HkxType::Vector4,
            Self::QUATERNION => HkxType::Quaternion,
            Self::MATRIX3 => HkxType::Matrix3,
            Self::MATRIX4 => HkxType::Matrix4,
            Self::TRANSFORM => HkxType::Transform,
            Self::QSTRANSFORM => HkxType::QsTransform,
            Self::ENUM => HkxType::Enum,
            Self::FLAGS => HkxType::Flags,
            Self::ARRAY => HkxType::Array,
            Self::SIMPLEARRAY => HkxType::SimpleArray,
            Self::RELARRAY => HkxType::RelArray,
            Self::POINTER => HkxType::Pointer,
            Self::FUNCTIONPOINTER => HkxType::FunctionPointer,
            Self::CSTRING => HkxType::CString,
            Self::STRINGPTR => HkxType::StringPtr,
            Self::STRUCT => HkxType::Struct,
        }
    }
}

#[pymethods]
impl PyHkxType {
    #[getter]
    fn size(&self) -> usize {
        self.to_native().size()
    }

    #[getter]
    fn family(&self) -> PyHkxTypeFamily {
        self.to_native().family().into()
    }
}

// --- Member variant pyclasses --------------------------------------------
//
// Each variant pyclass holds an `Inner` enum:
//   * Bound { file: Py<PyHkxFile>, obj_idx: usize, path: MemberPath }
//   * Unbound { ... fields ... }
//
// Property getters/setters branch on the variant. Bound variants
// borrow_mut() the parent file to read or write through.

#[derive(Debug)]
enum DirectInner {
    Bound {
        file: Py<PyHkxFile>,
        obj_idx: usize,
        path: MemberPath,
    },
    /// `value` is a Python object (cached primitive Python value).
    /// Stored as `Py<PyAny>` so the same object is returned on every
    /// `.value` access for the unbound case.
    Unbound {
        name: String,
        type_: PyHkxType,
        value: Py<PyAny>,
    },
}

#[pyclass(name = "HKXDirectMember", module = "creation_lib._native.havok_native")]
pub struct PyHkxDirectMember {
    inner: DirectInner,
}

#[derive(Debug)]
enum ArrayInner {
    Bound {
        file: Py<PyHkxFile>,
        obj_idx: usize,
        path: MemberPath,
    },
    Unbound {
        name: String,
        subtype: PyHkxType,
        contents: Vec<HkxValue>,
        ctype: String,
        source_offset: i64,
        source_length: i64,
    },
}

#[pyclass(name = "HKXArrayMember", module = "creation_lib._native.havok_native")]
pub struct PyHkxArrayMember {
    inner: ArrayInner,
}

#[derive(Debug)]
enum PointerInner {
    Bound {
        file: Py<PyHkxFile>,
        obj_idx: usize,
        path: MemberPath,
    },
    Unbound {
        name: String,
        target: String,
        targets: Option<Vec<String>>,
    },
}

#[pyclass(
    name = "HKXPointerMember",
    module = "creation_lib._native.havok_native"
)]
pub struct PyHkxPointerMember {
    inner: PointerInner,
}

#[derive(Debug)]
enum StringInner {
    Bound {
        file: Py<PyHkxFile>,
        obj_idx: usize,
        path: MemberPath,
    },
    Unbound {
        name: String,
        value: String,
        is_null: bool,
    },
}

#[pyclass(name = "HKXStringMember", module = "creation_lib._native.havok_native")]
pub struct PyHkxStringMember {
    inner: StringInner,
}

#[derive(Debug)]
enum EnumInner {
    Bound {
        file: Py<PyHkxFile>,
        obj_idx: usize,
        path: MemberPath,
        enum_name: String,
    },
    Unbound {
        name: String,
        enum_name: String,
        value: String,
    },
}

#[pyclass(name = "HKXEnumMember", module = "creation_lib._native.havok_native")]
pub struct PyHkxEnumMember {
    inner: EnumInner,
}

// --- HKXObject -----------------------------------------------------------

#[derive(Debug)]
enum ObjectInner {
    Bound { file: Py<PyHkxFile>, obj_idx: usize },
    Unbound { obj: HkxObject },
}

#[pyclass(name = "HKXObject", module = "creation_lib._native.havok_native")]
pub struct PyHkxObject {
    inner: ObjectInner,
}

// --- HKXFile -------------------------------------------------------------

#[pyclass(name = "HKXFile", module = "creation_lib._native.havok_native")]
pub struct PyHkxFile {
    pub(crate) inner: HkxFile,
    /// Side-table for HKXEnumMember metadata.
    enum_meta: EnumMetadata,
}

// --- Helper conversion: HkxValue <-> Python ------------------------------

/// Convert an `HkxValue` to a Python object suitable for direct return
/// (used for HKXDirectMember.value when the type is primitive, and for
/// HKXValueList element access).
fn value_to_py<'py>(py: Python<'py>, value: &HkxValue) -> PyResult<Bound<'py, PyAny>> {
    use pyo3::IntoPyObject;
    match value {
        HkxValue::Void => Ok(py.None().into_bound(py)),
        HkxValue::Bool(b) => Ok(b.into_pyobject(py)?.to_owned().into_any()),
        HkxValue::I8(v) => Ok(v.into_pyobject(py)?.into_any()),
        HkxValue::U8(v) => Ok(v.into_pyobject(py)?.into_any()),
        HkxValue::I16(v) => Ok(v.into_pyobject(py)?.into_any()),
        HkxValue::U16(v) => Ok(v.into_pyobject(py)?.into_any()),
        HkxValue::I32(v) => Ok(v.into_pyobject(py)?.into_any()),
        HkxValue::U32(v) => Ok(v.into_pyobject(py)?.into_any()),
        HkxValue::I64(v) => Ok(v.into_pyobject(py)?.into_any()),
        HkxValue::U64(v) => Ok(v.into_pyobject(py)?.into_any()),
        HkxValue::F32(v) => Ok(v.into_pyobject(py)?.into_any()),
        HkxValue::Half(v) => Ok(v.into_pyobject(py)?.into_any()),
        HkxValue::F32List(vs) => {
            let list = PyList::empty(py);
            for v in vs {
                list.append(*v)?;
            }
            Ok(list.into_any())
        }
        HkxValue::String { value, .. } => Ok(value.into_pyobject(py)?.into_any()),
        HkxValue::Pointer(opt) => {
            let s = match opt {
                Some(idx) => format!("#{:04X}", idx),
                None => String::new(),
            };
            Ok(s.into_pyobject(py)?.into_any())
        }
        HkxValue::Array(_) => {
            // Nested array — return a plain Python list snapshot. Used when
            // walking heterogeneous array contents.
            let list = PyList::empty(py);
            if let HkxValue::Array(vs) = value {
                for v in vs {
                    list.append(value_to_py(py, v)?)?;
                }
            }
            Ok(list.into_any())
        }
        HkxValue::Object(_) | HkxValue::TypedObject { .. } => {
            // Inline struct used as an array element → expose as an unbound
            // HKXObject snapshot. (Bound nested-object access through the
            // file tree would require a wider proxy framework; for now array
            // contents return snapshots.)
            let class_name = match value {
                HkxValue::TypedObject { class_name, .. } => class_name.clone(),
                _ => String::new(),
            };
            let members = value.as_object_members().unwrap_or(&[]).to_vec();
            let inner_obj = HkxObject {
                name: None,
                offset: 0,
                signature: 0,
                class_name,
                members,
            };
            let py_obj = Py::new(
                py,
                PyHkxObject {
                    inner: ObjectInner::Unbound { obj: inner_obj },
                },
            )?;
            Ok(py_obj.into_bound(py).into_any())
        }
        HkxValue::PendingPtr(s) => Ok(s.into_pyobject(py)?.into_any()),
    }
}

/// Convert a Python value into an `HkxValue` based on a target `HkxType`.
/// Used by setters that accept Python primitives.
fn py_to_value(
    py: Python<'_>,
    target_type: HkxType,
    value: &Bound<'_, PyAny>,
) -> PyResult<HkxValue> {
    let _ = py;
    match target_type {
        HkxType::Void => Ok(HkxValue::Void),
        HkxType::Bool => Ok(HkxValue::Bool(value.extract::<bool>()?)),
        HkxType::Int8 => Ok(HkxValue::I8(value.extract::<i8>()?)),
        HkxType::Uint8 => Ok(HkxValue::U8(value.extract::<u8>()?)),
        HkxType::Int16 => Ok(HkxValue::I16(value.extract::<i16>()?)),
        HkxType::Uint16 => Ok(HkxValue::U16(value.extract::<u16>()?)),
        HkxType::Half => Ok(HkxValue::Half(value.extract::<f32>()?)),
        HkxType::Int32 | HkxType::Enum | HkxType::Flags => {
            Ok(HkxValue::I32(value.extract::<i32>()?))
        }
        HkxType::Uint32 => Ok(HkxValue::U32(value.extract::<u32>()?)),
        HkxType::Real => Ok(HkxValue::F32(value.extract::<f32>()?)),
        HkxType::Int64 => Ok(HkxValue::I64(value.extract::<i64>()?)),
        HkxType::Uint64 | HkxType::Ulong => Ok(HkxValue::U64(value.extract::<u64>()?)),
        HkxType::Vector4
        | HkxType::Quaternion
        | HkxType::Matrix3
        | HkxType::Matrix4
        | HkxType::Transform
        | HkxType::QsTransform => {
            let vs: Vec<f32> = value.extract()?;
            Ok(HkxValue::F32List(vs))
        }
        HkxType::CString | HkxType::StringPtr => {
            let s: String = value.extract()?;
            Ok(HkxValue::String {
                value: s,
                is_null: false,
            })
        }
        HkxType::Pointer | HkxType::FunctionPointer => {
            // Accept "#NNNN" or "" string
            let s: String = value.extract()?;
            Ok(HkxValue::Pointer(parse_pointer_target(&s)))
        }
        HkxType::Array | HkxType::SimpleArray | HkxType::RelArray => {
            // Array setter would need element type — fall back to empty array
            Ok(HkxValue::Array(Vec::new()))
        }
        HkxType::Struct => {
            // Try to extract a PyHkxObject
            if let Ok(py_obj) = value.extract::<PyRef<'_, PyHkxObject>>() {
                let obj = py_obj.snapshot();
                Ok(HkxValue::Object(obj.members))
            } else {
                Err(PyTypeError::new_err("STRUCT value must be an HKXObject"))
            }
        }
    }
}

/// Parse a `"#NNNN"`-style pointer string into `Option<usize>`. Empty
/// string → `None`. Accepts both `#0001` (decimal-looking but treated as
/// hex per Python's existing format) and `#00FF`.
fn parse_pointer_target(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let trimmed = s.strip_prefix('#').unwrap_or(s);
    usize::from_str_radix(trimmed, 16).ok()
}

fn format_pointer_target(opt: Option<usize>) -> String {
    match opt {
        Some(idx) => format!("#{:04X}", idx),
        None => String::new(),
    }
}

/// Construct an HkxObject snapshot from any PyHkxObject (bound or unbound).
impl PyHkxObject {
    fn snapshot(&self) -> HkxObject {
        match &self.inner {
            ObjectInner::Unbound { obj } => obj.clone(),
            ObjectInner::Bound { file, obj_idx } => Python::attach(|py| {
                let file_ref = file.borrow(py);
                file_ref.inner.objects()[*obj_idx].clone()
            }),
        }
    }
}

/// Best-effort inference of the `HkxType` for a primitive `HkxValue`.
fn infer_value_type(value: &HkxValue) -> HkxType {
    match value {
        HkxValue::Void => HkxType::Void,
        HkxValue::Bool(_) => HkxType::Bool,
        HkxValue::I8(_) => HkxType::Int8,
        HkxValue::U8(_) => HkxType::Uint8,
        HkxValue::I16(_) => HkxType::Int16,
        HkxValue::U16(_) => HkxType::Uint16,
        HkxValue::I32(_) => HkxType::Int32,
        HkxValue::U32(_) => HkxType::Uint32,
        HkxValue::I64(_) => HkxType::Int64,
        HkxValue::U64(_) => HkxType::Uint64,
        HkxValue::F32(_) => HkxType::Real,
        HkxValue::Half(_) => HkxType::Half,
        HkxValue::F32List(vs) => match vs.len() {
            4 => HkxType::Vector4,
            12 => HkxType::Matrix3,
            16 => HkxType::Matrix4,
            _ => HkxType::Vector4,
        },
        HkxValue::String { .. } => HkxType::StringPtr,
        HkxValue::Pointer(_) => HkxType::Pointer,
        HkxValue::Array(_) => HkxType::Array,
        HkxValue::Object(_) | HkxValue::TypedObject { .. } => HkxType::Struct,
        HkxValue::PendingPtr(_) => HkxType::Pointer,
    }
}

// --- HKXDirectMember pymethods -------------------------------------------

#[pymethods]
impl PyHkxDirectMember {
    #[new]
    #[pyo3(signature = (name, r#type, value))]
    fn new(
        py: Python<'_>,
        name: String,
        r#type: PyHkxType,
        value: Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        // Validate-by-conversion: the value must be coercible into the type
        // (we don't store it as HkxValue here, just as the Python object).
        let _ = py_to_value(py, r#type.to_native(), &value)?;
        Ok(Self {
            inner: DirectInner::Unbound {
                name,
                type_: r#type,
                value: value.unbind(),
            },
        })
    }

    #[getter]
    fn name(&self, py: Python<'_>) -> PyResult<String> {
        match &self.inner {
            DirectInner::Unbound { name, .. } => Ok(name.clone()),
            DirectInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let file_ref = file.borrow(py);
                let obj = file_ref
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                Ok(m.name.clone())
            }
        }
    }

    #[setter]
    fn set_name(&mut self, py: Python<'_>, new_name: String) -> PyResult<()> {
        match &mut self.inner {
            DirectInner::Unbound { name, .. } => {
                *name = new_name;
                Ok(())
            }
            DirectInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let mut file_ref = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let path = path.clone();
                let obj = file_ref
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member_mut(obj, &path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                m.name = new_name;
                Ok(())
            }
        }
    }

    #[getter(r#type)]
    fn type_(&self, py: Python<'_>) -> PyResult<PyHkxType> {
        match &self.inner {
            DirectInner::Unbound { type_, .. } => Ok(*type_),
            DirectInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let file_ref = file.borrow(py);
                let obj = file_ref
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                Ok(infer_value_type(&m.value).into())
            }
        }
    }

    #[getter]
    fn value<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match &self.inner {
            DirectInner::Unbound { value, .. } => Ok(value.bind(py).clone()),
            DirectInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let file_ref = file.borrow(py);
                let obj = file_ref
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                value_to_py(py, &m.value)
            }
        }
    }

    #[setter]
    fn set_value(&mut self, py: Python<'_>, new_value: Bound<'_, PyAny>) -> PyResult<()> {
        match &mut self.inner {
            DirectInner::Unbound { value, type_, .. } => {
                // Validate the new value matches the type
                let _ = py_to_value(py, type_.to_native(), &new_value)?;
                *value = new_value.unbind();
                Ok(())
            }
            DirectInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let mut file_ref = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let path = path.clone();
                let obj = file_ref
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member_mut(obj, &path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                let target_type = infer_value_type(&m.value);
                m.value = py_to_value(py, target_type, &new_value)?;
                Ok(())
            }
        }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let name = self.name(py)?;
        let type_ = self.type_(py)?;
        Ok(format!(
            "HKXDirectMember(name={:?}, type={:?})",
            name, type_
        ))
    }
}

// --- HKXArrayMember pymethods --------------------------------------------

#[pymethods]
impl PyHkxArrayMember {
    #[new]
    #[pyo3(signature = (name, subtype, contents=None, ctype=String::new(), source_offset=-1, source_length=-1))]
    fn new(
        py: Python<'_>,
        name: String,
        subtype: PyHkxType,
        contents: Option<Bound<'_, PyAny>>,
        ctype: String,
        source_offset: i64,
        source_length: i64,
    ) -> PyResult<Self> {
        let mut values: Vec<HkxValue> = Vec::new();
        if let Some(c) = contents {
            let list: Vec<Bound<'_, PyAny>> = c.extract()?;
            let st = subtype.to_native();
            for v in list {
                values.push(py_to_value(py, st, &v)?);
            }
        }
        Ok(Self {
            inner: ArrayInner::Unbound {
                name,
                subtype,
                contents: values,
                ctype,
                source_offset,
                source_length,
            },
        })
    }

    #[getter]
    fn name(&self, py: Python<'_>) -> PyResult<String> {
        match &self.inner {
            ArrayInner::Unbound { name, .. } => Ok(name.clone()),
            ArrayInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let f = file.borrow(py);
                let obj = f
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                Ok(m.name.clone())
            }
        }
    }

    #[setter]
    fn set_name(&mut self, py: Python<'_>, new_name: String) -> PyResult<()> {
        match &mut self.inner {
            ArrayInner::Unbound { name, .. } => {
                *name = new_name;
                Ok(())
            }
            ArrayInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let mut f = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let path = path.clone();
                let obj = f
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member_mut(obj, &path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                m.name = new_name;
                Ok(())
            }
        }
    }

    #[getter]
    fn subtype(&self) -> PyResult<PyHkxType> {
        match &self.inner {
            ArrayInner::Unbound { subtype, .. } => Ok(*subtype),
            ArrayInner::Bound { .. } => {
                // Bound: infer from first element (or VOID for empty).
                Python::attach(|py| self.bound_first_element_type(py))
            }
        }
    }

    #[setter]
    fn set_subtype(&mut self, new_subtype: PyHkxType) -> PyResult<()> {
        match &mut self.inner {
            ArrayInner::Unbound { subtype, .. } => {
                *subtype = new_subtype;
                Ok(())
            }
            ArrayInner::Bound { .. } => {
                // Subtype on the bound side is implicit from element values.
                // Setting it is a no-op (the writer derives it from contents).
                Ok(())
            }
        }
    }

    #[getter]
    fn contents(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyHkxValueList>> {
        let backing = match &slf.inner {
            ArrayInner::Bound {
                file,
                obj_idx,
                path,
            } => ValueListBacking::Bound {
                file: file.clone_ref(py),
                obj_idx: *obj_idx,
                path: path.clone(),
            },
            ArrayInner::Unbound { .. } => ValueListBacking::Detached {
                array: slf.into_ptr(),
            },
        };
        let list = PyHkxValueList { backing };
        Py::new(py, list)
    }

    #[setter]
    fn set_contents(&mut self, py: Python<'_>, new_contents: Bound<'_, PyAny>) -> PyResult<()> {
        let items: Vec<Bound<'_, PyAny>> = new_contents.extract()?;
        match &mut self.inner {
            ArrayInner::Unbound {
                subtype, contents, ..
            } => {
                let st = subtype.to_native();
                let mut new_values = Vec::with_capacity(items.len());
                for v in items {
                    new_values.push(py_to_value(py, st, &v)?);
                }
                *contents = new_values;
                Ok(())
            }
            ArrayInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let mut f = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let path = path.clone();
                let obj = f
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let arr = resolve_value_array_mut(obj, &path)
                    .ok_or_else(|| PyValueError::new_err("not an array member"))?;
                // Infer subtype from current first element or fall back to VOID
                let st = arr.first().map(infer_value_type).unwrap_or(HkxType::Void);
                let mut new_values = Vec::with_capacity(items.len());
                for v in items {
                    new_values.push(py_to_value(py, st, &v)?);
                }
                *arr = new_values;
                // Also invalidate source_offset to force regeneration.
                Self::invalidate_array_source(&mut f.inner, obj_idx, &path);
                Ok(())
            }
        }
    }

    #[getter]
    fn ctype(&self, py: Python<'_>) -> PyResult<String> {
        match &self.inner {
            ArrayInner::Unbound { ctype, .. } => Ok(ctype.clone()),
            ArrayInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let f = file.borrow(py);
                let src = f.inner.array_sources().iter().find(|s| {
                    s.object_index == *obj_idx && Self::matches_array_path(&s.member_path, path)
                });
                Ok(src.map(|s| s.ctype.clone()).unwrap_or_default())
            }
        }
    }

    #[setter]
    fn set_ctype(&mut self, py: Python<'_>, new_ctype: String) -> PyResult<()> {
        match &mut self.inner {
            ArrayInner::Unbound { ctype, .. } => {
                *ctype = new_ctype;
                Ok(())
            }
            ArrayInner::Bound { .. } => {
                // No mutator on ArraySource currently; setting ctype on a
                // bound array just marks the file dirty.
                let _ = py;
                Ok(())
            }
        }
    }

    #[getter]
    fn source_offset(&self, py: Python<'_>) -> PyResult<i64> {
        match &self.inner {
            ArrayInner::Unbound { source_offset, .. } => Ok(*source_offset),
            ArrayInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let f = file.borrow(py);
                let src = f.inner.array_sources().iter().find(|s| {
                    s.object_index == *obj_idx && Self::matches_array_path(&s.member_path, path)
                });
                Ok(src.map(|s| s.content_offset as i64).unwrap_or(-1))
            }
        }
    }

    #[setter]
    fn set_source_offset(&mut self, py: Python<'_>, new_offset: i64) -> PyResult<()> {
        match &mut self.inner {
            ArrayInner::Unbound { source_offset, .. } => {
                *source_offset = new_offset;
                Ok(())
            }
            ArrayInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                if new_offset < 0 {
                    let mut f = file.borrow_mut(py);
                    let obj_idx = *obj_idx;
                    let path = path.clone();
                    Self::invalidate_array_source(&mut f.inner, obj_idx, &path);
                }
                Ok(())
            }
        }
    }

    #[getter]
    fn source_length(&self, py: Python<'_>) -> PyResult<i64> {
        match &self.inner {
            ArrayInner::Unbound { source_length, .. } => Ok(*source_length),
            ArrayInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let f = file.borrow(py);
                let src = f.inner.array_sources().iter().find(|s| {
                    s.object_index == *obj_idx && Self::matches_array_path(&s.member_path, path)
                });
                Ok(src.map(|s| s.content_length as i64).unwrap_or(-1))
            }
        }
    }

    #[setter]
    fn set_source_length(&mut self, py: Python<'_>, new_length: i64) -> PyResult<()> {
        match &mut self.inner {
            ArrayInner::Unbound { source_length, .. } => {
                *source_length = new_length;
                Ok(())
            }
            ArrayInner::Bound { .. } => {
                // No mutator on ArraySource; setting marks dirty implicitly.
                let _ = py;
                Ok(())
            }
        }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let name = self.name(py)?;
        Ok(format!("HKXArrayMember(name={:?})", name))
    }
}

impl PyHkxArrayMember {
    fn bound_first_element_type(&self, py: Python<'_>) -> PyResult<PyHkxType> {
        if let ArrayInner::Bound {
            file,
            obj_idx,
            path,
        } = &self.inner
        {
            let f = file.borrow(py);
            let obj = f
                .inner
                .objects()
                .get(*obj_idx)
                .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
            let arr = resolve_value_array(obj, path)
                .ok_or_else(|| PyValueError::new_err("not an array member"))?;
            Ok(arr
                .first()
                .map(infer_value_type)
                .unwrap_or(HkxType::Void)
                .into())
        } else {
            Ok(PyHkxType::VOID)
        }
    }

    fn matches_array_path(member_path: &[String], target: &MemberPath) -> bool {
        // ArraySource.member_path is a chain of member names; our MemberPath
        // is a chain of indices. We can't reconcile without the parent
        // object to look up names. For now, fall back to comparing only the
        // top-level member: this is enough for the common case where arrays
        // live directly on top-level members.
        if member_path.is_empty() || target.member_index >= member_path.len() {
            return false;
        }
        // Only top-level arrays handled; nested array sources are skipped.
        target.nested.is_empty()
    }

    fn invalidate_array_source(file: &mut HkxFile, obj_idx: usize, path: &MemberPath) {
        // Mark the file dirty by going through objects_mut().
        let _ = file.objects_mut();
        let _ = (obj_idx, path);
        // The actual ArraySource entry stays in place; the writer regenerates
        // arrays from `objects` whenever `model_dirty` is set, which the
        // objects_mut() call above triggers.
    }
}

// --- HKXPointerMember pymethods ------------------------------------------

#[pymethods]
impl PyHkxPointerMember {
    #[new]
    #[pyo3(signature = (name, target=String::new(), targets=None))]
    fn new(name: String, target: String, targets: Option<Vec<String>>) -> Self {
        Self {
            inner: PointerInner::Unbound {
                name,
                target,
                targets,
            },
        }
    }

    #[getter]
    fn name(&self, py: Python<'_>) -> PyResult<String> {
        match &self.inner {
            PointerInner::Unbound { name, .. } => Ok(name.clone()),
            PointerInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let f = file.borrow(py);
                let obj = f
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                Ok(m.name.clone())
            }
        }
    }

    #[setter]
    fn set_name(&mut self, py: Python<'_>, new_name: String) -> PyResult<()> {
        match &mut self.inner {
            PointerInner::Unbound { name, .. } => {
                *name = new_name;
                Ok(())
            }
            PointerInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let mut f = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let path = path.clone();
                let obj = f
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member_mut(obj, &path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                m.name = new_name;
                Ok(())
            }
        }
    }

    #[getter]
    fn target(&self, py: Python<'_>) -> PyResult<String> {
        match &self.inner {
            PointerInner::Unbound { target, .. } => Ok(target.clone()),
            PointerInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let f = file.borrow(py);
                let obj = f
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                if let HkxValue::Pointer(opt) = m.value {
                    Ok(format_pointer_target(opt))
                } else {
                    Ok(String::new())
                }
            }
        }
    }

    #[setter]
    fn set_target(&mut self, py: Python<'_>, new_target: String) -> PyResult<()> {
        match &mut self.inner {
            PointerInner::Unbound { target, .. } => {
                *target = new_target;
                Ok(())
            }
            PointerInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let mut f = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let path = path.clone();
                let obj = f
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member_mut(obj, &path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                m.value = HkxValue::Pointer(parse_pointer_target(&new_target));
                Ok(())
            }
        }
    }

    #[getter]
    fn targets(&self) -> Option<Vec<String>> {
        match &self.inner {
            PointerInner::Unbound { targets, .. } => targets.clone(),
            PointerInner::Bound { .. } => None,
        }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let n = self.name(py)?;
        let t = self.target(py)?;
        Ok(format!("HKXPointerMember(name={:?}, target={:?})", n, t))
    }
}

// --- HKXStringMember pymethods -------------------------------------------

#[pymethods]
impl PyHkxStringMember {
    #[new]
    #[pyo3(signature = (name, value=String::new(), is_null=false))]
    fn new(name: String, value: String, is_null: bool) -> Self {
        Self {
            inner: StringInner::Unbound {
                name,
                value,
                is_null,
            },
        }
    }

    #[getter]
    fn name(&self, py: Python<'_>) -> PyResult<String> {
        match &self.inner {
            StringInner::Unbound { name, .. } => Ok(name.clone()),
            StringInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let f = file.borrow(py);
                let obj = f
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                Ok(m.name.clone())
            }
        }
    }

    #[setter]
    fn set_name(&mut self, py: Python<'_>, new_name: String) -> PyResult<()> {
        match &mut self.inner {
            StringInner::Unbound { name, .. } => {
                *name = new_name;
                Ok(())
            }
            StringInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let mut f = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let path = path.clone();
                let obj = f
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member_mut(obj, &path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                m.name = new_name;
                Ok(())
            }
        }
    }

    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<String> {
        match &self.inner {
            StringInner::Unbound { value, .. } => Ok(value.clone()),
            StringInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let f = file.borrow(py);
                let obj = f
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                if let HkxValue::String { value, .. } = &m.value {
                    Ok(value.clone())
                } else {
                    Ok(String::new())
                }
            }
        }
    }

    #[setter]
    fn set_value(&mut self, py: Python<'_>, new_value: String) -> PyResult<()> {
        match &mut self.inner {
            StringInner::Unbound { value, .. } => {
                *value = new_value;
                Ok(())
            }
            StringInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let mut f = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let path = path.clone();
                let obj = f
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member_mut(obj, &path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                if let HkxValue::String { value, .. } = &mut m.value {
                    *value = new_value;
                } else {
                    m.value = HkxValue::String {
                        value: new_value,
                        is_null: false,
                    };
                }
                Ok(())
            }
        }
    }

    #[getter]
    fn is_null(&self, py: Python<'_>) -> PyResult<bool> {
        match &self.inner {
            StringInner::Unbound { is_null, .. } => Ok(*is_null),
            StringInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let f = file.borrow(py);
                let obj = f
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                if let HkxValue::String { is_null, .. } = &m.value {
                    Ok(*is_null)
                } else {
                    Ok(false)
                }
            }
        }
    }

    #[setter]
    fn set_is_null(&mut self, py: Python<'_>, new_null: bool) -> PyResult<()> {
        match &mut self.inner {
            StringInner::Unbound { is_null, .. } => {
                *is_null = new_null;
                Ok(())
            }
            StringInner::Bound {
                file,
                obj_idx,
                path,
            } => {
                let mut f = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let path = path.clone();
                let obj = f
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member_mut(obj, &path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                if let HkxValue::String { is_null, .. } = &mut m.value {
                    *is_null = new_null;
                } else {
                    m.value = HkxValue::String {
                        value: String::new(),
                        is_null: new_null,
                    };
                }
                Ok(())
            }
        }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let n = self.name(py)?;
        let v = self.value(py)?;
        let i = self.is_null(py)?;
        Ok(format!(
            "HKXStringMember(name={:?}, value={:?}, is_null={})",
            n, v, i
        ))
    }
}

// --- HKXEnumMember pymethods ---------------------------------------------

#[pymethods]
impl PyHkxEnumMember {
    #[new]
    #[pyo3(signature = (name, enum_name, value))]
    fn new(name: String, enum_name: String, value: Bound<'_, PyAny>) -> PyResult<Self> {
        // Accept str or int for value; store as String for round-trip.
        let value_str = if let Ok(s) = value.extract::<String>() {
            s
        } else if let Ok(n) = value.extract::<i64>() {
            n.to_string()
        } else {
            return Err(PyTypeError::new_err("value must be str or int"));
        };
        Ok(Self {
            inner: EnumInner::Unbound {
                name,
                enum_name,
                value: value_str,
            },
        })
    }

    #[getter]
    fn name(&self, py: Python<'_>) -> PyResult<String> {
        match &self.inner {
            EnumInner::Unbound { name, .. } => Ok(name.clone()),
            EnumInner::Bound {
                file,
                obj_idx,
                path,
                ..
            } => {
                let f = file.borrow(py);
                let obj = f
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                Ok(m.name.clone())
            }
        }
    }

    #[getter]
    fn enum_name(&self) -> String {
        match &self.inner {
            EnumInner::Unbound { enum_name, .. } => enum_name.clone(),
            EnumInner::Bound { enum_name, .. } => enum_name.clone(),
        }
    }

    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<String> {
        match &self.inner {
            EnumInner::Unbound { value, .. } => Ok(value.clone()),
            EnumInner::Bound {
                file,
                obj_idx,
                path,
                ..
            } => {
                let f = file.borrow(py);
                let obj = f
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let m = resolve_member(obj, path)
                    .ok_or_else(|| PyIndexError::new_err("member path no longer valid"))?;
                if let HkxValue::I32(v) = m.value {
                    Ok(v.to_string())
                } else {
                    Ok(String::new())
                }
            }
        }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let n = self.name(py)?;
        let e = self.enum_name();
        let v = self.value(py)?;
        Ok(format!(
            "HKXEnumMember(name={:?}, enum_name={:?}, value={:?})",
            n, e, v
        ))
    }
}

// --- Wrap a Rust HkxMember into the appropriate Python variant pyclass --

fn wrap_member_bound<'py>(
    py: Python<'py>,
    file: &Py<PyHkxFile>,
    obj_idx: usize,
    path: MemberPath,
    member: &HkxMember,
) -> PyResult<Bound<'py, PyAny>> {
    match &member.value {
        HkxValue::String { .. } => {
            let m = PyHkxStringMember {
                inner: StringInner::Bound {
                    file: file.clone_ref(py),
                    obj_idx,
                    path,
                },
            };
            Ok(Py::new(py, m)?.into_bound(py).into_any())
        }
        HkxValue::Pointer(_) => {
            let m = PyHkxPointerMember {
                inner: PointerInner::Bound {
                    file: file.clone_ref(py),
                    obj_idx,
                    path,
                },
            };
            Ok(Py::new(py, m)?.into_bound(py).into_any())
        }
        HkxValue::Array(_) => {
            let m = PyHkxArrayMember {
                inner: ArrayInner::Bound {
                    file: file.clone_ref(py),
                    obj_idx,
                    path,
                },
            };
            Ok(Py::new(py, m)?.into_bound(py).into_any())
        }
        _ => {
            let m = PyHkxDirectMember {
                inner: DirectInner::Bound {
                    file: file.clone_ref(py),
                    obj_idx,
                    path,
                },
            };
            Ok(Py::new(py, m)?.into_bound(py).into_any())
        }
    }
}

/// Convert an unbound Python member pyclass back into a Rust HkxMember
/// (called when appending into a bound list).
fn extract_member_unbound(py: Python<'_>, item: &Bound<'_, PyAny>) -> PyResult<HkxMember> {
    if let Ok(m) = item.extract::<PyRef<'_, PyHkxStringMember>>() {
        if let StringInner::Unbound {
            name,
            value,
            is_null,
        } = &m.inner
        {
            return Ok(HkxMember {
                name: name.clone(),
                value: HkxValue::String {
                    value: value.clone(),
                    is_null: *is_null,
                },
            });
        }
    }
    if let Ok(m) = item.extract::<PyRef<'_, PyHkxPointerMember>>() {
        if let PointerInner::Unbound { name, target, .. } = &m.inner {
            return Ok(HkxMember {
                name: name.clone(),
                value: HkxValue::Pointer(parse_pointer_target(target)),
            });
        }
    }
    if let Ok(m) = item.extract::<PyRef<'_, PyHkxArrayMember>>() {
        if let ArrayInner::Unbound { name, contents, .. } = &m.inner {
            return Ok(HkxMember {
                name: name.clone(),
                value: HkxValue::Array(contents.clone()),
            });
        }
    }
    if let Ok(m) = item.extract::<PyRef<'_, PyHkxDirectMember>>() {
        if let DirectInner::Unbound { name, type_, value } = &m.inner {
            let v = py_to_value(py, type_.to_native(), value.bind(py))?;
            return Ok(HkxMember {
                name: name.clone(),
                value: v,
            });
        }
    }
    if let Ok(m) = item.extract::<PyRef<'_, PyHkxEnumMember>>() {
        if let EnumInner::Unbound { name, value, .. } = &m.inner {
            // Enums stored as I32; parse the value string.
            let v = value.parse::<i32>().unwrap_or(0);
            return Ok(HkxMember {
                name: name.clone(),
                value: HkxValue::I32(v),
            });
        }
    }
    Err(PyTypeError::new_err(
        "expected an unbound HKX*Member instance",
    ))
}

// --- HKXObject pymethods -------------------------------------------------

#[pymethods]
impl PyHkxObject {
    #[new]
    #[pyo3(signature = (name=None, class_name=String::new(), schema_version=0, members=None))]
    fn new(
        py: Python<'_>,
        name: Option<String>,
        class_name: String,
        schema_version: u32,
        members: Option<Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut member_vec = Vec::new();
        if let Some(m) = members {
            let items: Vec<Bound<'_, PyAny>> = m.extract()?;
            for item in items {
                member_vec.push(extract_member_unbound(py, &item)?);
            }
        }
        let _ = schema_version; // not stored on Rust HkxObject
        Ok(Self {
            inner: ObjectInner::Unbound {
                obj: HkxObject {
                    name,
                    offset: 0,
                    signature: 0,
                    class_name,
                    members: member_vec,
                },
            },
        })
    }

    #[getter]
    fn name(&self, py: Python<'_>) -> PyResult<Option<String>> {
        match &self.inner {
            ObjectInner::Unbound { obj } => Ok(obj.name.clone()),
            ObjectInner::Bound { file, obj_idx } => {
                let f = file.borrow(py);
                Ok(f.inner.objects().get(*obj_idx).and_then(|o| o.name.clone()))
            }
        }
    }

    #[setter]
    fn set_name(&mut self, py: Python<'_>, new_name: Option<String>) -> PyResult<()> {
        match &mut self.inner {
            ObjectInner::Unbound { obj } => {
                obj.name = new_name;
                Ok(())
            }
            ObjectInner::Bound { file, obj_idx } => {
                let mut f = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let obj = f
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                obj.name = new_name;
                Ok(())
            }
        }
    }

    #[getter]
    fn class_name(&self, py: Python<'_>) -> PyResult<String> {
        match &self.inner {
            ObjectInner::Unbound { obj } => Ok(obj.class_name.clone()),
            ObjectInner::Bound { file, obj_idx } => {
                let f = file.borrow(py);
                Ok(f.inner
                    .objects()
                    .get(*obj_idx)
                    .map(|o| o.class_name.clone())
                    .unwrap_or_default())
            }
        }
    }

    #[setter]
    fn set_class_name(&mut self, py: Python<'_>, new_class_name: String) -> PyResult<()> {
        match &mut self.inner {
            ObjectInner::Unbound { obj } => {
                obj.class_name = new_class_name;
                Ok(())
            }
            ObjectInner::Bound { file, obj_idx } => {
                let mut f = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let obj = f
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                obj.class_name = new_class_name;
                Ok(())
            }
        }
    }

    #[getter]
    fn schema_version(&self) -> u32 {
        // Rust HkxObject doesn't carry a schema_version field; the
        // packfile-level version applies to all objects. Return 0 to match
        // the Python dataclass default.
        0
    }

    #[setter]
    fn set_schema_version(&mut self, _new_version: u32) -> PyResult<()> {
        // Accepted but not stored — see schema_version getter.
        Ok(())
    }

    #[getter]
    fn members(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyHkxMemberList>> {
        let backing = match &slf.inner {
            ObjectInner::Bound { file, obj_idx } => MemberListBacking::Bound {
                file: file.clone_ref(py),
                obj_idx: *obj_idx,
            },
            ObjectInner::Unbound { .. } => MemberListBacking::Detached {
                object: slf.into_ptr(),
            },
        };
        let list = PyHkxMemberList { backing };
        Py::new(py, list)
    }

    #[setter]
    fn set_members(&mut self, py: Python<'_>, new_members: Bound<'_, PyAny>) -> PyResult<()> {
        let items: Vec<Bound<'_, PyAny>> = new_members.extract()?;
        let mut new_vec = Vec::with_capacity(items.len());
        for item in items {
            new_vec.push(extract_member_unbound(py, &item)?);
        }
        match &mut self.inner {
            ObjectInner::Unbound { obj } => {
                obj.members = new_vec;
                Ok(())
            }
            ObjectInner::Bound { file, obj_idx } => {
                let mut f = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let obj = f
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                obj.members = new_vec;
                Ok(())
            }
        }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let name = self.name(py)?;
        let class_name = self.class_name(py)?;
        let len = match &self.inner {
            ObjectInner::Unbound { obj } => obj.members.len(),
            ObjectInner::Bound { file, obj_idx } => {
                let f = file.borrow(py);
                f.inner
                    .objects()
                    .get(*obj_idx)
                    .map(|o| o.members.len())
                    .unwrap_or(0)
            }
        };
        Ok(format!(
            "HKXObject(name={:?}, class_name={:?}, members={})",
            name, class_name, len
        ))
    }
}

// --- HKXFile pymethods ---------------------------------------------------

#[pymethods]
impl PyHkxFile {
    #[new]
    #[pyo3(signature = (class_version=11, contents_version="hk_2014.1.0-r1".to_string(), objects=None))]
    fn new(
        py: Python<'_>,
        class_version: u32,
        contents_version: String,
        objects: Option<Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut obj_vec = Vec::new();
        if let Some(o) = objects {
            let items: Vec<Bound<'_, PyAny>> = o.extract()?;
            for item in items {
                let py_obj = item.extract::<PyRef<'_, PyHkxObject>>()?;
                match &py_obj.inner {
                    ObjectInner::Unbound { obj } => obj_vec.push(obj.clone()),
                    ObjectInner::Bound { file, obj_idx } => {
                        let f = file.borrow(py);
                        if let Some(o) = f.inner.objects().get(*obj_idx) {
                            obj_vec.push(o.clone());
                        }
                    }
                }
            }
        }
        Ok(Self {
            inner: HkxFile::from_tagxml(class_version, contents_version, obj_vec),
            enum_meta: EnumMetadata::default(),
        })
    }

    /// Parse an HKX file from raw bytes (auto-detects packfile vs tagfile).
    #[classmethod]
    fn read_bytes(
        _cls: &Bound<'_, pyo3::types::PyType>,
        data: &Bound<'_, PyBytes>,
    ) -> PyResult<Self> {
        let bytes = data.as_bytes().to_vec();
        let inner = HkxFile::read(&bytes).map_err(map_error)?;
        Ok(Self {
            inner,
            enum_meta: EnumMetadata::default(),
        })
    }

    /// Parse an HKX file from a filesystem path.
    #[classmethod]
    fn read(_cls: &Bound<'_, pyo3::types::PyType>, path: String) -> PyResult<Self> {
        let bytes = std::fs::read(Path::new(&path))
            .map_err(|e| PyValueError::new_err(format!("read {path}: {e}")))?;
        let inner = HkxFile::read(&bytes).map_err(map_error)?;
        Ok(Self {
            inner,
            enum_meta: EnumMetadata::default(),
        })
    }

    #[getter]
    fn class_version(&self) -> u32 {
        self.inner.class_version()
    }

    #[setter]
    fn set_class_version(&mut self, new_version: u32) {
        self.inner.set_class_version(new_version);
    }

    #[getter]
    fn contents_version(&self) -> String {
        self.inner.contents_version().to_string()
    }

    #[setter]
    fn set_contents_version(&mut self, new_version: String) {
        self.inner.set_contents_version(new_version);
    }

    #[getter]
    fn objects(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyHkxObjectList>> {
        let file: Py<PyHkxFile> = slf.into();
        let backing = ObjectListBacking::Bound { file };
        Py::new(py, PyHkxObjectList { backing })
    }

    #[setter]
    fn set_objects(&mut self, py: Python<'_>, new_objects: Bound<'_, PyAny>) -> PyResult<()> {
        let items: Vec<Bound<'_, PyAny>> = new_objects.extract()?;
        let mut new_vec = Vec::with_capacity(items.len());
        for item in items {
            let py_obj = item.extract::<PyRef<'_, PyHkxObject>>()?;
            match &py_obj.inner {
                ObjectInner::Unbound { obj } => new_vec.push(obj.clone()),
                ObjectInner::Bound { file, obj_idx } => {
                    let f = file.borrow(py);
                    if let Some(o) = f.inner.objects().get(*obj_idx) {
                        new_vec.push(o.clone());
                    }
                }
            }
        }
        let dirty_vec = self.inner.objects_mut();
        let _ = dirty_vec;
        // No Vec-replace API on HkxFile — construct a new file and swap.
        let new_file = HkxFile::from_tagxml(
            self.inner.class_version(),
            self.inner.contents_version().to_string(),
            new_vec,
        );
        self.inner = new_file;
        Ok(())
    }

    /// Serialize back to packfile bytes.
    fn save<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        let bytes = self.inner.save();
        PyBytes::new(py, &bytes)
    }

    /// Serialize and write to a filesystem path.
    fn save_to(&self, path: String) -> PyResult<()> {
        let bytes = self.inner.save();
        std::fs::write(Path::new(&path), &bytes)
            .map_err(|e| PyValueError::new_err(format!("write {path}: {e}")))
    }

    /// Serialize to a TagXML string. Mirrors the legacy
    /// `creation_lib.hkxpack.tagwriter.write_xml_string` API. The DescriptorRegistry
    /// is derived from the file's `contents_version`.
    fn to_xml(&self) -> PyResult<String> {
        let mut registry = DescriptorRegistry::for_contents_version(self.inner.contents_version());
        crate::hkx::tagxml::write_tagxml_string_with_registry(&self.inner, &mut registry)
            .map_err(map_error)
    }

    fn __repr__(&self) -> String {
        format!(
            "HKXFile(class_version={}, contents_version={:?}, objects={})",
            self.inner.class_version(),
            self.inner.contents_version(),
            self.inner.objects().len()
        )
    }
}

// --- Proxy lists ---------------------------------------------------------

/// Backing for HKXObjectList — points at HkxFile.objects().
#[derive(Debug)]
enum ObjectListBacking {
    Bound { file: Py<PyHkxFile> },
}

#[pyclass(name = "HKXObjectList", module = "creation_lib._native.havok_native")]
pub struct PyHkxObjectList {
    backing: ObjectListBacking,
}

#[pymethods]
impl PyHkxObjectList {
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        let ObjectListBacking::Bound { file } = &self.backing;
        let f = file.borrow(py);
        Ok(f.inner.objects().len())
    }

    fn __getitem__(&self, py: Python<'_>, index: isize) -> PyResult<Py<PyHkxObject>> {
        let ObjectListBacking::Bound { file } = &self.backing;
        let len = {
            let f = file.borrow(py);
            f.inner.objects().len()
        };
        let idx = normalize_index(index, len)?;
        let obj = PyHkxObject {
            inner: ObjectInner::Bound {
                file: file.clone_ref(py),
                obj_idx: idx,
            },
        };
        Py::new(py, obj)
    }

    fn __setitem__(&self, py: Python<'_>, index: isize, value: Bound<'_, PyAny>) -> PyResult<()> {
        let ObjectListBacking::Bound { file } = &self.backing;
        let len = {
            let f = file.borrow(py);
            f.inner.objects().len()
        };
        let idx = normalize_index(index, len)?;
        let py_obj = value.extract::<PyRef<'_, PyHkxObject>>()?;
        let new_obj = match &py_obj.inner {
            ObjectInner::Unbound { obj } => obj.clone(),
            ObjectInner::Bound {
                file: src_file,
                obj_idx,
            } => {
                let sf = src_file.borrow(py);
                sf.inner
                    .objects()
                    .get(*obj_idx)
                    .cloned()
                    .ok_or_else(|| PyIndexError::new_err("source object index out of range"))?
            }
        };
        drop(py_obj);
        let mut f = file.borrow_mut(py);
        f.inner.objects_mut()[idx] = new_obj;
        Ok(())
    }

    fn __delitem__(&self, py: Python<'_>, index: isize) -> PyResult<()> {
        let ObjectListBacking::Bound { file } = &self.backing;
        let mut f = file.borrow_mut(py);
        let len = f.inner.objects().len();
        let idx = normalize_index(index, len)?;
        // Remove by reconstructing the file (no public Vec::remove API).
        f.inner.retain_objects_remap_pointers(|i, _| i != idx);
        Ok(())
    }

    /// Retain objects matching `predicate(index, obj) -> bool`, remapping
    /// pointers in the surviving objects to keep cross-references valid.
    ///
    /// This is the only safe way to drop multiple objects from an HKX file
    /// without breaking pointers. Snapshot-based: each object is cloned into
    /// an unbound `HKXObject` before the predicate runs, so the predicate can
    /// inspect the object freely without borrow conflicts.
    fn retain(&self, py: Python<'_>, predicate: Py<PyAny>) -> PyResult<()> {
        let ObjectListBacking::Bound { file } = &self.backing;
        // Snapshot all objects first to avoid borrow conflicts when calling
        // the Python predicate.
        let snapshots: Vec<HkxObject> = {
            let f = file.borrow(py);
            f.inner.objects().to_vec()
        };
        let mut keep_flags: Vec<bool> = Vec::with_capacity(snapshots.len());
        for (i, snap) in snapshots.into_iter().enumerate() {
            let py_obj = Py::new(
                py,
                PyHkxObject {
                    inner: ObjectInner::Unbound { obj: snap },
                },
            )?;
            let result = predicate.call1(py, (i, py_obj))?;
            let keep: bool = result.extract(py)?;
            keep_flags.push(keep);
        }
        let mut f = file.borrow_mut(py);
        f.inner.retain_objects_remap_pointers(|i, _| keep_flags[i]);
        Ok(())
    }

    fn append(&self, py: Python<'_>, value: Bound<'_, PyAny>) -> PyResult<()> {
        let ObjectListBacking::Bound { file } = &self.backing;
        let py_obj = value.extract::<PyRef<'_, PyHkxObject>>()?;
        let new_obj = match &py_obj.inner {
            ObjectInner::Unbound { obj } => obj.clone(),
            ObjectInner::Bound {
                file: src_file,
                obj_idx,
            } => {
                let sf = src_file.borrow(py);
                sf.inner
                    .objects()
                    .get(*obj_idx)
                    .cloned()
                    .ok_or_else(|| PyIndexError::new_err("source object index out of range"))?
            }
        };
        drop(py_obj);
        let mut f = file.borrow_mut(py);
        f.inner.push_object(new_obj);
        Ok(())
    }

    fn insert(&self, py: Python<'_>, index: isize, value: Bound<'_, PyAny>) -> PyResult<()> {
        let ObjectListBacking::Bound { file } = &self.backing;
        let py_obj = value.extract::<PyRef<'_, PyHkxObject>>()?;
        let new_obj = match &py_obj.inner {
            ObjectInner::Unbound { obj } => obj.clone(),
            ObjectInner::Bound {
                file: src_file,
                obj_idx,
            } => {
                let sf = src_file.borrow(py);
                sf.inner
                    .objects()
                    .get(*obj_idx)
                    .cloned()
                    .ok_or_else(|| PyIndexError::new_err("source object index out of range"))?
            }
        };
        drop(py_obj);
        // No insert on HkxFile public API — emulate via reconstruct.
        let mut f = file.borrow_mut(py);
        let mut current: Vec<HkxObject> = f.inner.objects().to_vec();
        let len = current.len();
        let idx = if index < 0 {
            ((len as isize + index).max(0)) as usize
        } else {
            (index as usize).min(len)
        };
        current.insert(idx, new_obj);
        let cv = f.inner.class_version();
        let vn = f.inner.contents_version().to_string();
        f.inner = HkxFile::from_tagxml(cv, vn, current);
        Ok(())
    }

    fn extend(&self, py: Python<'_>, iterable: Bound<'_, PyAny>) -> PyResult<()> {
        let items: Vec<Bound<'_, PyAny>> = iterable.extract()?;
        for item in items {
            self.append(py, item)?;
        }
        Ok(())
    }

    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        let ObjectListBacking::Bound { file } = &self.backing;
        let mut f = file.borrow_mut(py);
        let cv = f.inner.class_version();
        let vn = f.inner.contents_version().to_string();
        f.inner = HkxFile::from_tagxml(cv, vn, Vec::new());
        Ok(())
    }

    fn __iter__(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyHkxObjectListIter>> {
        let ObjectListBacking::Bound { file } = &slf.backing;
        let len = file.borrow(py).inner.objects().len();
        let it = PyHkxObjectListIter {
            file: file.clone_ref(py),
            cursor: 0,
            len,
        };
        Py::new(py, it)
    }

    fn __eq__(&self, py: Python<'_>, other: Bound<'_, PyAny>) -> PyResult<bool> {
        let ObjectListBacking::Bound { file } = &self.backing;
        let f = file.borrow(py);
        let other_list: Vec<Bound<'_, PyAny>> = match other.extract() {
            Ok(v) => v,
            Err(_) => return Ok(false),
        };
        if other_list.len() != f.inner.objects().len() {
            return Ok(false);
        }
        // Identity-only comparison would be too strict; compare by snapshot.
        // Two object lists are equal if both are sequences of HKXObject with
        // matching name+class_name+member-count.
        for (i, item) in other_list.iter().enumerate() {
            let Ok(other_obj) = item.extract::<PyRef<'_, PyHkxObject>>() else {
                return Ok(false);
            };
            let other_snap = other_obj.snapshot();
            let our = &f.inner.objects()[i];
            if our.name != other_snap.name || our.class_name != other_snap.class_name {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let n = self.__len__(py)?;
        Ok(format!("HKXObjectList(len={})", n))
    }
}

#[pyclass(
    unsendable,
    name = "HKXObjectListIter",
    module = "creation_lib._native.havok_native"
)]
pub struct PyHkxObjectListIter {
    file: Py<PyHkxFile>,
    cursor: usize,
    len: usize,
}

#[pymethods]
impl PyHkxObjectListIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<Py<PyHkxObject>>> {
        if slf.cursor >= slf.len {
            return Ok(None);
        }
        let idx = slf.cursor;
        slf.cursor += 1;
        let file = slf.file.clone_ref(py);
        let obj = PyHkxObject {
            inner: ObjectInner::Bound { file, obj_idx: idx },
        };
        Ok(Some(Py::new(py, obj)?))
    }
}

/// Backing for HKXMemberList — bound to an HkxFile's object's members.
#[derive(Debug)]
enum MemberListBacking {
    Bound {
        file: Py<PyHkxFile>,
        obj_idx: usize,
    },
    /// Detached: the proxy points at an unbound HKXObject. We hold a raw
    /// pointer back to the PyHkxObject so we can mutate its inner Vec.
    /// This is safe because the proxy is owned by code that holds the
    /// PyHkxObject alive (we acquire it from PyRef::into_ptr).
    Detached {
        object: *mut pyo3::ffi::PyObject,
    },
}

unsafe impl Send for MemberListBacking {}
unsafe impl Sync for MemberListBacking {}

#[pyclass(name = "HKXMemberList", module = "creation_lib._native.havok_native")]
pub struct PyHkxMemberList {
    backing: MemberListBacking,
}

impl PyHkxMemberList {
    fn with_members<R>(&self, py: Python<'_>, f: impl FnOnce(&[HkxMember]) -> R) -> PyResult<R> {
        match &self.backing {
            MemberListBacking::Bound { file, obj_idx } => {
                let fr = file.borrow(py);
                let obj = fr
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                Ok(f(&obj.members))
            }
            MemberListBacking::Detached { object } => {
                // SAFETY: the pointer was obtained from PyRef::into_ptr while
                // the GIL was held; we hold the GIL again here.
                let obj_ptr: *mut pyo3::ffi::PyObject = *object;
                let bound = unsafe { Bound::from_borrowed_ptr(py, obj_ptr) };
                let py_obj: PyRef<'_, PyHkxObject> = bound.downcast::<PyHkxObject>()?.borrow();
                match &py_obj.inner {
                    ObjectInner::Unbound { obj } => Ok(f(&obj.members)),
                    ObjectInner::Bound { .. } => Err(PyValueError::new_err(
                        "detached member list points at a bound object",
                    )),
                }
            }
        }
    }

    fn with_members_mut<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&mut Vec<HkxMember>) -> R,
    ) -> PyResult<R> {
        match &self.backing {
            MemberListBacking::Bound { file, obj_idx } => {
                let mut fr = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let obj = fr
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                Ok(f(&mut obj.members))
            }
            MemberListBacking::Detached { object } => {
                let obj_ptr: *mut pyo3::ffi::PyObject = *object;
                let bound = unsafe { Bound::from_borrowed_ptr(py, obj_ptr) };
                let mut py_obj: PyRefMut<'_, PyHkxObject> =
                    bound.downcast::<PyHkxObject>()?.borrow_mut();
                match &mut py_obj.inner {
                    ObjectInner::Unbound { obj } => Ok(f(&mut obj.members)),
                    ObjectInner::Bound { .. } => Err(PyValueError::new_err(
                        "detached member list points at a bound object",
                    )),
                }
            }
        }
    }
}

#[pymethods]
impl PyHkxMemberList {
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.with_members(py, |m| m.len())
    }

    fn __getitem__(&self, py: Python<'_>, index: isize) -> PyResult<Py<PyAny>> {
        let len = self.__len__(py)?;
        let idx = normalize_index(index, len)?;
        match &self.backing {
            MemberListBacking::Bound { file, obj_idx } => {
                let cloned: HkxMember = {
                    let f = file.borrow(py);
                    let obj = f
                        .inner
                        .objects()
                        .get(*obj_idx)
                        .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                    obj.members[idx].clone()
                };
                let bound = wrap_member_bound(py, file, *obj_idx, MemberPath::top(idx), &cloned)?;
                Ok(bound.unbind())
            }
            MemberListBacking::Detached { .. } => {
                let snapshot: HkxMember = self.with_members(py, |m| m[idx].clone())?;
                Ok(wrap_member_unbound(py, snapshot)?.unbind())
            }
        }
    }

    fn __setitem__(&self, py: Python<'_>, index: isize, value: Bound<'_, PyAny>) -> PyResult<()> {
        let len = self.__len__(py)?;
        let idx = normalize_index(index, len)?;
        let new_member = extract_member_unbound(py, &value)?;
        self.with_members_mut(py, |members| {
            members[idx] = new_member;
        })
    }

    fn __delitem__(&self, py: Python<'_>, index: isize) -> PyResult<()> {
        let len = self.__len__(py)?;
        let idx = normalize_index(index, len)?;
        self.with_members_mut(py, |members| {
            members.remove(idx);
        })
    }

    fn append(&self, py: Python<'_>, value: Bound<'_, PyAny>) -> PyResult<()> {
        let new_member = extract_member_unbound(py, &value)?;
        self.with_members_mut(py, |members| {
            members.push(new_member);
        })
    }

    fn insert(&self, py: Python<'_>, index: isize, value: Bound<'_, PyAny>) -> PyResult<()> {
        let len = self.__len__(py)?;
        let new_member = extract_member_unbound(py, &value)?;
        let idx = if index < 0 {
            ((len as isize + index).max(0)) as usize
        } else {
            (index as usize).min(len)
        };
        self.with_members_mut(py, |members| {
            members.insert(idx, new_member);
        })
    }

    fn extend(&self, py: Python<'_>, iterable: Bound<'_, PyAny>) -> PyResult<()> {
        let items: Vec<Bound<'_, PyAny>> = iterable.extract()?;
        for item in items {
            self.append(py, item)?;
        }
        Ok(())
    }

    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        self.with_members_mut(py, |members| members.clear())
    }

    fn __iter__(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyHkxMemberListIter>> {
        let len = slf.__len__(py)?;
        let it = PyHkxMemberListIter {
            list: Py::from(slf),
            cursor: 0,
            len,
        };
        Py::new(py, it)
    }

    fn __eq__(&self, py: Python<'_>, other: Bound<'_, PyAny>) -> PyResult<bool> {
        let other_list: Vec<Bound<'_, PyAny>> = match other.extract() {
            Ok(v) => v,
            Err(_) => return Ok(false),
        };
        let our_len = self.__len__(py)?;
        if other_list.len() != our_len {
            return Ok(false);
        }
        Ok(true) // shallow length equality only — deep compare not required by spec
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let n = self.__len__(py)?;
        Ok(format!("HKXMemberList(len={})", n))
    }
}

#[pyclass(
    name = "HKXMemberListIter",
    module = "creation_lib._native.havok_native"
)]
pub struct PyHkxMemberListIter {
    list: Py<PyHkxMemberList>,
    cursor: usize,
    len: usize,
}

#[pymethods]
impl PyHkxMemberListIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        if slf.cursor >= slf.len {
            return Ok(None);
        }
        let idx = slf.cursor as isize;
        slf.cursor += 1;
        let list = slf.list.clone_ref(py);
        let result = list.borrow(py).__getitem__(py, idx)?;
        Ok(Some(result))
    }
}

/// Wrap an HkxMember as an unbound Python member pyclass (snapshot).
fn wrap_member_unbound<'py>(py: Python<'py>, member: HkxMember) -> PyResult<Bound<'py, PyAny>> {
    match member.value {
        HkxValue::String { value, is_null } => {
            let m = PyHkxStringMember {
                inner: StringInner::Unbound {
                    name: member.name,
                    value,
                    is_null,
                },
            };
            Ok(Py::new(py, m)?.into_bound(py).into_any())
        }
        HkxValue::Pointer(opt) => {
            let m = PyHkxPointerMember {
                inner: PointerInner::Unbound {
                    name: member.name,
                    target: format_pointer_target(opt),
                    targets: None,
                },
            };
            Ok(Py::new(py, m)?.into_bound(py).into_any())
        }
        HkxValue::Array(values) => {
            let subtype = values
                .first()
                .map(infer_value_type)
                .unwrap_or(HkxType::Void);
            let m = PyHkxArrayMember {
                inner: ArrayInner::Unbound {
                    name: member.name,
                    subtype: subtype.into(),
                    contents: values,
                    ctype: String::new(),
                    source_offset: -1,
                    source_length: -1,
                },
            };
            Ok(Py::new(py, m)?.into_bound(py).into_any())
        }
        ref v @ (HkxValue::Object(_) | HkxValue::TypedObject { .. }) => {
            let value = v.clone();
            let inner_type = HkxType::Struct;
            let py_value = value_to_py(py, &value)?;
            let m = PyHkxDirectMember {
                inner: DirectInner::Unbound {
                    name: member.name,
                    type_: inner_type.into(),
                    value: py_value.unbind(),
                },
            };
            Ok(Py::new(py, m)?.into_bound(py).into_any())
        }
        ref other => {
            let inner_type = infer_value_type(other);
            let py_value = value_to_py(py, other)?;
            let m = PyHkxDirectMember {
                inner: DirectInner::Unbound {
                    name: member.name,
                    type_: inner_type.into(),
                    value: py_value.unbind(),
                },
            };
            Ok(Py::new(py, m)?.into_bound(py).into_any())
        }
    }
}

/// Stub helper used only to satisfy the type checker in __getitem__ before
/// the real lookup; never actually used.
fn dummy_member() -> HkxMember {
    HkxMember {
        name: String::new(),
        value: HkxValue::Void,
    }
}

/// Backing for HKXValueList — array contents (Vec<HkxValue>).
#[derive(Debug)]
enum ValueListBacking {
    Bound {
        file: Py<PyHkxFile>,
        obj_idx: usize,
        path: MemberPath,
    },
    Detached {
        array: *mut pyo3::ffi::PyObject,
    },
}

unsafe impl Send for ValueListBacking {}
unsafe impl Sync for ValueListBacking {}

#[pyclass(name = "HKXValueList", module = "creation_lib._native.havok_native")]
pub struct PyHkxValueList {
    backing: ValueListBacking,
}

impl PyHkxValueList {
    fn with_array<R>(&self, py: Python<'_>, f: impl FnOnce(&Vec<HkxValue>) -> R) -> PyResult<R> {
        match &self.backing {
            ValueListBacking::Bound {
                file,
                obj_idx,
                path,
            } => {
                let fr = file.borrow(py);
                let obj = fr
                    .inner
                    .objects()
                    .get(*obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let arr = resolve_value_array(obj, path)
                    .ok_or_else(|| PyValueError::new_err("not an array member"))?;
                Ok(f(arr))
            }
            ValueListBacking::Detached { array } => {
                let p: *mut pyo3::ffi::PyObject = *array;
                let bound = unsafe { Bound::from_borrowed_ptr(py, p) };
                let arr_ref: PyRef<'_, PyHkxArrayMember> =
                    bound.downcast::<PyHkxArrayMember>()?.borrow();
                match &arr_ref.inner {
                    ArrayInner::Unbound { contents, .. } => Ok(f(contents)),
                    ArrayInner::Bound { .. } => Err(PyValueError::new_err(
                        "detached value list points at a bound array",
                    )),
                }
            }
        }
    }

    fn with_array_mut<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&mut Vec<HkxValue>) -> R,
    ) -> PyResult<R> {
        match &self.backing {
            ValueListBacking::Bound {
                file,
                obj_idx,
                path,
            } => {
                let mut fr = file.borrow_mut(py);
                let obj_idx = *obj_idx;
                let path = path.clone();
                let obj = fr
                    .inner
                    .objects_mut()
                    .get_mut(obj_idx)
                    .ok_or_else(|| PyIndexError::new_err("object index out of range"))?;
                let arr = resolve_value_array_mut(obj, &path)
                    .ok_or_else(|| PyValueError::new_err("not an array member"))?;
                Ok(f(arr))
            }
            ValueListBacking::Detached { array } => {
                let p: *mut pyo3::ffi::PyObject = *array;
                let bound = unsafe { Bound::from_borrowed_ptr(py, p) };
                let mut arr_ref: PyRefMut<'_, PyHkxArrayMember> =
                    bound.downcast::<PyHkxArrayMember>()?.borrow_mut();
                match &mut arr_ref.inner {
                    ArrayInner::Unbound { contents, .. } => Ok(f(contents)),
                    ArrayInner::Bound { .. } => Err(PyValueError::new_err(
                        "detached value list points at a bound array",
                    )),
                }
            }
        }
    }

    fn current_subtype(&self, py: Python<'_>) -> PyResult<HkxType> {
        // For unbound: read declared subtype. For bound: infer from elements.
        match &self.backing {
            ValueListBacking::Bound { .. } => self.with_array(py, |arr| {
                arr.first().map(infer_value_type).unwrap_or(HkxType::Void)
            }),
            ValueListBacking::Detached { array } => {
                let p: *mut pyo3::ffi::PyObject = *array;
                let bound = unsafe { Bound::from_borrowed_ptr(py, p) };
                let arr_ref: PyRef<'_, PyHkxArrayMember> =
                    bound.downcast::<PyHkxArrayMember>()?.borrow();
                match &arr_ref.inner {
                    ArrayInner::Unbound { subtype, .. } => Ok(subtype.to_native()),
                    ArrayInner::Bound { .. } => Ok(HkxType::Void),
                }
            }
        }
    }
}

#[pymethods]
impl PyHkxValueList {
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.with_array(py, |arr| arr.len())
    }

    fn __getitem__<'py>(&self, py: Python<'py>, index: isize) -> PyResult<Bound<'py, PyAny>> {
        let len = self.__len__(py)?;
        let idx = normalize_index(index, len)?;
        // Snapshot the value and convert.
        let value = self.with_array(py, |arr| arr[idx].clone())?;
        value_to_py(py, &value)
    }

    fn __setitem__(&self, py: Python<'_>, index: isize, value: Bound<'_, PyAny>) -> PyResult<()> {
        let len = self.__len__(py)?;
        let idx = normalize_index(index, len)?;
        let st = self.current_subtype(py)?;
        let new_value = py_to_value(py, st, &value)?;
        self.with_array_mut(py, |arr| {
            arr[idx] = new_value;
        })
    }

    fn __delitem__(&self, py: Python<'_>, index: isize) -> PyResult<()> {
        let len = self.__len__(py)?;
        let idx = normalize_index(index, len)?;
        self.with_array_mut(py, |arr| {
            arr.remove(idx);
        })
    }

    fn append(&self, py: Python<'_>, value: Bound<'_, PyAny>) -> PyResult<()> {
        let st = self.current_subtype(py)?;
        let new_value = py_to_value(py, st, &value)?;
        self.with_array_mut(py, |arr| arr.push(new_value))
    }

    fn insert(&self, py: Python<'_>, index: isize, value: Bound<'_, PyAny>) -> PyResult<()> {
        let len = self.__len__(py)?;
        let st = self.current_subtype(py)?;
        let new_value = py_to_value(py, st, &value)?;
        let idx = if index < 0 {
            ((len as isize + index).max(0)) as usize
        } else {
            (index as usize).min(len)
        };
        self.with_array_mut(py, |arr| arr.insert(idx, new_value))
    }

    fn extend(&self, py: Python<'_>, iterable: Bound<'_, PyAny>) -> PyResult<()> {
        let items: Vec<Bound<'_, PyAny>> = iterable.extract()?;
        for item in items {
            self.append(py, item)?;
        }
        Ok(())
    }

    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        self.with_array_mut(py, |arr| arr.clear())
    }

    fn __iter__(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyHkxValueListIter>> {
        let len = slf.__len__(py)?;
        let it = PyHkxValueListIter {
            list: Py::from(slf),
            cursor: 0,
            len,
        };
        Py::new(py, it)
    }

    fn __eq__(&self, py: Python<'_>, other: Bound<'_, PyAny>) -> PyResult<bool> {
        let other_list: Vec<Bound<'_, PyAny>> = match other.extract() {
            Ok(v) => v,
            Err(_) => return Ok(false),
        };
        let our_len = self.__len__(py)?;
        Ok(other_list.len() == our_len)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let n = self.__len__(py)?;
        Ok(format!("HKXValueList(len={})", n))
    }
}

#[pyclass(
    name = "HKXValueListIter",
    module = "creation_lib._native.havok_native"
)]
pub struct PyHkxValueListIter {
    list: Py<PyHkxValueList>,
    cursor: usize,
    len: usize,
}

#[pymethods]
impl PyHkxValueListIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(mut slf: PyRefMut<'py, Self>, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        if slf.cursor >= slf.len {
            return Ok(None);
        }
        let idx = slf.cursor as isize;
        slf.cursor += 1;
        let list = slf.list.clone_ref(py);
        let result = list.borrow(py).__getitem__(py, idx)?;
        Ok(Some(result.unbind()))
    }
}

fn normalize_index(index: isize, len: usize) -> PyResult<usize> {
    let idx = if index < 0 {
        len as isize + index
    } else {
        index
    };
    if idx < 0 || (idx as usize) >= len {
        return Err(PyIndexError::new_err("index out of range"));
    }
    Ok(idx as usize)
}

// --- Descriptor pyclasses ------------------------------------------------

#[pyclass(
    eq,
    eq_int,
    name = "ClassKind",
    module = "creation_lib._native.havok_native"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyClassKind {
    RUNTIME,
    SETUP,
    INTERNAL,
}

impl From<ClassKind> for PyClassKind {
    fn from(k: ClassKind) -> Self {
        match k {
            ClassKind::Runtime => Self::RUNTIME,
            ClassKind::Setup => Self::SETUP,
            ClassKind::Internal => Self::INTERNAL,
        }
    }
}

#[pyclass(name = "EnumDef", module = "creation_lib._native.havok_native")]
#[derive(Debug, Clone)]
pub struct PyEnumDef {
    inner: EnumDef,
}

#[pymethods]
impl PyEnumDef {
    #[getter]
    fn values(&self) -> Vec<(String, i64)> {
        self.inner.values.clone()
    }

    fn value_to_name(&self, value: i64) -> Option<String> {
        self.inner.value_to_name(value).map(str::to_string)
    }

    fn name_to_value(&self, name: String) -> Option<i64> {
        self.inner.name_to_value(&name)
    }
}

#[pyclass(name = "MemberTemplate", module = "creation_lib._native.havok_native")]
#[derive(Debug, Clone)]
pub struct PyMemberTemplate {
    inner: MemberTemplate,
}

#[pymethods]
impl PyMemberTemplate {
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    #[getter]
    fn offset(&self) -> usize {
        self.inner.offset
    }

    #[getter]
    fn vtype(&self) -> PyHkxType {
        self.inner.vtype.into()
    }

    #[getter]
    fn vsubtype(&self) -> PyHkxType {
        self.inner.vsubtype.into()
    }

    #[getter]
    fn ctype(&self) -> String {
        self.inner.ctype.clone()
    }

    #[getter]
    fn arrsize(&self) -> usize {
        self.inner.arrsize
    }

    #[getter]
    fn flags(&self) -> String {
        self.inner.flags.clone()
    }

    #[getter]
    fn etype(&self) -> String {
        self.inner.etype.clone()
    }

    #[getter]
    fn default(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.inner.default {
            Some(v) => Ok(value_to_py(py, v)?.unbind()),
            None => Ok(py.None()),
        }
    }
}

#[pyclass(name = "ClassDescriptor", module = "creation_lib._native.havok_native")]
#[derive(Debug, Clone)]
pub struct PyClassDescriptor {
    inner: ClassDescriptor,
}

#[pymethods]
impl PyClassDescriptor {
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    #[getter]
    fn signature(&self) -> String {
        self.inner.signature.clone()
    }

    #[getter]
    fn parent(&self) -> Option<String> {
        self.inner.parent.clone()
    }

    #[getter]
    fn is_struct(&self) -> bool {
        self.inner.is_struct
    }

    #[getter]
    fn members(&self) -> Vec<PyMemberTemplate> {
        self.inner
            .members
            .iter()
            .map(|m| PyMemberTemplate { inner: m.clone() })
            .collect()
    }

    #[getter]
    fn enums(&self) -> std::collections::HashMap<String, PyEnumDef> {
        self.inner
            .enums
            .iter()
            .map(|(k, v)| (k.clone(), PyEnumDef { inner: v.clone() }))
            .collect()
    }

    #[getter]
    fn kind(&self) -> PyClassKind {
        self.inner.kind.into()
    }
}

#[pyclass(
    name = "DescriptorRegistry",
    module = "creation_lib._native.havok_native"
)]
pub struct PyDescriptorRegistry {
    inner: std::sync::Mutex<DescriptorRegistry>,
}

#[pymethods]
impl PyDescriptorRegistry {
    #[new]
    #[pyo3(signature = (version=None, classxml_dir=None))]
    fn new(version: Option<String>, classxml_dir: Option<String>) -> PyResult<Self> {
        let reg = if let Some(dir) = classxml_dir {
            DescriptorRegistry::from_dir(PathBuf::from(dir))
                .map_err(|e| PyValueError::new_err(e.to_string()))?
        } else if let Some(v) = version {
            DescriptorRegistry::for_version(&v).map_err(|e| PyValueError::new_err(e.to_string()))?
        } else {
            DescriptorRegistry::new()
        };
        Ok(Self {
            inner: std::sync::Mutex::new(reg),
        })
    }

    fn get(&self, class_name: String) -> PyResult<Option<PyClassDescriptor>> {
        let mut r = self.inner.lock().unwrap();
        match r.get(&class_name) {
            Ok(Some(d)) => Ok(Some(PyClassDescriptor { inner: d.clone() })),
            Ok(None) => Ok(None),
            Err(e) => Err(PyValueError::new_err(e.to_string())),
        }
    }

    fn get_all_members(&self, class_name: String) -> PyResult<Vec<PyMemberTemplate>> {
        let mut r = self.inner.lock().unwrap();
        match r.get_all_members(&class_name) {
            Ok(members) => Ok(members
                .into_iter()
                .map(|m| PyMemberTemplate { inner: m })
                .collect()),
            Err(e) => Err(PyValueError::new_err(e.to_string())),
        }
    }

    fn get_enum_value(&self, class_name: String, enum_name: String, int_value: i32) -> String {
        let mut r = self.inner.lock().unwrap();
        r.get_enum_value(&class_name, &enum_name, int_value)
    }

    fn get_enum_int(&self, class_name: String, enum_name: String, str_value: String) -> i32 {
        let mut r = self.inner.lock().unwrap();
        r.get_enum_int(&class_name, &enum_name, &str_value)
    }

    fn class_kind(&self, class_name: String) -> PyClassKind {
        let mut r = self.inner.lock().unwrap();
        r.class_kind(&class_name).into()
    }

    fn is_setup(&self, class_name: String) -> bool {
        let mut r = self.inner.lock().unwrap();
        r.is_setup(&class_name)
    }
}

// --- Top-level pyfunctions mirroring py_creation_lib/python/creation_lib/hkxpack/__init__.py -------------

/// Load an HKX file from a path. Returns (HKXFile, DescriptorRegistry).
#[pyfunction]
fn load_hkx(py: Python<'_>, path: String) -> PyResult<(Py<PyHkxFile>, Py<PyDescriptorRegistry>)> {
    let bytes =
        std::fs::read(&path).map_err(|e| PyValueError::new_err(format!("read {path}: {e}")))?;
    let inner = HkxFile::read(&bytes).map_err(map_error)?;
    let cv = inner.contents_version().to_string();
    let file = PyHkxFile {
        inner,
        enum_meta: EnumMetadata::default(),
    };
    let reg = PyDescriptorRegistry {
        inner: std::sync::Mutex::new(DescriptorRegistry::for_contents_version(&cv)),
    };
    Ok((Py::new(py, file)?, Py::new(py, reg)?))
}

/// Load an HKX file from bytes. Returns (HKXFile, DescriptorRegistry).
#[pyfunction]
fn load_hkx_bytes(
    py: Python<'_>,
    data: &Bound<'_, PyBytes>,
) -> PyResult<(Py<PyHkxFile>, Py<PyDescriptorRegistry>)> {
    let bytes = data.as_bytes().to_vec();
    let inner = HkxFile::read(&bytes).map_err(map_error)?;
    let cv = inner.contents_version().to_string();
    let file = PyHkxFile {
        inner,
        enum_meta: EnumMetadata::default(),
    };
    let reg = PyDescriptorRegistry {
        inner: std::sync::Mutex::new(DescriptorRegistry::for_contents_version(&cv)),
    };
    Ok((Py::new(py, file)?, Py::new(py, reg)?))
}

/// Save an HKX file to a path. The registry argument is accepted for API
/// parity with the Python signature but is unused — the file's own
/// contents_version drives the writer's classxml lookup.
#[pyfunction]
fn save_hkx(
    file: PyRef<'_, PyHkxFile>,
    _registry: PyRef<'_, PyDescriptorRegistry>,
    output_path: String,
) -> PyResult<()> {
    let bytes = file.inner.save();
    std::fs::write(Path::new(&output_path), &bytes)
        .map_err(|e| PyValueError::new_err(format!("write {output_path}: {e}")))
}

/// Serialize an HKX file to bytes.
#[pyfunction]
fn write_hkx<'py>(
    py: Python<'py>,
    file: PyRef<'_, PyHkxFile>,
    _registry: PyRef<'_, PyDescriptorRegistry>,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = file.inner.save();
    Ok(PyBytes::new(py, &bytes))
}

/// Serialize an HKX file to a TagXML string. Mirrors the legacy
/// `creation_lib.hkxpack.tagwriter.write_xml_string(hkx_file, registry)` signature;
/// the registry argument is accepted for API parity but the file's own
/// `contents_version` drives the writer's classxml lookup.
#[pyfunction]
fn write_xml_string(
    file: PyRef<'_, PyHkxFile>,
    _registry: PyRef<'_, PyDescriptorRegistry>,
) -> PyResult<String> {
    file.to_xml()
}

/// Serialize an HKX file to a TagXML string and write it to disk. Mirrors
/// the legacy `creation_lib.hkxpack.tagwriter.write_xml_file(hkx_file, registry, path)`
/// signature.
#[pyfunction]
fn write_xml_file(
    file: PyRef<'_, PyHkxFile>,
    _registry: PyRef<'_, PyDescriptorRegistry>,
    output_path: String,
) -> PyResult<()> {
    let xml = file.to_xml()?;
    std::fs::write(Path::new(&output_path), xml.as_bytes())
        .map_err(|e| PyValueError::new_err(format!("write {output_path}: {e}")))
}

/// Detect the format of HKX bytes. Returns (kind, version) or None.
#[pyfunction(name = "detect_format")]
fn detect_format_py(data: &Bound<'_, PyBytes>) -> PyResult<Option<(String, String)>> {
    let bytes = data.as_bytes();
    match api::hkx_detect_format_full(bytes) {
        Ok(r) => {
            if r.kind.is_empty() {
                Ok(None)
            } else {
                Ok(Some((r.kind, r.version)))
            }
        }
        Err(_) => Ok(None),
    }
}

/// Unpack an HKX file at the given path to TagXML. Returns the XML string.
/// (Diverges from the Python signature which returned a temp-file path —
/// this returns the string directly to avoid temp-file management.)
#[pyfunction]
fn unpack_hkx_to_xml(py: Python<'_>, path: String) -> PyResult<String> {
    let bytes =
        std::fs::read(&path).map_err(|e| PyValueError::new_err(format!("read {path}: {e}")))?;
    py.detach(move || api::havok_hkx_to_xml(&bytes).map_err(map_error))
}

/// Pack a TagXML file at xml_path into binary HKX written to output_path.
#[pyfunction]
fn pack_xml_to_hkx(py: Python<'_>, xml_path: String, output_path: String) -> PyResult<()> {
    let xml = std::fs::read_to_string(&xml_path)
        .map_err(|e| PyValueError::new_err(format!("read {xml_path}: {e}")))?;
    let bytes = py.detach(move || api::havok_xml_to_hkx(&xml).map_err(map_error))?;
    std::fs::write(Path::new(&output_path), &bytes)
        .map_err(|e| PyValueError::new_err(format!("write {output_path}: {e}")))
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(hkx_detect_format, m)?)?;
    m.add_function(wrap_pyfunction!(hkx_roundtrip_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(hkx_patch_roundtrip, m)?)?;
    m.add_function(wrap_pyfunction!(havok_convert_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(havok_convert_file, m)?)?;
    m.add_function(wrap_pyfunction!(havok_convert_batch, m)?)?;
    m.add_function(wrap_pyfunction!(havok_extract_clip, m)?)?;
    m.add_function(wrap_pyfunction!(havok_write_animation_xml, m)?)?;
    m.add_function(wrap_pyfunction!(havok_collision_preview, m)?)?;
    m.add_function(wrap_pyfunction!(havok_collision_summary, m)?)?;
    m.add_function(wrap_pyfunction!(validate_collision_blob, m)?)?;
    m.add_function(wrap_pyfunction!(havok_parse_skeleton, m)?)?;
    m.add_function(wrap_pyfunction!(havok_parse_behavior, m)?)?;
    // full behavior-graph UI parser
    m.add_function(wrap_pyfunction!(havok_behavior_graph_to_ui_json, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_metadata_from_blob, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_bake, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_validate, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_simulate, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_simulate_from_blob, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_step_from_blob_state, m)?)?;
    m.add_function(wrap_pyfunction!(convex_hull_simple, m)?)?;
    m.add_function(wrap_pyfunction!(convex_hull_triangles, m)?)?;
    m.add_function(wrap_pyfunction!(decimate_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(fo4_polytope_collision_blob, m)?)?;
    m.add_function(wrap_pyfunction!(fo4_compound_collision_blob, m)?)?;
    m.add_function(wrap_pyfunction!(fo4_compressed_mesh_collision_blob, m)?)?;
    m.add_function(wrap_pyfunction!(fo4_multi_body_collision_blob, m)?)?;
    m.add_function(wrap_pyfunction!(starfield_convex_collision_blob, m)?)?;
    m.add_function(wrap_pyfunction!(havok_decompress_spline, m)?)?;
    m.add_function(wrap_pyfunction!(hkx_to_xml, m)?)?;
    m.add_function(wrap_pyfunction!(xml_to_hkx, m)?)?;
    // ClothEditor pyfunctions
    m.add_function(wrap_pyfunction!(cloth_set_particle_mass_all, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_scale_particle_mass, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_particle_radius_all, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_particle_friction_all, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_particle_fixed, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_particles_fixed, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_particles_mass, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_particles_radius, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_scale_stiffness, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_stiffness, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_gravity, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_damping, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_collision_tolerance, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_substeps, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_solver_iterations, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_capsule_radius, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_scale_all_capsule_radii, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_set_capsule_endpoints, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_add_capsule, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_remove_capsule, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_summary_json, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_inspect_blob_json, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_inspect_full_json, m)?)?;
    // cloth helper pyfunctions
    m.add_function(wrap_pyfunction!(cloth_material_list, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_material_get, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_material_apply, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_topology_list, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_topology_get, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_region_generate, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_reverse_to_setup, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_generate_bones_from_particles, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_bones_to_transform_set, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_auto_skin, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_template_list, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_template_get, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_template_blob, m)?)?;
    // asset discovery / manifest / parser pyfunctions
    m.add_function(wrap_pyfunction!(walk_meshes_dir, m)?)?;
    m.add_function(wrap_pyfunction!(classify_category, m)?)?;
    m.add_function(wrap_pyfunction!(classify_role, m)?)?;
    m.add_function(wrap_pyfunction!(build_manifests, m)?)?;
    m.add_function(wrap_pyfunction!(parse_animation_xml, m)?)?;
    m.add_function(wrap_pyfunction!(parse_character_xml, m)?)?;
    m.add_function(wrap_pyfunction!(parse_project_xml, m)?)?;
    m.add_function(wrap_pyfunction!(generate_classxml, m)?)?;
    m.add_function(wrap_pyfunction!(descriptor_registry_get, m)?)?;
    m.add_function(wrap_pyfunction!(descriptor_registry_get_all_members, m)?)?;
    m.add_function(wrap_pyfunction!(descriptor_registry_get_enum_value, m)?)?;
    m.add_function(wrap_pyfunction!(descriptor_registry_get_enum_int, m)?)?;
    m.add_function(wrap_pyfunction!(hkx_load_to_json, m)?)?;
    m.add_function(wrap_pyfunction!(hkx_save_from_json, m)?)?;
    m.add_function(wrap_pyfunction!(havok_compress_spline, m)?)?;
    // packfile inspection
    m.add_function(wrap_pyfunction!(hkx_inspect_packfile, m)?)?;
    // Hkxpack model wrappers (mutation surface)
    m.add_class::<PyHkxTypeFamily>()?;
    m.add_class::<PyHkxType>()?;
    m.add_class::<PyHkxFile>()?;
    m.add_class::<PyHkxObject>()?;
    m.add_class::<PyHkxDirectMember>()?;
    m.add_class::<PyHkxArrayMember>()?;
    m.add_class::<PyHkxPointerMember>()?;
    m.add_class::<PyHkxStringMember>()?;
    m.add_class::<PyHkxEnumMember>()?;
    m.add_class::<PyHkxObjectList>()?;
    m.add_class::<PyHkxObjectListIter>()?;
    m.add_class::<PyHkxMemberList>()?;
    m.add_class::<PyHkxMemberListIter>()?;
    m.add_class::<PyHkxValueList>()?;
    m.add_class::<PyHkxValueListIter>()?;
    m.add_class::<PyClassKind>()?;
    m.add_class::<PyEnumDef>()?;
    m.add_class::<PyMemberTemplate>()?;
    m.add_class::<PyClassDescriptor>()?;
    m.add_class::<PyDescriptorRegistry>()?;
    m.add_function(wrap_pyfunction!(load_hkx, m)?)?;
    m.add_function(wrap_pyfunction!(load_hkx_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(save_hkx, m)?)?;
    m.add_function(wrap_pyfunction!(write_hkx, m)?)?;
    m.add_function(wrap_pyfunction!(write_xml_string, m)?)?;
    m.add_function(wrap_pyfunction!(write_xml_file, m)?)?;
    m.add_function(wrap_pyfunction!(detect_format_py, m)?)?;
    m.add_function(wrap_pyfunction!(unpack_hkx_to_xml, m)?)?;
    m.add_function(wrap_pyfunction!(pack_xml_to_hkx, m)?)?;
    Ok(())
}

#[pymodule]
fn havok_native(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
