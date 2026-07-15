// ClothState — typed wrapper over hclClothState.
//
// A cloth state is a named combination of operators referenced by index
// into the parent hclClothData.operators array.

use crate::hkx::types::HkxValue;

use super::base::ClothObjectRef;

pub struct ClothState<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> ClothState<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn name(&self) -> &str {
        self.inner.get_string("name").unwrap_or("")
    }

    /// Indices of the operators executed in this state (in execution order).
    pub fn operator_indices(&self) -> Vec<u32> {
        extract_u32_array(self.inner.get_array("operators"))
    }

    /// Indices of sim cloths used by this state.
    pub fn used_sim_cloth_indices(&self) -> Vec<u32> {
        extract_u32_array(self.inner.get_array("usedSimCloths"))
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

fn extract_u32_array(values: &[HkxValue]) -> Vec<u32> {
    values
        .iter()
        .filter_map(|v| match v {
            HkxValue::U8(n) => Some(u32::from(*n)),
            HkxValue::U16(n) => Some(u32::from(*n)),
            HkxValue::U32(n) => Some(*n),
            HkxValue::I32(n) => Some(*n as u32),
            _ => None,
        })
        .collect()
}
