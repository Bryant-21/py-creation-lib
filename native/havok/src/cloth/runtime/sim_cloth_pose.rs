// SimClothPose — typed wrapper over hclSimClothPose.
//
// A cloth pose is a snapshot of per-particle positions (hkVector4 → [f32;4]).

use crate::hkx::types::HkxValue;

use super::base::ClothObjectRef;

pub struct SimClothPose<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> SimClothPose<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn name(&self) -> &str {
        self.inner.get_string("name").unwrap_or("")
    }

    /// Per-particle positions as `[x, y, z]` tuples (w is dropped).
    pub fn positions(&self) -> Vec<[f32; 3]> {
        self.inner
            .get_array("positions")
            .iter()
            .filter_map(|v| match v {
                HkxValue::F32List(floats) if floats.len() >= 3 => {
                    Some([floats[0], floats[1], floats[2]])
                }
                _ => None,
            })
            .collect()
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}
