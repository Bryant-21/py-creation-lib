//! FO4 collision extraction and Starfield collision emission.
//!
//! Recipe and evidence: `bacup/docs/starfield_target/R7-collision.md`.

use crate::fo76_collision::nif_vertices_to_havok;
use crate::model::{NifFile, NifValue};

/// A collision primitive in Starfield metres: every `CollisionPrim` vertex is in
/// Starfield metres, never FO4 render units.
#[derive(Debug, Clone)]
pub enum CollisionPrim {
    Box {
        center: [f32; 3],
        half_extents: [f32; 3],
        rotation: [f32; 4],
    },
    ConvexHull {
        verts: Vec<[f32; 3]>,
    },
}

/// The NIF-embedding recipe for a converted static's collision (R7 §3):
/// a `BSXFlags "BSX"` extra-data of `bsx_flags_value` on the root node, a
/// `bhkNPCollisionObject` (`Target` = root node id, `Data` = the physics-system block id,
/// `Body ID` = `body_id`, `Flags` = `collision_object_flags`), and a `bhkPhysicsSystem`
/// whose `Binary Data` is `physics_system_binary_data` (the patched TAG0 blob).
/// `starfield_write` sets `Target` to the actual root node id when embedding this.
pub struct StarfieldCollisionOut {
    pub bsx_flags_value: u64,
    pub collision_object_flags: u32,
    pub body_id: i32,
    pub physics_system_binary_data: Vec<u8>,
}

/// Minimum full extent (metres) a degenerate/flat AABB-fallback axis is inflated to.
/// `compute_hull_topology_robust` errors on exactly-zero-extent input (R7 §6 step 4).
const DEGENERATE_MIN_EXTENT: f32 = 0.01;

/// The `bodyCinfo` array item's DATA offset inside a blob built by
/// `starfield_convex_collision_blob` / `build_convex_collision`. Fixed because that builder
/// always places the Novablast donor's `fixed_prefix_items` at their original reference
/// offsets (R7 §5) — NOT the same offset as any particular vanilla NIF's own item table.
const STATIC_BODYCINFO_DATA_OFFSET: usize = 304;

/// `_COLLISION_LAYERS["StarfieldLayer"]["STATIC"]` (R7 §2).
const STATIC_LAYER: u32 = 1;

/// `_BSX_HAVOK_MASK` (R7 §3 item 1).
const BSX_HAVOK_FLAG: u64 = 0x02;

/// `bhkCOFlags` `SYNC_ON_UPDATE`, the value vanilla statics use (R7 §3 item 3).
const COLLISION_OBJECT_SYNC_ON_UPDATE: u32 = 0x80;

/// Decode FO4 `bhkPhysicsSystem` collision blocks from a NIF; best-effort, never fails hard.
///
/// - No `bhkPhysicsSystem` block at all: legal "no collision" outcome; returns an
///   empty `Vec`.
/// - A decodable body (box/sphere/capsule/convex/polytope): its vertices carry through 1:1
///   as `CollisionPrim::ConvexHull` — FO4 collision blobs are already Havok metres, the same
///   space as Starfield (R7 §2 "Units").
/// - An undecodable body (mopp / `hknpCompressedMeshShape` / parse failure): falls back to
///   an axis-aligned box over the shape's *render* geometry, divided by `HAVOK_SCALE` to
///   reach metres (R7 §6), with any degenerate axis inflated to a minimum extent.
pub fn extract_fo4_collision_prims(fo4_nif_bytes: &[u8]) -> Result<Vec<CollisionPrim>, String> {
    let nif = NifFile::from_bytes(fo4_nif_bytes, None)
        .map_err(|error| format!("failed to read FO4 NIF: {error}"))?;

    let physics_blocks: Vec<_> = nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkPhysicsSystem")
        .collect();

    if physics_blocks.is_empty() {
        return Ok(Vec::new());
    }

    let mut prims = Vec::new();
    for block in &physics_blocks {
        let Some(binary_data) = block.get_field("Binary Data") else {
            continue;
        };
        let Ok(blob) = crate::cloth::byte_array_to_bytes(binary_data) else {
            continue;
        };
        // havok_scale = 1.0: FO4 collision vertices are already Havok metres, which is
        // Starfield space — no conversion (R7 §5 "Extraction side").
        let Ok(meshes) =
            havok_native::collision::extract_preview_meshes_from_blob(&blob, 1.0, None)
        else {
            continue;
        };
        for mesh in meshes {
            if mesh.shape_type == "compressed_mesh" || mesh.vertices.is_empty() {
                continue;
            }
            prims.push(CollisionPrim::ConvexHull {
                verts: mesh.vertices,
            });
        }
    }

    if !prims.is_empty() {
        return Ok(prims);
    }

    // Collision block(s) exist but nothing decoded (mopp / compressed mesh / parse
    // failure) — fall back to the render-geometry AABB.
    Ok(aabb_fallback_from_render_geometry(&nif)
        .into_iter()
        .collect())
}

