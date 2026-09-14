/// Shape descriptor types and atlas texture-key helpers.
///
/// Port sources:
/// - `ShapeFlags`       → ShapeFlags.cs
/// - `atlas_get_key`    → AtlasList.cs:13-62
/// - `atlas_build_key`  → AtlasList.cs:64-79
use crate::atlas::atlas::{AtlasList, strip_normalize_texture_path};
use crate::descriptors::BBox;
use crate::objects::geometry::LodGeometry;
use crate::objects::object_lod::SegmentDesc;

// ---------------------------------------------------------------------------
// ShaderKind — which shader property drove the texture extraction
// ---------------------------------------------------------------------------

/// Which shader property type was found on the geometry.
/// Port: `ShapeDesc.ShaderType` discriminant (ShapeDesc.cs:466/725/None).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderKind {
    /// BSLightingShaderProperty.
    Lighting,
    /// BSEffectShaderProperty (always passthru on FO4).
    Effect,
    /// No recognised shader property.
    None,
}

// ---------------------------------------------------------------------------
// ShapeDesc — one geometry block extracted from a ref's LOD NIF
// ---------------------------------------------------------------------------

/// One shape (geometry + shader + textures + flags) extracted from a LOD NIF.
/// Port: `ShapeDesc` ctor (ShapeDesc.cs:235-1313), FO4 object path only.
#[derive(Clone)]
pub struct ShapeDesc {
    /// Shape (block) name, with `\n`/`\r` stripped and trimmed.
    pub name: String,
    /// The LOD model path for this quad index, lowercased.
    pub static_model: String,
    /// Extracted, deduped LOD geometry.
    pub geometry: LodGeometry,
    /// Per-shape rendering flags.
    pub flags: ShapeFlags,
    /// 10 texture slots (lowercased, `Data\`-stripped, `.dds`-validated).
    pub textures: [String; 10],
    /// Shader material paths seen before REFR material-swap resolution.
    pub source_materials: Vec<String>,
    /// Atlas texture key (filled by transform_shape; empty here).
    pub textures_key: String,
    /// Texture clamp mode (0=CLAMP/CLAMP, 1, 2, 3=WRAP/WRAP).
    pub texture_clamp_mode: u32,
    /// Alpha test threshold.
    pub alpha_threshold: u8,
    /// NiAlphaProperty flags (or BGSM-derived).
    pub alpha_flags: u16,
    /// Backlight power (BSLightingShaderProperty / BGSM).
    pub backlight_power: f32,
    /// Grayscale-to-palette scale.
    pub grayscale_to_palette_scale: f32,
    /// Enable-parent ref id (0 = none).
    pub enable_parent: u32,
    /// Which shader property drove extraction.
    pub shader_type: ShaderKind,
    /// Shape X (quad-space; set by transform_shape — 0 here).
    pub x: f32,
    /// Shape Y (quad-space; set by transform_shape — 0 here).
    pub y: f32,
    /// Bounding box (world; grown by transform_shape — empty here).
    pub bounding_box: BBox,
    /// Segments (filled by generate_segments downstream — empty here).
    pub segments: Vec<SegmentDesc>,
    /// UV scale from the shader (default 1,1).
    pub uv_scale: [f32; 2],
    /// UV offset from the shader (default 0,0).
    pub uv_offset: [f32; 2],
    /// Original ref flags (carried for billboard/grass handling).
    pub ref_flags: u32,
    /// Accumulated NiNode parent transform (4x4, row-major) for this geom.
    /// Additive field: `transform_shape` applies
    /// it before the ref world transform. Port: IterateNodes parentTransform
    /// (LODApp.cs:308-316), consumed by TransformShape (LODApp.cs:586).
    pub node_transform: [[f32; 4]; 4],
    /// Accumulated NiNode parent scale (additive field; see `node_transform`).
    pub node_scale: f32,
    /// Per-ref translation derived from the stat (ShapeDesc.cs:358).
    pub translation: [f32; 3],
    /// Per-ref rotation matrix derived from the stat (ShapeDesc.cs:361-367).
    pub rotation: [[f32; 3]; 3],
    /// Optional final BTO transform for source BTO baked shapes.
    pub bto_translation: Option<[f32; 3]>,
    pub bto_scale: Option<f32>,
}

