// .bto object-LOD block-graph writer via nif_core.
//
// build_bto_nif assembles the FO4 object-LOD NIF block graph in memory
// (one BSMultiBoundNode subtree per merged shape); write_bto saves it to disk.
//
// Block graph (verified against tmp/xlodgen/.../DLC03FarHarbor.16.-9.5.bto):
//   NiNode "obj"
//     └─ BSMultiBoundNode ""   (one per shape)
//          ├─ BSSubIndexTriShape "obj"|"obj-at"
//          │     ├─ BSLightingShaderProperty "" | BSEffectShaderProperty ""
//          │     │     └─ BSShaderTextureSet ""   (deduped across shapes)
//          │     └─ NiAlphaProperty ""            (alpha shapes only)
//          └─ BSMultiBound ""
//                └─ BSMultiBoundAABB ""
//
// This is the writer half of `objects::build_bto` (which produces the
// `BtoShape` list). build_bto_nif consumes that list and hands it to nif_core.
// Port: LODApp.CreateLODNodesFO4 (LODApp.cs:2455-2651) + GenerateMultibound
// (LODApp.cs:241-259), block-write side only.

use indexmap::IndexMap;
use nif_core_native::io::BTO_NUM_PRIMITIVES_OVERRIDE_FIELD;
use nif_core_native::model::{NifFile, NifValue};

use crate::objects::object_lod::{BtoShader, BtoShape, expand_segments};
use crate::settings::Fo76BtoNodeLayout;

/// The 10-slot BSShaderTextureSet for an object-LOD shape, `Data\`-prefixed.
/// Slots [0]=diffuse, [1]=normal, [7]=specular, [3]=greyscale palette.
fn texture_set_block(nif: &mut NifFile, textures: &[String; 10]) -> usize {
    let mut fields = IndexMap::new();
    fields.insert("Num Textures".to_string(), NifValue::UInt(10));
    let slots: Vec<NifValue> = textures
        .iter()
        .map(|t| NifValue::String(data_prefixed_texture_path(t)))
        .collect();
    fields.insert("Textures".to_string(), NifValue::Array(slots));
    nif.add_block("BSShaderTextureSet", Some(fields))
}

fn data_prefixed_texture_path(texture: &str) -> String {
    let mut path = texture.trim().replace('/', "\\");
    if path.is_empty() {
        return String::new();
    }

    if path.len() >= 5 && path[..5].eq_ignore_ascii_case("data\\") {
        path = path[5..].to_string();
    }
    if !path
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("textures\\"))
    {
        path = format!("Textures\\{path}");
    }
    format!("Data\\{path}")
}

