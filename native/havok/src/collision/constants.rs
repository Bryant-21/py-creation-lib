/// hkRefCountedProperties entry key for hknpShapeMassProperties, as used by SDK
/// `hknpShape::getMassPropertiesEntry()`. LE bytes `0x00 0xF1`.
pub const REFCOUNTED_PROPS_KEY_MASS_PROPS: u16 = 0xF100;

/// hkRefCountedProperties property-bag entry key for hknpBSMaterialProperties.
///
/// Value 0xF601 is the Bethesda-extension key used by `hknpShape` to locate
/// the per-shape BSMaterial table (CRC + filter info). The runtime scans
/// `hknpShape::properties.entries[]` for an entry with this key when a body
/// is queried for material — a mismatched key (e.g. the mass-props key 0xF100
/// written by accident) makes the lookup return null and the broadphase
/// derefs that null pointer during workshop sphere casts
/// (Fallout4.exe+13E82D0). LE bytes: `0x01 0xF6`. Vanilla reference:
/// `extracted/fo4/Meshes/SetDressing/Safe/Safe01.nif` → `hkRefCountedProperties`
/// on the `hknpCompressedMeshShape`.
pub const REFCOUNTED_PROPS_KEY_BS_MATERIAL: u16 = 0xF601;