// ---------------------------------------------------------------------------
// ShapeFlags — port: ShapeFlags.cs plus conversion-only render state
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    /// Per-shape rendering/behaviour flags — port: ShapeFlags.cs.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ShapeFlags: u32 {
        const IS_PASSTHRU             = 0x00001;
        const IS_GROUP                = 0x00002;
        const IS_HIGH_DETAIL          = 0x00004;
        const IS_GRASS                = 0x00008;
        const HAS_VERTEX_COLOR        = 0x00010;
        const ALL_WHITE               = 0x00020;
        const IS_DOUBLE_SIDED         = 0x00040;
        const IS_ALPHA                = 0x00080;
        const IS_DECAL                = 0x00100;
        const HAS_LOD_FLAG            = 0x00200;
        const IS_GREYSCALE_TO_PALETTE = 0x00400;
        const IS_GREYSCALE_TO_ALPHA   = 0x00800;
        const IS_TREE                 = 0x01000;
        const IS_FLAT_TRUNK           = 0x02000;
        const IS_TRUNK                = 0x04000;
        const IS_CROWN                = 0x08000;
        const IS_BILLBOARD            = 0x10000;
        const HAS_VERTEX_ALPHA        = 0x20000;
        const CASTS_SHADOWS           = 0x40000;
    }
}

// ---------------------------------------------------------------------------
// atlas_get_key — port: AtlasList.cs:13-62
// ---------------------------------------------------------------------------

/// Get the atlas key for a shape's textures.
///
/// Lookup chain (descending specificity):
/// 1. `diffuse,normal,glow=at` (if glow non-empty)
/// 2. `diffuse,normal,glow`
/// 3. `diffuse,normal=at`
/// 4. `diffuse,normal`
/// 5. `diffuse=at`
/// 6. `diffuse,derived_normal=at`  (where derived_normal = replace .dds→_n.dds)
/// 7. `diffuse,derived_normal`
/// 8. fall back to raw `diffuse`
///
/// PBR normalization: `textures\pbr\..._linear.dds` → `textures\pbr\...dds`
/// (AtlasList.cs:15-18).
pub fn atlas_get_key(
    list: &AtlasList,
    diffuse: &str,
    normal: &str,
    glow: &str,
    alpha_threshold: u8,
) -> String {
    // Normalise `Data\`-prefixed paths (e.g. `Data\LOD\foo_d.dds` → `Textures\LOD\foo_d.dds`)
    // so that shapes whose NIF stored the diffuse with a bare `Data\LOD\...` prefix
    // (yielding `lod\foo_d.dds` after parse_nif's strip_data_prefix) find the same
    // atlas entry as the canonical `textures\lod\foo_d.dds` key.
    let diffuse_norm = strip_normalize_texture_path(diffuse);
    let normal_norm = if normal.is_empty() {
        normal.to_string()
    } else {
        strip_normalize_texture_path(normal)
    };
    let diffuse_ref: &str = &diffuse_norm;
    let normal_ref: &str = &normal_norm;

    // port: AtlasList.cs:15-18 — PBR linear normalisation
    let diffuse = normalize_pbr_linear(diffuse_ref);

    // floor(alphaThreshold / 16) as a string (AtlasList.cs:19)
    let at_str = ((alpha_threshold as f32 / 16.0).floor() as u32).to_string();

    // -- glow branch (AtlasList.cs:20-32) --
    if !glow.is_empty() {
        let base3 = format!("{},{},{}", diffuse, normal_ref, glow);
        let with_at = format!("{}={}", base3, at_str);
        if list.contains(&with_at) {
            return with_at;
        }
        if list.contains(&base3) {
            return base3;
        }
    }

    // -- normal branch (AtlasList.cs:33-45) --
    if !normal_ref.is_empty() {
        let base2 = format!("{},{}", diffuse, normal_ref);
        let with_at = format!("{}={}", base2, at_str);
        if list.contains(&with_at) {
            return with_at;
        }
        if list.contains(&base2) {
            return base2;
        }
    }

    // -- diffuse=at (AtlasList.cs:46-50) --
    {
        let with_at = format!("{}={}", diffuse, at_str);
        if list.contains(&with_at) {
            return with_at;
        }
    }

    // -- derived normal fallback (AtlasList.cs:51-61) --
    {
        let derived_n = get_normal_texture_name(&diffuse);
        let base_dn = format!("{},{}", diffuse, derived_n);
        let with_at = format!("{}={}", base_dn, at_str);
        if list.contains(&with_at) {
            return with_at;
        }
        if list.contains(&base_dn) {
            return base_dn;
        }
    }

    // -- fallback: raw diffuse (AtlasList.cs:61) --
    diffuse.into_owned()
}

