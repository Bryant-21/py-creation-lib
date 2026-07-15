use indexmap::IndexMap;

use crate::model::NifValue;

#[derive(Debug, Clone, Copy)]
pub struct SegmentSpec {
    pub triangle_start: u32,
    pub triangle_count: u32,
    pub user_index: u32,
}

pub fn build_segment_data(specs: &[SegmentSpec]) -> (u32, NifValue, u32) {
    if specs.is_empty() {
        let mut fields = IndexMap::new();
        fields.insert("Start Index".into(), NifValue::UInt(0));
        fields.insert("Num Primitives".into(), NifValue::UInt(0));
        fields.insert("Parent Array Index".into(), NifValue::UInt(u32::MAX as u64));
        fields.insert("Num Sub Segments".into(), NifValue::UInt(0));
        fields.insert("Sub Segment".into(), NifValue::Array(Vec::new()));
        fields.insert("User Index".into(), NifValue::UInt(0));
        let segments = NifValue::Array(vec![NifValue::Struct(fields)]);
        return (1, segments, 1);
    }

    let mut entries = Vec::with_capacity(specs.len());
    for spec in specs {
        let mut fields = IndexMap::new();
        fields.insert(
            "Start Index".into(),
            NifValue::UInt((spec.triangle_start * 3) as u64),
        );
        fields.insert(
            "Num Primitives".into(),
            NifValue::UInt(spec.triangle_count as u64),
        );
        fields.insert("Parent Array Index".into(), NifValue::UInt(u32::MAX as u64));
        fields.insert("Num Sub Segments".into(), NifValue::UInt(0));
        fields.insert("Sub Segment".into(), NifValue::Array(Vec::new()));
        fields.insert("User Index".into(), NifValue::UInt(spec.user_index as u64));
        entries.push(NifValue::Struct(fields));
    }
    let total = specs.len() as u32;
    (total, NifValue::Array(entries), total)
}
