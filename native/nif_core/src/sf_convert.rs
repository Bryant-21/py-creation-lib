//! Starfield `BSGeometry` + external `.mesh` -> FO4 `BSTriShape` conversion,
//! with hybrid (decoded / render-derived fallback) collision.
//!
//! Starfield uses metres; FO4 uses 69.99125 units per metre. Static shapes bake
//! world transforms, while weapon parts keep scaled local transforms and skin
//! conversion consumes raw local vertices through the source inverse binds.
//! Collision receives world-space FO4 vertices and converts them back to metres;
//! decoded collision is already in metres.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use crate::model::{NifBlock, NifFile, NifValue};
use crate::sf_mesh::{SfMeshData, read_sf_mesh, resolve_geometry_path};
use havok_native::collision::direct_starfield::decode_starfield_collision;
use havok_native::collision::multi_body::{BodyMeta, BodyMotionType};
use havok_native::collision::{BuildOptions, MultiBodyShape, build_fo4_multi_body_collision};

const HAVOK_SCALE: f32 = 69.99125;
const FO4_STATIC_LAYER: u8 = 1;
const DEFAULT_CONVEX_RADIUS: f32 = 0.01;
const FALLBACK_COLLISION_TRIANGLE_BUDGET: usize = 16_000;
const BSX_HAVOK_FLAG: u64 = 0x02;
const VF_VERTEX: i64 = 0x0001;
const VF_UVS: i64 = 0x0002;
const VF_NORMALS: i64 = 0x0008;
const VF_TANGENTS: i64 = 0x0010;
const VF_VERTEX_COLORS: i64 = 0x0020;
const FO4_DEFAULT_SHADER_FLAGS_1: u64 = 0x8040_0201;

