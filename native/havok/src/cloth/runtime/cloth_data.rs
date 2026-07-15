// ClothData — typed wrapper over hclClothData (root cloth container).
// HCL member names are camelCase; Rust accessors are snake_case.

use crate::hkx::HkxFile;

use super::base::ClothObjectRef;
use super::sim_cloth_data::SimClothData;

pub struct ClothData<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> ClothData<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    /// Find the first `hclClothData` object in `file` and wrap it.
    ///
    /// Returns `None` when the file has no `hclClothData` (e.g. an
    /// animation HKX that carries no cloth).
    pub fn from_hkx_file(file: &'a HkxFile) -> Option<Self> {
        file.objects()
            .iter()
            .find(|obj| obj.class_name == "hclClothData")
            .map(|obj| Self::new(ClothObjectRef::new(obj, file)))
    }

    /// Cloth item name as stored in the HKX (e.g. `"Robes"`).
    pub fn name(&self) -> &str {
        self.inner.get_string("name").unwrap_or("")
    }

    /// Simulation data objects — typically one per cloth item.
    pub fn sim_cloth_datas(&self) -> Vec<SimClothData<'a>> {
        self.inner
            .resolve_ptr_array("simClothDatas")
            .into_iter()
            .map(SimClothData::new)
            .collect()
    }

    /// Named cloth state combinations (operator lists).
    pub fn cloth_states(&self) -> Vec<ClothObjectRef<'a>> {
        self.inner.resolve_ptr_array("clothStateDatas")
    }

    /// Mixed-type operator graph.
    pub fn operators(&self) -> Vec<ClothObjectRef<'a>> {
        self.inner.resolve_ptr_array("operators")
    }

    /// Buffer definitions (display, static, sim, scratch).
    pub fn buffer_definitions(&self) -> Vec<ClothObjectRef<'a>> {
        self.inner.resolve_ptr_array("bufferDefinitions")
    }

    /// Transform set definitions (skeleton pose bindings).
    pub fn transform_set_definitions(&self) -> Vec<ClothObjectRef<'a>> {
        self.inner.resolve_ptr_array("transformSetDefinitions")
    }

    /// The underlying object ref (useful for generic access).
    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}