/// Build a fresh BSLightingShaderProperty for a non-passthru object shape.
/// Constants from the verified golden DLC03FarHarbor.16.-9.5.bto (block 3).
/// Port: LODApp.cs:2544-2620.
fn lighting_shader_block(
    nif: &mut NifFile,
    texture_set_ref: i32,
    flags1: u32,
    flags2: u32,
    clamp: u32,
    backlight: Option<f32>,
    grayscale_scale: Option<f32>,
) -> usize {
    let mut fields = IndexMap::new();
    // Shader Type 0 = "Default". Store the NUMERIC enum value, not the name string:
    // nif_core's condition evaluator coerces a String-valued enum to a truthy bool,
    // so a name string makes `cond="Shader Type == 1"` spuriously match and emits the
    // 6-byte `Shader Type == 1` tail (Environment Map Scale + 2 SSR bools), inflating
    // this BSLightingShaderProperty past the engine-expected size and desyncing FO4's
    // .bto parser (same defect as btr.rs). UInt(0) evaluates `0 == 1` correctly and
    // serializes byte-identically to the golden LODGen .bto.
    fields.insert("Shader Type".to_string(), NifValue::UInt(0));
    fields.insert("Name".to_string(), NifValue::String(String::new()));
    fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
    fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
    fields.insert("Controller".to_string(), NifValue::Ref(-1));
    // Version-suffixed bitflag fields (BS Version 130 == FO4). Inserting under
    // the bare name is silently dropped on write (see btr.rs / Phase-1 lesson).
    fields.insert(
        "Shader Flags 1:FO4".to_string(),
        NifValue::UInt(flags1 as u64),
    );
    fields.insert(
        "Shader Flags 2:FO4".to_string(),
        NifValue::UInt(flags2 as u64),
    );
    fields.insert(
        "UV Offset".to_string(),
        NifValue::Struct({
            let mut m = IndexMap::new();
            m.insert("u".to_string(), NifValue::Float(0.0));
            m.insert("v".to_string(), NifValue::Float(0.0));
            m
        }),
    );
    fields.insert(
        "UV Scale".to_string(),
        NifValue::Struct({
            let mut m = IndexMap::new();
            m.insert("u".to_string(), NifValue::Float(1.0));
            m.insert("v".to_string(), NifValue::Float(1.0));
            m
        }),
    );
    fields.insert("Texture Set".to_string(), NifValue::Ref(texture_set_ref));
    fields.insert(
        "Emissive Color".to_string(),
        NifValue::Color3([0.0, 0.0, 0.0]),
    );
    fields.insert("Emissive Multiple".to_string(), NifValue::Float(1.0));
    fields.insert("Root Material".to_string(), NifValue::String(String::new()));
    // clamp mode → enum string (0 = CLAMP_S_CLAMP_T, 3 = WRAP_S_WRAP_T).
    fields.insert(
        "Texture Clamp Mode".to_string(),
        NifValue::String(clamp_mode_name(clamp).to_string()),
    );
    fields.insert("Alpha".to_string(), NifValue::Float(1.0));
    fields.insert("Refraction Strength".to_string(), NifValue::Float(0.0));
    // Smoothness == glossiness; CreateLODNodesFO4 SetGlossiness(1f).
    fields.insert("Smoothness".to_string(), NifValue::Float(1.0));
    fields.insert(
        "Specular Color".to_string(),
        NifValue::Color3([1.0, 1.0, 1.0]),
    );
    fields.insert("Specular Strength".to_string(), NifValue::Float(1.0));
    fields.insert("Subsurface Rolloff".to_string(), NifValue::Float(0.0));
    fields.insert(
        "Rimlight Power".to_string(),
        NifValue::Float(f32::MAX as f64),
    );
    fields.insert(
        "Backlight Power".to_string(),
        NifValue::Float(backlight.unwrap_or(0.0) as f64),
    );
    fields.insert(
        "Grayscale to Palette Scale".to_string(),
        NifValue::Float(grayscale_scale.unwrap_or(1.0) as f64),
    );
    fields.insert("Fresnel Power".to_string(), NifValue::Float(5.0));
    fields.insert(
        "Wetness".to_string(),
        NifValue::Struct({
            let mut m = IndexMap::new();
            m.insert("Spec Scale".to_string(), NifValue::Float(-1.0));
            m.insert("Spec Power".to_string(), NifValue::Float(-1.0));
            m.insert("Min Var".to_string(), NifValue::Float(-1.0));
            m.insert("Env Map Scale".to_string(), NifValue::Float(-1.0));
            m.insert("Fresnel Power".to_string(), NifValue::Float(-1.0));
            m.insert("Metalness".to_string(), NifValue::Float(-1.0));
            m
        }),
    );
    nif.add_block("BSLightingShaderProperty", Some(fields))
}

fn clamp_mode_name(clamp: u32) -> &'static str {
    match clamp {
        0 => "CLAMP_S_CLAMP_T",
        1 => "CLAMP_S_WRAP_T",
        2 => "WRAP_S_CLAMP_T",
        _ => "WRAP_S_WRAP_T",
    }
}

