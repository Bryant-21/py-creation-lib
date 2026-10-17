use serde::Deserialize;
#[derive(Debug, Clone, Deserialize)]
pub struct TextureManifest {
    pub textures: Vec<TextureBundle>,
    /// FO76 sources only: every GCVR in the BTD's table, as the runtime loads it. Feeds the
    /// `.btd4` v5 grass-point channel; never emitted as records.
    #[serde(default)]
    pub ground_covers: Vec<GroundCoverManifest>,
}

/// One FO76 GCVR as `BGSGroundCover::Load` keeps it.
#[derive(Debug, Clone, Deserialize)]
pub struct GroundCoverManifest {
    pub form_key: String,
    #[serde(default)]
    pub editor_id: String,
    #[serde(default = "default_ground_cover_ynam")]
    pub ynam: f32,
    #[serde(default)]
    pub ltex_form_keys: Vec<String>,
    /// GNAMs followed by a DNAM, in file order; first occurrence of a repeated GNAM only.
    #[serde(default)]
    pub grasses: Vec<GroundCoverGrassManifest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroundCoverGrassManifest {
    pub form_key: String,
    /// Density percent, or 0xFFFF for the GRAS DNAM density byte.
    pub dnam: u16,
}

fn default_ground_cover_ynam() -> f32 {
    crate::fo76_grass::GCVR_DEFAULT_YNAM
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct TextureBundle {
    pub source_ltex_form_key: String,
    pub source_ltex_editor_id: String,
    #[serde(default)]
    pub source_gcvr_form_key: Option<String>,
    #[serde(default)]
    pub source_gcvr_editor_id: Option<String>,
    pub source_txst_form_key: String,
    pub source_txst_editor_id: String,
    pub diffuse_path: String,
    pub normal_path: String,
    pub reflectivity_path: String,
    pub lighting_path: String,
    pub output_prefix: String,
    #[serde(default)]
    pub raw_source_prefix: Option<String>,
    #[serde(default)]
    pub raw_material_rows: Option<[[f32; 4]; 3]>,
    // Present only for the .btd4 raw source bank: a texture to deliver, never a record.
    #[serde(default)]
    pub raw_bank_only: bool,
    pub output_material_path: Option<String>,
    pub material_type_object_id: Option<String>,
    pub havok_friction: Option<u8>,
    pub havok_restitution: Option<u8>,
    #[serde(default)]
    pub grass: Vec<ConvertedTerrainGrass>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ConvertedGrassObjectBounds {
    #[serde(default, rename = "ObjectBoundsX1")]
    pub x1: i16,
    #[serde(default, rename = "ObjectBoundsY1")]
    pub y1: i16,
    #[serde(default, rename = "ObjectBoundsZ1")]
    pub z1: i16,
    #[serde(default, rename = "ObjectBoundsX2")]
    pub x2: i16,
    #[serde(default, rename = "ObjectBoundsY2")]
    pub y2: i16,
    #[serde(default, rename = "ObjectBoundsZ2")]
    pub z2: i16,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ConvertedTerrainGrass {
    pub source_form_key: String,
    pub source_editor_id: String,
    #[serde(default)]
    pub object_bounds: ConvertedGrassObjectBounds,
    pub model_file_name: String,
    pub model_information: String,
    pub density: u8,
    pub max_slope: u8,
    pub position_range: f32,
    pub height_range: f32,
    pub color_range: f32,
    pub wave_period: f32,
    #[serde(default)]
    pub flags: Vec<String>,
    /// Source GCVR records that place this grass (BACUP manifest); keys the `.btd4` GCVR channel.
    #[serde(default)]
    pub source_gcvr_form_keys: Vec<String>,
    /// Target-game record this grass resolves to (`0BA520:Fallout4.esm`). BACUP's terrain
    /// import drops the emitted GRAS in favour of it, so `.btd4` references must use it.
    #[serde(default)]
    pub target_form_key: Option<String>,
    /// FO76 sources only: the raw GRAS DNAM, which places the `.btd4` v5 grass points.
    #[serde(default)]
    pub fo76_dnam: Option<crate::fo76_grass::GrassDnam>,
}

#[derive(Debug, Clone)]
pub struct ConvertedTerrainTexture {
    pub source_ltex_form_key: String,
    pub source_ltex_editor_id: String,
    pub source_gcvr_form_key: Option<String>,
    pub source_gcvr_editor_id: Option<String>,
    pub source_txst_form_key: String,
    pub source_txst_editor_id: String,
    pub suffix: String,
    pub diffuse_rel_path: String,
    pub normal_rel_path: String,
    pub specgloss_rel_path: String,
    pub glow_rel_path: String,
    pub material_type_object_id: Option<String>,
    pub havok_friction: u8,
    pub havok_restitution: u8,
    pub grass: Vec<ConvertedTerrainGrass>,
}

pub fn plan_required_textures(
    manifest: &TextureManifest,
) -> Result<Vec<ConvertedTerrainTexture>, String> {
    manifest
        .textures
        .iter()
        .filter(|bundle| !bundle.raw_bank_only)
        .map(plan_texture_bundle)
        .collect()
}

fn plan_texture_bundle(bundle: &TextureBundle) -> Result<ConvertedTerrainTexture, String> {
    let prefix = normalize_output_prefix(&bundle.output_prefix)?;
    Ok(ConvertedTerrainTexture {
        source_ltex_form_key: bundle.source_ltex_form_key.clone(),
        source_ltex_editor_id: bundle.source_ltex_editor_id.clone(),
        source_gcvr_form_key: bundle.source_gcvr_form_key.clone(),
        source_gcvr_editor_id: bundle.source_gcvr_editor_id.clone(),
        source_txst_form_key: bundle.source_txst_form_key.clone(),
        source_txst_editor_id: bundle.source_txst_editor_id.clone(),
        suffix: safe_suffix(&bundle.output_prefix),
        diffuse_rel_path: format!("{prefix}_d.dds"),
        normal_rel_path: format!("{prefix}_n.dds"),
        specgloss_rel_path: format!("{prefix}_s.dds"),
        glow_rel_path: format!("{prefix}_g.dds"),
        material_type_object_id: normalize_optional_object_id(
            bundle.material_type_object_id.as_deref(),
        )?,
        havok_friction: bundle.havok_friction.unwrap_or(30),
        havok_restitution: bundle.havok_restitution.unwrap_or(30),
        grass: bundle.grass.clone(),
    })
}

fn normalize_optional_object_id(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let without_prefix = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    let parsed = u32::from_str_radix(without_prefix, 16)
        .map_err(|_| format!("invalid material type object id: {value}"))?;
    Ok(Some(format!("{:06X}", parsed & 0x00FF_FFFF)))
}

fn normalize_output_prefix(value: &str) -> Result<String, String> {
    let normalized = value.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.contains("..") {
        return Err(format!("invalid texture output_prefix: {value}"));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_terrain_textures_does_not_require_or_write_dds_files() {
        let manifest = TextureManifest {
            textures: vec![TextureBundle {
                source_ltex_form_key: "SeventySix.esm:00ABCD".to_owned(),
                source_ltex_editor_id: "LTest".to_owned(),
                source_gcvr_form_key: None,
                source_gcvr_editor_id: None,
                source_txst_form_key: "SeventySix.esm:00DCBA".to_owned(),
                source_txst_editor_id: "TestTXST".to_owned(),
                diffuse_path: "missing/test_d.dds".to_owned(),
                normal_path: "missing/test_n.dds".to_owned(),
                reflectivity_path: "missing/test_r.dds".to_owned(),
                lighting_path: "missing/test_l.dds".to_owned(),
                output_prefix: "textures/terrain/appalachia/LTest".to_owned(),
                raw_source_prefix: None,
                raw_material_rows: None,
                raw_bank_only: false,
                output_material_path: None,
                material_type_object_id: None,
                havok_friction: None,
                havok_restitution: None,
                grass: Vec::new(),
            }],
            ground_covers: Vec::new(),
        };

        let planned = plan_required_textures(&manifest).unwrap();

        assert_eq!(planned.len(), 1);
        assert_eq!(
            planned[0].diffuse_rel_path,
            "textures/terrain/appalachia/LTest_d.dds"
        );
        assert_eq!(
            planned[0].specgloss_rel_path,
            "textures/terrain/appalachia/LTest_s.dds"
        );
    }
}

fn safe_suffix(value: &str) -> String {
    let stem = value
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or("Texture")
        .trim()
        .to_string();
    stem.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
