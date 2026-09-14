//! FO4 → Starfield `BSGeometry` NIF + `.mesh` writer.
//!
//! Recipe and evidence: `bacup/docs/starfield_target/R3-mesh-format.md` (NIF block layout,
//! `.mesh` binary field table, mesh-path scheme, MaterialID CRC) and
//! `bacup/docs/starfield_target/R7-collision.md` (collision embedding, via
//! `crate::starfield_collision`).
//!
//! nif_core already parses and writes Starfield NIFs generically (schema-driven `NifFile` /
//! `NifBlock`), so this module is "`.mesh` write path + block assembly", not a NIF writer from
//! scratch. It does not use the `crate::sf_mesh` reader: see R3 §1a for the three constants
//! that reader gets wrong and must not be copied here (position dequant asymmetry, DEC3N
//! `/511.5` basis, triangle winding).

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use indexmap::IndexMap;

use crate::fo76_collision::HAVOK_SCALE;
use crate::model::{NifBlock, NifFile, NifHeader, NifValue};
use crate::starfield_collision::{self, CollisionPrim};

/// FO4 units → Starfield metres (R3 §7 / R7 §2 "Units"). Applied exactly once to every
/// vertex position and node translation.
const FO4_TO_SF_SPATIAL: f32 = 1.0 / HAVOK_SCALE;

/// `.mesh` index buffer is `u16` — a single `BSGeometry`/`.mesh` pair cannot exceed this many
/// vertices (R3 §4 row 6, §10 risk 6).
const MAX_MESH_VERTS: usize = 65535;

pub struct StarfieldMeshOut {
    pub nif_bytes: Vec<u8>,
    /// `(relative path under Data, e.g. "geometries/<20hex>/<20hex>.mesh", bytes)`.
    pub mesh_files: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionMode {
    /// Extract FO4 `bhkPhysicsSystem` collision and re-emit it for Starfield
    /// (`crate::starfield_collision::extract_fo4_collision_prims` +
    /// `emit_starfield_collision`). Falls back to a render-geometry AABB internally when a
    /// FO4 collision blob exists but does not decode (R7 §6).
    FromFo4Prims,
    /// Skip FO4 collision extraction; emit a single box hull over the converted (already
    /// Starfield-space) render geometry.
    AabbFallback,
    /// No collision block is written. A legal, non-crashing outcome — not every vanilla
    /// static has collision.
    None,
}

/// Convert a single FO4 NIF's static geometry (and optionally its collision) into a Starfield
/// `BSGeometry` NIF plus the `.mesh` files it references.
///
/// `mat_path_for` maps a source FO4 shape (`BSTriShape`/`BSSubIndexTriShape` block) to the
/// output `.mat` path, e.g. `"materials/foo/bar.mat"` (forward slashes; tests pass a stub).
/// The crate has no shared cross-module `Shape` type — shapes are
/// schema-driven `NifBlock`s here, so that is what the callback receives.
pub fn convert_fo4_nif_to_starfield(
    fo4_nif_bytes: &[u8],
    mat_path_for: &dyn Fn(&NifBlock) -> String,
    collision: CollisionMode,
) -> Result<StarfieldMeshOut, String> {
    let nif_in = NifFile::from_bytes(fo4_nif_bytes, None)
        .map_err(|error| format!("starfield_write: failed to parse FO4 NIF: {error}"))?;

    let extracted: Vec<ExtractedShape> = nif_in.blocks.iter().filter_map(extract_shape).collect();
    if extracted.is_empty() {
        return Err(
            "starfield_write: no BSTriShape/BSSubIndexTriShape geometry found in input NIF"
                .to_string(),
        );
    }

    let expanded: Vec<ExtractedShape> = extracted
        .into_iter()
        .flat_map(|shape| split_oversized(shape, MAX_MESH_VERTS))
        .collect();

    let mut nif_out = NifFile {
        header: new_starfield_header(),
        ..NifFile::default()
    };

    let root_name = nif_in
        .blocks
        .iter()
        .find(|block| matches!(block.type_name.as_str(), "NiNode" | "BSFadeNode"))
        .and_then(|block| string_field(block, "Name"))
        .unwrap_or_default();
    let root_id = nif_out.add_block("NiNode", None);
    nif_out.blocks[root_id].set_field("Name", NifValue::String(root_name));
    nif_out.blocks[root_id].set_field("Flags", NifValue::UInt(14));
    nif_out.blocks[root_id].set_field("Collision Object", NifValue::Ref(-1));

    let mut mesh_files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut children: Vec<NifValue> = Vec::new();

    for (index, shape) in expanded.iter().enumerate() {
        let mesh_bytes = build_mesh_bytes(shape);
        let (folder, file) = mesh_path_from_bytes(&mesh_bytes, index);
        let mesh_ref = format!("{folder}\\{file}");
        mesh_files.push((format!("geometries/{folder}/{file}.mesh"), mesh_bytes));

        let mat_rel = mat_path_for(&shape.source_block);
        let mat_path_bs = mat_rel.replace('/', "\\");
        let material_id =
            materials_native::cdb::bethesda_crc32(mat_path_bs.to_ascii_lowercase().as_bytes());

        let shader_id = nif_out.add_block("BSLightingShaderProperty", None);
        nif_out.blocks[shader_id].set_field("Name", NifValue::String(mat_path_bs));

        let mut extra_fields = IndexMap::new();
        extra_fields.insert(
            "Name".to_string(),
            NifValue::String("MaterialID".to_string()),
        );
        extra_fields.insert(
            "Integer Data".to_string(),
            NifValue::UInt(material_id as u64),
        );
        let extra_id = nif_out.add_block("NiIntegerExtraData", Some(extra_fields));

        let geo_id = nif_out.add_block("BSGeometry", None);
        let indices_size = (shape.triangles.len() * 3) as u32;
        let num_verts = shape.positions.len() as u32;
        {
            let block = &mut nif_out.blocks[geo_id];
            block.set_field(
                "Name",
                NifValue::String(format!("{}:{}", shape.name, index)),
            );
            block.set_field("Flags", NifValue::UInt(14));
            block.set_field("Translation", NifValue::Vec3(shape.translation));
            block.set_field("Rotation", NifValue::Matrix33(shape.rotation));
            block.set_field("Scale", NifValue::Float(shape.scale as f64));
            block.set_field("Collision Object", NifValue::Ref(-1));
            block.set_field("Bounding Sphere", bounding_sphere_value(&shape.positions));
            block.set_field("Bounding Box", bounding_box_value(&shape.positions));
            block.set_field("Skin", NifValue::Ref(-1));
            block.set_field("Shader Property", NifValue::Ref(shader_id as i32));
            block.set_field("Alpha Property", NifValue::Ref(-1));
            block.set_field("Controller", NifValue::Ref(-1));
            block.set_field("Num Extra Data List", NifValue::UInt(1));
            block.set_field(
                "Extra Data List",
                NifValue::Array(vec![NifValue::Ref(extra_id as i32)]),
            );
            block.set_field(
                "Meshes",
                NifValue::Array(vec![
                    mesh_array_slot_populated(indices_size, num_verts, &mesh_ref),
                    mesh_array_slot_empty(),
                    mesh_array_slot_empty(),
                    mesh_array_slot_empty(),
                ]),
            );
        }
        children.push(NifValue::Ref(geo_id as i32));
    }

    let collision_out = match collision {
        CollisionMode::None => None,
        CollisionMode::FromFo4Prims => {
            let prims = starfield_collision::extract_fo4_collision_prims(fo4_nif_bytes)?;
            starfield_collision::emit_starfield_collision(&prims)?
        }
        CollisionMode::AabbFallback => {
            let prim = aabb_collision_prim(&expanded);
            starfield_collision::emit_starfield_collision(std::slice::from_ref(&prim))?
        }
    };

    let mut bsx_flags: u64 = 0;
    if let Some(out) = collision_out {
        let phys_id = nif_out.add_block("bhkPhysicsSystem", None);
        nif_out.blocks[phys_id].set_field(
            "Binary Data",
            crate::cloth::bytes_to_byte_array(&out.physics_system_binary_data),
        );

        let mut co_fields = IndexMap::new();
        co_fields.insert("Target".to_string(), NifValue::Ref(root_id as i32));
        co_fields.insert(
            "Flags".to_string(),
            NifValue::UInt(out.collision_object_flags as u64),
        );
        co_fields.insert("Data".to_string(), NifValue::Ref(phys_id as i32));
        co_fields.insert("Body ID".to_string(), NifValue::UInt(out.body_id as u64));
        let co_id = nif_out.add_block("bhkNPCollisionObject", Some(co_fields));

        nif_out.blocks[root_id].set_field("Collision Object", NifValue::Ref(co_id as i32));
        bsx_flags |= out.bsx_flags_value;
    }

    let mut bsx_fields = IndexMap::new();
    bsx_fields.insert("Name".to_string(), NifValue::String("BSX".to_string()));
    bsx_fields.insert("Integer Data".to_string(), NifValue::UInt(bsx_flags));
    let bsx_id = nif_out.add_block("BSXFlags", Some(bsx_fields));

    nif_out.blocks[root_id].set_field("Num Extra Data List", NifValue::UInt(1));
    nif_out.blocks[root_id].set_field(
        "Extra Data List",
        NifValue::Array(vec![NifValue::Ref(bsx_id as i32)]),
    );
    nif_out.blocks[root_id].set_field("Num Children", NifValue::UInt(children.len() as u64));
    nif_out.blocks[root_id].set_field("Children", NifValue::Array(children));

    nif_out.header.footer_roots = vec![root_id as i32];

    let nif_bytes = nif_out
        .to_bytes()
        .map_err(|error| format!("starfield_write: failed to serialize output NIF: {error}"))?;

    Ok(StarfieldMeshOut {
        nif_bytes,
        mesh_files,
    })
}

// ---------- header ----------

fn new_starfield_header() -> NifHeader {
    NifHeader {
        header_string: "Gamebryo File Format, Version 20.2.0.7".to_string(),
        version: (20, 2, 0, 7),
        version_packed: 0x1402_0007,
        endian_type: 1,
        user_version: 12,
        bs_version: 175,
        // R3 §2 header table: Author = "\0" (length-1 string holding one NUL byte).
        creator: "\0".to_string(),
        // Export Script = "\0", same convention.
        export_info: vec!["\0".to_string()],
        // Unknown Data (ExportDataSF, BSVer >= 170) = Length 1, Value [0x00] — the proven-legal
        // empty form (89/600 vanilla NIFs, R3 §2).
        sf_export_data: vec![0u8],
        ..Default::default()
    }
}

// ---------- geometry extraction (FO4 side) ----------

#[derive(Clone)]
struct ExtractedShape {
    name: String,
    /// Node translation, already ÷ `HAVOK_SCALE` (FO4_TO_SF_SPATIAL applied once).
    translation: [f32; 3],
    rotation: [[f32; 3]; 3],
    scale: f32,
    /// Local vertex positions, already ÷ `HAVOK_SCALE`.
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 3]>,
    bitangents: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    /// Same length as `positions`; only emitted into `.mesh` when `has_vertex_colors`.
    vertex_colors: Vec<[u8; 4]>,
    has_vertex_colors: bool,
    triangles: Vec<[u32; 3]>,
    /// The original FO4 shape block, handed to `mat_path_for` unmodified.
    source_block: NifBlock,
}