/// One BSSITS vertex-data struct from the per-shape geometry, matching the
/// object vertex desc (half pos + half Bitangent X + half UV + normbyte
/// Normal/BitanY + normbyte Tangent/BitanZ [+ byte Vertex Colors]).
/// The nif_core writer reads the layout from Vertex Desc >> 44.
fn vertex_struct(
    pos: [f32; 3],
    uv: [f32; 2],
    normal: [f32; 3],
    tangent: [f32; 3],
    bitangent: [f32; 3],
    color: Option<[f32; 4]>,
) -> NifValue {
    let mut data = IndexMap::new();
    data.insert("Vertex".to_string(), NifValue::Vec3(pos));
    // Bitangent is split across three half/normbyte components in the layout.
    data.insert(
        "Bitangent X".to_string(),
        NifValue::Float(bitangent[0] as f64),
    );
    let mut tex = IndexMap::new();
    tex.insert("u".to_string(), NifValue::Float(uv[0] as f64));
    tex.insert("v".to_string(), NifValue::Float(uv[1] as f64));
    data.insert("UV".to_string(), NifValue::Struct(tex));
    data.insert("Normal".to_string(), NifValue::Vec3(normal));
    data.insert(
        "Bitangent Y".to_string(),
        NifValue::Float(bitangent[1] as f64),
    );
    data.insert("Tangent".to_string(), NifValue::Vec3(tangent));
    data.insert(
        "Bitangent Z".to_string(),
        NifValue::Float(bitangent[2] as f64),
    );
    if let Some(c) = color {
        data.insert("Vertex Colors".to_string(), NifValue::Color4(c));
    }
    NifValue::Struct(data)
}

/// Compute the BSVertexDesc u64 for an object-LOD shape.
/// Half-precision position + UV + Normal + Tangent, optional Vertex Colors.
/// Verified: no-color desc == 474989027590661 (0x1b00000430205).
/// Port: Geometry.ToBSSubIndexTriShape vertex-desc assembly.
pub fn object_vertex_desc(has_colors: bool) -> u64 {
    if has_colors {
        // attributes 0x3b = Vertex|UVs|Normals|Tangents|Vertex_Colors,
        // vertexSize 6 words (24 bytes), color offset at word 5.
        // nibbles: size=6, uv1=2, normal=3, tangent=4, color=5.
        // 0x3b << 44 | 0x500000430206
        (0x3bu64 << 44) | 0x0000_0500_0043_0206
    } else {
        // Verified golden value.
        474989027590661
    }
}

fn new_bto_nif_with_root() -> (NifFile, usize) {
    let mut nif = NifFile::new("fo4");
    // Remove the default BSFadeNode root.
    nif.blocks.clear();
    nif.header.num_blocks = 0;
    nif.header.block_type_names.clear();
    nif.header.block_type_index.clear();
    nif.header.block_sizes.clear();

    // Root NiNode "obj".
    let root_id = {
        let mut fields = IndexMap::new();
        fields.insert("Name".to_string(), NifValue::String("obj".to_string()));
        fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
        fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
        fields.insert("Controller".to_string(), NifValue::Ref(-1));
        fields.insert("Flags".to_string(), NifValue::UInt(14));
        fields.insert("Translation".to_string(), NifValue::Vec3([0.0, 0.0, 0.0]));
        fields.insert(
            "Rotation".to_string(),
            NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        );
        fields.insert("Scale".to_string(), NifValue::Float(1.0));
        fields.insert("Collision Object".to_string(), NifValue::Ref(-1));
        fields.insert("Num Children".to_string(), NifValue::UInt(0));
        fields.insert("Children".to_string(), NifValue::Array(Vec::new()));
        nif.add_block("NiNode", Some(fields))
    };

    (nif, root_id)
}

/// Assemble the full `.bto` NIF in memory from the prepared shape list.
/// Port: LODApp.CreateLODNodesFO4 (LODApp.cs:2455-2651), write side.
pub fn build_bto_nif(shapes: &[BtoShape]) -> anyhow::Result<NifFile> {
    build_bto_nif_with_layout(shapes, Fo76BtoNodeLayout::Fo4PerShape)
}

pub fn build_bto_nif_with_layout(
    shapes: &[BtoShape],
    layout: Fo76BtoNodeLayout,
) -> anyhow::Result<NifFile> {
    match layout {
        Fo76BtoNodeLayout::Fo4PerShape => build_bto_nif_fo4_per_shape(shapes),
        Fo76BtoNodeLayout::Fo76Grouped => build_bto_nif_fo76_grouped(shapes),
    }
}

