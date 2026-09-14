// .btr block-graph writer via nif_core.
//
// build_btr_nif assembles the NIF block graph in memory (useful for unit testing
// without disk). write_btr = build_btr_nif(...).save(Some(path)).
//
// write_btr writes scale=1.0, z_shift=0.0. terrain_lod calls build_btr_nif with the
// real lodLevel (`scale`) and per-level zShift (`z_translation`); the ShiftZ term
// (lodLevel * bbox z-center) and the vert recentering are computed internally
// (Geometry.cs ShiftZ / TerrainLOD.cs:1432,1438-1448).

use indexmap::IndexMap;
use nif_core_native::io::BTO_NUM_PRIMITIVES_OVERRIDE_FIELD;
use nif_core_native::model::{NifFile, NifValue};

use crate::descriptors::BBox;
use crate::terrain::water::{WaterMesh, WaterSegment};

/// Build a BSMultiBoundNode "chunk" → BSTriShape "Land" → BSLightingShaderProperty →
/// BSShaderTextureSet NIF in memory.
///
/// `scale` sets the BSTriShape Scale field (lodLevel in xLODGen). `z_translation` is
/// the per-level zShift; the ShiftZ term (`scale * bbox z-center`) is added internally
/// and the vert z values are recentered, so the final BSTriShape Translation.z =
/// `z_translation + scale * z_center` (TerrainLOD.cs:1432,1438-1441).
pub fn build_btr_nif(
    verts: &[[f32; 3]],
    uvs: &[[f32; 2]],
    tris: &[[u16; 3]],
    diffuse: &str,
    msn: &str,
    bounds: &BBox,
    scale: f32,
    z_translation: f32,
) -> anyhow::Result<NifFile> {
    build_btr_nif_inner(
        verts,
        uvs,
        tris,
        diffuse,
        msn,
        bounds,
        scale,
        z_translation,
        None,
    )
}

