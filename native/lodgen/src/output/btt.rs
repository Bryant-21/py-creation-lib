/// Tree LOD block (`.btt`) and list (`.lst`) writers.
///
/// FO4 has no native `.btt` consumer; these writers serve billboard-mode output
/// (trees_3d=false) and other games. FO4's default 3D tree LOD folds trees into `.bto`.
///
/// Port sources:
/// - `encode_tree_block` → TwbLodTES5TreeBlock.SaveToFile (wbLOD.pas:939-961)
/// - `encode_tree_list`  → TwbLodTES5TreeList.SaveToFile (wbLOD.pas:701-713)
/// - `TreeRef`           → TwbLodTES5TreeRef packed record layout (wbLOD.pas:125-131)
/// - `LstEntry`          → TwbLodTES5TreeType (wbLOD.pas:115-122)
use std::path::Path;

// ---------------------------------------------------------------------------
// TreeRef — one billboard reference
// ---------------------------------------------------------------------------

/// One billboard tree reference for the `.btt` block.
///
/// On disk this is the Pascal `packed record TwbLodTES5TreeRef` (wbLOD.pas:125-131),
/// blitted in declaration order by `SaveToFile` (`Write(Refs[i][0], SizeOf(...)*Count)` :950):
///   X(f32), Y(f32), Z(f32), Rotation(f32), Scale(f32), RefFormID(u32), Unknown1(i32)=0, Unknown2(i32)=0
/// = 32 bytes. The two `Unknown` slots are always zero and not stored here;
/// `encode_tree_block` emits the full 32-byte packed layout.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRef {
    pub form_id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub scale: f32,
    pub rotation: f32,
}

// ---------------------------------------------------------------------------
// TreeType — one tree type bucket within a block
// ---------------------------------------------------------------------------

/// One tree type (species) bucket inside a `.btt` block.
pub struct TreeType {
    pub index: i32,
    pub refs: Vec<TreeRef>,
}

// ---------------------------------------------------------------------------
// encode_tree_block — port: TwbLodTES5TreeBlock.SaveToFile (wbLOD.pas:939-961)
// ---------------------------------------------------------------------------

/// Encode the `.btt` block body:
/// `[i32 numTypes][per type: i32 index, i32 count, TreeRef[count]]`.
///
/// Each `TreeRef` is the 32-byte packed `TwbLodTES5TreeRef` (wbLOD.pas:125-131), LE:
///   X, Y, Z, Rotation, Scale (5×f32), RefFormID (u32), Unknown1, Unknown2 (2×i32 = 0).
/// Iteration is insertion-order (wbLOD.pas:947 iterates 0..Length(Types)).
///
/// port: TwbLodTES5TreeBlock.SaveToFile (wbLOD.pas:939-961)
pub fn encode_tree_block(types: &[TreeType]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    // [i32 numTypes]
    buf.extend_from_slice(&(types.len() as i32).to_le_bytes());
    for tt in types {
        // [i32 index]
        buf.extend_from_slice(&tt.index.to_le_bytes());
        // [i32 count]
        buf.extend_from_slice(&(tt.refs.len() as i32).to_le_bytes());
        for r in &tt.refs {
            // TwbLodTES5TreeRef packed layout (declaration order, wbLOD.pas:125-131):
            // X, Y, Z, Rotation, Scale, RefFormID, Unknown1, Unknown2.
            buf.extend_from_slice(&r.x.to_le_bytes());
            buf.extend_from_slice(&r.y.to_le_bytes());
            buf.extend_from_slice(&r.z.to_le_bytes());
            buf.extend_from_slice(&r.rotation.to_le_bytes());
            buf.extend_from_slice(&r.scale.to_le_bytes());
            buf.extend_from_slice(&r.form_id.to_le_bytes());
            buf.extend_from_slice(&0i32.to_le_bytes()); // Unknown1
            buf.extend_from_slice(&0i32.to_le_bytes()); // Unknown2
        }
    }
    buf
}

/// Write the `.btt` block to `path`.
pub fn write_tree_block(path: &Path, types: &[TreeType]) -> anyhow::Result<()> {
    let bytes = encode_tree_block(types);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &bytes)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// LstEntry — port: TwbLodTES5TreeType (wbLOD.pas:115-122)
// ---------------------------------------------------------------------------

/// One tree-list entry for the `.lst` file.
///
/// Mirrors `TwbLodTES5TreeType` (wbLOD.pas:115-122):
/// `Index(i32), Width(f32), Height(f32), UVMinX(f32), UVMinY(f32), UVMaxX(f32), UVMaxY(f32), Unknown(i32)`.
/// Total = 8 × 4 = 32 bytes per entry.
pub struct LstEntry {
    pub index: i32,
    pub width: f32,
    pub height: f32,
    pub uv_min_x: f32,
    pub uv_max_x: f32,
    pub uv_min_y: f32,
    pub uv_max_y: f32,
}

// ---------------------------------------------------------------------------
// encode_tree_list — port: TwbLodTES5TreeList.SaveToFile (wbLOD.pas:701-713)
// ---------------------------------------------------------------------------

/// Encode the `.lst` tree list:
/// `[i32 numTrees][TwbLodTES5TreeType × numTrees]`.
///
/// `TwbLodTES5TreeType` is 8 × 4 = 32 bytes per entry (packed).
///
/// port: TwbLodTES5TreeList.SaveToFile (wbLOD.pas:701-713)
pub fn encode_tree_list(entries: &[LstEntry]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    // [i32 numTrees]
    buf.extend_from_slice(&(entries.len() as i32).to_le_bytes());
    for e in entries {
        // TwbLodTES5TreeType layout: Index, Width, Height, UVMinX, UVMinY, UVMaxX, UVMaxY, Unknown
        buf.extend_from_slice(&e.index.to_le_bytes());
        buf.extend_from_slice(&e.width.to_le_bytes());
        buf.extend_from_slice(&e.height.to_le_bytes());
        buf.extend_from_slice(&e.uv_min_x.to_le_bytes());
        buf.extend_from_slice(&e.uv_min_y.to_le_bytes());
        buf.extend_from_slice(&e.uv_max_x.to_le_bytes());
        buf.extend_from_slice(&e.uv_max_y.to_le_bytes());
        // Unknown field = 0 (wbLOD.pas:121 — `Unknown: Integer`)
        buf.extend_from_slice(&0i32.to_le_bytes());
    }
    buf
}

/// Write the `.lst` tree list to `path`.
pub fn write_tree_list(path: &Path, entries: &[LstEntry]) -> anyhow::Result<()> {
    let bytes = encode_tree_list(entries);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &bytes)?;
    Ok(())
}