fn build_bto_nif_fo4_per_shape(shapes: &[BtoShape]) -> anyhow::Result<NifFile> {
    let (mut nif, root_id) = new_bto_nif_with_root();

    // Texture-set dedup map keyed by the 10-slot texture array (insertion order
    // preserved — port: quad.textureBlockIndex hash dedup, LODApp.cs:2552-2580).
    let mut texset_cache: IndexMap<[String; 10], usize> = IndexMap::new();
    let mut mbn_children: Vec<NifValue> = Vec::new();

    for shape in shapes {
        // --- BSMultiBoundNode (child subtree root) ---
        let mbn_id = {
            let mut fields = IndexMap::new();
            fields.insert("Name".to_string(), NifValue::String(String::new()));
            fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
            fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
            fields.insert("Controller".to_string(), NifValue::Ref(-1));
            fields.insert("Flags".to_string(), NifValue::UInt(14));
            fields.insert("Translation".to_string(), NifValue::Vec3([0.0, 0.0, 0.0]));
            fields.insert(
                "Rotation".to_string(),
                NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
            );
            fields.insert("Scale".to_string(), NifValue::Float(1.0));
            fields.insert("Collision Object".to_string(), NifValue::Ref(-1));
            fields.insert("Num Children".to_string(), NifValue::UInt(0));
            fields.insert("Children".to_string(), NifValue::Array(Vec::new()));
            fields.insert(
                "Culling Mode".to_string(),
                NifValue::String("CULL_ALLPASS".to_string()),
            );
            nif.add_block("BSMultiBoundNode", Some(fields))
        };
        mbn_children.push(NifValue::Ref(mbn_id as i32));

        // --- Shader property (+ texture set, deduped) ---
        let (shader_id, alpha_id) = build_shape_properties(&mut nif, shape, &mut texset_cache);

        // --- ToggleRefID NiIntegerExtraData (enable-parent refs) ---
        // Port: LODApp.cs:2485-2491.
        let extra_data_id = if shape.enable_parent != 0 {
            let mut fields = IndexMap::new();
            fields.insert(
                "Name".to_string(),
                NifValue::String("ToggleRefID".to_string()),
            );
            fields.insert(
                "Integer Data".to_string(),
                NifValue::UInt(shape.enable_parent as u64),
            );
            Some(nif.add_block("NiIntegerExtraData", Some(fields)))
        } else {
            None
        };

        // --- BSSubIndexTriShape ---
        let shape_id = build_subindex_trishape(&mut nif, shape, shader_id, alpha_id, extra_data_id);

        // --- BSMultiBound → BSMultiBoundAABB ---
        let aabb_id = {
            let mut fields = IndexMap::new();
            fields.insert(
                "Position".to_string(),
                NifValue::Vec3(shape.multibound.position),
            );
            fields.insert(
                "Extent".to_string(),
                NifValue::Vec3(shape.multibound.extent),
            );
            nif.add_block("BSMultiBoundAABB", Some(fields))
        };
        let mb_id = {
            let mut fields = IndexMap::new();
            fields.insert("Data".to_string(), NifValue::Ref(aabb_id as i32));
            nif.add_block("BSMultiBound", Some(fields))
        };

        // Wire up the BSMultiBoundNode children + multibound ref.
        let mbn = &mut nif.blocks[mbn_id];
        mbn.fields
            .insert("Num Children".to_string(), NifValue::UInt(1));
        mbn.fields.insert(
            "Children".to_string(),
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );
        mbn.fields
            .insert("Multi Bound".to_string(), NifValue::Ref(mb_id as i32));
    }

    // Wire the root NiNode children.
    let n = mbn_children.len();
    let root = &mut nif.blocks[root_id];
    root.fields
        .insert("Num Children".to_string(), NifValue::UInt(n as u64));
    root.fields
        .insert("Children".to_string(), NifValue::Array(mbn_children));

    nif.rebuild_header();
    nif.header.footer_roots = vec![root_id as i32];

    Ok(nif)
}

