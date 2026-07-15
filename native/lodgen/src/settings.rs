use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Format {
    Bc1,
    Bc2,
    Bc3,
    Bc5,
    Bc7,
    Rgba8,
    Bgr565,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum OptimizeUnseen {
    Off,
    On,
    Depth(f32),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChunkBounds {
    pub level: i32,
    pub w: i32,
    pub s: i32,
    pub e: i32,
    pub n: i32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LodBounds {
    pub w: i32,
    pub s: i32,
    pub e: i32,
    pub n: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GlobalSettings {
    pub worldspaces: Vec<String>,
    pub lod_min: i32,
    pub lod_max: i32,
    pub stride: Option<i32>,
    pub align: i32,
    #[serde(default, alias = "sw_cell")]
    pub southwest_cell: Option<[i32; 2]>,
    #[serde(default)]
    pub bounds: Option<LodBounds>,
    pub write_lodsettings: bool,
    pub workers: usize,
    pub season: Option<String>,
    pub chunk: Option<ChunkBounds>,
    #[serde(default = "default_generate_phase")]
    pub generate_terrain: bool,
    #[serde(default = "default_generate_phase")]
    pub generate_objects: bool,
    #[serde(default = "default_generate_phase")]
    pub generate_trees: bool,
}

fn default_generate_phase() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerrainLevel {
    pub quality: f32,
    pub max_vertices: u32,
    pub optimize_unseen: OptimizeUnseen,
    pub diffuse_size: u32,
    pub diffuse_format: Format,
    pub diffuse_mipmap: bool,
    pub normal_size: u32,
    pub normal_format: Format,
    pub normal_mipmap: bool,
    pub normal_rise: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerrainSettings {
    pub levels: [TerrainLevel; 4],
    pub protect_cell_borders: bool,
    pub hide_quads: bool,
    pub skirts: i32,
    pub underside: bool,
    pub heightmaps: bool,
    pub brightness: f32,
    pub contrast: f32,
    pub gamma: [f32; 3],
    pub vertex_color_intensity: f32,
    pub bake_normals: bool,
    pub bake_specular: bool,
    pub default_diffuse_size: Option<u32>,
    pub default_normal_size: Option<u32>,
    /// Emit the landless/ocean WATER block (2nd `BSTriShape` +
    /// `BSEffectShaderProperty` under a `BSMultiBoundNode "WATER"`) in coarse
    /// `.btr` tiles. Default ON: shipped FO4/xLODGen terrain emits this shape
    /// where water data exists, and disabling it removes visible far-water sheets.
    /// `serde(default)` keeps pre-existing settings JSON deserializing.
    #[serde(default = "default_emit_water")]
    pub emit_water: bool,
}

/// Default for the additive `emit_water` field (ON — see `TerrainSettings`).
fn default_emit_water() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectSource {
    Records,
    Fo76Bto,
    Fo76BtoAtlas,
}

fn default_object_source() -> ObjectSource {
    ObjectSource::Records
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Fo76BtoMultiboundMode {
    Shape,
    Tile,
}

fn default_fo76_bto_multibound_mode() -> Fo76BtoMultiboundMode {
    Fo76BtoMultiboundMode::Shape
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Fo76BtoNodeLayout {
    Fo4PerShape,
    Fo76Grouped,
}

fn default_fo76_bto_node_layout() -> Fo76BtoNodeLayout {
    Fo76BtoNodeLayout::Fo4PerShape
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ObjectSettings {
    #[serde(default = "default_object_source")]
    pub source: ObjectSource,
    pub build_atlas: bool,
    pub atlas_size: u32,
    #[serde(default)]
    pub atlas_mip_flooding: bool,
    pub uv_range: f32,
    pub diffuse_format: Format,
    pub normal_format: Format,
    pub specular_format: Format,
    pub max_tile_size: u32,
    pub alpha_threshold: u8,
    pub use_alpha_threshold: bool,
    pub use_backlight: bool,
    pub no_vertex_colors: bool,
    pub no_tangents: bool,
    pub remove_unseen_faces: bool,
    #[serde(default = "default_qem_decimate_full_model_lod")]
    pub qem_decimate_full_model_lod: bool,
    #[serde(default = "default_qem_lod4_ratio")]
    pub qem_lod4_ratio: f32,
    #[serde(default = "default_qem_lod8_ratio")]
    pub qem_lod8_ratio: f32,
    #[serde(default = "default_qem_lod16_ratio")]
    pub qem_lod16_ratio: f32,
    #[serde(default = "default_qem_lod32_ratio")]
    pub qem_lod32_ratio: f32,
    #[serde(default = "default_meshopt_decimate_object_lod")]
    pub meshopt_decimate_object_lod: bool,
    #[serde(default = "default_meshopt_decimate_model_lod")]
    pub meshopt_decimate_model_lod: bool,
    #[serde(default = "default_meshopt_full_model_ratios")]
    pub meshopt_full_model_ratios: [f32; 4],
    #[serde(default = "default_meshopt_lod_model_ratios")]
    pub meshopt_lod_model_ratios: [f32; 4],
    #[serde(default = "default_meshopt_alpha_ratios")]
    pub meshopt_alpha_ratios: [f32; 4],
    #[serde(default = "default_meshopt_target_errors")]
    pub meshopt_target_errors: [f32; 4],
    #[serde(default = "default_meshopt_sloppy_from_lod")]
    pub meshopt_sloppy_from_lod: i32,
    #[serde(default = "default_meshopt_quad_tri_budgets")]
    pub meshopt_quad_tri_budgets: [u32; 4],
    #[serde(default = "default_object_lod_top_model_count")]
    pub object_lod_top_model_count: usize,
    #[serde(default = "default_object_lod_huge_bto_warn_mb")]
    pub object_lod_huge_bto_warn_mb: u64,
    #[serde(default = "default_object_lod_model_cache_mb")]
    pub object_lod_model_cache_mb: u64,
    #[serde(default = "default_fo76_bto_include_baked")]
    pub fo76_bto_include_baked: bool,
    #[serde(default = "default_fo76_bto_include_global_atlas_baked")]
    pub fo76_bto_include_global_atlas_baked: bool,
    #[serde(default = "default_fo76_bto_include_remeshed_baked")]
    pub fo76_bto_include_remeshed_baked: bool,
    #[serde(default = "default_fo76_bto_include_instances")]
    pub fo76_bto_include_instances: bool,
    #[serde(default = "default_fo76_bto_include_tree_instances")]
    pub fo76_bto_include_tree_instances: bool,
    #[serde(default = "default_fo76_bto_atlas_pages")]
    pub fo76_bto_atlas_pages: bool,
    #[serde(default = "default_fo76_bto_merge_atlassed_shapes")]
    pub fo76_bto_merge_atlassed_shapes: bool,
    #[serde(default = "default_fo76_bto_atlas_min_tile_size")]
    pub fo76_bto_atlas_min_tile_size: u32,
    #[serde(default = "default_fo76_bto_atlas_min_foliage_tile_size")]
    pub fo76_bto_atlas_min_foliage_tile_size: u32,
    #[serde(default = "default_fo76_bto_atlas_foliage_page_size")]
    pub fo76_bto_atlas_foliage_page_size: u32,
    #[serde(default = "default_fo76_bto_atlas_foliage_max_tile_size")]
    pub fo76_bto_atlas_foliage_max_tile_size: u32,
    #[serde(default = "default_fo76_bto_atlas_min_alpha_tested_tile_size")]
    pub fo76_bto_atlas_min_alpha_tested_tile_size: u32,
    #[serde(default = "default_fo76_bto_atlas_alpha_tested_page_size")]
    pub fo76_bto_atlas_alpha_tested_page_size: u32,
    #[serde(default = "default_fo76_bto_atlas_alpha_tested_max_tile_size")]
    pub fo76_bto_atlas_alpha_tested_max_tile_size: u32,
    #[serde(default = "default_fo76_bto_atlas_from_lod")]
    pub fo76_bto_atlas_from_lod: Option<i32>,
    #[serde(default = "default_fo76_bto_tree_billboard_from_lod")]
    pub fo76_bto_tree_billboard_from_lod: Option<i32>,
    #[serde(default = "default_fo76_bto_multibound_mode")]
    pub fo76_bto_multibound_mode: Fo76BtoMultiboundMode,
    #[serde(default = "default_fo76_bto_node_layout")]
    pub fo76_bto_node_layout: Fo76BtoNodeLayout,
}

fn default_qem_decimate_full_model_lod() -> bool {
    false
}

fn default_qem_lod4_ratio() -> f32 {
    0.35
}

fn default_qem_lod8_ratio() -> f32 {
    0.18
}

fn default_qem_lod16_ratio() -> f32 {
    0.08
}

fn default_qem_lod32_ratio() -> f32 {
    0.03
}

fn default_meshopt_decimate_object_lod() -> bool {
    false
}

fn default_meshopt_decimate_model_lod() -> bool {
    false
}

fn default_meshopt_full_model_ratios() -> [f32; 4] {
    [0.30, 0.12, 0.025, 0.008]
}

fn default_meshopt_lod_model_ratios() -> [f32; 4] {
    [0.80, 0.55, 0.22, 0.08]
}

fn default_meshopt_alpha_ratios() -> [f32; 4] {
    [0.55, 0.30, 0.08, 0.025]
}

fn default_meshopt_target_errors() -> [f32; 4] {
    [0.005, 0.01, 0.025, 0.05]
}

fn default_meshopt_sloppy_from_lod() -> i32 {
    16
}

fn default_meshopt_quad_tri_budgets() -> [u32; 4] {
    [220_000, 120_000, 50_000, 20_000]
}

fn default_object_lod_top_model_count() -> usize {
    10
}

fn default_object_lod_huge_bto_warn_mb() -> u64 {
    64
}

fn default_object_lod_model_cache_mb() -> u64 {
    0
}

fn default_fo76_bto_include_baked() -> bool {
    true
}

fn default_fo76_bto_include_global_atlas_baked() -> bool {
    true
}

fn default_fo76_bto_include_remeshed_baked() -> bool {
    true
}

fn default_fo76_bto_include_instances() -> bool {
    true
}

fn default_fo76_bto_include_tree_instances() -> bool {
    true
}

fn default_fo76_bto_atlas_pages() -> bool {
    false
}

fn default_fo76_bto_merge_atlassed_shapes() -> bool {
    false
}

fn default_fo76_bto_atlas_min_tile_size() -> u32 {
    0
}

fn default_fo76_bto_atlas_min_foliage_tile_size() -> u32 {
    0
}

fn default_fo76_bto_atlas_foliage_page_size() -> u32 {
    0
}

fn default_fo76_bto_atlas_foliage_max_tile_size() -> u32 {
    0
}

fn default_fo76_bto_atlas_min_alpha_tested_tile_size() -> u32 {
    0
}

fn default_fo76_bto_atlas_alpha_tested_page_size() -> u32 {
    0
}

fn default_fo76_bto_atlas_alpha_tested_max_tile_size() -> u32 {
    0
}

fn default_fo76_bto_atlas_from_lod() -> Option<i32> {
    None
}

fn default_fo76_bto_tree_billboard_from_lod() -> Option<i32> {
    None
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TreeSettings {
    pub trees_3d: bool,
    pub generate_billboards: bool,
    pub billboard_atlas_size: u32,
    pub billboard_brightness: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LodSettings {
    pub global: GlobalSettings,
    pub terrain: TerrainSettings,
    pub objects: ObjectSettings,
    pub trees: TreeSettings,
}

fn terrain_level(quality: f32, diffuse_mipmap: bool) -> TerrainLevel {
    // Native terrain LOD tiles are 256x256 BC1 at every level (xLODGen golden
    // corpus; 128x128 only for default/empty cells, see default_*_size). Mips
    // are written ONLY on L4 diffuse; L8/L16/L32 diffuse and all _msn are
    // single-mip (verified against tmp/xlodgen/Textures/Terrain/DLC03FarHarbor).
    TerrainLevel {
        quality,
        max_vertices: 32767,
        optimize_unseen: OptimizeUnseen::Off,
        diffuse_size: 256,
        diffuse_format: Format::Bc1,
        diffuse_mipmap,
        normal_size: 256,
        normal_format: Format::Bc1,
        normal_mipmap: false,
        normal_rise: 1.0,
    }
}

/// Returns a compact JSON string of the default FO4 LOD settings.
///
/// Consumed by Python callers via `lodgen_native.default_settings_json()` and
/// passed back into `generate_lod` as the `settings_json` argument. Using this
/// function instead of hand-rolling JSON ensures Python and Rust always agree on
/// the schema, and is the round-trip anchor for the cross-language snapshot test.
pub fn default_settings_json() -> String {
    serde_json::to_string(&LodSettings::fo4_default())
        .expect("LodSettings serialization must not fail")
}

impl LodSettings {
    /// xLODGen default profile for FO4 (Program.cs:205-206, R1 §4, R3 appendix)
    pub fn fo4_default() -> Self {
        LodSettings {
            global: GlobalSettings {
                worldspaces: Vec::new(),
                lod_min: 4,
                lod_max: 32,
                stride: None,
                align: 0,
                southwest_cell: None,
                bounds: None,
                write_lodsettings: true,
                workers: 0,
                season: None,
                chunk: None,
                generate_terrain: true,
                generate_objects: true,
                generate_trees: true,
            },
            terrain: TerrainSettings {
                levels: [
                    terrain_level(10.0, true),  // L4: diffuse mips only here
                    terrain_level(15.0, false), // L8
                    terrain_level(20.0, false), // L16
                    terrain_level(25.0, false), // L32
                ],
                protect_cell_borders: true,
                hide_quads: false,
                skirts: 256,
                underside: false,
                heightmaps: false,
                brightness: 0.0,
                contrast: 1.0,
                gamma: [1.0, 1.0, 1.0],
                vertex_color_intensity: 1.0,
                bake_normals: false,
                bake_specular: false,
                default_diffuse_size: Some(128),
                default_normal_size: Some(128),
                emit_water: true,
            },
            objects: ObjectSettings {
                source: ObjectSource::Records,
                build_atlas: true,
                atlas_size: 4096,
                atlas_mip_flooding: false,
                uv_range: 1.5,
                diffuse_format: Format::Bc2,
                normal_format: Format::Bc1,
                specular_format: Format::Bc5,
                max_tile_size: 512,
                alpha_threshold: 128,
                use_alpha_threshold: true,
                use_backlight: false,
                no_vertex_colors: false,
                no_tangents: false,
                remove_unseen_faces: true,
                qem_decimate_full_model_lod: false,
                qem_lod4_ratio: 0.35,
                qem_lod8_ratio: 0.18,
                qem_lod16_ratio: 0.08,
                qem_lod32_ratio: 0.03,
                meshopt_decimate_object_lod: false,
                meshopt_decimate_model_lod: false,
                meshopt_full_model_ratios: [0.30, 0.12, 0.025, 0.008],
                meshopt_lod_model_ratios: [0.80, 0.55, 0.22, 0.08],
                meshopt_alpha_ratios: [0.55, 0.30, 0.08, 0.025],
                meshopt_target_errors: [0.005, 0.01, 0.025, 0.05],
                meshopt_sloppy_from_lod: 16,
                meshopt_quad_tri_budgets: [220_000, 120_000, 50_000, 20_000],
                object_lod_top_model_count: 10,
                object_lod_huge_bto_warn_mb: 64,
                object_lod_model_cache_mb: 0,
                fo76_bto_include_baked: true,
                fo76_bto_include_global_atlas_baked: true,
                fo76_bto_include_remeshed_baked: true,
                fo76_bto_include_instances: true,
                fo76_bto_include_tree_instances: true,
                fo76_bto_atlas_pages: false,
                fo76_bto_merge_atlassed_shapes: false,
                fo76_bto_atlas_min_tile_size: 0,
                fo76_bto_atlas_min_foliage_tile_size: 0,
                fo76_bto_atlas_foliage_page_size: 0,
                fo76_bto_atlas_foliage_max_tile_size: 0,
                fo76_bto_atlas_min_alpha_tested_tile_size: 0,
                fo76_bto_atlas_alpha_tested_page_size: 0,
                fo76_bto_atlas_alpha_tested_max_tile_size: 0,
                fo76_bto_atlas_from_lod: None,
                fo76_bto_tree_billboard_from_lod: None,
                fo76_bto_multibound_mode: Fo76BtoMultiboundMode::Shape,
                fo76_bto_node_layout: Fo76BtoNodeLayout::Fo4PerShape,
            },
            trees: TreeSettings {
                trees_3d: true,
                generate_billboards: false,
                billboard_atlas_size: 2048,
                billboard_brightness: 1.0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fo4_default_matches_xlodgen_profile() {
        let s = LodSettings::fo4_default();
        // Global
        assert_eq!(s.global.lod_min, 4);
        assert_eq!(s.global.lod_max, 32);
        assert_eq!(s.global.southwest_cell, None);
        assert_eq!(s.global.bounds, None);
        assert!(s.global.write_lodsettings);
        // Terrain per-level quality defaults 10/15/20/25 (R1 §4, Program.cs:205)
        assert_eq!(s.terrain.levels[0].quality, 10.0); // LOD4
        assert_eq!(s.terrain.levels[1].quality, 15.0); // LOD8
        assert_eq!(s.terrain.levels[2].quality, 20.0); // LOD16
        assert_eq!(s.terrain.levels[3].quality, 25.0); // LOD32
        // Max-Vertices cap default 32767 (Program.cs:206)
        assert_eq!(s.terrain.levels[0].max_vertices, 32767);
        // Native terrain tile size = 256x256 BC1 at EVERY level (xLODGen golden
        // corpus DLC03FarHarbor; higher "settings" sizes are not used by FO4).
        assert_eq!(s.terrain.levels[0].diffuse_size, 256);
        assert_eq!(s.terrain.levels[3].diffuse_size, 256);
        assert_eq!(s.terrain.levels[0].normal_size, 256);
        // Default/empty-cell tiles are 128x128.
        assert_eq!(s.terrain.default_diffuse_size, Some(128));
        assert_eq!(s.terrain.default_normal_size, Some(128));
        // Mips ONLY on L4 diffuse; L8/L16/L32 diffuse + all _msn are single-mip.
        assert!(s.terrain.levels[0].diffuse_mipmap); // L4
        assert!(!s.terrain.levels[1].diffuse_mipmap); // L8
        assert!(!s.terrain.levels[2].diffuse_mipmap); // L16
        assert!(!s.terrain.levels[3].diffuse_mipmap); // L32
        assert!(s.terrain.levels.iter().all(|l| !l.normal_mipmap)); // _msn never
        // Terrain globals
        assert!(s.terrain.protect_cell_borders); // default true (R1 §6)
        assert!(!s.terrain.hide_quads); // default false (R1 §7)
        assert_eq!(s.terrain.skirts, 256); // default 256 (R1 §8)
        assert_eq!(s.terrain.vertex_color_intensity, 1.0);
        // Object defaults
        assert_eq!(s.objects.source, ObjectSource::Records);
        assert!(s.objects.build_atlas);
        assert_eq!(s.objects.atlas_size, 4096);
        assert!(!s.objects.atlas_mip_flooding);
        assert_eq!(s.objects.uv_range, 1.5);
        assert_eq!(s.objects.alpha_threshold, 128); // R3 appendix
        assert!(!s.objects.qem_decimate_full_model_lod);
        assert_eq!(s.objects.qem_lod4_ratio, 0.35);
        assert_eq!(s.objects.qem_lod8_ratio, 0.18);
        assert_eq!(s.objects.qem_lod16_ratio, 0.08);
        assert_eq!(s.objects.qem_lod32_ratio, 0.03);
        assert!(!s.objects.meshopt_decimate_object_lod);
        assert!(!s.objects.meshopt_decimate_model_lod);
        assert_eq!(
            s.objects.meshopt_full_model_ratios,
            [0.30, 0.12, 0.025, 0.008]
        );
        assert_eq!(s.objects.meshopt_lod_model_ratios, [0.80, 0.55, 0.22, 0.08]);
        assert_eq!(s.objects.meshopt_alpha_ratios, [0.55, 0.30, 0.08, 0.025]);
        assert_eq!(s.objects.meshopt_target_errors, [0.005, 0.01, 0.025, 0.05]);
        assert_eq!(s.objects.meshopt_sloppy_from_lod, 16);
        assert_eq!(
            s.objects.meshopt_quad_tri_budgets,
            [220_000, 120_000, 50_000, 20_000]
        );
        assert_eq!(s.objects.object_lod_top_model_count, 10);
        assert_eq!(s.objects.object_lod_huge_bto_warn_mb, 64);
        assert_eq!(s.objects.object_lod_model_cache_mb, 0);
        assert!(s.objects.fo76_bto_include_baked);
        assert!(s.objects.fo76_bto_include_global_atlas_baked);
        assert!(s.objects.fo76_bto_include_remeshed_baked);
        assert!(s.objects.fo76_bto_include_instances);
        assert!(s.objects.fo76_bto_include_tree_instances);
        assert!(!s.objects.fo76_bto_atlas_pages);
        assert!(!s.objects.fo76_bto_merge_atlassed_shapes);
        assert_eq!(s.objects.fo76_bto_atlas_min_tile_size, 0);
        assert_eq!(s.objects.fo76_bto_atlas_min_foliage_tile_size, 0);
        assert_eq!(s.objects.fo76_bto_atlas_foliage_page_size, 0);
        assert_eq!(s.objects.fo76_bto_atlas_foliage_max_tile_size, 0);
        assert_eq!(s.objects.fo76_bto_atlas_min_alpha_tested_tile_size, 0);
        assert_eq!(s.objects.fo76_bto_atlas_alpha_tested_page_size, 0);
        assert_eq!(s.objects.fo76_bto_atlas_alpha_tested_max_tile_size, 0);
        assert_eq!(s.objects.fo76_bto_atlas_from_lod, None);
        assert_eq!(s.objects.fo76_bto_tree_billboard_from_lod, None);
        assert_eq!(
            s.objects.fo76_bto_multibound_mode,
            Fo76BtoMultiboundMode::Shape
        );
        assert_eq!(
            s.objects.fo76_bto_node_layout,
            Fo76BtoNodeLayout::Fo4PerShape
        );
        // Trees
        assert!(s.trees.trees_3d);
    }

    /// Settings JSON that predates additive object fields must still deserialize.
    #[test]
    fn legacy_object_settings_json_defaults_additive_object_fields() {
        let s = LodSettings::fo4_default();
        let mut v: serde_json::Value = serde_json::to_value(&s).unwrap();
        let global = v["global"].as_object_mut().unwrap();
        global.remove("southwest_cell");
        global.remove("bounds");
        let objects = v["objects"].as_object_mut().unwrap();
        for field in [
            "source",
            "atlas_mip_flooding",
            "qem_decimate_full_model_lod",
            "qem_lod4_ratio",
            "qem_lod8_ratio",
            "qem_lod16_ratio",
            "qem_lod32_ratio",
            "meshopt_decimate_object_lod",
            "meshopt_decimate_model_lod",
            "meshopt_full_model_ratios",
            "meshopt_lod_model_ratios",
            "meshopt_alpha_ratios",
            "meshopt_target_errors",
            "meshopt_sloppy_from_lod",
            "meshopt_quad_tri_budgets",
            "object_lod_top_model_count",
            "object_lod_huge_bto_warn_mb",
            "object_lod_model_cache_mb",
            "fo76_bto_include_baked",
            "fo76_bto_include_global_atlas_baked",
            "fo76_bto_include_remeshed_baked",
            "fo76_bto_include_instances",
            "fo76_bto_include_tree_instances",
            "fo76_bto_atlas_pages",
            "fo76_bto_merge_atlassed_shapes",
            "fo76_bto_atlas_min_tile_size",
            "fo76_bto_atlas_min_foliage_tile_size",
            "fo76_bto_atlas_foliage_page_size",
            "fo76_bto_atlas_foliage_max_tile_size",
            "fo76_bto_atlas_min_alpha_tested_tile_size",
            "fo76_bto_atlas_alpha_tested_page_size",
            "fo76_bto_atlas_alpha_tested_max_tile_size",
            "fo76_bto_atlas_from_lod",
            "fo76_bto_tree_billboard_from_lod",
            "fo76_bto_multibound_mode",
            "fo76_bto_node_layout",
        ] {
            objects.remove(field);
        }
        let json = serde_json::to_string(&v).unwrap();
        let back: LodSettings = serde_json::from_str(&json).expect("legacy JSON must deserialize");
        assert!(!back.objects.qem_decimate_full_model_lod);
        assert!(!back.objects.atlas_mip_flooding);
        assert_eq!(back.objects.qem_lod4_ratio, 0.35);
        assert_eq!(back.objects.qem_lod8_ratio, 0.18);
        assert_eq!(back.objects.qem_lod16_ratio, 0.08);
        assert_eq!(back.objects.qem_lod32_ratio, 0.03);
        assert!(!back.objects.meshopt_decimate_object_lod);
        assert!(!back.objects.meshopt_decimate_model_lod);
        assert_eq!(
            back.objects.meshopt_full_model_ratios,
            [0.30, 0.12, 0.025, 0.008]
        );
        assert_eq!(
            back.objects.meshopt_lod_model_ratios,
            [0.80, 0.55, 0.22, 0.08]
        );
        assert_eq!(back.objects.meshopt_alpha_ratios, [0.55, 0.30, 0.08, 0.025]);
        assert_eq!(
            back.objects.meshopt_target_errors,
            [0.005, 0.01, 0.025, 0.05]
        );
        assert_eq!(back.objects.meshopt_sloppy_from_lod, 16);
        assert_eq!(
            back.objects.meshopt_quad_tri_budgets,
            [220_000, 120_000, 50_000, 20_000]
        );
        assert_eq!(back.objects.object_lod_top_model_count, 10);
        assert_eq!(back.objects.object_lod_huge_bto_warn_mb, 64);
        assert_eq!(back.objects.object_lod_model_cache_mb, 0);
        assert!(back.objects.fo76_bto_include_baked);
        assert!(back.objects.fo76_bto_include_global_atlas_baked);
        assert!(back.objects.fo76_bto_include_remeshed_baked);
        assert!(back.objects.fo76_bto_include_instances);
        assert!(back.objects.fo76_bto_include_tree_instances);
        assert!(!back.objects.fo76_bto_atlas_pages);
        assert!(!back.objects.fo76_bto_merge_atlassed_shapes);
        assert_eq!(back.objects.fo76_bto_atlas_min_tile_size, 0);
        assert_eq!(back.objects.fo76_bto_atlas_min_foliage_tile_size, 0);
        assert_eq!(back.objects.fo76_bto_atlas_foliage_page_size, 0);
        assert_eq!(back.objects.fo76_bto_atlas_foliage_max_tile_size, 0);
        assert_eq!(back.objects.fo76_bto_atlas_min_alpha_tested_tile_size, 0);
        assert_eq!(back.objects.fo76_bto_atlas_alpha_tested_page_size, 0);
        assert_eq!(back.objects.fo76_bto_atlas_alpha_tested_max_tile_size, 0);
        assert_eq!(back.objects.fo76_bto_atlas_from_lod, None);
        assert_eq!(back.objects.fo76_bto_tree_billboard_from_lod, None);
        assert_eq!(
            back.objects.fo76_bto_multibound_mode,
            Fo76BtoMultiboundMode::Shape
        );
        assert_eq!(
            back.objects.fo76_bto_node_layout,
            Fo76BtoNodeLayout::Fo4PerShape
        );
        assert_eq!(back, s, "legacy JSON round-trips to fo4_default()");
    }

    #[test]
    fn settings_roundtrip_json() {
        let s = LodSettings::fo4_default();
        let json = serde_json::to_string(&s).unwrap();
        let back: LodSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn object_source_accepts_fo76_bto_atlas() {
        let mut v = serde_json::to_value(LodSettings::fo4_default()).unwrap();
        let objects = v["objects"].as_object_mut().unwrap();
        objects.insert(
            "source".to_string(),
            serde_json::Value::String("fo76_bto_atlas".to_string()),
        );
        objects.insert(
            "fo76_bto_atlas_pages".to_string(),
            serde_json::Value::Bool(true),
        );
        objects.insert(
            "fo76_bto_merge_atlassed_shapes".to_string(),
            serde_json::Value::Bool(true),
        );
        objects.insert(
            "fo76_bto_tree_billboard_from_lod".to_string(),
            serde_json::Value::Number(16.into()),
        );
        objects.insert(
            "fo76_bto_atlas_min_tile_size".to_string(),
            serde_json::Value::Number(512.into()),
        );
        objects.insert(
            "fo76_bto_atlas_min_foliage_tile_size".to_string(),
            serde_json::Value::Number(1024.into()),
        );
        objects.insert(
            "fo76_bto_atlas_foliage_page_size".to_string(),
            serde_json::Value::Number(8192.into()),
        );
        objects.insert(
            "fo76_bto_atlas_foliage_max_tile_size".to_string(),
            serde_json::Value::Number(1024.into()),
        );
        objects.insert(
            "fo76_bto_atlas_min_alpha_tested_tile_size".to_string(),
            serde_json::Value::Number(1024.into()),
        );
        objects.insert(
            "fo76_bto_atlas_alpha_tested_page_size".to_string(),
            serde_json::Value::Number(4096.into()),
        );
        objects.insert(
            "fo76_bto_atlas_alpha_tested_max_tile_size".to_string(),
            serde_json::Value::Number(1024.into()),
        );
        objects.insert(
            "fo76_bto_atlas_from_lod".to_string(),
            serde_json::Value::Number(8.into()),
        );

        let parsed: LodSettings = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.objects.source, ObjectSource::Fo76BtoAtlas);
        assert!(parsed.objects.fo76_bto_atlas_pages);
        assert!(parsed.objects.fo76_bto_merge_atlassed_shapes);
        assert_eq!(parsed.objects.fo76_bto_atlas_min_tile_size, 512);
        assert_eq!(parsed.objects.fo76_bto_atlas_min_foliage_tile_size, 1024);
        assert_eq!(parsed.objects.fo76_bto_atlas_foliage_page_size, 8192);
        assert_eq!(parsed.objects.fo76_bto_atlas_foliage_max_tile_size, 1024);
        assert_eq!(
            parsed.objects.fo76_bto_atlas_min_alpha_tested_tile_size,
            1024
        );
        assert_eq!(parsed.objects.fo76_bto_atlas_alpha_tested_page_size, 4096);
        assert_eq!(
            parsed.objects.fo76_bto_atlas_alpha_tested_max_tile_size,
            1024
        );
        assert_eq!(parsed.objects.fo76_bto_atlas_from_lod, Some(8));
        assert_eq!(parsed.objects.fo76_bto_tree_billboard_from_lod, Some(16));
    }

    /// `default_settings_json()` must deserialize back to exactly `fo4_default()`.
    #[test]
    fn default_settings_json_round_trips() {
        let json = super::default_settings_json();
        let parsed: LodSettings =
            serde_json::from_str(&json).expect("default_settings_json must produce valid JSON");
        assert_eq!(
            parsed,
            LodSettings::fo4_default(),
            "default_settings_json() round-trips to fo4_default()"
        );
    }
}
