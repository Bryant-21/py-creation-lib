use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum MaterialClass {
    Metal,
    Painted,
    Wood,
    Fabric,
    Stone,
    Organic,
    Other,
    /// Skin, hair, face: FO4 skin uses SubsurfaceLighting that BGSM v22+ maps poorly.
    Skin,
    /// Landscape grass: rendered without a PBR path in FO4CS today.
    Grass,
}

impl MaterialClass {
    pub const ALL: [MaterialClass; 9] = [
        MaterialClass::Metal,
        MaterialClass::Painted,
        MaterialClass::Wood,
        MaterialClass::Fabric,
        MaterialClass::Stone,
        MaterialClass::Organic,
        MaterialClass::Other,
        MaterialClass::Skin,
        MaterialClass::Grass,
    ];
    pub const DEFAULT_ENABLED: [MaterialClass; 7] = [
        MaterialClass::Metal,
        MaterialClass::Painted,
        MaterialClass::Wood,
        MaterialClass::Fabric,
        MaterialClass::Stone,
        MaterialClass::Organic,
        MaterialClass::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MaterialClass::Metal => "metal",
            MaterialClass::Painted => "painted",
            MaterialClass::Wood => "wood",
            MaterialClass::Fabric => "fabric",
            MaterialClass::Stone => "stone",
            MaterialClass::Organic => "organic",
            MaterialClass::Other => "other",
            MaterialClass::Skin => "skin",
            MaterialClass::Grass => "grass",
        }
    }

    /// Smoothness for a flat or missing `_s`: per-class medians of the reference
    /// upgrade of shared FO4 assets.
    pub fn default_smoothness(self) -> f32 {
        match self {
            MaterialClass::Metal => 0.46,
            MaterialClass::Painted => 0.40,
            MaterialClass::Wood => 0.25,
            MaterialClass::Fabric => 0.19,
            MaterialClass::Stone => 0.21,
            MaterialClass::Organic | MaterialClass::Grass => 0.23,
            MaterialClass::Other => 0.34,
            MaterialClass::Skin => 0.30,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClassHints<'a> {
    pub diffuse: &'a str,
    pub root_material: &'a str,
    pub envmap: &'a str,
    pub material_path: &'a str,
    pub skin_tint: bool,
    pub facegen: bool,
    pub hair: bool,
    pub tree: bool,
}

const ORGANIC: &[&str] = &["leaf", "plants", "tree", "food", "meat", "steak", "foliage", "bush", "moss"];
const FABRIC: &[&str] = &["cloth", "outfit", "clothes", "rug", "doily", "sleepingbag", "tarp", "fabric", "leather", "carpet"];
const WOOD: &[&str] = &["wood", "plywood", "board", "plank", "pier"];
const STONE: &[&str] = &["rock", "brick", "concrete", "dirt", "stone", "statue", "footpath", "ground", "plaque", "asphalt", "cliff", "gravel"];
const METAL_ENVMAP: &[&str] = &["bronze", "copper", "chrome", "gold", "brushed", "metal", "steel", "silver"];
const METAL: &[&str] = &["metal", "weapon", "bronze", "copper", "chrome", "gold", "brushed", "cage", "steel", "pipe"];
const PAINTED: &[&str] = &["vehicle", "armor", "vault", "sign", "panel", "console", "machine", "register", "truck", "robot", "terminal"];
const SKIN: &[&str] = &["skin", "hair", "facegen", "/head", "eyebrow", "beard", "basehuman"];

fn any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

pub fn envmap_is_metal(envmap: &str) -> bool {
    let env = envmap.to_ascii_lowercase();
    !env.is_empty() && any(&env, METAL_ENVMAP) && !env.contains("defaultoutside1.dds")
}

/// Material class from the template root, envmap, texture/material paths and flags.
/// Order matters: the earliest family that matches wins.
pub fn classify_material(hints: &ClassHints<'_>) -> MaterialClass {
    let root = hints.root_material.to_ascii_lowercase();
    let paths = format!(
        "{} {} {}",
        root,
        hints.diffuse.to_ascii_lowercase().replace('\\', "/"),
        hints.material_path.to_ascii_lowercase().replace('\\', "/")
    );
    if hints.skin_tint || hints.facegen || hints.hair || any(&paths, SKIN) {
        return MaterialClass::Skin;
    }
    if paths.contains("landscape/grass") || root.contains("grass") {
        return MaterialClass::Grass;
    }
    if hints.tree || any(&paths, ORGANIC) {
        return MaterialClass::Organic;
    }
    // A metal cube is stronger evidence than a path word ("clipboard" holds "board").
    if envmap_is_metal(hints.envmap) {
        return MaterialClass::Metal;
    }
    if any(&paths, FABRIC) {
        return MaterialClass::Fabric;
    }
    if any(&paths, WOOD) {
        return MaterialClass::Wood;
    }
    if any(&paths, STONE) {
        return MaterialClass::Stone;
    }
    if any(&paths, METAL) {
        return MaterialClass::Metal;
    }
    if any(&paths, PAINTED) {
        return MaterialClass::Painted;
    }
    MaterialClass::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(diffuse: &str, root: &str, envmap: &str) -> MaterialClass {
        classify_material(&ClassHints { diffuse, root_material: root, envmap, ..Default::default() })
    }

    #[test]
    fn classifies_by_template_paths_and_envmap() {
        assert_eq!(class("Weapons/10mmPistol/10mmPistol02_d.dds", "Template/WeaponMetalTemplate_Wet.bgsm", ""), MaterialClass::Metal);
        assert_eq!(class("Architecture/Shacks/ShackPlywood03_d.dds", "Template/WoodTemplate_Wet.bgsm", ""), MaterialClass::Wood);
        assert_eq!(class("Landscape/Plants/HubFlower01_d.dds", "Template/LeafTemplate_Wet.bgsm", ""), MaterialClass::Organic);
        assert_eq!(class("Clothes/VaultSuit/VaultSuitF_d.dds", "Template/OutfitTemplate_Wet.bgsm", ""), MaterialClass::Fabric);
        assert_eq!(class("Architecture/Buildings/Concrete01a_d.dds", "Template/BrickTemplate_Wet.BGSM", ""), MaterialClass::Stone);
        assert_eq!(class("Vehicles/Automotive/Coupe01_d.dds", "Template/VehicleTemplate_Wet.bgsm", ""), MaterialClass::Painted);
        assert_eq!(class("Props/Clipboard_d.dds", "", "Shared/Cubemaps/Bronze_e.dds"), MaterialClass::Metal);
        assert_eq!(class("Props/Thing_d.dds", "", "Shared/Cubemaps/mipblur_DefaultOutside1.dds"), MaterialClass::Other);
    }

    #[test]
    fn skin_and_grass_are_their_own_classes() {
        assert_eq!(class("Actors/Character/BaseHumanMale/BaseMaleHead_d.dds", "", ""), MaterialClass::Skin);
        let hints = ClassHints { diffuse: "Props/x_d.dds", skin_tint: true, ..Default::default() };
        assert_eq!(classify_material(&hints), MaterialClass::Skin);
        assert_eq!(class("Landscape/Grass/Grass01_d.dds", "", ""), MaterialClass::Grass);
    }

    #[test]
    fn default_outside_cube_is_not_metal() {
        assert!(!envmap_is_metal("Shared/Cubemaps/mipblur_DefaultOutside1.dds"));
        assert!(envmap_is_metal("Shared/Cubemaps/MetalBrushed01Cube_e.dds"));
        assert!(!envmap_is_metal(""));
    }
}