const IDENTITY_MAT3: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn extract_shape(block: &NifBlock) -> Option<ExtractedShape> {
    if !matches!(
        block.type_name.as_str(),
        "BSTriShape" | "BSSubIndexTriShape"
    ) {
        return None;
    }
    let entries = value_array(block.get_field("Vertex Data"));
    if entries.is_empty() {
        return None;
    }

    let mut positions = Vec::with_capacity(entries.len());
    let mut normals = Vec::with_capacity(entries.len());
    let mut tangents = Vec::with_capacity(entries.len());
    let mut bitangents = Vec::with_capacity(entries.len());
    let mut uvs = Vec::with_capacity(entries.len());
    let mut vertex_colors = Vec::with_capacity(entries.len());
    let mut has_vertex_colors = false;

    for entry in &entries {
        let NifValue::Struct(fields) = entry else {
            continue;
        };
        let position = vec3_value(fields.get("Vertex")).unwrap_or([0.0, 0.0, 0.0]);
        positions.push([
            position[0] * FO4_TO_SF_SPATIAL,
            position[1] * FO4_TO_SF_SPATIAL,
            position[2] * FO4_TO_SF_SPATIAL,
        ]);
        normals.push(vec3_value(fields.get("Normal")).unwrap_or([0.0, 0.0, 1.0]));
        tangents.push(vec3_value(fields.get("Tangent")).unwrap_or([1.0, 0.0, 0.0]));
        bitangents.push([
            value_f64(fields.get("Bitangent X")).unwrap_or(0.0) as f32,
            value_f64(fields.get("Bitangent Y")).unwrap_or(0.0) as f32,
            value_f64(fields.get("Bitangent Z")).unwrap_or(0.0) as f32,
        ]);
        uvs.push(fields.get("UV").and_then(uv_value).unwrap_or([0.0, 0.0]));
        if let Some(color) = fields.get("Vertex Colors").and_then(color4_value) {
            has_vertex_colors = true;
            vertex_colors.push([
                (color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                (color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                (color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                (color[3].clamp(0.0, 1.0) * 255.0).round() as u8,
            ]);
        } else {
            vertex_colors.push([255, 255, 255, 255]);
        }
    }

    let triangles: Vec<[u32; 3]> = value_array(block.get_field("Triangles"))
        .iter()
        .filter_map(triangle_from_value)
        .collect();

    let translation = vec3_value(block.get_field("Translation"))
        .map(|t| {
            [
                t[0] * FO4_TO_SF_SPATIAL,
                t[1] * FO4_TO_SF_SPATIAL,
                t[2] * FO4_TO_SF_SPATIAL,
            ]
        })
        .unwrap_or([0.0, 0.0, 0.0]);
    let rotation = match block.get_field("Rotation") {
        Some(NifValue::Matrix33(m)) => *m,
        _ => IDENTITY_MAT3,
    };
    let scale = value_f64(block.get_field("Scale")).unwrap_or(1.0) as f32;
    let name = string_field(block, "Name").unwrap_or_default();

    Some(ExtractedShape {
        name,
        translation,
        rotation,
        scale,
        positions,
        normals,
        tangents,
        bitangents,
        uvs,
        vertex_colors,
        has_vertex_colors,
        triangles,
        source_block: block.clone(),
    })
}

/// Split a shape whose vertex count exceeds `max_verts` into multiple shapes, each a
/// self-contained subset of the original triangles with a freshly local-indexed vertex array
/// (R3 §10 risk 6 — `.mesh` indices are `u16`). Triangles are consumed in original order; a
/// new batch starts only when the *next* triangle would push the running unique-vertex count
/// over `max_verts`, so no batch can exceed the limit.
fn split_oversized(shape: ExtractedShape, max_verts: usize) -> Vec<ExtractedShape> {
    if shape.positions.len() <= max_verts {
        return vec![shape];
    }

    let mut out = Vec::new();
    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut current = empty_like(&shape);

    for triangle in &shape.triangles {
        let new_count = triangle
            .iter()
            .filter(|vertex| !remap.contains_key(vertex))
            .count();
        if !remap.is_empty() && remap.len() + new_count > max_verts {
            out.push(std::mem::replace(&mut current, empty_like(&shape)));
            remap.clear();
        }

        let mut mapped = [0u32; 3];
        for (slot, &vertex) in triangle.iter().enumerate() {
            let new_index = *remap.entry(vertex).or_insert_with(|| {
                let index = current.positions.len() as u32;
                let v = vertex as usize;
                current.positions.push(shape.positions[v]);
                current.normals.push(shape.normals[v]);
                current.tangents.push(shape.tangents[v]);
                current.bitangents.push(shape.bitangents[v]);
                current.uvs.push(shape.uvs[v]);
                current.vertex_colors.push(shape.vertex_colors[v]);
                index
            });
            mapped[slot] = new_index;
        }
        current.triangles.push(mapped);
    }

    if !current.positions.is_empty() {
        out.push(current);
    }
    out
}

fn empty_like(shape: &ExtractedShape) -> ExtractedShape {
    ExtractedShape {
        name: shape.name.clone(),
        translation: shape.translation,
        rotation: shape.rotation,
        scale: shape.scale,
        positions: Vec::new(),
        normals: Vec::new(),
        tangents: Vec::new(),
        bitangents: Vec::new(),
        uvs: Vec::new(),
        vertex_colors: Vec::new(),
        has_vertex_colors: shape.has_vertex_colors,
        triangles: Vec::new(),
        source_block: shape.source_block.clone(),
    }
}

// ---------- NIF field value helpers (local copies; see other modules for the same pattern) ----------

fn value_array(value: Option<&NifValue>) -> Vec<NifValue> {
    match value {
        Some(NifValue::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

fn value_f64(value: Option<&NifValue>) -> Option<f64> {
    match value? {
        NifValue::Float(v) => Some(*v),
        NifValue::Int(v) => Some(*v as f64),
        NifValue::UInt(v) => Some(*v as f64),
        _ => None,
    }
}

fn vec3_value(value: Option<&NifValue>) -> Option<[f32; 3]> {
    match value? {
        NifValue::Vec3(v) => Some(*v),
        NifValue::Struct(fields) => Some([
            value_f64(fields.get("x")).unwrap_or(0.0) as f32,
            value_f64(fields.get("y")).unwrap_or(0.0) as f32,
            value_f64(fields.get("z")).unwrap_or(0.0) as f32,
        ]),
        _ => None,
    }
}

fn uv_value(value: &NifValue) -> Option<[f32; 2]> {
    match value {
        NifValue::Struct(fields) => Some([
            value_f64(fields.get("u")).unwrap_or(0.0) as f32,
            value_f64(fields.get("v")).unwrap_or(0.0) as f32,
        ]),
        _ => None,
    }
}

fn color4_value(value: &NifValue) -> Option<[f32; 4]> {
    match value {
        NifValue::Color4(v) => Some(*v),
        NifValue::Struct(fields) => Some([
            value_f64(fields.get("r")).unwrap_or(1.0) as f32,
            value_f64(fields.get("g")).unwrap_or(1.0) as f32,
            value_f64(fields.get("b")).unwrap_or(1.0) as f32,
            value_f64(fields.get("a")).unwrap_or(1.0) as f32,
        ]),
        _ => None,
    }
}

fn triangle_from_value(value: &NifValue) -> Option<[u32; 3]> {
    let NifValue::Struct(fields) = value else {
        return None;
    };
    Some([
        fields.get("v1").map(NifValue::as_i64)? as u32,
        fields.get("v2").map(NifValue::as_i64)? as u32,
        fields.get("v3").map(NifValue::as_i64)? as u32,
    ])
}

fn string_field(block: &NifBlock, name: &str) -> Option<String> {
    match block.get_field(name)? {
        NifValue::String(v) => Some(v.trim_end_matches('\0').to_string()),
        _ => None,
    }
}

// ---------- BSGeometry field builders ----------

fn bounding_box_value(positions: &[[f32; 3]]) -> NifValue {
    let (min, max) = aabb(positions);
    let mut fields = IndexMap::new();
    fields.insert(
        "Center".to_string(),
        NifValue::Vec3([
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        ]),
    );
    fields.insert(
        "Dimensions".to_string(),
        NifValue::Vec3([
            (max[0] - min[0]) * 0.5,
            (max[1] - min[1]) * 0.5,
            (max[2] - min[2]) * 0.5,
        ]),
    );
    NifValue::Struct(fields)
}

fn bounding_sphere_value(positions: &[[f32; 3]]) -> NifValue {
    let (min, max) = aabb(positions);
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let mut radius = 0f32;
    for p in positions {
        let d =
            ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2))
                .sqrt();
        if d > radius {
            radius = d;
        }
    }
    let mut fields = IndexMap::new();
    fields.insert("Center".to_string(), NifValue::Vec3(center));
    fields.insert("Radius".to_string(), NifValue::Float(radius as f64));
    NifValue::Struct(fields)
}

fn aabb(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    if positions.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let mut min = positions[0];
    let mut max = positions[0];
    for p in positions {
        for k in 0..3 {
            if p[k] < min[k] {
                min[k] = p[k];
            }
            if p[k] > max[k] {
                max[k] = p[k];
            }
        }
    }
    (min, max)
}

fn mesh_array_slot_populated(indices_size: u32, num_verts: u32, mesh_ref: &str) -> NifValue {
    let mut mesh = IndexMap::new();
    mesh.insert(
        "Indices Size".to_string(),
        NifValue::UInt(indices_size as u64),
    );
    mesh.insert("Num Verts".to_string(), NifValue::UInt(num_verts as u64));
    mesh.insert("Flags".to_string(), NifValue::UInt(64));
    mesh.insert(
        "Mesh Path".to_string(),
        NifValue::String(mesh_ref.to_string()),
    );

    let mut slot = IndexMap::new();
    slot.insert("Has Mesh".to_string(), NifValue::UInt(1));
    slot.insert("Mesh".to_string(), NifValue::Struct(mesh));
    NifValue::Struct(slot)
}

fn mesh_array_slot_empty() -> NifValue {
    let mut slot = IndexMap::new();
    slot.insert("Has Mesh".to_string(), NifValue::UInt(0));
    NifValue::Struct(slot)
}

// ---------- mesh path scheme (R3 §6) ----------

/// Both `geometries/*.mesh` path components are opaque identifiers the engine never
/// validates (R3 §6) — content-hashing the emitted `.mesh` bytes is an explicitly endorsed
/// scheme ("gets free dedup ... nothing downstream parses the name"). `disambiguator` is the
/// output shape's index, so two shapes that happen to produce byte-identical `.mesh` content
/// (e.g. two unit cubes) still land at distinct paths.
fn mesh_path_from_bytes(mesh_bytes: &[u8], disambiguator: usize) -> (String, String) {
    (
        hash20(mesh_bytes, disambiguator as u64),
        hash20(mesh_bytes, (disambiguator as u64) ^ 0x9E37_79B9_7F4A_7C15),
    )
}

fn hash20(data: &[u8], salt: u64) -> String {
    let a = hash64(data, salt);
    let b = hash64(data, salt.wrapping_add(1));
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    bytes[..10].iter().map(|b| format!("{b:02x}")).collect()
}

fn hash64(data: &[u8], salt: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    salt.hash(&mut hasher);
    hasher.finish()
}

// ---------- `.mesh` binary (R3 §4) ----------

fn build_mesh_bytes(shape: &ExtractedShape) -> Vec<u8> {
    let num_verts = shape.positions.len() as u32;
    let indices: Vec<u16> = shape
        .triangles
        .iter()
        .flat_map(|t| [t[0] as u16, t[1] as u16, t[2] as u16])
        .collect();

    let mut max_abs = 0f32;
    for p in &shape.positions {
        for &c in p {
            max_abs = max_abs.max(c.abs());
        }
    }
    let scale = next_pow2_ge(max_abs);

    let mut out = Vec::new();
    out.extend_from_slice(&2u32.to_le_bytes()); // Version
    out.extend_from_slice(&(indices.len() as u32).to_le_bytes()); // Indices Size
    for idx in &indices {
        out.extend_from_slice(&idx.to_le_bytes());
    }
    out.extend_from_slice(&scale.to_le_bytes()); // Scale
    out.extend_from_slice(&0u32.to_le_bytes()); // Weights Per Vertex (static)
    out.extend_from_slice(&num_verts.to_le_bytes()); // Num Verts
    for p in &shape.positions {
        out.extend_from_slice(&encode_snorm16(p[0], scale).to_le_bytes());
        out.extend_from_slice(&encode_snorm16(p[1], scale).to_le_bytes());
        out.extend_from_slice(&encode_snorm16(p[2], scale).to_le_bytes());
    }

    // UV set 1
    out.extend_from_slice(&num_verts.to_le_bytes());
    for uv in &shape.uvs {
        out.extend_from_slice(&f32_to_half_bits(uv[0]).to_le_bytes());
        out.extend_from_slice(&f32_to_half_bits(uv[1]).to_le_bytes());
    }

    // UV set 2 — zero count, not omitted (R3 §5).
    out.extend_from_slice(&0u32.to_le_bytes());

    // Vertex colors — RGBA -> BGRA swizzle (R3 §4 row 13).
    if shape.has_vertex_colors {
        out.extend_from_slice(&num_verts.to_le_bytes());
        for c in &shape.vertex_colors {
            out.extend_from_slice(&[c[2], c[1], c[0], c[3]]);
        }
    } else {
        out.extend_from_slice(&0u32.to_le_bytes());
    }

    // Normals — DEC3N, w = 1 uniformly (R3 §4 row 15).
    out.extend_from_slice(&num_verts.to_le_bytes());
    for n in &shape.normals {
        out.extend_from_slice(&encode_dec3n(*n, 1).to_le_bytes());
    }

    // Tangents — DEC3N, w carries the per-vertex bitangent sign (R3 §4 row 17).
    out.extend_from_slice(&num_verts.to_le_bytes());
    for i in 0..shape.positions.len() {
        let w = if bitangent_sign_negative(shape.normals[i], shape.tangents[i], shape.bitangents[i])
        {
            3
        } else {
            0
        };
        out.extend_from_slice(&encode_dec3n(shape.tangents[i], w).to_le_bytes());
    }

    out.extend_from_slice(&0u32.to_le_bytes()); // Num Weights (static)
    out.extend_from_slice(&0u32.to_le_bytes()); // Num LODs
    out.extend_from_slice(&0u32.to_le_bytes()); // Num Meshlets
    out.extend_from_slice(&0u32.to_le_bytes()); // Num Cull Data

    out
}

/// Reads fields 1-6 of R3 §4 (`Version`, `Indices Size`, `Scale`, `Weights Per Vertex`,
/// `Num Verts`), skipping over the index buffer in between. Used to round-trip-verify
/// `build_mesh_bytes` output without depending on `crate::sf_mesh`.
pub fn parse_mesh_header(bytes: &[u8]) -> Result<(u32, u32, f32, u32, u32), String> {
    let mut cur = 0usize;
    let read_u32 = |cur: &mut usize| -> Result<u32, String> {
        let end = *cur + 4;
        let slice = bytes
            .get(*cur..end)
            .ok_or_else(|| "mesh: truncated header".to_string())?;
        *cur = end;
        Ok(u32::from_le_bytes(slice.try_into().unwrap()))
    };

    let version = read_u32(&mut cur)?;
    let indices_size = read_u32(&mut cur)?;
    cur = cur
        .checked_add(indices_size as usize * 2)
        .ok_or_else(|| "mesh: index count overflow".to_string())?;
    let scale = f32::from_bits(read_u32(&mut cur)?);
    let weights_per_vertex = read_u32(&mut cur)?;
    let num_verts = read_u32(&mut cur)?;

    Ok((version, indices_size, scale, weights_per_vertex, num_verts))
}

fn next_pow2_ge(x: f32) -> f32 {
    if !(x > 0.0) || !x.is_finite() {
        return 1.0;
    }
    let mut p = 1.0f32;
    while p < x {
        p *= 2.0;
    }
    p
}

/// Encode (R3 §4 row 7): `clamp(v/Scale, -1, 1)` then `round(x*32767)` for `x >= 0`,
/// `round(x*32768)` for `x < 0` — the asymmetric snorm rule vanilla actually uses (do not use
/// the symmetric `/32767.0` form `sf_mesh.rs` uses on read; see R3 §1a deviation (a)).
fn encode_snorm16(v: f32, scale: f32) -> i16 {
    let x = (v / scale).clamp(-1.0, 1.0);
    if x >= 0.0 {
        (x * 32767.0).round() as i16
    } else {
        (x * 32768.0).round() as i16
    }
}

#[cfg(test)]
fn decode_snorm16(q: i16, scale: f32) -> f32 {
    if q < 0 {
        (q as f32 / 32768.0) * scale
    } else {
        (q as f32 / 32767.0) * scale
    }
}

/// DEC3N 10:10:10:2 pack, `/511.5` basis (R3 §4 row 15 — not the `/511.0` basis `sf_mesh.rs`
/// uses on read; see R3 §1a deviation (b)).
fn encode_dec3n(v: [f32; 3], w: u32) -> u32 {
    let channel = |c: f32| -> u32 { (((c.clamp(-1.0, 1.0) + 1.0) * 511.5).round() as u32) & 0x3FF };
    let x = channel(v[0]);
    let y = channel(v[1]);
    let z = channel(v[2]);
    x | (y << 10) | (z << 20) | ((w & 0x3) << 30)
}

#[cfg(test)]
fn decode_dec3n(packed: u32) -> ([f32; 3], u32) {
    let x = (packed & 0x3FF) as f32 / 511.5 - 1.0;
    let y = ((packed >> 10) & 0x3FF) as f32 / 511.5 - 1.0;
    let z = ((packed >> 20) & 0x3FF) as f32 / 511.5 - 1.0;
    ([x, y, z], (packed >> 30) & 0x3)
}

fn bitangent_sign_negative(normal: [f32; 3], tangent: [f32; 3], bitangent: [f32; 3]) -> bool {
    let cross = [
        normal[1] * tangent[2] - normal[2] * tangent[1],
        normal[2] * tangent[0] - normal[0] * tangent[2],
        normal[0] * tangent[1] - normal[1] * tangent[0],
    ];
    let dot = cross[0] * bitangent[0] + cross[1] * bitangent[1] + cross[2] * bitangent[2];
    dot < 0.0
}

fn f32_to_half_bits(f: f32) -> u16 {
    let bits = f.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mant = bits & 0x007F_FFFF;

    if exp == 0xFF {
        let mant_h = if mant != 0 { 0x200 } else { 0 };
        return sign | 0x7C00 | mant_h;
    }

    let unbiased = exp - 127;
    if unbiased > 15 {
        return sign | 0x7C00; // overflow -> inf
    }
    if unbiased < -24 {
        return sign; // underflow -> 0
    }
    if unbiased < -14 {
        let shift = (-unbiased - 14) as u32;
        let mant_full = mant | 0x0080_0000;
        let shift_amt = 13 + shift;
        if shift_amt >= 32 {
            return sign;
        }
        let half_round = 1u32 << (shift_amt - 1);
        let m = (mant_full + half_round) >> shift_amt;
        return sign | (m as u16 & 0x3FF);
    }

    let exp_h = (unbiased + 15) as u16;
    let round_bit = 1u32 << 12;
    let rem = mant & 0x1FFF;
    let lsb = (mant >> 13) & 1;
    let mut mant_h = (mant >> 13) as u16;
    if rem > round_bit || (rem == round_bit && lsb == 1) {
        mant_h += 1;
        if mant_h == 0x400 {
            return sign | ((exp_h + 1) << 10);
        }
    }
    sign | (exp_h << 10) | mant_h
}

#[cfg(test)]
fn half_bits_to_f32(bits: u16) -> f32 {
    let sign = ((bits & 0x8000) as u32) << 16;
    let exp = ((bits & 0x7C00) >> 10) as i32;
    let mant = (bits & 0x03FF) as u32;

    if exp == 0 {
        if mant == 0 {
            return f32::from_bits(sign);
        }
        let mut mant = mant;
        let mut exp = 1i32;
        while mant & 0x0400 == 0 {
            mant <<= 1;
            exp -= 1;
        }
        mant &= 0x03FF;
        return f32::from_bits(sign | (((exp + 112) as u32) << 23) | (mant << 13));
    }
    if exp == 0x1F {
        return f32::from_bits(sign | 0x7F80_0000 | (mant << 13));
    }
    f32::from_bits(sign | (((exp - 15 + 127) as u32) << 23) | (mant << 13))
}

// ---------- collision (R7, via crate::starfield_collision) ----------

fn aabb_collision_prim(shapes: &[ExtractedShape]) -> CollisionPrim {
    const MIN_EXTENT: f32 = 0.01;

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for shape in shapes {
        for p in &shape.positions {
            let world = [
                p[0] + shape.translation[0],
                p[1] + shape.translation[1],
                p[2] + shape.translation[2],
            ];
            for k in 0..3 {
                if world[k] < min[k] {
                    min[k] = world[k];
                }
                if world[k] > max[k] {
                    max[k] = world[k];
                }
            }
        }
    }
    if !min[0].is_finite() {
        min = [0.0; 3];
        max = [0.0; 3];
    }

    let mut half_extents = [
        (max[0] - min[0]) * 0.5,
        (max[1] - min[1]) * 0.5,
        (max[2] - min[2]) * 0.5,
    ];
    for extent in half_extents.iter_mut() {
        if *extent < MIN_EXTENT {
            *extent = MIN_EXTENT;
        }
    }
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];

    CollisionPrim::Box {
        center,
        half_extents,
        rotation: [0.0, 0.0, 0.0, 1.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NifValue;
    use regex::Regex;

    /// Builds a minimal FO4 NIF (`NifFile::new("fo4")` + one `BSTriShape` child) via the
    /// crate's own model API — no fixture NIF file needed. A single flat triangle with a
    /// vertex placed at exactly `HAVOK_SCALE` on X, so the spatial-scale test can assert an
    /// exact post-conversion value.
    fn build_fo4_fixture() -> Vec<u8> {
        let mut nif = NifFile::new("fo4");

        let vertex = |x: f32, y: f32| -> NifValue {
            let mut fields = IndexMap::new();
            fields.insert("Vertex".to_string(), NifValue::Vec3([x, y, 0.0]));
            fields.insert("Bitangent X".to_string(), NifValue::Float(0.0));
            fields.insert("UV".to_string(), {
                let mut uv = IndexMap::new();
                uv.insert("u".to_string(), NifValue::Float((x / HAVOK_SCALE) as f64));
                uv.insert("v".to_string(), NifValue::Float((y / HAVOK_SCALE) as f64));
                NifValue::Struct(uv)
            });
            fields.insert("Normal".to_string(), NifValue::Vec3([0.0, 0.0, 1.0]));
            fields.insert("Bitangent Y".to_string(), NifValue::Float(1.0));
            fields.insert("Tangent".to_string(), NifValue::Vec3([1.0, 0.0, 0.0]));
            fields.insert("Bitangent Z".to_string(), NifValue::Float(0.0));
            NifValue::Struct(fields)
        };

        let vertex_data = NifValue::Array(vec![
            vertex(0.0, 0.0),
            vertex(HAVOK_SCALE, 0.0),
            vertex(0.0, HAVOK_SCALE),
        ]);
        let triangle = |a: i64, b: i64, c: i64| -> NifValue {
            let mut t = IndexMap::new();
            t.insert("v1".to_string(), NifValue::Int(a));
            t.insert("v2".to_string(), NifValue::Int(b));
            t.insert("v3".to_string(), NifValue::Int(c));
            NifValue::Struct(t)
        };

        let mut fields = IndexMap::new();
        fields.insert(
            "Name".to_string(),
            NifValue::String("TestShape".to_string()),
        );
        fields.insert("Flags".to_string(), NifValue::UInt(14));
        fields.insert("Translation".to_string(), NifValue::Vec3([0.0, 0.0, 0.0]));
        fields.insert("Rotation".to_string(), NifValue::Matrix33(IDENTITY_MAT3));
        fields.insert("Scale".to_string(), NifValue::Float(1.0));
        fields.insert("Collision Object".to_string(), NifValue::Ref(-1));
        fields.insert("Skin".to_string(), NifValue::Ref(-1));
        fields.insert("Shader Property".to_string(), NifValue::Ref(-1));
        fields.insert("Alpha Property".to_string(), NifValue::Ref(-1));
        // BSVertexDesc: bits 44+ select which attributes are present via each field's
        // `arg = "Vertex Desc >> 44"` schema condition. Full-precision Vertex (bit 0x400) +
        // VF_VERTEX(0x1) | VF_UVS(0x2) | VF_NORMALS(0x8) | VF_TANGENTS(0x10), matching
        // convert_file.rs's VF_* constants, so the fixture round-trips exact float positions
        // (no half-float rounding) through the FO4 NIF write/read the fixture itself does.
        const FULLPREC_VERTEX_DESC_FLAGS: i64 = 0x0001 | 0x0002 | 0x0008 | 0x0010 | 0x0400;
        fields.insert(
            "Vertex Desc".to_string(),
            NifValue::Int(FULLPREC_VERTEX_DESC_FLAGS << 44),
        );
        fields.insert("Num Triangles".to_string(), NifValue::UInt(1));
        fields.insert("Num Vertices".to_string(), NifValue::UInt(3));
        fields.insert("Data Size".to_string(), NifValue::UInt(1));
        fields.insert("Vertex Data".to_string(), vertex_data);
        fields.insert(
            "Triangles".to_string(),
            NifValue::Array(vec![triangle(0, 1, 2)]),
        );

        let shape_id = nif.add_block("BSTriShape", Some(fields));
        nif.blocks[0].set_field("Name", NifValue::String("TestRoot".to_string()));
        nif.blocks[0].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );
        nif.blocks[0].set_field("Num Children", NifValue::UInt(1));

        nif.rebuild_header();
        nif.header.footer_roots = vec![0];
        nif.to_bytes().expect("fixture NIF serializes")
    }

    fn stub_mat_path(_shape: &NifBlock) -> String {
        "materials/test/testmat.mat".to_string()
    }

    #[test]
    fn convert_produces_one_bsgeometry_per_shape_with_bs_version_175() {
        let fo4_bytes = build_fo4_fixture();
        let out = convert_fo4_nif_to_starfield(&fo4_bytes, &stub_mat_path, CollisionMode::None)
            .expect("conversion succeeds");

        let nif_out = NifFile::from_bytes(&out.nif_bytes, None).expect("output NIF parses");
        assert_eq!(nif_out.header.bs_version, 175);

        let geometry_count = nif_out
            .blocks
            .iter()
            .filter(|b| b.type_name == "BSGeometry")
            .count();
        assert_eq!(
            geometry_count, 1,
            "exactly one BSGeometry for one input shape"
        );
        assert_eq!(
            out.mesh_files.len(),
            1,
            "one .mesh file per unique geometry"
        );
    }

    #[test]
    fn mesh_path_matches_nif_reference_and_file_path() {
        let fo4_bytes = build_fo4_fixture();
        let out = convert_fo4_nif_to_starfield(&fo4_bytes, &stub_mat_path, CollisionMode::None)
            .expect("conversion succeeds");
        let nif_out = NifFile::from_bytes(&out.nif_bytes, None).expect("output NIF parses");

        let geometry = nif_out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSGeometry")
            .expect("has BSGeometry");
        let Some(NifValue::Array(slots)) = geometry.get_field("Meshes") else {
            panic!("Meshes field missing");
        };
        let NifValue::Struct(slot0) = &slots[0] else {
            panic!("slot 0 not a struct");
        };
        let NifValue::Struct(mesh) = slot0.get("Mesh").expect("slot 0 has Mesh") else {
            panic!("Mesh not a struct");
        };
        let NifValue::String(mesh_path) = mesh.get("Mesh Path").expect("Mesh Path present") else {
            panic!("Mesh Path not a string");
        };

        let re = Regex::new(r"^[0-9a-f]{20}\\[0-9a-f]{20}$").unwrap();
        assert!(
            re.is_match(mesh_path),
            "unexpected mesh path form: {mesh_path}"
        );

        let expected_file_path =
            format!("geometries/{}/{}.mesh", &mesh_path[..20], &mesh_path[21..]);
        assert_eq!(out.mesh_files[0].0, expected_file_path);

        for slot in &slots[1..] {
            let NifValue::Struct(s) = slot else {
                panic!("LOD slot not a struct");
            };
            assert_eq!(s.get("Has Mesh").map(NifValue::as_i64), Some(0));
        }
    }

    #[test]
    fn material_id_crc_matches_independent_computation() {
        let fo4_bytes = build_fo4_fixture();
        let out = convert_fo4_nif_to_starfield(&fo4_bytes, &stub_mat_path, CollisionMode::None)
            .expect("conversion succeeds");
        let nif_out = NifFile::from_bytes(&out.nif_bytes, None).expect("output NIF parses");

        let geometry = nif_out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSGeometry")
            .expect("has BSGeometry");
        let shader_ref = match geometry.get_field("Shader Property") {
            Some(NifValue::Ref(r)) => *r,
            _ => panic!("Shader Property missing"),
        };
        let shader = &nif_out.blocks[shader_ref as usize];
        let NifValue::String(mat_name) = shader.get_field("Name").expect("shader has Name") else {
            panic!("shader Name not a string");
        };
        assert_eq!(mat_name, "materials\\test\\testmat.mat");

        let extra_refs = match geometry.get_field("Extra Data List") {
            Some(NifValue::Array(items)) => items.clone(),
            _ => panic!("Extra Data List missing"),
        };
        let NifValue::Ref(extra_ref) = extra_refs[0] else {
            panic!("extra ref not a Ref");
        };
        let extra = &nif_out.blocks[extra_ref as usize];
        assert_eq!(extra.type_name, "NiIntegerExtraData");
        match extra.get_field("Name") {
            Some(NifValue::String(name)) => assert_eq!(name, "MaterialID"),
            other => panic!("expected NiIntegerExtraData Name string, got {other:?}"),
        }
        let stored_crc = extra
            .get_field("Integer Data")
            .map(NifValue::as_i64)
            .unwrap();

        // Independent computation (R3 §3): reflected CRC-32, poly 0xEDB88320, init 0, no
        // final XOR, over the lowercased backslash material path — same algorithm as
        // materials_native::cdb::bethesda_crc32, reimplemented here so the test does not
        // just call the same function under test.
        let expected = independent_crc32(b"materials\\test\\testmat.mat");
        assert_eq!(stored_crc as u32, expected);
        assert_eq!(
            expected,
            materials_native::cdb::bethesda_crc32(b"materials\\test\\testmat.mat")
        );
    }

    fn independent_crc32(data: &[u8]) -> u32 {
        let mut crc = 0u32;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        crc
    }

    #[test]
    fn positions_are_scaled_by_inverse_havok_scale() {
        let fo4_bytes = build_fo4_fixture();
        let out = convert_fo4_nif_to_starfield(&fo4_bytes, &stub_mat_path, CollisionMode::None)
            .expect("conversion succeeds");

        let mesh_bytes = &out.mesh_files[0].1;
        let (version, indices_size, scale, weights_per_vertex, num_verts) =
            parse_mesh_header(mesh_bytes).expect("mesh header parses");
        assert_eq!(version, 2);
        assert_eq!(indices_size, 3);
        assert_eq!(weights_per_vertex, 0);
        assert_eq!(num_verts, 3);

        // Decode the second vertex (source FO4 X = HAVOK_SCALE, Y = Z = 0) and confirm it
        // landed at exactly (1.0, 0.0, 0.0) — the fixture's known vertex, scaled by
        // FO4_TO_SF_SPATIAL == 1 / HAVOK_SCALE exactly once.
        let header_len = 4 + 4 + (indices_size as usize) * 2 + 4 + 4 + 4;
        let mut cur = header_len + 3 * 2; // skip vertex 0's 3 i16 components (6 bytes)
        let read_i16 = |bytes: &[u8], cur: &mut usize| -> i16 {
            let v = i16::from_le_bytes(bytes[*cur..*cur + 2].try_into().unwrap());
            *cur += 2;
            v
        };
        let qx = read_i16(mesh_bytes, &mut cur);
        let qy = read_i16(mesh_bytes, &mut cur);
        let qz = read_i16(mesh_bytes, &mut cur);
        let x = decode_snorm16(qx, scale);
        let y = decode_snorm16(qy, scale);
        let z = decode_snorm16(qz, scale);
        assert!((x - 1.0).abs() < 1e-4, "expected x == 1.0, got {x}");
        assert!(y.abs() < 1e-4, "expected y == 0.0, got {y}");
        assert!(z.abs() < 1e-4, "expected z == 0.0, got {z}");
    }

    #[test]
    fn parse_mesh_header_round_trips_build_mesh_bytes() {
        let shape = ExtractedShape {
            name: "Quad".to_string(),
            translation: [0.0, 0.0, 0.0],
            rotation: IDENTITY_MAT3,
            scale: 1.0,
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            tangents: vec![[1.0, 0.0, 0.0]; 4],
            bitangents: vec![[0.0, 1.0, 0.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            vertex_colors: vec![[255, 255, 255, 255]; 4],
            has_vertex_colors: false,
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            source_block: NifBlock::new(0, "BSTriShape"),
        };

        let mesh_bytes = build_mesh_bytes(&shape);
        let (version, indices_size, _scale, weights_per_vertex, num_verts) =
            parse_mesh_header(&mesh_bytes).expect("parses");
        assert_eq!(version, 2);
        assert_eq!(indices_size, shape.triangles.len() as u32 * 3);
        assert_eq!(weights_per_vertex, 0);
        assert_eq!(num_verts, shape.positions.len() as u32);
    }

    #[test]
    fn dec3n_round_trips_within_quantization_tolerance() {
        let v = [0.6, -0.8, 0.0f32];
        let packed = encode_dec3n(v, 3);
        let (decoded, w) = decode_dec3n(packed);
        assert_eq!(w, 3);
        for k in 0..3 {
            assert!(
                (decoded[k] - v[k]).abs() < 1e-2,
                "channel {k}: {} vs {}",
                decoded[k],
                v[k]
            );
        }
    }

    #[test]
    fn half_float_round_trips() {
        for v in [0.0f32, 1.0, -1.0, 0.5, 0.25, 3.75, -12.0] {
            let bits = f32_to_half_bits(v);
            let back = half_bits_to_f32(bits);
            assert!((back - v).abs() < 1e-3, "{v} round-tripped to {back}");
        }
    }

    #[test]
    fn vertex_colors_swizzle_rgba_to_bgra() {
        let shape = ExtractedShape {
            name: "Colored".to_string(),
            translation: [0.0, 0.0, 0.0],
            rotation: IDENTITY_MAT3,
            scale: 1.0,
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            tangents: vec![[1.0, 0.0, 0.0]; 3],
            bitangents: vec![[0.0, 1.0, 0.0]; 3],
            uvs: vec![[0.0, 0.0]; 3],
            vertex_colors: vec![[10, 20, 30, 40]; 3], // R,G,B,A
            has_vertex_colors: true,
            triangles: vec![[0, 1, 2]],
            source_block: NifBlock::new(0, "BSTriShape"),
        };
        let mesh_bytes = build_mesh_bytes(&shape);
        let (_v, indices_size, _s, _w, num_verts) = parse_mesh_header(&mesh_bytes).unwrap();

        // Walk past positions/UV1/UV2-count to the vertex-colors section.
        let header_len = 4 + 4 + (indices_size as usize) * 2 + 4 + 4 + 4;
        let positions_len = num_verts as usize * 3 * 2;
        let mut cur = header_len + positions_len;
        let num_uv1 = u32::from_le_bytes(mesh_bytes[cur..cur + 4].try_into().unwrap()) as usize;
        cur += 4 + num_uv1 * 4; // HalfTexCoord = 2 x f16
        let num_uv2 = u32::from_le_bytes(mesh_bytes[cur..cur + 4].try_into().unwrap()) as usize;
        assert_eq!(num_uv2, 0);
        cur += 4;
        let num_colors = u32::from_le_bytes(mesh_bytes[cur..cur + 4].try_into().unwrap()) as usize;
        assert_eq!(num_colors, 3);
        cur += 4;
        let first_color = &mesh_bytes[cur..cur + 4];
        assert_eq!(first_color, &[30, 20, 10, 40]); // BGRA
    }

    #[test]
    fn split_oversized_keeps_every_batch_within_the_limit() {
        // Two disjoint triangles sharing no vertices: 6 unique vertices total. With
        // max_verts = 4, the second triangle cannot fit in the first batch (4 + 3 > 4), so
        // it must start a new one — proving the split path actually runs without needing a
        // real 65536-vertex fixture.
        let shape = ExtractedShape {
            name: "Split".to_string(),
            translation: [0.0, 0.0, 0.0],
            rotation: IDENTITY_MAT3,
            scale: 1.0,
            positions: (0..6).map(|i| [i as f32, 0.0, 0.0]).collect(),
            normals: vec![[0.0, 0.0, 1.0]; 6],
            tangents: vec![[1.0, 0.0, 0.0]; 6],
            bitangents: vec![[0.0, 1.0, 0.0]; 6],
            uvs: vec![[0.0, 0.0]; 6],
            vertex_colors: vec![[255, 255, 255, 255]; 6],
            has_vertex_colors: false,
            triangles: vec![[0, 1, 2], [3, 4, 5]],
            source_block: NifBlock::new(0, "BSTriShape"),
        };

        let parts = split_oversized(shape, 4);
        assert_eq!(parts.len(), 2, "expected two batches");
        for part in &parts {
            assert!(part.positions.len() <= 4);
            assert_eq!(part.positions.len(), part.normals.len());
            assert_eq!(part.positions.len(), part.uvs.len());
        }
        let total_verts: usize = parts.iter().map(|p| p.positions.len()).sum();
        assert_eq!(
            total_verts, 6,
            "no vertices lost or duplicated across batches"
        );
        let total_tris: usize = parts.iter().map(|p| p.triangles.len()).sum();
        assert_eq!(total_tris, 2);
    }

    #[test]
    fn split_oversized_below_limit_is_a_no_op() {
        let shape = ExtractedShape {
            name: "Small".to_string(),
            translation: [0.0, 0.0, 0.0],
            rotation: IDENTITY_MAT3,
            scale: 1.0,
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            tangents: vec![[1.0, 0.0, 0.0]; 3],
            bitangents: vec![[0.0, 1.0, 0.0]; 3],
            uvs: vec![[0.0, 0.0]; 3],
            vertex_colors: vec![[255, 255, 255, 255]; 3],
            has_vertex_colors: false,
            triangles: vec![[0, 1, 2]],
            source_block: NifBlock::new(0, "BSTriShape"),
        };
        let parts = split_oversized(shape, 65535);
        assert_eq!(parts.len(), 1);
    }

    #[test]
    fn winding_is_carried_1to1_not_flipped() {
        let fo4_bytes = build_fo4_fixture();
        let out = convert_fo4_nif_to_starfield(&fo4_bytes, &stub_mat_path, CollisionMode::None)
            .expect("conversion succeeds");
        let mesh_bytes = &out.mesh_files[0].1;

        let indices_size = u32::from_le_bytes(mesh_bytes[4..8].try_into().unwrap()) as usize;
        let mut indices = Vec::with_capacity(indices_size);
        for i in 0..indices_size {
            let off = 8 + i * 2;
            indices.push(u16::from_le_bytes(
                mesh_bytes[off..off + 2].try_into().unwrap(),
            ));
        }
        // Fixture triangle is authored as (0, 1, 2) — R3 §4a: carry 1:1, no [t0, t2, t1] flip.
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[test]
    fn collision_none_emits_no_bhk_blocks_and_bsx_zero() {
        let fo4_bytes = build_fo4_fixture();
        let out = convert_fo4_nif_to_starfield(&fo4_bytes, &stub_mat_path, CollisionMode::None)
            .expect("conversion succeeds");
        let nif_out = NifFile::from_bytes(&out.nif_bytes, None).expect("output NIF parses");

        assert!(!nif_out.blocks.iter().any(|b| matches!(
            b.type_name.as_str(),
            "bhkNPCollisionObject" | "bhkPhysicsSystem"
        )));
        let bsx = nif_out
            .blocks
            .iter()
            .find(|b| b.type_name == "BSXFlags")
            .expect("root always carries a BSXFlags");
        assert_eq!(bsx.get_field("Integer Data").map(NifValue::as_i64), Some(0));
    }
}