/// Emit the Starfield-side collision payload (R7 §5): one hull over the union of all
/// `prims`' vertices, patched onto the STATIC collision layer with static motion
/// properties. `Ok(None)` when `prims` is empty — a legal "no collision" outcome, matching
/// `extract_fo4_collision_prims`'s empty result for a source static with no collision.
pub fn emit_starfield_collision(
    prims: &[CollisionPrim],
) -> Result<Option<StarfieldCollisionOut>, String> {
    let mut verts: Vec<[f32; 3]> = Vec::new();
    for prim in prims {
        match prim {
            CollisionPrim::Box {
                center,
                half_extents,
                rotation,
            } => verts.extend(box_corners(*center, *half_extents, *rotation)),
            CollisionPrim::ConvexHull { verts: hull_verts } => {
                verts.extend(hull_verts.iter().copied());
            }
        }
    }

    if verts.is_empty() {
        return Ok(None);
    }

    let blob = havok_native::api::starfield_convex_collision_blob(
        &verts,
        0.5,
        0.3,
        STATIC_LAYER as u8,
        0.0,
    )
    .map_err(|error| format!("starfield_convex_collision_blob failed: {error}"))?;
    let patched = patch_static_layer(&blob)?;

    Ok(Some(StarfieldCollisionOut {
        bsx_flags_value: BSX_HAVOK_FLAG,
        collision_object_flags: COLLISION_OBJECT_SYNC_ON_UPDATE,
        body_id: 0,
        physics_system_binary_data: patched,
    }))
}

/// Patch the emitted blob's `bodyCinfo` collision layer (STATIC) and `motionPropertiesId`
/// (static) fields (R7 §5). The donor blob backing `starfield_convex_collision_blob` is the
/// Novablast WEAPON, so unpatched output would ship every converted static on the WEAPON
/// layer with dynamic motion properties.
///
/// MANDATORY GUARD: refuses to patch unless the `bodyCinfo` item (`kind == 0x20 && type_idx
/// == 9`) sits at DATA offset 304 — the offset a future donor-blob swap could silently move,
/// which would otherwise make every converted static revert to the WEAPON layer with no
/// visible error.
fn patch_static_layer(blob: &[u8]) -> Result<Vec<u8>, String> {
    let parsed = havok_native::collision::parse_tagged_collision(blob)
        .map_err(|error| format!("failed to parse TAG0 collision blob for patching: {error}"))?;

    let bodycinfo_item = parsed
        .items
        .iter()
        .find(|item| item.kind == 0x20 && item.type_idx == 9)
        .ok_or_else(|| "TAG0 blob has no bodyCinfo (kind=0x20, type_idx=9) item".to_string())?;

    if bodycinfo_item.data_offset != STATIC_BODYCINFO_DATA_OFFSET {
        return Err(format!(
            "donor-blob layout guard failed: bodyCinfo item is at DATA offset {} (expected {STATIC_BODYCINFO_DATA_OFFSET}); \
             refusing to patch — a donor-blob swap must update this guard, or every converted \
             static would silently revert to the WEAPON collision layer",
            bodycinfo_item.data_offset,
        ));
    }

    let data_start = find_tag0_section_offset(blob, "DATA")?;
    let base = data_start + bodycinfo_item.data_offset;
    let mut patched = blob.to_vec();
    write_u32_le(&mut patched, base + 8, 0);
    write_u32_le(&mut patched, base + 16, STATIC_LAYER);
    write_u32_le(&mut patched, base + 40, 0);
    Ok(patched)
}