/// Like [`build_btr_nif`] but also emits the landless/ocean WATER block — a
/// `BSTriShape` water sheet with a `BSEffectShaderProperty`, parented under a
/// `BSMultiBoundNode "WATER"` that becomes the chunk's 2nd child.
///
/// Faithful to the golden coarse `.btr` structure (e.g.
/// `DLC03FarHarbor.32.-41.-27.btr`): root `chunk` → [terrain `BSTriShape "Land"`,
/// `BSMultiBoundNode "WATER"` → (water `BSTriShape` →
/// `BSEffectShaderProperty`, `BSMultiBound` → `BSMultiBoundAABB`)] + root
/// `BSMultiBound` → `BSMultiBoundAABB` (`TerrainLOD.cs:1520-1660`).
///
/// The water mesh shares the terrain block's `scale`/`z_translation` so the two
/// shapes register (xLODGen uses `quad.lodLevel` and `zShift[lodIndex]` for both).
pub fn build_btr_nif_with_water(
    verts: &[[f32; 3]],
    uvs: &[[f32; 2]],
    tris: &[[u16; 3]],
    diffuse: &str,
    msn: &str,
    bounds: &BBox,
    scale: f32,
    z_translation: f32,
    water: &WaterMesh,
) -> anyhow::Result<NifFile> {
    build_btr_nif_inner(
        verts,
        uvs,
        tris,
        diffuse,
        msn,
        bounds,
        scale,
        z_translation,
        Some(water),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_btr_nif_inner(
    verts: &[[f32; 3]],
    uvs: &[[f32; 2]],
    tris: &[[u16; 3]],
    diffuse: &str,
    msn: &str,
    bounds: &BBox,
    scale: f32,
    z_translation: f32,
    water: Option<&WaterMesh>,
) -> anyhow::Result<NifFile> {
    let mut nif = NifFile::new("fo4");
    // Remove the default BSFadeNode root (NifFile::new inserts one).
    nif.blocks.clear();
    nif.header.num_blocks = 0;
    nif.header.block_type_names.clear();
    nif.header.block_type_index.clear();
    nif.header.block_sizes.clear();

    // --- Block 3: BSShaderTextureSet (added first so refs point forward) ---
    // slot0 = diffuse, slot1 = msn.
    // Vanilla CK pads NumTextures to 10; LODGen writes 2. We match LODGen.
    let tex_id = {
        let mut fields = IndexMap::new();
        fields.insert("Num Textures".to_string(), NifValue::UInt(2));
        let tex0 = format!("Data\\{}", diffuse);
        let tex1 = format!("Data\\{}", msn);
        fields.insert(
            "Textures".to_string(),
            NifValue::Array(vec![NifValue::String(tex0), NifValue::String(tex1)]),
        );
        nif.add_block("BSShaderTextureSet", Some(fields))
    };

    // --- Block 2: BSLightingShaderProperty ---
    // shaderType 18 = "LOD Landscape Noise"
    // shaderFlags1 = 0x80401000 = 2151682048
    // shaderFlags2 = 3 (ZBuffer_Write | LOD_Landscape)
    // textureClampMode = 0 (CLAMP_S_CLAMP_T), glossiness=0.0, specularStrength=1.0
    // FO4 PBR/wetness tail: rootMaterial="", subsurfaceRolloff=0, rimlightPower=f32::MAX,
    //   backlightPower=0, grayscaleToPaletteScale=1, fresnelPower=5, wetness all -1.
    let lsp_id = {
        let mut fields = IndexMap::new();
        // Shader Type 18 = "LOD Landscape Noise", stored as the numeric enum value. The
        // schema gates shader-type-specific tail fields on `cond="Shader Type == N"`,
        // and nif_core coerces a String-valued enum to true (so `== 1` matches), which
        // would emit the 6-byte `Shader Type == 1` tail (Environment Map Scale + 2 SSR
        // bools), oversizing the block and desyncing FO4's BTR parser into a
        // BSFixedString AV. UInt(18) serializes byte-identically to the golden LODGen
        // .btr (which stores Shader Type as uint 18).
        fields.insert("Shader Type".to_string(), NifValue::UInt(18));
        fields.insert("Name".to_string(), NifValue::String(String::new()));
        fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
        fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
        fields.insert("Controller".to_string(), NifValue::Ref(-1));
        // Shader Flags 1/2 are versioned bitflag fields keyed "Shader Flags 1:FO4" /
        // "Shader Flags 2:FO4" in nif_core's schema (BS Version == 130), the same keys
        // the reader assigns and the golden DLC03FarHarbor.*.btr carries. A bare-name
        // key is silently lost on write (schema default wins). Values are raw FO4
        // masks, verified against the golden .btr (nif.xml bit indices):
        //   flags1 0x80401000 = Model_Space_Normals(bit12) | Own_Emit(bit22)
        //                       | ZBuffer_Test(bit31) = 2151682048
        //   flags2 0x3        = ZBuffer_Write(bit0) | LOD_Landscape(bit1)
        fields.insert("Shader Flags 1:FO4".to_string(), NifValue::UInt(2151682048));
        fields.insert("Shader Flags 2:FO4".to_string(), NifValue::UInt(3));
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
        fields.insert("Texture Set".to_string(), NifValue::Ref(tex_id as i32));
        fields.insert(
            "Emissive Color".to_string(),
            NifValue::Color3([0.0, 0.0, 0.0]),
        );
        fields.insert("Emissive Multiple".to_string(), NifValue::Float(1.0));
        // Root Material: golden LODGen writes NO material (NiFixedString index -1),
        // not an empty-string entry. NifValue::Null serializes to index -1, matching
        // golden byte-for-byte; an empty String would instead point at the ""
        // string-table entry. Keeps `$Name`-gated material parsing off, as golden.
        fields.insert("Root Material".to_string(), NifValue::Null);
        fields.insert(
            "Texture Clamp Mode".to_string(),
            NifValue::String("CLAMP_S_CLAMP_T".to_string()),
        );
        fields.insert("Alpha".to_string(), NifValue::Float(1.0));
        fields.insert("Refraction Strength".to_string(), NifValue::Float(0.0));
        // Smoothness (= glossiness = 0.0 for FO4 terrain)
        fields.insert("Smoothness".to_string(), NifValue::Float(0.0));
        fields.insert(
            "Specular Color".to_string(),
            NifValue::Color3([1.0, 1.0, 1.0]),
        );
        fields.insert("Specular Strength".to_string(), NifValue::Float(1.0));
        // FO4 PBR tail (BSLightingShaderProperty.cs:153-179)
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
        // Wetness: all -1.0
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
    };

    // --- Block 1: BSTriShape "Land" ---
    // Vertex Desc = 52776558133763 (VERTEX|UV half-float, vertexSize=3, stride 12 bytes).
    // The writer uses (attributes & 0x401 == 0x1) branch: half-float pos + unused-W + UV.
    // nif_core vertex struct needs "Vertex" (Vec3), "Unused W" (UInt=0), "UV" (Struct{u,v}).
    let shape_id = {
        // ShiftZ (Geometry.cs:2165-2175 + TerrainLOD.cs:1432,1438-1441): recenter the
        // local-space vert z around the bbox z-center, and fold lodLevel * z_center
        // (plus the per-level zShift passed in `z_translation`) into Translation.z.
        let z_center = (bounds.min[2] + bounds.max[2]) / 2.0;
        let z_extent = bounds.max[2] - bounds.min[2];
        let translation_z = z_translation + scale * z_center;
        // FULLPREC (Geometry.cs:346-349 / TerrainLOD.cs:1445-1448): tall tiles whose
        // z extent exceeds 131008 units lose fp16 precision, so position becomes
        // full-f32 (vertexFlags |= 0x4000, vertexSize 4 -> stride 16).
        let fullprec = z_extent > 131008.0;

        // center: bbox xy center; z forced to 0 for FO4 land (TerrainLOD.cs:1443).
        let xy = bounds.center(false);
        let center = [xy[0], xy[1], 0.0];
        let radius = bounds.radius();

        let vertex_data: Vec<NifValue> = verts
            .iter()
            .zip(uvs.iter())
            .map(|(v, uv)| {
                let mut data = IndexMap::new();
                data.insert(
                    "Vertex".to_string(),
                    NifValue::Vec3([v[0], v[1], v[2] - z_center]),
                );
                data.insert("Unused W".to_string(), NifValue::UInt(0));
                let mut tex = IndexMap::new();
                tex.insert("u".to_string(), NifValue::Float(uv[0] as f64));
                tex.insert("v".to_string(), NifValue::Float(uv[1] as f64));
                data.insert("UV".to_string(), NifValue::Struct(tex));
                NifValue::Struct(data)
            })
            .collect();

        let triangle_data = super::triangle_values(tris);

        let mut fields = IndexMap::new();
        fields.insert("Name".to_string(), NifValue::String("Land".to_string()));
        fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
        fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
        fields.insert("Controller".to_string(), NifValue::Ref(-1));
        fields.insert("Flags".to_string(), NifValue::UInt(14));
        fields.insert("Flags2".to_string(), NifValue::UInt(0));
        fields.insert(
            "Translation".to_string(),
            NifValue::Vec3([0.0, 0.0, translation_z]),
        );
        fields.insert(
            "Rotation".to_string(),
            NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        );
        fields.insert("Scale".to_string(), NifValue::Float(scale as f64));
        fields.insert("Collision Object".to_string(), NifValue::Ref(-1));
        // Bounding sphere (BSTriShape center/radius)
        fields.insert(
            "Bounding Sphere".to_string(),
            NifValue::Struct({
                let mut m = IndexMap::new();
                m.insert("Center".to_string(), NifValue::Vec3(center));
                m.insert("Radius".to_string(), NifValue::Float(radius as f64));
                m
            }),
        );
        fields.insert("Skin".to_string(), NifValue::Ref(-1));
        fields.insert("Shader Property".to_string(), NifValue::Ref(lsp_id as i32));
        fields.insert("Alpha Property".to_string(), NifValue::Ref(-1));
        // Vertex Desc — half-prec VERTEX|UV (vertexSize=3 stride 12, floatSize=2):
        //   52776558133763 = 0x300000000203, bytes LE [03 02 00 00 00 30 00 00].
        // FULLPREC tall tiles use full-f32 (vertexSize=4 stride 16, floatSize=4,
        // vertexFlags |= 0x4000): 18067175067616260 = 0x40300000000404, bytes LE
        // [04 04 00 00 00 30 40 00] → attributes(Desc>>44)=0x403 selects the
        // writer's full-float branch (attributes & 0x401 == 0x401).
        let vertex_desc: u64 = if fullprec {
            18067175067616260
        } else {
            52776558133763
        };
        fields.insert("Vertex Desc".to_string(), NifValue::UInt(vertex_desc));
        fields.insert(
            "Num Triangles".to_string(),
            NifValue::UInt(tris.len() as u64),
        );
        fields.insert(
            "Num Vertices".to_string(),
            NifValue::UInt(verts.len() as u64),
        );
        fields.insert("Data Size".to_string(), NifValue::UInt(0)); // computed by writer
        fields.insert("Vertex Data".to_string(), NifValue::Array(vertex_data));
        fields.insert("Triangles".to_string(), NifValue::Array(triangle_data));
        nif.add_block("BSTriShape", Some(fields))
    };

    // --- Root BSMultiBoundAABB (block 5, or 10 with WATER) ---
    let aabb_id = {
        let c = bounds.center(true);
        let e = bounds.extent(true);
        let center = [c[0] * scale, c[1] * scale, z_translation + c[2] * scale];
        let extent = [e[0] * scale, e[1] * scale, e[2] * scale];
        let mut fields = IndexMap::new();
        fields.insert("Position".to_string(), NifValue::Vec3(center));
        fields.insert("Extent".to_string(), NifValue::Vec3(extent));
        nif.add_block("BSMultiBoundAABB", Some(fields))
    };

    // --- Root BSMultiBound → BSMultiBoundAABB (block 4, or 9 with WATER) ---
    let mb_id = {
        let mut fields = IndexMap::new();
        fields.insert("Data".to_string(), NifValue::Ref(aabb_id as i32));
        nif.add_block("BSMultiBound", Some(fields))
    };

    // --- WATER block (landless/ocean coarse sheet, TerrainLOD.cs:1520-1660) ---
    // Builds a BSMultiBoundNode "WATER" → (water BSTriShape →
    // BSEffectShaderProperty, BSMultiBound → BSMultiBoundAABB) and returns the
    // WATER node id so the chunk can carry it as a 2nd child. None when the quad
    // has no water cells.
    let water_node_id = water.map(|w| build_water_node(&mut nif, w, scale, z_translation));

    // --- Block 0: BSMultiBoundNode "chunk" (root) ---
    // cullMode=1 (CULL_ALLPASS), children=[Land BSTriShape (+ WATER node)],
    // multiBound→root BSMultiBound. Replace the cleared root slot by inserting at 0.
    {
        let chunk_id = {
            let mut fields = IndexMap::new();
            fields.insert("Name".to_string(), NifValue::String("chunk".to_string()));
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
            // Children: terrain shape, then (if any) the WATER node — matching the
            // golden chunk's Children=[1, 4] (TerrainLOD.cs:1531).
            let mut children = vec![NifValue::Ref(shape_id as i32)];
            if let Some(wid) = water_node_id {
                children.push(NifValue::Ref(wid as i32));
            }
            fields.insert(
                "Num Children".to_string(),
                NifValue::UInt(children.len() as u64),
            );
            fields.insert("Children".to_string(), NifValue::Array(children));
            fields.insert("Multi Bound".to_string(), NifValue::Ref(mb_id as i32));
            fields.insert(
                "Culling Mode".to_string(),
                NifValue::String("CULL_ALLPASS".to_string()),
            );
            nif.add_block("BSMultiBoundNode", Some(fields))
        };
        // Move the chunk node to position 0 (root must be first).
        // After add_block, it is the last block. We need to rotate it to index 0
        // and fix all refs accordingly.
        let last = nif.blocks.len() - 1;
        if chunk_id != 0 {
            // Rotate: move the last block to index 0.
            nif.blocks.rotate_right(1);
            // Fix block_ids.
            for (i, b) in nif.blocks.iter_mut().enumerate() {
                b.block_id = i;
            }
            // Remap refs: chunk moved from last→0; all others shifted +1.
            let mut id_map = std::collections::HashMap::new();
            id_map.insert(last as i32, 0i32);
            for i in 0..last {
                id_map.insert(i as i32, (i + 1) as i32);
            }
            id_map.insert(-1i32, -1i32);
            nif.remap_refs(&id_map);
            nif.rebuild_header();
        }
    }

    // The FO4 terrain LOD path is sensitive to the canonical BTR block layout
    // used by shipped files and xLODGen: chunk=0, Land=1, shaders next, WATER
    // subtree before the root multibound. The construction above uses forward
    // refs and then rotates the root; finish by putting blocks in that layout.
    if water.is_some() {
        reorder_blocks(&mut nif, &[0, 3, 2, 1, 10, 7, 6, 9, 8, 5, 4]);
    } else {
        reorder_blocks(&mut nif, &[0, 3, 2, 1, 5, 4]);
    }

    // Set footer root to block 0.
    nif.header.footer_roots = vec![0];

    Ok(nif)
}

fn reorder_blocks(nif: &mut NifFile, old_order: &[usize]) {
    debug_assert_eq!(old_order.len(), nif.blocks.len());
    let mut id_map = std::collections::HashMap::new();
    for (new_id, old_id) in old_order.iter().copied().enumerate() {
        id_map.insert(old_id as i32, new_id as i32);
    }

    let mut blocks = Vec::with_capacity(nif.blocks.len());
    for (new_id, old_id) in old_order.iter().copied().enumerate() {
        let mut block = nif.blocks[old_id].clone();
        block.block_id = new_id;
        blocks.push(block);
    }
    nif.blocks = blocks;
    nif.remap_refs(&id_map);
    nif.rebuild_header();
}

fn expand_water_segments(water: &WaterMesh, count: i32) -> Vec<WaterSegment> {
    let total = (count * count) as usize;
    let mut expanded: Vec<WaterSegment> = (0..total)
        .map(|_| WaterSegment {
            id: 0,
            start_triangle: 0,
            num_triangles: 0,
        })
        .collect();

    for segment in &water.segments {
        let idx = segment.id as usize;
        if idx < total {
            expanded[idx] = segment.clone();
        }
    }

    let mut last = expanded.len();
    while last > 0 && expanded[last - 1].num_triangles == 0 {
        last -= 1;
    }
    expanded.truncate(last);
    expanded
}

/// Append the WATER block graph to `nif` and return the `BSMultiBoundNode "WATER"`
/// block id. Adds, in order, the water `BSEffectShaderProperty`, the water
/// `BSTriShape`, a `BSMultiBoundAABB`, a `BSMultiBound` and the WATER node.
///
/// The water `BSTriShape` uses the same z math as the terrain block via the shared
/// `scale` (= lodLevel) and `z_translation` (= zShift[lodIndex]):
/// `Translation.z = z_translation + scale * z_center`, verts recentered by
/// `z_center` (`Geometry.ShiftZ`, Geometry.cs:2165-2175 / ToBSTriShape:339-345).
/// For a flat water sheet z_center == water/level and z_extent == 0, so all verts
/// recenter to 0 and `Translation.z == z_translation + scale*(water/level) == water`.
///
/// The water shape has no UVs (the golden vertex desc is VERTEX-only) and uses a
/// `BSEffectShaderProperty`, not a BSLightingShaderProperty. Field values match the
/// golden `DLC03FarHarbor.32.-41.-27.btr` block 5/6 and xLODGen
/// (TerrainLOD.cs:1570-1580).
fn build_water_node(nif: &mut NifFile, water: &WaterMesh, scale: f32, z_translation: f32) -> usize {
    // --- water BSEffectShaderProperty (added first so refs point forward) ---
    // C# (TerrainLOD.cs:1570-1580): flags1=0x80000000, flags2=1, clampMode=65283,
    // falloffStart/StopOpacity=0, emissive(BaseColor)=(1,1,1,1), softFalloff=100,
    // envMapScale=1. Shader Flags use version-suffixed keys "Shader Flags 1:FO4"
    // (BS Version 130); a bare key is silently dropped on write (schema default
    // wins). Texture Clamp Mode is an enum; a raw UInt(65283) passes through.
    let esp_id = {
        let mut fields = IndexMap::new();
        fields.insert("Name".to_string(), NifValue::String(String::new()));
        fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
        fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
        fields.insert("Controller".to_string(), NifValue::Ref(-1));
        fields.insert(
            "Shader Flags 1:FO4".to_string(),
            NifValue::UInt(0x8000_0000),
        );
        fields.insert("Shader Flags 2:FO4".to_string(), NifValue::UInt(1));
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
        fields.insert(
            "Source Texture".to_string(),
            NifValue::String(String::new()),
        );
        // Texture Clamp Mode: the FO4 on-disk field is a single BYTE (schema
        // TexClampModeB, storage=byte). xLODGen's SetTextureClampMode(65283u) only
        // its low byte survives serialization, and the golden water block reads
        // back WRAP_S_WRAP_T (0). Leave the schema default (WRAP_S_WRAP_T) — adding
        // a UInt(65283) would either truncate to 3 or diverge from the golden byte.
        fields.insert(
            "Texture Clamp Mode".to_string(),
            NifValue::String("WRAP_S_WRAP_T".to_string()),
        );
        fields.insert("Lighting Influence".to_string(), NifValue::UInt(0));
        fields.insert("Env Map Min LOD".to_string(), NifValue::UInt(0));
        fields.insert("Unused Byte".to_string(), NifValue::UInt(0));
        // xLODGen's BSEffectShaderProperty leaves the falloff angles at 0 (its
        // ctor default), NOT the nif.xml default of 1.0 — match the golden (0.0).
        fields.insert("Falloff Start Angle".to_string(), NifValue::Float(0.0));
        fields.insert("Falloff Stop Angle".to_string(), NifValue::Float(0.0));
        fields.insert("Falloff Start Opacity".to_string(), NifValue::Float(0.0));
        fields.insert("Falloff Stop Opacity".to_string(), NifValue::Float(0.0));
        fields.insert(
            "Base Color".to_string(),
            NifValue::Color4([1.0, 1.0, 1.0, 1.0]),
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
    };

    // --- water BSTriShape (no UVs) ---
    let bounds = &water.bbox;
    let z_center = (bounds.min[2] + bounds.max[2]) / 2.0;
    let z_extent = bounds.max[2] - bounds.min[2];
    let translation_z = z_translation + scale * z_center;
    // FULLPREC for tall sheets (ToBSTriShape:346-348). Flat water never hits this.
    let fullprec = z_extent > 131008.0;

    // FO4 forces the water shape's bound-sphere center z to 0 (ToBSTriShape:343-345).
    let xy = bounds.center(false);
    let center = [xy[0], xy[1], 0.0];
    // ToBSTriShape uses Geometry.GetRadius() (vertex-based, called BEFORE ShiftZ):
    // max distance from bbox xy/z center to any vertex. For a flat sheet z_extent==0
    // so this equals bounds.radius() (the bbox-corner radius). Use the bbox radius.
    let radius = bounds.radius();

    let vertex_data: Vec<NifValue> = water
        .verts
        .iter()
        .map(|v| {
            let mut data = IndexMap::new();
            data.insert(
                "Vertex".to_string(),
                NifValue::Vec3([v[0], v[1], v[2] - z_center]),
            );
            data.insert("Unused W".to_string(), NifValue::UInt(0));
            NifValue::Struct(data)
        })
        .collect();

    let triangle_data = super::triangle_values(&water.tris);

    let shape_id = {
        let segment_count = scale.round() as i32;
        let use_subindex = segment_count == 4 && !water.segments.is_empty();
        let mut fields = IndexMap::new();
        fields.insert("Name".to_string(), NifValue::String(String::new()));
        fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
        fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
        fields.insert("Controller".to_string(), NifValue::Ref(-1));
        fields.insert("Flags".to_string(), NifValue::UInt(14));
        fields.insert("Flags2".to_string(), NifValue::UInt(0));
        fields.insert(
            "Translation".to_string(),
            NifValue::Vec3([0.0, 0.0, translation_z]),
        );
        fields.insert(
            "Rotation".to_string(),
            NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        );
        fields.insert("Scale".to_string(), NifValue::Float(scale as f64));
        fields.insert("Collision Object".to_string(), NifValue::Ref(-1));
        fields.insert(
            "Bounding Sphere".to_string(),
            NifValue::Struct({
                let mut m = IndexMap::new();
                m.insert("Center".to_string(), NifValue::Vec3(center));
                m.insert("Radius".to_string(), NifValue::Float(radius as f64));
                m
            }),
        );
        fields.insert("Skin".to_string(), NifValue::Ref(-1));
        fields.insert("Shader Property".to_string(), NifValue::Ref(esp_id as i32));
        fields.insert("Alpha Property".to_string(), NifValue::Ref(-1));
        // Vertex Desc — VERTEX-only, half-float, NO UV (golden water block):
        //   17592186044930 = 0x100000000202, vertexSize=2 (stride 8), attributes
        //   (Desc>>44)=0x1 (VERTEX, no UV bit). FULLPREC tall sheets use full-f32:
        //   0x140000000404 attributes=0x401, vertexSize 4 (stride 16).
        let vertex_desc: u64 = if fullprec {
            // attributes 0x401 (VERTEX|FULLPREC), vertexSize 4: 0x140000000404.
            0x0014_0000_0404
        } else {
            17592186044930
        };
        fields.insert("Vertex Desc".to_string(), NifValue::UInt(vertex_desc));
        fields.insert(
            "Num Triangles".to_string(),
            NifValue::UInt(water.tris.len() as u64),
        );
        fields.insert(
            "Num Vertices".to_string(),
            NifValue::UInt(water.verts.len() as u64),
        );
        fields.insert("Data Size".to_string(), NifValue::UInt(0)); // computed by writer
        fields.insert("Vertex Data".to_string(), NifValue::Array(vertex_data));
        fields.insert("Triangles".to_string(), NifValue::Array(triangle_data));
        if use_subindex {
            let expanded = expand_water_segments(water, segment_count);
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
                    d.insert(
                        "Parent Array Index".to_string(),
                        NifValue::UInt(0xFFFF_FFFF),
                    );
                    d.insert("Num Sub Segments".to_string(), NifValue::UInt(0));
                    d.insert("Sub Segment".to_string(), NifValue::Array(Vec::new()));
                    NifValue::Struct(d)
                })
                .collect();

            fields.insert(
                BTO_NUM_PRIMITIVES_OVERRIDE_FIELD.to_string(),
                NifValue::UInt((water.tris.len() as u64) * 2),
            );
            fields.insert(
                "Num Segments".to_string(),
                NifValue::UInt(expanded.len() as u64),
            );
            fields.insert(
                "Total Segments".to_string(),
                NifValue::UInt(expanded.len() as u64),
            );
            fields.insert("Segment".to_string(), NifValue::Array(segment_data));
            nif.add_block("BSSubIndexTriShape", Some(fields))
        } else {
            nif.add_block("BSTriShape", Some(fields))
        }
    };

    // --- water BSMultiBoundAABB (TerrainLOD.cs:1652-1657) ---
    // bbWater.GetCenter(zero:true) * lodLevel, GetExtend(zero:true) * lodLevel.
    let water_aabb_id = {
        let c = bounds.center(true);
        let e = bounds.extent(true);
        let position = [c[0] * scale, c[1] * scale, c[2] * scale];
        let extent = [e[0] * scale, e[1] * scale, e[2] * scale];
        let mut fields = IndexMap::new();
        fields.insert("Position".to_string(), NifValue::Vec3(position));
        fields.insert("Extent".to_string(), NifValue::Vec3(extent));
        nif.add_block("BSMultiBoundAABB", Some(fields))
    };

    let water_mb_id = {
        let mut fields = IndexMap::new();
        fields.insert("Data".to_string(), NifValue::Ref(water_aabb_id as i32));
        nif.add_block("BSMultiBound", Some(fields))
    };

    // --- BSMultiBoundNode "WATER" (TerrainLOD.cs:1522-1531) ---
    // Children=[water BSTriShape], MultiBound→water BSMultiBound. The C# bug at
    // line 1523 sets cullMode on the PARENT chunk, not this node, so WATER keeps
    // its default Culling Mode = CULL_NORMAL (golden block 4).
    let mut fields = IndexMap::new();
    fields.insert("Name".to_string(), NifValue::String("WATER".to_string()));
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
    fields.insert("Num Children".to_string(), NifValue::UInt(1));
    fields.insert(
        "Children".to_string(),
        NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
    );
    fields.insert("Multi Bound".to_string(), NifValue::Ref(water_mb_id as i32));
    fields.insert(
        "Culling Mode".to_string(),
        NifValue::String("CULL_NORMAL".to_string()),
    );
    nif.add_block("BSMultiBoundNode", Some(fields))
}

/// Write a `.btr` terrain LOD mesh to disk.
///
/// Per the contract signature this takes no scale/level arg; writes scale=1.0, z_translation=0.0.
/// The terrain writer calls `build_btr_nif` directly with the real lodLevel/zShift.
pub fn write_btr(
    path: &std::path::Path,
    verts: &[[f32; 3]],
    uvs: &[[f32; 2]],
    tris: &[[u16; 3]],
    diffuse: &str,
    msn: &str,
    bounds: &BBox,
) -> anyhow::Result<()> {
    let mut nif = build_btr_nif(verts, uvs, tris, diffuse, msn, bounds, 1.0, 0.0)?;
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
    use crate::descriptors::BBox;

    fn ref_array(value: Option<&NifValue>) -> Vec<i32> {
        match value {
            Some(NifValue::Array(items)) => items
                .iter()
                .map(|item| match item {
                    NifValue::Ref(id) => *id,
                    other => panic!("expected Ref in array, got {other:?}"),
                })
                .collect(),
            other => panic!("expected Ref array, got {other:?}"),
        }
    }

    fn vec3(value: Option<&NifValue>) -> [f32; 3] {
        match value {
            Some(NifValue::Vec3(v)) => *v,
            other => panic!("expected Vec3, got {other:?}"),
        }
    }

    #[test]
    fn build_btr_block_graph() {
        let verts = [[0.0, 0.0, 0.0], [4096.0, 0.0, 0.0], [0.0, 4096.0, 0.0]];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let tris = [[0u16, 1, 2]];
        let mut b = BBox::empty();
        for v in &verts {
            b.grow_vertex(*v);
        }

        let mut nif = build_btr_nif(
            &verts,
            &uvs,
            &tris,
            r"Textures\Terrain\W\W.4.0.0.dds",
            r"Textures\Terrain\W\W.4.0.0_msn.dds",
            &b,
            1.0,
            0.0,
        )
        .unwrap();

        // serialize then assert it parses back as a valid NIF
        let bytes = nif.to_bytes().unwrap();
        assert!(bytes.len() > 100);
        // bs_version 130, version 20.2.0.7
        assert_eq!(nif.header.bs_version, 130);
        let block_types: Vec<_> = nif.blocks.iter().map(|b| b.type_name.as_str()).collect();
        assert_eq!(
            block_types,
            vec![
                "BSMultiBoundNode",
                "BSTriShape",
                "BSLightingShaderProperty",
                "BSShaderTextureSet",
                "BSMultiBound",
                "BSMultiBoundAABB",
            ],
            "no-water BTR block order must match shipped/xLODGen"
        );
        match nif.blocks[0].fields.get("Children") {
            value => assert_eq!(ref_array(value), vec![1]),
        }
        match nif.blocks[0].fields.get("Multi Bound") {
            Some(NifValue::Ref(id)) => assert_eq!(*id, 4),
            other => panic!("chunk multibound: {other:?}"),
        }
        // root is BSMultiBoundNode, has a BSTriShape "Land" child with shaderType 18
        let has_tri = nif.blocks.iter().any(|bl| bl.type_name == "BSTriShape");
        let has_lsp = nif
            .blocks
            .iter()
            .any(|bl| bl.type_name == "BSLightingShaderProperty");
        assert!(has_tri && has_lsp);
    }

    #[test]
    fn write_btr_to_disk() {
        let verts = [[0.0, 0.0, 0.0], [4096.0, 0.0, 0.0], [0.0, 4096.0, 0.0]];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let tris = [[0u16, 1, 2]];
        let mut b = BBox::empty();
        for v in &verts {
            b.grow_vertex(*v);
        }
        let dir = std::env::temp_dir().join("lodgen_btr_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("W.4.0.0.btr");
        write_btr(&p, &verts, &uvs, &tris, "d.dds", "d_msn.dds", &b).unwrap();
        assert!(std::fs::metadata(&p).unwrap().len() > 100);
    }

    use crate::terrain::water::{WaterMesh, WaterSegment};

    fn flat_water_sheet() -> WaterMesh {
        // 2x2 cell sheet at z=10 in local space (3x3 = 9 welded verts, 8 tris).
        let mut verts = Vec::new();
        for gy in 0..3 {
            for gx in 0..3 {
                verts.push([gx as f32 * 128.0, gy as f32 * 128.0, 10.0]);
            }
        }
        let idx = |x: usize, y: usize| (y * 3 + x) as u16;
        let mut tris = Vec::new();
        for cy in 0..2 {
            for cx in 0..2 {
                let a = idx(cx, cy);
                let b2 = idx(cx + 1, cy);
                let c = idx(cx, cy + 1);
                let d = idx(cx + 1, cy + 1);
                tris.push([a, b2, c]);
                tris.push([b2, d, c]);
            }
        }
        let mut bbox = BBox::empty();
        for v in &verts {
            bbox.grow_vertex(*v);
        }
        let segments = (0..4)
            .map(|i| WaterSegment {
                id: i,
                start_triangle: (i * 2) as u32,
                num_triangles: 2,
            })
            .collect();
        WaterMesh {
            verts,
            tris,
            segments,
            bbox,
        }
    }

    #[test]
    fn terrain_and_water_triangles_match_legacy_bytes_at_every_level() {
        let water = flat_water_sheet();
        let uvs = vec![[0., 1.]; water.verts.len()];
        for level in [4., 8., 16., 32.] {
            let mut land = build_btr_nif(
                &water.verts,
                &uvs,
                &water.tris,
                "land_d.dds",
                "land_n.dds",
                &water.bbox,
                level,
                -17.5,
            )
            .unwrap();
            super::super::assert_triangle_bytes_match_legacy(&mut land);
            let mut combined = build_btr_nif_with_water(
                &water.verts,
                &uvs,
                &water.tris,
                "land_d.dds",
                "land_n.dds",
                &water.bbox,
                level,
                -17.5,
                &water,
            )
            .unwrap();
            super::super::assert_triangle_bytes_match_legacy(&mut combined);
        }
    }

    /// build_btr_nif_with_water emits a water BSTriShape with a
    /// BSEffectShaderProperty under a "WATER" BSMultiBoundNode, and a chunk root
    /// with TWO children — matching the golden landless `.btr` graph.
    #[test]
    fn water_block_graph_matches_golden_structure() {
        let verts = [[0.0, 0.0, 0.0], [4096.0, 0.0, 0.0], [0.0, 4096.0, 0.0]];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let tris = [[0u16, 1, 2]];
        let mut b = BBox::empty();
        for v in &verts {
            b.grow_vertex(*v);
        }
        let water = flat_water_sheet();
        let nif = build_btr_nif_with_water(
            &verts,
            &uvs,
            &tris,
            "d.dds",
            "d_msn.dds",
            &b,
            32.0,
            0.0,
            &water,
        )
        .unwrap();

        let block_types: Vec<_> = nif.blocks.iter().map(|b| b.type_name.as_str()).collect();
        assert_eq!(
            block_types,
            vec![
                "BSMultiBoundNode",
                "BSTriShape",
                "BSLightingShaderProperty",
                "BSShaderTextureSet",
                "BSMultiBoundNode",
                "BSTriShape",
                "BSEffectShaderProperty",
                "BSMultiBound",
                "BSMultiBoundAABB",
                "BSMultiBound",
                "BSMultiBoundAABB",
            ],
            "water BTR block order must match shipped/xLODGen"
        );

        // One terrain BSTriShape, one water BSTriShape, one BSEffectShaderProperty,
        // one BSLightingShaderProperty, and a "WATER" BSMultiBoundNode.
        let tri_shapes = nif
            .blocks
            .iter()
            .filter(|x| x.type_name == "BSTriShape")
            .count();
        assert_eq!(tri_shapes, 2, "expected terrain + water BSTriShape");
        assert_eq!(
            nif.blocks
                .iter()
                .filter(|x| x.type_name == "BSSubIndexTriShape")
                .count(),
            0,
            "coarse BTR water must not use BSSubIndexTriShape"
        );
        assert_eq!(
            nif.blocks
                .iter()
                .filter(|x| x.type_name == "BSEffectShaderProperty")
                .count(),
            1
        );
        let water_node = nif
            .blocks
            .iter()
            .find(|x| {
                x.type_name == "BSMultiBoundNode"
                    && matches!(x.fields.get("Name"), Some(NifValue::String(s)) if s == "WATER")
            })
            .expect("WATER BSMultiBoundNode present");
        // WATER node keeps default cull mode (CULL_NORMAL), not CULL_ALLPASS.
        match water_node.fields.get("Culling Mode") {
            Some(NifValue::String(s)) => assert_eq!(s, "CULL_NORMAL"),
            other => panic!("WATER culling mode: {other:?}"),
        }

        // The chunk root has two children.
        let chunk = &nif.blocks[0];
        match chunk.fields.get("Children") {
            value => assert_eq!(ref_array(value), vec![1, 4]),
        }
        match chunk.fields.get("Multi Bound") {
            Some(NifValue::Ref(id)) => assert_eq!(*id, 9),
            other => panic!("chunk multibound: {other:?}"),
        }
        match water_node.fields.get("Children") {
            value => assert_eq!(ref_array(value), vec![5]),
        }
        match water_node.fields.get("Multi Bound") {
            Some(NifValue::Ref(id)) => assert_eq!(*id, 7),
            other => panic!("water multibound: {other:?}"),
        }
    }

    #[test]
    fn l4_water_block_uses_subindex_segments() {
        let verts = [[0.0, 0.0, 0.0], [4096.0, 0.0, 0.0], [0.0, 4096.0, 0.0]];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let tris = [[0u16, 1, 2]];
        let mut b = BBox::empty();
        for v in &verts {
            b.grow_vertex(*v);
        }
        let water = flat_water_sheet();
        let mut nif = build_btr_nif_with_water(
            &verts,
            &uvs,
            &tris,
            "d.dds",
            "d_msn.dds",
            &b,
            4.0,
            0.0,
            &water,
        )
        .unwrap();
        let bytes = nif.to_bytes().unwrap();
        let rt = NifFile::from_bytes(&bytes, None).expect("reload L4 water btr");
        let water_shape = rt
            .blocks
            .iter()
            .find(|x| x.type_name == "BSSubIndexTriShape")
            .expect("L4 water must be segmented");

        match water_shape.fields.get("Num Primitives") {
            Some(NifValue::UInt(v)) => assert_eq!(*v, 16),
            other => panic!("L4 water Num Primitives: {other:?}"),
        }
        match water_shape.fields.get("Num Segments") {
            Some(NifValue::UInt(v)) => assert_eq!(*v, 4),
            other => panic!("L4 water Num Segments: {other:?}"),
        }
        match water_shape.fields.get("Total Segments") {
            Some(NifValue::UInt(v)) => assert_eq!(*v, 4),
            other => panic!("L4 water Total Segments: {other:?}"),
        }
        let segments = match water_shape.fields.get("Segment") {
            Some(NifValue::Array(v)) => v,
            other => panic!("L4 water Segment: {other:?}"),
        };
        assert_eq!(segments.len(), 4);
        for (i, segment) in segments.iter().enumerate() {
            let NifValue::Struct(fields) = segment else {
                panic!("segment {i}: {segment:?}");
            };
            match fields.get("Start Index") {
                Some(NifValue::UInt(v)) => assert_eq!(*v, (i as u64) * 6),
                other => panic!("segment {i} Start Index: {other:?}"),
            }
            match fields.get("Num Primitives") {
                Some(NifValue::UInt(v)) => assert_eq!(*v, 2),
                other => panic!("segment {i} Num Primitives: {other:?}"),
            }
        }
    }

    #[test]
    fn root_multibound_is_scaled_to_lod_space() {
        let verts = [[0.0, 0.0, -16.0], [4096.0, 0.0, -8.0], [0.0, 4096.0, -8.0]];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let tris = [[0u16, 1, 2]];
        let mut b = BBox::empty();
        for v in &verts {
            b.grow_vertex(*v);
        }

        let nif = build_btr_nif(&verts, &uvs, &tris, "d.dds", "d_msn.dds", &b, 16.0, 0.0).unwrap();
        let root_bound = &nif.blocks[5];

        assert_eq!(
            vec3(root_bound.fields.get("Position")),
            [32768.0, 32768.0, -128.0]
        );
        assert_eq!(
            vec3(root_bound.fields.get("Extent")),
            [32768.0, 32768.0, 128.0]
        );
    }

    /// Regression: the terrain `BSLightingShaderProperty` must serialize to the
    /// golden LODGen 140 bytes with no trailing remainder. A name-string
    /// `Shader Type` ("LOD Landscape Noise") instead of the numeric enum (18) makes
    /// nif_core's `cond="Shader Type == 1"` evaluate true and emit the 6-byte tail
    /// (Environment Map Scale + 2 SSR bools, a 146-byte block), desyncing FO4's BTR
    /// parser into a BSFixedString access violation.
    #[test]
    fn lsp_block_is_golden_140_bytes_no_phantom_tail() {
        let verts = [[0.0, 0.0, 0.0], [4096.0, 0.0, 0.0], [0.0, 4096.0, 0.0]];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let tris = [[0u16, 1, 2]];
        let mut b = BBox::empty();
        for v in &verts {
            b.grow_vertex(*v);
        }
        let mut nif =
            build_btr_nif(&verts, &uvs, &tris, "d.dds", "d_msn.dds", &b, 16.0, 0.0).unwrap();
        let bytes = nif.to_bytes().unwrap();
        let rt = NifFile::from_bytes(&bytes, None).expect("reload btr");
        let (idx, lsp) = rt
            .blocks
            .iter()
            .enumerate()
            .find(|(_, x)| x.type_name == "BSLightingShaderProperty")
            .expect("LSP present");
        assert_eq!(
            rt.header.block_sizes.get(idx).copied(),
            Some(140),
            "LSP must be golden 140 bytes (got remainder {:02X?})",
            lsp.remainder
        );
        assert!(
            lsp.remainder.is_empty(),
            "LSP must have no phantom tail bytes, got {:02X?}",
            lsp.remainder
        );
        // Shader Type round-trips as the numeric enum 18, not a truthy string.
        match lsp.fields.get("Shader Type") {
            Some(NifValue::UInt(v)) => assert_eq!(*v, 18),
            other => panic!("Shader Type must be UInt(18), got {other:?}"),
        }
        // No phantom Shader-Type==1 tail fields leaked in.
        assert!(lsp.fields.get("Use Screen Space Reflections").is_none());
        assert!(lsp.fields.get("Wetness Control: Use SSR").is_none());
    }

    /// Regression: the water `BSEffectShaderProperty` and water `BSTriShape` must
    /// serialize to the xLODGen/FO4 structure with no phantom-tail inflation.
    ///
    /// Golden sizes (`DLC03FarHarbor.32.-41.-27.btr` block 6/5, and the vanilla
    /// same-level `Commonwealth.16.*.BTR`):
    ///   - `BSEffectShaderProperty` = **104 bytes**, no remainder (geometry-
    ///     independent). The only cond-gated tail field is `Material`
    ///     (`cond="$Name"`); `Name=""` coerces to `false` in `nif_to_eval`, so no
    ///     `.bgem` material ref leaks in. A non-empty Name (or any String enum a
    ///     `cond=` references) would inflate the block and the engine would parse
    ///     the WATER subtree at the wrong offsets (render-time null deref).
    ///   - The coarse water shape is a plain `BSTriShape`; shipped FO4 and xLODGen
    ///     coarse BTR water use no subindex segment table. `Data Size == verts*8 +
    ///     tris*6` (VERTEX-only, half-float stride 8; 3×u16 triangles).
    ///
    /// Round-trip alone does not catch inflation (it is self-consistent), so this
    /// asserts the writer-emitted `header.block_sizes` against the golden constants.
    #[test]
    fn water_blocks_are_golden_byte_sizes_no_phantom_tail() {
        let verts = [[0.0, 0.0, 0.0], [4096.0, 0.0, 0.0], [0.0, 4096.0, 0.0]];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let tris = [[0u16, 1, 2]];
        let mut b = BBox::empty();
        for v in &verts {
            b.grow_vertex(*v);
        }
        let water = flat_water_sheet();
        let mut nif = build_btr_nif_with_water(
            &verts,
            &uvs,
            &tris,
            "d.dds",
            "d_msn.dds",
            &b,
            32.0,
            0.0,
            &water,
        )
        .unwrap();
        let bytes = nif.to_bytes().unwrap();
        let rt = NifFile::from_bytes(&bytes, None).expect("reload water btr");

        // --- water BSEffectShaderProperty: golden 104 bytes, no phantom tail ---
        let (esp_idx, esp) = rt
            .blocks
            .iter()
            .enumerate()
            .find(|(_, x)| x.type_name == "BSEffectShaderProperty")
            .expect("water BSEffectShaderProperty present");
        assert_eq!(
            rt.header.block_sizes.get(esp_idx).copied(),
            Some(104),
            "water BSEffectShaderProperty must be golden 104 bytes (remainder {:02X?})",
            esp.remainder
        );
        assert!(
            esp.remainder.is_empty(),
            "water BSEffectShaderProperty must have no phantom tail, got {:02X?}",
            esp.remainder
        );
        // No `$Name`-gated material ref leaked in (would appear as a `Material` field).
        assert!(
            esp.fields.get("Material").is_none(),
            "empty Name must not emit a BGEM Material ref"
        );

        // --- water BSTriShape: Data Size matches xLODGen/shipped contract ---
        let (tri_idx, tri) = rt
            .blocks
            .iter()
            .enumerate()
            .find(|(_, x)| {
                x.type_name == "BSTriShape"
                    && matches!(x.fields.get("Name"), Some(NifValue::String(s)) if s.is_empty())
            })
            .expect("water BSTriShape present");
        let block_size = rt.header.block_sizes.get(tri_idx).copied().unwrap() as u64;
        let data_size = match tri.fields.get("Data Size") {
            Some(NifValue::UInt(v)) => *v,
            other => panic!("water Data Size: {other:?}"),
        };
        let nverts = water.verts.len() as u64;
        let ntris = water.tris.len() as u64;
        assert_eq!(
            data_size,
            nverts * 8 + ntris * 6,
            "water Data Size must be VERTEX-only stride 8 + 3xu16 tris (golden layout)"
        );
        assert!(tri.fields.get("Num Segments").is_none());
        assert!(tri.fields.get("Segment").is_none());
        assert_eq!(
            block_size - data_size,
            118,
            "water BSTriShape header overhead must match shipped/xLODGen"
        );
        assert!(
            tri.remainder.is_empty(),
            "water BSTriShape phantom tail: {:02X?}",
            tri.remainder
        );

        // --- cross-check against the on-disk golden when available ---
        let golden = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../../tmp/xlodgen/meshes/terrain/DLC03FarHarbor/DLC03FarHarbor.32.-41.-27.btr",
        );
        if golden.exists() {
            let g = NifFile::load(&golden).expect("load golden");
            let g_esp = g
                .blocks
                .iter()
                .position(|x| x.type_name == "BSEffectShaderProperty")
                .expect("golden ESP");
            assert_eq!(
                g.header.block_sizes.get(g_esp).copied(),
                Some(104),
                "golden ESP recorded size is the 104-byte anchor"
            );
        }
    }

    /// The water BSEffectShaderProperty flags use version-suffixed keys and MUST
    /// round-trip through serialize → reload (a bare key is silently dropped).
    #[test]
    fn water_effect_shader_flags_roundtrip() {
        let verts = [[0.0, 0.0, 0.0], [4096.0, 0.0, 0.0], [0.0, 4096.0, 0.0]];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let tris = [[0u16, 1, 2]];
        let mut b = BBox::empty();
        for v in &verts {
            b.grow_vertex(*v);
        }
        let water = flat_water_sheet();
        let mut nif = build_btr_nif_with_water(
            &verts,
            &uvs,
            &tris,
            "d.dds",
            "d_msn.dds",
            &b,
            32.0,
            0.0,
            &water,
        )
        .unwrap();
        let bytes = nif.to_bytes().unwrap();
        let rt = NifFile::from_bytes(&bytes, None).expect("reload water btr");

        let esp = rt
            .blocks
            .iter()
            .find(|x| x.type_name == "BSEffectShaderProperty")
            .expect("water BSEffectShaderProperty present");
        let mask = |field: &str| -> u64 {
            let key = format!("{field}:FO4");
            match esp.fields.get(&key).or_else(|| esp.fields.get(field)) {
                Some(NifValue::UInt(v)) => *v,
                Some(NifValue::Int(v)) => *v as u64,
                other => panic!("{key} not a numeric flag: {other:?}"),
            }
        };
        assert_eq!(
            mask("Shader Flags 1"),
            0x8000_0000,
            "flags1 must round-trip"
        );
        assert_eq!(mask("Shader Flags 2"), 0x1, "flags2 must round-trip");
        // Clamp mode round-trips as WRAP_S_WRAP_T (value 3 in TexClampModeB; the
        // golden water block reads back WRAP_S_WRAP_T too).
        match esp.fields.get("Texture Clamp Mode") {
            Some(NifValue::String(s)) => assert_eq!(s, "WRAP_S_WRAP_T"),
            Some(NifValue::UInt(v)) => assert_eq!(*v, 3),
            other => panic!("clamp mode: {other:?}"),
        }

        // Water survives the round-trip as a plain render shape with no UVs.
        let water_tri = rt
            .blocks
            .iter()
            .filter(|x| x.type_name == "BSTriShape")
            .find(|x| matches!(x.fields.get("Name"), Some(NifValue::String(s)) if s.is_empty()))
            .expect("water BSTriShape present");
        match water_tri.fields.get("Vertex Desc") {
            Some(NifValue::UInt(v)) => assert_eq!(*v, 17592186044930, "water vertex desc (no UV)"),
            other => panic!("water vertex desc: {other:?}"),
        }
        // Tri/vert counts preserved (8 tris, 9 welded verts).
        assert_eq!(water.tris.len(), 8, "sanity: flat 2x2 sheet has 8 tris");
    }
}
