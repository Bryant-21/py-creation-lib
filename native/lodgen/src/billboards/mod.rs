/// Rust-side consumer of the Python billboard generator's output.
///
/// The Python generator (`creation_lib.lod.billboards`) renders each tree species
/// offscreen, packs them into an RGBA atlas, and writes:
///   - an atlas DDS (`naming::billboard_atlas`)
///   - a companion `_n.dds` flat-normal atlas
///   - this JSON manifest
///
/// The Rust billboard-placement path (`trees::billboard_place`) loads the manifest
/// and uses it to assign tree refs to species indices and UV rects.
///
/// The manifest schema is the cross-language contract — both sides MUST agree.
/// The Python side writes exactly these field names; see `manifest_schema_keys`
/// test which pins the contract.
///
/// Field set mirrors `TwbLodTES5TreeType` UV rect fields (wbLOD.pas:847-855) so the
/// placement math in `billboard_place::generate_quad` is identical to the Pascal original.
use std::path::Path;

// ---------------------------------------------------------------------------
// BillboardEntry — one rendered species
// ---------------------------------------------------------------------------

/// One rendered tree species in the billboard atlas.
///
/// `uv_*` coordinates are atlas-normalized [0, 1].
/// `model` is the source LOD model path (the lookup key for `by_model`).
/// `index` is the tree-list index the Python generator assigned (stable across runs
/// because the generator sorts species by model path before packing).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BillboardEntry {
    /// Source LOD model path (the key), e.g. `meshes/trees/pinetree01_lod.nif`.
    pub model: String,
    /// Tree-list index (0-based, assigned by the generator in model-sorted order).
    pub index: i32,
    /// In-game billboard width in game units (from the .txt sidecar or record).
    pub width: f32,
    /// In-game billboard height in game units.
    pub height: f32,
    /// Z-shift of the billboard origin (lift off the ground).
    pub shift_z: f32,
    /// Atlas UV left bound [0, 1].
    pub uv_min_x: f32,
    /// Atlas UV right bound [0, 1].
    pub uv_max_x: f32,
    /// Atlas UV top bound [0, 1] (V=0 is top in the atlas image convention).
    pub uv_min_y: f32,
    /// Atlas UV bottom bound [0, 1].
    pub uv_max_y: f32,
}

// ---------------------------------------------------------------------------
// BillboardManifest — the generator's output manifest
// ---------------------------------------------------------------------------

/// The full output manifest written by `creation_lib.lod.billboards.write_manifest`.
///
/// `atlas` and `atlas_normal` are Data-relative paths (e.g.
/// `Textures\Terrain\LODGen\World\WorldTreeLod.dds`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BillboardManifest {
    /// Data-relative path to the diffuse atlas DDS.
    pub atlas: String,
    /// Data-relative path to the normal atlas DDS (flat-normal sibling, `_n.dds`).
    pub atlas_normal: String,
    /// Atlas width in pixels (power-of-two).
    pub atlas_w: u32,
    /// Atlas height in pixels (power-of-two).
    pub atlas_h: u32,
    /// Per-species billboard entries, sorted by `index` (deterministic).
    pub entries: Vec<BillboardEntry>,
}

impl BillboardManifest {
    /// Load a manifest from a JSON file on disk.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let f = std::fs::File::open(path)
            .map_err(|e| anyhow::anyhow!("cannot open manifest {:?}: {e}", path))?;
        let m: BillboardManifest = serde_json::from_reader(f)
            .map_err(|e| anyhow::anyhow!("cannot parse manifest {:?}: {e}", path))?;
        Ok(m)
    }

    /// Look up a species entry by model path (case-insensitive substring match).
    ///
    /// Matches if the entry's `model` lowercased contains `model` lowercased,
    /// or if the entry's model stem (filename without extension) contains the query stem.
    /// Returns the first match.
    pub fn by_model(&self, model: &str) -> Option<&BillboardEntry> {
        let q = model.to_lowercase();
        // Exact (case-insensitive) match first
        if let Some(e) = self.entries.iter().find(|e| e.model.to_lowercase() == q) {
            return Some(e);
        }
        // Stem/substring match: query "pinetree01" matches "meshes/trees/pinetree01_lod.nif"
        self.entries.iter().find(|e| {
            let m_lower = e.model.to_lowercase();
            // Extract the stem (filename without any extension)
            let stem = m_lower
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&m_lower)
                .split('.')
                .next()
                .unwrap_or(&m_lower);
            stem.contains(&q) || q.contains(stem)
        })
    }
}
