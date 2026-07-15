//! Cloth subsystem units.
//!
//! Canonical choice (matches the SDK comment in
//! `refs/hk2018_1_0_r1/Source/Cloth/Cloth/SimCloth/hclSimClothData.h:174`
//! "Gravity. Defaults to (0,0,-9.8f).") :
//!
//!   * **Z-up** axis convention (gravity along negative Z).
//!   * **m/s²** for gravity magnitude (SI metres per second squared).
//!
/// Canonical Earth gravity along world Z, in m/s².
pub const GRAVITY_Z: f32 = -9.81;

/// Game-units-per-metre for FO4 (1 metre ≈ 70 game units in BSXFlags scale).
/// Use only at the scale boundary where bake/runtime needs cm/s² instead of m/s².
pub const GAME_UNITS_PER_METER: f32 = 70.0;