fn build_bto_nif_fo76_grouped(shapes: &[BtoShape]) -> anyhow::Result<NifFile> {
    let (mut nif, root_id) = new_bto_nif_with_root();
    let mut texset_cache: IndexMap<[String; 10], usize> = IndexMap::new();

    let mbn_id = {
        let mut fields = IndexMap::new();
        fields.insert("Name".to_string(), NifValue::String(String::new()));
        fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
        fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
        fields.insert("Controller".to_string(), NifValue::Ref(-1));
        fields.insert("Flags".to_string(), NifValue::UInt(14));
        fields.insert("Translation".to_string(), NifValue::Vec3([0.0, 0.0, 0.0]));
        fields.insert(
            "Rotation".to_string(),
            NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        );
        fields.insert("Scale".to_string(), NifValue::Float(1.0));
        fields.insert("Collision Object".to_string(), NifValue::Ref(-1));
        fields.insert("Num Children".to_string(), NifValue::UInt(0));
        fields.insert("Children".to_string(), NifValue::Array(Vec::new()));
        fields.insert(
            "Culling Mode".to_string(),
            NifValue::String("CULL_ALLPASS".to_string()),
        );
        nif.add_block("BSMultiBoundNode", Some(fields))
    };

    let mut children = Vec::with_capacity(shapes.len());
    for shape in shapes {
        let (shader_id, alpha_id) = build_shape_properties(&mut nif, shape, &mut texset_cache);
        let extra_data_id = if shape.enable_parent != 0 {
            let mut fields = IndexMap::new();
            fields.insert(
                "Name".to_string(),
                NifValue::String("ToggleRefID".to_string()),
            );
            fields.insert(
                "Integer Data".to_string(),
                NifValue::UInt(shape.enable_parent as u64),
            );
            Some(nif.add_block("NiIntegerExtraData", Some(fields)))
        } else {
            None
        };
        let shape_id = build_subindex_trishape(&mut nif, shape, shader_id, alpha_id, extra_data_id);
        children.push(NifValue::Ref(shape_id as i32));
    }

    let multibound = combined_multibound(shapes);
    let aabb_id = {
        let mut fields = IndexMap::new();
        fields.insert("Position".to_string(), NifValue::Vec3(multibound.position));
        fields.insert("Extent".to_string(), NifValue::Vec3(multibound.extent));
        nif.add_block("BSMultiBoundAABB", Some(fields))
    };
    let mb_id = {
        let mut fields = IndexMap::new();
        fields.insert("Data".to_string(), NifValue::Ref(aabb_id as i32));
        nif.add_block("BSMultiBound", Some(fields))
    };

    let mbn = &mut nif.blocks[mbn_id];
    mbn.fields.insert(
        "Num Children".to_string(),
        NifValue::UInt(children.len() as u64),
    );
    mbn.fields
        .insert("Children".to_string(), NifValue::Array(children));
    mbn.fields
        .insert("Multi Bound".to_string(), NifValue::Ref(mb_id as i32));

    let root = &mut nif.blocks[root_id];
    root.fields
        .insert("Num Children".to_string(), NifValue::UInt(1));
    root.fields.insert(
        "Children".to_string(),
        NifValue::Array(vec![NifValue::Ref(mbn_id as i32)]),
    );

    nif.rebuild_header();
    nif.header.footer_roots = vec![root_id as i32];

    Ok(nif)
}

fn combined_multibound(shapes: &[BtoShape]) -> crate::objects::object_lod::MultiBoundAabb {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for shape in shapes {
        for i in 0..3 {
            min[i] = min[i].min(shape.multibound.position[i] - shape.multibound.extent[i]);
            max[i] = max[i].max(shape.multibound.position[i] + shape.multibound.extent[i]);
        }
    }

    if !min[0].is_finite() || !max[0].is_finite() {
        return crate::objects::object_lod::MultiBoundAabb {
            position: [0.0, 0.0, 0.0],
            extent: [0.0, 0.0, 0.0],
        };
    }

    crate::objects::object_lod::MultiBoundAabb {
        position: [
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        ],
        extent: [
            (max[0] - min[0]) * 0.5,
            (max[1] - min[1]) * 0.5,
            (max[2] - min[2]) * 0.5,
        ],
    }
}

