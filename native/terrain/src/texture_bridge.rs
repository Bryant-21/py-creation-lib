use serde::Deserialize;
#[derive(Debug, Clone, Deserialize)]
pub struct TextureManifest {
    pub textures: Vec<TextureBundle>,
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
    manifest.textures.iter().map(plan_texture_bundle).collect()
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
                output_material_path: None,
                material_type_object_id: None,
                havok_friction: None,
                havok_restitution: None,
                grass: Vec::new(),
            }],
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