/// Build an atlas key from a texture list and alpha threshold.
/// Port: AtlasList.cs:64-79.
pub fn atlas_build_key(list: &AtlasList, textures: &[String], alpha_threshold: u8) -> String {
    match textures.len() {
        0 => String::new(),
        1 => atlas_get_key(list, &textures[0], "", "", alpha_threshold),
        2 => atlas_get_key(list, &textures[0], &textures[1], "", alpha_threshold),
        _ => atlas_get_key(
            list,
            &textures[0],
            &textures[1],
            &textures[2],
            alpha_threshold,
        ),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Replace `_linear.dds` with `.dds` in a PBR texture path (case-insensitive).
/// Port: AtlasList.cs:15-18.
fn normalize_pbr_linear(diffuse: &str) -> std::borrow::Cow<'_, str> {
    let lower = diffuse.to_lowercase();
    if lower.contains("linear.dds") && lower.contains(r"textures\pbr\") {
        std::borrow::Cow::Owned(
            // case-insensitive replace: find the offset in the original
            case_insensitive_replace(diffuse, "linear.dds", ".dds"),
        )
    } else {
        std::borrow::Cow::Borrowed(diffuse)
    }
}

/// Case-insensitive string replacement (replaces the FIRST occurrence).
fn case_insensitive_replace(s: &str, from: &str, to: &str) -> String {
    let lower_s = s.to_lowercase();
    let lower_from = from.to_lowercase();
    if let Some(pos) = lower_s.find(&lower_from) {
        format!("{}{}{}", &s[..pos], to, &s[pos + from.len()..])
    } else {
        s.to_string()
    }
}

/// Derive the normal texture name from a diffuse name.
/// Port: Utils.cs:348-351 — replace `.dds` with `_n.dds` (case-insensitive, last occurrence).
pub fn get_normal_texture_name(diffuse: &str) -> String {
    let lower = diffuse.to_lowercase();
    if let Some(pos) = lower.rfind(".dds") {
        format!("{}_n.dds", &diffuse[..pos])
    } else {
        format!("{}_n.dds", diffuse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_pbr_linear_replaces() {
        // AtlasList.cs:17 CaseInsensitiveReplace("linear.dds", ".dds"):
        // replaces the substring "linear.dds" with ".dds", leaving "_" prefix intact.
        let result = normalize_pbr_linear(r"textures\pbr\x_linear.dds");
        assert_eq!(result, r"textures\pbr\x_.dds");
    }

    #[test]
    fn get_normal_texture_name_replaces_suffix() {
        assert_eq!(get_normal_texture_name("a_d.dds"), "a_d_n.dds");
        // standard: replaces .dds → _n.dds  (Utils.cs replaces all ".dds" → "_n.dds")
        let n = get_normal_texture_name(r"textures\lod\airport01_lod_d.dds");
        assert_eq!(n, r"textures\lod\airport01_lod_d_n.dds");
    }
}
