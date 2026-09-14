use std::collections::HashSet;
use std::path::Path;

use havok_native::hkx::descriptors::{DescriptorRegistry, StructureLayout};
use havok_native::hkx::types::HkxValue;

use crate::model::{NifFile, NifValue};

const HAVOK_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";

pub fn convert_nif_to_ps4(nif: &mut NifFile, source_path: &Path) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    convert_embedded_havok(nif, &mut changes)?;
    normalize_ssf_roots(nif, &mut changes);
    if source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("bto"))
    {
        double_bto_primitive_counts(nif, &mut changes)?;
    }
    Ok(changes)
}

fn convert_embedded_havok(nif: &mut NifFile, changes: &mut Vec<String>) -> Result<(), String> {
    let system_ids = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "bhkPhysicsSystem" | "bhkRagdollSystem"
            )
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();

    for system_id in system_ids {
        let preferred_order = collision_body_order(nif, system_id);
        let source = embedded_bytes(&nif.blocks[system_id])
            .ok_or_else(|| format!("block {system_id} has invalid Binary Data"))?;
        if source.is_empty() {
            continue;
        }
        if !source.starts_with(HAVOK_MAGIC) {
            return Err(format!(
                "block {system_id} does not contain a Havok packfile"
            ));
        }

        let (converted, old_to_new) = convert_havok_packfile(&source, &preferred_order)?;
        set_embedded_bytes(&mut nif.blocks[system_id], converted)?;
        let remapped = remap_collision_body_ids(nif, system_id, &old_to_new);
        changes.push(format!(
            "{system_id} {}: Converted embedded Havok to PS4 layout; remapped {remapped} collision body ID(s)",
            nif.blocks[system_id].type_name
        ));
    }
    Ok(())
}

fn collision_body_order(nif: &NifFile, system_id: usize) -> Vec<usize> {
    nif.blocks
        .iter()
        .filter(|block| block.type_name == "bhkNPCollisionObject")
        .filter(|block| value_ref(block.get_field("Data")) == Some(system_id))
        .filter_map(|block| value_usize(block.get_field("Body ID")))
        .collect()
}

fn convert_havok_packfile(
    source: &[u8],
    preferred_order: &[usize],
) -> Result<(Vec<u8>, Vec<usize>), String> {
    let parsed =
        havok_native::hkx::packfile::parse_packfile(source).map_err(|error| error.to_string())?;
    if parsed.header.version != 11
        || parsed.header.version_name != "hk_2014.1.0-r1"
        || parsed.header.pointer_size != 8
        || parsed.header.little_endian != 1
    {
        return Err(
            "PS4 embedded Havok conversion requires a 64-bit little-endian hk_2014.1.0-r1 packfile"
                .to_string(),
        );
    }

    let mut file = havok_native::hkx::read_packfile(source).map_err(|error| error.to_string())?;
    let old_to_new = reorder_havok_bodies(&mut file, preferred_order)?;
    if parsed.header.reuse_padding_optimization != 0 && is_identity_remap(&old_to_new) {
        return Ok((source.to_vec(), old_to_new));
    }
    let mut registry = DescriptorRegistry::for_contents_version(file.contents_version());
    let bytes =
        havok_native::hkx::write_hkx_with_layout(&file, &mut registry, StructureLayout::Generic);
    Ok((bytes, old_to_new))
}

fn reorder_havok_bodies(
    file: &mut havok_native::hkx::HkxFile,
    preferred_order: &[usize],
) -> Result<Vec<usize>, String> {
    let Some(system) = file.objects_mut().iter_mut().find(|object| {
        matches!(
            object.class_name.as_str(),
            "hknpPhysicsSystemData" | "hknpRagdollData"
        )
    }) else {
        return Ok(Vec::new());
    };
    let body_member = system
        .members
        .iter_mut()
        .find(|member| member.name == "bodyCinfos")
        .ok_or_else(|| format!("{} has no bodyCinfos", system.class_name))?;
    let HkxValue::Array(bodies) = &mut body_member.value else {
        return Err(format!("{}.bodyCinfos is not an array", system.class_name));
    };

    let body_count = bodies.len();
    let mut seen = HashSet::new();
    let mut order = preferred_order
        .iter()
        .copied()
        .filter(|body| *body < body_count && seen.insert(*body))
        .collect::<Vec<_>>();
    order.extend((0..body_count).filter(|body| seen.insert(*body)));
    let mut old_to_new = vec![0; body_count];
    for (new, old) in order.iter().copied().enumerate() {
        old_to_new[old] = new;
    }
    if !is_identity_remap(&old_to_new) {
        let original = bodies.clone();
        *bodies = order.iter().map(|old| original[*old].clone()).collect();
        if let Some(constraints) = system
            .members
            .iter_mut()
            .find(|member| member.name == "constraintCinfos")
            .and_then(|member| match &mut member.value {
                HkxValue::Array(values) => Some(values),
                _ => None,
            })
        {
            for constraint in constraints {
                remap_havok_body_member(constraint, "bodyA", &old_to_new);
                remap_havok_body_member(constraint, "bodyB", &old_to_new);
            }
        }
    }
    Ok(old_to_new)
}