/// Build the shader property (and texture set / alpha property) for one shape.
/// Returns (shader_block_id, alpha_block_id?). Port: LODApp.cs:2492-2647.
fn build_shape_properties(
    nif: &mut NifFile,
    shape: &BtoShape,
    texset_cache: &mut IndexMap<[String; 10], usize>,
) -> (usize, Option<usize>) {
    let shader_id = match &shape.shader {
        BtoShader::Lighting {
            texture_set,
            flags1,
            flags2,
            clamp,
            backlight,
            grayscale_scale,
        } => {
            // Texture-set dedup (port: quad.textureBlockIndex, LODApp.cs:2552-2580).
            let texset_id = if let Some(&id) = texset_cache.get(texture_set) {
                id as i32
            } else {
                let id = texture_set_block(nif, texture_set);
                texset_cache.insert(texture_set.clone(), id);
                id as i32
            };
            lighting_shader_block(
                nif,
                texset_id,
                *flags1,
                *flags2,
                *clamp,
                *backlight,
                *grayscale_scale,
            )
        }
    };

    let alpha_id = shape.alpha.as_ref().map(|a| {
        let mut fields = IndexMap::new();
        fields.insert("Flags".to_string(), NifValue::UInt(a.flags as u64));
        fields.insert("Threshold".to_string(), NifValue::UInt(a.threshold as u64));
        nif.add_block("NiAlphaProperty", Some(fields))
    });

    (shader_id, alpha_id)
}

