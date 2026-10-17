use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::classify::MaterialClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MetalMode {
    /// Every upgraded material is dielectric (F0 0.04). Best measured default.
    #[default]
    Off,
    /// Whole material becomes grey metal when its FO4 envmap names a metal cube.
    Envmap,
    /// Per-texel mask from `_s.R` on metal-class materials (low precision).
    Mask,
    /// Per-texel mask read from `<metal_mask_dir>/<diffuse stem>_metal.dds` (R channel).
    UserMask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpgradeOptions {
    /// Data folders, mod folders or `.ba2` files. Later sources override earlier ones;
    /// inside a folder, loose files override its archives.
    pub sources: Vec<PathBuf>,
    /// Read before `sources` and overridden by them, never upgraded: the game Data a mod's
    /// materials and meshes reference textures from.
    pub base_sources: Vec<PathBuf>,
    /// Read after `sources` and override them, but never upgraded themselves: an asset
    /// whose winning copy is here (a converted mod's own materials) is left as authored.
    pub overlay_sources: Vec<PathBuf>,
    /// Overlay materials upgraded all the same: FO4-era leftovers the converter kept legacy.
    pub overlay_upgrade_paths: Vec<String>,
    /// Root of the separate output mod (receives `materials/`, `textures/`, `meshes/`).
    pub output: PathBuf,
    pub classes: Vec<MaterialClass>,
    pub metal_mode: MetalMode,
    pub metal_mask_dir: Option<PathBuf>,
    pub smoothness_gain: f32,
    /// Highlight knee: smoothness above it is compressed to 30 % slope.
    pub knee: Option<f32>,
    pub include_materials: bool,
    /// NIF-embedded `BSShaderTextureSet`s without a `.bgsm` (slots 8/9 are filled).
    pub include_nifs: bool,
    /// Landscape layer bank (`*_source_{d,n,l,r}.dds`) from LTEX/TXST in `plugins`.
    pub include_terrain: bool,
    pub plugins: Vec<PathBuf>,
    pub terrain_layer_size: u32,
    /// Write `terrain/fo4cs/<worldspace>.btd4` for the staged bank (FO4CS native terrain).
    pub terrain_sidecars: bool,
    /// Worldspace editor ids to write sidecars for; empty = every worldspace with LAND.
    pub terrain_worldspaces: Vec<String>,
    /// Lowercase data-relative paths that must not be upgraded (e.g. assets being
    /// replaced by native versions). Applied after planning.
    pub exclude_paths: Vec<String>,
    /// Optional lowercase prefixes (`materials/weapons/`) restricting the scan.
    pub include_prefixes: Vec<String>,
    pub workers: usize,
}

impl Default for UpgradeOptions {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            base_sources: Vec::new(),
            overlay_sources: Vec::new(),
            overlay_upgrade_paths: Vec::new(),
            output: PathBuf::new(),
            classes: MaterialClass::DEFAULT_ENABLED.to_vec(),
            metal_mode: MetalMode::Off,
            metal_mask_dir: None,
            smoothness_gain: 1.0,
            knee: None,
            include_materials: true,
            include_nifs: false,
            include_terrain: false,
            plugins: Vec::new(),
            terrain_layer_size: 1024,
            terrain_sidecars: true,
            terrain_worldspaces: Vec::new(),
            exclude_paths: Vec::new(),
            include_prefixes: Vec::new(),
            workers: 4,
        }
    }
}

impl UpgradeOptions {
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|err| format!("invalid upgrade options: {err}"))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.sources.is_empty() {
            return Err("at least one source (Data folder, mod folder or .ba2) is required".into());
        }
        if !(0.0..=4.0).contains(&self.smoothness_gain) {
            return Err(format!("smoothness gain {} outside 0..4", self.smoothness_gain));
        }
        if let Some(knee) = self.knee {
            if !(0.05..=1.0).contains(&knee) {
                return Err(format!("knee {knee} outside 0.05..1"));
            }
        }
        if self.metal_mode == MetalMode::UserMask && self.metal_mask_dir.is_none() {
            return Err("metal mode user-mask needs metal_mask_dir".into());
        }
        if self.include_terrain && self.plugins.is_empty() {
            return Err("terrain upgrade needs at least one plugin (LTEX/TXST source)".into());
        }
        if !self.terrain_layer_size.is_power_of_two() || self.terrain_layer_size < 64 {
            return Err("terrain_layer_size must be a power of two >= 64".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe() {
        let opts = UpgradeOptions::default();
        assert_eq!(opts.metal_mode, MetalMode::Off);
        assert_eq!(opts.smoothness_gain, 1.0);
        assert!(opts.knee.is_none());
        assert!(!opts.classes.contains(&MaterialClass::Skin));
        assert!(!opts.classes.contains(&MaterialClass::Grass));
        assert!(opts.include_materials && !opts.include_nifs && !opts.include_terrain);
    }

    #[test]
    fn json_round_trip_with_partial_fields() {
        let opts = UpgradeOptions::from_json(
            r#"{"sources":["D:/Data"],"metal_mode":"envmap","classes":["metal","wood"],"knee":0.5}"#,
        )
        .unwrap();
        assert_eq!(opts.metal_mode, MetalMode::Envmap);
        assert_eq!(opts.classes, vec![MaterialClass::Metal, MaterialClass::Wood]);
        assert_eq!(opts.knee, Some(0.5));
        assert_eq!(opts.workers, 4);
        opts.validate().unwrap();
    }

    #[test]
    fn validation_rejects_bad_input() {
        assert!(UpgradeOptions::default().validate().is_err());
        let mut opts = UpgradeOptions { sources: vec!["x".into()], ..Default::default() };
        opts.metal_mode = MetalMode::UserMask;
        assert!(opts.validate().is_err());
        opts.metal_mode = MetalMode::Off;
        opts.include_terrain = true;
        assert!(opts.validate().is_err());
    }
}
