// SimClothData — typed wrapper over hclSimClothData.

use crate::hkx::types::HkxValue;

use super::base::ClothObjectRef;

pub struct SimClothData<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> SimClothData<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn name(&self) -> &str {
        self.inner.get_string("name").unwrap_or("")
    }

    /// Number of particles (read from `particleDatas` array length).
    pub fn num_particles(&self) -> usize {
        self.inner.get_array("particleDatas").len()
    }

    /// Raw particle data entries (structs with position/mass/etc).
    pub fn particles(&self) -> &'a [HkxValue] {
        self.inner.get_array("particleDatas")
    }

    /// Indices of pinned ("fixed") particles.
    ///
    /// The HKX reader stores `hclSimClothData.fixedParticles` as an array of
    /// integer values (U16 or U32 depending on particle count).
    pub fn fixed_particle_indices(&self) -> Vec<u32> {
        self.inner
            .get_array("fixedParticles")
            .iter()
            .filter_map(|v| match v {
                HkxValue::U8(n) => Some(u32::from(*n)),
                HkxValue::U16(n) => Some(u32::from(*n)),
                HkxValue::U32(n) => Some(*n),
                HkxValue::I32(n) => Some(*n as u32),
                HkxValue::U64(n) => Some(*n as u32),
                HkxValue::I64(n) => Some(*n as u32),
                _ => None,
            })
            .collect()
    }

    /// Constraint sets (mixed types: StandardLink, Stretch, Bend, ...).
    pub fn constraint_sets(&self) -> Vec<ClothObjectRef<'a>> {
        self.inner.resolve_ptr_array("staticConstraintSets")
    }

    /// Collidables associated with this sim cloth instance.
    pub fn per_instance_collidables(&self) -> Vec<ClothObjectRef<'a>> {
        self.inner.resolve_ptr_array("perInstanceCollidables")
    }

    /// Named cloth poses (typically one: `"DefaultClothPose"`).
    pub fn sim_cloth_poses(&self) -> Vec<ClothObjectRef<'a>> {
        self.inner.resolve_ptr_array("simClothPoses")
    }

    /// The first cloth pose, if any.
    pub fn default_pose(&self) -> Option<ClothObjectRef<'a>> {
        self.sim_cloth_poses().into_iter().next()
    }

    /// The underlying object ref.
    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}