pub struct SfConvertOptions {
    pub geometries_root: PathBuf,
    pub material_path_rewriter: Option<Box<dyn Fn(&str) -> String + Send + Sync>>,
    pub target_skeleton: Option<PathBuf>,
    pub translation_maps_dir: Option<PathBuf>,
    pub weapon_role: Option<String>,
    pub cloth_companions: Vec<PathBuf>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SfConvertReport {
    pub shapes: u32,
    pub collision_decoded: u32,
    pub collision_fallback: u32,
    pub collision_none: u32,
    pub skinned_shapes: u32,
    pub reduced_skin_vertices: u32,
    pub max_discarded_skin_weight: f32,
    pub cloth_blobs: u32,
    pub custom_cloth_bones: u32,
    pub unbound_cloth_bones: u32,
}

/// Walk up from `nif_path`'s directory looking for the extracted
/// `geometries/` tree. Starfield's generated LOD and animated-kit NIFs can be
/// nested more than eight directories below the extraction root.
pub(crate) fn default_geometries_root(nif_path: &Path) -> PathBuf {
    let Some(start) = nif_path.parent() else {
        return PathBuf::from("geometries");
    };
    let mut dir = start.to_path_buf();
    for _ in 0..32 {
        let candidate = dir.join("geometries");
        if candidate.is_dir() {
            return candidate;
        }
        let Some(parent) = dir.parent() else { break };
        if parent == dir {
            break;
        }
        dir = parent.to_path_buf();
    }
    start.join("geometries")
}

pub fn convert_starfield_nif(
    source: &Path,
    dest: &Path,
    opts: &SfConvertOptions,
) -> Result<SfConvertReport, String> {
    let src_nif = NifFile::load(source.to_path_buf())
        .map_err(|error| format!("sf convert: failed to read {}: {error}", source.display()))?;

    let root_id = src_nif
        .header
        .footer_roots
        .first()
        .map(|id| *id as usize)
        .unwrap_or(0);

    let mut geometries: Vec<(Xform, usize)> = Vec::new();
    collect_geometries(
        &src_nif,
        root_id,
        identity_xform(),
        &mut geometries,
        &mut HashSet::new(),
    );
    let requires_complete_geometry = opts.weapon_role.is_some()
        || geometries.iter().any(|(_, id)| {
            matches!(src_nif.blocks[*id].get_field("Skin"), Some(NifValue::Ref(id)) if *id >= 0)
        });

    let mut dest_nif = NifFile::new("fo4");
    dest_nif.header.footer_roots = vec![0];
    let dest_root_id = 0usize;
    let preserved_scene = if opts.weapon_role.is_some() {
        Some(crate::sf_scene::preserve_weapon_scene(
            &src_nif,
            &mut dest_nif,
            root_id,
        )?)
    } else {
        None
    };

    let mut report = SfConvertReport::default();
    let mut mesh_failures = Vec::new();
    let mut fallback_vertices: Vec<[f32; 3]> = Vec::new();
    let mut fallback_triangles: Vec<[u32; 3]> = Vec::new();
    let mut skin_context = None;

    for (world, block_id) in &geometries {
        let Some(block) = src_nif.get_block(*block_id) else {
            continue;
        };
        let Some(mesh_id) = external_mesh_path(block) else {
            continue;
        };
        let resolved = resolve_geometry_path(&opts.geometries_root, &mesh_id);
        let mesh = match read_sf_mesh(&resolved) {
            Ok(mesh) => mesh,
            Err(error) => {
                if mesh_failures.len() < 3 {
                    mesh_failures.push(format!("{}: {error}", resolved.display()));
                }
                continue;
            }
        };
        if mesh.positions.is_empty() || mesh.triangles.is_empty() {
            continue;
        }

        let has_skin = mesh.bone_influences.is_some()
            || matches!(block.get_field("Skin"), Some(NifValue::Ref(id)) if *id >= 0);
        let skinned = if has_skin {
            if skin_context.is_none() {
                let skeleton = opts
                    .target_skeleton
                    .as_deref()
                    .ok_or("Starfield skinned geometry requires an explicit target skeleton")?;
                let maps = opts
                    .translation_maps_dir
                    .as_deref()
                    .ok_or("Starfield skinned geometry requires skeleton translation maps")?;
                skin_context = Some(crate::sf_skin::SkinContext::load(
                    &mut dest_nif,
                    skeleton,
                    maps,
                    &src_nif,
                    &opts.cloth_companions,
                )?);
                let context = skin_context.as_ref().unwrap();
                report.cloth_blobs = context.cloth_blobs;
                report.custom_cloth_bones = context.custom_cloth_bones;
                report.unbound_cloth_bones = context.unbound_cloth_bones;
            }
            let skin =
                skin_context
                    .as_ref()
                    .unwrap()
                    .convert(&src_nif, block, &mesh, &mut dest_nif)?;
            report.skinned_shapes += 1;
            report.reduced_skin_vertices += skin.reduced_vertices;
            report.max_discarded_skin_weight = report
                .max_discarded_skin_weight
                .max(skin.max_discarded_weight);
            Some(skin)
        } else {
            None
        };
        let shape_world = if preserved_scene.is_some() && !has_skin {
            identity_xform()
        } else {
            *world
        };
        let shape_id = build_bstrishape(
            &mut dest_nif,
            block,
            &mesh,
            &shape_world,
            &src_nif,
            opts,
            skinned.as_ref(),
        );
        let parent = if !has_skin {
            if let Some(scene) = &preserved_scene {
                crate::sf_scene::copy_shape_transform(block, &mut dest_nif.blocks[shape_id])?;
                *scene
                    .get(block_id)
                    .ok_or("weapon geometry has no preserved parent")?
            } else {
                dest_root_id
            }
        } else {
            dest_root_id
        };
        attach_child(&mut dest_nif, parent, shape_id);
        report.shapes += 1;

        if has_skin {
            continue;
        }
        let base = fallback_vertices.len() as u32;
        for position in &mesh.positions {
            let mut point = apply_xform(world, *position);
            if preserved_scene.is_some() {
                point =
                    inverse_apply_xform(&read_local_transform(&src_nif.blocks[root_id]), point)?;
            }
            fallback_vertices.push(point.map(|c| c * HAVOK_SCALE));
        }
        for triangle in &mesh.triangles {
            fallback_triangles.push([base + triangle[0], base + triangle[1], base + triangle[2]]);
        }
    }

    if report.shapes == 0 && !(preserved_scene.is_some() && geometries.is_empty()) {
        let detail = if geometries.is_empty() {
            "no BSGeometry blocks found".to_string()
        } else if mesh_failures.is_empty() {
            format!(
                "{} BSGeometry block(s) had no usable external mesh reference",
                geometries.len()
            )
        } else {
            format!(
                "{} BSGeometry block(s) failed external mesh reads: {}",
                geometries.len(),
                mesh_failures.join("; ")
            )
        };
        return Err(format!("sf convert: no convertible geometry: {detail}"));
    }
    if requires_complete_geometry && report.shapes as usize != geometries.len() {
        return Err(format!(
            "sf convert: incomplete Starfield weapon/outfit geometry: converted {} of {} shapes; {}",
            report.shapes,
            geometries.len(),
            mesh_failures.join("; ")
        ));
    }
    if report.cloth_blobs as usize != src_nif.find_blocks("BSClothExtraData").len() {
        return Err(
            "sf convert: Starfield cloth requires a supported target skin and simulation binding"
                .into(),
        );
    }

    if report.skinned_shapes == 0 {
        install_collision(
            &src_nif,
            &mut dest_nif,
            dest_root_id,
            &fallback_vertices,
            &fallback_triangles,
            &mut report,
            opts.weapon_role.is_some(),
        )?;
    } else {
        report.collision_none = 1;
    }

    dest_nif
        .save(Some(dest.to_path_buf()))
        .map_err(|error| format!("sf convert: failed to write {}: {error}", dest.display()))?;

    Ok(report)
}

#[cfg(test)]
mod root_tests {
    use super::*;