fn write_u32_le(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Minimal TAG0 section walker mirroring `havok_native`'s private
/// `collision::payload::walk_sections` — locates the absolute byte offset of a leaf
/// section's content by tag name. Reimplemented locally because `walk_sections` is not
/// `pub` in the `havok` crate.
fn find_tag0_section_offset(blob: &[u8], target_tag: &str) -> Result<usize, String> {
    fn walk(data: &[u8], offset: usize, end: usize, target: &str) -> Result<Option<usize>, String> {
        let mut pos = offset;
        while pos < end {
            if pos + 8 > end {
                break;
            }
            let raw = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
            let type_byte = ((raw >> 24) & 0xFF) as u8;
            let size = (raw & 0x00FF_FFFF) as usize;
            if size < 8 || pos + size > end {
                return Err(format!("invalid TAG0 section at offset {pos}: size={size}"));
            }
            let tag = std::str::from_utf8(&data[pos + 4..pos + 8])
                .map_err(|_| "TAG0 section tag is not valid UTF-8".to_string())?;
            let content_start = pos + 8;
            let content_end = pos + size;
            if tag == target {
                return Ok(Some(content_start));
            }
            let is_leaf = (type_byte & 0x40) != 0;
            if !is_leaf {
                if let Some(found) = walk(data, content_start, content_end, target)? {
                    return Ok(Some(found));
                }
            }
            pos += size;
        }
        Ok(None)
    }

    walk(blob, 0, blob.len(), target_tag)?
        .ok_or_else(|| format!("TAG0 blob missing {target_tag} section"))
}

fn box_corners(center: [f32; 3], half_extents: [f32; 3], rotation: [f32; 4]) -> Vec<[f32; 3]> {
    let signs = [-1.0f32, 1.0f32];
    let mut corners = Vec::with_capacity(8);
    for &sx in &signs {
        for &sy in &signs {
            for &sz in &signs {
                let local = [
                    sx * half_extents[0],
                    sy * half_extents[1],
                    sz * half_extents[2],
                ];
                let rotated = rotate_vector(local, rotation);
                corners.push([
                    center[0] + rotated[0],
                    center[1] + rotated[1],
                    center[2] + rotated[2],
                ]);
            }
        }
    }
    corners
}

/// Rotate `v` by quaternion `q = [x, y, z, w]`.
fn rotate_vector(v: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    let [qx, qy, qz, qw] = q;
    let uv = [
        qy * v[2] - qz * v[1],
        qz * v[0] - qx * v[2],
        qx * v[1] - qy * v[0],
    ];
    let uuv = [
        qy * uv[2] - qz * uv[1],
        qz * uv[0] - qx * uv[2],
        qx * uv[1] - qy * uv[0],
    ];
    [
        v[0] + 2.0 * (qw * uv[0] + uuv[0]),
        v[1] + 2.0 * (qw * uv[1] + uuv[1]),
        v[2] + 2.0 * (qw * uv[2] + uuv[2]),
    ]
}

fn aabb_fallback_from_render_geometry(nif: &NifFile) -> Option<CollisionPrim> {
    let mut mins = [f32::INFINITY; 3];
    let mut maxs = [f32::NEG_INFINITY; 3];
    let mut found = false;

    for block in &nif.blocks {
        if !matches!(
            block.type_name.as_str(),
            "BSTriShape" | "BSSubIndexTriShape"
        ) {
            continue;
        }
        let Some(NifValue::Array(entries)) = block.get_field("Vertex Data") else {
            continue;
        };
        for entry in entries {
            let NifValue::Struct(fields) = entry else {
                continue;
            };
            let Some(position) = vec3_value(fields.get("Vertex")) else {
                continue;
            };
            for axis in 0..3 {
                mins[axis] = mins[axis].min(position[axis]);
                maxs[axis] = maxs[axis].max(position[axis]);
            }
            found = true;
        }
    }

    if !found {
        return None;
    }

    // FO4 render units -> Starfield metres (R7 §6 step 2).
    let converted = nif_vertices_to_havok(&[mins, maxs]);
    let (havok_mins, havok_maxs) = (converted[0], converted[1]);

    let mut center = [0.0f32; 3];
    let mut half_extents = [0.0f32; 3];
    for axis in 0..3 {
        center[axis] = (havok_mins[axis] + havok_maxs[axis]) * 0.5;
        let half = (havok_maxs[axis] - havok_mins[axis]) * 0.5;
        half_extents[axis] = half.max(DEGENERATE_MIN_EXTENT * 0.5);
    }

    Some(CollisionPrim::Box {
        center,
        half_extents,
        rotation: [0.0, 0.0, 0.0, 1.0],
    })
}

/// Reads a NIF `Vec3`-shaped field, accepting either the compact `NifValue::Vec3`
/// representation or the `Struct{"x", "y", "z"}` shape the schema-driven reader produces
/// (mirrors `convert_file::vec3_value`, private to that module).
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

fn value_f64(value: Option<&NifValue>) -> Option<f64> {
    match value? {
        NifValue::Float(value) => Some(*value),
        NifValue::Int(value) => Some(*value as f64),
        NifValue::UInt(value) => Some(*value as f64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use crate::fo76_collision::HAVOK_SCALE;

    use super::*;

    // -- FO4 fixture NIF construction -----------------------------------------------
    //
    // Field shapes copied from the proven-roundtripping test
    // `io::writer::tests::new_fo4_root_and_calc_bstrishape_roundtrip`.

    fn vertex_data(vertex: [f32; 3]) -> NifValue {
        let mut data = IndexMap::new();
        data.insert("Vertex".to_string(), NifValue::Vec3(vertex));
        data.insert("Unused W".to_string(), NifValue::UInt(0));
        data.insert("UV".to_string(), tex_coord([0.0, 0.0]));
        data.insert("Normal".to_string(), NifValue::Vec3([0.0, 0.0, 1.0]));
        data.insert("Bitangent Y".to_string(), NifValue::Float(0.0));
        NifValue::Struct(data)
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

    /// Builds a minimal FO4 NIF: one `BSTriShape` carrying `vertices`, and — when
    /// `physics_binary` is `Some` — one `bhkPhysicsSystem` block with that raw blob as its
    /// `Binary Data`.
    fn fo4_fixture_nif_bytes(vertices: &[[f32; 3]], physics_binary: Option<Vec<u8>>) -> Vec<u8> {
        let mut nif = NifFile::new("fo4");

        let mut fields = IndexMap::new();
        fields.insert("Name".to_string(), NifValue::String("Fixture".to_string()));
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
        fields.insert(
            "Bounding Sphere".to_string(),
            NifValue::Struct(IndexMap::new()),
        );
        fields.insert("Skin".to_string(), NifValue::Ref(-1));
        fields.insert("Shader Property".to_string(), NifValue::Ref(-1));
        fields.insert("Alpha Property".to_string(), NifValue::Ref(-1));
        fields.insert(
            "Vertex Desc".to_string(),
            NifValue::Int(193_514_046_685_700),
        );
        fields.insert(
            "Vertex Data".to_string(),
            NifValue::Array(vertices.iter().map(|v| vertex_data(*v)).collect()),
        );
        fields.insert(
            "Num Vertices".to_string(),
            NifValue::UInt(vertices.len() as u64),
        );
        fields.insert(
            "Triangles".to_string(),
            NifValue::Array(vec![triangle(0, 1, 2)]),
        );
        fields.insert("Num Triangles".to_string(), NifValue::UInt(1));
        fields.insert("Data Size".to_string(), NifValue::UInt(0));

        let shape_id = nif.add_block("BSTriShape", Some(fields));

        if let Some(binary) = physics_binary {
            let mut physics_fields = IndexMap::new();
            physics_fields.insert(
                "Binary Data".to_string(),
                crate::cloth::bytes_to_byte_array(&binary),
            );
            nif.add_block("bhkPhysicsSystem", Some(physics_fields));
        }

        let root = nif.blocks.get_mut(0).expect("root block");
        root.set_field("Num Children", NifValue::UInt(1));
        root.set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );

        nif.to_bytes().expect("write fixture NIF")
    }

    /// Decode the `hknpConvexHull::Face` array directly from the ITEM table by fixed
    /// emission order, bypassing `parse_tagged_collision`'s unreliable byte-size
    /// classifier for this array (see the comment at its call site). A convex hull
    /// blob's geometry items (`kind == 0x20`, `count > 1`) always appear in the order
    /// vertices, planes, faces, indices, edges, per-vertex first-edge (R7 §2's item
    /// table) — the faces item is the third by ascending `data_offset`.
    fn decode_faces_directly(
        blob: &[u8],
        data_start: usize,
        parsed: &havok_native::collision::ParsedCollision,
    ) -> Vec<(u16, u8, u8)> {
        let mut geometry_items: Vec<_> = parsed
            .items
            .iter()
            .filter(|item| item.kind == 0x20 && item.count > 1)
            .collect();
        geometry_items.sort_by_key(|item| item.data_offset);
        assert_eq!(
            geometry_items.len(),
            6,
            "expected exactly 6 geometry arrays (vertices/planes/faces/indices/edges/vertex-edges), got {geometry_items:?}"
        );
        let faces_item = geometry_items[2];
        (0..faces_item.count)
            .map(|j| {
                let off = data_start + faces_item.data_offset + j * 4;
                let first_idx = u16::from_le_bytes(blob[off..off + 2].try_into().unwrap());
                (first_idx, blob[off + 2], blob[off + 3])
            })
            .collect()
    }

    /// A structurally real TAG0 blob (built by the production emitter)
    /// whose magic bytes are then zeroed. Every TAG0/packfile parser bails out cleanly at
    /// format detection, so this reliably reaches `extract_fo4_collision_prims`'s "nothing
    /// decoded" fallback branch without risking an out-of-bounds panic in a lower-level
    /// reader that a hand-rolled short garbage buffer might trigger.
    fn unparseable_collision_blob() -> Vec<u8> {
        let verts = box_corners([0.0, 0.0, 0.0], [0.5, 0.5, 0.5], [0.0, 0.0, 0.0, 1.0]);
        let mut blob = havok_native::api::starfield_convex_collision_blob(&verts, 0.5, 0.3, 1, 0.0)
            .expect("build reference blob for corruption");
        blob[0..8].fill(0);
        blob
    }

    // -- (a) 8-vert box -> hull topology 8/6/6/24/24/8, patched layer/motion ------------

    #[test]
    fn eight_vert_box_hull_topology_and_patched_layer() {
        let prim = CollisionPrim::Box {
            center: [0.0, 0.0, 0.0],
            half_extents: [0.5, 0.5, 0.5],
            rotation: [0.0, 0.0, 0.0, 1.0],
        };
        let out = emit_starfield_collision(&[prim])
            .expect("emit must not error")
            .expect("non-empty prims must produce Some");

        let blob = &out.physics_system_binary_data;

        // TAG0 container assertions directly (do NOT use havok_collision_summary /
        // havok_collision_preview; both fail on vanilla blobs).
        assert_eq!(&blob[4..8], b"TAG0", "TAG0 magic");
        let sdkv_start = find_tag0_section_offset(blob, "SDKV").expect("SDKV section");
        assert_eq!(&blob[sdkv_start..sdkv_start + 8], b"20190200", "SDKV");

        let parsed =
            havok_native::collision::parse_tagged_collision(blob).expect("parse TAG0 blob");
        let data_start = find_tag0_section_offset(blob, "DATA").expect("DATA section");
        assert_eq!(parsed.vertices.len(), 8, "hull vertices");
        assert_eq!(parsed.planes.len(), 6, "hull planes");
        // `parsed.faces` is unreliable here: `parse_tagged_collision`'s byte-size
        // classifier infers an item's length from the next item's offset, which
        // includes inter-item alignment padding. For a 6-element (u16,u8,u8) face
        // array that inflates 24 true bytes to 32, pushing elem_size to 5.33 B —
        // outside the parser's `4.0 +/- 0.5` tolerance — so the item is silently
        // dropped instead of classified as faces. Decode it directly from the ITEM
        // table instead, reading `count` fixed-stride records from the item's own
        // `data_offset` rather than inferring a size.
        let faces = decode_faces_directly(blob, data_start, &parsed);
        assert_eq!(faces.len(), 6, "hull faces");
        assert_eq!(parsed.indices.len(), 24, "hull face-index list");
        assert_eq!(parsed.edges.len(), 24, "hull edges");
        assert_eq!(parsed.vertex_edges.len(), 8, "hull per-vertex first-edge");

        let bodycinfo = parsed
            .items
            .iter()
            .find(|item| item.kind == 0x20 && item.type_idx == 9)
            .expect("bodyCinfo item");
        assert_eq!(bodycinfo.data_offset, STATIC_BODYCINFO_DATA_OFFSET);

        let base = data_start + bodycinfo.data_offset;
        let layer = u32::from_le_bytes(blob[base + 16..base + 20].try_into().unwrap());
        let motion = u32::from_le_bytes(blob[base + 40..base + 44].try_into().unwrap());
        assert_eq!(layer, 1, "collision layer must be STATIC after patch");
        assert_eq!(motion, 0, "motionPropertiesId must be static after patch");

        assert_eq!(out.bsx_flags_value, 2);
        assert_eq!(out.collision_object_flags, 0x80);
        assert_eq!(out.body_id, 0);
    }

    // -- (b) guard trips on a doctored blob where the item is not at 304 ---------------

    #[test]
    fn guard_trips_on_doctored_bodycinfo_offset() {
        let verts = box_corners([0.0, 0.0, 0.0], [0.5, 0.5, 0.5], [0.0, 0.0, 0.0, 1.0]);
        let blob = havok_native::api::starfield_convex_collision_blob(&verts, 0.5, 0.3, 1, 0.0)
            .expect("build blob");

        let parsed = havok_native::collision::parse_tagged_collision(&blob).expect("parse");
        let bodycinfo = parsed
            .items
            .iter()
            .find(|item| item.kind == 0x20 && item.type_idx == 9)
            .expect("bodyCinfo item");
        assert_eq!(
            bodycinfo.data_offset, STATIC_BODYCINFO_DATA_OFFSET,
            "precondition: donor layout unchanged"
        );

        let item_start = find_tag0_section_offset(&blob, "ITEM").expect("ITEM section");
        let record_offset = item_start + bodycinfo.index * 12;

        let mut doctored = blob.clone();
        // Move the bodyCinfo item's declared DATA offset away from 304.
        doctored[record_offset + 4..record_offset + 8].copy_from_slice(&999u32.to_le_bytes());

        let error =
            patch_static_layer(&doctored).expect_err("guard must reject an item not at offset 304");
        assert!(
            error.contains("304"),
            "error should cite the expected offset: {error}"
        );
        assert!(
            error.contains("999"),
            "error should cite the actual (doctored) offset: {error}"
        );
    }

    // -- (c) unparseable FO4 collision -> AABB fallback box of render bounds / 69.99125 -

    #[test]
    fn unparseable_fo4_collision_falls_back_to_render_aabb() {
        // FO4 render-space unit cube: 0..HAVOK_SCALE on every axis -> 0..1 m.
        let vertices = [
            [0.0, 0.0, 0.0],
            [HAVOK_SCALE, 0.0, 0.0],
            [0.0, HAVOK_SCALE, 0.0],
            [0.0, 0.0, HAVOK_SCALE],
        ];
        let nif_bytes = fo4_fixture_nif_bytes(&vertices, Some(unparseable_collision_blob()));

        let prims = extract_fo4_collision_prims(&nif_bytes).expect("extract");
        assert_eq!(prims.len(), 1);
        match &prims[0] {
            CollisionPrim::Box {
                center,
                half_extents,
                ..
            } => {
                for axis in 0..3 {
                    assert!(
                        (half_extents[axis] - 0.5).abs() < 1e-3,
                        "axis {axis} half-extent: {half_extents:?}"
                    );
                    assert!(
                        (center[axis] - 0.5).abs() < 1e-3,
                        "axis {axis} center: {center:?}"
                    );
                }
            }
            other => panic!("expected AABB Box fallback, got {other:?}"),
        }
    }

    // -- (d) degenerate flat axis inflated ----------------------------------------------

    #[test]
    fn degenerate_flat_axis_is_inflated() {
        // A flat plaque: zero extent on Z.
        let z = 5.0 * HAVOK_SCALE;
        let vertices = [
            [0.0, 0.0, z],
            [HAVOK_SCALE, 0.0, z],
            [0.0, HAVOK_SCALE, z],
            [HAVOK_SCALE, HAVOK_SCALE, z],
        ];
        let nif_bytes = fo4_fixture_nif_bytes(&vertices, Some(unparseable_collision_blob()));

        let prims = extract_fo4_collision_prims(&nif_bytes).expect("extract");
        assert_eq!(prims.len(), 1);
        let CollisionPrim::Box { half_extents, .. } = &prims[0] else {
            panic!("expected AABB Box fallback");
        };
        assert!(
            (half_extents[2] - DEGENERATE_MIN_EXTENT * 0.5).abs() < 1e-6,
            "flat Z axis must be inflated to the minimum extent, got {}",
            half_extents[2]
        );
        // Non-degenerate axes are untouched.
        assert!((half_extents[0] - 0.5).abs() < 1e-3);
        assert!((half_extents[1] - 0.5).abs() < 1e-3);

        // The practical purpose of the inflation: hull computation must not error on the
        // resulting (non-coplanar) box.
        let emitted =
            emit_starfield_collision(&prims).expect("emit must not error on inflated box");
        assert!(emitted.is_some());
    }
}