fn remap_havok_body_member(value: &mut HkxValue, name: &str, remap: &[usize]) {
    let Some(members) = value.as_object_members_mut() else {
        return;
    };
    let Some(member) = members.iter_mut().find(|member| member.name == name) else {
        return;
    };
    let Some(old) = hkx_usize(&member.value) else {
        return;
    };
    let Some(new) = remap.get(old).copied() else {
        return;
    };
    set_hkx_usize(&mut member.value, new);
}

fn hkx_usize(value: &HkxValue) -> Option<usize> {
    match value {
        HkxValue::U8(value) => Some(*value as usize),
        HkxValue::U16(value) => Some(*value as usize),
        HkxValue::U32(value) => Some(*value as usize),
        HkxValue::U64(value) => usize::try_from(*value).ok(),
        HkxValue::I8(value) => usize::try_from(*value).ok(),
        HkxValue::I16(value) => usize::try_from(*value).ok(),
        HkxValue::I32(value) => usize::try_from(*value).ok(),
        HkxValue::I64(value) => usize::try_from(*value).ok(),
        _ => None,
    }
}

fn set_hkx_usize(value: &mut HkxValue, replacement: usize) {
    *value = match value {
        HkxValue::U8(_) => HkxValue::U8(replacement as u8),
        HkxValue::U16(_) => HkxValue::U16(replacement as u16),
        HkxValue::U32(_) => HkxValue::U32(replacement as u32),
        HkxValue::U64(_) => HkxValue::U64(replacement as u64),
        HkxValue::I8(_) => HkxValue::I8(replacement as i8),
        HkxValue::I16(_) => HkxValue::I16(replacement as i16),
        HkxValue::I32(_) => HkxValue::I32(replacement as i32),
        HkxValue::I64(_) => HkxValue::I64(replacement as i64),
        _ => return,
    };
}

fn is_identity_remap(remap: &[usize]) -> bool {
    remap.iter().enumerate().all(|(old, new)| old == *new)
}

fn remap_collision_body_ids(nif: &mut NifFile, system_id: usize, remap: &[usize]) -> usize {
    let mut changed = 0;
    for block in nif.blocks.iter_mut().filter(|block| {
        block.type_name == "bhkNPCollisionObject"
            && value_ref(block.get_field("Data")) == Some(system_id)
    }) {
        let Some(old) = value_usize(block.get_field("Body ID")) else {
            continue;
        };
        let Some(new) = remap.get(old).copied() else {
            continue;
        };
        if old != new {
            block.set_field("Body ID", NifValue::UInt(new as u64));
            changed += 1;
        }
    }
    changed
}

fn normalize_ssf_roots(nif: &mut NifFile, changes: &mut Vec<String>) {
    for block in nif
        .blocks
        .iter_mut()
        .filter(|block| block.type_name == "BSSubIndexTriShape")
    {
        let Some(NifValue::Struct(segment_data)) = block.get_field_mut("Segment Data") else {
            continue;
        };
        let Some(NifValue::String(path)) = segment_data.get_mut("SSF File") else {
            continue;
        };
        if path.len() >= 7
            && path[..7].eq_ignore_ascii_case("meshes\\")
            && !path.starts_with("Meshes\\")
        {
            path.replace_range(..7, "Meshes\\");
            changes.push(format!(
                "{} BSSubIndexTriShape: Normalized Segment Data.SSF File root to Meshes\\",
                block.block_id
            ));
        }
    }
}

fn double_bto_primitive_counts(nif: &mut NifFile, changes: &mut Vec<String>) -> Result<(), String> {
    for block in nif
        .blocks
        .iter_mut()
        .filter(|block| block.type_name == "BSSubIndexTriShape")
    {
        let Some(value) = block.get_field("Num Primitives") else {
            continue;
        };
        let count = value_usize(Some(value)).unwrap_or(0);
        let triangle_count = value_usize(block.get_field("Num Triangles")).unwrap_or(0);
        if triangle_count > 0 && count == triangle_count * 2 {
            continue;
        }
        if triangle_count == 0 || count != triangle_count {
            return Err(format!(
                "{} BSSubIndexTriShape has Num Primitives={count}, Num Triangles={triangle_count}; refusing an ambiguous BTO conversion",
                block.block_id
            ));
        }
        let doubled = count.checked_mul(2).ok_or_else(|| {
            format!(
                "{} BSSubIndexTriShape.Num Primitives overflow",
                block.block_id
            )
        })?;
        block.set_field("Num Primitives", NifValue::UInt(doubled as u64));
        changes.push(format!(
            "{} BSSubIndexTriShape: Doubled Num Primitives from {count} to {doubled}",
            block.block_id
        ));
    }
    Ok(())
}