    #[test]
    fn geometries_root_resolves_for_deep_generated_lod_path() {
        let temp = tempfile::tempdir().unwrap();
        let geometries = temp.path().join("geometries");
        std::fs::create_dir_all(&geometries).unwrap();
        let nif = temp
            .path()
            .join("Meshes/a/b/c/d/e/f/g/h/i/j/generated_lod.nif");

        assert_eq!(default_geometries_root(&nif), geometries);
    }

    #[test]
    fn weapon_fallback_points_are_relative_to_preserved_root() {
        let root = Xform {
            translation: [4.0, 5.0, 6.0],
            rotation: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 2.0,
        };
        let child = Xform {
            translation: [1.0, 2.0, 3.0],
            ..identity_xform()
        };
        let position = [0.5, 0.7, 0.9];
        let world_point = apply_xform(&combine(&root, &child), position);
        let collision_point = inverse_apply_xform(&root, world_point).unwrap();
        let expected = apply_xform(&child, position);
        for axis in 0..3 {
            assert!((collision_point[axis] - expected[axis]).abs() < 1e-6);
        }
        assert_ne!(collision_point, world_point);
        let reapplied = apply_xform(&root, collision_point);
        for axis in 0..3 {
            assert!((reapplied[axis] - world_point[axis]).abs() < 1e-6);
        }
    }
}

// ---------------------------------------------------------------------------
// Scene-graph transform composition (raw Starfield metres, no unit scale).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Xform {
    translation: [f32; 3],
    rotation: [[f32; 3]; 3],
    scale: f32,
}

fn identity_xform() -> Xform {
    Xform {
        translation: [0.0; 3],
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        scale: 1.0,
    }
}

fn matmul3(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for row in 0..3 {
        for col in 0..3 {
            out[row][col] = a[row][0] * b[0][col] + a[row][1] * b[1][col] + a[row][2] * b[2][col];
        }
    }
    out
}

fn rotate_only(rotation: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        v[0] * rotation[0][0] + v[1] * rotation[1][0] + v[2] * rotation[2][0],
        v[0] * rotation[0][1] + v[1] * rotation[1][1] + v[2] * rotation[2][1],
        v[0] * rotation[0][2] + v[1] * rotation[1][2] + v[2] * rotation[2][2],
    ]
}

fn apply_xform(xform: &Xform, local: [f32; 3]) -> [f32; 3] {
    let rotated = rotate_only(&xform.rotation, local);
    [
        rotated[0] * xform.scale + xform.translation[0],
        rotated[1] * xform.scale + xform.translation[1],
        rotated[2] * xform.scale + xform.translation[2],
    ]
}

fn inverse_apply_xform(xform: &Xform, point: [f32; 3]) -> Result<[f32; 3], String> {
    if !xform.scale.is_finite() || xform.scale.abs() < 1e-12 {
        return Err("Starfield weapon root has a singular scale".into());
    }
    let relative =
        std::array::from_fn(|axis| (point[axis] - xform.translation[axis]) / xform.scale);
    let inverse_rotation =
        std::array::from_fn(|row| std::array::from_fn(|col| xform.rotation[col][row]));
    let point = rotate_only(&inverse_rotation, relative);
    if point.iter().any(|value| !value.is_finite()) {
        return Err("Starfield weapon root has a nonfinite transform".into());
    }
    Ok(point)
}

fn combine(parent: &Xform, local: &Xform) -> Xform {
    Xform {
        rotation: matmul3(local.rotation, parent.rotation),
        scale: local.scale * parent.scale,
        translation: apply_xform(parent, local.translation),
    }
}

fn read_local_transform(block: &NifBlock) -> Xform {
    let translation = match block.get_field("Translation") {
        Some(NifValue::Vec3(v)) => *v,
        _ => [0.0; 3],
    };
    let rotation = match block.get_field("Rotation") {
        Some(NifValue::Matrix33(m)) => *m,
        _ => identity_xform().rotation,
    };
    let scale = match block.get_field("Scale") {
        Some(NifValue::Float(v)) => *v as f32,
        Some(NifValue::Int(v)) => *v as f32,
        Some(NifValue::UInt(v)) => *v as f32,
        _ => 1.0,
    };
    Xform {
        translation,
        rotation,
        scale,
    }
}