/// Build the BSSubIndexTriShape block for one shape.
/// Port: Geometry.ToBSSubIndexTriShape + CreateLODNodesFO4 (LODApp.cs:2461-2483).
fn build_subindex_trishape(
    nif: &mut NifFile,
    shape: &BtoShape,
    shader_id: usize,
    alpha_id: Option<usize>,
    extra_data_id: Option<usize>,
) -> usize {
    let g = &shape.geometry;
    let nv = g.vertices.len();
    let nt = g.triangles.len();
    let has_colors = !g.vertex_colors.is_empty();

    let vertex_data: Vec<NifValue> = (0..nv)
        .map(|i| {
            let pos = g.vertices[i];
            let uv = g.uvcoords.get(i).copied().unwrap_or([0.0, 0.0]);
            let normal = g.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
            let tangent = g.tangents.get(i).copied().unwrap_or([1.0, 0.0, 0.0]);
            let bitangent = g.bitangents.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
            let color = if has_colors {
                Some(
                    g.vertex_colors
                        .get(i)
                        .copied()
                        .unwrap_or([1.0, 1.0, 1.0, 1.0]),
                )
            } else {
                None
            };
            vertex_struct(pos, uv, normal, tangent, bitangent, color)
        })
        .collect();

    let triangle_data: Vec<NifValue> = g
        .triangles
        .iter()
        .map(|t| {
            let mut d = IndexMap::new();
            d.insert("v1".to_string(), NifValue::Int(t[0] as i64));
            d.insert("v2".to_string(), NifValue::Int(t[1] as i64));
            d.insert("v3".to_string(), NifValue::Int(t[2] as i64));
            NifValue::Struct(d)
        })
        .collect();

    // Expand the per-shape segments to the level grid and trim trailing zeros.
    // Port: BSSubIndexTriShape.SetSegments (BSSubIndexTriShape.cs:160-183).
    let expanded = expand_segments(&shape.segments, shape.segment_count);
    let num_segments = expanded.len() as u64;
    let segment_data: Vec<NifValue> = expanded
        .iter()
        .map(|s| {
            let mut d = IndexMap::new();
            d.insert(
                "Start Index".to_string(),
                NifValue::UInt((s.start_triangle as u64) * 3),
            );
            d.insert(
                "Num Primitives".to_string(),
                NifValue::UInt(s.num_triangles as u64),
            );
            // 0xFFFFFFFF sentinel (no parent) — matches golden.
            d.insert(
                "Parent Array Index".to_string(),
                NifValue::UInt(0xFFFF_FFFF),
            );
            d.insert("Num Sub Segments".to_string(), NifValue::UInt(0));
            d.insert("Sub Segment".to_string(), NifValue::Array(Vec::new()));
            NifValue::Struct(d)
        })
        .collect();

    let mut fields = IndexMap::new();
    let name = if shape.name_index_at { "obj-at" } else { "obj" };
    fields.insert("Name".to_string(), NifValue::String(name.to_string()));
    match extra_data_id {
        Some(id) => {
            fields.insert("Num Extra Data List".to_string(), NifValue::UInt(1));
            fields.insert(
                "Extra Data List".to_string(),
                NifValue::Array(vec![NifValue::Ref(id as i32)]),
            );
        }
        None => {
            fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
            fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
        }
    }
    fields.insert("Controller".to_string(), NifValue::Ref(-1));
    fields.insert("Flags".to_string(), NifValue::UInt(14));
    fields.insert("Flags2".to_string(), NifValue::UInt(0));
    fields.insert("Translation".to_string(), NifValue::Vec3(shape.translation));
    fields.insert(
        "Rotation".to_string(),
        NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
    );
    fields.insert("Scale".to_string(), NifValue::Float(shape.scale as f64));
    fields.insert("Collision Object".to_string(), NifValue::Ref(-1));
    fields.insert(
        "Bounding Sphere".to_string(),
        NifValue::Struct({
            let mut m = IndexMap::new();
            m.insert("Center".to_string(), NifValue::Vec3(shape.center));
            m.insert("Radius".to_string(), NifValue::Float(shape.radius as f64));
            m
        }),
    );
    fields.insert("Skin".to_string(), NifValue::Ref(-1));
    fields.insert(
        "Shader Property".to_string(),
        NifValue::Ref(shader_id as i32),
    );
    fields.insert(
        "Alpha Property".to_string(),
        NifValue::Ref(alpha_id.map(|a| a as i32).unwrap_or(-1)),
    );
    fields.insert(
        "Vertex Desc".to_string(),
        NifValue::UInt(object_vertex_desc(has_colors)),
    );
    fields.insert("Num Triangles".to_string(), NifValue::UInt(nt as u64));
    fields.insert("Num Vertices".to_string(), NifValue::UInt(nv as u64));
    fields.insert("Data Size".to_string(), NifValue::UInt(0)); // calc'd by writer
    fields.insert("Vertex Data".to_string(), NifValue::Array(vertex_data));
    fields.insert("Triangles".to_string(), NifValue::Array(triangle_data));
    fields.insert(
        BTO_NUM_PRIMITIVES_OVERRIDE_FIELD.to_string(),
        NifValue::UInt((nt * 2) as u64),
    );
    fields.insert("Num Segments".to_string(), NifValue::UInt(num_segments));
    fields.insert("Total Segments".to_string(), NifValue::UInt(num_segments));
    fields.insert("Segment".to_string(), NifValue::Array(segment_data));

    nif.add_block("BSSubIndexTriShape", Some(fields))
}

/// Write a `.bto` object-LOD mesh to disk.
pub fn write_bto(path: &std::path::Path, shapes: &[BtoShape]) -> anyhow::Result<()> {
    let mut nif = build_bto_nif(shapes)?;
    write_bto_nif(path, &mut nif)
}

pub fn write_bto_with_layout(
    path: &std::path::Path,
    shapes: &[BtoShape],
    layout: Fo76BtoNodeLayout,
) -> anyhow::Result<()> {
    let mut nif = build_bto_nif_with_layout(shapes, layout)?;
    write_bto_nif(path, &mut nif)
}