fn embedded_bytes(block: &crate::model::NifBlock) -> Option<Vec<u8>> {
    let NifValue::Struct(binary) = block.get_field("Binary Data")? else {
        return None;
    };
    match binary.get("Data")? {
        NifValue::Bytes(bytes) => Some(bytes.clone()),
        NifValue::Array(values) => Some(values.iter().map(|value| value.as_i64() as u8).collect()),
        _ => None,
    }
}

fn set_embedded_bytes(block: &mut crate::model::NifBlock, bytes: Vec<u8>) -> Result<(), String> {
    let Some(NifValue::Struct(binary)) = block.get_field_mut("Binary Data") else {
        return Err(format!("block {} has invalid Binary Data", block.block_id));
    };
    binary.insert("Data Size".to_string(), NifValue::UInt(bytes.len() as u64));
    binary.insert("Data".to_string(), NifValue::Bytes(bytes));
    Ok(())
}

fn value_ref(value: Option<&NifValue>) -> Option<usize> {
    match value? {
        NifValue::Ref(value) => usize::try_from(*value).ok(),
        NifValue::Int(value) => usize::try_from(*value).ok(),
        NifValue::UInt(value) => usize::try_from(*value).ok(),
        _ => None,
    }
}

fn value_usize(value: Option<&NifValue>) -> Option<usize> {
    match value? {
        NifValue::UInt(value) => usize::try_from(*value).ok(),
        NifValue::Int(value) => usize::try_from(*value).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::model::NifBlock;

    #[test]
    fn ssf_root_normalization_changes_only_the_root_case() {
        let mut nif = NifFile::new("fo4");
        let mut segment_data = IndexMap::new();
        segment_data.insert(
            "SSF File".to_string(),
            NifValue::String("meshes\\Actors\\Human\\Body.ssf".to_string()),
        );
        let shape = nif.add_block("BSSubIndexTriShape", None);
        nif.blocks[shape].set_field("Segment Data", NifValue::Struct(segment_data));

        let mut changes = Vec::new();
        normalize_ssf_roots(&mut nif, &mut changes);

        let NifValue::Struct(segment_data) = nif.blocks[shape].get_field("Segment Data").unwrap()
        else {
            panic!("segment data");
        };
        assert_eq!(
            segment_data.get("SSF File"),
            Some(&NifValue::String(
                "Meshes\\Actors\\Human\\Body.ssf".to_string()
            ))
        );
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn bto_conversion_doubles_only_top_level_primitive_count() {
        let mut nif = NifFile::new("fo4");
        let shape = nif.add_block("BSSubIndexTriShape", None);
        nif.blocks[shape].set_field("Num Triangles", NifValue::UInt(4048));
        nif.blocks[shape].set_field("Num Primitives", NifValue::UInt(4048));
        nif.blocks[shape].set_field(
            "Segment",
            NifValue::Array(vec![NifValue::Struct(IndexMap::from([(
                "Num Primitives".to_string(),
                NifValue::UInt(4048),
            )]))]),
        );

        let mut changes = Vec::new();
        double_bto_primitive_counts(&mut nif, &mut changes).unwrap();

        assert_eq!(
            nif.blocks[shape].get_field("Num Primitives"),
            Some(&NifValue::UInt(8096))
        );
        let NifValue::Array(segments) = nif.blocks[shape].get_field("Segment").unwrap() else {
            panic!("segments");
        };
        let NifValue::Struct(segment) = &segments[0] else {
            panic!("segment");
        };
        assert_eq!(segment.get("Num Primitives"), Some(&NifValue::UInt(4048)));

        let mut second_changes = Vec::new();
        double_bto_primitive_counts(&mut nif, &mut second_changes).unwrap();
        assert!(second_changes.is_empty());
        assert_eq!(
            nif.blocks[shape].get_field("Num Primitives"),
            Some(&NifValue::UInt(8096))
        );
    }

    #[test]
    fn collision_body_ids_follow_the_havok_remap() {
        let mut nif = NifFile::new("fo4");
        let system_id = nif.add_block("bhkRagdollSystem", None);
        for body_id in [2, 0, 1] {
            let mut block = NifBlock::new(nif.blocks.len(), "bhkNPCollisionObject");
            block.set_field("Data", NifValue::Ref(system_id as i32));
            block.set_field("Body ID", NifValue::UInt(body_id));
            nif.blocks.push(block);
        }

        let changed = remap_collision_body_ids(&mut nif, system_id, &[1, 2, 0]);

        assert_eq!(changed, 3);
        assert_eq!(
            nif.blocks
                .iter()
                .filter(|block| block.type_name == "bhkNPCollisionObject")
                .map(|block| value_usize(block.get_field("Body ID")).unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }
}