fn collect_geometries(
    nif: &NifFile,
    block_id: usize,
    parent_world: Xform,
    out: &mut Vec<(Xform, usize)>,
    visited: &mut HashSet<usize>,
) {
    if !visited.insert(block_id) {
        return;
    }
    let Some(block) = nif.get_block(block_id) else {
        return;
    };
    let world = combine(&parent_world, &read_local_transform(block));
    if block.type_name == "BSGeometry" {
        out.push((world, block_id));
        return;
    }
    let Some(NifValue::Array(children)) = block.get_field("Children") else {
        return;
    };
    for child in children {
        if let NifValue::Ref(child_id) = child {
            if *child_id >= 0 {
                collect_geometries(nif, *child_id as usize, world, out, visited);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BSGeometry -> BSTriShape
// ---------------------------------------------------------------------------

fn external_mesh_path(block: &NifBlock) -> Option<String> {
    let NifValue::Array(entries) = block.get_field("Meshes")? else {
        return None;
    };
    for entry in entries {
        let NifValue::Struct(fields) = entry else {
            continue;
        };
        let has_mesh = fields.get("Has Mesh").map(NifValue::as_i64).unwrap_or(0) != 0;
        if !has_mesh {
            continue;
        }
        let Some(NifValue::Struct(mesh_fields)) = fields.get("Mesh") else {
            continue;
        };
        if let Some(NifValue::String(path)) = mesh_fields.get("Mesh Path") {
            let trimmed = path.trim_end_matches('\0').trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn build_bstrishape(
    dest_nif: &mut NifFile,
    source_block: &NifBlock,
    mesh: &SfMeshData,
    world: &Xform,
    src_nif: &NifFile,
    opts: &SfConvertOptions,
    skin: Option<&crate::sf_skin::SkinnedGeometry>,
) -> usize {
    let (vertex_data, bounds_min, bounds_max) = match skin {
        Some(skin) => {
            let (tangents, bitangents) = crate::skin::pack::recompute_tangents_lengyel(
                &skin.positions,
                &skin.normals,
                &mesh.uvs,
                &mesh.triangles,
            );
            let colors = mesh.vertex_colors.as_ref().map(|colors| {
                colors
                    .iter()
                    .map(|color| color.map(|c| c as f32 / 255.0))
                    .collect::<Vec<_>>()
            });
            let data = crate::skin::pack::pack_skinned_vertex_data(
                &skin.positions,
                &skin.normals,
                &tangents,
                &bitangents,
                &mesh.uvs,
                colors.as_deref(),
                &skin.influences,
            );
            let min = std::array::from_fn(|axis| {
                skin.positions
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::INFINITY, f32::min)
            });
            let max = std::array::from_fn(|axis| {
                skin.positions
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::NEG_INFINITY, f32::max)
            });
            (data, min, max)
        }
        None => build_vertex_data(mesh, world),
    };
    let has_vertex_colors = mesh.vertex_colors.is_some();

    let center = [
        (bounds_min[0] + bounds_max[0]) * 0.5,
        (bounds_min[1] + bounds_max[1]) * 0.5,
        (bounds_min[2] + bounds_max[2]) * 0.5,
    ];
    let diagonal = [
        bounds_max[0] - bounds_min[0],
        bounds_max[1] - bounds_min[1],
        bounds_max[2] - bounds_min[2],
    ];
    let radius =
        ((diagonal[0] * diagonal[0] + diagonal[1] * diagonal[1] + diagonal[2] * diagonal[2]).sqrt()
            * 0.5) as f64;

    let mut sphere = IndexMap::new();
    sphere.insert("Center".to_string(), NifValue::Vec3(center));
    sphere.insert("Radius".to_string(), NifValue::Float(radius));

    let triangles: Vec<NifValue> = mesh
        .triangles
        .iter()
        .map(|t| triangle_value(t[0] as i64, t[1] as i64, t[2] as i64))
        .collect();

    let shader_id = build_shader(dest_nif, source_block, src_nif, opts);
    if skin.is_some() {
        let shader = &mut dest_nif.blocks[shader_id];
        let flags = match shader.get_field("Shader Flags 1:FO4") {
            Some(NifValue::UInt(flags)) => *flags,
            _ => FO4_DEFAULT_SHADER_FLAGS_1,
        };
        shader.set_field("Shader Flags 1:FO4", NifValue::UInt(flags | 2));
    }

    let shape_id = dest_nif.add_block("BSTriShape", None);
    if let Some(block) = dest_nif.blocks.get_mut(shape_id) {
        block.set_field(
            "Name",
            source_block
                .get_field("Name")
                .cloned()
                .unwrap_or_else(|| NifValue::String(String::new())),
        );
        block.set_field("Flags", NifValue::UInt(14));
        block.set_field("Bounding Sphere", NifValue::Struct(sphere));
        block.set_field("Shader Property", NifValue::Ref(shader_id as i32));
        block.set_field(
            "Vertex Desc",
            NifValue::Int(if skin.is_some() {
                crate::skin::pack::vertex_desc_skinned(has_vertex_colors)
            } else {
                vertex_desc(has_vertex_colors)
            }),
        );
        if let Some(skin) = skin {
            block.set_field("Skin", NifValue::Ref(skin.instance as i32));
        }
        block.set_field("Num Vertices", NifValue::UInt(vertex_data.len() as u64));
        block.set_field("Num Triangles", NifValue::UInt(triangles.len() as u64));
        block.set_field("Vertex Data", NifValue::Array(vertex_data));
        block.set_field("Triangles", NifValue::Array(triangles));
    }
    shape_id
}

/// Composes each vertex's world position and multiplies by `HAVOK_SCALE`
/// exactly once here, before the bounding sphere above is computed from the
/// returned min/max — the single bake-time multiply site for this path.
fn build_vertex_data(mesh: &SfMeshData, world: &Xform) -> (Vec<NifValue>, [f32; 3], [f32; 3]) {
    let count = mesh.positions.len();
    let (tangents, bitangents) =
        compute_tangent_space(&mesh.positions, &mesh.normals, &mesh.uvs, &mesh.triangles);

    let mut entries = Vec::with_capacity(count);
    let mut bounds_min = [f32::MAX; 3];
    let mut bounds_max = [f32::MIN; 3];

    for index in 0..count {
        let fo4_position = apply_xform(world, mesh.positions[index]).map(|c| c * HAVOK_SCALE);
        for axis in 0..3 {
            bounds_min[axis] = bounds_min[axis].min(fo4_position[axis]);
            bounds_max[axis] = bounds_max[axis].max(fo4_position[axis]);
        }

        let normal = rotate_only(
            &world.rotation,
            mesh.normals.get(index).copied().unwrap_or([0.0, 0.0, 1.0]),
        );
        let tangent = rotate_only(&world.rotation, tangents[index]);
        let bitangent = rotate_only(&world.rotation, bitangents[index]);
        let uv = mesh.uvs.get(index).copied().unwrap_or([0.0, 0.0]);

        let mut entry = IndexMap::new();
        entry.insert("Vertex".to_string(), NifValue::Vec3(fo4_position));
        entry.insert(
            "Bitangent X".to_string(),
            NifValue::Float(bitangent[0] as f64),
        );
        entry.insert("UV".to_string(), tex_coord_value(uv));
        entry.insert("Normal".to_string(), NifValue::Vec3(normal));
        entry.insert(
            "Bitangent Y".to_string(),
            NifValue::Float(bitangent[1] as f64),
        );
        entry.insert("Tangent".to_string(), NifValue::Vec3(tangent));
        entry.insert(
            "Bitangent Z".to_string(),
            NifValue::Float(bitangent[2] as f64),
        );
        if let Some(color) = mesh
            .vertex_colors
            .as_ref()
            .and_then(|colors| colors.get(index))
        {
            entry.insert(
                "Vertex Colors".to_string(),
                NifValue::Color4([
                    color[0] as f32 / 255.0,
                    color[1] as f32 / 255.0,
                    color[2] as f32 / 255.0,
                    color[3] as f32 / 255.0,
                ]),
            );
        }
        entries.push(NifValue::Struct(entry));
    }

    if count == 0 {
        bounds_min = [0.0; 3];
        bounds_max = [0.0; 3];
    }
    (entries, bounds_min, bounds_max)
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

fn tex_coord_value(uv: [f32; 2]) -> NifValue {
    let mut data = IndexMap::new();
    data.insert("u".to_string(), NifValue::Float(uv[0] as f64));
    data.insert("v".to_string(), NifValue::Float(uv[1] as f64));
    NifValue::Struct(data)
}

fn triangle_value(v1: i64, v2: i64, v3: i64) -> NifValue {
    let mut data = IndexMap::new();
    data.insert("v1".to_string(), NifValue::Int(v1));
    data.insert("v2".to_string(), NifValue::Int(v2));
    data.insert("v3".to_string(), NifValue::Int(v3));
    NifValue::Struct(data)
}

/// Lengyel per-triangle tangent-space accumulation, Gram-Schmidt orthogonalized
/// against the vertex normal. Recomputed after conversion so the tangent frame
/// agrees with the emitted positions, normals and UVs.
fn compute_tangent_space(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    triangles: &[[u32; 3]],
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let count = positions.len();
    let mut accum_tangent = vec![[0.0f32; 3]; count];
    let mut accum_bitangent = vec![[0.0f32; 3]; count];

    for triangle in triangles {
        let (i0, i1, i2) = (
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        );
        if i0 >= count || i1 >= count || i2 >= count {
            continue;
        }
        let (p0, p1, p2) = (positions[i0], positions[i1], positions[i2]);
        let (w0, w1, w2) = (
            uvs.get(i0).copied().unwrap_or([0.0, 0.0]),
            uvs.get(i1).copied().unwrap_or([0.0, 0.0]),
            uvs.get(i2).copied().unwrap_or([0.0, 0.0]),
        );
        let edge1 = sub3(p1, p0);
        let edge2 = sub3(p2, p0);
        let (s1, t1) = (w1[0] - w0[0], w1[1] - w0[1]);
        let (s2, t2) = (w2[0] - w0[0], w2[1] - w0[1]);
        let denom = s1 * t2 - s2 * t1;
        if denom.abs() < 1e-12 {
            continue;
        }
        let inv = 1.0 / denom;
        let sdir = scale3(sub3(scale3(edge1, t2), scale3(edge2, t1)), inv);
        let tdir = scale3(sub3(scale3(edge2, s1), scale3(edge1, s2)), inv);
        for &i in &[i0, i1, i2] {
            accum_tangent[i] = add3(accum_tangent[i], sdir);
            accum_bitangent[i] = add3(accum_bitangent[i], tdir);
        }
    }

    let mut tangents = Vec::with_capacity(count);
    let mut bitangents = Vec::with_capacity(count);
    for index in 0..count {
        let normal = normals.get(index).copied().unwrap_or([0.0, 0.0, 1.0]);
        let raw_tangent = accum_tangent[index];
        let projected = sub3(raw_tangent, scale3(normal, dot3(normal, raw_tangent)));
        let tangent = normalize3(projected).unwrap_or_else(|| arbitrary_orthogonal(normal));
        let handedness = if dot3(cross3(normal, raw_tangent), accum_bitangent[index]) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let bitangent = scale3(cross3(normal, tangent), handedness);
        tangents.push(tangent);
        bitangents.push(bitangent);
    }
    (tangents, bitangents)
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize3(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = dot3(v, v).sqrt();
    if len < 1e-8 || !len.is_finite() {
        None
    } else {
        Some(scale3(v, 1.0 / len))
    }
}
fn arbitrary_orthogonal(normal: [f32; 3]) -> [f32; 3] {
    let candidate = if normal[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    normalize3(sub3(candidate, scale3(normal, dot3(normal, candidate)))).unwrap_or([1.0, 0.0, 0.0])
}

// ---------------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------------

fn build_shader(
    dest_nif: &mut NifFile,
    source_block: &NifBlock,
    src_nif: &NifFile,
    opts: &SfConvertOptions,
) -> usize {
    let material_path = source_shader_material_path(src_nif, source_block)
        .map(|path| rewrite_material_path(&path, &opts.material_path_rewriter))
        .unwrap_or_default();
    let shader_id = dest_nif.add_block("BSLightingShaderProperty", None);
    if let Some(block) = dest_nif.blocks.get_mut(shader_id) {
        populate_fo4_external_shader_defaults(block, material_path);
    }
    shader_id
}

fn populate_fo4_external_shader_defaults(block: &mut NifBlock, material_path: String) {
    block.set_field("Shader Type", NifValue::UInt(0));
    block.set_field("Name", NifValue::String(material_path));
    block.set_field(
        "Shader Flags 1:FO4",
        NifValue::UInt(FO4_DEFAULT_SHADER_FLAGS_1),
    );
    block.set_field("Shader Flags 2:FO4", NifValue::UInt(1));
    block.set_field("UV Offset", tex_coord_value([0.0, 0.0]));
    block.set_field("UV Scale", tex_coord_value([1.0, 1.0]));
    block.set_field("Texture Set", NifValue::Ref(-1));
    block.set_field("Emissive Color", NifValue::Color3([0.0, 0.0, 0.0]));
    block.set_field("Emissive Multiple", NifValue::Float(1.0));
    block.set_field("Root Material", NifValue::String(String::new()));
    block.set_field("Texture Clamp Mode", NifValue::UInt(3));
    block.set_field("Alpha", NifValue::Float(1.0));
    block.set_field("Refraction Strength", NifValue::Float(0.0));
    block.set_field("Smoothness", NifValue::Float(1.0));
    block.set_field("Specular Color", NifValue::Color3([1.0, 1.0, 1.0]));
    block.set_field("Specular Strength", NifValue::Float(1.0));
    block.set_field("Subsurface Rolloff", NifValue::Float(0.0));
    // FO4 readers gate the following Backlight field on this sentinel.
    block.set_field("Rimlight Power", NifValue::Float(f32::MAX as f64));
    block.set_field("Backlight Power", NifValue::Float(0.0));
    block.set_field("Grayscale to Palette Scale", NifValue::Float(1.0));
    block.set_field("Fresnel Power", NifValue::Float(5.0));
    block.set_field("Wetness", default_fo4_wetness());
}

fn default_fo4_wetness() -> NifValue {
    let mut fields = IndexMap::new();
    for name in [
        "Spec Scale",
        "Spec Power",
        "Min Var",
        "Env Map Scale",
        "Fresnel Power",
        "Metalness",
    ] {
        fields.insert(name.to_string(), NifValue::Float(-1.0));
    }
    NifValue::Struct(fields)
}

fn source_shader_material_path(src_nif: &NifFile, geometry: &NifBlock) -> Option<String> {
    let shader_ref = match geometry.get_field("Shader Property") {
        Some(NifValue::Ref(id)) if *id >= 0 => *id as usize,
        _ => return None,
    };
    let shader = src_nif.get_block(shader_ref)?;
    match shader.get_field("Name") {
        Some(NifValue::String(name)) => {
            let trimmed = name.trim_end_matches('\0').trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        _ => None,
    }
}

fn rewrite_material_path(
    path: &str,
    rewriter: &Option<Box<dyn Fn(&str) -> String + Send + Sync>>,
) -> String {
    rewriter
        .as_ref()
        .map_or_else(|| converted_material_path(path), |rewrite| rewrite(path))
}

pub(crate) fn converted_material_path(path: &str) -> String {
    if path.to_ascii_lowercase().ends_with(".mat") {
        format!("{}.bgsm", &path[..path.len() - 4])
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod native_material_tests {
    use super::*;

    #[test]
    fn native_material_rewriter_sees_mat_paths_while_default_still_uses_bgsm() {
        let path = "Materials/Weapons/Shotty.MAT";
        assert_eq!(
            rewrite_material_path(path, &None),
            "Materials/Weapons/Shotty.bgsm"
        );
        let native: Option<Box<dyn Fn(&str) -> String + Send + Sync>> =
            Some(Box::new(str::to_owned));
        assert_eq!(rewrite_material_path(path, &native), path);
    }
}

// ---------------------------------------------------------------------------
// Scene graph attach helper
// ---------------------------------------------------------------------------

fn attach_child(nif: &mut NifFile, parent_id: usize, child_id: usize) {
    let mut children: Vec<NifValue> = match nif
        .get_block(parent_id)
        .and_then(|b| b.get_field("Children"))
    {
        Some(NifValue::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    children.push(NifValue::Ref(child_id as i32));
    let count = children.len() as u64;
    if let Some(block) = nif.blocks.get_mut(parent_id) {
        block.set_field("Children", NifValue::Array(children));
        block.set_field("Num Children", NifValue::UInt(count));
    }
}

// ---------------------------------------------------------------------------
// Collision hybrid: decode -> fallback -> none
// ---------------------------------------------------------------------------

fn install_collision(
    src_nif: &NifFile,
    dest_nif: &mut NifFile,
    parent_id: usize,
    fallback_vertices: &[[f32; 3]],
    fallback_triangles: &[[u32; 3]],
    report: &mut SfConvertReport,
    weapon: bool,
) -> Result<(), String> {
    let mut decoded_shapes: Vec<MultiBodyShape> = Vec::new();
    let mut decoded_metas = Vec::new();
    let mut any_failure = false;

    for block in &src_nif.blocks {
        if block.type_name != "bhkPhysicsSystem" {
            continue;
        }
        let Some(binary_data) = block.get_field("Binary Data") else {
            continue;
        };
        let Some(bytes) = crate::cloth::byte_array_to_bytes(binary_data).ok() else {
            continue;
        };
        match decode_starfield_collision(&bytes) {
            Ok(mut shapes) => {
                let context = crate::fo76_collision::SourceCollisionContext::new(&bytes);
                for (index, shape) in shapes.iter_mut().enumerate() {
                    let mut meta = BodyMeta::default();
                    if weapon {
                        let source = context.body_metadata(index);
                        meta.layer = 5;
                        meta.dynamic = Some(source.motion_type.map_or(true, |motion| motion == 2));
                        meta.motion_type = if source.motion_type == Some(1) {
                            BodyMotionType::Keyframed
                        } else {
                            BodyMotionType::Static
                        };
                        meta.body_mass = source.body_mass;
                        meta.mass_distribution = context.mass_distribution(index);
                        meta.position = source.position.unwrap_or(meta.position);
                        meta.orientation = source.orientation.unwrap_or(meta.orientation);
                        if meta.dynamic == Some(true) {
                            make_dynamic_shape_convex(shape);
                        }
                    }
                    decoded_metas.push(meta);
                }
                decoded_shapes.append(&mut shapes);
            }
            // Not a collision payload (e.g. cloth data sharing the NIF's TAG0
            // slot on banner/cloth assets) — absence is normal, skip silently.
            Err(error) if error.contains("hknpPhysicsSystemData") => {}
            Err(_) => any_failure = true,
        }
    }

    if any_failure {
        if let Some(shape) = build_fallback_shape(fallback_vertices, fallback_triangles) {
            let mut shape = shape;
            let mut meta = BodyMeta::default();
            if weapon {
                make_dynamic_shape_convex(&mut shape);
                meta.layer = 5;
                meta.dynamic = Some(true);
                meta.body_mass = decoded_metas
                    .iter()
                    .filter_map(|meta| meta.body_mass)
                    .reduce(|a, b| a + b);
            }
            if attach_collision(dest_nif, parent_id, vec![shape], &[meta]).is_ok() {
                report.collision_fallback += 1;
                return Ok(());
            }
        }
        report.collision_none += 1;
        return if weapon {
            Err("Starfield weapon collision could not be converted or rebuilt".into())
        } else {
            Ok(())
        };
    }

    if !decoded_shapes.is_empty() {
        if let Err(error) = attach_collision(dest_nif, parent_id, decoded_shapes, &decoded_metas) {
            if weapon {
                return Err(format!("Starfield weapon physics: {error}"));
            }
        } else {
            report.collision_decoded += 1;
            return Ok(());
        }
        report.collision_none += 1;
        return Ok(());
    }

    report.collision_none += 1;
    Ok(())
}

fn make_dynamic_shape_convex(shape: &mut MultiBodyShape) {
    if let MultiBodyShape::CompressedMesh { vertices, .. } = shape {
        *shape = MultiBodyShape::Polytope {
            vertices: std::mem::take(vertices),
        };
    }
}

/// `vertices` are already the FO4-unit (post `HAVOK_SCALE`) render positions
/// this NIF just wrote — divide by `HAVOK_SCALE` exactly once to reach the
/// Havok-metre space `MultiBodyShape::CompressedMesh` expects.
fn build_fallback_shape(vertices: &[[f32; 3]], triangles: &[[u32; 3]]) -> Option<MultiBodyShape> {
    if vertices.is_empty() || triangles.is_empty() {
        return None;
    }
    let metre_vertices: Vec<[f32; 3]> = vertices
        .iter()
        .map(|v| v.map(|c| c / HAVOK_SCALE))
        .collect();
    let (vertices, triangles) = if triangles.len() > FALLBACK_COLLISION_TRIANGLE_BUDGET {
        let decimated = havok_native::geometry::decimate(
            &metre_vertices,
            triangles,
            FALLBACK_COLLISION_TRIANGLE_BUDGET,
        );
        (decimated.vertices, decimated.triangles)
    } else {
        (metre_vertices, triangles.to_vec())
    };
    Some(MultiBodyShape::CompressedMesh {
        vertices,
        triangles,
    })
}

fn attach_collision(
    nif: &mut NifFile,
    parent_id: usize,
    shapes: Vec<MultiBodyShape>,
    metas: &[BodyMeta],
) -> Result<(), String> {
    if shapes.is_empty() {
        return Err("no collision shapes".to_string());
    }
    let options = BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer: FO4_STATIC_LAYER,
        mass: 0.0,
        convex_radius: DEFAULT_CONVEX_RADIUS,
        materials: Vec::new(),
        user_data: None,
        body_props_raw: None,
        mass_distribution: None,
    };
    let blob = build_fo4_multi_body_collision(&shapes, &options, None, Some(metas))
        .map_err(|error| error.to_string())?;

    let physics_id = nif.add_block("bhkPhysicsSystem", None);
    if let Some(block) = nif.blocks.get_mut(physics_id) {
        block.set_field("Binary Data", crate::cloth::bytes_to_byte_array(&blob));
    }
    let collision_id = nif.add_block("bhkNPCollisionObject", None);
    if let Some(block) = nif.blocks.get_mut(collision_id) {
        block.set_field("Flags", NifValue::UInt(0x80));
        block.set_field("Target", NifValue::Ref(parent_id as i32));
        block.set_field("Data", NifValue::Ref(physics_id as i32));
        block.set_field("Body ID", NifValue::UInt(0));
    }
    if let Some(block) = nif.blocks.get_mut(parent_id) {
        block.set_field("Collision Object", NifValue::Ref(collision_id as i32));
    }
    ensure_havok_bsx_flag(nif, parent_id);
    Ok(())
}

fn ensure_havok_bsx_flag(nif: &mut NifFile, root_id: usize) {
    let mut extra_ids: Vec<i32> = match nif
        .get_block(root_id)
        .and_then(|b| b.get_field("Extra Data List"))
    {
        Some(NifValue::Array(items)) => items
            .iter()
            .filter_map(|value| match value {
                NifValue::Ref(id) => Some(*id),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    for extra_id in extra_ids.iter().copied().filter(|id| *id >= 0) {
        if let Some(extra) = nif.blocks.get_mut(extra_id as usize) {
            if extra.type_name == "BSXFlags" {
                let current = extra
                    .get_field("Integer Data")
                    .map(NifValue::as_i64)
                    .unwrap_or(0) as u64;
                extra.set_field("Integer Data", NifValue::UInt(current | BSX_HAVOK_FLAG));
                return;
            }
        }
    }

    let bsx_id = nif.add_block("BSXFlags", None);
    if let Some(block) = nif.blocks.get_mut(bsx_id) {
        block.set_field("Name", NifValue::String("BSX".to_string()));
        block.set_field("Integer Data", NifValue::UInt(BSX_HAVOK_FLAG));
    }
    extra_ids.push(bsx_id as i32);
    let count = extra_ids.len() as u64;
    if let Some(block) = nif.blocks.get_mut(root_id) {
        block.set_field(
            "Extra Data List",
            NifValue::Array(extra_ids.into_iter().map(NifValue::Ref).collect()),
        );
        block.set_field("Num Extra Data List", NifValue::UInt(count));
    }
}