fn write_bto_nif(path: &std::path::Path, nif: &mut NifFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    nif.save(Some(path.to_path_buf()))
        .map_err(|e| anyhow::anyhow!("nif save failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::geometry::LodGeometry;
    use crate::objects::object_lod::{MultiBoundAabb, SegmentDesc};

    fn tex_slot(block: &nif_core_native::model::NifBlock, index: usize) -> String {
        match block.get_field("Textures") {
            Some(NifValue::Array(items)) => match &items[index] {
                NifValue::String(s) => s.clone(),
                other => panic!("texture slot {index} is not a string: {other:?}"),
            },
            other => panic!("missing texture array: {other:?}"),
        }
    }

    #[test]
    fn texture_set_block_prefixes_bare_texture_paths() {
        let mut nif = NifFile::new("fo4");
        let mut textures: [String; 10] = Default::default();
        textures[0] = "setdressing/metalofficelamp01_d.dds".to_string();
        textures[1] = r"Data\SetDressing\MetalOfficeLamp01_n.dds".to_string();
        textures[7] = r"Textures\Terrain\APPALACHIA\Objects\APPALACHIA.Objects_s.dds".to_string();

        let block_id = texture_set_block(&mut nif, &textures);
        let block = &nif.blocks[block_id];

        assert_eq!(
            tex_slot(block, 0),
            r"Data\Textures\setdressing\metalofficelamp01_d.dds"
        );
        assert_eq!(
            tex_slot(block, 1),
            r"Data\Textures\SetDressing\MetalOfficeLamp01_n.dds"
        );
        assert_eq!(
            tex_slot(block, 7),
            r"Data\Textures\Terrain\APPALACHIA\Objects\APPALACHIA.Objects_s.dds"
        );
        assert_eq!(tex_slot(block, 2), "");
    }

    fn segment_start(block: &nif_core_native::model::NifBlock, index: usize) -> i64 {
        match block.get_field("Segment") {
            Some(NifValue::Array(items)) => match &items[index] {
                NifValue::Struct(fields) => fields
                    .get("Start Index")
                    .map(NifValue::as_i64)
                    .expect("segment start index"),
                other => panic!("segment {index} is not a struct: {other:?}"),
            },
            other => panic!("missing segment array: {other:?}"),
        }
    }

    #[test]
    fn subindex_segment_start_index_is_index_buffer_offset() {
        let mut geometry = LodGeometry::new();
        geometry.vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
        ];
        geometry.uvcoords = vec![[0.0, 0.0]; 6];
        geometry.normals = vec![[0.0, 0.0, 1.0]; 6];
        geometry.tangents = vec![[1.0, 0.0, 0.0]; 6];
        geometry.bitangents = vec![[0.0, 1.0, 0.0]; 6];
        geometry.triangles = vec![[0, 1, 2], [1, 3, 2], [1, 4, 3], [4, 5, 3]];
        geometry.update_bbox();

        let shape = BtoShape {
            geometry,
            segments: vec![
                SegmentDesc {
                    id: 0,
                    start_triangle: 0,
                    num_triangles: 2,
                },
                SegmentDesc {
                    id: 1,
                    start_triangle: 2,
                    num_triangles: 2,
                },
            ],
            segment_count: 2,
            translation: [0.0, 0.0, 0.0],
            scale: 4.0,
            center: [1.0, 0.5, 0.0],
            radius: 1.0,
            multibound: MultiBoundAabb {
                position: [2.0, 2.0, 0.0],
                extent: [2.0, 2.0, 1.0],
            },
            name_index_at: false,
            shader: BtoShader::Lighting {
                texture_set: Default::default(),
                flags1: 0x8040_0001,
                flags2: 5,
                clamp: 0,
                backlight: None,
                grayscale_scale: None,
            },
            alpha: None,
            enable_parent: 0,
        };

        let mut nif = build_bto_nif(&[shape]).expect("build bto nif");
        let bytes = nif.to_bytes().expect("serialize bto nif");
        let reloaded = NifFile::from_bytes(&bytes, None).expect("reload bto nif");
        let sits = reloaded
            .blocks
            .iter()
            .find(|b| b.type_name == "BSSubIndexTriShape")
            .expect("BSSubIndexTriShape");

        assert_eq!(segment_start(sits, 0), 0);
        assert_eq!(segment_start(sits, 1), 6);
    }
}
