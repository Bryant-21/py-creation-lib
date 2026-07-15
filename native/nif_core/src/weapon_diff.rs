use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::model::{NifBlock, NifFile, NifValue};

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
struct BlockSignature {
    type_name: String,
    name: String,
    translation: String,
    rotation: String,
    geometry_hash: u64,
}

fn geometry_hash(block: &NifBlock) -> u64 {
    if !matches!(
        block.type_name.as_str(),
        "BSTriShape" | "BSSubIndexTriShape" | "NiTriShape"
    ) {
        return 0;
    }
    let mut hasher = DefaultHasher::new();
    for field_name in ["Vertex Data", "Triangles", "Data"] {
        if let Some(value) = block.get_field(field_name) {
            hash_value(value, &mut hasher);
        }
    }
    hasher.finish()
}

fn hash_value(value: &NifValue, hasher: &mut DefaultHasher) {
    match value {
        NifValue::Null => "null".hash(hasher),
        NifValue::Bool(v) => v.hash(hasher),
        NifValue::Int(v) => v.hash(hasher),
        NifValue::UInt(v) => v.hash(hasher),
        NifValue::Float(v) => v.to_bits().hash(hasher),
        NifValue::FloatNan(v) => v.hash(hasher),
        NifValue::String(v) | NifValue::Char(v) => v.hash(hasher),
        NifValue::Ref(v) => v.hash(hasher),
        NifValue::Vec3(v) => v.iter().for_each(|item| item.to_bits().hash(hasher)),
        NifValue::Vec4(v) | NifValue::Color4(v) | NifValue::Quaternion(v) => {
            v.iter().for_each(|item| item.to_bits().hash(hasher))
        }
        NifValue::Color3(v) => v.iter().for_each(|item| item.to_bits().hash(hasher)),
        NifValue::Matrix33(v) => v
            .iter()
            .flat_map(|row| row.iter())
            .for_each(|item| item.to_bits().hash(hasher)),
        NifValue::Matrix44(v) => v
            .iter()
            .flat_map(|row| row.iter())
            .for_each(|item| item.to_bits().hash(hasher)),
        NifValue::Array(items) => items.iter().for_each(|item| hash_value(item, hasher)),
        NifValue::Struct(fields) => {
            for (key, item) in fields {
                key.hash(hasher);
                hash_value(item, hasher);
            }
        }
        NifValue::Bytes(v) => v.hash(hasher),
    }
}

fn block_signature(block: &NifBlock) -> BlockSignature {
    BlockSignature {
        type_name: block.type_name.clone(),
        name: match block.get_field("Name") {
            Some(NifValue::String(value)) => value.clone(),
            _ => String::new(),
        },
        translation: format!("{:?}", block.get_field("Translation")),
        rotation: format!("{:?}", block.get_field("Rotation")),
        geometry_hash: geometry_hash(block),
    }
}

pub fn weapon_block_diff(base: &NifFile, mod_nif: &NifFile) -> Vec<i32> {
    let base_signatures: HashSet<BlockSignature> =
        base.blocks.iter().map(block_signature).collect();

    mod_nif
        .blocks
        .iter()
        .filter(|block| !base_signatures.contains(&block_signature(block)))
        .map(|block| block.block_id as i32)
        .collect()
}
