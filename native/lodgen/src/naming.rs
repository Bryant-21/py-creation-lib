/// Data-relative path for a `.btr` terrain mesh block.
/// Pattern: `Meshes\Terrain\{world}\{world}.{level}.{x}.{y}.btr`
/// (TerrainLOD.cs:1364)
pub fn btr(world: &str, level: i32, x: i32, y: i32) -> String {
    format!(r"Meshes\Terrain\{world}\{world}.{level}.{x}.{y}.btr")
}

/// Data-relative path for a `.bto` object LOD mesh block.
/// Pattern: `Meshes\Terrain\{world}\Objects\{world}.{level}.{x}.{y}{season}.bto`
/// (TerrainLOD.cs:1374)
pub fn bto(world: &str, level: i32, x: i32, y: i32, season: &str) -> String {
    format!(r"Meshes\Terrain\{world}\Objects\{world}.{level}.{x}.{y}{season}.bto")
}

/// Data-relative path for a terrain diffuse DDS tile.
/// Pattern: `Textures\Terrain\{world}\{world}.{level}.{x}.{y}{season}.dds`
/// Season is inserted before `.dds` (R3 §5).
pub fn terrain_diffuse(world: &str, level: i32, x: i32, y: i32, season: &str) -> String {
    format!(r"Textures\Terrain\{world}\{world}.{level}.{x}.{y}{season}.dds")
}

/// Data-relative path for a terrain micro-surface normal (_msn) DDS tile.
/// Pattern: `Textures\Terrain\{world}\{world}.{level}.{x}.{y}{season}_msn.dds`
/// (TerrainLOD.cs:1681-1684, R3 §5)
pub fn terrain_msn(world: &str, level: i32, x: i32, y: i32, season: &str) -> String {
    format!(r"Textures\Terrain\{world}\{world}.{level}.{x}.{y}{season}_msn.dds")
}

/// Data-relative path for the `.lod` LODSettings file.
/// Pattern: `LODSettings\{world}.lod` (wbLOD.pas:432-459)
pub fn lodsettings(world: &str) -> String {
    format!(r"LODSettings\{world}.lod")
}

/// Data-relative path for the object LOD texture atlas.
/// Pattern: `Textures\Terrain\{world}\Objects\{world}.Objects.dds` (DOT separator).
/// Verified against the golden corpus + the `.bto` texture-set slot strings, which
/// reference `data\textures\terrain\<world>\objects\<world>.objects.dds`
/// (R3 §2c / §5; golden DLC03FarHarbor.16.-9.5.bto block 4).
pub fn object_atlas(world: &str) -> String {
    format!(r"Textures\Terrain\{world}\Objects\{world}.Objects.dds")
}

/// Data-relative path for a `.btt` tree LOD block.
/// Pattern: `Meshes\Terrain\{world}\Trees\{world}.{level}.{x}.{y}.btt`
///
/// FO4 reuses the Skyrim `meshes\terrain\<W>\trees\` layout (wbLOD.pas:882).
/// NOTE: the `.btt` format is used only in billboard-mode (`trees_3d=false`).
/// FO4 default 3D-tree LOD folds trees into `.bto`.
///
/// port: TwbLodTES5TreeBlock.GetBlockFileName (wbLOD.pas:877-895)
pub fn btt(world: &str, level: i32, x: i32, y: i32) -> String {
    format!(r"Meshes\Terrain\{world}\Trees\{world}.{level}.{x}.{y}.btt")
}

/// Data-relative path for the tree LOD list (`.lst`) file.
/// Pattern: `Meshes\Terrain\{world}\Trees\{world}.lst`
///
/// port: TwbLodTES5TreeList.GetListFileName (wbLOD.pas:596-616, Skyrim branch)
pub fn tree_list(world: &str) -> String {
    format!(r"Meshes\Terrain\{world}\Trees\{world}.lst")
}

/// Data-relative path for the billboard atlas DDS.
/// Pattern: `Textures\Terrain\LODGen\{world}\{world}TreeLod.dds`
///
/// port: TwbLodTES5TreeList.GetAtlasFileName (wbLOD.pas:605-616, Skyrim branch)
pub fn billboard_atlas(world: &str) -> String {
    format!(r"Textures\Terrain\LODGen\{world}\{world}TreeLod.dds")
}

/// File name for the billboard manifest JSON written by the Python generator.
/// Pattern: `{world}_billboard_manifest.json`
///
/// Placed in `ctx.paths.output_dir` by `creation_lib.lod.billboards.write_manifest`;
/// the Rust billboard-placement path (`trees::generate_quad`) reads it from there
/// (output_dir first, then data_dirs).
pub fn billboard_manifest(world: &str) -> String {
    format!("{world}_billboard_manifest.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming_matches_corpus() {
        assert_eq!(
            btr("DLC03FarHarbor", 16, -25, -11),
            r"Meshes\Terrain\DLC03FarHarbor\DLC03FarHarbor.16.-25.-11.btr"
        );
        assert_eq!(
            terrain_diffuse("DLC03FarHarbor", 16, -25, -11, ""),
            r"Textures\Terrain\DLC03FarHarbor\DLC03FarHarbor.16.-25.-11.dds"
        );
        assert_eq!(
            terrain_msn("DLC03FarHarbor", 16, -25, -11, ""),
            r"Textures\Terrain\DLC03FarHarbor\DLC03FarHarbor.16.-25.-11_msn.dds"
        );
        // season inserted BEFORE extension (R3 §5)
        assert_eq!(
            terrain_diffuse("Commonwealth", 4, 0, 0, ".Winter"),
            r"Textures\Terrain\Commonwealth\Commonwealth.4.0.0.Winter.dds"
        );
        assert_eq!(lodsettings("Commonwealth"), r"LODSettings\Commonwealth.lod");
        // DOT separator — matches the golden `.bto` texture-set slot strings
        // (data\textures\terrain\<world>\objects\<world>.objects.dds) and R3 §2c/§5.
        assert_eq!(
            object_atlas("Commonwealth"),
            r"Textures\Terrain\Commonwealth\Objects\Commonwealth.Objects.dds"
        );
        // Golden corpus worldspace (lowercased on-disk: dlc03farharbor.objects.dds).
        assert_eq!(
            object_atlas("DLC03FarHarbor"),
            r"Textures\Terrain\DLC03FarHarbor\Objects\DLC03FarHarbor.Objects.dds"
        );
        assert!(
            object_atlas("DLC03FarHarbor")
                .to_lowercase()
                .ends_with(r"objects\dlc03farharbor.objects.dds")
        );
    }
}
